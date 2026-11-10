# 0063: Rigs and clips: keyframe animation sampled by tick, a rig text file beside the `.vox`, a model viewer in the fly-through, and voxels from text

> **Status:** in-progress (Phases 1 and 2, stacked on `plan-0062` at bc80246; Phases 3 and 4
> after Plan 0062 closes)
> **Created:** 2026-11-10
> **Milestone:** M6 in [`docs/roadmap.md`](../roadmap.md)
> **Related ADRs:** [ADR-0051](../adrs/0051-rigs-and-clips-are-text-files-sampled-in-fixed-point-by-tick.md)
> (proposed; the file and the sampling), accepted at this plan's close. Rests on
> [ADR-0050](../adrs/0050-entities-are-posed-voxel-volumes-in-their-own-pool-traced-after-the-world-march.md)
> (models and poses).
> **Source:** the owner, 2026-11-10: "flow is I create a model, or you help me create it, with
> possibility of emissive light (gloving eyes for example) - we place model on a level", and
> the answer "Engine plays keyframe clips".
> **Starts after:** [Plan 0062](0062-entities-voxel-models-on-rigid-parts-traced-lit-shadowed-and-queried.md).
> Phases 1 and 2 are headless and need only its Phase 1 (`vxe-entity`); they may run as a
> side lane once that has merged. Phases 3 and 4 need Plan 0062 closed.

## TL;DR

Plan 0062 draws a model in whatever pose the consumer hands it. This plan makes the poses:
**clips** of keys per part, sampled by integer tick in fixed point, so the server and every
client get the same pose. A model's rig and clips live in one **`.rig` text file** beside its
`.vox`, written by hand or by an agent, parsed in `vxe-assets` with no new dependency. The
fly-through becomes the **viewer**: `--model walker.rig` shows the model, plays its clips, and
reloads the file when it changes, so the authoring loop is edit, save, look. A small tool turns
a **text drawing of voxels into a `.vox`** and back, so a model can be made without opening
MagicaVoxel and still be opened in it. The first thing anyone sees: `flythrough --model
apps/flythrough/assets/walker.rig`, the walker strides in place, you change a number in the
file, and its legs swing wider a second later.

## Context & problem

The owner's flow has three steps: make a model, give it glowing parts, put it on a level.
Plan 0062 covers drawing and lighting it. What is missing is the middle: a way to say which
`.vox` node is which part, where each part pivots, which palette entries are emissive, and how
the parts move over time; and a way to see the result without a rebuild.

**What the tree gives this plan** (at 6c76234): `.vox` parsing with the scene graph and
palette (`crates/vxe-assets/src/vox.rs`) and a writer (`write.rs`); a palette-to-material map
that produces prefabs (Plan 0008); after Plan 0062, `vxe_entity::Model`, `Pose` and
`EntitySet`, and the fly-through's `7` and `8`.

**What `.vox` cannot hold:** pivots, a rotation that is not one of the 24 axis rotations, and
motion over time. That is the `.rig` file's job (ADR-0051).

