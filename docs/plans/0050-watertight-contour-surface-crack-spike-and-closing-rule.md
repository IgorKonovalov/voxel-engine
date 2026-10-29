# 0050: A watertight contour surface: a crack spike that names the wedges and triangles, then the closing rule chosen by measurement

> **Status:** in-progress (2026-10-29)
> **Created:** 2026-10-29
> **Approved:** 2026-10-29 (owner), with the answers under "Owner answers": alone on main
> with the light spike after its Phase 4 check; the contour goldens re-blessed by name; the
> shadow origin in scope as sub-candidate O; the losing rules and `closing old` removed after
> the pick.
> **Amended:** 2026-10-29 (owner, after Phase 1's stop on done-when 2): the clean-surface
> control is the `(1, 1, 1)` slab; analytic bars leave out hits near the mirror window's faces;
> a new Phase 2 puts the plane code's normal on codes 0..=62 so a zero component is exact
> (ADR-0038, proposed); the closing phases follow as Phases 3 to 6. Owner answers 5 to 7.
> **Milestone:** M2 in [`docs/roadmap.md`](../roadmap.md) (the surface-look decision: "smooth
> and natural rock, not visible cubes")
> **Related ADRs:** [ADR-0011](../adrs/0011-contour-planes-in-attribute-pages-with-procedural-detail.md)
> (the per-voxel plane and its hit rule, which this plan closes the cracks of; its Negative
> consequences name the crude offset and the shadow-ray artifacts);
> [ADR-0038](../adrs/0038-the-contour-plane-code-holds-a-zero-normal-component-exactly.md)
> (proposed: the plane code's scale, Phase 2). The closing rule's ADR comes before Phase 4:
> see Decision.
> **Source:** the owner at Plan 0049's live session 2, 2026-10-29 (`LIGHT-FEEDBACK.md`,
> finding 5; screenshots `wedges`, `step3` and `step5` in
> `.git/lane-archive/plan-0049/session2/`): dark triangular wedges where a ledge meets the wall
> below it, small bright triangles exactly where the facets of the smoothed floor meet, and
> thin dark and light slivers along ledge edges inside the lamp's cone. They do not flicker,
> and they are the same with indirect light off. At Plan 0039's Phase 4 the owner had seen
> "artifacts with sharp corners of voxels on some smooth walls".
> **Starts after:** nothing; it runs on main at f0dd17a, alone. Plan 0049 stays parked at its
> Phase 2 (its worktree at 121737f keeps its uncommitted `live/` files). The light spike the
> architect proposed on 2026-10-29 follows this plan's owner check, Phase 5 since the
> amendment (owner, 2026-10-29).

## TL;DR

The contour surface is not watertight. Every surface voxel carries its own plane, and the walk
hits a voxel only where the ray enters that plane's solid half-space inside the voxel's own
cube. Two neighbours' planes do not meet at their shared cube face, so a ray can pass between
them. It then either reaches an interior voxel a voxel deeper, which hits as a dark cube face,
or enters the next surface voxel already inside its half-space and shades with that voxel's
normal, which can face the lamp. Those are the owner's wedges and triangles (read from the
code, not yet measured: Phase 1 measures it). The plan is a short spike that classifies every
primary ray by how it reached its hit and shows the owner the classes as a view, then three
candidate closing rules behind one setting, each measured on analytic scenes and on the two
rig poses, with the cheapest that closes the cracks and holds the look guardrails kept as the
default. Nothing in storage changes: every candidate lives in the walk. The first thing anyone
sees is a `cracks` view in the fly-through that paints each pixel by its class, and a test that
prints the class table at the vein and the patch.

## Context & problem

`project-context.md` (Rendering, P0): first person, stable at arm's length, smooth natural
rock. A dark wedge at every ledge corner and a bright speck at every facet vertex read as a
rendering fault, and they would survive any lighting fix.

**What the tree does today** (ADR-0011; `march.wgsl` `contour_test`, `attrib.wgsl`
`voxel_attr`; read, not measured):

- The attribute pass gives each surface voxel (a solid voxel with an air face neighbour) a
  plane: the normal from the 27-voxel occupancy gradient (weighted, the default since Plan
  0039), the offset `(count / 27 - 0.5) * 3` voxels along the normal from the voxel centre,
  clamped to `[-0.875, 0.875]` and coded in eighths. A solid voxel whose six face neighbours
  are solid, or whose gradient vanishes, has no plane and hits as a cube.
- The walk tests a solid voxel that has a plane against the ray's segment through the voxel's
  cube alone. Signed distance `s_in` at the entry and `s_out` at the exit: `s_in <= 0` is a
  hit at the entry with the plane's normal; `s_in > 0` and `s_out <= 0` is a hit at the
  crossing; `s_out > 0` passes the voxel as air.
- The shadow ray of a plane hit leaves `SHADOW_OFFSET` 0.5 voxel along the plane normal
  (Plan 0013), from a point that lies inside the voxel's cube.

**The three ways a ray reaches a wrong hit**, all after passing at least one surface voxel
outside its half-space (the plan calls them classes; Phase 1 counts them):

- **(i) pass, then an interior voxel.** The next solid cell has no plane (an interior voxel,
  a uniform brick) and hits as a cube face about a voxel below the surface. Its face normal
  points along an axis, its shadow origin sits in the crack, so it is dark. At a concave corner
  the surface voxels have diagonal normals and a high solid fraction, so their offsets push the
  planes toward the air and the half-space can miss the cube entirely: the corner voxel acts as
  air, and the ray sees the voxel behind it. That is the wedge where a ledge meets a wall.
- **(ii) pass, then an entry already inside.** The next surface voxel's `s_in` is already
  negative at its cube face, so the hit is on the shared face with that voxel's normal. The
  ray crossed that plane inside the passed voxel, where the passed voxel's plane said air.
  The normal belongs to a different facet and can face the lamp while the point lies in a
  crevice: the bright triangles at the vertices where three or more facets meet, and the
  slivers along convex edges, whose voxels have a low solid fraction and an inward offset.
- **(iii) pass, then a plane crossing** inside the next voxel. The hit is on a plane, a little
  late. It is a step in the surface, not an artifact on its own.

**The code adds its own steps (measured in Phase 1, 2026-10-29).** The plane's octahedral
coordinates sit on codes `0..=63`, where 0 falls at 31.5, so no decoded normal has a zero
component. Neighbouring surface voxels with the same neighbourhood get parallel planes that step
by about 1/63 voxel per voxel across the lean: a 45-degree slab reads 0.035 % of hits in class 2,
all on faces between y-neighbours, and a `(1, 1, 1)` slab, whose normal the code holds exactly,
reads 0. Phase 2 fixes the code (ADR-0038).

The shadow ray adds a fourth: a plane hit's origin, 0.5 voxel along its normal, can lie inside
a neighbouring voxel's solid half-space, so the ray reports rock at once. Plan 0013 took the
contour scene's only-with-shadows specks from 372 to 52 with the plane-normal origin and did
not take them to 0. The dark slivers in the lamp's cone may be these.

**The `nfr.md` figures touched:** §2 the primary rays (the hit pass, 3.0 ms line; the level's
spawn reads 1.378 ms contour on the RTX 3080 Laptop and 9.908 ms on the Radeon, Plan 0039),
and nothing in §3: no candidate adds storage.

## Decision

Measure first, then close the cracks in the walk. Phase 1 builds the classification and the
view, reads the classes on analytic scenes and the two rig poses, and shows the owner that the
classes land where the wedges and triangles are. Phase 2 removes the code's own steps: the
plane normal's octahedral coordinates move to codes `0..=62`, so a zero component is exact
(ADR-0038); the attribute's size, pages and pass do not change (Plan 0028's addressing
stands). Phase 3 builds the three candidates under one `SurfaceTuning` setting and keeps the
cheapest that brings classes (i) and (ii) to zero on the analytic scenes, holds Plan 0039's
look guardrails and the speck bar, and stays within its cost bar on both adapters. No golden
outside the contour look moves and no VRAM figure moves.

