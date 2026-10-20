# 0023: The render thread's mirror CPU: profile a chunk crossing, then cut the attribute hitch and the streaming line with targeted fixes

> **Status:** draft
> **Created:** 2026-10-20
> **Milestone:** M2 in `docs/roadmap.md` (owed before the lighting plan; it is the
> "streaming-CPU plan" the roadmap's M1 notes name, merged with the Plan 0013 attribute-hitch
> followup)
> **Related ADRs:** [ADR-0017](../adrs/0017-mirror-cpu-work-bounded-in-the-frame-loop-narrowing-on-stream-workers.md) (the frame loop keeps its shape: bounded, incremental, narrowing on the stream workers; proposed with this plan), [ADR-0013](../adrs/0013-mirror-reserves-its-vram-budget-up-front.md) (the reservation and deferral rules `sync` keeps), [ADR-0011](../adrs/0011-contour-planes-in-attribute-pages-with-procedural-detail.md) (the attribute pages and "no plane"), [ADR-0003](../adrs/0003-sphere-residency-with-lod-ring-and-edit-persistence.md) (residency bands), [ADR-0004](../adrs/0004-palette-narrowed-sharded-brick-pool.md) and [ADR-0012](../adrs/0012-mask-first-brick-with-per-brick-palette-planes.md) (the narrowing the workers take over)
> **Source:** the owner's interview answers, 2026-10-20 (scope, approach, the 250 ms lag, both
> bars), Plan 0002's Followups (the streaming-CPU plan) and Plan 0013's close (the attribute
> hitch).

## TL;DR

In a 10 m/s flight, the render thread spends p50 4.5 to 5.0 ms per frame in
`update + poll + sync` (the §2 line is 1.0 ms). In the contour look it also spends p99 138 to
180 ms in `update_attrs`, a hitch the eye sees, and the default look flies 35 to 47 % fewer
frames than `--look plain`. This plan starts by measuring before it changes anything. Phase 1
profiles a chunk-crossing frame on the dev box with `perf` and per-stage span timers, factors
the attribute work-list builder out of `GpuWorld`, and commits a headless bench that reproduces
one crossing's work list without a window or a GPU. Phase 1 also tests the working hypothesis.
The hypothesis is that about 2000 LOD chunks evicted on each crossing each drive a 488-brick
shell scan, close to 10^6 hash lookups. If the profile refutes it, the plan stops for a
re-plan. If it holds, the fixes follow and the frame loop keeps its shape:

- The shell pass is narrowed to the bricks of neighbouring full chunks.
- The attribute pass is capped per frame with a carry-over queue. A deferred brick reads
  "no plane", so new rock can show face-mode normals for up to 250 ms.
- The stream queue, the residency sweep and the window shift become incremental.
- Narrowing and staging move to the existing stream workers.

The first thing a reader sees is the Phase 1 bench, which counts today's crossing as 956 968
shell brick visits.

## Context & problem

Two frame costs on the render thread miss their bars. Both are dev-reference readings from
`docs/nfr.md` §2 and §5, quiet box at 2796062:

- **The streaming line** (`nfr.md` §2, "Streaming, upload, light extraction", 1.0 ms) has been a
  recorded miss since Plan 0002. Under `flythrough --autopilot --headless --seed 1` it reads
  p50 4.993 and 4.966 ms (plain) and 4.740 and 4.505 ms (default), and p99 32.6 to 65.3 ms.
- **The attribute work list** (Plan 0013, no `nfr.md` line yet) reads `update_attrs` CPU p50
  4.600 and 4.700 ms, p99 179.598 and 138.104 ms, max 390.376 and 156.825 ms. The work list is
  p50 4773 and 4462 bricks and max 11 477 and 7555. The default flew 2211 and 2695 frames
  against plain's 4123 and 4155.

What the code does today (`crates/vxe-render/src/gpu_world.rs`):

