# Non-functional requirements

The budgets the engine is held to. Every "fast", "large" and "stable" claim elsewhere means one
of the figures below. Each figure is a **proposal until a measurement replaces it**; a
measurement names the commit, the machine, the resolution and the scene. A plan that touches
storage or the frame states which figures it affects and measures them at its close.

Written 2026-10-16. Readings are added under the figure they measure, each naming its source.

## 1. Hardware

| | Baseline (the floor) | Dev reference |
|---|---|---|
| GPU | 2019 mid-range desktop: GTX 1660 / RX 5600 class, 6 GB VRAM, Vulkan 1.2, no RT hardware | NVIDIA RTX 3080 Laptop, 8 GB, driver 610 |
| CPU | 4 cores / 8 threads | 16 threads (AMD, Cezanne) |
| RAM | 16 GB | 30 GB |
| OS | Linux (Vulkan), Windows (Vulkan or DX12 through wgpu) | Arch Linux, Wayland |

A perf figure measured only on the dev reference is labelled as such. The baseline card is to be
acquired or borrowed before M2 closes; until then the baseline numbers are projections.

## 2. Frame budget

Target: **60 fps at 1080p output** on the baseline, with the upscaler allowed (internal
resolution between 720p and 1080p). Proposed split of the 16.6 ms, to be rebalanced by
measurement:

| Stage | Budget |
|---|---|
| Streaming, upload, light extraction (CPU-side and copies) | 1.0 ms |
| Primary rays (DDA, full detail + LOD) | 3.0 ms |
| Lighting (path trace 1 spp, next-event estimation, shadow rays) | 6.0 ms |
| Denoise (temporal + spatial) | 3.0 ms |
| Upscale, tone map, post | 1.5 ms |
| Headroom (entities, volumetrics, game work) | 2.1 ms |

**Primary rays, dev reference (not the baseline):** 1.59 ms with brick occupancy masks, 1.99 to
2.01 ms without them. Timestamp queries around the march pass, median of the last 30 of 300
frames, `flythrough --seed 1 --render 1920x1080 --frames 300` from the spawn pose, three runs
each. The pass includes the headlamp's one shadow ray per lit pixel, so it covers more than the
primary-ray line. NVIDIA GeForce RTX 3080 Laptop GPU, Vulkan, driver 610.57.04 (Plan 0001
Phase 8). No baseline reading yet (§1).

**Streaming line, first observation (dev reference, Plan 0001 close, 1d61aba):** the
fly-through's `B` key (100 radius-1 m carves near the camera, then one `GpuWorld::sync`)
uploaded 1.2 to 2.0 MB in 3.9 to 12.9 ms of render-thread CPU over about 20 runs, against
the 1.0 ms line. Part of that is `sync` visiting every chunk; Plan 0002 Phase 3 adds a
dirty-chunk set and a byte budget. Same session, `F1` read the march pass at 3.8 to 8.1 ms at
2530 × 1378 away from the spawn pose: views differ by more than 2× from the spawn reading
above. RTX 3080 Laptop GPU, Ryzen 9 5900HS.

**Streaming line under motion (dev reference, Plan 0002 Phase 8, headless `flythrough
--autopilot`, unbounded cave seed 1, r_full 16, r_lod 24, h 1, 15 workers, 2 MiB sync budget,
RTX 3080 Laptop GPU, Ryzen 9 5900HS):** render-thread `update + poll + sync` per frame over a
60 s flight at 10 m/s, two runs: p50 7.01 and 6.97 ms, p99 57.8 and 60.7 ms, max 83.1 and
105.6 ms, against the 1.0 ms line. Most frames spend it in `sync` narrowing and uploading
about 2 MiB; the p99 frames are the ones whose focus chunk changed (queue rebuild, residency
sweep and window shift over about 58 000 chunks). At rest after full residency a frame's CPU
reads 0.35 ms. The march pass from the unbounded cave's spawn at 1920 x 1080 reads 6.69 ms at
rest (headless, timestamps), against 1.95 ms from the bounded level's spawn at the same commit.

No frame may stall on paging: work attributable to streaming on the render thread is capped at
the streaming line above; the rest runs on background threads.

## 3. Memory