**The analytic window rule.** An analytic scene's surface is cut where it meets the mirror
window's faces, and the surface voxels there see a cut neighbourhood. Every analytic bar in
Phases 1 to 4 counts only hit pixels whose hit point lies at least 2 voxels inside the window
on every axis; each reading prints how many hits it left out. The level poses are a cave and
take every hit.

**The closing rule's ADR.** The closing rule is a revisitable tradeoff with named
alternatives, so the architect writes the next free ADR as an amendment before Phase 4 lands
the pick as the default, naming the kept rule and the two rejected ones with Phase 3's
readings. `docs/adrs/README.md` has the number.

## Architecture diagram

```mermaid
flowchart LR
    subgraph vxe-render
        A[attrib.wgsl: plane per surface voxel, code 0..62] --> P[pool attribute pages, unchanged]
        P --> W[march.wgsl: the walk]
        W --> T[contour_test: hit, pass or none]
        T --> C[closing rule: Old, Snap, Extend or Blend]
        C --> H[hit buffer: position, plane normal, class in the cracks view]
        H --> L[light.wgsl: shadow origin]
    end
    subgraph flythrough tests
        H --> R[class table per scene and pose, look guardrails, specks, hit-pass cost]
    end
```

## Implementation phases

Each phase ships as its own commit. `dev` implements all phases in one session with no review
between phases. The architect reviews once at the end, in a fresh session. Lane rule 9c
applies: exploratory sweeps may run on the RTX 3080 Laptop through `VXE_TEST_ADAPTER` once
Plan 0049's `reading_device` is on main (it is on branch `plan-0049` today; until it merges,
dev may copy the function into `tests/support/mod.rs` and the merge keeps one), labelled with
the adapter; every asserted reading, golden and gate stays on lavapipe.

### Phase 1: The crack spike: classes, the view, and where they land
- **Owner skill:** dev
- **What:** the walk records how each primary ray reached its hit, a `cracks` view paints it,
  and a test prints the class table on analytic scenes and at the rig poses. No hit, shader
  default or look changes.
- **The classification** (`march.wgsl`): the walk counts the surface voxels it passed outside
  their half-space (`contour_test` kind 2) since the last cell it entered through air, and at
  the hit records the class: `0` clean (no pass, or a plane crossing with no pass), `1` class
  (i) (a pass, then a cell with no plane: an interior voxel, a uniform brick, a mip cell), `2`
  class (ii) (a pass, then `s_in <= 0` at the next voxel's entry), `3` class (iii) (a pass,
  then a crossing inside the next voxel). It goes out through the hit pass's layer texture in
  the new view only, as the steps view carries its counters, so the hit record does not change.
- **The shadow self-hit** (`light.wgsl`): in the `cracks` view the light pass writes, per
  lighting pixel, whether the lamp's shadow ray found rock within `SELF_HIT` 1.5 voxels of its
  origin, beside the class. The threshold is three times `SHADOW_OFFSET`: a true occluder that
  close is a neighbouring voxel's own plane.
- **The view:** `ViewMode::Cracks` in the fly-through's view cycle, after `steps`. The
  composite paints class 0 as the shaded image at half brightness, class 1 red, class 2
  yellow, class 3 blue, and a self-hit as a magenta dot. The banner names the view, and the
  HUD prints the four classes' shares of hit pixels and the self-hit share each frame.
- **The scenes** (`tests/look.rs` and `tests/support/rigs.rs`, reusing the look rigs):
  - the cavity (a sphere) and the pillar of Plan 0005, as they are;
  - the `(1, 1, 1)` slab of done-when 2 and the 45-degree slab, as readings;
  - **a tilted slab:** solid below the plane with normal `normalize(1, 2, 3)` through the
    box's centre, so neighbouring surface voxels see different 27-neighbourhoods and their
    offsets disagree (an axis-aligned 45-degree slab gives every voxel the same neighbourhood
    and no crack, which is why it is not the scene);
  - **a concave corner:** two solid slabs meeting at 90 degrees, seen from inside the corner
    at a pitch of about -30 degrees, the wedge case;
  - the level at `VEIN_POSE` and `PATCH_POSE` (`flythrough::VEIN_POSE`, `PATCH_POSE` on
    branch `plan-0049`; until that branch merges, the poses are copied as constants and the
    merge keeps one).
- **The readings** per scene, lamp on, at the look rigs' size and at 384 x 216 for the level:
  the share of hit pixels in each class; the share of class 1 and 2 pixels whose display luma
  is under 32/255 (dark) or over the 5 x 5 median by more than 48/255 (bright); the self-hit
  share; and for the level poses a 24 x 24-tile map of the class 1 and 2 density beside the
  display image, written as a side-by-side under `live/cracks/`, so the owner can see that the
  wedges and the triangles are where the classes are.
- **Sweeps by scratch patch** (readings only): `PlaneOffset::VolumeMatched` against
  `Fraction`, `PlaneFit::Uniform27` against `Weighted27`, and the offset clamp at 0.5 against
  0.875. The question each answers: does an existing knob remove a class?
- **Files touched:** `crates/vxe-render/src/shaders/march.wgsl` (the count and the class),
  `crates/vxe-render/src/shaders/light.wgsl` (the self-hit in the view),
  `crates/vxe-render/src/shaders/composite.wgsl` (the view), `crates/vxe-render/src/marcher.rs`
  (`ViewMode::Cracks`), `apps/flythrough/src/main.rs` and `apps/flythrough/src/hud.rs` (the
  cycle, the banner, the shares), `apps/flythrough/tests/support/rigs.rs` (the two scenes),
  `apps/flythrough/tests/look.rs` (the readings), this plan's log.
- **Done when:**
  1. `a_tilted_slab_has_cracks_today` asserts on lavapipe that the tilted slab in today's
     contour look has at least one pixel in class 1 or 2 (the reading is printed). If it has
     none, that is the finding and dev stops for the architect: the mechanism named above is
     not the one. (Met 2026-10-29: 3.814 % class 2, 0.006 % class 1.)
  2. `a_diagonal_slab_has_none` asserts on lavapipe that a slab of normal `normalize(1, 1, 1)`
     through the box's centre, seen nearly head-on, reads 0 in classes 1 and 2 under the
     analytic window rule. Its surface voxels (`x + y + z` one value) share one
     neighbourhood, and today's code holds `(1/3, 1/3)` exactly (code 42 of 63), so they
     share one plane: it pins that the classification does not count a clean surface. The
     45-degree slab (`normalize(1, 0, 1)`) is a reading in `crack_classes`, not a bar: the
     code leans its normal off the y plane, and its class 2 pixels are the code's steps.
  3. `crack_classes` prints the table for every scene and pose above, today's look and each
     sweep, with the dark and bright shares and the self-hit share. The log carries it.
  4. The side-by-sides exist for both poses, and the log says in one line per pose whether the
     class map lands on the owner's wedges and triangles, or where it does not.
  5. The fly-through cycles into the `cracks` view, the banner names it, and the HUD prints
     the shares; the banner test reads the new mode.
  6. The attribution in the log names the class that owns the wedges and the class that owns
     the triangles and slivers, and whether the self-hit owns any dark sliver inside the
     lamp's cone.

