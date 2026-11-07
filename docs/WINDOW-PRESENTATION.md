# External-request window presentation

The current Files source adopts the exact committed toolkit `fac0705`
export and its `native::begin_present` / `PresentRequest::poll` flow. The
export contains 467 upstream files plus the Files-owned `UPSTREAM.json`,
including all 26 tracked upstream receipts. It retains notices and keyed
tabs; optional GPU, accessibility, images, playback, QR and Mermaid features
remain disabled. The committed `overlay::Sheet` API from `8be476b` is also
included, but Files has not adopted it.

The export identity is recorded in
[UPSTREAM.json](../vendor/lib-fltk-omarchy/UPSTREAM.json), with exact bytes,
Git blobs and modes in the
[export manifest](evidence/async-presentation-20261106/vendor-manifest.json).
The native contract is in the toolkit's
[PRESENTATION.md](../vendor/lib-fltk-omarchy/docs/PRESENTATION.md).

Source `8b69160` passed private delayed-proxy and ordinary native validation
and is installed on NODE and native ARM Linux M2. The historical
`9904f88`/`99db7b1` delivery, recorded in `024bb87`, remains validated and
installed within its original synchronous scope. Earlier FileManager1 and
notice results retain their original scope as well.

## Admission before activation

The FileManager1 broker runs bounded, read-only helper preflight and checks
the current pane, tab, generation and edit/modal/operation guards before
admitting navigation or properties. A rejected, cancelled, expired or stale
request does not produce a presentation intent.

Successful admission creates one owned intent containing the exact incoming
startup ID and the resulting view ownership. The broker queues the D-Bus
admission reply before beginning native presentation. It does not promise
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
owner and generation. `window_presentation::Manager` owns one pending slot
containing that intent and, after native work starts, its UI-thread-only
`PresentRequest`. A newer admission drops the older intent and handle,
cancelling its pending protocol work. There is no activation history, FIFO
or replay after rejection.

For an already mapped window, the manager can begin and poll presentation
in the admission iteration. For a hidden initial window, it first validates
the intent, maps the owned window and requests redraw, then waits for the
paint revision to advance on an event-loop turn. Before `begin_present`,
the slot expires two seconds after scheduling when next checked. Once a
native handle exists, that first-paint expiry no longer applies: the toolkit
owns separate discovery/token deadlines. The slot is not a general
deferred-activation queue.

The manager checks ownership and current interaction guards before mapping,
before `begin_present`, after beginning and before every pending poll.
Changed generation, navigation, a competing dialog, an edit, an incompatible
operation, replacement, closing or a lost window drops the intent and any
native handle. A properties request may retain its own exact inspector ID
while its read-only inspection is pending or settled; another inspector, an
Apply, dismissal or an unrelated surface cannot borrow that exception.

Canvas keyboard focus is restored once when native work starts. It is not
retaken on each `Pending` turn. The manager rechecks the window, canvas,
shared close latch and model ownership after that focus operation and after
beginning native work. A close callback can therefore revoke presentation
before `Model.closing` is processed. Closing drops the pending handle;
retired work cannot later activate a replacement context.

Mapping, focus, `begin_present` and polling run outside a `Model` borrow
because native calls may dispatch callbacks synchronously. Each UI turn
drains queued notice and tab gestures before pumping presentation, then
advances listing/watch work. Strict generation ownership remains in force
throughout the native handshake. The manager retains the intent and handle
across `Pending`, returns to the normal UI loop, and consumes them on a
terminal result. Completed toolkit polls cache their result without replay.

Fixture diagnostics retain `pending_stage` (`paint` or `native`), request and
begin-attempt counts, `native_polls`, `dispatches` for `Requested` outcomes,
`cancelled_native`, superseded/skipped counts and error categories. These
counters distinguish a queued intent, real pending native progress and
terminal dispatch. They do not prove reply delivery or granted focus and do
not expose startup-ID contents.

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

## Asynchronous phases and remaining scope

On Wayland, `begin_present` prepares private discovery work without sending
activation. Each poll performs a protocol pass with a zero-timeout socket
poll, dispatching only its private callbacks and pairing prepared reads with
a read or cancellation before returning. Discovery and token acquisition
each have a separate two-second elapsed deadline checked on subsequent
polls. There is no internal worker, timer or compositor wait in these
phases; the application must continue ordinary UI turns while pending.
Neither phase deadline bounds normal mapping or the X11 backend.

The toolkit captures native input permission before its own `window.show()`,
retains owned redacted hints and never borrows a later gesture's serial.
It revalidates the display/socket and live seat, tracks origin deletion and
resolves current origin/target surfaces before use. Cancellation/drop retire
private protocol callbacks. They prevent a pending activation, but cannot
undo mapping or recall a terminal `Requested` dispatch.

X11 still performs its existing synchronous native I/O in the first poll.
Other platforms defer their existing native call until that poll; the handle
does not make every OS API asynchronous. Windows/macOS have toolkit
typechecks but no Files runtime acceptance. ARM Linux on M2 is not macOS.

The committed upstream CPU/default-feature receipts report 284 unit checks,
6 integration checks, 189 UI checks, 51 doctests and four directed UI checks
before the TLS-exit fix. They separately retain the TLS exit regression
(134 before, 0 after), post-fix X11 wire/Wayland gesture probes, Clippy and
cross-target checks. See the
[upstream summary](../vendor/lib-fltk-omarchy/docs/evidence/async-presentation-20261106/summary.json).
These are toolkit records with a separate scope from the Files acceptance
below. Resource/startup gates remain `NOT_MEASURED`.

## Asynchronous delivery — 2026-11-06

Source `8b69160` passed 1,729 workspace tests (730 core, 113 helper, 886 UI;
three ignored), formatting, Clippy and native NODE/ARM release builds. Each
machine passed nine source scopes and four installed checks. These include
the previous usability regressions, ordinary token/focus validation and the
new delayed-token integration driver.

While a private proxy retained `xdg_activation_token_v1.done`, the same
request remained pending as UI polls, real keyboard input and canvas draws
continued. The Sidebar rendered intermediate widths 123 and 149 between
52 and 170 pixels. Modal, navigation, replacement and close cancelled four
pending native requests; releasing their late replies did not dispatch an
obsolete activation. The proxy preserves descriptor and object lifecycle
ordering, including retained `delete_id`; its reproducible socket tests pass.

The proxy changes compositor-visible peer credentials and is used only for
progress/cancellation evidence. Separate tests without it validate real
startup tokens and focus. Only token replies were selectively delayed;
discovery has source review and normal protocol coverage, not equivalent
peer-delay acceptance. The key-event observer sees an unconsumed F12 event;
the initial Down attempt was consumed by the canvas before that observer.

Both installations backed up and verified all five artifacts, retained the
native HTML recipe and passed async, ordinary presentation, keyed-tab and
rendered-motion checks. User configuration hashes remained unchanged.
[Reports, manifests and retained failures](evidence/async-presentation-20261106/README.md)
separate immutable binary source from four later fixture overlays.

## Historical validated delivery — 2026-11-06

The following results belong to source `9904f88`/`99db7b1`, recorded in
`024bb87`, with exact toolkit `767235b` and its synchronous native boundary.
They do not establish acceptance of the current asynchronous adapter.

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
