# Recent and starred items — 2026-11-10

Closes rows 25 and 26 of [NAUTILUS-PARITY](NAUTILUS-PARITY.md): a browsable
"Recientes" location, a "Destacados" (starred) location, and Files recording
the files it opens in the desktop's recent list.

## Virtual locations

Every file action in Files joins an entry name to `Model::path`, so a list of
absolute paths from many folders cannot pose as a folder listing without
touching every action. Files already models places that are not one folder
(the trash catalog, search results) as model-owned panels; Recientes and
Destacados follow the trash browser's model (`crates/ui/src/virtual_list.rs`):

- The panel paints over the listing area as a list (name, containing folder,
  time) and replaces its hit targets; the places rail stays usable. Clicks on
  empty space do nothing (no rubber band, no selection change underneath).
- It loads off the UI thread (one short-lived thread per load) and keeps the
  folder underneath untouched: closing it (Backspace with an empty filter,
  Alt+Left, Esc) returns exactly where the person was. Rail places, Home,
  history forward, tabs and search close it first.
- `virtual_list::gate` runs inside `Model::action` and refuses every file
  action while a listing is open, whatever asked for it (key, palette, menu,
  toolbar, operation bar): trash, delete permanently, rename, cut, copy,
  paste, system paste, move, copy review, new file/folder, links. The refusal
  explains itself in the panel caption and the status line. Delete (and F8,
  the trash chord) only removes the entry from the list; Shift+Delete is
  refused.
- Keys: arrows, PgUp/PgDn, Home/End, Enter (files open with their default
  application through the usual open worker; folders navigate), Ctrl+Alt+O
  "Abrir carpeta contenedora" (navigates to the parent and selects the item),
  Ctrl+E star/unstar, typing filters by name, Menu/Shift+F10 or right click
  opens the row menu (open, containing folder, remove from the list, star;
  never a file action), F5/Ctrl+R reload, Ctrl+Alt+R / Ctrl+Alt+E switch
  listings. The palette, help and preferences stay available.

Recientes: `recently-used.xbel` (files and folders that still exist) merged
with Ofolder's recents (see [OFOLDER](OFOLDER.md)), newest first, at most 200.
Removing an entry rewrites the XBEL without it; an entry known only from
Ofolder (whose list Files never writes) is hidden through
`$XDG_DATA_HOME/files/recent-hidden` until Ofolder records a newer use. The
sidebar row opens the listing; its row menu keeps the 40-item quick menu.

Destacados: the starred list, newest star first. Items that no longer exist
stay listed as "no disponible" and can be removed with Delete.

## Recording opened files

When Files opens a file with an application (Enter, double click, "Abrir
con", the listings) and the launcher succeeds, the open worker thread calls
`files_core::recent_store::Recorder::opened`. It adds or refreshes the
file's `<bookmark>` in `$XDG_DATA_HOME/recently-used.xbel`:

- `href` is a `file://` URI with every byte outside `A-Za-z0-9-._~/`
  percent-encoded (spaces, non-ASCII, `#`, `%`...); `added`, `modified` and
  `visited` are ISO 8601 UTC with microseconds, as GLib writes them;
  `mime:mime-type` comes from Files' MIME detection; Files registers
  `<bookmark:application name="Files" exec="&apos;files %u&apos;"
  modified="…" count="N"/>`, incrementing `count` on each open.
- The write is a text splice: only the one element changes; every other
  entry, attribute, comment and unknown element is kept byte-for-byte. A
  missing `xmlns:bookmark` / `xmlns:mime` declaration is added to the root.
- It runs under an advisory `flock` on `$XDG_DATA_HOME/files/recently-used.lock`
  (GLib writers do not take it) and replaces the file atomically through
  `persistence::atomic_write` (`0600` temporary in the same folder, fsync,
  rename, folder fsync).
- A document larger than 4 MiB, not UTF-8, not a regular file (a symlink is
  never followed or replaced) or structurally unsound is not rewritten: the
  worker logs one `FILES_RECENT … skipped: <reason>` line.
- Never recorded: trash folders (`$XDG_DATA_HOME/Trash`, any `.Trash`,
  `.Trash-*` component), `$XDG_CACHE_HOME`, `$XDG_RUNTIME_DIR` and Files'
  own `$XDG_DATA_HOME/files`.
- Preference `record_recent = true|false` in `config.toml` (default true),
  toggled from Preferences ("Registrar archivos abiertos en Recientes") or the
  palette. GNOME's `remember-recent-files` lives in dconf, which is not
  cheaply readable without GSettings, so it is not consulted. Disabled
  recording opens no file at all. Test builds never record from the open
  worker; the data folder is injected everywhere (`Recorder::data_home`,
  `virtual_list::State`, `starred_state::State`).

## Starred format

`$XDG_DATA_HOME/files/starred`, UTF-8 text, written through
`persistence::atomic_write` while the process holds a lock:

```text
# Files starred items, format 1
file:///home/u/Documents/informe%20final.odt
file:///home/u/Proyectos
```

One percent-encoded `file://` URI per line (any path, even with a line
break, fits on one line), in starring order, at most 10,000. `#` lines and
blank lines are ignored; an unreadable file is never rewritten.

Stars are not bookmarks: bookmarks (Ctrl+D) are sidebar places and are
shared with Ofolder's favorites; stars flag files and folders. Files never
writes stars into Ofolder's favorites. Ofolder has no starred concept today;
the stable documented file above is how it (or any tool) can offer starred
folders without Files changing Ofolder's data.

A small star emblem marks starred rows in List view (`view.rs`, beside the
symlink arrow) and in the listings; Grid and Columns have no emblem yet.

## Chords

Shared by both profiles (`shortcuts.rs`, checked for collisions by its
tests): Ctrl+Alt+R "Ir a Recientes", Ctrl+Alt+E "Ir a Destacados", Ctrl+E
"Destacar / quitar", Ctrl+Alt+O "Abrir carpeta contenedora" (GNOME Files'
"Open Item Location" chord). Help and the palette list them from the table.

## Not verified

- On-screen look of the listing, the emblem and the new rail row (model and
  unit tests only; no real window was launched).
- Interoperation with GTK applications reading and writing the XBEL at the
  same time (GLib does not take Files' lock; a race loses one side's latest
  change, never the file).
