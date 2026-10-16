# ADR-0003: Sphere residency with an LOD-only ring, edited-chunk persistence with disk spill

> **Status:** accepted (2026-10-16)
> **Date:** 2026-10-16
> **Related plan(s):** [Plan 0002](../plans/done/0002-streaming-residency-lod-ring-and-mirror-narrowing.md)

## Context

Plan 0001 built a bounded world: every chunk of an extent is generated before the first frame
and stays resident. M1's second half makes the world stream around focus points, as ADR-0001
requires. Plan 0001's readings shape the decision (Implementation log of Plan 0001, `nfr.md`
§3 and §5):

- The cave's `Full` brick fraction is 20.8 % to 23.4 %, double the 10 % the roadmap assumed. A
  32³-chunk cube mirrors to 3.5 GB at `u16`, over the 3 GB world-VRAM line before any LOD.
- Generation runs at about 1.1e8 voxels per second per thread on the dev box. At 10 m/s and a
  100 m full-detail radius the leading face takes about 1250 chunks per second, about 3.3e8
  voxels per second. Any LOD tier that a chunk reaches only by generating it at full detail
  and downsampling adds to that rate.
- Per-voxel RAM for a resident chunk averages about 117 KB on the cave (3.84 GB over 32 768
  chunks). Keeping a far ring's full voxels in RAM to build mips cheaply is a multi-gigabyte
  cost.
- Edits are operations on resident chunks (ADR-0002) and fail with `NotResident` elsewhere.
  An evicted chunk that holds edits must come back with them. The consumer's levels are
  bounded and need no streaming, but the engine promises it (`nfr.md` §5).
- ADR-0002's access model lets an edit check residency and then lock chunks one at a time. With
  eviction in the picture, a chunk can leave the store between the check and the write, and
  the edit would land in an orphaned chunk.

## Decision

**Residency is a set of spheres in chunk units around focus points, in two tiers.** A chunk
`c` is *full* when `|c - f|^2 <= r_full^2` for some focus chunk `f`, *LOD-only* when it is not
full and `|c - f|^2 <= r_lod^2`, and not resident otherwise; the arithmetic is integer. A
hysteresis of `h` chunks applies at both boundaries: a full chunk is demoted only past
`(r_full + h)^2`, a LOD-only chunk is evicted only past `(r_lod + h)^2`. The radii are
parameters; their defaults are set from a measured sweep at the plan's close.

**A full chunk is the ADR-0002 chunk.** Edits, queries and raycasts see only full chunks. **A
LOD-only chunk keeps only its mips**: three levels of 32³, 16³ and 8³ cells, each stored as
mip-bricks of 8³ cells that use the same `Brick` type (`Uniform` or `Full`) as voxel bricks,
so a mip-brick is one brick-pool slot in the mirror. A mip cell is solid when any of its eight
children is solid, and takes the most frequent non-air child material, ties to the lowest id.
Level `n` is built from level `n - 1`. A LOD-only chunk is built from a full chunk: freshly
generated, reloaded from persistence, or demoted from residency, in which case its mips
include its edits. Promoting a LOD-only chunk regenerates or reloads its full voxels.

**Generation and demotion run on a `std::thread` worker pool** that `vxe-world` owns, fed by a
queue ordered by squared distance to the nearest focus with ties broken by coordinate. Full
promotions come before LOD work. A job whose chunk is no longer wanted when a worker takes
it is dropped. The caller's thread drives residency with `update(focus points)` and
`poll()`. `poll` inserts finished chunks and performs evictions, and nothing in it reads a
clock.

**Edits and evictions exclude each other through the store map lock.** `World::apply` holds
the store map's read lock for the whole operation, and insertion and removal take its write
lock. An op therefore sees one residency for its check and its writes, and an `Ok` result is
never lost to a racing eviction.

**An evicted chunk whose voxels were ever edited is persisted.** A chunk carries an `edited`
flag, set by any op that changed it and never cleared by the mirror. On eviction an edited
chunk moves into a persistence map in RAM, as it is, with uniform bricks still 2 bytes. When
that map exceeds a RAM cap, its oldest entries (by an eviction sequence number, not a clock)
spill to files in a caller-supplied directory. Each file is one chunk in an engine-private
format: magic, version, a caller-supplied world key, the coordinate, the bricks and an
FNV-1a 64 trailer. A load that fails its magic, version, key or trailer check reports an
error and leaves the chunk non-resident; it never silently regenerates over lost edits. A
file is deleted once its chunk is resident again. Without a spill directory, the map grows
past the cap and reports that it did. The spill is a cache for one process, not a save
format, and makes no compatibility promise across engine versions.

## Consequences

### Positive

- Spheres hold `pi / 6`, about 52 %, of the chunks of the bounding cube. That share of
  generation, RAM and VRAM is saved before any other lever is pulled.
- The LOD ring costs mips, not bricks: about 73 mip-bricks per chunk at most against 512
  bricks, so far geometry is drawn at a fraction of full-detail VRAM.
- Mips reuse the brick type, the pool, the record encoding and the occupancy mask, so the
  marcher's LOD walk is the full-detail walk with larger cells.
