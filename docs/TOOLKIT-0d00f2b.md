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
