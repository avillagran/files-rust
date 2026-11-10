//! Read-only extras for the Properties dialog: owner/group names, birth time,
//! volume space and filesystem type, MIME type and default application.
//!
//! Everything here may touch the filesystem or the NSS databases, so callers
//! run it on a worker, never on the UI thread. Nothing is followed implicitly:
//! the object is always examined with lstat/AT_SYMLINK_NOFOLLOW semantics, a
//! symlink's own volume is the one of its directory entry.
use crate::file_properties::{Kind, Properties, Timestamp};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Volume {
    /// Filesystem type from mountinfo, when the mount can be identified.
    pub filesystem: Option<String>,
    pub total_bytes: u64,
    /// Space available to unprivileged users (f_bavail).
    pub free_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Details {
    /// "name (uid)", or the bare number when the name cannot be resolved.
    pub owner: String,
    pub group: String,
    /// statx btime; None when the kernel or filesystem does not record it.
    pub birth: Option<Timestamp>,
    pub volume: Option<Volume>,
    /// Symlinks only: whether the raw target currently resolves.
    pub link_resolves: Option<bool>,
    pub mime: Option<String>,
    pub default_app: Option<String>,
}

/// "name (id)" or just the number: the numeric id is never hidden.
pub fn label(name: Option<&str>, id: u32) -> String {
    match name {
        Some(name) if !name.is_empty() => format!("{name} ({id})"),
        _ => id.to_string(),
    }
}

#[cfg(unix)]
fn lookup(
    id: u32,
    call: unsafe extern "C" fn(
        u32,
        *mut libc::passwd,
        *mut libc::c_char,
        libc::size_t,
        *mut *mut libc::passwd,
    ) -> libc::c_int,
) -> Option<String> {
    let mut record = std::mem::MaybeUninit::<libc::passwd>::zeroed();
    let mut buffer = vec![0u8; 1024];
    loop {
        let mut found: *mut libc::passwd = std::ptr::null_mut();
        let code = unsafe {
            call(
                id,
                record.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut found,
            )
        };
        if code == libc::ERANGE && buffer.len() < 64 * 1024 {
            buffer.resize(buffer.len() * 4, 0);
            continue;
        }
        if code != 0 || found.is_null() {
            return None;
        }
        let name = unsafe { std::ffi::CStr::from_ptr((*found).pw_name) };
        return Some(name.to_string_lossy().into_owned());
    }
}
/// User name through getpwuid_r; None when unknown or the lookup fails.
#[cfg(unix)]
pub fn user_name(uid: u32) -> Option<String> {
    lookup(uid, libc::getpwuid_r)
}
/// Group name through getgrgid_r; None when unknown or the lookup fails.
#[cfg(unix)]
pub fn group_name(gid: u32) -> Option<String> {
    let mut record = std::mem::MaybeUninit::<libc::group>::zeroed();
    let mut buffer = vec![0u8; 1024];
    loop {
        let mut found: *mut libc::group = std::ptr::null_mut();
        let code = unsafe {
            libc::getgrgid_r(
                gid,
                record.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut found,
            )
        };
        if code == libc::ERANGE && buffer.len() < 64 * 1024 {
            buffer.resize(buffer.len() * 4, 0);
            continue;
        }
        if code != 0 || found.is_null() {
            return None;
        }
        let name = unsafe { std::ffi::CStr::from_ptr((*found).gr_name) };
        return Some(name.to_string_lossy().into_owned());
    }
}
#[cfg(not(unix))]
pub fn user_name(_: u32) -> Option<String> {
    None
}
#[cfg(not(unix))]
pub fn group_name(_: u32) -> Option<String> {
    None
}

/// Creation time through statx(AT_SYMLINK_NOFOLLOW); None when unsupported.
#[cfg(target_os = "linux")]
pub fn birth_time(path: &Path) -> Option<Timestamp> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut buffer = std::mem::MaybeUninit::<libc::statx>::zeroed();
    let code = unsafe {
        libc::statx(
            libc::AT_FDCWD,
            path.as_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
            libc::STATX_BTIME,
            buffer.as_mut_ptr(),
        )
    };
    if code != 0 {
        return None;
    }
    let buffer = unsafe { buffer.assume_init() };
    (buffer.stx_mask & libc::STATX_BTIME != 0 && buffer.stx_btime.tv_nsec < 1_000_000_000)
        .then_some(Timestamp {
            seconds: buffer.stx_btime.tv_sec,
            nanoseconds: buffer.stx_btime.tv_nsec,
        })
}
#[cfg(not(target_os = "linux"))]
pub fn birth_time(_: &Path) -> Option<Timestamp> {
    None
}

