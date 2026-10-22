# 0033: The far field: a clipmap of three coarse chunk levels to about 1 km

> **Status:** approved (2026-10-22)
> **Created:** 2026-10-22
> **Milestone:** M10: Outdoors in [`docs/roadmap.md`](../roadmap.md)
> **Related ADRs:** [ADR-0028](../adrs/0028-far-field-as-a-clipmap-of-source-generated-coarse-chunks.md)
> (proposed with this plan: the clipmap, the source's coarse path, the half rule, the edit
> overlay and the level rule; heightfield, ESVO, impostors and downsampling rejected),
> [ADR-0003](../adrs/0003-sphere-residency-with-lod-ring-and-edit-persistence.md) (sphere
> residency, hysteresis and persistence, which each level reuses; its LOD-only ring stays for
> sources with no coarse path),
> [ADR-0013](../adrs/0013-mirror-reserves-its-vram-budget-up-front.md) (the reservation and
> the pressure rule, which clip chunks draw on),
> [ADR-0012](../adrs/0012-mask-first-brick-with-per-brick-palette-planes.md) (the packed brick
> a clip chunk stores), ADR-0027 (Plan 0032's per-pixel sun shadow ray, which this plan
> continues into the far field),
> [ADR-0024](../adrs/0024-own-temporal-upscaler-on-the-hit-buffer.md) (the upscaler that
> resolves the dithered seam band)
> **Source:** the owner's outdoor answers (2026-10-22: view range about 1 km with the
> far-field clipmap); the roadmap's far-field note and its "Floor, profiles and scene scale"
> decision; ADR-0003's Negative on LOD generation cost.
> **Starts after:** [Plan 0028](done/0028-size-classed-mirror-slots-and-surface-only-attributes.md)
> closes (this plan's byte arithmetic is in its 64-byte units, its 2112-byte record and its
> header; it rewrites what `upload_brick` and the records hold) and Plan 0032 closes (its
> outdoor testbed source is this plan's scene, its directional sample is the shadow ray Phase 6
> extends). Drafted against Plan 0024's `Frame`, hit buffer, `light.wgsl` and `Modes` banner,
> and Plan 0032's `OutdoorSource`, `--source outdoor` and directional sample as drafted; the
> names dev finds in the tree win.
> **Amended:** 2026-10-22 (owner): the scene includes Plan 0032 Phase 8's seeded forest
> (amended into that plan the same day). Phase 2's coarse path carries the trees under the
> half rule, its census counts how many trunks and canopies survive at levels 1 to 3, and a
> headless check pins a canopy-sized block and a thin trunk at level 1 with worked
> arithmetic. Phase 7 adds far-field low-sun readings with the forest on and off on both
> adapters and an observation of whether far tree shadows survive. Phase 8 adds the owner's
> judgement of distant forest shadows at sunset.
> **Amended:** 2026-10-22 (owner, second set): build both readings of the half rule and judge
> them on screen. The level-by-level rule (at least 4 of 8 children) stays the default; the
> by-volume rule (a coarse cell is solid when at least half its full-detail volume is) is a
> selectable mode, `--clip-rule level|volume`, shown on the banner. Phase 1 adds the mode,
> Phase 2's census and Phase 5's overlay check cover both rules, Phase 7's readings add the
> volume rule and leaf fill 64, and Phase 8 picks the default rule.

## TL;DR

Beyond the full-detail sphere the world is drawn from three coarse levels of 20, 40 and 80 cm
cells. Each level is a sphere of 22 *clip chunks* (a clip chunk is a chunk of 64³ cells, so
12.8, 25.6 and 51.2 m on an edge) around the camera, generated directly by the source's new
`coarse_chunk` method, stored as ordinary packed chunks in the mirror's pool, and addressed
through a wrapped table window per level. The marcher switches level by distance from the
camera (at 83 to 90 m, 243 to 256 m and 486 to 512 m at the defaults, a dithered band the
upscaler resolves) and ends at 1024 m; a cell is 1.6 to 2.4 pixels wide where its level starts
at 1080p. Edits fold in from full detail through an overlay of half-rule mips, and the sun's
shadow ray walks the same levels, so a ridge 400 m away shadows the valley at the camera's feet.
The first thing a reader sees is a headless test: a box 150 m away on a ground plane, absent
with the far field off, hit at exactly its face with the far field on.

## Context & problem

The owner wants outdoor scenes seen to about 1 km as a general engine capability (2026-10-22).
Today nothing beyond `r_lod` (24 chunks, 154 m) is drawn, and the roadmap's far-field note
shows why the LOD-only ring cannot simply grow: a 1 km ball is about 30 million chunks, and
the ring generates every chunk at full detail before downsampling. ADR-0028 records the
decision and its rejected alternatives; this plan builds it.

The numbers the plan rests on (arithmetic, `chunks_in_ball` and the disc of the same rule):

- **Pixel footprint.** At 1080p and the fly-through's 1-radian vertical field of view a pixel
  subtends 2 tan(0.5) / 1080 = 1.012 mrad. A level-`k` cell (`0.1 x 2^k` m) is one pixel at
  `98.8 x 2^k` m.
- **Switch distances.** Level `k` takes over at `s_k = (r_(k-1) - 2) x 64 x 2^(k-1)` voxels,
  `r_0 = r_full`, `r_k = r_clip`: the distance inside which every finer chunk is resident (a
  point at distance `D` lies in a chunk whose centre is at most `D / e + sqrt(3)` chunks of edge
  `e` from the focus chunk). With the upscaler on the switch moves closer by up to one
  level-`(k - 1)` chunk, `w_k = 64 x 2^(k-1)` voxels. At `r_full` 16 and `r_clip` 22: s_1 =
  896 voxels (89.6 m, band from 83.2 m), s_2 = 2560 (256 m, from 243.2 m), s_3 = 5120 (512 m,
  from 486.4 m); level 3 is guaranteed to (22 - 2) x 512 = 10 240 voxels, 1024 m, which is the
  far limit. A cell where its level starts is 0.2 / (83.2 x 1.012e-3) = 2.38 px for level 1 and
  1.63 px for levels 2 and 3; 0.77 px at each level's end.
- **Residency.** `chunks_in_ball(22)` = 44 473 clip chunks per level, `chunks_in_ball(23)` =
  50 883 with the hysteresis; the table window is 2 (22 + 1) + 1 = 47 on an edge, 103 823
  entries, 415 292 B per level, 1 245 876 B for three. A one-chunk step of a settled level
  brings in disc(22) = 1517 clip chunks.
