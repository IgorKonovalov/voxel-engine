# Changelog

One entry per closed plan, written at the architect's close ceremony together with the version
bump in `Cargo.toml` (`workspace.package.version`). Format: [Keep a Changelog](https://keepachangelog.com/),
SemVer 0.x (any minor may break the API until 1.0).

## [Unreleased]

Plan 0012 (no version: repository tooling): CI runs `scripts/check-comment-hygiene.mjs` (no relative link or plan-relative narration in `.rs`/`.wgsl` comments) and `scripts/check-index-rows.mjs` with `--self-test` (rows inside `roster:begin` markers at most 320 bytes); seeded fixture trees under `scripts/fixtures/`.

## [0.8.0] - 2026-10-17

Plan 0009: the collapse half of M8 (ADR-0008, accepted). A support query on the coarse grid
reports which solid regions lost their anchor after an edit, and two integer ops drop a region
straight down or remove it with rubble. Every reading is under its proposal (`docs/nfr.md`
section 4, "Collapse cost"; AMD Ryzen 9 5900HS): the query 327-396 us against 1 ms, the slab
drop 49-72 us against 100 us, the removal 119-127 us against 200 us, and the 4096-cell cube
drop 6.8-8.6 ms against 20 ms. Three windowed checks await the owner.

- `vxe-world`: `CellCoord` (a global coarse-grid cell), `CellChange::cell` and
  `CoarseGrid::cell_at`.
- `vxe-world`: `SupportParams`, `Anchors`, `Region` and `find_unsupported`, a capped flood over
  load-bearing cells from an edit's changed cells. A chunk outside a non-empty extent reads as
  air, so `boundary_anchors` decides whether the extent's faces hold anything up.
- `vxe-world`: `World::drop_region`, a rigid in-place vertical translation by the air gap
  under the region, at most `max_drop`. `World::remove_region` carves the region and piles a
  fraction as rubble; it spares `INDESTRUCTIBLE` voxels, which `drop_region` moves.
  `DropResult` and `RemoveResult` carry `edit_result()` for `CoarseGrid::update`.
- `vxe-world` tests and benches: the `collapse` bench (query, slab and cube drops aligned and
  unaligned, removal) and the collapse course test in `tests/collapse.rs`, pinned at
  `0x3178_0a15_61c1_3041`.
- `apps/flythrough`: on `--level`, the support query runs after every dig and an unsupported
  region falls 8 voxels a frame. `R` toggles removal with rubble instead of the fall, and `F1`
  prints the last query's time. A `RUBBLE` material (row 4) joins `cave_materials()`.

## [0.7.0] - 2026-10-17

Plan 0014: the second pass on the `docs/nfr.md` §4 edit cost. Edits are cheaper, but the
2 ms figure is still missed; the choice between a brick-representation ADR and a measured §4
cap awaits the owner. Readings (AMD Ryzen 9 5900HS, `cargo bench -p vxe-world --bench edits`,
paired with the Plan 0003 tip): `worst` 3.55 -> 3.19-3.26 ms, `mixed` 2.55 -> 2.33-2.36 ms,
`tool` 8.05 -> 6.78-6.80 ms, and the new `worst_warm` 3.03-3.05 ms.

- `vxe-world`: a brick-array pool. Expanding a uniform brick takes a pooled array, collapsing
  one and evicting a chunk give arrays back. New API: `World::BRICK_ARRAY_POOL_CAP` (8192
  arrays, at most 8 MiB), `World::reserve_brick_arrays` and `World::pool_len`.
- `vxe-world`: the sphere's row spans come from a brick-row span walk, and a capsule row is
  solved analytically from its nearest centre plus a bisection instead of a linear scan.
- `vxe-world`: `World::stamp` takes and gives back pooled arrays.
- `vxe-world` tests and benches: a uniform-world reference oracle
  (`apply_matches_the_reference_on_uniform_worlds`); the `worst_warm` and `tool_warm` edit
  bench cases (after a reserve) and the `expand` bench group.
- `apps/flythrough`: tops the pool up to a 4096-array reserve after each frame's edits and
  prints the pool size on its `bench:` line.

## [0.6.0] - 2026-10-17

Plan 0008: the headless halves of M6 (`.vox` import) and M7 (prefab stamping). The stamp
bench reads 22.7 us for a 16³ shelled box and about 2 ms for a 64³ prefab against the 100 us
and 5 ms proposals (`docs/nfr.md` section 4). Two windowed checks await the owner.

- `vxe-assets` (new crate, depends on `vxe-world` only): a `.vox` parser for `SIZE`, `XYZI`,
  `RGBA`, `PACK` and the `nTRN`/`nGRP`/`nSHP` scene graph, other chunks skipped by size;
  the generated MagicaVoxel default palette; `IMAP` presence reported; `VoxMap` with
  `Slot::{Air, Material, Skip, Unset}` and `to_prefab`; scene-graph `placements()` composing
  integer translations and rotations, with `Placement::min_corner`. Hostile input is refused
  without a panic: `MAX_MODEL_EDGE` 256, `MAX_PLACEMENTS` 65_536, `MAX_TRANSLATION` 1 << 20,
  and composed translations checked against `i32`.
- `vxe-world`: `Prefab` (dense materials plus a write mask) and `Rot` (the 24 axis-aligned
  rotations); `World::stamp`, which writes every masked voxel and returns an `EditResult`
  like an edit; `StampedSource`, a `WorldSource` that blits placements after its base, in
  order and independent of the thread count (pinned hash); the `stamp` bench.
- `apps/flythrough`: `--vault` places a shelled box ahead of the spawn, and `P` stamps one at
  the aim point through the same post-edit path as a dig.

## [0.5.0] - 2026-10-17

Plan 0004: M3, collision, a character controller and coarse grids (ADR-0005). The F2 wall
sweep reads 26.3 us against the 5 us proposal (`docs/nfr.md` section 8) and awaits the
owner's call; a body embedded by an edit is not pushed out (no depenetration).

- `vxe-sim` (new crate, depends on `vxe-world` only): a vertical-cylinder `Body`, integer
  `distance2` and `overlaps`, a conservative-advancement `sweep` that leaves out boxes the
  move does not close on and never takes an advance into a box, and a `CharacterController`
  with walk, step-up, wall slide, gravity, jump with the landing speed reported, and a
  multi-tick ledge climb; a pinned course hash in `tests/course.rs`; `isqrt`,
  `MAX_SWEEP_ITERATIONS`, `sweep_with_stats`, `distance2_with_stats`; the `controller` bench.
- `vxe-world`: `CoarseGrid` (one `CoarseCell { solid, dominant }` per brick, updated from an
  `EditResult`, returning sorted `CellChange`s, with `chunk_loaded` and `chunk_evicted` hooks
  not yet wired to the streamer) and `WalkableMap` with `WalkableParams` and
  `WalkableStats`, updated incrementally from the grid's changes; the `coarse` bench.
- `apps/flythrough`: walk mode on the bounded level (`--walk`, `G`, `W A S D` at 3 m/s,
  `Space` to jump and climb), the coarse grid and walkable map kept from every edit (`K`
  prints them), and the coarse-grid lines in `world_budget --chunks`.
- Dev profile: `opt-level = 2` for `vxe-sim`.

## [0.4.1] - 2026-10-17

Plan 0003: the edit-cost pass. `World::apply` returns the same voxels and results as before
(a brute-force oracle and both pinned hashes say so) and the `worst` edit bench drops from
about 4.6 to 3.56 ms on the dev box, still over the 2 ms figure in `docs/nfr.md` §4, which
stands; Plan 0014 is the second pass.

- `vxe-world`: edits resolve the mode once per op into a target table, write each brick row
  as a span solved from the shape, and tally counts by material id and bounds once per brick.
  Public, newly: `Shape::voxel_bounds` and `Shape::contains` (were crate-private), and
  `Shape::row_span` (new), the inclusive run of voxel `x` indices in a row whose centres lie
  inside the shape.
- Tests and benches: `reference_apply` (a brute-force edit model in test support), the
  oracle test over 200 seeded ops, a row-span property test, and the `worst`, `mixed` and
  `tool` cases of the `edits` bench.

## [0.4.0] - 2026-10-16

Plan 0005: the surface-look spike, the first M2 plan. The owner chose per-voxel contour planes
with procedural detail (ADR-0011); the fly-through's default stays face normals until the
follow-up fixes contour's shadow-ray origin and its streamed footprint, which runs out of
memory on an 8 GiB card at the default radius (`docs/nfr.md` §3).

- `vxe-world`: `Material::detail` (feature scale in voxels, strength in `[0, 1]`; default no
  detail).
- `vxe-render`: `LookParams`, `NormalMode` (`Face`, `Trilinear8`, `Gradient27`, `Contour`) and
  `ViewMode` (`Shaded`, `Normals`, `HitMask`) in `Params.flags`, set with `Marcher::set_look`;
  `GpuWorld::with_surface_attrs`, a mirror whose full chunks' bricks live in attribute pages
  of the existing pool shards, filled by the `attrib.wgsl` pass (`update_attrs`,
  `update_attrs_timed`); contour-plane hits in the marcher; procedural per-material detail in
  the shader. Breaking: `MATERIAL_FLOATS` 8 -> 16 (64-byte material rows), and `free_slots`
  returns only the plain pages' free list.
- `apps/flythrough`: keys `N` and `V`; flags `--look`, `--view`, `--detail`, `--rock-detail`;
  `look_materials`, `ROCK_DETAIL`; the look harness (`tests/look.rs`: cavity normal error,
  pillar silhouette error, the comparison over every mode) and goldens `look_trilinear8.png`,
  `look_gradient27.png`, `look_contour.png`, `look_detail.png`.

## [0.3.0] - 2026-10-16

Plan 0002: the streaming half of M1, which closes M1 with recorded misses on the streaming
and streamed primary-ray lines (`docs/nfr.md` §2).

- `vxe-world`: `Streamer`, two-tier sphere residency (full detail inside `r_full`, an
  LOD-only ring of mips out to `r_lod`, with hysteresis) on a `std::thread` worker pool
  (ADR-0003); edited-chunk persistence in RAM with checked disk spill files; `ChunkMips` under
  a conservative majority rule; a dirty-chunk set on the store; `World::apply` excludes
  eviction for the whole op; unbounded worlds through `World::new`.
- `vxe-render`: a mirror window that follows the camera with a nearest-first, byte-budgeted,
  batched `sync`; 8-bit per-chunk palettes with a wide fallback; the brick pool sharded over
  four storage bindings sized from adapter limits (ADR-0004); LOD records and a mip-level
  walk in the marcher; `MirrorError::RecordsTooLarge` beside `PoolTooLarge`;
  `GpuWorld::allocated_bytes` beside the live `mirror_bytes`. Breaking: `SLOT_BYTES` replaces
  `BRICK_BYTES`, `pool_shard` replaces `pool`, `Params` changes layout.
- `apps/flythrough`: the unbounded streaming cave by default (`--level` for the bounded one),
  a headless autopilot that reads streaming CPU, residency latency, mirror bytes (live and
  allocated) and peak RSS, `world_budget --stream`, and the `lod.png` golden.
- Dev profile: `opt-level = 2` for `vxe-world` and `flythrough`.

## [0.2.0] - 2026-10-16

Plan 0001: the bounded half of M1.

- `vxe-world`: a bounded brick-map world (`u16` voxels, uniform-or-full 8³ bricks, 64³
  chunks under per-chunk locks, ADR-0002), a material table with an indestructible flag,
  integer edit ops (sphere, box, capsule, tool-along-ray; carve, fill, replace) that return
  per-material counts, touched chunks and bounds, an integer raycast, point and box queries,
  brick occupancy masks, and FNV-1a and value-noise helpers.
- `vxe-render`: a GPU mirror of the world (brick pool, masks, chunk records, chunk table)
  uploaded per dirty brick in batched runs, and a WGSL compute ray marcher with chunk, brick,
  block and voxel steps, an offset headlamp and one shadow ray.
- `vxe-app`: a `winit` shell with a surface, input and a frame loop.
- `apps/flythrough`: a seeded integer-noise cave, the fly-through with dig, fill, a carve
  benchmark and frame readings, a cross-platform determinism vector, and golden images on
  lavapipe.

## [0.1.0] - 2026-10-16

- Repository bootstrapped: workspace skeleton, harness, ADR-0001, roadmap and NFR budgets.
  No engine code yet.
