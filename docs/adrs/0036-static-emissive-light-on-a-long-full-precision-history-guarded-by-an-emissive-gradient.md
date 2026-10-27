# ADR-0036: Static emissive light accumulates on a long full-precision history, guarded by an emissive-only gradient

> **Status:** proposed
> **Date:** 2026-10-27
> **Related plan(s):** [Plan 0047](../plans/0047-emissive-light-holds-still-long-guarded-history.md);
> refines the temporal pass of
> [ADR-0018](0018-direct-light-per-pixel-one-sample-from-a-hit-buffer-with-svgf-history-killed-by-edits.md)
> and reads the candidates of
> [ADR-0025](0025-light-candidates-from-a-power-over-distance-table-of-near-emitter-chunks.md)
> unchanged

## Context

With the lamp off near an ore vein, wall patches step on one by one and the light flickers at
low frequency. The owner calls it "really unnatural". Plan 0040's headless investigation
(its Implementation log, step 4) measured it on the level at 192 x 108, under `Manual`
exposure so the eye is ruled out. About 70 of 312 wall tiles move by more than 10 % per half
second in steady state. The patches go to 0 with the emissive candidates off. They rise to
about 178 with the emissive history at 1 frame, and fall to about 48 with the flat sampler,
43 with four times the candidates (32) and 32 with a 64-frame history. Streaming is not a
factor: the mirror is synced whole before the run.

The mechanism is in `light.wgsl` and `temporal.wgsl`. Each lighting pixel takes one emissive
sample a frame, resampled from 8 candidates drawn from the camera's table of near emitter
chunks (ADR-0025). The resampling target leaves visibility out, so most kept points are behind
rock and the one shadow ray returns 0. The rare unoccluded point returns up to the firefly
clamp. The per-frame estimate is therefore a rare-event signal of high relative variance. The
temporal pass blends it at `max(1 / (age + 1), 1 / 32)`. That is a running mean for 32 frames,
then an exponential window of about 63 frames' effective weight. Reservoir reuse holds a kept
point for many frames (M is capped at 20 times the candidates) and spreads it to neighbours
within 30 lighting pixels. Neighbouring errors are therefore correlated and persistent, and
the window holds fewer independent samples than its length. The a-trous filter spreads
whatever error remains into blobs about a tile wide. A blob holds while its reservoirs keep
their points, and moves when they change. That move is the step the owner sees, and the
reason why 4x the candidates helped less than 2x the history.

The emissive part is static light. Its history dies today only in edit boxes and, for 11
frames, after an emission-scale change. The 32-frame cap guards one further case: light that
changes outside an edit box, as when a dig opens a vein onto a far wall. At 32 frames that
light reaches 90 % in about 73 frames, and at 256 frames in about 589. The accumulation is
`rgba16float`. A blend of `1/H` stores no change while a sample lies within about
`H x 2^-11` of the history: 1.6 % at 32 and 12.5 % at 256. Plan 0040 also rescales that
history by the exposure ratio every frame. A long history in half precision therefore stalls
away from its mean.

## Decision

**The emissive part of the temporal pass blends at `max(1 / (age + 1), 1 / H)` with `H` long
(chosen by Plan 0047's measurement from 64 to 512), into a full-precision (`f32`)
accumulation. The A-SVGF gradient pass runs whenever emissive light is on and writes an
emissive-only lambda with a dead zone, which shortens the emissive history where re-shading
last frame's kept point says its light changed.**

- The gradient pass already re-shades one pixel per 3 x 3 stratum at last frame's hit with
  last frame's kept emitter point, its current radiance and its current visibility. It now
  also writes that emissive re-shade's lambda alone, `|e_new - e_old| / max(e_new, e_old)`
  over the emissive luminance, beside today's combined lambda. Below a dead zone it reads 0.
  The dead zone sits above the bfloat16 W rounding of about 0.004, so a still scene keeps its
  whole history.
- The temporal pass raises the emissive blend to that lambda, averaged over the 3 x 3 strata
  as today. The lamp's part keeps its own rules (alpha 0.2 and the lamp-change reset), so a
  moving headlamp never shortens the static light's history.
- The pass runs in the shaded view when emissive candidates are above 0, whatever
  `HistoryKill` says. `HistoryKill::Gradient` keeps its meaning for the lamp's part.
- The reservoir keeps storing its kept point whatever the shadow ray said. That is deliberate:
  a point behind rock that a dig exposes is exactly what the guard re-shades to catch the
  reveal. A reservoir that zeroed its occluded points would hide the reveal from the guard.
- The emissive reuse cap (`REUSE_M_CAP`) and `H` are set together by Plan 0047's sweep against
  its patch bar. The cap may fall, since a shorter hold on a kept point decorrelates the
  window.

## Consequences

### Positive
- Static light holds still. A wall's emissive irradiance averages `H` frames' samples, not 32,
  and the f32 store keeps the mean where the samples put it.
- No new sampler, table or world-space structure. The cost is one f32 target pair and the
  gradient pass's one shadow ray per 9 lighting pixels.
- An undetected change is bounded by `H`, and a detected one (the reveal, an emitter dug
  away) follows at the gradient's pace.

### Negative
- The guard sees only what last frame's kept points see. A change to light that no
  reservoir's point covers is caught by nothing but `H`: at 256 that is about 10 s to 90 %,
  where today it is about 1.2 s.
- A newly seen surface still starts from its first noisy frames. Disocclusion, a turn or a
  dig converges at `1 / (age + 1)` as today, so motion looks no better.
- The gradient pass becomes part of every emissive frame: one more dispatch and shadow ray,
  read on both adapters by Plan 0047.
- The accumulation doubles its bytes: 16 B a lighting pixel per copy, not 8.

## Alternatives considered

### Alternative A: a world-space emissive cache
Accumulate the emissive estimate per exposed voxel face (or per hashed surface cell) in a
table that outlives the view, and read it back at each hit. Static light then converges once
and stays put through turns and disocclusions. This is the principled fix, and it handles
motion too. It lost on size and risk for the problem measured. It needs a new VRAM structure
and fixed-point atomics in WGSL. It needs a cell size that agrees with the contour surfaces
at 10 cm. Its invalidation is the same unsolved visibility question (which cells a dig
relights) that the gradient answers in screen space today. The steady-state patches are a
still-camera problem, which a longer guarded screen-space history reaches. If Plan 0047's
owner check finds motion the remaining fault, this is its successor.

### Alternative B: lower the estimator's variance at its source
Use more candidates, tables built per region around the hit rather than around the camera,
visibility in the resampling target, or a light tree, so each frame's sample is better. The
measurements argue against it as the fix. Four times the candidates cut the patches from about
70 to 43 at four times the candidate cost, while doubling the history (free) cut them to 32.
Per-region tables help the far, occluded candidates without making static light static.
They stay open as a cost-quality knob for a later plan.

### Alternative C: kill the emissive history on any edit near an emitter
Keep a long history and, instead of a gradient, raise the "static light changed" bit (0.2
blend for 11 frames) for the whole frame whenever an edit lands within the light radius.
It costs nothing. But every dig near a vein would make the whole screen's emissive light
boil for a fifth of a second, and a change with no edit (an emitter streamed in, a far edit
by another client) would still lag by `H`.

## Outcome

_(Added at acceptance if the implementation falsifies something above.)_
