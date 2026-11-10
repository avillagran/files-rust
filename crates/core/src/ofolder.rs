//! Shared favorites and recent places with Ofolder (jrimmer/ofolder, MIT),
//! the Omarchy add-on that docks favorites and recent folders beside the
//! Open/Save dialogs. Files reads both of its lists and keeps the favorites in
//! step when the person adds or removes one in Files, so a folder saved in a
//! dialog shows up in Files and the other way round.
//!
//! Formats, as Ofolder writes them (checked against its source, 2026-11-09):
//! - `$OFOLDER_FAVORITES`, else `$XDG_CONFIG_HOME/ofolder/favorites.json`:
//!   `{"version": 1, "favorites": [{"path": "/a", "name": "Alpha"}]}`, `name`
//!   optional. Writers hold an exclusive `flock` on `favorites.json.lock` and
//!   replace the file through a `0600` temporary plus `rename`; readers do not
//!   lock.
//! - `$XDG_DATA_HOME/ofolder/recents.json`:
//!   `{"folders": [{"path", "apps", "at"}], "files": [...]}`, `at` in Unix
//!   seconds, newest first. Only Ofolder writes it; Files never does.
//!
//! Files only writes favorites when Ofolder is installed (its configuration
//! folder exists), never rewrites a file it cannot parse or whose version is
//! newer than 1, and keeps every field it does not know.
use std::{
    fs,
    io::{self, Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

/// Favorites file format this build understands.
pub const FORMAT_VERSION: u64 = 1;
/// Largest list file read; Ofolder's own limits keep them far smaller.
const MAX_BYTES: u64 = 1024 * 1024;
const LOCK_WAIT: Duration = Duration::from_secs(2);

/// Where Ofolder keeps its lists for this session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    pub favorites: PathBuf,
    pub recents: PathBuf,
}

impl Paths {
    /// Resolved like Ofolder does: its override, then XDG, then `~`.
    pub fn from_env() -> Self {
        let home = crate::theme::home();
        let favorites = std::env::var_os("OFOLDER_FAVORITES")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::var_os("XDG_CONFIG_HOME")
                    .map(PathBuf::from)
                    .filter(|p| p.is_absolute())
                    .unwrap_or_else(|| home.join(".config"))
                    .join("ofolder/favorites.json")
            });
        let recents = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".local/share"))
            .join("ofolder/recents.json");
        Self { favorites, recents }
    }

    /// Ofolder is installed: its configuration folder exists.
    pub fn installed(&self) -> bool {
        self.favorites.parent().is_some_and(Path::is_dir)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Favorite {
    pub path: PathBuf,
    /// The name given in Ofolder, when it has one.
    pub name: Option<String>,
}

impl Favorite {
    /// Label for lists: the given name, else the folder name.
    pub fn label(&self) -> String {
        self.name.clone().unwrap_or_else(|| {
            self.path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| self.path.display().to_string())
        })
    }
}

/// A recent folder or file from Ofolder, newest first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recent {
    pub path: PathBuf,
    pub folder: bool,
    /// Unix seconds of the last use.
    pub at: u64,
}

fn read_bounded(path: &Path) -> io::Result<Option<String>> {
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut text = String::new();
    file.take(MAX_BYTES + 1).read_to_string(&mut text)?;
    if text.len() as u64 > MAX_BYTES {
        return Err(io::Error::other("archivo de Ofolder demasiado grande"));
    }
    Ok(Some(text))
}

fn parse_favorites(text: &str) -> Result<serde_json::Value, String> {
    let doc: serde_json::Value =
        serde_json::from_str(text).map_err(|_| "favoritos de Ofolder ilegibles".to_string())?;
    let version = match doc.get("version") {
        None => FORMAT_VERSION,
        Some(v) => v
            .as_u64()
            .ok_or_else(|| "versión de favoritos de Ofolder inválida".to_string())?,
    };
    if version > FORMAT_VERSION {
        return Err(format!(
            "favoritos de Ofolder en formato {version}, más nuevo que el que Files entiende"
        ));
    }
    if !doc
        .get("favorites")
        .is_some_and(serde_json::Value::is_array)
    {
        return Err("favoritos de Ofolder sin lista".into());
    }
    Ok(doc)
}

fn favorites_of(doc: &serde_json::Value) -> Vec<Favorite> {
    let mut out: Vec<Favorite> = Vec::new();
    for item in doc["favorites"].as_array().into_iter().flatten() {
        let Some(path) = item.get("path").and_then(|p| p.as_str()) else {
            continue;
        };
        let path = PathBuf::from(path);
        if !path.is_absolute() || out.iter().any(|f| f.path == path) {
            continue;
        }
        let name = item
            .get("name")
            .and_then(|n| n.as_str())
            .filter(|n| !n.trim().is_empty())
            .map(str::to_string);
        out.push(Favorite { path, name });
    }
    out
}

