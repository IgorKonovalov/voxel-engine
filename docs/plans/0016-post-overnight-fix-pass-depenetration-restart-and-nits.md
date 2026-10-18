# 0016: The post-overnight fix pass: depenetration, a logic that restarts after a trap, and the review nits

> **Status:** in-progress
> **Created:** 2026-10-17
> **Milestone:** M3 (depenetration), M5 (restart) and M7 (the HUD nits) in `docs/roadmap.md`
> **Related ADRs:** [ADR-0005](../adrs/0005-vertical-cylinder-collision-by-integer-conservative-advancement.md)
> (its "no depenetration" consequence closes with a dated Outcome),
> [ADR-0010](../adrs/0010-game-logic-as-a-component-behind-a-logic-trait.md) (its fail-stop
> Outcome is overturned by a dated Outcome, the owner's call of 2026-10-17)
> **Prerequisite:** none. Touches `vxe-sim`, `vxe-script-wasm`, `vxe-net`, the fly-through's
> HUD and `vxe-world` comments; independent of Plans 0006, 0013 and 0015.
> **Source:** `docs/decisions-pending.md` D6 (the owner overturned two overnight calls) and
> D7 item 3; Plan 0004 Followup 1; Plan 0010 Followups (the first three) and Plan 0014
> Followups (two comment nits).
> **Amended:** 2026-10-17, before `dev` started, with two findings from the owner's windowed
> test of v0.12.0: rubble that rounds away on a thin region (Phase 4) and a scroll wheel that
> never changes the dig radius on smooth scrolling (Phase 5), rechecked in a window (Phase 6).
> The test's third finding, collapse that rarely triggers in the cave, is a stress model the
> owner wants as its own ADR and plan (see "What this plan does NOT do").

## TL;DR

Three small things the owner asked for after the overnight run, in one dev pass. A body that
an edit fills into is lifted to the nearest free pose instead of staying embedded; a hosted
logic that traps is restarted from its seed on the next tick, up to a cap, instead of
staying stopped for the run; and the five review nits from Plans 0010 and 0014 are fixed
(a shown HUD allocates nothing per frame, the memory estimate counts the brick-array pool,
two stale comments, an unused parameter, two missing doc comments). The first thing a
reader sees is the level walk test: a fill on the body's feet is followed by a tick that
ends with the body standing on the new floor. The amendment adds two fixes from the owner's
windowed test: a removed region's rubble carries each column's rounding remainder to the next
column, so a thin region piles its quarter instead of nothing, and the fly-through banks
fractional scroll until it makes a whole step.

## Context & problem

**Depenetration.** `sweep` reports `started_overlapping` and moves nothing
(`crates/vxe-sim/src/sweep.rs:132-136`); `tick` never reads the flag
(`controller.rs:257`), so a body an edit fills into stays embedded, every sweep returning
0. `Walk::at_camera` refuses to place an overlapping body (`apps/flythrough/src/fly.rs:99`),
so only an edit onto the body causes it. Plan 0004 recorded it as a known gap and ADR-0005
as a negative consequence; the owner overturned the call on 2026-10-17.

**Restart after a trap.** Plan 0011's amendment chose fail-stop: after a trap (fuel,
memory, a guest or host panic) `try_tick` returns `LogicError::Stopped { tick, since }`
without touching the instance again (`crates/vxe-script-wasm/src/host.rs:365-391`), and
the server logs the inputs, applies submitted ops and suppresses the broadcast
(`crates/vxe-net/src/server.rs:218-264`). The rejected alternative (a), a fresh instance
with `init` again, was recorded with its hazard: "the restarted logic believes it is at
the start of the run while the world holds every edit since; `init`'s own ops apply a
second time". The owner chose (a) anyway; the plan keeps the hazard visible by capping the
restarts and reporting each one.

**The nits.** Plan 0010's review: the HUD's rows `Vec` (`apps/flythrough/src/main.rs:957-962`),
the `last_edit` clone (`:935`) and the ms-formatting closure (`apps/flythrough/src/hud.rs:356`)
allocate every frame while the HUD is shown; `cpu_bytes_estimate` (`hud.rs:231-234`) counts
`Full` bricks and chunks but not the brick-array pool (`World::pool_len`,
`crates/vxe-world/src/store.rs:235`), and the "pool" row reports mirror slots;
`MarchTimer::read_ms` has an unused `_queue` (`crates/vxe-render/src/marcher.rs:510`);
`HudOverlay` and its `scale` lack doc comments (`hud.rs:49`, `:55`). Plan 0014: the "4 Mi"
comment (`main.rs:92`) means 2 Mi voxels, 4 MiB, and `CapsuleRow::nearest`
(`crates/vxe-world/src/shape.rs:379`) should state the `i128` range its closed form stays
within.

