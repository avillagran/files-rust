# Nautilus as the minimum floor — parity matrix

2026-11-04. User request: Files must have at least the capabilities
of Nautilus (GNOME Files 47–49) and take the best from other file managers.

Nautilus sources: GNOME release notes 47/48, Nautilus `NEWS`, the GNOME Files
help and, from version 49.0, `help-overlay.ui` (shortcuts),
`nautilus-preferences-dialog.ui` and `freedesktop-dbus-interfaces.xml`.
The original matrix was checked against code at `6348af9`. The dated
2026-11-06 reconciliation below updates the entries supported by newer
code and retained evidence through `cbc83a9`; unchanged rows
retain their earlier scope and are not a fresh full-parity audit.
Source notes distinguish the validated synchronous `99db7b1` / `024bb87`
presentation delivery from the current exact-toolkit `fac0705` asynchronous
adapter. Files asynchronous acceptance and NODE/M2 installation passed in
`8b69160`; its application evidence is separate from upstream validation.

States: **yes** · **partial** · **no**. Priority: P1 = noticeable in daily use.

| # | Capability (Nautilus) | Files | Gap | Priority |
|---|---|---|---|---|
| 1 | List/grid, zoom Ctrl±/0 | partial | zoom only in grid | P2 |
| 2 | Configurable columns and sort by type/owner/permissions/dates | partial | block 5: sort by type; configurable columns and owner/permissions/creation missing | P2 |
| 3 | Expandable folders in list | no | — (Columns/Miller as an alternative) | P3 |
| 4 | Tabs, per-tab history | yes | keyed toolkit strip, drag/keyboard/menu reorder and native context/close in a0f1fff/e47cf81; tear-off and native macOS acceptance pending | P2 |
| 5 | Path bar + Ctrl+L, `/` and `~` | yes | hidden ancestors, Alt+Down and background destinations verified in 74d205f; 2026-11-10: typing `/` or `~` in the list opens the field, Tab completes paths (candidates in the status line) | — |
| 6 | Type to filter | yes | — | — |
| 7 | Hidden files Ctrl+H | yes | — | — |
| 8 | Image/video/PDF/cover thumbnails | partial | 2026-11-10: PDF first page (pdftoppm) and SVG (rsvg-convert), both degrading quietly to the icon when the tool is missing; reads and writes the freedesktop shared cache `~/.cache/thumbnails` (see THUMBNAILS.md). Audio cover art, office documents and thumbnails in the list view are still missing; interop with other apps' thumbnails is untested on a real desktop | P2 |
| 9 | Rubber-band (marquee) selection | **yes** | block 3: Groups/List/Grid, Ctrl/Shift adds, click on empty space clears | — |
| 10 | Select by pattern Ctrl+S | **yes** | block 6 | — |
| 11 | Clipboard with other apps | partial | block 3: Ctrl+V pastes the most recent copy (Files or system); private native file-import end-to-end coverage remains pending | P2 |
| 12 | Drag and drop with other apps | partial | real Files-to-Files Wayland drop onto the Sidebar verified on NODE/M2; other-app interoperability remains pending; drops use copy review | P2 |
| 13 | Trash: empty, restore selected, Shift+Delete | **yes** | block 1: Shift+Delete with confirmation, Restore/Delete/Empty in the catalog; 2026-11-10: per-volume trash (`$topdir/.Trash/$uid` or `.Trash-$uid`, relative `Path=`) for items on other filesystems, merged into the catalog, Empty and Undo; verified only with simulated volumes, no real second filesystem exercised | — |
| 14 | Undo/redo operations | partial | only rename and the last batch sent to trash | P2 |
| 15 | Rename and bulk rename | yes | (Files goes further: regex, sequences) | — |
| 16 | New folder, with selection, `~/Templates` templates | **yes** | block 5 | — |
| 17 | Compress/extract zip, tar.*, 7z | partial | ZIP only | P2 |
| 18 | Create link Ctrl+M / Shift+Ctrl+M | **yes** | blocks 1 and 6 | — |
| 19 | Properties: contents, free space, owner/group, recursive | partial | owner/group resolved to `name (id)` (getpwuid_r/getgrgid_r, numeric fallback) on the properties worker; folder and multi-selection contents (apparent bytes, files, folders, links) via the cancellable helper `measure_contents` traversal that never follows symlinks, shows `calculando… N elementos` and reports unreadable subtrees/failed objects as PARCIAL; volume line (filesystem type, free/total via statvfs + mountinfo); symlink raw target, resolution state and lstat "Permisos del enlace"; octal/symbolic mode, "Permitir ejecutar como programa" toggle (user execute bit, existing chmod path); modified/accessed/created (statx btime)/inode-change times; MIME type and default application name (change via Ctrl+Shift+O). Missing: image dimensions (header parsing lives only in the helper, no cheap shared reader), checksum shortcut (row 42), on-screen layout not verified by screenshot | P2 |
| 20 | Open with another app and set as default | **yes** | block 2: Enter opens with the default; Open with… (Ctrl+Shift+O) | — |
| 21 | Terminal here (Ctrl+.) | **yes** | block 1 | — |
| 22 | User scripts / actions | no | — | P2 |
| 23 | Conflicts: replace, merge, per file | partial | no replace or merge | P2 |
| 24 | Operation progress | yes | — | — |
| 25 | Starred | no | — | P2 |
| 26 | Recent | partial | bounded local XBEL menu locates an existing file in its folder; no dedicated recent-files view or remote entries | P2 |
| 27 | Sidebar: bookmarks, live devices, network | partial | toolkit Sidebar: places, bookmark reorder, row menus, device actions and reviewed folder drops; network browsing missing; motion acceptance remains separate | P3 |
| 28 | Mount/unmount/eject, space per device | **yes** | block 4: udisksctl, free space, eject with power-off | — |
| 29 | Network: sftp/smb/ftp/dav/nfs | partial | typed URIs mount through the session's GVfs (d636633; FTP exercised end to end, SMB/WebDAV not against a real server); no discovery or keyring | P3 |
| 30 | Search with filters and full text | yes | — | — |
| 31 | Executables and `.desktop` | no | — | P2 |
| 32 | File chooser (portal) | no | — | P3 |
| 33 | `org.freedesktop.FileManager1` and `.desktop` | partial | explicit three-method service and select/folders/properties CLI implemented; startup context and cancellable presentation through toolkit fac0705 in source; async NODE/M2 acceptance and installation passed; automatic desktop activation/default registration remains open | P1 |
| 34 | Shortcut help | yes | F1 or Ctrl+?; lists every binding; the palette and tooltips carry the chords | — |
| 35 | Bookmarks Ctrl+D | yes | — | — |
| 36 | Copy path | **yes** | block 1: Ctrl+Alt+C and context menu | — |
| 37 | Free space | yes | — | — |
| 38 | Wallpaper | no | — | P3 |
| 39 | i18n | no | — | P3 |
| 40 | AT-SPI accessibility | no | toolkit has optional AccessKit adapters; Files leaves `a11y` disabled and its browser canvas lacks complete virtual rows/actions and native AT-SPI validation | P2 |
| 41 | Space preview | yes | Files goes further (video, PDF, HTML, DOCX, ZIP) | — |
| 42 | Checksum | partial | not in properties | P3 |
| 43 | Keyboard: tab chords, open in new tab, places rail | **yes** | 2026-11-10 (3a955e5): Ctrl+PgUp/PgDn, Alt+1..9, Ctrl/Shift+Enter, Enter opens every marked item, Alt+Home, rail focus with Tab or Ctrl+Alt+S. Kept on purpose and differing from Nautilus: F5 copy, F6 move, F9 dual pane, Ctrl+1/2 Groups/List, Ctrl+I invert, Ctrl+N new file, Ctrl+Space folder size, Alt+Down ancestors | owner decision |
| 44 | Symlinked folders browse as folders | **yes** | 2026-11-10 (c1b6db8): icon, folders-first order, Enter/new tab keep the link path; recursive and mutating code still treats them as links | — |

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
- **Properties:** `properties_dialog.rs` shows owner/group names with ids,
  octal and symbolic mode, a user-execute toggle, local-time modified,
  accessed, created (statx btime when recorded) and inode-change times,
  MIME type with the default application, the volume's filesystem type and
  free/total space, the raw symlink target with its resolution state and the
  link's own lstat permissions. Folder contents (and multi-selection totals)
  come from the `files-helper` `measure_contents` census: symlinks are counted
  and never entered, unreadable subtrees make the figures an explicit partial
  lower bound, and closing the dialog cancels the job. Image dimensions are
  not shown.
