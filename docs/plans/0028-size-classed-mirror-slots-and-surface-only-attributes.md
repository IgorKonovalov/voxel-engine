# 0028: Size-classed mirror slots: the packed brick's own bytes in the pool, no chunk palette, and contour attributes for surface voxels only

> **Status:** approved (2026-10-21)
> **Created:** 2026-10-21
> **Milestone:** M2 in `docs/roadmap.md` (ADR-0012's third and last step; the contour
> footprint ADR-0011 and Plan 0013 owe; `nfr.md` §3)
> **Related ADRs:** [ADR-0012](../adrs/0012-mask-first-brick-with-per-brick-palette-planes.md)
> (the decision this plan finishes: the same bytes on the CPU and in the mirror, size classes
> of 64-byte units, surface-only attributes by popcount),
> [ADR-0013](../adrs/0013-mirror-reserves-its-vram-budget-up-front.md) (the reservation, page
> re-kinding and the pressure rule, which carry over to bins),
> [ADR-0011](../adrs/0011-contour-planes-in-attribute-pages-with-procedural-detail.md) (the
> attribute's encoding and `NO_PLANE`, unchanged; its page layout goes),
> [ADR-0004](../adrs/0004-palette-narrowed-sharded-brick-pool.md) (the shards and bindings,
> which stand; its per-chunk palette goes, as ADR-0012 says)
> **Source:** ADR-0012's Decision and Outcome; Plan 0013's Followups (the footprint), Plan
> 0017's Followups (page fragmentation), Plan 0023's Followups (the attribute pass's GPU cost,
> `attr_cap`, the windowed saturation at 40 fps); Plan 0013 Phase 3's plane-voxel census.
> **Starts after:** Plan 0024 closes (it changes what the march writes; this plan changes
> what it reads, and rebases on the hit pass) and Plan 0027 (the belt, which this plan's
> `upload_brick` writes through).

## TL;DR

A `Full` brick's mirror slot becomes its packed CPU bytes: the opaque mask, a header with the
brick's own palette, and its bit planes, in a size class of 64-byte units named by the record
entry. The per-chunk palette, the wide-record path and the separate mask buffer go. The
contour attribute is stored for the brick's surface voxels only, addressed by a popcount of a
surface mask, so the default look holds `r_full` 16 under the 3 GB line without the pressure
rule lowering it. The pool is allocated in bins of unit sizes over ADR-0013's reservation and
pages. The first thing a reader sees is the level's mirror falling from 232 MB to about 63 MB
with every golden byte for byte, then the streamed default look settling at `r_full` 16 with
pressure under the restore line.

## Context & problem

Three recorded facts, all in `nfr.md` §3. The default look (contour) fits the 3 GB line only
by the pressure rule: at 16/24 it settles at `r_full` 12 to 13 (76.8 to 83.2 m) against the
section's "about 100 m", with 2.86 GB live, because each `Full` brick of a full chunk costs
512 B of indices, 72 B of masks and 1168 B of attribute slots. In the owner's windowed flight
(Plan 0023 Phase 7) the frame rate fell to about 40 fps on fast passes with the mirror at its
budget and deferral streaks up to 2015 syncs. With attribute pages the pool defers chunks
under the 0.9 trigger because plain and attribute slots sit spare in each other's pages (Plan
0017's fragmentation followup).

What is already in hand. ADR-0012's CPU format (Plan 0021) holds a `Full` brick as a 64-byte
mask, a palette in ascending order and bit planes in classes of 72, 136, 200, 352 or 1024 B;
the mirror still narrows those bytes into 512 one-byte indices through a per-chunk palette on
every upload, which is 34 % of `sync`'s CPU (Plan 0023's profile) and the largest share once
Plan 0027 removes the copies. Plan 0013's census at 16/24 reads 1 640 292 `Full` bricks in
full chunks with 57 703 350 plane voxels, 35.2 per brick, 6.87 % of their voxels: 44 % of the
bricks have under 32, 38 % under 64, 18 % under 96, and 177 bricks more. The attribute pass
already writes `NO_PLANE` for a voxel whose six face neighbours are solid, so the set of
voxels that can carry a plane is exactly the set of solid voxels with an air face neighbour.
Plan 0021's census of the level reads 383 608 `Full` bricks: 275 466 with one material,
107 325 with two, 817 with three or four.

