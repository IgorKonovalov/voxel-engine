# ADR-0032: The night sky is drawn analytically on miss pixels: a binned star catalogue, sky bodies the game lists, and an aurora shell that lights the scene through a hemisphere map

> **Status:** proposed
> **Date:** 2026-10-22
> **Related plan(s):** [Plan 0037](../plans/0037-night-sky-stars-moon-phases-sky-bodies-and-aurora.md)
> (builds it)
> **Builds on:** [ADR-0026](0026-atmosphere-from-hillaire-luts-built-on-the-gpu-at-runtime.md)
> (the transmittance and sky-view LUTs the night terms sit behind),
> [ADR-0030](0030-volumetric-cloud-shell-amortised-march-and-projected-shadow-map.md) (the
> cloud buffer that covers them, its update modes and its reconstruction),
> [ADR-0023](0023-indirect-light-from-a-scrolling-probe-volume.md) and
> [ADR-0027](0027-the-sun-shadow-is-one-per-pixel-ray-in-the-light-pass.md) (the lighting
> rays whose misses read the sky)

## Context

After Plans 0032 to 0036 the sky has an atmosphere, a sun and a moon disc, clouds and weather,
and nothing else. At night a miss pixel shows the moonlit sky-view LUT or black. The owner
asked (2026-10-22) for stars, a comet and northern lights. The engine stays game-agnostic: it
offers mechanisms (a star field, a list of bodies in the sky, an aurora layer) and the game
decides what each one means and when it shows. The engine reads no clock: anything that moves
does so by a time or a direction the game sets.

The facts that decide the shape. A star is a point: the brightest subtends milliarcseconds,
while a pixel of a 1080p frame with a 90 degree field is about 0.07 degrees wide. What a star
contributes to a pixel is its flux, not a radiance to sample. The frame renders at an
internal resolution with a sub-pixel jitter per frame, and Plan 0030's upscaler takes miss
pixels with no history (alpha 1.0), so a point sampled at the jittered ray's direction
flickers on and off from frame to frame. Northern lights are emissive curtains about 1 km
thick, hundreds of kilometres long, from about 100 km up to 300 km of altitude, with vertical
ray striations. Overhead, those rays converge in perspective into the corona, and at the
horizon the curtains stack into bands. Their look comes from three-dimensional geometry seen
from one point. Their light is dim but real: a bright display lights snow at a moonless
night. Sky bodies (a comet, a second moon, a planet, a meteor) sit at infinity for the
camera. They are directions with an angular size, never positions in the world.

The budget: `nfr.md` §2 gives volumetrics a share of the 2.1 ms headroom line, which Plan
0035's clouds already draw on, and the composite sits in the 1.5 ms post line. At night, most
of an outdoor frame can be sky.

## Decision

**We draw the night sky in `vxe-render` on primary-miss pixels, behind the atmosphere's
transmittance and under the clouds, from three analytic sources.**

1. **Stars are a catalogue, binned on a cube and splatted by flux.** A `StarCatalogue` is a
   list of stars (a celestial direction, a magnitude, a colour). Either the CPU generates it
   from a seed with a stated magnitude distribution, or the game supplies it (a real
   catalogue, or its own). It is uploaded once, sorted into the cells of a 6 x 32 x 32 cube
   grid. The composite tests each miss pixel against the stars of its cell and the eight
   neighbours. It adds each star's illuminance, times the fraction of a small Gaussian
   point-spread function (sigma in internal pixels) that falls in the pixel's box, divided by
   the pixel's solid angle. Every pixel shares the frame's jitter, so the boxes tile the
   screen, and a star's total flux is the same at every jitter.
2. **Sky bodies are a list the game sets every frame** of at most 64 entries: direction,
   angular radius, illuminance, a look (flat, or lit by the sun as a sphere with
   Lommel-Seeliger shading, which also gives the moon its phases) and an optional texture. An
   optional tail is an analytic fan along an axis, either away from the sun or along a
   direction the game gives, so the same primitive draws a comet's tail and a meteor's trail.
   Opaque looks hide the stars behind them. Bodies cast no light and add nothing to the LUTs.
3. **The aurora is an emissive spherical shell** (by default 90 to 320 km of altitude). Its
   emission is a product of a horizontal curtain field and a vertical profile with a colour
   per altitude band. The curtain field is the distance to warped, game-placed curtain lines,
   with striations along each line, animated by the game's time. It is marched per pixel of a
   quarter-resolution buffer (half the internal size on each axis), on sky pixels only, and
   each step integrates the curtain's Gaussian cross-section analytically along its length,
   so a curtain thinner than a step is not missed. The march is skipped where the clouds are
   already opaque. It takes Plan 0035's three update modes and its reconstruction, with a
   reprojection by direction. Its light reaches the scene through a separate 128 x 64
   upper-hemisphere `night_light` map, rebuilt every frame from the same march with the
   airglow background and the stars' mean radiance. The lighting rays' sky lookup adds that
   map (Plan 0032's probe misses and sky ray, and Plan 0035's `cloud_sky` background). The
   sky-view LUT stays the atmosphere alone, because primary misses read it and would show the
   aurora twice.