/// Total and available space (statvfs) plus the filesystem type of the volume
/// holding `path`. `device` is the object's st_dev, used to find its mount.
#[cfg(target_os = "linux")]
pub fn volume(path: &Path, device: u64) -> Option<Volume> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut s = std::mem::MaybeUninit::<libc::statvfs>::zeroed();
    if unsafe { libc::statvfs(c.as_ptr(), s.as_mut_ptr()) } != 0 {
        return None;
    }
    let s = unsafe { s.assume_init() };
    // statvfs field widths differ between 32 and 64 bit targets.
    #[allow(clippy::unnecessary_cast)]
    let (frsize, blocks, avail) = (s.f_frsize as u64, s.f_blocks as u64, s.f_bavail as u64);
    let mounts = std::fs::read("/proc/self/mountinfo")
        .ok()
        .and_then(|bytes| crate::mount_table::parse(&bytes).ok())
        .unwrap_or_default();
    Some(Volume {
        filesystem: filesystem_type(&mounts, path, device),
        total_bytes: blocks.saturating_mul(frsize),
        free_bytes: avail.saturating_mul(frsize),
    })
}
#[cfg(not(target_os = "linux"))]
pub fn volume(_: &Path, _: u64) -> Option<Volume> {
    None
}

/// Mount whose device matches `device`, else (btrfs subvolumes report a
/// st_dev that differs from mountinfo) the deepest mount point prefixing `path`.
#[cfg(target_os = "linux")]
pub fn filesystem_type(
    mounts: &[crate::mount_table::Mount],
    path: &Path,
    device: u64,
) -> Option<String> {
    let (major, minor) = (libc::major(device), libc::minor(device));
    let deepest = |candidates: &mut dyn Iterator<Item = &crate::mount_table::Mount>| {
        candidates
            .filter(|m| path.starts_with(&m.point))
            .max_by_key(|m| (m.point.as_os_str().len(), m.id))
            .map(|m| String::from_utf8_lossy(&m.filesystem).into_owned())
    };
    deepest(&mut mounts.iter().filter(|m| m.device == (major, minor)))
        .or_else(|| deepest(&mut mounts.iter()))
}

/// Gather everything for one captured object. `path` is the verified path of
/// the object itself (never resolved through a final symlink).
pub fn gather(path: &Path, properties: &Properties) -> Details {
    let symlink = properties.identity.kind == Kind::Symlink;
    // The volume of a symlink is the one of its directory entry.
    let volume_path: PathBuf = if symlink {
        path.parent().unwrap_or(path).to_path_buf()
    } else {
        path.to_path_buf()
    };
    let (mime, default_app) = if symlink {
        (None, None)
    } else {
        let mime = crate::apps::detect_mime(path);
        let catalog = crate::apps::scan();
        let app = crate::apps::default_for(&mime, &catalog)
            .and_then(|id| crate::apps::find(&catalog, &id).map(|a| a.name.clone()));
        (Some(mime), app)
    };
    Details {
        owner: label(user_name(properties.uid).as_deref(), properties.uid),
        group: label(group_name(properties.gid).as_deref(), properties.gid),
        birth: birth_time(path),
        volume: volume(&volume_path, properties.identity.device),
        // metadata() follows the link only to test it; nothing else uses it.
        link_resolves: symlink.then(|| std::fs::metadata(path).is_ok()),
        mime,
        default_app,
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;

    #[test]
    fn unknown_ids_fall_back_to_the_bare_number_and_known_ones_show_both() {
        assert_eq!(label(None, 4_294_967_294), "4294967294");
        assert_eq!(label(Some(""), 7), "7");
        assert_eq!(label(Some("root"), 0), "root (0)");
        // Resolve through libc: root is uid/gid 0 on every Linux host.
        assert_eq!(user_name(0).as_deref(), Some("root"));
        assert!(group_name(0).is_some());
        assert_eq!(user_name(4_294_967_294), None);
        assert_eq!(group_name(4_294_967_294), None);
    }

    #[test]
    fn volume_line_has_space_and_a_filesystem_type_for_the_private_dir() {
        let dir = std::env::temp_dir();
        let device = std::fs::metadata(&dir).unwrap().dev();
        let volume = volume(&dir, device).expect("statvfs of the temp dir");
        assert!(volume.total_bytes > 0 && volume.free_bytes <= volume.total_bytes);
        assert!(volume.filesystem.is_some_and(|f| !f.is_empty()));
    }

    #[test]
    fn gather_uses_lstat_semantics_for_a_symlink_and_reports_resolution() {
        let base = std::env::temp_dir().join(format!("files-details-{}", std::process::id()));
        std::fs::create_dir(&base).unwrap();
        let target = base.join("target.txt");
        std::fs::write(&target, b"x").unwrap();
        let live = base.join("live");
        let dead = base.join("dead");
        std::os::unix::fs::symlink(&target, &live).unwrap();
        std::os::unix::fs::symlink(base.join("nowhere"), &dead).unwrap();
        for (link, resolves) in [(&live, true), (&dead, false)] {
            let snapshot = crate::file_properties::capture(link).unwrap();
            assert_eq!(snapshot.properties.identity.kind, Kind::Symlink);
            let details = gather(link, &snapshot.properties);
            assert_eq!(details.link_resolves, Some(resolves));
            assert!(details.mime.is_none() && details.default_app.is_none());
            assert!(details.volume.is_some());
        }
        let snapshot = crate::file_properties::capture(&target).unwrap();
        let details = gather(&target, &snapshot.properties);
        assert_eq!(details.link_resolves, None);
        assert!(details.mime.is_some());
        assert!(details.owner.contains(&snapshot.properties.uid.to_string()));
    }
}
