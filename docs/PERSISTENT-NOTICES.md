# Persistent operation notices

Files uses the toolkit's `overlay::Notice` for compact operation feedback.
The notice adapter originally adopted exact committed toolkit `0f68089`.
The current source exports exact toolkit `fac0705` (467 upstream files plus
Files-owned provenance), retaining that notice API and adding manually polled
native presentation. The included `8be476b` Sheet API is not adopted by
Files. See [WINDOW-PRESENTATION.md](WINDOW-PRESENTATION.md).
Optional GPU, accessibility, images, playback, QR and Mermaid features
remain disabled. The original notice export's 430 upstream files and Files'
`UPSTREAM.json` are recorded in its
[export manifest](evidence/persistent-notices-20261106/vendor-manifest.json)
and [API review](evidence/persistent-notices-20261106/vendor-review.md).

This document describes implemented behavior and the acceptance contract.
Release, installation and native-validation results belong to the
[delivery evidence](evidence/persistent-notices-20261106/README.md); the
presence of the implementation alone does not establish those results.

## Everyday behavior

A notice appears over its containing window without taking browser focus.
It has no expiry timer. Files keeps one current feedback record and one
active operation card; a newer result replaces the older presentation.
This is not a new operation-history list or a second worker-result queue.

The card body retains at most 1 KiB of UTF-8 text, adding an ellipsis on a
character boundary when necessary. The toolkit displays at most two wrapped
body lines. Detailed copy/move/archive output stays in the existing single
retained worker result and its report surface, rather than being copied into
the card. Long output therefore needs the explicit report action.

| Feedback origin | Card actions |
|---|---|
| Informational message | Acknowledge only |
| Typed successful mutation result | Undo or Redo when the current worker history permits it, plus Acknowledge |
| Typed copy/move/archive result | View report for that exact retained result, plus Acknowledge |

The Acknowledge action, Close hit and focused-card dismissal hide the card.
They do not clear the model's message, detailed report or retained result,
cancel a worker, release a held copy queue or submit an operation. Dismissing
an unchanged message keeps it hidden. A new typed completion is a new event
even if its displayed text is identical.

The last retained copy/move/archive report remains reachable through the
Actions menu and the command palette's `last-operation-result` entry after
card dismissal. Those routes still use the existing busy/admission guards;
they do not bypass an operation already in progress.

## Keyboard, pointer and focus

| Input | Behavior |
|---|---|
| Ctrl+Alt+N | Focus the card's first enabled action, or Acknowledge when it is the only enabled action; do not execute it |
| Left/Right, Home/End | Move the focused card's action cursor |
| Enter or Space | Activate its current action |
| Escape, Delete or Backspace | Dismiss the focused card |
| Primary press/release on the same action or Close hit | Activate that hit once |
| Body, middle or right click | No operation action or implicit dismissal; auxiliary clicks do not take focus |

The toolkit owns card hit geometry, action wrapping, stack placement and
focus restoration. Files uses that geometry when routing pointer input;
the canvas must not interpret an overlay click as an entry action below it.
The existing toolkit focus router remains installed. Closing a focused card
can synchronously restore native focus, so the adapter closes, replaces and
focuses cards outside a `Model` borrow.

Cards are temporarily removed while an existing edit, menu, preview, review
or incompatible operation covers the browser. Removing a covered widget is
not user acknowledgement: the current record can reappear when its context
is available. Window closure explicitly closes its card and drops pending
presentation resources without starting application work.

## Action ownership and admission

`operation_feedback.rs` owns the bounded record. Each typed publication
receives a revision and an origin (`Info`, mutation ID or retained copy ID).
Snapshots copy only the bounded body and control metadata. Mutation-history
eligibility comes from a successful typed result and current Undo/Redo
flags, not from a shared request counter that a read-only inspection may
also advance. Informational text cannot inherit an older result's history
or report action merely because that result is still retained.

`operation_notices.rs` owns the native card. Toolkit callbacks enqueue only
the displayed revision, origin and requested action. The inbox accepts at
most eight pending requests. It does not retain a second report or execute
worker commands inside a callback.

