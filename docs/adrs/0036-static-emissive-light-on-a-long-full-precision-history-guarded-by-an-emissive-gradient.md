# ADR-0036: Static emissive light accumulates on a long full-precision history, guarded by an emissive-only gradient

> **Status:** accepted (2026-10-29, Plan 0047 closed; see Outcome)
> **Date:** 2026-10-27
> **Related plan(s):** [Plan 0047](../plans/done/0047-emissive-light-holds-still-long-guarded-history.md);
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
- It does not reach indirect light (added 2026-10-28). The probe rays take their own
  emissive sample (ADR-0023), and Plan 0047 Phase 1 found the probe gather making most of the
  patches at one vein pose, untouched by `H`. Plan 0047 Phase 4 fixes that within ADR-0023's
  design. A probe mechanism beyond it is a decision of its own, not part of this one.

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

**Accepted 2026-10-29 at Plan 0047's close (v0.36.0).** The decision shipped as written, plus
one change to the probes that it did not foresee.

- **What shipped.** The temporal pass's accumulation pair is `rgba32float`. On a 2048-frame
  replay at `H` 512 it tracks an `f64` replay of the blend rule within 0.0001 %. The
  `rgba16float` store drifted 13.92 % on the same replay. The gradient pass writes an
  emissive-only lambda in the strata's `w` with a dead zone of 0.05 (`LAMBDA_FLOOR`), and runs
  whenever emissive candidates are on (`Params::lighting` bit 10, `LIGHTING_EMISSIVE_GUARD`).
  The temporal pass raises the emissive blend and lowers the age by that lambda. The lamp's
  part never reads it. `Lighting` gains `reuse_m_cap` and `emissive_guard`. The fly-through
  gains `--emissive-guard` and `--reuse-m-cap`, and its banner names both.
- **The defaults.** `DEFAULT_EMISSIVE_HISTORY` 256 (was 32), `DEFAULT_REUSE_M_CAP` 20
  (unchanged), the guard on, the `f32` store. Plan 0047's sweep of `H` 64 to 512 against cap
  20, 8 and 4 picked them: 256 is the shortest `H` that meets the bar at both vein poses, and
  20 is the highest cap at 256. At the defaults the patch pose reads 1.50 composite patches a
  step (0.75 direct-attributed), against 74 to 78 at 32. `VEIN_POSE` reads 0.05. The guard
  brings a whole reveal behind a one-voxel plug to 90 % in 1 frame (430 with the guard off).
  A partial opening is slower: 222 frames for a 10 x 10 window. The owner accepted that within
  the 589-frame bound of `H` 256, so Alternative C stays unbuilt.
- **The probes, outside this decision.** Plan 0047 Phase 1 found the probe gather (ADR-0023)
  making 83 % of the patches at `VEIN_POSE`, in whole-screen steps. Their cause was the slice
  cadence: a slice took every slot with the same `slot mod slices`. The fix stays inside
  ADR-0023's design. The slices are now spread by a 15-bit bijective hash of the slot
  (`probes::slot_slice`), which takes the gather at `VEIN_POSE` to 0 patches a step. The patch
  pose keeps about 8 gather patches a step. Two further mechanisms were measured and rejected:
  - **An `f32` irradiance atlas with a settled hysteresis above 0.97.** At 0.99 it stilled the
    gather (0.30 patches a step). But a reveal took about 1831 frames to reach 90 %, the furnace
    no longer converged by 256 frames (texel mean -4.59 % against a 4 % bar), and the atlas
    would have cost 16 777 216 B. At 0.97 it moved nothing (under 1 %).
  - **A per-ray clamp on the probe rays' emissive term.** It biased the gather's mean 38 to 53 %
    low, against a 3 % bar.
- **The furnace test, rebased.** The probe furnace no longer judges against pi. Its fixed point
  is the probe rays' own direct mean `D` at their hits. The test measures `D` with the centre
  probe's tile zeroed (7.7 to 8.4 % under pi, because the firefly clamp of ADR-0018's estimator
  biases those hits low) and holds every texel and the floor within 7 % of `D` and the texel
  mean within 4 % at 256 frames. Plan 0048 takes the clamp's bias and decides whether the test
  returns to pi.
- **Measured cost** (`nfr.md` §2 and §3, Plan 0047 Phase 6). The gradient pass costs 0.083 to
  0.085 ms on the RTX 3080 Laptop and 0.454 to 0.462 ms on the Radeon (Cezanne, RADV). The
  hashed slices add +4.20 % to the probe update on the Radeon with the lamp off, read quiet,
  against a 5 % bar. `Frame::frame_bytes` grows by 3 686 400 B at the default, exactly the 16 B
  a lighting pixel the Consequences named.
- **What it did not fix.** The owner's windowed check (Phase 7) chose outcome (c): "much
  better, but still bad". Still walls hold, and two faults remain. Indirect light takes
  seconds to settle after a move into a new area or a lamp toggle. Small light dots show on a
  fast turn or a dig, the same as with the old 32-frame look. The second is the motion case
  the Negative consequences predicted. Plan 0049 takes both as a measure-first plan. Any
  mechanism beyond this decision or ADR-0023 is a new ADR there, with Alternative A among its
  options.
