# Non-functional requirements

The budgets the engine is held to. Every "fast", "large" and "stable" claim elsewhere means one
of the figures below. Each figure is a **proposal until a measurement replaces it**; a
measurement names the commit, the machine, the resolution and the scene. A plan that touches
storage or the frame states which figures it affects and measures them at its close.

Written 2026-10-16. Readings are added under the figure they measure, each naming its source.

## 1. Hardware

| | Baseline (the floor) | Dev reference | Slow adapter (on hand) | Portable profile (a quality profile, not a gate) |
|---|---|---|---|---|
| GPU | 2019 mid-range desktop: GTX 1660 / RX 5600 class, 6 GB VRAM, Vulkan 1.2, no RT hardware | NVIDIA RTX 3080 Laptop, 8 GB, driver 610 | the dev box's AMD Cezanne iGPU (Radeon Vega, RADV on Mesa, Vulkan 1.4): about 2 TFLOPS, DDR4 shared with the CPU, below the floor's class | Switch-2 class: about 1.7 to 3 TFLOPS, 68 to 102 GB/s, published figures; no device on hand |
| CPU | 4 cores / 8 threads | 16 threads (AMD, Cezanne) | the same | 8 ARM cores class |
| RAM | 16 GB | 30 GB | the same, shared with the GPU | 9 GB shared by CPU and GPU |
| OS | Linux (Vulkan), Windows (Vulkan or DX12 through wgpu) | Arch Linux, Wayland | the same | none targeted; the profile is held on a PC of that class |

