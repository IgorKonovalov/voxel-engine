# 0029: Indirect light from a probe volume that scrolls with the camera

> **Status:** approved (2026-10-21)
> **Created:** 2026-10-21
> **Milestone:** M2 in `docs/roadmap.md` (the indirect-light plan after Plan 0024; "indirect
> light from probes in both tiers", the floor decision of 2026-10-17)
> **Related ADRs:** [ADR-0023](../adrs/0023-indirect-light-from-a-scrolling-probe-volume.md)
> (proposed with this plan: the probe volume and the rejected per-pixel bounce),
> [ADR-0018](../adrs/0018-direct-light-per-pixel-one-sample-from-a-hit-buffer-with-svgf-history-killed-by-edits.md)
> (the direct estimator the probe rays reuse, the hit buffer the gather reads),
> [ADR-0013](../adrs/0013-mirror-reserves-its-vram-budget-up-front.md) (the atlases are
> reserved once)
> **Source:** Plan 0024's Followups (the indirect-light plan); the owner's choice of the probe
> family (2026-10-21); `docs/references/voxel-optimizations.md` (DDGI).
> **Starts after:** Plan 0024 closes. This plan is drafted against Plan 0024's data shapes
> (`Frame`, `View`, the hit buffer, `light.wgsl`'s estimator, `composite.wgsl`,
> `Frame::invalidate`) and names them as that plan does; the names dev finds in the tree win.

## TL;DR

A grid of 32 x 32 x 32 irradiance probes at 1.6 m spacing follows the camera. Each frame, one
eighth of the probes and every probe an edit touched trace 64 rays each through the full walk;
a ray returns the reflected radiance at its hit (albedo over pi times the direct estimate
there plus the probe irradiance there, so bounces compound), and the probe blends it into a
6 x 6 octahedral irradiance map and a 14 x 14 visibility map. A gather pass reads the eight
probes around each hit pixel with visibility weights and writes an indirect irradiance that
the composite adds to the direct one. The first thing a reader sees is a test box whose walls
glow and whose floor reads the furnace value, then the dark cave's walls lit by the vein
around the corner.

## Context & problem

After Plan 0024 a surface that faces no light is black. The consumer's dark cave
(`project-context.md`, Rendering) is read by its indirect light: the far wall of a headlamp's
cavern, the rock behind a glowing vein, the underside of a ledge. The roadmap's M2 lists the
Sibenik reference whose "soft light" is baked occlusion we do not store, and names probes as
the fallback GI in both tiers. ADR-0023 chooses the probe volume and says why not a per-pixel
bounce, cone tracing, a per-voxel cache or radiance cascades.

Cost facts: a full-walk ray costs about 0.9 ms per million at 1080p on the dev box (Plan
0015); 32 768 probes at one eighth per frame are 4096 probes x 64 rays = 262 144 rays, plus
two coarse shadow rays each (Plan 0024 Phase 4's walk), so about 0.25 ms of full walk and
under 0.3 ms of shadow walk, a projection. The gather reads 8 probes x 2 texels per hit
pixel, about 0.3 to 0.5 ms at 1080p. The §2 lighting line is 6.0 ms; Plan 0024's direct light
projects 1.5 to 2.5 ms there. Memory: 32 768 x (8 x 8 x 8 + 16 x 16 x 4) B = 50.3 MB of
atlases plus 32 KiB of state, under §3's 1.5 GB "everything else" beside Plan 0024's 276 MB.

The furnace arithmetic that the headless proof rests on. In a closed box whose every wall
emits radiance `L_e = 1` with albedo `rho = 0.5`, the radiance everywhere is `L = L_e / (1 -
rho) = 2` and the irradiance at any wall point is `pi L = 2 pi`. Plan 0024's direct estimator
at a wall point sees emitters over its whole hemisphere, so its expected direct irradiance is
`pi L_e = pi`. The indirect irradiance is therefore `pi` (about 3.14). A probe ray in this
plan returns reflected radiance only (the hit's emission is not added, because the direct
estimator already samples every emissive voxel), `rho / pi x (E_direct + E_probe)` at the
hit, so the probes' fixed point is `E = pi rho / (1 - rho) = pi` as well: the two agree, which
is the test.

## Decision

We build ADR-0023's volume in `vxe-render`: a `ProbeVolume` with two atlases reserved once,
a state buffer, an origin in probe steps, an update pass (`probe_update.wgsl`) that traces and
blends a slice of probes per frame plus the edit-touched ones, and a gather pass
(`probe_gather.wgsl`) that writes an indirect irradiance texture at the lighting resolution,
which `composite.wgsl` adds before multiplying the albedo back. Probe rays are shaded by the
same functions as Plan 0024's light pass (next-event estimation of the headlamp, one emissive
sample from the light-sample buffer, coarse shadow rays), reading the previous frame's probe
irradiance at the hit for the bounce. The volume is one per `Frame`, shared by its views, and
identical in both lighting tiers. We rejected the per-pixel bounce, cone tracing, the
per-voxel cache and radiance cascades (ADR-0023).

## Architecture diagram

```mermaid
flowchart LR
    subgraph world [vxe-world]
        E[EditResult.aabb]
    end
    subgraph render [vxe-render]
        HB[(hit buffer)] --> G[probe_gather\n8 probes, visibility weights]
        PU[probe_update\n1/8 of probes + touched,\n64 rays each, full walk] --> AT[(irradiance atlas 6x6+1\nvisibility atlas 14x14+1\nstate: inside, age)]
        AT --> G
        AT -. previous frame .-> PU
        LS[(light samples)] --> PU
        G --> IR[(indirect irradiance)] --> C[composite\nalbedo x direct + indirect]
        E -.-> PU
    end
```

## Implementation phases

Each phase ships as its own commit. `dev` implements all phases in one session with no review
between phases. The architect reviews once at the end, in a fresh session.

### Phase 1: The volume, the gather, and a constant probe
- **Owner skill:** dev
- **What:** `ProbeVolume` (atlases, state, origin, `set_origin`, `write_test_irradiance` for
  tests), the gather pass reading the hit buffer and writing the indirect texture, and the
  composite adding it. `Frame::indirect(bool)` and the fly-through's `--indirect off`.
- **Files touched:** new `crates/vxe-render/src/probes.rs`, new
  `crates/vxe-render/src/shaders/probe_gather.wgsl`, `composite.wgsl`, `marcher.rs` (the
  `Frame`), `apps/flythrough/src/main.rs`, new `apps/flythrough/tests/indirect.rs`.
- **Done when:** on the cavity scene at 320 x 180 on lavapipe with the lamp off, no emissive
  material, the filter off and the identity tone map, every probe preset to irradiance
  `[1, 1, 1]` with visibility "unoccluded" makes every hit pixel read its material's albedo
  within 2/255 per channel, and every miss pixel 0; with `--indirect off` every pixel reads 0;
  a probe preset to the inside flag contributes nothing (a pixel between one live and one
  inside probe reads half the albedo within 4/255).

### Phase 2: The update pass and the furnace
- **Owner skill:** dev
- **What:** the update pass: a slice of 4096 probes per frame in round robin, 64 spherical
  Fibonacci directions rotated per frame by a blue-noise angle, full-walk rays, reflected
  radiance at the hit from Plan 0024's estimator plus the previous atlas's irradiance at the
  hit, octahedral blend at hysteresis 0.97 (0.5 for a probe whose age is 0), the visibility
  map's mean and squared distance, back-face hits as negative distance and the inside flag
  over a quarter of back faces. The gather's Chebyshev visibility weight.
- **Files touched:** new `crates/vxe-render/src/shaders/probe_update.wgsl`,
  `probe_gather.wgsl`, `probes.rs`, `light.wgsl` (its estimator as callable functions),
  `tests/indirect.rs`.
- **Done when:** in a closed box of 24 x 24 x 12 interior voxels whose six walls are an
  emissive material with emission `[1, 1, 1]` and albedo 0.5, with the lamp off, the gathered
  indirect irradiance at the floor's centre pixel, read back after 256 frames, is within 10 %
  of pi (3.14; the furnace arithmetic above), and the probe nearest the box's centre holds
  within 10 % of pi in every irradiance texel; two such boxes side by side, separated by a
  2-voxel wall, only one of them emissive, read the dark box's floor-centre indirect under 2 %
  of the lit box's after 256 frames (no leak); a probe placed inside solid rock reads the
  inside flag after its first update and its texels are never read by the gather.

### Phase 3: Scrolling and edits
- **Owner skill:** dev
- **What:** the origin follows the camera in whole probe steps; a scrolled-in probe starts at
  age 0; `Frame::invalidate(aabb)` also marks every probe whose cell the box touches for the
  next update at hysteresis 0.5; a probe whose cell an edit filled re-tests the inside flag.
- **Files touched:** `probes.rs`, `probe_update.wgsl`, `marcher.rs`, `tests/indirect.rs`.
- **Done when:** after a camera jump of 20 probe steps along `x`, the 12 x 32 x 32 probes
  that stay read back texel for texel what they held before the jump and the entering probes
  read age 0; in the two-box scene, a radius-4 carve through the dividing wall at the frame
  after convergence makes the dark box's floor-centre indirect rise above 20 % of the lit
  box's within 32 frames (the touched probes refresh at once and the rest within four
  slices), and filling the hole back drops it under 5 % within 64 frames.

