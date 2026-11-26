# ADR-0063: Surface detail is a procedural shell marched at the hit, described per material on the render side, cosmetic and client-only

> **Status:** proposed
> **Re-read 2026-11-17 (architect, Plan 0081's re-read):** the style's index does not ride the
> material row's spare lane (the rows are the mirror's, not the frame's); it lives in the
> `Frame`-owned per-material style word Plan 0086 Phase 2 introduces with `SHADOW_AS_BRICK`,
> one word and one setter for both. The baked light under an element is read in the temporal
> pass for the face voxel, not the element's position. Plan 0081's amendment of that date
> carries them; the body stands.
> **Revised 2026-11-26 (architect, Plan 0081's second re-read; still proposed):** three points
> of the Decision read otherwise, after ADR-0082 (no screen history in the light passes) and
> Plan 0086 as built. (1) **A detail hit takes its face's light**: the sky visibility, the key
> light's visibility and the emissive record are read for the face under the element, from the
> sources that face would take; the element's normal enters the cosine and the root term only.
> "The sun's shadow ray starts at the element" is withdrawn as the default and kept as a
> setting's arm (a ray from the rest position); a list light's ray starts at the rest position.
> Rejected as the default because a per-frame ray from a thin, swaying element needs the
> history ADR-0082 removed, or shows its noise. (2) The style index is bits 2 to 7 of the
> style word, which is an `f32` lane exact to 24 bits, so at most 63 styles. (3) The fade's
> candidates are `[6, 12]` and the far normal's band `[4, 8]` pixels a voxel (ADR-0042's band
> as ADR-0082 set it), so no element is drawn where the shading normal is the far one; the
> owner confirms both at Plan 0081's questions 1 and 2.
> **Date:** 2026-11-16
> **Related plan(s):** [Plan 0081](../plans/0081-a-surface-detail-layer-blades-caps-and-pebbles-marched-above-styled-faces-swayed-by-wind.md)
> (builds it); [Plan 0032](../plans/done/0032-sun-and-sky-hillaire-atmosphere-time-of-day-and-an-outdoor-testbed.md)
> (the outdoor source whose `grass` row is its first user);
> [Plan 0036](../plans/0036-weather-rain-snow-wind-wet-surfaces-snow-cover-lightning.md)
> (the wind that will drive its sway)

## Context

The owner asked for grass outdoors (2026-11-16) and, in the same breath, for the mechanism to
be general: "maybe this new layer can be used for something else as well". At 10 cm a voxel
cannot be a blade. Grass built of voxels is a lumpy carpet: a blade is a centimetre wide and
a few tens of centimetres tall, so a voxel field reads as green rubble, and the honest-cubes
default (ADR-0041) makes every voxel's edges visible at arm's length. The same holds for moss,
cave fungus, flowers, pebbles and snow tufts: fine, numerous, sub-voxel things that stand on a
surface. The first consumer is an underground horror game (`project-context.md`), so moss on
a wall, a glowing cap in a crevice and rubble on a floor matter more to it than grass does;
grass is the first user because Plan 0032's outdoor source has a `grass` row and a sun.

Four facts of the tree shape the choice.

- **The world is deterministic and the renderer is a mirror.** `svet-world` is headless, does
  integer edit math, and every edit is a logged op (CLAUDE.md). A blade that existed as world
  data would be collided with, queried, logged, synced and baked. Nothing about a blade needs
  any of that: it is cosmetic. Plan 0055 removed ADR-0011's per-material procedural detail
  from the world's `Material` row and the shaders because it read as a blur and saved nothing
  off; the row's GPU layout kept its lanes as spare (`gpu_world/format.rs`). So a detail
  description has a place on the render side and a precedent against living in the world.
- **The frame is a compute ray marcher with no raster pass.** The hit pass walks a primary ray
  per pixel through the mirror and writes a hit record (position, `t`, a shading normal, a
  material id and flags in `rgba16uint`); the light pass shades that record with the list
  lights' and the sun's shadow rays (ADR-0044, ADR-0027) and reads the baked emissive store
  for its face (ADR-0046); the a-trous, the temporal pass and FSR 2 at 1.5 follow (ADR-0034).
  Entities are traced after the world march with the world hit as the limit (ADR-0050).
  Anything drawn in the frame either becomes a hit record or bypasses every light and
  denoise stage.
- **The owner's one priority is a still image** (2026-11-03): no flicker, no aliasing. Thin
  geometry under a jittered upscaler is the textbook source of both. ADR-0042 already fades
  the shading normal by the voxel's size on screen; Plan 0055 found that far staircases are
  geometry no upscaler removes. A detail layer must therefore vanish before its elements are
  thinner than a pixel, and be placed by a function of the world position so nothing moves
  between frames but what the wind moves.
- **The slow adapter has no headroom.** The Radeon iGPU's hit pass reads about 4.3 ms at
  1.5 and its light pass 10 ms (`docs/nfr/02-frame-budget.md`); its frame already misses
  16.6 ms. Whatever the layer costs there is paid only where styled faces are on screen, and
  the Portable profile (Plan 0069) can set it off.

The literature (`docs/references/surface-detail-layers.md`) splits into shells traced by rays
over a surface (Kajiya & Kay 1989; Perlin & Hoffert 1989; Porumbescu et al. 2005; Policarpo
et al. 2005), layered alpha shells with fins (Lengyel et al. 2001), and rasterised instanced
blades (Wohllaib 2021). Grass fades to a tint with distance in every production scheme
(Boulanger et al. 2009).

## Decision

**Surface detail is a procedural shell over a face, marched in the hit pass where a primary
ray meets a styled face, described per material in a render-side style table, drawn as a hit
record and lit as one, and faded to the face's own albedo by the voxel's size on screen. It
is cosmetic and client-only: nothing in `svet-world`, the op log, the queries, the sync or the
bake knows it exists.** In detail:

- **A style, not a thing.** `svet-render` takes a table of `SurfaceDetail` styles from the
  consumer (`Frame::set_surface_details`): per style a shape primitive (`Stalk`, a tapered
  bendable segment; `Cap`, an ellipsoid on a stalk; `Pebble`, a hemisphere), the face
  directions it grows on, a count per face cell, a height range and a root width in voxels,
  a bend and a wind response, a root and a tip colour, an emission, a seed, and the fade's
  `far_shade`. A style is bound to a material id; the material row's spare lane carries the
  style's index to the GPU, so the hit pass has the style with the row it already reads. The
  world's `Material` is unchanged. Nothing in `crates/` names grass, moss or a mushroom; the
  testbed's presets do.
- **Marched at the hit, stateless.** When the walk hits a face whose material has a style,
  the ray is marched back through the shell of height `h` over that face, over the face cells
  its projection crossed (nearest first, capped), and each cell's elements, placed by an
  integer hash of the world cell, the face axis and the style's seed, are tested
  analytically. The nearest element hit replaces the face hit. A ray that hits no styled face
  may probe the ground under its last stretch so elements show against a wall or the sky
  (the silhouette arm; Plan 0081's spike reads its cost). No storage, no list of exposed
  faces, no update: a dug voxel has no face, so it has no layer; a face a dig exposes has its
  own material's style.
- **A hit record, lit as one.** A detail hit writes the hit buffer: the element's position
  and `t`, a shading normal blended toward the face's, the face's material id, a detail flag
  and the root-to-tip parameter `s`. The light pass shades it as any hit: the sun's and the
  list lights' shadow rays start at the element, the baked emissive light is the face's under
  it (the store has no cell for air), and a root term darkens `s` toward 0. The composite
  takes the colour from the style (root to tip by `s`) and its emission in place of the row's
  albedo and emission. An element casts no shadow and lights nothing: shadow rays walk the
  mirror, and the bake reads voxels. A style that glows is bright on screen; a consumer that
  wants it to light the wall attaches a list light (ADR-0044) or makes the voxel emissive.
- **Faded by the voxel's size on screen.** The layer's count and height fall to zero between
  two projected sizes in pixels per voxel (a setting; Plan 0081 reads two candidates), and an
  element is never drawn thinner than a minimum screen width. Where the layer is gone, the
  face shows its own albedo times the style's `far_shade`, so the far field (Plan 0033),
  which draws the face's albedo, needs nothing from this ADR.
- **Swayed in place.** A wind vector the consumer sets (`Frame::set_detail_wind`; Plan 0036
  hands its `WeatherState.wind` to it) and a phase advanced by the frame's `dt` bend each
  element's tip by its wind response. The hit buffer's reprojection position is the element's
  rest position, so the temporal pass and FSR 2 find the same element last frame; only the
  shading position and normal sway. The engine holds no clock: with no `dt` nothing moves.
- **Entities in it.** The detail hit is the world hit the entity trace takes as its limit
  (ADR-0050), so an element in front of an entity occludes it and one behind is hidden. Nothing
  bends under a foot.
- **A setting.** On, off and the fade's thresholds are render settings (Plan 0069) with the
  Portable profile off; an unpicked arm of Plan 0081's spike that costs nothing when off stays
  as a setting (owner, 2026-11-12).

## Consequences

### Positive
- One mechanism carries grass, moss, strands, fungus caps, flowers, pebbles and snow tufts,
  and each is a row of a table the consumer owns. Plan 0036's snow cover can swap a face's
  style for a white `Cap` set as it recolours the face.
- No world data, no op, no sync, no bake, no query changes: the determinism story of
  `svet-world` is untouched, and the server's hit test ignores the layer as it should.
- Edits are free: the layer is a function of the faces that exist this frame.
- The layer is lit and denoised by the stages that exist, with the shadow rays and the baked
  light the face has, so it does not look pasted on.
- Its cost is per ray that meets a styled face inside the fade distance, zero elsewhere, and
  zero with the table empty: the first consumer pays nothing until it styles a row.

### Negative
- **Elements cast no shadow and light nothing.** A field under a low sun has no blade
  shadows on the ground; a glowing cap does not light the wall unless the consumer adds a
  list light. The root term fakes the carpet's self-shadowing and nothing else.
- **Silhouettes need the probe arm.** Marched at the hit alone, elements show only against
  their own face: a grass hill's horizon is a cube staircase against the sky. The probe arm
  buys the silhouette for a bounded number of short walks per ray; Plan 0081 measures it,
  and if the eye asks for it on the slow adapter it may not be affordable there.
- **The fade is short at 10 cm.** A blade 1.5 cm wide is under a pixel below about 6 pixels
  a voxel, which at 1080p output through FSR 2 at 1.5 and the fly-through's field of view is
  about 11 m. The minimum screen width stretches that at the price of blades that read as
  thickened strokes. Where the fade lands is the owner's pick in the window; it is the one
  place the aliasing lives.
- **Thin geometry under a jittered upscaler.** Even placed by a hash and faded in time, a
  carpet of thin elements is the hardest input FSR 2 at 1.5 sees; the sway moves the tips
  without motion vectors of their own. Plan 0081 reads the stillness rig with the layer on and
  off before the owner's window and stops if the layer alone fails the lamp's reference.
- **The hit pass grows a branch** on every hit's material row. With the table empty the branch
  is one lane read; with styles it is a shell march whose cost on the slow adapter is read,
  not projected.
- **A style is per material, not per place.** Two patches of the same material get the same
  style; a consumer that wants a bare patch uses another material row for it.

## Alternatives considered

### Alternative A: rasterised instanced blades composited with the traced depth

Blades as instanced geometry in a vertex and fragment pipeline, depth-tested against the hit
buffer's `t`, as Ghost of Tsushima draws them. Rejected for three costs at once: the frame has
no raster pass and would gain a second pipeline kind with its own jittered projection to match
FSR 2's; a fragment is not a hit record, so it is either shaded forward with its own lighting
(no shadow rays, no baked light, a second light path to keep consistent) or written into the
hit buffer anyway, at which point the raster pass only found the hits a shell march finds
directly; and it needs a list of exposed styled faces per chunk that the sync, already over
its line (`nfr.md` section 2), would rebuild on every edit.

### Alternative B: shell texturing, alpha layers over the face

Lengyel's fur: `N` concentric layers above the face, each a hashed presence test, the first
present layer wins; a few plane intersections per pixel and a trivial sway (offset the sample
by the wind times the height). Rejected as the design because the layers come apart at
grazing angles, which is the first-person view of the ground from 2 to 10 m, and the fix,
fins at silhouettes, is geometry the ray marcher has nowhere to put. Kept as the spike's
cheap arm: if analytic elements cost too much on the slow adapter, a layered form behind the
same style table is the fallback Plan 0081's Phase 1 can read beside them.

### Alternative C: finer voxels where detail is wanted

A finer grid (2.5 cm, the `detail` of ADR-0050's entity parts) in a world overlay over styled
surfaces. Rejected: 64 times the voxels of a 30 cm layer over the full-detail disc in the
mirror and the sync, a four-fold step count on every ray through it, world data that the op
log, the queries and the bake would have to understand, still static, and still a staircase
that aliases at distance by geometry (Plan 0055's lesson), at a quarter the step.

### Alternative D: grass as voxel content and no engine work

Stamp `grass` voxels and tufts from a prefab as Plan 0032 stamps trees. Rejected by the
owner's eye: at 10 cm a voxel field is rubble, not grass, and there is no sway. It stays the
right answer for things a voxel can be, such as the trees, and nothing here changes that.

## Outcome

_(Added at acceptance if Plan 0081 falsifies something above. Dated. Never rewrite the body.)_