### Phase 2: A zero normal component, exact in the plane code
- **Owner skill:** dev
- **What:** the plane normal's octahedral coordinates move to codes `0..=62` (ADR-0038): the
  encoder writes `round((p + 1) * 31)` clamped to 62, the decoder reads `q / 31 - 1`, and the
  mirror tests' CPU decoder follows. The offset code, the attribute's size and pages and the
  pass are as they were. The clean-surface control moves to the axis planes, which the new code
  holds exactly; the `(1, 1, 1)` slab, which it no longer holds exactly (`q = 41.33`), becomes a
  reading.
- **Files touched:** `crates/vxe-render/src/shaders/attrib.wgsl` (`encode_octahedral`),
  `crates/vxe-render/src/shaders/march.wgsl` (`decode_octahedral`),
  `crates/vxe-render/tests/mirror.rs` (`decode_attr`, the tests that pin a code),
  `apps/flythrough/tests/support/rigs.rs` (a floor and a wall scene),
  `apps/flythrough/tests/look.rs`, `apps/flythrough/tests/golden/*.png` (the contour goldens
  that move), this plan's log.
- **Done when:**
  1. `axis_planes_have_no_cracks` asserts on lavapipe that a flat floor (rock at `z < 64`) and
     a vertical wall (rock at `x >= 80`), each seen obliquely in the start look, read 0 in
     classes 1 and 2 under the analytic window rule. It replaces `a_diagonal_slab_has_none` as
     the control; the diagonal slab's reading stays in the table.
  2. A mirror test asserts that a floor's and a wall's surface voxels decode to the exact axis
     normal (`(0, 0, 1)`, `(-1, 0, 0)`, every component bit-exact), and every test that pinned
     a code is updated with the old and new code in the log.
  3. Phase 1's class table is re-read for every scene and pose beside Phase 1's, with Plan
     0039's guardrails (cavity normal error at most 12 degrees, pillar silhouette error at most
     half of face mode's, `A` at most 10 %) and the speck test's reading.
  4. Every contour golden that moved is re-blessed and named in the log with its share of
     pixels over 2/255; the face goldens read byte for byte.

### Phase 3: The closing rules, each measured
- **Owner skill:** dev
- **What:** three candidates behind `SurfaceTuning::closing`, old behaviour selectable; keep
  the one the decision rule names.
- **Candidates** (all in `contour_test` and the walk's step; the attribute is read as today):
  - **S, snap.** The walk keeps the last passed voxel's plane and its exit point. When the
    next solid cell would be class 1, the hit is at the passed voxel's exit point with the
    passed plane's normal. When it would be class 2, the hit is at the shared face with the
    normalised mean of the two planes' normals. Class 3 is unchanged. No extra reads; a few
    registers. It removes the depth and the foreign normal and keeps a normal step at the
    crack, which is the faceting the surface has anyway.
  - **E, extend.** The plane is tested against the ray's segment through the cube dilated by
    `e` voxels on every side (swept at 0.25 and 0.5), and a crossing found in the dilation hits
    at the cube's exit point. It closes cracks narrower than `e`; wider ones stay. It can fatten
    a silhouette by up to `e`, which the pillar's silhouette error reads.
  - **B, blend.** Inside a surface voxel the signed distance at a point is the trilinear blend
    over the eight voxel centres around it of each voxel's plane distance there, with an air
    voxel counting `+1` and a solid voxel with no plane `-1` (a cube hit for a voxel whose
    neighbours are all no-plane, as today). The surface is the blend's zero set, continuous
    across cube faces, so it is watertight by construction. The crossing is bracketed by the
    entry and exit values and refined by two secant steps; the normal is the blend of the
    planes' normals at the crossing. Up to eight attribute reads per surface voxel stepped,
    which is the cost reading.
  - **O, the shadow origin,** a sub-candidate for the self-hit: the origin is pushed along the
    normal, in steps of 0.25 voxel up to 1.0, until it leaves every neighbouring plane's
    half-space, read with the kept rule's distance.
- **The decision rule:**
  1. Candidates are read in the order S, E, B. The first that meets all of the following is
     kept:
     - classes 1 and 2 read 0 on the tilted slab, the concave corner and the cavity, under
       the analytic window rule;
     - at both level poses classes 1 and 2 together read at most 2 % of Phase 2's count (a
       guess at draft, to be replaced by the owner's call in Phase 5 if the eye asks for
       less);
     - Plan 0039's guardrails hold: cavity normal error at most 12 degrees, pillar silhouette
       error at most half of face mode's, axis fraction `A` at most 10 %;
     - the speck test's asserted bar holds, and its reading is at most Phase 2's;
     - the hit pass at the level's spawn, 1080p, release, exclusive perf lock, is within +5 %
       of the pre-plan binary on the RTX 3080 Laptop and within +10 % on the Radeon, alternated
       in one session, three quiet runs each.
  2. If S or E meets every bar but the two-percent one and B meets it, B is kept and its cost
     reading goes to the owner as a recorded miss only if it misses the cost bar.
  3. O is kept beside the kept rule if the self-hit share falls and the speck reading does
     not rise.
  4. If no candidate meets the bars, dev stops for an architect amendment.
- **Files touched:** `crates/vxe-render/src/shaders/march.wgsl`,
  `crates/vxe-render/src/shaders/light.wgsl` (O), `crates/vxe-render/src/marcher.rs`
  (`CrackClosing`, the flag bits), `apps/flythrough/src/lib.rs` and `main.rs` (`--closing`,
  the banner), `apps/flythrough/tests/look.rs`, this plan's log.
- **Done when:**
  1. The class table of Phase 1 is in the log for every candidate beside Phase 2's, with the
     guardrails, the speck reading and the hit-pass cost on both adapters, the pre-plan binary
     in the same session.
  2. The log names the kept rule and the readings that kept it, or the stop.
  3. With `closing old`, every golden and every look test reads byte for byte as at Phase 2's
     commit.

### Phase 4: The pick as the default
- **Owner skill:** dev
- **What:** the kept rule becomes the default, the contour goldens that moved are re-blessed
  by name, and the readings go to `nfr.md`. The old rule and the losing candidates stay
  selectable until Phase 5, each named in the banner.
- **Files touched:** `apps/flythrough/src/lib.rs` (the default), `apps/flythrough/tests/golden/*.png`
  (only those that moved), `apps/flythrough/tests/look.rs`, `docs/nfr.md` §2, this plan's log.
- **Done when:**
  1. `the_closing_rule_leaves_no_crack` asserts classes 1 and 2 at 0 on the tilted slab, the
     concave corner and the cavity under the analytic window rule, and at or under the kept
     count at both level poses, on lavapipe.
  2. Every moved golden is named in the log with its share of pixels over 2/255, and no golden
     outside the contour look moved.
  3. The hit-pass readings on both adapters are in `nfr.md` §2 with machine, commit and load.

### Phase 5: The owner's windowed check
- **Owner skill:** human
- **What:** the owner runs the release fly-through at `--pose vein` and `--pose patch`, lamp
  on and off, in the kept rule and in `closing old`, and in the `cracks` view, looking at the
  ledge corners, the floor's facet vertices and the ledge edges inside the lamp's cone.
- **Files touched:** this plan's log.
- **Done when:** the owner records one of three outcomes: (a) the wedges, triangles and
  slivers are gone and the default stands; (b) another candidate or sweep value looks better,
  and it becomes the default; (c) something remains, named by pose and place.

