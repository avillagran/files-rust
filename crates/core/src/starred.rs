//! Starred files and folders: a small Files-owned list (GNOME Files keeps
//! stars in Tracker, which Omarchy does not ship).
//!
//! Format of `$XDG_DATA_HOME/files/starred`, UTF-8 text:
//!
//! ```text
//! # Files starred items, format 1
//! file:///home/u/Documents/informe%20final.odt
//! file:///home/u/Proyectos
//! ```
//!
//! One percent-encoded `file://` URI per line (so any path, even one with a
//! line break, fits on a line), in the order the items were starred. Lines
//! starting with `#` and blank lines are ignored. Writes go through the
//! shared persistence helper (`0600` temporary, fsync, rename, folder fsync)
//! while this process holds a lock, so the list is never half written. A
//! file Files cannot read (too large, not UTF-8) is never rewritten.
//!
//! Stars are not bookmarks: bookmarks are sidebar places, stars flag items.
//! Files never writes stars into Ofolder's favorites; other tools may read
//! this file (format above) to offer starred folders.
use crate::recent_store::{file_uri, uri_path};
use std::{
    io,
    path::{Path, PathBuf},
};

pub const MAX_ITEMS: usize = 10_000;
const MAX_BYTES: usize = crate::persistence::MAX_CONFIG_BYTES;
const HEADER: &str = "# Files starred items, format 1";
static PROCESS: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn path(data_home: &Path) -> PathBuf {
    data_home.join("files/starred")
}

/// Items of a starred document, in order, once each.
pub fn decode(text: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(p) = uri_path(line)
            && !out.contains(&p)
        {
            out.push(p);
        }
        if out.len() >= MAX_ITEMS {
            break;
        }
    }
    out
}

pub fn encode(list: &[PathBuf]) -> String {
    let mut text = format!("{HEADER}\n");
    for uri in list.iter().filter_map(|p| file_uri(p)) {
        text.push_str(&uri);
        text.push('\n');
    }
    text
}

/// The starred items; a missing list is empty.
pub fn read(data_home: &Path) -> io::Result<Vec<PathBuf>> {
    let bytes = match crate::persistence::read_bounded(&path(data_home), MAX_BYTES) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let text = String::from_utf8(bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "lista no UTF-8"))?;
    Ok(decode(&text))
}

/// Stars (`on`) or unstars `items` and returns the new list. Relative
/// paths are ignored; the list holds at most [`MAX_ITEMS`].
pub fn set(data_home: &Path, items: &[PathBuf], on: bool) -> Result<Vec<PathBuf>, String> {
    let _guard = PROCESS
        .lock()
        .map_err(|_| "bloqueo de destacados envenenado")?;
    let mut list = read(data_home).map_err(|e| format!("Destacados ilegibles: {e}"))?;
    let before = list.clone();
    if on {
        for item in items.iter().filter(|p| p.is_absolute()) {
            if !list.contains(item) {
                if list.len() >= MAX_ITEMS {
                    return Err(format!("Máximo {MAX_ITEMS} destacados"));
                }
                list.push(item.clone());
            }
        }
    } else {
        list.retain(|p| !items.contains(p));
    }
    if list != before {
        crate::persistence::atomic_write(&path(data_home), encode(&list).as_bytes())
            .map_err(|e| e.to_string())?;
    }
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starred_list_round_trips_atomically_and_ignores_junk() {
        use std::os::unix::fs::PermissionsExt;
        let data = crate::persistence::test_dir("starred");
        assert!(read(&data).unwrap().is_empty());
        let odd = PathBuf::from("/home/u/con\nsalto #1 100%");
        let folder = PathBuf::from("/home/u/Proyectos");
        let list = set(&data, &[odd.clone(), folder.clone()], true).unwrap();
        assert_eq!(list, vec![odd.clone(), folder.clone()]);
        assert_eq!(read(&data).unwrap(), list);
        let text = std::fs::read_to_string(path(&data)).unwrap();
        assert_eq!(
            text,
            "# Files starred items, format 1\nfile:///home/u/con%0Asalto%20%231%20100%25\nfile:///home/u/Proyectos\n"
        );
        assert_eq!(
            std::fs::metadata(path(&data)).unwrap().permissions().mode() & 0o777,
            0o600
        );
        // Starring twice and relative paths change nothing.
        assert_eq!(
            set(&data, &[folder.clone(), PathBuf::from("rel")], true).unwrap(),
            list
        );
        assert_eq!(
            set(&data, std::slice::from_ref(&odd), false).unwrap(),
            vec![folder.clone()]
        );
        assert_eq!(read(&data).unwrap(), vec![folder.clone()]);
        // Only the list itself is in Files' folder: no temporaries remain.
        assert_eq!(std::fs::read_dir(data.join("files")).unwrap().count(), 1);
        assert_eq!(
            decode("# comment\n\nnot a uri\nfile:///a\nfile:///a\nfile://host/b\n"),
            vec![PathBuf::from("/a")]
        );
    }

    #[test]
    fn unreadable_list_is_never_rewritten() {
        let data = crate::persistence::test_dir("starred-bad");
        std::fs::create_dir_all(data.join("files")).unwrap();
        let bad = b"# Files starred items, format 1\nfile:///\xff\n\xfe".to_vec();
        std::fs::write(path(&data), &bad).unwrap();
        assert!(read(&data).is_err());
        assert!(set(&data, &[PathBuf::from("/x")], true).is_err());
        assert_eq!(std::fs::read(path(&data)).unwrap(), bad);
    }
}
