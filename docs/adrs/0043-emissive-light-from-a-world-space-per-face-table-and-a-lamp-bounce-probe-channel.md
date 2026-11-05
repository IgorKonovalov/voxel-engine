# ADR-0043: Emissive light is read from a world-space table of per-face irradiance, and the probe volume becomes a lamp-bounce channel

> **Status:** accepted (2026-11-05, Plan 0057's close; the table, fill, preload and guard are expected to be superseded by ADR-0046 on the owner's verdict in Plan 0059's Phase 13)
> **Date:** 2026-11-04
> **Related plan(s):** [Plan 0057](../plans/done/0057-steady-light-the-per-face-table-as-the-emissive-store-and-a-lamp-bounce-channel.md)
> (builds it); [Plan 0051](../plans/done/0051-steady-light-spike-converge-and-freeze-and-a-world-space-emissive-cache.md)
> (the spike whose Phase 4 verdict this records; its readings are in `docs/plans/logs/0051.md`).
> Amends [ADR-0018](0018-direct-light-per-pixel-one-sample-from-a-hit-buffer-with-svgf-history-killed-by-edits.md)
> (the emissive part leaves the screen history), [ADR-0023](0023-indirect-light-from-a-scrolling-probe-volume.md)
> (what the probe rays shade and how a probe blends) and
> [ADR-0036](0036-static-emissive-light-on-a-long-full-precision-history-guarded-by-an-emissive-gradient.md)
> (the long emissive history and its screen-space guard, both superseded). Makes
> [ADR-0037](0037-probe-rays-direct-light-keeps-its-energy-apart-from-the-screen-firefly-clamp.md) moot.
> Leaves [ADR-0033](0033-eye-adaptation-as-an-asymmetric-two-stage-exposure-on-the-gpu.md) and
> [ADR-0041](0041-the-surface-is-the-trilinear-zero-set-of-the-27-count-field.md) as they are, and
> rests on ADR-0041's cubes for its key.

## Context

Everything on screen but the headlamp is static light, and the renderer of ADR-0018, ADR-0023 and
ADR-0036 re-estimates it with fresh random numbers every frame and stores the estimate where it
cannot hold still: in screen pixels, which lose it on every turn, and in probes blended by an
exponential average, which never converges. Plan 0051 built three ways out behind settings and
read each on the showcase's hall, the level's two poses, and in the owner's window on 2026-11-04.
The readings that decide this (the spike's log, RTX 3080 Laptop unless named, unasserted):

- **The hall at a still camera, lamp off, native scale, 4096 against 8192 frames, mean per
  pixel in 8-bit levels:** today 2.193; A (probes and pixels converge and freeze) 0.330; B alone
  (the per-face table, probes as today) 1.472; A plus B 0.003; **B with indirect off 0.004**.
  Under FSR 2 at 1.5 every arm reads 0.5 or more (B with indirect off 0.525), so the upscaler
  moves the image on its own, which Plan 0056 reads.
- **No arm holds still within the owner's "I can count to 5"** (about 300 frames): at 512 frames
  against its own 8192 the least mean is 0.977 (A plus B, native); B with indirect off reads
  1.133.
- **The level at the two poses, lamp on, frames 1025 to 8192, `sigma_view` of the composite:**
  today 0.0413 at the vein; B with indirect off 0.0009 at both poses (jitter across face edges
  under FSR 2 is the rest); the emissive part alone reads 0.0000 at scale 1.
- **Cost at rest, the level's spawn, 1080p from 720p, loaded (indicative), frame ms:** RTX today
  4.944, B with indirect off 3.255; AMD Cezanne on RADV today 35.438, B with indirect off 21.053.
  The light pass under B reads 0.113 ms on the RTX against 0.802, because a capped face's pixels
  draw nothing.
- **The owner's window.** On the hall: today "still settles for a long time"; A "a bit better but
  off"; A with the lamp out of the probes "worse, esp when i turn fast"; A plus B plus C "first
  real improvement! there is no dots ... only colored lights when im in a radius of better
  lighting"; **B with indirect off "i think i like it, there is no radius with better lighting,
  there is no colored flickering. but problem of fast turn is back, also when i dig it takes some
  time for light to settle in hole"**. On the level every arm kept "small yellow dots" on fast
  motion. The owner's verdict: B with indirect off, the nearest read, exposure unchanged, and the
  probes "shrink to a lamp-bounce channel" because "maybe we will come back with indirect light
  for some sources later, this is why I want to keep some probes logic. But we clearly are doing
  something wrong and logic should be redone anyways."

Two facts about the tree make the table cheap and exact. ADR-0041's honest cubes give every hit
an entry face and an integer voxel, so the key `floor(p - 0.5 nf)` plus the face axis and sign
names one world surface element with nothing stored in the mirror; and the table's sums are
integer atomics in Q17.15, so a cell's mean does not depend on the order its pixels add. The
spike's table filled 0.0142 of its 2^22 cells after a flight, and its rare-event estimator
reaches 16 samples in about one frame at arm's length (54 lighting pixels a face) but in about
48 frames on a face 88 voxels away (0.34 lighting pixels a face), which is where the owner's
"fast turn" and "settle in hole" complaints sit.

## Decision

**The emissive direct irradiance of a cube face lives in a world-space table keyed by voxel and
face, and a pixel's emissive light is its face's mean, read nearest.** The screen holds no
emissive history. The probe volume holds the headlamp's bounce and nothing else, and is off when
the lamp is off. In detail:

- **The table is the only emissive store.** A cell holds its key, a sample count and three Q17.15
  sums. The light pass adds its one resampled emissive sample to the hit's cell and the pixel
  reads the cell's mean the same frame, from the first sample on (`C_MIN` 1); a face whose
  count has reached the cap adds nothing and its pixels draw no emissive candidates and cast no
  emissive shadow ray. The read is nearest: one value a face, no blend across faces. A hit that
  has no face (the opt-in field look, a translucent hit, a ray that started in rock) keeps a
  short screen history of its own sample, the lamp part's rule; it is the fallback, not a
  second emissive path. The a-trous filters the lamp's part alone.
- **Faces are sampled, not only pixels.** A fill pass draws a fixed number of samples a frame
  for every young face in view (a cell under its cap that a pixel touched this frame), from
  points on the face, so a far face converges at the fill rate and not at its footprint on
  screen; and a small set of rays from the eye over the full sphere warms the faces about to
  come into view. Plan 0057 reads whether the young faces are the owner's fast-turn fault before
  it builds the fill pass (its Phase 1), and the rates are its readings; if the young faces are
  not the fault the fill pass is dropped and this ADR's Outcome says so.
- **Invalidation is an edit box plus a world-space guard.** An edit box, dilated by one voxel,
  clears every cell of every face in it (the spike's reset, with its dispatch capped and an
  overflowed frame clearing the whole table). Beyond the box, each cell keeps the emitter face
  that contributed most to it and that sample's luminance; the gradient pass re-shades that
  point from the cell's face at the stratum cadence of ADR-0036's guard (one cell a 3 x 3
  stratum of lighting pixels a frame, a shadow ray and the emitter's radiance now) and reopens
  the cell where the result moved past a dead zone. A reveal, a dug-away emitter or a changed
  emission therefore reaches every cell in view within about nine frames, and a cell out of view
  is re-verified when it is next seen.
- **Size, eviction, the full table.** 2^20 cells by default (provisional, set by Plan 0057's
  occupancy reading over a long flight), linear probing over four slots. When the camera's chunk
  changes, a sweep clears every cell whose voxel lies beyond the mirror's light radius of the
  camera, which bounds the table to the surfaces the light table also covers; the cell carries
  its key so that the sweep can read it. A key that finds neither its cell nor a free slot is
  counted and shows its raw sample, and the counter is a HUD row.
- **The probe volume is a lamp-bounce channel.** The probe rays shade their hits with the
  headlamp alone: no emissive sample, no hit emission, no previous-bounce term. Only probes
  within the lamp's reach are listed; the reach is set from a reading of the bounce's share of
  a lit wall at the eye and is bounded by the volume, which shrinks to it (a guess: 16 probes a
  side at the spacing of 16 voxels). A probe blends as a running mean that freezes when the lamp
  is still (Plan 0051's A1), and at a fixed short blend while the lamp moves, set by a reading of
  the flight's flicker against the lag at a lamp step. With the lamp off no probe pass runs and
  the gather adds nothing. Inside probes are skipped (A2). What the channel keeps of ADR-0023 is
  the scaffolding a later per-source indirect light needs: the scrolling tagged volume, the
  octahedral irradiance and visibility maps, the gather's trilinear, cosine and Chebyshev weights,
  the inside flag, the edit-touch list, the Fibonacci ray set and the on-demand allocation. The
  lamp is the first source fed into it; the owner's parked "bounce only for certain sources"
  (2026-10-22) is a flag per source into the same channel, later.
- **Exposure's settle (ADR-0033) does not change.** The owner's answer at the verdict.

**What this deletes**, reconciling the owner's "B's list plus the probes" with the shrink: the
emissive screen history and its `f32` store (the accumulation pair returns to `rgba16float`; its
emissive channels carry the table's read, or the fallback's short blend where a hit has no face);
the emissive guard's screen-space form (the cell carries the kept point instead) and the
reservoir textures that held it; reservoir reuse and the `Sampler` choice (the emissive sample is
resampled from the candidates alone); the a-trous over the emissive part; Plan 0051's A3 and A4
(`emissive_history`, `skip_converged_draw`) and the `smooth` read; the `--steady` and
`--cache-read` presets once the default lands; in the probes, the emissive sample and its shadow
ray, the hit's emission and the previous-bounce recursion, the per-material emission scale and
the emission-change window, the hysteresis schedule (`HYSTERESIS`, `YOUNG_UPDATES`,
`YOUNG_HYSTERESIS`), the lamp-reach rule at `intensity / d^2 < 1 / 256` (554 voxels, past the
volume's far corner), `probe_lamp` and C as a setting, and the emissive furnace tests with Plan
0048's question.

## Consequences

### Positive
- Static light is static. A still camera at the hall reads 0.004 against today's 2.193 (native,
  the spike), and the emissive part's wander reads 0.0000 at scale 1 at the level; nothing moves
  in the image but the lamp, the upscaler and what an edit changed.
- A turn or a flight reads values that already converged: the table outlives the view. With the
  fill pass a face converges at a rate the plan sets, not at its footprint, which is the spike's
  arithmetic for the fast turn and the dug hole.
- The cost at rest is the cost of what changed. A capped face's pixels draw nothing (the light
  pass 0.113 ms against 0.802 on the RTX), the probe passes run only with the lamp on and only
  within its reach, and the frame at the level's spawn read 3.255 ms against 4.944 on the RTX,
  21.053 against 35.438 on the Radeon, indirect off (indicative).
- The emissive sums are integer atomics, so a face's mean does not depend on the order its
  pixels add; the screen's `f32` history was not order-independent.
- The key costs the mirror nothing (ADR-0041's cubes), and a dig's cells are countable from its
  box, so invalidation is exact where it is cheap.
- The probe scaffolding stays for a later per-source indirect light, with the part the owner
  called wrong (what the rays shade, how a probe blends) redone rather than patched.

### Negative
- **One value a face.** A step at every voxel edge in the emissive part at arm's length (about
  54 lighting pixels a face), and no per-pixel detail from emissive light. The owner saw
  `nearest` and `smooth` side by side and kept `nearest`; the step is the accepted look.
- **No indirect light from emitters.** A wall lit by an ore casts no bounce; the hall and every
  lamp-off scene are direct light only. The owner saw that look ("i think i like it") and the
  probe edge's "radius of better lighting" went with it. Indirect light from some sources is the
  later return the channel is kept for.
- **A stale value where nothing looks.** A cell out of view keeps its old light until it is seen
  and re-verified, about nine frames after; a cell whose kept emitter did not change but whose
  light did (a second emitter behind the first) is caught only when a pixel's new sample lands
  on the reopened cell. The guard sees one point a cell, as ADR-0036's saw one a pixel.
- **The estimator's clamp bias lives in the mean.** A face near a bright emitter stores the mean
  of clamped samples, biased low by the clamp's share; it is now a steady world-space bias
  rather than a flicker. Plan 0048's unbiased schemes are moot for the probes and were never for
  the screen.
- **Memory.** 2^20 cells at about 40 bytes (key, count, sums, the kept point, its luminance and
  the fill list's frame stamp) is 41 943 040 B, arithmetic, against the spike's 83 886 080 B; the
  emissive history's `f32` pair and the probe volume's eightfold shrink give some of it back
  (arithmetic in Plan 0057's Budgets). The table is per world, not per view, which is the VR door kept open.
- **The fallback flickers.** The field look, translucent hits and rays from inside rock keep a
  short screen history of one sample a frame with no table behind it, so the opt-in field look is
  noisier than the cubes under this design; a smooth material's key is a later question.
- **The lamp channel lags the lamp.** Its blend while the lamp moves trades flicker for lag; the
  reading sets it, and the bounce of a fast-swung lamp arrives a few frames late.
- **M2's headless proof changes shape.** "Irradiance in a closed test box matches an analytic
  value" was the emissive furnace; it becomes a lamp furnace (a closed box lit by the lamp,
  against the lamp's direct mean) or is dropped from the milestone, Plan 0057's call to record.

## Alternatives considered

### Alternative A: Today's design, tuned
ADR-0018's screen history, ADR-0036's 256-frame emissive history and guard, ADR-0023's probes at
hysteresis 0.97. The spike re-read it on the default look: the hall drifts 2.193 at native 4096
against 8192 frames, the gather owns 0.65 to 0.90 of the view-mean variance at the vein, and the
owner's notes were "no good" and "still settles for a long time". Every knob position of this
design (Plan 0049's R, P, L, G, Y1 to Y5, hysteresis 0.99) trades "still at rest" against "fast
after a change" along one history length, which is why it lost.

### Alternative B: Converge and freeze inside today's design (the spike's A)
Probes and emissive pixels as running means that freeze, inside probes skipped, converged pixels
skipping the draw. It is the cheapest change and reads well at rest (a-native drift 0.330), but
its pixels converge only where the screen keeps them, so a turn restarts from one sample as
today, its freeze holds a stale value wherever nothing restarts it, and its indirect-off wander
rose 1.63 to 3.96 times today's at the level. The owner: "feels a bit better but off"; with the
lamp out of the probes "worse, esp when i turn fast". Its running mean survives as the lamp
channel's blend; the rest goes.

### Alternative C: The table with the probes kept (the spike's `abc`)
A plus B plus the lamp out of the probes, the owner's best hall arm ("first real improvement!
there is no dots"), ab-native drift 0.003. It lost on two counts: the owner saw "colored lights
when im in a radius of better lighting", which is the probe volume's edge and goes only with the
emissive gather; and it keeps the whole probe mechanism the owner called wrong, with its
emissive sample, windows and schedule, for a bounce the owner then chose to live without. The
table without the probes reads the same drift (0.004) with none of that.

### Alternative D: The table fed by pixels alone (B as spiked), read at `C_MIN` 16
What the spike built: a cell reads only after 16 samples, every sample comes from a pixel's hit,
nothing warms a face before it is seen. Its convergence is the face's footprint on screen: one
frame at arm's length, about 48 frames at 88 voxels, and no arm settled within 300 frames at the
hall. That is the arithmetic behind the owner's "problem of fast turn is back" and "takes some
time for light to settle in hole", so the decision samples faces and reads from the first sample.
Whether that is the whole of the fast-turn fault is Plan 0057's first reading.

## Outcome

Accepted 2026-11-05 at Plan 0057's close, v0.43.0, because it describes what ships. Readings are in `docs/plans/logs/0057.md`.

**The owner did not accept the table as the destination.** First window, 2026-11-05: "there is no flicker nether when i turn or go forward/backward", and "it feels like this is wrong way". Second window: "quite a few of dark patches when moving fast, but it seems they are gone after stabilization", and then "no matter performance, but build something that would look really polished". **What held:** a still face read from a settled value in world space, and the empty screen history. **What failed:** a fill that follows the view, because a face's light depends on when someone looked. The table stays the working default until a bake replaces it (ADR-0046, Plan 0059). The lamp channel and the face-keyed read outlive it.

**What implementation changed against the Decision above:**

- **"The read is nearest: one value a face, no blend across faces" is reversed.** The owner's first window showed grain at voxel scale, so a count-weighted 3 x 3 coplanar blend (5 x 5 where the pool is under 4096 samples) is the default, computed once a seen cell a frame, with `--blend off` beside it. It still reads one value a face. The window gave no words on its softening of edges; that question moves to Plan 0059's Phase 5.
- **Faces are sampled, as read.** The fill of every young face in view and the warm rays are dropped (Phase 1's stop rule: the young-cell share of the fast yaws' excess read 0.005 to 0.099). A fill for cells an edit or the guard reopened is built, and the settle is a camera-radius brick sweep around the eye (the preload, `R` 64, `r0` 32) instead of rays from the eye. The owner: "preload doesnt really change anything - or it should be much much bigger".
- **The fallback is a frozen mean,** not a short screen history, so a still fallback pixel holds still and this ADR's "the fallback flickers" no longer holds.
- **The guard runs only within 256 frames of an event** (the still reopens were a defect against the owner's first rule). So "a cell out of view is re-verified when it is next seen" is true only inside that window: a cell whose emitter changed while it was out of view, first seen later, stays stale until the next event. Inferred from the code, not read on a rig. No test covers it, and it goes with the table.
- **Size, eviction, the full table.** 2^20 fell to the window: 993 304 cells and 194 079 missed pixels a frame at the hall. Now 2^22 cells of 48 B (201 326 592 B), fewer where the device's storage binding stops short (2^21 on lavapipe), 16 probes, a sweep and an age-out in slices of 2^20 cells. Peak 833 237 cells at the hall with 0 misses at the window's size.
- **The age-out's re-arm is not in the plan's text and is accepted at this close.** A pass that leaves the load over the mark, having freed a cell or started on growth, runs again `age` frames later, and a re-run that frees nothing ends it. The literal rule could not clear a second view (Plan 0057's done-when 2). It frees only cells beyond `R` that nothing has read in `age` frames, so it touches nothing in view, and a still camera ends after a bounded number of passes.
- **A frame over `MAX_EDITS` still clears the whole table:** the vein's `B` presses (78 to 87 carves). A frame in budget clears by a walk over the cells.
- **The lamp channel** is side 8, not the guessed 16 (a reach read from the brightest probe texel over `D`, not from the gather at a lit wall), and `Lighting::default()` keeps it off with the fly-through on. Two faults stay: inside probes are traced while the lamp moves (cost only, the gather ignores them), and `the_lamp_furnace_reads_its_albedo_times_the_direct_mean` takes its `D` from the shader's own output, so it defends the running mean and not the albedo. Plan 0058 owes the fix when it touches the channel.

**Bars the owner closed as missed:** the vein's `G_noise` p50 / p90 2.23 / 9.79 (FSR 2) and 2.07 / 7.97 (native) against 1 / 2, and `G_plane` p90 at 0.59 and 0.77 of before against 0.5; the first frame within 4 levels at 0.62 to 0.64 against 0.9, at any `POOL_MIN`, blend on or off. New faces hold 256 to over 1024 samples on that frame, so the reading is estimator variance beside the emitters, not young cells; this was not tested directly, and Plan 0059's Phase 1 reads it. **Unread:** the amended still shares of Phase 4's done-whens 14 and 16 (the notes cite the old bar); moot if the preload goes. **Worse than before:** the carved region within 2 levels at +1 reads 0.55 against 0.83 native, on the blend's tree, not attributed further; its `Q90` of 52 frames meets the bar.

**A close-review fix:** a cell past a hole in its probe run dropped out of its own blend and could read black with no neighbours. The centre now reads its own slot (6738b9f, `a_cell_past_a_hole_keeps_its_own_light`).