| | Budget |
|---|---|
| World in VRAM at the full-detail radius (about 100 m) plus LOD to about 1 km | 3 GB |
| World in RAM (resident chunks, edit persistence for paged-out chunks) | 4 GB |
| Everything else in VRAM (frame targets, history, entities, upscaler) | 1.5 GB |

The world figure rests on the uniform-brick assumption in `roadmap.md` (under 10 % of bricks
non-uniform at full detail). When exceeded, far chunks drop to LOD-only residency rather than
the frame dropping.

**Readings (dev reference, 2026-10-16, Plan 0001 Phase 4 commit, `flythrough` cave seed 1,
`cargo run --release -p flythrough --example world_budget`):**

| Extent (chunks) | `Full` fraction | CPU bytes | Mirror bytes (no LOD) |
|---|---|---|---|
| 20 × 20 × 8 (the level, 128 × 128 × 51 m) | 23.41 % | 399.6 MiB | 383.8 MiB |
| 32 × 32 × 32 (204.8 m cube) | 20.79 % | 3661.6 MiB | 3496.2 MiB |

With occupancy masks (Plan 0001 Phase 8), the level-size mirror reads 402 449 984 B from the
fly-through's first sync. Both measured `Full` fractions are above the 10 % assumption above; the cave is about 29 %
air with caverns on a 6.4 m lattice.

With 8-bit palette indices (Plan 0002 Phase 4, ADR-0004) the level-size mirror reads
207 681 088 B: pool 196 407 296 B (512 B for each of the 383 608 `Full` bricks, no wide chunk),
masks 8 B per brick, records 2560 B per chunk (512 entries and a 256-entry `u16` palette),
table 4 B per chunk, from `apps/flythrough/tests/level_mirror.rs` on the dev box.

**Streamed residency (dev reference, Plan 0002 Phase 8, unbounded cave seed 1 around its spawn,
hysteresis 1, `cargo run --release -p flythrough --example world_budget -- --stream
R_FULL,R_LOD`, 16 workers, Ryzen 9 5900HS).** Mirror bytes at 8-bit palette indices (ADR-0004):
512 + 8 B per `Full` brick or mip-brick, 2560 B per record for full and LOD-only chunks alike,
plus the `W^3` chunk table. CPU bytes: full chunks' slot arrays and `Full` bricks, LOD-only
chunks' mip-brick slots and `Full` mip-bricks. In every row about 18.5 to 18.8 % of a full
chunk's bricks and about 39 % of the ring's mip-bricks are `Full`.

| r_full | r_lod | Full chunks | LOD-only chunks | CPU MiB | Mirror MiB (full) | Mirror MiB (LOD) | Mirror MiB (total) |
|---|---|---|---|---|---|---|---|
| 10 | 14 | 4 169 | 7 344 | 634.2 | 205.7 | 123.4 | 329.2 |
| 10 | 18 | 4 169 | 20 236 | 1 006.2 | 205.7 | 336.0 | 541.9 |
| 12 | 16 | 7 153 | 9 924 | 1 012.7 | 355.9 | 165.5 | 521.5 |
| 12 | 20 | 7 153 | 26 248 | 1 486.2 | 355.9 | 436.0 | 792.2 |
| 16 | 20 | 17 077 | 16 324 | 2 210.0 | 855.1 | 270.5 | 1 126.0 |
| 16 | 24 | 17 077 | 40 700 | 2 915.9 | 855.1 | 673.9 | 1 529.5 |

Under motion (headless `flythrough --autopilot`, r_full 16, r_lod 24, two runs) the mirror
peaked at 1 686 395 092 and 1 696 279 292 B, resident RAM at 3 243 445 888 and
3 239 272 240 B, and persisted RAM at 233 616 B.

## 4. Edits

