# ADR-0042: The far shading normal blends from the face normal to the 27-count gradient by the voxel's size on screen

> **Status:** accepted (2026-11-03, at Plan 0055's close). The mechanism is built and on by default;
> the setting `[2, 6]` was the owner's provisional pick; see Outcome (2026-11-11, Plan 0061): it stays
> **Date:** 2026-11-03
> **Related plan(s):** [Plan 0055](../plans/done/0055-clarity-pass-no-detail-fsr2-default-and-a-far-normal-spike.md)
> (Phase 3's spike and Phase 4's owner answers are the evidence; the readings are in
> [`docs/plans/logs/0055.md`](../plans/logs/0055.md))

## Context

The owner's baseline of 2026-11-03 saw swirling contour bands on far ceilings and walls. They are
geometry: a dome of 10 cm cubes is a set of contour lines at 10 cm height spacing, each riser a dark
line and each tread a lit strip, and below about two pixels of spacing they alias into swirls. They
stayed under every upscaler setting and vanished under both smoothed normal modes. The a-trous
filter never blurs across them, because its normal weight is `max(n.n', 0)^128`.

Whole-frame `gradient27` removes them, but the owner calls it plastic up close, and it costs about
+1.5 ms in the hit pass at the showcase start on the RTX 3080 Laptop. ADR-0041 makes honest cubes
the default surface, so the fix has to leave the cubes' geometry alone and change only the shading
normal where a voxel is small on screen.

## Decision

**The `face` look's shading normal blends toward `gradient27_normal` by the voxel's projected size.**
A hit at distance `t` voxels in a view `W` internal pixels wide spans
`px = W / ((tan_left + tan_right) t)` pixels. The weight is `w = 1 - smoothstep(lo, hi, px)`. At
`w == 0` the normal is the face normal and the 26 neighbour reads are skipped; at `w == 1` it is the
gradient; between, `normalize(mix(nf, g, w))`. A ray that starts inside rock keeps the face normal,
and the shadow ray's origin keeps it as well (`shadow_normal_of`). The knob is
`LookParams::far_normal`, `[0, 0]` off, and the fly-through's `--far-normal LO,HI`.
`LookPreset::DEFAULT` carries `[2, 6]`; `--look face` is the far normal off. The library's default
stays off.

**`[2, 6]` is provisional.** The owner, from one window of six variants on the showcase, RTX 3080
Laptop: "i think variant 4 is the best. difficuilt to say with problematic lighting", then "keep 2,6
as provisional". The setting is reopened after Plan 0051, when the light no longer gets in the way
of judging it.

## Consequences

### Positive
- The far bands go. Hall shot (`strip_run`, RTX 3080 Laptop, 1920 x 1080, FSR 2 at 1.5, tree on
  a2c4dc5), mean neighbour luma difference, h / v: the dome crop reads 0.00188 / 0.00192 against
  face's 0.00550 / 0.00732, at most half of face as the plan's bar asked. Flight flicker's median
  falls from 0.00277 to 0.00094 per pixel.
- It needs no storage and no new pass: two floats in spare `Params` words; `frame_bytes` is
  unchanged.
- It costs nothing where voxels are big on screen. At the level's `vein` and `spawn` poses the
  crops read the same as face, and `cmp` of `[3, 10]` against face reports no byte differing.

### Negative
- **At the showcase's hall shot `[2, 6]` is close to whole-frame `gradient27`.** Its crops read
  within 3 % of `gradient27`, and its cost is the same.
- **It misses both of Plan 0055's other Phase 3 bars, and the owner took it anyway.** The near pillar
  loses 32 % / 28 % of its energy against the bar's 5 %: 0.00163 / 0.00171 against face's 0.00240 /
  0.00238. That is the plastic this mechanism was meant to avoid, at this pose. The hit pass reads
  4.350 and 4.354 ms against face's median of 2.850, about +1.5 ms against the bar's +0.8 (a guess);
  the GPU frame reads about 12.04 ms against 10.58. No setting read meets the dome and pillar bars
  together. Dev's pick was `[1, 2]`, which meets the dome and cost bars and reads -9.6 % / -8.4 % on
  the pillar.
- **The Radeon pays more.** RADV RENOIR, 1920 x 1080 from 1280 x 720, showcase start: frame 55.0 ms
  against 48.5 with `--look face`, hit 21.08 against 14.47. The vein pose is unchanged within its
  spread. A slower adapter's only switch is `--look face`.
- **The band is in internal pixels,** so the voxels it smooths depend on the internal resolution:
  the same `[2, 6]` reaches nearer at a lower one.
- **It was judged on the showcase only.** Phase 4's level run did not happen, and the owner's own
  words say the lighting made it hard to judge. The transition band in motion is read only by the
  strips' flicker map.

## Alternatives considered

### Alternative A: a mip-based far normal

A normal filtered from a coarser level. It needs new storage and a pass before anything is on
screen, and the blend answers whether a far normal helps at all without either. It is the next thing
to try if the setting cannot be made to leave near surfaces alone.

### Alternative B: supersampling or more temporal filtering

The lines are geometry, not sampling error: they stayed under every upscaler setting, and a stable
reconstruction of a staircase is still a staircase.

### Alternative C: the field surface (ADR-0041)

It smooths the far bands by changing the surface. In the owner's window it costs about 3x `face`'s
GPU frame, which is why it is opt-in, and the owner has priced it out as the default.

### Alternative D: whole-frame `gradient27`

The owner's word for it up close is plastic. It remains a diagnostic look. At the hall shot `[2, 6]`
reads about the same; the two differ only where voxels span over 2 pixels, and from 6 pixels up
the blend is the face normal.

## Outcome

**2026-11-11, Plan 0061's close (v0.48.0): `[2, 6]` stays the default, now under the baked store
read by the far normal (ADR-0049).** After Plan 0059 the store was read by the face's direction,
so the far normal reached only the lamp and the light filters' weights, and the bands came back
(Plan 0061 Phase 1). ADR-0049 brings the far normal back to the light. In that window the owner
picked `[2, 6]` over `[1, 2]`, which Plan 0056 had picked under the table's light and which never
landed. `[2, 6]` costs about 1 ms more in the hit pass than `[1, 2]` on the RTX and about 6 ms more
on the RADV RENOIR at the hall (Plan 0061 Phase 2's frame rows). FSR 2 at scale 1 takes the same
voxels at `--far-normal 3,9`.
