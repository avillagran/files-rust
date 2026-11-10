use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread,
    time::SystemTime,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Directory,
    File,
    Symlink,
    Other,
}
#[derive(Clone, Debug)]
pub struct Metadata {
    pub size: u64,
    pub modified: SystemTime,
}
/// What a symbolic link pointed at when the listing read it. Browsing-only
/// information: it never turns the link into a directory for code that walks,
/// measures or mutates trees.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    /// The raw link text (`readlink`), not canonicalized; `None` when unreadable.
    pub target: Option<PathBuf>,
    /// The link resolved (one `stat`) to a directory. Broken or looping links
    /// are `false`.
    pub target_dir: bool,
}
#[derive(Clone, Debug)]
pub struct Entry {
    pub name: OsString,
    pub kind: Kind,
    pub metadata: Option<Metadata>,
    pub stat_attempted: bool,
    /// Set only for `Kind::Symlink` entries produced by [`LocalVfs::list`].
    pub link: Option<Box<Link>>,
}
impl Entry {
    pub fn load_metadata(&mut self, parent: &Path) -> io::Result<()> {
        if self.stat_attempted {
            return Ok(());
        }
        self.stat_attempted = true;
        self.metadata = Some(LocalVfs.stat(&parent.join(&self.name))?);
        Ok(())
    }
    /// A real directory. Symbolic links are never directories here, even when
    /// they point at one: recursive and mutating code must rely on this.
    pub fn is_dir(&self) -> bool {
        self.kind == Kind::Directory
    }
    /// A directory, or a symbolic link that resolved to one when listed.
    /// Use only for browsing: opening/navigating, icons, folders-first order,
    /// column descent and tab opening. Never to decide whether to descend in
    /// copy, move, delete, trash, size, search, compare, sync or archive work.
    pub fn browses_as_dir(&self) -> bool {
        self.is_dir()
            || (self.kind == Kind::Symlink && self.link.as_ref().is_some_and(|l| l.target_dir))
    }
    /// A symbolic link that resolved to a directory when listed.
    pub fn is_dir_link(&self) -> bool {
        !self.is_dir() && self.browses_as_dir()
    }
}
/// Reads a link's text and resolves it once, without walking anything.
pub fn link_info(path: &Path) -> Link {
    Link {
        target: fs::read_link(path).ok(),
        target_dir: fs::metadata(path).is_ok_and(|m| m.is_dir()),
    }
}
pub trait Vfs: Send + Sync {
    fn list(&self, path: &Path) -> io::Result<Box<dyn Iterator<Item = io::Result<Entry>> + Send>>;
    fn stat(&self, path: &Path) -> io::Result<Metadata>;
    fn open_read(&self, path: &Path) -> io::Result<Box<dyn io::Read + Send>>;
    // Contratos para futuros backends; ninguna operación mutante se habilita en 0.0.1.
    fn open_write(&self, _path: &Path) -> io::Result<Box<dyn io::Write + Send>> {
        Err(unsupported())
    }
    fn rename(&self, _from: &Path, _to: &Path) -> io::Result<()> {
        Err(unsupported())
    }
    fn remove(&self, _path: &Path) -> io::Result<()> {
        Err(unsupported())
    }
    fn mkdir(&self, _path: &Path) -> io::Result<()> {
        Err(unsupported())
    }
    fn watch(
        &self,
        _path: &Path,
    ) -> io::Result<Box<dyn Iterator<Item = io::Result<PathBuf>> + Send>> {
        Err(unsupported())
    }
}
fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "Disponible en una entrega posterior",
    )
}
pub struct LocalVfs;
impl Vfs for LocalVfs {
    fn list(&self, path: &Path) -> io::Result<Box<dyn Iterator<Item = io::Result<Entry>> + Send>> {
        Ok(Box::new(fs::read_dir(path)?.map(|item| {
            let item = item?;
            let t = item.file_type()?;
            let link = t.is_symlink().then(|| Box::new(link_info(&item.path())));
            Ok(Entry {
                name: item.file_name(),
                kind: if t.is_dir() {
                    Kind::Directory
                } else if t.is_symlink() {
                    Kind::Symlink
                } else if t.is_file() {
                    Kind::File
                } else {
                    Kind::Other
                },
                metadata: None,
                stat_attempted: false,
                link,
            })
        })))
    }
    fn stat(&self, path: &Path) -> io::Result<Metadata> {
        let m = fs::metadata(path)?;
        Ok(Metadata {
            size: m.len(),
            modified: m.modified()?,
        })
    }
    fn open_read(&self, path: &Path) -> io::Result<Box<dyn io::Read + Send>> {
        Ok(Box::new(fs::File::open(path)?))
    }
}
pub enum ListingEvent {
    Batch(Vec<Entry>),
    Error(String),
    Done,
}
pub struct Listing {
    pub receiver: Receiver<ListingEvent>,
    cancel: Arc<AtomicBool>,
}
impl Drop for Listing {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
fn send(tx: &SyncSender<ListingEvent>, mut event: ListingEvent, cancel: &AtomicBool) -> bool {
    loop {
        if cancel.load(Ordering::Relaxed) {
            return false;
        }
        match tx.try_send(event) {
            Ok(()) => return true,
            Err(TrySendError::Disconnected(_)) => return false,
            Err(TrySendError::Full(e)) => {
                event = e;
                thread::park_timeout(std::time::Duration::from_millis(2));
            }
        }
    }
}
pub fn stream(path: PathBuf) -> Listing {
    let (tx, receiver) = mpsc::sync_channel(4);
    let cancel = Arc::new(AtomicBool::new(false));
    let c = cancel.clone();
    thread::spawn(move || {
        match LocalVfs.list(&path) {
            Ok(items) => {
                let mut batch = Vec::with_capacity(256);
                for item in items {
                    if c.load(Ordering::Relaxed) {
                        return;
                    }
                    match item {
                        Ok(e) => batch.push(e),
                        Err(e) => {
                            if !send(&tx, ListingEvent::Error(e.to_string()), &c) {
                                return;
                            }
                        }
                    }
                    if batch.len() == 256
                        && !send(
                            &tx,
                            ListingEvent::Batch(std::mem::replace(
                                &mut batch,
                                Vec::with_capacity(256),
                            )),
                            &c,
                        )
                    {
                        return;
                    }
                }
                if !batch.is_empty() {
                    send(&tx, ListingEvent::Batch(batch), &c);
                }
            }
            Err(e) => {
                send(&tx, ListingEvent::Error(e.to_string()), &c);
            }
        }
        send(&tx, ListingEvent::Done, &c);
    });
    Listing { receiver, cancel }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Name,
    Size,
    Modified,
    /// By kind (extension, lowercase), then name: Nautilus "Type".
    Type,
}
/// Sort key of [`Sort::Type`]: the lowercase extension ("" first).
pub fn type_key(name: &std::ffi::OsStr) -> String {
    let n = name.to_string_lossy();
    match n.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => ext.to_lowercase(),
        _ => String::new(),
    }
}
/// Known totals precede unknown observations in either direction. Zero is known.
pub fn compare_sizes(a: Option<u64>, b: Option<u64>, descending: bool) -> std::cmp::Ordering {
    match (a, b) {
        (Some(a), Some(b)) => {
            if descending {
                b.cmp(&a)
            } else {
                a.cmp(&b)
            }
        }
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}
pub fn sort(entries: &mut [Entry], by: Sort, descending: bool) {
    entries.sort_by_cached_key(|e| crate::natural_sort::NaturalKey::new(&e.name));
    if by != Sort::Name {
        entries.sort_by(|a, b| {
            let order = match by {
                Sort::Size => {
                    let known = |e: &Entry| {
                        if e.browses_as_dir() {
                            None
                        } else {
                            e.metadata.as_ref().map(|m| m.size)
                        }
                    };
                    return compare_sizes(known(a), known(b), descending);
                }
                Sort::Modified => a
                    .metadata
                    .as_ref()
                    .map(|m| m.modified)
                    .cmp(&b.metadata.as_ref().map(|m| m.modified)),
                Sort::Type => type_key(&a.name).cmp(&type_key(&b.name)),
                _ => std::cmp::Ordering::Equal,
            };
            if descending { order.reverse() } else { order }
        });
    } else if descending {
        entries.reverse();
    }
    entries.sort_by_key(|e| !e.browses_as_dir());
}
/// Filtro de subsecuencia Unicode; bonifica coincidencias contiguas y comienzos de palabra.
pub fn fuzzy_score(name: &str, query: &str) -> Option<i32> {
    let mut chars = name.chars().flat_map(char::to_lowercase).enumerate();
    let mut score = 0;
    let mut previous = None;
    for q in query.chars().flat_map(char::to_lowercase) {
        let (pos, _) = chars.find(|(_, c)| *c == q)?;
        score += 10;
        if previous.is_some_and(|p| p + 1 == pos) {
            score += 8;
        }
        if pos == 0 {
            score += 15;
        }
        score -= pos.min(100) as i32;
        previous = Some(pos);
    }
    Some(score)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn size_order_ignores_directory_inode_lengths_and_keeps_unknown_after_zero() {
        let row = |name: &str, kind, size: Option<u64>| Entry {
            name: name.into(),
            kind,
            metadata: size.map(|size| Metadata {
                size,
                modified: SystemTime::UNIX_EPOCH,
            }),
            stat_attempted: true,
            link: None,
        };
        let mut rows = vec![
            row("z-dir", Kind::Directory, Some(1)),
            row("a-dir", Kind::Directory, Some(9999)),
            row("zero", Kind::File, Some(0)),
            row("seven", Kind::File, Some(7)),
            row("unknown", Kind::File, None),
        ];
        for (descending, expected) in [
            (false, vec!["a-dir", "z-dir", "zero", "seven", "unknown"]),
            (true, vec!["a-dir", "z-dir", "seven", "zero", "unknown"]),
        ] {
            sort(&mut rows, Sort::Size, descending);
            assert_eq!(
                rows.iter()
                    .map(|e| e.name.to_str().unwrap())
                    .collect::<Vec<_>>(),
                expected
            );
        }
    }

    /// Fresh, exclusive fixture under the private test TMPDIR. Never removed.
    #[cfg(unix)]
    fn link_fixture(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "files-links-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        dir
    }

    #[cfg(unix)]
    #[test]
    fn directory_links_browse_as_folders_without_becoming_directories() {
        use std::os::unix::fs::symlink;
        let dir = link_fixture("browse");
        fs::create_dir(dir.join("real")).unwrap();
        fs::write(dir.join("a-file.txt"), b"x").unwrap();
        // Every link target is relative and stays inside the private fixture.
        symlink("real", dir.join("dir-link")).unwrap();
        symlink("a-file.txt", dir.join("file-link")).unwrap();
        symlink("missing", dir.join("broken-link")).unwrap();
        symlink("loop-link", dir.join("loop-link")).unwrap();
        let mut entries: Vec<Entry> = LocalVfs.list(&dir).unwrap().map(Result::unwrap).collect();
        let get = |entries: &[Entry], name: &str| {
            entries
                .iter()
                .find(|e| e.name == name)
                .cloned()
                .unwrap_or_else(|| panic!("{name} listed"))
        };
        let link = get(&entries, "dir-link");
        assert_eq!(link.kind, Kind::Symlink);
        assert!(!link.is_dir(), "is_dir must keep meaning real directory");
        assert!(link.browses_as_dir() && link.is_dir_link());
        assert_eq!(
            link.link.as_deref().unwrap().target.as_deref(),
            Some(Path::new("real"))
        );
        for name in ["file-link", "broken-link", "loop-link"] {
            let e = get(&entries, name);
            assert_eq!(e.kind, Kind::Symlink, "{name}");
            assert!(!e.is_dir() && !e.browses_as_dir(), "{name}");
        }
        let real = get(&entries, "real");
        assert!(real.is_dir() && real.browses_as_dir() && real.link.is_none());
        assert!(!get(&entries, "a-file.txt").browses_as_dir());
        for by in [Sort::Name, Sort::Size, Sort::Type] {
            sort(&mut entries, by, false);
            let names: Vec<_> = entries.iter().map(|e| e.name.to_str().unwrap()).collect();
            assert_eq!(&names[..2], ["dir-link", "real"], "{by:?} folders first");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn recursive_capture_records_directory_link_without_descending() {
        use std::os::unix::fs::symlink;
        let dir = link_fixture("capture");
        fs::create_dir_all(dir.join("tree/real")).unwrap();
        fs::write(dir.join("tree/real/inner.txt"), b"data").unwrap();
        symlink("real", dir.join("tree/link")).unwrap();
        let listed: Vec<Entry> = LocalVfs
            .list(&dir.join("tree"))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert!(
            listed
                .iter()
                .any(|e| e.name == "link" && e.browses_as_dir())
        );
        // The copy/move source inventory is the recursive plan: it must keep
        // the link as one link entry and never walk through it.
        let parent = fs::File::open(&dir).unwrap();
        let cancel = AtomicBool::new(false);
        let inventory =
            crate::exdev_source::capture(&parent, std::ffi::OsStr::new("tree"), &cancel).unwrap();
        let mut relative: Vec<_> = inventory
            .entries()
            .iter()
            .map(|e| String::from_utf8(e.relative.clone()).unwrap())
            .collect();
        relative.sort();
        assert_eq!(
            relative,
            ["tree", "tree/link", "tree/real", "tree/real/inner.txt"]
        );
        let link = inventory
            .entries()
            .iter()
            .find(|e| e.relative == b"tree/link")
            .unwrap();
        assert_eq!(link.identity.kind, crate::move_manifest::Kind::Symlink);
    }

    #[test]
    fn ten_thousand_streaming_lazy_sort_filter() {
        let dir = crate::test_support::private_dir("core-listing");
        for i in 0..10_000 {
            fs::write(dir.join(format!("file-{i:05}.txt")), b"test").unwrap();
        }
        fs::create_dir(dir.join("zzz-folder")).unwrap();
        let listing = stream(dir.clone());
        let mut entries = Vec::new();
        loop {
            match listing
                .receiver
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap()
            {
                ListingEvent::Batch(b) => {
                    assert!(b.len() <= 256);
                    entries.extend(b);
                }
                ListingEvent::Done => break,
                ListingEvent::Error(e) => panic!("{e}"),
            }
        }
        assert_eq!(entries.len(), 10_001);
        assert!(entries.iter().all(|e| e.metadata.is_none()));
        sort(&mut entries, Sort::Name, false);
        assert!(entries[0].is_dir());
        entries[1].load_metadata(&dir).unwrap();
        assert_eq!(entries[1].metadata.as_ref().unwrap().size, 4);
        assert_eq!(entries.iter().filter(|e| e.metadata.is_some()).count(), 1);
        assert!(fuzzy_score("file-00123.txt", "f123").is_some());
        assert!(fuzzy_score("abc", "z").is_none());
    }
}
