# M2 private acceptance and user-requested installation

M2 now has source `e4dab48caca8b4977dade49dc9e957810f95ea39`,
app SHA-256 `445570ece0749548677250c74d33b0f420b8fdece68b603c93bbd6ad6ac350ca`.
Native private acceptance and the installed-binary check both passed.
The five artifacts were backed up and hashed before individual atomic
replacement; installed hashes match the accepted artifacts. The unchanged
native HTML helper, license notice and recipe stamp were reused.

See [acceptance](guard-e4dab48/scoped-acceptance.json),
[installation and backup](guard-e4dab48/installed.json), and
[installed check](post-install/report.json). Existing user processes were not
controlled. Settings, session, three layouts and MIME associations retained
their baseline hashes. Open a new Files window to use the new executable.

The user explicitly requested an M2 installation for manual testing on
2026-11-07. This authorizes the native build and private Files acceptance
recorded here. The toolkit test harness involved in the earlier incident
remains stopped; this work does not run it or validate its static repairs.

The first candidate's binary source was `7ec4ed54e6344dab5ba814063bb68315fc21c202`
(product changes through `f310421`), using the unchanged toolkit export
`dd481f1`. The source manifest covers 1,032 tracked build-input files;
documentation and mockups are excluded. All included paths, bytes, Git blob
identities and executable modes were verified before building, and source
bytes were checked afterward. No NODE executable was transferred.

The native ARM release build passed. The unchanged native HTML helper was
reused only after checking its recipe fingerprint and binary stamp.
The initial CMake library discovery failure and a subsequent cached negative
`trunc` check are retained in separate logs. Setting `CMAKE_LIBRARY_PATH` to
the native pkg-config library directory and preserving the stale CMake cache
before configuring again resolved them without source or registry edits.

Full regression, motion parity, resource and startup gates
are outside this scoped installation; no complete Nautilus/Finder parity is
claimed. NODE remains on its previous installation.

## Rejected candidate and native backend repair

Candidate `7ec4ed5` was not installed. Private Sway acceptance found a real
SIGSEGV in the pinned FLTK Wayland keymap callback: a zero-length mapping
returned `MAP_FAILED`, which reached XKB string parsing after release-mode
assertions were compiled out. The core establishes the zero size and invalid
pointer; the event format and exact keyboard-device source are not proven.
The initial five installed hashes and configuration/association baselines
remained unchanged. An existing user Files process was left alone.

Source `e4dab48caca8b4977dade49dc9e957810f95ea39` adds a pinned,
generated-source repair without editing the registry or vendor. It rejects
unsupported, empty, oversized, truncated, unmappable and unterminated keymaps,
closes descriptors and publishes a replacement only after map and state
creation succeed. Three callbacks avoid decoding without an initial state.
The v4 build entrypoints and link guard require both patched native objects
from the same build inputs. The corrected candidate passed its own private
acceptance before installation. A dedicated malformed-keymap protocol fixture
was not executed; the exact zero-length event origin remains unproven.

## Private acceptance scope

Both pre-install and installed checks passed nine scoped checks: native
focus/caret/selection; Unicode paste and text undo; create file/folder;
cancel and retained Apply until real ACK; conflict preservation; raw-name
rejection without lossy mutation; stale Plan ACK; single rename and undo.
The same run's create and preview captures were inspected for legibility,
clipping and duplicate sheets. A private Sway session provided native input;
Hyprland and motion comparison were not run.

Failed setup and adapter attempts were retained. In particular, minimal
virtual keycodes in XKB's 10–19 range collided with FLTK's physical number-row
handling. Reserving unused keymap entries moved real test keys above that
range; the original focus, selection, clipboard and undo checks then passed.
The rename undo check was also corrected to wait for closed Sheet and worker
ACK, rather than racing the first appearance of the destination file.
