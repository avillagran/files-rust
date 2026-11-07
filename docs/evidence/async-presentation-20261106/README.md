# Asynchronous presentation — 2026-11-06

Files source `8b69160`, with fixtures/docs `fd1915d`, adopts exact toolkit
`fac0705` and is validated and
installed on NODE and native ARM Linux M2. The earlier synchronous delivery
remains in [its own evidence](../native-presentation-20261106/README.md).
The full Nautilus/Finder goal and resource/startup gates remain open.

## Result and lifecycle

The manager retains an accepted intent and a UI-owned `PresentRequest`.
It begins native work once, polls on ordinary UI turns outside model
borrows, and drops the handle when navigation/modal/generation ownership,
the close latch or a newer admission invalidates it. Canvas focus is restored
only at native start. First-paint expiry applies before begin; native
discovery/token phases then have separate two-second elapsed deadlines.

Wayland polling never waits for a compositor reply. It owns a private queue,
stable callback storage and a single request lease. Cancellation retires
proxies before releasing callback data; terminal results are cached without
activation replay. Captured native context cannot borrow a later input
event's permission. `Requested` remains dispatch, not focus confirmation.
Neither cancelling native work nor closing the window guarantees delivery
of an already queued D-Bus admission reply.

X11 retains synchronous server I/O in its first poll. Windows/macOS have
toolkit typechecks but no Files runtime acceptance. ARM Linux M2 is not
macOS. No default association or desktop activation provider was installed.

## Application validation

| Scope | NODE | ARM Linux M2 |
|---|---|---|
| Delayed token: input/motion and modal/navigation/replacement/close cancellation | PASS | PASS |
| Ordinary presentation: real/invalid/reused token and focus | PASS | PASS |
| FileManager1/CLI and properties | PASS | PASS |
| Keyed tabs | PASS | PASS |
| Rendered tab motion | PASS | PASS |
| Operation notices and history/report actions | PASS | PASS |
| Keyboard selection | PASS | PASS |
| Preview text copying | PASS | PASS |
| Real reviewed Sidebar transfers | PASS | PASS |
| Installed async, ordinary presentation, keyed tabs and tab motion | PASS | PASS |

These are nine source acceptance scopes and four installed checks per
machine. The workspace passed 1,729 tests: 730 core, 113 helper and 886 UI,
with three ignored core tests. Formatting, Clippy with warnings denied and
native release builds passed. Compilation and validation used private
configuration/data roots; NODE's real Files configuration remained absent.

The proxy retained only `xdg_activation_token_v1.done`, plus its corresponding
`delete_id` to preserve object lifecycle order. During a genuinely pending
request, native polling, UI-loop turns, a real unconsumed F12 event and canvas
draws continued. The actual sampled Sidebar widths advanced from 52 through
123 and 149 to 170 pixels. The intermediate states required new paints while
the token reply remained held. Each run cancelled four native requests;
releasing late replies after modal, navigation, replacement or close did not
increase the dispatch count. Final state had no pending request.

The proxy changes compositor-visible peer credentials. Its own PID and the
service PID are recorded separately, and its focus arrangement is only test
setup for input/progress. A separate driver without the proxy establishes
ordinary activation and real startup-token/focus behavior. Discovery was
reviewed and exercised normally, but was not selectively delayed here.

## Source provenance and installation

The exact vendor contains 467 upstream files plus Files' `UPSTREAM.json`,
including 26 upstream receipts. No WIP, build directories or vendor patches
were copied. The committed lazy `overlay::Sheet` API is included but is not
used by Files. Optional GPU/accessibility/media features remain disabled.

M2 received 1,187 exact committed source files, checked by path set, Git blob,
bytes and mode. Archive SHA-256:
`35cf0c2d482be9bf1539bacc417ebb3f3791bc2a919089c4861c417f02735632`.
The ARM build took 101.35 seconds; nine inherited FLTK cache directories were
preserved before regeneration. No NODE ELF was transferred. Four test
sources were subsequently overlaid separately; their hashes and original
checkpoint hashes are in [fixture-overlays.json](provenance/fixture-overlays.json).
They do not change the compiled application source.

| Machine | Installed app SHA-256 |
|---|---|
| NODE | `d4c563a603f13a1f50bb2ac2592c447698e82821df91d7fb9b49885806b50930` |
| M2 ARM Linux | `4e6b3ee79506f0382701f6c4ef23cd537927da40d75d53395445e065dcd8a063` |

All five installed artifacts and their backups were verified: application,
helper, HTML helper, licenses and build stamp. Replacement was staged and
atomic per artifact. Each native HTML helper retained its verified recipe.
M2 configuration/session/layout hashes matched the initial snapshot; user
windows, settings and associations were not controlled or changed.

## Retained failures and supporting checks

The first proxy run sent Down, which the canvas consumed before the window
observer could count it. UI polling, painting and intermediate Sidebar
widths already advanced, but the input assertion failed. The driver now
uses an unconsumed F12 event; the original FAIL remains unchanged.

The toolkit separately reproduced a pending-request TLS teardown abort
(134) and fixed cleanup to tolerate a previously destroyed cache (exit 0).
Its full CPU suite, final directed UI checks, post-fix Wayland gesture/X11
wire probes and crosschecks retain their original scope in
[the upstream receipt](../../../vendor/lib-fltk-omarchy/docs/evidence/async-presentation-20261106/summary.json).
Those records are separate from Files end-to-end acceptance.

The reproducible `scripts/wayland-activation-proxy-check.py` checks client
and server fragmentation, selective retention, ordered `done`/`delete_id`,
ID retirement/reuse, real `SCM_RIGHTS`, failure on ambiguous descriptor
association and cleanup limited to sockets the proxy owns. Receipts and
source hashes are in `provenance/`. No token payload is persisted.

## Evidence map and remaining scope

`node-collection.json` indexes 77 source/installed regression and build/test
files. `node-presentation/manifest.json` indexes 36 presentation/fixture
files. `m2-source/` and `m2-installed/` retain original relative paths with
complete SHA/byte manifests: 67 and 32 transferred files respectively.
`node-install/` and M2 audits check all five backups and installed hashes.
The top-level vendor manifest/review and `provenance/` record source and
fixture identities. Ignored raw logs are intentionally included unchanged.

This delivery does not establish all delayed discovery/reconnect/origin
lifetime paths, Files X11 nonblocking behavior, native macOS/Windows
acceptance, global resource/startup gates, complete i18n/accessibility or
full Nautilus/Finder parity. Initial libdbus connection/authentication still
has no hard total deadline. Sheet adoption needs controlled dismissal while
operations await acknowledgement; that requirement was sent to the toolkit
thread before migrating dialogs.
