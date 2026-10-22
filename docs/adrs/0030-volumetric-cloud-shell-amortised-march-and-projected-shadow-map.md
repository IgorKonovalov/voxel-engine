# ADR-0030: Clouds are a ray-marched shell of seeded noise, one pixel in four of a quarter-resolution buffer per frame, rebuilt temporally and shadowing the ground through a projected map

> **Status:** proposed
> **Date:** 2026-10-22
> **Related plan(s):** [Plan 0035](../plans/0035-aerial-perspective-and-volumetric-clouds-with-cloud-shadows.md)

## Context

The owner chose volumetric clouds in the Horizon Zero Dawn and Nubis style for the outdoor
batch (interview, 2026-10-22): a ray-marched layer at reduced resolution with temporal
accumulation, casting shadows on the ground, with coverage driven by weather. The 2.5D layer
(a lit, parallax-mapped sheet) was rejected in that interview and is not reopened here. What
remains open is how the layer is sampled, how often each pixel is marched, how it is lit, and
how its shadow reaches the ground: each has a cheaper and a more exact answer, and the choice
moves a budget line.

The facts that decide it. `docs/nfr.md` §2 gives volumetrics a share of the 2.1 ms headroom
line on the floor at 1080p with the upscaler (internal 720p, Plan 0030), and the floor is two
to three times slower than the dev box (a projection; the baseline card is not on hand) and
the slow adapter slower still. A cloud ray crosses kilometres of shell: from the ground at 5
degrees of elevation a shell at 1.5 to 4 km altitude spans 16.95 to 44.15 km along the ray
(sphere arithmetic, Plan 0035), so a march of 64 steps with six light samples at each dense
step reads a few hundred 3D-texture samples per ray. At 720p that is about 0.9 million rays
per frame if every pixel is marched, against 57 600 if one pixel in sixteen is. Clouds are
smooth, distant and slow (wind moves them a fraction of a metre per frame), so the history of
a cloud pixel is almost always valid, which is what makes amortising the march over frames
safe where it would not be for the near field.

The sun's shadow is Plan 0032's per-pixel shadow ray (ADR-0027), one more light sample in
Plan 0024's light pass and in Plan 0029's probe rays. Cloud transmittance along that ray is
the integral of the same 3D noise over up to tens of kilometres: per pixel it would cost as
much as a second cloud march, and per probe ray more again. The cloud field changes slowly
and is far above every receiver, so a 2D table of its transmittance toward the sun serves
every receiver at the cost of one texture read.

Lighting: Schneider's Beer-Powder term ("The Real-time Volumetric Cloudscapes of Horizon Zero
Dawn", SIGGRAPH 2015) darkens cloud edges facing the light cheaply, but it does not conserve
energy, and Hillaire ("Physically Based Sky, Atmosphere and Cloud Rendering in Frostbite",
SIGGRAPH 2016) replaces it with an energy-conserving step integral and Wrenninge's octaves
of attenuated extinction, scattering and phase eccentricity as a multiple-scattering
approximation. Which looks better on our scenes is the owner's call by eye (decide by
measurement), so both stay switchable until seen.

## Decision

**We render clouds as a spherical shell over the planet of Plan 0032's atmosphere, whose
density is a seeded noise field shaped by a weather map, marched in a quarter-resolution
buffer (half the internal resolution on each axis) where each frame marches one pixel of
every 2 x 2 block in a fixed order and a reconstruction pass rebuilds the rest from history
reprojected by the cloud's own depth and the wind's displacement; one in sixteen (a 4 x 4
order) and every pixel are the other two update modes.** The density follows the Nubis recipe:
a 128^3 Perlin-Worley and Worley base volume and a 32^3 Worley detail volume, both `rgba8unorm`
with mips, generated once on the GPU from a `u32` seed; a 2D weather map (coverage,
precipitation, type) remaps the base shape, a height profile per cloud type shapes it, and the
detail erodes its edges. Lighting integrates each step energy-conservingly (Hillaire 2016)
with three scattering octaves (Wrenninge) and a two-lobe Henyey-Greenstein phase, a six-sample
cone march toward the key light (the sun, or the moon at night), the key light's colour from
Plan 0032's transmittance LUT, and an ambient term reduced from its sky-view LUT; Beer-Powder
is the switchable alternative. The march stops at the near or far field's hit, and the
composite applies a cloud texel to a pixel only where the pixel's hit lies beyond the cloud's
front depth. The ground's cloud shadow is a 512 x 512 `r32float` map of transmittance toward
the key light, indexed by a receiver's position projected along the light direction onto the
shell's base altitude, rebuilt every frame over a 4 km square snapped to its texels around the
camera's projection; Plan 0032's sun sample, in the light pass and in the probe rays,
multiplies its visibility by it. A camera inside or above the shell uses the same segment
logic; inside the shell the update falls back to every pixel.

