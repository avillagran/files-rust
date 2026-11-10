//! Writes to the desktop's `recently-used.xbel` (the list GTK, Qt and most
//! desktop apps share; [`crate::recent`] reads it).
//!
//! Files records a file there after it opened it with an application, and
//! removes an entry when the person asks for it in the "Recientes" listing.
//! Every write is a text splice of the existing document: only the one
//! `<bookmark>` element concerned changes, every other entry, attribute,
//! comment and unknown element stays byte-for-byte. The document is replaced
//! atomically (`0600` temporary in the same folder, fsync, rename, folder
//! fsync) while an advisory `flock` is held on `files/recently-used.lock`
//! under the data folder. Other writers (GLib) do not take that lock; a
//! write that races with one of them loses one side's latest change, never
//! the file.
//!
//! A document larger than [`MAX_REWRITE`], not UTF-8, not a regular file or
//! not structurally sound is never rewritten: the call returns
//! [`Outcome::Skipped`] and the caller logs one line.
//!
//! Callers run these functions off the UI thread.
use std::{
    fs, io,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// Largest document Files rewrites (the persistence helper's limit).
pub const MAX_REWRITE: usize = crate::persistence::MAX_CONFIG_BYTES;
/// Name Files registers itself under in each entry.
pub const APP_NAME: &str = "Files";
/// Command line GLib stores for the registering application.
pub const APP_EXEC: &str = "'files %u'";
const BOOKMARK_NS: &str = "http://www.freedesktop.org/standards/desktop-bookmarks";
const MIME_NS: &str = "http://www.freedesktop.org/standards/shared-mime-info";
const LOCK_WAIT: Duration = Duration::from_secs(2);
static PROCESS: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// What a write did.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Written,
    /// Nothing to change (no entry for that path).
    Unchanged,
    /// Recording is turned off; no file was opened.
    Disabled,
    /// The path is private or ephemeral and is never recorded.
    Private,
    /// The document was left untouched for this reason.
    Skipped(&'static str),
}

/// `recently-used.xbel` under `data_home`.
pub fn xbel_path(data_home: &Path) -> PathBuf {
    data_home.join("recently-used.xbel")
}

/// `file://` URI of an absolute path, percent-encoded like GLib: every byte
/// outside the unreserved set and `/` is escaped (spaces, non-ASCII, `#`,
/// `%`, `?`, `&`...).
pub fn file_uri(path: &Path) -> Option<String> {
    if !path.is_absolute() {
        return None;
    }
    let mut out = String::from("file://");
    for &b in path.as_os_str().as_encoded_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    Some(out)
}

/// The local path of a `file://` URI (`file:///a` or `file://localhost/a`).
pub fn uri_path(uri: &str) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let rest = uri.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    if out.contains(&0) {
        return None;
    }
    Some(PathBuf::from(std::ffi::OsString::from_vec(out)))
}

/// GLib's XBEL time: ISO 8601 UTC with microseconds.
pub fn timestamp(at: SystemTime) -> String {
    let d = at.duration_since(UNIX_EPOCH).unwrap_or_default();
    let base = crate::ofolder::iso_utc(d.as_secs());
    format!("{}.{:06}Z", base.trim_end_matches('Z'), d.subsec_micros())
}

/// Locations whose files are never recorded: trash folders, caches, the
/// runtime folder and Files' own data.
#[derive(Clone, Debug)]
pub struct Exclusions {
    roots: Vec<PathBuf>,
}

impl Exclusions {
    pub fn new(data_home: &Path, cache_home: &Path, runtime: Option<&Path>) -> Self {
        let mut roots = vec![
            data_home.join("Trash"),
            data_home.join("files"),
            cache_home.to_path_buf(),
        ];
        roots.extend(runtime.map(Path::to_path_buf));
        Self { roots }
    }

    /// From the environment, like the rest of Files resolves XDG folders.
    pub fn from_env(data_home: &Path) -> Self {
        let home = crate::theme::home();
        let cache = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".cache"));
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute());
        Self::new(data_home, &cache, runtime.as_deref())
    }

    pub fn excludes(&self, path: &Path) -> bool {
        let trash = path.components().any(|c| {
            let name = c.as_os_str().as_encoded_bytes();
            name == b".Trash" || name.starts_with(b".Trash-")
        });
        trash || self.roots.iter().any(|root| path.starts_with(root))
    }
}

