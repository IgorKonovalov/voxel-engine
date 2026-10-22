# ADR-0024: The upscaler is a temporal upscaler of our own, driven by jittered rays and the hit buffer, with a sharpening pass

> **Status:** accepted (2026-10-22, at Plan 0030's close; see Outcome: shipped opt-in, not as the
> default)
> **Date:** 2026-10-21
> **Amended:** 2026-10-22 (owner, at Plan 0030's Phase 2 stop, while proposed): Plan 0024's
> lighting passes become jitter-aware, and history is accepted from a 3 x 3 neighbourhood.
> Amended again 2026-10-22 (owner, at Plan 0030's windowed check, while proposed): the image
> read blurry, so the history fetch, the reconstruction kernel, the blend weight and the
> still-pixel clip are reworked by the plan's Phases 6 and 7 before the close.
> **Related plan(s):** [Plan 0030](../plans/done/0030-temporal-upscaler.md)

## Context

`docs/nfr.md` §2 budgets 60 fps at 1080p output on the floor "with the upscaler allowed
(internal resolution between 720p and 1080p)", and gives upscale, tone map and post 1.5 ms.
Every lighting figure in Plan 0024 projects over the floor's lines at full resolution and
under them at half, so the upscaler is what lets the floor render fewer primary rays and
still fill the screen. The roadmap's M2 names "an open-source upscaler ported to WGSL" with a
TAA upscaler of our own as the fallback.

The facts that decide it. FSR 2 (`docs/references/voxel-optimizations.md`) is MIT, proven and
under 0.6 ms at 1080p on an RX 5700 XT, but it is about ten HLSL passes with their own
luminance pyramid, lock and depth-clip stages, and it wants motion vectors, depth, a reactive
mask and exposure as inputs built to its conventions. No WGSL port exists. A ray marcher has
what a temporal upscaler needs for free: the hit pass can jitter each pixel's ray by a
sub-pixel offset at no cost (there is no rasterised geometry to re-sample), and Plan 0024's
hit buffer holds the exact world position of every pixel, so reprojection by hit position
through the previous view is an exact motion vector for a static world. Plan 0024's temporal
pass already reprojects that way, and keeps history per view (VR-readiness decision). FSR 1's
RCAS sharpening is one function, MIT, and ports in an afternoon.

## Decision

**The frame renders at an internal resolution and a temporal upscaler of our own produces the
output.** The internal scale is a quality setting (1.0, 1.5 or 2.0 per axis; 1.5 is 720p for
1080p). The hit pass offsets each pixel's ray through a per-frame sub-pixel jitter (Halton 2,
3, sixteen phases) that every view carries. The direct light and denoiser passes of Plan 0024
run at internal resolution on the jittered hits and are made jitter-aware (amended
2026-10-22, owner): the temporal pass reprojects as if every ray went through its pixel's
centre and accepts history from the 3 x 3 previous pixels around the reprojected one, the
a-trous filter stops on inputs the jitter does not move, and the half tier's sample pixel
accounts for the jitter. An upscale pass at output resolution, per
view, reprojects each output pixel's previous output through the previous view by the nearest
internal hit's world position, rejects the history when none of the 3 x 3 previous internal
pixels around it agrees in depth, normal and material within thresholds, or the hit is inside
an edit box, clips the accepted
history to the YCoCg bounding box of the current 3 x 3 internal neighbourhood, and blends it
with the current samples weighted by a kernel of each jittered sample's distance to the
output pixel centre (1.0 on a reject). Amended 2026-10-22 (owner, windowed check): the history
is fetched with a Catmull-Rom filter rather than bilinearly, the kernel is a narrow
Blackman-Harris-like window over the 3 x 3 samples rather than Lanczos-2 over 5 x 5, the blend
weight falls with the samples' distance from the centre rather than staying at a fixed 0.1,
and a change of light under a still camera is rejected rather than left unclipped; Plan 0030
Phases 6 and 7 set the values by measurement. A sharpening pass (RCAS, ported from
FSR 1 with its licence notice) runs on the result, before the HUD. History is one output-
resolution colour texture per view plus the previous view.

We rejected porting FSR 2 (ten passes and their input conventions before any image, for a
pipeline whose motion vectors and depth it would have to be fed from the hit buffer anyway), a
spatial-only upscaler (EASU plus RCAS: no sub-pixel information, so edges stay at internal
resolution), and vendor upscalers (DLSS, XeSS, FSR 3 and 4 need a vendor path, which ADR-0001
rules out).

## Consequences

### Positive
- The floor renders 44 % of the primary rays (720p for 1080p at scale 1.5) and the lighting
  passes at the same fraction, which is the headroom every §2 lighting projection needs.
- The upscaler's inputs are what the engine has: jittered hits and exact reprojection. No
  motion-vector pass, no depth conventions, no reactive mask.
- Four shaders of ours, each testable headless on lavapipe with a golden and a still-camera
  convergence test against a 2 x 2 supersampled native render, the reference that scores
  resolved detail above copied aliasing (amended 2026-10-22, owner).
- Per-view history keeps the VR door open; the jitter sequence is per view.

### Negative
- **Our own artifacts to tune.** Ghosting on disocclusion, flicker on thin features and
  over-sharpening are the known failure modes of every TAA upscaler; the plan's tests bound
  them and the owner's eye is the last word. FSR 2 would have brought AMD's tuning.
- **The denoiser sees jittered samples.** The draft held that matching history by world
  position made Plan 0024's temporal pass indifferent to the jitter. Plan 0030 Phase 2
  measured otherwise (amended 2026-10-22, owner): at scale 1 the jittered image sat 50 %
  farther from a supersampled render than native did with the half tier and the a-trous on,
  3 % with both off, so the lighting passes are changed to ignore the jitter (above). If that
  fails, the jitter amplitude is a parameter down to zero (then the upscaler is spatial).
- **Translucent layers and the HUD.** The translucent composite happens at internal
  resolution and is upscaled with everything else; the HUD draws at output resolution after
  sharpening.
- **A moving entity (M6)** reprojects wrongly until entities write motion vectors, the same
  debt Plan 0024's temporal pass records.

## Alternatives considered

### Alternative A: port FSR 2 to WGSL

The best image of the three and a known cost on the floor's class. It lost because the port is
plan-sized work before anything is seen, its inputs (motion vectors, depth in its conventions,
a reactive mask, exposure) have to be derived from the hit buffer by passes of ours anyway,
and its licence requires carrying its notices through a translation whose fidelity nobody can
check against AMD's tests. If our upscaler misses the bar on the owner's eye, this is the next
plan and supersedes this clause.

### Alternative B: spatial upscaling only (EASU plus RCAS)

Two small passes, no history, nothing to ghost. It lost because a spatial filter cannot
recover the sub-pixel detail the jittered rays provide, so voxel edges and the contour planes'
silhouettes stay at internal resolution, which is the first thing the eye sees at arm's
length (`project-context.md`, Rendering: stable at close range).

### Alternative C: render at output resolution and lower the lighting tier instead

Keep every primary ray at 1080p and spend the saving on the floor through Plan 0024's half-
resolution lighting alone. It lost on arithmetic: the hit pass itself projects at 3 to 5 ms on
the floor at 1080p with contour, so without fewer primary rays the floor's 16.6 ms has no room
for the indirect, entity and volumetric lines.

## Outcome

**2026-10-22, Plan 0030's close (v0.28.0): our own upscaler shipped, opt-in, not as the
default.** The decision above was built as written, with the 2026-10-22 amendments: a 3 x 3
Gaussian reconstruction of width 0.6 output pixels, a Catmull-Rom history fetch, a blend
weighted by the nearest sample's nearness (0.1 still, 0.2 where the history moved), and a
still-pixel clip that fires where the lamp's term at the hit changed over the last 24 frames.
Measured, headless on lavapipe at 384 x 216 output unless stated:

- **Sharpness (stripe wall, sharpening 0, arm's length):** edge spread at scale 1 1.192 px
  against native's 0.798 and the 2 x 2 supersampled reference's 1.186; at scale 1.5 1.252 px
  against the bilinear blit's 1.718. Panning at 0.37 px a frame: 1.436 px (scale 1) and
  1.780 px (1.5), against 2.020 and 2.404 for the first build (Lanczos-2, bilinear fetch).
- **Still camera:** 2.840/255 from the supersampled reference against the bilinear blit's
  3.044 and plain native's 2.066.
- **Light changes:** a sweeping headlamp lags 5.82 (scale 1) and 5.92 (1.5) frames against
  Plan 0024's frame's 5.38; a lamp step settles in 18 frames, as Plan 0024's frame does.
- **Time (Plan 0030 Phase 8, 1080p output, render code 12c6c57, exclusive perf lock, 1-minute
  load 2.3 to 4.5: loaded, indicative):** RTX 3080 Laptop at scale 1.5: upscale, sharpen and
  composite 0.755 to 0.770 ms against the 1.5 ms line, frame 2.727 to 2.780 ms against 5.294
  to 5.546 at scale 1 with the jitter on. RADV RENOIR iGPU: frame 18.797 to 18.865 ms at 1.5
  against 38.011 to 38.246 at 1; the upscale pass alone 3.2 ms. `frame_bytes` 124 621 440 B
  at scale 1.5. The readings are in `docs/nfr.md` §2 and §3.

The owner's windowed check (Plan 0030 Phase 9, RTX 3080 Laptop) read the first build blurry
and the reworked one better but not as good as Plan 0024's frame ("some ghosting ... maybe it
is fine as a part of visual style"; scale 1 jittered "meh"; Plan 0024's frame "almost fine";
"too much shadow motion" in face mode). His ruling: the default is Plan 0024's frame (scale
1, jitter off, the upscaler not dispatched; the library's `Frame` defaults to the same), and
the upscaler is a performance setting (`--internal-scale 1.5 --jitter on` in the fly-through,
`Frame::with_sizes` and `Frame::set_jitter` in the library). The consequence the Positive list
counted on, the floor rendering 44 % of the primary rays, is therefore available but not the
default. Alternative A, an FSR 2 port, is taken up by ADR-0034 (proposed) and Plan 0041; if
accepted it supersedes this decision's choice of upscaler.
