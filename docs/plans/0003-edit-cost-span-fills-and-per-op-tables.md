# 0003: Edit cost under 2 ms: row spans, a per-op target table and per-brick tallies

> **Status:** in-progress
> **Created:** 2026-10-16
> **Milestone:** M1 follow-up in `docs/roadmap.md` (the `nfr.md` §4 miss recorded by Plan 0001)
> **Related ADRs:** [ADR-0002](../adrs/0002-brick-map-layout-and-per-chunk-locked-access.md) (layout and access model, unchanged by this plan). No new ADR: the choices here are implementation choices with no architecture-level alternative worth a record, and the rejected ones are named under "Decision".
> **Prerequisite:** Plan 0002 closed, as the Plan 0001 close decided (`nfr.md` §4: "the edit cost gets its own plan after Plan 0002"). Nothing here depends on Plan 0002's readings; the sequencing avoids a merge in `edit.rs`, which Plan 0002 Phases 1 and 3 also change.

## TL;DR

Plan 0001's edit bench reads 4.92 ms for 100 radius-10 carves on a fresh solid world against
the 2 ms figure in `nfr.md` §4. We rewrite the inner loop of `World::apply` without changing one
result: each brick row is written as a span of voxels computed from the shape (analytically for
sphere and box, by a convexity scan for the capsule), the per-voxel material-table lookup
becomes a per-op lookup table, and counts and bounds accumulate per brick instead of per voxel.
The first thing a reader sees is the oracle test: 200 seeded ops of every shape and mode produce
the same voxels and the same results from the new path as from a brute-force reference kept in
test support, and the determinism constant `0x2b6b_6a0d_1f9b_8d75` does not move. The close reads
the bench under 2 ms on the dev box, or records the breakdown that says why not.

## Context & problem

`nfr.md` §4 asks for 100 sphere carves of radius 1 m (4169 voxels each) in 2 ms of CPU, results
and dirty marking included. Plan 0001 Phase 4 measured 4.92 ms (confidence interval 4.87 to
4.98 ms, AMD Ryzen 9 5900HS, release bench profile) on the worst case: every carve expands
`Uniform` bricks. That is 49 µs per carve; the budget is 20 µs. The fly-through's `B` key,
carving into an already dug cave, read 3.07 to 4.63 ms over about 20 runs (Plan 0001 close),
so the realistic case misses too. The owner kept the figure at the Plan 0001 close and asked
for this plan.

Reading `crates/vxe-world/src/edit.rs` at the Plan 0001 tip, one radius-10 carve does the
following work. The figures are counts from the geometry, not measurements; Phase 1 takes the
measurement.

- The bounding box holds 9261 voxel centres (21³); 4169 lie inside. The sphere enters 32
  bricks, and its bounding box reaches up to 64, each tested for coverage.
- For every candidate centre in a `Partial` brick: `Shape::contains` (an `i64` squared distance
  for the sphere; an `i128` segment distance for the capsule), then `Mode::target`, which calls
  `MaterialTable::get` and reads the flags, then `brick.expand()` (a `match` on every call),
  then `Tally::add`, which searches `counts` linearly and updates six bounds.
- `Brick::collapse` scans all 512 voxels of every changed brick.
- Per op: a `Vec` of `(coord, bounds, Arc)` for the chunks, the `touched` `Vec`, and the result.

The consumer requirement is `project-context.md` "The first consumer", Edits: many small edits
per frame at interactive cost. The `nfr.md` figure touched is §4 (cost). §6 (determinism) is
the gate: this plan may not move a hash.

## Decision

We keep ADR-0002's layout, the public API, and every observable result, and change only how
`apply` walks a brick:

1. **A per-op target table.** `Mode` resolves once per op against the material table into a
   table indexed by old material: for `Carve`, a bit per material row saying whether it carves
   (non-air and not indestructible); for `Fill(m)`, "air becomes `m`"; for `Replace`, the pair.
   The inner loop reads one bit or one compare instead of calling into the table.
2. **Row spans.** Within a brick, for every row (fixed `y`, `z`), the shape gives the inclusive
   range of voxel indices whose centres lie inside. The sphere solves `dx² <= r² - dy² - dz²`
   with an integer square root; the box clips; the capsule, being convex, scans the row from
   both ends with `contains` and stops at the first inside voxel on each side. The row is then
   written as a contiguous slice, x fastest, reading each old value once.