/// Where and whether opened files are recorded. The data folder is injected
/// so tests never touch the real list.
#[derive(Clone, Debug)]
pub struct Recorder {
    pub enabled: bool,
    pub data_home: PathBuf,
    pub exclusions: Exclusions,
}

impl Recorder {
    pub fn from_env(enabled: bool) -> Self {
        let data_home = crate::apps::data_home();
        let exclusions = Exclusions::from_env(&data_home);
        Self {
            enabled,
            data_home,
            exclusions,
        }
    }

    /// Records a file opened with an application. Disabled recording and
    /// private paths return before any file is opened.
    pub fn opened(&self, path: &Path, mime: &str) -> Result<Outcome, String> {
        if !self.enabled {
            return Ok(Outcome::Disabled);
        }
        if !path.is_absolute() || self.exclusions.excludes(path) {
            return Ok(Outcome::Private);
        }
        record(&self.data_home, path, mime, SystemTime::now())
    }
}

/// Adds or refreshes the entry of `path` (count, modified, visited).
pub fn record(
    data_home: &Path,
    path: &Path,
    mime: &str,
    now: SystemTime,
) -> Result<Outcome, String> {
    let Some(uri) = file_uri(path) else {
        return Ok(Outcome::Skipped("ruta relativa"));
    };
    let stamp = timestamp(now);
    update(data_home, true, |xml| {
        let xml = xml.unwrap_or(EMPTY_DOCUMENT);
        apply_record(xml, &uri, path, mime, &stamp).map(Some)
    })
}

/// Removes every entry of `path`. The file itself is never touched.
pub fn remove(data_home: &Path, path: &Path) -> Result<Outcome, String> {
    update(data_home, false, |xml| {
        let Some(xml) = xml else {
            return Ok(None);
        };
        match apply_remove(xml, path)? {
            (_, 0) => Ok(None),
            (text, _) => Ok(Some(text)),
        }
    })
}

const EMPTY_DOCUMENT: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<xbel version=\"1.0\"\n      xmlns:bookmark=\"http://www.freedesktop.org/standards/desktop-bookmarks\"\n      xmlns:mime=\"http://www.freedesktop.org/standards/shared-mime-info\"\n>\n</xbel>\n";

/// Read, edit and replace the document under the locks. `edit` gets the
/// current text (None when absent) and returns the new text, None for no
/// change, or a skip reason.
fn update(
    data_home: &Path,
    create: bool,
    edit: impl FnOnce(Option<&str>) -> Result<Option<String>, &'static str>,
) -> Result<Outcome, String> {
    let target = xbel_path(data_home);
    let _guard = PROCESS
        .lock()
        .map_err(|_| "bloqueo de recientes envenenado")?;
    let lock_dir = data_home.join("files");
    fs::create_dir_all(&lock_dir).map_err(|e| e.to_string())?;
    let _lock = Lock::take(&lock_dir.join("recently-used.lock")).map_err(|e| e.to_string())?;
    let current = match fs::symlink_metadata(&target) {
        Ok(meta) if !meta.file_type().is_file() => {
            return Ok(Outcome::Skipped("no es un archivo normal"));
        }
        Ok(meta) if meta.len() > MAX_REWRITE as u64 => {
            return Ok(Outcome::Skipped("demasiado grande"));
        }
        Ok(_) => match crate::persistence::read_bounded(&target, MAX_REWRITE) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(text) if text.trim().is_empty() => None,
                Ok(text) => Some(text),
                Err(_) => return Ok(Outcome::Skipped("no es UTF-8")),
            },
            Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                return Ok(Outcome::Skipped("demasiado grande"));
            }
            Err(e) => return Err(e.to_string()),
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.to_string()),
    };
    if current.is_none() && !create {
        return Ok(Outcome::Unchanged);
    }
    let next = match edit(current.as_deref()) {
        Ok(Some(text)) => text,
        Ok(None) => return Ok(Outcome::Unchanged),
        Err(reason) => return Ok(Outcome::Skipped(reason)),
    };
    if next.len() > MAX_REWRITE {
        return Ok(Outcome::Skipped("demasiado grande"));
    }
    crate::persistence::atomic_write(&target, next.as_bytes()).map_err(|e| e.to_string())?;
    Ok(Outcome::Written)
}

