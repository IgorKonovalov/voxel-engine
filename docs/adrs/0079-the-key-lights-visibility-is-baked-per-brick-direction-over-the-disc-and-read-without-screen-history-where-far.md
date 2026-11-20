# ADR-0079: The key light's visibility is baked per brick direction over the disc, read without screen history where a voxel is small on screen, re-baked when the sun steps, and seeds the near ray's restarted mean

> **Status:** proposed (Plan 0095, draft)
> **Date:** 2026-11-20
> **Related plan(s):** [Plan 0095](../plans/0095-far-key-light-from-the-bake-the-key-visibility-per-brick-direction-no-screen-history-at-the-far-band-a-stepped-re-bake.md)
> (builds it); Plan 0094 (the screen history's transitions, closing beside this; its ADR-0076
> rule 6 is the standard this is read by)
> **Builds on:** [ADR-0070](0070-sky-light-is-baked-visibility-coloured-at-read-with-a-long-history-ray-where-stale.md)
> (the sky's visibility baked per brick direction, the pair the key mean rides, the seed from a
> record), [ADR-0046](0046-static-emissive-light-is-baked-per-chunk-when-a-chunk-loads-or-changes.md)
> (the store, its rings, its identity rule and `FADE`),
> [ADR-0027](0027-the-sun-shadow-is-one-per-pixel-ray-in-the-light-pass.md) (the per-pixel
> disc ray, which the near keeps; its Alternative D is taken here for the visibility alone, at
> the far, with the sun's motion answered below),
> [ADR-0042](0042-the-far-shading-normal-blends-to-the-27-count-gradient-by-the-voxels-size-on-screen.md)
> (the far normal's band, which is the gate here),
> [ADR-0048](0048-the-live-queue-orders-by-the-view-and-withdraws-the-work-a-flight-has-passed.md)
> (the queue order the re-bake rides),
> [ADR-0076](0076-light-stability-rules-world-history-per-source-a-fade-per-transition-and-a-ratcheted-pop-counter.md)
> (rules 2 and 6: the estimate lives in the world; the undrained image is the judged one)

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
  drains, 30 000 frames read). That is ADR-0076 rule 6's case and Plan 0094's and the perf
  plan's work, not this decision's; the seam is stated in Plan 0095.

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

## Decision

**The key light's visibility at a face is baked per brick direction over the sun's disc, in
ADR-0046's store beside the sky's visibility, and read there without screen history where
the far normal's weight is 1; between the band's ends the baked value and the per-pixel ray's
running mean blend by that weight; a near pixel keeps ADR-0027's disc ray, and its restarted
mean seeds from the baked value at `SEED_AGE`, as the sky pair seeds from its record. The
channel is re-baked, by key-only jobs in the queue's view order, when the sun has turned past
the disc since a block's bake and at least a settable step, and with the sky's rings at an
edit; every landing is one fade.**

- **The quantity.** `k_d` in [0, 1] per brick direction `d` of six: the mean over the
  direction's faces of the fraction of the disc visible from the face (ADR-0027's visibility,
  with the SHADOW_ABSORB transmittance folded in), the same number the light pass's ray
  estimates per pixel, so the bake and the ray are two estimators of one quantity and the
  switch between them is a weight, not a seam. Stored as a `u8` per direction in two new
  record words (six bytes, a `KEY_BAKED` flag, the stale bit shared with the sky's
  `SKY_STALE`): the record goes from 9 to 11 words. A block carries the sun direction it was
  baked under (one word per chunk block).
