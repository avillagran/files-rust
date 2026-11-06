//! Bounded local FileManager1 requests. Planning preserves raw path identity;
//! read-only preflight belongs on a worker, never the UI thread.
use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};

pub const MAX_URIS: usize = 32;
pub const MAX_BYTES: usize = 64 * 1024;
pub const MAX_PATH_BYTES: usize = 4096;
pub const MAX_STARTUP_ID: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    ShowFolders,
    ShowItems,
    ShowItemProperties,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub method: Method,
    pub paths: Vec<PathBuf>,
    pub startup_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub directory: PathBuf,
    pub names: Vec<OsString>,
}

fn count(count: usize) -> Result<(), String> {
    if count == 0 || count > MAX_URIS {
        Err(format!(
            "A file-manager request needs 1–{MAX_URIS} locations"
        ))
    } else {
        Ok(())
    }
}

fn startup_id(id: &str) -> Result<(), String> {
    if id.len() > MAX_STARTUP_ID || id.chars().any(char::is_control) {
        Err("Invalid or oversized startup notification ID".into())
    } else {
        Ok(())
    }
}

fn budget(total: usize, additional: usize) -> Result<usize, String> {
    total
        .checked_add(additional)
        .filter(|&n| n <= MAX_BYTES)
        .ok_or_else(|| "File-manager request exceeds the 64 KiB payload limit".into())
}

fn validate_path(path: &Path) -> Result<(), String> {
    let bytes = path.as_os_str().as_encoded_bytes();
    if !path.is_absolute() || bytes.len() > MAX_PATH_BYTES || bytes.contains(&0) {
        Err("File-manager paths must be absolute, NUL-free and at most 4096 bytes".into())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn uri_path(uri: &str) -> Result<PathBuf, String> {
    use std::os::unix::ffi::OsStringExt;
    let invalid = || "Expected one bounded local file:// URI".to_string();
    if !uri
        .get(..7)
        .is_some_and(|s| s.eq_ignore_ascii_case("file://"))
    {
        return Err(invalid());
    }
    let rest = &uri.as_bytes()[7..];
    let slash = rest.iter().position(|&b| b == b'/').ok_or_else(invalid)?;
    let authority = &rest[..slash];
    if !authority.is_empty() && !authority.eq_ignore_ascii_case(b"localhost") {
        return Err("Remote file URI authorities are not supported".into());
    }
    let encoded = &rest[slash..];
    let mut decoded = Vec::with_capacity(encoded.len().min(MAX_PATH_BYTES));
    let hex = |b| match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    };
    let mut i = 0;
    while i < encoded.len() {
        let byte = if encoded[i] == b'%' {
            let hi = encoded
                .get(i + 1)
                .and_then(|&b| hex(b))
                .ok_or_else(invalid)?;
            let lo = encoded
                .get(i + 2)
                .and_then(|&b| hex(b))
                .ok_or_else(invalid)?;
            i += 3;
            hi * 16 + lo
        } else {
            let byte = encoded[i];
            i += 1;
            // An individual URI cannot become a newline-delimited URI list.
            // Unicode names remain supported through percent-encoded UTF-8.
            if byte <= b' ' || byte >= 127 || matches!(byte, b'?' | b'#' | b'\\') {
                return Err(invalid());
            }
            byte
        };
        if byte == 0 || decoded.len() == MAX_PATH_BYTES {
            return Err(invalid());
        }
        decoded.push(byte);
    }
    while decoded.len() > 1 && decoded.last() == Some(&b'/') {
        decoded.pop();
    }
    if decoded[1..]
        .split(|&b| b == b'/')
        .any(|part| part.is_empty() || part == b"." || part == b"..")
        && decoded.len() != 1
    {
        return Err("File URIs must not contain empty or dot path segments".into());
    }
    Ok(PathBuf::from(OsString::from_vec(decoded)))
}

#[cfg(not(unix))]
fn uri_path(_: &str) -> Result<PathBuf, String> {
    Err("Local file URI decoding is not implemented on this platform".into())
}

fn directory(path: &Path) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // A raced FIFO/special node must not block an advisory directory probe.
        options.custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .map_err(|e| format!("Cannot open requested directory: {e}"))?;
    if !file
        .metadata()
        .map_err(|e| format!("Cannot inspect requested directory: {e}"))?
        .is_dir()
    {
        return Err("The requested location is not a directory".into());
    }
    Ok(file)
}

impl Request {
    pub fn from_uris(method: Method, uris: &[String], startup: &str) -> Result<Self, String> {
        count(uris.len())?;
        startup_id(startup)?;
        let mut bytes = startup.len();
        for uri in uris {
            bytes = budget(bytes, uri.len())?;
        }
        let paths = uris
            .iter()
            .map(|uri| uri_path(uri))
            .collect::<Result<Vec<_>, _>>()?;
        let mut request = Self::from_paths(method, paths)?;
        request.startup_id = startup.into();
        Ok(request)
    }

