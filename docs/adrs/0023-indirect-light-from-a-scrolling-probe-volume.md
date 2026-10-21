# ADR-0023: Indirect light comes from a scrolling volume of irradiance probes around the camera, refreshed by a few rays per probe per frame and gathered with visibility weights

> **Status:** proposed
> **Date:** 2026-10-21
> **Related plan(s):** [Plan 0029](../plans/0029-indirect-light-probe-volume.md)

## Context

The consumer's P0 bar is a dark cave with few lights (`project-context.md`, Rendering). Plan
0024 (ADR-0018) lights each pixel from one direct sample, so after it every surface that faces
no light is black: the far wall of a cavern lit by a headlamp, the underside of a ledge, the
rock around a glowing vein's corner. Indirect light is what makes a lit cave readable, and it is
the half of the Sibenik reference (`docs/roadmap.md`, M2) that the baked occlusion carried.

Three decisions already constrain the mechanism. The floor decision of 2026-10-17 (roadmap,
"Floor, profiles and scene scale") says the lighting tiers differ in their direct light and
take "indirect light from probes in both". The VR-readiness decision asks that lighting be
view-independent or shareable across views where the cost is equal. ADR-0018 rejected a
per-voxel irradiance cache on memory (0.5 to 1 GB streamed at our surface density) and
rejected cone tracing because the mirror holds no mips for full chunks.

The cost of a ray is settled: a full-walk ray reads about 0.9 ms per million at 1080p on the
dev box (Plan 0015's 1.95 ms for 2.07 M primary rays with their shadow rays), and the coarse
shadow walk of Plan 0024 Phase 4 is cheaper. The §2 lighting line is 6.0 ms, of which Plan
0024's direct light projects 1.5 to 2.5 ms on the dev box at full resolution. A per-pixel
diffuse bounce is one more full-walk ray and one more shadow ray per pixel: 2 to 3 ms on the
dev box, 5 to 9 ms projected on the floor, before its own denoising; and it is per view.

A probe volume's cost is independent of the view count, the light count and the resolution.
DDGI (Majercik et al. 2019, `docs/references/voxel-optimizations.md`) refreshes a grid of
probes with a few rays each per frame and gathers with a visibility test (mean and squared
distance per direction, Chebyshev) that stops light leaking through walls, which is the failure
every probe scheme has in a cave. The engine already has the inputs: the hit buffer gives the
gather its position and normal, the direct estimator shades a probe ray's hit exactly as it
shades a pixel, and the edit results say which probes a dig invalidated.

## Decision

**Indirect light is a grid of irradiance probes that scrolls with the camera.** The grid is
32 probes on a side at a spacing of 16 voxels (1.6 m), a 51.2 m cube, anchored to the camera
in whole probe steps so a scroll moves no probe and only the probes that enter are new (they
start at zero weight). Each probe stores an octahedral irradiance map of 6 x 6 texels and a
visibility map of 14 x 14 texels (mean distance and its square), each with a one-texel border,
in two texture atlases, `rgba16float` and `rg16float`. Every frame, one eighth of the probes
in round robin plus every probe whose cell an edit box of the frame touches trace 64 rays each
(a spherical Fibonacci set rotated per frame by blue noise) through the full walk; a ray's
radiance is the hit's emission plus the direct estimator at the hit (headlamp by next-event
estimation and one emissive sample, with coarse shadow rays, as in Plan 0024's light pass)
plus the hit's albedo times the previous frame's probe irradiance there, which gives multiple
bounces at the refresh rate. A ray that hits a back face records a negative distance; a probe
with more than a quarter of its rays on back faces is marked inside rock and gathers with
zero weight. Updates blend into the probe with hysteresis 0.97 for irradiance and 0.97 for
visibility, dropped to 0.5 for a probe an edit touched. A gather pass reads, for each hit
pixel, the eight surrounding probes with trilinear, normal-cosine and Chebyshev visibility
weights, and writes an indirect irradiance texture that the composite adds to the direct
irradiance before multiplying the albedo back. The volume is one per world, shared by every
view, and it runs identically in both lighting tiers.

## Consequences

### Positive
- The cost is bounded and small: 4096 probes times 64 rays is 262 144 full-walk rays and about
  twice as many coarse shadow rays per frame, about an eighth of a 1080p primary pass; the
  gather is one pass of texture reads per hit pixel. It does not grow with lights, views or
  output resolution.
- View-independent: two VR eyes gather from the same volume at no extra cost, which is the
  door the VR-readiness decision keeps open.
- No screen-space history, so no ghosting or boiling from the indirect term; the a-trous
  filtered direct light carries the contact shadows.
- Edits refresh exactly the probes they touched, from a result the engine already returns
  (`EditResult.aabb`), so a dug-through wall lights its new room within a few frames.
- Memory is 32 768 x (8 x 8 x 8 + 16 x 16 x 4) B, about 50 MB, plus a state byte per probe,
  against the §3 "everything else" line of 1.5 GB.

### Negative
- **Spatial resolution is the probe spacing.** Indirect light has no detail finer than 1.6 m;
  the darkening in a 30 cm crevice comes from the direct term and the gather's normal weight,
  not from the probes. If the owner's eye asks for finer, the spacing halves and the cost
  grows eightfold per axis halving; a finer inner cascade is the next step, not a redesign.
- **Latency.** A probe refreshes every eight frames and settles over about 30 refreshes at
  hysteresis 0.97: a headlamp's indirect trail lags about half a second. The direct term does
  not lag.
- **Probes in rock.** A cave puts most probes inside solid. The inside flag and the Chebyshev
  test keep them from leaking, but a room narrower than the spacing can have no live probe in
  it and gathers from the nearest live ones through the visibility weight, which darkens it.
  The plan's leak and furnace tests are the bar; relocation (DDGI's probe offsets) is the
  fallback if a listed scene fails.
- **The mirror window.** Probe rays walk the same mirror as the camera's rays; a probe near the
  window's edge sees unfilled chunks as air and lights through them for that frame. The scroll
  volume is inside `r_full` at the defaults (25.6 m against 102 m).

## Alternatives considered

### Alternative A: one diffuse bounce per pixel through Plan 0024's denoiser

A cosine-weighted ray from each hit, shaded at its hit by the direct estimator, accumulated
and filtered as a second channel of the SVGF pipeline, with ReSTIR GI as the quality lever.
Sharper than probes: contact darkening and colour bleed at pixel scale. It lost on the floor's
arithmetic (two more rays per pixel at full resolution, so half resolution on the floor with
its own history), on view count (per eye), and on the ghosting a screen-space history adds to
a term that is already dim and noisy. It stays the quality mode to revisit once the probes
are on screen, and Plan 0024's hit buffer is all it needs.

### Alternative B: voxel cone tracing over a near-field mip pyramid

Crassin's cones through a pyramid of occupancy and radiance. The mirror holds mips for
LOD-only chunks only; building a radiance pyramid for full chunks is a storage plan of its
own (a radiance per mip cell, re-lit on every edit and light change), and cones leak through
thin walls at the coarse levels, which a cave is made of. Rejected, as in ADR-0018.

### Alternative C: a per-voxel irradiance cache

Rejected in ADR-0018 on memory and lag; the reasons hold for the indirect term.

### Alternative D: radiance cascades

Cost independent of light count and no probe placement problem, but the published form is
for the far field and large penumbrae, and no implementation exists in WGSL to measure.
Worth a look for the far-field clipmap, not for the near-field cave.

## Outcome

_(Added at acceptance, from Plan 0029's readings and the owner's windowed check.)_
