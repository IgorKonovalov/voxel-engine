# ADR-0088: Half-precision shader math comes from one WGSL source, built per adapter as f16 or f32

> **Status:** proposed
> **Date:** 2026-11-27
> **Related plan(s):** [Plan 0110](../plans/0110-the-math-hot-paths-measured-then-cut-shader-math-f16-and-the-integer-root.md)

## Context

The shaders do all their arithmetic in `f32`. `f16` exists only as storage, packed with
`pack2x16float` in the probe buffers (`probes.rs`). WGSL can do `f16` arithmetic behind
`enable f16;`, which wgpu 30 accepts when the device was created with `Features::SHADER_F16`.
On Vulkan that needs `shaderFloat16` plus 16-bit storage access. On DX12 it needs Shader
Model 6.2 half support through DXC, which the Windows arm already links (ADR-0035).

On 2026-11-27, `vulkaninfo` on the dev box reported `shaderFloat16 = true` for all three
adapters the engine is read on: the Radeon RENOIR (RADV), the RTX 3080 Laptop and lavapipe.
The gain differs by vendor. Vega-class Radeons, the slow adapter among them, run packed `f16`
at twice the `f32` rate. Consumer Ampere runs it at the same rate as `f32`, so there the gain
is fewer registers and higher occupancy, not more arithmetic per clock. Nothing is known about
the floor's card (not acquired) or the Portable class (no device on hand). A path that
requires the feature could shut out an adapter the floor otherwise accepts.

Three constraints apply. Nothing in `svet-render` branches on the backend, and any path
specific to one vendor or feature needs an ADR (this one). Owner rule 2026-11-03: a stable
image comes first. `f16` has an 11-bit mantissa and a maximum of 65504, so it can band a
gradient or saturate radiance, and in temporal history it can flicker. Owner rule 2026-11-12:
a mode that costs nothing when off stays a setting.

## Decision

The hot shaders declare their reduced-precision values through one scalar alias, `hf` (with
`vec3<hf>` and so on), and one WGSL source serves both precisions. `frame/compose.rs` builds
each module once per device. When the device has `SHADER_F16` and the render setting
`shader_f16` is on, the module starts with `enable f16;` and `alias hf = f16;`. Otherwise it
gets `alias hf = f32;`. Only one variant is compiled in a run. The app shell (`svet-app`) and
`test_support` ask for `SHADER_F16` whenever the adapter offers it, and the renderer reads
`device.features()`. It never reads the backend or the vendor. `hf` is allowed only for
bounded values: unit vectors, weights and factors in [0, 1], albedo, and colours after the
exposure scale. Positions, distances, ray `t`, depths, world coordinates, accumulation and
history buffers, and exposure luminance stay `f32`. The setting is a render-settings row and
stays there whatever the default becomes.

## Consequences

### Positive
- The slow adapter can get double-rate arithmetic in the passes the readings name as
  arithmetic-limited, with no second copy of any shader to maintain.
- An adapter without the feature runs the same source as `f32`, and the image is the same up to
  rounding. The floor does not change.
- Both variants are testable on lavapipe, which reports `shaderFloat16`. Turning the setting off
  forces `f32` on any device.

### Negative
- Every pipeline has two possible compiled forms. A golden image blessed under one precision is
  read under the other within a tolerance, not bit for bit.
- The `hf` rule (bounded values only) is a review rule, not a type check. A position typed `hf`
  compiles and bands at a distance.
- Asking for `SHADER_F16` changes the device request of every consumer that uses the shell. A
  consumer that builds its own device has to ask for it to get the gain.
- On Ampere-class cards the gain may be zero or negative (conversion instructions). The keep
  bar of Plan 0110 decides per pass, and a pass that loses stays `f32` on every adapter, since
  the variant is chosen by the feature and never by the vendor.

## Alternatives considered

### Alternative A: Require `SHADER_F16` (f16 becomes part of the floor)

One variant and the simplest code. Every adapter on hand has it, and so do the GTX 1660 and
RX 5600 class on Vulkan. It lost because the Portable class is unknown, DX12 half support
depends on the driver's shader model, and the gain on Ampere does not pay for refusing even
one adapter.

### Alternative B: f16 as storage only

Pack intermediates as `pack2x16float` or `rgba16float` and keep the arithmetic in `f32`. This
needs no feature, and the engine already does it in the probes. It lost as the mechanism for
this decision because it only saves bandwidth. Plan 0110 already measures, per pass, whether a
pass is limited by arithmetic or by memory. Storage packing for a memory-limited pass is not
ruled out, and is a backlog item for that plan's readings.

### Alternative C: Two hand-written shader files per hot pass

Explicit, with no alias. It lost because every later edit to a lighting pass would have to be
made twice, and the two copies would drift.

## Outcome

_(Added only at acceptance if implementation falsified something above.)_
