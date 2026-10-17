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

**Streaming line under motion: a recorded miss (dev reference, Plan 0002 Phase 8 at 05334d7
and the close review at ffc73e8, headless `flythrough --autopilot`, unbounded cave seed 1,
r_full 16, r_lod 24, h 1, 15 workers, 2 MiB sync budget, RTX 3080 Laptop GPU, Ryzen 9
5900HS):** render-thread `update + poll + sync` per frame over a 60 s flight at 10 m/s, four
runs: p50 6.94 to 7.13 ms, p99 51.8 to 60.7 ms, max 83.1 to 119.8 ms, against the 1.0 ms line.
The line stands (owner's ruling at the Plan 0002 close, 2026-10-16); a streaming-CPU plan is
the next work against it. Most frames spend it in `sync` narrowing and uploading
about 2 MiB; the p99 frames are the ones whose focus chunk changed (queue rebuild, residency
sweep and window shift over about 58 000 chunks). At rest after full residency a frame's CPU
reads 0.35 ms.

**Primary rays in the streamed world: a recorded miss (dev reference, Plan 0002 Phase 8,
05334d7, reproduced at ffc73e8):** the march pass from the unbounded cave's spawn at
1920 x 1080 reads 6.69 ms at rest after full residency (headless, timestamps, median of the
last 30 of 300 frames), against the 3.0 ms line and against 1.95 ms from the bounded level's
spawn at the same commit. The line stands (owner's ruling, 2026-10-16); a march-pass diagnosis
spike runs before M2's lighting. The autopilot's flight reading (p50 0.46 ms) is not evidence
for the line: its +x path flies through rock.

**Primary rays per surface-look mode (dev reference, Plan 0005 Phase 5, 37b22fa, headless
`flythrough --level --seed 1 --render 1920x1080 --frames 300 --look MODE [--detail]` from the
spawn pose, median of the last 30 of 300 frames, three runs each, NVIDIA GeForce RTX 3080
Laptop GPU; taken while the machine's 1-minute load read between 3 and 5):** face 1.876 to
1.877 ms (1.826 to 1.859 ms without `--look`, so without attribute pages), face with detail
1.976 to 1.995 ms, `Trilinear8` 2.874 to 2.907 ms (with detail 3.020 to 3.051), `Gradient27`
4.564 to 4.612 ms (with detail 4.583 to 4.675), `Contour` 1.927 to 1.944 ms (with detail
2.068 to 2.069). The march pass includes the headlamp's shadow ray. ADR-0011 chooses `Contour`
with detail; on the baseline (about two to three times slower) the level's contour reading
projects to roughly 3.9 to 5.8 ms against the 3.0 ms line, a projection, not a reading.

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

**Under motion (dev reference, Plan 0002 fix pass, 076b1c4, headless `flythrough --autopilot`,
unbounded cave seed 1, r_full 16, r_lod 24, h 1, 15 workers, RTX 3080 Laptop GPU, NVIDIA
610.57.04, Ryzen 9 5900HS, two runs):**

- **World VRAM:** the mirror peaked at 1 672 823 572 and 1 692 743 692 B by live accounting
  (`GpuWorld::mirror_bytes`) and at 2 431 425 836 B allocated (`GpuWorld::allocated_bytes`,
  both runs), against the 3 GB line. The budget is read on allocated bytes: buffers grow by
  half again and never shrink (ADR-0004 Outcome). At full residency, 1 603 820 012 B live and
  1 675 304 748 B allocated.
- **World RAM:** resident chunks account for 3 242 091 312 and 3 239 741 232 B at peak (slot
  arrays, `Full` bricks and mips), and persisted RAM for 233 616 B. The process peak RSS
  (`VmHWM`, the whole fly-through, dev reference) reads 3 869 564 928 and 3 873 890 304 B,
  about 3.87 GB against the 4 GB line. RAM is the tightest margin at these defaults; they are
  revisited on the baseline card at M2.

**Attribute pages (dev reference, Plan 0005, RTX 3080 Laptop GPU).** The contour attribute
(`u16` per voxel) lives in attribute pages of the pool: 1040 B more live per `Full` brick of a
full chunk, and a page holds 1364 bricks in 4096 slots. At the level (`flythrough --level
--headless --seed 1 --frames 1`, 6d77f12): 207 681 088 B live and 213 334 144 B allocated
without attribute pages; 606 633 408 B live and 613 759 104 B allocated with them
(`--look contour`), a live rise of exactly 1040 x 383 608 B. 128 B more with 64-byte material
rows at 37b22fa. Streamed (`flythrough --autopilot --headless --look contour`, 37b22fa, r_full
16, r_lod 24, h 1, 15 workers): at full residency 3 309 723 820 B live and 4 103 413 164 B
allocated, over the 3 GB line, against Plan 0002's 1 675 304 748 B allocated in face mode; the
run then stopped in its outbound flight on `wgpu error: Out of Memory` on the 8 GiB card, so
no peak was read. A recorded miss: ADR-0011 keeps contour planes, and fitting this footprint
under the 3 GB line is owed to the surface follow-up plan (Plan 0005 follow-up 2).

**Prefabs (arithmetic, Plan 0008; no reading needed).** A `Prefab` holds 2 B of material plus
one mask bit per voxel: a 16³ prefab is 8 KiB + 512 B, a 64³ one 512 KiB + 32 KiB. A parsed
`.vox` model is capped at 256³ per axis (`MAX_MODEL_EDGE`), so the largest `to_prefab`
allocation is about 50 MiB including its index buffer. `StampedSource` holds one rotated copy
of the prefab per placement, so its RAM grows with the placement count, not the prefab count.

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
  **Reading (dev reference, AMD Ryzen 9 5900HS, Plan 0003, 96afb82):** the same `worst` case
  reads 3.656 ms (CI 3.641 to 3.672 ms) with the dev box under outside load (load average
  about 10), against 4.548 to 4.641 ms for the Plan 0001 edit path run alternately in the same
  window; the fly-through `B` line (`--level --seed 1`) reads 3.33 to 3.41 ms against 4.42 to
  4.46 ms. **Quiet-box reading at the Plan 0003 close** (a958ec1, same `vxe-world` tree,
  exclusive perf lock, 1-minute load under 3.5 and falling, nothing else running): `worst`
  3.556 to 3.566 ms (CI 3.552 to 3.572 ms over two runs), `mixed` 2.586 to 2.596 ms, `tool`
  8.119 to 8.120 ms; the fly-through `B` line 3.30 to 3.34 ms. It misses the 2 ms budget; the
  budget stands (owner's call at the Plan 0003 close) and Plan 0014 is the second pass.
- **Results** are returned synchronously with the edit, per material, exact.
- **Stamp cost (proposals, Plan 0008):** a 16³ prefab stamp (the shelled box `Prefab::vault`)
  into solid rock costs under 100 µs, and a 64³ all-written prefab under 5 ms, both including
  the `EditResult` tally. **Readings (dev reference, AMD Ryzen 9 5900HS, `cargo bench -p
  vxe-world --bench stamp`, exclusive perf lock, box under outside load of about 2 to 5,
  commits 83ab811 and 39d3700):** `vault16` 22.6 to 22.8 µs; `prefab64_mixed` 2.02 to 2.07 ms;
  `prefab64_uniform` 1.455 ms. Both under their proposals; the proposals stand as budgets.
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
- **Residency latency (dev reference, Plan 0002 Phase 8, 05334d7, and the close review at
  ffc73e8, headless autopilot as below):** wanted-to-mirrored p95 431 to 481 ms over four
  runs, under the 500 ms proposal; max 1678 to 3669 ms, over it. The proposal stands as a p95
  figure; the tail is the streaming-CPU plan's.
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
- Chunk mips join the determinism vector (`apps/flythrough/tests/determinism.rs`, Plan 0002):
  the mips hash `0x6cc1_4fda_4b7f_5188` is blessed on Linux. The Windows arm confirms it on
  the first `main` CI run after Plan 0002's merge; a mismatch is a defect in an integer path,
  fixed forward.
- Generation with prefab placements (`StampedSource`, Plan 0008) is independent of the thread
  count: `generate_with_threads` at 1 and 7 threads gives the same hash, and
  `crates/vxe-world/tests/stamp.rs` pins it at `0x7ee7_eeb7_0956_d069`, blessed on Linux; the
  stamp is integer-only, so the Windows arm must reproduce it.

## 7. Startup and build

- Time to first frame in `apps/flythrough` under 3 s on the dev reference (proposal).
  **Reading (Plan 0001, 112cfa0, level-size bounded cave seed 1, 3200 chunks, RTX 3080 Laptop
  GPU, Ryzen 9 5900HS, Hyprland):** 2226 to 2301 ms cold (dGPU suspended before launch),
  1063 to 1187 ms warm; device creation dominates cold start (1838 to 1903 ms), the mirror
  upload is 349 to 357 ms for 402 MB. 2282 ms on the owner's interactive run at 1d61aba.
  **Reading (Plan 0002 Phase 8, streaming unbounded cave seed 1, headless on the RTX 3080
  Laptop GPU, so no window or surface):** the first frame waits for the 33 chunks within
  squared distance 4 of the spawn chunk: first frame at 1491 and 457 ms (device ready at 1268
  and 417 ms); full residency (17 077 full, 40 700 LOD-only chunks) at 12 798 and 12 685 ms,
  and at 12 134 and 11 697 ms at the fix-pass commit (076b1c4). The close review's cold run
  (ffc73e8) read device ready 1968 ms and first frame 2075 ms. These runs create no window or
  surface, so a windowed first frame adds that cost.
  **Level-size first sync after narrowing (Plan 0002 Phases 5 and 8, RTX 3080 Laptop GPU):**
  497 to 517 ms, up from Plan 0001's 349 to 357 ms for the `u16` mirror; the palette scan
  before indices are written. Accepted at the Plan 0002 close (owner, 2026-10-16); TTFF stays
  under 3 s.
- An incremental `cargo build` of a one-crate change under 10 s on the dev reference; the
  everyday loop does not compile crates a plan does not touch (`default-members`).

## 8. Collision and coarse grids

Added at Plan 0004's close (2026-10-17). `vxe-sim` sweeps and the character controller,
`vxe-world`'s coarse grid and walkable map. All of it is integer and joins §6: the controller
course hash `0x0c80_8031_d14f_27eb` (`crates/vxe-sim/tests/course.rs`) is blessed on Linux and
the Windows arm confirms it on the first `main` CI run after Plan 0004's merge.

| Figure | Budget | Measured by |
|---|---|---|
| Controller tick, one body | 20 µs | `cargo bench -p vxe-sim --bench controller` |
| Sweep, one body, one contact | the measured figure (see the reading; proposal was 5 µs) | same bench, `1000 sweeps of the F2 wall case` |
| Coarse grid update after a radius-1 m carve | 50 µs | `cargo bench -p vxe-world --bench coarse` |
| Walkable update after that change | 20 µs | same bench |
| Coarse grid RAM | 4 B per brick, 2048 B per chunk | `world_budget --chunks` |

**Readings (dev reference, quiet box, AMD Ryzen 9 5900HS laptop, Linux, commit b94622a (the
merge of `main` into Plan 0004's branch; `vxe-sim` identical to cf3bbc6), `cargo bench`,
criterion median (low, high), exclusive perf lock, load average 1.7 to 2.3 at each start):**

- **Controller tick:** `60 ticks of the F6 staircase, all on terrain` 156.01 µs (155.84,
  156.21), **2.60 µs per tick**; `300 ticks of the F6 course` 522.16 µs, 1.74 µs per tick;
  `1000 ticks of the F6 course` 772.89 µs, 0.77 µs per tick (most of those ticks are free fall
  past the fixture's extent). Under the 20 µs line. The worst case, a 64-iteration grazing
  corner sweep inside a tick, is not benched.
- **Sweep:** `1000 sweeps of the F2 wall case` 26.303 ms (26.295, 26.312), and 26.264 ms on
  a second run: **26.3 µs per sweep**, against the 5 µs proposal. The owner accepted the
  measured figure as the line at 17.5 µs (fe50d6d); the walk-check fix pass (closes-on filter,
  overlap guard, rounded-up step length; ADR-0005 Outcome) raised it to 26.3 µs. **Above
  20 µs: awaits the owner's call** whether 26.3 µs stands as the line or a sweep-cost
  followup is scheduled.
- **Coarse grid update** after a radius-10 (1 m) carve on the solid world: 100 updates
  1.3544 ms (1.3528, 1.3560), **13.5 µs per update**. **Walkable update** from those changes:
  807.17 µs (805.83, 808.64), **8.1 µs per update**. Both under their lines. Plan 0003's
  `mixed` world was not on the branch; the bench runs on the solid world.
- **Coarse grid RAM:** 6 553 600 B at the level (3200 chunks, `flythrough --level --seed 1`,
  fe50d6d). At 32 × 32 × 32 chunks it is 67 108 864 B by the same 2048 B per chunk
  (arithmetic, not read). Grid build at the level, one thread (`world_budget --chunks 20x20x8
  --seed 1`): 156.3 ms at the close review; grid plus walkable map in the fly-through 231 ms.

## 9. Not goals (for now)

- Binary size.
- Mobile, web or console targets.
- Frame rates above 60 fps as a gate; they are welcome, not required.
- VR on the baseline. The VR-readiness decision in `roadmap.md` keeps the renderer's doors
  open; a VR plan adds its own profile here (two views, 90 Hz, a named card) when scheduled.
