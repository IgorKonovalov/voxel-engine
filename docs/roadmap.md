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
- **A logic boundary** so a game's server logic is a component behind one trait, developed
  natively and shipped as WebAssembly (ADR-0010); a Lua host for client-side scripts is
  deferred until a consumer asks.

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
| Logic as a component (added 2026-10-16, ADR-0010) | the game's whole server logic implements one `Logic` trait (inputs in, edits and queries through a `Host` handle, opaque bytes out); the same crate links natively for development and ships as a WebAssembly component; the logic produces a level layout and the engine's parallel generator turns it into voxels; Lua is deferred | an engine-owned loop with script hooks; native-only logic; Lua for server logic; generation inside the component |
| Entities | voxel models (MagicaVoxel-style volumes) on rigs of rigid parts, traced in the same pipeline as the world | skinned triangle meshes with a hybrid rasterize + RT path; deciding later |
| Repo | public GitHub, MIT OR Apache-2.0 | private; local only |
| Harness | Ritmolux-style: ADRs, phased plans, architect/dev skills, deny hooks, CI gates | CLAUDE.md + ADRs only |
| Surface look (added 2026-10-16) | smooth and natural rock, not visible cubes, with the Sibenik cathedral figure of Laine & Karras 2010 (ESVO, Figure 1) as the quality reference: not photorealism, but that level of believable surface and soft light; the mechanism (per-voxel normal smoothing, per-voxel contour planes in the GPU mirror, procedural per-material surface detail, or another) is chosen by measurement after M1, balancing image, frame cost and simplicity | a blocky aesthetic; photorealism; committing to ESVO-style contours before anything is on screen |
| VR readiness (added 2026-10-16) | PC VR is a possible later target, so the renderer keeps three doors open from M2 on: a view is an asymmetric frustum (origin, basis, four tangent half-angles) and the frame renders N views, not one camera with a symmetric field of view; lighting is view-independent or shareable across views where the cost is equal; presentation is decoupled from the window surface. Actual VR support (OpenXR, 90 Hz, two eyes) is an unscheduled milestone with its own hardware floor, never a promise on the baseline GPU. See "Unscheduled: PC VR" below | building OpenXR support now; standalone headsets (mobile GPU, Android); treating VR as a mode of the baseline budget |
| Physics (added 2026-10-16) | **voxel dynamics, not a rigid-body engine.** Collapse is emergent and authoritative: `vxe-world` answers which region lost its connection to the anchor after an edit, and deterministic ops decide what the world looks like afterwards. Fall damage is a controller event. Debris and falling pieces may be animated client-side by a rigid-body library as a cosmetic layer that never writes to the world. Revises M8 and the consumer's "scripted collapse only" wording | a general physics engine (`rapier`) in the authoritative world; scripted-only collapse; no collapse at all |
| Floor, profiles and scene scale (added 2026-10-17) | the GTX 1660-class floor of `nfr.md` §1 stays the 60 fps gate at 1080p with an upscaler; a named **portable profile** (Switch-2-class: about 1.7 to 3 TFLOPS, 68 to 102 GB/s, 9 GB shared) is a target of the quality settings (internal resolution, full-detail radius, translucent layers, lighting tier), at 720p and 30 to 60 fps; 120 fps is welcome where a fast machine reaches it through a performance profile, not a gate. Lighting has two tiers: direct light by one shadow ray per light at half resolution with blur and temporal reprojection on the floor, ReSTIR direct lighting where the budget allows, and indirect light from probes in both. Glass, water and lava are translucent voxels in the brick map; mist, smoke and fire are a separate coarse volume. Open-sky and kilometre-scale scenes are in scope, so the far-field clipmap is a scheduled plan, not a maybe. The brick representation that serves all of this is ADR-0012 | Switch 1 (about 0.4 TFLOPS, 25.6 GB/s shared, no public graphics API for `wgpu`); raising the floor to the dev laptop; 120 fps as a budget; media as voxels in the brick map |

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
| Collapse (emergent, per the physics decision above), loot highlighting, dedicated servers | P2 | M8 (support query, collapse ops, cosmetic debris), M6 (emissive entities; the highlight itself is game-side), M9+ |
| Fall damage | P0 (implied by the controller) | M3: the controller reports the landing speed; the damage rule is the game's |

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
  the first consumer needs it. M1 ships an LOD-only ring of mips around the full-detail sphere
  (ADR-0003), out to `r_lod` chunks (about 154 m at the default 24); the clipmap is its own
  later plan, and until then the far field beyond the ring is simply not drawn.
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

