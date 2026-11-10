//! Recently used files from the XDG `recently-used.xbel` (GTK, Qt and most
//! desktop apps write it). Read-only and bounded: the file is read up to
//! `MAX_BYTES`, only local `file://` entries are kept, newest first, at most
//! `MAX_ITEMS`. Callers run `read` off the UI thread.
use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// Largest `recently-used.xbel` read (bigger files are cut there).
pub const MAX_BYTES: u64 = 8 * 1024 * 1024;
/// Entries returned for the quick menu.
pub const MAX_ITEMS: usize = 40;
/// Entries of the browsable "Recientes" listing.
pub const MAX_LISTED: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recent {
    pub path: PathBuf,
    /// ISO 8601 UTC as written by the app (`2026-11-05T21:00:00.123456Z`);
    /// sortable as text. `modified`, else `visited`, else `added`.
    pub when: String,
    /// A folder (Ofolder records recent folders too); the desktop list holds files.
    pub folder: bool,
}

/// The value of attribute `name` in a tag's text, XML entities decoded.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let mut rest = tag;
    loop {
        let at = rest.find(name)?;
        let before = rest[..at].chars().last();
        let after = &rest[at + name.len()..];
        if before.is_some_and(char::is_whitespace) && after.starts_with("=\"") {
            let value = &after[2..];
            let end = value.find('"')?;
            return Some(unescape(&value[..end]));
        }
        rest = after;
    }
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Entries of an xbel document, newest first, one per path.
pub fn parse(xml: &str) -> Vec<Recent> {
    let mut out: Vec<Recent> = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<bookmark ") {
        let tag_body = &rest[start..];
        let Some(end) = tag_body.find('>') else { break };
        let tag = &tag_body[..end];
        rest = &tag_body[end..];
        let Some(href) = attribute(tag, "href") else {
            continue;
        };
        if !href.starts_with("file://") {
            continue;
        }
        let Ok(files) =
            crate::file_transfer::decode(crate::file_transfer::Format::UriList, href.as_bytes())
        else {
            continue;
        };
        let Some(path) = files.paths.into_iter().next() else {
            continue;
        };
        let when = ["modified", "visited", "added"]
            .iter()
            .find_map(|a| attribute(tag, a))
            .unwrap_or_default();
        match out.iter_mut().find(|r| r.path == path) {
            Some(r) if r.when < when => r.when = when,
            Some(_) => {}
            None => out.push(Recent {
                path,
                when,
                folder: false,
            }),
        }
    }
    out.sort_by(|a, b| b.when.cmp(&a.when));
    out
}

/// The newest existing regular files listed in `data_home/recently-used.xbel`.
/// A missing file is an empty list.
pub fn read(data_home: &Path) -> io::Result<Vec<Recent>> {
    Ok(read_raw(data_home)?
        .into_iter()
        .filter(|r| std::fs::metadata(&r.path).is_ok_and(|m| m.is_file()))
        .take(MAX_ITEMS)
        .collect())
}

/// The newest existing files and folders listed in the document, at most
/// `max`, for the browsable listing (folders are marked as such).
pub fn read_listing(data_home: &Path, max: usize) -> io::Result<Vec<Recent>> {
    Ok(read_raw(data_home)?
        .into_iter()
        .filter_map(|mut r| {
            let meta = std::fs::metadata(&r.path).ok()?;
            r.folder = meta.is_dir();
            Some(r)
        })
        .take(max)
        .collect())
}

fn read_raw(data_home: &Path) -> io::Result<Vec<Recent>> {
    let file = match std::fs::File::open(data_home.join("recently-used.xbel")) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut bytes = Vec::new();
    file.take(MAX_BYTES).read_to_end(&mut bytes)?;
    let xml = String::from_utf8_lossy(&bytes);
    Ok(parse(&xml))
}

/// `2026-11-05T21:03:09.12Z` → `2026-11-05 21:03` (UTC as written).
pub fn short_time(when: &str) -> String {
    match (when.get(..10), when.get(11..16)) {
        (Some(date), Some(time)) => format!("{date} {time}"),
        _ => when.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const XBEL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<xbel version="1.0" xmlns:bookmark="http://www.freedesktop.org/standards/desktop-bookmarks">
  <bookmark href="file:///home/u/old.txt" added="2026-10-01T10:00:00Z" modified="2026-10-01T10:00:00Z" visited="2026-10-01T10:00:00Z">
  </bookmark>
  <bookmark href="file:///home/u/con%20espacio%20%26%20m%C3%A1s.pdf" added="2026-11-01T10:00:00Z" modified="2026-11-05T21:03:09.120000Z" visited="2026-11-05T21:03:09Z">
  </bookmark>
  <bookmark href="https://example.com/x" added="2026-11-06T00:00:00Z" modified="2026-11-06T00:00:00Z" visited="2026-11-06T00:00:00Z"/>
  <bookmark href="file:///home/u/old.txt" added="2026-11-02T10:00:00Z" modified="2026-11-02T10:00:00Z" visited="2026-11-02T10:00:00Z"/>
  <bookmark href="file:///home/u/a&amp;b.txt" added="2026-11-03T00:00:00Z" modified="2026-11-03T00:00:00Z" visited="2026-11-03T00:00:00Z"/>
</xbel>"#;

    #[test]
    fn parses_local_entries_newest_first_once_each() {
        let got = parse(XBEL);
        let paths: Vec<_> = got
            .iter()
            .map(|r| r.path.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            paths,
            [
                "/home/u/con espacio & más.pdf",
                "/home/u/a&b.txt",
                "/home/u/old.txt"
            ]
        );
        // The later of two entries for one path wins.
        assert_eq!(got[2].when, "2026-11-02T10:00:00Z");
        assert_eq!(short_time(&got[0].when), "2026-11-05 21:03");
    }

    #[test]
    fn malformed_and_truncated_documents_do_not_panic() {
        for cut in 0..XBEL.len() {
            if XBEL.is_char_boundary(cut) {
                let _ = parse(&XBEL[..cut]);
            }
        }
        assert!(parse("<bookmark href=\"file://relative\">").is_empty());
        assert!(parse("<bookmark nohref=\"file:///a\">").is_empty());
    }

    #[test]
    fn read_keeps_existing_files_and_missing_document_is_empty() {
        let dir = crate::persistence::test_dir("recent-read");
        assert!(read(&dir).unwrap().is_empty());
        let file = dir.join("visto.txt");
        std::fs::write(&file, b"x").unwrap();
        let gone = dir.join("borrado.txt");
        let xml = format!(
            "<xbel><bookmark href=\"file://{}\" modified=\"2026-11-05T10:00:00Z\"/><bookmark href=\"file://{}\" modified=\"2026-11-05T11:00:00Z\"/><bookmark href=\"file://{}\" modified=\"2026-11-05T12:00:00Z\"/></xbel>",
            file.display(),
            gone.display(),
            dir.display()
        );
        std::fs::write(dir.join("recently-used.xbel"), xml).unwrap();
        let got = read(&dir).unwrap();
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].path, file);
        std::fs::remove_file(dir.join("recently-used.xbel")).unwrap();
        std::fs::remove_file(&file).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }
}
