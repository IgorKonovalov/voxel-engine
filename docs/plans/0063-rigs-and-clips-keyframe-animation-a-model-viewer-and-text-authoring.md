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

> **Amended 2026-11-11 (architect, owner away; default taken, confirmed by the owner the same
> day): the rest pose.** Plan 0062's Phase 1 review found the gap: the `.rig` gives each part a rest place
> (`at`), Plan 0062's answer 6 puts a child's rest layout in `PartPose::t` (`Part` has no rest
> field), but the draft's `Rig::build` returned no rest pose and its `sample` let an untracked
> part "keep its rest pose", which `Pose::rest` sets to zero: the part would sit on its
> parent's pivot. **The rule:** (1) `Rig::build` returns a rest `Pose` beside the model and
> clips, each part's `at` in its `PartPose::t` (1/256 world voxel), every rotation the
> identity, the root at the origin; (2) `sample` and `Playback::pose` take that rest pose and
> start from it, so a part with no track copies its rest `PartPose`; (3) a key's translation
> is an **offset added to** the rest `t`, and a key's rotation **replaces** the rest
> rotation. Replace, not compose: the `.rig` grammar has no rest rotation, so every rest that
> `Rig::build` makes is unturned and the two rules agree for every rig; replacing keeps a key's
> angle the angle the file states and the angle `blend` mixes, costs no quaternion product per
> part per tick, and gives a consumer-built rest with a turned joint no second, hidden turn.
> If the grammar ever gains a rest rotation, this choice is re-read. **Rejected:** a rest field
> on `vxe_entity::Part` (contradicts answer 6, changes `Model::id` and Plan 0062's committed
> fixed vectors, and reopens a merged crate); absolute key translations (every key repeats its
> part's `at`, and editing `at` would not move an animated part, so the file holds two sources
> for one place); baking `at` into a track for every part at build (an entity placed before a
> clip plays, and `blend` from rest, still need the laid-out pose). The phases below carry the
> rule, Phase 1 done-when 8 and Phase 2 done-when 7 defend it, and the tree's names replace
> the draft's (Notes under Implementation log; mapping in
> [`logs/0063.md`](logs/0063.md)). This is owner point 5 below. Phases 1 and 2 were built to
> this rule on this branch (7aed6b7, 5c87d16) before the amendment was written; it records the
> contract, it does not change their code. Other names from Plan 0062 Phase 1's tree: a
> "Q14 quaternion" is `Quat([i16; 4])`; a pose's root is `root: Fixed3` and `root_q: Quat`;
> no phase here uses `EntityHit::face: Option<Face>`. Phase 3's `DisplayPose` and
> `Frame::set_entities` belong to Plan 0062's later phases, not in the tree yet; the tree's
> names win when Phase 3 starts.

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
  - **`Clip`**: a length in ticks, a `looped` flag, and per part a `Track` of `Key`s sorted by
    tick; a `Key { tick: u32, t: [i32; 3], q: Quat }` holds a translation **offset from the
    part's rest `t`** (1/256 world voxel, along the parent's turned axes) and a rotation (the
    tree's `Quat([i16; 4])`, Q14, `Q14_ONE` = 16384) that **replaces** the rest rotation.
    `Clip::new` refuses an unsorted track, a key past the length and a part index the model
    lacks.
  - **`sample(model, rest: &Pose, clip, tick: u64) -> Pose`** (the root is left at the
    origin, `root_q` the identity, for the caller to place): it starts from `rest` (the rest
    pose `Rig::build` returns, or any the consumer lays out). Between two keys the translation
    is linear with rounding toward zero, the rotation a normalised linear blend along the
    shorter arc (the second key is negated when the dot product is negative); the result is
    the rest `t` plus the sampled offset, and the sampled rotation. Before a track's first key
    and after its last, the nearest key holds. A part with no track copies its `rest`
    `PartPose` bit for bit. A tick past the length wraps when `looped` and holds the last tick
    otherwise. `Pose::rest(model)` (every `t` zero) is a valid `rest` for a model whose parts
    all share a pivot place, as the tests below use it.
  - **`blend(a, b, w) -> Pose`**, `w` from 0 to 256, the same two rules per part.
  - **`Playback`**: `{ clip: Arc<Clip>, start_tick, rate }` with `rate` in 1/256;
    `tick_at(now)` is integer; `pose(model, rest, now)` samples at it. The consumer owns it;
    the crate keeps no clock.
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
  8. **A part with no track keeps its rest place, and a key adds to it** (amended 2026-11-11;
     asserted in `a_translation_key_pair_is_linear` at 7aed6b7): with a rest pose whose part 0
     sits at `t` (5, 6, 7) and part 1 at (1000, 0, -3), and a clip that keys only part 1 from
     0 to (256, 0, 0) over 40 ticks, tick 10 gives part 0 equal to its rest `PartPose` (not the
     zero `t` of `Pose::rest`) and part 1 at (1064, 0, -3). Under 1 s.

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
    Result<RigModel, RigError>`**, `RigModel { model, part_names, rest: Pose, clips:
    Vec<(String, Clip)> }`. **Where `at` goes:** a part's `at` is its pivot's rest place from
    its parent's pivot, in the parent's model voxels (the part's own for a root part); build
    divides it by that detail into 1/256 world voxel and writes it to the part's `t` in
    `rest`; `rest`'s rotations are the identity and its root the origin. A key's `move x y z`
    is an offset from that place in the same units, and its `rot` replaces the rest rotation
    (the amendment above). An unknown material name, node, parent or part is an error that
    names it. `Rig::write` prints a rig back in a canonical form.
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
  7. **`at` lands in the rest pose and an untracked part stays there** (amended 2026-11-11;
     asserted in `the_sample_rig_builds_its_model_and_clips` at 5c87d16): the sample rig's
     `rest` holds `torso` at (0, 0, 0), `head` at (0, 1536, 0) and `leg_l` at (-256, 0, 0)
     (12 and -2 model voxels at `torso`'s detail 2, 128 each); `sample(model, rest, walk, 10)`
     gives `head` (no track) equal to its rest `PartPose`, not (0, 0, 0) on `torso`'s pivot,
     and `leg_l` at (-256, 0, 0) with an unturned rotation within 1 (the midpoint of +30 and
     -30 degrees about x). Under 1 s.

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
     material names (no device); every part with an `at` other than zero has that place in
     the built `rest`, and at tick 0 of `idle` every part `idle` does not key equals its rest
     `PartPose`. The viewer samples with the built `rest` and places an instance with it
     before a clip plays. Under 1 s.
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

4. **The rig's axes (owner, 2026-11-11, the architect's recommendation).** z is up: a rig's
   lengths are in the prefab's axes as `VoxMap::to_prefab` lays a `.vox` model out, as
   MagicaVoxel and `vxe-sim`'s controller do. The rest offset is Plan 0062's answer 6.

5. **The rest pose (owner, 2026-11-11, confirmed the default taken while away).**
   `Rig::build` returns a rest `Pose` holding each part's `at`; `sample` starts from it; key
   translations add to the rest place and key rotations replace the rest rotation (the
   amendment under the header, with the rejected alternatives). The owner also confirmed
   answer 4's z-up over the sample rig's y-up: Phase 3's walker rig is drawn z-up.

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
