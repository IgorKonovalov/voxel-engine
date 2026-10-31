# ADR-0041: Smooth materials' surface is the trilinear zero set of the 27-count field in band slots, shaded by its gradient; crisp materials are cubes

> **Status:** proposed
> **Date:** 2026-10-31
> **Related plan(s):** [Plan 0054](../plans/0054-field-surface-trilinear-zero-set-of-the-27-fraction.md)
> (Phase 1's readings, the owner's Phase 2 pick and the owner's Phase 3b pick, owner answer 7,
> are the evidence; its Phase 5 accepts this at the close; amended 2026-10-31 on answer 7). At acceptance this supersedes
> [ADR-0011](0011-contour-planes-in-attribute-pages-with-procedural-detail.md)'s surface
> (its procedural detail stays),
> [ADR-0040](0040-one-voxel-features-melt-into-the-contour-surface-by-a-majority-fit-and-a-normal-field.md),
> of which this is Alternative B, and
> [ADR-0039](0039-the-contour-walk-closes-a-crack-at-the-passed-voxels-exit.md) as moot.

## Context

ADR-0011's surface is one cube per solid voxel clipped by one plane fit from binary occupancy.
It cannot show anything smaller than its own 27-voxel kernel: a one-voxel rod, pillar, sheet or
lone cube keeps exposed cube faces, and two-voxel features and concave corners keep steps.
ADR-0039 closed the cracks between neighbouring planes and Plan 0053 changed the fit's input and
the shading normal; the owner's windowed check (Plan 0053 Phase 4 with Plan 0054 Phase 2,
2026-10-31, session 1) found the rods and slabs standing at the vein in every plane look.

