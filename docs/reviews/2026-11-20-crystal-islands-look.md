# The Crystal Islands look, 2026-11-20: what the frames show, what we do, and the gap

> **Written:** 2026-11-20 on the branch `look-ideas` from main `e7752e0d`. **Read-only:** no
> cargo, no GPU run (the owner's window session holds the perf lock). Every engine claim below
> is read from the file or ADR it names; a claim this review infers says so.
> **Asked for by the owner,** who found John Lin's "New Voxel Engine Reveal - Crystal Islands
> Experiment" (YouTube `8ptH79R53c0`) and asked what gives it its colours. Lin published no
> rendering write-up, so the judgement is from thirteen frames at 1280 wide, named below by
> their timestamp. The frames are copyrighted and are not in the tree; each is described.
> **What this is not:** a plan. It splits content levers (our testbed's palette and scenes, a
> consumer's assets) from engine levers, and the engine levers worth keeping went to
> [`docs/backlog.md`](../backlog.md) the same day, each citing this review.

The one-paragraph verdict: **the look is honest cubes, pale high-albedo rock, strongly
saturated emissive crystals, a per-voxel colour jitter on nearly every material, and a low-key
exposure in the caves.** Nothing in the cave frames needs a mechanism we lack: our baked store
already puts an emitter's coloured irradiance on the faces around it. Our testbed draws the
same physics on a dark warm-grey rock (`albedo 0.42 0.38 0.34`) with three glowing rows, so
the walls cannot take the crystals' colour the way Lin's near-white limestone does. The one
engine mechanism the frames show that we have nothing for is the per-voxel albedo variation;
it is cheap and stateless, so it fits the stability rules. The daylight frames add a
sky-dominated soft light, aerial haze and sub-voxel foliage, which Plans 0035 and 0081 already
carry.

## The frames

| at | what is on screen |
|---|---|
| 0:10 | Close foliage: cobalt-blue lupin spikes built of clearly visible cubes, five to eight voxels across; fan-leaved plants in two greens with paler tips; a tan trunk; a magenta crystal cluster behind, its core near white; a pale rock wall. High-key, no hard shadow. |
| 0:35 | A palm from below: saturated green fronds one voxel thick, a banded ochre trunk, a white-ish bright sky through the gaps. Fronds shade by orientation only, no cast shadow visible. |
| 1:00 | A wide valley at sunset: peach sky, far rock desaturating toward the sky's colour (aerial haze), pale tan and pink rock, dozens of pastel crystal clusters (green, yellow, white, pink), trees in three or four greens, purple and magenta flower beds. Two huge white crystals top right and bottom right blow out to flat white. |
| 1:20 | A crevice from above, low-key: tan rock in deep shadow, foliage in blue-green and grey-green, muted; the floor of the crevice almost black. Sky light only, from one side. |
| 1:38 | The cave: near-white limestone vault, exposure low so most rock sits in the dark greys; cyan, blue, violet and red crystal clusters each a saturated emitter; the wall within two to three cluster-widths of each takes its hue as a soft gradient; a red glow at the bottom. No visible halo beyond the geometry's own glow; a soft edge on the top cluster that may be the video's compression. The cubes are not visible at this distance. |
| 2:00 | One cobalt crystal cluster at arm's length: the prisms show stepped voxel edges, a dark saturated body, pale streaks along each prism and bright tips; nothing specular in the sense of a moving highlight, the streaks look painted into the voxels. Pale rock beside it takes a blue cast; a magenta cluster below throws magenta on the floor. |
| 2:25 | Foreground rock in shadow (umber), a cyan and a lime cluster behind, white clusters further back. The shadowed rock keeps a faint cyan on the side facing the cluster. |
| 2:50 | A green prism filling the left half: its surface is a dense per-voxel speckle of two or three greens (sugar-like), stepped edges on every face, brighter streaks; rock behind in flat tan. The per-voxel variation is unmistakable here. |
| 3:15 | A great vault from inside: tens of clusters in green, lime, yellow, white, cyan, blue, violet and magenta; pale pink-grey walls lit mostly by them; one palm in the middle. Low-key, the far ceiling in near black. |
| 3:40 | Blue and violet clusters on pinkish rock, low-key; the rock takes violet near the upper cluster and blue near the lower ones; no halo. |
| 4:04 | Daylight over the islands: a flat grey-blue sky, tan rock blocks, dense foliage in many greens with red and white flower beds, cyan and pink clusters. Soft shading throughout, no hard sun shadow anywhere in the frame; the tree crowns darken toward their undersides as sky light would. High-key. |
| 4:35 | Red lupins against a clear blue sky: the reds are fully saturated, white fan-leaf plants, lime grass, a pale tan rock wall. The sky is clear yet the shading is still soft: a weak sun, or none, over a bright sky. |
| 4:55 | Straight down: a carpet of trees in four or five greens, red and blue flower beds, pastel clusters, tan rock in the gaps. Per-voxel speckle on every crown. |

What the two frames the orchestrator read first said, checked: the 1:38 claims hold except
the bloom, which is faint at most and absent at 2:00, 3:40 and 2:25; the 4:04 claims hold,
and 4:35 sharpens "overcast-like" to "sky-dominated even under a clear sky". The cubes are
visible at arm's length in every close frame (0:10, 2:00, 2:50), so this is the honest-cubes
look we chose (ADR-0041), not a smooth surface. Lin's voxel size relative to his plants is
finer than our 10 cm (a lupin spike is six to eight voxels wide); it is unknown in metres.

## Per trait

### 1. Neutral pale walls take the emitters' colour (1:38, 2:00, 2:25, 3:15, 3:40)

- **In the frames:** the wall's lit colour is the crystal's hue at roughly a third of its
  saturation, falling off over a few cluster-widths; where two clusters overlap the wall mixes
  them. The rock's own albedo is near white with a pink or tan bias.
- **Ours today:** [ADR-0046](../adrs/0046-static-emissive-light-is-baked-per-chunk-when-a-chunk-loads-or-changes.md)
  bakes per surface brick six direction values of emissive irradiance at unit emission, one
  shadow ray per sample from the face to an emitter, and the read multiplies by the dominant
  emitter material's emission now (`crates/vxe-render/src/bake.rs`, `temporal.wgsl`'s store
  read). The composite draws `albedo * lit / pi + emission` (`composite.wgsl`, `composite_px`).
  So the wall's tint is the emitter's colour times the wall's albedo, as in the frames.
  A brick with no value reads the fallback chain
  ([ADR-0053](../adrs/0053-a-light-summary-per-block-and-a-two-level-parent-table-light-a-brick-with-no-value.md)).
  There is no second bounce from the wall onward; the probe volume is a lamp-only channel
  (ADR-0043, `Lighting::lamp_bounce`).
- **The gap is content first.** `apps/flythrough/assets/materials.txt`: rock `0.42 0.38 0.34`,
  rubble `0.36 0.33 0.30`, the showcase's three glowing rows (`crystal` cyan, `amethyst`
  violet, `ember` red) and the ore's orange. At albedo 0.4 the wall returns less than half the
  irradiance Lin's wall does, and under the auto exposure
  ([ADR-0033](../adrs/0033-eye-adaptation-as-an-asymmetric-two-stage-exposure-on-the-gpu.md))
  the key then sits on the emitters, which clip. `docs/images/hero.png` shows exactly this: tan
  pillars, near-white lamp cores, a weak cyan on the dome. **Engine side:** the store's cell is
  the 8-voxel brick, bilinear over 2 x 2, so the falloff within a cluster-width of a small
  emitter is smoothed over 8 voxels; the backlog's "per-face light cells as a near-camera
  enhancement" entry holds that option, and nothing in the frames says it is needed before a
  window on a re-paletted scene says so.

