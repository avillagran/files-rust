# Nautilus as the minimum floor — parity matrix

2026-11-04. User request: Files must have at least the capabilities
of Nautilus (GNOME Files 47–49) and take the best from other file managers.

Nautilus sources: GNOME release notes 47/48, Nautilus `NEWS`, the GNOME Files
help and, from version 49.0, `help-overlay.ui` (shortcuts),
`nautilus-preferences-dialog.ui` and `freedesktop-dbus-interfaces.xml`.
The original matrix was checked against code at `6348af9`. The dated
2026-11-06 reconciliation below updates the entries supported by newer
code and retained evidence through `8454476` / `f38f658`; unchanged rows
retain their earlier scope and are not a fresh full-parity audit.

States: **yes** · **partial** · **no**. Priority: P1 = noticeable in daily use.

| # | Capability (Nautilus) | Files | Gap | Priority |
|---|---|---|---|---|
| 1 | List/grid, zoom Ctrl±/0 | partial | zoom only in grid | P2 |
| 2 | Configurable columns and sort by type/owner/permissions/dates | partial | block 5: sort by type; configurable columns and owner/permissions/creation missing | P2 |
| 3 | Expandable folders in list | no | — (Columns/Miller as an alternative) | P3 |
| 4 | Tabs, per-tab history | yes | — | — |
| 5 | Path bar + Ctrl+L, `/` and `~` | yes | — | — |
| 6 | Type to filter | yes | — | — |
| 7 | Hidden files Ctrl+H | yes | — | — |
| 8 | Image/video/PDF/cover thumbnails | partial | PDF, audio cover art, list | P2 |
| 9 | Rubber-band (marquee) selection | **yes** | block 3: Groups/List/Grid, Ctrl/Shift adds, click on empty space clears | — |
| 10 | Select by pattern Ctrl+S | **yes** | block 6 | — |
| 11 | Clipboard with other apps | partial | block 3: Ctrl+V pastes the most recent copy (Files or system); private native file-import end-to-end coverage remains pending | P2 |
| 12 | Drag and drop with other apps | partial | real Files-to-Files Wayland drop onto the Sidebar verified on NODE/M2; other-app interoperability remains pending; drops use copy review | P2 |
| 13 | Trash: empty, restore selected, Shift+Delete | **yes** | block 1: Shift+Delete with confirmation, Restore/Delete/Empty in the catalog | — |
| 14 | Undo/redo operations | partial | only rename and the last batch sent to trash | P2 |
| 15 | Rename and bulk rename | yes | (Files goes further: regex, sequences) | — |
| 16 | New folder, with selection, `~/Templates` templates | **yes** | block 5 | — |
| 17 | Compress/extract zip, tar.*, 7z | partial | ZIP only | P2 |
| 18 | Create link Ctrl+M / Shift+Ctrl+M | **yes** | blocks 1 and 6 | — |
| 19 | Properties: contents, free space, owner/group, recursive | partial | numeric UID/GID, inode metadata and apparent/allocated size exist; owner/group names, recursive contents/count and filesystem free space in properties remain missing | P2 |
| 20 | Open with another app and set as default | **yes** | block 2: Enter opens with the default; Open with… (Ctrl+Shift+O) | — |
| 21 | Terminal here (Ctrl+.) | **yes** | block 1 | — |
| 22 | User scripts / actions | no | — | P2 |
| 23 | Conflicts: replace, merge, per file | partial | no replace or merge | P2 |
| 24 | Operation progress | yes | — | — |
| 25 | Starred | no | — | P2 |
| 26 | Recent | partial | bounded local XBEL menu locates an existing file in its folder; no dedicated recent-files view or remote entries | P2 |
| 27 | Sidebar: bookmarks, live devices, network | partial | toolkit Sidebar: places, bookmark reorder, row menus, device actions and reviewed folder drops; network browsing missing; motion acceptance remains separate | P3 |
| 28 | Mount/unmount/eject, space per device | **yes** | block 4: udisksctl, free space, eject with power-off | — |
| 29 | Network: sftp/smb/ftp/dav/nfs | partial | SFTP only | P3 |
| 30 | Search with filters and full text | yes | — | — |
| 31 | Executables and `.desktop` | no | — | P2 |
| 32 | File chooser (portal) | no | — | P3 |
| 33 | `org.freedesktop.FileManager1` and `.desktop` | no | «Show in folder» from other apps | P1 |
| 34 | Shortcut help | yes | — | — |
| 35 | Bookmarks Ctrl+D | yes | — | — |
| 36 | Copy path | **yes** | block 1: Ctrl+Alt+C and context menu | — |
| 37 | Free space | yes | — | — |
| 38 | Wallpaper | no | — | P3 |
| 39 | i18n | no | — | P3 |
| 40 | AT-SPI accessibility | no | toolkit has optional AccessKit adapters; Files leaves `a11y` disabled and its browser canvas lacks complete virtual rows/actions and native AT-SPI validation | P2 |
| 41 | Space preview | yes | Files goes further (video, PDF, HTML, DOCX, ZIP) | — |
| 42 | Checksum | partial | not in properties | P3 |