- **Accessibility:** the vendored toolkit's `a11y` feature offers AccessKit
  platform adapters. Files does not enable it. Smoke-only testkit inspection
  of Sidebar elements is useful test evidence, not proof that the custom
  browser canvas is accessible to AT-SPI. See
  [toolkit adoption scope](TOOLKIT-0d00f2b.md).

Functional reports do not close the resource/startup gates, which remain
`NOT_MEASURED`, or establish full Nautilus/Finder parity.

### Next navigation priorities

1. Continue navigation-surface focus and hover acceptance. Sidebar/browser
   motion, reversal, pinning, resize and disabled motion are delivered in
   `de480ce`; see [SIDEBAR-MOTION.md](SIDEBAR-MOTION.md).
2. Extend tab/platform acceptance and consider tear-off.
   Middle folder/background and middle-close gestures are delivered in
   `c52b96b`, including native NODE/M2 input evidence; stable menu/keyboard
   reorder and inactive-tab context follow in `1c2cdee`. The keyed strip,
   drag reorder, independent cursor and toolkit motion follow in
   `a0f1fff`/`e47cf81`; see [KEYED-TABS.md](KEYED-TABS.md).
   Tear-off and broader modifier/platform acceptance remain open.
3. Extend cross-view keyboard/accessibility acceptance. Hidden breadcrumb
   ancestors, exact path identity, background destinations and focus return
   are delivered in `74d205f`; see
   [BREADCRUMB-OVERFLOW.md](BREADCRUMB-OVERFLOW.md).

