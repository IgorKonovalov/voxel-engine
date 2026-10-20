# ADR-0016: A falling region crushes the sub-threshold voxels in its path

> **Status:** proposed
> **Date:** 2026-10-20
> **Related plan(s):** [Plan 0022](../plans/0022-a-falling-region-crushes-sub-threshold-voxels.md)
> **Amends:** ADR-0008's drop clause ("translates a region's voxels straight down by the minimum
> air gap under its columns"). The flood, the cap, the removal and the rejected alternatives of
> ADR-0008 stand.

## Context

ADR-0008 made collapse two halves that use different rules for the same voxels. The support
query treats a coarse cell with fewer than `solid_min` solid voxels as holding nothing up (it is
neither in a region nor a path to an anchor; `crates/vxe-world/src/support.rs`, the
`cell.solid < params.solid_min` skips). `drop_region` stops each column's gap at the first solid
voxel outside the region, whatever its cell holds (`Snapshot::gap_below` in
`crates/vxe-world/src/collapse.rs`). ADR-0008's Negative section lists both effects ("a thin
ledge under a slab reads as unsupported"; "a region whose columns are all obstructed drops zero")
without connecting them.

Together they produce a region that is unsupported and wedged at once. The owner hit it on
2026-10-20 in `flythrough --level`: every unsupported region reported `landed after 0 ticks, fell
0 voxels` (recorded in Plan 0018's implementation log at 84a3998). A headless repro of 40 000
seeded digs near the level's spawn found six unsupported regions; all six dropped 0 and none was
refused. The first is a 4-cell, 501-voxel piece whose columns rest on four cells holding 31 to
38 solid voxels each, half the threshold. In the generated cave this is the common case, not an
edge: a carve leaves ragged slivers, and the region the query detaches nearly always sits on
one.

The two halves have to agree on what a sub-threshold cell is. The owner chose on 2026-10-20 that
the drop changes, not the query: a falling region goes through the sub-threshold voxels below
it. Whether it destroys them (crush) or pushes them down ahead of it (carry) was left open.

## Decision

**`drop_region` takes a `yield_below` threshold. A solid voxel outside the region whose cell
holds fewer than `yield_below` solid voxels, and whose material is not `INDESTRUCTIBLE`, yields:
the gap scan passes through it as if it were air, and the drop crushes it.** In each column of
the footprint, every yielding voxel in the span a region run sweeps (from the run's lowest voxel
down by `dropped`) becomes air or is overwritten by the landed region. The result reports the
crushed voxels per material in a new `DropResult::crushed`, and its touched chunks and bounds
include them. Cell counts are read from the bricks when the op starts, so the rule is integer
and does not depend on the coarse grid. The scan still stops at a voxel of a cell at or above the
threshold, an `INDESTRUCTIBLE` voxel and a non-resident chunk, as it does today.

`yield_below` is a parameter of the op, not a world setting, so a logged op replays without a
side channel. `yield_below == 0` yields nothing and is exactly today's drop. A caller that sets
`yield_below` to the query's `solid_min` gets a drop that agrees with the query. With that
setting, a region the query has just reported moves at least one voxel unless an
`INDESTRUCTIBLE` voxel or a non-resident chunk blocks one of its columns. The cell under the
lowest region cell of any column is not load-bearing, or the flood would have taken it in, so
it yields. On the wire, a drop with `yield_below == 0` keeps op kind 3, byte for byte. Any
other value is the new op kind 5, which adds the threshold as a `u16`; `LOG_FORMAT` stays 2.

**The crushed volume comes back as rubble, through a separate op.** `World::pile_rubble(cells,
rubble, count, max_depth)`, op kind 6, spreads `count` rubble voxels over the footprint columns
of `cells`. It uses `remove_region`'s rubble stage without the carve: the same fixed column
order, the same integer accumulator (each column weighs 1, the fraction is `count` over the
column count), and the same upward write into air from the first solid voxel at or below the
top of each column's highest named cell. A caller that wants debris issues it when a fall
ends. The fly-through's logic does so with the fall's whole crushed total. The drop itself
stays a pure move-and-crush: its moved set equals the region, and any rubble it placed during a
fall would sit above the piece, in cells the next tick does not name.

### Owner decisions (2026-10-20)

Crush, not carry. The crushed volume becomes rubble, piled at the end of a fall through the
server and the log. Op kind 5 for a drop with a threshold, with `LOG_FORMAT` kept at 2. One
shared threshold: the logic passes `vxe_net::DEFAULT_SUPPORT.solid_min`, which the fly-through
also sets on its server.

## Consequences

### Positive

- The support query and the drop agree. A detached piece falls through the slivers it rests on
  and lands on the first cell that bears load, which is what the query said holds things up.
- The result says what the drop destroyed (`crushed`), so the edit-result contract holds, and
  the pile op turns that volume back into rubble deterministically, through the log.
- Bounded: a drop crushes at most `dropped` voxels per footprint column, all inside the bounds it
  already writes.
- Saves stay compatible. Kind-3 entries decode to `yield_below: 0` and replay bit-identically.

### Negative

- The crushed voxels' materials are lost: the pile is one rubble material, on top of the landed
  piece rather than where the sliver was, and in a low passage it can run out of air and place
  fewer than it was given (the result's `placed` says how many).
- A fall that crushed anything costs one more logged op (kind 6, `27 + 12n` bytes).
- The rule cuts a sliver per column, so a thin sheet that reaches past the footprint keeps its
  overhanging part, hanging in the air at the old height. The query does not report it: its cell
  is below the threshold. This is the same class of leftover as ADR-0008's "voxels left in cells
  below the threshold stay where they are".
- Two wire kinds encode one op. Decoding refuses a kind-5 entry whose threshold is 0, so every
  op has one encoding. A build older than this one refuses a save that holds a kind-5 or kind-6
  entry with `UnknownKind` at that entry, not at the header (the log format stays 2).
- Pinned hashes move: the collapse course gains a crush step, and any saved session whose fall
  crosses a sliver lands lower. A session replayed by inputs on a newer logic can differ from its
  save, because a log names no logic version. That gap already existed.
- A region can still be wedged, by an `INDESTRUCTIBLE` voxel in a thin cell, or by a caller
  that names cells other than the query's or a `yield_below` other than its `solid_min`. A
  non-resident chunk refuses the drop. The logic must tell
  this case from a refusal and from a landing, and must not report it again forever (Plan 0022
  Phase 4).

## Alternatives considered

### Alternative A: Carry: the region pushes the yielding voxels down ahead of it

Volume is conserved: the sliver lands under the slab instead of vanishing. Rejected because the
carried voxels sit in cells that are not in the region. The logic re-sends the region's cells one
cell lower after each whole step, so on the next tick the carried voxels are in no named cell.
They are left behind mid-air, or the op has to return a new cell list that the logic adopts.
That changes the op's contract and the Plan 0018 bookkeeping for a cosmetic gain. Carry also
needs a rule for a column that stacks carried voxels from two cells (compact or keep rigid), and
a sliver that reaches sideways past the footprint is torn either way. Crush keeps the moved set
equal to the region and leaves debris to the game. Carry can return as a mode of the same
yield rule if a consumer asks for it.

### Alternative B: The support query adds the touching sub-threshold cells to the region

Rejected by the owner. It changes what the query means: a region's cells and counts would
include slivers, and a sliver touching both the region and a supported wall would join whichever
flood reaches it first. That makes regions depend on visiting order in a way they do not today.
It also moves every region the query reports, the collapse course's included. The drop would
then carry whole thin cells, including voxels far outside the falling piece's footprint.

### Alternative C: The logic crumbles a region that drops zero

Rejected by the owner. It hides the engine's disagreement behind one game's policy: every consumer
would learn the same fallback, a ceiling that should fall would turn to rubble at once, and the
engine would still report regions it cannot move. A wedged region and a sliver-blocked one would
look the same to the game.
