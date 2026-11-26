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

**Two workflows, both first class (owner, 2026-11-17).** The engine aims to be best in class and
to be worked with by a person alone or by an agent alone, neither a port of the other: "I want
this engine be best in class, and with possibility to work with it for both pure human workflow,
and agentic workflow as well". For a person: readable text formats, a window that shows every
mode it is in, clear errors. For an agent: text it writes reliably, a headless build, run and
check loop with machine-readable output (run reports, diagnostics with file and line, rendered
stills, deterministic replay). A plan's review checks both. The authoring direction that follows
from it (level and logic text compiled to a logic component, a visual editor optional on top) is
ADR-0072, proposed and unscheduled.

## Decisions from the founding interview

Settled on 2026-10-16. The first three are recorded in ADR-0001; the rest get their ADR with the
plan that implements them (see [`adrs/README.md`](adrs/README.md)).

| Topic | Decision | Rejected |
|---|---|---|
| GPU path | `wgpu` + WGSL, DDA ray marching in compute, own denoiser and upscaler. Amended 2026-11-03 (Plan 0055, ADR-0034's Outcome): the FSR 2.2.1 port in WGSL is the default upscaler at 1.5, ours is opt-in | native Vulkan + hardware RT; `wgpu` with a native escape hatch |
| Engine shape | library workspace, no framework, optional thin `winit` shell | Bevy plugins; a full framework with its own ECS and loop |
| World model | streaming unbounded from day one; full detail hundreds of meters at 10 cm, LOD to about 1 km; voxel size one scalar per world | bounded volumes only; kilometers at full detail; multi-resolution voxels |
| Platforms | Linux + Windows, 2019 mid-range desktop GPU baseline, 6 GB VRAM | + macOS; RTX/RDNA2-only |
| First milestone | a fly-through with digging, built on a headless world crate that is tested without a window | headless-only first; sound graybox first |
| API surface | Rust crates by git dependency, plus scripting: **WASM (wasmtime) for deterministic server logic, Lua (mlua) for client-side scripts** | Rust only; a C ABI; a single runtime for both |
| Logic as a component (added 2026-10-16, ADR-0010) | the game's whole server logic implements one `Logic` trait (inputs in, edits and queries through a `Host` handle, opaque bytes out); the same crate links natively for development and ships as a WebAssembly component; the logic produces a level layout and the engine's parallel generator turns it into voxels; Lua is deferred | an engine-owned loop with script hooks; native-only logic; Lua for server logic; generation inside the component |
| Entities | voxel models (MagicaVoxel-style volumes) on rigs of rigid parts, traced in the same pipeline as the world | skinned triangle meshes with a hybrid rasterize + RT path; deciding later |
| Repo | public GitHub, MIT OR Apache-2.0 | private; local only |
| Harness | Ritmolux-style: ADRs, phased plans, architect/dev skills, deny hooks, CI gates | CLAUDE.md + ADRs only |
| Surface look (added 2026-10-16; decided 2026-11-01) | honest cubes (`face`) are the default and meet M2's goal, and the field surface is an opt-in look (`--look field`; ADR-0041, Plan 0054's close). The requirement as first set, kept as history: smooth and natural rock, not visible cubes, with the Sibenik cathedral figure of Laine & Karras 2010 (ESVO, Figure 1) as the quality reference: not photorealism, but that level of believable surface and soft light; the mechanism (per-voxel normal smoothing, per-voxel contour planes in the GPU mirror, procedural per-material surface detail, or another) is chosen by measurement after M1, balancing image, frame cost and simplicity | photorealism; the smooth surface as the default, on frame cost (owner, Plan 0054 answers 13 and 14). As first set: a blocky aesthetic; committing to ESVO-style contours before anything is on screen |
| VR readiness (added 2026-10-16) | PC VR is a possible later target, so the renderer keeps three doors open from M2 on: a view is an asymmetric frustum (origin, basis, four tangent half-angles) and the frame renders N views, not one camera with a symmetric field of view; lighting is view-independent or shareable across views where the cost is equal; presentation is decoupled from the window surface. Actual VR support (OpenXR, 90 Hz, two eyes) is an unscheduled milestone with its own hardware floor, never a promise on the baseline GPU. See "Unscheduled: PC VR" below | building OpenXR support now; standalone headsets (mobile GPU, Android); treating VR as a mode of the baseline budget |
| Physics (added 2026-10-16) | **voxel dynamics, not a rigid-body engine.** Collapse is emergent and authoritative: `svet-world` answers which region lost its connection to the anchor after an edit, and deterministic ops decide what the world looks like afterwards. Fall damage is a controller event. Debris and falling pieces may be animated client-side by a rigid-body library as a cosmetic layer that never writes to the world. Revises M8 and the consumer's "scripted collapse only" wording | a general physics engine (`rapier`) in the authoritative world; scripted-only collapse; no collapse at all |
| Floor, profiles and scene scale (added 2026-10-17) | the GTX 1660-class floor of `nfr.md` §1 stays the 60 fps gate at 1080p with an upscaler; a named **portable profile** (Switch-2-class: about 1.7 to 3 TFLOPS, 68 to 102 GB/s, 9 GB shared) is a target of the quality settings (internal resolution, full-detail radius, translucent layers, lighting tier), at 720p and 30 to 60 fps; 120 fps is welcome where a fast machine reaches it through a performance profile, not a gate. Lighting has two tiers: direct light by one shadow ray per light at half resolution with blur and temporal reprojection on the floor, ReSTIR direct lighting where the budget allows, and, under ADR-0046 (accepted 2026-11-09, Plan 0059), emissive direct light from a per-chunk brick-level store baked at load and at edit, with no screen history; under ADR-0043, emissive indirect off and the probes a lamp-bounce channel (amended at Plans 0057's and 0059's closes; the rest of this row is the floor's plan). Glass, water and lava are translucent voxels in the brick map; mist, smoke and fire are a separate coarse volume. Open-sky and kilometre-scale scenes are in scope, so the far-field clipmap is a scheduled plan, not a maybe. The brick representation that serves all of this is ADR-0012 | Switch 1 (about 0.4 TFLOPS, 25.6 GB/s shared, no public graphics API for `wgpu`); raising the floor to the dev laptop; 120 fps as a budget; media as voxels in the brick map |
| Baseline card (added 2026-10-21) | the GTX 1660-class card is **not acquired**. The floor stays the gate on paper, read as a projection from two readings taken on the dev box: the NVIDIA card (the dev reference) and the AMD Cezanne iGPU on RADV, named in `nfr.md` §1 as the **slow adapter** (about the portable profile's class, below the floor's). From Plan 0028 on, every render plan's close takes its GPU readings on both adapters and labels each by its adapter | buying or borrowing a GTX 1660 before M2 closes; projections from the NVIDIA card alone |
| Outdoors (added 2026-10-22) | outdoors is a general engine capability (M10): a Hillaire atmosphere with a time of day the game sets, the sun's shadow as one per-pixel ray, a far-field clipmap to about 1 km, liquid surfaces on translucent voxels with flow later, volumetric clouds, and weather as cosmetic client state; terrain stays the game's source | a testbed-only showcase; near radius plus fog as the horizon; precomputed or analytic skies; shadow maps or heightmaps for the sun; a 2.5D cloud layer; weather written as voxels |
| The image first (added 2026-11-03) | a stable image with no flicker or aliasing artifacts comes first, performance considered. The owner: "the main thing that I care about for now is a stable image without any flicker or aliasing artiacts. we move towards this image by any means possible, considering performance of course" | any other work ahead of it: "all other things doesnt really matter as of now" |

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
hash moved; the owner kept the 2 ms figure. Plan 0014 (closed 2026-10-17, v0.7.0) is the second pass: a brick-array pool, a sphere span walk and an analytic capsule row cut `worst` to 3.19-3.26 ms (3.03-3.05 ms with a warm pool) and `tool` from 8.05 to 6.78-6.80 ms. The §4 miss carries forward: the row write bounds the cost inside ADR-0002's representation, and the owner chose a representation: ADR-0012 (accepted 2026-10-17), whose CPU-format step is Plan 0021. Plan 0021 closed 2026-10-20 (v0.18.0): the planar edit kernel reads the warm worst case at 1.34 to 1.52 ms on the dev box (not quiet; 3.03 ms before) and the fly-through `B` line at 1.52 ms. The owner held the 2 ms figure on the dev box (2026-10-20), so the line is met there; there is no baseline-GPU reading. A quiet-box re-read the same day (2796062) did not reproduce those figures: `worst_warm` 1.79 ms, and the `B` line 2.06 to 2.09 ms, with an unchanged `svet-world`. The owner raised the line to 2.2 ms on the dev box and planned one more optimization pass: [Plan 0031](plans/done/0031-edit-cost-third-pass.md) (closed 2026-10-21, v0.25.0) builds each brick's shape words straight from row spans solved once per op, and on a quiet dev box reads `worst_warm` 0.77 ms and the `B` line 0.99 to 1.00 ms (8dad5ca), so the owner returned the line to 2.0 ms on the dev box, met there; there is no baseline-GPU reading.

**Before M2's lighting (both closed 2026-10-21):** a streaming-CPU plan (incremental queue, residency sweep and
window shift; narrowing and staging on workers), drafted 2026-10-20 as
[Plan 0023](plans/done/0023-render-thread-cpu-attribute-hitch-and-streaming-line.md) together with
Plan 0013's attribute-hitch followup. The march-pass diagnosis this
paragraph also asked for landed in Plan 0015 (the steps view, its headless histogram and the
autopilot readings), which brought the streamed primary-ray line under 3.0 ms on the dev box.
Plan 0002's Followups list the rest.

