# ADR-0001: Rust library workspace, wgpu compute ray marching, streaming brick-map world

> **Status:** accepted
> **Date:** 2026-10-16
> **Related plan(s):** none yet; Plan 0001 (milestone M1 in [`docs/roadmap.md`](../roadmap.md)) is the first consumer

## Context

We are building a voxel engine that has to serve more than one game. The owner's requirements
are: reusable and game-agnostic, extremely frugal with memory, CPU and GPU, able to render large
worlds, and ray traced. The first consumer is a first-person sound-horror game
(`~/Work/voxel-roguelite`, `docs/engine-requirements.md` there) whose non-negotiable needs are:

- 10 cm voxels seen at arm's length, in levels of roughly 128 × 128 × 48 m, so about 800 M cells
  per level of which almost all are solid rock or air.
- Runtime carving that returns, for every edit, the number of voxels removed per material. That
  result drives the game's loot, so it has to be exact and identical on every machine.
- Mostly dark scenes lit by a few sources, including emissive voxels whose emission changes at
  runtime, with a denoiser that survives disocclusion caused by digging.
- Acoustic queries (occlusion, transmission, early reflections) through the same grid the
  renderer uses, plus a coarse occupancy grid the game reads for its own noise model.
- A server/client split from day one even in solo play, with edits as compact deterministic
  operations, and no online dependency.

Four forces make the technology choice non-obvious:

1. **Voxels and hardware ray tracing are a poor fit.** RT cores accelerate triangle BVHs. Voxel
   geometry would enter as procedural AABBs with intersection shaders and lose most of the
   hardware's advantage. Every production voxel ray tracer we know of (Teardown is the reference)
   walks a brick map or sparse octree with a DDA in a compute shader instead.
2. **Portability versus vendor features.** A portable abstraction gives Linux and Windows (and
   CI's software adapter) from one code path. Native Vulkan gives DLSS, vendor denoisers and
   earlier access to new extensions, at the price of a desktop-only, NVIDIA-leaning engine and
   several times the boilerplate.
3. **A framework's ergonomics versus its cost.** Bevy would give an ECS, assets, input and UI for
   free, but we replace its entire render pipeline, and its app and schedule model fights a
   deterministic, authoritative server loop.
4. **Bounded versus streaming worlds.** The first consumer's levels are bounded. The owner wants
   the engine to serve larger worlds too. Paging retrofitted onto a bounded brick map rewrites
   its addressing.

The owner already runs a Rust + `wgpu` + `winit` application in production on three platforms
(Ritmolux), with a plan/ADR harness that this repo copies. The toolchain, the software-adapter
CI pattern and the lint gates are known quantities.

## Decision

We build the engine as a **Cargo workspace of focused library crates in stable Rust** (edition
2024, toolchain pinned in `rust-toolchain.toml`). There is no framework: a game composes the
crates it needs (`vxe-world`, `vxe-render`, later `vxe-sim`, `vxe-audio`, scripting hosts,
networking) and owns its own loop and entity model. `vxe-app` is a thin, optional `winit` shell
for games and tools that want one.

Rendering goes through **`wgpu` with WGSL shaders**. Primary and secondary rays are **DDA walks
through a brick map in compute shaders**; there is no hardware ray-tracing dependency. The
temporal denoiser and the upscaler are ours (the upscaler ported from an open-source shader
set). The quality floor is a **2019 mid-range desktop GPU with 6 GB of VRAM** on **Linux and
Windows**; `docs/nfr.md` carries the numbers. macOS is not excluded by the API choice but is
neither tested nor gated.

The world is a **streaming, chunked brick map from the first milestone**: chunks page in and out
around one or more focus points, each chunk carries LOD mips for distant rays, and generation
runs on background threads behind a `WorldSource` trait the game implements. A bounded level is
a finite source, not a different storage. Full detail is targeted at hundreds of meters at 10 cm,
with LOD out to about a kilometer. Voxel size is one scalar per world (meters per voxel), not a
multi-resolution scheme.

`vxe-world` is **headless and deterministic**: no GPU API, no wall clock, no unseeded RNG, and
edit operations evaluated in integer or fixed-point arithmetic so that the same operation on the
same world produces bit-identical voxels and bit-identical per-material results on every
platform. Edits are operations (shape, parameters, tool), never voxel diffs, and every edit
returns what it changed.

## Consequences

### Positive

- One shader language and one backend abstraction cover both shipped platforms and the
  software adapter CI runs on, so render tests are deterministic and automated.
- Compute DDA gives voxel throughput equal to or better than BVH tracing for this geometry, with
  no RT-hardware floor, so the baseline GPU is a 2019 card rather than an RTX one.
- The library shape keeps the engine small, keeps every crate testable without a window, and
  leaves the ECS and loop choice to each game. Reuse is by Cargo dependency.
- Streaming from day one means the storage layout and chunk addressing never need a rewrite when
  a consumer wants a larger world; bounded worlds cost nothing extra.
- Determinism and edits-as-operations make the server/client split, replays and late-join
  (seed plus op log) cheap later instead of a retrofit.

### Negative

- **No DLSS and no vendor denoiser.** Image quality in dark, sparse-light scenes rests on our own
  denoiser and upscaler, which is the largest single technical risk in the roadmap (M2).
- **`wgpu` lags native Vulkan** on a few features we may want (bindless resource arrays, 64-bit
  atomics on all adapters, ray queries are experimental). Some techniques need workarounds, and a
  vendor-specific path is a new ADR, not a quiet branch.
- **Streaming lands before the first pixel.** Paging, LOD mips, async generation and persistence
  of edits in paged-out chunks are complexity Plan 0001 pays for that a bounded world would not.
- **No framework means we write the plumbing.** Window, input and asset loading are ours, thin
  as they are, and games get no scene editor from us.
- **Compile times.** `wgpu`, `wasmtime` and `mlua` are heavy. The workspace split and
  `default-members` keep the everyday loop to the crates a plan touches.

## Alternatives considered

### Alternative A: Native Vulkan through `ash` with hardware ray tracing

Direct Vulkan, `VK_KHR_ray_query`, DLSS and a vendor denoiser. Rejected because voxels gain
little from triangle RT hardware, the engine would be desktop-only and NVIDIA-leaning, and the
boilerplate is several times `wgpu`'s. DLSS was the only strong draw, and an open upscaler
covers the baseline target.

### Alternative B: Bevy plugins

Engine crates as Bevy plugins, reusing its ECS, assets, input and UI. Rejected because the whole
render pipeline is replaced anyway, Bevy's app and schedule model fights a deterministic
authoritative server loop, and Bevy's release churn would set our upgrade cadence.

### Alternative C: Bounded volumes only, paging later

Simplest storage and the exact shape of the first consumer's levels. Rejected because the owner
wants the engine to serve larger worlds, and adding paging to a bounded brick map later rewrites
chunk addressing, upload and LOD, which is most of the storage layer.

### Alternative D: C++

The industry default for this class of engine, with the broadest middleware. Rejected for memory
safety in multi-threaded streaming and GPU upload code, and because the owner's toolchain,
harness and production experience are Rust.