The historical Ctrl+Shift+End SpaceMap failure came from an obsolete fixture
expectation of one mark. `1c2cdee` corrects it to the complete inclusive range
and adds an additive-range case retaining distant marks and anchor. Fresh
private handler/render scenes pass; see [TAB-CONTEXT.md](TAB-CONTEXT.md).
The original failing reports remain preserved. This is not complete native
keyboard or accessibility acceptance.

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
  with the binary from before the parity work (fa50e53): a pre-existing failure.
  Resolved by the corrected fixture in `1c2cdee`, documented above; the
  historical failure is retained rather than reclassified as a passing run.
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

### 2026-11-06 — Background folder tab gestures

`c52b96b` adds middle-click folder admission without changing selection,
anchor, history or pane focus; middle-click closes the clicked tab ID
without first selecting it. List/Grid/Columns and inactive-pane ownership
pass private fixtures on NODE and M2, with separate real Wayland
BTN_MIDDLE checks. See [TAB-GESTURES.md](TAB-GESTURES.md). Drag reordering,
tear-off and native macOS verification remain open.

### 2026-11-06 — Complete breadcrumb ancestry

`74d205f` keeps every hidden ancestor reachable through the toolkit popup,
including above HOME. Bounded painted targets, measured widths, exact raw
paths, Alt+Down, Escape focus return and hidden/visible ancestor background
tabs pass private fixtures on NODE and M2 with animations on and off.
See [BREADCRUMB-OVERFLOW.md](BREADCRUMB-OVERFLOW.md). Native OS input coverage
for this path-bar scenario remains distinct from these handler checks.

### 2026-11-06 — Stable tab context and ordering

`1c2cdee` adds right-click actions for inactive tabs without selecting them,
Close Others/Close Right with whole-action snapshot admission, and stable-ID
menu/keyboard reordering. Ctrl+Shift+F10 opens active-tab actions;
Ctrl+Shift+PageUp/PageDown moves the active tab while preserving its browser.
The application controllers can receive the toolkit keyed strip's future
callbacks; drag reordering and the widget migration remain pending.
See [TAB-CONTEXT.md](TAB-CONTEXT.md) for scoped evidence and limitations.

### 2026-11-06 — Pane-owned empty states

`acdc119` removes the global empty-state overlay from dual pane and Columns,
so an empty active pane no longer paints over a populated neighbor. Each
surface distinguishes pending loading, an explicit error, a folder with no
entries and an active filter with no matches. Private scenes verify either
empty-pane position at normal/minimum sizes and preserved sibling selection
on NODE and native ARM M2. See
[BROWSER-EMPTY-STATES.md](BROWSER-EMPTY-STATES.md).

### 2026-11-06 — Keyed toolkit tabs and motion