- **Generation.** At 10 m/s a level-`k` step comes every `6.4 x 2^k / 10` s, so the three
  leading faces take 1517 x (0.78 + 0.39 + 0.20) = 2074 clip chunks a second, 5.4e8 cells if
  every cell were computed, against the near sphere's 1245 chunks and 3.3e8 voxels a second
  (disc(16) = 797). Outdoors the face is a vertical disc the terrain crosses in a strip about
  44 chunks long and 1.3 deep, about 57 of 1517 clip chunks (3.8 %, a guess for Plan 0032's
  terrain): 78 clip chunks a second need cells, 2.0e7 cells a second, about 0.3 of one worker at
  `nfr.md` §5's 6.95e7 voxels a second under streaming. The other 2000 a second are `Uniform`
  answers from the source's column bounds.
- **Bytes (guess until Phase 2's census).** About 1.3 x disc(23) = 2149 clip chunks per level
  hold the surface. At 96 to 160 `Full` bricks each (the surface's brick layer plus a material
  layer below it) and 192 B a brick (Plan 0028's two-material class), plus a 2112 B record:
  20 to 33 KB a clip chunk, 44 to 71 MB a level, **about 0.13 to 0.21 GB for three levels**
  plus 1.2 MB of tables, against `nfr.md` §3's 3 GB line, which already names "LOD to about
  1 km". RAM: the same clip chunks at 8 KiB of slots plus about 150 B a packed brick, about
  0.2 GB against the 4 GB line; uniform clip chunks are a few bytes.

- **The forest at distance** (amendment 2026-10-22, owner). Plan 0032 Phase 8's trees stand
  one per 48-voxel scatter cell at `--trees forest` (about 94 % kept), with trunks of radius
  2 or 3, canopies of radius 18 to 29 and half their canopy voxels kept by a hash. A level-`k`
  clip chunk's footprint holds about `(64 x 2^k / 48)^2 x 0.94` trees: 6.7, 26.7 and 107 at
  levels 1 to 3. At about 54 400 voxels a mean tree, its exact half-rule cells cost about
  3.7e5, 1.5e6 and 5.8e6 voxel samples a surface clip chunk; at Phase 2's 3.8 % guess of a
  face (45, 22 and 11.5 surface clip chunks a second at 10 m/s) that is about 1.2e8 samples
  a second, some 1.7 workers at `nfr.md` §5's rate, against 0.3 for the bare hills (a guess
  until Phase 2 reads it). Under the half rule as this plan's data shape states it (at least
  4 of 8 children, level by level), a canopy interior at half fill keeps a level-1 cell with
  probability `(70 + 56 + 28 + 8 + 1) / 256 = 163 / 256 = 63.7 %`, a level-2 cell with
  `P(Bin(8, 0.637) >= 4) = 87.8 %` and a level-3 cell with `P(Bin(8, 0.878) >= 4) = 99.9 %`:
  canopies survive and grow denser with distance. A radius-2 trunk keeps 3 level-1 cells per
  2-voxel slab at the alignment Phase 2 pins, and none at level 2 (beyond about 243 m).

- **Two readings of the half rule** (amendment 2026-10-22, owner, second set). The
  *level-by-level* rule above (`MipRule::Half`, the default) reduces each level from the one
  below. The *by-volume* rule (`MipRule::HalfVolume`) keeps a level-`k` cell when at least half
  of its `8^k` full-detail voxels are solid. The two agree at level 1 (one reduction of
  voxels) and differ from level 2. Over a canopy interior at fill `f` (each voxel kept with
  probability `f / 256`, independently by the hash), the share of cells kept at levels 1 to 3
  is:

  | leaf fill | level-by-level | by-volume |
  |---|---|---|
  | 128 (half) | 63.7 %, 87.8 %, 99.9 % | 63.7 %, 55.0 %, 51.8 % |
  | 64 (a quarter) | 11.4 %, 0.80 %, under 0.001 % | 11.4 %, 0.001 %, under 0.001 % |

  (`P(Bin(8, p) >= 4)` chained level by level, against `P(Bin(8^k, f/256) >= 8^k / 2)`; at fill
  128 the by-volume figures are `P(Bin(64, 0.5) >= 32) = 0.5 + C(64, 32) / 2^65` and the same
  at 512.) At fill 128 the level rule thickens far canopies and the volume rule keeps them
  near their real density; at fill 64 both rules thin canopies to almost nothing beyond level
  1 (about 243 m), which Phase 7's readings and Phase 8's flight show. The volume rule costs
  the same voxel reads for an exact tree cell (each voxel is read once either way); Phase 2
  measures both.

