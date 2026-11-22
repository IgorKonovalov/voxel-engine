# 0096: Performance: the bake queue's drain (a record by rule, a tier by size on screen, a drain slice), the step off the depth, the slow adapter's light pass, and the sync's `alloc` and eviction scan

> **Status:** in-progress (dev, 2026-11-21). Approved 2026-11-20 (owner go; starts after Plan 0095 closes and merges). Owner picks, all as recommended: the drain slice 4.0 ms Floor / 6.0 ms Portable while in-view work waits, 1.0 once landed; a far chunk's 256-sample record is final until the camera brings it nearer; if the RENOIR forest stays over 4.0 ms the Portable profile owns the levers it picks and the Floor line stays with the miss disclosed; the window includes the 60 s cave glide. **Amended 2026-11-20 (owner)**, at Plan 0095's window W1: the far tiers' reach rule moves into this plan (ADR-0080 rule 4; a chunk whose reach crosses the full radius bakes its mid or far tier from the chunks resident, the near tier keeps the full reach), Phase 1's census counts reach-bound chunks by the exact test and names the no-record bricks if Plan 0095 Phase 6 cannot, Phase 6 (c) reads far pixels on the baked key or the column map's horizon, the landed state counts a LOD-ring face served by the horizon, and 0095's extra bake rays are conditional on its window W2. **Amended 2026-11-22 (architect, owner confirm owed)**, overnight at the Phase 6 checkpoint `2665f41f`: Phase 4's bar 2 is read at the default drain slice and the F2 rig's miss is a disclosed reading with its cause (the opening's candidate cap, routed to the Phase 5 bar 3 decision); Phase 6's lever (b) leaves bar 1 and the window (a no-op on cube faces, bounded at 0.16 ms by Phase 1's `--sky-rays 0` arm), the setting stays, and bar 1 becomes the key cap's test; Phase 6's paths fixed (`crates/vxe-render/tests/gpu_sun/key.rs`, `cli/outdoor.rs`).
> **Created:** 2026-11-20
> **Milestone:** M10 (the outdoor scenes are where the queue never drains) and cross-cutting
> like [Plan 0067](done/0067-the-frames-cpu-a-cheap-queue-step-a-bake-budget-by-time-and-no-black-for-missing-light.md):
> it serves `docs/nfr.md` section 2's streaming, bake-slice and lighting lines and section 1's
> slow adapter. The owner's "address performance next" (2026-11-11), placed after Plans 0094 and
> 0095 (2026-11-20).
> **Related ADRs:** [ADR-0080](../adrs/0080-the-live-queue-bakes-what-the-image-reads-a-record-by-rule-a-tier-by-size-on-screen-and-a-drain-slice.md)
> (proposed with this plan: the queue bakes what the image reads; amended 2026-11-20 with rule
> 4, the reach rule by tier) and ADR-0079 (on branch `plan-0095` until Plan 0095 merges: the
> far's key light is the column map's horizon, refined by the record this plan lands). It rests on
> [ADR-0046](../adrs/0046-static-emissive-light-is-baked-per-chunk-when-a-chunk-loads-or-changes.md)
> (the store and its queue), [ADR-0048](../adrs/0048-the-live-queue-orders-by-the-view-and-withdraws-the-work-a-flight-has-passed.md)
> (the order and the withdrawal), [ADR-0052](../adrs/0052-the-live-queues-bake-budget-is-time-per-second-not-a-slice-per-frame.md)
> (the budget's unit), [ADR-0053](../adrs/0053-a-light-summary-per-block-and-a-two-level-parent-table-light-a-brick-with-no-value.md)
> (the fallback chain, the coarse answer rule 6 names), [ADR-0070](../adrs/0070-sky-light-is-baked-visibility-coloured-at-read-with-a-long-history-ray-where-stale.md)
> (the sky channel and the column-map rule) and
> [ADR-0076](../adrs/0076-light-stability-rules-world-history-per-source-a-fade-per-transition-and-a-ratcheted-pop-counter.md)
> rule 6 (proposed on Plan 0094's branch, 2026-11-20: the undrained image is the judged image;
> the drain time is this plan's budget). The sync phases rest on
> [ADR-0017](../adrs/0017-mirror-cpu-work-bounded-in-the-frame-loop-narrowing-on-stream-workers.md).

## TL;DR

The forest's bake queue held 12 752 to 16 150 jobs through the owner's window 94a and drained
at 9 to 38 chunks a second, so the precise light lands 7 to 30 minutes after the load; the owner:
"if we have to wait for a minute for light to settle, something is wrong with the light and we
need approximate things, it is not acceptable for user". This plan makes **the time the queue
takes to land what is in view a budget in `docs/nfr.md`** and meets it three ways (ADR-0080):
a chunk the rules can answer gets its record **without a GPU job** (no exposed face, or buried
under the column map with no emitter in reach: a zero record), every other chunk bakes at **a
sample tier set by its size on screen** (full where a voxel is over the far-normal band, the
coarse count where it is under it, promoted when the camera comes near, as one rule 3 fade;
**a mid or far chunk whose light reach crosses the streamer's full radius bakes from the chunks
resident**, since under the default reach rule nothing past about 72 m ever bakes and the far
tier would be empty), and the bake's slice **rises while in-view work waits** and falls to the
line when it has landed. Beside it: the queue's step on the main thread (6.3 ms p50 at 1080p while the queue is
deep) gets its cost off the queue's depth; the RADV RENOIR's outdoor light pass (8.0 and 12.0 ms
against the 4.0 ms line) is read lever by lever; and the sync's `alloc` stage and the light
placement's eviction scan (up to 1.2 s a sync in a windowed glide) take the backlog's two sync
entries. Phase 1 is readings only, on both adapters, **on the undrained image** (rule 6). The
first thing anyone sees: the `bake queue:` line and the HUD's queue row say how many in-view
chunks wait, by tier, and how many seconds the drain is projected to take.

## Context & problem

Every figure is a reading unless marked; "RTX" is the NVIDIA GeForce RTX 3080 Laptop GPU,
"RENOIR" the AMD Radeon Graphics (RADV RENOIR), the slow adapter of `docs/nfr.md` section 1.

- **The queue never drains outdoors.** Window 94a (Plan 0094's readings log, Phase 7, release
  `37783825`): the forest at 15:00 read a queue depth of 12 752 (run 2) and 14 499 with 4 419
  in-view chunks waiting, 782 of them coarse (run 4); run 6's `bake queue:` lines read depth
  16 099 to 16 150 over 5 s while `baked` rose by 9 to 38 chunks a second; run 2's first
  seconds read +94 to +190 a second (the coarse jobs). Full residency is 17 077 chunks at
  29.9 s, so nearly every resident chunk is listed. Plan 0086's close (`docs/nfr/02-frame-budget.md`,
  2026-11-19) read the hills' queue over 14 000 deep in every 900-frame run and the pre-roll
  on the RTX's hills lengthened from 7.3 s to 14.4 s with the sky channel; Plan 0094's and the
  wobble spike's rigs never drained the forest in 30 000 frames. The wobble spike
  (`docs/plans/logs/wobble-spike.md` on branch `spike-motion-wobble`) attributes two thirds of
  the far pixels still differing from their settled value at a stop to faces waiting for a
  record, the other third to the screen-space key and sky means that Plan 0095 takes.
- **The reach rule caps the baked far at about 72 m** (amended 2026-11-20; Plan 0095's
  readings log, "The idle queue after W1", `b7ad63f7` on branch `plan-0095`): a chunk bakes only
  when every chunk in its light reach is mirrored full (`in_reach`, offsets up to 5.39 chunks at
  `DEFAULT_LIGHT_RADIUS` 4), and the full radius is 16 chunks (`lod.r_full`; 12 in the Portable
  profile), so under `ReachMode::Full` no chunk past 11.18 chunks (about 72 m; about 42 m in
  the Portable profile) ever bakes and the LOD ring (16 to 24 chunks) never does. The hills at
  noon drained to 0 jobs at 26 s with 2 964 of 3 561 in-view chunks waiting reach-bound; the
  exact test (`reach_crosses`) marks 11 814 of the full sphere's chunks, the HUD's `len + lr >
  r_full` 9 924 (the 1 890 between lie at 10.68 to 12.0 chunks; Plan 0095 Phase 6 fixes the
  count). The last black dump there also found 461 px over 24 chunks within 10 chunks whose
  chunk holds a block and whose hit brick has no record, at every dump from frame 60 on,
  unexplained. The far band of this plan's tiers begins at about 38.5 m at 1080p (a voxel under
  2 px), so nearly every far-band chunk is past the reach line: without a reach rule the far
  tier has nothing to bake. `ReachMode::Partial` (`--reach partial`, Plan 0067 Phase 5) bakes
  such a chunk from the chunks resident and re-queues it when its reach arrives; the owner kept
  `Full` globally at Plan 0067's window ("not much difference really").
- **The arithmetic of the drain** (from Plan 0067's readings, `docs/plans/logs/0067.md`): a
  full job (4096 samples a brick direction, `QUEUE_BRICK_SAMPLES`) draws about 1.2e6 samples a
  chunk and a coarse one (256, `DEFAULT_COARSE_SAMPLES`) about 7.4e4; the RTX draws 5.1 ns a
  sample, the RENOIR 47.3. So a full chunk costs about 6.5 ms of RTX GPU and 60 ms of RENOIR
  GPU, a coarse one 0.4 and 3.5 ms (the sky channel since Plan 0086 adds 256 rays a brick
  direction, longer ones: not read per job). Under the default `frame:1.5` the queue draws
  about 40 ms of bake a wall second on the RTX and 30 on the RENOIR (`docs/nfr.md` section 2),
  so 16 000 full chunks are 104 s of RTX GPU and about 43 minutes at the slice; on the RENOIR
  960 s of GPU and about nine hours. No slice makes the RENOIR bake every resident chunk at
  the full count. The 94a drain rate (9 to 38 a second) is this mix of full and coarse jobs.
- **Rule 6 says what the image shows meanwhile** (ADR-0076, proposed, amended on Plan 0094's
  branch at `38460762`): a face whose record has not landed shows a coarse stable answer (the
  chain's neighbour or parent summary, ADR-0053; the no-ray prior Plan 0094 builds), and the
  record arrives as one fade. The rule leaves the drain's time to this plan as a budget. The
  budget is empty today: `docs/nfr.md` has a 1.0 ms bake-slice line the default `frame:1.5`
  already exceeds (the owner took the lines as proposed at Plan 0069's close) and no line for
  how long a chunk in view may wait.
- **The step on the main thread.** Plan 0086 Phase 24c (`docs/plans/logs/0086.md`, RTX,
  1080p, the tunnel's stairs at rest, the queue 14 103 to 15 444 deep): the queue's step p50
  6.2 to 6.6 ms, frame CPU p50 7.4 to 7.8, so the wall is the step and the GPU frame (4.5 ms)
  in series: 13.4 ms at 1080p, 16.6 at 1440p, where the GPU is within every Floor line. Plan
  0067 made a still queue's step cost 0.075 ms and a flight's regime B step 9 to 13 ms (nearly
  every step opening a job; `docs/nfr.md` section 2); its stage laps (`STEP_STAGES`,
  `crates/vxe-render/src/bake/stages.rs`) read the opening's `candidates`, `list` and `sort` as
  the large parts, each walking the candidate pool. Phase 5's reading (0086) put the step with
  the sky channel at 5.7 to 7.6 ms p50 while baking. The backlog's "CPU settle outdoors"
  entry read the same: 7.9 to 15.0 ms p50 for the 7 to 10 s after every load on a world with
  no emitter in reach whose store ends all zeros (3 285 chunks baked by 13.7 s), 0.05 ms once
  drained. `bake/queue/mod.rs` is at the 1200-line cap.
- **The slow adapter's light pass at rest** (Plan 0086's close, `docs/nfr/02-frame-budget.md`,
  release, headless 1080p at the defaults, the queue drained by `--bake-budget frame:40`,
  loaded): RENOIR light 8.025 ms on the hills at noon and 12.034 in the forest at 15:00
  against the 4.0 ms lighting line (10.5 and 12.5 before ADR-0070), inside frames of 53.3 and
  43.7 ms whose largest pass is the hit (21.9 and 10.7 ms); at the tunnel's stairs 3.88 + 0.34.
  The RTX reads 1.570 and 2.529, within the line. Plan 0032 Phase 6 read a sky ray at about
  4.8 ms on the RENOIR and 0.25 on the RTX; what the RENOIR pays at rest now is the key light's
  disc ray and the baked read. The review of Plan 0032's window
  (`docs/reviews/2026-11-17-plan-0032-window.md`, E2) named two levers unbuilt: a cap on the
  key ray's length and no sky ray from faces that look down.
- **The sync's two entries** (`docs/backlog.md` until this plan; Plan 0074's close, RTX host on
  `hpet`): `update + poll + sync` p99 18.0 to 23.7 ms in the tester cave and 20.4 to 22.7 in
  the castle against Plan 0067's 16.6 ms; the largest stage left is `alloc` (p99 7.3 to 8.8
  ms), whose profile read `fill_slot` 3.15 %, `BufferViewMut::drop` 3.12 %, `upload_brick`
  2.52 % of the main thread in a glide. The forest at leaf fill 128 read a sync p99 of 11.0 ms
  (`alloc` 5.3) and `update + poll + sync` max 150 ms (Plan 0032 Phase 9). In the owner's
  windowed `--glide 300` cave flight every second's slowest sync from about 4 s in is `light
  place`, up to 1 186 ms over 3 697 evictions and 99.9 M scan steps (r 0.997 against
  `light_scan_steps`, 11 to 12 ns a step), the free ranges growing to about 6 000; no headless
  glide (19 runs) evicted a range. The scan is `write_chunk_lights`
  (`crates/vxe-render/src/gpu_world/lights.rs`): on a failed first fit it walks every placed
  chunk for the farthest victim, once per eviction. Plan 0074's Phase 2 draft, a first-fit
  index (`fit.rs`), is kept untracked in the main checkout at `live/plan-0074/drafts/`.
- **The smaller backlog items this plan takes:** the queue step's remaining `filter` cost on a
  moving camera (about 0.074 ms p50; `gone`, `drop_reach`, `touched` each walking the mirror's
  changes), and the RTX's submits drawing 5.3 of a granted 8 ms slice under a per-second budget
  (Plan 0067's followup F2: the rounds planned against the slice, or `IN_FLIGHT` 2).

## Decision

**The queue bakes what the image reads, and the drain of what is in view is a budget**
(ADR-0080). Three mechanisms, in the order they pay: (1) a chunk the rules answer takes no GPU
job: a chunk with no exposed face holds nothing to light and is marked baked-empty; a chunk
whose reach box (the chunk dilated by the light radius, `DEFAULT_LIGHT_RADIUS` 4) lies under the
column map's lowest top over that footprint and holds no emitter in reach gets a zero record
written on the CPU with the baked bits set, since ADR-0070's rule already counts every ray
ending under the column top as not sky and no emitter can reach it. (2) Every other chunk bakes
at a tier set by its size on screen through the far-normal band (`far_normal` `[lo, hi]` pixels
a voxel, `[2, 6]` at the defaults): full (4096) where its nearest voxel projects over `hi`, a mid
count (1024) between, and the coarse count (256) under `lo`; a chunk that moves into a nearer
band as the camera moves is re-listed as a refinement and lands as one `FADE` (rule 3); one
that moves out is withdrawn like a passed job (ADR-0048) if not yet submitted. The far tier is
final, not a first pass: the queue reaches a landed state. **The reach rule is by tier**
(ADR-0080 rule 4, amended 2026-11-20): a near-band chunk waits for its full reach (ADR-0046); a
mid- or far-band chunk whose reach crosses the full radius (`reach_crosses`) bakes from the
chunks resident once nothing in its reach waits to upload, the LOD-only chunks beyond the sphere
casting from their mips, flagged partial in the queue's maps as `ReachMode::Partial` does
today; the partial record is refined by the band promotion, not by the reach filling (the
re-queue on arrival is off for mid and far records), and the dig's ring keeps the near count
with the re-queue. A LOD-only chunk holds no record; its faces read the column map (ADR-0079's
horizon, ADR-0070's sky rule) for good, and the landed state counts them as landed. (3) The slice (ADR-0052's unit) has
two values: a drain value while any in-view chunk waits for its tier's record and the line's
value once none does; both are settings rows per profile, judged in the owner's window.

Beside the queue: the step's opening cost is made proportional to the jobs it opens and the
chunks whose score changed, not to the queue's depth; the RENOIR's light pass is read lever by
lever behind settings (the key ray's cap, no sky ray from faces looking down, far pixels on the
baked key with no ray and no pair update once Plan 0095 lands), each lever a cheap mode kept as
a setting whether or not the Portable profile picks it; and the sync takes Plan 0074's Phase 2
index for the eviction scan and the `alloc` fix its profile names.

We rejected, for the reach (ADR-0080 Alternatives E to G), **a wider full residency** (cube-law
memory on about 0.87 GB live, the LOD ring still unbaked, the Portable profile cannot pay it),
**`--reach partial` as the global default** (the owner kept `Full` at Plan 0067's window; it
changes the near records that ADR-0046's identity test and the dig's ring stand on, for a gain
only the far needs) and **refining a partial far record when its reach fills** (one more job and
fade per approached chunk at about 72 m, where the mid band's re-bake at about 38 m lands at
the full reach anyway). We rejected **a larger slice alone** (the arithmetic above: the RENOIR cannot bake the
residency at the full count in any time the owner would call settling, and the RTX would spend
minutes), **a progressive queue that refines every chunk to the full count when idle** (no
landed state, so rule 6's "one fade when it lands" has no landing and the step never rests),
**a per-brick skip inside the GPU job** (the step still lists, scores and submits the chunk;
the CPU cost this plan chases stays), and **baking on the stream workers' CPUs** (ADR-0046's
rejected alternative; the GPU bake at 5 ns a sample is the fast path). A second GPU queue was
rejected in ADR-0052 (wgpu has one).

## Architecture diagram

```mermaid
flowchart LR
    subgraph vxe-render
        L[list: chunks in reach] --> R{rules}
        R -- no face --> E[baked-empty mark]
        R -- buried, no emitter --> Z[zero record on the CPU]
        R -- else --> T[tier by size on screen]
        T -- near: full reach / mid, far: resident reach --> Q[queue: score, tier, class]
        Q --> S[slice: drain while in view waits, line when landed]
        S --> J[GPU job at the tier's samples]
        J --> ST[store: record + FADE]
        Z --> ST
        C[camera moved] --> T
        C --> W[withdraw passed / demoted]
        W --> Q
    end
    subgraph apps/flythrough
        H[bake queue: line, HUD row: in view waiting by tier, drain s]
    end
    Q --> H
```

## Implementation phases

Each phase ships as its own commit. `dev` implements all phases in one session with no review
between phases; the architect reviews once at the end, in a fresh session. Phases 3, 4 and 6 are
**exploration** (lane rule 2b): they name readings, not image tests, and the owner's window
(Phase 8) decides their values; Phase 9 writes the bars against the picks. Every reading is
taken on the **undrained image as well as the drained one**, with the queue's depth and the
in-view waiting count beside it (ADR-0076 rule 6), on the RTX (Floor profile, 1080p, FSR 2 at
1.5) and the RENOIR (`--adapter AMD`, Portable profile, 720p), release, under the exclusive perf
lock with the 1-minute load logged.

**Starts after Plan 0095 (the motion wobble) closes and merges**, which follows Plan 0094: 0094
lands rule 6's coarse prior and moves the default-off arms out of the default pipeline (the
RENOIR light-pass readings must be taken on that pipeline), and 0095 (amended 2026-11-20 at its
window W1) gives the far's key light an instant estimate from the column map's horizon that the
baked record refines where it lands (the owner's pick), so far pixels read a baked key or the
horizon with no ray; its key-visibility bake adds about 6 % to every job's rays (its guess)
**only if its window W2 keeps the record as the far's refinement**; if W2 holds the horizon for
good, 0095's key bake defaults off and the jobs' rays are unchanged. Either way the drain's cost
and the RENOIR's light pass are re-read here. Plan 0095 Phase 6 also lands the reach-bound
count's fix (the exact extent, 5.39 chunks at light radius 4) that Phase 1's census reads, and
names or hands over the 461 no-record pixels. If 0095 is still open when the lane is free, Phase
1 may run on 0094's tip with the +6 % carried as a projection and re-read at Phase 9.

### Phase 1: Readings on the undrained image, and the drain readout
- **Owner skill:** dev
- **What:** The one instrument this plan adds, then every reading the fix phases bar against.
  The instrument: the per-second `bake queue:` line (`apps/flythrough/src/run/queue.rs`,
  `print_queue_line`) and `run.json` gain the in-view waiting count split by the chunk's band
  (near, mid, far, by the far-normal band applied to the chunk's nearest corner) and by class
  (no record, coarse), the samples those chunks still need, and `drain_s`, the projection of
  that need over the last second's samples a second; the HUD's queue row shows `in view N
  waiting, drain Ns`. The per-submit GPU time (`LoopTimer`, `crates/vxe-render/src/bake/job_loop.rs`)
  is split across the submit's jobs by their samples into a ns-a-sample per class (full, coarse),
  printed on the line as an attribution, not a measurement. **No profiler scopes ride here**:
  Plan 0091's `budget_scope!` wraps `queue step` and `sync` later at the lines this plan sets;
  what Phase 1 needs is on the existing lines and `PassTimer`.
  The readings, in the readings log with machine, adapter, commit, load and the queue's depth:
  1. **The drain curves** at rest from the first world frame, the hills at noon and at 17:00
     and the forest at 15:00 (`--source outdoor`, `--spawn forest --trees forest`): depth,
     in-view waiting by band and class, baked a second by class, bake ms a second drawn
     against the slice granted, `drain_s`, until drained or 600 s; both adapters.
  2. **The census of the listed chunks** at the first world frame on the same scenes: how many
     hold no exposed face, how many are buried under the column map with no emitter in reach
     (the rule of Phase 2, computed on the CPU from the mirror's face masks, the column map and
     the emitter census), how many are surface chunks by band (near, mid, far by the far-normal
     band at 1080p) and in or out of the view cone, and how many of the surface chunks by band
     are **reach-bound by the exact test** (`reach_crosses`, or `len + reach_extent(lr) >
     r_full` with the extent 5.39 chunks at light radius 4, as Plan 0095 Phase 6 lands it; not
     the old `len + lr`), which is what Phase 3's reach rule bakes partial. This census sizes
     Phases 2 and 3; the guess it tests: the residency is a sphere of about 16 chunks' radius
     (17 077 chunks), about 800 columns, and the surface band is 2 to 3 chunks a column, so
     1 600 to 2 400 surface chunks on the hills and more under the forest's canopies, with
     most of the far band past the 11.18-chunk reach line. If Plan 0095 Phase 6 did not name
     the cause of the about 461 px whose chunk holds a block and whose hit brick has no record
     (`--dump-black` on the hills at noon, 24 to 28 chunks within 10 chunks, every dump from
     frame 60 on), this census takes them: the dump's materials, faces and cell states, one
     paragraph in the readings log naming the cause, and the fix in Phase 2 if it is a listing
     rule's miss (reported to the architect if larger).
  3. **The step's 21 stage means** (`bake queue step:` line, per regime) at a depth over 10 000
     and at the drained state, at rest and in a `--glide 300` flight; and the mirror's change
     count a step in the glide (printed beside the `filter` stages, the backlog's ask).
  4. **The moving camera:** in the glide and in a 90-degree turn (the wobble rig's protocol):
     jobs withdrawn before submit, jobs that completed after their chunk left the cone (wasted
     samples), chunks re-listed.
  5. **The RENOIR light pass by arm,** `PassTimer`'s `light` median of 30 frames at rest,
     drained and undrained, the hills at noon and the forest at 15:00: the default; `KEY_OFF`
     (the key ray's share); the sky ray never cast (a hook that reads the record as fresh, or
     `sky_rays 0` where it exists) and `SKY_RAY_ALWAYS`; `far_normal` off; the full tier
     against half; the a-trous at 1, 2 and 4 iterations. The RTX beside it.
  6. **The sync:** the tester cave bench and the forest at leaf fill 128 with the stage lines,
     a `perf record` of the main thread at `alloc`'s p99 syncs (the stages' callees by share);
     and a headless glide that fills the light buffer by shrinking it (`--light-samples` at
     1/16 of `DEFAULT_LIGHT_SAMPLES`, 1 << 20), which is the plan's bet on reproducing the
     window's eviction scan headless; its `light place` p99, evictions and scan steps.
- **Files touched:** `apps/flythrough/src/run/queue.rs`, `apps/flythrough/src/report.rs`,
  `apps/flythrough/src/hud/` (the queue row), `crates/vxe-render/src/bake/queue/open.rs`
  (`view_census` by band), `crates/vxe-render/src/bake/job_loop.rs` (the per-class split),
  `apps/flythrough/tests/queue_line.rs` or the existing CPU test of the line,
  `docs/plans/logs/0096.md`.
- **Done when:** `the_queue_line_reports_in_view_waiting_by_band_and_a_drain_projection` (CPU,
  under 1 s): a synthetic view of 12 chunks (4 near, 4 mid, 4 far, 6 without a record) at a
  known samples-a-second prints `in view 6 waiting (near 2 mid 2 far 2)` and a `drain_s` equal
  to the waiting samples over the rate, and `run.json` carries the same fields; the readings log
  holds tables 1 to 6 with the queue's depth beside every row; the readings of 2 and 3 name the
  census counts the Phase 2 and 5 bars use.

### Phase 2: A record without rays: no face, or buried with no emitter in reach
- **Owner skill:** dev
- **What:** At listing (`push_listed`, `crates/vxe-render/src/bake/queue/list.rs`) a chunk whose
  mirrored face masks hold no exposed face is marked baked-empty in the store's index (no record
  bytes, the reads already return zero for a missing record through the chain, which reads its
  neighbours' summaries as today); a chunk whose reach box lies under the column map's lowest
  top over the box's footprint (`gpu_world/columns.rs`) and whose emitter census in reach is
  empty (`exposed_emitters`, `gpu_world/lights.rs`) gets a zero record written on the CPU with
  `VIS_BAKED` set and `SKY_STALE` clear and a zero summary (ADR-0053), no GPU job. Both marks
  are undone by what undoes a bake today: a dig in the chunk or its reach re-lists it (the dig
  rings), a column top that drops (the column map's finality and catch-up path) re-lists the
  chunks whose box the rule covered, an emitter arriving in reach re-lists them. The rule lives
  in a new `bake/queue/rule.rs`; `bake/queue/mod.rs` (at the 1200-line cap) is split first, its
  tally and stats types moving to `bake/queue/stats.rs`.
- **Files touched:** `crates/vxe-render/src/bake/queue/{mod,list,rule,stats}.rs`,
  `crates/vxe-render/src/bake/{store,record,summary}.rs`, `crates/vxe-render/src/gpu_world/columns.rs`
  (a lowest-top query over a footprint), `crates/vxe-render/src/bake/queue/tests.rs`,
  `apps/flythrough/tests/gpu_bake.rs`.
- **Done when:** (1) `a_buried_chunk_with_no_emitter_in_reach_gets_the_bakes_bytes_without_a_job`
  (`gpu_bake`, lavapipe, about 4 s): a 7 x 7 x 7-chunk world of rock with a 3-chunk cave under a
  flat surface, no emitter: the cave chunks' records written by the rule equal byte for byte the
  records a full GPU bake of the same chunks writes, and the queue submitted 0 jobs for them.
  (2) `a_chunk_with_an_emitter_in_reach_or_a_box_crossing_the_column_top_is_not_skipped` (CPU,
  under 1 s): one emitter voxel 3 chunks away, or the box's top one chunk over the lowest
  column top, and the chunk is listed with a job. (3) `a_dig_to_the_sky_over_a_buried_chunk_relists_it`
  (CPU, under 2 s): a column carved from the surface into the cave lowers the column map and
  the cave's chunks re-enter the queue as stale. (4) `a_chunk_with_no_exposed_face_holds_no_record`
  (CPU, under 1 s): a uniform rock chunk and a uniform air chunk are marked baked-empty and the
  store's bytes do not grow. (5) The hills at noon and the forest at 15:00 at the first world
  frame on the RTX: the queue's depth equals Phase 1's census of surface chunks within 2 %
  (the census and the depth are read on the same frame), and the drained image at 320 x 180
  equals the base's drained image byte for byte (`outdoor_noon` and the forest golden rig of
  `gpu_golden.rs`, the queue drained by `frame:40` in both; lavapipe, about 8 s each, in the
  gate if under 10 s, else `#[ignore = "soak"]`).
- **Amended 2026-11-21 (owner), at the `e1944c81` checkpoint: the no-face verdict moves to the
  stream workers.** At the first world frame the forest's queue held 16 249 listed against about
  5 924 surface chunks, because faceless chunks (6 901) leave the pool only when listed, at 16
  checks an opening. The owner picked over a budgeted main-thread listing at pool sync (a full
  listing read 2.1 s across 16 threads, against Phase 5's 1.0 ms step line) and over reading bar 5
  after the queue has listed: the stream workers compute a chunk's no-face verdict when they mesh
  its mirror data, from its own face masks and its six neighbours' border masks (a neighbour not
  yet streamed counts as open, so the verdict errs toward a job), and the pool admits such a chunk
  already baked-empty; a neighbour arriving later that opens a border re-lists it. The buried
  rule stays at listing (it needs the column map's finality). Bar 5 stands with one change: the
  depth at the first world frame equals the census's surface plus buried-unresolved chunks within
  2 %, and the faceless count admitted to the queue is 0. The image half is read by bar 1's
  live-queue byte test, since the golden rigs bake through `bake_until`, not the live queue. The
  verdict's worker cost is read (microseconds a chunk) and the main thread gains no listing work.

### Phase 3: A tier by size on screen (exploration)
- **Owner skill:** dev
- **What:** `push_listed` and the refinement classes gain the band: the chunk's nearest corner
  projected at the output's focal length against `far_normal` `[lo, hi]` (px a voxel); over `hi`
  is the full tier (`QUEUE_BRICK_SAMPLES` 4096), between is mid (1024), under `lo` is the coarse
  tier (`DEFAULT_COARSE_SAMPLES` 256) **as its final record**. The tier is written in the record's
  head word (two bits of `VIS_WORD`'s free bits, `record.rs`) so the read can tell it and the
  hash tests can pin it. When the camera's chunk changes (`follow_window`, `step.rs`) the bands
  are re-evaluated for the chunks in the view cone: a chunk now in a nearer band than its record
  is re-listed as a refinement (the existing class, scored by ADR-0048), lands and fades over
  `FADE`; a listed job whose band fell is withdrawn if not submitted (ADR-0048's rule, one more
  reason). The sky channel's `SKY_BAKE_RAYS` (256) follows the tier (64 under `lo`) unless Phase
  1's census shows the sky rays are not the far chunks' cost. The tiers are a settings row
  (`bake.tiers`, three counts; the Portable profile's proposal 1024 / 256 / 256, a guess read in
  Phase 8). `--full-first` keeps its meaning inside the near band.
  **The reach rule by tier** (ADR-0080 rule 4, amended 2026-11-20): `reach_ready` keeps
  ADR-0046's rule for a near-band candidate; a mid- or far-band candidate whose reach crosses
  the full radius (`reach_crosses`, `bake/queue/mod.rs`) takes the `Partial` path
  (`summary.rs`, `Reach::Partial`) once nothing in its reach waits to upload, is flagged in
  `partial` as today, and its job's shadow walk reads the mirror as the frame does from the
  job's chunk (`bake_origin_chunk`), so a LOD-only chunk beyond the sphere casts from its mip;
  the phase confirms that with a test (below), since the walk's level choice is by distance
  from the origin chunk and a LOD-only chunk has no level-0 bricks. `requeue_partial` skips a
  flagged chunk whose record is at the mid or far tier (the band promotion is its refinement);
  the dig's ring, at the near count, keeps the re-queue. `ReachMode` becomes the near tier's
  mode (`Full` by default; `--reach partial` and key `8` keep their meaning for the near band,
  a setting that costs nothing when off). The `reach_bound` census (`open.rs`) splits into
  near-band reach-bound (waiting for good at a still camera) and mid/far reach-bound (listed
  partial). Readings, not bars: the drain curves of Phase 1 (1) re-read, the in-view drain time
  by adapter at `frame:1.5`, the record bytes of a far chunk against its full bake and of a
  partial far record against the same chunk baked with its reach filled (the value differences,
  for the owner's eye in Phase 8), the baked far's farthest chunk in view (about 16 chunks
  expected, 11.18 before).
- **Files touched:** `crates/vxe-render/src/bake/queue/{mod,list,open,step}.rs` (`reach_crosses`,
  `requeue_partial`, the census split), `crates/vxe-render/src/bake/{order,record,job,summary}.rs`
  (`ReachMode` scoped to the near tier), `crates/vxe-render/src/settings.rs` (`BakeKnobs`),
  `crates/vxe-render/src/frame/settings.rs` (the profile rows), `apps/flythrough/src/cli/flags.rs`,
  `docs/flythrough.md` (`--reach`), `crates/vxe-render/src/bake/queue/tests.rs`,
  `apps/flythrough/tests/gpu_bake.rs`.
- **Done when:** (1) `the_band_of_a_chunk_follows_the_far_normal_band_at_the_outputs_focal_length`
  (CPU, under 1 s): at 1080p with the fly-through's 70-degree vertical field (f = 540 / tan 35
  degrees, about 771 px) and `[2, 6]`, a chunk whose nearest corner is at 10 m reads full (a
  voxel 7.7 px), at 25 m mid (3.1 px), at 50 m coarse (1.5 px); at 720p the same chunks read
  full, coarse, coarse (f about 514 px: 5.1, 2.1, 1.0 px). (2) `a_far_record_is_promoted_as_one_fade_when_the_camera_comes_near`
  (`gpu_bake`, lavapipe, about 6 s): a chunk baked at the coarse tier at 50 m is re-listed when
  the camera steps to 10 m, its new record carries the full tier's bits, and the store's fade
  state for its blocks runs `FADE` frames once (no second restart). (3) `a_tiers_record_bytes_are_the_same_on_every_run`
  (`gpu_bake`, lavapipe, about 4 s): the coarse-tier record of a chunk hashes the same across
  two runs and two slicings, as ADR-0046's full record does; a partial far record, baked from a
  still camera after the sphere is resident, hashes the same across two runs too. (4)
  `a_far_band_chunk_whose_reach_crosses_the_full_radius_bakes_from_the_chunks_resident` (CPU,
  under 2 s): a streamed `QueueView` with `r_full` 16 and light radius 4 and a candidate list of
  the full sphere: every far-band chunk past 11.18 chunks out is listed with a job and flagged
  partial once nothing in its reach is pending, a near-band chunk at the same reach state (a
  shrunk `r_full` of 4 to make one) waits, and the step after a chunk arrives in a flagged far
  chunk's reach re-queues nothing while the same arrival in a flagged dig ring's reach re-queues
  the ring (the near count). (5) `a_lod_only_chunk_in_a_partial_reach_casts_from_its_mip`
  (`gpu_bake`, lavapipe, about 6 s): a world with one emitter chunk, a far-band receiver and a
  wall chunk between them that is mirrored LOD-only (the streamer's sphere set so the wall is
  past `r_full`): the receiver's partial record is darker on the wall's side than the same
  world with the wall absent, and equals within the coarse tier's noise the record baked with
  the wall resident full. (6) The readings log holds the in-view drain time at `frame:1.5` on
  both adapters, the far record's value difference against the full count, the partial record's
  against the filled reach, and the farthest baked chunk in view.

### Phase 4: The drain slice, and the submits that draw their slice (exploration)
- **Owner skill:** dev
- **What:** `Budget` (`bake/budget.rs`) gains a drain value: while the in-view waiting count is
  over zero the slice is `drain_ms` a frame (ADR-0052's per-second form stays available), and
  when it reaches zero the slice falls to the line's value (`line_ms`, 1.0, the `docs/nfr.md`
  bake line; the default `frame:1.5` becomes the drain value's floor). Proposed defaults, a
  guess until Phase 8: Floor `drain_ms` 4.0, Portable 6.0 (at 30 fps that is 180 ms a second),
  `line_ms` 1.0 on both; the arithmetic that sizes them: after Phases 2 and 3 the in-view work
  on the RTX's forest is about 30 near chunks at 6.5 ms, 250 mid at 1.6 and 1 000 far at 0.4,
  about 1 s of GPU (+6 % for Plan 0095's key rays), so 240 ms a second lands it in about 5 s;
  the RENOIR at 9.3 x the ns a sample and the Portable tiers (1024 / 256 / 256) is about 5 s of
  GPU, about 28 s at 180 ms a second. Both are estimates from Plan 0067's per-sample figures,
  not readings. The RTX's submits drawing 5.3 of a granted 8 ms (Plan 0067 F2) is read by a
  lap on `plan_rounds`, `send` and the `IN_FLIGHT` wait (`job_loop.rs`) and fixed where the lap
  points (rounds planned to the slice, or `IN_FLIGHT` 3, a setting). Readings, not bars: the
  drain curves at the drain slice on both adapters, the GPU frame while draining (the Floor
  profile's 16.6 ms line on the RTX: the hills at noon read 6.7 ms of GPU at 1080p, so 4 ms of
  slice leaves 5.9), the frame's wall and fps.
- **Files touched:** `crates/vxe-render/src/bake/{budget,job_loop}.rs`,
  `crates/vxe-render/src/bake/queue/step.rs`, `crates/vxe-render/src/settings.rs`,
  `crates/vxe-render/src/frame/{baked,settings}.rs`, `apps/flythrough/src/cli/flags.rs`,
  `apps/flythrough/src/run/queue.rs`.
- **Done when:** (1) `the_slice_is_the_drain_value_while_in_view_work_waits_and_the_line_after`
  (CPU, under 1 s): a synthetic queue with 3 in-view chunks waiting reads a slice of `drain_ms`
  at 60 Hz, and the step after the third lands reads `line_ms`. (2) `the_submits_draw_at_least_85_percent_of_the_granted_slice`
  is a reading on the RTX (the per-second line's `slice p50` over the granted slice at
  `second:150,0.25`, the F2 rig), not a lavapipe test; the readings log holds it before and
  after the fix. (3) The drain curves at the proposed defaults on both adapters, with the GPU
  frame and the wall beside them.
- **Amended 2026-11-22 (architect, owner confirm owed): bar 2 is read at the default slice
  form, and the F2 rig's miss is a disclosed reading, not a fix in this plan.** The lap
  (readings log, Phase 4, `da6d113a`) found the plans stopping out of rounds (716 of 1 116 at the
  F2 rig), not at the sample cap, with no step finding both submit slots unread: the submit is
  short of jobs, not of slots or of planned rounds, so neither named fix (rounds planned to the
  slice, `IN_FLIGHT` 3) reaches the cause. The jobs come from the opening, which admits at most
  `QUEUE_CHECKS` (16) candidates a lead a step (`open.rs`), and at the forest's tier mix after
  Phase 3 what a step admits drew 4.35 to 5.48 ms p50 of RTX GPU whatever the grant: under the F2
  rig's per-second grant of 6.3 to 8.9 ms (0.60 to 0.69 drawn), over the drain slice's 4.0 ms,
  where the cap binds (stops at the cap 1 800 against 188 out of rounds, 0.94 to 0.99 drawn).
  Phase 5's lazy walk changed the order's cost, not the cap, so a re-read after it would read the
  same. The fix that reaches the cause is an opening that admits candidates until the opened jobs'
  projected samples (the tier's count times the faces listed, at Phase 1's ns a sample) fill the
  granted slice; every candidate admitted costs a `list` (about 1 ms each, Phase 5's readings),
  the stage Phase 5's bar 3 already misses, so raising the cap trades GPU utilisation under the
  per-second budget for main-thread step time, the cost the owner named first (2026-11-11). So:
  (1) bar 2 reads **at the default slice form** (`bake.drain`, Floor 4,1 / Portable 6,1): the
  submits draw at least 85 % of the granted drain slice while in-view work waits (read 0.94 to
  0.99 at `da6d113a`, loaded; re-read quiet at Phase 9 on both adapters); (2) the F2 rig
  (`second:150,0.25`, ADR-0052's per-second form, no longer the default) stays a disclosed reading
  with its cause, in the log and in `docs/nfr/02-frame-budget.md` at Phase 9; (3) the
  opening-to-the-slice fix rides the Phase 5 bar 3 decision (the owner question in the readings
  log): if the listing leaves the main thread, the cap becomes the slice's and bar 2 is re-read
  at the F2 rig there; if not, Plan 0067's F2 entry returns to `docs/backlog.md` re-pointed at
  the opening's cap, not the planner. No code changes in Phase 4.

### Phase 5: The step off the queue's depth
- **Owner skill:** dev
- **What:** The opening's `candidates`, `list` and `sort` stages visit only the chunks whose
  score can have changed since the last step: the candidate pool keeps its scores and a dirty
  set (the camera's chunk changed: the view-cone members; a chunk arrived or left the mirror;
  a record landed), and the sort becomes a bucket per band and class kept incrementally, the
  step taking the first `OPEN_JOBS` from the nearest non-empty bucket. The `filter` stages
  (`gone`, `drop_reach`, `touched`, `forget`) share one walk of the mirror's change list
  (the backlog's F1). Phase 2's depth cut comes first, so this phase's bar is read at a depth the
  skip leaves (the forest's surface chunks, Phase 1's census) and at a synthetic 20 000.
- **Files touched:** `crates/vxe-render/src/bake/queue/{open,list,step,stats}.rs`,
  `crates/vxe-render/src/bake/order.rs`, `crates/vxe-render/src/bake/stages.rs`,
  `crates/vxe-render/src/bake/queue/tests.rs`.
- **Done when:** (1) `a_still_step_visits_no_candidate_and_a_one_chunk_move_visits_only_the_cone`
  (CPU, under 2 s): a synthetic pool of 20 000 chunks around a still camera: the step's
  `candidates` visit counter (a new stat) reads 0 on the second step; after the camera moves
  one chunk it reads at most the view cone's membership (counted by the test from the same
  geometry), never the pool's size. (2) `the_opened_jobs_are_the_nearest_of_their_band_whatever_the_depth`
  (CPU, under 2 s): the same pool at 2 000 and 20 000 opens the same first 16 jobs in the same
  order (ADR-0048's score kept). (3) The step on the RTX at the forest at 15:00 from the first
  world frame, regime B, release: **p50 at most 1.0 ms and p99 at most 4.0 ms** (today 6.2 to
  7.6 ms p50; the lines proposed under Budgets), read with the depth and regime on the line.
  (4) `images_are_unchanged_by_the_incremental_order` (`gpu_bake`, lavapipe, about 6 s): the
  drained store of a 5 x 5 x 5 world hashes the same before and after this phase (the order
  changes when, not what).

### Phase 6: The slow adapter's light pass, lever by lever (exploration)
- **Owner skill:** dev
- **What:** Each lever is a setting (the cheap-modes rule, owner 2026-11-12), its delta read on
  both adapters at rest, drained and undrained, the hills at noon and the forest at 15:00, the
  default pipeline Plan 0094 leaves (the default-off arms out of it): (a) a cap on the key ray's
  length in voxels past which the face counts lit (`sky_light.wgsl` `key_visibility`; the
  review's E2, 1536 voxels its first guess); (b) no sky ray from a face whose normal points
  down (`n.z < -0.5`), which reaches no sky through the ground under the column rule; (c) far
  pixels (the far-normal weight 1) read **the baked key where the record is fresh, else the
  column map's horizon toward the sun** (ADR-0079; Plan 0095 Phases 2 and 4 land both reads),
  with no key ray, and skip the pair's update (`temporal.wgsl`), which also removes a texture
  write a lighting pixel there; this lever's reading is the horizon march's cost
  (`COLUMN_KEY_STEPS` texture loads a far pixel) against the disc ray it replaces, on both
  adapters, and where Plan 0095 already made it the default the arm is its before; (d) the
  a-trous at 2 iterations in the Portable profile (Plan 0058's setting, unpicked on the
  RENOIR). The RTX is read beside each lever and must stay within 0.1 ms of its base. Readings,
  not bars; Phase 8 picks what the Portable profile owns.
- **Files touched:** `crates/vxe-render/src/shaders/{sky_light,shade,temporal}.wgsl`,
  `crates/vxe-render/src/frame/settings.rs` (the rows and flags), `apps/flythrough/src/cli/outdoor.rs`
  (the outdoor flags live there, not in `cli/flags.rs`; corrected 2026-11-22),
  `crates/vxe-render/tests/gpu_sun/key.rs` (the readings arms; the plan first named
  `apps/flythrough/tests/gpu_sun.rs`, which does not exist; corrected 2026-11-22).
- **Done when:** (1) `the_key_cap_and_the_down_face_skip_leave_the_lit_image_unchanged_where_they_do_not_apply`
  (`gpu_sun`, lavapipe, 320 x 180, about 8 s): a flat open floor at noon renders byte-identical
  with (a) and (b) on and off (no ray there is longer than the cap, no face looks down), and a
  box with an overhang differs only under the overhang. (2) The readings log holds the lever
  table on both adapters with the RTX deltas; the RENOIR's light pass with every lever on at
  the Portable profile is the number Phase 8 weighs against the 4.0 ms line.
- **Amended 2026-11-20 (owner), from Plan 0095's Phase 9 readings (`c6b6bf52`):** the approach
  penumbra is the default on every adapter and profile (the owner's pick over keeping `disc` on the
  slow adapter), and it is this phase's first named lever. Headless 1080p, forest 15:00, at rest:
  RENOIR 34.0 ms against 11.8 ms under `disc` (hills noon 11.2 against 7.4), RTX 3.32 against
  1.21. The lever table reads approach's own cost and the levers that cut it (its step count, its
  reach, a half-rate form) on both adapters; the Portable profile keeps approach unless the owner
  picks otherwise at Phase 8.
- **Amended 2026-11-22 (architect, owner confirm owed): lever (b) leaves bar 1 and the window;
  the setting stays; bar 1 becomes the key cap's test.** Bar 1's overhang check cannot hold on
  cube faces: `sky_visibility_sample` (`sky_light.wgsl`) draws its rays as a cosine lobe about the
  shading normal `n` and marches only a ray with `d.z > 0`, so on a face whose normal points
  straight down (`n.z = -1`) every ray is already counted blocked without a march, and the skip
  changes neither the image nor the cost there (the first run read "0 pixels differ, 5 388 on the
  underside"); it reaches only a face whose smoothed or far normal has `-1 < n.z < -0.5`, where it
  also darkens a face a sideways-up ray could have lit past an overhang's edge, so it is not exact
  there either. Its gain is bounded above by Phase 1's table 5 `--sky-rays 0` arm, which casts no
  sky ray at all: RENOIR forest 17.68 against 17.68 ms, hills noon 4.78 against 4.94, at most about
  0.16 ms against the 4.0 ms line, because at rest the sky ray is cast only where the record is
  stale (ADR-0070). A bar for 0.16 ms is not worth holding, and the third option (the skip by the
  hit's cube normal) is a no-op on the image by the same arithmetic; reading the lever on the far
  normal's band would read a band Plan 0098 is about to move. So: lever (b) is **dropped from bar
  1 and from Phase 8 (d)** (the window reads the key cap's look and 2 a-trous iterations), the
  built setting `lighting.sky_down_skip` / `--sky-down-skip` stays, default off (the cheap-modes
  rule, owner 2026-11-12), and the `skydown` arms the lever table is already running are recorded
  as the reading that closes it. **Bar 1 becomes the key cap's test**
  (`the_key_cap_leaves_the_lit_image_unchanged_where_no_ray_is_longer`,
  `crates/vxe-render/tests/gpu_sun/key.rs`, lavapipe, 320 x 180): a flat open floor at noon
  renders byte-identical with the cap on and off (a clear ray counts lit at the cap as at
  `KEY_RAY_LEN`), and a box with an overhang differs only on the floor under the overhang, whose
  ray meets the slab at the clearance height: the test sets the cap under that clearance (at the
  default 1 536 voxels nothing in a small rig is longer than the cap, which is why the first run
  read the overhang identical under the cap), and that floor counts lit under the cap while the
  open floor and the box's lit faces are unchanged. Its wall is read quiet; over the 10 s soak
  line (the first run read 54.3 s with six frames) it is born `#[ignore = "soak"]` and named so in
  the Implementation log. The approach penumbra's own lever waits for Plan 0098's window W1 and
  is out of this phase (the orchestrator's scope, 2026-11-22).

### Phase 7: The sync: the eviction scan's index and the `alloc` stage
- **Owner skill:** dev
- **What:** Plan 0074's Phase 2 as drafted: the light buffer's free ranges in a first-fit index
  by size (the draft `fit.rs`, untracked in the main checkout at `live/plan-0074/drafts/`,
  read for its shape, not merged as is) and the eviction's victim from a distance-ordered index
  of the placed chunks (a `BTreeMap` by squared distance to the focus, kept on placement and
  on focus moves), so an eviction is O(log n) and `light_scan_steps` counts only the index
  walks; the first fit's and the victims' steps split on the line (the backlog's ask). Then
  `alloc` from Phase 1 (6)'s profile: the candidates named by the profile are `fill_slot`'s
  per-brick path, the mapped belt view's drop (`BufferViewMut::drop`, one unmap per chunk
  where one per sync would do) and `upload_brick`; the fix is what the profile points at, bytes
  unchanged.
- **Files touched:** `crates/vxe-render/src/gpu_world/{lights,alloc,slot,stage,window}.rs` and a
  new `gpu_world/fit.rs`, `crates/vxe-render/tests/` (the mirror's light placement tests),
  `apps/flythrough/src/report.rs`.
- **Done when:** (1) `the_indexed_placement_writes_the_same_bytes_as_the_scan` (CPU, under 2 s):
  a recorded sequence of 2 000 placements and 300 evictions with a moving focus gives the same
  ranges and the same light buffer bytes through the index as through today's walk (the walk
  kept behind a test-only path until this test passes, then deleted). (2) In the headless glide
  that fills the buffer (Phase 1 (6)'s rig, `--light-samples` at 1/16): `light place` p99 under
  2 ms over 300 s with at least 1 000 evictions logged (today the window's up to 1 186 ms; if the
  headless rig does not evict, the bar is read in Phase 8's glide). (3) The tester cave bench on
  the RTX host: **`alloc` p99 at most 3.0 ms** (today 7.3 to 8.8) and **`update + poll + sync`
  p99 at most 16.6 ms** (today 18.0 to 23.7), the forest at leaf fill 128 sync p99 at most
  6 ms (today 11.0); `pack + size` read beside them for the backlog's "Slot words packed on the
  workers" entry.

### Phase 8: The owner's window
- **Owner skill:** human
- **What:** One session on the dev box (Linux; the RENOIR is the same laptop's iGPU through
  `--adapter AMD`), release at Phase 7's tip, the runner `live/session96.sh` with a banner word
  per arm, stdout visible, HUD on with the queue row. (a) **Rule 6 by eye:** the forest at
  15:00 and the hills at 17:00 from a fresh start on both adapters: does the image before the
  drain look like the image after it but coarser, does each landing read as one fade, is
  anything popping or black while `in view N waiting` falls to 0, and how long until it does
  (the HUD's `drain Ns` against the stopwatch). (b) **The far tier:** fly slowly toward a far
  tree and a far ridge: is the promotion's fade visible, do the far shadows at 256 samples look
  wrong against the same view drained at `bake.tiers 4096,4096,4096`; on the approach from 100
  to 40 m, does the far ridge's partial record (its rim cast from mips) read any different from
  the mid band's full-reach record it fades into, and is the line where the baked far ends and
  the horizon begins (about 102 m, the LOD ring) visible as a seam in the key light's shadows.
  (c) **The drain slice's
  trade:** arms `drain_ms` 1.5, 4.0 and 8.0 on the RTX and 3.0, 6.0 and 12.0 on the RENOIR:
  the fps the owner feels against the seconds to land. (d) **The RENOIR's light pass levers**
  (Phase 6's a to d) in the Portable profile: the key cap's and the down-face skip's look, 2
  a-trous iterations. (e) **The sync:** a 60 s `--glide 300` on the tester's cave path with
  the sync's stage row visible; the stagger at 30 to 40 s present or gone. The owner's words
  and picks are quoted in the readings log; the architect amends Phase 9's values.
- **Files touched:** none (the runner under `live/`, untracked).
- **Done when:** the readings log quotes the owner's verdict on (a) to (e) and a pick for
  `drain_ms` per profile, the tiers per profile, and the light-pass levers the Portable profile
  owns; a pick the owner declines to make takes the recommended value.

### Phase 9: The bars, the lines and the goldens after the picks
- **Owner skill:** dev
- **What:** The picks become the defaults and the profile rows; the bars pinned: the drain time
  of what is in view and of the whole residency, the step's lines, the sync's lines, the light
  pass readings, in `docs/nfr.md` section 2 with the readings in `docs/nfr/02-frame-budget.md`
  (the bake-slice line rewritten as two values, drain and landed); the goldens the picks moved
  re-blessed with their reason; the backlog's entries this plan took closed or re-pointed; the
  dig's enqueue read once more with the tiers (a dig's ring is baked at the near tier whatever
  the band, since the owner is looking at it).
- **Files touched:** `crates/vxe-render/src/frame/settings.rs`, `apps/flythrough/tests/gpu_bake.rs`,
  `apps/flythrough/tests/gpu_golden.rs` and its goldens, `docs/nfr.md`,
  `docs/nfr/02-frame-budget.md`, `docs/backlog.md`, `docs/flythrough.md`.
- **Done when:** (1) `the_in_view_drain_of_the_forest_lands_within_its_budget` is a release
  reading on both adapters recorded in the log (the headless rig of Phase 1 (1), the budget
  lines below), not a lavapipe test; (2) `a_digs_ring_bakes_at_the_near_tier_in_the_far_band`
  (`gpu_bake`, lavapipe, about 4 s): a dig at 50 m lists its ring at 4096; (3) every Phase 2 to
  7 test passes at the picked defaults; (4) `cargo nextest run --profile gate` at the tip under
  the perf lock, its wall recorded against section 7's 19 min; (5) the nfr lines read the
  values below, with machine and commit.

## Data shapes

```rust
// illustrative
pub enum Band { Near, Mid, Far }            // by the far-normal band at the output's focal length
pub struct Tiers { pub near: u32, pub mid: u32, pub far: u32 }  // samples a brick direction; 4096 / 1024 / 256
pub enum Verdict { NoFace, ZeroByRule, Bake(Band, Reach) }     // what the rules say of a listed chunk;
                                                                // Reach::Partial only for Mid and Far
pub struct Budget { pub drain_ms: f64, pub line_ms: f64, pub per_second: Option<(f64, f64)> }
// record head word: VIS_BAKED (1 << 16), SKY_STALE (1 << 17), TIER (2 bits of the free bits)
```

The store's bytes do not grow: the tier takes free bits of the record's head word, the
baked-empty mark is an index state with no record. The index of placed light chunks by distance
is one `BTreeMap<(u64, ChunkCoord), Range>` beside today's `chunks` map, about 24 bytes a placed
chunk (8 192 chunks at the cap: under 200 KB).

## Budgets

| `docs/nfr.md` figure | today (reading) | proposed line | how the close measures it |
|---|---|---|---|
| **New: in-view drain** (every chunk in the view cone that can hold a record holds its tier's record, from the first world frame, at rest; a LOD-ring face served by the column map's horizon and sky rule has no record to wait for and counts as landed, amended 2026-11-20) | forest 15:00 never in 30 000 frames; 4 419 in view waiting at 94a; the hills "landed" at 26 s with 2 964 in view reach-bound under the old rule | **RTX Floor at most 10 s; RENOIR Portable at most 30 s** (estimates from the arithmetic in Phase 4; pinned from Phase 9's readings) | the Phase 1 (1) rig, release, both adapters, hills noon and forest 15:00, `drain_s` and the stopwatch frame, the in-view waiting count at 0 with the reach-bound count beside it |
| **New: whole-residency drain** at rest | 7 to 30 min on the RTX (94a) | **RTX at most 60 s; RENOIR at most 180 s** (estimates) | the same rig to depth 0 |
| The live bake's slice, 1.0 ms | `frame:1.5` default, over the line | two values: `drain_ms` while in view waits (Floor 4.0, Portable 6.0, the owner's pick), **`line_ms` 1.0 once landed**; the Floor frame stays within 16.6 while draining | the per-second line's slice p50 and the GPU frame, both adapters |
| The queue's step (inside the 1.0 ms streaming line; no line of its own today) | 6.2 to 7.6 ms p50 at a depth over 14 000; 0.075 drained | **p50 at most 1.0 ms at any depth, p99 at most 4.0 ms**, both regimes | the `bake queue step:` line at the forest from the first world frame, RTX, release |
| Lighting, 4.0 ms (RENOIR) | 8.025 hills, 12.034 forest at rest, drained | the levers' deltas read; **the Portable profile's light pass at most 4.0 ms on the hills at noon**; the forest's figure is a reading the owner weighs in Phase 8 (the line stays; a miss is disclosed, not moved) | `PassTimer` `light` median of 30 frames at rest, drained and undrained, 720p Portable |
| Lighting (RTX) | 1.570 / 2.529 | unchanged within 0.1 ms by every lever | the same, 1080p Floor |
| `update + poll + sync` p99 (streaming) | 18.0 to 23.7 ms cave | **at most 16.6 ms**; `alloc` p99 at most 3.0; `light place` p99 under 2 ms in a glide that evicts | the tester cave bench and the Phase 1 (6) glide, RTX host, `hpet` noted |
| Images | | drained images byte-identical through Phase 2 and Phase 5; moved only by the tiers and the levers the owner picks (re-blessed with the reason) | `gpu_golden` on lavapipe |
| Section 7, the gate | 1792.6 s at 0086's close, over 19 min | this plan adds about 50 s of lavapipe (the tests named, each under 10 s) and no soak test by default | the tip's gate wall |

## Risks & open questions

- **The zero-by-rule record at the column map's edge.** The rule is exact under ADR-0070's
  column clause only if the column map is final for the footprint; a chunk whose footprint's
  columns are still provisional is baked, not ruled (the census says how many). A dig that
  lowers a column must re-list every chunk the rule covered (Phase 2's test 3); a miss there is
  a dark cave under a new hole, visible at once.
- **The store becomes camera-dependent.** ADR-0046's bit-identity "whatever the slicing" now
  holds per tier; the `--bake-file` hash and Plan 0089's mirror oracle read the tier bits
  beside the bytes. Phase 3's test 3 pins it.
- **The far tier's look.** 256 samples a brick direction is the coarse pass the owner has seen
  as a first pass, never as a final record on a far ridge; if the window rejects it the mid
  count becomes the far tier (the arithmetic in Phase 4 then roughly doubles the drain).
- **A partial far record's bytes are the residency's at the bake** (amended 2026-11-20). At a
  still camera the job waits until nothing in reach is pending, so the record is the sphere's
  and Phase 3's hash test holds; in motion it is whatever had arrived and stays until the band
  promotion. A camera that stops at 60 m keeps far records cast partly from mips; Phase 8 (b)
  reads whether the eye can tell. If it can, the re-queue on arrival comes back for the far
  tiers (ADR-0080 Alternative G) at one more fade per approached chunk.
- **The drain's order may not match what is visible** (the backlog's "The bake queue ordered by
  what the march touched", 2026-11-20): the in-view drain orders by an estimated size on screen
  and bakes chunks behind hills and trunks before some the eye sees. If the in-view drain misses
  its 10 s bar after Phases 2 to 4, the march's touched set (a bit per chunk hit this frame,
  read back a frame late) is the optional lever: it orders the queue by actual visibility and
  skips the occluded. Taken inside this plan only on that miss; otherwise the entry stays.
- **The RENOIR may not reach 4.0 ms in the forest** at any lever this plan names; the hit pass
  (10.7 to 21.9 ms there) is not this plan's and the backlog's beam prepass is the next lever.
  The plan discloses the miss; the Portable profile owns the levers it picks.
- **The eviction scan may not reproduce headless** even with a shrunk buffer (the window's
  buffer fills, the headless glides' did not, cause open); Phase 8 (e) is the fallback reading
  and Phase 7's bytes test holds either way.
- **`IN_FLIGHT` 3** (if the F2 lap points there) holds one more submit's prepared bytes
  (`PREPARED_BYTES_MAX` 64 MiB is the cap) and lengthens the withdrawal's blind spot by one
  submit.
- **Plan 0095's bake rays** (+6 %, a guess; only if its window W2 keeps the record as the far's
  refinement) and its far-pixel read (the baked key, else the horizon) change Phases 3, 4 and
  6's numbers; the plan starts after it merges, and Phase 1 re-reads if it ran before.
- **The baked far ends at the sphere** (about 102 m at the defaults, 77 m in the Portable
  profile): past it the key light is the horizon's heightfield shadow for good. Whether that
  seam shows is Phase 8 (b)'s question; the answer that moves it is ADR-0079's, not a wider
  residency (ADR-0080 Alternative E).
- **Threading:** nothing moves off the render thread; the CPU-written zero record goes through
  the store's existing write path on that thread.

## What this plan does NOT do

- **The hit pass on the RENOIR** (21.9 ms on the hills against 3.0): the backlog's "A beam
  prepass, or a distance value per brick", before Plan 0033.
- **The screen-space key and sky means' restart** (the other half of "everything wobbles"):
  Plan 0095.
- **The coarse prior's look before a landing** (rule 6's "what the image shows meanwhile"):
  Plan 0094.
- **A profiler.** Plan 0091's scopes, GPU lane and `budget_scope!` come after; this plan's one
  readout (Phase 1) is a line, and 0091 wraps `queue step` and `sync` at the lines Phase 9 sets.
- **FSR 2's upscale line** (9.97 ms on the RENOIR) and the denoise line (7.0 to 7.6): the
  profile plan's lines, read beside Phase 6's table and left.
- **Slot words packed on the workers** (`pack + size` at 17 % of the sync p99): its backlog
  entry's rule (half or more of what is left after `alloc`) is re-read at Phase 7 (3).
- **The first-block fallback fade's CPU under `chain`** (about 1.7 ms of a regime B step): a
  look change or an ADR-0053 Outcome, read beside Phase 5's bar and left in the backlog.
- **A worker thread for the step** (ADR-0017's narrowing): not needed if Phase 5's bar holds;
  named as the next lever if it does not.
- **The pipeline cache** (Plan 0094 gives the GPU tests one; the engine's is the backlog's).
- **Windows readings** (owner, 2026-11-15: once in a while, not per plan).

## Implementation log

> Written by `dev`: one row per phase as its commit lands, then the close block. **The phases
> above are the contract. This section records what happened.** Observations, never conclusions:
> no pass list, no self-assessment. Deviations and unmet done-whens are **always** disclosed.
> Keep it shorter than `## Implementation phases`. Readings, gate counts, attempts, sweeps and
> resume notes go to the plan's readings log, `docs/plans/logs/0096.md` (started from
> `templates/readings-log.md`), and this section links it once it exists.

Readings log: [logs/0096.md](logs/0096.md).

| phase | owner | state | commit |
|---|---|---|---|
| 1: readings on the undrained image, the drain readout | dev | done; readout code in `e1944c81` (wip, with Phase 2's first cut); table 5 lacks `far_normal` off, `KEY_OFF` and `SKY_RAY_ALWAYS` (re-run queued on `--light-hooks`, a reading flag added in `cli/settings.rs`), table 6 lacks the `perf` profile; the 1/16 light-buffer glide evicts headless (`light place` p50 813 ms) and runs about 1 frame a second | `2437b054` |
| 2: a record without rays | dev | done as amended: the zero record by rule at listing (`e1944c81`), the no-face verdict on the stream workers (`bake/seal.rs`, `bake/queue/sealed.rs`); bar 4 is a lavapipe test (the store needs a device); bar 5 (the census against the depth on the amended binary) and the verdict's worker cost not read (the GPU lock is held by the Phase 1 glide); bar 1 read 10.4 s loaded, its quiet re-read owed; `apps/flythrough/src/setup.rs` and `run/pilot.rs` outside the files touched | `c6af7f7d` |
| 3: a tier by size on screen | dev | done (exploration), from the `9800edb0` wip; the reach code moved to a new `bake/queue/reach.rs`; bars 2, 3, 5 in a new `tests/gpu_bake_tiers.rs`, not `gpu_bake.rs`; bar 1's 720p bands read mid, mid, far (the plan's text: full, coarse, coarse); bar 4 reads the crossing past 10.61 chunks, an octant of the sphere; far-tier sky rays 64 without the census reading; `apps/flythrough/src/run/queue.rs`, `cli/{mod,parse,settings}.rs`, `run/{loading,profile}.rs`, `frame/apply.rs`, `lib.rs`, `bake/queue/{digs,keys,stats,tiers}.rs` outside the files touched; bars 2, 3, 5 and bar 6's readings not read (the perf lock held by plan 0097's window) | `01b35ccb` |
| 4: the drain slice, the submits' slice | dev | done (exploration), code in the `da6d113a` wip (with Phase 3's `retier` idle fix and its tests' rigs); `bake/queue/{mod,open,stats,tiers,tests}.rs`, `frame/apply.rs`, `lib.rs`, `apps/flythrough/src/cli/{mod,parse,settings}.rs` (`--bake-drain` there, not `cli/flags.rs`), `run/{loading,profile}.rs`, `docs/flythrough.md` outside the files touched; bar 2 not met: the F2 rig draws 0.60 to 0.69 of the granted slice, plans stopping out of rounds, and neither named fix (rounds to the slice, `IN_FLIGHT` 3) applied; bar 3's readings loaded (load 0.9 to 14.8) | `b95913fb` |
| 5: the step off the depth | dev | done; the order is a lazy walk over 4-chunk cells per class, not a bucket per band; bar 1 drives `Ordered`, not a queue step; bar 4 in `gpu_queue.rs` against a one-shot bake, not a before/after hash; F1 shares one walk for `gone` and `forget` only; `apps/flythrough/src/run/queue.rs`, `tests/gpu_queue.rs` outside the files touched, `stages.rs` untouched; bar 3 not met (step B p50 20.7 ms, p99 55.0, loaded): owner decision owed: (a) disclose / (b) amend bar to Phase 5's stages / (c) listing on a worker thread | committed with this row |
| 6: the slow adapter's light pass, lever by lever | dev | done (exploration) as amended, code in the `2665f41f` wip; lever (b) built and default off, out of bar 1; bar 1 is the key cap's test, `#[ignore = "soak"]` (20.8 s, loaded), its rig under `key_far ray` (under the default `baked` the horizon replaces the ray and no cap shows) and a floating slab (no post); `cli/mod.rs`, `hud/modes.rs`, `frame/{compose,encode}.rs`, `settings.rs`, `layout_tests.rs`, `common.wgsl`, `docs/flythrough.md` outside the files touched; the lever table read loaded (load 1.2 to 4.1); the RENOIR's all-levers light pass at Portable 4.66 ms hills noon, 17.43 forest, over the 4.0 line | committed with this row |
| 7: the sync's eviction index and `alloc` | dev | done; the indexes in `fit.rs` and `lights.rs` `Ranges`, the walk kept as the test's `#[cfg(test)]` reference (not deleted); bar 1 a unit test in `gpu_world::lights`, not under `tests/`; `light_scan_steps` split into fit and victim steps; `alloc` unchanged: the frame-pointer profile names no item holding a third of it; bar 2 not met (glide `light place` p99 17.6 ms, loaded: the walk retries ~15 400 refused chunks a sync); bar 3 not met (cave `alloc` p99 9.4 ms, `update + poll + sync` 21.4 to 21.7, forest leaf 128 sync 7.5); `gpu_world/{alloc,slot,stage,window}.rs` untouched, `tests/gpu_mirror_budget.rs`, `apps/flythrough/tests/path.rs` touched | committed with this row |
| 8: the owner's window | human | not started | |
| 9: the bars, the lines and the goldens | dev | not started | |

### Notes

_(Deviations from the plan, with the commit, stated without justification. Done-whens not
satisfiable as stated, and what was done instead. Followups noticed and not acted on. One line
each. Empty is fine.)_

### Close triggers

_(Facts for the architect. No recommendations, and no suggested version bump.)_

- **What shipped:**
- **Public API changed:**
- **Budget readings:**
- **Gate at the tip:**
- **Outstanding `human` phases:**

## Followups

- (empty at draft time)