What Files already has and Nautilus does not: dual pane (F9), Columns/Miller,
space map, folder sizes, compare/sync folders, duplicates,
regex rename, saved search, command palette, previews
of video/PDF/HTML/DOCX/ZIP.

From other file managers (pending, after the floor): custom actions with
`%f/%F` (Thunar/Dolphin/Nemo), terminal panel (Dolphin), color tags
(Finder), fuzzy/frecency jump like zoxide (yazi), a unified background task
manager (yazi).

## Work order

1. Full trash, create link, copy path and terminal with shortcuts.
2. Open with / default, `.desktop` + FileManager1.
3. Ctrl+V from the system, rubber-band selection.
4. Live devices and mount/eject, bookmarks in the sidebar.
5. Columns/sort, templates, recent, starred, tar, conflicts, scripts.

Progress on each row is recorded here with commit and evidence.

## Reconciliation — 2026-11-06

- **Sidebar and recent files:** commits `164b7aa`, `24db8de`, `22178ad`
  and `8454476` provide the toolkit Sidebar, bookmark reorder and a recent
  files menu. Recents read at most 8 MiB of local `recently-used.xbel`, keep
  at most 40 existing regular files, and locate the chosen file in its
  folder. One `task::Latest` loader per window discards superseded results.
  The [NODE Sidebar report](evidence/0.0.4/toolkit-sidebar-ui-1793946222728778808.json)
  and [native ARM M2 Sidebar report](evidence/toolkit-20261106/m2/docs/evidence/0.0.4/toolkit-sidebar-ui-1793946395642985978.json)
  pass recent-file location, bookmarks, row menus, simulated device actions
  and reviewed copies. Device fixtures do not prove real hardware eject;
  those scenes run with animations off and do not prove transition quality.
- **Real Wayland drop:** `da49afe` integrates the native reception fix;
  `f38f658` corrects the private harness for Lua/legacy Hyprland dispatchers.
  The [NODE real-drop report](evidence/sidebar-drop-20261105/lua-dispatch-1793946576953621044.json)
  and [M2 real-drop report](evidence/toolkit-20261106/m2/docs/evidence/sidebar-drop-20261105/lua-dispatch-1793946576062018538.json)
  pass a drag between two Files windows in a private nested compositor,
  opening copy review on the destination row and verifying copied bytes.
  This is not a complete external-application or cross-platform DnD matrix.
- **Properties:** `properties_dialog.rs` displays numeric UID/GID, mode,
  inode/device, hardlink count, timestamps and apparent/allocated bytes.
  Directory size is explicitly non-recursive. The owner/group row above
  therefore distinguishes existing numeric metadata from missing named
  identities and recursive summaries.
- **Accessibility:** the vendored toolkit's `a11y` feature offers AccessKit
  platform adapters. Files does not enable it. Smoke-only testkit inspection
  of Sidebar elements is useful test evidence, not proof that the custom
  browser canvas is accessible to AT-SPI. See
  [toolkit adoption scope](TOOLKIT-0d00f2b.md).

