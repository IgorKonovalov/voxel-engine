# 0060: The live queue by the view, nearest first, and withdrawing the work a flight has passed

> **Status:** draft
> **Created:** 2026-11-10
> **Milestone:** M2 in [`docs/roadmap.md`](../roadmap.md) (the emissive light behind "dark scenes, emissive voxels
> as the light")
> **Related ADRs:**
> [ADR-0048](../adrs/0048-the-live-queue-orders-by-the-view-and-withdraws-the-work-a-flight-has-passed.md)
> (proposed; this plan builds it, its close accepts it), which amends the queue clause of
> [ADR-0046](../adrs/0046-static-emissive-light-is-baked-per-chunk-when-a-chunk-loads-or-changes.md)'s Outcome.
> **Source:** the owner's window of 2026-11-09, Plan 0059 Phase 13 (`docs/plans/logs/0059.md`), parked in
> [`docs/backlog.md`](../backlog.md) and taken here. Drafted without an interview (orchestrator's instruction);
> the guesses are the open questions below.
> **Starts after:** Plan 0059's close (done, v0.44.0). Independent of the antialiasing plan and Plan 0058; it
> shares `crates/vxe-render/src/bake.rs` and `apps/flythrough/src/main.rs` with Plan 0045's split, so whichever
> lands second rebases its paths.

## TL;DR

The baked light's live queue (`bake::BakeQueue`) opens chunks by distance first, so far light in the view waits
for every nearer chunk, behind the camera included, and a fast flight keeps baking chunks it has passed. This
plan reads where the queue's samples go in a flight first, then sorts the candidates by tier (the near shell,
then the view, then the rest), and then withdraws the jobs of chunks the camera has passed. The first thing the
owner sees is a HUD row that counts the chunks in view still waiting; the last is a window where the far cave
refines while he looks at it and a fast flight's light lands ahead of him.

## Context & problem

ADR-0046's Outcome built one queue on the render thread, nearest by chunk distance, ties to the chunk ahead.
In `BakeQueue::open` (`crates/vxe-render/src/bake.rs`) the sort key is `(class, d2, -cos, coord)`: class 0 (a
full chunk with no block) before class 1 (a coarse block to refine), then squared chunk distance, the view only
as a tie. The owner, Plan 0059 Phase 13:

- the patch, at rest: "in long distance, light is not recalculated until I come closer, it would be better to
  recalculate all in the field of view, starting from the nearest chunks";
- the streamed cave, fast flight: "it takes quite a bit of time to settle light even as coarse. I think this is
  becouse we are starting to calculate places that camera is already passed. We should be able to cancel it when
  camera passed".

What the tree says about each (read, not measured):

- **At rest.** After the pre-roll every chunk of the level is coarse or full; refinements run nearest first in
  every direction. Plan 0059 Phase 8c read the vein's level all at 4096 after 157 s. A chunk in view at distance
  6 waits for every chunk at 5 or less.
- **In flight.** On the streamed cave the mirror holds about 17 000 full chunks at `r_full` 16 (Phase 8c's
  pre-roll read 17 077), so class 0 never empties while the camera moves and the coarse pass takes the slice. The
  nearest unbaked chunks are about as likely behind the camera as ahead. A job runs to its end in the sink:
  nothing in `JobLoop` takes a job out early except `stale` (an edit), which still lets it run.
- **Throughput is not this plan's lever.** At 1.5 ms a frame and the 36 to 64 ns a sample Plan 0059 read, the
  slice draws roughly 2 to 4 x 10^4 samples a frame (a guess from those figures, not a reading); a refinement at
  4096 is about 1.2 x 10^6 samples (Phase 8a: 3.836 x 10^9 over 3168 chunks), so tens of frames a chunk. The order
  decides which chunks get those frames; Phase 1 reads how far that goes.

What it serves: `project-context.md`'s dark-scene requirement (light where the player looks), and the owner's
standing rule that a stable image comes first. The `nfr.md` figures touched: section 2 (the slice and the queue
step's CPU), section 3 (no new buffer), section 7 (time to first frame, through the pre-roll's order).

## Decision

Build ADR-0048: the queue sorts its candidates by **(tier, class, squared distance, view cosine)**, tier 0 the
near shell (Chebyshev `near` of the camera's chunk, every direction), tier 1 the chunks whose box meets the
frustum widened by a margin, tier 2 the rest, digs ahead of all; and it **withdraws** an open or pending job
that is not a dig or ring 3 once its chunk leaves the shell and a wider keep cone while a tier 0 or 1 candidate
is ready. Both are behind fly-through flags with a live key, so the owner compares them with Plan 0059's order
in one window. We rejected a continuous distance-by-angle score because no weight guarantees "all of the view
first"; a predicted eye (velocity lead) because it adds a second knob and equals the view order at rest; and
demotion without withdrawal unless Phase 1 shows the passed work in flight is small (then it is the fallback).

## Architecture diagram

```mermaid
flowchart LR
    subgraph app[apps/flythrough]
        F[fly camera + View tangents] --> QV[QueueView: eye, basis, tangents]
        K[order / withdraw key] --> QV
    end
    subgraph render[vxe-render bake.rs]
        QV --> T[tier: shell, view, rest]
        T --> S[sort: tier, class, d2, cos]
        S --> O[open jobs within two slices]
        T --> W[withdraw passed jobs]
        W --> L[JobLoop: drop pending, close open, release place]
        O --> L
        L --> St[Store blocks + fades]
    end
    St --> HUD[HUD row: in view waiting, withdrawn]
```

## Implementation phases

Each phase ships as its own commit. `dev` implements all `dev` phases in one session. The readings go to
`docs/plans/logs/0060.md` (from the readings-log template); the Implementation log below keeps one row a phase.
**Phase 1 is a spike:** it reads where the queue's samples go before any order changes, and Phase 3's shape
depends on its reading (the stop rule there).

### Phase 1: The queue's tiers as a reading, and where a flight's samples go
- **Owner skill:** dev
- **What:** the queue classifies every candidate and every job into ADR-0048's tiers without changing its
  order, and counts where its samples go, so the HUD and stdout show what the owner complained about.
- **Files touched:** `crates/vxe-render/src/bake.rs` (`QueueView` gains the camera's basis and four tangents,
  a `tier` function and its unit test, `QueueStats` counters), `crates/vxe-render/src/lib.rs` (exports, if any),
  `apps/flythrough/src/main.rs` (fill the view from the fly camera's `View`, the stdout line),
  `apps/flythrough/src/hud.rs` (the queue row), the live queue's test file
  (`apps/flythrough/tests/queue.rs`, or its `gpu_` name if the backlog's test-tier chore has landed; the rig's
  `QueueView` takes the rig camera's tangents), `docs/plans/logs/0060.md`.
- **The counters (dev may rename):** per tier, the chunks waiting (no block, coarse) and the samples read back
  for jobs whose chunk was in that tier when the job ended; the samples of jobs whose chunk was tier 2 and behind
  the camera (view cosine of its centre under 0) at their end ("passed"); for each chunk that enters tier 1 with
  no full block, the time to its first block and to its full block (p50, p95 over the run).
- **Done when:**
  - A unit test `the_view_tiers_split_chunks_by_shell_and_frustum` (CPU, under 1 s): a camera at the centre of
    chunk [2, 0, 0] looking +x with a 70 degree vertical field at 16:9 and `near` 1 puts the camera's chunk and
    its 26 neighbours in tier 0 whatever their direction; [7, 0, 0] (straight ahead, distance 5) in tier 1;
    [-3, 0, 0] (straight behind) in tier 2; a chunk off to the side whose box stays outside the frustum widened
    by the margin in tier 2, and the same chunk with the margin raised until its box meets the frustum in tier 1.
  - The HUD's queue row shows `in view: N waiting (C coarse)` and the per-second stdout line adds the samples
    by tier, the passed samples and the in-view ages; under the default order every existing queue test still
    passes unchanged (the order is not touched in this phase).
  - Readings (RTX 3080 Laptop by `--adapter NVIDIA`, release, exclusive perf lock, headless 2530 x 1378; one row
    on the RADV RENOIR at 1280 x 720) into the readings log, with the 1-minute load:
    1. the patch at rest, facing down the cave (`--level --pose patch`, 120 s): the time until every tier 1 chunk
       holds a full block, and the tier 1 chunks still coarse at 30, 60 and 120 s;
    2. `--pose vein-flight --glide 30` and `--glide 120`: the share of the slice's samples by tier and passed;
    3. the streamed cave with `--glide 150` and `--glide 300` from the spawn (15 and 30 m/s at 10 cm voxels),
       60 s each: the same shares, the in-view ages, the no-value and coarse shares of the lighting pixels the
       existing line prints, and `waiting reach` among tier 1 chunks;
    4. the autopilot (`--autopilot --headless --seed 1`): the same shares, as the baseline for the close.
  - **Stop rule (dev reports, the architect amends Phase 3 before it starts):** if in run 3 at `--glide 300` the
    passed samples are under 10 % of the slice's samples, the passed work is not the flight's fault and Phase 3
    becomes ADR-0048's Alternative D (demote only: withdraw pending jobs, let open ones finish). If tier 1's
    waiting chunks are mostly `waiting reach` (over half), the flight's fault is the streaming edge, not the
    order, and the plan stops for an owner question before Phase 2.

### Phase 2: The view order
- **Owner skill:** dev
- **What:** `BakeQueue::open` sorts by (tier, class, d2, cos), the pre-roll included, behind `--queue-order
  view|distance` (default `view`; `distance` is Plan 0059's key) and a live key.
- **Files touched:** `crates/vxe-render/src/bake.rs`, `crates/vxe-render/src/marcher.rs` (a setter, if the queue
  is reached through `Frame`), `apps/flythrough/src/main.rs`, `apps/flythrough/src/hud.rs`, the live queue's
  test file, `docs/plans/logs/0060.md`.
- **Done when:**
  - `the_queue_runs_the_view_before_what_is_behind` (lavapipe, gpu tier, about 3 s by its siblings' 2.5 s): the
    six-chunk row of `the_queue_runs_near_first_and_within_its_samples`, camera in chunk 2 looking +x, `near` 0,
    coarse pass on: chunks first appear in `jobs()` in the order [2, 3, 4, 5, 1, 0]; under `distance` the same
    rig reads Plan 0059's [2, 3, 1, 4, 0, 5] with the coarse pass's classes first.
  - `the_queue_runs_near_first_and_within_its_samples` passes unchanged at `near` 2 (chunks 0 to 4 are the near
    shell and keep distance order; chunk 5, beyond it and in view, comes last): the shell is not reordered by
    the view.
  - `a_turn_resorts_the_queue` (lavapipe, about 3 s): the same row at `near` 0, the camera turned to look -x
    after its own chunk opens; of the chunks not yet in `jobs()` at the turn, 1 and 0 open before 3, 4 and 5.
  - `the_queue_settles_to_a_direct_bake` and a copy with the camera turned half-way pass: every block
    bit-identical to a one-shot bake whatever the order (ADR-0046's order independence).
  - The banner names the order (`order view` or `order distance`) and the HUD's queue row shows it, changing on
    the key in the same frame; stdout prints a line at each change.
  - Phase 1's runs 1 to 4 read again under `view` into the readings log, beside Phase 1's rows; the pre-roll's
    first frame at the vein and the hall beside Plan 0059 Phase 8c's 7137 and 1839 ms.

### Phase 3: Withdrawing passed work
- **Owner skill:** dev
- **What:** `JobLoop` gains a withdrawal (a pending job dropped; an open job closed with no write, its table
  slot and pool place released, nothing in flight read into it), and the queue withdraws by ADR-0048's rule,
  behind `--queue-withdraw on|off` (default `on`) and a live key. Its shape follows Phase 1's stop rule.
- **Files touched:** `crates/vxe-render/src/bake.rs`, `apps/flythrough/src/main.rs`, `apps/flythrough/src/hud.rs`,
  the live queue's test file, `docs/plans/logs/0060.md`.
- **Done when:**
  - `a_passed_chunk_is_withdrawn` (lavapipe, gpu tier, about 5 s, a guess): eight chunks in a row, each a 32^3
    rock cube aligned to bricks (96 brick directions, so 393 216 samples a chunk at 4096 a brick direction: 24
    steps at the cap's floor of 16 384), `near` 0, coarse off, slice 1e-6 ms; the camera in chunk 0 looking +x
    until chunks 1 and 2 are in `jobs()`, then moved to chunk 5 looking +x. At the next step `jobs()` holds no
    chunk in 0 to 4, the withdrawn count and samples in `QueueStats` are above 0, `tracks(1)` and `tracks(2)`
    hold nothing but candidacy, and chunks 1 and 2 hold no block and no pool place (every pool word is the
    header's, a held block's or free, the eviction test's check). Run to empty, every block is bit-identical to a
    one-shot bake.
  - `a_dig_is_never_withdrawn` (lavapipe, about 3 s): the same row with a carve in chunk 1 before the move; the
    dig's job stays in `jobs()` after the move and its chunk settles to a one-shot bake of the edited world.
  - `the_edge_of_the_view_does_not_churn` (lavapipe, about 3 s): a camera yawing by plus and minus 5 degrees a
    step about a chunk at the frustum's edge withdraws nothing over 64 steps (the keep cone).
  - With `--queue-withdraw off` the Phase 2 tests read as before; the banner and HUD name `withdraw on/off`, and
    the HUD adds `withdrawn N`.
  - Phase 1's runs 2 to 4 read again under `view` with withdrawal on, beside the Phase 2 rows: the passed samples'
    share, the samples withdrawn, the in-view ages.

### Phase 4: The owner's window
- **Owner skill:** human
- **What:** the owner judges the order and the withdrawal by eye on the release binary, stdout visible.
- **Files touched:** `docs/plans/logs/0060.md` (the owner's words verbatim, by the orchestrator or `dev`).
- **The runs** (a `live/session60.sh` in the lane, untracked, the fly-through's defaults with `--hud`; each step
  starts on `order view, withdraw on` and the owner swaps with the keys):
  1. the patch at rest, looking down the cave: does the far light in view refine first, nearest first;
  2. the streamed cave, flown fast with the pad or keys: does the coarse light land ahead of the flight, and
     what does a turn back show;
  3. the vein: a slow flight and a dig, to confirm nothing regressed;
  4. one run of step 2 at `--bake-slice 3`, the existing flag, for the throughput question;
  5. the Radeon, step 2 at 1280 x 720.
- **Done when:** the owner has picked, for each of the order (view or distance), the withdrawal (on or off) and
  the slice (1.5 or 3 ms), or has named what is still wrong; the words are in the readings log.

### Phase 5: The defaults at the owner's pick, and the close's readings
- **Owner skill:** dev
- **What:** the defaults follow Phase 4's picks; a losing arm's flag stays as a comparison flag unless the owner
  asked for it gone; the budget readings the close needs.
- **Files touched:** `crates/vxe-render/src/bake.rs`, `apps/flythrough/src/main.rs`, the live queue's test file,
  `docs/plans/logs/0060.md`.
- **Done when:**
  - The fly-through's defaults equal the owner's picks, and the banner names them on a plain start.
  - The autopilot read under `docs/nfr.md` section 5's protocol (RTX, 2530 x 1378, FSR 2 at 1.5, `--seed 1`, two
    rounds, exclusive perf lock): the queue step's CPU per frame p50 / p99 / max beside Plan 0059's fix-pass
    rounds (1.029 / 18.424 and 1.019 / 18.198 ms), and `frame_bytes` beside 888 490 396 B.
  - `cargo nextest run --profile gate` at the tip, exit 0, its wall time and the new tests' times in the
    readings log.

## Data shapes

```rust
// illustrative
pub struct QueueView<'a> {
    pub eye: [f32; 3],
    pub forward: [f32; 3],
    pub right: [f32; 3],
    pub up: [f32; 3],
    /// tan of the left, right, up and down half-angles (the View's), widened by the margin in the queue.
    pub tangents: [f32; 4],
    // resident, samples, hold as today
}
pub enum QueueOrder { View, Distance }
// QueueStats gains: per-tier waiting and samples, passed samples, withdrawn jobs and samples.
```

## Budgets

- **Section 2, frame.** The slice stays 1.5 ms (`DEFAULT_BAKE_SLICE_MS`) unless the owner picks 3 in Phase 4,
  which then is a reading of the frame's GPU time at the patch and the hall against Plan 0059 Phase 12's rest
  rows. The queue step's CPU per frame: the tiering adds a box-against-frustum test per candidate (up to about
  17 000 on the streamed cave), so the bar is p50 within 0.1 ms of 1.019 ms and p99 within the two rounds' 17.3
  to 18.4 ms spread (a proposal; Phase 5 reads it).
- **Section 3, memory.** No new buffer; `frame_bytes` unchanged (888 490 396 B), the pool unchanged; a withdrawn
  job's place returns to the pool (asserted in Phase 3).
- **Section 7, startup.** The pre-roll follows the order; the first frame at the vein and the hall is read
  against Plan 0059 Phase 8c's 7137 and 1839 ms (RTX), and may not exceed them by more than 10 % (a proposal).
- **Section 5, streaming.** Not touched: the streamer's load order stays by distance. Residency p95 is not
  re-read.
- The in-view ages and the passed share are this plan's own readings, recorded in the readings log and in
  ADR-0048's Outcome, with no `nfr.md` line: the owner judges them by eye.

## Risks & open questions

- **The streaming edge, not the order.** A tier 1 chunk within the light radius of the mirror's edge waits for
  its reach (`reach_ready`). If the flight's in-view chunks mostly wait there, ordering cannot help; Phase 1's
  stop rule catches it.
- **Churn.** A withdrawn chunk that comes back into view starts over. The keep cone and the "a better candidate
  is ready" condition bound it; Phase 3's edge test and the withdrawn samples read it.
- **A turn shows the back.** Behind the near shell the light is coarse or absent until the queue reaches it.
  The window decides whether `near` 2 is enough.
- **Determinism.** Render-side only; `vxe-world` is not touched. Order and withdrawal must not change a block's
  bits; Phases 2 and 3 assert it against a one-shot bake.
- **Owner questions (drafted with the recommended option first; the orchestrator batches them):**
  1. Tier before class: (a) the view's first passes and refinements before anything out of view (recommended);
     (b) every first pass anywhere before any refinement, the view inside each class (Plan 0059's classes kept);
     (c) one continuous distance-by-angle score.
  2. Passed work: (a) withdraw open and pending jobs outside the shell and the keep cone (recommended);
     (b) withdraw pending jobs only, let open ones finish; (c) no withdrawal, the order alone.
  3. What counts as the view: (a) the frustum widened by 15 degrees, plus the near shell (recommended);
     (b) the exact frustum; (c) the forward half-space.
  4. Throughput: (a) keep the 1.5 ms slice and judge the order first (recommended); (b) the default slice to
     3 ms; (c) a slice that grows while tier 1 chunks wait, as the dig's `edit_ms` does.

## What this plan does NOT do

- No change to the slice, the counts, the coarse pass's samples, the pool or the store's layout (ADR-0046).
- No velocity-predicted order (ADR-0048's Alternative C), parked for a later plan if the window asks.
- No change to the streamer's own load order in `vxe-world`.
- No antialiasing (the hall's "a lot of aliasing" goes to the antialiasing plan).
- No VR union of frusta; the queue takes the one camera's view.

## Implementation log

> Written by `dev`: one row per phase as its commit lands, then the close block. Readings, attempts and resume
> notes go to `docs/plans/logs/0060.md`.

| phase | owner | state | commit |
|---|---|---|---|
| 1: The queue's tiers as a reading | dev | not started | |
| 2: The view order | dev | not started | |
| 3: Withdrawing passed work | dev | not started | |
| 4: The owner's window | human | not started | |
| 5: The defaults and the close's readings | dev | not started | |

### Notes

### Close triggers

## Followups