3. **Per-brick tallies.** Counts go into an array indexed by material id (sized to the table,
   zeroed once per op) plus a list of ids seen; bounds come from the span extents, not from
   every voxel. The result is assembled once at the end in ascending material order, as now.
4. **One expansion per brick** before its rows are written, and `collapse` only on bricks that
   changed.

We rejected explicit SIMD (platform-specific code in the deterministic crate; the scalar loop
has to reach the figure first, and the compiler vectorises contiguous `u16` loops). We rejected
threading a single op across bricks (a 1 m carve is at most 64 bricks; thread hand-off costs
more than the work, and `apply` is called from a game tick that may already be parallel). We
rejected changing the brick representation (a bitmask-plus-palette brick is an ADR-0002
supersession and a renderer change; nothing in the breakdown says the `u16` array is the cost).

## Architecture diagram

```mermaid
flowchart LR
    subgraph world["vxe-world (headless)"]
        OP[EditOp] --> TT[TargetTable<br/>per op, from MaterialTable]
        OP --> RS[Shape::row_span<br/>y, z -> x0..=x1]
        TT --> EB[edit_brick<br/>expand once, write rows]
        RS --> EB
        EB --> BT[BrickTally<br/>counts by id, bounds]
        BT --> ER[EditResult<br/>changed_from, touched, aabb]
    end
```

## Implementation phases

Each phase ships as its own commit. `dev` implements all phases in one session with no review
between phases. The architect reviews once at the end, in a fresh session. Phase 1 is the
walking skeleton: the oracle that every later phase is checked against.

Shared conventions for every phase:

- Plan 0001's conventions hold: coordinates, fixed point at 1/256 voxel, integer shape tests,
  the "solid material-3 world" from `UniformSource(3)` over chunks `[-2, 2)³`, the test table
  with rows 0 (air), 1 to 5, and 4 flagged indestructible, FNV-1a 64 hashes.
- **Every phase ends with the same three assertions green and unchanged:** the oracle test of
  Phase 1, `apps/flythrough/tests/determinism.rs` with its constant `0x2b6b_6a0d_1f9b_8d75`, and
  `raycast_hit_list_is_pinned` with `0xa0f2_c477_8892_c8fb`. A phase that moves either constant
  has a defect; the constant is not re-blessed.
- **The edit path keeps Plan 0002's side effects** (amended at Plan 0002's close): `apply`
  holds the store map's read lock for the whole op, marks the store's dirty-chunk set and
  sets `Chunk::is_edited` on every chunk it changes. `crates/vxe-world/tests/stream.rs` and
  `crates/vxe-world/tests/persist.rs` stay green and unchanged in every phase.
- No `f32` or `f64` enters `edit.rs` or `shape.rs`. `i64::isqrt` (stable since Rust 1.84) is
  the square root.
- The bench is run with `cargo bench -p vxe-world --bench edits` on the dev box; every reading
  in the log names the case, the mean, the confidence interval, the machine and the commit.

### Phase 1: The breakdown and the oracle
- **Owner skill:** dev
- **What:** two more bench cases next to the existing worst case, a measured breakdown of
  where the 49 µs go, and a brute-force reference implementation in test support that every
  later phase is checked against.
- **Files touched:** `crates/vxe-world/benches/edits.rs`, `crates/vxe-world/tests/edits.rs`,
  `crates/vxe-world/tests/support/mod.rs`.
