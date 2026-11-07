//! Viewport-prioritized automatic measurements and provisional persisted observations.
use crate::{
    aux_io::Request,
    model::{Model, View},
    size_queue::{MAX_ROWS, Status},
};
use files_core::{
    folder_size_cache::{Checked, Completed, Observation, Op, Stamp, Updated},
    folder_usage::Mode,
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};
const MAX_VISIBLE: usize = 64;
const SETTLE: Duration = Duration::from_millis(300);
const SAVE_EVERY: Duration = Duration::from_secs(2);
const REUSE_FOR: Duration = Duration::from_secs(5 * 60);
const MAX_MEMORY: usize = 256;
const MAX_MEMORY_PATH_BYTES: usize = 1024 * 1024;
#[derive(Clone)]
struct Remembered {
    path: PathBuf,
    stamp: Stamp,
    value: Observation,
}
fn reusable(value: Observation, now: SystemTime) -> bool {
    now.duration_since(value.at)
        .is_ok_and(|age| age < REUSE_FOR)
}
fn background_location(
    path: &Path,
    mounts: &[files_core::places::Mount],
    descendants: bool,
) -> bool {
    let excluded = |fs: &str| {
        fs.starts_with("fuse")
            || matches!(
                fs,
                "nfs"
                    | "nfs4"
                    | "cifs"
                    | "smbfs"
                    | "9p"
                    | "ceph"
                    | "glusterfs"
                    | "afs"
                    | "davfs"
                    | "autofs"
                    | "proc"
                    | "sysfs"
                    | "devtmpfs"
                    | "devpts"
                    | "cgroup"
                    | "cgroup2"
                    | "mqueue"
                    | "bpf"
                    | "tracefs"
                    | "debugfs"
                    | "securityfs"
                    | "pstore"
                    | "configfs"
            )
    };
    // Mount observations are a conservative scheduling hint, not permission to
    // traverse a mount. Explicit size requests retain their existing behavior.
    !mounts.iter().any(|mount| {
        excluded(&mount.fs)
            && (path.starts_with(&mount.path) || descendants && mount.path.starts_with(path))
    })
}
enum PendingKind {
    Read(Vec<OsString>),
    Save,
    Invalidate(Vec<PathBuf>),
}
struct Pending {
    id: u64,
    epoch: u64,
    kind: PendingKind,
}
pub struct State {
    epoch: u64,
    pending: Option<Pending>,
    next: u64,
    ready_at: Instant,
    last_save: Option<Instant>,
    checked: HashSet<OsString>,
    stamps: HashMap<OsString, Stamp>,
    proof_ids: HashMap<OsString, u64>,
    pub(crate) cached: HashMap<OsString, Observation>,
    automatic: HashSet<u64>,
    pub(crate) partials: HashMap<OsString, (u64, u64)>,
    published: HashSet<u64>,
    yielded: HashSet<OsString>,
    refresh_after_ack: HashSet<u64>,
    warning: Option<String>,
    cached_hits: usize,
    memory: VecDeque<Remembered>,
    previous_visible: HashSet<OsString>,
    probed_at: HashMap<OsString, Instant>,
    touched: HashMap<u64, u64>,
    touch_clock: u64,
    invalidated: HashMap<PathBuf, SystemTime>,
    invalidate_pending: HashSet<PathBuf>,
    disk_reuse_disabled: bool,
    dispatches: VecDeque<(PathBuf, u64)>,
    dispatch_total: u64,
}
impl Default for State {
    fn default() -> Self {
        Self {
            epoch: u64::MAX,
            pending: None,
            next: 1,
            ready_at: Instant::now() + SETTLE,
            last_save: None,
            checked: HashSet::new(),
            stamps: HashMap::new(),
            proof_ids: HashMap::new(),
            cached: HashMap::new(),
            automatic: HashSet::new(),
            partials: HashMap::new(),
            published: HashSet::new(),
            yielded: HashSet::new(),
            refresh_after_ack: HashSet::new(),
            warning: None,
            cached_hits: 0,
            memory: VecDeque::new(),
            previous_visible: HashSet::new(),
            probed_at: HashMap::new(),
            touched: HashMap::new(),
            touch_clock: 0,
            invalidated: HashMap::new(),
            invalidate_pending: HashSet::new(),
            disk_reuse_disabled: false,
            dispatches: VecDeque::new(),
            dispatch_total: 0,
        }
    }
}
impl State {
    pub(crate) fn owns(&self, id: u64) -> bool {
        self.automatic.contains(&id)
    }
    pub(crate) fn cached_hits(&self) -> usize {
        self.cached_hits
    }
    pub(crate) fn warning(&self) -> Option<&str> {
        self.warning.as_deref()
    }
    fn synchronize(&mut self, epoch: u64) {
        if self.epoch != epoch {
            self.epoch = epoch;
            self.ready_at = Instant::now() + SETTLE;
            self.checked.clear();
            self.stamps.clear();
            self.proof_ids.clear();
            self.cached.clear();
            self.automatic.clear();
            self.partials.clear();
            self.published.clear();
            self.yielded.clear();
            self.refresh_after_ack.clear();
            self.cached_hits = 0;
            self.previous_visible.clear();
            self.probed_at.clear();
            self.touched.clear();
        }
    }
    pub(crate) fn note_dispatch(&mut self, path: PathBuf, id: u64, mode: Mode) {
        // A proof already obtained before dispatch can also accompany an explicit
        // measurement. Never attach metadata that arrives after the walk starts.
        if mode == Mode::Apparent
            && let Some(name) = path.file_name()
            && self.stamps.contains_key(name)
        {
            self.proof_ids.insert(name.to_owned(), id);
        }
        self.dispatch_total = self.dispatch_total.saturating_add(1);
        let count = self
            .dispatches
            .iter()
            .position(|(p, _)| p == &path)
            .and_then(|i| self.dispatches.remove(i))
            .map_or(1, |(_, n)| n.saturating_add(1));
        self.dispatches.push_back((path, count));
        while self.dispatches.len() > MAX_MEMORY {
            self.dispatches.pop_front();
        }
    }
    pub(crate) fn dispatch_count(&self, path: &Path) -> Option<u64> {
        self.dispatches
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, n)| *n)
    }
    pub(crate) fn dispatch_total(&self) -> u64 {
        self.dispatch_total
    }
    fn rejected_observation(&self, path: &Path, value: Observation) -> bool {
        self.invalidated
            .iter()
            .any(|(root, at)| (path.starts_with(root) || root.starts_with(path)) && value.at <= *at)
    }
    fn remember(&mut self, value: Remembered) {
        if self.rejected_observation(&value.path, value.value) {
            return;
        }
        self.memory.retain(|r| r.path != value.path);
        self.memory.push_back(value);
        while self.memory.len() > MAX_MEMORY
            || self
                .memory
                .iter()
                .map(|r| r.path.as_os_str().as_encoded_bytes().len())
                .sum::<usize>()
                > MAX_MEMORY_PATH_BYTES
        {
            self.memory.pop_front();
        }
    }
    fn recalled(&mut self, path: &Path, stamp: Stamp, now: SystemTime) -> Option<Observation> {
        let i = self.memory.iter().position(|r| r.path == path)?;
        let row = self.memory.remove(i)?;
        if row.stamp != stamp
            || !reusable(row.value, now)
            || self.rejected_observation(path, row.value)
        {
            return None;
        }
        let value = row.value;
        self.memory.push_back(row);
        Some(value)
    }
    pub(crate) fn remember_completed(
        &mut self,
        root: &Path,
        name: &std::ffi::OsStr,
        id: u64,
        value: Observation,
    ) {
        if self.proof_ids.get(name) != Some(&id) {
            return;
        }
        if let Some(stamp) = self.stamps.get(name).copied() {
            self.remember(Remembered {
                path: root.join(name),
                stamp,
                value,
            });
        }
    }
    pub(crate) fn invalidate_path(&mut self, path: PathBuf, root: &Path) {
        let now = SystemTime::now();
        self.invalidated
            .retain(|_, at| now.duration_since(*at).is_ok_and(|age| age < REUSE_FOR));
        self.memory
            .retain(|r| !r.path.starts_with(&path) && !path.starts_with(&r.path));
        if self.invalidated.len() == MAX_MEMORY && !self.invalidated.contains_key(&path) {
            self.disk_reuse_disabled = true;
            self.memory.clear();
            if let Some(old) = self.invalidated.keys().next().cloned() {
                self.invalidated.remove(&old);
            }
        }
        self.invalidated.insert(path.clone(), now);
        if self.invalidate_pending.len() < MAX_MEMORY {
            self.invalidate_pending.insert(path.clone());
        } else {
            self.disk_reuse_disabled = true;
        }
        self.cached.retain(|name, _| {
            !root.join(name).starts_with(&path) && !path.starts_with(root.join(name))
        });
        self.checked.retain(|name| {
            !root.join(name).starts_with(&path) && !path.starts_with(root.join(name))
        });
        self.stamps.retain(|name, _| {
            !root.join(name).starts_with(&path) && !path.starts_with(root.join(name))
        });
        self.proof_ids.retain(|name, _| {
            !root.join(name).starts_with(&path) && !path.starts_with(root.join(name))
        });
        self.probed_at.retain(|name, _| {
            !root.join(name).starts_with(&path) && !path.starts_with(root.join(name))
        });
    }
    fn admit_reused(&mut self, queue: &mut crate::size_queue::State, names: &[OsString]) -> bool {
        let root = queue.root().to_owned();
        let mut dirty = false;
        let now = SystemTime::now();
        for name in names {
            if queue.status(&root, name).is_some() {
                continue;
            }
            let Some(stamp) = self.stamps.get(name).copied() else {
                continue;
            };
            if let Some(value) = self.recalled(&root.join(name), stamp, now)
                && let Some(id) = queue.restore_automatic_observation(&root, name, value)
            {
                self.automatic.insert(id);
                self.proof_ids.insert(name.clone(), id);
                self.published.insert(id);
                dirty = true;
            }
        }
        dirty
    }
    fn reject_probe(
        &mut self,
        queue: &mut crate::size_queue::State,
        root: &Path,
        name: &std::ffi::OsStr,
    ) -> bool {
        self.memory.retain(|r| r.path != root.join(name));
        self.cached.remove(name);
        self.stamps.remove(name);
        self.proof_ids.remove(name);
        let record = queue
            .rows(root)
            .iter()
            .find(|r| r.name == name && self.automatic.contains(&r.id))
            .map(|r| r.id);
        if let Some(id) = record {
            if queue.active_id() == Some(id) {
                self.refresh_after_ack.insert(id);
                return queue.cancel_id(queue.epoch(), id);
            }
            queue.forget_automatic(id);
            self.automatic.remove(&id);
            self.published.remove(&id);
            self.touched.remove(&id);
        }
        false
    }
    fn process_checked(
        &mut self,
        queue: &mut crate::size_queue::State,
        root: &Path,
        row: Checked,
    ) -> bool {
        let Checked {
            name,
            stamp,
            cached,
        } = row;
        let path = root.join(&name);
        let now = SystemTime::now();
        let remembered = self.recalled(&path, stamp, now);
        let existing = queue
            .rows(root)
            .iter()
            .find(|r| {
                r.name == name
                    && self.automatic.contains(&r.id)
                    && matches!(r.status, Status::Measured { .. })
            })
            .map(|r| r.id);
        let mut dirty = false;
        if let Some(id) = existing
            && remembered.is_none()
            && queue.forget_automatic(id)
        {
            self.automatic.remove(&id);
            self.published.remove(&id);
            self.proof_ids.remove(&name);
            self.touched.remove(&id);
            dirty = true;
        }
        self.stamps.insert(name.clone(), stamp);
        self.probed_at.insert(name.clone(), Instant::now());
        self.cached.remove(&name);
        if let Some(value) = cached
            && !self.disk_reuse_disabled
            && !self.rejected_observation(&path, value)
        {
            self.cached_hits = self.cached_hits.saturating_add(1);
            self.cached.insert(name, value);
        }
        dirty
    }
    fn allocate(&mut self) -> u64 {
        let id = self.next;
        self.next = self
            .next
            .checked_add(1)
            .expect("automatic size request id exhausted");
        id
    }
    fn submit(&mut self, m: &Model, op: Op, kind: PendingKind) -> bool {
        if self.pending.is_some() {
            return false;
        }
        let id = self.allocate();
        if m.aux
            .tx
            .try_send(Request::FolderSizes(id, Arc::new(op)))
            .is_err()
        {
            return false;
        }
        self.pending = Some(Pending {
            id,
            epoch: m.sizes.epoch(),
            kind,
        });
        true
    }
    pub(crate) fn mark_refresh_after_ack(&mut self, id: u64) {
        self.refresh_after_ack.insert(id);
    }
    pub(crate) fn clear_yielded(&mut self) {
        self.yielded.clear();
        self.refresh_after_ack.clear();
    }
    pub(crate) fn mark_yielded(&mut self, rows: &[crate::size_queue::Record]) {
        self.yielded.extend(
            rows.iter()
                .filter(|row| {
                    self.automatic.contains(&row.id)
                        && matches!(row.status, Status::Queued | Status::Running { .. })
                })
                .map(|row| row.name.clone()),
        );
    }
    fn resume_yielded(
        &mut self,
        queue: &mut crate::size_queue::State,
        names: &[OsString],
        helper_drained: bool,
    ) -> bool {
        if !helper_drained {
            return false;
        }
        let mut dirty = false;
        for name in names {
            if !self.yielded.contains(name) {
                continue;
            }
            if let Some((id, true)) = queue
                .rows(queue.root())
                .iter()
                .find(|row| &row.name == name)
                .map(|row| (row.id, matches!(row.status, Status::Cancelled)))
                && let Some(new) = queue.retry_id(queue.epoch(), id)
            {
                self.automatic.remove(&id);
                self.automatic.insert(new);
                self.proof_ids.insert(name.clone(), new);
                self.partials.remove(name);
                dirty = true;
            }
            self.yielded.remove(name);
        }
        dirty
    }
    fn reconcile_viewport(
        &mut self,
        queue: &mut crate::size_queue::State,
        names: &[OsString],
    ) -> (bool, bool) {
        let root = queue.root().to_owned();
        let mut dirty = false;
        let mut cancel = false;
        self.touch_clock = self.touch_clock.saturating_add(1);
        for row in queue.rows(&root).iter().filter(|r| names.contains(&r.name)) {
            self.touched.insert(row.id, self.touch_clock);
        }
        let refresh: Vec<_> = self
            .refresh_after_ack
            .iter()
            .copied()
            .filter(|id| queue.active_id() != Some(*id))
            .collect();
        for id in refresh {
            queue.forget_automatic(id);
            self.automatic.remove(&id);
            self.proof_ids.retain(|_, record| *record != id);
            self.published.remove(&id);
            self.refresh_after_ack.remove(&id);
            dirty = true;
        }
        // Leaving the viewport only cancels unfinished background work. Retain
        // completed rows until bounded LRU pressure actually needs their slot.
        let obsolete: Vec<_> = queue
            .rows(&root)
            .iter()
            .filter(|r| {
                self.automatic.contains(&r.id)
                    && !names.contains(&r.name)
                    && !matches!(r.status, Status::Measured { .. })
            })
            .map(|r| (r.id, r.name.clone()))
            .collect();
        for (id, name) in obsolete {
            if queue.active_id() == Some(id) {
                if queue.cancel_id(queue.epoch(), id) {
                    cancel = true;
                    dirty = true;
                }
            } else if queue.forget_automatic(id) {
                self.automatic.remove(&id);
                self.published.remove(&id);
                self.partials.remove(&name);
                self.proof_ids.remove(&name);
                self.yielded.remove(&name);
                self.touched.remove(&id);
                dirty = true;
            }
        }
        let needed = names
            .iter()
            .filter(|name| queue.status(&root, name).is_none())
            .count();
        while queue.rows(&root).len().saturating_add(needed) > MAX_ROWS {
            let victim = queue
                .rows(&root)
                .iter()
                .filter(|r| {
                    self.automatic.contains(&r.id)
                        && !names.contains(&r.name)
                        && matches!(r.status, Status::Measured { .. })
                })
                .min_by_key(|r| self.touched.get(&r.id).copied().unwrap_or(0))
                .map(|r| r.id);
            let Some(id) = victim else {
                break;
            };
            queue.forget_automatic(id);
            self.automatic.remove(&id);
            self.published.remove(&id);
            self.touched.remove(&id);
            dirty = true;
        }
        self.checked
            .retain(|n| names.contains(n) || queue.rows(&root).iter().any(|r| &r.name == n));
        self.stamps.retain(|n, _| self.checked.contains(n));
        self.cached.retain(|n, _| self.checked.contains(n));
        self.partials.retain(|n, _| self.checked.contains(n));
        self.proof_ids.retain(|n, _| self.checked.contains(n));
        self.yielded.retain(|n| self.checked.contains(n));
        self.probed_at.retain(|n, _| self.checked.contains(n));
        self.touched
            .retain(|id, _| queue.rows(&root).iter().any(|r| r.id == *id));
        (dirty, cancel)
    }
    fn needs_measurement(&self, root: &Path, name: &std::ffi::OsStr, now: SystemTime) -> bool {
        self.stamps.contains_key(name)
            && !self.cached.get(name).is_some_and(|value| {
                reusable(*value, now) && !self.rejected_observation(&root.join(name), *value)
            })
    }
    fn pump(&mut self, m: &mut Model) -> bool {
        m.sizes.bind_automatic_root(&m.path);
        self.synchronize(m.sizes.epoch());
        self.automatic
            .retain(|id| m.sizes.rows(&m.path).iter().any(|row| row.id == *id));
        self.published
            .retain(|id| m.sizes.rows(&m.path).iter().any(|row| row.id == *id));
        let allowed = m.auto_folder_sizes
            && m.loaded
            && background_location(&m.path, &m.mounts, false)
            && m.media_visible
            && !m.closing
            && m.visible_generation == m.generation
            && !m.editing
            && !m.filter_edit.active
            && !m.settings
            && m.help.is_none()
            && m.commands.is_none()
            && m.size_panel.is_none()
            && m.rename_dialog.is_none()
            && m.properties_dialog.is_none()
            && m.copy_form.is_none()
            && !m.copy_busy()
            && !m.mutations.busy
            && !m.mutation_applied
            && !m.search.busy()
            && !m.preview.visible()
            && m.preview.is_drained()
            && m.view != View::SpaceMap
            && m.space_map.is_drained()
            && m.comparison.is_none()
            && m.checksums.is_none()
            && m.trash_browser.is_none();
        let mut dirty = false;
        if !allowed {
            if let Some(id) = m.sizes.active_id()
                && self.automatic.contains(&id)
            {
                let name = m
                    .sizes
                    .rows(&m.path)
                    .iter()
                    .find(|r| r.id == id)
                    .map(|r| r.name.clone());
                if m.sizes.cancel_id(m.sizes.epoch(), id) {
                    if let Some(job) = &m.du_job {
                        job.cancel();
                    }
                    if let Some(name) = name {
                        self.yielded.insert(name);
                    }
                    dirty = true;
                }
            }
            // Queued background work must not block foreground helper admission.
            let ids: Vec<_> = m
                .sizes
                .rows(&m.path)
                .iter()
                .filter(|r| self.automatic.contains(&r.id) && matches!(r.status, Status::Queued))
                .map(|r| (r.id, r.name.clone()))
                .collect();
            for (id, name) in ids {
                m.sizes.cancel_id(m.sizes.epoch(), id);
                self.yielded.insert(name);
                dirty = true;
            }
            return dirty;
        }
        if Instant::now() < self.ready_at {
            return false;
        }
        let mut names = Vec::with_capacity(MAX_VISIBLE);
        for &index in &m.visible {
            if let Some(e) = m.entries.get(index)
                && e.is_dir()
                && background_location(&m.path.join(&e.name), &m.mounts, true)
                && !names.contains(&e.name)
            {
                names.push(e.name.clone());
                if names.len() == MAX_VISIBLE {
                    break;
                }
            }
        }
        let (viewport_dirty, cancel) = self.reconcile_viewport(&mut m.sizes, &names);
        dirty |= viewport_dirty;
        if cancel && let Some(job) = &m.du_job {
            job.cancel();
        }
        // Re-entry performs only an identity/mtime probe. A fresh unchanged
        // observation is reused; recursive work is not a viewport side effect.
        for name in &names {
            if !self.previous_visible.contains(name)
                || self
                    .probed_at
                    .get(name)
                    .is_some_and(|at| at.elapsed() >= REUSE_FOR)
            {
                self.checked.remove(name);
                self.stamps.remove(name);
            }
        }
        self.previous_visible = names.iter().cloned().collect();
        if self.pending.is_none() && !self.invalidate_pending.is_empty() {
            let roots: Vec<_> = self
                .invalidate_pending
                .iter()
                .take(MAX_VISIBLE)
                .cloned()
                .collect();
            self.submit(
                m,
                Op::Invalidate {
                    roots: roots.clone(),
                },
                PendingKind::Invalidate(roots),
            );
        }
        // Persist completed observations in one bounded write, not one fsync per tile.
        if self.pending.is_none()
            && self
                .last_save
                .is_none_or(|last| last.elapsed() >= SAVE_EVERY)
        {
            let completed: Vec<_> = m
                .sizes
                .rows(&m.path)
                .iter()
                .filter_map(|r| {
                    if r.mode != Mode::Apparent
                        || self.published.contains(&r.id)
                        || self.proof_ids.get(&r.name) != Some(&r.id)
                    {
                        return None;
                    }
                    let Status::Measured { bytes, entries, at } = r.status else {
                        return None;
                    };
                    Some((
                        r.id,
                        Completed {
                            name: r.name.clone(),
                            stamp: *self.stamps.get(&r.name)?,
                            value: Observation { bytes, entries, at },
                        },
                    ))
                })
                .take(MAX_VISIBLE)
                .collect();
            if !completed.is_empty()
                && self.submit(
                    m,
                    Op::Save {
                        root: m.path.clone(),
                        rows: completed.iter().map(|(_, r)| r.clone()).collect(),
                    },
                    PendingKind::Save,
                )
            {
                self.published.extend(completed.iter().map(|(id, _)| *id));
                self.last_save = Some(Instant::now());
            }
        }
        dirty |= self.resume_yielded(&mut m.sizes, &names, m.du_job.is_none());
        dirty |= self.admit_reused(&mut m.sizes, &names);
        for name in &names {
            if self.needs_measurement(&m.path, name, SystemTime::now())
                && m.sizes.status(&m.path, name).is_none()
                && m.sizes.rows(&m.path).len() < MAX_ROWS
                && m.sizes
                    .enqueue(&m.path, std::iter::once(name.as_os_str()))
                    .is_ok()
            {
                if let Some(row) = m.sizes.rows(&m.path).iter().find(|r| &r.name == name) {
                    self.automatic.insert(row.id);
                    self.proof_ids.insert(name.clone(), row.id);
                }
                dirty = true;
            }
        }
        if self.pending.is_none() {
            let missing: Vec<_> = names
                .into_iter()
                .filter(|n| !self.checked.contains(n))
                .take(MAX_VISIBLE)
                .collect();
            if !missing.is_empty() && self.checked.len() + missing.len() <= MAX_MEMORY {
                self.submit(
                    m,
                    Op::Read {
                        root: m.path.clone(),
                        names: missing.clone(),
                    },
                    PendingKind::Read(missing),
                );
            }
        }
        dirty
    }
    fn response(&mut self, m: &mut Model, id: u64, result: Result<Updated, String>) -> bool {
        if !self.pending.as_ref().is_some_and(|p| p.id == id) {
            return false;
        }
        let pending = self.pending.take().expect("matched request");
        if pending.epoch != m.sizes.epoch() {
            return false;
        }
        match (pending.kind, result) {
            (PendingKind::Read(names), Ok(Updated::Read(rows, warning))) => {
                self.checked.extend(names.iter().cloned());
                self.warning = warning;
                let root = m.path.clone();
                let found: HashSet<_> = rows.iter().map(|row| row.name.clone()).collect();
                for name in names.iter().filter(|name| !found.contains(*name)) {
                    if self.reject_probe(&mut m.sizes, &root, name)
                        && let Some(job) = &m.du_job
                    {
                        job.cancel();
                    }
                }
                for row in rows {
                    self.process_checked(&mut m.sizes, &root, row);
                }
                true
            }
            (PendingKind::Read(names), Err(error)) => {
                self.checked.extend(names.iter().cloned());
                self.warning = Some(error);
                let root = m.path.clone();
                for name in names {
                    if self.reject_probe(&mut m.sizes, &root, &name)
                        && let Some(job) = &m.du_job
                    {
                        job.cancel();
                    }
                }
                true
            }
            (PendingKind::Invalidate(roots), Ok(Updated::Invalidated)) => {
                for root in roots {
                    self.invalidate_pending.remove(&root);
                }
                false
            }
            (PendingKind::Invalidate(roots), Err(error)) => {
                for root in roots {
                    self.invalidate_pending.remove(&root);
                }
                self.warning = Some(error);
                false
            }
            (PendingKind::Save, Ok(Updated::Saved)) => {
                self.warning = None;
                false
            }
            (_, Err(error)) => {
                self.warning = Some(error);
                false
            }
            _ => {
                self.warning = Some("Unexpected folder-size cache response".into());
                false
            }
        }
    }
}
impl Model {
    /// Call once after size-worker ACK handling and before helper admission.
    pub fn pump_automatic_sizes(&mut self) -> bool {
        let mut state = std::mem::take(&mut self.sizes.automatic);
        let dirty = state.pump(self);
        self.sizes.automatic = state;
        dirty
    }
    pub fn automatic_sizes_response(&mut self, id: u64, result: Result<Updated, String>) -> bool {
        let mut state = std::mem::take(&mut self.sizes.automatic);
        let dirty = state.response(self, id, result);
        self.sizes.automatic = state;
        dirty
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{ffi::OsStr, path::Path};
    fn stamp() -> Stamp {
        Stamp {
            device: 1,
            inode: 2,
            modified_seconds: 3,
            modified_nanos: 4,
        }
    }
    fn measured_only() -> crate::size_queue::State {
        let mut queue = crate::size_queue::State::default();
        let root = Path::new("/private/browser");
        queue
            .enqueue(root, [OsStr::new("automatic")].into_iter())
            .unwrap();
        queue.automatic.synchronize(queue.epoch());
        let id = queue.rows(root)[0].id;
        queue.automatic.automatic.insert(id);
        queue.automatic.stamps.insert("automatic".into(), stamp());
        queue.dispatch_work().unwrap();
        assert!(
            queue.finish_measurement(Ok(files_core::folder_usage::Measurement::apparent(42, 2)))
        );
        queue
    }
    #[test]
    fn scrolling_away_and_back_keeps_a_completed_measurement_without_dispatch() {
        let mut queue = measured_only();
        let id = queue.rows(queue.root())[0].id;
        let mut automatic = std::mem::take(&mut queue.automatic);
        assert_eq!(
            automatic.reconcile_viewport(&mut queue, &[]),
            (false, false)
        );
        assert_eq!(
            automatic.reconcile_viewport(&mut queue, &["automatic".into()]),
            (false, false)
        );
        let root = queue.root().to_owned();
        automatic.process_checked(
            &mut queue,
            &root,
            Checked {
                name: "automatic".into(),
                stamp: stamp(),
                cached: None,
            },
        );
        assert_eq!(queue.rows(&root)[0].id, id);
        assert!(queue.dispatch_work().is_none());
        assert_eq!(automatic.dispatch_count(&root.join("automatic")), Some(1));
    }
    #[test]
    fn directory_navigation_reuses_memory_only_after_matching_identity_probe() {
        let mut queue = measured_only();
        let root = queue.root().to_owned();
        queue.invalidate();
        queue.bind_automatic_root(Path::new("/private/other"));
        let mut automatic = std::mem::take(&mut queue.automatic);
        automatic.synchronize(queue.epoch());
        queue.bind_automatic_root(&root);
        automatic.synchronize(queue.epoch());
        assert!(!automatic.admit_reused(&mut queue, &["automatic".into()]));
        automatic.process_checked(
            &mut queue,
            &root,
            Checked {
                name: "automatic".into(),
                stamp: stamp(),
                cached: None,
            },
        );
        assert!(automatic.admit_reused(&mut queue, &["automatic".into()]));
        assert_eq!(queue.measured(&root, OsStr::new("automatic")), Some(42));
        assert!(queue.dispatch_work().is_none());
        assert_eq!(automatic.dispatch_count(&root.join("automatic")), Some(1));
    }
    #[test]
    fn replaced_identity_or_expired_observation_requires_a_new_measurement() {
        let mut queue = measured_only();
        let root = queue.root().to_owned();
        let mut automatic = std::mem::take(&mut queue.automatic);
        let mut changed = stamp();
        changed.inode += 1;
        assert!(automatic.process_checked(
            &mut queue,
            &root,
            Checked {
                name: "automatic".into(),
                stamp: changed,
                cached: None
            }
        ));
        assert!(!automatic.admit_reused(&mut queue, &["automatic".into()]));
        assert!(automatic.needs_measurement(&root, OsStr::new("automatic"), SystemTime::now()));
        automatic.remember(Remembered {
            path: root.join("automatic"),
            stamp: changed,
            value: Observation {
                bytes: 9,
                entries: 1,
                at: SystemTime::now() - REUSE_FOR - Duration::from_secs(1),
            },
        });
        assert!(
            automatic
                .recalled(&root.join("automatic"), changed, SystemTime::now())
                .is_none()
        );
        automatic.remember(Remembered {
            path: root.join("automatic"),
            stamp: changed,
            value: Observation {
                bytes: 12,
                entries: 1,
                at: SystemTime::now(),
            },
        });
        let mut modified = changed;
        modified.modified_seconds += 1;
        assert!(
            automatic
                .recalled(&root.join("automatic"), modified, SystemTime::now())
                .is_none()
        );
    }
    #[test]
    fn a_missing_or_unreadable_probe_cannot_keep_an_exact_old_size() {
        let mut queue = measured_only();
        let root = queue.root().to_owned();
        let mut automatic = std::mem::take(&mut queue.automatic);
        assert!(!automatic.reject_probe(&mut queue, &root, OsStr::new("automatic")));
        assert!(queue.rows(&root).is_empty());
        assert!(automatic.memory.is_empty());
        assert!(automatic.cached.is_empty());
        assert!(!automatic.needs_measurement(&root, OsStr::new("automatic"), SystemTime::now()));
    }
    #[test]
    fn fresh_persisted_values_are_provisional_without_starting_a_recursive_walk() {
        let mut queue = crate::size_queue::State::default();
        let root = Path::new("/private/browser");
        queue.bind_automatic_root(root);
        let mut automatic = std::mem::take(&mut queue.automatic);
        automatic.synchronize(queue.epoch());
        let at = SystemTime::now();
        automatic.process_checked(
            &mut queue,
            root,
            Checked {
                name: "automatic".into(),
                stamp: stamp(),
                cached: Some(Observation {
                    bytes: 42,
                    entries: 2,
                    at,
                }),
            },
        );
        assert!(!automatic.needs_measurement(root, OsStr::new("automatic"), at));
        assert!(automatic.needs_measurement(root, OsStr::new("automatic"), at + REUSE_FOR));
        queue.automatic = automatic;
        assert_eq!(
            queue.label(root, OsStr::new("automatic")).as_deref(),
            Some("≈ 42 B")
        );
        assert!(queue.measured_map(root).is_empty());
        assert!(queue.dispatch_work().is_none());
        assert_eq!(queue.dispatch_count_total(), 0);
    }
    #[test]
    fn real_change_revokes_memory_and_rejects_an_older_pending_disk_hit() {
        let mut queue = measured_only();
        let root = queue.root().to_owned();
        let old = Observation {
            bytes: 42,
            entries: 2,
            at: SystemTime::now() - Duration::from_secs(1),
        };
        assert!(!queue.invalidate_automatic_name(&root, OsStr::new("automatic")));
        let mut automatic = std::mem::take(&mut queue.automatic);
        assert!(automatic.memory.is_empty());
        assert!(queue.rows(&root).is_empty());
        automatic.process_checked(
            &mut queue,
            &root,
            Checked {
                name: "automatic".into(),
                stamp: stamp(),
                cached: Some(old),
            },
        );
        assert!(automatic.cached.is_empty());
        assert!(!automatic.admit_reused(&mut queue, &["automatic".into()]));
        assert!(automatic.needs_measurement(&root, OsStr::new("automatic"), SystemTime::now()));
        assert!(
            automatic
                .invalidate_pending
                .contains(&root.join("automatic"))
        );
    }
    #[test]
    fn a_known_nested_change_revokes_ancestors_and_descendants_but_not_siblings() {
        let mut queue = measured_only();
        let root = queue.root().to_owned();
        let at = SystemTime::now();
        for path in [root.join("automatic/nested"), root.join("sibling")] {
            queue.automatic.remember(Remembered {
                path,
                stamp: stamp(),
                value: Observation {
                    bytes: 9,
                    entries: 1,
                    at,
                },
            });
        }
        queue.invalidate_automatic_root(&root.join("automatic/nested"));
        assert!(queue.rows(&root).is_empty());
        assert_eq!(queue.automatic.memory.len(), 1);
        assert_eq!(queue.automatic.memory[0].path, root.join("sibling"));
    }
    #[test]
    fn observation_and_dispatch_histories_remain_bounded_and_keep_recent_entries() {
        let mut automatic = State::default();
        let now = SystemTime::now();
        for n in 0..MAX_MEMORY + 1 {
            let path = PathBuf::from(format!("/private/{n}"));
            automatic.remember(Remembered {
                path: path.clone(),
                stamp: stamp(),
                value: Observation {
                    bytes: n as u64,
                    entries: 1,
                    at: now,
                },
            });
            automatic.note_dispatch(path, n as u64, Mode::Apparent);
        }
        assert_eq!(automatic.memory.len(), MAX_MEMORY);
        assert_eq!(automatic.dispatches.len(), MAX_MEMORY);
        assert_eq!(automatic.dispatch_count(Path::new("/private/0")), None);
        assert_eq!(
            automatic.dispatch_count(&PathBuf::from(format!("/private/{MAX_MEMORY}"))),
            Some(1)
        );
    }
    fn queue() -> crate::size_queue::State {
        let mut queue = crate::size_queue::State::default();
        queue
            .enqueue(
                Path::new("/private/browser"),
                [OsStr::new("automatic"), OsStr::new("manual")].into_iter(),
            )
            .unwrap();
        let id = queue.rows(queue.root())[0].id;
        queue.automatic.automatic.insert(id);
        queue
    }
    #[test]
    fn view_yield_resumes_only_automatic_work_after_the_old_ack() {
        let mut queue = queue();
        let first_id = queue.rows(queue.root())[0].id;
        queue.dispatch_work().unwrap();
        queue.cancel_for_view();
        let mut automatic = std::mem::take(&mut queue.automatic);
        let visible = vec![OsString::from("automatic"), OsString::from("manual")];
        assert!(!automatic.resume_yielded(&mut queue, &visible, false));
        assert!(
            queue
                .rows(queue.root())
                .iter()
                .all(|row| matches!(row.status, Status::Cancelled))
        );
        assert!(!queue.finish_measurement(Err("cancelled and reaped".into())));
        assert!(automatic.resume_yielded(&mut queue, &visible, true));
        let rows = queue.rows(queue.root());
        assert!(matches!(rows[0].status, Status::Queued));
        assert_ne!(rows[0].id, first_id);
        assert!(matches!(rows[1].status, Status::Cancelled));
        assert!(!automatic.resume_yielded(&mut queue, &visible, true));
    }
    #[test]
    fn explicit_stop_revokes_an_existing_temporary_view_resume_intent() {
        let mut queue = queue();
        queue.cancel_for_view();
        assert!(!queue.automatic.yielded.is_empty());
        queue.cancel_all();
        let mut automatic = std::mem::take(&mut queue.automatic);
        assert!(!automatic.resume_yielded(
            &mut queue,
            &["automatic".into(), "manual".into()],
            true
        ));
        assert!(
            queue
                .rows(queue.root())
                .iter()
                .all(|row| matches!(row.status, Status::Cancelled))
        );
    }
    #[test]
    fn explicit_stop_does_not_gain_a_view_resume_intent() {
        let mut queue = queue();
        queue.cancel_all();
        let mut automatic = std::mem::take(&mut queue.automatic);
        assert!(!automatic.resume_yielded(
            &mut queue,
            &["automatic".into(), "manual".into()],
            true
        ));
        assert!(
            queue
                .rows(queue.root())
                .iter()
                .all(|row| matches!(row.status, Status::Cancelled))
        );
    }
}
