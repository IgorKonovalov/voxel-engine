# 0022: A falling region crushes the sub-threshold voxels under it, and the logic tells a wedge from a refusal and a landing

> **Status:** draft
> **Created:** 2026-10-20
> **Milestone:** M8 (collapse) in `docs/roadmap.md`
> **Related ADRs:** [ADR-0016](../adrs/0016-a-falling-region-crushes-the-sub-threshold-voxels-in-its-path.md) (the yield rule, crush over carry; proposed with this plan), [ADR-0008](../adrs/0008-collapse-as-capped-support-flood-and-rigid-vertical-drop.md) (the flood and the drop it amends), [ADR-0007](../adrs/0007-server-op-log-byte-channel-shared-and-owned-clients.md) (the log), [ADR-0010](../adrs/0010-game-logic-as-a-component-behind-a-logic-trait.md) (the logic boundary)
> **Source:** the owner's windowed check of Plan 0018 on 2026-10-20, recorded in
> [Plan 0018's implementation log](done/0018-server-routed-stamp-drop-and-remove.md) at 84a3998,
> and the owner's choice (a) the same day.

## TL;DR

In `flythrough --level` no unsupported region ever falls: each one reports `landed after 0
ticks, fell 0 voxels`. The support query says a cell with fewer than `solid_min` (64) solid
voxels holds nothing up, but `drop_region` stops at any solid voxel below, so a region resting
on a thin sliver is unsupported and wedged at once. This plan makes the drop agree with the
query. `World::drop_region_through(cells, max_drop, yield_below)` lets the falling region
pass through and crush every non-`INDESTRUCTIBLE` voxel whose cell holds fewer than
`yield_below` solid voxels, and reports them in `DropResult::crushed`. On the wire, a drop
with a threshold is op kind 5; kind 3 keeps its bytes and meaning, so Plan 0018's saves replay
unchanged. `DigLogic` passes the query's `solid_min` and tells three outcomes apart: a refusal
(`drop refused: ...`), a region that cannot move (`wedged ...`, remembered so it is not reported
again until it changes) and a landing. The first thing a reader sees is a committed test on the
seed-1 level. Today it pins the bug: the first region the repro's digs detach (4 cells, 501
voxels) drops 0. After Phase 2 the same region falls, and the test says how many voxels it
crushed.

## Context & problem

Plan 0009 shipped both halves of collapse (ADR-0008). The flood skips a cell whose `solid` is
under `solid_min` (`crates/vxe-world/src/support.rs`, the two `cell.solid < params.solid_min`
checks in `find_unsupported`). `Snapshot::gap_below` (`crates/vxe-world/src/collapse.rs`)
ends a column's gap at the first solid voxel outside the region, and `drop_region_with`
returns `DropResult::default()` when any column's gap is 0. A region the query reports therefore
always rests on cells the query called too weak to hold it, and in the generated cave those
cells nearly always hold some voxels.

