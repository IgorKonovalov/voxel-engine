# ADR-0044: Direct light from dynamic sources comes from a fixed list of analytic lights the consumer hands the frame, shaded exactly with one shadow ray each

> **Status:** proposed
> **Date:** 2026-11-04
> **Related plan(s):** [Plan 0058](../plans/0058-dynamic-lights-a-list-of-analytic-lights-dropped-carried-and-bounced.md)
> (builds it). Generalises the headlamp of
> [ADR-0018](0018-direct-light-per-pixel-one-sample-from-a-hit-buffer-with-svgf-history-killed-by-edits.md)
> (the lamp's exact estimate and its reset on a change) and the lamp channel of
> [ADR-0043](0043-emissive-light-from-a-world-space-per-face-table-and-a-lamp-bounce-probe-channel.md)
> (which names "a flag per source into the same channel, later": this is that flag). Leaves
> [ADR-0025](0025-light-candidates-from-a-power-over-distance-table-of-near-emitter-chunks.md)
> (emissive candidates) and the baked store of
> [ADR-0046](0046-static-emissive-light-is-baked-per-chunk-when-a-chunk-loads-or-changes.md)
> as they are: a list light is never an emitter and is never baked.
> **Refreshed 2026-11-10, while proposed:** the Context and Alternatives A and D now argue
> against the baked store, which replaced ADR-0043's per-face table (Plan 0059); the slot rule
> carries the owner's answer of that day. The Decision's mechanism is unchanged.

## Context

The consumer's P0 names three kinds of light: "headlamp, flares, glowing crystal"
(`project-context.md`, Rendering). The engine has the first and the third. The headlamp is one
analytic spot light whose position is derived from the camera each frame (`LightParams`); the
glowing crystal is an emissive voxel, whose light on a cube face is baked per chunk into a
world-space store when the chunk loads or changes (ADR-0046) and sampled through the
near-emitter table (ADR-0025) only at a hit with no face. Nothing in the renderer takes a light that
is not the camera's and not a voxel. A flare thrown into a cavern, a lantern set down on the
floor, a torch carried by an entity (M6) have no path to the pixels.

The owner's request (2026-11-04): "light a cavern by dropping some light source inside, and see
lighting change in real time". The owner's rules on light stand (`LIGHT-FEEDBACK.md`): nothing
changes while nothing moves, no dots in motion, the cost matches what is on screen.

Two facts about the tree decide the shape. **The headlamp's estimate is exact up to its shadow
ray:** no random number is drawn for it, so its screen history exists only to smooth the half
tier's jittered lighting grid, and on the frame the lamp changes the pixel takes the new sample
as it is (`LIGHTING_LAMP_CHANGED`, ADR-0018). The owner found a flying headlamp free of flicker
in every session. **One lamp shadow ray costs about 0.1 ms** on the RTX 3080 Laptop at 720p
internal on the half tier (Plan 0051's `b --indirect off` light pass read 0.113 ms with the lamp
on and every emissive face capped) and about seven to eight times that on the Cezanne iGPU (the
light pass ratio in `docs/nfr/02-frame-budget.md`, Plan 0041's table). So a handful of exact
lights fits the lighting line on the dev reference and is tight on the slow adapter, and a
stochastic pick among lights would reintroduce the noise the store took out of emissive light.

The baked store of ADR-0046 is the wrong place for a light that moves: a record holds a brick's
static emissive light for a direction, filled by thousands of samples from a queue of jobs and
re-baked when a dig changes the chunk. A moving source would put every chunk in its reach back
on the queue every frame, and the store would show the light seconds late, coarse first. A light
that has stopped moving could in principle be baked, but its direct light is exact without the
store, so the bake buys nothing there.

## Decision

**The frame takes a fixed-size list of analytic lights from the consumer every frame, and a lit
pixel shades every light in the list whose reach covers it, exactly, with one shadow ray per
light. The headlamp is slot 0 of that list while it is on.** In detail:

- **A light** is a position, a radiant intensity (radiance on the axis at one voxel, as the
  lamp's `intensity`), a colour, a direction with inner and outer cone cosines (a point light
  has `cos_outer` -1), and flags: `on`, `shadows` (cast the shadow ray), `bounce` (feed the
  probe channel). `MAX_LIGHTS` is 8. The list is a uniform the frame uploads when any field
  changed; the engine keeps no light state between frames beyond that change detection.
  `LightParams` stays as the headlamp's description and fills slot 0 from the camera when
  `Lighting::lamp` is on, so every existing caller is unchanged; the consumer's lights then
  take slots 1 to 7. With the lamp off the consumer's lights take all eight slots (the owner,
  2026-11-10).
- **Shading is exact.** Per lit pixel, for each light within its reach: `E += colour *
  PI * intensity * cone * max(n . l, 0) / d^2 * window(d)`, zero where the shadow ray toward
  the light finds rock, in the frame's shadow walk (coarse or full, as today). No random number,
  no resampling, no reservoir. The sum is the frame's **direct part**, in rgb (today's white
  scalar in the irradiance's `a`, widened).
- **Reach bounds the cost.** A light reaches `r = sqrt(intensity / I_FLOOR)` voxels (`I_FLOOR` a
  `Lighting` setting, default 0.1: the headlamp's 1200 reaches about 110 voxels, 11 m, a guess
  Plan 0058 reads), and its contribution is windowed by `(1 - (d / r)^4)^2` so the reach has no
  visible edge. A floor of 0 means no reach and no window, the lamp's arithmetic before this
  ADR. A pixel beyond a light's reach casts no shadow ray toward it. So the light
  pass's cost is the shadow rays of the lights in reach of what is on screen, not of the list.
- **History as the lamp's.** The direct part blends on the screen history as the lamp's part
  does today (`ALPHA` while the camera moves, longer at rest) and takes this frame's sample as
  it is on a frame where any light in the list changed. A carried light therefore behaves as
  the flying headlamp does, which the owner accepted. The a-trous filters the direct part in
  rgb. The upscalers' change and reactive mask compares the list now against the list of the
  older frame it holds, as it compares the lamp today, so a pixel a changed light reaches is
  marked under FSR 2 and under `own`.
- **Bounce by flag.** A light with `bounce` set is a source of ADR-0043's probe channel: the
  probe rays shade their hits with every flagged light in reach, the probes listed are those
  within the channel's reach (`Lighting::lamp_reach`) of any flagged light, and the running
  mean freezes when every flagged light is still. The channel's volume still scrolls with the
  camera, so a flagged light far from the camera adds no bounce. The headlamp's flag is on by default (ADR-0043's lamp channel unchanged); a dropped
  light's is the consumer's call, off in the testbed by default. Emissive materials gain the same
  flag later, when indirect light from emitters returns.