- `GpuWorld::attr_work` takes three change sets every frame. It takes every brick of the chunks
  whose record was written whole (`attr_chunks`, 512 `attr_item` calls each). It runs a
  27-brick neighbourhood around each re-uploaded brick (`attr_bricks`). It also visits a
  one-brick shell of 10^3 - 8^3 = 488 bricks around each chunk in `shell_chunks`, and each
  visit does up to three hash operations: `chunks.contains`, `seen.insert` and the
  `mirrored.get` in `attr_item`.
- `drop_record` inserts **every** dropped chunk into `shell_chunks`, LOD-only ones included.
  So does `upload_record` for every new record and every LOD record.
- `update_attrs` records the pass, and `AttribPass::encode` (`attrib.rs`) re-creates the work
  buffer when the list outgrows it.

The churn of a one-chunk +x focus step, computed for ideal balls at the fly-through's residency
(`r_full` 16, `r_lod` 24, hysteresis 1; `flythrough::RESIDENCY`):

| per crossing | chunks | how |
|---|---|---|
| evicted (left the `r_lod + h` = 25 ball) | 1961 | ball(25) minus its +x shift |
| loaded LOD-only (entered the 24 ball) | 1793 | shifted ball(24) minus ball(24) |
| promoted to full (entered the 16 ball) | 797 | shifted ball(16) minus ball(16) |
| demoted (left the 17 ball) | 901 | ball(17) minus its +x shift |

The evictions alone, at 488 shell bricks each, are 1961 × 488 = 956 968 brick visits in the
frame whose `poll` runs the sweep. That is the "~10^6 lookups" hypothesis. The evictions
cannot need any attribute work. An evicted chunk lies more than 25 chunks from the focus,
every full chunk lies within 17, and a neighbour is at most sqrt(3) ≈ 1.73 chunks away
(25 - 1.73 = 23.27 > 17). At 10 m/s a 6.4 m chunk is crossed every 0.64 s, about every 44
frames at plain's 68.7 fps (4123 frames in 60 s). That puts 2.3 % of the frames on a crossing,
so the p99 lands on crossing frames. **This is still a hypothesis.** The profile may instead
show the cost in `release_page`'s `free.retain`, in `select`'s scan, or in the 512
`attr_item` calls of each of the 797 promotions.

`crates/vxe-world/src/stream.rs` adds its own crossing cost. On a focus-chunk change,
`Streamer::update` rebuilds the queue: it enumerates 49^3 = 117 649 offsets, keeps the
57 777 inside ball(24), sorts them and checks each with `needs_job`. The next `poll` sweeps
every resident chunk, about ball(25) = 65 267. `GpuWorld::reconcile_window` filters every
mirrored and pending key to find the chunks leaving the window, while a one-chunk shift only
moves a slab of 51 × 51 = 2601 coordinates. The per-frame (p50) cost is the sync itself.
`select` reads each pending chunk under its lock. `upload_record` scans palettes, and
`upload_brick` narrows each `Full` brick and stages it, up to the fly-through's 2 MiB budget:
2 097 152 / (512 + 72) = 3591 brick slots and their masks per frame.

Consumer requirement served: the first consumer's first-person delve at walking and running
speed with no visible hitch (`references/project-context.md`, streaming and frame rate). The
`nfr.md` figures touched are §2 (the streaming line, and the attribute pass, which needs a
line), §5 (residency latency) and §3 (RAM, through the prepared results and the carry-over
queue).

## Decision

Profile first, then make targeted fixes that keep the frame loop's shape (ADR-0017). Phase 1
measures where `update_attrs` and `update + poll + sync` spend a crossing frame, and commits a
headless repro. The fixes come after it, each with an entry condition from the profile:

- narrow the shell pass,
- cap the attribute list with a carry-over queue,
- make the stream queue, the sweep and the window shift incremental,
- move narrowing and staging to the stream workers.

We rejected a dedicated mirror-prep thread (`GpuWorld` split into a CPU model and a GPU
uploader). It splits the one owner of ADR-0013's reservation and deferral rules across two
threads, and adds a frame of edit-to-mirror latency, before a profile shows the bounded
version misses. We rejected moving the work list and the uploads to the GPU (compute expansion
of change bits, a mapped staging ring). The cost being attacked is mostly lookups the list
does not need, and both add a new pass or a new synchronisation surface to every backend.
ADR-0017 records both.