    /// Raw CLI paths are already resolved by the caller. Never URI-decode them.
    pub fn from_paths(method: Method, paths: Vec<PathBuf>) -> Result<Self, String> {
        count(paths.len())?;
        let mut bytes = 0;
        for path in &paths {
            validate_path(path)?;
            bytes = budget(bytes, path.as_os_str().as_encoded_bytes().len())?;
        }
        let mut unique = Vec::with_capacity(paths.len());
        for path in paths {
            if !unique
                .iter()
                .any(|p: &PathBuf| p.as_os_str() == path.as_os_str())
            {
                unique.push(path);
            }
        }
        Ok(Self {
            method,
            paths: unique,
            startup_id: String::new(),
        })
    }

    /// First-seen directory and basename order is stable. Properties callers
    /// may use the same item grouping but retain the original paths for Inspect.
    pub fn groups(&self) -> Vec<Group> {
        let mut groups: Vec<Group> = Vec::new();
        for path in &self.paths {
            let (parent, name) = if self.method == Method::ShowFolders {
                (path.as_path(), None)
            } else {
                match (path.parent(), path.file_name()) {
                    (Some(parent), Some(name)) => (parent, Some(name)),
                    _ => (path.as_path(), None),
                }
            };
            let index = groups
                .iter()
                .position(|g| g.directory.as_os_str() == parent.as_os_str());
            let group = if let Some(i) = index {
                &mut groups[i]
            } else {
                groups.push(Group {
                    directory: parent.to_path_buf(),
                    names: Vec::new(),
                });
                groups.last_mut().expect("just inserted")
            };
            if let Some(name) = name
                && !group.names.iter().any(|n| n == name)
            {
                group.names.push(name.to_os_string());
            }
        }
        groups
    }

