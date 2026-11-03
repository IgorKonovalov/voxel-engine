# 0053: One-voxel features on the contour surface: a box metric, a majority-filtered plane fit, a normal field, and the owner's call

> **Status:** in-progress
> **Approved:** 2026-10-30 (owner, as drafted; the order against Plan 0051 and the two cost bars stay the owner's to set)
> **Amended:** 2026-10-30 (architect, owner's answer): Phase 4 waits for
> [Plan 0054](0054-field-surface-trilinear-zero-set-of-the-27-fraction.md)'s Phase 1, the
> field-surface spike stacked on this plan's branch after Phase 3, and compares its
> `--look field` as a sixth look in the same windowed session. Outcome (b) names the field
> surface by its reading, not by a future plan.
> **Created:** 2026-10-30
> **Milestone:** M2 in [`docs/roadmap.md`](../roadmap.md) (the surface-look decision: "smooth
> and natural rock, not visible cubes")
> **Related ADRs:** [ADR-0011](../adrs/0011-contour-planes-in-attribute-pages-with-procedural-detail.md)
> (the per-voxel plane, whose one-clipped-cube shape this plan measures the limits of);
> [ADR-0039](../adrs/0039-the-contour-walk-closes-a-crack-at-the-passed-voxels-exit.md)
> (snap, the closing rule this plan builds on);
> [ADR-0040](../adrs/0040-one-voxel-features-melt-into-the-contour-surface-by-a-majority-fit-and-a-normal-field.md)
> (proposed: the majority fit and the normal field; Phase 4 accepts or supersedes it).
> **Source:** the owner at Plan 0050's Phase 5 (2026-10-30, screenshots in that worktree's
> untracked `live/p5/`): voxel-sized flat-faced boxes at the vein and the patch, in `old` and
> in snap alike, reading class 0 in the cracks view. "We are not able to proceed with this bug
> forward. Either we fix it, or if it is unfixable, we will not use contours."
> **Starts after:** Plan 0050 merges (its Phase 6 deletes the losing rules this plan would
> otherwise carry, and its pose rigs, cracks records and snap default are this plan's base).
> Alone on main. Its order against Plan 0051 is the owner's call; the architect's proposal is
> before it, since Phase 4 decides whether the contour look stays, which Plan 0051's look
> readings assume.

## TL;DR

The boxes are not cracks: ADR-0011's surface is one cube per solid voxel clipped by one
plane, and the plane of a one-voxel feature is fit from a neighbourhood its base surface
dominates, so a rod keeps a flat top and cube-face sides, and the normals around it tilt in
three hard-edged tiles. This plan first measures that (a box metric and a tile-contrast
metric on six one-voxel scenes and at the two poses, with the owner's neighbour-clipping
hypothesis read as a scratch), then builds two changes behind settings: the plane fit on a
majority-filtered occupancy, which melts one-voxel features into the surface under them with
no walk or storage change, and a shading normal that is a trilinear field over the planes,
which removes the tile edges everywhere. The owner compares today's look, each change and
both at the two poses, beside a cubes-shaded-by-the-field preset that is the fallback if
contours go. The pick lands as the default and ADR-0040 is accepted; if the eye still refuses,
the next plan is the field surface or the fallback, the owner's call. The first thing anyone
sees is a test that prints how many pixels at each pose land on an exposed cube face of a
voxel with two or more air faces, and a rod on a floor that reads hundreds of them.

## Context & problem

`project-context.md` (Rendering, P0): first person, stable at arm's length, smooth natural
rock. A 10 cm box with three flat tones on a smooth wall reads as a rendering fault and
survives every lighting fix. The owner will not ship the contour look with it.

**What the tree does (ADR-0011, `attrib.wgsl` `voxel_attr`, `march.wgsl` `contour_test`;
read from the code and the formula, measured only as Phase 1 says):**

- The attribute pass fits each surface voxel a plane: the normal from the 27-voxel occupancy
  gradient (weighted, Plan 0039), the offset `(count / 27 - 0.5) * 3` voxels along it,
  clamped to `[-0.875, 0.875]`. The surface mask (the voxels that get an attribute) is the
  CPU's: solid voxels with an air face neighbour.
- The walk clips the voxel's cube by its plane. A ray entering the cube already inside the
  half-space hits the cube face (kind 1); one crossing the plane inside hits the plane
  (kind 3); one staying outside passes (kind 2), and snap closes the crack at the passed
  voxel's exit.
- **A rod on a flat floor:** the rod voxel reads count 12, offset -0.167, normal straight up:
  a slab a third of a voxel high with cube-face sides. The floor voxels beside it read count
  21, offset 0.833 (beyond the cube) and normals tilted about 20 degrees toward the rod.
  Three flat tiles, three tones. **A one-voxel groove:** its bottom voxel reads count 24
  (a flat cube face), its side voxels count 15 with normals tilted 25 degrees into it,
  opposite ways. **A lone cube on a flat floor** reads count 10: a plate a ninth of a voxel
  high; the owner's lone cubes stand where the count leaves the whole cube. **A free pillar
  or a sheet** has a vanishing gradient and no plane: full cubes.
- Plan 0050's Phase 5 scratch (not committed) found 2 of 82 944 hits at the vein and 0 at
  the patch on a voxel without a plane: the boxes are planed voxels hit on their cube faces.

**Why no closing rule reaches them:** the cube faces are hit directly from air, with no pass
before them. The clipped cube removes material from the feature voxel's own cube; a smooth
rod flares onto the floor beside it and a smooth groove is a fillet, and both lie in the
neighbouring air cells.

**The `nfr.md` figures touched:** §2 the hit pass (snap alone: RTX 3080 Laptop 0.604 ms,
Radeon RENOIR 4.364 ms at the level's spawn, 1080p from 720p) and the light pass (0.913 and
7.026 ms), the attribute pass (the level's fill 17.0 ms under the weighted fit, Plan 0039;
the flight's p50 and p99 of Plan 0013); §3 mirror bytes (157 872 640 B live at the level),
which no phase here moves.

## Decision

Measure, then two cheap changes, then the owner's call. Phase 1 defines the box and
tile-contrast metrics, reads them on six one-voxel scenes and at the poses, and reads the
owner's neighbour-clipping hypothesis and the existing offset knobs as scratches, so the
mechanism is named by a number before anything is built. Phase 2 fits the plane on a
majority-filtered occupancy inside the attribute pass (ADR-0040): one-voxel features melt into
the surface under them, with the mask, the attribute, the walk and the storage unchanged.
Phase 3 shades with a normal field, the trilinear blend of the eight surrounding planes'
normals at the hit point, and builds the cubes-shaded-by-the-field preset. Phase 4 is the
owner's windowed check of five looks at both poses. Phase 5 lands the pick, removes the
losers and re-blesses by name.

We rejected neighbour clipping because it only removes material from the feature's cube
(worked in ADR-0040, read in Phase 1). We deferred the field surface (the trilinear zero set
of the 27-fraction, watertight and corner-filleting) because it needs attributes on air voxels
and a walk cost no one has measured; it is the next plan if Phase 4 refuses. We deferred
dilation for the same storage. We rejected a wider per-voxel normal fit because it keeps one
constant per voxel. Dropping contours for cubes shaded by the field is built as the fallback,
not chosen.

**The analytic window rule** of Plan 0050 holds: analytic bars count hits at least 2 voxels
inside the mirror window and print what they left out.

## Architecture diagram

```mermaid
flowchart LR
    subgraph vxe-render
        O[attrib.wgsl: occupancy, brick + 2-voxel shell] --> F[majority filter: 14 of 27]
        F --> P[plane fit per surface voxel, same u16, same pages]
        P --> W[march.wgsl: clipped cube, snap]
        W --> H[hit record: position, plane normal]
        H --> L[light.wgsl: normal field from the 8 planes around the hit]
        H --> S[shadow origin: plane normal]
    end
    subgraph flythrough tests
        H --> M[box pixels, tile contrast, guardrails, cost on both adapters]
    end
```

## Implementation phases

Each phase ships as its own commit. `dev` implements all phases in one session with no review
between phases. The architect reviews once at the end, in a fresh session. Lane rule 9c
applies: exploratory sweeps may run on the RTX 3080 Laptop through `VXE_TEST_ADAPTER`,
labelled with the adapter; every asserted reading, golden and gate stays on lavapipe.

### Phase 1: The box spike: two metrics, six scenes, the poses, and the hypotheses read
- **Owner skill:** dev
- **What:** two metrics over the hit records and the CPU world, six one-voxel scenes on a
  flat floor, the readings at the poses, and the hypotheses as scratches. No default, hit or
  shader change.
- **The box metric** (`tests/look.rs`, from the hit positions, the hit records and the CPU
  world): a *box pixel* is a hit pixel whose hit point lies on a cube face (a coordinate
  within 1e-3 of an integer, as Plan 0050's scratch read it) of a solid voxel with two or more
  air face neighbours in the world. A flat floor's face hits have one air face and do not
  count; a convex edge voxel whose exposed faces its plane cuts away has no face hit and does
  not count. Amended 2026-10-31 (architect, from Phase 2's finding): a hit that also lies on
  one of the box voxel's faces toward a solid neighbour does not count either; without this the
  filtered rod read 8 pixels at z = 60.000, the floor plane's outline around the rod's footprint.
  Phase 1's readings are logged under both forms. Each box pixel is also sorted by the voxel's 27-count: under 14 (a protrusion the
  majority filter melts) or 14 and over (a groove, a corner, a two-voxel feature: what the
  filter leaves), and by the voxel's air faces (2, 3, 4 or more).
- **The tile-contrast metric:** over horizontally and vertically adjacent hit-pixel pairs
  whose hit points lie within 1.5 voxels of each other (one surface, no depth edge), the share
  whose shading normals differ by more than 15 degrees, read in the normals view with detail
  off. It reads the hard tile edges; a smooth curved wall's neighbours differ by a few
  degrees and do not count.
- **The scenes** (`tests/support/rigs.rs`, each `floor_world` with a feature on its top
  layer at `z = 64`, seen from about 1 m at the floor camera's pitch, or `wall_world` with
  the feature on its face): `rod` (one voxel high and wide, 12 long along x, on the floor);
  `lone cube`; `groove` (one wide, one deep, 12 long, cut into the floor's top layer); `wall
  pillar` (a one-voxel column 6 high standing against the wall, the owner's pillars); `ridge`
  (two wide, one high, 12 long: the two-voxel feature the filter keeps, a reading of what
  remains); `free pillar` (a one-voxel column 6 high on the floor, touching nothing) and
  `sheet` (one thick, 8 high, 12 long, on the floor): the two hazards ADR-0040 names, whose
  upper rows keep an empty filtered neighbourhood and stay cubes. The bare floor is the
  control.
- **The readings:** box pixels by count class and air faces, the tile-contrast share, and
  the crack classes, per scene and at `VEIN_POSE` and `PATCH_POSE` (384 x 216, the pose's
  exposure, indirect off, lamp on and off), in the start look (snap). A side-by-side per pose
  under `live/boxes/`: the shaded image, the normals view, and the shaded image with box
  pixels painted red, so the owner can see the metric lands on the boxes they photographed.
- **The hypotheses, as scratches** (readings only, reverted, as Plan 0050's clamp sweep):
  - **neighbour clipping** (ADR-0040 Alternative A): a kind 1 hit on an exposed cube face
    also requires the entry point inside the solid half-spaces of the planes of the voxel's
    face neighbours that carry one; else the walk advances along the segment to the first
    point inside all of them, or passes. The prediction to confirm or refute: the rod's side
    face shrinks (box pixels fall by about a third on `rod`), the groove's do not move;
  - `PlaneOffset::VolumeMatched` and the offset clamp at 0.5 (the knobs Plan 0050 swept), on
    the six scenes: the prediction is no change on `rod` (both give -0.167 for an axis
    normal).
- **Files touched:** `apps/flythrough/tests/look.rs` (the metrics, the table, the
  side-by-sides), `apps/flythrough/tests/support/rigs.rs` (the six scenes), this plan's log.
- **Done when:**
  1. `one_voxel_features_render_as_boxes_today` asserts on lavapipe, under the analytic window
     rule, that `rod`, `lone cube`, `groove` and `wall pillar` each read at least 1 box pixel
     in the start look and the bare floor reads 0 (the readings are printed). If `rod` reads
     0, dev stops for the architect: the metric or the mechanism is wrong.
  2. `box_readings` (ignored, a reading) prints the table for every scene and pose, today's
     look and each scratch, with the count-class and air-face splits and the tile-contrast
     share; the log carries it.
  3. The side-by-sides exist for both poses and the log says in one line per pose whether
     the red pixels land on the owner's rods, pillars and cubes, or where they do not.
  4. The log states, per scratch, the prediction and the reading.

### Phase 2: The plane fit on a majority-filtered occupancy
- **Owner skill:** dev
- **What:** behind `SurfaceFit::filter` (`OccupancyFilter::None | Majority`, reaching the
  attribute pass through its fit bits, `--fit-filter none|majority` and the banner), the
  attribute pass loads the brick's occupancy with a two-voxel shell (a 12-cube), computes
  each inner cell's 27-count, and fits every attributed voxel's plane on the filtered
  occupancy (solid when the count is at least 14). A voxel whose filtered neighbourhood is
  all solid keeps a plane whose normal is the unfiltered fit's and whose offset is the clamp
  (a cube face with the hull's normal); one whose filtered neighbourhood is empty has no
  plane (a cube, as today). The surface mask, the attribute's encoding, the walk and the
  storage do not change. The work list's 26 neighbours cover the two-voxel reach.
- **Files touched:** `crates/vxe-render/src/shaders/attrib.wgsl` (the 12-cube, the counts,
  the filtered fit), `crates/vxe-render/src/marcher.rs` and `gpu_world.rs` (the setting and
  its bit), `apps/flythrough/src/lib.rs`, `main.rs`, `hud.rs` (the flag, the banner),
  `apps/flythrough/tests/look.rs`, `crates/vxe-render/tests/mirror.rs` (a filtered
  attribute pinned), this plan's log.
- **Done when:**
  1. `the_majority_fit_melts_one_voxel_features` asserts on lavapipe that `rod`, `lone cube`
     and `wall pillar` read 0 box pixels under the filter and that the bare floor and the
     floor and wall scenes of Plan 0050 read the same hit mask and normals view byte for byte
     with and without it (every voxel next to their surfaces counts 18: nothing is filtered).
     The tilted and 45-degree slabs are readings: a slab voxel whose centre lies just inside
     the plane counts under 14 and melts, so their images move a little, and their crack
     classes 1 and 2 must stay 0. `groove` and `ridge` are readings in the table.
  2. A mirror test pins the rod voxel's attribute under the filter: normal `(0, 0, 1)`, offset
     code 3 (offset -0.5, its bottom face), and the floor voxel beside it the flat floor's
     attribute (offset code 11), bit-exact.
  3. `free pillar` and `sheet` under the filter are read, not barred: the log records their
     hit counts against today's and where their lower rows thin and their upper rows stay
     cubes (ADR-0040's hazard, stated as the reading).
  4. The box and tile-contrast tables of Phase 1 are re-read at both poses under the filter,
     with the crack classes (classes 1 and 2 stay 0 under snap: the filter must open no crack)
     and Plan 0039's guardrails (cavity normal error at most 12 degrees, pillar silhouette at
     most half of face mode's, `A` at most 10 %).
  5. The attribute pass's cost: the level's initial fill and the autopilot flight's p50 and
     p99 on both adapters, filter off and on, three alternated runs, release, perf lock; the
     bar is 1.8x (a guess from 1728 against 1000 cells loaded; the owner may move it).
     `mirror_bytes` and `allocated_bytes` equal the pre-plan binary's.

### Phase 3: The normal field, and the cubes-shaded-by-the-field preset
- **Owner skill:** dev
- **What:** behind `SurfaceTuning::normal_field` (a flag bit, `--normal-field on|off`, the
  banner), the light pass shades a contour hit with the normalised trilinear blend, over the
  eight voxel centres around the hit point, of the plane normals of the voxels that carry one
  (air, no-plane and eroded-to-no-plane voxels contribute nothing; where nothing contributes,
  the hit's own normal). The hit record and the shadow origin keep the plane normal; the
  normals view and the detail tilt take the field. Where the eight reads happen (the hit pass
  writing a second normal, or the light pass reading the attributes at the hit position) is
  dev's call under the cost bar; the hit record's layout is not grown. A look preset
  `cubes-field` hits every voxel as a cube (the plane test off) and shades with the field:
  ADR-0040's Alternative D, the fallback the owner compares.
- **Files touched:** `crates/vxe-render/src/shaders/light.wgsl` (the field), `march.wgsl`
  (the plane test off for the preset, the field if it lives in the hit pass),
  `composite.wgsl` (the normals view), `crates/vxe-render/src/marcher.rs`,
  `apps/flythrough/src/lib.rs` (`LookPreset::CUBES_FIELD`), `main.rs`, `hud.rs`,
  `apps/flythrough/tests/look.rs`, `sharpness.rs`, this plan's log.
- **Done when:**
  1. `the_normal_field_has_no_tile_edges` asserts on lavapipe that `rod`, `lone cube` and
     `groove` read 0 tile-contrast pairs under the field with the filter on, prints the
     readings with the filter off and for `ridge` (amended 2026-10-31 (owner): it asked 0 for
     all four, filter off and on; with the filter off the rod reads 0.145 % and the groove
     0.195 %, every rod pair across its 0.375-voxel step inside the metric's 1.5-voxel reach,
     and the ridge 0.005 to 0.010 %; the field alone is therefore not a default candidate,
     only field plus filter), and that the hit mask of every scene is byte for byte the same
     with the field on and off (the field moves no hit).
  2. Plan 0039's guardrails under the field: cavity normal error at most today's 7.13 degrees
     (the blend rule read 5.81 with a related normal), `A` at most 10 %, the silhouette
     unchanged; the speck test's total at most today's (amended 2026-10-31 (owner): it read
     "the speck test's reading", per class; the shadow-only class went 6 to 8 while the
     totals fell 365 to 240 with shadows and 360 to 233 without; the total is the bar and the
     classes are readings).
  3. `cubes-field` reads the face silhouette (172 px against the pillar) and a cavity normal
     error at most 12 degrees; the banner names it.
  4. The cost: the hit and light passes at the level's spawn on both adapters, field off and
     on, filter on, three alternated runs against the Plan 0050 close binary, release, perf
     lock, 1080p from 720p. The bar: hit plus light within +10 % (RTX) and +20 % (Radeon), a
     guess the owner may move; the reading goes to the log either way, and a miss does not
     stop the phase, since Phase 4 weighs the look against the cost.
  5. The box and tile-contrast tables at both poses under each of the four combinations
     (filter off/on, field off/on) and under `cubes-field`, with the crack classes.

### Phase 4: The owner's windowed check
- **Owner skill:** human
- **What:** the owner runs the release fly-through at `--pose vein` and `--pose patch`, lamp
  on and off, shaded and normals views, in six looks: today's (snap), `--fit-filter
  majority`, `--normal-field on`, both, `--look cubes-field`, and Plan 0054's `--look field`
  (its Phase 1 spike, stacked on this branch after Phase 3, with its cost and census tables in
  hand), looking at the rods and pillars along walls and ledge edges and the lone cubes on
  floors and ceilings from the Phase 5 screenshots of Plan 0050, and at the steps that remain
  at ledge corners.
- **Files touched:** this plan's log and Plan 0054's.
- **Done when:** the owner records one of: (a) a pick among the first four becomes the
  default, with the cost reading of Phase 3 accepted or a bar set; (b) the field surface is
  the pick (Plan 0054's Phase 2 outcome (a), its bars set there) and this plan's Phase 5 lands
  nothing but the removals Plan 0054 does not cover; (c) `cubes-field` is the default
  (contours dropped, ADR-0011 superseded). Either way Phase 5 runs.

### Phase 5: The pick as the default, the losers removed
- **Owner skill:** dev
- **What:** the owner's pick becomes the contour looks' default (or `cubes-field` the
  fly-through's), the goldens that move are re-blessed by name under Plan 0050's owner answer
  2 (the face goldens must not move), the readings go to `nfr.md` §2, and the losing settings
  are removed with their flags and banner modes as Plan 0048's rule says, unless the owner
  keeps one for comparison. ADR-0040 is accepted at the close on (a), or marked superseded
  with the owner's choice on (b).
- **Files touched:** `apps/flythrough/src/lib.rs` (the default), the moved goldens,
  `apps/flythrough/tests/look.rs`, `docs/nfr.md` §2, the files of Phases 2 and 3 for the
  removals, this plan's log.
- **Done when:**
  1. `the_contour_surface_has_no_one_voxel_boxes` asserts on lavapipe, at the default, 0 box
     pixels on `rod`, `lone cube` and `wall pillar` under the analytic window rule, 0 tile-contrast
     pairs on `rod`, `lone cube`, `groove` and `ridge`, and at both poses box pixels at most
     the pick's Phase 3 reading and classes 1 and 2 at 0. On (b) with `cubes-field` the test
     asserts the tile-contrast bars and the face silhouette instead, and says so.
  2. Every moved golden is named in the log with its share of pixels over 2/255; no face
     golden moved.
  3. The hit, light and attribute pass readings on both adapters are in `nfr.md` §2 with
     machine, commit and load; mirror bytes are unchanged.
  4. No losing setting's flag remains in `--help` or the banner.

  Amended 2026-10-31 (architect, on the owner's Phase 4 outcome (b), the field surface): this
  phase has no work of its own. Plan 0054 Phase 4 makes the field the default walk and removes
  the planes, and with them this plan's majority filter, normal field and `cubes-field` with
  their flags and banner words; Plan 0054 Phase 5 marks ADR-0040 superseded by the field
  surface's ADR. This plan closes together with Plan 0054, and its close review checks that
  nothing of Phases 2 and 3 but the box metric and its tests survives.

## Data shapes

```rust
// illustrative: vxe-render
pub enum OccupancyFilter { None, Majority }   // the fit's occupancy; a bit in the attribute pass's fit word
pub struct SurfaceFit {
    pub fit: PlaneFit,                // unchanged
    pub offset: PlaneOffset,          // unchanged
    pub filter: OccupancyFilter,      // new; lives in the mirror like `fit`
}
pub struct SurfaceTuning {
    pub fit: SurfaceFit,
    pub face_mix: f32,                // unchanged
    pub closing: CrackClosing,        // Plan 0050's
    pub shadow_push: bool,            // Plan 0050's
    pub normal_field: bool,           // new; a spare Params flag bit
}
// The attribute's u16, its pages, the surface mask, the hit record and the pool layout do not
// change. `cubes-field` is a LookPreset, not a new NormalMode.
```

## Budgets

| `nfr.md` figure | Base (read) | Expected | How the close measures it |
|---|---|---|---|
| §2 the hit pass and the light pass | level spawn, 1080p from 720p, snap alone: RTX 0.604 + 0.913 ms, Radeon 4.364 + 7.026 ms (Plan 0050) | filter: 0 (the walk is unchanged). Field: eight attribute reads per shaded pixel, **a guess** of +5 to +10 % RTX, +10 to +20 % Radeon on hit plus light | `flythrough --level --seed 1 --headless --render 1920x1080 --frames 300 --adapter NVIDIA\|AMD` per setting and the Plan 0050 close binary, alternated, release, perf lock, memory cap, three quiet runs; bar in Phase 3 |
| §2 the attribute pass | the level's fill 17.0 ms (weighted fit, dev box, Plan 0039); the flight's p50 2.0 / p99 4.3 ms GPU (Plan 0013, dev box) | filter: a 12-cube loaded and 1000 counts of 27 before 512 fits of 27, **a guess** of 1.5 to 1.8x | the initial fill and `--autopilot --headless --seed 1` p50 and p99 on both adapters, filter off and on, three alternated runs; bar 1.8x in Phase 2 |
| §3 mirror bytes | 157 872 640 B live, 259 203 264 B allocated at the level (Plan 0050) | 0 | `mirror_bytes` and `allocated_bytes` equal the pre-plan binary's in every run |

Determinism is not touched: everything is in `vxe-render`, a client-side view.

## Risks & open questions

- **The metric may not land on the owner's boxes.** Phase 1's side-by-sides are the check;
  if the red pixels miss the photographed rods, dev stops and the architect re-reads the hit
  records with the owner's screenshots in hand.
- **Features under the filter change while they still collide.** A rod, a lone cube or a
  one-voxel bump on a surface melts into it; a free pillar or a sheet keeps its upper rows as
  cubes over a thinned base (ADR-0040's negative consequence). Phase 2 reads both and the
  owner sees them at Phase 4. A per-material opt-out of the filter is a followup if the
  consumer asks.
- **Two-voxel features keep flat sides and concave corners stay cubes.** The field normal
  shades them smoothly; the geometric step stays. Phase 4's outcome (b) names the field
  surface for them.
- **The field's cost on the slow adapter.** Eight attribute reads per shaded pixel through
  the popcount addressing. The bar is a guess; the reading, the look and the owner decide.
- **The attribute pass under flight.** A 12-cube in workgroup memory and 1000 extra counts
  per brick: Plan 0023's hitch budget is on the render thread's CPU, which this does not
  touch, but the GPU pass under the autopilot is read on both adapters.
- **Snap and the filter.** An eroded voxel's plane is at or beyond its cube on the air side,
  so rays pass it or cross onto the hull: classes 1 and 2 must stay 0. Phase 2's done-when 4
  reads it.

## What this plan does NOT do

- Build the field surface (ADR-0040 Alternative B) or dilation (Alternative C): the next plan
  on Phase 4's outcome (b), with its own storage and walk cost spike.
- Change the attribute's encoding, the surface mask, the pages, the hit rule or the closing
  rule.
- Give the LOD ring's mip cells a plane, or touch the lighting beyond which normal it shades
  with (Plan 0051 is the light spike).
- A per-material opt-out of the filter.

## Owner answers

1. **Phase 3's two bars (owner, 2026-10-31, recommended options).** Done-when 1 reads 0 tile
   edges under the filter for rod, lone cube and groove, the rest as readings; done-when 2's
   speck bar is the total, not the per-class count. The orchestrator wrote both amendments.
2. **Phase 2's edge rule (architect, 2026-10-31).** Accepted into Phase 1's box metric; the
   definition carries it. Plan 0054's spike read its zeros under this rule.

3. **Phase 4, the owner's windowed check (owner, 2026-10-31, session 1 with Plan 0054's
   Phase 2, 14 runs).** Outcome (b): the boxes still fail the eye in the plane looks and the
   owner chooses the field surface (ADR-0040 Alternative B, Plan 0054), with N1 analytic
   normals. Readings by look, in the owner's words: today, the filter, the field and both at
   the vein "same problem" (the rods and slabs stand in every one); at the patch "very few
   blocks, maybe acceptable" for the plane looks, so the plane looks fail at the vein, not
   the patch. `cubes-field` at the vein: "not even too bad, if not for the lighting flickers";
   at the patch: "not acceptable really, there are small light lines" along the cube edges
   under the lamp (a finding: with cube hits on, the field normal and the cube face disagree
   at an edge and a sliver of lamp light passes). The field surface: "smooth, wonderful" at
   both poses; N2 "not much of a visual difference". Phase 3's RTX cost miss (+12 % hit plus
   light for the normal field) is moot with the pick. The flicker is the lighting's (Plan
   0051), seen in every look alike.

## Implementation log

> Written by `dev`: one row per phase as its commit lands, then the close block. **The phases
> above are the contract. This section records what happened.** Observations, never conclusions:
> no pass list, no self-assessment. Deviations and unmet done-whens are **always** disclosed.
> Keep it shorter than `## Implementation phases`.

| phase | owner | state | commit |
|---|---|---|---|
| 1: The box spike: two metrics, six scenes, the poses, and the hypotheses read | dev | done | eaea0f0 |
| 2: The plane fit on a majority-filtered occupancy | dev | done | 11a9b1e |
| 3: The normal field, and the cubes-shaded-by-the-field preset | dev | done (the code in 811b37e, the amended bars' tests with this row) | committed with this row |
| 4: The owner's windowed check | human | done (b): the field surface (owner, 2026-10-31; owner answer 3) | owner answers, this commit |
| 5: The pick as the default, the losers removed | dev | subsumed by Plan 0054 Phases 4 and 5 (amended 2026-10-31) | |

#### Phase 1

Readings: lavapipe (llvmpipe, Mesa 26.2.5, LLVM 23.1.3), exclusive perf lock, 1-minute load
0.6 to 2.2, worktree at cde413a plus this phase's diff. `cargo nextest run -p flythrough --test
look --run-ignored only --no-capture box_readings`; the scenes at 320 x 180 under the analytic
window rule, the level at 384 x 216 with the pose's exposure, indirect off, no upscaler. Box px
splits: the box voxel's 27-count (under 14, 14 and over) and its air faces (2, 3, 4 or more).
Tile edges: share of the counted adjacent pairs within 1.5 voxels whose normals-view normals
(detail off) differ by more than 15 degrees. Start look (snap):

| scene | hits | left out | box px | < 14 | >= 14 | 2 air | 3 air | 4+ air | pairs | tile edges % | class 1 % | class 2 % | class 3 % |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| bare floor | 50747 | 77 | 0 | 0 | 0 | 0 | 0 | 0 | 97251 | 0.000 | 0.000 | 0.000 | 0.000 |
| rod | 50747 | 77 | 503 | 503 | 0 | 0 | 422 | 81 | 97251 | 0.785 | 0.000 | 0.000 | 0.747 |
| lone cube | 50728 | 76 | 24 | 24 | 0 | 0 | 0 | 24 | 97232 | 0.000 | 0.000 | 0.000 | 0.000 |
| groove | 51626 | 63 | 703 | 0 | 703 | 703 | 0 | 0 | 98926 | 0.647 | 0.000 | 0.000 | 1.253 |
| wall pillar | 41400 | 180 | 276 | 276 | 0 | 0 | 210 | 66 | 80260 | 0.381 | 0.000 | 0.000 | 0.077 |
| ridge | 50729 | 76 | 575 | 118 | 457 | 457 | 118 | 0 | 97216 | 0.728 | 0.000 | 0.000 | 1.459 |
| free pillar | 48590 | 110 | 830 | 830 | 0 | 0 | 0 | 830 | 93192 | 0.098 | 0.000 | 0.000 | 0.000 |
| sheet | 47782 | 116 | 8143 | 7751 | 392 | 8059 | 84 | 0 | 91542 | 0.965 | 0.000 | 0.000 | 0.375 |
| level vein | 82944 | 0 | 35208 | 0 | 35208 | 21804 | 13404 | 0 | 165288 | 0.676 | 0.000 | 0.000 | 0.581 |
| level vein lamp off | 82944 | 0 | 35208 | 0 | 35208 | 21804 | 13404 | 0 | 165288 | 0.676 | 0.000 | 0.000 | 0.581 |
| level patch | 82944 | 0 | 38371 | 200 | 38171 | 24704 | 13467 | 200 | 165133 | 0.771 | 0.000 | 0.000 | 2.318 |
| level patch lamp off | 82944 | 0 | 38371 | 200 | 38171 | 24704 | 13467 | 200 | 165133 | 0.771 | 0.000 | 0.000 | 2.318 |

- `offset volume` (`PlaneOffset::VolumeMatched`) reads the same row on all eight scenes; at the
  poses: vein 29654 box px (0, 29654; 17826, 11828, 0), tile edges 0.676 %, class 3 1.594 %;
  patch 34759 (200, 34559; 21602, 12957, 200), 0.766 %, class 3 4.151 %. Lamp off reads the
  same rows as lamp on in every look.
- Scratch, neighbour clipping (`live/scratch/neighbour_clip.py` patches `march.wgsl`'s
  `contour_test`: the segment through the cube is clipped by the voxel's own plane and the
  planes of its six face neighbours that carry one, hit at its first point inside all of them,
  passed when empty; read, reverted). Prediction: `rod` box px fall by about a third, `groove`
  unchanged. Reading: `rod` 503 (unchanged), `groove` 621 (from 703); `wall pillar` 276,
  `lone cube` 24, `free pillar` 830 unchanged; `ridge` 572, `sheet` 8062; tile edges rise on
  every scene with a feature (`rod` 1.600 %, `groove` 1.105 %, `ridge` 1.652 %); vein 27062 box
  px, class 3 3.719 %; patch 31918, class 3 8.735 %. The floor voxel the ADR's arithmetic cuts
  the rod with, `(y - 1, z - 1)` of the rod voxel, is an edge neighbour, not a face neighbour;
  the face neighbour under the rod reads count 21, normal +z, offset +0.833, the rod's own
  plane height.
- Scratch, clamp 0.5 (`attrib.wgsl`'s clamp patched to `[-0.5, 0.5]`, read, reverted).
  Prediction: no change on `rod`. Reading: `rod` 503 (unchanged), every scene's box px unchanged
  but `sheet` 8177; vein 31390 box px, patch 34236; class 3 rises at both (2.719 %, 5.487 %).
- `offset volume` prediction (no change on `rod`, -0.167 for an axis normal either way):
  reading `rod` 503, every scene unchanged.
- Done-when 1: `one_voxel_features_render_as_boxes_today` (lavapipe) reads `rod` 503, `lone
  cube` 24, `groove` 703, `wall pillar` 276 box px, `bare floor` 0.
- Done-when 3, side-by-sides (untracked) `live/boxes/{vein,patch}-start-look[-lamp-off].png`
  (shaded, normals view, shaded with box pixels red where the voxel counts 14 or more, yellow
  under 14), and one per scene and look:
  - Vein: red covers 42 % of the hits, triangles and quads on nearly every voxel tile of the
    wall and ledge in frame (the cube-face part of clipped surface voxels with 2 or 3 air faces,
    every one of count 14 or more); no yellow; no rod, pillar or lone cube of the owner's
    screenshots (`.git/lane-archive/plan-0050/live/p5/`) is resolved in this 384 x 216 frame, so
    the red does not single them out.
  - Patch: red covers 46 % of the hits the same way over the floor and the wall; 200 yellow
    pixels (count under 14, 4 or more air faces) on one protrusion at the frame's right edge by
    the floor's edge, and a row of red rectangles along the floor's edge under the wall.
- Deviations: the scenes' floor is rock at `z < 60` with the features at `z = 60`, and the
  wall pillar's wall rock at `x >= 76` (`FEATURE_Z`, `WALL_FEATURE_X` in `rigs.rs`), not
  `floor_world` (`z < 64`) and `wall_world` (`x >= 80`): their surfaces end on a brick face, so
  the floor's and the wall's top bricks are uniform rock without attributes (first read on the RTX 3080 Laptop:
  `rod` tile edges 0.000 %, every floor hit an axis cube face); the floor camera is placed 10
  voxels from each feature's centre at the floor camera's pitch and 45 degrees of yaw, the
  wall pillar's along the wall camera's direction; the side-by-sides paint box pixels of count
  under 14 yellow; `box_readings` takes `VXE_BOX_TAG` to name a scratch build's rows.
- Gate at this commit: build, clippy `-D warnings` 0, fmt 0, doctests 11 ok, the four
  `scripts/check-*.mjs` 0; `--profile quick` 539 run, 538 passed, 1 timeout at load 23
  (`flythrough::replay a_stamp_and_a_collapse_replay_from_the_save` 30 s; alone 13.8 s), 3
  skipped; `--profile gpu --no-fail-fast` (perf lock, load 1.3 to 3.2) 186 run, 185 passed, 1
  failed, 30 skipped: `sharpness detail_after_lighting_is_exact_for_the_lamp`, the base's
  failure (Plan 0050 Phase 6's floor question), untouched.

#### Phase 2

Readings as Phase 1's (lavapipe, exclusive perf lock, load 0.9 to 3.3, `box_readings`), at
eaea0f0 plus this phase's diff, with the box metric's edge rule below; `majority` is the start
look with `--fit-filter majority`. Lamp off reads the same rows as lamp on.

| scene | look | hits | left out | box px | < 14 | >= 14 | 2 air | 3 air | 4+ air | pairs | tile edges % | class 1 % | class 2 % | class 3 % |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| bare floor | start look | 50747 | 77 | 0 | 0 | 0 | 0 | 0 | 0 | 97251 | 0.000 | 0.000 | 0.000 | 0.000 |
| rod | start look | 50747 | 77 | 499 | 499 | 0 | 0 | 418 | 81 | 97251 | 0.785 | 0.000 | 0.000 | 0.747 |
| lone cube | start look | 50728 | 76 | 24 | 24 | 0 | 0 | 0 | 24 | 97232 | 0.000 | 0.000 | 0.000 | 0.000 |
| groove | start look | 51626 | 63 | 697 | 0 | 697 | 697 | 0 | 0 | 98926 | 0.647 | 0.000 | 0.000 | 1.253 |
| wall pillar | start look | 41400 | 180 | 276 | 276 | 0 | 0 | 210 | 66 | 80260 | 0.381 | 0.000 | 0.000 | 0.077 |
| ridge | start look | 50729 | 76 | 568 | 115 | 453 | 453 | 115 | 0 | 97216 | 0.728 | 0.000 | 0.000 | 1.459 |
| free pillar | start look | 48590 | 110 | 828 | 828 | 0 | 0 | 0 | 828 | 93192 | 0.098 | 0.000 | 0.000 | 0.000 |
| sheet | start look | 47782 | 116 | 8108 | 7717 | 391 | 8026 | 82 | 0 | 91542 | 0.965 | 0.000 | 0.000 | 0.375 |
| level vein | start look | 82944 | 0 | 35098 | 0 | 35098 | 21734 | 13364 | 0 | 165288 | 0.676 | 0.000 | 0.000 | 0.581 |
| level patch | start look | 82944 | 0 | 38250 | 199 | 38051 | 24624 | 13427 | 199 | 165133 | 0.771 | 0.000 | 0.000 | 2.318 |
| bare floor | majority | 50747 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 97251 | 0.000 | 0.000 | 0.000 | 0.000 |
| rod | majority | 50747 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 97251 | 0.000 | 0.000 | 0.000 | 0.824 |
| lone cube | majority | 50728 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 97232 | 0.000 | 0.000 | 0.000 | 0.000 |
| groove | majority | 51626 | 0 | 4870 | 0 | 4870 | 4870 | 0 | 0 | 98827 | 0.000 | 0.000 | 0.000 | 0.000 |
| wall pillar | majority | 41400 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 80260 | 0.000 | 0.000 | 0.000 | 0.063 |
| ridge | majority | 50729 | 0 | 498 | 63 | 435 | 435 | 63 | 0 | 97232 | 0.622 | 0.000 | 0.000 | 1.768 |
| free pillar | majority | 48590 | 0 | 1182 | 1182 | 0 | 0 | 0 | 1182 | 93160 | 0.114 | 0.000 | 0.000 | 0.000 |
| sheet | majority | 47803 | 0 | 14497 | 13912 | 585 | 7468 | 5700 | 1329 | 92035 | 0.759 | 0.000 | 0.000 | 0.987 |
| level vein | majority | 82944 | 0 | 39263 | 0 | 39263 | 22337 | 16926 | 0 | 165288 | 0.717 | 0.000 | 0.000 | 0.565 |
| level patch | majority | 82944 | 0 | 42080 | 257 | 41823 | 25769 | 16054 | 257 | 165006 | 1.208 | 0.000 | 0.000 | 2.171 |

- The box metric's edge rule (this commit): a hit point that also lies within 1e-3 of one of
  the box voxel's faces toward a solid neighbour is not a box pixel. Under the filter the rod
  read 8 box pixels without it, every one at `z = 60.000` on the outline of the rod's footprint
  (a plane hit on the floor's level, within 1e-3 of the rod's side or end face); with it, 0.
  The start look's rows above are Phase 1's re-read under the rule (rod 503 to 499, groove 703
  to 697, ridge 575 to 568, free pillar 830 to 828, sheet 8143 to 8108, vein 35208 to 35098,
  patch 38371 to 38250; done-when 1 of Phase 1 still reads `rod` 499, `lone cube` 24, `groove`
  697, `wall pillar` 276, `bare floor` 0).
- Done-when 1: `the_majority_fit_melts_one_voxel_features` (lavapipe) reads 0 box px on `rod`,
  `lone cube` and `wall pillar`; the hit mask and normals view equal byte for byte at the
  pixels the analytic window rule keeps in both renders: bare floor 50747 hits (77 left out),
  Plan 0050's floor 43012 (392), wall 37800 (180); tilted slab 49357 hits, 113 left out, clean
  98.17 %, classes 1 and 2 0, class 3 1.825 %; 45-degree slab 48333, 19, 97.90 %, 0, 0, 2.102
  %.
- Done-when 2: `the_majority_filter_fits_a_rod_the_floors_plane` (`mirror.rs`, lavapipe):
  under every fit with the filter the rod voxel reads `0x37df` (normal `(0, 0, 1)` bit-exact,
  code 3), the floor voxel beside it `0xb7df`, the far floor's word (code 11); without the
  filter the rod voxel reads code 6. Mirror bytes equal with and without the filter.
- Done-when 3, `free pillar`: 48590 hits with and without the filter, box px 828 to 1182.
  Per voxel row (RTX 3080 Laptop scratch, `VXE_TEST_ADAPTER=rtx`): the lowest row (z 60) 30
  box px to 0 (its plane is its bottom face), rows 61 to 64 unchanged (150, 182, 210, 256), the
  top row (z 65) 0 to 384 (a cube without a plane under the filter; today a plane with offset
  -0.833). `sheet`: hits 47782 to 47803, box px 8108 to 14497; per row z 60 473 to 647, z 61
  752 to 0, z 62 to 66 893, 1068, 1300, 1603, 2019 to 1154, 1398, 1721, 2152, 2761, the top row
  (z 67) 0 to 4664; the end columns and the top row, cut by their planes today, are full cubes
  under the filter (`live/boxes/sheet-majority.png`).
- Done-when 4: the poses under the filter, classes 1 and 2 at 0 (table). Guardrails
  (`the_majority_fit_meets_the_guardrails`, lavapipe): cavity normal error 7.16 degrees
  (start look 7.13), silhouette 32 px against face 172 (32), `A` 6.78 % (6.73 %).
- Done-when 5: the attribute pass, `flythrough --level --headless --seed 1 --render 1920x1080
  --frames 300` (the initial fill) and `flythrough --autopilot --headless --seed 1` (GPU p50,
  p99 over the frames with work), `--adapter NVIDIA|AMD`, `--fit-filter none|majority`,
  release, each run under the exclusive perf lock and the memory cap, three alternated rounds,
  medians; 1-minute load 1.0 to 11.0 (the autopilot's own world generation loads the box):
  loaded, indicative. Adapters selected by `--adapter`.

  | adapter | fill none | fill majority | ratio | p50 none | p50 majority | ratio | p99 none | p99 majority | ratio |
  |---|---|---|---|---|---|---|---|---|---|
  | NVIDIA GeForce RTX 3080 Laptop GPU | 14.216 | 20.593 | 1.45 | 0.906 | 1.355 | 1.50 | 1.196 | 1.874 | 1.57 |
  | AMD Radeon Graphics (RADV RENOIR) | 59.767 | 73.473 | 1.23 | 1.272 | 1.569 | 1.23 | 1.613 | 2.053 | 1.27 |

  The pre-plan binary (cde413a), three runs alternated with the tip's `none` at load 1.6 to
  3.8: initial fill 17.84 ms (RTX), 95.57 ms (Radeon) against 14.20 and 59.73. Mirror 157 872
  640 B live, 259 203 264 B allocated in every run of both binaries. The first
  implementation (one 12-bit row a word, the filtered rows in their own array, 15 776 B of
  workgroup memory, a per-cell neighbour switch and a per-cell popcount count) read fill 20.34
  (none) and 47.28 ms (majority) on the RTX (2.32x), p50 1.249 and 2.859, p99 1.793 and
  3.583; Radeon fill 81.34 and 141.47 (1.74x); rewritten before this commit (rows paired, the
  filtered rows in the staged masks' words, a brick's row run from one mask word, the count
  bit-sliced; a loop form of the same adds read RTX fill 14.200 and 21.227, Radeon 59.804
  and 73.405, and was unrolled for the shader-portability test), the table above.
- Deviations: files outside the list: `crates/vxe-render/src/attrib.rs` (`OccupancyFilter`,
  `SurfaceFit::filter`, the bit), `crates/vxe-render/src/lib.rs` (the export),
  `apps/flythrough/tests/sharpness.rs` (`..SurfaceFit::default()`); `marcher.rs` and
  `gpu_world.rs` unchanged (the fit's bits reach the pass through `SurfaceFit::bits`); a voxel
  whose six face neighbours are all filtered solid counts as buried (the unfiltered normal at
  offset 7/8); without the filter the pass loads only the 10-cube of the 12-cube's rows; the filtered rows
  live in the staged neighbour masks' words once the occupancy is set; the
  byte-for-byte comparison of done-when 1 is over the pixels the analytic window rule keeps
  (the floor's edge voxels on the window's faces erode under the filter); the box metric's edge
  rule above; the banner names `fit filter majority`, the look line `+ majority`.
- Gate at this commit: build, clippy `-D warnings` 0, fmt 0, doctests 11 ok, `cargo deny check` ok, the four
  `scripts/check-*.mjs` 0; `--profile quick` 539 run, 538 passed, 1 timeout (`flythrough::replay
  a_stamp_and_a_collapse_replay_from_the_save` 30 s, load 1.1 at the start; alone 13.8 s), 3
  skipped; `--profile gpu --no-fail-fast` (perf lock, load 1.2 to 6.6) 189 run, 188 passed, 1
  failed (`sharpness detail_after_lighting_is_exact_for_the_lamp`, the base's), 30 skipped.

#### Phase 3

Readings as Phase 2's (lavapipe, exclusive perf lock, `box_readings`), at 11a9b1e plus this
phase's diff; `field` is the start look with `--normal-field on`, `majority + field` with both,
`cubes-field` the preset. The start look and `majority` rows read as Phase 2's table. Lamp off
reads the same rows as lamp on.

| scene | look | hits | left out | box px | < 14 | >= 14 | 2 air | 3 air | 4+ air | pairs | tile edges % | class 1 % | class 2 % | class 3 % |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| bare floor | field | 50747 | 77 | 0 | 0 | 0 | 0 | 0 | 0 | 97251 | 0.000 | 0.000 | 0.000 | 0.000 |
| rod | field | 50747 | 77 | 499 | 499 | 0 | 0 | 418 | 81 | 97251 | 0.145 | 0.000 | 0.000 | 0.747 |
| lone cube | field | 50728 | 76 | 24 | 24 | 0 | 0 | 0 | 24 | 97232 | 0.000 | 0.000 | 0.000 | 0.000 |
| groove | field | 51626 | 63 | 697 | 0 | 697 | 697 | 0 | 0 | 98926 | 0.195 | 0.000 | 0.000 | 1.253 |
| wall pillar | field | 41400 | 180 | 276 | 276 | 0 | 0 | 210 | 66 | 80260 | 0.042 | 0.000 | 0.000 | 0.077 |
| ridge | field | 50729 | 76 | 568 | 115 | 453 | 453 | 115 | 0 | 97216 | 0.005 | 0.000 | 0.000 | 1.459 |
| free pillar | field | 48590 | 110 | 828 | 828 | 0 | 0 | 0 | 828 | 93192 | 0.093 | 0.000 | 0.000 | 0.000 |
| sheet | field | 47782 | 116 | 8108 | 7717 | 391 | 8026 | 82 | 0 | 91542 | 0.592 | 0.000 | 0.000 | 0.375 |
| level vein | field | 82944 | 0 | 35098 | 0 | 35098 | 21734 | 13364 | 0 | 165288 | 0.034 | 0.000 | 0.000 | 0.581 |
| level patch | field | 82944 | 0 | 38250 | 199 | 38051 | 24624 | 13427 | 199 | 165133 | 0.342 | 0.000 | 0.000 | 2.318 |
| bare floor | majority + field | 50747 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 97251 | 0.000 | 0.000 | 0.000 | 0.000 |
| rod | majority + field | 50747 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 97251 | 0.000 | 0.000 | 0.000 | 0.824 |
| lone cube | majority + field | 50728 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 97232 | 0.000 | 0.000 | 0.000 | 0.000 |
| groove | majority + field | 51626 | 0 | 4870 | 0 | 4870 | 4870 | 0 | 0 | 98827 | 0.000 | 0.000 | 0.000 | 0.000 |
| wall pillar | majority + field | 41400 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 80260 | 0.000 | 0.000 | 0.000 | 0.063 |
| ridge | majority + field | 50729 | 0 | 498 | 63 | 435 | 435 | 63 | 0 | 97232 | 0.010 | 0.000 | 0.000 | 1.768 |
| free pillar | majority + field | 48590 | 0 | 1182 | 1182 | 0 | 0 | 0 | 1182 | 93160 | 0.117 | 0.000 | 0.000 | 0.000 |
| sheet | majority + field | 47803 | 0 | 14497 | 13912 | 585 | 7468 | 5700 | 1329 | 92035 | 0.454 | 0.000 | 0.000 | 0.987 |
| level vein | majority + field | 82944 | 0 | 39263 | 0 | 39263 | 22337 | 16926 | 0 | 165288 | 0.008 | 0.000 | 0.000 | 0.565 |
| level patch | majority + field | 82944 | 0 | 42080 | 257 | 41823 | 25769 | 16054 | 257 | 165006 | 0.118 | 0.000 | 0.000 | 2.171 |
| bare floor | cubes-field | 50747 | 153 | 0 | 0 | 0 | 0 | 0 | 0 | 97251 | 0.000 | 0.000 | 0.000 | 0.000 |
| rod | cubes-field | 50747 | 153 | 3003 | 3003 | 0 | 0 | 2285 | 718 | 97031 | 0.000 | 0.000 | 0.000 | 0.000 |
| lone cube | cubes-field | 50728 | 152 | 282 | 282 | 0 | 0 | 0 | 282 | 97200 | 0.000 | 0.000 | 0.000 | 0.000 |
| groove | cubes-field | 51626 | 121 | 4870 | 0 | 4870 | 4870 | 0 | 0 | 98827 | 0.100 | 0.000 | 0.000 | 0.000 |
| wall pillar | cubes-field | 41400 | 180 | 1438 | 1438 | 0 | 0 | 956 | 482 | 80158 | 0.000 | 0.000 | 0.000 | 0.000 |
| ridge | cubes-field | 50729 | 153 | 4490 | 1080 | 3410 | 3410 | 1080 | 0 | 97000 | 0.016 | 0.000 | 0.000 | 0.000 |
| free pillar | cubes-field | 48590 | 206 | 1312 | 1312 | 0 | 0 | 0 | 1312 | 93168 | 0.093 | 0.000 | 0.000 | 0.000 |
| sheet | cubes-field | 47803 | 219 | 15630 | 14988 | 642 | 8277 | 6024 | 1329 | 92115 | 0.543 | 0.000 | 0.000 | 0.000 |
| level vein | cubes-field | 82944 | 0 | 67408 | 0 | 67408 | 25570 | 41838 | 0 | 165288 | 0.048 | 0.000 | 0.000 | 0.000 |
| level patch | cubes-field | 82944 | 0 | 73887 | 655 | 73232 | 30927 | 42305 | 655 | 164977 | 0.254 | 0.000 | 0.000 | 0.000 |

- Done-when 1 (as amended 2026-10-31): `the_normal_field_has_no_tile_edges` (lavapipe)
  asserts 0 tile-contrast pairs under `majority + field` for `rod`, `lone cube` and `groove`
  and prints the rest; it passes, reading the table's rows. At 811b37e, before the amendment,
  it asserted 0 for all four scenes under both looks and failed at its first scene, `rod` under
  `field`, 0.145 % tile edges (141 pairs). From the table: under `field` `rod`
  0.145 %, `lone cube` 0, `groove` 0.195 %, `ridge` 0.005 %; under `majority + field` `rod`,
  `lone cube`, `groove` 0, `ridge` 0.010 %. The remaining pairs straddle a step in depth
  within the 1.5-voxel reach: on `rod` every one pairs a floor hit beside the rod (`z = 60`,
  field tilted about 17 degrees toward `+y`) with a hit on the rod's slab top or side (`z =
  60.375`, field within 3 degrees of `+z`), 0.6 voxel apart (RTX 3080 Laptop scratch dump).
  `the_normal_field_moves_no_hit` (lavapipe): every scene's hit mask equal byte for byte with
  the field on and off, filter off and on.
- Done-when 2: `the_normal_field_meets_the_guardrails` (lavapipe): cavity normal error 5.69
  degrees under `field`, 5.72 under `majority + field` (start look 7.13), silhouette 32 px both
  (32), `A` 5.41 and 5.50 %. `the_normal_field_adds_no_dark_speck` (contour golden scene,
  `look(Contour)`, full shadow walk) reads with, without, only with shadows 365, 360, 6
  without the field and 240, 233, 8 with it; as amended 2026-10-31 it asserts the totals
  (240 at most 365, 233 at most 360) and prints the per-class counts, and passes. At 811b37e
  it asserted the shadow-only class (8 at most 6) and failed.
- Done-when 3: `cubes-field` silhouette 172 px against face 172, cavity normal error 5.74
  degrees, `A` 5.41 %; `the_banner_names_the_view` reads `look cubes-field, detail on`.
- Done-when 4: hit and light at the level's spawn, `flythrough --level --headless --seed 1
  --render 1920x1080 --frames 300 --adapter NVIDIA|AMD [--fit-filter majority --normal-field
  off|on]`, 1080p from 1280 x 720, release, each run under the exclusive perf lock and the
  memory cap, three alternated rounds, medians of each run's median of the last 30 frames;
  "base" is cde413a's binary (the Plan 0050 close binary's stand-in); 1-minute load 0.9 to 3.4
  (round 1 at 1.6 to 3.4, rounds 2 and 3 at 0.9 to 1.9); `--adapter`.

  | adapter | binary, setting | hit ms | light ms | hit + light | against base |
  |---|---|---|---|---|---|
  | NVIDIA GeForce RTX 3080 Laptop GPU | base | 0.593 | 0.886 | 1.479 | |
  | | tip, majority, field off | 0.587 | 0.883 | 1.470 | -0.6 % |
  | | tip, majority, field on | 0.604 | 1.053 | 1.657 | +12.0 % |
  | AMD Radeon Graphics (RADV RENOIR) | base | 4.291 | 7.014 | 11.305 | |
  | | tip, majority, field off | 4.293 | 7.025 | 11.318 | +0.1 % |
  | | tip, majority, field on | 4.290 | 7.844 | 12.134 | +7.3 % |

  The RTX misses the +10 % guess (+12.0 %); the Radeon is inside +20 %. Initial fill in the
  same runs: base 17.833 and 95.612 ms, tip with the filter 20.603 and 73.495 (one RTX run read
  122.273). Mirror 157 872 640 B live, 259 203 264 B allocated in every run.
- Done-when 5: the table above, with the crack classes.
- Deviations: `SurfaceTuning::cube_hits` (flag bit 16) carries `cubes-field`'s plane test off,
  beside `normal_field` (bit 15); the field applies to every hit of the contour look, plane or
  cube; `HitRecord::nf` holds the field after `with_field`, read by `surface_detail`,
  `lighting_normal` and `detail_lamp_ratio`, while the composite's bilateral weights and the
  temporal passes keep the record's normal; `with_field` resets the walk's held words before
  its lookups; files outside the list: `probe_gather.wgsl` and `gradient.wgsl` (they shade
  through `surface_detail` and `lighting_normal`, so they take the field too); `sharpness.rs`
  unchanged; `BANNER_ROWS` 9 (the look's words wrap into three rows with the push, the filter
  and the field all named); `cubes-field` sits in `LookPreset::ALL` after `contour-grain`; the
  tests of done-whens 1 and 2 are split four ways (tile edges, hits, guardrails, specks).
- Gate at 811b37e: build, clippy `-D warnings` 0, fmt 0, doctests 11 ok, `cargo deny check`
  ok, the four `scripts/check-*.mjs` 0; `--profile quick` 539 run, 538 passed, 1 timeout
  (`flythrough::replay a_stamp_and_a_collapse_replay_from_the_save` 30 s; alone 13.7 s), 3
  skipped; `--profile gpu --no-fail-fast` 193 run, 190 passed, 3 failed, 30 skipped: the two
  tests of done-whens 1 and 2 at their first bars, and `sharpness
  detail_after_lighting_is_exact_for_the_lamp` (the base's).
- Gate at this commit: build ok; clippy `--workspace --all-targets -D warnings` 0 and `-p
  flythrough --features wasm --all-targets -D warnings` 0; fmt 0; doctests 11 binaries ok (7
  passed, 1 ignored); `cargo deny check` ok; the four `scripts/check-*.mjs` 0; `--profile
  quick` 539 run, 537 passed, 2 timed out (`flythrough::castle
  the_castle_stands_in_the_hall_with_its_gate_facing_the_spawn` 30.1 s, alone 8.9 s;
  `flythrough::replay a_stamp_and_a_collapse_replay_from_the_save` 30.0 s, alone 15.2 s; load
  3.0 at the start, 23 at the end), 3 skipped; `--profile gpu --no-fail-fast` on lavapipe
  (exclusive perf lock, 1-minute load 2.4 at the start) 193 run, 192 passed, 1 failed, 30
  skipped: `sharpness detail_after_lighting_is_exact_for_the_lamp` (98.028 %, the base's).

### Notes

_(Deviations from the plan, with the commit, stated without justification. Done-whens not
satisfiable as stated, and what was done instead. Followups noticed and not acted on. One line
each. Empty is fine.)_

- Base: a stacked lane on branch `plan-0050-p6` at cde413a (Plan 0050 Phase 6's checkpoint, no
  close commit yet), which holds what the header's "Starts after" names: the losing closing
  rules deleted, `CrackClosing` and `SurfaceTuning::closing` gone (snap the walk's only rule),
  the shadow push on flag bit 14, the cracks palette fix, the re-blessed `look_contour.png`,
  `look_wall_contour.png` and `showcase_hall.png`, and Plan 0052 (v0.37.0: lab modes removed,
  `Params::light_span` is `_pad4`, `limits_for` returns a `Result`, the probe volume lazy).
  "The Plan 0050 close binary" reads as cde413a's binary. `sharpness
  detail_after_lighting_is_exact_for_the_lamp` fails in the gpu tier on the base (98.028 %
  against the 98.3 % floor waiting on the owner) and on every phase here; not touched.
- Phase 1: the scenes' floor and wall sit inside a brick (rock at `z < 60`, `x >= 76`), not on
  `floor_world` and `wall_world`; box pixels of count under 14 are yellow in the side-by-sides.
- Phase 2: the box metric's edge rule (a point on one of the box voxel's faces toward a solid
  neighbour is not a box pixel) changes Phase 1's readings slightly; both are in the log.
- Phase 3: `SurfaceTuning::cube_hits` beside `normal_field`; the field in `probe_gather.wgsl`
  and `gradient.wgsl` too; `BANNER_ROWS` 9. The code is in 811b37e; the phase commit carries
  the two tests at the amended bars (owner, 2026-10-31) and this log.
- Phase 3: done-whens 1 and 2 as first written (0 tile edges with the filter off and on for
  all four scenes; the shadow-only speck class at most today's) were not met; the owner
  amended both on 2026-10-31 and the tests assert the amended bars.
- Phase 3: done-when 4's RTX reading, +12.0 % hit plus light, misses the +10 % guess; the
  Radeon's +7.3 % is inside +20 %.
- Phase 1's box metric definition now carries Phase 2's edge rule as accepted text
  (architect, 2026-10-31); the code already applied it, nothing changed.
- Noticed, not acted on: at both poses the box metric counts 42 % and 46 % of the hits, the
  cube-face parts of ordinary clipped surface voxels with two or three air faces (count 14 or
  more); only the count-under-14 class (0 at the vein, 200 px at the patch) is specific to
  protrusions.
- Noticed, not acted on: under the filter the poses' box pixels rise (vein 35098 to 39263, patch
  38250 to 42080) and the patch's tile edges rise (0.771 to 1.208 %); the groove's box pixels
  rise 697 to 4870 (its side voxels' faces into the groove, hit as cube faces under the floor's
  plane).
- Review fix (Plan 0054's close review, minor): the close triggers filled; committed with this
  line.

### Close triggers

_(Facts for the architect. No recommendations, and no suggested version bump.)_

- **What shipped:** of Phases 2 and 3 nothing survives; the box metric (Phase 1's, with Phase
  2's edge rule) stays in `apps/flythrough/tests/look.rs` as `box_voxel` and `box_reading`, read
  by the field look's tests. The majority-filtered fit, `SurfaceTuning::normal_field`,
  `cube_hits` and `cubes-field` were removed in Plan 0054 Phase 4c (2bca6e4).
- **Public API changed:** none left (every addition of Phases 2 and 3 removed in 2bca6e4).
- **Budget readings:** the Phase 2 and Phase 3 tables in this log, superseded by Plan 0054's;
  none in `docs/nfr.md`.
- **Gate at the tip:** Plan 0054's (this plan closes with it on branch `plan-0054`; that plan's
  close triggers).
- **Outstanding `human` phases:** none (Phase 4 done; Phase 5 subsumed by Plan 0054).

## Followups