**Goal:** a window showing a ray-marched, streaming voxel cave lit by a headlamp. You fly, carve
spheres, and the per-material voxel counts print to the console.

M1 is two plans. Plan 0001 is the bounded walking skeleton: brick map, edits, queries, renderer,
a finite source. Plan 0002 adds streaming, eviction and LOD mips on top. The storage ADR is
written with Plan 0001 and already carries chunk addressing for paging, so Plan 0002 changes no
layout.

- `svet-world` (Plan 0001): brick map and chunk store with the access model above; material
  table (id, name, albedo, roughness, emission, hardness, flags, opaque payload); `WorldSource`
  trait, `(seed, chunk coord) -> chunk`; the edit ops listed under "Storage shape" in integer
  math returning results; CPU raycast (DDA), point query and region material counts; dirty
  tracking for upload.
- `svet-world` (Plan 0002, ADR-0003): sphere residency around one or more focus points on a
  background thread pool, in two tiers: full detail inside `r_full`, and an LOD-only ring out
  to `r_lod` that keeps only mips (32³, 16³, 8³), not bricks. Edited chunks are persisted on
  eviction, in RAM with disk spill. Plan 0001's `Full` fraction reading (about 21 %, not 10 %)
  is why the ring replaces bricks instead of adding mips to every resident chunk.
- `svet-render`: incremental per-brick upload; primary-ray DDA in compute over the brick
  occupancy masks; one spot light with a shadow ray; blit to the swapchain. Plan 0002
  (ADR-0004): a wrapped window following the camera, budgeted batched uploads, an 8-bit
  per-chunk palette, a pool sharded over up to four bindings, and a mip walk for LOD-only
  chunks.
- `svet-app`: window, keyboard and mouse, frame loop, surface resize.
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

### M2: Lighting that holds in the dark (in progress: Plans 0005, 0013, 0015, 0017, 0021, 0023, 0024, 0027, 0028, 0029, 0030, 0038, 0039, 0040, 0041, 0047, 0050, 0051, 0053, 0054, 0055, 0056, 0057, 0059, 0060, 0061, 0067, 0070, 0074 and 0058 closed; 0049 abandoned)

Plan 0067, the frame's CPU, closed 2026-11-12 (v0.49.0; Outcomes on ADR-0052, ADR-0053, ADR-0048 and
ADR-0046): the live queue's step is split into stages and read per regime (A starved, B keeping up). At
rest it drains instead of withdrawing the same job every step, 0.075 ms p50 against 3.26 on the RTX. A
brick with no baked value reads its chunk's summary, then its neighbours', then a two-level parent table
(`chain`, the owner's pick), and a first block fades in from it. The owner kept the bake at `frame:1.5`,
the near shell's full jobs at 2 and the reach rule; the other modes stay render settings for Plan 0069's
profiles. Under `chain` a regime-B step pays about 1.7 ms of first-block fades (backlog); the sync's
p99 in flight is Plan 0070's. Plan 0070, closed 2026-11-13 (v0.52.0), cut the sync in place on the render thread (a
dense emitter rescan set, light walks on change), to p50 15 to 19 us and `update + poll + sync` p99
22.8 to 28.2 ms in the tester's benches, still over 16.6 ms, with `light_note` the largest stage left
and the worker packing of ADR-0056 deferred (backlog). [Plan 0074](plans/done/0074-the-syncs-light-stages-placement-spikes-attributed-and-exposed-emitters-by-masks.md)
closed 2026-11-14 (v0.53.0): it found exposed emitters by masks (`light_note` p99 2.5 ms against 8.3,
`update + poll + sync` p99 18.0 to 23.7 ms in the cave), left `alloc` the largest stage of the benches, and its
window put the glide's stagger in the light placement's eviction scan, unindexed; the backlog's head takes the
eviction scan first, then `alloc`.

[Plan 0058](plans/done/0058-dynamic-lights-a-list-of-analytic-lights-dropped-carried-and-bounced.md),
dynamic lights, closed 2026-11-15 (v0.54.0; ADR-0044 accepted with an Outcome): the consumer hands the
frame up to sixteen point or spot lights (the headlamp is slot 0 while it is on, with no reach), each
shaded exactly with one shadow ray inside its reach. The direct part is rgb in a target of its own, and
a light's `BOUNCE` flag feeds the lamp channel. The fly-through drops, carries, fires, recolours and
flags lights (`1` to `6`). The owner's windows saw no trail or shimmer under FSR 2. The light pass
reads about 0.17 ms per light in reach on the RTX and 1.5 ms on the RADV RENOIR (24.8 ms at sixteen at
the spawn), so Plan 0069's Portable profile caps the count. This meets the consumer's "flares"; a
glowing projectile's body is M6's.

Plan 0061, antialiasing, closed 2026-11-11 (v0.48.0; ADR-0049 accepted with an Outcome, Outcomes on
ADR-0042 and ADR-0034): the owner's "a lot of aliasing" at the hall was ADR-0042's far bands back, mixed
by the half-resolution light's filters. Where the far normal leans a face's normal, the baked store is now
read as the mix of the cell's direction values, and the filters weigh taps by the face normal; both are
the fly-through's defaults, at far normal `[2, 6]` and FSR 2 at 1.5. The owner's window: "no aliazing",
"maybe even a bit too soft". FSR 2 at scale 1 is a quality setting for Plan 0069's profiles; the emitter
cap is gone. The Radeon's a-trous with the face weights is over the denoise line (Plans 0067, 0069).

