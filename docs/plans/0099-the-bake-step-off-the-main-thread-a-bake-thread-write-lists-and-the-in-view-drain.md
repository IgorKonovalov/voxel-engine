# 0099: The bake step off the main thread: a bake thread, write lists the render thread applies, and the in-view drain

> **Status:** approved (owner, 2026-11-22; his answers are under "Risks & open questions":
> the recommended option in each, question 2 by his own words). **After: 0096, 0098, 0087** (owner, 2026-11-22:
> drafted now, the lane after the rename; then 0089 and 0093, 0091, ...).
> **Re-read 2026-11-24** after Plan 0098 (v0.72.0) and the rename (v0.73.0): Phases 1 to 3 are
> amended under "Amended 2026-11-24 (architect re-read)", which wins over the phases' text where
> the two differ; **question 6 (the dig's held record with the thread on) is open with the
> owner** and holds Phase 3's dig clause only. Phases 1 and 2 start without it.
> **Created:** 2026-11-22
> **Milestone:** M10 (the outdoor scenes are where the queue is deep) and cross-cutting, like
> [Plan 0096](done/0096-the-bake-queues-drain-tiers-a-drain-slice-the-step-and-the-slow-adapters-light-pass.md):
> it serves `docs/nfr.md` section 2's streaming line, the bake queue's step inside it and the
> Floor frame, and takes the lines Plan 0096 closes with as disclosed misses.
> **Related ADRs:** [ADR-0083](../adrs/0083-the-live-bake-queue-runs-on-a-bake-thread-the-render-thread-applies-its-write-lists-and-issues-its-submits.md)
> (proposed with this plan), which extends
> [ADR-0017](../adrs/0017-mirror-cpu-work-bounded-in-the-frame-loop-narrowing-on-stream-workers.md);
> [ADR-0080](../adrs/0080-the-live-queue-bakes-what-the-image-reads-a-record-by-rule-a-tier-by-size-on-screen-and-a-drain-slice.md)
> (the tiers and the drain slice),
> [ADR-0076](../adrs/0076-light-stability-rules-world-history-per-source-a-fade-per-transition-and-a-ratcheted-pop-counter.md)
> rules 3 and 6, [ADR-0046](../adrs/0046-static-emissive-light-is-baked-per-chunk-when-a-chunk-loads-or-changes.md)
> (the store's bit-identity).
> **Source:** Plan 0096's Phase 8 window (2026-11-22) and its section "The window's verdict and
> what is left"; the readings are in `docs/plans/logs/0096.md` on branch `plan-0096` until that
> plan merges.

## TL;DR

After a fresh start in the forest the RTX 3080 Laptop holds about 30 fps while the bake queue
drains, because the queue's step costs 28 ms a frame on the render thread; the GPU needs 11.6.
This plan moves the queue's books and all of its per-job CPU work to **one bake thread**
(ADR-0083). The render thread sends the view and the mirror's changes once a frame, never
waits, and applies what comes back: **write lists** (a block's words and its table word always
together, so no frame reads half a landing) under a byte cap, and at most one prepared submit.
The same code runs in **lockstep** on the calling thread as a setting, which is how the tests,
the pre-roll and every assertion on the undrained image stay reproducible. The store's bytes
do not change. Then the bake thread is made to keep the submits fed, because the drain slice
is granted per frame and a doubled frame rate is twice the bake a second only if the CPU side
keeps up. The first thing anyone sees: the `bake queue step:` line splits the step into what
must stay on the render thread and what can leave, on the unchanged forest fresh start.

## Context & problem

Every figure is a reading unless marked. "RTX" is the NVIDIA GeForce RTX 3080 Laptop GPU,
"RENOIR" the AMD Radeon Graphics (RADV RENOIR), the slow adapter of `docs/nfr.md` section 1.
The readings are Plan 0096's (`docs/plans/logs/0096.md` on `plan-0096`).

**The window (Phase 8, 2026-11-22, release `1844a99b`, 2530 x 1378, internal 1687 x 919, mains,
performance profile, 1-minute load 0.98).** Forest: `--source outdoor --spawn forest --trees
forest --time 15:00`; hills: `--source outdoor --time 17:00`.

| run | the owner's words | fps | wall p50 / p99 | frame CPU p50 | march GPU p50 | step, regime B, p50 / p99 | in view landed after the first world frame |
|---|---|---|---|---|---|---|---|
| RTX forest (a1) | "works, but fps around 30" | 25 to 41 | 28.3 / 59.1 ms | 28.0 ms | 11.6 ms | 28.0 / 51.1 ms (465 steps) | not reached in 17.9 s |
| RTX hills (a2) | "fully settled in around a minute or so if I am looking for stable fps (70+)" | 31 to 45, then 55 to 69, then 68 to 77 | 13.3 / 34.3 ms | 1.2 ms | 11.8 ms | 13.1 / 34.3 ms; drained 0.05 to 0.07 | 16.1 s; empty at 27.5 s |
| RTX forest, profile tiers (b1) | none | not split | 20.5 / 60.5 ms | 19.8 ms | 12.1 ms | 21.5 / 60.0 ms | 22.3 s |
| RENOIR forest (a3) | "very slow, around 8-9 fps" | 8 to 11 | 106.8 / 119.5 ms | 9.8 ms | 100.6 ms | 8.7 / 17.5 ms | not reached |

The RENOIR is GPU-bound at that size (3.79 times the Portable profile's pixels; Plan 0096
Phase 9 re-reads it at 720p). Its step is 7 to 9 ms all the same.

**What a step costs, by stage** (Plan 0096 Phase 5, `e3b03e75`, RTX, Floor, 1920 x 1080,
headless, the forest at 15:00, loaded; stage means a line in regime B): `list` 10.0 to 18.1 ms
(the faces of the 16 to 32 candidates an opening admits, about 1 ms each, already spread over
scoped threads the render thread waits for), `collect` 5.0 to 7.0, `submit` 5.0 to 8.1,
`fades` 4.3 to 7.5, `summaries` 1.6 to 2.6; together 25.9 to 43.3 ms. `candidates` reads 1.1
to 1.6 and the depth-bound stages are gone (0 to 56 candidates scored a step against a pool
of 9 361). The step read 20.7 / 55.0 ms p50/p99 against Plan 0096's proposed line of **1.0 /
4.0 ms**, which that plan pins as a disclosed miss and hands here.

**What is not known, and Phase 1 reads.** How each of those stages splits between pure CPU
(which can leave the render thread) and `wgpu` calls (`write_buffer` copies, the encode, the
submit, the mapped read; ADR-0083 keeps the writes and the submits on the render thread).
ADR-0017's Outcome found `write_buffer` at 41.8 % of the mirror sync's samples, so the split
is not a formality: it decides whether the render thread's share can reach 1.0 ms and which
of Phase 4's levers are needed.

**The drain is the other half.** Plan 0096 proposed "in view landed at most 10 s after the
first world frame" on the RTX and reads 16.1 s (hills) and 22.3 s (forest) at the window. The
drain slice (4.0 ms at Floor while in-view work waits) is granted per frame and the submits
draw 0.94 to 0.99 of it, so at about 30 fps the bake gets about 120 ms of GPU a wall second.
At 70 fps the grant is 280 ms. **Arithmetic, not a reading:** 22.3 s x 30 / 70 is 9.6 s, if
the bake side prepares 2.3 times the jobs a second. One regime B step's CPU (26 to 43 ms) fed
4.35 to 5.48 ms of RTX GPU (Plan 0096 Phase 4), so one thread running nothing else makes 23 to
38 steps' worth a second, 100 to 210 ms of GPU, **36 to 75 % of that grant**. Moving the step
restores the frame rate; it does not by itself shorten the drain. Phase 4 is for that, and
takes the opening's candidate cap with it (Plan 0096's F2 reading: the per-second rig draws
0.60 to 0.69 of its grant because an opening admits at most a fixed count of candidates).

**The rules this is held to** (owner): a stable image is the one priority, performance
considered; no waiting for light (ADR-0076 rule 6; a minute's settle is unacceptable); a
budget is measured on named hardware and a figure without a measurement says it is a guess;
no scene or fixture is simplified so a bar passes; `svet-world` stays headless and
deterministic and GPU code lives in `svet-render` only.

## Decision

The queue's books and its per-job CPU work run on one dedicated bake thread; the render
thread keeps every `wgpu` submit and every write the image reads, applied once a frame from
write lists; the same code runs in lockstep as a setting. ADR-0083 holds the decision and its
costs. We rejected **spreading the step over the stream workers** because only the listing is
a pure function of a chunk and 16 to 25 ms would stay; **capping the opens on the render
thread** because it buys the frame rate with a longer drain; **amortising the listing**
because the step still reads 16 to 25 ms with the listing free; **a bake thread that also
submits and writes** because a block and its table word could land in different frames, a
pop by construction; and **a fork and join inside the frame** because the frame still waits
for a 28 ms step.

## Architecture diagram

```mermaid
flowchart LR
    subgraph world [svet-world]
        CH[chunks behind their own locks]
    end
    subgraph render [svet-render: the render thread, once a frame]
        SY[mirror sync]
        MSG[frame message: view, frame number,<br/>mirror changes, mapped read-backs]
        AP[apply write lists whole,<br/>under the byte cap]
        SUB[issue the prepared submit]
        LP[light pass reads pool and table]
    end
    subgraph bake [svet-render: the bake thread]
        IN[inbox, coalescing]
        BK[the queue's books: order, opening, listing,<br/>rounds, accumulation, placement,<br/>fades, summaries]
        OUT[outbox, bounded]
    end
    SY --> MSG --> IN --> BK --> OUT --> AP --> SUB
    AP --> LP
    CH -. read locks, one chunk's listing .-> BK
    SUB -. read-back maps, a frame or two on .-> MSG
```

In lockstep the three bake boxes run inside the frame's call on the calling thread, one step
per frame, and the two channels hold one item.

## Implementation phases

Each phase ships as its own commit. `dev` implements all phases in one session with no review
between phases; the architect reviews once at the end, in a fresh session.

**Starts after Plans 0096 and 0098 have merged and after Plan 0087 (the rename)**: 0096 is the
base for this subsystem, 0098's Phase 6 edits the landing fade in the same step file, and 0087
renames the crates and identifiers. **The tree's names win**: every path and identifier below
is as of `plan-0096`'s tip `33003841` and names a mechanism; where the tree differs, dev
follows the tree and notes it. Phase 1 re-reads the base, because Plan 0096's Phase 9 and
Plan 0098 both move it.

**Inherited from Plan 0098** (added 2026-11-24 by the architect at that plan's close, v0.72.0;
facts of the base; the architect's re-read of Phases 1 to 3 against them is the section "Amended
2026-11-24 (architect re-read)"). Plan 0098 closed after this plan was approved and changed the
dig's path through `bake/queue/`:

- **A dig's ring 1 is a job of its own.** `open_digs` splits a dug chunk: the bricks the box
  dilated by one voxel touches re-bake first, in a job opened in the edit's step into a table
  slot (`open_now`; the last open job that is neither a dig's nor key-only is withdrawn where
  none is free) and promoted ahead of every job; the chunk's other dig bricks open as a second
  job once the first is placed **and its `DIG_FADE` fade has ended**. A chunk splits only where
  its provisional block took the listing's size in the pool.
- **Stale jobs are withdrawn at the edit**, not awaited (`take_boxes`, `withdraw_at`,
  `JobLoop::withdraw`): a dig's bricks rejoin `digs`, another job's chunk returns to the
  candidates. A ring-1 job with no live face closes in its step's submit
  (`JobLoop::close_ended`); a placement that throws a stale job's block away marks its chunk
  touched.
- **The held record is made inside `take_boxes`** (`bake/held.rs`, through `list_dug` on
  scoped listing threads): a CPU estimate against the world and the store's blocks as the
  edit found them, written over the provisional block. It moves to the bake thread with the
  step; it reads the world's chunks under their read locks (the "Risks" line on a dig's write
  lock applies to it too).
- **The base of "an edit's light"** (`docs/nfr/04-edits.md`, "An edit's light"): ring 1 lands 1
  or 2 frames after the dig on the RTX 3080 Laptop and on the RADV RENOIR at Portable, on a
  waiting queue too, so the Budgets table's line reads "within 4 frames" with the thread on;
  the lavapipe image test's bar is 8 frames and "no later than the reference plus 1".
- **The dig frame's step time is a reading this plan owns**: 25.9 ms (sequence A) and 14.5 ms
  (B) in the means on the RTX, about 7 and 5.5 on the RENOIR, with a run-to-run spread of up
  to 30 ms on the RTX. Phase 1 reads it again with at least eight alternated runs a tree
  before any bar is held against it. The dig rig's queue never drains (its frames do not wait
  on the device; `docs/backlog.md`, "Plan 0098's leftovers"), which Phase 1's base reading on
  that rig must fix or avoid.
- **Tests that pin this behaviour:** `gpu_queue::a_dig_jumps_every_open_job_and_withdraws_its_stale_one`,
  `gpu_queue_reach::{a_fade_moves_a_record_by_its_share_a_frame, a_full_pool_keeps_the_queue_alive,
  an_emitter_dug_away_darkens_what_it_lit}`, `gpu_sun::a_dig_rebakes_the_key_words_with_the_sky_ring`,
  `bake::held::tests`, and the soak image test
  `flicker::a_fresh_hole_holds_its_value_until_its_light_lands`. `open_digs` knows a dig's fade
  by `frames == DIG_FADE`, which a fade paced by elapsed frames (Phase 2) must keep true or
  replace with a flag.
- **Files near the size gate:** `bake/queue/mod.rs` 1060 and `bake/job_loop.rs` 1047 of 1200
  lines.

**Test cost** (the rules Plans 0096 and 0098 run under): per phase,
`node scripts/test-changed.mjs --profile quick` over the changed crates; the phase's GPU run
is `--profile gpu --binaries` with the binaries the phase names, under the perf lock; clippy
and fmt per phase; `cargo nextest run --profile gate` once, at the tip (Phase 6). No soak or
full tier. Every perf reading is release, under the exclusive perf lock, with the 1-minute
load logged (under 2 at the start to pin a line; a loaded run is labelled indicative and pins
nothing), on the RTX (`--adapter RTX --profile floor`, 1920 x 1080, FSR 2 at 1.5) and the
RENOIR (`--adapter RENOIR --profile portable`, 1280 x 720), **and names its mode, lockstep or
thread**. The scene is the forest fresh start the owner saw, flags unchanged (above), from the
first world frame, with the hills at 17:00 beside it; no tree count, radius, tier or sample
count is lowered for a bar. The RTX forest is also read at the window's 2530 x 1378 as a
reading (no line is defined at that size). This plan is not look-driven: the store's bytes
are asserted equal, so no phase is exploration and no golden is expected to move.

### Phase 1: The step's split on the line, and the base readings
- **Owner skill:** dev
- **What:** No behaviour changes. The step's stage timers gain a second axis: each stage's
  time in `wgpu` calls (`write_buffer`, the encode, the submit, the poll, the mapped read and
  unmap) against the rest, and the bytes written a step to the pool, the table and the
  summary buffers. The `bake queue step:` line and the HUD's queue row print the two sums as
  `render` (what ADR-0083 leaves on the render thread) and `movable`. The fresh-start rig is
  named (a script line in the readings log, the window's flags). Readings: the base on both
  adapters and both scenes; the RTX forest at the window's size; the dig's ring today (frames
  from a dig that removes an emitter to the near ring's records changing, the existing
  `gpu_bake` test's rig); the edit's wait today (p99 of a dig op's wall while the queue is in
  regime B); the same RTX forest run pinned to 8 logical CPUs of 4 cores (`taskset`), labelled
  as the dev box's stand-in for the floor's CPU, not as the floor.
- **Files touched:** `crates/svet-render/src/bake/stages.rs`,
  `crates/svet-render/src/bake/queue/{step,stats}.rs`, `crates/svet-render/src/bake/store.rs`,
  `crates/svet-render/src/bake/job_loop.rs`, `apps/flythrough/src/run/queue.rs`, the HUD's
  queue row, `docs/flythrough.md`, `docs/plans/logs/0099.md` (new).
- **Done when:** (1) `the_step_line_splits_render_and_movable_and_they_sum_to_the_step`
  (CPU, under 1 s): a step's stage record with known laps formats to a line whose `render`
  and `movable` sums equal the stages' sum within 0.001 ms. (2) The readings log holds, per
  adapter and scene, in regime B: the step p50/p99, `render` and `movable` p50/p99, the bytes
  written a step p50/p99, the frame's wall and CPU p50/p99 while in-view work waits, the
  in-view and whole-residency drain after the first world frame, the submits' drawn share of
  the granted slice; and the dig's and the edit's base figures. (3) **A stop, not a bar:** if
  `render` p50 on the RTX forest is over 1.0 ms, dev reports it to the architect before
  Phase 2 with the split by call; the staging-belt lever of Phase 4 then moves ahead of
  Phase 3 by amendment. (4) `--binaries gpu_queue gpu_bake` pass unchanged.

### Phase 2: The queue in two halves, in lockstep
- **Owner skill:** dev
- **What:** The step is cut along ADR-0083's line with both halves still on the calling
  thread. The **bake half** owns the queue and the store's books and reads nothing of the
  frame but a message: the view, the frame's number, the mirror's changes with each changed
  chunk's new state, the store's window, the mapped read-backs, the GPU times. It keeps a
  shadow of the mirror's membership from those changes and holds no reference to the mirror.
  It returns one **write list** (ordered buffer writes: block words, table words, summary and
  parent words, fades' shown words, the window's move) and at most one **prepared submit**
  (planned rounds and the entry table's bytes). The **render half** holds the GPU buffers
  and no rule: it applies whole lists in order under a byte cap, carries what does not fit,
  writes and issues the prepared submit after the sync, and collects mapped read-backs. A
  submit is stamped with its frame; a read-back is placed only after the changes up to the
  frame it mapped in. Fades are paced by the frame number in the message. The store's table
  is read by the light pass through the store's own window, so a window that moves a step
  late shows the fallback chain for an entering chunk, never another chunk's record; if the
  tree reads the table by the mirror's window, that is fixed here and noted. Public reads of
  the books (statistics, a block's words, the bake file) go through the bake half.
- **Files touched:** `crates/svet-render/src/bake/queue/` (the step and its neighbours, and
  new files for the message, the write list and the shadow membership),
  `crates/svet-render/src/bake/{store,job_loop,summary,job}.rs`,
  `crates/svet-render/src/frame/baked.rs`, `crates/svet-render/src/bake/queue/tests.rs`,
  `crates/svet-render/tests/gpu_queue.rs`.
- **Done when:** (1) `a_landing_is_applied_whole_or_not_at_all` (CPU, under 1 s): against a
  recording applier, three lists of 40, 40 and 40 KiB under a 64 KiB cap apply as one list a
  frame over three frames, in order; no frame holds a block's words without its table word;
  a single list over the cap is refused at its making, not split. (2)
  `a_change_after_a_submit_stales_its_result_and_one_before_does_not` (CPU, under 1 s): a
  submit stamped frame 5 whose read-back maps at frame 7: a change inside its chunk's reach
  in the frame 6 message releases the block and re-queues the chunk; the same change in the
  frame 4 message leaves it placed. (3) `the_shadow_membership_answers_as_the_mirror_does`
  (CPU, under 2 s): over a recorded sequence of 2 000 mirror changes (arrivals, LOD swaps,
  evictions, a window move), the shadow's held-full, LOD-only and lights-ready answers equal
  the mirror's after every message. (4) `a_fade_is_monotone_when_steps_are_skipped` (CPU,
  under 1 s): a 16-frame fade whose bake steps arrive at frames 1, 2, 5, 9 and 16: every
  shown word lies between its old and new value, never moves back, moves at most the elapsed
  frames' share of the change plus one unit of rounding, and equals the new value at frame
  16. (5) The existing `gpu_queue` test that settles a 5 x 5 x 5 world through the live queue
  against a one-shot bake still reads every block bit-identical (lavapipe, about 16 s loaded
  at Plan 0096; over the soak line it is run by name here and marked as that plan leaves it),
  and `gpu_bake` passes unchanged. (6) Reading: the lockstep step on the RTX forest is within
  10 % of Phase 1's base p50 (the cut itself must not cost).
  `--binaries gpu_queue gpu_bake`.

### Phase 3: The bake half on its own thread
- **Owner skill:** dev
- **What:** One named thread runs the bake half in a loop: take the inbox (coalescing: the
  newest view replaces the older, change lists append in order), run one step, push the list
  and the submit to a bounded outbox, wait when it is full or when there is nothing to do.
  The render thread's calls only swap and try: it never waits on the bake thread. A render
  setting `bake thread` (`on` in a window, `off` being lockstep; ADR-0055's row, a
  settings-file row, `--bake-thread on|off`) picks the mode and **the HUD's queue row and the
  banner show it**. Lockstep stays the mode of the pre-roll, a one-shot bake, and every test
  and headless run that asserts bytes before the drain; the reading rigs pass `on`. A dig's
  ring is a priority message: the bake thread serves it before the opening. The world's
  chunks reach the thread through a handle the world already shares with its stream workers
  (or one added to `svet-world` that carries no bake concept); a read lock is held for one
  chunk's listing at most. Dropping the renderer stops and joins the thread; a panic on it
  surfaces as the step's error on the next frame.
- **Files touched:** `crates/svet-render/src/bake/queue/` (the thread, the inbox and outbox),
  `crates/svet-render/src/frame/{baked,settings}.rs`, `crates/svet-render/src/settings.rs`,
  `crates/svet-world/src/store.rs` only if a shareable handle is missing,
  `apps/flythrough/src/cli/` (the flag and its test), `apps/flythrough/src/hud/` (the row),
  `apps/flythrough/src/run/queue.rs`, `crates/svet-render/tests/gpu_queue.rs`,
  `docs/flythrough.md`.
- **Done when:** (1) `the_render_side_returns_with_the_bake_side_stalled` (CPU, under 2 s):
  with the bake half held by a test hook, 100 frame calls return, apply nothing, and leave
  an inbox of one view and a change set no larger than the distinct chunks sent. (2)
  `the_outbox_is_bounded_and_the_bake_side_waits` (CPU, under 2 s): with the render half not
  applying, the outbox stops at its cap in bytes and the bake half's step count stops
  rising. (3) `dropping_the_queue_joins_the_bake_thread` and
  `a_panic_on_the_bake_thread_is_the_next_steps_error` (CPU, under 1 s each). (4)
  `the_threaded_queue_drains_to_the_lockstep_store` (`gpu_queue`, lavapipe, a guess of 20 s,
  so born `#[ignore = "soak"]` and run by name in this phase and at the tip): the same
  5 x 5 x 5 world drained with the thread on holds every block bit-identical to the lockstep
  store's and to a one-shot bake's. (5) The dig: with the thread on, the near ring's records
  change within Phase 1's base frame count plus 2 (`gpu_bake`, the existing dug-emitter rig
  with the mode as a parameter, at the wall the tree's test has). (6) `the_flag_and_the_row_name_the_mode` (CPU,
  under 1 s): `--bake-thread off` parses to lockstep and the HUD's row reads `bake thread
  off`; unset in a window it reads `on`. (7) **The bars, thread on, quiet, RTX, the forest
  fresh start at Floor 1080p, regime B:** the render thread's bake share **p50 at most
  1.0 ms and p99 at most 4.0 ms**; the frame's wall **p50 at most 16.6 ms** while in-view
  work waits; the same share bar on the RENOIR at Portable 720p, its wall p50 not over Phase
  1's base by more than 2 %. (8) Readings: the frame's fps over the drain at the window's
  size; the share of frames that carried a fade step while a block was fading (a guess: at
  least 90 % at 60 fps); the edit's wait p99 against Phase 1's base; the `taskset` arm;
  the in-view drain, which this phase is not expected to shorten.
  `--binaries gpu_queue gpu_bake`.

### Phase 4: The bake thread keeps the slice fed
- **Owner skill:** dev
- **What:** The drain's bar, by the largest bake-side stage first, as Phase 3's readings
  order them. The named levers, each taken only if its stage is in the way: (a) **the opening
  admits to the slice**: candidates are admitted until the opened jobs' projected samples
  (the tier's count times the faces listed, at the measured time a sample) cover the granted
  slice at the frame rate read, in place of the fixed count per step (Plan 0096's F2
  entry); (b) **the listing off the bake loop**: a kept pool in place of threads spawned per
  batch, sized so the process does not run more busy threads than logical cores beside the
  stream workers, the bake loop taking results as they finish; (c) **the fades**: mixed only
  for blocks whose target differs from what is shown, at the cadence the frame number sets;
  (d) **the copies**, only if Phase 1's stop fired or the byte cap is what holds the drain:
  the bake thread fills a mapped staging range and the render thread encodes copies
  (ADR-0022's mechanism, no new decision). No lever changes a record's bytes, a tier or the
  slice.
- **Files touched:** `crates/svet-render/src/bake/queue/{open,list,digs}.rs` and the thread's
  files, `crates/svet-render/src/bake/{order,budget,store}.rs`,
  `crates/svet-render/src/bake/queue/tests.rs`, `apps/flythrough/src/run/queue.rs`.
- **Done when:** (1) `the_opening_admits_until_the_projected_samples_cover_the_slice` (CPU,
  under 1 s): a synthetic pool whose jobs project 0.4 ms each under a 4.0 ms slice opens at
  least 10 and at most 11 a step (10 x 0.4 covers 4.0); under a 1.0 ms slice at least 3 and
  at most 4 (3 x 0.4 is the first sum over 1.0); one job of overshoot, never a fixed count. (2) `the_opened_jobs_are_the_nearest_of_their_band`
  (the existing order test, CPU, under 2 s) holds: the levers change how many open, not which
  first. (3) Phase 3's test 4 and the lockstep store test still pass. (4) **The bars, thread
  on, quiet, from the first world frame:** on the RTX at Floor 1080p the submits draw **at
  least 0.85 of the granted drain slice** while in-view work waits, and **in view has landed
  within 10 s** on the forest and on the hills; on the RENOIR at Portable 720p **within
  30 s**; Phase 3's share and wall bars still hold. (5) Readings: the whole-residency drain
  on both adapters; the per-second rig (`second:150,0.25`) drawn share, Plan 0096's F2
  figure; the bake thread's busy share of a core and the process's runnable threads against
  the logical cores, with the `taskset` arm beside them. (6) A bar in (4) that is missed
  after the levers are in is disclosed with the stage that holds it; it is not met by
  raising the slice or lowering a tier. `--binaries gpu_queue gpu_bake`.

### Phase 5: The owner's window
- **Owner skill:** human
- **What:** One window, on Linux, on the dev box on mains, release at Phase 4's tip, the
  mode shown on the HUD. The runner starts each run and the stdout stays visible. (a) RTX,
  Floor, the forest fresh start, thread on, then the same with `--bake-thread off`: is the
  frame rate there from the first seconds, and is anything different in the image but the
  frame rate (a landing that pops, a fade that steps, a block that flashes)? (b) RTX, the
  hills at 17:00, thread on: how long until it reads settled. (c) RTX, a few digs next to a
  lamp while the queue is deep: does the light follow the dig as it does with the thread
  off? (d) RENOIR, Portable (at its 720p, as Plan 0096 Phase 9 leaves it), the forest,
  thread on: nothing worse than before.
- **Files touched:** none (the verdicts go to `docs/plans/logs/0099.md`, recorded by the
  architect or the orchestrator).
- **Done when:** the owner has said, for (a) to (d), fine or what is wrong, in his words; an
  item he did not judge is recorded as unjudged, and an image-stability item takes no
  recommended answer in his place.

### Phase 6: The lines, the defaults and the documents
- **Owner skill:** dev
- **What:** The window's findings in mechanisms this plan built are fixed here or routed by
  the architect's amendment. The lines are pinned quiet on both adapters, in the thread
  mode: the render thread's bake share, the frame's wall while draining, the in-view and
  whole-residency drain, the submits' drawn share, the dig's ring and the edit's wait, the
  outbox's and the shadow's peak bytes. Each goes to `docs/nfr.md` section 2 or its readings
  file with machine, adapter, scene, mode and commit; a line that is missed is pinned as a
  disclosed miss with its cause, not moved to its reading. `docs/flythrough.md` and the
  flag's help name the mode. The backlog's entries this plan took are closed into it.
- **Files touched:** `docs/nfr.md`, `docs/nfr/02-frame-budget.md`, `docs/flythrough.md`,
  `docs/backlog.md`, whatever the window's findings touch, `docs/plans/logs/0099.md`.
- **Done when:** (1) every figure under Budgets has a quiet reading on both adapters with
  its mode, or is named as not read with the reason. (2) Every test this plan named passes
  at the defaults; the soak-marked ones were run by name once at this tip. (3) The gate at
  the tip: build, `cargo nextest run --profile gate`, doctests, clippy, fmt, deny, the doc
  gates; its wall is recorded against section 7. (4) No golden moved; one that did is
  reported with the frame it differs in, not re-blessed.

## Amended 2026-11-24 (architect re-read): Phases 1 to 3 against the tree

Read against `be97152a` (v0.73.0), the code and not this plan's memory of it. The phases'
intent stands. What follows restates only the What, Files touched and Done when that change;
where it differs from the phase's text above, this section wins. Phase 4 on is not re-read,
with one name fixed: its order test is
`the_opened_jobs_are_the_nearest_of_their_band_whatever_the_depth`.

**Names the tree has.** The queue's GPU tests are `apps/flythrough/tests/gpu_queue.rs` and
`gpu_queue_reach.rs` (no `crates/svet-render/tests/gpu_queue.rs`); `gpu_sun` is under
`crates/svet-render/tests/`. `gpu_bake`'s dug-emitter test
(`a_dug_emitter_leaves_the_near_ring_at_once`) digs through the blocking `Frame::rebake_digs`
and never enters the live queue, so it is not the dig's base or bar: the queue's dig rigs are
`gpu_queue::a_dig_jumps_every_open_job_and_withdraws_its_stale_one` and `gpu_outdoor`'s
`dig_line::reading::dig_timeline` (`SVET_LINE`, sequences A and B, stepped by
`support::defects::Stepper`). The flag lands in `apps/flythrough/src/cli/queue_flags.rs`, the
render settings under `crates/svet-render/src/frame/settings/`. Every other path the three
phases name exists.

**Where a dig's path lives.** In lockstep (Phase 2) all of it runs in the edit's step, as today,
so ring 1 still lands 1 or 2 frames on. On the thread (Phase 3) each row is a frame or more
later unless question 6 is answered (a).

| piece (the tree's name) | Phase 2: which half | Phase 3: which thread | guarded by |
|---|---|---|---|
| the box's intake (`Frame::invalidate`, `BakeQueue::invalidate`) | the render half puts the box in the frame's message; the books' part (sealed, masks, facelessness) is the bake half's | bake thread, ahead of the opening | Phase 2 (7), Phase 3 (5a) |
| `take_boxes`: `withdraw_at`, the sky-stale marks, `list_dug` with `held_visibilities`, the provisional and held block's put | bake half; its writes are that step's list; the world under read locks | bake thread | Phase 2 (7); `bake::held::tests`; the soak image test by name |
| `open_digs`, `open_now`, ring 2's wait on ring 1's fade | bake half | bake thread | Phase 2 (8); `gpu_queue`'s dig test |
| ring 1 promoted, `close_ended`, its rounds planned first | bake half plans; the render half writes the slot's candidate table and issues the submit after the sync | the plan on the bake thread, the issue on the render thread a list later | Phase 2 (7), Phase 3 (5b) |
| the read-back, `Placement::place`, `refade` over `DIG_FADE` | the render half collects the map; the bake half places | bake thread | Phase 2 (2), `gpu_queue_reach` |
| `advance_fades` | bake half, by the message's frame number | bake thread | Phase 2 (4) and (8) |

### Phase 1, amended

- **What, added.** (a) The dig base is read on the queue's own rig, `dig_timeline`, drained and
  waiting, sequences A and B, both adapters; `gpu_bake` is not read for it. (b) **The rig's
  stepper waits on the device once a frame** (as `support::queue`'s rig does, standing for the
  fly-through's present), as a mode the dig readings use; a gate test that takes the mode has
  its binary run in this phase. Without it the rig's queue never drains and "drained" is a
  waiting queue (`docs/backlog.md`, "Plan 0098's leftovers"). (c) The dig frame's step (D+0) is
  read with **at least eight runs a sequence and adapter**, with its `boxes` and `digs` laps
  and the new `render` and `movable` sums. (d) `render` counts the candidate tables' writes
  (`JobLoop::refresh_tables`), the parameters' write and a dig's block writes with the rest.
- **Files touched, added:** `apps/flythrough/tests/support/defects.rs`,
  `apps/flythrough/tests/gpu_outdoor/dig_line.rs`.
- **Done when, added.** (5) With the stepper's mode on, the rig's `drained` state reaches 0
  chunks in view waiting on the RTX and on the RENOIR, the frame it does so logged. **A stop:**
  if it does not, dev reports the read-backs a frame to the architect before the dig base is
  read. (6) The readings log holds, per adapter and sequence, eight runs' D+0 step in ms with
  the mean and the range, the `boxes` and `digs` laps, and ring 1's landing in frames for
  every dig on the drained and on the waiting queue; a landing that is not the 1 or 2 frames
  of `docs/nfr/04-edits.md` is reported to the architect before Phase 2. **No later bar or
  comparison is held against a dig-frame figure with fewer than eight alternated runs a
  side.** (4) reads `--binaries gpu_queue gpu_queue_reach gpu_bake`.

### Phase 2, amended

- **What, added.** The message's list above is not the whole of what the step reads today.
  1. **The message also carries** the edit boxes since the last one, the bake's sky bits and
     key light, the settings the frame sets each step (the column wait, the key step, the
     landing fade, `dig hold`, the far-normal band, the knobs) and the stream workers'
     prepared masks. `QueueView::resident` is a borrowed closure; the bake half takes the
     streamer's residency as owned data.
  2. **The shadow answers every mirror read the bake half makes**, not membership alone:
     held full, LOD-only, **waiting to upload** (`is_pending`, `pending_chunks`, and with them
     `column_final`), **each chunk column's lowest top** (`column_lowest`, the record by rule),
     and per chunk **whether it holds an exposed emitter and whether its light range is
     current** (`emitters_near`, `lights_ready_near`). A light range's change is not in the
     mirror's change list (`take_changes` notes membership and uploads); the render half adds
     the chunks whose range or dirty mark changed, and the columns a change touched with their
     new lowest top. The rule is answered on the snapshot; a later change relists the zero
     record as it does today.
  3. **The candidate tables stay with the render half**: a pure function of the mirror's light
     ranges, written for the slots a prepared submit names as it goes out and again when the
     ranges move. It is not a rule of the queue's.
  4. **A first block's landing word is the frame its list is applied in, plus one.** The shader
     fades a block's source by that word (Plan 0098 Phase 6b), so the word of the frame the
     bake half made the list in would start the fade part-way. The list marks the word; the
     render half writes it at apply.
  5. **A submit's stamp is the newest message the bake half had read when it planned it**, not
     the frame the render half issued it in: a change in any later message inside the job's
     reach stales it.
  6. **`Fade` says it is a dig's** (a flag set where `refade` takes a dug block), and
     `open_digs` reads the flag in place of `frames == DIG_FADE`.
  7. **An edit's writes are never carried.** The step that takes a box puts the sky-stale
     marks and every held and provisional block in its list whole, whatever their size, and
     they count against the frame's cap first; landings wait for the room that is left.
- **Files touched, added:** `crates/svet-render/src/gpu_world/` (change notes and snapshot
  reads only; nothing of `sync` moves), `crates/svet-render/src/bake/held.rs`,
  `apps/flythrough/tests/{gpu_queue,gpu_queue_reach}.rs`. New code goes to new files:
  `bake/queue/mod.rs` and `bake/job_loop.rs` are near the size gate.
- **Done when, changed and added.** (2) gains a case: a submit planned from the frame 4
  message and issued in frame 6 is staled by a change in the frame 5 message. (3) reads, over
  a sequence that also holds light ranges written, moved and dropped, uploads queued and done,
  and a dig that lowers a column's top: the shadow's held-full, LOD-only, **pending,
  column-final, lowest-top, emitters-near** and lights-ready answers equal the mirror's after
  every message. (7) `a_dig_is_served_whole_in_the_edits_step` (CPU, under 2 s; the bake half
  against a recording applier): a box in a chunk that holds a block, with landings over the cap
  already waiting: the list of the step that took the box holds, for each touched chunk, its
  sky-stale marks and its held block's words with its table word, all applied in that frame;
  that step's prepared submit plans ring 1's rounds ahead of every other job's; the landings
  are the ones carried. (8) `ring_two_waits_out_a_dig_fade_whose_steps_are_skipped` (CPU, under
  1 s): ring 1 of a split chunk placed at frame 10, bake steps at frames 11, 13 and 14: ring 2
  opens at neither 11 nor 13 and opens at 14; a chunk under a 16-frame landing fade opens its
  dig's bricks in the first step. (9) `a_first_blocks_landing_word_is_the_frame_it_is_applied_in`
  (CPU, under 1 s): a first block whose list was made at frame 5 and applied at frame 7 holds
  8 at its landing word; a refinement applied at frame 9 keeps 8. (10) The dig's tests pass
  unchanged in lockstep: `--binaries gpu_queue gpu_queue_reach gpu_bake gpu_sun`, and the soak
  image test `flicker::a_fresh_hole_holds_its_value_until_its_light_lands` by name, once.

### Phase 3, amended

- **What, changed.** The dig's box is in the message since Phase 2. **A step in progress admits
  no further candidate once a box waits in the inbox**, and the next step takes the box first;
  this holds under either answer to question 6. Whether the edit's frame then waits for that
  step's list is question 6; dev builds (a) or (b) as answered and **does not start the dig
  clause before the answer**. Every dig test that reads an image or a record before the drain
  stays in lockstep (the plan's rule), so the thread's dig is guarded by (5) alone.
- **Files touched, changed:** `apps/flythrough/tests/gpu_queue.rs` in place of the
  `crates/svet-render/tests/` path; `apps/flythrough/tests/gpu_outdoor/dig_line.rs` and
  `support/defects.rs` (the mode as the rig's parameter).
- **Done when, changed.** (5) is three parts. (5a) `a_waiting_box_ends_the_opening` (CPU, under
  2 s): 32 candidates ready and a test hook that posts a box after the second listing: the
  step lists at most 3, and the next list is the dig's (its held block and table word). (5b)
  On `gpu_queue`'s dig rig with the mode as a parameter (lavapipe, at the wall the tree's test
  has), thread on: ring 1's block is applied within Phase 1's base frame count plus 2. (5c)
  **Under answer (a):** on the same rig, the pool read back after the edit's own frame holds
  the held block for every touched brick (no frame between the edit and the held record).
  **Under answer (b):** the frames from the edit to the held record's apply are logged, at most
  2 on that rig. (8) gains, on the forest in regime B with the thread on, both adapters,
  sequences A and B, eight runs alternated with lockstep: the frames from a dig to its held
  record applied and to ring 1 applied, the dig frame's wall against Phase 1's, and the share
  of dig fades that moved in all 4 of their frames. (7)'s bars are read on runs with no edit.
  `--binaries gpu_queue gpu_queue_reach gpu_bake gpu_sun`.

## Data shapes

```rust
// illustrative; the tree's names win
struct FrameMessage {            // render thread -> bake thread, one a frame, coalesced
    frame: u64,
    view: View,                  // eye, forward, the slice's interval
    changes: Vec<(ChunkCoord, Held)>,   // Held: Full { lights_ready }, Lod, Gone
    window: ChunkExtent,
    read_backs: Vec<ReadBack>,   // mapped, each with the frame its submit went out in
}
struct WriteList {               // bake thread -> render thread, applied whole in one frame
    step: u64,
    bytes: usize,                // at most the frame's cap, by construction
    writes: Vec<Write>,          // Pool { at, words } | Table { .. } | Summary { .. } | Window(..)
    submit: Option<PreparedSubmit>,     // planned rounds and the entry table's bytes
}
enum BakeThreading { Lockstep, Thread }
```

Memory, **guesses until Phase 6 reads them**: the shadow membership about 24 bytes a resident
chunk (about 0.5 MB at 20 000 chunks, arithmetic); the outbox capped at 8 MiB of write lists;
the frame's apply cap 1 MiB (sized from Phase 1's bytes a step, which is not read yet). The
store's GPU bytes and its CPU copy of the blocks do not change.

## Budgets

| `docs/nfr.md` figure | today (reading) | this plan's line | standing | how the close measures it |
|---|---|---|---|---|
| The bake queue's step on the render thread (inside the 1.0 ms streaming line of section 2) | 28.0 / 51.1 ms p50/p99 at the window; 20.7 / 55.0 headless 1080p, loaded | the render thread's bake share **p50 at most 1.0 ms, p99 at most 4.0 ms**, regime B, both adapters | Plan 0096's proposed line, carried. **Whether the copies fit it is a guess** until Phase 1's split | the `bake queue step:` line's `render` share, the forest fresh start, thread on, quiet |
| The Floor frame, 16.6 ms, while in-view work waits | wall p50 28.3 ms at the window's size | **wall p50 at most 16.6 ms** on the RTX at Floor 1080p | an existing line (Plan 0096's Budgets row), not met today | the run's `frames:` line split at the in-view landing |
| In-view drain (Plan 0096's new line) | RTX 16.1 s hills, 22.3 s forest at the window; RENOIR 35.2 s hills at 3.79 times its pixels | **RTX Floor at most 10 s, RENOIR Portable at most 30 s** | Plan 0096's estimates, carried. **A guess**: 9.6 s is arithmetic from the frame rate, and needs Phase 4 | `drain_s` and the in-view waiting count at 0, both scenes |
| Whole-residency drain | RTX 27.5 s hills at the window | Plan 0096's lines (RTX 60 s, RENOIR 180 s), re-read | carried, no new claim | the same rig to depth 0 |
| The submits' drawn share of the drain slice | 0.94 to 0.99 at about 30 fps; 0.60 to 0.69 at the per-second rig | **at least 0.85** at the frame rate the thread gives | Plan 0096's bar, re-read where it is harder | the `bake queue slice:` line |
| An edit's light (section 4) | read in Phase 1 | the dig's near ring within the base's frames **plus 2**; the dig op's wall p99 within the base plus one chunk's listing | **a guess** until Phase 1 reads the base | `gpu_bake`'s dug-emitter rig; the fly-through's dig while in regime B |
| Memory (section 3) | none | outbox at most 8 MiB, shadow about 0.5 MB | **guesses** | peak counters on the queue's line |
| The floor's CPU (4 cores, 8 threads) | not read; no machine | no line; the `taskset` arm's wall and share beside the unpinned run | a stand-in on the dev box, labelled, not the floor | Phase 1, 3 and 4's pinned runs |
| Images | | every drained block bit-identical across lockstep, the thread and a one-shot bake; no golden moves | a property, asserted | `gpu_queue`, `gpu_golden` at the tip |
| Section 7, the gate | per Plan 0096's close | about 10 s of CPU tests and under 15 s of lavapipe in the gate; one soak-born test | **a guess** | the tip's gate wall |

## Risks & open questions

**Questions the owner answers before the go** (the first option is the recommendation):

**Answered (owner, 2026-11-22): (a) in all five.** Question 2 in his words: "I agree with
keeping fade length"; the other four: "agreed on other recommended".

1. **What runs in which mode.** (a) The thread in a window and in the perf rigs; lockstep for
   the pre-roll, the tests, the goldens, the pop counter and any headless run that asserts
   bytes before the drain. (b) The thread everywhere, and such assertions wait for the drain.
   (c) Lockstep stays the default and the thread is opt-in until a later plan.
2. **The fade when the bake thread is late.** This one is image stability, so it takes no
   answer by default. (a) The fade keeps its length in frames and takes one larger step when
   a bake step is late; never backwards; judged at the window. (b) The fade moves into the
   shader (two blocks and a factor; more pool words and a read in the light pass, its own
   ADR and plan). (c) The fades stay on the render thread (4.3 to 7.5 ms of the step stays,
   and the 1.0 ms line is missed by decision).
3. **Does this plan own the drain's 10 s?** (a) Yes: Phase 4 is in, and a miss after its
   levers is disclosed. (b) No: this plan takes the frame rate only and the drain gets its
   own plan after Plan 0091's scopes. (c) Raise the drain slice instead (it costs the frame
   the 16.6 ms line).
4. **The floor's CPU.** (a) A `taskset` run on the dev box, labelled a stand-in. (b) No
   reading, a projection only. (c) Hold the line until a 4-core machine is on hand.
5. **The window.** (a) As Phase 5 is written: the RTX's forest both modes, the hills, the
   digs, one RENOIR run. (b) RTX only.

6. **Open (owner; added by the re-read of 2026-11-24). The dig's held record with the bake
   thread on.** Image stability, so it takes no answer by default. Today the edit's step
   writes the held record over the dug bricks before that frame's light pass reads the store:
   no frame shows the hole without it, the picture accepted at Plan 0098's window 98e. As
   Phase 3 was written, the box goes to the bake thread in that frame's message and the held
   record comes back in a later frame's list; until then the hole's new faces read the old
   block, which holds no record for them (the picture before `dig hold`). How many frames is
   not read. Arithmetic: a regime B step is 26 to 43 ms on its thread, 2 to 3 frames at
   70 fps, and the dig's own work reads `boxes` 2.2 to 2.4 ms and `digs` under 0.5 on the RTX
   (Plan 0098's log); ring 1's landing is later by about twice that, the box in and the
   read-back in.
   (a) **The edit's frame waits for the dig** (recommended). A frame that carries a box does
   not return from the bake call until the bake side has served it, and applies that list in
   the same frame, as lockstep does. The wait is one listing in progress plus the dig's own
   work: a guess of 3 to 6 ms on the RTX, on dig frames only, against today's 14.5 to 25.9 ms
   dig-frame step. ADR-0083 already gives lockstep "a dig's ring where the consumer asks for it
   at once"; this makes it what the thread does by default, and "the render thread never
   waits" gains that one exception (an Outcome line on the ADR).
   (b) **As written:** the held record arrives when the bake thread reaches the box, the owner
   judges the frames without it at Phase 5 (c), and (a) is the fix pass if he sees it. No
   hitch; the picture he rejected at windows 98c and 98d may be back for a few frames a dig.
   (c) **Lockstep while a dig waits or bakes:** the whole step returns to the render thread
   for those frames (about 28 ms each in regime B). The simplest; a dig in a deep queue
   drops the frame rate for as long as its rings take.

**Risks.**

- **Determinism.** `svet-world` is read through its existing locks and gains nothing. The
  store's bytes are mode-independent and asserted so. What is lost is the frame a record
  lands in: any test that reads the undrained image with the thread on is flaky by
  construction, and the plan's rule (lockstep for such tests) is one a later plan can break
  silently. `scripts/check-test-tiers.mjs` is the place for a mechanical guard if it
  happens; not built here.
- **The copies may not fit 1.0 ms.** Phase 1's stop exists for this. The byte cap then holds
  the frame and delays landings; lever (d) is the answer, and it is the larger half of
  ADR-0022's work for this store.
- **One thread may not feed the slice** (36 to 75 % of the 70 fps grant by the arithmetic
  above). Phase 4's levers are ordered by reading, and its bar may be missed and disclosed.
- **Lock contention with edits.** The bake thread's listing holds chunk read locks while a
  dig wants a write lock. Bounded by one chunk's listing; read in Phases 1 and 3. If the
  edit's p99 moves by more than that, the listing reads from the workers' prepared masks
  only and falls back to the world for a neighbour without them, under a try-lock.
- **The dig's ring** is a frame or two later with the thread on. If the owner sees it at
  (c), the ring's re-bake stays a lockstep call on the main thread (it is bounded by the
  ring).
- **Oversubscription on few cores.** The stream workers default to half the logical cores;
  the listing's fan-out and the bake thread add to them. The `taskset` arm reads it; the
  floor itself is not read.
- **The store's window lags the mirror's by a step.** Safe only if the light pass reads the
  table by the store's own window (Phase 2 checks and fixes it). A wrong record for one
  frame would be a pop.
- **Shutdown and panics.** A bake thread that outlives the device, or dies silently and
  leaves the queue frozen, is a hang in a test and a dark world in a window. Phase 3 tests
  both.
- **GPU features:** none new. Nothing branches on the backend.
- **Plan 0098's fade test** bounds a queue step's move by the step's share of the change;
  Phase 2 restates it as the elapsed frames' share, which is the same in lockstep. If 0098
  lands a different bound, the tree's wins and the difference is noted.

## What this plan does NOT do

- **Slot words packed on the stream workers** (ADR-0056, the sync's `alloc`): its own plan,
  drafted at Plan 0096's close.
- **The mirror's sync.** ADR-0017's rule for the mirror stands; nothing of `sync` moves.
- **The fade in the shader**, unless the owner picks question 2 (b), which is then its own
  ADR and plan.
- **The drain slice's value, the tiers, the reach rule, the order's score** (ADR-0080 and
  ADR-0048): unchanged.
- **The RENOIR's hit pass and FSR 2's upscale line**: the backlog's beam prepass and the
  profile lines' own.
- **A profiler.** Plan 0091 comes after and wraps the bake thread and the render thread's
  apply at the lines Phase 6 pins; this plan's readout is the queue's line.
- **A general job system.** One thread for one owner; the listing's pool is the queue's.
- **Windows readings** (owner, 2026-11-15: once in a while, not per plan).

## Implementation log

> Written by `dev`: one row per phase as its commit lands, then the close block. **The phases
> above are the contract. This section records what happened.** Observations, never conclusions:
> no pass list, no self-assessment. Deviations and unmet done-whens are **always** disclosed.
> Keep it shorter than `## Implementation phases`. Readings, gate counts, attempts, sweeps and
> resume notes go to the plan's readings log, `docs/plans/logs/0099.md` (started from
> `templates/readings-log.md`), and this section links it once it exists.

| phase | owner | state | commit |
|---|---|---|---|
| 1: the step's split on the line, the base readings | dev | not started | |
| 2: the queue in two halves, in lockstep | dev | not started | |
| 3: the bake half on its own thread | dev | not started | |
| 4: the bake thread keeps the slice fed | dev | not started | |
| 5: the owner's window | human | not started | |
| 6: the lines, the defaults and the documents | dev | not started | |

### Notes

- (architect, at the draft) Every path and identifier in this plan is as of `plan-0096`'s tip
  `33003841`, before Plan 0098's merge and before Plan 0087 renames the crates, the
  environment variables and the identifiers. The mechanisms are the contract; the tree's
  names win.
- (architect, at the draft) The backlog entry "The bake step off the main thread" exists on
  `plan-0096` only; at that plan's merge it is marked taken by this plan.

### Close triggers

_(Facts for the architect. No recommendations, and no suggested version bump.)_

- **What shipped:**
- **Public API changed:**
- **Budget readings:**
- **Gate at the tip:**
- **Outstanding `human` phases:**

## Followups

- (empty at draft time)
