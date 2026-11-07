//! Bounded presentation and callback admission for the rename/create sheet.
//! Native callbacks enqueue values; only `Controller::drain` touches Model.
use crate::{model::Model, rename_dialog::Mode};
use files_core::rename_plan::Case;
use std::{
    cell::RefCell,
    collections::VecDeque,
    ffi::{OsStr, OsString},
    os::unix::ffi::OsStrExt,
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::Instant,
};

pub const FIELD_BYTES: usize = 2048;
pub const PREVIEW_ROWS: usize = 32;
const REQUEST_LIMIT: usize = 8;
const LABEL_BYTES: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub dialog: u64,
    pub revision: u64,
    pub pane: u64,
    pub tab: u64,
    pub generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewRow {
    pub before: String,
    pub after: String,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub stamp: Stamp,
    pub mode: Mode,
    /// Exact editable strings, never display labels or percent-decoded names.
    pub fields: [String; 5],
    pub focus: usize,
    pub case: Case,
    pub regex: bool,
    pub directory: String,
    pub selected_count: usize,
    pub preview_total: usize,
    pub preview_start: usize,
    pub preview: Vec<PreviewRow>,
    pub error: String,
    pub planning: bool,
    pub applying: bool,
    pub closing: bool,
    /// UI admission only; neither rollback nor completion is implied.
    pub cancel_requested: bool,
    pub can_edit: bool,
    pub can_apply: bool,
    pub can_dismiss: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    // Fixtures assign exact values; native callbacks use the relative variants.
    #[cfg_attr(not(test), allow(dead_code))]
    Case(Case),
    CycleCase,
    #[cfg_attr(not(test), allow(dead_code))]
    Regex(bool),
    ToggleRegex,
    Apply,
    /// During Apply this requests cancellation, keeping the sheet until ACK.
    Cancel,
    /// Escape/backdrop dismissal is admitted only before Apply starts.
    Dismiss,
    #[cfg_attr(not(test), allow(dead_code))]
    Scroll(usize),
    ScrollBy(isize),
}

enum Value {
    Edit { field: usize, text: String },
    Action(Action),
}
struct Request {
    stamp: Stamp,
    value: Value,
}

#[derive(Clone, Default)]
pub struct Inbox(Rc<RefCell<VecDeque<Request>>>);
impl Inbox {
    /// Commit the complete native field value, with a strict pre-allocation cap.
    pub fn edit(&self, stamp: Stamp, field: usize, text: &str) -> bool {
        if field >= 5 || text.len() > FIELD_BYTES {
            return false;
        }
        let mut queue = self.0.borrow_mut();
        // Coalesce only adjacent edit batches: never move an edit across Apply.
        for request in queue.iter_mut().rev() {
            let Value::Edit {
                field: old,
                text: previous,
            } = &mut request.value
            else {
                break;
            };
            if request.stamp == stamp && *old == field {
                previous.clear();
                previous.push_str(text);
                return true;
            }
        }
        if queue.len() == REQUEST_LIMIT {
            return false;
        }
        queue.push_back(Request {
            stamp,
            value: Value::Edit {
                field,
                text: text.into(),
            },
        });
        true
    }

    pub fn request(&self, stamp: Stamp, action: Action) -> bool {
        let mut queue = self.0.borrow_mut();
        // Cancellation must remain usable even after a burst of native edits.
        if matches!(action, Action::Cancel | Action::Dismiss) {
            queue.retain(|request| request.stamp != stamp);
        }
        if queue.len() == REQUEST_LIMIT {
            return false;
        }
        queue.push_back(Request {
            stamp,
            value: Value::Action(action),
        });
        true
    }
}

struct Observed {
    stamp: Stamp,
    // Retaining the allocation prevents pointer reuse after a close/reopen.
    // This clones one Arc, never the selected names or the prepared plan.
    selected: Arc<[OsString]>,
    directory: PathBuf,
    mode: Mode,
    fields: [String; 5],
    case: Case,
    regex: bool,
    edited_at: Instant,
    cancel_requested: bool,
}

#[derive(Default, Debug, PartialEq, Eq)]
pub struct Effects {
    pub dirty: bool,
    pub accepted: usize,
    pub rejected: usize,
}

