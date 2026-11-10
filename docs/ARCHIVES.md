# Archives beyond ZIP32: tar.*, 7z, RAR, ZIP64 and single compressed files

2026-11-10, source only (not installed). Extends the native ZIP32 path
documented in `ARCHIVE-OPERATIONS-CONTRACT.md` and `ZIP-PREVIEW.md`; that path
is unchanged and still handles every ZIP32 archive it accepts.

## Formats

| Format | List (preview) | Extract | Create |
|---|---|---|---|
| `.zip` (ZIP32) | native parser | native (unchanged) | native ZIP32 (unchanged) |
| `.zip` (ZIP64, >4 GiB, over the native caps) | bsdtar fallback | bsdtar fallback | — (native ZIP32 limits remain) |
| `.tar`, `.tar.gz`/`.tgz`, `.tar.bz2`/`.tbz2`, `.tar.xz`/`.txz`, `.tar.zst`/`.tzst` | bsdtar | bsdtar | `.tar.xz`, `.tar.gz`, `.tar.zst` (pax) |
| `.7z` | bsdtar | bsdtar | bsdtar (`--format 7zip`) |
| `.rar` (RAR4/RAR5) | bsdtar | bsdtar, read-only | never |
| `.gz`, `.bz2`, `.xz`, `.zst` (one file) | synthetic one-row listing | `gzip`/`bzip2`/`xz`/`zstd -d -c` | — |

The format comes from the basename suffix and must agree with the magic bytes
(`archive_format::Format::magic_matches`; plain `.tar` needs `ustar` at offset
257). For containers, the reader that libarchive reports in `bsdtar -tvv`
("Archive Format: …") must belong to the declared family
(`Format::reader_matches`). This keeps mtree, cpio, ISO and other libarchive
readers out of the pipeline even when a valid gzip/xz/zstd stream wraps them;
an mtree spec can reference host files through `contents=`, so a `.tar.gz`
containing one is refused before anything is written.

## Dependencies and processes

No crate was added. `bsdtar` (libarchive, BSD-2-Clause) is part of the
Arch/Omarchy base system and runs as an ephemeral child process, like the
existing ffmpeg/poppler tools; nothing is linked into the UI process. Tools are
looked up only in `/usr/bin` and `/bin`, never `$PATH`. A missing tool is
reported as `formato no disponible: falta bsdtar` (or `falta xz`, …).

Child contract (`crates/core/src/archive_bsdtar.rs`, `supervise`): fixed argv,
no shell, cleared environment with `LC_ALL=C.UTF-8` and `PATH=/usr/bin:/bin`,
`setsid()` (no controlling terminal, so a passphrase prompt cannot block on a
TTY), `PR_SET_PDEATHSIG=SIGKILL`, stdin closed or fed from a pinned descriptor,
stdout to `/dev/null`, our own file or a bounded line reader (64 KiB lines,
64-line channel), stderr capped at 2 KiB head + 2 KiB tail. Cancellation, an
error from a callback, or dropping the guard kills the whole process group and
reaps it. A paused job sends `SIGSTOP`/`SIGCONT` to the group.

`LC_ALL=C.UTF-8` instead of plain `C`: under `C` libarchive cannot convert
7z/ZIP/pax names to the locale charset and silently skips every non-ASCII
member ("Pathname cannot be converted…"). `C.UTF-8` keeps C messages and date
formats with a UTF-8 charset; non-UTF-8 bytes are still printed as `\ooo`
escapes and unescaped back to the exact raw bytes.

## Extraction safety (hostile input)

Review (`archive_bsdtar_extract::Prepared::capture`, AuxIo worker, read-only):

- opens the source with `O_NOFOLLOW`, checks magic, pins dev/inode/size/mtime;
- lists with `bsdtar -tvv --numeric-owner -f -` (source fed on stdin) and
  parses each line (`parse_row`); unparseable or over-long lines reject;
- rejects any member that is absolute, contains `..`, has a component over
  255 B, a path over 4096 B or more than 128 levels, is a hardlink, a device,
  FIFO or socket, or is a symlink whose target is absolute or lexically leaves
  the archive root;
- rejects encrypted archives (`(aes)`/`encrypt` in the reported reader, or a
  passphrase/encryption diagnostic) with `archivo cifrado: no admitido todavía`;
- budgets: at most 2,000,000 entries; declared bytes plus a 256 MiB reserve
  must fit in the destination filesystem's free space.

Execution (`Prepared::execute`, the existing global operation worker):

1. Re-checks the destination parent identity and the source identity; a
   changed archive must be reviewed again.
2. Creates a fresh 0700 `.files-extract-<16 hex>` directory next to the
   destination (same filesystem).
3. Runs `bsdtar -x -f - --no-same-owner --no-same-permissions --no-acls
   --no-xattrs --no-fflags -C <stage>` with the pinned source on stdin and
   the stage as working directory. `-P` is never used, so libarchive's own
   `..`/absolute/symlink-traversal guards stay on. Single compressed files use
   `<tool> -d -c` with stdout redirected into an exclusive file in the stage.