/// Holds an exclusive `flock` on a Files-owned lock file while alive.
struct Lock(fs::File);

impl Lock {
    fn take(path: &Path) -> io::Result<Self> {
        use std::os::fd::AsRawFd;
        let file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?;
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
                    "otra ventana está guardando los recientes",
                ));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
    }
}

/// One `<bookmark>` element: its byte span, the end of its opening tag
/// (index of `>`), and whether it is self-closing.
struct Element {
    start: usize,
    tag_end: usize,
    end: usize,
    empty: bool,
}

/// Every bookmark element, or a reason the document is not sound enough to
/// edit. Only the structure Files edits is checked.
fn elements(xml: &str) -> Result<Vec<Element>, &'static str> {
    let root = xml.find("<xbel").ok_or("sin elemento xbel")?;
    let close = xml.rfind("</xbel>").ok_or("documento incompleto")?;
    if close < root {
        return Err("documento incompleto");
    }
    let mut out = Vec::new();
    let mut at = root;
    while let Some(found) = xml[at..].find("<bookmark") {
        let start = at + found;
        let next = xml.as_bytes().get(start + 9).copied();
        if !next.is_some_and(|b| b.is_ascii_whitespace() || b == b'>' || b == b'/') {
            // `<bookmark:...>` children outside an element are not ours.
            at = start + 9;
            continue;
        }
        if start > close {
            return Err("entrada fuera del documento");
        }
        let tag_end = start + xml[start..].find('>').ok_or("etiqueta sin cerrar")?;
        let empty = xml[..tag_end].ends_with('/');
        let end = if empty {
            tag_end + 1
        } else {
            let rel = xml[tag_end..]
                .find("</bookmark>")
                .ok_or("entrada sin cerrar")?;
            let body = &xml[tag_end..tag_end + rel];
            if body.contains("<bookmark ") || body.contains("<bookmark>") {
                return Err("entradas anidadas");
            }
            tag_end + rel + "</bookmark>".len()
        };
        if end > close {
            return Err("entrada fuera del documento");
        }
        out.push(Element {
            start,
            tag_end,
            end,
            empty,
        });
        at = end;
    }
    Ok(out)
}

