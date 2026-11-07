# Compact UI acceptance, 2026-11-07

Production source checkpoint: `03c2ad3`. The independently manifested
`harness/` is an overlay and was not part of that production build. The M2
production source manifest is in `summaries/`.

The NODE and M2 summaries record the exact binary and helper hashes, the four
private Wayland profiles per machine, their report hashes, and comparisons of
the settled Preferences control. Each profile uses a fresh private Sway,
session bus, home, configuration, data root, and retained fixture. The
comparator covers that control only. The ON profile observed an intermediate
frame; the OFF profile reached the same final pixels immediately. This is not
a test of the operating system's reduced-motion detection.

The native runs cover the title and app ID, all ten toolbar hover controls,
the Preferences submenu, pin persistence, automatic folder sizing and a
persisted cache read, and a small Space scan. NODE also exercised an invalid
cache and preserved its bytes. The status warning is visible; its underlying
English diagnostic is truncated. The original user-reported Space error was
not reproduced. Desktop shell icon rendering, resource usage and startup time
were not measured. Sidebar drag reorder remains blocked by toolkit input
semantics; this bundle does not claim that functionality.

The post-install profiles are `node/postinstalled/abb4ca550cac/` and
`m2/6c4b3408a19d/`. Installation and desktop identity receipts are in
`receipts/`; the installer receipts identify the retained backups and hashes.
No default association was changed by these checks.

`node/failures-970/` retains two earlier runs: the first used a pointer that
disconnected after movement; the second reached the restart but the driver
waited on the previous log. Both failed fixtures are retained as evidence,
and neither is counted toward the `03c2ad3` acceptance. Raw PPMs and command
signals remain in the original private fixture directories named in each
report. This bundle copies PNGs, reports and logs, and verifies M2 transfer
hashes; it does not delete or replace those original fixtures.

`verification/` records the 20 safe targeted Rust cases, strict all-targets
Clippy, final native builds and installation hash audit. Earlier build/check
failures are retained: NODE initially resolved an unconfigured mise CMake shim;
the build then used the installed executable through per-process environment
variables. Tuple-pattern and shared-marker compilation fixes are retained,
as are the intermediate lint results. No system tool configuration changed.

The M2 settings audit matches all eight retained configuration/association
baselines. NODE Files settings remained absent. Desktop identity was installed
separately with backups and hashes, without default association changes.
Sidebar explicit row-order commands have pure model coverage; native row-menu
reordering was not exercised. No drag-and-drop functionality is claimed.
