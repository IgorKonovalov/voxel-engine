# Voxel engine optimizations: external sources

An annotated reading list of the techniques production voxel ray tracers and the literature use
to keep memory and frame time down, collected 2026-10-16 while reviewing `roadmap.md`. Every
entry names what it describes and the numbers it claims, so a plan can cite it by title. Numbers
are the sources' own, measured on their hardware, not ours; `nfr.md` holds our figures.

Each entry ends with the milestone it informs. The distilled list is at the end.

## Storage and traversal

- **Amanatides & Woo, "A Fast Voxel Traversal Algorithm for Ray Tracing", Eurographics 1987.**
  <https://www.eecs.yorku.ca/~amana/research/grid.pdf>. The 3D-DDA every voxel marcher is built
  on: stepping to the next cell costs two comparisons and one addition. M1. Our walks
  (`vxe-world` raycast, `march.wgsl`) skip empty space in cells of several aligned sizes, so
  they recompute each step's exit from the ray origin instead of accumulating `tMax += tDelta`:
  the incremental form assumes one cell size, and recomputing keeps the CPU walk exact.
- **Laine & Karras, "Efficient Sparse Voxel Octrees", I3D 2010.**
  <https://www.nvidia.com/docs/IO/88889/laine2010i3d_paper.pdf>, extended tech report
  <https://research.nvidia.com/sites/default/files/publications/laine2010tr1_paper.pdf>. The
  reference GPU octree ray caster: compact child descriptors, contour data for sub-voxel
  geometry, a normal compression format. The structure is static; editing it is the problem the
  brick-map camp avoids. M1 (as the rejected alternative to a brick map). Section 4.2, beam
  optimization: a coarse pass casts one conservative distance ray per 4×4 or 8×8 pixel block
  (contours off, stopping at any voxel too small to cover a whole ray of the coarse grid), and
  each pixel's ray starts at its block's minimum distance less a margin. Their Table 2 reads 3
  to 27 % more primary rays per second from it across scenes and resolutions: a trim on top of
  a hierarchy, not a replacement for one. M2 (a Plan 0015 followup). Sections 3.2 to 3.4,
  contours: the one published precedent for a per-voxel plane clipping a voxel's cube
  (ADR-0011). A contour is a pair of parallel planes in 32 bits (three 6-bit normal
  components, two 7-bit offsets), and a voxel's final shape is its cube intersected with its
  own contour and every ancestor's, which is how they approximate sharp corners. Two facts
  separate their scheme from ours. The planes are fit top-down from the **original triangle
  surface** inside the voxel, greedily along the direction the cube most overestimates it, and
  placed so the surface is fully enclosed; contours are omitted where the ancestors' shape
  already fits. And they give up smoothness on purpose: "as long as we ensure that the
  original surface is fully enclosed by each contour, we are guaranteed to get an
  approximation that contains no holes", while "discontinuities at voxel boundaries may
  introduce problems such as false self-shadowing or interreflections", worked around by
  offsetting secondary rays. That is the crack and shadow-origin story of ADR-0011 and
  ADR-0039 with enclosure as the cure instead of snap. A fit from binary occupancy has no
  surface to enclose, so a one-voxel feature takes its surroundings' plane; the planes of a
  feature's neighbours cannot tighten it the way enclosing ancestors can (ADR-0040,
  Alternative A). Contours are off in the beam pass. Their purpose was silhouette error and
  memory on undersampled meshes, not a smooth look. M2 (ADR-0011, ADR-0039, ADR-0040).
