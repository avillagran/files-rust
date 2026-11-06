# Stable tab context and order — 2026-11-06

App source: `1c2cdee`. Keyboard readiness harness: `8ae0de8`.
Toolkit vendor: exact `0d00f2b`; no uncommitted keyed widget exported.

| Check | NODE | M2 native ARM |
|---|---|---|
| Context source, animations on/off | PASS | PASS |
| Installed context, animations on | PASS | PASS |
| Corrected keyboard matrix, installed | PASS | PASS |
| Five installed and backup artifact hashes | PASS | PASS |
| HTML helper recipe | verified/reused | verified/reused |

Workspace: 723 core + 112 helper + 837 UI = 1,672 passed, 3 ignored.
Clippy and native release builds pass. NODE breadcrumb and toolkit menu
regressions pass. Reports identify exact app/helper/HTML helper hashes.
`node/artifact-audit.json` and `m2/artifact-audit.json` verify installed and
backup hashes against the receipts.

The reports under each machine's `docs/evidence/` retain their original
absolute machine paths. The corresponding filenames/captures are preserved
at the same relative paths in this bundle. Only private fixtures were used.
Context and keyboard scenes exercise production handlers, toolkit popup/key
routes and actual framebuffers; they do not prove native OS input delivery.

Retained failures:

- NODE menu `1794000063121040218`: concurrent Xvfb startup lost the display
  (`X I/O error`), before submenu capture. Explicit display `:174` passed
  with the same app (`1794000102439808729`). No product fix was made for it.
- M2 keyboard `1794000196133924035`: initial watch refresh overlapped a key;
  generation changed 1→2 and guarded navigation preserved selection.
  Harness `8ae0de8` waits for watch/active-pane readiness before each action
  and uses the newest complete record. Source and installed runs then pass
  without replay or production changes.

Resource and startup gates remain `NOT_MEASURED`. Drag reordering, toolkit
tab-strip migration, native keyboard/accessibility acceptance and the wider
Nautilus/Finder roadmap remain open.
