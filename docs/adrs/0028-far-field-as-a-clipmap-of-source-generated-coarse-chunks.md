# ADR-0028: The far field is a clipmap of coarse chunk levels that the source generates directly, with edits folded in from full detail

> **Status:** proposed
> **Date:** 2026-10-22
> **Related plan(s):** [Plan 0033](../plans/0033-far-field-clipmap-to-a-kilometre.md)
> **Claims:** the founding-interview decision "Far-field clipmap (coarser rings beyond the
> full-detail radius)" listed in the [ADR index](README.md) under "Decisions taken but not
> yet recorded", and settles ADR-0003's open Negative ("a `WorldSource` method that produces
> mips directly is the far-field clipmap's job").

## Context

The owner set the outdoor view range at about 1 km (2026-10-22). Today the world is drawn to
`r_lod` chunks: the full-detail sphere (`r_full` 16, 102 m) and ADR-0003's LOD-only ring to
24 chunks (154 m), whose chunks are generated at full detail and downsampled to three mip
levels. The roadmap's far-field note does the arithmetic that rules out stretching that ring:
a 1 km ball is about 30 million chunks, and generating them at full detail to downsample is a
thousand times the near sphere's work. The near sphere's leading face alone is about 1245
chunks a second at 10 m/s, 3.3e8 voxels a second (disc(16) = 797 chunks per 6.4 m step), and
`nfr.md` §5 reads 6.95e7 voxels a second per worker under streaming on the dev box.

Three facts constrain the shape. **The screen sets the resolution needed:** at 1080p and the
fly-through's 1-radian vertical field of view a pixel subtends 1.01 mrad, so a 10 cm voxel is
one pixel at 99 m and a cell of `2^k` voxels is one pixel at `99 x 2^k` m. A level that
doubles its cell size each time it doubles its distance holds a constant one to two pixels
per cell. **The marcher already walks scaled levels:** a LOD-only chunk is walked at mip
level L with every skip scaled by `2^L` (`march.wgsl`), over the same `Brick` type, pool,
record encoding and masks. **Outdoor worlds are mostly uniform at kilometre scale:** a ball of
1 km holds terrain in a thin slab; nearly every coarse chunk is all air or all rock, so a
coarse chunk that is one material should cost a table entry, not a record or a generation
pass.

Edits happen only in full chunks (ADR-0003), but a dug crater must stay visible once the
camera leaves it, after its chunk is evicted, and after a far chunk is regenerated from the
source. The existing persistence keeps edited chunks exactly; the far field needs a cheap,
bounded way to see them.

## Decision

**The far field is a clipmap of up to three coarse levels beyond the full-detail sphere.**
Level `k` (1 to 3) is a grid of *clip chunks* of 64³ cells, each cell `2^k` voxels on an edge,
so a level-`k` clip chunk spans `6.4 x 2^k` m and has exactly the layout of a chunk: 8³ bricks
of 8³ cells, `Brick` values, the packed format, a chunk record in the mirror. Each level is
resident as a sphere of `r_clip` clip chunks (default 22) around the focus's level-`k` chunk
with ADR-0003's integer rule and hysteresis, and is mirrored as its own toroidally wrapped
table window of edge `2 (r_clip + h) + 1` at the tail of the chunk table. A level holds its
whole ball, the part under the finer levels included, so a missing finer chunk falls back to
coarser data instead of a hole. A clip chunk that is one material is a table entry with no
record.

