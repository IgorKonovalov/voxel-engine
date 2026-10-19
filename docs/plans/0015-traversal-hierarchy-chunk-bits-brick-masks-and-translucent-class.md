# 0015: The traversal hierarchy: chunk bits, full-resolution brick masks, a one-read-per-level walk, and the translucent class

> **Status:** in-progress
> **Created:** 2026-10-17
> **Milestone:** M2 in `docs/roadmap.md` (the frame-time floor before the lighting plan)
> **Related ADRs:** [ADR-0012](../adrs/0012-mask-first-brick-with-per-brick-palette-planes.md)
> (proposed; this plan is its first step and changes no brick format),
> [ADR-0004](../adrs/0004-palette-narrowed-sharded-brick-pool.md) (the pool, records and
> shards this plan keeps), ADR-0003 (the LOD ring whose mip-bricks get the same masks).
> **Prerequisite:** none. Independent of Plans 0006 and 0013; it touches `march.wgsl` and
> `gpu_world.rs`, which Plan 0013's branch also touches, so whichever merges second resolves
> the conflict.
> **Amended:** 2026-10-17, before `dev` started, from the traversal efficiency review of
> that day: the steps view lands as its own phase before any skip, so the walk's cost is
> read before and after; the mask buffer keeps every opaque mask on one 64-byte line
> (groups of eight slots); the kernel split and the coarse secondary walk are named as the
> lighting plan's, and beam optimization as a followup.
> **Amended:** 2026-10-18, after Phase 1 (52d6a7b), before Phase 2: the 64³ chunk skip
> reads an `EMPTY_BIT` in the chunk table entry instead of the record's 16 bits words, so
> an empty chunk costs its table read alone and Phase 4's counts are satisfiable (bullet 1
> becomes at most 7 reads, the rock chunk's entry read included); the walk holds the bits
> words it last read; Phase 2 also reads the march time and the per-block maximum of steps;
> the budget's expectation is restated from Phase 1's readings (the gap is not mainly step
> count); the streamed spawn is read at `--frames 3000`.
> **Amended:** 2026-10-19, inside Phase 3 (its code uncommitted on 106998d), from the resume
> note: the walk's off-axis coordinates at a cell exit follow plane order, so the image is
> independent of every skip; Phase 3's bit-exact bullet becomes the identity of the walk with
> every skip on and every skip off, plus a one-time re-bless of the goldens bounded to isolated
> pixels against 106998d; Phases 4 and 5 inherit the identity; Phase 5's seal bullet becomes
> an immutability test, there being no `seal` or `replace_material`; every reading of the plan
> is taken after Phase 6, serially and unattended, from each phase's commit.

## TL;DR

The marcher spends five dependent reads at every DDA step and walks an empty chunk brick by
brick, which is why the streamed cave's spawn view reads 6.69 ms against the small level's
1.59 ms at the same resolution. This plan gives the walk a hierarchy of occupancy bits, one
read per level per ray segment: 512 brick bits at the head of each chunk record, a 512-bit
opaque mask per brick in place of the 8-byte 2³ mask, and a walk that re-derives its chunk
and brick state only when it crosses into a new one, so the pool is read at the hit alone.
It also introduces the opacity class: translucent materials (glass, water, lava) are air to
the opaque walk and are composited by a second, bounded walk of at most `N` layers. The
first thing a reader sees is a `--view steps` heat map and a headless step histogram of
today's walk, landed before any skip so that every later phase reads against it, and the
frame figure the plan is held to is the streamed spawn's march time.

## Context & problem