### Phase 6: The owner's findings, and the losers removed
- **Owner skill:** dev
- **What:** on (b), set the owner's pick as the default and re-read Phase 4's test and
  goldens. On (c) for a class a candidate covers, sweep that candidate and re-read; a remaining
  fault outside the classes stops for an architect amendment. On (a), log "not needed". Then
  the losing candidates and `closing old` are removed with their flags and banner modes, as
  Plan 0048's rule says (owner, 2026-10-28), unless the owner keeps one for comparison.
- **Files touched:** as Phases 3 and 4, this plan's log.
- **Done when:** the log records the outcome, Phase 4's test passes at the default, and no
  losing candidate's flag remains in `--help` or the banner.

## Data shapes

```rust
// illustrative: vxe-render
pub enum CrackClosing { Old, Snap, Extend, Blend }
pub struct SurfaceTuning {
    pub fit: SurfaceFit,          // unchanged
    pub face_mix: f32,            // unchanged
    pub closing: CrackClosing,    // new; reaches march.wgsl through two spare Params flag bits
    pub shadow_push: bool,        // O
}
// `Params` gains no word: the closing rule and O take spare flag bits. The hit record, the
// attribute encoding and the pool layout do not change. The cracks view writes its class and
// self-hit through the layer texture, as the steps view writes its counters.
```

## Budgets

| `nfr.md` figure | Base (read) | Expected | How the close measures it |
|---|---|---|---|
| §2 primary rays, the hit pass (3.0 ms line) | level spawn 1080p contour: RTX 3080 Laptop 1.378 ms, Radeon (RADV) 9.908 ms (Plan 0039) | S: a few registers, **a guess** under 1 %. E: one more plane evaluation per passed voxel, under 2 %. B: up to 8 attribute reads per surface voxel stepped, **a guess** of +5 to +20 % on the Radeon, less on the RTX; this is the reading that decides B | `flythrough --level --seed 1 --headless --render 1920x1080 --frames 300` per rule and the pre-plan binary, alternated, release, exclusive perf lock, memory cap, three quiet runs, both adapters; bar in Phase 3's rule |
| §2 the attribute pass | Plan 0039's | 0: Phase 2 changes two constants of the code, not the pass | the level's initial fill equals the pre-plan reading to its noise |
| §3 mirror bytes | Plan 0028's | 0 | `mirror_bytes` and `allocated_bytes` equal the pre-plan binary's |

Determinism is not touched: everything is in `vxe-render`, a client-side view.

## Risks & open questions

- **The mechanism named here may not be the one.** Phase 1's first done-when is the check:
  a tilted slab must show classes 1 and 2 today. If it does not, dev stops and the architect
  re-reads the hit rule with the class map in hand.
- **S keeps a normal step at every crack.** It is the same faceting the surface shows
  everywhere else, and the owner's Phase 5 eye decides whether it reads as a line.
- **B's cost on the slow adapter.** Eight attribute reads are eight pool words through the
  shard addressing. The cost bar and the alternated reading are there for it; if B is the only
  rule that closes the cracks and misses the bar, the owner decides between the look and the
  cost, and the candidate stays selectable.
- **A watertight surface moves silhouettes a little,** so every contour golden moves. They are
  re-blessed by name under Plan 0047's 1 % rule, and the face goldens must not move at all.
- **Plan 0039's sharpness metric may fall.** `S_fine` counts any fine contrast, cracks
  included. It is read, not barred; the guardrails are the bar.
- **The cracks are not the only surface fault.** The LOD ring is cubic (ADR-0011), the offset
  formula is crude, and the gather's normal bias of 2 voxels (`PROBE_NORMAL_BIAS`) reads the
  probes off the surface. None of these is this plan's.

## What this plan does NOT do

- Change the attribute's size, pages, pass or fit beyond Phase 2's scale and what Phase 1 sweeps as
  readings. A least-squares plane fit or a per-corner density is a later plan with its own ADR.
- Give the LOD ring's mip cells a plane.
- Touch the lighting: the light spike proposed on 2026-10-29 follows this plan's Phase 5.
- Change the procedural detail or the face mix.

## Owner answers

1. **Order (owner, 2026-10-29).** This plan runs alone on main to its owner check (Phase 5
   since the amendment). The light spike the architect proposed the same day starts after
   that check. Plan 0049 stays parked
   at its Phase 2.
2. **Goldens (owner, 2026-10-29).** A watertight surface may move silhouettes, so the contour
   goldens that move are re-blessed by name with their changed-pixel share in the log. The
   face goldens must not move.
3. **The shadow origin (owner, 2026-10-29).** In scope as sub-candidate O. Phase 1 reads the
   self-hit share; Phase 3 keeps the push if it lowers self-hits without raising the speck
   reading.
4. **The losers (owner, 2026-10-29).** After the Phase 5 pick, the losing rules and
   `closing old` are removed with their flags and banner modes, as Plan 0048's rule says. Their
   readings stay in the log.

5. **The clean-surface control (owner, 2026-10-29).** The `(1, 1, 1)` slab asserts 0 in
   classes 1 and 2 in Phase 1, and the 45-degree slab is a reading; Phase 2 moves the control
   to the axis planes.
6. **The window's faces (owner, 2026-10-29).** Analytic bars count only hits at least 2
   voxels inside the mirror window; each reading prints the number left out.
7. **The code's own steps (owner, 2026-10-29).** Folded into this plan as Phase 2, with the
   scale-62 code of ADR-0038 over snapping on decode; a zero component must be exact, not
   patched by the closing rule.

## Implementation log

> Written by `dev`: one row per phase as its commit lands, then the close block. **The phases
> above are the contract. This section records what happened.** Observations, never conclusions:
> no pass list, no self-assessment. Deviations and unmet done-whens are **always** disclosed.
> Keep it shorter than `## Implementation phases`.

| phase | owner | state | commit |
|---|---|---|---|
| 1: The crack spike: classes, the view, and where they land | dev | done | 00cacd4 |
| 2: A zero normal component, exact in the plane code | dev | done | 4b137e7 |
| 3: The closing rules, each measured | dev | done | committed with this row |
| 4: The pick as the default | dev | not started | |
| 5: The owner's windowed check | human | not started | |
| 6: The owner's findings, and the losers removed | dev | not started | |

#### Phase 1

Readings: lavapipe (llvmpipe, Mesa 26.2.5, LLVM 23.1.3), exclusive perf lock, worktree at
00ca974 plus this phase's diff. Analytic scenes at 320 x 180 under the analytic window rule
("left out" is the hits it excluded); the level at 384 x 216, the pose's manual exposure,
indirect off, no upscaler. Start look = `LookPreset::DEFAULT`, detail on, headlamp, lamp on
unless named. `cargo nextest run -p flythrough --test look --run-ignored only --no-capture
crack_classes`; two runs read identical rows.

