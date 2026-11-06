# Keyed toolkit tabs — 2026-11-06

Source: `a0f1fff` + startup-intent/readiness follow-up `e47cf81`.
Exact toolkit export: `a029d45c1fc63f99763f9836d1af0c817ab82ffd`, based on
`89632dc`; 424 upstream files plus Files' `UPSTREAM.json`, with no vendor
patches. Export manifest is retained in `node/vendor-export.json`.
Optional GPU/a11y/media features remain disabled.

| Acceptance | NODE x86_64 | M2 native ARM Linux |
|---|---|---|
| Source native tabs/focus/startup intent | PASS | PASS |
| Source 8-frame motion gate | PASS | PASS |
| Installed native tabs/focus/startup intent | PASS | PASS |
| Installed keyboard matrix | PASS | PASS |
| Installed private preview copy | PASS | PASS |
| Installed private Sidebar drop/copy review | PASS | PASS |
| Five installed and backup artifact hashes | PASS | PASS |
| Native HTML helper recipe | verified/reused | verified/reused |

Workspace: 723 core + 112 helper + 844 UI = 1,679 passed, 3 ignored.
Clippy, formatting and both native release builds pass. NODE context and
keyboard handler regressions also passed at the preceding `a0f1fff`
candidate; final `e47cf81` installed keyboard/native checks are separately
retained. Each machine's `artifact-audit.json` checks all five installed and
backup hashes against its receipt. No existing user windows, associations
or desktop settings were changed.

## Scope

Native reports cover right body/close context without activation, middle
body/close, drag reorder and outside cancellation, cursor-only focus,
keyboard reorder/context, origin focus return, primary/Enter activation,
global cycling, dual-pane Tab/arrows and compound keys. Home must reveal an
already-active first tab after cursor-only scrolling to the last. A native
click on an unchanged active key and native Ctrl+Alt+T both supersede pending
session restoration without navigating or changing browser selection.

Actual mapped native framebuffers are 508×325 on NODE and 810×449 on M2.
The requested size is not evidence of a different mapped size. Fixed
1000×650 full-window rendering checks eight snapshots for an intermediate
underline, reversal, single accent run, endpoint agreement and identical
settled ON/OFF strip pixels. These do not independently prove native
partial-damage ghost prevention. The app follows the upstream-tested host
pattern: clear its Group's background, draw the transparent child and
disable the Group's default pre-draw. Visual review found legible labels
and menus; pointer-origin menus intentionally overlap the strip at the click
position. Keyboard-origin menus anchor below it.

This delivery does not establish native macOS/Windows input, screen-reader
acceptance, full i18n, tear-off or complete Nautilus/Finder parity. Resource
and startup gates remain `NOT_MEASURED`.

## Preserved failures and resolutions

- NODE `2qsxekab`: drag driver aimed before the destination tab's midpoint;
  no move was the correct result. The driver now crosses its midpoint.
- NODE `mzvlyq4k`: fixture requested canvas focus while holding the model
  borrow needed by its Focus handler. Focus preparation moved outside that
  borrow; native Escape was independently checked.
- NODE `ysmio6jh`: real Ctrl+Alt+T reached the focused canvas's generic Ctrl+T
  handling before the window interceptor, creating a tab. The canvas now
  intercepts the explicit focus chord first. Focused-strip Ctrl+Shift+F10 is
  also distinguished from cursor Menu context at the widget callback.
- NODE `9n59k0hu`: the exact already-active Home/overflow case failed with
  toolkit `89632dc`. Upstream `a029d45` reveals and redraws on repeated
  activation; final native NODE/M2 gates pass without skipping this case.
- M2 `f_i9s3qh`: the driver read its prior cursor state before native Home
  arrived. It now waits for the first cursor key before applying the same
  strict visibility assertion. No key is replayed and no guard is disabled.

Earlier report hashes identify their own intermediate binaries; these
failures are not reclassified as passing final runs. `a0f1fff` was installed
on NODE during integration, then superseded by final `e47cf81`; its receipt
and backup chain remain recorded.

Reports preserve original absolute machine paths. Machine `docs/evidence/`
files are mirrored at their relative paths; native run directories are under
`native/`. All payloads and clipboard/compositor activity belong to private
fixtures. Running and failed private evidence was retained.
