# ADR-0050: An entity is a set of posed voxel volumes in a pool of its own, traced after the world march, with poses in fixed point

> **Status:** proposed
> **Date:** 2026-11-10
> **Related plan(s):** [Plan 0062](../plans/0062-entities-voxel-models-on-rigid-parts-traced-lit-shadowed-and-queried.md)
> (builds it), [Plan 0063](../plans/0063-rigs-and-clips-keyframe-animation-a-model-viewer-and-text-authoring.md)
> (clips over it)

## Context

The founding interview settled that dynamic entities are voxel models on rigs of rigid parts,
traced in the same pipeline as the world (`docs/roadmap.md`, M6), and left the concrete shape to
the entities plan. The owner's answers of 2026-11-10 size it: about 64 entities and 512 parts on
screen, a headless ray query that names the entity, part and voxel a ray hits, emissive voxels
that glow, and light on the walls through a list light the game attaches (ADR-0044).

Three facts force choices. The world's GPU mirror is a windowed chunk table over a brick pool
that `sync` fills from dirty chunks (ADR-0002, ADR-0012): it has no notion of a rotated volume.
The consumer is server-authoritative, so a query that decides whether a shot hit a creature runs
on the server and must give the same answer on Linux and Windows. And every light path the
renderer has is steady because it is exact or baked: an entity moves every frame, so whatever
it touches (its own shading, the shadow it casts) cannot lean on a slow history.

## Decision

We describe an entity in a new headless, deterministic crate, `vxe-entity`, and mirror it in
`vxe-render`.

- **Model.** A model is up to 16 parts. A part is a `vxe_world::Prefab` volume of at most 64
  voxels a side, a parent part, a pivot, and a `detail` of 1, 2 or 4 (a model voxel is 1 /
  detail of a world voxel). Parts are rigid: no voxel moves inside its part.
- **Pose in fixed point.** An entity's root position is a `Fixed3`; a part's local translation
  is `i32` at 1/256 voxel and its rotation a unit quaternion in `i16` Q14. Composition down the
  hierarchy and the ray query are integer arithmetic, so the same pose and ray give bit-identical
  hits on every platform. The renderer converts the composed transforms to `f32`.
- **Ray query.** `EntitySet::raycast` transforms the ray into each candidate part's space and
  walks the prefab's grid. It returns the entity, the part, the model voxel, the face and the
  distance. The server calls it; the renderer does not.
- **GPU mirror.** Part volumes live in an **entity pool** (a storage buffer): per volume one
  occupancy bit a voxel, a mask of its 4-voxel blocks, an 8-bit palette index a voxel and a
  256-entry palette of material ids. Instances (one per part) carry the transform now and of
  the frame before. A binary bounding-volume tree over the entities' boxes, at most 127 nodes,
  is built on the CPU each frame; a leaf lists its entity's parts.
- **Tracing.** A ray marches the world first, then walks the tree with the world hit as its
  limit, and keeps the nearer hit. Primary rays and the list lights' shadow rays do this. Probe
  rays and bake rays never see an entity.
- **Light.** An entity hit is shaded by the list lights exactly, as a world hit is, and shows
  its material's emission. For the emissive light of the world it takes the fallback a hit with
  no cube face takes (ADR-0043). A pixel whose shadow ray crossed the box of an entity that
  moved takes this frame's direct sample as it is.

## Consequences

### Positive
- Free rotation and a model voxel finer than the world's, with no change to the world mirror,
  its sync, the op log or the bake.
- The server's hit test is deterministic and needs no GPU.
- With no entity in the frame the tree walk is skipped: the cost of the feature is paid only
  where entities are.
- The pool is small: a 16-voxel cube part is under 6 KiB, so hundreds of distinct parts fit in
  8 MiB.

### Negative
- **Every primary ray and every shadow ray walks a second structure.** The cost is unmeasured;
  Plan 0062 reads it at 0, 16 and 64 entities on both adapters. On the slow adapter it may
  exceed the headroom line of `docs/nfr.md` section 2, which the outdoor plans already claim.
- Fixed-point quaternions make the pose code longer and harder to read than `glam` would.
- The renderer's hit (`f32`) and the query's hit (integer) can disagree by a fraction of a
  voxel at a silhouette. The query is the authority; the picture is not.
- An entity's emissive voxels light nothing by themselves. A glowing eye that lights a wall
  costs the game one of ADR-0044's eight slots.
- The fallback's emissive estimate on a moving entity is a running mean and may show grain;
  Plan 0062's window judges it.
- Entities are invisible to the bounce channel and to the baked light: a creature casts no
  shadow in baked emissive light and blocks no bounce.

## Alternatives considered

### Alternative A: stamp entities into the world's bricks each frame
The world mirror would draw them for nothing. It lost because a stamp is an edit: it enters the
op log, dirties chunks, triggers the bake, and allows only the 24 axis rotations at the world's
voxel size.

### Alternative B: part volumes as chunks in the world's brick pool
One pool and one walk format. It lost because the chunk table is indexed by world chunk
coordinate inside a moving window and `sync` owns the pool's slots; a rotated volume needs a
second lookup path through the same walker anyway, and the pool's budget is the world's.

### Alternative C: a flat list of part boxes, no tree
Simplest to build and enough for 16 entities. It lost at the owner's count: 512 box tests on
every primary and shadow ray. The tree is built on the CPU from at most 64 boxes, which is
cheap, and a flat walk remains the tree with one leaf.

### Alternative D: floating-point poses and a render-only entity
Shorter code, and `glam` does the math. It lost because the consumer's server decides hits, and
a float pose composed on two platforms is not guaranteed bit-identical.