#[derive(Default)]
pub struct Controller {
    inbox: Inbox,
    observed: Option<Observed>,
    next_dialog: u64,
}
impl Controller {
    pub fn inbox(&self) -> Inbox {
        self.inbox.clone()
    }

    fn observe(&mut self, m: &Model) -> Option<Stamp> {
        let Some(d) = m.rename_dialog.as_ref() else {
            self.observed = None;
            return None;
        };
        if d.fields.iter().any(|s| s.len() > FIELD_BYTES)
            || d.directory.as_os_str().as_bytes().len() > 4096
        {
            self.observed = None;
            return None;
        }
        let (pane, tab) = m.pane_owner();
        let same = self.observed.as_ref().is_some_and(|old| {
            Arc::ptr_eq(&old.selected, &d.selected)
                && old.directory == d.directory
                && old.mode == d.mode
                && (old.stamp.pane, old.stamp.tab, old.stamp.generation)
                    == (pane, tab, m.generation)
        });
        if !same {
            self.next_dialog = self.next_dialog.checked_add(1)?;
            self.observed = Some(Observed {
                stamp: Stamp {
                    dialog: self.next_dialog,
                    revision: 0,
                    pane,
                    tab,
                    generation: m.generation,
                },
                selected: Arc::clone(&d.selected),
                directory: d.directory.clone(),
                mode: d.mode,
                fields: d.fields.clone(),
                case: d.case,
                regex: d.regex,
                edited_at: d.edited_at,
                cancel_requested: false,
            });
        }
        let old = self.observed.as_mut()?;
        if old.fields != d.fields
            || old.case != d.case
            || old.regex != d.regex
            || old.edited_at != d.edited_at
        {
            old.stamp.revision = old.stamp.revision.checked_add(1)?;
            old.fields.clone_from(&d.fields);
            old.case = d.case;
            old.regex = d.regex;
            old.edited_at = d.edited_at;
        }
        old.cancel_requested |= m.closing && m.mutation_applied;
        Some(old.stamp)
    }

    pub fn snapshot(&mut self, m: &Model) -> Option<Snapshot> {
        let stamp = self.observe(m)?;
        // An in-flight Apply stays visible until its ACK, including while
        // the owning window waits to close. Competing surfaces still win.
        if competing_surface(m) || (m.closing && !m.mutation_applied) {
            return None;
        }
        let d = m.rename_dialog.as_ref()?;
        let preview_total = d.prepared.as_ref().map_or(0, |p| p.len());
        let preview_start = d.scroll.min(preview_total.saturating_sub(1));
        let preview = d.prepared.as_ref().map_or_else(Vec::new, |p| {
            p.renames()
                .skip(preview_start)
                .take(PREVIEW_ROWS)
                .map(|(before, after)| PreviewRow {
                    before: label(before),
                    after: label(after),
                })
                .collect()
        });
        let can_edit = editable(m);
        let error = if !d.error.is_empty() {
            bounded(&d.error, LABEL_BYTES)
        } else if d.mode != Mode::Rename && !d.fields[0].is_empty() {
            validate_create(&d.fields[0])
                .err()
                .map_or_else(String::new, |error| bounded(&error, LABEL_BYTES))
        } else {
            String::new()
        };
        Some(Snapshot {
            stamp,
            mode: d.mode,
            fields: d.fields.clone(),
            focus: d.focus.min(if d.mode == Mode::Rename { 4 } else { 0 }),
            case: d.case,
            regex: d.regex,
            directory: label(d.directory.as_os_str()),
            selected_count: d.selected.len(),
            preview_total,
            preview_start,
            preview,
            error,
            planning: (d.changed || m.mutations.busy) && !m.mutation_applied,
            applying: m.mutation_applied,
            closing: m.closing,
            cancel_requested: self.observed.as_ref().is_some_and(|s| s.cancel_requested),
            can_edit,
            can_apply: applicable(m),
            can_dismiss: !m.mutation_applied,
        })
    }

