# ADR-0029: A liquid surface is where a translucent liquid meets air, shaded with an analytic wave normal, one traced reflection ray and one bent refraction ray, absorbed by Beer-Lambert

> **Status:** proposed; refreshed 2026-11-10 (architect, review pass) against ADR-0046's baked
> store, ADR-0047's path-length filter, Plan 0058's direct target and Plan 0032's redesigned
> sky light. The probe volume lights nothing here: it is the lamp's bounce channel (ADR-0043)
> **Date:** 2026-10-22
> **Related plan(s):** [Plan 0034](../plans/0034-water-surface-waves-reflection-refraction-underwater-fog.md)

## Context

Glass, water and lava are translucent voxels in the brick map (the roadmap's "Floor, profiles
and scene scale" decision; the owner's answer of 2026-10-22: liquids render first, flow
later). Plan 0015 gave them a class (`MaterialFlags::TRANSLUCENT`, `Material::opacity`) and a
bounded second walk that composites up to `translucent_layers` voxels front to back, each at
a fixed opacity, along the unbent primary ray. That is right for a pane of glass one voxel
thick and wrong for a lake: at 10 cm voxels a 2 m deep lake is 20 voxels, so a per-voxel
alpha either saturates in the first few layers or needs a cap the depth exceeds; the ray does
not bend, nothing reflects, and the brightness does not follow Fresnel. Water is read by its
reflection at grazing angles and its depth colour looking down.

The facts that decide the shape. The renderer is a compute ray marcher with a hit buffer
(ADR-0018): any pixel can trace another ray through the same walk at the cost of a walk,
about 0.9 ms per million rays at 1080p on the dev box (Plan 0015's figure, a projection for
this use). The surface of a
voxel lake is flat at voxel resolution: the top faces of its upper voxel layer. A wave at the
scale people read on water (decimetres to metres) is a few voxels high at most, so the visible
motion is in the normal, not the silhouette. The world must stay unchanged by rendering:
edits are deterministic operations through the op log, and the GPU mirror is uploaded from
them (ADR-0002, ADR-0022), so an animated surface may not be a stream of edits. The game owns
the clock: `vxe-render` takes time as a parameter.

The denoiser filters albedo-demodulated irradiance on the primary surface with depth and
normal edge-stopping (ADR-0018), and the upscaler reprojects by the primary hit's world
position (ADR-0024). A reflection belongs to a different surface than the pixel's, and a
refracted floor point under moving waves does not sit on the straight line through the pixel.
Both need a rule, or the filter blurs the reflected image across its edges and the history
smears the waves.

Since this ADR was drafted the light changed under it. Static emissive light is baked per chunk
into a brick store that a cube-face hit reads (ADR-0046); the bake's shadow test treats a
translucent voxel as air. A translucent solid is a filter by path length, `T(L) = tint^(L / d)`,
with a small reflected share of its entry face's baked light, and the composite returns
`reflected * exposure + T * behind` (ADR-0047, which names this ADR's Beer-Lambert as the same
rule). The lamp and the list lights are exact terms in a `direct` target with a short blend
(ADR-0044, Plan 0058), and Plan 0032 adds the key light and a sky ray to it. The probe volume
is the lamp's bounce channel and takes no sky, sun or emissive light (ADR-0043). FSR 2 is the
default upscaler (ADR-0034).

## Decision