The consumer requirement is general (the owner's outdoor capability); `project-context.md`'s
Rendering row sets the look (smooth, not visible cubes). The `nfr.md` figures touched are §2
(primary rays and lighting), §3 (world VRAM and RAM) and §5 (generation throughput, residency
latency).

## Decision

**Three clip levels of clip chunks, generated by the source, mirrored as chunks** (ADR-0028).
`WorldSource::coarse_chunk(at: ClipCoord, out: &mut Chunk) -> Coarse` is a provided method
answering `Unsupported`; a source that implements it fills a level's 64³ cells under the half
rule or answers `Uniform(material)`. `vxe-world` gains `clip.rs`: `ClipCoord`, `ClipChunk`
(`Uniform` or `Cells(Chunk)`), the `ClipResidency` (levels 1 to 3, `r_clip`, hysteresis) the
`Streamer` drives as more tiers on its worker pool, and the edit overlay (`ChunkMips` built
under `MipRule::Half`, kept per edited chunk beside its persistence entry). The `World` holds
resident clip chunks as it holds LOD mips today. `vxe-render`'s mirror gives each level a
wrapped table window at the tail of the chunk table; a `Uniform` clip chunk is a table entry
with no record, a `Cells` clip chunk a record and slots from Plan 0028's bins under ADR-0013's
reservation, with clip records reserved apart from the near ones. `march` gains the level rule
(the larger of the ray's level so far and the level of `|p - camera|`, coarser on an absent
chunk), the dithered band, a per-level bound of non-empty chunks a ray may leave early through,
the level in the hit flags and a far normal mode (face, or a gradient over the hit cell's 3³
neighbourhood); `light.wgsl`'s sun shadow ray starts at its hit's level. With no clip levels
the walk is today's and every existing golden holds its floor.

*Amended 2026-10-22 (owner):* the half rule has two readings, and the plan builds both.
`ClipResidency::rule` is `MipRule::Half` (level by level, the default) or
`MipRule::HalfVolume` (by volume); the streamer passes it to `coarse_chunk` and builds the
overlay under it, so the source's cells and the overlay never disagree on the rule. The
fly-through selects it with `--clip-rule level|volume`, and Phase 8 picks the default.

We rejected a cylinder (vertical extent capped) instead of a sphere per level because uniform
clip chunks cost a table entry and nothing else, and one residency rule is worth more than the
saving. We rejected storing only each level's shell because a level that holds its whole ball
is the fallback for late finer data and the support of the continuous switch. We rejected
switching level on chunk residency alone (the boundary would be the chunk sphere's staircase,
moving in chunk steps) in favour of the distance rule, with residency as the fallback.

## Architecture diagram

```mermaid
flowchart LR
    subgraph world [vxe-world]
        S[WorldSource\ncoarse_chunk] --> W[Streamer\nfull + clip tiers\non workers]
        E[edit result\ntouched chunks] --> O[edit overlay\nhalf-rule mips]
        O --> W
        W --> C[(World: full chunks,\nclip chunks per level)]
    end
    subgraph render [vxe-render]
        C --> M[mirror sync\nnear window + 3 level\ntables, pool, records]
        M --> H[march.wgsl\nlevel rule, band,\nbounds, level flag]
        H --> L[light.wgsl\nsun shadow ray\nfrom the hit level]
    end
```

## Implementation phases

Each phase ships as its own commit. `dev` implements all phases in one session with no review
between phases. The architect reviews once at the end, in a fresh session. Every existing golden
renders under `assert_golden`'s floor (at most 0.1 % of pixels differing by more than 2/255)
with the far field off; a re-bless of an existing golden is a deviation to disclose.

### Phase 1: Walking skeleton: one level, bounded, a box at 150 m
- **Owner skill:** dev
- **What:** `Coarse`, `ClipCoord`, `ClipChunk`, `WorldSource::coarse_chunk` (default
  `Unsupported`), `World::{insert_clip, clip, remove_clip, clip_coords}` and a synchronous
  `World::generate_clip(level, extent, source, threads)` for tests; `MipRule::{AnySolid, Half}`
  in `mips.rs` (`ChunkMips::build` keeps `AnySolid`); the mirror's level-1 table window at the
  tail of `chunk_table` (static, no scrolling yet), uniform table entries, clip records; the
  march's level rule with a hard switch at `s_1 - w_1 / 2` and the level in hit flags bits
  5..7; `UniformSource` answers `Uniform`; a test source `PlaneBoxSource` (voxels with `z < 0`
  material 3, plus axis-aligned boxes) whose coarse path is exact.
- **Files touched:** `crates/vxe-world/src/{source.rs, clip.rs, mips.rs, store.rs, lib.rs}`,
  `crates/vxe-world/tests/clip.rs`, `crates/vxe-render/src/{gpu_world.rs, marcher.rs}`,
  `crates/vxe-render/src/shaders/march.wgsl`, `apps/flythrough/tests/far_field.rs`,
  `apps/flythrough/tests/support/mod.rs`, `apps/flythrough/tests/golden/far_skeleton.png`.
- **Done when:** with the near store generated over chunks [-4, 4) x [-4, 4) x [-1, 1), level 1
  over clip chunks [-4, 16) x [-8, 8) x [-2, 2), `r_full` 4 (so s_1 = 2 x 64 = 128 voxels,
  w_1 = 64, the hard switch at 96) and a box at x in [1500, 1600), y in [-100, 100), z in
  [0, 200) voxels, a camera at (0.5, 0.5, 20.5) looking +x: the hit buffer's ground pixels
  with t < 96 carry level 0 and those with t >= 96 level 1, every ground hit at z = 0 within 1e-3 voxels with normal +z; the pixel
  whose ray passes nearest (1500, 0.5, 20.5) hits x = 1500 within 1e-3 voxels with normal -x
  and level 1; with no clip levels the same pixel misses and no ground hit lies beyond the near
  extent; `far_skeleton.png` is blessed on lavapipe; `half_rule_matches_on_aligned_planes`
  asserts the half and any-solid rules agree on the plane source (every cell boundary of
  levels 1 to 3 is aligned at z = 0); `half_rule_keeps_the_exposed_layer` builds the half-rule
  mips of a chunk that is air at local z >= 32, a material with a higher id in the layer
  z = 31 and a lower-id material below it, and finds every cell of the top solid layer of
  levels 1 to 3 (the cells just under local z = 32) holding the top material (each has four
  top and four lower children, so a plain majority would tie to the lower id and fail); every
  existing golden holds its floor.
  *Amended 2026-10-22 (owner):* `MipRule::HalfVolume` joins `mips.rs` (a level-`k` cell is
  solid when at least `8^k / 2` of its voxels are; its material is the most frequent among
  its exposed solid voxels, else among all its solid voxels, ties to the lowest id), and
  `coarse_chunk`, `generate_clip` and `ChunkMips::build_with` take the rule. `half_rule_matches_on_aligned_planes`
  also finds the volume rule equal to the other two on the plane source, and
  `half_rule_keeps_the_exposed_layer` holds under the volume rule too (a cell of the top solid
  layer is wholly solid, and its only exposed voxels are its `4^k` voxels in the z = 31
  layer, all of the top material, so that material wins).

### Phase 2: The outdoor source's coarse path and the census
- **Owner skill:** dev
- **What:** Plan 0032's `OutdoorSource` implements `coarse_chunk`: a column-height bound over
  the clip chunk's footprint answers `Uniform(air)` above the highest surface and
  `Uniform(rock)` for a clip chunk wholly more than 40 voxels under the lowest (the caverns and
  the tunnel are interiors a viewer outside cannot see, and are left out of the far field);
  otherwise each cell takes occupancy from the surface height at its centre and the surface
  cell of each column takes `grass`, the cells within the soil depth `soil`, in the source's
  integer arithmetic; `world_budget --clip LEVELS,R` prints per level the clip chunks by kind (uniform air, uniform solid, cells), `Full` bricks
  by Plan 0028 class, projected mirror bytes (pool by bin, records, tables), CPU bytes, and
  the coarse generation rate in cells and clip chunks per second per thread; a property test
  compares the source's level-1 cells with the half-rule downsample of its full-detail
  generation over a test region. *Amended 2026-10-22 (owner):* with trees on (Plan 0032 Phase
  8), every cell inside a tree's bounding box takes the half rule over the tree's full-detail
  voxels (`trees.rs`'s per-voxel function), the same reduction `MipRule::Half` applies, so a
  tree downsamples exactly as an edit's overlay would; terrain cells keep the centre-sampled
  rule. A clip chunk that a tree's box reaches is never answered `Uniform(air)`.
  `world_budget --clip LEVELS,R --trees forest` also prints, per level, the trees whose axis
  lies in that level's ring (from its switch distance to the next level's, or to the far
  limit), the share whose trunk keeps at least one `bark` cell under its canopy, the share
  whose canopy keeps at least one `leaf` cell, and the mean ratio of a canopy's solid cell
  volume (`8^k` voxels a cell) to its full-detail `leaf` volume.
  *Amended 2026-10-22 (owner, second set):* the tree cells take the rule the residency names
  (`Half` or `HalfVolume`), and `world_budget` gains `--clip-rule level|volume` and
  `--leaf-fill N`, so the census runs under either rule at any fill.
- **Files touched:** `apps/flythrough/src/outdoor.rs` (Plan 0032's source; the name follows
  that plan), `apps/flythrough/src/trees.rs` (amendment: the coarse path's tree cells),
  `apps/flythrough/examples/world_budget.rs`, `crates/vxe-world/src/census.rs`,
  `crates/vxe-world/tests/clip.rs` (amendment: the canopy and trunk check),
  `apps/flythrough/tests/far_field.rs`, `docs/nfr.md` §3 and §5 (projection rows, labelled
  arithmetic).
- **Done when:** the census prints for levels 1 to 3 at `r_clip` 22 around the outdoor spawn,
  and the plan's guesses (2149 surface clip chunks a level, 96 to 160 `Full` bricks each, 0.13
  to 0.21 GB for three levels, about 0.2 GB of RAM, 3.8 % of a leading face needing cells) are
  confirmed or amended in the log before Phase 3 starts, with the Phase 7 bars they feed (an
  amendment is a note, not a rewrite); `coarse_cells_sit_within_a_cell_of_the_downsample`
  asserts that over 8 x 8 x 4 level-1 clip chunks of the outdoor source, every cell where the
  coarse path and the half-rule downsample differ lies within one cell (26-neighbourhood) of
  a cell where the downsample changes between solid and air, and records the share of cells
  that differ, and the share of exposed cells whose material differs; the generation rate is
  recorded on the dev box with its load.
  *Amended 2026-10-22 (owner):*
  1. **A canopy-sized block and a thin trunk at level 1** (`half_rule_canopy_block_and_trunk`,
     headless, `MipRule::Half` on one chunk of air; level 1 is one reduction of voxels, so
     every reading of the rule agrees there). A solid `leaf` block at local `[8, 32)^3` (24
     voxels, aligned to the 2-voxel cells) gives exactly `12^3 = 1728` level-1 `leaf` cells
     and no other solid cell. The same block at `[9, 33)^3` gives exactly 2057: on each axis
     11 cells are full (`[10, 32)`) and 2 hold one voxel of 2 (`[8, 10)` and `[32, 34)`), a
     cell's solid children number the product of its axes' counts, and the rule keeps a cell
     with at least 4 of 8, so a cell keeps when at most one axis is a boundary: `11^3 + 3 x 2
     x 11^2 = 1331 + 726`. A `bark` trunk of the columns `dx^2 + dy^2 <= 4` around local
     column (32, 32), from z = 0 to 32, gives exactly 48 level-1 cells: the cells (16, 16),
     (15, 16) and (16, 15) hold 4, 3 and 3 of their 4 columns (8, 6 and 6 children), the
     other cells it touches hold one column (2 children), and there are 16 slabs. At level 2
     the same trunk gives no solid cell: each level-2 cell over it holds one solid level-1
     column through its two level-1 slabs, 2 of 8 children.
     *Amended 2026-10-22 (owner, second set): both rules at levels 2 and 3.* The same test
     runs under `Half` and `HalfVolume`. The aligned block `[8, 32)^3` gives 216 level-2 and
     27 level-3 cells under both (`6^3` and `3^3`, every cell full or empty). The block
     `[9, 33)^3` separates them. At level 2 (4-voxel cells) each axis has a cell holding 3
     voxels (`[8, 12)`), 5 full cells (`[12, 32)`) and a cell holding 1 (`[32, 36)`).
     *By volume* a cell keeps when the product of its axes' counts is at least 32: `4 4 4`,
     `3 4 4` and `3 3 4` keep, `3 3 3` (27) and any axis at 1 (at most 16) do not, so
     `5^3 + 3 x 25 + 3 x 5 = 215`. *Level by level*, over the level-1 cells above, a level-2
     cell whose axes are each "boundary pair" (a level-1 boundary cell and a full one) or
     full keeps at least 4 of its 8 children in every mix, and so does one with a single axis
     on `[32, 36)` and the others full (4 of 8), so `125 + 75 + 15 + 1 + 75 = 291`. At level
     3 (8-voxel cells; per axis `[8, 16)` holds 7, `[16, 32)` two full cells, `[32, 40)`
     holds 1) the volume rule keeps `3^3 = 27` (the 1-voxel axis gives at most 64 of 256) and
     the level rule `27 + 3 x 2 x 2 = 39` (a cell with one axis on `[32, 40)` and the others
     full keeps 4 of 8). The trunk gives no level-2 cell under either rule: by volume the
     level-2 cells over it hold 24, 12, 12 and 4 of 64 voxels.
  2. **Tree cells are exact.** `coarse_cells_sit_within_a_cell_of_the_downsample`, run again
     with `--trees forest` over the same region, finds every cell inside a tree's bounding box
     equal to the half-rule downsample at levels 1 to 3. *Amended 2026-10-22 (owner, second
     set):* it runs under each rule against that rule's downsample.
  3. **The forest census** at levels 1 to 3 around the `FOREST` spawn is recorded in the log
     with the bare hills' beside it: the trunk and canopy survival shares, the canopy volume
     ratios against this plan's interior arithmetic (63.7 %, 87.8 % and 99.9 % of a canopy's
     interior cells kept, so at most 1.27, 1.76 and 2.0 times the half-filled leaf volume),
     the forest's clip bytes and the coarse generation rate with trees. These are
     observations; the bars of Phase 7 read the bare hills (below).
     *Amended 2026-10-22 (owner, second set):* the forest census runs four times, under
     `--clip-rule level` and `volume` at `--leaf-fill` 128 and 64, beside the bare hills (one
     run: the hills' cells are centre-sampled, so the rule does not change them). Each run
     records per level the canopy fill (the share of a canopy's interior cells kept, against
     the Context table: 63.7, 87.8 and 99.9 % against 63.7, 55.0 and 51.8 % at fill 128;
     11.4, 0.80 and about 0 % against 11.4, 0.001 and about 0 % at fill 64), the survival
     shares, the clip bytes and the coarse generation rate in cells and clip chunks per
     second per thread, so the two rules' generation cost sits side by side.

### Phase 3: Clip residency on the streamer
- **Owner skill:** dev
- **What:** `Residency` gains `clip: Option<ClipResidency>`; `update` keeps each level's ball
  around the focus's level-`k` chunk with ADR-0003's integer rule and hysteresis, incremental
  on a one-chunk step as the near tier is; jobs order full loads and promotions first, then
  clip chunks outside their finer level's switch distance (the ring) by world distance, then
  the rest (the fallback); `poll` inserts and evicts clip chunks and reports them in
  `StreamEvents` with per-level latency ticks; a source answering `Unsupported` leaves the
  clip tiers empty and says so once in `StreamStats`.
- **Files touched:** `crates/vxe-world/src/{stream.rs, clip.rs, store.rs}`,
  `crates/vxe-world/tests/clip_stream.rs`.
- **Done when:** with `UniformSource`, the plane source and a seeded integer hills test source
  (an exact coarse path), levels 1 to 3, `r_clip` 22, `h` 1: a settled residency holds 44 473 clip chunks per level; a one-chunk step of the focus in
  +x at level 1 queues exactly 1517 level-1 clip chunks and none at levels 2 and 3 until the
  focus crosses their own chunk boundaries; evictions follow `(r_clip + h)^2`; with `r_lod`
  equal to `r_full` no `LodOnly` chunk is ever loaded; the job order test finds every full job
  before every clip job and every ring job before every fallback job; the hash of every hills clip
  chunk after `wait_idle` is equal at 1 and 7 workers and is pinned in the test, blessed on
  Linux (the Windows arm confirms it); a source answering `Unsupported` yields no clip chunk
  and one `StreamStats` flag.

### Phase 4: Three levels in the mirror, scrolling, the band and the far normals
- **Owner skill:** dev
- **What:** the mirror's three wrapped level windows, moved when the focus's level-`k` chunk
  changes (entries leaving freed, entering written absent), uniform entries, clip records
  reserved by `MirrorConfig::clip_records` per level, sync ordered near first, then ring, then
  fallback clip chunks, under the existing partial-sync and deferral rules; the per-level
  bound of non-empty clip chunks (an AABB in voxels, refreshed at sync) through which a ray
  leaves early; the dithered band from the blue-noise texture when the upscaler is on (a hard
  switch at `s_k - w_k / 2` otherwise); the far limit at `(r_clip - 2) x 64 x 8` voxels; the
  far normal mode (`face` or `gradient`) for clip hits; a `levels` view that tints hits by
  level.
- **Files touched:** `crates/vxe-render/src/{gpu_world.rs, marcher.rs, lib.rs}`,
  `crates/vxe-render/src/shaders/march.wgsl`, `crates/vxe-render/tests/mirror.rs`,
  `apps/flythrough/tests/far_field.rs`, `apps/flythrough/tests/golden/{far_field.png,
  far_levels.png}`.
- **Done when:** with the plane source streamed at `r_full` 4, `r_clip` 6, levels 1 to 3 (so
  s_1 = 128, s_2 = 4 x 128 = 512, s_3 = 4 x 256 = 1024 voxels, far limit 4 x 512 = 2048):
  every below-horizon pixel's hit lies at z = 0 within 1e-3 voxels with normal +z, band on and
  off, and its level is 0 for t < 64, 1 for 128 <= t < 384, 2 for 512 <= t < 768 and 3 for
  t >= 1024 (with the band on, either neighbour inside [s_k - w_k, s_k)); no pixel hits beyond
  t = 2048; after the focus moves 20 level-1 chunks in +x and the mirror syncs, the same
  assertions hold from the new pose and a level table entry's readback matches
  `World::clip` for every resident clip chunk; on the plane `z < x / 4` every clip hit lies
  within one cell of its level of the analytic surface, and the gradient mode's mean angular
  error against the analytic normal (14.04 degrees from +z) is below the face mode's, both
  recorded; `far_field.png` (the outdoor source from Plan 0032's spawn, looking at the horizon,
  band off) and `far_levels.png` are blessed on lavapipe; with no clip levels every existing
  golden holds its floor and Plan 0015's skip-equality test still holds.

### Phase 5: Edits fold into the far field
- **Owner skill:** dev
- **What:** `Streamer::note_edits(&[ChunkCoord])` (the application passes each edit result's
  touched chunks) queues a worker job that builds the chunk's half-rule mips under its read
  lock into the overlay; `poll` patches the resident clip chunks of every level whose
  footprint holds it and marks them dirty; `coarse_chunk` results take the overlay before
  insertion; an overlay entry lives while its chunk's persistence entry does (RAM or spill);
  `StreamStats` reports overlay entries and bytes.
- **Files touched:** `crates/vxe-world/src/{clip.rs, stream.rs, persist.rs}`,
  `crates/vxe-world/tests/clip_edits.rs`, `apps/flythrough/src/main.rs` (passes touched
  chunks).
- **Done when:** in a world of `UniformSource(3)` (coarse path `Uniform(3)`), streamed at
  `r_full` 2, `r_clip` 4, `h` 1, levels 1 to 3, a radius-10
  sphere carve at voxel (32, 32, 32) of chunk (0, 0, 0) removes `{3: 4169}` and, after `poll`,
  leaves exactly 492 air cells in level-1 clip chunk (0, 0, 0), 53 in level 2 and 7 in level 3,
  all inside chunk (0, 0, 0)'s footprint (the half rule applied to the carve, level by level);
  after the focus moves until chunk (0, 0, 0) is evicted and level-1 clip chunk (0, 0, 0) is
  evicted and regenerated, the regenerated clip chunk equals the patched one byte for byte,
  with a RAM persistence cap of 0 and a spill directory as well; the clip hash of Phase 3 is
  unchanged by a run with no edits.
  *Amended 2026-10-22 (owner, second set):* the same carve with the residency's rule
  `HalfVolume` leaves exactly 492, 60 and 8 air cells at levels 1 to 3 (a level-`k` cell is
  air when more than half its `8^k` voxels are carved; level 1 agrees with the level rule),
  and the regenerated clip chunk equals the patched one under that rule too.

### Phase 6: The sun's shadow ray walks the far field
- **Owner skill:** dev
- **What:** `light.wgsl`'s directional shadow ray toward the sun or moon (ADR-0027, Plan
  0032) starts at its hit's level with an offset of `SHADOW_OFFSET x 2^k` along the shadow
  normal, takes the level rule by distance from the camera, honours the coarse shadow walk at
  every level, and ends at the far limit. With the far field on, Plan 0032's lighting-ray rules
  move outward: an absent chunk sends the ray one level coarser, only an absent chunk inside the
  coarsest level's ball is an occluder, and leaving that ball or passing the far limit reaches
  the sky. The sky ray keeps its 512-voxel cap. Every other ray's hit and light pass is
  unchanged.
