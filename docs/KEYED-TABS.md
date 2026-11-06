# Toolkit keyed tab strip

Files uses `KeyedTabBar<(pane_id, tab_id)>` from the exact toolkit export
`a029d45`, based on the keyed API in `89632dc`. Titles are presentation;
raw paths, tab snapshots, history and admission remain owned by Files.
Optional GPU, accessibility and media features remain disabled.

## Interaction

Primary clicks select on release. Dragging reorders without first activating
the source tab; dropping outside or pressing Escape cancels the gesture.
Middle clicks request closure of the indicated tab. Right clicks on the
body or close hit open its context without activation. Overflow scrolls
within the widget, preserving the current browser.

`Ctrl+Alt+T` focuses the tab cursor; Escape returns to the browser without
clearing its selection. Left/Right/Home/End select; Alt+Left/Right moves only
the cursor; Ctrl+Shift+Left/Right reorders the cursor tab; Enter activates it;
Delete closes it; Menu/Shift+F10 opens its context. Ctrl+Shift+F10 always
targets the active tab. Keyboard-origin menus restore strip focus, while
browser-origin menus restore browser focus. Existing Ctrl+Tab cycling and
Ctrl+Shift+PageUp/PageDown active-tab ordering remain available.

The toolkit fix in `a029d45` explicitly reveals a repeatedly selected key.
This covers Home after moving the independent cursor to an overflowed last
tab while the first tab remains active.

## Ownership and rendering

Widget callbacks enqueue bounded requests and never borrow the app model or
rewrite the selected State during notification. Each request carries the
displayed pane/active-tab owner and listing generation. Draining before the
session pump preserves user intent against late startup restoration.
Missing/foreign/stale keys, covered surfaces and active edits or mutations
reject requests. Consecutive selection requests from one display stamp keep
the final intent. Rejected local selection changes are reconciled to the
accepted app state on the next paint.

The app synchronizes the ListModel and selected State under a notification
guard. Stable keys preserve toolkit item identity across labels and moves.
The existing snapshot controller admits all tab closures and navigation;
the strip never removes snapshots itself.

A background-owning Group clears its own rectangle before drawing the
transparent toolkit control, including animation timer frames. Its default
pre-draw is disabled. The toolkit owns indicator timing, easing and the
reduced-motion policy. Files exports actual widget body/close geometry after
drawing and refreshes it after independent scrolling or cursor movement.
The native window bridge remains in place. Auxiliary strip events reach the
widget's release handler once; other auxiliary browser gestures retain their
existing route. The toolkit focus router is installed for native keyboard
mode and focus rings.

## Acceptance

`scripts/keyed-tabs-nested-check.py` exercises native pointer/key delivery
only on an isolated nested Wayland socket. It records actual mapped sizes
separately from requested sizes. `scripts/tab-strip-motion-ui-check.py`
checks eight frozen-clock rendered snapshots: intermediate indicator bounds,
rapid reversal, a single underline, settled ON/OFF equality and disabled
motion reaching its endpoint in the input frame. Full rendered snapshots
do not independently establish partial-damage screen behavior.

Model tests cover request ownership, stale targets, bounded input,
programmatic synchronization and selection bursts. Existing context,
keyboard, Sidebar, preview and native transfer acceptance remain relevant
regressions after focus-router adoption. Installation evidence records the
exact app/helper hashes and all five installed/backed-up artifacts per
machine. Native macOS/Windows, full accessibility/i18n, tear-off and the
remaining Nautilus/Finder roadmap are separate open requirements. Resource
and startup gates remain `NOT_MEASURED`.