**A material row flagged `MaterialFlags::LIQUID` (always with `TRANSLUCENT`) is a liquid, and
its surface is the first face along a ray where a cell of that liquid meets a cell that is
neither opaque nor the liquid.** The hit pass finds that face in Plan 0015's translucent walk
and records it in a per-pixel liquid record. On a face whose normal is world up, the shading
normal is the analytic normal of a sum of eight Gerstner waves (direction, wavelength,
amplitude, steepness each) evaluated at the face point and the time the game supplies, with
deep-water dispersion and every wave faded out where it is shorter than two pixel footprints;
other faces keep the face normal. The geometry never moves. At the surface the exact
dielectric Fresnel equations (not Schlick, which fails from the dense side) split the ray:
**the transmitted ray bends by Snell's law and is walked through the brick map**, its span
inside the liquid measured and its hit written to the hit buffer, so the floor beneath is lit
by the whole lighting pipeline like any other surface and its history is reprojected by its
apparent point (the unbent ray at the bent path's length); **the reflected ray is walked
through the brick map** in a liquid pass and shaded at its hit: the brick store's value on a
cube face, one stochastic emissive sample only where the hit has no face, the exact direct
terms (the lamp and the key light with a shadow ray each, the list lights unshadowed), one sky
ray, and the sky on a miss. Along every liquid span the light falls by Beer-Lambert,
`exp(-(absorption + scattering) L)` per channel, which is ADR-0047's path-length law with the
coefficients stated by the row, and gains single-scattered ambient light from the filtered
irradiance the pixel's own floor receives. **One law, one composite rule, separate stores:**
the composite returns `glass_reflected * ex + T_glass * (liquid_add + liquid_mul * c_hit)`, and
the liquid's two terms keep `rgba16float` targets of their own, because they are computed after
lighting and their bars need more than rgb9e5's nine bits. The per-frame shadow rays from a hit
under liquid apply the same optical depth; **the bake does not**, so baked emissive light
reaches an underwater face unabsorbed. A camera inside a liquid starts its ray in the span and meets the surface from
below, with total internal reflection beyond the critical angle. A liquid whose one-voxel
transmittance is under 1/255 in every channel is opaque at its surface (lava): the surface
itself is the hit, lit and emissive like rock, and only the reflection is added. The reflected
and refracted radiance bypass the a-trous filter and keep no history of their own; the
upscaler, FSR 2, takes a liquid pixel's surface point in its motion input and a reactive
value; Plan 0034 measures liquid pixels under FSR 2 before it builds that half. A rough
liquid (roughness 0.3 or more) traces no reflection.

## Consequences

### Positive
- Off-screen and behind-the-camera content reflects (a cave ceiling over a pool, the shore
  behind the viewer), because the reflection is a ray in the world, not a lookup in the frame.
- The lake floor gets the baked emissive light, the headlamp, the list lights, the key light's
  shadow and the sky ray through the existing passes; nothing about lighting is duplicated for
  underwater surfaces.
- A reflection of a cube face reads the baked store, so it holds still; and liquids and
  translucent solids share one absorption law and one composite rule.
- Depth colour is physically consistent at any depth and from any angle, from one parameter
  set per material; a deeper lake reads darker without a layer cap.
- No world data changes: the class is a table property, the waves are a render parameter, and
  `vxe-world`'s determinism hashes do not move.
- Lava is the same mechanism with emission and a high absorption, plus one rule for opaque
  liquids; emissive lava becomes light samples for the bake through `exposed_emitters`.

### Negative
- About two more walks per transmitting liquid pixel (refraction in the hit pass, reflection in
  the liquid pass) plus two shadow rays and a sky ray at the reflected hit: a lake filling the screen roughly
  doubles the primary-ray cost of those pixels. The reflection mode (`trace`, `sky`, `off`) is
  the quality knob.
- The silhouette of a wave is flat: at grazing angles the horizon line of a choppy lake is a
  straight voxel edge with a moving normal on it. Waves taller than a voxel are out of reach.
- Light paths through the surface are straight: no caustics, and the sun's light on a lake
  floor is not bent. A reflection ray, a shadow ray or a probe ray that meets a second liquid
  surface treats it as a medium without an interface.
- The reflected hit's sky ray and key-light sample, and the emissive sample on a hit with no
  face, are one-sample and unfiltered; only the upscaler's history smooths them.
- Baked light under water is not absorbed: a pool floor lit only by emissive rock reads
  brighter and less coloured than the same floor under the lamp.
- The in-scatter takes the floor's light for the whole span, so a deep absorbing lake's
  in-scatter reads low near its surface.
- Two more `rgba16float` targets beside ADR-0047's `transmit`, 16 B a pixel, until the factor
  can fold into it.
- Waves on a face only where the face normal is world up, so the engine fixes +z as up for
  liquids, as the cutaway view does.

## Alternatives considered

### Alternative A: Displace the geometry

Either move voxels each frame (an edit stream that rewrites the top layer) or intersect a
height field in a slab above the flat surface inside the walk. The edit stream turns a
cosmetic effect into world mutation through the op log and the upload belt, every frame, for
every visible lake, and breaks "edits are operations"; it also quantises the waves to 10 cm
steps, the Lego look the owner rejects. The slab intersection is honest about silhouettes but
costs a root search per pixel in a cell the walk treats as air, for a height change under a
voxel at our scale. It stays a later quality mode if the owner sees flat horizons.

### Alternative B: Screen-space reflections, or a planar reflection view

Screen-space reflections march the hit buffer: cheap, but blind to everything off screen,
which at a cave pool is most of what reflects (the ceiling over it), and they fail where the
reflected point is occluded in the frame. A planar reflection renders a second view mirrored
about the water plane: exact for one flat lake at one height, but a cave holds pools at many
heights, waves break the single plane, and it doubles the primary cost of the whole frame
rather than the liquid pixels'. We already march rays, so one ray per liquid pixel is the
cheaper exact answer.

### Alternative C: Keep the translucent solid's filter for liquids

Treat water as ADR-0047 treats glass: a path-length filter along the straight ray with a
Schlick share of the entry face's baked light. (When this ADR was drafted the mechanism was
Plan 0015's per-voxel layers, which ADR-0047 replaced.) The absorption law is already right,
but the ray cannot bend, nothing of the world reflects, the tint comes from the albedo instead
of a depth colour, there is no in-scatter, and a camera under the surface sees the same filter
in every direction. It stays the mechanism for non-liquid translucent rows (glass).

### Alternative D: Stochastic rough reflections through the denoiser

Sample a GGX lobe per pixel and let the SVGF filter clean it. That is the general glossy
answer, but the filter's edge-stopping works on the primary surface, which for a lake is one
flat plane, so it would blur the reflected image across its own edges, and a water surface is
nearly a mirror. Rough reflections belong to a glossy-indirect plan for every material, not to
liquids alone.

### Alternative E: Absorb the baked light too

Give the bake's shadow test the liquid's optical depth, so the store's value on an underwater
face is already dimmed and tinted. It is the consistent answer, and the term is static, so it
fits a bake. But the bake treats every translucent voxel as air today, the change re-bakes
every chunk that holds a liquid, and it would put a per-material optics read on the bake's
shadow path for the first cut of a feature the owner has not seen. It stays a followup of Plan
0034; the per-frame shadow rays carry the term now.

### Alternative F: Light the medium with a ray of its own, or with an unshadowed sky term

Trace a sky ray or a key-light shadow ray from the span's midpoint for the in-scatter, or take
`PI * sky_radiance(+z)` without a ray. The ray is one more noisy sample per liquid pixel with
no history to hold it; the unshadowed term lights a cave pool with open sky. The pixel's own
filtered floor irradiance is shadowed, still and free, at the cost of a bias in deep water.