- **Files touched:** `crates/vxe-render/src/shaders/{light.wgsl, march.wgsl}`,
  `crates/vxe-render/src/marcher.rs`, `apps/flythrough/tests/far_field.rs`.
- **Done when:** on the ground plane with a wall at x in [4000, 4200), |y| < 2000, z in [0,
  1000) voxels (aligned to every level's cells; at 400 m it is level 2's at `r_full` 4 and
  `r_clip` 22) and the sun in the +x direction, the ground is shadowed for x > 4000 - 1000 /
  tan(elevation): at 10 degrees from x = -1671, at 30 degrees from x = 2268. The sun term of the
  light pass, from a camera at (0.5, 0.5, 20.5) looking +x and down, reads 0 on every ground
  pixel with hit x in [0, 500) at 10 degrees, non-zero on all of them at 30 degrees, and
  non-zero on all of them at 10 degrees with the far field off; from a camera at (0.5, 0.5,
  1500.5) looking at the wall's top (z = 1000, normal +z, level-2 hits) every top pixel reads a
  non-zero sun term at 10 degrees (no self-shadowing at its level).

### Phase 7: The fly-through's switches, HUD rows and readings on both adapters
- **Owner skill:** dev
- **What:** `--far-field [off|on]`, `--clip-radius`, the outdoor default on; `r_lod` set to
  `r_full` when the far field is on; a key cycling the far field `on`, `levels` view, `off`, and
  a key switching far normals `gradient` and `face`, both through `Modes` and its banner
  (`far field on, 3 levels to 1024 m, normals gradient`); HUD rows for clip chunks resident per
  level against their ball, clip bytes live and the overlay; the outdoor autopilot flight at 10
  m/s with the far field on and off; the readings below on the NVIDIA card and the AMD iGPU.
  *Amended 2026-10-22 (owner):* `--spawn vista` stands over Plan 0032's `FOREST` column, 30 m
  above the ground (above every canopy, whose tops reach at most 11.8 m), facing +y (north)
  with pitch 0, so the forest runs to the horizon under the camera. With `--frames`, an exit
  line (beside the steps line) prints, per hit level 0 to 3, the count of ground pixels (hit
  material `grass` or `soil`) and their mean linear luminance before the tone map.
  *Amended 2026-10-22 (owner, second set):* `--clip-rule level|volume` (default `level`)
  sets `ClipResidency::rule`, and the banner names it (`far field on, 3 levels to 1024 m,
  normals gradient, rule level`). It is a start-up flag, not a key: changing it regenerates
  every clip chunk and the overlay.
