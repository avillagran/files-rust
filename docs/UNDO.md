# Undo and Redo of operations

2026-11-10. Ctrl+Z / Ctrl+Shift+Z (the existing `Cmd::Undo` / `Cmd::Redo`
chords, both shortcut profiles) act on one linear, in-memory history per
window. The Actions menu and the command palette name the top entry, for
example "Deshacer copiar 3 elementos" or "Rehacer crear carpeta «Fotos»"; the
compact status keeps its Deshacer/Rehacer button and, after an Undo or Redo,
shows the result ("Deshecho: copiar 3 elementos · 3 en la Papelera"). The
history does not survive closing the window.

The owner's rule drives the whole design: **nothing is ever deleted by
inference**. Undo never unlinks or removes a tree; every object it touches was
recorded, with its identity, by the operation that created or moved it, and is
re-verified right before it is acted on.

## What is undoable

| Operation | Undo | Redo |
|---|---|---|
| Rename / bulk rename | existing transaction token (unchanged) | token again |
| Send to trash (batch ≤ 64) | restore from the receipts (unchanged) | none (Redo history is cleared) |
| New folder, new empty file | send exactly that inode to the trash | restore it from its trash receipt |
| New document from `~/Templates` | same | same |
| Create link(s) (Ctrl+M, Shift+Ctrl+M) | send the links (as links) to the trash | restore them |
| Copy (completed, partial or cancelled job) | send the roots the job reports as created to the trash | restore them |
| Move inside one filesystem (native engine, single or multi-parent batch) | rename each root back with no-replace | rename it forward again with no-replace |

Not undoable, deliberately:

- **Cross-filesystem moves** (EXDEV engine, journal recovery): the notice says
  "no se puede deshacer entre discos". The reverse direction would have the
  copy-then-remove engine remove the destination copy, a permanent removal of
  an object the user may already consider theirs; the trash cannot take it
  back across devices either. That cannot be shown to meet the rule, so it is
  refused rather than approximated.
- Archive extraction and compression, "Nueva carpeta con la selección",
  permission changes (they clear the history, as before), Shift+Delete and the
  trash catalog's own actions.
- Any copy or move with more than 64 roots, or whose report lacks an identity
  receipt for some completed root ("no se puede deshacer: …"); the operation is
  refused whole rather than partially recorded under a misleading label.
- A job whose final state is unconfirmed (worker lost/panicked, uncertain
  roots).

## Safety rules as implemented

Line numbers refer to the commit that introduced this document.

1. **Undo never deletes.** Undoing a creation calls only
   `TrashContext::trash_expected` (`crates/ui/src/undo_history.rs:457`), the
   existing fd-rooted trash path plus an identity precondition
   (`crates/core/src/trash.rs:234`, check at `trash.rs:1473`). The trash
   engine's own pre-commit check (`trash.rs:1555`) proves that the inode it
   renames into `files/` is that same object. Redo restores through
   `trash::restore` (`undo_history.rs:558`), which never replaces an occupant.
   No code path in the history calls `unlink`, `remove_file` or
   `remove_dir_all`; tests assert that every undone object is present in the
   test's private trash.
2. **Only provably created objects, identified at creation time.** Receipts
   are `undo_ops::Created` / `undo_ops::Moved`
   (`crates/core/src/undo_ops.rs`) holding (device, inode, type) and the
   canonical path:
   - copy: taken from the engine's own held root handle at publication and
     pushed next to `CopyReport.completed`
     (`crates/core/src/copy_engine.rs:1537`, same cap);
   - native move: from the pinned source/destination parent descriptors
     (`crates/core/src/move_engine.rs:404`, pushed at `:523`);
   - new folder/file: `fstat` of the handle the worker created
     (`crates/ui/src/rename_job.rs:1006` and `:1025`);
   - template: `fstat` of the new file's handle after its contents were
     synced (`crates/core/src/quick_ops.rs`, `from_template_recorded`);
   - links: `lstat` of the name right after `symlink(2)`
     (`crates/ui/src/delete_ui.rs:136`).
   Names are never re-derived. A copy is recorded only when every completed
   root has a receipt (`undo_history.rs:733`; moves `:726`), so keep-both,
   skipped, partial and cancelled jobs are undoable exactly for the roots the
   job reported as its own.
