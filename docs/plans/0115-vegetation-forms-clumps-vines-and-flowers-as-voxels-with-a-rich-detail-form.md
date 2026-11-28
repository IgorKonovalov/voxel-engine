# 0115: Vegetation forms: clumps, vines and flowers as world voxels, with a rich form of sub-voxel leaves and strands on their faces

> **Status:** draft (2026-11-28), awaits the owner's approval
> **Drafted on Opus 2026-11-28 (Fable exhausted for the week); a Fable second pass reviews and
> fixes this draft after the weekly reset (owner).**
> **Created:** 2026-11-28
> **Milestone:** M10: Outdoors in [`docs/roadmap.md`](../roadmap.md), the target look
> **Related ADRs:** [ADR-0090](../adrs/0090-every-look-feature-has-a-floor-form-and-a-rich-form-and-a-rich-profile-holds-its-own-budget-on-a-named-card.md)
> (the two forms), [ADR-0063](../adrs/0063-surface-detail-is-a-procedural-shell-marched-at-the-hit-described-per-material-on-the-render-side.md)
> (the detail layer the rich form adds element kinds to),
> [ADR-0066](../adrs/0066-a-large-prefab-is-stored-as-bricks-and-its-stamp-is-a-placement-the-world-finishes-as-chunks-arrive.md)
> (placements for the forms larger than Plan 0113's reach)
> **Source:** the owner, 2026-11-28: the target look's "foliage as dense clumps of small
> voxels, not blades", and after Plan 0081's brief 3d window: "besides grass we should have
> some other forms of vegetation - lians, grass with flowers etc etc" (the backlog's "More
> vegetation forms than grass blades"). Taller grass blades are Plan 0081's own (its lane's
> amendment before 4a), not this plan's.
> **Starts after:** Phases 1 and 2 (world side, headless) after Plan 0113 merges, on the world
> lane; Phases 3 to 6 on the look track after Plan 0034 closes (the costly half last,
> ADR-0090's order), with Plan 0081's element path on main.

## TL;DR

Vegetation in two forms. **The floor form is world voxels:** `svet-world` gains seeded shape
builders that make prefabs (a clump: a noise-perturbed ellipsoid filled densely enough to show
no pixel-sized holes; a strand: a hanging chain one voxel wide with a seeded drift; a scatter of
single voxels for flowers), which Plan 0113's anchors and ADR-0066's placements put in the
world. They are dug, collided with and heard like any voxel, and cost no frame time beyond the
march. **The rich form adds drawn detail on those voxels' faces** through Plan 0081's layer:
two element kinds, `Leaf` (small tilted discs on every face of a foliage row) and `Hang` (thin
strands below the down faces of a vine row), faded by size and swayed by the wind, applied by
the rich profile only. The first thing seen is the outdoor hills with clumps on their tops,
from a CPU test that the clump builder's output is one connected piece.

## Context & problem

- The target look's greenery is dense clumps of small voxels. Our trees are sparse leaf voxels
  stamped as prefabs, which sparkle and wobble in motion under jitter (the backlog's "Sunlit
  tree crowns sparkle in motion" and the wobble work, Plans 0095 and 0104).
- Plan 0081 draws blades, caps and pebbles on faces; nothing makes hanging vines or volume.
- Floor budget (`docs/nfr.md` section 2): the hit line is already met by the march alone (0081's
  2b reading), so a floor form must not add a pass; world voxels add none.

## Decision

The floor form is geometry in the world, built by generic seeded shape builders in
`svet-world` (`prefab` builders; no plant names in the engine: they are blob, strand and
scatter, and the consumer gives them materials). Clumps are dense by construction: the
builder's fill fraction is at least 0.85 inside the ellipsoid and the surface noise moves the
boundary by whole voxels, so the clump's silhouette changes and its interior shows no holes.
The rich form is render-side, two element kinds in Plan 0081's style table. We rejected
**the detail layer alone** (foliage as elements on grass faces: no volume, no silhouette at a
distance, and nothing at Floor if the layer's cost keeps it off there), **entities with a fine
grain** for foliage (ADR-0078's grain finer than a world voxel: an entity per clump would not
scale to a canyon of them), and **denser world foliage on the rich profile** (ADR-0090 rule 1).

## Implementation phases

### Phase 1: The shape builders and clumps on the hills (walking skeleton)
- **Owner skill:** dev
- **What:** `prefab::blob(seed, radii, fill, roughness)`, `prefab::strand(seed, length, drift)`,
  `prefab::scatter(seed, extent, count)`; the outdoor hills source gets clumps on its tops
  through a Plan 0113 anchor rule in the fly-through. An anchored clump fits the reach (radii
  up to (7, 7, 5): an extent of 15 voxels against a reach of 16); larger clumps are
  source-level placements (ADR-0066).
- **Files touched:** `crates/svet-world/src/prefab.rs` (or a new `prefab/build.rs`), its tests,
  `apps/flythrough/src/outdoor.rs`.
- **Done when:**
  1. `a_blob_is_one_dense_piece` (CPU, under 1 s): for 64 seeds at radii (12, 12, 8) and fill
     0.85, the blob is 6-connected, and every voxel inside the ellipsoid shrunk by 2 voxels
     is solid.
  2. `a_strand_hangs_connected` (CPU, under 1 s): for 64 seeds and length 30 the strand has 30
     voxels, each one below or beside the one before, never above.
  3. Builders are pure: equal seeds give equal prefab hashes; a fixed vector on both CI arms.

### Phase 2: Vines and flowers through the overgrowth pass
- **Owner skill:** dev
- **What:** short vines (up to the reach) as anchored strands under lips; longer ones as
  placements; flowers as a scatter on grass tops; demonstrated in the hills and handed to Plan
  0114's Phase 5.
- **Files touched:** `apps/flythrough/src/outdoor.rs`, tests in the fly-through, `docs/plans/logs/0115.md`.
- **Done when:** the hills' order test (the fly-through's source tests) holds with the forms; a
  reading of generation time and mirror bytes with and without them.

### Phase 3: The floor form's stillness (exploration)
- **Owner skill:** dev
- **What:** readings, no new mechanism: clumps at fill 0.85, 0.9 and 0.95 and two roughness
  values, in motion on the hills and Plan 0114's ledges (if merged), on Plan 0104's ranked
  clips or Plan 0090's scenes (if merged): pixels changing per frame on a slow pan against the
  bare terrain's figure.
- **Files touched:** `docs/plans/logs/0115.md`, the reading's `mod reading`.
- **Done when:** the readings log holds the table for both adapters and names the setting
  with the fewest changing pixels.

### Phase 4: The rich form: `Leaf` and `Hang` elements (exploration)
- **Owner skill:** dev
- **What:** two element kinds in Plan 0081's detail path, a style row each for the foliage and
  vine rows, faded with 0081's band and swayed by its wind; `RICH_FIELDS` (Plan 0112) gains
  their switch.
- **Files touched:** `crates/svet-render/src/shaders/detail.wgsl` (and the files Plan 0081 lands
  its elements in), the style table, `crates/svet-render/src/settings.rs`, its tests,
  `apps/flythrough/tests/gpu_detail.rs`.
- **Done when:**
  1. CPU: the style table round-trips both kinds; Plan 0112's profile test holds with them listed.
  2. A probe test in `gpu_detail.rs` (about 3 s): a `Hang` element's lowest point at wind 0 is
     its row's length below the face, within one 32nd of a voxel.
  3. Readings: cost at Rich on the RTX on the hills and the ruins, both scenes; stillness at
     wind 0 on 0081's 3c rig.

### Phase 5: The owner's window
- **Owner skill:** human
- **What:** the owner flies the ruins (or the hills) at Floor and at Rich: picks the clump
  fill and roughness, the vine lengths, the flower density, and keeps or drops each rich kind.
- **Done when:** the picks are in the readings log, each with its profile.

### Phase 6: The picks as tests, and the cost per profile
- **Owner skill:** dev
- **What:** defaults and profile values; goldens `vegetation_floor.png` and
  `vegetation_rich.png` on lavapipe (SSIM floor 0.98); the cost table in
  `docs/nfr/02-frame-budget.md`.
- **Done when:** the goldens pass; the table names each form per profile on both adapters.

## Budgets

| profile | form | projection (a guess) | how the close measures it |
|---|---|---|---|
| Portable, Floor | world voxels only | frame: the march's change from more surfaces, under 0.3 ms RTX; RAM and mirror: read | Phase 3 and 6 readings |
| Rich | + `Leaf` and `Hang` | 1.0 to 2.5 ms RTX, inside the Rich forms line (Plan 0112) | Phase 4 and 6, RTX |
| Section 5, generation | clumps and strands through anchors | within Plan 0113's 2x at reach 8 | Phase 2 reading |

## Risks & open questions

- **Clumps can still shimmer** at the silhouette under FSR 2; Phase 3 measures it before any
  rich form is built, and the fill and roughness are the levers.
- **The rich form depends on Plan 0081's run cost** (its 2b ruling, the resume pass): if 0081's
  stop rule fires, `Leaf` waits for 0081's amendment.

## What this plan does NOT do

- No trees as a growth system; trees stay prefabs the consumer places.
- No taller grass (Plan 0081's lane).
- No foliage physics or interaction; the wind is 0081's (and Plan 0036's when it lands).

## Implementation log

> Written by `dev`: one row per phase as its commit lands, then the close block. Readings,
> attempts and resume notes go to `docs/plans/logs/0115.md`.

| phase | owner | state | commit |
|---|---|---|---|
| 1: the shape builders and clumps on the hills | dev | not started | |
| 2: vines and flowers through the overgrowth pass | dev | not started | |
| 3: the floor form's stillness | dev | not started | |
| 4: the rich form, `Leaf` and `Hang` | dev | not started | |
| 5: the owner's window | human | not started | |
| 6: the picks as tests | dev | not started | |

### Notes

### Close triggers

## Followups
