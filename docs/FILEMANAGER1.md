# FileManager1 and external navigation

Files implements the three methods in the freedesktop.org
[File Manager D-Bus Interface specification](https://wiki.freedesktop.org/www/Specifications/file-manager-interface/).
The Linux service is explicit: start it with `--file-manager-service`.
Ordinary Files windows and the location CLI do not claim the shared bus name.
This delivery does not install activation files, register a default folder
handler or change MIME associations.

## Command line

Use one location mode per invocation. Operands may be local paths or local
`file://` URIs; relative paths resolve against the invoking process's current
directory. `--` ends option parsing, including for names beginning with `-`.

```sh
files --select -- /absolute/report.txt /absolute/another.txt
files --select -- 'file:///absolute/report%20draft.txt'
files --show-folders -- /absolute/FolderA /absolute/FolderB
files --properties -- /absolute/report.txt /absolute/another.txt
```

`--select` opens parent folders and selects the requested items.
`--show-folders` opens the contents of the requested folders.
`--properties` opens the existing properties dialog, with a page per requested
object. These commands start their own Files process; they do not forward
operands to an unrelated file manager or require a running Files service.
They use an untouched initial tab and skip session restoration. The initial
window is presented after the request is admitted.

The same paged properties dialog accepts up to 32 selected browser objects.
Each page loads the object's metadata and existing explicit permission actions;
there is no batch permission change implied by a multiple-object request.

## Explicit session-bus service

```sh
files --file-manager-service
```

The service flag cannot be combined with initial locations or the three
location modes. It requires an existing `DBUS_SESSION_BUS_ADDRESS` consisting
of local Unix transports. Files opens a private libdbus connection, requests
exclusive ownership with `DO_NOT_QUEUE`, and refuses startup if another
application already owns the name. It never replaces that owner or waits to
take over later. The initial window remains hidden until a request succeeds.

| D-Bus field | Value |
|---|---|
| Bus | Session bus |
| Destination and interface | `org.freedesktop.FileManager1` |
| Object path | `/org/freedesktop/FileManager1` |
| Methods | `ShowFolders`, `ShowItems`, `ShowItemProperties` |
| Input signature | `ass`: URI string array, then startup ID string |
| Successful return | No arguments |

For a manually started service and an existing example file:

```sh
gdbus call --session \
  --dest org.freedesktop.FileManager1 \
  --object-path /org/freedesktop/FileManager1 \
  --method org.freedesktop.FileManager1.ShowItems \
  "['file:///absolute/report.txt']" ""
```

The object also supports `Introspect`, `Ping` and `GetMachineId` on their
standard interfaces. Unknown objects, methods and signatures return errors.
The transport does not invoke a shell, use TCP/autolaunch fallback or request
interactive authorization.

## Admission and presentation

All supplied locations must pass a read-only filesystem preflight before any
navigation or properties dialog is admitted. The preflight runs in the
existing owned helper process, with bounded input/output, cancellation,
deadline, EOF verification and child reaping. It checks every location in the
batch. An invalid or inaccessible location rejects the batch.

An active dialog, edit or incompatible operation rejects an incoming request
immediately. The broker records pane, tab and generation ownership before
starting the helper and checks it again before applying the result. A view
change, cancelled request or expired bus ticket prevents later activation.
Filesystem preflight is advisory: it does not grant authority for a later
mutation or guarantee that a file cannot change after admission.

Folders and item parents are grouped by exact path. Existing matching tabs
can be reused; the first group becomes active and further groups open in
background tabs. The full group batch must fit the existing tab and snapshot
budgets before navigation is applied. Item selection retains exact Unix name
bytes. A targeted hidden item is temporarily visible in its destination tab;
the global hidden-file preference is unchanged. Item requests switch Columns
or SpaceMap destinations to List when needed for exact selection. If a
requested item disappears while the listing loads, Files clears the target
selection and reports the failure rather than selecting a neighboring row.
Same-location refreshes retain the complete pending name set, including when
a watcher refresh follows a partially restored listing. They cannot silently
turn an external request into a partial selection.

A successful D-Bus reply acknowledges UI admission. Folder enumeration,
painting and properties metadata may finish afterward. A service disconnect
does not terminate an already presented Files window. The broker queues the
reply before native presentation; it does not acknowledge focus transfer.
Immediate window closure can disconnect a caller before its queued reply
is delivered.

## Bounds and lifecycle

| Resource | Limit |
|---|---|
| Locations per request | 1–32, before deduplication |
| Original URI text plus startup ID | 65,536 bytes in total, before deduplication |
| Decoded path | 4,096 bytes per location |
| Startup ID | 4,096 bytes, without control characters |
| Complete incoming D-Bus message | 65,536 + 4,096 bytes, including envelope/alignment |
| File descriptors attached to a request | None |
| Pending D-Bus calls | 8 |
| UI admission deadline | 5 seconds per bus call |
| Filesystem helper deadline | 3 seconds |
| Service worker polling interval | At most 25 ms requested per poll |
| `Hello` and `RequestName` reply timeout | 1 second for each call |

Only local `file://` URIs with an empty authority or `localhost` are accepted.
URI spaces, controls and non-ASCII bytes must be percent encoded. Decoding
happens once; malformed escapes, NULs and remote authorities are rejected.
On Unix, percent-encoded raw filename bytes survive decoding without lossy
Unicode conversion. Oversized or descriptor-bearing messages can cause
libdbus to close the service connection; disconnect does not exit the app.

Cancellation and normal established-connection shutdown keep the UI thread
free of blocking joins. There is **no hard total deadline for initial local
libdbus connection/authentication**. The Unix implementation performs a
blocking `connect()` before making its socket nonblocking; a pathological
local socket or authentication peer can therefore delay worker completion
and shutdown draining. The one-second method reply timeouts and 25 ms polling
interval do not bound that initial phase. See the
[libdbus Unix connection implementation](https://dbus.freedesktop.org/doc/api/html/dbus-sysdeps-unix_8c_source.html#l01019).

## Activation and remaining scope

The current source adopts exact toolkit `767235b` and forwards the accepted
request's startup ID unchanged to `native::present`. The owned intent is
created only after UI admission, and the D-Bus reply is queued before native
activation. An already mapped window can be presented immediately; a hidden
initial window waits for its first paint in one pending slot with a
two-second expiry. Current view/modal/operation guards are checked again
before the native call, allowing only a properties request's own inspector.
Rejected or stale intents are dropped without replay or activation retries.

`Requested` means native dispatch, not confirmed focus. Files records the
toolkit's activation outcome separately from visibility and does not force
foreground ownership. Startup context and native error text are redacted;
activation environment variables are neither imported nor mutated. The
current Wayland private-queue implementation is synchronous and can wait
for two separate two-second handshakes on its first no-token call. This
wait follows admission-reply queuing and remains a usability limitation.
Files-specific native acceptance and installation passed on NODE and native
ARM Linux M2. See [WINDOW-PRESENTATION.md](WINDOW-PRESENTATION.md) for the complete
contract and the requested, unadopted nonblocking toolkit follow-up.

The source-only
[optional service template](../packaging/dbus/org.freedesktop.FileManager1.service.in)
is for a separately reviewed packaging decision. It is not consumed by normal
builds, update/install scripts or the application. A packager must replace
`@FILES_ABSOLUTE_EXECUTABLE@` with an absolute executable path and preserve
correct `Exec` quoting; shell expansion of `~` or environment variables is
not a path substitution mechanism. The launched environment must provide the
local session address required above. Installing a standard-name activation
file changes which application the bus can launch and can conflict with an
existing provider; no such installation is part of this implementation.
See the [D-Bus activation specification](https://dbus.freedesktop.org/doc/dbus-specification.html#message-bus-starting-services).

Automatic desktop activation/default registration and native macOS/Windows
acceptance remain open. Functional evidence for each delivery is recorded
separately; this implementation does not close
the project's `NOT_MEASURED` resource/startup gates or establish complete
Nautilus/Finder parity.


## Validated installation — 2026-11-06

Source checkpoints `0c9a59c` and `cbc83a9`, with the private native driver
introduced in `8107d36`, passed 1,710 workspace tests (730 core, 113 helper,
867 UI; 3 ignored), Clippy, formatting and native release builds. NODE and
native ARM M2 each passed six source acceptance scopes: FileManager1/CLI,
keyed tabs, rendered motion, keyboard selection, preview copying and real
reviewed Sidebar transfers. Installed FileManager1, tabs and motion checks
also passed on both machines.

| Machine | Installed app SHA-256 |
|---|---|
| NODE x86_64 | `95600f0976a77c4aecd007807291fe5cceea2811bec2a0b92b920b93b2f1d17e` |
| M2 native ARM Linux | `75ea16c6bacc7f24e237e475eac1b6baa7b0396a1039e7de6439710aa9bbb5b1` |

All five installed artifacts and their backups were hashed. Each unchanged
native HTML helper was reused after recipe verification. No default service
activation or folder association was registered. Reports, screenshots,
intermediate failures, the watch-race correction and the resolved test
persistence incident are in [the evidence bundle](evidence/filemanager1-20261106/README.md).
