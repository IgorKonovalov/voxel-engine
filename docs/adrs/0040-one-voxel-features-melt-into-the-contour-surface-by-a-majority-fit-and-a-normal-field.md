# ADR-0040: One-voxel features melt into the contour surface: the plane is fit on a majority-filtered occupancy, and the shading normal is a field over the planes

> **Status:** proposed
> **Date:** 2026-10-30
> **Related plan(s):** [Plan 0053](../plans/0053-one-voxel-features-on-the-contour-surface-majority-fit-and-a-normal-field.md)
> (its Phase 4, the owner's check, accepts or supersedes this). Amends
> [ADR-0011](0011-contour-planes-in-attribute-pages-with-procedural-detail.md) in what
> occupancy the plane fit reads and what normal a hit shades with; the attribute, its pages
> and the hit rule (with ADR-0039's closing) are unchanged.

## Context

Plan 0050 closed the contour surface's cracks and its owner check (Phase 5, 2026-10-30)
found what remains at both poses, in the old rule and in snap alike: voxel-sized flat-faced
boxes, one-voxel rods and pillars along walls and ledge edges, lone cubes on floors and
ceilings, often two-toned in the normals view. The owner's words: fix it or drop contours.

ADR-0011's surface is one cube per solid voxel, clipped by one plane fit from the voxel's
27-neighbourhood (the occupancy gradient for the normal, the solid count for the offset,
`(count / 27 - 0.5) * 3` voxels along the normal). What that does to a one-voxel feature
follows from the formula (worked, not measured; Plan 0053's Phase 1 measures it):

- **A rod on a flat floor** (one voxel high, one wide). The rod voxel's neighbourhood is the
  nine floor voxels below and its two rod neighbours: count 12, offset -0.167, and the
  gradient points straight down, so its plane is horizontal a third of a voxel below its
  centre. The rod renders as a slab a third of a voxel high whose sides are its cube faces,
  hit directly from air. The floor voxels beside the rod see it diagonally: count 21,
  offset 0.833 beyond their cube, normals tilted about 20 degrees toward the rod. Three
  flat tiles, three tones.
- **A lone cube on a floor** (count 10, offset -0.389) is a plate a ninth of a voxel high;
  the owner's lone cubes stand on rough surfaces where the count-based offset leaves the
  whole cube.
- **A one-voxel groove** in a flat wall. Its bottom voxel reads count 24: the plane lies
  beyond the cube and the bottom is a flat cube face. Its two side voxels read count 15 with
  normals tilted 25 degrees into the groove, opposite ways: a two-toned stripe.
- **A free pillar or a one-voxel sheet** has a symmetric neighbourhood: the gradient vanishes
  and the voxel has no plane, so it is a full cube today.

The clipped cube can only remove material from the feature voxel's own cube. A smooth version
of a rod flares onto the floor beside it, and a smooth groove is a fillet: both lie in the
neighbouring air cells, which no clipping of a solid cube reaches. The two-tone comes from the
normals being one constant per voxel: a one-voxel feature swings the 27-voxel gradient by 20
to 25 degrees across three tiles with hard edges between them. The eye reads three hard-edged
tiles as a box; it read the same cubes shaded with a smooth gradient as rock at Plan 0005,
where `Gradient27` was the owner's first preference on screen and lost on cost and silhouette.

The prior art says the same (`docs/references/voxel-optimizations.md`, Laine & Karras 2010,
sections 3.2 to 3.4). ESVO's contours are the one published form of a plane-clipped voxel
cube, and they fit each plane from the original triangle surface so that it encloses it; a
one-voxel rod there gets a slab as thin as the real rod. Our fit reads binary occupancy and
has no surface to enclose, so the fault is in what the fit reads, not in the clipped cube.
That is why this decision changes the fit's input and the shading normal and leaves the hit
rule alone. No shipped voxel renderer we know of kept contours; the smooth ones march or mesh
a field (Alternative B's family) and the ones at our voxel scale keep cubes (Alternative D).

The budgets this touches: `nfr.md` §2 the hit pass (RTX 3080 Laptop 0.604 ms, Radeon RENOIR
4.364 ms at the level's spawn, 1080p from 720p, snap alone) and the light pass (0.913 and
7.026 ms), the attribute pass (the level's fill 17.0 ms on the dev box under the weighted
fit, Plan 0039); §3 mirror bytes (157 872 640 B live at the level).

## Decision

Two changes, each behind its own setting until the owner's check, then the pick as the
default.

**The plane is fit on a majority-filtered occupancy.** The attribute pass reads the real
occupancy of the brick and a two-voxel shell, filters it (a cell is solid for the fit when at
least 14 of its 27 neighbours, itself included, are solid) and fits every attributed voxel's
plane, normal and offset, on the filtered occupancy. Nothing else changes: the surface mask,
the attribute, the walk and the storage are as they were. An eroded voxel (solid, filtered
away) gets the plane of the hull under it: a rod voxel over a floor reads count 9 and offset
-0.5, its plane is its own bottom face, and every ray passes it or crosses onto the floor's
face; the floor voxels beside it no longer see it and keep the floor's plane. A voxel the
filter buries (a groove's bottom, filtered to 27 solid neighbours) keeps a plane whose normal
is the unfiltered fit's and whose offset is the clamp, so it hits as a cube face with the
hull's normal and never an axis normal. A voxel whose filtered neighbourhood is empty (a free
pillar or a sheet above its lowest rows) has no plane and stays a cube, as today.

**The shading normal is a field.** At a hit point the light pass shades with the normalised
trilinear blend, over the eight voxel centres around the point, of the planes' normals of the
voxels that have one; air and planeless voxels contribute nothing; where none contributes the
hit's own normal stands. The hit record and the shadow ray's origin keep the plane normal; the
normals view shows the field. The detail tilt and Plan 0039's lamp ratio apply to the field's
normal as they applied to the plane's.

The owner's Phase 4 check decides, on screen at the vein and the patch, between today's look,
each change alone and both; the pick becomes the default and this ADR is accepted. If the
boxes or the steps that remain still fail the eye, this ADR is superseded by the owner's
choice between Alternative B (the field surface) and Alternative D (cubes shaded by the
field, contours dropped).

## Consequences

### Positive
- Every one-voxel feature that stands on a surface melts into it: no exposed cube face of a
  rod, a lone cube or a bump remains to be hit from air, and the normals around it stop
  tilting toward it. A one-voxel groove keeps its depth but shades like its wall.
- The hard tile edges of the normal field go everywhere, not only at the boxes: the
  "sharp voxel corners on some smooth walls" of Plan 0039 are the same edges.
- No storage, mask, page or walk change. The hit pass cost is the eight reads of the field,
  once per shaded pixel, which Plan 0053 bars.

### Negative
- **Features under the filter's scale change.** A rod, a lone cube, a one-voxel bump on a
  surface or a one-voxel pillar against a wall is no longer visible in the contour look,
  though it still collides, blocks sound and counts in every world query. A free one-voxel
  pillar or sheet keeps its upper rows as cubes (their filtered neighbourhood is empty, so
  they have no plane) over a base the filter thins: worked from the formula, a sheet's first
  row on a floor counts 15 and keeps a third of its height, its second row counts 9 and keeps
  an eighth, and the rows above are cubes. A game that builds thin things builds them two
  voxels thick, or asks for a per-material opt-out of the filter, which is a followup, not
  this decision.
- Two-voxel features keep flat sides: a two-wide ridge on a floor keeps a vertical cube face
  about 0.45 voxel tall under a roof-shaped top (worked from the formula). The field normal
  shades that face smoothly; the geometric step stays.
- Concave corners stay cubes. The count-based offset puts the plane beyond the cube wherever
  the neighbourhood is more than two thirds solid, and the filter does not change that; a
  fillet needs the surface in the air cell, Alternative B.
- The attribute pass loads a 12-cube instead of a 10-cube per brick and counts 27 neighbours
  per cell before the fit, a guess of up to 1.8x its cost, barred in Plan 0053.
- The shading normal and the geometric normal differ at every hit: the light pass's eight
  reads per pixel are a cost on the slow adapter, barred in Plan 0053.

## Alternatives considered

### Alternative A: clip each voxel's cube by its neighbours' planes

The owner's hypothesis: a solid voxel's shape is its cube cut by its own plane and by the
planes of its face neighbours, so a hit on an exposed cube face must also lie inside the
neighbours' half-spaces. Worked on the rod: the floor voxel beside it has normal
`(0, 0.35, 0.94)` and offset 0.833, and extended into the rod's cube that plane cuts the rod's
top from 0.167 below its centre at the far side to 0.30 below at the near side, so the rod's
side face shrinks from 0.33 to about 0.20 voxel and stays vertical; on the groove the side
voxels' planes, extended into the bottom voxel, cut nothing of its exposed face. It lost
because clipping removes material from the feature's own cube and the flare and the fillet the
eye needs are in the air cells beside it; Plan 0053's Phase 1 reads it as a scratch so the
measurement, not this arithmetic, settles it. ESVO clips a voxel by its ancestors' contours
in the same spirit, and it works there because every contour encloses the real surface, so
an intersection only tightens toward it; a neighbour's occupancy-fit plane encloses nothing,
so the intersection can only cut.

### Alternative B: a field surface, the trilinear zero set of the 27-fraction

Store `(0.5 - count / 27) * 3` at every voxel within one voxel of the surface on both sides
(the attribute's offset code already holds it to an eighth of a voxel for the solid side),
stop the walk at band voxels through the attribute mask, and solve the trilinear crossing once
per dual cell. It is the surface the per-voxel offsets approximate, watertight by
construction, rounding convex edges and filleting concave corners with no cube face anywhere,
and it melts one-voxel features as the filter does. It is deferred, not rejected: air voxels
beside the surface need attributes, so air bricks next to rock need a slot (mask and
attributes, no payload), a storage change of two to three times today's attribute bytes
(a guess), and the walk cost is unmeasured: the blend rule's eight reads per evaluation, four
evaluations per voxel stepped and 128 VGPRs read 5.5x and 8.2x the hit pass; one solve per
dual cell at 64 VGPRs is a guess of 1.3x to 2x. It is the next plan if Phase 4 says the steps
still fail the eye.

### Alternative C: dilation as well, air voxels the filter fills made hittable

The majority filter also marks air voxels with 14 or more solid neighbours solid, which fills
one-voxel grooves and the air cell of every 90-degree concave corner. Hitting them needs
attributes on air voxels, Alternative B's storage. Deferred with B; this decision's filter
reads dilated cells for the fit only, so a groove's walls lose their tilt while the groove
keeps its depth.

### Alternative D: drop contours for cubes shaded by the field

Face geometry, the attributes' normals blended as above: `Gradient27`'s look (the owner's
first preference at Plan 0005) at eight attribute reads instead of 27 lookups per hit, with
face mode's 172-pixel silhouette against contour's 32. Plan 0053 builds it as a look preset so
the owner compares it in Phase 4; it is the fallback if the boxes stand, not the decision.

### Alternative E: a smoother per-voxel normal

Fit the normal over a 5-cube instead of a 3-cube. It lowers the tilt of the tiles around a
feature from 25 to about 10 degrees and widens it to five tiles. It lost because the normal
stays one constant per voxel and the hard edges between tiles are what the eye reads as a box.