- **Files touched:** `apps/flythrough/src/{main.rs, hud.rs}`, `apps/flythrough/tests/hud_golden.rs`,
  the root `README.md` (the flags and keys), `docs/nfr.md` §2, §3 and §5.
- **Done when:** a test presses the far-field key three times and finds `Modes::line()` naming
  `far field levels`, `far field off`, `far field on` in turn with the banner shown from each
  press, and the far-normals key flipping `normals gradient` to `normals face`, and (amended
  2026-10-22, owner) a parse of `--clip-rule volume` naming `rule volume` on the line while
  the default names `rule level`; the outdoor
  autopilot with the far field on reads: clip residency per level at full residency equal to
  its ball, clip live bytes recorded against Phase 2's census (bar: under 0.5 GB, the census's
  figure with room), the near tier's residency latency p95 under 500 ms (`nfr.md` §5, the far
  field must not delay it), each level's ring latency p95 under one of its own steps at 10 m/s
  (1.28, 2.56 and 5.12 s), clip generation in cells per second per worker and the workers'
  busy share, `update + poll + sync` p50 and p99 beside the far-field-off run; at the outdoor
  spawn looking at the horizon at 1080p the march GPU time with the far field on and off
  (bar: at most 3.0 ms on the NVIDIA card, `nfr.md` §2's line; recorded on the iGPU) and the
  light pass with the sun at 45 and 10 degrees elevation, on both adapters, labelled by adapter.
  *Amended 2026-10-22 (owner), the forest under a low sun at distance:*
  1. **Readings on both adapters** (NVIDIA GeForce RTX 3080 Laptop GPU, dev reference; AMD
     Cezanne iGPU on RADV, the slow adapter; quiet box, median of the last 30 of 3000 frames,
     three runs each): `flythrough --source outdoor --headless --seed 1 --render 1920x1080
     --frames 3000 --spawn vista --sun 10,270 --far-field F --trees T` for `F` in {on, off}
     and `T` in {none, forest}. Each reads the march, the light pass and the frame's GPU time
     (first pass's begin to last pass's end), and a separate `--view steps` run reads the
     directional shadow ray's mean and p99 steps. They go in `nfr.md` §2 beside Plan 0032
     Phase 8's near-field forest table, with the ratios forest/open (far field on) and on/off
     (forest). The clip live bytes of the forest run are recorded against the 0.5 GB line as
     under or over. **No bar**: the bars above read the bare hills, and a forest bar would be
     invented. The march's 3.0 ms bar does not apply to the forest runs; the log states each
     against it as under or over.
     *Amended 2026-10-22 (owner, second set): both rules and leaf fill 64.* The forest runs
     also take `--leaf-fill L` for `L` in {128, 64} and, with the far field on,
     `--clip-rule R` for `R` in {level, volume}. The configurations per adapter are eight:
     far field off with `none`, forest at 128 and forest at 64 (three; the rule does not
     apply), and far field on with `none` (one; the bare hills' cells are centre-sampled, so
     the rule changes nothing without edits) and forest at 128 and 64 under each rule (four).
     That is 16 configurations on the two adapters, 48 timed runs and 16 `--view steps` runs.
     The ratios add level/volume per fill and adapter, and fill 64/128 per rule. Each forest
     run with the far field on also prints Phase 2's per-level canopy fill for its rule and
     fill, so the frame cost and the far canopy's density are read together.
  2. **Do far tree shadows survive? An observation.** In the four runs above, with the
     exposure fixed at the value auto settles to in the open, far-field-on run (logged), the
     exit line's mean ground luminance per hit level goes in the log for forest and none, and
     their ratio per level (1.0 at a level means no tree shadow there survives; Phase 2's census
     says how many trunks and canopies the level holds). The shaded and `levels` views of the
     forest run are saved under `target/` and named in the log. The log states, per level,
     whether trunk shadows and canopy shadows are visible in the saved image. *Amended
     2026-10-22 (owner, second set):* the luminance ratios and the saved images are taken
     for each forest configuration with the far field on (two rules at two fills) against
     `none`, at the same fixed exposure.

### Phase 8: The owner's windowed outdoor flight
- **Owner skill:** human
- **What:** the outdoor fly-through at 2560 x 1440, banner and HUD visible: still views of the
  horizon at each switch distance in the `levels` view and shaded, slow and 30 m/s passes along
  the terrain watching for seams and popping, far normals `gradient` against `face`, and one
  flight at `r_full` 24 against 16 (level 1 then starts at 134 to 141 m, about 1.5 px a cell at
  1080p, and the near tier's cost is the census's). *Amended 2026-10-22 (owner):* from
  `--spawn vista --trees forest`, the owner runs the day with `/` from 16:00 through sunset
  and holds `--sun 10,270`, in the shaded and `levels` views, judging distant forest shadows.
  *Amended 2026-10-22 (owner, second set):* the owner flies the forest vista at
  `--clip-rule level` and `--clip-rule volume`, each at `--leaf-fill` 128 and 64, the banner
  naming the rule, and picks the default rule.
- **Files touched:** this plan's log.
- **Done when:** the owner has recorded in the Implementation log a verdict on the seams and
  popping, the default far normal mode, and whether the outdoor `r_full` default stays 16.
  *Amended 2026-10-22 (owner):* also a verdict on distant forest shadows at sunset: whether
  they read at each level band, whether far canopies look too dense or too sparse, and
  whether thin trunks vanishing beyond about 243 m is visible. *Amended 2026-10-22 (owner,
  second set):* also the default clip rule (`level` or `volume`), which dev then sets as the
  `--clip-rule` default and records; ADR-0028's Outcome records it at the close.

## Data shapes

```rust
// illustrative: vxe-world
pub struct ClipCoord { pub level: u8, pub chunk: ChunkCoord } // level 1..=3; spans 64 << level voxels
pub enum Coarse { Unsupported, Uniform(MaterialId), Cells }   // Cells: `out` holds the 64^3 cells
pub enum ClipChunk { Uniform(MaterialId), Cells(Chunk) }
pub struct ClipResidency { pub levels: u8, pub r_clip: i32, pub hysteresis: i32,
                           pub rule: MipRule }                // 3, 22, 1, Half
pub enum MipRule { AnySolid, Half, HalfVolume }
// Half (default): solid when >= 4 of 8 children solid, level by level; the majority
//   material of the exposed solid children, else of all; a child on the chunk's face
//   counts as exposed there.
// HalfVolume (amended 2026-10-22, owner): solid when >= 8^k / 2 of the cell's voxels are
//   solid; the same material rule over its voxels.
pub trait WorldSource {
    fn chunk(&self, coord: ChunkCoord, out: &mut Chunk);
    fn coarse_chunk(&self, _at: ClipCoord, _rule: MipRule, _out: &mut Chunk) -> Coarse {
        Coarse::Unsupported
    }
}
```

- **Chunk table:** `[near window W0^3][level 1 W^3][level 2 W^3][level 3 W^3]`, `W = 2 (r_clip +
  h) + 1`, index `rem_euclid(coord, W)` per axis. A clip entry is absent (`0xFFFFFFFF`), a record
  index with the existing empty bit, or bit 31 set with the uniform material in bits 0..16 and
  no record (uniform air is the empty bit with no record).
- **Params:** `clip_levels`, `clip_window` (W), `clip_min[3]` (`vec4<i32>`), `clip_switch` and
  `clip_band` (`vec4<f32>`, voxels), `clip_bounds_min[3]` and `clip_bounds_max[3]`, `far_limit`,
  `far_normals`. The near walk's `lod1_r2` and `lod2_r2` stay for the LOD-only ring.
- **Hit flags:** bits 5..7 the hit's level (0 near, 1 to 3 clip), read by the light pass for the
  shadow start and by the `levels` view.
- **Mirror:** clip records reserved apart (`MirrorConfig::clip_records`, per level, default
  4096: 3 x 4096 x 2112 B = 25 952 256 B), clip slots from the shared bins; pressure's record
  term reads near and clip records separately, the larger counting.
- **Overlay:** `ChunkCoord -> Arc<ChunkMips>` built under the residency's rule (`MipRule::Half`
  by default), kept while the chunk has a persistence entry or is resident and edited.