**The `nfr.md` figures touched:** section 6 (clip sampling joins the deterministic set) and
section 7 (the gate's wall time). Sampling on the CPU is read against section 2's streaming
and CPU line.

## Decision

Build ADR-0051 in four phases. **Clips** (Phase 1) are pure functions in `vxe-entity`, proven
on computed keys. **The files** (Phase 2) are the `.rig` parser and the voxel text tool in
`vxe-assets`. **The viewer** (Phase 3) loads a rig in the fly-through, plays it with a float
interpolation between ticks for display, reloads on change, and ships a sample walker. **The
window** (Phase 4) is the owner making a change and a new clip.

We rejected glTF animation, `.vox` frames and clips in code only (ADR-0051 A to C). We
rejected a viewer binary of its own: the fly-through already has the window, the lights and
the HUD, and one more app is one more thing to keep building.

## Implementation phases

Each phase ships as its own commit; `dev` implements the dev phases in one session; the
architect reviews once at the end, in a fresh session. Per phase: `node
scripts/test-changed.mjs --profile quick`, and `--profile gpu` under the perf lock for Phase
3, clippy, fmt. At the tip: the gate tier under the perf lock, doctests, clippy, fmt, deny,
the doc and tier gates. Readings go to `docs/plans/logs/0063.md`. Headless tests live in the
crates' `tests/`; the one GPU test goes in `apps/flythrough/tests/gpu_entities.rs`. **Test
times are estimates**, none measured.

### Phase 1: Clips in `vxe-entity`
- **Owner skill:** dev
- **What:**
  - **`Clip`**: a length in ticks, a `looped` flag, and per part a track of keys sorted by
    tick; a key holds a translation (`i32`, 1/256 voxel) and a rotation (Q14 quaternion).
    `Clip::new` refuses an unsorted track, a key past the length and a part index the model
    lacks.
  - **`sample(model, clip, tick) -> Pose`** (the root is left at identity for the caller to
    place): between two keys the translation is linear with rounding toward zero, the rotation
    a normalised linear blend along the shorter arc (the second key is negated when the dot
    product is negative). Before a track's first key and after its last, the nearest key
    holds. A part with no track keeps its rest pose. A tick past the length wraps when
    `looped` and holds the last tick otherwise.
  - **`blend(a, b, w) -> Pose`**, `w` from 0 to 256, the same two rules per part.
  - **`Playback`**: `{ clip, start_tick, rate }` with `rate` in 1/256; `tick_at(now)` is
    integer. The consumer owns it; the crate keeps no clock.
- **Files touched:** `crates/vxe-entity/src/clip.rs` (new), `lib.rs`, `quat.rs`,
  `tests/clip.rs` (new), `tests/determinism.rs`, `docs/plans/logs/0063.md` (new).
- **Done when:**
  1. `a_translation_key_pair_is_linear`: keys of 0 at tick 0 and 256 at tick 40 on one axis
     sample to 64 at tick 10 and 128 at tick 20. Under 1 s.
  2. `a_half_turn_key_pair_passes_through_a_quarter_turn`: keys of identity (16384, 0, 0, 0)
     at tick 0 and a half turn about x (0, 16384, 0, 0) at tick 40 sample at tick 20 to
     (11585, 11585, 0, 0) within 1 in each component (`16384 / sqrt(2)` = 11585.2), and at
     tick 10 to (15543, 5181, 0, 0) within 1 (`(0.75, 0.25)` normalised: the blend's uneven
     speed, 36.9 degrees where an even turn gives 45, stated here so nobody mistakes it for a
     bug). Under 1 s.
  3. `the_shorter_arc_is_taken`: a key pair whose quaternions have a negative dot product
     samples at the midpoint to the same rotation as the pair with the second key negated.
     Under 1 s.
  4. `a_loop_wraps_and_a_one_shot_holds`: on a 40-tick clip, tick 50 equals tick 10 when
     looped and equals tick 40 otherwise. Under 1 s.
  5. `blend_at_half_is_the_midpoint`: rest blended with a pose translated by 256 at `w` 128
     gives 128; `w` 0 and 256 return `a` and `b` bit for bit. Under 1 s.
  6. `clip_poses_match_the_fixed_vectors` (`tests/determinism.rs`): every tick of a committed
     three-part clip hashes to a committed constant, on Linux and Windows in CI. Under 1 s.
  7. `Clip::new`'s three refusals each return their typed error. Under 1 s.

### Phase 2: The `.rig` file and voxels from text
- **Owner skill:** dev
- **What:** in `vxe-assets`, which gains `vxe-entity` as a dependency.
  - **`.rig`**, line-based, `#` comments, one statement a line:

    ```
    # illustrative
    vox walker.vox
    rate 30                              # ticks per second, for viewers
    material 9 glow_amber                # palette index -> a material name
    part torso node torso pivot 4 0 3 detail 2
    part head  node head  parent torso at 0 12 0  pivot 3 0 3 detail 2
    part leg_l node leg   parent torso at -2 0 0  pivot 1.5 8 1.5 detail 2
    clip walk ticks 40 loop
    key 0  leg_l rot x 30
    key 20 leg_l rot x -30
    key 40 leg_l rot x 30
    ```

    Lengths are in model voxels and must be multiples of 1/256. A rotation is an axis and
    whole degrees; several `rot` on one key compose in the order written; degrees become Q14
    through a committed table of the sines of 0 to 90 degrees in half-degree steps (181
    entries), never the platform's `sin`. A part names its voxels by the `.vox` node's name
    or by `model <index>`.
  - **`Rig::parse(text) -> Result<Rig, RigError>`** with the line number and a message in
    every error; **`Rig::build(&VoxFile, materials: &dyn Fn(&str) -> Option<MaterialId>) ->
    Result<(Model, Vec<(String, Clip)>), RigError>`**. An unknown material name, node, parent
    or part is an error that names it. `Rig::write` prints a rig back in a canonical form.
  - **Voxels from text.** An example binary, `cargo run -p vxe-assets --example voxtext`:
    `voxtext to-vox in.voxtext out.vox` and `voxtext from-vox in.vox out.voxtext`. A
    `.voxtext` holds a legend (a character, a palette index, an rgb colour) and, per named
    model, its layers from the bottom up as rows of characters. It is how an agent draws a
    model and reads one the owner edited in MagicaVoxel.
- **Files touched:** `crates/vxe-assets/Cargo.toml`, `src/rig.rs` (new), `src/sines.rs` (new),
  `src/voxtext.rs` (new), `src/vox.rs` (node names, if `_name` is not kept at the start
  commit), `src/lib.rs`, `examples/voxtext.rs` (new), `tests/rig.rs` (new), `tests/voxtext.rs`
  (new), `CLAUDE.md` (the crate's line), `docs/plans/logs/0063.md`.
- **Done when:**
  1. `the_sample_rig_builds_its_model_and_clips`: the rig above against a three-node `.vox`
     built in the test gives a model of three parts, `head`'s parent `torso`, `leg_l`'s pivot
     (384, 2048, 384) in 1/256 voxel, detail 2, palette index 9 mapped to the material the
     lookup returns for `glow_amber`, and one looped clip `walk` of 40 ticks with three keys
     on `leg_l`. Under 1 s.
  2. `thirty_degrees_about_x_is_the_table_value`: `rot x 30` parses to (15826, 4240, 0, 0)
     within 1 (`16384 cos 15` = 15825.7, `16384 sin 15` = 4240.5). Under 1 s.
  3. `errors_name_their_line`: an unknown part in a `key`, a length of 0.3 (not a multiple of
     1/256), a fractional degree and a `parent` cycle each fail with the right line number and
     a message that quotes the token. Under 1 s.
  4. `a_rig_round_trips`: `parse(write(parse(text)))` equals `parse(text)` for the sample
     rig. Under 1 s.
  5. `the_parser_never_panics`: 10 000 seeded byte strings and every prefix of the sample rig
     return `Ok` or `Err`. Under 5 s (an estimate).
  6. `voxtext_round_trips_through_vox`: a two-model `.voxtext` to `.vox` to `VoxFile` gives
     the drawn voxels and node names; `from-vox` then `to-vox` gives identical bytes. Under
     1 s.

### Phase 3: The viewer, display interpolation and a sample model
- **Owner skill:** dev
- **What:**
  - **Display interpolation.** `vxe_render::DisplayPose::between(&Model, &Pose, &Pose, alpha:
    f32)` blends two ticks' composed transforms in `f32` for drawing; `Frame::set_entities`
    gains a variant that takes display poses. Queries never see it.
  - **The viewer.** `flythrough --model <file.rig> [--clip <name>]` loads the rig and its
    `.vox`; `7` then places that model in place of the built-in walker, playing the clip at
    the rig's rate from the frame's time step. `BracketLeft` and `BracketRight` cycle the
    clips, `Backslash` pauses and resumes. The fly-through checks the two files' modified
    times once a second and reloads on a change, keeping every placed instance; on an error
    the banner shows `rig line <n>: <message>` and the old model stays. The HUD gains a row
    `model <name> clip <clip> <tick>/<length> [paused]`; the banner names the clip on a
    cycle and `reloaded` on a reload. Material names resolve against the fly-through's
    material table.
  - **The sample.** `apps/flythrough/assets/walker.voxtext`, the `walker.vox` generated from
    it (committed, and a test regenerates and compares it), and `walker.rig` with clips
    `idle` and `walk`, eyes mapped to the fly-through's emissive material.
- **Files touched:** `crates/vxe-render/src/entities.rs`, `lib.rs`,
  `apps/flythrough/src/main.rs`, `hud.rs`, `viewer.rs` (new), `apps/flythrough/Cargo.toml`
  (`vxe-assets`, if absent), `apps/flythrough/assets/walker.voxtext`, `walker.vox`,
  `walker.rig` (new), `apps/flythrough/tests/gpu_entities.rs`, `tests/viewer.rs` (new), a
  baseline under `tests/golden/`, `docs/plans/logs/0063.md`.
- **Done when:**
  1. `display_between_two_ticks_is_between_them`: for a part translated 0 at one tick and 256
     at the next, `alpha` 0.25 puts its transform's translation at 0.25 voxel within 1e-4;
     `alpha` 0 and 1 equal the two ticks' transforms within 1e-6 (`vxe-render`, no device).
     Under 1 s.
  2. `the_sample_assets_are_in_step`: `walker.voxtext` converts to bytes identical to the
     committed `walker.vox`, and `walker.rig` builds against it with the fly-through's
     material names (no device). Under 1 s.
  3. `a_changed_rig_reloads_and_a_broken_one_keeps_the_old` (`tests/viewer.rs`, no device):
     the viewer's reload step, driven with injected file contents, swaps the model on a valid
     change and reports `rig line 9` with the old model kept when line 9 is broken. Under 1 s.
  4. `walker_walk_golden`: the sample walker at tick 10 of `walk` on Plan 0062's lit box, 192
     x 108, `--lighting half`, blessed once on lavapipe. About 3 s (an estimate).
  5. The HUD and banner tests cover the row, the cycle, the pause and `reloaded` (no device,
     under 1 s); `--model` and `--clip` parse, and a missing file is a startup error that
     names the path.
  6. The readings: `sample` plus the transform composition for 64 sixteen-part entities on
     the dev box (a `mod reading` test; the prediction: under 0.05 ms, a guess); the frame at
     `--entities 16` with the sample walker walking against Plan 0062's walker.

### Phase 4: The owner's window
- **Owner skill:** human
- **What:** one sitting with an editor beside the fly-through.

  ```
  cargo build --release -p flythrough
  target/release/flythrough --source showcase --shot hall --exposure 8 --hud \
      --model apps/flythrough/assets/walker.rig --clip walk
  ```

  Place the walker (`7`), change a leg's swing in `walker.rig` and save; break a line and
  save; ask the agent for a new clip (a head turn) and a changed model (longer arms, through
  `walker.voxtext`), and watch both arrive; open `walker.vox` in MagicaVoxel, repaint the
  eyes, save, and watch that arrive.
- **Files touched:** this plan's log.
- **Done when:** the log records the owner's words on: (a) the loop's speed, save to
  picture; (b) whether typed keys are enough for a walk and a head turn, or a timeline is
  wanted; (c) the motion between ticks (smooth or stepped); (d) the error banner; (e) whether
  the round trip through MagicaVoxel kept what it should. **Stop rule:** a "no" on (b) goes
  to the architect, and ADR-0051's Alternative A (glTF from Blender) is the first thing
  re-read. The plan closes with that record.

## Budgets

| `nfr.md` figure | Base | Expected | How the close measures it |
|---|---|---|---|
| §2 streaming and CPU line | the start commit's frame | **a guess:** sampling and composing 64 sixteen-part entities under 0.05 ms on the dev box | Phase 3's `mod reading` test, the perf lock |
| §6 determinism | Plan 0062's vectors | clip poses equal on Linux and Windows | CI, Phase 1 done-when 6 |
| §7 gate wall time | the start commit's gate | **an estimate:** + about 15 s | `node scripts/test-times.mjs` on the tip's gate log |

No GPU memory is added: a clip changes transforms, not volumes.

## Risks & open questions

- **Typed keys may be too slow to author.** The plan bets that a dozen keys make a walk and
  that an agent writes them well enough. Phase 4 (b) is the test, with its stop rule.
- **The blend's uneven speed.** Keys far apart turn unevenly (Phase 1 done-when 2). The rig
  parser warns on a key pair more than 90 degrees apart.
- **`.vox` node names.** A part finds its voxels by name; MagicaVoxel lets a node go unnamed
  and lets two share a name. An ambiguous or missing name is a parse error that says so, and
  `model <index>` is the way round it.
- **Reload while instances are placed.** A reload that changes the part count invalidates
  held poses; the viewer re-samples every instance on reload. A consumer's hot reload is its
  own business.
- **The modified-time poll reads the wall clock.** It lives in the fly-through, not in a
  crate under `crates/`.
- **Open, for the owner:** the file's name (`.rig`) and whether clips should live in a
  second file so one rig can share them.

## What this plan does NOT do

- An editor, a timeline, a curve view or a gizmo. The viewer shows; a text editor and
  MagicaVoxel edit.
- glTF, FBX or Blender import (ADR-0051 A, the route if Phase 4 says typed keys fail).
- Animation state machines, layers, inverse kinematics, root motion, events on a tick: the
  consumer builds them from `sample`, `blend` and `Playback`.
- Deforming parts, or swapping a part's volume per frame.
- Placing models on a level or saving a placement: the consumer's. The viewer's placements
  are gone when it exits.
- An asset pipeline: no packing, no cache, no build step. A consumer loads a `.rig` and a
  `.vox` at startup.
- Replicating clips or playback over the network (M9).

## Owner answers

2026-11-10, at the draft:

1. **Animation:** the engine plays keyframe clips (this plan; Plan 0062 takes the poses).
2. **Authoring:** the owner makes models or asks the agent to; no editor is built (the
   architect's recommendation; the owner: "agreed").
3. **The order:** after Plan 0062, ahead of M10.

The "go" is given (owner, 2026-11-10: "move all draft plans to approved"). Open: the two
questions under Risks.

## Implementation log

> Written by `dev`: one row per phase as its commit lands, then the close block. **The phases
> above are the contract. This section records what happened.** Observations, never conclusions:
> no pass list, no self-assessment. Deviations and unmet done-whens are **always** disclosed.
> Keep it shorter than `## Implementation phases`. Readings, gate counts, attempts, sweeps and
> resume notes go to `docs/plans/logs/0063.md`.

Readings and the name mapping: [`logs/0063.md`](logs/0063.md).

| phase | owner | state | commit |
|---|---|---|---|
| 1: Clips in `vxe-entity` | dev | done | 7aed6b7 |
| 2: The `.rig` file and voxels from text | dev | done | committed with this row |
| 3: The viewer, display interpolation and a sample model | dev | not started | |
| 4: The owner's window | human | not started | |

### Notes

_(Deviations from the plan, with the commit, stated without justification. Done-whens not
satisfiable as stated, and what was done instead. Followups noticed and not acted on. One line
each. Empty is fine.)_

- Phase 1: `sample` takes a rest `Pose` (`sample(model, rest, clip, tick)`), and a key's
  translation is an offset from the rest translation; `Part` has no rest offset (Plan 0062's
  option (a)). `Clip::new` also refuses a zero length, an empty track and two tracks for one
  part; equal ticks in a track count as unsorted. `Playback` holds an `Arc<Clip>`.
- Phase 2: `Rig::build` returns `RigModel { model, part_names, rest, clips }` (the rest pose
  added, the plan's tuple a struct). `VoxFile` gains `names` (node `_name`s), which also
  touched `apps/flythrough/src/castle.rs` and `crates/vxe-assets/tests/vox.rs` (one
  `names: Default::default()` each, outside the phase's files). The `.rig` grammar adds
  `move x y z` on a key, `at` measured in the parent's model voxels, and parents declared
  later in the file; `.voxtext` is `color` / `model <name> [at x y z]` / `layer` / rows.

### Close triggers

_(Facts for the architect. No recommendations, and no suggested version bump.)_

- **What shipped:** feature / fix-only / docs-chore-only
- **Public API changed:** types, traits, functions, shader interfaces, config (list them, or none)
- **Budget readings:** each `nfr.md` figure touched, with its measured value, machine and commit
- **Gate at the tip:** the commands run (build, nextest, doctests, clippy, fmt, deny), exit codes, test counts
- **Outstanding `human` phases:** which, or none

## Followups
