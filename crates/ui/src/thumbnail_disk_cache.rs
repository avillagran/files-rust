//! Disposable thumbnail records, bounded independently of the RAM LRU.
//! Fast private tier in front of the freedesktop shared cache: a hit is raw
//! 160 px RGB and needs no helper decode. Each of 1024 fixed slots is at most
//! 80 KiB (about 80 MiB of disk at worst, no RAM). A hash collision is a cache miss,
//! never identity proof: every read compares the full raw path and revision.
#![cfg_attr(not(unix), allow(dead_code, unused_imports))]
use crate::image_preview_job::{Bitmap, THUMBNAIL_DIMENSION};
use files_core::media_source::Revision;
use std::{
    io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

const MAGIC: &[u8; 8] = b"FILESTH3";
const MAX_ENTRY: usize = 80 * 1024;
const SLOTS: u64 = 1024;
const TTL: u64 = 30 * 24 * 3600;
// Bump the namespace when decoder recipe, sampling or color conversion changes.
const RECIPE: u32 = 3;

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "La caché de miniaturas es incompatible o está dañada",
    )
}
fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    })
}
fn slot(path: &Path) -> String {
    format!(
        "slot-{:03}.bin",
        hash(path.as_os_str().as_encoded_bytes()) % SLOTS
    )
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn valid_path(path: &Path) -> bool {
    path.is_absolute()
        && path.as_os_str().as_encoded_bytes().len() <= 4096
        && !path.as_os_str().as_encoded_bytes().contains(&0)
}
fn encode(path: &Path, bitmap: Option<&Bitmap>) -> io::Result<Vec<u8>> {
    if !valid_path(path) {
        return Err(invalid());
    }
    let raw = path.as_os_str().as_encoded_bytes();
    let mut b = Vec::with_capacity(MAX_ENTRY);
    b.extend_from_slice(MAGIC);
    b.extend_from_slice(&RECIPE.to_le_bytes());
    b.extend_from_slice(&(THUMBNAIL_DIMENSION).to_le_bytes());
    b.extend_from_slice(&now().to_le_bytes());
    b.extend_from_slice(&(raw.len() as u16).to_le_bytes());
    b.extend_from_slice(raw);
    b.push(u8::from(bitmap.is_some()));
    if let Some(v) = bitmap {
        let r = v.source.ok_or_else(invalid)?;
        if v.width == 0
            || v.height == 0
            || v.width > THUMBNAIL_DIMENSION
            || v.height > THUMBNAIL_DIMENSION
            || v.source_width == 0
            || v.source_height == 0
            || v.rgb.len() != v.width as usize * v.height as usize * 3
        {
            return Err(invalid());
        }
        let format = match v.format.as_str() {
            "png" => 1,
            "jpeg" => 2,
            "webp" => 3,
            "video" => 4,
            "pdf" => 5,
            "svg" => 6,
            _ => return Err(invalid()),
        };
        for n in [
            r.device,
            r.inode,
            r.bytes,
            r.modified_seconds as u64,
            r.modified_nanoseconds as u64,
            r.changed_seconds as u64,
            r.changed_nanoseconds as u64,
        ] {
            b.extend_from_slice(&n.to_le_bytes());
        }
        for n in [v.width, v.height, v.source_width, v.source_height] {
            b.extend_from_slice(&n.to_le_bytes());
        }
        b.push(format);
        b.extend_from_slice(&v.rgb);
    }
    let checksum = hash(&b);
    b.extend_from_slice(&checksum.to_le_bytes());
    if b.len() > MAX_ENTRY {
        return Err(invalid());
    }
    Ok(b)
}
struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> io::Result<&'a [u8]> {
        let end = self
            .at
            .checked_add(n)
            .filter(|e| *e <= self.b.len())
            .ok_or_else(invalid)?;
        let v = &self.b[self.at..end];
        self.at = end;
        Ok(v)
    }
    fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
}
fn same_path(bytes: &[u8], path: &Path) -> bool {
    let mut r = Reader { b: bytes, at: 0 };
    (|| -> io::Result<bool> {
        r.take(24)?;
        let n = u16::from_le_bytes(r.take(2)?.try_into().unwrap()) as usize;
        Ok(n <= 4096 && r.take(n)? == path.as_os_str().as_encoded_bytes())
    })()
    .unwrap_or(false)
}
fn decode(bytes: &[u8], path: &Path, expected: Option<Revision>) -> io::Result<Option<Bitmap>> {
    if bytes.len() < 8 || bytes.len() > MAX_ENTRY {
        return Err(invalid());
    }
    let len = bytes.len() - 8;
    let checksum = u64::from_le_bytes(bytes[len..].try_into().unwrap());
    if hash(&bytes[..len]) != checksum {
        return Err(invalid());
    }
    let mut r = Reader {
        b: &bytes[..len],
        at: 0,
    };
    if r.take(8)? != MAGIC || r.u32()? != RECIPE || r.u32()? != THUMBNAIL_DIMENSION {
        return Err(invalid());
    }
    let at = r.u64()?;
    let n = u16::from_le_bytes(r.take(2)?.try_into().unwrap()) as usize;
    if n > 4096 {
        return Err(invalid());
    }
    let same = r.take(n)? == path.as_os_str().as_encoded_bytes();
    let kind = r.take(1)?[0];
    if kind == 0 {
        if r.at != len {
            return Err(invalid());
        }
        return Ok(None);
    }
    if kind != 1 {
        return Err(invalid());
    }
    let revision = Revision {
        device: r.u64()?,
        inode: r.u64()?,
        bytes: r.u64()?,
        modified_seconds: r.u64()? as i64,
        modified_nanoseconds: r.u64()? as i64,
        changed_seconds: r.u64()? as i64,
        changed_nanoseconds: r.u64()? as i64,
    };
    let width = r.u32()?;
    let height = r.u32()?;
    let source_width = r.u32()?;
    let source_height = r.u32()?;
    let format = match r.take(1)?[0] {
        1 => "png",
        2 => "jpeg",
        3 => "webp",
        4 => "video",
        5 => "pdf",
        6 => "svg",
        _ => return Err(invalid()),
    };
    if width == 0
        || height == 0
        || width > THUMBNAIL_DIMENSION
        || height > THUMBNAIL_DIMENSION
        || source_width == 0
        || source_height == 0
        || !(0..1_000_000_000).contains(&revision.modified_nanoseconds)
        || !(0..1_000_000_000).contains(&revision.changed_nanoseconds)
    {
        return Err(invalid());
    }
    let rgb = r.take(width as usize * height as usize * 3)?;
    if r.at != len {
        return Err(invalid());
    }
    if expected.is_none()
        || !same
        || expected.is_some_and(|e| e != revision)
        || at > now()
        || now().saturating_sub(at) > TTL
    {
        return Ok(None);
    }
    Ok(Some(Bitmap {
        width,
        height,
        source_width,
        source_height,
        format: format.into(),
        rgb: rgb.to_vec(),
        source: Some(revision),
    }))
}