- **Nothing in `vxe-world` changes.** A list light is a render-side input, not a voxel, not an
  op, not logged; the consumer's server decides where lights are and replicates that as its
  own state. A light that must persist in the world as a thing one can dig around is an
  emissive voxel stamped through an op, as today.

## Consequences

### Positive
- "Drop a light, see the cavern change this frame" is literal: the light is in the list on the
  frame it is dropped, and every pixel in its reach shades it exactly that frame. No settle, no
  dots, no history to wait for.
- A still list is as still as the headlamp at rest: the same exact estimate every frame.
- The cost is a shadow ray per light in reach per lighting pixel, bounded by the reach and read
  per light count; with no light in reach the pass costs what it costs today.
- The headlamp becomes one of eight, so a game can turn it off, light the player another
  way without touching the renderer, and have all eight slots for it.
- The bounce flag is the per-source switch the owner asked for on 2026-10-22, on the channel
  ADR-0043 kept for it.
- Entities (M6) that carry a light get it by writing a slot each frame; nothing else is needed.

### Negative
- **Eight lights.** A ninth is refused (the frame keeps the first eight slots' worth, seven
  beside the headlamp, and counts the rest). A scene with many small dynamic lights needs a stochastic pick, a
  tiled cull, or emissive voxels, none of which this records.
