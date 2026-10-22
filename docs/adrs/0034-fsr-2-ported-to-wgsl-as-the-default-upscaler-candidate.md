# ADR-0034: AMD FSR 2.2.1 ported to WGSL is the upscaler's default candidate; our own upscaler stays opt-in

> **Status:** proposed
> **Date:** 2026-10-22
> **Related plan(s):** [Plan 0041](../plans/0041-fsr2-port-to-wgsl.md)
> **Supersedes in part:** [ADR-0024](0024-own-temporal-upscaler-on-the-hit-buffer.md): its
> rejection of FSR 2 (its Alternative A) and its claim to be the M2 upscaler. Its jitter,
> internal resolution, jitter-aware lighting and RCAS clauses stand and are reused.

## Context

ADR-0024 chose a temporal upscaler of our own and named an FSR 2 port as the next plan "if our
upscaler misses the bar on the owner's eye". Plan 0030 built it and ran two rework rounds
(its Phases 6 and 7). What it measured, on its stripe-wall rig at arm's length (lavapipe, 384 x
216 output): still, it meets its sharpness bars (edge spread 1.192 px at scale 1 and 1.252 at
1.5, against native's 0.798). Panning, edges soften to 1.436 px at scale 1 and 1.780 at 1.5.
It halves the frame: RTX 3080 Laptop 2.73 to 2.78 ms at scale 1.5 against 5.29 to 5.55 ms at
scale 1, and RADV iGPU 18.8 against 38.0 ms at 1080p. Its readings live in Plan 0030's
Implementation log. Jitter moves the per-pixel shadow rays, so shadows shimmer.

The owner's windowed check (2026-10-22, Plan 0030 Phase 9) gave these verdicts. Scale 1.5:
"some ghosting, but not as bad, maybe fine as a part of visual style". Scale 1 with the jitter
on: "meh". No upscaler: "almost fine". Scale 1.5 in the face look: "meh, too much shadow
motion". He ruled that Plan 0030 closes with Plan 0024's frame (scale 1, jitter off) as the
default and its upscaler opt-in, and that an FSR 2 port is the next plan.

What decides the shape of the port. AMD FidelityFX Super Resolution 2 is MIT ("Copyright (c)
2022-2023 Advanced Micro Devices, Inc."). Its last standalone release is v2.2.1 of
`GPUOpen-Effects/FidelityFX-FSR2`, and it ships each pass in both HLSL and GLSL. The upscaling
path runs six passes in order: compute luminance pyramid, reconstruct previous depth (with
dilated motion and depth), depth clip, lock, accumulate (optionally with sharpening) and RCAS.
It keeps about twenty internal resources, and it takes colour, depth, motion vectors, exposure,
a reactive mask and a transparency-and-composition mask. Every one of those inputs can be
derived from Plan 0024's hit buffer and Plan 0030's reprojection. Four of its GPU habits have
no portable WGSL form: image atomics (`InterlockedMax` on an `R32_UINT` texture), a
`globallycoherent` storage texture with a last-workgroup tail in its single-pass downsampler,
16-bit and 8-bit storage formats (`R16G16_FLOAT`, `R16_FLOAT`, `R8_UNORM`, `R11G11B10_FLOAT`,
none of them WGSL core storage formats), and optional FP16 and wave operations. ADR-0001 rules
out vendor paths and backend branches. FSR 2 is a set of portable compute shaders, so a port
is not a vendor path.

## Decision

**We port FSR 2.2.1's upscaling path to WGSL in `vxe-render`, and it becomes the upscaler's
default candidate.** Everything in the port is decided by the portable WGSL feature set. Only
the FP32 path is ported, with no `SHADER_F16`. Only the no-wave-operation paths are ported,
with no `SUBGROUP`. Image atomics become `atomicMax` on a storage buffer indexed by pixel.
Read-write resources become storage buffers. A 16-bit or 8-bit storage format becomes the
nearest core format that is at least as precise (`rg32float`, `r32float`, `rgba8unorm`). The
downsampler's last-workgroup tail becomes a second dispatch. FSR 2's own reactive autogen and
auto-exposure are not ported. The engine feeds FSR 2 these inputs:

- linear HDR colour from the composite, before tone mapping;
- reversed, infinite device depth computed from the hit distance;
- unjittered motion vectors from Plan 0030's centred reprojection;
- a 1x1 exposure: the frame's manual exposure, or eye adaptation's value once Plan 0040 lands.
  ADR-0033's pre-exposure becomes FSR's `preExposure`, so FSR's history rescale is the one
  that ADR asks of every history-keeping pass;
- a reactive mask built from what the engine knows: translucent layers, edit boxes and Plan
  0030's light-change signal;
- the transparency-and-composition mask, written as zero, which is the seam for liquid
  surfaces.

Tone mapping moves after the upscaler, so the HUD draws after the tone pass. Each view keeps
its own FSR 2 context. The edit boxes force history off where the world changed. That rule is
an engine input fed through the port's depth-clip term, not a change to AMD's arithmetic, and
the source marks it.

Our own upscaler (ADR-0024, Plan 0030) stays selectable beside FSR 2. **The default is not
decided here.** Plan 0041's owner comparison chooses among Plan 0024's frame, our upscaler and
FSR 2. If FSR 2 does not win, it ships opt-in and this ADR's Outcome says so. When this ADR is
accepted, ADR-0024's status line reads "accepted; superseded in part by ADR-0034". Plan
0030's close writes ADR-0024's own Outcome (opt-in, default scale 1 with the jitter off).

## Consequences

### Positive
- AMD's motion handling replaces our own. FSR 2 adds locks on thin features, a Lanczos
  upsample with a bias table, history weighted by depth clip and luminance stability, and a
  shading-change term from its luminance pyramid. These target exactly where Plan 0030 fell
  short: softness while panning, and history that has to be retuned for every light.
- The inputs come from what the engine already has. Depth and motion are exact for a static
  world, from the hit buffer, and the reactive mask carries edits and lights from the passes
  that already know about them.
- The FSR path stays portable. One WGSL source per pass, no feature beyond Plan 0024's, and
  every pass is testable headless on lavapipe.
- Plan 0030's rigs (stripe wall, carve, headlamp sweep and step, flicker, wobble) compare the
  two upscalers in one run on one tree.

### Negative
- **The port is the largest render work since Plan 0024.** It covers six AMD passes, two passes
  of ours (input and tone), about twenty resources and the AMD maths in the shared headers
  (`ffx_fsr2_*.h`, `ffx_core*.h`, `ffx_spd.h`). Plan 0041 Phase 1 counts the source lines; until
  then the size is a guess.
- **Fidelity cannot be proven against AMD's own tests.** The port is checked by its
  behavioural bars and by reading the source, not by bit-matching it. A transcription error
  shows up as an image artifact before it shows up as a failed assertion.
- **VRAM.** Widening the storage formats costs about 30 MB at 1080p output. The FSR resources
  plus our inputs come to about 150 MB at scale 1.5, against our upscaler's 33.2 MB of history
  (arithmetic in Plan 0041, Data shapes, not a reading).
- **The slow adapter may lose part of the halving.** We have no FSR timing on the RADV iGPU,
  and FP32 without wave operations is AMD's slowest path. Plan 0030's upscale pass already
  reads 3.2 ms there.
- **Mismatched assumptions.** FSR 2 assumes a symmetric projection (`cameraFovAngleVertical`),
  which a flat screen gives and an asymmetric VR view does not. It also assumes one depth per
  pixel, so a translucent layer reaches it only through the reactive mask.
- **The notice travels.** Every ported file carries AMD's MIT notice, as `sharpen.wgsl` does for
  FSR 1.
- **Two upscalers to maintain** until the owner's comparison retires one. Plan 0041 does not
  remove either.

## Alternatives considered

### Alternative A: keep tuning our own upscaler

The cheapest next step: a third rework round on `upscale.wgsl`. It lost because Plan 0030's
Phase 6 readings show the panning softness is structural. With history almost off while the
camera moves, edge spread still read 1.752 px at scale 1.5, because one upscaled frame is softer
than the bar. The two remaining levers, locks on thin features and a history weight driven by
disocclusion and shading change, are what FSR 2 already implements and AMD has already tuned.
Rebuilding them is the port without the reference.

### Alternative B: FSR 3 or FSR 4, DLSS, XeSS

FSR 4 is a neural network that runs only on RDNA 4. DLSS needs NVIDIA's runtime. XeSS ships as
a closed library, even on its cross-vendor DP4a path. All three are vendor paths, which ADR-0001
rules out, and none of them runs on lavapipe. FSR 3's upscaler is FSR 2's algorithm, revised,
and is also MIT, but it ships inside the FidelityFX SDK's backend framework next to frame
interpolation, and that framework is the part a port would have to unpick. We take 2.2.1, which
is self-contained, with GLSL that maps onto Vulkan compute line by line. Moving to 3.x's
upscaler later is a diff against a port that already works.

### Alternative C: a temporal upscaler from another engine

Unreal's TSR and TAAU are under the Unreal EULA, which is not an open licence. Bevy's TAA is
WGSL and MIT/Apache, but it anti-aliases at native resolution. It does not upscale, and it is
the same family as Plan 0030's pass, so it adds nothing we lack. Godot ships an FSR 2.2
integration, but it is FSR 2 again (GLSL, not WGSL). Plan 0041 may read it as a second view of
AMD's source, but there is nothing to adopt from it.

### Alternative D: spatial-only FSR 1 (EASU plus RCAS)

Two small passes with no history, so nothing to ghost and no jitter to make shadows move. That
answers the owner's "too much shadow motion" directly. It lost again for ADR-0024's reason (its
Alternative B): no sub-pixel information, so voxel edges stay at the internal resolution. The
owner's "almost fine" for the unupscaled frame says the edge detail is what he is protecting.
For the cost of one bilinear-class pass, it is not worth a mode.

## Outcome

_(Added at acceptance, from Plan 0041's readings and the owner's comparison.)_
