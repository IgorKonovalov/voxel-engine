# ADR-0022: Mirror uploads go through a persistently mapped staging belt and copy commands, not `Queue::write_buffer`

> **Status:** accepted (2026-10-21, at Plan 0027's close; Outcome below)
> **Date:** 2026-10-21
> **Related plan(s):** [Plan 0027](../plans/done/0027-mirror-uploads-through-a-staging-belt.md)
> **Supersedes in part, once accepted:** ADR-0017's rejection of its Alternative B, for the
> staging-ring half of that alternative only. The GPU-built work list stays rejected, and
> ADR-0017's frame-loop shape, single mirror owner and per-frame caps stand.

## Context

The streaming line of `docs/nfr.md` §2 (1.0 ms of render-thread CPU per frame for streaming,
upload and light extraction) has been a recorded miss since Plan 0002. After Plan 0023 made the
stream queue, the residency sweep and the window shift incremental, plain flight still reads
`update + poll + sync` at p50 4.9 to 5.0 ms, p99 12 to 24 ms and max 22 to 60 ms on the quiet
dev box, nearly all of it in `GpuWorld::sync`.

Plan 0023's entry profile (frame-pointer call graphs over a plain flight, 16 839 render-thread
samples under `sync`) puts `Queue::write_buffer` at 41.8 % of `sync`, ahead of the narrowing
in `upload_record` at about 34 % and the palette scans at 9 %. Of `write_buffer`'s own
samples, 53.8 % are its self time and 24.5 % unresolved `libc`; `wgpu_hal` buffer creation,
mapping and transitions are under 2 % each. The mirror already coalesces its writes into runs
(`stage` appends consecutive slots up to a page, `stage_record` up to 2048 records), so a
sync issues a handful of `write_buffer` calls of up to 2 MiB, not thousands of small ones. The
cost is therefore per byte, not per call: each `write_buffer` makes `wgpu-core` take a fresh
staging allocation, map it, copy the bytes in and schedule the copy at submit. Fresh
host-visible pages are touched for the first time on every call, and a 2 MiB run is 512 page
faults inside the memcpy. That is the shape the profile shows: self time plus `libc` with no
driver work to speak of.

`wgpu` 30 ships `wgpu::util::StagingBelt`: a ring of persistently mapped chunks sub-allocated
per write, with `finish_and_recall_on_submit` re-mapping a chunk through the command encoder's
own submission callback. The fence tracking that ADR-0017 counted against a mapped ring is
inside the belt, not in our code.

The mirror's remaining CPU cost after the copies is the narrowing (ADR-0012's mirror-slot plan
removes most of it, because the slot then holds the CPU brick's own bytes) and the stream
workers' hook that ADR-0017 kept open.

## Decision

**`GpuWorld` owns a `StagingBelt`, and `sync` takes the frame's command encoder.** Each staged
run (pool slots, mask groups, records, table runs, the material rows) is written into a
belt slice and copied to its destination with `copy_buffer_to_buffer` recorded into that
encoder, in the order the runs are staged today; `sync` ends by calling
`finish_and_recall_on_submit` on the encoder, and the caller submits the encoder as it does
now. The belt's chunk size is 4 MiB, above the fly-through's 2 MiB per-frame sync budget
plus a frame's records, so a sync normally fills one chunk and the belt holds two or three
chunks once the frames in flight have recalled theirs. `sync` outside a frame (the bench's
immediate sync, the first level sync, tests) records into an encoder of its own and submits
it. Nothing else about `sync` changes: its selection, its reservation and deferral rules
(ADR-0013), the byte budget, the order of effects and the bytes written are the same.

## Consequences

### Positive
- No staging allocation and no first-touch page fault per sync once the belt is warm: the
  copy is one memcpy into mapped memory plus a copy command the GPU executes before the
  march, which is what every shipped engine's upload path does.
- The copies land in the frame's own submission, before the attribute pass and the march, so
  the ordering today's `write_buffer` gives (writes before the submit) is kept without a
  second queue operation.
- The belt is a `wgpu` utility, not a new synchronisation surface: its recall rides the
  submission callback on every backend.