- **The bake.** The job that bakes a brick's sky directions also draws `KEY_BAKE_RAYS` rays
  per direction (64, a guess: the binomial standard deviation at a half-lit disc is 1/16;
  Plan 0095 Phase 1 reads it) toward uniform points of the disc from points on the direction's
  faces, with the frame's shadow walk, the sample points and disc points drawn from a hash of
  the voxel, face and sample index, so the bake is bit-identical for a given sun and geometry
  (ADR-0046's identity). The ray is capped at the chunk's reach like the sky's; a ray that
  ends at the cap, at an absent chunk or at the window's side counts as lit, the light pass's
  own rule for the key ray (ADR-0070's Outcome: "the key light's ray keeps the old rule").
  The channel is baked only while the frame holds a `Sky` with a key light above the horizon;
  with none its words are zero and the emissive and sky words are byte-equal with and without
  it.
- **The read.** At a lighting pixel whose hit's record is fresh (`KEY_BAKED`, not stale), the
  temporal pass takes `k = mix(ray_mean, k_store, w)`, `w` the far normal's weight
  (`far_normal_weight`, 0 where a voxel spans `far_hi` pixels or more, 1 at `far_lo` and
  under), with `k_store` the record's bilinear read over the 2 x 2 cells and the normal's
  blend of the three nearest directions, as the sky's. At `w` 1 the light pass casts no key
  ray at all, which is the term the slow adapter pays most for at rest. The reset of a pixel's
  key mean (any history rejection) seeds the mean from `k_store` at age `SEED_AGE` (8) instead
  of from its first sample, so at a near pixel a restart is a correction from the brick's
  value toward the pixel's, not a `1/n` climb from 0 or 1. A pixel whose record is absent or
  stale keeps the ray and the mean as today (Plan 0094's prior and fades).
- **The sun's motion.** A frame whose key direction has turned past the disc's angular radius
  from a block's baked direction, and past `lighting.key_step` degrees (3, a guess, the
  testbed's disc radius; a live setting), marks the block for a key-only re-bake: a job that
  redraws the six key directions and nothing else, ordered by ADR-0048's view score behind the
  dig jobs and under the queue's budget. Until it lands the far reads the baked value under the
  old sun (rule 6's coarse answer: the shadow lags the sun by at most the step plus the queue's
  delay, and never wobbles), the near ray is exact at once, and the landing is ADR-0046's `FADE`.
  At a fixed sun (the default) no key job is ever queued. An edit re-bakes the key words with
  the sky's rings and clears the shared stale bit.

## Consequences

### Positive
- The term the wobble came from is gone at the far: a far pixel reads one stored number with
  no history, so a slow fly and a turn leave the far canopies and trunks where they are, as
  the still camera already does. The spike's figures to meet are Plan 0095's bars.
- A near restart starts from the right neighbourhood: the climb is from the brick's value,
  not from one sample, so the ground's penumbrae and the near trunks drift far less.
- The slow adapter pays less at rest: no key ray at the far pixels (17 % of the forest rig's
  display; more on the hills), where its 3-degree disc ray is the light pass's largest term
  since the sky went to the bake (ADR-0070's Outcome).
- The bake, the ray and the seed estimate one quantity, so the far band is a blend of two
  estimates and not a second lighting model with a seam.
- With no `Sky`, or a sky with no key light, nothing changes: the first consumer pays nothing.

### Negative
- **The far shadow is coarse.** A brick direction's value is one number over a 0.8 m cell's
  faces (bilinear over the cells): a far shadow edge is soft at the cell's scale and leaks
  inside a brick (ADR-0046's Alternative E, accepted for emissive and sky light already). A far
  thin feature (a trunk under a canopy) reads its brick's mean. Accepted by the owner for far
  surfaces (2026-11-20); the window judges the look.
- **The far misses casters beyond the reach.** A key ray in the bake is capped at the reach
  (256 voxels); a hill's shadow longer than 25 m is not baked, while the near ray walks to
  the window's edge. A far hillside under a far mountain's shadow reads lit where the near ray
  would read shadowed; the band blends the two. A longer cap breaks ADR-0046's identity or
  pays the walk at every lit face. Named for the window; the far field (Plan 0033) is where
  such shadows belong.
- **The sun does not hold still, and this bakes it.** With the day cycle on the far shadows
  step, not turn: at the default day the step is every 10 s, and the whole resident set
  re-bakes per step (key-only jobs, in view order, so what is on screen lands first; the rest
  may lag several steps on the slow adapter). ADR-0027's objection stands in that regime and
  is accepted: the far is an approximation (the owner's direction), the near is exact, the
  cycle is off by default, and the step is a setting a consumer raises. The re-bake rate is a
  budget Plan 0095 reads on both adapters.
- The record grows by 8 B (+22 %; the hills' store 10.6 to about 13 MB, arithmetic), and the
  bake draws `KEY_BAKE_RAYS` more rays per brick direction while a key light is set: on the
  pre-roll and the dig budget (`docs/nfr.md` sections 2 and 4), read by Plan 0095.
- A key re-bake is a store write the frame did not ask for by an edit: rule 4's locality is
  read against the sun step as the event, whose region is everything lit.
- The band blends the ray's mean at weight `1 - w`, so a pixel in the band keeps that share of
  the wobble; the band is 16 to 48 m at 1080p and the default `[2, 6]` with the fly-through's
  field of view (arithmetic: 96 pixels a voxel at 1 m). Whether a blend or a step at `w` 1/2
  reads better is the window's pick.
- Two more words to fetch per lit face in the temporal pass (the record is already fetched
  for the sky), and a third write path in the bake job.

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
seam against the near ray that is not a blend of one quantity. Decisive: the store already
holds the sky's version of the same number, with the bilinear read, the rings, the seed and
the fade built, and the owner accepted the brick level's coarseness. The depth map is the
upgrade if the window reads the brick-level far shadow as wrong.

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

## Outcome

_(Added at acceptance if Plan 0095 falsifies something above.)_
