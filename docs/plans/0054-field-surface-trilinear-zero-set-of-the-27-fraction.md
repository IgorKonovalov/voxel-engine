# 0054: The field surface: the trilinear zero set of the 27-fraction, a cost-and-look spike first, then band storage and the walk if the owner picks it

> **Status:** draft
> **Created:** 2026-10-30
> **Milestone:** M2 in [`docs/roadmap.md`](../roadmap.md) (the surface-look decision: "smooth
> and natural rock, not visible cubes")
> **Related ADRs:** [ADR-0011](../adrs/0011-contour-planes-in-attribute-pages-with-procedural-detail.md)
> (the per-voxel plane this surface would replace);
> [ADR-0039](../adrs/0039-the-contour-walk-closes-a-crack-at-the-passed-voxels-exit.md)
> (snap, which a field surface makes moot);
> [ADR-0040](../adrs/0040-one-voxel-features-melt-into-the-contour-surface-by-a-majority-fit-and-a-normal-field.md)
> (whose Alternative B this plan is, and whose Alternative D stays the last fallback). The
> field surface's own ADR (encoding, normal source, what the planes' removal leaves) is
> drafted after Phase 1's readings and accepted or dropped at this plan's close.
> **Source:** the owner at Plan 0050's Phase 5 ("fix it, or if it is unfixable, we will not
> use contours") and the owner's answers of 2026-10-30: the spike runs before Plan 0053's
> windowed check so one session compares every candidate; the storage and cost bars are set
> on Phase 1's readings, the hard line being `nfr.md` §3's 3 GB at `r_full` 16.
> **Starts after:** Plan 0050 Phase 6 and Plan 0053 Phases 1 to 3, stacked on the `plan-0053`
> branch at its Phase 3 tip, so the scratch look sits in the binary the owner compares. Phase 2
> here and Plan 0053's Phase 4 are one windowed session. Phases 3 to 5 run only on the owner's
> pick (a) and then alone on main.

## TL;DR

The per-voxel plane (ADR-0011) is one cube clipped by one plane fit from binary occupancy,
and it cannot represent anything smaller than its own kernel: one-voxel rods, pillars and
lone cubes stay boxes (Plan 0050 Phase 5), two-voxel ridges and concave corners stay steps
even after Plan 0053's filter and normal field. The field surface is the surface those planes
approximate: store at every voxel within one voxel of rock, on both sides, its solid count
over the 27-neighbourhood, define `phi = count / 27 - 0.5`, and hit the ray where the
trilinear interpolation of `phi` over the eight surrounding voxel centres crosses zero. It is
watertight by construction (no cube face, no crack, no closing rule), it melts a one-voxel rod
into a bump a third of a voxel high (worked below), rounds convex edges and fillets concave
corners, and it is what every shipped smooth voxel renderer does, meshed or marched
(`docs/references/voxel-optimizations.md`, ESVO and the isosurface entries). Its price is
field values on air voxels, so air bricks beside rock need a mirror slot, and a cubic solve per
dual cell the ray crosses. Phase 1 builds it as a scratch (dense storage, no size classes) so
the owner sees it in the window beside Plan 0053's looks and the hit pass is measured on both
adapters, and counts the real storage on the level and the flight. If the owner picks it,
Phases 3 to 5 build the band slots, make it the default and remove the planes. The first thing
anyone sees is `--look field` in the fly-through and a test that reads every hit on the flat
floor rig at `z = 64` within a thousandth of a voxel.

## Context & problem

`project-context.md` (Rendering, P0): first person, stable at arm's length, smooth natural
rock. The owner will not ship a contour look with 10 cm boxes on the walls.

**Where the surface stands.** ADR-0011's plane, ADR-0039's snap (Plan 0050: classes 1 and 2
at 0, hit pass +1.0 % RTX, +2.3 % Radeon) and Plan 0053's two changes (the plane fit on a
majority-filtered occupancy, a trilinear normal field over the planes) give a surface whose
remaining faults are geometric: one-voxel features gone or boxed, two-voxel features and
concave corners stepped, shaded smooth. ADR-0040 deferred the field surface for two unmeasured
costs, the air-side storage and the walk, and named it the next plan if the owner's eye
refuses the steps. The owner chose (2026-10-30) to measure it before that check rather than
after.