/// Value span of attribute `name` inside `tag` (no entity decoding).
fn attribute_span(tag: &str, name: &str) -> Option<(usize, usize)> {
    let mut from = 0;
    while let Some(found) = tag[from..].find(name) {
        let at = from + found;
        let before = tag[..at].chars().last();
        let after = &tag[at + name.len()..];
        if before.is_some_and(char::is_whitespace) {
            let trimmed = after.trim_start();
            if let Some(rest) = trimmed.strip_prefix('=') {
                let rest_trim = rest.trim_start();
                let quote = rest_trim.chars().next();
                if let Some(q @ ('"' | '\'')) = quote {
                    let value_start = tag.len() - rest_trim.len() + 1;
                    let len = tag[value_start..].find(q)?;
                    return Some((value_start, value_start + len));
                }
            }
        }
        from = at + name.len();
    }
    None
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// `tag` (an opening tag without its final `>`) with `name` set to `value`.
fn set_attribute(tag: &str, name: &str, value: &str) -> String {
    let value = escape(value);
    match attribute_span(tag, name) {
        Some((a, b)) => format!("{}{}{}", &tag[..a], value, &tag[b..]),
        None => {
            let (head, tail) = match tag.strip_suffix('/') {
                Some(head) => (head.trim_end(), "/"),
                None => (tag.trim_end(), ""),
            };
            format!("{head} {name}=\"{value}\"{tail}")
        }
    }
}

fn path_of(xml: &str, e: &Element) -> Option<PathBuf> {
    let tag = &xml[e.start..e.tag_end];
    let (a, b) = attribute_span(tag, "href")?;
    uri_path(&unescape(&tag[a..b]))
}

fn application_tag(stamp: &str, count: u64) -> String {
    format!(
        "<bookmark:application name=\"{}\" exec=\"{}\" modified=\"{stamp}\" count=\"{count}\"/>",
        escape(APP_NAME),
        escape(APP_EXEC)
    )
}

fn info_block(mime: &str, stamp: &str, indent: &str) -> String {
    format!(
        "{indent}  <info>\n{indent}    <metadata owner=\"http://freedesktop.org\">\n{indent}      <mime:mime-type type=\"{}\"/>\n{indent}      <bookmark:applications>\n{indent}        {}\n{indent}      </bookmark:applications>\n{indent}    </metadata>\n{indent}  </info>\n",
        escape(mime),
        application_tag(stamp, 1)
    )
}

/// The root tag with both namespaces declared.
fn with_namespaces(xml: &str) -> Option<String> {
    let root = xml.find("<xbel")?;
    let end = root + xml[root..].find('>')?;
    let mut tag = xml[root..end].to_string();
    for (prefix, ns) in [("xmlns:bookmark", BOOKMARK_NS), ("xmlns:mime", MIME_NS)] {
        if attribute_span(&tag, prefix).is_none() {
            tag = set_attribute(&tag, prefix, ns);
        }
    }
    Some(format!("{}{}{}", &xml[..root], tag, &xml[end..]))
}

/// The document with `path` recorded once more; None never happens for a
/// sound document, Err names why it is not.
pub(crate) fn apply_record(
    xml: &str,
    uri: &str,
    path: &Path,
    mime: &str,
    stamp: &str,
) -> Result<String, &'static str> {
    elements(xml)?;
    let xml = with_namespaces(xml).ok_or("sin elemento xbel")?;
    let found = elements(&xml)?;
    let Some(e) = found
        .iter()
        .find(|e| path_of(&xml, e).as_deref() == Some(path))
    else {
        let close = xml.rfind("</xbel>").ok_or("documento incompleto")?;
        let element = format!(
            "  <bookmark href=\"{}\" added=\"{stamp}\" modified=\"{stamp}\" visited=\"{stamp}\">\n{}  </bookmark>\n",
            escape(uri),
            info_block(mime, stamp, "  ")
        );
        let line_start = xml[..close].rfind('\n').map_or(0, |i| i + 1);
        let insert_at = if xml[line_start..close].trim().is_empty() {
            line_start
        } else {
            close
        };
        return Ok(format!(
            "{}{}{}",
            &xml[..insert_at],
            element,
            &xml[insert_at..]
        ));
    };
    let mut tag = xml[e.start..e.tag_end].to_string();
    for name in ["modified", "visited"] {
        tag = set_attribute(&tag, name, stamp);
    }
    if e.empty {
        // `<bookmark .../>` gains a body with Files' application entry.
        let head = tag.strip_suffix('/').unwrap_or(&tag).trim_end().to_string();
        return Ok(format!(
            "{}{head}>\n{}  </bookmark>{}",
            &xml[..e.start],
            info_block(mime, stamp, "  "),
            &xml[e.end..]
        ));
    }
    let body_start = e.tag_end + 1;
    let body_end = e.end - "</bookmark>".len();
    let body = &xml[body_start..body_end];
    let new_body = if let Some(app) = files_application(body) {
        let app_tag = &body[app.0..app.1];
        let count = attribute_span(app_tag, "count")
            .and_then(|(a, b)| app_tag[a..b].trim().parse::<u64>().ok())
            .unwrap_or(0)
            .saturating_add(1);
        let mut edited = set_attribute(app_tag, "count", &count.to_string());
        let time = if attribute_span(app_tag, "modified").is_none()
            && attribute_span(app_tag, "timestamp").is_some()
        {
            "timestamp"
        } else {
            "modified"
        };
        edited = set_attribute(&edited, time, stamp);
        format!("{}{}{}", &body[..app.0], edited, &body[app.1..])
    } else if let Some(at) = body.find("</bookmark:applications>") {
        format!(
            "{}  {}\n      {}",
            &body[..at],
            application_tag(stamp, 1),
            &body[at..]
        )
    } else if let Some(at) = body.find("</metadata>") {
        format!(
            "{}  <bookmark:applications>\n        {}\n      </bookmark:applications>\n    {}",
            &body[..at],
            application_tag(stamp, 1),
            &body[at..]
        )
    } else if !body.contains("<info") {
        format!(
            "{}\n{}  ",
            body.trim_end(),
            info_block(mime, stamp, "  ").trim_end()
        )
    } else {
        return Err("metadatos desconocidos");
    };
    Ok(format!(
        "{}{}>{}{}",
        &xml[..e.start],
        tag,
        new_body,
        &xml[body_end..]
    ))
}

