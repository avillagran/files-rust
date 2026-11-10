//! The window's single linear Undo/Redo history, owned by the mutation worker.
//!
//! Safety contract (docs/UNDO.md):
//! - Undo never deletes. Undoing a creation sends exactly the recorded objects
//!   to the trash through `TrashContext::trash_expected`; undoing a move
//!   renames back with `RENAME_NOREPLACE`.
//! - Only objects recorded from the completed operation's own report, with
//!   (device, inode, type), are eligible. Each is re-verified right before it
//!   is acted on (`undo_ops::verify` / `undo_ops::move_back`); anything that
//!   changed is skipped and reported, never "repaired".
//! - Redo only restores trashed objects from their receipts or moves again
//!   with no-replace. A failed precondition is reported; nothing is replaced.
//! - Bounded: `MAX_ENTRIES` entries, `MAX_OBJECTS` objects per entry and
//!   `MAX_RECEIPTS` trash receipts (three descriptors each) in total.
use crate::{rename_job::Outcome, trash_history};
use files_core::{
    rename_transaction::UndoToken,
    trash::{self, Receipt, TrashContext},
    undo_ops::{self, Created, Moved, Refusal, Stamp},
};
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

pub const MAX_ENTRIES: usize = 20;
/// Larger operations are reported as not undoable instead of being truncated.
pub const MAX_OBJECTS: usize = 64;
/// Trash receipts retained by the whole history (about 3 FDs each).
pub const MAX_RECEIPTS: usize = 128;
const MAX_DETAILS: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    NewFolder,
    NewFile,
    Template,
    Links,
    Copy,
}
/// A completed operation handed to the history. Holds no descriptors.
#[derive(Clone, Debug)]
pub enum Record {
    Created {
        kind: Kind,
        objects: Vec<Created>,
        fence: Stamp,
    },
    Moved {
        items: Vec<Moved>,
    },
}
/// Non-filesystem commands queued by the UI for the worker's history.
#[derive(Debug)]
pub enum Command {
    Record(Box<Record>),
    /// The trash was emptied or edited outside this history: receipts are
    /// no longer a sound basis for Undo/Redo, so those entries are dropped.
    InvalidateTrash,
    /// A volume was unmounted or ejected: recorded paths and receipts may
    /// now name other objects, so the whole history is dropped (nothing on
    /// disk changes; trashed objects stay restorable from the trash).
    InvalidateAll,
}

enum Entry {
    Rename {
        token: UndoToken,
        directory: PathBuf,
        count: usize,
    },
    /// Undo restores; there is no Redo for a trash batch.
    Trash {
        batch: trash_history::History,
        directory: PathBuf,
    },
    /// Undo sends the objects to the trash.
    Created {
        kind: Kind,
        objects: Vec<Created>,
        fence: Stamp,
    },
    /// Redo form of `Created`: receipts of what Undo trashed, plus a fence
    /// taken right after trashing (proves nothing changed while in the trash).
    Trashed {
        kind: Kind,
        items: Vec<(Created, Receipt)>,
        fence: Stamp,
    },
    /// On the Undo stack, Undo moves back to the origin; on the Redo stack,
    /// Redo moves to the destination again.
    Moved { items: Vec<Moved> },
}
impl Entry {
    fn receipts(&self) -> usize {
        match self {
            Self::Trash { batch, .. } => batch.len(),
            Self::Trashed { items, .. } => items.len(),
            _ => 0,
        }
    }
    /// The folder whose listing this entry's Undo (or Redo) changes.
    fn directory(&self, redo: bool) -> Option<PathBuf> {
        let parent = |p: &Path| p.parent().map(Path::to_path_buf);
        match self {
            Self::Rename { directory, .. } | Self::Trash { directory, .. } => {
                Some(directory.clone())
            }
            Self::Created { objects, .. } => objects.first().and_then(|c| parent(&c.path)),
            Self::Trashed { items, .. } => items.first().and_then(|(c, _)| parent(&c.path)),
            Self::Moved { items, .. } => items
                .first()
                .and_then(|m| parent(if redo { &m.to } else { &m.from })),
        }
    }
    fn label(&self) -> String {
        match self {
            Self::Rename { count: n, .. } => count("renombrar", *n),
            Self::Trash { batch, .. } => count("enviar a Papelera", batch.len()),
            Self::Created { kind, objects, .. } => describe(
                *kind,
                objects.len(),
                objects.first().map(|c| c.path.as_path()),
            ),
            Self::Trashed { kind, items, .. } => describe(
                *kind,
                items.len(),
                items.first().map(|(c, _)| c.path.as_path()),
            ),
            Self::Moved { items, .. } => count("mover", items.len()),
        }
    }
}
fn count(verb: &str, n: usize) -> String {
    if n == 1 {
        format!("{verb} 1 elemento")
    } else {
        format!("{verb} {n} elementos")
    }
}
fn name(path: Option<&Path>) -> String {
    path.and_then(Path::file_name)
        .map(|n| files_core::name_label::label(n).into_owned())
        .unwrap_or_default()
}
fn describe(kind: Kind, n: usize, first: Option<&Path>) -> String {
    match kind {
        Kind::NewFolder => format!("crear carpeta «{}»", name(first)),
        Kind::NewFile => format!("crear archivo «{}»", name(first)),
        Kind::Template => format!("crear documento «{}»", name(first)),
        Kind::Links if n == 1 => format!("crear enlace «{}»", name(first)),
        Kind::Links => format!("crear {n} enlaces"),
        Kind::Copy => count("copiar", n),
    }
}
fn capitalized(verb: &str, label: &str) -> String {
    format!("{verb} {label}")
}
fn push_detail(details: &mut Vec<String>, text: String) {
    if details.len() < MAX_DETAILS {
        details.push(text);
    } else if details.len() == MAX_DETAILS {
        details.push("Informe truncado".into());
    }
}

