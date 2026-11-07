# External-request window presentation

Files adopts the exact committed toolkit `767235b` export for
`native::present`. It retains the notices and keyed tabs delivered with
`0f68089`; optional toolkit features remain disabled. The export identity is
recorded in [UPSTREAM.json](../vendor/lib-fltk-omarchy/UPSTREAM.json), and the
native API contract is in the toolkit's
[PRESENTATION.md](../vendor/lib-fltk-omarchy/docs/PRESENTATION.md).

This document describes source `9904f88`/`99db7b1`, validated and installed
on NODE and native ARM Linux M2. Earlier FileManager1 and notice delivery
results retain their original scope.

## Admission before activation

The FileManager1 broker runs bounded, read-only helper preflight and checks
the current pane, tab, generation and edit/modal/operation guards before
admitting navigation or properties. A rejected, cancelled, expired or stale
request does not produce a presentation intent.

Successful admission creates one owned intent containing the exact incoming
startup ID and the resulting view ownership. The broker queues the D-Bus
admission reply before the native presentation call. It does not promise
focus, completed enumeration, painting or properties metadata. A later
native refusal or error cannot retract the already acknowledged UI change.
The bus ticket is an admission guard, not a post-reply activation lease.
Immediate window closure or bus disconnection can prevent delivery of a
queued reply; the caller then observes disconnection rather than success.

The startup ID is an opaque string within the existing FileManager1 limits.
Files forwards it unchanged through `PresentOptions.startup_id`, including
an explicit empty string when no context was supplied. Location CLI requests
use that empty context. Neither this adapter nor the toolkit imports,
overwrites or removes `XDG_ACTIVATION_TOKEN` or `DESKTOP_STARTUP_ID`.
Diagnostics expose only context presence and bounded control metadata;
they do not copy the startup ID or native error text.

## One current intent and the first paint

`file_manager_ui::PresentationIntent` records the post-admission pane/tab
owner and generation. `window_presentation::Manager` owns at most one
pending intent. A newer admission consumes the older pending context; there
is no activation history, FIFO or replay after rejection.

For an already mapped window, the manager can attempt presentation in the
same loop iteration. For a hidden initial window, it first validates the
intent, maps the owned window and requests redraw, then waits for the paint
revision to advance on an event-loop turn. The pending slot expires two
seconds after scheduling when next checked. This is a short first-paint
gate, not a general deferred-activation queue.

The manager checks ownership and current interaction guards before mapping
and immediately before native presentation, including after restoring the
canvas keyboard target. Changed generation, navigation, a competing dialog,
an edit, an incompatible operation, closing or a lost window discards the
intent. A properties request may retain its own exact inspector ID while
its read-only inspection is pending or settled; another inspector, an Apply,
dismissal or an unrelated surface cannot borrow that exception.

Mapping, native focus operations and `native::present` run outside a
`Model` borrow because FLTK may dispatch callbacks synchronously. Pending
presentation is checked after event dispatch and before listing/watch
pumping, so the initial watch attachment cannot invalidate an otherwise
current first-frame request. User interaction still invalidates ownership.
Queued notice and tab gestures are applied before that check. The manager
also reads the close callback's shared latch before mapping and after focus,
so a callback can revoke activation before `Model.closing` is processed.
Closing the window drops pending context.

## Native outcomes and policy

| Recorded status | Meaning |
|---|---|
| `requested` | The native request was dispatched; focus acceptance is unknown. |
| `granted` | The toolkit confirmed native foreground/key-window state or a synchronous success at that call. |
| `refused` | The native API explicitly refused activation. |
| `unsupported` | The native activation mechanism is unavailable; the window may still be visible. |
| `error` | The call failed; only an error category is retained. |

`requested` never becomes `granted` merely because a window was mapped or
the call returned successfully. Wayland xdg-activation has no grant/refusal
reply, so dispatch remains `requested`, including for an invalid token.
The compositor decides whether to activate the surface. Native outcomes
describe activation separately from visibility; an error after mapping does
not imply that mapping or admitted navigation was undone.

Files does not force foreground ownership, use compositor IPC, target other
windows, retry rejected activation or fall back to unconditional raising.
The existing canvas keyboard target does not establish native user intent.
Toolkit platform behavior and its own upstream evidence do not establish
Files acceptance on another platform.

## Synchronous limitation and remaining scope

At this checkpoint, Wayland discovery/token acquisition runs synchronously
on the UI thread. It uses a private protocol queue, avoiding application
callback dispatch during the handshake, but the first no-token call can
wait for two separate two-second handshakes; later calls can need one.
The two-second first-paint gate does not bound this native call. The D-Bus
admission reply has already been queued before that wait. This bounded
stall remains a usability limitation and is not evidence that startup or
resource gates have passed.

A nonblocking variant has been requested in the toolkit thread. That
follow-up is not part of this committed export; uncommitted toolkit work
must not be adopted as though it had completed review or acceptance.

## Validated delivery — 2026-11-06

The workspace passed 1,728 tests (730 core, 113 helper, 885 UI; 3 ignored),
Clippy and native release builds. Final ownership/close scheduling changes
also passed all 885 UI tests and Clippy. Both machines passed eight source
scopes: native presentation, FileManager1/CLI, keyed tabs, rendered tab
motion, operation notices, keyboard selection, preview copying and real
reviewed Sidebar transfers. Presentation, keyed tabs and rendered motion
passed again through the installed paths.

The presentation fixture uses a private bus and compositor. Its own Wayland
client receives a real keyboard serial and obtains a token through an
anonymous pipe. The driver passes it to the real FileManager1 method, checks
the `requested` result and independently observes the compositor's focused
PID. Only the private compositor enables and reads back
[`focus_on_activate`](https://wiki.hypr.land/Configuring/Basics/Variables/).
Invalid and reused tokens retained origin focus in these runs. The real
close callback at the first-map boundary produced one admitted intent,
zero native attempts, one skipped intent and no replay. A queued admission
reply can lose delivery in that immediate-close scenario.

Pure ownership tests cover stale navigation, edits, modal competition and
the exact properties inspector. Native FileManager1 fixtures cover protected
modal rejection. The implementation does not import or mutate activation
environment variables; this is a source contract, not a claim that every
launcher environment or callback interleaving was exercised.

All five installed artifacts and their backups were hashed on both machines;
native HTML helpers were reused after recipe verification. NODE's Files
configuration remained absent, and M2 configuration/session/layout hashes
remained unchanged. Reports and retained intermediate failures are in the
[evidence bundle](evidence/native-presentation-20261106/README.md).

Files-specific activation acceptance here is Wayland/Hyprland. Upstream's
X11 and synthetic-serial probes are separate evidence, not Files acceptance.
macOS and Windows have no Files runtime acceptance for this integration.
ARM Linux on M2 is a separate platform from macOS.

Automatic desktop activation and default-provider registration remain
outside this change. The service stays opt-in through
`--file-manager-service`, and the source-only activation template is not
installed. The existing lack of a hard total deadline for initial local
libdbus connection/authentication is unchanged; see
[FILEMANAGER1.md](FILEMANAGER1.md). Full Nautilus/Finder parity, language,
accessibility and the `NOT_MEASURED` resource/startup gates remain open.
