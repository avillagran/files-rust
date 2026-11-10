//! Cancellable folder-contents census for the Properties dialog. Directories
//! are measured by the existing `files-helper` traversal (never on the UI
//! thread, never following symlinks); this module only schedules the
//! selection, aggregates and keeps partial/failed states explicit.
use files_core::folder_usage::Census;
use files_core::protocol::{Request, Response};
use std::{
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::Duration,
};

/// Aggregate over the whole selection. `failed` objects were not measured at
/// all (helper error, limit, timeout); `census.unreadable` counts unreadable
/// entries inside measured folders. Either one makes every figure a lower bound.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    pub census: Census,
    pub failed: u64,
    pub first_error: Option<String>,
}
impl Totals {
    pub fn is_partial(&self) -> bool {
        self.failed > 0 || self.census.is_partial()
    }
}

type Measure = dyn Fn(&Path, &AtomicBool, &dyn Fn(u64)) -> io::Result<Census>;

pub struct Job {
    cancel: Arc<AtomicBool>,
    result: Receiver<Totals>,
    items: Arc<Mutex<u64>>,
}
impl Job {
    pub fn start(paths: Vec<PathBuf>) -> Result<Self, String> {
        let executable = std::env::current_exe()
            .map_err(|e| e.to_string())?
            .with_file_name("files-helper");
        Self::spawn(paths, move |path, cancel, items| {
            helper_census(path, cancel, &executable, items)
        })
    }
    fn spawn(
        paths: Vec<PathBuf>,
        measure: impl Fn(&Path, &AtomicBool, &dyn Fn(u64)) -> io::Result<Census> + Send + 'static,
    ) -> Result<Self, String> {
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let items = Arc::new(Mutex::new(0u64));
        let published = items.clone();
        let (tx, result) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("files-properties-census".into())
            .spawn(move || {
                let totals = run_all(&paths, &flag, &published, &measure);
                let _ = tx.send(totals);
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            cancel,
            result,
            items,
        })
    }
    /// Items counted so far (finished objects plus the running folder).
    pub fn items(&self) -> u64 {
        *self.items.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub fn poll(&self) -> Option<Totals> {
        self.result.try_recv().ok()
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

fn run_all(
    paths: &[PathBuf],
    cancel: &AtomicBool,
    published: &Mutex<u64>,
    measure: &Measure,
) -> Totals {
    let mut totals = Totals::default();
    let mut base = 0u64;
    for path in paths {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let start = base;
        let publish = move |running: u64| {
            *published.lock().unwrap_or_else(|e| e.into_inner()) = start.saturating_add(running);
        };
        // lstat: the object itself, a final symlink is one link and is not entered.
        let one = match std::fs::symlink_metadata(path) {
            Err(error) => {
                totals.census.unreadable = totals.census.unreadable.saturating_add(1);
                totals.first_error.get_or_insert_with(|| error.to_string());
                continue;
            }
            Ok(m) if m.is_dir() => match measure(path, cancel, &publish) {
                Ok(mut census) => {
                    // The selected folder itself is one item.
                    census.folders = census.folders.saturating_add(1);
                    census
                }
                Err(error) => {
                    totals.failed += 1;
                    totals.first_error.get_or_insert_with(|| error.to_string());
                    continue;
                }
            },
            Ok(m) => {
                let mut census = Census::default();
                if m.is_file() {
                    census.files = 1;
                    census.bytes = m.len();
                } else if m.file_type().is_symlink() {
                    census.links = 1;
                } else {
                    census.others = 1;
                }
                census
            }
        };
        totals.census.add(&one);
        base = base.saturating_add(one.items());
        publish(0);
    }
    totals
}

fn helper_census(
    path: &Path,
    cancel: &AtomicBool,
    executable: &Path,
    publish: &dyn Fn(u64),
) -> io::Result<Census> {
    let mut request = serde_json::to_vec(&Request::MeasureContents {
        id: 1,
        path: path.to_path_buf(),
    })?;
    request.push(b'\n');
    if request.len() > 64 * 1024 {
        return Err(io::Error::other("Ruta demasiado larga para calcular"));
    }
    let mut stream = Stream::default();
    crate::space_map_job::run_helper_stream(
        &request,
        cancel,
        executable.to_path_buf(),
        Duration::from_secs(30),
        "measure_contents",
        |data| {
            stream.feed(data)?;
            publish(stream.entries);
            Ok(())
        },
    )?;
    stream.finish()
}

#[derive(Default)]
struct Stream {
    line: Vec<u8>,
    entries: u64,
    result: Option<io::Result<Census>>,
}
impl Stream {
    fn feed(&mut self, data: &[u8]) -> io::Result<()> {
        for (i, part) in data.split(|&b| b == b'\n').enumerate() {
            if i > 0 {
                self.decode()?;
                self.line.clear();
            }
            if self.line.len() + part.len() > 4096 {
                return Err(io::Error::other("Línea de progreso superior a4096B"));
            }
            self.line.extend_from_slice(part);
        }
        Ok(())
    }
    fn decode(&mut self) -> io::Result<()> {
        if self.result.is_some() {
            return Err(io::Error::other("Datos después del resultado final"));
        }
        match serde_json::from_slice::<Response>(&self.line)? {
            Response::UsageProgress { id: 1, entries, .. } => {
                if entries < self.entries {
                    return Err(io::Error::other("Progreso de conteo inválido"));
                }
                self.entries = entries;
            }
            Response::Contents { id: 1, census } => self.result = Some(Ok(census)),
            Response::Error { id: 1, mut message } => {
                let mut end = message.len().min(512);
                while !message.is_char_boundary(end) {
                    end -= 1;
                }
                message.truncate(end);
                self.result = Some(Err(io::Error::other(message)));
            }
            _ => return Err(io::Error::other("Mensaje de conteo inesperado")),
        }
        Ok(())
    }
    fn finish(self) -> io::Result<Census> {
        if !self.line.is_empty() {
            return Err(io::Error::other("Respuesta de conteo truncada"));
        }
        self.result
            .unwrap_or_else(|| Err(io::Error::other("El helper terminó sin resumen completo")))
    }
}

/// Spanish one-line summary; "calculando" while running, lower bounds flagged.
pub fn summary(totals: Option<&Totals>, running_items: u64) -> String {
    let Some(t) = totals else {
        return format!("calculando… {running_items} elementos");
    };
    let c = &t.census;
    let mut text = format!(
        "{} · {} archivos, {} carpetas{}",
        files_core::format_size(c.bytes),
        c.files,
        c.folders,
        if c.links > 0 {
            format!(", {} enlaces (sin seguir)", c.links)
        } else {
            String::new()
        }
    );
    if t.is_partial() {
        text.push_str(" · PARCIAL");
        if c.unreadable > 0 {
            text.push_str(&format!(": {} sin permiso de lectura", c.unreadable));
        }
        if t.failed > 0 {
            text.push_str(&format!(": {} objeto(s) sin calcular", t.failed));
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn fake(
        calls: Arc<AtomicUsize>,
    ) -> impl Fn(&Path, &AtomicBool, &dyn Fn(u64)) -> io::Result<Census> + Send + 'static {
        move |path, _, publish| {
            calls.fetch_add(1, Ordering::SeqCst);
            publish(7);
            if path.ends_with("bad") {
                return Err(io::Error::other("Límite de entradas excedido"));
            }
            Ok(Census {
                bytes: 100,
                files: 3,
                folders: 1,
                links: 1,
                others: 0,
                unreadable: u64::from(path.ends_with("locked")),
            })
        }
    }
    struct Dir(PathBuf);
    impl Dir {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "files-census-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    #[test]
    fn selection_sums_files_links_and_folders_and_marks_partial_honestly() {
        let d = Dir::new();
        for name in ["a", "locked", "bad"] {
            std::fs::create_dir(d.0.join(name)).unwrap();
        }
        std::fs::write(d.0.join("f.txt"), b"12345").unwrap();
        // A link to a directory inside the private fixture: one link, not entered.
        std::os::unix::fs::symlink(d.0.join("a"), d.0.join("ln")).unwrap();
        let paths: Vec<_> = ["a", "locked", "bad", "f.txt", "ln", "missing"]
            .iter()
            .map(|n| d.0.join(n))
            .collect();
        let calls = Arc::new(AtomicUsize::new(0));
        let published = Mutex::new(0);
        let t = run_all(
            &paths,
            &AtomicBool::new(false),
            &published,
            &fake(calls.clone()),
        );
        // Only the three real directories reach the helper; the link does not.
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert_eq!(t.failed, 1);
        // a + locked: 2 * (100 B, 3 files, 1+1 folders, 1 link) + file + link.
        assert_eq!(t.census.bytes, 205);
        assert_eq!(t.census.files, 7);
        assert_eq!(t.census.folders, 4);
        assert_eq!(t.census.links, 3);
        // locked subtree + the missing object.
        assert_eq!(t.census.unreadable, 2);
        assert!(t.is_partial());
        let text = summary(Some(&t), 0);
        assert!(text.contains("PARCIAL") && text.contains("sin permiso de lectura"));
        assert!(text.contains("sin calcular"));
        assert_eq!(summary(None, 12), "calculando… 12 elementos");
    }

    #[test]
    fn cancelled_job_stops_before_touching_further_objects() {
        let d = Dir::new();
        let paths = vec![d.0.clone(), d.0.clone()];
        let calls = Arc::new(AtomicUsize::new(0));
        let cancel = AtomicBool::new(true);
        let t = run_all(&paths, &cancel, &Mutex::new(0), &fake(calls.clone()));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(t, Totals::default());
    }

    #[test]
    fn stream_decodes_progress_then_contents_and_rejects_bad_input() {
        let census = Census {
            bytes: 9,
            files: 2,
            folders: 1,
            links: 1,
            others: 0,
            unreadable: 1,
        };
        let mut s = Stream::default();
        let mut bytes = serde_json::to_vec(&Response::UsageProgress {
            id: 1,
            bytes: 3,
            entries: 4,
        })
        .unwrap();
        bytes.push(b'\n');
        s.feed(&bytes[..7]).unwrap();
        s.feed(&bytes[7..]).unwrap();
        assert_eq!(s.entries, 4);
        let mut last = serde_json::to_vec(&Response::Contents { id: 1, census }).unwrap();
        last.push(b'\n');
        s.feed(&last).unwrap();
        assert!(s.feed(&last).is_err(), "no data after the final result");
        let mut s = Stream::default();
        s.feed(&bytes).unwrap();
        assert!(s.finish().is_err(), "progress alone is never a result");
        let mut s = Stream::default();
        s.feed(&last).unwrap();
        assert_eq!(s.finish().unwrap(), census);
        let mut s = Stream::default();
        assert!(s.feed(&vec![b'x'; 4097]).is_err());
    }

    #[test]
    fn dropping_the_job_cancels_the_worker() {
        let started = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(AtomicBool::new(false));
        let (go, flag) = (started.clone(), seen.clone());
        let d = Dir::new();
        let job = Job::spawn(vec![d.0.clone()], move |_, cancel, _| {
            go.store(true, Ordering::SeqCst);
            for _ in 0..500 {
                if cancel.load(Ordering::Relaxed) {
                    flag.store(true, Ordering::SeqCst);
                    return Err(io::Error::other("cancelado"));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(Census::default())
        })
        .unwrap();
        let start = std::time::Instant::now();
        while !started.load(Ordering::SeqCst) && start.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(started.load(Ordering::SeqCst));
        drop(job);
        while !seen.load(Ordering::SeqCst) && start.elapsed() < Duration::from_secs(4) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(seen.load(Ordering::SeqCst));
    }
}
