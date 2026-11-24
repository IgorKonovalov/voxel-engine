# ADR-0064: A part's scale is per axis in its own frame, about its pivot; it moves its children's pivots and never changes their shapes

> **Status:** proposed
> **Date:** 2026-11-16
> **Related plan(s):** [Plan 0082](../plans/0082-squash-and-stretch-a-scale-per-part-in-the-pose-and-the-clips.md)

## Context

ADR-0050 makes an entity a tree of rigid parts: a part's pose is a translation in 1/256 voxel
and a Q14 quaternion, composed down the tree in integers, so the server's ray query gives the
same hit on every platform. ADR-0051 keys those two quantities in a `.rig` clip. The owner asked
(2026-11-16) whether jelly-like entities, slimes, are possible. A slime made of up to sixteen
rigid parts reads stiff; what sells a soft body in animation is squash and stretch, a scale per
axis that changes over the clip.

Three facts shape the choice. The renderer already carries a full 3 x 4 affine map per part
instance and transforms each ray into the part's own space before it walks the voxel grid
(`entity.wgsl`), so a diagonal scale costs it nothing new. The headless query does the same in
exact fractions (`svet-entity`'s `raycast.rs`), where the part's map must stay something an
integer walk can invert. And a non-uniform scale that reaches a rotated child shears the child:
its linear map stops being a rotation times a diagonal, its face normals stop being its rotated
axes, and the renderer's cheap inverse (the transpose over the detail) stops being an inverse.

## Decision

A `PartPose` gains a scale per axis, `s: [u16; 3]` in 1/256 (256 is one; 64 to 1024, a quarter
to four times), applied in the part's own frame about its pivot, before the part's rotation.
The scale changes two things and nothing else: the part's own volume (voxel `(i, j, k)` spans
`[i s_x, (i + 1) s_x)` model voxels along x, and so on), and the rest place of each child (a
child's `t` is multiplied by its parent's scale componentwise, in the parent's frame where `t`
already lives). A child's own volume, its rotation and its scale are its own: a squashed body
carries its eyes down with it, and the eyes keep their shape unless their own key squashes them.
Every part's model-to-world map is therefore a rotation times a diagonal over the detail, with
no shear anywhere in the tree. A key's scale replaces the rest scale, as a key's rotation does
(Plan 0063's amendment of 2026-11-11); the rest scale is one on every part, and the `.rig`
grammar states a scale on a `key` only.

## Consequences

### Positive
- Squash and stretch for a slime, a breathing chest, a blinking eye or a recoiling tentacle,
  from the parts a model already has, with no new volume in the pool and no new instance word:
  the instance's affine maps carry it.
- Face normals on a scaled part stay the part's rotated axes, so the light pass and the shadow
  rays are untouched; the hit record's normal path already applies the inverse transpose, which
  gives the same direction.
- The headless query stays exact: the ray carried into a scaled part is still an integer line,
  the crossings are still fractions compared by cross-multiplication, and at a scale of one
  every result is bit-identical to the unscaled walk, so the committed vectors stand.
- A child's `t` scaled in its parent's frame is one multiply per axis, rounded in the same
  division as the rotation, so the composition stays one pass and exact.

### Negative
- **A child does not stretch with its parent.** An arm on a body stretched tall does not grow;
  the author keys the arm's scale (or its `move`) too. Full inheritance would do it for free
  and bring shear with it (Alternative B).
- A key's scale restates the whole scale, not a change to it; there is no additive scale key.
- The query's `i128` budget shrinks: the scale multiplies both sides of the walk's fractions.
  Plan 0082 either reduces the shared factor or narrows the documented domain of origins and
  directions, and writes the bound in the module doc.
- Scale is not a soft body. A blob that bends round a corner, splits or merges is a different
  representation (Alternative C), parked in `docs/backlog.md`.

## Alternatives considered

### Alternative A: a scale that does not inherit at all
Each part scales its own volume about its pivot and nothing else. The simplest rule and no
shear, but a squashed body leaves its eyes hanging where they were, and every squash key then
needs a `move` on every child. It lost because it makes the common case (a body with features
on it) the most typing.

### Alternative B: a scale inherited down the tree, as a full affine
The parent's scale multiplies the child's whole map, volume included, as scene graphs do. A
stretched body stretches its arms for free. It lost on shear: a non-uniform scale under a
rotated child makes the child's map a general affine, so its normals need an inverse transpose
of a non-orthogonal matrix, the renderer's cheap inverse goes, the headless walk's fraction
units differ per axis in a frame that is no longer the part's own grid, and the shape of a
rotated child under a squashed parent is a parallelepiped nobody keyed.

### Alternative C: a deformation field or vertex-style soft body
Move the voxels themselves: a lattice or a field over the part's volume, sampled by the ray.
That is what a real jelly needs (bend, split, merge), and it is out of reach here: the pool
stores the volume's bits once per model, the ray walks a regular grid, and the query's
determinism rests on that grid. It is the later ADR the backlog names, not a rejected shape of
this one.

## Outcome

_(Added at acceptance if Plan 0082 falsifies something above.)_