### Phase 4: The cave, the switches and the readings
- **Owner skill:** dev
- **What:** HUD rows for the update and gather passes' GPU time and the live probe count;
  `--indirect off|on`, `--probe-spacing` (16 voxels default; 8 as a test of the finer cascade
  the ADR names); the autopilot prints both passes' p50 and p99 and the volume's bytes in
  `Frame::frame_bytes`; a golden of the cavity scene lit only by an emissive vein around a
  corner from the camera after 64 frames.
- **Files touched:** `apps/flythrough/src/main.rs`, `hud.rs`, `apps/flythrough/tests/golden.rs`,
  `tests/indirect.rs`, the root `README.md` (flags, keys), `docs/nfr.md` §2 and §3.
- **Done when:** `indirect.png` is blessed and with `--indirect off` the same view differs in
  at least 20 % of its hit pixels by more than 8/255 (the vein's bounce light is what the
  golden shows); the update plus gather passes read at most 1.5 ms at 1080p at the streamed
  spawn on the dev box (median of the last 30 of 3000 frames, three runs), and the full §2
  lighting line (Plan 0024's light pass plus temporal plus this plan's two passes, at full
  resolution) at most 6.0 ms there; both passes are also read on the slow adapter (`nfr.md`
  §1) with no bar; `frame_bytes` rises by 50.3 MB plus the state buffer and no more.

