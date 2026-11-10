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
> the owner answered its four questions on 2026-11-10 (Owner answers, below).
> **Starts after:** Plan 0059's close (done, v0.44.0). Runs before Plan 0061 (antialiasing): 0061's Phase 1
> (readings only) runs beside this plan, and the rest of 0061 waits for this plan's merge (orchestrator,
> 2026-11-10). Independent of Plan 0058. It shares `crates/vxe-render/src/bake.rs` and
> `apps/flythrough/src/main.rs` with Plan 0045's split, so whichever lands second rebases its paths.

> **Owner answers, 2026-11-10** (to the four questions of the draft, through the orchestrator):
> 1. **Order: one blended score** of distance and angle to the view (the draft's option (c), not its
>    recommendation of hard tiers). The plan and ADR-0048 are revised to it: the score's weights are named
>    constants, Phase 1 reads what they need, Phase 2 builds it, the owner tunes it by eye in Phase 4.
> 2. **Passed work: withdraw** open and pending jobs outside the near shell and the keep cone (as drafted).
> 3. **The view:** the frustum widened by 15 degrees, plus the near shell (as drafted).
> 4. **Bake speed:** keep the 1.5 ms slice and judge the order first (as drafted).

## TL;DR

The baked light's live queue (`bake::BakeQueue`) opens chunks by distance first, so far light in the view waits
for every nearer chunk, behind the camera included, and a fast flight keeps baking chunks it has passed. This
plan reads where the queue's samples go in a flight first, then sorts the candidates by one score that blends
a chunk's distance with its angle to the view (the owner's pick), and then withdraws the jobs of chunks the
camera has passed. The first thing the owner sees is a HUD row that counts the chunks in view still waiting;
the last is a window where he turns the score's weight with a key until the far cave refines while he looks at
it and a fast flight's light lands ahead of him.

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

Build ADR-0048: the queue sorts its candidates by **one score**, `|o| x (1 + W_ANGLE x a) x (W_REFINE for a
coarse block, else 1)`, where `o` is the chunk's integer offset from the camera's chunk and `a` its angle beyond
the view cone over pi (0 inside the frustum's widest half-angle plus 15 degrees, and 0 in the near shell, every
direction); digs stay ahead of all. It **withdraws** an open or pending job that is not a dig or ring 3 once its
chunk leaves the shell and a wider keep cone while a candidate with `a` 0 is ready. Both are behind fly-through
flags, the weight on a live key, so the owner compares them with Plan 0059's order and tunes them in one window.
We rejected hard tiers (all of the view first), the draft's recommendation, on the owner's pick of the blended
score; a predicted eye (velocity lead) because it adds a knob and equals the view score at rest; and demotion
without withdrawal on the owner's answer 2.

## Architecture diagram

