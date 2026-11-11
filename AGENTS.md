# Files project instructions

## Isolate model and UI test persistence

Run Rust tests with a private `TMPDIR`, `FILES_CONFIG_DIR` and `XDG_DATA_HOME`
created for that invocation. A private temporary directory alone does not
isolate application preferences. Model-only fixtures that exercise gestures
must replace `AuxIo` with disconnected channels before pumping, unless the
test explicitly validates persistence inside its private configuration root.
Never use the user's live settings as test fixtures.

## Retain test fixtures

Review filesystem writes and cleanup in unfamiliar test harnesses and their
reachable helpers before execution. Use an exclusive private directory for
each run, keep fixtures and evidence, and reject symlink traversal. Do not
delete directory trees or replace a prior run's output. Namespace-removal
cases can move their owned entries aside while preserving the original data.

Every crate's tests obtain fixtures from `test_support::private_dir(label)`
(`crates/core/src/test_support.rs`, shared by the other crates): a fresh
exclusive 0700 directory under `std::env::temp_dir()` that is never deleted,
plus `assert_inside` to call before any destructive product call under test.
A guard test (`sources_have_no_home_fixtures_or_recursive_deletion`) fails when
any source file of the crate mentions the old home-based fixture root,
`remove_dir_all(` or `tempfile::` outside its justified allowlist.

Run tests with a private 0700 `TMPDIR` that contains `FILES_CONFIG_DIR` and
`XDG_DATA_HOME`, for example `TMPDIR=$R/tmp FILES_CONFIG_DIR=$R/tmp/config
XDG_DATA_HOME=$R/tmp/data cargo test --workspace`. Tests no longer write below
`$HOME`, so a private `HOME` is optional.
Put that `TMPDIR` on a persistent filesystem (not tmpfs): the exdev namespace/alias
tests compare inode timestamps and are only stable there.

If an unexpected deletion occurs, stop and inform the user. Do not retry the
command or use another runner. Check the current project journal and incident
record before resuming; fixing source does not by itself lift an existing stop.
The retained-fixture toolkit repair and its pending authorization are recorded
in `docs/evidence/mutation-sheets-20261106/README.md`.

## M2 installation (standing user instruction, 2026-11-04)

Install every new validated Files version on M2, not only on NODE. This is
already authorized; do not ask for permission again for each update.

- Authoritative development remains on NODE. Build native ARM artifacts on M2
  (`avillagran@<m2-lan-host>`); never copy the NODE x86_64 binary there.
- Validate the new M2 artifact with private fixture data before installing.
- Back up and hash all five installed artifacts before atomic replacement:
  files, files-helper, files-html-helper, its LICENSES and build.json.
- Reuse an unchanged verified native HTML helper when its recipe matches.
- Check installed hashes and run a scoped post-install check. Do not interrupt
  the user's existing windows or change default associations/settings.
- Preserve failed evidence, record the source checkpoint, and save the result
  in the project journal. An unvalidated/failed build is not a new release to
  install; report the concrete blocker rather than installing it anyway.
