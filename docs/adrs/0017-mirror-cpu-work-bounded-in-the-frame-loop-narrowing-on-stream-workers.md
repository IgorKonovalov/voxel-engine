# ADR-0017: The mirror's CPU work stays in the frame loop, bounded per frame, with narrowing on the stream workers

> **Status:** proposed
> **Date:** 2026-10-20
> **Related plan(s):** [Plan 0023](../plans/0023-render-thread-cpu-attribute-hitch-and-streaming-line.md)

## Context

The render thread does all of the mirror's CPU work inside one frame: `Streamer::update` and
`poll` (`crates/vxe-world/src/stream.rs`), `GpuWorld::apply_events` and `sync`, then
`GpuWorld::update_attrs` (`crates/vxe-render/src/gpu_world.rs`). Two `nfr.md` §2 figures miss
because of it. The streaming line (1.0 ms) has been a recorded miss since Plan 0002: on the
quiet box at 2796062 it reads p50 4.5 to 5.0 ms and p99 33 to 65 ms. The attribute work list
that the contour look added (ADR-0011, Plan 0013) reads p99 138 to 180 ms of render-thread CPU
in the same flight, a hitch the eye sees. The default look then flies 35 to 47 % fewer frames
than `--look plain`.

Both costs come from work that grows with what changed in a frame. Some of it is not needed at
all. `drop_record` puts every dropped chunk, LOD-only ones included, into the attribute shell
set, and each shell chunk visits a one-brick shell of 10^3 - 8^3 = 488 bricks. Other work is
needed but arrives in bursts: a focus step of one chunk rebuilds the stream queue from about
58 000 wanted chunks and sweeps about 65 000 resident ones. Narrowing a chunk's bricks into
palette indices and staging them runs on the render thread for every chunk the sync writes,
up to the 2 MiB per-frame upload budget.

How to move that work off the frame is a threading and ownership decision. It also fixes where
the next CPU cost lands: the lighting plan's light extraction is the third item on the same
§2 line.

## Decision

**The mirror's CPU work stays in the render thread's frame loop, and `GpuWorld` stays the one
owner of the mirror's CPU model and its uploads. Each piece of work is made proportional to
what changed and capped per frame where it can wait.** Concretely:

- Work that is not needed is skipped. The attribute shell of a chunk visits only the bricks of
  neighbouring full chunks, after one lookup per neighbour chunk.
- Work that can wait is capped. The attribute pass takes at most a fixed number of bricks per
  frame from a carry-over queue. A deferred brick's attribute slots read "no plane" until its
  turn comes, so it shows face-mode normals and never stale planes.
- Work that scales with the focus step is made incremental. The stream queue, the residency
  sweep and the window shift touch only the entering and leaving shells or slabs.
- Pure functions of a chunk's content run on the existing stream workers. Narrowing and
  palette building for a chunk that gets a new record are done there, through a hook that
  `vxe-world` declares and `vxe-render` implements. The render thread still allocates slots,
  writes records and issues every `queue.write_buffer`, and it narrows inline whenever a
  prepared result is missing or stale.

## Consequences

### Positive
- The frame loop keeps its shape. `sync`'s order of effects and its reservation and deferral
  rules (ADR-0013) do not change, and neither does the mirror's byte layout (ADR-0004,
  ADR-0012). Each change can be tested headless against the code it replaces.
- No lock or channel can block the render thread on a second mirror owner. The stream workers
  already exist, and their output already crosses to the render thread in `poll`.
- `vxe-world` stays GPU-free. The hook is a trait that takes chunk content and returns an
  opaque value, and the world never interprets it.

### Negative
- New rock can show face-mode normals for up to the cap's carry-over age (250 ms is the bar).
  An edit's bricks are not deferred.
- The hook couples the stream workers to the mirror's narrowing format. A format change has to
  update both the inline path and the prepared path, and an equality test guards them.
- Prepared results take RAM between a worker's `Done` and the sync that uses them. Under mirror
  pressure, deferred chunks would hold them indefinitely, so they are capped and dropped
  oldest-first.
- The render thread still pays for slot allocation, record writes and the `write_buffer`
  copies. If those alone exceed the 1.0 ms line, this decision has not been enough, and the
  rejected alternatives come back.

## Alternatives considered

### Alternative A: a dedicated mirror-prep thread

Split `GpuWorld` into a CPU model (records, slot allocator, palettes, change sets, the
attribute work list) owned by its own thread, and a thin GPU uploader on the render thread
that applies ready batches. This takes the most work off the frame. It lost because it splits
the one owner of the reservation and deferral rules (ADR-0013) across two threads. Every
`sync` invariant, such as "a deferred chunk keeps its old record" or "a sync that fits nothing
writes nothing", would become a cross-thread protocol with its own tests. It would also add a
frame of latency between a world edit and its mirror. A profile has not yet shown that the
bounded, incremental version misses.

### Alternative B: build the work list and stage uploads on the GPU

Expand the attribute work list in a compute pass from per-chunk change bits, and stage pool
writes through a persistently mapped staging ring instead of `queue.write_buffer`. This moves
the list's cost off the CPU completely and saves a copy. It lost because the cost being
attacked is mostly hash lookups the list does not need, which skipping removes on the CPU.
GPU expansion would also add a pass and its bounds checks to the frame budget. A mapped ring
needs explicit fence tracking that `wgpu` exposes only through `map_async` polling, which is
a new synchronisation surface on every backend for a copy that has not been profiled.

## Outcome

_(Added only at acceptance if implementation falsified something above.)_
