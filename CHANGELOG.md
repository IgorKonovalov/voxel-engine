# Changelog

One entry per closed plan, written at the architect's close ceremony together with the version
bump in `Cargo.toml` (`workspace.package.version`). Format: [Keep a Changelog](https://keepachangelog.com/),
SemVer 0.x (any minor may break the API until 1.0).

## [Unreleased]

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
