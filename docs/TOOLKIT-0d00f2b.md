# Toolkit 0d00f2b adoption

Files vendors the exact committed upstream checkpoint
`0d00f2b` from `lib-fltk-omarchy`, without local toolkit patches.
`vendor/lib-fltk-omarchy/UPSTREAM.json` records provenance. This is an
untagged local upstream checkpoint; it does not claim a published release.

The update includes the toolkit task controller, testkit, navigation and
scroll improvements, theme additions, command tables, split panes, and
optional media, diff and Mermaid components. Files keeps GPU, accessibility,
images, playback, QR and Mermaid features disabled. Existing preview engines
remain in use.

## Adopted APIs

- Each Files window owns one `task::Latest` recent-file loader. Repeated
  requests cancel the previous task and discard its result. The existing
  event loop drains toolkit callbacks. Files reads its bounded XBEL input
  on the toolkit worker pool instead of creating a thread and polling a
  private channel every 20 ms for every activation.
- Read errors still reach the user. Results cannot open a menu over a
  closing/deleted/hidden window or a canvas modal. Cancellation is
  cooperative: an in-progress filesystem call cannot be interrupted.
- Smoke fixtures arm `testkit` before constructing controls. The private
  `sidebar-tree` command exports the toolkit element tree; the sidebar UI
  check verifies visible recent-file navigation and virtual row identities.
  Ordinary runs do not arm this instrumentation.

The environment script runner requires `App::run`; Files has its own event
loop, so fixture commands explicitly call the tree API. This adoption does
not replace the custom browser canvas or establish accessibility for it.

## Validation

Validation reports are saved under `docs/evidence/0.0.4`. Resource/startup
performance gates remain `NOT_MEASURED`; functional checks do not establish
resource compliance. NODE and native ARM M2 artifacts must each pass their
private fixture check before installation; installation preserves the five
previous artifacts and verifies replacement hashes.

## Validated checkpoint 8454476

- Workspace: 1,652 passing tests, three ignored; Clippy and release build pass.
- NODE: Sidebar (including 19 testkit elements), menus and keyboard checks
  pass. Installed app SHA-256 starts `62942cf838ae`; installed private-data
  launch/render/close check passes.
- M2: native aarch64 build, Sidebar scene and installed private-data smoke
  pass. Installed app SHA-256 starts `f93ca0fe9e67`. The native HTML helper
  was reused after its recipe and stamp were verified.
- Both installations back up all five prior artifacts and verify hashes.
- Initial environment failures are retained: shared temporary-directory
  permissions, an early UI check against the prior binary, and the relocated
  M2 CMake cache. Private TMPDIR, a finished release build and fresh CMake
  configuration resolve those setup failures, respectively. No toolkit patch
  or filesystem safety relaxation was needed.

Real Wayland drag-and-drop also passes on NODE and M2 with the same app
binaries: two private fixture windows, Sidebar destination, copy review and
byte verification. The harness now checks dispatcher replies and supports
both Lua and legacy Hyprland dispatchers inside the private compositor.
An initial M2 failure used rejected legacy commands; an intermediate
output-width assumption was rejected on both machines and removed. Failed
reports remain alongside the final passing reports. No parent desktop
window is driven by the corrected harness.
