# Project context: voxel engine

Facts the architect needs to ground decisions. **This file does not enumerate ADRs or plans.**
`docs/adrs/README.md` and `docs/plans/README.md` are the live indexes, and a copy here would rot.
Trust `git` and the tree over this file when they disagree, and surface the drift.

## What the engine is

A reusable, game-agnostic voxel engine: a streaming brick-map world of small voxels, ray-marched
in compute shaders through `wgpu`, edited by deterministic operations that report what they
changed, and queried for collision, acoustics and navigation. A library workspace, not a
framework (ADR-0001). `docs/roadmap.md` has the milestone sequence; `docs/nfr.md` the budgets.

## The first consumer

`~/Work/voxel-roguelite`: a first-person sound-horror delve with ARPG-depth loot, in seeded
procedural underground levels where destruction feeds the loot and spawn systems. Its engine
needs are in that repo's `docs/engine-requirements.md` (living document, read it when planning).
Condensed, with the consumer's own priorities (P0 first playable slice, P1 before content, P2
later):

| Area | Needs |
|---|---|
| World | P0: bounded levels about 128 × 128 × 48 m at 10 cm voxels, adjustable; one 16-bit material id per voxel. P1: stacked levels generated in the background; prefab stamping with rotation and an indestructible-shell mask |
| Edits | P0: sphere, box, capsule and tool-along-ray carves, many per frame, local acceleration updates; **every edit returns voxels removed per material plus location**; indestructible flag. P1: additive edits; visual debris. P2: scripted collapse. No structural physics |
| Rendering | P0: first person, stable at arm's length; dark scenes with few lights; emissive voxels as runtime-changeable lights; denoising that survives disocclusion from digging. P1: volumetrics; entities lit consistently. P2: loot highlighting. Look (owner, 2026-10-16): smooth, natural rock, not visible cubes; the mechanism is chosen by measurement after M1 |
| Materials | P0: data-driven table with albedo, roughness, emission, hardness, indestructible flag, an acoustic triple (absorption, transmission, resonance) and an opaque loot reference the engine only stores |
| Audio | P0: HRTF positional playback; occlusion and transmission through voxels by material. P1: early reflections and reverb from ray probes; material-driven synthesis. Separately, P0: a coarse 1 m occupancy/material grid the game reads for its own deterministic noise model, independent of render or audio rays |
| Queries | P0: raycast, sweeps, point and region material queries; character controller. P1: region change notifications; walkable-space extraction |
| Determinism | P0: generation a pure function of (seed, index, version); explicit seeds; no engine-owned gameplay RNG. P1: edits as compact ops for replay and networking |
| Networking | P0: server/client responsibility split even solo, in-memory channel, deterministic edit application, offline always. P1: listen server, prediction, replication, late join by seed plus log, positional voice. P2: dedicated servers |
| Tooling | P1: `.vox` import with material mapping; debug views |

The consumer's open questions for the engine (platforms, entity representation, final voxel size,
transport) were answered in the founding interview; see `docs/roadmap.md`.

The consumer's `CLAUDE.md` refers to this repo as `~/Work/voxel_engine` (underscore). The real
path is `~/Work/voxel-engine`.

## Stack (ADR-0001) and versions as of 2026-10-16

| Concern | Choice | Latest on crates.io at bootstrap |
|---|---|---|
| Language | Rust stable, pinned in `rust-toolchain.toml`, edition 2024 | 1.97.1 |
| GPU | `wgpu`, WGSL, compute DDA ray marching | wgpu 30.0.1, naga 30.0.1 |
| Window | `winit` | 0.30 stable (0.31 in beta) |
| Math | `glam` | 0.34.1 |
| GPU-bound structs | `bytemuck` | 1.25.2 |
| Audio I/O | `cpal` | 0.18.2 |
| Server scripting | `wasmtime`, component model | 50.0.0-rc.1 (use the latest stable) |
| Client scripting | `mlua`, vendored Lua 5.4 | 0.12.2 |
| Networking (M9) | `quinn` (QUIC) | 0.11.12 |
| CPU benchmarks | `criterion` | 0.8.2 |
| Tests | `cargo-nextest`, doctests, golden images on lavapipe | |
| Gates | clippy `-D warnings`, rustfmt, `cargo-deny`, hook bite tests, doc-link check | |

Pin exact versions in `[workspace.dependencies]` when a plan first needs a crate. Check the
adapter's feature set (subgroups, 64-bit atomics, binding array limits) in the plan that first
relies on it.

## Machine facts (dev reference)

- Arch Linux (Omarchy), Wayland. NVIDIA RTX 3080 Laptop 8 GB (driver 610) plus an AMD Cezanne
  integrated GPU on RADV (Mesa 26.2). Vulkan 1.4 on both. 16 threads, 30 GB RAM.
- `cargo-nextest` and `cargo-deny` are installed system-wide. Rustup has no global default
  toolchain; the repo's `rust-toolchain.toml` selects 1.97.1 inside the tree.
- Software adapter for headless render tests: Mesa's lavapipe (`mesa-vulkan-drivers` in CI).
- The baseline GPU (`docs/nfr.md` §1) is not on hand yet.

## Lessons from sibling projects

- **Ritmolux** (Rust, `wgpu`, `winit`, three platforms): keep heavy or non-shipping crates out
  of `default-members` so the everyday loop stays fast; bless golden images on lavapipe and
  record other adapters' readings separately; ban the wall clock in deterministic code through
  `clippy.toml`; a perf or coverage number in a doc names the machine and commit or it rots.
- **personal_expenses_bot**: the harness that works is small and mechanical: committed `.claude/`,
  deny-hooks with bite tests, independent plan and ADR sequences, a fresh-session close review.
  A rule that keeps being broken gets a gate, not a paragraph.
- **Both:** stage by explicit path, no agent attribution, the owner pushes.
