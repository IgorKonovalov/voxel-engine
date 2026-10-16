# ADR-0002: Brick-map storage layout, GPU mirror, and per-chunk locked access

> **Status:** accepted (2026-10-16); mirror clauses (`u16` pool voxels, one pool binding) superseded by ADR-0004 (accepted 2026-10-16)
> **Date:** 2026-10-16
> **Related plan(s):** [Plan 0001](../plans/done/0001-bounded-brick-map-edits-and-headlamp-marcher.md)

## Context

The world is a streaming brick map of `u16` material ids at one voxel size per world
(ADR-0001). Before the first line of storage code we have to fix three things that every later
crate depends on: the layout on the CPU, the layout of its GPU mirror, and who may touch a chunk
while someone else reads it. The roadmap's "Storage shape" section proposed the sizes; this ADR
makes them concrete and adds the decision the roadmap left open, the access model.

The forces:

- **Edits are frequent, local and must be cheap.** The consumer carves many small shapes per
  frame and reads exact per-material counts back synchronously (`nfr.md` §4: 100 carves of
  radius 1 m in 2 ms of CPU). A structure that must be rebuilt or recompressed on edit (an
  octree, a DAG) loses here; the production voxel ray tracers that edit at runtime (Teardown,
  Wingerden's thesis, DoonEngine; see `docs/references/voxel-optimizations.md`) all keep the
  editable field raw in bricks.
- **Caves are almost all uniform.** A 128 × 128 × 48 m level at 10 cm is about 840 M voxels,
  nearly all solid rock or air. The VRAM budget (`nfr.md` §3, 3 GB at the full-detail radius)
  only closes if a uniform brick costs bytes, not kilobytes.
- **Bandwidth, not arithmetic, bounds the marcher.** Every fast GPU marcher in the reference
  list skips empty space through a 64-bit occupancy word per block before it reads a voxel.
- **Several threads will share the store.** By M4 the generator pool writes chunks, the render
  thread copies dirty bricks out, the audio thread probes rays, and the game queries and edits.
  By M5 edits are applied by a server tick and mirrored on a client. The access model has to
  survive that without a rewrite.
- **Determinism.** Edit results must be bit-identical across Linux and Windows. Anything that
  makes a result depend on thread timing or on iteration order of a hash map is out.

## Decision

**CPU layout.** A **voxel** is one `u16` material id; 0 is air. A **brick** is 8 × 8 × 8 voxels
and is either `Uniform(MaterialId)` or `Full(Box<[u16; 512]>)` (1 KiB). A brick that becomes
all one material after an edit collapses back to `Uniform`. A **chunk** is 8 × 8 × 8 bricks
(64³ voxels), stored as a fixed array of 512 brick slots plus a 512-bit dirty mask, and is the
unit of generation, upload, dirty tracking and (from Plan 0002) paging. Chunks are addressed by
signed integer `ChunkCoord`; voxels by signed integer `VoxelCoord`; shape parameters by
fixed-point integers at 1/256 voxel. A chunk never stores its own world position twice: the
store's key is the coordinate.

**GPU mirror.** `vxe-render` mirrors the store into four storage buffers, uploaded per dirty
brick:

- a **brick pool** of 1 KiB slots holding the voxels of every `Full` brick, allocated from a
  CPU-side free list;
- a **brick mask** buffer with one 64-bit word per pool slot, one bit per 2 × 2 × 2 block, set
  when the block holds any non-air voxel;
- a **chunk record** buffer with 512 `u32` brick entries per resident chunk, each entry either
  `uniform | material` or `full | pool slot`;
- a **chunk table** mapping a chunk coordinate to its record, addressed by *wrapped* coordinate
  (`coord mod window`). In Plan 0001 the window equals the bounded extent, so the wrap is the
  identity; Plan 0002 shrinks the window to the resident radius and the layout does not change.

The mirror stores voxels at the CPU width (`u16`). Narrowing to a per-chunk 8-bit palette is a
named follow-up, taken only if Plan 0001's VRAM reading says it is needed. A second pool,
parallel to the brick pool and indexed by the same slot, is the reserved place for an optional
per-voxel attribute (the M2 surface spike); it does not exist until a plan needs it.

**Access model.** The store is a map from `ChunkCoord` to `Arc<RwLock<Chunk>>` (`std::sync`,
no new dependency), itself behind a `RwLock` that is written only on insert and remove. **An
edit writes bricks in place under the chunk's write lock, one chunk at a time, in ascending
coordinate order**, after checking that every chunk the shape touches is resident; if one is
not, the op fails with that coordinate and changes nothing. Readers (upload, queries, later
audio probes) take short read locks per chunk. A reader may observe a multi-chunk op half
applied; the caller that issued the op sees it complete when the call returns, which is the
only consistency the consumer needs. Iteration for hashing or results always sorts by
coordinate; nothing observable depends on map iteration order.

## Consequences

### Positive

- An edit costs its lattice walk and nothing else: no copies, no rebuild, no allocation beyond
  expanding a `Uniform` brick the shape enters.
- Uniform bricks cost 2 bytes on the CPU and 4 bytes in the mirror, so the memory budget rests
  on the measured uniform fraction, which Plan 0001 reads at two scene sizes.
- The marcher skips a uniform brick with one entry read and a full brick's empty blocks with
  one mask read, which is the shape of every fast marcher in the literature.
- Per-chunk locks let the generator pool, the uploader and the audio thread share the store
  without a global lock, and let the M5 client mirror be the same type as the server world.
- The chunk table's wrapped addressing is paging-ready, so Plan 0002 adds streaming without
  touching a buffer layout or a shader binding.

### Negative

- **No free snapshots.** Replay and late join come from seed plus op log, not from a frozen
  copy of the store. A reader that needs a consistent view of several chunks must lock them
  all, in coordinate order, or accept tearing.
- **Readers can wait behind an edit** for the microseconds a chunk's write lock is held. The
  render thread's upload copies under a read lock and may be delayed by a burst of edits; the
  `nfr.md` §2 streaming line is where that shows up.
- **`u16` in the mirror doubles VRAM against a palette.** At 10 % non-uniform bricks over a
  100 m radius that is about 1.7 GB versus 0.85 GB. The palette step exists on paper and is
  taken on measurement, not now.
- **A `Full` brick is 1 KiB even when it holds one stray voxel.** Caves produce such bricks at
  every surface; the surface fraction is what the close measures.
- **Storage buffer binding limits.** A single pool buffer holding the full resident set may
  exceed an adapter's `max_storage_buffer_binding_size`. Plan 0001 records the limits of the
  adapters it runs on; sharding the pool across bindings is Plan 0002's problem if the readings
  demand it.

## Alternatives considered

### Alternative A: Sparse voxel octree or DAG (ESVO, SVDAG, Aokana)

The best memory per voxel and the natural LOD, which is why the far field may still use it.
Rejected for the editable near field because an edit rebuilds a subtree or needs the special
machinery of Careil et al. to patch a compressed structure, and because the consumer's count
results then need a second pass over what changed. The reference comparison (VoxelRT) shows the
masked brick map tracing primary rays as fast as the trees.

### Alternative B: Immutable chunks with copy-on-write snapshot swap

Chunks as immutable `Arc`s; an edit copies the bricks it touches into a new chunk and swaps the
pointer, so readers never block and get free snapshots. Rejected because every edit then copies
up to 64 KiB (a radius-1 m carve touches up to 64 bricks) and 100 edits per frame copy several
megabytes before the lattice walk even starts, spending the `nfr.md` §4 budget on memcpy; and
because the snapshot the model buys is already provided by seed plus op log.

### Alternative C: Single owner thread with message passing

One thread owns the world outright and every other party sends ops and queries over channels,
the renderer keeping its own CPU-side mirror. Lock-free and deterministic by construction, and
the M5 split falls out of it. Rejected because every game and audio query becomes a round trip
with a tick of latency (audio occlusion probes hundreds of rays per block), and because the
renderer would need a second full copy of the voxels in RAM from day one.

## Outcome

Added at acceptance, 2026-10-16, from Plan 0001's readings (its Implementation log):

- **The uniform fraction is lower than assumed.** The cave's `Full` fraction is 20.8 % to
  23.4 %, not 10 %, so the `u16` mirror estimate above (1.7 GB at a 100 m radius) is low by
  about double; a 32³-chunk cube mirrors to 3.5 GB. ADR-0004 narrows and shards the mirror in
  response.
- **The uploader takes a chunk write lock, not a read lock.** `GpuWorld::sync` clears the
  chunk's dirty bits as it copies, so it holds the write lock for the copy. The cost is the one
  named under Negative (readers wait behind the uploader as they would behind an edit); the
  access model is otherwise as written.
- **Binding limits bite at level size.** lavapipe binds 128 MiB per storage buffer, below the
  level-size pool (about 375 MiB at `u16`); the dev box binds 2 GiB. ADR-0004 shards the pool.
