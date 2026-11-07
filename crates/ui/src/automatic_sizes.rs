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
    collections::{HashMap, HashSet},
    ffi::OsString,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
const MAX_VISIBLE: usize = 64;
const SETTLE: Duration = Duration::from_millis(300);
const SAVE_EVERY: Duration = Duration::from_secs(2);
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
    warning: Option<String>,
    cached_hits: usize,
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
            warning: None,
            cached_hits: 0,
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
            self.cached_hits = 0;
        }
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
        // A viewport change discards only our own offscreen records. Explicitly
        // requested measurements and the single active ACK slot remain owned.
        let obsolete: Vec<_> = m
            .sizes
            .rows(&m.path)
            .iter()
            .filter(|r| self.automatic.contains(&r.id) && !names.contains(&r.name))
            .map(|r| (r.id, r.name.clone()))
            .collect();
        for (id, name) in obsolete {
            if m.sizes.active_id() == Some(id) {
                if m.sizes.cancel_id(m.sizes.epoch(), id) {
                    if let Some(job) = &m.du_job {
                        job.cancel();
                    }
                    dirty = true;
                }
            } else if m.sizes.forget_automatic(id) {
                self.automatic.remove(&id);
                self.published.remove(&id);
                self.cached.remove(&name);
                self.partials.remove(&name);
                self.stamps.remove(&name);
                self.proof_ids.remove(&name);
                self.checked.remove(&name);
                self.yielded.remove(&name);
                dirty = true;
            }
        }
        self.checked
            .retain(|n| names.contains(n) || m.sizes.rows(&m.path).iter().any(|r| &r.name == n));
        self.stamps.retain(|n, _| self.checked.contains(n));
        self.cached.retain(|n, _| self.checked.contains(n));
        self.partials.retain(|n, _| self.checked.contains(n));
        self.proof_ids.retain(|n, _| self.checked.contains(n));
        self.yielded.retain(|n| self.checked.contains(n));
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
        for name in &names {
            if self.stamps.contains_key(name)
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
            if !missing.is_empty() && self.checked.len() + missing.len() <= MAX_ROWS {
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
                self.checked.extend(names);
                self.warning = warning;
                for Checked {
                    name,
                    stamp,
                    cached,
                } in rows
                {
                    self.stamps.insert(name.clone(), stamp);
                    if let Some(value) = cached {
                        self.cached_hits = self.cached_hits.saturating_add(1);
                        self.cached.insert(name, value);
                    }
                }
                true
            }
            (PendingKind::Read(names), Err(error)) => {
                self.checked.extend(names);
                self.warning = Some(error);
                true
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
        queue.yield_automatic();
        queue.cancel_all();
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
