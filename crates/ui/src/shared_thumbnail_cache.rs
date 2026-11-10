//! Freedesktop shared thumbnail cache (`$XDG_CACHE_HOME/thumbnails`).
//!
//! Read tier: `large/` then `normal/` entries named `MD5(file-uri).png` are
//! accepted only when `Thumb::URI` and `Thumb::MTime` (and `Thumb::Size` when
//! present) match the verified source revision. The cache is untrusted: only
//! chunk headers and small text chunks are read here, under fixed byte and
//! dimension limits; pixels are decoded by the ephemeral helper process.
//!
//! Write tier: thumbnails Files generated are written to `normal/` (128 px)
//! through a 0600 temporary file in the same 0700 directory and an atomic
//! rename. Nothing is written for sources inside the cache itself, on
//! network/FUSE filesystems (NFS, SMB/CIFS, sshfs, GVfs, ...) or under
//! removable-media mount points. `fail/` is neither read nor written.
//!
//! All IO runs on the thumbnail worker thread, never in draw/UI code.
#![cfg_attr(not(unix), allow(dead_code, unused_imports))]
use crate::image_preview_job::Bitmap;
use files_core::{media_source::Revision, thumbnail_standard as spec};
use std::{
    fs::File,
    io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

/// The standard's root, or `None` under `cfg(test)`: unit tests must inject
/// a private root and can never touch the user's real cache.
pub fn default_root() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .map(|home| home.join(".cache"))
        })?;
    Some(base.join("thumbnails"))
}

/// The canonical URI and cache file name of one source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub uri: String,
    pub name: String,
}

/// `None` for sources inside the cache (thumbnails are never thumbnailed)
/// or without a canonical local URI.
pub fn entry(root: &Path, path: &Path) -> Option<Entry> {
    if !root.is_absolute() || path.starts_with(root) {
        return None;
    }
    let uri = spec::file_uri(path)?;
    Some(Entry {
        name: spec::cache_name(&uri),
        uri,
    })
}

/// Removable-media mount roots used by udisks; never written to the cache.
const REMOVABLE_ROOTS: [&str; 2] = ["/run/media", "/media"];

/// Filesystems whose files are never written to the shared cache.
#[cfg(target_os = "linux")]
fn remote_filesystem(source: &File) -> bool {
    use std::os::fd::AsRawFd;
    const REMOTE: [u64; 13] = [
        0x6969,     // NFS
        0x517B,     // SMB
        0xFF534D42, // CIFS
        0xFE534D42, // SMB2
        0x65735546, // FUSE: sshfs, GVfs, rclone, ...
        0x73757245, // Coda
        0x5346414F, // AFS
        0x6B414653, // kAFS
        0x00C36400, // Ceph
        0x01021997, // 9p
        0x564C,     // NCP
        0x7461636F, // OCFS2
        0x01161970, // GFS2
    ];
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstatfs(source.as_raw_fd(), &mut st) } != 0 {
        // Unknown filesystem: do not publish.
        return true;
    }
    REMOTE.contains(&((st.f_type as u64) & 0xFFFF_FFFF))
}
#[cfg(not(target_os = "linux"))]
fn remote_filesystem(_: &File) -> bool {
    true
}

/// Whether a thumbnail of `path` may be published to the shared cache.
pub fn writable_source(root: &Path, path: &Path, source: &File) -> bool {
    if path.starts_with(root)
        || REMOVABLE_ROOTS.iter().any(|r| path.starts_with(r))
        || remote_filesystem(source)
    {
        return false;
    }
    // A symlinked parent can still lead into the cache: compare real paths.
    match (
        path.parent().and_then(|p| std::fs::canonicalize(p).ok()),
        std::fs::canonicalize(root),
    ) {
        (Some(parent), Ok(real_root)) => !parent.starts_with(real_root),
        (Some(_), Err(_)) => true,
        (None, _) => false,
    }
}

