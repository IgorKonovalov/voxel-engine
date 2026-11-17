# ADR-0070: Sky light is a visibility baked per brick direction, coloured by the sky at read; a long-history ray covers what the bake has not caught

> **Status:** proposed
> **Date:** 2026-11-17
> **Related plan(s):** [Plan 0086](../plans/0086-still-outdoor-light-baked-sky-visibility-a-long-history-solid-canopy-shadows-and-stair-history.md)
> (builds it); [Plan 0032](../plans/0032-sun-and-sky-hillaire-atmosphere-time-of-day-and-an-outdoor-testbed.md)
> (whose sky ray this replaces at rest; its Phase 6 stop rule fired at the owner's window of
> 2026-11-17)
> **Builds on:** [ADR-0046](0046-static-emissive-light-is-baked-per-chunk-when-a-chunk-loads-or-changes.md)
> (the per-chunk brick store and its queue, which gain a channel; its Outcome of 2026-11-17),
> [ADR-0027](0027-the-sun-shadow-is-one-per-pixel-ray-in-the-light-pass.md) (the sky ray and
> the key light on the direct target's short blend; its Alternative F, a longer history, is
> taken here for the rays; its Outcome of 2026-11-17),
> [ADR-0026](0026-atmosphere-from-hillaire-luts-built-on-the-gpu-at-runtime.md) (the sky-view
> LUT the colour is read from),
> [ADR-0018](0018-direct-light-per-pixel-one-sample-from-a-hit-buffer-with-svgf-history-killed-by-edits.md)
> (the temporal pass whose reprojection and resets the long history reuses)

## Context

Plan 0032 built sky light as one cosine-weighted ray per lighting pixel whose sample is `pi`
times the sky's radiance when the ray reaches the sky and 0 otherwise, added to the direct
target on its eleven-frame blend (`ALPHA` 0.2). The owner's window of 2026-11-17 (the readings
log, Phase 9; the review `docs/reviews/2026-11-17-plan-0032-window.md`) read it as flicker:
"very much flicker on tunnel edge", "towards the end of tunnel it looks super flickery". The
mechanism is the estimator. At the tunnel's rim the ray reaches the sky with a probability of
about 0.3 to 0.6, a Bernoulli whose 0.2-blend has a standard deviation of about a sixth of the
full value; at the far end the mouth subtends a few percent of the hemisphere, so a rare hit
adds the whole `pi L`, thirty times the mean, and decays over three or four frames. Along the
voxel staircase's contours the temporal pass rejects history where the tread and the riser
alternate under FSR 2's jitter (`HISTORY_COS`), and a rejected pixel shows the raw 0-or-full
sample.

Plan 0032's own guard, Phase 6's stop rule, compared the sky ray's stillness with the lamp's on
an open floor under a uniform sky, where every ray reaches the sky and the variance is exactly
0. The rig had no partial occluder, so the rule passed at one ray and never read the case the
window showed.

A longer history alone cannot fix the far end. A running mean over `N` frames of a Bernoulli
at hit rate `p` has a relative standard deviation of `sqrt((1 - p) / (p N))`: at `p` 0.03 and
`N` 64 that is 0.71, and ten percent needs about 3200 frames. The dim end of a tunnel is
exactly where a screen history is weakest, and it is also where the geometry holds still.

The lighting base already bakes per chunk: ADR-0046's store holds, per surface brick, six
direction values of emissive irradiance, baked when the chunk's reach is resident and
re-baked in rings when an edit touches it, read bilinear over the 2 x 2 cells around a face
with no screen history. A still face read from the store does not flicker; the owner has seen
that since Plan 0059. The bake's rays per brick and its queue step are a cost on the frame's
CPU and on the dig budget (`docs/nfr.md` sections 2 and 4), read in Plan 0067.

The costs read (readings log, Phases 6 to 8): the sky ray is about 0.25 ms of the RTX 3080
Laptop's light pass at 1080p and about 4.8 ms of the RADV RENOIR's, per ray; the RENOIR's
outdoor light pass reads 10.5 ms against section 2's 6.0 ms line with one sky ray. The owner's
standing priority is a stable image first, performance considered (2026-11-03), and "use what
is faster" among choices that look the same (2026-11-17). Plan 0032 Phase 9's done-when also
asks that a dug skylight lights the chamber on the next frames without a visible lag. The first
consumer sets the sky off and must pay nothing.

## Decision

**Sky light at a face is a visibility times the sky's irradiance about the face normal. The
visibility is geometry, baked into ADR-0046's store beside the emissive values; the irradiance
is the sky's, read each frame from the sky-view LUT. Where the store has no fresh visibility
yet, a per-frame ray estimates the same visibility on a long running mean of its own with a
firefly clamp and a variance reset; at rest inside the bake radius no sky ray is cast.**

