# Keyed toolkit adoption: 89632dc plus a029d45

Files exports the exact committed upstream checkpoint
`a029d45c1fc63f99763f9836d1af0c817ab82ffd` from `lib-fltk-omarchy`.
The keyed-tab feature base is `89632dc7cbcef39d1da8cf680b6b235e9cc4be58`;
`a029d45` adds the repeated-activation reveal fix and plain-FLTK frame
validation required by this integration.
This is an untagged upstream checkpoint, not a claim of a published release.
The vendor has no local toolkit patches. `UPSTREAM.json` is the sole
Files-owned provenance file in the vendor directory.

## Export verification

All 424 upstream files match the committed archive byte for byte, including
their executable modes. The initial keyed export added 65 files and changed
32 relative to `0d00f2b`. The exact `a029d45` export adds one more file and
changes five relative to `89632dc`; neither step removed upstream files.
The provenance update is additional to those upstream deltas.

- Committed archive SHA-256:
  `30355d72634a107cc0f9c529b7ce33a2a23d2ef074ae28f24a46609fea514289`.
- Sorted file manifest SHA-256 (each line is `mode sha256 path`):
  `bf48d0550201c97aa95f7590d33f2b4827da2a0f7027bb91180aec68beee0a41`.
- Both prior vendor exports were preserved before replacement. Export
  verification manifests and backups are under `/var/tmp/files-keyed/`.

The runtime dependency and feature graph is unchanged, so the Files
workspace lockfile needs no update. The upstream Cargo manifest only adds
a GPU-gated example. Files keeps GPU, accessibility, images, playback, QR
and Mermaid features disabled; default rendering remains software.

## Available APIs and adoption scope

`TabBar::from_model` exposes a `KeyedTabBar<K>` backed by stable domain keys,
including `(pane_id, tab_id)`. Close, context and reorder callbacks request
application operations without first selecting or removing a tab. Reorder
uses insertion-before keys. Geometry APIs return visible body/close targets
and hit identities in window logical coordinates, including clipping,
scrolling and RTL layout. An independent keyboard cursor can request
reordering without switching the active browser.

Repeated activation of the already-selected tab now reveals it even after
cursor-only navigation has scrolled it out of view. A plain FLTK host must
own the strip's background: clear its surface and then draw visible children
on every group draw, including timer frames, with `super_draw(false)`. The
host must preserve the toolkit widget's own draw callback. This is the
documented pattern for avoiding stale hover/animation pixels in Files'
existing native window.

The toolkit also contains newer native-dialog/portal support and retained
CPU/GPU scene infrastructure. Exporting those modules does not mean Files
adopts those features or replaces its existing dialogs, previews or native
window bridge.

Files integration must preserve the existing guarded snapshot controllers,
pane ownership, session-restore invalidation and canvas behavior. Widget
requests need validation outside synchronous `State` notifications and
drawing; exported geometry must reflect the widget's completed draw/reveal.
Auxiliary strip events must be delivered once rather than also closing
through the legacy canvas handler.

## Acceptance boundaries

Upstream private X11 CPU input reports exercise right-click body/close,
middle close, pointer and keyboard reordering, primary selection and repeated
Home activation revealing an overflowed active tab without another selection
change. A separate plain-FLTK regression compares the mapped RGB buffer
against a fresh render at hover and start/middle/end animation frames and
checks that frame scheduling stops when motion settles.
Those reports establish toolkit behavior, not Files integration, native
Wayland delivery, M2 acceptance or complete accessibility.

Files must pass its own unit/Clippy/build checks, handler/framebuffer scenes,
focus and native-input checks before installation. NODE and native ARM M2
must each be validated; all five installed artifacts and their backups must
be hashed. Resource and startup gates remain `NOT_MEASURED`, and the complete
Nautilus/Finder, language, accessibility and platform roadmap remains open.
