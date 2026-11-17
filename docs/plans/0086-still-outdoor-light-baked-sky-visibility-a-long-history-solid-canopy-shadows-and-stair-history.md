# 0086: Still outdoor light: a sky visibility baked per brick, a long history for the sky and key rays, canopies that shadow as bricks, stairs that keep their history, and a penumbra spike

> **Status:** in-progress (the owner's go, 2026-11-17; it runs before Plan 0081, and Phase 6 is
> conditional on its reading as drafted)
> **Created:** 2026-11-17
> **Milestone:** M10: Outdoors in [`docs/roadmap.md`](../roadmap.md)
> **Related ADRs:** [ADR-0070](../adrs/0070-sky-light-is-baked-visibility-coloured-at-read-with-a-long-history-ray-where-stale.md)
> (proposed with this plan: the sky visibility baked per brick direction, coloured at read; the
> ray on a long history where the bake is stale),
> [ADR-0027](../adrs/0027-the-sun-shadow-is-one-per-pixel-ray-in-the-light-pass.md) (the key
> light's ray; its Outcome of 2026-11-17 and its Alternative F, taken here for the rays),
> [ADR-0046](../adrs/0046-static-emissive-light-is-baked-per-chunk-when-a-chunk-loads-or-changes.md)
> (the store and the queue that gain the channel; its Outcome of 2026-11-17),
> [ADR-0042](../adrs/0042-the-far-shading-normal-blends-to-the-27-count-gradient-by-the-voxels-size-on-screen.md)
> (the far normal whose regime Phase 3 uses),
> [ADR-0018](../adrs/0018-direct-light-per-pixel-one-sample-from-a-hit-buffer-with-svgf-history-killed-by-edits.md)
> (the temporal pass's reprojection and resets),
> [ADR-0063](../adrs/0063-surface-detail-is-a-procedural-shell-marched-at-the-hit-described-per-material-on-the-render-side.md)
> (Plan 0081's render-side style table, where Phase 2's flag lives once both have landed)
> **Source:** the owner's Phase 9 window of Plan 0032 (2026-11-17; the verdict verbatim in
> `docs/plans/logs/0032.md`), the diagnosis and options in
> [the review of 2026-11-17](../reviews/2026-11-17-plan-0032-window.md), and the owner's
> picks of the same day: C1 and C2 both, D1 to D4 all four, each behind the attribution
> reading the review names; stable image first, performance considered.
> **Starts after:** [Plan 0032](done/0032-sun-and-sky-hillaire-atmosphere-time-of-day-and-an-outdoor-testbed.md)
> closes and merges (its Phase 10 makes an absent chunk sky, its Phase 11 sets the defaults
> this plan's readings stand on). The architect recommends it ahead of Plan 0081 (the owner's
> priority is the still image; Phase 2 names the merge item if 0081 runs beside).

## TL;DR

The outdoor light holds still. The sky's light at a face comes from a visibility baked per
brick direction into the per-chunk store, coloured by the sky each frame, so the tunnel's rim
and far end read one stored number instead of a 0-or-full sample; the per-frame sky ray is
cast only where the store has no fresh value, on a running mean of its own with a firefly
clamp, so a dug skylight still lights the chamber on the next frame. A canopy shadows as its
brick, so a forest casts a blob and no flecks. Where a voxel is smaller than the far normal's
reach, the temporal pass keeps history across a stair and the key ray starts above it, so the
contour speckle bands go. The key light's visibility may ride the same long history, after a
reading of the 3-degree penumbra's grain. A spike reads a deterministic penumbra from the
cell hierarchy's closest approach and stops. Phase 1 is a reading that splits the flicker by
term at the tunnel and the forest, so each fix is attributed before it is built. The first
thing a reader sees is Phase 1's table; in the window, the tunnel at noon without sparkle.

## Context & problem

Plan 0032's Phase 9 window (2026-11-17): "very much flicker on tunnel edge", "towards the end
of tunnel it looks super flickery", "a lot of flicker in tree shadows, some simplification is
needed", "when light settles and trees are coarse shadows look amazing". The review of the
same day read every complaint but the sun's size to three mechanisms, each from the code and
the readings:

- **One binary sample on an eleven-frame blend.** The sky ray and the key light's shadow ray
  each add 0 or the full value to the direct target, blended at `max(0.2, 1 / (age + 1))`
  (`temporal.wgsl`). At the tunnel's rim the sky ray's hit rate is about 0.3 to 0.6 (a
  standard deviation of about a sixth of the full value on the blend); at the far end it is a
  few percent, so a rare hit adds thirty times the mean and decays over three or four frames.
  Along the voxel staircase's contours the temporal pass rejects history where the tread and
  the riser alternate under FSR 2's jitter (`HISTORY_COS` 0.8, the material test), and a
  rejected pixel shows the raw sample; the a-trous leaves it alone because its moments are
  fresh there.
- **Sun flecks through the canopy.** The coarse shadow walk treats a 2^3 block with any set
  bit as solid; at `LEAF_FILL` 64 a tenth of a canopy's blocks are empty (`0.75^8`), so every
  tree casts hundreds of 2-voxel sun flecks that move with the sun and the jitter. The LOD
  ring's mip-1 trees are solid under the same rule, which is the look the owner called amazing.
- **The 3-degree penumbra.** The owner's pick widens every penumbra (8.4 voxels under a canopy
  80 voxels up) and a single disc ray is noisiest at its middle (standard deviation 0.17 of the
  full value at `p` 0.5).

