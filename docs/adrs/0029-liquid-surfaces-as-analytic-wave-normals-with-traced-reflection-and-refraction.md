# ADR-0029: A liquid surface is where a translucent liquid meets air, shaded with an analytic wave normal, one traced reflection ray and one bent refraction ray, absorbed by Beer-Lambert

> **Status:** proposed
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
(ADR-0018): any pixel can trace another ray through the same walk at the cost of a walk, and
Plan 0029 already traces 262 144 full-walk rays per frame for probes at about 0.25 ms on the
dev box (a projection from Plan 0015's 0.9 ms per million rays at 1080p). The surface of a
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
through the brick map** in a liquid pass and shaded at its hit by the light pass's estimator
plus the probe irradiance, with the sky on a miss. Along every liquid span the light falls by
Beer-Lambert, `exp(-(absorption + scattering) L)` per channel, and gains single-scattered
ambient light from the probe irradiance; shadow rays from a hit under liquid apply the same
optical depth. A camera inside a liquid starts its ray in the span and meets the surface from
below, with total internal reflection beyond the critical angle. A liquid whose one-voxel
transmittance is under 1/255 in every channel is opaque at its surface (lava): the surface
itself is the hit, lit and emissive like rock, and only the reflection is added. The reflected
and refracted radiance bypass the a-trous filter and keep no history of their own; the
upscaler reprojects a liquid pixel by its surface point at a higher blend weight. A rough
liquid (roughness 0.3 or more) traces no reflection.

## Consequences

### Positive
- Off-screen and behind-the-camera content reflects (a cave ceiling over a pool, the shore
  behind the viewer), because the reflection is a ray in the world, not a lookup in the frame.
- The lake floor gets direct emissive light, the headlamp, the sun's shadow and the probes
  through the existing passes; nothing about lighting is duplicated for underwater surfaces.
- Depth colour is physically consistent at any depth and from any angle, from one parameter
  set per material; a deeper lake reads darker without a layer cap.
- No world data changes: the class is a table property, the waves are a render parameter, and
  `vxe-world`'s determinism hashes do not move.
- Lava is the same mechanism with emission and a high absorption, plus one rule for opaque
  liquids; emissive lava becomes light samples through Plan 0024's extraction.

### Negative
- About two more walks per transmitting liquid pixel (refraction in the hit pass, reflection in
  the liquid pass) plus a shadow ray at the reflected hit: a lake filling the screen roughly
  doubles the primary-ray cost of those pixels. The reflection mode (`trace`, `sky`, `off`) is
  the quality knob.
- The silhouette of a wave is flat: at grazing angles the horizon line of a choppy lake is a
  straight voxel edge with a moving normal on it. Waves taller than a voxel are out of reach.
- Light paths through the surface are straight: no caustics, and the sun's light on a lake
  floor is not bent. A reflection ray, a shadow ray or a probe ray that meets a second liquid
  surface treats it as a medium without an interface.
- The reflected hit's one emissive sample is noisy and nothing denoises it but the upscaler's
  clipped history; reflections of emitter-lit rock may shimmer.
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

### Alternative C: Keep Plan 0015's per-voxel compositing for liquids

Treat water as translucent layers at a fixed opacity per voxel along the straight ray. It
costs nothing new, but it cannot bend, reflect or follow Fresnel, its colour depends on the
layer cap instead of the depth, and a camera under the surface sees the same tint in every
direction. It stays the mechanism for non-liquid translucent rows (glass).

### Alternative D: Stochastic rough reflections through the denoiser

Sample a GGX lobe per pixel and let the SVGF filter clean it. That is the general glossy
answer, but the filter's edge-stopping works on the primary surface, which for a lake is one
flat plane, so it would blur the reflected image across its own edges, and a water surface is
nearly a mirror. Rough reflections belong to a glossy-indirect plan for every material, not to
liquids alone.