## Consequences

### Positive
- Cloud rays per frame are a sixteenth of the internal pixels in the default mode, so the
  march's cost scales with screen area, not with how cloudy the sky is (early exit at full
  opacity aside), and the one-in-sixteen mode is a further factor of four for the slow
  adapter, chosen by reading.
- A cloud shadow costs every receiver one bilinear read: the near field, the far field and
  the probes darken under the same cloud without a second march.
- The noise is generated from a seed on the GPU, so no texture asset ships and two consumers
  with different seeds get different skies; the weather map is the one input Plan 0036
  drives (coverage, type, precipitation, wind offset).
- Clouds are cosmetic and client-side: nothing here touches `vxe-world` or determinism.

### Negative
- Temporal reconstruction ghosts where history is wrong: a fast turn reveals screen edges with
  only a quarter (or a sixteenth) of their pixels marched, and the first frames after a cut
  show the spatial fill's blockiness. A camera inside the shell sees near cloud move with
  parallax that a one-in-four update cannot follow, hence the fallback to every pixel there.
- One front depth per cloud texel is a coarse occlusion test: terrain that pierces the cloud
  layer (a peak in the cloud) or a pixel at a depth edge gets the cloud's whole contribution
  or none. Acceptable while the world's terrain stays below the shell base; a consumer with
  mountains into the clouds would need a per-pixel march end.
- The projected shadow map assumes the receivers lie in a flat slab under a thin-in-curvature
  4 km square: it is wrong by the planet's curvature at its edges (about 1.3 m of drop at
  4 km, invisible at 7.8 m texels) and it has nothing beyond its square, where receivers read
  the coverage's mean transmittance instead.
- Two lighting models, three update modes and a debug flat density are switch surface to
  keep tested until the owner retires them.
- About 31 MB of VRAM at a 1080p internal resolution (Plan 0035's table), most of it the base
  noise and the cloud history.

## Alternatives considered

### Alternative A: march every pixel every frame, at full or half internal resolution

The exact answer, with no history and no ghosting, and the cheapest to get right. It loses on
cost: at 720p internal, every pixel is 921 600 rays a frame and half resolution is 230 400,
sixteen and four times the default mode's 57 600; on the slow adapter either would spend the
whole headroom line on clouds before Plan 0036's rain or M8's fog. It remains as the `every`
update mode, used inside the shell and by the goldens.

### Alternative B: cloud shadow by marching the noise along each sun shadow ray

Exact per receiver, and it shadows a peak that pierces the layer correctly. It costs a second
cloud march per lit pixel and per probe ray: at 720p and one sun sample per pixel, 921 600
light marches of 16 or more samples a frame against the map's 262 144 texels at the same
sample count, and the probe rays add their own. The receivers sit far below a slowly moving
field, which is the case a 2D table serves.

### Alternative C: clouds in a froxel volume shared with the aerial perspective and the fog

One camera-aligned volume (Hillaire's "Towards Unified and Physically-Based Volumetric
Lighting in Frostbite", 2015) would integrate clouds, haze and local fog in one pass. A froxel
grid's slices are kilometres deep at cloud distances (the aerial-perspective LUT's last slices
are about 2 km thick, Plan 0035), so a cumulus edge or a gap of a few hundred metres would
smear into a slice; clouds need a per-ray march at metre-scale steps where they are dense.
The aerial-perspective LUT and M8's fog keep their own volumes (Plan 0035 says how they relate).
