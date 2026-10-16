# Roadmap

The high-level plan: what the engine is, the decisions that shape it, and the milestone sequence.
Each milestone becomes one or more numbered plans in `docs/plans/` when it is drafted. Numbers in
this document are **starting proposals** to be checked against measurements, not specs; the
budgets they feed are in [`nfr.md`](nfr.md).

Written 2026-10-16 from the founding interview. Revise this file when a milestone closes or a
decision changes; a changed decision also gets an ADR. The external techniques and numbers the
storage, LOD and lighting proposals below lean on are collected, with links, in
[`references/voxel-optimizations.md`](references/voxel-optimizations.md).

## What we are building

A game-agnostic voxel engine, as a Rust library workspace (ADR-0001):

- A **streaming world** of small voxels stored as a chunked brick map with LOD mips, paged
  around focus points, generated on background threads by a source the game provides.
- A **compute ray marcher** on `wgpu` that lights the world from few sources, including emissive
  voxels, and denoises and upscales to the baseline GPU's budget.
- **Deterministic edit operations** that return what they changed, so a game can hang gameplay
  on destruction and replicate edits as compact ops.
- **Spatial queries** (raycast, sweeps, region counts, coarse grids) that serve collision,
  acoustics and navigation from the same data.
- **Scripting hosts** (WASM for deterministic server-side logic, Lua for client-side scripts)
  so a game's rules can be data and scripts rather than engine code.

What the engine is not: a framework. No ECS, no scene editor, no asset pipeline beyond voxel
model import. A game owns its loop and composes the crates it needs.

## Decisions from the founding interview

Settled on 2026-10-16. The first three are recorded in ADR-0001; the rest get their ADR with the
plan that implements them (see [`adrs/README.md`](adrs/README.md)).

| Topic | Decision | Rejected |
|---|---|---|
| GPU path | `wgpu` + WGSL, DDA ray marching in compute, own denoiser and upscaler | native Vulkan + hardware RT; `wgpu` with a native escape hatch |
| Engine shape | library workspace, no framework, optional thin `winit` shell | Bevy plugins; a full framework with its own ECS and loop |
| World model | streaming unbounded from day one; full detail hundreds of meters at 10 cm, LOD to about 1 km; voxel size one scalar per world | bounded volumes only; kilometers at full detail; multi-resolution voxels |
| Platforms | Linux + Windows, 2019 mid-range desktop GPU baseline, 6 GB VRAM | + macOS; RTX/RDNA2-only |
| First milestone | a fly-through with digging, built on a headless world crate that is tested without a window | headless-only first; sound graybox first |
| API surface | Rust crates by git dependency, plus scripting: **WASM (wasmtime) for deterministic server logic, Lua (mlua) for client-side scripts** | Rust only; a C ABI; a single runtime for both |
| Entities | voxel models (MagicaVoxel-style volumes) on rigs of rigid parts, traced in the same pipeline as the world | skinned triangle meshes with a hybrid rasterize + RT path; deciding later |
| Repo | public GitHub, MIT OR Apache-2.0 | private; local only |
| Harness | Ritmolux-style: ADRs, phased plans, architect/dev skills, deny hooks, CI gates | CLAUDE.md + ADRs only |
| Surface look (added 2026-10-16) | smooth and natural rock, not visible cubes, with the Sibenik cathedral figure of Laine & Karras 2010 (ESVO, Figure 1) as the quality reference: not photorealism, but that level of believable surface and soft light; the mechanism (per-voxel normal smoothing, per-voxel contour planes in the GPU mirror, procedural per-material surface detail, or another) is chosen by measurement after M1, balancing image, frame cost and simplicity | a blocky aesthetic; photorealism; committing to ESVO-style contours before anything is on screen |

## The first consumer's needs, mapped

From `~/Work/voxel-roguelite/docs/engine-requirements.md` (P0 = first playable slice, P1 =
before content production, P2 = later). The consumer's first playable slice needs M1 through M5.

