//! Reusable single-flight probe/decode worker. No thread per visible file.
//! A readonly filesystem probe can be uninterruptible; close may abandon that
//! probe only after the gate forbids all future helper spawns. A helper phase
//! always requires its final response after kill/reap before normal exit.
use crate::image_preview_job::Bitmap;
use files_core::media_source::Revision;
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
};
pub enum Value {
    Unchanged(Revision),
    Bitmap(Bitmap),
    Cached(Bitmap),
    Unavailable(Revision, String),
}
pub struct Reply {
    pub id: u64,
    pub result: Result<Value, String>,
}
struct Request {
    id: u64,
    path: PathBuf,
    previous: Option<Revision>,
    cancel: Arc<AtomicBool>,
}
enum Command {
    Request(Request),
    Stop,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    ReadOnly,
    Cache,
    Helper,
    Stopped,
}
struct Gate {
    phase: Phase,
    closing: bool,
}
#[derive(Default)]
struct Stats {
    requests: AtomicU64,
    probes: AtomicU64,
    decodes: AtomicU64,
    disk_hits: AtomicU64,
    missing: AtomicU64,
    cache_errors: AtomicU64,
    warning: Mutex<Option<String>>,
}
pub struct Worker {
    stats: Arc<Stats>,
    send: SyncSender<Command>,
    result: Receiver<Reply>,
    gate: Arc<Mutex<Gate>>,
    cancel: Option<Arc<AtomicBool>>,
    busy: bool,
}
impl Worker {
    pub fn new() -> Self {
        let (send, rx) = mpsc::sync_channel(1);
        let (tx, result) = mpsc::sync_channel(1);
        let gate = Arc::new(Mutex::new(Gate {
            phase: Phase::Idle,
            closing: false,
        }));
        let state = gate.clone();
        let stats = Arc::new(Stats::default());
        let counts = stats.clone();
        std::thread::Builder::new()
            .name("files-thumbnails".into())
            .spawn(move || {
                while let Ok(command) = rx.recv() {
                    let Command::Request(request) = command else {
                        break;
                    };
                    state.lock().unwrap().phase = Phase::ReadOnly;
                    let result = (|| -> std::io::Result<Value> {
                        if request.cancel.load(Ordering::Acquire) {
                            return Err(std::io::Error::other("Miniatura cancelada"));
                        }
                        counts.probes.fetch_add(1, Ordering::Relaxed);
                        let (source, revision) = match files_core::media_source::open(&request.path)
                        {
                            Ok(v) => v,
                            Err(error) => {
                                if error.kind() == std::io::ErrorKind::NotFound {
                                    counts.missing.fetch_add(1, Ordering::Relaxed);
                                }
                                // The source name is no longer usable: retire its owned cache
                                // slot without traversing or modifying the original directory.
                                let _ = crate::thumbnail_disk_cache::invalidate(
                                    &request.path,
                                    &request.cancel,
                                );
                                return Err(error);
                            }
                        };
                        if request.cancel.load(Ordering::Acquire) {
                            return Err(std::io::Error::other("Miniatura cancelada"));
                        }
                        if request.previous == Some(revision) {
                            return Ok(Value::Unchanged(revision));
                        }
                        match crate::thumbnail_disk_cache::load(&request.path, revision) {
                            Ok(Some(bitmap)) => {
                                source_revision_current(
                                    &source,
                                    revision,
                                    &request.path,
                                    &request.cancel,
                                )?;
                                counts.disk_hits.fetch_add(1, Ordering::Relaxed);
                                return Ok(Value::Cached(bitmap));
                            }
                            Ok(None) => (),
                            Err(error) => cache_warning(&counts, error),
                        }
                        source_revision_current(&source, revision, &request.path, &request.cancel)?;
                        {
                            let mut gate = state.lock().unwrap();
                            if gate.closing || request.cancel.load(Ordering::Acquire) {
                                return Err(std::io::Error::other("Miniatura cancelada"));
                            }
                            // Mark Helper BEFORE releasing gate. Close cannot report a
                            // readonly phase and then race a subsequent child spawn.
                            gate.phase = Phase::Helper;
                        }
                        counts.decodes.fetch_add(1, Ordering::Relaxed);
                        let bitmap = if is_video(&request.path) {
                            crate::video_preview_job::thumbnail(
                                request.path.clone(),
                                revision,
                                &request.cancel,
                            )
                        } else {
                            crate::image_preview_job::thumbnail(
                                request.path.clone(),
                                revision,
                                &request.cancel,
                            )
                        };
                        if request.cancel.load(Ordering::Acquire) {
                            return Err(std::io::Error::other("Miniatura cancelada"));
                        }
                        Ok(match bitmap {
                            Ok(bitmap) => {
                                if bitmap.source != Some(revision) {
                                    return Err(std::io::Error::other(
                                        "La miniatura no coincide con su origen verificado",
                                    ));
                                }
                                source_revision_current(
                                    &source,
                                    revision,
                                    &request.path,
                                    &request.cancel,
                                )?;
                                // Decoder has reaped its helper. Cache IO may be abandoned on
                                // close; no helper can spawn after entering this phase.
                                state.lock().unwrap().phase = Phase::Cache;
                                if let Err(error) = crate::thumbnail_disk_cache::save(
                                    &request.path,
                                    &bitmap,
                                    &request.cancel,
                                ) {
                                    cache_warning(&counts, error);
                                }
                                source_revision_current(
                                    &source,
                                    revision,
                                    &request.path,
                                    &request.cancel,
                                )?;
                                Value::Bitmap(bitmap)
                            }
                            Err(error) => Value::Unavailable(revision, error.to_string()),
                        })
                    })()
                    .map_err(|e| e.to_string());
                    state.lock().unwrap().phase = Phase::Idle;
                    if tx
                        .send(Reply {
                            id: request.id,
                            result,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                state.lock().unwrap().phase = Phase::Stopped;
            })
            .expect("thumbnail worker");
        Self {
            stats,
            send,
            result,
            gate,
            cancel: None,
            busy: false,
        }
    }
    pub fn request(&mut self, id: u64, path: PathBuf, previous: Option<Revision>) -> bool {
        if self.busy || self.gate.lock().unwrap().closing {
            return false;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        if self
            .send
            .try_send(Command::Request(Request {
                id,
                path,
                previous,
                cancel: cancel.clone(),
            }))
            .is_err()
        {
            return false;
        }
        self.stats.requests.fetch_add(1, Ordering::Relaxed);
        self.cancel = Some(cancel);
        self.busy = true;
        true
    }
    pub fn cancel(&self) {
        if let Some(cancel) = &self.cancel {
            cancel.store(true, Ordering::Release);
        }
    }
    pub fn poll(&mut self) -> Option<Reply> {
        if !self.busy {
            return None;
        }
        let reply = match self.result.try_recv() {
            Ok(reply) => reply,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => Reply {
                id: 0,
                result: Err("El worker de miniaturas terminó sin ACK".into()),
            },
        };
        self.busy = false;
        self.cancel = None;
        Some(reply)
    }
    pub fn diagnostics(&self) -> serde_json::Value {
        let s = &self.stats;
        serde_json::json!({"requests":s.requests.load(Ordering::Relaxed),"probes":s.probes.load(Ordering::Relaxed),
            "decodes":s.decodes.load(Ordering::Relaxed),"disk_hits":s.disk_hits.load(Ordering::Relaxed),
            "missing":s.missing.load(Ordering::Relaxed),"cache_errors":s.cache_errors.load(Ordering::Relaxed)})
    }
    pub fn cache_warning(&self) -> Option<String> {
        self.stats.warning.lock().unwrap().clone()
    }
    pub fn busy(&self) -> bool {
        self.busy
    }
    pub fn resume(&self) {
        self.gate.lock().unwrap().closing = false;
    }
    pub fn begin_close(&self) {
        let mut gate = self.gate.lock().unwrap();
        gate.closing = true;
        self.cancel();
    }
    /// True is permission to exit, not an ACK of a stuck readonly syscall.
    pub fn close_allowed(&self) -> bool {
        let gate = self.gate.lock().unwrap();
        gate.closing
            && (!self.busy
                || matches!(
                    gate.phase,
                    Phase::ReadOnly | Phase::Cache | Phase::Idle | Phase::Stopped
                ))
    }
    pub fn readonly_pending(&self) -> bool {
        self.busy && self.gate.lock().unwrap().phase == Phase::ReadOnly
    }
}
fn source_revision_current(
    source: &std::fs::File,
    revision: Revision,
    path: &std::path::Path,
    cancel: &AtomicBool,
) -> std::io::Result<()> {
    if cancel.load(Ordering::Acquire) {
        return Err(std::io::Error::other("Miniatura cancelada"));
    }
    revision.revalidate(source, path)
}
fn cache_warning(stats: &Stats, error: std::io::Error) {
    stats.cache_errors.fetch_add(1, Ordering::Relaxed);
    let mut text = error.to_string();
    let mut end = text.len().min(512);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    *stats.warning.lock().unwrap() = Some(format!("Caché de miniaturas: {text}"));
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.begin_close();
        let _ = self.send.try_send(Command::Stop);
    }
}
pub fn supported(path: &std::path::Path) -> bool {
    crate::text_preview_state::thumbnail_source(path)
}
pub fn is_video(path: &std::path::Path) -> bool {
    supported(path)
        && path.extension().and_then(|s| s.to_str()).is_some_and(|s| {
            !matches!(
                s.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "webp"
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gif_uses_existing_video_thumbnail_and_source_guard_path() {
        let p = std::path::Path::new("/fixture/anim.GIF");
        assert!(supported(p));
        assert!(is_video(p));
        assert!(crate::text_preview_state::source_required(p));
    }
    #[cfg(unix)]
    #[test]
    fn single_request_stays_busy_until_ack_and_reuses_same_worker_for_probe() {
        let dir = crate::thumbnail_disk_cache::retained_test_directory("thumbnail-probe");
        let path = dir.join("raw.png");
        std::fs::write(&path, b"file").unwrap();
        let (_, revision) = files_core::media_source::open(&path).unwrap();
        let mut worker = Worker::new();
        for id in 1..=2 {
            assert!(worker.request(id, path.clone(), Some(revision)));
            assert!(!worker.request(id + 1, path.clone(), Some(revision)));
            let end = std::time::Instant::now() + std::time::Duration::from_secs(3);
            loop {
                if let Some(reply) = worker.poll() {
                    assert_eq!(reply.id, id);
                    assert!(matches!(reply.result,Ok(Value::Unchanged(r))if r==revision));
                    break;
                }
                assert!(std::time::Instant::now() < end);
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            assert!(!worker.busy());
        }
        worker.begin_close();
        assert!(worker.close_allowed());
        assert!(dir.join("raw.png").is_file(), "Retain all fixture data");
    }
    #[test]
    fn close_distinguishes_readonly_from_helper_phase_without_join() {
        let (send, _rx) = mpsc::sync_channel(1);
        let (_tx, result) = mpsc::sync_channel(1);
        let gate = Arc::new(Mutex::new(Gate {
            phase: Phase::ReadOnly,
            closing: false,
        }));
        let worker = Worker {
            stats: Arc::new(Stats::default()),
            send,
            result,
            gate: gate.clone(),
            cancel: Some(Arc::new(AtomicBool::new(false))),
            busy: true,
        };
        assert!(!worker.close_allowed());
        worker.begin_close();
        assert!(worker.close_allowed());
        assert!(worker.readonly_pending());
        assert!(worker.cancel.as_ref().unwrap().load(Ordering::Acquire));
        gate.lock().unwrap().phase = Phase::Helper;
        assert!(!worker.close_allowed());
        gate.lock().unwrap().phase = Phase::Cache;
        assert!(worker.close_allowed());
        gate.lock().unwrap().phase = Phase::Idle;
        assert!(worker.close_allowed());
    }
}