- **Marmitt, Kleer, Wald, Friedrich, Slusallek, "Fast and Accurate Ray-Voxel Intersection
  Techniques for Iso-Surface Ray Tracing", VMV 2004** (not re-verified today; the
  bibliographic record is from memory), with **Parker et al., "Interactive Ray Tracing for
  Isosurface Rendering", IEEE Visualization 1998** as the first GPU-free form. The ray-marched
  isosurface of a trilinear field: inside one cell the field along the ray is a cubic in `t`,
  and Marmitt isolates its roots by the cubic's extrema and refines the first sign change with
  a few Newton or repeated-linear steps, which is both cheaper and more exact than Parker's
  closed-form cubic. The trilinear zero set is continuous across cells, so the surface has no
  cracks; its analytic gradient is not, so normals need a smoother source where the tiling
  shows. The whole smooth-voxel family (marching cubes, dual contouring, surface nets, SDF
  marchers such as Claybook and Dreams) renders a field this way, meshed or marched. M2
  (Plan 0054, ADR-0040 Alternative B).
- **Kämpe, Sintorn, Assarsson, "High Resolution Sparse Voxel DAGs", SIGGRAPH 2013.**
  <https://www.cse.chalmers.se/~uffe/HighResolutionSparseVoxelDAGs.pdf>. Merging identical
  subtrees turns an SVO into a DAG with one to three orders of magnitude fewer nodes, traversed
  without decompression. Binary occupancy only. A candidate for static far-field LOD, not for
  the editable near field. M1 LOD, later.
- **Dado et al., "Geometry and Attribute Compression for Voxel Scenes", Eurographics 2016.**
  <https://graphics.tudelft.nl/Publications-new/2016/DKBTE16>. Decouples the DAG topology from
  attributes and palette-compresses the attributes; 128K³ scenes in core on commodity hardware.
  The palette idea applies to our material ids independently of the DAG. M1.
- **Careil, Billeter, Eisemann, "Interactively Modifying Compressed Sparse Voxel
  Representations", Eurographics 2020.** <https://graphics.tudelft.nl/Publications-new/2020/CBE20>.
  Carving, filling and painting a compressed DAG without decompressing it. The existence proof
  that compressed far-field storage can still take edits, if we ever need that. Later.
- **van Wingerden, "Real-time Ray tracing and Editing of Large Voxel Scenes", MSc thesis,
  Utrecht 2015.** <https://hgpu.org/?p=14319> (repository page
  <https://studenttheses.uu.nl/handle/20.500.12932/20460>). The brick-map argument in full: keep
  raw voxels in layered grids so there is no preprocessing to redo on edit; empty space is
  skipped at the coarse layer; occlusion-guided streaming from disk; "potentially infinite
  landscapes in finite memory". OpenCL on a Radeon HD 7970. M1.
- **Crassin et al., "GigaVoxels: Ray-Guided Streaming for Efficient and Detailed Voxel
  Rendering", I3D 2009.** <https://artis.inrialpes.fr/Publications/2009/CNLE09>. Bricks hung off
  an octree, with the rays themselves reporting which bricks are missing so the streamer loads
  what is visible rather than what is near. Several billion voxels at 20 to 90 fps on 2009
  hardware. M1 streaming (visibility-driven paging as an alternative to radius-driven).
- **Hoetzlein, "GVDB: Raytracing Sparse Voxel Database Structures on the GPU", HPG 2016**
  <https://ramakarl.com/?p=2639>, and **Museth, "NanoVDB", 2021**
  <https://research.nvidia.com/labs/prl/nanovdb/nanovdb2021.pdf>. GPU versions of the VDB
  hierarchy of grids; GVDB's indexed memory pooling for dynamic topology is the closest
  published relative of a paged brick pool. Neither targets `wgpu`; the ideas port, the code does
  not. M1.
- **Liu & Rokne, "A micro 64-tree structure for accelerating ray tracing on a GPU", Graphics
  Interface 2013.** <https://graphicsinterface.org/proceedings/gi2013/gi2013-22>. Each node's
  4×4×4 children occupancy in one 64-bit word; upper levels preloaded into shared memory.
  Fewer traversal steps and less bandwidth than a flat grid for modest build cost. M1.
