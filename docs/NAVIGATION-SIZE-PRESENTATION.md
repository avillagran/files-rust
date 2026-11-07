# Navigation and compact folder sizes

Production checkpoint: `74984cc0ec02d5bf2d770ed59aa09491e355e973`
(presentation changes started at `23d09ed`).

Breadcrumbs show the full ancestry when it fits, label the user's home as
`Inicio`, and separate components with `›`. Overflow represents hidden ancestors
only; the current directory stays visible. In the narrowest address field that
named current directory opens the ancestor menu itself, with a dropdown caret
when space permits. Destinations remain raw paths independent of displayed text,
and Alt+Down continues to open the ancestor menu.

Folder rows omit storage implementation labels. An approximate observation uses
`≈`, a confirmed recursive sum uses `∑`, and partial progress uses `≥`. Before a
value exists, a small `…` occupies the size position. Approximate observations
remain excluded from confirmed-size sorting; freshness and scheduling are unchanged.

Group cards now use this compact size label, rather than the inspector's verbose
measurement detail. Card content is clipped to its interior. Detailed information
remains available in the inspector and properties instead of occupying the card.

Presentation verification covers 29 reviewed targeted tests: eight breadcrumb layout/caption
cases, twenty size-label/provisional cases and one rendered folder-card regression.
Workspace/all-targets strict Clippy and rustfmt on the five changed Rust files pass.
Final native builds take 70.333 seconds on NODE and 113.471 seconds on M2, with 1054
exported source inputs verified before and after the native ARM build.

Four private source profiles and two installed profiles pass on NODE and M2.
Both installations retain five verified backups and accepted native artifacts.
See [the evidence bundle](evidence/navigation-size-focus-20261107-74984cc/README.md)
for captures, receipts, retained failures and exact validation scope.
The visual fixture deliberately delays size measurement through a bounded private
helper proxy to capture the in-progress layout; it does not measure performance.

Returning from a child with Backspace or Up also restores the child directory's
selection and keyboard focus in its parent. Restoration waits for the listing,
uses the raw child name, and is tied to the active owner, path and generation.
A bounded in-memory position ledger retains the earlier scroll where compatible;
revealing the selected item takes priority. Later user input cancels the pending
restoration. This state is not written to preferences or reused as file-operation
authority.

Five additional keyboard tests cover raw child names, owner/generation changes,
same-folder refresh, multi-level returns, input cancellation and a missing child.
The return regression runs two full paint cycles with a zero content height left
from the child. Idle wheel state must preserve the restored offset until the
current layout can clamp it. Native testing caught the original defect: selection
returned correctly, but an idle scroll update reset its position using the old
layout and hid the selected folder. That failed candidate was not installed.