4. Every 250 ms: free space must stay above the 256 MiB reserve, used bytes
   within the reviewed budget, and created inodes within 2,000,000; otherwise
   the child group is killed. Any bsdtar diagnostic, even with exit status 0,
   fails the run.
5. Verifies the staged tree on disk (`verify_tree`) without following symlinks:
   only regular files with a single link, directories and symlinks; every
   symlink is resolved component by component with lstat/readlink
   (`confined`, at most 40 hops) and must stay inside what will be published
   (the single top-level folder, or the whole stage); budgets for entries,
   bytes, path length and depth are enforced again. setuid/setgid/sticky bits
   are cleared; directories get owner `rwx` so the tree stays traversable.
6. `syncfs` on the stage, re-checks the parent, then publishes like Nautilus:
   a single top-level folder or file keeps its own name; several top-level
   entries are published as a folder named after the archive (the dialog's
   name). Publication is `renameat2(RENAME_NOREPLACE)` with the existing
   numbered names (`name`, `name (2)`, … via `copy_unique_name::Names`); it
   never overwrites and never merges. The emptied stage is removed with
   `rmdir` only; the parent is fsynced.

Any failure or cancellation after the stage exists kills the child and renames
the stage to `.files-extract-failed-<hex>`, kept as evidence and reported in
the result panel; nothing is deleted recursively. Nothing is published on
failure. The native ZIP32 recovery buttons do not apply to these jobs.

Not covered: this is not a snapshot. A same-UID process racing the stage
between verification and rename, or rewriting the archive in place without
changing its size or mtime, is outside the guarantee (same scope as the native
path). ACLs, xattrs, owners and timestamps beyond what bsdtar sets as an
unprivileged user are not restored.

## Creation

`archive_bsdtar_create::Prepared::capture` (review): the destination suffix
selects the format; review walks the selection with lstat (symlinks are never
followed), rejects devices/FIFOs/sockets, hard-linked files for tar formats
(they would become hardlink members that extraction refuses; 7z stores them
as independent files), an output folder inside a selected folder, more than
2,000,000 entries, and source bytes plus the 256 MiB reserve above free space.

Execution runs `bsdtar -c -f /proc/self/fd/1 --no-acls --no-xattrs --no-fflags
--format pax --xz|--gzip|--zstd` (or `--format 7zip`) `-C <source> --null -T -`
with the selected names NUL-separated on stdin. `--null` also disables the
`-C` directive inside the list, so names starting with `-` or `@` or
containing newlines are literal. Stdout is an exclusive 0600
`.files-create-<hex>.partial` in the destination; `/proc/self/fd/1` (instead
of `-f -`) makes libarchive write an unpadded regular file, because stdout
output is padded to 10 KiB and corrupts zstd streams. The file is synced and
published with `RENAME_NOREPLACE` under a numbered name if taken. On failure
or cancellation only that partial file, identified by dev/inode, is
unlinked. Sources are never modified. Non-UTF-8 source names make bsdtar warn
and therefore fail the job (conservative).

## UI

- **Extraer aquí** (context menu *Comprimir*, Actions toolkit menu, command
  palette `extract-here`): the same review with the archive's own folder as
  destination; a successful review is enqueued without a second click.
- **Extraer en…** (`ExtractArchive`): the existing review/Apply dialog with an
  editable destination. ZIP32 keeps the exclusive `<stem>-extraido` folder;
  other formats default to the archive stem.
- **Comprimir…** (`CreateArchive`): the destination's extension picks the
  format; the *Formato* button or **F4** cycles `.zip → .tar.xz → .tar.gz →
  .tar.zst → .7z`, which invalidates a previous review.
- Space preview lists tar.*, 7z, RAR, ZIP64 and single compressed files through
  the helper (`preview_archive`, 30 s supervisor, helper address-space limit
  768 MiB for this command only, inherited by bsdtar), marking unsafe members.
- Progress, pause and cancel use the existing queue bar; jobs are charged
  1 MiB plus their retained metadata to the existing controller budget and
  10 descriptors.

## Evidence

Tests build their fixtures with bsdtar inside a fresh private 0700 directory
under `TMPDIR`, never delete trees and skip with a message when bsdtar is
missing: `crates/core/src/archive_bsdtar_tests.rs` (round trip for each
writable format with a symlink, a UTF-8 name, a newline name and a leading
`-`; several top-level entries; bsdtar ZIP and single gz/xz/zst/bz2; symlinks
stored as links and never followed; absolute, `..`, escaping symlink plus a
file written through it, absolute symlink, hardlink and FIFO archives rejected
with nothing published; mtree inside gzip and wrong magic refused; AES and
ZipCrypto archives reported as encrypted; numbered names instead of
overwriting for extraction and creation; cancellation publishes nothing and a
running child is killed; entry and byte budgets; symlink chains that escape
through `a -> .`) and `crates/ui/src/archive_ui/external_tests.rs`
(*Extraer aquí* with an occupied name, compress dialog F4 → `.tar.xz`).

Unverified: real ZIP64 archives over 4 GiB, real RAR/RAR5 archives (no RAR
writer is available), encrypted 7z/RAR, very large archives against the
budgets, power loss, and the on-screen look of the new dialog button and
menu entries.
