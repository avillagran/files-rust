# Persistent notice toolkit export

Files exports the committed upstream tree
`0f680890cba7038fa0519be19d159eed4f433a25`, based on the preceding exact
`a029d45c1fc63f99763f9836d1af0c817ab82ffd` export. The baseline 424 files
matched their committed bytes and executable modes before modification.
The new tree contains 430 source files: 13 changed paths, six added files and
no removals. Files owns the additional `UPSTREAM.json` provenance record.

`vendor-manifest.json` records each upstream blob, SHA-256, byte count and
Git mode, plus the source tree and archive hash. All 430 exported files
match that manifest. The upstream working tree was never copied or edited;
its uncommitted platform/native changes remain excluded. Cargo manifests,
lockfiles and enabled optional features are unchanged. GPU, accessibility,
images, playback, QR and Mermaid remain disabled in Files.

## Adapter API review

`overlay` publicly exports `Notice`, `NoticeAction`, `NoticeDismiss` and
`NoticeHandle`. The types are available with the existing default feature
set and the app's current `fltk` version. `Notice::show(&anchor)` returns
`Option<NoticeHandle>`; missing/deleted host cases must remain nonfatal in
the Files adapter. Files already installs the toolkit focus router.

Showing a card does not take focus or start an expiry timer. Action IDs are
stable, nonempty and unique; enabled/disabled status is selected when the
card is built. Summary, body and action labels can bind to live
`State<String>` values. The body displays at most two wrapped lines, so long
operation reports still need the app's explicit details surface.

`on_action` runs on the UI thread without a toolkit callback borrow and does
not dismiss automatically. It must enqueue Files requests, then validate the
current operation identity and admission rules before execution. The
`NoticeDismiss` reason distinguishes app-driven closure from the Close hit
and keyboard dismissal. Dropping a handle leaves the card open: the adapter
must explicitly close owned cards before dropping/replacing them.

`rect()`, `action_rect(id)` and `close_rect()` expose actual logical-window
geometry for private acceptance. `focus_action(id)` focuses without
executing an action; `activate(id)` follows the same callback path as input.
Neither replaces native keyboard/pointer acceptance. Window deletion drops
card callbacks without invoking application actions. Shared overlay motion
and per-window stacking remain toolkit-owned.

This worker verified source/API consistency and exact export provenance. It
did not run Cargo compilation, toolkit UI tests or native GUI acceptance;
the integrated Files checks must establish those results separately.