The headless repro (40 000 seeded capsule carves of radius 5 voxels within 200 voxels of the
seed-1 level's spawn, `SupportParams { solid_min: 64, max_region: 4096, boundary_anchors: true
}`, every reported region dropped by 8) finds six unsupported regions in about 6 s (release,
dev box). All six drop 0 and none is refused. They are the same piece reported again as
later digs nibble it: loop iteration 3 243 finds 4 cells, 501 voxels, aabb `[553, 794, 80]` to
`[566, 803, 87]`, resting on cells `[69..70, 99..100, 9]` that hold 31 to 38 solid voxels each.
Iterations 5 786, 7 558 and 12 205 find it again with 469, 469 and 468 voxels. So the bug has a
second half: `DigLogic::fall` (`apps/flythrough/logic/src/lib.rs`) maps a refusal, a zero drop
and a landing to the same `landed` line, and nothing stops the next dig from reporting the same
stuck region again.

Crumble (`Op::Remove`) works because it does not scan for a landing. The consumer requirement is
emergent, authoritative collapse (`docs/roadmap.md`, "Physics"; `project-context.md`, collapse).
Figures touched: `docs/nfr.md` §4 (collapse cost, log entry sizes) and §6 (the collapse course
hash and the session hashes).

## Decision

The drop gains a yield threshold and crushes what yields (ADR-0016). A solid voxel outside the
region yields when its cell holds fewer than `yield_below` solid voxels (counted from the brick
when the op starts) and its material is not `INDESTRUCTIBLE`. The gap scan passes through a
yielding voxel as if it were air. In each footprint column, every yielding voxel in the span
below each region run, from the run's lowest voxel down by `dropped`, is crushed: it becomes
air or is overwritten by the landed region. `yield_below == 0` is exactly today's drop.
`drop_region(cells, max_drop)` keeps its signature and means `drop_region_through(cells,
max_drop, 0)`. The fly-through's logic passes `vxe_net::DEFAULT_SUPPORT.solid_min`, so its drops
agree with the server's query.

We chose crush over carry because carried voxels would sit in cells the logic does not name on
the next tick (it re-sends the region's cells one cell lower after each whole step). Carry would
change the op's contract for a cosmetic gain. The owner rejected (b), adding the touching
sub-threshold cells to the region, because it changes what every region means and depends on
visiting order. The owner also rejected (c), a logic-only crumble of a wedged region, because it
hides the engine's disagreement behind one game's policy.

## Architecture diagram

```mermaid
sequenceDiagram
    participant L as DigLogic (flythrough-logic)
    participant H as Server host (vxe-net)
    participant W as World::drop_region_through (vxe-world)
    participant G as op log (vxe-world)
    L->>H: apply(Op::Drop{cells, max_drop: 8, yield_below: 64})
    H->>W: drop_region_through
    W-->>H: DropResult{moved, dropped, crushed, touched, aabb}
    H->>G: record (kind 5; kind 3 when yield_below == 0)
    H-->>L: Ok(OpResult::Drop) or Err(EditError)
    alt Err
        L->>L: "drop refused: ..." and stop
    else dropped == 0 on the first tick
        L->>L: "wedged ..." and remember (cells, counts)
    else dropped < 8
        L->>L: "landed after N ticks, fell M voxels, crushed K voxels"
    else dropped == 8
        L->>L: cells one cell lower, fall on
    end
```

## Implementation phases

Each phase ships as its own commit. `dev` implements all phases in one session with no review
between phases. The architect reviews once at the end, in a fresh session.

Fixture terms below: **F** is the collapse fixture of `crates/vxe-world/tests/collapse.rs`: one
chunk `[0, 0, 0]..[1, 1, 1]` of air, floor `[0, 0, 0]..[64, 64, 8]`, pillar
`[32, 32, 8]..[40, 40, 40]` and slab `[24, 24, 40]..[48, 48, 48]`, all material 3 (bounds
half-open). **The sliver** is F after two carves: `[32, 32, 8]..[40, 40, 39]` (1 984 voxels),
then `[36, 32, 39]..[40, 40, 40]` (32 voxels). They leave cell `(4, 4, 4)` holding the 32 voxels
`[32, 32, 39]..[36, 40, 40]`, half the threshold.

### Phase 1: The repro, committed, pinning the bug

- **Owner skill:** dev
- **What:** Three tests that reproduce the disagreement and pin today's behaviour, green on
  `main`, each saying in its doc comment that Phase 2 or Phase 4 flips its expectation. The
  gate stays green, and the later diffs show the fix as a changed assertion.
- **Files touched:** `crates/vxe-world/tests/collapse.rs`, `apps/flythrough/tests/collapse_level.rs`
  (new), `apps/flythrough/src/main.rs` (its test module, next to `ledge_world`).
- **Done when:**
  - `a_slab_on_a_sliver_is_unsupported_and_wedged` (F, the sliver): the query over the second
    carve's changes returns one region, the nine slab cells `(3..=5, 3..=5, 5)`, counts
    `[(3, 4608)]`, aabb `[24, 24, 40]..=[47, 47, 47]`. `drop_region(&cells, 64)` returns
    `DropResult::default()`, and the world hash is unchanged.
  - `the_level_repro_finds_a_region_that_drops_nothing` (`apps/flythrough/tests/collapse_level.rs`)
    is the repro moved into the tree: the seed-1 level from `flythrough::cave::CaveSource` and
    `flythrough::cave_materials()`, the repro's SplitMix-style `mix` stream seeded 1, origins
    within ±200 voxels of `spawn()`, a direction in ±256 per axis, an air origin, a raycast of
    reach 100 voxels, then `EditOp::capsule(origin, hit point, 5 voxels, Carve)`, then
    `CoarseGrid::update` and `find_unsupported` with the parameters above. It runs until the
    first region and asserts that region is 4 cells and 501 voxels with aabb
    `[553, 794, 80]..=[566, 803, 87]`, found at loop iteration 3 243, and that
    `drop_region(&cells, 8)` returns `dropped == 0`. If the fly-through's table or an
    intervening change moves the iteration or the counts, the test pins what it finds, and the
    log notes the difference from the scratch repro's numbers.
  - The full sweep (40 000 iterations, every region dropped by 8 as the repro does) counts 6
    regions, 0 moved and 0 refused. It is a second test in the same file. If it runs longer
    than 10 s under `cargo nextest run` on the dev box, it is `#[ignore]`d with that reason,
    and the first-region test carries the gate. The log records the runtime either way.
  - Logic level (`apps/flythrough/src/main.rs` tests): a `sliver_ledge_world`. It is
    `ledge_world` with the pillar's top cell cut to the 32-voxel sliver
    `[32, 32, 39]..[36, 40, 40]` (the pillar stays full over `[32, 32, 8]..[40, 40, 32]`), plus
    an 8-voxel rock lump `[18, 36, 42]..[20, 38, 44]` in cell `(2, 4, 5)`, which is not touching
    the slab. A dig of radius 2 voxels from the lump's centre `(19, 37, 43)` carves `{rock: 8}`.
    The query over its cells reports `region 9 1:4608`, and the next tick prints
    `landed after 0 ticks, fell 0 voxels`.