- **Cost:** 100 sphere carves of radius 1 m (exactly 4169 voxels each at 10 cm, the lattice
  count on voxel centres; the continuous volume is about 4189) in one frame cost at most 2 ms
  of CPU including result counting and dirty marking, and their upload (up to about 6400 bricks,
  6.4 MB, if every touched brick is non-uniform) fits in the streaming line of §2.
  **Readings (dev reference, AMD Ryzen 9 5900HS, Plan 0001):** `cargo bench -p vxe-world
  --bench edits` (100 radius-10 carves on a fresh solid world, every carve expanding uniform
  bricks, the worst case): 4.92 ms (CI 4.87 to 4.98 ms), Phase 4 commit. In the fly-through
  (`B` key, carves into an already dug cave, 1d61aba): 3.07 to 4.63 ms over about 20 runs.
  Both miss the 2 ms budget; the budget stands (owner's call at the Plan 0001 close) and the
  edit cost gets its own plan after Plan 0002.
- **Results** are returned synchronously with the edit, per material, exact.
- **Determinism:** the same op list on the same seed yields bit-identical voxels and results on
  Linux and Windows. CI asserts a fixed vector's hash on both arms. **Asserted:**
  `determinism_vector` passed on the Linux and Windows CI arms at 9565c5a and 1d61aba
  (Plan 0001), with the hash blessed on Linux.

## 5. Streaming

- Chunk generation and LOD building run on background threads; the render thread only swaps
  pointers and issues uploads.
- Moving the focus point at 10 m/s keeps the full-detail radius resident; a chunk entering the
  radius is resident within 500 ms (proposal). At a 100 m radius that is about 1200 chunks per
  second, roughly 300 M voxels per second across the baseline's four cores, so generation
  throughput per core is a measured figure from Plan 0002 on and the radius default follows it.
  **Reading (dev reference, AMD Ryzen 9 5900HS, 16 threads, 2026-10-16, Plan 0001 Phase 4
  commit):** the `flythrough` cave generates at 1.03e8 voxels per second per thread at
  20 × 20 × 8 chunks (0.509 s) and 1.12e8 at 32 × 32 × 32 (4.785 s), bounded and synchronous,
  from `world_budget`.
- An edited chunk that is evicted and later reloaded reproduces the edit.
  **Asserted** headlessly by `crates/vxe-world/tests/persist.rs` (RAM, spill file, racing
  eviction) and under motion by `flythrough --autopilot` (`return: 2 chunks, hash match`
  after flying 300 m out and back), Plan 0002 Phase 8.
- **Reading under motion (dev reference, Plan 0002 Phase 8, headless `flythrough
  --autopilot`, unbounded cave seed 1, r_full 16, r_lod 24, 15 workers, RTX 3080 Laptop GPU,
  Ryzen 9 5900HS):** at 10 m/s, 0 frames out of 2775 and 2772 had a chunk wanted full within
  14 chunks of the camera's not yet resident and mirrored. Wanted-to-mirrored latency of the
  73 000 chunks loaded full: p95 462 and 455 ms, max 1779 and 2359 ms. Generation under
  streaming: 6.95e7 voxels/s/worker over the flight (both tiers, 15 workers beside the render
  thread), against 1.23e8 to 1.34e8 headless in `world_budget --stream`.

## 6. Determinism and headless operation

- `vxe-world` builds and tests without a GPU, a window or a display server.
- No wall-clock reads in `vxe-world` or in server-side code; `clippy.toml` enforces it.
- Everything the engine generates takes an explicit seed. There is no engine-owned global RNG.
- Render tests run on the software adapter (lavapipe) in CI; golden baselines are blessed there
  and real-vendor readings are recorded separately, never substituted.

## 7. Startup and build

- Time to first frame in `apps/flythrough` under 3 s on the dev reference (proposal).
  **Reading (Plan 0001, 112cfa0, level-size bounded cave seed 1, 3200 chunks, RTX 3080 Laptop
  GPU, Ryzen 9 5900HS, Hyprland):** 2226 to 2301 ms cold (dGPU suspended before launch),
  1063 to 1187 ms warm; device creation dominates cold start (1838 to 1903 ms), the mirror
  upload is 349 to 357 ms for 402 MB. 2282 ms on the owner's interactive run at 1d61aba.
  **Reading (Plan 0002 Phase 8, streaming unbounded cave seed 1, headless on the RTX 3080
  Laptop GPU, so no window or surface):** the first frame waits for the 33 chunks within
  squared distance 4 of the spawn chunk: first frame at 1491 and 457 ms (device ready at 1268
  and 417 ms); full residency (17 077 full, 40 700 LOD-only chunks) at 12 798 and 12 685 ms.
- An incremental `cargo build` of a one-crate change under 10 s on the dev reference; the
  everyday loop does not compile crates a plan does not touch (`default-members`).

## 8. Not goals (for now)

- Binary size.
- Mobile, web or console targets.
- Frame rates above 60 fps as a gate; they are welcome, not required.
