# Plans index

The one-minute "what's in flight" view. Read this first each session instead of re-deriving
state from `git log`. Completed plans move to `done/`.

**Next free number: 0015** (ADRs are a separate sequence; next free there is **0012**).

## Active roster

<!-- roster:begin cap=320 -->
| Plan | Title | Status | Live constraint |
|------|-------|--------|-----------------|
| [0006](0006-spatial-audio-hrtf-mixer-and-occlusion.md) | Spatial audio: a headless HRTF mixer on the device callback, occlusion and early reflections through the material table | approved | M4, new crate `vxe-audio`; amended three times 2026-10-16; Phase 4 unblocked on `plan-0006`. Paired with ADR-0006. |
| [0007](0007-op-log-byte-channel-shared-and-owned-clients.md) | Server-owned op log, a byte channel, the logic boundary, shared and owned clients, and replay from seed plus log | approved | M5's split, new crate `vxe-net`; independent of Plans 0002 to 0006. Paired with ADR-0007 and ADR-0010. |
| [0009](0009-support-query-and-collapse-ops.md) | A support query on the coarse grid, and deterministic drop and rubble ops for collapse | approved | M8's collapse half; prerequisite Plan 0004 closed, otherwise independent. Paired with ADR-0008. |
| [0010](0010-stats-hud-bitmap-font-overlay.md) | A toggleable stats HUD: a bitmap-font text grid and a frame-time strip drawn as one overlay pass | approved | M7's overlay; amended 2026-10-17 (march check as before/after equality, world stats sampled). Phases 3-4 open on `plan-0010`. Paired with ADR-0009. |
| [0011](0011-wasm-host-for-the-logic-component.md) | A WebAssembly host for the logic component, proven by native-versus-component equality | approved | M5's scripting half, new crate `vxe-script-wasm`; prerequisite Plan 0007 closed. Paired with ADR-0010. |
| [0013](0013-contour-default-shadow-origin-and-streamed-footprint.md) | Contour as the fly-through's default: the shadow-ray origin, a streamed footprint under 3 GB, and a weaker rock detail | approved | M2 surface follow-up to ADR-0011; two `human` phases. The footprint pick gets its own ADR. |
| [0014](0014-edit-cost-brick-array-pool-and-cheaper-spans.md) | Edit cost under 2 ms, second pass: the allocation measured, a brick-array pool, cheaper sphere spans and an analytic capsule row | approved | The `nfr.md` §4 edit figure, 3.56 ms quiet at Plan 0003's close; prerequisites met. No ADR. |
<!-- roster:end -->

## Recently closed

<!-- roster:begin cap=320 -->
| Plan | Title | Closed |
|------|-------|--------|
| [0008](done/0008-vox-import-and-prefab-stamping.md) | MagicaVoxel import with a material map, and prefab stamping with 24 rotations at generation time and at runtime | 2026-10-17, v0.6.0 |
| [0012](done/0012-comment-hygiene-and-index-row-gates.md) | Two repository gates copied from Ritmolux: comment hygiene in sources, and a byte cap on index rows | 2026-10-17, no version bump |
| [0004](done/0004-cylinder-sweeps-character-controller-and-coarse-grid.md) | Cylinder sweeps, a character controller, and a per-brick coarse grid with change notifications | 2026-10-17, v0.5.0 |
| [0003](done/0003-edit-cost-span-fills-and-per-op-tables.md) | Edit cost under 2 ms: row spans, a per-op target table and per-brick tallies | 2026-10-17, v0.4.1 |
| [0005](done/0005-surface-look-spike.md) | The surface-look spike: smoothed normals, per-voxel contour planes and procedural detail, measured on screen | 2026-10-16, v0.4.0 |
| [0002](done/0002-streaming-residency-lod-ring-and-mirror-narrowing.md) | Streaming residency, an LOD-only ring, and a narrowed, sharded mirror | 2026-10-16, v0.3.0 |
| [0001](done/0001-bounded-brick-map-edits-and-headlamp-marcher.md) | Bounded brick map, deterministic edits, and a headlamp ray marcher | 2026-10-16, v0.2.0 |
<!-- roster:end -->

## Conventions

- One plan per milestone slice, numbered when drafted, never reserved ahead.
- A row carries at most two sentences of live constraint. What landed and why belongs in the
  plan file.
- A closed plan leaves this table; `done/` records it.
