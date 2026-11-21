# ADR-0079: The key light's visibility is read with no screen history: the baked record where fresh, else the column map's horizon at the far band; a deterministic penumbra near; the record or the horizon seeds any ray mean that is left

> **Status:** accepted (2026-11-21, Plan 0095's close), with the Outcome below; amended
> 2026-11-20 while proposed, at Plan 0095's window W1: the far's instant estimate and the
> near's deterministic penumbra added, the bake kept as the far's refinement, see "Context,
> amended"
> **Date:** 2026-11-20
> **Related plan(s):** [Plan 0095](../plans/done/0095-a-calm-key-light-in-motion-the-column-maps-horizon-at-the-far-the-baked-record-where-it-lands-a-deterministic-penumbra-near-no-pre-roll.md)
> (builds it); Plan 0094 (the screen history's transitions, closing beside this; its ADR-0076
> rule 6 is the standard this is read by); Plan 0096 (the queue's drain and the far tiers' reach
> rule, which decide where and when a record lands at the far)
> **Builds on:** [ADR-0070](0070-sky-light-is-baked-visibility-coloured-at-read-with-a-long-history-ray-where-stale.md)
> (the sky's visibility baked per brick direction, the pair the key mean rides, the seed from a
> record, the column map and its rule "under the column's top is not sky"),
> [ADR-0046](0046-static-emissive-light-is-baked-per-chunk-when-a-chunk-loads-or-changes.md)
> (the store, its rings, its identity rule and `FADE`),
> [ADR-0027](0027-the-sun-shadow-is-one-per-pixel-ray-in-the-light-pass.md) (the per-pixel
> disc ray; its Alternative A, the heightmap, is taken here at the far band alone as the prior,
> and its Alternative D for the visibility alone, at the far, with the sun's motion answered
> below),
> [ADR-0042](0042-the-far-shading-normal-blends-to-the-27-count-gradient-by-the-voxels-size-on-screen.md)
> (the far normal's band, which is the gate here),
> [ADR-0048](0048-the-live-queue-orders-by-the-view-and-withdraws-the-work-a-flight-has-passed.md)
> (the queue order the re-bake rides),
> [ADR-0076](0076-light-stability-rules-world-history-per-source-a-fade-per-transition-and-a-ratcheted-pop-counter.md)
> (rules 2, 3 and 6: the estimate lives in the world; one fade per transition; the undrained
> image is the judged one),
> ADR-0080 (proposed on main with Plan 0096, not on this plan's branch: the far tiers and the
> drain, which bound how long the far reads its prior)

## Context

The owner's complaint after Plan 0094's window 94a (2026-11-20): "a wobble of everything after
turn or when moving, it is clearly seen on far trees for example when I'm slowly fly, it just
can't settle." The spike on branch `spike-motion-wobble` measured it (`docs/plans/logs/wobble-spike.md`,
the forest of seed 1 at 1080p through FSR 2 at 1.5, RTX 3080 Laptop, loaded readings):

- **It is the lighting history, not the geometry or the upscaler.** With the lighting off
  (`albedo`) the far class's frame-to-frame change while flying at 1 m/s is 1.3 % of pixels
  over 4 levels against 14.2 % shaded, and after a stop or a 60-degree turn the far image is
  within 1 % of its final state at once. FSR 2, its jitter, the far normal, the a-trous, the
  half-resolution lighting and the exposure were each switched or pinned: none moved the
  figures beyond run-to-run noise.
- **The mechanism is the restart of the per-pixel running means from one binary sample.** The
  key light's visibility mean (`key_mean`, cap 256 frames) and the sky pair (cap 64) restart
  at every history rejection and converge as `1/n`. A turn restarts 90 % of the far pixels;
  a slow fly restarts a far thin feature (a leaf, a far trunk's edge) continuously, because
  under FSR 2's jitter its hit moves between voxels frame to frame. Per frame pair the far
  class is under 1 % over 4 levels, so a pair-based stillness rule calls it still, but 45 %
  of the far pixels are still over 4 levels from their settled value 2 s after the turn and
  22 % after 4 s: a monotonic sub-threshold drift, which is what the eye reads as "it can't
  settle". Turning the key ray off halves the fly change (8.4 %) and the stop's drift
  (mean |dY| 9.7 to 5.9 levels); with the sky rays off too the fly is 2.4 %, and with the
  queue frozen and the exposure pinned 1.1 %, the albedo's floor.
- **The screen-space tweaks do not remove it.** A short cap (`key_history` 16) shortens the
  drift (14 % at +2 s against 45 %) and raises the still grain (the mean's variance at cap 16
  is 16 times cap 256's); the owner rejected grain at 94a. Accepting any history whatever the
  material and normal (`anyhist`) halves the motion flicker (7.7 %) and leaves the drift
  untouched: a restart from one sample follows either way. A hard shadow (no disc jitter,
  `knj`) does not help: the mean restarts whatever the sample, and the hit itself moves. A
  world-hashed disc point per voxel and face (`tag=hashall`, every distance; `hashfar`, past
  150 voxels), so the sample has no variance at a fixed hit, reads identical to its control
  within noise (hold +2 s 15.0 % against 15.0 % with the queue frozen and the exposure pinned):
  the hit moves between a leaf's voxels, each with its own point, and the mean restarts from
  whichever came first. A coarser caster points the other way: with the canopy's shadow cast
  as whole bricks (`leaf shadow brick`) the fly change halves (6.6 %) and so does the drift
  (29 % against 59 % at the turn's first frame), so the far does not need the leaf-level
  shadow to look still.
- **The second term is the live queue's landings** while the camera rests (two thirds of a
  stop's differing far pixels were faces still waiting for a record; the forest's queue never
  drains, 30 000 frames read). That is ADR-0076 rule 6's case and Plan 0096's work (ADR-0080),
  not this decision's; the seam is stated in Plan 0095.

The owner's direction (2026-11-20): "almost good picture without wobble is better than perfect
picture with wobble. Especially when we are looking at the far distance. the further the
object - more we can approximate its lighting and we don't really need to recalculate it over
and over in case of motion - only changing parts"; and, at 94a, no waiting for light: the
image before the bake lands is the judged image (ADR-0076 rule 6). The owner answered the
spike's two questions the same day: a coarse far shadow baked per brick face direction (a
0.8 m cell, a smooth penumbra) is accepted for far surfaces, and the far gate follows the far
normal's band (ADR-0042's pixels-per-voxel band, `[2, 6]` by default), not metres.

The store already holds exactly this quantity for the sky: ADR-0070 bakes a visibility per
brick direction, reads it with no screen history where the record is fresh, and seeds a reset
pair from the record. A still face read from the store does not wobble, which the owner has
seen since Plan 0059. The sun is one direction instead of a hemisphere, so its visibility
over the disc is a cheaper bake than the sky's, and the testbed's disc (`--sun-radius`, 3
degrees by default; the real sun's 0.27) makes a penumbra about 1 m wide at a 20 m shadow,
wider than the brick cell. ADR-0027 rejected baking the sun (its Alternative D) because "the
store is for light that holds still, and the sun does not": the fly-through's day cycle is
off by default (`Clock::running` false; the owner's windows run at a fixed `--time`), and
when it runs the sun turns 0.3 degrees a second at the default 1200 s day, past the testbed's
disc radius every 10 s. The sky's channel re-bakes on no clock at all (ADR-0070).

### Context, amended 2026-11-20: what the first form showed at window W1

Plan 0095 built the bake and the far read (its Phases 1 and 2) and the owner looked at them
(`docs/plans/logs/0095.md`, W1), with a 30 to 40 s pre-roll drain before every run so that the
far held a record. The verdicts and the readings behind them:

- **The bake alone is not the far's answer, because the far has no record when it matters.**
  A chunk bakes only when its whole light reach is resident full (ADR-0046's identity rule;
  `in_reach` reaches 5.39 chunks at the default light radius 4), and the streamer's full radius
  is 16 chunks, so no chunk beyond about 11.2 chunks (about 72 m) ever bakes under
  `ReachMode::Full`, and the LOD-only ring (16 to 24 chunks) never does: the far baked key
  covers 15 to 72 m at best. Inside that, the queue lands what is in view in minutes (Plan
  0096's problem): after one move about 1 000 to 2 300 in-view chunks had no record (HUD
  `missing 2337 (fallback 1535, black 802)`), and the far fell back to the ray with its
  restart. The drained image was judged "better after full load, but user won't stay that
  long"; the owner's rule restated: a long pre-roll is not a fix.
- **The far is a small share of the wobble the eye sees.** Pixels past 15 m are 10.3 to 12.2 %
  of the lighting pixels in the forest and on the hills from 2 m up; the baked blend "changed
  nothing visible". The near ground's grain and its post-turn wobble were untouched by
  design: the near kept the disc ray and its restarting mean, now seeded from the brick's
  value, and under `step` the owner saw "a fine dotted pattern along each step edge" on the
  near ground: the first samples after a reset moving a seeded mean by `1 / (n + 8)` on the
  penumbra pixels whose sample is 0 or 1 at even odds (a guess until Plan 0095 Phase 5 reads
  the near class by arm).
- **The deterministic penumbra exists and is still by construction.** Plan 0086 Phase 7's
  spike (`lighting.penumbra approach`, a setting, default `disc`): one centre ray that scores
  its closest approach to the rock beside it over the cone's radius at that depth; its frame 1
  against frame 2 reads a change of 0.0000, its rise is narrower than the disc's (1.08 against
  1.76 voxels at 3 degrees) and lies on the lit side of the edge by about half a voxel; it cost
  +0.8 ms of light pass on the RTX and +6.6 ms on the RADV RENOIR in the forest, with
  storage loads six times the disc ray's, and its forest floor metered 1.9 times darker (the
  probes count a leaf's near miss as shadow). Not judged by the owner's eye at 0086's window.
- **A heightfield of the resident world exists and is already the sky's rule.** ADR-0070's
  column map holds, per 2 x 2 voxel column over the whole window's footprint (full and
  LOD-only chunks, the LOD sheets from the level-1 mips), the highest opaque voxel; the sky
  rays, the bake and the sky's no-record seed read it ("a ray ending under the column's top
  is not sky"). It is on the GPU every frame, costs nothing new, follows every upload and
  edit, and covers the LOD ring the bake can never reach.

## Decision

**The key light's visibility at a lit face is a deterministic estimate read with no screen
history at every distance, so that the image right after a move or a turn is the image: at the
far band (the far normal's weight 1) the face reads the store's baked record where it is fresh,
else the column map's horizon toward the sun, softened by the disc's angular radius; near (the
weight 0) the face reads the closest-approach penumbra of one centre ray, which has no mean to
restart; between, the two blend by the weight. The disc ray with its running mean stays as a
setting; where it runs, a reset mean seeds from the far estimate (the record, else the horizon)
and never from its own first sample. The record's landing at a far face, and a key re-bake, are
each one fade. The channel is baked per brick direction over the disc as before, re-baked by
key-only jobs when the sun has turned past the disc and a settable step, and at an edit's rings,
for as long as the window keeps the record as the far's refinement.**

- **The far's instant estimate: the column map's horizon.** At a far pixel with no fresh record
  (no `KEY_BAKED`, or the shared stale bit set), the temporal pass lifts the hit a cell along
  its face normal (ADR-0070's `SEED_LIFT`, out of the wall's own column) and marches the sun's
  horizontal direction over the column map at `COLUMN_KEY_STEPS` (12, a guess) geometric
  distances from 4 to 512 voxels, the tops read bilinear over 2 x 2 cells; the horizon's tangent
  is the largest `(top - z) / d` seen, and the visibility is
  `clamp((tan(elevation) - tan(horizon)) / (2 tan(radius)) + 1/2, 0, 1)` with `radius` the
  disc's angular radius: 1 above every top, 0 under one, a penumbra of the disc's width at an
  edge (a slab `H` voxels over the floor reads 1/2 where its edge is `(H - lift) / tan(elevation)`
  away from the lifted point, 8.9 voxels for `H` 10 at elevation 40, and ramps over about one
  voxel at a 3-degree disc; arithmetic). A march that
  leaves the window's footprint counts the rest as lit. Where the blocking top's material
  absorbs (`SHADOW_ABSORB`, the leaves), the prior reads a constant transmittance
  `COLUMN_ABSORB_T` instead of 0 if Plan 0095's reading shows the plain rule meters the forest
  floor darker than the record by over a fifth (the map gains one flag bit beside the top's
  `z`). The estimate is a continuous function of the hit's position with no state, so a slow
  fly and a turn show it still, as the albedo is; the hit's jump between a leaf's voxels moves
  it by a fraction of the ramp, not from 0 to 1.
- **The far's refinement: the record.** Where the record is fresh the far pixel reads the
  baked key visibility (bilinear, the normal's three nearest directions, as the sky's) with no
  history, as the first form decided. The switch from the horizon to the record at a landing,
  and from the old record to the new at a re-bake, is one fade of `FADE` frames in the pair's
  key slot (`no_ray_fade`'s form, Plan 0094 Phase 5; the key words snap in the store today);
  rule 3's form. Whether the far keeps the record at all, or holds the horizon for good and
  turns the key bake off, is the owner's pick at Plan 0095's window W2 (its question 1): the
  record has the leaves' porosity and the disc's true penumbra; the horizon has no landing, no
  reach limit, no re-bake, and follows the sun every frame.
- **The near: the closest-approach penumbra.** At a near pixel the light pass casts one ray
  toward the disc's centre and returns the approach visibility times the ray's transmittance
  (Plan 0086 Phase 7 as built), and the temporal pass takes it as the frame's value: no
  `key_mean`, no restart, no seed. The probes skip absorbing materials, whose shadow the ray's
  transmittance already carries, which is the named candidate for the darker metering. The arm
  moves into the default pipeline when picked (Plan 0094 Phase 2a's rule for a picked arm).
  `lighting.penumbra disc` keeps ADR-0027's jittered ray with its running mean; there the
  reset's seed is the far estimate at `SEED_AGE`, never the first sample.
- **The band.** `k = mix(k_near, k_far, w)`, `w` the far normal's weight (`far_normal_weight`,
  0 where a voxel spans `far_hi` pixels or more, 1 at `far_lo` and under; 16 to 48 m at 1080p
  at the defaults); or the step at `w` 1/2 (`lighting.key_far_mix`). With both ends
  deterministic the band is too, so the blend keeps no share of the wobble.
- **The bake and the sun's motion** as decided on 2026-11-20 and built: `KEY_BAKE_RAYS` (64)
  disc rays per brick direction from a hash of the voxel, face and sample index, the ray
  capped at the chunk's reach like the sky's, a ray ending at the cap, an absent chunk or the
  window's side counting as lit; two record words, `KEY_BAKED`, the stale bit shared with
  `SKY_STALE`, the block's sun stamp in the summary's first key word; a frame whose key
  direction has turned past the disc's radius and `lighting.key_step` (3 degrees) from a
  block's stamp marks the block for a key-only job in ADR-0048's view order under the queue's
  budget; a fixed sun queues none; an edit's rings re-bake the key words with the sky's. Built
  only while the record stays the far's refinement; the horizon part of the far needs none of
  it and follows the sun each frame.
- **Where the record lands is the queue's business.** How soon an in-view chunk has a record,
  and whether a chunk whose reach crosses the full radius bakes at a far tier from the chunks
  resident (`ReachMode::Partial`, the LOD mips as casters), are ADR-0080's and Plan 0096's;
  this decision makes the far correct enough without a record that neither is waited for.

## Consequences

### Positive
- The image right after a move or a turn is calm near and far without a pre-roll: no estimator
  in the key term carries a restarting screen history, so the far canopies, the far trunks, the
  near ground's treads and the near trunks' penumbrae hold as the albedo does. The spike's
  figures to meet are Plan 0095's bars, read with the queue stepping from the first frame.
- The far reads something sensible from frame 1 at any distance the window holds, the LOD ring
  included: a hill's long shadow and a far ridge's are the heightfield's, which the bake's
  reach cap cannot give (the first form's second negative, gone).
- A record's landing refines a far face from a coarse right answer, not from black or from a
  young ray, so the queue's drain time (Plan 0096's budget) bounds a refinement, not the
  first image.
- The slow adapter pays less at the far (a dozen bilinear texture loads a far pixel instead of a
  disc ray walked to the window's edge); the near pays the approach probes (read: +6.6 ms in
  the forest on the RADV RENOIR before any saving, +0.8 on the RTX), a figure Plan 0095 reads
  net on both adapters and the Portable profile may decline.
- Under the day cycle the horizon's shadows turn with the sun every frame; only the record's
  part steps and fades.
- With no `Sky`, or a sky with no key light, nothing changes: the first consumer pays nothing.

### Negative
- **The far's prior is a heightfield.** Under an overhang, in a far cave mouth lit by a low
  sun, on a sunlit ledge below a cliff's lip, the horizon says shadow where the ray would say
  light: ADR-0027's objection to its Alternative A, confined here to pixels whose voxel spans
  two pixels or less, where the sky already takes the same rule (ADR-0070), and corrected by
  the record where one lands. Under a porous canopy the plain rule reads 0 where the disc reads
  about 0.3 (the leaves' transmittance); a prior wrong by a factor is what ADR-0076 rule 6
  warns against at the landing, so the absorbing top's constant is the named fix and the
  window judges the landing's fade.
- **The near penumbra is an approximation of a different shape.** The approach's rise is
  narrower than the disc's and sits on the lit side of the edge by about half a voxel; a leaf's
  near miss shadows a point the disc would half-light; the shadow of a caster beside the ray
  that the probes do not reach (farther than `APPROACH_REACH` voxels across the ray) is hard.
  The owner's eye judges it at W2 against the seeded disc.
- **Two far estimators, one transition.** A far face changes value once when its record lands
  (and once per re-bake), as a fade; a scene whose queue lands in view order while the camera
  rests shows fades walking outward until Plan 0096 makes the landing seconds. If the window
  reads the fades as the wobble under another name, the far holds the horizon and the bake is
  turned off (question 1).
- **The far shadow is coarse** where the record answers: a brick direction's value is one
  number over a 0.8 m cell (bilinear over the cells), soft at the cell's scale and leaking
  inside a brick; a far trunk under a canopy reads its brick's mean. Accepted by the owner for
  far surfaces (2026-11-20).
- **The sun does not hold still, and the record bakes it.** With the day cycle on the record's
  far shadows step, not turn, every 10 s at the default day, and the resident set re-bakes per
  step in view order (the slow adapter lags several steps); the horizon's part beside it turns
  smoothly, so a band may show a stepped shadow next to a turning one until the re-bake lands.
  The cycle is off by default and the step is a setting a consumer raises.
- The record grows by 8 B (+22 %; the hills' store 10.6 to about 13 MB, arithmetic) and the
  bake draws `KEY_BAKE_RAYS` more rays per brick direction while a key light is set; if the
  window drops the far record the words come out again (Plan 0095 Phase 9).
- A key re-bake is a store write the frame did not ask for by an edit: rule 4's locality is
  read against the sun step as the event, whose region is everything lit.
- The column map gains a flag bit if the absorbing top's constant is built, and the march is
  `COLUMN_KEY_STEPS` x 4 texture loads a far pixel a frame.

## Alternatives considered

### Alternative A: A shorter key history (`key_history` 16)
Measured in the spike: the drift after a turn shortens (14 % of far pixels over 4 levels at
+2 s against 45 %) and the still grain rises (0.38 against 0.33 % over 4 a frame, the mean's
variance 16 times cap 256's). It trades drift for grain, and the owner rejected grain at 94a.
Decisive: a running mean of binary samples cannot be both quick and quiet; the sample has to go.

### Alternative B: The world-hashed disc point (the spike's option B)
The disc point a pixel samples is a hash of its hit voxel and face, the same every frame, so a
still hit's sample has no variance and the mean has nothing to converge over. Built and read
(`KEY_WORLD_HASH`): no gain within noise at any distance, with the queue frozen and the
exposure pinned or not. Under FSR 2's jitter the hit moves between a leaf's voxels frame to
frame, each with its own point, and the restarted mean follows whichever came first; and a
fixed point per voxel is a hard, aliased shadow per voxel, not a penumbra. Decisive: measured.

### Alternative C: A far sun depth map (ADR-0027's Alternative B at the far band alone)
A depth image ray-marched from the sun over the resident footprint, re-rendered at a sun step
or an edit, read with a penumbra filter by the far pixels. It is per voxel (sharper than the
brick cell), has no reach cap short of the footprint, and needs no bake queue. It is a second
shadow model with its own frame (bias, acne, the footprint's edge), about 10 MB at a texel a
voxel over the fly-through's window (arithmetic: a 1632-voxel footprint squared at 4 B), a
re-render of 2.7 million rays per sun step, a penumbra in texels rather than degrees, and a
seam against the near ray that is not a blend of one quantity. Decisive: the column map is
already on the GPU, already the sky's rule, and gives the far its heightfield shadow with no
new image; the record gives the porosity the depth map would also lack.

### Alternative D: Accept any history whatever the material and normal (`anyhist`)
Measured: half the motion flicker (fly 7.7 % against 14.2 %), none of the drift (the hold's
figures within noise), and a wrong mean kept across a real change of surface. Decisive: it
does not touch the restart.

### Alternative E: A wider young pool (`POOL_AGE`)
Plan 0094 Phase 6's age-weighted pool lends a young pixel its settled neighbours' mean. The
pool ends at age 32 and the key mean converges to 256; the far canopy's neighbours are as
young after a turn; and the pool is a screen-space answer. Decisive: Plan 0094 read it as
raising the hole band's slow share (0.4 to 2.4 %) for no fall elsewhere, and turned it off.

### Alternative F: A world-space running mean of the screen's key samples per face
ADR-0076 rule 2's "estimate lives in the world" met by writing each pixel's key sample into a
per-face mean in the store, read back as the estimate. A face's first frames on screen are
still a `1/n` climb from one sample (the spike's mechanism, moved into the store), a face
never on screen has no value until it is, the sun's motion stales it the same, and the frame
gains a write path into the store from the lighting pass. Decisive: the bake gives the same
world-space estimate complete, at rest, before the face is ever on screen.

### Alternative G: The key visibility at stations along the sun's arc
Bake `K` sun positions per brick direction (the day's arc is known) and interpolate, so the
cycle needs no re-bake. `K` bytes per direction (`K` 8 is 48 B a record, more than the record
holds today), and a shadow interpolated between two stations 45 degrees apart is two half
shadows, not one moving shadow. Decisive: the bytes and the look, for a cycle that is off by
default.

### Alternative H: Blend the ray and the bake everywhere
The ray at every pixel, its mean blended with the stored value at a fixed weight. Two
estimators of one number at rest, the ray's cost on every adapter for ever, and the wobble at
the blend's weight. ADR-0070 rejected it for the sky (its Alternative G); the same here.

### Alternative I: The bake alone as the far's answer, with a pre-roll (the first form, added 2026-11-20)
What Plan 0095 Phases 1 and 2 built and W1 judged with a 30 to 40 s drain before each run.
Where the record held, the far was still; after one move 1 000 to 2 300 in-view chunks had
none and the far was the ray again, and nothing past about 72 m (the reach rule at the full
radius) or in the LOD ring ever holds one. The owner: "better after full load, but user won't
stay that long"; a pre-roll is not a fix. Decisive: the far needs an answer that exists before
any bake lands and beyond where one can.

### Alternative J: A wider full residency (added 2026-11-20)
Raise the streamer's full radius so the reach rule bakes farther (`r_full` 20 moves the line
from about 72 to about 97 m). The resident set grows as the cube (the default's 0.87 GB live at
16 is about 1.7 GB at 20 and 2.9 GB at 24; `docs/nfr.md` section 3), the queue's depth with it
(Plan 0096's problem, larger), the LOD ring stays unbaked, and the first image still waits for
the landing. Decisive: it moves the line and pays for it everywhere; the far needs no record to
be calm.

### Alternative K: `ReachMode::Partial` by distance as the far's fix (added 2026-11-20)
Let a chunk whose reach crosses the full radius bake from the chunks resident, the LOD mips as
casters, and refine when the reach fills: the baked far reaches about 102 m instead of 72, the
LOD ring still never, and every face still waits for its landing and shows the ray meanwhile.
It is the right rule for Plan 0096's far tiers (a far tier that can never bake is no tier) and
goes there with ADR-0080; it is not what makes the first image calm. Decisive: a queue rule
cannot answer a frame that has no record yet.

### Alternative L: The disc ray with a seeded, never-restarting mean at the near (added 2026-11-20)
Keep ADR-0027's jittered ray and seed every reset from the far estimate at `SEED_AGE`, which
Plan 0095 Phase 2 built from the record. The owner saw its result at W1 on the near ground: a
dotted pattern along the step edges, the first samples after each reset moving a seeded mean by
`1 / (n + 8)` at the penumbra pixels, where a sample is 0 or 1 at even odds; a seed changes
where the climb starts, not that there is one. Kept as the `disc` setting with the seed, and
as W2's control arm. Decisive: a binary sample with any screen history shows its first samples;
the near needs a value with no samples.

### Alternative M: More disc rays per near pixel (added 2026-11-20)
Four rays a pixel cut the mean's variance by four and the first frames' grain by two; the
RADV RENOIR pays four times its largest light-pass term, and the restart remains. Decisive:
cost for a half measure.

## Outcome (2026-11-21, Plan 0095's close)

Plan 0095 built the decision as amended; what it read against the body above (the readings log
`docs/plans/logs/0095.md`, Phases 4 to 10 and the close review's fix pass):

- **The far estimate reaches few pixels yet.** The record, else the horizon, is read only at
  the far normal's weight 1; inside the band (`[2, 6]` pixels a voxel) the key ray is still
  cast. In the forest at 1080p 96.9 % of the far's key samples over a hold came from the ray,
  1.8 % from the horizon and 1.3 % from the record, so the far's motion reads about the
  before's: queue frozen, hold +60 27.2 % against 30.8 %, fly 13.2 against 14.8 %. The bars
  were set by the owner to "no worse than the before + 1 point" (lavapipe gate scene, soak);
  narrowing the key's far gate is the backlog's "Narrow the far normal's band so far pixels stop
  casting the ray", for the far-field work or Plan 0096's far tiers.
- **The horizon as built** reads each column top as a box from the cell's entry distance, with
  a refinement walk of at most 8 cells in the steepest interval, not bilinear over 2 x 2 cells
  (bilinear put a 10-voxel slab's ramp at 7.9 voxels where the geometry says 8.9). A lifted
  point under its own cell's top reads 0. The absorbing top was not built: the forest floor
  meters within 0.1 EV of the drained record under the horizon (the condition was a fifth).
- **The near's approach penumbra is the fly-through's default on every adapter and profile**
  (the owner's pick over keeping the disc on the slow adapter), its probes compiled into the
  default light pipelines; the library's `Lighting::default()` keeps `disc`. Its cost is the
  negative this body under-stated: the RADV RENOIR's forest light pass reads 34.0 ms against
  12.0 under disc (hills 11.2 against 8.0), over section 2's 4.0 ms line; the RTX 3.0 to 3.4
  ms. Plan 0096 Phase 6 takes it as its first lever. At 1080p on the RTX the near stops clean
  (stop +0 near 0.81 % against the lighting-off 0.04 %, the disc's 14.9 %); at lavapipe's
  320-pixel grid the band starts at about 2.2 m, so the near bar is asserted only there.
- **The bake stays the far's refinement** (W2 kept it): the record is 11 words (+8 B, the store
  11.0 to 13.6 MB live outdoors); key-only re-bakes follow a turning sun past `key_step` (3
  degrees), re-keying every marked block 2.3 to 5.7 s after a step on the RTX and 7.1 to 49.5 s
  on the RENOIR under the default day. A dig's ring re-bakes the key words with the sky's:
  about +21 % of the ring's samples, the dig frame's light pass +1.31 ms on the RENOIR (to 3.31
  ms); the dug box's over-4 share at the dig's frame rises (RTX 3.54 against 1.21 %, RENOIR
  2.97 against 1.26 %) and the RENOIR's ring lands fewer bricks by N + 2 (138 against 368), a
  reading left to the backlog's dug-hole entry.
- **Undrained**, as the owner sees it (ADR-0076 rule 6), the hold's far +120 reads 60.7 %
  until records land (7.3 % at +210): the landing waits on the queue, Plan 0096's. The rule 2
  far return (84.7 % against the before's 77.3 %) is a reading, not a bar.
- **The owner at W2:** the default against the before, "before - grain"; overall "we are much
  better than before". The step lines along slopes (both arms) went to the backlog and the
  exposure in deep shade to Plan 0097.