- **Done when:**
  - The bench has three cases: `worst` (the Plan 0001 case: 100 radius-10 carves at seeded
    centres on a fresh solid world), `mixed` (the same 100 carves on a world from the closure
    source `material = if ((x >> 4) + (y >> 4) + (z >> 4)).rem_euclid(3) != 0 { 1 } else { 0 }`,
    so two thirds of the volume is rock in 1.6 m cubes and the carves meet `Full`, uniform-air
    and uniform-rock bricks), and `tool` (100 capsule carves of radius 5 voxels and length 30
    voxels at seeded centres and seeded directions on the fresh solid world). All three read on
    the dev box at the Plan 0001 tip and the log records them.
  - The log records a breakdown of the `worst` case at the Plan 0001 tip: the top five symbols
    by self time from `perf record` on the bench binary (or an equivalent sampling profiler),
    with their percentages. The instrumentation is not committed.
  - `tests/support/mod.rs` has `reference_apply(&World, &EditOp) -> Expected`, a brute-force
    model that walks every voxel centre of the shape's voxel bounds, tests `Shape::contains`,
    applies the mode by a direct `MaterialTable` lookup, and produces the expected
    `changed_from`, `touched`, `aabb`, the expected error, and the expected material of every
    voxel in the bounds after the op. It never calls `World::apply`.
  - `tests/edits.rs::apply_matches_the_reference_over_seeded_ops`: on a world over `[-2, 2)³`
    from the closure source `material = hash(v) % 6` (a SplitMix64-seeded integer hash of the
    coordinate, so bricks are `Full` and all six materials occur), 200 ops seeded by
    `SplitMix64(0x0003_0001)` cycle through sphere, box and capsule and through `Carve`,
    `Fill(m)` and `Replace { from, to }`, with radii from 1 to 12 voxels and centres within
    ±100 voxels of the origin; at least five ops are placed so they reach a non-resident chunk.
    For every op, `World::apply` returns exactly the reference's result or error, and every
    voxel in the op's bounds reads the reference's material afterwards. The test prints how
    many ops changed something; it is at least 150.

### Phase 2: The per-op target table
- **Owner skill:** dev
- **What:** `Mode` resolves once per `apply` into a `TargetTable`; the inner loop consults it
  instead of `MaterialTable`.
- **Files touched:** `crates/vxe-world/src/{edit.rs, material.rs}`,
  `crates/vxe-world/tests/edits.rs`.
- **Done when:**
  - `TargetTable::carve(&MaterialTable)` holds one bit per table row; row `m` is set when
    `m != AIR` and `m` is not indestructible. A unit test on the test table asserts bits 1, 2,
    3 and 5 set and 0 and 4 clear, and that a material id beyond the table's rows reads clear.
  - `Mode::target` is no longer called per voxel; `edit_brick` takes `&TargetTable`. A `Uniform`
    brick whose material has no target is skipped before any coverage test on its voxels, and a
    `Full` brick with `Inside` coverage whose 512 voxels all have no target (an all-indestructible
    brick under `Carve`) is left untouched with no write.
  - The three shared assertions hold. The bench reads on the dev box and the log records them.

### Phase 3: Row spans
- **Owner skill:** dev
- **What:** `Shape::row_span` and a row-wise `edit_brick` that writes contiguous `x` runs.
- **Files touched:** `crates/vxe-world/src/{shape.rs, edit.rs}`,
  `crates/vxe-world/tests/{edits.rs, shapes.rs}`.
- **Done when:**
  - `Shape::row_span(y, z, x_lo, x_hi) -> Option<(i64, i64)>` returns the inclusive range of
    voxel `x` indices within `x_lo..=x_hi` whose centres `(256 x + 128, 256 y + 128,
    256 z + 128)` lie inside, or `None`. For the sphere: `rem = r² - dy² - dz²`; `None` when
    `rem < 0`; otherwise `h = isqrt(rem)` and the range is the voxels with
    `|256 x + 128 - cx| <= h`, computed with `div_euclid` and no float. For the box, the clip
    of the half-open fixed range to voxel indices. For the capsule, a scan inward from both ends
    of `x_lo..=x_hi` using `contains`, which is exact because the capsule is convex.
  - `tests/shapes.rs::row_span_equals_the_set_of_contained_centres`: for 1000 SplitMix64-seeded
    `(shape, y, z)` triples over all three shapes (radii 1 to 40 voxels, random centres and
    segment ends within ±64 voxels), the span equals the exact set `{x in x_lo..=x_hi :
    contains(centre(x, y, z))}`: both `None` when the set is empty, and otherwise the set is
    exactly the range (no gaps, no extras). The boundary case `|256 x + 128 - cx| == h` is
    inside.
  - `edit_brick` iterates rows, takes the span, expands the brick once if any row has a span and
    a target, and writes each span as a slice. The per-voxel work is: read old, look up the
    target, write new, count old.
  - The three shared assertions hold. The bench reads on the dev box and the log records them.