### Phase 2: The yielding drop in `vxe-world`

- **Owner skill:** dev
- **What:** `World::drop_region_through(cells, max_drop, yield_below)` and its decoded
  reference `drop_region_through_decoded`; `DropResult::crushed`; `drop_region` and
  `drop_region_decoded` delegate with `yield_below = 0`. `gap_below` learns the yield rule: the
  solid count of a non-region cell is read once per cell from its brick's solid masks and cached
  for the op. The composer carves the crushed voxels of each swept span that the landed region
  does not overwrite. `DropResult::edit_result` merges `moved` and `crushed` into
  `changed_from`, and `touched` and `aabb` include every crushed voxel, so
  `CoarseGrid::update` sees the cells that lost voxels. The module and method docs state the
  rule. Phase 1's world and level tests flip, and the collapse course gains a crush.
- **Files touched:** `crates/vxe-world/src/collapse.rs`, `crates/vxe-world/src/lib.rs`
  (re-exports if any), `crates/vxe-world/tests/collapse.rs`,
  `apps/flythrough/tests/collapse_level.rs`, `crates/vxe-world/benches/collapse.rs`.
- **Done when:**
  - The sliver: `drop_region(&cells, 64)` still returns `DropResult::default()`. `drop_region_through(&cells,
    64, 64)` returns `dropped: 32`, `moved: [(3, 4608)]`, `crushed: [(3, 32)]`,
    `touched: [ChunkCoord([0, 0, 0])]` and `aabb: Some(([24, 24, 8], [47, 47, 47]))`. The
    world then equals the floor plus the box `[24, 24, 8]..[48, 48, 16]` of material 3 (by
    `voxel_hash` against a world built directly). The arithmetic: the slab's bottom is z 40, the
    sliver at z 39 yields, z 38 to 8 is air, and the floor at z 7 sits in a cell of 512.
  - The sliver made of material 4 (`INDESTRUCTIBLE` in `test_table`), filled after the
    whole-pillar carve: `drop_region_through(&cells, 64, 64)` returns `DropResult::default()`,
    and the world hash is unchanged.
  - `a_solid_voxel_between_two_region_runs_wedges_the_piece` keeps its `drop_region`
    assertions. With `drop_region_through(&cells, 64, 64)`, the material-5 voxel at
    `[32, 32, 39]` (its cell holds 1) yields: `dropped: 4`, `moved: [(3, 1024)]`,
    `crushed: [(5, 1)]`, and the world equals the test's own "without the voxel" world.
  - The native path equals the decoded reference on `dressed_cubes` drops with `yield_below`
    of 0, 64 and 300 (extend `the_native_collapse_matches_the_decoded_one`).
  - The collapse course gains a third part: the sliver, its query, then
    `drop_region_through(cells, 8, 64)` per tick with the cells one cell lower after each whole
    step. Falls `[8, 8, 8, 8, 0]`, `crushed` `[(3, 32)]` on the first drop only, every
    `crushed` fed to the hash. The course hash moves from `0x3178_0a15_61c1_3041`. The new
    value is blessed on Linux; the log records old and new.
  - The level tests flip. The first region at iteration 3 243 now drops `> 0` with
    `drop_region_through(&cells, 8, 64)` and crushes `> 0`, with the exact values pinned. The
    full sweep, every region falling tick by tick with `max_drop` 8 and `yield_below` 64 until
    a drop is under 8, reports **no** region whose first drop is 0 and none refused. The region
    count and the total crushed are pinned. If a region rests on a `shell` voxel, the test names
    it and asserts its first drop is 0 with an `INDESTRUCTIBLE` blocker, instead of failing.
  - Bench: a new arm `drop_region_through of the 9-cell slab through a 32-voxel sliver, 32
    voxels` and a yielding twin (`yield_below` 64) of the 4 096-cell cube drop by 64.