- **The quantity.** `v_d` in [0, 1] per brick direction `d` of six: the cosine-weighted
  fraction of the hemisphere about `d`, from the direction's faces, from which a ray reaches the
  sky. A ray that leaves the mirror's window or passes the bake's length cap with no hit has
  reached the sky; one that would reach it below the horizon (its direction's `z` under 0 when
  it escapes) counts as blocked, as Plan 0032's ray does. Stored as a `u8` per direction (a
  step is 0.4 % of the open sky's irradiance), eight bytes a record with two spare.
- **The bake.** The job that bakes a brick's emissive directions also draws `SKY_BAKE_RAYS`
  cosine-weighted rays per direction (256, a guess: the binomial standard deviation at `p` 0.5
  is 1/32, eight `u8` steps; Plan 0086 reads it) from points on that direction's faces, with
  the frame's shadow walk, capped at the chunk's reach in voxels, so ADR-0046's reach rule (a
  chunk bakes when every chunk within its reach is mirrored or known empty) guarantees no
  absent chunk inside the cap, and nothing waits on more. The channel is baked only while the
  frame holds a `Sky`; with none its words are zero, the emissive words are byte-equal with the
  sky on and off, and the first consumer pays nothing. A dig re-bakes it with the emissive
  values in ADR-0046's rings; the third ring opens also when a touched brick's visibility was
  over zero before or after the edit, since a dug roof lets the sky down its whole reach.
- **The colour at read.** `E_sky(n)`, the cosine-weighted sky irradiance about a normal `n`
  (`pi` times the cosine-weighted mean of `sky_radiance` over the hemisphere), is computed once
  a frame for the six axis directions by a reduction over the sky-view LUT and carried in the
  sky uniform; a face reads its direction's value and the far normal (ADR-0042) blends the
  three nearest by the normal's squared components. The sky term at a baked face is
  `v(n) E_sky(n)`, with `v` the store's bilinear read, written where the store's emissive part
  is written, with no screen history. Under `Sky::uniform(L)`, `E_sky` is `pi L` for `+z`,
  `pi L / 2` for a horizontal normal and 0 for `-z`, which is what the tests assert. The
  direction of the visible sky is lost: a warm horizon seen through a slit reads as the sky's
  cosine mean about the face. That is the accepted price of a channel that bakes as geometry
  and so needs no re-bake as the day turns.
