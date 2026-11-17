# ADR-0060: Entity volumes are freed by handle into a coalescing free list, and a volume never moves

> **Status:** accepted 2026-11-16 (Plan 0078 closed, v0.59.0; Outcome below)
> **Date:** 2026-11-16
> **Related plan(s):** [Plan 0078](../plans/done/0078-the-entity-pool-frees-models-by-handle-stable-instance-slots-and-a-reset.md)

## Context

ADR-0050 gives entities a pool of their own: one storage buffer in `vxe-render` holding the
tree, the instances and every part's volume (occupancy bits, block bits, palette indices and a
256-entry palette). `EntityMirror::upload_model` writes each distinct volume once, keyed by its
prefab's content id (ADR-0014), by bumping a word counter. Nothing frees. A model whose parts
change on a hot reload (Plan 0063's viewer, Plan 0077's library) leaves its old volumes in the
pool forever, and a consumer that unloads a level keeps every creature's volumes in VRAM until
the frame is dropped. The pool is 8 MiB by default (`DEFAULT_ENTITY_POOL_BYTES`): a 64-a-side
part is 296 480 B, so twenty-eight saves of such a part fill it; the walker's torso (1708 B)
takes about 4900. The owner asked for a proper reset (2026-11-16, Plan 0077's owner question 6).

Three facts shape the choice. Volumes vary in size by almost 300 to 1 (1068 B for a one-voxel
part, 296 480 B for the largest), so fixed blocks waste up to half the pool. Every write to the
entity buffer goes through `wgpu::Queue::write_buffer`, which the queue orders after every
command buffer submitted before it and before the next submission, so a slot rewritten between
two frames is read by neither with the wrong bytes. And the instances name a volume by its word
offset (`entity.wgsl` reads word 24 of the instance), so a volume that moved would need every
instance that names it rewritten in the same frame, and the previous frame's hit records would
still name the old place.

## Decision

The consumer that uploaded a model frees it, by its handle: `EntityMirror::free_model(handle)`
drops the model's claim on each of its volumes, and a volume nobody claims returns its words to
a **free list of word ranges**, ordered by address and coalesced with its neighbours.
`upload_model` takes the first range that fits, from the lowest address. A volume never moves
once written. A handle carries a generation beside its index, so a freed handle is refused
(`UnknownModel`) and an instance listed under one is counted, not drawn. `clear()` frees
everything at once. A refusal names the largest hole as well as the free total
(`PoolFull { needed, free, largest }`), so a consumer can tell a full pool from a fragmented one
and answer the second with `clear()` and a re-upload of what it still draws.

The pool is safe to reuse the moment a range is freed, with no frames-in-flight bookkeeping,
because every write to the buffer is queue-ordered. A mapped or persistently-written path into
this buffer would break that rule and is not allowed.

## Consequences

### Positive
- A long authoring session's pool holds exactly the live volumes: a reload that changes one part
  costs that part's bytes once, and a model unloaded gives its bytes back.
- No volume moves, so the shaders, the instances and the hit records of the frame before stay
  valid without a rewrite or a deferred free.
- Two models sharing a prefab keep sharing its volume; the claim count, not the model count,
  frees it.
- The allocator is a pure function of its call sequence (an ordered map by address, no hash
  iteration), so the CPU side of the mirror is testable without a device and identical on every
  platform.

### Negative
- **External fragmentation.** Sizes vary, so the free total can exceed a request no single hole
  holds. First fit from the lowest address keeps the live set compact in practice, but the
  guarantee is only the refusal's `largest` figure and the consumer's `clear()`; a plan reads
  the waste on a seeded alloc-free sequence before anyone trusts a number.
- The consumer owns a lifetime. A model never freed is the same leak as today; the fly-through's
  HUD row shows the pool's use so the leak is visible.
- The handle grows a generation and loses its public field; code that built `ModelHandle(n)` by
  hand changes.

## Alternatives considered

### Alternative A: compaction (move live volumes down on free)
No fragmentation ever. It lost because a move rewrites every instance that names the volume, in
the same frame, and needs either a CPU copy of the pool (8 MiB of RAM mirroring VRAM) or
GPU-side copies through a scratch buffer, since `copy_buffer_to_buffer` within one buffer
refuses overlapping ranges. The previous frame's hit records would also name the old words.

### Alternative B: fixed-size blocks, or size classes as ADR-0028's mirror slots
Constant-time allocation and no fragmentation within a class. It lost on waste: volumes span
1068 to 296 480 B, so power-of-two classes waste up to half of every volume, and nine classes
each need their own reservation inside an 8 MiB pool that is already sized for hundreds of
parts, not for nine partitions. ADR-0028 chose classes because bricks come in four sizes.

### Alternative C: one buffer per model
Freeing is dropping a buffer. It lost because the walk reads one bound storage buffer and the
baseline adapter has no binding arrays (ADR-0050); a buffer per model means a bind group and a
pass per model, or a rebuild of the one buffer on every change.

### Alternative D: the mirror frees what the frame stopped listing
No consumer call: a model unlisted for N frames is freed. It lost because Plan 0077's library
uploads a model on first use and keeps it for the next placement; a sweep would free it and
re-upload it at the next placement, a hitch the consumer cannot prevent. A reference-counted
`Arc<Model>` sweep ties VRAM residency to Rust lifetimes the mirror cannot see.

## Outcome

2026-11-16, at Plan 0078's close. The decision held: a free by handle into a coalescing free
list, first fit from the lowest word, no volume moved, a generation in the handle, `clear()`,
and `PoolFull { needed, free, largest }`. Two details the body leaves open were settled in the
tree. A model's new volumes are allocated as **one range**, laid in part order, and each is
freed on its own, so `needed` is one request. The handle is 16 bits of slot and 16 of
generation, and a 65 537th live model is refused (`TooManyModels`). The waste on a seeded
10 000-call sequence over the default pool (`reading::the_waste_of_a_seeded_sequence`): free
bytes outside the largest hole 0.16 % of the live bytes at the live peak, 14.48 % at worst
with the pool at least half live; 875 refusals a `clear` answers against 648 of a full pool
(`docs/nfr/03-memory.md`). The fly-through answers a fragmented refusal with `clear()` and a
re-upload (`entity pool reset`).