The arithmetic this plan rests on (64-byte units; the layout is in Data shapes). A one-material
brick is 2 units (128 B), two materials 3 (192 B), three or four 4 (256 B), five to sixteen 6
(384 B), more 18 (1152 B). The level's pool is 275 466 x 128 + 107 325 x 192 + 817 x 256 =
56 075 200 B, its records 3200 x 2112 = 6 758 400 B: about 63 MB against 232 436 992 B today.
Surface attributes add one unit of surface mask and `ceil(2 n / 64)` units of `u16` attributes
for `n` surface voxels: at the census's distribution about 1.75 units, 176 B per brick with
the mask unit, against 1168 B today. At 16/24 that is 1 640 292 x (about 147 B of brick plus
176 B of attributes) plus 1 158 516 mip-bricks x 147 B plus 93 239 records x 2112 B, about
0.93 GB, against 2.86 GB at `r_full` 13 today; the record term of the pressure (0.62 to 0.65
at full residency) then exceeds the unit term (about 0.3), and both are under the 0.7 restore
line, so the rule never lowers the radius. The chunk-face rule below inflates the surface
count on chunk-boundary bricks; Phase 1 measures by how much, and 2 x 176 B still leaves the
total near 1.2 GB.

The consumer requirement is `project-context.md`'s Rendering row (stable at arm's length, the
smooth look) at the World row's scale. The `nfr.md` figures touched are §3 (world VRAM, the
reservation and the pressure rule), §2 (primary rays and the attribute pass) and the streaming
line (narrowing).

## Decision

**The mirror stores the packed brick** (ADR-0012). A `Full` brick's slot is a run of 64-byte
units in a shard: unit 0 the opaque mask, unit 1 the header (the brick's palette of up to 16
`u16`, the two translucent `2^3` words, a per-layer prefix of the surface mask's popcounts, the
attribute unit count and flags), then its plane units (0, 1, 2 or 4; a brick over 16 materials
stores 16 units of raw `u16` ids and no planes), then, for a full chunk's brick when surface
attributes are on, one unit of surface mask and the attribute units. The record entry names
the unit index and the class; the chunk record keeps its brick bits and 512 entries and loses
its palette. The separate masks buffer goes: the hierarchy walk reads unit 0 as the mask line
it reads today, and the hit reads the header and the plane words at the voxel's bit. The
opaque mask is the CPU mask less the translucent entries' voxels; the planes and palette are
the CPU words copied, so `upload_brick` narrows nothing. Slots are allocated in **bins** of
unit counts (2, 3, 4, 6, 8, 12, 16, 20, 24 and 36 units) over 4096-unit pages that take a bin
when first issued and re-bin when empty, exactly ADR-0013's page kinds generalised; the
reservation, the nearest-first partial sync, the deferral rules and the pressure rule stand,
with pressure's slot term read on units. A brick whose class or attribute count changes moves
to a slot of its new bin in the same sync, and its entry is rewritten with it. The **surface
mask** is computed on the CPU at upload as the brick's solid voxels with a non-solid face
neighbour inside the same chunk, plus its solid voxels on a face of the chunk (the neighbour
chunk is not read, so no cross-chunk coupling); an upload of a dirty brick also re-uploads
its face neighbours within the chunk, whose surface masks may have changed. The attribute
pass writes a surface voxel's attribute at its popcount index and computes nothing for the
others.

We rejected a general offset allocator over units (an `offset-allocator`-style two-level
bitmask: first fit with no bins) because ADR-0013's fit replay, pressure term and page
re-kinding are written against pages of one kind, and bins keep every one of those tests;
compaction, if fragmentation measures badly, is a followup either way. We rejected reading
neighbour chunks for an exact surface mask because an edit in one chunk would then re-upload
its neighbours' boundary bricks, a coupling the conservative chunk-face rule avoids at a
measured cost. We rejected keeping the masks buffer beside the slots (the mask line is one
read in either place, and the binding is worth more free).