/// Span of Files' own `<bookmark:application .../>` tag (without `>`) in a body.
fn files_application(body: &str) -> Option<(usize, usize)> {
    let mut from = 0;
    while let Some(found) = body[from..].find("<bookmark:application ") {
        let start = from + found;
        let end = start + body[start..].find('>')?;
        let tag = &body[start..end];
        if attribute_span(tag, "name").is_some_and(|(a, b)| unescape(&tag[a..b]) == APP_NAME) {
            return Some((start, end));
        }
        from = end;
    }
    None
}

/// The document without any entry for `path`, and how many were removed.
pub(crate) fn apply_remove(xml: &str, path: &Path) -> Result<(String, usize), &'static str> {
    let found = elements(xml)?;
    let mut out = String::with_capacity(xml.len());
    let mut at = 0;
    let mut removed = 0;
    for e in &found {
        if path_of(xml, e).as_deref() != Some(path) {
            continue;
        }
        // Take the element's own indentation and line break with it.
        let line_start = xml[..e.start].rfind('\n').map_or(0, |i| i + 1);
        let start = if xml[line_start..e.start].trim().is_empty() && line_start >= at {
            line_start
        } else {
            e.start
        };
        let end = if xml[e.end..].starts_with('\n') && start == line_start {
            e.end + 1
        } else {
            e.end
        };
        out.push_str(&xml[at..start]);
        at = end;
        removed += 1;
    }
    out.push_str(&xml[at..]);
    Ok((out, removed))
}

/// Recent entries the person removed from the listing although another
/// program keeps them (Ofolder's recents): hidden until used again.
///
/// `$XDG_DATA_HOME/files/recent-hidden`, UTF-8 text: a `# Files hidden
/// recents, format 1` header, then one `<unix seconds> <file URI>` per line.
pub mod hidden {
    use super::*;

    pub const MAX_LINES: usize = 1000;
    const HEADER: &str = "# Files hidden recents, format 1";

    pub fn path(data_home: &Path) -> PathBuf {
        data_home.join("files/recent-hidden")
    }

