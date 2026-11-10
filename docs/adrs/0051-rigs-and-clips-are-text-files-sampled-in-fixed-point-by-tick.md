# ADR-0051: Rigs and clips are small text files, and a clip is sampled in fixed point by tick

> **Status:** proposed
> **Date:** 2026-11-10
> **Related plan(s):** [Plan 0063](../plans/0063-rigs-and-clips-keyframe-animation-a-model-viewer-and-text-authoring.md)

## Context

The owner wants the engine to play keyframe clips on the entities of ADR-0050, and wants to make
a model alone or with an agent's help, without the engine growing an editor (`docs/roadmap.md`:
no scene editor). The voxels of a model already have an editor and a format: MagicaVoxel and
`.vox`, which `vxe-assets` reads and writes. What `.vox` lacks is a rig: it has no pivots, and
its node rotations are the 24 axis rotations, so it cannot hold a limb's swing.

A pose feeds the server's hit test (ADR-0050), so sampling a clip must be deterministic: no
wall clock and no float. The workspace has no serialisation dependency, and dependencies are a
cost.

## Decision

A model's rig and its clips are one **`.rig` text file** beside its `.vox`, line-based, parsed
by hand in `vxe-assets` with no new dependency. It names the `.vox`, and for each part the
`.vox` node it takes its voxels from, its parent, its pivot and its detail; it maps palette
indices to material names; and it holds clips as keys of `(tick, part, translation, rotation)`,
rotations written as an axis and whole degrees. Parsing turns degrees into Q14 quaternions
through a committed table of half-degree sines, not through the platform's `sin`.

A clip is sampled by **integer tick** in `vxe-entity`: translation is linear in fixed point,
rotation is a normalised linear blend in Q14 along the shorter arc. `sample(clip, tick)` and
`blend(a, b, weight)` are pure functions; playback state is a struct the game owns. The renderer
interpolates between two ticks' poses in `f32` for display only.

## Consequences

### Positive
- A person or an agent writes and diffs a rig in any text editor; the fly-through's viewer
  reloads it on change (Plan 0063), which is the authoring loop.
- No dependency added. The same clip gives the same pose on server and client.
- Voxel authoring stays in MagicaVoxel.

### Negative
- **Writing keys as numbers is slow for a long or subtle animation.** There is no timeline and
  no curve editor; a walk cycle is a dozen typed keys.
- A normalised linear blend is not constant-speed across a wide key gap; keys more than 90
  degrees apart need an in-between key.
- A format of our own means nobody else's tools export it.
- The parser is ours to maintain and to fuzz.

## Alternatives considered

### Alternative A: glTF animation, authored in Blender
A real timeline and curve editor. It lost on cost: a parsing dependency, a float pipeline that
needs re-quantising, and a skinned-mesh workflow in which voxel parts are a foreign body. It
stays the route if typed keys prove too slow.

### Alternative B: `.vox` frame animation
No new file. It lost because its rotations are the 24 axis rotations and its frames replace
whole volumes, which multiplies pool memory by the frame count.

### Alternative C: clips in code only
The game builds `Clip` values itself, which Plan 0062's API already allows. It lost as the only
route because every tweak is then a rebuild, and the owner's flow is edit, reload, look.
