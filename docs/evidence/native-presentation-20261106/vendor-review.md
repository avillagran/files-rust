# Native presentation toolkit export

Files exports the committed toolkit tree
`767235b79d9f6fd5ef9b20495585d5d2e21cc141`, based on the preceding exact
`0f680890cba7038fa0519be19d159eed4f433a25` export. All 430 baseline files
matched their committed bytes and modes before modification. The new tree
contains 443 upstream files: 18 changed paths, 13 additions and no removals.
Files owns the additional `UPSTREAM.json` provenance record.

The export includes the committed xdg-activation license and all 13 existing
tracked receipts under upstream `docs/evidence/`. These historical receipts
are part of source provenance, not proof of the new Files integration.
Twenty historical executable modes remain intact; the two new executable
probes, `script/present-check` and `script/present-wayland-check`, retain Git
mode `100755`. The other 421 files use `100644`.

`vendor-manifest.json` records the source tree/archive hashes and each file's
Git blob ID, SHA-256, byte count and mode. All 443 exported files match the
committed blobs. No upstream working-tree files were copied or edited. The
manifest records the excluded dirty paths at export time. Cargo manifests,
lockfiles and feature activation remain unchanged; optional GPU,
accessibility, images, playback, QR and Mermaid remain disabled in Files.

## Public API

`fltk_omarchy::native` exports:

- `present<W: WindowExt>(&mut W, PresentOptions<'_>) -> Result<Presentation, PresentError>`.
- `PresentOptions`: explicit `activation_token`, `startup_id` and X11
  `timestamp` hints; no environment lookup or mutation.
- `Presentation`: the actual `PresentBackend` and `PresentStatus`.
- `PresentStatus`: `Unsupported`, `Requested`, `Granted` and `Refused`.
- `PresentError`: wrong UI thread, deleted/child window, invalid hint,
  reentrancy (`Busy`), timeout or native failure.

The adapter must call on the FLTK UI thread with an app-owned top-level
window, outside model borrows. Hints reject NUL or more than 60 KiB before
mapping; empty strings mean absent. Existing stricter Files input limits
need not be relaxed. Debug output redacts hint contents. No worker thread,
external command or compositor IPC is created by the toolkit API.

## Outcome and timing limits

A hidden own window is mapped normally; an already shown window is not
hidden/remapped. The result describes activation, not visibility. An error
after mapping need not hide the window again. `Requested` means the request
was dispatched, never that focus was granted. Wayland xdg-activation has no
granted/refused acknowledgement and can silently ignore invalid tokens.

On Wayland, `activation_token` takes precedence over `startup_id`; both are
opaque tokens. Token discovery/acquisition is synchronous but bounded by
private protocol-queue handshakes: at most two 2-second handshakes on the
first no-token call, one thereafter. The queue avoids application callback
reentrancy but the UI thread can still wait during the call. Synthetic
focus helpers cannot provide the native event permission used by the token
path; the compositor remains the authority.

X11 uses EWMH/startup notification without an `XSetInputFocus` or unconditional
raise fallback. Windows uses native foreground restrictions; macOS uses its
own application/window activation API. Windows/macOS ignore startup hints.
Unsupported platforms may still map a hidden window while returning
`Unsupported`. Minimized/newly mapped windows remain platform-policy cases.

This worker verified exact provenance and public source/API consistency.
It did not execute Cargo compilation or GUI tests. Upstream typechecks and
its private Linux probes do not establish Files integration, native M2,
Windows/macOS runtime, every timeout/reconnect path or universal compositor
acceptance. Files must record its own token forwarding, ownership,
rejection/focus, modal/close and per-machine acceptance separately.
