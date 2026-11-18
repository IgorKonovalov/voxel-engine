# 0086: Still outdoor light: a sky visibility baked per brick, a long history for the sky and key rays, canopies that shadow as bricks, stairs that keep their history, and a penumbra spike

> **Status:** in-progress (the owner's go, 2026-11-17; it runs before Plan 0081, and Phase 6 is
> conditional on its reading as drafted; **amended 2026-11-18** after Phase 8's window failed:
> Phases 9 to 15 added; **amended again 2026-11-18** after Phase 15's window: Phases 16 to 24
> are the fix pass and the third window, the picks as tests are Phase 25; the owner's picks
> and answers are in the second amendment, and the go for Phases 16 to 25 is the owner's
> "all recommended, go ahead" of that date)
> **Created:** 2026-11-17
> **Milestone:** M10: Outdoors in [`docs/roadmap.md`](../roadmap.md)
> **Related ADRs:** [ADR-0070](../adrs/0070-sky-light-is-baked-visibility-coloured-at-read-with-a-long-history-ray-where-stale.md)
> (proposed with this plan: the sky visibility baked per brick direction, coloured at read; the
> ray on a long history where the bake is stale),
> [ADR-0027](../adrs/0027-the-sun-shadow-is-one-per-pixel-ray-in-the-light-pass.md) (the key
> light's ray; its Outcome of 2026-11-17 and its Alternative F, taken here for the rays),
> [ADR-0046](../adrs/0046-static-emissive-light-is-baked-per-chunk-when-a-chunk-loads-or-changes.md)
> (the store and the queue that gain the channel; its Outcome of 2026-11-17),
> [ADR-0042](../adrs/0042-the-far-shading-normal-blends-to-the-27-count-gradient-by-the-voxels-size-on-screen.md)
> (the far normal whose regime Phase 3 uses),
> [ADR-0018](../adrs/0018-direct-light-per-pixel-one-sample-from-a-hit-buffer-with-svgf-history-killed-by-edits.md)
> (the temporal pass's reprojection and resets),
> [ADR-0063](../adrs/0063-surface-detail-is-a-procedural-shell-marched-at-the-hit-described-per-material-on-the-render-side.md)
> (Plan 0081's render-side style table, where Phase 2's flag lives once both have landed)
> **Source:** the owner's Phase 9 window of Plan 0032 (2026-11-17; the verdict verbatim in
> `docs/plans/logs/0032.md`), the diagnosis and options in
> [the review of 2026-11-17](../reviews/2026-11-17-plan-0032-window.md), and the owner's
> picks of the same day: C1 and C2 both, D1 to D4 all four, each behind the attribution
> reading the review names; stable image first, performance considered.
> **Starts after:** [Plan 0032](done/0032-sun-and-sky-hillaire-atmosphere-time-of-day-and-an-outdoor-testbed.md)
> closes and merges (its Phase 10 makes an absent chunk sky, its Phase 11 sets the defaults
> this plan's readings stand on). The architect recommends it ahead of Plan 0081 (the owner's
> priority is the still image; Phase 2 names the merge item if 0081 runs beside).

## TL;DR

The outdoor light holds still. The sky's light at a face comes from a visibility baked per
brick direction into the per-chunk store, coloured by the sky each frame, so the tunnel's rim
and far end read one stored number instead of a 0-or-full sample; the per-frame sky ray is
cast only where the store has no fresh value, on a running mean of its own with a firefly
clamp, so a dug skylight still lights the chamber on the next frame. A canopy shadows as its
brick, so a forest casts a blob and no flecks. Where a voxel is smaller than the far normal's
reach, the temporal pass keeps history across a stair and the key ray starts above it, so the
contour speckle bands go. The key light's visibility may ride the same long history, after a
reading of the 3-degree penumbra's grain. A spike reads a deterministic penumbra from the
cell hierarchy's closest approach and stops. Phase 1 is a reading that splits the flicker by
term at the tunnel and the forest, so each fix is attributed before it is built. The first
thing a reader sees is Phase 1's table; in the window, the tunnel at noon without sparkle.

## Context & problem

Plan 0032's Phase 9 window (2026-11-17): "very much flicker on tunnel edge", "towards the end
of tunnel it looks super flickery", "a lot of flicker in tree shadows, some simplification is
needed", "when light settles and trees are coarse shadows look amazing". The review of the
same day read every complaint but the sun's size to three mechanisms, each from the code and
the readings:

- **One binary sample on an eleven-frame blend.** The sky ray and the key light's shadow ray
  each add 0 or the full value to the direct target, blended at `max(0.2, 1 / (age + 1))`
  (`temporal.wgsl`). At the tunnel's rim the sky ray's hit rate is about 0.3 to 0.6 (a
  standard deviation of about a sixth of the full value on the blend); at the far end it is a
  few percent, so a rare hit adds thirty times the mean and decays over three or four frames.
  Along the voxel staircase's contours the temporal pass rejects history where the tread and
  the riser alternate under FSR 2's jitter (`HISTORY_COS` 0.8, the material test), and a
  rejected pixel shows the raw sample; the a-trous leaves it alone because its moments are
  fresh there.
- **Sun flecks through the canopy.** The coarse shadow walk treats a 2^3 block with any set
  bit as solid; at `LEAF_FILL` 64 a tenth of a canopy's blocks are empty (`0.75^8`), so every
  tree casts hundreds of 2-voxel sun flecks that move with the sun and the jitter. The LOD
  ring's mip-1 trees are solid under the same rule, which is the look the owner called amazing.
- **The 3-degree penumbra.** The owner's pick widens every penumbra (8.4 voxels under a canopy
  80 voxels up) and a single disc ray is noisiest at its middle (standard deviation 0.17 of the
  full value at `p` 0.5).

