# Native request presentation — 2026-11-06

Application checkpoints `9904f88` and `99db7b1`, with acceptance fixtures and
documentation in `7652b9b`, adopt exact toolkit `767235b`. Both NODE and
native ARM Linux M2 passed scoped source and installed acceptance. This
delivery does not establish complete Nautilus/Finder parity or pass the
resource/startup gates, which remain `NOT_MEASURED`.

## Behavior and ownership

Accepted FileManager1 requests preserve the exact opaque StartupId and queue
their admission reply before native work. A reply acknowledges UI admission,
not foreground ownership. The manager retains one intent, checks its
post-admission pane/tab/generation and dialog ownership, and consumes it
without activation replay. A hidden window maps normally and waits for its
first paint, with a two-second pending deadline. That deadline does not bound
the toolkit's synchronous native handshake.

Queued user gestures apply before delayed presentation, while the initial
listing/watch refresh follows it. Mapping, canvas focus and native calls run
outside model borrows. The native close callback shares a latch with the
manager, so a close observed during mapping or focus revokes activation
before normal model-close processing. The retained first-map close fixture
reports `requests=1`, `attempts=0`, `skipped=1`, `pending=false` and clean
draining. Immediate close can disconnect the caller before a queued reply
is delivered; that outcome is recorded rather than called an acknowledged
success.

See [WINDOW-PRESENTATION.md](../../WINDOW-PRESENTATION.md) for the contract.

## Validation

| Scope | NODE | Native ARM Linux M2 |
|---|---|---|
| Presentation: initial map, invalid/fresh/reused token, first-map close | PASS | PASS |
| FileManager1/CLI and properties | PASS | PASS |
| Keyed tabs and native focus | PASS | PASS |
| Rendered tab motion | PASS | PASS |
| Operation notices and real history/report actions | PASS | PASS |
| Keyboard selection | PASS | PASS |
| Preview text copying | PASS | PASS |
| Real reviewed Sidebar transfer | PASS | PASS |
| Installed presentation, keyed tabs and rendered motion | PASS | PASS |

The workspace passed 1,728 tests: 730 core, 113 helper and 885 UI, with three
ignored core tests. Final close and gesture scheduling refinements passed
the 885 UI tests again. Formatting, Clippy with warnings denied and native
release builds passed. The private policy parser also passed seven bounded
type/value cases without GUI execution.

The presentation driver uses a private D-Bus session and its own compositor.
A small owned Wayland client receives a real keyboard event and obtains a
token using its seat, serial and surface. An anonymous pipe transfers that
token to the driver, which passes it through the real FileManager1 call.
Tokens are absent from reports, logs and persisted command files.

Native Wayland results remain `requested`. Focus is checked independently
through the private compositor. Its activation policy is explicitly enabled
and read back, using the supported legacy or Lua configuration API. A fresh
token activated Files; invalid and reused tokens retained origin focus in
these runs. No test focuses the target through IPC. Private origin focus
arrangement is separate from the target's activation request.

M2's export contains 1,160 exact committed files, including all 443 toolkit
files, Files' provenance record and 13 upstream receipts. Bytes, modes and
Git blob IDs were checked. The archive SHA-256 is
`9209e98fafb26f302dde1f1d804e270de70baf73edf8d8482f3f9b021cc75207`.
The build ran natively on ARM. Ten inherited FLTK cache directories were
preserved before regeneration; no NODE executable was transferred.

## Installation

| Machine | Installed application SHA-256 |
|---|---|
| NODE x86_64 | `865330c61d7c7b0e2ae2bd4725e7a36a70d5573740286901ff2db63031b2a6a1` |
| M2 ARM Linux | `a22e71378cfc6924e7049dcb262acc95b535c518c746a7d6b8421930770c29e6` |

Each installation backed up and hashed all five artifacts before staged,
individual atomic replacement: application, helper, HTML helper, its licenses
and build stamp. Installed and backup hashes were checked against the
receipts. Existing native HTML helpers retained their verified recipe.
NODE's Files configuration remained absent; M2 configuration, session and
layout hashes matched their initial snapshot. User windows and default
associations were not controlled or changed.

## Retained intermediate evidence

- The first Rust suite used `/var/tmp`; the existing Trash secure-ancestor
  check rejected that writable ancestor. The suite passed under a private
  `/run/user/1000` root without weakening the policy.
- Headless compositor initialization and an overlong IPC socket path were
  rejected. The fixture uses a short private runtime and a hidden owned
  nested compositor with an explicit private bus.
- Hyprland's launcher did not supply an environment token. The final fixture
  obtains one through a native Wayland keyboard event instead.
- Earlier focus-policy and ARM SSH environment assumptions failed. The
  final driver sets and reads the private activation policy, accepts both
  `int: 1` and `bool: true` representations, and preserves the original
  reports. No production activation fallback was added.
- The initial close fixture required reply delivery after immediate close.
  Its final contract instead checks actual admission, zero native attempts,
  no replay and clean draining, while retaining the caller's real result.

## Evidence map and limits

`node-collection.json` indexes source regressions and build/test logs;
`node-presentation/manifest.json` indexes native presentation attempts;
`node-installed-collection.json` indexes installed tab/motion checks.
`node-install/` contains the receipt and five-artifact audit. `m2-source/`
and `m2-installed/` retain original M2 paths and complete SHA/byte manifests;
all 53 source and 46 additional transferred files were verified.
`provenance/` records the source export, exact vendor and native ARM build.
The top-level vendor review and manifest document upstream provenance.

New Files activation acceptance is Wayland/Hyprland. Upstream X11 and
synthetic-input probes retain their separate scope; they do not establish
Files X11 activation or macOS/Windows runtime acceptance. The native API's
synchronous private-queue waits can still stall the UI. A nonblocking API
request is queued in the toolkit thread; it is not adopted here. Initial
libdbus connection/authentication still has no hard total deadline. Automatic
desktop activation/default registration, full language/accessibility and
the broader project goal remain open.