/// Ofolder's favorites in its order; empty when there is no file.
pub fn read_favorites(path: &Path) -> Result<Vec<Favorite>, String> {
    match read_bounded(path).map_err(|e| e.to_string())? {
        None => Ok(Vec::new()),
        Some(text) => parse_favorites(&text).map(|doc| favorites_of(&doc)),
    }
}

/// Ofolder's recent folders and files, newest first, at most `max` of each.
/// An unreadable file reads as empty: Ofolder owns and repairs it.
pub fn read_recents(path: &Path, max: usize) -> Vec<Recent> {
    let Ok(Some(text)) = read_bounded(path) else {
        return Vec::new();
    };
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (key, folder) in [("folders", true), ("files", false)] {
        for item in doc
            .get(key)
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .take(max)
        {
            let (Some(path), Some(at)) = (
                item.get("path").and_then(|p| p.as_str()),
                item.get("at").and_then(|a| a.as_u64()),
            ) else {
                continue;
            };
            let path = PathBuf::from(path);
            if path.is_absolute() {
                out.push(Recent { path, folder, at });
            }
        }
    }
    out.sort_by_key(|r| std::cmp::Reverse(r.at));
    out
}

/// Holds Ofolder's exclusive lock on `<file>.lock` while alive.
struct Lock(fs::File);

impl Lock {
    fn take(target: &Path) -> io::Result<Self> {
        use std::os::fd::AsRawFd;
        let name = target
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".into());
        let file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(target.with_file_name(format!("{name}.lock")))?;
        let deadline = Instant::now() + LOCK_WAIT;
        loop {
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                return Ok(Self(file));
            }
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::EWOULDBLOCK) {
                return Err(error);
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "Ofolder está guardando sus favoritos; reintenta",
                ));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
    }
}

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// Replaces `path` like Ofolder does: `0600` temporary, fsync, rename, fsync dir.
fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    let tmp = dir.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&tmp)?;
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    if let Err(e) = written.and_then(|()| fs::rename(&tmp, path)) {
        // Only the temporary this call created exclusively is removed.
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    fs::File::open(dir)?.sync_all()
}

/// Adds (`on`) or removes `folder` in Ofolder's favorites. Returns whether
/// the file changed; `Ok(false)` too when Ofolder is not installed.
pub fn set_favorite(paths: &Paths, folder: &Path, on: bool) -> Result<bool, String> {
    if !paths.installed() {
        return Ok(false);
    }
    let Some(text_path) = folder.to_str().filter(|_| folder.is_absolute()) else {
        return Err("Ofolder solo guarda rutas absolutas en UTF-8".into());
    };
    let target = &paths.favorites;
    if fs::symlink_metadata(target).is_ok_and(|m| !m.file_type().is_file()) {
        return Err("favoritos de Ofolder no es un archivo normal".into());
    }
    let _lock = Lock::take(target).map_err(|e| e.to_string())?;
    let mut doc = match read_bounded(target).map_err(|e| e.to_string())? {
        Some(text) => parse_favorites(&text)?,
        None => serde_json::json!({ "version": FORMAT_VERSION, "favorites": [] }),
    };
    let list = doc["favorites"].as_array_mut().expect("checked list");
    let present = list
        .iter()
        .any(|item| item.get("path").and_then(|p| p.as_str()) == Some(text_path));
    match (on, present) {
        (true, true) | (false, false) => return Ok(false),
        (true, false) => list.push(serde_json::json!({ "path": text_path })),
        (false, true) => {
            list.retain(|item| item.get("path").and_then(|p| p.as_str()) != Some(text_path))
        }
    }
    if doc.get("version").is_none() {
        doc["version"] = FORMAT_VERSION.into();
    }
    let mut text = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
    text.push('\n');
    write_atomic(target, text.as_bytes()).map_err(|e| e.to_string())?;
    Ok(true)
}