Plan 0032 Phase 6's stop rule did not see the first: its rig was an open floor under a uniform
sky, where every ray reaches the sky and the variance is 0. Restated on the tunnel (Plan
0032's amendment of 2026-11-17) it fires, and this plan is its amendment. A longer history
alone cannot carry the far end: a running mean of `N` frames of a Bernoulli at hit rate `p`
has a relative standard deviation `sqrt((1 - p) / (p N))`, 0.71 at `p` 0.03 and `N` 64. The
far end is where the geometry holds still and a stored value is right.

The lighting base: ADR-0046's store bakes six direction values per surface brick when a chunk's
reach is resident, re-bakes in rings at an edit, reads bilinear with no screen history; a
still face read from it does not flicker (Plan 0059). Costs read (Plan 0032's readings log,
Phases 6 to 8; `docs/nfr/02-frame-budget.md`): the sky ray about 0.25 ms of the RTX 3080
Laptop's light pass at 1080p and about 4.8 ms of the RADV RENOIR's, per ray; the RENOIR's
outdoor light pass 10.5 ms (12.5 in the forest) against section 2's 6.0 ms line; the key ray's
steps p99 79 to 92 in the forest. `nfr.md` figures touched: section 2 Lighting (the light
pass, the temporal pass), section 3 (the store's record, a target pair), section 4 (a dig's
re-bake with the channel), the streaming line's bake-queue step.

## Decision

We build ADR-0070 and the three shadow fixes, each behind a reading, in this order: the
attribution reading; the canopy flag and the stair history (the cheap, direct ones); the
baked visibility; the rays' long history; the key light on it; the penumbra spike; the window;
the tests. **Visibility is geometry and radiance is the sky:** the store gains a `u8` per
brick direction, the cosine-weighted fraction of the hemisphere that reaches the sky, baked
by the emissive job while a `Sky` is set and re-baked in its rings; a face reads it bilinear
and multiplies by the sky's cosine-weighted irradiance about its normal, six values a frame
from the sky-view LUT. The ray estimates the same visibility where the bake is stale or
absent, on a pair of the lighting grid with a running mean to 64 frames, a firefly clamp and a
variance reset; at rest inside the bake radius no sky ray is cast, and `sky_rays` 0 is the
bake-only setting. A render-side per-material style flag makes the shadow walk stop at the
first brick of that material with any set bit, set on the testbed's `leaf`. In the far-normal
regime the temporal pass tests the smoothed normal and drops the material test for the
irradiance history, and the key and sky rays start from the hit lifted along the smoothed
normal, with a reading that a thin far wall leaks under a pixel. The key light's visibility
moves to the pair only if the penumbra's grain, read after the first two fixes, is over the
lamp's. The penumbra spike builds a setting, reads it and promises nothing.

We rejected the long history alone, more rays where the neighbourhood disagrees, the probes,
an unoccluded hemisphere, baking the sky's radiance, `L1` visibility, the ray blended with
the bake everywhere and per-face sky cells (ADR-0070); a 4-voxel shadow walk everywhere (the
review's D5: the Lego look the owner refuses); and two radii per body (A2: the fallback if the
penumbra's grain survives this plan).

## Architecture diagram

```mermaid
flowchart LR
    subgraph render [vxe-render]
        Q[bake queue\nper chunk, rings at a dig] --> ST[store record\nsix emissive values\n+ six sky visibilities u8]
        LUT[sky-view LUT] --> ES[E_sky: six axis\nirradiances a frame]
        ST --> T[temporal pass\nv(n) x E_sky(n), no history]
        ES --> T
        H[hit] --> LP[light pass\nsky ray only where stale\nkey ray, disc sampled]
        LP --> P[visibility pair\nrunning mean to 64\nfirefly clamp, variance reset]
        P --> T
        ES --> LP
        F[material style flag\nshadow as brick] --> LP
        T --> C[composite]
    end
```

## Interface for Plans 0033 to 0037

Changes to Plan 0032's Interface that later outdoor plans rely on; the rest of that section
stands.

- **The sky term at a face** is `v(n) E_sky(n)`: `v` the store's bilinear visibility read
  (or the pair's running mean where the store is stale), `E_sky(n)` the cosine-weighted sky
  irradiance about `n` from the sky uniform's six axis values. A plan that changes the sky's
  radiance (clouds, an aurora's glow, a night tint) changes `E_sky` by changing the sky-view
  LUT; it bakes nothing and re-bakes nothing. A plan that occludes the sky with geometry the
  mirror does not hold (Plan 0033's far field, Plan 0035's clouds) multiplies `v` at the one
  place the temporal pass reads it, or leaves it.
- **The visibility pair** holds the sky ray's and (if Phase 6 takes it) the key light's
  visibility on a running mean to `SKY_HISTORY_CAP` frames, reset by the direct
  accumulation's tests and by the variance reset. A later term that changes the key light's
  visibility frame to frame (a cloud shadow) either stays under the reset's band or sets the
  sky-change bit on the frame it changes; it does not write the pair.
- **The style flag** `SHADOW_AS_BRICK` is a bit of the render-side per-material style word
  (Plan 0081's table, or this plan's word that 0081 absorbs). A consumer sets it per material
  through `Frame::set_material_styles` (the name Plan 0081 gives, or this plan's).
- **Settings** (Plan 0069's surface): `lighting.sky_rays` accepts 0 (bake only);
  `lighting.sky_history` (the cap), `lighting.leaf_shadow` is the testbed's A/B, not a
  setting; `lighting.penumbra approach|disc` from the spike, `disc` the default.
- **The column map** (added 2026-11-18, ADR-0070's amendment): the renderer keeps a
  camera-centred 2D map of the top of solid per column over the mirror's footprint, built from
  the resident chunks (a LOD chunk's mips make it conservative). A sky ray that ends with no
  hit, at its cap, at an absent chunk or at the window's side, has reached the sky only above
  that column's top. Plan 0036's fall map (ADR-0031) at the vertical fall direction is this map;
  Plan 0036 extends it to slanted directions rather than building a second one. Plan 0033's far
  field may raise a column's top from its coarse levels where the mirror has no chunk.

## Amended 2026-11-18 (Phase 8 window)

Phase 8 ran on 2026-11-18 and failed on five points; the owner's words per run and the stills
are in `live/window86a/verdicts.md` (1.png to 17.png beside it), the orchestrator's summary and
the owner's picks under it. The diagnosis `dev` ran after it is the readings log's section
"After Phase 8: the underground leak, a diagnosis" (commit `2fe4598c`). What the window said,
and what each point is attributed to:

1. **Underground lit with no emitter** (run 12, 17.png: eye at 743 131 15, chunk 11 2 0, noon,
   scene -2.7 EV, walls lit everywhere). Attributed and measured: a sky ray that meets no rock
   within its cap counts as sky (the bake's cap 256 voxels, the light pass's 512), and a ray
   that leaves the window or enters an absent chunk counts as sky too (Plan 0032 Phase 10's
   rule). In a closed cavern 0.38 % of rays pass 256 voxels, and times the noon sky's irradiance
   that is a stored part of 0.57 to 2.75 at exposure 1 where the truth is 0 (the open ground
   reads about 150). ADR-0070's amendment of this date decides what counts as sky; Phase 9
   builds it. The cave mouth's seal bar (`the_cave_mouth_is_lit_from_outside`, sealed walls at
   24 % of unsealed against a 2 % bar) is the same leak.
2. **The tunnel too bright and "lit almost evenly" deep inside** (runs 2, 3 and 12; 2.png,
   4.png, 16.png; scene -11.9 EV, the auto eye pinned at its +2 EV ceiling). Two causes, one
   measured: from station 7 on the cap's false sky is 2 to 4 times the true term, so the deep
   floor stops falling (Phase 9 removes it); nearer the mouth the brightness is the true sky
   term lifted by the eye's ceiling (`OUTDOOR_MAX` 4, +2 EV, the fly-through's outdoor start
   exposure), which the owner reads as "should be much much darker". Phase 12 makes the ceiling
   a call the owner can take by eye.
3. **Zebra stripes on stepped faces** (treads lit, risers black; 2.png, 11.png). Measured on the
   bake: one visibility per brick and axis, and in the tunnel `+z` reads 10 to 30 times the
   horizontal directions near the mouth (station 3: 0.37 against 0.011 to 0.14; 3 to 4 times
   deep inside), with the cap raised too. A tread reads its brick's `+z`, a riser a horizontal
   value, and the eye's lift makes the ratio visible. Not the cap. Phase 13 is its design.
4. **The forest floor's grain at every sun angle, worst at `10,180` and during the day run;
   too bright with voxel leaves** (runs 5 to 9; 7.png to 10.png, 12.png). Not attributed by a
   reading in the forest: Phase 6's slab reading (3-degree p99 equal to the lamp's) did not
   represent a canopy's edges at a low sun, where the penumbra stretches by `1 / sin 10` to
   about 48 voxels and most of the floor is penumbra under one disc ray on the direct target's
   eleven-frame blend. The brightness with voxel leaves is a guess too (the sun through the
   canopy's holes, or the baked sky through them). Phase 14 reads first, then builds.
5. **Black cells while settling** (runs 9 and 10; 13.png missing 29 638 with 24 074 black,
   14.png 44 638 with 30 062): faces with no store block whose missing-light chain finds no
   summary, after a jump, for seconds. The young visibility pair after a jump reads 0.17 to
   0.19 in its first frames underground (the streamed rig), which is the absent-is-sky rule
   while the window fills, remembered by the running mean. Phase 10 takes both.

**The owner's picks** (2026-11-18): the route is the diagnosis, then this amendment; leaf
shadow `voxel` becomes the default (brick stays the setting); the penumbra `disc` stays the
default and `approach` stays an off-by-default setting (the owner "doesn't see much of a
change", the approach reads 1.43 times the light pass on the RADV RENOIR, 21.8 against 15.2 ms,
and run 10's "strange" forest had 44 k black cells, so the difference seen was likely the
settle); recheck both after the fixes. Phase 11 sets the defaults.

**The order and what may shrink.** Phase 9 lands first: it changes the sky term everywhere a
ray ends without a hit, so every later reading is taken on top of it. After it, the deep
tunnel's floor falls by itself (point 2's first cause), the young pair's first frames lose
their cause (Phase 10 may shrink to its bar), and the stripes deep inside lose the lift that
made them visible once Phase 12 lowers the eye (Phase 13 may shrink to the near-mouth case).
The forest (points 4 and 5) stands under open sky and is not expected to move. Phases 12 to
14 are exploration (lane rules 2b): settings and readings, the owner's second window (Phase 15)
their gate; Phase 16 writes the picks as tests. The Implementation log's rows for Phases 9 to
16 were added by this amendment (the architect's, at the renumbering); `dev` fills them.

**Owner questions** (recommended option first; Phase 9 starts without the answers, Phases 12
to 14 read every arm the window needs and the window picks):

**Answered 2026-11-18 (owner, "all recommended"), with the go for Phases 9 to 16:** 1 (a), 2 (a), 3 (a), 4 (a).

1. **The eye's ceiling outdoors (Phase 12).** (a) Anchor it to the sky: the eye opens at most
   `N` EV above the exposure the sunlit ground needs, derived each frame from the sky's
   irradiance, so a noon tunnel stays dark and a night scene still opens; `N` is a setting the
   window reads at 0, 2, 4 and 6 (recommended: it follows the sun, and the first consumer, with
   no sky, keeps ADR-0033's eye untouched). (b) A fixed lower ceiling (0 EV instead of +2),
   one constant, the same at night. (c) Keep +2 EV and darken the tone curve's toe below a
   luminance (a look change on every scene, lamp-lit ones included).
2. **The stripes (Phase 13).** (a) The sky term reads its visibility by the smoothed 27-count
   normal at every distance (the surface stays honest cubes; its sky light becomes continuous
   across a one-voxel stair, as the far normal already makes it at a distance), and a bounce
   floor mixes a share of the brick's mean visibility (recommended; both are settings and the
   window picks). (b) The bounce floor alone (the stripes soften, the tread stays brighter).
   (c) Leave the stripes to the sunlight bounce, a plan of its own (the backlog's).
3. **The canopy's third arm (Phase 14).** (a) Build `absorb`: a per-material shadow absorption
   by path length through the material's bricks, so a canopy shadows as a continuous
   transmittance with no fleck and a depth the material sets, read beside `brick` and `voxel`
   in the window (recommended; it is ADR-0047's translucent walk on shadow rays, a second walk
   through flagged bricks only). (b) Read `brick` and `voxel` only, on the long history, and
   park `absorb` in the backlog.
4. **The first second after a jump (Phases 9 and 10).** A column whose chunks have not all
   arrived has an unknown top. (a) Count its sky as before, from the chunks that are resident
   (an under-estimate: a brief leak underground that closes as the chunks above arrive, and the
   far ground stays lit while moving, Plan 0032 Phase 10's rule; recommended: the bake waits
   for the column to be final either way, and the first consumer sets no sky). (b) Count an
   absent chunk in the column as solid (conservative: no leak ever, and the open ground after a
   jump is dark until its columns fill, then brightens).

## Amended 2026-11-18, Phase 15 window

Phase 15 ran on 2026-11-18 (session86b, the release binary of `e48e2aee`, the before binary
`live/base-target` built from `a37c908b`). The owner's words per run, the orchestrator's
readings and the stills 19.png to 22.png are in `live/window86b/verdicts.md` (gitignored; the
words are quoted here). The window **passed on the leak and the picks and failed on six
defects**; Phases 16 to 24 are the fix pass and the third window, and the picks as tests move
to Phase 25.

**What the window said.** Run 1 (before): "flickers". Run 2 (the tunnel, noon): "its much much
better, but i see a clear boundary between types of colors" (19.png: the near walls and steps
flat and clean from the baked store, the deeper tunnel grainy from the per-pixel sky rays, and
the edge between them). Run 1b (17.png's underground spot): "looks right" (the leak is fixed by
eye). Run 2b (anchor 0, five lights fired): "when I shoot light, walls becoming grainy" (20.png;
the light pass 0.66 to 5.59 ms, `black` 1128, the lit patch's edge a dither of single lit
pixels). Runs 2a to 2e: "ceiling 4 is the best". Runs 3a to 3d: "smoothed normal looks good,
face normal with bounce are also fine". Run 4a (column wait on, the hills at 17:00): "there is
huge artifact" (21.png: a black region with straight edges on the hill floor; `missing` 7327,
fallback 6831, `black` 496, queue depth 14 722, waiting 1051), and it **stays**; run 4b (wait
off) shows no such spot and starts "a bit faster yes". The dig (4a/4b): "when I dig - area
around the place I dig flickers shortly, can't put it to screenshot - its too fast", and later:
"flash around the dig were similar to the problem that we had before - small lights when
digging and flicker. its just much faster than before". Runs 5a to 5f (the forest): "light is
not settled even after wait" (22.png, voxel 60: the sunlit treads clip to white, the stripes
back under face with no anchor), brick 60 "same", absorb 60 "shadows are good but fps dropped
significantly - around 20", voxel 10 "grainy, especially after fast turn", brick 10 "same",
absorb 10 "bit better, but grainy after turn". Run 6 (the day to sunset, no anchor): "ok, maybe
even too bright". Runs 7a/7b (disc against approach): "no significant difference". Run 8 (a
skylight dug then sealed): "light disappears quickly, fine".

**The owner's picks and answers** (2026-11-18, "all recommended, go ahead": the go for Phases
16 to 25):

- **Defaults:** the exposure's sky anchor `N` 4; the sky normal `smoothed` (the bounce stays a
  setting, 0 by default); leaf shadow `voxel` (already); penumbra `disc` (`approach` stays a
  setting if it costs nothing when off, else goes).
- **Phase 9 bar 5** (the cave's seal at 24 % of unsealed after 64 frames, bar 2 %): the owner
  judged the sealed cave by eye, "light disappears quickly, fine"; the bar is re-derived from
  the measured decay (Phase 9's readings: 24.3 % at 64 frames after the seal, 8.7 % at 128,
  1.1 % at 256, 0.01 % at 512).
- **A, the column wait:** off by default (the faster start, no black spot); kept as a setting;
  its defect fixed anyway.
- **B, the sun's mean on the pair** (the Phase 14 owner question): option (a): the key's mean
  in `g` runs on the direct part's history age (`moments_out.z`, capped at `sky_history`) and
  resets with it, everywhere the key light is sampled; the sky's clamp and band stay the sky's;
  no new target.
- **C, absorb:** stays an off-by-default setting; making it cheaper is a backlog entry
  (`docs/backlog.md`, 2026-11-18; it cost about 20 fps on the RTX, the light pass 18 ms against
  voxel's 4.5).

**The six defects**, each with the phase that takes it. Where the cause is not known the
reading comes first (Phase 16) and the fix phase takes its cause from it and no further:

1. **A black region that stays, with the column wait on** (21.png). A defect of Phase 9's
   finality: some chunks never bake. Candidates: a column that never turns final because a
   chunk it waits for never arrives (within the LOD sphere but not mirrored by hysteresis,
   above or below the world's extent, empty by generation and never uploaded), so `bakeable`
   refuses every chunk whose reach footprint holds it; and 496 of those cells read black rather
   than the chain's fallback because their neighbours are unbaked too. Phase 16 reads which;
   Phase 18 fixes it and makes the wait a setting, off. (Phase 16, 2026-11-18, read neither:
   no never-final column, the same `black` under both wait modes, the unbaked chunks the `Full`
   edge ring; Phase 18's note of that date re-scopes its bars. Phase 18's second note: the
   counted pixels are lit by the sky ray, not black; the ring is the ray region at rest.)
2. **A seam between the baked sky term and the ray's** (19.png). Known in part: Phase 13's
   arms apply to the baked read only (the pair's term keeps the shading normal), Phase 5 built
   the switch with no fade, and the ray's running mean at a low hit rate is grainy by the
   arithmetic in Context (relative sd 0.71 at `p` 0.03, `N` 64). Open: whether the ray region
   at 19.png is the wait (it shrinks as the queue drains), the reach (it stays) or the stale
   bit. Phase 20.
3. **Walls grainy where fired lights land, under anchor 0** (20.png). Not attributed. The
   first candidate: a dropped light re-bakes its rings (Plan 0059's rings, ADR-0046) and the
   rings' bricks take `SKY_STALE` though no voxel changed, so the sky ray returns over the
   rings with its 0-or-1 samples at a very low exposure (the light pass's rise from 0.66 to
   5.6 ms and the 1128 black cells fit a re-bake). The second: Plan 0058's fired light's own
   shadow-ray grain, made visible by the anchor's dark tunnel. Phase 16 reads it against the
   before binary with the exposure pinned; Phase 22 fixes the first or files the second.
4. **A brief flash around a dig**: "small lights when digging and flicker", the dig sparkle
   of Plan 0049's window (bright dots in pixels 1 to 63 frames old, which the baked store
   removed by taking the emissive part off the screen history), back and short-lived. The most
   likely mechanism, from the code: a dig marks the rings' bricks `SKY_STALE` (ring 3, the
   light radius, opens on open ground since a touched brick's visibility was over zero), so the
   sky ray runs over the rings; the pair at a pixel that resets (the hole's fresh faces, which
   have no history, and the dilated edit box) starts at age 0 from one raw sample, 0 or the
   full `E_sky` (about 150 at exposure 1 at noon, against a shadowed floor's stored 10), and
   the firefly clamp applies only once the age passes 8, so the first frames show bright and
   black specks that the mean then averages out at `1 / (age + 1)`: "much faster than before"
   because the mean runs to 64 and the a-trous filters the direct part. Two more candidates:
   the ring's coarse re-bake landing per brick at a low sample count (brick-sized patches that
   jump as the full pass lands); and the sky term's move from the stored part to the direct
   part and back at the switch, with the direct part's `ALPHA` blend. Phase 16 reproduces it
   headlessly and splits the three; Phase 21 fixes what it names.
5. **The forest floor's grain at a low sun and after a fast turn**, under every leaf arm.
   Phase 14's attribution names the key light at `60,180` (p99 13 against the sky's 4); at
   `10,180` neither term alone. The grain after a turn is disocclusion: the new pixels'
   history is young. Phase 19 builds B (a) and reads the turn.
6. **The sunlit forest treads clip to white** (22.png, no anchor in that run). Not attributed:
   the anchor at 4 may be the fix (run 6 under no anchor read "too bright"; Phase 12's table has
   the forest at 17:45 2.08 EV lower at `N` 4 than `none`), else the eye meters the shadowed
   floor and the sunlit tread sits 5 EV above the key (Phase 14's parts: direct 440 against
   stored 10.6 at `60,180`), which the tone curve must shoulder. Phase 16 reads the clipped
   share under `none`, `4` and the before binary; Phase 23 takes its result.

**ADR-0070** is amended in place (it is `proposed`): the bake's finality wait becomes a
setting, off by default, and a chunk baked while a column over its reach was not final is
re-baked once when it turns final, so the end state is the decision's. The Implementation log's
rows for Phases 16 to 25 were added by this amendment; `dev` fills them.

## Amended 2026-11-18, Phase 21's fade (the architect's call, no owner question)

**The decision: the direct part never carries the pair's sky term. The filter adds it from
this frame's pair.** Phase 16 attributed the dig flash to the stored/direct switch at the
stale bit, and the dev's resume notes (readings log, Phase 18) named the mechanism: the
temporal pass adds the pair's sky term to the direct accumulation at the direct part's blend
weight (`acc_direct += a_direct * t`, `a` 0.2 at a mature pixel), so at the switch to the ray
the stored part drops the baked term at once while the direct part takes the pair's at a fifth
(a dip of four fifths of the term for a frame, Phase 16's "half the region over 16 levels"),
and at the switch back the stored part takes the re-baked term at once while the direct
history still holds the pair's and decays it over frames (the flash). The pair is already the
estimate: a long mean to `sky_history`, seeded from the baked value at that age where no ray
is cast, so its value is continuous across the switch to the ray. The direct part's blend is
a second history of the same quantity, and the second history is the lag. ADR-0076 rule 2
says it: a screen-space accumulation carries a world-space estimate, it does not become one.
So the fix removes the second history instead of flagging it: the temporal pass stops adding
the sky part of `t` to `acc_direct` (its luminance stays in the moments, so the filter's
variance still counts it); the a-trous's `variance` step, which already builds the filtered
quantity as the direct part plus a fallback's emissive part, adds `pair.x * sky_open_by_normal(n)
* exposure_now()` where this frame's `vis.a` says the ray was cast; the composite's
`LIGHTING_FILTER_OFF` branch adds the same term. The key light's term (Phase 19, `key_light_at
x pair.y`) stays in the direct part: it is sampled every frame wherever the key light is on, so
it has no switch to lag at.

**Where the history lives and which fade carries the switch (ADR-0076's binding clause).** The
sky term's history is the store's words and the pair (ADR-0070); after this amendment no
screen-space history of it exists but the pair's. The switch to the ray needs no fade: the pair
starts at the baked value (the seed at age `sky_history`), and the two read one quantity up to
Phase 20 (i)'s normal arm. The switch back lands the re-baked value at once, a step of the
dig's real light change and nothing else (old to new in one frame, monotonic, no overshoot, no
dip: rule 3's degenerate case). If the third window wants that step slower, Phase 20 (ii) is
its home (the store's fade over the sky words), not this phase.

**Rejected.** (1) A per-pixel flag "last frame's direct history held the term" in an existing
channel: every reprojected channel is taken (the pair's four: the sky mean, the key mean, the
short mean, the age; the moments' `xyz` reprojected and `w` the a-trous's stored flag, not
reprojected; `direct_accum.w` the luminance the a-trous's `dark_store` and the tests'
`alpha` readers use; `accum.w` the upscaler's fresh flag). Packing two meanings into one
channel under bilinear reprojection breaks one of them; a linear share would blend correctly
across the taps, but there is no free linear channel. (2) A new one-channel target: written
by the temporal pass it is a fifth storage texture against `Limits::default()`'s four
(`limits_for` raises storage buffers only), a limit the baseline adapter may not offer; as a
ping-pong of `vis` (`rgba8unorm`, +921 600 B at the 1080p half tier's 640 x 360) read at the
taps it adds a per-tap subtraction to the reprojection loop and keeps the second history to
correct every frame. More state to keep right than removing the state. (3) Putting the pair's
term into the stored part (`accum`, taken as is, unfiltered): one pass, no binding, but a face
whose block holds no sky words (a block baked before the sky was set, the ring's coarse
re-bake) would lose the a-trous over its raw samples, which Phase 20 bar 2 leans on. (4)
Resetting the direct part (`a` 1) at the switch frames, as `LIGHTING_LIGHTS_CHANGED` does:
removes the lag and the lights' history on the ring with it (a grain burst, Phase 16's other
24 dots), and still needs the switch detected per pixel. No ADR: the decision is a plan-level
mechanism under ADR-0070 (whose Phase 5 clause, "the direct target no longer receives the sky
ray", this amendment makes literally true) and ADR-0076 (proposed, on main, not edited here).

**Cost (every figure a guess until Phase 21 bar 6 reads it):** the variance step gains two
texture loads (`vis`, the pair) and the sky's six open irradiances per lighting pixel, about
+0.01 ms RTX and +0.1 ms RENOIR on `nfr.md` section 2's Denoise line (2.0 ms); the temporal
pass gives back the same adds, about 0 net; `frame_bytes` unchanged (section 3: 0 B); the
variance's and the composite's bind groups gain sampled and uniform bindings only. The
budgets table's Phase 21 row is updated below.

## Amended 2026-11-18, after the overnight flicker check (the architect's calls; the owner asleep)

The readings log's flicker section (2026-11-18, the fourth agent) attributed two things: the
castle the owner watched flicker (the Plan 0083 window's 25.png: a freshly placed castle on the
hills at noon, the eye in the keep) and Phase 19's gate regression. Five calls follow, each
with its grounds and the options not taken; the phases below are re-written where a call
touches them. Nothing here changes a default or an ADR. The owner's rule throughout: a flame
may flicker, nothing else may (ADR-0076, proposed on main; its rule 3: every transition is one
monotonic fade).

### 1. A stamped chunk is a dig, and a reset pixel under the sky ray starts from an estimate

**The reading.** The castle's chunks were air before the stamp: no block and no provisional
block, so `take_boxes` skips them (a ring chunk with no block), and the carve's ring of 173
hill chunks opens as digs ahead of everything; while the dig jobs hold two slices no candidate
opens (`open_step`), so the stamped chunks wait in the pool until the ring is done. Paced at
60 Hz on the RTX the keep's interior reads no record for about 5 s (`missing` 49 884 at the
stamp, 0 from frame 300), about 30 650 of its lighting pixels cast the per-pixel sky ray every
frame, and of the fresh strip's over-8 pixels 66 % are young and 15 % ray-cast. So the defect
has two parts. The bake is late. And while it is late every pixel whose history reset (the
whole wall: a frame before, its pixels saw the hill or the sky) runs the sky ray's mean from
one raw sample at age 1 (`pair_step` on no history: 0 or 1, at a hit rate near 0.02), which is
the sparkle; and the exposure meters 50 000 such pixels, which is the whole-frame swing. The
second part is the dig's dots (Phase 16: about 26 of 50) and the forest's grain after a turn
(the flicker table: 86.7 % of the forest's fresh over-8 pixels are young) by the same
mechanism; the castle only makes it a wall wide and five seconds long.

**The decision, two mechanisms.** (i) **Phase 21 (a) is built, not conditional, and widened:**
a pixel that resets where the sky ray is cast seeds the pair from the best world-space
estimate at age `SEED_AGE`, in this order: its face's record visibility (a stale ring brick's),
else the chain's (`store_sky_fallback`: the chunk's or its face neighbours' summaries, which a
stamped wall next to the hill has), else the open visibility 1; the samples then pull it at
`1 / (SEED_AGE + 1)` with the firefly clamp live from the first frame. The pair stays the one
history of the term; the seed is a value, not a second history. (ii) **A new Phase 21a: a
chunk the edit's box touches that holds no block is a dug chunk whose every brick is a dig
slot.** It enters `digs` at the edit, opens in `open_digs`' order (near first, then by distance
from the eye: the keep around the owner first), is listed on the opening thread as a dig with
no pre-listing is today (a chunk left with no face is faceless, as today), and lands as a dig
job. The landing of its sky words is a step (the amendment above: rule 3's degenerate case,
from a mean the seed and the samples have already pulled toward the baked value; Phase 20
(ii) is its home if the window asks for slower).

**Where the history lives and which fade carries each switch (ADR-0076's binding clause).**
The stamped face's history is the store's words once baked and the pair before; the switch
air-to-wall is the edit's own change (the frame the wall exists); wall-under-ray to
wall-baked is the landing's step. No screen-space history of the term exists but the pair's.

**Why both, and the baseline's rate.** (ii) alone brings the bake sooner at the RTX's rate but
the frames before it still sparkle, and at the baseline adapter's rate (the bake's slice is a
share of the frame and its throughput the adapter's) the window is longer, not shorter: the
sparkle's length is set by the bake, which is the wrong clock for it. (i) alone calms the ray
but leaves the keep ray-lit for seconds (a per-pixel ray over 30 000 pixels is the light
pass's cost on the slow adapter, 4.8 ms a ray in Plan 0032's reading) and screen-space (rule 2:
a turn loses it) until the pool's turn comes. (i) is per frame and does not care what the bake
costs; (ii) ends the ray region as fast as a dig's.

**Rejected.** (b) Candidate openings running while dig jobs fill the loop: a throughput change
that shortens the window on the RTX only, slows the digs the gate exists to put first, and
leaves the sparkle's mechanism in place; the stamped chunks are digs by the rule (an edit
touched them), so they take the dig's path instead of the pool's. (c) A calmer fallback for
pixels with no record than the per-pixel ray, or keeping them out of the exposure meter: a
fallback with no history is a held prior that is wrong for an interior and lands in a step,
which is what the seed does on its first frame and then corrects; the meter swings because
the pixels it meters sparkle, so the seed removes the swing at its source; and excluding
no-record pixels would leave a block-less rig (Phase 21 bar 1's open floor under
`Sky::uniform`) with no metered pixel and a held exposure. A provisional block for the
stamped chunk at the edit (the dug chunk's path): its listing on the edit's frame is the
pool's listing moved onto one frame (the castle's 719 chunks, a guess of over 100 ms), and its
only gain over the chain's value is a per-brick emissive guess; the seed's chain source covers
the wall next to the hill and the open prior the rest. A spatial seed (the 3 x 3 accepted
neighbours' pair, which converges at once for a disoccluded sliver next to settled pixels) is
the upgrade if the forest's turn strip stays over its bar after (i): in Followups, not built
here. No ADR: both are plan-level mechanisms under ADR-0070 (the long-history ray's start
value) and ADR-0046 (the dig path); ADR-0076 is proposed on main and not edited here.

### 2. The tree test: `Frame::reset_history` is fixed and the rig resets; the claim stands

The regression is the rig's carried history (the readings log's attribution: the floor's
history accepted across the world swap, `key_mean` at age 64 moving 1/65 a frame, the
shadow's summed weight 27.6 against 96.2) and `Frame::reset_history` undone by `Frame::step`'s
`advance()`, so the rig's reset did nothing. **(a):** `reset_history` also clears `encoded`
(`live/reset-history-fix.patch`, a `fix` commit under Phase 19 that touches
`crates/vxe-render/src/frame/mod.rs`, outside the phase's files, with a unit test that pins
the claim: after `reset_history` between two `step` calls the next frame's history age reads
0), and `tree_shadow` resets before each of its reads; the claim as written stands (the dev's
run with both: ratio 1.733). **Rejected:** (b) 64 settle frames before each read moves the
reference instead of fixing a public method that is a no-op in the common calling pattern,
and adds about 190 lavapipe frames to a gate test; (c) a 0.2 floor on `key_mean` is `ALPHA`'s
five-frame memory again, the mechanism Phase 14 attributed the forest's grain to (the key
alone p99 13 at `60,180`), so it undoes Phase 19.

**The app's counterpart, accepted for this plan.** A shadow an edit casts outside its edit box
(a placed castle's on the hill; a removed occluder's lit patch) arrives over up to
`sky_history` frames (about 1 s at 60 fps) instead of about 16, monotonic, with no overshoot:
a fade, not a flicker, and longer than ADR-0076's candidate `F` (16, calibrated by the owner's
eye in Plan 0090 Phase 5; a guess until then). The edit box does not reset the key mean
outside itself: a sun shadow's influence region is unbounded (a tower's shadow is long), so a
box dilated by any reach is a guess, and a reset there restarts the grain Phase 19 removed on
every pixel in it. If the calibrated `F` comes out under the fade's length, the fix is the
mechanism's own: a band test on the key mean as `pair_step` has for the sky (a short mean
beside the long one, in a channel to be found) that drops the age when the samples leave the
band, planned where Plan 0090 reads it. Owner question 1 below.

### 3. Phase 19's other bars: accepted under `T_still`; the drift rig attributed or dropped

- **Bar 1, the penumbra's p99 1 to 2** (the lamp's 1; mean 0.340 to 0.355, exposure 3.13 to
  3.34): accepted. Two display levels at the 99th percentile is the smallest failure an
  integer statistic can show and lies under ADR-0076's starting `T_still` (4 levels, Plan
  0090); the mean moved 4 %. The bar is re-stated as p99 at or under 4 with the lamp's and
  the mean printed. Rejected: the a-trous lever (the spatial variance at a young age) with
  nothing attributing the level; Phase 19 allowed it only against bar 4, which is met.
- **Bar 3 at `60,180`, 3.00 against the lamp's 2.11** (8.16 before): accepted as a reading
  under `T_still`, not met against the lamp; the lamp stays printed as the reference; Phase
  25 writes the bar at the value the third window keeps, and Plan 0090's counter ratchets
  from there. The residual's split (the key mean's reset pixels against the leaf voxel's own
  jitter) is Plan 0090 Phase 2's per-pixel attribution, not another reading here. Owner
  question 2.
- **The drift rig, off about 13.5 voxels on both binaries** against a geometric move of 1.19:
  the rig, not the mechanism (the same wrong number before and after the phase; `set_sky`
  raises no sky-change bit on a drift, by `sky_changed`'s contract, so it is not a reset a
  frame). One bounded reading (30 minutes): both runs' centroids every 16 frames and the sun
  direction fed, on one binary. A rig fault with a small fix is fixed and the bar read; else
  the bar is dropped from this plan, the log says so, the property goes to Plan 0090's scene H
  (the sun sweep, whose stepping count reads the same lag), and
  `a_sun_jump_resets_the_direct_part` stays as the jump's test.

### 4. Phase 20's bars 1 and 2 read the drain's switch, as a reading

At 19.png's pose at rest no pixel is ray-lit under the default (the reading: 0 from frame
300), so "the two paths agree" and "the ray side is as still as the lamp" have no pixels at
rest, and Phase 25 bar 1 reads the rest state. What the owner saw at 19.png was the drain
(frame 120: 2863 ray-lit, 5373 chained), so the seam is a transition and ADR-0076 rule 3 is
its form. Bars 1 and 2 become one reading of the switch from the pair to the store per pixel
as the queue drains, taken after Phases 21 and 21a land (the switch's step is what they
change), with (i) built only if the relative step is over 10 % and (ii) only if pixels leave
the band; re-stated in the phase. Rejected: forcing a ray region at rest to keep the bars as
written (`column_wait on` baked 0 chunks in 600 frames; `LIGHTING_HOOK_SKY_RAY_ALWAYS`), which
measures a state the default never shows; dropping the bars with nothing read on the drain,
which is what the owner saw.

### 5. The order for the dev

1. Phase 19's fix pass: `reset_history` (the patch, its unit test), the rig's reset, the gate
   test green; bar 1 and bar 3 re-stated in the log; the drift rig's bounded reading.
2. Phase 21 (c): `live/p21-c.patch` and `live/p21-dig.rs` applied, the owed test moves
   (`centre_lit` and its users to `filtered()` or the pair), bars 1, 2, 5 and 6.
3. Phase 21 (a), the seed, on top of (c): bars 3, 4 and the seed's own bar; bar 7.
4. Phase 21a: the touched chunk with no block as a dig; its bars.
5. Phase 20's re-stated reading.
6. Phase 23 (`live/p23-defects.patch`, `live/p23-test.rs`).
7. The flicker strips' `after21` rows (`live/flicker.sh`): the hills, the forest, the dig and
   the paced castle, in the log's table.
8. The named gpu binaries at the tip (`gpu_sun`, `gpu_outdoor`, `gpu_outdoor_golden`,
   `--no-fail-fast`), then Phase 24's runner.

(c) goes before the seed because both edit `temporal.wgsl` and the seed is written on the
pass as (c) leaves it; the `reset_history` fix goes first because the gate is red until it
lands.

### Owner questions (the dev proceeds on the recommendation)

1. **A shadow an edit casts outside its edit box fades in over about 1 s at 60 fps (up to 64
   frames), monotonic, instead of about 16 frames.** Is that fade acceptable under "a flame
   may flicker, nothing else may" until Plan 0090 calibrates `F`? Recommended: yes; a longer
   fade is the price of the key light's long mean, which removed the forest's grain, and the
   band test above is the fix if the calibrated `F` is shorter. (Phase 24 run 4b watches it.)
2. **Two Phase 19 bars are accepted under ADR-0076's starting `T_still` (4 levels) rather
   than the lamp's reference the plan wrote** (the penumbra p99 2 against the lamp's 1; the
   forest floor at `60,180` p99 3.00 against the lamp's 2.11, from 8.16). Recommended: accept;
   Phase 25 pins the window's values and the counter ratchets from them, so neither can rise
   unseen.

## Implementation phases

Each phase ships as its own commit. `dev` implements all phases in one session with no review
between phases. The architect reviews once at the end, in a fresh session. Phases 1 to 7 are
**exploration** (lane rules 2b): the owner's window in Phase 8 is their gate; Phases 12 to 14
are exploration too, gated by the second window, Phase 15, while Phases 9 to 11 carry bars
that bite (the sky rule is a correctness fix, not a look). Phases 16 to 23 (the fix pass) carry
bars that bite where a defect is measured headlessly, and readings where the window judges;
Phase 24 is the third window and Phase 25 writes the picks as tests. Each lands its
CPU tests of data, determinism and API; every bar that judges the image runs as a reading from
a `mod reading`, recorded in the readings log, and Phase 16 writes it as a test against the
picks. The readings log is `docs/plans/logs/0086.md`. Per phase the gpu check runs only the
`gpu_*` binaries the phase names; a named test over 10 s on lavapipe is born
`#[ignore = "soak"]`. Every wall time below is an estimate.

**The rigs.** Two poses from Plan 0032's testbed, used throughout: the **tunnel** (`--source
outdoor --spawn inside --time 12:00`, the lamp off, seed 1) with two regions the Phase 1
reading pins by pixel rectangle at 320 x 180, the mouth's **rim band** (the sky ray's hit rate
about 0.3 to 0.6) and the **far end** (under 0.05); and the **forest** (`--spawn forest
--trees forest --sun 60,180`, `LEAF_FILL` 64) with the stepped ground's shadow edges. The
statistics are `apps/flythrough/tests/support/stillness.rs`'s (patches a step, sudden
changes a second, flicker in log2 a frame, over 64 still frames after 64 settle frames) and
Plan 0032 Phase 6's (the 99th-percentile frame-to-frame change in display levels), each
beside the headlamp on the same pose with the sky off, the reference the owner accepted.

### Phase 1: The attribution reading: the flicker split by term at the tunnel and the forest
- **Owner skill:** dev
- **What:** `reading::flicker_by_term` in `apps/flythrough/tests/gpu_outdoor.rs` runs both
  rigs through a matrix and prints one table: the term (the key light alone, by `sky_rays` 0;
  the sky ray alone, by a test hook that zeroes the key light's sample, listed in
  `NOT_SETTINGS`; both), the filter (off, on), the history (the temporal pass as it is; a hook
  that accepts history whatever the normal and material, to read how much the rejection
  contributes). Beside the statistics: per region, the sky ray's hit rate (the share of
  frames the sample is non-zero at the region's pixels) and the firefly rate (samples over
  four times the region's mean); at the forest, the share of lighting pixels whose history
  was rejected per frame, split by cause (normal, material, position), from a counter the
  temporal pass writes behind the steps view. FSR 2 at 1.5 and native both.
- **Files touched:** `apps/flythrough/tests/gpu_outdoor.rs`, `tests/support/stillness.rs`,
  `crates/vxe-render/src/shaders/temporal.wgsl` (the counter and the hooks),
  `frame/settings.rs` (the hooks, `NOT_SETTINGS`), `docs/plans/logs/0086.md`.
- **Done when:**
  1. The log holds the table, both poses, every cell, and names per pose the dominant term
     and the share the history rejection carries (a reading; no bar).
  2. The far end's hit rate and the rim's are read; `SKY_HISTORY_CAP` and `FIREFLY_K` in
     Phase 5 are set from them by the arithmetic in ADR-0070 (the log shows the step).
  3. The hooks and the counter cost nothing when off: the default frame's shaders are
     byte-identical in their WGSL output or the counter is a uniform branch (the log says
     which).

### Phase 2: A canopy shadows as its brick (the review's D1)
- **Owner skill:** dev
- **What:** a render-side per-material style word with a `SHADOW_AS_BRICK` bit. If Plan 0081
  has landed, it is a bit of its style table; otherwise this plan adds a `u32` word per
  material id beside the renderer's material upload (albedo, emission), set through
  `Frame::set_material_styles(&[u32])`, which Plan 0081 absorbs (a merge item named in both
  plans' logs). The shadow walk (every shadow ray: the key light's, the sky ray, the lamp's
  and the list's; not a primary ray and not the bake's emissive samples, whose light the owner
  did not complain of) on entering a non-empty brick reads one voxel's material (the brick's
  first set bit through the chunk's palette, a guess at the cheapest; `dev` may read the entry
  voxel instead) and, if its bit is set, returns a hit at the brick's entry. The testbed sets
  the bit on `leaf` (`scene.rs`); `--leaf-shadow brick|voxel` is the window's A/B (default
  `brick`), shown in the banner as `leaf shadow brick`.
- **Files touched:** `crates/vxe-render/src/shaders/walk.wgsl`, the material upload's
  module and `frame/pipelines.rs` (the word's binding, if 0081 has not landed),
  `frame/mod.rs` (`set_material_styles`), `apps/flythrough/src/scene.rs`, `cli/outdoor.rs`,
  `hud/modes.rs`, `crates/vxe-render/tests/gpu_sun.rs`, `apps/flythrough/tests/gpu_outdoor.rs`,
  `docs/flythrough.md`, `docs/plans/logs/0086.md`.
- **Done when:**
  1. **No fleck.** `a_flagged_canopy_shadows_as_its_bricks` in `gpu_sun.rs` (Plan 0032 Phase
     8 bar 5's tree with a canopy at `leaf_fill` 64, a zenith sun at the real radius, the key
     light alone, the coarse walk, 16-frame mean, 4 pixels a voxel; about 8 s): with the bit
     set, no floor pixel inside the projection of the canopy's occupied bricks reads over 1 %
     of the open value; with the bit clear, at least 3 % of those pixels read over 50 % (at
     fill 64 a tenth of the 2^3 blocks are holes, so a column of about 6 blocks is open with
     probability about `0.1^6` straight down, but the holes' flecks spread under the disc
     and the jitter; the 3 % is the floor of that reading, taken first and pinned in the log).
  2. **The silhouette grows by at most a brick.** Same test: with the bit set, every floor
     pixel more than 8 voxels outside the canopy's brick projection reads the open value
     within 1 %.
  3. **Nothing else moves.** With no material flagged, every existing golden is
     byte-identical (the existing golden tests), and `forest_vector`'s hash is unchanged (the
     flag touches no voxel).
  4. **The banner and the flag** (no adapter, under 1 s): the parse test pins
     `--leaf-shadow` and the banner reads `leaf shadow brick` with a sky set.
  5. **Readings:** the key ray's mean and p99 steps in the forest at `--sun 10,180` with the
     bit set and clear (Plan 0032 Phase 8's `--view steps` run), and the light pass's GPU
     time on both adapters; the forest's stillness statistics at `60,180` with the bit set
     and clear. No bar; the window judges.

### Phase 3: History across a stair in the far-normal regime (D2)
- **Owner skill:** dev
- **What:** where the far normal's blend weight is over one half (ADR-0042: the voxel is
  smaller on screen than the blend's lower bound), (i) the temporal pass's acceptance test
  compares the smoothed normal of the previous and the current hit, not the face normal, and
  drops the material test for the irradiance history (the direct target holds irradiance
  before albedo, so a tread of `grass` and a riser of `soil` share a history; the stored
  emissive part has no history to keep); (ii) the key ray and the sky ray start from the hit
  lifted along the smoothed normal by `LIFT` voxels times the blend weight (`LIFT` 1.0, a
  guess), and the walk skips the cell the lifted origin lands in if it is solid, so a stair
  smaller than a pixel casts no self-shadow. Near the camera the weight is 0 and nothing
  changes.
- **Files touched:** `crates/vxe-render/src/shaders/temporal.wgsl`, `sky_light.wgsl`,
  `walk.wgsl` (the skip), `crates/vxe-render/tests/gpu_sun.rs`,
  `apps/flythrough/tests/gpu_outdoor.rs`, `docs/plans/logs/0086.md`.
- **Done when:**
  1. **The staircase keeps its history.** `a_far_staircase_keeps_its_history` in
     `gpu_sun.rs` (a stepped floor of 64 one-voxel steps seen from a distance at which a
     voxel is under the far normal's lower bound in pixels, under FSR 2 at 1.5, a still
     camera, a zenith sun; about 8 s): Phase 1's rejection counter over the staircase's
     lighting pixels reads under 1 % a frame over 64 frames; the same rig before this phase
     is read and printed (the review expects tens of percent).
  2. **A thin far wall does not leak.** `a_thin_far_wall_does_not_leak` in `gpu_sun.rs`
     (a one-voxel wall 40 voxels tall on the floor at the same distance, the sun 20 degrees
     above the horizon on the wall's far side; about 8 s): of the floor pixels in the
     wall's shadow band between 1 and 3 pixels from the wall's screen position, under 1 %
     read over 10 % of the open value. The leak stays within a pixel. The same at `LIFT` 2
     is read and printed, no bar.
  3. **The near regime is untouched.** Every existing golden whose scene has no far-normal
     regime pixel stays byte-identical; a golden that has (the tester's castle gate, if its
     far walls qualify) is re-blessed as a disclosed change and the log names it with the
     difference count.
  4. **Reading:** the forest's stillness statistics at `60,180` with Phase 2's bit set,
     before and after this phase; Plan 0032 Phase 9's Image #3 foreground (the hills' lit
     treads) at the noon pose as a before/after pair of stills in the log's words.

### Phase 4: The sky visibility baked per brick direction (C1, ADR-0070)
- **Owner skill:** dev
- **What:** the store's record gains eight bytes: six `u8` visibilities and two spare. The
  bake job draws `SKY_BAKE_RAYS` (256, a guess) cosine-weighted rays per direction from
  points on that direction's faces, the frame's shadow walk, capped at the chunk's reach in
  voxels (`light_radius` times 64), beyond the cap sky, below the horizon blocked; the
  channel bakes only while the frame holds a `Sky` (`Frame::set_sky(Some)`), else its words
  are zero. A dig re-bakes it with the emissive words in the queue's rings; ring 3 opens also
  when a touched brick's visibility was over zero before or after. The sky-view pass (or a
  pass after it) reduces the LUT to six cosine-weighted axis irradiances `E_sky` in the sky
  uniform. The temporal pass, where it writes the store's emissive part, adds
  `v(n) E_sky(n) exposure_now()` with `v` the bilinear read of the hit face's direction and
  `E_sky(n)` the axis value, blended for a smoothed normal by the normal's squared components.
  The direct target no longer receives the sky ray at a pixel with a fresh value (Phase 5
  finishes the switch; in this phase the ray still runs everywhere and the reading compares).
- **Files touched:** `crates/vxe-render/src/shaders/bake.wgsl`, `temporal.wgsl`,
  `sky_view.wgsl`, `sky.wgsl`, `sky_atmosphere.wgsl`, `crates/vxe-render/src/bake/*` (the
  record, the job, the rings' test, the summary), `frame/sky.rs`, `frame/settings.rs`,
  `apps/flythrough/tests/gpu_bake.rs`, `crates/vxe-render/tests/gpu_sun.rs`, `gpu_sky.rs`,
  `docs/nfr/03-memory.md`, `docs/plans/logs/0086.md`.
- **Done when:**
  1. **The bake does not see the sky, still.** `a_baked_face_ignores_the_sky` (Plan 0032
     Phase 5 bar 8, `gpu_bake.rs`) keeps its claim on the emissive words, and the six
     visibility words are zero with `Sky` `None`.
  2. **The channel reads the open fraction.** `the_sky_channel_reads_the_open_fraction` in
     `gpu_bake.rs` (about 8 s): an open floor's `+z` reads 255 within one step; a vertical
     wall's outward direction in the open reads 0.50 within 0.03 (half the cosine-weighted
     hemisphere about a horizontal normal is sky, by symmetry); the floor brick centred under
     a 16 x 16 slab 20 voxels up reads 0.83 within 0.05 (the form factor from the centre
     point to the slab is four corner rectangles of 8 x 8 at height 20, each
     `(1 / 2 pi) [2 x (0.4 / 1.077) atan(0.4 / 1.077)] = 0.0420`, so 0.168 blocked; the
     brick's faces off centre are blocked a little less, inside the tolerance); the floor of
     a closed rock box with 5-voxel walls reads 0 exactly.
  3. **The baked term matches the ray.** `the_baked_sky_term_matches_the_ray` in `gpu_sun.rs`
     (`Sky::uniform(1.0)`, filter off, exposure 1, the queue drained; about 10 s): the open
     floor's baked sky term reads `pi` within 3 % (Plan 0032 Phase 6 bar 2's ray read
     3.1406), the wall's `pi / 2` within 5 %, the closed box's 0.
  4. **`E_sky` is the LUT's integral.** `e_sky_is_the_cosine_mean_of_the_lut` in
     `gpu_sky.rs` (about 4 s): under the default noon sky the `+z` value equals the
     cosine-weighted numeric integral of the read-back sky-view LUT times `pi` within 2 %,
     and under `Sky::uniform(L)` the six values read `pi L`, four of `pi L / 2`, and 0.
  5. **Deterministic.** ADR-0046's bake-identity test covers the new words: a chunk's record
     is bit-identical whatever the slicing and the job order.
  6. **Still at rest.** `reading::the_tunnel_at_rest_under_the_bake` in `gpu_outdoor.rs`: the
     tunnel pose with `sky_rays` 0 (Phase 5 makes 0 legal; here a test hook) after the queue
     drains reads 0 patches a step over the rim and the far end (nothing moves: a stored value
     with no history), and the two regions' mean luminance against the ray's 64-frame mean
     from Phase 1, printed.
  7. **Cost readings:** the pre-roll's wall time and the queue's step CPU p50 on the hills
     and in the forest with and without the channel, both adapters (Plan 0032 Phase 9's
     `bake queue:` lines); the store's live bytes on the hills and in the forest (Plan 0032
     read 8.8 to 15.5 MB; the record grows by about 29 %); the bake's samples a second. In
     `docs/nfr/03-memory.md` and the log; the `--bake-file` hash changes once.

### Phase 5: The rays' own long history, and the sky ray only where the bake is stale (C2)
- **Owner skill:** dev
- **What:** the **visibility pair**, two `rgba16float` on the lighting grid (r the sky
  visibility's running mean, g the key light's (unused until Phase 6), b the sky's short mean
  over 8 frames, a the age), reprojected with the direct accumulation's reprojection and reset
  by its tests (an edit box, no history, the sky-change bit, Phase 3's acceptance). The sky
  ray scores 1 or 0 (a visibility, coloured by `E_sky(n)` at read); the mean runs to
  `SKY_HISTORY_CAP` (64, a guess Phase 1 sets); a sample is clamped to `FIREFLY_K` times the
  mean once the age passes 8 (4, a guess); when the short mean leaves the long mean's
  three-sigma band under the binomial estimate from the long mean and its age, the age falls
  to 8. The light pass casts the sky ray only where the hit brick has no fresh visibility: no
  store block, or a `SKY_STALE` bit in the record set at the dig for the rings' bricks and
  cleared when their bake lands; the switch from the pair's mean to the stored value fades
  over the store's fade. `sky_rays` 0 is legal: bake only. The direct target no longer
  receives the sky ray.
- **Files touched:** `crates/vxe-render/src/shaders/temporal.wgsl`, `shade.wgsl`,
  `sky_light.wgsl`, `bake.wgsl` (the stale bit), `crates/vxe-render/src/frame/targets.rs`
  (the pair), `frame/encode.rs`, `frame/settings.rs` (`sky_rays` 0, `sky_history`),
  `bake/*` (the stale bit's set and clear), `apps/flythrough/src/cli/outdoor.rs`,
  `hud/modes.rs`, `crates/vxe-render/tests/gpu_sun.rs`, `apps/flythrough/tests/gpu_outdoor.rs`,
  `gpu_upscale.rs` (`frame_bytes`), `docs/nfr/03-memory.md`, `docs/plans/logs/0086.md`.
- **Done when:**
  1. **The mean converges and holds.** `the_visibility_pairs_mean_holds_still` in
     `gpu_sun.rs` (a floor point at the foot of a half-plane roof's edge, 20 voxels up, under
     `Sky::uniform(1.0)`, a hit rate of 0.5 by the form factor of a half plane; the ray
     forced on by a hook; about 8 s): after frame 64 the pair's mean at that pixel reads 0.5
     within 0.1, and its frame-to-frame change is at most `1 / 64` (the running mean's step
     at the cap), against 0.2 on the direct target's blend.
  2. **A firefly is clamped.** `a_rare_hit_is_clamped` in `gpu_sun.rs` (a floor point under
     a roof with a 6 x 6 hole 20 voxels straight above, form factor `36 / (400 pi) = 0.029`;
     about 8 s): over frames 9 to 128 the pair's mean never exceeds 0.25 (a hit of 1 is
     clamped to `FIREFLY_K` times a mean near 0.03), and the same rig with the clamp off is
     read and printed (a hit frame reads near 1 / age). The dark bias is printed beside it:
     the clamped mean against 0.029.
  3. **A moving shadow resets.** `a_moving_shadow_edge_resets_the_history` in `gpu_sun.rs`
     (Plan 0032 Phase 5's 4 x 4 pillar moved one voxel a frame across the open floor under a
     uniform sky; about 8 s): every floor pixel the shadow leaves reads at least 90 % of the
     open value within 12 frames (the open ground's sigma is 0, so the first non-zero sample
     resets; 12 is Plan 0032 Phase 5 bar 6's settle).
  4. **The ray runs only where the bake is stale.** `the_sky_ray_runs_only_where_stale` in
     `gpu_sun.rs` (the open floor baked, the steps view's sky-ray counter; about 10 s): at
     rest the counter reads 0 sky rays; after a 1 m dig in the floor it reads non-zero over
     the dug rings' pixels and returns to 0 once their re-bake lands; the frame count is
     printed. With `sky_rays` 0 it reads 0 throughout, and the dug hole's floor takes its
     baked value when the re-bake lands and nothing before.
  5. **The flag, the banner and the bytes** (no adapter, under 1 s): the parse test accepts
     `--sky-rays 0` and the banner reads `sky rays 0`; `frame_bytes` grows by exactly the
     pair's bytes (two targets of 8 B a lighting pixel) at the test's size.
  6. **Readings:** the tunnel's and the forest's statistics (Phase 1's matrix at the
     defaults) after this phase; the light pass on both adapters at rest on the hills with the
     queue drained (Plan 0032 read 10.5 ms on the RADV RENOIR and 1.04 on the RTX; the
     expectation is the key ray's share alone, a guess of about 5.7 and 0.8) and during a dig.

### Phase 6: The key light on the long history (D3), after a reading
- **Owner skill:** dev
- **What:** first the reading: with Phases 2 to 5 in place, the penumbra's frame-to-frame
  change p99 at 3 degrees on Plan 0032 Phase 5 bar 3's slab rig (the penumbra on the floor
  `2 x 20 x tan 3 = 2.1` voxels wide, 8.4 pixels at 4 a voxel) and on the forest's shadow
  edges at `60,180`, against the lamp's on the same floor. **If the penumbra's p99 is at or
  under the lamp's, this phase stops at the reading and the key light stays on the direct
  target** (the log says so; the mechanism stays unbuilt, not a setting). Otherwise the key
  light's visibility (the shadow ray's 0 or 1; `key_light_at` stays exact) moves to the
  pair's `g` on the same running mean, clamp and reset, and the direct target receives
  `key_light_at(p, n) x g`.
- **Files touched:** `temporal.wgsl`, `shade.wgsl`, `sky_light.wgsl`,
  `crates/vxe-render/tests/gpu_sun.rs`, `apps/flythrough/tests/gpu_outdoor.rs`,
  `docs/plans/logs/0086.md`.
- **Done when:**
  1. The reading is in the log with the decision it made.
  2. If built: `the_penumbra_holds_still_on_the_long_history` in `gpu_sun.rs` (the slab rig
     at 3 degrees, FSR 2 at 1.5; about 10 s): after 64 frames the penumbra band's p99
     frame-to-frame change is at or under the lamp's on the same floor.
  3. If built: **a drift lags under half a voxel.** On Plan 0032 Phase 8 bar 5's tree rig,
     the sun drifting at the testbed's day (`2 pi / 1200` rad a second, 0.005 degrees a frame
     at 60 fps) for 64 frames moves the shadow's centroid by the geometric amount within 0.5
     voxel (the running mean's lag is about half the cap, 32 frames, 0.16 degrees, `80 x tan
     0.16 = 0.22` voxels at the canopy's height).
  4. If built: Plan 0032 Phase 5 bar 6 still holds (a 10-degree jump resets the pair through
     the sky-change bit; 90 % of the new lit value 11 frames later), and bar 3 (`a_moving_
     shadow_edge_resets_the_history`) holds for the key light's shadow too.

### Phase 7: A spike: a deterministic penumbra from the cell hierarchy's closest approach (D4)
- **Owner skill:** dev
- **What:** a spike behind `lighting.penumbra approach|disc` (`disc` the default; the
  banner reads `penumbra approach`): one centre ray toward the body that tracks, along the
  coarse walk, the smallest distance `d_min` from the ray to an occupied cell it passes
  beside, at the depth `t` where it occurs, and returns a visibility
  `clamp(d_min / (t tan r), 0, 1)` with `r` the body's angular radius (the SDF soft-shadow
  rule on the cell hierarchy). The DDA knows the cells it enters, not the ones beside it, so
  the spike reads the neighbouring 2^3 blocks' bits at each coarse step (a guess at the
  cost: four to eight loads a step). Time-boxed: the spike ends at its readings whatever they
  read, and promises no mechanism.
- **Files touched:** `walk.wgsl`, `sky_light.wgsl`, `frame/settings.rs`, `settings.rs`,
  `apps/flythrough/src/cli/*`, `hud/modes.rs`, `crates/vxe-render/tests/gpu_sun.rs`,
  `docs/plans/logs/0086.md`.
- **Done when:**
  1. **The profile against the disc.** `reading::approach_penumbra_against_the_disc` in
     `gpu_sun.rs`: Plan 0032 Phase 5 bar 3's slab rig at 5 degrees, the approach visibility
     at one frame against the disc-sampled 64-frame mean (the reading: a 5 % to 95 % rise of
     2.75 voxels); the profile's deviation in voxels along the row and the width of its rise,
     printed; the same at 3 degrees and under the canopy at `60,180`.
  2. **Stillness and cost:** the forest's statistics with `approach`; the key ray's steps
     and the light pass on both adapters with `approach` and `disc`.
  3. The setting exists, shows its mode, and changes nothing with `disc`. The go or no-go
     is the owner's, at Phase 8.

### Phase 8: The owner's windowed check
- **Owner skill:** human
- **What:** the owner stands in the tunnel at noon and looks at the rim and the far end,
  with `--sky-rays 1` and `0`; digs a skylight into the chamber and watches the chamber's
  floor take the light (the ray first, the bake as it lands); walks the hills at noon and
  watches the far ground while moving (Plan 0032 Phase 10's rule) and the hills' stairs in
  the foreground (Phase 3); stands in the forest at `60,180` and `10,180` with
  `--leaf-shadow brick` and `voxel`, and runs the day from 15:00 to sunset; looks at a
  penumbra at 3 degrees with and without Phase 6 (if built) and with `--penumbra approach`;
  reads the HUD's light pass and the bake queue line through the settle. A ping reaches the
  owner when the window is ready, and the fly-through's stdout stays visible.
- **Files touched:** `docs/plans/logs/0086.md`.
- **Done when:** the owner has recorded a verdict on the tunnel's rim and far end at rest;
  on the dug skylight's lag; on the far ground while moving; on the stairs' speckle; on the
  canopy's shadow as a brick; on the penumbra's grain, and whether Phase 6's history stays
  on by default if it was built; a go or no-go on the approach penumbra; and on any cost the
  HUD showed that the owner will not pay.

### Phase 9: The sky rule: the column map, and sky only above it, in the bake and the light pass
- **Owner skill:** dev
- **What:** ADR-0070's amendment of 2026-11-18. The renderer keeps a **column map**: a
  camera-centred 2D map over the mirror's footprint whose texel is the highest solid voxel's
  `z` in its column, from the resident chunks (a full chunk's bricks exactly; a LOD chunk's
  mips, conservative by at most `2^L - 1` voxels), `none` where the column holds no solid. A
  per-chunk column sheet (64 x 64 tops, computed where the chunk is packed or on its upload)
  feeds it; a chunk's arrival, departure or edit marks its column tile dirty and a small pass
  takes the max down the column. Texel size and layout are `dev`'s (2-voxel cells over the
  footprint are about 4.5 MB at the fly-through's residency: a 47-chunk footprint is 3008
  voxels, `1504^2 x 2 B`; 4-voxel cells a quarter of that). **The rule, one function in
  `walk.wgsl` used by both the bake and the light pass:** a sky ray that ends with no hit, at
  its length cap, on entering an absent chunk or on leaving the window's side, has reached the
  sky iff its end point's `z` is above its column's top (the window's top face is above every
  top; the side clamps to the edge column); a ray below the horizon stays blocked. A column
  not yet final (a chunk of it within the window absent) reads the resident chunks' top
  (question 4 (a), unless the owner picks (b)). **The bake's reach rule gains finality:** a
  chunk bakes when its reach is resident (as today) and every column tile over the reach's
  footprint is final, so the bake's input is the same whatever the slicing and the order
  (ADR-0046's identity). `SKY_RAY_VOXELS` and the bake's cap stay as they are; the longer cap
  was rejected (the amendment's Alternative C).
- **Files touched:** `crates/vxe-render/src/shaders/walk.wgsl`, `bake.wgsl`, `sky_light.wgsl`,
  a new `column_map.wgsl` and `src/column_map.rs` (or inside `mirror/`), `src/mirror/*` (the
  per-chunk sheet), `bake/queue/mod.rs` (`reach_ready`), `frame/targets.rs`, `frame/encode.rs`,
  `crates/vxe-render/tests/gpu_sun.rs`, `apps/flythrough/tests/gpu_bake.rs`, `gpu_outdoor.rs`,
  `gpu_outdoor_golden.rs`, `docs/nfr/03-memory.md`, `docs/plans/logs/0086.md`.
- **Done when:**
  1. **The map is the top of solid.** `the_column_map_is_the_top_of_solid` in `gpu_sun.rs`
     (a 3 x 3 x 3-chunk stepped terrain, every column's top known from the world; about 4 s):
     every texel over a full chunk equals its column's highest solid `z`; over a LOD-only
     chunk it is at least that and at most `2^L - 1` more; a column of air reads `none`; a
     1-voxel dig of a column's top lowers its texel within 8 frames of the edit's upload, and a
     stamp above it raises it on the frame the stamp uploads.
  2. **An enclosed pocket longer than the cap reads no sky.**
     `a_sealed_cavern_wider_than_the_cap_reads_no_sky` in `gpu_bake.rs` (a 288-voxel air cube
     sealed in rock, a 7 x 7 x 7-chunk world, the floor's central chunk baked under
     `Sky::uniform(1.0)`; from the floor an upward ray meets the ceiling at 288, past the 256
     cap; about 10 s, born `#[ignore = "soak"]` if over): every baked visibility of the floor's
     bricks reads 0 exactly; the same rig under the rule as built is read once and printed
     (the expectation is near 1 for `+z`).
  3. **The underground eye reads nothing.** `the_underground_eye` at `VXE_UNDER_R=7`
     (`gpu_outdoor.rs`, a reading today, over 10 s: the bar is a `#[ignore = "soak"]` test,
     run once by name under the lock): the near-eye baked visibility's mean per face is under
     0.001 in every direction, the stored part under 0.02 and the direct part under 0.02 at
     exposure 1 in all six views (today 0.0063 to 2.75 and 0), with the surface reference's
     stored and direct parts within 5 % of today's 149.7 and 1345.
  4. **The open ground does not move.** Phase 4 bar 2's three values (255 within a step, 0.50
     within 0.03, 0.83 within 0.05), bar 3's `pi` within 3 % and `an_absent_chunk_is_sky_to_
     the_sky_ray` as written (its absent chunk is above the floor's top) still hold; its twin
     `an_absent_chunk_below_the_column_top_is_rock_to_the_sky_ray` in `gpu_sun.rs` (the floor
     under a resident roof chunk, the chunk beside the floor's absent; about 4 s): the floor
     reads under 1 % of `pi`.
  5. **The sealed cave stays dark.** `the_cave_mouth_is_lit_from_outside`'s seal bar
     (`gpu_outdoor_golden.rs`, soak) passes at its 2 % as written, run once by name.
  6. **Deterministic.** ADR-0046's bake-identity test holds with the map in the bake's input,
     and a chunk whose reach's columns are not final is not baked (a unit test of
     `reach_ready` with one absent chunk high in a column; under 1 s).
  7. **Readings:** the tunnel's stations as the diagnosis read them (`the_tunnel_bake_by_
     station`: `+z` and `-x` per station against the "as built" and "cap 4096" columns); the
     light pass on both adapters on the hills at rest and in the forest (Phase 7's runs); the
     pre-roll's wall time and the queue's step against Phase 4's table (the finality wait);
     the map's bytes and its pass's time while flying the hills path. In the log and
     `docs/nfr/03-memory.md`.

### Phase 10: The first seconds after a jump: the young pair and the black cells
- **Owner skill:** dev
- **What:** first the reading, after Phase 9: in the streamed underground rig
  (`the_underground_eye_streamed`) the pair's mean and the direct part per frame for 240
  frames; in the forest after a jump (`--spawn forest --sun 10,180`, the fly-through headless
  from frame 0) the HUD's `missing` and `black` counts per frame, and what a black face shows
  (the sky term's source per lighting pixel: the pair, a stale record, nothing). Then the
  fixes the reading names, from this list and no further: the variance reset's band at a young
  age (if the false-sky memory outlives the leak); the missing-light chain carrying the sky
  channel (a chunk summary's mean visibility per direction, read where a face has no block, so
  a young chunk shows its neighbours' or parent's sky instead of a black or a raw young ray);
  the coarse first pass (`coarse 8/256 first`) baking the channel at its sample count with
  `VIS_BAKED` set only by the full pass.
- **Files touched:** `temporal.wgsl`, `bake/summary.rs`, `bake/store.rs`, `bake/queue/*`,
  `apps/flythrough/tests/gpu_outdoor.rs`, `crates/vxe-render/tests/gpu_sun.rs`,
  `docs/plans/logs/0086.md`.
- **Done when:**
  1. **The young pair forgets the leak.** `the_underground_eye_streamed` (soak, by name): the
     direct part reads under 0.02 at frame 240 (today 0.122) and under 0.05 by frame 60; the
     frame-1 value is printed (0.194 today; question 4's pick decides whether it falls).
  2. **A young face is not black for long.** `a_young_chunk_reads_its_neighbours_sky` in
     `gpu_sun.rs` (an open floor of 3 x 3 chunks, the centre chunk's block withheld by a hook
     while its neighbours are baked; about 6 s): the centre's faces read the neighbours' sky
     term within 10 % from the first frame the chain finds them, and the HUD's `black` count
     for the rig reads 0 once the neighbours' blocks exist.
  3. **Reading:** the forest after a jump: frames until `black` falls under 1 000 and until
     `missing` falls under 5 000, before and after this phase, both adapters, in the log.

### Phase 11: The window's defaults
- **Owner skill:** dev
- **What:** `LeafShadow::Voxel` is the fly-through's default (`--leaf-shadow brick` stays;
  the banner names the non-default `leaf shadow brick`); `Penumbra::Disc` stays the default
  and `approach` stays a setting (it costs nothing with `disc`: a uniform branch, Phase 7's
  done-when 3); `sky_rays` 1 stays. The goldens these move are re-blessed as a disclosed
  change with the difference counts in the log, once, after Phase 9 (so the sky rule's
  change and the leaf default's land in one re-bless).
- **Files touched:** `apps/flythrough/src/scene.rs`, `cli/outdoor.rs`, `cli/tests.rs`,
  `hud/modes.rs`, `docs/flythrough.md`, the `golden/` images Phase 9 and this phase move,
  `docs/plans/logs/0086.md`.
- **Done when:** the parse test pins `voxel` as the default and `--leaf-shadow brick` as
  the A/B with its banner word; `outdoor_styles(LeafShadow::Voxel)` sets no bit; every
  outdoor golden passes at the tip and the log names each re-blessed one with its difference
  count and the mechanism that moved it (Phase 9's rule or this default).

### Phase 12: The tunnel's brightness: the eye's ceiling anchored to the sky
- **Owner skill:** dev
- **What (exploration):** the auto exposure's ceiling becomes a call the owner can take by
  eye. Today `AutoExposure::max` is a constant (ADR-0033's eye 2^4; the fly-through's outdoor
  start sets `OUTDOOR_MAX` 4, +2 EV), so a -11.9 EV tunnel is lifted by the whole ceiling and
  the walls at its mouth read mid-grey. Question 1's arms, all built as settings so the window
  compares them on one build: (a) `exposure.sky_anchor_ev` `N`: while the frame holds a `Sky`,
  the ceiling is `2^N` times the exposure that puts a sunlit horizontal face of albedo 0.5 at
  the key (`key / (0.5 (E_sun cos theta_s + E_sky(+z)) / pi)`, the irradiances from the sky
  uniform, so at night the anchor falls with the sun and the eye opens); off (`none`) is the
  constant ceiling; (b) `exposure.max_ev` as a plain setting (the constant, exposed); (c) is
  not built (a tone-curve toe is a look change on every scene; it goes to the backlog if the
  owner asks for it). The HUD's exposure line shows the ceiling in force and its source
  (`ceiling +2.00 EV (anchor 4)`).
- **Files touched:** `crates/vxe-render/src/exposure.rs`, `shaders/exposure.wgsl`,
  `frame/sky.rs`, `settings.rs`, `apps/flythrough/src/cli/*`, `hud/stats.rs`,
  `crates/vxe-render/tests/exposure.rs` (CPU), `docs/flythrough.md`, `docs/plans/logs/0086.md`.
- **Done when:**
  1. **The anchor follows the sky.** `the_ceiling_follows_the_sky` (CPU, under 1 s): under
     `Sky::uniform(L)` with no sun the ceiling at `N` 0 equals `key pi / (0.5 pi L)` within
     1e-6; doubling `L` halves it; `N` 2 quadruples it; with the sky `None` the ceiling is
     `AutoExposure::max` unchanged, byte for byte in the exposure state.
  2. **The tunnel meters at the ceiling and the ceiling is lower.** `reading::the_tunnel_
     under_each_ceiling` in `gpu_outdoor.rs`: the tunnel pose's metered exposure and the rim
     band's and far end's mean display luminance at `N` 0, 2, 4, 6 and `none`, and the same
     on the hills at noon (which must not move: the eye sits under the ceiling there) and at
     the forest sunset (where the anchor must not clip the eye: the sunset's exposure against
     Plan 0032's `outdoor_forest_sunset` golden's, within 0.5 EV at `N` 4). Printed; the
     window picks.
  3. The settings exist, show their mode, and `none` with `max` 4 renders the tunnel pose
     byte-identical to before this phase.

### Phase 13: The zebra stripes: the sky term's normal and a bounce floor
- **Owner skill:** dev
- **What (exploration):** two settings, both arms of question 2, read on the tunnel's stairs
  and the hills' foreground treads. `lighting.sky_normal face|smoothed`: with `smoothed` the
  sky term's six-direction blend uses the 27-count gradient's squared components at every
  distance, not only past the far normal's lower bound (ADR-0042's blend, extended for this
  one term; the surface and the key light keep the face normal), so a tread and a riser of a
  one-voxel stair read the slope's blend; the per-material style word may opt a material out
  (a cliff of real 1-voxel ledges). `lighting.sky_bounce b` in [0, 1]: the sky term is
  `(1 - b) v(n) + b mean(v)` with `mean(v)` the brick's six visibilities' cosine-weighted mean
  about `n` (a crude bounce from the brick's neighbourhood, one more read, no new ray; 0 is
  today). A stripe metric for the readings: at the stair's contour pixels after 64 frames,
  the ratio of the treads' mean display luminance to the risers' (today's is printed first).
- **Files touched:** `temporal.wgsl`, `shade.wgsl` (if the gradient is not at hand near),
  `frame/settings.rs`, `settings.rs`, `apps/flythrough/src/cli/*`, `hud/modes.rs`,
  `crates/vxe-render/tests/gpu_sun.rs`, `apps/flythrough/tests/gpu_outdoor.rs`,
  `docs/flythrough.md`, `docs/plans/logs/0086.md`.
- **Done when:**
  1. **A stair reads one slope.** `reading::a_one_voxel_stair_reads_one_slope` in
     `gpu_sun.rs` (a 45-degree staircase of one-voxel steps under a half-plane roof so `+z`
     and the horizontal differ by design, `Sky::uniform(1.0)`, the queue drained; about 6 s):
     the tread-to-riser ratio under `face` (printed; the geometry predicts over 3), under
     `smoothed` (within 1.2 of 1 is the expectation) and under `sky_bounce` 0.3; a flat open
     floor's value is unchanged under every setting within 1 %.
  2. **Readings:** the tunnel's stairs and the hills' foreground at noon under each arm, the
     stripe metric and Phase 1's stillness statistics; the temporal pass on both adapters
     under `face` and `smoothed`. The window picks.
  3. The settings exist, show their mode, and `face` with `sky_bounce` 0 renders every
     golden byte-identical.

### Phase 14: The forest floor: the grain's attribution, the key light on the pair, the canopy's transmittance
- **Owner skill:** dev
- **What (exploration):** first the reading Phase 6's slab did not take: in the forest at
  `60,180` and `10,180` with voxel leaves after the queue drains, Phase 1's matrix on the
  floor's lighting pixels (the key light alone by `sky_rays` 0 and a fresh store; the sky
  alone by the key hook; both), the share of floor pixels whose key sample flips between
  frames, and the floor's level split into the stored sky part and the key's direct part under
  `brick` and `voxel`, against the open ground's. Then: **if the key light's flips carry the
  grain** (the expectation), Phase 6's mechanism is built as it was specified there (the key
  visibility on the pair's `g`, the same mean, clamp and reset; Phase 6's bars 2 to 4 become
  this phase's), else the dominant term's fix is named in the log and routed back to the
  architect. **The canopy's third arm** (question 3 (a)): `--leaf-shadow absorb` sets a
  per-material shadow absorption in the style word; the shadow walk through a flagged brick
  runs ADR-0047's translucent walk and attenuates by `exp(-sigma x)` over the path `x` through
  the material's voxels, a continuous transmittance with no fleck; `sigma` from the testbed's
  leaf (a guess of 0.15 a voxel: about 5 % through 20 voxels). The bake's sky rays take the
  same walk, so the baked floor under a canopy darkens with the key light.
- **Files touched:** `walk.wgsl`, `sky_light.wgsl`, `bake.wgsl`, `temporal.wgsl`,
  `shade.wgsl`, `frame/settings.rs`, `apps/flythrough/src/scene.rs`, `cli/outdoor.rs`,
  `hud/modes.rs`, `crates/vxe-render/tests/gpu_sun.rs`, `apps/flythrough/tests/gpu_outdoor.rs`,
  `docs/flythrough.md`, `docs/plans/logs/0086.md`.
- **Done when:**
  1. The attribution table is in the log with the term it names at each sun angle.
  2. If built: `the_penumbra_holds_still_on_the_long_history` and Phase 6's bars 3 and 4 as
     written there, plus the forest: `reading::the_forest_floor_at_a_low_sun` (the forest at
     `10,180`, 128 frames at 320 x 180; about 20 s, soak): the floor's 99th-percentile
     frame-to-frame change against the lamp's floor, printed, both before and after.
  3. **The absorb arm transmits by depth.** `a_canopy_absorbs_by_path_length` in `gpu_sun.rs`
     (Phase 2 bar 1's tree, a zenith sun, the key light alone; about 8 s): the floor under the
     canopy's centre reads `exp(-sigma x)` of the open value within 15 % for the canopy's
     mean occupied depth `x` (computed from the world), with no floor pixel over 50 % of the
     open value inside the brick projection (no fleck), and the silhouette grows by at most a
     brick as bar 2 asked.
  4. **Readings:** the floor's stillness and level under `brick`, `voxel` and `absorb` at
     both sun angles; the light pass on both adapters under each (the translucent walk's
     cost in the forest is the number that decides whether `absorb` may be a default).
  5. The arms exist, show their mode, and `voxel` renders byte-identical to Phase 11's tip.

### Phase 15: The owner's second window
- **Owner skill:** human
- **What:** a runner mirroring the failed runs, in the same order, on one build at the tip
  of Phase 14, the fly-through's stdout visible and a ping when ready: (1) the tunnel at noon
  before and after the sky rays, the rim and the far end, then deeper to 17.png's position
  (`743 131 15`), the HUD's scene EV read aloud; (2) the same under each ceiling of Phase 12
  (`N` 0, 2, 4, 6, `none`); (3) the tunnel's stairs and the hills' foreground under Phase 13's
  `face` and `smoothed`, `sky_bounce` 0 and 0.3; (4) the hills at noon, the far ground while
  moving, and a jump to a new spawn with the settle watched (Phase 10's `black` count on the
  HUD); (5) the forest at `60,180` and `10,180` under `brick`, `voxel` and `absorb`, before
  and after the queue drains; (6) the day from 15:00 to sunset in the forest; (7) the
  penumbra `disc` against `approach` in the forest once settled, the light pass's ms read
  aloud; (8) a dug skylight into the chamber. Each run's screenshot and the owner's words go
  to `live/window86b/verdicts.md` as before.
- **Files touched:** `docs/plans/logs/0086.md`.
- **Done when:** the owner has recorded a verdict on the underground's darkness, the tunnel's
  brightness and the ceiling picked, the stripes and the arm picked, the settle after a jump,
  the forest floor's grain and brightness and the leaf arm picked, the penumbra's default, and
  any cost the HUD showed that the owner will not pay; or a no-go that names the run.

### Phase 16: The second window's defects attributed, each against the before binary
- **Owner skill:** dev
- **What (readings, no fix):** four readings on the tip and on the before binaries, one table in
  the log. The before binaries: the window's (`live/base-target`, `a37c908b`: Phases 1 to 8 in)
  and, where the question is whether this plan introduced a defect, a release build of the
  plan's base on `main` (the commit the lane branched from, before Phase 1). The fly-through's
  headless run (`--headless --render 1920x1080 --frames N --seed 1`) is the rig unless a test
  rig is named; the per-frame numbers come from the HUD's counters (`missing`, `fallback`,
  `black`, the bake queue line, the pass times) and a display read-back per frame
  (`Frame::set_composite_capture` or the stillness support's).
  1. **Defect 1, the black region.** The hills at `--time 17:00` at 21.png's pose (eye about
     `-? -30.2 671.8`, chunk `-6 -1 10`; the orchestrator's `live/window86b/verdicts.md` has
     the HUD line), `--column-wait on`, 1800 frames at rest: every 60 frames the three counts,
     the queue's depth, waiting and coarse. At the end, for every chunk within the bake radius
     with no block: which column over its reach `bakeable` finds not final, and which chunk of
     that column the finality waits for and why it is absent (pending upload; within the LOD
     sphere but not mirrored; above or below the world's extent; empty by generation and never
     uploaded), from a reading hook on the queue (a `bake queue:` dump or `QueueView`). And why
     the 496 black cells read black rather than the chain's fallback (their neighbours' blocks).
  2. **Defect 3, the fired lights.** The tunnel at 19.png's pose (`--spawn inside`, noon, eye
     `221 -4.4 493.6`), the exposure pinned at -10.75 EV (`--exposure E`) on every binary, five
     point lights placed where the owner's fired lights stopped (a reading hook or the input
     log; the key `6` fires one along the view), 128 frames: the lit patch's p99 and mean
     frame-to-frame change and the share of its edge pixels whose direct sample flips; the
     light pass's ms before and after the lights; the HUD's `black` and the sky-ray counter per
     frame (does the sky ray return over the lights' rings, and do the rings' bricks carry
     `SKY_STALE`); the same on the base build with the same lights. The verdict names the grain
     as new in this plan (and which term) or Plan 0058's.
  3. **Defect 4, the dig flash.** The hills at noon, the queue drained, a 1 m dig at the view's
     centre at frame 300, every frame from 270 to 420, over the lighting pixels within 64 voxels
     of the dig's box (the hole's fresh faces included: they are where "small lights" show): the
     count of pixels whose display luminance is over 2 times their own value at frame 420 (the
     settled view) and over 2 times the neighbourhood's settled mean (Plan 0049's excess-dots
     statistic, per frame and summed over frames 1 to 60 after the dig), the share of pixels
     changing by over 16 levels a frame, the sky-ray counter and the pair's mean age over the
     ring's pixels, and the frames until the ring's re-bake lands (the stale count). Three arms
     split the candidates: `sky_rays` 0 (no ray: what is left is the re-bake's landing and the
     switch), `sky_rays` 1 with `LightHooks::no_clamp` off and on, and the sun off (the sky
     alone). The same on the window's before binary. The log says which of the three carries
     the excess.
  4. **Defect 6, the clipped treads.** The forest at 22.png's pose (`--spawn forest --trees
     forest --time 15:00`), after the queue drains, 64 frames: the share of lighting pixels at
     255 in the display's largest channel, the sunlit treads' mean level (the floor pixels whose
     key sample is lit), the metered exposure and the ceiling in force, under `--sky-anchor
     none`, `4`, and on the window's before binary.
- **Files touched:** `apps/flythrough/tests/gpu_outdoor.rs`, `tests/support/*` (the
  capture and the hooks), `crates/vxe-render/src/bake/queue/*` (the dump hook),
  `apps/flythrough/src/lights.rs` (the placement hook, if the input log does not serve),
  `docs/plans/logs/0086.md`.
- **Done when:** the table is in the log; per defect one line names the cause (or "not
  reproduced headlessly", with the rig and what it did show) and whether the before binary shows
  it; the dig's excess is split across the three candidates with numbers; Phases 18, 21, 22 and
  23 take their mechanism from this table. Readings only; no bar. Wall time: about 15 minutes
  of release runs on the RTX, outside the gate.

### Phase 17: The window's picks as defaults, and the seal bar re-derived
- **Owner skill:** dev
- **What:** `AutoExposure::sky_anchor_ev` defaults to `Some(4.0)` in `vxe-render` (with the sky
  `None` the ceiling is `max` unchanged, Phase 12 bar 1, so the first consumer sees no change);
  `Lighting::sky_normal` defaults to `Smoothed`; `sky_bounce` stays 0 and a setting;
  `LeafShadow::Voxel` and `Penumbra::Disc` stay; `approach` stays a setting (Phase 7's
  done-when 3: a uniform branch, byte-identical goldens under `disc`; the log restates that
  claim against the tip's goldens and Phase 14's light-pass table, no new reading). The
  fly-through's `--sky-anchor none` and `--sky-normal face` are the A/Bs; the banner names the
  non-default (`sky anchor none`, `sky normal face`) and nothing for the defaults. **The seal
  bar:** `the_cave_mouth_is_lit_from_outside`'s seal clause reads the sealed walls at 64 and at
  256 frames after the seal: under 30 % of unsealed at 64 (24.3 read) and under 2 % at 256 (1.1
  read); the 128-frame value is printed. **The goldens** the defaults move are re-blessed once,
  with the share of each default read separately against the tracked images (anchor alone,
  smoothed alone) before the re-bless.
- **Files touched:** `crates/vxe-render/src/exposure.rs`, `frame/settings.rs`, `settings.rs`
  (the defaults' descriptions), `apps/flythrough/src/cli/outdoor.rs`, `cli/tests/outdoor.rs`,
  `hud/modes.rs`, `apps/flythrough/tests/gpu_outdoor_golden.rs`, the `golden/` images,
  `docs/flythrough.md`, `docs/plans/logs/0086.md`.
- **Done when:**
  1. CPU (under 1 s): `AutoExposure::default().sky_anchor_ev == Some(4.0)`,
     `Lighting::default().sky_normal == SkyNormal::Smoothed`, `sky_bounce` 0; the parse test
     pins the defaults and both A/B flags with their banner words.
  2. Every golden whose scene sets no `Sky` is byte-identical at the tip (the showcase, cave
     and entity goldens: the defaults change nothing without a sky).
  3. The seal bar as re-derived passes once by name (soak; its wall time printed, the 192 extra
     frames on top of Phase 9's run).
  4. The outdoor goldens are re-blessed and the log names each with its difference count per
     default.

### Phase 18: The column wait: off by default, its black region named, the bake caught up when a column turns final
- **Owner skill:** dev
- **Amended 2026-11-18, after Phase 16 (architect).** Phase 16 (`f39e7dd3`) did not reproduce
  defect 1 as a wait defect: at 21.png's pose `black` read 1338 from frame 300 on with the wait
  on and off alike, every column turned final, `finality_blockers` was empty for every unbaked
  chunk, and the hit chunks with no block are the horizon ring 7 to 12 chunks out whose reach
  crosses the full radius, which `ReachMode::Full` never bakes while the camera is still (Plan
  0067 Phase 5 built `Partial` for exactly that ring; ADR-0046's Outcome of 2026-11-12 kept
  `full` with `chain` on the owner's "not much difference"). The owner's region (at the view's
  centre, straight edges, stays with the wait on, absent with it off) has no named cause, and
  the rig's dark blob at the centre lies on a chunk that holds a block. So this phase **builds
  the setting and the catch-up as written** (bars 3 to 5 stand: the owner picked off, and the
  pre-roll's double is a measured fact whatever the region's cause), **re-scopes bars 1 and 2**
  (bar 1 is a property test on a constructed never-deliverable column, bar 2 reads the wait's
  black against the wait-off's and builds the dump the third window needs to name the case), and
  **adds bar 6**: the `Full` edge ring read under `partial`, a permanent-black source in view
  regardless of the wait. Rejected: naming the edge ring as 21.png's cause (its black is the
  same under both wait modes, and it sits on the horizon band, not at the centre); another
  window without a dump (21.png hid the eye's x and the HUD names no chunk for a black cell, so
  the rig took the chunk's middle and missed, and a third window would leave the same gap);
  holding the setting until the cause is named (the owner's pick stands on the start time).
- **Amended 2026-11-18, after Phase 18's readings (architect).** Bars 2 and 6 as re-scoped
  above were read at `76eae309` and both fail as written (`docs/plans/logs/0086.md`, Phase
  18). The readings say the bars measured the wrong thing, not that the queue is slow or
  wrong: **no mechanism is added, no default changes, Phase 24's run 1 gains no `partial`
  arm**, and the reach's question moves to Phase 20's reading. Bar 2's second clause and bar
  6 are re-stated below. Grounds, three. (i) **The rig's and the dump's "reach within the full
  radius" is the census estimate**, `d + lr <= r_full` (`tests/support/streamed.rs`'s
  `unbaked_inner`, `run/dump_black.rs`'s `crosses`), which `docs/backlog.md` ("The census's
  reach-bound estimate undercounts", 2026-11-12) already names as wrong: the queue's own test
  (`bakeable_reach`: any offset of `reach_offsets(lr)` past the full sphere) has a reach five
  chunks long on an axis at `lr` 4, so a chunk 10.3 to 12 chunks out is crossing by the queue
  and "inner" by the rig. The dump's 15 no-block chunks within 12 chunks (`live/p18-full.log`)
  are 14 at 10.8 to 12.4 chunks, tracks `blocked`, and one faceless at 6.2 (1 px): by the
  queue's test the residue is zero, and the 4492 "unbaked inner" are reach-crossing chunks
  plus chunks never listed and so never known faceless, which `partial` lists and bakes and
  `full` leaves by ADR-0046's rule and its Outcome of 2026-11-12. Not a throughput limit: under
  `full` the queue bakes 87 blocks in frames 300 to 1800 because nothing it may bake is left,
  not because its slice is spent (`partial` drains the same band in 900 frames on the same
  slice). (ii) **The counted pixels are not black on screen.** `black` is the temporal pass's
  counters word 4, the cube-face hits with no record that the *emissive* chain left black; the
  sky at such a pixel is the long-history ray's (`shade.wgsl` casts the sky ray where neither
  `sky_baked` nor `sky_chained` holds; ADR-0070, "the ray where the bake is stale or absent"),
  and on the hills no emitter reaches the ring, so a block there would hold zero emissive light
  too. `live/p18-full.png` and `p18-partial.png` show the same lit hills, and the mask puts the
  counted pixels on the horizon ridge, which is lit. The band is the ray region at rest, not a
  black band: Phase 20's question (where the ray region lies at rest) and ADR-0076 rule 2's
  recorded breach, not a defect of this phase. (iii) Plan 0067's 1 % still-black line was read
  in a cave, where the counter and the screen agree; under a sky the counter measures an
  emissive gap the hills do not have, so it is the wrong figure for bar 6. Rejected: baking
  the LOD ring or a wider sky bake radius (the 234 chunks beyond 16, 1150 of the 1338 px, are
  LOD-only, which no reach mode bakes, ADR-0053; the far field is Plan 0033's); `Partial` as
  the default on this reading (the Outcome of 2026-11-12 stands until a window reads a
  difference, and the two PNGs show none); the sky chain through the parents (Phase 10's
  unbuilt item) for the LOD ring (it would replace a ray that is clean at a hit rate near 1
  with a chunk mean, and nothing read asks for it); another 1800-frame run against the 1 %
  line. For Plan 0090: its `--dump-pops` builds on `--dump-black`, and must attribute such a
  pixel to the sky ray, not to "black".
- **What:** ADR-0070's amendment of this date. **The wait is a setting**, `lighting.column_wait`
  (off by default; `--column-wait on|off` keeps its words; the banner names `column wait on`):
  on, `bakeable` requires every column over the reach's footprint final, as Phase 9 built it;
  off, a chunk bakes when its reach is resident and no chunk of its columns is pending upload
  (today's `off`). **Catching up:** a block baked while a column over its reach was not final
  carries a provisional mark (a bit of the record's spare word, or a per-chunk flag in the
  queue); when every column over its footprint is final the chunk is re-queued once, behind the
  first bakes (the light appears first everywhere, then corrects), and the mark clears when the
  re-bake lands; the queue keeps `lod_radius` in both modes to know finality. **A column's
  finality is reachable** (the property, whatever 21.png's cause): a chunk the streamer will
  never deliver (above or below the world's extent, outside the residency the streamer holds,
  empty by generation and never uploaded) counts as arrived; `column_final` and
  `reach_columns_final` say so in one place (`held_by_streamer` is that guard as built; bar 1
  pins it). **The black-cell dump** (the reading lever for the third window): a fly-through flag
  `--dump-black` (off by default; the banner names `dump black`) prints to stdout, once a second
  while the HUD's `black` count is over 0, the black lighting pixels grouped by the chunk they
  hit: coordinate, pixel count, distance from the eye in chunks, whether the chunk holds a
  block, its queue track, `finality_blockers`, and whether its reach crosses the full radius
  (what Phase 16's `Hold` report prints in process, from a read-back of the lighting grid's hit
  chunks for the black pixels; the mechanism is `dev`'s, the read-back runs only under the
  flag and at most once a second). The HUD's eye line prints the three coordinates in full
  (21.png's x was cut off).
- **Files touched:** `crates/vxe-render/src/bake/queue/*` (`bakeable`, the provisional mark, the
  re-queue), `gpu_world/columns.rs` (`column_final`), `bake/record.rs` (the mark's bit, if a
  record bit), `frame/settings.rs`, `settings.rs`, `apps/flythrough/src/cli/outdoor.rs`,
  `run/queue.rs`, `run/loading.rs`, `run/hud_feed.rs` (the dump, the eye line), `hud/modes.rs`,
  `crates/vxe-render/tests/gpu_sun.rs`, `apps/flythrough/tests/gpu_outdoor.rs`,
  `tests/support/streamed.rs`, `docs/flythrough.md`, `docs/plans/logs/0086.md`; for the
  second note's `fix` commit: `bake/summary.rs` or `bake/queue/mod.rs` (the exact reach test
  made public), `run/dump_black.rs`, `docs/backlog.md` (the estimate's entry leaves).
- **Done when:**
  1. **No chunk waits forever.** `every_chunk_in_reach_bakes_under_the_wait` (a unit test of
     the queue on a constructed case, under 1 s): a column whose top chunk lies past the
     residency the streamer holds (outside `lod_radius` about the eye) or above the world's
     extent; with the wait on, the chunk under it is `bakeable` once the deliverable chunks are
     resident; the Phase 9 unit test (`a_chunk_absent_high_in_a_column_holds_the_bake`) still
     holds for a chunk that is pending. If the guard as built passes it unchanged, the log says
     so: the test then pins the property.
  2. **The wait adds no black, and the black cells are named.**
     `reading::the_hills_at_dusk_under_each_wait` (Phase 16's `the_hills_at_dusk_held` rig at
     21.png's pose, wait on and off, 1800 frames; a release-run reading, outside the gate):
     from frame 300 on the wait-on `black` count is within 1 % of the wait-off's (1338 read
     for both at Phase 16), every chunk within the full radius that **the queue's own reach
     test** (`bakeable_reach`'s: no offset of `reach_offsets(lr)` past the full sphere) calls
     bakeable holds a block, or is faceless, by the end under both (re-stated 2026-11-18, the
     second note: the rig's `unbaked_inner` and the dump's `crosses` take that test, made
     public from `vxe-render`, and the backlog's estimate entry leaves; read with the estimate
     at `76eae309` the clause holds by the dump, 14 no-block chunks at 10.8 to 12.4 chunks
     crossing by the exact test and one faceless; the re-read with the exact test goes to the
     log as a `fix` commit of this phase), and the frame each mode reaches it and the
     pre-roll's wall time are printed against Phase 9's table (on about 21 s, off expected
     near the finality-off build's 10.3 s). The dump, run on the same rig through the
     fly-through's path (a headless run with `--dump-black`, or the support's call of the same
     function), names the same chunks as the `Hold` report: the horizon ring, no block, reach
     crossing the full radius. A `black` of 0 is not this bar's (the `Full` ring is counted
     under both modes; bar 6 reads it).
  3. **The end state is the wait's.** `a_provisional_bake_is_caught_up` in `gpu_sun.rs` (the
     3 x 3 x 3 stepped terrain of Phase 9 bar 1 streamed so a high chunk arrives last, the wait
     off; about 6 s): the floor chunk bakes before the high chunk arrives (its block
     provisional), is re-queued once when the column turns final, and its record after the
     re-bake is bit-identical to the same world baked with the wait on. The provisional block's
     `+z` visibility before the re-bake is printed (the leak the catch-up closes).
  4. **The underground stays dark with the wait off.** `the_underground_eye_streamed` (soak,
     by name) at `column_wait` off: the direct part under 0.05 by frame 60 and under 0.02 at
     240, as Phase 10 bar 1; the frames 1 to 30 printed (a brief provisional leak is allowed
     there).
  5. **Cost reading:** the re-bakes' count and their share of the queue's jobs through the
     pre-roll on the hills and in the forest (RTX), against the wait-on run; the setting shows
     its mode.
  6. **The `Full` edge ring, read under `partial`** (added 2026-11-18; a reading, no default
     change; re-stated the same day by the second note, the 1 % line withdrawn): bar 2's rig
     with the wait off under `--reach partial` against `full`: `black`, the hit chunks with no
     block and the share of lighting pixels whose sky term is the ray's (the sky-ray cast
     counter, or `vis_out`'s a) every 60 frames to 1800, and the display PNG of each. Read at
     `76eae309`: `black` 967 against 1338, the PNGs alike to the eye, the counted pixels lit by
     the ray on the horizon ridge, `partial` draining the band at 10 to 16 chunks by frame 1200
     and the 234 LOD-ring chunks beyond 16 (1150 px) staying under both. The clause met: no
     black band on screen under either mode. Phase 24's run 1 gains no `partial` arm; whether
     `partial` earns one is Phase 20's reading (its fourth arm), since what it moves is the
     ray region's extent, the seam's question, and whether `Partial` becomes the default under
     a sky is the owner's at that window (a new Outcome on ADR-0046, the architect's, not this
     phase's).

### Phase 19: The key light on the pair's `g` (B (a)), and the grain after a turn
- **Owner skill:** dev
- **What:** the Phase 14 owner question's option (a): the key light's visibility (the shadow
  ray's 0 or 1; `key_light_at` stays exact) runs a mean in the pair's `g` on the direct part's
  history age (`moments_out.z`, capped at `sky_history`), everywhere the key light is sampled,
  reprojected with the direct part's taps and reset where the direct part resets (an edit box,
  no accepted history, `LIGHTING_SKY_CHANGED`); the direct target receives `key_light_at(p, n)
  x g`; the sky's clamp and band stay the sky's; no new target. Where the direct part's
  history is young (a disoccluded pixel after a turn) the mean restarts at its first sample,
  which the a-trous is meant to hide; if the turn reading below stays over its bar, the one
  more lever this phase may take is the a-trous's variance at a young age (SVGF's spatial
  variance estimate where the age is under 4, ADR-0018), and no other.
- **Files touched:** `crates/vxe-render/src/shaders/temporal.wgsl`, `shade.wgsl`,
  `sky_light.wgsl`, `atrous.wgsl` (only if the lever is taken), `crates/vxe-render/tests/gpu_sun.rs`,
  `gpu_sun/reading.rs`, `apps/flythrough/tests/gpu_outdoor.rs`, `docs/plans/logs/0086.md`.
- **Done when:**
  1. **The penumbra holds still.** `the_penumbra_holds_still_on_the_long_history` in
     `gpu_sun.rs` (Phase 6 bar 2 as written: the slab rig at 3 degrees, FSR 2 at 1.5; about
     10 s, soak if over): after 64 frames the penumbra band's p99 frame-to-frame change is at
     or under the lamp's on the same floor.
  2. **A drift lags under half a voxel, and a jump resets.** Phase 6's bars 3 and 4 as written
     there (the sun drifting at the testbed's day for 64 frames moves the shadow's centroid by
     the geometric amount within 0.5 voxel; a 10-degree jump reaches 90 % of the new lit value
     11 frames later; `a_moving_shadow_edge_resets_the_history` holds for the key light's
     shadow).
  3. **The forest floor at rest.** `reading::the_forest_floor_at_a_low_sun` (Phase 14 bar 2:
     the forest at `60,180` and `10,180`, voxel leaves, 128 frames at 320 x 180, soak): the
     floor's p99 and mean frame-to-frame change before and after this phase, against the lamp's
     floor; the floor's pixels split from the canopy's (by material), so a leaf voxel's own
     jitter is read apart from the floor's light. Bar at `60,180`: the floor's p99 at or under
     the lamp-lit forest floor's. At `10,180` a reading; if the floor's pixels alone are at the
     bar and the canopy's carry the rest, the log says so and the architect takes it (a
     canopy's screen-space stability is the far normal's and the upscaler's question, not this
     plan's).
  4. **After a turn.** `reading::the_forest_after_a_turn` (the same rig: 64 settle frames, a
     90-degree yaw over 10 frames, 64 still frames): the floor's p99 change per frame after the
     turn and the frames until it is within 2 display levels of the at-rest value, before and
     after this phase, and the lamp-lit floor after the same turn. Bar: the frames to settle at
     or under the lamp's plus 16 (a guess: the mean at age 16 steps by 1/17).
  5. `frame_bytes` is unchanged; `sky_rays` 0 and the pinned-exposure goldens of the lamp-lit
     scenes are byte-identical (no sky, no key light: nothing runs).
- **Fix pass (amended 2026-11-18, the flicker check's calls 2 and 3).** Files added:
  `crates/vxe-render/src/frame/mod.rs` (`reset_history` clears `encoded` too, the patch in
  `live/reset-history-fix.patch`), its unit test beside the frame's, `apps/flythrough/tests/
  gpu_outdoor.rs` (`tree_shadow` resets before each read; the drift rig's reading). Bars
  re-stated:
  1. p99 at or under 4 levels (ADR-0076's starting `T_still`), the lamp's p99 and both means
     printed; the reading at `37e03683` (2, mean 0.355) is the first value of record.
  2. The drift clause: one bounded reading (both runs' centroids every 16 frames, the sun
     direction fed; 30 minutes); fixed and read if the rig's fault is small, else dropped with
     the log saying so (Plan 0090 scene H takes the property). The jump clauses stand.
  3. At `60,180`: a reading under `T_still` (3.00 read), the lamp's 2.11 printed beside it;
     Phase 25 pins the window's value.
  6. **`reset_history` resets.** `a_reset_between_steps_restarts_the_history` (a unit or
     `gpu_sun.rs` test, about 1 s): after `Frame::reset_history` between two `step` calls, the
     next frame's history age reads 0 and `restarts` counts it; the gate test
     `a_tree_shadow_travels_with_the_sun_at_320x180` passes as written (ratio 1.732 within
     10 %, each centroid within 3 voxels).

### Phase 20: The seam between the baked sky term and the ray's
- **Owner skill:** dev
- **What:** first the reading at 19.png's pose (`--spawn inside`, noon, eye `221 -4.4 493.6`,
  anchor 4, smoothed): per lighting pixel the sky term's source (baked, chained, the pair) and
  the pair's age and hit rate, every 60 frames for 600: does the ray region shrink as the queue
  drains (the wait: Phase 18's), stay (the reach), or follow the stale bit? **Amended
  2026-11-18 (after Phase 18):** the reading runs a fourth arm, `--reach partial`, beside
  those three (Phase 18 bar 6: the horizon's counted pixels are the ray's, and `partial`
  drains the band at 10 to 16 chunks that `full` never bakes at rest); the ray region's share
  of the lighting pixels under each arm goes in the table. **The reading runs on a binary that
  holds `1e16a8fd` (Phase 18 bar 5's fix) and no older:** before it no pre-rolled block held
  sky words, so the shade pass cast the sky ray over every pre-rolled face until the queue
  re-baked it, a ray region at rest of its own; a reading on an older binary attributes the
  pre-roll's gap, not the seam, and the log names the binary's commit beside the table. If `partial` removes the ray region
  at 19.png's pose at rest where (i) to (iii) do not, Phase 24's run 2 gains the `--reach
  partial` arm, and whether `Partial` becomes the default under a sky is the owner's at that
  window (a new Outcome on ADR-0046, the architect's); else it stays a setting and the log
  says so. Then, from this list and no further: (i) the pair's term takes the same normal arm and bounce as the baked
  read (Phase 13's deviation undone: `sky_baked_term`'s blend applied to the pair's visibility,
  so the two estimate one quantity by one normal); (ii) the switch fades over the store's fade
  (Phase 5's "fades over the store's fade", built as none); (iii) a spatial filter over the
  pair's long mean on the lighting grid where the age is at the cap and the mean is under
  `1 / FIREFLY_K` (one 5 x 5 a-trous iteration on the visibility, edge-stopped by depth and
  normal: a sky visibility varies slowly along a wall, so the filter is right where the ray is
  noisiest). If the reading says the ray region lies beyond the bake radius at rest, (iii) is
  what carries it, and the log says so; a wider bake radius for the sky channel is not this
  plan's (ADR-0046's reach).
- **Files touched:** `temporal.wgsl`, `shade.wgsl`, a new `pair_filter.wgsl` and its pass in
  `frame/encode.rs`, `frame/pipelines.rs` (if (iii)), `crates/vxe-render/tests/gpu_sun.rs`,
  `apps/flythrough/tests/gpu_outdoor.rs`, `docs/plans/logs/0086.md`.
- **Re-stated 2026-11-18 (the flicker check's call 4):** the reading at `44e24090` found no
  ray-lit pixel at rest under the default, so bars 1 and 2 as written have nothing to measure
  and are replaced by bar 1a below, taken after Phases 21 and 21a land. (i) is built only if
  bar 1a's relative step is over 10 %, (ii) only if its band share is over 1 %, (iii) not at
  all (no ray region at rest to filter). Files: `apps/flythrough/tests/gpu_outdoor/defects.rs`
  (the rig is `the_tunnel_ray_region`'s), the shaders only if (i) or (ii).
- **Done when:**
  1. ~~The two paths agree~~ and 2. ~~the ray side is as still as the lamp~~: replaced by
     **1a. The ray region lands within the fade.** `defects::reading::the_ray_region_lands`
     (19.png's pose streamed at residency 16, 24, the wait off, `full`, 320 x 180, 300 frames
     on the RTX; a release-run reading outside the gate; Phase 25 writes it as a soak test if
     its lavapipe wall allows, else it stays a reading named in the log): per lighting pixel
     each frame the sky term's source (the pair, the chain, the store) and the display value;
     for every pixel whose source switches from the pair to the store between frames `f` and
     `f + 1`, `old` its display value at `f` and `new` at `f + 16`: the share of such pixels
     with any frame in `f..f + 16` outside `[min(old, new) - 4, max(old, new) + 4]` levels
     (ADR-0076 rule 3's form, `T_fade` 4 the starting guess), the mean and p99 of `|old -
     new|` in levels (the seam's size: how far the ray's mean sat from the baked value when the
     bake landed), the mean `|old - new| / new` (bar 1's 10 % re-stated: over it, (i) is built
     and the reading re-run), and the frame-to-frame p99 over the ray-lit pixels at frames 60
     and 120 (bar 2 on the transient, the lamp's 2.11 printed beside it). The band share's
     line is 1 % (over it, (ii)); the count of switching pixels is printed and is at least
     1000 (the reading's 14 131 at frame 0 drain by frame 300), else the rig is at fault.
  3. **Nothing moves where the store is fresh.** The hills' noon golden is byte-identical
     (every pixel baked); the tunnel goldens that hold ray-lit pixels are re-blessed as a
     disclosed change with counts.
  4. **Cost:** the temporal pass and (if built) the filter pass on both adapters at the tunnel
     pose (the budgets table's guesses).

### Phase 21: The flash around a dig
- **Owner skill:** dev
- **What:** the mechanism Phase 16's split named, fixed from this list and no further, (c)
  first. **(c), decided 2026-11-18 (the amendment above): the pair's sky term leaves the direct
  part and joins at the filter.** In `temporal.wgsl` the sky part of `t` (`pair.x *
  sky_open_by_normal(rec.n) * exposure_now()` where `sky_cast`) is no longer added to
  `acc_direct`; its luminance stays in the moments' blend (`l2`), so the filter's variance
  counts it; the key light's part of `t` stays as Phase 19 built it. `sky_open_by_normal` and
  `sky_open_irradiance` move from `temporal.wgsl` to a shader both passes include (`sky.wgsl`,
  beside `sky_on`). In `atrous.wgsl` the `variance` step binds this frame's `vis`
  (`texture_2d<f32>`, the shade pass's `rgba8unorm`), the pair written this frame
  (`texture_2d<f32>`), `sky_e` (`array<vec4<f32>, 6>`, as the temporal pass binds it) and the
  sky uniform, and adds the same term to the filtered quantity `c` where `vis.a > 0.5` and
  `sky_on()`; `dark_store` keeps its test. In `composite.wgsl` the `LIGHTING_FILTER_OFF`
  branch adds the same term after the direct accumulation, with the same bindings. No new
  target, no storage texture: the temporal pass's four storage textures are
  `Limits::default()`'s four. The debug read-backs that assert the sky term now read the
  filter's output or the pair, never `direct_accumulated`. **(a), built after (c), not
  conditional (amended 2026-11-18, the flicker check's call 1):** a pixel that resets where
  the sky ray is cast (`sky_cast` with no accepted history, an edit box, `LIGHTING_SKY_CHANGED`)
  seeds the pair at age `SEED_AGE` from the best world-space estimate, in this order: its
  face's record visibility (a stale ring brick's: the touched brick keeps its words under the
  stale bit), else the chain's (`store_sky_fallback`), else the open visibility 1; the first
  samples then blend at `1 / (SEED_AGE + 1)` and the firefly clamp is live from the first
  frame. `SEED_AGE` is 8 (a guess; the dev may take it anywhere in 4 to 16 to meet bars 3 and
  4, and the log names the value). The seed is a value, not a second history: the pair stays
  the term's one screen-side history. This is the castle's sparkle, the dig's dots and the
  forest's young pixels after a turn in one mechanism; Phase 21a brings the stamped chunk's
  bake. (b) as before: the ring's coarse re-bake does not write the sky words (`VIS_BAKED` from
  the full pass only, Phase 10's unbuilt item), only if bar 2 fails on a brick-sized patch.
  **Phase 16 (2026-11-18) points at (c):** the stored/direct switch at the stale bit carries
  the area (with the sky alone, half the region moves over 16 levels for two frames; the ray's
  young samples about 26 dots of 50, the key light's reset pixels about 24, the clamp 45
  against 50), so (c) is built first and bars 1 and 2 read, then (a) and bars 3, 4 and 4a.
- **Files touched:** `crates/vxe-render/src/shaders/temporal.wgsl`, `atrous.wgsl`,
  `composite.wgsl`, `sky.wgsl`, `crates/vxe-render/src/frame/pipelines.rs` and
  `frame/encode.rs` (the variance's and the composite's bind group layouts and groups),
  `bake/job.rs`, `bake/queue/*` (only if (a) or (b)), `crates/vxe-render/tests/gpu_sun.rs`,
  `apps/flythrough/tests/gpu_outdoor.rs`, `docs/plans/logs/0086.md`.
- **Done when:**
  1. **The direct part holds no sky term.** `the_direct_part_holds_no_sky_term` in
     `gpu_sun.rs` (the open floor under `Sky::uniform(1.0)` with no store block, so every
     face casts the ray; the key light off by `LIGHTING_HOOK_KEY_OFF`; a lamp lit far enough
     that a chosen floor pixel receives none of it; 16 frames at 160 x 90; about 3 s): at that
     pixel `direct_accumulated` reads 0 in rgb at every frame, and the filter's output reads
     the pair's term, `pair.x * E_sky(n) * exposure`, within 5 %. Before this phase
     `direct_accumulated` there reads the term's blend (near the full term by frame 16); the
     value is printed.
  2. **The switch neither dips nor flashes.** `the_sky_term_switches_without_a_dip` in
     `gpu_sun.rs` (bar 3's rig with the key light off, so the sky alone lights the floor, as
     Phase 16's cleanest arm; about 8 s): over the ring's pixels (faces whose record carries
     `SKY_STALE` at any frame after the dig, outside the edit box dilated by one voxel), with
     `before` the display value at frame 63 and `after` the value at frame 124, every frame's
     value from 63 to 124 lies in `[min(before, after) - 2, max(before, after) + 2]` display
     levels of 255 (ADR-0076 rule 3's form; 2 levels stands in for `T_fade` until Plan 0090
     calibrates it), and the share of ring pixels that leave the band at any frame is 0. The
     same statistic before this phase is read and printed (the expectation: the first stale
     frame dips by four fifths of the sky term at a mature pixel, and the frame after the
     re-bake lands overshoots by the direct history's share of it). The ring is non-empty: at
     least one frame casts the ray over at least 1 % of the pixels within 64 voxels of the
     box (Phase 16 read 7357 and 4765 rays on two frames), else the test fails as a rig fault.
  3. **A dig leaves no dots.** `a_dig_leaves_no_dots` in `gpu_sun.rs` (the open floor of 3 x 3
     chunks baked whole under `Sky::uniform(1.0)`, a zenith sun, the live queue at the
     fly-through's settings so the re-bake takes frames, a 1 m dig at frame 64, 60 frames
     read; about 8 s): in every frame after the dig, the count of lighting pixels within 64
     voxels of the box whose display luminance is over 2 times their value at frame 124 and
     over 2 times the neighbourhood's settled mean is under 1 % of those pixels, and 0 from
     frame 16 on; the same before this phase is read and printed (the expectation: tens of
     percent in the first frames on the hole's faces).
  4. **The hole takes its light.** The same test: 60 frames after the dig the hole's floor
     reads its baked value within 10 % (Phase 5 bar 4's clause), and no frame between has the
     share of pixels changing over 16 levels above 5 %; the dots on the hole's own faces (the
     pixels inside the dilated box) are counted apart and printed, and their share is under
     1 % on every frame after frame 16 (the bar (a) meets; before (a) it is printed).
  4a. **A reset pixel starts from its record.** `a_reset_pixel_starts_from_its_record` in
     `gpu_sun.rs` (bar 3's rig; about 3 s): at the first frame after the dig, over the
     pixels inside the dilated box whose face holds a stale record, the pair's `x` is within
     `1 / (SEED_AGE + 1)` of that record's visibility and its age reads `SEED_AGE`; on the open
     floor under `Sky::uniform(1.0)` with no store block (bar 1's rig) a pixel's first frame
     reads `x` 1 at age `SEED_AGE`; the mean of the pair's `x` over the box's pixels at frames
     1, 8, 16 and 60 is printed (monotonic toward the settled value, no overshoot by more than
     one sample's share).
  5. **Nothing moves where no ray is cast.** The hills' noon golden and the lamp-lit scenes'
     pinned-exposure goldens are byte-identical (`vis.a` is 0 on every pixel there, so the
     filter adds nothing and the direct part lost nothing); a golden that holds ray-lit pixels
     at rest is re-blessed only if it moves, as a disclosed change with counts (the blend's
     residual at rest is under one level). `frame_bytes` is unchanged.
  6. **Cost:** the variance pass and the temporal pass on both adapters at the tunnel pose,
     with Phase 20 bar 4's runs (the amendment's guesses: +0.01 ms RTX, +0.1 ms RENOIR on the
     variance pass, the same back from the temporal pass).
  7. The fly-through's dig at Phase 16's rig re-read with bar 3's statistic, tip against the
     window's before binary, in the log.

### Phase 21a: A chunk an edit fills from air is a dig
- **Owner skill:** dev
- **What (added 2026-11-18, the flicker check's call 1 (ii)):** in `take_boxes`, a chunk the
  edit's box touches (its bricks within the box dilated by one) that holds no block is no
  longer skipped: it enters `digs` as a whole-chunk dig (every brick of its listing a slot, no
  kept bricks; a `DigChunk` flag or an empty kept set says so), `near` set, and leaves
  `faceless` as a touched chunk does. `open_digs` opens it in its order (near first, then by
  distance from the eye), lists it on the opening thread as it lists a dig with no pre-listing
  today, drops it as faceless if the listing has no face (a ring's air chunk), and lands it as
  a dig job; no provisional block at the edit (the amendment's rejected list says why), no
  change to the dig gate (`open_step`'s two slices) or to the dig budget's `edit_ms`. Ring
  chunks the box does not touch are unchanged. Until the job lands the chunk's faces are
  ray-lit under Phase 21 (a)'s seed (the chain's value next to a baked neighbour, the open
  prior elsewhere), and the landing is the step the amendment accepts.
- **Files touched:** `crates/vxe-render/src/bake/queue/digs.rs`, `bake/queue/mod.rs`
  (`DigChunk`), `crates/vxe-render/tests/gpu_sun.rs`,
  `apps/flythrough/tests/gpu_outdoor/flicker.rs`, `docs/plans/logs/0086.md`.
- **Done when:**
  1. **A stamp into air is a dig.** `a_stamp_into_air_is_a_dig` in `gpu_sun.rs` (one floor
     chunk baked with a block, a solid box stamped into the air chunk above it, the live queue
     stepped; about 3 s): after the edit's step the air chunk is in the queue's digs (the
     `tracks` report names it `digs` or `dig_open`), it opens before a pending candidate
     listed earlier (the floor's neighbour chunk, added to the pool the same step), and its
     block lands with `VIS_BAKED` and no `SKY_STALE` on its lit bricks; a box stamped into an
     air chunk that stays faceless (an empty prefab) lands nothing and the chunk is in
     `faceless`.
  2. **The keep fills from the eye out.** `flicker::reading::flicker_castle` re-read on the
     tip (the paced rig; RTX, release): `missing` at frames 100, 200 and 300 against the
     before's 49 282 / 17 653 / 0, with the bar `missing` at frame 100 under half the before's
     (the keep's chunks, the nearest, open ahead of the ring's far side) and 0 by frame 300;
     the fresh strip's over-8 mean under 0.5 % (a guess; 2.274 % before, with Phase 21 (a)
     carrying most of the cut) and its young share printed against 66.5 %; the settled strip
     unchanged within noise.
  3. **A dig's ring is unchanged.** `a_dig_leaves_no_dots` and Phase 21's bars read the same
     values (the dig's touched chunks all hold blocks, so nothing new enters `digs`); the
     hills' noon golden is byte-identical.
  4. **Cost:** the edit's step (`take_boxes`) in ms on the RTX for the castle's 17 boxes,
     before and after; the frames until the queue drains against the before's 2315 paced
     frames; the queue's `digs` and `digs_baked` counters printed.

### Phase 22: The fired lights' grain under the anchor
- **Owner skill:** dev
- **What:** if Phase 16 names the first candidate: a dropped or fired light's re-bake rings
  (ADR-0046's rings on a light change) leave the sky words and the `SKY_STALE` bit alone (the
  sky visibility is geometry; only a voxel edit stales it), so no sky ray returns over a lit
  wall; the light pass's rise is then the lights' own shadow rays. If Phase 16 names Plan
  0058's grain made visible by the anchor's dark tunnel, this phase writes no mechanism: the
  reading goes to `docs/backlog.md` as its own entry under Engine mechanisms (a Plan 0058
  follow-up: the list lights' visibility on the pair's `g` is the natural upgrade), and the
  window judges it under the default anchor 4. **Phase 16 (2026-11-18) names the second:** the
  grain is the same with no sky, no sky ray returns over the lights, and the light pass's rise
  is the five lights' shadow rays (Plan 0058's fired light), so this phase writes no mechanism:
  the backlog entry and bar 2 only.
- **Files touched:** `bake/queue/*` (`take_boxes` or the light-change path), `bake/store.rs`,
  `crates/vxe-render/tests/gpu_sun.rs`, `docs/backlog.md` (if filed), `docs/plans/logs/0086.md`.
- **Done when:**
  1. If built: `a_dropped_light_does_not_stale_the_sky` in `gpu_sun.rs` (the open floor baked
     whole, a point light dropped at frame 64; about 6 s): the sky-ray counter reads 0 over the
     frames after, every record's `SKY_STALE` stays clear, and the floor's stored sky part is
     unchanged within 1 %; the light pass's ms before and after within the light's own share
     (`nfr.md` section 2: 0.175 ms a light on the RTX).
  2. Either way: the tunnel at 19.png's pose with five lights under anchor 4 and 0, the lit
     patch's p99 change and edge flips against Phase 16's table, in the log.

### Phase 23: The sunlit treads' clip
- **Owner skill:** dev
- **What:** from Phase 16's reading. If the clipped share under anchor 4 is under the bar, this
  phase writes the bar and nothing else. Else, from this list: the auto exposure's metering
  (ADR-0033) weighs the lit floor's share so the key sits between the shadowed and the sunlit
  floor, or the tone curve's shoulder compresses what lies over the key (if the curve has none
  above the key, adding one is a look change on every scene and an owner question before it
  is built: the architect's, not `dev`'s). **Phase 16 (2026-11-18): anchor 4 removes the clip**
  (0.000 % at 255 against 1.323 % under `none`; the treads' mean level 166.5 against 243.6), so
  this phase writes the bar only, pinned from that table, and no tone-curve question is owed.
- **Files touched:** `crates/vxe-render/src/exposure.rs`, `shaders/exposure.wgsl` or
  `composite.wgsl` (only if a mechanism), `apps/flythrough/tests/gpu_outdoor.rs`,
  `docs/plans/logs/0086.md`.
- **Done when:** `the_sunlit_forest_floor_does_not_clip` in `gpu_outdoor.rs` (the forest at
  15:00 at the defaults, the queue drained, 64 frames at 320 x 180; about 10 s, soak if over):
  under 0.5 % of the lighting pixels read 255 in the display's largest channel and the sunlit
  treads' mean level is under 240 (both guesses, pinned from Phase 16's table before the test is
  written); the share under `--sky-anchor none` is printed beside it.

### Phase 24: The owner's third window
- **Owner skill:** human
- **What:** a runner (session86c) on one build at the tip of Phase 23, the window's before
  binary and Phase 15's binary (`live/bin86b`) beside it, the fly-through's stdout visible and
  a ping when ready, each run re-checking one defect or one pick, in this order: (1) the hills
  at 17:00 at 21.png's pose, `--column-wait on` then `off`, both with `--dump-black` (Phase
  18): no black region, the start time read from the HUD; if a black region shows, the owner
  holds the pose a few seconds and the dump's lines and the HUD's eye line go to the verdicts
  file beside the still, so the case is named this time (no `--reach partial` arm here:
  Phase 18's second note); (2) the tunnel at 19.png's pose at the defaults: the seam between
  the near walls and the deeper tunnel, then deeper to the far end, with a `--reach partial`
  arm only if Phase 20's reading says so; (3) the same with five lights
  fired (key `6`) under anchor 4, then `--sky-anchor 0`: the walls where the lights land, the
  light pass's ms read aloud; (4) a dig on the hills at noon and one in the tunnel, watched for
  the flash, twice each, against Phase 15's binary once; (4b, added 2026-11-18) a castle
  placed on the hills at noon on the belt's binary (`CASTLE_BIN`, as the runner has it), the
  eye walked into the keep as it fills: the walls' sparkle (gone or not), the exposure's
  motion (one ramp or swings), and the castle's shadow on the hill as it fades in (owner
  question 1 of the flicker amendment: about 1 s, acceptable or not); (5) the forest at `60,180` and
  `10,180` under `voxel`, settled, then a fast turn and the settle after it; `absorb` once at
  60 for comparison, its fps read aloud; (6) the forest at 15:00: the treads, then the day to
  sunset; (7) the defaults' look as one scene each: the tunnel's stairs under `smoothed` against
  `--sky-normal face`, the ceiling under anchor 4 against `none`; (8) a skylight dug into the
  chamber, then sealed. Each run's screenshot and the owner's words go to
  `live/window86c/verdicts.md` as before.
- **Files touched:** `docs/plans/logs/0086.md`.
- **Done when:** the owner has recorded, per defect 1 to 6, fixed / not fixed / worse, and a
  verdict on each default (the anchor, the smoothed normal, the wait off); or a no-go that names
  the run. Each verdict's words are quoted into the readings log.

### Phase 25: The picks as tests
- **Owner skill:** dev
- **What:** the bars Phases 1 to 7, 9 to 14 and 16 to 23 read, written as tests against the
  window's picks; the defaults the window chose; the goldens Plan 0032 blessed that these mechanisms move
  (`outdoor_noon`, `cave_mouth`, `outdoor_forest_sunset`, the sunset and night ones if the
  bake moves them) re-blessed as a disclosed change with the difference counts in the log; a
  mechanism the window dropped stays a setting if it costs nothing when off (owner,
  2026-11-12) and goes otherwise, and the log names it.
- **Files touched:** the `gpu_*` test files and `golden/` images above, `docs/plans/logs/0086.md`.
- **Done when:**
  1. **The restated stop rule as a test.** `the_tunnel_is_as_still_as_the_lamp_lit_floor` in
     `gpu_outdoor.rs` (the tunnel pose at the defaults, 128 frames at 320 x 180 under FSR 2
     at 1.5; about 20 s, so born `#[ignore = "soak"]`, with a 32-frame native variant in the
     gate): the 99th-percentile frame-to-frame change over the rim band and the far end is at
     or under the headlamp's on the same pose with the sky off. This is Plan 0032 Phase 6's
     stop rule on the occluded rig, as a regression test.
  2. Every bar Phases 2 to 7, 9 to 14 and 16 to 23 name that the window kept exists, asserts the picked
     value, and passes in the gpu tier under the lock; each over 10 s on lavapipe is
     `#[ignore = "soak"]` and passes once by name. Phase 9's bars 2 to 5 stay whatever the
     window picks: the sky rule is not a look.
  3. The goldens are re-blessed and the log names each with its difference count and the
     mechanism that moved it.

## Data shapes

```rust
// illustrative: crates/vxe-render/src/bake/record.rs (the store's record, ADR-0046 as built)
pub struct BrickRecord {
    pub emissive: [u32; 6],      // rgb9e5 per direction, as built
    pub material_mask: u32,      // material | direction mask << 16, as built; a SKY_STALE bit
    pub sky_visibility: [u8; 6], // the cosine-weighted fraction of the hemisphere that is sky
    pub spare: [u8; 2],
}
// The visibility pair (two targets, lighting grid, rgba16float):
//   r sky visibility's running mean, g key visibility's (Phase 6), b sky short mean, a age.
// The sky uniform gains six vec4: E_sky for +x, -x, +y, -y, +z, -z (xyz; w spare).
// Frame::set_material_styles(&[u32]) (or Plan 0081's name): bit 0 SHADOW_AS_BRICK.
```

Sizes worked out: the record grows by 8 B (about 29 % of today's); on Plan 0032's hills the
store read 8.8 MB and the forest 15.5 MB, so about +2.5 and +4.5 MB. The pair at the owner's
window (844 x 460 lighting pixels) is `2 x 388 240 x 8 = 6.2 MB`; at 1080p's half tier
(960 x 540) 8.3 MB. The sky uniform grows by 96 B.

## Budgets

| `nfr.md` figure | projection (a guess unless marked read) | how the close measures it |
|---|---|---|
| §2 Lighting, the light pass | at rest inside the bake radius the sky ray is gone: RADV RENOIR about 10.5 to about 5.7 ms on the hills (read: 4.8 ms a ray, Plan 0032 Phase 6), RTX about 1.04 to about 0.8; during a dig the ray returns over the rings' pixels; the canopy flag shortens the key ray in a forest (steps p99 79 to 92 read) | Phase 5's and Phase 2's readings, both adapters, the hills and the forest, at rest and in a dig |
| §2 Lighting, the temporal pass | the store read gains one visibility read and a six-value blend; the pair's read and write | the same runs, the `temporal` pass |
| §2 the streaming line, the bake queue's step | `SKY_BAKE_RAYS` more rays per brick direction while a sky is set: about +6 % of the emissive job's samples at 256 against 4096, but each ray is long | Phase 4's pre-roll and step readings against Plan 0032 Phase 9's `bake queue:` lines |
| §3 World in VRAM (the store) | +8 B a record, about +29 %: about +2.5 MB on the hills, +4.5 in the forest | the store's live bytes at the last frame, Phase 4 |
| §3 Everything else (frame targets) | +6.2 MB at the window, +8.3 at 1080p's half tier; +96 B uniform | `frame_bytes`, Phase 5 |
| §4 Edits, a dig's re-bake | the rings' jobs carry the channel; the owner's lag bar is the ray's, not the bake's | Phase 5 bar 4's frame count; the dig's settle frames as Plan 0059 read them |

Added 2026-11-18 (every figure a guess until the phase reads it; the Floor profile's Lighting
line is 4.0 ms since Plan 0069's close, and the RADV RENOIR is the slow adapter below the
floor's class, not the baseline):

| `nfr.md` figure | projection (a guess) | how the close measures it |
|---|---|---|
| §2 Lighting, the light pass (Phase 9) | one texture load per sky ray that ends without a hit: +0.02 ms RTX, +0.2 ms RENOIR; this plan read the outdoor light pass at 1.06 to 1.53 ms RTX and 12.9 to 15.2 ms RENOIR with the queue not drained | Phase 9 bar 7's runs, both adapters |
| §2 the streaming line, the column map's pass (Phase 9) | the dirty tiles' max per frame while flying: +0.1 ms RTX, +0.5 ms RENOIR on a line that is 1.0 ms and a recorded miss | Phase 9 bar 7, the hills path |
| §2 the bake queue's step and the pre-roll (Phase 9) | the finality wait lengthens the pre-roll by the columns' upper chunks (empty chunks upload without waiting): +0 to 20 % of Phase 4's 7.3 s | Phase 9 bar 7 against Phase 4's table |
| §3 Everything else, the column map (Phase 9) | at most 4.5 MB at 2-voxel cells over the fly-through's footprint, about 1.1 MB at 4-voxel cells | the map's bytes in `frame_bytes` or the mirror's live bytes, Phase 9 |
| §2 Lighting, the temporal pass (Phases 10, 13) | the chain's sky read and the bounce floor's read: +0.02 ms RTX, +0.1 ms RENOIR each; the smoothed normal near, if its gradient is not at hand: +0.05 ms RTX, +0.4 ms RENOIR | Phases 10 and 13's readings, both adapters |
| §2 Lighting, the light pass (Phase 14) | the key light on the pair adds no ray: about 0; the `absorb` arm's translucent walk through leaf bricks: +0.3 ms RTX, +2.5 ms RENOIR in the forest, off unless picked | Phase 14 bar 4 |
| the exposure pass (Phase 12) | a few multiplies: 0 | Phase 12 bar 2's runs |

Added 2026-11-18, Phase 15 window (every figure a guess until its phase reads it; the readings
this plan has: the outdoor light pass 1.1 to 1.8 ms RTX and 10.1 to 15.7 RENOIR at rest, the
temporal pass 0.10 to 0.12 RTX and 0.9 to 1.6 RENOIR, Phases 9 and 13):

| `nfr.md` figure | projection (a guess) | how the close measures it |
|---|---|---|
| §2 Lighting, the light pass (Phase 19) | the key light on the pair adds no ray and one more read and write of a channel the pass already touches: about 0 | Phase 19 bar 3's runs, both adapters |
| §2 Lighting, the temporal pass (Phase 20) | the pair's term through the baked read's blend and the fade: +0.02 ms RTX, +0.1 ms RENOIR; the seed at a reset: 0 | Phase 20 bar 4 |
| §2 Denoise, the variance step (Phase 21 (c)) | two texture loads and six open irradiances per lighting pixel: +0.01 ms RTX, +0.1 ms RENOIR; the temporal pass gives the same back: about 0 net; section 3: 0 B, no storage texture | Phase 21 bar 6, with Phase 20 bar 4's runs |
| §2 Denoise, a pair filter pass (Phase 20 (iii), if built) | one 5 x 5 iteration on one channel of the lighting grid: +0.05 ms RTX, +0.4 ms RENOIR, against the Denoise line's 2.0 ms | Phase 20 bar 4 |
| §2 the bake queue's step and the pre-roll (Phase 18) | the wait off: the pre-roll back near Phase 9's finality-off reading (10.3 against 21.2 s RTX); the catch-up re-bakes at most one more job per chunk that baked provisional, a guess of 20 to 50 % of the pre-roll's jobs on the hills, behind the first bakes | Phase 18 bars 2 and 5 against Phase 9's table |
| §3 World in VRAM (the store) | the provisional mark is a bit of the record's spare word: 0 | Phase 18 |
| the exposure pass (Phases 17, 23) | the anchor's default: 0; a metering or shoulder change, if any: 0 | Phase 23 |

Added 2026-11-18, the flicker check (guesses until read):

| `nfr.md` figure | projection (a guess) | how the close measures it |
|---|---|---|
| §2 Lighting, the temporal pass (Phase 21 (a)) | the seed's record and chain reads at a reset pixel only (a few percent of pixels a frame at rest, a wall's worth at an edit): about 0 at rest, under +0.05 ms RTX on the edit's frames | Phase 21 bar 6 |
| §4 Edits, an edit's step (Phase 21a) | `take_boxes` gains the touched blockless chunks' entries only (their listing moves to `open_digs`' thread, where a dig's already runs): under +1 ms on the edit's frame for the castle's 17 boxes; the queue's dig work grows by the stamped chunks' jobs, which the pool would have baked anyway, moved ahead | Phase 21a bar 4 |

On the slow adapter the outdoor light pass is over section 2's line in every configuration
Plan 0032 read; this plan's Phase 5 is the first reading that can move it under, and `nfr.md`
section 2 records the result at the close.

## Risks & open questions

- **The brick-level leak.** A brick holding a roof's top and a cave floor's `+z` faces
  averages their visibilities (ADR-0046's Alternative E, accepted). The tunnel's 25-voxel
  radius and the chamber's 60 are many bricks wide, so the testbed does not show it at the
  owner's poses; a one-brick roof would. Per-face cells near the camera are the backlog's.
- **The colour is averaged.** A warm horizon through a slit reads as the sky's cosine mean
  about the face. ADR-0070's Alternative F (`L1` visibility) is the upgrade if the window
  reads it as wrong.
- **The clamp's dark bias** where the hit rate is lowest, during a re-bake only: about `K p`
  of the truth for `p` under `1 / K`. Phase 5 prints it.
- **The lift's leak.** A lift of up to `LIFT` voxels can pass a one-voxel wall at a distance.
  Phase 3 bar 2 reads it under a pixel or the lift shrinks.
- **Where the style flag lives.** Plan 0081's table or this plan's word; whichever lands
  second absorbs the other's, and both logs name the merge item.
- **The record's growth** against `STORE_BINDING_BYTES` (256 MiB): the forest's 15.5 MB
  grows to about 20; far from the cap, read anyway.
- **The far field.** Beyond the bake radius and before the queue reaches a chunk, the ray and
  its history are what there is, as today; Plan 0033's far field is the end state.
- **The history's lag** on a moving occluder deep in a penumbra, where the variance reset's
  band is wide. Phase 5 bar 3 reads the open-ground edge; the window reads the walkers.
- **Determinism.** Nothing touches `vxe-world`. The channel is render-side, baked from the
  mirror, bit-identical whatever the slicing (ADR-0046), and no gameplay query reads it.
- **The spike's cost.** Four to eight loads a coarse step is a guess; the spike reads it and
  ends. A no-go leaves `penumbra approach` as a setting only if it costs nothing with `disc`
  (a uniform branch), else it goes.
- **The column map's far-field error** (added 2026-11-18). A ray that ends in open air above
  its column's top counts as sky though a hill beyond the cap may block it; one that ends
  under an overhang open to the side counts as blocked. Both are errors at the cap's distance
  or more, the class the far field (Plan 0033) owns, and the same class the rule as built had
  on open ground. The exact answer, a sky flood from the top of every column through the
  air, is the upgrade if the window shows an arch or a bore longer than the cap read wrong.
- **The finality wait** (added 2026-11-18). A chunk whose reach's columns hold an absent chunk
  high up does not bake until it arrives; the pre-roll may lengthen (Phase 9 reads it). If it
  lengthens by more than a fifth, the fallback is a per-column finality that counts a chunk
  above the window's z-extent as empty, which is what the rule assumes already.
- **The transient after a jump** (added 2026-11-18, question 4). Under (a) the underground
  reads a brief false sky while its columns fill, remembered by the pair until the variance
  reset catches it; Phase 10 bar 1 bounds it.
- **The provisional bake's leak** (added 2026-11-18, Phase 15 window). With the wait off a
  chunk baked before a chunk above it arrives holds a false sky until the catch-up re-bake
  lands; Phase 18 bar 4 bounds it in the streamed rig and bar 3 proves the end state. If the
  catch-up's re-bakes cost more than half the pre-roll's jobs, the fallback is the wait on by
  default for the first pre-roll only.
- **The far end beyond the reach** (added 2026-11-18, Phase 15 window). If Phase 20's reading
  puts the ray region beyond the bake radius at rest, the pair's filter is what carries it, and
  the owner may still see a seam at the radius; a wider sky bake radius is an ADR-0046 change
  for a later plan.

## What this plan does NOT do

- **The far field's fallback for an absent chunk** (the review's B3): Plan 0033.
- **A sunlight bounce**, sky light in the probes, or per-face sky cells (the backlog's).
- **Two radii per body** (A2): the fallback if the penumbra's grain survives this plan.
- **The key ray's length cap and the down-facing skip on the slow adapter** (the review's
  E2): not picked; a profile plan's measured change if the RADV reading after Phase 5 asks.
- **The CPU settle** (E3: the bake queue's step on a world with no emitter, the forest's sync
  p99): `docs/backlog.md`, dated 2026-11-17.
- **The 4-voxel shadow walk everywhere** (D5): the Lego look.
- **Baseline-GPU readings.** The card is not on hand.
- **A sky flood** (added 2026-11-18): the exact connectivity of air to the sky, a volumetric
  flood per edit; ADR-0070's amendment names it as the upgrade past the column map.
- **A tone-curve toe for dark scenes** (question 1 (c)) and **the sunlight bounce** (question
  2 (c)): a look plan of their own if the owner asks.
- **Plan 0036's fall map at slanted directions**: Plan 0036 extends the column map; this plan
  builds the vertical one only.

## Implementation log

> Written by `dev`: one row per phase as its commit lands, then the close block. **The phases
> above are the contract. This section records what happened.** Observations, never conclusions:
> no pass list, no self-assessment. Deviations and unmet done-whens are **always** disclosed.
> Keep it shorter than `## Implementation phases`. Readings, gate counts, attempts, sweeps and
> resume notes go to the plan's readings log, `docs/plans/logs/0086.md` (started from
> `templates/readings-log.md`), and this section links it once it exists.

Readings, gates and attempts: [`logs/0086.md`](logs/0086.md).

| phase | owner | state | commit |
|---|---|---|---|
| 1: The attribution reading | dev | done | 851f7c72 |
| 2: A canopy shadows as its brick | dev | done | 2b5fff92 |
| 3: History across a stair in the far-normal regime | dev | done | 7a3c6d72 |
| 4: The sky visibility baked per brick direction | dev | done | b8a23a9c |
| 5: The rays' own long history, the sky ray only where stale | dev | done | 012bd208 |
| 6: The key light on the long history, after a reading | dev | done (reading only: 3-degree p99 1 at the lamp's 1, not built) | 12fe6373 |
| 7: A spike: a deterministic penumbra from the closest approach | dev | done (spike; cost on the RTX approach arm and the RENOIR not read) | ed47dc5c |
| 8: The owner's windowed check | human | done 2026-11-18, failed on five points (`live/window86a/verdicts.md`; the amendment above) | |
| 9: The sky rule: the column map, sky only above it | dev | done; bar 5 (the seal) fails as written (24.3 %, the pair's history decaying: 1.1 % at +128 frames); the bake's finality doubles the pre-roll (10.3 to 21.2 s, RTX) | d821e3e2 |
| 10: The first seconds after a jump | dev | done; bar 1 met by Phase 9 (no band change); built the chain's sky; bar 3's frames not reached in the rig (its queue bakes one chunk in 600 frames) | ead32aff |
| 11: The window's defaults | dev | done; five outdoor goldens re-blessed (the leaf default moves none; Phases 9 and 10's share read against the base) | 06269523 |
| 12: The tunnel's brightness: the eye's ceiling anchored to the sky | dev | done (exploration); the anchor is an `AutoExposure` field; the forest sunset at `N` 4 reads 1.37 EV under `none` (bar 2's 0.5 EV not met); RENOIR not read | f05cf2af |
| 13: The zebra stripes: the sky term's normal and a bounce floor | dev | done (exploration); the arms apply to the baked read only; the stair rig reads face 1.62, smoothed 1.00; the open floor not in view | ad4f241c |
| 14: The forest floor: attribution, the key light on the pair, the canopy's transmittance | dev | done in part (exploration): `absorb` built; the key light carries the grain at 60 (p99 13 against the sky's 4), neither term at 10; Phase 6's mechanism not built (an owner question: the pair's one age); bar 3's no-pixel-over-50 % not met (49 % of the projection has no leaf); absorb's light pass 4.1 to 4.8 times voxel's | `5616cfbb` |
| 15: The owner's second window | human | done 2026-11-18, passed on the leak and the picks, failed on six defects (`live/window86b/verdicts.md`; the second amendment) | |
| 16: The second window's defects attributed | dev | done (readings, in-process rigs; no before-binary run): defect 1 not reproduced as a wait defect; 3 is Plan 0058's; 4 the stored/direct switch carries the area; 6 removed by anchor 4 | `f39e7dd3` |
| 17: The picks as defaults, the seal bar re-derived | dev | done: anchor 4 and `smoothed` the defaults; five outdoor goldens re-blessed (counts per default in the log); seal 24.347 / 8.689 / 1.100 % passes; the sky-less goldens unmoved by the defaults (three differ from their tracked bytes at the base, not re-blessed) | `512724cf` |
| 18: The column wait: off by default, its black region, the catch-up | dev | done in part: the setting, the catch-up (bar 3 passes), `--dump-black`; bar 2 the wait adds no black (0.0000) but 4492 chunks with their reach in the full radius stay unbaked at 1800 under both waits; bar 5 the fly-through opens 0 provisional jobs; bar 6 `partial` 967 black against 1338, not under 1 % | `76eae309` |
| 19: The key light on the pair's `g`, the grain after a turn | dev | done in part: built; bar 1 fails (p99 2 against the lamp's 1, 1 before); the gate test `a_tree_shadow_travels_with_the_sun_at_320x180` fails (ratio 0.767); bar 3 at `60,180` 3.00 against 2.11; bar 4 met (7 frames); the drift rig off 13.9 voxels before and after. Fix pass: `reset_history` clears `encoded`, the tree test passes (1.733); the drift rig's fault (one held open floor) fixed, off 0.385 voxels; bars 1 and 3 accepted as re-stated | `37e03683`, `fd1e4f2d`, `5646af8b` |
| 20: The seam between the baked sky term and the ray's | dev | done (reading only): at 19.png's pose the ray region is 0 from frame 300 under the default and `partial`, 100 % with the wait on (0 chunks baked in 600 frames); none of (i) to (iii) built; bars 1 and 2 not written (no ray-lit pixel at rest) | `44e24090` |
| 21: The flash around a dig | dev | done in part: (c) and the seed (`SEED_AGE` 8) built; bars 1, 2, 4 and 4a (re-stated: no stale record inside the box) pass; bar 3 fails after the seed (1.854 % dots at the dig's first frame, 0 from 11 frames on); the open seed fails `a_closed_box_takes_no_sky_light` (0.232 against 0.063) and `an_absent_chunk_below_the_column_top_is_rock_to_the_sky_ray` (0.663 against 0.031); bar 7 dots 36 to 29 | `0f8cde39`, committed with this row |
| 21a: A chunk an edit fills from air is a dig | dev | not started (added 2026-11-18) | |
| 22: The fired lights' grain under the anchor | dev | done: no mechanism, the backlog entry; bar 2 read with two auto-exposure arms (anchor 4: at rest p99 10, edge flips 6.09 %; anchor 0: 6, 1.10 %); landed before Phase 21 | `59d0ea90` |
| 23: The sunlit treads' clip | dev | not started | |
| 24: The owner's third window | human | not started | |
| 25: The picks as tests | dev | not started | |

### Notes

- Tree names (main since the draft): Plan 0069's surface is `RenderSettings` in
  `crates/vxe-render/src/settings.rs` (`FIELDS`, `NOT_SETTINGS`); `frame/settings.rs` holds
  `Lighting`. Plan 0081 has not landed.
- Phase 1: the rejection counter is four words after the store's five in the temporal pass's
  counter buffer, written under the `COUNT_STORE` override (`Frame::set_count_store`), read by
  `Frame::read_history_rejections`, not a steps-view output. The hooks are `Frame::set_light_hooks`
  (`LightHooks`), two `Params::lighting` bits. The tunnel's regions are chosen per lighting pixel
  by the sky ray's hit rate (as Plan 0032's `the_tunnel_under_one_sky_ray`), not by a pinned
  rectangle; the rectangles are printed. The forest region is every hit.
- Phase 2: the style word lives in a third block of the emission-scale buffer, not a new
  binding beside the material upload; a brick is styled by any id of its palette, not one
  voxel's. The banner names only the non-default `leaf voxel` (the start banner is at its
  9-row cap); the parse test pins that. Bars 1 and 2 run as `reading::a_flagged_canopy_shadows_as_its_bricks`
  in `gpu_sun.rs` (exploration); bar 3's goldens run at the tip's gpu tier.
- Phase 2 commit 2b5fff92 left `hud/modes.rs` at 1205 lines (over the 1200 cap); Phase 3's
  commit brings it to 1200. Phase 1's matrix ran without the fly-through's far normal.
- Phase 3: a reading hook `LightHooks::no_stair` reads the rig before and after in one build;
  bars 1 and 2 run as readings (`reading::a_far_staircase_keeps_its_history`,
  `reading::a_thin_far_wall_does_not_leak`); `LIFT` 2 is not read. Bar 1's rig reads 0.05 %
  resets with the rules off: at the far normal's full weight the stored normals already agree.
- Phase 4: `E_sky` lives in a 96 B storage buffer the temporal pass binds (`Frame::sky_irradiance`),
  written by a new pass `sky_irradiance.wgsl`, not in the sky uniform. The baked term is
  `v E_sky / h`, `h` the hemisphere's cosine share above the horizon (1 for `+z`, a half
  sideways), not `v E_sky`: bars 2 to 4 hold together only so. The switch is per lighting pixel
  on the hit face's own cell (`VIS_BAKED`), with no fade. Ring 3 opens on a touched brick's
  visibility before the edit, not after. Bars 2, 3, 4 and 6 run as readings; bar 7 is partial
  (the log lists what is owed); bar 6 reports Phase 1's change statistics and mean levels, not
  patches a step. One GPU reading ran while a live session runner was up.
- Phase 5: the light pass hands the sky rays' share to the temporal pass through a third
  target (`rgba8unorm`), so `frame_bytes` grows by 20 B a lighting pixel, not the pair's 16.
  The age falls in proportion to the band's breach (at least 2) rather than to 8, and the clamp
  is at `FIREFLY_K` times the larger of the long and short means. No fade from the pair's mean
  to the stored value; the pair is seeded with the baked visibility where no ray is cast. With
  `sky_rays` 0 a stale ring brick shows its stale value (bar 4's "nothing before" not taken).
  `LightHooks::sky_ray_always` and `no_clamp` are reading hooks. Bars 1 to 4 run as readings;
  bar 3's rig selects one pixel; bar 5 is a gpu-tier test not yet run; bar 6's light pass during
  a dig is not read. `FRAME_PASSES` gained `sky irradiance` (fixes Phase 4's shifted pass names).
  `encode.rs`'s output binds moved to `frame/output_binds.rs`. `docs/flythrough.md` rows for
  `--sky-rays` and `--sky-history`.
- Phase 5 follow-up (a `fix` commit before Phase 6): the pair's term goes to the direct part at
  every hit, a cube face's too (Phase 5 put a face's into the unfiltered stored part; the far
  end of the unbaked tunnel read 90 p99 and metered dark for about 90 frames). The readings'
  `run_cell` pinned a stale exposure (one readout after the loop); it reads every frame now.
  Two clippy lints in Phase 5's `gpu_sun.rs` fixed.
- Phase 6: stopped at its reading (the 3-degree band's p99 1, the lamp's 1); the lamp's
  calibrated level read 193 rather than 118. No forest lamp reading.
- Phase 7: the probes run in empty cells of edge 8 or less at points about 2 voxels apart
  (the plan's sketch: each coarse step; the resume sketch: cells of 2 or less), and the steps
  view's key ray takes the approach too (`shade.wgsl`, outside the phase's files), as do
  `frame/compose.rs`, `lib.rs` and `docs/flythrough.md`. The approach penumbra is one-sided
  (lit side only). The readings' `run_cell` reads the exposure blocking now. Bar 1's canopy
  profile not read; bar 2's light pass read for the disc on the RTX only.
- Before Phase 8 (a `fix` commit): the gpu tier's `an_absent_chunk_is_sky_to_the_sky_ray` read
  0: `centre_lit` (Phase 5) read the per-frame direct target, and the follow-up's fix put the
  pair's term in the accumulated direct part; `centre_lit` reads `direct_accumulated` now.
  The soak test `the_cave_mouth_is_lit_from_outside` fails its seal bar at the tip (sealed
  walls 24.3 % of unsealed after 64 frames, bar 2 %) and at `012bd208` (unsealed 0); not
  changed.

- Phase 9: the column map's max down a column is CPU work in the sync (`gpu_world/columns.rs`,
  `queue.write_texture` of a column's tile), not a GPU pass; no `column_map.wgsl`. The map is an
  `r32sint` texture at group 0 binding 19, 2-voxel cells, i32's minimum for none. The key light's
  ray keeps the old rule (only the sky rays take the column rule). `QueueView` gained
  `lod_radius` (the columns' wanted chunks), so `apps/flythrough/src/run/queue.rs`,
  `run/loading.rs`, `tests/support/queue.rs` and `bake/pre_roll.rs` changed outside the phase's
  files. Reading hook `LightHooks::sky_end_is_sky`. Bar 1's dig is a 2 x 2 x 1 dig (a texel's
  columns), read at the dig's sync. Bar 3 is the new soak test
  `the_underground_eye_reads_no_sky` over `reading::underground_eye`. Bar 4: the wall's 0.549
  is outside 0.50 within 0.03, equal to Phase 4's reading. Bar 5's seal bar fails as written
  (24.347 %); bar 5's golden fails (re-blessed at Phase 11). The bake's finality lengthens the
  pre-roll past the risk's fifth (readings log).
- Phase 10: shrank to one fix, the missing-light chain's sky (`store_sky_fallback`,
  `sky_chained`; own and neighbour summaries, no parents); the young pair's band and the coarse
  pass's `VIS_BAKED` not built (bar 1 read 0 at frame 1 after Phase 9). `STORE_SUMMARY_AT` moved
  to `hitrec.wgsl`, and `bake/summary.rs`'s test with it (outside the phase's files). Bar 3's
  rig is the streamed test rig, not the fly-through (no headless jump); its frames were not
  reached. The bar-1 soak test and the forest reading landed in Phase 9's commit.

### Close triggers

_(Facts for the architect. No recommendations, and no suggested version bump.)_

- **What shipped:** feature / fix-only / docs-chore-only
- **Public API changed:** types, traits, functions, shader interfaces, config (list them, or none)
- **Budget readings:** each `nfr.md` figure touched, with its measured value, machine and commit
- **Gate at the tip:** the commands run (build, nextest, doctests, clippy, fmt, deny), exit codes, test counts
- **Outstanding `human` phases:** which, or none

## Followups

- `L1` sky visibility per brick direction (ADR-0070 Alternative F), if the window reads the
  averaged colour as wrong.
- Per-face sky cells near the camera, with the backlog's per-face emissive cells.
- A render profile (Plan 0069) that sets `sky_rays` 0 on the slow adapter, once Phase 5's
  reading says what the ray costs during a dig there.
- A spatial seed for the pair (the 3 x 3 accepted neighbours' mean at a reset pixel, before
  the record's and the chain's): if the forest's turn strip stays over its bar after Phase 21
  (a) (added 2026-11-18).
- Plan 0090 gains a stamp scene (a prefab placed on the hills at noon, the eye in it; rules 3,
  4 and 5) and reads the key mean's fade-in length against the calibrated `F` (the flicker
  amendment's owner question 1); an architect amendment on main, owed at this plan's close.
