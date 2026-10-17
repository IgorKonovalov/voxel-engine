# ADR-0012: A brick is occupancy masks plus a per-brick palette in bit planes, the same bytes on the CPU and in the mirror

> **Status:** accepted (2026-10-17)
> **Date:** 2026-10-17
> **Related plan(s):** [Plan 0015](../plans/0015-traversal-hierarchy-chunk-bits-brick-masks-and-translucent-class.md)
> (the traversal hierarchy on today's pool), then a CPU-format plan and a mirror-slot plan,
> drafted when Plan 0015 is reviewed.
> **Supersedes in part, once accepted:** ADR-0002's CPU brick clause (`Full` as
> `Box<[u16; 512]>`) and ADR-0004's mirror layout (512 one-byte indices into a per-chunk
> palette, 8-byte masks at 2³). ADR-0002's chunk, locking and access model, ADR-0003's
> residency and ADR-0004's sharding stand. Also settles the contour footprint that ADR-0011
> and Plan 0013 Phase 4 owe: the attribute is stored for surface voxels only, addressed by
> popcount of a surface mask.

## Context

The owner reframed the open edit-cost question (`docs/decisions-pending.md`, D3) on
2026-10-17: the decision that matters is whether the renderer holds a stable 60 fps on
complex scenes (wide view, many materials, translucent voxels, several dynamic lights) on
the floor machine, and 120 fps where the hardware allows. Four facts shape it.

**Frame time is bandwidth, and today's walk spends five dependent reads per step.** The
marcher (`march.wgsl`) reads the chunk table, the chunk record entry, the 2³ mask word, the
pool byte and the palette pair, one after another, at every DDA step. There is no chunk-level
skip: a chunk of uniform-air bricks costs eight record reads to cross. The streamed cave's
spawn view reads 6.69 ms at 1080p on the dev box for primary rays plus one shadow ray
(`nfr.md` §2, a recorded miss against 3.0 ms) where the small level reads 1.59 ms; the
difference is steps through air, not hits. At 1080p and 60 fps with primary, shadow and AO
rays, the dev box affords about 20 uncached 64-byte lines per ray, a GTX 1660 about 8, a
Switch-2-class portable about 3 (arithmetic from the published bandwidths; see
`docs/references/voxel-optimizations.md`). Every production and research marcher that runs
at that rate spends one line per hierarchy level per ray segment: a coarse bit grid, an 8³
brick whose occupancy bits fit one line, and the material read only at the hit (Teardown's
1-bit volume and mips, GVDB's 8³ leaves, the 64-tree write-ups, VoxelRT's masked brick map).

**Memory is the binding constraint for the scale the owner wants.** Resident chunks hold
3.24 GB of the 4 GB RAM line at a 154 m ring (peak RSS 3.87 GB), and the mirror with contour
attributes reads 4.10 GB allocated against the 3 GB VRAM line and ran the 8 GiB dev card out
of memory (`nfr.md` §3). A `Full` brick costs 1 KiB in RAM and 512 + 8 B in the mirror, plus
1040 B of contour attribute; the testbed's bricks hold two to four materials. Kilometre-scale
scenes and a Switch-2-class portable profile (about 9 GB shared between CPU and GPU) both
need the brick several times smaller. Editable worlds in the literature stay raw-plus-masks:
compressed DAGs edit in milliseconds (HashDAG) but re-upload in tens of milliseconds, and
distance fields lose on bandwidth (VoxelRT). The lever that remains is a per-brick palette
at one to four bits per voxel, three to eight times smaller than a byte.

**Translucent materials are in scope; gas is not a voxel.** The owner wants glass, water and
lava as voxels in the brick map, and mist, smoke and fire as a separate coarse volume
(2026-10-17). A primary ray must pass through a translucent voxel as it passes through air
and stop at opaque rock, so a brick's occupancy has to be split by opacity class before the
walk reads any material.

**The edit cost is bounded by the format.** Plan 0014 closed at 3.04 ms for the warm worst
case against 2 ms, with 1.66 ms in the per-voxel row write of `[u16; 512]`; the arithmetic in
`decisions-pending.md` D3a puts a word-operation kernel on masks and planes at 0.2 to 0.5 ms
for that share, a guess until the spike measures it.

## Decision

**A `Full` brick is a 512-bit occupancy mask (one bit per voxel, one `u64` per z-layer), a
palette of its non-air materials in first-appearance order, and `ceil(log2(n))` bit planes
of 512 bits that index the palette; air is only ever the clear mask bit.** A brick with more
than 256 non-air materials stores sixteen planes holding the material ids directly and no
palette. `Uniform` bricks stay a single material id. The encoding is canonical: a palette
entry with no voxel is removed when the edit that emptied it finishes, so equal voxels have
equal bytes and the derived equality of `Brick` is exact. World hashes and the op log read
decoded voxels and are unchanged by this decision.