#[derive(Default)]
pub struct History {
    undo: Vec<Entry>,
    redo: Vec<Entry>,
}
impl History {
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    /// Spanish labels for the top entries ("Deshacer copiar 3 elementos").
    pub fn labels(&self) -> (Option<String>, Option<String>) {
        (
            self.undo
                .last()
                .map(|e| capitalized("Deshacer", &e.label())),
            self.redo.last().map(|e| capitalized("Rehacer", &e.label())),
        )
    }
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
    fn receipts(&self) -> usize {
        self.undo
            .iter()
            .chain(&self.redo)
            .map(Entry::receipts)
            .sum()
    }
    /// A new operation: Redo is discarded (its receipts are dropped, which
    /// never erases anything from the trash) and the bounds are enforced.
    fn push(&mut self, entry: Entry) {
        self.redo.clear();
        self.undo.push(entry);
        self.bound();
    }
    fn bound(&mut self) {
        while self.undo.len() > MAX_ENTRIES {
            self.undo.remove(0);
        }
        while self.redo.len() > MAX_ENTRIES {
            self.redo.remove(0);
        }
        // Oldest receipt holders go first; the newest entry is kept.
        while self.receipts() > MAX_RECEIPTS {
            if let Some(i) = self.redo.iter().position(|e| e.receipts() > 0)
                && self.redo.len() > 1
            {
                self.redo.remove(i);
            } else if let Some(i) = self.undo.iter().position(|e| e.receipts() > 0)
                && i + 1 < self.undo.len()
            {
                self.undo.remove(i);
            } else {
                break;
            }
        }
    }
    pub fn push_rename(&mut self, token: UndoToken, directory: PathBuf, count: usize) {
        self.push(Entry::Rename {
            token,
            directory,
            count,
        });
    }
    /// A trash batch with at least one committed receipt.
    pub fn push_trash(&mut self, batch: trash_history::History, directory: PathBuf) {
        if batch.available() {
            self.push(Entry::Trash { batch, directory });
        }
    }
    /// Folder to refresh after the next Undo (`redo == false`) or Redo.
    pub fn directory(&self, redo: bool) -> Option<PathBuf> {
        if redo {
            self.redo.last()
        } else {
            self.undo.last()
        }
        .and_then(|e| e.directory(redo))
    }
    pub fn apply(&mut self, command: Command) {
        match command {
            Command::Record(record) => match *record {
                Record::Created {
                    kind,
                    objects,
                    fence,
                } if !objects.is_empty() && objects.len() <= MAX_OBJECTS => {
                    self.push(Entry::Created {
                        kind,
                        objects,
                        fence,
                    })
                }
                Record::Moved { items } if !items.is_empty() && items.len() <= MAX_OBJECTS => {
                    self.push(Entry::Moved { items })
                }
                _ => {}
            },
            Command::InvalidateAll => self.clear(),
            Command::InvalidateTrash => {
                self.undo.retain(|e| e.receipts() == 0);
                self.redo.retain(|e| e.receipts() == 0);
            }
        }
    }
    fn finish(&self, mut outcome: Outcome) -> Outcome {
        outcome.undo = self.can_undo();
        outcome.redo = self.can_redo();
        outcome
    }
    fn nothing(&self, redo: bool) -> Outcome {
        self.finish(Outcome {
            success: false,
            message: if redo {
                "No hay operación para rehacer"
            } else {
                "No hay operación para deshacer"
            }
            .into(),
            details: Vec::new(),
            undo: false,
            redo: false,
        })
    }

