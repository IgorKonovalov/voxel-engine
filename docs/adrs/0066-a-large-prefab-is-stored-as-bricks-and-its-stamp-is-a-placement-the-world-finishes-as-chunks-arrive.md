# ADR-0066: A large prefab is stored as bricks, and its stamp is a placement the world finishes as chunks arrive

> **Status:** accepted (Plan 0083, 2026-11-19, with an Outcome)
> **Date:** 2026-11-16
> **Related plan(s):** [Plan 0083](../plans/done/0083-anything-from-a-vox-palette-materials-a-material-file-and-large-scenes-stamped-into-the-world.md)

## Context

A `vxe_world::Prefab` is a dense box: 2 B of material per voxel plus one mask bit
(`crates/vxe-world/src/prefab.rs`). A MagicaVoxel model is at most 256 a side
(`MAX_MODEL_EDGE`), so one model is up to 16 777 216 voxels, 35 651 584 B dense. Plan 0042's
castle, 21 kept tiles, held 343 566 464 B of prefabs, about 92 % air, and the plan left sparse
storage as a followup. The owner now wants to "import nearly anything that can be created as a
.vox file", large architecture first; a hundred tiles dense is 3.6 GB, near the whole 4 GB
world-in-RAM budget (`docs/nfr.md` section 3) before a chunk is loaded. A save carries every
prefab its stamps name (ADR-0014), so a dense castle stamped at runtime puts 365 MiB into the
save.

`World::stamp` writes resident chunks only: it fails, changing nothing, with the first chunk
that holds a masked voxel and is not resident. Residency is a sphere of chunks around the
camera (ADR-0003; `r_full` 16 chunks, about 100 m, the default), so a structure wider than the
window, or placed at a distance, cannot be stamped at runtime at all. Plan 0042 avoided this by
placing the castle at generation time through `StampedSource`, which blits each placement into
a chunk as the source generates it; that path is not an op, is not logged, and cannot be used
after the world exists.

The world's bricks are 8³ (ADR-0002), a chunk 64³ = 512 bricks, and a uniform brick is one
word. A stamp today writes voxel by voxel inside each chunk (`blit`).

## Decision

**A prefab is stored as 8³ bricks, a uniform brick collapsed to one word, and a stamp whose
box reaches a chunk that is not resident is kept by the world as a placement, applied to that
chunk when it is generated or reloaded.**

- **Storage.** `Prefab` holds its size and a brick grid (`ceil(size / 8)` on each axis); a
  brick is `Uniform { material, written }` (every voxel the same material and the same mask
  bit, including "all air, not written") or `Full` (512 ids and a 512-bit mask). Its content
  id stays the hash of the dense canonical bytes (size, voxels, mask), computed brick by brick
  without materialising the dense form, so every id, the stamp vector
  `0x7ee7_eeb7_0956_d069` and ADR-0014's table lookups are unchanged. Rotation permutes
  bricks and the voxels inside them. The save's prefab table and the wire encode the brick
  form (`LOG_FORMAT` 3; format 2 still decodes).
- **Placement.** `World::stamp` writes every resident chunk its box meets now and records a
  `Placement { prefab: Arc<Prefab>, origin, rot, seq }` for the chunks it could not write.
  When the streamer generates a chunk, every placement whose box meets it is blitted after the
  source, in `seq` order, as `StampedSource` does today. A spilled chunk (ADR-0003's edit
  persistence) records the `seq` it last saw; on reload, the placements after it are blitted.
  The op stays kind 2 (prefab id, origin, rotation; ADR-0014): the log, the server and the
  replay change nothing, and a replay recreates the placements in order. A placement is
  dropped once every chunk its box meets has been written at least once; the world reports
  the count of pending placements.
- **The result.** `EditResult` reports what the resident write changed, as today, and the
  stamp's `aabb` is the whole box; the counts for chunks written later are not reported (they
  are generation, not an edit, as in `StampedSource`).

## Consequences

