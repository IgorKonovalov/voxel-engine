# ADR-0069: A posed entity enters the world as a baked prefab, clipped to air on the server, through the stamp op

> **Status:** proposed
> **Date:** 2026-11-16
> **Related plan(s):** [Plan 0085](../plans/0085-entity-variants-holding-clips-and-an-entity-stamped-into-the-world.md)

## Context

The owner wants a dead entity to become part of the map (2026-11-16): diggable, collidable, lit
by the baked light, costing no entity slot and no entity trace, which ties in with the wish that
destructible structures are part of the map. An entity is a tree of rigid parts in arbitrary
Q14 rotations at a model voxel of 1, 1/2 or 1/4 of a world voxel (ADR-0050); the world is a
brick map on one grid whose only volume edit is `World::stamp` of a `Prefab` at a whole-voxel
origin in one of the 24 axis rotations (Plan 0008). So the entity has to be **resampled** onto
the world grid first, and that resampling runs on the server, where the hit test runs, so it
must be integer arithmetic that gives the same prefab on Linux and Windows.

Three facts shape how the result reaches the world. Every edit is an op in the log (ADR-0007):
a stamp op names its prefab by 64-bit content id, the server resolves the id in a registry the
application filled, and a save carries the content of every prefab the log names (ADR-0014). A
stamp writes every masked voxel of its prefab whatever is there (`stamp.rs`); there is no
"only into air" mode and no mode byte in op kind 2's payload. And a rig-built part volume
writes air across its whole box when stamped (its empty cells are written `AIR`, not skipped),
so the resampled prefab must carry a mask of its own.

## Decision

`vxe-entity` gains a pure, deterministic **bake**: given a model, a pose and a variant it
returns a `Prefab` on the world grid and its origin, with a mask set only where the posed
model is solid, built by integer sampling of each drawn part's chosen volume at the sub-centres
of every world voxel in the part's world box (one sample at detail 1, eight at 2, sixty-four
at 4; the rule for how many make a voxel solid is a parameter the owner's window picks).
Overlapping parts resolve by part order, materials pass through the variant's remap, and a
hidden part bakes nothing. On the server, before registration, the consumer **clips the bake
to air**: `Baked::clip_to_air(&World)` drops every voxel whose world cell is solid, so a
corpse lies on the floor instead of replacing it. The consumer then registers the prefab (its
content id) and submits an ordinary **stamp op** (kind 2) at the bake's origin with the
identity rotation, through the server, the log and the host as every stamp goes, and removes
the entity. No wire format, op kind or save format changes.

## Consequences

### Positive
- Everything downstream exists: the op log, the save's prefab table, the chunk dirtying, the
  live bake of the light store, the coarse grid and the collision all see a stamp. A charred,
  emissive corpse lights its corner through the baked light at no list-light cost.
- The bake is a pure function of model, pose and variant, so it joins the fixed vectors
  (`docs/nfr.md` section 6) and a replayed log reproduces the corpse bit for bit.
- Clipping to air on the server keeps the world's authority over its rock: the op writes only
  cells that were air when the server decided, and the replay writes the same cells because the
  prefab it names is the clipped one.
- Any variant can be stamped, so a game can leave only a part behind (every other part hidden).

### Negative
- **Saves grow by a prefab per stamped entity.** Unlike an asset stamped many times, every
  corpse is its own content: the posed spider's box is tens of thousands of voxels, so each is
  about 100 to 200 KB of save (Plan 0085 reads the figure). A level with many corpses carries
  megabytes; the save's prefab table was built for a few assets.
- **A remote client must hold the prefab before the op arrives** (ADR-0014): until M9 ships
  prefab content over the channel, a stamped entity is a desync for an owned client that did
  not bake it. The shared client of solo play is unaffected.
- **The bake is a one-off cost on the server tick** (a guess of 10 to 20 ms for the spider on
  the dev box; Plan 0085 reads it). A consumer with many deaths in one tick spreads them.
- The resampled corpse is a stepped copy of the model: a tilted leg becomes a staircase, and a
  feature thinner than a world voxel survives or vanishes by the solid rule.
- The stamp's chunks reach the GPU through the mirror's sync, one frame or more after the
  entity leaves the set; the window judges the blink and Plan 0085 reads it.

## Alternatives considered

### Alternative A: a new op kind carrying the model id, the pose and the variant
The server and every client re-bake at apply time; the op is about 450 B whatever the model's
size, and the save carries no corpse content. It lost for now because it needs a model registry
beside the prefab registry, a model table in the save header (parts, pivots, details, their
prefabs), a new wire kind in `vxe-world`, `vxe-net` and the host, and a bake on every peer. The
bake's determinism keeps this door open; the save-growth reading says when to walk through it
(M9, with the content channel).

### Alternative B: an "only into air" mode on the stamp op
A mode byte in op kind 2's payload (or a kind 7) and `World::stamp_into_air`. It is more general
than a clip on the server (generation-time stamps could use it) but it changes the wire and the
log's decoding for a rule the consumer can apply before the op, and the replay then depends on
the world state at apply time rather than on the prefab's bytes alone. It lost on the wire
change; a later plan may still want it for generation.

### Alternative C: stamp the bake as it is, overwriting rock
No clipping. The simplest. It lost because a corpse whose legs replace floor voxels changes the
floor's material under it, which a dig then removes as "shell", and because a stamp into a wall
would carve the wall's shape out of the corpse's box only where the mask is set, leaving the
corpse embedded.

### Alternative D: keep the dead entity as an entity
No bake, no op. It lost because the owner's ask is a corpse that is part of the map: diggable,
collidable, baked-lit, and free of the trace that every primary and shadow ray pays per entity
(Plan 0062 Phase 5's readings).

## Outcome

_(Added at acceptance if Plan 0085 falsifies something above.)_
