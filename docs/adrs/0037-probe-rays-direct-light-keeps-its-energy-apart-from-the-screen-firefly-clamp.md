# ADR-0037: The probe rays' direct light keeps its energy, under a scheme of its own apart from the screen's firefly clamp

> **Status:** proposed
> **Date:** 2026-10-28
> **Related plan(s):** [Plan 0048](../plans/0048-probe-rays-direct-light-without-the-clamp-bias.md);
> refines how the probe update of
> [ADR-0023](0023-indirect-light-from-a-scrolling-probe-volume.md) calls the emissive
> estimator of
> [ADR-0018](0018-direct-light-per-pixel-one-sample-from-a-hit-buffer-with-svgf-history-killed-by-edits.md),
> and reads the candidates of
> [ADR-0025](0025-light-candidates-from-a-power-over-distance-table-of-near-emitter-chunks.md)
> unchanged

## Context

The emissive estimator (`resolve_emissive` in `light.wgsl`) clamps one sample's luminance at
`FIREFLY_CLAMP` (2) times `PI lum(Le)` of the kept emitter. On screen pixels it is nearly
free: the light pass's reservoirs carry M up to 160 through reuse, so `w_sum / M` rarely
reaches the clamp, and the furnace floor centre's direct mean reads -0.025 % of pi (Plan 0047,
step C). The probe update calls the same estimator through `emissive_estimate`, with the 8
fresh candidates and no reuse, at hits spread over every wall, edge and corner a probe sees.
There a sample often exceeds the clamp. Plan 0047's step C measured the probe rays' direct
mean in the furnace box at D = 2.874, -8.5 % of pi. The box's fixed point is `E = D`, so the
furnace sits 7.5 to 7.8 % under pi, and Plan 0047 rebased its furnace test on D.

Removing the clamp is not a fix by itself. With `FIREFLY_CLAMP` at 1e6, the furnace's texel
mean over frames 1793 to 2048 read +6.3 % of pi. Its series sat between 3.04 and 3.15 up to
frame 1794, then jumped to 3.40 to 3.44 for the last three samples. The jump is one rare
event dominating a 256-frame mean. Area sampling a point on a unit emitter face gives a
weight of `cos cos / d^2` over a uniform area density. For a hit near the edge where its wall
meets an emitting wall, the second moment of that weight grows without bound as the hit nears
the edge (the architect's analysis; Plan 0048 Phase 1 reads the tail). The clamp is what
keeps the variance finite.

Plan 0047's step D measured a stronger clamp at the probe rays, one relative to the probe's
own irradiance. At `k` 8 to 2 it cut the gather's mean luminance by 38 to 53 % in the real
cave. So in a cave, much of the probe rays' direct light arrives in the tail that a clamp cuts.

The probes also average differently from pixels. A texel blends at hysteresis 0.97 over 64
rays a visit, so a single ray's spike reaches the screen as 3 % of a cosine lobe over about
half the texels. A pixel's spike reaches its history at the emissive blend directly, and the
a-trous filter spreads it into a tile. So the probes may tolerate a variance that screen
pixels cannot, and the scheme that suits one need not suit the other.

## Decision

**The probe rays' direct estimate is made to keep its energy in the mean, by a scheme chosen
for the probes alone. The screen pixels keep ADR-0018's estimator and its clamp of 2.** The
probe update passes its own setting to the estimator (`Lighting::probe_direct`, Plan 0048's
name, which dev may refine), and the light pass's path is bit-identical. The candidate
schemes are a separate probe clamp `k`, a clamp whose clipped energy is spread over the same
update, a clamp with a per-probe running gain that returns the clipped energy in the mean,
solid-angle sampling of the kept face (Urena, Fajardo and King 2013, "An Area-Preserving
Parametrization for Spherical Rectangles") under a clamp kept as a safety net, and more
candidates at probe hits. Multiple importance sampling with a short cosine ray is built only
if none of these meets Plan 0048's bars. Plan 0048 measures each against the bias in two
closed furnaces whose analytic answer is pi, the stillness of Plan 0047's rig and the probe
update's cost, and its decision rule picks the default. The schemes that meet the bars stay
selectable until the owner's windowed check. The default and the measured readings are
recorded in this ADR's Outcome at acceptance.

## Consequences

### Positive
- The furnace test can judge the probes against pi again, which is M2's headless proof
  ("irradiance in a closed test box matches an analytic value"), instead of against a mean
  the estimator's clamp sets.
- Indirect light in caves stops being darkened by an amount that depends on the geometry near
  each probe ray's hit (edges, corners, a vein in a crevice).
- The screen's direct light, its histories and Plan 0047's stillness readings for the
  emissive part do not move. Only the gather changes.

### Negative
- The estimator gains a second caller-specific path. The probe and screen readings of the
  same light now differ by design, and a later change to `resolve_emissive` has to be read
  at both callers.
- Every unbiased scheme adds variance or cost at the probe rays. More variance shows as
  gather patches in Plan 0047's rig, and more cost lands on the probe update pass. That pass
  is already 7 to 8 ms on the slow adapter, and the dev box reads 1.6 ms for update plus
  gather against the 1.7 ms line (`docs/nfr.md` §2).
- A running gain, if kept, returns the energy in the mean but not in its direction: a probe
  near a vein puts the clipped part into every texel it lights, by the share the clamped
  samples had.
- Indirect light gets brighter in every scene with emitters near walls, so every golden with
  the probes on is re-blessed, and the owner judges the new look.

## Alternatives considered

### Alternative A: keep the clamp and judge the furnace against D

This is Plan 0047's state. It costs nothing and the test still holds the probe update to its
own fixed point. It lost because the bias is not a constant of the estimator. It depends on
how much of a probe's view lies near edges and emitter contacts: -8.5 % in a 24 x 24 x 12
box, more in a narrow one, and an unknown share in a cave. Plan 0047's step D suggests that
share is large. A test against D cannot see it grow.

### Alternative B: a constant gain on the probe rays' direct light

Multiplying the probe rays' direct estimate by `pi / D_furnace` (about 1.093) would return
the furnace to pi in one line. It lost for the same reason: the gain is right for one box
only, and it brightens probes whose rays never meet the clamp.

### Alternative C: one higher clamp shared by the screen and the probes

Raising `FIREFLY_CLAMP` for both callers is the smallest code change. It lost because the
screen pays for it where it gains nothing: its bias is already -0.025 % at the furnace floor,
and a pixel's spike lands in an emissive history that Plan 0047 makes long, so one firefly
stays on screen for many frames. The probe-only clamp keeps the screen as it is.

### Alternative D: a per-ray clamp relative to the probe's own irradiance

Measured in Plan 0047's step D to still the gather. At every `k` tried (8, 4, 2) it cut the
gather's mean luminance by 38 to 53 %, and at 8 and 4 it raised the composite's patches. It
is the opposite of this decision: it buys stillness with more bias.
