# Native titlebar format

Files publishes its current absolute path and brief state as the client title.
The desktop entry provides `Name=Files`, `Icon=files` and `StartupWMClass=files`.
This lets the Omarchy Hyprland titlebar plugin draw the icon, a dimmed application
name, its configured separator, and the explanatory title as separate elements.
Files does not add a literal app prefix or pick a separator for the compositor.

The integration is based on titlebars PR #11, commit
`94ee0bddddc6035e6ec5eefe9d0ebbe332e16aa2`. The local plugin update additionally
preserves the app prefix for absolute path titles containing the app name. The
normal duplicate-name heuristic still applies to descriptive browser titles.
This matters for paths such as `/home/example/Work/files` and for private fixtures
whose parent directory contains `files`.

The intended active defaults are `bar_icon=true`, `bar_title_source=title`, and
`bar_title_separator=›`. Compositor titlebars only apply where the existing
workspace and client-decoration policies enable them. These settings are owned
by the desktop; Files does not write Hyprland configuration.

Production Files source is `3e1f64ab11da392313787c56ca5459883145876a`.
NODE and ARM binaries are built natively. Each plugin candidate preserves its
machine's existing features: NODE retains its glass, stencil and application
button changes; M2 gains the current PR #11 title format without unrelated
feature additions. Source manifests retain the exact local snapshots.

Private native visual acceptance passes on both architectures. M2 is installed
and its post-install visual check passes. The NODE application has since been updated with the navigation/size release;
its separate titlebar plugin replacement still waits for system Polkit
authentication. See the
[evidence bundle](evidence/native-titlebar-20261107-3e1f64a/README.md) for precise
checks, retained setup failures, receipts and source provenance.
Existing Files processes keep their previous executable and title behavior until
the user reopens them. Replacing the installed binary does not restart a window.