**Rubble that rounds away.** `World::remove_region` piles `h = n * fraction.0 / fraction.1`
rubble voxels per voxel column, by integer division per column
(`crates/vxe-world/src/collapse.rs:575-579`). A column with fewer than 4 carved voxels gets
none at the fly-through's quarter, so a region that is thin in z piles nothing: the owner's
test printed `removed: {rock: 201}, 0 rubble` for a 2-cell region. The doc comment states
the per-column rule, so this is the specified behaviour, not a bug in the code; the owner
wants the region's quarter on the floor.

**The scroll wheel.** `handle_input` adds `input.scroll().round()` steps to the dig radius
each frame (`apps/flythrough/src/main.rs:1975-1981`), and `vxe-app` reports a pixel delta
as `y / 40` lines (`crates/vxe-app/src/shell.rs:320-323`). Smooth or touchpad scrolling
delivers less than half a line per frame, which rounds to zero and is cleared with the
frame's input: the owner scrolled about 40 times and every line read `dig radius 0.5 m`.

Budgets touched: none moves. `nfr.md` §8's controller tick (2.6 µs on terrain) is re-read
because the tick gains an overlap test at its start.

## Decision

Depenetration lives in the controller, not in the sweep, so `sweep` keeps its contract
(`a_body_that_starts_overlapping_does_not_move` stays as it is): at the start of `tick`,
if the body overlaps solid, the controller searches a fixed, integer sequence of candidate
poses and takes the first free one, reporting the lift in the tick's result; a body with no
free pose within reach stays and reports `embedded`. The host restarts a trapped logic on
the next tick with a fresh instance and `init(seed)`, at most `max_restarts` times per
session (default 8), then stops as today; each restart is reported. The nits are fixed as
the reviews worded them. `remove_region` keeps one integer accumulator across the columns in
their fixed order (cell stacks in `BTreeMap` order, then `ly`, then `lx`): each column adds
`n * fraction.0`, takes the accumulator's whole multiples of `fraction.1` as its `h` and
keeps the rest, so the region piles `floor(N * fraction.0 / fraction.1)` in total less only
what finds no ground or no air. The fly-through banks scroll in an accumulator, takes whole
steps toward zero and keeps the fraction. We rejected depenetration inside `sweep` (it would change what
every caller of `sweep` sees and the pinned sweep test), and an unbounded restart loop (a
logic that traps every tick would re-instantiate the component every tick, a server-side
cost the guest controls). For rubble we rejected rounding each column up (a one-voxel
sheet would pile as much rubble as it had voxels, not a quarter). For the scroll we
rejected changing `vxe-app`'s `Input` to bank fractions (it reports what the frame
delivered; how a consumer quantises it is the consumer's choice).

## Architecture diagram

```mermaid
flowchart LR
    subgraph vxe-sim
        T[tick] --> O{overlaps at start?}
        O -->|no| S[sweeps as today]
        O -->|yes| D[depenetrate: fixed candidate order]
        D -->|free pose| S
        D -->|none| E[embedded, no move]
    end
    subgraph vxe-script-wasm
        C[call] --> F{failed_at set?}
        F -->|restarts < cap| R[fresh instance, init seed, then tick]
        F -->|cap reached| X[Stopped]
    end
```

## Implementation phases

Each phase ships as its own commit. `dev` implements all phases in one session with no review
between phases. The architect reviews once at the end, in a fresh session.

### Phase 1: Depenetration in the controller
- **Owner skill:** dev
- **What:** `tick` tests the body against the world first; when it overlaps, it tries, in
  this order and stops at the first free pose: up by 1 voxel at a time to `step_height`,
  then for each distance `d` from 1 voxel to `climb_height` in voxel steps the six offsets
  `+x, -x, +y, -y, +z, -z` scaled by `d` (a fixed order, so the result is a function of the
  world and the body alone); the move is recorded in a new `MoveResult::lifted: Fixed3`
  (zero when nothing was lifted) and `MoveResult::embedded: bool`. The course feed
  (`crates/vxe-sim/tests/course.rs:10-37`) is not changed, so the pinned hash stands.