`a0f1fff`/`e47cf81` connect exact toolkit `a029d45` to Files' stable tab and
snapshot controllers. Native right/middle gestures, drag/cursor reorder,
focus return, Home/overflow, compound pane keys and unchanged-selection
startup intent pass on installed NODE and ARM M2. The shared indicator has
intermediate/reversal/reduced-motion rendered frame acceptance. See
[KEYED-TABS.md](KEYED-TABS.md) for exact viewport and input scope. Tear-off,
native macOS/Windows, full accessibility/i18n and wider parity remain open.

### 2026-11-06 — Explicit FileManager1 and external locations

The Linux implementation provides all three FileManager1 methods behind
`--file-manager-service`, plus `--select`, `--show-folders` and `--properties`.
Bounded local URI decoding, exact item identity, whole-batch helper preflight,
tab-budget admission and stale-view/modal cancellation precede UI effects.
Multiple objects use the existing paged properties dialog, including browser
multi-selection, without introducing batch permission changes. See
[FILEMANAGER1.md](FILEMANAGER1.md) for limits, the startup/connect caveat and
the source-only optional activation template. No activation provider or
default association is installed by this work. The current source adopts
explicit startup-context forwarding as described below; native platform
acceptance and the resource/startup gates remain open. Delivery-specific
acceptance is recorded separately.


### 2026-11-06 — Persistent operation feedback

`893b25a` adopts exact toolkit `0f68089` notices: one current non-modal card,
explicit acknowledgement and typed Undo/Redo or retained-result reports.
Callbacks revalidate result identity and current admission; informational
messages cannot inherit another operation's actions. Required recovery/error
reviews and held queues retain their existing behavior. Native source and
installed NODE/M2 checks pass, including real history actions and card-only
ON/OFF pixel equality. See [PERSISTENT-NOTICES.md](PERSISTENT-NOTICES.md).
This improves feedback without closing the remaining operation, platform,
language, accessibility or activation gaps in this matrix.

### 2026-11-06 — Historical synchronous presentation delivery

Source `9904f88`/`99db7b1`, recorded in `024bb87`, used exact toolkit
`767235b` and its synchronous `native::present` boundary.
Accepted FileManager1 requests retain their exact startup context, queue a
reply before native work and recheck view/modal ownership immediately before
presentation. A single expiring first-paint slot handles initially hidden
windows. Native `Requested` does not establish granted focus, and Files
does not force activation or replay rejected context. This historical
delivery used bounded synchronous Wayland private-queue waits; its results
do not validate the newer asynchronous transitions below.

Files-specific native acceptance and installation passed on NODE and ARM
Linux M2, including real gesture tokens, compositor focus and first-map
close cancellation. Windows/macOS runtime acceptance,
automatic activation/default registration and the resource/startup gates
remain open. See [WINDOW-PRESENTATION.md](WINDOW-PRESENTATION.md). This source
change does not turn row 33 into complete desktop interoperability or close
the broader Nautilus/Finder goal.


### 2026-11-06 — Asynchronous presentation implemented in source

Exact toolkit `fac0705` supplies `begin_present` and a UI-thread-only handle
with owned redacted hints, poll/cancel and cached terminal outcomes. Files
retains one intent plus handle, polls during ordinary UI turns outside model
borrows and drops pending work on stale generation/modal/latch ownership,
replacement or close. Canvas focus is restored only at native start. The
two-second first-paint/start gate ends when native begin succeeds; Wayland
then uses separate two-second discovery/token deadlines without compositor
waits or an internal worker/timer. X11's first poll still performs synchronous
native I/O. Queued admission ACK and `Requested` retain their earlier scope:
neither establishes granted focus or guarantees reply delivery during close.

The exact export contains 467 upstream files, all 26 tracked receipts and
Files-owned provenance, with CPU/default-feature configuration unchanged.
It includes the committed lazy Sheet source from `8be476b`, which Files has
not adopted. Toolkit CPU suites, the TLS exit fix and native probes are
upstream evidence. Files passed nine source scopes and four installed checks
on each machine, including delayed token reply/input/motion and late-cancel
protection. Native macOS/Windows,
automatic activation/default registration, full language/accessibility,
remaining operation gaps and `NOT_MEASURED` resource/startup gates stay open.
See [WINDOW-PRESENTATION.md](WINDOW-PRESENTATION.md).
