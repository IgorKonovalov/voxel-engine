# Procedural terrain and vegetation: external sources, and what fits this engine

An annotated reading list on generating terrain and placing vegetation procedurally, collected
2026-11-28 after ADR-0090 and the drafts of Plans 0113 to 0115. Every entry says what the
source describes and what we take from it, so a plan can cite it by title. Numbers are the
sources' own; `nfr.md` holds ours. The last two sections list what the drafts already cover,
the gaps, and the owner questions this reading raises.

## The filter: what a technique must survive here

Most of the literature assumes a whole heightmap in memory and a GPU. This engine has rules
that rule out much of it, so every entry below is judged against them:

1. **Pull, not push.** A chunk is a pure function of (seed, chunk coordinate, version), built
   in any order on any thread (roadmap, "Generation determinism"; Plan 0113's pull rule).
   Anything that needs its neighbours' *output* (a simulation, a flood fill, an iterative
   relaxation) is ruled out unless it runs over a bounded, seeded region that every chunk
   can recompute.
2. **Integer or fixed point, bit-identical on Linux and Windows.** Float noise from a crate is
   out for anything the engine ships; the consumer's own source may use floats only if it
   accepts platform drift.
3. **Evaluable at any scale.** ADR-0028's far field asks the source for coarse chunks
   directly, with cells of `2^k` voxels out to about 1 km. A feature that exists only after
   the full-detail chunk is built (a decoration pass, a stamped prefab) is invisible there
   unless it also has a coarse answer.
4. **10 cm voxels.** A 20 m tree is 200 voxels tall; a leaf is smaller than a voxel. Plant
   forms are rasterised volumes, not meshes with cards, and a sparse canopy sparkles under
   jitter (the backlog's "Sunlit tree crowns sparkle in motion").
5. **The engine owns mechanisms, the consumer owns rules** (CLAUDE.md). Noise, scatter,
   neighbourhood queries and shape builders can live in `svet-world`; biomes, plant species
   and what grows where are the consumer's (here `apps/`, Plan 0114).

## Terrain

### Surveys

- **Galin, Guérin, Peytavie, Cordonnier, Cani, Benes & Gain, "A Review of Digital Terrain
  Modeling", Computer Graphics Forum 38(2), Eurographics 2019 STAR.** Sorts terrain methods
  into procedural, physically simulated (erosion, uplift) and example-based, and compares them
  on landform variety, realism and scale. What we take: the map of the field, and its
  observation that procedural methods are the only family that is local, which is what rule 1
  needs. Simulation and examples are for offline, bounded maps. Free copy on the Purdue CGV
  lab page.

### Noise and its shaping

- **Ken Perlin, "Improving Noise", SIGGRAPH 2002; Inigo Quilez's articles on value noise
  derivatives, fBm and domain warping (iquilezles.org).** Gradient noise with a quintic fade
  is C2 across cells; value noise with a linear blend is not. Analytic derivatives come almost
  for free, and they drive slope-aware octaves ("erosion-like" fBm that damps detail on steep
  slopes) and normals. Domain warping (`f(p + g(p))`) turns round blobs into flowing,
  folded shapes. What we take: our `svet_world::hash::ValueNoise` blends linearly, so a
  terrain built from it has creases on the lattice and no usable derivative. An integer
  gradient noise with a quintic fade and fixed-point derivatives is the missing primitive
  (see "Gaps").
- **Minecraft Java 1.18's terrain (the Minecraft Wiki, "Tutorial: Custom world generation",
  and the 1.18 data-pack density functions).** Five low-frequency noises (continentalness,
  erosion, peaks-and-valleys from weirdness, temperature, humidity) are mapped through
  authored splines into a height offset, a vertical squash factor and a jaggedness. The final
  density is a 3D function (`squash * (offset - y) + jagged * noise3d`), so cliffs, overhangs
  and arches come from the same function as plains. Biomes are picked as the nearest point
  in the 5D parameter space, so terrain shape and biome always agree. What we take: the
  pattern of **parameter fields → splines → a 3D density**, every stage a pure function of
  position. It is pull-safe, has an obvious coarse answer (sample at the cell centre) and
  gives vegetation its inputs (humidity, temperature, erosion) for free. Its noise and
  cheese/spaghetti/noodle caves are the same idea underground, which suits the first
  consumer. Sources are community and decompiled-data pages, not official docs.