### Phase 5: The owner's windowed check
- **Owner skill:** human
- **What:** the owner flies the dark cave with the lamp on and off, around the vein's
  corners, and digs into a dark room next to a lit one, with `--indirect` toggled; a verdict
  on the softness (1.6 m spacing) and on leaks, and whether the finer spacing is wanted.
- **Files touched:** this plan's log, ADR-0023's Outcome at the close.
- **Done when:** the owner has recorded a verdict and the spacing choice in the log.

## Data shapes

```rust
// illustrative: the volume
pub const PROBES_PER_AXIS: u32 = 32;
pub const PROBE_SPACING_VOXELS: u32 = 16;             // 1.6 m
pub const IRRADIANCE_TEXELS: u32 = 6;                  // + 1 border each side in the atlas
pub const VISIBILITY_TEXELS: u32 = 14;                 // + 1 border
pub struct ProbeVolume {
    irradiance: wgpu::Texture,   // rgba16float, 2048 x 1024 texels (256 x 128 tiles of 8), 16.8 MB
    visibility: wgpu::Texture,   // rg16float,   4096 x 2048 texels (256 x 128 tiles of 16), 33.6 MB
                                 // both under the 8192 default texture limit
    state: wgpu::Buffer,         // u8 per probe: bit 0 inside, bits 1..8 age (capped)
    origin: [i32; 3],            // in probe steps, world voxel = origin * 16 + index * 16
    slice: u32,                  // 0..8, the round-robin cursor
}
// probe index (x, y, z) -> atlas tile (x + 32 (y % 8), y / 8 + 4 z); a direction -> octahedral uv
// update: 4096 probes x 64 rays per frame; touched probes added to the slice
```

