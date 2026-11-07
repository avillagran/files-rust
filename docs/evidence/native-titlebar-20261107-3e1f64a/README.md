# Native titlebar integration — 2026-11-07

Files source: `3e1f64ab11da392313787c56ca5459883145876a`.
Acceptance driver: `32386d8`. All artifacts were built natively.

| Machine | Files candidate | Plugin candidate | Private visual acceptance | Deployment |
| --- | --- | --- | --- | --- |
| NODE | `3e7b52f3386f` | `653ee0dfc0f8` | PASS | Waiting for system Polkit authentication |
| M2 ARM Linux | `563b83865d29` | `9a28285b2e60` | PASS | Installed, mapped, audited and post-install visual PASS |

The observed titlebar has the folder icon, one dimmed `Files ›` prefix, and a
bright absolute path. The path contains `files`, exercising the corrected prefix
heuristic. Captures show the same window before and after plugin unload/load,
and native Alt+Up updates its title. The focused-window scope is explicit;
unfocused retitling was not tested. Raw reports remain CAPTURED until a separate
visual receipt records the actual image inspection.

Files publishes path/status only. The plugin derives name and icon from the
desktop entry, and owns the separator and styling. The original NODE plugin and
its seven local working-tree modifications remain preserved; the candidate is a
separate snapshot with the absolute-path prefix fix. M2 integrates PR #11 at
94ee0bd plus that same fix, preserving its existing feature set. NODE retains its
additional glass/stencil/application button support.

One reviewed pure title test, strict workspace/all-targets Clippy, scoped rustfmt
and both native release builds pass. NODE build took 87.636 s; M2 108.423 s,
with 1053 Files source inputs verified before and after. Native helper and HTML
artifacts are unchanged and their recipe is verified. This is not a full suite,
startup/resource measurement or a global desktop acceptance result.

M2 backs up and hashes its plugin before replacing its inode atomically. The
reloaded compositor keeps the same PID/start time, active window, all nine window
geometries/states, prior plugin options and configuration errors. All plugin maps
match the new installed inode. Files' five prior artifacts are backed up and
hashed before replacement; their backups and installed hashes independently
match the validated release. Eight preference/session/layout/association paths
are unchanged. Original Files windows retain their old executable/title behavior
until reopened; no application window was closed or controlled on the real desktop.

Private graphical validation uses native KWin 6.8.1 with its virtual backend as
the parent of nested Hyprland. KWin is extracted into a private payload rather
than installed on either system. Its advertised xdg_wm_base 6, compositor 6 and
seat 11 satisfy Aquamarine. Socket owner/PID identity is checked before IPC,
keyboard input and captures. HOME, data, config, cache, temporary paths and bus are
private, and runtime paths are short enough for Unix sockets.

Retained setup failures: pure headless Hyprland could not create its allocator;
then Sway 1.12 advertised xdg_wm_base 5 where Aquamarine requires 6. Both aborted
before the plugin and Files. A registry preflight now rejects that combination
without starting Hyprland. The first attempt inherited the host cache root and
created its own crash report there; it remains preserved. Subsequent cache roots
are private. No evidence was deleted. The original titlebars verifier uses tree
cleanup and was not run; neither was the stopped FLTK toolkit incident harness.

A first remote evidence archive stopped on a missing optional payload preparation
script before manifest completion. That partial archive remains private; the v2
archive omitted the nonexistent optional file, and every transferred member was
verified against its recorded SHA. This packaging failure is not a GUI PASS.

Copy manifests preserve original paths, lengths and hashes. Plugin source,
licenses and build/source manifests are retained; no compiled executable, real
user data or core dump is included. The new source remains local: no GitHub push,
PR update, CI or publication was performed.