**Stop rule.** Phase 1 ends with a verdict on the hypothesis. If it is refuted, `dev` stops
after Phase 1, writes the profile into the Implementation log and hands back to the architect.
The fixes below are aimed at the hypothesised costs, and applying them to a different cost
would be blind. Phase 5 has its own entry condition, from a profile taken after Phase 4.

## Architecture diagram

```mermaid
flowchart LR
    subgraph world[vxe-world]
        UPD[Streamer::update<br/>incremental queue] --> Q[(job queue)]
        Q --> W[stream workers<br/>generate, mips]
        W --> PREP[ChunkPrep hook<br/>palette + narrowed bricks]
        POLL[Streamer::poll<br/>shell-only sweep]
    end
    subgraph render[vxe-render]
        AE[GpuWorld::apply_events<br/>keep prepared results] --> SYNC[GpuWorld::sync<br/>slab window shift,<br/>prepared or inline narrowing]
        SYNC --> CQ[(attribute carry-over queue)]
        CQ --> UA[update_attrs<br/>capped list + NO_PLANE clears]
    end
    PREP -- StreamEvents --> POLL --> AE
    UA --> GPU[attrib pass, march]
```

## Implementation phases

Each phase ships as its own commit. `dev` implements all phases in one session with no review
between phases, subject to the stop rule above. The architect reviews once at the end, in a
fresh session. Every reading names the machine, the commit, the 1-minute load at each start
and whether the run was quiet. The quiet-box protocol is the one from Plan 0006's close
(exclusive machine lock, 1-minute load under 2 at each start, no other process above 7 % CPU).
The flights are `flythrough --autopilot --headless --seed 1`, alternated with
`--look plain`, and every flight must end with `return: 2 chunks, hash match`.

### Phase 1: Profile a crossing, and a headless repro of its work list
- **Owner skill:** dev
- **What:** The skeleton: the work-list builder becomes a `GpuWorld`-free unit, a criterion
  bench and a counting test reproduce one crossing's list without a window or adapter, and a
  profile of the real flight says where both costs go.
  - **Factor the builder** out of `GpuWorld::attr_work` into `crates/vxe-render/src/attr_work.rs`.
    It is a pure function over a read-only view of the mirror's CPU model: whether a chunk is
    mirrored full, and the entry of a brick slot of a full chunk. The function takes the three
    change sets and returns the items plus counters: chunks taken whole, dirty bricks, shell
    chunks, `visit` calls, hash operations and items. `GpuWorld` implements the view.
    Behaviour does not change.
  - **The bench**, `crates/vxe-render/benches/attr_work.rs` (criterion, workspace pin), builds a
    synthetic view at the fly-through's residency around the origin. Every chunk with
    d2 <= 17^2 is full, with every brick `Full`, which is the worst case. Every other chunk
    with d2 <= 25^2 is LOD-only. The bench times the builder on the crossing frame's
    `shell_chunks` (the 1961 evictions of a +x step) and on a promotion frame (the 797
    promoted chunks as `attr_chunks`).
  - **The profile:** the autopilot report gains per-stage lines for the flight. The streaming
    CPU is split into `update`, `poll`, `apply_events` and `sync`, timed in `stream_step`.
    `update_attrs` is split into the list build and the pass recording. The builder's counters
    are reported as p50, p99 and max over the flight, plus their values on every frame where
    `update_attrs` took over 16 ms. Then `perf record -F 999 -g --call-graph dwarf` runs on
    one default flight of a release build with line tables
    (`CARGO_PROFILE_RELEASE_DEBUG=line-tables-only` set in the environment, no manifest
    change). `perf report --no-children --tid <render thread>` gives the top 15 self-time
    symbols of the render thread, which go into the Implementation log.
- **Files touched:** `crates/vxe-render/src/attr_work.rs` (new), `crates/vxe-render/src/gpu_world.rs`,
  `crates/vxe-render/src/lib.rs`, `crates/vxe-render/benches/attr_work.rs` (new),
  `crates/vxe-render/Cargo.toml` (criterion dev-dependency, `[[bench]]`),
  `apps/flythrough/src/main.rs` (per-stage report lines), this plan's Implementation log.