## Budgets

| `nfr.md` figure | line | projection (dev box) | how the close measures it |
|---|---|---|---|
| §3 World in VRAM, live | 3 GB | clip levels 0.13 to 0.21 GB plus 1.2 MB of tables and 26 MB of clip records (guess, Phase 2 amends) | `world_budget --clip`; the outdoor autopilot's live bytes, both adapters |
| §3 World in RAM | 4 GB | clip chunks about 0.2 GB; overlay bounded by edited chunks | `world_budget --clip`; `StreamStats`; `VmHWM` in the flight |
| §5 Generation throughput | near face 3.3e8 voxels/s at 10 m/s | clip faces 2074 chunks/s, about 2.0e7 cells/s computed (0.3 of a worker), if 3.8 % of a face needs cells | Phase 2 census rate; Phase 7 cells/s/worker and busy share |
| §5 Residency latency | 500 ms p95 near | unchanged near; ring p95 under 1.28, 2.56, 5.12 s by level (bars) | Phase 7 autopilot, far field on |
| §2 Primary rays | 3.0 ms | outdoor horizon view, far field on: under 3.0 ms on the NVIDIA card (bar, a guess of +0.3 to 1.0 ms over off); iGPU recorded | Phase 7, timestamps, median of the last 30 of 300 frames, three runs, quiet box, both adapters |
| §2 Lighting | 6.0 ms | the sun ray grows with the far field, most at low sun; recorded at 45 and 10 degrees | Phase 7, both adapters |
| §2 Primary rays, lighting and frame, forest at low sun (amendment 2026-10-22, owner) | 3.0 ms; 6.0 ms; 16.6 ms | no projection: at fill 128 canopies keep 64 to 100 % of their interior cells at levels 1 to 3 under the level rule and 52 to 64 % under the volume rule, at fill 64 at most 11 %, so a grazing far ray meets denser masks than the bare hills at fill 128 | Phase 7, `--spawn vista --sun 10,270`, the eight configurations per adapter (amended 2026-10-22, owner: both rules, fills 128 and 64), both adapters |
| §3 World in VRAM, forest (amendment 2026-10-22, owner) | 3 GB | a guess: canopies add about 270, 170 and 120 `Full` bricks to a surface clip chunk at levels 1 to 3 (bounded by the canopy layer, 6, 3 and 2 bricks tall), about +0.15 to 0.25 GB, so 0.3 to 0.45 GB for three levels | Phase 2's forest census; Phase 7's forest run, recorded against 0.5 GB |
| §5 Generation throughput, forest (amendment 2026-10-22, owner) | as above | exact tree cells about 1.2e8 voxel samples a second at 10 m/s, some 1.7 workers (a guess, Context) | Phase 2's forest rate |
| §2 Streaming line (render thread) | 1.0 ms | clip syncs add uploads of surface clip chunks and table entries; recorded beside off | Phase 7 autopilot timers |

