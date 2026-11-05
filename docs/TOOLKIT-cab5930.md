# Toolkit cab5930: places rail on the toolkit Sidebar

User request (2026-11-05): adopt the animated toolkit `Sidebar` for the places
rail once the toolkit had the pieces Files asked for (secondary action / eject,
detail line, dimmed rows, sections, drops, per-item menu). Files does not patch
the vendor.

## Vendor

- `vendor/lib-fltk-omarchy` is the exact export of `cab5930` (GitHub main,
  untagged), verified with `diff -r` against `git archive cab5930`, plus
  `UPSTREAM.json`. The `*.log` evidence files of the toolkit are not
  committed because the Files `.gitignore` ignores `*.log` (same as before).
- `cab5930` adds `navtree::NavTree` and rebuilds `widgets::Sidebar` on it
  (section headers, collapsible sections, detail line, usage bar, secondary
  action, dimmed / disabled / busy rows, keyed `set_items`, `on_context`,
  drops, scrolling, `item_rect`).
- Files built and passed its unit tests against the new vendor before any Files
  change (812/812).
- Do not use `cargo fmt --all`: it reformats the vendor. Format only the Files
  crates (`cargo fmt -p files-ui -p files-core -p files-app -p files-helper`).

## Rail

- `places_sidebar::rail(&Model)` is the single description of the rail:
  Favorites (opens the bookmarks panel), the XDG places, up to eight
  bookmarks under a "Marcadores" header, devices under a collapsible
  "Dispositivos" header, and Papelera as a footer row. Each row carries the Files
  `Action` it runs; mounted devices carry an eject / unmount secondary
  action. Keys are stable (`place:<path>`, `mark:<path>`, `dev:<node>`), so
  `set_items` keeps the cursor and folded sections and animates arrivals.
- Devices show the free space as the detail line, a usage bar when the size is
  known, dimmed rows when unmounted, and a spinner while an operation runs.
- The widget sits over the canvas strip `y = 90 .. browser_height - 32`
  in push mode, with widths 52 / 170 as in `mockups/v3.html`. The canvas paints
  the strip and then draws the rail with `draw_child`, because plain FLTK
  redraws damaged siblings independently. `m.rail` mirrors the widget's
  expanded state, so every existing layout that offsets by the rail keeps
  working.
- `places_sidebar::covered` hides the widget while a canvas-drawn modal or
  full-window surface is open (dialogs, previews, panels, trash browser). The
  canvas then paints the old rail rows underneath those surfaces.
- The canvas still registers the rail hit targets (with a null painter while
  the widget is present): scenes and fixtures use them, and the pointer
  reaches the widget first.
- The rail is kept out of Tab navigation (`clear_visible_focus`), because Tab
  switches panes in Files. After a row runs its action, focus goes back to the
  browser.
- The canvas pins the widget's width to 52 / 170 on every paint. FLTK scales
  children with the window (the canvas is the resizable widget), and a
  compositor-resized window had shrunk the rail to 41 px while the listing
  still started at 68.

## Context menu

`on_context` (right click, Menu, Shift+F10) opens a `PopupMenu` built by
`places_sidebar::menu`:

- Favorites: open the bookmarks panel; add this folder to the bookmarks.
- Place: open; open in a new tab.
- Bookmark: open; open in a new tab; remove the bookmark (the saved entry,
  through the same bookmark request as the panel).
- Mounted device: open; open in a new tab; unmount; eject (removable only).
- Unmounted device: mount and open; mount.
- Trash: open the trash.

## Recent files

A "Recientes" row under Favoritos opens the toolkit's filter menu
(`overlay::Menu`) with the recently used files that desktop apps record in
`$XDG_DATA_HOME/recently-used.xbel`. Picking one goes to its folder and
selects it (`places_sidebar::locate`, the same path as search and checksum
"locate").

- `files_core::recent` reads the file off the UI thread: at most 8 MiB, only
  local `file://` entries, URIs decoded with `file_transfer::decode`, one row
  per path (the newest date wins), newest first, existing regular files only,
  at most 40.
- The menu opens when the list arrives (a 20 ms UI-thread poll, 5 s cap).
  An empty list shows "No hay archivos recientes"; a read error says so.
