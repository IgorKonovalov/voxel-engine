# Architecture Decision Records

Numbered, append-only records of decisions that have a rejected alternative worth remembering.
Accepted ADRs are never edited in place. To change a decision, write a new ADR that supersedes
the old one and update the status here.

Rule of thumb: if you can't name an option you're *not* taking, you don't need an ADR, you need
a code comment.

**Next free number: 0036.**

An index row is a pointer, not an abstract: the link, the title as the ADR's `H1` writes it, and
the status.

<!-- roster:begin cap=320 -->
| ADR  | Title | Status |
|------|-------|--------|
| [0001](0001-rust-library-workspace-wgpu-compute-ray-marching.md) | Rust library workspace, wgpu compute ray marching, streaming brick-map world | accepted |
| [0002](0002-brick-map-layout-and-per-chunk-locked-access.md) | Brick-map storage layout, GPU mirror, and per-chunk locked access | accepted; mirror clauses superseded by ADR-0004; attribute reservation settled by ADR-0011; `Full` clause superseded by ADR-0012 (Plan 0021) |
| [0003](0003-sphere-residency-with-lod-ring-and-edit-persistence.md) | Sphere residency with an LOD-only ring, edited-chunk persistence with disk spill | accepted |
| [0004](0004-palette-narrowed-sharded-brick-pool.md) | Per-chunk 8-bit palette mirror and a brick pool sharded over fixed bindings | accepted; record head and masks changed by Plan 0015 under ADR-0012 (Outcome addendum) |
| [0005](0005-vertical-cylinder-collision-by-integer-conservative-advancement.md) | Character collision as a vertical cylinder swept by integer conservative advancement | accepted |
| [0006](0006-hrtf-source-and-audio-thread-model.md) | SADIE II HRTFs in a packed f32 format, time-domain FIR, mixer on the device callback | accepted |
| [0007](0007-server-op-log-byte-channel-shared-and-owned-clients.md) | Server-owned op log replayed from seed, a byte channel, and a client that shares the world in solo play | accepted |
| [0008](0008-collapse-as-capped-support-flood-and-rigid-vertical-drop.md) | Collapse as a capped support flood on the coarse grid and rigid vertical drop ops, not a rigid-body engine | accepted; drop clause amended by ADR-0016 |
| [0009](0009-testbed-hud-as-embedded-bitmap-font-overlay-pass.md) | The testbed HUD is an embedded 8 × 8 bitmap font drawn in one overlay pass, with no UI library | accepted |
| [0010](0010-game-logic-as-a-component-behind-a-logic-trait.md) | The game's server logic is a component behind a `Logic` trait, developed natively and shipped as WebAssembly; Lua is deferred | accepted |
| [0011](0011-contour-planes-in-attribute-pages-with-procedural-detail.md) | Smooth surfaces from per-voxel contour planes in attribute pages of the brick pool, with procedural per-material detail | accepted; shadow rule built, footprint via the pressure rule by Plan 0013 (Outcome) |
| [0012](0012-mask-first-brick-with-per-brick-palette-planes.md) | A brick is occupancy masks plus a per-brick palette in bit planes, the same bytes on the CPU and in the mirror | accepted; CPU format by Plan 0021, mirror slots and surface-only attributes by Plan 0028; supersedes ADR-0004's mirror layout (Outcome) |
| [0013](0013-mirror-reserves-its-vram-budget-up-front.md) | The mirror reserves its VRAM budget up front and never reallocates | accepted |
| [0014](0014-prefab-identity-in-the-op-log.md) | A stamp op names its prefab by content id, and a save carries the prefabs it names | accepted |
| [0015](0015-late-reverb-as-a-shared-fdn-from-a-listener-ray-fan.md) | Late reverb is one listener-shared feedback delay network driven by a ray fan from the listener; early echoes render from their wall's direction | accepted; level and room-change clauses superseded by ADR-0021 |
| [0016](0016-a-falling-region-crushes-the-sub-threshold-voxels-in-its-path.md) | A falling region crushes the sub-threshold voxels in its path | accepted (Plan 0022); amends ADR-0008's drop clause |
| [0017](0017-mirror-cpu-work-bounded-in-the-frame-loop-narrowing-on-stream-workers.md) | The mirror's CPU work stays in the frame loop, bounded per frame, with narrowing on the stream workers | accepted; worker narrowing not built (Outcome); Alternative B's ring taken up by ADR-0022 |
| [0018](0018-direct-light-per-pixel-one-sample-from-a-hit-buffer-with-svgf-history-killed-by-edits.md) | Direct light is one sample per pixel from a hit buffer, denoised by an SVGF-family filter whose history dies where the world was edited | accepted (Plan 0024) |
| [0019](0019-gamepad-input-as-generic-axes-in-the-shell-fed-by-the-testbed.md) | Gamepad input is generic axes and pad buttons in the shell's `Input`, fed by the testbed's own gilrs | accepted |
| [0020](0020-paired-mixer-bench-bounds-read-with-loops-aligned.md) | A paired mixer-bench bound is read with both builds' loops aligned to 64 bytes | accepted |
| [0021](0021-echo-and-tail-levels-by-room-size-curves-and-a-tail-handed-between-two-networks.md) | Echo and tail levels follow listening-fitted curves of the room's mean free path, and a room change hands the tail to a second network | accepted (Plan 0020) |
| [0022](0022-mirror-uploads-through-a-staging-belt-and-copy-commands.md) | Mirror uploads go through a persistently mapped staging belt and copy commands, not `Queue::write_buffer` | accepted (Plan 0027; Outcome: belt share a miss); supersedes ADR-0017's Alternative B rejection in part |
| [0023](0023-indirect-light-from-a-scrolling-probe-volume.md) | Indirect light comes from a scrolling volume of irradiance probes around the camera, refreshed by a few rays per probe per frame and gathered with visibility weights | accepted (Plan 0029, Outcome) |
| [0024](0024-own-temporal-upscaler-on-the-hit-buffer.md) | The upscaler is a temporal upscaler of our own, driven by jittered rays and the hit buffer, with a sharpening pass | accepted (Plan 0030); superseded in part by ADR-0034; the fly-through's default at 1.5 since Plan 0041 |
| [0025](0025-light-candidates-from-a-power-over-distance-table-of-near-emitter-chunks.md) | Light candidates come from a power-over-distance table of the emitter chunks near the camera, and only those chunks hold light samples on the GPU | accepted (Plan 0024) |
| [0026](0026-atmosphere-from-hillaire-luts-built-on-the-gpu-at-runtime.md) | The atmosphere is Hillaire's LUTs, built on the GPU at runtime from parameters the game sets | proposed (Plan 0032) |
| [0027](0027-the-sun-shadow-is-one-per-pixel-ray-in-the-light-pass.md) | The sun's shadow is one per-pixel ray in the light pass, a dedicated directional sample beside the emissive one | proposed (Plan 0032) |
| [0028](0028-far-field-as-a-clipmap-of-source-generated-coarse-chunks.md) | The far field is a clipmap of coarse chunk levels that the source generates directly, with edits folded in from full detail | proposed (Plan 0033) |
| [0029](0029-liquid-surfaces-as-analytic-wave-normals-with-traced-reflection-and-refraction.md) | A liquid surface is where a translucent liquid meets air, shaded with an analytic wave normal, one traced reflection ray and one bent refraction ray, absorbed by Beer-Lambert | proposed (Plan 0034) |
| [0030](0030-volumetric-cloud-shell-amortised-march-and-projected-shadow-map.md) | Clouds are a ray-marched shell of seeded noise, one pixel in four of a quarter-resolution buffer per frame, rebuilt temporally and shadowing the ground through a projected map | proposed (Plan 0035) |
| [0031](0031-weather-is-cosmetic-client-state-rendered-against-a-fall-map.md) | Weather is cosmetic client state stepped by the game's ticks, and rain, snow, wetness and snow cover are rendered against a fall map traced through the mirror, never written as voxels | proposed (Plan 0036) |
| [0032](0032-night-sky-drawn-analytically-on-miss-pixels-with-a-marched-aurora-shell.md) | The night sky is drawn analytically on miss pixels (a binned star catalogue, sky bodies the game lists, an aurora shell lighting the scene via a hemisphere map), and the night look is a luminance tint | proposed (Plan 0037) |
| [0033](0033-eye-adaptation-as-an-asymmetric-two-stage-exposure-on-the-gpu.md) | Exposure adapts like an eye: fast toward bright, slow toward dark in two stages, metered and stepped on the GPU, applied as pre-exposure | accepted, Outcome (Plan 0040) |
| [0034](0034-fsr-2-ported-to-wgsl-as-the-default-upscaler-candidate.md) | AMD FSR 2.2.1 ported to WGSL is the upscaler's default candidate; our own upscaler stays opt-in | accepted (Plan 0041, Outcome): shipped opt-in, ADR-0024's upscaler at 1.5 the default |
| [0035](0035-dx12-shaders-compile-with-wgpus-bundled-static-dxc.md) | DX12 shaders compile with wgpu's bundled static DXC, never FXC | accepted, Outcome (Plan 0046) |
<!-- roster:end -->

## Decisions taken but not yet recorded

Settled in the founding interview (2026-10-16) and written up in
[`docs/roadmap.md`](../roadmap.md) under "Decisions from the founding interview". Each becomes
an ADR when the plan that implements it is drafted, so the ADR carries the concrete shape
rather than the intention:

- Dynamic entities as voxel models on rigid-part rigs: with the entities plan.
- Network transport: with the networking plan.