- **Files touched:** `crates/vxe-sim/src/controller.rs`, `crates/vxe-sim/src/lib.rs`
  (`MoveResult` fields), `crates/vxe-sim/tests/controller.rs` (new cases),
  `apps/flythrough/src/fly.rs` (the walk test gains a fill-on-the-body step; the fly-through
  prints the lift once), `docs/adrs/0005-*.md` (a dated Outcome at the close, by the
  architect), `docs/nfr.md` §8 (the controller tick re-read).
- **Done when:**
  - On F1 with the body standing on the floor, a fill that raises the floor by 2 voxels
    under the whole footprint is followed by one tick whose result has `lifted == [0, 0,
    512]`, `embedded == false`, and `!overlaps` afterwards; the body's horizontal position
    is unchanged.
  - A fill that raises the floor by `step_height + 1` voxel is answered by a lift along the
    first free horizontal offset when one exists at distance 1, and otherwise by an upward
    lift at the smallest distance that frees the body; the test pins both with exact poses.
  - A body enclosed in solid on all sides within `climb_height` reports `embedded == true`,
    `lifted == 0` and moves nothing; the next tick reports the same.
  - `random_walks_on_the_level_never_freeze_or_overlap` gains a fill on the body every 50th
    walk and still asserts `!overlaps` after every tick.
  - `crates/vxe-sim/tests/course.rs` still pins `0x0c80_8031_d14f_27eb`; the sweep test
    `a_body_that_starts_overlapping_does_not_move` is unchanged.
  - The `60 ticks of the F6 staircase` bench reads within 10 % of 156 µs (2.6 µs per tick),
    quiet box, recorded in the log.

### Phase 2: A trapped logic restarts from its seed, up to a cap
- **Owner skill:** dev
- **What:** `WasmConfig::max_restarts` (default 8); after a failed call at tick `T`, the
  next call instantiates a fresh component, runs `init(seed)` and then the requested
  `tick`, and the report carries `LogicError::Restarted { tick, after: T, restarts: n }`
  for that tick's failure record; after `max_restarts` restarts the logic stops with
  `Stopped { tick, since }` as today. `init`'s ops apply on the world as it is.
- **Files touched:** `crates/vxe-script-wasm/src/host.rs`, `crates/vxe-net/src/logic.rs`
  (`LogicError::Restarted`), `crates/vxe-net/src/server.rs` (the report), the tests in
  `crates/vxe-script-wasm/tests/determinism.rs`, `apps/flythrough/src/main.rs` (prints each
  restart), `docs/adrs/0010-*.md` (a dated Outcome at the close, by the architect),
  `docs/nfr.md` §4 (the component paragraph).
- **Done when:**
  - `after_running_out_of_fuel_the_logic_is_stopped` becomes `..._restarts_then_stops`: with
    `spin`, ticks 0 to 9 report `Fuel{0}`, then `Restarted{1, 0, 1}` with `Fuel{1}` ...
    through the eighth restart, and ticks 9 onward report `Stopped{tick, 8}`; the test
    states the exact sequence.
  - `edits_applied_before_a_failure_stay_and_resimulate`: with `hog` at `max_restarts = 1`,
    the op log holds the edit applied before the memory trap and the edit `init` applies on
    restart, in that order; re-simulation from the log gives an equal hash and equal log
    bytes, and a second session gives an identical report and log.
  - `a_failed_init_stops_the_logic_from_tick_zero` holds with `max_restarts = 0`, and with
    the default it restarts once per tick until the cap, each attempt failing in `init`.
  - `ECHO_HASH` (`0x8f7d_4016_15d9_5c4d`) is unchanged.
  - `WasmConfig { max_restarts: 0, .. }` reproduces today's fail-stop exactly (the existing
    sequences under that setting).

### Phase 3: The review nits
- **Owner skill:** dev
- **What:** the five items as the reviews worded them.
- **Files touched:** `apps/flythrough/src/main.rs` (rows buffer reused; `last_edit` taken
  by reference or swapped; the "4 Mi" comment), `apps/flythrough/src/hud.rs` (the ms
  formatter writes into a reused `String`; `WorldSample` gains `pool_arrays`;
  `cpu_bytes_estimate` adds `pool_arrays × 1024`; the "pool" row relabelled `gpu pool`;
  doc comments on `HudOverlay` and `scale`), `crates/vxe-render/src/marcher.rs`
  (`read_ms(&self, device)`), its callers (`main.rs`, `crates/vxe-render/tests/hud.rs`),
  `crates/vxe-world/src/shape.rs` (the `nearest` comment states the `i128` range),
  `apps/flythrough/tests/golden/hud.png` (re-blessed on lavapipe for the relabelled row).