The field the planes approximate has a surface of its own. A voxel's count is its solid voxels
among the 27 around it, itself included; `phi = count / 27 - 1/2` at every voxel centre,
interpolated trilinearly, crosses zero on a surface that is watertight by construction, lies
exactly on every axis-aligned face, rounds convex edges, fillets concave corners and melts a
one-voxel feature into a low bump. Its price is the air side: voxels in air beside rock need
counts too, so air bricks beside rock need storage, and the walk solves a cubic per dual cell it
crosses. ADR-0040 deferred it on those two unmeasured costs. Plan 0054 Phase 1 built it as a
dense scratch pool beside the mirror and measured both (the readings below; the full tables are
in that plan's Implementation log).

Plan 0054 Phase 3a then showed what the field cannot draw: a free-standing feature under about
three voxels across (a 10 cm branch) renders as nothing while its shadow stays, and the owner
picked the field per material at Phase 3b (owner answer 7), crisp materials drawn as honest
cubes.

`nfr.md` §2 (the hit, light and attribute passes) and §3 (mirror bytes, the 3 GB line at
`r_full` 16) are the figures it moves. `nfr.md` §1 names the Radeon RENOIR iGPU as the slow
adapter.

## Decision

**The rendered surface of a smooth material at full detail is the zero set of the trilinear interpolation of
`phi = count / 27 - 1/2` over the voxel centres**, the count taken over the attribute pass's
occupancy (solid is not air; a translucent voxel counts as solid). **A voxel is in the band when
its count is neither 0 nor 27, and a brick is a band brick when it holds a band voxel**: a Full
brick with a surface voxel, and a uniform-solid or uniform-air brick with a voxel of the other
kind within one voxel of it. **Every band brick carries a band slot in the size-classed mirror**:
a band mask and its band voxels' `u8` counts packed in band-index order, a new slot class for
the uniform band bricks, counted by the pressure rule and inside `mirror_bytes` and
`allocated_bytes`. The attribute pass fills the counts. **The walk** runs the DDA to the first
band voxel, splits the ray's segment through it at the voxel's mid-planes into the dual cells it
crosses, and in each cell whose eight corners mix signs finds the first root of the cubic `phi(t)`
(the spike: the cubic's extrema bound monotone runs, Illinois steps find the root); a ray that
reaches a count-27 voxel without a root is a leak, counted where the cracks view counted. **The
shading normal is N1, the analytic gradient of the trilinear `phi` at the hit**; nothing is
stored for it. The detail tilt and the lamp ratio read it, and the shadow ray leaves the hit
along it. **A material is smooth or crisp, smooth by default.** A crisp material's voxels count as air in
the field, sit in the band, and carry bit 7 of their `u8` count; the walk hits such a voxel as
its cube where the ray enters it, with the entry face's normal, which is the plain cube walk's
hit, kept as the crisp path. Where a smooth surface meets a crisp voxel it rounds away from it
as from air: a crisp feature standing on a flat face leaves the face exactly flat, and a crisp
inlay flush in a face opens a groove about 0.75 voxel deep along its edge (worked from the
counts in Plan 0054 Phase 4c, pending the owner's seam rule there). The flag is a render
property of the material row: `MaterialFlags::CRISP` in `vxe-world`'s table beside
`TRANSLUCENT`, which no edit, query, collision or audio path reads, so the world stays
headless and deterministic; it has no effect on a `TRANSLUCENT` row, and the opaque payload
stays the consumer's (pending the owner's choice of its home in Plan 0054 Phase 4c; Alternative
H is the renderer-side set). A chunk held as mips has no crisp voxel: its cells are cubes for
every material. **Shadow rays walk the voxel occupancy**, crisp and smooth voxels alike, from
an origin pushed along the hit's normal by the existing shadow push; they do not walk the
field. **Translucent voxels count as solid in the field** for the primary hit, and the
translucent walk after it runs over the voxel occupancy as before. The mechanism is in
`vxe-render`: `field.rs` (the band test and the spike's pool),
`attrib.wgsl`'s `field_main`, and the field walk in `march.wgsl` (`field_march`,
`field_voxel`, `field_piece`, `field_normal`). `vxe-world`, its edits, results and collision
do not change.

## Consequences

### Positive
- No cube face, no crack, no closing rule: the analytic scenes and both poses read 0 leaks under
  the spike, and the flat rigs lie on their face planes (worst hit 7.63e-6 voxel; lavapipe,
  b282738).
- One-voxel features melt and corners round: box pixels against snap, lavapipe at b282738 under
  the plane look's position-only metric, rod 9 against 499, lone cube 0 against 24, wall pillar
  2 against 276, free pillar 0 against 828, sheet 6 against 8108, the vein 296 against 35 098.
  The owner: "smooth, wonderful" at both poses.
- The guardrails hold with room (cavity normal error 4.13 degrees, silhouette 24 px against
  face's 172, `A` 3.15 %; lavapipe, b282738).
- The storage replaces the `u16` attribute rather than adding to it, and the cracks view, the
  fit settings and the snap memory go with the planes.

### Negative
- **The hit pass costs 6x.** At the level's spawn, 1080p from 720p, release, perf lock, three
  alternated runs at b282738 against 811b37e: RTX 3080 Laptop 3.46 against 0.58 ms, Radeon
  RENOIR 30.04 against 4.30 ms (7x); light unchanged (0.90 and 6.91 ms). The field hit
  pipeline holds 128 VGPRs against the plane walk's 64 (RADV shader stats). The owner's bar
  (Plan 0054 owner answer 3): **RTX hit plus light at most 5 ms**; the spike read 4.36.
- **The Radeon is out of budget, and that is open.** 30.04 + 6.91 ms on `nfr.md` §1's slow
  adapter. Whether the iGPU ships the field or falls back is decided by a walk-speed spike with
  its own plan and ADR after Plan 0054; until then the field is an RTX-class surface.
- **The attribute pass does more.** Initial fill at the level, same runs: 14.2 to 23.8 ms on the
  RTX, 59.7 to 96.7 ms on the Radeon; the autopilot's p50 and p99 rise too (loaded readings, in
  the plan's log).
- **Storage grows on the air side.** The census at b282738 (the level at `--seed 1` on lavapipe;
  the flight's full residency on the RTX 3080 Laptop): packed band bytes 96 MB at the level and
  483 MB over the flight, against today's attribute bytes of 95 MB and 369 MB; dense, as the
  spike stored them, 241 MB and 1.15 GB. The owner's bar (owner answer 4): **live band bytes at
  most the packed figures plus 10 %**, inside the mirror's accounting; `r_full` 16 under 3 GB
  stays the hard line.
- **The look and the collision disagree by up to half a voxel**: the surface lies inside a convex
  edge and outside a concave corner, and a one-voxel feature still collides, blocks sound and
  counts in every world query while it renders as a bump (the rod's flat top at `z = 60.333`;
  a free pillar thins away above its base). Two voxels thick shows a feature only where it rests
  on a surface (a 2x2 rod on a floor keeps its lower half as a bump).
- **A free-standing feature of a smooth material under about three voxels across renders as
  nothing, and its shadow stays** (the shadow ray walks the voxels). Measured on Plan 0054
  Phase 3a's rig (lavapipe, 17045d8): the one-voxel and diagonal branches, the 2x2 rod and the
  one-voxel sheet read 0 % of the face look's pixels, the 3x3 rod and the two-voxel sheet about
  76 %; marked crisp, every feature reads 100 %. The owner, of the field everywhere: "not
  acceptable - invisible materials, only one trunk is visible, invisible cast shadow". A
  consumer marks a material with thin features crisp or models them three voxels thick; a voxel
  left thin by a carve in a smooth material (a one-voxel wall between two tunnels) is invisible
  and still collides. Whether that is accepted or met by an automatic per-voxel fallback is
  open in Plan 0054 Phase 4c (pending the owner). The arithmetic, from
  the formula, not measured: a free one-voxel rod counts 3 (`phi = -0.39`), a free 2x2 rod 12
  (`-0.056`), a free one-voxel sheet 9 (`-1/6`); no dual cell around them mixes signs. A free
  3x3 rod and a two-voxel sheet show at about their true size. At 10 cm voxels a tree branch, a
  rope, a rail or a grate vanishes while it still collides, blocks sound and counts in every
  world query. Phase 1's free pillar (0 box pixels) and sheet (6) agree.
- **Crisp voxels cost on both sides.** The walk tests a band voxel's crisp bit: `field-cubes`
  read 5.41 against `field`'s 4.95 ms hit at the level's spawn on the RTX 3080 Laptop with no
  crisp material present (Plan 0054 Phase 3a, 17045d8). Band bricks within one brick of a crisp
  voxel take their counts from the CPU at the sync, a cost that grows with a world's crisp
  content.
- **The bar is not met yet, and the window reads worse than the bar's figure.** Field-cubes hit
  plus light read 6.38 ms at the spawn against the 5 ms bar (17045d8), and the owner's window at
  the vein and patch read a march GPU p50 of 29.3 and 30.4 ms ("fps is really low"). Plan 0054
  Phase 4a attributes it and Phase 4b works it under a stop rule that returns the call to the
  owner.
- N1's gradient is only C0 across cells: the normals view shows soft creases along the dual-cell
  grid (tile contrast at the vein 0.397 % under N1, 0 under N2). The owner did not see them in
  the shaded view.

### What the planes' removal leaves
- **The box metric and its tests** stay as the regression gate, with the field look's condition
  that a box pixel's shading normal lies within 15 degrees of the face's axis normal (Plan 0054
  owner answer 1); the leak count takes the crack classes' place.
- **The LOD ring's mip cells stay cubes**, as they were under the planes; no transition at the
  full radius beyond today's.
- **Shadows** leave a hit along its normal and walk the voxel occupancy (the Decision); in the
  spike they walked the planes.
- **Translucency** counts as solid in the field for the primary hit, and the translucent walk
  after the hit is the voxel walk it was; the spike's field look ran none.
- **The plain cube walk stays** as the crisp materials' path, and `face` as the comparison look;
  `cubes-field` goes with the attribute whose normal field it shades by.
- **ADR-0011's procedural per-material detail** stays, tilting N1.
- ADR-0038's plane code goes with the attribute it encodes.

## Alternatives considered

### Alternative A: the per-voxel plane with snap (ADR-0011, ADR-0039)

The status quo: watertight after ADR-0039, cheap (hit pass 0.58 ms RTX), and one clipped cube
per voxel. It lost on the owner's eye: one-voxel rods and slabs at the vein, 499 box pixels on
the rod scene and 35 098 at the vein (lavapipe, b282738).

### Alternative B: the plane fit on a majority-filtered occupancy (ADR-0040)

Melts what stands on a surface (rod, lone cube and wall pillar at 0 box pixels) at no storage
cost. It lost because it turns what does not stand on a surface into full cubes, a free pillar's
top row and a sheet's rows (free pillar 828 to 1182, sheet 8108 to 14 497 box pixels), raises the
groove's (697 to 4870) and reaches none of the vein's features (35 098 to 39 263; Plan 0053
Phase 2, lavapipe at 11a9b1e). The owner: "same problem" at the vein.

### Alternative C: the normal field over the planes (ADR-0040)

Smooth shading at the same hits: the hit mask is byte for byte the plane look's, so every box
stays a box (Plan 0053 Phase 3, lavapipe at 811b37e), at +12 % hit plus light on the RTX. The
owner: "same problem" at the vein, alone or with the filter.

### Alternative D: cubes shaded by the field (`cubes-field`, ADR-0040 Alternative D)

Honest cubes with the field's normal: the face silhouette (172 px) and calm at the vein ("not
even too bad"), but at the patch thin lit lines along the cube edges under the lamp, where the
field normal and the cube face disagree ("not acceptable really"; Plan 0053 owner answer 3).

### Alternative E: the field with N2, the corners' blended normals

The same surface shaded by the trilinear blend of the eight corners' normals: no dual-cell
creases (tile contrast 0), but "not much of a visual difference" to the owner, at about twice
N1's hit cost on the RTX (7.00 against 3.46 ms) and 1.4x on the Radeon (42.33 against 30.04),
and a stored normal per band voxel. Rejected on the owner's eye and the cost.

### Alternative F: the field for every material (Plan 0054 Phase 3b's outcome b)

One surface everywhere, thin features modelled three voxels thick. Rejected by the owner on
the thin rig (owner answer 7): the one-voxel, diagonal and 2x2 features vanished while their
shadows stayed. It is cheaper by the crisp test (4.95 against 5.41 ms hit, RTX, 17045d8).

### Alternative G: crisp voxels count as solid in the field

The smooth surface would run flush into a crisp inlay with no groove, but it would also fillet
the concave corner at every crisp feature's root with a smooth skin of about one voxel, so a
crisp branch would grow a rounded collar where it meets the trunk. Phase 3a built crisp as air
and the owner judged that look ("seems to be good"); this stays the alternative if the seam's
groove is refused (Plan 0054 Phase 4d).

### Alternative H: the crisp set held by the renderer (Phase 3a's scratch)

`GpuWorld::set_crisp_materials` by material id leaves `vxe-world` untouched, but splits a
material's definition between the world's table and a renderer call, and every renderer
instance must be told. The material row already carries the renderer's properties
(`TRANSLUCENT`, `detail`, `opacity`); the flag joins them (pending the owner, Plan 0054
Phase 4c).