    pub fn drain(&mut self, m: &mut Model) -> Effects {
        let base = self.observe(m);
        let requests = std::mem::take(&mut *self.inbox.0.borrow_mut());
        let mut effects = Effects::default();
        for request in requests {
            // Edits in this batch share a native snapshot. Revisions generated
            // by earlier admitted edits in the same batch are intentional.
            let same_dialog = self.observe(m).zip(base).is_some_and(|(now, base)| {
                now.dialog == base.dialog
                    && now.pane == base.pane
                    && now.tab == base.tab
                    && now.generation == base.generation
            });
            if Some(request.stamp) != base || !same_dialog || covered(m) {
                effects.rejected += 1;
                continue;
            }
            let accepted = match request.value {
                Value::Edit { field, text } => {
                    if !editable(m)
                        || (field != 0 && m.rename_dialog.as_ref().unwrap().mode != Mode::Rename)
                    {
                        false
                    } else {
                        m.mutations.cancel();
                        let d = m.rename_dialog.as_mut().unwrap();
                        d.focus = field;
                        d.replace = true;
                        d.input(&text);
                        true
                    }
                }
                Value::Action(action @ (Action::Case(_) | Action::CycleCase)) => {
                    if !editable_rename(m) {
                        false
                    } else {
                        let d = m.rename_dialog.as_mut().unwrap();
                        let case = if let Action::Case(case) = action {
                            case
                        } else {
                            match d.case {
                                Case::Preserve => Case::Lower,
                                Case::Lower => Case::Upper,
                                Case::Upper => Case::Preserve,
                            }
                        };
                        if d.case != case {
                            m.mutations.cancel();
                            d.case = case;
                            d.change();
                        }
                        true
                    }
                }
                Value::Action(action @ (Action::Regex(_) | Action::ToggleRegex)) => {
                    if !editable_rename(m) {
                        false
                    } else {
                        let d = m.rename_dialog.as_mut().unwrap();
                        let regex = if let Action::Regex(regex) = action {
                            regex
                        } else {
                            !d.regex
                        };
                        if d.regex != regex {
                            m.mutations.cancel();
                            d.regex = regex;
                            d.change();
                        }
                        true
                    }
                }
                Value::Action(Action::Apply) => {
                    let can_apply = applicable(m);
                    if can_apply {
                        m.apply_rename();
                    }
                    can_apply
                }
                Value::Action(Action::Cancel) => {
                    if m.mutation_applied
                        && let Some(observed) = &mut self.observed
                    {
                        observed.cancel_requested = true;
                    }
                    m.cancel_rename();
                    true
                }
                Value::Action(Action::Dismiss) => {
                    if m.mutation_applied {
                        false
                    } else {
                        m.cancel_rename();
                        true
                    }
                }
                Value::Action(action @ (Action::Scroll(_) | Action::ScrollBy(_))) => {
                    let d = m.rename_dialog.as_mut().unwrap();
                    let start = match action {
                        Action::Scroll(start) => start,
                        Action::ScrollBy(delta) => d.scroll.saturating_add_signed(delta),
                        _ => unreachable!(),
                    };
                    d.scroll =
                        start.min(d.prepared.as_ref().map_or(0, |p| p.len().saturating_sub(1)));
                    true
                }
            };
            if accepted {
                effects.accepted += 1;
                effects.dirty = true;
            } else {
                effects.rejected += 1;
            }
        }
        self.observe(m);
        effects
    }
}

fn editable(m: &Model) -> bool {
    !m.mutation_applied && !m.closing && m.rename_dialog.is_some()
}
fn editable_rename(m: &Model) -> bool {
    editable(m)
        && m.rename_dialog
            .as_ref()
            .is_some_and(|d| d.mode == Mode::Rename)
}
fn applicable(m: &Model) -> bool {
    editable(m)
        && !m.mutations.busy
        && m.rename_dialog.as_ref().is_some_and(|d| {
            d.error.is_empty()
                && (if d.mode != Mode::Rename {
                    validate_create(&d.fields[0]).is_ok()
                } else {
                    !d.changed && d.id != 0 && d.prepared.as_ref().is_some_and(|p| !p.is_empty())
                })
        })
}

