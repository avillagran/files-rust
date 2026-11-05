# FLTK 1.5.23 own-window DnD hook

This is a narrow modification of the FLTK Wayland clipboard/DnD translation
unit shipped in the checksummed fltk-sys1.5.23 crate. `SOURCE.json` identifies
archive/original/modified/patch/license hashes. `upstream/` and the zero-fuzz
patch reconstruct the modified file. All FLTK changes retain its LGPL with the
static-link exception (`COPYING`); this file is not relicensed as application MIT.

The C ABI `fl_files_set_dnd_delegate_v1(predicate)` opts in only windows accepted
by a lifetime-checked application predicate. At enter it captures delegation
before upstream set_actions. The delegated sequence skips upstream motion,
roundtrip, drop/read/finish, while still releasing its FLTK-owned offer on
leave/drop/unregister. Unregister preserves sequence suppression until a NEW
enter, including late motion/drop. Clipboard/default windows follow upstream.
The app-owned data_device has different proxies and retains its own lifetimes;
v1 never replaces those listeners or gives the app a FLTK-owned offer (v2 below
hands over exactly one drag offer per delegated enter; still no listener swap).

v2 (`fl_files_set_dnd_forward_v2(callbacks)`) exists because the app-owned
device alone does not work everywhere: Hyprland (0.56, `dataDeviceForClient`)
delivers drag enter/motion/drop to only ONE wl_data_device per client, the
first one created, which is FLTK's. Verified with WAYLAND_DEBUG in a nested
Hyprland: every drag event reached FLTK's device and none the app's, so a
delegated window swallowed every external drop. With v2 registered, the
delegated sequence is forwarded instead of swallowed: offer metadata
(data_offer, MIME types, source actions, action) is reported for every offer
of FLTK's device by pointer only (no ownership), and at a delegated enter the
drag offer's OWNERSHIP passes to the application, which accepts, negotiates,
reads, finishes and destroys it exactly once. FLTK keeps its NULL-data
listener on that proxy and never references it again; motion/leave/drop of the
sequence are forwarded. Without v2 registered, v1 behaviour is unchanged. On
compositors that deliver the same enter (same serial) to every device
(wlroots), the bridge lets the first device that reports a serial own the
sequence and keeps the other one inert. Selection/clipboard events are NOT
forwarded; FLTK keeps those offers.

`native/fltk/inject-v1.cmake` defers until target fltk exists and replaces exactly
ONE source, checking the original SHA. It copies into the build directory,
sets its original header include in TARGET_DIRECTORY fltk, and preserves other
source entries (including generated protocols). Registry files are READ ONLY.
The workspace CMAKE wrapper forwards other configure/build/version argv intact;
only pinned cfltk1.5.23 gets CMAKE_PROJECT_INCLUDE. It does not change compilers,
global installs or other projects. The companion toolchain covers clean trees;
a newly supplied toolchain alone was experimentally ignored by an existing
CMake tree, hence the configure wrapper.

The build-input contract covers original/modified source, patch, license,
SOURCE.json, injection/guard/toolchain/wrapper. CMake appends a strong native
symbol named by that full digest to the generated source. ui/build.rs watches
repo inputs, checks generated contents and references that exact strong symbol
from the bridge. An old OTHER build directory cannot silently validate the
library eventually linked: final app linkage must resolve the precise symbol.
A changed input without fltk-sys rerun fails closed. Bump BOTH versioned wrapper
and CFLTK_TOOLCHAIN paths in .cargo/config.toml when updating inputs (and update
the paths in contract-v1.cmake/build.rs); do not bypass the guard or edit registry.
Keep the retired toolchain file (toolchain-v2.cmake): existing CMake trees
include its exact path from CMakeSystem.cmake and fail to reconfigure without it.
The runtime weak API by itself is not content/version proof.

Specific lifecycle regression: tests/delegate.cxx includes the actual modified
TU and wraps only libwayland version/marshal for opaque private pointers. It
checks single destruction and late-event suppression plus default negotiation.
It uses NO display/window/clipboard/input/protocol handshake. The compiled static
object guard and this fixture do NOT prove native drag/drop, app runtime or
interoperability. scripts/sidebar-drop-nested-check.py exercises genuine Wayland
events between two Files windows in a nested Hyprland (v2 path); other
compositors and other clients remain unvalidated, so O06 stays open for them.