A perf figure is labelled by the adapter it was read on. **The baseline card is not acquired
(owner, 2026-10-21; `roadmap.md`, "Baseline card")**: the floor's figures stay projections,
bracketed by two readings on the dev box, the NVIDIA card above the floor and the slow adapter
below it. From Plan 0028 on, every render plan's close takes its GPU readings on both adapters
(the fly-through's `--adapter` flag, Plan 0028 Phase 4). A figure read on the NVIDIA card alone
is labelled "dev reference" as before.

The portable profile (owner, 2026-10-17; `roadmap.md`, "Floor, profiles and scene scale") is
a named setting of the quality knobs (internal resolution, full-detail radius, translucent
layers, lighting tier) at 720p output and 30 to 60 fps. No machine of that class is on hand;
the slow adapter above is the nearest thing to it and its readings stand in, labelled as the
slow adapter's, beside the storage reads per ray that Plan 0015 counts, against about three
uncached 64-byte lines per ray at 720p and 60 fps with three rays a pixel (arithmetic from
the published bandwidth).

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

The upscale line is over under the fly-through's FSR 2 default since Plan 0055: 2.932 to 2.958 ms on
the RTX 3080 Laptop, 9.967 to 9.983 ms on the RADV RENOIR. The close keeps the line and records the
reading; the default stands on the owner's eye. At Plan 0065's close (03b00ab, 2530 x 1378 output)
FSR 2's passes read 2.103 to 2.424 ms on the RTX and 15.691 to 16.058 ms on the RADV RENOIR, still
over; the lamp-on a-trous at the full lighting tier (3.088 to 3.321 ms RTX) is over the denoise line.

Two lines inside the streaming line (Plan 0067, 2026-11-12; RTX 3080 Laptop and RADV RENOIR,
headless, quiet, against the start commit 4bc8100):

- **The live bake queue's step** (render thread, every frame). At rest it drains and costs
  0.075 ms p50 on the RTX (0.283 on the RENOIR while still refining) against 3.26 and 3.79 ms
  of the start commit's opening loop; the frame CPU at rest reads 1.68 and 1.12 ms p50. In a
  flight it depends on the regime: about 1.0 to 1.5 ms p50 when it is starved (A), 9 to 13 ms
  when it keeps up (B, nearly every step opening), with p99 16.5 to 21.4 ms on the RTX against
  a proposed 16.6. Under the default missing light (`chain`) a B step pays about 1.7 ms of
  first-block fallback fades and summaries.
- **The bake budget** is `frame:1.5`, a 1.5 ms slice a frame (ADR-0052's unit stays available
  as `second:MS,CAP`): about 40 ms of bake a wall second on the RTX and 30 on the RENOIR in the
  streamed glide.

Every reading against this budget, with its machine, scene and commit, is in
[`docs/nfr/02-frame-budget.md`](nfr/02-frame-budget.md); new readings go there.

## 3. Memory

| | Budget |
|---|---|
| World in VRAM at the full-detail radius (about 100 m) plus LOD to about 1 km: live bytes against the reserved budget | 3 GB |
| World in RAM (resident chunks, edit persistence for paged-out chunks) | 4 GB |
| Everything else in VRAM (frame targets, history, entities, upscaler) | 1.5 GB |

The world figure rests on the uniform-brick assumption in `roadmap.md` (under 10 % of bricks
non-uniform at full detail). When exceeded, far chunks drop to LOD-only residency rather than
the frame dropping.

Every reading against this budget, with its machine, scene and commit, is in
[`docs/nfr/03-memory.md`](nfr/03-memory.md); new readings go there.

## 4. Edits

Every reading against these budgets, with its machine and commit, is in
[`docs/nfr/04-edits.md`](nfr/04-edits.md); new readings go there.

- **Cost:** 100 sphere carves of radius 1 m (exactly 4169 voxels each at 10 cm, the lattice
  count on voxel centres; the continuous volume is about 4189) in one frame cost at most 2.0 ms
  of CPU on the dev reference (raised to 2.2 ms by the owner on 2026-10-20 after the quiet-box
  re-read in the readings file, and returned to 2.0 ms on 2026-10-21 after Plan 0031's quiet-box reading;
  **met on the dev box**, no baseline-GPU reading) including result counting and dirty marking, and their upload (up to about 6400 bricks,
  6.4 MB, if every touched brick is non-uniform) fits in the streaming line of §2.
  Readings in `docs/nfr/04-edits.md`.
- **Results** are returned synchronously with the edit, per material, exact.
- **Stamp cost (proposals, Plan 0008):** a 16³ prefab stamp (the shelled box `Prefab::vault`)
  into solid rock costs under 100 µs, and a 64³ all-written prefab under 5 ms, both including
  the `EditResult` tally.
  Readings in `docs/nfr/04-edits.md`.
- **Collapse cost (proposals, Plan 0009):** `find_unsupported` after a radius-10 carve at the
  centre of a solid 4³-chunk world (every flood runs to the cap) under 1 ms; `drop_region` of
  the 9-cell test slab under 100 µs; `remove_region` of that slab with rubble 1/4 under
  200 µs; `drop_region` of a 4096-cell cube (2 097 152 voxels) under 20 ms (a guess at the
  proposal). **Readings (dev reference, AMD Ryzen 9 5900HS, Linux, `cargo bench -p vxe-world
  --bench collapse`, criterion medians, exclusive perf lock; at b0d5565 with 1-minute load
  4.57 falling to 2.18, and at 09556ed after the brick-array pool merge with load 11.75 falling
  to 4.18):** `find_unsupported` 327 to 396 µs; slab drop by 32 72 µs and by 10 (not a
  multiple of the brick edge) 49 to 50 µs; `remove_region` 119 to 127 µs; cube drop by 64
  6.8 to 6.9 ms and by 60 8.1 to 8.6 ms; cube drop by 8 5.15 ms (no proposal). In the
  fly-through, the query after a 0.5 m dig on the seed-1 level reads 492 to 619 µs over six
  digs (release build, 25b483d; no proposal). Every reading is under its proposal; the
  proposals stand as budgets.
  **Yielding drop and rubble pile (proposals, Plan 0022, ADR-0016):** `drop_region_through` of
  the 9-cell slab through a 32-voxel sliver under the slab's 100 µs; of the 4096-cell cube at
  yield 64 under the cube's 20 ms; `pile_rubble` of 32 on the 9-cell slab under the removal's
  200 µs (a pile is the removal's second half). **Readings (dev reference, AMD Ryzen 9 5900HS,
  Linux, same bench, criterion medians, exclusive perf lock, at 7e42bbf, 1-minute load 2.2 to
  6.8, three runs):** slab through the sliver 91.5 to 94.7 µs; yielding cube 7.4 to 10.6 ms;
  pile 35.4 to 37.9 µs. In the same lock hold, paired with the pre-plan build (ec99160, load 4.5
  to 9.1), the plan build reads within noise of or under it on every older arm:
  `find_unsupported` 337 to 395 µs against 393; slab by 32 78 to 82 µs against 84 to 89; by 10
  57 µs against 62 to 68; `remove_region` 154 to 160 µs against 153 to 161; cube by 64 7.4 to
  8.7 ms against 10.2 to 11.0. Every reading is under its proposal; the proposals stand as
  budgets.
- **Logs (Plan 0007, Plan 0018):** an op log entry is 47 bytes for a sphere carve (65 for a box
  fill, 75 for a capsule replace), 34 for a stamp (whatever the prefab's size: it names the
  prefab by content id, ADR-0014), `21 + 12n` for a drop of `n` cells and `31 + 12n` for a
  removal (49 173 bytes for a 4 096-cell drop), `23 + 12n` for a drop through sub-threshold
  cells (kind 5, Plan 0022: 131 bytes for 9 cells, 49 175 for 4 096) and `27 + 12n` for a
  rubble pile (kind 6: 135 bytes for 9 cells); a boundary entry 13, an input entry 14 bytes
  plus its payload. A log file adds a header of 60 fixed bytes, a 4-byte prefab count, and
  per prefab its stamps name an 8-byte id and its content (`12 + 2n` bytes for `n` voxels plus
  8 per 64 voxels of mask: 8 716 for the 16³ vault), once per prefab however often it is
  stamped. A falling region re-sends its cells every tick it drops (129 bytes a tick for a
  9-cell slab, 131 as kind 5), and a fall that crushed anything adds one kind-6 pile entry at
  its end. All asserted in `crates/vxe-world/tests/log.rs`.
- **Replay throughput:** no budget; a reading only, bound by the edit cost above (the ops
  dominate, not the decode).
  Readings in `docs/nfr/04-edits.md`.
- **Tick overhead:** 100 inputs through one server tick and one shared-client poll cost at
  most 100 µs more than applying the same 100 ops directly (budget since the Plan 0007 close,
  backed by the reading below).
  Readings in `docs/nfr/04-edits.md`.
- **Component boundary (budgets since the Plan 0011 close):** a host call from a logic
  component (the `apply` import of a radius-1 m carve: the op's wire bytes out, the tagged
  answer back) costs at most 2 µs more than a native `Host::apply`; the fly-through's `Bench`
  tick (100 carves) through the component costs under 2× native `DigLogic`. The per-call
  proposal was 1 µs, and the readings below missed it at 1.37 to 1.42 µs; the 2 µs budget was
  adopted at the close (an orchestrator call, overturnable by the owner). The empty component
  tick (1.1 to 1.25 µs) is a reading, not a budget. Host-call work is charged to fuel at the
  configured rates (Plan 0019): before any host work, `apply` and `unsupported` are charged
  `WorkRates::per_byte` (default 1) for each byte of their list argument (the op bytes; 12 per
  changed cell); after each import returns, the tick's fuel loses `per_call` (100), plus
  `per_voxel` (10) for each voxel an `apply`'s op bounds hold and 64 per chunk it touched,
  `per_step` (10) for each step of a `raycast`'s walk and for each cell the `unsupported`
  query looked at, and `per_voxel` for each voxel a `count_in_box` counted. A refused or
  undecodable call is charged its bytes and `per_call` only, and a charge past the fuel left
  is the same `Fuel` failure as an instruction overrun. The server's support query reads a
  `changed` list longer than one in-range edit can change (`(edit_axis_max / 8 + 1)^3`,
  2 146 689 cells at the defaults) as no regions. **Asserted:** a `count_in_box` over
  `1 024^3` resident voxels (`10 x 2^30` fuel) fails the tick on fuel at the default `10^10`;
  100 rays of `max_t = 2^22` through an empty world are charged exactly `100 x (100 + 10 s)`;
  an undecodable 4 096-byte `apply` costs `100 + 4 096`; `unsupported` over `K` cells costs
  `100 + 12 K + 10 s`. A cheap query's per-call cost is not read yet.
  Readings in `docs/nfr/04-edits.md`.
- **Parameter range (Plan 0019):** every edit, ray, box query and region op is checked
  against the world's `WorldLimits` before any arithmetic and refused with
  `EditError::OutOfRange` outside it, changing nothing. Defaults: coordinates and radii within
  `±2^30` fixed units (±4 M voxels); a capsule's `b - a` within `2^30` per axis (the bound
  that keeps its `i128` products exact); an edit's voxel bounds at most 1 024 per axis (at
  most 4 913 chunk coordinates walked); a ray's reach `|dir[k]| x max_t / 256` at most `2^22`
  fixed units per axis (at most 258 cell steps along an axis through empty chunks, 772 on a
  diagonal); a box query at most 1 024 voxels per axis and `2^30` voxels; a drop or removal
  at most 4 096 cells. A world built `with_limits` takes its own; a server and an owned client
  hold the same limits or refuse different ops.
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
- **With the packed brick (dev reference, Plan 0021 Phase 8, 8440aa9 with Phase 8's two
  printouts, Ryzen 9 5900HS, RTX 3080 Laptop GPU, exclusive perf lock, not quiet, alternated
  with main at 5ca4151):** generation encodes each `Full` brick. `world_budget --chunks
  20,20,8` reads 1.254e8 voxels/s/thread (load 10.1) and `--stream 16,24` 1.344e8
  voxels/s/worker (load 10.1), beside Plan 0001's 1.03e8 and 1.12e8. The autopilot at load
  8.4 to 13.2: residency latency wanted-to-mirrored p95 611 ms, max 2449 ms over 72 614 chunks
  against main's 1031 ms and 6523 ms in the same window, both over the 500 ms proposal at this
  load (Plan 0015's quiet reading was 493 ms; the quiet-box re-read is below); streaming CPU per frame p50 5.352 ms, p99
  64.449 ms against 7.575 and 80.785 ms; march GPU p50 1.745 ms against 1.576 ms (Plan 0015's
  spawn march 1.95 ms). A second Phase 8 run (load 10.2 to 14.2) read p95 658 ms, max 2456 ms;
  CPU p50 5.227, p99 63.362 ms; march p50 1.636 ms. The per-chunk mip build, which decodes
  each `Full` brick, reads 0.202 ms per chunk on one thread over the level's 3200 chunks (no
  proposal).
