# Thumbnails: kinds, cache tiers and the freedesktop shared cache

Status 2026-11-10: implemented and unit-tested. The on-screen look, interop
with thumbnails written by other applications on a real desktop, and process
tree PSS have not been measured. [MEDIA-THUMBNAILS](MEDIA-THUMBNAILS.md)
still describes the base pipeline: visible demand, revision proof and the
RAM LRU.

## Kinds

| Kind | Extensions | Producer | Tool needed |
|---|---|---|---|
| Image | png, jpg, jpeg, webp | `files-helper` (bounded PNG/JPEG/WebP decoders) | none |
| Video/GIF | mp4, mkv, webm, mov, gif, ... | existing first-frame snapshot | `ffprobe`, `ffmpeg` |
| PDF first page | pdf | `pdftoppm -f 1 -l 1 -singlefile -scale-to 160` | Poppler `pdftoppm` |
| SVG | svg, svgz | `rsvg-convert --width 160 --height 160 --keep-aspect-ratio --format png -`, then the helper decodes the PNG | librsvg `rsvg-convert` |

Not supported:

- Audio cover art. The audio preview only reads tags through `ffprobe` and
  has no cover extraction to reuse, so audio files keep their type icon.
- Office documents. The DOCX preview produces text, not a first-page image.

Every producer is a direct child process: fixed argv, no shell, no nested
helpers. It runs from the single thumbnail worker thread, never the UI
thread. The common supervisor gives it a wall timeout, cancel, a stdout cap
and a reap before the reply.

- PDF: the same pinned source descriptor (`/proc/self/fd/N`) and rlimits as
  the PDF preview (96 MiB address space, 3 s CPU, bounded descriptors, no
  core). Source of at most 256 MiB. PPM output of at most 160x160x3 + 4 KiB,
  validated strictly before any pixel is used.
- SVG: the verified descriptor becomes the child's stdin and the argument is
  `-`. librsvg therefore has no base URL and refuses every relative or
  `file://` reference; it never fetches network resources. `--unlimited` is
  not passed, so librsvg keeps its XML size limits. A test checks that a
  sibling image referenced both relatively and by `file://` is not loaded.
  Rlimits: 384 MiB address space (fontconfig and cairo need about 192 MiB of
  mappings), 3 s CPU, 64 descriptors, no core. Source of at most 8 MiB; PNG
  output of at most 512 KiB. The PNG goes through one owned 0600 scratch
  file in the private cache directory (`render-<pid>.png`, removed right
  after). The existing helper decodes it to at most 160 px, so no decoder
  runs in the UI process.

A missing tool is detected when its spawn fails with "not found". The worker
remembers that for the rest of the session: later files of that kind get
`NoThumbnail` at once, without spawning anything. `NoThumbnail` shows the
type icon with no error text and is not counted in the grid's error total.
Other per-file failures (protected or damaged PDF, invalid SVG) keep the
existing behaviour: one error per revision, without re-decoding.

## Scheduling and opt-out

The new kinds go through the same `thumbnail_job::supported` predicate and
the same demand as images:

- `show_thumbnails = false` (View menu) suspends all demand, including PDF
  and SVG.
- Demand covers only the selection and up to 32 visible rows
  (`model.rs` `update_thumbnail_demand`).
- One worker with a single in-flight request (`sync_channel(1)`). The
  concurrency cap is therefore one child process at a time.

Scrolling a folder of 5,000 PDFs spawns `pdftoppm` only for the rows that
are visible, one at a time. Rows that scroll away are cancelled. The
once-per-second revision re-probe answers `Unchanged` without spawning.

## Cache tiers

1. **RAM LRU** (unchanged): 32 entries, 4 MiB of RGB, shared between the
   grid and the inspector.
2. **Private packed tier** (`thumbnail-cache-v3` under Files' config
   directory): raw 160 px RGB, so a hit needs no helper process. This tier
   was kept rather than replaced because it is the cheapest hit and already
   holds descriptor-level identity (device, inode, size, mtime and ctime with
   nanoseconds). It now has **1024** direct-mapped slots instead of 256, each
   at most 80 KiB, about 80 MiB of disk at worst and no RAM. Slot names are
   unchanged, so earlier records are reused and orphan none. New format codes:
   `pdf` = 5, `svg` = 6.
3. **Shared freedesktop cache** (`$XDG_CACHE_HOME/thumbnails`, or
   `~/.cache/thumbnails`). It is read on a private-tier miss and written
   after Files generates a thumbnail.
4. **Generate** with the kind's producer.

### Reading the shared cache

