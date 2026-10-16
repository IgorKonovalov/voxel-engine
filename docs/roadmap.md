# Roadmap

The high-level plan: what the engine is, the decisions that shape it, and the milestone sequence.
Each milestone becomes one or more numbered plans in `docs/plans/` when it is drafted. Numbers in
this document are **starting proposals** to be checked against measurements, not specs; the
budgets they feed are in [`nfr.md`](nfr.md).

Written 2026-10-16 from the founding interview. Revise this file when a milestone closes or a
decision changes; a changed decision also gets an ADR.

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
| Raycast, sweeps, point and region material queries | P0 | M1 (raycast, region counts), M3 (sweeps) |
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
| Volumetrics, scripted collapse, loot highlighting, dedicated servers | P2 | M8, M8, game-side, M9+ |

## Storage shape (starting proposal, fixed by Plan 0001's ADR)

- **Voxel:** one `u16` material id. Material 0 is air. Flags such as indestructible live in the
  material table, not per voxel, unless a plan shows a need.
- **Brick:** 8 × 8 × 8 voxels, 1 KiB. A brick whose voxels are all one material is stored as that
  one value, not as 1 KiB. Caves are mostly uniform rock or air, so most bricks are uniform.
- **Chunk:** 64 × 64 × 64 voxels = 8 × 8 × 8 bricks, 6.4 m at 10 cm. The chunk is the unit of
  generation, paging, upload, dirty tracking and edit persistence. It carries LOD mips (32³, 16³,
  8³, majority material) for rays beyond the full-detail radius and for far chunks that are
  resident only at LOD.
- **Scale check:** a 100 m full-detail radius is about 32 chunks across, roughly 33 k chunks. If
  every brick were non-uniform that would be 17 GB, so the budget relies on uniformity: the
  working assumption is under 10 % non-uniform bricks, about 1.7 GB, and the VRAM budget in
  `nfr.md` is 3 GB with eviction to LOD when exceeded. Plan 0001 measures this on a real cave.
- **Edits** address voxels in integer coordinates; shape parameters are fixed-point (1/256
  voxel). A sphere test is `dx² + dy² + dz² <= r²` in 64-bit integers. Results are per-material
  counts plus the touched chunk set.
- **Coarse grids** (for the consumer's noise model and navigation) are derived per chunk from
  bricks and updated incrementally from the touched set.

## Milestones

Each milestone names its headless proof (runs in CI without a window), its visible proof (what
you see in `apps/flythrough`), and the budgets it touches. Order is a recommendation; M3 and M4
are independent of M2, and M5 can move ahead of M4 if the consumer's server/client work needs it.

### M0: Bootstrap (done 2026-10-16)

Workspace skeleton, harness, ADR-0001, this roadmap, `nfr.md`. No engine code.

### M1: Fly-through with digging (Plan 0001)

**Goal:** a window showing a ray-marched, streaming voxel cave lit by a headlamp. You fly, carve
spheres, and the per-material voxel counts print to the console.

- `vxe-world`: brick map and chunk store; material table (id, name, albedo, roughness, emission,
  hardness, flags, opaque payload); `WorldSource` trait, `(seed, chunk coord) -> chunk`;
  streaming around a focus point on a background thread pool with eviction; edit ops (sphere,
  box, capsule, additive and subtractive) in integer math returning results; CPU raycast (DDA)
  and region material counts; dirty tracking for upload; LOD mips.
- `vxe-render`: incremental brick upload; primary-ray DDA in compute; one spot light with a
  shadow ray; LOD rays beyond a distance; blit to the swapchain.
- `vxe-app`: window, keyboard and mouse, frame loop, surface resize.
- `apps/flythrough`: a noise-cave `WorldSource`; prints edit results.
- **Headless proof:** a fixed op list on a fixed seed hashes to the same voxels and the same
  per-material counts on the Linux and Windows CI arms; raycast hits known voxels; streaming
  evicts and reloads a chunk with its edits intact.
- **Visible proof:** a golden image on lavapipe of a fixed view after a fixed carve.
- **Budgets:** world VRAM and RAM at the full-detail radius; edit cost (100 r = 1 m carves per
  frame); primary-ray frame time at 1080p on the dev box and on the baseline when available.
- **ADRs expected:** storage layout.

### M2: Lighting that holds in the dark

**Goal:** the consumer's P0 rendering bar. Few lights, emissive voxels, destruction, no ghosting.

- Light extraction from bricks (emissive voxels become light samples; emission changeable at
  runtime by material table edits or per-voxel override).
- One-sample path tracing with next-event estimation; temporal accumulation; a spatiotemporal
  denoiser (SVGF-family) whose history is invalidated in regions touched by edits.
- Exposure, tone mapping, an open-source upscaler ported to WGSL.
- **Headless proof:** irradiance in a closed test box matches an analytic value within a stated
  tolerance; a carve invalidates exactly the touched history region.
- **Visible proof:** golden images of a dark cave with one emissive vein, before and after a
  carve, with a stated SSIM floor against the reference.
- **Budgets:** the full frame at 1080p output on the baseline GPU.
- **ADRs expected:** denoiser family; upscaler choice.

### M3: Collision, character controller, coarse grids

- `vxe-sim`: raycasts and capsule sweeps against bricks; a character controller with steps,
  slopes and climbing out of holes; shape queries for tool reach.
- `vxe-world`: a 1 m occupancy and dominant-material grid, updated incrementally from touched
  chunks; region change notifications; walkable-space extraction.
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
- **VRAM at the full-detail radius** rests on a uniformity assumption. Mitigation: Plan 0001
  measures it and the LOD eviction path exists from M1.
- **`wgpu` feature gaps.** Mitigation: the renderer's needs list (subgroups, 64-bit atomics,
  bindless) is checked against the baseline adapter in M1, and any vendor path is an ADR.
- **Determinism across platforms** rests on integer edit math and on scripting hosts configured
  for it. Mitigation: CI asserts hashes on both arms from M1 on.

## What the engine will not do

- Own gameplay: loot, monsters, items, levels, AI, a gameplay noise model. The consumer builds
  those on the engine's queries and results.
- Be a framework: no ECS, no scene editor, no general asset pipeline.
- Structural physics or rigid-body debris.
- Hardware ray tracing or vendor upscalers, unless a later ADR adds a vendor path.
- macOS testing or gating, until a consumer needs it.
