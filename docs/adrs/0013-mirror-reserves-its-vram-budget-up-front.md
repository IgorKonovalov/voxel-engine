# ADR-0013: The mirror reserves its VRAM budget up front and never reallocates

> **Status:** accepted (2026-10-17)
> **Date:** 2026-10-17
> **Related plan(s):** [Plan 0017](../plans/done/0017-mirror-budget-reservation-page-kinds-and-pressure.md)
> **Supersedes in part, once accepted:** ADR-0004's Outcome clause "buffers grow by half
> again and never shrink; the budget is read on allocated bytes".

## Context

The mirror's pool shards, mask buffer and record buffer are created small and grown on
demand: each grow allocates a buffer of `max(need, 1.5 x current)` rounded to a page, copies
the old contents on the GPU, and keeps the new size for good. Freed slots return to a free
list; no page or buffer is ever released, and a page keeps the kind (plain or attribute) it
was first issued as. Three consequences were measured in Plan 0002 and Plan 0013
(`nfr.md` §3, `decisions-pending.md` D1):

- Allocated bytes run 1.44x to 1.93x above live bytes under motion: a plateau reached by
  grows has little slack, the streamer's hysteresis pushes live just past it, and one more
  step adds half again. With attribute pages the plain and attribute free lists cannot serve
  each other, so the pool tracks the sum of each kind's peak rather than the peak of the sum.
- The budget line (3 GB) is read on allocated bytes, so a world whose live bytes fit (2.56
  GB at a 14-chunk radius with contour) still fails it, which is why Plan 0013's radius had
  to be chosen against the overhead and not against the content.
- During a grow the old and new shard both exist until the copy completes, about 2.5x the
  old shard, which `allocated_bytes` does not count. The contour run at the default radius
  stopped on `wgpu error: Out of Memory` on the 8 GiB card at 4.10 GB allocated; the
  transient is the plausible cause. Each grow is also a GPU copy of up to 2 GB, a frame hitch
  on the order of 10 ms on the dev box (an estimate).

`wgpu` cannot resize a buffer in place. Shrinking means truncating a free tail or compacting
live slots with GPU copies and a rewrite of every record entry, and the attribute pass
addresses a brick's attributes by its slot.

## Decision

**The mirror reserves its pool, mask and record buffers once, at the size the caller
budgets, and never creates another.** `MirrorConfig` carries the budget in bytes; the
mirror splits it over the shards and the record buffer by the adapter's limits, and a sync
that would exceed it fails with `PoolTooLarge` or `RecordsTooLarge` before writing anything,
as today. Allocated bytes equal the budget from the first frame; the `nfr.md` §3 line is
read on live bytes against that budget. **An empty page changes kind**: a page whose live
slots reach zero returns to a free-page list that either kind may draw from, so plain and
attribute slots share one high-water mark. The mirror reports its pressure (live over
budget) and the application decides what to do when it rises: the fly-through lowers the
full-detail radius by one chunk at 90 % pressure and restores it below 70 %, so the frame
never waits on a reallocation and the world degrades to LOD-only at the edge, which is what
`nfr.md` §3 says happens when the budget is exceeded.

## Consequences

### Positive

- No grow copy, no hitch and no uncounted transient: the mirror's VRAM is one number, set
  before the first frame, and the out-of-memory failure of the contour run cannot recur
  from the mirror's own allocation.
- Live bytes are the figure that matters, so a radius is chosen against content. At the
  interim radius 14 with contour (D1), 2.56 GB live fits the 3 GB budget with the streamer's
  hysteresis on top.
- The kind split stops costing the sum of two peaks.
- The reservation is the shape every shipped engine uses for its world VRAM, and it is what
  a fixed-memory portable profile needs: a budget, not a growth curve.

### Negative

- **A small world reserves the full budget.** The level (208 MB live) would hold 3 GB of
  VRAM if the application passed the default. The fly-through passes a budget sized to the
  world it loads (the level's extent, or the streamer's radii), and tests pass small ones;
  the reservation is cheap to create (no fill) but it is held.
- **The budget is a guess until the adapter is known.** `wgpu` does not report VRAM size
  portably, so the budget is configuration (the `nfr.md` §3 line by default), clamped to
  the adapter's binding limits. A machine with less VRAM than the budget fails at creation,
  which is an explicit error instead of a later OOM.
- **Pressure handling is the application's**, not the mirror's: the mirror cannot evict
  world residency. The fly-through's radius rule is one policy; a game may prefer another.
- **Empty-page re-kinding needs a live count per page** (one `u16` per 4096 slots) and
  a free-page list; a page that is almost empty still holds its kind.

## Alternatives considered

### Alternative A: keep growing, add shrinking by compaction

Compact live slots into the low pages at a quiet moment and truncate. It keeps a small
world small. It lost because compaction moves slots: every record entry, the attribute
slots and any staged run must be rewritten under the chunk locks, the copy is itself a hitch,
and "a quiet moment" does not exist while the camera moves. The reservation removes the
problem instead of managing it.

### Alternative B: grow by need in page units, no 1.5x factor

Allocate exactly the pages needed each time. Allocated tracks live closely, but every grow
still copies the whole shard, so the number of copies rises from a handful to hundreds over
a flight, each a hitch and each an uncounted transient. Rejected.

### Alternative C: a per-page buffer list bound as a binding array

Allocate each 4096-slot page as its own buffer, so nothing is ever copied and pages are
released when empty. It needs storage-buffer binding arrays with non-uniform indexing, the
native-only feature ADR-0004 rejected for the shards; a fixed set of 8 bindings cannot hold
hundreds of pages. Rejected for the same reason.

## Outcome

Added 2026-10-20, from Plan 0017 Phase 3 (its Amendment of that date):

- **Pressure is the larger of pool-slot occupancy and record occupancy, not live over
  budget.** Live bytes over the whole reservation counted records reserved for chunks that are
  never resident and could not reach the 0.9 trigger before the pool ran out. The thresholds
  (lower at 0.9, restore under 0.7) and the rule's shape (`r_full` only, a chunk at a time)
  stand; the LOD radius never moves.
- **The streamed mirror reserves records for the residency sphere**, sized so the record term
  stays under the restore line while the streamer keeps its radius, instead of a record for
  every chunk of the window cube. A mirror given no record count keeps the cube.

Added 2026-10-20, from Plan 0017's review fix pass (its second Amendment of that date):

- **A sync writes what fits, nearest first, never part of a chunk.** It no longer fails
  before writing anything for the whole selection. The Decision's "fails with `PoolTooLarge` or
  `RecordsTooLarge` before writing anything" now holds only for a sync whose nearest chunk
  does not fit and no demotion behind it does. Otherwise it uploads the nearest-first chunks
  that fit, after crediting their own tier-change frees, plus any demotion that fits. It
  defers the rest, which stay queued. With attribute pages the pool ran out one page short
  above the 0.9 trigger, while about 9 % of its slots sat spare in plain pages. Rejecting
  the whole selection then hid every edit until the rule's decrease landed. The
  reservation, the page kinds and the rule stand.

Added 2026-10-22, from Plan 0028 (ADR-0012's mirror-slot step):

- **Page kinds became bins.** A page of 4096 64-byte units takes one of ten bins (2 to 36
  units a slot) when first issued and returns to the free pages when its last slot leaves, the
  two kinds generalised. The reservation, the nearest-first partial sync, the deferral rules
  and the pressure rule stand, with the slot term read on units. At the streamed default's
  full residency the bins hold 0.10 % of live units spare, and the default no longer reaches
  the rule's trigger.
