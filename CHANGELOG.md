# Changelog

One entry per closed plan, written at the architect's close ceremony together with the version
bump in `Cargo.toml` (`workspace.package.version`). Format: [Keep a Changelog](https://keepachangelog.com/),
SemVer 0.x (any minor may break the API until 1.0).

## [Unreleased]

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