### M1: Fly-through with digging (Plans 0001 and 0002; done 2026-10-16, with recorded misses)

Plan 0001 closed 2026-10-16 (v0.2.0): the bounded half, with the determinism vector green on
both CI arms. Plan 0002 closed 2026-10-16 (v0.3.0): streaming residency, the LOD-only ring,
edit persistence and the narrowed, sharded mirror, all tested headless. M1 is done with
**recorded misses on two `nfr.md` §2 lines** (owner's ruling at the Plan 0002 close): the
streaming line (render-thread CPU p50 about 7 ms, p99 52 to 61 ms against 1.0 ms) and primary
rays in the streamed world (6.69 ms at 1080p against 3.0 ms), both on the dev reference. The
budgets stand. The §4 edit-cost miss carries forward from Plan 0001 to Plan 0003 (closed 2026-10-17,
v0.4.1), which cut the `worst` bench from 4.6 to 3.56 ms on a quiet dev box with no result or
hash moved; the owner kept the 2 ms figure. Plan 0014 (closed 2026-10-17, v0.7.0) is the second pass: a brick-array pool, a sphere span walk and an analytic capsule row cut `worst` to 3.19-3.26 ms (3.03-3.05 ms with a warm pool) and `tool` from 8.05 to 6.78-6.80 ms. The §4 miss carries forward: the row write bounds the cost inside ADR-0002's representation, and the decision between a representation ADR and a measured §4 cap awaits the owner.

**Next, before M2's lighting:** a streaming-CPU plan (incremental queue, residency sweep and
window shift; narrowing and staging on workers) and a march-pass diagnosis spike (step counts,
the 4096-step cap, LOD radii; an autopilot path through open cave). Neither is drafted yet.
Plan 0002's Followups list the rest.

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
- `vxe-world` (Plan 0002, ADR-0003): sphere residency around one or more focus points on a
  background thread pool, in two tiers: full detail inside `r_full`, and an LOD-only ring out
  to `r_lod` that keeps only mips (32³, 16³, 8³), not bricks. Edited chunks are persisted on
  eviction, in RAM with disk spill. Plan 0001's `Full` fraction reading (about 21 %, not 10 %)
  is why the ring replaces bricks instead of adding mips to every resident chunk.
- `vxe-render`: incremental per-brick upload; primary-ray DDA in compute over the brick
  occupancy masks; one spot light with a shadow ray; blit to the swapchain. Plan 0002
  (ADR-0004): a wrapped window following the camera, budgeted batched uploads, an 8-bit
  per-chunk palette, a pool sharded over up to four bindings, and a mip walk for LOD-only
  chunks.
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

### M2: Lighting that holds in the dark (in progress: Plan 0005 closed; Plan 0013 is the surface follow-up; Plan 0015 drafts the traversal hierarchy; the lighting plan next)

Plan 0015 (draft, 2026-10-17) comes before the lighting plan: it gives the marcher a
hierarchy of occupancy bits (chunk bits, 512-bit brick masks, a walk that reads one line per
level) and the translucent material class, on today's pool, as the first step of ADR-0012.
Every lighting pass casts rays through that walk, so its cost per ray is settled first.

