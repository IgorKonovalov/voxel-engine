# ADR-0032: The night sky is drawn analytically on miss pixels (a binned star catalogue, sky bodies the game lists, an aurora shell lighting the scene via a hemisphere map), and the night look is a luminance tint

> **Status:** proposed; redrafted 2026-11-10 (architect, review pass) against the tree after
> Plan 0059: the night's light reaches the ground through Plan 0032's sky ray alone, into
> Plan 0058's direct target; no probe and no baked store takes it (Alternative G)
> **Date:** 2026-10-22
> **Related plan(s):** [Plan 0037](../plans/0037-night-sky-stars-moon-phases-sky-bodies-and-aurora.md)
> (builds it)
> **Builds on:** [ADR-0026](0026-atmosphere-from-hillaire-luts-built-on-the-gpu-at-runtime.md)
> (the transmittance and sky-view LUTs the night terms sit behind),
> [ADR-0030](0030-volumetric-cloud-shell-amortised-march-and-projected-shadow-map.md) (the
> cloud buffer that covers them, its update modes and its reconstruction),
> [ADR-0027](0027-the-sun-shadow-is-one-per-pixel-ray-in-the-light-pass.md) (the sky ray
> whose reach of the sky reads the night's light),
> [ADR-0044](0044-direct-light-from-a-fixed-list-of-analytic-lights-one-shadow-ray-each.md)
> (the direct target that ray's light lands in),
> [ADR-0046](0046-static-emissive-light-is-baked-per-chunk-when-a-chunk-loads-or-changes.md)
> (the baked store, emissive only)

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
internal resolution with a sub-pixel jitter per frame. FSR 2 at 1.5, the default since Plan
0055, accumulates miss pixels reprojected by direction; a point sampled at the jittered ray's
direction flickers on and off from frame to frame. Northern lights are emissive curtains about 1 km
thick, hundreds of kilometres long, from about 100 km up to 300 km of altitude, with vertical
ray striations. Overhead, those rays converge in perspective into the corona, and at the
horizon the curtains stack into bands. Their look comes from three-dimensional geometry seen
from one point. Their light is dim but real: a bright display lights snow at a moonless
night. Sky bodies (a comet, a second moon, a planet, a meteor) sit at infinity for the
camera. They are directions with an angular size, never positions in the world.

The budget: `nfr.md` section 2 gives volumetrics a share of the headroom line, which Plan
0035 allots across the outdoor plans, and the composite sits in the post line, which is
already over under the FSR 2 default. At night, most of an outdoor frame can be sky.

The light on the ground has one path. Under ADR-0043 the probe volume is the headlamp's bounce
channel and reads no sky; under ADR-0046 the baked store is emissive only; under ADR-0027 as
redrafted, sky light is a cosine-weighted sky ray per lighting pixel into Plan 0058's direct
target, on that target's short blend. The owner's priority (2026-11-03) is an image with no
flicker.

The night's look is a separate question from its contents. At physical brightness under
Plan 0032's exposure, a moonlit landscape is a dim copy of the day in the same colours; the
eye sees a night scene bluer and greyer as rods take over (the Purkinje shift), and film and
games draw night that way. The owner chose that look (2026-10-22).

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
   upper-hemisphere `night_light` map with a mip chain, rebuilt every frame from the same
   march with the airglow background and the stars' mean radiance. The sky ray adds that map
   to the sky's radiance where it reaches the sky, as do Plan 0034's secondary-ray misses and
   the background of Plan 0035's `cloud_sky` map, and the result lands in the direct target.
   Which mip the sky ray reads is set by a stillness reading against the headlamp's, not
   here: a bright band read sharply by one ray a pixel is noisy. A night light that changes
   from frame to frame raises no lighting change bit; it adds a now-and-then pair to the
   upscalers' comparison. The sky-view LUT stays the atmosphere alone, because primary misses
   read it and would show the aurora twice.

An optional sky texture (a cube map in the celestial frame, for a painted Milky Way or
nebulae) adds behind the stars.

4. **The night look is a tint driven by scene luminance** (the owner's choice, 2026-10-22: a
   blue-shifted, desaturated "movie night", a scotopic or Purkinje-style shift, as a post
   effect). A `NightTint { strength, hue, full_below, none_above }` the game sets applies in
   the composite to every pixel, after the exposure metering reads the frame and before the
   exposure multiply and the tone map. From the pixel's pre-exposure luminance `Y` (frame
   units) a weight runs from 0 above `none_above` to `strength` below `full_below`, a
   smoothstep in `log10 Y` between, and the pixel's colour moves by that weight toward `Y`
   times the game's hue normalised to unit luminance. Luminance is kept exactly, so only
   chromaticity moves: saturation falls and the hue goes toward blue as the scene darkens,
   which is the look of rod vision (Thompson, Shirley and Ferwerda, "A Spatial
   Post-Processing Algorithm for Images of Night Scenes", 2002). It does not reweight
   brightness by the scotopic luminosity function (CIE 1951 `V'(lambda)`, peak near 507 nm),
   which would move the metering and the stars' calibrated flux. Its bounds default to the
   mesopic band, about 0.001 to 3 cd/m2 (CIE 191:2010, "Recommended System for Mesopic
   Photometry Based on Visual Performance"), in the game's units. It is independent of the
   night sky: a dark cave can use it.

Everything here is off by default, and a frame with no night sky and no tint set is byte for
byte the frame before.

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
  through the one path the sky light already uses, the sky ray, at every distance.
- The night terms sit in front of the clouds and behind the atmosphere in one place in the
  composite, under Plan 0035's media rule. Clouds hide the stars and the aurora, and the
  horizon dims and reddens them, with no special case.

### Negative
- Every miss pixel at night tests about 13 stars (1.5 per cell on average over 9 cells at the
  default 9000) and the active bodies. That is a cost in the composite for as long as stars
  are on. The pass skips the loop when the frame's exposure makes the brightest star
  invisible, so daylight pays a branch, not the loop.
- The point-spread function is a look choice: a sigma under about half an internal pixel
  aliases, and one over a pixel blurs. FSR 2's lock and sharpen passes can brighten or ring a
  one-pixel peak. The plan measures a still star through FSR 2 right after the splat exists,
  with a stop rule, before anything else is built on it.
- The aurora is a stylised emission model, not a simulation of particle precipitation. Its
  parameters (curtain count, fold scale, colours) are art direction that the game owns.
- The aurora's light follows the direct target's blend (90 % of a step within eleven frames)
  and `cloud_sky`'s one-eighth refresh, so a fast pulse lights the ground a little late and
  smoothed. If the sky ray must read a coarse mip to hold still, the ground loses the aurora's
  directional shadowing. The night's light has no bounce and does not reach under an
  overhang the sky ray cannot see out of. The aurora's glow scattered in the lower atmosphere
  (a halo around a bright display) is not modelled.
- Bodies are always at infinity and drawn on misses only. A body cannot pass in front of the
  terrain, cast light or shadow, eclipse the sun or show motion blur. A meteor's speed reads
  through its trail, which the game sets.
- A catalogue costs memory per star (32 bytes; 288 KB at 9000 stars), and the cube binning
  caps the point-spread function's reach at one cell (about 1.6 degrees at a cube corner).
  That suits any field of view a frame uses.
- About a dozen new public types and many parameters are surface to keep tested.
- The tint's weight depends on each pixel's own luminance, so luminance noise in the ramp
  between its bounds becomes chromaticity noise. The plan bounds it against the tint off on a
  dim lamp-lit wall, with a filtered luminance as the named fallback.
- The tint is art direction, not physiology. Keeping luminance means a night-blue scene is
  no darker than the physical one, so darkness still comes from the exposure's `max` and
  the game's light levels. Its bounds are in frame units and so tied to the game's unit
  convention; a game that rescales its sun moves them with it. A chromaticity shift is a
  look some games will not want, which is why it is a separate, optional setting.

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

### Alternative E: Pure physical darkness, tuned through exposure alone
Leave the night physically dark and let the game set Plan 0032's exposure `max`, the
stars' gain and the airglow until it reads. It adds nothing to the engine, and a game that
wants a neutral night still gets it (the tint at `None`). It is not the look the owner
wants: a physically exposed moonlit scene is a dim daylight scene with the same colours, and
the eye's and the film convention's night is bluer and greyer. Exposure tuning can only make
it brighter or darker. The decisive reason is that the owner judged the colour, not the
level, to be what is missing. The tuning stays, under the tint.

### Alternative F: A night-adapted exposure target (Krawczyk et al. 2005)
Let auto exposure's key fall with the scene's mean luminance (Krawczyk, Myszkowski and
Seidel, "Lightness Perception in Tone Reproduction for High Dynamic Range Images", 2005), so
the night stays darker than the day without a hard `max` clamp. It addresses how dark the
night reads, not its colour, and it changes every auto-exposure frame of every scene, day
and cave included, so its testing surface is far larger than a post effect's. It is not
taken now and not ruled out: it composes with the tint (the tint reads pre-exposure
luminance, so a different exposure leaves it unchanged) and is a possible later addition
with its own plan and ADR if the owner's night check finds the tint alone leaves the night
too bright.

### Alternative G: The night's light through the probe volume or the baked store
Feed the aurora and the airglow into the lamp's probe channel as one more source, or bake a
sky term into the per-chunk store, so the ground's night light is filtered in world space and
holds still for free. The probe channel is sized to the headlamp's reach around the camera and
runs only with the lamp on (ADR-0043), so it would light the ground near the player and
nothing beyond, and not at all with the lamp off. The store is baked at load and at edit
(ADR-0046); an aurora that moves every frame would re-bake the world or go stale, and Plan
0032 keeps every sky term out of the bake so a baked face does not depend on the sky. The
decisive reason is that Plan 0032 already gives sky light one path at every distance, the sky
ray, and a second path for the night alone would be a second thing to keep still. If the sky
ray's reading fails even at a coarse mip, the fallback is ADR-0027's Alternative F, a longer
history for the sky ray's target, not this.

## Outcome

_(Added only at acceptance if implementation falsified something above. Dated. Never rewrite the
body.)_