/// Match the worker's portable component and UTF-8 byte limits without I/O.
fn validate_create(name: &str) -> Result<(), String> {
    files_core::rename_plan::valid_name(name, true)?;
    if name.len() > 255 {
        return Err("Nombre demasiado largo (máximo 255 bytes)".into());
    }
    Ok(())
}

fn covered(m: &Model) -> bool {
    m.closing || competing_surface(m)
}

/// Competing surfaces exclude this sheet's own dialog and pending worker.
fn competing_surface(m: &Model) -> bool {
    m.delete.confirm.is_some()
        || m.delete.busy()
        || m.copy_busy()
        || m.open_with.is_some()
        || m.trash_browser.is_some()
        || m.comparison.is_some()
        || m.checksums.is_some()
        || m.sftp.form.is_some()
        || m.saved_searches.panel.is_some()
        || m.bookmarks.panel.is_some()
        || m.commands.is_some()
        || m.search.visible
        || m.size_panel.is_some()
        || m.help.is_some()
        || m.context_menu.is_some()
        || m.properties_dialog.is_some()
        || m.recovery_catalog.open
        || m.notice_open
        || m.copy_form.is_some()
        || m.preview.visible()
        || m.settings
        || m.actions_menu
        || m.tab_context.is_some()
        || m.breadcrumbs.open
        || m.editing
        || m.filter_edit.active
}

fn bounded(text: &str, bytes: usize) -> String {
    if text.len() <= bytes {
        return text.into();
    }
    let mut end = bytes.saturating_sub('…'.len_utf8());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}
