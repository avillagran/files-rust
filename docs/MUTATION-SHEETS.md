# Mutation sheets

The current source presents create-file, create-folder and rename forms with
toolkit `overlay::Sheet`, native `TextField` inputs and toolkit buttons. The
existing model, planning worker, transaction code, history and result reviews
remain authoritative. A sheet is a presentation of that workflow.

Files exports exact committed toolkit `dd481f1`: 480 upstream files plus
Files-owned [UPSTREAM.json](../vendor/lib-fltk-omarchy/UPSTREAM.json), including
the live `bind_dismissible` / `is_dismissible` policy. See the toolkit
[SHEET contract](../vendor/lib-fltk-omarchy/docs/SHEETS.md). No WIP or vendor
patch is part of that export.

This document records implemented source behavior and acceptance requirements.
It does not claim completed Files compilation, native acceptance or NODE/M2
installation for this new adapter. The earlier asynchronous-presentation
delivery retains [its own scope](evidence/async-presentation-20261106/README.md).

## Editing and layout

Create forms have one name field. Rename has pattern, start, step, search and
replacement fields, case and regex controls, and a before/after preview.
Enter in a field queues Apply; focused buttons retain normal Enter/Space
behavior. Native inputs own composition, selection, caret, clipboard editing
and undo. The adapter adds no second editor or Tab router. Toolkit modal
traversal contains Tab/Shift+Tab and restores focus when the sheet closes.

Each complete field is limited to 2,048 UTF-8 bytes before copying it into
the inbox. The model retains CR/LF/NUL cleanup, name validation and existing
mutation limits. The native Input's defensive character limit does not
replace this byte check. Rejected edits restore the accepted model value
without submitting it or invalidating its prepared revision. Unicode and
percent characters remain literal input, never percent-decoded display text.

Matching native values are left alone during synchronization, preserving
composition, caret and undo. Required replacements use the native setter
without another edit callback, restoring positions at UTF-8 boundaries.
A new logical dialog receives fresh controls and editing history. Temporary
coverage can reuse the same session's retained content; captured focus and
selection can survive its cold reconstruction.

The form scrolls vertically, stacks secondary fields at narrow widths and
reveals focused controls after focus/viewport changes. Preview snapshots copy
at most 32 bounded rows from the prepared plan, with no widget per file or
full-selection clone. Directory, preview and diagnostic labels are bounded;
the model and worker retain exact paths and names.

## Callback identity and ownership

`mutation_sheet_state::Controller` publishes snapshots and drains a bounded
inbox. Native callbacks enqueue values or typed requests; they never borrow
`Model` or start mutations. Eight requests can be retained. Adjacent edits
with the same field/stamp coalesce without crossing Apply. Cancel/Dismiss
can remove queued work for that stamp so typing bursts do not disable them.

A request carries a logical dialog epoch, draft revision, pane, tab and
generation. The epoch is independent of changing Plan/Apply worker IDs.
The controller retains the selection allocation while observing it, so a
closed/reopened dialog cannot inherit callbacks from the old allocation.
Changed ownership, draft or competing surface rejects old input. Edits
admitted in the same batch may update that batch's draft before its Apply.

The loop drains requests before pumping mutation results, then snapshots
under the model borrow. It publishes State and opens, updates, focuses or
closes widgets only after releasing that borrow: listeners and native
focus/dismissal callbacks may run synchronously. Programmatic closure clears
the active stamp first, preventing teardown from cancelling a replacement.

While the native sheet owns input, canvas text, pointer, wheel and navigation
events cannot reach the legacy form or browser underneath. Existing guards
for competing dialogs, reviews, copy work and closing remain. Disabled
controls are presentation; controller/model admission is checked again when
queued requests are drained.

## Planning, Apply and cancellation

A Plan is read-only and fields remain editable while its worker is busy.
An accepted edit cancels the old preview, invalidates its prepared plan and
leaves the old response to be drained. Existing debounce and process-global
worker IDs prevent stale results from becoming a newer draft's preview.
Rename Apply requires a current nonempty plan, no draft change/error and an
available worker. Create retains existing validation and no-clobber behavior.