- The row's context menu offers "Abrir recientes".

## Reordering bookmarks

Bookmark rows can be dragged among themselves; nothing else moves. This uses
`Sidebar::tree()` with `Reorder::Siblings` and `accept_move` restricted to
`mark:` rows under the "Marcadores" header.

- The row moves at once (`navtree::move_item` on the widget's model), and the
  new position is saved with the new core operation
  `bookmarks::Op::Move { expected, to }`. It is locked and written atomically
  like the other bookmark operations, keeps ids and labels, clamps the
  target, and refuses a stale entry.
- If the save does not happen, the next paint finds the widget's order
  different from the saved one (once no bookmark request is running) and
  calls `set_items` to put it back.

## Drops

Files receives drops through its own Wayland transfer layer, with a copy
review, not through FLTK DnD. So the Sidebar refuses the toolkit path
(`accept_drop(|_| false)`), and `native_drop::resolve` gains
`Scope::Place`:

- Every paint records the rectangles of the folder rows (places, bookmarks,
  mounted devices) in `Model::rail_drops`.
- A drop on one of them opens the same copy review as the listing, with that
  folder as the destination.
- Files paints the drop highlight over the widget.
- The target is stale once the row is no longer painted (hidden rail,
  re-listed rows).

## Verification

- Unit tests: 814 pass (812 + 2 new ones in `places_sidebar`), with a private
  `TMPDIR`. Under the session's `/var/tmp` temporary directory,
  `rename_job::trash_create_failure_conflict_and_restore_retry_keep_history`
  fails by design: the trash refuses ancestors writable by others. This is
  unrelated to the toolkit.
- `scripts/toolkit-sidebar-ui-check.py` (Xvfb, `OMARCHY_FLTK_ANIM=off`, HOME
  inside the fixture, fixture `lsblk` and a recording `udisksctl`) checks:
  - the collapsed rail at (0, 90, 52, h) and the 170 px expansion that pushes
    the listing;
  - a click on Descargas navigates there and marks the row active;
  - a click on the mounted fixture stick opens it, and its eject button records
    `unmount` + `power-off` (no real disk);
  - the trash browser hides the rail and closing it shows the rail again;
  - right click on Documentos, End + Enter: "Abrir en pestaña nueva" opens a
    second tab on Documentos;
  - from Descargas, "Recientes" lists only the existing recent file (a deleted
    one is skipped), and Enter goes to Documentos with that file selected;
  - Descargas and Música added as bookmarks from the Favoritos row menu;
    dragging Música above Descargas saves the new order ("Favorito movido");
  - a file dropped on Documentos (private-pipe fixture through Files' native
    transfer) opens the review with Documentos as destination, copies
    nothing until confirmed, then copies the exact bytes; the source stays
    intact.
- `hidden-context-ui-check.py` now runs with animations off. Its marquee check
  compares whole frames, and the Sidebar slides in the device rows that are
  listed after the first paint. They take about 0.4 s and are then still; idle
  CPU stayed at about 1 %.
- Not run on NODE: `ui-check.py` (needs a real Hyprland session). The checks
  that hash `files-html-helper` need that optional helper, which NODE lost on
  2026-11-05; they were run through a wrapper that only skips that hash.

## Real drag and drop (open, predates this change)

`scripts/sidebar-drop-nested-check.py` runs two Files windows in a private
nested Hyprland (hidden special workspace, virtual pointer on the nested
socket only). It drags a file from A's listing onto B's Documentos row, or onto
B's listing with `--onto body` as the control.

The source window reports "Arrastre de copia solicitado al compositor", but
the target gets no drag event at all (`FILES_DND_TRACE=1` logs none), for the
rail and for the listing alike. Real Wayland DnD between applications was
never validated in Files (open item O06), and this result is the evidence:
the gap is in the native transfer or the harness, not in the rail. Fixing it is
separate work.

`scripts/tools/vpointer.c` gains a `d<ms>` mode for this: press, move slowly,
then rest on the target before releasing.

## Known

- The earlier truncation at 170 px ("Document…") came from the check's
  18 px shell font. With the default font the rows are compact and the labels
  fit.
- The toolkit's known limit stands: a removed row fades out in place but does
  not shrink.