Functional reports do not close the resource/startup gates, which remain
`NOT_MEASURED`, or establish full Nautilus/Finder parity.

### Next navigation priorities

1. Validate Sidebar motion and browser displacement together, including
   rapid reversal, pin/hover, resize, covered modals and animations disabled.
   The [UI implementation sequence](UI-IMPLEMENTATION-PLAN.md) records this
   first delivery; the animation-disabled scenes above are its baseline.
2. Add deliberate tab/folder pointer gestures: middle-click close/open in a
   background tab, consistent modifier behavior and stable-ID tab reorder.
   The current canvas routes non-right-button presses through the generic
   selection path; this is not a validated middle-button interaction.
3. Preserve access to clipped breadcrumb ancestors with an overflow menu,
   exact path identities, background-tab opening and predictable focus return.
   The current path bar removes early segments when the last segments fill
   its width.

Before treating the historical Ctrl+Shift+End SpaceMap failure as a product
regression, reconcile `scripts/keyboard-nav-ui-check.py`: its final assertion
still expects one marked item, while the current selection implementation
supports Shift ranges and additive Ctrl+Shift ranges. The old failing report
remains evidence of that run; a fresh corrected scene is required before
claiming complete keyboard-navigation acceptance.

## Block 1 — 2026-11-04

- Permanent deletion (`remove_tree`): pinned descriptors, does not follow
  links, does not cross mounts, iterative and cancellable, on its own thread.
- Trash: restore by name with renameat2 NOREPLACE, delete one
  item with its metadata, empty (including orphaned metadata).
- Create link (Shift+Ctrl+M), Ctrl+. terminal, Ctrl+Alt+C copy path.
- User requests in the same delivery: a context menu grouped by
  type (OPEN, EDIT, ORGANIZE, COMPRESS, DELETE, SELECTION / CREATE,
  PASTE, VIEW, SELECTION, TOOLS), «Add new» next to FOLDERS in
  Groups, and refreshing visible subfolders (`child_watch`): non-recursive inotify
  on ≤64 visible folders, on its own thread; it waits 1.5 s of
  calm, rereads only that row (count and date) and recomputes a measured size
  at most once per minute per folder, in the sequential size
  queue. No polling: zero cost at rest. Limit: changes more than one
  level deep are not detected until recalculated by hand.
- Validation: `scripts/nautilus-floor-ui-check.py` in a real window with a
  trash and private fixtures; 712/112/807 Rust tests, Clippy, fmt.

Fixes in the same delivery (2026-11-04):
- Groups button: Nerd icon «folder +» (nf-md-folder_plus), without text,
  at the user's request.
- Intermittent subfolder refresh (2/3 failures on install): a reload
  of the open folder removed the watches (`loaded=false`) and then the
  reordered visible indices dropped them for a moment. Now a watch
  lasts 5 s outside the view and a folder watched again after a gap
  rereads its row. 6/6 source and 3/3 installed on NODE and M2; earlier FAILs
  kept in `docs/evidence/nautilus-floor-20261104`.
- Installed: NODE 7f35c3bd4f47, M2 a7298e28e489 (42587aa), 5 backups and
  hashes verified on both.

## Block 2 — 2026-11-04: Open and Open with

