# Plans index

The one-minute "what's in flight" view. Read this first each session instead of re-deriving
state from `git log`. Completed plans move to `done/`.

**Next free number: 0007** (ADRs are a separate sequence; next free there is **0007**).

## Active roster

| Plan | Title | Status | Live constraint |
|------|-------|--------|-----------------|
| [0003](0003-edit-cost-span-fills-and-per-op-tables.md) | Edit cost under 2 ms: row spans, a per-op target table and per-brick tallies | approved | Closes the `nfr.md` §4 miss (4.92 ms) without moving a result or a hash; independent of Plan 0002. No ADR. |
| [0004](0004-cylinder-sweeps-character-controller-and-coarse-grid.md) | Cylinder sweeps, a character controller, and a per-brick coarse grid with change notifications | approved | M3, new crate `vxe-sim`; independent of Plan 0002 except the grid's streaming hooks, wired later. Paired with ADR-0005. |
| [0005](0005-surface-look-spike.md) | The surface-look spike: smoothed normals, per-voxel contour planes and procedural detail, measured on screen | approved | First M2 plan; unblocked by Plan 0002's close (both rewrite `march.wgsl` and `gpu_world.rs`), with drift against it to settle before "go". Ends in the owner's on-screen call; the surface ADR is written at its close. |
| [0006](0006-spatial-audio-hrtf-mixer-and-occlusion.md) | Spatial audio: a headless HRTF mixer on the device callback, occlusion and early reflections through the material table | approved | M4, new crate `vxe-audio`; independent of Plans 0002 to 0005. Paired with ADR-0006. |

## Recently closed

| Plan | Title | Closed |
|------|-------|--------|
| [0002](done/0002-streaming-residency-lod-ring-and-mirror-narrowing.md) | Streaming residency, an LOD-only ring, and a narrowed, sharded mirror | 2026-10-16, v0.3.0 |
| [0001](done/0001-bounded-brick-map-edits-and-headlamp-marcher.md) | Bounded brick map, deterministic edits, and a headlamp ray marcher | 2026-10-16, v0.2.0 |

## Conventions

- One plan per milestone slice, numbered when drafted, never reserved ahead.
- A row carries at most two sentences of live constraint. What landed and why belongs in the
  plan file.
- A closed plan leaves this table; `done/` records it.