- **Quiet-box re-read under motion (dev reference, 2796062, Ryzen 9 5900HS, RTX 3080 Laptop
  GPU, exclusive perf lock, `flythrough --autopilot --headless --seed 1`, each run started at
  a 1-minute load of 1.7 to 1.9, alternated with `--look plain`):** with `--look plain` (face
  mode, `r_full` 16 throughout) the residency latency p95 reads **381 and 381 ms**, max 574
  and 518 ms over 73 193 and 73 124 chunks, under the 500 ms proposal; streaming CPU per frame
  p50 4.993 and 4.966 ms, p99 32.840 and 32.640 ms. With the default contour look, p95 reads
  904 and 270 ms, max 6014 and 2017 ms over 45 537 and 46 470 chunks; streaming CPU p50 4.740
  and 4.505 ms, p99 65.300 and 35.261 ms. The contour run at 904 ms had 196 syncs with a
  deferral (1531 chunks waiting for space at one tick) against 3 in the other, so the default
  look's latency follows mirror pressure (r_full 16 to 12, ending 13; peak live 0.8894 of the
  budget in both), the footprint ADR-0012's mirror-slot plan owns. The streaming line (1.0 ms)
  stays a recorded miss.
- **After Plan 0023 (same machine and protocol, quiet re-read, 2026-10-21):** plain p95 403
  and 385 ms over the flight, under the 500 ms proposal (the plan's 381 ms bar, set at one
  pre-plan reading, missed by 22 and 4 ms; the pre-plan binary read 383 ms in the same
  session). The default reads 188 and 191 ms. In the owner's windowed default flight the mirror
  sat at its 3 GiB budget with `r_full` 13 and deferral streaks of up to 2015 syncs, which
  ADR-0012's mirror-slot plan owns.
- **Noted 2026-11-04:** the deferrals above are history. Plan 0028 (v0.29.0) closed the
  mirror-slot plan: the default reaches full residency at `r_full` 16 with 0.87 GB live, and the
  pressure rule no longer fires. The "default contour look" of these rows no longer exists
  (Plan 0054, v0.39.0: honest cubes, no attribute pass). No residency p95 has been re-read since
  Plan 0023; that re-read is owed to a plan that touches streaming.
