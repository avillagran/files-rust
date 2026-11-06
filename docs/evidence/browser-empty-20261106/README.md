# Pane-owned empty states — 2026-11-06

App source `acdc119`, private fixture `b3d7a92`, toolkit vendor `0d00f2b`.

| Check | NODE | M2 native ARM |
|---|---|---|
| Private source scenes | PASS | PASS |
| Installed scenes | PASS | PASS |
| Installed keyboard matrix | PASS | PASS |
| Five installed and backup artifact hashes | PASS | PASS |
| HTML helper recipe | verified/reused | verified/reused |

Workspace: 723 core + 112 helper + 840 UI = 1,675 passed, 3 ignored.
Clippy, three focused Painter regressions and native release builds pass.
The source/installed empty-state scenes each have 28 states, 9 captures,
two normal closures and intact private fixtures. They cover List/Grid
no-match filter clearing without reload, empty List/Grid/Columns, and each
active empty-pane position beside a populated sibling with preserved raw
listing and selection, at 800×440 and 440×440.

Painter regressions assert body-message count and bounds, including errors
and pending listings. Framebuffer captures were visually checked for scoped
messages and duplicate removal. These scenes use production handlers and
framebuffers; they do not establish native OS input delivery, accessibility
or complete Nautilus/Finder parity. No new failure was observed in this
delivery. Resource/startup gates remain `NOT_MEASURED`.

Machine bundles retain original report paths. Their files are mirrored at
the corresponding relative paths. `artifact-audit.json` verifies every
installed/backup hash against the receipt. No user windows, associations or
desktop settings were changed.
