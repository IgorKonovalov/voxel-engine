# ADR-0049: Where a voxel is small on screen, the baked light is read by the far shading normal, a squared-cosine blend of the store's direction values

> **Status:** proposed
> **Date:** 2026-11-10, amended 2026-11-11 (proposed, edited in place: the weight rule, after
> Plan 0061 Phase 1's readings and the owner's pick)
> **Related plan(s):** [Plan 0061](../plans/0061-antialiasing-far-bands-under-the-baked-light-and-the-edges-left.md)
> (Phase 1 checks the source this ADR assumes and can reject it, by two stop rules: the far normal
> must not move the far crops under the store, and direction changes must carry the largest share
> of their distance from the reference; Phase 4's window picks or drops it). Extends
> [ADR-0042](0042-the-far-shading-normal-blends-to-the-27-count-gradient-by-the-voxels-size-on-screen.md)
> (the far normal) to the light of
> [ADR-0046](0046-static-emissive-light-is-baked-per-chunk-when-a-chunk-loads-or-changes.md)
> (the store). Leaves [ADR-0034](0034-fsr-2-ported-to-wgsl-as-the-default-upscaler-candidate.md)'s
> FSR 2 at 1.5 as it is.

## Context

The owner, Plan 0059's Phase 13 window (2026-11-09, the fly-through's defaults: FSR 2 at 1.5, far
normal `[2, 6]`, the baked light, lamp off), at the showcase's hall: "a lot of aliasing on this
step, doesn't look good". The still shows swirling contour lines over the far ceiling, the cyan
wall and the floor. In Phase 8d, the first window on the store, the same hall read "aliasing got
much worse". These are ADR-0042's bands: a dome of 10 cm cubes is contour lines at one-voxel
spacing, each riser one face direction and each tread another, and below about two pixels of
spacing they alias.

ADR-0042 removed them by blending the shading normal toward the 27-count gradient where a voxel is
small. That worked because every light then read the shading normal. **Under the store it does
not, by the code:** `temporal.wgsl` reads a cube-face hit's emissive part as
`store_brick_value(store_voxel(p, nf), store_face(axis, nf))`, the brick's mean for the hit face's
direction, with `nf` the face normal from `shadow_normal_of`. The blended normal `rec.n` reaches
only the lamp (`light.wgsl`) and the filters' weights. So in a lamp-off scene the far normal costs
its hit-pass time (Plan 0056: RTX +1.5 ms at `[2, 6]`, RADV +6.6 ms, the showcase start) and
changes nothing on the light. Plan 0061's Phase 1 reads whether this is what the eye sees.

Phase 1 read it (RTX, the hall, 2026-11-10): with the light passes at full resolution the far
normal moves no far-crop figure, so the store is read by face as above. At the default half
resolution it does move them (energy about −10 % at `[2, 6]`), and the cause is the filters: the
upsample in `lit_at` weighs a light tap by `max(n.n', 0)^32` and the a-trous `edge_weight` by
`^128`, both on `rec.n`. Where the far normal bends a riser's and a tread's normals toward each
other, their taps stop being orthogonal and mix two different face values, which blurs the far
crops and doubles FSR 2's still jumps (20 739 to 43 605). The pixels at a direction change carry
97.4 % of the far crops' distance from the reference.

