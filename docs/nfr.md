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

- **Cost:** 100 sphere carves of radius 1 m (exactly 4169 voxels each at 10 cm, the lattice
  count on voxel centres; the continuous volume is about 4189) in one frame cost at most 2.0 ms
  of CPU on the dev reference (raised to 2.2 ms by the owner on 2026-10-20 after the quiet-box
  re-read below, and returned to 2.0 ms on 2026-10-21 after Plan 0031's quiet-box reading;
  **met on the dev box**, no baseline-GPU reading) including result counting and dirty marking, and their upload (up to about 6400 bricks,
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
  **Quiet-box reading at the Plan 0014 tip (dev reference, AMD Ryzen 9 5900HS, 5850218,
  exclusive perf lock, 1-minute load 1.93 before and 1.50 after, paired and alternated with
  the Plan 0003 tip 2129bfb):** `worst_warm` (the `worst` case after
  `World::reserve_brick_arrays`) 3.034 to 3.054 ms (CI 3.031 to 3.057 ms over two runs);
  `worst` 3.189 to 3.264 ms (CI 3.187 to 3.269 ms), against 3.549 ms for the Plan 0003 tip
  in the same window; `mixed` 2.326 to 2.355 ms; `tool` 6.777 to 6.798 ms, `tool_warm` 6.660
  to 6.688 ms. The fly-through `B` line (`--level --seed 1 --headless --bench 1`) reads 2.98
  to 3.04 ms against 3.31 to 3.32 ms, three runs each. The warm figure assumes the reserve
  runs off the edit path: the caller tops the pool up between frames (the fly-through does,
  after each frame's edits), so a frame's expansions take pooled arrays instead of
  allocating. Both readings miss the 2 ms budget; the 2 ms figure is unchanged, and the
  remaining cost is the row write bounded by the brick representation (Plan 0014's close). The decision awaits the owner (a representation ADR versus a measured cap).
  **Reading at the Plan 0021 tip (dev reference, AMD Ryzen 9 5900HS, ADR-0012's planar edit
  kernel, exclusive perf lock, not quiet):** `worst_warm` 1.339 ms against 2.811 ms for the
  Plan 0021 Phase 1 tip 992fdaa alternated in the same window (load 17.6; 1.485 against
  2.864 ms at load 24.1); `worst` 1.356 against 3.527 ms. At the Phase 8 tree (8440aa9, load
  10.4) `worst_warm` 1.516 ms and `worst` 1.538 ms; with the pool gone both cases build the
  same world. The fly-through `B` line (`--level --seed 1 --headless --bench 1 --frames 1`)
  reads 1.52 ms against Plan 0014's 2.98 to 3.04 ms. **Under the 2 ms line on the dev box**; no
  baseline-GPU reading. The owner held the 2 ms figure on the dev box (2026-10-20), so the
  line is **met on the dev box**. The allocation share the retired pool covered: 2495
  expansions of 11.2 ns, 27.9 us, 1.8 % of the 1.516 ms `worst_warm`, under the 5 % that would
  have called for per-class pools.
  **Quiet-box re-read (dev reference, AMD Ryzen 9 5900HS, on AC with the performance profile
  and governor, 2796062, exclusive perf lock, 1-minute load 1.81 at the start):** `worst`
  1.793 ms, `worst_warm` 1.794 ms, `mixed` 1.417 ms, `tool` 5.823 ms, `tool_warm` 5.821 ms.
  Paired with 8440aa9 (the Plan 0021 Phase 8 tree; `vxe-world` is unchanged since),
  alternated twice at load 1.76 to 2.27: `worst_warm` 1.785 and 1.789 ms on main against
  1.783 and 1.783 ms, so the gap to the 1.34 to 1.52 ms above is machine state, not code, and
  is unexplained. The fly-through `B` line reads **2.06, 2.07 and 2.09 ms**, over 2 ms and
  under **2.2 ms, the line the owner set on 2026-10-20**. One more optimization pass is
  planned; no baseline-GPU reading.
  **Quiet-box reading at the Plan 0031 tip (dev reference, AMD Ryzen 9 5900HS, on AC with the
  performance profile and governor, 8dad5ca, exclusive perf lock, 1-minute load 0.48 to 1.98
  and no other process above 7 % CPU before every run, three runs alternated with 2796062's
  tree, criterion medians):** `worst` 0.766, 0.774 and 0.774 ms against 1.318, 1.316 and
  1.337 ms; `worst_warm` 0.765, 0.775 and 0.776 ms against 1.316, 1.319 and 1.338 ms;
  `mixed` 0.622, 0.628 and 0.630 ms against 1.044, 1.047 and 1.055 ms; `tool` 3.585, 3.578
  and 3.590 ms against 4.282, 4.194 and 4.217 ms; `tool_warm` 3.585, 3.586 and 3.590 ms
  against 4.284, 4.195 and 4.219 ms. The fly-through `B` line (`--level --seed 1 --headless
  --bench 1 --frames 1`) reads **1.00, 0.99 and 0.99 ms** against 1.49, 1.49 and 1.54 ms for
  2796062 in the same window, the same carve counts on both. 2796062 read 1.79 ms `worst`
  and a 2.06 to 2.09 ms `B` line at the re-read above; the gap to its 1.32 and 1.49 to 1.54
  ms here is machine state, not code. The planar kernel is unchanged; the op path builds each
  brick's shape words straight from row spans solved once per op (Plan 0031). On this reading
  the owner returned the line to **2.0 ms on the dev box** (2026-10-21): the `B` line meets it
  with about 1 ms to spare. No baseline-GPU reading.
  **Span arena (Plan 0052 Phase 5, c130daf; bar amended by the owner 2026-10-30: `tool` and
  `tool_warm` within 2 % of the base, `tool_long` not above it).** `OpRows` keeps row spans in
  one arena of slabs, one per brick row it reaches, instead of a table over the op's whole
  bounds: a diagonal capsule of 999 voxels at radius 2 writes 13 828 span entries against the
  338 724 rows of its bounds (the arena is reserved for the bounds, about 5.4 MB of address
  space, and only the reached rows are written). **Readings** (dev reference, AMD Ryzen 9
  5900HS, `cargo bench -p vxe-world --bench edits -- "^tool"`, exclusive perf lock, criterion
  medians; base binary built from `a392e47`'s `edit.rs` with the current bench file): quiet run,
  tip then base, 1-minute load 1.47 to 1.97: tip `tool` 4.8885 ms, `tool_warm` 4.8714 ms,
  `tool_long` 5.8762 ms; base 4.8221, 4.8096 and 5.8850 ms. `tool` +1.4 %, `tool_warm` +1.3 %
  (their intervals do not overlap), `tool_long` -0.1 %: within the amended bar. The base's
  bounds-wide table is the waste removed; a short capsule pays the arena's indexing for it.
  `tool_long` is new in this plan (eight radius-2 capsules along the space diagonals, about 430
  voxels long).
- **Results** are returned synchronously with the edit, per material, exact.
- **Stamp cost (proposals, Plan 0008):** a 16³ prefab stamp (the shelled box `Prefab::vault`)
  into solid rock costs under 100 µs, and a 64³ all-written prefab under 5 ms, both including
  the `EditResult` tally. **Readings (dev reference, AMD Ryzen 9 5900HS, `cargo bench -p
  vxe-world --bench stamp`, exclusive perf lock, box under outside load of about 2 to 5,
  commits 83ab811 and 39d3700):** `vault16` 22.6 to 22.8 µs; `prefab64_mixed` 2.02 to 2.07 ms;
  `prefab64_uniform` 1.455 ms. Both under their proposals; the proposals stand as budgets.
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
  dominate, not the decode). **Reading (dev reference, AMD Ryzen 9 5900HS, Plan
  0007 Phase 6, exclusive perf lock, 1-minute load 1.45):** `cargo bench -p vxe-world --bench
  replay`, decode plus replay of a 470 060-byte log file of 10 000 radius-1 m carves on a
  solid 4³-chunk world: 279.07 ms (CI 278.48 to 279.70 ms), about 35 800 ops per second.
  **Quiet-box reading at the Plan 0031 tip (8dad5ca, same machine and protocol as §4's Plan
  0031 reading, three runs alternated with 2796062):** 61.29, 61.29 and 61.20 ms (about
  163 000 ops per second) against 103.16, 102.64 and 103.19 ms.
- **Tick overhead:** 100 inputs through one server tick and one shared-client poll cost at
  most 100 µs more than applying the same 100 ops directly (budget since the Plan 0007 close,
  backed by the reading below).
  **Reading (dev reference, AMD Ryzen 9 5900HS, Plan 0007 Phase 6, exclusive perf lock,
  1-minute load 1.66):** `cargo bench -p vxe-net --bench tick`: 3.4706 ms (CI 3.4663 to
  3.4756 ms) through `EchoLogic` against 3.3859 ms (CI 3.3782 to 3.3990 ms) direct, an
  overhead of 84.7 µs per 100 inputs.
  **Reading with the server's coarse grid (dev reference, AMD Ryzen 9 5900HS, Linux, Plan
  0018 Phase 5, a9014d0 plus the phase's bench arm, exclusive perf lock but not quiet: other
  worktrees building and testing, 1-minute load 14.46 falling to 6.50):** the server now
  updates its coarse grid after every applied op, which is edit work, so the overhead is read
  against the direct carves each followed by the same grid update. Three rounds, arms
  alternated, criterion medians: through `EchoLogic` 4.3895, 3.9923 and 3.8381 ms; direct
  carves with grid updates 4.3400, 3.9843 and 4.0753 ms; direct carves alone 2.8636, 2.4967
  and 2.8504 ms. Overhead per 100 inputs: 49.5 µs, 8.0 µs and -237.2 µs (the last round's
  noise exceeds the difference). Under the 100 µs line in every round; the readings are noisy
  and want a quiet-box re-read.
  **Close-review re-read (Plan 0018, c1ea275, same machine, exclusive perf lock, still not
  quiet: 1-minute load 20.82 at the start and 13.60 at the end):** one round, through
  `EchoLogic` 3.7783 ms (CI 3.7630 to 3.7943 ms), direct carves with grid updates 3.7716 ms (CI
  3.7528 to 3.7914 ms), direct carves alone 2.4188 ms. The overhead is 6.7 µs per 100 inputs,
  inside the overlapping intervals and consistent with the line.
  **Quiet-box reading (2796062, same machine, exclusive perf lock, 1-minute load 1.36 at the
  start and 1.29 at the end):** through `EchoLogic` 2.0012 ms (CI 1.9990 to 2.0037 ms), direct
  carves with grid updates 1.9072 ms (CI 1.9055 to 1.9089 ms), direct carves alone 1.7929 ms.
  The overhead is **94.0 µs per 100 inputs**, under the 100 µs line with 6 µs to spare; the
  loaded readings above understated it.
  **Quiet-box reading at the Plan 0031 tip (8dad5ca, same machine and protocol as §4's Plan
  0031 reading, three runs alternated with 2796062):** through `EchoLogic` 0.9284, 0.9280 and
  0.9264 ms; direct carves with grid updates 0.8581, 0.8647 and 0.8573 ms; direct carves alone
  0.7809, 0.7801 and 0.7843 ms. The overhead is **70.3, 63.3 and 69.1 µs per 100 inputs**
  against 64.7, 63.5 and 70.4 µs for 2796062 in the same window (1.4913 / 1.4266 / 1.3377,
  1.4821 / 1.4186 / 1.3326 and 1.4871 / 1.4167 / 1.3384 ms); the grid updates 77.2, 84.6 and
  73.0 µs against 88.9, 86.0 and 78.3 µs.
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
  **Readings (dev reference, AMD Ryzen 9 5900HS, Linux, `wasmtime` 49.0.3, Plan 0011 Phase 4
  at 07a3be1 (70b40ec plus the bench), exclusive perf lock, 1-minute load 1.87 before and 1.66
  after two runs):** `cargo bench -p vxe-script-wasm --bench boundary`, each tick on a fresh
  `solid3_world()` through a host straight on the world: 100 carves through the echo
  component 3.1181 and 3.1225 ms against native `EchoLogic` 2.9765 and 2.9858 ms, so 1.42 and
  1.37 µs per host call, the tick's entry and the inputs' lowering included (an empty
  component tick reads 1.12 and 1.25 µs). The per-call figure misses the 1 µs proposal. The
  `Bench` tick through the `flythrough-logic` component 3.1984 and 3.2065 ms against native
  `DigLogic` 3.1033 and 3.1025 ms: 1.031× and 1.034×, under the 2× proposal. Re-run after the
  close review's fix pass (host imports under `catch_unwind`): 1.38 µs per call, empty tick
  1.125 µs, `Bench` 1.033×.
  **Restart after a failure (Plan 0016 Phase 2):** a failed call is followed on the next tick
  by a fresh instance and `init`, at most `WasmConfig::max_restarts` times (default 8) per
  session, then the logic stops. One restart of the `flythrough-logic` component
  (`WasmLogic::try_init`: store, instantiation from the pre-linked component, `init`) read
  a median of 26.9 and 29.3 µs over 50 restarts each (min 24.5, max 109.0 µs): a one-off
  release-build timing loop, not a committed bench, AMD Ryzen 9 5900HS, Linux, not quiet
  (load average 7.4 to 9.4), at 1d1b3aa plus Phase 2's working tree.
  **Re-read with host-call charging (Plan 0019 Phase 2, AMD Ryzen 9 5900HS, Linux, `wasmtime`
  49.0.3, exclusive perf lock, not quiet), paired with its parent:** parent c04bfd1 (1-minute
  load 3.94 to 4.66): 100 carves through the echo component 2.4928 ms against native
  `EchoLogic` 2.3589 ms, 1.339 µs per host call, empty component tick 1.092 µs, `Bench`
  1.028×. With charging, 3445f30 (load 7.15 to 4.58): 2.4593 ms against 2.3306 ms, 1.287 µs
  per host call, empty tick 0.974 µs, `Bench` 2.5510 against 2.4014 ms, 1.062×. The 2 µs
  line holds; the charge's fuel read and write is inside the run-to-run noise.
  **Re-read with the per-byte charge (Plan 0019 close review, AMD Ryzen 9 5900HS, Linux,
  exclusive perf lock, not quiet, paired):** parent 40d2b91 (1-minute load 8.17 to 8.26):
  2.5902 ms against native 2.5167 ms, 0.74 µs per host call, empty tick 1.269 µs, `Bench`
  2.9585 against 2.5515 ms, 1.160×. With `per_byte`, adfca8b (load 8.26 to 6.79): 2.7319 ms
  against 2.7332 ms, under 0.1 µs per host call (the native arm read slower than the
  component), empty tick 1.463 µs, `Bench` 2.6337 against 2.5813 ms, 1.020×. The load moved
  the native arm by 0.22 ms between runs, more than the whole host-call cost, so this pair
  resolves no difference the per-byte charge makes; the 2 µs line and the 2× `Bench` line hold
  in both.
  **Quiet-box reading (2796062, same machine, exclusive perf lock, 1-minute load 1.29 at the
  start and 1.41 at the end, one run, not paired with a parent):** 100 carves through the echo
  component 1.8005 ms against native `EchoLogic` 1.6445 ms, **1.56 µs per host call**; empty
  component tick 1.007 µs; `Bench` 1.8380 against 1.7269 ms, **1.064×**. Both lines hold.
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
- The test gate (Plan 0043; re-cut 2026-11-03 for iteration speed, owner): a dev phase runs
  `node scripts/test-changed.mjs --profile quick` (the CPU tests of the changed crates and
  their dependents, no perf lock) and adds `--profile gpu` under the lock when the change
  reaches the renderer; a plan's tip and its close review run `--profile gate` (quick plus
  gpu, everything but the soak list), CI's push run its `ci` twin (the GPU group
  unthrottled). The soak list (`.config/nextest.toml`) holds the long series: `--profile soak`
  runs on request and weekly in CI (`soak.yml`), `--profile full` only when the owner asks.
  Budgets, proposals until read on the dev reference, quiet: quick under 60 s, so a phase gate
  holds no lock; gpu under 10 min wall; gate under 12 min wall; **no single gate test over 30 s
  on the dev reference's lavapipe, or over 60 s on CI's Linux arm**. A test over the line moves
  to the soak list, which has no budget. `node scripts/test-times.mjs <log>` reads a run logged
  with `--status-level pass` and names the tests over the line.
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
  lanes' logs at 87e4b67. The CI Linux job time against run 39808788735's 981 s is owed by the
  first CI run after the merge into main (the Phase 3 amendment).
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
  execution; the test runs `--look plain` there. The Windows `cargo build --workspace` step
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