### Phase 3: Op kind 5 on the wire, through the server and in the log

- **Owner skill:** dev
- **What:** `Op::Drop` gains `yield_below: u16`. It encodes as kind 3 (unchanged bytes) when
  `yield_below == 0` and as the new kind 5 otherwise: `max_drop i32`, `yield_below u16`, cell
  count `u32`, the cells. Decoding kind 3 gives `yield_below: 0`. Decoding kind 5 with
  `yield_below == 0` is refused, so every op has one encoding; a new `WireError` variant is
  fine if `UnknownKind` would mislead. The op-result encoding of a drop gains the `crushed`
  counts after `moved`. Results cross only the channel and the component boundary, never a log
  file. The server applies `Op::Drop` through `drop_region_through` and updates its grid from
  `edit_result()`. `LOG_FORMAT` stays 2: op kinds are self-describing, and the header's layout
  does not change. The WIT does not change, since ops and results cross as wire bytes. Every
  `Op::Drop` construction site outside the logic passes `yield_below: 0`, which keeps its bytes
  and its pinned hash. The module tables in `wire.rs` and `log.rs` list kind 5.
- **Files touched:** `crates/vxe-world/src/log.rs`, `crates/vxe-world/src/wire.rs`,
  `crates/vxe-world/tests/log.rs`, `crates/vxe-net/src/server.rs`,
  `crates/vxe-net/src/test_logic.rs`, `crates/vxe-net/tests/*.rs` as needed,
  `crates/vxe-script-wasm/tests/determinism.rs`, `crates/vxe-script-wasm/src/lib.rs` if it
  matches on `Op`.
- **Done when:**
  - `every_tag_round_trips` covers kind 5, including a 4 096-cell drop with `yield_below` 64.
    The entry size is asserted at `23 + 12n`: 12 for tick and seq, plus kind 1, `max_drop` 4,
    `yield_below` 2, count 4. That is 131 bytes for 9 cells and 49 175 for 4 096. Kind 3 stays
    `21 + 12n`.
  - A kind-3 entry encoded by Plan 0018's layout (literal bytes in the test) decodes to
    `yield_below: 0` and re-encodes byte-identical. A kind-5 entry with `yield_below` 0 is
    refused.
  - The vxe-net slab session still hashes to `SLAB_SESSION_HASH` (`0x2077_5c41_d63d_17c5`),
    and its op-log bytes are unchanged. That proves a kind-3 save's replay is untouched.
    `OP_LOG_HASH` (`0x5ac8_737d_db0f_ccec`) and `ECHO_HASH` (`0x8f7d_4016_15d9_5c4d`) do not
    move.
  - A server-routed kind-5 drop of the sliver's slab (`max_drop` 32, `yield_below` 64)
    reaches a shared client as `Applied(OpResult::Drop { dropped: 32, crushed: [(3, 32)], .. })`.
    Replaying the server's log by ops gives the live world's hash, and the server's grid equals
    `CoarseGrid::build` of the world.