pub fn directory() -> PathBuf {
    files_core::theme::config_dir().join("thumbnail-cache-v3")
}

#[cfg(unix)]
mod storage {
    use super::*;
    use std::{
        ffi::CString,
        fs::File,
        io::{Read, Seek, SeekFrom, Write},
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::fs::MetadataExt,
        },
        path::Component,
    };
    fn wrap(fd: i32) -> io::Result<File> {
        if fd < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    }
    fn name(s: &str) -> CString {
        CString::new(s).unwrap()
    }
    fn private(file: &File, dir: bool) -> io::Result<()> {
        let m = file.metadata()?;
        if m.uid() != unsafe { libc::geteuid() }
            || m.mode() & 0o077 != 0
            || if dir {
                !m.is_dir()
            } else {
                !m.is_file() || m.nlink() != 1 || m.len() > MAX_ENTRY as u64
            }
        {
            return Err(invalid());
        }
        Ok(())
    }
    pub(super) fn directory(path: &Path, create: bool) -> io::Result<File> {
        if !valid_path(path) {
            return Err(invalid());
        }
        let mut dir = wrap(unsafe {
            libc::open(
                c"/".as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        })?;
        for part in path.components() {
            match part {
                Component::RootDir => (),
                Component::Normal(part) => {
                    let c = CString::new(part.as_encoded_bytes()).map_err(|_| invalid())?;
                    let flags =
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
                    let mut fd = unsafe { libc::openat(dir.as_raw_fd(), c.as_ptr(), flags) };
                    if fd < 0
                        && create
                        && io::Error::last_os_error().kind() == io::ErrorKind::NotFound
                    {
                        if unsafe { libc::mkdirat(dir.as_raw_fd(), c.as_ptr(), 0o700) } != 0
                            && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists
                        {
                            return Err(io::Error::last_os_error());
                        }
                        fd = unsafe { libc::openat(dir.as_raw_fd(), c.as_ptr(), flags) };
                    }
                    dir = wrap(fd)?;
                }
                _ => return Err(invalid()),
            }
        }
        private(&dir, true)?;
        Ok(dir)
    }
    fn read(dir: &File, path: &Path) -> io::Result<Option<Vec<u8>>> {
        let c = name(&slot(path));
        let fd = unsafe {
            libc::openat(
                dir.as_raw_fd(),
                c.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        let file = match wrap(fd) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        private(&file, false)?;
        let mut bytes = Vec::with_capacity(MAX_ENTRY + 1);
        file.take(MAX_ENTRY as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > MAX_ENTRY {
            return Err(invalid());
        }
        Ok(Some(bytes))
    }
    pub fn load(root: &Path, path: &Path, revision: Revision) -> io::Result<Option<Bitmap>> {
        let dir = match directory(root, false) {
            Ok(d) => d,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        match read(&dir, path)? {
            Some(b) => decode(&b, path, Some(revision)),
            None => Ok(None),
        }
    }
    pub fn save(
        root: &Path,
        path: &Path,
        bitmap: Option<&Bitmap>,
        cancel: &AtomicBool,
    ) -> io::Result<()> {
        let dir = match directory(root, bitmap.is_some()) {
            Ok(dir) => dir,
            Err(error) if bitmap.is_none() && error.kind() == io::ErrorKind::NotFound => {
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let flags =
            libc::O_RDWR | libc::O_CREAT | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC;
        let lock =
            wrap(unsafe { libc::openat(dir.as_raw_fd(), c"cache.lock".as_ptr(), flags, 0o600) })?;
        private(&lock, false)?;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::WouldBlock {
                return Ok(());
            }
            return Err(error);
        }
        // Never truncate an unexpected/future record, nor replace an unrelated
        // source on invalidation. A normal write may evict a colliding slot.
        if let Some(bytes) = read(&dir, path)? {
            let _ = decode(&bytes, path, None)?;
            if bitmap.is_none() && !same_path(&bytes, path) {
                return Ok(());
            }
        } else if bitmap.is_none() {
            return Ok(());
        }
        if cancel.load(Ordering::Acquire) {
            return Ok(());
        }
        let bytes = encode(path, bitmap)?;
        let mut pending =
            wrap(unsafe { libc::openat(dir.as_raw_fd(), c"pending.bin".as_ptr(), flags, 0o600) })?;
        private(&pending, false)?;
        pending.set_len(0)?;
        pending.seek(SeekFrom::Start(0))?;
        pending.write_all(&bytes)?;
        if !cancel.load(Ordering::Acquire) {
            let target = name(&slot(path));
            if unsafe {
                libc::renameat(
                    dir.as_raw_fd(),
                    c"pending.bin".as_ptr(),
                    dir.as_raw_fd(),
                    target.as_ptr(),
                )
            } != 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        // No fsync: records are disposable, checksummed and atomically replaced.
        // A failed/abandoned write retains at most one bounded pending file.
        Ok(())
    }
    /// Hand bounded tool output (an rsvg-convert PNG) to the helper decoder
    /// through one owned 0600 file in the private directory, removed after.
    pub fn with_scratch<T>(
        root: &Path,
        bytes: &[u8],
        decode: impl FnOnce(&Path, Revision) -> io::Result<T>,
    ) -> io::Result<T> {
        if bytes.len() > SCRATCH_MAX {
            return Err(invalid());
        }
        let dir = directory(root, true)?;
        let file_name = format!("render-{}.png", std::process::id());
        let c = name(&file_name);
        let mut file = wrap(unsafe {
            libc::openat(
                dir.as_raw_fd(),
                c.as_ptr(),
                libc::O_RDWR
                    | libc::O_CREAT
                    | libc::O_TRUNC
                    | libc::O_NOFOLLOW
                    | libc::O_NONBLOCK
                    | libc::O_CLOEXEC,
                0o600,
            )
        })?;
        let m = file.metadata()?;
        if m.uid() != unsafe { libc::geteuid() } || m.mode() & 0o077 != 0 || m.nlink() != 1 {
            return Err(invalid());
        }
        let result = (|| {
            file.write_all(bytes)?;
            let revision = Revision::from_file(&file)?;
            decode(&root.join(&file_name), revision)
        })();
        drop(file);
        // Only this process's own scratch file; never a directory.
        unsafe { libc::unlinkat(dir.as_raw_fd(), c.as_ptr(), 0) };
        result
    }
}
/// Largest tool output accepted for helper decoding.
pub const SCRATCH_MAX: usize = 512 * 1024;
/// See [`storage::with_scratch`].
pub fn with_scratch<T>(
    bytes: &[u8],
    decode: impl FnOnce(&Path, Revision) -> io::Result<T>,
) -> io::Result<T> {
    #[cfg(unix)]
    {
        storage::with_scratch(&directory(), bytes, decode)
    }
    #[cfg(not(unix))]
    {
        let _ = (bytes, decode);
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}
pub fn load(path: &Path, revision: Revision) -> io::Result<Option<Bitmap>> {
    #[cfg(unix)]
    {
        storage::load(&directory(), path, revision)
    }
    #[cfg(not(unix))]
    {
        let _ = (path, revision);
        Ok(None)
    }
}
pub fn save(path: &Path, bitmap: &Bitmap, cancel: &AtomicBool) -> io::Result<()> {
    #[cfg(unix)]
    {
        storage::save(&directory(), path, Some(bitmap), cancel)
    }
    #[cfg(not(unix))]
    {
        let _ = (path, bitmap, cancel);
        Ok(())
    }
}
pub fn invalidate(path: &Path, cancel: &AtomicBool) -> io::Result<()> {
    #[cfg(unix)]
    {
        storage::save(&directory(), path, None, cancel)
    }
    #[cfg(not(unix))]
    {
        let _ = (path, cancel);
        Ok(())
    }
}

#[cfg(all(test, unix))]
pub(crate) fn retained_test_directory(prefix: &str) -> PathBuf {
    use std::{ffi::CString, os::fd::AsRawFd, sync::atomic::AtomicU64};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let parent =
        PathBuf::from(std::env::var_os("TMPDIR").expect("Thumbnail tests require private TMPDIR"));
    let fd = storage::directory(&parent, false).unwrap();
    let name = format!(
        "{prefix}-{}-{}-{}",
        std::process::id(),
        now(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let c = CString::new(name.as_bytes()).unwrap();
    assert_eq!(
        unsafe { libc::mkdirat(fd.as_raw_fd(), c.as_ptr(), 0o700) },
        0
    );
    parent.join(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn revision(n: u64) -> Revision {
        Revision {
            device: 1,
            inode: n,
            bytes: 3,
            modified_seconds: 4,
            modified_nanoseconds: 5,
            changed_seconds: 6,
            changed_nanoseconds: 7,
        }
    }
    fn bitmap(n: u64) -> Bitmap {
        Bitmap {
            width: 1,
            height: 1,
            source_width: 1,
            source_height: 1,
            format: "png".into(),
            rgb: vec![1, 2, 3],
            source: Some(revision(n)),
        }
    }
    #[test]
    fn codec_requires_full_path_revision_recipe_checksum_and_bounded_rgb() {
        let p = Path::new("/fixture/image.png");
        let bytes = encode(p, Some(&bitmap(1))).unwrap();
        assert_eq!(
            decode(&bytes, p, Some(revision(1))).unwrap().unwrap().rgb,
            [1, 2, 3]
        );
        assert!(decode(&bytes, p, Some(revision(2))).unwrap().is_none());
        assert!(
            decode(&bytes, Path::new("/fixture/other.png"), Some(revision(1)))
                .unwrap()
                .is_none()
        );
        let mut bad = bytes.clone();
        bad[10] ^= 1;
        assert!(decode(&bad, p, Some(revision(1))).is_err());
        let with_header = |at: usize, value: &[u8]| {
            let mut record = bytes.clone();
            record[at..at + value.len()].copy_from_slice(value);
            let end = record.len() - 8;
            let checksum = hash(&record[..end]);
            record[end..].copy_from_slice(&checksum.to_le_bytes());
            record
        };
        let future_recipe = with_header(8, &(RECIPE + 1).to_le_bytes());
        assert!(decode(&future_recipe, p, Some(revision(1))).is_err());
        let future_timestamp = with_header(16, &(now() + 3600).to_le_bytes());
        assert!(
            decode(&future_timestamp, p, Some(revision(1)))
                .unwrap()
                .is_none()
        );
        let expired = with_header(16, &(now() - TTL - 1).to_le_bytes());
        assert!(decode(&expired, p, Some(revision(1))).unwrap().is_none());
        assert!(decode(&bytes[..bytes.len() - 1], p, Some(revision(1))).is_err());
        let mut huge = bitmap(1);
        huge.width = THUMBNAIL_DIMENSION + 1;
        assert!(encode(p, Some(&huge)).is_err());
        let tombstone = encode(p, None).unwrap();
        assert!(decode(&tombstone, p, Some(revision(1))).unwrap().is_none());
    }
    #[cfg(unix)]
    #[test]
    fn persisted_slot_reloads_after_reopen_and_rejects_replaced_or_missing_source() {
        let root = retained_test_directory("thumbnail-disk");
        let dir = root.join("cache");
        let path = root.join("image.png");
        std::fs::write(&path, b"red").unwrap();
        let (source, revision) = files_core::media_source::open(&path).unwrap();
        let mut image = bitmap(1);
        image.source = Some(revision);
        let cancel = AtomicBool::new(false);
        storage::save(&dir, &path, Some(&image), &cancel).unwrap();
        assert_eq!(
            storage::load(&dir, &path, revision).unwrap().unwrap().rgb,
            [1, 2, 3]
        );
        std::fs::rename(&path, root.join("original-retained.png")).unwrap();
        std::fs::write(&path, b"blu").unwrap();
        let (_, replacement) = files_core::media_source::open(&path).unwrap();
        assert_ne!(revision, replacement);
        assert!(storage::load(&dir, &path, replacement).unwrap().is_none());
        assert!(revision.revalidate(&source, &path).is_err());
        std::fs::write(
            root.join("retained-cache-record.bin"),
            std::fs::read(dir.join(slot(&path))).unwrap(),
        )
        .unwrap();
        storage::save(&dir, &path, None, &cancel).unwrap();
        assert!(storage::load(&dir, &path, revision).unwrap().is_none());
        assert_eq!(
            std::fs::read(root.join("original-retained.png")).unwrap(),
            b"red"
        );
    }
    #[cfg(unix)]
    #[test]
    fn cache_refuses_symlink_slots_and_preserves_incompatible_records() {
        use std::os::unix::fs::{DirBuilderExt, symlink};
        let root = retained_test_directory("thumbnail-links");
        let dir = root.join("cache");
        std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        let original = root.join("retained-target");
        std::fs::write(&original, b"private sentinel").unwrap();
        let path = root.join("a.png");
        symlink(&original, dir.join(slot(&path))).unwrap();
        assert!(storage::load(&dir, &path, revision(1)).is_err());
        assert!(storage::save(&dir, &path, Some(&bitmap(1)), &AtomicBool::new(false)).is_err());
        assert_eq!(std::fs::read(&original).unwrap(), b"private sentinel");
        let second = root.join("other-cache");
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&second)
            .unwrap();
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(second.join(slot(&path)))
            .unwrap();
        use std::io::Write;
        file.write_all(b"future cache data").unwrap();
        drop(file);
        assert!(storage::save(&second, &path, Some(&bitmap(1)), &AtomicBool::new(false)).is_err());
        assert_eq!(
            std::fs::read(second.join(slot(&path))).unwrap(),
            b"future cache data"
        );
    }
}
