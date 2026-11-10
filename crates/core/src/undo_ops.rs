//! Identity receipts and verification for undoing operations Files performed.
//!
//! Nothing in this module deletes anything. Undoing a creation hands each
//! verified object to the trash engine (`TrashContext::trash_expected`), and
//! undoing a move renames the object back with `RENAME_NOREPLACE`. Every step
//! re-verifies the (device, inode, type) identity recorded when the operation
//! completed, never follows a symlink and never descends into one.
//!
//! "Unchanged since creation" is proven with the inode change time: every
//! write, truncation, chmod, rename into or out of a directory and link-count
//! change sets `st_ctime` to the current time, and user space cannot set it
//! back. A [`fence`] taken when the operation completes is a time strictly
//! below every later change, so an object (or any entry of a copied tree)
//! whose ctime is newer than the fence was touched afterwards and is kept.
//! A backwards wall-clock jump can defeat this proof; see docs/UNDO.md.
use std::{
    io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

/// Entries examined per undo of copied trees; beyond it the root is kept.
pub const SCAN_BUDGET: u64 = 20_000;
/// Deepest copied directory examined (the copy engine's own maximum).
pub const MAX_DEPTH: usize = 128;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Stamp {
    pub sec: i64,
    pub nsec: i64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity {
    pub device: u64,
    pub inode: u64,
    /// `st_mode & S_IFMT`.
    pub kind: u32,
}
/// An object this operation created, as reported by the operation itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Created {
    /// Canonical parent plus the published name.
    pub path: PathBuf,
    pub identity: Identity,
    pub size: u64,
    pub mtime: Stamp,
    /// True for a copied directory, whose entries the same job created. A new
    /// empty folder is `false` and must still be empty to be undone.
    pub tree: bool,
}
/// One root moved inside a filesystem by the native move engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Moved {
    /// Where the object was before the move (canonical parent + name).
    pub from: PathBuf,
    /// Where the move published it (canonical parent + name).
    pub to: PathBuf,
    pub identity: Identity,
    pub from_parent: Identity,
    pub to_parent: Identity,
}
/// Why one object was not acted on. `Cancelled` stops the whole batch.
#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    Skip(String),
    Cancelled,
}
impl Refusal {
    fn skip(text: impl Into<String>) -> Self {
        Self::Skip(text.into())
    }
}
pub const MODIFIED: &str = "modificado después; se conserva";