| scene | look | hits | left out | clean % | class 1 % | class 2 % | class 3 % | 1+2 px | dark % | bright % | self-hit % |
|---|---|---|---|---|---|---|---|---|---|---|---|
| tilted slab | start | 49364 | 192 | 94.53 | 0.000 | 3.806 | 1.663 | 1879 | 54.1 | 0.0 | 0.000 |
| concave corner | start | 57600 | 0 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| 45-degree slab | start | 48335 | 144 | 98.10 | 0.000 | 0.035 | 1.862 | 17 | 82.4 | 0.0 | 0.000 |
| diagonal slab | start | 57600 | 0 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| cavity | start | 57600 | 0 | 98.12 | 0.087 | 1.356 | 0.434 | 831 | 43.2 | 0.0 | 0.000 |
| pillar | start | 3476 | 0 | 96.61 | 0.460 | 2.934 | 0.000 | 118 | 63.6 | 0.0 | 0.000 |
| level vein | start | 82944 | 0 | 96.71 | 0.099 | 2.614 | 0.576 | 2250 | 32.7 | 0.5 | 0.110 |
| level vein, lamp off | start | 82944 | 0 | 96.71 | 0.099 | 2.614 | 0.576 | 2250 | 76.8 | 1.2 | 0.000 |
| level patch | start | 82944 | 0 | 91.95 | 0.047 | 5.744 | 2.256 | 4803 | 4.5 | 2.0 | 0.117 |
| level patch, lamp off | start | 82944 | 0 | 91.95 | 0.047 | 5.744 | 2.256 | 4803 | 21.4 | 3.2 | 0.000 |
| tilted slab | fit uniform27 | 49373 | 188 | 93.12 | 0.000 | 4.922 | 1.963 | 2430 | 54.3 | 0.0 | 0.000 |
| concave corner | fit uniform27 | 57600 | 0 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| 45-degree slab | fit uniform27 | 48335 | 145 | 98.10 | 0.000 | 0.035 | 1.862 | 17 | 82.4 | 0.0 | 0.000 |
| diagonal slab | fit uniform27 | 57600 | 0 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| cavity | fit uniform27 | 57600 | 0 | 97.86 | 0.073 | 1.559 | 0.503 | 940 | 40.0 | 0.0 | 0.000 |
| pillar | fit uniform27 | 3476 | 0 | 95.28 | 0.230 | 4.488 | 0.000 | 164 | 69.5 | 1.2 | 0.000 |
| level vein | fit uniform27 | 82944 | 0 | 96.21 | 0.058 | 3.007 | 0.728 | 2542 | 34.6 | 0.7 | 0.122 |
| level patch | fit uniform27 | 82944 | 0 | 90.55 | 0.055 | 6.821 | 2.570 | 5704 | 4.6 | 2.3 | 0.166 |
| tilted slab | offset volume | 49348 | 211 | 93.81 | 0.000 | 2.634 | 3.554 | 1300 | 55.9 | 0.0 | 0.000 |
| concave corner | offset volume | 57600 | 0 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| 45-degree slab | offset volume | 48314 | 142 | 94.88 | 0.000 | 0.048 | 5.075 | 23 | 91.3 | 0.0 | 0.000 |
| diagonal slab | offset volume | 57600 | 0 | 99.77 | 0.233 | 0.000 | 0.000 | 134 | 60.4 | 0.0 | 0.000 |
| cavity | offset volume | 57600 | 0 | 98.00 | 0.090 | 1.073 | 0.840 | 670 | 43.1 | 0.0 | 0.000 |
| pillar | offset volume | 3456 | 0 | 97.34 | 0.463 | 1.273 | 0.926 | 60 | 53.3 | 0.0 | 0.000 |
| level vein | offset volume | 82944 | 0 | 96.30 | 0.131 | 1.962 | 1.602 | 1736 | 31.3 | 1.0 | 0.082 |
| level patch | offset volume | 82944 | 0 | 91.64 | 0.054 | 4.348 | 3.958 | 3651 | 5.5 | 1.7 | 0.077 |
| tilted slab | clamp 0.5 (scratch) | 49359 | 194 | 93.63 | 0.012 | 3.158 | 3.199 | 1565 | 53.7 | 0.0 | 0.000 |
| concave corner | clamp 0.5 (scratch) | 57600 | 0 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| 45-degree slab | clamp 0.5 (scratch) | 48335 | 144 | 98.10 | 0.000 | 0.035 | 1.862 | 17 | 82.4 | 0.0 | 0.000 |
| diagonal slab | clamp 0.5 (scratch) | 57600 | 0 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| cavity | clamp 0.5 (scratch) | 57600 | 0 | 98.08 | 0.097 | 0.507 | 1.316 | 348 | 34.8 | 0.0 | 0.000 |
| pillar | clamp 0.5 (scratch) | 3476 | 0 | 96.03 | 0.460 | 1.554 | 1.956 | 70 | 64.3 | 0.0 | 0.000 |
| level vein | clamp 0.5 (scratch) | 82944 | 0 | 96.25 | 0.147 | 0.917 | 2.690 | 883 | 56.9 | 0.0 | 0.117 |
| level patch | clamp 0.5 (scratch) | 82944 | 0 | 91.50 | 0.115 | 3.213 | 5.170 | 2760 | 9.9 | 2.0 | 0.099 |

- The clamp sweep: `attrib.wgsl`'s `clamp(offset, -0.875, 0.875)` patched to `-0.5, 0.5`,
  read, reverted (no diff in `attrib.wgsl` at this commit).
- Done-when 1: `a_tilted_slab_has_cracks_today` reads 1879 class 1+2 pixels under the window
  rule; the cracks view's colours decode to the layers' shares over every hit.
- Done-when 2: `a_diagonal_slab_has_none` reads 0 in classes 1 and 2, 0 left out.
- The concave corner (`corner_world`, `corner_camera` as specified) reads 0 in every class.
- Side-by-sides (untracked): `live/cracks/{vein,patch}-{start-look,start-look-lamp-off,fit-uniform27,offset-volume}.png`.
- Vein: the class 2 (yellow) regions are wedges on the bottoms and ends of the ledges, where
  the owner's `wedges` and `step3` screenshots have the dark wedges; class 1 is a few red
  specks on the same ledges. At 384 x 216 the shaded panel does not resolve the wedges by eye.
- Patch: the class 2 regions are small triangles in a lattice across the floor, at the
  vertices where the facets meet, where `step5` has the bright triangles; the upper wall has
  larger class 2 saw-tooth slivers along its ledge edges.
- Attribution (done-when 6): class 2 owns the wedges (76.8 % of the vein's class 1+2 pixels
  dark with the lamp off), not class 1 (0.099 % of hits at the vein, 0.047 % at the patch).
  Class 2 owns the triangles and the slivers (3.2 % of the patch's class 1+2 pixels bright with
  the lamp off). The self-hit is 91 pixels at the vein and 97 at the patch with the lamp on
  (0.110 %, 0.117 %), magenta dots inside the cone along ledge edges, against 2168 and 4764
  class 2 pixels at the same poses: some dots inside the cone are self-hits, and the slivers
  along those edges are class 2.
- Deviations: no view-cycle key existed, `F9` added; the self-hit is computed in the hit pass
  (`march.wgsl`, `lamp_self_hit`, per full pixel), not per lighting pixel in `light.wgsl`, and
  `light.wgsl`'s shadow origins go through `shadow_origin()` (`gradient.wgsl` keeps its own);
  the cracks view drops translucent layers, as the steps view does; files outside the list:
  `crates/vxe-render/src/lib.rs` (the `CrackShares` export), `tests/support/mod.rs`
  (`reading_device`, copied from branch `plan-0049`), `apps/flythrough/src/lib.rs` and the
  pose parts of `main.rs` and `hud.rs` (`Pose`, `VEIN_POSE`, `PATCH_POSE`, `POSES`, `--pose`,
  copied from branch `plan-0049`); the level is read with indirect off, the pose's manual
  exposure and no upscaler; the poses are also read with the lamp off (start look); the dark
  and bright shares are over the counted pixels.