A figure read only on the NVIDIA card is labelled dev reference; the iGPU's readings are the
slow adapter's (`nfr.md` §1). The VRAM projection does not depend on the adapter; on the iGPU it
is shared DDR4.

## Risks & open questions

- **Thin trunks vanish and canopies thicken at distance** (amendment 2026-10-22, owner). A
  radius-2 trunk is gone at level 2 in the alignment Phase 2 pins (beyond about 243 m); a
  radius-3 trunk or a luckier alignment lasts longer, which the census counts. A half-filled
  canopy keeps 63.7, 87.8 and 99.9 % of its interior cells at levels 1 to 3 under the
  level-by-level rule, so far canopies are opaque blobs and their shadows, if anything, too
  dark. The by-volume rule keeps 55 % and 52 % at levels 2 and 3 instead (`P(Bin(64, 0.5) >=
  32)` and `P(Bin(512, 0.5) >= 256)`). *Settled 2026-10-22 (owner):* both are built, the
  level rule is the default, `--clip-rule volume` selects the other, Phases 2 and 7 measure
  both, and Phase 8 picks the default. At leaf fill 64 both rules thin far canopies to
  almost nothing beyond level 1, so the fill the owner keeps (Plan 0032 Phase 9) matters as
  much as the rule.
- **Tree cells may be too slow to generate exactly** (amendment 2026-10-22, owner). If
  Phase 2's forest rate cannot keep the ring latency bars at 10 m/s, the lever is a
  centre-sampled canopy that keeps a cell by a per-cell hash at the interior shares above,
  an approximation the property test then bounds instead of matching exactly.