    pub fn decode(text: &str) -> Vec<(PathBuf, u64)> {
        text.lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| {
                let (at, uri) = l.split_once(' ')?;
                Some((uri_path(uri.trim())?, at.parse().ok()?))
            })
            .collect()
    }

    pub fn read(data_home: &Path) -> Vec<(PathBuf, u64)> {
        crate::persistence::read_bounded(&path(data_home), MAX_REWRITE)
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
            .map(|t| decode(&t))
            .unwrap_or_default()
    }

    /// Hides `item` for uses up to `at` (Unix seconds).
    pub fn add(data_home: &Path, item: &Path, at: u64) -> Result<(), String> {
        let _guard = PROCESS
            .lock()
            .map_err(|_| "bloqueo de recientes envenenado")?;
        let mut list = read(data_home);
        list.retain(|(p, _)| p != item);
        list.push((item.to_path_buf(), at));
        let skip = list.len().saturating_sub(MAX_LINES);
        let mut text = format!("{HEADER}\n");
        for (p, at) in list.iter().skip(skip) {
            if let Some(uri) = file_uri(p) {
                text.push_str(&format!("{at} {uri}\n"));
            }
        }
        crate::persistence::atomic_write(&path(data_home), text.as_bytes())
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAMP: &str = "2026-11-10T12:00:00.000001Z";

    const FOREIGN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<xbel version="1.0"
      xmlns:bookmark="http://www.freedesktop.org/standards/desktop-bookmarks"
      xmlns:mime="http://www.freedesktop.org/standards/shared-mime-info"
>
  <!-- written by another app -->
  <bookmark href="file:///home/u/Documents/informe%20final.odt" added="2026-10-01T10:00:00.000000Z" modified="2026-10-01T10:00:00.000000Z" visited="2026-10-01T10:00:00.000000Z" x-custom='kept'>
    <info>
      <metadata owner="http://freedesktop.org">
        <mime:mime-type type="application/vnd.oasis.opendocument.text"/>
        <bookmark:groups><bookmark:group>LibreOffice</bookmark:group></bookmark:groups>
        <bookmark:applications>
          <bookmark:application name="LibreOffice" exec="&apos;soffice %u&apos;" modified="2026-10-01T10:00:00.000000Z" count="3"/>
        </bookmark:applications>
      </metadata>
      <metadata owner="http://example.com/unknown"><x:thing a="1"/></metadata>
    </info>
  </bookmark>
  <bookmark href="https://example.com/page" added="2026-10-02T00:00:00Z" modified="2026-10-02T00:00:00Z" visited="2026-10-02T00:00:00Z"/>
</xbel>
"#;

    #[test]
    fn uris_percent_encode_spaces_non_ascii_hash_and_percent_and_round_trip() {
        let p = Path::new("/home/u/año 100% #1 & más?.txt");
        let uri = file_uri(p).unwrap();
        assert_eq!(
            uri,
            "file:///home/u/a%C3%B1o%20100%25%20%231%20%26%20m%C3%A1s%3F.txt"
        );
        assert_eq!(uri_path(&uri).as_deref(), Some(p));
        assert_eq!(
            uri_path("file://localhost/a%20b").as_deref(),
            Some(Path::new("/a b"))
        );
        assert!(file_uri(Path::new("relative")).is_none());
        assert!(uri_path("file://host/x").is_none());
        assert!(uri_path("file:///bad%2").is_none());
        assert!(uri_path("file:///nul%00").is_none());
        // Non-UTF-8 names survive byte for byte.
        use std::os::unix::ffi::OsStrExt;
        let raw = Path::new(std::ffi::OsStr::from_bytes(b"/tmp/\xff\xfe"));
        assert_eq!(uri_path(&file_uri(raw).unwrap()).as_deref(), Some(raw));
    }

    #[test]
    fn timestamps_have_microseconds_like_glib() {
        let at = UNIX_EPOCH + Duration::new(1_762_776_000, 123_456_000);
        assert_eq!(timestamp(at), "2025-11-10T12:00:00.123456Z");
    }

    #[test]
    fn new_entry_keeps_every_foreign_byte_and_is_well_formed() {
        let path = Path::new("/home/u/nuevo #2.pdf");
        let uri = file_uri(path).unwrap();
        let out = apply_record(FOREIGN, &uri, path, "application/pdf", STAMP).unwrap();
        // The original document is a prefix-and-suffix of the result: only
        // the new element was inserted before </xbel>.
        let close = FOREIGN.rfind("</xbel>").unwrap();
        assert!(out.starts_with(&FOREIGN[..close]));
        assert!(out.ends_with(&FOREIGN[close..]));
        let inserted = &out[close..out.len() - (FOREIGN.len() - close)];
        assert!(inserted.contains(&format!("href=\"{uri}\"")));
        assert!(inserted.contains(r#"<mime:mime-type type="application/pdf"/>"#));
        assert!(inserted.contains(
            r#"<bookmark:application name="Files" exec="&apos;files %u&apos;" modified="2026-11-10T12:00:00.000001Z" count="1"/>"#
        ));
        // The reader sees it, newest first, beside the foreign file entry.
        let read = crate::recent::parse(&out);
        assert_eq!(read[0].path, path);
        assert_eq!(
            read[1].path,
            Path::new("/home/u/Documents/informe final.odt")
        );
        assert_eq!(elements(&out).unwrap().len(), 3);
    }

    #[test]
    fn existing_entry_gains_files_application_and_keeps_foreign_metadata() {
        let path = Path::new("/home/u/Documents/informe final.odt");
        let uri = file_uri(path).unwrap();
        let once = apply_record(FOREIGN, &uri, path, "x/ignored", STAMP).unwrap();
        for kept in [
            "x-custom='kept'",
            "added=\"2026-10-01T10:00:00.000000Z\"",
            "<bookmark:groups><bookmark:group>LibreOffice</bookmark:group></bookmark:groups>",
            r#"<bookmark:application name="LibreOffice" exec="&apos;soffice %u&apos;" modified="2026-10-01T10:00:00.000000Z" count="3"/>"#,
            r#"<metadata owner="http://example.com/unknown"><x:thing a="1"/></metadata>"#,
            "<!-- written by another app -->",
            r#"<bookmark href="https://example.com/page" added="2026-10-02T00:00:00Z" modified="2026-10-02T00:00:00Z" visited="2026-10-02T00:00:00Z"/>"#,
        ] {
            assert!(once.contains(kept), "lost {kept}\n{once}");
        }
        assert!(once.contains(&format!("modified=\"{STAMP}\" visited=\"{STAMP}\"")));
        assert!(!once.contains("x/ignored"), "mime of a known entry is kept");
        assert!(once.contains("name=\"Files\""));
        assert!(once.contains("count=\"1\""));
        let later = "2026-11-11T08:00:00.000000Z";
        let twice = apply_record(&once, &uri, path, "x/ignored", later).unwrap();
        assert!(twice.contains(&format!(
            r#"<bookmark:application name="Files" exec="&apos;files %u&apos;" modified="{later}" count="2"/>"#
        )));
        assert_eq!(twice.matches("name=\"Files\"").count(), 1);
        assert_eq!(elements(&twice).unwrap().len(), 2);
        // Removing it again leaves the rest of the document as it was.
        let (removed, n) = apply_remove(&twice, path).unwrap();
        assert_eq!(n, 1);
        let other = FOREIGN.find("  <bookmark href=\"https").unwrap();
        let first = FOREIGN.find("  <bookmark href=\"file").unwrap();
        assert_eq!(
            removed,
            format!("{}{}", &FOREIGN[..first], &FOREIGN[other..])
        );
    }

    #[test]
    fn self_closing_entry_and_missing_namespace_are_completed() {
        let xml = "<xbel version=\"1.0\">\n  <bookmark href=\"file:///a\" added=\"x\"/>\n</xbel>\n";
        let out = apply_record(xml, "file:///a", Path::new("/a"), "text/plain", STAMP).unwrap();
        assert!(
            out.contains(
                "xmlns:bookmark=\"http://www.freedesktop.org/standards/desktop-bookmarks\""
            )
        );
        assert!(out.contains("xmlns:mime="));
        assert!(out.contains("added=\"x\""));
        assert!(out.contains("</bookmark>"));
        assert_eq!(elements(&out).unwrap().len(), 1);
    }

    #[test]
    fn malformed_documents_are_refused() {
        for bad in [
            "",
            "<xbel>",
            "<xbel><bookmark href=\"file:///a\"></xbel>",
            "<xbel><bookmark href=\"file:///a\"><bookmark href=\"file:///b\"/></bookmark></xbel>",
            "not xml at all",
        ] {
            assert!(
                apply_record(bad, "file:///a", Path::new("/a"), "t/p", STAMP).is_err(),
                "{bad}"
            );
            assert!(apply_remove(bad, Path::new("/a")).is_err(), "{bad}");
        }
    }

    fn private_root(label: &str) -> PathBuf {
        crate::persistence::test_dir(label)
    }

    #[test]
    fn record_creates_updates_and_removes_with_private_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let data = private_root("recent-store");
        let file = data.join("visto con espacio.txt");
        fs::write(&file, b"x").unwrap();
        let t = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        assert_eq!(record(&data, &file, "text/plain", t), Ok(Outcome::Written));
        let xbel = xbel_path(&data);
        assert_eq!(
            fs::metadata(&xbel).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let listed = crate::recent::read(&data).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].path, file);
        assert_eq!(record(&data, &file, "text/plain", t), Ok(Outcome::Written));
        assert!(fs::read_to_string(&xbel).unwrap().contains("count=\"2\""));
        assert_eq!(remove(&data, &file), Ok(Outcome::Written));
        assert_eq!(remove(&data, &file), Ok(Outcome::Unchanged));
        assert!(file.exists(), "removing an entry never touches the file");
        assert!(crate::recent::read(&data).unwrap().is_empty());
        // No temporaries left behind; only the lock beside Files' data.
        let names: Vec<_> = fs::read_dir(&data)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 3, "{names:?}");
    }

    #[test]
    fn huge_unparseable_or_linked_documents_are_left_untouched() {
        let data = private_root("recent-untouched");
        let xbel = xbel_path(&data);
        let file = data.join("a.txt");
        fs::write(&file, b"x").unwrap();
        let t = SystemTime::now();
        let broken = b"<xbel><bookmark href=\"file:///a\"></xbel>".to_vec();
        fs::write(&xbel, &broken).unwrap();
        assert!(matches!(
            record(&data, &file, "text/plain", t),
            Ok(Outcome::Skipped(_))
        ));
        assert_eq!(fs::read(&xbel).unwrap(), broken);
        let mut huge = b"<xbel>".to_vec();
        huge.resize(MAX_REWRITE + 10, b' ');
        huge.extend_from_slice(b"</xbel>");
        fs::write(&xbel, &huge).unwrap();
        assert_eq!(
            record(&data, &file, "text/plain", t),
            Ok(Outcome::Skipped("demasiado grande"))
        );
        assert_eq!(fs::metadata(&xbel).unwrap().len(), huge.len() as u64);
        let latin1 = b"<xbel>\xe9</xbel>".to_vec();
        fs::write(&xbel, &latin1).unwrap();
        assert_eq!(
            record(&data, &file, "text/plain", t),
            Ok(Outcome::Skipped("no es UTF-8"))
        );
        assert_eq!(fs::read(&xbel).unwrap(), latin1);
        // A link in place of the list is not followed or replaced; it points
        // inside this private folder only.
        let real = data.join("real.xbel");
        fs::write(&real, EMPTY_DOCUMENT).unwrap();
        fs::rename(&xbel, data.join("old.xbel")).unwrap();
        std::os::unix::fs::symlink(&real, &xbel).unwrap();
        assert_eq!(
            record(&data, &file, "text/plain", t),
            Ok(Outcome::Skipped("no es un archivo normal"))
        );
        assert_eq!(fs::read_to_string(&real).unwrap(), EMPTY_DOCUMENT);
        assert!(
            fs::symlink_metadata(&xbel)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn disabled_recording_and_private_paths_write_nothing() {
        let data = private_root("recent-disabled");
        let cache = data.join("cache");
        let runtime = data.join("run");
        let file = data.join("doc.txt");
        fs::write(&file, b"x").unwrap();
        let off = Recorder {
            enabled: false,
            data_home: data.clone(),
            exclusions: Exclusions::new(&data, &cache, Some(&runtime)),
        };
        assert_eq!(off.opened(&file, "text/plain"), Ok(Outcome::Disabled));
        assert!(!xbel_path(&data).exists());
        assert!(!data.join("files").exists(), "not even the lock is created");
        let on = Recorder {
            enabled: true,
            ..off
        };
        for private in [
            data.join("Trash/files/x.txt"),
            data.join("files/starred"),
            cache.join("thumbs/x.png"),
            runtime.join("doc.txt"),
            PathBuf::from("/media/usb/.Trash-1000/files/x"),
            PathBuf::from("relative.txt"),
        ] {
            assert_eq!(on.opened(&private, "text/plain"), Ok(Outcome::Private));
        }
        assert!(!xbel_path(&data).exists());
        assert_eq!(on.opened(&file, "text/plain"), Ok(Outcome::Written));
        assert!(xbel_path(&data).exists());
    }

    #[test]
    fn hidden_recents_round_trip_and_stay_bounded() {
        let data = private_root("recent-hidden");
        assert!(hidden::read(&data).is_empty());
        hidden::add(&data, Path::new("/a b/c#d"), 10).unwrap();
        hidden::add(&data, Path::new("/x"), 20).unwrap();
        hidden::add(&data, Path::new("/a b/c#d"), 30).unwrap();
        assert_eq!(
            hidden::read(&data),
            vec![(PathBuf::from("/x"), 20), (PathBuf::from("/a b/c#d"), 30)]
        );
    }
}