fn label(name: &OsStr) -> String {
    // Input names are bounded by the model; cap defensively before escaping.
    let bytes = name.as_bytes();
    let prefix = OsStr::from_bytes(&bytes[..bytes.len().min(4096)]);
    bounded(&files_core::name_label::label(prefix), LABEL_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;
    use files_core::rename_transaction::PreparedRename;
    use std::{
        fs,
        os::unix::fs::DirBuilderExt,
        sync::{
            atomic::{AtomicU64, Ordering},
            mpsc,
        },
        time::{Duration, Instant},
    };

    static NEXT: AtomicU64 = AtomicU64::new(0);

    /// Retain private evidence; no recursive cleanup or host path fixtures.
    fn model(mode: Mode) -> Model {
        let temp = PathBuf::from(std::env::var_os("TMPDIR").expect("private TMPDIR is required"));
        let root = temp.canonicalize().unwrap();
        assert_ne!(root, std::env::temp_dir().parent().unwrap_or(&root));
        for key in ["FILES_CONFIG_DIR", "XDG_DATA_HOME"] {
            let path =
                PathBuf::from(std::env::var_os(key).expect("private settings roots are required"));
            assert!(
                path.canonicalize().unwrap().starts_with(&root),
                "{key} must be inside private TMPDIR"
            );
        }
        let path = root.join(format!(
            "mutation-sheet-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        let mut m = Model::new(path.clone());
        let (tx, _) = mpsc::sync_channel(4);
        let (_, rx) = mpsc::channel();
        m.aux = crate::aux_io::AuxIo { tx, rx };
        m.rename_dialog = Some(crate::rename_dialog::Dialog::new(
            19,
            path,
            Arc::from([]),
            mode,
        ));
        m
    }

    fn stamp(controller: &mut Controller, m: &Model) -> Stamp {
        controller.snapshot(m).unwrap().stamp
    }

    #[test]
    fn worker_ids_and_completion_do_not_change_draft_identity() {
        let mut m = model(Mode::Rename);
        let mut c = Controller::default();
        let original = stamp(&mut c, &m);
        let d = m.rename_dialog.as_mut().unwrap();
        d.id = 0;
        d.changed = false;
        d.error = "Planning result".into();
        assert_eq!(original, stamp(&mut c, &m));
        m.rename_dialog.as_mut().unwrap().id = 900;
        assert_eq!(original, stamp(&mut c, &m));
        m.rename_dialog
            .as_mut()
            .unwrap()
            .input("{name}-{n:03}{dotext}");
        let edited = stamp(&mut c, &m);
        assert_eq!(original.dialog, edited.dialog);
        assert_eq!(edited.revision, original.revision + 1);
        let d = m.rename_dialog.as_mut().unwrap();
        d.replace = true;
        d.input("temporary");
        d.replace = true;
        d.input("{name}-{n:03}{dotext}");
        assert_eq!(stamp(&mut c, &m).revision, edited.revision + 1);
    }

    #[test]
    fn closed_and_reopened_empty_selection_rejects_old_callbacks() {
        let mut m = model(Mode::File);
        let mut c = Controller::default();
        let old = stamp(&mut c, &m);
        let inbox = c.inbox();
        assert!(inbox.edit(old, 0, "obsolete.txt"));
        m.cancel_rename();
        m.open_rename(Mode::File);
        assert_eq!(c.drain(&mut m).rejected, 1);
        assert_ne!(old.dialog, stamp(&mut c, &m).dialog);
        assert!(m.rename_dialog.as_ref().unwrap().fields[0].is_empty());
    }

    #[test]
    fn owner_generation_draft_and_competing_surface_reject_stale_requests() {
        for change in 0..5 {
            let mut m = model(Mode::File);
            let mut c = Controller::default();
            let old = stamp(&mut c, &m);
            c.inbox().edit(old, 0, "stale.txt");
            match change {
                0 => m.generation += 1,
                1 => m.tabs.active += 1,
                2 => m.dual.active_id += 1,
                3 => m.rename_dialog.as_mut().unwrap().input("external.txt"),
                _ => m.settings = true,
            }
            assert_eq!(c.drain(&mut m).rejected, 1);
            assert_ne!(m.rename_dialog.as_ref().unwrap().fields[0], "stale.txt");
        }
    }

    #[test]
    fn full_text_edits_coalesce_and_keep_raw_percent_and_unicode() {
        let mut m = model(Mode::Rename);
        let mut c = Controller::default();
        let old = stamp(&mut c, &m);
        let q = c.inbox();
        assert!(q.edit(old, 0, "old"));
        assert!(q.edit(old, 0, "100%-%2F-é-{n}"));
        assert!(q.edit(old, 1, "30"));
        assert!(q.edit(old, 2, "2"));
        assert!(q.edit(old, 3, "^(.*)$"));
        assert!(q.edit(old, 4, "$1%20"));
        assert!(q.request(old, Action::Case(Case::Upper)));
        assert!(q.request(old, Action::Regex(true)));
        let effects = c.drain(&mut m);
        assert_eq!(effects.accepted, 7);
        assert_eq!(effects.rejected, 0);
        let d = m.rename_dialog.as_ref().unwrap();
        assert_eq!(d.fields[0], "100%-%2F-é-{n}");
        let options = d.options().unwrap();
        assert_eq!(
            (options.start, options.step, options.case, options.regex),
            (30, 2, Case::Upper, true)
        );
        assert_eq!(options.replace, Some(("^(.*)$".into(), "$1%20".into())));
        let current = stamp(&mut c, &m);
        for action in [
            Action::CycleCase,
            Action::CycleCase,
            Action::ToggleRegex,
            Action::ToggleRegex,
        ] {
            assert!(q.request(current, action));
        }
        assert_eq!(c.drain(&mut m).accepted, 4);
        let options = m.rename_dialog.as_ref().unwrap().options().unwrap();
        assert_eq!((options.case, options.regex), (Case::Lower, true));
    }

    #[test]
    fn request_and_field_budgets_are_enforced_before_model_changes() {
        let mut m = model(Mode::File);
        let mut c = Controller::default();
        let old = stamp(&mut c, &m);
        let q = c.inbox();
        assert!(!q.edit(old, 0, &"é".repeat(FIELD_BYTES / 2 + 1)));
        assert!(!q.edit(old, 5, "invalid field"));
        assert!(q.edit(old, 0, &"é".repeat(FIELD_BYTES / 2)));
        for _ in 1..REQUEST_LIMIT {
            assert!(q.request(old, Action::Scroll(usize::MAX)));
        }
        assert!(!q.request(old, Action::Apply));
        assert_eq!(q.0.borrow().len(), REQUEST_LIMIT);
        let e = c.drain(&mut m);
        assert_eq!(e.accepted, REQUEST_LIMIT);
        assert_eq!(
            m.rename_dialog.as_ref().unwrap().fields[0].len(),
            FIELD_BYTES
        );
        assert!(!m.mutation_applied);
        let current = stamp(&mut c, &m);
        assert!(q.request(current, Action::Apply));
        assert!(q.request(current, Action::Dismiss));
        assert_eq!(c.drain(&mut m).accepted, 1);
        assert!(m.rename_dialog.is_none());
        assert!(!m.mutation_applied);
        assert!(!m.mutations.busy);
    }

    #[test]
    fn create_only_admits_its_name_field_and_reuses_input_sanitization() {
        for mode in [Mode::File, Mode::Folder] {
            let mut m = model(mode);
            let mut c = Controller::default();
            let empty = c.snapshot(&m).unwrap();
            assert!(!empty.can_apply);
            assert!(empty.error.is_empty());
            let old = stamp(&mut c, &m);
            let q = c.inbox();
            q.edit(old, 1, "20");
            q.edit(old, 0, "a\n\0\ré%20.txt");
            q.request(old, Action::Regex(true));
            let e = c.drain(&mut m);
            assert_eq!((e.accepted, e.rejected), (1, 2));
            let d = m.rename_dialog.as_ref().unwrap();
            assert_eq!(d.fields[0], "aé%20.txt");
            assert_eq!(d.fields[1], "1");
            assert!(!d.regex);
            assert!(c.snapshot(&m).unwrap().can_apply);
            assert!(c.snapshot(&m).unwrap().error.is_empty());
            for name in [
                String::new(),
                ".".into(),
                "..".into(),
                "part/name".into(),
                "CON.txt".into(),
                "x".repeat(256),
                "é".repeat(128),
            ] {
                let current = stamp(&mut c, &m);
                assert!(q.edit(current, 0, &name));
                assert_eq!(c.drain(&mut m).accepted, 1);
                let invalid = c.snapshot(&m).unwrap();
                assert!(!invalid.can_apply, "invalid create name was admitted");
                assert_eq!(invalid.error.is_empty(), name.is_empty());
                assert!(q.request(invalid.stamp, Action::Apply));
                assert_eq!(c.drain(&mut m).rejected, 1);
                assert!(!m.mutations.busy);
                assert!(!m.mutation_applied);
            }
            for name in ["100%-%2F-é.txt".into(), format!("{}x", "é".repeat(127))] {
                let current = stamp(&mut c, &m);
                assert!(q.edit(current, 0, &name));
                c.drain(&mut m);
                let valid = c.snapshot(&m).unwrap();
                assert!(valid.can_apply);
                assert!(valid.error.is_empty());
                assert_eq!(valid.fields[0], name);
            }
        }
    }

    #[test]
    fn edits_invalidate_prepared_plan_before_apply_in_the_same_batch() {
        let mut m = model(Mode::Rename);
        fs::write(m.path.join("old.txt"), b"keep").unwrap();
        let prepared =
            PreparedRename::capture(&m.path, vec![("old.txt".into(), "new.txt".into())]).unwrap();
        let d = m.rename_dialog.as_mut().unwrap();
        d.prepared = Some(prepared);
        d.changed = false;
        let mut c = Controller::default();
        assert!(c.snapshot(&m).unwrap().can_apply);
        m.rename_dialog.as_mut().unwrap().error = "Rejected preview".into();
        assert!(!c.snapshot(&m).unwrap().can_apply);
        m.rename_dialog.as_mut().unwrap().error.clear();
        let old = stamp(&mut c, &m);
        let q = c.inbox();
        q.edit(old, 0, "another.txt");
        q.request(old, Action::Apply);
        let e = c.drain(&mut m);
        assert_eq!((e.accepted, e.rejected), (1, 1));
        assert!(m.rename_dialog.as_ref().unwrap().prepared.is_none());
        assert!(!m.mutation_applied);
        assert_eq!(fs::read(m.path.join("old.txt")).unwrap(), b"keep");
        assert!(!m.path.join("new.txt").exists());
    }

    #[test]
    fn snapshot_borrows_large_selection_and_pages_escaped_preview() {
        let mut m = model(Mode::Rename);
        let mut names = Vec::new();
        for i in 0..(PREVIEW_ROWS + 5) {
            let from = OsString::from(format!("name-{i}%-é.txt"));
            fs::write(m.path.join(&from), b"keep").unwrap();
            names.push((from, OsString::from(format!("new-{i}.txt"))));
        }
        let raw = OsStr::from_bytes(b"raw-\xff%.txt").to_owned();
        fs::write(m.path.join(&raw), b"raw").unwrap();
        names[0].0 = raw;
        let prepared = PreparedRename::capture(&m.path, names).unwrap();
        let d = m.rename_dialog.as_mut().unwrap();
        d.selected = Arc::from(vec![OsString::from("a"); 100_000]);
        let selected = Arc::clone(&d.selected);
        d.prepared = Some(prepared);
        d.changed = false;
        d.error = "é".repeat(3000);
        let ptr = d.prepared.as_ref().unwrap() as *const PreparedRename;
        let mut c = Controller::default();
        let snapshot = c.snapshot(&m).unwrap();
        assert_eq!(Arc::strong_count(&selected), 3);
        assert_eq!(snapshot.selected_count, 100_000);
        assert_eq!(snapshot.preview.len(), PREVIEW_ROWS);
        assert_eq!(snapshot.preview_total, PREVIEW_ROWS + 5);
        assert_eq!(snapshot.preview[0].before, "raw-\\xFF%.txt");
        assert!(snapshot.error.len() <= LABEL_BYTES);
        assert_eq!(
            ptr,
            m.rename_dialog.as_ref().unwrap().prepared.as_ref().unwrap() as *const PreparedRename
        );
        for delta in [3, 3, -2] {
            assert!(c.inbox().request(snapshot.stamp, Action::ScrollBy(delta)));
        }
        assert_eq!(c.drain(&mut m).accepted, 3);
        assert_eq!(c.snapshot(&m).unwrap().preview_start, 4);
        assert!(
            c.inbox()
                .request(snapshot.stamp, Action::ScrollBy(isize::MIN))
        );
        c.drain(&mut m);
        assert_eq!(c.snapshot(&m).unwrap().preview_start, 0);
        assert!(
            c.inbox()
                .request(snapshot.stamp, Action::ScrollBy(isize::MAX))
        );
        c.drain(&mut m);
        assert_eq!(c.snapshot(&m).unwrap().preview_start, PREVIEW_ROWS + 4);
        c.inbox()
            .request(snapshot.stamp, Action::Scroll(usize::MAX));
        c.drain(&mut m);
        let end = c.snapshot(&m).unwrap();
        assert_eq!(end.preview_start, PREVIEW_ROWS + 4);
        assert_eq!(end.preview.len(), 1);
    }

    #[test]
    fn readonly_planning_can_cancel_but_pending_apply_requires_ack() {
        let mut m = model(Mode::Rename);
        fs::write(m.path.join("before.txt"), b"keep").unwrap();
        let d = m.rename_dialog.as_mut().unwrap();
        d.selected = Arc::from([OsString::from("before.txt")]);
        d.input("old-plan.txt");
        d.edited_at = Instant::now() - Duration::from_millis(200);
        m.pump_mutations();
        assert!(m.mutations.busy);
        let mut c = Controller::default();
        let planning = c.snapshot(&m).unwrap();
        assert!(planning.can_edit);
        assert!(!planning.can_apply);
        c.inbox().edit(planning.stamp, 0, "new-plan.txt");
        assert_eq!(c.drain(&mut m).accepted, 1);
        // Hold the next debounce while the obsolete worker ACK is drained.
        m.rename_dialog.as_mut().unwrap().edited_at = Instant::now() + Duration::from_secs(30);
        let deadline = Instant::now() + Duration::from_secs(5);
        while m.mutations.busy {
            m.pump_mutations();
            assert!(Instant::now() < deadline, "obsolete plan ACK timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(m.rename_dialog.as_ref().unwrap().prepared.is_none());
        assert_eq!(m.rename_dialog.as_ref().unwrap().fields[0], "new-plan.txt");
        m.rename_dialog.as_mut().unwrap().edited_at = Instant::now() - Duration::from_millis(200);
        m.pump_mutations();
        while m.mutations.busy {
            m.pump_mutations();
            assert!(Instant::now() < deadline, "current plan ACK timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(c.snapshot(&m).unwrap().can_apply);
        let prepared = m.rename_dialog.as_ref().unwrap().prepared.as_ref().unwrap();
        assert_eq!(
            prepared.renames().next().unwrap().1,
            OsStr::new("new-plan.txt")
        );
        assert_eq!(fs::read(m.path.join("before.txt")).unwrap(), b"keep");

        let mut m = model(Mode::File);
        let mut c = Controller::default();
        m.mutations.busy = true;
        let planning = c.snapshot(&m).unwrap();
        assert!(planning.can_dismiss);
        assert!(planning.can_edit);
        m.closing = true;
        assert!(c.snapshot(&m).is_none());
        m.closing = false;
        c.inbox().request(planning.stamp, Action::Dismiss);
        c.drain(&mut m);
        assert!(m.rename_dialog.is_none());

        let mut m = model(Mode::File);
        m.mutation_applied = true;
        m.mutations.busy = true;
        let pending = c.snapshot(&m).unwrap();
        assert!(pending.applying);
        assert!(!pending.cancel_requested);
        assert!(!pending.can_dismiss);
        c.inbox().request(pending.stamp, Action::Dismiss);
        assert_eq!(c.drain(&mut m).rejected, 1);
        c.inbox().request(pending.stamp, Action::Cancel);
        assert_eq!(c.drain(&mut m).accepted, 1);
        assert!(m.rename_dialog.is_some());
        assert!(m.mutation_applied);
        assert!(c.snapshot(&m).unwrap().cancel_requested);

        let mut m = model(Mode::File);
        assert!(!c.snapshot(&m).unwrap().cancel_requested);
        let before = stamp(&mut c, &m);
        c.inbox().edit(before, 0, "closing-ack.txt");
        c.inbox().request(before, Action::Apply);
        assert_eq!(c.drain(&mut m).accepted, 2);
        m.closing = true;
        let waiting = c.snapshot(&m).expect("Apply must remain visible until ACK");
        assert!(waiting.closing && waiting.applying);
        assert!(waiting.cancel_requested);
        assert!(!waiting.can_edit && !waiting.can_apply && !waiting.can_dismiss);
        c.inbox().request(waiting.stamp, Action::Cancel);
        assert_eq!(c.drain(&mut m).rejected, 1);
        assert!(m.rename_dialog.is_some() && m.mutation_applied);
        m.settings = true;
        assert!(
            c.snapshot(&m).is_none(),
            "a competing surface still covers Apply"
        );
        m.settings = false;
        assert!(c.snapshot(&m).is_some());
        let deadline = Instant::now() + Duration::from_secs(5);
        while m.mutations.busy {
            m.pump_mutations();
            assert!(Instant::now() < deadline, "closing Apply ACK timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!m.mutation_applied);
        assert!(m.rename_dialog.is_none());
        assert!(c.snapshot(&m).is_none());
    }

    #[test]
    fn create_uses_controller_worker_and_closes_only_after_finished_ack() {
        for (mode, folder) in [(Mode::File, false), (Mode::Folder, true)] {
            let mut m = model(mode);
            let mut c = Controller::default();
            let old = stamp(&mut c, &m);
            c.inbox().edit(old, 0, "100%-%20-é");
            c.inbox().request(old, Action::Apply);
            assert_eq!(c.drain(&mut m).accepted, 2);
            assert!(m.mutation_applied);
            assert!(m.rename_dialog.is_some());
            assert!(!c.snapshot(&m).unwrap().can_dismiss);
            let deadline = Instant::now() + Duration::from_secs(5);
            while m.mutations.busy {
                m.pump_mutations();
                assert!(Instant::now() < deadline, "worker ACK timed out");
                std::thread::sleep(Duration::from_millis(1));
            }
            assert!(!m.mutation_applied);
            assert!(m.rename_dialog.is_none());
            let metadata = fs::metadata(m.path.join("100%-%20-é")).unwrap();
            assert_eq!(metadata.is_dir(), folder);
        }
    }
}
