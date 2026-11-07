//! Bounded, disposable apparent-size observations; never mutation authority.
//! Cache hits are provisional and are remeasured by the browser's normal helper.
#![cfg_attr(not(unix), allow(dead_code, unused_imports))]
use std::{
    ffi::{OsStr, OsString},
    fs::{File, Metadata},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
pub const MAX_ROWS: usize = 256;
pub const MAX_BYTES: usize = 1024 * 1024;
pub const MAX_NAMES: usize = 64;
const MAX_AGE: Duration = Duration::from_secs(24 * 3600);
const MAGIC: &[u8; 8] = b"FILESDU1";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub device: u64,
    pub inode: u64,
    pub modified_seconds: i64,
    pub modified_nanos: i64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Observation {
    pub bytes: u64,
    pub entries: u64,
    pub at: SystemTime,
}
#[derive(Clone, Debug)]
pub struct Checked {
    pub name: OsString,
    pub stamp: Stamp,
    pub cached: Option<Observation>,
}
#[derive(Clone, Debug)]
pub struct Completed {
    pub name: OsString,
    pub stamp: Stamp,
    pub value: Observation,
}
#[derive(Debug)]
pub enum Op {
    Read { root: PathBuf, names: Vec<OsString> },
    Save { root: PathBuf, rows: Vec<Completed> },
    Invalidate { roots: Vec<PathBuf> },
}
#[derive(Debug)]
pub enum Updated {
    Read(Vec<Checked>, Option<String>),
    Saved,
    Invalidated,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Row {
    path: PathBuf,
    stamp: Stamp,
    value: Observation,
}
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Folder-size cache is incompatible or exceeds its budget",
    )
}
fn valid_path(path: &Path) -> bool {
    path.is_absolute()
        && path.as_os_str().as_encoded_bytes().len() <= 4096
        && path
            .components()
            .all(|c| matches!(c, Component::RootDir | Component::Normal(_)))
        && !path.as_os_str().as_encoded_bytes().contains(&0)
}
fn valid_name(name: &OsStr) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name
            .as_encoded_bytes()
            .iter()
            .any(|b| *b == 0 || *b == b'/')
}
fn fresh(value: Observation, now: SystemTime) -> bool {
    value.entries <= crate::folder_usage::MAX_ENTRIES
        && now.duration_since(value.at).is_ok_and(|age| age <= MAX_AGE)
}
fn raw_path(raw: &[u8]) -> io::Result<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Ok(Path::new(OsStr::from_bytes(raw)).to_path_buf())
    }
    #[cfg(not(unix))]
    {
        Ok(PathBuf::from(
            std::str::from_utf8(raw).map_err(|_| invalid())?,
        ))
    }
}
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> io::Result<&'a [u8]> {
        let end = self
            .at
            .checked_add(n)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(invalid)?;
        let b = &self.bytes[self.at..end];
        self.at = end;
        Ok(b)
    }
    fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> io::Result<i64> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
}
fn decode(bytes: &[u8]) -> io::Result<Vec<Row>> {
    if bytes.len() > MAX_BYTES {
        return Err(invalid());
    }
    let mut r = Reader { bytes, at: 0 };
    if r.take(8)? != MAGIC {
        return Err(invalid());
    }
    let count = u16::from_le_bytes(r.take(2)?.try_into().unwrap()) as usize;
    if count > MAX_ROWS {
        return Err(invalid());
    }
    let mut rows = Vec::with_capacity(count);
    for _ in 0..count {
        let len = u16::from_le_bytes(r.take(2)?.try_into().unwrap()) as usize;
        if len > 4096 {
            return Err(invalid());
        }
        let path = raw_path(r.take(len)?)?;
        if !valid_path(&path) || rows.iter().any(|row: &Row| row.path == path) {
            return Err(invalid());
        }
        let stamp = Stamp {
            device: r.u64()?,
            inode: r.u64()?,
            modified_seconds: r.i64()?,
            modified_nanos: r.i64()?,
        };
        let value = Observation {
            bytes: r.u64()?,
            entries: r.u64()?,
            at: UNIX_EPOCH
                .checked_add(Duration::from_secs(r.u64()?))
                .ok_or_else(invalid)?,
        };
        if value.entries > crate::folder_usage::MAX_ENTRIES
            || !(0..1_000_000_000).contains(&stamp.modified_nanos)
        {
            return Err(invalid());
        }
        rows.push(Row { path, stamp, value });
    }
    if r.at != bytes.len() {
        return Err(invalid());
    }
    Ok(rows)
}
fn encode(rows: &[Row]) -> io::Result<Vec<u8>> {
    if rows.len() > MAX_ROWS {
        return Err(invalid());
    }
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(rows.len() as u16).to_le_bytes());
    for (i, row) in rows.iter().enumerate() {
        if !valid_path(&row.path)
            || rows[..i].iter().any(|r| r.path == row.path)
            || row.value.entries > crate::folder_usage::MAX_ENTRIES
            || !(0..1_000_000_000).contains(&row.stamp.modified_nanos)
        {
            return Err(invalid());
        }
        let raw = row.path.as_os_str().as_encoded_bytes();
        if out.len() + 2 + raw.len() + 56 > MAX_BYTES {
            return Err(invalid());
        }
        out.extend_from_slice(&(raw.len() as u16).to_le_bytes());
        out.extend_from_slice(raw);
        out.extend_from_slice(&row.stamp.device.to_le_bytes());
        out.extend_from_slice(&row.stamp.inode.to_le_bytes());
        out.extend_from_slice(&row.stamp.modified_seconds.to_le_bytes());
        out.extend_from_slice(&row.stamp.modified_nanos.to_le_bytes());
        out.extend_from_slice(&row.value.bytes.to_le_bytes());
        out.extend_from_slice(&row.value.entries.to_le_bytes());
        out.extend_from_slice(
            &row.value
                .at
                .duration_since(UNIX_EPOCH)
                .map_err(|_| invalid())?
                .as_secs()
                .to_le_bytes(),
        );
    }
    Ok(out)
}
#[cfg(unix)]
mod storage {
    use super::*;
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::fs::MetadataExt,
        },
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Lock {
        file: File,
        owner: libc::pid_t,
    }
    impl Drop for Lock {
        fn drop(&mut self) {
            if self.owner == unsafe { libc::getpid() } {
                loop {
                    if unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) } == 0
                        || io::Error::last_os_error().kind() != io::ErrorKind::Interrupted
                    {
                        break;
                    }
                }
            }
        }
    }
    fn name(raw: &OsStr) -> io::Result<CString> {
        CString::new(raw.as_encoded_bytes()).map_err(|_| invalid())
    }
    fn wrap(fd: libc::c_int) -> io::Result<File> {
        if fd < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    }
    pub fn directory(path: &Path, create: bool) -> io::Result<File> {
        if !valid_path(path) {
            return Err(invalid());
        }
        let mut dir = wrap(unsafe {
            libc::open(
                c"/".as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        })?;
        for component in path.components() {
            if let Component::Normal(part) = component {
                let part = name(part)?;
                let mut fd = unsafe {
                    libc::openat(
                        dir.as_raw_fd(),
                        part.as_ptr(),
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                    )
                };
                if fd < 0 && create && io::Error::last_os_error().kind() == io::ErrorKind::NotFound
                {
                    if unsafe { libc::mkdirat(dir.as_raw_fd(), part.as_ptr(), 0o700) } != 0
                        && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists
                    {
                        return Err(io::Error::last_os_error());
                    }
                    fd = unsafe {
                        libc::openat(
                            dir.as_raw_fd(),
                            part.as_ptr(),
                            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                        )
                    };
                }
                dir = wrap(fd)?;
            }
        }
        Ok(dir)
    }
    pub fn child(dir: &File, part: &OsStr) -> io::Result<File> {
        if !valid_name(part) {
            return Err(invalid());
        }
        let part = name(part)?;
        wrap(unsafe {
            libc::openat(
                dir.as_raw_fd(),
                part.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        })
    }
    pub fn stamp(metadata: &Metadata) -> Stamp {
        Stamp {
            device: metadata.dev(),
            inode: metadata.ino(),
            modified_seconds: metadata.mtime(),
            modified_nanos: metadata.mtime_nsec(),
        }
    }
    fn check_private(dir: &File) -> io::Result<()> {
        let m = dir.metadata()?;
        if m.uid() != unsafe { libc::geteuid() } || m.mode() & 0o077 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Folder-size cache directory must be private",
            ));
        }
        Ok(())
    }
    pub fn read(config: &Path) -> io::Result<Vec<Row>> {
        let dir = match directory(&config.join("size-cache"), false) {
            Ok(d) => d,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        check_private(&dir)?;
        read_from(&dir)
    }
    fn read_from(dir: &File) -> io::Result<Vec<Row>> {
        let fd = unsafe {
            libc::openat(
                dir.as_raw_fd(),
                c"observations.bin".as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        let file = match wrap(fd) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let m = file.metadata()?;
        if !m.is_file()
            || m.uid() != unsafe { libc::geteuid() }
            || m.mode() & 0o077 != 0
            || m.len() > MAX_BYTES as u64
        {
            return Err(invalid());
        }
        let mut bytes = Vec::new();
        file.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes)?;
        decode(&bytes)
    }
    pub fn save(
        config: &Path,
        added: Vec<Row>,
        removed: &[PathBuf],
        now: SystemTime,
    ) -> io::Result<()> {
        let dir = directory(&config.join("size-cache"), true)?;
        check_private(&dir)?;
        // Coordination is best effort, but never block the shared auxiliary worker.
        let lock = wrap(unsafe {
            libc::openat(
                dir.as_raw_fd(),
                c"observations.lock".as_ptr(),
                libc::O_RDWR
                    | libc::O_CREAT
                    | libc::O_NOFOLLOW
                    | libc::O_NONBLOCK
                    | libc::O_CLOEXEC,
                0o600,
            )
        })?;
        let lm = lock.metadata()?;
        if !lm.is_file() || lm.uid() != unsafe { libc::geteuid() } || lm.mode() & 0o077 != 0 {
            return Err(invalid());
        }
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let _guard = Lock {
            file: lock,
            owner: unsafe { libc::getpid() },
        };
        let mut rows = read_from(&dir)?;
        rows.retain(|r| {
            !added.iter().any(|a| a.path == r.path)
                && !removed
                    .iter()
                    .any(|root| r.path.starts_with(root) || root.starts_with(&r.path))
                && fresh(r.value, now)
        });
        rows.extend(added);
        rows.sort_by_key(|r| r.value.at);
        while rows.len() > MAX_ROWS
            || rows
                .iter()
                .map(|r| r.path.as_os_str().as_encoded_bytes().len() + 58)
                .sum::<usize>()
                + 10
                > MAX_BYTES
        {
            rows.remove(0);
        }
        let bytes = encode(&rows)?;
        let mut temp = None;
        for _ in 0..64 {
            let n = CString::new(format!(
                "observations.{}-{}.tmp",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ))
            .unwrap();
            let fd = unsafe {
                libc::openat(
                    dir.as_raw_fd(),
                    n.as_ptr(),
                    libc::O_WRONLY
                        | libc::O_CREAT
                        | libc::O_EXCL
                        | libc::O_NOFOLLOW
                        | libc::O_CLOEXEC,
                    0o600,
                )
            };
            match wrap(fd) {
                Ok(f) => {
                    temp = Some((n, f));
                    break;
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
        }
        let (name, mut file) = temp.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::AlreadyExists,
                "Cache temporary-name budget exhausted",
            )
        })?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        // Retain an unpublished temporary on failure; never remove fixture trees.
        if unsafe {
            libc::renameat(
                dir.as_raw_fd(),
                name.as_ptr(),
                dir.as_raw_fd(),
                c"observations.bin".as_ptr(),
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        dir.sync_all()?;
        Ok(())
    }
}
/// Runs on the auxiliary IO worker; no filesystem operation belongs on the UI thread.
pub fn apply(config: &Path, op: &Op) -> io::Result<Updated> {
    #[cfg(not(unix))]
    {
        let _ = (config, op);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Folder-size observation cache is unavailable on this platform",
        ))
    }
    #[cfg(unix)]
    {
        let now = SystemTime::now();
        match op {
            Op::Invalidate { roots } => {
                if roots.len() > MAX_NAMES || roots.iter().any(|root| !valid_path(root)) {
                    return Err(invalid());
                }
                storage::save(config, Vec::new(), roots, now)?;
                Ok(Updated::Invalidated)
            }
            Op::Read { root, names } => {
                if names.len() > MAX_NAMES
                    || !valid_path(root)
                    || names
                        .iter()
                        .enumerate()
                        .any(|(i, n)| !valid_name(n) || names[..i].contains(n))
                {
                    return Err(invalid());
                }
                let parent = storage::directory(root, false)?;
                let (rows, warning) = match storage::read(config) {
                    Ok(rows) => (rows, None),
                    Err(error) => (Vec::new(), Some(error.to_string())),
                };
                let mut result = Vec::with_capacity(names.len());
                for name in names {
                    let Ok(child) = storage::child(&parent, name) else {
                        continue;
                    };
                    let stamp = storage::stamp(&child.metadata()?);
                    let path = root.join(name);
                    let cached = rows
                        .iter()
                        .find(|r| r.path == path && r.stamp == stamp && fresh(r.value, now))
                        .map(|r| r.value);
                    result.push(Checked {
                        name: name.clone(),
                        stamp,
                        cached,
                    });
                }
                Ok(Updated::Read(result, warning))
            }
            Op::Save { root, rows } => {
                if !valid_path(root)
                    || rows.len() > MAX_NAMES
                    || rows.iter().enumerate().any(|(i, row)| {
                        !valid_name(&row.name)
                            || !fresh(row.value, now)
                            || rows[..i].iter().any(|old| old.name == row.name)
                    })
                {
                    return Err(invalid());
                }
                let parent = storage::directory(root, false)?;
                let mut added = Vec::with_capacity(rows.len());
                for row in rows {
                    let Ok(child) = storage::child(&parent, &row.name) else {
                        continue;
                    };
                    if storage::stamp(&child.metadata()?) == row.stamp {
                        added.push(Row {
                            path: root.join(&row.name),
                            stamp: row.stamp,
                            value: row.value,
                        });
                    }
                }
                if !added.is_empty() {
                    storage::save(config, added, &[], now)?;
                }
                Ok(Updated::Saved)
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{
        fs,
        os::unix::{
            ffi::OsStringExt,
            fs::{DirBuilderExt, MetadataExt, symlink},
        },
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    fn fixture() -> PathBuf {
        let parent = std::env::temp_dir();
        let meta = fs::symlink_metadata(&parent).unwrap();
        assert!(
            meta.is_dir()
                && !meta.file_type().is_symlink()
                && meta.uid() == unsafe { libc::geteuid() }
                && meta.mode() & 0o077 == 0,
            "Tests require an owned private TMPDIR"
        );
        let root = parent.join(format!(
            "folder-cache-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        root
    }
    fn observation(bytes: u64) -> Observation {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        Observation {
            bytes,
            entries: 1,
            at: UNIX_EPOCH + Duration::from_secs(secs),
        }
    }
    fn row(path: PathBuf) -> Row {
        Row {
            path,
            stamp: Stamp {
                device: 4,
                inode: 5,
                modified_seconds: 6,
                modified_nanos: 7,
            },
            value: observation(0),
        }
    }
    #[test]
    fn codec_retains_raw_paths_and_zero_is_a_real_observation() {
        let a = row(PathBuf::from(OsString::from_vec(
            b"/private/raw-\xff".to_vec(),
        )));
        let encoded = encode(std::slice::from_ref(&a)).unwrap();
        assert_eq!(decode(&encoded).unwrap(), vec![a.clone()]);
        assert!(encode(&[a.clone(), a]).is_err());
        for n in 0..encoded.len() {
            assert!(decode(&encoded[..n]).is_err());
        }
        let mut trailing = encoded;
        trailing.push(0);
        assert!(decode(&trailing).is_err());
    }
    #[test]
    fn codec_and_age_are_bounded_without_reinterpreting_future_or_stale_values() {
        let now = SystemTime::now();
        let mut a = row(PathBuf::from("/private/a"));
        a.value.at = now + Duration::from_secs(1);
        assert!(!fresh(a.value, now));
        a.value.at = now - MAX_AGE - Duration::from_secs(1);
        assert!(!fresh(a.value, now));
        a.value.entries = crate::folder_usage::MAX_ENTRIES + 1;
        assert!(encode(&[a.clone()]).is_err());
        a.value = observation(8);
        a.path = PathBuf::from(format!("/{}", "x".repeat(4096)));
        assert!(encode(&[a]).is_err());
        assert!(decode(&vec![0; MAX_BYTES + 1]).is_err());
    }
    #[test]
    fn persisted_observation_reloads_but_replaced_directory_identity_does_not() {
        let run = fixture();
        let config = run.join("config");
        let root = run.join("browser");
        fs::DirBuilder::new().mode(0o700).create(&config).unwrap();
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        fs::create_dir(root.join("a")).unwrap();
        fs::write(root.join("a/original"), b"retained bytes").unwrap();
        let Updated::Read(first, None) = apply(
            &config,
            &Op::Read {
                root: root.clone(),
                names: vec!["a".into()],
            },
        )
        .unwrap() else {
            panic!("read")
        };
        assert!(first[0].cached.is_none());
        let value = observation(14);
        apply(
            &config,
            &Op::Save {
                root: root.clone(),
                rows: vec![Completed {
                    name: "a".into(),
                    stamp: first[0].stamp,
                    value,
                }],
            },
        )
        .unwrap();
        let Updated::Read(second, None) = apply(
            &config,
            &Op::Read {
                root: root.clone(),
                names: vec!["a".into()],
            },
        )
        .unwrap() else {
            panic!("reload")
        };
        assert_eq!(second[0].cached, Some(value));
        fs::rename(root.join("a"), run.join("retained-a")).unwrap();
        fs::create_dir(root.join("a")).unwrap();
        let Updated::Read(third, None) = apply(
            &config,
            &Op::Read {
                root: root.clone(),
                names: vec!["a".into()],
            },
        )
        .unwrap() else {
            panic!("replacement")
        };
        assert!(third[0].cached.is_none());
        assert_eq!(
            fs::read(run.join("retained-a/original")).unwrap(),
            b"retained bytes"
        );
        apply(
            &config,
            &Op::Save {
                root: root.clone(),
                rows: vec![Completed {
                    name: "a".into(),
                    stamp: first[0].stamp,
                    value,
                }],
            },
        )
        .unwrap();
        let Updated::Read(last, None) = apply(
            &config,
            &Op::Read {
                root,
                names: vec!["a".into()],
            },
        )
        .unwrap() else {
            panic!("stale save")
        };
        assert!(last[0].cached.is_none());
    }
    #[test]
    fn observed_nested_changes_invalidate_ancestors_without_touching_siblings() {
        let run = fixture();
        let config = run.join("config");
        let root = run.join("browser");
        for path in [&config, &root] {
            fs::DirBuilder::new().mode(0o700).create(path).unwrap();
        }
        for name in ["a", "ab"] {
            fs::create_dir(root.join(name)).unwrap();
            fs::write(root.join(name).join("retained"), b"unchanged").unwrap();
        }
        fs::create_dir(root.join("a/nested")).unwrap();
        let Updated::Read(rows, None) = apply(
            &config,
            &Op::Read {
                root: root.clone(),
                names: vec!["a".into(), "ab".into()],
            },
        )
        .unwrap() else {
            panic!("read")
        };
        apply(
            &config,
            &Op::Save {
                root: root.clone(),
                rows: rows
                    .iter()
                    .map(|r| Completed {
                        name: r.name.clone(),
                        stamp: r.stamp,
                        value: observation(9),
                    })
                    .collect(),
            },
        )
        .unwrap();
        fs::write(root.join("a/nested/new"), b"new bytes").unwrap();
        assert_eq!(
            storage::stamp(&fs::metadata(root.join("a")).unwrap()),
            rows[0].stamp
        );
        assert!(matches!(
            apply(
                &config,
                &Op::Invalidate {
                    roots: vec![root.join("a/nested")]
                }
            )
            .unwrap(),
            Updated::Invalidated
        ));
        let Updated::Read(after, None) = apply(
            &config,
            &Op::Read {
                root: root.clone(),
                names: vec!["a".into(), "ab".into()],
            },
        )
        .unwrap() else {
            panic!("read invalidated")
        };
        assert!(after[0].cached.is_none());
        assert!(after[1].cached.is_some());
        assert_eq!(fs::read(root.join("a/retained")).unwrap(), b"unchanged");
        assert_eq!(fs::read(root.join("ab/retained")).unwrap(), b"unchanged");
    }
    #[test]
    fn links_remain_inside_the_fixture_and_are_never_followed_for_sizes_or_cache() {
        let run = fixture();
        let config = run.join("config");
        let root = run.join("browser");
        let target = run.join("private-target");
        for path in [&config, &root, &target] {
            fs::DirBuilder::new().mode(0o700).create(path).unwrap();
        }
        fs::write(target.join("sentinel"), b"unchanged").unwrap();
        symlink(&target, root.join("linked")).unwrap();
        fs::create_dir(root.join("a")).unwrap();
        let Updated::Read(rows, None) = apply(
            &config,
            &Op::Read {
                root: root.clone(),
                names: vec!["a".into(), "linked".into()],
            },
        )
        .unwrap() else {
            panic!("read")
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "a");
        fs::DirBuilder::new()
            .mode(0o700)
            .create(config.join("size-cache"))
            .unwrap();
        symlink(
            target.join("sentinel"),
            config.join("size-cache/observations.bin"),
        )
        .unwrap();
        let Updated::Read(rows, Some(_)) = apply(
            &config,
            &Op::Read {
                root: root.clone(),
                names: vec!["a".into()],
            },
        )
        .unwrap() else {
            panic!("cache link rejected")
        };
        assert!(rows[0].cached.is_none());
        assert!(
            apply(
                &config,
                &Op::Save {
                    root,
                    rows: vec![Completed {
                        name: "a".into(),
                        stamp: rows[0].stamp,
                        value: observation(0)
                    }]
                }
            )
            .is_err()
        );
        assert_eq!(fs::read(target.join("sentinel")).unwrap(), b"unchanged");
    }
}