The store holds, per 8-voxel cell, six direction values, each the mean of that direction's faces,
and a face reads the 2 x 2 cells around it in its plane (ADR-0046's Outcome). A riser and the tread
beside it sit in the same cell and read two of its six values. The information to light a far
staircase as one smooth surface is already in the cell.

## Decision

**Where the far normal's weight is above 0, a cube-face hit reads the store by its shading normal:
the sum over the three axes of `n_a^2` times the cell's value for the direction `n` faces on that
axis, over the directions the cell holds, renormalised.** At weight 0 the shading normal is the
face normal, one term has weight 1, and the read is today's to the bit, so near faces stay honest
cubes. Each term is `store_brick_value` at the same voxel with that direction, its 2 x 2 in-plane
taps included, so a far pixel costs up to three of today's reads. A direction the cell lacks drops
out of the sum; with none left but the hit face's, the read is the hit face's. The knob is a
`Lighting` bit, off in the library; Plan 0061's window decides whether the fly-through's default
takes it. The glass's reflected share (ADR-0047) keeps the face read.

"Where the far normal's weight is above 0" is the default look's case (`face`), where the shading
normal leaves the face normal only there. Under the whole-frame normal looks (`gradient27`,
`trilinear8`, opt-in) the shading normal is the look's at every distance, and the read follows it
on near faces too, as the lamp already does. The field look does not read the store by face.

**The weight rule (amended 2026-11-11, the owner's pick).** A second `Lighting` bit, off in the
library: the half-resolution light's filters weigh two hits by their face normals (the unit
vector on the hit's face axis, signed against the view ray) instead of the shading normal, in
`lit_at`'s upsample and the a-trous `edge_weight`; a hit with no face axis keeps the shading
normal, and the plane term is unchanged. A far riser's and tread's taps then stay apart in the
filters, and the read above is what makes their values smooth. Where the shading normal is the face
normal (every near face under `face`) the weights are today's to the bit. Full-resolution light
(`--lighting full`, ADR-0018's other tier) stays a comparison arm in Plan 0061's window.

The blend is the "ambient cube" read of Half-Life 2's shading (McTaggart, Valve, 2004) applied to
the store's six direction means. It is not exact: a staircase's true footprint average weights each
face by its projected area toward the eye, not by the normal's components. It is chosen because it
needs no new storage, no pass and no view dependence, and it meets today's read exactly at weight 0.

## Consequences

### Positive
- The far normal reaches the light again, so ADR-0042's fix applies to the baked scenes it was
  designed for, with the cubes' geometry untouched.
- No storage, no pass, `frame_bytes` unchanged; the store and its bake are untouched.
- The image FSR 2 gets is smoother at the source, so its jitter has fewer sub-pixel contrasts to
  lock and crawl on (Plan 0056: without the far normal, FSR 2's still jumps rose from 4766 to
  221 412 under the eye lamp).

### Negative
- **Up to three store reads a far pixel** in the temporal pass instead of one, each 4 taps. The cost
  is a guess until Plan 0061 Phase 2 reads it on both adapters.
- **It inherits the far normal's band in internal pixels** (ADR-0042): at FSR 2's 1.5 it reaches
  nearer than at native, and the setting picked by eye (`[1, 2]` in Plan 0056, under the table's
  light) has to be picked again under this read.
- **Not the true footprint average.** A staircase seen edge-on reads the normal's mix, not what a
  supersampled reference shows; Plan 0061 reads the distance from that reference.
- **A cell's direction mean leaks.** A riser's value is the mean of the cell's faces of that
  direction, which may face another room (ADR-0046's Alternative E leak). The blend spreads that
  leak to the treads' pixels far away.
- The far normal's own hit-pass cost stays; this ADR adds to it.
- **Face weights give up the far normal's smoothing in the filters.** Across a far step the
  upsample and the a-trous filter keep the riser's and the tread's light apart, as with the far
  normal off; if the read does not make the two values agree, the far light shows the light grid's
  half-resolution blocks instead of blur.

## Alternatives considered

### Alternative A: more rays per output pixel (FSR 2 at scale 1, or native with a quality profile)

Plan 0056's window called full-rate FSR 2 "antialiazing works to some degree, picture is better" at
about double the march (20.6 against 8.7 ms, RTX, the window's size), and native at 2530 x 1378
reads 18 to 20 ms on the RTX and 88 ms on the Radeon at the showcase start. ADR-0042's Alternative B
holds as well: a stable reconstruction of a staircase whose two faces read different light is still
stripes, only finer. It stays an opt-in quality setting.

### Alternative B: a store prefiltered by footprint (a mip of the direction values)

A coarser level of the store, read by the voxel's size on screen. It needs new storage, a bake
step, and a choice of what a coarse cell's six values mean; the six values of the cell already
there answer the question first. It is the next thing to try if the blend's distance from the
reference is too large.

### Alternative C: a screen-space filter on the stored part

Pass the stored part through the a-trous filter, or blur the far composite. The a-trous normal
weight (`max(n.n', 0)^128`) does not cross a riser by design, and loosening it smears the near
cubes; a screen blur is camera-anchored and shimmers when the image moves under it (Plan 0056's
static dither read the same way, `C` up from 0.818 to 1.097).

### Alternative D: change the floor and dome generators

Smoother terrain makes fewer terraces. The geometry is the consumer's, and a consumer's cave will
have the same terraces; the engine has to draw them well.

### Alternative E: full-resolution light as the fix (rejected by the owner, 2026-11-11)

`--lighting full` removes the filters' mixing (Plan 0061 Phase 1: FSR 2's still jumps 1 309, the
far crops nearer the reference) at +3.41 ms GPU p50 on the RTX with the lamp off (+4.7 ms lamp on,
+3.81 ms on the Radeon), the denoise at 4.35 ms against its 3.0 ms line and the RTX's wall p99 at
18.0 to 18.6 ms. The direction bands stay (edge ratio 2.52), since the store is still read by face.
It stays a comparison arm; a default flip to it supersedes ADR-0018's resolution pick in a new ADR.

### Alternative F: keep the filters' weights on the shading normal (the read alone)

The read as first proposed. Phase 1 read the mixing it leaves under the far normal: the far crops'
energy down 10 % and the still jumps doubled. Rejected by the owner, 2026-11-11.