**What the field is, worked from the formula (not measured; Phase 1 measures it):**

- **A flat floor**, solid below `z = 64`. The top solid layer counts 18 (`phi = +1/6`), the
  air layer above counts 9 (`phi = -1/6`); the zero crossing is their midpoint, `z = 64`,
  exactly the cube face. Flat walls do not move. The same holds for every axis-aligned face.
- **A rod on the floor** (one voxel high and wide, along x). The rod voxel counts 12
  (`phi = -0.056`), as do the air voxels beside it in its layer; the floor voxels under and
  within one voxel of it count 21 (`+0.278`). The crossing between the floor layer and the
  rod's layer sits at `0.278 / 0.334 = 0.83` of the way up: a bump 0.33 voxel high and three
  voxels wide, with no vertical face. A lone cube (count 10, floor under it 19) is a bump 0.11
  voxel high. Plan 0053's filter flattens both completely; the field keeps a hint of them.
- **A convex 90-degree edge voxel** counts 12 (`-0.056`): its centre lies just outside, the
  edge is rounded over about one voxel. **A concave corner's air voxel** counts 15 (`+0.056`):
  it is filleted. A one-voxel groove's bottom voxel counts 24 and its air cell 15: the groove
  is a shallow trough, not a slot with a flat floor.
- **A free one-voxel pillar or sheet** has centres at `-0.056` with air at `-1/6` or lower
  around them: the pillar's rows above the base thin to a bump at the base and vanish above
  it. The hazard ADR-0040 names for the filter is the field's too, in a different shape.

**Why the blend rule's cost is not this plan's cost.** Plan 0050's `blend` was a trilinear
field over plane distances evaluated by two secant steps per voxel stepped, with eight
`lookup`s and eight attribute decodes per evaluation: 5.5x and 8.2x the hit pass at 128 VGPRs.
This plan stores a scalar per voxel (a `u8` count), evaluates once per dual cell crossed with
the cubic solved analytically, and reads the band from a mask, so the per-cell work is eight
byte reads and one cubic. That it lands near ADR-0040's guess of 1.3x to 2x is the thing
Phase 1 reads; nothing here assumes it.

**The `nfr.md` figures touched:** §2 the hit pass (snap alone: RTX 3080 Laptop 0.604 ms,
Radeon RENOIR 4.364 ms at the level's spawn, 1080p from 720p), the light pass (0.913 and
7.026 ms), the attribute pass (the level's fill 17.0 ms under the weighted fit, dev box; the
flight's p50 and p99); §3 the mirror (157 872 640 B live and 259 203 264 B allocated at the
level; the flight's full residency 871 679 852 B live of 3 221 014 080 B allocated at `r_full`
16, under the 3 GB line).

## Decision

Measure first, in the owner's binary. Phase 1 builds the field surface as a scratch look
(`--look field`): a dense field pool beside the mirror (nine units per band brick, a direct
index, no size classes), the attribute pass's own occupancy shell producing the counts for
every band brick including the air and uniform-solid bricks beside rock, and a walk that tests
dual cells from a band mask and solves the trilinear crossing per cell. It reads the hit and
light passes on both adapters, the box, tile-contrast and guardrail metrics of Plans 0039 and
0053, the flat-rig exactness, a leak count, two normal sources, and a CPU census of the band's
real storage packed and dense. Phase 2 is the owner's windowed check, the same session as
Plan 0053's Phase 4, with the field surface as a sixth look. On pick (a), Phase 3 builds the
band slots in the size-classed mirror (a new slot class for air-side bricks, counts packed by
popcount, the sync and the pressure rule aware of them), Phase 4 makes the field the default
walk with the normal source Phase 1 picked and removes the planes, snap and Plan 0053's
losers, and Phase 5 lands the readings and the ADR.

We rejected finishing on the plane (Plan 0053 alone) as the only path because its ceiling is
now known and the owner wants the alternative measured before judging. We rejected a tricubic
or wider reconstruction for the surface itself because the 27-count is already a box filter
and the trilinear zero set of it is C0 across cells, which is all watertightness needs; the
normal is where smoothness is read and Phase 1 compares two sources for it. We rejected
dilation (ADR-0040 Alternative C) because the field fillets concave corners by itself. We
keep `cubes-field` (Alternative D) as the last fallback, built by Plan 0053.

