# UI implementation sequence

The active goal prioritizes usability, animation quality and a coherent UI,
with Nautilus and Finder as behavioral references. The existing mockup v3
and file-operation safety contracts remain constraints. Toolkit adoption is
not complete merely because the vendor builds.

## 1. Navigation and motion

Shared sampled Sidebar/browser geometry is delivered in `de480ce`; see
[SIDEBAR-MOTION.md](SIDEBAR-MOTION.md). It uses the toolkit clock and motion
gate rather than a separate timer. Frame validation covers intermediate
widths, rapid reversal, resize, pinning, modal coverage and motion disabled;
widget edges and browser/entry hit targets remain aligned.

Continue auditing hover intent, focus changes and transitions between
navigation surfaces. Preserve bookmark ordering and real Wayland drops.

## 2. Everyday navigation and selection

Complete hidden breadcrumb ancestry is delivered in `74d205f`, including
keyboard opening, focus return and background destinations; see
[BREADCRUMB-OVERFLOW.md](BREADCRUMB-OVERFLOW.md). The keyboard fixture now
checks inclusive/additive SpaceMap ranges against the current contract.

Background folder tabs and middle-close gestures are delivered in `c52b96b`;
see [TAB-GESTURES.md](TAB-GESTURES.md). Stable menu/keyboard reordering and
inactive-tab context actions are delivered in `1c2cdee`; see
[TAB-CONTEXT.md](TAB-CONTEXT.md). The keyed toolkit strip and drag reordering
are implemented in `a0f1fff`/`e47cf81`; see [KEYED-TABS.md](KEYED-TABS.md).
Tear-off and broader platform/accessibility acceptance remain open.

Explicit FileManager1 service mode and the select/folders/properties CLI are
implemented; see [FILEMANAGER1.md](FILEMANAGER1.md). Requests preserve exact
path identity, use cancellable helper preflight and reject stale view/modal
ownership before UI admission. Automatic desktop activation/default-provider
registration and startup-token focus handling remain open. Initial local
libdbus connection/authentication has no hard total deadline.

Reconcile the existing Nautilus parity matrix with current implementation;
several entries predate Sidebar, recent files and AccessKit in the toolkit.
Validate keyboard navigation, selection, focus return, inline rename,
breadcrumbs and tabs across list, grid, columns and dual pane. Prioritize
observable failures over adding more commands. Preserve raw path identity.

## 3. Surfaces and feedback

Unify modal focus, Escape, menu placement, loading/empty/error states and
operation feedback. Adopt toolkit controls where they replace duplicated
behavior without losing keyboard or mutation safety. Verify narrow windows,
long names and scaling. Do not add unrelated preview engines.

Empty browser ownership is delivered in `acdc119`: the global fallback no
longer overlays a populated neighboring pane or duplicates Columns messages.
Loading, errors, empty folders and no-match filters have distinct scoped
states. See [BROWSER-EMPTY-STATES.md](BROWSER-EMPTY-STATES.md).

## 4. Language and accessibility

Use toolkit i18n and accessible descriptions systematically. Language
selection must cover views, menus, errors and plurals; do not claim complete
i18n for a translated toolbar. Canvas accessibility requires explicit
virtual rows and actions, not just enabling a Cargo feature.

## 5. Acceptance and installation

Each delivery records scope and remaining gaps. Run relevant unit checks,
Clippy, release build, private UI fixtures and animation frame assertions.
Use hidden nested compositors or Xvfb; do not drive the user's windows.
Build ARM on M2, validate it, preserve all five installed artifacts and
hashes, install atomically per artifact, and check the installed binary.
Keep performance/startup status separate from functional acceptance and
respect the existing freeze on resource experiments.

The complete project roadmap remains open. This sequence organizes UI work;
it does not replace the operation, interoperability or platform requirements.

## Toolkit review at f02aacc

The upstream review found native dialogs/portal support (`4eef3bd`) and
retained CPU/GPU scene surfaces (`c3c46ab` through `f02aacc`). These are
committed upstream capabilities, not adopted Files functionality. The
current exact `0f68089` vendor export includes those capabilities, but they
are not enabled as Files features merely by exporting that source.

Adopt dialogs through explicit user actions and preserve existing path,
mutation-review and close/cancel contracts. Review scene-backed motion
separately; do not silently enable GPU/effects or remove resource gates.
The keyed strip adoption below connects the existing tab controllers rather
than replacing their snapshot or mutation contracts. Wider accessibility and
other usability work remains independent of the strip.

## Keyed strip adoption at a029d45

Upstream `89632dc` now supplies composite-key tabs, close/context/reorder
requests, an independent keyboard cursor and authoritative hit geometry.
The follow-up `a029d45` fixes reveal when activating an already-selected key
after independent cursor scrolling. Files exports that committed tree
exactly in the original keyed-strip delivery; `0f68089` retains that fix
with optional GPU/effects disabled. Its own acceptance is recorded in
[KEYED-TABS.md](KEYED-TABS.md), separately from upstream X11 evidence.

The adapter queues requests outside drawing and synchronous State
notifications, validates pane/generation/key ownership, and restores accepted
selection after rejected admission. It exports actual widget hit geometry,
delegates strip auxiliary events once, preserves the native window bridge
and cancels late startup restoration even for unchanged selection/focus.
A background-owning Group clears transparent controls on every child frame;
the toolkit supplies indicator motion and its reduced-motion policy.

Continue native platform acceptance beyond NODE/M2 Linux, screen-reader and
language integration, tear-off and modal focus consistency across all views.
Rendered motion snapshots and mapped native input have distinct scopes;
resource/startup acceptance remains separate.

## Persistent feedback implemented at 0f68089

The exact committed `0f68089` export and the Files adapter are now
implemented in source; see [PERSISTENT-NOTICES.md](PERSISTENT-NOTICES.md).
One current feedback record supplies a bounded body and typed result
identity to one persistent `overlay::Notice`. Clean results and information
can remain non-modal; required error, recovery and truncated-report reviews
retain their existing modal/queue-hold behavior. Undo/Redo uses a successful
mutation origin and current worker history, and View report targets the
exact retained copy result. Informational cards offer acknowledgement only.

Callbacks enqueue at most eight requests and never borrow the model or
start operations. The main loop revalidates the current record and guards
before dispatch, with native card closure/focus restoration outside model
borrows. Ctrl+Alt+N focuses the card without activating it. The normal
runtime suppresses duplicate canvas feedback, preserving a fallback when
the toolkit anchor is unavailable. Dismissal does not discard the retained
copy report, which remains reachable through the menu and command palette.

Complete private native ON/OFF, narrow-layout, focus, actual Undo/Redo,
report and regression acceptance on NODE and M2 before recording a release
or installation result. Requested and actual mapped geometry must be
reported separately. Optional GPU/accessibility/media features remain off;
full language, accessibility and platform acceptance remain open.

## Next toolkit integration: native presentation

Committed upstream `767235b` now provides `native::present` and explicit
startup/activation hints. Its bounded Wayland private-queue handshakes do
not imply that a `Requested` outcome grants focus. The current Files vendor
remains `0f68089`; adopt `767235b` as a separate exact export after this
feedback delivery, then validate Files' token forwarding and native M2
behavior. Do not treat the existence of the upstream API as Files adoption
or claim that the current `show()` consumes a caller's startup ID.
