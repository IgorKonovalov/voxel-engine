# Non-functional requirements

The budgets the engine is held to. Every "fast", "large" and "stable" claim elsewhere means one
of the figures below. Each figure is a **proposal until a measurement replaces it**; a
measurement names the commit, the machine, the resolution and the scene. A plan that touches
storage or the frame states which figures it affects and measures them at its close.

Written 2026-10-16. Nothing here is measured yet.

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

## 4. Edits

- **Cost:** 100 sphere carves of radius 1 m (about 4189 voxels each at 10 cm) in one frame cost
  at most 2 ms of CPU including result counting and dirty marking, and their upload fits in the
  streaming line of §2.
- **Results** are returned synchronously with the edit, per material, exact.
- **Determinism:** the same op list on the same seed yields bit-identical voxels and results on
  Linux and Windows. CI asserts a fixed vector's hash on both arms.

## 5. Streaming

- Chunk generation and LOD building run on background threads; the render thread only swaps
  pointers and issues uploads.
- Moving the focus point at 10 m/s keeps the full-detail radius resident; a chunk entering the
  radius is resident within 500 ms (proposal).
- An edited chunk that is evicted and later reloaded reproduces the edit.

## 6. Determinism and headless operation

- `vxe-world` builds and tests without a GPU, a window or a display server.
- No wall-clock reads in `vxe-world` or in server-side code; `clippy.toml` enforces it.
- Everything the engine generates takes an explicit seed. There is no engine-owned global RNG.
- Render tests run on the software adapter (lavapipe) in CI; golden baselines are blessed there
  and real-vendor readings are recorded separately, never substituted.

## 7. Startup and build

- Time to first frame in `apps/flythrough` under 3 s on the dev reference (proposal).
- An incremental `cargo build` of a one-crate change under 10 s on the dev reference; the
  everyday loop does not compile crates a plan does not touch (`default-members`).

## 8. Not goals (for now)

- Binary size.
- Mobile, web or console targets.
- Frame rates above 60 fps as a gate; they are welcome, not required.
