# ADR-0004: Per-chunk 8-bit palette mirror and a brick pool sharded over fixed bindings

> **Status:** accepted (2026-10-16)
> **Date:** 2026-10-16
> **Related plan(s):** [Plan 0002](../plans/done/0002-streaming-residency-lod-ring-and-mirror-narrowing.md)
> **Supersedes in part:** ADR-0002's mirror clauses: voxels at `u16` in the mirror, and the
> brick pool as one storage buffer. The CPU layout and the access model of ADR-0002 stand.

## Context

ADR-0002 mirrors voxels at the CPU width (`u16`) in one brick-pool storage buffer. It names a
per-chunk 8-bit palette as a follow-up to take on measurement, and leaves pool sharding to
Plan 0002 if the binding limits demand it. Plan 0001 measured both:

- **VRAM.** The cave's `Full` fraction is about 21 %. At `u16`, a 100 m full-detail sphere
  (17 077 chunks at `r_full = 16`) is about 1.9 GB of mirror, and a 32³ cube is 3.5 GB. Halving
  the voxel width halves the dominant term.
- **Binding limits.** One storage-buffer binding holds at most 2 147 483 644 B on the RTX 3080
  Laptop and on RADV, and 134 217 728 B on lavapipe. The level-size pool (383 608 `Full`
  bricks, about 375 MiB at `u16`) already fails on lavapipe. The baseline card's limit is not
  known yet.
- **Bindings per stage.** WebGPU's default is 8 storage buffers per shader stage. After
  Plan 0001 the marcher binds pool, masks, records, table and materials, which is 5.
- **Materials per chunk.** A cave chunk holds a handful of materials. A prefab or a test can
  hold more. The material id stays 16-bit in the engine's contract.

## Decision

**The mirror stores a `Full` brick as 512 one-byte palette indices (512 B).** Each mirrored
chunk record carries a palette of 256 `u16` material ids after its brick entries. Index 0 is
air in every palette, so the marcher tests air without a lookup. A chunk's palette is
append-only while the chunk is mirrored: an edit that brings in a new material appends it,
and the bricks already uploaded keep their indices. A uniform brick's record entry still
holds its material id directly.

**A chunk that needs more than 255 non-air materials is *wide*.** Each of its `Full` bricks
takes two adjacent pool slots, starting on an even slot, and holds `u16` voxels. Its record
entries carry a second flag bit. A chunk that overflows while mirrored becomes wide and
uploads its `Full` bricks again.

**The pool is sharded over up to four fixed storage bindings.** Pool slot `s` lives in shard
`s / slots_per_shard`. `slots_per_shard` is the largest whole number of 4096-slot pages that
fits the adapter's `max_storage_buffer_binding_size` and `max_buffer_size`. The shader picks a
shard with a `switch` over the four bindings, and unused shards bind a one-slot buffer. The
shard count is `min(4, max_storage_buffers_per_shader_stage - 4)`, computed from the adapter's
limits, never from its backend or vendor. Occupancy masks, chunk records, the chunk table and
materials stay one buffer each, indexed by global slot or record.

## Consequences

### Positive

- Brick VRAM halves: a `Full` brick costs 512 B and its mask 8 B. At the measured fraction a
  full-detail chunk averages about 57 KB of mirror, against about 112 KB at `u16`.
- Mip-bricks (ADR-0003) fit the same 512 B slot, so the LOD ring adds no pool kind.
- The pool reaches up to four times the per-binding limit on every adapter that exposes 8
  storage buffers per stage. On lavapipe that is 512 MiB, which covers the level-size world.
- The CPU stays the source of truth at `u16`, and no determinism-relevant code changes.

### Negative

- **A palette lookup per voxel hit** in the shader: one more dependent read, on a record
  already in cache for the brick entry. The close measures primary-ray time before and after.
- **A record grows from 2 KiB to 2.5 KiB** for the palette. Records then total about 44 MB at
  17 077 full chunks, and about 104 MB more across a 40 700-chunk LOD ring if LOD records,
  which use only 73 of the 512 entries, share the stride.
