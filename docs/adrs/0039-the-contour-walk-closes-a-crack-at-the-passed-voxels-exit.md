# ADR-0039: The contour walk closes a crack at the passed voxel's exit, with the passed plane's normal

> **Status:** proposed
> **Date:** 2026-10-30
> **Related plan(s):** [Plan 0050](../plans/0050-watertight-contour-surface-crack-spike-and-closing-rule.md)
> (Phases 3 and 4, whose readings this records). Amends
> [ADR-0011](0011-contour-planes-in-attribute-pages-with-procedural-detail.md)'s hit rule in
> what the walk does after it passes a surface voxel outside its plane.

## Context

ADR-0011's walk tests a solid voxel's plane against the ray's segment through the voxel's own
cube and passes the voxel as air when the segment stays outside the plane's solid half-space.
Two neighbours' planes do not meet at their shared cube face, so a ray can pass between them
and reach a voxel one deeper (crack class 1: a dark cube face about a voxel below the surface)
or enter the next surface voxel already inside its half-space (class 2: a hit on the shared
face with that voxel's normal, which can face the lamp from inside a crevice). Plan 0050's
Phase 1 measured them at the owner's poses under the old rule: 2250 class 1 and 2 pixels of
82 944 at the vein, 4803 at the patch, and attributed the owner's dark wedges to class 2 at
the ledges and the bright triangles to class 2 at the facet vertices.

Three closing rules were built behind one setting and read on the same scenes, poses,
guardrails and cost rig (Plan 0050 Phase 3, lavapipe for the images, the RTX 3080 Laptop GPU
and the Radeon RENOIR for the hit pass at the level's spawn, 1080p from 720p):

| rule | class 1+2 px, vein / patch | cavity normal error | pillar silhouette (face 172) | specks only with shadows | hit pass against the pre-plan binary |
|---|---|---|---|---|---|
| old | 2247 / 5266 | 7.19 | 32 | 6 | baseline |
| snap | 0 / 0 | 7.13 | 32 | 6 | +1.0 % RTX, +2.3 % Radeon (snap alone) |
| extend 0.25 | 99 / 1047 | 7.13 | 28 | 7 | about old's |
| extend 0.5 | 0 / 136 | 7.13 | 32 | 7 | about old's |
| blend | 0 / 0 | 5.81 | 62 | 5 | 5.5x RTX, 8.2x Radeon; 128 VGPRs |

## Decision

The walk remembers the attribute of the last surface voxel it passed outside its plane since
the last empty cell. When the next solid cell would be a class 1 hit (no plane) the hit is at
the passed voxel's exit point with the passed plane's normal; when it would be a class 2 hit
(an entry already inside) the hit is at the shared face with the normalised mean of the two
planes' normals; a plane crossing inside the next voxel (class 3) is unchanged. The memory is
one register. The rule is `CrackClosing::Snap`, the contour looks' default; the shadow push
(the origin stepped along the normal out of every neighbouring plane's half-space) stays
selectable and off, for its light pass cost (1.439 against 0.913 ms on the RTX, 9.165 against
7.026 on the Radeon).

## Consequences

### Positive
- Classes 1 and 2 read 0 on the tilted slab, the concave corner, the cavity and at both poses;
  the owner's Phase 5 check found the wedges, triangles and slivers gone.
- Within the cost bar: +1.0 % RTX and +2.3 % Radeon on the hit pass once the losing rules
  are deleted, and the guardrails, the specks and every face golden hold.

### Negative
- The closed crack is a flat sliver on a cube face with a normal that is a mean, not a surface
  the planes agree on: the rule hides a crack it does not remove. The surface stays one clipped
  cube per voxel, with everything ADR-0011 and Plan 0053 say about that.
- While the losing rules stay selectable the hit shader holds 128 VGPRs and costs +10 % RTX
  and +40 % Radeon; Plan 0050's Phase 6 deletes them.

## Alternatives considered

### Alternative A: extend, the plane tested through a dilated cube

The plane is tested against the ray's segment through the cube dilated by 0.25 or 0.5 voxel,
and a crossing in the dilation hits at the cube's exit. It closes only cracks narrower than the
dilation: 1047 class 1+2 pixels remain at the patch at 0.25, 136 at 0.5, and a wider dilation
fattens silhouettes. It lost because it cannot close a crack of any width.

### Alternative B: blend, a trilinear field of the eight planes' distances

The signed distance inside a surface voxel is the trilinear blend of the eight surrounding
voxels' plane distances, air counting +1 and a planeless solid voxel -1, so the zero set is
continuous across cube faces. It closes every crack and reads the best cavity normal error
(5.81 degrees) and the fewest specks, but it evaluates eight lookups and eight attribute reads
up to four times per surface voxel stepped: the hit pass read 3.302 ms against 0.598 on the
RTX and 35.151 against 4.264 on the Radeon, and its silhouette error doubled (62 px). It lost
on cost by an order of magnitude. A field surface evaluated once per dual cell, in air cells
too, is Plan 0053's deferred alternative, not this rule.

### Alternative C: the old rule, cracks left open

The status quo. The owner's eye rejected the wedges and triangles at Plan 0049's live session.