Plan 0005 closed 2026-10-16 (v0.4.0): the surface-look spike measured four normal modes and
procedural detail, and the owner chose per-voxel contour planes with weaker rock detail
(ADR-0011). The fly-through's default stays face normals until a surface follow-up plan fixes
contour's shadow-ray origin and fits its streamed footprint, which ran the 8 GiB dev card out
of memory at the default radius; Plan 0005's "Follow-up items" list the work. Plan 0013
(draft) is that follow-up: the shadow-ray origin, a footprint chosen by measurement and recorded
in its own ADR, the owner's rock detail, then contour plus detail as the default. The lighting
plan is not drafted yet.

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
- The view generalisation from the VR-readiness decision: the marcher takes a list of views,
  each an origin, a basis and four tangent half-angles (left, right, up, down), and every stage
  that keeps history (accumulation, denoiser, upscaler) keeps it per view. On a flat screen the
  list has one symmetric entry and nothing costs more. This lands in the lighting plan because
  that plan rewrites the marcher's parameter block and creates the history buffers; doing it
  later would touch every stage twice.
- **Headless proof:** irradiance in a closed test box matches an analytic value within a stated
  tolerance; a carve invalidates exactly the touched history region.
- **Visible proof:** golden images of a dark cave with one emissive vein, before and after a
  carve, with a stated SSIM floor against the reference.
- **Budgets:** the full frame at 1080p output on the baseline GPU.
- **ADRs expected:** denoiser family; upscaler choice. Surface representation: ADR-0011; its
  streamed footprint: a new ADR at Plan 0013 Phase 4.

### M3: Collision, character controller, coarse grids (Plan 0004; done 2026-10-17)