Plan 0056, the aliasing spike, closed 2026-11-04 (v0.42.0, no ADR): it read the final image
under an eye lamp that cannot move. FSR 2's frame-to-frame jumps at 1.5 sit on silhouettes and
emitters, are locked to its jitter and survive the frozen light, so they are the upscaler's,
not the light's; native holds bit-identical. The floor's rings are its 1-voxel terraces, and the
small yellow dots on fast turns are light on creases, mostly new each frame (handed to Plan
0057). Candidates stay opt-in knobs. The owner's window kept FSR 2 at 1.5, picked far normal
`[1, 2]` by eye (about 7 times `[2, 6]`'s still jumps headless, recorded beside it), called the
emitter cap a fault, and routed the remaining complaint, slow light settle, to Plan 0057 as a
camera-radius preload. Owed: a plan whose first phase makes `[1, 2]` the default with
ADR-0042's Outcome and whose next item is the edge dots left at `[1, 2]`, drafted with the antialiasing plan at Plan 0059's Phase 3 verdict; the reference runs at `[1, 2]` by flag.

Plans 0053 and 0054, the surface-look decision, closed 2026-11-03 (v0.39.0, ADR-0041 accepted
with an Outcome, ADR-0040 rejected): the majority fit and the normal field failed the owner's
eye, the field surface was built to its 5 ms bar and then judged not worth about 3x the cubes'
GPU frame in the window, and honest cubes (`face`) are the default, with the planes, snap and
the attribute pass removed. Hit plus light at the level's spawn reads 1.397 ms on the RTX 3080
Laptop and 10.796 ms on the Radeon RENOIR, both under the planes' figures. The field stays an
opt-in look, `--look field`, with `MaterialFlags::CRISP`; its crisp seam is a known open defect.

Plan 0050, a watertight contour surface, closed 2026-10-31 (v0.38.0, ADR-0038 and ADR-0039
accepted with Outcomes): a cracks view classifies how each primary ray reached its hit, the
plane code holds a zero normal component exactly, and the walk closes a crack at the passed
voxel's exit (snap) at +1.3 % (RTX) and +0.4 % (Radeon) on the hit pass. The owner's check
found the wedges and triangles gone; the voxel-sized flat-faced boxes that remain go to Plan
0053, with Plan 0054's field surface as its alternative.
Amended 2026-11-01 (owner, Plan 0054 owner answer 13): honest cubes are the default surface
look, and the contour planes are removed; Plan 0054's field surface stays as an opt-in look,
off by default, at about 3x the cubes' GPU frame in the owner's window. Whether liquids take
it is Plan 0034's question.
Amended 2026-11-01 (owner, Plan 0054 owner answer 14): cubes meet M2's surface-look goal;
the field is a quality option revisited per material class, liquids first.

Plan 0051, the steady-light spike, closed 2026-11-04 (v0.41.0, no ADR at close): three experiments
built behind `Lighting` settings, all off by default, and read headless and on both adapters. A
(probes and emissive pixels as running means that freeze, inside probes skipped, converged pixels
skip the draw), C (the headlamp out of the probe rays) and B (the emissive direct irradiance per
cube face in a hashed world-space table, with an edit-box reset). At the showcase's hall, native,
B with indirect off drifts 0.004 against today's 2.19 (4096 against 8192 frames); under FSR 2 it
reads 0.525, and no arm holds still within the owner's five seconds. In the owner's window the
small yellow dots on fast motion stayed in every arm. The owner's verdict: B with indirect off,
the nearest read, the probes shrunk to a lamp-bounce channel whose logic is redone, exposure
unchanged. The ADR and the plan the verdict names follow; Plan 0056 hunts the dots first.
ADR-0043 (proposed, 2026-11-04) records the verdict: the per-face table is the only emissive
store, faces are sampled and guarded in world space, and the probe volume becomes a lamp-bounce
channel that keeps its scaffolding for a later per-source indirect light. Plan 0057
built it, closed 2026-11-05 (v0.43.0, ADR-0043 accepted with an Outcome), the first phase a reading of the verdict's open questions (the fast turn, the
dug hole); Plan 0048 is moot under it and was dropped (owner, 2026-11-04). After Plan 0057's window
of 2026-11-05 ("it feels like this is wrong way") the owner asked for light that is already there:
ADR-0046 and Plan 0059 bake static emissive light per chunk. After Plan 0057's second window the owner
set the order (2026-11-05): a reference with no budget first, emissive light only, judged in the window,
then cuts one at a time by eye (antialiasing moved after them, 2026-11-06). Plan 0059 closed 2026-11-09
(v0.44.0; ADR-0046 and ADR-0047 accepted with Outcomes, ADR-0043's table superseded): the owner picked the
bake, a brick-level store at 8 voxels and 4096 samples a direction filled by a live queue and re-baked by
digs, and the table is deleted. The order from here (owner, 2026-11-10, at the review of every active
plan): Plan 0060 (the queue's view order and withdrawal, ADR-0048), Plan 0061 (antialiasing, ADR-0049),
Plan 0058 (dynamic lights, redrafted over the store; ADR-0045 withdrawn), then the code-health Plans 0044
and 0045, then M10. `docs/plans/README.md` carries the sequence. Plan 0060 closed 2026-11-11 (v0.45.0, ADR-0048
accepted with an Outcome): the queue sorts by a view score with the class kept as the leading key and withdraws
the jobs a flight has passed; at the window the order made no visible difference, because the slice's
throughput, not the order, limits the far light (Plan 0067 takes it). Amended 2026-11-11 (owner): Plan
0045 moves ahead of Plan 0058 and the rest of Plan 0044, once Plans 0061, 0067 and 0068 have merged. Plan
0045 closed 2026-11-13 (v0.50.0): the mega files are directories of owned parts, `svet-render`'s
surface is a curated root with `debug` and `internals`, and two gates cap file and function size;
no behaviour changed. Plan 0044 closed 2026-11-16 (v0.55.0): the audio callback frees nothing, the frame keeps its
bind groups by what they bind, the march reads a material only at its hit, the render tests share one
device rule and a naga layout test pins the uniform structs; images, hashes and samples unchanged.

Plan 0055, the clarity pass, closed 2026-11-03 (v0.40.0, ADR-0042 accepted; Outcomes on ADR-0011,
ADR-0024, ADR-0034 and ADR-0041): procedural detail is removed, so ADR-0011 has nothing left in the
tree. FSR 2 at scale 1.5 is the fly-through's default, and our upscaler is opt-in. A far shading
normal blends the face normal to the 27-count gradient by the voxel's size on screen, on by default
at `[2, 6]`, the owner's provisional pick, reopened in Plan 0056 (its window picked `[1, 2]`). FSR 2 stands on the owner's
word ("fsr2 stays", "fsr2 stays counts"); the scripted slow pan did not run. Upscale reads 2.93 to
2.96 ms on the RTX 3080 Laptop against section 2's 1.5 ms line, and the showcase frame on the RADV
RENOIR reads 55 ms. Plan 0051 reads its light bars on this look.

Plan 0047, still emissive light, closed 2026-10-29 (v0.36.0, ADR-0036 accepted with an
Outcome): the emissive part of the temporal pass accumulates in `f32` over a 256-frame
history (`DEFAULT_EMISSIVE_HISTORY`, was 32), an emissive-only gradient guard shortens it
where a reveal changes the light, and the probe slices are spread by a hash of the slot. A
headless patch bar (`flythrough::emissive_stillness`) reads 1.50 patches a step at the patch
pose with the lamp off, from 74 to 78, and 0.05 at `VEIN_POSE`. The owner's check found still
walls much better. Indirect light that settles over seconds after a move or a lamp toggle, and
light dots on a fast turn or a dig, went to Plan 0049, dropped 2026-11-03 for Plan 0051. The firefly clamp's bias on the probe
rays goes to Plan 0048.

Plan 0038, the showcase cavern, closed 2026-10-28 (v0.35.0, testbed only, no ADR): the
fly-through gains `--source cave|level|showcase` and `--shot`, and the showcase is a bounded,
seeded, integer-only 72 x 58 x 24 m hall with pillars, cones, ledges, a chasm and a bridge,
lit with the lamp off by four emissive ores. By the owner's Phase 6 call no ore lies on the
roof, about half the floor is dark and the crystal clusters are the main light. `hero`
defaults to the showcase's `hall` shot at 1920x1080 with indirect light at spacing 32 and our
upscaler at 1.5, and the README's hero still was re-shot from it (the README's images are now
the rows of `docs/images/shots.json`). It shows M2's lighting and
changes no engine crate.

Plan 0040, eye adaptation, closed 2026-10-28 (v0.34.0, ADR-0033 accepted with an Outcome):
the exposure is automatic by default, metered in the composite and stepped on the GPU by an
asymmetric two-stage model (0.15 s toward bright; 2 s, then 10 s below a cone floor, toward
dark; at most 16), and every HDR target stores pre-exposed values. `Lighting::exposure` is
`Exposure` (`Manual` or `Auto`); the fly-through has `--exposure`, `--adaptation`, `--adapt`,
`--metering` and `Q`. This closes the "exposure" part of M2's exposure, tone mapping and
upscaler bullet. The light patches seen while adapting belong to the emissive estimator
(Plan 0047).

Plan 0041, the FSR 2 port, closed 2026-10-27 (v0.33.0, ADR-0034 accepted with an Outcome):
FSR 2.2.1's six upscaling passes in WGSL, FP32 with no wave operations, fed by an input pass
from the hit buffer and finished by a tone pass, beside our upscaler (`--upscaler
off|own|fsr2`, `F10`). It holds its still, ghost, history, lock and lamp bars, but it misses
the slow-pan bars against ours (its motion weight and history fetch, no port defect found)
and the 1.2 ms bar on the dev box (1.89 to 1.91 ms at scale 1.5). The owner's windowed
comparison found "not much difference" and made our own upscaler at scale 1.5, jittered, the
fly-through's default; FSR 2 stays opt-in. §2's "with the upscaler allowed" now rests on the
default path.

Plan 0039, the contour look at arm's length, closed 2026-10-27 (v0.32.0, an Outcome on
ADR-0011): a headless sharpness harness found the half lighting tier the main source of the
softness, and the fly-through's default became `contour-sharp`, a weighted plane fit with the
detail's lamp term applied after lighting, by the owner's call, re-confirmed with probe GI.
`contour-grain` stays opt-in. The owner saw sharp voxel corners on some smooth walls in the
default look; that and a per-material smooth flag are the next look work, in the plan's
followups with grain that fades at grazing incidence.

Plan 0029, the indirect-light plan, closed 2026-10-22 (v0.31.0, ADR-0023 accepted with an
Outcome): a 32^3 probe volume at 1.6 m scrolls with the camera, one probe in 8 traces 64 rays a
frame, and a visibility-weighted gather adds the bounce to the composite. A light change opens
a 16-frame window that refreshes one probe in 4 at hysteresis 0 and resets the direct light's
lamp history; probe rays read their own emission scale, so a pulsing emitter feeds them its
mean. On the dev box update plus gather reads 1.57 ms (2.70 ms inside a window) under the
1.7 ms bar, and the lighting line at half 4.4 ms under 6.0. The owner kept the bounce and the
spacing; the lamp-off fade reads "almost instant". The owner's followup, a per-light and
per-emissive-material switch for feeding the probes, is in M2's list below.

Plan 0028 closed 2026-10-22 (v0.29.0, ADR-0012's last step, with Outcomes on ADR-0012,
ADR-0013 and ADR-0011): a `Full` brick's mirror slot is its packed CPU bytes in 64-byte units
allocated in bins over ADR-0013's pages, the chunk palette and the masks buffer are gone, and
the contour attribute is stored for a brick's chunk-local surface voxels only. The level's
mirror fell from 232 to 63 MB plain and from 680 to 158 MB contour with every golden byte for
byte; the default at 16/24 reaches full residency at `r_full` 16 with 0.87 GB live and pressure
0.65, so the pressure rule no longer fires. The attribute pass, rebuilt at four bricks a
workgroup, reads p99 0.71 to 0.75 ms with a 25 ms backlog age; plain `sync` p50 fell to 0.17 ms
under the streaming line. The default look's `sync` reads 3.1 ms p50 (the surface masks on the
render thread), its followup (moot since Plan 0054: the default has no attribute pass); light
extraction's p99 miss stays open without a plan. The
fly-through gains `--adapter`, and the first slow-adapter (RADV iGPU) readings are in `nfr.md`.
The owner's windowed flights at full detail "feel fine".

Plan 0030, the upscaler plan, closed 2026-10-22 (v0.28.0, ADR-0024 accepted with an Outcome):
an internal resolution, a per-frame Halton ray jitter, lighting passes that ignore it, an
upscale pass at the output size (hit-position reprojection, a 3 x 3 Gaussian, a Catmull-Rom
history fetch, a light-change clip) and RCAS. It meets its headless bars and the upscale line
on the dev box (0.76 ms at scale 1.5), and halves the frame's GPU time on both adapters at 720p
for 1080p, but the owner's eye kept Plan 0024's frame (scale 1, jitter off) as the default: the
upscaler ships as an opt-in performance setting, and an FSR 2 port (ADR-0024 Alternative A) is
Plan 0041 with ADR-0034 (proposed). §2's "with the upscaler allowed" therefore rests on an
opt-in path until that plan reads.

