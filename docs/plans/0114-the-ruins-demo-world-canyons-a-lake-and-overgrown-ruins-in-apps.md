# 0114: The ruins demo world: canyons, a lake and overgrown ruins, a generic scene in `apps/` where the target look is judged

> **Status:** draft (2026-11-28), awaits the owner's approval (and his pick of where it lives,
> owner question 5 of the 2026-11-28 batch)
> **Drafted on Opus 2026-11-28 (Fable exhausted for the week); a Fable second pass reviews and
> fixes this draft after the weekly reset (owner).**
> **Created:** 2026-11-28
> **Milestone:** M10: Outdoors in [`docs/roadmap.md`](../roadmap.md), the target look
> **Related ADRs:** [ADR-0090](../adrs/0090-every-look-feature-has-a-floor-form-and-a-rich-form-and-a-rich-profile-holds-its-own-budget-on-a-named-card.md),
> [ADR-0066](../adrs/0066-a-large-prefab-is-stored-as-bricks-and-its-stamp-is-a-placement-the-world-finishes-as-chunks-arrive.md)
> (the ruins as placements), [ADR-0028](../adrs/0028-far-field-as-a-clipmap-of-source-generated-coarse-chunks.md)
> (the source's coarse path for the far field)
> **Source:** the owner, 2026-11-28: "for what game provides, I think we can include small demo
> here in this repo, so it would be able to showcase engine capabilities, and we keep all in
> one place. just something generic but beautiful"; the target look
> (`docs/references/local/wbk-voxel-2026/`).
> **Starts after:** Phases 1 and 2 start on the world lane after Plan 0033's world briefs; each
> later phase names the plan it waits for. It is the scene every target-look window flies.

## TL;DR

A new library crate under `apps/` (recommended `apps/ruins`, crate `ruins`; question 5) holds
the "game's side" of the target look with no gameplay: a seeded canyon terrain with terraced
cliffs and a lake, procedural ruins (blocks, towers and walls with window grids, recesses and
arches, partly collapsed) placed on the canyon floors and ledges, the overgrowth rules for Plan
0113's pass (moss on tops, spills over lips, stains under ledges), the vegetation placements of
Plan 0115, named camera shots and a tester path. The fly-through opens it as `--scene ruins`
and the tester as its fifth scene. Nothing under `crates/` names it. The first thing seen is
the canyon and its lake in the fly-through, from a named shot.

## Context & problem

- The target look needs a scene that has its shapes: cliffs, ledges, water, built forms. The
  testbed's scenes (vein, hall, castle, showcase cavern, outdoor hills) have none of canyons,
  lake and ruins together, and every look plan from 0112 on needs one place to be judged.
- The rules of overgrowth and the content are the consumer's (CLAUDE.md: the engine knows no
  gameplay). The owner wants them in this repo, generic and beautiful.
- Touches `docs/nfr.md` sections 3 and 5 (a denser world: mixed bricks, generation time) and
  section 2 through every look plan read in it.

## Decision

The demo is a library crate under `apps/`, depended on by the fly-through and the tester,
every value a pure integer function of the seed and the voxel coordinate (as the showcase
is), with direct `halo` support for Plan 0113 and a coarse path for Plan 0033. The ruins are
generated as prefabs from integer rules and placed through `StampedSource` (ADR-0066). The
world is unbounded: content repeats per 256 m region cell by seed. We rejected (question 5)
**a module inside the fly-through** beside `showcase` (fewest moving parts, but the testbed
grows again and a later standalone demo or the web demo could not reuse the world without the
testbed), and **a standalone demo binary now** (its own loop and UI before the look settles;
it stays a followup on top of the crate).

## Implementation phases