- **Done when:**
  - A unit test in `attr_work.rs` drives the builder on the bench's synthetic view with the
    1961 evicted chunks as `shell_chunks`. It asserts 1961 × 488 = 956 968 `visit` calls and
    0 items: no evicted chunk neighbours a full one, since 25 - sqrt(3) > 17. A second case
    asserts that the 797 promoted chunks give 797 × 512 = 408 064 `attr_item` calls and
    408 064 items in the all-`Full` view.
  - The `crates/vxe-render/tests/mirror.rs` suite passes unchanged on lavapipe, and a new
    case there asserts that `GpuWorld::update_attrs` returns the same item count before and
    after the factoring on its existing attribute scenes.
  - The bench runs and its two medians are logged (no gate).
  - The log records, for one quiet default flight and one quiet plain flight: the per-stage
    p50, p99 and max; the counters on the frames over 16 ms; the perf top 15; and a
    **verdict**. The hypothesis is confirmed when, on every default frame where `update_attrs`
    exceeds 16 ms, shell `visit` calls are at least half of all `visit` plus `attr_item`
    calls, and perf attributes the largest self-time share inside the builder to the shell
    loop's hash operations. Otherwise it is refuted and the stop rule applies.
  - The log also names the largest p50 share inside `sync`: palette scan, narrowing,
    `stage` copies, `queue.write_buffer`, `select`'s scan, or `release_page`.

### Phase 2: The shell visits only full neighbours' border bricks
- **Owner skill:** dev
- **What:** The shell pass of a chunk `c` looks up each of `c`'s 26 neighbour chunks once.
  Only for a neighbour mirrored full does it visit that neighbour's bricks that border `c`:
  64 for a face neighbour, 8 for an edge neighbour, 1 for a corner neighbour. Those are the
  same bricks the 488-brick shell visits today, filtered by chunk. `drop_record` still records
  the shell, because a demoted chunk next to a full one does change the full chunk's border
  attributes. The cost now falls with the neighbours instead of with the shell's volume.
  `seen` stays the deduplication across the three sources.