- **Re-read under the loaded light (Plan 0059 Phase 12, 393aa68 plus the pre-roll's tick fix, same
  protocol at 2530 x 1378, 2026-11-09, loaded by the flight's own workers to 5 to 15):** RTX 3080
  Laptop GPU, the default live queue: p95 365 ms (FSR 2 at 1.5) and 379 ms (native), max 445 and
  488 ms over 73 381 chunks; the table: p95 302 and 296 ms, max 371 and 350 ms. RADV RENOIR at
  1280 x 720: queue p95 319 ms, max 404; table p95 321, max 394. Streaming CPU per frame p50 3.2 to
  19.8 ms (the RTX queue runs at load 12 to 15 read 10.0 and 19.8).

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
- The collapse course (`crates/vxe-world/tests/collapse.rs`, Plan 0009: a query, four per-tick
  drops and a removal, hashing every result and the world after each step; Plan 0022 adds a
  slab dropped through a sliver and its rubble pile) pins `0xd223_d848_c8fc_ca0f`, blessed on
  Linux; the Windows arm confirms it on the first CI run after the owner pushes `main`. The
  course's first two parts still hash to the earlier `0x3178_0a15_61c1_3041`. The ops are
  integer-only, so a mismatch is a defect, fixed forward.
- The op log (Plan 0007): the determinism vector's 24 ops encoded as an op log (1446 bytes)
  hash to `0x5ac8_737d_db0f_ccec` (`apps/flythrough/tests/determinism.rs`), and decoding and
  replaying that log reproduces the vector hash; `vxe-net`'s re-simulation tests hold a seeded
  logic to equal op-log bytes. **Asserted** on Linux, blessed at d77ef85; the Windows arm
  confirms it on the first CI run after a push of `main`.
- Native versus component (Plan 0011): one logic linked natively and hosted as a WebAssembly
  component gives byte-equal op logs and equal world hashes from the same inputs. The echo
  logic pins `0x8f7d_4016_15d9_5c4d` (`crates/vxe-script-wasm/tests/equality.rs`), and the
  fly-through's dig logic is equal on the 20-dig replay fixture, the `Bench` input and a saved
  `--logic wasm` session replayed with `--logic native` (`0x86ca427a47e8908c`). **Asserted** on
  Linux; the Windows arm confirms it on the first CI run after a push of `main`.

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
  **Reading (Plan 0017, ef37395 and 3b9ea25, streaming seed 1 headless on the RTX 3080 Laptop
  GPU, exclusive perf lock):** the mirror's one reservation at startup takes 63.0 to 109.9 ms
  for 1930 to 3072 MiB (Phase 1's guess was under 50 ms; a probe put `create_buffer` near
  18 ms per GiB, so the time follows the budget). First frame 516 to 1105 ms warm under
  1-minute load 10.6 to 23.2 from other builds (2135 and 2669 ms with a cold device); Phase 1
  read 414 to 513 ms at load 9.6 to 11.0 beside 367 and 375 ms on the base commit d11d0cb.
  No quiet-box reading: every lane was building during these runs.
  **Level-size first sync after narrowing (Plan 0002 Phases 5 and 8, RTX 3080 Laptop GPU):**
  497 to 517 ms, up from Plan 0001's 349 to 357 ms for the `u16` mirror; the palette scan
  before indices are written. Accepted at the Plan 0002 close (owner, 2026-10-16); TTFF stays
  under 3 s.
  **Reading (Plan 0042 Phase 4, 1474e59, `--castle`, streaming seed 1 headless, release, RTX
  3080 Laptop GPU, exclusive perf lock, 1-minute load 2.4 to 13.0, loaded):** the castle's
  build runs before the first chunk, single-threaded but for the tiles: generate 86 to 138 ms
  (tiles on the machine's threads), encode 30 to 44 ms, parse 43 to 64 ms, map (palette to
  prefabs) 405 to 492 ms, place (`StampedSource::new`, a rotated copy per prefab) 37 to 82 ms,
  about 0.7 s in all over 18 runs. First frame 1087 to 2638 ms with the castle (1105 to 1303
  ms at the gate shot) against 510 to 559 ms without it; full residency (17 077 full, 40 700
  LOD-only chunks) at 8735 to 11 200 ms with it against 9343 to 9396 ms without. The census
  at full residency: 1 547 872 `Full` bricks with the castle (its materials in 295 bricks of
  3 to 4 materials and 407 with glass) against 1 640 292 without, the hall replacing cave
  surface; mirror 841 816 812 B live against 871 679 852 B.
  **Reading (Plan 0038 Phase 5, 654d3c6, `flythrough --source showcase --headless --frames
  60`, release, RTX 3080 Laptop GPU, three warm runs, exclusive perf lock, 1-minute load 8.9,
  loaded):** generation of the 1176 chunks 281 to 300 ms (71 282 `Full` bricks), device ready
  443 to 470 ms, the first sync 83 to 86 ms for 33 554 128 B, first frame 664 to 700 ms. The
  first sync's light-sample extraction (`SyncStats::light_us`) 8.297 to 8.812 ms for 16 471
  samples from 21 484 bricks, the sync's sample budget (16 384 plus the last chunk); the rest of
  the 99 326 follow over six more syncs (29 806 in two syncs since Phase 6a; the close's first
  sync read 4.269 ms for 16 564 samples). Headless, so no window or surface.
  **Reading (Plan 0038 Phase 6, e4b43bb, `flythrough --source showcase`, windowed, release, RTX
  3080 Laptop GPU, exclusive perf lock per run):** cold (the dGPU runtime-suspended before
  each) 2077, 2047 and 2081 ms at load 1.24, 0.68 and 7.56 (the last loaded); warm 671, 660
  and 701 ms at load 7.12, 6.71 and 6.17 (loaded). Under the 3 s line. The session's first run
  (load 1.50, not one of the six) read 4015 ms, over the line; it was not reproduced in the
  next three (689 to 742 ms) and its cause was not read.
- Incremental build of a one-crate change under 10 s on the dev reference; the
  everyday loop does not compile crates a plan does not touch (`default-members`).
  **Reading (Plan 0011 Phase 4, dev reference, AMD Ryzen 9 5900HS, `CARGO_BUILD_JOBS=3`,
  exclusive perf lock, 1-minute load 1.99 to 1.91):** default-members `cargo build` after a
  one-line change (a `pub const`) to `crates/vxe-world/src/lib.rs`, alternated three times:
  1.40 to 1.49 s on the tree before Plan 0011 (066d90e), 1.46 to 1.48 s at its tip (70b40ec
  plus the bench). The default members compile no `wasmtime`: `cargo tree -e normal,build`
  of them and `cargo tree -p flythrough -e normal` list none; `flythrough --features wasm`
  brings it in. Under the 10 s budget, and unchanged at the Plan 0011 close: the default build
  still compiles no `wasmtime` or `tokio`.
  **Reading (Plan 0072 Phase 1, 2026-11-12, 5909af8, dev reference, rustc 1.97.1,
  `CARGO_BUILD_JOBS=4`, exclusive perf lock, quiet: 1-minute load 1.80 to 1.93 at the start):**
  the same change, a `pub const` appended to `crates/vxe-world/src/lib.rs`, three runs:
  default-members `cargo build` **3.3, 4.3 and 4.4 s, the 10 s budget met** (the chain
  `vxe-world` 0.6, `vxe-render` 0.9, `flythrough` lib 0.5, its bin 1.3 s; `vxe-tester` beside
  it). The quick tier's test build for it (`cargo nextest run --workspace --profile quick
  --no-run`) 43.2, 41.0 and 64.5 s (the third ended at load 14.6), 167 to 251 s of units at
  four jobs, the longest the `flythrough` and `vxe-script-wasm` test binaries (4.8 to 12.6 s
  each). A fresh worktree's first test build is 455 to
  479 s at four jobs, a third of its unit time `wasmtime` and `cranelift`
  (`docs/plans/logs/0072.md`).
  **Reading (Plan 0072 Phase 2, 2026-11-13, 17fb491 with dependencies at `debug =
  "line-tables-only"`, the profile kept, the same machine, jobs, lock and scope, load 1.48 to
  1.85 at the start, each in a new target dir):** the same change's `cargo build` **2.7 to
  4.0 s, the 10 s budget met**; its quick-tier test build 17.8 to 22.4 s against the base's
  33.1 to 52.1 s re-read beside it; the cold test build 402.6 to 416.8 s against 442.4 to
  493.2 s, the target dir 12.2 GB against 17.7. With `[profile.dev] split-debuginfo =
  "unpacked"` as well, 2.4 to 2.5, 12.2 to 12.5 and 397.8 to 400.4 s, 7.6 GB; not kept, since
  rustc rejects it on x86_64-pc-windows-msvc and wasm32 and a profile cannot scope it to Linux.
- The test gate (Plan 0043; re-cut 2026-11-03 for iteration speed, owner): a dev phase runs
  `node scripts/test-changed.mjs --profile quick` (the CPU tests of the changed crates and
  their dependents, no perf lock) and adds `--profile gpu` under the lock when the change
  reaches the renderer; a plan's tip and its close review run `--profile gate` (quick plus
  gpu, everything but soak), CI's push run its `ci` twin (the GPU group unthrottled). The
  tiers select by marks (`.config/nextest.toml`, 2026-11-10): a GPU test file is
  `tests/gpu_*.rs`, a long series is `#[ignore = "soak"]`, and a hand-run measurement is
  `#[ignore]`d inside a `mod reading`. `--profile soak --run-ignored only` runs the long
  series on request and weekly in CI (`soak.yml`), `--profile full --run-ignored all` only
  when the owner asks; both leave the readings out.
  Budgets, proposals until read on the dev reference, quiet: quick under 60 s, so a phase gate
  holds no lock; gpu under 10 min wall; gate under 12 min wall; **no single gate test over 10 s
  on the dev reference's lavapipe, or over 60 s on CI's Linux arm** (the lavapipe line was
  30 s until 2026-11-10; the owner lowered it to bring the gate under 12 min, Plan 0066). A
  lavapipe test over the line is marked `#[ignore = "soak"]`; a CPU test over it stays in the
  gate on both arms, since it runs beside the lavapipe group and the cross-platform vectors
  need the Windows arm (owner, 2026-11-11); the soak tier has no budget, runs weekly
  in CI, and the close review runs the soak tests of the crates its plan changed. `node scripts/test-times.mjs <log>` reads a run logged
  with `--status-level pass` and names the tests over the line.
  **Reading (Plan 0057 close review, 2026-11-05, bb5fa9d with the fix pass and main merged, 731 tests, dev reference on lavapipe, exclusive perf lock, 16G cap, built tree, `--status-level pass`, load 0.93 at the start and 2.03 at the end, nothing else running):** gate **1300.7 s, over the 12 min proposal**. Earlier the same day, with reads beside it, 1702.5 s, 730 tests, load 1.77 to 1.85; dev's run at 2de29c6 924.7 s. Sum of tests about 1636 s, `flythrough::settle` the largest at 358 s over 37 tests, then `golden` 218 s, `look` 182 s. Over the 30 s line: `a_corner_tie_ray_hits_the_same_voxel_through_the_same_face_with_and_without_skips` 40.4 s (69.1 s in the earlier run), `a_dug_hole_settles_within_the_bar` 31.8 s.
  **Reading (2026-11-05, 7cd77ad, 729 tests, all passed, dev reference on lavapipe, exclusive perf lock, 16G cap, built tree, `--status-level pass`, load 4.62 at the start and 2.81 at the end, so loaded, nothing else running):** gate **1313.4 s, over the 12 min proposal**, with those two tests on the soak list. Sum of tests about 1662 s, `flythrough::settle` 338 s over 36 tests. Over the 30 s line: `smoothed_normals_cut_the_normal_error_and_move_no_hit` 32.3 s, `the_tone_pass_reproduces_the_composites_bytes_on_every_golden_pose` 31.8 s, `castle_gate_golden` 31.1 s (its first render on that cache; alone 27.4 s cold, 11.4 to 13.3 s warm), `a_face_hidden_from_every_eye_is_settled_before_it_is_seen` 30.8 s.
  **Reading (Plan 0059 close review, 2026-11-09, d1e3468 with main merged, 725 tests, all passed, 75 skipped, dev reference on lavapipe, exclusive perf lock, 16G cap, build in the same timed run, `--status-level pass`, load 1.92 at the start and 2.34 at the end):** gate **850.2 s, over the 12 min proposal** but 35 % under the 1313.4 s before the table's deletion, with the `flythrough::settle` binary back in the gate. Sum of tests 20.0 min serial. None over the 30 s line; the slowest `the_castle_stands_in_the_hall_with_its_gate_facing_the_spawn` 25.7 s.
  **Reading (Plan 0066 Phase 2, 2026-11-10, dev reference on lavapipe, exclusive perf lock, 16G cap, built tree, `--status-level pass`, nothing else running):** before the 10 s marks, 9359b82, 725 tests, all passed, load 0.70 at the start and 1.60 at the end: gate **771 s**, 18.5 min serial, 28 tests over 10 s (7.5 min of it; the list is in the plan's readings log). With the 28 marked `#[ignore = "soak"]`, 697 tests, all passed, load 1.40 at the start and 1.56 at the end: gate **505 s, under the 12 min proposal**, 11.6 min serial; four tests read 10.9 to 12.9 s that read 5.9 to 10.0 s in the first run. The 28 alone under `--profile soak --run-ignored only`: 272 s, all passed. One phase's gpu check, `binary(=gpu_look)` (what `--binaries gpu_look` selects): 60 s wall, 11 tests. At the close (owner, 2026-11-11) `castle_vector` and `showcase_vector` went back into the gate as CPU tests (548f4b9), leaving 26 soak marks; the gate was not re-read after it.
  **Reading (Plan 0064 close review, 2026-11-11, 72ff4cb with main merged, dev reference on lavapipe, exclusive perf lock, 16G cap, `scripts/build-guests.sh` first, `-j 12`, `--status-level pass`, load 3.8 at the start and 1.2 at the end):** 757 tests, all passed, 102 skipped: nextest **553.4 s**, 597 s with the build of the merged crates; under the 12 min proposal. Over the 10 s line, none of them the plan's: `castle_vector` 17.9 and `showcase_vector` 16.0 (CPU, in the gate by the owner's call), `a_baked_view_is_final_on_its_first_frame` 12.5, `uniform_chunks_render_identically_from_their_mips` 11.5, `the_rig_reads_the_default_look` 11.3, `the_score_blends_distance_and_the_view` 10.2 s.
  **Reading (Plan 0072 Phase 1, 2026-11-12, 5909af8, dev reference on lavapipe, exclusive perf lock, 6G cap, built tree, guests built, `--status-level pass`, three runs, load 1.74 to 1.96 at the start and 1.46 to 2.76 at the end):** 830 tests, all passed, 104 skipped: nextest **700.4, 793.6 and 785.2 s**, the second and third over the 12 min proposal. The lavapipe group (189 tests, `max-threads = 1`) sums to 700.3, 793.5 and 785.1 s serial, the wall to 0.1 s; the 641 CPU tests sum to 241 to 265 s beside it. Over the 10 s line in the first run: `castle_vector` 18.7 and `showcase_vector` 18.4 (CPU), `a_baked_view_is_final_on_its_first_frame` 17.6, `the_castle_path_flies_in_air` 15.0 (CPU), and six lavapipe tests at 10.3 to 12.8 s.
  **Reading (Plan 0072 Phases 2 and 4, 2026-11-13, 1626405 and e34e106, the same setup, Phase 2's dev profiles, load 1.30 to 1.95 at the start):** at `max-threads = 1`, **656.0 s**, under the 12 min proposal, 830 passed, 104 skipped. The lavapipe group at 2 and 3, three runs a cell, 830 passed in each: N = 2 354.6 to 368.2 s, N = 3 258.6 to 262.0 s, with or without `LP_NUM_THREADS=8`; but in every run `gpu_bake`'s `a_dug_emitter_leaves_the_near_ring_at_once` took 2.5 to 2.7 times its N = 1 time (17.4 to 19.2 s against 6.2 to 7.9), and at N = 3 `a_dug_hole_is_final_on_its_first_frame` too (18.2 to 19.1 s against 5.4 to 9.5). By Plan 0072's rule (no test over twice its N = 1 time) the group stays at 1 and the 12 min proposal stands; the review's 8 min (480 s) proposal is not taken.
  **Reading (Plan 0072 close review, 2026-11-13, 4b7f822 with main c1d1d3f merged, dev reference on lavapipe, exclusive perf lock, 6G cap, -j4, built tree, guests built, `--status-level pass`, load 1.46 at the start and 2.16 at the end, a second lane building beside it):** 844 tests, all passed, 78 skipped: nextest **609.8 s, under the 12 min proposal**, the lavapipe group at `max-threads = 1`. Over the 10 s line: `castle_vector` 18.0, `the_castle_path_flies_in_air` 16.5, `showcase_vector` 16.1 (CPU), `a_baked_view_is_final_on_its_first_frame` 12.9, and four more at 10.3 to 11.4 s.
  **Reading (CI after Plan 0072, 2026-11-13, GitHub-hosted runners):** run 43377881019 on 9c256bb (cold cache, the profile change): Linux 26 min 29 s, Windows 33 min 9 s; gate on Linux 844 passed, 78 skipped, 437.6 s. Run 43395622401 on 393ed45 (warm): **Linux 15 min 6 s, Windows 18 min 47 s**, over Plan 0072's under-10 and under-15 min proposals. **Accepted by the owner 2026-11-13 ("record this time as accepted, let it be")**: these walls are the CI line; no further cut is owed.
  **Reading (CI run 41332396750 on 929d1b3, 2026-11-01, the Linux arm, lavapipe, 745 tests,
  the `ci` profile before the re-cut):** 5386 s wall for the test step, 354 min of serial
  test time; 41 tests over 60 s take 320 of those minutes, 603 tests under 5 s take 4. The
  five longest: `fsr2_at_scale_1_keeps_stripe_edges_sharp_at_arms_length` 2947 s,
  `a_hole_in_the_wall_lights_the_dark_box_and_filling_it_darkens_it_again` 2173 s,
  `the_vein_wall_patches_are_counted` 1538 s,
  `a_reveal_behind_a_plug_follows_within_seventy_three_frames` 1374 s,
  `the_vein_wall_holds_still_with_the_lamp_off` 1337 s. Those 41 are the first soak list. The
  dev-reference reading of the re-cut tiers is owed by the first lane that runs them: the GPU
  was held by Plan 0054's close review when the cut was made.
  **Reading (Plan 0043 close review, 5877b8a, 599 tests, dev reference, Ryzen 9 5900HS and RTX
  3080 Laptop GPU on lavapipe, built tree, `--test-threads 2`, other lanes running, so all
  loaded):** quick 470 tests in 45.4 s, no lock, 1-minute load 6.40 to 8.80; gpu 124 tests in
  201.5 s, exclusive perf lock, load 9.51 to 2.53; full 599 tests in 327.1 s, exclusive perf
  lock, load 7.90 to 3.71. The dev lane read 44.8, 205.2 and 362.7 s at 8059d41, also loaded.
  Before the tiers every phase gate was a full run under the lock, 4 to 7 minutes in the
  lanes' logs at 87e4b67. The CI Linux job time against run 39808788735's 981 s was owed by the
  first CI run after the merge into main (the Phase 3 amendment); the reading is run
  41332396750's, above.
  **Reading (Plan 0052 close review, 200607c with main 7ef72e6 merged, 745 tests, dev
  reference on lavapipe, `CARGO_BUILD_JOBS=4`, `--test-threads 2`, exclusive perf lock, a 6 GiB
  memory cap, built tree):** full 745 run, 744 passed, 1 failed (main's
  `showcase_hall_golden_is_lit_by_its_four_ores`, owed to Plan 0050 Phase 6), 12 skipped,
  **6040.6 s**, 1-minute load 3.94 at the start and 6.84 at the end with another project's
  processes running (loaded). Plan 0040's close read 690 tests in 1440 s at load 3.5; the slow
  list (Plan 0047's long emissive runs and the 1600-frame indirect golden among them) is most of
  the difference, but the run printed no per-test times, so the split is not read.
  **The two control renders (Plan 0052 Phase 6).** The indirect golden's control pass renders
  64 frames (was 1600) and the showcase golden's dark pass 8 (was 64). Quiet readings, each test
  alone, lavapipe, exclusive perf lock, load at each start 1.48 to 12.18 (lavapipe's own threads
  from the run before): `indirect_golden_shows_the_vein_s_bounce_light` 151.57 s at the base,
  115.70 s at the tip (-24 %); `showcase_hall_golden_is_lit_by_its_four_ores` 17.90 s and
  14.56 s (-19 %). The showcase golden left the slow list. Re-read at the close review alone at
  load 6.15 to 14.11 (loaded): indirect 149.0 s, showcase 16.5 s.
- Pipeline creation, one compute pipeline per pass the renderer builds (attrib, the march hit
  pass and the nine march-prefixed passes), read by
  `cargo run -p vxe-render --release --example pipeline_times` (cold is the first creation on
  a fresh device, min the fastest of three). No budget; part of time to first frame.
  **Reading (Plan 0046 Phase 1, on ce62f0c, Linux on Vulkan, driver shader caches off with
  `MESA_SHADER_CACHE_DISABLE=true` and `__GL_SHADER_DISK_CACHE=0`, adapter by
  `WGPU_ADAPTER_NAME`, exclusive perf lock, 1-minute load 10.4 to 11.1, so loaded):** sum of
  the eleven cold creations 1396.1 ms on the RTX 3080 Laptop GPU (light 522.6, march 312.3,
  gradient 265.1, attrib 75.3), 458.4 ms on the Radeon iGPU on RADV (light 152.7, march 90.9,
  gradient 70.8, attrib 54.2), 299.0 ms on lavapipe (light 96.3, march 53.9, attrib 32.4).
  Microsoft's Linux `dxc` 1.9.2609 on naga's Shader Model 6.0 HLSL of the same passes,
  `-HV 2018 -Ges` as wgpu passes them, wall time per process, at load 8.95: attrib 437 ms,
  march 283, light 517, gradient 390, upscale 111, the other six 46 to 81.
  **Reading (Plan 0046 close, Windows DX12 on WARP, the `windows-latest` CI arm, a software
  rasterizer, run 39843578678 on f3ed4a3):** sum of the eleven cold creations 76 623.6 ms
  under FXC (`WGPU_DX12_COMPILER=fxc`; attrib 42 412.2, light 18 050.1, march 10 214.0,
  gradient 4876.8) and 2507.9 ms under the default static DXC (light 744.6, attrib 596.0,
  gradient 469.0, march 452.5); the full table is in Plan 0046's close review. Debug
  `flythrough.exe` 37 744 128 B with static DXC linked; `machdxcompiler.lib` 113 589 902 B, an
  upper bound on what the feature adds (no before-size was read). The compile is not WARP's
  whole startup cost: with DXC the headless fly-through's `flythrough::shutdown` trace test on
  the default look still failed at 121.055 s against its 120 s bound, and each `mirror`
  attribute test still passed nextest's 90 s slow marker (15.3 s for the three at 578c8d3,
  152.8 to 158.2 s under FXC after Plan 0028), so WARP's cost is the attribute pass's
  execution. The test then ran `--look plain` there, a look Plan 0055 removed; it runs the
  default look now, which has no attribute pass. The Windows `cargo build --workspace` step
  delta is not read: the merge run's cache key changed with the lockfile.

## 8. Collision and coarse grids

Added at Plan 0004's close (2026-10-17). `vxe-sim` sweeps and the character controller,
`vxe-world`'s coarse grid and walkable map. All of it is integer and joins §6: the controller
course hash `0x0c80_8031_d14f_27eb` (`crates/vxe-sim/tests/course.rs`) is blessed on Linux and
the Windows arm confirms it on the first `main` CI run after Plan 0004's merge.

| Figure | Budget | Measured by |
|---|---|---|
| Controller tick, one body | 20 µs | `cargo bench -p vxe-sim --bench controller` |
| Sweep, one body, one contact | 26.3 µs (the Plan 0004 close reading, accepted by the owner 2026-10-17; the proposal was 5 µs); a sweep profile is scheduled only when a scene sweeps more than about 100 bodies a tick | same bench, `1000 sweeps of the F2 wall case` |
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
- **Controller tick with depenetration (Plan 0016 Phase 1, an overlap test at every tick's
  start):** `60 ticks of the F6 staircase, all on terrain` 108.17, 110.59 and 109.61 µs
  against 108.56, 108.35 and 111.60 µs for the same bench at e23f309 (no overlap test),
  alternated on the same box: AMD Ryzen 9 5900HS laptop, Linux, not quiet (load average 13.4
  falling to 5.2, other processes running), no perf lock. About **1.8 µs per tick**; the two
  arms do not separate. The 156.01 µs above was not reproduced on this day by either arm.
- **Sweep:** `1000 sweeps of the F2 wall case` 26.303 ms (26.295, 26.312), and 26.264 ms on
  a second run: **26.3 µs per sweep**, against the 5 µs proposal. The owner accepted the
  measured figure as the line at 17.5 µs (fe50d6d); the walk-check fix pass (closes-on filter,
  overlap guard, rounded-up step length; ADR-0005 Outcome) raised it to 26.3 µs. **The
  owner accepted 26.3 µs as the line (2026-10-17)**, with a sweep profile scheduled only
  when a scene sweeps more than about 100 bodies a tick.
- **Coarse grid update** after a radius-10 (1 m) carve on the solid world: 100 updates
  1.3544 ms (1.3528, 1.3560), **13.5 µs per update**. **Walkable update** from those changes:
  807.17 µs (805.83, 808.64), **8.1 µs per update**. Both under their lines. Plan 0003's
  `mixed` world was not on the branch; the bench runs on the solid world.
- **Coarse grid update on the server path (Plan 0018 Phase 5):** the server updates its grid
  over each applied op's aabb, in op order on the server thread. `cargo bench -p vxe-net
  --bench tick` (100 radius-1 m carves on `solid3_world()` each followed by
  `CoarseGrid::update_aabb`, against the carves alone), AMD Ryzen 9 5900HS, Linux, a9014d0
  plus the phase's bench arm, exclusive perf lock, not quiet (1-minute load 14.46 falling to
  6.50), three alternated rounds: 1.4764, 1.4876 and 1.2249 ms per 100, **12.2 to 14.9 µs per
  update**, under the 50 µs line and beside the 13.5 µs above.
- **Coarse grid RAM:** 6 553 600 B at the level (3200 chunks, `flythrough --level --seed 1`,
  fe50d6d). At 32 × 32 × 32 chunks it is 67 108 864 B by the same 2048 B per chunk
  (arithmetic, not read). Grid build at the level, one thread (`world_budget --chunks 20x20x8
  --seed 1`): 156.3 ms at the close review; grid plus walkable map in the fly-through 231 ms.

## 9. Audio

`vxe-audio` (Plan 0006 closed 2026-10-17, ADR-0006): 48 kHz stereo `f32`, 256-frame blocks
(5.33 ms), mixed on the device callback. The baseline CPU is not on hand, so every reading is
dev reference: Ryzen 9 5900HS laptop, release, the quiet-box protocol of Plan 0006's close
(exclusive machine lock, 1-minute load average under 2, no other process above 7 % CPU),
44a1582's tree unless stated. Readings taken under a load average of 9 to 21 are in the
plan's Close triggers as superseded figures.

**Paired mixer readings (ADR-0020, 2026-10-21).** A plan's *relative* bound on `cargo bench -p
vxe-audio --bench mixer` (within N percent of the parent's) is read with both builds compiled
with `RUSTFLAGS="-C llvm-args=-align-loops=64"` and `CARGO_TARGET_DIR=target/aligned`. Each build
gets three alternating runs in one exclusive perf-lock hold, with the 1-minute load and the top
processes stated, and each case's median of the three criterion medians is compared. The
default-flag pair may be logged beside it, but it does not decide the bound: the placement of
the FIR loop alone moved the static case by 5.8 percent in Plan 0020 Phase 13. The *absolute*
mixer figures below are read on the default build, the code that ships, under the quiet-box
protocol above. Other benches keep default-flag paired readings unless their plan adopts this.

- **Mixer callback, 32 HRTF voices, one 256-frame block:** 1.0 ms on the baseline CPU
  (proposal; 32 x 2 x 128 x 256 = 2.1 million multiply-adds plus filters and taps).
  **Reading (`cargo bench -p vxe-audio --bench mixer`, 32 voices of looping noise, each
  low-passed and carrying 16 taps):** 0.3619 ms (0.3615 to 0.3623) static; 0.6667 ms (0.6659
  to 0.6673) with the listener yawing 90 degrees per block, every voice crossfading its HRIR.
  **Silent voices (Plan 0052 Phase 5, 14f3841).** The voice smoothers (own gain, send, gain,
  distance gain) settle on their target within `1e-7` at each block edge and the voice
  low-pass state flushes under reverb's `FLUSH_BELOW`, so a voice at gain 0 no longer decays
  into subnormal floats. **Reading** (`cargo bench -p vxe-audio --bench mixer`,
  `mixer_block_silent`, dev box, exclusive perf lock, load 1.36 to 2.96, default build, not the
  ADR-0020 paired protocol): 64 voices at gain 0 after 10 s 1.7909 ms at the base (47 % over 64
  fresh voices' 1.2177 ms), 1.1721 ms at the tip, inside the fresh case's spread (1.1951 to
  1.2164 ms) and under it.
- **HRTF ceiling (arithmetic, Plan 0052; no budget).** The HRIR convolution is the direct form:
  2 ears x 128 taps per sample, 64 x 2 x 128 x 48 000 = 0.79 G multiply-adds a second at 64
  voices (about 1 G with the filters and taps), twice
  that while sources move and every voice crossfades two HRIRs. That is the mixer's ceiling on
  voice count; a partitioned FFT convolution is the step when a consumer needs more voices.
- **`Acoustics::update`, 32 sources:** about 1.0 ms per call, accepted by the owner
  2026-10-17 (D2b) over the plan's 0.5 ms proposal, which had no arithmetic behind it: about
  240 rays of about 100 voxel steps at about 40 ns, each step a chunk-map lookup and lock.
  **Reading (`cargo bench -p flythrough --bench acoustics`, seed-1 level-size cave, listener
  at the spawn, sources 2 to 12 m out, every source due, 512 taps sent):** 1.0294 ms (1.0278
  to 1.0313). Speed-ups (a chunk handle cached across steps, a probe worker thread) are
  followups.
- **Audio RAM:** the pack plus 200 ms delay lines plus scratch (proposal). **Reading:** pack
  3 194 204 B in memory (3 044 420 B on disk); delay line 39 424 B per voice, 2 523 136 B for
  the 64-voice pool allocated at `Mixer::new`.
- **Startup:** pack load under 10 ms (proposal). **Reading:** 13.5 to 13.6 ms in the release
  fly-through over three runs (`--seed 1 --audio --headless --frames 10`, a null-ALSA
  device); 7.1 to 8.9 ms in a release test build. Accepted as the reading (D2c): it runs once.
- **Fly-through `F1`:** the callback times itself (`OutputStats::busy_max_ms`) and `F1` prints
  the longest callback over the last second, the voices, the active taps and the last
  acoustics update's time. **Reading by the owner, 2026-10-17, `main` at 7e8b3d2 with 8
  voices playing:** callback max 0.429 ms, 128 active taps.
- **Determinism:** `World::trace_materials` is integer and its 500-ray hash is pinned in
  `crates/vxe-world/tests/queries.rs` on both arms; nothing else in `vxe-audio` is
  deterministic by contract (§6).
- **Plan 0020 (the late tail, sixteen direction buses, the room fan, gliding taps; closed
  2026-10-21, v0.24.0).** Proposals: mixer block, 32 HRTF voices, static at most 0.6 ms and
  turning at most 1.0 ms; `Acoustics::update`, 32 sources with a 16-ray fan slice, at most
  1.1 ms. **Readings at the close (recorded misses, accepted by the owner 2026-10-21; a
  quiet-box re-read is a followup, then a mixer optimisation if it stays over):** the shipped
  code (Phase 13, c85bac5), default build, dev box, release, exclusive perf lock, median of
  three runs at load average 4.6 to 9.3 with other processes near 100 % CPU: mixer static
  0.6897 ms, turning 1.1441 ms; with the direct delay on, 0.8011 and 1.2577 ms. The same code
  aligned (ADR-0020, load 3.2 to 5.5): 0.657 and 1.110 ms. `Acoustics::update` at Phase 9's
  rays (100 m range, load 13.6 to 9.8): 0.7370 ms. Audio RAM printed by the mixer bench: echo
  lines 116 224 B per voice (7 438 336 B for the 64-voice pool, 600 ms), two tail networks
  3 616 768 B, the pre-delay line 39 424 B, buses 24 512 B, tap slots 8 192 B. **Earlier
  readings, dev box,
  release, exclusive perf lock, vxe-audio and both benches as at d7eb1f8, 2026-10-20; the box
  was not quiet (1-minute load average 1.87 to 2.98, and other processes above 7 % CPU: one at
  86 %, two at 37 %), so they read high against the protocol above:** mixer static 0.5078 ms
  (0.5004 to 0.5162), turning 0.8786 ms (0.8700 to 0.8887) (`cargo bench -p vxe-audio --bench
  mixer`, 32 voices with 16 taps each into the buses, the 12 m room's tail running);
  `Acoustics::update` 0.8388 ms (0.8327 to 0.8463) (`cargo bench -p flythrough --bench
  acoustics`, the fan's first slice in every call; the fan's rays stop at the first solid
  voxel). Audio RAM printed by the mixer bench: the tail's lines 323 584 B, the buses 24 512 B,
  tap state 128 B per voice (8 192 B for the 64-voice pool); a streamed voice's ring 19 200 B.
  Room estimates the fly-through prints headlessly (`--room N --headless --frames 600`, no
  device): 12 m mfp 7.999 m and T60 2.257 s, 36 m T60 6.770 s, 4 m T60 0.752 s; the seed-1
  streaming spawn mfp 16.082 m, T60 4.537 s, energy -7.5 dB.

## 10. Not goals (for now)

- Binary size.
- Mobile, web or console targets as such. The portable profile of §1 is a PC-class setting
  that a Switch-2-class device could run; a console port, its SDK and its certification are
  not planned, and Switch 1 is out of reach for a ray marcher of this shape (owner,
  2026-10-17).
- Worlds wider than 2^24 voxels (about 1.7 km at 10 cm) from the camera. The march does its
  world math in `f32`, exact to 2^24 voxels; a following window past that needs
  camera-relative march coordinates, a future plan's work (Plan 0052's Followups).
- Frame rates above 60 fps as a gate; they are welcome, not required. A fast machine
  reaches 120 fps through a performance profile, not through a budget row here.
- VR on the baseline. The VR-readiness decision in `roadmap.md` keeps the renderer's doors
  open; a VR plan adds its own profile here (two views, 90 Hz, a named card) when scheduled.