- **Vorxel devlog, "Tetahexacontree 4x4x4 Occupancy Bitmasks Lookup Table", 2024.**
  <https://teknologicus.itch.io/vorxel/devlog/839586/tetahexacontree-4x4x4-occupancy-bitmasks-lookup-table>.
  A hobby engine's account of the same 64-bit-mask skip, with a lookup table of "which bits
  matter for this ray octant" and the finding that a float-in-[1,2) coordinate trick was only
  slightly faster and caused seam artifacts. M1.
- **dubiousconst282, "VoxelRT: voxel rendering experiments", GitHub.**
  <https://github.com/dubiousconst282/VoxelRT>. Benchmarks nine traversal schemes on the same
  scenes: flat DDA, two-level DDA with 8³ bricks, a three-level brick map with occupancy masks,
  distance fields (Manhattan, Euclidean, per-octant), ESVO, a 4³-branching "Tree64", and a BVH.
  Primary rays: flat DDA 23 to 33 Mrays/s, the leaders 100 to 215 Mrays/s. Distance fields lose
  on bandwidth despite fewer steps; the masked brick map and Tree64 have the best
  speed-to-simplicity ratio; trees suit LOD better than grids. The single most useful comparison
  for Plan 0001's layout ADR. M1.
- **Bikker, "Ray tracing with voxels in C++", blog series, 2024.**
  <https://jacco.ompf2.com/2024/04/24/ray-tracing-with-voxels-in-c-series-part-1/>, code at
  <https://github.com/jbikker/voxpopuli>. A CPU teaching series that still carries the plan for
  acceleration: flat grid, then multi-level grid, then a TLAS of voxel objects; blue noise,
  reprojection and path tracing in between. M1, M6 (TLAS of voxel entities).
- **Majercik, Crassin, Shirley, McGuire, "A Ray-Box Intersection Algorithm and Efficient
  Dynamic Voxel Rendering", JCGT 2018.** <https://jcgt.org/published/0007/03/04/paper.pdf>.
  Ray versus oriented box 2 to 10 times faster than prior GPU methods; the primitive for tracing
  transformed voxel models inside the world trace. M6.
- **Aila & Laine, "Understanding the Efficiency of Ray Traversal on GPUs", HPG 2009.**
  <https://research.nvidia.com/sites/default/files/pubs/2009-08_Understanding-the-Efficiency/aila2009hpg_paper.pdf>.
  Naive kernels sit 1.5 to 2.5 times below the hardware optimum because of work distribution;
  persistent threads pulling from a global pool recover 80 to 90 percent of it. The
  divergence argument applies verbatim to compute DDA. M1, M2.
- **Laine, Karras, Aila, "Megakernels Considered Harmful: Wavefront Path Tracing on GPUs",
  HPG 2013.** <https://research.nvidia.com/sites/default/files/publications/laine2013hpg_paper.pdf>.
  One kernel that marches, shades and spawns the next ray pays for the register count of its
  heaviest stage at every stage and diverges at every branch; a wavefront of lean kernels (a
  ray-cast kernel with few registers, a shading kernel, a queue between them) runs the cast at
  full occupancy and the shading in coherent batches. The argument for splitting
  `march.wgsl`'s primary march from its shading and shadow ray before the lighting plan adds
  more stages to the same loop. M2 (the lighting plan).
- **Xu et al., "Aokana: A GPU-Driven Voxel Rendering Framework for Open World Games", 2025.**
  <https://arxiv.org/abs/2505.02017>. SVDAG storage plus an LOD mechanism and a streaming
  system for tens of billions of voxels; claims up to 9 times less memory and 4.8 times faster
  rendering than the prior state of the art. The most recent treatment of LOD plus streaming at
  the scale the roadmap's "LOD to 1 km" implies. M1 LOD, later.
- **Minecraft chunk format, "Paletted containers".** <https://wiki.vg/Chunk_Format>. Each 16³
  section keeps its own palette and packs indices at the minimum bit width (4 bits or more). The
  cheapest known way to keep a 16-bit material space while storing most bricks at 4 or 8 bits.
  M1.