After Apply/Create is submitted, fields and Apply are disabled. Escape,
backdrop and the close glyph cannot dismiss while `mutation_applied` is
true. Cancel becomes an explicit cancellation request, leaving the form
visible until the real response arrives. It does not confirm rollback, undo
a committed Create or turn a successful result into failure.

Before Apply, dismissal cancels planning and closes the logical dialog while
its worker response is drained. Programmatic/owner teardown can remove the
UI despite the user dismissal gate; application closure still cancels and
drains workers. Widget destruction is never an operation ACK.

Real completion uses existing history, refresh and feedback paths. Required
error, partial-result, recovery and acknowledgement reviews keep their
current behavior. Neither sheet dismissal nor a notice acknowledges those
reviews, releases their queues or authorizes a retry. Properties and other
modal surfaces are not migrated by this adapter.

## Rendering and scope

The old canvas form is suppressed only when the toolkit sheet is managed.
If it cannot open, the existing model-owned canvas form remains available.
Result/recovery surfaces retain their current renderer and controllers.

CPU rendering remains the default. Sheet uses shared toolkit motion, theme
and reduced-motion policy; no GPU, accessibility or media feature is enabled.
The approved [v3 mockup](../mockups/v3.html) is a visual/theme reference, not
evidence of widget behavior or a replacement for operation/focus contracts.

This change does not establish full application i18n, screen-reader support,
native macOS/Windows behavior, every IME/layout combination or complete
Nautilus/Finder parity. Resource/startup gates remain separate.

## Private worker gate

Optional Linux instrumentation in `rename_job.rs` requires both
`FILES_TEST_MUTATIONS=1` and `FILES_TEST_MUTATION_GATE_DIR`. The gate must be
an existing strict descendant of explicit `TMPDIR`, with root and descendant
directories owned by the current UID and mode `0700`. Every component is
opened without following symlinks. Signal operations use the retained
directory descriptor. Live configuration is not an alternative root.

The worker captures this directory when first started; arm files can change
afterward. Driver control files must be private regular files of that UID,
normally `0600`.

| Signal | Meaning |
|---|---|
| `<phase>.arm` | Enable that phase for entering requests |
| `<phase>-<id>.waiting` | Worker reached the gate |
| `<phase>-<id>.release` | Driver permits continuation |
| `<phase>-<id>.cancelled` | Cancellation observed; not confirmation of rollback |
| `<phase>-<id>.finished` | Released, cancelled-before-execution or timed-out |

Pre-execution phases `plan`, `apply` and `create` leave promptly on
cancellation; the normal operation handles that same flag. Optional
`ack-plan`, `ack-apply` and `ack-create` run after computing the actual result.
They can hold its unchanged response through cancellation to exercise stale
delivery after close/reopen, releasing on a signal or a real ten-second
elapsed deadline. Signals contain only phase/ID/control data and are retained.

The gate never fabricates a plan, success, rollback or failure, modifies an
operation response or changes operation files. Cancellation after a completed
Create/Apply cannot retroactively make it unsuccessful. Fixtures can therefore
distinguish cancellation intent, real filesystem effects and waiting for ACK.

## Acceptance requirements

The private native driver is
[mutation-sheet-nested-check.py](../scripts/mutation-sheet-nested-check.py).
Against final source and installed artifacts it must exercise native
text/paste/undo, focus/Tab containment, idle dismissal/reopen, literal Unicode
and percent names, conflicting create, non-UTF-8 preservation, stale Plan ACK,
real rename/create effects and cancellation while waiting for ACK. Run normal
and narrow viewports with motion on/off; retain actual mapped dimensions and
failed instrumentation with its scope.

Unit coverage must distinguish byte bounds, coalescing/action order, stale
dialog/owner/revision, editable Plan, disabled Apply and actual response
draining. Existing tabs, keyboard, notice, external-navigation, preview-copy
and reviewed Sidebar-transfer regressions remain relevant.

NODE results do not establish native ARM Linux M2 acceptance. Build the final
ARM artifact on M2, validate private fixtures, verify backups and all five
installed artifacts, install only that validated version and run scoped
installed checks. Those pending steps are not marked complete here.