- **Hytale's world generator (Hypixel, "A closer look at landscape generation in Hytale",
  2020; the V2 "Orbis" generator's node editor and zone, biome and prefab layers, 2025-26).**
  Zones hold biomes; biomes hold props, prefabs and points of interest; designers author it
  in a node graph. What we take: the layering (zone → biome → prefab), and the reminder that
  designers want the rules as editable data. That fits ADR-0072's text-first authoring, but a
  node editor is out of scope. Mostly fan and modding docs; treat the details as unverified.

### Erosion

- **Rune Skovbo Johansen (runevision), "Fast and Gorgeous Erosion Filter", blog post, March
  2026; Shadertoy source under MPL-2.0; Rust port `bevy_erosion_filter`; 80.lv's write-up.**
  A noise-like filter that adds branching gullies and ridges to any heightfield, given its
  height and gradient. Each point is evaluated on its own in one pass, with no simulation,
  built on the Fewes and clayjohn Shadertoy erosion noise. The author's motivation is exactly
  rule 1: simulated erosion cannot make infinite terrain. **The best fit found in this
  reading.** It is pull-safe, has a coarse answer (evaluate fewer octaves), and gives the
  eroded look that the canyon cliffs of Plan 0114 want. Costs: it needs the base height's
  gradient (so the gradient noise above first) and a fixed-point port. MPL-2.0 is in
  `deny.toml`'s allow list, but a port of the shader is a derived work under MPL's
  file-level copyleft: the ported file keeps its licence header.
- **Mei, Decaudin & Hu, "Fast Hydraulic Erosion Simulation and Visualization on GPU", Pacific
  Graphics 2007; Št'ava, Beneš, Brisbin & Křivánek, "Interactive Terrain Modeling Using
  Hydraulic Erosion", SCA 2008.** Water, sediment and outflow pipes per heightmap cell,
  iterated on the GPU. What we take: nothing for streamed generation (rule 1 and rule 2). It
  could serve an offline tool that bakes a bounded region map for a consumer to ship as
  data, which a `WorldSource` could read. Not the engine's job unless a consumer asks.
- **Cordonnier, Braun, Cani, Benes, Galin, Peytavie & Guérin, "Large Scale Terrain
  Generation from Tectonic Uplift and Fluvial Erosion", Eurographics 2016.** Builds a
  river-network graph over sparse points and solves the stream-power law for height.
  Geologically right ridges and valleys at kilometre scale. What we take: the idea that
  **large features can be decided on a sparse, seeded lattice** that each chunk recomputes
  locally (see "Regions" below). The full solve is global, so it stays offline.

### Rivers, lakes and other non-local features

- **The ridged-noise channel** (common practice; Quilez, and Plan 0114's "winding channel").
  A river is where a low-frequency noise crosses zero: `|n(p)| < w` carves the bed, and the
  same field slopes the banks. It is local and has a coarse answer. It cannot guarantee
  rivers flow downhill or join; for a game that only needs water that looks plausible, that is
  enough.
- **Regions on a jittered lattice** (the pattern behind Minecraft's structures, Hytale's
  zones and our own `trees.rs` scatter cell). A coarse lattice (for example the 256 m region
  cell of Plan 0114) where each site's decisions (a lake here at this level, a ruin there of
  this kind) are a hash of (seed, site). A chunk recomputes the few sites whose influence
  radius reaches it. This is how non-local things (a lake basin with a flat level, a ruin
  footprint that flattens terrain under it) stay pull-safe: the decision is non-local, but
  it is computed from the seed, not from other chunks' output. It is also the place for a
  consumer's hand-placed sites.