Plan 0102 closed 2026-11-25 (v0.74.0, ADR-0086 accepted with an Outcome, superseding ADR-0024):
that upscaler is removed. FSR 2 is the engine's only upscaler, and a frame without it renders
at its output size.

Plan 0024, the lighting plan, closed 2026-10-22 (v0.27.0, ADR-0018 and ADR-0025 accepted with
Outcomes): a hit pass, a light pass with one emissive sample resampled from a near-emitter
table and one shadow ray, a temporal pass whose history dies in edit boxes (the emissive part
on a 32-frame history, the lamp's short), four a-trous iterations and a composite. The owner's
defaults are half-resolution lighting, the coarse shadow walk, reservoir reuse and edit-box
history; the other modes stay. On the dev box the defaults read under the lighting and denoise
lines; full resolution misses both, and light extraction stays a recorded miss for Plan 0028.
Plan 0023 closed 2026-10-21 (v0.23.0, ADR-0017 accepted with an Outcome): the attribute
work list's shell looks up neighbour chunks instead of visiting a 488-brick volume, the pass is
capped with a carry-over queue whose waiting bricks read "no plane", and the stream queue,
the residency sweep and the window shift are incremental. The contour look's render-thread
hitch fell from p99 138 to 180 ms to 5.5 to 6.0 ms against a 4 ms line (a recorded miss), and
the default now flies more frames than plain. The streaming line stays a recorded miss at p50
about 5 ms in plain flight; its largest share is the `write_buffer` copies, so narrowing on the
workers was skipped and the copies need their own ADR (ADR-0017's Outcome). The owner accepted the close with its four
misses as followups (2026-10-21) and ordered the copies' own ADR and plan after Plan 0024, apart
from ADR-0012's mirror-slot plan. In the owner's
windowed flight the frame rate fell to about 40 fps on sustained fast passes with the mirror
at its budget; the mirror-slot plan owns that footprint.
Plan 0027 closed 2026-10-21 (v0.26.0, ADR-0022 accepted with an Outcome): the mirror's
uploads go through a staging belt and copy commands in the frame's encoder, `write_buffer`
is gone from `sync`, and on a quiet dev box plain `sync` p50 reads 1.40 to 1.66 ms (3.28 to
3.69 ms before) with every mirror byte unchanged. The 1.0 ms line stays a recorded miss, now
led by narrowing; the belt's own 25.4 % share is a second miss the owner accepted. The
fly-through prints a `frames:` summary at exit; the owner's windowed flight read about 30 fps
with the overlay's timer serializing CPU and GPU, which Plan 0028's windowed flight reads
again with the overlay hidden.

Plan 0015 closed 2026-10-19 (v0.14.0): the marcher walks a hierarchy of occupancy bits
(chunk bits with an empty bit in the table, 512-bit brick masks on aligned lines, a walk that
holds its state and reads one line per level) and composites the translucent material class
in a bounded second walk, on today's pool, as the first step of ADR-0012. The streamed
spawn's march fell from 6.8 to 1.95 ms on the dev box against the 3.0 ms line, the level's
from 1.8 to 1.45 ms, with no baseline reading; a cell exit follows plane order, so the image
does not depend on the skips (ADR-0012's Outcome). Every lighting pass casts rays through
that walk, so its cost per ray is settled first. The traversal efficiency review of 2026-10-17 (the ESVO,
VoxelRT and wavefront sources in `references/voxel-optimizations.md`) found nothing in the
literature that beats a bit hierarchy for an editable near field, and left two design inputs
for the lighting plan, listed with its bullets below.

Plan 0017 closed 2026-10-20 (v0.17.0): the mirror reserves its VRAM budget once (ADR-0013):
no grow copies, empty pages shared by both kinds, records for the residency's ball, and a
pressure rule that lowers `r_full` when the pool or records near full. A sync writes the
nearest chunks that fit whole and defers the rest. `nfr.md` §3 reads live bytes against the
reserved budget; contour at 16/24 settles at r_full 12 to 13 under 3 GB. Page fragmentation with attribute pages is its open followup.

Plan 0021 closed 2026-10-20 (v0.18.0): a `Full` brick is a packed occupancy mask, an ascending
palette and bit planes in five fixed classes (ADR-0012's CPU-format step), edits are word
operations with popcount tallies, and the hot readers read masks natively. On the dev box,
resident chunks at the streamed defaults fell from 3.24 to 0.44 GB, and peak RSS from 3.87 to
about 1.0 GB. The mirror bytes did not move, and neither did any pinned hash. The
size-classed mirror slot plan is next in ADR-0012's sequence. The residency p95 read 611 ms
against the 500 ms proposal under load; the quiet-box re-read (2796062, `nfr.md` §5) reads
381 ms in face mode, under the proposal, and 270 to 904 ms with the default contour look,
which Plan 0023's attribute work owns.

Plan 0013 closed 2026-10-20 (v0.19.0): contour with detail (rock `[4.0, 0.1]`) is the
fly-through's default and `--look plain` keeps face mode. A plane hit's shadow ray leaves along
the plane normal (specks 52 against face mode's 169). The footprint is not reduced: the pressure
rule holds the default at r_full 12 to 13 under 3 GB, and surface-only attribute storage is
ADR-0012's mirror-slot plan (ADR-0011 Outcome). The default's march at the settled streamed
spawn is 2.28 ms against face's 2.13. Its attribute pass under motion misses the plan's
proposals: GPU p99 4.3 ms, and render-thread p99 139 to 167 ms, a visible hitch. These are
diagnostic readings. The hitch is open and is owed before the lighting plan, with the
streaming-CPU plan. A quiet-box re-read (2796062, 2026-10-20) confirmed it: render-thread p99
138 to 180 ms, and the default flies 35 to 47 % fewer frames than plain. The owner confirmed it
owed before the lighting plan the same day.

Plan 0005 closed 2026-10-16 (v0.4.0): the surface-look spike measured four normal modes and
procedural detail, and the owner chose per-voxel contour planes with weaker rock detail
(ADR-0011); Plan 0013 made it the default.

The lighting plan is [Plan 0024](plans/done/0024-direct-light-hit-buffer-emissive-samples-and-svgf-denoiser.md)
(drafted 2026-10-20, with ADR-0018): direct light and the denoiser only. A hit pass writes a
per-pixel record, a light pass takes one sample per pixel (the headlamp, or an emissive voxel
resampled from samples the mirror extracts at upload) with one shadow ray, a temporal pass
reprojects by hit position and kills history inside the frame's edit boxes, and four a-trous
iterations filter the rest; the frame renders a view list. Four mechanisms stay switchable
until the owner has seen them: the coarse or full shadow walk, reservoir reuse or a flat
list, edit-box or gradient history, half or full lighting resolution. It starts after Plan
0023 closes and before ADR-0012's mirror-slot plan; the streaming line's upload-copy ADR and
plan follow it (owner, 2026-10-21). The plans around it were drafted on 2026-10-21:
[Plan 0027](plans/done/0027-mirror-uploads-through-a-staging-belt.md) (ADR-0022, the streaming
line's copies through a staging belt; closed 2026-10-21),
[Plan 0028](plans/done/0028-size-classed-mirror-slots-and-surface-only-attributes.md) (ADR-0012's
mirror-slot step, after Plans 0024 and 0027),
[Plan 0029](plans/done/0029-indirect-light-probe-volume.md) (ADR-0023: indirect light from a probe
volume that scrolls with the camera, the owner's choice over a per-pixel bounce) and
[Plan 0030](plans/done/0030-temporal-upscaler.md) (ADR-0024: a temporal upscaler of our own over
an FSR 2 port). The edit-cost pass M1's paragraph names, Plan 0031, closed 2026-10-21.

**Goal:** the consumer's P0 rendering bar. Few lights, emissive voxels, destruction, no ghosting.
The quality reference is the Sibenik cathedral in Laine & Karras 2010, Figure 1. That image is
5 mm voxels with a unique colour and precomputed ambient occlusion per voxel, 2.7 GB on the GPU:
twenty times our resolution and shading data we do not store. At 10 cm and one material id per
voxel the same impression has to come from three cheaper places, each measured in its own
spike: the silhouette (contours or normal smoothing), the surface texture (procedural
per-material detail evaluated in the shader from world position, the way Teardown's materials
work), and the light (real-time GI standing in for the baked occlusion).

- The march split from the shading: a lean hit pass that writes a compact hit buffer
  (position, normal, material, plane) and separate passes for lighting, so the march kernel
  carries no shading registers and secondary rays run as coherent dispatches instead of
  branches inside the primary kernel (Laine, Karras, Aila 2013). The denoiser needs the hit
  buffer anyway, so this is ordering, not extra work; the plan reads the march kernel's
  register count and occupancy before and after on the dev box.
- A coarse walk for secondary rays: shadow and ambient-occlusion rays stop at the 2³ level
  of Plan 0015's masks, treating any block with a set bit as solid (Teardown's 1-bit shadow
  volume), so they never read a voxel or the pool. A named mode of the walk, measured
  against the full-resolution walk on the same rays.
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
- A per-source switch on feeding the probes (the owner, Plan 0029 Phase 11, 2026-10-22):
  "leave indirect light only for certain sources, like fire on stick or fire in dungeon - and
  for simple lights it should be off". A flag per light sample (the headlamp, later lights) and
  per emissive material on whether it feeds the probe rays, on Plan 0029's per-material probe
  emission row; not yet planned.
- Static emissive light that holds still (the owner, after Plan 0040's second session,
  2026-10-27): with the lamp off near a vein, wall patches step on one by one and flicker.
  Plan 0047 closed it for a still camera (v0.36.0); the motion faults go to Plan 0051.
- The probe rays' direct light without the firefly clamp's bias (Plan 0047's step C,
  2026-10-28): the clamp cuts the probe rays' direct light about 8.5 % low in the furnace.
  Plan 0048 (approved 2026-10-28, ADR-0037 proposed) measures probe-only schemes against pi,
  keeps the winner and today's clamp, and returns the furnace test to pi. The order is Plan
  0051, Plan 0056 (the aliasing spike) from the verdict, the verdict plan, Plans 0044 and 0045, Plan 0048 if it stands, then M10 (amended
  2026-11-03, owner): 0048 holds whatever settle, dot and shimmer tests the verdict plan asserts.
  The verdict plan is Plan 0057 (ADR-0043, 2026-11-04); under it 0048 is moot and was dropped (owner,
  2026-11-04).
- Indirect light that settles fast, light that holds still in flight, and young pixels
  without dots (the owner at Plan 0047's windowed check, 2026-10-28 and 2026-10-29): seconds
  of settling after a fly-in, a lamp toggle or a dig, bright dots after a fast turn or a dig,
  and shimmer during ordinary flight. Plan 0049 measured them on a spike and found the light
  never settles at a still camera; the owner dropped it on 2026-11-03, its readings kept in the
  plan under `done/`. The rule of 2026-10-29 stands: every other plan stays parked until the
  owner accepts the light. **Plan 0051** closed 2026-11-04 (v0.41.0): the owner's window picked
  the world-space emissive table keyed by cube face with no indirect gather, and the probes
  shrink to a lamp-bounce channel. **ADR-0043** and **Plan 0057** (closed 2026-11-05, v0.43.0)
  built it: the table as the only emissive store, faces sampled and guarded in world space, the
  lamp channel; Plan 0056 reads the dots that no arm removed beside it and hands over their class.
- **Headless proof:** irradiance in a closed test box matches an analytic value within a stated
  tolerance; a carve invalidates exactly the touched history region.
- **Visible proof:** golden images of a dark cave with one emissive vein, before and after a
  carve, with a stated SSIM floor against the reference.
- **Budgets:** the full frame at 1080p output on the dev box's two adapters (`nfr.md` §1); the
  floor is a projection from them.
- **ADRs expected:** denoiser family; upscaler choice. Surface representation: ADR-0011; its
  streamed footprint: ADR-0012 (final, surface-only); until then the pressure rule's radius.

### M3: Collision, character controller, coarse grids (Plan 0004; done 2026-10-17)

Plan 0004 closed 2026-10-17 (v0.5.0): `svet-sim` sweeps a vertical cylinder (ADR-0005, not a
capsule) and drives a controller with step, slide, fall, jump and ledge climb, pinned by a
course hash; `svet-world` keeps the coarse grid and walkable map from edit results; the
fly-through walks on the bounded level and the owner drove it by hand. Open: the sweep cost
(`nfr.md` section 8, awaiting the owner's call) and wiring the grid to the streamer; Plan
0004's Followups list them. Plan 0016 (closed 2026-10-18, v0.13.0) added depenetration to the
controller (ADR-0005's Outcome addendum). Plan 0009 (collapse) is unblocked.


- `svet-sim`: capsule sweeps against bricks (the raycast stays in `svet-world`, which owns all
  primitive queries); a character controller with steps, slopes and climbing out of holes;
  shape queries for tool reach.
- `svet-world`: a one-cell-per-brick (0.8 m) occupancy and dominant-material grid, updated
  incrementally from touched chunks; region change notifications; walkable-space extraction.
- **Headless proof:** a capsule walking a known course ends where the fixture says; a carve
  under its feet changes the coarse grid cell and emits one notification.
- **Visible proof:** walking instead of flying in the fly-through.

### M4: Spatial audio (Plans 0006, v0.12.0, and 0020, v0.24.0, done)

- `svet-audio`: output through `cpal`, a mixer, HRTF spatialization, occlusion and transmission
  by ray probes through the material table's acoustic triple, early reflections from ray probes,
  hooks for material-driven synthesis. Separate from any gameplay noise model, which a game
  builds on M3's coarse grid.
- **Headless proof:** a source behind 5 m of a material with known transmission arrives at the
  listener attenuated by the table's value; an open corridor yields a reflection at the expected
  delay.
- **Visible proof:** a knock in the fly-through sounds different behind thin and thick walls.
- **ADRs expected:** HRTF source and audio thread model (ADR-0006); the late tail's synthesis
  and its parameter source (ADR-0015).
- **Second slice (Plan 0020):** a late tail whose decay time and level follow the room the
  listener sees (a ray fan, Eyring), early echoes heard from their wall's direction, taps that
  glide instead of swapping (the moving-listener clicks), and a streamed voice so a microphone
  can be heard echoing in the cave. Headless proof: three closed cubes of 4, 12 and 36 m give
  Schroeder-measured decay times within 20 percent of 0.75, 2.26 and 6.77 s and in that order.
  Heard proof: the owner's `--room` checks and the cave walk. Closed 2026-10-21 (v0.24.0): the
  physical levels lost to the ear, so echo and tail levels follow listening-fitted curves of the
  room's mean free path and a room change hands the tail to a second network (ADR-0021).

### M5: Simulation split and the logic component (done: Plans 0007, v0.10.0, and 0011, v0.11.0)

Plan 0007 closed 2026-10-17 (v0.10.0): `svet-net` (byte channel, server tick, shared and
owned clients with desync detection), `svet-world`'s op and input logs, log file and wire, and
the fly-through's digs, fills and bench through a server, saved and replayed under `--level`
(ADR-0007, accepted). Plan 0018 (closed 2026-10-20, v0.15.0) completed the split for
every op: stamp, drop and remove are op kinds 2 to 4 through `submit`, the log, `Host::apply`
and the owned client's check. A stamp names its prefab by content id and a save carries the
prefabs it names (ADR-0014, `LOG_FORMAT` 2). The server keeps a coarse grid and answers
`Host::unsupported`, and the fly-through's `P` and collapse run in `DigLogic`, saved and
replayed. Plan 0016 (closed 2026-10-18, v0.13.0) replaced fail-stop with a capped
restart from the seed (ADR-0010's Outcome addendum).
Plan 0019 (closed 2026-10-20, v0.16.0) bounded and metered host-call work: `svet-world`
refuses out-of-range shapes, rays, box queries and region lists per world (`WorldLimits`,
`EditError::OutOfRange`), and the host charges each import's work to the tick's fuel at
`WorkRates`, list arguments by their bytes before any host work (ADR-0010's second Outcome
addendum).

Plan 0011 closed 2026-10-17 (v0.11.0): `svet-script-wasm` hosts a logic component under
deterministic WASI (clocks fixed to the tick, always-ready timers, seeded entropy), fuel per
call and a total memory cap, and stops a logic after its first failed call (ADR-0010,
accepted). The fly-through's dig logic (`flythrough-logic`) runs natively or, with
`--features wasm` and `--logic wasm`, as a component, to the same hash. Followups from the
close: bounded or metered host-call work, with `svet-world` range checks on shapes (done in
Plan 0019); a component
hash and the `WasmConfig` in the log header; batched query imports; a per-call measurement of
a cheap query.


- An authoritative world plus op and input logs on the server side; clients send inputs, a
  `Logic` behind a trait decides the edits through a `Host` handle; a client mirror fed by
  ops over an in-memory channel, sharing the world in solo; replay from seed plus ops, or
  re-simulation from seed plus inputs (ADR-0007, ADR-0010).
- `svet-script-wasm`: a `wasmtime` host implementing `Logic` for a component built from the
  same crate as the native logic, with deterministic configuration (canonical NaNs, clocks
  and entropy from the tick, fuel per tick, a memory cap).
- Deferred: a Lua host for client-side scripts, until a consumer asks. Follow-ups named by
  the plans: the level-layout format the logic produces and the generator consumes; entity
  snapshots across the boundary (with M6); the server owning streaming focus points.
- **Headless proof:** a replayed log reproduces the world hash; the native and component
  builds of one logic produce byte-equal op logs on both CI arms.
- **ADRs:** ADR-0007 (log, channel, client modes) and ADR-0010 (logic as a component), both
  accepted at their plans' closes.

### M6: Entities as voxel models (Plan 0008 closed the `.vox` import; Plan 0062 closed the render half's entities, v0.56.0: posed voxel models traced with the world, lit and shadowed by the list lights, a headless ray query; Plan 0063 closed rigs and clips, v0.57.0: keyframe clips sampled by tick in fixed point, a `.rig` text file beside the `.vox`, the fly-through as a model viewer that reloads on save, voxels drawn as text; Plan 0078 closed the pool's free, v0.59.0: models freed by handle into a coalescing free list, stable instance slots with a generation, a removed entity's shadow gone on the next frame; Plan 0077 closed the entity picker, v0.64.0: the entities library, a chooser with still icons, a belt of nine slots, a placement mode with a marked ghost, an outline and a tint per instance in the composite and image tiles in the overlay, ADR-0074 and ADR-0075; Plan 0079 closed entities aware of the floor and the walls, v0.65.0: a sphere surface mover beside the cylinder controller in `svet-sim`, the solid-count gradient as the smoothed normal, `Quat::from_frame`, a `body` and a clip `stride` in the `.rig`, the fly-through's movers at a 60 Hz tick with a seeded wander, ADR-0062; Plan 0083 added a coarse grain per part, 1 to 16 world voxels per model voxel in the same integer arithmetic, v0.66.0, ADR-0078)

- `svet-assets`: `.vox` import with material mapping. Done in Plan 0008 (v0.6.0).
- `svet-render`: instanced voxel volumes with transforms traced in the same pipeline (a small
  top-level structure of AABBs walked in compute), rigid-part rigs, consistent lighting and
  shadows, emissive entities. Plan 0024's temporal pass reprojects by hit position, so a
  moving entity ghosts until this milestone's entities write motion vectors into it.
- **Visible proof:** a golden image of an animated model casting a shadow on the world.
- Squash and stretch: a scale per axis per part in the pose and the clips, so a slime reads
  soft without a soft body (Plan 0082, approved 2026-11-16; ADR-0064). See-through
  jelly and true soft bodies stay out (`docs/backlog.md`).
- Modes of one entity: variants declared in the `.rig` (a part hidden, a volume swapped, a
  material remapped) picked per instance and honoured by the query; a one-shot clip that
  holds; a posed entity baked onto the world grid and stamped through the op log, so a corpse
  is part of the map (Plan 0085, approved 2026-11-16; ADR-0068, ADR-0069).
  Per-entity voxel damage stays out (`docs/backlog.md`).
- **ADRs:** ADR-0050 (entity representation; accepted at Plan 0062's close, with an Outcome), ADR-0051 (rigs and clips; accepted at Plan 0063's close, with an Outcome), ADR-0062 (surface locomotion as a kinematic sphere in `svet-sim`; accepted at Plan 0079's close, with an Outcome); ADR-0064 (a part's scale and its children) with Plan 0082; ADR-0068 (variants as recipes) and ADR-0069 (the bake and the stamp) with Plan 0085.

### M7: Tooling (Plan 0008 closed prefab stamping (runtime and generation time); Plan 0010 closed the stats HUD, v0.9.0, and Plan 0016 its allocation-free refresh, v0.13.0; Plan 0025 closed the cutaway view, v0.20.0; the other debug views and the profiler across passes remain, and Plan 0068 closed the benchmark lane's first form, the tester build (camera paths, run reports, Linux and Windows archives; shipped in v0.54.0, the first release, 2026-11-15); Plan 0088 added its fourth scene, the outdoor hills, trees and tunnel on a computed path, v0.63.0, 2026-11-17; Plan 0026 closed gamepad flight, v0.21.0; Plan 0043 closed the test tiers, v0.29.1; Plan 0042 closed the generated castle, v0.30.0; Plan 0066 closed the 10 s soak line, 2026-11-11; Plan 0075 closed the loading screen, v0.58.0: start-up stepped on the app's frame loop behind an engine loading frame, 2026-11-16; Plan 0083 closed structures from any `.vox`, v0.66.0, 2026-11-19: every palette entry a material row and the table a text file reloaded live (ADR-0065), prefabs as bricks and a stamp that outlives residency as a placement (ADR-0066), a structure as a belt item placed under a coarse ghost of itself (ADR-0078))

- Prefab stamping with rotation and masks; debug views (material ids, acoustic properties,
  coarse grids, flags); an in-app profiler overlay with GPU timestamps; a benchmark lane
  (`criterion` for CPU, timestamp queries for GPU) with readings committed per named machine.
- The profiler across passes and threads: Plan 0091 (approved 2026-11-19, ADR-0077 accepted):
  `profiling` facade scopes in `svet-render` and the apps, none in `svet-world`,
  collected by puffin only under the fly-through's `profiler` feature; a dump as a Chrome trace,
  a text tree and `run.json`'s `scopes_top` with a ranking script for agents; the GPU passes as
  a lane; budget scopes that count overruns against `docs/nfr.md`'s lines; allocation counting;
  the HUD's frame ticker with a frozen frame. **Order (owner, 2026-11-19): after Plans 0087 and
  0089, before Plan 0090**; `docs/plans/README.md` carries the sequence.
- WGSL hot reload in the fly-through: Plan 0092 (approved 2026-11-19, no ADR): an
  mtime poll, the frame's pipelines rebuilt from disk inside an error scope, the old ones kept
  and the error shown on the window on a compile failure. The owner placed it right
  after Plan 0091, before Plan 0090, in its lane.
- The runtime stamp is op kind 2 through `Server`, logged and replayed like an edit (Plan
  0018, v0.15.0).
- Stamping at authored-building scale: a generated castle and dungeon, a 27-tile `.vox`
  scene through the importer and `StampedSource`, with its RAM and generation costs read (Plan
  0042, v0.30.0: 343 566 464 B of prefabs, about 92 % air; sparse prefab storage stays a
  followup).
- Anything from a `.vox` (Plans 0083 and 0084, drafted 2026-11-16, await go): every palette
  entry a material row and the material table a text file (ADR-0065); prefabs stored as bricks
  and a stamp past the residency sphere kept as a placement the world finishes as chunks arrive
  (ADR-0066); layers, hidden nodes, mirrors and an integer scale from the scene graph; a
  Structures page in the fly-through; then staged structures as logged stamps with a write
  policy and a region, revealed over ticks and demolished (ADR-0067).
- Test tiers for the gate every lane runs: `quick` (CPU only, no perf lock) after every dev
  phase, `gpu` under the lock for render phases, `gate` (everything but the soak list) at a
  plan's tip, its close and in CI, `soak` weekly and on request, `full` only when the owner
  asks (re-cut 2026-11-03), kept true by `scripts/check-test-tiers.mjs` (Plan 0043, v0.29.1). The soak
  line is 10 s on lavapipe, and a phase runs only the `gpu_*` binaries it names
  (`test-changed.mjs --binaries`; Plan 0066, 2026-11-11).

### M8: Volumetrics, debris and collapse (support query and collapse ops shipped in Plan 0009, v0.8.0; volumetrics, debris and the cosmetic rigid-body layer remain open, the last after M6)

- Dust, fog, mist and smoke as a separate coarse volume (a froxel grid or a per-brick density
  grid, marched at half resolution with a fixed step count and composited over the opaque
  hit; owner's call, 2026-10-17), never as voxels in the brick map; short-lived particle
  debris from edits. Translucent solids (glass, water, lava) are voxels of a translucent
  material class instead, composited by Plan 0015's bounded walk.
- **Support query** in `svet-world`: after an edit, the set of coarse-grid cells (0.8 m, from
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
  region, and replicate through M5's log as op kinds 3 and 4. Shipped in Plan 0009 as
  `World::drop_region` and `World::remove_region`, and logged since Plan 0018 (v0.15.0), where
  the logic asks `Host::unsupported` and issues the drops or removals. Since Plan 0022
  (v0.22.0, ADR-0016) a drop takes a yield threshold (`World::drop_region_through`, op kind 5):
  it passes through and crushes the non-indestructible voxels of cells under the query's
  `solid_min`, so a region the query detaches falls instead of wedging on a sliver, and
  `World::pile_rubble` (op kind 6) puts the crushed volume back as rubble on the landed piece.
- **Stress (open; owner's call, 2026-10-17):** overhangs fail by span and weight, not only
  when the support query finds them disconnected. The owner's windowed test of v0.12.0
  found collapse rarely triggers in the cave, because a cell one eighth solid still bears
  load and a thin bridge holds an overhang up. Needs an ADR revising ADR-0008's
  connectivity-only rule (the alternatives include tuning the load-bearing threshold, which
  the owner did not choose) and its own plan; not drafted.
- **Cosmetic rigid bodies**: a client-side layer, in its own optional crate, that animates the
  pieces of a collapsed region with a rigid-body library (`rapier` or similar) between the
  server's "region R fell at tick T" and the landed result, then fades the pieces into the
  authoritative world. It never writes to `svet-world` and nothing deterministic reads it.
- **Headless proof:** carving the single pillar under a 3 × 3 cell slab returns that slab as
  one unsupported region with its exact material counts; the drop op lands it on the floor
  with every voxel in place and the same hash on both CI arms.
- **ADRs expected:** voxel dynamics versus a physics engine (the rejected alternatives are a
  general rigid-body engine in the authoritative world, scripted-only collapse, and rotating
  pieces): ADR-0008, accepted at Plan 0009's close.

### M9: Networking

- `svet-net`: a QUIC listen server (`quinn`), prediction and reconciliation for local edits,
  entity replication with interest management, late join by seed plus compressed op log,
  positional voice as an audio source through M4's occlusion.
- **ADRs expected:** transport; voice codec.

### M10: Outdoors (Plans 0032 to 0037 approved 2026-10-22; its first plan, Plan 0069 the render profiles, closed 2026-11-17, v0.60.0; Plan 0032 closed 2026-11-17, v0.61.0; Plan 0086 closed 2026-11-19, v0.67.0; Plan 0094 closed 2026-11-20, v0.68.0; Plan 0095 closed 2026-11-21, v0.69.0; Plan 0097 closed 2026-11-23, v0.70.0; Plan 0096 closed 2026-11-23, v0.71.0; Plan 0098 closed 2026-11-24, v0.72.0; Plan 0100 closed 2026-11-25, v0.75.0; Plan 0099 closed 2026-11-26, v0.76.0; Plan 0104 closed 2026-11-26, v0.77.0; Plan 0101 closed 2026-11-26, v0.78.0)

The open-sky half of the "Floor, profiles and scene scale" decision, as general engine
mechanisms: the first consumer is underground, so nothing here is on its P0 list, and terrain
stays the game's `WorldSource` (the fly-through gets an outdoor source with a cave mouth to
show it). Owner's interview, 2026-10-22: outdoors is a general capability, the view reaches
about 1 km, liquids render first and flow later, and the sky and weather are fully dynamic but
set by the game, never by a wall clock in the engine.

**Order (owner, 2026-11-03):** Plan 0051, Plan 0056 (the aliasing spike) from its verdict, the verdict plan, Plans 0044 and 0045, Plan 0048 if it
stands (the verdict shrinks it to scheme A or C, or drops it), then this milestone. As of
2026-11-05: 0051, 0056 and the verdict plan 0057 are closed, Plan 0059's reference and an antialiasing plan come next (owner), the far-normal plan Plan 0056
owes comes with the antialiasing plan, and 0048 was dropped under
ADR-0043 (owner, 2026-11-04). Its first plan
is a render-profile plan, the owner's parked settings and profiles idea, which holds M10 against
the budget and comes before Plan 0032. Drafted 2026-11-11 as [Plan 0069](plans/done/0069-render-settings-one-surface-an-apply-class-per-field-and-named-quality-profiles.md) with ADR-0055, approved the same day with every recommendation taken; closed 2026-11-17 (v0.60.0) with four profiles (Floor, Portable, Performance, High) and the frame lines below.

**Order (owner, 2026-11-10; amended 2026-11-11 and 2026-11-12):** Plans 0060, 0061, 0067, 0045, 0070, 0058,
the rest of 0044, the render-profile plan (Plan 0069), Plan 0071, Plan 0073 (the settings menu and file, as pages of Plan 0080's menu; 0080 itself closed 2026-11-17, v0.62.0, ADR-0061 accepted), then the plan that removes the `own` upscaler (Plan 0102, closed
2026-11-25, v0.74.0, ADR-0086 accepted), then M6 (Plans 0062, 0063), then this milestone in plan order,
0032 to 0037, each on main after the one before it closes. **Reordered 2026-11-16 (owner):** Plan 0069, then 0032, start once Plans 0062 and 0063 merge, beside Plans 0077 to 0080; Plan 0071, Plan 0073 and the `own` removal no longer hold M10. The six plans were rewritten on 2026-11-10
against the baked store: no probe or store term takes sun or sky light.

- **Sun and sky** ([Plan 0032](plans/done/0032-sun-and-sky-hillaire-atmosphere-time-of-day-and-an-outdoor-testbed.md),
  closed 2026-11-17, v0.61.0; ADR-0026 and ADR-0027 accepted with Outcomes): Hillaire's atmosphere LUTs built on the GPU,
  a time of day the game sets, the sun or moon as one dedicated directional sample with a
  per-pixel shadow ray in the light pass, sky light from a sky ray; both land in Plan 0058's
  direct target, with no probe or store term and no sunlight bounce (redesigned 2026-11-10).
  Auto exposure. After Plan 0058 and the render-profile plan. Its testbed trees are prefabs
  stamped at generation time (amended 2026-11-16, owner). Its Phase 9 window (2026-11-17)
  picked the defaults and reversed "an absent chunk is solid": unknown is lit (ADR-0027's
  Outcome).
- **Still outdoor light** ([Plan 0086](plans/done/0086-still-outdoor-light-baked-sky-visibility-a-long-history-solid-canopy-shadows-and-stair-history.md),
  closed 2026-11-19, v0.67.0; ADR-0070 accepted with an Outcome): a sky visibility baked per
  brick direction into the per-chunk store and coloured by the sky at read, a sky ray that has
  reached the sky only above its column's top (the column map), the sky ray only where the
  bake is stale on a running mean with a firefly clamp and a seed from the record, the key
  light's visibility on the same pair at its own cap, a canopy that shadows as its brick (the
  default `voxel`; `brick` and `absorb` settings), history kept across a stair in the far-normal
  regime, the exposure anchored to the sky, a chunk an edit fills from air baked as a dig, and
  a penumbra spike kept as a setting. Four owner windows picked the defaults. What it left
  (the canopy's grain at rest, the dig's hole band, the tunnel's far end, the cave's seal
  clause, the light pass's compile cost, three Phase 5 readings that regressed) is Plan 0094's.
- **The light's transitions** ([Plan 0094](plans/done/0094-light-transitions-the-key-means-age-the-no-ray-prior-and-its-fades-the-hole-band-and-the-castle-fill-in.md),
  closed 2026-11-20, v0.68.0; ADR-0070 gains an Outcome, ADR-0076's rules 3 and 5 built and
  rule 6 added while it stays proposed with Plan 0090): the key mean keeps its age at a still camera, a
  face with no record reads its own chunk's summary or casts the ray seeded from the column,
  one 16-frame fade for every no-ray value change, the exposure moves at 3.75 EV/s, its
  ceiling follows the sky in view and the sky anchor never lifts it over `max` (the night at
  +2 EV), the default-off light arms out of the default pipeline; the tunnel's rim band and
  the cave's seal clause back as tests. Three owner windows (94a to 94c). The hole's floor, the
  dig's first frames and the motion wobble stay open (the backlog, Plan 0095).
- **A calm key light in motion** ([Plan 0095](plans/done/0095-a-calm-key-light-in-motion-the-column-maps-horizon-at-the-far-the-baked-record-where-it-lands-a-deterministic-penumbra-near-no-pre-roll.md),
  closed 2026-11-21, v0.69.0; ADR-0079 accepted with an Outcome): the key light's estimate
  carries no screen history: the far reads the record baked per brick direction where fresh,
  else the column map's horizon toward the sun, with one landing fade; the near reads the
  closest-approach penumbra of one centre ray (the fly-through's default on every adapter);
  key-only re-bakes follow a turning sun. Two owner windows (W1, W2: "overall we are much
  better than before"). Left open: the far normal's band still casts the ray at most far pixels
  (the backlog), the RENOIR's light pass at 34 ms in the forest (Plan 0096 Phase 6), the
  landings' wait (Plan 0096), the exposure in deep shade (Plan 0097).
- **The exposure's anchor by the sky around the camera** ([Plan 0097](plans/done/0097-the-exposures-anchor-by-the-sky-around-the-camera-the-floor-record-under-the-eye-a-time-constant-and-the-canopys-shade.md),
  closed 2026-11-23, v0.70.0; ADR-0081 accepted with an Outcome): the ceiling's anchor holds
  by the larger of the sky in view and the sky over the camera's floor (the baked record under
  the eye, smoothed over 1 s), so a canopy closing over the last sky pixel no longer opens the
  eye; where it releases, it opens at most a lift of 4 EV over the daylight anchor, so a
  tunnel by day stays dark and opens to `max` at night; highlight protection caps the eye
  under the brightest percent of the view. Three owner windows (97: "deep shade was better",
  97b, 97c: "1a looks great"). Left open: whether highlight protection wants its own ADR (the
  owner), the time constant's 1 s unjudged by eye, the near jumps after a stop (Plan 0098).
- **The bake queue's drain as a budget** ([Plan 0096](plans/done/0096-the-bake-queues-drain-tiers-a-drain-slice-the-step-and-the-slow-adapters-light-pass.md),
  closed 2026-11-23, v0.71.0; ADR-0080 accepted with an Outcome): a chunk with no face or
  buried with no emitter in reach gets its record without a GPU job, every other chunk bakes at
  a sample tier set by its size on screen (the far tiers from the chunks resident), and the
  slice holds a drain value while in-view work waits; the queue's step no longer grows with
  its depth and the eviction scan's stagger is gone (the owner: "no stagger"). The outdoor
  queue now lands, which it never did. **Its lines are mostly missed and disclosed:** the RTX
  lands the forest's view in 36.5 s against 10 at about 30 fps, bound by the step's CPU per
  job on the render thread (Plan 0099); the RENOIR at Portable lands in 27.1 s against 30 and
  draws 23.7 fps in a window (its march at 853 x 480, FSR 2 to 1280 x 720, stretched; the
  owner: softer, "acceptable"). Two owner windows (W96, W2: the far tiers' "shadows are
  fine"). Left open: the dig under Portable (bright spots, "grainy", not attributed), the
  record by rule at every opening and the bake's key ray at its cap (a small plan ahead of
  Plan 0099), `alloc` and the glide's `light place` (the slot words on the workers), the
  RENOIR's hit pass (the beam prepass).
- **A calm light by default** ([Plan 0098](plans/done/0098-a-calm-light-by-default-no-screen-history-smooth-sources-to-the-mid-range-and-the-fsr-2-turn.md),
  closed 2026-11-24, v0.72.0; ADR-0082 accepted with an Outcome): the light passes keep no
  screen history on any profile; the sun's shadow on a world face is the column map's horizon
  at every distance and the far normal's band is `4..8` pixels a voxel, so the image right
  after a move is the image at rest (the owner: "wobble on slopes is gone", "no grain"; light
  history on rejected at W3). The unshown key ray is not cast (the RENOIR's light pass at
  1080p 1.6 to 2.1 ms, under its line). A dig's near ring jumps the queue and lands in 1 or 2
  frames on both adapters, the hole holding a record in the store until then (window 98e: "so
  much better, not ideal but acceptable"). Five owner windows. Left open: no sun under cover,
  the noon terraces' step shadows and the coarse tier's wobble (Plan 0100, closed 2026-11-25
  with the first two still open, see the next entry); the
  distant trees under FSR 2 (the backlog, Plan 0033's re-read); the slight colour change at a
  dig's landing and the staged-records delay (the backlog); the bake step on the main thread
  (Plan 0099, closed 2026-11-26).
- **A sun fixed in the world** ([Plan 0100](plans/done/0100-a-sun-fixed-in-the-world-the-key-shadow-a-face-at-a-time-the-coarse-tier-held-still-and-sun-under-cover.md),
  closed 2026-11-25, v0.75.0; ADR-0084 accepted with an Outcome, superseding two clauses of
  ADR-0079). **The wobble it was written to remove is still there**, on sunlit faces, and no
  reading of the plan shows it; the plan closed on what the owner's window left standing. No
  default changed on any profile. The one change every build takes: a baked key ray that
  ends under its column's top is not lit (a closed cave no longer bakes sunlit). Kept as
  settings, off: `key cover ray|record` (sun under cover, a source that only adds light),
  `key at face|face-smooth`, `key horizon walk` (every cell of the column map); of these only
  `key cover ray` stands since Plan 0104's close (the next entry). Under `key
  far column` no key ray is cast at a cube face for the penumbra. Closed with no change:
  the near trunks under coarse light (no fault read or seen). Left open, to the follow-on
  plan (the backlog's "The sun's term at small faces"): the wobble, the tunnel slot's
  shimmer, the near shadow's hole, sun under cover as a default; it starts from clips the
  owner judges, then a measure that ranks what he ranked (Plan 0104, the next entry).
- **The wobble anchored to the owner's eye** ([Plan 0104](plans/done/0104-the-wobble-anchored-to-the-owners-eye-clips-he-ranks-a-measure-that-ranks-them-then-the-remedy.md),
  closed 2026-11-26, v0.77.0; no ADR, an addition to ADR-0084's Outcome). **Closed with no
  remedy, by the owner's call:** his move was rendered to five lossless clips at his window's
  size, he watched them and ranked two pairs blind, and found the defaults "almost good".
  No default, golden or image bar moved. Removed, as ADR-0084 rules for a setting the
  follow-on plan does not pick: `key cover record`, `key at`, `key horizon` (its walk shader
  and the column map's coarse rows with it); **`key cover ray` stays a setting, off on every
  profile**, as the one control for sun under cover. The wobble rigs read a baked store (one
  shared settle with a census), and the forest soak test asserts on one such store. Left
  open, in the backlog with his clips as anchors: sunlit tree crowns sparkle in motion (cause
  not shown; not shown to be the sun's shadow); the picture settles for about a second after
  a turn (read as FSR 2's history: outlines change 22.05 levels with it, 0.007 without); the
  near shadow's hole and sun under cover as a default, not taken up. No measure of the wobble
  exists: the one reading that followed his eye is provisional, on one pair.
- **The bake step off the render thread** ([Plan 0099](plans/done/0099-the-bake-step-off-the-main-thread-a-bake-thread-write-lists-and-the-in-view-drain.md),
  closed 2026-11-26, v0.76.0; ADR-0083 accepted with an Outcome, extending ADR-0017): the live
  bake queue's books and per-job CPU work run on one bake thread; the render thread applies
  its write lists in the order made (a block's words and its table word always in one list)
  and issues its submits, one a frame; an edit's frame waits for its step's lists, so a dug
  hole holds its record in the frame of the dig; the same code runs in lockstep as a setting
  (`bake_thread`, on in a window, off under `--headless` and in every profile, so tests and
  goldens are unchanged). The owner at its window: "with thread it got so much better, this
  is the way!". On the RTX 3080 Laptop at Floor the frame while the forest's view bakes is
  8.3 ms against 21.9, the render thread's bake share 0.76 to 0.80 ms p50; the RENOIR at
  Portable 24 ms against 33.7. **Missed and disclosed:** one thread feeds 0.58 to 0.61 of the
  RTX's drain slice against 0.85, and the RTX forest's view is final at 17.9 and 19.0 s
  against 10 (every chunk in view has some light at about 8 s; the hills meet at 6.5 s).
  Left open: the feed's levers and the readings not taken (the backlog), and a flash when
  digging deeper than 2 to 3 m, the same with the thread off (Plan 0105, drafted).
- **A surface detail layer** ([Plan 0081](plans/0081-a-surface-detail-layer-blades-caps-and-pebbles-marched-above-styled-faces-swayed-by-wind.md),
  drafted 2026-11-16, awaits the owner's go; ADR-0063 proposed): grass, moss, strands,
  fungus caps, flowers, pebbles and snow tufts as a procedural shell marched in the hit pass
  where a ray meets a face whose material has a style, lit as a hit, faded to the face's
  albedo by the voxel's size on screen, swayed by a wind the game sets; a render-side table
  per material id, cosmetic and client-only, nothing in the world, the op log, the queries or
  the bake. After Plan 0032, before Plan 0033; it takes Plan 0036's wind when that lands.
- **The far field** (Plan 0033; ADR-0028, the founding interview's far-field clipmap): up to
  three coarse chunk levels to about 1 km, generated by the source directly, edits folded in
  from full detail; the sun's shadow ray continues into it. The level rule is per caller: the
  bake and the probe rays take no far field, and a far hit reads no baked light. After Plan 0032.
- **Liquid surfaces** (Plan 0034; ADR-0029): water and lava stay translucent voxels; their
  surface gets analytic waves, a traced reflection ray, a bent refraction ray and Beer-Lambert
  absorption, lit by the store, the direct target and the sky ray. After Plan 0033. Flat faces with analytic normals stay, which answers the owner's
  open item of 2026-11-01 on smooth liquids (owner, 2026-11-03).
- **Aerial perspective and clouds** (Plan 0035; ADR-0030): an aerial-perspective volume over
  the near and far field, a volumetric cloud shell marched in an amortised quarter-resolution
  buffer, and a cloud shadow map on the sun sample in the light pass. It shares M8's composite
  rule, not its volume. After Plan 0034.
- **Weather** (Plan 0036; ADR-0031): rain, snow, wind, wet surfaces, snow cover and lightning
  as cosmetic client state the game steps, rendered against a fall map traced through the
  mirror and never written as voxels. Puddles reuse Plan 0034's reflection; the flash stays out
  of the exposure meter and the light list. After Plan 0035.
- **The night sky** (Plan 0037; ADR-0032, approved 2026-10-22): a seeded star field, the
  moon's phases, sky bodies the game lists (a comet's tail, meteors), an aurora shell whose
  glow lights the ground through a hemisphere map read by the sky ray, and a blue-shifted,
  desaturated night tint driven by scene luminance. After Plan 0036.
- **The castle outdoors** (not drafted): Plan 0042's generated castle on open terrain under
  the sky, the owner's pick after its flight. After Plan 0032's outdoor source.
- **Flowing liquids** (not drafted): a deterministic cellular flow in `svet-world` whose steps
  are ops through the log, a later plan with its own ADR.
- **Headless proof:** each plan's goldens on lavapipe (noon, sunset, night, cave mouth, far
  field, lake, under water, lava, overcast, cloud shadow, rain at a cave mouth, snow cover,
  lightning) and analytic bars (zenith transmittance, sun-lit floor irradiance, Fresnel and
  refraction offsets, shadow-line positions).
- **Budgets:** frame time on both adapters (`nfr.md` §1, §2), the mirror's VRAM line for the
  clip levels (§3), and streaming throughput for coarse generation (§5).

### Unscheduled: PC VR

Not a numbered milestone and not on the first consumer's list. Recorded so that the doors the
VR-readiness decision keeps open stay open, and so the cost is on paper before anyone asks.

- **What favours the engine.** Compute ray marching has no per-view vertex work: a second eye
  is a second ray origin and basis, and the head pose is a uniform written just before the
  dispatch, so late latching is free. The marcher picks its own rays, so foveation is a
  dispatch pattern over tiles rather than a vendor extension. HRTF audio (M4) is already
  head-relative. Voxel-model entities (M6) trace in the same pass, so there is no second
  pipeline to make stereo.
- **What it would take.** A `svet-xr` shell next to `svet-app` that owns the OpenXR session and
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
  SSIM, analytic irradiance) from the start, and the slow adapter's readings at every render
  plan's close (`nfr.md` §1); the baseline card is not acquired (decisions table, 2026-10-21),
  so the floor stays a projection.
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
  both arms from M1 on, and `clippy.toml` can deny the platform transcendentals in `svet-world`.
- **The upscaler port (M2).** No WGSL port of FSR 2 exists. Mitigation: the M2 plan sizes the
  port as its own phase and names the TAA fallback. Plan 0030 shipped our own upscaler opt-in
  (2026-10-22); Plan 0041 (ADR-0034) ported FSR 2.2.1 and closed 2026-10-27 with FSR 2
  opt-in and our upscaler at scale 1.5 the default; Plan 0055 (2026-11-03) made FSR 2 at 1.5
  the default and ours opt-in: the risk is retired.

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