    /// Advisory read-only validation of the complete batch. This holds no
    /// durable filesystem identity and never authorizes a mutation. Call only
    /// on a worker; the UI must still load/reconcile its fresh listing.
    pub fn preflight(&self) -> Result<(), String> {
        count(self.paths.len())?;
        startup_id(&self.startup_id)?;
        let mut bytes = self.startup_id.len();
        for path in &self.paths {
            validate_path(path)?;
            bytes = budget(bytes, path.as_os_str().as_encoded_bytes().len())?;
        }
        for path in &self.paths {
            if self.method == Method::ShowFolders {
                if !fs::metadata(path)
                    .map_err(|e| format!("Cannot inspect requested folder: {e}"))?
                    .is_dir()
                {
                    return Err("The requested folder is not a directory".into());
                }
                directory(path)?;
            } else {
                // A dangling symlink itself is a valid item/properties target.
                fs::symlink_metadata(path)
                    .map_err(|e| format!("Cannot inspect requested item: {e}"))?;
                directory(path.parent().unwrap_or(path))?;
            }
        }
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{
        os::unix::{
            ffi::OsStringExt,
            fs::{DirBuilderExt, PermissionsExt, symlink},
        },
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);

    // These owned fixtures are intentionally retained; no test removes trees
    // or follows a fixture link during cleanup.
    fn fixture() -> PathBuf {
        let base = std::env::var_os("TMPDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/var/tmp/files-core-file-manager"));
        if !base.exists() {
            fs::DirBuilder::new().mode(0o700).create(&base).unwrap();
        }
        let meta = fs::symlink_metadata(&base).unwrap();
        assert!(meta.is_dir() && !meta.file_type().is_symlink());
        assert_eq!(meta.permissions().mode() & 0o777, 0o700);
        let path = base.join(format!(
            "file-manager-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        path
    }

    #[test]
    fn local_uris_decode_raw_bytes_once_and_accept_root_and_folder_suffix() {
        let request = Request::from_uris(
            Method::ShowItems,
            &[
                "file:///A/raw-%FF-%0A%2525".into(),
                "FILE://LOCALHOST/A/other/".into(),
                "file:///".into(),
            ],
            "token-123",
        )
        .unwrap();
        assert_eq!(
            request.paths[0].as_os_str().as_encoded_bytes(),
            b"/A/raw-\xff-\n%25"
        );
        assert_eq!(request.paths[1], PathBuf::from("/A/other"));
        assert_eq!(request.paths[2], PathBuf::from("/"));
        assert_eq!(request.startup_id, "token-123");
        let raw = Request::from_paths(Method::ShowItems, vec![PathBuf::from("/A/%FF")]).unwrap();
        assert_eq!(raw.paths[0], PathBuf::from("/A/%FF"));
    }

    #[test]
    fn malformed_nonlocal_and_literal_control_uris_never_create_extra_locations() {
        for uri in [
            "sftp://host/A",
            "file:/A",
            "file://remote/A",
            "file://user@localhost/A",
            "file://localhost:1/A",
            "file:///A%",
            "file:///A%GG",
            "file:///A%00",
            "file:///A\nfile:///B",
            "file:///A?query",
            "file:///A#fragment",
            "file:///A B",
            "file:///A/./B",
            "file:///A/%2e%2e/B",
            "file:///A//B",
            "file:///",
        ] {
            if uri == "file:///" {
                continue;
            }
            assert!(
                Request::from_uris(Method::ShowItems, &[uri.into()], "").is_err(),
                "{uri:?}"
            );
        }
    }

    #[test]
    fn grouping_preserves_parent_name_order_and_deduplicates_raw_paths() {
        let raw = OsString::from_vec(b"raw-\xff".to_vec());
        let paths = vec![
            PathBuf::from("/A/first"),
            PathBuf::from("/B/second"),
            PathBuf::from("/A").join(&raw),
            PathBuf::from("/A/first"),
            PathBuf::from("/"),
        ];
        let request = Request::from_paths(Method::ShowItems, paths).unwrap();
        assert_eq!(request.paths.len(), 4);
        assert_eq!(
            request.groups(),
            vec![
                Group {
                    directory: "/A".into(),
                    names: vec!["first".into(), raw]
                },
                Group {
                    directory: "/B".into(),
                    names: vec!["second".into()]
                },
                Group {
                    directory: "/".into(),
                    names: vec![]
                }
            ]
        );
        let folders = Request::from_paths(
            Method::ShowFolders,
            vec!["/A".into(), "/A".into(), "/B".into()],
        )
        .unwrap();
        assert_eq!(
            folders.groups(),
            vec![
                Group {
                    directory: "/A".into(),
                    names: vec![]
                },
                Group {
                    directory: "/B".into(),
                    names: vec![]
                }
            ]
        );
    }

    #[test]
    fn request_limits_cover_original_count_wire_decoded_paths_and_startup() {
        assert!(Request::from_paths(Method::ShowItems, vec![]).is_err());
        assert!(
            Request::from_uris(
                Method::ShowItems,
                &vec!["file:///A".into(); MAX_URIS + 1],
                ""
            )
            .is_err()
        );
        assert!(
            Request::from_uris(
                Method::ShowItems,
                &["file:///A".into()],
                &"x".repeat(MAX_STARTUP_ID + 1)
            )
            .is_err()
        );
        assert!(
            Request::from_uris(Method::ShowItems, &["file:///A".into()], "line\nbreak").is_err()
        );
        assert!(
            Request::from_uris(
                Method::ShowItems,
                &[format!("file:///{}", "%41".repeat(MAX_PATH_BYTES))],
                ""
            )
            .is_err()
        );
        let max_path = PathBuf::from(format!("/{}", "a".repeat(MAX_PATH_BYTES - 1)));
        assert!(
            Request::from_paths(
                Method::ShowItems,
                vec![max_path.clone(); MAX_BYTES / MAX_PATH_BYTES]
            )
            .is_ok()
        );
        assert!(
            Request::from_paths(
                Method::ShowItems,
                vec![max_path; MAX_BYTES / MAX_PATH_BYTES + 1]
            )
            .is_err()
        );
        assert!(Request::from_paths(Method::ShowItems, vec![PathBuf::from("relative")]).is_err());
        assert!(
            Request::from_paths(
                Method::ShowItems,
                vec![PathBuf::from(OsString::from_vec(b"/A/nul\0".to_vec()))]
            )
            .is_err()
        );
    }

    #[test]
    fn preflight_checks_the_whole_batch_and_preserves_fixture_payloads() {
        let base = fixture();
        let folder = base.join("folder");
        fs::create_dir(&folder).unwrap();
        let item = folder.join("item");
        fs::write(&item, b"owned payload").unwrap();
        let valid = Request::from_paths(Method::ShowItems, vec![item.clone()]).unwrap();
        assert!(valid.preflight().is_ok());
        let batch = Request::from_paths(
            Method::ShowItems,
            vec![item.clone(), folder.join("missing")],
        )
        .unwrap();
        assert!(batch.preflight().is_err());
        let folders = Request::from_paths(Method::ShowFolders, vec![folder, item.clone()]).unwrap();
        assert!(folders.preflight().is_err());
        assert_eq!(fs::read(item).unwrap(), b"owned payload");
    }

    #[test]
    fn dangling_symlink_is_an_item_and_properties_target_but_not_a_folder() {
        let base = fixture();
        let link = base.join("broken");
        symlink(base.join("missing-owned-target"), &link).unwrap();
        for method in [Method::ShowItems, Method::ShowItemProperties] {
            assert!(
                Request::from_paths(method, vec![link.clone()])
                    .unwrap()
                    .preflight()
                    .is_ok()
            );
        }
        assert!(
            Request::from_paths(Method::ShowFolders, vec![link])
                .unwrap()
                .preflight()
                .is_err()
        );
        let real = base.join("real");
        fs::create_dir(&real).unwrap();
        let alias = base.join("alias");
        symlink(&real, &alias).unwrap();
        let request = Request::from_paths(Method::ShowFolders, vec![alias.clone()]).unwrap();
        assert!(request.preflight().is_ok());
        assert_eq!(request.groups()[0].directory, alias);
    }
}