An optional sky texture (a cube map in the celestial frame, for a painted Milky Way or
nebulae) adds behind the stars. Everything here is off by default, and a frame with no night
sky set is byte for byte the frame before.

## Consequences

### Positive
- A star's brightness is its flux whatever the resolution, the field of view or the jitter,
  so stars neither flicker with the jitter nor change brightness with the internal scale. The
  magnitude distribution is exact and testable on the CPU, because the catalogue is a list.
- A game can hand in a real catalogue and get real constellations, rotated by the time of day
  and the latitude through the same geometry as Plan 0032's `body_direction`.
- One body primitive covers a comet, a meteor, a second moon, a planet with phases and the
  moon's own phases. The engine names none of them.
- The aurora's curtains, folds, rays and overhead corona come from geometry, so they look
  right from the ground, from a mountain and in any direction. Its glow lights the ground
  through the paths the sky light already uses.
- The night terms sit in front of the clouds and behind the atmosphere in one place in the
  composite, under Plan 0035's media rule. Clouds hide the stars and the aurora, and the
  horizon dims and reddens them, with no special case.

### Negative
- Every miss pixel at night tests about 13 stars (1.5 per cell on average over 9 cells at the
  default 9000) and the active bodies. That is a cost in the composite for as long as stars
  are on. The pass skips the loop when the frame's exposure makes the brightest star
  invisible, so daylight pays a branch, not the loop.
- The point-spread function is a look choice: a sigma under about half an internal pixel
  aliases, and one over a pixel blurs. The upscaler's Lanczos kernel with no history on misses
  can still shimmer a star a little at scale 1.5. The plan measures it.
- The aurora is a stylised emission model, not a simulation of particle precipitation. Its
  parameters (curtain count, fold scale, colours) are art direction that the game owns.
- The aurora's light reaches the probes through their hysteresis and `cloud_sky` through its
  one-eighth refresh, so a fast pulse lights the ground late and smoothed. The aurora's glow
  scattered in the lower atmosphere (a halo around a bright display) is not modelled.
- Bodies are always at infinity and drawn on misses only. A body cannot pass in front of the
  terrain, cast light or shadow, eclipse the sun or show motion blur. A meteor's speed reads
  through its trail, which the game sets.
- A catalogue costs memory per star (32 bytes; 288 KB at 9000 stars), and the cube binning
  caps the point-spread function's reach at one cell (about 1.6 degrees at a cube corner).
  That suits any field of view a frame uses.
- About a dozen new public types and many parameters are surface to keep tested.

## Alternatives considered

### Alternative A: A painted skybox cube map only
Stars, the Milky Way and an aurora painted or baked into a cube map, sampled by direction. It
is the cheapest to draw and the most art-directable. A star in a texture is a texel, though.
To keep one texel under one output pixel at 1080p and a 90 degree field, each face needs
about 1920 texels on a side, which is 88 MB in `rgb9e5` and still magnifies under a narrower
field. Bilinear sampling smears a star over four pixels at some jitters and not at others,
which is the flicker the flux splat avoids. A texture also cannot twinkle per star, cannot
rotate an aurora's curtains in depth or animate them, and cannot occlude stars behind a
body. We keep it as the optional background layer, which is what it is good at. The decisive
reason is that a point source is not a texture.

### Alternative B: The aurora as a 2D layer (a textured sheet or a sky-view LUT term)
Draw the aurora as an animated texture on a sphere at about 110 km, or as emission added into
ADR-0026's sky-view LUT. Both are cheap: one texture read per pixel, or none. A sheet has no
vertical extent, though, so it cannot show the curtains' height, their folds seen edge-on, or
the corona where the rays converge overhead, and it parallaxes wrongly when the camera
climbs. The sky-view LUT's texel is about 1.9 degrees of azimuth (ADR-0026), and a curtain
1 km thick at 150 km subtends 0.4 degrees, so the LUT holds a blur, and primary misses would
show it under the marched curtain. The decisive reason is that the look the owner asked for
is the three-dimensional curtain. The LUT-resolution blur is exactly what lighting needs,
which is why the separate `night_light` map exists.

### Alternative C: Sky bodies as voxel models placed far away
Stamp or render a comet or a second moon as a voxel entity at a large distance, so it gets
the renderer's existing shading. Anything in the world is inside the near field or the far
field's kilometre (ADR-0028). A body there shows parallax as the camera moves, gets aerial
perspective and fog as terrain, can be hit by lighting rays, and occludes the sun's shadow
ray. A moon 1 km away that slides as the player walks is wrong in every frame. Entities are
also not built yet (roadmap, "Dynamic entities"). The decisive reason is that a sky body is a
direction, not a position.

### Alternative D: Stars hashed per pixel with no catalogue
For each cell of a direction grid, hash the seed to decide whether a star lives there and how
bright it is. This needs no buffer and has no star cap. A game cannot supply a real
catalogue, though, and the magnitude distribution is only a probability per cell, so the
count of stars brighter than a magnitude is not a stated number a test can pin. The decisive
reason is that a list serves both the seeded field and a supplied one with the same shader,
and makes the distribution exact.

## Outcome

_(Added only at acceptance if implementation falsified something above. Dated. Never rewrite the
body.)_
