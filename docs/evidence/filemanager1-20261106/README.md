# FileManager1 acceptance — 2026-11-06

Application source: `0c9a59c`, native driver/test-isolation rules: `8107d36`,
and same-location watch reconciliation: `cbc83a9`. Toolkit remains the exact
`a029d45` export, with optional GPU, accessibility and media features off.

The final workspace run passes 1,710 tests (730 core, 113 helper, 867 UI;
3 ignored), Clippy and the native release build. Model tests run with private
configuration/data roots; the new gesture fixture disconnects auxiliary
persistence. The watch regression preserves the full external name set when
ordinary Snapshot restoration has found only a subset before a refresh.

## Functional scope

FileManager1 uses a private session-bus connection and exclusive, explicit
`--file-manager-service` ownership. Native private-bus checks cover all three
methods, strict signatures/limits/URIs, raw Unix names, hidden targets,
multiple parent tabs, paged properties, read-only root metadata, protected
modal rejection without replay, duplicate ownership, the three CLI modes and
clean service closure. Source reports and post-install reports record exact
app/helper hashes. Private fixture contents and permissions remain intact.

The regression suite also covers native keyed tabs, eight frozen-clock motion
snapshots, keyboard selection, preview copying and real reviewed Sidebar
transfers. Rendered motion snapshots force complete frames; they do not prove
native partial-damage rendering. GUI work uses private compositors/buses.
Mapped dimensions and requested sizes are separate: FileManager1 captures
include a real 1000×650 viewport and a 508×325 narrow viewport on NODE.

## Final installed artifacts

| Machine | App SHA-256 | Installed and backup audit |
|---|---|---|
| NODE x86_64 | `95600f0976a77c4aecd007807291fe5cceea2811bec2a0b92b920b93b2f1d17e` | [five artifacts](node/installed/artifact-audit.json) |
| M2 native ARM Linux | `75ea16c6bacc7f24e237e475eac1b6baa7b0396a1039e7de6439710aa9bbb5b1` | [five artifacts](m2/installed/artifact-audit.json) |

Each machine passed six source scopes and three post-install scopes. The
native HTML helper was unchanged and reused only after recipe verification.
NODE reports are grouped under [source](node/source) and
[installed](node/installed); M2 reports, captures and provenance are under
[m2](m2). The private driver supports both Lua and legacy Hyprland dispatch
syntax. A prior legacy-only geometry failure on M2 is preserved; it did not
require an application change or a rebuild.

## Earlier candidates and corrections

`node/candidate-final2` and `node/candidate-build3-source` retain passing
pre-fix candidates. `node/candidate-build3-install` records the interim NODE
installation and its five-artifact audit. Static review subsequently found
that a watcher refresh could discard the pending all-or-nothing selection
check; `cbc83a9` fixes that edge and supersedes those candidates. The M2
pre-fix candidate was preserved without installation.

The `earlier` directory retains harness expectation failures: hidden smoke
output did not imply a mapped window; owned-file properties may be editable;
and modal rejection uses a generic D-Bus Failed error. A requested floating
width that the compositor did not provide is also retained as a harness
geometry failure. These results are not final acceptance.

Captured stdout/stderr logs retain their original bytes, including trailing
spaces in compositor ASCII banners and test-runner output. Source/document
whitespace checks exclude those raw text artifacts rather than rewriting them.

The `checks` directory retains compilation/lint failures and their corrected
runs. One newly added model fixture initially used real AuxIo persistence
and created `~/.config/files` with hidden-file preferences, causing seven
unrelated tests to fail. The fixture now disconnects persistence and full
runs isolate configuration/data. A pre-run hourly snapshot and matching
directory/test-temporary-file birth times proved that the live configuration
directory had not existed before the test. The two test-created files and
their directory were moved into the incident backup, restoring the original
absence without deleting a tree. The restoration record and successful
private rerun are retained; native post-install checks verify continued
absence. No personal file data was removed.

## Limits

Automatic bus activation/default registration is intentionally not installed.
The source-only service template is optional. StartupId is bounded but
presentation remains best-effort `show()`; platform token consumption is
pending the public toolkit API. Initial local libdbus connect/authentication
has no hard total deadline, unlike established-call/helper admission.

See [the implementation contract](../../FILEMANAGER1.md). Complete
Nautilus/Finder parity, macOS/Windows, accessibility/i18n and the remaining
roadmap stay open. Resource/startup gates remain `NOT_MEASURED`.