```mermaid
flowchart LR
    subgraph app[apps/flythrough]
        F[fly camera + View tangents] --> QV[QueueView: eye, basis, tangents]
        K[order, weight, withdraw keys] --> QV
    end
    subgraph render[vxe-render bake.rs]
        QV --> T[angle beyond the view cone, near shell]
        T --> S[score: distance x angle x refine weight]
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

### Phase 1: The view cone as a reading, and where a flight's samples go
- **Owner skill:** dev
- **What:** the queue computes each candidate's angle term and view class (near shell, in view, out of view)
  without changing its order, counts where its samples go, and logs what the score's weights need, so the HUD
  and stdout show what the owner complained about.
- **Files touched:** `crates/vxe-render/src/bake.rs` (`QueueView` gains the camera's basis and four tangents,
  an `angle_term` function and its unit test, `QueueStats` counters), `crates/vxe-render/src/lib.rs` (exports,
  if any), `apps/flythrough/src/main.rs` (fill the view from the fly camera's `View`, the stdout line),
  `apps/flythrough/src/hud.rs` (the queue row), the live queue's test file
  (`apps/flythrough/tests/queue.rs`, or its `gpu_` name if the backlog's test-tier chore has landed; the rig's
  `QueueView` takes the rig camera's tangents), `docs/plans/logs/0060.md`.
- **The counters (dev may rename):** per view class, the chunks waiting (no block, coarse) and the samples read
  back for jobs whose chunk was in that class when the job ended; the samples of jobs whose chunk was out of view
  and behind the camera (view cosine of its centre under 0) at their end ("passed"); for each chunk that enters
  the view with no full block, the time to its first block and to its full block (p50, p95 over the run).
- **What the weights need (logged once a second):** the waiting candidates as a histogram over distance (chunk
  bands 0-2, 3-5, 6-10, over 10), `a` (0, under 0.2, 0.2-0.4, over 0.4) and class; and, from it, the
  `W_ANGLE` at which the chunks the slice baked in the last second, re-ranked by the score, would have been in
  view at p50 and p90 of those chunks, and the `W_REFINE` at which a coarse chunk in view at distance 3 ranks
  with a blank one at 5 or 6. The log gives the range Phase 2's constants start in; the owner's eye picks.
- **Done when:**
  - A unit test `the_view_cone_scores_a_chunk_by_its_angle` (CPU, under 1 s): a camera at the centre of chunk
    [2, 0, 0] looking +x with a 70 degree vertical field at 16:9 (horizontal half-angle atan(tan 35 x 16 / 9),
    51.2 degrees, plus the 15 degree margin: 66.2) and `near` 1: [7, 0, 0] (straight ahead) has `a` 0;
    [-3, 0, 0] (straight behind) has `a` (180 - 66.2) / 180 = 0.632 within 0.005; [1, 0, 0] (behind, in the
    shell) has `a` 0; with `W_ANGLE` 0 every score is `|o|` (times `W_REFINE` for a coarse block).
  - The HUD's queue row shows `in view: N waiting (C coarse)`, and the per-second stdout line adds the samples
    by view class, the passed samples, the in-view ages and the weight log; under the default order every
    existing queue test still passes unchanged (the order is not touched in this phase).
  - Readings (RTX 3080 Laptop by `--adapter NVIDIA`, release, exclusive perf lock, headless 2530 x 1378; one row
    on the RADV RENOIR at 1280 x 720) into the readings log, with the 1-minute load:
    1. the patch at rest, facing down the cave (`--level --pose patch`, 120 s): the time until every chunk in
       view holds a full block, and the chunks in view still coarse at 30, 60 and 120 s;
    2. `--pose vein-flight --glide 30` and `--glide 120`: the share of the slice's samples by view class and
       passed;
    3. the streamed cave with `--glide 150` and `--glide 300` from the spawn (15 and 30 m/s at 10 cm voxels),
       60 s each: the same shares, the in-view ages, the no-value and coarse shares of the lighting pixels the
       existing line prints, and `waiting reach` among the chunks in view;
    4. the autopilot (`--autopilot --headless --seed 1`): the same shares, as the baseline for the close.
  - **Stop rule (dev reports; the orchestrator takes it to the owner before Phase 3 starts):** if in run 3 at
    `--glide 300` the passed samples are under 10 % of the slice's samples, the passed work is not the flight's
    fault, and the owner is asked again whether to build the withdrawal (answer 2) or demote only (ADR-0048's
    Alternative D). If the waiting chunks in view are mostly `waiting reach` (over half), the flight's fault is
    the streaming edge, not the order, and the plan stops for an owner question before Phase 2.

### Phase 2: The blended score
- **Owner skill:** dev
- **What:** `BakeQueue::open` sorts by ADR-0048's score, ties by the view cosine then the coordinate, the
  pre-roll included, behind `--queue-order score|distance` (default `score`; `distance` is Plan 0059's key,
  class first) and a live key; the weights are named constants `QUEUE_ANGLE_WEIGHT` and `QUEUE_REFINE_WEIGHT`
  (starting values from Phase 1's log; 4 and 2 are guesses until then), overridden by `--queue-weights A,R`,
  and a live key pair steps the angle weight (halve, double; dev picks free keys and the banner names them).
- **Files touched:** `crates/vxe-render/src/bake.rs`, `crates/vxe-render/src/marcher.rs` (a setter, if the queue
  is reached through `Frame`), `apps/flythrough/src/main.rs`, `apps/flythrough/src/hud.rs`, the live queue's
  test file, `docs/plans/logs/0060.md`.
- **Done when:**
  - `the_score_blends_distance_and_the_view` (lavapipe, gpu tier, about 3 s by its siblings' 2.5 s): the
    six-chunk row of `the_queue_runs_near_first_and_within_its_samples`, camera in chunk 2 looking +x, `near` 0,
    coarse pass on, `W_REFINE` 2. Chunks first appear in `jobs()` in the order [2, 3, 4, 5, 1, 0] at `W_ANGLE` 4
    (scores 1, 2, 3 ahead; 1 x 3.53 and 2 x 3.53 behind), [2, 3, 1, 4, 5, 0] at `W_ANGLE` 1 (behind 1.63 and
    3.26), and [2, 3, 1, 4, 0, 5] at `W_ANGLE` 0 (distance, ties ahead first). Under `distance` the rig reads
    Plan 0059's [2, 3, 1, 4, 0, 5].
  - `the_queue_runs_near_first_and_within_its_samples` passes unchanged at `near` 2 (chunks 0 to 4 are the near
    shell, `a` 0, so they keep distance order; chunk 5, beyond it and in view, comes last).
  - `a_turn_resorts_the_queue` (lavapipe, about 3 s): the same row at `near` 0 and `W_ANGLE` 4, the camera
    turned to look -x after its own chunk opens; of the chunks not yet in `jobs()` at the turn, 1 and 0 open
    before 4 and 5.
  - `the_queue_settles_to_a_direct_bake` and a copy with the camera turned half-way pass: every block
    bit-identical to a one-shot bake whatever the order and the weights (ADR-0046's order independence).
  - The banner names the order and the weights (`order score, angle 4, refine 2` or `order distance`) and the
    HUD's queue row shows them, changing on a key in the same frame; stdout prints a line at each change.
  - Phase 1's runs 1 to 4 read again under `score` at the starting weights into the readings log, beside Phase
    1's rows; the pre-roll's first frame at the vein and the hall beside Plan 0059 Phase 8c's 7137 and 1839 ms.

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
  - Phase 1's runs 2 to 4 read again under `score` with withdrawal on, beside the Phase 2 rows: the passed samples'
    share, the samples withdrawn, the in-view ages.

### Phase 4: The owner's window
- **Owner skill:** human
- **What:** the owner judges the order and the withdrawal by eye on the release binary, stdout visible.
- **Files touched:** `docs/plans/logs/0060.md` (the owner's words verbatim, by the orchestrator or `dev`).
- **The runs** (a `live/session60.sh` in the lane, untracked, the fly-through's defaults with `--hud`; each step
  starts on `order score` at Phase 2's weights with withdrawal on; the owner steps the angle weight and swaps
  the order and the withdrawal with the keys):
  1. the patch at rest, looking down the cave: does the far light in view refine soon enough, nearest first,
     and at which angle weight;
  2. the streamed cave, flown fast with the pad or keys: does the coarse light land ahead of the flight, and
     what does a turn back show;
  3. the vein: a slow flight and a dig, to confirm nothing regressed;
  4. the Radeon, step 2 at 1280 x 720.
- **Done when:** the owner has picked the angle and refine weights (or `distance`) and the withdrawal (on or
  off), or has named what is still wrong; the words and the weights on screen are in the readings log.

### Phase 5: The defaults at the owner's pick, and the close's readings
- **Owner skill:** dev
- **What:** the defaults follow Phase 4's picks, the weights as the named constants' values; a losing arm's flag stays as a comparison flag unless the owner
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
pub enum QueueOrder { Score { angle: f32, refine: f32 }, Distance }
pub const QUEUE_ANGLE_WEIGHT: f32 = 4.0; // a guess until Phase 1's log; the owner's pick at Phase 4
pub const QUEUE_REFINE_WEIGHT: f32 = 2.0; // the same
// QueueStats gains: per view class waiting and samples, passed samples, withdrawn jobs and samples.
```