- **Files touched:** `crates/vxe-render/src/attr_work.rs`, `crates/vxe-render/src/gpu_world.rs`
  (if `drop_record`'s insert changes shape), `crates/vxe-render/benches/attr_work.rs`.
- **Done when:**
  - The Phase 1 eviction case reads 1961 × 26 = 50 986 neighbour lookups, 0 brick visits and
    0 items.
  - An oracle test keeps the Phase 1 builder as a test-only reference. Over 64 seeded random
    small views (a 6^3-chunk region with a seeded mix of full, LOD-only and absent chunks and
    of `Full` and uniform bricks, and seeded change sets), the new builder's item set equals
    the reference's, compared as sorted sets of `(slot, origin)`.
  - The bench's eviction-frame median is logged beside Phase 1's.

### Phase 3: The attribute pass is capped, with a carry-over queue and "no plane" for deferred bricks
- **Owner skill:** dev
- **What:** `update_attrs` takes at most `attr_cap` bricks per call (a `MirrorConfig` field,
  proposal 8192) from a carry-over queue. Some rules keep this correct:
  - **The queue holds brick coordinates**, deduplicated, and records the `update_attrs` call
    index at which each was enqueued. A whole chunk is queued as its coordinate and expanded
    lazily within the cap. A brick is resolved to `(slot, origin)` when it is taken, so a slot
    freed and reused while the brick waited is never written with the old brick's origin. A
    brick whose chunk is no longer mirrored full is skipped.
  - **Re-uploaded (edited) bricks and their 26 neighbours go to the front** and are never
    deferred. A carve shows its contour in the same frame, as it does today.
  - **A deferred brick never shows stale planes.** A brick written to a slot this sync whose
    compute item does not run this frame gets a clear item. Clear items are flagged in the
    item (illustrative: the slot word's top bit). The pass writes `NO_PLANE` (0xFFFF) to the
    brick's attribute words and returns. So until its turn comes, the brick hits as face mode,
    which is what ADR-0011's "no plane" already means to the march. Clear items are not
    capped; they are bounded by the bricks one sync writes.
  - **The work buffer is sized once.** It holds `attr_cap` items plus the clear items one sync
    can produce under the configured `sync_budget_bytes`: 8192 + 3591 + 512 = 12 295 items at
    the fly-through's 2 MiB, which is 196 720 B at 16 B per item. The 512 covers the one chunk
    a sync may finish past its budget. With an unbounded sync budget it grows as today.
  - The fly-through reports the oldest carried-over brick's age at every `update_attrs`, in
    milliseconds through its frame timestamps, as p50, p99 and max over the flight.
- **Files touched:** `crates/vxe-render/src/attr_work.rs`, `crates/vxe-render/src/gpu_world.rs`,
  `crates/vxe-render/src/attrib.rs`, `crates/vxe-render/src/shaders/attrib.wgsl`,
  `crates/vxe-render/tests/mirror.rs`, `apps/flythrough/src/main.rs`.
- **Done when:**
  - A lavapipe test in `tests/mirror.rs` uses `attr_cap` 64 and a scene whose first sync
    writes K > 64 `Full` bricks into attribute pages; the test prints K. The first
    `update_attrs` returns 64 compute items. Read back, every other new brick's two attribute
    slots are all `NO_PLANE`. After ceil(K / 64) calls the attribute bytes of every brick equal
    those of an uncapped mirror on the same scene, bit for bit.
  - A carve applied while the queue holds more than 64 waiting bricks has every one of its
    re-uploaded bricks in the very next call's list.
  - A slot freed and reused while its old brick waited ends with the new brick's attributes,
    equal to an uncapped mirror.
  - `AttribPass::buffer_builds()` reads 1 after the autopilot's first flight frame and at its
    end.
  - The `shader_portability` test still validates `attrib.wgsl`.

### Phase 4: An incremental stream queue, residency sweep and window shift
- **Owner skill:** dev
- **What:** All three changes are pure bookkeeping and change no decision.
  - **The streamer.** When the single focus moves by one step on each axis (|Δ| <= 1, the 26
    unit steps), `Streamer::update` adds the jobs for chunks entering the wanted ball and
    drops queued jobs that left it. The entering offsets are cached per step vector as the
    difference of the ball's offset sets. It re-keys the existing queue by the new `d2` and
    sorts it. The resulting queue order and contents are exactly what `rebuild_queue` would
    produce, `wanted_since` included. The next `poll`'s sweep visits only the leaving shells
    of the `r_full + h` and `r_lod + h` balls, plus a recheck set of the chunks inserted
    outside their band since the last sweep (today's "the sweep demotes it" case). A jump,
    several focus points, `set_radii` or the first `update` keep today's full rebuild and
    full sweep.
  - **The mirror.** `GpuWorld::reconcile_window` finds the chunks leaving the window from the
    slab `box_difference(old, new)` by direct lookups in `mirrored` and `pending_d2`, instead
    of filtering every key.
- **Files touched:** `crates/vxe-world/src/stream.rs`, `crates/vxe-world/tests/stream.rs`,
  `crates/vxe-render/src/gpu_world.rs`, `crates/vxe-render/tests/mirror.rs`.
- **Done when:**
  - In `crates/vxe-world/tests/stream.rs`, a streamer runs a seeded 300-step walk: unit steps
    in all 26 directions, a jump of 10 chunks every 60 steps, and one `set_radii` mid-walk. It
    uses a test-only check that recomputes the full rebuild and the full sweep from the same
    state. After every step, the incremental queue equals the rebuilt one as an ordered list
    of keys, and the sweep's evict and demote lists equal the full scan's.
  - The existing residency tests (`resident set's world hash identical with 1 and 8 workers`
    and the persistence ones) pass unchanged.
  - A unit test in `gpu_world.rs` drives `reconcile_window` through 50 seeded shifts of
    -2..=2 chunks per axis. The leaving set equals the old filter's set, and the table entries
    cleared are the same.
  - The autopilot's per-stage `update` and `poll` max are logged beside Phase 1's.

### Phase 5: Narrowing and staging on the stream workers
- **Owner skill:** dev
- **What:** Entry condition. First re-run the Phase 1 profile on one quiet plain flight at the
  Phase 4 tip. Proceed only if narrowing is the largest share of `sync`'s p50: palette scan,
  `upload_brick`'s narrowing and the `stage` copies together. If another cost leads,
  `queue.write_buffer`'s copies or `select`'s scan for example, stop, log the profile and
  hand back to the architect. When the condition holds:
  - **The hook.** `vxe-world` declares a `ChunkPrep` hook. The `Streamer` takes it optionally
    and runs it on the worker after a full chunk is generated or restored, and after a mip
    build. Its opaque, `Send + Sync` result rides `StreamEvents` next to `loaded` and
    `demoted`.
  - **What it prepares.** `vxe-render` implements the hook. For a chunk that will get a new
    narrow record, it builds the first-appearance chunk palette and each `Full` brick's slot
    bytes and masks, the same bytes `upload_brick` would stage. It reads the translucency
    flags from the world's material table on the worker.
  - **How `sync` uses it.** `GpuWorld::apply_events` keeps each prepared result keyed by
    coordinate. `upload_record` uses it only for a record written new and narrow from content
    identical to what was prepared, under the same material table. Every other case narrows
    inline as today: a chunk edited between preparation and sync, a wide chunk, a re-uploaded
    record, or a missing result. The render thread still allocates slots, writes the record
    and issues every `queue.write_buffer`.
  - **A RAM cap.** Prepared results are held to a cap of 64 MiB (proposal). That is 224
    worst-case chunks at 512 × 584 = 299 008 B each. A chunk dropped or evicted drops its
    result, and past the cap the oldest result is dropped and that chunk narrows inline.
- **Files touched:** `crates/vxe-world/src/stream.rs`, `crates/vxe-world/src/lib.rs`,
  `crates/vxe-world/src/chunk.rs` (only if content identity needs a revision counter,
  incremented deterministically on every write), `crates/vxe-render/src/prep.rs` (new),
  `crates/vxe-render/src/gpu_world.rs`, `crates/vxe-render/tests/mirror.rs`,
  `apps/flythrough/src/main.rs` (installs the hook, reports peak prepared bytes).
- **Done when:**
  - A lavapipe test in `tests/mirror.rs` streams the seed-1 cave to residency 4/6 twice, once
    with the hook and once without. It reads back the pool shards, masks, records and table
    and asserts them equal byte for byte. It also asserts that the hooked run took the
    prepared path for at least one chunk, by a counter in `SyncStats`.
  - A second case carves a chunk after its worker prepared it and before its sync. That chunk
    is narrowed inline, and the mirror equals the hook-free mirror.
  - `vxe-world`'s cross-platform hash tests are unchanged, since the hook writes nothing into
    the world.
  - The autopilot reports peak prepared bytes under 64 MiB, and the log records the `sync`
    p50 beside Phase 1's.

### Phase 6: Readings against the bars
- **Owner skill:** dev
- **What:** Four quiet-box flights at the tip, alternated default, plain, default, plain, each
  started at a 1-minute load under 2, with the exclusive perf lock. Every line goes in the
  Close triggers with machine, commit and loads.
- **Files touched:** this plan's Implementation log only (the close moves readings into
  `docs/nfr.md`).
- **Done when:** every bar below reads true in both pairs. A bar that misses is logged as a
  miss with its numbers, not re-run until it passes.
  - `update_attrs` CPU p99 under 4 ms in both default runs (today 138 to 180 ms).
  - Each default run flies at least 95 % of its paired plain run's frames (today 2211 against
    4123 and 2695 against 4155; the bar is 3917 and 3948 frames if plain reads the same
    again). See Risks: the attribute pass's GPU time may hold this bar out of reach of CPU
    work alone.
  - Streaming CPU (`update + poll + sync`, as `stream_step` times it) p50 under 1.0 ms in all
    four runs, and its max under 16 ms in all four. Its p99 is recorded and does not gate.
  - Plain's residency latency p95 at or under 381 ms in both plain runs.
  - The longest attribute carry-over age at or under 250 ms in both default runs.
  - `return: 2 chunks, hash match` in all four runs.
  - The attribute pass's GPU p50, p99 and max and the work list's p50, p99 and max are
    recorded, not gated.

### Phase 7: The owner's windowed flight
- **Owner skill:** human
- **What:** The owner flies `flythrough --seed 1` windowed, default look, with stdout visible.
  The flight includes a sustained fast pass across many chunk boundaries in both directions
  and a few carves, and the owner judges two things. Is the hitch gone? Is the up-to-250 ms
  face-mode lag on newly streamed rock invisible, or at least unobjectionable? A carve must
  show its contour in the frame it lands. `dev` sends a push notification when the build is
  ready, batched with any other pending live checks.
- **Files touched:** this plan's Implementation log (the owner's verdict, dated).
- **Done when:** the owner's verdict on both questions is recorded. A "no" on either goes to
  the close review as a finding.