### Negative
- **`sync` needs an encoder**, so every caller changes: the frame loop creates its encoder
  before `sync` instead of after, the bench and first-sync paths make and submit one, and the
  mirror tests gain a helper that submits and polls. The API break is minor-version work.
- **Mapped chunks are host RAM held for the mirror's life:** two or three 4 MiB chunks. A
  caller that records into an encoder and never submits it makes the belt allocate a new
  chunk per sync; the mirror exposes the chunk count so a test can pin it.
- **The belt does not fix narrowing.** With the copies gone, `upload_record`'s plane-to-index
  loop is the largest share of `sync`; the mirror-slot plan removes it by uploading the packed
  brick's own words, and ADR-0017's worker hook stays available for what is left.

## Alternatives considered

### Alternative A: `Queue::write_buffer_with`

Write each run straight into the staging view `wgpu` hands back, dropping the mirror's own
`stage_voxels` copy. The smallest change, and it removes one memcpy of the two. It lost
because it keeps the per-call staging allocation and its first-touch faults, which the profile
says is the cost; the mirror's own copy is 0.6 % of `sync`.

### Alternative B: narrowing and staging on the stream workers first

ADR-0017's unbuilt fourth bullet: workers narrow a chunk's bricks and build its palette, the
render thread copies prepared bytes. It attacks the 34 % share, not the 42 %, and leaves the
copies where they are. It lost as the next step for that reason, and because the mirror-slot
plan makes most of the narrowing disappear; the hook stays an option after both.

### Alternative C: a mapped ring of our own with explicit fences

One `MAP_WRITE | COPY_SRC` buffer per frame in flight, re-mapped by `map_async` after a
`device.poll`. The same mechanism as the belt with our own bookkeeping; the belt already does
the chunk rotation, the sub-allocation and the recall-on-submit, so writing it again buys
nothing and adds the fence surface ADR-0017 was right to avoid.

### Alternative D: write the pool shards directly from the CPU

Create the shards `MAP_WRITE | STORAGE` and write voxels into the mapped mirror. Needs
`MAPPABLE_PRIMARY_BUFFERS`, a native-only feature that is a performance trap on discrete
cards (the shard would live in host-visible memory the march reads over the bus). Rejected
under ADR-0001's no-vendor-path rule and ADR-0004's binding model.

## Outcome

2026-10-21, from Plan 0027's readings (dev reference, RTX 3080 Laptop GPU, Ryzen 9 5900HS,
quiet box, 843c5a3 paired with the pre-plan ae58f0c, three runs each).

- **The decision held.** `Queue::write_buffer` has no samples under `sync`; plain `sync` p50
  fell from 3.28 to 3.69 ms to 1.40 to 1.66 ms and `update + poll + sync` p50 from 3.33 to
  3.78 ms to 1.72 to 1.99 ms, with `sync` max from 15.1 to 35.6 ms to 8.8 to 9.6 ms. The mirror's
  bytes are unchanged: a 16 MiB-budget build reads back equal to the unbudgeted one buffer
  for buffer, and every golden renders byte for byte. The 1.0 ms line of `nfr.md` §2 stays a
  recorded miss, led by narrowing (43.2 % of `sync`).
- **"One memcpy into mapped memory" is not free.** The belt reads 25.4 % of `sync`, most of
  it (20.5 %) at the first locked instruction after the copy into the mapped chunk
  (`BufferViewMut::drop`), against the plan's under-5 % bar: a recorded miss the owner
  accepted. The per-call allocation is gone; the cost of writing into mapped memory remains.
  Staging the narrowed voxels straight into the slice, or the chunks' memory type, is the
  next look (Plan 0028 or after).
- **The chunk count needed a cap the Decision did not name.** An unbudgeted first sync of the
  bounded level takes about 56 chunks; the mirror replaces the belt when a sync leaves it
  holding more than three (`BELT_KEEP_CHUNKS`), so the burst's chunks drop once their copies
  complete. In flight the belt holds one or two chunks (4 to 8 MiB).
