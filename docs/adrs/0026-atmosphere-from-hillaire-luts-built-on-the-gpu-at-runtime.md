# ADR-0026: The atmosphere is Hillaire's LUTs, built on the GPU at runtime from parameters the game sets

> **Status:** proposed
> **Date:** 2026-10-22
> **Related plan(s):** [Plan 0032](../plans/0032-sun-and-sky-hillaire-atmosphere-time-of-day-and-an-outdoor-testbed.md)
> (builds it); Plan 0035 (adds the aerial-perspective volume and the clouds on top of it);
> Plan 0036 (weather changes the atmosphere's parameters)

## Context

Outdoors is a general engine capability (owner, 2026-10-22). The first consumer lives
underground, but the engine must also show open sky with a dynamic day-night cycle, a sun and
a moon, and later clouds and weather. Every lighting path of the frame needs the sky: a
primary ray that leaves the world shows it; a probe ray of the indirect volume (ADR-0023) that
misses returns its radiance; the sun's direct light at a hit is the light at the top of the
atmosphere times the atmosphere's transmittance along the way. These must agree, or the sky's
colour and the ground's sunlight drift apart at sunset.

Time of day and weather are parameters the game sets each frame (owner, 2026-10-22). The
engine owns no wall clock, and nothing about the sky is deterministic or server-side. So the
model must answer for any sun direction, and later for a changing haze or rain density, with
no offline bake.

The frame's budget is tight. `nfr.md` §2 gives lighting 6.0 ms on the baseline, and Plans
0024 and 0029 already project most of it. The slow adapter on hand (an AMD Cezanne iGPU at
about 2 TFLOPS, `nfr.md` §1) is in the class of the PS4 that Hillaire's paper measured on.
The baseline is above that class.

## Decision

We render the atmosphere with Hillaire's technique ("A Scalable and Production Ready Sky and
Atmosphere Rendering Technique", EGSR 2020). Compute passes in `vxe-render` build small
look-up tables at runtime from an `Atmosphere` parameter block (planet and atmosphere radii;
Rayleigh, Mie and ozone coefficients and profiles; ground albedo) and the sun's and moon's
directions and illuminances:

- a transmittance LUT (256 x 64, `rgba16float`, by altitude and zenith cosine)
- a multiple-scattering LUT (32 x 32, by altitude and light zenith cosine), the paper's
  isotropic infinite-order approximation
- a sky-view LUT (192 x 108, by azimuth and a horizon-weighted elevation, for the camera's
  altitude), in which the sun's and the moon's in-scattering are summed

The transmittance and multiple-scattering LUTs depend on the atmosphere alone. They are
rebuilt whenever its parameters change, which may be every frame once weather drives them.
The sky-view LUT is rebuilt every frame. The sky-view LUT's azimuth is absolute (world
+x east, +y north, +z up), not relative to the sun, so one table holds two light sources. LUT
radiance is per unit of illuminance at the top of the atmosphere, and the game's illuminance,
in the frame's radiance units, scales it. The aerial-perspective volume (32 x 32 x 32 in the
paper) is part of this technique, but Plan 0035 builds it. Within the near field's
reach of about 150 m its optical depth is under 0.005 (Plan 0032's arithmetic), so it matters
only once Plan 0033's far field draws a kilometre. A frame with no `Sky` set builds none of
this and renders exactly as before.

## Consequences

### Positive
- One model answers the primary-ray miss, the probe-ray miss, the sun's transmittance at a
  hit and, later, the aerial perspective and the clouds' lighting. All of them read the same
  tables, so the ground's sunlight and the sky's colour agree at every sun angle.
- Any sun or moon direction and any atmosphere costs one rebuild with no bake. Weather
  (Plan 0036) changes Mie density and the sky follows within a frame.
- The tables are about 0.3 MB in total and are built in a few compute dispatches. Building
  them on the GPU every frame removes the precompute step and the asset.
- Night falls out of the same machinery: the moon is a second light in the sky-view LUT, and
  the multiple-scattering LUT does not depend on which body lights it.

### Negative
- The multiple-scattering term is an approximation. It is isotropic and assumes the same
  in-scattered radiance at every point around a sample, so it is less exact than iterated
  orders of scattering at deep twilight. We accept it because the paper validates it against a
  path tracer for Earth-like atmospheres.
- The sky-view LUT is a per-frame cost that a static sky would not pay, and its 192-texel
  azimuth (about 1.9 degrees per texel) can soften the sun's Mie halo. The sun disc itself is
  drawn analytically, not from the LUT.
- Absolute azimuth gives up the paper's sun-relative layout. That layout puts the
  texel density where the Mie peak is, so a narrow halo may need more azimuth texels, a
  measured change.
- The planet is curved for the atmosphere and flat for the voxel world. The two disagree at
  the horizon by an angle that is negligible inside the near field (at 150 m the planet's
  drop is under 2 mm). At a kilometre it is still under 0.1 m, but Plan 0033 must state it.
- Writing per-frame compute passes means owning the shader code (the paper publishes the
  method, and the reference implementation is Unreal's under its own licence). We write our own
  WGSL from the paper.

## Alternatives considered

### Alternative A: Bruneton and Neyret's precomputed scattering tables
"Precomputed Atmospheric Scattering" (2008) and its 2017 revision precompute 4D in-scattering
tables by iterated orders of scattering. They are exact for any view, but the precompute takes
from many milliseconds to seconds and the tables run to megabytes. They must be rebuilt
whenever the atmosphere changes, so weather-driven haze becomes a stall or a background job
with stale skies. The decisive reason is that weather must change the atmosphere live.

### Alternative B: An analytic sky model (Hosek-Wilkie 2012, Preetham 1999)
A fitted formula gives clear-sky radiance from the ground for a sun above the horizon at a
fraction of a LUT's cost. It has no altitude, no arbitrary atmosphere, no night and no second
light source. Its transmittance is not consistent with an aerial perspective, so the sun's
light on the ground would have to come from a different model than the sky it sits under. The
decisive reason is that it cannot do a sun below the horizon, a moon or weather.

### Alternative C: Ray-march the atmosphere per pixel with no tables
This is exact single scattering at full rate. It costs a march of 30 or more steps with a
nested transmittance march at every miss pixel and at every probe-ray miss, which is
orders of magnitude more than a LUT read, and multiple scattering is still missing. The
decisive reason is the frame budget: outdoors, a large share of pixels miss.

## Outcome

_(Added only at acceptance if implementation falsified something above. Dated. Never rewrite the
body.)_
