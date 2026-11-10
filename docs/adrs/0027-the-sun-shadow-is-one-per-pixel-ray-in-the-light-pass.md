# ADR-0027: The sun's shadow is one per-pixel ray in the light pass, a dedicated directional sample beside the emissive one

> **Status:** proposed; redrafted 2026-11-10 with Plan 0032's redesign: the directional sample
> and the sky ray are stored in Plan 0058's direct target on its short blend, the probe volume
> and the baked store take no sun or sky term, and a drifting sun raises no change bit
> **Date:** 2026-10-22
> **Related plan(s):** [Plan 0032](../plans/0032-sun-and-sky-hillaire-atmosphere-time-of-day-and-an-outdoor-testbed.md)
> (builds it); Plan 0033 (continues the ray into the far-field clipmap)
> **Builds on:** [ADR-0018](0018-direct-light-per-pixel-one-sample-from-a-hit-buffer-with-svgf-history-killed-by-edits.md)
> (the light pass, its shadow rays and the temporal pass),
> [ADR-0044](0044-direct-light-from-a-fixed-list-of-analytic-lights-one-shadow-ray-each.md)
> (proposed: the direct target the sample is stored in),
> [ADR-0046](0046-static-emissive-light-is-baked-per-chunk-when-a-chunk-loads-or-changes.md)
> (the baked emissive store),
> [ADR-0043](0043-emissive-light-from-a-world-space-per-face-table-and-a-lamp-bounce-probe-channel.md)
> (the probe volume as the lamp's bounce channel)

## Context

The sun and the moon are directional lights that the game moves with the time of day.
Outdoors, the sun dominates every lit surface. The voxel world has caves, overhangs, arches
and tunnels everywhere. Its geometry changes at runtime through edits, and the frame must show
a dug hole letting sunlight into a cave on the next frame.

The light pass already casts one shadow ray per lighting pixel toward the headlamp, and Plan
0058 adds one per analytic light in reach. The walk is the same one the primary ray takes: the
mirror's hierarchy skips empty chunks, empty brick groups and empty 2^3 blocks. A shadow ray
needs no data that the mirror does not already hold. Every edit is in the mirror by the next
frame, and the temporal pass's edit boxes kill history where it changed.

Three places hold light in the frame. Emissive light on a cube face is baked per chunk into a
brick store and read with no screen history (ADR-0046). The probe volume carries the
headlamp's bounce alone and runs only with the lamp on (ADR-0043). The lamp's and the list
lights' direct terms sit in a coloured direct target on a short screen blend, reset where a
light changes (ADR-0044, Plan 0058).

The owner chose the ray family (2026-10-22) and a stable image as the first priority
(2026-11-03). This ADR records three things: that the shadow is a ray and not a map, that the
sun's sample is its own and not an emissive candidate, and where the sun's and the sky's
terms are stored.

## Decision

The light pass takes **one directional sample per lighting pixel**: the sun, or the moon when
the sun is below the horizon and the moon above it. A CPU rule picks the body each frame, the
one with the larger top-of-atmosphere illuminance among those above the horizon. The sample
is a direction drawn uniformly in the body's disc (its angular radius, which the game sets),
and one shadow ray goes along it. Its contribution is the body's illuminance, times the
atmosphere's transmittance at the hit's altitude (ADR-0026's LUT), times `max(n.l, 0)`, times
the ray's visibility. Sampling the disc gives a soft penumbra whose width is the occluder distance times
twice the tangent of the angular radius. The temporal pass removes the noise.

The sample is **dedicated**: it is not a candidate in an emissive sampler and not an entry of
ADR-0044's list. In daylight a pixel casts the lamp's ray, a ray per list light in reach, the
directional ray and the sky ray. The directional ray uses the frame's shadow walk. It runs
until it leaves the near field, which is the residency ball or the mirror window. Inside the
ball, an absent chunk counts as an occluder, so a chunk that has not streamed in yet does not
let the sun in. Once Plan 0033's clipmap exists, the ray continues through it so that far
terrain casts shadows too. Leaving everything counts as reaching the sky.

**Sky light is a cosine-weighted sky ray** at every lighting pixel with a hit, with the same
walk and a length cap. It adds `pi` times the sky's radiance along the ray when the ray
reaches the sky above the horizon. Plan 0032 measures its frame-to-frame change against the
headlamp's before it fixes the number of rays.

**Both terms are stored in the direct target** (ADR-0044), on its blend of `max(ALPHA, 1 /
(age + 1))`. A sun that drifts is followed by the blend and raises nothing. A jump (a turn of
more than the body's angular radius in one frame, a switch of body, a change of illuminance,
the sky set or cleared) sets a sky-change bit of its own for one frame, on which the direct
accumulation takes the frame's sample. The upscalers compare the unshadowed key light of this
frame and of one frame older, beside ADR-0044's comparison of the list. The list's change bit
and the emission-change bit are never raised by the sky.

**The baked store and the probe volume take no sun and no sky term.** A baked face is
byte-equal with the sky on and off. Outdoors the only indirect light is the sky ray; sunlight
does not bounce.

## Consequences

### Positive
- Shadows are exact for any geometry the mirror holds: overhangs, arches, a tunnel's mouth, a
  freshly dug skylight. No second representation of the world has to be built or kept in sync
  with edits.
- Soft shadows come free from the disc sample. The softness follows the real geometry: sharp
  at contact, wider with the occluder's distance.
- Emissive light is untouched by the sun: the store holds it, and a vein in the shade of a
  cave mouth reads the same at noon and at night.
- No new frame target and no second history. The sun follows the rules a dropped light
  follows, so a dug skylight and a turned sun answer within the same eleven frames.
- A running day cycle resets nothing. The image is not clipped or softened as the sun drifts.

### Negative
- It costs one more shadow ray per lit pixel in daylight. A sunward ray outdoors walks until it
  leaves the near field, so its length is the distance to the edge of the world, not to a
  light. The hierarchy makes empty chunks cheap, but the cost is a projection that Plan 0032's
  close must read on both adapters. The lighting line is already tight.
- Daylight adds two rays per pixel, the directional and the sky ray, to the lamp's and the
  list's. A floor tier that cannot afford that
  needs a cheaper walk for the sun (the coarse walk is the knob) or a lower lighting
  resolution, not a different shadow technique.
- Penumbra and sky-ray noise are temporal noise on a short blend. The sky ray is the
  noisiest term, and it may need more than one ray a pixel to hold as still as the lamp.
- No sunlight bounce. A cave mouth a few metres in is dark at noon but for the sky seen
  through the opening. A per-source bounce is a later decision.
- A jump resets the list lights' accumulation for a frame too, since the pair is shared.
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

### Alternative C: The sun as one more candidate in an emissive sampler
Add the sun and moon to a candidate list and let one shadow ray serve all of them. A sampler
weighs candidates by their unshadowed contribution, and in daylight the sun's term dwarfs a
vein's. In the shade the sun wins almost every draw, its ray is occluded, and the emissive
light that should light that shade gets no samples. Since ADR-0046 a cube face draws no
emissive sample at all, so there is no sampler left to join. The decisive reason is that it
starves emissive lights exactly where they are the only light.

### Alternative D: Bake the sun and the sky into the store
Add the directional and sky terms to the per-chunk bake, so they read as steadily as emissive
light. The store is rebuilt at load and at an edit. A sun that moves all day would re-bake
every resident chunk continuously, and the bake's value would depend on the frame. The
decisive reason is that the store is for light that holds still, and the sun does not.

### Alternative E: Feed the sun into the probe volume
Let the lamp's bounce channel shade the sun at its hits and return the sky on a miss, so
sunlit rock lights the shade. The volume is sized to the lamp's reach around the camera and
runs only with the lamp on (ADR-0043), so outdoors most of the view lies outside it, and it
would have to run always and grow. The decisive reason is that it reopens the probe design
the owner narrowed on 2026-11-04, for a bounce no reading has asked for yet.

### Alternative F: A target of their own, with a longer history
Give the directional and sky terms their own accumulation pair with a history longer than the
lamp's. A static sky would then hold stiller for the same ray count. It costs a target pair
on the lighting grid, a second change rule, and a slower answer to an edit or a turned sun.
We do not take it without a reading; it is the named fallback if Plan 0032's stillness
measurement finds four sky rays a pixel are not enough on the short blend.

## Outcome

_(Added only at acceptance if implementation falsified something above. Dated. Never rewrite the
body.)_