    /// Undo the top entry. `root` is the home trash root.
    pub fn undo(&mut self, root: &Path, cancel: &AtomicBool) -> Outcome {
        let Some(entry) = self.undo.pop() else {
            return self.nothing(false);
        };
        let label = entry.label();
        let outcome = match entry {
            Entry::Rename {
                token,
                directory,
                count,
            } => self.rename(token, directory, count, false, cancel),
            Entry::Trash {
                mut batch,
                directory,
            } => {
                let outcome = batch.restore(cancel);
                if batch.available() {
                    self.undo.push(Entry::Trash { batch, directory });
                } else {
                    // Not redoable: keep the history linear.
                    self.redo.clear();
                }
                outcome
            }
            Entry::Created {
                kind,
                objects,
                fence,
            } => self.trash_created(kind, objects, fence, root, cancel, &label),
            Entry::Moved { items, .. } => self.move_items(items, false, cancel, &label),
            Entry::Trashed { .. } => unreachable!("Trashed entries live on the Redo stack"),
        };
        self.finish(outcome)
    }
    pub fn redo(&mut self, cancel: &AtomicBool) -> Outcome {
        let Some(entry) = self.redo.pop() else {
            return self.nothing(true);
        };
        let label = entry.label();
        let outcome = match entry {
            Entry::Rename {
                token,
                directory,
                count,
            } => self.rename(token, directory, count, true, cancel),
            Entry::Trashed { kind, items, fence } => {
                self.restore_trashed(kind, items, fence, cancel, &label)
            }
            Entry::Moved { items, .. } => self.move_items(items, true, cancel, &label),
            Entry::Trash { .. } | Entry::Created { .. } => {
                unreachable!("only Undo results live on the Redo stack")
            }
        };
        self.finish(outcome)
    }

    fn rename(
        &mut self,
        token: UndoToken,
        directory: PathBuf,
        count: usize,
        redo: bool,
        cancel: &AtomicBool,
    ) -> Outcome {
        match token.undo_cancellable(cancel) {
            Ok(next) => {
                let details = crate::rename_job::cleanup_details(&next);
                let entry = Entry::Rename {
                    token: next,
                    directory,
                    count,
                };
                if redo {
                    self.undo.push(entry);
                } else {
                    self.redo.push(entry);
                }
                Outcome {
                    success: true,
                    message: if redo {
                        "Renombrado rehecho"
                    } else {
                        "Renombrado deshecho"
                    }
                    .into(),
                    details,
                    undo: false,
                    redo: false,
                }
            }
            Err(error) => {
                let restored = error.preserved_object_count == 0
                    && error.unresolved_preserved_objects == 0
                    && error.rollback_errors.is_empty()
                    && error.omitted_rollback_errors == 0;
                if restored {
                    let entry = Entry::Rename {
                        token,
                        directory,
                        count,
                    };
                    if redo {
                        self.redo.push(entry);
                    } else {
                        self.undo.push(entry);
                    }
                }
                crate::rename_job::failed(error)
            }
        }
    }

    /// Undo of a creation: verify each recorded object, then trash exactly
    /// that inode. Skipped objects are reported and leave the history.
    fn trash_created(
        &mut self,
        kind: Kind,
        mut objects: Vec<Created>,
        fence: Stamp,
        root: &Path,
        cancel: &AtomicBool,
        label: &str,
    ) -> Outcome {
        let context = match TrashContext::open_with(root, trash_history::volumes()) {
            Ok(c) => c,
            Err(e) => {
                self.undo.push(Entry::Created {
                    kind,
                    objects,
                    fence,
                });
                return Outcome {
                    success: false,
                    message: format!(
                        "No se pudo abrir la Papelera: {}; no se deshizo nada",
                        trash_history::cause_message(&e.cause)
                    ),
                    details: trash_history::failure_details(&e),
                    undo: false,
                    redo: false,
                };
            }
        };
        let mut budget = undo_ops::SCAN_BUDGET;
        let mut trashed = Vec::new();
        let mut details = Vec::new();
        let mut skipped = 0usize;
        let mut was_cancelled = false;
        while let Some(object) = objects.pop() {
            let shown = name(Some(&object.path));
            match undo_ops::verify(&object, fence, &mut budget, cancel) {
                Err(Refusal::Cancelled) => {
                    objects.push(object);
                    was_cancelled = true;
                    break;
                }
                Err(Refusal::Skip(reason)) => {
                    skipped += 1;
                    push_detail(&mut details, format!("{shown}: {reason}"));
                }
                Ok(()) => match context.trash_expected(
                    &object.path,
                    object.identity.device,
                    object.identity.inode,
                    cancel,
                ) {
                    Ok(receipt) => trashed.push((object, receipt)),
                    Err(e) if e.cause.kind() == std::io::ErrorKind::Interrupted => {
                        objects.push(object);
                        was_cancelled = true;
                        break;
                    }
                    Err(e) => {
                        skipped += 1;
                        push_detail(
                            &mut details,
                            format!(
                                "{shown}: no se envió a la Papelera ({}); se conserva",
                                trash_history::cause_message(&e.cause)
                            ),
                        );
                        for d in trash_history::failure_details(&e) {
                            push_detail(&mut details, d);
                        }
                    }
                },
            }
        }
        let done = trashed.len();
        let remaining = objects.len();
        if remaining > 0 {
            self.undo.push(Entry::Created {
                kind,
                objects,
                fence,
            });
        }
        if done > 0 {
            match undo_ops::fence() {
                Ok(trash_fence) => self.redo.push(Entry::Trashed {
                    kind,
                    items: trashed,
                    fence: trash_fence,
                }),
                Err(e) => {
                    push_detail(&mut details, format!("Rehacer no disponible: {e}"));
                    self.redo.clear();
                }
            }
        }
        Outcome {
            success: skipped == 0 && !was_cancelled,
            message: summary(
                "Deshecho",
                label,
                done,
                "en la Papelera",
                skipped,
                remaining,
            ),
            details,
            undo: false,
            redo: false,
        }
    }