### Rejected for the engine

- **Example-based and learned terrain** (GAN or diffusion heightmaps, texture-synthesis from
  DEMs; Galin et al.'s third family). It brings a model dependency, cannot be bit-identical
  across platforms under rule 2, and offers nothing that the spline pattern does not, at our
  scale.

## Vegetation

### Where things grow (distribution)

- **Jaap van Muijden, "GPU-Based Run-Time Procedural Placement in Horizon Zero Dawn", GDC
  2017 (Guerrilla's slides and the GDC Vault).** Placement as layers. Each asset type has a
  density expressed as a small graph over world maps (height, slope, moisture, painted
  masks) and over other layers' footprints, so trees clear space and undergrowth avoids
  trunks. The density is dithered into points on the GPU as the player moves. They report
  over 500 asset types, about 100 000 objects in the scene and about 4 MB of world data per
  km². What we take: **density as a function of local terrain and of the parameter fields,
  then a deterministic dither, layered big-to-small with exclusion**. Their GPU step is out
  (rule 2). The CPU equivalent is a hash threshold per scatter cell, which `trees.rs` already
  does for one layer.
- **Deterministic Poisson-disk scatter by priority** (common practice; the "dart throwing
  with hashed priorities" variant). Every candidate (one per small cell, jittered by a hash)
  carries a hashed priority. A candidate survives if no candidate within radius `r` has a
  higher priority and also survives, which with a bounded `r` needs at most two rings of
  neighbours. It is pull-safe and order-free, and gives blue-noise spacing instead of the
  clumps of a plain per-cell hash. Layering is the same rule with a cross-layer radius
  (trees exclude shrubs within 2 m). What we take: this is the scatter primitive the engine
  could own. Plan 0113's anchors and ADR-0066's placements both need a "where" that is
  better than one hash per cell.
- **Makowski, Hädrich, Scheffczyk, Michels, Pirk & Pałubicki, "Synthetic Silviculture:
  Multi-scale Modeling of Plant Ecosystems", SIGGRAPH 2019.** Simulates competition for
  light, water and space over up to about 500 000 plants and centuries, reusing plant modules
  across the ecosystem. What we take: only the result, that a forest is graded by competition
  (big crowns spaced out, understory in the gaps, edges denser where light reaches). Priority
  scatter with a per-layer radius and a light term from the terrain (open sky above, Plan
  0113's `sky_above`) fakes it locally. The simulation itself breaks rule 1.

### What grows (plant forms)

- **Prusinkiewicz & Lindenmayer, "The Algorithmic Beauty of Plants", 1990
  (algorithmicbotany.org, free PDF).** L-systems: string rewriting plus a turtle. They give
  controllable branching, and deterministic ones are seeded rewriting. What we take: fine
  for small authored plants (ferns, vines, flowers), where a consumer wants to type the rule
  as text (ADR-0072's text-first authoring).
- **Runions, Lane & Prusinkiewicz, "Modeling Trees with a Space Colonization Algorithm",
  Eurographics Workshop on Natural Phenomena 2007.** Scatter attraction points in the crown
  volume, and grow branches toward the points within an influence radius, deleting each point
  once a branch gets within a kill radius. Competition for space gives natural, non-recursive
  crowns, and the crown shape is just the point volume. What we take: **the recommended tree
  builder.** It runs once per variant, at load, on a seed, like `trees.rs`'s
  `variant_shape`, so it is offline with respect to chunks and has no pull constraint. Branch
  thickness comes from the pipe model (a parent's area is the sum of its children's), which
  rasterises well into voxels.
- **Weber & Penn, "Creation and Rendering of Realistic Trees", SIGGRAPH 1995.** Parametric
  trees (levels of branching, curvature, taper, crown shape) with the parameters that
  arborists use. Simpler than colonisation, with less natural crowns. It is the alternative
  if colonisation's cost at load matters.
- **Infinigen (Raistrick et al., "Infinite Photorealistic Worlds using Procedural
  Generation", CVPR 2023; BSD-3, Blender-based).** Every asset (plants, terrain, scatters) is
  generated from math, with no stored art. What we take: an existence proof, and a catalogue of
  per-species rules worth reading when writing the demo's plants. Not a dependency (Blender,
  Python, float).

### What we already have, and why canopies sparkle

`apps/flythrough/src/trees.rs` builds trees as masked prefabs: a trunk, and a canopy whose
voxels are kept by a hash threshold (`--leaf-fill`). The hash makes the canopy sparse at the
voxel scale, and a sparse volume of one-voxel holes is exactly what aliases under jitter and
FSR 2 (rule 4). Plan 0115's dense clumps (a fill of at least 0.85, a boundary moved by whole
voxels) fix that by construction. Colonisation adds structure on top: **leaf clumps at the
branch tips** instead of one big crown volume, so the tree reads as layered masses with gaps
between them (the target look's sheet), and every mass is solid.

## How this maps onto the drafts

| Need | Technique | Where it lands today |
|---|---|---|
| Canyon terrain, terraces, cliffs | Height or density from noise, terraced | Plan 0114 Phase 1 (consumer side) |
| Eroded cliffs, gullies | Analytic erosion filter | **Not drafted** |
| Terrain and biome agree | Parameter fields → splines → density | **Not drafted** (0114 uses one heightfield) |
| Lake, ruins, non-local sites | Region lattice, sites recomputed per chunk | Plan 0114 (the 256 m region cell), ADR-0066 placements |
| Moss on tops, spills, stains | Neighbourhood rules, pull with a halo | Plan 0113 |
| Where clumps and trees go | Density × priority scatter, layered | Plan 0113 anchors, `trees.rs` per-cell hash; **no shared scatter primitive** |
| Tree skeletons | Space colonisation per variant at load | **Not drafted** (`trees.rs` is trunk + crown volume) |
| Clumps, vines, flowers as voxels | Shape builders (blob, strand, scatter) | Plan 0115 floor form |
| Sub-voxel leaves and blades | Procedural shell at the hit | Plan 0081, Plan 0115 rich form |
| Vegetation at 1 km | A coarse answer for every pass | **Missing** (see below) |

## Gaps and recommendations

In order of value. None is a plan yet; each would be an amendment to a draft at the Fable
pass, or a backlog entry.

1. **The far field does not see overgrowth or vegetation.** ADR-0028 asks the source for
   coarse chunks directly. Plan 0113's `DecoratedSource` and the stamped placements are
   defined on full-detail chunks only, and neither 0113 nor 0115 says what a clip chunk holds.
   In the target look's sheet the far cliffs are green on top, so a canyon whose distant
   ledges come back as bare stone fails the look at exactly the distances it is judged.
   Recommendation: every generation pass declares a **coarse answer**. For decoration, a
   rule evaluated on the coarse grid with its reach scaled (moss on a coarse top is still "a
   top with sky above"). For placements, a canopy-density field the coarse path samples, so
   a coarse cell under a forest is foliage without stamping one tree. An amendment to 0113
   and 0115, checked against 0033's seam reading.
2. **A smooth noise primitive in `svet-world`.** Integer gradient noise with a quintic fade
   and fixed-point analytic derivatives, beside `ValueNoise`, with the same determinism tests.
   Terraces, domain warp, slope-aware fBm and the erosion filter all need a derivative, and
   linear value noise shows its lattice on smooth slopes. Small, headless and engine-owned (a
   mechanism, rule 5).
3. **The erosion filter in the demo's terrain.** A fixed-point port of runevision's filter,
   in `apps/` (consumer side) first, keeping its MPL-2.0 header, applied to Plan 0114's
   canyon before terracing. It is promoted to `svet-world` only if a second scene wants it.
   Judged by eye in 0114's window, per the owner's rule.
4. **Priority scatter as an engine primitive.** `svet_world` gains a deterministic
   blue-noise scatter (candidates per cell, hashed priority, bounded radius, layers with
   cross-layer exclusion and a density callback). Plan 0113's anchors, 0115's placements and
   the demo's trees all use it instead of three hand-rolled per-cell hashes.
5. **Trees by space colonisation, leaf masses at the tips.** A seeded builder (attraction
   points in a crown shape, then the pipe model for thickness, then 0115's blob builder at the
   tips) that makes prefabs per variant at load. It replaces the hash-thinned crown, the
   sparkle's structural cause. The builder is a mechanism (`svet-world`'s prefab builders,
   beside 0115's blob and strand), and the species parameters are the consumer's.
6. **Parameter fields for the consumer, not biomes in the engine.** The Minecraft pattern
   (low-frequency fields → splines → density, biome as nearest point) belongs in the demo
   crate. The engine at most offers a fixed-point spline evaluator. Deferred until the demo
   needs more than one biome.

What we should **not** do: simulate erosion or ecosystems during streaming, use float noise
crates in anything the engine ships, or generate vegetation on the GPU. Each breaks a rule
above for a look the local techniques reach.

## Owner questions this raised (answered 2026-11-28)

1. Should the far field show vegetation and moss (gap 1)? **Yes.** Every generation pass
   gets a coarse answer, by amendment to Plans 0113 and 0115 at their Fable pass.
2. Space-colonisation trees (gap 5) inside Plan 0115 or as their own plan? **Their own
   plan**, after 0115. Parked in the backlog until drafted.
3. An MPL-2.0 port of the erosion filter, or a reimplementation (gap 3)? **A clean
   reimplementation from the published description is preferred; if that proves too costly,
   skip the filter** rather than port the MPL source.

## Sources

- Galin et al. 2019: <https://www.cs.purdue.edu/homes/bbenes/papers/Galin19CGF.pdf>,
  <https://diglib.eg.org/handle/10.1111/cgf13657>
- runevision 2026: <https://blog.runevision.com/2026/03/>,
  <https://80.lv/articles/fast-terrain-erosion-filter-that-emulates-erosion-without-simulation>,
  <https://docs.rs/crate/bevy_erosion_filter/latest/source/README.md>,
  <https://gist.github.com/runevision/c867c458f06284ad4c0f8aa3b974bc59>
- Minecraft 1.18: <https://minecraft.wiki/w/Tutorial:Custom_world_generation>,
  <https://minecraft.wiki/w/Terrain>
- Hytale: <https://www.hytale.com/news/2020/6/a-closer-look-at-landscape-generation-in-hytale>,
  <https://www.pcgamer.com/heres-how-hytale-creates-its-big-blocky-worlds>
- Horizon Zero Dawn: <https://www.guerrilla-games.com/read/gpu-based-procedural-placement-in-horizon-zero-dawn>,
  <https://www.gdcvault.com/play/1024120/GPU-Based-Run-Time-Procedural>
- Runions et al. 2007: <https://algorithmicbotany.org/papers/colonization.egwnp2007.html>
- Synthetic Silviculture 2019: <https://storage.googleapis.com/pirk.io/projects/synthetic_silviculture/index.html>
- Infinigen: <https://infinigen.org/>, <https://arxiv.org/pdf/2306.09310>
- Mei et al. 2007, Št'ava et al. 2008, Cordonnier et al. 2016, Weber & Penn 1995, Perlin
  2002, Prusinkiewicz & Lindenmayer 1990: find each by title in the ACM or Eurographics
  digital libraries; "The Algorithmic Beauty of Plants" is free on algorithmicbotany.org.