## Data shapes

```rust
// illustrative: vxe-render, the builder's view and counters
pub trait MirrorView {
    fn is_full_chunk(&self, c: ChunkCoord) -> bool;
    /// The slot of world brick `b` if it is a `Full` brick of a mirrored full chunk.
    fn full_slot(&self, b: [i32; 3]) -> Option<u32>;
}
pub struct AttrWorkStats { pub whole_chunks: u32, pub dirty_bricks: u32,
    pub shell_chunks: u32, pub visits: u64, pub lookups: u64, pub items: u32 }

// illustrative: vxe-world, the worker hook (opaque to the world)
pub trait ChunkPrep: Send + Sync {
    fn full(&self, coord: ChunkCoord, chunk: &Chunk, materials: &MaterialTable) -> Option<Prepared>;
    fn lod(&self, coord: ChunkCoord, mips: &ChunkMips, materials: &MaterialTable) -> Option<Prepared>;
}
pub type Prepared = std::sync::Arc<dyn std::any::Any + Send + Sync>;
```

The attribute work item keeps its 16 bytes (`AttribItem { slot, origin }`). A clear item is
marked inside `slot` (illustrative: bit 31; slots fit in 29 bits).

## Budgets

| `nfr.md` figure | today (quiet box, 2796062) | this plan's bar | how the close measures it |
|---|---|---|---|
| §2 streaming line, 1.0 ms | p50 4.5 to 5.0 ms, p99 33 to 65 ms, max 41 to 108 ms | p50 under 1.0 ms; max under 16 ms; p99 recorded | Phase 6, `stream_step`'s timing, four flights |
| §2 attribute pass, render-thread CPU (**no line yet; proposed here**) | p50 4.6 to 4.7 ms, p99 138 to 180 ms | p99 under 4 ms; carry-over age at most 250 ms | Phase 6, default flights |
| §2 frame rate in motion (diagnostic) | default 2211 and 2695 frames against plain's 4123 and 4155 | default at least 95 % of plain | Phase 6 pairs |
| §5 residency latency, 500 ms p95 proposal | plain p95 381 and 381 ms | plain p95 at most 381 ms | Phase 6 plain flights |
| §5 edits persist across eviction | `return: 2 chunks, hash match` | unchanged | every flight |
| §3 memory | mirror reservation fixed (ADR-0013); RAM per §3 | unchanged; prepared results at most 64 MiB of RAM, the work buffer 196 720 B | Phase 5 peak prepared bytes, Phase 6 peak resident RAM line |