## Production reference: Teardown

- **Wittens, "Teardown Frame Teardown", acko.net, 2021.**
  <https://acko.net/blog/teardown-frame-teardown/>. A frame capture of Teardown: one byte per
  voxel indexing a 256-entry palette; objects as 3D textures padded to multiples of 4 with two
  extra mip levels so the marcher skips 2³ and 4³ empty blocks; a 1-bit-per-2×2×2 volumetric
  shadow map of the whole level (about 262 MB for the largest map) traced with a rough cone for
  shadows and reflections; one jittered shadow ray per light per pixel; two AO samples; a
  depth-and-normal-weighted spiral blur plus temporal reprojection applied four times a frame;
  volumetrics at half resolution; blue-noise sampling throughout. M1, M2.
- **Gustafsson, "From screen space to voxel space", voxagon blog, 2018.**
  <https://blog.voxagon.se/2018/10/17/from-screen-space-to-voxel-space.html>. The author's
  own account: a 100×100×25 m volume at 5 cm, bit-packed eight voxels per byte, with mips at 10,
  20 and 40 cm, about 292 MB; coarse-to-fine traversal across the mips; supercover stepping
  where leaks matter; soft shadows by jittering the light position. About 9 ms at 1080p for AO,
  ten lights, fog and reflections on a GTX 1080. M2.
- **Gustafsson, "Year summary", voxagon blog, 2024.**
  <https://blog.voxagon.se/2024/12/29/year-summary.html>. The next engine replaces Teardown's
  dense grids with 8×8×8 chunks tracked by a 3D bitmap ("updates are fast"), and moves to
  hardware ray tracing with intersection shaders. The storage choice matches our brick map; the
  RT choice is the path ADR-0001 rejected for the baseline GPU. M1.
- **Gustafsson, "The unlikely story of Teardown Multiplayer", voxagon blog, 2026.**
  <https://blog.voxagon.se/2026/03/13/teardown-multiplayer.html>. Destruction replicates as
  deterministic commands ("cut this shape at x,y,z") on a reliable channel, physics as state on
  an unreliable one, about 1 Mbit per client. Confirms edits-as-operations; warns that floats are
  "mostly safe if you know how to navigate around the pitfalls". M5, M9.

## Lighting, denoising, upscaling

- **Schied et al., "Spatiotemporal Variance-Guided Filtering", HPG 2017.**
  <https://cg.ivd.kit.edu/publications/2017/svgf/svgf_preprint.pdf>. The 1 spp reconstruction
  baseline: temporal accumulation, variance-driven à-trous wavelet filter; 10 ms at 1080p on
  2017 hardware. The roadmap's "SVGF-family" starting point. M2.
- **Schied, Peters, Dachsbacher, "Gradient Estimation for Real-Time Adaptive Temporal
  Filtering" (A-SVGF), HPG 2018.** <https://cg.ivd.kit.edu/atf.php>. Re-shades a sparse set of
  last frame's samples with the same random sequence to detect lighting change and drop stale
  history per pixel; 2 ms at 1080p. The published answer to "the headlamp moved, the crystal
  lit up, the wall was dug out" without ghosting. M2.
- **NVIDIA, "Real-Time Path Tracing and Denoising in Quake II RTX", GTC 2019.**
  <https://developer.download.nvidia.com/video/gputechconf/gtc/2019/presentation/s91046-real-time-path-tracing-and-denoising-in-quake-2.pdf>,
  overview <https://developer.nvidia.com/blog/path-tracing-quake-ii>. A shipped 1 spp path
  tracer with A-SVGF, temporal AA and light-list sampling; the closest analogue to our M2
  pipeline, measured on RTX hardware. M2.