#[cfg(unix)]
mod storage {
    use super::*;
    use std::{
        ffi::CString,
        io::Write,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::fs::{MetadataExt, PermissionsExt},
        },
    };
    fn wrap(fd: i32) -> io::Result<File> {
        if fd < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    }
    fn cname(name: &str) -> io::Result<CString> {
        CString::new(name).map_err(|_| io::Error::other("Nombre de caché no válido"))
    }
    fn owned_dir(dir: &File, strict: bool) -> io::Result<()> {
        let m = dir.metadata()?;
        if !m.is_dir() || m.uid() != unsafe { libc::geteuid() } || (strict && m.mode() & 0o022 != 0)
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Directorio de miniaturas compartidas ajeno o escribible por otros",
            ));
        }
        Ok(())
    }
    /// One component below `parent`, never following a symlink; created 0700.
    fn subdirectory(parent: &File, name: &str, create: bool) -> io::Result<File> {
        let c = cname(name)?;
        let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        let mut fd = unsafe { libc::openat(parent.as_raw_fd(), c.as_ptr(), flags) };
        if fd < 0 && create && io::Error::last_os_error().kind() == io::ErrorKind::NotFound {
            if unsafe { libc::mkdirat(parent.as_raw_fd(), c.as_ptr(), 0o700) } != 0
                && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists
            {
                return Err(io::Error::last_os_error());
            }
            fd = unsafe { libc::openat(parent.as_raw_fd(), c.as_ptr(), flags) };
        }
        let dir = wrap(fd)?;
        owned_dir(&dir, create)?;
        Ok(dir)
    }
    fn open_path_dir(path: &Path) -> io::Result<File> {
        let c = CString::new(path.as_os_str().as_encoded_bytes())
            .map_err(|_| io::Error::other("Ruta de caché no válida"))?;
        wrap(unsafe {
            libc::open(
                c.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        })
    }
    /// The root's own parent may be a user symlink (e.g. a relocated
    /// `~/.cache`); the `thumbnails` component and below never follow links.
    fn root(root: &Path, create: bool) -> io::Result<File> {
        let (Some(parent), Some(name)) = (root.parent(), root.file_name()) else {
            return Err(io::Error::other("Raíz de miniaturas no válida"));
        };
        let name = name
            .to_str()
            .ok_or_else(|| io::Error::other("Raíz de miniaturas no válida"))?;
        subdirectory(&open_path_dir(parent)?, name, create)
    }

    pub fn lookup(
        root_path: &Path,
        entry: &Entry,
        revision: Revision,
    ) -> io::Result<Option<(PathBuf, Revision)>> {
        let root = match root(root_path, false) {
            Ok(dir) => dir,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        for (size_dir, side) in [("large", spec::LARGE_SIZE), ("normal", spec::NORMAL_SIZE)] {
            let dir = match subdirectory(&root, size_dir, false) {
                Ok(dir) => dir,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };
            let c = cname(&entry.name)?;
            let fd = unsafe {
                libc::openat(
                    dir.as_raw_fd(),
                    c.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
                )
            };
            let mut file = match wrap(fd) {
                Ok(file) => file,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                // A symlinked entry (ELOOP) or unreadable entry is untrusted.
                Err(_) => continue,
            };
            let m = file.metadata()?;
            if !m.is_file()
                || m.uid() != unsafe { libc::geteuid() }
                || m.len() > spec::MAX_CACHE_FILE
            {
                continue;
            }
            let Ok(meta) = spec::read_meta(&mut file, m.len(), side) else {
                continue;
            };
            if !meta.matches(&entry.uri, revision.modified_seconds, revision.bytes) {
                continue;
            }
            let cached = Revision::from_metadata(&m)?;
            return Ok(Some((root_path.join(size_dir).join(&entry.name), cached)));
        }
        Ok(None)
    }

    static NEXT: AtomicU64 = AtomicU64::new(0);

    pub fn store(
        root_path: &Path,
        entry: &Entry,
        revision: Revision,
        bitmap: &Bitmap,
        cancel: &AtomicBool,
    ) -> io::Result<()> {
        let (width, height, rgb) =
            spec::fit_rgb(bitmap.width, bitmap.height, &bitmap.rgb, spec::NORMAL_SIZE);
        let png = spec::encode_png(
            width,
            height,
            &rgb,
            &entry.uri,
            revision.modified_seconds,
            revision.bytes,
        )?;
        drop(rgb);
        let dir = subdirectory(&root(root_path, true)?, "normal", true)?;
        let temp = format!(
            "{}.files-{}-{}.tmp",
            entry.name,
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let temp_c = cname(&temp)?;
        let mut file = wrap(unsafe {
            libc::openat(
                dir.as_raw_fd(),
                temp_c.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        })?;
        let written = (|| -> io::Result<()> {
            // Exactly 0600 regardless of umask, as the standard requires.
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            file.write_all(&png)?;
            if cancel.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Miniatura cancelada",
                ));
            }
            let target = cname(&entry.name)?;
            if unsafe {
                libc::renameat(
                    dir.as_raw_fd(),
                    temp_c.as_ptr(),
                    dir.as_raw_fd(),
                    target.as_ptr(),
                )
            } != 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        })();
        if written.is_err() {
            // Remove only the single temporary file this call created.
            unsafe { libc::unlinkat(dir.as_raw_fd(), temp_c.as_ptr(), 0) };
        }
        written
    }
}