The file name is `MD5(uri).png`. The URI is the absolute `file://` URI with
GLib's `g_filename_to_uri` escaping: ASCII alphanumerics and
`!$&'()*+,-./:=@_~` stay literal; every other byte becomes `%XX` in upper
case. Symlinks are not resolved, the same as GIO. `large/` (at most 256 px)
is tried first, then `normal/` (at most 128 px).

The cache is untrusted input:

- The `thumbnails` directory and the size directories are opened without
  following symlinks and must belong to the user. The entry is opened with
  `O_NOFOLLOW`, must be a regular file owned by the user and at most
  512 KiB.
- `thumbnail_standard::read_meta` walks chunk headers only. It requires the
  signature, an initial 13-byte IHDR with dimensions from 1 up to the
  directory's size, at most 1024 chunks and a reachable IEND. It reads only
  uncompressed `tEXt`/`iTXt` chunks of at most 16 KiB and checks their CRC.
  A duplicated `Thumb::` key rejects the file. `zTXt` and compressed `iTXt`
  are ignored, so nothing is inflated in the UI process.
- An entry is accepted only when `Thumb::URI` equals the URI, `Thumb::MTime`
  equals the source mtime in seconds, and `Thumb::Size`, when present,
  equals the source size. Any mismatch or malformed entry is a miss, never
  an error.
- Pixels are decoded by `files-helper preview_thumbnail` on the cache file.
  The helper re-checks the cache file's own revision (device, inode, size,
  mtime and ctime) before decoding and uses the bounded PNG row decoder, with
  output of at most 160 px. If the decode fails, Files generates as usual.
  A hit is copied into the private tier.

`fail/` is not consulted. Its entries are per application and describe other
decoders' failures, so Files never writes `fail/` either. Files' own failures
are remembered per revision for the session.

### Writing the shared cache

After a successful generation, Files writes `normal/MD5(uri).png`. It box
filters the 160 px RGB down to at most 128 px; it never upscales. The PNG
has 8-bit RGB and the tEXt keys `Thumb::URI`, `Thumb::MTime`, `Thumb::Size`
and `Software=Files`. The write is atomic:

- Missing directories are created 0700. An existing directory that belongs
  to someone else, is group- or world-writable, or is a symlink is refused.
- The data goes to `<name>.files-<pid>-<n>.tmp`, opened with
  `O_CREAT|O_EXCL|O_NOFOLLOW` and set to exactly 0600 whatever the umask.
- The temporary file is renamed over the target. A failure or cancel
  unlinks only that temporary file.

Nothing is written for:

- sources inside the cache root, compared lexically and through the real
  path of the parent (thumbnails are never thumbnailed);
- sources on network or FUSE filesystems, detected with `fstatfs` on the
  verified descriptor: NFS, SMB/CIFS/SMB2, FUSE (sshfs, GVfs, rclone),
  Coda, AFS, Ceph, 9p, NCP, OCFS2 and GFS2;
- sources under `/run/media` or `/media` (removable media);
- paths without a canonical URI (relative paths or `..`).

Reads are still allowed for network files, because a hit avoids reading the
remote file.

Shared-cache write failures are counted in the worker diagnostics
(`shared_errors`). They do not raise a notice, because another application's
directory layout is not a Files error. The diagnostics also expose
`shared_hits`, `shared_writes` and `tools_missing`.

### Test safety

`shared_thumbnail_cache::default_root()` returns `None` under `cfg(test)`, so
unit tests never read or write the real `~/.cache/thumbnails`. Tests inject
a root inside the private run directory through `thumbnail_job::Options`,
retain every fixture and delete no directory trees.

## Memory

No RAM bound changes. The RAM LRU, demand keys and sampling buffer keep
their limits. Transient worker scratch per request is at most:

- one encoded PNG (128 px RGB, about 50 KiB before compression);
- one text chunk of at most 16 KiB;
- tool output of at most 512 KiB (SVG PNG) or 76.8 KiB plus 4 KiB (PDF PPM);
- the existing 160 px RGB result.

All of it is freed before the next request. Decoders and rasterizers live
only in the ephemeral child processes; their rlimits are containment, not a
PSS measurement.

## Code

- `crates/core/src/thumbnail_standard.rs`: URI, MD5, chunk reader, PNG writer
  and box filter.
- `crates/ui/src/shared_thumbnail_cache.rs`: lookup, store and exclusions.
- `crates/ui/src/svg_thumbnail.rs`, `pdf_preview_job::thumbnail`: producers.
- `crates/ui/src/thumbnail_job.rs`: kinds, tier order, the tool memo and
  `NoThumbnail`.
