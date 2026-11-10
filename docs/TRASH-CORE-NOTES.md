# Linux Trash: partial core

2026-10-26. Partial Linux integration, visible in Actions/Delete and Undo. Functional and unit results are recorded below; resources and startup NOT_MEASURED because of the user's explicit suspension. It adds no dependencies and does not declare O04 complete.

The [freedesktop Trash Specification 1.0](https://specifications.freedesktop.org/trash/1.0/) defines the main trash at `$XDG_DATA_HOME/Trash`, with `files` and `info` directories. Each object has a unique name and a matching `.trashinfo`. The metadata is reserved first with exclusive creation; `Path` stores the path bytes URL-escaped and `DeletionDate` uses local date/time. This engine applies that format to an explicit main trash root and, since 2026-11-10, to per-volume trashes (see below). It never interprets a stored name as a restore path.

## Per-volume trash (2026-11-10)

`TrashContext::trash` routes each item by the device of its (canonical) parent directory. Same device as the home trash root: the home trash, exactly as before. Otherwise the volume topdir of that parent is used:

1. `$topdir/.Trash/$uid` when `$topdir/.Trash` is a real directory (checked with `AT_SYMLINK_NOFOLLOW`, never a symlink) with the sticky bit, owned by root or by the effective user, and the opened descriptor is the same inode that was checked. `$uid` is created `0700` when missing.
2. Otherwise `$topdir/.Trash-$uid`, created with mode `0700`.

`files/` and `info/` are created `0700`; existing modes are never changed and must still be exactly `0700`/user-owned. `Path=` is written relative to the topdir and percent-encoded with the same lossless encoder; before writing, the topdir descriptor must be an ancestor of the pinned parent descriptor and the original path must lie under the topdir path. Every other property is unchanged: fd-rooted operations, `O_NOFOLLOW`, exclusive metadata, `RENAME_NOREPLACE`, unique names, identity checks and rollback rules. The real `st_dev` of the object and of the chosen `files/` must still match; a mismatch or `EXDEV` keeps the origin intact. There is no copy fallback and nothing is deleted on failure. A read-only volume (`EROFS`) or a trash that cannot be created (permission, insecure topdir) fails with a Spanish message that names the volume, says why and states that the origin is intact.

Volume lookups are injectable through the `Volumes` trait (`device`, `topdir`, `mounted`). `SystemVolumes` uses `st_dev`, the topmost same-device ancestor walked through `..` descriptors (GLib-style; btrfs subvolume boundaries count as volumes, matching `EXDEV`), and a bounded `/proc/self/mountinfo` read via `mount_table::parse`. `HomeOnly` disables volume trashes. Tests use a fake implementation over private directories: nothing is mounted, no root is needed and no real mount point, home or trash is touched. The UI uses `HomeOnly` in unit tests and when `FILES_TEST_PREVIEW` is set, so scripted sessions never list or empty the trash of real volumes. A batch caches at most 16 volume contexts (four FDs each: root, files, info, topdir); each receipt pins the context it used, so Undo of a batch restores per-volume items through the same descriptors.

`catalog_all` lists the home trash plus the existing trashes of mounted local volumes under one 4096-name / 4 MiB budget. Mount types that are network, FUSE (except `fuseblk`), pseudo, automount or read-only images are filtered before any `stat`. Listing never creates a directory; missing, insecure or unreadable volume trashes are skipped, and the same trash reached twice (bind mounts) is listed once. Each `CatalogItem` carries `volume: Option<Volume>`; `restore_volume_item` and `purge_volume_item` accept only the two spec locations of the effective user under the given topdir, and a relative `Path=` resolves against that topdir (absolute keys are accepted too; relative keys in the home trash are rejected). `empty_all` empties the home trash and every listed volume trash.

Ancestor policy change: a root- or user-owned directory with the sticky bit (such as `/tmp`, `/var/tmp` or a spec `.Trash`) is now accepted as a non-immediate ancestor even though it is world-writable, because others cannot rename or remove entries we own there. The immediate parent of the home trash must still be user-owned and not group/other writable. This also lets the trash tests and the `rename_job` trash test run under a private `TMPDIR` below `/var/tmp`.

Not verified: no real second filesystem, USB drive, read-only mount or vfat/exfat/NTFS volume was exercised. Read-only behaviour is covered by the error mapping and an unwritable topdir fixture, not by `EROFS` from a real mount.

## API

- `TrashContext::open(root) -> Result<TrashContext, Failure>`: opens or creates only `root`, `root/files` and `root/info`. The parent of `root` must exist.
- `context.trash(source, cancel) -> Result<Receipt, Failure>` and the convenience `trash(source, root, cancel)` for one object.
- `context.trash_expected(source, device, inode, cancel)`: the same, but refused before anything is written unless the object pinned at `source` (final symlink not followed) is exactly that inode; the pre-commit identity check then proves the renamed object is it. Undo uses only this entry point ([UNDO.md](UNDO.md)).
- `restore(&receipt, cancel) -> Result<RestoreReport, Failure>`: returns the object to the **original parent's descriptor**, even if that directory was renamed. The original final name is kept, without replacing any new occupant.
- `Receipt::original_path()` is historical and matches the metadata. `current_original_path()`, `trashed_path()` and `info_path()` resolve current locations through `/proc/self/fd` and verify identities. A resolution error is never turned into an old path presented as rescue.
- `Failure` keeps `cause`, `preserved_paths`, `cleanup_paths`, `rollback_errors`, `unresolved_paths` and `restore_retryable`. The latter starts as `false` and, when a restore fails, only becomes `true` if a fresh check finds both the expected object and its expected metadata still in their Trash slots. The error type does not decide the retry; any error while checking identities keeps `false`. Paths of preserved objects and of leftover metadata have different roles; the caller must show them.
- `RestoreReport` confirms that the restore move and the object's identity were already verified. `restored_path: Option<PathBuf>` contains `Some(verified_current_path)` or `None` if only the diagnostic resolution after the commit fails. In that case `unresolved_paths` increases and `cleanup_errors` explains the failure, without inventing a historical path or returning a move error. The caller must remove that receipt from the history on any `Ok(report)`, even with warnings, and keep showing the diagnostics. Cleaning up its own `.trashinfo` is attempted anyway. `cleanup_paths/cleanup_errors` also report leftover metadata; each leftover path that cannot be resolved increases `unresolved_paths`.

The context is `Clone` through `Arc` and shares three FDs: root, `files`, `info`. Each `Receipt`, without `Clone`, keeps three FDs: original parent, `O_PATH` object and metadata. The last two avoid confusing an inode reuse with the confirmed object. A UI batch of 64 receipts with a shared context holds roughly 195 FDs, on top of whatever the process already has. The caller must bound the batch and the history; this API does not promise 100000 receipts nor an unbounded history. Dropping the receipt removes nothing from the trash.

## Security and limits

The root, `files` and `info` must belong to the effective UID and have mode exactly `0700`; the created `.trashinfo` files are regular, single-link and `0600`. Existing permissions are not silently fixed. Each managed component is walked without following symlinks. Ancestors must belong to root or to the effective UID and must not be group/other writable; the immediate parent must belong to the user. A `~/.local/share` parent with mode `0755` is valid. `create_dir_all` is not used to create unchecked ancestors. The caller must safely prepare `$XDG_DATA_HOME` if it is missing and pass that root; the library does not infer the global configuration or use personal data in tests.

Input paths are at most 4096 bytes, absolute, without NUL, `.` or `..`; `/` is not accepted. The source's parent is resolved once, keeping the last component as the object without following its symlink. The source can be a file, a non-empty directory or a symlink; `renameat2(RENAME_NOREPLACE)` keeps the object and its content/metadata when moving it. A source inside this trash, or one that is the root or one of its ancestors, is rejected, also checking directory identities by FD.

Before moving, the exclusive metadata and its directory are written/synced. Context, object and metadata are revalidated, and cancellation is checked right before the move. Cancellation is cooperative: it does not interrupt a blocked syscall; after a confirmed commit it returns success. The later check detects observed substitutions. Recovery uses exclusive moves and never overwrites to recover. If it cannot prove a current path, it keeps an unresolved count instead of inventing one.

Linux has no general "rename/unlink only if this inode is still here" syscall. A concurrent actor can change the interval between check and syscall; identities and FDs do not turn the whole sequence into a conditional transaction. Processes with the same UID can modify a `0700` folder. This is not isolation against a hostile actor with the same credentials and does not guarantee a snapshot of content modified in place. Rollback failures are exposed; an absolute absence of races is not claimed.

There is no copy fallback. A different device or `EXDEV` produces a clear error and keeps the source. Per-volume trashes are described above; redo and crash recovery are pending. User content is never permanently deleted. Interrupting the process can leave an orphaned metadata file or an object pending recovery; the earlier `fsync` calls are not a journal nor atomicity against power loss. The optional `directorysizes` cache is not implemented.

Stored names use a bounded raw prefix of the original name plus PID/sequence; up to 128 exclusive candidates are tried. `.trashinfo` stays below the typical Linux `NAME_MAX`. Original names stay lossless in the `Receipt` and in the percent-encoded `Path`, including Unicode, `%`, line breaks and non-UTF-8 bytes. Diagnostic text is never treated as an operable path.

A single-object operation keeps constant state: metadata under 13 KiB with a 4096-byte path, bounded names and at most four diagnostic paths per collection of 8192 bytes and messages of 1024 bytes. These are bounds on retained data, not a process PSS measurement nor a UI batch budget. The resource/startup measurement phase has not been opened.

## Test coverage

The Linux tests use only their own exclusive folders under the invocation's private `TMPDIR` (formerly `~/.cache/files-test`) and retain them; they no longer delete fixture trees. Per-volume cases: a cross-volume item goes to `<topdir>/.Trash-<uid>` with a relative percent-encoded `Path=` and Undo restores it; a valid sticky `.Trash/<uid>` is preferred; a symlinked or non-sticky `.Trash` is rejected; the merged listing, restore, purge and empty of a volume trash; listing never creates a trash; an uncreatable volume trash leaves the origin intact with the error; a symlink item is trashed as a link with its target untouched. Earlier cases: content/round trip, non-UTF-8 names and escaping, no-clobber on restore, symlinks not followed, non-empty directory, cancellation before commit, source substitution, renamed/replaced parent, relocated trash root, replaced object/metadata, repeated names, occupied destination, permissions/ancestors and cleanup that rejects foreign metadata. The pure tests cover format/escaping and preflight. The integrated suite and the UI scene are verified serially at the root. EXDEV needs an isolated fixture on a second filesystem, never a personal mount/data by inference.

## Fixing the diagnostic after restore

The integration review found a static case: after moving the object and confirming its identity, a failed `/proc/self/fd` resolution could return `Err`. The history would then keep a receipt whose location in `files` was already empty. The fix returns `Ok(RestoreReport { restored_path: None, unresolved_paths: 1, ... })` for that diagnostic failure, continues the revalidated cleanup of the metadata and keeps an explicit warning. A confirmed commit is not rolled back just because a display path is missing. Errors before the commit and identity failures after it are still `Failure` with recovery/diagnostics.

Regression prepared with an injected resolver: inside the callback it checks that the object is already in the original parent with the expected identity, that the Trash slot is empty and that the metadata still exists; it forces `PermissionDenied`; it requires `Ok`, path `None`, one unresolved location and the later revalidated removal of its own metadata. The regression is part of the core's integrated suite.

## External substitution during restore

If after the move the original parent's name holds a different identity, that foreign occupant is not moved to the Trash as a supposed rollback: a substitution before the syscall cannot be told apart from one after it. The observed occupant stays where it is, the original object is located through its `O_PATH` FD, and the metadata is kept for manual recovery. The report shows the verified current locations, without running new relocations on foreign identities.

`Failure.restore_retryable` is computed when leaving the restore wrapper from the current identities of both slots, not from `AlreadyExists`/cancellation. A normal conflict and cancellation keep `true`; a moved/replaced object, replaced metadata or an unavailable check leave `false`. The UI must separate retryable receipts from receipts kept only for recovery, keeping the total bounded and without offering Undo for a stale token. Fixtures were added for substitution after the move and for cancellation with metadata replaced afterwards, and the asserts of the earlier pre-operation replacement case were extended. No Cargo, processes or measurements were run during this adjustment.

## Visible integration and bounded recovery

Actions/Delete sends the selection to the single mutation worker; it does not open or probe
the filesystem on the UI thread. The borrowed preflight rejects batches of more than 64
objects, repeated names or more than 256 KiB before cloning. It never silently truncates a selection.
Since 2026-11-10 a batch with at least one confirmed commit is pushed onto the
window's linear Undo history ([UNDO.md](UNDO.md)) instead of replacing it;
earlier failures keep Undo. A failed creation does not clear the history either. There is no
Trash redo nor atomic rollback of a batch: receipts of the confirmed objects
are kept and errors and rescue paths are shown.

Undo restores in reverse order. An occupied destination keeps the new occupant and
allows retrying as long as object and metadata are still the expected ones. A
non-retryable receipt is set aside for manual recovery, without offering a stale Undo;
it holds its FDs until the next confirmed mutation or close. The total of
usable and set-aside receipts is still limited to 64. Close cancels and drains the
worker's result before exiting. The functional scene verifies a normal close
after ACK; it does not test closing during a blocked syscall nor crash recovery.

Errors use a panel with vertical scroll and horizontal scrolling through
Shift+wheel or the arrows. Debug paths are diagnostic, including non-UTF-8 ones;
they are never reinterpreted as instructions. Success uses a compact status.

The review found an important symmetry: both when sending and when restoring,
an external substitution after the move never authorizes moving the foreign
occupant as a supposed rollback. Only returning the expected inode is attempted, with
a fresh check and an exclusive destination; if it is no longer in its slot, everything is kept
in its observed location and manual recovery is reported. A regression leaves
the original name empty so that no-clobber cannot hide the wrong case.
The check→syscall interval against another process with the same UID remains explicit.

## Integrated verification

314 Rust tests (98 core, 37 helper, 179 UI), Clippy workspace/all-targets
`-D warnings` and `cargo build --release` PASS. Final scene
`trash-ui-1793045192155795906.json` functional PASS for a batch of four, conflict and
retry, raw non-UTF-8, directory/symlink and rejection of 65 items; normal close
after ACK. Exact captures and limits in VALIDATION. Resources/startup NOT_MEASURED.