### Phase 4: `DigLogic` tells a refusal, a wedge and a landing apart, and forgets nothing it should not

- **Owner skill:** dev
- **What:** In `apps/flythrough/logic/src/lib.rs`:
  - The logic's drops carry `yield_below: YIELD_BELOW`, a constant equal to
    `vxe_net::DEFAULT_SUPPORT.solid_min`. The fly-through's own `SUPPORT` const in
    `apps/flythrough/src/main.rs` is replaced by `vxe_net::DEFAULT_SUPPORT`, so the two cannot
    drift.
  - `fall` splits its outcomes:
    - `Err(e)`: `drop refused: <e as Display>, after N ticks, fell M voxels`, and the region
      stops falling.
    - `Ok` with `dropped == 0` on the region's first tick: `wedged <cells> <material>:<voxels>
      ...` (the `region_line` grammar with the verb `wedged`), and the region is remembered.
    - `Ok` with `0 <= dropped < FALL_STEP` after at least one moving tick: the existing
      `landed after N ticks, fell M voxels` line. It gains `, crushed K voxels` when the fall
      crushed anything (`K` is the sum of `crushed` over its ticks).
  - `Falling` keeps the region's counts and the crushed total.
  - The memory of stuck regions is a FIFO of at most `STUCK_MAX = 64` keys, each `mix64` folded
    over the region's cells and counts. A wedged or refused region is pushed onto it.
    `collapse` skips a reported region whose key is in the memory: no `region` line and no op.
    A dig that changes the region changes its counts, so its key changes and it is tried again.
    The memory is logic state, a pure function of inputs and host answers, so input replay and
    the component reproduce it.
  - The fly-through translates `wedged` and `drop refused` lines for the terminal as it does
    `region` lines (`wedged: 9 cells, {rock: 4608}, cannot fall`).
  - The crate docs' broadcast description lists the new lines. Rebuild the guests
    (`scripts/build-guests.sh`) for the component tests.
- **Files touched:** `apps/flythrough/logic/src/lib.rs`, `apps/flythrough/src/main.rs`,
  `apps/flythrough/tests/replay.rs`, `apps/flythrough/tests/component.rs` if it pins lines.
- **Done when:**
  - The Phase 1 logic test flips, on `sliver_ledge_world` after the lump dig. The arithmetic:
    the slab's bottom is z 40, the sliver at z 39 yields, z 38 to 32 is air, and the pillar's
    full top cell holds z 31. Tick 1 drops 8 and crushes the 32 sliver voxels. Tick 2 drops 0
    onto the pillar and prints `op dropped`, then `landed after 1 ticks, fell 8 voxels, crushed
    32 voxels`. Then `count_in_box([24, 24, 32], [47, 47, 39]) == [(ROCK, 4608)]`, and the
    server's grid equals `CoarseGrid::build(&world)`.
  - The same world with the sliver made of `SHELL` (3): tick 1 prints `op dropped`, then
    `wedged 9 1:4608`. A dig of a second 8-voxel lump, `[50, 36, 42]..[52, 38, 44]` in cell
    `(6, 4, 5)`, reports nothing: its lines are
    `["op carved"]` and no op follows. A dig that takes voxels from the slab reports the region
    again with its new counts and prints `wedged` again.
  - A slab with no floor in the one-chunk world (`[24, 24, 40]..[48, 48, 48]` and the lump
    only) falls five ticks of 8, z 40 to 0. On tick 6 the scan reaches chunk `[0, 0, -1]`,
    which is outside the extent, and the logic prints `drop refused: chunk [0, 0, -1] is not
    resident, after 5 ticks, fell 40 voxels`. No later tick re-issues the drop.
  - `cutting_the_pillar_drops_the_ledge_a_cell_a_tick` and its `R` twin pass unchanged: no
    sub-threshold cell is under that ledge.
  - `a_stamp_and_a_collapse_replay_from_the_save` passes with the ops now of kind 5. If the
    scripted fall crosses a sliver, its `landed` line and `SESSION_HASH`
    (`0x422b_7654_317b_145d`) move. The new values are blessed on Linux, and the log records old
    and new with the reason. The `--logic wasm` session equals the native one byte for byte.
- **Not here:** a `Host` call that reports the server's `SupportParams` (a WIT change); see
  Risks.

### Phase 5: Readings and the budget lines

- **Owner skill:** dev
- **What:** The collapse bench readings, the log-size lines and the hash lines in
  `docs/nfr.md`, and the implementation log's close triggers.
- **Files touched:** `docs/nfr.md` (§4 collapse cost and logs, §6 the collapse course and
  session hashes), this plan's implementation log.
- **Done when:**
  - `cargo bench -p vxe-world --bench collapse`, criterion medians, with the exclusive perf lock
    and the 1-minute load recorded, on the dev reference (AMD Ryzen 9 5900HS). Every existing
    arm reads within noise of its §4 reading, and the yield-0 path is the same code as today's.
    The new slab-through-sliver arm reads under the slab proposal of 100 µs. The yielding cube
    drop reads under the 20 ms cube proposal.
  - §4's log line names kind 5 at `23 + 12n` (131 B a tick for a 9-cell region, 49 175 B for
    4 096 cells). §6's collapse course line carries the new hash, marked "blessed on Linux; the
    Windows arm confirms it on the first CI run after the owner pushes `main`", as does any
    session hash that moved.

### Phase 6: The owner's windowed check

- **Owner skill:** human
- **What:** In `cargo run --release -p flythrough -- --level`, dig under a ledge until the
  terminal reports `unsupported: ...`. The piece falls over the next ticks and the terminal
  prints `landed after N ticks, fell M voxels, crushed K voxels` with `M > 0`. `R` and a crumble
  still behave as before. Optionally run `--level --vault --save s.vxl` then `--replay s.vxl`
  with a fall in the session: the same hash. A save made before this plan (for example the
  owner's 2026-10-20 session, `0xada8a4f022567a9f`) replays by ops to its old hash.
- **Files touched:** none (the result is recorded in the implementation log).
- **Done when:** the owner has watched a region fall in the level and the line above is
  recorded with its numbers.

## Data shapes

```rust
// illustrative
pub struct DropResult {
    pub moved: Vec<(MaterialId, u32)>,
    pub dropped: i32,
    /// Voxels outside the region that the fall destroyed, by material, ascending: solid
    /// voxels in cells holding fewer than `yield_below` solid voxels, not INDESTRUCTIBLE,
    /// in the span each region run swept. Empty when `dropped == 0` or `yield_below == 0`.
    pub crushed: Vec<(MaterialId, u32)>,
    pub touched: Vec<ChunkCoord>,   // now includes crushed voxels' chunks
    pub aabb: Option<(VoxelCoord, VoxelCoord)>, // now includes crushed voxels
}