- Goldens: every golden re-rendered with `VXE_BLESS=1` (castle_golden, sharpness, golden,
  look, fsr2 under `--profile gpu`) at 00ca974 and at this commit reads identical pixels, 29 of
  29. Both differ from the committed PNGs by at most 2/255 in 8 of them (castle_gate,
  dark_cave, dark_cave_half, look_wall_contour, look_wall_contour_grain,
  look_wall_contour_sharp, look_wall_face, upscaled), 0 pixels over 2/255: the same at the
  base, so the committed goldens were not touched.
- Gate at this commit: build, clippy `-D warnings`, fmt, doctests, comment-hygiene and
  test-tiers scripts clean; `--profile quick` 517 of 518 passed, 1 timeout
  (`flythrough::replay a_stamp_and_a_collapse_replay_from_the_save` at 30 s, load average 31
  from another session; alone it passed in 14.6 s); `--profile gpu` 178 passed, 31 skipped.

#### Phase 2

Readings as Phase 1's (lavapipe, exclusive perf lock), at 00cacd4 plus this phase's diff.

| scene | look | hits | left out | clean % | class 1 % | class 2 % | class 3 % | 1+2 px | dark % | bright % | self-hit % |
|---|---|---|---|---|---|---|---|---|---|---|---|
| tilted slab | start | 49366 | 191 | 94.11 | 0.000 | 4.070 | 1.825 | 2009 | 54.3 | 0.0 | 0.000 |
| concave corner | start | 57600 | 0 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| 45-degree slab | start | 48333 | 142 | 97.90 | 0.000 | 0.000 | 2.102 | 0 | 0.0 | 0.0 | 0.000 |
| floor | start | 43012 | 392 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| wall | start | 37800 | 180 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| diagonal slab | start | 57600 | 0 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| cavity | start | 57600 | 0 | 98.18 | 0.101 | 1.260 | 0.458 | 784 | 45.3 | 0.0 | 0.000 |
| pillar | start | 3476 | 0 | 96.43 | 0.460 | 3.107 | 0.000 | 124 | 61.3 | 0.0 | 0.000 |
| level vein | start | 82944 | 0 | 96.71 | 0.113 | 2.596 | 0.581 | 2247 | 32.0 | 0.4 | 0.077 |
| level vein, lamp off | start | 82944 | 0 | 96.71 | 0.113 | 2.596 | 0.581 | 2247 | 77.6 | 1.1 | 0.000 |
| level patch | start | 82944 | 0 | 91.33 | 0.034 | 6.315 | 2.318 | 5266 | 2.1 | 2.2 | 0.136 |
| level patch, lamp off | start | 82944 | 0 | 91.33 | 0.034 | 6.315 | 2.318 | 5266 | 20.2 | 3.2 | 0.000 |
| tilted slab | fit uniform27 | 49370 | 188 | 93.21 | 0.000 | 4.642 | 2.151 | 2292 | 53.8 | 0.0 | 0.000 |
| concave corner | fit uniform27 | 57600 | 0 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| 45-degree slab | fit uniform27 | 48333 | 143 | 97.90 | 0.000 | 0.000 | 2.102 | 0 | 0.0 | 0.0 | 0.000 |
| floor | fit uniform27 | 43012 | 392 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| wall | fit uniform27 | 37800 | 180 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| diagonal slab | fit uniform27 | 57600 | 0 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| cavity | fit uniform27 | 57600 | 0 | 97.84 | 0.115 | 1.576 | 0.469 | 974 | 40.8 | 0.0 | 0.000 |
| pillar | fit uniform27 | 3472 | 0 | 95.05 | 0.461 | 4.493 | 0.000 | 172 | 66.3 | 0.0 | 0.000 |
| level vein | fit uniform27 | 82944 | 0 | 96.28 | 0.113 | 2.920 | 0.682 | 2516 | 35.1 | 0.5 | 0.109 |
| level patch | fit uniform27 | 82944 | 0 | 90.48 | 0.043 | 6.834 | 2.644 | 5704 | 4.0 | 2.2 | 0.197 |
| tilted slab | offset volume | 49351 | 207 | 93.43 | 0.000 | 2.616 | 3.955 | 1291 | 56.2 | 0.0 | 0.000 |
| concave corner | offset volume | 57600 | 0 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| 45-degree slab | offset volume | 48315 | 141 | 94.49 | 0.000 | 0.000 | 5.512 | 0 | 0.0 | 0.0 | 0.000 |
| floor | offset volume | 43012 | 392 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| wall | offset volume | 37800 | 180 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| diagonal slab | offset volume | 57600 | 0 | 99.67 | 0.330 | 0.000 | 0.000 | 190 | 58.9 | 0.0 | 0.000 |
| cavity | offset volume | 57600 | 0 | 98.04 | 0.122 | 0.997 | 0.844 | 644 | 46.4 | 0.0 | 0.000 |
| pillar | offset volume | 3456 | 0 | 97.11 | 0.579 | 1.389 | 0.926 | 68 | 50.0 | 4.4 | 0.000 |
| level vein | offset volume | 82944 | 0 | 96.33 | 0.152 | 1.919 | 1.594 | 1718 | 29.7 | 0.5 | 0.053 |
| level patch | offset volume | 82944 | 0 | 91.00 | 0.046 | 4.802 | 4.151 | 4021 | 2.6 | 1.8 | 0.102 |

- Done-when 1: `axis_planes_have_no_cracks` reads 0 in classes 1 and 2 for the floor (392
  left out) and the wall (180 left out). The same test with the old code (the two shader
  constants reverted as a scratch, then restored) also reads 0 for both, every class.
