# 0017: The mirror reserves its budget: fixed buffers, empty pages that change kind, and a pressure signal the streamer follows

> **Status:** done (closed 2026-10-20, v0.17.0, fourth review at 16e17a7: the per-chunk admit check is exact for every need it admits; the reservation, page kinds and pressure rule land as planned)
> **Created:** 2026-10-17
> **Milestone:** M2 in `docs/roadmap.md` (the footprint work that lets Plan 0013's radius be chosen against content)
> **Related ADRs:** [ADR-0013](../../adrs/0013-mirror-reserves-its-vram-budget-up-front.md)
> (accepted; this plan implements it), [ADR-0004](../../adrs/0004-palette-narrowed-sharded-brick-pool.md)
> (whose growth Outcome it supersedes in part), ADR-0003 (the residency radii the pressure
> rule moves).
> **Prerequisite:** none. Touches `gpu_world.rs`, which Plans 0013 and 0015 also touch;
> merge order resolves the conflicts. Best taken after Plan 0015 so the 72-byte mask slot is
> reserved once.
> **Unblocks:** [Plan 0013](0013-contour-default-shadow-origin-and-streamed-footprint.md)
> Phase 5, which waits for this plan (the owner's call, 2026-10-18). When this plan merges to
> main, continue Plan 0013 on its branch: merge main into `plan-0013`, then Phases 5 and 7
> (its human Phases 4 and 6 are done).
> **Source:** `docs/decisions-pending.md` (answered and deleted; read it with `git show 880982c:docs/decisions-pending.md`) D1 and D7 item 4; ADR-0004's Outcome; `nfr.md` §3's
> two recorded misses.

## TL;DR

The mirror's buffers grow by half again, copy themselves on the GPU when they do, and never
shrink, so allocated VRAM runs 1.44x to 1.93x above live bytes and the budget line is failed
by content that fits. This plan makes the mirror reserve its budget once at creation and
never create another buffer, lets an empty page serve either kind so attribute and plain
slots share one high-water mark, and gives the application a pressure figure (the larger of
pool-slot and record occupancy, amended 2026-10-20) that the fly-through's streamer answers by lowering the full-detail radius a chunk
at a time. The first thing a reader sees is `allocated_bytes` equal to the budget from the
first frame, and the autopilot's peak live at the interim radius 14 with contour under
3 GB with no out-of-memory stop.

## Context & problem

`grow_to` (`crates/vxe-render/src/gpu_world.rs:1560-1626`) allocates `max(need, 1.5 x
current)` rounded to 4096-slot pages, copies the old slots and masks in one submit, and the
record buffer does the same in 1024-record pages (`:1429-1471`). `drop_record` pushes freed
slots onto their kind's free list (`:850-853`); no page or buffer is ever released, and
`page_kind` only grows (`:370-371, 1481-1492`). `allocated_bytes` (`:695-701`) is the sum
of the buffer sizes; `mirror_bytes` (`:683-689`) the live slots. The readings: 1.69 GB live
against 2.43 GB allocated under motion at the defaults; 3.31 GB live against 4.10 GB
allocated with contour, then `wgpu error: Out of Memory` on the 8 GiB card (`nfr.md` §3).
Plan 0013's footprint table (D1) had to be read on allocated bytes, which sank every
16-chunk row; the owner chose radius 14 as the interim and asked for this rule.

The mechanism, from the code: a plateau reached by grows has about 4 % slack; the
streamer's hysteresis pushes live past it under motion and one more step adds half again;
with attribute pages the two kinds cannot serve each other, so the pool tracks the sum of
each kind's peak; and during a grow the old and new shard coexist, about 2.5x the old
shard, uncounted, which is the plausible cause of the OOM.

Budgets touched: `nfr.md` §3 world VRAM (allocated becomes the budget; live is the reading
against it), §7 time to first frame (the reservation at startup), §2 streaming line (the
grow copies, a hitch source, disappear).

## Decision

`MirrorConfig` carries `budget_bytes`; `GpuWorld::new` splits it over the shards and the
record buffer by the adapter's limits and creates every buffer once; `grow_to` becomes a
capacity check that fails the sync before any write, as `PoolTooLarge` and
`RecordsTooLarge` do today. A page tracks its live slot count; at zero it returns to a
free-page list that either kind draws from. `GpuWorld::pressure()` returns the larger of
pool-slot occupancy and record occupancy (amended 2026-10-20; it was live over budget), and
the fly-through's streamer lowers `r_full` by one chunk when pressure exceeds
0.9 for 30 consecutive frames and raises it by one when pressure has been under 0.7 for
300 frames, between a floor of 6 and the configured radius. We rejected compaction (slots
move, every record and attribute address with them, and the copy is itself a hitch), growth
by exact need (hundreds of copies over a flight) and per-page buffers (binding arrays,
rejected in ADR-0004); ADR-0013 records them.

## Amendment 2026-10-20: records for the sphere, pressure on what runs out, a reachable budget

Phase 3's third done-when failed (Implementation log, Resume notes): at `--mirror-budget 1024`
the pool ran out before any radius change. Two causes. Phase 1 reserves a record for every
chunk of the 51^3 window cube, 132 651 records or 348 076 224 B (32.4 % of 1 GiB), while
57 777 are live at rest; pressure was live bytes over the whole reservation, so it peaked near
0.78 and the 0.9 trigger never fired. And the rule moves only `r_full`: at its floor (6, 24)
the LOD ring alone needs 1285.5 MiB at rest (`world_budget --stream 6,24`), so no budget near
1 GiB is reachable. The owner chose the three changes below. The LOD radius never moves;
ADR-0013 stands, with a dated Outcome on the pressure definition. Phase 3 re-opens Phase 1's
record count (one `dev` phase, no new step), and Phase 4 reads the result.

**1. Records for the residency sphere.** A record is `RECORD_BYTES` = 4 x (16 brick-bit
words + 512 entries + 128 palette words) = 2624 B. The streamer evicts a chunk once
`d2 > (r_lod + h)^2`, so after a poll one focus holds at most the lattice chunks of that
ball: `chunks_in_ball(25)` = 65 267 at the defaults (at rest `chunks_in_ball(24)` = 57 777,
which is `world_budget`'s 925 + 56 852). The streamed mirror reserves
`ceil(chunks_in_ball(max(r_lod, r_full) + h) / PRESSURE_LOW)` = ceil(65 267 / 0.7) =
**93 239 records, 244 659 136 B (233.3 MiB)**, against the cube's 348 076 224 B: 103 417 088 B
less. The division by 0.7 keeps the record term below the rule's restore line whenever the
streamer keeps its radius (0.620 at rest, at most 0.69999 in motion). Moving `r_full` does not
change the record count (a LOD-only chunk holds a record too), so a record term that crossed
0.7 would block every restore and one past 0.9 would walk `r_full` to its floor for nothing;
with exactly 65 267 records it would read 0.885 at rest and reach 1.0 in motion. It reaches 1
only when residency outgrows the sphere (several foci), which no radius rule cures, and
`RecordsTooLarge` reports it. A mirror given no record count keeps the window's `W^3`, so
the level and the tests are unchanged.

**2. Pressure on what runs out.** `pressure() = max(slot occupancy, record occupancy)`, with
slot occupancy `(live_slots() + 2 x live_attr_slots()) / reserved_slots()` (a brick in an
attribute page holds its two attribute slots too) and record occupancy `mirrored_chunks() /
reserved_records()`. Slot occupancy falls as bricks are freed, unlike the high-water mark
`pool_slots()`. With attribute pages a pool can fail a little below 1.0 (each kind's partly
filled page, four unused slots an attribute page); the 0.1 between trigger and exhaustion
covers it. Live over budget stays a reported figure: it is not the trigger, and it counts the
chunk table and material rows that lie outside the reservation (Phase 1's `allocated_bytes`
note).

**3. A budget the r_full-only rule can reach: `--mirror-budget 1930`.** A page is
`PAGE_BYTES` = 4096 x (512 + 72) = 2 392 064 B. Live slots from `world_budget --stream R_FULL,R_LOD
--seed 1` (face, h 1, at rest, this commit's tree) and the Phase 3 reading at 3072 MiB,
against 743 pages (3 043 328 slots):

| residency | what it stands for | live slots | slot occupancy |
|---|---|---|---|
| 6, 24 | the floor at rest | 1 709 813 | 0.562 |
| 7, 25 | the floor in motion: everything its hysteresis keeps | 1 957 717 | 0.643 |
| 15, 24 | `r_full` 14 with its hysteresis, at rest | 2 602 153 | 0.855 |
| 16, 24 | the start at rest | 2 798 808 | 0.920 |
| 16, 24 flight peak (measured, 3072 run) | the start in motion | 2 928 253 to 2 961 907 | 0.962 to 0.973 |
| 17, 25 | the start in motion, everything kept (a bound) | 3 241 039 | 1.065 |

The window: the rule must fire at the start, 2 798 808 / S > 0.9, so at most 759 pages; the
pool must hold the measured start peak even if the rule were late, S >= 2 961 907, so at least
724 pages; the floor must sit under the trigger with its hysteresis, S >= 1 957 717 / 0.9, at
least 532 pages (implied). 743 pages sits mid-window. Budget = 743 x 2 392 064 + 93 239 x
2624 + `PLACEHOLDER_BYTES` 2624 = 1 777 303 552 + 244 659 136 + 2624 = 2 021 965 312 B
(1928.3 MiB); `--mirror-budget 1930` (2 023 751 680 B) reserves floor(1 779 089 920 / 2 392 064)
= 743 pages. 2048 MiB reserves 795 pages, where the start reads 0.859 at rest and about 0.91
only at the flight's peak, too briefly to promise the 30 frames: it does not reliably decrease.
At 3072 MiB the split is 1244 pages and the measured peak reads 0.581, records at most 0.69999:
no change. Projection, a guess from at-rest figures: 16 to 15 demotes nothing (the keep
radius 16 still holds ball 16), 30 frames later 15 to 14 demotes the 16-shell and slots fall
to about 0.855, between the two lines, so `r_full` rests at 14 after two decreases. The
(17, 25) bound does not fit, so zero `PoolTooLarge` at 1930 rests on the measured peak and
the rule acting; it is a reading, not a proof. Peak live over budget at 1930, without the
rule, would be 1 881 891 684 / 2 023 751 680 = 0.930.

## Amendment 2026-10-20 (second): a sync writes what fits, nearest first

The review fix pass re-read contour at the default 3072 MiB after `pool_need` began crediting
tier-change frees (Implementation log, Resume notes). Two of four runs still failed syncs: 9 at
16/24 (longest run 6) and 20 at 14/22 (longest run 20). Every failure came above the trigger
(0.908 and 0.906) and each was exactly one page short. About 9 % of slots were spare, but
almost all were free single slots in plain pages that each still hold a live mip-brick, so
the attribute pages that promotions need could not use them. A failed sync rejected the whole
selection, nearest and dug chunks included, so the mirror showed no edit until the rule's
decrease cleared it. The owner chose a partial sync over a page-occupancy pressure term. The
rule, its thresholds, the record sizing and `pressure()` are unchanged.

**The contract.** A sync writes what fits, nearest first, and never part of a chunk.
`select` walks the pending set nearest the focus first, as now, and replays the fit as it
walks (`pool_need`'s replay moves into it). Each chunk's own tier-change frees are credited
before its own allocations, as `pool_need` does at 7112269. Each chunk's need is then checked
against what is left, in slots of its kind, free pages and pages never handed out, and in
records. The first chunk that does not fit is **deferred**, and so is every later
chunk, with one exception: a **demotion** (a mirrored full chunk whose source is now LOD-only)
still uploads if it fits on the replayed state. Demotions are the radius rule's lever, and a
deferred chunk near the focus (a dug one) must not block the frees the rule asked for behind
it. A deferred chunk is not counted against `sync_budget_bytes`, so the walk reaches the
demotions behind it. A deferred chunk stays in the pending set with its dirty bits, and
keeps its mirrored record of either tier: a deferred promotion does not drop its LOD record.
Deferral is decided before any write. A chunk that turns wide during upload (ADR-0004's rare
path) has its doubled need re-checked before its first slot is taken. If it no longer fits,
it is deferred whole, with every non-demotion after it. No staged run of a deferred chunk is
flushed.

**What is a failure.** A sync that uploads at least one chunk returns `Ok`, with
`SyncStats::deferred_chunks` counting the chunks it deferred. A sync that defers chunks and
uploads none returns `PoolTooLarge` or `RecordsTooLarge` as before, writing nothing (an
**empty sync**). This happens when the nearest selected chunk does not fit and no demotion
behind it does. The old whole-selection rejection, a sync that wrote nothing although its
nearest chunk fitted, cannot happen. An empty sync is still expected at a full pool. Once a
partial sync has spent the last attribute spare, the next sync's nearest pending chunk is
the same stuck edge promotion until the rule's decrease makes it stale. So the bar below is
on how fast deferrals clear, not on zero empty syncs.

**What the fly-through reports.** A **deferral streak** is consecutive syncs that each defer
at least one chunk; empty syncs count toward it. At each such sync it notes the pressure and
its two terms (the fix pass's recording, now keyed on deferral). It prints the first
deferral's pool readings as now. The autopilot report prints:
- the deferred-chunk total (summed over syncs, so a chunk deferred five times counts five);
- the syncs with a deferral and the empty syncs among them;
- the pressure and term ranges at deferral;
- the longest deferral streak in syncs, a streak still open at the flight's end included.

The fly-through syncs once per frame, so syncs are frames.

**The bar, and N.** A deferral needs a full pool, and every reading put that above the 0.9
trigger. From a streak's first sync, the rule lowers `r_full` after at most
`PRESSURE_HIGH_FRAMES` = 30 frames above 0.9 (fewer if pressure crossed earlier). With h = 1
that first decrease can demote nothing (the first Amendment's 16 to 15, whose keep radius
still holds the old ball), so the second lands by frame 60. Its demotions then reach the
world in the next polls and the mirror in the sync after. The stuck edge promotions go stale
at the same time, since their chunks are LOD-only in the world. One more rule period, 30
frames, covers that landing, which is a guess the readings check. **N = 2 x 30 + 30 = 90
syncs.** The fix pass's longest runs, 6 and 20, sit inside it. A streak longer than 90, or
any deferral at pressure at or under 0.9 that does not clear within 90, is a stall the rule
does not clear. The dev stops and hands back with the log readings (Phase 4).

**Ownership.** The mirror contract, its tests and the fly-through's reporting are Phase 3's
(the pressure loop, whose Files touched already hold `gpu_world.rs`, `mirror.rs` and
`main.rs`). The readings are Phase 4's. Phase 1's done-when on a failing sync now holds for
the empty sync only. Both phases keep their single `dev` tag. ADR-0013 carries a dated
Outcome line.

## Architecture diagram

```mermaid
flowchart LR
    subgraph vxe-render
        N[GpuWorld::new: budget] --> B[shards, masks, records at full size]
        S[sync] -->|nearest-first chunks that fit, and demotions| A
        S -->|the rest stay queued| E[deferred, or PoolTooLarge when none fit]
        A[alloc: free list, then free pages of any kind, then cursor]
        D[drop_record] --> A
        P[pressure = max of slot and record occupancy]
    end
    subgraph apps/flythrough
        P --> R[radius rule: -1 at 0.9, +1 at 0.7]
        R --> W[Streamer radii]
    end
```

## Implementation phases

### Phase 1: One reservation, no second buffer
- **Owner skill:** dev
- **What:** `MirrorConfig::budget_bytes` (the fly-through default is the `nfr.md` §3 line,
  3 GB, overridable by `--mirror-budget MiB`; tests pass small budgets); `ShardLayout`
  gains the split (records get `RECORD_BYTES` per chunk of the window's capacity, capped by
  the binding; the rest goes to pool pages across the shards in order; masks follow the
  pool); every buffer is created in `new`; `grow_to` only checks; `SyncStats` counts
  `buffers_created` and `copies`.
- **Amended 2026-10-20:** the streamed mirror's record count ("a record per chunk of the
  window's capacity") is re-done in Phase 3 (see the Amendment); this phase's done-whens stand
  as written and were read against the window's `W^3`.
- **Files touched:** `crates/vxe-render/src/gpu_world.rs`, `crates/vxe-render/tests/mirror.rs`,
  `apps/flythrough/tests/level_mirror.rs`, `apps/flythrough/src/main.rs` and `lib.rs`
  (the flag and the default sized from the world: the level's extent, or the radii),
  `apps/flythrough/examples/world_budget.rs` (prints the budget a radius needs; fixes the
  material row to `MATERIAL_FLOATS`), `README.md`.
- **Done when:**
  - `allocated_bytes()` equals the budget (rounded down to whole pages and records) right
    after `new`, and is unchanged after any number of syncs, evictions and window moves;
    `SyncStats::buffers_created == 0` and `copies == 0` over a whole
    `a_following_window_mirrors_the_streamed_residency` run.
  - A sync whose pool need exceeds the reservation fails with `PoolTooLarge { need,
    capacity }` before writing, as `a_world_past_four_shards_fails` asserts today with the
    budget in place of the layout limit; records likewise. (Amended 2026-10-20, second
    Amendment: this now holds for an empty sync, one whose nearest selected chunk does not
    fit and no demotion behind it does; a sync with a prefix that fits uploads it, Phase 3.)
  - On lavapipe the level test passes a 256 MiB budget and reads `live_pool_bytes() ==
    196 407 296` as today.
  - `--mirror-budget 3072` on the dev box creates the buffers in under 50 ms (a guess; the
    log records the reading) and the first frame of the streamed cave is within 10 % of
    Plan 0002's reading.

### Phase 2: Empty pages change kind
- **Owner skill:** dev
- **What:** per-page live counts; a page whose count reaches zero leaves its kind's cursor
  and free lists and joins `free_pages`; `alloc` takes a free page of any former kind
  before advancing the cursor, re-kinding it; attribute pages are issued the same way.
- **Files touched:** `crates/vxe-render/src/gpu_world.rs`, `crates/vxe-render/tests/mirror.rs`.
- **Done when:**
  - Kind churn: a test alternates 20 times between a residency where 8 chunks are full with
    attributes and one where the same 8 are LOD-only, in a following window; the pages ever
    issued stay at most `ceil(peak slots of one kind / page slots) + 1` per kind, where the
    old code issues one new page per promotion (the test asserts the bound on
    `pages_issued()`).
  - `attribute_pages_hold_full_bricks_and_plain_pages_mip_bricks` still holds (a page is of
    one kind while it has live slots).
  - No slot number changes for a live brick (the attribute addressing stays valid): a
    readback after the churn decodes every live brick to its CPU voxels.

### Phase 3: Pressure, and the streamer follows it
- **Owner skill:** dev
- **What:** `GpuWorld::pressure() -> f32`; `Streamer::set_radii` if the radii are fixed at
  creation (they are read from `StreamConfig` at `stream.rs`; the plan assumes a setter is
  needed); the fly-through's rule above, printing each change and showing it on the HUD.
  Amended 2026-10-20, re-opening Phase 1's record count: `pressure()` is the larger of slot
  and record occupancy; `MirrorConfig::record_chunks: Option<u64>` (none keeps the window's
  `W^3`); `vxe_world::chunks_in_ball(r)`, the lattice chunks with `d2 <= r^2` in the
  streamer's metric; the fly-through's streamed mirror reserves
  `ceil(chunks_in_ball(max(r_lod, r_full) + h) / PRESSURE_LOW)` records; `world_budget`
  prints its budget lines with that record count; `README.md` lists `--residency`,
  `--mirror-budget` and the rule.
  Amended 2026-10-20 (second Amendment), in the review fix pass: the partial sync. `select`
  replays the fit as it walks and defers the first chunk that does not fit and every later
  chunk except demotions that fit; deferral is decided before any write; a chunk turning wide
  is re-checked before its first slot; `SyncStats::deferred_chunks`; a sync that uploads
  nothing and defers something returns the existing errors untouched. The fly-through counts
  deferrals, empty syncs and the longest deferral streak, and keeps the pressure ranges at
  deferral, in place of the failed-sync counter.
- **Files touched:** `crates/vxe-render/src/gpu_world.rs`, `crates/vxe-world/src/stream.rs`,
  `crates/vxe-world/src/lib.rs`, `apps/flythrough/src/main.rs`, `apps/flythrough/src/lib.rs`,
  `apps/flythrough/src/hud.rs`, `apps/flythrough/examples/world_budget.rs`,
  `crates/vxe-render/tests/mirror.rs`, `crates/vxe-world/tests/stream.rs`, `README.md`.
- **Done when:**
  - `pressure()` equals `max((live_slots() + 2 x live_attr_slots()) / reserved_slots(),
    mirrored_chunks() / reserved_records())` within `1e-6`; one mirror test drives it past 0.9
    on slots with records under 0.5, another on records with slots under 0.5.
  - `chunks_in_ball(24) == 57_777` and `chunks_in_ball(25) == 65_267`; the fly-through's
    record count at r_full 16, r_lod 24, h 1 is 93 239; a mirror given `record_chunks` reserves
    exactly that many and `allocated_bytes()` still equals its budget rounded down.
  - `Streamer::set_radii(r_full - 1, r_lod)` demotes the chunks at the old edge to LOD-only
    within the next polls, and a later `set_radii` back promotes them; the hashes of their
    voxels after the round trip equal the originals (persistence, `tests/persist.rs`'s
    helpers); in the same test the resident chunk count after every poll is at most
    `chunks_in_ball(r_lod + h)`.
  - Headless `flythrough --autopilot --mirror-budget 1930` (r_full 16, r_lod 24, h 1, seed 1,
    no `--look`) prints a reservation of 3 043 328 slots and 93 239 records, finishes its
    flight with zero `PoolTooLarge` and zero `RecordsTooLarge` syncs, prints at least one
    radius decrease and none below 6, and reports peak pressure under 1.0 and peak live /
    budget under 0.95 (0.930 projected without the rule; the Amendment projects two decreases,
    to 14, a guess). The same run at 3072 prints no change.
  - Partial sync (second Amendment), headless tests in `crates/vxe-render/tests/mirror.rs` on
    lavapipe:
    - **One page short.** With attribute pages, a pool filled to one page short of a selection
      of promotions at increasing distance: the sync returns `Ok`, the nearest chunks upload
      and read back to their voxels, the farthest are deferred (`deferred_chunks` equals their
      count, `pending_chunks` includes them), and a following sync after a demotion frees a
      page uploads them.
    - **A dug chunk goes first.** A chunk at the focus, edited to gain `Full` bricks, is
      selected with farther promotions on a pool that cannot take them all: the dug chunk
      uploads and reads back to its edited voxels; only chunks farther than every uploaded
      non-demotion are deferred.
    - **A demotion is not blocked.** The nearest selected chunk does not fit, and a farther
      demotion that frees an attribute page does fit: the demotion uploads and reads back to
      its mips, and the near chunk is deferred and uploads in the next sync.
    - **Nothing half-written.** After a partial sync, every deferred chunk's record entries,
      palette and table entry read back as before the sync, a deferred promotion still reads
      back to its mips, and its dirty bits are set. `live_slots()` equals the slots of the
      uploaded chunks plus those before the sync, less their frees. A chunk that turns wide in
      the sync, on a pool that can take it narrow but not wide, is deferred whole, and its
      narrow record still reads back.
    - **Empty sync.** A selection whose nearest chunk alone does not fit, with no demotion
      behind it, fails with `PoolTooLarge` and leaves `bytes_written`, `mirrored_chunks` and
      `live_slots` unchanged. The existing "fails and leaves the mirror untouched" tests are
      rebuilt so the nearest chunk itself does not fit, or else assert the partial upload.
    - The fly-through's streak bookkeeping test: a deferral extends a streak, a sync with
      none ends it, an empty sync counts, and a streak open at the end counts toward the
      longest.

### Phase 4: Readings and the budget line
- **Owner skill:** dev
- **What:** the readings `nfr.md` §3 and §7 need, and the roadmap note.
- **Files touched:** `docs/nfr.md`, `docs/roadmap.md`.
- **Done when:**
  - `--autopilot --headless` at r_full 16 / r_lod 24 and at 14 / 22, each with and without
    `--look contour`, dev box, two runs each: peak live, budget, the split (pages and
    records), peak pressure with its slot and record terms, radius changes, the reservation
    time, first-frame time, and no OOM; recorded in §3 with machine and commit. Timing runs
    hold the perf lock and state the 1-minute load: Phase 1 read 414 to 513 ms first frames
    (one 12 % over Plan 0002's 457 ms) under build load 9.6 to 11.
  - Phase 3's `--mirror-budget 1930` run, twice: radius changes, the final `r_full`, peak
    pressure, failed syncs, recorded in §3 as the pressure rule's reading.
  - Amended 2026-10-20 (second Amendment), in the review fix pass, after the partial sync:
    `--autopilot --headless --look contour` at the default 3072 MiB, at 16/24 and at
    `--residency 14,22`, two runs each, exclusive perf lock, 1-minute load stated. Each run
    reads a longest deferral streak of at most **90 syncs** (N above), a streak open at the end included. Each run's return
    hash matches, with no out-of-memory stop. The log records for each run the deferred-chunk
    total, the syncs with a deferral and the empty syncs, the pressure and term ranges at
    deferral, the longest streak, and the frames from each decrease to the end of its
    streak. The face gate at `--mirror-budget 1930`, once, reads 0 deferred chunks, at least
    one decrease and none below 6, peak pressure under 1.0, peak live / budget under 0.95, and
    a matching hash. These replace §3's contour rows and the paragraph under them, with
    machine and commit. A streak over 90, or a deferral at or under 0.9 that does not clear
    within 90, is a stall: stop and hand back to the architect with those readings. Do not
    widen N.
  - §7 records the reservation time as a reading in place of Phase 1's under-50 ms guess.
  - §3's line reads "live bytes against the reserved budget", with the readings replacing
    Plan 0002's allocated figures.

## Data shapes

```rust
// illustrative
pub struct MirrorConfig {
    // ... as today
    /// VRAM reserved for pool shards, masks and records, split by the adapter's limits.
    pub budget_bytes: u64,
    /// Records to reserve; `None` reserves the window's W^3 (amended 2026-10-20).
    pub record_chunks: Option<u64>,
}
pub struct SyncStats {
    // ... as today
    pub buffers_created: u32, // 0 after `new`
    pub copies: u32,          // 0 after `new`
    /// Chunks selected this sync but deferred for space; they stay pending (amended
    /// 2026-10-20, second Amendment). 0 when everything fit.
    pub deferred_chunks: u64,
}
// sync: Ok(stats) when it uploaded at least one chunk, whatever it deferred;
// Err(PoolTooLarge | RecordsTooLarge) only when it deferred chunks and uploaded none,
// having written nothing.
impl GpuWorld {
    pub fn pressure(&self) -> f32; // max(slot occupancy, record occupancy)
    pub fn pages_issued(&self) -> u32;
}
```

The split: `records = min(record_chunks (else window capacity) x RECORD_BYTES, binding limit)`;
`pool = budget - records`, in whole pages, filling shard 0 to its limit, then 1, and so on; masks are
`MASK_BYTES x pool slots`.

The fly-through's deferral record (second Amendment) replaces the failed-sync counter with:
deferred-chunk total, syncs with a deferral, empty syncs, the pressure and term ranges at
deferral, and the current and longest deferral streak in syncs.

## Budgets

- **§3 world VRAM:** allocated equals the budget (3 GB by default); live is the reading.
  Projection for radius 14 with contour: 2.56 GB live at rest (D1's table), plus the
  hysteresis transient, under 3 GB; the Phase 4 reading decides. From the 2026-10-20
  amendment, records take 244 659 136 B of the budget at r_lod 24 (93 239, the sphere) rather
  than 348 076 224 B (the cube), and pressure is the larger of slot and record occupancy; the
  pressure rule's own reading is the Phase 3 run at 1930 MiB.
- **§7 first frame:** one reservation of 3 GB at startup. Phase 1 read 69.6 to 86.0 ms on the
  dev box under build load against a guess of under 50 ms; a throwaway probe put
  `create_buffer` near 18 ms per GiB, so the time follows the budget, not the split. The
  reading replaces the guess; Phase 4 re-reads it with the first frame.
- **§2 streaming line:** the grow copies (up to about 2 GB each on the GPU) no longer occur;
  the autopilot's p99 frame is re-read in Phase 4 beside Plan 0002's 51.8 to 60.7 ms.

## Risks & open questions

- **A 6 GB card with a 3 GB budget** plus the frame targets and attribute pages: the
  reservation makes the failure explicit at startup; the baseline card is not on hand, and
  the portable profile sets a smaller budget.
- **Re-kinding and the attribute pass.** The pass addresses attributes by slot inside the
  page; a page is only re-kinded when empty, so no live brick moves.
- **Streamer radii at runtime.** If `Streamer` cannot change radii without a rebuild, Phase
  3 adds the setter in `vxe-world`; the residency tests cover it.
- **The 1930 MiB gate rests on a measurement.** The bound on the start in motion (17, 25)
  does not fit 743 pages; the gate holds because the measured flight peak does and the rule
  acts within its first 60 frames. If a run fails it, the Phase 3 log records the peak and
  the frame of each change, and the window (724 to 759 pages) is re-derived, not widened.
- **The record term is inert by sizing.** It stays under 0.7 only while the streamer keeps its
  radius around one focus; an application with several foci passes its own `record_chunks`.
- **A deferral can outlive the rule (second Amendment).** The bound N = 90 rests on two
  things. Deferrals need pressure above 0.9, every reading so far. And the rule's decreases
  clear them, by staling the edge promotions and landing demotions. A fragmented pool that
  runs out at or under 0.9 would defer with the rule never armed. The fix pass's
  fragmentation was about 9 % of slots, against a 0.1 gap. If the readings show it, the
  remedy the owner set aside, a page-occupancy term in `pressure()`, is the next plan's
  question, not a widening of N.
- **A dug chunk can wait behind a full pool.** Nearest first puts it ahead of every
  non-demotion, but if it alone does not fit, it waits for the rule like any other. On screen
  the edit shows late; the world's voxels and the op result are unaffected (render-side
  only).
- **The walk past a deferral.** Deferred chunks do not count against `sync_budget_bytes`, so
  a sync at a full pool walks every pending chunk to find the demotions. The pending set is
  bounded by the residency, and Phase 4's p99 frame reading covers the cost; if it shows,
  bound the walk by count, not by skipping demotions.
- **Determinism:** none; the mirror is render-side. `chunks_in_ball` is integer arithmetic.

## What this plan does NOT do

- Size-classed brick slots (ADR-0012's mirror-slot plan): the reservation is in 512-byte
  slots today and in 64-byte units then.
- Compaction or shrinking.
- A VRAM query from the adapter: the budget stays configuration.

## Implementation log

> Written by `dev`: one row per phase as its commit lands, then the close block. **The phases
> above are the contract. This section records what happened.** Observations, never
> conclusions: no pass list, no self-assessment. Deviations and unmet done-whens are
> **always** disclosed. Keep it shorter than `## Implementation phases`.

| phase | owner | state | commit |
|---|---|---|---|
| 1: One reservation, no second buffer | dev | done | 3b9ea25 |
| 2: Empty pages change kind | dev | done | cd90f89 |
| 3: Pressure, and the streamer follows it | dev | done (stopped at fc98db0 on the 1024 MiB done-when; amended at 8f41540) | 06d1f17 |
| 4: Readings and the budget line | dev | done | e5ce0b5 |

### Notes

_(Deviations from the plan, with the commit, stated without justification. Done-whens not
satisfiable as stated, and what was done instead. Followups noticed and not acted on. One line
each. Empty is fine.)_

- Phase 1: the budget covers the pool shards (placeholders included), masks and records; the chunk table and material rows are outside it, and `allocated_bytes()` now sums only those three kinds of buffer (it counted the table and materials before). `mirror_bytes()` counts the material rows written instead of the materials buffer's size. (Phase 1 commit)
- Phase 1: the materials buffer is still created at the first sync, sized to the table (`look.rs` asserts its size equals the rows); `SyncStats::buffers_created` counts pool, mask and record buffers only. Both new `SyncStats` fields are zero by construction (no path creates or copies after `new`); the test also asserts the six reserved buffers are the same handles after the run. (Phase 1 commit)
- Phase 1: `pool_slots()` is now the pages handed out (the high-water mark) rather than the buffer capacity, and `shards_in_use()` follows it; `reserved_slots()`, `reserved_records()`, `ShardLayout::{reserved_slots, reserved_records, with_budget, reserved_bytes}`, `budget_for`, `PAGE_BYTES`, `PLACEHOLDER_BYTES` and `DEFAULT_BUDGET_BYTES` are new. (Phase 1 commit)
- Phase 1: `MirrorConfig::new` defaults the budget to 3 GiB clamped to the device, so tests outside Files touched (`golden.rs`, `look.rs`, `walk.rs`, `translucent.rs`, `hud_golden.rs`, `hero.rs`) reserve lavapipe's four 128 MiB shards; the mirror tests in Files touched pass 64 MiB, the level tests 256 MiB. (Phase 1 commit)
- Phase 1: the fly-through's level default is `level_mirror_budget` (its `Full` bricks plus the 64 Ki dig headroom, in attribute pages under `--look`, and a record per extent chunk); the streamed default is 3 GiB with records for the window's `W^3`. `POOL_HEADROOM_BRICKS` moved from `main.rs` to `lib.rs`. (Phase 1 commit)
- Phase 1 added an `update` call at the top of `init_stream` described as overlapping generation with the reservation; `main` already starts the workers before the device opens, so the call was a no-op and the first-frame difference across those runs was load. Removed in the Phase 3 commit.
- Phase 1 done-when not met as stated: the 3072 MiB reservation took 69.6 to 86.0 ms against the 50 ms guess; the streamed first frame read 513, 447 and 414 ms against Plan 0002's 457 ms, one run 12 % over (see Close triggers). (Phase 1 commit)
- Phase 2: the churn bound counts "page slots" per kind as the brick slots a page of that kind hands out (1364 attribute, 4096 plain). The churn test drives the tiers by moving chunks between the store and `insert_lod` in a following window, not through a `Streamer`. It read 2 pages issued for peaks of 584 plain and 1343 attribute slots (bound 4); the old code was not run against it. (Phase 2 commit)
- Phase 2: `pages_issued()`, `free_pages()` and `spare_slots()` are new; `a_following_window_mirrors_the_streamed_residency` and `lod_records_decode_to_the_mips_and_demotion_frees_the_full_slots` now accept an evicted slot in a free page, and assert `live_slots() + spare_slots() == pool_slots()`. `an_empty_attribute_page_serves_plain_slots` is a second test beside the churn one: page 0 goes attribute, free, plain, attribute with `pages_issued()` at 4. (Phase 2 commit)
- Phase 2: a page re-kinded from plain to attribute keeps the stale masks of its former plain bricks in its attribute slots; nothing reads a mask there, and the fresh-mirror test that asserts those masks are zero still holds. (Phase 2 commit)
- Phase 3: `--residency R_FULL,R_LOD` added to the fly-through (Phase 4's runs at 14/22 need it; the `plan-0013` branch adds a flag of the same name and form, so the merge will meet it twice). (fc98db0)
- Phase 3: a streamed sync that fails past the reservation no longer ends the fly-through: it prints the first failure, counts them for the autopilot report, and leaves the chunks queued. The one counter (`syncs past the reservation`) covers `PoolTooLarge` and `RecordsTooLarge` together. (fc98db0)
- Phase 3: the HUD's pressure row appears only with a reading (`HudStats::pressure`), so `HudStats::fixed()` and the HUD golden are unchanged. (fc98db0)
- Phase 3 (amended): `apps/flythrough/tests/level_mirror.rs`, outside the amended Files touched, gained `record_chunks: None` in its one `MirrorConfig` literal so it compiles. `flythrough::PRESSURE_LOW` and `stream_record_chunks` live in `lib.rs` (the count is `ceil(ball x 10 / 7)` in integers). `settle` in `tests/stream.rs` replaces `wait_idle` in the set_radii test so every poll's resident count is checked. (Phase 3 commit)
- Between Phases 3 and 4, a commit outside any phase row: the autopilot's report line adds the reserved slots and records and the peak of each pressure term, which Phase 4's done-when reads; `main.rs` only. (the commit after 06d1f17)
- Phase 4: with `--look contour` at the default 3072 MiB, every run failed some syncs past the reservation (548, 113 at 16/24; 22, 89 at 14/22) while slot occupancy read 0.911 to 0.946: the pool ran out of pages below 1.0. Those syncs wrote nothing, their chunks stayed queued, the rule's next decrease cleared them, and every flight finished with a matching hash. `pool_need` counts neither pages that empty during the same sync nor the free single slots of the other kind, so with two kinds it reports need before the slots are all used. Not acted on. (Phase 4 commit)
- Phase 4: no first-frame reading on a quiet box: the 1-minute load stayed between 10.6 and 23.2 through every run because other lanes were building. (Phase 4 commit)
- Review fix, finding 1 (major, `pool_need` credits nothing for tier changes): `pool_need` now replays the selection in order, crediting each tier change's freed slots to its kind and each page those frees empty as a free page less its spare slots, before that chunk's own allocations; `select` records each chunk's new slots per kind. Tests `a_demotion_that_empties_attribute_pages_fits_a_full_pool`, `a_plain_demotion_fits_a_full_pool` and `a_selection_past_the_reservation_net_of_its_frees_still_fails_untouched`; run once against the old `pool_need` (gpu_world.rs stashed): the first two failed with `PoolTooLarge { needed: 6291456, limit: 4194304 }` and `{ needed: 2134528, limit: 2097152 }`, the third still failed but with `needed: 2621440` where the new count reads 2359296. (fix commit for finding 1)
- Review fix, finding 2 (major, contour's failed syncs): the fly-through records the pressure and its two terms at each failed sync, prints at the first one each pool reading it has (plain free slots, spare slots, free pages, pages issued, `r_full`), and the autopilot report prints the count, the pressure and term ranges at failure and the longest run of consecutive failed syncs. At bf9ac5f two of four contour runs still failed syncs (9 and 20, all at pressure 0.906 to 0.908), which led to the second Amendment. (bf9ac5f)
- Review fix, finding 1 follow-up: the two tier-change tests' comments state the old check's failure as a property of a check without the credit (comment-hygiene gate). (fix commit for finding 2)
- Review fix, second Amendment (partial sync): `select` replays the fit as it walks (`Fit`, which absorbs `pool_need`) and defers the first chunk that does not fit and every later non-demotion; `upload` re-checks a chunk about to turn wide against the room for slot pairs before dropping or writing anything of it; `SyncStats::deferred_chunks`; an empty sync returns the first deferral's error. Tests: `one_page_short_uploads_the_nearest_and_defers_the_rest_untouched` (also the nothing-half-written checks), `a_dug_chunk_uploads_ahead_of_farther_promotions`, `a_demotion_behind_a_deferred_chunk_still_uploads`, `a_chunk_turning_wide_past_the_pool_is_deferred_whole`; the four "fails and leaves the mirror untouched" tests now assert the partial upload, then an empty sync that writes nothing (`partial_then_empty`). The fly-through's `Deferrals` replaces the failed-sync counter; streak ends and radius changes print their streamer tick. An empty sync's deferred count is the pending chunks, since it returns no stats. (fix commit for the partial sync)
- Review fix, Phase 4 re-read after the partial sync (52cf84d, contour twice at 16/24 and 14/22, the 1930 MiB face gate once; in `nfr.md` §3 and Close triggers): longest deferral streak 1, 3, 2 and 6 syncs against the bar of 90; no empty sync; the face gate read 0 deferrals. Deferrals at pressure under the 0.9 trigger did occur at 16/24 (0.814 to 0.855, 8 and 80 syncs); each streak cleared within 3 syncs with no radius change, so none is a stall by the Amendment's definition. (Phase 4 refresh commit)
- Round-2 review, finding 1 (major, `Fit` credited a page as emptied after the same sync allocated into it): once a kind has taken spare slots, `Fit` credits that kind's later frees as slots, never as an emptied page; `upload` checks each chunk alone against the allocator as it is (`Fit::new` then `place`, the doubled need when it turns wide, plus `pair_room` for the pairs' alignment) before dropping or writing any of it, and defers it whole with every later non-demotion. Tests `a_page_the_sync_allocated_into_is_not_credited_free` (the reviewer's probe) and `a_widening_chunk_defers_the_later_chunk_it_crowds_out`; against 52cf84d's `gpu_world.rs` both failed, with `PoolTooLarge { needed: 6291456, limit: 4194304 }` and `PoolTooLarge { needed: 4194304, limit: 2097152 }`. (fix commit for round-2 finding 1)
- Round-2 review, minor 1 (a widening chunk spending room counted on for later chunks): covered by the same backstop and its second test. Minor 2: `a_chunk_turning_wide_past_the_pool_is_deferred_whole` asserts the chunk keeps its dirty bits. (fix commit for round-2 finding 1)
- Round-2 review, readings re-taken at b474737 and the nfr nit: `nfr.md` §3's contour rows (every column, full residency included), the paragraph under them and the gate's run 3 carry the new figures; the text states the face rows are ef37395 readings in every column. Longest deferral streaks 1, 0, 0 and 3 syncs; the face gate read 0 deferrals. (round-2 readings commit)
- Round-3 review, finding 1 (major, the backstop counted free slots the allocator could not hand out for the need): `alloc` takes a free single, then the cursor, then splits a free pair, before it takes a page, so every slot `available()` counts serves a narrow brick; the backstop checks a wide need (a chunk turning wide, or an already-wide record's new bricks) against pair room only, and `pair_room` skips the odd cursor slot. Tests `free_pairs_serve_narrow_bricks` (the review's first probe) and `an_already_wide_chunk_is_not_counted_free_singles` (the second); against 6d7d5ab's `gpu_world.rs` both failed with `PoolTooLarge { needed: 4194304, limit: 2097152 }`. With pair splitting the first probe fits whole (3 chunks, 0 deferred) where the review expected x = 7 deferred; the test asserts that. A narrow-only world never has free pairs, so its admit decisions are unchanged and the Phase 4 readings were not re-taken. (fix commit for round-3 finding 1)
- Round-3 review, minor 1 (an empty sync after the upload's check deferred everything still wrote the material rows and queued table changes): moved, not documented as an exception. The material flags are set before the upload; the rows, the staged runs and the table changes are written only when the sync goes on to return `Ok` or the upload's own error; an empty sync leaves them for the next sync that writes. `a_chunk_turning_wide_past_the_pool_is_deferred_whole` asserts `bytes_written()` is unchanged by its empty sync. (fix commit for round-3 minor 1)
- The resume notes both Amendments cite were deleted when their fixes landed; they read at fc98db0 (first) and bf9ac5f (second).
- Phase 3 (amended): the first 1930 MiB run also raised `r_full` once, 14 to 15 at pressure 0.676, 15 s into the flight; it ended at 15. (Phase 3 commit)

### Close triggers

_(Facts for the architect. No recommendations, and no suggested version bump.)_

- **What shipped:** the mirror reserves its pool shards, masks and records once from `MirrorConfig::budget_bytes` and never creates or copies another (3b9ea25); an emptied page joins a free-page set either kind takes (cd90f89); `GpuWorld::pressure` (the larger of slot and record occupancy), `MirrorConfig::record_chunks`, `vxe_world::chunks_in_ball`, `Streamer::set_radii`, and the fly-through's radius rule with its HUD row, `--mirror-budget` and `--residency` (fc98db0, 06d1f17, ef37395); `nfr.md` §3 and §7 and the roadmap note (e5ce0b5).
- **Public API changed:** `vxe-render`: `MirrorConfig::{budget_bytes, record_chunks}` (a struct literal must name them), `SyncStats::{buffers_created, copies}`, `ShardLayout::{reserved_slots, reserved_records, with_budget, reserved_bytes}`, `budget_for`, `PAGE_BYTES`, `PLACEHOLDER_BYTES`, `DEFAULT_BUDGET_BYTES`, `GpuWorld::{reserved_slots, reserved_records, pages_issued, free_pages, spare_slots, pressure}`; `allocated_bytes()` now sums only the reserved buffers, `pool_slots()` is the pages handed out, `shards_in_use()` follows it, `mirror_bytes()` counts material rows written; `MirrorError` messages say "the reservation holds". `vxe-world`: `Streamer::set_radii`, `chunks_in_ball`. `flythrough` lib: `STREAM_MIRROR_BUDGET`, `POOL_HEADROOM_BRICKS` (moved from `main.rs`), `level_pool_slots`, `level_mirror_budget`, `PRESSURE_LOW`, `stream_record_chunks`; `hud::{Pressure, PRESSURE_HIGH}`, `HudStats::pressure`.
- **Budget readings:**
  - Phase 1, `flythrough --headless --frames 30 --mirror-budget 3072` (streamed, seed 1), NVIDIA GeForce RTX 3080 Laptop GPU, Ryzen 9 5900HS, release, exclusive perf lock, 1-minute load 9.60 to 10.97 (other lanes building): `mirror: reserved 3220946112 B of a 3221225472 B budget (4919296 slots over 2 shards, 132651 records)` in 82.3, 79.3 and 69.6 ms; device ready 380, 314, 300 ms; first frame 513, 447, 414 ms. Base d11d0cb alternated with it: first frame 1970 (cold, device 1910 ms), 375, 367 ms. Two earlier runs, before the no-op `init_stream` update (load 15.96 to 16.60): reserved in 86.0 and 80.2 ms, first frame 570 and 526 ms against base 3521 (cold) and 446 ms. A throwaway probe on the same GPU (not committed): `create_buffer` of 256 MiB, 1 GiB, 2 GiB took 4.2, 18.2, 36.3 ms; `GpuWorld::with_config` at 3 GiB 51.2 to 53.6 ms.
  - Phase 3, `flythrough --autopilot --headless --mirror-budget 3072` (r_full 16, r_lod 24, h 1, 15 workers), RTX 3080 Laptop GPU, exclusive perf lock, 1-minute load 19.37 to 25.08: reserved in 85.4 ms; full residency after 10 263 ms at 1 786 641 644 B live, pressure 0.555; flight peak 1 881 891 684 B live, peak live / budget 0.5843, 0 radius changes, 0 syncs past the reservation; return hash match; march GPU p99 13.479 ms at 1280x720. With `--mirror-budget 1024` (load 14.26 to 6.10): reserved 1 072 873 152 B (1 241 088 slots, 132 651 records) in 20.9 ms, then the failed sync above and no full residency in 600 s.
  - Phase 3 (amended, 06d1f17), `flythrough --autopilot --headless --mirror-budget 1930`, r_full 16, r_lod 24, h 1, seed 1, 15 workers, RTX 3080 Laptop GPU, exclusive perf lock, 1-minute load 24.13 to 23.03: `reserved 2021964224 B of a 2023751680 B budget (3043328 slots over 1 shards, 93239 records)` in 63.0 ms; full residency at 13 093 ms, 1 786 641 644 B live, pressure 0.920; `r_full 16 -> 15` at 0.920 (13 172 ms), `15 -> 14` at 0.920 (13 386 ms), `14 -> 15` at 0.676 (28 075 ms); 0 syncs past the reservation; peak pressure 0.9197, peak live / budget 0.8836; return hash match. At `--mirror-budget 3072` (load 21.59 to 20.16): 5 095 424 slots and 93 239 records reserved in 94.9 ms, pressure 0.620 at full residency, peak pressure 0.6363, peak live / budget 0.5780, 0 radius changes, 0 failed syncs.
  - Round-2 fix (b474737), same machine, flags and lock:
    - contour 16/24 run 1 (load 13.64 to 15.90): 1 sync with a deferral, 0 empty, 1605 chunks, at pressure 0.7961 (slots 0.7961, records 0.2421), at tick 1289, cleared in 1 sync with no radius change; decreases at ticks 469, 499, 664, 905 (16 to 12); full residency 2 864 309 692 B live at r_full 13, pressure 0.911; peak pressure 0.9114, peak live / budget 0.8894; reserve 100.6 ms, first frame 2330 ms (cold device).
    - contour 16/24 run 2 (load 21.11 to 26.04): no deferral; the same four decreases; full residency 2 864 309 692 B; peak pressure 0.9114, peak live / budget 0.8894; reserve 96.2 ms, first frame 857 ms.
    - contour 14/22 run 1 (load 15.90 to 21.11): no deferral, no radius change; full residency 2 607 896 708 B, pressure 0.822; peak pressure 0.9020, peak live / budget 0.8747; reserve 105.4 ms, first frame 673 ms.
    - contour 14/22 run 2 (load 26.04 to 18.08): 15 syncs with a deferral, 0 empty, 19 801 chunks, pressure at deferral 0.8554 to 0.9441 (records 0.2916 to 0.5307), longest streak 3; the decrease 14 to 13 at tick 796 came before the first deferral (tick 807); peak pressure 0.9442, peak live / budget 0.9198; reserve 197.3 ms, first frame 1030 ms.
    - face 1930 MiB (load 19.82 to 14.92): 0 deferrals; 16 to 15 to 14 at 0.920 (ticks 854, 884), back to 15 at 0.604; peak pressure 0.9197, peak live / budget 0.8836; reserve 52.0 ms, first frame 2061 ms.
    - Every return hash matched; no out-of-memory stop.
  - Review fix pass (52cf84d, partial sync), `flythrough --autopilot --headless`, seed 1, 15 workers, RTX 3080 Laptop GPU, Ryzen 9 5900HS, exclusive perf lock, default 3072 MiB:
    - contour 16/24 run 1 (load 15.31 to 17.54): 8 syncs with a deferral, 0 empty, 12 654 deferred chunks, pressure at deferral 0.8227 to 0.8420 (slots the same, records 0.3132 to 0.3550), longest streak 1; decreases at ticks 469, 499, 664, 905 (16 to 12), all before the first deferral at tick 1096, so no decrease ended a streak; peak pressure 0.9114, peak live / budget 0.8894; reserve 85.2 ms, first frame 2280 ms (cold device).
    - contour 16/24 run 2 (load 17.05 to 14.94): 80 syncs with a deferral, 0 empty, 154 751 deferred chunks, pressure at deferral 0.8141 to 0.8551 (records 0.1804 to 0.2525), longest streak 3, deferrals between ticks 2334 and 2677, after the restore 12 to 13 at tick 2045 and with no decrease after them; ends at r_full 13; peak pressure 0.9114, peak live / budget 0.8894; reserve 101.5 ms, first frame 562 ms.
    - contour 14/22 run 1 (load 17.54 to 17.05): 7 syncs with a deferral, 0 empty, 12 332 chunks, pressure at deferral 0.9061 to 0.9236 (records 0.4304 to 0.4713), longest streak 2; the decrease 14 to 13 at tick 773 came in the frame its streak ended (0 frames); peak pressure 0.9293, peak live / budget 0.9035; reserve 84.8 ms, first frame 614 ms.
    - contour 14/22 run 2 (load 22.79 to 22.82): 22 syncs with a deferral, 0 empty, 13 902 chunks, pressure at deferral 0.8514 to 0.9273 (records 0.2686 to 0.4885), longest streak 6; the decrease 14 to 13 at tick 917, its streak ended at tick 918 (1 frame); peak pressure 0.9277, peak live / budget 0.9018; reserve 79.4 ms, first frame 642 ms.
    - face 1930 MiB (load 22.82 to 14.34): 0 deferrals; 16 to 15 to 14 at 0.920 (ticks 854, 884), back to 15 at 0.610 and 16 at 0.661; peak pressure 0.9197, peak live / budget 0.8836; reserve 50.0 ms, first frame 613 ms.
    - Every return hash matched; no out-of-memory stop. No quiet-box first-frame reading: load never fell under 3.
  - Phase 4 (ef37395): the autopilot matrix at 16/24 and 14/22, face and contour, two runs each, and the second 1930 MiB run, are in `nfr.md` §3 ("The reserved budget" and "The pressure rule's gate") with machine, load and commit; §7 holds the reservation and first-frame times. No run stopped on out-of-memory.
  - Phase 1, `flythrough --level --headless --frames 3`, same machine: `reserved 271524864 B of a 271526464 B budget (450560 slots over 2 shards, 3200 records) in 4.2 ms`, 232 436 992 B live; with `--look contour` `reserved 797779456 B of a 797780544 B budget (1351680 slots over 1 shards, 3200 records) in 49.9 ms`, 680 491 136 B live.
- **Gate after the round-3 fixes (this tip):** `cargo nextest run --workspace --test-threads 4` exit 0, 410 run, 410 passed, 4 skipped (perf lock, load 6.79; run before the one added assertion in `a_chunk_turning_wide_past_the_pool_is_deferred_whole`, which the mirror suite re-ran); `cargo test --doc --workspace` 7 passed; clippy (`-D warnings`), `cargo fmt --all --check` and `cargo deny check` exit 0; doc-links, comment hygiene and index rows pass. Phase 4 readings not re-taken: narrow-only worlds never hold free pairs, so their admit decisions are unchanged.
- **Gate after the round-2 fix (b474737):** `cargo nextest run --workspace --test-threads 4` exit 0, 408 run, 408 passed, 4 skipped (perf lock, load 8.88); `cargo test --doc --workspace` 7 passed; clippy (`-D warnings`), `cargo fmt --all --check` and `cargo deny check` exit 0; doc-links, comment hygiene and index rows pass.
- **Gate after the second fix pass (52cf84d):** `cargo nextest run --workspace --test-threads 4` exit 0, 406 run, 406 passed, 4 skipped (perf lock, load 15.78); `cargo test --doc --workspace` 7 passed; `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all --check` and `cargo deny check` exit 0; doc-links, comment hygiene and index rows pass.
- **Gate after the first fix pass (bf9ac5f; the run preceded only a comment edit in `mirror.rs`):** `cargo build` exit 0; `cargo nextest run --workspace --test-threads 4` exit 0, 402 run, 402 passed, 4 skipped (perf lock, load 15.36); `cargo test --doc --workspace` 7 passed; clippy, fmt and `cargo deny check` exit 0; doc-links and comment hygiene pass. Failed syncs remained with contour (the resume notes at bf9ac5f).
- **Gate at the first close (e5ce0b5):** `cargo build` exit 0; `cargo nextest run --workspace --test-threads 4` exit 0, 398 run, 398 passed, 4 skipped (perf lock, load 19.76); `cargo test --doc --workspace` exit 0, 7 passed; `cargo clippy --workspace --all-targets -- -D warnings` exit 0; `cargo fmt --all --check` exit 0; `cargo deny check` exit 0 (advisories, bans, licenses, sources ok); `check-doc-links` 128 links resolve; comment hygiene OK; index rows OK. The worktree needed `scripts/build-guests.sh` before the wasm tests pass.
- **Outstanding `human` phases:** none.

## Followups

- **Page fragmentation under attribute pages.** With `--look contour` the pool can defer chunks
  under the 0.9 trigger (0.796 in a 16/24 run, `nfr.md` §3): attribute and plain slots sit spare in
  partly filled pages of the other kind, and an attribute page holds 1364 bricks in 4096
  slots. The streaks clear within 3 syncs (`nfr.md` §3), so it is a finding, not a miss. A
  future mirror plan weighs compaction (moving a page's last live bricks so it frees), a
  pressure term that counts each kind's spare in partly filled pages, or a lower trigger with
  attribute pages, and measures deferrals per minute against the same autopilot runs.
- **A tier change that turns wide is checked against pair room before its own frees.** In
  `upload`, `pair_room` is read before the old record is dropped, so a demotion or promotion
  whose new record is wide is not credited the pages its old record empties. It can only be
  deferred when it would have fitted, never admitted when it does not; it needs a table of
  more than 256 materials. Fold it into the fragmentation plan if that plan touches the check.
- **`GpuWorld::pressure`'s doc still says a sync past the reservation "fails instead of
  growing it".** Since the second Amendment a sync writes what fits and defers the rest; the
  sentence wants that wording at the next touch of `gpu_world.rs`.
