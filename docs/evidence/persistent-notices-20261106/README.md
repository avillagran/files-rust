# Persistent operation notices acceptance — 2026-11-06

Application source: `893b25a`; native driver, reproducible pixel comparator
and implementation documentation: `ddc5a9c`. The toolkit export is exact
`0f68089`: 430 upstream files plus Files' own provenance record. The
[manifest](vendor-manifest.json) verifies bytes, Git blobs and executable
modes. Uncommitted upstream work and later `767235b` presentation support
are outside this export. Optional GPU/accessibility/media features stay off.

## Contract and scope

One current feedback record owns at most 1 KiB of UTF-8 card text and one
active native card. Eight queued requests carry only the displayed revision,
origin and action. Typed successful mutation results may expose current
Undo/Redo; copy/move/archive cards may open only their exact retained result.
Information acknowledges without inheriting another result's actions.
Closing a card does not discard reports, acknowledge modal recovery or
release a held queue. Required reviews retain their existing UI/controller.

Toolkit callbacks never borrow Model or start filesystem work. Requests are
revalidated before and after native card closure/focus restoration outside
Model borrows. Read-only properties preserve preceding feedback; actual
permission changes keep their existing result/history behavior. The menu
and command palette preserve access to the last retained transfer report.

The final workspace passes 1,722 tests (730 core, 113 helper, 879 UI;
3 ignored), Clippy, formatting and native builds. Twelve new contract and
admission tests cover provenance, exact result identity, stale/duplicate
callbacks, bounded text/requests, modal/queue protection and dismissal.

## Native acceptance

Each machine runs three notice profiles: ON normal, OFF normal and a smaller
mapped viewport. Input is delivered on a private nested Wayland compositor
with isolated HOME/config/data and a private bus without service activation.
Showing feedback preserves browser focus; Ctrl+Alt+N focuses the card,
Escape restores browser interaction, and real reviewed rename/copy setup
produces Undo and View report. Cancelling a later rename preview advances
request IDs without invalidating the earlier history. Installed acceptance
also exercises Undo → Redo → Undo on real owned fixture data.

The comparator verifies matching app/helper identities, viewport, message,
selection/focus state and card rectangle before comparing its actual crop.
ON opening/intermediate/settled frames differ, OFF reaches its endpoint in
the input frame, and ON/OFF settled card pixels match exactly. Whole-window
pixels include variable status metrics and are not the comparison scope.
These captures do not certify native partial-damage rendering or every
intermediate animation frame.

Normal captured viewports are 1000×650. The requested 600×650 profile maps
to 508×325 on NODE and 810×449 on M2; those actual dimensions are recorded.
Normal windows can extend beyond the nested output, so their interaction
checks use native keyboard input. Pointer actions are exercised within the
mapped fitting viewport; an out-of-output pointer attempt was refused and
preserved as a harness failure, rather than misreported as app failure.
M2 PNG capture dimensions equal its logical window dimensions; the complete
card crop needs no HiDPI coordinate conversion.

Six additional source regressions pass on each architecture: FileManager1,
keyed tabs, eight rendered tab-motion frames, keyboard selection, preview
copying and real reviewed Sidebar transfers. Notice, tabs and tab motion
are checked again on the installed binary. Physical platform coverage beyond
these private Linux sessions remains separate.

## Installed artifacts

| Machine | App SHA-256 | Installed and backup audit |
|---|---|---|
| NODE x86_64 | `478975afde6fa35fb0bbbb9548341888a3aad585a5aecefc2da900f426812693` | [five artifacts](node/installed/artifact-audit.json) |
| M2 native ARM Linux | `5e7342a887e692bbd6ba23ae47e34d2464ac0f8e78b77e7a73f2bedb0acb53f5` | [five artifacts](m2/installed/artifact-audit.json) |

Native HTML helpers were unchanged and reused only after recipe verification.
The M2 build preserves all 1,135 base source files, the exact full toolkit
vendor including its upstream evidence receipts, and the two GUI drivers as
a separately hashed `ddc5a9c` overlay. No NODE ELF was sent to M2. Its GUI
transfer manifest verifies 116 copied evidence files byte-for-byte and by SHA.

## Evidence organization

`node/source` and `node/installed` contain the corresponding native reports,
PNG captures and original logs copied with `.txt` extensions. M2 acceptance
and build provenance are under `m2`. `checks` retains successful Rust/lint
runs and the initial outdated action-count expectation (23 rather than 24)
that was corrected together with the new compact menu label. `earlier`
retains preliminary reports and safe pointer-geometry refusals.

Raw stdout/stderr preserves original bytes, including compositor ASCII
banner whitespace. Source/document whitespace checks exclude those immutable
text logs. Private fixtures and original remote logs remain preserved.

## Remaining work

`767235b` now provides native presentation with explicit startup context;
Files has not adopted it in this delivery. Full i18n/accessibility, tear-off,
other platforms and the remaining Nautilus/Finder roadmap stay open. This
functional delivery keeps resource/startup gates at `NOT_MEASURED`.

See [the implementation guide](../../PERSISTENT-NOTICES.md).
