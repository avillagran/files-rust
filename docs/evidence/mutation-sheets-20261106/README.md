# Controlled mutation sheets: suspended validation

This report records the suspended validation as of November 6. It is **not a
complete release acceptance**. On November 7 the user requested an M2 test
installation; the later native backend repair, private acceptance and M2
installation are recorded in [the separate M2 report](../mutation-sheets-20261107-m2/README.md).
NODE retains the earlier asynchronous-presentation installation.

Files source `f310421` introduces native create/rename sheets, bounded draft
admission and preview presentation, editable planning, and retained UI while
Apply/cancellation waits for the real worker ACK. The model/controller source
at `ac070dc` passed 1,744 workspace tests (three ignored), formatting and strict
Clippy. Later presentation/probe changes have not completed native acceptance.

Toolkit `dd481f1` is the current exact vendor export. Its manifest records
480 upstream files plus the Files-owned provenance record. Disabled-control
candidate `23a5b7f` remains in an isolated toolkit worktree and was **not**
exported into Files. A static correction to its active TextField border is
recorded separately as `f2fdc2b`; it has not been executed.

Static harness repair `d2bfb5a` creates a new private retained root, rejects
symlink/reparse components, and replaces five tree and six exact-file cleanup
sites with preservation by rename. It has not been compiled or tested. The
repair was presented for explicit user authorization before any resumption.

While authorization remains pending, Files `9b963ae` hardens the private
input/fixture driver and `7df6769` hardens comparison reads, output and
decoder budgets. These commits have only static review; no script, compiler,
GUI, decoder or test was executed to validate them after the stop. Automatic
goal continuation is not interpreted as resume authorization. Additional
Unix/macOS/Windows fixture-parent checks are being reviewed separately.

The isolated toolkit branch now records static-only `a85c591` for platform
fixture ownership/ACL and CI setup, `9695ad5` for retained entrypoint
documentation, and `100a55a` for unit fixtures and owned displays. The latter
removes seven library-unit recursive cleanup sites, prepares five fixture
contract tests, repairs two native scripts and preserves all eight original
X11 CI command argument sets behind `script/retained-x11`. Wayland keeps its
own runtime, logs and exact child-process lifecycle. No Cargo, build, test,
PowerShell, GUI or CI execution validated these changes; only static review
and explicitly scoped source formatting occurred. Hostile-ACL Windows
runtime coverage remains unverified. Three template cleanup sites and the
separate `frame-bench` wrapper remain outside this repair and are not
alternative validators while execution is stopped.

## Retained candidate results

- NODE candidate `d734ce7` compiled and passed one native 1000 by 650
  preflight. It predates the owner-close/ACK and later presentation fixes.
- ARM candidate `f1ce00c` compiled natively on M2, with source/archive and
  installed/configuration baselines verified. It was neither tested in the
  GUI nor installed.
- Toolkit candidate `23a5b7f` passed the default UI suite and a real-input
  X11 Sheet guard check. Its new disabled controls passed their checks.
  The all-features suite and a later isolated directed candidate run failed
  reopened-Sheet GPU/CPU parity at maximum RGB delta 13. A directed baseline
  passed; this alone does not establish the cause.
- A directed candidate invocation reused the baseline executable from a
  shared target. Its PASS is excluded from candidate evidence.

## Stop and evidence limits

After those executions, the inherited toolkit UI harness was found to call
`remove_dir_all` on its fixed `CARGO_MANIFEST_DIR/build/test-ui` root and four
theme-fixture trees. All tests, GUI runs, builds and installation in this
workflow were stopped. The user was informed; no test command was retried
after detection. Repair and review remain static pending explicit resume
authorization.

Subsequent pre-detection runs had erased earlier screenshots in those fixed
roots. Current screenshots belong to the later directed executions; they
are not presented as images from the original full-suite failure. External
logs survived. Static paths and logs provide no evidence of deletion outside
the two toolkit fixture roots; a complete historical filesystem or symlink
audit is unavailable. NODE's five installed artifacts still match its
baseline, and its Files configuration remains absent.

[stopped-validation/manifest.json](stopped-validation/manifest.json) records
105 retained files and their byte counts and hashes at collection. Files
preflight failures for missing readiness instrumentation, Unicode log offsets
and an overly granular native Undo expectation are retained with their
original reports. None of the candidate or suspended results establish final
NODE/M2 acceptance, motion parity, installation, accessibility, other OS
runtime behavior, resource use or startup performance.