- **Done when:**
  - A test in `apps/flythrough` (or `hud.rs`) renders 100 frames with the HUD shown and
    asserts, through a counting allocator or `HudOverlay::allocations()` counter, that
    frames 2 to 100 allocate 0 bytes in the HUD's update and encode.
  - `cpu_bytes_counts_full_brick_voxels_and_chunks` becomes `..._and_the_pool`: the estimate
    with 4096 pooled arrays is the old value plus 4 194 304.
  - `fixed_stats_format_to_their_rows` shows `gpu pool` on the pool row; `hud.png` is
    re-blessed and matches bit-exactly on lavapipe.
  - `cargo clippy --workspace --all-targets -- -D warnings` passes with `_queue` gone and
    `missing_docs` clean on `HudOverlay` and `scale`; the two comments read as the reviews
    asked.

### Phase 4: Rubble carries the rounding remainder across columns
- **Owner skill:** dev
- **What:** in `remove_region`'s column scan, a `u64` accumulator starts at 0; for each
  column with a carved voxel, `acc += n * fraction.0`, `h = acc / fraction.1`,
  `acc -= h * fraction.1` (both 0 when `fraction.1 == 0` or `rubble` is air, as today). A
  column with no ground within `max_depth`, or whose air runs out, loses the part of its
  `h` it cannot place; the remainder still carries. The doc comment states the new rule.
- **Files touched:** `crates/vxe-world/src/collapse.rs`, `crates/vxe-world/tests/collapse.rs`.
- **Done when:**
  - A new test removes a one-voxel-thick 8 x 8 sheet of material 3 (64 columns, `n = 1`
    each) held 10 voxels over a solid floor, with `(1, 4)` and `max_depth = 64`: `removed`
    is `{3: 64}` and `placed == 16`, one rubble voxel on the floor under each column where
    the accumulator reaches 4, that is local `lx` in `{3, 7}` for every `ly` in the
    cell's column order; the test lists the 16 voxels. Today's code places 0 there.
  - The existing pins stand unchanged: the slab piles 1152, the slab with one
    indestructible voxel 1151 (`floor(4607 / 4)`, as before), `(0, 1)` piles 0 and `(1, 1)`
    4608, and the depth test's 32 versus 33 split.
  - The cross-arm collapse hash test, if it pins a `remove_region` outcome on a region with
    a column of `n mod 4 != 0` other than the spared one, is re-pinned in this commit with
    the reason in the log; otherwise it is unchanged.

### Phase 5: Smooth scrolling changes the dig radius
- **Owner skill:** dev
- **What:** a `flythrough::ScrollSteps` holds an `f32` carry; `add(delta) -> i64` adds the
  frame's scroll, returns the whole steps toward zero and keeps the fraction. The
  fly-through uses it for the dig radius, prints the radius only when it changes, and
  clears the carry when the radius sits at either end of `DIG_RANGE` and the scroll
  pushes past it, so scrolling past a limit banks nothing.
- **Files touched:** `apps/flythrough/src/lib.rs`, `apps/flythrough/src/main.rs`.
- **Done when:**
  - A unit test feeds `ScrollSteps`: four deltas of `0.25` give `0, 0, 0, 1`; `0.3` then
    `-0.3` give `0, 0`; `2.0` gives `2`; `-0.6, -0.6` give `0, -1` and leave a carry of
    `-0.2` (within `1e-6`).
  - A test of the fly-through's radius handling (or of the function it calls) shows that
    32 frames of `0.125` lines (exact in `f32`) raise the radius from 0.5 m to 0.9 m, and that at 3.0 m
    further upward scroll prints nothing and a later downward line steps to 2.9 m at once.

### Phase 6: The owner rechecks in a window
- **Owner skill:** human
- **What:** the owner runs `flythrough --level` from their own terminal, scrolls with the
  device that failed and removes a thin region with `R` on.
- **Done when:** the dig radius follows the scroll in 0.1 m steps, and a removed thin region
  prints a rubble count near a quarter of its removed voxels (less only the columns with no
  ground within 6.4 m).

## Data shapes

```rust
// illustrative
pub struct MoveResult {
    // ... as today
    /// The offset the controller lifted the body by before sweeping, zero when none.
    pub lifted: Fixed3,
    /// The body overlapped solid at the start and no free pose was within reach.
    pub embedded: bool,
}
pub struct WasmConfig {
    // ... as today
    /// Fresh instances started after a failure before the logic stops for good.
    pub max_restarts: u32, // default 8
}
pub enum LogicError {
    // ... as today
    Restarted { tick: u64, after: u64, restarts: u32 },
}
```

