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

Background folder tabs and middle-close gestures are delivered in `c52b96b`;
see [TAB-GESTURES.md](TAB-GESTURES.md). Tab reordering and migration to the
toolkit strip remain open. The toolkit thread has the stable-key/context/
reorder requirements and the right-button activation finding.

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
vendor remains the verified `0d00f2b` checkpoint during this delivery.

Adopt dialogs through explicit user actions and preserve existing path,
mutation-review and close/cancel contracts. Review scene-backed motion
separately; do not silently enable GPU/effects or remove resource gates.
The toolkit TabBar already has an animated indicator, overflow, keyboard
cycling and middle-close, but migration must preserve stable tab identities,
right-button safety and pane ownership. Coordinate missing APIs upstream
before replacing the current strip. Breadcrumb overflow and the navigation
fixture reconciliation can proceed independently.