- **Grazing rays are the step cost.** A ray skimming terrain to the horizon crosses every level
  at brick or cell granularity near the surface; the bound of non-empty chunks helps only rays
  that leave the slab. If Phase 7's march misses its bar, the next levers are a coarser shadow
  walk by distance (a followup) and a lower `r_clip` in the portable profile (two levels end at
  512 m).
- **The seam without the upscaler.** The band is dithered only when the upscaler runs (Plan
  0030); otherwise the hard switch shows up to half a coarse cell of surface offset on grazing
  terrain. Phase 4's plane test bounds it; Phase 8 judges it.
- **Interiors left out of the far field wall off long sightlines underground.** The outdoor
  coarse path draws no caverns, so a camera in a cavern sees rock where a passage continues
  past s_1 (83 m). The cave-like consumer has no far field; a source that needs it there pays
  for interior clip chunks in records and slots, which Phase 2's census would show.
- **Thin surface layers at distance.** The half rule's exposed-material choice keeps grass on
  grass-over-soil, but where the rule empties the cell holding the exposed layer, the cell
  under it shows soil. Phase 2 records the share; Phase 8 judges the colour.
- **The source's coarse path is the consumer's work.** A coarse path far from the half rule
  shows seams around edited chunks and at the level boundary. Phase 2's property test is the
  template a consumer copies; `Unsupported` is always safe (no far field).
- **Plan 0028 and Plan 0032 must land first.** The byte arithmetic is in Plan 0028's units and
  record; the scene, sun and shadow ray are Plan 0032's. If either changes shape, the census
  (Phase 2) re-derives the bars and the log notes it.
- **Determinism:** clip chunks and overlays are integer, seeded through the source, and
  thread-order independent (Phase 3's hash at 1 and 7 workers). Rendering is not deterministic;
  its goldens are blessed on lavapipe with the band off.
- **Threading:** coarse generation and overlay builds run on the existing workers, outside any
  lock but the chunk read lock an overlay build takes, as demotion does today.
- **Precision:** at 1 km a position is 10 240 voxels from the camera; `f32` keeps about 1e-3
  voxel there, within the walk's plane-order tolerance. A camera far from the world origin is
  an existing limit, not this plan's.
- **Probe and reflection rays** go through `march` and so see the far field; Plan 0029's probe
  rays have their own `t_limit` and stay near.

## What this plan does NOT do

- **Sky, sun, moon and time of day** (Plan 0032): this plan extends its sun shadow ray.
- **Water surfaces, reflection and refraction** (Plan 0034): the translucent walk runs at clip
  levels as at mip levels, but water optics at distance are that plan's.
- **Aerial perspective and volumetric clouds** (Plan 0035): it consumes the far-field hit
  distance this plan writes; cloud shadows on far terrain are that plan's.
- **Weather, wet surfaces and snow cover** (Plan 0036), including snow on far terrain.
- **Retiring ADR-0003's LOD-only ring**, or a coarse path for the cave source: a later
  decision by measurement.
- **More than three levels, or a horizon beyond 1024 m**: the overlay uses the three
  `ChunkMips` levels; deeper levels need deeper mips.
- **Prefab placements in the far field**: `StampedSource` forwards `coarse_chunk` to its base,
  so stamped prefabs are absent from clip levels until a followup downsamples them.
- **CPU queries, raycasts or collision against clip levels**, and edits beyond `r_full`.
- **A shadow walk that coarsens with distance from its origin** (cone-like): a followup if
  Phase 7's low-sun reading asks for it.
- **Probes beyond the near volume** (Plan 0029 stays near-field).

## Implementation log

> Written by `dev`: one row per phase as its commit lands, then the close block. **The phases
> above are the contract. This section records what happened.** Observations, never conclusions:
> no pass list, no self-assessment. Deviations and unmet done-whens are **always** disclosed.
> Keep it shorter than `## Implementation phases`.

| phase | owner | state | commit |
|---|---|---|---|
| 1: Walking skeleton: one level, bounded, a box at 150 m | dev | not started | |
| 2: The outdoor source's coarse path and the census | dev | not started | |
| 3: Clip residency on the streamer | dev | not started | |
| 4: Three levels in the mirror, scrolling, the band and the far normals | dev | not started | |
| 5: Edits fold into the far field | dev | not started | |
| 6: The sun's shadow ray walks the far field | dev | not started | |
| 7: The fly-through's switches, HUD rows and readings on both adapters | dev | not started | |
| 8: The owner's windowed outdoor flight | human | not started | |

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

- A reference worth adding to `docs/references/`: Losasso and Hoppe, "Geometry Clipmaps:
  Terrain Rendering Using Nested Regular Grids" (2004), and Tanner, Migdal and Jones, "The
  Clipmap: A Virtual Mipmap" (1998), the sources of the nested-level, toroidal-update shape.
- At close: ADR-0003 gets an Outcome line pointing at ADR-0028 for the far field, if the ring's
  role changed in the fly-through's defaults.