    /// Redo of a creation: restore each receipt (never replacing an occupant).
    fn restore_trashed(
        &mut self,
        kind: Kind,
        mut items: Vec<(Created, Receipt)>,
        fence: Stamp,
        cancel: &AtomicBool,
        label: &str,
    ) -> Outcome {
        let mut restored = Vec::new();
        let mut details = Vec::new();
        let mut skipped = 0usize;
        let mut stopped = false;
        let mut budget = undo_ops::SCAN_BUDGET;
        while let Some((object, receipt)) = items.pop() {
            if cancel.load(Ordering::Acquire) {
                items.push((object, receipt));
                stopped = true;
                break;
            }
            let shown = name(Some(&object.path));
            // Unchanged while it sat in the trash? Otherwise it is restored
            // (that never destroys anything) but leaves this history.
            let pristine = receipt.trashed_path().is_ok_and(|slot| {
                undo_ops::verify(
                    &Created {
                        path: slot,
                        ..object.clone()
                    },
                    fence,
                    &mut budget,
                    cancel,
                )
                .is_ok()
            });
            match trash::restore(&receipt, cancel) {
                Ok(report) => {
                    for path in &report.cleanup_paths {
                        push_detail(&mut details, format!("Metadatos pendientes: {path:?}"));
                    }
                    match report.restored_path {
                        Some(path) if pristine => restored.push(Created { path, ..object }),
                        Some(_) => push_detail(
                            &mut details,
                            format!("{shown}: cambió en la Papelera; restaurado sin Deshacer"),
                        ),
                        None => push_detail(
                            &mut details,
                            format!("{shown}: restaurado; ruta no resuelta, sin Deshacer"),
                        ),
                    }
                }
                Err(e) if e.restore_retryable => {
                    push_detail(
                        &mut details,
                        format!("{shown}: {}", trash_history::cause_message(&e.cause)),
                    );
                    items.push((object, receipt));
                    stopped = true;
                    break;
                }
                Err(e) => {
                    skipped += 1;
                    push_detail(
                        &mut details,
                        format!(
                            "{shown}: no se pudo restaurar ({}); revisa la Papelera",
                            trash_history::cause_message(&e.cause)
                        ),
                    );
                    for d in trash_history::failure_details(&e) {
                        push_detail(&mut details, d);
                    }
                }
            }
        }
        let done = restored.len();
        let remaining = items.len();
        if remaining > 0 {
            self.redo.push(Entry::Trashed { kind, items, fence });
        }
        if done > 0 {
            match undo_ops::fence() {
                Ok(fence) => self.undo.push(Entry::Created {
                    kind,
                    objects: restored,
                    fence,
                }),
                Err(e) => push_detail(&mut details, format!("Deshacer no disponible: {e}")),
            }
        }
        Outcome {
            success: skipped == 0 && !stopped,
            message: summary("Rehecho", label, done, "restaurados", skipped, remaining),
            details,
            undo: false,
            redo: false,
        }
    }

    fn move_items(
        &mut self,
        mut items: Vec<Moved>,
        redo: bool,
        cancel: &AtomicBool,
        label: &str,
    ) -> Outcome {
        let mut done = Vec::new();
        let mut details = Vec::new();
        let mut skipped = 0usize;
        let mut was_cancelled = false;
        while let Some(item) = items.pop() {
            let shown = name(Some(&item.to));
            match undo_ops::move_back(&item, redo, cancel) {
                Ok(_) => done.push(item),
                Err(Refusal::Cancelled) => {
                    items.push(item);
                    was_cancelled = true;
                    break;
                }
                Err(Refusal::Skip(reason)) => {
                    skipped += 1;
                    push_detail(&mut details, format!("{shown}: {reason}"));
                }
            }
        }
        let moved = done.len();
        let remaining = items.len();
        if remaining > 0 {
            let entry = Entry::Moved { items };
            if redo {
                self.redo.push(entry);
            } else {
                self.undo.push(entry);
            }
        }
        if moved > 0 {
            done.reverse();
            if redo {
                self.undo.push(Entry::Moved { items: done });
            } else {
                self.redo.push(Entry::Moved { items: done });
            }
        }
        Outcome {
            success: skipped == 0 && !was_cancelled,
            message: summary(
                if redo { "Rehecho" } else { "Deshecho" },
                label,
                moved,
                "movidos",
                skipped,
                remaining,
            ),
            details,
            undo: false,
            redo: false,
        }
    }
}
fn summary(
    verb: &str,
    label: &str,
    done: usize,
    what: &str,
    skipped: usize,
    remaining: usize,
) -> String {
    let mut text = format!("{verb}: {label} · {done} {what}");
    if skipped > 0 {
        text.push_str(&format!(" · {skipped} conservados"));
    }
    if remaining > 0 {
        text.push_str(&format!(" · {remaining} pendientes (cancelado)"));
    }
    text
}