### Phase 4: Per-brick tallies and one collapse
- **Owner skill:** dev
- **What:** counts and bounds accumulate per brick into fixed storage; `collapse` runs once per
  changed brick; nothing in the loop allocates.
- **Files touched:** `crates/vxe-world/src/{edit.rs, brick.rs}`, `crates/vxe-world/tests/edits.rs`.
- **Done when:**
  - `Tally` holds `counts: Vec<u32>` sized to `table.len()` (allocated once per op) and `seen:
    Vec<MaterialId>`; `EditResult::changed_from` is built from `seen` sorted ascending, with the
    counts, and equals what Plan 0001 produced (the oracle and the determinism constant say so).
  - Bounds come from span extents: per row, the span's first and last voxel; per brick, the
    rows written. A brick that changed nothing contributes nothing.
  - `Brick::collapse` is called once per changed brick, and `Brick::expand` once per brick that
    gets a write.
  - The three shared assertions hold. The bench reads on the dev box and the log records all
    three cases with the before and after for this phase.

### Phase 5: Readings against the figure
- **Owner skill:** dev
- **What:** the final readings, the in-frame figure from the fly-through, and `nfr.md` §4.
- **Files touched:** `docs/nfr.md`, `apps/flythrough/src/main.rs` (only if the `B` key's line
  needs the upload bytes split out; otherwise untouched).
- **Done when:**
  - `cargo bench -p vxe-world --bench edits` on the dev box reads `worst` under 2.0 ms (the
    `nfr.md` §4 figure), and the log records `worst`, `mixed` and `tool` with confidence
    intervals, machine and commit. If `worst` reads 2.0 ms or more, the log records the
    Phase 1 breakdown repeated at the tip and the plan stops here without widening the figure:
    the close decides between a follow-up and an `nfr.md` revision with the reading.
  - `cargo run --release -p flythrough -- --level --seed 1`, key `B` or the `--bench` flag
    (100 radius-10 carves within 10 m of the spawn, on the bounded level so the figures compare
    with Plans 0001 and 0002; amended at Plan 0002's close, since the fly-through now streams
    by default): the log records the `bench:` line's edit CPU time and sync bytes on the dev
    box, before and after this plan.
  - `nfr.md` §4 carries the `worst` reading as a dev-reference reading under the figure, with
    machine and commit, replacing nothing (the figure stays 2 ms).

## Data shapes

```rust
// illustrative: vxe-world, all private to edit.rs and shape.rs
enum TargetTable {
    Carve { carves: Vec<u64> },             // bit m set: material m becomes AIR
    Fill(MaterialId),                        // AIR becomes m
    Replace { from: MaterialId, to: MaterialId },
}
impl TargetTable {
    fn target(&self, old: MaterialId) -> Option<MaterialId>;   // None: unchanged
}

impl Shape {
    fn row_span(&self, y: i64, z: i64, x_lo: i64, x_hi: i64) -> Option<(i64, i64)>;
}

struct Tally { counts: Vec<u32>, seen: Vec<MaterialId>, lo: [i64; 3], hi: [i64; 3], any: bool }
```

No public type changes. No new dependency.

## Budgets

| `nfr.md` figure | Expected effect | How the close measures it |
|---|---|---|
| §4 edit cost (2 ms per 100 radius-1 m carves) | From 4.92 ms toward under 2 ms on the `worst` case. The estimate behind the target: per carve, 64 coverage tests at about 20 ns, 32 expansions at about 100 ns each (allocation and a 1 KiB fill), 441 row solves at about 10 ns, 4169 voxel writes at 1 to 2 ns, about 15 µs; this is a guess, and Phase 1's breakdown replaces it | `cargo bench -p vxe-world --bench edits`, three cases, dev box, before and after each phase; the fly-through's `B` key |
| §6 determinism | Unchanged, asserted | the determinism vector and the pinned raycast hash on both CI arms; the oracle test |
| §2 streaming line (upload volume of 100 carves) | Unchanged: the same bricks go dirty | not re-measured; the `B` line's sync bytes are recorded for the record |

