# Surface detail layers: external sources

An annotated reading list on drawing fine detail (grass, fur, moss, pebbles) over a coarse
surface without storing it as geometry, collected 2026-11-16 while drafting ADR-0063 and Plan
0081. Every entry names what it describes and what we take from it, so a plan can cite it by
title. Numbers are the sources' own, on their hardware; `nfr.md` holds ours. Find each paper by
its title in the ACM Digital Library or on the authors' pages; the talk is in the GDC Vault.

## Procedural volumes over a surface, traced by rays

- **Kajiya & Kay, "Rendering Fur with Three Dimensional Textures", SIGGRAPH 1989.** A thin
  volume ("texel") of density and orientation over a surface, ray-marched; the cylinder
  shading model for a hair (diffuse `sin(t, l)`, specular on the cone of reflection) that
  every real-time fur and grass shader since has used in some form. What we take: a strand lit
  as a thin cylinder, not as a face, and the idea that the detail lives in a slab of fixed
  height over the surface. ADR-0063.
- **Perlin & Hoffert, "Hypertexture", SIGGRAPH 1989.** Procedural density functions evaluated
  inside a soft region around a surface during the ray march: fur, fire, erosion, all with no
  stored geometry, the shape a pure function of position. What we take: the layer is a
  function of the world position evaluated where the ray is, so it needs no storage, no
  upload and no update on an edit; a dug voxel loses its layer because the function's input
  changed. ADR-0063 Decision.
- **Porumbescu, Budge, Feng & Joy, "Shell Maps", SIGGRAPH 2005.** A bijection between a shell
  volume over a surface and a texture-space slab, so geometry or volume detail authored in the
  slab is ray traced over any surface. What we take: the vocabulary (a shell of height `h`
  over a face) and the observation that the shell's cost is per ray that enters it, not per
  surface element. Our shell is axis-aligned over a cube face, so the map is trivial.
- **Policarpo, Oliveira & Comba, "Real-Time Relief Mapping on Arbitrary Polygonal Surfaces",
  I3D 2005.** A per-pixel march of a height field inside the tangent slab, in the fragment
  shader, with a binary-search refinement at the crossing. What we take: the per-pixel slab
  march is cheap enough for every pixel of a surface at real-time rates on 2005 hardware, and
  its cost grows with the slab's grazing length (`h / tan e`), which is why Plan 0081 caps the
  cells a ray's run visits.

## Fur shells and the silhouette problem

- **Lengyel, Praun, Finkelstein & Hoppe, "Real-Time Fur over Arbitrary Surfaces", I3D 2001.**
  Fur as a stack of concentric alpha-textured shells drawn over the mesh, plus "fins" (quads
  along silhouette edges) because the shells fall apart where the view is grazing: between
  shells the eye sees through. What we take: a layered shell is the cheap form of the detail
  and it fails first at grazing angles and silhouettes, exactly the first-person view of
  ground at 2 to 10 m. ADR-0063, Alternative B rejects shells as the design and keeps them as
  the spike's cheap arm.

## Grass in shipped games

- **Wohllaib (Sucker Punch), "Procedural Grass in 'Ghost of Tsushima'", GDC 2021.** Blades
  generated in a compute shader per tile from hashed seeds (no artist placement), each a
  quadratic Bezier with a per-blade height, width, tilt and bend; the blade's shading normal
  is blended toward the terrain normal so a field reads as one surface under the light;
  density and blade detail fall with distance; wind comes from a global displacement texture
  the tips follow; a per-blade "root darkening" term fakes self-shadowing in the carpet. What
  we take: the blade's shape parameters, the normal blend, the root term and the LOD by
  distance. What we do not take: the rasterised instanced geometry (their renderer is a
  rasteriser; ours is a compute ray marcher with no raster pass in the frame, Alternative A).
- **Boulanger, Pattanaik & Hervé, "Rendering Grass in Real Time with Dynamic Lighting", IEEE
  Computer Graphics and Applications 29(1), 2009.** Three levels of detail by distance:
  geometry near, volumetric slices in the middle, a flat textured surface far, blended so the
  switch is not seen. What we take: the far form of grass is a tint on the surface, and the
  near form fades into it; Plan 0081's fade target is the face's own albedo.

## Distilled

- Detail over a surface is a function of position inside a slab of fixed height, evaluated
  where a ray is: no storage, no update on an edit, cost per ray that enters the slab.
- The cost grows with the slab's grazing length; cap the cells a run visits.
- Shells (alpha layers) are the cheap form and fail at grazing angles and silhouettes.
- A blade is lit as a thin cylinder with its normal blended toward the surface's, with the
  root darkened; a field then reads as one surface.
- Density and detail fall with distance to a tint of the surface; the fade is where the
  aliasing lives, so the fade's distance is set by the element's size on screen.