**The mirror holds the same masks and planes, split by opacity class.** A brick's mirror slot
holds its opaque mask, its translucent mask when it has translucent voxels (a material flag;
the split is derived at upload from the palette), its planes and its palette, in a size
class of 64-byte units named by the chunk record entry. Per-chunk palettes go. The contour
attribute of ADR-0011 is stored for the brick's surface voxels only, at the popcount index
of a surface mask. The marcher walks a hierarchy of bits, one read per level per ray
segment: a 512-bit brick-occupancy word per chunk record, the brick's opaque mask, then the
planes and palette at the hit. A second, bounded walk accumulates up to `N` translucent
layers before the opaque hit; `N` is a quality-profile setting. Mip-bricks use the same
format.

**The work lands in three plans, hierarchy first.** Plan 0015 builds the chunk bits, the
512-bit opaque masks and the translucent class on today's 512-byte pool, because the frame
time and the long-corridor miss do not wait for the CPU format. The CPU format plan follows
with a census of materials per brick and a kernel bench as its first phase, then the
format, the edit kernel and every reader. The mirror-slot plan replaces the 512-byte slot by
size classes and moves the contour attribute into them.

## Consequences

### Positive

- **One read per level.** A ray segment through an empty chunk costs one table read and one
  chunk-bits read; through a brick, one mask line; the pool is read at the hit only. That is
  the shape every marcher at the target rate has, and it holds on the portable profile's
  three lines per ray.
- **Memory.** A two-material brick is about 70 B in RAM (mask, one entry, header) against
  1056 B; three or four materials about 130 to 200 B. At the testbed census the resident
  bricks fall from about 3.2 GB to under 1 GB (a guess from D3a's arithmetic, settled by the
  census phase). The mirror slot with the contour attribute falls from 1552 B to a size
  class of a few hundred bytes for a surface brick. Spill files and any later chunk
  snapshot shrink about tenfold before compression.
- **Edits become word operations**: a carve is `solid &= !(shape & carvable)` over eight
  words per plane with popcount tallies; a replace that renames a palette entry is O(1); a
  collapse drop by whole voxels shifts z-layer words.
- **Translucency costs nothing where it is absent**: a brick without translucent voxels has
  no translucent mask, and the opaque walk never branches on it.
- **The contour footprint question (Plan 0013 Phase 4) has its answer**: surface-only
  storage, lossless, which D1's table projected at 1.56 GB live at a 14-chunk radius and
  1.82 GB at 16 before this format shrinks the rest.

### Negative

- **Every brick reader changes.** `Brick` is public and `vxe-sim` matches on it; the type
  becomes a view with `get`, `set` and `decode`, a minor-version break with a consumer note.
  Cold readers (mips, collapse, stamp) go through a decode-to-scratch adapter first, at about
  0.5 to 2 µs per brick (a guess).
- **A per-voxel read is slower in isolation**: a mask bit, then one bit from each of up to
  four planes, then a palette entry, against one array index today. Hot readers (raycast,
  coarse grid, queries) get native mask paths, and the random-access cost is measured in the
  format plan's close before it is accepted.
- **Two encodings of the same world are possible without the canonical rule.** The rule
  costs a popcount per palette entry at the end of each brick edit, and a test holds it.
- **The mirror needs a size-classed allocator** (64-byte units, classes by brick size), which
  ADR-0004 deferred as its Alternative A. A brick that grows a class moves slots, and the
  record entry is rewritten in the same sync.
- **Three plans before the full benefit lands.** Plan 0015 alone leaves RAM and the contour
  VRAM where they are.
- **A material that changes opacity class at runtime re-uploads every brick holding it.**
  The class is a static property of the material row; the engine refuses to change it after
  the table is sealed, and the consumer models "frozen water" as a second material row, as
  ADR-0002 does for every per-voxel variant.

## Alternatives considered

### Alternative A: the hierarchy only, both formats unchanged

Add the chunk bits and the 512-bit masks to the mirror, keep `[u16; 512]` on the CPU with a
SIMD row kernel, keep the 512-byte slot. It gets the same frame-time win for one plan and no
API break. It lost because it leaves the two recorded memory misses in place: resident RAM
at 3.87 GB of 4 and the contour mirror at 4.10 GB of 3, which kilometre scale and the
portable profile cannot absorb. Plan 0015 is this alternative's content, taken as the first
step rather than the last.

### Alternative B: raw CPU bricks, a surface-only mirror

Keep the CPU format and upload only a 1-bit mip chain plus the materials and attributes of
surface voxels. It reaches the smallest mirror of the three. It lost because the RAM line
stays where it is, the upload does the most work per dirty brick (surface extraction and
packing for every edited brick and its neighbours), and the CPU and GPU formats diverge
completely, so neither the masks nor the planes are shared.

### Alternative C: a compressed tree for the near field

An SVO, SVDAG or HashDAG at the chunk level. Rejected in ADR-0002 for the editable near
field, and the reasons hold: edits need path rewrites or hashing machinery, and the GPU
re-upload after an edit is the cost that matters for a dirty-set mirror. A DAG stays a
candidate for the far-field clipmap, where nothing is edited.

### Alternative D: a per-brick distance field for skipping

Store a distance to the nearest solid voxel per voxel or per brick instead of occupancy
bits. Fewer steps per ray, but every step reads a byte or more, the field changes for every
voxel within its radius on each edit, and VoxelRT's comparison shows it losing to the masked
brick map on bandwidth. Rejected.
