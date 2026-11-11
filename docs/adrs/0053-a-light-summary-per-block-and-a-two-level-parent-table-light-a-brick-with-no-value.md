# ADR-0053: A light summary per block and a two-level parent table light a brick with no baked value

> **Status:** accepted (2026-11-11, owner: the go on Plan 0067 with the defaults on its open
> points; the design is that plan's Phase 4 as approved)
> **Date:** 2026-11-11
> **Related plan(s):** [Plan 0067](../plans/0067-the-frames-cpu-a-cheap-queue-step-a-bake-budget-by-time-and-no-black-for-missing-light.md)
> (Phase 4 builds it; Phase 6's window picks the default mode and may overrule a default below).
> Adds two levels above the brick level of
> [ADR-0046](0046-static-emissive-light-is-baked-per-chunk-when-a-chunk-loads-or-changes.md)'s
> store and changes nothing it decided about the bake, the queue or the brick record.

## Context

ADR-0046's store, as built (its Outcome), holds light at one level: a surface brick's record,
`BRICK_WORDS` (7) words a cell, six `rgb9e5` direction values and `material | direction mask
<< 16` (`crates/vxe-render/src/bake.rs`). The pool's header is 16 words (the frame's counters in
words 0 to 2, the window's origin at 4 and size at 8, the flags at 12; words 3, 7, 11 and 13 to
15 unused), then `STORE_TABLE` one word per mirror-window chunk naming its block, bit 31
`TABLE_COARSE` on a coarse block; the blocks follow. A cube face reads the 2 x 2 cells around it
in its plane (`store_brick_value` in `temporal.wgsl`, `store_tap` in `march.wgsl`), bilinear over
those that have its direction, else its own cell, else 0, counted in `store_missing`. Nothing
coarser than a brick holds light, and ADR-0003's mips carry geometry only.

So a pixel whose chunk has no block reads black. The queue frees the block of every chunk the
mirror does not hold full (`BakeQueue::step`: "evicted, or LOD-only"), so the whole LOD ring is
black, as is every full chunk still waiting. On the Radeon (RADV RENOIR) at the window's size
Plan 0060's window read 36 932 bricks missing after 137 s, and the owner saw distant caves stay
black (`docs/plans/logs/0060.md`, Phase 4). ADR-0052 gives the slow adapter the fast one's bake
time a second but not its bake; something has to light the wait.

The owner asked (2026-11-11) that a brick with no value take its **neighbour's or its parent's**
light, declined a flat constant, and at the go on Plan 0067 took the defaults on how: neighbour
before parent; black (counted) where neither has light; "parent" a new two-level table plus the
chunk's own summary; six face neighbours with a plain mean; no fade of a neighbour's arrival into
its neighbours' fallback; the LOD ring lit by the chain.

## Decision

**Every block carries one summary record, and a two-level parent table after the chunk table
carries one record a cell; a cube face with no brick value reads, nearest first, its chunk's
summary, the mean of its six face neighbours' summaries, then level 1, then level 2, then 0.**

- **The summary.** One record in the brick record's format (`BRICK_WORDS`, 28 bytes), placed
  so the chunk's table word reaches it with no second table read (the place is `dev`'s, named in
  `bake.rs`'s module docs). A direction's value is the mean of the block's cell values of that
  direction over the cells that have it; the tail's material is the emitter material the most of
  those cells keep, its mask the union. The CPU writes it when the queue places a block and on
  every frame of that block's fade, from the words it already holds (`Store::words`, the fade's
  shown words), and it goes with the block when the block is freed.
- **The parent table.** Level 1 over cells of 2 x 2 x 2 chunks (128 voxels, 12.8 m at 10 cm),
  level 2 over 4 x 4 x 4 (256 voxels), placed after the chunk table in the header region and
  sized from the window's size. A cell is aligned to world chunk coordinates (`c >> 1`, `c >> 2`),
  so a window move re-bins no chunk; the plan left the alignment open and this ADR fixes it. A
  level-1 record is the mean by direction of its children's present summaries, a level-2 record
  the mean of level 1's present records; a cell with no present child holds no value. The CPU
  rewrites the two records above a summary whenever it changes, so a parent fades with its
  children and empties when its last child's block is freed. A rewindow rewrites the table with
  the chunk table; a window of another size empties both, as it empties the store today.
