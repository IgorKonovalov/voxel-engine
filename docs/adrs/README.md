# Architecture Decision Records

Numbered, append-only records of decisions that have a rejected alternative worth remembering.
Accepted ADRs are never edited in place. To change a decision, write a new ADR that supersedes
the old one and update the status here.

Rule of thumb: if you can't name an option you're *not* taking, you don't need an ADR, you need
a code comment.

**Next free number: 0005.**

An index row is a pointer, not an abstract: the link, the title as the ADR's `H1` writes it, and
the status.

| ADR  | Title | Status |
|------|-------|--------|
| [0001](0001-rust-library-workspace-wgpu-compute-ray-marching.md) | Rust library workspace, wgpu compute ray marching, streaming brick-map world | accepted |
| [0002](0002-brick-map-layout-and-per-chunk-locked-access.md) | Brick-map storage layout, GPU mirror, and per-chunk locked access | accepted |
| [0003](0003-sphere-residency-with-lod-ring-and-edit-persistence.md) | Sphere residency with an LOD-only ring, edited-chunk persistence with disk spill | proposed |
| [0004](0004-palette-narrowed-sharded-brick-pool.md) | Per-chunk 8-bit palette mirror and a brick pool sharded over fixed bindings | proposed |

## Decisions taken but not yet recorded

Settled in the founding interview (2026-10-16) and written up in
[`docs/roadmap.md`](../roadmap.md) under "Decisions from the founding interview". Each becomes
an ADR when the plan that implements it is drafted, so the ADR carries the concrete shape
rather than the intention:

- Far-field clipmap (coarser rings beyond the full-detail radius): with the far-field plan,
  after M1.
- Scripting split, WASM for server logic and Lua for client scripts: with the scripting plan.
- Dynamic entities as voxel models on rigid-part rigs: with the entities plan.
- Network transport: with the networking plan.