The baseline CPU (`nfr.md` §1, 4 cores) is not on hand; the readings are dev reference and say
so. The figure is a per-frame CPU cost on one thread, so the dev box's single-thread speed is
the relevant comparison and is roughly 1.5 to 2 times the baseline's; a 1.5 ms reading on the
dev box projects to about 3 ms there. That projection is a guess until the baseline is measured.

## Risks & open questions

- **The figure may not be reachable with the `u16` array brick.** If Phase 4 lands above
  2 ms, the breakdown says whether the time is in the writes (then the representation is the
  cost and a follow-up ADR considers palette bricks) or in the shape math (then the span solve
  needs work). The plan does not widen the figure; the close decides.
- **Boundary exactness of the span.** `isqrt` floors; the inclusive test `|dx| <= h` must agree
  with `dx² <= rem` for every integer `dx`, which holds because `dx² <= rem` iff `|dx| <=
  isqrt(rem)` for non-negative integers. The Phase 3 property test over 1000 rows is the
  check, and the oracle test is the second.
- **Overflow.** `r² - dy² - dz²` is `i64`; with `r` up to 2³¹ fixed units (8 million voxels) the
  squares fit. The capsule keeps its `i128` path.
- **Determinism.** Integer only, same iteration order (bricks ascending within a chunk, chunks
  ascending), same result assembly. The two pinned constants are the gate.
- **The bench measures allocation too.** `iter_batched` builds a fresh world per iteration
  outside the timed region; the timed region still allocates 32 bricks per carve, which is
  real cost the figure includes.

## What this plan does NOT do

- **An allocation-free `EditResult`** (a caller-provided scratch): follow-up if the frame path
  shows it.
- **`reaches` for non-resident chunks**: it runs only on the failure path.
- **The upload cost of edits** (Plan 0002 Phase 3 batches writes).
- **Parallel ops, SIMD, a new brick representation**: named rejected above.
- **The baseline reading**: `nfr.md` §1, M2 close.

## Implementation log

> Written by `dev`: one row per phase as its commit lands, then the close block. **The phases
> above are the contract. This section records what happened.** Observations, never conclusions:
> no pass list, no self-assessment. Deviations and unmet done-whens are **always** disclosed.
> Keep it shorter than `## Implementation phases`.

| phase | owner | state | commit |
|---|---|---|---|
| 1: The breakdown and the oracle | dev | done | committed with this row |
| 2: The per-op target table | dev | not started | |
| 3: Row spans | dev | not started | |
| 4: Per-brick tallies and one collapse | dev | not started | |
| 5: Readings against the figure | dev | not started | |

### Notes

- Phase 1: `Shape::voxel_bounds` and `Shape::contains` went from `pub(crate)` to `pub` so the
  test-support reference can call them; `crates/vxe-world/src/shape.rs` is outside Phase 1's
  Files touched.
- Phase 1: the profile is from `eu-stack -i` sampling the bench (`--profile-time 25`, release
  bench profile with debuginfo, built in a scratch target dir); `perf` and `samply` are not
  installed. 235 main-thread samples, 187 inside `World::apply`.

### Close triggers

_(Facts for the architect. No recommendations, and no suggested version bump.)_

- **What shipped:**
- **Public API changed:**
- **Budget readings:** (all `cargo bench -p vxe-world --bench edits`, AMD Ryzen 9 5900HS dev
  box, release bench profile, taken under the exclusive perf lock; mean, CI in brackets)
  - Phase 1, edit path of `486ea9c` (the Plan 0001 path as amended by Plan 0002): `worst`
    4.613 ms [4.568, 4.667]; `mixed` 3.103 ms [3.090, 3.116]; `tool` 8.287 ms [8.254, 8.324].
  - Phase 1 breakdown of `worst` at `486ea9c`, self time over the 187 samples inside
    `World::apply` (inlined frames attributed to the inlined function): libc, all under
    `System::alloc`, 20.3%; `RangeInclusive<i64>::next` (the voxel loops) 12.3%;
    `edit_brick` 9.6%; `Mode::target` 9.1% plus `bool::then_some` 9.1%; `Tally::add` 7.0%;
    `i64::lt` 7.0%; `Shape::contains` 4.3%.
- **Gate at the tip:**
- **Outstanding `human` phases:**

## Followups

- Allocation-free result path, if the frame-path rule bites.
