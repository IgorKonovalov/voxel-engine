# ADR-0078: An entity part has a grain: a detail finer than a world voxel, or a coarse factor of world voxels per model voxel

> **Status:** accepted (Plan 0083 Phase 7b, written at the plan's close 2026-11-19; the
> implementation preceded the record, as the plan said it would)
> **Date:** 2026-11-19
> **Related plan(s):** [Plan 0083](../plans/done/0083-anything-from-a-vox-palette-materials-a-material-file-and-large-scenes-stamped-into-the-world.md)
> (Phase 7b); extends [ADR-0050](0050-entities-are-posed-voxel-volumes-in-their-own-pool-traced-after-the-world-march.md),
> whose Outcome points here.

## Context

ADR-0050 fixes an entity part at most `MAX_EDGE` (64) model voxels a side and gives it a
`detail` of 1, 2 or 4 model voxels per world voxel, so a part spans at most 64 world voxels
(6.4 m). Plan 0083 places structures of 600 world voxels a side (the generated castle) from the
fly-through's belt, and the owner's window (session 83a, 2026-11-18) asked for a ghost that shows
what will stand there, not a frame alone: "something simplified at least to understand what it
is". The ghost is an entity instance (ADR-0074's mark, touching no world voxel), so a 60 m body
has to fit one part.

Every pose, hit and transform of `vxe-entity` is integer arithmetic (Q14 quaternions, 1/256
voxel positions, ADR-0050), and the server's ray query must stay bit-identical on Linux and
Windows; whatever makes a part larger has to enter that arithmetic as an integer factor and
leave the `coarse` 1 case untouched.

## Decision

**A part's grain is either a `detail` of 1, 2 or 4 model voxels per world voxel, as before, or a
`coarse` factor of 1 to 16 world voxels per model voxel (`MAX_COARSE`); never both.** `Part`
gains `coarse: u8`; a `coarse` over 1 needs `detail` 1 (`ModelError::BadCoarse`, as `BadDetail`
refuses a detail outside 1, 2, 4). `MAX_PARTS` and `MAX_EDGE` stay, in model voxels, so a part
spans at most 1024 world voxels, the largest box a stamp takes (`edit_axis_max`). The factor
enters exactly where `detail` enters, as its inverse, in integers: `ComposedPose::part_point`
multiplies by `coarse` before its one rounding division, `part_reach` scales by it, and
`part_raycast` carries a local coordinate in units of `1 / (256 x 16384 x coarse)` model voxel
so every quantity stays an integer; the overflow bound in the raycast's header was re-read and
does not move (the coarse factor's four bits land on terms well under the world terms). The
coarse byte follows the detail byte in `Model::id`. In `vxe-render` the grain is one float,
`detail / coarse`, taken where `detail` was; instance word 27 holds its inverse (the shadow
offset's scale); the shaders carry the scale in the part transforms already and changed only
in comments. At `coarse` 1 every pose, hit, transform and instance word is bit-identical to
before, which the fixed-vector tests and the entity goldens hold.

## Consequences

### Positive
- A structure of any size a stamp accepts has a one-part coarse body (the castle at `coarse`
  10: 60 x 60 x 50 model voxels, 204 KB in the pool), standing at world scale inside its marked
  frame; the owner's window 83c read it as the castle.
- Nothing else of ADR-0050 moves: the caps, the pool, the tree, the mark and the tracing order
  are as they were, and a `coarse` 1 model is the model before.
- The same grain serves a far entity drawn coarse later (the backlog's distance tiers), without
  a second model type.

### Negative
- A coarse body lies about detail: one model voxel is a cube of `k` world voxels, so a thin
  feature under `k` voxels is gone or a block. It is a ghost's mechanism; a full-detail preview
  stays out (the backlog).
- `Model::id` moved for every model (the new byte); nothing pins or persists a model id, so
  nothing broke, but a consumer that had cached ids across versions would have to rebuild.
- `vxe-render` derives word 27 and the inverse map from the one f32 grain, which is off by an
  ulp for a few factors (7, 13, 14, 15); visually nothing, noted in the backlog as a one-line
  follow-up.

## Alternatives considered

### Alternative A: the structure itself as an entity
Raise `MAX_EDGE` or let a model have hundreds of parts, so the ghost is the structure at full
detail. The castle is about 800 parts at `MAX_EDGE`; the caps bound every entity's pool share
and every ray's walk, so raising them for one ghost costs every entity, every frame. Lost on
cost.

### Alternative B: a maquette at `detail` 1 inside the frame
Draw the coarse volume as a plain `detail` 1 model: a 6 m model of a 60 m castle inside its
frame. No engine change, but it lies about scale exactly where the owner wants to see what will
stand there; the frame then says "this big" and the model "this shape" and the eye has to
compose them. Lost on the window's purpose.

### Alternative C: a temporary stamp in a ghost row
Stamp the structure into the world in a ghost material and remove it at the click or on leaving
the mode. It writes voxels the bake, the collision, the census and the op log see; ADR-0074's
ghost touches nothing, and a logged write-then-unwrite pair per re-stand is not an edit anyone
asked for. Lost on the non-negotiable that edits are operations with results.

Two more fell at once: Plan 0082's pose scale (a pose property that animates, capped at 4, not
merged when the ghost was needed) and the overlay drawing each placement's box (the overlay is
2-D text and tiles, ADR-0075, and the frame is a wire box already).
