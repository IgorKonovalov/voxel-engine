# ADR-0056: The mirror's slot image is packed on the stream workers; the render thread only allocates and copies

> **Status:** accepted (owner, 2026-11-12, with its plan)
> **Date:** 2026-11-12
> **Related plan(s):** [Plan 0070](../plans/0070-the-sync-off-the-frame-slot-words-packed-on-the-workers.md)

## Context

The mirror (`GpuWorld`) turns a resident chunk into GPU bytes on the render thread, inside
`sync`: it sizes the chunk (`need`: 512 solid masks and the surface bands under the chunk's
write lock), packs each `Full` brick's slot words, scans the bricks for emissive samples, and
copies the result into the staging belt (ADR-0022). Only the last step and the record allocation
need the render thread. Plan 0059's quiet re-read puts the sync stage at 3.19 to 3.90 ms p50 of a
streaming CPU p50 of 3.4 to 4.2 ms, against the 1.0 ms line of `docs/nfr.md` section 2, with a p99
of 45.9 to 83.6 ms; Plan 0068's bench reads `update + poll + sync` p99 at 39.9 ms in the cave and
40.7 ms in the castle at the RTX window (release), wall p95 36.6 and 58.1 ms with the GPU at
6.3 ms.

A time budget on the render-thread sync does not work: Plan 0064 read residency p95 at 3.0 to
13.7 s with a 2 to 8 ms budget against 0.30 s without one, because a chunk costs about 0.3 ms
whether it is spread over frames or not. The cost has to leave the thread, not be rationed.

The workers already run a pure function per loaded chunk: `vxe_world::ChunkPrep` (ADR-0017,
Plan 0059), whose `FacePrep` computes the bake's face masks at a mean of 177.8 us a chunk. Since
Plan 0028 a `Full` brick's slot is its packed CPU bytes in 64-byte units (ADR-0012), so the slot
words of a chunk do not depend on where the mirror places them; only the record and the slot
offsets do. The roadmap has owed an "upload-copy ADR" since Plan 0023's close (2026-10-21), and
the backlog carries the mechanism as "Slot words packed on the workers".

## Decision

A loaded full chunk's **slot image**, its per-brick slot words with their size classes, its
emissive samples and its face masks, is produced by one `ChunkPrep` on the stream workers at
load and carried to the render thread in the stream's events. The render thread's `sync`
allocates the chunk's record and slots, copies the image's bytes into the staging belt and
writes the record; it computes nothing per brick for a chunk that arrives with an image. A
chunk edited since its image was made, or loaded before the prep existed, takes today's inline
path for its **dirty bricks only**, so an edit's cost on the render thread is bounded by the
bricks it touched, as `docs/nfr.md` section 4 already requires of its upload. The image is a
pure function of the chunk's content and the material table, so the mirror's bytes are
byte-identical to today's whichever thread packed them, and a test asserts it.

## Consequences

### Positive
- The render thread's per-chunk work falls from a pack to a copy: at about 50 KB a chunk on
  average (0.87 GB live over about 17 000 full chunks at `r_full` 16), tens of microseconds of
  memcpy instead of about 300 us. The streaming line becomes reachable and a per-frame bound on
  the sync becomes meaningful.
- The work moves to threads that already exist and already touch every loaded chunk; no new
  thread, lock or queue.
- The mirror's format stays where it is; no shader or layout changes.

### Negative
- Memory: an image per loaded chunk waits between the worker and the sync, about 50 KB average
  and 512 KB worst (a chunk of 512 `Full` bricks at 1 KiB), bounded by the pending set, and
  dropped on edit. Plan 0070 reads the peak.
- Two packing paths (worker image, inline dirty bricks) that must agree byte for byte; the
  equality test is the guard, and a divergence is a stop.
- The material table's flags (emissive, translucent) reach the workers; a table change at
  runtime (none exists today, `MaterialTable` has no setter) would invalidate every image.
- The type-erased `Prepared(Arc<dyn Any>)` carries a larger value; Plan 0070 may type it.

## Alternatives considered

### Alternative A: a time or chunk budget on the render-thread sync
Plan 0064 built it (`--sync-ms`) and read residency p95 at 3.0 to 13.7 s against 0.30 s. A
budget rations work it does not reduce; the chunk's 0.3 ms is paid either way, and the frames
that pay it are the frames the camera moves into. Rejected by reading.

### Alternative B: a dedicated upload thread with its own command encoder
`wgpu::Queue` is `Send + Sync`, so a second thread could pack and `write_buffer`. ADR-0022 moved
the mirror off `write_buffer` because the copies were the streaming line's largest share, and
the belt's copies are recorded into the frame's encoder for ordering against the frame's passes.
A second thread would reintroduce the copies the belt removed or need cross-thread encoder
ordering wgpu does not offer. Rejected.

### Alternative C: pack on the GPU from the raw packed bricks
Upload the chunk's packed bricks as they are and let a compute pass write the slots. The slots
are the packed bytes already (ADR-0012, Plan 0028), so the pass would compute nothing; the
render-thread cost is `need`'s masks and the per-brick loop, which would stay. Rejected.

## Outcome

### 2026-11-13: deferred by reading (Plan 0070 Phase 1; orchestrator default, owner may overturn)

Plan 0070's Phase 1 put the sync's stages on a clock and read the premise false for today's
costs. On the RTX 3080 Laptop host (tester cave bench, headless 2530 x 1378, release, both
adapters; `docs/plans/logs/0070.md`), with the per-brick clock reads removed, the packing alone
reads about 1.5 ms and `pack + size + light` 34 % of a sync p99 of 44 to 47 ms. The p99 stage is
the record's brick loop (about 28 ms), whose largest self cost in the profile is a
`BTreeSet<usize>` of bricks to rescan for emitters; the p50 stage is the light walks run every
sync. The "about 0.3 ms a chunk of packing" this record's Context assumes was mostly not
packing.

The decision is not superseded: nothing refutes the mechanism, only its share. Plan 0070 was
amended to cut the measured stages in place on the render thread first; its Phase 5 re-reads
`pack + size` against what remains and, at half or more of the sync's p99, the worker packing
returns as its own plan under this record. Until then nothing here is built.