/// The Undo record of a finished copy/move job, from its own report.
/// `Ok(None)`: nothing to record. `Err(reason)`: shown to the user.
pub fn from_copy(
    outcome: &crate::copy_job::Outcome,
    unknown: bool,
) -> Result<Option<Record>, &'static str> {
    if outcome.extracting || outcome.creating || outcome.archive_action.is_some() {
        return Ok(None);
    }
    if unknown || outcome.supervisor_state_unknown() {
        return Err("no se puede deshacer: estado sin confirmar");
    }
    if outcome.moving && outcome.exdev.is_some() {
        return Err("no se puede deshacer entre discos");
    }
    let Some(report) = outcome.report() else {
        return Ok(None);
    };
    if report.completed_count == 0 {
        return Ok(None);
    }
    if report.completed_count > MAX_OBJECTS as u64 {
        return Err("no se puede deshacer: más de 64 elementos");
    }
    if outcome.moving {
        if report.completed_omitted > 0 || report.moved.len() as u64 != report.completed_count {
            return Err("no se puede deshacer: movimiento sin recibos completos");
        }
        return Ok(Some(Record::Moved {
            items: report.moved.clone(),
        }));
    }
    if report.completed_omitted > 0 || report.created.len() as u64 != report.completed_count {
        return Err("no se puede deshacer: copia sin recibos completos");
    }
    let Some(fence) = outcome.fence else {
        return Err("no se puede deshacer: sin marca de finalización");
    };
    Ok(Some(Record::Created {
        kind: Kind::Copy,
        objects: report.created.clone(),
        fence,
    }))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    //! Private, exclusive and retained fixtures under the invocation's TMPDIR.
    //! Nothing here deletes a tree; symlink fixtures point inside the fixture.
    //! "Nothing was permanently removed" is asserted by finding every undone
    //! object in the test's own trash (`<fixture>/Trash/files`).
    use super::*;
    use files_core::copy_engine::{self, Control, CopyOptions, CopyRequest};
    use std::{fs, os::unix::fs::symlink, path::PathBuf};

    fn fixture(label: &str) -> PathBuf {
        use std::sync::atomic::AtomicU64;
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "files-undo-history-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        path.canonicalize().unwrap()
    }
    fn go() -> AtomicBool {
        AtomicBool::new(false)
    }
    /// Names and kinds of what sits in the fixture's trash.
    fn trashed(root: &Path) -> Vec<(String, fs::FileType)> {
        let mut out: Vec<_> = fs::read_dir(root.join("files"))
            .map(|d| {
                d.map(|e| {
                    let e = e.unwrap();
                    (
                        e.file_name().to_string_lossy().into_owned(),
                        e.file_type().unwrap(),
                    )
                })
                .collect()
            })
            .unwrap_or_default();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }
    fn in_trash(root: &Path, prefix: &str) -> Option<PathBuf> {
        trashed(root)
            .into_iter()
            .find(|(n, _)| n.starts_with(prefix))
            .map(|(n, _)| root.join("files").join(n))
    }
    fn record_new(history: &mut History, kind: Kind, path: &Path, tree: bool) {
        let object = Created::just_created(path, tree).unwrap();
        history.apply(Command::Record(Box::new(Record::Created {
            kind,
            objects: vec![object],
            fence: undo_ops::fence().unwrap(),
        })));
    }
    fn copy_outcome(requests: &[CopyRequest], moving: bool) -> crate::copy_job::Outcome {
        let result = if moving {
            files_core::move_engine::PreparedMove::capture(requests.to_vec(), &go())
                .unwrap()
                .execute(&Control::default(), |_| {})
                .unwrap()
        } else {
            copy_engine::copy_paths(
                requests,
                &CopyOptions::default(),
                &Control::default(),
                |_| {},
            )
            .unwrap()
        };
        crate::copy_job::Outcome {
            exdev: None,
            moving,
            extracting: false,
            creating: false,
            archive_action: None,
            id: 1,
            result: Ok(result),
            progress: Default::default(),
            fence: (!moving).then(|| undo_ops::fence().unwrap()),
        }
    }
    fn record(history: &mut History, outcome: &crate::copy_job::Outcome) {
        let record = from_copy(outcome, false).unwrap().unwrap();
        history.apply(Command::Record(Box::new(record)));
    }

    #[test]
    fn undo_new_folder_sends_it_to_trash_and_redo_restores_it() {
        let d = fixture("folder");
        let root = d.join("Trash");
        let mut h = History::default();
        fs::create_dir(d.join("Carpeta nueva")).unwrap();
        record_new(&mut h, Kind::NewFolder, &d.join("Carpeta nueva"), false);
        assert_eq!(
            h.labels().0.as_deref(),
            Some("Deshacer crear carpeta «Carpeta nueva»")
        );
        let out = h.undo(&root, &go());
        assert!(
            out.success && !out.undo && out.redo,
            "{} {:?}",
            out.message,
            out.details
        );
        assert!(!d.join("Carpeta nueva").exists());
        let slot = in_trash(&root, "Carpeta nueva").expect("folder is in the trash");
        assert!(slot.is_dir());
        assert_eq!(
            h.labels().1.as_deref(),
            Some("Rehacer crear carpeta «Carpeta nueva»")
        );
        let out = h.redo(&go());
        assert!(
            out.success && out.undo && !out.redo,
            "{} {:?}",
            out.message,
            out.details
        );
        assert!(d.join("Carpeta nueva").is_dir());
        assert!(trashed(&root).is_empty());
        // The restored folder is undoable again (fresh fence after restore).
        assert!(h.undo(&root, &go()).success);
        assert!(in_trash(&root, "Carpeta nueva").is_some());
    }

    #[test]
    fn folder_the_user_added_a_file_to_is_skipped_and_intact() {
        let d = fixture("folder-used");
        let root = d.join("Trash");
        let mut h = History::default();
        fs::create_dir(d.join("nueva")).unwrap();
        record_new(&mut h, Kind::NewFolder, &d.join("nueva"), false);
        fs::write(d.join("nueva/mine.txt"), b"user data").unwrap();
        let out = h.undo(&root, &go());
        assert!(!out.success && !out.undo && !out.redo);
        assert!(
            out.details
                .iter()
                .any(|l| l.contains("contiene elementos nuevos")),
            "{:?}",
            out.details
        );
        assert_eq!(fs::read(d.join("nueva/mine.txt")).unwrap(), b"user data");
        assert!(trashed(&root).is_empty());
    }

    #[test]
    fn undo_copy_trashes_only_the_copies_and_redo_restores_them() {
        let d = fixture("copy");
        let root = d.join("Trash");
        fs::create_dir_all(d.join("src/tree/sub")).unwrap();
        fs::write(d.join("src/tree/sub/a.bin"), b"alpha").unwrap();
        fs::write(d.join("src/file.txt"), b"bravo").unwrap();
        fs::write(d.join("target.txt"), b"link target").unwrap();
        symlink(d.join("target.txt"), d.join("src/link")).unwrap();
        fs::create_dir(d.join("dst")).unwrap();
        let requests: Vec<_> = ["tree", "file.txt", "link"]
            .into_iter()
            .map(|n| CopyRequest {
                source: d.join("src").join(n),
                destination: d.join("dst").join(n),
            })
            .collect();
        let outcome = copy_outcome(&requests, false);
        let mut h = History::default();
        record(&mut h, &outcome);
        assert_eq!(h.labels().0.as_deref(), Some("Deshacer copiar 3 elementos"));
        let out = h.undo(&root, &go());
        assert!(out.success, "{} {:?}", out.message, out.details);
        assert_eq!(fs::read_dir(d.join("dst")).unwrap().count(), 0);
        // Every copy is in the trash, nothing was unlinked.
        assert_eq!(trashed(&root).len(), 3);
        let link = in_trash(&root, "link").unwrap();
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_link(&link).unwrap(), d.join("target.txt"));
        assert_eq!(
            fs::read(in_trash(&root, "tree").unwrap().join("sub/a.bin")).unwrap(),
            b"alpha"
        );
        // Sources and the link target are byte-identical and untouched.
        assert_eq!(fs::read(d.join("src/tree/sub/a.bin")).unwrap(), b"alpha");
        assert_eq!(fs::read(d.join("src/file.txt")).unwrap(), b"bravo");
        assert_eq!(fs::read(d.join("target.txt")).unwrap(), b"link target");
        assert!(
            fs::symlink_metadata(d.join("src/link"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        let out = h.redo(&go());
        assert!(out.success, "{} {:?}", out.message, out.details);
        assert_eq!(fs::read(d.join("dst/tree/sub/a.bin")).unwrap(), b"alpha");
        assert_eq!(fs::read(d.join("dst/file.txt")).unwrap(), b"bravo");
        assert_eq!(
            fs::read_link(d.join("dst/link")).unwrap(),
            d.join("target.txt")
        );
        assert!(trashed(&root).is_empty());
    }

    #[test]
    fn edited_copy_and_replaced_inode_are_kept() {
        let d = fixture("copy-edited");
        let root = d.join("Trash");
        fs::create_dir_all(d.join("src/dir")).unwrap();
        fs::create_dir(d.join("dst")).unwrap();
        for n in ["a", "b", "c"] {
            fs::write(d.join("src").join(n), n.as_bytes()).unwrap();
        }
        fs::write(d.join("src/dir/inner"), b"inner").unwrap();
        let requests: Vec<_> = ["a", "b", "c", "dir"]
            .into_iter()
            .map(|n| CopyRequest {
                source: d.join("src").join(n),
                destination: d.join("dst").join(n),
            })
            .collect();
        let outcome = copy_outcome(&requests, false);
        let mut h = History::default();
        record(&mut h, &outcome);
        // a: edited in place. b: replaced by a new inode. dir: a file inside edited.
        fs::write(d.join("dst/a"), b"edited by the user").unwrap();
        fs::rename(d.join("dst/b"), d.join("b-aside")).unwrap();
        fs::write(d.join("dst/b"), b"new inode").unwrap();
        fs::write(d.join("dst/dir/inner"), b"changed").unwrap();
        let out = h.undo(&root, &go());
        assert!(!out.success);
        let text = out.details.join("\n");
        assert!(
            text.contains("a: modificado después; se conserva"),
            "{text}"
        );
        assert!(text.contains("b: reemplazado por otro objeto"), "{text}");
        assert!(text.contains("dir: contenido modificado después"), "{text}");
        assert_eq!(fs::read(d.join("dst/a")).unwrap(), b"edited by the user");
        assert_eq!(fs::read(d.join("dst/b")).unwrap(), b"new inode");
        assert_eq!(fs::read(d.join("b-aside")).unwrap(), b"b");
        assert_eq!(fs::read(d.join("dst/dir/inner")).unwrap(), b"changed");
        assert!(!d.join("dst/c").exists());
        assert_eq!(trashed(&root).len(), 1);
        assert_eq!(fs::read(in_trash(&root, "c").unwrap()).unwrap(), b"c");
    }

    #[test]
    fn undo_move_returns_objects_and_never_overwrites_an_occupied_name() {
        let d = fixture("move");
        fs::create_dir_all(d.join("a/folder")).unwrap();
        fs::create_dir(d.join("b")).unwrap();
        fs::write(d.join("a/one"), b"one").unwrap();
        fs::write(d.join("a/folder/x"), b"x").unwrap();
        let requests: Vec<_> = ["one", "folder"]
            .into_iter()
            .map(|n| CopyRequest {
                source: d.join("a").join(n),
                destination: d.join("b").join(n),
            })
            .collect();
        let outcome = copy_outcome(&requests, true);
        assert!(d.join("b/one").exists() && !d.join("a/one").exists());
        let mut h = History::default();
        record(&mut h, &outcome);
        assert_eq!(h.labels().0.as_deref(), Some("Deshacer mover 2 elementos"));
        let out = h.undo(&d.join("Trash"), &go());
        assert!(out.success, "{} {:?}", out.message, out.details);
        assert_eq!(fs::read(d.join("a/one")).unwrap(), b"one");
        assert_eq!(fs::read(d.join("a/folder/x")).unwrap(), b"x");
        assert!(fs::read_dir(d.join("b")).unwrap().next().is_none());
        let out = h.redo(&go());
        assert!(out.success, "{} {:?}", out.message, out.details);
        assert_eq!(fs::read(d.join("b/one")).unwrap(), b"one");
        // The original name is now occupied: skipped, nothing overwritten.
        fs::write(d.join("a/one"), b"occupant").unwrap();
        let out = h.undo(&d.join("Trash"), &go());
        assert!(!out.success);
        assert!(
            out.details
                .iter()
                .any(|l| l.contains("ocupado; no se reemplazó")),
            "{:?}",
            out.details
        );
        assert_eq!(fs::read(d.join("a/one")).unwrap(), b"occupant");
        assert_eq!(fs::read(d.join("b/one")).unwrap(), b"one");
        assert_eq!(fs::read(d.join("a/folder/x")).unwrap(), b"x");
        assert!(!d.join("Trash").exists());
    }

    #[test]
    fn cross_filesystem_and_unconfirmed_jobs_are_not_undoable() {
        let d = fixture("exdev");
        let mut outcome = crate::copy_job::Outcome {
            exdev: Some(crate::move_operation::Context {
                job: [0; 16],
                proof: None,
                journals: d.join("journals"),
                source: d.join("a"),
                destination: d.join("b"),
                recovery: None,
                reviewed_phase: None,
            }),
            moving: true,
            extracting: false,
            creating: false,
            archive_action: None,
            id: 1,
            result: Ok(Default::default()),
            progress: Default::default(),
            fence: None,
        };
        assert_eq!(
            from_copy(&outcome, false).err(),
            Some("no se puede deshacer entre discos")
        );
        outcome.exdev = None;
        assert!(from_copy(&outcome, true).is_err());
        // A copy whose report lacks receipts for some roots is refused whole.
        outcome.moving = false;
        outcome.result = Ok(files_core::copy_engine::CopyReport {
            completed_count: 2,
            ..Default::default()
        });
        assert!(from_copy(&outcome, false).is_err());
    }

    #[test]
    fn redo_fails_when_the_original_name_is_occupied_and_never_replaces() {
        let d = fixture("redo");
        let root = d.join("Trash");
        let mut h = History::default();
        fs::write(d.join("nuevo.txt"), b"").unwrap();
        record_new(&mut h, Kind::NewFile, &d.join("nuevo.txt"), false);
        assert!(h.undo(&root, &go()).success);
        fs::write(d.join("nuevo.txt"), b"someone else").unwrap();
        let out = h.redo(&go());
        assert!(
            !out.success && out.redo && !out.undo,
            "{} {:?}",
            out.message,
            out.details
        );
        assert_eq!(fs::read(d.join("nuevo.txt")).unwrap(), b"someone else");
        assert!(in_trash(&root, "nuevo.txt").is_some());
        fs::rename(d.join("nuevo.txt"), d.join("aside.txt")).unwrap();
        let out = h.redo(&go());
        assert!(out.success && out.undo, "{} {:?}", out.message, out.details);
        assert_eq!(fs::read(d.join("nuevo.txt")).unwrap(), b"");
        assert_eq!(fs::read(d.join("aside.txt")).unwrap(), b"someone else");
    }

    #[test]
    fn history_is_bounded_and_trash_invalidation_drops_receipts() {
        let d = fixture("bound");
        let root = d.join("Trash");
        let mut h = History::default();
        for i in 0..(MAX_ENTRIES + 5) {
            let p = d.join(format!("f{i}"));
            fs::write(&p, b"").unwrap();
            record_new(&mut h, Kind::NewFile, &p, false);
        }
        assert_eq!(h.undo.len(), MAX_ENTRIES);
        assert_eq!(
            h.labels().0.as_deref(),
            Some("Deshacer crear archivo «f24»")
        );
        // Oversized records are refused instead of truncated.
        let objects = vec![Created::just_created(&d.join("f0"), false).unwrap(); MAX_OBJECTS + 1];
        h.apply(Command::Record(Box::new(Record::Created {
            kind: Kind::Copy,
            objects,
            fence: undo_ops::fence().unwrap(),
        })));
        assert_eq!(
            h.labels().0.as_deref(),
            Some("Deshacer crear archivo «f24»")
        );
        assert!(h.undo(&root, &go()).success);
        assert!(h.can_redo());
        h.apply(Command::InvalidateTrash);
        assert!(!h.can_redo() && h.can_undo());
        // The trashed file stays in the trash: invalidation never erases.
        assert!(in_trash(&root, "f24").is_some());
        assert_eq!(h.receipts(), 0);
    }

    #[test]
    fn worker_records_new_folder_and_undoes_it_through_the_trash() {
        use crate::rename_job::{Request, Response, Worker};
        let d = fixture("worker");
        let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from);
        let Some(data) = data.filter(|p| p.starts_with(std::env::temp_dir())) else {
            // Only with the private XDG_DATA_HOME of the documented test run.
            return;
        };
        let name = format!("undo-worker-{}", d.file_name().unwrap().to_string_lossy());
        let mut worker = Worker::new();
        let run = |worker: &mut Worker, request: Request| -> crate::rename_job::Outcome {
            assert!(worker.submit(request).is_ok());
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            loop {
                if let Some(Response::Finished(_, outcome, _)) = worker.poll() {
                    return outcome;
                }
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        };
        let out = run(
            &mut worker,
            Request::Create {
                id: 1,
                directory: d.clone(),
                name: name.clone(),
                folder: true,
            },
        );
        assert!(out.success && out.undo, "{} {:?}", out.message, out.details);
        assert_eq!(
            worker.labels().0,
            Some(format!("Deshacer crear carpeta «{name}»"))
        );
        let out = run(&mut worker, Request::History { id: 2, redo: false });
        assert!(
            out.success && !out.undo && out.redo,
            "{} {:?}",
            out.message,
            out.details
        );
        assert!(!d.join(&name).exists());
        assert!(in_trash(&data.join("Trash"), &name).is_some());
        let out = run(&mut worker, Request::History { id: 3, redo: true });
        assert!(out.success && out.undo, "{} {:?}", out.message, out.details);
        assert!(d.join(&name).is_dir());
    }

    #[test]
    fn window_records_a_copy_and_ctrl_z_sends_it_to_the_trash() {
        let Some(data) = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.starts_with(std::env::temp_dir()))
        else {
            return;
        };
        let d = fixture("window");
        fs::create_dir(d.join("dst")).unwrap();
        let unique = format!("copied-{}", d.file_name().unwrap().to_string_lossy());
        fs::write(d.join(&unique), b"payload").unwrap();
        let outcome = copy_outcome(
            &[CopyRequest {
                source: d.join(&unique),
                destination: d.join("dst").join(&unique),
            }],
            false,
        );
        let mut m = crate::model::Model::new(d.clone());
        let pump = |m: &mut crate::model::Model, done: &dyn Fn(&crate::model::Model) -> bool| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while !done(m) {
                m.pump_mutations();
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        };
        m.record_undo(Command::Record(Box::new(
            from_copy(&outcome, false).unwrap().unwrap(),
        )));
        pump(&mut m, &|m| m.can_undo && !m.mutations.busy);
        assert_eq!(
            m.history_labels().0.as_deref(),
            Some("Deshacer copiar 1 elemento")
        );
        m.undo_rename(false);
        pump(&mut m, &|m| !m.mutations.busy && !m.mutation_applied);
        assert!(m.can_redo && !m.can_undo, "{:?}", m.operation_message);
        assert!(!d.join("dst").join(&unique).exists());
        assert_eq!(fs::read(d.join(&unique)).unwrap(), b"payload");
        let slot = in_trash(&data.join("Trash"), &unique).expect("copy is in the trash");
        assert_eq!(fs::read(slot).unwrap(), b"payload");
    }
}