## Budgets

- `nfr.md` §8 controller tick: an overlap test per tick (one `overlaps` call, about the
  cost of a sweep's box gather); the bench is re-read and must stay under the 20 µs line.
- `nfr.md` §4 component boundary: a restart costs an instantiation (the TTFF reading of
  Plan 0011: tens of milliseconds for the fly-through's component, read in the log), paid
  at most `max_restarts` times.
- `remove_region`: one integer add, divide and subtract per column, beside a column scan
  that already reads every voxel; no reading needed.
- Nothing else moves. The HUD's "no per-frame allocation" is a property, asserted in a test.

## Risks & open questions

- **Determinism of depenetration.** The candidate order is fixed and integer; the result
  depends only on the world and the body. The course hash is unchanged because the course
  never starts a tick overlapping (its harness asserts `!overlaps` after every tick).
- **A restart re-applies `init`'s ops.** Recorded in ADR-0010's new Outcome; the report
  makes every restart visible, and the cap bounds the damage. A consumer that wants
  fail-stop sets `max_restarts = 0`.
- **Restart cost under a trap loop.** Eight instantiations at most, then stop.
- **The HUD golden** changes for one label; it is re-blessed on lavapipe in the same commit.

## What this plan does NOT do

- A stress model for collapse. The owner's test found that collapse rarely triggers in the
  cave: a 0.8 m cell with 64 of 512 voxels solid still bears load, so a thin leftover bridge
  holds an overhang up. The owner chose (2026-10-17) overhangs that fail by span and weight,
  not only by disconnection: a new engine mechanism with its own ADR (ADR-0008's
  connectivity-only query is the alternative it would revise) and plan, recorded under M8
  in `docs/roadmap.md`. This plan does not tune the fly-through's `SUPPORT` constants.
- Snapshot restore of a logic's state (ADR-0010 rejected it; it needs its own ADR).
- Depenetration for bodies other than the controller's cylinder, or a notification to the
  game beyond `MoveResult`.
- Plan 0010's remaining followups (the gate after full residency) and Plan 0014's
  remaining items.

## Implementation log

> Written by `dev`: one row per phase as its commit lands, then the close block. **The phases
> above are the contract. This section records what happened.** Observations, never
> conclusions: no pass list, no self-assessment. Deviations and unmet done-whens are
> **always** disclosed. Keep it shorter than `## Implementation phases`.

| phase | owner | state | commit |
|---|---|---|---|
| 1: Depenetration in the controller | dev | done: `tick` tests `overlaps` first and takes the first free candidate; `MoveResult::lifted`, `embedded`; 2-voxel fill lifts `[0, 0, 512]` to `[2688, 2688, 512]`; 5-voxel fill lifts `[256, 0, 0]` (fill ending at x = 8) or `[0, 0, 1280]` (fill over the reach); enclosed body `embedded` twice, state unchanged; level walk: 16 fills, 16 lifts, no embed; course hash unchanged; F6 60-tick bench 108.17 to 110.59 µs against 108.35 to 111.60 µs at e23f309, same box, not quiet | committed with this row |
| 2: A trapped logic restarts from its seed | dev | not started | |
| 3: The review nits | dev | not started | |
| 4: Rubble carries the rounding remainder | dev | not started | |
| 5: Smooth scrolling changes the dig radius | dev | not started | |
| 6: The owner rechecks in a window | human | not started | |

### Notes

_(Deviations from the plan, with the commit, stated without justification. Done-whens not
satisfiable as stated, and what was done instead. Followups noticed and not acted on. One line
each. Empty is fine.)_

- Phase 1: the F6 60-tick bench read 108 to 111 µs, outside "within 10 % of 156 µs"; the base
  commit e23f309 read the same range in alternated runs on the same day (not a quiet box,
  load 13.4 to 5.2). `nfr.md` §8 carries both arms.
- Phase 1: an embedded tick returns before the climb, horizontal and vertical moves, so it
  leaves `vz`, `grounded` and the climb state as they were.
- Phase 1: the fly-through prints `walk: lifted out of solid` on each lifting tick and
  `walk: embedded` once per run of embedded ticks (`Walk::embedded`).

### Close triggers

_(Facts for the architect. No recommendations, and no suggested version bump.)_

- **What shipped:**
- **Public API changed:**
- **Budget readings:**
- **Gate at the tip:**
- **Outstanding `human` phases:**

## Followups

- A stress model for collapse (span and weight), its own ADR and plan under M8; see "What
  this plan does NOT do".