- Persistence is exact and needs no replay machinery. A reloaded chunk is byte-identical to
  the one evicted, which the spill's trailer and a hash test assert.
- No new dependency: threads, queue and file I/O are `std`.

### Negative

- **A LOD-only chunk costs a full generation.** The ring is generated at full detail and then
  downsampled, and a promotion generates the chunk again. At 10 m/s the total generation rate
  is the full sphere's leading face plus the ring's. A `WorldSource` method that produces mips
  directly is the far-field clipmap's job and does not exist yet. On the baseline's four cores
  the ring may lag. The ring is the far edge, so the lag shows up as late far geometry, never
  as a hole near the camera.
- **Far edits are invisible to the CPU.** Raycasts and queries skip LOD-only chunks as empty,
  as they skip non-resident ones. A game that needs a query beyond `r_full` raises it.
- **Mips are conservative.** Any solid child makes a cell solid, so thin gaps close in the
  LOD and openings shrink by up to one cell. The on-screen judgement is the plan's.
- **Edits wait for insertions and evictions** for the time the store map's write lock is
  held. That is a map insert or remove, not generation, which runs outside any lock.
- **Persisted chunks grow without bound** in a world dug everywhere and without a spill
  directory. The map reports when it passes its cap. Dropping edits is never the fallback.
- **The spill directory is the caller's to clean.** Files of a crashed process stay where
  they are, and the world key keeps a later world from reading them.

## Alternatives considered

### Alternative A: Cube residency, matching the mirror's wrapped window

A Chebyshev cube is the simplest set to compute and maps one-to-one onto the wrapped chunk
table. It was rejected because it costs about 1.9 times the chunks of the sphere in
generation, RAM and VRAM, and the measured `Full` fraction leaves no room for that. The mirror
keeps a cubic window that contains the sphere.

### Alternative B: Mips in every resident chunk, LOD rays inside the full-detail set

The roadmap's starting proposal: every resident chunk carries its mips, and rays beyond a
distance read them. It was rejected because mips stored next to full bricks add VRAM where
the readings say there is none. The pixel footprint at 1080p and a 90 degree field of view
reaches 10 cm only at about 120 m, so inside a 100 m radius full detail is not sub-pixel and
LOD buys little. An LOD tier that replaces bricks rather than adding to them is what closes
the budget.

### Alternative C: Persistence by per-chunk op-log replay

Regenerate the chunk from the seed and replay the ops that touched it, clipped to that chunk.
It is compact and close to M5's seed-plus-log model. It was rejected because it needs a
per-chunk clipped form of every op, makes a reload cost generation plus replay, and leaves
the log growing with every edit even when the chunk is resident. M5 owns the op log. Keeping
the edited chunk itself is exact today, and its RAM cost is bounded by how much was dug.

### Alternative D: Keep the LOD ring's full voxels in RAM

Promotion and demotion would then cost no generation. It was rejected on RAM. At the
measured 117 KB per chunk, a ring from 16 to 24 chunks (40 700 chunks) is about 4.8 GB, more
than the whole 4 GB world-RAM line.

## Outcome

Added at acceptance, 2026-10-16, from Plan 0002's readings (its Implementation log; dev
reference, RTX 3080 Laptop GPU, Ryzen 9 5900HS):

- **The defaults are `r_full = 16`, `r_lod = 24`, `h = 1`**, from the `world_budget --stream`
  sweep in `nfr.md` §3: 17 077 full and 40 700 LOD-only chunks, 2915.9 MiB of CPU and
  1529.5 MiB of mirror around the cave's spawn. They are dev-reference defaults; the baseline
  card revisits them at M2.
- **The ring costs a little more per chunk than guessed.** About 39 % of level-1 mip-bricks are
  `Full`, against the 35 % assumed, so a ring chunk mirrors to about 17.4 KB against the
  16.5 KB estimate. A full chunk mirrors to about 52.5 KB, below the 57.9 KB estimate, because
  the streamed cave's `Full` fraction is 18.5 to 18.8 %, not the bounded level's 20.8 %.
- **RAM is the tightest margin.** The resident set accounts for about 3.24 GB at the
  defaults and the process peaks at 3.87 GB RSS, against the 4 GB world-RAM line.
- **Generation under streaming is slower than headless.** 6.95e7 voxels per second per worker
  with 15 workers beside the render thread, against 1.23e8 to 1.34e8 in `world_budget`. The
  dev box keeps the full sphere resident at 10 m/s (no unready-near frame); the baseline's four
  cores will not reach the leading face's rate, as the Negative consequence foresaw.
- **A demoting chunk leaves the store when the demotion is queued**, not when its mips
  return: edits see it as non-resident meanwhile, and the mirror keeps drawing its full record
  until the LOD record replaces it. This refines, and does not change, the edit/eviction
  exclusion above.
- **The render-thread side of residency misses the streaming line.** `update` and `poll`
  rebuild the queue and sweep the whole resident set when the focus chunk changes, which puts
  the p99 frame at 52 to 61 ms. The tiers and the exclusion stand; an incremental queue and
  sweep are the streaming-CPU plan's.
