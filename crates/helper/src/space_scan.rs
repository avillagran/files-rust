//! Bounded apparent-byte scan for a treemap, not a disk-block ownership report.
//! Only immediate child names/results are retained. Directory streams and stat /
//! openat calls are anchored to pinned FDs; final symlinks are never followed.
//! Symlinks/special objects contribute zero bytes and are not traversed. Hardlink
//! aliases are counted per regular-file name. Concurrent in-place changes and
//! mount traversal are not a snapshot; cooperative timeouts cannot stop an FS
//! syscall. The caller must supervise/kill/reap this one-shot helper off the UI.
//! Node.files counts observed regular-file names; Node.directories includes that
//! immediate directory itself. visited counts encountered non-dot entries, not
//! the scan root. omitted_children refers only to unrepresented immediate
//! children: Some(0) requires EOF on the root stream, even if a listed subtree
//! was partial. None is used whenever enumeration stopped before proving EOF.
use files_core::space_map::{
    MAX_ERROR_BYTES, MAX_ERRORS_BYTES, MAX_NAME_BYTES, MAX_NAMES_BYTES, MAX_NODES,
    MAX_SCAN_SECONDS, MAX_VISITED, NodeKind, NodeStatus, SpaceNode, SpaceReport,
};
use std::{
    path::Path,
    time::{Duration, Instant},
};
#[derive(Clone, Copy)]
struct Limits {
    visited: u64,
    depth: usize,
    elapsed: Duration,
    nodes: usize,
    names: usize,
    errors: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            visited: MAX_VISITED,
            depth: 128,
            elapsed: Duration::from_secs(MAX_SCAN_SECONDS),
            nodes: MAX_NODES,
            names: MAX_NAMES_BYTES,
            errors: MAX_ERRORS_BYTES,
        }
    }
}
pub fn scan(path: &Path) -> Result<SpaceReport, String> {
    platform::scan(path, Limits::default())
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use std::{
        ffi::{CStr, CString},
        fmt::{self, Write},
        fs::File,
        io,
        os::{
            fd::{AsRawFd, FromRawFd, IntoRawFd},
            unix::ffi::OsStrExt,
        },
        ptr::NonNull,
        sync::{
            Condvar, Mutex, MutexGuard,
            atomic::{AtomicBool, AtomicU64, Ordering},
        },
    };
    struct Stream {
        directory: NonNull<libc::DIR>,
    }
    impl Stream {
        fn from_file(file: File) -> io::Result<Self> {
            let fd = file.into_raw_fd();
            let directory = unsafe { libc::fdopendir(fd) };
            match NonNull::new(directory) {
                Some(directory) => Ok(Self { directory }),
                None => {
                    let error = io::Error::last_os_error();
                    unsafe {
                        libc::close(fd);
                    }
                    Err(error)
                }
            }
        }
        fn fd(&self) -> i32 {
            unsafe { libc::dirfd(self.directory.as_ptr()) }
        }
        fn next(&mut self) -> io::Result<Option<&CStr>> {
            loop {
                unsafe {
                    *libc::__errno_location() = 0;
                }
                let entry = unsafe { libc::readdir(self.directory.as_ptr()) };
                if entry.is_null() {
                    let error = io::Error::last_os_error();
                    return if error.raw_os_error() == Some(0) {
                        Ok(None)
                    } else {
                        Err(error)
                    };
                }
                // Linux readdir returns a NUL-terminated name in its stream-owned
                // buffer. Keep it borrowed until the next call on THIS stream.
                let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
                if !matches!(name.to_bytes(), b"." | b"..") {
                    return Ok(Some(name));
                }
            }
        }
    }
    impl Drop for Stream {
        fn drop(&mut self) {
            unsafe {
                libc::closedir(self.directory.as_ptr());
            }
        }
    }
    fn file(fd: i32) -> io::Result<File> {
        if fd < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    }
    fn stat_fd(file: &File) -> io::Result<libc::stat> {
        let mut out = std::mem::MaybeUninit::uninit();
        if unsafe { libc::fstat(file.as_raw_fd(), out.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(unsafe { out.assume_init() })
    }
    fn stat_at(fd: i32, name: &CStr) -> io::Result<libc::stat> {
        let mut out = std::mem::MaybeUninit::uninit();
        if unsafe {
            libc::fstatat(
                fd,
                name.as_ptr(),
                out.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(unsafe { out.assume_init() })
    }
    fn child_directory(fd: i32, name: &CStr, expected: &libc::stat) -> io::Result<Stream> {
        let file = file(unsafe {
            libc::openat(
                fd,
                name.as_ptr(),
                libc::O_RDONLY
                    | libc::O_DIRECTORY
                    | libc::O_NOFOLLOW
                    | libc::O_CLOEXEC
                    | libc::O_NONBLOCK,
            )
        })?;
        let actual = stat_fd(&file)?;
        if expected.st_dev != actual.st_dev
            || expected.st_ino != actual.st_ino
            || actual.st_mode & libc::S_IFMT != libc::S_IFDIR
        {
            return Err(io::Error::other(
                "La carpeta cambió antes de abrirse; recorrido parcial",
            ));
        }
        Stream::from_file(file)
    }
    fn kind(stat: &libc::stat) -> NodeKind {
        match stat.st_mode & libc::S_IFMT {
            libc::S_IFREG => NodeKind::File,
            libc::S_IFDIR => NodeKind::Directory,
            libc::S_IFLNK => NodeKind::Symlink,
            _ => NodeKind::Other,
        }
    }
    struct Text {
        text: String,
        cap: usize,
    }
    impl fmt::Write for Text {
        fn write_str(&mut self, value: &str) -> fmt::Result {
            let remaining = self.cap.saturating_sub(self.text.len());
            if value.len() <= remaining {
                self.text.push_str(value);
                return Ok(());
            }
            let mut end = remaining;
            while !value.is_char_boundary(end) {
                end -= 1;
            }
            self.text.push_str(&value[..end]);
            Err(fmt::Error)
        }
    }
    fn text(arguments: fmt::Arguments<'_>, cap: usize) -> String {
        let mut text = Text {
            text: String::with_capacity(cap),
            cap,
        };
        let _ = text.write_fmt(arguments);
        text.text
    }
    struct Budget {
        limits: Limits,
        started: Instant,
        visited: u64,
        names: usize,
        errors_reserved: usize,
    }
    impl Budget {
        fn new(limits: Limits) -> Self {
            // Reserve one bounded reason for the report BEFORE node diagnostics.
            Self {
                limits,
                started: Instant::now(),
                visited: 0,
                names: 0,
                errors_reserved: MAX_ERROR_BYTES.min(limits.errors),
            }
        }
        fn time(&self) -> Result<(), Issue> {
            if self.started.elapsed() >= self.limits.elapsed {
                Err(Issue::limit("Tiempo de cálculo de espacio agotado"))
            } else {
                Ok(())
            }
        }
        fn visit(&mut self) -> Result<(), Issue> {
            if self.visited >= self.limits.visited {
                Err(Issue::limit("Límite de entradas de espacio alcanzado"))
            } else {
                self.visited += 1;
                Ok(())
            }
        }
        fn name(&mut self, name: &[u8]) -> Result<(), Issue> {
            if name.is_empty() || name.len() > MAX_NAME_BYTES {
                return Err(Issue::limit(
                    "Nombre inmediato fuera del límite de4096bytes",
                ));
            }
            self.names = self
                .names
                .checked_add(name.len())
                .filter(|n| *n <= self.limits.names)
                .ok_or_else(|| {
                    Issue::limit("Presupuesto agregado de nombres inmediatos agotado")
                })?;
            Ok(())
        }
        fn error(&mut self, issue: &Issue) -> Option<String> {
            let cap = self
                .limits
                .errors
                .saturating_sub(self.errors_reserved)
                .min(MAX_ERROR_BYTES);
            if cap == 0 {
                return None;
            }
            self.errors_reserved += cap; // account BEFORE allocating/formatting
            Some(text(format_args!("{}: {}", issue.reason, issue.error), cap))
        }
    }
    struct Issue {
        reason: &'static str,
        error: io::Error,
        stop_all: bool,
    }
    impl Issue {
        fn limit(reason: &'static str) -> Self {
            Self {
                reason,
                error: io::Error::other(reason),
                stop_all: true,
            }
        }
        fn io(reason: &'static str, error: io::Error) -> Self {
            Self {
                reason,
                error,
                stop_all: false,
            }
        }
    }
    #[derive(Default)]
    struct Counts {
        bytes: u64,
        files: u64,
        directories: u64,
    }
    fn file_bytes(stat: &libc::stat, counts: &mut Counts) -> Result<(), Issue> {
        let bytes = u64::try_from(stat.st_size).map_err(|_| {
            Issue::io(
                "Tamaño aparente no válido",
                io::Error::other("Tamaño negativo"),
            )
        })?;
        counts.bytes = counts.bytes.checked_add(bytes).ok_or_else(|| {
            Issue::io(
                "Tamaño aparente fuera de rango",
                io::Error::other("Desbordamiento de bytes"),
            )
        })?;
        counts.files += 1;
        Ok(())
    }
    // SAFETY: a DIR stream is used by one thread at a time. Ownership moves
    // between walk workers through the shared queue and is never aliased.
    unsafe impl Send for Stream {}
    /// Upper bound of walk workers for one subtree. Cold metadata reads are
    /// latency bound, so a few concurrent walkers keep the device busy.
    const MAX_WORKERS: usize = 8;
    const WORKER_STACK: usize = 256 * 1024;
    struct Shared {
        queue: Vec<(Stream, usize)>,
        idle: usize,
        workers: usize,
        spawned: bool,
        done: bool,
    }
    /// One subtree walk shared by its workers. Each worker descends depth
    /// first with its own stack of pinned directory streams; it hands a newly
    /// opened subdirectory to the shared queue only while another worker is
    /// idle, so open descriptors stay bounded by workers x depth.
    struct Walk {
        limits: Limits,
        started: Instant,
        visited: AtomicU64,
        stop: AtomicBool,
        issue: Mutex<Option<Issue>>,
        totals: Mutex<Result<Counts, Issue>>,
        shared: Mutex<Shared>,
        ready: Condvar,
        max: usize,
    }
    fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
        m.lock().unwrap_or_else(|e| e.into_inner())
    }
    impl Walk {
        fn time(&self) -> Result<(), Issue> {
            if self.started.elapsed() >= self.limits.elapsed {
                Err(Issue::limit("Tiempo de cálculo de espacio agotado"))
            } else {
                Ok(())
            }
        }
        /// Exact visit cap shared by every worker.
        fn visit(&self) -> Result<(), Issue> {
            let limit = self.limits.visited;
            let mut seen = self.visited.load(Ordering::Acquire);
            loop {
                if seen >= limit {
                    return Err(Issue::limit("Límite de entradas de espacio alcanzado"));
                }
                match self.visited.compare_exchange_weak(
                    seen,
                    seen + 1,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => return Ok(()),
                    Err(now) => seen = now,
                }
            }
        }
        /// A problem that leaves the rest of the subtree measurable: the walk
        /// continues and the subtree is reported partial with this reason.
        fn note(&self, issue: Issue) {
            let mut slot = lock(&self.issue);
            if slot.is_none() {
                *slot = Some(issue);
            }
        }
        /// A limit: every worker stops. It replaces a noted problem so the
        /// caller can tell a budget stop from an unreadable entry.
        fn fail(&self, issue: Issue) {
            {
                let mut slot = lock(&self.issue);
                if slot.as_ref().is_none_or(|i| !i.stop_all) {
                    *slot = Some(issue);
                }
            }
            self.stop.store(true, Ordering::Release);
            let _guard = lock(&self.shared);
            self.ready.notify_all();
        }
        fn take(&self) -> Option<(Stream, usize)> {
            let mut s = lock(&self.shared);
            loop {
                if s.done || self.stop.load(Ordering::Acquire) {
                    return None;
                }
                if let Some(item) = s.queue.pop() {
                    return Some(item);
                }
                s.idle += 1;
                if s.idle == s.workers {
                    s.done = true;
                    self.ready.notify_all();
                    return None;
                }
                s = self.ready.wait(s).unwrap_or_else(|e| e.into_inner());
                s.idle -= 1;
            }
        }
        /// Hands `child` to an idle worker (spawning the workers on the first
        /// subdirectory) or keeps it on the local stack.
        fn place(
            &self,
            child: Stream,
            depth: usize,
            local: &mut Vec<(Stream, usize)>,
            spawn: &dyn Fn(&mut Shared),
        ) {
            if self.max > 1 {
                let mut s = lock(&self.shared);
                if !s.spawned {
                    s.spawned = true;
                    spawn(&mut s);
                }
                if s.idle > s.queue.len() {
                    s.queue.push((child, depth));
                    self.ready.notify_one();
                    return;
                }
            }
            local.push((child, depth));
        }
        fn run(&self, spawn: &dyn Fn(&mut Shared)) {
            let mut counts = Counts::default();
            let mut local: Vec<(Stream, usize)> = Vec::new();
            loop {
                if local.is_empty() {
                    match self.take() {
                        Some(item) => local.push(item),
                        None => break,
                    }
                }
                self.descend(&mut local, &mut counts, spawn);
                if self.stop.load(Ordering::Acquire) {
                    break;
                }
            }
            drop(local);
            let mut totals = lock(&self.totals);
            if let Ok(t) = totals.as_mut() {
                match t.bytes.checked_add(counts.bytes) {
                    Some(bytes) => {
                        t.bytes = bytes;
                        t.files += counts.files;
                        t.directories += counts.directories;
                    }
                    None => {
                        *totals = Err(Issue::io(
                            "Tamaño aparente fuera de rango",
                            io::Error::other("Desbordamiento de bytes"),
                        ))
                    }
                }
            }
        }
        fn descend(
            &self,
            local: &mut Vec<(Stream, usize)>,
            counts: &mut Counts,
            spawn: &dyn Fn(&mut Shared),
        ) {
            while let Some((frame, depth)) = local.last_mut() {
                if self.stop.load(Ordering::Acquire) {
                    return;
                }
                if let Err(issue) = self.time() {
                    return self.fail(issue);
                }
                let (fd, depth) = (frame.fd(), *depth);
                let name = match frame.next() {
                    Ok(Some(name)) => name,
                    Ok(None) => {
                        local.pop();
                        continue;
                    }
                    Err(error) => {
                        self.note(Issue::io("No se pudo enumerar una subcarpeta", error));
                        local.pop();
                        continue;
                    }
                };
                if let Err(issue) = self.visit() {
                    return self.fail(issue);
                }
                if name.to_bytes().len() > MAX_NAME_BYTES {
                    return self.fail(Issue::limit("Nombre de subcarpeta fuera del límite"));
                }
                let stat = match stat_at(fd, name) {
                    Ok(stat) => stat,
                    Err(error) => {
                        self.note(Issue::io(
                            "No se pudo leer una entrada de subcarpeta",
                            error,
                        ));
                        continue;
                    }
                };
                match kind(&stat) {
                    NodeKind::File => {
                        if let Err(issue) = file_bytes(&stat, counts) {
                            self.note(issue);
                        }
                    }
                    NodeKind::Directory => {
                        counts.directories += 1;
                        if depth >= self.limits.depth {
                            return self
                                .fail(Issue::limit("Límite de profundidad de espacio alcanzado"));
                        }
                        match child_directory(fd, name, &stat) {
                            Ok(child) => self.place(child, depth + 1, local, spawn),
                            Err(error) => {
                                self.note(Issue::io("No se pudo abrir una subcarpeta", error))
                            }
                        }
                    }
                    NodeKind::Symlink | NodeKind::Other => (),
                }
            }
        }
    }
    fn workers() -> usize {
        std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .clamp(1, MAX_WORKERS)
    }
    /// Walks `root` (already counted as one directory in `counts`) with up to
    /// [`MAX_WORKERS`] threads under `budget`. Entries that cannot be read are
    /// skipped and reported; limits stop the whole walk.
    fn walk(root: Stream, budget: &mut Budget, counts: &mut Counts) -> Result<(), Issue> {
        if budget.limits.depth == 0 {
            return Err(Issue::limit("Límite de profundidad de espacio alcanzado"));
        }
        let walk = Walk {
            limits: budget.limits,
            started: budget.started,
            visited: AtomicU64::new(budget.visited),
            stop: AtomicBool::new(false),
            issue: Mutex::new(None),
            totals: Mutex::new(Ok(Counts::default())),
            shared: Mutex::new(Shared {
                queue: vec![(root, 1)],
                idle: 0,
                workers: 1,
                spawned: false,
                done: false,
            }),
            ready: Condvar::new(),
            max: workers(),
        };
        std::thread::scope(|scope| {
            let spawn = |s: &mut Shared| {
                for _ in 1..walk.max {
                    // Counted before the thread can look for work, so the
                    // idle count never reaches the worker count early.
                    s.workers += 1;
                    let started = std::thread::Builder::new()
                        .name("space-walk".into())
                        .stack_size(WORKER_STACK)
                        .spawn_scoped(scope, || walk.run(&|_| ()));
                    if started.is_err() {
                        s.workers -= 1;
                        break;
                    }
                }
            };
            walk.run(&spawn);
        });
        budget.visited = walk.visited.into_inner();
        let totals = walk.totals.into_inner().unwrap_or_else(|e| e.into_inner());
        let issue = walk.issue.into_inner().unwrap_or_else(|e| e.into_inner());
        let totals = totals?;
        counts.bytes = counts.bytes.checked_add(totals.bytes).ok_or_else(|| {
            Issue::io(
                "Tamaño aparente fuera de rango",
                io::Error::other("Desbordamiento de bytes"),
            )
        })?;
        counts.files += totals.files;
        counts.directories += totals.directories;
        match issue {
            Some(issue) => Err(issue),
            None => Ok(()),
        }
    }
    fn inspect_entry(fd: i32, name: &CStr) -> (SpaceNode, Option<libc::stat>, Option<Issue>) {
        // The immediate-name budget was checked by the caller before this clone.
        let mut node = SpaceNode {
            name: name.to_bytes().to_vec(),
            kind: NodeKind::Other,
            bytes: None,
            observed_bytes: 0,
            files: 0,
            directories: 0,
            status: NodeStatus::Error,
            error: None,
        };
        let stat = match stat_at(fd, name) {
            Ok(stat) => stat,
            Err(error) => {
                return (
                    node,
                    None,
                    Some(Issue::io("No se pudo leer la entrada inmediata", error)),
                );
            }
        };
        node.kind = kind(&stat);
        let mut counts = Counts::default();
        let result = match node.kind {
            NodeKind::File => file_bytes(&stat, &mut counts),
            NodeKind::Directory => {
                // Retain every admitted immediate child before descending. A
                // large first subtree must not hide later files and folders.
                node.directories = 1;
                node.status = NodeStatus::Partial;
                return (node, Some(stat), None);
            }
            NodeKind::Symlink | NodeKind::Other => Ok(()),
        };
        node.observed_bytes = counts.bytes;
        node.files = counts.files;
        node.directories = counts.directories;
        match result {
            Ok(()) => {
                node.status = NodeStatus::Complete;
                node.bytes = Some(counts.bytes);
                (node, None, None)
            }
            Err(issue) => {
                node.status = NodeStatus::Partial;
                (node, None, Some(issue))
            }
        }
    }
    fn measure_directory(
        fd: i32,
        node: &mut SpaceNode,
        expected: &libc::stat,
        budget: &mut Budget,
        remaining: usize,
    ) -> Option<Issue> {
        if let Err(issue) = budget.time() {
            return Some(issue);
        }
        let name = CString::new(node.name.as_slice()).expect("validated child component");
        let directory = match child_directory(fd, &name, expected) {
            Ok(directory) => directory,
            Err(error) => {
                node.status = NodeStatus::Error;
                return Some(Issue::io("No se pudo abrir la carpeta inmediata", error));
            }
        };
        // Distribute the remaining cooperative budget among admitted siblings.
        // There is still only one helper and at most one depth-bounded walk.
        let remaining = remaining.max(1) as u32;
        let visits = budget.limits.visited.saturating_sub(budget.visited);
        let share = (visits / u64::from(remaining)).max(1).min(visits);
        let elapsed = budget.started.elapsed();
        let time = budget.limits.elapsed.saturating_sub(elapsed) / remaining;
        let mut slice = Budget {
            limits: Limits {
                visited: budget.visited + share,
                elapsed: elapsed.saturating_add(time).min(budget.limits.elapsed),
                ..budget.limits
            },
            started: budget.started,
            visited: budget.visited,
            names: budget.names,
            errors_reserved: budget.errors_reserved,
        };
        let mut counts = Counts {
            directories: 1,
            ..Counts::default()
        };
        let result = walk(directory, &mut slice, &mut counts);
        budget.visited = slice.visited;
        node.observed_bytes = counts.bytes;
        node.files = counts.files;
        node.directories = counts.directories;
        match result {
            Ok(()) => {
                node.status = NodeStatus::Complete;
                node.bytes = Some(counts.bytes);
                None
            }
            Err(mut issue) => {
                if issue.stop_all
                    && budget.time().is_ok()
                    && budget.visited < budget.limits.visited
                    && matches!(
                        issue.reason,
                        "Tiempo de cálculo de espacio agotado"
                            | "Límite de entradas de espacio alcanzado"
                    )
                {
                    issue = Issue::io(
                        SHARED_TURN,
                        io::Error::other("Las demás carpetas conservan su turno"),
                    );
                }
                Some(issue)
            }
        }
    }
    /// A subtree stopped only by its fair share; it is retried after its siblings.
    const SHARED_TURN: &str = "Medición parcial para repartir el presupuesto entre carpetas";
    fn reason(report: &mut SpaceReport, issue: &Issue) {
        report.complete = false;
        if report.reason.is_none() {
            report.reason = Some(text(
                format_args!("{}: {}", issue.reason, issue.error),
                MAX_ERROR_BYTES,
            ));
        }
    }
    pub(super) fn scan(path: &Path, mut limits: Limits) -> Result<SpaceReport, String> {
        limits.visited = limits.visited.min(MAX_VISITED);
        limits.depth = limits.depth.min(128);
        limits.elapsed = limits.elapsed.min(Duration::from_secs(MAX_SCAN_SECONDS));
        limits.nodes = limits.nodes.min(MAX_NODES);
        limits.names = limits.names.min(MAX_NAMES_BYTES);
        limits.errors = limits.errors.min(MAX_ERRORS_BYTES);
        let bytes = path.as_os_str().as_bytes();
        if !path.is_absolute() || bytes.len() > 4096 || bytes.contains(&0) {
            return Err(
                "La vista Espacio requiere una ruta absoluta de hasta4096bytes, sin NUL".into(),
            );
        }
        // Remove trailing separators so O_NOFOLLOW applies to the named root
        // itself: a path ending in "symlink/" must not bypass the final-link
        // rejection. Ancestor symlinks retain normal path-resolution semantics.
        let mut end = bytes.len();
        while end > 1 && bytes[end - 1] == b'/' {
            end -= 1;
        }
        let path = CString::new(&bytes[..end]).map_err(|_| "NUL en ruta de espacio".to_string())?;
        // The helper runs under a 96 MiB address-space limit. Walk workers
        // allocate little, so they share the main malloc arena instead of
        // reserving a 64 MiB arena each.
        #[cfg(target_env = "gnu")]
        unsafe {
            libc::mallopt(libc::M_ARENA_MAX, 1);
        }
        let mut budget = Budget::new(limits);
        let root = file(unsafe {
            libc::open(
                path.as_ptr(),
                libc::O_RDONLY
                    | libc::O_DIRECTORY
                    | libc::O_NOFOLLOW
                    | libc::O_CLOEXEC
                    | libc::O_NONBLOCK,
            )
        })
        .map_err(|error| {
            text(
                format_args!("No se pudo abrir una carpeta real para Espacio: {error}"),
                MAX_ERROR_BYTES,
            )
        })?;
        let mut root = Stream::from_file(root).map_err(|error| {
            text(
                format_args!("No se pudo enumerar Espacio: {error}"),
                MAX_ERROR_BYTES,
            )
        })?;
        let mut report = SpaceReport {
            nodes: Vec::new(),
            total_bytes: 0,
            observed_bytes: 0,
            visited: 0,
            complete: true,
            reason: None,
            omitted_children: None,
        };
        let mut pending = Vec::new();
        loop {
            if let Err(issue) = budget.time() {
                reason(&mut report, &issue);
                break;
            }
            let fd = root.fd();
            let name = match root.next() {
                Ok(Some(name)) => name,
                Ok(None) => {
                    report.omitted_children = Some(0);
                    break;
                }
                Err(error) => {
                    reason(
                        &mut report,
                        &Issue::io("No se pudo enumerar la carpeta raíz", error),
                    );
                    break;
                }
            };
            if report.nodes.len() >= budget.limits.nodes {
                reason(
                    &mut report,
                    &Issue::limit("Límite de hijos inmediatos de Espacio alcanzado"),
                );
                break;
            }
            if let Err(issue) = budget.name(name.to_bytes()).and_then(|()| budget.visit()) {
                reason(&mut report, &issue);
                break;
            }
            let (mut node, directory, issue) = inspect_entry(fd, name);
            let mut stop = false;
            if let Some(issue) = issue {
                reason(&mut report, &issue);
                node.error = budget.error(&issue);
                if node.error.is_none() {
                    report.reason = Some("Presupuesto agregado de diagnósticos de Espacio agotado; recorrido detenido".into());
                }
                stop = issue.stop_all || node.error.is_none();
            }
            if let Some(stat) = directory {
                pending.push((report.nodes.len(), stat));
            }
            report.nodes.push(node);
            if stop {
                break;
            }
        }
        let directories = pending.len();
        // First pass: each subtree gets a fair share so a large one cannot hide
        // its later siblings. Second pass: subtrees stopped only by that share
        // are measured again from scratch with everything that remains, so a
        // scan with enough budget is complete. Re-walked entries count again
        // in `visited`, which is a cost, not a population.
        let mut retry = Vec::new();
        let mut measured = Vec::with_capacity(directories);
        for (rank, (index, expected)) in pending.iter().enumerate() {
            let issue = measure_directory(
                root.fd(),
                &mut report.nodes[*index],
                expected,
                &mut budget,
                directories - rank,
            );
            match issue {
                Some(issue) if issue.reason == SHARED_TURN => retry.push((*index, *expected)),
                other => measured.push((*index, other)),
            }
        }
        for (index, expected) in &retry {
            let issue = measure_directory(
                root.fd(),
                &mut report.nodes[*index],
                expected,
                &mut budget,
                1,
            );
            measured.push((*index, issue));
        }
        for (index, issue) in measured {
            let index = &index;
            if let Some(issue) = issue {
                reason(&mut report, &issue);
                report.nodes[*index].error = budget.error(&issue);
                if report.nodes[*index].error.is_none() {
                    report.reason = Some("Presupuesto agregado de diagnósticos de Espacio agotado; recorrido detenido".into());
                    break;
                }
            }
        }
        // Sum only after enumeration and measurement; pending nodes remain
        // explicitly unknown, while measured lower bounds remain available.
        for node in &report.nodes {
            report.observed_bytes = report
                .observed_bytes
                .checked_add(node.observed_bytes)
                .ok_or("Suma observada de espacio fuera de rango")?;
            report.total_bytes = report
                .total_bytes
                .checked_add(node.bytes.unwrap_or(0))
                .ok_or("Suma completa de espacio fuera de rango")?;
        }
        report.visited = budget.visited;
        Ok(report)
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        use std::{
            fs,
            os::unix::{ffi::OsStringExt, fs::symlink},
            sync::atomic::{AtomicU64, Ordering},
        };
        struct Fixture(std::path::PathBuf);
        impl Fixture {
            fn new() -> Self {
                static NEXT: AtomicU64 = AtomicU64::new(0);
                let base = std::path::PathBuf::from(
                    std::env::var_os("TMPDIR").expect("space tests require a private TMPDIR"),
                );
                assert!(base.is_absolute());
                let mut parent = file(unsafe {
                    libc::open(
                        c"/".as_ptr(),
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
                    )
                })
                .unwrap();
                for component in base.components() {
                    match component {
                        std::path::Component::RootDir => (),
                        std::path::Component::Normal(name) => {
                            let name = CString::new(name.as_bytes()).unwrap();
                            parent = file(unsafe {
                                libc::openat(
                                    parent.as_raw_fd(),
                                    name.as_ptr(),
                                    libc::O_RDONLY
                                        | libc::O_DIRECTORY
                                        | libc::O_NOFOLLOW
                                        | libc::O_CLOEXEC,
                                )
                            })
                            .unwrap();
                        }
                        _ => panic!("private TMPDIR must not contain traversal components"),
                    }
                }
                let metadata = stat_fd(&parent).unwrap();
                assert_eq!(metadata.st_uid, unsafe { libc::geteuid() });
                assert_eq!(metadata.st_mode & 0o077, 0, "TMPDIR must be private");
                let name = format!(
                    "space-scan-{}-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                );
                let component = CString::new(name.as_bytes()).unwrap();
                assert_eq!(
                    unsafe { libc::mkdirat(parent.as_raw_fd(), component.as_ptr(), 0o700) },
                    0
                );
                // No Drop cleanup: every fixture, including link targets, remains
                // inside the exclusive test run for inspection after failures.
                Self(base.join(name))
            }
        }
        fn defaults(path: &Path) -> SpaceReport {
            scan(path, Limits::default()).unwrap()
        }
        #[test]
        fn true_weights_empty_and_lossless_names() {
            let f = Fixture::new();
            fs::write(f.0.join("eight"), [0; 8]).unwrap();
            fs::create_dir(f.0.join("four")).unwrap();
            fs::write(f.0.join("four/data"), [0; 4]).unwrap();
            let raw = std::ffi::OsString::from_vec(b"two\xff".to_vec());
            fs::write(f.0.join(&raw), [0; 2]).unwrap();
            fs::create_dir(f.0.join("empty")).unwrap();
            let report = defaults(&f.0);
            assert!(report.complete);
            assert_eq!((report.total_bytes, report.observed_bytes), (14, 14));
            assert_eq!(report.omitted_children, Some(0));
            assert_eq!(report.visited, 5);
            for (name, bytes) in [
                (b"eight".as_slice(), 8),
                (b"four", 4),
                (b"two\xff", 2),
                (b"empty", 0),
            ] {
                let node = report.nodes.iter().find(|node| node.name == name).unwrap();
                assert_eq!(node.bytes, Some(bytes));
                assert_eq!(node.observed_bytes, bytes);
                assert!(matches!(node.status, NodeStatus::Complete));
            }
        }
        #[test]
        fn no_symlink_cycles_external_follow_or_hardlink_deduplication() {
            let f = Fixture::new();
            let outside = Fixture::new();
            fs::write(outside.0.join("secret"), [0; 100]).unwrap();
            fs::write(f.0.join("file"), [0; 7]).unwrap();
            fs::hard_link(f.0.join("file"), f.0.join("alias")).unwrap();
            symlink(&outside.0, f.0.join("external")).unwrap();
            symlink(&f.0, f.0.join("cycle")).unwrap();
            let report = defaults(&f.0);
            assert!(report.complete);
            assert_eq!(report.total_bytes, 14);
            for node in &report.nodes {
                if matches!(node.kind, NodeKind::Symlink) {
                    assert_eq!(node.bytes, Some(0));
                    assert_eq!(node.files, 0);
                }
            }
            assert!(scan(&f.0.join("external"), Limits::default()).is_err());
            let trailing = f.0.join("external").join("");
            assert!(scan(&trailing, Limits::default()).is_err());
        }
        #[test]
        fn empty_directory_is_complete_but_zero_time_is_not() {
            let f = Fixture::new();
            let report = defaults(&f.0);
            assert!(report.complete && report.nodes.is_empty());
            assert_eq!(report.omitted_children, Some(0));
            let report = scan(
                &f.0,
                Limits {
                    elapsed: Duration::ZERO,
                    ..Limits::default()
                },
            )
            .unwrap();
            assert!(!report.complete);
            assert!(report.reason.is_some());
            assert_eq!(report.omitted_children, None);
        }
        #[test]
        fn depth_and_visit_caps_never_promote_partial_bytes_to_complete() {
            let f = Fixture::new();
            fs::create_dir(f.0.join("child")).unwrap();
            fs::create_dir(f.0.join("child/deep")).unwrap();
            fs::write(f.0.join("child/deep/data"), [0; 20]).unwrap();
            for limits in [
                Limits {
                    depth: 1,
                    ..Limits::default()
                },
                Limits {
                    visited: 1,
                    ..Limits::default()
                },
            ] {
                let report = scan(&f.0, limits).unwrap();
                assert!(!report.complete);
                assert!(report.reason.is_some());
                assert_eq!(report.nodes.len(), 1);
                assert_eq!(report.nodes[0].bytes, None);
                assert!(matches!(report.nodes[0].status, NodeStatus::Partial));
                assert_eq!(report.total_bytes, 0);
                assert_eq!(report.omitted_children, Some(0));
            }
        }
        #[test]
        fn large_subtree_cannot_hide_later_siblings_or_consume_their_whole_budget() {
            let f = Fixture::new();
            fs::create_dir(f.0.join("large")).unwrap();
            for index in 0..20 {
                fs::write(f.0.join("large").join(index.to_string()), [1u8]).unwrap();
            }
            fs::create_dir(f.0.join("small")).unwrap();
            fs::write(f.0.join("small/data"), [2u8; 3]).unwrap();
            fs::write(f.0.join("direct"), [3u8; 7]).unwrap();
            let report = scan(
                &f.0,
                Limits {
                    visited: 8,
                    ..Limits::default()
                },
            )
            .unwrap();
            files_core::space_map::validate_report(&report).unwrap();
            assert!(!report.complete);
            assert_eq!(report.nodes.len(), 3);
            assert_eq!(report.omitted_children, Some(0));
            assert!(report.visited <= 8);
            let child = |name: &[u8]| report.nodes.iter().find(|n| n.name == name).unwrap();
            assert_eq!(child(b"small").bytes, Some(3));
            assert_eq!(child(b"direct").bytes, Some(7));
            assert_eq!(child(b"large").status, NodeStatus::Partial);
            assert_eq!(child(b"large").bytes, None);
            assert!(child(b"large").observed_bytes > 0);
        }
        #[test]
        fn subtree_stopped_by_its_share_is_completed_when_budget_remains() {
            let f = Fixture::new();
            fs::create_dir(f.0.join("large")).unwrap();
            for index in 0..20 {
                fs::write(f.0.join("large").join(index.to_string()), [1u8]).unwrap();
            }
            for name in ["a", "b", "c", "d"] {
                fs::create_dir(f.0.join(name)).unwrap();
                fs::write(f.0.join(name).join("data"), [2u8; 3]).unwrap();
            }
            // Any enumeration order gives "large" at most 17 visits in the
            // first pass; the retry has room for all 20 of its entries.
            let report = scan(
                &f.0,
                Limits {
                    visited: 60,
                    ..Limits::default()
                },
            )
            .unwrap();
            files_core::space_map::validate_report(&report).unwrap();
            assert!(report.complete, "{:?}", report.reason);
            let child = |name: &[u8]| report.nodes.iter().find(|n| n.name == name).unwrap();
            assert_eq!(child(b"large").status, NodeStatus::Complete);
            assert_eq!(child(b"large").bytes, Some(20));
            assert_eq!(child(b"large").files, 20);
            assert_eq!(report.total_bytes, 32);
        }
        #[test]
        fn parallel_walk_counts_wide_and_deep_trees_exactly() {
            let f = Fixture::new();
            let mut expected = 0u64;
            let mut entries = 1u64; // "tree" itself
            for a in 0..12 {
                let first = f.0.join("tree").join(a.to_string());
                for b in 0..6 {
                    let second = first.join(b.to_string());
                    fs::create_dir_all(&second).unwrap();
                    for c in 0..5 {
                        fs::write(second.join(c.to_string()), vec![1u8; a + b + c]).unwrap();
                        expected += (a + b + c) as u64;
                    }
                    entries += 1 + 5;
                }
                entries += 1;
            }
            fs::create_dir_all(f.0.join("tree/deep/1/2/3/4/5/6/7/8")).unwrap();
            fs::write(f.0.join("tree/deep/1/2/3/4/5/6/7/8/end"), [0u8; 9]).unwrap();
            expected += 9;
            entries += 10;
            for _ in 0..20 {
                let report = defaults(&f.0);
                assert!(report.complete, "{:?}", report.reason);
                assert_eq!(report.total_bytes, expected);
                assert_eq!(report.visited, entries);
                assert_eq!(report.nodes[0].files, 12 * 6 * 5 + 1);
            }
            // The exact visit cap holds with several workers: a cap one short
            // of the population is never reported complete.
            let report = scan(
                &f.0,
                Limits {
                    visited: entries - 1,
                    ..Limits::default()
                },
            )
            .unwrap();
            assert!(!report.complete);
            assert!(report.visited < entries);
            assert_eq!(report.nodes[0].bytes, None);
        }
        #[test]
        fn unreadable_subfolder_keeps_measuring_its_siblings() {
            use std::os::unix::fs::PermissionsExt;
            if unsafe { libc::geteuid() } == 0 {
                return; // root reads mode 000 folders; nothing to observe
            }
            let f = Fixture::new();
            fs::create_dir_all(f.0.join("work/locked")).unwrap();
            fs::write(f.0.join("work/locked/hidden"), [0u8; 50]).unwrap();
            for name in ["a", "b", "c"] {
                fs::create_dir(f.0.join("work").join(name)).unwrap();
                fs::write(f.0.join("work").join(name).join("data"), [0u8; 10]).unwrap();
            }
            fs::write(f.0.join("work/top"), [0u8; 5]).unwrap();
            let locked = f.0.join("work/locked");
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
            let report = defaults(&f.0);
            // Restore the mode so the retained fixture stays inspectable.
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();
            let work = &report.nodes[0];
            assert!(!report.complete);
            assert_eq!(work.status, NodeStatus::Partial);
            assert_eq!(work.bytes, None);
            assert_eq!(work.observed_bytes, 35, "every readable sibling counted");
            assert!(
                work.error
                    .as_deref()
                    .is_some_and(|e| e.contains("subcarpeta"))
            );
        }
        #[test]
        fn name_and_node_caps_stop_without_claiming_zero_omissions() {
            let f = Fixture::new();
            fs::write(f.0.join("one"), [0; 1]).unwrap();
            fs::write(f.0.join("two"), [0; 2]).unwrap();
            for limits in [
                Limits {
                    nodes: 1,
                    ..Limits::default()
                },
                Limits {
                    names: 2,
                    ..Limits::default()
                },
            ] {
                let report = scan(&f.0, limits).unwrap();
                assert!(!report.complete);
                assert!(report.reason.is_some());
                assert_eq!(report.omitted_children, None);
                assert!(report.nodes.len() <= 1);
            }
        }
        #[test]
        fn stat_failure_and_directory_replacement_are_errors_not_measured_totals() {
            let f = Fixture::new();
            fs::create_dir(f.0.join("child")).unwrap();
            let root = Stream::from_file(fs::File::open(&f.0).unwrap()).unwrap();
            let before = stat_at(root.fd(), c"child").unwrap();
            fs::rename(f.0.join("child"), f.0.join("moved-original")).unwrap();
            fs::create_dir(f.0.join("child")).unwrap();
            assert!(child_directory(root.fd(), c"child", &before).is_err());
            let mut budget = Budget::new(Limits::default());
            let (node, directory, issue) = inspect_entry(-1, c"fixture-missing");
            assert!(directory.is_none());
            assert!(matches!(node.status, NodeStatus::Error));
            assert!(node.bytes.is_none());
            assert_eq!(node.observed_bytes, 0);
            let issue = issue.unwrap();
            assert!(budget.error(&issue).is_some());
            fs::write(f.0.join("regular"), b"file").unwrap();
            assert!(scan(&f.0.join("regular"), Limits::default()).is_err());
        }
        #[test]
        fn errors_are_bounded_before_formatting() {
            let mut budget = Budget::new(Limits {
                errors: MAX_ERROR_BYTES * 2,
                ..Limits::default()
            });
            let issue = Issue::io("Error", io::Error::other("漢".repeat(5000)));
            let message = budget.error(&issue).unwrap();
            assert!(message.len() <= MAX_ERROR_BYTES);
            assert!(budget.error(&issue).is_none());
        }
    }
}
#[cfg(not(target_os = "linux"))]
mod platform {
    use super::*;
    pub(super) fn scan(_: &Path, _: Limits) -> Result<SpaceReport, String> {
        Err("El recorrido de Espacio por descriptores está pendiente en esta plataforma".into())
    }
}