- **Hard shadows.** One shadow ray per light gives a point light's shadow with no penumbra. A
  soft shadow is a stochastic estimate and reintroduces noise and history; it is not offered.
- **The slow adapter.** Eight lights in reach of every pixel project to about 7 ms of shadow
  rays on the Cezanne, over its share of the 6.0 ms lighting line; the reach, the window and the
  consumer's restraint are the only bounds. The portable profile may cap the list at a lower
  count.
- **The direct part widens to rgb.** One more `rgba16float` target on the lighting grid (the
  arithmetic is Plan 0058's) and its accumulation pair, and the temporal, a-trous and composite
  passes change their layout once more, after Plans 0057 and 0059 changed it.
- **The headlamp gains a reach.** At the default floor its light ends at about 110 voxels,
  where it fell off without end; the lamp-lit goldens are re-blessed once.
- **A light the consumer forgets to resend goes out.** The list is per frame by design; a
  consumer that wants persistence keeps the list itself.
- **No light flickers in a way the renderer smooths.** A flame's intensity is the consumer's to
  animate frame by frame, and each change resets the direct part's blend on every pixel the
  light reaches; on the half tier that exposes the lighting grid's jitter for that frame, as the
  flying headlamp does today. If that reads as shimmer in the window, the reset narrows to the
  changed lights' reach, a Plan 0058 reading.

## Alternatives considered

### Alternative A: Every dynamic light is an emissive voxel
Stamp a one-voxel emissive material where the light should be, and let the bake of ADR-0046
do the rest. It needs no new shader path and the store makes it steady once it has landed. It
lost because it cannot move in real time: each step of a carried or thrown light is an edit
that puts every chunk in its reach back on the bake's queue, the faces show the light only when
their job has run, coarse first, and the owner's "see lighting change in real time" is exactly
what the queue cannot give. Its six emitting faces are also sampled stochastically where an
analytic point is exact. It stays the right form for a light that is part of the world (a wall
sconce, a vein).

### Alternative B: A longer list with one resampled light per pixel
Thirty-two or more lights, a per-pixel pick by power over distance with one shadow ray, as the
emissive candidates do. Cheap at any count. It lost because it is stochastic: the picked light
differs frame to frame, and the direct part regains noise, history, dots in motion and the
whole settle problem ADR-0043 and ADR-0046 removed from emissive light. It is the extension if a consumer
ever needs more than eight, and ADR-0025's machinery is where it would start.

### Alternative C: Hardware ray queries for the shadow rays
wgpu 30 exposes experimental ray queries and AABB acceleration structures, and naga's WGSL
accepts the ray-query builtins, so the shadow rays of a longer list could run on RT hardware on
RTX-class cards. It lost here because the floor of `docs/nfr.md` section 1 has no RT hardware
(ADR-0001), so the compute walk stays the required path and the hardware path would be a second
implementation of the same ray; and because ray cost is not what bounds eight lights on the
dev reference. If a later reading says the shadow rays of a dynamic-light scene miss the line on
RTX-class cards, an opt-in tier is its own ADR.

### Alternative D: Bake a still light into the store
Treat a light that has stopped moving as a static source and bake its direct light into
ADR-0046's store, so a still scene costs no shadow rays for it. It lost because the exact
estimate already costs one ray and never flickers, so the bake would trade a known 0.1 ms for
queued jobs over every chunk in reach, a re-bake on every move, a visible hand-over between
the exact light and the baked one, and a second code path for the same light.

## Outcome

_(Added at acceptance, at Plan 0058's close, if implementation falsified something above.)_