| Requirement area | Priority | Milestone |
|---|---|---|
| Bounded level volumes, 10 cm voxels, sparse storage, 16-bit material id | P0 | M1 |
| Carve ops (sphere, box, capsule, tool-along-ray) returning per-material counts; indestructible flag | P0 | M1 |
| First-person camera, stable close-range image | P0 | M1 |
| Dark scenes with few lights, emissive voxels as runtime-changeable lights, denoising under destruction | P0 | M2 |
| Material table: visuals, hardness, acoustic triple, opaque payload | P0 | M1 (table), M4 (acoustics used) |
| Raycast, sweeps, point and region material queries | P0 | M1 (raycast, point and region queries), M3 (sweeps) |
| Character controller against voxels | P0 | M3 |
| Coarse 1 m occupancy grid, incrementally updated | P0 | M3 |
| 3D positional audio with HRTF; occlusion and transmission through voxels | P0 | M4 |
| Generation as a pure function of (seed, index, version); explicit seeds everywhere | P0 | M1 |
| Server/client split in-process; edits as deterministic compact ops; offline | P0 | M1 (ops), M5 (split) |
| Stacked levels generated in the background; prefab stamping with masks | P1 | M1 (streaming covers background generation), M7 (stamping) |
| Additive edits; debris particles; region change notifications; walkable-space extraction | P1 | M1 (additive), M8 (debris), M3 (notifications, walkable) |
| Early reflections and reverb; material-driven synthesis hooks | P1 | M4 |
| Dynamic entities lit consistently | P1 | M6 |
| Listen server, prediction, replication, late join, positional voice | P1 | M9 |
| `.vox` import, debug views | P1 | M6 (import), M7 (views) |
| Volumetrics (dust, fog) | P1 | M8, a deliberate deferral: it needs M2's marcher and denoiser first |
| Scripted collapse, loot highlighting, dedicated servers | P2 | M8, M6 (emissive entities; the highlight itself is game-side), M9+ |

## Storage shape (starting proposal, fixed by Plan 0001's ADR)

- **Voxel:** one `u16` material id and nothing else. Material 0 is air. Every per-voxel
  property is a property of its material row: the indestructible flag, emission, hardness. A
  voxel that must differ from its neighbours of the same substance (the prefab's indestructible
  shell, the crystal that lights up when sound reaches it) is a second material row (`limestone
  shell`, `crystal lit`), and changing it at runtime is a replace-material edit op, not a
  per-voxel override. The consumer's shell mask and runtime emission both resolve this way.
- **Brick:** 8 × 8 × 8 voxels, 1 KiB. A brick whose voxels are all one material is stored as that
  one value, not as 1 KiB. Caves are mostly uniform rock or air, so most bricks are uniform. The
  GPU mirror keeps a 64-bit occupancy mask per brick (one bit per 2 × 2 × 2 block) so the marcher
  skips empty space without reading voxels; the `u16` width is the CPU truth, and the mirror may
  narrow to a per-chunk palette index where Plan 0001's measurement says bandwidth needs it.
- **Chunk:** 64 × 64 × 64 voxels = 8 × 8 × 8 bricks, 6.4 m at 10 cm. The chunk is the unit of
  generation, paging, upload, dirty tracking and edit persistence. It carries LOD mips (32³, 16³,
  8³) for rays beyond the full-detail distance inside the resident set. Mips are as sparse as
  bricks: a uniform chunk stores none, and a mip cell is an occupancy bit plus a one-byte
  palette or majority index, never a `u16`. At full width the three mips are about 73 KiB per
  chunk, which across the full-detail cube is about 2.3 GiB on their own, more than the bricks;
  at one byte they are under 300 MB, and sparsity cuts that further.
- **Far field.** "LOD to about 1 km" is not reachable by paging chunks at LOD: a 1 km radius is
  about 30 million chunks, and generating them at full detail to downsample is a thousand times
  the full-detail work. The far field is a clipmap: rings of coarser cells with their own paging
  unit and a `WorldSource` method that produces a cell at a coarser level directly. Nothing in
  the first consumer needs it. M1 ships mips inside the resident set; the clipmap is its own
  later plan, and until then the far field is simply not drawn.
