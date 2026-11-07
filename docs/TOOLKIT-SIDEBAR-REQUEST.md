# Sidebar interaction contract needed by Files

Files needs discoverable pin/unpin, persistent ordering by stable keys, and
dragging content without activating its destination. Current Files integration
provides a persisted pin footer and explicit row-order commands. Reordering
must affect navigation rows, never files on disk. Bookmarks retain their
existing persistent IDs; paths use lossless platform-local keys.

The current toolkit starts row activation in `FL_PUSH`, before crossing the
five-pixel drag threshold. Only successful drops invoke `on_move`; cancelled,
outside and hidden-window drags have no public lifecycle signal. Sections
cannot initiate a drag. Files therefore cannot implement a reliable delayed
navigation adapter through the public API, and does not enable this drag path.

## Requested public behavior

- An optional pin control with accessible name/state and a callback; pin keeps
  the sidebar expanded, while unpin restores hover behavior and reduced motion.
- Distinct click and drag gestures: activate only on a completed click, or
  provide a dedicated drag handle. Keyboard activation remains immediate.
- Drag begin, move, commit and cancel signals keyed by stable row/section ID.
  Cancellation covers Escape, release outside, target rejection, hide/delete,
  model revision and loss of capture. It never triggers late activation.
- Explicit section reorder policy, in addition to sibling-row reorder. The
  application owns persistence, identity and reconciliation of absent devices.
- A rejected or stale move restores visual order and preserves selection,
  keyboard focus, scroll and context menu targets. No path conversion occurs.

## Required acceptance

Use new private retained fixtures and genuine pointer input: click a place;
drag it without navigating; cancel at each lifecycle boundary; reorder rows
and sections; reject a move; hide/delete while dragging; reopen with saved
order and missing/reappearing devices. Check keyboard, focus, native capture,
animation reversal and reduced-motion endpoints. Test raw non-UTF-8 path keys.

The toolkit harness involved in the fixture-cleanup incident remains stopped.
This request does not authorize running it, an alternative runner or CI.
The connected thread tool currently reports `Authentication required`; this
document is the concrete request pending delivery to the toolkit thread.
