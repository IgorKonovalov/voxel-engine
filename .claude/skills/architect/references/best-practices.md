# Best practices: voxel engine

The correctness rules for this codebase. The architect checks against them in Mode 4, and `dev`
implements against them whether or not a plan restates them. They're ordered roughly by how much
damage a violation does. A rule that turns out wrong for this project is changed by an ADR, not
ignored.

## Game-agnostic API

- **Nothing in `crates/` names a gameplay concept.** No loot, monster, item, level, noise meter.
  A material row carries an opaque payload (`u32` or bytes) the consumer interprets; the engine
  never reads it.
- **A consumer need becomes a general mechanism.** "Return per-material counts from an edit"
  is general. "Roll the loot table" is not.
- **Public types are newtypes with units.** `VoxelCoord` (i32 lattice), `ChunkCoord`,
  `MaterialId(u16)`, `Fixed` (1/256 voxel). Meters appear as `f32` only at the API edge with a
  documented conversion through the world's voxel size.
- **Feature flags, not forks.** Optional subsystems are Cargo features or separate crates; there
  is one world type.

## Determinism

- **`vxe-world` is a pure function of its inputs.** No `Instant::now`, `SystemTime::now`
  (`clippy.toml` denies them), no `rand::rng()`, no environment reads. Seeds are explicit
  parameters.
- **Edit predicates use integer or fixed-point math.** A sphere is `dx² + dy² + dz² <= r²` in
  `i64`; a capsule is a segment-distance test in the same arithmetic. No `f32` anywhere between
  op parameters and voxel writes. Rounding rules for fixed-point are documented once and tested.
- **Results are canonical.** Per-material counts are reported in material-id order, never in
  `HashMap` iteration order. Touched chunk sets are sorted.
- **Thread order never leaks into results.** Parallel generation or edit application produces
  the same bytes as serial; a test asserts it.
- **A determinism test pins a hash** of the voxels and the results on a fixed op list and seed,
  and CI asserts it on Linux and Windows.
- **Server-side scripting is configured for determinism:** canonical NaNs, no clock imports,
  fuel metering, the same module bytes on every peer.

## World and memory

- **Storage serves edits, streaming and determinism first.** The GPU layout mirrors it and is
  uploaded incrementally from dirty tracking; the renderer never becomes the owner.
- **Uniform bricks are one value.** Expanding a uniform brick to 1 KiB is a bug, not a
  simplification. Compaction after an edit that makes a brick uniform again is part of the edit.
- **Nothing grows with the world unbounded.** Resident chunk count is capped by the budget;
  eviction is a first-class path with a test. Edit persistence for evicted chunks is bounded and
  measured.
- **No per-frame heap allocation in the frame path.** Scratch buffers are reused; a `Vec` that is
  allocated per frame is a finding.
- **A budget figure is measured on named hardware** and recorded with its commit. "Fast enough on
  my machine" is not a reading.

## GPU

- **All `wgpu` types stay inside `vxe-render`.** The world exposes byte slices and dirty ranges;
  the renderer owns buffers, bind groups and pipelines.
- **Feature use is checked against the adapter at device creation**, with a documented fallback
  or a clear error. Subgroups, 64-bit atomics and binding arrays are the usual suspects.
- **Shaders never branch on the backend.** One WGSL source per stage, documented bindings at the
  top, bounds checks on every indexed read that depends on world data.
- **Resources are created once and resized on demand**, never re-created per frame. Dispatch
  sizes respect workgroup limits for the smallest supported adapter.
- **Timing is measured with timestamp queries**, not wall-clock around `queue.submit`.
- **Golden images are blessed on lavapipe** with a stated floor (SSIM or max pixel delta).
  Readings from other adapters are recorded as readings, never used to bless.

## Threading

- **The render thread never blocks on generation.** Chunks arrive through a channel; the thread
  swaps pointers and issues uploads inside its streaming budget.
- **No lock is held across a chunk generation, an upload or a script call.**
- **Background workers take explicit seeds and chunk coordinates**, so their output is
  reproducible regardless of scheduling.

## Errors, safety, dependencies

- **`Result` at crate boundaries** for expected failures (missing chunk, unsupported adapter,
  malformed asset). **`panic` only for invariants**, and never in the frame path once the device
  exists.
- **`unsafe` is rare, wrapped in a safe function, and carries a `// SAFETY:` comment** stating
  the invariant. `bytemuck` derives for GPU-bound structs; no hand-rolled transmutes.
- **Dependencies are pinned exactly** in `[workspace.dependencies]`, the lockfile is committed,
  `cargo deny` runs in CI, and a new dependency is named in the plan that adds it.

## Tests

- **Every feature has a headless test.** The world crate is tested without a GPU; render tests
  use the software adapter and skip, with a printed reason, where none exists.
- **An assertion states the plan's claim with its exact value**, or a property with a stated
  tolerance. `is_ok()`, `is_some()` and "did not panic" are not assertions of behavior.
- **A test never pins live, growing data** (the material table of the fly-through, the shader
  list). It derives the expectation from the source or uses a fixture.
- **Benchmarks are readings, not gates**, until a figure in `nfr.md` has been measured on the
  baseline; then the reading becomes the floor and a regression is a finding.
