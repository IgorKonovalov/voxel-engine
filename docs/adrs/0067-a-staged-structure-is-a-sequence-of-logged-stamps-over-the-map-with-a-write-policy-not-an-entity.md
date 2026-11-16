# ADR-0067: A staged structure is a sequence of logged stamps over the map with a write policy, not an entity

> **Status:** proposed
> **Date:** 2026-11-16
> **Related plan(s):** [Plan 0084](../plans/0084-staged-structures-stages-from-a-vox-a-stamp-with-a-write-policy-and-a-region-a-reveal-and-a-demolish.md)

## Context

The owner (2026-11-16) wants "model entities with several different levels, e.g. a castle
being built: first the foundation, then walls, then a tower in progress", and "all of that we
should be able to destroy live, so this maybe not an entity by itself but part of the map".

The two representations the engine has are far apart. An **entity** (ADR-0050) is up to 16
rigid parts of at most 64 voxels a side in a pool of its own, traced after the world march,
posed in fixed point; nothing digs it, nothing collides with it but the ray query, the baked
light does not see it, and a castle of 600 voxels a side does not fit its caps. **The world**
is dug by carve ops that report what they removed, collides through `vxe-sim`, takes the baked
light per chunk, collapses through the support query, and every change to it is a logged
deterministic op (ADR-0007, ADR-0014). Edits are operations, not diffs (`CLAUDE.md`).

A stamp today writes every masked voxel whatever stands there: it is an authoring operation,
so a shell can replace anything. A later stage applied over a structure the player has
damaged, dug or piled rubble against must decide what it does to those voxels, and the engine
should not decide that for the game.

## Decision

**A staged structure is part of the map: an ordered list of stages, each a prefab, applied as
logged stamps that carry a write policy and an optional region. The engine offers the stamp,
the policy, the region and a pure planner that turns a stage into a sequence of ops; the game
owns when a stage advances. Entities stay rigid and whole; a moving part of a building (a
drawbridge, a crane) is an entity.**

- **The op.** `Op::Stamp` gains `policy` and `region`, wire kind 7 (kind 2 stays the whole
  prefab, `Overwrite`). `policy` is `Overwrite` (today's), `IntoAir` (a masked voxel is written
  only where the world holds air), or `Erase` (a masked voxel is set to air only where the
  world holds the prefab's material there). `region` is an inclusive voxel box in the prefab's
  rotated local frame; only masked voxels inside it are considered. The result reports, per
  old material, what changed, and how many masked voxels the policy skipped.
- **The stages.** `vxe-assets` reads a structure's stages from its `.vox`: one stage per layer
  in layer order (MagicaVoxel `LAYR`, the nodes' `_layer_id`), or per top-level group named
  `stage <n> ...`. A stage is the prefabs and placements of its models. The engine has no
  "structure" type with state: the game keeps the structure's cursor (which stage, how far).
- **The planner.** `vxe_world::reveal(stage, slab)` is a pure function: the ops that stamp a
  stage slab by slab along an axis (`region` boxes of `slab` voxels), in order, with a policy.
  Demolition is the same ops with `Erase`, in reverse. What falls afterwards is the game's
  support query and drop ops (ADR-0008).

## Consequences

### Positive
- A structure under construction is dug, collided with, lit and saved like any wall: no second
  world. A replay of the log rebuilds it stage by stage.
- The policy makes the engine's answer to "a stage over a dug hole" the game's choice:
  `IntoAir` leaves the player's hole and their rubble, `Overwrite` restores the design.
- A reveal is `n` small ops a tick, not an engine-side clock; the server's tick order is kept
  (ADR-0007), and the log grows by about 60 bytes per slab.
- Erase removes only what the structure put there, so a player's wall through the footprint
  survives a demolition.

### Negative
- The log is the structure's state; a 40 m stage revealed a voxel-layer a tick is 400 ops.
  Acceptable (34 bytes a stamp op today, about 60 with a region); a game wanting fewer ops
  uses thicker slabs.
- `IntoAir` cannot restore a damaged earlier stage; a game that wants repair applies the
  earlier stage again with `Overwrite` over its own materials only, which this ADR does not
  offer as a policy. If a consumer asks, `IntoAirOrOwn(materials)` is a fourth arm.
- Layers as stages is a MagicaVoxel convention this ADR adopts; a `.vox` from another tool
  with no layers is one stage.
- A policy is a per-voxel branch in the blit; the stamp bench reads the cost.

## Alternatives considered

### Alternative A: a staged structure is an entity with several models

Swap the entity's model as it is built, move it, pose it. Rejected: entities are not dug, do
not collide beyond the ray query, do not take the baked light, and are capped at 16 parts of
64 voxels; a castle fits none of those, and the owner's own reading ("part of the map") is
right.

### Alternative B: snapshots and diffs

Store each stage as a diff of the world and apply or revert diffs. Rejected: edits are
operations with results, not diffs (`CLAUDE.md`), and a diff recorded against one world state
is wrong against a dug one; the policy on a stamp answers the same question deterministically.

### Alternative C: an engine-side construction scheduler

The world advances a structure over ticks by itself. Rejected: the engine has no clock of its
own (ADR-0007: the game's tick drives ops), and "building a castle" is gameplay; the pure
planner gives the game the ops and keeps the engine out of the decision.