- **NVIDIA Real-Time Denoisers (NRD).** <https://developer.nvidia.com/nrd>, source at
  <https://github.com/NVIDIA-RTX/NRD-Sample>. ReBLUR (recurrent blur), ReLAX (SVGF-derived, for
  ReSTIR signals) and SIGMA (shadows), MIT, API-agnostic, designed for 0.5 to 1 rays per pixel.
  Not WGSL and not portable as a binary into `wgpu`, but the reference design to read before
  writing ours. M2.
- **Bitterli et al., "Spatiotemporal reservoir resampling for real-time ray tracing with
  dynamic direct lighting" (ReSTIR), SIGGRAPH 2020.**
  <https://cs.dartmouth.edu/~wjarosz/publications/bitterli20spatiotemporal.html>. Reservoir
  resampling across pixels and frames picks good light samples among millions of emitters with
  no light data structure; 6 to 60 times faster to equal error. Directly answers "a crystal vein
  is thousands of emissive voxels". M2.
- **Ouyang et al., "ReSTIR GI: Path Resampling for Real-Time Path Tracing", HPG 2021.**
  <https://research.nvidia.com/publication/2021-06_restir-gi-path-resampling-real-time-path-tracing>.
  The same resampling applied to indirect paths; 9 to 166 times lower MSE at 1 spp before the
  denoiser. M2, if one bounce is not enough in the dark.
- **Crassin et al., "Interactive Indirect Illumination Using Voxel Cone Tracing", Pacific
  Graphics 2011.** <https://research.nvidia.com/labs/rtr/publication/crassin2011givoxels/>.
  Cone marching through a mip pyramid for two bounces at 25 to 70 fps on 2011 hardware. We
  already own the pyramid; a cheap fallback GI if 1 spp path tracing misses the baseline budget.
  M2 alternative.
- **Majercik et al., "Dynamic Diffuse Global Illumination with Ray-Traced Irradiance Fields"
  (DDGI), JCGT 2019.** <https://research.nvidia.com/publication/2019-05_dynamic-diffuse-global-illumination-ray-traced-irradiance-fields>.
  Probe grid updated by a few rays per probe per frame, with a visibility-aware interpolant that
  stops light leaks. The other cheap-GI fallback; probes per chunk fit the streaming unit. M2
  alternative.
- **Sannikov, "Radiance Cascades", 2023; Chalmers theses 2025; "Split Radiance Cascades",
  arXiv 2026.** <https://arxiv.org/abs/2607.20384>,
  <https://odr.chalmers.se/items/3ee9fb4e-1880-46c2-802a-a660e38dc9ee>. Overlapping probe
  cascades with exponentially growing spacing and ray length; cost independent of the number
  of lights; weak on small penumbrae. Worth a look for the far field. M2 alternative.
- **frozein, "DoonEngine", GitHub.** <https://github.com/frozein/DoonEngine>. Lighting
  computed per voxel rather than per pixel and accumulated over frames, with direct and specular
  from a fixed ray count so edits and camera moves update instantly; 8³ chunks so an edit
  re-uploads one chunk. A hobby project, but the per-voxel irradiance cache is the simplest
  thing that could make dark caves stable. M2 alternative.
- **AMD FidelityFX Super Resolution 2.** <https://gpuopen.com/fsr2/>. MIT, HLSL plus C++,
  temporal upscaling with no ML hardware; AMD's upper bound at 1080p quality mode is under
  0.6 ms on an RX 5700 XT, the card nearest our baseline. Superseded by FSR 3 and 4 in AMD's
  SDK but the source stays. No WGSL port was found on 2026-10-16; porting is a plan-sized job,
  and the roadmap's "open-source upscaler ported to WGSL" should budget for it or fall back to
  a TAA upscaler of our own. M2.

## Determinism