fn cancelled(cancel: &AtomicBool) -> Result<(), Refusal> {
    if cancel.load(Ordering::Acquire) {
        Err(Refusal::Cancelled)
    } else {
        Ok(())
    }
}
fn split(path: &Path) -> io::Result<(&Path, &std::ffi::OsStr)> {
    let parent = path
        .parent()
        .filter(|p| p.is_absolute())
        .ok_or_else(|| io::Error::other("ruta sin carpeta absoluta"))?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::other("ruta sin nombre"))?;
    Ok((parent, name))
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::{
        ffi::{CString, OsStr},
        fs::File,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::ffi::OsStrExt,
        },
    };
    pub(super) fn c(name: &OsStr) -> io::Result<CString> {
        CString::new(name.as_bytes()).map_err(|_| io::Error::other("NUL en ruta"))
    }
    fn file(fd: libc::c_int) -> io::Result<File> {
        if fd < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    }
    pub(super) fn identity(s: &libc::stat) -> Identity {
        Identity {
            device: s.st_dev,
            inode: s.st_ino,
            kind: s.st_mode & libc::S_IFMT,
        }
    }
    fn ctime(s: &libc::stat) -> Stamp {
        Stamp {
            sec: s.st_ctime,
            nsec: s.st_ctime_nsec,
        }
    }
    pub(super) fn mtime(s: &libc::stat) -> Stamp {
        Stamp {
            sec: s.st_mtime,
            nsec: s.st_mtime_nsec,
        }
    }
    pub(super) fn now(clock: libc::clockid_t) -> io::Result<Stamp> {
        let mut t = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        if unsafe { libc::clock_gettime(clock, &mut t) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Stamp {
            sec: t.tv_sec,
            nsec: t.tv_nsec,
        })
    }
    /// The final component must be a real directory, never a symlink.
    pub(super) fn open_dir(path: &Path, flags: libc::c_int) -> io::Result<File> {
        file(unsafe {
            libc::open(
                c(path.as_os_str())?.as_ptr(),
                flags | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        })
    }
    pub(super) fn open_child_dir(parent: &File, name: &OsStr) -> io::Result<File> {
        file(unsafe {
            libc::openat(
                parent.as_raw_fd(),
                c(name)?.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        })
    }
    pub(super) fn stat_fd(f: &File) -> io::Result<libc::stat> {
        let mut s = std::mem::MaybeUninit::uninit();
        if unsafe { libc::fstat(f.as_raw_fd(), s.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(unsafe { s.assume_init() })
    }
    pub(super) fn stat_at(parent: &File, name: &OsStr) -> io::Result<libc::stat> {
        let mut s = std::mem::MaybeUninit::uninit();
        if unsafe {
            libc::fstatat(
                parent.as_raw_fd(),
                c(name)?.as_ptr(),
                s.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(unsafe { s.assume_init() })
    }
    /// Names in a directory, without `.`/`..`; `limit` bounds the read.
    pub(super) fn names(dir: &File, limit: u64) -> io::Result<Vec<std::ffi::OsString>> {
        let dup = unsafe { libc::fcntl(dir.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
        if dup < 0 {
            return Err(io::Error::last_os_error());
        }
        let stream = unsafe { libc::fdopendir(dup) };
        if stream.is_null() {
            let error = io::Error::last_os_error();
            unsafe { libc::close(dup) };
            return Err(error);
        }
        unsafe { libc::rewinddir(stream) };
        let mut out = Vec::new();
        let result = loop {
            unsafe { *libc::__errno_location() = 0 };
            let entry = unsafe { libc::readdir(stream) };
            if entry.is_null() {
                let errno = unsafe { *libc::__errno_location() };
                break if errno == 0 {
                    Ok(())
                } else {
                    Err(io::Error::from_raw_os_error(errno))
                };
            }
            let name = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) };
            let bytes = name.to_bytes();
            if bytes == b"." || bytes == b".." {
                continue;
            }
            if out.len() as u64 >= limit {
                break Ok(());
            }
            out.push(OsStr::from_bytes(bytes).to_os_string());
        };
        unsafe { libc::closedir(stream) };
        result.map(|()| out)
    }
    pub(super) fn rename_noreplace(
        from_dir: &File,
        from: &OsStr,
        to_dir: &File,
        to: &OsStr,
    ) -> io::Result<()> {
        if unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                from_dir.as_raw_fd(),
                c(from)?.as_ptr(),
                to_dir.as_raw_fd(),
                c(to)?.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    pub(super) fn is_newer(s: &libc::stat, fence: Stamp) -> bool {
        ctime(s) > fence
    }
}

/// A time strictly below every later inode change. It reads the fine wall
/// clock, then waits (at most ~100 ms, normally one tick) until the coarse
/// clock that stamps inode changes has passed it. Call it on a worker, right
/// after the operation's last write and before recording it as undoable.
#[cfg(target_os = "linux")]
pub fn fence() -> io::Result<Stamp> {
    let fence = linux::now(libc::CLOCK_REALTIME)?;
    for _ in 0..100 {
        if linux::now(libc::CLOCK_REALTIME_COARSE)? > fence {
            return Ok(fence);
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    Err(io::Error::other(
        "El reloj del sistema no avanzó; la operación no se podrá deshacer",
    ))
}
#[cfg(not(target_os = "linux"))]
pub fn fence() -> io::Result<Stamp> {
    Err(io::Error::other("Deshacer requiere Linux"))
}

#[cfg(target_os = "linux")]
impl Created {
    /// Receipt from a stat of the object handle the operation itself held.
    pub fn from_stat(path: PathBuf, stat: &libc::stat, tree: bool) -> Self {
        Self {
            path,
            identity: linux::identity(stat),
            size: stat.st_size as u64,
            mtime: linux::mtime(stat),
            tree: tree && stat.st_mode & libc::S_IFMT == libc::S_IFDIR,
        }
    }
    /// Receipt from an open handle of the created object (`fstat`).
    pub fn from_file(path: PathBuf, file: &std::fs::File, tree: bool) -> io::Result<Self> {
        Ok(Self::from_stat(path, &linux::stat_fd(file)?, tree))
    }
    /// Receipt for an object just created by name (`symlink(2)`, `mkdirat`)
    /// where no handle exists. Never follows the final component.
    pub fn just_created(path: &Path, tree: bool) -> io::Result<Self> {
        let (parent, name) = split(path)?;
        let parent = linux::open_dir(&parent.canonicalize()?, libc::O_PATH)?;
        let stat = linux::stat_at(&parent, name)?;
        let canonical = descriptor_dir(&parent)?.join(name);
        Ok(Self::from_stat(canonical, &stat, tree))
    }
}
#[cfg(target_os = "linux")]
fn descriptor_dir(dir: &std::fs::File) -> io::Result<PathBuf> {
    use std::os::fd::AsRawFd;
    let link = std::fs::read_link(format!("/proc/self/fd/{}", dir.as_raw_fd()))?;
    let check = linux::open_dir(&link, libc::O_PATH)?;
    if linux::identity(&linux::stat_fd(&check)?) != linux::identity(&linux::stat_fd(dir)?) {
        return Err(io::Error::other("La carpeta cambió al resolverla"));
    }
    Ok(link)
}

/// Re-verifies that `created` is still exactly what the operation made:
/// same (device, inode, type) at the same path, no inode change after
/// `fence`, the same size and mtime for a file, no entries for a new folder
/// and, for a copied tree, every entry unchanged within `budget` entries.
/// `Ok` is the only answer that permits sending the object to the trash.
#[cfg(target_os = "linux")]
pub fn verify(
    created: &Created,
    fence: Stamp,
    budget: &mut u64,
    cancel: &AtomicBool,
) -> Result<(), Refusal> {
    cancelled(cancel)?;
    let (parent, name) = split(&created.path).map_err(|e| Refusal::skip(e.to_string()))?;
    let parent = linux::open_dir(parent, libc::O_PATH)
        .map_err(|e| Refusal::skip(format!("carpeta no disponible ({e}); se conserva")))?;
    let stat = match linux::stat_at(&parent, name) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(Refusal::skip("ya no está en su ubicación"));
        }
        Err(e) => return Err(Refusal::skip(format!("no se pudo verificar: {e}"))),
    };
    if linux::identity(&stat) != created.identity {
        return Err(Refusal::skip("reemplazado por otro objeto; se conserva"));
    }
    let newer = linux::is_newer(&stat, fence);
    if newer && created.identity.kind != libc::S_IFDIR {
        return Err(Refusal::skip(MODIFIED));
    }
    match created.identity.kind {
        libc::S_IFREG => {
            if stat.st_size as u64 != created.size || linux::mtime(&stat) != created.mtime {
                return Err(Refusal::skip(MODIFIED));
            }
            Ok(())
        }
        libc::S_IFLNK => Ok(()),
        libc::S_IFDIR => {
            let dir = linux::open_child_dir(&parent, name)
                .map_err(|e| Refusal::skip(format!("no se pudo abrir ({e}); se conserva")))?;
            if linux::identity(&linux::stat_fd(&dir).map_err(|e| Refusal::skip(e.to_string()))?)
                != created.identity
            {
                return Err(Refusal::skip("reemplazado por otro objeto; se conserva"));
            }
            if !created.tree {
                let found = linux::names(&dir, 1).map_err(|e| Refusal::skip(e.to_string()))?;
                if !found.is_empty() {
                    return Err(Refusal::skip("contiene elementos nuevos; se conserva"));
                }
            }
            if newer {
                return Err(Refusal::skip(MODIFIED));
            }
            if !created.tree {
                return Ok(());
            }
            scan(dir, created.identity.device, fence, budget, cancel)
        }
        _ => Err(Refusal::skip("tipo de objeto no admitido; se conserva")),
    }
}
#[cfg(not(target_os = "linux"))]
pub fn verify(_: &Created, _: Stamp, _: &mut u64, _: &AtomicBool) -> Result<(), Refusal> {
    Err(Refusal::skip("Deshacer requiere Linux"))
}

/// Depth-first, never through symlinks or into another filesystem. Every
/// entry costs one unit of `budget`; exhausting it refuses the whole root.
#[cfg(target_os = "linux")]
fn scan(
    root: std::fs::File,
    device: u64,
    fence: Stamp,
    budget: &mut u64,
    cancel: &AtomicBool,
) -> Result<(), Refusal> {
    let mut stack = vec![(root, 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let names = linux::names(&dir, budget.saturating_add(1)).map_err(|e| {
            Refusal::skip(format!("no se pudo leer el contenido ({e}); se conserva"))
        })?;
        for name in names {
            cancelled(cancel)?;
            if *budget == 0 {
                return Err(Refusal::skip(format!(
                    "demasiado grande para verificar (más de {SCAN_BUDGET} elementos); se conserva"
                )));
            }
            *budget -= 1;
            let stat = linux::stat_at(&dir, &name)
                .map_err(|e| Refusal::skip(format!("contenido cambiado ({e}); se conserva")))?;
            if stat.st_dev != device {
                return Err(Refusal::skip(
                    "contiene otro sistema de archivos; se conserva",
                ));
            }
            if linux::is_newer(&stat, fence) {
                return Err(Refusal::skip(format!("contenido {MODIFIED}")));
            }
            if stat.st_mode & libc::S_IFMT == libc::S_IFDIR {
                if depth + 1 > MAX_DEPTH {
                    return Err(Refusal::skip(
                        "demasiado profundo para verificar; se conserva",
                    ));
                }
                let child = linux::open_child_dir(&dir, &name)
                    .map_err(|e| Refusal::skip(format!("contenido cambiado ({e}); se conserva")))?;
                let opened = linux::stat_fd(&child).map_err(|e| Refusal::skip(e.to_string()))?;
                if linux::identity(&opened) != linux::identity(&stat) {
                    return Err(Refusal::skip("contenido reemplazado; se conserva"));
                }
                stack.push((child, depth + 1));
            }
        }
    }
    Ok(())
}

/// Moves `moved` back to where it came from (`reverse == false`, Undo) or
/// again to where the move had put it (`reverse == true`, Redo). Both parent
/// directories and the object must still have their recorded identities;
/// the rename is `RENAME_NOREPLACE`, so an occupied name is never replaced.
/// Returns the object's new path.
#[cfg(target_os = "linux")]
pub fn move_back(moved: &Moved, reverse: bool, cancel: &AtomicBool) -> Result<PathBuf, Refusal> {
    let (current, current_parent, target, target_parent) = if reverse {
        (&moved.from, moved.from_parent, &moved.to, moved.to_parent)
    } else {
        (&moved.to, moved.to_parent, &moved.from, moved.from_parent)
    };
    cancelled(cancel)?;
    let (source_dir, name) = split(current).map_err(|e| Refusal::skip(e.to_string()))?;
    let (target_dir, target_name) = split(target).map_err(|e| Refusal::skip(e.to_string()))?;
    let open = |path: &Path, expected: Identity, what: &str| {
        let dir = linux::open_dir(path, libc::O_PATH)
            .map_err(|e| Refusal::skip(format!("{what} no disponible ({e}); no se movió")))?;
        let found = linux::stat_fd(&dir).map_err(|e| Refusal::skip(e.to_string()))?;
        if linux::identity(&found) != expected {
            return Err(Refusal::skip(format!(
                "{what} ya no es la misma; no se movió"
            )));
        }
        Ok(dir)
    };
    let from = open(source_dir, current_parent, "la carpeta actual")?;
    let to = open(target_dir, target_parent, "la carpeta de destino")?;
    let check = || -> Result<(), Refusal> {
        match linux::stat_at(&from, name) {
            Ok(s) if linux::identity(&s) == moved.identity => Ok(()),
            Ok(_) => Err(Refusal::skip("reemplazado por otro objeto; se conserva")),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                Err(Refusal::skip("ya no está en su ubicación"))
            }
            Err(e) => Err(Refusal::skip(format!("no se pudo verificar: {e}"))),
        }
    };
    check()?;
    match linux::stat_at(&to, target_name) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Ok(_) => {
            return Err(Refusal::skip(
                "el nombre de destino está ocupado; no se reemplazó",
            ));
        }
        Err(e) => {
            return Err(Refusal::skip(format!(
                "no se pudo verificar el destino: {e}"
            )));
        }
    }
    cancelled(cancel)?;
    check()?;
    match linux::rename_noreplace(&from, name, &to, target_name) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            return Err(Refusal::skip(
                "el nombre de destino está ocupado; no se reemplazó",
            ));
        }
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
            return Err(Refusal::skip("no se puede deshacer entre discos"));
        }
        Err(e) => return Err(Refusal::skip(format!("no se movió: {e}"))),
    }
    match linux::stat_at(&to, target_name) {
        Ok(s) if linux::identity(&s) == moved.identity => Ok(target.clone()),
        _ => Err(Refusal::skip(
            "identidad posterior al movimiento no confirmada; revisa ambas carpetas",
        )),
    }
}
#[cfg(not(target_os = "linux"))]
pub fn move_back(_: &Moved, _: bool, _: &AtomicBool) -> Result<PathBuf, Refusal> {
    Err(Refusal::skip("Deshacer requiere Linux"))
}

/// Identity of a directory reached by path (final component not followed).
#[cfg(target_os = "linux")]
pub fn directory_identity(path: &Path) -> io::Result<Identity> {
    Ok(linux::identity(&linux::stat_fd(&linux::open_dir(
        path,
        libc::O_PATH,
    )?)?))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;

    /// Exclusive, retained directory in the invocation's private TMPDIR.
    fn fixture(label: &str) -> PathBuf {
        use std::sync::atomic::AtomicU64;
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "files-undo-ops-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        path.canonicalize().unwrap()
    }
    fn go() -> AtomicBool {
        AtomicBool::new(false)
    }

    #[test]
    fn new_folder_must_stay_empty_and_unreplaced() {
        let d = fixture("folder");
        fs::create_dir(d.join("new")).unwrap();
        let created = Created::just_created(&d.join("new"), false).unwrap();
        let fence = fence().unwrap();
        let mut budget = SCAN_BUDGET;
        assert_eq!(verify(&created, fence, &mut budget, &go()), Ok(()));
        fs::write(d.join("new/user.txt"), b"mine").unwrap();
        assert!(matches!(
            verify(&created, fence, &mut budget, &go()),
            Err(Refusal::Skip(_))
        ));
        fs::rename(d.join("new"), d.join("moved-aside")).unwrap();
        fs::create_dir(d.join("new")).unwrap();
        assert_eq!(
            verify(&created, fence, &mut budget, &go()),
            Err(Refusal::Skip(
                "reemplazado por otro objeto; se conserva".into()
            ))
        );
        assert_eq!(fs::read(d.join("moved-aside/user.txt")).unwrap(), b"mine");
    }

    #[test]
    fn edited_file_and_newer_tree_entries_are_refused() {
        let d = fixture("tree");
        fs::create_dir_all(d.join("copy/sub")).unwrap();
        fs::write(d.join("copy/sub/a"), b"a").unwrap();
        fs::write(d.join("file"), b"abc").unwrap();
        let tree = Created::just_created(&d.join("copy"), true).unwrap();
        let file = Created::just_created(&d.join("file"), false).unwrap();
        let fence = fence().unwrap();
        let mut budget = SCAN_BUDGET;
        assert_eq!(verify(&tree, fence, &mut budget, &go()), Ok(()));
        assert_eq!(verify(&file, fence, &mut budget, &go()), Ok(()));
        // Budget: three entries examined (sub, sub/a) plus nothing for the file.
        assert_eq!(budget, SCAN_BUDGET - 2);
        let mut tiny = 1;
        assert!(matches!(
            verify(&tree, fence, &mut tiny, &go()),
            Err(Refusal::Skip(s)) if s.contains("demasiado grande")
        ));
        fs::write(d.join("copy/sub/a"), b"edited").unwrap();
        fs::write(d.join("file"), b"xyz").unwrap();
        assert!(matches!(
            verify(&tree, fence, &mut budget, &go()),
            Err(Refusal::Skip(s)) if s.contains(MODIFIED)
        ));
        assert_eq!(
            verify(&file, fence, &mut budget, &go()),
            Err(Refusal::Skip(MODIFIED.into()))
        );
        assert_eq!(
            verify(&tree, fence, &mut budget, &AtomicBool::new(true)),
            Err(Refusal::Cancelled)
        );
    }

    #[test]
    fn symlink_is_verified_as_a_link_without_following() {
        let d = fixture("link");
        fs::create_dir(d.join("target")).unwrap();
        symlink(d.join("target"), d.join("link")).unwrap();
        let link = Created::just_created(&d.join("link"), true).unwrap();
        assert_eq!(link.identity.kind, libc::S_IFLNK);
        assert!(!link.tree);
        let fence = fence().unwrap();
        let mut budget = SCAN_BUDGET;
        assert_eq!(verify(&link, fence, &mut budget, &go()), Ok(()));
        assert_eq!(budget, SCAN_BUDGET);
    }

    #[test]
    fn move_back_never_replaces_and_checks_identity() {
        let d = fixture("move");
        fs::create_dir(d.join("a")).unwrap();
        fs::create_dir(d.join("b")).unwrap();
        fs::write(d.join("a/item"), b"payload").unwrap();
        let identity = Created::just_created(&d.join("a/item"), false)
            .unwrap()
            .identity;
        fs::rename(d.join("a/item"), d.join("b/item")).unwrap();
        let moved = Moved {
            from: d.join("a/item"),
            to: d.join("b/item"),
            identity,
            from_parent: directory_identity(&d.join("a")).unwrap(),
            to_parent: directory_identity(&d.join("b")).unwrap(),
        };
        fs::write(d.join("a/item"), b"occupant").unwrap();
        assert!(matches!(
            move_back(&moved, false, &go()),
            Err(Refusal::Skip(s)) if s.contains("ocupado")
        ));
        assert_eq!(fs::read(d.join("a/item")).unwrap(), b"occupant");
        assert_eq!(fs::read(d.join("b/item")).unwrap(), b"payload");
        fs::rename(d.join("a/item"), d.join("occupant-aside")).unwrap();
        assert_eq!(move_back(&moved, false, &go()), Ok(d.join("a/item")));
        assert_eq!(fs::read(d.join("a/item")).unwrap(), b"payload");
        assert_eq!(move_back(&moved, true, &go()), Ok(d.join("b/item")));
        assert_eq!(fs::read(d.join("b/item")).unwrap(), b"payload");
        // A different inode at the moved path is never moved.
        fs::rename(d.join("b/item"), d.join("payload-aside")).unwrap();
        fs::write(d.join("b/item"), b"stranger").unwrap();
        assert!(matches!(
            move_back(&moved, false, &go()),
            Err(Refusal::Skip(s)) if s.contains("reemplazado")
        ));
        assert_eq!(fs::read(d.join("b/item")).unwrap(), b"stranger");
        assert_eq!(fs::read(d.join("payload-aside")).unwrap(), b"payload");
    }
}
