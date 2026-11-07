# Navigation, delayed shortcuts and cache reuse — 2026-11-07

The production source checkpoint is `dd80534ce42b7a8831e65a4d9c12f4951e8bf1e3`.
The separate native acceptance driver is `1d5a5c0`, refined in `2cdd732`;
it changes no product source.

The later [native titlebar integration](NATIVE-TITLEBAR-FORMAT.md) replaces the
literal client prefix described by this initial feedback checkpoint. M2 now uses
the compositor's app prefix and styling; NODE's final plugin deployment awaits
system authentication.

## Behavior

- The client title is `Files > <path>`, followed by brief operation state when
  needed. The existing native icon, desktop entry and `files` app ID remain the
  application identity. Compositor title formatting is an independent layer.
- Toolbar controls retain their animated name labels. Their canvas tooltip
  contains only the keyboard shortcut, after a fresh two-second hover on each
  control. Moving to another control restarts the delay; input and exit hide it.
  No toolkit-wide delay or desktop setting is changed.
- Up navigates to the actual parent independently of Back/Forward history;
  `Alt+Up` invokes the same action. It is disabled at the filesystem root.
- Preferences > View can disable both image and video thumbnails. Turning them
  back on first checks the source and reuses unchanged RAM pixels. Restarting
  can reuse the persistent, bounded cache without decoding again.

## Size observations

Completed automatic measurements survive leaving the viewport in a bounded
128-row queue. A separate 256-entry, one-MiB path ledger can reuse observations
across directory navigation. A re-entering visible folder receives a cheap
identity/mtime probe on the IO worker rather than an unconditional recursive
scan. Fresh unchanged observations are reusable for five minutes. Persistent
observations remain approximate and cannot order rows as confirmed sizes.

Real events from watched visible subfolders invalidate the affected observation,
including known cached ancestors and descendants. Merely dropping and reattaching
a watch requests metadata, not another recursive scan. Explicit Refresh also
invalidates the current root. Watches are non-recursive and bounded; an unseen
deep change is not promised to appear immediately, and the five-minute reuse
limit bounds that observation's lifetime. These sizes are not filesystem snapshots
or authority for mutations.

## Thumbnail observations

See [MEDIA-THUMBNAILS.md](MEDIA-THUMBNAILS.md) for byte limits and source proof.
The disk cache compares full raw path, device/inode, size, mtime/ctime with
nanoseconds, recipe, dimensions, age and checksum. Source changes miss the old
record. Missing sources cannot display cached pixels. Unvisited stale slots are
bounded and expire; deletion does not trigger a global sweep of the disk.

A size job pauses new thumbnail admission without discarding displayed demand
and verified pixels. The single-helper exclusion remains. First-time thumbnails
in a mixed folder can still wait behind queued size work; fair alternation is a
separate scheduling improvement.

## Validation scope

Thirty-three reviewed, narrowly selected Rust tests pass with retained private
fixtures, as do workspace/all-targets check and strict Clippy. No full legacy
suite, toolkit incident harness, CI, resource budget or startup claim is included.
Modified Rust files pass scoped rustfmt. A workspace format-only check reports
pre-existing module ordering in untouched `crates/core/src/lib.rs`; it is retained
as a formatting failure and was not folded into the frozen candidate.
Four private native source profiles and two installed profiles pass on NODE and
M2, including narrow layouts. All five backups and installed artifacts are
hash-verified per machine, with configuration and association checks unchanged.
See [the evidence bundle](evidence/feedback-cache-20261107-dd80534/README.md) for
receipts, retained failed attempts, source provenance and precise scope.

Sidebar dragging remains disabled pending the toolkit contract in
[TOOLKIT-SIDEBAR-REQUEST.md](TOOLKIT-SIDEBAR-REQUEST.md). The requested toolkit
thread is `omarchy-thread-e31bf060-0c36-4330-a4c4-2782fc6a872b`; connected
thread tools returned `Authentication required`, so delivery is unconfirmed.