### Positive
- A 256³ model that is 92 % air costs about a tenth of its dense bytes (a guess from the
  castle's air share; Plan 0083 reads it); a hundred-tile scene fits beside the world.
- A structure of any size, anywhere in the world, is one logged op; the far side appears as the
  camera arrives, the same on every client and in every replay.
- `StampedSource` becomes the generation-time face of the same placement list; one blit path.
- Brick-to-brick copies when the origin is brick-aligned, and a uniform written brick into a
  chunk is one word.

### Negative
- A chunk's generation now consults the placement list (a box test per placement; thousands
  are cheap, a hundred thousand are not; `MAX_PLACEMENTS` bounds one file, not a session).
- The spill format gains a sequence word; a spill from before this ADR reads as `seq` 0 and
  receives every placement, which is correct for worlds saved before any placement existed.
- The counts a stamp reports cover the resident part only. A game that wants the whole count
  reads it from the prefab (`written_count`) minus what the result reports as unchanged.
- A placement pins its prefab in memory until every chunk it touches has been generated once,
  which for a far placement in a streaming world may be the session's life.
- A `Full` brick costs 1088 B against a dense brick's 1024 + 64; the brick grid's index adds
  up to 4 B a brick (32 768 bricks, 128 KB, for a 256³ model). A prefab with no uniform bricks
  is about 1 % larger than today.

## Alternatives considered

### Alternative A: keep dense prefabs, refuse non-resident chunks (today)

No change. Rejected: 3.6 GB for a hundred tiles, and no runtime placement larger than the
residency window or farther than the camera's sphere, which is the owner's request.

### Alternative B: force the chunks resident, stamp, spill

Generate every non-resident chunk the box meets on the spot, write it, spill it to disk. One
code path (today's `stamp`) and whole counts. Rejected: the castle meets about a thousand
chunks, each generated and spilled synchronously (Plan 0042 read the castle doubling a chunk's
generation mean), so a far placement stalls the server tick for seconds and writes a thousand
spills for chunks nobody will visit; and a placement a kilometre away would load the
kilometre.

### Alternative C: a voxel list instead of bricks

Store the `.vox`'s own `XYZI` form (4 B a voxel, nothing for air). Cheapest for sparse models.
Rejected: a list cannot say "write air here" (the mask's explicit air, which the castle's
dungeon uses) without a second list; a blit must bin the list per chunk and per brick on every
stamp; and rotation is a pass over the list rather than a permutation. Bricks match the
world's layout, so the copy is the simplest one.

### Alternative D: an octree

Smaller still for large uniform regions. Rejected: more code than bricks for a gain only past
the brick level, and the world it is blitted into is bricks; a prefab octree would be
flattened to bricks at every stamp anyway.

## Outcome

2026-11-19, at Plan 0083's close (v0.66.0). The brick storage, the unchanged ids and stamp
vector, `LOG_FORMAT` 3 with format 2 decoding, and the stamp that writes the resident chunks
now and leaves a placement for the rest shipped as decided; the castle's 21 kept tiles read
44 966 592 B against 343 566 464 B dense, `prefab64_mixed` +1.4 % over the dense form, a
uniform 64³ prefab 7.9 µs (`docs/nfr/04-edits.md`). Four points differ from the text above:

- **A brick is a `u32` word**, not an enum: a uniform brick is its material and write bit in
  the word, a stored brick the index of a 1088-byte record (`FULL` bit, a 31-bit index), so a
  uniform brick is 4 B.
- **A placement keeps the set of chunks it still has to write, not a `seq`.** An inserted
  chunk, generated or restored from persistence, takes every pending placement that lists it,
  in stamp order; the spill format is unchanged (no `seq` word), and a placement lists a
  spilled chunk until it is reloaded and written once. `StampedSource` is unchanged.
- **`EditResult::aabb` is the bounds of the voxels changed now**, not the whole box; the
  pending count is read through `ChunkStore::pending_placements`.
- **The placement copies the prefab** (`Arc::new(prefab.clone())`): `World::stamp` takes
  `&Prefab`, so the registry's `Arc` is not shared and a rotated stamp materialises its
  rotation once more. The cost is bounded by the pending prefabs' bytes (at most the castle's
  45 MB again while its far tiles wait). A `stamp` that takes the `Arc`, the blit moved out
  from under the store's write lock on the poll thread, and dropping the coordinates a finite
  extent can never stream are follow-ups in `docs/backlog.md`, not changes to this decision.