## Architecture diagram

```mermaid
flowchart LR
    subgraph world [vxe-world]
        P[Packed brick\nmask, palette, planes]
    end
    subgraph render [vxe-render]
        U[upload_brick\nopaque mask, header,\nplanes copied, surface mask] --> A[bin allocator\npages per bin\nADR-0013 rules]
        A --> S[(shards: unit runs\nmask | header | planes | surface | attrs)]
        R[(records: bits + entries\nclass, unit index)]
        S --> M[march.wgsl\nmask line at the walk,\nheader + planes at the hit,\nattr by popcount]
        R --> M
        S --> T[attrib.wgsl\nsurface bits only]
    end
    P --> U
    U --> R
```

## Implementation phases

Each phase ships as its own commit. `dev` implements all phases in one session with no review
between phases. The architect reviews once at the end, in a fresh session. Every golden this
plan touches must render byte for byte; a re-bless is a deviation to disclose, not a step.

### Phase 1: The census of units
- **Owner skill:** dev
- **What:** `world_budget --surface` (and `--census`) also prints, per brick class, the unit
  count of this plan's layout, the exact plane-voxel count (today's), this plan's
  chunk-local surface count (face neighbours inside the chunk, plus chunk-face solid
  voxels), the attribute units each implies, and the projected mirror bytes (pool by bin,
  records, table) for the level and for the streamed residencies 12/24, 14/24 and 16/24,
  in both looks.
- **Files touched:** `apps/flythrough/examples/world_budget.rs`, `crates/vxe-world/src/census.rs`,
  `docs/nfr.md` §3 (a projection row, labelled arithmetic).