### 2. Saturated emitters whose brightest parts roll to white (0:10, 1:00, 1:38, 2:00)

- **In the frames:** a cluster's body stays saturated and its tips and the largest crystals
  go to white (1:00's giants are flat white). That is a per-channel tone curve clipping, the
  same family as ours.
- **Ours today:** the ACES fit per channel, clamped (`composite.wgsl`, `aces`, `display`);
  `ToneMap::Identity` is the only other option (`frame/settings.rs`). The emitter's own face
  adds its emission unscaled. So a cyan `0.35 1.6 2.0` row at exposure 1 lands at about
  `0.49 0.89 0.92`: pale cyan, as the hero shows.
- **Gap:** none in kind. The backlog's "hue-keeping tone map" entry (2026-11-15) asks for the
  body to keep its hue longer before white; the frames say Lin lets the brightest parts clip
  too, so that entry is a refinement, not a prerequisite.

### 3. Low-key cave exposure (1:20, 1:38, 3:15, 3:40)

- **In the frames:** most of the rock sits in the lower quarter of the display range; the
  emitters are the only things near white; the far ceiling goes black.
- **Ours today:** the auto exposure meters centre-weighted log luminance toward a key of 0.18
  with a game-set `compensation_ev` (`crates/vxe-render/src/exposure.rs`, `AutoExposure`), and
  the profiles ([ADR-0055](../adrs/0055-render-settings-are-one-struct-with-an-apply-class-per-field-and-profiles-are-named-values.md))
  carry its settings. A dark key is one number.
- **Gap:** content and settings. The backlog's "darker scenes, and light intensity varied by
  seed" entry (2026-11-15) already carries the owner's wish; the frames give it a reference:
  a key one to two EV under ours in the cave scenes, the emitters dimmer and more varied.

### 4. Per-voxel colour variation on nearly every material (0:10, 0:35, 2:50, 4:04, 4:55)

- **In the frames:** foliage crowns are a speckle of three or four greens per tree; the 2:50
  prism's faces are a sugar of two or three greens; even the tan rock at 2:25 and 3:15 is
  mottled. The variation is per voxel and does not move, so it reads as texture, not noise.
- **Ours today:** one `albedo` per material row (`crates/vxe-world/src/material.rs`); the
  composite reads `materials[rec.material * 4u].xyz` and nothing varies it. A `.vox` asset
  can carry variety because every palette entry becomes a row
  ([ADR-0065](../adrs/0065-materials-are-data-a-table-file-in-the-rig-grammar-and-every-vox-palette-entry-becomes-a-row.md)),
  but the procedural sources (`apps/flythrough/src/cave.rs`, `trees.rs`: one `leaf`, one
  `bark`, one `rock`) put one row on everything, and a consumer's generator would too.
- **Gap: engine, and the one worth building.** A per-material variation amplitude applied at
  the composite from a hash of the hit voxel's integer position (the hit record gives the
  position and the face; ADR-0043's key `floor(p - 0.5 n)` names the voxel). Stateless, no
  history, no random stream, so it adds nothing a pop counter
  ([ADR-0076](../adrs/0076-light-stability-rules-world-history-per-source-a-fade-per-transition-and-a-ratcheted-pop-counter.md))
  can see, and under FSR 2 it is a static albedo pattern like any other. Cost: one hash and
  one multiply a pixel; a guess until measured. Where the amplitude lives is the owner's
  call: a row field (the material file gains a line; an ADR since
  ADR-0065's row changes shape) or the render-side per-material style word
  ([ADR-0063](../adrs/0063-surface-detail-is-a-procedural-shell-marched-at-the-hit-described-per-material-on-the-render-side.md),
  cosmetic and client-only, which is what this is). The backlog's "ambient occlusion driving
  a per-material colour ramp" entry names a hue jitter by low simplex noise over world `xz`
  for the same purpose; this is the per-voxel form of it.

### 5. Many hues in one scene (1:00, 3:15, 4:04, 4:55)

- **In the frames:** eight or more crystal hues, four or five foliage greens, flowers in red,
  white, purple, blue and pink, rock in tan, pink and grey.
- **Ours today:** rows are data and the table is large (`EMISSION_ROWS` 65536 in
  `common.wgsl`); the showcase picks three glowing rows and the cave one ore.
- **Gap: content.** A testbed palette pass: a pale rock with two or three tints, six to eight
  crystal rows at lower and varied emission, two or three leaf rows the tree generator picks by
  hash. Apps only; no engine change.

### 6. Soft, sky-dominated daylight (0:10, 0:35, 4:04, 4:35, 4:55)

- **In the frames:** no hard cast shadow in any daylight frame, even under 4:35's clear blue
  sky; crowns darken toward their undersides; crevices (1:20) go dark by occlusion of the sky.
- **Ours today:** the sun is one jittered ray at a 3 degree disc
  ([ADR-0027](../adrs/0027-the-sun-shadow-is-one-per-pixel-ray-in-the-light-pass.md), the
  owner's pick at Plan 0032's window), sky light is baked visibility coloured at read
  ([ADR-0070](../adrs/0070-sky-light-is-baked-visibility-coloured-at-read-with-a-long-history-ray-where-stale.md)),
  and the sky's radiance is Hillaire's LUTs scaled by the sun's illuminance
  ([ADR-0026](../adrs/0026-atmosphere-from-hillaire-luts-built-on-the-gpu-at-runtime.md);
  `crates/vxe-render/src/sky.rs`, `view_scale` is one over the larger body's illuminance). So
  dimming the sun dims the sky with it; `Sky::uniform` is the only sky that is bright with no
  sun. Whether the baked sky term reads the same scale: not checked here.
- **Gap:** an overcast or sky-dominated preset. Plan 0035 (approved) carries the cloud layer
  whose `cloud_sky` term is the natural owner of "a bright sky under a hidden sun"; the
  backlog's canopy-shadow entries carry the rest. Nothing new to park; the frames are a
  reference for Plan 0035's look phase. The cave consumer does not need it.

### 7. Aerial haze (1:00)

- **In the frames:** far rock fades toward the sky's peach; the near stays saturated.
- **Ours today:** nothing. The composite adds no distance term; `sky_view.wgsl` is the sky
  alone.
- **Gap:** Plan 0035's aerial perspective, approved and not started. Not a cave trait.

### 8. Fine detail: thin fronds, flower spikes, speckled crowns (0:10, 0:35, 4:04)

- **In the frames:** fronds one voxel thick, flowers a few voxels wide, so the voxel is small
  against the plant.
- **Ours today:** 10 cm world voxels; the detail layer of Plan 0081 (approved, ADR-0063) for
  blades, caps and pebbles; entity grain finer than a world voxel
  ([ADR-0078](../adrs/0078-an-entity-part-has-a-grain-a-detail-finer-than-a-world-voxel-or-a-coarse-factor-of-world-voxels-per-model-voxel.md)).
- **Gap:** scale and content, carried by Plan 0081 and the consumer's assets. A crystal
  cluster built as a stamped prefab at world scale reads as 2:00 does with no new mechanism.

### 9. Crystal bodies: dark saturated core, pale streaks, no moving highlight (2:00, 2:50)

- **In the frames:** the streaks sit on fixed voxels and do not look view-dependent; the
  body is darker than the tips, so the emission is not uniform over the cluster.
- **Ours today:** an emissive row emits uniformly; a translucent row filters by path length
  with a small Fresnel share ([ADR-0047](../adrs/0047-translucent-solids-filter-by-path-length-with-a-baked-reflected-share.md));
  the mirror knows emissive translucent as a class (`gpu_world/emitters.rs`). No specular term
  reads `roughness` in any shader (the word appears once, in a layout comment in
  `common.wgsl`).
- **Gap:** content, through trait 4. Two rows per crystal (a dark body, a bright tip or
  streak) or the per-voxel variation applied to emission as well as albedo gives 2:00's body;
  a specular term is not what the frames show.

### 10. Bloom (1:38, 2:00, 3:40)

- **In the frames:** faint at most; the clusters' edges are sharp against the rock at 2:00
  and 3:40, and 1:38's soft edge is within what the video's compression does to a saturated
  edge.
- **Ours today:** none.
- **Gap:** the backlog's "bloom from HDR emission" entry (Bonsai) stands, lower than before:
  the frames say the look does not rest on it.

## Content levers (no engine change; the testbed's palette and scenes, a consumer's assets)

1. A pale rock (albedo about 0.7 to 0.8 with a tan or pink bias) in the showcase and the cave,
   so the emitters' colour has something to land on. One line each in `materials.txt`; the
   file is re-read once a second while the window is open (Plan 0083), so the owner can tune
   it live.
2. More and dimmer glowing rows: six to eight hues at a third to a half of today's emission,
   and the ore's orange among them rather than alone.
3. A darker key or a negative `compensation_ev` in the cave profiles, one to two EV.
4. Several leaf and rock rows picked by hash in the procedural sources (apps side), until the
   engine's per-voxel variation (below) makes that unnecessary.

## Engine levers, ranked by payoff over cost and by fit with the stability rules

1. **Per-voxel albedo variation by a seeded hash, a per-material amplitude** (trait 4). New.
   Small; stateless; the only mechanism the frames show that we have no form of.
2. **A hue-keeping tone map** (trait 2). Existing entry; small; moves goldens; the owner's eye.
3. **Per-face light cells near the camera** (trait 1). Existing entry; medium; only if a
   window on the re-paletted scene reads the brick smear.
4. **Per-source indirect light** (trait 1's second bounce). Existing entry; large; the frames
   do not need it for the first-order look.
5. **Bloom** (trait 10). Existing entry; small to medium; the frames weaken its case.
6. Sky-dominated daylight and aerial haze (traits 6, 7): Plan 0035 as approved; the frames
   are a reference for its look phase, no new entry.

## For the owner

- Whether the content pass (levers 1 to 3) runs now as a small apps chore, judged in a
  window, before any engine lever: this review recommends it, since it is the cheapest test
  of the thesis that the colours are albedo times irradiance we already compute.
- Where the per-voxel variation's amplitude lives: the material row (an ADR amending
  ADR-0065's file) or the render-side style word of ADR-0063. The review leans to the style
  word: it is cosmetic, client-only and already per material.
