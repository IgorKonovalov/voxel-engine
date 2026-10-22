# ADR-0027: The sun's shadow is one per-pixel ray in the light pass, a dedicated directional sample beside the emissive one

> **Status:** proposed
> **Date:** 2026-10-22
> **Related plan(s):** [Plan 0032](../plans/0032-sun-and-sky-hillaire-atmosphere-time-of-day-and-an-outdoor-testbed.md)
> (builds it); Plan 0033 (continues the ray into the far-field clipmap)
> **Builds on:** [ADR-0018](0018-direct-light-per-pixel-one-sample-from-a-hit-buffer-with-svgf-history-killed-by-edits.md)
> (the light pass, its shadow rays and the temporal pass),
> [ADR-0023](0023-indirect-light-from-a-scrolling-probe-volume.md) (probe rays shade their hits
> with the light pass's estimator)

## Context

The sun and the moon are directional lights that the game moves with the time of day.
Outdoors, the sun dominates every lit surface. The voxel world has caves, overhangs, arches
and tunnels everywhere. Its geometry changes at runtime through edits, and the frame must show
a dug hole letting sunlight into a cave on the next frame.

After Plan 0024, the light pass already casts one shadow ray per lighting pixel toward the
headlamp, and one toward the emissive sample chosen by resampled importance sampling (RIS)
over candidates from the near-emitter table (ADR-0025). The walk is the same one the primary ray takes: the
mirror's hierarchy skips empty chunks, empty brick groups and empty 2^3 blocks. A shadow ray
needs no data that the mirror does not already hold. Every edit is in the mirror by the next
frame, and the temporal pass's edit boxes kill history where it changed.

The owner chose this family (2026-10-22). This ADR records two things: that the shadow is a
ray and not a map, and how the sun's sample relates to the emissive sampler. The second has a
real alternative.

## Decision

The light pass takes **one directional sample per lighting pixel**: the sun, or the moon when
the sun is below the horizon and the moon above it. A CPU rule picks the body each frame, the
one with the larger top-of-atmosphere illuminance among those above the horizon. The sample
is a direction drawn uniformly in the body's disc (its angular radius, which the game sets),
and one shadow ray goes along it. Its contribution is the body's illuminance, times the
atmosphere's transmittance at the hit's altitude (ADR-0026's LUT), times `max(n.l, 0)`, times
the ray's visibility. Sampling the disc gives a soft penumbra whose width is the occluder distance times
twice the tangent of the angular radius. The temporal pass removes the noise.

The sample is **dedicated, not a candidate in the emissive RIS**. The emissive sampler is
unchanged, and in daylight a pixel casts up to three shadow rays: the lamp, the emissive
sample and the directional sample. The directional ray uses the frame's shadow walk. It runs
until it leaves the near field, which is the residency ball or the mirror window. Inside the
ball, an absent chunk counts as an occluder, so a chunk that has not streamed in yet does not
let the sun in. Once Plan 0033's clipmap exists, the ray continues through it so that far
terrain casts shadows too. Leaving everything counts as reaching the sky.

The directional part of the irradiance joins the emissive part's long history (ADR-0018's
temporal pass, Plan 0024 Phase 13). When the body's direction drifts by more than its angular
radius since the last flag, the frame raises the same "static light changed" bit that an
emission-scale change raises, and the blend stays at or above 0.2 for the following frames.

## Consequences

### Positive
- Shadows are exact for any geometry the mirror holds: overhangs, arches, a tunnel's mouth, a
  freshly dug skylight. No second representation of the world has to be built or kept in sync
  with edits.
- Soft shadows come free from the disc sample. The softness follows the real geometry: sharp
  at contact, wider with the occluder's distance.
- Emissive lights keep their share of the sampler in a sunlit scene. A pixel in the shade of a
  cave mouth still gets its RIS draw at the vein beside it.
- The probe volume's rays get sunlight bounce from the same function, so sunlit rock lights a
  cave's entrance zone with no extra mechanism.

### Negative
- It costs one more shadow ray per lit pixel in daylight. A sunward ray outdoors walks until it
  leaves the near field, so its length is the distance to the edge of the world, not to a
  light. The hierarchy makes empty chunks cheap, but the cost is a projection that Plan 0032's
  close must read on both adapters. The lighting line is already tight.
- Daylight puts the frame at three shadow rays per pixel. A floor tier that cannot afford that
  needs a cheaper walk for the sun (the coarse walk is the knob) or a lower lighting
  resolution, not a different shadow technique.
- Penumbra noise is temporal noise. A sun that moves (a running day cycle, or a time key that
  jumps) shortens history, and the shadow edge is noisier while it moves.
- Until Plan 0033, terrain beyond the residency ball casts no shadow, so a low sun lights a
  valley that a distant ridge should shade.
- An absent chunk counted as an occluder darkens a newly streamed edge for a frame or two
  until it arrives. We prefer a short dark flicker to sunlight leaking into a cave.

## Alternatives considered

### Alternative A: A top-down shadow heightmap
Store, per world column, the highest solid voxel. This is cheap to build and to sample, and
for a sun near the zenith over open terrain it is nearly right. The engine's premise is caves,
overhangs and arches, though, and a heightmap knows only the topmost surface. It cannot light
the floor under an arch from a low sun, it shades a sunlit ledge below an overhang wrongly,
and every edit must update the column. The decisive reason is that it is wrong for the
geometry this engine exists to draw.

### Alternative B: Cascaded shadow maps
Render depth from the sun into several cascades and compare at each hit. This is the
rasterizer's standard. The renderer, though, has no rasterization path for the brick map, so
each cascade would be a ray-marched depth image re-rendered whenever the sun or an edit moves.
At 2048 x 2048 per cascade, four cascades are about 16.8 million rays, more than the primary
rays of a 1080p frame several times over. It also brings back shadow-map bias, acne,
peter-panning and cascade seams. The decisive reason is that the per-pixel ray is both
cheaper and exact here, because the march already exists.

### Alternative C: The sun as one more candidate in the emissive RIS
Add the sun and moon to the candidate list and let one shadow ray serve all of them. This
keeps one shadow ray per pixel. RIS weighs candidates by their unshadowed contribution,
however, and in daylight the sun's term dwarfs a vein's. In the shade (a cave mouth, under a
ledge) the sun wins almost every draw, its ray is occluded, and the emissive light that should
light that shade gets no samples. Reservoir reuse helps only where a neighbour already found
the vein. The decisive reason is that it starves emissive lights exactly where they are the
only light, and that is the first consumer's scene.

## Outcome

_(Added only at acceptance if implementation falsified something above. Dated. Never rewrite the
body.)_