- **Done when:** the example prints the table; the level's projected plain pool reads
  56 075 200 B (the arithmetic above from Plan 0021's census) and its records 6 758 400 B;
  the 16/24 exact plane-voxel count reads 57 703 350 (Plan 0013's figure, so the census is
  the same census); the chunk-local surface count and its ratio to the exact count are in
  the log, and the Phase 4 bars below are confirmed or amended from it in the log before
  Phase 2 starts (an amendment is a note, not a rewrite).

### Phase 2: The slot format, the bins and the march, plain look
- **Owner skill:** dev
- **What:** the entry, header, unit and bin constants; `ShardLayout` in units; the allocator
  (per-bin cursors and free lists over pages, `Fit` replay per bin, page re-binning,
  pressure on units); `upload_brick` writes unit 0 from the mask less translucent voxels,
  the header, and the plane or raw-id units copied from `Packed::words`; records lose the
  palette (`RECORD_WORDS` 528); `turns_wide`, `pair_room`, `alloc_pair`, `rev`,
  `first_appearance` and `Mirrored.palette` go; the masks buffer and its binding go;
  `march.wgsl`'s walk reads unit 0 as its mask line and its hit reads the header and planes;
  the translucent walk reads the header's words; the cutaway and the steps view follow.
  `world_budget`, `level_mirror` and the HUD's byte estimates follow the layout.
- **Files touched:** `crates/vxe-render/src/gpu_world.rs`, `crates/vxe-render/src/marcher.rs`,
  `crates/vxe-render/src/shaders/march.wgsl`, `crates/vxe-render/src/lib.rs`,
  `crates/vxe-render/tests/mirror.rs`, `tests/walk.rs`, `tests/translucent.rs`,
  `apps/flythrough/tests/level_mirror.rs`, `apps/flythrough/examples/world_budget.rs`,
  `apps/flythrough/src/hud.rs` (byte rows), `docs/nfr.md`.
- **Done when:** `primary.png`, `headlamp.png`, `lod.png`, `translucent.png`,
  `cutaway_level.png` and `hud.png` render byte for byte, and Plan 0015's byte-for-byte test
  of every skip against the single-voxel walk still holds; a staged-bytes test reads a
  two-material opaque brick's slot back and finds unit 0 equal to the CPU mask, the header's
  palette equal to `Packed::palette`, and its plane unit equal to `Packed::words`'s plane; the
  same brick with one material swapped for the translucent glass row finds unit 0 clear at
  the glass voxels and the header's `2^3` words set at their blocks; a brick of 17 materials
  reads its `u16` ids back; a carve that adds a third material to a two-material brick moves
  it from the 3-unit to the 4-unit bin in one sync, with its entry naming the new unit and the
  old units on the 3-unit free list; a page whose last brick leaves joins the free pages and
  is next issued to a different bin on demand; Plan 0017's partial-sync and deferral tests
  pass with bins (nearest first, never part of a chunk, pressure the larger of the unit and
  record terms); `level_mirror` reads under 64 000 000 B live (the arithmetic says
  62 846 400 B plus the table); the level's spawn march reads at most 1.55 ms at 1080p on
  the dev box (quiet, median of the last 30 of 300, three runs; 1.43 to 1.47 ms at 2796062)
  and the plain streamed spawn at `--frames 3000` and `r_full` 16 at most 2.1 ms (1.93 to
  1.98 ms at d3d2f1c); `GpuWorld::sync`'s CPU at the level's first sync is read beside Plan
  0021's about 190 ms.

### Phase 3: Surface-only attributes, contour look
- **Owner skill:** dev
- **What:** the surface mask unit and the attribute units of a full chunk's brick when
  `with_surface_attrs`; the header's prefix popcounts; `upload_record` re-uploads the
  within-chunk face neighbours of every dirty brick; `attr_work` items carry the brick's
  unit index and the pass iterates the surface bits only, writing `NO_PLANE` over the
  attribute units for a clear item; `march.wgsl`'s `contour_attr` reads by prefix plus
  popcount; `attr_slot` and `ATTR_PAGE_BRICK_SLOTS` go.
- **Files touched:** `crates/vxe-render/src/gpu_world.rs`, `attrib.rs`, `attr_work.rs`,
  `crates/vxe-render/src/shaders/attrib.wgsl`, `march.wgsl`, `crates/vxe-render/tests/mirror.rs`,
  `apps/flythrough/tests/look.rs`, `level_mirror.rs`, `docs/nfr.md`.
- **Done when:** `look_contour.png`, `look_detail.png`, `look_gradient27.png` and
  `look_trilinear8.png` render byte for byte and `contour_metrics_at_each_attribute_precision`
  reads 7.41 degrees and 32 px as before; a brick holding a 4 x 4 x 4 cube at voxels 2..6 of
  air has 56 surface voxels, two attribute units, the attribute of voxel (2, 2, 2) at index 0
  and of (5, 5, 5) at index 55, read back from the pool after the pass; a carve of one cube
  voxel re-uploads the brick with 55 surface voxels and the pass's readback matches an
  uncapped mirror's; a brick at a chunk's face whose chunk-face layer is solid counts those
  64 voxels as surface; the level with `--look contour` reads under 180 000 000 B live
  (680 491 136 B today; the arithmetic is 63 MB plus 383 608 x about 176 to 300 B); the
  attribute pass's GPU time under the default autopilot reads p99 at most 1.0 ms (Plan
  0013's proposal; the pass now computes about 35 voxels per brick instead of 512), with
  `attr_cap` 8192 and 4096 both read (Plan 0023's followup) and the carry-over age at each.

### Phase 4: The streamed default at `r_full` 16, and the slow adapter
- **Owner skill:** dev
- **What:** the default and plain autopilot flights at 16/24 on the quiet box, paired with
  the pre-plan binary; the bins' spare units per bin and the deferral counters (fragmentation);
  the fly-through gains `--adapter <substring>` (the first adapter whose name contains it,
  else today's choice) so the same runs are taken on the dev box's AMD iGPU, the slow adapter
  `nfr.md` §1 names from this plan on; the frame summary of Plan 0027 Phase 3 in each run.
- **Files touched:** `apps/flythrough/src/main.rs`, `docs/nfr.md` §1, §2 and §3, the root
  `README.md` (the flag).
- **Done when:** the default look at 16/24 reaches full residency with `r_full` 16, peak
  pressure under 0.7 (so the rule never fires), no deferred chunk, live bytes recorded (the
  arithmetic says about 0.93 to 1.2 GB) and the return hash matching; the plain flight's
  `sync` p50 is at most 2.0 ms (Plan 0027's reading less the narrowing share, which this plan
  removes; the 1.0 ms line stays open if it misses, with the profile's largest share named);
  spare units per bin at full residency total under 2 % of live units; the march at the
  settled default spawn (now `r_full` 16, more full chunks in view than at 13) is recorded
  with no bar, beside plain's; on the iGPU, the level's spawn march and the settled streamed
  spawn's march at 1080p are recorded as the first slow-adapter readings, with no bar.

### Phase 5: The owner's windowed flight
- **Owner skill:** human
- **What:** the default look at 2530 x 1378, sustained fast passes across chunk boundaries,
  the exit summary read; the expected outcome is full detail to 102 m, no deferral streaks
  and a frame-time summary the owner compares with Plan 0023 Phase 7's about 40 fps.
  Amended at Plan 0027's close (2026-10-21): the first measured baseline is Plan 0027 Phase
  4's `frames:` line (frame CPU p50 10.2 ms, `update + poll + sync` p50 4.4 ms, march GPU p50
  6.6 ms, about 30 fps at 2560x1440 with the overlay shown); the 40 fps was read off the
  display. The flight is flown twice, overlay hidden and overlay shown, because a shown
  overlay turns the march timer on and its readback serializes CPU and GPU.
- **Files touched:** this plan's log.
- **Done when:** the owner has recorded the summary and a verdict on the 40 fps saturation
  in the Implementation log.

## Data shapes

```rust
// illustrative: a record entry (u32)
// bit 31 FULL, bit 29 TRANSLUCENT (as today); bits 26..29 class; bits 0..26 unit index
// (3 GiB / 64 B = 50 331 648 units needs 26 bits). A uniform brick keeps its material in
// the low 16 bits with FULL clear.
pub const UNIT_BYTES: u64 = 64;
pub enum Class { P0 = 1, P1 = 2, P2 = 3, P4 = 4, Wide = 5 }   // planes 0, 1, 2, 4, raw ids
pub const BINS: [u32; 10] = [2, 3, 4, 6, 8, 12, 16, 20, 24, 36]; // units per slot
pub const PAGE_UNITS: u32 = 4096;                                // 256 KiB per page

// illustrative: the header unit (64 B)
#[repr(C)]
pub struct Header {
    pub palette: [u16; 16],      // ascending, index 0 is the first non-air material
    pub translucent: [u32; 2],   // 2^3 blocks holding a translucent voxel
    pub surface_prefix: [u16; 8],// surface voxels in layers 0..z, for popcount addressing
    pub attr_units: u16,         // 0 when the brick carries no attributes
    pub flags: u16,              // bit 0 has surface mask
    pub _pad: [u16; 2],
}
// units: [0] opaque mask, [1] header, [2..2+k] planes (k = 0, 1, 2, 4) or 16 raw-id units,
// then [surface mask], then attr_units of u16 attributes, NO_PLANE padded.
// attr index of voxel (x, y, z) = surface_prefix[z] + popcount(surface[z] & ((1 << (x + 8 y)) - 1))
```

The record: 16 words of brick bits and 512 entries, 2112 B (2624 B today). The chunk table and
the material rows are unchanged. Bindings: the masks buffer's binding is freed.

## Budgets

| `nfr.md` figure | line | projection (dev box) | how the close measures it |
|---|---|---|---|
| §3 World in VRAM, live against the reservation | 3 GB | the level 63 MB plain, under 180 MB contour; 16/24 default about 0.93 to 1.2 GB live, pressure peak about 0.65 (records) | `level_mirror`, the Phase 4 autopilot flights |
| §3 the pressure rule | `r_full` restored above 0.7 | never fires at 16/24 | Phase 4 |
| §2 Primary rays | 3.0 ms | level 1.43 to 1.55 ms; plain streamed 1.93 to 2.1 ms; the default at `r_full` 16 recorded | Phase 2 and 4 timings, three runs, quiet box, both adapters |
| Attribute pass GPU (Plan 0013's proposal) | 1.0 ms p99 | under 1.0 ms (35 voxels per brick computed, not 512) | Phase 3 autopilot |
| §2 Streaming line | 1.0 ms | plain `sync` p50 about 2.0 ms (narrowing gone), still a miss | Phase 4, paired with Plan 0027's binary |
| Slow adapter (`nfr.md` §1 from this plan) | none yet | first readings | Phase 4 on the iGPU |

## Risks & open questions

- **Byte-for-byte goldens are the correctness bar**, and the walk's float path must not move:
  the mask line is the same 64 bytes, the hit's material the same id. If any golden differs,
  the phase stops and the diff is a finding.
- **Fragmentation across ten bins** is the memory risk: ten partial pages are 2.5 MiB, but a
  bin whose bricks all leave (a demoted region) holds its pages until empty. Phase 4 reads
  spare units per bin; compaction is the followup if it exceeds 2 %.
- **A brick that changes class on every edit** (a carve that alternates a brick between one
  and two materials) moves slots each time; the move is a unit copy inside the same sync and
  costs no more than a re-upload today.
- **The conservative chunk-face rule** can double the attribute bytes of chunk-boundary bricks
  (a solid chunk face is 64 voxels); Phase 1 measures it and the 180 MB and 1.2 GB bars have
  that room. The exact rule (reading neighbour chunks) is the fallback, with its re-upload
  coupling.
- **Determinism:** nothing in `vxe-world` changes; the census is CPU-only and seeded.
- **Threading:** unchanged; `sync` runs on the render thread through Plan 0027's belt.
- **The hit pass of Plan 0024** reads the material at the hit from the slot; this plan
  rebases that read, not its output.

## What this plan does NOT do

- **An opaque-only brick-bits word in the record** (ADR-0012's Outcome): the testbed has no
  glass; a followup when a scene does.
- **Compaction or a general allocator**: a followup on Phase 4's fragmentation reading.
- **Shrinking the record further** (entries at 16 bits): the record term then leads the
  pressure; a followup if it ever reaches 0.9 first.
- **Narrowing on the stream workers** (ADR-0017's hook): there is nothing left to narrow but
  the surface mask and the translucent split; re-profiled at Phase 4.
- **The far-field clipmap, the residency defaults, light extraction** (Plan 0024 Phase 2
  extracts from the palette it reads; it rebases on the header).

## Implementation log

> Written by `dev`: one row per phase as its commit lands, then the close block. **The phases
> above are the contract. This section records what happened.** Observations, never conclusions:
> no pass list, no self-assessment. Deviations and unmet done-whens are **always** disclosed.
> Keep it shorter than `## Implementation phases`.

| phase | owner | state | commit |
|---|---|---|---|
| 1: The census of units | dev | not started | |
| 2: The slot format, the bins and the march, plain look | dev | not started | |
| 3: Surface-only attributes, contour look | dev | not started | |
| 4: The streamed default at `r_full` 16, and the slow adapter | dev | not started | |
| 5: The owner's windowed flight | human | not started | |

### Notes

_(Deviations from the plan, with the commit, stated without justification. Done-whens not
satisfiable as stated, and what was done instead. Followups noticed and not acted on. One line
each. Empty is fine.)_

### Close triggers

_(Facts for the architect. No recommendations, and no suggested version bump.)_

- **What shipped:** feature / fix-only / docs-chore-only
- **Public API changed:** types, traits, functions, shader interfaces, config (list them, or none)
- **Budget readings:** each `nfr.md` figure touched, with its measured value, machine and commit
- **Gate at the tip:** the commands run (build, nextest, doctests, clippy, fmt, deny), exit codes, test counts
- **Outstanding `human` phases:** which, or none

## Followups

- At close: ADR-0012 gets its final Outcome addendum (the mirror format in; the three plans
  done); ADR-0011's index row drops "footprint via the pressure rule"; ADR-0013's Outcome
  records bins as the page kinds.
