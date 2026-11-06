# Files project instructions

## Isolate model and UI test persistence

Run Rust tests with a private `TMPDIR`, `FILES_CONFIG_DIR` and `XDG_DATA_HOME`
created for that invocation. A private temporary directory alone does not
isolate application preferences. Model-only fixtures that exercise gestures
must replace `AuxIo` with disconnected channels before pumping, unless the
test explicitly validates persistence inside its private configuration root.
Never use the user's live settings as test fixtures.

## M2 installation (standing user instruction, 2026-11-04)

Install every new validated Files version on M2, not only on NODE. This is
already authorized; do not ask for permission again for each update.

- Authoritative development remains on NODE. Build native ARM artifacts on M2
  (`avillagran@192.168.1.31`); never copy the NODE x86_64 binary there.
- Validate the new M2 artifact with private fixture data before installing.
- Back up and hash all five installed artifacts before atomic replacement:
  files, files-helper, files-html-helper, its LICENSES and build.json.
- Reuse an unchanged verified native HTML helper when its recipe matches.
- Check installed hashes and run a scoped post-install check. Do not interrupt
  the user's existing windows or change default associations/settings.
- Preserve failed evidence, record the source checkpoint, and save the result
  in the project journal. An unvalidated/failed build is not a new release to
  install; report the concrete blocker rather than installing it anyway.