- **Palettes only grow while a chunk is mirrored.** A chunk dug from five materials down to
  one keeps five entries until it leaves the mirror. That is harmless at 256 entries.
- **Wide chunks are a second code path** that a cave never exercises. Only a dedicated test
  keeps it honest.
- **Four shards cap the pool at four bindings.** An adapter with a small binding limit and 6
  GB of VRAM may hold more than it can bind. When the cap binds before the VRAM budget, a
  later plan revisits it (binding arrays, or more shards on adapters with more bindings).
- **The chunk table and records stay one binding each.** At 2.5 KiB per record, 2 GiB holds
  about 850 k records, far above any planned radius.

## Alternatives considered

### Alternative A: Per-brick palette with a variable bit width

Bricks of one or two materials would pack at 1 bit per voxel, of up to 16 materials at 4
bits. It is the densest layout and what some production engines use. It was rejected for now
because it needs a variable-size slot allocator and a bit-width branch on every voxel read.
The flat 8-bit layout lands the larger saving first, and the reading after this plan shows
whether the rest is worth that complexity.

### Alternative B: One global 8-bit material remap

Mirror every voxel as an index into one table of 256 materials. It is simpler than per-chunk
palettes. It was rejected because it caps a whole world at 255 non-air materials and breaks
the 16-bit material contract that the consumer's runtime material variants (`crystal lit`,
`limestone shell`) rely on.

### Alternative C: Binding arrays of storage buffers

A `binding_array<array<u32>>` indexed by shard removes the fixed shard count. It was rejected
because it needs a native-only `wgpu` feature (storage-buffer binding arrays with non-uniform
indexing) that is not checked on the baseline adapter and that lavapipe's CI path would have
to prove. Any vendor- or feature-specific path is an ADR of its own. Fixed bindings work
within WebGPU's default limits on every adapter we target.

## Outcome

Added at acceptance, 2026-10-16, from Plan 0002's readings and its close review:

- **The shader always binds four shard bindings.** Unused shards bind a one-slot placeholder,
  so the marcher needs 8 storage buffers per stage whatever `shards` computes to. The computed
  shard count decides capacity, not the bind group layout. 8 is the WebGPU default, so this
  holds on every adapter we target, but a further storage binding (a per-voxel attribute pool,
  for one) needs either a higher requested limit or a packed binding.
- **Records are capped per binding, not by 2 GiB.** The records stay one binding, but its size
  is the adapter's `max_storage_buffer_binding_size`: lavapipe's 128 MiB holds 52 428 records,
  fewer than the 57 777 chunks the default residency mirrors. `ShardLayout::record_capacity`
  carries the cap and `sync` fails with `MirrorError::RecordsTooLarge`, writing nothing, before
  it would grow past it, as `PoolTooLarge` does for the pool.
- **Buffers grow by half again and never shrink.** The pool shards and the record buffer grow
  1.5x at a time and keep their size after evictions, so allocated bytes exceed the live
  accounting. At the defaults the mirror peaks at about 1.69 GB live and 2.43 GB allocated
  (`GpuWorld::mirror_bytes` against `GpuWorld::allocated_bytes`), against the 3 GB line. The
  budget is read on allocated bytes.
- **Narrowing costs about 150 ms on the level's first sync** (about 345 to about 500 ms), the
  palette scan before indices are written. Accepted; the scan moves to workers in the
  streaming-CPU plan.
- **The palette lookup costs nothing measurable:** the level's 1080p march pass read 1.854 to
  1.874 ms before narrowing and 1.869 to 1.889 ms after.
- **No cave chunk is wide.** The wide path is held by its dedicated test only. A chunk that turns
  wide during a sync is estimated narrow when the sync is budgeted, so a failed allocation on
  that path can leave a half-updated record; that is a recorded followup, not a change to the
  decision.
- **The LOD record shares the full record's stride.** About 99 MiB of LOD records at the
  defaults, under budget; a smaller stride stays open.