- **Rust users forum, "F64::ln_1p platform-specific behavior"**
  <https://users.rust-lang.org/t/f64-ln-1p-platform-specific-behavior/77113>; **Rapier
  determinism guide** <https://rapier.rs/docs/user_guides/rust/determinism/>; **`libm` crate**
  (pure-Rust transcendentals). IEEE add, subtract, multiply, divide and sqrt are bit-exact
  across our targets; `sin`, `exp`, `ln`, `powf` come from the platform C library and differ in
  the last bit between glibc and MSVC. A noise generator that calls them is not a pure function
  of its seed across the two CI arms. M1 (the fly-through's source and the hash test).

## Main optimization points

The techniques that recur across the sources above, in the order they matter to this engine.

1. **Skip empty space with bit masks, not voxel reads.** Every fast marcher (Teardown's next
   engine, VoxelRT's masked brick map and Tree64, the micro 64-tree, Vorxel) keeps a 64-bit
   occupancy word per 4×4×4 or 8×8×8 block and walks those words before touching voxels. A
   uniform brick is one word and one compare.
2. **Memory bandwidth is the limit, not arithmetic.** VoxelRT's distance fields take fewer
   steps and still lose; Aila and Laine's divergence findings apply; Teardown's 1-bit shadow
   volume and 8-bit palette exist to shrink reads. Store the GPU mirror at the smallest width
   that works (palette indices, occupancy bits) and keep `u16` materials in the CPU truth.
3. **Keep the editable near field raw.** Brick maps and layered grids (Wingerden, Teardown,
   DoonEngine) take edits in place; octrees and DAGs (ESVO, SVDAG, Aokana) win on memory and
   far-field LOD but need rebuilding or special machinery (Careil) to edit. Use the compressed
   forms, if at all, for far chunks only.
4. **A mip pyramid pays for itself three times.** Coarse-to-fine traversal (Voxagon 2018,
   Teardown's two extra mips), LOD for distant rays, and cone or probe GI (Crassin, DDGI)
   all read the same pyramid. Store it as occupancy bits or palette indices, not full-width
   material ids.
5. **Stream by visibility, re-upload by brick.** GigaVoxels and Wingerden let the rays request
   bricks; DoonEngine and Teardown re-upload only the touched block. A radius is a fine first
   policy, but the upload path should already be per brick.
6. **Few lights: one jittered shadow ray per light, temporal blend, blue noise.** Teardown ships
   on that. Many emissive voxels: ReSTIR reservoirs rather than a light list.
7. **Denoise with history that knows when to die.** SVGF is the base; A-SVGF's gradient test is
   what survives a headlamp sweep and a dug-out wall; NRD's ReLAX is the production reference.
   The roadmap's "invalidate history in the touched region" is a cheaper, edit-specific version
   of the same idea and should be measured against A-SVGF.
8. **Have a cheaper GI on the shelf.** Voxel cone tracing, DDGI probes, per-voxel irradiance
   (DoonEngine) and radiance cascades all run under a 1 spp path tracer's cost on a card
   without RT hardware. M2 should name its fallback before it starts.
9. **Upscaling is a solved, licensed problem but not a free one in WGSL.** FSR 2's cost is well
   under a millisecond at 1080p on a 5700 XT; the port is the work.
10. **Edits as commands is proven at scale.** Teardown's multiplayer sends shapes, not voxels,
    at about 1 Mbit per client. The one trap the sources flag is float transcendentals across
    platforms; keep edit math integer and audit the generator for `sin` and friends.
11. **Keep the march kernel lean, and march secondary rays coarsely.** The wavefront paper's
    register argument says the hit pass should carry no shading; Teardown's 1-bit 2×2×2
    shadow volume says shadow and occlusion rays need no voxel-level reads. With the 2³ bits
    on the brick's mask line (Plan 0015), a coarse walk costs no storage.
12. **Start primary rays late only after the hierarchy is in.** ESVO's beam optimization is
    worth 3 to 27 % on top of a hierarchy; a plan that reaches for it before the bit levels
    exist is treating a trim as a rescue.
