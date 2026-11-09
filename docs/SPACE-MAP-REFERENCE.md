# Space view: bounded scanning and reusable rendering

## Reference and scope

Reviewed disktree commit `9b18b272ca3c5a145f6ff58eaf89dc4d311eeaea`
without running its build, tests or application:

- [Scanner](https://github.com/tobi/disktree/blob/9b18b272ca3c5a145f6ff58eaf89dc4d311eeaea/crates/disktree-core/src/scan.rs): directory tasks aggregate completed children; progress counters and cancellation are separate from the final tree.
- [Admission](https://github.com/tobi/disktree/blob/9b18b272ca3c5a145f6ff58eaf89dc4d311eeaea/crates/disktree-core/src/scan_threads.rs): each scan has a concurrency budget instead of occupying workers with blocked queued work.
- [Layout](https://github.com/tobi/disktree/blob/9b18b272ca3c5a145f6ff58eaf89dc4d311eeaea/crates/disktree-core/src/treemap.rs): squarified rows improve tile aspect ratios; layout is distinct from drawing.
- [Canvas](https://github.com/tobi/disktree/blob/9b18b272ca3c5a145f6ff58eaf89dc4d311eeaea/crates/disktree-app/src/treemap_view.rs): tiles share a painted surface rather than allocating a widget for every rectangle.

Files already caches geometry and paints a bounded surface. A GPUI migration
or copying a whole recursive tree is unnecessary. No disktree code was copied
and no throughput, memory or latency improvement has been measured.

## Concrete repairs

Previously the helper descended into the first directory before even admitting
later immediate children. One large subtree could exhaust the whole deadline,
hiding later names and direct-file measurements. It now gathers bounded root
metadata first, then shares the remaining cooperative time and visit budgets
between admitted directories. Only one depth-bounded walk is active. Root
omission knowledge is independent of whether individual subtrees complete.
The shared deadline, 100,000,000 visits, 2,048 nodes and aggregate metadata caps
remain. A subtree stopped only by its fair share is measured again from scratch
after all its siblings, with every remaining visit and second, so unused budget
completes it instead of leaving it partial (2026-11-09: a home folder whose
largest child exceeded its share showed only a lower bound). Re-walked entries
count again in `visited`.

Partial sizes remain lower bounds and are never promoted to exact areas.
Root-relative descriptor opens still reject final symlinks and verify the
captured directory identity. Subtrees are not filesystem snapshots. A blocked
filesystem syscall still requires the existing off-UI helper supervisor.

A rejected numerical layout previously lost every positive node's keyboard
and row target. Geometry failure now retains all visible children in the
fallback list and reserves only a compact diagnostic above it. It invents no
rectangles or byte values.

These repairs do not establish the cause of the user's reported error; the
exact message or runtime evidence is still needed to identify that incident.

## Toolkit proposal

A reusable treemap surface is appropriate for the toolkit if more consumers
need it. Its contract should accept stable application-owned keys and bounded,
finite nonnegative weights; cache layout by data/viewport revision; and expose
selection, hover, activation and keyboard navigation by key. Squarified layout,
viewport clipping, eased hover/selection and reduced-motion endpoints belong
there. Small or unrepresentable tiles need an accessible overflow/list slot;
zero and unknown weights must remain distinct. There must be no per-tile native
window, filesystem work, recursive watcher, deletion authority or implicit
normalization of partial measurements inside the widget.

Files should retain scanning, path identity, cache invalidation, error policy
and worker cancellation/ACK ownership. A later scanner improvement could use
a small bounded directory admission pool and replaceable progress snapshots;
that requires separate protocol and resource validation. This proposal is not
an implemented or validated toolkit API.

## Validation state

Targeted scanner and geometry regression cases are prepared. Scanner fixtures
require a private owned `TMPDIR`, reject symlink components and retain their
whole tree without cleanup. No tests, builds, GUI runs or performance probes
were executed for this change; retained-fixture boundaries require review before execution.