## Budgets

| `nfr.md` figure | line | projection (dev box, 1080p) | how the close measures it |
|---|---|---|---|
| §2 Lighting (with Plan 0024's passes) | 6.0 ms | update 0.5 to 0.8 ms, gather 0.3 to 0.5 ms, on top of direct 1.5 to 2.5 ms | Phase 4 runs at the streamed spawn and the level, three each, both adapters |
| §3 Everything else in VRAM | 1.5 GB | +50.3 MB atlases, +32 KiB state | `frame_bytes` |
| §2 Streaming line | 1.0 ms | unchanged (no CPU work per frame beyond the origin step and the touched list) | the autopilot's render-thread timers |

## Risks & open questions

- **Probes in rock** are the quality risk in a cave: a room narrower than 1.6 m may hold no
  live probe. Phase 2's leak test and Phase 5's eye are the bar; `--probe-spacing 8` is the
  measured fallback (eight times the probes, 400 MB, which only a finer inner cascade would
  make affordable), and DDGI's probe relocation the design fallback.
- **The bounce reads the previous atlas while the slice writes the current one**: the atlas
  is double-buffered or the slice reads probes outside itself; Phase 2 picks the former (two
  atlases, 100 MB) only if the readback test shows a race on lavapipe.
- **Determinism** per adapter, as Plan 0024: a `(probe, ray, frame)` seed through blue noise;
  the furnace tolerance is 10 % for that reason. Nothing touches `vxe-world`.
- **Adapter features:** `rgba16float` and `rg16float` storage textures with read and write
  in the same pass are not guaranteed; the update writes a new slice texture and copies, or
  uses storage buffers for the atlases if lavapipe refuses the format (a Phase 1 check, logged).
- **The mirror window:** probe rays that leave the window see air; the volume's 25.6 m
  half-extent is inside `r_full` 16 (102 m) and inside the settled 12 (77 m).
- **Threading:** none; every pass is a compute dispatch on the frame's encoder.

## What this plan does NOT do

- **A per-pixel bounce or ReSTIR GI** (ADR-0023 Alternative A): a later quality mode.
- **Specular or glossy indirect**: the material table has roughness, and nothing here reads
  it; a followup with the entities plan if the eye asks.
- **Probes for the LOD ring or the far field**: the volume is near-field only.
- **A finer inner cascade**: Phase 5's call, its own plan.
- **Translucent voxels** as receivers or emitters of indirect light: M8's media plan.

## Implementation log

> Written by `dev`: one row per phase as its commit lands, then the close block. **The phases
> above are the contract. This section records what happened.** Observations, never conclusions:
> no pass list, no self-assessment. Deviations and unmet done-whens are **always** disclosed.
> Keep it shorter than `## Implementation phases`.

| phase | owner | state | commit |
|---|---|---|---|
| 1: The volume, the gather, and a constant probe | dev | not started | |
| 2: The update pass and the furnace | dev | not started | |
| 3: Scrolling and edits | dev | not started | |
| 4: The cave, the switches and the readings | dev | not started | |
| 5: The owner's windowed check | human | not started | |

### Notes

_(Deviations from the plan, with the commit, stated without justification. Done-whens not
satisfiable as stated, and what was done instead. Followups noticed and not acted on. One line
each. Empty is fine.)_

### Close triggers

_(Facts for the architect. No recommendations, and no suggested version bump.)_

- **What shipped:** feature / fix-only / docs-chore-only
- **Public API changed:** types, traits, functions, shader interfaces, config (list them, or none)
- **Budget readings:** each `nfr.md` figure touched, with its measured value, machine and commit
- **Gate at the tip:** the commands run (build, nextest, doctests, clippy, fmt, deny), exit codes, test counts
- **Outstanding `human` phases:** which, or none

## Followups

- The showcase cave (owner, 2026-10-20): several ore materials with different emission
  colours as the light sources, after this plan.
