# 0014: Edit cost under 2 ms, second pass: the allocation measured, a brick-array pool, cheaper sphere spans and an analytic capsule row

> **Status:** approved
> **Created:** 2026-10-16
> **Milestone:** M1 follow-up in `docs/roadmap.md` (the `nfr.md` §4 miss, carried from Plans
> 0001 and 0003)
> **Related ADRs:** [ADR-0002](../adrs/0002-brick-map-layout-and-per-chunk-locked-access.md)
> (brick layout and access model, unchanged here: a phase that would change the brick
> representation stops and goes to the architect as an ADR question). No new ADR.
> **Prerequisite:** Plan 0003 closed (its tip is this plan's baseline). `perf` installed on the
> dev box by the owner (`pacman -S perf`; `kernel.perf_event_paranoid` at its current 2 allows
> user-space sampling and fault counts of one's own processes).

## TL;DR

Plan 0003 cut the `worst` edit bench from 4.61 to 3.66 ms (read under load 10 to 14) against
the 2 ms figure in `nfr.md` §4, which the owner keeps as the target. Its close breakdown says
where the rest is: `Brick::expand` 36.5 % (almost all in libc under `Box::new`), `write_rows`
22.3 %, `Shape::row_span` 15.4 % with `isqrt` about 10 %, and the capsule's row scan making
`tool` no faster than before. That breakdown came from 260 gdb samples at 30 ms on a loaded
box. This plan measures first, on a quiet box with `perf`, what the allocation cost is
(allocator, the 1 KiB fill, or the first touch of fresh pages), then removes it with a pool of
brick arrays filled off the edit path, replaces the sphere's per-row square root with an exact
span walk, gives the capsule an analytic row interval, and adds the oracle run the Uniform
expand path was missing. The first thing a reader sees is the oracle on a world of uniform
bricks, then a profile that names the allocation's parts.

## Context & problem

`nfr.md` §4: 100 sphere carves of radius 1 m (4169 voxels each) in one frame at most 2 ms of
CPU, results and dirty marking included. Plan 0003's readings (AMD Ryzen 9 5900HS, the dev
box, release bench profile, exclusive perf lock): at its tip `worst` 3.656 ms, `mixed`
2.670 ms, `tool` 8.265 ms with the 1-minute load at 10 to 14; its Phase 3 tree read `worst`
3.120 ms on a quieter box. Every Plan 0003 reading after Phase 3 was taken with other sessions
loading the machine, by up to 30 % between runs of one binary, so the gap to 2 ms is not known
to better than about 1 to 1.7 ms.

What the close review found, from Plan 0003's log and code:

- **The allocation.** `Brick::expand` does `Box::new([m; 512])` for every `Uniform` brick a
  carve writes into. The bench builds a fresh solid world per iteration, so every carve
  expands its partial bricks from fresh heap. 35.0 % of `apply`'s samples sat in libc under
  that `Box::new`. A tunable that keeps the heap from being trimmed between iterations moved
  `worst` by about 1.5 %, so the cost is not heap trimming; whether it is the allocator's own
  path, the fill, or first-touch page faults on fresh heap is unmeasured, and the fix differs
  for each.
- **The sphere span.** `row_span` solves `isqrt(r² - dy² - dz²)` once per voxel row of a brick
  row; `isqrt` was about 10 % of `apply`.
- **The capsule row.** A row with no inside centre still scans its whole x window with the
  capsule test, so `tool` (100 capsules of radius 5 and length 30 voxels) read 8.27 ms against
  `worst`'s 3.66 although it carves fewer voxels: about `pi 25 x 30 + 4/3 pi 125 = 2880` voxel
  centres per op against 4169, with the same surface area (`2 pi 5 x 30 + 4 pi 25 = 4 pi 100`),
  so about the same bricks touched.
- **The oracle's blind spot.** `apply_matches_the_reference_over_seeded_ops` runs on a world
  of `hash(v) % 6`, where every brick is `Full`; the `Uniform` expand path, the one the bench
  spends a third of its time in, is checked only by the determinism vector.

The consumer requirement is `project-context.md`'s many small edits per frame at interactive
cost. `nfr.md` §6 (determinism) is the gate: nothing here may move a hash.

## Decision

We keep ADR-0002's representation (`Brick::Uniform(m)` or `Brick::Full(Box<[u16; 512]>)`), the
public API and every result, and change where arrays come from and how spans are found:

1. **Measure first** (Phase 1): `perf` at about 5 kHz on the three bench cases and a
   micro-bench that separates allocator, fill and first touch, plus page-fault counts.
2. **A brick-array pool** (Phase 2): the world keeps a bounded free list of boxed arrays.
   `expand` takes one from it, `collapse` and chunk eviction return theirs, and
   `World::reserve_brick_arrays(n)` tops it up off the edit path (the caller's choice of
   thread; the fly-through calls it after each frame's edits and the streaming workers after
   generation). The arrays keep their type, so the brick representation does not change. An
   empty pool falls back to `Box::new`, so correctness never depends on it.
3. **An exact span walk for the sphere** (Phase 3): each row's span starts from the previous
   row's ends and steps by squared-distance comparisons, with `isqrt` only for a brick row's
   first row.
4. **An analytic capsule row** (Phase 4): the row's nearest lattice point to the segment from
   the closed form, then each end found by bisection with the capsule test, so a row with no
   inside centre costs two tests.

We rejected a slab or arena with index handles (`Full(u32)` into a world-owned array store):
that changes ADR-0002's representation and every reader of a brick, and it is an ADR question
if Phase 1 shows the pool cannot reach the figure. We rejected `f64` square roots (`edit.rs`
and `shape.rs` stay integer). We rejected SIMD and threading one op across bricks for Plan
0003's reasons, which still hold.

## Architecture diagram

```mermaid
flowchart LR
    subgraph world["vxe-world (headless)"]
        OP[EditOp] --> RS[span: sphere walk<br/>capsule closed form + bisection]
        RS --> EB[edit_brick<br/>write rows]
        POOL[(brick-array pool<br/>bounded free list)] -->|expand takes| EB
        EB -->|collapse returns| POOL
        EV[chunk eviction] -->|returns| POOL
        RES[reserve_brick_arrays<br/>off the edit path] --> POOL
    end
```

## Implementation phases

Each phase ships as its own commit. `dev` implements all phases in one session with no review
between phases. The architect reviews once at the end, in a fresh session. Phase 1 is the
walking skeleton: the oracle that guards every later phase, and the measurement that orders
them.

Shared conventions:

- Plan 0003's conventions hold: integer shape math, no `f32` or `f64` in `edit.rs` or
  `shape.rs`, and every phase ends with the oracle tests (Plan 0003's and this plan's),
  `determinism.rs`'s `0x2b6b_6a0d_1f9b_8d75` and `raycast_hit_list_is_pinned`'s
  `0xa0f2_c477_8892_c8fb` green and unchanged. A phase that moves a constant has a defect.
- **A reading is taken on a quiet box:** the 1-minute load average from `uptime` under 2.0
  before and after the run, both recorded with it, under the exclusive perf lock. A run that
  misses that is recorded as diagnostic and does not count against a done-when. Comparisons
  between phases are paired runs of both binaries, alternated, as Plan 0003 did.
- Bench readings name the case, the mean, the confidence interval, the machine, the commit and
  the load.

### Phase 1: The uniform-brick oracle and the allocation measured
- **Owner skill:** dev
- **What:** the oracle on worlds of uniform bricks; a `perf` breakdown of the three cases at
  Plan 0003's tip; a micro-bench that splits the expansion's cost; counts per carve.
- **Files touched:** `crates/vxe-world/tests/edits.rs`, `crates/vxe-world/tests/support/mod.rs`,
  `crates/vxe-world/benches/edits.rs`.
- **Done when:**
  - `apply_matches_the_reference_on_uniform_worlds` runs Plan 0003's 200 seeded ops (same
    seed, shapes, modes and radii) in sequence on two worlds and checks every result, error and
    voxel against `reference_apply`: the bench's cubes world (`material = 1` where
    `((x >> 4) + (y >> 4) + (z >> 4)).rem_euclid(3) != 0`, else air; cube edges at multiples of
    16 voxels align with 8-voxel bricks, so every brick starts `Uniform`) and the solid world
    (`UniformSource(3)` over `[-2, 2)³`). The test asserts that before the first op no brick
    of either world is `Full`, and prints how many ops changed something on each.
  - The log records, at Plan 0003's tip on a quiet box: `perf record -F 4999 -e cycles:u
    --call-graph dwarf` on each of `worst`, `mixed` and `tool`, with the top ten symbols by
    self time inside `World::apply` and their percentages and sample counts; `perf stat -e
    task-clock,page-faults,cycles:u,instructions:u` per case, with user and system seconds.
  - A bench group `expand` times, per 1000 arrays: `Box::new([3; 512])` kept alive (fresh
    heap, as the bench sees it); the same with each box dropped before the next (the
    allocator's hot path); a 1 KiB fill into an existing array; and `Box::new` after the
    heap was pre-touched by allocating and freeing the same total. The log records the four
    means and states which share of `Brick::expand`'s time each explains.
  - The log records, for one `worst` carve and one `tool` carve, the bricks expanded, the
    bricks collapsed, the rows solved and the capsule tests per row, counted by instrumentation
    that is not committed.

### Phase 2: A brick-array pool
- **Owner skill:** dev
- **What:** `BrickArrays`, a bounded free list of boxed arrays owned by the world; `expand`
  takes from it, `collapse` and eviction give back, `reserve_brick_arrays` refills off the
  edit path. If Phase 1 puts the 1 KiB fill above 10 % of `worst`, `expand` also stops
  filling voxels the same brick's spans overwrite (a fused expand and write).
- **Files touched:** `crates/vxe-world/src/{brick.rs, edit.rs, store.rs, stream.rs, lib.rs}`,
  `crates/vxe-world/tests/{edits.rs, stream.rs}`,
  `crates/vxe-world/benches/edits.rs`, `apps/flythrough/src/main.rs` (the per-frame reserve).
- **Done when:**
  - The pool is one `Mutex<Vec<Box<[MaterialId; 512]>>>` in the world, locked at most twice
    per `apply` (take a batch sized by the op's partial-brick bound, return the unused rest
    and the collapsed arrays), with a cap of `World::BRICK_ARRAY_POOL_CAP` arrays (8192, 8 MiB);
    past the cap a returned array is freed. `pool_len()` is public for tests and the HUD.
  - `a_pooled_expand_allocates_nothing`: after `reserve_brick_arrays(4096)`, 100 radius-10
    carves on the solid world run with a counting global allocator in the test and allocate
    no `[u16; 512]`-sized block; the oracle tests and the two constants hold.
  - `an_evicted_chunk_returns_its_arrays`: a chunk with `F` `Full` bricks evicted from a
    residency, with no other holder of its `Arc`, raises `pool_len()` by `min(F, cap - len)`;
    a chunk still held elsewhere returns nothing and is freed as today when the last holder
    drops it.
  - The bench gains `worst_warm` (the `worst` case with `reserve_brick_arrays` called in the
    untimed setup, sized from Phase 1's expansions per carve times 100) and keeps `worst`
    cold. Both read on a quiet box, paired with Phase 1's tree, and the log records them.

### Phase 3: An exact span walk for the sphere
- **Owner skill:** dev
- **What:** `row_span` for the sphere walks from the previous row's ends by squared-distance
  steps; `isqrt` remains for the first row of each brick row and as the test's reference.
- **Files touched:** `crates/vxe-world/src/{shape.rs, edit.rs}`,
  `crates/vxe-world/tests/shapes.rs`.
- **Done when:**
  - `row_span_equals_the_set_of_contained_centres` (Plan 0003's 1000 seeded rows) passes
    unchanged, and a new `walked_spans_equal_solved_spans` asserts, for 200 seeded spheres
    (radius 1 to 40 voxels, centres at arbitrary fixed-point positions within ±64 voxels), that
    every row of every brick row walked in `edit_brick`'s order gives exactly the `isqrt`
    span, including the boundary `|256 x + 128 - cx| == h` inside.
  - A quiet-box `perf` profile of `worst_warm` at this phase puts the sphere's span work
    (`row_span` and what it inlines) under 5 % of the samples inside `World::apply` (15.4 % at
    Plan 0003's close); the log records it.

### Phase 4: An analytic capsule row
- **Owner skill:** dev
- **What:** `CapsuleRow` finds the row's lattice point nearest the segment from the closed
  form, returns `None` if that point is outside, and otherwise bisects each end with the
  capsule test.
- **Files touched:** `crates/vxe-world/src/shape.rs`, `crates/vxe-world/tests/shapes.rs`.
- **Done when:**
  - The row along `x` at fixed `(y, z)` is a line, so the real `x` minimising the squared
    distance to the segment has a closed form (the line-to-segment closest point, clamped to
    the segment's ends), computed in `i128`; the integer minimiser is its floor or ceiling
    (clamped to the window), because the squared distance along the row is convex. A row
    costs at most `2 + 2 ceil(log2(w))` capsule tests for a window of `w` voxels, and 2 when
    no centre is inside.
  - Plan 0003's 1000-row property test passes unchanged, and a new 1000-row capsule-only
    property test (radius 1 to 40, ends within ±64 voxels, including segments parallel to `x`
    and degenerate segments of length 0) gives exactly the `contains` set.
  - `tool_warm` reads no more than `worst_warm` in the same quiet-box run (the capsules carve
    fewer centres, about 2880 against 4169, over the same surface area); the log records both,
    or the breakdown if it does not.

### Phase 5: Readings against the figure
- **Owner skill:** dev
- **What:** the final quiet-box readings, the fly-through's in-frame figure, and `nfr.md` §4.
- **Files touched:** `docs/nfr.md`.
- **Done when:**
  - `cargo bench -p vxe-world --bench edits` on a quiet box reads `worst_warm` under 2.0 ms,
    and the log records `worst`, `worst_warm`, `mixed`, `tool` and `tool_warm` with confidence
    intervals, load, machine and commit, each paired with Plan 0003's tip. If `worst_warm`
    reads 2.0 ms or more, the log records the Phase 1 profile repeated at the tip and the plan
    stops there without widening the figure: the close decides between the brick
    representation ADR question and an `nfr.md` revision.
  - `cargo run --release -p flythrough -- --level --seed 1 --headless --bench 1 --frames 1`
    (100 radius-10 carves near the spawn, the reserve called before the bench): the log records
    the `bench:` line's edit CPU time, three runs, paired with Plan 0003's tip, and the
    `carved:` line, which must equal Plan 0003's (`rock 340502, ore 126 (39 chunks)`).
  - `nfr.md` §4 carries `worst_warm` and `worst` as dev-reference readings with machine,
    commit and load, and says that the warm figure assumes the reserve runs off the edit path.

## Data shapes

```rust
// illustrative: vxe-world
pub struct World { /* … */ brick_arrays: Mutex<Vec<Box<[MaterialId; BRICK_VOXELS]>>> }
impl World {
    pub const BRICK_ARRAY_POOL_CAP: usize = 8192;      // 8 MiB of u16 arrays
    pub fn reserve_brick_arrays(&self, n: usize);       // tops the pool up to min(n, cap)
    pub fn pool_len(&self) -> usize;
}
// Brick is unchanged: Uniform(MaterialId) | Full(Box<[MaterialId; 512]>).
```

No new dependency. `perf` is a tool on the dev box, not a crate.

## Budgets

| `nfr.md` figure | Expected effect | How the close measures it |
|---|---|---|
| §4 edit cost (2 ms per 100 radius-1 m carves) | From Plan 0003's 3.1 to 3.7 ms (load-dependent) to under 2 ms for `worst_warm`. Arithmetic on Plan 0003's tip breakdown: the pool removes most of `Brick::expand`'s 36.5 % if first touch and the allocator are the cost (about 1.2 ms), the span walk most of `row_span`'s 15.4 % (about 0.4 ms); that lands near 2 ms, a guess until Phase 1 says what the 36.5 % is | `cargo bench -p vxe-world --bench edits` on a quiet box, paired with Plan 0003's tip; the fly-through's `--bench` line |
| §3 world RAM (4 GB; 3.87 GB peak RSS read by Plan 0002) | The pool adds at most 8 MiB | `pool_len()` times 1 KiB, printed by the fly-through's `bench:` line |
| §6 determinism | Unchanged, asserted | both constants on both CI arms; the oracle tests |

The baseline CPU is not on hand; readings are dev reference. Per Plan 0003, the dev box's
single-thread speed is roughly 1.5 to 2 times the baseline's, so a 2 ms reading here projects
to 3 to 4 ms there; a guess until the baseline is measured.

## Risks & open questions

- **The cost may be first touch, not malloc.** If Phase 1 shows the time is page faults on
  fresh heap, only a pool filled and touched before the frame helps, and `worst` cold stays
  slow by construction; the figure then rests on `worst_warm`, with the reserve's cost on
  another thread named in `nfr.md`. The owner decides at the close whether that reading
  satisfies the figure.
- **The pool is shared state behind a lock.** Two `apply` calls on different chunks contend on
  it twice each; a contended lock costs microseconds, not milliseconds, and the lock is never
  held across brick work. It never affects results, so determinism is untouched.
- **The brick representation may be the floor.** If the pool, the walk and the capsule row
  land above 2 ms and the profile puts the rest in `write_rows`, the next lever is the
  representation (a palette or bitmask brick, or arena handles). That supersedes part of
  ADR-0002 and touches the mirror's narrowing; it is an ADR question for the architect at the
  close, not a phase here.
- **Overflow in the capsule closed form.** The line-to-segment terms are products of
  fixed-point differences up to about 2^40 (window and segment within a few chunks); `i128`
  holds their products. The property test includes the extremes of its ranges.

## What this plan does NOT do

- **Change the brick representation** (arena handles, palette or bitmask bricks): an ADR
  question if Phase 5 misses.
- **An allocation-free `EditResult`**: Plan 0003's followup, if the frame path shows it.
- **SIMD, threading an op across bricks**: rejected by Plan 0003 and still rejected.
- **The upload cost of edits** and the streaming line: other plans.
- **Baseline readings**: `nfr.md` §1, M2 close.

## Implementation log

> Written by `dev`: one row per phase as its commit lands, then the close block. **The phases
> above are the contract. This section records what happened.** Observations, never conclusions:
> no pass list, no self-assessment. Deviations and unmet done-whens are **always** disclosed.
> Keep it shorter than `## Implementation phases`.

| phase | owner | state | commit |
|---|---|---|---|
| 1: The uniform-brick oracle and the allocation measured | dev | not started | |
| 2: A brick-array pool | dev | not started | |
| 3: An exact span walk for the sphere | dev | not started | |
| 4: An analytic capsule row | dev | not started | |
| 5: Readings against the figure | dev | not started | |

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

- Allocation-free result path (Plan 0003's followup), if the frame path shows it.