**The source generates coarse chunks directly.** `WorldSource` gains a provided method,
`coarse_chunk(at, out) -> Coarse`, that fills a level's cells or answers `Uniform(material)`
for the whole chunk; its default answers `Unsupported`, and a source that does not implement
it has no far field (the cave keeps ADR-0003's ring). The contract is the **half rule**: a cell
is solid when at least half its volume is solid, and it takes the material a viewer sees on it:
the most frequent material among its exposed solid children (those with an air face
neighbour), or among all its solid children when none is exposed, ties to the lowest id. A
source approximates the rule as it sees fit (sampling the cell centre for occupancy and giving
the column's surface cell its surface material is the expected implementation), and may leave
out interiors no viewer outside can see. The engine's own coarse cells (`ChunkMips` built under
`MipRule::Half`) follow the same rule, so a coarse surface sits within half a cell of the fine
one instead of a cell outside it, and a thin surface layer (grass over soil) keeps its colour
at distance.

**Edits fold in from full detail.** After an edit, the touched full chunks' half-rule mips
(levels 1 to 3, the three `ChunkMips` levels) are rebuilt on a stream worker and kept in an
*edit overlay* keyed by chunk; a clip chunk is the source's coarse cells with every overlay
entry inside its footprint written over them, in chunk-coordinate order (footprints are
disjoint, so the order never matters). Resident clip chunks are patched when an overlay entry
changes; a regenerated clip chunk takes the overlay again. Overlay entries live as long as
their chunk's persistence entry, so the far field shows every edit the near field would.

**The marcher picks a level per point, never finer along a ray.** A ray's walk level at a
point `p` is the larger of the level so far and the level `|p - camera|` falls in; level `k`
takes over at `s_k = (r_(k-1) - 2) x 64 x 2^(k-1)` voxels (with `r_0 = r_full` and
`r_k = r_clip`), inside the distance every finer chunk is guaranteed resident, moved closer by
up to one level-`(k-1)` chunk with a per-pixel blue-noise offset when the temporal upscaler
runs, so the seam is a dithered band the upscaler resolves. An absent chunk also sends the walk
one level coarser at the same point; only at the coarsest level does an absent chunk inside the
ball count as an occluder for a lighting ray (ADR-0027's rule, moved outward). The directional
shadow ray toward the sun or moon (ADR-0027) starts at its hit's level, offset by half a cell of
that level, and follows the same rule out to the far limit, where it reaches the sky. When the
far field is off (no clip levels), the walk is today's.

## Consequences

### Positive

- **About 1 km at one to two pixels per cell, with three levels.** At `r_full` 16 and
  `r_clip` 22 the levels take over at 83 to 90 m, 243 to 256 m and 486 to 512 m, and level 3
  is guaranteed to 1024 m. A cell measures 2.4, 1.6 and 1.6 pixels where its level starts at
  1080p and 0.77 at its far edge (Plan 0033's arithmetic).
- **Nothing new to walk or store.** A clip chunk is a chunk: the packed brick, Plan 0028's
  slots and bins, the records, ADR-0013's reservation and pressure, and the marcher's scaled
  walk all apply. The new parts are the per-level tables, the level rule and the overlay.
- **Generation scales with the surface, not the volume.** Naively, the three levels' leading
  faces at 10 m/s are 2074 clip chunks a second, 5.4e8 cells, more than the near sphere's
  3.3e8. A source that answers `Uniform` for the clip chunks its terrain bounds clear pays
  per-cell work only on the few percent the surface crosses (Plan 0033 measures the share).
- **Edits stay visible everywhere and the far field stays deterministic.** A clip chunk is a
  pure function of the source, the level, the coordinate and the overlay; the overlay is a
  pure function of edited voxels. Both are integer and thread-order independent.
- **Every ray type gets the far field.** Shadow, probe and reflection rays go through the
  same `march`, so a distant ridge shadows a near valley without a second structure.
- **The pixel footprint's fallback is free:** a level holds its whole ball, so late finer data
  shows coarser geometry, never sky, and a ring boundary moves continuously with the camera.

### Negative

- **Each level also stores the ball its finer level draws.** That inner region is about
  `(r_clip / 2)^3 / r_clip^3`, one eighth, of each level's ball; outdoors it is mostly
  table-only uniform chunks, but a cave-like source would pay it in records and slots.
- **Two coarse rules in one engine.** ADR-0003's LOD-only mips keep the conservative
  any-solid rule; the far field uses the half rule. A source with no coarse path keeps the
  ring; with the far field on, the application sets `r_lod = r_full` and the ring is empty.
  Retiring the ring is a later decision, by measurement.
- **The material rule is a heuristic.** Where the half rule turns a cell holding the exposed
  layer into air, the solid cell under it has no exposed children and takes its interior
  material: a thin grass layer on a steep slope shows soil-coloured cells at distance.
- **The half rule loses thin things.** A feature under half a cell thick vanishes at the
  level that holds it: a one-voxel pole is gone beyond about 83 m. At one to two pixels per
  cell that is near the pixel limit, but at the level 1 start it is 2.4 pixels at 1080p and
  about 3.2 at 1440p, which the owner judges on screen.
- **The source's coarse cells and the engine's overlay can disagree.** Where an edited chunk
  meets unedited ones in a clip chunk, the overlay's downsample of real voxels sits beside the
  source's approximation; a source whose coarse path is not close to the half rule shows a
  seam around every edited chunk at distance.
- **The seam is a band only with the upscaler.** Without it the switch is a hard line at a
  fixed distance, up to half a coarse cell of surface offset, which shows on grazing terrain.
- **Coarse geometry is invisible to the CPU.** Raycasts, queries and collision see full chunks
  only, as with the ring today.
- **More steps per ray.** A ray that reaches 1 km crosses about 50 chunk-sized cells of air
  across the four windows before it leaves; a per-level bound on the non-empty chunks lets a
  ray that leaves every remaining level's bound end early, but grazing rays along terrain pay
  for every level they cross.

## Alternatives considered

### Alternative A: a heightfield beyond the full-detail radius

A 2D height and material map rendered by ray marching a mip-mapped heightfield is the classic
terrain far field and costs a few megabytes for kilometres. It lost because it is a second
representation with a second walk, a seam between voxels and height samples at every distance,
and a second edit path; and because it cannot hold an overhang, an arch, a cave mouth, a
floating structure or a world that is not terrain, which an engine for general consumers must
not assume away. The clipmap keeps one representation at every distance.

### Alternative B: a sparse voxel octree (ESVO) or DAG for the far field

An octree or a DAG compresses kilometres of uniform space well and has a published GPU
traversal (Laine and Karras, "Efficient Sparse Voxel Octrees"). It lost on fit: the brick map
already compresses uniform space through uniform bricks and the table-only uniform chunks of
this decision, and an octree would add a second traversal, a build step on every streamed
region, and an edit path that rebuilds paths to the root (ADR-0012 rejected compressed DAGs
for editing for the same reason). The clipmap reuses the walk the near field uses.

### Alternative C: impostors or a rendered horizon

Rendering distant terrain into view-dependent cards or a horizon panorama, refreshed as the
camera moves, is cheap per frame. It lost because it is not geometry: a sun shadow ray, a probe
ray or a reflection ray cannot trace it, parallax is wrong between refreshes, and an edit or a
time-of-day change re-renders it. The owner's sun shadows (ADR-0027) need occluders a ray can
hit.

### Alternative D: build the far field by downsampling full-detail generation

The path ADR-0003's ring takes. A level-`k` clip chunk covers `8^k` world chunks: 8 at level 1
(about 2.1e6 voxels, some 30 ms of one worker at the measured rate), 512 at level 3 (1.3e8
voxels, about 2 s). At 10 m/s the level-1 face alone needs 1185 clip chunks a second, about
35 worker-seconds a second. Downsampling is the right answer only where full voxels already
exist, which is what the edit overlay does.

## Outcome

_(Added only at acceptance if implementation falsified something above. Dated. Never rewrite the
body.)_