**The analytic window rule** of Plan 0050 holds: analytic bars count hits at least 2 voxels
inside the mirror window and print what they left out.

## Architecture diagram

```mermaid
flowchart LR
    subgraph vxe-render
        O[attrib.wgsl: occupancy, brick + shell] --> C[count per band voxel, solid and air]
        C --> S[band slot: band mask + u8 counts, packed by popcount]
        S --> W[march.wgsl: DDA to a band voxel, dual cells crossed]
        W --> Z[trilinear phi along the ray, cubic root in the cell]
        Z --> H[hit record: position, field normal]
        H --> L[light.wgsl: shade, shadow origin along the field normal]
    end
    subgraph vxe-world
        M[brick kinds: full, uniform solid, uniform air] -.->|which bricks are band| S
    end
    subgraph flythrough tests
        H --> T[flat-rig exactness, leaks, box and tile metrics, guardrails, cost]
        M --> N[CPU census: band voxels and bricks, bytes packed and dense]
    end
```

## Implementation phases

Each phase ships as its own commit. `dev` implements Phase 1 in the stacked lane, stops for
the owner's Phase 2, and implements Phases 3 to 5 in one session on main after pick (a). The
architect reviews once at the end, in a fresh session. Lane rule 9c applies: exploratory
sweeps may run on the RTX 3080 Laptop through `VXE_TEST_ADAPTER`, labelled with the adapter;
every asserted reading, golden and gate stays on lavapipe.

### Phase 1: The field as a scratch look: dense storage, the walk, the readings and the census
- **Owner skill:** dev
- **What:** `LookPreset::FIELD` (`--look field`, the banner names it) renders the field
  surface from a scratch pool, with every reading below. No default changes; the plane looks
  are untouched byte for byte with the field off.
- **The band.** A voxel is in the band when its 27-count is neither 0 nor 27. A brick is a
  band brick when it holds a band voxel: every Full brick with a surface voxel, every
  uniform-solid brick with an air voxel within one voxel of it, every uniform-air brick with a
  solid voxel within one voxel of it. The field's occupancy is the attribute pass's occupancy
  today (dev states in the log whether translucent voxels count as solid or air there, and
  keeps it).
- **The scratch pool.** A separate storage buffer: per band brick in the mirror window, nine
  units (one band mask of 512 bits, eight units of 512 `u8` counts in voxel order), indexed by
  a per-record table of 512 `u32` parallel to `chunk_records` (an index or `ABSENT`). Filled
  by the attribute pass from its 10-cube occupancy shell (the counts of the brick's own 512
  voxels fall out of it), with the work list extended to the air and uniform-solid band bricks
  the 26-neighbour reach already visits. Allocation is a bump allocator over the window,
  rebuilt on a window move; it is a scratch and the log says so.
- **The walk.** The DDA runs as today to the first band voxel the ray enters (the band mask
  read where the opaque mask is read now; a band-air brick has a zero opaque mask and a slot
  index). Inside a band voxel the ray's segment is split at the voxel's three mid-planes into
  the dual cells it crosses (at most four), and in each cell whose eight corner `phi` values
  mix signs the trilinear `phi` along the segment is a cubic in `t`, solved by Marmitt et
  al.'s root isolation (`docs/references/voxel-optimizations.md`, the isosurface entry) or by
  bracketing the sign change and two Newton steps, dev's call under the cost bar; the first
  root on the segment is the hit. A cell whose corners are all solid is a hit at the segment's
  start (the ray is inside rock, which only happens where the band is incomplete; counted as
  a leak). Rays that leave the band into a solid voxel of count 27 without a hit are leaks,
  counted per frame in the cracks view's place.
- **The normal, two sources read:** N1, the analytic gradient of the trilinear `phi` at the
  hit; N2, the trilinear blend at the hit of the eight corner voxels' 27-gradients (Plan 0053
  Phase 3's field, read through the plane attribute where one exists; for an air corner the
  gradient of the occupancy around it, computed in the attribute pass into the scratch pool's
  spare bits or a tenth unit). N1 costs nothing to store and is discontinuous across cell
  faces; N2 is smooth and costs a normal per band voxel. Both are selectable in the scratch
  (`--field-normal analytic|blend`) and read on the tile-contrast metric.