/// A validated candidate: the cache PNG path and the revision of that cache
/// file, which the helper re-checks on its own descriptor before decoding.
pub fn lookup(
    root: &Path,
    entry: &Entry,
    revision: Revision,
) -> io::Result<Option<(PathBuf, Revision)>> {
    #[cfg(unix)]
    {
        storage::lookup(root, entry, revision)
    }
    #[cfg(not(unix))]
    {
        let _ = (root, entry, revision);
        Ok(None)
    }
}

/// Publish a generated thumbnail. Callers check [`writable_source`] first.
pub fn store(
    root: &Path,
    entry: &Entry,
    revision: Revision,
    bitmap: &Bitmap,
    cancel: &AtomicBool,
) -> io::Result<()> {
    #[cfg(unix)]
    {
        storage::store(root, entry, revision, bitmap, cancel)
    }
    #[cfg(not(unix))]
    {
        let _ = (root, entry, revision, bitmap, cancel);
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt, symlink};

    fn root(prefix: &str) -> (PathBuf, PathBuf) {
        let base = crate::thumbnail_disk_cache::retained_test_directory(prefix);
        // XDG_CACHE_HOME stand-in inside the private run root.
        let cache = base.join("xdg-cache");
        std::fs::create_dir(&cache).unwrap();
        (base, cache.join("thumbnails"))
    }
    fn bitmap(revision: Revision, side: u32) -> Bitmap {
        Bitmap {
            width: side,
            height: side / 2,
            source_width: side,
            source_height: side / 2,
            format: "png".into(),
            rgb: vec![200; side as usize * (side / 2) as usize * 3],
            source: Some(revision),
        }
    }

    #[test]
    fn default_root_is_disabled_under_test() {
        assert!(default_root().is_none());
    }

    #[test]
    fn atomic_write_has_standard_keys_modes_and_reads_back() {
        let (base, cache) = root("shared-write");
        let source = base.join("a photo é.png");
        std::fs::write(&source, b"fixture").unwrap();
        let (file, revision) = files_core::media_source::open(&source).unwrap();
        let entry = entry(&cache, &source).unwrap();
        assert!(entry.uri.ends_with("/a%20photo%20%C3%A9.png"));
        assert!(writable_source(&cache, &source, &file));
        store(
            &cache,
            &entry,
            revision,
            &bitmap(revision, 160),
            &AtomicBool::new(false),
        )
        .unwrap();
        let dir = cache.join("normal");
        for d in [&cache, &dir] {
            assert_eq!(std::fs::metadata(d).unwrap().mode() & 0o777, 0o700);
        }
        let written = dir.join(&entry.name);
        assert_eq!(std::fs::metadata(&written).unwrap().mode() & 0o777, 0o600);
        // Only the final file remains: the temporary was renamed, not copied.
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, [std::ffi::OsString::from(&entry.name)]);
        let bytes = std::fs::read(&written).unwrap();
        let meta = spec::read_meta(
            &mut std::io::Cursor::new(&bytes),
            bytes.len() as u64,
            spec::NORMAL_SIZE,
        )
        .unwrap();
        assert_eq!((meta.width, meta.height), (128, 64));
        assert_eq!(meta.uri.as_deref(), Some(entry.uri.as_bytes()));
        assert_eq!(meta.mtime, Some(revision.modified_seconds));
        assert_eq!(meta.size, Some(revision.bytes));
        let (path, cached) = lookup(&cache, &entry, revision).unwrap().unwrap();
        assert_eq!(path, written);
        assert_eq!(cached.inode, std::fs::metadata(&written).unwrap().ino());
        assert_eq!(std::fs::read(&source).unwrap(), b"fixture");
    }

    #[test]
    fn hit_requires_matching_uri_and_mtime_and_ignores_bad_entries() {
        let (base, cache) = root("shared-read");
        let source = base.join("doc.pdf");
        std::fs::write(&source, b"%PDF").unwrap();
        let (_, revision) = files_core::media_source::open(&source).unwrap();
        let entry = entry(&cache, &source).unwrap();
        let large = cache.join("large");
        std::fs::create_dir_all(&large).unwrap();
        let put = |name: &str, bytes: &[u8]| std::fs::write(large.join(name), bytes).unwrap();
        let rgb = vec![9u8; 4 * 4 * 3];
        // Stale mtime: ignored.
        let stale = spec::encode_png(
            4,
            4,
            &rgb,
            &entry.uri,
            revision.modified_seconds - 1,
            revision.bytes,
        )
        .unwrap();
        put(&entry.name, &stale);
        assert!(lookup(&cache, &entry, revision).unwrap().is_none());
        // Another file's URI under this name (collision or forgery): ignored.
        let other = spec::encode_png(
            4,
            4,
            &rgb,
            "file:///elsewhere.pdf",
            revision.modified_seconds,
            revision.bytes,
        )
        .unwrap();
        put(&entry.name, &other);
        assert!(lookup(&cache, &entry, revision).unwrap().is_none());
        // Wrong declared size: ignored.
        let size = spec::encode_png(4, 4, &rgb, &entry.uri, revision.modified_seconds, 1).unwrap();
        put(&entry.name, &size);
        assert!(lookup(&cache, &entry, revision).unwrap().is_none());
        // Malformed bytes and an oversized file: ignored, not trusted.
        put(&entry.name, b"\x89PNG\r\n\x1a\nnot really");
        assert!(lookup(&cache, &entry, revision).unwrap().is_none());
        let mut huge = stale.clone();
        huge.resize(spec::MAX_CACHE_FILE as usize + 1, 0);
        put(&entry.name, &huge);
        assert!(lookup(&cache, &entry, revision).unwrap().is_none());
        // A symlinked entry pointing at a valid PNG elsewhere is refused.
        let good = spec::encode_png(
            4,
            4,
            &rgb,
            &entry.uri,
            revision.modified_seconds,
            revision.bytes,
        )
        .unwrap();
        std::fs::write(base.join("retained-target.png"), &good).unwrap();
        std::fs::rename(large.join(&entry.name), base.join("retained-huge.png")).unwrap();
        symlink(base.join("retained-target.png"), large.join(&entry.name)).unwrap();
        assert!(lookup(&cache, &entry, revision).unwrap().is_none());
        // A large/ thumbnail above 256 px is refused by dimension.
        let normal = cache.join("normal");
        std::fs::create_dir_all(&normal).unwrap();
        let wide = spec::encode_png(
            129,
            1,
            &vec![0; 129 * 3],
            &entry.uri,
            revision.modified_seconds,
            revision.bytes,
        )
        .unwrap();
        std::fs::write(normal.join(&entry.name), &wide).unwrap();
        assert!(lookup(&cache, &entry, revision).unwrap().is_none());
        // A valid normal/ entry matches.
        std::fs::write(normal.join(&entry.name), &good).unwrap();
        let (path, _) = lookup(&cache, &entry, revision).unwrap().unwrap();
        assert_eq!(path, normal.join(&entry.name));
    }

    #[test]
    fn nothing_is_written_for_excluded_locations() {
        let (base, cache) = root("shared-excluded");
        std::fs::create_dir_all(cache.join("normal")).unwrap();
        let inside = cache.join("normal").join("x.png");
        std::fs::write(&inside, b"thumb").unwrap();
        let (file, _) = files_core::media_source::open(&inside).unwrap();
        assert!(entry(&cache, &inside).is_none());
        assert!(!writable_source(&cache, &inside, &file));
        // Reached through a symlinked parent: still inside the cache.
        symlink(cache.join("normal"), base.join("alias")).unwrap();
        let alias = base.join("alias").join("x.png");
        let (file, _) = files_core::media_source::open(&alias).unwrap();
        assert!(!writable_source(&cache, &alias, &file));
        // Removable-media mount points are excluded by path.
        let local = base.join("local.png");
        std::fs::write(&local, b"x").unwrap();
        let (file, _) = files_core::media_source::open(&local).unwrap();
        assert!(!writable_source(
            &cache,
            Path::new("/run/media/user/USB/a.png"),
            &file
        ));
        assert!(!writable_source(
            &cache,
            Path::new("/media/disk/a.png"),
            &file
        ));
        assert!(writable_source(&cache, &local, &file));
        assert!(entry(&cache, Path::new("relative.png")).is_none());
    }

    #[test]
    fn foreign_or_group_writable_cache_directory_is_not_written() {
        use std::os::unix::fs::PermissionsExt;
        let (base, cache) = root("shared-perms");
        std::fs::create_dir(&cache).unwrap();
        std::fs::set_permissions(&cache, std::fs::Permissions::from_mode(0o777)).unwrap();
        let source = base.join("a.png");
        std::fs::write(&source, b"x").unwrap();
        let (_, revision) = files_core::media_source::open(&source).unwrap();
        let entry = entry(&cache, &source).unwrap();
        assert!(
            store(
                &cache,
                &entry,
                revision,
                &bitmap(revision, 64),
                &AtomicBool::new(false)
            )
            .is_err()
        );
        assert!(!cache.join("normal").exists());
        // A symlinked thumbnails root is refused too.
        let (base2, cache2) = root("shared-link-root");
        std::fs::create_dir(base2.join("real")).unwrap();
        symlink(base2.join("real"), &cache2).unwrap();
        assert!(
            store(
                &cache2,
                &entry,
                revision,
                &bitmap(revision, 64),
                &AtomicBool::new(false)
            )
            .is_err()
        );
        assert!(
            std::fs::read_dir(base2.join("real"))
                .unwrap()
                .next()
                .is_none()
        );
    }
}