Critical finding: Enter/double click ran the `open` program, which in
Omarchy is only a bash function (it is not in PATH on NODE nor M2):
opening files did NOT work. Now, like Nautilus, `files_core::apps`
resolves the MIME type (by name, and by content if unknown), the default
application (`mimeapps.list` chain, installed entries,
user over system) and launches with `uwsm-app -t service -- <id> <path>`
(Omarchy's launcher; returns immediately). Without an application: a visible error.

«Open with…» (Ctrl+Shift+O, OPEN menu): recommended apps with the
default first, Tab shows all, Enter opens, Ctrl+Enter «Always
use» writes the user's `[Default Applications]` + `[Added Associations]`
atomically, keeping the rest. NoDisplay hidden as in
GNOME. Scanning and writing outside the UI thread.

Pending, with the user's consent: registering Files as the folder
manager (`.desktop` + `org.freedesktop.FileManager1`), because it would replace
Omarchy's current behavior (Nautilus) for «Show in folder».

Validation: `scripts/open-with-ui-check.py` (recording launcher, private XDG;
it never touches the user's associations) 3/3 + final;
716/112/807 tests, Clippy. `open-close-ui-check.py` stays SKIPPED on
Linux: running it would launch the user's real application.

## Block 3 — 2026-11-04: system Ctrl+V and rubber-band selection

- Ctrl+V pastes the most recent copy, like Nautilus: Files' buffer while it
  is still the system selection (it keeps «cut») or if the system
  offers no files; if another app copied files afterwards, those are imported.
  The native bridge exposes `selection_ours` (published, not cancelled source)
  and `files_offered`. The decision is covered by tests; the real import
  needs a real gesture and the user's clipboard: NOT validated end to end.
- Rubber-band selection in Groups/List/Grid with an accent border;
  Ctrl/Shift adds; click on empty space clears (before it did nothing).
- `build.rs`: two identical copies of fltk-sys in different registry
  indices (another project) no longer break the build; different copies still
  fail.
- Menu fixtures look up the entry by action (the sections moved the
  indices). `keyboard-nav-ui-check` fails in Space (Ctrl+Shift+End) just the same
  with the binary from before the parity work (fa50e53): a pre-existing failure, pending.
- Test windows must be launched with one Hyprland `exec` per script:
  when chained, the workspace rule expires and they appeared on the desktop.

## Block 4 — 2026-11-04: devices and bookmarks in the sidebar

- `files_core::devices`: `lsblk -J` (no system mounts, no
  EFI/reserved/recovery/Apple boot partitions, no filesystems the kernel
  does not support), free space, and mount/unmount/
  eject (unmount + `power-off` of the disk) with `udisksctl` (polkit, no
  root). Event-based watching: `/proc/self/mountinfo` (POLLPRI) and inotify
  on `/dev/disk/by-uuid`, a 400 ms delay, and a reread every 2 min for
  free space. Before, mounts were read only once at startup.
- Sidebar: up to 8 bookmarks and the devices (USB/disk icon, mounted
  or dimmed, free space, eject/unmount button when the sidebar is
  expanded). Clicking an unmounted device mounts and opens it; when
  ejecting the current one it goes back Home. Background menu: DEVICE section.
- The real NODE would list `/mnt/ubuntu`, Gbte, Peliculas and `/mnt/avd`.
- Validation: `scripts/devices-ui-check.py` (fixture lsblk + recording udisksctl;
  no real disk touched), tests 719/112/808, Clippy.

## Block 5 — 2026-11-04: sort by type, templates, folder from selection

- `Sort::Type` (extension, then natural name), in list, groups, dual
  pane, session (3) and per-folder views (3); a fourth TYPE column and the
  `sort-type` command. Type starts ascending; size and date descending (before,
  everything that was not name started descending).
- «New: <template>» (background menu, CREATE) from `XDG_TEMPLATES_DIR`
  (`user-dirs.dirs`) or `~/Templates`: a copy with a free name, ≤16 MiB.
- «New folder with selection» (ORGANIZE): «Carpeta nueva (n)» and
  renameat2 NOREPLACE of each item.
- Dynamic menu labels (`Cow`).
- Validation: floor check (templates ×2, group, sort by type) PASS;
  720/112/808 tests, Clippy; folder-views-ui-check PASS.

## Block 6 — 2026-11-04: Ctrl+S and Ctrl+M

- Ctrl+S opens the filter in selection mode: the pattern (glob, `*.jpg`) is
  typed as usual and Enter marks what matches and shows everything
  again; Esc cancels. Background menu SELECTION.
- Ctrl+M creates links here to what was copied in Files (background menu PASTE).
- Validation: floor check (`*.txt` → 4 marked; Ctrl+M «Enlace a
  zeta.ab»); 720/112/808 tests, Clippy.