Plan 0004 closed 2026-10-17 (v0.5.0): `vxe-sim` sweeps a vertical cylinder (ADR-0005, not a
capsule) and drives a controller with step, slide, fall, jump and ledge climb, pinned by a
course hash; `vxe-world` keeps the coarse grid and walkable map from edit results; the
fly-through walks on the bounded level and the owner drove it by hand. Open: the sweep cost
(`nfr.md` section 8, awaiting the owner's call), depenetration, and wiring the grid to the
streamer; Plan 0004's Followups list them. Plan 0009 (collapse) is unblocked.


- `vxe-sim`: capsule sweeps against bricks (the raycast stays in `vxe-world`, which owns all
  primitive queries); a character controller with steps, slopes and climbing out of holes;
  shape queries for tool reach.
- `vxe-world`: a one-cell-per-brick (0.8 m) occupancy and dominant-material grid, updated
  incrementally from touched chunks; region change notifications; walkable-space extraction.
- **Headless proof:** a capsule walking a known course ends where the fixture says; a carve
  under its feet changes the coarse grid cell and emits one notification.
- **Visible proof:** walking instead of flying in the fly-through.

### M4: Spatial audio (Plan 0006; drafted)

- `vxe-audio`: output through `cpal`, a mixer, HRTF spatialization, occlusion and transmission
  by ray probes through the material table's acoustic triple, early reflections from ray probes,
  hooks for material-driven synthesis. Separate from any gameplay noise model, which a game
  builds on M3's coarse grid.
- **Headless proof:** a source behind 5 m of a material with known transmission arrives at the
  listener attenuated by the table's value; an open corridor yields a reflection at the expected
  delay.
- **Visible proof:** a knock in the fly-through sounds different behind thin and thick walls.
- **ADRs expected:** HRTF source and audio thread model.

### M5: Simulation split and the logic component (done: Plans 0007, v0.10.0, and 0011, v0.11.0)

Plan 0007 closed 2026-10-17 (v0.10.0): `vxe-net` (byte channel, server tick, shared and
owned clients with desync detection), `vxe-world`'s op and input logs, log file and wire, and
the fly-through's digs, fills and bench through a server, saved and replayed under `--level`
(ADR-0007, accepted). Followup from the close: op kinds 2 to 4 (stamp, drop, remove) in
`Op`, the wire and `Host`, with the fly-through's stamp and collapse moving into `DigLogic`, so
a saved session no longer refuses them; a stamp entry needs a prefab reference the log can
name, likely an ADR.

Plan 0011 closed 2026-10-17 (v0.11.0): `vxe-script-wasm` hosts a logic component under
deterministic WASI (clocks fixed to the tick, always-ready timers, seeded entropy), fuel per
call and a total memory cap, and stops a logic after its first failed call (ADR-0010,
accepted). The fly-through's dig logic (`flythrough-logic`) runs natively or, with
`--features wasm` and `--logic wasm`, as a component, to the same hash. Followups from the
close: bounded or metered host-call work, with `vxe-world` range checks on shapes; a component
hash and the `WasmConfig` in the log header; batched query imports; a per-call measurement of
a cheap query.


- An authoritative world plus op and input logs on the server side; clients send inputs, a
  `Logic` behind a trait decides the edits through a `Host` handle; a client mirror fed by
  ops over an in-memory channel, sharing the world in solo; replay from seed plus ops, or
  re-simulation from seed plus inputs (ADR-0007, ADR-0010).
- `vxe-script-wasm`: a `wasmtime` host implementing `Logic` for a component built from the
  same crate as the native logic, with deterministic configuration (canonical NaNs, clocks
  and entropy from the tick, fuel per tick, a memory cap).
- Deferred: a Lua host for client-side scripts, until a consumer asks. Follow-ups named by
  the plans: the level-layout format the logic produces and the generator consumes; entity
  snapshots across the boundary (with M6); the server owning streaming focus points.
- **Headless proof:** a replayed log reproduces the world hash; the native and component
  builds of one logic produce byte-equal op logs on both CI arms.
- **ADRs:** ADR-0007 (log, channel, client modes) and ADR-0010 (logic as a component), both
  accepted at their plans' closes.

### M6: Entities as voxel models (Plan 0008 closed the `.vox` import; the render half follows Plan 0005)

- `vxe-assets`: `.vox` import with material mapping. Done in Plan 0008 (v0.6.0).
- `vxe-render`: instanced voxel volumes with transforms traced in the same pipeline (a small
  top-level structure of AABBs walked in compute), rigid-part rigs, consistent lighting and
  shadows, emissive entities.
- **Visible proof:** a golden image of an animated model casting a shadow on the world.
- **ADRs expected:** entity representation.

### M7: Tooling (Plan 0008 closed prefab stamping (runtime and generation time); Plan 0010 closed the stats HUD, v0.9.0; debug views, the profiler across passes and the benchmark lane remain)

- Prefab stamping with rotation and masks; debug views (material ids, acoustic properties,
  coarse grids, flags); an in-app profiler overlay with GPU timestamps; a benchmark lane
  (`criterion` for CPU, timestamp queries for GPU) with readings committed per named machine.
- Follow-up: once Plan 0007 closes, the runtime stamp goes through `Server` as op kind 2
  (reserved there), so a stamp is a logged, replayable op like an edit.

### M8: Volumetrics, debris and collapse (support query and collapse ops shipped in Plan 0009, v0.8.0; volumetrics, debris and the cosmetic rigid-body layer remain open, the last after M6)

- Dust, fog, mist and smoke as a separate coarse volume (a froxel grid or a per-brick density
  grid, marched at half resolution with a fixed step count and composited over the opaque
  hit; owner's call, 2026-10-17), never as voxels in the brick map; short-lived particle
  debris from edits. Translucent solids (glass, water, lava) are voxels of a translucent
  material class instead, composited by Plan 0015's bounded walk.
- **Support query** in `vxe-world`: after an edit, the set of coarse-grid cells (0.8 m, from
  M3) that are solid but no longer connected to the anchor (the indestructible shell, the
  world boundary, or an explicit anchor set the game supplies). A capped flood (ADR-0008)
  from the edit's changed cells over load-bearing cells, integer, deterministic and holding
  nothing between calls, so a server tick can run it after every carve. Returns regions as
  cell sets with their voxel counts per material, the same shape as an edit result. Shipped
  in Plan 0009.
- **Collapse ops**: translate a region's voxels straight down in place, as one rigid piece,
  by the integer air gap under its columns (at most a per-call maximum, so a per-tick call is
  an animated fall). Translation only, no rotation, so the move is lossless and the outcome is
  bit-identical on both CI arms. A second op removes an unsupported region outright and fills
  the cells below with a rubble material. The game picks one per region from the support
  query's answer. Both are ordinary ops: they return per-material counts and the touched
  region, and replicate through M5's log once they are wired as op kinds 3 and 4 (a
  followup of Plan 0009). Shipped in Plan 0009 as `World::drop_region` and
  `World::remove_region`.
- **Cosmetic rigid bodies**: a client-side layer, in its own optional crate, that animates the
  pieces of a collapsed region with a rigid-body library (`rapier` or similar) between the
  server's "region R fell at tick T" and the landed result, then fades the pieces into the
  authoritative world. It never writes to `vxe-world` and nothing deterministic reads it.
- **Headless proof:** carving the single pillar under a 3 × 3 cell slab returns that slab as
  one unsupported region with its exact material counts; the drop op lands it on the floor
  with every voxel in place and the same hash on both CI arms.
- **ADRs expected:** voxel dynamics versus a physics engine (the rejected alternatives are a
  general rigid-body engine in the authoritative world, scripted-only collapse, and rotating
  pieces): ADR-0008, accepted at Plan 0009's close.

### M9: Networking

- `vxe-net`: a QUIC listen server (`quinn`), prediction and reconciliation for local edits,
  entity replication with interest management, late join by seed plus compressed op log,
  positional voice as an audio source through M4's occlusion.
- **ADRs expected:** transport; voice codec.

### Unscheduled: PC VR

Not a numbered milestone and not on the first consumer's list. Recorded so that the doors the
VR-readiness decision keeps open stay open, and so the cost is on paper before anyone asks.

- **What favours the engine.** Compute ray marching has no per-view vertex work: a second eye
  is a second ray origin and basis, and the head pose is a uniform written just before the
  dispatch, so late latching is free. The marcher picks its own rays, so foveation is a
  dispatch pattern over tiles rather than a vendor extension. HRTF audio (M4) is already
  head-relative. Voxel-model entities (M6) trace in the same pass, so there is no second
  pipeline to make stereo.
- **What it would take.** A `vxe-xr` shell next to `vxe-app` that owns the OpenXR session and
  swapchains (`openxr` crate, Vulkan handles exchanged through `wgpu-hal`), hands the renderer
  two views per frame and submits the textures with depth for the runtime's reprojection; a
  VR profile in `nfr.md` (two eyes, 90 Hz, a named high-end card) that is its own floor; and a
  lighting path that both eyes share, which is the M2 fallback (per-chunk irradiance probes or
  cone tracing over the mips), since 1 spp path tracing with a temporal denoiser is what head
  motion punishes most.
- **The hard number.** Pixel rate. A Valve Index at 90 Hz is about 3.3× the pixels per second
  of 1080p at 60 Hz; a Quest 3 over Link about 6.6×. Ray-march cost is linear in pixels, so
  VR on the baseline GPU of `nfr.md` §1 is not a goal and never will be; a VR plan names its
  own card.
- **Prerequisites before a plan is drafted:** M2 closed with the view list in place, M6 for
  hands and tools as entities, and a headset on the dev box.

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
- Run a rigid-body engine in the authoritative world. Collapse is voxel dynamics (M8): a
  support query and deterministic ops. A rigid-body library may animate debris client-side,
  as a cosmetic layer that nothing deterministic reads.
- Promise VR on the baseline GPU. VR readiness is a set of doors kept open (see the decisions
  table and "Unscheduled: PC VR"); VR itself names its own hardware floor.
- Hardware ray tracing or vendor upscalers, unless a later ADR adds a vendor path.
- macOS testing or gating, until a consumer needs it.
