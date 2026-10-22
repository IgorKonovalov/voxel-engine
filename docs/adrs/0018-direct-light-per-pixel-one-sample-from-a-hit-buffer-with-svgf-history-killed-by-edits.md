# ADR-0018: Direct light is one sample per pixel from a hit buffer, denoised by an SVGF-family filter whose history dies where the world was edited

> **Status:** accepted (2026-10-22, at Plan 0024's close)
> **Date:** 2026-10-20
> **Related plan(s):** [Plan 0024](../plans/done/0024-direct-light-hit-buffer-emissive-samples-and-svgf-denoiser.md)

## Context

The consumer's P0 bar is a dark cave with few lights, emissive voxels as lights whose emission
changes at runtime, and denoising that survives the disocclusion a dig causes without heavy
ghosting. The engine's marcher today shades one analytic lamp inside the primary-ray kernel.

The cost of a ray is settled: a primary ray with its shadow ray reads 1.43 to 1.95 ms at 1080p
on the dev box (Plan 0015, Plan 0013; `docs/nfr.md` §2), and the baseline GTX 1660 class is
two to three times slower (a projection). The §2 lighting line of 6.0 ms buys about two
full-resolution rays per pixel on the baseline, or eight at half resolution. Any family that
needs more than one or two shadow rays per pixel per frame at full resolution misses the
floor.

The emissive light of this world is voxels: a crystal vein is thousands of emitters, and a dig
or a `Replace` op changes which ones exist from one frame to the next. The material table is
fixed after `World::generate` (ADR-0012), so emission changes by swapping a voxel's material
through the op log, or by a render-side per-material scale that is not authoritative.

The mirror holds mips only for LOD-only chunks (ADR-0003; `gpu_world.rs`), so a cone-tracing
or probe design has no pyramid in the near field without building one. The surface density
is high: the level alone holds 383 608 `Full` bricks, and a cache entry per surface voxel is
tens of millions of entries at the level and five times that streamed.

The roadmap's VR-readiness decision asks that lighting be view-independent or shareable
across views where the cost is equal, and that history-keeping stages keep it per view.

## Decision

**The frame lights each pixel from one sample and denoises it with an SVGF-family filter.** A
hit pass writes a per-pixel record (position, normal, material, plane flag) and no colour. A
light pass picks one light sample per pixel, the headlamp by next-event estimation and one
emissive voxel by resampled importance sampling over candidates drawn from a buffer of light
samples the mirror extracts at upload, with spatiotemporal reservoir reuse (ReSTIR) as one
mode and a plain candidate list as the other; it casts one shadow ray, through the full walk
or a coarse walk over the 2^3 blocks of the masks. A temporal pass reprojects by hit position
through the previous view (the world does not move; the camera and the edits do), accumulates
albedo-demodulated irradiance and its moments, and resets history in every pixel whose hit
lies inside one of this frame's edit boxes, dilated by one voxel; A-SVGF's gradient test is the
other mode. An a-trous wavelet filter weighted by variance runs four iterations; a composite
multiplies the albedo back, adds emission, and applies exposure and tone mapping. Lighting
runs at full or half internal resolution. History is kept per view. Which mode of each pair
becomes the default is the owner's on-screen call at the plan's close, recorded in this ADR's
Outcome; both modes stay in the code until then.

## Consequences

### Positive
- One shadow ray per pixel per frame fits the floor's arithmetic at half resolution and the
  dev box at full, and the many-light case (a vein) costs no more rays than one lamp.
- Edits invalidate exactly what they touched, from a result the engine already returns
  (`EditResult.aabb`), so a dug-out wall never shows stale light; the gradient mode catches the
  cases an edit box cannot (a moved lamp, a changed emission scale).
- The hit buffer is what every later stage needs (indirect light, the upscaler, entities), so
  the split is ordering, not extra work, and the primary kernel sheds its shading registers.
- Nothing per voxel is stored: the frame's buffers are about 276 MB at 1080p, independent of
  the world's size.
- The view list and per-view history keep the VR door open at no cost on a flat screen.

### Negative
- Screen-space history ghosts and boils when its tests are wrong; the plan's age, no-ghost
  and settle tests are the bar, and the owner's eye is the last word. A moving entity (M6)
  will ghost until it writes motion vectors.
- The filter is the budget's largest unknown: published SVGF reads about 10 ms at 1080p on
  2017 hardware near the baseline's class. If the dev box reads over the 3.0 ms line at full
  resolution, the floor runs lighting at half resolution.
- Two modes of three mechanisms are built and measured before one is kept; that is the price
  of deciding by on-screen results.
- The light is not view-independent, so two VR eyes pay twice for the light and filter passes;
  the per-voxel cache below is the VR-era answer if that ever matters.

## Alternatives considered

### Alternative A: A per-voxel irradiance cache

Light accumulated per surface voxel by a few rays per voxel per frame (DoonEngine; Minecraft's
light levels are the flood-filled ancestor), read by the pixel pass. View-independent and free
of screen-space ghosting. It lost on memory: at our density a 6-byte entry per surface voxel is
100 to 200 MB at the level and 0.5 to 1 GB streamed, on a mirror that already runs the
pressure rule near 0.9; and on lag, since a sweeping headlamp updates at the cache's refresh
rate. It stays the fallback for a view-independent future.

### Alternative B: A clustered light list with jittered shadow rays and a blur, no reservoirs

Teardown's recipe: emissive bricks clustered to centroids, `K` nearest lights, one jittered
shadow ray each at reduced resolution, a depth-and-normal-weighted blur plus temporal
reprojection. Proven on a GTX 1080 and the cheapest to build. It lost as the only sampler
because a vein of thousands of emitters becomes a handful of point lights, so large glowing
surfaces light coarsely and the choice of `K` pops under motion. It survives as the
flat-list mode of the chosen design, which is the same code with reuse off.

### Alternative C: An NRD-style recurrent blur (ReBLUR) or a ReLAX port as the filter

The production reference for 0.5 to 1 rays per pixel, with better stability than the
original SVGF. It lost for now because it is a larger design to port into WGSL before the
simpler filter has been measured against the 3.0 ms line; if four a-trous iterations do not
hold the dark cave, the port is the next filter plan and supersedes this clause.

### Alternative D: Voxel cone tracing over the mips

Rejected for this plan because the mirror has no mips for full chunks; building a near-field
pyramid is a storage decision that belongs to the indirect-light plan if it wants one.

## Outcome

Accepted 2026-10-22 at Plan 0024's close. **The owner's choice per mode pair** (Phases 12 and
15, live sessions 4 and 5, on the dev box): lighting resolution **half** (the only tier under
the lines; full looks sharper and stays a mode), shadow walk **coarse** (darker by contract,
cheaper by 0.4 to 0.6 ms at half; full stays a mode), sampler **reservoir reuse** (beats the
list at both tiers; the list stays a mode), history killer **edit boxes** with A-SVGF's
gradient kept as a mode (no visible difference on a still camera; it differs only when light
changes without an edit). All four modes stay in the code.

**What the implementation changed against the body above:**

- **Two histories, not one.** The owner saw static emissive light shimmer on a still camera
  under a single 0.2 blend (Phase 12). The light pass writes the emissive estimate and the
  lamp's apart, and the temporal pass blends the emissive part over at least
  `Lighting::emissive_history` frames (default 32) and the lamp's at 0.2; an emission-scale
  change raises the emissive blend to 0.2 for 12 frames. Edit boxes still reset both. The
  still vein's mean frame-to-frame step fell to 0.16 to 0.18 of 255 from 0.57 to 1.04.
- **The gradient reads the whole light,** so a lamp change where steady emissive light is
  strong resets only part of the lamp's history (all pixels by 7 to 9 frames, not 2; accepted
  by the owner). Separate lamp and emissive change values are a followup.
- **The half tier samples a fixed pixel of each 2 x 2**, not a cycle: the cycle moved a still
  camera's upsampled image.
- **The frame's targets** read 308.7 MB at full and 195.4 MB at half at 1080p, against the
  276 MB in Consequences (a translucent-layer target, the raw irradiance and the gradient
  strata were left out of the table).

**Measured lines** (dev box only, RTX 3080 Laptop GPU, 1080p, the defaults; `docs/nfr.md` §2
"The coarse shadow walk as the default" and "Quiet-box readings at the lighting defaults"):
light 1.97 to 2.81 ms, light plus temporal 2.11 to 2.95 ms against 6.0, a-trous 1.09 to 1.15
ms against 3.0. Full resolution misses: light 10.5 to 14.5 ms, a-trous 5.0 to 5.3 ms. No
baseline or iGPU reading.