### Phase 1: The canyon and the lake (walking skeleton)
- **Owner skill:** dev
- **What:** the crate; a canyon source (a heightfield of integer noise, terraced into cliffs by
  a quantised height, carved by a winding channel), a lake at a fixed level filled with the
  water row (a translucent row until Plan 0034's liquid row lands), rows for stone, sand,
  moss, stain, foliage and water in a material table file; `--scene ruins` with three named
  shots; the direct `halo`.
- **Files touched:** new `apps/ruins/` (`Cargo.toml`, `src/lib.rs`, `src/canyon.rs`,
  `assets/materials.txt` (the table file in the `.rig` grammar, as the fly-through's), `src/tests.rs`), the workspace
  `Cargo.toml`, `apps/flythrough/src/scene.rs`, `apps/flythrough/src/cli/parse.rs`,
  `docs/flythrough.md`, `docs/development.md`.
- **Done when:**
  1. `the_canyon_is_the_same_in_any_order` (CPU, about 2 s): 4 x 4 x 2 chunks generated in two
     orders and on 4 threads give equal per-chunk hashes, and a fixed vector of 8 chunk
     hashes at seed 1 matches on both CI arms.
  2. `the_lake_fills_to_its_level` (CPU, under 1 s): in a named column at the lake's centre,
     every voxel from the bed up to the lake level is water and the one above is air.
  3. The fly-through opens `--scene ruins --shot lake` and shows the scene name in the banner.

### Phase 2: The ruins
- **Owner skill:** dev
- **What:** a seeded ruin generator (blocks of 4 to 30 m, towers, wall runs; facades with
  window grids, recesses, ledges and arches; a collapse mask that removes a seeded share from
  the top) placed on canyon floors and ledges, never over water deeper than a set limit.
- **Files touched:** `apps/ruins/src/ruins.rs`, `apps/ruins/src/lib.rs`, `apps/ruins/src/tests.rs`.
- **Done when:**
  1. `no_ruin_voxel_floats` (CPU, about 3 s): over a 4 x 4 x 4 chunk extent at seed 1,
     `svet-world`'s support query from the ground finds every ruin voxel supported.
  2. The order test of Phase 1 holds with the ruins.
  3. A reading: generation time per chunk and mixed-brick share against Phase 1's.

### Phase 3: The far path (after Plan 0033's coarse source path lands)
- **Owner skill:** dev
- **What:** the source's coarse generation for Plan 0033's clip levels.
- **Files touched:** `apps/ruins/src/canyon.rs`, `apps/ruins/src/ruins.rs`, `apps/ruins/src/tests.rs`.
- **Done when:** Plan 0033's census test, pointed at this source, holds: each coarse voxel
  equals the census of the full-detail voxels it covers, at three named regions.

### Phase 4: The overgrowth rules (after Plan 0113 merges)
- **Owner skill:** dev
- **What:** pass 1: stone and sand with sky above become moss where a noise field over 8 m
  cells is above a threshold; air over a mossed lip becomes a 1 to 3 voxel spill. Pass 2:
  stone faces under moss within a seeded 3 to 10 voxels become stain. Anchors for small
  foliage tufts on moss.
- **Files touched:** new `apps/ruins/src/overgrowth.rs`, `apps/ruins/src/tests.rs`.
- **Done when:** the order test holds with the pass on; a test states the moss share of the
  open stone tops at seed 1 within 2 % of the rule's threshold share; readings of generation
  time and mirror bytes with the pass on and off (Plan 0113 Phase 4's).

### Phase 5: The vegetation (after Plan 0115 Phase 2)
- **Owner skill:** dev
- **What:** clumps on mossed tops and ledges, vines down cliffs and facades, flowers in grass,
  placed with Plan 0115's shape builders through anchors and placements.
- **Files touched:** `apps/ruins/src/overgrowth.rs`, `apps/ruins/src/vegetation.rs`, tests.
- **Done when:** the order test holds; the share of ledges with a clump is the rule's within 2 %.

### Phase 6: The tester scene and the shots
- **Owner skill:** dev
- **What:** the tester's fifth scene with a computed camera path (canyon flight, lake, a
  facade close-up); five named shots; a README image row in `docs/images/shots.json` if the
  owner picks it in Phase 7.
- **Files touched:** `apps/tester/src/menu.rs`, `apps/tester/tests/run.rs`, the fly-through's
  path module, `apps/ruins/src/lib.rs` (shots).
- **Done when:** `each_choice_gives_its_exact_flag_list` and `the_flythrough_accepts_every_choice` (`apps/tester/tests/run.rs`, existing) cover the `ruins` scene, and the scene's path runs headless in the tester's benchmark mode.

### Phase 7: The owner's window
- **Owner skill:** human
- **What:** the owner flies the ruins at Floor and at Rich and judges the content (scale,
  density of ruins and overgrowth, colours); the look features are each plan's own windows.
- **Done when:** the picks (seeds, densities, the material colours) are in the readings log.

### Phase 8: The picks as defaults and vectors
- **Owner skill:** dev
- **What:** the picks as the crate's defaults; the fixed vectors re-blessed; a golden of the
  `lake` shot at Floor on lavapipe (SSIM floor 0.98).
- **Files touched:** `apps/ruins/src/*`, `apps/flythrough/tests/gpu_ruins.rs` (new).
- **Done when:** the vectors pass on both CI arms; the golden passes (`gpu_ruins`, about 6 s).

## Budgets

| `nfr.md` figure | projection (a guess) | how the close measures it |
|---|---|---|
| Section 5, generation per chunk | canyon about the hills source's; ruins and overgrowth up to 3x it | Phases 2 and 4 readings, RTX host |
| Section 3, mirror and RAM | the densest scene we have; read, not projected | Phase 4 reading |
| Section 2, frame, per profile | the scene each look plan reads; its own frame read at Floor (RTX, RENOIR at Portable) and Rich (RTX) | Phase 8 |

## Risks & open questions

- **Generation time** on the stream workers may be the scene's limit before the frame is; the
  readings in Phases 2 and 4 come first, and region-level caching of ruin prefabs is the lever.
- **Beauty is a judgement:** Phase 7 can send content back; the phases keep seeds and
  densities as constants for that.

## What this plan does NOT do

- No gameplay, no entities with behaviour, no audio design.
- No standalone demo binary (a followup on the crate) and no web demo (a separate app, owner
  2026-10-21).
- No engine mechanism: every mechanism it uses is another plan's.

## Implementation log

> Written by `dev`: one row per phase as its commit lands, then the close block. Readings,
> attempts and resume notes go to `docs/plans/logs/0114.md`.

| phase | owner | state | commit |
|---|---|---|---|
| 1: the canyon and the lake | dev | not started | |
| 2: the ruins | dev | not started | |
| 3: the far path | dev | not started | |
| 4: the overgrowth rules | dev | not started | |
| 5: the vegetation | dev | not started | |
| 6: the tester scene and the shots | dev | not started | |
| 7: the owner's window | human | not started | |
| 8: the picks as defaults and vectors | dev | not started | |

### Notes

### Close triggers

## Followups

- A standalone demo binary on top of the crate, once the look settles.