- **Scale check:** a 100 m full-detail radius is about 32 chunks across, roughly 33 k chunks. If
  every brick were non-uniform that would be 17 GB, so the budget relies on uniformity: the
  working assumption is under 10 % non-uniform bricks, about 1.7 GB, and the VRAM budget in
  `nfr.md` is 3 GB with eviction to LOD when exceeded. Plan 0001 measures this on a real cave.
  The first consumer's bounded level is about 3200 chunks, a tenth of that.
- **Edits** address voxels in integer coordinates; shape parameters are fixed-point (1/256
  voxel). A sphere test is `dx² + dy² + dz² <= r²` in 64-bit integers on voxel centres, so a
  radius-10 sphere in solid material covers exactly 4169 voxels (the lattice count; the
  continuous volume is about 4189). Results are per-material counts plus the touched chunk set.
  Ops are sphere, box, capsule, tool-along-ray (a capsule from the ray origin to its hit),
  additive, subtractive and replace-material. An op whose shape touches a non-resident chunk
  fails with that chunk's coordinate rather than generating it synchronously; the caller (the
  server's focus points) keeps the chunks it edits resident.
- **Coarse grids** (for the consumer's noise model and navigation) have one cell per brick
  (0.8 m), so a cell never straddles a chunk and a uniform brick's single value is its occupancy
  for free. They are derived per chunk and updated incrementally from the touched set. The
  consumer asked for "about 1 m"; 0.8 m is within that.
- **World access model.** By M4 the generator pool writes chunks, the render thread uploads
  them, the audio thread probes them and the game queries them. The chunk store is a concurrent
  structure with per-chunk interior locking or snapshot swapping, decided in Plan 0001's ADR
  alongside the layout, because it shapes every later crate.
- **Generation determinism.** `WorldSource` is the game's, but anything the engine ships (the
  fly-through's cave, the CI vector) uses integer hashing and integer noise, or a pure-Rust
  `libm`. Platform transcendentals (`sin`, `exp`, `powf`) differ in the last bit between glibc
  and MSVC and would break the cross-arm hash by luck rather than by design.

## Milestones

Each milestone names its headless proof (runs in CI without a window), its visible proof (what
you see in `apps/flythrough`), and the budgets it touches. Order is a recommendation; M3 and M4
are independent of M2, and M5 can move ahead of M4 if the consumer's server/client work needs it.

### M0: Bootstrap (done 2026-10-16)

Workspace skeleton, harness, ADR-0001, this roadmap, `nfr.md`. No engine code.

### M1: Fly-through with digging (Plan 0001)

**Goal:** a window showing a ray-marched, streaming voxel cave lit by a headlamp. You fly, carve
spheres, and the per-material voxel counts print to the console.

M1 is two plans. Plan 0001 is the bounded walking skeleton: brick map, edits, queries, renderer,
a finite source. Plan 0002 adds streaming, eviction and LOD mips on top. The storage ADR is
written with Plan 0001 and already carries chunk addressing for paging, so Plan 0002 changes no
layout.

- `vxe-world` (Plan 0001): brick map and chunk store with the access model above; material
  table (id, name, albedo, roughness, emission, hardness, flags, opaque payload); `WorldSource`
  trait, `(seed, chunk coord) -> chunk`; the edit ops listed under "Storage shape" in integer
  math returning results; CPU raycast (DDA), point query and region material counts; dirty
  tracking for upload.
- `vxe-world` (Plan 0002): streaming around one or more focus points on a background thread
  pool with eviction and edit persistence for paged-out chunks; LOD mips.
- `vxe-render`: incremental per-brick upload; primary-ray DDA in compute over the brick
  occupancy masks; one spot light with a shadow ray; LOD rays beyond a distance (Plan 0002);
  blit to the swapchain.
- `vxe-app`: window, keyboard and mouse, frame loop, surface resize.
- `apps/flythrough`: an integer-noise cave `WorldSource`; prints edit results.
- **Headless proof:** a fixed op list on a fixed seed hashes to the same voxels and the same
  per-material counts on the Linux and Windows CI arms; a radius-10 carve in solid material
  returns the lattice count; raycast hits known voxels; an op on a non-resident chunk fails
  with that coordinate; streaming evicts and reloads a chunk with its edits intact
  (Plan 0002).
- **Visible proof:** a golden image on lavapipe of a fixed view after a fixed carve.
- **Budgets:** world VRAM and RAM at the full-detail radius, bricks and mips separately; edit
  cost (100 r = 1 m carves per frame) and their upload; generation throughput per core;
  primary-ray frame time at 1080p on the dev box and on the baseline when available.
- **ADRs expected:** storage layout and world access model (one ADR, Plan 0001).

### M2: Lighting that holds in the dark

**Goal:** the consumer's P0 rendering bar. Few lights, emissive voxels, destruction, no ghosting.
The quality reference is the Sibenik cathedral in Laine & Karras 2010, Figure 1. That image is
5 mm voxels with a unique colour and precomputed ambient occlusion per voxel, 2.7 GB on the GPU:
twenty times our resolution and shading data we do not store. At 10 cm and one material id per
voxel the same impression has to come from three cheaper places, each measured in its own
spike: the silhouette (contours or normal smoothing), the surface texture (procedural
per-material detail evaluated in the shader from world position, the way Teardown's materials
work), and the light (real-time GI standing in for the baked occlusion).

- Light extraction from bricks: emissive voxels become light samples, and a vein of thousands
  of them is sampled by reservoir resampling (ReSTIR-style) rather than a flat light list.
  Emission changes at runtime through the material table or a replace-material op.
- One-sample path tracing with next-event estimation; temporal accumulation; a spatiotemporal
  denoiser (SVGF-family) whose history is invalidated in regions touched by edits, measured
  against A-SVGF's gradient test before the cheaper invalidation is kept.
- A named fallback before the plan starts, in case 1 spp path tracing misses the baseline
  budget: voxel cone tracing over the mips we already own, or irradiance probes per chunk.
- The smooth-surface spike: normal smoothing against per-voxel contour planes (ESVO's
  contours without the octree, fitted from the generator's distance field, the carve's
  analytic shape, or the 3 × 3 × 3 neighbourhood), compared on screen and by frame cost and
  VRAM on surface voxels. Plan 0001's storage ADR leaves room for an optional per-voxel
  attribute in the GPU mirror of non-uniform bricks so this spike changes no layout.
- Exposure, tone mapping, and an upscaler. FSR 2 is MIT and cheap at 1080p but has no WGSL
  port; the port is plan-sized work, and a TAA upscaler of our own is the fallback.
- **Headless proof:** irradiance in a closed test box matches an analytic value within a stated
  tolerance; a carve invalidates exactly the touched history region.
- **Visible proof:** golden images of a dark cave with one emissive vein, before and after a
  carve, with a stated SSIM floor against the reference.
- **Budgets:** the full frame at 1080p output on the baseline GPU.
- **ADRs expected:** denoiser family; upscaler choice.

### M3: Collision, character controller, coarse grids

- `vxe-sim`: capsule sweeps against bricks (the raycast stays in `vxe-world`, which owns all
  primitive queries); a character controller with steps, slopes and climbing out of holes;
  shape queries for tool reach.
- `vxe-world`: a one-cell-per-brick (0.8 m) occupancy and dominant-material grid, updated
  incrementally from touched chunks; region change notifications; walkable-space extraction.
- **Headless proof:** a capsule walking a known course ends where the fixture says; a carve
  under its feet changes the coarse grid cell and emits one notification.
- **Visible proof:** walking instead of flying in the fly-through.

### M4: Spatial audio

- `vxe-audio`: output through `cpal`, a mixer, HRTF spatialization, occlusion and transmission
  by ray probes through the material table's acoustic triple, early reflections from ray probes,
  hooks for material-driven synthesis. Separate from any gameplay noise model, which a game
  builds on M3's coarse grid.
- **Headless proof:** a source behind 5 m of a material with known transmission arrives at the
  listener attenuated by the table's value; an open corridor yields a reflection at the expected
  delay.
- **Visible proof:** a knock in the fly-through sounds different behind thin and thick walls.
- **ADRs expected:** HRTF source and audio thread model.

### M5: Simulation split and scripting

- An authoritative world plus edit log on the server side; a client mirror fed by ops over an
  in-memory channel; snapshot and replay from seed plus log.
- `vxe-script-wasm`: a `wasmtime` host for server logic with deterministic configuration
  (canonical NaNs, no clock, fuel metering), typed bindings to world queries and edits.
- `vxe-script-lua`: an `mlua` host for client-side scripts (presentation, UI hooks), hot reload.
- **Headless proof:** a replayed log reproduces the world hash; a WASM script's output is
  identical on both CI arms.
- **ADRs expected:** scripting split and binding style; channel abstraction.

### M6: Entities as voxel models

- `vxe-assets`: `.vox` import with material mapping.
- `vxe-render`: instanced voxel volumes with transforms traced in the same pipeline (a small
  top-level structure of AABBs walked in compute), rigid-part rigs, consistent lighting and
  shadows, emissive entities.
- **Visible proof:** a golden image of an animated model casting a shadow on the world.
- **ADRs expected:** entity representation.

### M7: Tooling

- Prefab stamping with rotation and masks; debug views (material ids, acoustic properties,
  coarse grids, flags); an in-app profiler overlay with GPU timestamps; a benchmark lane
  (`criterion` for CPU, timestamp queries for GPU) with readings committed per named machine.

### M8: Volumetrics, debris and scripted collapse

- Dust and fog in the ray marcher; short-lived particle debris from edits; region move and
  removal ops for scripted collapse. No structural physics.

### M9: Networking

- `vxe-net`: a QUIC listen server (`quinn`), prediction and reconciliation for local edits,
  entity replication with interest management, late join by seed plus compressed op log,
  positional voice as an audio source through M4's occlusion.
- **ADRs expected:** transport; voice codec.

## Risks

- **Denoising sparse light (M2)** is the quality risk. Mitigation: a measurable bar (golden
  SSIM, analytic irradiance) from the start, and the baseline GPU in hand before M2 closes.
- **Streaming complexity before the first pixel (M1).** Mitigation: the fly-through's source is
  trivial noise; paging is tested headless with a tiny radius.
- **VRAM at the full-detail radius** rests on a uniformity assumption, and the mips can cost
  more than the bricks if stored at full width. Mitigation: Plan 0001 measures bricks and mips
  separately, mips are sparse and one byte per cell, and the LOD eviction path exists from
  Plan 0002.
- **Generation throughput.** Keeping a 100 m full-detail radius resident at 10 m/s means about
  1200 chunks, roughly 300 M voxels, per second on the baseline's four cores. Mitigation: the
  radius is a parameter with a conservative default, Plan 0002 measures voxels per second per
  core, and the first consumer's bounded level needs no streaming at all.
- **`wgpu` feature gaps.** Mitigation: the renderer's needs list (subgroups, 64-bit atomics,
  bindless) is checked against the baseline adapter in M1, and any vendor path is an ADR.
- **Determinism across platforms** rests on integer edit math, integer generation in anything
  the engine ships, and scripting hosts configured for it. Mitigation: CI asserts hashes on
  both arms from M1 on, and `clippy.toml` can deny the platform transcendentals in `vxe-world`.
- **The upscaler port (M2).** No WGSL port of FSR 2 exists. Mitigation: the M2 plan sizes the
  port as its own phase and names the TAA fallback.

## What the engine will not do

- Own gameplay: loot, monsters, items, levels, AI, a gameplay noise model. The consumer builds
  those on the engine's queries and results.
- Be a framework: no ECS, no scene editor, no general asset pipeline.
- Structural physics or rigid-body debris.
- Hardware ray tracing or vendor upscalers, unless a later ADR adds a vendor path.
- macOS testing or gating, until a consumer needs it.