- **The ray where the bake is stale or absent.** At a lighting pixel whose hit brick has no
  fresh visibility (its chunk has no store block yet, or its record carries a stale bit set at
  an edit for the bricks of the re-bake's rings and cleared when their bake lands) the light
  pass casts the sky ray and scores it 1 or 0: a visibility sample, coloured by the same
  `E_sky(n)` at read, so the bake and the ray estimate one number and the switch between them
  is a switch between two estimates of it, faded over the store's fade. The ray's estimate
  lives in a pair of its own on the lighting grid (`rgba16float`, the sky visibility's running
  mean, the key light's visibility mean once Plan 0086's reading takes it there, a short mean
  for the reset, and the age), reprojected with the direct accumulation's reprojection and
  reset by its tests (an edit box, no history, the sky-change bit), as a running mean to
  `SKY_HISTORY_CAP` frames (64, as the fallback's emissive mean, a guess), with a firefly clamp
  (a sample is clamped to `FIREFLY_K` times the pixel's mean once the age passes a few frames;
  4, a guess) and a variance reset (when the short mean leaves the long mean's three-sigma band
  under the binomial estimate from the long mean and its age, the age falls to the short
  window; at an open-ground shadow edge the sigma is 0, so a moving occluder's edge resets at
  once). `sky_rays` 1 stays the default so a dug skylight lights the chamber on the next frame
  from the ray while its rings re-bake; `sky_rays` 0 is the bake-only setting for a profile
  that cannot pay the ray at a dig (Plan 0069's profiles), where a dig waits for its re-bake.
- **The key light.** ADR-0027's directional sample stays one disc-sampled shadow ray a pixel;
  its visibility may ride the same pair on the same running mean (ADR-0027's Alternative F,
  taken for the rays and not for a radiance), after Plan 0086 reads the 3-degree penumbra's
  grain with the canopy and staircase fixes in place. The direct target keeps the lamp and the
  list on their short blend.

## Consequences

### Positive
- The term the sparkle came from is gone wherever the store is fresh: a still face reads one
  stored number, so the tunnel's rim and far end hold still at rest, as emissive light has
  since Plan 0059. At rest inside the bake radius no sky ray is cast, which is the slow
  adapter's 4.8 ms a ray (a reading, not a promise).
- The channel is geometry. A turning sun and a running day change `E_sky` alone, six values a
  frame; no re-bake follows the clock, which is why ADR-0027's Alternative D (bake the sun and
  the sky) was rejected and this is not it.
- The bake's and the ray's estimate are one quantity, so a re-bake's arrival is a fade between
  two estimates of a visibility and not a seam between two lighting models.
- A dug skylight still lights the chamber on the next frame, from the ray, by default.
- The first consumer pays nothing: with no `Sky` the channel is zero and the bake's emissive
  words are unchanged.

### Negative
- The store's record grows by eight bytes (about 29 % of today's record), and the bake draws
  `SKY_BAKE_RAYS` more rays per brick direction while a sky is set: a cost on the queue's step,
  on the pre-roll's wait and on the dig budget, read against `docs/nfr.md` sections 2 and 4
  by Plan 0086. A sky ray in the bake is long (it runs to the cap or the sky), unlike an
  emissive sample's ray to an emitter.
- The brick level is soft and leaks inside a brick, ADR-0046's Alternative E as accepted: a
  brick that holds both a roof's top and a cave floor's `+z` faces averages their visibilities.
- The direction of the visible sky is averaged away at every face, baked or rayed.
- The firefly clamp biases dark where the hit rate is lowest: at hit rate `p` and clamp `K`
  the clamped mean is about `K p` of the true one for `p` under `1 / K`. That is the far end of
  a freshly dug tunnel until its re-bake lands, and nowhere at rest.
- A history to 64 frames lags: a moving occluder's shadow trails unless the variance reset
  catches it, and it catches it at the open-ground edge, not deep in a penumbra; a drifting
  sun on the pair lags by about half the history (0.16 degrees at the testbed's day, under a
  quarter voxel at 80 voxels up).
- A pair of `rgba16float` on the lighting grid (about 6 MB at the owner's window) and a
  second read path in the light and temporal passes.
- Beyond the bake radius and before the queue reaches a chunk, the ray and its history are
  what there is, as today; Plan 0033's far field is the end state there.

## Alternatives considered

### Alternative A: The long history alone, the ray kept everywhere (the review's C2 by itself)
No bake change: the sky ray's term on a running mean to 64 frames with the clamps above. It
is this ADR's stopgap made the whole answer. The decisive reason it lost is the arithmetic
above: at the far end's hit rate a 64-frame mean is still off by 70 % and the clamp darkens
it; and the slow adapter keeps paying 4.8 ms a ray at rest.

### Alternative B: More rays where the neighbourhood disagrees (C3)
Four sky rays only at pixels whose last-frame neighbourhood held both 0 and non-zero samples.
It halves the noise where it is and leaves the far end's fireflies four times as frequent at a
quarter the size; in the tunnel on the RENOIR every pixel qualifies, up to 15 ms more. The
decisive reason is that it does not remove the term, and the owner picks what is faster.

### Alternative C: Sky light through the probe volume
ADR-0027's Alternative E: the lamp's bounce channel returns the sky on a miss. The volume is
sized to the lamp's reach and runs with the lamp on (ADR-0043); outdoors most of the view is
outside it. Lost for the reason it lost there.

### Alternative D: An unoccluded hemisphere term
`E_sky(n)` with `v` 1 everywhere: no ray, no bake, no noise. It lights a cave's floor through
its roof. Plan 0032 rejected it at its redesign and it is still wrong.

### Alternative E: Bake the sky's radiance, not a visibility
ADR-0027's Alternative D: the store holds the sky's irradiance per direction. It carries the
direction of the visible sky, which this ADR averages away, and it depends on the frame: the
sun's and the sky's colour move all day, so every resident chunk re-bakes continuously.

### Alternative F: Visibility as four spherical-harmonic coefficients per direction
`L1` visibility per brick direction (a mean and three gradients) dotted with the sky's `L1`
recovers the first order of where the sky is visible, so a warm horizon through a slit reads
warm. It is four times the channel's bytes and a second reduction over the LUT. It is the
upgrade if the window reads the averaged colour as wrong; it is not taken without that reading.

### Alternative G: The ray always, blended with the bake
Cast the ray at every pixel and blend its history with the stored value. Two estimators of one
number at rest, the ray's cost on every adapter for ever, and a blend weight with no reading
behind it. Lost because at rest the bake alone is exact to its quantisation and free.

### Alternative H: Per-face sky cells near the camera
The backlog's per-face cells (ADR-0046's rejected face level), for the sky channel alone. It
removes the brick-level leak at the camera's feet for a store several times the size and the
face directory ADR-0046's Outcome deleted. The leak is accepted at the brick level for emissive
light already; the sky channel takes the same level.

## Outcome

_(Added only at acceptance if implementation falsified something above. Dated. Never rewrite the
body.)_
