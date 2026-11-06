# UI implementation sequence

The active goal prioritizes usability, animation quality and a coherent UI,
with Nautilus and Finder as behavioral references. The existing mockup v3
and file-operation safety contracts remain constraints. Toolkit adoption is
not complete merely because the vendor builds.

## 1. Navigation and motion

Audit Sidebar expansion and browser displacement together. Files currently
uses the binary expanded state to choose 52/170 px during every paint;
verify whether this overrides the toolkit's intermediate animation widths.
Use the toolkit width State and motion gate where supported rather than
adding a second animation timer. Check hover, pin, rapid reversal, window
resize, modal coverage and animations disabled. Row hit targets must match
what is painted throughout a transition. Preserve bookmark ordering and
real Wayland drops.

## 2. Everyday navigation and selection

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