- Done-when 2: `axis_planes_decode_to_the_exact_axis_normal` (mirror.rs), under all four
  fits: floor voxel (10, 10, 2) attribute `0xb7df`, codes (31, 31), normal `(0, 0, 1)`; wall
  voxel (63, 100, 20) `0xb7c0`, codes (0, 31), normal `(-1, 0, 0)`, bits equal. By the old
  formula (`round(p / 2 + 1/2) x 63`, round-half-even) they were codes (32, 32) and (0, 32),
  `0xb820` and `0xb800`. The offset codes the mirror tests pin (floor 11, edge 6) read as
  before. Two mirror tests pin hashes of attribute words and move: `FIVE_BRICK_HASH`
  `0xd795_d691_eff6_7ad3` to `0x2097_bf15_d528_0e26`
  (`a_five_brick_list_matches_an_uncapped_mirror_and_the_one_brick_pass`, whose list still
  equals the uncapped mirror's words) and `BESIDE_LOD_HASH` `0xda7d_5f1f_ace6_9921` to
  `0xb326_6753_424e_d0ec` (`attributes_beside_lod_only_chunks_read_their_mips`, 40 face bricks
  changed by the LOD neighbour).
- Done-when 3, guardrails (lavapipe), 00ca974 then this commit:
  `the_default_look_meets_the_guardrails` cavity normal error 7.02 then 7.19 degrees,
  silhouette 32 then 32 px against face 172, `A` 6.81 then 6.83 %;
  `contour_planes_cut_the_silhouette_and_normal_errors` (contour mode) at this commit 7.46
  degrees, 36 px, 6.84 %; speck test contour only-with-shadows 4 then 6 (with 401 then 379,
  without 399 then 374), face 34 then 34.
- Done-when 4, moved goldens, share of pixels over 2/255 against the committed PNG:
  `look_contour.png` 4.012 %, `look_wall_contour.png` 3.304 %,
  `look_wall_contour_grain.png` 6.200 %, `look_wall_contour_sharp.png` 3.373 %. Every other
  golden re-rendered at this commit reads the same pixels as Phase 1's re-render, and is not
  re-blessed; `look_wall_face.png` and the other face goldens unchanged.
- Deviation: `apps/flythrough/tests/sharpness.rs` (not in the list):
  `detail_after_lighting_is_exact_for_the_lamp` read 98.389 % within 2/255 at this phase
  against its floor 98.4 % (98.465 % at 00ca974; misses unlit 0.896 then 0.931 %, darker
  0.639 then 0.681 %). The owner set the floor to 98.3 % (2026-10-29); the doc comment states
  the reading.
- Deviation: `crack_classes` gains the floor and wall rows; `mirror.rs` gains `raw_attr_at`.
- Gate at this commit: build, clippy `-D warnings`, fmt, doctests, comment-hygiene and
  test-tiers clean; `--profile quick --test-threads 8` 518 passed (at the default thread count
  two CPU tests timed out at 30 s with the load average at 27, and passed alone in 14.8 s);
  `--profile gpu` 177 of 179 passed, the two hash pins above failing, then `mirror` alone 45
  of 45 at the new hashes.

#### Phase 3

Class readings as Phase 1's (lavapipe, exclusive perf lock, `closing_candidates`), at 4b137e7
plus this phase's diff; the `old` rows equal Phase 2's table above row for row and are not
repeated. `+ push` is the rule with the shadow push (O), read at the poses only.

| scene | rule | hits | left out | clean % | class 1 % | class 2 % | class 3 % | 1+2 px | dark % | bright % | self-hit % |
|---|---|---|---|---|---|---|---|---|---|---|---|
| level vein | old + push | 82944 | 0 | 96.71 | 0.113 | 2.596 | 0.581 | 2247 | 32.0 | 0.4 | 0.000 |
| level patch | old + push | 82944 | 0 | 91.33 | 0.034 | 6.315 | 2.318 | 5266 | 2.1 | 2.2 | 0.041 |
| tilted slab | snap | 49366 | 191 | 98.17 | 0.000 | 0.000 | 1.825 | 0 | 0.0 | 0.0 | 0.000 |
| concave corner | snap | 57600 | 0 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| 45-degree slab | snap | 48333 | 142 | 97.90 | 0.000 | 0.000 | 2.102 | 0 | 0.0 | 0.0 | 0.000 |
| floor | snap | 43012 | 392 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| wall | snap | 37800 | 180 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| diagonal slab | snap | 57600 | 0 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| cavity | snap | 57600 | 0 | 99.54 | 0.000 | 0.000 | 0.458 | 0 | 0.0 | 0.0 | 0.000 |
| pillar | snap | 3476 | 0 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| level vein | snap | 82944 | 0 | 99.42 | 0.000 | 0.000 | 0.581 | 0 | 0.0 | 0.0 | 0.077 |
| level vein lamp off | snap | 82944 | 0 | 99.42 | 0.000 | 0.000 | 0.581 | 0 | 0.0 | 0.0 | 0.000 |
| level patch | snap | 82944 | 0 | 97.68 | 0.000 | 0.000 | 2.318 | 0 | 0.0 | 0.0 | 0.136 |
| level patch lamp off | snap | 82944 | 0 | 97.68 | 0.000 | 0.000 | 2.318 | 0 | 0.0 | 0.0 | 0.000 |
| level vein | snap + push | 82944 | 0 | 99.42 | 0.000 | 0.000 | 0.581 | 0 | 0.0 | 0.0 | 0.000 |
| level patch | snap + push | 82944 | 0 | 97.68 | 0.000 | 0.000 | 2.318 | 0 | 0.0 | 0.0 | 0.041 |
| tilted slab | extend 0.25 | 49399 | 186 | 98.85 | 0.000 | 0.119 | 1.030 | 59 | 84.7 | 0.0 | 0.000 |
| concave corner | extend 0.25 | 57600 | 0 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| 45-degree slab | extend 0.25 | 48346 | 172 | 99.81 | 0.000 | 0.000 | 0.186 | 0 | 0.0 | 0.0 | 0.000 |
| floor | extend 0.25 | 43012 | 392 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| wall | extend 0.25 | 37800 | 180 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| diagonal slab | extend 0.25 | 57600 | 0 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| cavity | extend 0.25 | 57600 | 0 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| pillar | extend 0.25 | 3480 | 0 | 97.59 | 0.000 | 2.414 | 0.000 | 84 | 71.4 | 0.0 | 0.000 |
| level vein | extend 0.25 | 82944 | 0 | 99.86 | 0.000 | 0.119 | 0.018 | 99 | 92.9 | 0.0 | 0.077 |
| level vein lamp off | extend 0.25 | 82944 | 0 | 99.86 | 0.000 | 0.119 | 0.018 | 99 | 100.0 | 0.0 | 0.000 |
| level patch | extend 0.25 | 82944 | 0 | 97.20 | 0.000 | 1.262 | 1.536 | 1047 | 2.2 | 4.1 | 0.127 |
| level patch lamp off | extend 0.25 | 82944 | 0 | 97.20 | 0.000 | 1.262 | 1.536 | 1047 | 13.7 | 4.6 | 0.000 |
| level vein | extend 0.25 + push | 82944 | 0 | 99.86 | 0.000 | 0.119 | 0.018 | 99 | 92.9 | 0.0 | 0.000 |
| level patch | extend 0.25 + push | 82944 | 0 | 97.20 | 0.000 | 1.262 | 1.536 | 1047 | 2.2 | 4.0 | 0.041 |
| tilted slab | extend 0.5 | 49417 | 192 | 99.74 | 0.000 | 0.000 | 0.257 | 0 | 0.0 | 0.0 | 0.000 |
| concave corner | extend 0.5 | 57600 | 0 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| 45-degree slab | extend 0.5 | 48351 | 170 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| floor | extend 0.5 | 43012 | 392 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| wall | extend 0.5 | 37800 | 180 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| diagonal slab | extend 0.5 | 57600 | 0 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| cavity | extend 0.5 | 57600 | 0 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| pillar | extend 0.5 | 3484 | 0 | 98.62 | 0.000 | 1.378 | 0.000 | 48 | 75.0 | 0.0 | 0.000 |
| level vein | extend 0.5 | 82944 | 0 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.077 |
| level vein lamp off | extend 0.5 | 82944 | 0 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| level patch | extend 0.5 | 82944 | 0 | 99.00 | 0.000 | 0.164 | 0.838 | 136 | 6.6 | 5.9 | 0.124 |
| level patch lamp off | extend 0.5 | 82944 | 0 | 99.00 | 0.000 | 0.164 | 0.838 | 136 | 6.6 | 5.1 | 0.000 |
| level vein | extend 0.5 + push | 82944 | 0 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| level patch | extend 0.5 + push | 82944 | 0 | 99.00 | 0.000 | 0.164 | 0.838 | 136 | 6.6 | 5.9 | 0.039 |
| tilted slab | blend | 49254 | 193 | 82.96 | 0.006 | 0.000 | 17.036 | 3 | 66.7 | 0.0 | 0.000 |
| concave corner | blend | 57600 | 0 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| 45-degree slab | blend | 48235 | 159 | 85.27 | 0.000 | 0.000 | 14.730 | 0 | 0.0 | 0.0 | 0.000 |
| floor | blend | 43012 | 392 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| wall | blend | 37800 | 180 | 100.00 | 0.000 | 0.000 | 0.000 | 0 | 0.0 | 0.0 | 0.000 |
| diagonal slab | blend | 57600 | 0 | 94.47 | 0.101 | 0.000 | 5.434 | 58 | 53.4 | 0.0 | 0.000 |
| cavity | blend | 57600 | 0 | 93.51 | 0.036 | 0.000 | 6.453 | 21 | 14.3 | 0.0 | 0.000 |
| pillar | blend | 3390 | 0 | 93.69 | 0.118 | 0.000 | 6.195 | 4 | 25.0 | 75.0 | 0.000 |
| level vein | blend | 82944 | 0 | 88.70 | 0.000 | 0.000 | 11.300 | 0 | 0.0 | 0.0 | 0.000 |
| level vein lamp off | blend | 82944 | 0 | 88.70 | 0.000 | 0.000 | 11.300 | 0 | 0.0 | 0.0 | 0.000 |
| level patch | blend | 82944 | 0 | 79.74 | 0.000 | 0.000 | 20.259 | 0 | 0.0 | 0.0 | 0.000 |
| level patch lamp off | blend | 82944 | 0 | 79.74 | 0.000 | 0.000 | 20.259 | 0 | 0.0 | 0.0 | 0.000 |
| level vein | blend + push | 82944 | 0 | 88.70 | 0.000 | 0.000 | 11.300 | 0 | 0.0 | 0.0 | 0.000 |
| level patch | blend + push | 82944 | 0 | 79.74 | 0.000 | 0.000 | 20.259 | 0 | 0.0 | 0.0 | 0.000 |

Guardrails (cavity normal error, pillar silhouette against face 172 px, `A`) and the speck test
(contour golden scene, full shadow walk: with shadows, without, only with), lavapipe:

| rule | normal error | silhouette px | `A` % | specks | specks + push |
|---|---|---|---|---|---|
| old | 7.19 | 32 | 6.83 | 379, 374, 6 | 379, 374, 6 |
| snap | 7.13 | 32 | 6.73 | 365, 360, 6 | 365, 360, 6 |
| extend 0.25 | 7.13 | 28 | 6.73 | 361, 355, 7 | 361, 355, 7 |
| extend 0.5 | 7.13 | 32 | 6.73 | 345, 338, 7 | 344, 338, 6 |
| blend | 5.81 | 62 | 6.51 | 210, 206, 5 | 210, 206, 5 |

Cost, the hit and light passes at the level's spawn (`flythrough --level --seed 1 --headless
--render 1920x1080 --frames 300 --adapter NVIDIA|AMD [--closing RULE] [--shadow-push on]`,
release, each run under the exclusive perf lock and the memory cap, the median over three
alternated rounds of each run's median of the last 30 frames; 1080p from 1280 x 720 internal;
1-minute load 0.5 to 3.5). NVIDIA GeForce RTX 3080 Laptop GPU and AMD Radeon Graphics (RADV
RENOIR), `--adapter`. "pre" is the binary of 00ca974; "tree" is this commit's binary with
every rule selectable; "snap alone" is this commit's tree with the blend and extend branches
deleted (a scratch build, `live/strip-losers.py`):

| binary, rule | RTX hit ms | RTX light ms | Radeon hit ms | Radeon light ms |
|---|---|---|---|---|
| pre | 0.598 | 0.904 | 4.264 | 7.012 |
| tree, old | 0.662 | 1.536 | 5.978 | 8.911 |
| tree, snap | 0.658 | 1.529 | 5.981 | 8.893 |
| snap alone, old | 0.600 | 0.906 | 4.353 | 7.039 |
| snap alone, snap | 0.604 | 0.913 | 4.364 | 7.026 |
| snap alone, snap + push | 0.586 | 1.439 | 4.360 | 9.165 |
| snap alone, snap + push on the lamp ray only (scratch, two rounds) | 0.598, 0.604 | 1.091, 1.101 | 4.359, 4.365 | 7.907, 7.915 |

An earlier round on the tree before the register changes below (same command, same session
pattern): RTX hit pre 0.600, old 0.662, snap 0.671, extend 0.25 0.659, extend 0.5 0.661,
blend 3.302 (light pre 0.904, blend 3.096); Radeon hit pre 4.261, old 5.972, snap 5.974,
extend 0.25 5.928, extend 0.5 5.926, blend 35.151 (light pre 7.005, blend 13.439). Mirror
157 872 640 B live, 259 203 264 B allocated in every run.

- Attribution of the cost, Radeon hit pass (three alternated rounds): pre 4.264, Phase 1's
  binary 4.717, Phase 2's 4.715. With Phase 1's cracks-view self-hit replaced by `false`
  (scratch) 4.285; with the cracks branch removed 4.266. RADV's shader stats
  (`RADV_DEBUG=shaderstats,nocache`) give the hit shader 64 VGPRs before the plan, 84 at Phase
  1 and with snap's `vec3` memory, 128 in the tree with blend, and 64 for snap alone after the
  two changes below; the light shader 84 before the plan and for snap alone, 128 in the tree.
- Changes from the cost reading: the cracks view's classification and self-hit compile only
  into a second hit pipeline (`override CRACKS_VIEW`, `Marcher::cracks`), and the walk's pass
  memory is one `u32` (the passed voxel's attribute) instead of a normal and a count.
- Kept: snap (S), the first in order that meets every bar: classes 1 and 2 at 0 on the tilted
  slab, the concave corner and the cavity, 0 at both poses (Phase 2: 2247 and 5266), the
  guardrails, specks 6 (Phase 2's 6, face 34), and the hit pass alone +1.0 % RTX and +2.3 %
  Radeon against pre. The bar was read on "snap alone": the tree with every rule selectable
  reads +10.0 % RTX and +40.3 % Radeon, from blend's registers.
- O: the self-hit share falls under every rule (vein 0.077 to 0.000 %, patch 0.136 to 0.041
  % under snap) and the speck reading does not rise, which rule 3 keeps. The owner kept snap
  as the default with the push off and selectable (2026-10-29), for its light pass cost.
- Owner (2026-10-29): the tree's cost with every rule selectable is accepted until Phase 6
  removes the losers; `nfr.md` gets both readings.
- Done-when 3: every golden re-rendered with `VXE_BLESS=1` at this commit (closing old, the
  default) reads the same pixels as Phase 2's re-render, 29 of 29.
- `castle_gate_golden`'s peak RSS: 952 848 384 B at this commit alone, 806 113 280 B at
  4b137e7 alone, 1 494 994 944 B (over its 1 GiB bar) once in the bless run at load 2 to 3.
- Deviations: `CrackClosing` has five values (`extend 0.25` and `extend 0.5` apart, flag bits
  14 to 16) and the push is bit 17; files outside the list: `crates/vxe-render/src/lib.rs`
  (the export), `apps/flythrough/src/hud.rs` (the banner names the rule, `BANNER_ROWS` 8),
  `apps/flythrough/tests/sharpness.rs` (`..SurfaceTuning::default()`); the blend rule tests a
  solid voxel without a plane in a Full brick too, and hits at its second secant point; O
  starts at 0.5 and tries 0.75 before falling back to 1.0, over the eight voxels around the
  origin.
- Owed: the closing rule's ADR (the architect's, before Phase 4 lands the default).
- The class table, guardrails and specks were read twice on lavapipe, before and after the
  register changes, and match line for line.
- Gate at this commit: build, clippy `-D warnings`, fmt, doctests, comment-hygiene and
  test-tiers clean; `--profile quick --test-threads 8` 518 passed; `--profile gpu` 179 passed,
  32 skipped.

### Notes

### Close triggers

- **What shipped:**
- **Public API changed:**
- **Budget readings:**
- **Gate at the tip:**
- **Outstanding `human` phases:**

## Followups
