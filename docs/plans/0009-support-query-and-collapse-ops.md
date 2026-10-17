# 0009: A support query on the coarse grid, and deterministic drop and rubble ops for collapse

> **Status:** in-progress
> **Created:** 2026-10-16
> **Milestone:** M8 in `docs/roadmap.md` (the collapse half; volumetrics, debris and the cosmetic rigid-body layer stay in M8 proper)
> **Related ADRs:** [ADR-0008](../adrs/0008-collapse-as-capped-support-flood-and-rigid-vertical-drop.md) (the mechanism and what it is not), ADR-0005 (the controller's landing event), ADR-0002 (layout, access model)
> **Prerequisite:** Plan 0004 closed (`CoarseGrid`, `CellChange`). Independent of Plans 0002, 0003 and 0005 to 0008. Plan 0007 reserves op kinds 3 and 4 for the two ops here; wiring them through `Server` is a follow-up once both close.

## TL;DR

`vxe-world` gains `find_unsupported`, a capped flood over load-bearing cells of the coarse
grid that reports which solid regions lost their connection to an anchor after an edit, and
two ops: `drop_region`, which translates a region straight down by the air gap under it (up to
a per-call maximum, so a per-tick call is an animated, authoritative fall), and
`remove_region`, which carves a region and piles a fraction of it as rubble below. The first
thing a reader sees is a headless test on a one-chunk fixture: carving the single pillar under
a 3 × 3-cell slab reports that slab as one unsupported region of exactly 4608 voxels, and
dropping it yields a world hash-equal to one built with the slab already on the floor. The
first thing the user sees is the fly-through: dig out the rock under a ledge and watch the
ledge fall at a fixed speed, with its counts printed.

## Context & problem

The owner's physics decision (`docs/roadmap.md`, 2026-10-16) makes collapse emergent and
authoritative, and ADR-0008 fixes the mechanism: a support flood on the grid, rigid vertical
drops, rubble, and no rigid-body engine in the authoritative world. Plan 0004 delivers the
grid and the per-edit changed cells this plan consumes; nothing else exists. `nfr.md` has no
figure for support or collapse; this plan proposes them under "Budgets".

## Decision

We implement ADR-0008 in `vxe-world`, as derived queries and ops over the grid and the bricks,
in integer arithmetic, with every result in the edit-result vocabulary. Candidates for the
flood are the load-bearing cells among an edit's changed cells and their six neighbours,
visited in ascending coordinate order; each flood is a breadth-first search in a fixed
neighbour order, so the output is canonical. Cells in non-resident chunks count as anchors
(unknown is conservative). The ops do not run inside `World::apply`: the game calls the query
with the changes it got from `CoarseGrid::update` and chooses what to do with each region.

We rejected a rigid-body engine, scripted-only collapse, exact dynamic connectivity and
voxel-granular support (ADR-0008). We also rejected triggering the query inside `apply`,
for the same reason Plan 0004 keeps the grid update outside it: a consumer that does not want
it should not pay for it, and the §4 edit figure must stay what it measures.

## Architecture diagram

```mermaid
flowchart LR
    subgraph consumer["game / apps/flythrough"]
        ED[EditOp] --> AP
        CHOOSE[per region:<br/>drop per tick, or remove]
    end
    subgraph world["vxe-world (headless)"]
        AP[World::apply -> EditResult]
        CG[CoarseGrid::update<br/>-> changed cells]
        SUP[find_unsupported<br/>capped flood]
        DROP[World::drop_region<br/>min column gap]
        REM[World::remove_region<br/>carve + rubble]
        W[(bricks)]
    end
    AP --> CG --> SUP -->|regions| CHOOSE
    CHOOSE --> DROP --> W
    CHOOSE --> REM --> W
    DROP -->|result| CG
    REM -->|result| CG
```

## The fixture

Every done-when below uses fixture `F` unless stated: a one-chunk extent `[0, 0, 0]..[1, 1,
1]` generated from `UniformSource(AIR)` with material 3 filled by three box ops, voxel bounds
half-open:

| Part | Voxels | Count | Cells (global brick coordinates) |
|---|---|---|---|
| Floor | `[0, 0, 0]..[64, 64, 8]` | 32 768 | 64 cells at z = 0 (on the extent's bottom face: anchors) |
| Pillar | `[32, 32, 8]..[40, 40, 40]` | 2048 | (4, 4, 1) to (4, 4, 4) |
| Slab | `[24, 24, 40]..[48, 48, 48]` | 4608 | (3..=5, 3..=5, 5): 9 cells |

Params unless stated: `solid_min = 64`, `max_region = 4096`, `boundary_anchors = true`, no
explicit anchors. "The pillar carve" is `Carve` of box `[32, 32, 8]..[40, 40, 40]`, which
returns `[(3, 2048)]` and makes `CoarseGrid::update` report 4 changes (cells (4, 4, 1) to
(4, 4, 4), 512 to 0). "Hash-equal to a direct build" means the world's `voxel_hash` equals
that of a fresh fixture generated with the named parts only.

## Implementation phases

### Phase 1: The support flood
- **Owner skill:** dev
- **What:** `CellCoord` helpers, anchors, `find_unsupported`, and the bench.
- **Files touched:** `crates/vxe-world/src/{lib.rs, coarse.rs, support.rs}`,
  `crates/vxe-world/tests/collapse.rs`, `crates/vxe-world/benches/collapse.rs`,
  `crates/vxe-world/Cargo.toml`.
- **Done when:**
  - `CellCoord([i32; 3])` is the global brick coordinate (chunk × 8 + brick), with
    `CellCoord::from(chunk, slot)` and `split() -> (ChunkCoord, usize)` in `coarse.rs`;
    `CellChange` gains or exposes its `CellCoord`.
  - `SupportParams { solid_min: u16, max_region: u32, boundary_anchors: bool }` and
    `Anchors<'a> { explicit: &'a [CellCoord] }`. A cell is load-bearing iff `solid >=
    solid_min`. A cell is an anchor iff its `dominant` material is `INDESTRUCTIBLE`, or it
    lies on a boundary face of the world extent and `boundary_anchors`, or it is in
    `explicit`, or its chunk is not resident.
  - `find_unsupported(&World, &CoarseGrid, &SupportParams, &Anchors, changed: &[CellChange])
    -> Vec<Region>`: candidates are the load-bearing cells among `changed` and their
    6-neighbours, in ascending `(x, y, z)` order; each unvisited candidate starts a
    breadth-first flood over load-bearing 6-neighbours in the order −x, +x, −y, +y, −z, +z;
    a flood that reaches an anchor or exceeds `max_region` marks its cells supported; a flood
    that exhausts yields `Region { cells: Vec<CellCoord>` (ascending)`, counts: Vec<(MaterialId,
    u32)>` (per material over the region's bricks, ascending by material)`, aabb: (VoxelCoord,
    VoxelCoord)` (inclusive bounds of its solid voxels)`}`. Regions are returned in the order
    their first cell was visited.
  - On `F` with `changed` = every cell (an initial sweep): no regions.
  - After the pillar carve: exactly one region, `cells` = the 9 slab cells ascending, `counts
    == [(3, 4608)]`, `aabb == ([24, 24, 40], [47, 47, 47])`.
  - A floor hole: `Carve` of `[8, 8, 0]..[16, 16, 8]` (one floor cell) yields 1 change and
    no regions, because the floor is boundary-anchored.
  - Two pillars: `F` plus a second pillar at `[24, 24, 8]..[32, 32, 40]`. Carving the first
    pillar yields no regions; carving both yields the slab region as above.
  - The cap: `max_region = 8` after the pillar carve yields no regions (the 9-cell slab
    exceeds the cap and is treated as supported; this documents the semantics).
  - The threshold: on `F`, `Carve` of `[32, 32, 16]..[40, 36, 24]` removes 256 voxels of
    cell (4, 4, 2), leaving 256. With `solid_min = 300` the cell is no longer load-bearing
    and the region is the slab plus cells (4, 4, 3) and (4, 4, 4): 11 cells, `counts ==
    [(3, 5632)]`. With `solid_min = 200` there are no regions.
  - `cargo bench -p vxe-world --bench collapse`: `find_unsupported` after a radius-10 carve
    at the centre of a solid 4³-chunk world (about 32 changed cells, every flood running to
    the cap), reported in microseconds per query. Proposal: under 1 ms.

### Phase 2: The drop op
- **Owner skill:** dev
- **What:** `World::drop_region`, the rigid vertical translation.
- **Files touched:** `crates/vxe-world/src/{collapse.rs, lib.rs}`,
  `crates/vxe-world/tests/collapse.rs`.
- **Done when:**
  - `World::drop_region(&self, cells: &[CellCoord], max_drop: i32) -> Result<DropResult,
    EditError>` with `DropResult { moved: Vec<(MaterialId, u32)>, dropped: i32, touched:
    Vec<ChunkCoord>, aabb: Option<(VoxelCoord, VoxelCoord)> }`. For every (x, y) column of
    the cells' footprint that holds a solid voxel in the region's bricks, the gap is the
    number of air voxels directly below the column's lowest solid region voxel, down to the
    first solid voxel, capped at `max_drop`; a column whose scan enters a non-resident chunk
    fails with `NotResident(c)` before anything moves. `dropped` is the minimum gap over the
    columns (0 when any column has gap 0). When `dropped > 0` every solid voxel of the
    region's bricks is cleared and rewritten `dropped` lower; destinations are air or
    vacated region cells by construction, so nothing is overwritten. `moved` counts the
    region's voxels by material when `dropped > 0` and is empty otherwise; `touched` and
    `aabb` cover both the vacated and the landed voxels.
  - After the pillar carve, `drop_region(slab cells, 64)` returns `dropped == 32`, `moved ==
    [(3, 4608)]`, `touched == [[0, 0, 0]]`, `aabb == Some(([24, 24, 8], [47, 47, 47]))`, and
    the world is hash-equal to a direct build of floor plus slab at `[24, 24, 8]..[48, 48,
    16]`. Calling it again returns `dropped == 0` and empty `moved`.
  - Per-tick fall: from the carved state, `drop_region(slab, 10)` four times returns
    `dropped` of 10, 10, 10 and 2, then 0; the world after the fourth call is hash-equal to
    the 64-drop result.
  - Uneven bottom: carve only the pillar's lower three cells (`[32, 32, 8]..[40, 40, 32]`);
    the region is the slab plus cell (4, 4, 4), 10 cells, `counts == [(3, 5120)]`; the lowest
    voxels are the stub's at z = 32, so `dropped == 24` and the world is hash-equal to floor
    plus slab at `[24, 24, 16]..[48, 48, 24]` plus stub at `[32, 32, 8]..[40, 40, 16]`.

### Phase 3: The remove op with rubble
- **Owner skill:** dev
- **What:** `World::remove_region`.
- **Files touched:** `crates/vxe-world/src/collapse.rs`, `crates/vxe-world/tests/collapse.rs`.
- **Done when:**
  - `World::remove_region(&self, cells: &[CellCoord], rubble: MaterialId, fraction: (u32,
    u32), max_depth: i32) -> Result<RemoveResult, EditError>` with `RemoveResult { removed:
    Vec<(MaterialId, u32)>, placed: u32, touched, aabb }`. Every solid voxel of the region's
    bricks is carved and counted in `removed`. Per column, `n` is the number of voxels
    removed in that column and `h = n * fraction.0 / fraction.1` (integer division); from
    the column's lowest removed voxel the scan goes down through air to the first solid
    voxel within `max_depth` (non-resident: `NotResident` before anything changes), and `h`
    rubble voxels are written upward from that solid voxel, fewer if the air run is shorter;
    `placed` is the number written.
  - After the pillar carve, `remove_region(slab, 7, (1, 4), 64)` returns `removed == [(3,
    4608)]`, `placed == 1152` (576 columns × 2), `count_in_box([24, 24, 8], [48, 48, 10]) ==
    [(7, 1152)]`, and the world is hash-equal to floor plus a box of material 7 at `[24, 24,
    8]..[48, 48, 10]`.
  - `fraction = (0, 1)` returns `placed == 0` and leaves the world hash-equal to the floor
    alone. `fraction = (1, 1)` returns `placed == 4608` and a world hash-equal to floor plus
    a box of 7 at `[24, 24, 8]..[48, 48, 16]`.
  - The course: on `F`, the pillar carve, the query, four drops of 10, a second pillar carve
    on a fresh `F`, the query and a remove at (1, 4), hashing every result and the world
    after each step with FNV-1a 64 into one constant blessed on Linux and asserted on the
    Windows CI arm.

### Phase 4: Collapse in the fly-through
- **Owner skill:** dev
- **What:** the testbed keeps a grid, runs the query after every dig and drops what falls.
- **Files touched:** `apps/flythrough/src/{main.rs, lib.rs}`.
- **Done when:**
  - The fly-through builds a `CoarseGrid` at start (its build time printed with the startup
    lines) and updates it from every edit result. After each dig it calls `find_unsupported`
    with `solid_min = 64`, `max_region = 4096`, boundary anchors on, and prints each region
    as `unsupported: N cells, {material: count, ...}`.
  - Each unsupported region is dropped by 8 voxels per frame (`drop_region(.., 8)`) until
    `dropped == 0`, then printed as `landed after N frames, fell M voxels`. Key `R` toggles
    remove-with-rubble instead (fraction 1/4, rubble material `RUBBLE`, a new row in
    `cave_materials()` that changes no voxel and so no existing constant).
  - `F1` adds the last query's microseconds and the number of regions. The log records a
    reading of the query after a typical dig in the cave on the dev box.
  - Once Plan 0007 lands, the drops and removes go through `Server` as op kinds 3 and 4:
    follow-up.

## Data shapes

```rust
// illustrative
pub struct CellCoord(pub [i32; 3]);                       // global brick coordinate
pub struct SupportParams { pub solid_min: u16, pub max_region: u32, pub boundary_anchors: bool }
pub struct Anchors<'a> { pub explicit: &'a [CellCoord] }
pub struct Region { pub cells: Vec<CellCoord>, pub counts: Vec<(MaterialId, u32)>, pub aabb: (VoxelCoord, VoxelCoord) }
pub struct DropResult { pub moved: Vec<(MaterialId, u32)>, pub dropped: i32, pub touched: Vec<ChunkCoord>, pub aabb: Option<(VoxelCoord, VoxelCoord)> }
pub struct RemoveResult { pub removed: Vec<(MaterialId, u32)>, pub placed: u32, pub touched: Vec<ChunkCoord>, pub aabb: Option<(VoxelCoord, VoxelCoord)> }
```

Working memory: the flood's visited set is a `HashSet<CellCoord>` bounded by `max_region`
per flood plus the supported marks of the candidates' components; it is not kept between
calls. The drop's column table is one `i32` per column of the footprint (576 for the slab).

## Budgets

| Figure | Expected effect | Close measures by |
|---|---|---|
| `nfr.md` §4 edits (new lines) | Support query after a 1 m carve under 1 ms; drop of the 9-cell slab under 100 µs; remove under 200 µs; a 4096-cell region drop under 20 ms (a guess: 2 M voxels moved) | `cargo bench -p vxe-world --bench collapse`, dev box, commit named |
| §6 determinism | The course constant on both arms | Phase 3 course test |
| §3 RAM | Nothing retained between calls | By inspection |

## Risks & open questions

- **The cap and the threshold** are the two documented limits of ADR-0008. The fly-through's
  defaults are a starting point, not a reading; the consumer tunes per level scale.
- **Floating leftovers.** Voxels in cells below `solid_min` are neither load-bearing nor part
  of a region, so a thin tooth under a dropped slab stays in the air. The game sweeps with a
  carve or a remove; the plan does not hide this.
- **Two regions in one tick** are processed in the order the game calls; the second's gap
  sees the first's landed voxels. Deterministic, order-dependent, and the game's call.
- **Column scans** cost `columns × gap`; `max_drop` bounds the gap and a per-tick call keeps
  it small. A single 64-voxel drop over a 4096-cell region is the worst case in the bench.
- **Open:** whether `drop_region` should refuse a region with cells in more than one chunk
  column when some are non-resident (it does, by the first-missing-chunk rule) or drop what
  it can. The plan refuses; an op either happens or does not.

## What this plan does NOT do

- **Cosmetic rigid bodies**: an optional client-side crate after M6, when voxel-model
  entities can draw the pieces.
- **Rotation, sliding, stacking, material strength**: ADR-0008.
- **Running the query inside `apply`**, or inside the controller.
- **Wiring into `Server` and the log**: follow-up with Plan 0007 (kinds 3 and 4 reserved).
- **Volumetrics and particle debris**: M8 proper.

## Implementation log

| phase | owner | state | commit |
|---|---|---|---|
| 1: The support flood | dev | done | db8dba9 |
| 2: The drop op | dev | done | cff74d5 |
| 3: The remove op with rubble | dev | done | a5ac4dd |
| 4: Collapse in the fly-through | dev | done | 25b483d |

### Notes

- Phase 1: public API beyond the data shapes: `CellCoord::min_voxel`, `CellChange::cell()`, `CoarseGrid::cell_at(CellCoord)`. `CellCoord` has no `From<[i32; 3]>` (the inherent `from(chunk, slot)` would shadow it).
- Phase 1: a chunk is resident for the query when the grid holds it and the world's store holds it; a neighbour in any other chunk ends the flood as supported. A flood also ends as supported when it reaches a cell an earlier flood of the same call settled as supported. An anchor rule other than non-residency applies to load-bearing cells only (the flood never steps on another).
- Phase 1: "exceeds `max_region`" is taken as more than `max_region` cells in the flood (`max_region = 9` on the slab yields the region; 8 does not; both asserted).
- Phase 1: in a bounded world every cell on the extent's faces borders a chunk outside the extent, which is not resident, so the faces anchor through non-residency even with `boundary_anchors = false` (test `a_cell_in_a_chunk_the_grid_does_not_hold_anchors`).
- Phase 1: a `Region`'s `aabb` falls back to its cells' bounds when its bricks hold no solid voxel (only reachable with `solid_min = 0`).
- Phase 1: bench reading: `find_unsupported` after the radius-10 carve at the origin of a solid `[-2, 2)^3` world, 32 changed cells: 390.5 us per query (criterion median of [390.14, 390.53, 390.99] us), AMD Ryzen 9 5900HS, taken under the exclusive lock, at the Phase 1 commit. The first candidate's flood reaches the cap (4097 cells); every later candidate meets a cell it settled as supported.
- Phase 2: the per-tick fall test passes, per call, the cells holding the slab's voxels at that point (read from the grid kept current with `DropResult::edit_result()`, within the slab's 3 x 3 footprint above the floor), not the original 9 slab cells: after the first drop of 10 those cells hold no voxel, and a call naming them moves nothing.
- Phase 2: a column whose region voxels are split by a solid voxel outside the region is scanned below every run of region voxels, the region's own voxels counting as free; the plan's rule (below the lowest region voxel only) is the same when no such voxel exists, and otherwise could land a voxel on a solid one. Test `a_solid_voxel_between_two_region_runs_wedges_the_piece` (dropped 0 with the voxel, 4 without).
- Phase 2: public API beyond the data shapes: `DropResult::edit_result()` (`changed_from = moved`), for `CoarseGrid::update`. `dropped == 0` and `max_drop <= 0` return `DropResult::default()`. A cell in a non-resident chunk fails with the least such chunk; a scan fails with the chunk of the first non-resident voxel, columns ascending by `(x, y)`, runs ascending by `z`, each scanned to `max_drop`.
- Phase 2: also touches `crates/vxe-world/benches/collapse.rs` (the drop budgets) and `crates/vxe-world/src/support.rs` (the cell-key hasher shared with `collapse.rs`); neither is in Files touched.
- Phase 2: bench readings, AMD Ryzen 9 5900HS, exclusive lock, at the Phase 2 commit: `drop_region` of the fixture's 9-cell slab by 32: 79.7 us (criterion median of [79.55, 79.68, 79.82] us); a 16^3-cell cube (2 097 152 voxels) by 64: 6.95 ms ([6.945, 6.948, 6.951] ms, 10 samples). A first per-voxel version measured 879 us and 312 ms; the op now composes bricks in memory and moves a whole `Uniform` brick when the drop is a multiple of 8.
- Phase 3: `count_in_box` takes inclusive bounds; the done-when's `count_in_box([24, 24, 8], [48, 48, 10])` is asserted as `count_in_box([24, 24, 8], [47, 47, 9]) == [(7, 1152)]` (the half-open box). Called with `[48, 48, 10]` inclusive it also counts 723 air voxels.
- Phase 3: "within `max_depth`": the solid voxel must be one of the `max_depth` voxels scanned below the lowest carved voxel; a column with none gets no rubble (floor 32 below the slab: `max_depth = 32` places 0, `33` places 1152). The rubble run stops at the first voxel that is not air after the carve (a voxel in a non-resident chunk counts as not air). `h = 0` when `fraction.1 == 0` or `rubble == AIR`. Every solid voxel of the region's bricks is removed, an `INDESTRUCTIBLE` one included.
- Phase 3: public API beyond the data shapes: `RemoveResult::edit_result()` (`changed_from = removed`).
- Phase 3: the course drops the slab with the cells re-derived from the grid after each drop (as in Phase 2) and asserts the falls `[10, 10, 10, 2]`; it hashes each `EditResult`, the `Region`s, each `DropResult` and the `RemoveResult` (counts, touched chunks, aabb, `dropped`, `placed`) and `voxel_hash` after every step. Constant `0x3178_0a15_61c1_3041`, blessed on the dev box (Linux); the Windows arm is CI's (`os: [ubuntu-latest, windows-latest]`).
- Phase 3: also touches `crates/vxe-world/benches/collapse.rs` (the remove budget) and `crates/vxe-world/src/lib.rs` (the `RemoveResult` export); neither is in Files touched.
- Phase 3: bench readings, AMD Ryzen 9 5900HS, exclusive lock, at the Phase 3 commit: `remove_region` of the fixture's 9-cell slab, rubble 1/4: 125.1 us ([124.85, 125.10, 125.38] us); in the same run `find_unsupported` 391.0 us, the slab drop 78.2 us, the 4096-cell drop 6.56 ms.
- Phase 4: the grid, the query and the falls run on the bounded level (`--level`) only, where the fly-through already kept a coarse grid; while streaming no grid is kept and `R` prints `collapse: not kept while streaming`. The startup line is the existing `coarse: <chunks> chunks, <bytes> bytes, <n> walkable cells, built in <ms> ms`.
- Phase 4, deviation: a falling region ends on the first drop shorter than 8 voxels (zero included), not on a call returning `dropped == 0`. After a whole-step drop the region's cells are its old cells one cell lower; after a shorter drop the voxels straddle two cell layers and the cells holding only the region are not known, while a column met a solid voxel outside the region, so the next drop would be 0. `landed after N frames` counts the frames the region moved on.
- Phase 4: a region the query reports that overlaps a region already falling is skipped. Fills do not run the query (the done-when names digs). Rubble removal is 1/4 down to 64 voxels (6.4 m); the line is `removed: {rock: 4608}, 1152 rubble`. `F1` prints `collapse: last query <us> us, <n> regions` after the frame line.
- Phase 4: `RUBBLE = 4` is defined in `lib.rs` beside `cave_materials()` (the cave's other ids live in `cave.rs`, outside Files touched). Every existing test and golden passed unchanged with the row added.
- Phase 4: headless scripted tests `tests::digging_out_the_pillar_drops_the_ledge_8_voxels_a_frame` (one-chunk ledge fixture in rock: `unsupported: 9 cells, {rock: 4608}`, four frames of 8, `landed after 4 frames, fell 32 voxels`, grid equal to a rebuild) and `tests::with_r_the_ledge_is_removed_leaving_a_quarter_as_rubble`; the reading test `tests::a_dig_in_the_level_prints_the_query_time` is `#[ignore]`d (level generation in an unoptimized test build).
- Phase 4 reading, AMD Ryzen 9 5900HS, release build, exclusive lock, at the Phase 4 commit: seed-1 level, six default-radius (0.5 m) digs from the spawn along +x, -x, +y, -y, +z, -z: 329/118/329/329/329/513 voxels, 8/5/8/8/8/13 changed cells, query 619/501/531/495/493/492 us, 0 regions each (`cargo test --release -p flythrough --bin flythrough -- --ignored --nocapture a_dig_in_the_level_prints_the_query_time`). `flythrough --level --seed 1 --headless --frames 10` on the RTX 3080 Laptop printed `coarse: 3200 chunks, 6553600 bytes, 3010 walkable cells, built in 263 ms`.
- Phase 4, awaits the owner (windowed, by hand): `cargo run --release -p flythrough -- --level --seed 1`; dig out the rock under a ledge or an overhang and judge the `unsupported: N cells, {...}` line, the ledge falling at a steady 8 voxels a frame and `landed after N frames, fell M voxels`; press `R`, dig under another and judge the region vanishing into a rubble pile; press `F1` and read the `collapse: last query` line. The log does not record this as driven by hand.
- Op kinds: no phase touches op-kind numbering or wire tags. Plan 0007 on main reserves kinds 2 to 4 for Plans 0008 and 0009 (this plan's header says kinds 3 and 4 for drop and remove); `Server` wiring of `drop_region` and `remove_region` stays the follow-up both plans name. Plan 0007's boundary entry kind 0x80 does not interact with anything here.
- Fix pass, review finding 1 (`boundary_anchors = false` inert in a bounded world), d156896: a chunk outside a non-empty extent reads as known air (`CoarseCell { solid: 0, dominant: AIR }`); a chunk the grid or world lacks inside the extent, or in an unbounded world, stays unknown (anchored). This supersedes the Phase 1 note on faces anchoring through non-residency. Test `a_cell_in_a_chunk_the_grid_does_not_hold_anchors` is rewritten as `boundary_anchors_off_lets_the_floor_go_and_a_missing_chunk_anchors` (off: floor 64 cells `[(3, 32768)]` then slab 18 cells `[(3, 9216)]`; on: the slab only; off after evicting chunk `[1, 0, 0]`: none). The course constant is unchanged.
- Fix pass, review finding 2 (`remove_region` destroyed `INDESTRUCTIBLE` voxels), committed with this line: `remove_region` spares them as `Mode::Carve` does (left in place, not in `removed`, not in a column's `n`, solid to the rubble scan and run; region bricks are composed with them kept). `drop_region` still moves them, and its doc says so. This supersedes the last sentence of the Phase 3 note on "within `max_depth`". Test `remove_region_spares_indestructible_voxels`: query `[(3, 4607), (4, 1)]` over the 9 slab cells; `removed == [(3, 4607)]`, `placed == 1151`, voxel [30, 30, 40] still 4, rubble 7 at [30, 30, 41]; world hash equal to a direct build. The course constant is unchanged.
- Followups noticed, not acted on: `walkable.rs` keeps private `global`/`split` helpers equal to `CellCoord::from`/`split` (outside Files touched). `nfr.md` §4 has no collapse lines yet (not in Files touched; the Budgets table says the close measures them).

### Close triggers

_(Facts for the architect. No recommendations, and no suggested version bump.)_

- **What shipped:** `vxe-world`: `CellCoord`, `SupportParams`, `Anchors`, `Region`, `find_unsupported` (`support.rs`); `World::drop_region`, `World::remove_region`, `DropResult`, `RemoveResult` (`collapse.rs`); bench `vxe-world/collapse`; test `tests/collapse.rs` with the course constant `0x3178_0a15_61c1_3041`. Fly-through: support query after every dig on the bounded level, per-frame drops of 8 voxels, `R` remove with rubble, `F1` query line, `RUBBLE` row 4 in `cave_materials()`.
- **Public API changed:** `vxe-world` (added): `CellCoord`, `CellChange::cell`, `CoarseGrid::cell_at`, `SupportParams`, `Anchors`, `Region`, `find_unsupported`, `DropResult` (+ `edit_result`), `RemoveResult` (+ `edit_result`), `World::drop_region`, `World::remove_region`. `flythrough` lib (added): `RUBBLE`.
- **Budget readings** (dev box AMD Ryzen 9 5900HS laptop, Linux; `cargo bench -p vxe-world --bench collapse`; criterion median (low, high); exclusive perf lock), at a5ac4dd: `find_unsupported` after a radius-10 (1 m) carve at the centre of a solid 4^3-chunk world, 32 changed cells: 391.04 us (390.52, 391.63), proposal 1 ms; `drop_region` of the 9-cell slab by 32: 78.19 us (78.12, 78.27), proposal 100 us; `remove_region` of the slab, rubble 1/4: 125.10 us (124.85, 125.38), proposal 200 us; `drop_region` of a 4096-cell cube (2 097 152 voxels) by 64: 6.559 ms (6.553, 6.563), 10 samples, proposal 20 ms (a guess). Query after a typical dig in the cave (seed-1 level, 0.5 m dig, release build, at 25b483d): 492 to 619 us over six digs (Notes, Phase 4). RAM: nothing is kept between calls (the flood's sets and the ops' snapshots are locals); the fly-through keeps the falling regions' cell lists.
- **Determinism:** course constant blessed on Linux; the Windows arm of CI runs the same test.
- **Gate at 25b483d (the close commit changes this plan only):** `cargo build --workspace --all-targets` 0; `cargo nextest run --workspace` 191 passed, 2 skipped, exit 0 (exclusive lock, NVIDIA GeForce RTX 3080 Laptop GPU present); `cargo test --workspace --doc` 0; `cargo clippy --workspace --all-targets -- -D warnings` 0; `cargo fmt --all --check` 0; `cargo deny check` advisories, bans, licenses, sources ok; `node scripts/check-doc-links.mjs` 0; `node scripts/check-comment-hygiene.mjs` 0; `node scripts/check-index-rows.mjs` 0 and `--self-test` 9/9; `node --test .claude/hooks/*.test.mjs` 0. CARGO_BUILD_JOBS=3.
- **Outstanding `human` phases:** none. Phase 4's hand-driven checks await the owner (Notes).