- **The readings** (lavapipe for every asserted figure, both adapters for cost, release, perf
  lock, three alternated runs against the lane's base binary):
  - the flat rigs: `floor` and `wall`, every hit's distance from the plane of the voxel face;
  - the six one-voxel scenes of Plan 0053 and its tilted, 45-degree and diagonal slabs, the
    cavity and the pillar: box pixels (by Plan 0053's metric), tile-contrast share under N1
    and N2, leaks, hit count against the plane look;
  - Plan 0039's guardrails: cavity normal error, pillar silhouette (face 172 px, contour
    32 px), `A`; the speck test;
  - `VEIN_POSE` and `PATCH_POSE` at 384 x 216, lamp on and off: box pixels, tile contrast,
    leaks, side-by-sides under `live/field/` (shaded, normals view, the plane look beside);
  - cost: the hit and light passes at the level's spawn, 1080p from 720p, field against the
    lane's base look, RTX 3080 Laptop and Radeon RENOIR, with RADV shader stats (VGPRs) for
    the hit shader; the attribute pass's initial fill and the autopilot's p50 and p99, both
    adapters;
  - the census (CPU, headless, over the world through `vxe-world`'s public queries): band
    voxels solid-side and air-side, band bricks by kind (Full, uniform solid, uniform air),
    on the level at `--seed 1` and over the streamed flight's full residency; the bytes
    packed (one mask unit plus `ceil(n / 64)` units of `u8` per band brick, plus two header
    units for a brick that has no slot today) and dense (nine units per band brick), against
    today's attribute bytes and the live totals of `nfr.md` §3.
- **Files touched:** `crates/vxe-render/src/shaders/attrib.wgsl` (the counts, the air and
  uniform-solid work items), `march.wgsl` (the band read, the dual cells, the cubic, N1 and
  N2), `light.wgsl` (the shadow origin along the field normal), `composite.wgsl` (the normals
  view, the leak count in the cracks view's place), `crates/vxe-render/src/marcher.rs` and
  `gpu_world.rs` (the scratch pool, the index table, the work list), `lib.rs`,
  `apps/flythrough/src/lib.rs` (`LookPreset::FIELD`), `main.rs`, `hud.rs` (the flags, the
  banner), `apps/flythrough/tests/look.rs`, `sharpness.rs`, `level_mirror.rs` (the census),
  `tests/support/rigs.rs`, this plan's log.
- **Done when:**
  1. `the_field_surface_lies_on_the_flat_faces` asserts on lavapipe, under the analytic
     window rule, that every hit on `floor` lies within 1e-3 voxel of `z = 64` and every hit
     on `wall` within 1e-3 of its face plane, and that both read 0 leaks; the hit counts are
     printed beside the plane look's.
  2. `the_field_surface_has_no_boxes_and_no_leaks` asserts 0 box pixels on `rod`, `lone
     cube`, `groove`, `wall pillar` and `ridge`, 0 leaks on every analytic scene and at both
     poses, and prints the tile-contrast share under N1 and N2 for every scene. If any scene
     reads leaks, dev stops for the architect: the band definition or the cell test is wrong.
  3. `field_readings` (ignored, a reading) prints the full table: every scene and pose, N1
     and N2, box pixels by count class, tile contrast, leaks, guardrails, the speck test, hit
     counts against the plane look; the log carries it and names, per guardrail, whether the
     field reads within Plan 0039's bars (cavity normal error at most 12 degrees, silhouette
     at most half of face mode's, `A` at most 10 %).
  4. The cost table is in the log: hit and light passes on both adapters, field against base,
     VGPRs; attribute pass fill and flight p50 and p99 on both adapters. No bar: the owner
     sets it at Phase 2 on these numbers.
  5. `band_census` (a test, asserted on the level) prints the census and asserts the
     invariant that every Full brick with at least one surface voxel is a band brick, and that
     the GPU scratch pool's band-brick count at the level's spawn equals the CPU census's for
     the mirror window. The packed and dense byte totals, level and flight, are in the log
     against today's figures. No bar: the owner sets it at Phase 2.
  6. With the field off, every golden reads the same pixels as the lane's base (29 of 29 or
     the count at the base), and `mirror_bytes` and `allocated_bytes` equal the base's: the
     scratch pool is outside the mirror's accounting and the log says its bytes.
  7. The side-by-sides exist for both poses, and the log says in one line per pose what the
     field does to the owner's rods, pillars and lone cubes of Plan 0050's Phase 5 shots and
     to the ledge steps.

### Phase 2: The owner's windowed check, one session with Plan 0053's Phase 4
- **Owner skill:** human
- **What:** the owner runs the release fly-through at `--pose vein` and `--pose patch`, lamp
  on and off, shaded and normals views, in six looks: Plan 0053's four (today's snap,
  `--fit-filter majority`, `--normal-field on`, both), `--look cubes-field`, and `--look
  field` under N1 and N2, with the cost and census tables of Phase 1 in hand.
- **Files touched:** this plan's log and Plan 0053's.
- **Done when:** the owner records one of: (a) the field surface is the pick; the owner sets
  the storage bar (live bytes at the level and the flight, against `nfr.md` §3's 3 GB at
  `r_full` 16) and the cost bar (the hit plus light passes on the Radeon, and on the RTX),
  and names the normal source or leaves it to Phase 1's tile-contrast reading; Phases 3 to 5
  run. (b) a Plan 0053 look is the pick; this plan is abandoned, its Phase 1 commit is not
  merged, and the log says what the field read so the next reader need not rebuild it. (c)
  `cubes-field` is the pick; as (b), and Plan 0053's Phase 5 lands it.

### Phase 3: Band slots in the size-classed mirror
- **Owner skill:** dev
- **What:** the field's storage as a mirror slot, replacing the scratch pool. A band brick's
  slot carries, after its existing units, one band-mask unit and `ceil(n / 64)` units of `u8`
  counts in band-index order (prefix per layer in the header, as the surface prefix is today);
  a uniform-solid or uniform-air band brick gets a slot of a new class (opaque mask as today's
  rule for the kind, header, band mask, counts; no payload) and its record entry's bit 31 and
  class, so the walk enters it. The CPU side (`gpu_world.rs` sync) decides band membership
  from the world's brick kinds and the neighbour masks it already reads for the surface mask,
  reserves the units within the slot bins, and the pressure rule (Plan 0017) counts them. An
  edit that exposes rock to an air brick without a slot allocates one through the same path
  as a brick turning Full. The attribute pass fills the counts (and N2's normals if the owner
  picked N2) for every band brick in the work list.
- **Files touched:** `crates/vxe-render/src/gpu_world.rs` (the class, the units, the sync,
  the pressure count), `src/shaders/attrib.wgsl` and `march.wgsl` (the slot layout read),
  `crates/vxe-render/tests/mirror.rs` (slot layout pins), `apps/flythrough/tests/level_mirror.rs`
  (the bytes), this plan's log.
- **Done when:**
  1. A mirror test pins a band-air brick's slot bit-exact for a one-brick world beside a solid
     brick: its class, its band mask (the face layer's 64 bits set), its first count unit's
     first bytes (9 for a face-adjacent air voxel on a flat face, 6 at an edge, 4 at a corner).
  2. `mirror_bytes` at the level reads the census's packed figure plus the header units,
     within 1 %, and the flight's full residency and the level's live bytes are within the
     owner's storage bar; `allocated_bytes` at `r_full` 16 stays under 3 GB.
  3. The field look's hit mask and normals view at both poses are byte for byte Phase 1's
     (the storage moved, the surface did not).
  4. An edit test: a sphere carve that opens a uniform-solid brick to air gives the exposed
     air brick a band slot within the frame of the edit's sync, and the field look renders the
     new hole with 0 leaks on lavapipe.
  5. The attribute pass's cost under the autopilot flight on both adapters, three alternated
     runs against the Plan 0053 close binary; the reading is in the log against Phase 1's.

### Phase 4: The field as the default walk, the planes removed
- **Owner skill:** dev
- **What:** the contour looks become field looks: the walk's hit test is the dual-cell
  crossing with the owner's normal source, the shadow origin steps along the field normal,
  the detail tilt and Plan 0039's lamp ratio read the field normal. The plane attribute, its
  fit settings (`PlaneFit`, `PlaneOffset`, `OccupancyFilter`), `CrackClosing` and snap, the
  cracks view's classes, `SurfaceTuning::normal_field` and the losing looks of Plan 0053 are
  removed as Plan 0048's rule says, unless the owner keeps one for comparison at Phase 2. The
  `u16` attribute goes; N2, if picked, stores its normal as an octahedral `u16` per band voxel
  in the same units. `cubes-field` stays until the close as the comparison look and goes with
  the ADR's acceptance unless the owner keeps it. The LOD ring's mip cells stay cubes, as the
  planes stopped at the full radius today.
- **Files touched:** `crates/vxe-render/src/shaders/march.wgsl`, `attrib.wgsl`, `light.wgsl`,
  `composite.wgsl`, `crates/vxe-render/src/marcher.rs`, `gpu_world.rs`, `lib.rs`,
  `apps/flythrough/src/lib.rs`, `main.rs`, `hud.rs`, `apps/flythrough/tests/look.rs`,
  `sharpness.rs`, `golden.rs` and the moved goldens, `crates/vxe-render/tests/mirror.rs`,
  `walk.rs`, this plan's log.
- **Done when:**
  1. `the_default_surface_is_the_field` asserts on lavapipe at the default: the flat-rig
     exactness of Phase 1's done-when 1, 0 box pixels on the five one-voxel scenes, 0 leaks on
     every scene and at both poses, the tile-contrast share at most Phase 1's reading for the
     picked normal source, and Plan 0039's guardrails within their bars.
  2. Every moved golden is named in the log with its share of pixels over 2/255; the face
     goldens did not move (Plan 0050 owner answer 2).
  3. The hit and light passes at the level's spawn on both adapters, three alternated runs
     against the Plan 0053 close binary, within the owner's cost bar; the VGPR counts of the
     hit and light shaders are in the log.
  4. No removed setting's flag remains in `--help` or the banner; `cargo doc` has no mention
     of a plane attribute outside the ADR citations.

### Phase 5: The readings, the ADR, the close block
- **Owner skill:** dev
- **What:** `nfr.md` §2 and §3 carry the field's readings with machine, commit and load; the
  plan's log carries the close block. The field surface's ADR (drafted by the architect after
  Phase 1, number from the index) is accepted at the close; ADR-0011 and ADR-0040 are marked
  superseded by it, ADR-0039 superseded as moot (no cube face, no crack).
- **Files touched:** `docs/nfr.md`, this plan's log.
- **Done when:** the close block names every reading above; the full tier, clippy, fmt,
  doctests, deny and the doc gates are clean at the tip, with the full tier's wall time and
  load recorded.

## Data shapes

```rust
// illustrative: vxe-render
pub enum SurfaceModel { Planes, Field }        // Phase 1: a look's choice; Phase 4: Field alone
pub enum FieldNormal { Analytic, Blend }       // N1, N2; Phase 4 keeps the pick
pub struct SurfaceTuning {
    pub model: SurfaceModel,                   // new
    pub field_normal: FieldNormal,             // new
    pub face_mix: f32,                         // unchanged
    // Plan 0050's `closing` and `shadow_push`, Plan 0053's `fit` and `normal_field`:
    // present through Phase 3, removed in Phase 4 on pick (a)
}
// A band brick's slot (Phase 3), after today's units:
//   one band-mask unit (bit i: voxel i has 0 < count < 27),
//   ceil(n / 64) units of u8 counts in band-index order (prefix per layer in the header),
//   and, under N2, ceil(n / 32) units of octahedral u16 normals.
// A uniform band brick gets a slot of class 6 (illustrative): opaque mask, header, band
// mask, counts; no payload. Its record entry carries bit 31 and the class, as Full bricks do.
```

The CPU world, its edits and its results do not change. Determinism is untouched: everything
is in `vxe-render`, a client-side view.

## Budgets

| `nfr.md` figure | Base (read) | Expected | How the close measures it |
|---|---|---|---|
| §2 the hit pass and the light pass | level spawn, 1080p from 720p, snap alone: RTX 0.604 + 0.913 ms, Radeon 4.364 + 7.026 ms (Plan 0050) | **a guess** of 1.3x to 2x on the hit pass (ADR-0040), one cubic per dual cell crossed and eight byte reads, VGPRs near snap alone's 64; light unchanged but for the normal | `flythrough --level --seed 1 --headless --render 1920x1080 --frames 300 --adapter NVIDIA\|AMD` per look against the Plan 0053 close binary, alternated, release, perf lock, memory cap, three quiet runs; bar set by the owner at Phase 2 |
| §2 the attribute pass | the level's fill 17.0 ms (weighted fit, dev box, Plan 0039); the flight's p50 and p99 of Plan 0028 and 0039 | **a guess** of 1.5x to 2.5x the work items (the air and uniform-solid band bricks), each cheaper (counts, no fit) | the initial fill and `--autopilot --headless --seed 1` p50 and p99 on both adapters, three alternated runs |
| §3 mirror bytes | 157 872 640 B live, 259 203 264 B allocated at the level; 871 679 852 B live of 3 221 014 080 B allocated over the flight (Plan 0039) | the band's bytes replace the attribute's: **a guess** of 1.5x to 3x today's attribute bytes, the air side the larger part; the census of Phase 1 replaces the guess | `mirror_bytes` and `allocated_bytes` at the level and over the flight, under the owner's bar; `r_full` 16 under 3 GB is the hard line |

## Risks & open questions

- **The band's air side is bigger than the solid side in a cave.** Concave caverns have more
  air next to rock than rock next to air, and every uniform-air brick touching rock gets a
  slot. The census is the answer, and it comes before any slot code is written. If the owner's
  bar fails at the level, the fallback inside this plan is a two-byte-per-brick-face scheme
  (counts for a face layer only) or `r_full` 14, both the owner's call; outside it, Plan 0053's
  pick stands.
- **Trilinear normals crease.** N1's gradient is discontinuous across dual cells and may show
  a faint tiling under the lamp at arm's length. N2 removes it at a `u16` per band voxel.
  Phase 1's tile-contrast metric reads both; the owner sees both.
- **The cubic's precision.** `f32` along a ray at the level's extent (`2^24` voxels is Plan
  0052's followup line) and a near-tangent crossing can miss or double a root. Bracketing the
  sign change before the Newton steps bounds it; the leak count is the gate.
- **Mixed-sign cells with no crossing on the segment** (the ray passes a cell whose corners
  mix signs without touching the surface) are the common case along a wall: the cubic must
  reject them cheaply. Marmitt's extremum test does; a miss here is the cost risk.
- **The walk enters more bricks.** Band-air bricks have slots, so the hierarchy skips less
  air near rock. The hit pass reading carries it; the mask-level skips inside a band-air brick
  (its opaque mask is zero) keep the cost to the band voxels themselves.
- **Collision keeps the voxels.** The visible surface lies up to half a voxel inside a convex
  edge and outside a concave corner. At a 30 cm capsule and 10 cm voxels the player slides
  over it; a consumer wanting the look and the collision to agree gets a followup, not this
  plan.
- **Deliberate one-voxel detail in prefabs** becomes a bump. A consumer who wants 10 cm
  detail visible models it two voxels thick, as under Plan 0053's filter.
- **The attribute pass under edits** now touches air bricks. Plan 0023's hitch budget is on
  the render thread's CPU, which the GPU pass does not load; the sync's new slot allocations
  for exposed air bricks do, and Phase 3's done-when 4 reads it.

## What this plan does NOT do

- Change `vxe-world`, its edits, results, queries or collision. The field is a render-side
  view of the voxels.
- Give the LOD ring's mip cells a field, or a transition between the field and the cubes at
  the full radius beyond what the planes have today.
- A tricubic or filtered surface, dilation, or per-material opt-outs.
- The lighting: Plan 0051 is the light spike; this plan changes only which normal shades.
- Decide the look. Phase 2 is the owner's, and (b) and (c) are real outcomes.

## Implementation log

> Written by `dev`: one row per phase as its commit lands, then the close block. **The phases
> above are the contract. This section records what happened.** Observations, never conclusions:
> no pass list, no self-assessment. Deviations and unmet done-whens are **always** disclosed.
> Keep it shorter than `## Implementation phases`.

| phase | owner | state | commit |
|---|---|---|---|
| 1: The field as a scratch look: dense storage, the walk, the readings and the census | dev | not started | |
| 2: The owner's windowed check, one session with Plan 0053's Phase 4 | human | not started | |
| 3: Band slots in the size-classed mirror | dev | not started | |
| 4: The field as the default walk, the planes removed | dev | not started | |
| 5: The readings, the ADR, the close block | dev | not started | |

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
