# Ofolder integration

[Ofolder](https://github.com/jrimmer/ofolder) (MIT) docks favorites, recent
folders and a preview beside the Open/Save dialogs on Omarchy. Files shares
those lists so that a folder used in a dialog shows up in Files and the other
way round. Nothing is required: without Ofolder the code paths read nothing
and write nothing.

## What Files does

- **Favorites.** Ofolder's favorites appear in the rail's "Marcadores"
  section after Files' own bookmarks, marked "Ofolder". A favorite both lists
  share is shown once, as the Files bookmark. Ofolder's rows do not reorder
  (their order belongs to Ofolder); their menu offers "Quitar de favoritos de
  Ofolder". Adding a bookmark in Files (Ctrl+D) also adds it to Ofolder, and
  removing a Files bookmark removes it there too. Bookmarks that existed
  before are not copied in bulk.
- **Recents.** The "Recientes" menu merges Ofolder's recent files and folders
  with the desktop's `recently-used.xbel`, newest first, one entry per path.
  Picking a folder opens it; picking a file opens its folder with the file
  selected. Files never writes Ofolder's recents.

## Files and protocol (checked against Ofolder's source on 2026-11-09)

| List | Path | Shape |
| --- | --- | --- |
| Favorites | `$OFOLDER_FAVORITES`, else `$XDG_CONFIG_HOME/ofolder/favorites.json` | `{"version": 1, "favorites": [{"path": "/a", "name": "Alpha"}]}` (`name` optional) |
| Recents | `$XDG_DATA_HOME/ofolder/recents.json` | `{"folders": [{"path", "apps", "at"}], "files": [...]}`, `at` in Unix seconds |

- Files writes favorites only when Ofolder's configuration folder exists.
- A write takes the same exclusive `flock` on `favorites.json.lock` that
  Ofolder takes (waiting at most 2 s), re-reads the file, changes one entry
  and replaces the file through a `0600` temporary, `fsync`, `rename` and a
  directory `fsync`, exactly like Ofolder's `write_private_atomic`.
- Unknown fields are kept. A file Files cannot parse, or whose `version` is
  newer than 1, is never rewritten (Ofolder sets broken files aside itself).
- The watcher checks the favorites file's size and date every 3 s and re-reads
  it only when they change.
- Ofolder's dialog rail reads favorites when it opens and saves its whole
  list on each change (last save wins), so a change made in Files while an
  Ofolder rail is open can be overwritten by that rail's next save.
