# ADR-0008: Collapse as a capped support flood on the coarse grid and rigid vertical drop ops, not a rigid-body engine

> **Status:** proposed
> **Date:** 2026-10-16
> **Related plan(s):** [Plan 0009](../plans/0009-support-query-and-collapse-ops.md)

## Context

The owner decided on 2026-10-16 (`docs/roadmap.md`, "Physics") that collapse is emergent and
authoritative: digging out the supports brings the ceiling down, on the server, bit-identically
on every machine. Fall damage is a controller event (Plan 0004). Debris may be animated on the
client by a rigid-body library as a cosmetic layer. The first consumer's requirements say
"scripted collapse, no general structural physics"; the owner's decision supersedes that and
the consumer's document is to follow.

What exists or is planned: edits return per-material counts and a touched region
(Plan 0001); the coarse grid of Plan 0004 keeps a solid count and a dominant material per
brick (0.8 m) and reports changed cells after each edit; the controller runs on the server
tick in integer math (ADR-0005). Teardown, the reference voxel destruction game, checks
connectivity after every break and turns disconnected pieces into dynamic bodies; the
authoritative question there is also "what is still attached".

Three forces shape the mechanism:

- **Determinism and cost per edit.** Whatever answers "what lost support" runs after every
  carve on a server tick, so its cost must be bounded and its result integer-exact.
- **Granularity.** The grid already summarises support per brick. A voxel-granular flood is
  512 times the work for a distinction (a 10 cm strand of rock holding up a slab) that no
  game wants to honour anyway.
- **Authoritative versus cosmetic.** A falling slab that blocks a corridor is gameplay and
  must be the same on server and client; tumbling rubble is not and need not be.

## Decision

`vxe-world` answers which solid regions lost their connection to an anchor with a flood over
6-connected load-bearing cells of the coarse grid (cells with `solid >= solid_min`), started
from the cells around an edit and capped at `max_region` cells. Reaching an anchor cell (a cell
whose dominant material is indestructible, a cell on the world extent's boundary when the
caller says so, an explicit anchor the game supplies, or a cell in a non-resident chunk, which
is unknown and therefore conservative) or exceeding the cap means supported; exhausting the
flood yields an unsupported region, reported as cells with per-material voxel counts and an
aabb, in the vocabulary of an edit result.

Two deterministic ops change the world. `drop_region` translates a region's voxels straight
down by the minimum air gap under its columns, up to a caller-given maximum, and writes them
back losslessly; called once per tick with a per-tick maximum, it is the authoritative fall.
`remove_region` carves the region and piles a caller-given fraction of each column's removed
volume as rubble on the first solid voxel below. The game chooses per region. Rotation,
tumbling, friction and stacking are not simulated in the authoritative world. A client may
animate pieces with a rigid-body library as a cosmetic layer that never writes to the world and
that nothing deterministic reads.

## Consequences

### Positive

- Integer, bounded per edit, deterministic on both arms; the output reuses the edit-result
  vocabulary and the ops replicate through the op log (Plan 0007) like any edit.
- The game decides what falls, what crumbles and what stays; the engine only reports and
  executes.
- A per-tick `max_drop` gives an animated fall from authoritative state with no second
  simulation.

### Negative

- A region larger than `max_region` never falls: the roof of a large cavern is "supported" by
  its size. The cap is a parameter the game sets per scale, and it is a documented limit, not a
  bug to be fixed later.
- A brick with fewer than `solid_min` solid voxels is not load-bearing, so a thin sheet of rock
  holds nothing up, and a thin ledge under a slab reads as unsupported.
- Regions are whole bricks, so a piece's cut is 0.8 m blocky where the flood separated it.
  Voxels left in cells below the threshold stay where they are; the game sweeps them if it
  wants to.
- No horizontal motion: a leaning slab drops straight. A region whose columns are all
  obstructed drops zero and stays, which a game may read as "wedged".
- The cosmetic layer, when it exists, can show pieces where the world has none for a few
  hundred milliseconds, and the two will not match exactly.

## Alternatives considered

### Alternative A: A rigid-body engine (`rapier`) in the authoritative world

Pieces as rigid bodies with the world as a collider. Rejected because the simulation is
floating point and not bit-identical across platforms without controls we do not own, because
the world would need a collider representation rebuilt on every edit (the reason ADR-0005
rejected it for the controller), and because the result would differ between server and client
exactly where it matters.

### Alternative B: Scripted-only collapse

The game names a region to drop; the engine never asks what is supported. Rejected by the
owner's decision: emergent collapse is the point, and the support flood is the small part of
the work.

### Alternative C: Exact incremental connectivity

Maintain connected components of the solid cell graph under deletions (dynamic connectivity)
so every edit reports exactly what detached, with no cap. Rejected because deletions are the
hard case of dynamic connectivity, the bookkeeping spans chunks and streaming, and the capped
flood is a few hundred lines with a cost bound the server tick can live with.

### Alternative D: Voxel-granular support

Flood over voxels instead of cells. Rejected for cost: 512 times the cells, for a
distinction no consumer asked for, when the cell's solid count already captures how much of a
brick is there.

## Outcome

_(Added at acceptance if Plan 0009 falsifies something above.)_