**Proposed `nfr.md` §2 line, for the close to write:** "Mirror attribute pass (contour look),
render-thread CPU: p99 under 4 ms on the dev reference, deferred work visible as face-mode
normals for at most 250 ms". The pass's GPU time stays a recorded figure here. Plan 0013's
1.0 ms GPU proposal remains unmet and unowned, and it is listed below as not done.

## Risks & open questions

- **The 5 % frame bar may be out of reach of CPU work.** During flight the autopilot reads its
  timestamp queries back synchronously every frame (`MarchTimer::read_ms` waits on the device).
  So a flight frame costs its CPU plus its GPU. Plain flies at about 14.5 ms a frame (60 s over
  4123 frames). The bar allows the default about 0.77 ms more per frame (3917 frames).
  The attribute pass's GPU reads p50 2.4 to 3.0 ms on 67 to 78 % of the default's frames
  (1717 of 2211, 1802 of 2695), roughly 1.6 to 2.3 ms per frame on average. Capping only
  spreads that work, because the bricks per second are set by the flight speed. That alone
  could cost the default about 10 to 14 % of plain's frames with zero CPU cost. The carry-over
  queue's deduplication of bricks re-queued by consecutive neighbour promotions may recover
  some of it, but how much is unknown. **Owner question, asked with this draft.**
