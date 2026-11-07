# Navigation, folder sizes and parent focus — 2026-11-07

Production source: `74984cc0ec02d5bf2d770ed59aa09491e355e973`.
Native acceptance driver: `2b102be` (SHA recorded in the report indices).

Files is installed on NODE (`e5520ef67e9f`) and native ARM M2 (`0d4ea36c2068`).
All five prior artifacts were backed up and hashed before replacement; independent
backup/installed checks and configuration/association audits pass. Native HTML
helpers retain the verified recipe. Existing application windows were not closed.

Four source profiles and two installed profiles pass on private Sway sessions,
covering normal and narrow windows. Raw reports and separate visual receipts are
retained. The private proxy delays only size measurement, then executes the exact
verified native helper; its source and hashes are retained. This captures genuine
in-flight model state without claiming a performance result or an unmodified
helper launch sequence.

The captures show readable current breadcrumb names, separators and an ancestor
menu opened with native pointer input. Size labels contain no cache word and
Group cards show a small pending indicator rather than verbose measurement text.
Approximate/partial/confirmed markers retain their distinct meaning. Enter into
Folder20, followed by Backspace, restores its selection, browser keyboard focus
and visibility. Normal scroll is 340 before/after; narrow scroll is 490 before/after.

Thirty-four reviewed targeted tests pass: eight breadcrumb cases, twenty size
label/provisional cases, one folder-card regression and five keyboard cases. The
parent-return regression executes two paint frames with the child's obsolete
zero content height and requires the retained scroll and visible selected child.
Strict workspace/all-targets Clippy and formatting of changed Rust files pass.
No legacy full suite, stopped toolkit incident harness or CI was executed. Resource,
startup, every view/platform combination and complete Nautilus parity are outside
this acceptance.

Retained failures include an obsolete pause-label expectation, a wrong marker
channel, clicks issued before pointer motion had settled, and the real failed
parent-return candidate `7bf59df`. Its selection/focus returned, but idle wheel
advancement clamped the restored scroll against the previous layout and hid the
selected folder. That candidate was not installed. The final fix leaves idle
programmatic offsets for the current painted layout to clamp. The root no-op unit
case was also changed to avoid starting a read-only listing of the host root.
No fixture trees or failed evidence were deleted.

Initial presentation candidate `23d09ed` passed its narrower UI scope before the
user added the parent-focus requirement. Its reports remain as historical evidence,
not acceptance for the final feature. NODE final release build took 70.333 s; M2
113.471 s with 1054 exact source inputs verified before and after. Intermediate
build logs remain. Source remained frozen during compilation and acceptance.

Manifests hash every retained bundle file. The remote archive was SHA-verified
before each member was checked and copied exclusively. Large duplicate PPMs,
compiled tools and filesystem fixtures are omitted; private originals remain.
No source push, PR update or publication occurred. The separate NODE titlebar
plugin update still awaits system authentication; these Files UI changes install
at user level and do not assert that the pending plugin was replaced.