`march.wgsl`'s `lookup` reads the chunk table, the chunk record entry, the 2³ mask word, the
pool byte and the palette pair in sequence, at every step. A chunk whose bricks are all
uniform air costs eight record reads to cross along an axis; the 2³ mask is the only skip
inside a brick, so a ray grazing a wall reads pool bytes for every voxel it visits. At 1080p
and 60 fps with three rays per pixel the dev box affords about twenty uncached 64-byte lines
per ray, a GTX 1660 about eight, a Switch-2-class portable about three (ADR-0012). The
streamed cave's spawn view reads 6.69 ms for primary rays plus the headlamp's shadow ray on
the dev box (`nfr.md` §2, a recorded miss against the 3.0 ms line); the level's spawn reads
1.59 ms. The difference is steps through air, which the hierarchy removes. (Amended
2026-10-18: Phase 1 does not bear this out. The streamed spawn's mean is 15.55 steps per ray
against the level's 10.48, 1.5 times, while its march time is 3.7 times; the p99 is 84
against 15. Most of the gap is cost per step, five dependent reads into a mirror of about
1.7 GB against the level's tens of MB, and the long tail of a SIMT group, which runs as long
as its slowest ray. The hierarchy addresses both, through Phase 4's reads per step and Phase
2's skips in the tail, but in that order of weight; see `## Budgets`.)

The consumer wants translucent voxels (glass, water, lava; owner, 2026-10-17) and
`project-context.md` §"Rendering" lists volumetrics at P1; gas and smoke are a separate
coarse volume (M8), not this plan. A translucent voxel must not stop the opaque walk, so
occupancy has to be split by class before any material is read.

Budgets touched: `nfr.md` §2 primary rays (the streamed and level spawn readings), §2
streaming line (masks raise a `Full` brick's upload from 520 to 584 B), §3 world VRAM (mask
and record bytes).

## Decision

We add a bit hierarchy to the mirror and restructure the walk around it, on today's 512-byte
pool slots and per-chunk palettes, so that nothing in `vxe-world` changes and ADR-0012's
format plans can follow without this plan waiting for them. Chunk bits live at the head of
the record and the 512-bit masks in the existing mask buffer, in groups of eight slots: eight
64-byte opaque masks, each on its own 64-byte line, then one line holding the eight slots'
translucent 2³ words (72 B per slot on average), so the marcher still binds eight storage
buffers, WebGPU's default limit. The grouping is what makes "one line per level" true in the
hardware and not only in word counts: a flat 72-byte slot would start seven of every eight
opaque masks off a line boundary, so each of those mask reads would touch two lines. The
64³ level is not a record word: a chunk whose bricks are all `Uniform(AIR)` carries
`EMPTY_BIT` (bit 29) in its chunk table entry, so the table read the walk already makes
decides the chunk skip and an empty chunk's record is never read (amended 2026-10-18). We
rejected testing the record's 16 bits words on chunk entry and counting the 64-byte line as
one read: it is one line, but 16 loads and a dependent read after the table's, and it would
make the Phase 4 counts a convention rather than a load count. We rejected a 17th summary
word in the record for the same dependent read. The record index keeps 29 bits, room for
536 870 912 records. We rejected a separate 1-bit mip chain for this plan because
the two mask levels plus the 4³ and 2³ tests on the same line already give four cell sizes
from two reads; a chain for secondary rays is the lighting plan's to add if it needs one. We
rejected changing the mip-level selection or adding the far-field clipmap here: they change
the look and belong to the far-field plan. We rejected screen-door transparency for now: it
needs the temporal pass the lighting plan brings, and a bounded accumulation with `N = 1` is
the floor profile's equivalent until then.

**The walk's off-axis coordinates at a cell exit follow plane order, not the float position**
(amended 2026-10-19, inside Phase 3). Today the walk sets the exit-axis coordinate exactly
from the face and the other two from `floor(o + d t)`, so a bigger empty cell that leaves
through a different face near a lattice corner can land one voxel diagonally off where the
voxel walk would not: the 4³ skip moved isolated pixels in every golden, and the 64³ and 16³
skips moved none by luck. We make the voxel entered a function of the per-plane crossing
values alone. Exiting through axis `k` at `t`, the walk advances each other axis `j` across
every voxel plane `B` whose `t_j(B) = (B - o_j) inv_j` sorts before `(t, k)` in `(t, axis)`
lexicographic order, which is exactly the order in which the voxel-by-voxel walk crosses them
(nearest face first, the lower axis on a tie), so any decomposition of empty space into
aligned cells enters the same solid voxel at the same `t` through the same face. The
mechanism is dev's; the shape we expect is an estimate from the float position corrected by
at most one plane on each off-axis with that predicate, where the estimate may be contracted
into an FMA and the predicate decides. We rejected re-blessing the goldens with an
isolated-pixel allowance against the skips: it turns "the hierarchy changes no pixel" from a
property into a tolerance that Phase 4's held state, Phase 5's second walk and the lighting
plan's coarse walk would all inherit. We rejected dropping the 4³ skip: it trades a read-free
level for luck, since the 16³ and 64³ skips carry the same exposure. We rejected an integer
walk on fixed-point `o` and `d`: the cross-multiplied predicates need more than 32 bits at the
window's range, WGSL has no portable 64-bit integer, and geometric exactness was never the
claim; cell-size independence is, and float arithmetic gives it under the plane-order rule.
The goldens re-bless once when the rule lands, because it breaks the same corner ties
differently from the float position; from then on bit-exactness is held by the identity of
the walk with every skip on and every skip off, which a tolerance never defended.

## Architecture diagram

```mermaid
flowchart LR
    subgraph vxe-world
        W[Chunk: 512 Brick] -->|sync| U
    end
    subgraph vxe-render
        U[GpuWorld::sync] --> R[record: 16 words brick bits, 512 entries, palette]
        U --> M[masks: per 8 slots, 8 aligned opaque lines + 1 line of translucent 2^3 words]
        U --> P[pool: 512 palette bytes per slot]
        T[chunk table: record index, LOD bit, EMPTY bit] -->|not empty| R
        R -->|bit set| M
        M -->|bit set| P
        P --> H[hit: material, contour]
        M -->|translucent bits| A[bounded walk, N layers]
        A --> H
    end
```

## Implementation phases

Each phase ships as its own commit. `dev` implements all phases in one session with no review
between phases. The architect reviews once at the end, in a fresh session.

### Phase 1: The steps view and the headless histogram, on today's walk
- **Owner skill:** dev
- **What:** a `steps` debug view and a headless histogram make the walk's cost visible
  before any skip changes it. This phase changes no layout and no walk; its readings are the
  "before" that every later phase is read against.
- **Files touched:** `crates/vxe-render/src/marcher.rs` (`ViewMode::Steps`; the view field
  widens to 3 bits), `crates/vxe-render/src/shaders/march.wgsl`,
  `crates/vxe-render/src/readback.rs` (a step histogram from the steps view),
  `crates/vxe-render/tests/mirror.rs` or a new `crates/vxe-render/tests/walk.rs` (the fixed-ray
  step counts, which Phases 2 to 4 extend), `apps/flythrough/src/main.rs` (`--view steps`;
  `--headless --frames N` prints mean and p99 steps per ray when the view is `steps`),
  `README.md` (the flag table).
- **Done when:**
  - In `VIEW_STEPS` the red channel holds `min(steps, 255) / 255` and the green channel
    `min(pool reads, 255) / 255` for the primary ray, where a step is one iteration of the
    walk's loop, the hitting iteration included. On a closure world of 4 × 1 × 1 chunks whose
    first three chunks are uniform air and whose fourth is uniform rock, a +x ray from the
    centre of the first chunk's -x face takes exactly 25 steps on today's walk (eight
    uniform-air bricks per air chunk, then the hit on the rock chunk's face) and 0 pool reads.
  - `apps/flythrough/tests/golden/{primary,headlamp,lod}.png` match bit-exactly on lavapipe:
    the view changes no shaded pixel.
  - `flythrough --headless --seed 1 --render 1920x1080 --frames 60 --view steps` prints mean
    and p99 steps per ray for the streamed spawn and, with `--level`, for the level spawn;
    the log records both, beside the march time of the same two views at this commit
    (median of the last 30 of 300 frames, timestamps, three runs, quiet dev box), as the
    plan's before figures.

### Phase 2: Chunk bits, the chunk-level and 16³ skips
- **Owner skill:** dev
- **What:** each chunk record starts with 512 brick bits (bit `b` set when brick `b` is not
  uniform air), and each chunk table entry carries `EMPTY_BIT` (bit 29) when the chunk's
  record holds no brick that is not uniform air; the walk skips an absent or empty chunk as
  one 64³ cell from the table entry alone, and a 2×2×2 brick group whose 8 bits are clear as
  one 16³ cell.
- **Files touched:** `crates/vxe-render/src/gpu_world.rs` (record layout: words 0..16 bits,
  16..528 entries, 528..656 palette; `RECORD_WORDS` 656; table entry: record index in bits
  0..29, `EMPTY_BIT` 1 << 29, `LOD_BIT` 1 << 30 as today, `ABSENT` as today; the table entry
  is rewritten whenever a re-synced record's emptiness changes, not only for a new record),
  `crates/vxe-render/src/readback.rs` (the mean over 8 × 8 pixel blocks of each block's
  maximum steps),
  `crates/vxe-render/src/shaders/march.wgsl`, `crates/vxe-render/tests/mirror.rs`,
  `apps/flythrough/tests/level_mirror.rs`, the Phase 1 fixed-ray test file.
- **Done when:**
  - On the level-size seed-1 world, a readback of every record's first 16 words equals, for
    each chunk, the bits of its bricks that are not `Uniform(AIR)` (`level_mirror.rs`), and
    the record stride is 2624 B.
  - On the same world, a readback of the chunk table has `EMPTY_BIT` set on exactly the
    entries whose record's 16 bits words are all zero, and the record index of every entry
    that is not `ABSENT` equals the CPU's. In `mirror.rs`, an edit that places one voxel in a
    mirrored empty chunk clears the entry's `EMPTY_BIT` at the next sync, and the edit that
    removes it sets the bit again.
  - On the Phase 1 four-chunk closure world, the same +x ray takes exactly 4 steps (one per
    air chunk, then the hit on the rock chunk's face) and 0 pool reads, from 25 in Phase 1.
    On the same world with the three air chunks replaced by chunks whose bricks are uniform
    air except a solid floor layer of bricks, a +x ray through the air above the floor takes
    at most 4 steps per chunk.
  - `apps/flythrough/tests/golden/{primary,headlamp,lod}.png` match bit-exactly on lavapipe.
  - The Phase 1 histogram command is re-run for both spawns (the streamed one at
    `--frames 3000`) and also prints the mean over 8 × 8 pixel blocks of each block's maximum
    steps, the count a SIMT group pays. The log records mean, p99 and block-max mean beside
    Phase 1's, with the block-max mean of Phase 1's walk read at 52d6a7b plus the readback
    change, and one march-time reading of each spawn (median of the last 30 frames, the
    streamed one at `--frames 3000`, the level at `--frames 300`), so the chunk skip's share
    of the gap is read before Phase 3.

### Phase 3: Full-resolution opaque masks, with 4³, 2³ and voxel skips from one line
- **Owner skill:** dev
- **What:** the mask buffer holds 16 words of opaque voxel bits per slot (word `2z + (y >= 4)`
  holds rows `y % 4` of layer `z`, bit `x + 8 (y % 4)`), laid out in groups of eight slots so
  that every opaque mask starts on a 64-byte boundary; the walk tests 4³ and 2³ blocks by
  masking the line and single voxels by bit, and reads the pool only when it has a hit.
  Mip-bricks get the same masks from their cells. The walk's off-axis coordinates at a cell
  exit follow plane order (amended 2026-10-19, see `## Decision`): exiting through axis `k`
  at `t`, every plane of another axis whose crossing sorts before `(t, k)` is crossed, so the
  voxel entered does not depend on the size of the cell left. A comparison switch disables
  every skip (the 64³, 16³ and 8³ cells and the mask tests), so the hierarchical walk can be
  held byte-identical to the voxel-by-voxel walk; whether that is `FLAG_MASKS` clear widened
  or a second debug flag is dev's call.
- **Files touched:** `crates/vxe-render/src/gpu_world.rs` (`MASK_GROUP_SLOTS` 8 and
  `MASK_GROUP_WORDS` 144: slot `s` has its opaque words at `144 (s / 8) + 16 (s % 8)` and
  its translucent words at `144 (s / 8) + 128 + 2 (s % 8)`; the buffer is sized in whole
  groups, and `grow_to` copies it as one block as today; the upload computes the bits in the
  pass that narrows the brick), `crates/vxe-render/src/shaders/march.wgsl` (the plane-order
  exit and its header comment), `crates/vxe-render/src/marcher.rs` if the switch is a new
  flag, `crates/vxe-world/src/brick.rs` (a `solid_bits() -> [u64; 8]` helper beside
  `occupancy_mask`), `crates/vxe-render/tests/mirror.rs`, `crates/vxe-render/tests/walk.rs`
  (the diagonal ray), `apps/flythrough/tests/level_mirror.rs`,
  `apps/flythrough/tests/golden.rs` (the identity test replaces
  `masks_move_only_isolated_pixels`), every PNG under `apps/flythrough/tests/golden/` (one
  re-bless), `docs/nfr.md` §3 (mask bytes).
- **Done when:**
  - On the level-size seed-1 world, a readback of each `Full` brick's 16 mask words equals
    its CPU `solid_bits()`, and the mask buffer's live bytes are `72 × Full bricks`:
    27 619 776 B for its 383 608 `Full` bricks (live accounting is per slot; the allocated
    size is whole 576-byte groups).
  - A layout test asserts, for slots 0 to 63, that the byte offset of a slot's first opaque
    word is a multiple of 64 and that its two translucent words lie in the group's ninth
    line (`144 (s / 8) + 128 + 2 (s % 8)`).
  - A primary ray that hits a voxel of a `Full` brick reads the pool exactly once (the green
    channel of the steps view reads `1 / 255` on the hit pixel of a fixed ray into a
    `Full` brick with a single solid voxel at its far corner, and the ray takes at most 4
    steps inside that brick: the empty 4³ block, the empty 2³ block, the empty voxel before
    the corner, the hit).
  - A ray through a `Full` brick whose voxels are all air except one 2³ block it misses
    reads the pool 0 times.
  - `FLAG_MASKS` cleared still walks every voxel of a `Full` brick through the pool, as a
    comparison switch (on the far-corner world above, the same ray reads 8 steps and 8 pool
    reads).
  - **The walk is independent of its skips** (amended 2026-10-19). On one lavapipe device,
    the `primary.png`, `headlamp.png` and `lod.png` scenes from their golden poses render
    byte-identically with every skip on and with every skip off (the comparison switch), and
    again with the mask tests alone off; the check is a strict byte comparison, as in
    `goldens_match_bit_exactly_over_one_and_several_shards`, with no pixel allowance. The
    voxel-by-voxel walk stays under `MAX_STEPS` on these scenes (their windows are at most a
    few hundred voxels across; a ray that ran out of steps would show as a differing pixel).
  - **The rule holds on a ray built to tie at every corner** (amended 2026-10-19). In
    `walk.rs`, on a one-chunk world of air but for a lone solid voxel at `(4, 3, 0)`, the ray
    from `(0.5, 0.5, 0.5)` along the normalized `(1, 1, 0)` hits that voxel at the same `t`
    through the same face with every skip on and off (the skips cross the empty 4³ block
    `[0, 4)²`, whose exit is a corner tie, and the 2³ block `[4, 6) × [2, 4)`; the voxel walk
    crosses a tie at every lattice corner on its way). With the lone voxel at `(3, 4, 0)`
    instead, the two walks again agree (both hit it, or both miss it and leave the window).
  - **The goldens re-bless once, and only at isolated pixels** (amended 2026-10-19). After
    the plane-order exit lands, every PNG under `apps/flythrough/tests/golden/` is re-blessed
    on lavapipe (`VXE_BLESS=1`), and each re-blessed image differs from its file at 106998d
    at isolated pixels only: no two differing pixels adjacent, 8-neighbourhood, and at most 8
    differing pixels per image (dev's experiment read 1 or 2 per image before the rule). The
    log lists each image's differing pixel coordinates and largest channel difference; the
    close review re-derives them with `git show 106998d:<path>`. From this re-bless on, the
    plan's remaining golden bullets are bit-exact against these files, and `lod.png` joins
    the strict byte comparison against its file (today only `primary.png` and `headlamp.png`
    are compared byte for byte in `golden.rs`; `lod.png` goes through `assert_golden`'s
    0.1 % tolerance, so "bit-exact" for it has rested on a re-bless leaving git clean).

### Phase 4: A walk that re-derives chunk and brick state only at their boundaries
- **Owner skill:** dev
- **What:** `march` keeps the current chunk's record base, level and the current brick's
  slot across steps, re-reading the table only on entering a chunk and the record entry
  only on entering a brick, so that per ray segment each level costs one read. It also holds
  the bits words it last read with their word index, so testing a 16³ group or a brick whose
  bits lie in a word already held costs no read; entering a chunk drops them. The steps
  view's green channel becomes total storage loads issued (table, record bits and entries,
  mask, pool, palette), each 32-bit load counted once.
- **Files touched:** `crates/vxe-render/src/shaders/march.wgsl`,
  `crates/vxe-render/tests/mirror.rs` or a new `crates/vxe-render/tests/walk.rs` (fixed-ray
  read counts), `docs/nfr.md` §2.
- **Done when:**
  - On the Phase 1 four-chunk closure world, the +x ray reads exactly 4 table entries (the
    three air chunks are skipped on their `EMPTY_BIT`), at most 2 bits words and exactly 1
    record entry, all in the rock chunk, and 0 mask, pool or palette words: at most 7 reads.
    (The ray enters at brick `(0, 4, 4)`; its 16³ group's 8 bits lie in words 9 and 11.)
  - On a closure world of one chunk that is uniform air except one `Full` brick holding one
    solid voxel at its far corner, a +x ray through that voxel reads: 1 table entry, at
    most 2 bits words (a 16³ group's 8 bits span two words, and along x every group and brick
    of a row of groups lies in the same two), 1 record entry, mask words all from that
    brick's one 64-byte opaque line (at most 8), 1 pool word, 1 palette word. A +x ray
    through the same chunk that misses the brick, through the same row of 16³ groups, reads
    1 table entry and at most 2 bits words.
  - The goldens match bit-exactly on lavapipe (the Phase 3 files), and Phase 3's identity of
    the walk with every skip on and off still holds: the state held across steps changes
    which words are read, never which planes are crossed (amended 2026-10-19).
  - March-pass readings (median of the last 30 of 300 frames, three runs, timestamp
    queries, dev box, quiet) for: `--level --seed 1 --render 1920x1080` from the spawn;
    `--seed 1 --render 1920x1080 --frames 3000` from the streamed spawn (full residency
    takes about 1500 frames, so the median is of the last 30 of 3000); and the
    autopilot's p50 and p99 (`--autopilot --headless`). Each is recorded in the log beside
    Phase 1's reading of the same view, and the histogram command's mean, p99 and block-max
    mean beside Phase 1's and Phase 2's.

### Phase 5: The translucent class and the bounded accumulation walk
- **Owner skill:** dev
- **What:** a material may be translucent (`MaterialFlags::TRANSLUCENT`, `Material::opacity`
  in `[0, 1]`); the upload leaves translucent voxels out of the opaque mask and sets their
  2³ bits in the slot's two translucent words (the group's ninth line); after the opaque
  hit, a second walk from the ray
  origin to the hit visits translucent 2³ blocks, reads their voxels and composites up to
  `translucent_layers` voxels front to back, each `albedo × opacity` under the headlamp's
  cone and distance term with no shadow ray; an `albedo` debug view composites unlit.
- **Files touched:** `crates/vxe-world/src/material.rs` (`TRANSLUCENT`, `opacity`; the
  table refuses to change a sealed row's class), `crates/vxe-render/src/gpu_world.rs`
  (class split at upload; `opacity` in the material row's spare float), `marcher.rs`
  (`LookParams::translucent_layers`, default 4; `ViewMode::Albedo`), `march.wgsl`,
  `crates/vxe-render/tests/translucent.rs` (new), `apps/flythrough/src/main.rs`
  (`--translucent-layers N`, `--view albedo`), `apps/flythrough/tests/golden/translucent.png`
  (new, lavapipe), `README.md`.
- **Done when:**
  - A closure world of one chunk: a one-voxel-thick wall of material 2 (`TRANSLUCENT`,
    opacity 0.5, albedo `[1, 0, 0]`) at `x = 20`, and a wall of material 1 (opaque, albedo
    `[0.5, 0.5, 0.5]`) at `x = 40`. A +x ray from `x = 10` in `VIEW_ALBEDO` reads
    `[0.75, 0.25, 0.25]` within `1 / 255` per channel (`0.5 × red + 0.5 × grey`), and in
    `VIEW_HITMASK` reads a hit (the opaque wall). With `translucent_layers = 0` the albedo
    view reads the grey wall alone. With two red walls and `translucent_layers = 1` only the
    nearer one is composited: `[0.75, 0.25, 0.25]`; with `2`, `[0.875, 0.125, 0.125]`.
  - The opaque walk's reads on the ray through one red wall equal its reads on the same
    world without the wall: translucent voxels cost the opaque walk nothing.
  - `golden/translucent.png` (the closure scene from a fixed pose, shaded) matches
    bit-exactly on lavapipe; the three existing goldens are unchanged (no translucent
    material in the level).
  - A material's opacity class is fixed for the world's life (amended 2026-10-19: there is no
    `seal` and no `World::replace_material`, so the bullet as first written was
    unsatisfiable; the owner's call, 2026-10-18). `World::materials` hands out a shared
    reference and nothing in `vxe-world`'s public API reaches the table mutably after
    `generate`, pinned by a `compile_fail` doc test on `world.materials().push(..)` or an
    equivalent API test; the `.vox` material map in `vxe-assets` accepts translucent rows like
    any other. ADR-0012's "the engine refuses to change the class after the table is sealed"
    is met by there being no path to change it.
  - Phase 3's identity of the walk with every skip on and off still holds on the three opaque
    golden scenes with `translucent_layers` at its default: the second walk runs only when a
    translucent bit was set along the ray (amended 2026-10-19).
  - `apps/flythrough/tests/determinism.rs` hashes are unchanged (no `vxe-world` data
    changed; the flag is a table property).

### Phase 6: Readings, budgets and the stats line
- **Owner skill:** dev
- **What:** the readings `nfr.md` §2 and §3 need, the mean steps per ray on the HUD's text
  grid, and the `README.md` and roadmap notes.
- **Files touched:** `docs/nfr.md` (§2 primary rays: the three Phase 4 views at the tip; §3:
  mask and record bytes at the level and at the streamed defaults), `crates/vxe-render/src/hud.rs`
  and `apps/flythrough/src/main.rs` (a `steps` line when the steps view is on), `README.md`.
- **Done when:**
  - `nfr.md` §2 holds the Phase 4 readings, re-taken at the tip with Phase 4's commands (the
    streamed spawn at `--frames 3000`), with machine, commit, command and load, and §3
    holds live and allocated mirror bytes at the level and at the streamed defaults
    (`--autopilot --headless`, peak), against Plan 0002's 1 672 823 572 B live and
    2 431 425 836 B allocated.
  - With `--hud --view steps` the HUD prints mean steps per ray of the last frame.

## Data shapes

```text
// illustrative: the chunk table entry (u32)
0xFFFFFFFF  ABSENT, as today
bits 0..29  record index
bit 29      EMPTY_BIT: every entry of the record is uniform air (bits words all zero)
bit 30      LOD_BIT, as today
bit 31      clear unless ABSENT

// illustrative: the chunk record (u32 words)
0..16     brick bits: bit b of word b/32 set when brick b is not Uniform(AIR)
16..528   brick entries as today (FULL_BIT | WIDE_BIT | slot, or the uniform material)
528..656  palette: 256 u16 pairs, index 0 air

// illustrative: the mask buffer (u32 words), eight pool slots per 144-word group; slot s
// is slot i = s % 8 of group g = s / 8
144 g + 16 i .. + 16   opaque bits of slot s, one 64-byte line: word 2z + (y >= 4), bit
                       x + 8 (y % 4); set when the voxel's material is non-air and not
                       TRANSLUCENT
144 g + 128 + 2 i ..   translucent 2^3 bits of slot s, two words in the group's ninth line,
          + 2          the layout of today's occupancy mask: bit bx + 4 by + 16 bz set when
                       block (bx, by, bz) holds a TRANSLUCENT voxel
```

```rust
// illustrative
pub struct Material {
    // ... as today
    /// Alpha of a translucent material's voxel in [0, 1]; ignored without TRANSLUCENT.
    pub opacity: f32,
}
pub struct LookParams {
    // ... as today
    /// Translucent voxels composited in front of the opaque hit, nearest first; 0 ignores them.
    pub translucent_layers: u32,
}
```

The 4³ test on the opaque line: for a block at `(bx, by, bz)` in `0..2`, the four words
`2 (4 bz + k) + by` for `k` in `0..4`, each masked with `0x0F0F0F0F << (4 bx)`. The 2³ test
for a block at `(cx, cy, cz)` in `0..4`: the two words `2 (2 cz + k) + (cy >= 2)` for `k` in
`0..2`, masked with `0x0303 << (2 cx + 16 (cy % 2))`.

## Budgets

- **§2 primary rays.** The streamed spawn (6.69 ms, dev box) is the figure this plan is
  held to; the goal is the 3.0 ms line on the dev box, a guess until Phase 1's histogram
  says how many steps the view spends in air. The level spawn (1.59 ms) is hit-bound and is
  expected to move little. Measured at Phase 1 (before), Phase 4 and again at Phase 6,
  three runs each, and once at Phase 2.
  Amended 2026-10-18 from Phase 1's readings (6.81 ms streamed, 1.86 ms level; mean steps
  15.55 against 10.48, p99 84 against 15): step count explains a minority of the 3.7 times
  gap, so the 3.0 ms goal is a guess with weaker footing than when it was set, and the plan
  expects most of any gain from Phase 4's reads per step, not Phase 2's skips. The phase
  order stays: Phase 4's single-read walk needs Phase 2's bits and Phase 3's masks to be the
  levels it reads, and Phases 2 and 3 are small. Phase 2's march and block-max readings say
  how much of the gap the skips take; if Phase 4 leaves the streamed spawn above 3.0 ms, the
  close records it as a miss with the per-step figures beside it, and the remaining gap goes
  to the slot plan's smaller working set (ADR-0012) and the lighting plan's kernel split, not
  to a new phase here.
- **§2 streaming line.** A `Full` brick's upload grows from 520 to 584 B (+12 %), and a
  record from 2560 to 2624 B. The 2 MiB sync budget uploads about 11 % fewer bricks per
  frame; `--autopilot`'s residency latency (p95 455 to 481 ms against 500) is re-read at
  Phase 6.
- **§3 world VRAM.** Masks: 72 B against 8 B per slot (576 B per group of eight; the
  grouping adds no bytes over a flat 72-byte slot). At the level, 27.6 MB against 3.1 MB.
  At the streamed defaults, about 3.1 M slots (855.1 + 673.9 MiB of pool at 520 B a slot),
  so about 221 MB of masks against 25 MB, and 3.7 MB more of records: about +200 MB live,
  and with ADR-0004's 1.5× growth up to +300 MB allocated, against Plan 0002's 2.43 GB peak
  allocated and the 3 GB line. Under the line without attribute pages; with them the mirror
  is over the line already (Plan 0013 and ADR-0012's slot plan own that).
- **The portable profile.** No reading: the hardware is not on hand. The plan's read
  counts per ray (Phase 4) are the figure a portable reading is later compared against.

## Risks & open questions

- **Binding count.** The marcher binds exactly 8 storage buffers; this plan adds none by
  packing bits into the record and the masks into the grouped mask slots. A later binding
  (the lighting plan's history) needs a packed binding or a raised limit, as ADR-0004's
  Outcome notes.
- **lavapipe's 128 MiB binding.** The level's masks (27.6 MB) and records (8.4 MB) fit;
  the streamed defaults already exceed lavapipe's record cap (ADR-0004 Outcome) and are
  read on the dev box only.
- **Step counts across adapters.** The fixed-ray tests use axis-aligned rays from voxel
  centres so that every `t` is exact; the goldens stay the only cross-adapter image check.
  The plane-order rule makes the image independent of the skips on one adapter; across
  adapters `(B - o) inv` can still round differently, so the goldens stay lavapipe-only
  (amended 2026-10-19).
- **The plane-order correction costs ALU per step** (amended 2026-10-19): two plane
  crossings per off-axis, a handful of multiplies and compares against a walk whose cost is
  dependent memory reads. If Phase 4's level-spawn march time (hit-bound, few steps) moves
  against Phase 2's reading, the log records it beside the step histogram; the identity test
  is not negotiable, the shape of the correction is.
- **Register pressure.** Phase 4 keeps more state per ray; if the occupancy drops and the
  march slows on the level view, the log records both and the review decides which state
  to drop. The march loop today also carries the contour test, the detail noise and the
  shading, and the shadow ray is a second inlined march; separating the march from the
  shading (the lighting plan, see below) is the structural remedy, not this plan's.
- **Translucent voxels and the CPU.** `raycast`, the coarse grid, sweeps and collapse treat
  translucent voxels as solid, as today; a passable class for water is a later decision,
  and the plan's test pins that the hashes do not move.
- **Determinism.** Nothing in `vxe-world`'s data changes; the flag and opacity are table
  properties. The render tests are headless on lavapipe.

## What this plan does NOT do

- The CPU brick format, the edit kernel and the size-classed mirror slots (ADR-0012's two
  following plans).
- The contour attribute's footprint (Plan 0013, then the slot plan).
- Screen-door transparency, lighting of translucent layers beyond the headlamp tint,
  refraction and reflection (the lighting plan).
- Gas, mist and smoke as a coarse volume (M8).
- Mip-level selection by projected size and the far-field clipmap (the far-field plan).
- Any `vxe-world` reader change for translucency (collision, coarse grid, collapse).
- The split of the march into a lean hit pass and a separate shading pass, and a coarse
  walk for secondary rays that stops at the 2³ level: both are the lighting plan's design
  inputs (`docs/roadmap.md`, M2). This plan keeps the 2³ bits on the opaque line so the
  coarse walk costs no storage when it comes.
- Beam optimization (a conservative start distance per pixel block from a coarse pre-pass,
  Laine & Karras 2010 section 4.2): a followup, decided from this plan's histograms.

## Implementation log

> Written by `dev`: one row per phase as its commit lands, then the close block. **The phases
> above are the contract. This section records what happened.** Observations, never
> conclusions: no pass list, no self-assessment. Deviations and unmet done-whens are
> **always** disclosed. Keep it shorter than `## Implementation phases`.

| phase | owner | state | commit |
|---|---|---|---|
| 1: The steps view and the headless histogram, on today's walk | dev | done | 52d6a7b |
| 2: Chunk bits, the chunk-level and 16³ skips | dev | done | 106998d |
| 3: Full-resolution opaque masks | dev | not started | |
| 4: A walk that re-derives state at boundaries | dev | not started | |
| 5: The translucent class and the bounded accumulation walk | dev | not started | |
| 6: Readings, budgets and the stats line | dev | not started | |

> **Resume note (2026-10-18, delete when Phase 3 lands; answered by the 2026-10-19
> amendment above):** stopped inside Phase 3 at 106998d, routed to `/architect` (owner's
> call). Phase 3's grouped masks, `solid_bits()`, the
> 4³ / 2³ / voxel tests and its tests are written and uncommitted in the main checkout
> (`gpu_world.rs`, `march.wgsl`, `brick.rs`, `mirror.rs`, `walk.rs`, `level_mirror.rs`,
> `look.rs`); every Phase 3 bullet holds except bit-exact goldens. Diagnosis: the 4³ skip
> moves isolated pixels in every golden (`primary.png` and `headlamp.png` 2 pixels each,
> (167, 43) and (198, 150); `lod.png`, `hud.png` (167, 43); `look_detail.png` 2), largest
> channel differences 7 to 84 of 255. With the 4³ test disabled and the 2³ and voxel tests kept,
> every golden re-blesses unchanged. A cell's exit puts the off-axis coordinates from the
> float position, so a larger cell that leaves through a different plane near a corner tie
> lands one voxel off where a smaller cell would not; Phase 2's 64³ and 16³ skips moved no
> golden pixel on these scenes. Options raised: re-bless at Phase 3 with an isolated-pixel
> test against the old goldens and log the bullet unmet as stated; drop the 4³ skip (the
> "at most 4 steps inside the brick" bullet becomes 5); amend the plan. The plan's readings
> (Phase 1 block-max at 52d6a7b plus the readback change, Phase 2's histograms, block-max and
> march) are not taken yet: the dev box was under other load. Gate at 106998d: build,
> nextest (372 passed, 4 skipped), doctests, clippy, fmt clean.

### Notes

- Phase 1: `FLAG_ATTR_PAGES` moved from bit 7 to bit 8 so the view field can take bits 5..8.
- Phase 1: the streamed spawn reaches full residency after about 15 s (about 1500 frames), so
  its readings are taken with `--frames 3000`; at `--frames 60` and `--frames 300` the
  mirror is not fully resident (640 860 404 B live at 300). The 60-frame histogram read the
  same as the 3000-frame one.
- Phase 1: bit-exactness of the goldens is checked by re-blessing them on lavapipe
  (`VXE_BLESS=1`) and finding the PNGs unchanged in git; `assert_golden` itself tolerates
  0.1 % of pixels off by more than 2/255.
- Phase 2: `crates/vxe-render/src/shaders/attrib.wgsl` (outside the phase's files) reads the
  new record layout and table bits too, and skips an `EMPTY_BIT` chunk as air;
  `apps/flythrough/tests/look.rs` (outside the phase's files) has its record bytes moved from
  2560 to 2624.
- Phase 2: the 16³ group test runs at level 0 and at mip levels 1 (one bits word) and 2 (the
  level's 8 entries); level 3 has no group. `StepHistogram::from_rgba8` takes the row width.
- Phase 2: the Phase 2 readings (histograms, block-max and march) and Phase 1's block-max are
  not in this commit: the dev box was under load from other work, so every reading of the plan
  is taken after Phase 6, serially, from each phase's commit, and recorded in the close
  triggers.
- Phase 2: `apps/flythrough/examples/world_budget.rs` still counts a 2560-byte record and an
  8-byte mask.

### Close triggers

_(Facts for the architect. No recommendations, and no suggested version bump.)_

- **What shipped:**
- **Public API changed:**
- **Budget readings:** RTX 3080 Laptop GPU, Vulkan, headless, 1920 x 1080, seed 1,
  timestamps, median of the last 30 frames, three runs each; load average 5 to 11 from the
  runs' own streaming workers.
  - Phase 1 (before, 80c6ab0 plus the steps view): streamed spawn `--frames 3000` march
    6.817 / 6.815 / 6.810 ms; level spawn `--level --frames 300` 1.859 / 1.875 / 1.858 ms.
    Histogram `--view steps`: streamed spawn mean 15.55, p99 84 steps, pool reads mean 2.45,
    p99 7 (same at `--frames 60` and `3000`); level spawn mean 10.48, p99 15, pool reads mean
    2.15, p99 4. Steps-view march (no shadow ray): streamed 6.021 ms, level 1.483 ms.
- **Gate at the tip:**
- **Outstanding `human` phases:**

## Followups

- **Beam optimization**, if the Phase 4 histogram still shows the streamed spawn spending
  most of its steps in air: one conservative distance ray per 8 × 8 pixel block through the
  bit hierarchy with contours off, and each pixel's ray started at its block's minimum less a
  margin (Laine & Karras 2010, section 4.2; their Table 2 reads 3 to 27 % more primary rays
  per second from it, so it is a trim, not a rescue). A small plan of its own; the Phase 1
  view is its measurement.