- **Refuted hypothesis.** The stop rule covers it: Phase 1 hands back, and nothing later
  starts.
- **Prepared-result staleness** is the correctness risk of Phase 5. The byte-equality test and
  the edit-between test defend it. The fallback is always inline narrowing, so a missed case
  costs time, not correctness, as long as the identity check is conservative.
- **Threading.** The hook runs on workers that already hold no lock across generation. It
  must not take the store's map lock or any `GpuWorld` state, and it receives the chunk by
  reference before insertion. No channel is added that the render thread could block on:
  results ride the existing `Done` channel, which `poll` drains with `try_recv`.
- **Determinism.** `vxe-world` stays deterministic. The incremental queue must equal the full
  rebuild's queue (Phase 4's oracle), and the hook writes nothing into the world. A revision
  counter, if added, is integer and incremented on every write.
- **Residency latency.** Worker time spent narrowing delays the next generation job. The
  plain p95 bar of 381 ms guards it. If it fails, the cheaper move is preparing full chunks
  only and leaving LOD mips inline.

## What this plan does NOT do

- **The default look's mirror footprint** (surface-only attribute storage, `r_full` held at 12
  to 13 under pressure): ADR-0012's mirror-slot plan.
- **The attribute pass's GPU cost** (Plan 0013's 1.0 ms p99 proposal, read 4.3 to 5.6 ms).
  It is recorded here, not attacked.
- **Lighting and light extraction**, the third item on the streaming line: M2's lighting plan,
  which this plan precedes.
- **Baseline-GPU or baseline-CPU readings.** Every bar here is dev reference. The baseline
  card is not on hand (`nfr.md` §1).
- **Incremental updates for several focus points**, or for jumps: they keep the full rebuild.
- **The level-size first sync's palette scan** (Plan 0002's followup, about 500 ms). The
  prepared path may shorten it, but it is not measured or gated here.
- **The far-field clipmap** and any change to the residency radii or their defaults.

## Implementation log

> Written by `dev`: one row per phase as its commit lands, then the close block. **The phases
> above are the contract. This section records what happened.** Observations, never conclusions:
> no pass list, no self-assessment. Deviations and unmet done-whens are **always** disclosed.
> Keep it shorter than `## Implementation phases`.

| phase | owner | state | commit |
|---|---|---|---|
| 1: Profile a crossing, and a headless repro | dev | not started | |
| 2: The shell visits only full neighbours | dev | not started | |
| 3: Capped attribute pass with carry-over | dev | not started | |
| 4: Incremental queue, sweep and window shift | dev | not started | |
| 5: Narrowing and staging on the workers | dev | not started | |
| 6: Readings against the bars | dev | not started | |
| 7: The owner's windowed flight | human | not started | |

### Notes

_(Deviations from the plan, with the commit, stated without justification. Done-whens not
satisfiable as stated, and what was done instead. Followups noticed and not acted on. One line
each. Empty is fine.)_

### Close triggers

_(Facts for the architect. No recommendations, and no suggested version bump.)_

- **What shipped:** feature / fix-only / docs-chore-only
- **Public API changed:** types, traits, functions, shader interfaces, config (list them, or none)
- **Budget readings:** each `nfr.md` figure touched, with its measured value, machine and commit
- **Gate at the tip:** the commands run (build, nextest, doctests, clippy, fmt, deny), exit codes, test counts
- **Outstanding `human` phases:** which, or none

## Followups

- The attribute pass's GPU cost against Plan 0013's 1.0 ms proposal (unowned).
