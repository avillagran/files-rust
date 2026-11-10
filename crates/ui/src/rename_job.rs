//! One bounded mutation worker. Filesystem probes and planning never run on UI.
use files_core::{
    rename_plan::{self, RenameOptions},
    rename_transaction::{PreparedRename, TransactionError, UndoToken},
};
use std::{
    ffi::OsString,
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
};
const MAX_NAMES: usize = 100_000;
const MAX_INPUT_BYTES: usize = 8 * 1024 * 1024;

// Private acceptance instrumentation only. An armed pre-execution gate exits
// on cancellation; an armed computed-ACK gate retains the actual response
// through cancellation until release or its real ten-second deadline.
#[cfg(target_os = "linux")]
mod fixture_gate {
    use std::{
        ffi::{CString, OsStr},
        fs::File,
        io::Write,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::{ffi::OsStrExt, fs::MetadataExt},
        },
        path::{Component, Path},
        sync::atomic::{AtomicBool, Ordering},
        time::{Duration, Instant},
    };

    const DEADLINE: Duration = Duration::from_secs(10);
    const POLL: Duration = Duration::from_millis(10);

    pub(super) struct Gate {
        directory: File,
    }
    #[derive(Debug, PartialEq, Eq)]
    enum Exit {
        Unarmed,
        Released,
        Cancelled,
        TimedOut,
    }

    impl Gate {
        pub(super) fn from_environment() -> Option<Self> {
            let enabled = std::env::var_os("FILES_TEST_MUTATIONS");
            let root = std::env::var_os("TMPDIR");
            let requested = std::env::var_os("FILES_TEST_MUTATION_GATE_DIR");
            Self::configured(enabled.as_deref(), root.as_deref(), requested.as_deref())
        }

        fn configured(
            enabled: Option<&OsStr>,
            root: Option<&OsStr>,
            requested: Option<&OsStr>,
        ) -> Option<Self> {
            if enabled != Some(OsStr::new("1")) {
                return None;
            }
            // Never accept the default shared temp directory or live settings.
            let root = Path::new(root?);
            let requested = Path::new(requested?);
            let clean = |path: &Path| {
                path.is_absolute()
                    && path
                        .components()
                        .all(|part| matches!(part, Component::RootDir | Component::Normal(_)))
            };
            if !clean(root) || !clean(requested) {
                return None;
            }
            let relative = requested.strip_prefix(root).ok()?;
            if relative.as_os_str().is_empty() {
                return None;
            }
            let mut directory = File::open("/").ok()?;
            for part in root.components() {
                if let Component::Normal(name) = part {
                    directory = open_child(&directory, name)?;
                }
            }
            if !private_directory(&directory) {
                return None;
            }
            for part in relative.components() {
                let Component::Normal(name) = part else {
                    return None;
                };
                directory = open_child(&directory, name)?;
                if !private_directory(&directory) {
                    return None;
                }
            }
            Some(Self { directory })
        }

        pub(super) fn wait(&self, phase: &str, id: u64, cancel: &AtomicBool, ack: bool) {
            let phase = if ack {
                format!("ack-{phase}")
            } else {
                phase.to_owned()
            };
            let _ = self.wait_until(&phase, id, cancel, ack, DEADLINE);
        }

        fn wait_until(
            &self,
            phase: &str,
            id: u64,
            cancel: &AtomicBool,
            ack: bool,
            timeout: Duration,
        ) -> Exit {
            if !self.signal_exists(&format!("{phase}.arm")) {
                return Exit::Unarmed;
            }
            let prefix = format!("{phase}-{id}");
            if !self.signal(&format!("{prefix}.waiting"), "waiting\n") {
                return Exit::Unarmed;
            }
            let started = Instant::now();
            let mut observed_cancel = false;
            let exit = loop {
                if cancel.load(Ordering::Acquire) {
                    if !observed_cancel {
                        let _ = self.signal(&format!("{prefix}.cancelled"), "cancel-observed\n");
                        observed_cancel = true;
                    }
                    if !ack {
                        break Exit::Cancelled;
                    }
                    // The result already exists. Holding its ACK can exercise
                    // stale delivery after close/reopen; this is not rollback.
                }
                if self.signal_exists(&format!("{prefix}.release")) {
                    break Exit::Released;
                }
                let remaining = timeout.saturating_sub(started.elapsed());
                if remaining.is_zero() {
                    break Exit::TimedOut;
                }
                std::thread::sleep(remaining.min(POLL));
            };
            let text = match exit {
                Exit::Released => "released\n",
                Exit::Cancelled => "cancelled-before-execution\n",
                Exit::TimedOut => "timed-out\n",
                Exit::Unarmed => unreachable!(),
            };
            let _ = self.signal(&format!("{prefix}.finished"), text);
            exit
        }

        fn signal_exists(&self, name: &str) -> bool {
            let Ok(name) = CString::new(name) else {
                return false;
            };
            let mut info = std::mem::MaybeUninit::<libc::stat>::uninit();
            // SAFETY: inspect one internal phase/id name relative to our
            // retained directory descriptor, without following symlinks.
            if unsafe {
                libc::fstatat(
                    self.directory.as_raw_fd(),
                    name.as_ptr(),
                    info.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            } != 0
            {
                return false;
            }
            // SAFETY: successful fstatat initialized the entire stat.
            let info = unsafe { info.assume_init() };
            info.st_mode & libc::S_IFMT == libc::S_IFREG
                && info.st_mode & 0o077 == 0
                && info.st_uid == unsafe { libc::geteuid() }
        }

        fn signal(&self, name: &str, text: &str) -> bool {
            let Ok(name) = CString::new(name) else {
                return false;
            };
            // SAFETY: create a new internal phase/id name. O_EXCL and
            // O_NOFOLLOW prevent replacing existing entries or linked data.
            let fd = unsafe {
                libc::openat(
                    self.directory.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_WRONLY
                        | libc::O_CREAT
                        | libc::O_EXCL
                        | libc::O_NOFOLLOW
                        | libc::O_CLOEXEC
                        | libc::O_NONBLOCK,
                    0o600,
                )
            };
            if fd < 0 {
                return false;
            }
            // SAFETY: exactly one File takes ownership of the new descriptor.
            let mut file = unsafe { File::from_raw_fd(fd) };
            file.write_all(text.as_bytes()).is_ok()
        }
    }

    fn private_directory(file: &File) -> bool {
        file.metadata().is_ok_and(|metadata| {
            metadata.is_dir()
                && metadata.mode() & 0o777 == 0o700
                && metadata.uid() == unsafe { libc::geteuid() }
        })
    }

    fn open_child(parent: &File, name: &OsStr) -> Option<File> {
        let name = CString::new(name.as_bytes()).ok()?;
        // SAFETY: open one component while its parent remains owned.
        // O_DIRECTORY and O_NOFOLLOW reject linked or non-directory paths.
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        (fd >= 0).then(|| unsafe { File::from_raw_fd(fd) })
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::{
            fs,
            os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt, symlink},
            path::PathBuf,
            sync::atomic::AtomicU64,
        };

        struct Fixture {
            root: PathBuf,
            gate: PathBuf,
        }
        impl Fixture {
            fn new() -> Self {
                static NEXT: AtomicU64 = AtomicU64::new(0);
                let root = std::env::temp_dir().join(format!(
                    "files-mutation-gate-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
                let gate = root.join("gate");
                fs::DirBuilder::new().mode(0o700).create(&gate).unwrap();
                // Retain these owned fixtures; never remove filesystem trees.
                Self { root, gate }
            }
            fn configured(&self) -> Gate {
                Gate::configured(
                    Some(OsStr::new("1")),
                    Some(self.root.as_os_str()),
                    Some(self.gate.as_os_str()),
                )
                .unwrap()
            }
            fn signal(&self, name: &str) {
                fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(self.gate.join(name))
                    .unwrap();
            }
        }

        #[test]
        fn private_gate_requires_flag_private_root_and_strict_descendant() {
            let f = Fixture::new();
            for flag in [None, Some(OsStr::new("0"))] {
                assert!(
                    Gate::configured(flag, Some(f.root.as_os_str()), Some(f.gate.as_os_str()))
                        .is_none()
                );
            }
            assert!(
                Gate::configured(Some(OsStr::new("1")), None, Some(f.gate.as_os_str())).is_none()
            );
            assert!(
                Gate::configured(
                    Some(OsStr::new("1")),
                    Some(f.gate.as_os_str()),
                    Some(f.root.as_os_str())
                )
                .is_none()
            );
            assert!(
                Gate::configured(
                    Some(OsStr::new("1")),
                    Some(f.root.as_os_str()),
                    Some(f.root.as_os_str())
                )
                .is_none()
            );
            fs::set_permissions(&f.root, fs::Permissions::from_mode(0o755)).unwrap();
            assert!(
                Gate::configured(
                    Some(OsStr::new("1")),
                    Some(f.root.as_os_str()),
                    Some(f.gate.as_os_str())
                )
                .is_none()
            );
        }

        #[test]
        fn private_gate_rejects_directory_and_release_symlinks() {
            let f = Fixture::new();
            let alias = f.root.join("alias");
            symlink(&f.gate, &alias).unwrap();
            assert!(
                Gate::configured(
                    Some(OsStr::new("1")),
                    Some(f.root.as_os_str()),
                    Some(alias.as_os_str())
                )
                .is_none()
            );
            f.signal("plan.arm");
            symlink(f.gate.join("plan.arm"), f.gate.join("plan-1.release")).unwrap();
            assert_eq!(
                f.configured().wait_until(
                    "plan",
                    1,
                    &AtomicBool::new(false),
                    false,
                    Duration::from_millis(20)
                ),
                Exit::TimedOut
            );
            assert_eq!(fs::read(f.gate.join("plan.arm")).unwrap(), b"");
        }

        #[test]
        fn private_gate_unarmed_release_and_cancel_are_distinct() {
            let f = Fixture::new();
            let gate = f.configured();
            assert_eq!(
                gate.wait_until("plan", 1, &AtomicBool::new(false), false, Duration::ZERO),
                Exit::Unarmed
            );
            assert!(!f.gate.join("plan-1.waiting").exists());
            f.signal("plan.arm");
            f.signal("plan-2.release");
            assert_eq!(
                gate.wait_until("plan", 2, &AtomicBool::new(false), false, Duration::ZERO),
                Exit::Released
            );
            assert_eq!(
                gate.wait_until(
                    "plan",
                    3,
                    &AtomicBool::new(true),
                    false,
                    Duration::from_secs(1)
                ),
                Exit::Cancelled
            );
            assert!(f.gate.join("plan-3.cancelled").is_file());
        }

        #[test]
        fn private_gate_computed_ack_observes_cancel_and_has_a_deadline() {
            let f = Fixture::new();
            f.signal("ack-plan.arm");
            assert_eq!(
                f.configured().wait_until(
                    "ack-plan",
                    4,
                    &AtomicBool::new(true),
                    true,
                    Duration::from_millis(20)
                ),
                Exit::TimedOut
            );
            assert!(f.gate.join("ack-plan-4.cancelled").is_file());
            assert_eq!(
                fs::read_to_string(f.gate.join("ack-plan-4.finished")).unwrap(),
                "timed-out\n"
            );
        }

        #[test]
        fn private_gate_cancelled_computed_create_ack_preserves_actual_success() {
            let f = Fixture::new();
            f.signal("ack-create.arm");
            let mut worker = super::super::Worker::new();
            worker.start_with_gate(Some(f.configured()));
            assert!(
                worker
                    .submit(super::super::Request::Create {
                        id: 9,
                        directory: f.root.clone(),
                        name: "created.txt".into(),
                        folder: false,
                    })
                    .is_ok()
            );
            let deadline = Instant::now() + Duration::from_secs(5);
            while !f.gate.join("ack-create-9.waiting").exists() {
                assert!(Instant::now() < deadline);
                std::thread::sleep(POLL);
            }
            assert!(f.root.join("created.txt").is_file());
            worker.cancel();
            while !f.gate.join("ack-create-9.cancelled").exists() {
                assert!(Instant::now() < deadline);
                std::thread::sleep(POLL);
            }
            assert!(worker.busy && worker.poll().is_none());
            f.signal("ack-create-9.release");
            let response = loop {
                if let Some(response) = worker.poll() {
                    break response;
                }
                assert!(Instant::now() < deadline);
                std::thread::sleep(POLL);
            };
            let super::super::Response::Finished(id, outcome, _) = response else {
                panic!("expected actual create result")
            };
            assert_eq!(id, 9);
            assert!(outcome.success && outcome.details.is_empty());
            assert_eq!(fs::read(f.root.join("created.txt")).unwrap(), b"");
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod fixture_gate {
    pub(super) struct Gate;
    impl Gate {
        pub(super) fn from_environment() -> Option<Self> {
            None
        }
        pub(super) fn wait(&self, _: &str, _: u64, _: &std::sync::atomic::AtomicBool, _: bool) {}
    }
}

pub enum Request {
    Inspect {
        id: u64,
        path: PathBuf,
    },
    Permissions {
        id: u64,
        snapshot: Box<files_core::file_properties::Snapshot>,
        mode: u32,
    },
    Plan {
        id: u64,
        directory: PathBuf,
        selected: Arc<[OsString]>,
        options: RenameOptions,
    },
    Apply {
        id: u64,
        prepared: PreparedRename,
    },
    History {
        id: u64,
        redo: bool,
    },
    Trash {
        id: u64,
        directory: PathBuf,
        names: Arc<[OsString]>,
        root: PathBuf,
    },
    Create {
        id: u64,
        directory: PathBuf,
        name: String,
        folder: bool,
    },
    /// History bookkeeping from other workers (copy/move, links, templates).
    /// No filesystem access; answered with `Response::Recorded`.
    Record {
        id: u64,
        commands: Vec<crate::undo_history::Command>,
    },
}
impl Request {
    fn fixture_phase(&self) -> Option<&'static str> {
        match self {
            Self::Plan { .. } => Some("plan"),
            Self::Apply { .. } => Some("apply"),
            Self::Create { .. } => Some("create"),
            _ => None,
        }
    }
    fn id(&self) -> u64 {
        match self {
            Self::Inspect { id, .. }
            | Self::Permissions { id, .. }
            | Self::Plan { id, .. }
            | Self::Apply { id, .. }
            | Self::History { id, .. }
            | Self::Create { id, .. }
            | Self::Record { id, .. }
            | Self::Trash { id, .. } => *id,
        }
    }
}
pub struct Outcome {
    pub success: bool,
    pub message: String,
    pub details: Vec<String>,
    pub undo: bool,
    pub redo: bool,
}
pub enum Response {
    Properties(u64, Box<crate::properties_job::Reply>),
    Planned(u64, Result<PreparedRename, String>),
    Finished(u64, Outcome, Option<PathBuf>),
    /// A `Request::Record` was applied; only the history labels changed.
    Recorded,
}
type Labels = Arc<std::sync::Mutex<(Option<String>, Option<String>)>>;
pub struct Worker {
    tx: Option<SyncSender<Request>>,
    rx: Option<Receiver<Response>>,
    cancel: Arc<AtomicBool>,
    pub busy: bool,
    /// Labels of the history's top Undo/Redo entries, published by the worker.
    labels: Labels,
    /// Commands waiting for an idle worker, flushed by `flush_history`.
    queue: Vec<crate::undo_history::Command>,
    /// Bookkeeping requests sent but not yet acknowledged. They never make
    /// the worker `busy`, so they do not gate close, trash or rename.
    records_in_flight: usize,
}
impl Worker {
    pub fn new() -> Self {
        Self {
            tx: None,
            rx: None,
            cancel: Arc::new(AtomicBool::new(false)),
            busy: false,
            labels: Arc::default(),
            queue: Vec::new(),
            records_in_flight: 0,
        }
    }
    /// "Deshacer copiar 3 elementos" / "Rehacer …" for menus and the palette.
    pub fn labels(&self) -> (Option<String>, Option<String>) {
        self.labels
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    /// Queue a history command; it reaches the worker when it is idle.
    pub fn enqueue(&mut self, command: crate::undo_history::Command) {
        use crate::undo_history::Command;
        if matches!(command, Command::InvalidateTrash | Command::InvalidateAll)
            && self
                .queue
                .last()
                .is_some_and(|c| std::mem::discriminant(c) == std::mem::discriminant(&command))
        {
            return;
        }
        // Bounded like the history itself: the oldest pending record is
        // dropped (it simply becomes not undoable); invalidations are kept.
        if self.queue.len() >= crate::undo_history::MAX_ENTRIES
            && let Some(i) = self
                .queue
                .iter()
                .position(|c| matches!(c, Command::Record(_)))
        {
            self.queue.remove(i);
        }
        self.queue.push(command);
    }
    /// True while recorded operations have not reached the history yet.
    pub fn history_pending(&self) -> bool {
        !self.queue.is_empty() || self.records_in_flight > 0
    }
    /// Send queued commands when idle, without marking the worker busy and
    /// without touching the cancellation flag. Returns true when sent.
    pub fn flush_history(&mut self, id: u64) -> bool {
        if self.busy || self.queue.is_empty() || self.records_in_flight > 0 {
            return false;
        }
        if self.tx.is_none() {
            self.start();
        }
        let commands = std::mem::take(&mut self.queue);
        match self
            .tx
            .as_ref()
            .expect("worker started")
            .try_send(Request::Record { id, commands })
        {
            Ok(()) => {
                self.records_in_flight += 1;
                true
            }
            Err(
                mpsc::TrySendError::Full(Request::Record { commands, .. })
                | mpsc::TrySendError::Disconnected(Request::Record { commands, .. }),
            ) => {
                self.queue = commands;
                false
            }
            Err(_) => false,
        }
    }
    fn start(&mut self) {
        self.start_with_gate(fixture_gate::Gate::from_environment());
    }
    fn start_with_gate(&mut self, gate: Option<fixture_gate::Gate>) {
        let (tx, input) = mpsc::sync_channel::<Request>(1);
        let (output, rx) = mpsc::sync_channel(1);
        let flag = self.cancel.clone();
        let labels = Arc::clone(&self.labels);
        std::thread::spawn(move || {
            // One linear, bounded history (undo_history.rs), including the
            // descriptors pinned by rename tokens and trash receipts.
            let mut history = crate::undo_history::History::default();
            while let Ok(request) = input.recv() {
                let id = request.id();
                let fixture_phase = request.fixture_phase();
                if let (Some(gate), Some(phase)) = (&gate, fixture_phase) {
                    gate.wait(phase, id, &flag, false);
                }
                let refresh_directory = match &request {
                    Request::Apply { prepared, .. } => Some(prepared.directory().to_path_buf()),
                    Request::Create { directory, .. } | Request::Trash { directory, .. } => {
                        Some(directory.clone())
                    }
                    Request::History { redo, .. } => history.directory(*redo),
                    _ => None,
                };
                let response = match request {
                    Request::Inspect { path, .. } => Response::Properties(
                        id,
                        Box::new(crate::properties_job::inspect(path, &flag)),
                    ),
                    Request::Permissions { snapshot, mode, .. } => {
                        let reply = crate::properties_job::apply(snapshot, mode, &flag);
                        if reply.changed {
                            // Not undoable: keep the history linear.
                            history.clear();
                        }
                        Response::Properties(id, Box::new(reply))
                    }
                    Request::Plan {
                        directory,
                        selected,
                        options,
                        ..
                    } => Response::Planned(id, plan(directory, &selected, &options, &flag)),
                    Request::Apply { prepared, .. } => {
                        let count = prepared.len();
                        match prepared.execute_cancellable(&flag) {
                            Ok(token) => {
                                let details = cleanup_details(&token);
                                history.push_rename(
                                    token,
                                    refresh_directory.clone().unwrap_or_default(),
                                    count,
                                );
                                Response::Finished(
                                    id,
                                    Outcome {
                                        success: true,
                                        message: "Nombres actualizados".into(),
                                        details,
                                        undo: true,
                                        redo: false,
                                    },
                                    refresh_directory,
                                )
                            }
                            Err(error) => Response::Finished(id, failed(error), refresh_directory),
                        }
                    }
                    Request::History { redo, .. } => {
                        let outcome = if redo {
                            history.redo(&flag)
                        } else {
                            match crate::trash_history::root() {
                                Ok(root) => history.undo(&root, &flag),
                                Err(message) => Outcome {
                                    success: false,
                                    message,
                                    details: Vec::new(),
                                    undo: true,
                                    redo: false,
                                },
                            }
                        };
                        Response::Finished(id, outcome, refresh_directory)
                    }
                    Request::Trash {
                        directory,
                        names,
                        root,
                        ..
                    } => {
                        let mut batch = crate::trash_history::History::default();
                        let outcome = batch.send(&directory, &names, &root, &flag, || {});
                        history.push_trash(batch, directory);
                        Response::Finished(id, outcome, refresh_directory)
                    }
                    Request::Create {
                        directory,
                        name,
                        folder,
                        ..
                    } => {
                        let outcome = match create(&directory, &name, folder, &flag) {
                            Ok(created) => {
                                let mut details = Vec::new();
                                match created
                                    .ok_or_else(|| std::io::Error::other("sin recibo de identidad"))
                                {
                                    Ok(object) => match files_core::undo_ops::fence() {
                                        Ok(fence) => {
                                            history.apply(crate::undo_history::Command::Record(
                                                Box::new(crate::undo_history::Record::Created {
                                                    kind: if folder {
                                                        crate::undo_history::Kind::NewFolder
                                                    } else {
                                                        crate::undo_history::Kind::NewFile
                                                    },
                                                    objects: vec![object],
                                                    fence,
                                                }),
                                            ))
                                        }
                                        Err(e) => details.push(format!("Sin Deshacer: {e}")),
                                    },
                                    Err(e) => details.push(format!("Sin Deshacer: {e}")),
                                }
                                Outcome {
                                    success: true,
                                    message: format!("Creado: {name}"),
                                    details,
                                    undo: false,
                                    redo: false,
                                }
                            }
                            Err(e) => Outcome {
                                success: false,
                                message: format!("No se pudo crear: {e}"),
                                details: Vec::new(),
                                undo: false,
                                redo: false,
                            },
                        };
                        Response::Finished(id, outcome, refresh_directory)
                    }
                    Request::Record { commands, .. } => {
                        for command in commands {
                            history.apply(command);
                        }
                        Response::Recorded
                    }
                };
                let response = match response {
                    Response::Finished(id, mut outcome, directory) => {
                        outcome.undo = history.can_undo();
                        outcome.redo = history.can_redo();
                        Response::Finished(id, outcome, directory)
                    }
                    other => other,
                };
                *labels.lock().unwrap_or_else(|e| e.into_inner()) = history.labels();
                if let (Some(gate), Some(phase)) = (&gate, fixture_phase) {
                    gate.wait(phase, id, &flag, true);
                }
                if output.send(response).is_err() {
                    break;
                }
            }
        });
        self.tx = Some(tx);
        self.rx = Some(rx);
    }
    /// Busy includes an unread response: callers cannot accumulate prepared plans.
    pub fn submit(&mut self, request: Request) -> Result<(), Box<Request>> {
        if self.busy {
            return Err(Box::new(request));
        }
        self.cancel.store(false, Ordering::Release);
        if self.tx.is_none() {
            self.start();
        }
        match self.tx.as_ref().expect("worker started").try_send(request) {
            Ok(()) => {
                self.busy = true;
                Ok(())
            }
            Err(mpsc::TrySendError::Full(request) | mpsc::TrySendError::Disconnected(request)) => {
                Err(Box::new(request))
            }
        }
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
    pub fn poll(&mut self) -> Option<Response> {
        let rx = self.rx.as_ref()?;
        match rx.try_recv() {
            Ok(Response::Recorded) => {
                self.records_in_flight = self.records_in_flight.saturating_sub(1);
                Some(Response::Recorded)
            }
            Ok(value) => {
                self.busy = false;
                Some(value)
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) if self.records_in_flight > 0 && !self.busy => {
                self.records_in_flight = 0;
                None
            }
            Err(mpsc::TryRecvError::Disconnected) if self.busy => {
                self.busy = false;
                self.records_in_flight = 0;
                Some(Response::Finished(0, Outcome { success: false, message: "El trabajador terminó sin respuesta; revisa las rutas antes de continuar".into(), details: Vec::new(), undo: false, redo: false },None))
            }
            _ => None,
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel();
    }
}
fn check(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("Operación cancelada".into())
    } else {
        Ok(())
    }
}
fn plan(
    directory: PathBuf,
    selected: &[OsString],
    options: &RenameOptions,
    cancel: &AtomicBool,
) -> Result<PreparedRename, String> {
    plan_with_entry_limit(directory, selected, options, cancel, MAX_NAMES)
}
fn plan_with_entry_limit(
    directory: PathBuf,
    selected: &[OsString],
    options: &RenameOptions,
    cancel: &AtomicBool,
    max_entries: usize,
) -> Result<PreparedRename, String> {
    check(cancel)?;
    if selected.len() > MAX_NAMES {
        return Err("Reduce el lote: máximo 100000 nombres".into());
    }
    let mut budget = MAX_INPUT_BYTES;
    let mut names = Vec::with_capacity(selected.len());
    for name in selected {
        check(cancel)?;
        let bytes = name.as_encoded_bytes().len();
        if bytes > 4096 {
            return Err("Nombre demasiado largo".into());
        }
        budget = budget
            .checked_sub(bytes)
            .ok_or("Reduce el lote: presupuesto de nombres excedido")?;
        names.push(
            name.to_str()
                .ok_or(
                    "El lote seleccionado contiene nombres no UTF-8; no se convertirán con pérdida",
                )?
                .to_owned(),
        );
    }
    let mut existing = Vec::new();
    for (seen, entry) in fs::read_dir(&directory)
        .map_err(|e| e.to_string())?
        .enumerate()
    {
        check(cancel)?;
        if seen >= max_entries {
            return Err(
                "Carpeta mayor al límite de 100000 entradas (incluye nombres no UTF-8)".into(),
            );
        }
        let name = entry.map_err(|e| e.to_string())?.file_name();
        budget = budget
            .checked_sub(name.as_encoded_bytes().len())
            .ok_or("Reduce el lote: presupuesto de nombres excedido")?;
        // Invalid UTF-8 cannot equal any generated UTF-8 byte name. Preserve such
        // unselected entries; kernel no-clobber still validates actual targets.
        if let Ok(name) = name.into_string() {
            existing.push(name);
        }
    }
    let rows = rename_plan::plan_cancellable(&names, &existing, options, cancel)?;
    check(cancel)?;
    PreparedRename::capture(
        directory,
        rows.into_iter()
            .map(|r| (r.from.into(), r.to.into()))
            .collect(),
    )
    .map_err(|e| e.to_string())
}
pub(crate) fn cleanup_details(token: &UndoToken) -> Vec<String> {
    token
        .cleanup_errors
        .iter()
        .map(|e| format!("Limpieza pendiente: {e}"))
        .chain(
            token
                .cleanup_paths
                .iter()
                .map(|p| format!("Ruta de limpieza: {p:?}")),
        )
        .collect()
}
pub(crate) fn failed(error: TransactionError) -> Outcome {
    let mut details = error.rollback_errors;
    details.extend(
        error
            .preserved_paths
            .into_iter()
            .map(|p| format!("Rescate verificado: {p:?}")),
    );
    details.extend(
        error
            .unverified_parent_paths
            .into_iter()
            .map(|p| format!("Ubicación anterior NO verificada: {p:?}")),
    );
    details.push(format!("No restaurados: {}; sin ubicación verificada: {}; rutas agrupadas: {}; errores omitidos/truncados: {}/{}", error.preserved_object_count, error.unresolved_preserved_objects, error.aggregated_preserved_paths, error.omitted_rollback_errors, error.truncated_rollback_errors));
    Outcome {
        success: false,
        message: format!("No se pudo completar: {}", error.cause),
        details,
        undo: false,
        redo: false,
    }
}
/// Creates without replacing; on Linux also returns the Undo identity receipt
/// of exactly the inode this call created (`None` if it could not be taken).
fn create(
    directory: &std::path::Path,
    name: &str,
    folder: bool,
    cancel: &AtomicBool,
) -> std::io::Result<Option<files_core::undo_ops::Created>> {
    check(cancel).map_err(std::io::Error::other)?;
    rename_plan::valid_name(name, true).map_err(std::io::Error::other)?;
    if name.len() > 255 {
        return Err(std::io::Error::other(
            "Nombre demasiado largo (máximo 255 bytes)",
        ));
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::FromRawFd;
        use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
        let canonical = directory.canonicalize()?;
        let parent = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&canonical)?;
        let component = std::ffi::CString::new(name).map_err(std::io::Error::other)?;
        let path = canonical.join(name);
        if folder {
            if unsafe { libc::mkdirat(parent.as_raw_fd(), component.as_ptr(), 0o777) } != 0 {
                return Err(std::io::Error::last_os_error());
            }
            // No handle comes back from mkdirat: open it without following,
            // and accept the receipt only for an empty directory.
            let fd = unsafe {
                libc::openat(
                    parent.as_raw_fd(),
                    component.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                return Ok(None);
            }
            let handle = unsafe { fs::File::from_raw_fd(fd) };
            let empty = fs::read_dir(&path).is_ok_and(|mut d| d.next().is_none());
            Ok(empty
                .then(|| files_core::undo_ops::Created::from_file(path, &handle, false).ok())
                .flatten())
        } else {
            let fd = unsafe {
                libc::openat(
                    parent.as_raw_fd(),
                    component.as_ptr(),
                    libc::O_CREAT
                        | libc::O_EXCL
                        | libc::O_WRONLY
                        | libc::O_CLOEXEC
                        | libc::O_NOFOLLOW,
                    0o666,
                )
            };
            if fd < 0 {
                return Err(std::io::Error::last_os_error());
            }
            let handle = unsafe { fs::File::from_raw_fd(fd) };
            Ok(files_core::undo_ops::Created::from_file(path, &handle, false).ok())
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let target = directory.join(name);
        if folder {
            fs::create_dir(target).map(|()| None)
        } else {
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(target)
                .map(|_| None)
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;
    use std::time::{Duration, Instant};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "files-rename-worker-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    fn receive(worker: &mut Worker) -> Response {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(response) = worker.poll() {
                return response;
            }
            assert!(Instant::now() < deadline, "worker response timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn run(worker: &mut Worker, request: Request) -> Outcome {
        let expected = request.id();
        assert!(worker.submit(request).is_ok());
        match receive(worker) {
            Response::Finished(id, outcome, _) => {
                assert_eq!(id, expected);
                outcome
            }
            Response::Planned(_, _) | Response::Properties(_, _) | Response::Recorded => {
                panic!("expected finished mutation")
            }
        }
    }

    #[test]
    fn trash_create_failure_conflict_and_restore_retry_keep_history() {
        let dir = Fixture::new();
        let root = dir.0.join("Trash");
        fs::write(dir.0.join("before"), b"original").unwrap();
        let mut worker = Worker::new();
        let result = run(
            &mut worker,
            Request::Trash {
                id: 1,
                directory: dir.0.clone(),
                names: vec!["before".into()].into(),
                root: root.clone(),
            },
        );
        assert!(
            result.success && result.undo && !result.redo,
            "{} {:?}",
            result.message,
            result.details
        );
        assert!(!dir.0.join("before").exists());
        assert_eq!(fs::read_dir(root.join("files")).unwrap().count(), 1);
        fs::write(dir.0.join("occupied"), b"unchanged").unwrap();
        let failed = run(
            &mut worker,
            Request::Create {
                id: 2,
                directory: dir.0.clone(),
                name: "occupied".into(),
                folder: false,
            },
        );
        assert!(!failed.success && failed.undo && !failed.redo);
        fs::write(dir.0.join("before"), b"replacement").unwrap();
        let conflict = run(&mut worker, Request::History { id: 3, redo: false });
        assert!(!conflict.success && conflict.undo);
        assert_eq!(fs::read(dir.0.join("before")).unwrap(), b"replacement");
        assert!(
            conflict
                .details
                .iter()
                .any(|d| d.starts_with("Conservado:")),
            "{:?}",
            conflict.details
        );
        fs::remove_file(dir.0.join("before")).unwrap();
        let result = run(&mut worker, Request::History { id: 4, redo: false });
        assert!(
            result.success && !result.undo && !result.redo,
            "{} {:?}",
            result.message,
            result.details
        );
        assert_eq!(fs::read(dir.0.join("before")).unwrap(), b"original");
        assert_eq!(fs::read(dir.0.join("occupied")).unwrap(), b"unchanged");
        assert_eq!(fs::read_dir(root.join("files")).unwrap().count(), 0);
        assert_eq!(fs::read_dir(root.join("info")).unwrap().count(), 0);
    }
    #[test]
    fn trash_failure_before_commit_preserves_rename_undo() {
        let dir = Fixture::new();
        fs::write(dir.0.join("before"), b"keep").unwrap();
        let mut worker = Worker::new();
        let prepared =
            PreparedRename::capture(dir.0.clone(), vec![("before".into(), "after".into())])
                .unwrap();
        assert!(run(&mut worker, Request::Apply { id: 1, prepared }).success);
        let result = run(
            &mut worker,
            Request::Trash {
                id: 2,
                directory: dir.0.clone(),
                names: vec!["missing".into()].into(),
                root: dir.0.join("Trash"),
            },
        );
        assert!(!result.success && result.undo && !result.redo);
        assert!(run(&mut worker, Request::History { id: 3, redo: false }).success);
        assert_eq!(fs::read(dir.0.join("before")).unwrap(), b"keep");
    }

    #[test]
    fn failed_creation_keeps_undo_and_redo_accessible() {
        let dir = Fixture::new();
        fs::write(dir.0.join("before"), b"preserved contents").unwrap();
        let mut worker = Worker::new();
        let prepared =
            PreparedRename::capture(dir.0.clone(), vec![("before".into(), "after".into())])
                .unwrap();
        let result = run(&mut worker, Request::Apply { id: 1, prepared });
        assert!(result.success && result.undo && !result.redo);
        let result = run(
            &mut worker,
            Request::Create {
                id: 2,
                directory: dir.0.clone(),
                name: "after".into(),
                folder: false,
            },
        );
        assert!(!result.success && result.undo && !result.redo);
        assert_eq!(
            fs::read(dir.0.join("after")).unwrap(),
            b"preserved contents"
        );
        let result = run(&mut worker, Request::History { id: 3, redo: false });
        assert!(result.success && !result.undo && result.redo);
        let result = run(
            &mut worker,
            Request::Create {
                id: 4,
                directory: dir.0.clone(),
                name: "before".into(),
                folder: true,
            },
        );
        assert!(!result.success && !result.undo && result.redo);
        let result = run(&mut worker, Request::History { id: 5, redo: true });
        assert!(result.success && result.undo && !result.redo);
        assert_eq!(
            fs::read(dir.0.join("after")).unwrap(),
            b"preserved contents"
        );
    }

    #[test]
    fn cancelled_history_retains_token_for_retry() {
        let dir = Fixture::new();
        fs::write(dir.0.join("before"), b"data").unwrap();
        let token = PreparedRename::capture(dir.0.clone(), vec![("before".into(), "after".into())])
            .unwrap()
            .execute_cancellable(&AtomicBool::new(false))
            .unwrap();
        let mut history = crate::undo_history::History::default();
        history.push_rename(token, dir.0.clone(), 1);
        let root = dir.0.join("Trash");
        let result = history.undo(&root, &AtomicBool::new(true));
        assert!(!result.success && result.undo && !result.redo);
        assert!(dir.0.join("after").exists());
        let result = history.undo(&root, &AtomicBool::new(false));
        assert!(result.success && !result.undo && result.redo);
        assert_eq!(fs::read(dir.0.join("before")).unwrap(), b"data");
    }

    #[test]
    fn prepared_response_must_be_consumed_before_next_request() {
        let dir = Fixture::new();
        fs::write(dir.0.join("a"), b"a").unwrap();
        let mut worker = Worker::new();
        assert!(
            worker
                .submit(Request::Plan {
                    id: 7,
                    directory: dir.0.clone(),
                    selected: vec![OsString::from("a")].into(),
                    options: RenameOptions::default()
                })
                .is_ok()
        );
        assert!(
            worker
                .submit(Request::History { id: 8, redo: false })
                .is_err()
        );
        match receive(&mut worker) {
            Response::Planned(id, plan) => {
                assert_eq!(id, 7);
                assert!(plan.is_ok());
            }
            _ => panic!("expected plan"),
        }
        assert!(!worker.busy);
    }

    #[test]
    fn creation_rejects_reserved_names_and_never_clobbers() {
        let dir = Fixture::new();
        let cancel = AtomicBool::new(false);
        for name in ["CON.txt", "LPT1", "../outside", "a/b"] {
            assert!(create(&dir.0, name, false, &cancel).is_err());
        }
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 0);
        create(&dir.0, "valid.txt", false, &cancel).unwrap();
        fs::write(dir.0.join("valid.txt"), b"keep").unwrap();
        assert!(create(&dir.0, "valid.txt", false, &cancel).is_err());
        assert_eq!(fs::read(dir.0.join("valid.txt")).unwrap(), b"keep");
        cancel.store(true, Ordering::Release);
        assert!(create(&dir.0, "cancelled", true, &cancel).is_err());
        assert!(!dir.0.join("cancelled").exists());
    }
    #[test]
    fn every_directory_entry_counts_toward_limit_even_when_not_utf8() {
        use std::os::unix::ffi::OsStringExt;
        let dir = Fixture::new();
        fs::write(dir.0.join("a"), b"a").unwrap();
        fs::write(dir.0.join(OsString::from_vec(vec![0xff])), b"keep").unwrap();
        let flag = AtomicBool::new(false);
        let selected = vec![OsString::from("a")];
        assert!(
            plan_with_entry_limit(
                dir.0.clone(),
                &selected,
                &RenameOptions::default(),
                &flag,
                2
            )
            .is_ok()
        );
        fs::write(dir.0.join(OsString::from_vec(vec![0xfe])), b"keep").unwrap();
        let error = plan_with_entry_limit(
            dir.0.clone(),
            &selected,
            &RenameOptions::default(),
            &flag,
            2,
        )
        .err()
        .unwrap();
        assert!(error.contains("incluye nombres no UTF-8"));
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 3);
    }
}