## Budgets

- **Section 2, frame.** The slice stays 1.5 ms (`DEFAULT_BAKE_SLICE_MS`, owner answer 4). The queue step's CPU
  per frame: the score adds an angle and a multiply per candidate (up to about
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

- **The streaming edge, not the order.** A chunk in view within the light radius of the mirror's edge waits
  for its reach (`reach_ready`). If the flight's in-view chunks mostly wait there, ordering cannot help; Phase
  1's stop rule catches it.
- **No weight is right everywhere.** A blended score lets a near chunk behind beat a far one in view at any
  finite angle weight; the patch at rest and the cave in flight may want different weights. Phase 4 reads the
  owner's weight on both; if they differ, the owner picks one or asks for a later plan.
- **Churn.** A withdrawn chunk that comes back into view starts over. The keep cone and the "a better candidate
  is ready" condition bound it; Phase 3's edge test and the withdrawn samples read it.
- **A turn shows the back.** Behind the near shell the light is coarse or absent until the queue reaches it.
  The window decides whether `near` 2 is enough.
- **Determinism.** Render-side only; `vxe-world` is not touched. Order, weights and withdrawal must not change
  a block's bits; Phases 2 and 3 assert it against a one-shot bake.
- **Open:** none left from the draft; the owner answered all four on 2026-11-10 (the header). Phase 1's stop
  rule can raise one.

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
| 1: The view cone as a reading | dev | not started | |
| 2: The blended score | dev | not started | |
| 3: Withdrawing passed work | dev | not started | |
| 4: The owner's window | human | not started | |
| 5: The defaults and the close's readings | dev | not started | |

### Notes

### Close triggers

## Followups
