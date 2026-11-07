# Asynchronous presentation toolkit export

Files exports the exact committed toolkit tree
`fac0705d25eabc43f34d62ab94ee2b9e48902a75`, following the prior exact
`767235b79d9f6fd5ef9b20495585d5d2e21cc141` export. All 443 baseline files
and the Files-owned `UPSTREAM.json` record matched their recorded bytes and
modes before modification. The new tree contains 467 upstream files: 17
modified paths, 24 additions and no removals. Files owns the additional
provenance record, for 468 vendor files in total.

The export preserves all 26 tracked upstream receipts, including the 13
new async-presentation receipts, and the xdg-activation license. These
upstream records retain their original scope and ordering, including checks
before and after the TLS-exit fix. They establish source provenance rather
than Files application acceptance. All 22 prior executable modes remain;
three new async probe scripts make 25 files with Git mode `100755`. The
remaining 442 upstream files use `100644`.

`vendor-manifest.json` records the source commit/tree, the reproducible Git
archive SHA-256, all path changes and each file's Git blob ID, SHA-256, byte
count and mode. The archive path set, committed blob identities, final vendor
path set, bytes and modes were verified. Every upstream file came from that
commit's archive; no working-tree, untracked, Git metadata or build-target
content was copied. No vendor patch was applied. Upstream Cargo files and
Files workspace Cargo files remain unchanged. Optional GPU, accessibility,
images, playback, QR and Mermaid features remain disabled; this adoption
retains the existing CPU rendering configuration.

## Presentation API and integration boundary

The native facade now exports
`begin_present(&mut window, PresentOptions) -> Result<PresentRequest, PresentError>`.
The UI-owned handle provides `poll`, `cancel` and `is_pending`, is neither
`Send` nor `Sync`, tracks the target's deletion and owns retained startup
hints. Hints remain bounded and redacted; no environment token is imported,
modified or logged. One pending lease excludes another asynchronous request
and synchronous `present` on that UI thread. Completion/cancel/drop release
that lease once; cached terminal results never replay native activation.

Wayland captures native input context before mapping, revalidates the
connection and current seat, resolves a tracked live origin surface and
resolves the current target surface immediately before dispatch. Its
private-queue discovery and token phases are manually polled without a
compositor wait, each with a separate two-second elapsed deadline. The API
creates no internal timer or worker. The application must continue normal
UI turns while pending and revalidate ownership outside model borrows before
each poll; supersession or close must cancel/drop stale work.

Begin may map a hidden window but does not send activation before polling.
Cancel/drop before dispatch prevent activation; they cannot undo normal
mapping or recall terminal `Requested`. Wayland has no focus-grant reply,
so `Requested` is never evidence that the compositor focused the window.
The original synchronous API remains available. X11's native roundtrips
still occur synchronously in the first poll; other backends likewise defer
their existing native call rather than promise fully asynchronous OS APIs.
Windows/macOS typechecks are not native runtime validation.

The same committed tree includes the lazy modal `overlay::Sheet` API from
`8be476bdfc66d921f611fe7fa8ed8d487912198c`, its documentation, example and
tests. Files has not adopted Sheet. Its inclusion in an exact source export
does not change Files dialog dismissal, draft persistence or operation flows.

This export verifies provenance and source/API consistency only. It does not
run Cargo, open a GUI, validate the Files adapter, install a release or claim
M2/Windows/macOS native acceptance. Files must record its own delayed pending
progress, focus/ownership, stale/cancel/close, token forwarding and per-machine
validation separately before release and installation.
