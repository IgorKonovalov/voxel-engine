# ADR-0038: The contour plane code holds a zero normal component exactly

> **Status:** proposed
> **Date:** 2026-10-29
> **Related plan(s):** [Plan 0050](../plans/0050-watertight-contour-surface-crack-spike-and-closing-rule.md)
> (Phase 2). Amends [ADR-0011](0011-contour-planes-in-attribute-pages-with-procedural-detail.md)'s
> attribute encoding in its normal's scale only.

## Context

ADR-0011 codes a surface voxel's plane in a `u16`: a 6 + 6-bit octahedral normal and a 4-bit
offset. The encoder (`attrib.wgsl`, `encode_octahedral`) maps each octahedral coordinate
`p` in `[-1, 1]` to `round((p / 2 + 1/2) * 63)`, and the decoder (`march.wgsl`,
`decode_octahedral`) maps a code `q` back to `q / 63 * 2 - 1`. On that scale `p = 0` falls at
`q = 31.5`, between two codes, so no decoded normal has a zero component: a floor's plane
leans about 0.9 degrees, and the planes of two neighbouring surface voxels that see the same
neighbourhood are parallel but step by about 1/63 of a voxel per voxel across the lean.

Plan 0050's Phase 1 measured what that step does to the walk. A 45-degree slab
(`normalize(1, 0, 1)`, every surface voxel with the same 27-neighbourhood) reads 17 pixels
(0.035 % of hits) in crack class 2 on lavapipe, every one on a face between y-neighbours, and a
`normalize(1, 1, 1)` slab, whose `p = (1/3, 1/3)` is code 42 exactly, reads 0. The steps are
real cracks of the contour surface, too small to see on their own, and they sit on the most
common surfaces a cave has: floors, walls and slopes in an axis plane.

## Decision

We put the octahedral coordinates on codes `0..=62`: the encoder writes
`round((p + 1) * 31)` clamped to 62, and the decoder reads `q / 31 - 1`. Then `p = 0` and
`p = +-1` are exact, so every axis normal and every zero component decodes exactly, and the
surface voxels of a floor or a wall, which share a neighbourhood, share one plane. The
attribute stays 16 bits in the same pages (Plan 0028's addressing), the offset code is
unchanged, and code 63 is unused.

## Consequences

### Positive
- A flat floor, a vertical wall and any slope whose normal lies in an axis plane lose the
  lean-induced step across that axis: one plane for a shared neighbourhood.
- The change is two constants and the CPU decoder of the mirror tests. No storage, page or
  pass changes, and no frame cost.

### Negative
- The step between codes grows from 2/63 to 2/62 of the octahedral square, about 1.6 % coarser.
- Directions exact today become inexact: `p = 1/3` (the `(1, 1, 1)` family) lands at
  `q = 41.33`, about 1 % off, and `p = 1/2` (the 45-degree family in an axis plane,
  `normalize(1, 0, 1)`) at `q = 46.5`, so those slabs keep a small step along their diagonal.
  No 6-bit uniform scale holds both 0 and 1/2 and 1/3 exactly.
- Every contour golden moves and is re-blessed (Plan 0050's owner answer 2); the face goldens do
  not.

## Alternatives considered

### Alternative A: keep codes 0..=63 and snap the two codes beside zero to 0 on decode

The decoder alone changes, and the `(1, 1, 1)` family stays exact. It lost because it widens the
gap around every axis to about 1.8 degrees and flattens any normal within 0.9 degrees of an axis
plane onto it, so near-flat surfaces facet against the axis instead of leaning, and the encoder
and decoder no longer agree on what a code means.

### Alternative B: leave the code and let the crack-closing rule absorb the steps

Plan 0050's closing candidates (snap, extend, blend) close steps of 0.02 voxel along with the
larger cracks, so the walk would hide them. It lost because the owner wants the surface itself
to be one plane where the rock is flat (2026-10-29): a closing rule that patches a step it
created by rounding hides the cause instead of fixing it.