pub enum Op {
    // ...
    /// Kind 3 when `yield_below == 0`, kind 5 otherwise.
    Drop { cells: Vec<CellCoord>, max_drop: i32, yield_below: u16 },
}
```

| kind | op | payload | entry bytes |
|------|----|---------|-------------|
| 3 | drop (Plan 0018, unchanged) | `max_drop i32`, count `u32`, cells | `21 + 12n` |
| 5 | drop through sub-threshold cells | `max_drop i32`, `yield_below u16` (never 0), count `u32`, cells | `23 + 12n` |

## Budgets

- **§4 Edits, collapse cost:** the slab drop proposal (under 100 µs) and the cube drop proposal
  (under 20 ms) cover the yielding arms. The added work is one popcount per non-region cell the
  scan reaches, cached per op, plus at most `dropped` crushed voxels per footprint column, all
  inside bricks the op already composes. Expected effect: within noise for yield 0, a few
  percent for yield 64 (a guess). Measured in Phase 5 as above. `find_unsupported` is untouched.
  The edit line (2.2 ms) is not touched: shape edits do not change.
- **§4 Logs:** kind 5 `23 + 12n`, two bytes more than kind 3. A falling 9-cell region re-sends
  131 bytes a tick instead of 129. Asserted in `crates/vxe-world/tests/log.rs`.
- **§6 Determinism:** the yield rule is integer: cell counts are popcounts, and spans and gaps
  are `i32`. Columns are visited in the fixed `(x, y)` order the op already uses. The collapse
  course hash moves (Phase 2), and `SESSION_HASH` moves only if the scripted fall crosses a
  sliver (Phase 4). `SLAB_SESSION_HASH`, `OP_LOG_HASH`, `ECHO_HASH`, the stamp hash
  `0x7ee7_eeb7_0956_d069` and the mips hash must not move: a change to any of them is a
  regression in the kind-3 path. Every reblessed value is blessed on Linux. The Windows arm
  confirms it on the first CI run after the owner pushes `main`. Plan 0018's Windows
  confirmations of `SLAB_SESSION_HASH` and `SESSION_HASH` are still outstanding, so that run
  confirms both plans.

## Risks & open questions

- **The swept-span rule leaves overhangs.** A sliver that reaches past the footprint keeps its
  outside part, at its old height, in a sub-threshold cell the query never reports. This is
  ADR-0016's stated negative. The level sweep's pinned crushed totals make it visible. A
  followup could sweep such leftovers with `remove_region`; this plan does not.
- **The logic's threshold is a constant, not the server's.** `YIELD_BELOW` equals
  `DEFAULT_SUPPORT.solid_min`, and the fly-through now sets exactly that. A consumer that calls
  `set_support` with another `solid_min` must pass the same value in its drops. A `Host` call
  that returns the server's `SupportParams` would close this, but it is a WIT change, and it is
  deferred until a second consumer needs it.
- **Input replay of an old save.** Ops replay of a Plan 0018 save is bit-identical (kind 3
  means `yield_below` 0). Input replay re-runs today's logic, which issues kind-5 drops, so a
  saved session whose fall crossed a sliver replays by inputs to a different world. A log names
  no logic version; that gap predates this plan (see "does NOT do").
- **Forward compatibility.** A pre-0022 build reading a save with a kind-5 entry fails with
  `UnknownKind(5)` at that entry, not at the header. We keep `LOG_FORMAT` at 2 because the header
  does not change. A format bump is the owner's call (see the closing questions).
- **Stuck memory by hash.** A 64-bit key collision would skip a genuinely new region. The
  probability is negligible at 64 entries, and the outcome is benign: the region stays where it
  is until a dig changes it.
- **Threading and determinism:** unchanged. The op holds the store map's read lock and writes
  chunk by chunk in ascending order, as `drop_region` does today.

## What this plan does NOT do

- Carry (push the yielding voxels down ahead of the region). ADR-0016 rejects it for now. It
  could return as a mode of the same yield rule.
- Turn crushed volume into rubble. The result reports it, and a game may pile it. That is a
  `DigLogic` followup if the owner wants debris.
- Change the support query, `solid_min`, or `remove_region`.
- Record a logic version in the log header, or bump `LOG_FORMAT`.
- Tell the server's grid about streamed chunks, so collapse works on the streamed cave (Plan
  0018's followup). Collapse stays on `--level`.
- The cosmetic rigid-body layer for falling pieces (M8, after M6).

## Implementation log

> Written by `dev`: one row per phase as its commit lands, then the close block. **The phases
> above are the contract. This section records what happened.** Observations, never conclusions:
> no pass list, no self-assessment. Deviations and unmet done-whens are **always** disclosed.
> Keep it shorter than `## Implementation phases`.

| phase | owner | state | commit |
|---|---|---|---|
| 1: The repro, committed, pinning the bug | dev | not started | |
| 2: The yielding drop in `vxe-world` | dev | not started | |
| 3: Op kind 5 on the wire, through the server and in the log | dev | not started | |
| 4: `DigLogic` tells a refusal, a wedge and a landing apart | dev | not started | |
| 5: Readings and the budget lines | dev | not started | |
| 6: The owner's windowed check | human | not started | |

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

- At close: accept ADR-0016, and mark ADR-0008's index row "drop clause amended by ADR-0016".
  ADR-0008's body stays as accepted.