- **The read** (`temporal.wgsl`, cube faces only), where `store_brick_value` finds no value for
  the face's direction: (i) the hit's own chunk's summary, where the chunk holds a block; (ii) the
  plain mean of the six face-neighbour chunks' summaries that have the direction; (iii) level 1's
  record, then level 2's; (iv) 0, counted still black. One value per chunk and direction, never
  per voxel, times the summary's one material's emission scale now, as a cell's read is. The LOD
  ring is read like any other chunk. `march.wgsl`'s `glass_store_value` keeps reading 0.
- **Arrival.** Under a fallback mode a chunk's first block fades over `FADE` frames from a block
  whose every cell holds the chain's value for that chunk when it landed (the CPU walks the same
  chain over the records it wrote); under `black` it is written directly, as today. A
  neighbour's arrival is not faded into the fallback of the chunks around it; a weight on a
  summary that ramps over `FADE` frames is the fix if Phase 6's window shows the step.
- **Counted.** Two of the header's unused words count the pixels that took a fallback and those
  the chain left black, beside `store_missing`.

## Consequences

### Positive
- A waiting chunk, a dug face before its re-bake and the LOD ring read the nearest baked light
  the store holds instead of black, at no bake cost: every summary and parent is a mean of
  records the CPU already wrote.
- One value per chunk and direction is a value the arriving block can fade from, so a first
  block does not pop where the chain had light.
- Pixels with a brick value are unchanged: the chain runs only where today's read returns 0, and
  under `black` every golden stays byte-identical.

### Negative
- **Borrowed light.** A sealed cave beside a lit hall shows the hall's light until its own block
  lands. The LOD ring within a level-2 cell (25.6 m) of a baked chunk is lit from it, and the
  rest of the ring stays black: a change to the far ring's look the owner judges in the window.
- **Tiles.** A fallback chunk is one value a direction across 6.4 m; its edge against a baked
  chunk can read as a tile, and a neighbour's first block changes its neighbours' fallback in one
  frame.
- **One emission scale a summary.** A chunk lit by two emitter materials whose scales change
  apart reads a borrowed value off by their ratio until its own block lands.
- **Cost at a no-value pixel:** up to 7 table words and 9 records (own, six neighbours, two
  parents). A guess, unmeasured: at most +0.1 ms on the RTX 3080 Laptop and +0.5 ms on the
  Radeon at the window's size; Plan 0067 Phase 4 reads it.
- **Memory** (arithmetic): 28 bytes a block, 89 KB at the level's 3 168 chunks, 476 KB at 17 000.
  The parent table at a window 51 chunks a side (a guess at the side from `r_lod` 24 and
  hysteresis 1, not read): at most 27^3 level-1 and 14^3 level-2 cells, about 0.63 MB, inside
  the 256 MiB pool, whose blocks now start that much later.
- **CPU per change:** one summary over a block's records and two parent records, at placement
  and on every fade frame. A guess: under 0.1 ms of the queue's step p50 in flight; Plan 0067's
  step breakdown reads it.
- A chunk with no baked chunk within its level-2 cell stays black. A third level (512 voxels) or
  the window's mean would light it; neither is built.

## Alternatives considered

### Alternative A: A flat constant `L` for missing light

Plan 0067's draft recommendation: one constant the read returns where a brick has no value. The
owner did not take it (2026-11-11). It lights every waiting cave alike, dark or lit, and carries
nothing of the light around it, while the store already holds the nearest real light.

### Alternative B: A per-brick neighbour read

Read the neighbour chunk's nearest brick across the shared face, or widen `store_brick_value`'s
2 x 2 tap until it finds a cell. The brick across the face is often air or holds only faces of
the other direction, so the read misses exactly where it is needed, and a miss turns into a
search of unbounded table, directory and record reads per pixel. Its value is per voxel, so it
cannot be the value an arriving block fades from, and the chunk would pop when its block lands. A
per-block summary answers whenever the neighbour has any face of the direction, in one record.

### Alternative C: Parents built on the GPU by a reduction pass

A pass that reduces the store's records into coarse levels each frame or on change. The blocks
sit at scattered places in one pool with no fixed layout, so the pass would walk every chunk's
directory; the CPU already holds every record it writes (`Store::words`) and changes a summary
only when it writes a block or a fade frame, two parent records at a time.
