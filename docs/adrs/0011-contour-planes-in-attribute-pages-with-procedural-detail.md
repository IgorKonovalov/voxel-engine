# ADR-0011: Smooth surfaces from per-voxel contour planes in attribute pages of the brick pool, with procedural per-material detail

> **Status:** accepted (2026-10-16)
> **Date:** 2026-10-16
> **Related plan(s):** [Plan 0005](../plans/done/0005-surface-look-spike.md) (the spike this
> decision is written from; its comparison table is the evidence)
> **Settles:** ADR-0002's reserved per-voxel attribute (the reservation is used, packed into the
> pool rather than as a parallel buffer; see Decision).

## Context

The marcher shaded every hit with its face normal, so the cave read as cubes. The roadmap's M2
surface-look decision wants smooth natural rock at 10 cm voxels, one material id per voxel and
no stored colour or occlusion, and wanted the mechanism chosen by measurement. Plan 0005 built
four normal modes (face, `Trilinear8`, `Gradient27`, `Contour`) and a procedural detail term
behind flags, measured each on two analytic scenes and on the level, and ended in the owner's
on-screen call.

The evidence (Plan 0005 Phase 5 table; image metrics on lavapipe at 320 x 180, 1080p march
pass on the NVIDIA GeForce RTX 3080 Laptop GPU at 37b22fa, `flythrough --level --headless
--seed 1`, taken while the machine's 1-minute load read 3 to 5):

| mode | cavity normal error (deg) | pillar silhouette error (px) | 1080p march (ms) |
|---|---|---|---|
| Face | 40.75 | 172 | 1.876 to 1.877 (1.826 to 1.859 without attribute pages) |
| Trilinear8 | 18.62 | 172 | 2.874 to 2.907 |
| Gradient27 | 6.95 | 172 | 4.564 to 4.612 |
| Contour | 7.41 | 32 | 1.927 to 1.944 |

Detail at rock `[4.0, 0.3]` adds 0.04 to 0.15 ms in every mode. The owner's windowed readings
(fullscreen 2530 x 1378, `F1` medians, detail off) were face 3.96 ms, contour 5.06 ms and
`Gradient27` 8.06 ms. Only `Contour` moves the silhouette; the shading-only modes cannot.

The constraint that shapes the storage: the marcher already binds WebGPU's default 8 storage
buffers (four pool shards, records, table, materials, masks; ADR-0004), and neither a voxel nor
a record entry has spare bits for a 16-bit attribute. VRAM is the other force: the attribute is
`u16` per voxel, twice a narrow brick.

## Decision

**Surfaces are per-voxel contour planes.** Each voxel of a `Full` brick in a full chunk carries
a `u16` attribute: a 6 + 6-bit octahedral normal and a 4-bit offset code `k` in 0..=14 for
`(k - 7) / 8` voxel along the normal from the voxel centre; `0xFFFF` (`NO_PLANE`) marks an
interior voxel. The GPU computes it in `attrib.wgsl` from the pool itself (`Gradient27` of
occupancy for the normal, the 27-voxel solid fraction for the offset), for every brick
`GpuWorld::sync` wrote plus its 26 neighbours and the boundary bricks next to any chunk whose
record changed. In `Contour` mode the marcher intersects the ray with the hit voxel's plane:
it hits at the plane, skips the voxel when the ray stays outside the solid half-space, and
hits as a cube where there is no plane (an interior voxel, a uniform brick, a mip cell).
**The attribute lives in attribute pages of the existing pool shards**, with no new binding:
with `GpuWorld::with_surface_attrs`, a 4096-slot page is either plain (mip-bricks) or an
attribute page, whose slots 0..1364 are brick slots and whose slots `1364 + 2i` and
`1365 + 2i` hold the 1024 B of attributes of brick slot `i`; the attribute slot is computed
(`attr_slot`), never stored, and always in its brick's shard. A wide brick uses its first
slot's pair and leaves the second pair unused; a page keeps its kind for the mirror's life.
**Mip-bricks carry no attribute**, so the LOD ring hits and shades as cubes. **Procedural
per-material detail stays on:** `Material::detail` in `vxe-world` (feature scale in voxels,
strength in `[0, 1]`) drives two octaves of value noise at world position that tilt the
shading normal (gain `DETAIL_TILT = 4`) and modulate albedo; strength 0 is exactly no detail.
The rock default becomes weaker than the spike's `[4, 0.3]`, set by the follow-up from the
owner's eye. Two rules are decided here and owed to the follow-up plan: **the shadow ray
leaves a plane hit offset along the plane normal** (the spike offset every mode along the
face normal, which put contour's shadow origins inside neighbouring rock), and **the streamed
footprint must fit the 8 GiB dev card and the 3 GB §3 line**; which reduction (attributes on
surface voxels only, a smaller encoding, a per-block plane, a smaller `r_full`) is the
follow-up plan's measured choice.

## Consequences

### Positive

- The only measured mode that smooths the silhouette: pillar error 32 px against 172, and a
  cavity normal error of 7.41 degrees, within half a degree of `Gradient27`.
- Nearly free per frame: 0.05 to 0.07 ms over face at the level on the dev box, against
  2.7 ms more for `Gradient27`; the per-hit cost is one attribute read and a plane test, and the
  smoothing work moves into a pass that runs only on dirty bricks (initial level fill 39.6 to
  55.0 ms for 383 608 bricks; one 1 m carve 0.020 ms for 46 bricks; 99 bench carves 0.310 ms).
- No ninth binding: `limits_for` keeps requesting 8 storage buffers, and ADR-0004's shard
  scheme carries the attribute unchanged. Without `with_surface_attrs` the mirror is byte for
  byte Plan 0002's, so the face goldens and every Plan 0002 reading stand.
- Consecutive brick slots stay consecutive, so staged runs stay batched: the level's first
  sync issues 571 queue writes with attribute pages (195 without), not one per brick.
- Detail is a pure function of world position and a material row, so it costs no storage and
  stays camera-stable.

### Negative

- **VRAM is the open cost.** 1040 B more live per narrow brick slot, and a page holds 1364
  bricks in 4096 slots. The level reads 606 633 408 B live and 613 759 104 B allocated against
  207 681 088 B and 213 334 144 B without attribute pages (6d77f12). Streamed at the defaults
  (r_full 16, r_lod 24, 37b22fa) the mirror reached 3 309 723 820 B live and
  4 103 413 164 B allocated at full residency, over the 3 GB line, then stopped on
  `wgpu error: Out of Memory` on the 8 GiB card. The decision is not shippable at the default
  radius until the follow-up reduces the footprint.
- **Lavapipe cannot hold the level with attribute pages** (256 pages of 1364 under four
  128 MiB shards hold 349 184 brick slots against 383 608): level-size contour readings are
  dev-box only; CI covers the mode on the test scenes.
- **Artifacts under the shadow ray.** Contour's black lines and dark specks come with the
  shadow ray and vanish with it off; the speck count on the contour golden scene is 372
  only-with-shadows in contour against 174 in face (llvmpipe). The origin rule above is
  decided; its fix and measurement are the follow-up's.
- **The LOD ring stays cubic.** A mip cell has no plane; a downsampled plane per mip cell is a
  later decision if the LOD look calls for it.
- **The offset formula is crude** (`(f - 0.5) * 3` from a 27-voxel fraction). A least-squares
  plane fit is the remedy if silhouettes look lumpy; no metric demanded it yet.
- **Pages fixed in kind waste space.** A partly filled attribute page reserves its attribute
  slots, a wide brick's second pair is unused, and a page freed of bricks never returns to the
  plain kind. `GpuWorld::free_slots` returns the plain kind's free list only, which changes its
  meaning for a mirror with attribute pages.
- **No baseline reading.** The dev box is about two to three times the baseline's throughput,
  so the level's 1.93 to 1.94 ms projects to roughly 3.9 to 5.8 ms there (face: 3.8 to
  5.6 ms), against a 3.0 ms line that this pass, which includes the shadow ray, already misses
  in projection in every mode. This is a projection, not a measurement. The streamed
  attribute-pass cost under motion is not read either.

## Alternatives considered

### Alternative A: `Gradient27` normals smoothed in the shader

The owner's first preference on screen and the lowest cavity error (6.95 degrees), with no
storage at all. It lost on two counts: it cannot move a hit, so silhouettes stay cubic (172 px,
as face), and it reads 27 voxels per hit, 4.56 to 4.61 ms at the level on the dev box (8.06 ms
at the owner's 2530 x 1378), which projects to two or three times the 3.0 ms line on the
baseline. Its shadows also read wrong to the owner. `Trilinear8` (18.62 degrees, 2.87 to
2.91 ms) halves the cost and still does not smooth enough.

### Alternative B: a parallel attribute buffer (ADR-0002's original reservation)

A ninth storage binding beside the shards, indexed by slot. It needs
`max_storage_buffers_per_shader_stage` above WebGPU's default 8, the alternative ADR-0004's
Outcome names to packing. Attribute pages give the same slot-indexed attribute inside the
existing bindings. Other packings lost earlier in Plan 0005: spare bits in the voxel or the
entry (none exist), attribute slots adjacent to each brick (breaks every staged run into one
write per brick, about 383 608 writes for the level's first sync against 195), and an
attribute pointer per slot in the masks buffer (changes the mask stride for every mirror and
adds a dependent read per hit).

### Alternative C: face normals with procedural detail only

The cheapest (detail adds about 0.1 ms) and no VRAM. The owner's eye rejected the cube look the
spike was started to remove; detail alone reads "between noise and shadows" at 0.3 and cannot
hide a stepped silhouette.

## Outcome

Added 2026-10-20 at the Plan 0013 close (v0.19.0), from the plan's readings and its
fresh-session review. Contour with detail is the fly-through's default; `--look plain` keeps
face mode without attribute pages.

- **The shadow-ray rule holds.** A plane hit's shadow ray starts half a voxel off the hit
  along the plane normal; a cube hit keeps the face normal. On the contour golden scene
  (llvmpipe, 320 x 180) the only-with-shadows specks fell from 372 to 52 against face mode's
  169, and a test gates contour at or below face.
- **The footprint rule is met in bytes, not in radius.** The streamed footprint was not
  reduced here: the owner moved surface-only attribute storage to ADR-0012's mirror-slot plan
  (2026-10-20). Under ADR-0013's 3 GiB reservation the pressure rule lowers `r_full` from 16 to
  12 and settles at 13; peak live is 2 864 318 452 B, under the 3 GB line, with no
  out-of-memory stop on the 8 GiB card. Full detail reaches 76.8 to 83.2 m against `nfr.md`
  §3's "about 100 m" until that plan.
- **Rock detail is `[4.0, 0.1]`**, the owner's pick on screen.
- **"Nearly free per frame" holds for the march, not for the attribute pass under motion.**
  At the streamed spawn once residency settles, contour with detail reads 2.28 ms against face
  mode's 2.13 ms (dev box, loaded, 3000 frames). But the attribute pass under the autopilot's
  flight costs 2.0 ms of GPU at p50 and 4.3 ms at p99. On the render thread, building its work
  list costs 3.7 ms at p50 and 139 to 167 ms at p99, which is a visible hitch. All of these are
  diagnostic readings taken at a 1-minute load of 3.6 to 19 (`docs/nfr.md` §2). The work-list
  cost is a followup; the decision stands.
- **The attribute pass keeps its buffers.** The uniform, the work list (grown to powers of
  two) and the bind group are kept between calls. The GPU chunk table is created filled with
  `ABSENT`, so a boundary brick next to an absent chunk reads its plane. A headless test covers
  the shell recompute when a neighbour arrives and leaves.

Added 2026-10-22, from Plan 0028 (ADR-0012's mirror-slot step):

- **The footprint is settled, and the attribute pages are gone.** The attribute's encoding and
  `NO_PLANE` are unchanged; it is now stored inside a full chunk's brick slot for its
  chunk-local surface voxels only, addressed by popcount. The level's contour mirror reads
  157 872 640 B live (680 491 136 B before), and the default holds `r_full` 16 at 16/24 without
  the pressure rule.

Added 2026-10-27, from Plan 0039 (the contour look at arm's length):

- **The encoding stands; the fit and the detail's lighting path changed.** The attribute
  pass gains two fits into the same `u16` (`PlaneFit::Weighted27`, a `1 / |o|^2`-weighted
  gradient, and `PlaneOffset::VolumeMatched`, the slab-volume offset this ADR called crude),
  and the composite can apply the detail's lamp term after lighting: the light pass shades
  with the undetailed normal, the composite multiplies the direct part by
  `clamp(max(n_d.l, 0) / max(n.l, 1e-3), 0, 2)` and adds the probes' indirect part unscaled.
  The engine's defaults stay every part off; the fly-through's default look is
  `contour-sharp` (the weighted fit plus the ratio, rock `[4.0, 0.1]`), the owner's call on
  screen, re-confirmed with probe GI.
- **The softness was the lighting path, not the fit.** On the headless wall at 1 m under the
  lamp (lavapipe), moving from the half to the full lighting tier recovered 74.6 % of the
  face-to-contour fine-band gap and the filter 3.5 %; the fit variants lowered the fine band
  by 1 to 4 %. Mirror bytes are unchanged under every fit (the level 157 872 640 B live).
- **The volume-matched offset is not free.** It fills the level's attributes in 46.6 ms
  against 17.0 ms on the dev box and won no preset; the weighted fit costs about 5 to 9 % of
  the fill.
- **The face mix fails the cavity normal guardrail** (12.14 degrees at a quarter mix), so a
  partial face normal is not a route back to edge contrast.
- **One plane per voxel still shows at arm's length.** The owner saw sharp voxel corners on
  some smooth walls in the default look; the plan's followups carry it.
