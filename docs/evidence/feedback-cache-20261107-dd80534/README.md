# Feedback acceptance and installation — 2026-11-07

Production source: `dd80534ce42b7a8831e65a4d9c12f4951e8bf1e3`.
The initial driver is `1d5a5c0`, its final corrections are `2cdd732`, and the
installation backup check is `30bb484`. These later commits do not change the
compiled product. There is no global parity, startup or resource PASS.

## Accepted behavior

| Machine | Run | Requested window | Actual client area | Result |
| --- | --- | --- | --- | --- |
| NODE | `3d5115ad8ffd` source | 1000×650 | 1000×624 | PASS |
| NODE | `f174b8e9739c` source | 700×500 | 704×474 | PASS |
| M2 ARM Linux | `bf610faf0511` source | 1000×650 | 1000×624 | PASS |
| M2 ARM Linux | `5bc550c04c95` source | 700×500 | 704×474 | PASS |
| NODE | `db55ec511762` installed | 1000×650 | 1000×624 | PASS |
| M2 ARM Linux | `da8cae9c6a3e` installed | 1000×650 | 1000×624 | PASS |

Each run uses a fresh retained private HOME/config/data, session bus and headless
Sway compositor. Pointer and keyboard input verify the socket owner, compositor
PID and focused application before sending events. All six accepted compositors
stopped. Existing desktop windows were not controlled. Normal and narrow captures
were inspected for readable shortcuts, updated folder size and replacement image.

The checks cover real client title/app ID, Up from a fresh window without history,
shortcut-only tooltips before/after two seconds independently per control,
Preferences toggle with unchanged decode count when re-enabled, and reuse after
scroll. Folder00 remains at dispatch 1 after leaving and returning; a real visible
child creation causes dispatch 2 and changes 21 B to 4.1 kB without manual Refresh.
The restarted media scenario reads a persistent record with one disk hit and zero
decodes. Removing its source from the directory hides it; publishing a blue PNG
with a new revision produces one decode and new pixels. Grid supplies demand in
the narrow layout, where the inspector is hidden. The new media fixture is PNG;
this is not a new video playback acceptance run.

## Builds, checks and installation

- Thirty-three narrowly selected reviewed Rust tests pass, with retained private
  fixtures. Workspace/all-targets check and strict Clippy pass. No full legacy
  suite, toolkit incident harness or CI was executed.
- Modified Rust files pass rustfmt. Workspace formatting reports pre-existing
  module order in untouched `crates/core/src/lib.rs`; that failure is retained in
  `checks/format-scope.json` rather than silently changing the frozen source.
- NODE release: 81.999 s. App `c7a610fdc0f63a2e957b90768819022b8c09835adb9757e5dc9fba0f98adc90d`.
- M2 native ARM release: 120.478 s, 1052 exact source inputs checked before and
  after build, nine stale CMake cache entries preserved. App
  `de3d488a7a2d225abab9a0ed9502a3a39ac1adbf75482970618d406a6ae6b8f1`.
  No NODE executable was transferred to M2. Each existing native HTML helper
  matches the unchanged adapter recipe `babce060e9ec324af33f55b825a376e6a01213227bcd75e5227a8e95d2c69375`.
- Both installations hashed all five backups before the first atomic replacement.
  Independent audits match backups to prior installed hashes and installed files
  to accepted artifacts. NODE config remains absent and its MIME association file
  is unchanged; eight M2 preference/session/layout/association paths are unchanged
  before and after post-install acceptance. Receipts and audits are under each
  machine directory. Later repository HEADs identify tooling, separately from
  `binary_source_checkpoint`.

## Retained failures and practical limits

Six failed GUI profiles remain in this bundle. Two early clicks did not open
Preferences; their cause is unresolved. The final driver waits for stable hover
geometry, clicks the actual expanded control and waits for a real popup-open
receipt before sending keys. This proves that deliberate path, not universal
immediate-click behavior. Another run incorrectly required Home scroll to be 0;
Groups reveals the first entry at 28. Another expected a republished thumbnail in
Groups after removing its selection, which supplied no demand. Two narrow runs
also incorrectly assumed an inspector; using Grid supplies visible demand.
No product patch was made to disguise these failures. Initial compilation during
an in-progress size edit and Clippy lint failures are retained beside the final
successful checks.

Size watches are bounded and non-recursive. Unobserved deep changes are not
instantaneous; fresh observations have a five-minute reuse limit. Persistent size
values stay approximate. First-time thumbnails can still wait behind queued size
work. Missing unvisited media records are bounded/expired rather than globally
swept. Actual Hyprland titlebar rendering, video playback, full platform behavior,
resource budgets and startup are outside this acceptance.

Sidebar drag and drop remains disabled pending the toolkit contract. Two send
attempts returned `Authentication required`; the latest request record is
`checks/toolkit-delivery.json`. The message was not delivered. The prior toolkit
incident harness remains stopped. `overall_request=INCOMPLETE_DND_BLOCKED` is
preserved independently of the scoped functional PASS.

## Provenance

`MANIFEST.json` hashes every bundle file except itself. Per-machine copy manifests
preserve original paths, byte counts and hashes; remote archives were SHA-verified
before extraction, then every member was checked against its manifest. Overlay
source, protocols and licenses are retained to reproduce each profile. PPM copies,
compiled tools, user data and original filesystem fixtures are excluded; all
private originals remain in place.