/// `at` as the ISO 8601 UTC text the recent-files menu sorts and shows.
pub fn iso_utc(at: u64) -> String {
    let days = (at / 86_400) as i64;
    let rem = at % 86_400;
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A private Ofolder layout inside the test's own temporary folder.
    fn layout(tag: &str) -> Paths {
        let root = std::env::temp_dir().join(format!(
            "files-ofolder-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(root.join("config/ofolder")).unwrap();
        fs::create_dir_all(root.join("data/ofolder")).unwrap();
        Paths {
            favorites: root.join("config/ofolder/favorites.json"),
            recents: root.join("data/ofolder/recents.json"),
        }
    }

    #[test]
    fn favorites_round_trip_keeping_unknown_fields_and_names() {
        let p = layout("roundtrip");
        fs::write(
            &p.favorites,
            r#"{"version":1,"favorites":[{"path":"/srv/a","name":"Alpha","color":"red"},{"path":"/srv/b"},{"path":"/srv/a"}],"extra":true}"#,
        )
        .unwrap();
        let read = read_favorites(&p.favorites).unwrap();
        assert_eq!(read.len(), 2, "duplicates are dropped like Ofolder does");
        assert_eq!(read[0].label(), "Alpha");
        assert_eq!(read[1].label(), "b");
        assert_eq!(set_favorite(&p, Path::new("/srv/c"), true), Ok(true));
        assert_eq!(set_favorite(&p, Path::new("/srv/c"), true), Ok(false));
        assert_eq!(set_favorite(&p, Path::new("/srv/b"), false), Ok(true));
        let doc: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&p.favorites).unwrap()).unwrap();
        assert_eq!(doc["extra"], true);
        assert_eq!(doc["favorites"][0]["color"], "red");
        let paths: Vec<_> = read_favorites(&p.favorites)
            .unwrap()
            .into_iter()
            .map(|f| f.path)
            .collect();
        assert_eq!(paths, [PathBuf::from("/srv/a"), PathBuf::from("/srv/c")]);
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&p.favorites).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert!(p.favorites.with_file_name("favorites.json.lock").is_file());
    }

    #[test]
    fn files_never_rewrites_what_it_cannot_read_or_creates_ofolder() {
        let p = layout("refuse");
        let newer = r#"{"version":2,"favorites":[]}"#;
        fs::write(&p.favorites, newer).unwrap();
        assert!(read_favorites(&p.favorites).is_err());
        assert!(set_favorite(&p, Path::new("/srv/a"), true).is_err());
        assert_eq!(fs::read_to_string(&p.favorites).unwrap(), newer);
        fs::write(&p.favorites, "not json").unwrap();
        assert!(set_favorite(&p, Path::new("/srv/a"), true).is_err());
        assert_eq!(fs::read_to_string(&p.favorites).unwrap(), "not json");
        let absent = Paths {
            favorites: p.favorites.with_file_name("missing-dir/favorites.json"),
            recents: p.recents.clone(),
        };
        assert_eq!(set_favorite(&absent, Path::new("/srv/a"), true), Ok(false));
        assert!(!absent.favorites.parent().unwrap().exists());
        let created = layout("create");
        assert_eq!(set_favorite(&created, Path::new("/srv/a"), true), Ok(true));
        assert_eq!(
            read_favorites(&created.favorites).unwrap()[0].path,
            Path::new("/srv/a")
        );
    }

    #[test]
    fn a_held_ofolder_lock_makes_the_write_wait_then_give_up() {
        let p = layout("locked");
        let held = Lock::take(&p.favorites).unwrap();
        let started = Instant::now();
        let refused = set_favorite(&p, Path::new("/srv/a"), true);
        assert!(refused.unwrap_err().contains("reintenta"));
        assert!(started.elapsed() >= LOCK_WAIT);
        drop(held);
        assert_eq!(set_favorite(&p, Path::new("/srv/a"), true), Ok(true));
    }

    #[test]
    fn recents_merge_folders_and_files_newest_first() {
        let p = layout("recents");
        fs::write(
            &p.recents,
            r#"{"folders":[{"path":"/home/j/Docs","apps":["class:zen"],"at":200},{"path":"rel","apps":[],"at":900}],
                "files":[{"path":"/home/j/Docs/a.pdf","apps":[],"at":300}]}"#,
        )
        .unwrap();
        let list = read_recents(&p.recents, 20);
        assert_eq!(
            list.iter()
                .map(|r| (r.path.to_str().unwrap(), r.folder))
                .collect::<Vec<_>>(),
            [("/home/j/Docs/a.pdf", false), ("/home/j/Docs", true)]
        );
        fs::write(&p.recents, "{broken").unwrap();
        assert!(read_recents(&p.recents, 20).is_empty());
    }

    #[test]
    fn unix_seconds_become_sortable_utc_text() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_utc(1_767_225_600), "2026-01-01T00:00:00Z");
        assert_eq!(iso_utc(1_794_278_109), "2026-11-10T02:35:09Z");
    }
}