Plan 0032 Phase 6's stop rule did not see the first: its rig was an open floor under a uniform
sky, where every ray reaches the sky and the variance is 0. Restated on the tunnel (Plan
0032's amendment of 2026-11-17) it fires, and this plan is its amendment. A longer history
alone cannot carry the far end: a running mean of `N` frames of a Bernoulli at hit rate `p`
has a relative standard deviation `sqrt((1 - p) / (p N))`, 0.71 at `p` 0.03 and `N` 64. The
far end is where the geometry holds still and a stored value is right.

The lighting base: ADR-0046's store bakes six direction values per surface brick when a chunk's
reach is resident, re-bakes in rings at an edit, reads bilinear with no screen history; a
still face read from it does not flicker (Plan 0059). Costs read (Plan 0032's readings log,
Phases 6 to 8; `docs/nfr/02-frame-budget.md`): the sky ray about 0.25 ms of the RTX 3080
Laptop's light pass at 1080p and about 4.8 ms of the RADV RENOIR's, per ray; the RENOIR's
outdoor light pass 10.5 ms (12.5 in the forest) against section 2's 6.0 ms line; the key ray's
steps p99 79 to 92 in the forest. `nfr.md` figures touched: section 2 Lighting (the light
pass, the temporal pass), section 3 (the store's record, a target pair), section 4 (a dig's
re-bake with the channel), the streaming line's bake-queue step.

## Decision

We build ADR-0070 and the three shadow fixes, each behind a reading, in this order: the
attribution reading; the canopy flag and the stair history (the cheap, direct ones); the
baked visibility; the rays' long history; the key light on it; the penumbra spike; the window;
the tests. **Visibility is geometry and radiance is the sky:** the store gains a `u8` per
brick direction, the cosine-weighted fraction of the hemisphere that reaches the sky, baked
by the emissive job while a `Sky` is set and re-baked in its rings; a face reads it bilinear
and multiplies by the sky's cosine-weighted irradiance about its normal, six values a frame
from the sky-view LUT. The ray estimates the same visibility where the bake is stale or
absent, on a pair of the lighting grid with a running mean to 64 frames, a firefly clamp and a
variance reset; at rest inside the bake radius no sky ray is cast, and `sky_rays` 0 is the
bake-only setting. A render-side per-material style flag makes the shadow walk stop at the
first brick of that material with any set bit, set on the testbed's `leaf`. In the far-normal
regime the temporal pass tests the smoothed normal and drops the material test for the
irradiance history, and the key and sky rays start from the hit lifted along the smoothed
normal, with a reading that a thin far wall leaks under a pixel. The key light's visibility
moves to the pair only if the penumbra's grain, read after the first two fixes, is over the
lamp's. The penumbra spike builds a setting, reads it and promises nothing.

We rejected the long history alone, more rays where the neighbourhood disagrees, the probes,
an unoccluded hemisphere, baking the sky's radiance, `L1` visibility, the ray blended with
the bake everywhere and per-face sky cells (ADR-0070); a 4-voxel shadow walk everywhere (the
review's D5: the Lego look the owner refuses); and two radii per body (A2: the fallback if the
penumbra's grain survives this plan).

## Architecture diagram

```mermaid
flowchart LR
    subgraph render [vxe-render]
        Q[bake queue\nper chunk, rings at a dig] --> ST[store record\nsix emissive values\n+ six sky visibilities u8]
        LUT[sky-view LUT] --> ES[E_sky: six axis\nirradiances a frame]
        ST --> T[temporal pass\nv(n) x E_sky(n), no history]
        ES --> T
        H[hit] --> LP[light pass\nsky ray only where stale\nkey ray, disc sampled]
        LP --> P[visibility pair\nrunning mean to 64\nfirefly clamp, variance reset]
        P --> T
        ES --> LP
        F[material style flag\nshadow as brick] --> LP
        T --> C[composite]
    end
```

## Interface for Plans 0033 to 0037

Changes to Plan 0032's Interface that later outdoor plans rely on; the rest of that section
stands.

- **The sky term at a face** is `v(n) E_sky(n)`: `v` the store's bilinear visibility read
  (or the pair's running mean where the store is stale), `E_sky(n)` the cosine-weighted sky
  irradiance about `n` from the sky uniform's six axis values. A plan that changes the sky's
  radiance (clouds, an aurora's glow, a night tint) changes `E_sky` by changing the sky-view
  LUT; it bakes nothing and re-bakes nothing. A plan that occludes the sky with geometry the
  mirror does not hold (Plan 0033's far field, Plan 0035's clouds) multiplies `v` at the one
  place the temporal pass reads it, or leaves it.
- **The visibility pair** holds the sky ray's and (if Phase 6 takes it) the key light's
  visibility on a running mean to `SKY_HISTORY_CAP` frames, reset by the direct
  accumulation's tests and by the variance reset. A later term that changes the key light's
  visibility frame to frame (a cloud shadow) either stays under the reset's band or sets the
  sky-change bit on the frame it changes; it does not write the pair.
- **The style flag** `SHADOW_AS_BRICK` is a bit of the render-side per-material style word
  (Plan 0081's table, or this plan's word that 0081 absorbs). A consumer sets it per material
  through `Frame::set_material_styles` (the name Plan 0081 gives, or this plan's).
- **Settings** (Plan 0069's surface): `lighting.sky_rays` accepts 0 (bake only);
  `lighting.sky_history` (the cap), `lighting.leaf_shadow` is the testbed's A/B, not a
  setting; `lighting.penumbra approach|disc` from the spike, `disc` the default.

## Implementation phases

Each phase ships as its own commit. `dev` implements all phases in one session with no review
between phases. The architect reviews once at the end, in a fresh session. Phases 1 to 7 are
**exploration** (lane rules 2b): the owner's window in Phase 8 is their gate. Each lands its
CPU tests of data, determinism and API; every bar that judges the image runs as a reading from
a `mod reading`, recorded in the readings log, and Phase 9 writes it as a test against the
picks. The readings log is `docs/plans/logs/0086.md`. Per phase the gpu check runs only the
`gpu_*` binaries the phase names; a named test over 10 s on lavapipe is born
`#[ignore = "soak"]`. Every wall time below is an estimate.

**The rigs.** Two poses from Plan 0032's testbed, used throughout: the **tunnel** (`--source
outdoor --spawn inside --time 12:00`, the lamp off, seed 1) with two regions the Phase 1
reading pins by pixel rectangle at 320 x 180, the mouth's **rim band** (the sky ray's hit rate
about 0.3 to 0.6) and the **far end** (under 0.05); and the **forest** (`--spawn forest
--trees forest --sun 60,180`, `LEAF_FILL` 64) with the stepped ground's shadow edges. The
statistics are `apps/flythrough/tests/support/stillness.rs`'s (patches a step, sudden
changes a second, flicker in log2 a frame, over 64 still frames after 64 settle frames) and
Plan 0032 Phase 6's (the 99th-percentile frame-to-frame change in display levels), each
beside the headlamp on the same pose with the sky off, the reference the owner accepted.

### Phase 1: The attribution reading: the flicker split by term at the tunnel and the forest
- **Owner skill:** dev
- **What:** `reading::flicker_by_term` in `apps/flythrough/tests/gpu_outdoor.rs` runs both
  rigs through a matrix and prints one table: the term (the key light alone, by `sky_rays` 0;
  the sky ray alone, by a test hook that zeroes the key light's sample, listed in
  `NOT_SETTINGS`; both), the filter (off, on), the history (the temporal pass as it is; a hook
  that accepts history whatever the normal and material, to read how much the rejection
  contributes). Beside the statistics: per region, the sky ray's hit rate (the share of
  frames the sample is non-zero at the region's pixels) and the firefly rate (samples over
  four times the region's mean); at the forest, the share of lighting pixels whose history
  was rejected per frame, split by cause (normal, material, position), from a counter the
  temporal pass writes behind the steps view. FSR 2 at 1.5 and native both.
- **Files touched:** `apps/flythrough/tests/gpu_outdoor.rs`, `tests/support/stillness.rs`,
  `crates/vxe-render/src/shaders/temporal.wgsl` (the counter and the hooks),
  `frame/settings.rs` (the hooks, `NOT_SETTINGS`), `docs/plans/logs/0086.md`.
- **Done when:**
  1. The log holds the table, both poses, every cell, and names per pose the dominant term
     and the share the history rejection carries (a reading; no bar).
  2. The far end's hit rate and the rim's are read; `SKY_HISTORY_CAP` and `FIREFLY_K` in
     Phase 5 are set from them by the arithmetic in ADR-0070 (the log shows the step).
  3. The hooks and the counter cost nothing when off: the default frame's shaders are
     byte-identical in their WGSL output or the counter is a uniform branch (the log says
     which).

### Phase 2: A canopy shadows as its brick (the review's D1)
- **Owner skill:** dev
- **What:** a render-side per-material style word with a `SHADOW_AS_BRICK` bit. If Plan 0081
  has landed, it is a bit of its style table; otherwise this plan adds a `u32` word per
  material id beside the renderer's material upload (albedo, emission), set through
  `Frame::set_material_styles(&[u32])`, which Plan 0081 absorbs (a merge item named in both
  plans' logs). The shadow walk (every shadow ray: the key light's, the sky ray, the lamp's
  and the list's; not a primary ray and not the bake's emissive samples, whose light the owner
  did not complain of) on entering a non-empty brick reads one voxel's material (the brick's
  first set bit through the chunk's palette, a guess at the cheapest; `dev` may read the entry
  voxel instead) and, if its bit is set, returns a hit at the brick's entry. The testbed sets
  the bit on `leaf` (`scene.rs`); `--leaf-shadow brick|voxel` is the window's A/B (default
  `brick`), shown in the banner as `leaf shadow brick`.
- **Files touched:** `crates/vxe-render/src/shaders/walk.wgsl`, the material upload's
  module and `frame/pipelines.rs` (the word's binding, if 0081 has not landed),
  `frame/mod.rs` (`set_material_styles`), `apps/flythrough/src/scene.rs`, `cli/outdoor.rs`,
  `hud/modes.rs`, `crates/vxe-render/tests/gpu_sun.rs`, `apps/flythrough/tests/gpu_outdoor.rs`,
  `docs/flythrough.md`, `docs/plans/logs/0086.md`.
- **Done when:**
  1. **No fleck.** `a_flagged_canopy_shadows_as_its_bricks` in `gpu_sun.rs` (Plan 0032 Phase
     8 bar 5's tree with a canopy at `leaf_fill` 64, a zenith sun at the real radius, the key
     light alone, the coarse walk, 16-frame mean, 4 pixels a voxel; about 8 s): with the bit
     set, no floor pixel inside the projection of the canopy's occupied bricks reads over 1 %
     of the open value; with the bit clear, at least 3 % of those pixels read over 50 % (at
     fill 64 a tenth of the 2^3 blocks are holes, so a column of about 6 blocks is open with
     probability about `0.1^6` straight down, but the holes' flecks spread under the disc
     and the jitter; the 3 % is the floor of that reading, taken first and pinned in the log).
  2. **The silhouette grows by at most a brick.** Same test: with the bit set, every floor
     pixel more than 8 voxels outside the canopy's brick projection reads the open value
     within 1 %.
  3. **Nothing else moves.** With no material flagged, every existing golden is
     byte-identical (the existing golden tests), and `forest_vector`'s hash is unchanged (the
     flag touches no voxel).
  4. **The banner and the flag** (no adapter, under 1 s): the parse test pins
     `--leaf-shadow` and the banner reads `leaf shadow brick` with a sky set.
  5. **Readings:** the key ray's mean and p99 steps in the forest at `--sun 10,180` with the
     bit set and clear (Plan 0032 Phase 8's `--view steps` run), and the light pass's GPU
     time on both adapters; the forest's stillness statistics at `60,180` with the bit set
     and clear. No bar; the window judges.

### Phase 3: History across a stair in the far-normal regime (D2)
- **Owner skill:** dev
- **What:** where the far normal's blend weight is over one half (ADR-0042: the voxel is
  smaller on screen than the blend's lower bound), (i) the temporal pass's acceptance test
  compares the smoothed normal of the previous and the current hit, not the face normal, and
  drops the material test for the irradiance history (the direct target holds irradiance
  before albedo, so a tread of `grass` and a riser of `soil` share a history; the stored
  emissive part has no history to keep); (ii) the key ray and the sky ray start from the hit
  lifted along the smoothed normal by `LIFT` voxels times the blend weight (`LIFT` 1.0, a
  guess), and the walk skips the cell the lifted origin lands in if it is solid, so a stair
  smaller than a pixel casts no self-shadow. Near the camera the weight is 0 and nothing
  changes.
- **Files touched:** `crates/vxe-render/src/shaders/temporal.wgsl`, `sky_light.wgsl`,
  `walk.wgsl` (the skip), `crates/vxe-render/tests/gpu_sun.rs`,
  `apps/flythrough/tests/gpu_outdoor.rs`, `docs/plans/logs/0086.md`.
- **Done when:**
  1. **The staircase keeps its history.** `a_far_staircase_keeps_its_history` in
     `gpu_sun.rs` (a stepped floor of 64 one-voxel steps seen from a distance at which a
     voxel is under the far normal's lower bound in pixels, under FSR 2 at 1.5, a still
     camera, a zenith sun; about 8 s): Phase 1's rejection counter over the staircase's
     lighting pixels reads under 1 % a frame over 64 frames; the same rig before this phase
     is read and printed (the review expects tens of percent).
  2. **A thin far wall does not leak.** `a_thin_far_wall_does_not_leak` in `gpu_sun.rs`
     (a one-voxel wall 40 voxels tall on the floor at the same distance, the sun 20 degrees
     above the horizon on the wall's far side; about 8 s): of the floor pixels in the
     wall's shadow band between 1 and 3 pixels from the wall's screen position, under 1 %
     read over 10 % of the open value. The leak stays within a pixel. The same at `LIFT` 2
     is read and printed, no bar.
  3. **The near regime is untouched.** Every existing golden whose scene has no far-normal
     regime pixel stays byte-identical; a golden that has (the tester's castle gate, if its
     far walls qualify) is re-blessed as a disclosed change and the log names it with the
     difference count.
  4. **Reading:** the forest's stillness statistics at `60,180` with Phase 2's bit set,
     before and after this phase; Plan 0032 Phase 9's Image #3 foreground (the hills' lit
     treads) at the noon pose as a before/after pair of stills in the log's words.

### Phase 4: The sky visibility baked per brick direction (C1, ADR-0070)
- **Owner skill:** dev
- **What:** the store's record gains eight bytes: six `u8` visibilities and two spare. The
  bake job draws `SKY_BAKE_RAYS` (256, a guess) cosine-weighted rays per direction from
  points on that direction's faces, the frame's shadow walk, capped at the chunk's reach in
  voxels (`light_radius` times 64), beyond the cap sky, below the horizon blocked; the
  channel bakes only while the frame holds a `Sky` (`Frame::set_sky(Some)`), else its words
  are zero. A dig re-bakes it with the emissive words in the queue's rings; ring 3 opens also
  when a touched brick's visibility was over zero before or after. The sky-view pass (or a
  pass after it) reduces the LUT to six cosine-weighted axis irradiances `E_sky` in the sky
  uniform. The temporal pass, where it writes the store's emissive part, adds
  `v(n) E_sky(n) exposure_now()` with `v` the bilinear read of the hit face's direction and
  `E_sky(n)` the axis value, blended for a smoothed normal by the normal's squared components.
  The direct target no longer receives the sky ray at a pixel with a fresh value (Phase 5
  finishes the switch; in this phase the ray still runs everywhere and the reading compares).
- **Files touched:** `crates/vxe-render/src/shaders/bake.wgsl`, `temporal.wgsl`,
  `sky_view.wgsl`, `sky.wgsl`, `sky_atmosphere.wgsl`, `crates/vxe-render/src/bake/*` (the
  record, the job, the rings' test, the summary), `frame/sky.rs`, `frame/settings.rs`,
  `apps/flythrough/tests/gpu_bake.rs`, `crates/vxe-render/tests/gpu_sun.rs`, `gpu_sky.rs`,
  `docs/nfr/03-memory.md`, `docs/plans/logs/0086.md`.
- **Done when:**
  1. **The bake does not see the sky, still.** `a_baked_face_ignores_the_sky` (Plan 0032
     Phase 5 bar 8, `gpu_bake.rs`) keeps its claim on the emissive words, and the six
     visibility words are zero with `Sky` `None`.
  2. **The channel reads the open fraction.** `the_sky_channel_reads_the_open_fraction` in
     `gpu_bake.rs` (about 8 s): an open floor's `+z` reads 255 within one step; a vertical
     wall's outward direction in the open reads 0.50 within 0.03 (half the cosine-weighted
     hemisphere about a horizontal normal is sky, by symmetry); the floor brick centred under
     a 16 x 16 slab 20 voxels up reads 0.83 within 0.05 (the form factor from the centre
     point to the slab is four corner rectangles of 8 x 8 at height 20, each
     `(1 / 2 pi) [2 x (0.4 / 1.077) atan(0.4 / 1.077)] = 0.0420`, so 0.168 blocked; the
     brick's faces off centre are blocked a little less, inside the tolerance); the floor of
     a closed rock box with 5-voxel walls reads 0 exactly.
  3. **The baked term matches the ray.** `the_baked_sky_term_matches_the_ray` in `gpu_sun.rs`
     (`Sky::uniform(1.0)`, filter off, exposure 1, the queue drained; about 10 s): the open
     floor's baked sky term reads `pi` within 3 % (Plan 0032 Phase 6 bar 2's ray read
     3.1406), the wall's `pi / 2` within 5 %, the closed box's 0.
  4. **`E_sky` is the LUT's integral.** `e_sky_is_the_cosine_mean_of_the_lut` in
     `gpu_sky.rs` (about 4 s): under the default noon sky the `+z` value equals the
     cosine-weighted numeric integral of the read-back sky-view LUT times `pi` within 2 %,
     and under `Sky::uniform(L)` the six values read `pi L`, four of `pi L / 2`, and 0.
  5. **Deterministic.** ADR-0046's bake-identity test covers the new words: a chunk's record
     is bit-identical whatever the slicing and the job order.
  6. **Still at rest.** `reading::the_tunnel_at_rest_under_the_bake` in `gpu_outdoor.rs`: the
     tunnel pose with `sky_rays` 0 (Phase 5 makes 0 legal; here a test hook) after the queue
     drains reads 0 patches a step over the rim and the far end (nothing moves: a stored value
     with no history), and the two regions' mean luminance against the ray's 64-frame mean
     from Phase 1, printed.
  7. **Cost readings:** the pre-roll's wall time and the queue's step CPU p50 on the hills
     and in the forest with and without the channel, both adapters (Plan 0032 Phase 9's
     `bake queue:` lines); the store's live bytes on the hills and in the forest (Plan 0032
     read 8.8 to 15.5 MB; the record grows by about 29 %); the bake's samples a second. In
     `docs/nfr/03-memory.md` and the log; the `--bake-file` hash changes once.

### Phase 5: The rays' own long history, and the sky ray only where the bake is stale (C2)
- **Owner skill:** dev
- **What:** the **visibility pair**, two `rgba16float` on the lighting grid (r the sky
  visibility's running mean, g the key light's (unused until Phase 6), b the sky's short mean
  over 8 frames, a the age), reprojected with the direct accumulation's reprojection and reset
  by its tests (an edit box, no history, the sky-change bit, Phase 3's acceptance). The sky
  ray scores 1 or 0 (a visibility, coloured by `E_sky(n)` at read); the mean runs to
  `SKY_HISTORY_CAP` (64, a guess Phase 1 sets); a sample is clamped to `FIREFLY_K` times the
  mean once the age passes 8 (4, a guess); when the short mean leaves the long mean's
  three-sigma band under the binomial estimate from the long mean and its age, the age falls
  to 8. The light pass casts the sky ray only where the hit brick has no fresh visibility: no
  store block, or a `SKY_STALE` bit in the record set at the dig for the rings' bricks and
  cleared when their bake lands; the switch from the pair's mean to the stored value fades
  over the store's fade. `sky_rays` 0 is legal: bake only. The direct target no longer
  receives the sky ray.
- **Files touched:** `crates/vxe-render/src/shaders/temporal.wgsl`, `shade.wgsl`,
  `sky_light.wgsl`, `bake.wgsl` (the stale bit), `crates/vxe-render/src/frame/targets.rs`
  (the pair), `frame/encode.rs`, `frame/settings.rs` (`sky_rays` 0, `sky_history`),
  `bake/*` (the stale bit's set and clear), `apps/flythrough/src/cli/outdoor.rs`,
  `hud/modes.rs`, `crates/vxe-render/tests/gpu_sun.rs`, `apps/flythrough/tests/gpu_outdoor.rs`,
  `gpu_upscale.rs` (`frame_bytes`), `docs/nfr/03-memory.md`, `docs/plans/logs/0086.md`.
- **Done when:**
  1. **The mean converges and holds.** `the_visibility_pairs_mean_holds_still` in
     `gpu_sun.rs` (a floor point at the foot of a half-plane roof's edge, 20 voxels up, under
     `Sky::uniform(1.0)`, a hit rate of 0.5 by the form factor of a half plane; the ray
     forced on by a hook; about 8 s): after frame 64 the pair's mean at that pixel reads 0.5
     within 0.1, and its frame-to-frame change is at most `1 / 64` (the running mean's step
     at the cap), against 0.2 on the direct target's blend.
  2. **A firefly is clamped.** `a_rare_hit_is_clamped` in `gpu_sun.rs` (a floor point under
     a roof with a 6 x 6 hole 20 voxels straight above, form factor `36 / (400 pi) = 0.029`;
     about 8 s): over frames 9 to 128 the pair's mean never exceeds 0.25 (a hit of 1 is
     clamped to `FIREFLY_K` times a mean near 0.03), and the same rig with the clamp off is
     read and printed (a hit frame reads near 1 / age). The dark bias is printed beside it:
     the clamped mean against 0.029.
  3. **A moving shadow resets.** `a_moving_shadow_edge_resets_the_history` in `gpu_sun.rs`
     (Plan 0032 Phase 5's 4 x 4 pillar moved one voxel a frame across the open floor under a
     uniform sky; about 8 s): every floor pixel the shadow leaves reads at least 90 % of the
     open value within 12 frames (the open ground's sigma is 0, so the first non-zero sample
     resets; 12 is Plan 0032 Phase 5 bar 6's settle).
  4. **The ray runs only where the bake is stale.** `the_sky_ray_runs_only_where_stale` in
     `gpu_sun.rs` (the open floor baked, the steps view's sky-ray counter; about 10 s): at
     rest the counter reads 0 sky rays; after a 1 m dig in the floor it reads non-zero over
     the dug rings' pixels and returns to 0 once their re-bake lands; the frame count is
     printed. With `sky_rays` 0 it reads 0 throughout, and the dug hole's floor takes its
     baked value when the re-bake lands and nothing before.
  5. **The flag, the banner and the bytes** (no adapter, under 1 s): the parse test accepts
     `--sky-rays 0` and the banner reads `sky rays 0`; `frame_bytes` grows by exactly the
     pair's bytes (two targets of 8 B a lighting pixel) at the test's size.
  6. **Readings:** the tunnel's and the forest's statistics (Phase 1's matrix at the
     defaults) after this phase; the light pass on both adapters at rest on the hills with the
     queue drained (Plan 0032 read 10.5 ms on the RADV RENOIR and 1.04 on the RTX; the
     expectation is the key ray's share alone, a guess of about 5.7 and 0.8) and during a dig.

### Phase 6: The key light on the long history (D3), after a reading
- **Owner skill:** dev
- **What:** first the reading: with Phases 2 to 5 in place, the penumbra's frame-to-frame
  change p99 at 3 degrees on Plan 0032 Phase 5 bar 3's slab rig (the penumbra on the floor
  `2 x 20 x tan 3 = 2.1` voxels wide, 8.4 pixels at 4 a voxel) and on the forest's shadow
  edges at `60,180`, against the lamp's on the same floor. **If the penumbra's p99 is at or
  under the lamp's, this phase stops at the reading and the key light stays on the direct
  target** (the log says so; the mechanism stays unbuilt, not a setting). Otherwise the key
  light's visibility (the shadow ray's 0 or 1; `key_light_at` stays exact) moves to the
  pair's `g` on the same running mean, clamp and reset, and the direct target receives
  `key_light_at(p, n) x g`.
- **Files touched:** `temporal.wgsl`, `shade.wgsl`, `sky_light.wgsl`,
  `crates/vxe-render/tests/gpu_sun.rs`, `apps/flythrough/tests/gpu_outdoor.rs`,
  `docs/plans/logs/0086.md`.
- **Done when:**
  1. The reading is in the log with the decision it made.
  2. If built: `the_penumbra_holds_still_on_the_long_history` in `gpu_sun.rs` (the slab rig
     at 3 degrees, FSR 2 at 1.5; about 10 s): after 64 frames the penumbra band's p99
     frame-to-frame change is at or under the lamp's on the same floor.
  3. If built: **a drift lags under half a voxel.** On Plan 0032 Phase 8 bar 5's tree rig,
     the sun drifting at the testbed's day (`2 pi / 1200` rad a second, 0.005 degrees a frame
     at 60 fps) for 64 frames moves the shadow's centroid by the geometric amount within 0.5
     voxel (the running mean's lag is about half the cap, 32 frames, 0.16 degrees, `80 x tan
     0.16 = 0.22` voxels at the canopy's height).
  4. If built: Plan 0032 Phase 5 bar 6 still holds (a 10-degree jump resets the pair through
     the sky-change bit; 90 % of the new lit value 11 frames later), and bar 3 (`a_moving_
     shadow_edge_resets_the_history`) holds for the key light's shadow too.

### Phase 7: A spike: a deterministic penumbra from the cell hierarchy's closest approach (D4)
- **Owner skill:** dev
- **What:** a spike behind `lighting.penumbra approach|disc` (`disc` the default; the
  banner reads `penumbra approach`): one centre ray toward the body that tracks, along the
  coarse walk, the smallest distance `d_min` from the ray to an occupied cell it passes
  beside, at the depth `t` where it occurs, and returns a visibility
  `clamp(d_min / (t tan r), 0, 1)` with `r` the body's angular radius (the SDF soft-shadow
  rule on the cell hierarchy). The DDA knows the cells it enters, not the ones beside it, so
  the spike reads the neighbouring 2^3 blocks' bits at each coarse step (a guess at the
  cost: four to eight loads a step). Time-boxed: the spike ends at its readings whatever they
  read, and promises no mechanism.
- **Files touched:** `walk.wgsl`, `sky_light.wgsl`, `frame/settings.rs`, `settings.rs`,
  `apps/flythrough/src/cli/*`, `hud/modes.rs`, `crates/vxe-render/tests/gpu_sun.rs`,
  `docs/plans/logs/0086.md`.
- **Done when:**
  1. **The profile against the disc.** `reading::approach_penumbra_against_the_disc` in
     `gpu_sun.rs`: Plan 0032 Phase 5 bar 3's slab rig at 5 degrees, the approach visibility
     at one frame against the disc-sampled 64-frame mean (the reading: a 5 % to 95 % rise of
     2.75 voxels); the profile's deviation in voxels along the row and the width of its rise,
     printed; the same at 3 degrees and under the canopy at `60,180`.
  2. **Stillness and cost:** the forest's statistics with `approach`; the key ray's steps
     and the light pass on both adapters with `approach` and `disc`.
  3. The setting exists, shows its mode, and changes nothing with `disc`. The go or no-go
     is the owner's, at Phase 8.

### Phase 8: The owner's windowed check
- **Owner skill:** human
- **What:** the owner stands in the tunnel at noon and looks at the rim and the far end,
  with `--sky-rays 1` and `0`; digs a skylight into the chamber and watches the chamber's
  floor take the light (the ray first, the bake as it lands); walks the hills at noon and
  watches the far ground while moving (Plan 0032 Phase 10's rule) and the hills' stairs in
  the foreground (Phase 3); stands in the forest at `60,180` and `10,180` with
  `--leaf-shadow brick` and `voxel`, and runs the day from 15:00 to sunset; looks at a
  penumbra at 3 degrees with and without Phase 6 (if built) and with `--penumbra approach`;
  reads the HUD's light pass and the bake queue line through the settle. A ping reaches the
  owner when the window is ready, and the fly-through's stdout stays visible.
- **Files touched:** `docs/plans/logs/0086.md`.
- **Done when:** the owner has recorded a verdict on the tunnel's rim and far end at rest;
  on the dug skylight's lag; on the far ground while moving; on the stairs' speckle; on the
  canopy's shadow as a brick; on the penumbra's grain, and whether Phase 6's history stays
  on by default if it was built; a go or no-go on the approach penumbra; and on any cost the
  HUD showed that the owner will not pay.

### Phase 9: The picks as tests
- **Owner skill:** dev
- **What:** the bars Phases 1 to 7 read, written as tests against the window's picks; the
  defaults the window chose; the goldens Plan 0032 blessed that these mechanisms move
  (`outdoor_noon`, `cave_mouth`, `outdoor_forest_sunset`, the sunset and night ones if the
  bake moves them) re-blessed as a disclosed change with the difference counts in the log; a
  mechanism the window dropped stays a setting if it costs nothing when off (owner,
  2026-11-12) and goes otherwise, and the log names it.
- **Files touched:** the `gpu_*` test files and `golden/` images above, `docs/plans/logs/0086.md`.
- **Done when:**
  1. **The restated stop rule as a test.** `the_tunnel_is_as_still_as_the_lamp_lit_floor` in
     `gpu_outdoor.rs` (the tunnel pose at the defaults, 128 frames at 320 x 180 under FSR 2
     at 1.5; about 20 s, so born `#[ignore = "soak"]`, with a 32-frame native variant in the
     gate): the 99th-percentile frame-to-frame change over the rim band and the far end is at
     or under the headlamp's on the same pose with the sky off. This is Plan 0032 Phase 6's
     stop rule on the occluded rig, as a regression test.
  2. Every bar Phases 2 to 7 name that the window kept exists, asserts the picked value, and
     passes in the gpu tier under the lock; each over 10 s on lavapipe is `#[ignore = "soak"]`
     and passes once by name.
  3. The goldens are re-blessed and the log names each with its difference count and the
     mechanism that moved it.

## Data shapes

```rust
// illustrative: crates/vxe-render/src/bake/record.rs (the store's record, ADR-0046 as built)
pub struct BrickRecord {
    pub emissive: [u32; 6],      // rgb9e5 per direction, as built
    pub material_mask: u32,      // material | direction mask << 16, as built; a SKY_STALE bit
    pub sky_visibility: [u8; 6], // the cosine-weighted fraction of the hemisphere that is sky
    pub spare: [u8; 2],
}
// The visibility pair (two targets, lighting grid, rgba16float):
//   r sky visibility's running mean, g key visibility's (Phase 6), b sky short mean, a age.
// The sky uniform gains six vec4: E_sky for +x, -x, +y, -y, +z, -z (xyz; w spare).
// Frame::set_material_styles(&[u32]) (or Plan 0081's name): bit 0 SHADOW_AS_BRICK.
```

Sizes worked out: the record grows by 8 B (about 29 % of today's); on Plan 0032's hills the
store read 8.8 MB and the forest 15.5 MB, so about +2.5 and +4.5 MB. The pair at the owner's
window (844 x 460 lighting pixels) is `2 x 388 240 x 8 = 6.2 MB`; at 1080p's half tier
(960 x 540) 8.3 MB. The sky uniform grows by 96 B.

## Budgets

| `nfr.md` figure | projection (a guess unless marked read) | how the close measures it |
|---|---|---|
| §2 Lighting, the light pass | at rest inside the bake radius the sky ray is gone: RADV RENOIR about 10.5 to about 5.7 ms on the hills (read: 4.8 ms a ray, Plan 0032 Phase 6), RTX about 1.04 to about 0.8; during a dig the ray returns over the rings' pixels; the canopy flag shortens the key ray in a forest (steps p99 79 to 92 read) | Phase 5's and Phase 2's readings, both adapters, the hills and the forest, at rest and in a dig |
| §2 Lighting, the temporal pass | the store read gains one visibility read and a six-value blend; the pair's read and write | the same runs, the `temporal` pass |
| §2 the streaming line, the bake queue's step | `SKY_BAKE_RAYS` more rays per brick direction while a sky is set: about +6 % of the emissive job's samples at 256 against 4096, but each ray is long | Phase 4's pre-roll and step readings against Plan 0032 Phase 9's `bake queue:` lines |
| §3 World in VRAM (the store) | +8 B a record, about +29 %: about +2.5 MB on the hills, +4.5 in the forest | the store's live bytes at the last frame, Phase 4 |
| §3 Everything else (frame targets) | +6.2 MB at the window, +8.3 at 1080p's half tier; +96 B uniform | `frame_bytes`, Phase 5 |
| §4 Edits, a dig's re-bake | the rings' jobs carry the channel; the owner's lag bar is the ray's, not the bake's | Phase 5 bar 4's frame count; the dig's settle frames as Plan 0059 read them |

On the slow adapter the outdoor light pass is over section 2's line in every configuration
Plan 0032 read; this plan's Phase 5 is the first reading that can move it under, and `nfr.md`
section 2 records the result at the close.

## Risks & open questions

- **The brick-level leak.** A brick holding a roof's top and a cave floor's `+z` faces
  averages their visibilities (ADR-0046's Alternative E, accepted). The tunnel's 25-voxel
  radius and the chamber's 60 are many bricks wide, so the testbed does not show it at the
  owner's poses; a one-brick roof would. Per-face cells near the camera are the backlog's.
- **The colour is averaged.** A warm horizon through a slit reads as the sky's cosine mean
  about the face. ADR-0070's Alternative F (`L1` visibility) is the upgrade if the window
  reads it as wrong.
- **The clamp's dark bias** where the hit rate is lowest, during a re-bake only: about `K p`
  of the truth for `p` under `1 / K`. Phase 5 prints it.
- **The lift's leak.** A lift of up to `LIFT` voxels can pass a one-voxel wall at a distance.
  Phase 3 bar 2 reads it under a pixel or the lift shrinks.
- **Where the style flag lives.** Plan 0081's table or this plan's word; whichever lands
  second absorbs the other's, and both logs name the merge item.
- **The record's growth** against `STORE_BINDING_BYTES` (256 MiB): the forest's 15.5 MB
  grows to about 20; far from the cap, read anyway.
- **The far field.** Beyond the bake radius and before the queue reaches a chunk, the ray and
  its history are what there is, as today; Plan 0033's far field is the end state.
- **The history's lag** on a moving occluder deep in a penumbra, where the variance reset's
  band is wide. Phase 5 bar 3 reads the open-ground edge; the window reads the walkers.
- **Determinism.** Nothing touches `vxe-world`. The channel is render-side, baked from the
  mirror, bit-identical whatever the slicing (ADR-0046), and no gameplay query reads it.
- **The spike's cost.** Four to eight loads a coarse step is a guess; the spike reads it and
  ends. A no-go leaves `penumbra approach` as a setting only if it costs nothing with `disc`
  (a uniform branch), else it goes.

## What this plan does NOT do

- **The far field's fallback for an absent chunk** (the review's B3): Plan 0033.
- **A sunlight bounce**, sky light in the probes, or per-face sky cells (the backlog's).
- **Two radii per body** (A2): the fallback if the penumbra's grain survives this plan.
- **The key ray's length cap and the down-facing skip on the slow adapter** (the review's
  E2): not picked; a profile plan's measured change if the RADV reading after Phase 5 asks.
- **The CPU settle** (E3: the bake queue's step on a world with no emitter, the forest's sync
  p99): `docs/backlog.md`, dated 2026-11-17.
- **The 4-voxel shadow walk everywhere** (D5): the Lego look.
- **Baseline-GPU readings.** The card is not on hand.

## Implementation log

> Written by `dev`: one row per phase as its commit lands, then the close block. **The phases
> above are the contract. This section records what happened.** Observations, never conclusions:
> no pass list, no self-assessment. Deviations and unmet done-whens are **always** disclosed.
> Keep it shorter than `## Implementation phases`. Readings, gate counts, attempts, sweeps and
> resume notes go to the plan's readings log, `docs/plans/logs/0086.md` (started from
> `templates/readings-log.md`), and this section links it once it exists.

Readings, gates and attempts: [`logs/0086.md`](logs/0086.md).

| phase | owner | state | commit |
|---|---|---|---|
| 1: The attribution reading | dev | done | 851f7c72 |
| 2: A canopy shadows as its brick | dev | done | 2b5fff92 |
| 3: History across a stair in the far-normal regime | dev | done | 7a3c6d72 |
| 4: The sky visibility baked per brick direction | dev | done | committed with this row |
| 5: The rays' own long history, the sky ray only where stale | dev | not started | |
| 6: The key light on the long history, after a reading | dev | not started | |
| 7: A spike: a deterministic penumbra from the closest approach | dev | not started | |
| 8: The owner's windowed check | human | not started | |
| 9: The picks as tests | dev | not started | |

### Notes

- Tree names (main since the draft): Plan 0069's surface is `RenderSettings` in
  `crates/vxe-render/src/settings.rs` (`FIELDS`, `NOT_SETTINGS`); `frame/settings.rs` holds
  `Lighting`. Plan 0081 has not landed.
- Phase 1: the rejection counter is four words after the store's five in the temporal pass's
  counter buffer, written under the `COUNT_STORE` override (`Frame::set_count_store`), read by
  `Frame::read_history_rejections`, not a steps-view output. The hooks are `Frame::set_light_hooks`
  (`LightHooks`), two `Params::lighting` bits. The tunnel's regions are chosen per lighting pixel
  by the sky ray's hit rate (as Plan 0032's `the_tunnel_under_one_sky_ray`), not by a pinned
  rectangle; the rectangles are printed. The forest region is every hit.
- Phase 2: the style word lives in a third block of the emission-scale buffer, not a new
  binding beside the material upload; a brick is styled by any id of its palette, not one
  voxel's. The banner names only the non-default `leaf voxel` (the start banner is at its
  9-row cap); the parse test pins that. Bars 1 and 2 run as `reading::a_flagged_canopy_shadows_as_its_bricks`
  in `gpu_sun.rs` (exploration); bar 3's goldens run at the tip's gpu tier.
- Phase 2 commit 2b5fff92 left `hud/modes.rs` at 1205 lines (over the 1200 cap); Phase 3's
  commit brings it to 1200. Phase 1's matrix ran without the fly-through's far normal.
- Phase 3: a reading hook `LightHooks::no_stair` reads the rig before and after in one build;
  bars 1 and 2 run as readings (`reading::a_far_staircase_keeps_its_history`,
  `reading::a_thin_far_wall_does_not_leak`); `LIFT` 2 is not read. Bar 1's rig reads 0.05 %
  resets with the rules off: at the far normal's full weight the stored normals already agree.
- Phase 4: `E_sky` lives in a 96 B storage buffer the temporal pass binds (`Frame::sky_irradiance`),
  written by a new pass `sky_irradiance.wgsl`, not in the sky uniform. The baked term is
  `v E_sky / h`, `h` the hemisphere's cosine share above the horizon (1 for `+z`, a half
  sideways), not `v E_sky`: bars 2 to 4 hold together only so. The switch is per lighting pixel
  on the hit face's own cell (`VIS_BAKED`), with no fade. Ring 3 opens on a touched brick's
  visibility before the edit, not after. Bars 2, 3, 4 and 6 run as readings; bar 7 is partial
  (the log lists what is owed); bar 6 reports Phase 1's change statistics and mean levels, not
  patches a step. One GPU reading ran while a live session runner was up.

### Close triggers

_(Facts for the architect. No recommendations, and no suggested version bump.)_

- **What shipped:** feature / fix-only / docs-chore-only
- **Public API changed:** types, traits, functions, shader interfaces, config (list them, or none)
- **Budget readings:** each `nfr.md` figure touched, with its measured value, machine and commit
- **Gate at the tip:** the commands run (build, nextest, doctests, clippy, fmt, deny), exit codes, test counts
- **Outstanding `human` phases:** which, or none

## Followups

- `L1` sky visibility per brick direction (ADR-0070 Alternative F), if the window reads the
  averaged colour as wrong.
- Per-face sky cells near the camera, with the backlog's per-face emissive cells.
- A render profile (Plan 0069) that sets `sky_rays` 0 on the slow adapter, once Phase 5's
  reading says what the ray costs during a dig there.