The main loop drains requests outside drawing and toolkit callbacks. It
first observes any changed informational assignment, then validates the
exact current stamp, visibility, busy/modal state and action capability.
The adapter closes its native card outside the model borrow, rechecks
admission, and only then calls the existing report or history controller.
Old callbacks cannot target a newer result, including one with identical
text. Viewing a report additionally requires the exact retained copy ID.
No action retries itself after rejection.

Read-only properties inspection preserves the preceding operation message
and history affordance. A permission Apply that actually changes metadata
can publish its own result and invalidate prior history as before.

## Existing reviews and fallback

The persistent card does not replace operation confirmation, uncertain or
failed-result review, recovery controls, truncated-report handling or held
queue decisions. Those paths keep their existing modal reports and explicit
acknowledgement requirements. While such a report is open, the card is
covered and its queued commands cannot acknowledge that report or release
its queue. Showing a card never authorizes a mutation or automatic retry.

When the native toolkit anchor is available, the old compact canvas notice
and duplicate retained-copy result bubble are suppressed. If the toolkit
cannot create its layer, the source retains those canvas surfaces as a
fallback. Dismissal still leaves the last retained report accessible through
the menu and command palette.

## Acceptance and remaining scope

Private native acceptance must exercise cards with motion enabled and
disabled, narrow layouts, browser-focus preservation, Ctrl+Alt+N and action
navigation, dismissal/focus return, real reviewed rename Undo/Redo and the
exact retained-copy report. Modal coverage and stale callbacks must not
submit commands or release held queues. The normal keyboard, tab, preview,
external-navigation and Sidebar-transfer regressions remain relevant after
the shared toolkit overlay changes.

A requested window size is driver intent, not proof of its mapped size.
Record the actual logical window size and capture dimensions separately;
a compositor may assign different geometry. End-state ON/OFF captures do
not establish every animation frame or native partial-damage behavior.
The toolkit owns existing overlay motion and its reduced-motion policy.

Native platform, language and accessibility acceptance remain independent.
These cards do not complete Nautilus/Finder parity, enable Files' optional
accessibility feature, establish full application translation or close the
resource/startup gates, which remain `NOT_MEASURED`.

The current source separately adopts exact `fac0705` for cancellable external
request presentation. It preserves startup context without logging it,
queues the admission reply before native begin and drops stale or competing
modal intent plus its native handle. Wayland phases are polled without
compositor waits; X11's first poll still performs synchronous native I/O.
`Requested` does not establish granted focus. Files asynchronous acceptance,
notice regressions after this export and NODE/M2 installation passed.
The preceding synchronous `99db7b1` / `024bb87` presentation delivery passed
its own scoped acceptance; those results and the historical `0f68089` /
`893b25a` notice delivery below retain their original scope. Optional features
remain disabled. See [WINDOW-PRESENTATION.md](WINDOW-PRESENTATION.md) and
[FILEMANAGER1.md](FILEMANAGER1.md).


## Validated delivery — 2026-11-06

Application `893b25a` and acceptance/documentation `ddc5a9c` passed 1,722
workspace tests (730 core, 113 helper, 879 UI; 3 ignored), Clippy, formatting
and native release builds. Both NODE and native ARM M2 passed three notice
profiles, the card-pixel comparator and six relevant source regressions.
Installed notice, keyed-tab and tab-motion checks also passed; installed
notice checks include the real Undo → Redo → Undo sequence.

| Machine | Installed app SHA-256 |
|---|---|
| NODE x86_64 | `478975afde6fa35fb0bbbb9548341888a3aad585a5aecefc2da900f426812693` |
| M2 native ARM Linux | `5e7342a887e692bbd6ba23ae47e34d2464ac0f8e78b77e7a73f2bedb0acb53f5` |

All five installed artifacts and their backups were hashed. Existing native
HTML helpers were unchanged and reused after recipe verification. Private
sessions preserved personal configuration, associations and existing windows.
Normal captures are 1000×650; the requested smaller profile mapped to
508×325 on NODE and 810×449 on M2. Whole-card endpoint pixel equality is
proved for ON/OFF, separately from native partial-damage behavior. See the
[evidence bundle](evidence/persistent-notices-20261106/README.md) for scope,
reports, frames, intermediate harness failures and installation receipts.
