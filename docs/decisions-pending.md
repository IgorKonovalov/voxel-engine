# Decisions pending: the owner's review list

Written 2026-10-17 after the overnight run that closed plans 0003, 0004, 0007 to 0012 and 0014
(main at v0.11.0, not pushed). Each section gives the context, the options with their costs, a
recommendation and what an answer sets in motion. Delete this file once every item is answered;
the answers themselves land in the plans, ADRs and `nfr.md` as usual.

Answer in any form ("D1: B, D2: accept, ..."); anything not answered keeps the recommendation
that is already in effect, where one is.

Contents:

- [D1. Plan 0013: the contour attribute footprint](#d1-plan-0013-the-contour-attribute-footprint)
- [D2. Plan 0006: the listening checks and two audio budgets](#d2-plan-0006-the-listening-checks-and-two-audio-budgets)
- [D3. Edit cost: a new brick representation, or a measured target](#d3-edit-cost-a-new-brick-representation-or-a-measured-target)
- [D4. The cylinder sweep at 26.3 us](#d4-the-cylinder-sweep-at-263-us)
- [D5. Push main](#d5-push-main)
- [D6. Calls taken overnight that you can overturn](#d6-calls-taken-overnight-that-you-can-overturn)
- [D7. Follow-up plans to draft](#d7-follow-up-plans-to-draft)
- [Windowed checks](#windowed-checks)

---

## D1. Plan 0013: the contour attribute footprint

**Context.** You picked contour as the best-looking surface mode. Contour stores a plane per
surface voxel in "attribute pages" on the GPU. At the current full-detail radius (16 chunks,
about 102 m) those pages add 1.71 GB on top of the world mirror, so the streamed world needs
about 3.31 GB live, against `nfr.md` §3's 3 GB line. Plan 0013 measured four ways to shrink it
(Phases 1 to 3 are done on branch `plan-0013`; Phase 4 is your pick).

The candidates, as the plan defines them:

- **A, surface voxels only:** a 64 B mask per brick plus 2 B per voxel that actually has a
  plane. Lossless. Needs a variable-size allocator inside the GPU pool, because the plane count
  per brick changes with every edit; that allocator, its fragmentation and compaction are new
  work in Phase 5.
- **B, an 8-bit code per voxel:** the plane quantised to 3+3 bits of direction and a 2-bit
  offset. Lossy (normal error doubles). Fixed size, simple.
- **C, one plane per 2x2x2 block:** 128 B per brick. Smallest. Lossy at 2-voxel features
  (silhouettes widen).
- **D, a smaller full-detail radius** with today's layout: 12 chunks (77 m) or 14 (90 m).
  Lossless, no new code, gives up view distance against §3's "about 100 m".

Measured (Phase 3; allocation multipliers 1.44x and 1.93x are the grow-never-shrink buffer
overhead: 1.44x is the plan's guess, 1.93x is what a streamed flight actually reached):

| Row | Mirror live | Alloc x1.44 | Alloc x1.93 | Normal error / silhouette |
|---|---|---|---|---|
| current, 16 chunks | 3.31 GB | 4.77 GB | 6.38 GB | 7.41 deg / 32 px |
| A, 16 | 1.82 GB | 2.63 GB | 3.52 GB | 7.41 deg / 32 px |
| B, 16 | 2.46 GB | 3.54 GB | 4.74 GB | 14.71 deg / 40 px |
| C, 16 | 1.81 GB | 2.61 GB | 3.50 GB | 5.50 deg / 44 px |
| D, 12 | 1.96 GB | 2.83 GB | 3.79 GB | 7.41 deg / 32 px |
| D, 14 | 2.56 GB | 3.69 GB | 4.94 GB | 7.41 deg / 32 px |
| A or C at 14 | 1.56 GB | 2.24 GB | about 3.0 GB | as A or C |
| A or C at 12 | 1.35 GB | 1.94 GB | 2.59 GB | as A or C |

**The finding that matters most.** D at 12 chunks was flown for real: live memory peaked at
2.13 GB but *allocated* memory reached 4.10 GB, because GPU buffers grow and never shrink
(ADR-0004). So under motion no single 16-chunk row fits 3 GB once allocation is counted. There
are two levers, and they compose:

1. **Bytes per brick** (A, B, C).
2. **Radius** (D), or **the allocation growth rule** (let buffers shrink or reuse slots
   better), which is the ADR-0004 question the plan's Risks named.

**Options.**

- **A at 16, plus the growth-rule question.** Pros: lossless, full radius, same look you
  approved. Cons: the biggest Phase 5 (a sub-slot allocator); still 3.52 GB at the measured
  1.93x, so it only fits if allocation overhead is also tamed.
- **C at 16.** Pros: smallest and simplest (fixed size), lowest normal error on the metric.
  Cons: lossy at 2-voxel features (silhouettes 44 px vs 32); same 3.50 GB at 1.93x.
- **A or C at 12 to 14 chunks.** Pros: the only rows under 3 GB at 1.93x (2.59 to about 3.0
  GB). Cons: view distance 77 to 90 m, plus whichever layout cost you pick.
- **D alone (12 chunks).** Pros: no new code at all. Cons: measured 4.10 GB allocated, so it
  does not fit unless the growth rule changes too.
- **Change the growth rule first, then pick.** Pros: attacks the 1.93x that sinks every row;
  could make A or C at 16 fit. Cons: an ADR-0004 revision and its own plan before 0013 can
  finish.

**How to judge the lossy ones by eye** (in the plan's worktree, one at a time):
`cd .claude/worktrees/plan-0013 && cargo run --release -p flythrough -- --level --seed 1 --look contour --attrib-quant code8`
(B), then `block2` (C), then `none` (reference). `N` cycles normal modes, `V` detail, `L`
shadows. Look at silhouettes and walls close up and at about 5 m.

**Recommendation.** Look at `block2` (C) first. If it holds up by eye, take **C at 14 chunks**
(about 3.0 GB at the measured overhead, 90 m radius, smallest code) and open a separate plan
for the growth rule; if that later brings overhead down, the radius can go back to 16. If C
does not hold up, take **A at 14**.

**What an answer starts.** The architect writes ADR-0012 from your pick, the plan-0013 branch
merges main (it is 10 plans behind, with known conflicts in `march.wgsl`, `gpu_world.rs` and
`world_budget.rs`), then dev builds Phase 5. Phase 6 (you set rock detail strength by eye) and
Phase 7 (contour becomes the default) follow.

---

## D2. Plan 0006: the listening checks and two audio budgets

**Context.** Spatial audio is implemented and reviewed on branch `plan-0006` and fixed after
review (the mixer was 4.7x too slow because its FIR loop could not vectorise; now 0.36 ms).
The HRIR pack is byte-identical to the one you approved without clicks. It cannot close
without your ears, and two readings miss their proposals.

**D2a. The listening checks** (headset worn swapped, as before;
`cd .claude/worktrees/plan-0006 && cargo run --release -p flythrough -- --seed 1 --audio`,
or `--audio-debug` to also record a wav):

1. Dig a hole, press `H`, turn until the hole is at the left screen edge, press `H` again: the
   knock should come from the left.
2. Dig, press `J`, walk behind rock: the hum should turn duller and quieter, clear again in
   line of sight, and no echo should be louder than the hum behind rock.
3. Press `J`, turn slowly and fast: no clicks.
4. With about 8 sounds playing, press `F1` and note the callback max, active taps and the last
   acoustics update time.

**D2b. `Acoustics::update` at 1.03 ms against a 0.5 ms proposal.** This is the per-frame
occlusion and reflection probe for 32 sources. The 0.5 ms figure had no arithmetic behind it.
The cost is about 240 rays x about 100 voxel steps x about 40 ns, and each step is the exact
integer DDA shared with `World::raycast`, with a chunk-map lookup and lock per step.

- **Option (a), record the reading as the budget (about 1.0 ms) with its arithmetic, and make
  speed-ups follow-ups** (cache the chunk handle across steps; run probes on a worker thread).
  Pros: honest, closes now; probes are capped by count and cadence so 1 ms is a ceiling, not
  every frame. Cons: about 6% of a 16.7 ms frame while all sources are due at once (walking can
  make that every few frames).
- **Option (b), optimise `trace_materials` in this plan.** Pros: the budget stays tight. Cons:
  the DDA is deliberately shared with the engine's raycast; changing it here touches a pinned
  hash's code path and delays the close.
- **Recommendation: (a).**

**D2c. HRIR pack load at 13.6 ms against 10 ms.** Measured in the release fly-through on a
quiet box; a release test build read 7 to 9 ms. It runs once at startup.

- **Accept 13.6 ms as the reading** (startup only), or **ask for a faster loader** (for
  example a memory-mapped pack). Recommendation: accept; a startup cost this small does not
  earn new code.

**Merge note.** `default-members` in `Cargo.toml` is now an explicit list (plan 0011 needed
that so the default build skips `wasmtime`). When 0006 merges, `crates/vxe-audio` must be
added or it silently drops out of `cargo build`. The branch is also behind main and will need a
merge.

---

## D3. Edit cost: a new brick representation, or a measured target

**Context.** `nfr.md` §4 says 100 radius-1 m carves cost under 2 ms. Plans 0003 and 0014 took
the worst case from 4.92 ms to 3.19-3.26 ms (3.03 ms with a warm array pool) on the dev box,
and stopped there. The profile of the warm worst case (3.04 ms):

| Part | Share | Time |
|---|---|---|
| Writing the voxels row by row (`write_rows`) | 54.6% | about 1.66 ms |
| Solving each row's extent inside the sphere | 12.2% | 0.37 ms |
| Row-span cache lookup | 7.1% | 0.22 ms |
| Expanding and collapsing bricks | 5.2% | 0.16 ms |
| Everything else (per-brick work, tallies) | about 21% | about 0.64 ms |

100 carves write about 417 000 voxels, about 4 ns (16 cycles) per voxel to read the old
value, decide the new one, write it and count it. Even if the solve, the lookup and the
expansion all went to zero, the edit would still take about 2.4 ms. **Nothing inside today's
representation (ADR-0002: a brick is either uniform or a full `[u16; 512]`) reaches 2 ms.**

- **Option A: a brick-representation ADR** (for example a bitmask or palette brick, or arena
  handles), superseding part of ADR-0002.
  - Pros: a carve becomes per-row word operations and the tallies come from popcounts; a guess
    of 0.2 to 0.4 ms for the write puts the warm worst case near 1.7 ms on the dev box; it
    probably lowers RAM too.
  - Cons: every reader of bricks changes (raycast, mips, coarse grid, walkable map, stamp,
    collapse, persistence format, the GPU mirror's narrowing); a multi-phase plan with a
    byte-identical determinism gate; and on the baseline machine (guessed 1.5 to 2x slower)
    it may still miss 2 ms.
- **Option B: revise §4 to a measured cap** (for example "100 radius-1 m carves in at most 3.5
  ms on the dev reference"), or restate it around the consumer's real per-frame edit volume.
  - Pros: free now; the consumer asks for "many edits per frame", and the fly-through's real
    dig is one 0.5 m op, so 100 x 1 m in one frame is a stress figure rather than a stated
    need. The representation can be revisited when a profile of real play shows edit cost.
  - Cons: the target moves to meet the code; if the consumer later does need 100 big edits a
    frame, the work returns with more of the engine built on today's representation.
- **Recommendation: B now, with A recorded as a deferred follow-up**, to be reopened when the
  baseline CPU is measured (M2's close) or a real frame shows edit cost.

---

## D4. The cylinder sweep at 26.3 us

**Context.** The collision sweep (a vertical cylinder moving through voxels) had a 5 us
proposal; you accepted the measured 17.5 us earlier. The walk-check fixes (climbing that
works, no freezing in corners, an overlap guard) cost about 50% on a quiet box: the F2 wall
case now reads 26.3 us per sweep. `nfr.md` §8 says "awaits the owner's call". A full
controller tick on terrain is 2.6 us against a 20 us budget, so the sweep is not hurting a
frame today.

- **Accept 26.3 us as the budget.** Pros: the walk felt natural in your check; the tick
  budget has 7x headroom. Cons: a body-heavy scene (many NPCs sweeping) costs more than
  planned.
- **Profile against 20 us** as a small follow-up plan. Pros: holds the line before more
  bodies arrive. Cons: work now for a cost no consumer feels yet.
- **Recommendation: accept, and record "profile if a scene sweeps more than about 100 bodies
  a tick"** as the trigger to revisit.

---

## D5. Push main

**Context.** Main is many commits ahead of `origin/main` (v0.4.0 to v0.11.0). Nothing was
pushed overnight because you had not asked. Pushing also triggers CI's Windows arm, which has
never run the new cross-platform determinism checks: the collapse course hash, the op-log
hash, the stamped-world hash, and native-versus-component equality. Until it runs, `nfr.md`
§6's "both arms" claims are pending.

- **Push now.** Pros: Windows proves (or disproves) determinism while the context is fresh.
  Cons: if Windows fails, main's CI goes red until a fix lands.
- **Hold.** Pros: none beyond reviewing first. Cons: the Windows unknowns grow with every
  plan built on top.
- **Recommendation: push after reading this file.** Say "push" and it is one command.

---

## D6. Calls taken overnight that you can overturn

These were applied under your standing "go with your recommendations". Each closed plan's
`## Close` section lists them as overturnable. If you overturn one, the plan or ADR gets an
amendment and dev a small pass.

| Plan | Call | Why | Main alternative and its cost |
|---|---|---|---|
| 0004 | A body an edit fills into is not pushed out (no depenetration); recorded as a known gap | Only an edit onto the body causes it; `G` refuses an overlapping placement | Lift the body out by up to a step height: a small fix pass |
| 0007 | A 13-byte boundary entry in the op log marks where directly submitted ops end and logic ops begin | Only option that replays exactly, leaves ADR-0010 alone and moves no budget or hash | Log submitted ops in the input log (breaks "engine never decodes inputs"), or a third log (bigger header) |
| 0007 | `--vault`, `P` and collapse are refused under `--save`/`--replay` instead of being logged | Logging them needs op kinds 2 to 4 and a way for the log to name a prefab, likely an ADR | Wire them now: a plan of its own (D7) |
| 0008 | Hostile `.vox` caps: model edge 256, 65 536 placements, translation 2^20 | A 70-byte file could otherwise allocate 3 GB; 1 KB could ask for 32 GB of placements | Larger caps: only if a real asset needs them |
| 0009 | Chunks outside a bounded world count as air, so `boundary_anchors` means something | Before, the flag did nothing | Treat the edge as rock: the flag stays inert |
| 0009 | Rubble spares indestructible voxels; a rigid drop moves them | Matches how a carve treats them | Destroy them: breaks the material table's rule |
| 0010 | The overlay's 5% cost check gates on the bounded level only; the streamed figure is logged | The streamed runs vary 21% on their own while chunks load | Gate after full residency: a new run mode |
| 0011 | A script that traps (fuel, memory, panic) stays stopped; later ticks report `Stopped` | A restart would think it is tick 0 and re-apply its setup edits | Restart from seed (silent state reset), or snapshots (needs an ADR) |
| 0011 | Script timers are always ready; host-side panics become a script trap | Real-time timers broke determinism; a panic killed the server | None sensible |
| 0011 | Per host call budget 2 us (the 1 us proposal missed at 1.4 us); host-call work is not metered by fuel | A whole tick through the component is only 1.03x native | Batch the imports now: work before any consumer needs it |
| 0012 | No version bump for the repository gates | Nothing a consumer links changed | A patch bump |
| 0014 | Phase 3's target restated to what was measured (span share 19% to 13%, not under 5%) | Under 5% needs about 11 cycles a row | Keep the plan blocked: see D3 instead |

---

## D7. Follow-up plans to draft

Say which to draft; the architect writes each as a normal plan for your approval.

1. **Route stamp, drop and remove through the server** (op kinds 2 to 4 in the op log, the
   wire and the logic host), so `P` and collapse work under `--save`. Needs a decision on how
   the log names a prefab (an id plus content hash in the header, or the prefab bytes), likely
   an ADR.
2. **Meter host-call work and range-check shapes in `vxe-world`**: today a script can ask for
   a raycast or a box count big enough to take seconds, and extreme shapes panic in debug
   builds (now caught and turned into a trap, but the engine should refuse them).
3. **A small dev pass of leftover nits**: the overlay allocates a little every frame while
   shown, its memory estimate leaves out the brick-array pool, a stale "4 Mi" comment, an
   unused parameter, two missing doc comments.
4. **The GPU buffer growth rule** (ADR-0004), if D1 goes the way recommended.
5. **The sweep profile**, only if D4 is "profile".

---

## Windowed checks

Each was proven headless; these are the looks only you can judge. Run from the repo root on
main unless noted.

- **Prefab vault** (plan 0008): `cargo run --release -p flythrough -- --level --vault`. A dark
  1.6 m box sits about 10 m ahead along +x; digging around it leaves its shell.
- **Stamp key** (plan 0008): `cargo run --release -p flythrough -- --level`, aim at rock within
  10 m and press `P`: a 1.6 m box appears with a corner at the aimed voxel, and a `stamped:`
  line prints. Aim at air: `stamped: no surface within 10 m`.
- **Collapse** (plan 0009): on `--level`, dig out the rock under a ledge. The terminal prints
  `unsupported: N cells`, the piece falls 8 voxels a frame and prints `landed after ...`. Press
  `R` and repeat under another ledge: it turns into a rubble pile. `F1` shows the last query's
  time.
- **Stats overlay** (plan 0010): `cargo run --release -p flythrough -- --level --hud`. Rows at
  top left (over-budget rows yellow), a frame-time strip with a red 16.67 ms line at bottom
  left, `F2` hides and shows it.
- **Audio** (plan 0006): see D2a.
- **Footprint** (plan 0013): see D1.