3. **Re-verification before each object** (`undo_ops::verify`,
   `undo_ops.rs:315`): same identity at the same path (`:332`), else "reemplazado
   por otro objeto; se conserva"; for a file, same size and mtime (`:341`);
   for every object, no inode change after the operation's completion fence
   (`:335`). A new folder must still be empty (`:358`, "contiene elementos
   nuevos; se conserva"). Anything that fails is skipped, listed in the notice
   as "modificado después; se conserva" (or the specific reason) and leaves
   the history; the rest of the entry still proceeds.
4. **Never through symlinks.** Parents are opened with `O_NOFOLLOW`
   (`undo_ops.rs:151`), every stat is `AT_SYMLINK_NOFOLLOW`, directories are
   entered only with `O_NOFOLLOW | O_DIRECTORY` and re-checked by identity
   (`undo_ops.rs:160`, scan at `:380`). A copied symlink is trashed as a link;
   its target is never examined.
5. **Copied trees: bounded proof.** A copied directory may contain entries
   the job created, so the scan (`undo_ops.rs:380`) checks that every entry's
   `st_ctime` is not newer than the job's fence, stays on the root's device
   (`:402`) and fits an explicit budget of 20,000 entries per Undo (`:394`,
   `SCAN_BUDGET`); beyond the budget or depth 128 the root is kept ("demasiado
   grande para verificar"). The fence (`undo_ops::fence`, `:255`) is taken on
   the copy worker right after the job (`crates/ui/src/copy_job.rs:794`): it
   reads the fine wall clock and waits until the coarse clock that stamps
   inode changes has passed it, so any later write, chmod, rename into/out of
   a directory or link-count change gets a newer ctime. Only roots are
   recorded (≤ 64 per entry), never every file.
6. **Undo of a move** (`undo_ops::move_back`, `undo_ops.rs:435`): both parent
   directories must still have their recorded identities (`:450`), the object
   at the moved path must be the recorded inode, the original name must be
   free, and the rename is `renameat2(RENAME_NOREPLACE)` (`:483`). An occupied
   name is "el nombre de destino está ocupado; no se reemplazó"; nothing is
   merged or overwritten. Redo is the same function in the other direction.
7. **Redo only when deterministic and non-destructive.** Creations are redone
   by restoring the exact trashed object from its receipt (stricter than
   re-creating a lookalike: the same inode returns, and the trash slot is not
   left behind). Before restoring, the object in the trash is verified
   against a fence taken right after Undo trashed it (`undo_history.rs:546`);
   if it changed there it is still restored (that destroys nothing) but no
   longer offered for Undo. A retryable conflict (name occupied) keeps the
   Redo entry and reports it. Moves are redone with no-replace.
8. **Bounded and invalidated.** 20 entries, 64 objects per entry, 128 trash
   receipts in total (about 3 descriptors each; the oldest receipt holders are
   evicted first) (`undo_history.rs:26-30`, `bound` at `:206`). Emptying the
   trash, deleting or restoring an item from the catalog drops every entry that
   holds receipts (`delete_ui.rs:394` → `undo_history.rs:270`); unmounting or
   ejecting a volume drops the whole history (`devices_ui.rs:156`, `:160` →
   `undo_history.rs:269`). Neither invalidation changes anything on disk.
9. **Off the UI thread.** The history lives in the existing mutation worker.
   Completed operations reach it as non-filesystem `Request::Record` commands
   that never mark the worker busy (`rename_job.rs:596`); Undo/Redo run as the
   existing cancellable `Request::History` (`rename_job.rs:694`) with the
   usual "Aplicando cambios… / Cancelar tarea" overlay. Cancellation stops
   between objects: what was done moves to the other stack, the rest stays.
   The final notice lists done, kept and pending counts plus one line per
   kept object. Undo waits for running copies, deletions and unflushed
   records (`mutations.rs:200`).

## Known limits (not hidden)

- Linux has no "rename only if this inode is still here" syscall. Identity
  is checked immediately before each syscall (and by the trash engine inside
  its own commit), but a same-UID process racing in that window is not
  excluded; this matches the trash core's documented model.
- The ctime proof assumes the wall clock does not jump backwards between the
  operation and the Undo. A backwards jump could let a later change look old.
- A new folder's receipt is taken by opening the name just after `mkdirat`
  and links by `lstat` just after `symlink(2)`; an actor replacing the name in
  that microsecond window would be recorded instead (its later Undo would
  still only trash a verified, unchanged object).
- Undo of a trash batch still has no Redo.
- Cross-filesystem behaviour of the volume trash was exercised only with the
  trash core's simulated volumes; no real second filesystem was mounted.

## Tests

`crates/core/src/undo_ops.rs` and `crates/ui/src/undo_history.rs`, private
exclusive fixtures under the invocation's `TMPDIR`, never deleted, symlink
targets inside the fixture: new folder → trash → redo; folder with a user
file kept; copy of a tree, a file and a symlink → only the copies in the
trash, sources byte-identical, link trashed as a link with its target
untouched; edited copy, replaced inode and edited file inside a copied tree
kept; move undo/redo and an occupied original name kept; cross-filesystem and
receipt-less jobs refused; redo blocked by an occupant then succeeding;
history bound, oversized records refused, trash invalidation; the worker and a
window (`Model`) recording a copy and undoing it with Ctrl+Z through the
private `XDG_DATA_HOME` trash.
