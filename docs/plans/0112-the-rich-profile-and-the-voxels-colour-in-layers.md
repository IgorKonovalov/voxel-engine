# 0112: The rich profile and the voxels' colour in layers: a fifth profile with its own budget, then speckle, drift and a grain

> **Status:** draft (2026-11-28), awaits the owner's approval
> **Drafted on Opus 2026-11-28 (Fable exhausted for the week); a Fable second pass reviews and
> fixes this draft after the weekly reset (owner).**
> **Created:** 2026-11-28
> **Milestone:** M10: Outdoors in [`docs/roadmap.md`](../roadmap.md), the target look's first plan
> **Related ADRs:** [ADR-0090](../adrs/0090-every-look-feature-has-a-floor-form-and-a-rich-form-and-a-rich-profile-holds-its-own-budget-on-a-named-card.md)
> (proposed with this plan: floor and rich forms, the rich profile's budget),
> [ADR-0055](../adrs/0055-render-settings-are-one-struct-with-an-apply-class-per-field-and-profiles-are-named-values.md)
> (the profiles this adds to),
> [ADR-0063](../adrs/0063-surface-detail-is-a-procedural-shell-marched-at-the-hit-described-per-material-on-the-render-side.md)
> (the render-side per-material style table the amplitudes join, and the fade by size),
> [ADR-0043](../adrs/0043-emissive-light-from-a-world-space-per-face-table-and-a-lamp-bounce-probe-channel.md) (the voxel key
> `floor(p - 0.5 n)` the hash reads)
> **Source:** the owner, 2026-11-28 (the target look, `docs/references/local/wbk-voxel-2026/`;
> "make everything that we can do now ... and then maybe increase performance budget"); the
> backlog entries this plan takes: "Surface colour as the voxels' texture, in three layers",
> "Per-voxel albedo variation by a seeded hash" and "Ambient occlusion driving a per-material
> colour ramp" (as an arm of layer 2).
> **Starts after:** Plan 0081 closes (its style table and fade rule are this plan's). On the
> look track, before Plan 0035's aerial brief.
> **Exploration (lane rules 2b):** Phases 2 to 4 are exploration; the owner's window in Phase 5
> is their gate. Each lands its CPU and probe tests of data, determinism and API; every image
> judgement is a reading in `docs/plans/logs/0112.md` until Phase 6.

## TL;DR

ADR-0090 splits every look feature into a floor form and a rich form. This plan builds the
fifth render profile that applies the rich forms, `Profile::Rich` (name provisional), with its
own 16.6 ms budget on the RTX 3080 Laptop in `docs/nfr.md` section 2, and a test that the
profile differs from Floor only in fields a look plan lists. Then it builds the first feature
in both forms, the voxels' colour, cheapest first: a fixed per-voxel speckle of a few shades
(layer 1) and a world-space drift of hue and brightness over tens of metres with an optional
AO-driven colour ramp (layer 2) as the floor form, and a sub-voxel procedural grain faded by
the voxel's size on screen (layer 3) as the rich form. All three are stateless: no history,
no random stream, a static pattern under FSR 2. The first thing seen is `--profile rich` in
the banner, reading the same frame as Floor until a rich form exists.

## Context & problem

- The target look's stone reads as texture: tonal variation per voxel and over a wall, moss
  and stain colour in the crevices. Ours puts one `albedo` per row on every voxel
  (`composite.wgsl` reads `materials[rec.material * 4u].xyz`), so a ruin wall is flat.
- Image textures are refused (the backlog's "Surface colour" entry: they outline every cube,
  go sub-pixel past a few metres and sparkle under FSR 2).
- ADR-0055's profiles spend a faster card only on internal resolution; nothing above the
  floor buys look. The RTX 3080 Laptop reads Floor frames of 2.9 to 6.8 ms (`docs/nfr.md`
  section 2), two thirds unspent.
- Touches `docs/nfr.md` section 2 (a new profile's lines; the composite's cost inside the
  Floor "upscale, tone map, post" line, which is already over: the reading is a delta).

## Decision

`svet-render` gains `Profile::Rich`, applied by `RenderSettings::profile` like the others,
whose values are Floor's plus the fields listed in one constant, `Profile::RICH_FIELDS` (the
rich forms; empty at Phase 1). The colour layers are computed in the composite at the hit,
from the voxel key and the world position only: layer 1, `albedo * (1 + a * s)` with `s` one
of four shades from `hash3(seed, key)` and `a` the material's amplitude in the render-side
style table; layer 2, a value noise over world position at a cell of 16 to 64 m (a setting)
shifting hue and brightness by per-material amounts, and an arm where corner AO at the hit
(the face's 8 neighbours) picks a colour along a per-material ramp; layer 3, a 2D value noise
in the face plane at 4 to 8 cells a voxel, its amplitude scaled by `smoothstep` over the
voxel's projected size with Plan 0081's fade band. Layers 1 and 2 are the floor form; layer 3
is the rich form. We rejected image textures (above), colour on the material row (the owner,
2026-11-20: amplitudes are render settings), and a colour baked into the world per voxel (it
would cost storage and change the world per profile, against ADR-0090 rule 1).

## Implementation phases

### Phase 1: The rich profile, its budget lines and the banner (walking skeleton)
- **Owner skill:** dev
- **What:** `Profile::Rich` in `Profile::ALL` after High; `RICH_FIELDS` holds Plan 0081's rich
  picks from its window (its 2026-11-28 amendment), else nothing; the fly-through's
  `--profile rich` and its cycle; the tester's quality menu offers it; the banner names it;
  `docs/nfr.md` section 2 gains the Rich lines (Data shapes) as proposals.
- **Files touched:** `crates/svet-render/src/settings.rs`, `crates/svet-render/src/settings/tests.rs`,
  `apps/flythrough/src/cli/settings.rs`, `apps/tester/src/menu.rs`, `apps/tester/tests/run.rs`,
  `docs/flythrough.md`, `docs/nfr.md`, `docs/plans/logs/0112.md`.
- **Done when:**
  1. `the_rich_profile_differs_from_floor_only_in_its_listed_fields` (CPU, under 1 s): every
     `FIELDS` row's value under Rich equals Floor's unless the field is in `RICH_FIELDS`, and
     every listed field differs; with the list empty, the two values are equal.
  2. The banner test names `profile rich` after the cycle steps past High.
  3. A reading, RTX 3080 Laptop, `--profile rich` against `--profile floor` on the hills, the
     forest and the castle, three runs each, quiet, logged with the power state: with no field
     listed, the frame's GPU time is within the Floor runs' range; with Plan 0081's picks
     listed, the difference is that plan's own reading of those arms within the same range.

### Phase 2: Layer 1, the per-voxel speckle (exploration)
- **Owner skill:** dev
- **What:** the speckle in the composite's albedo read, amplitude per style row (0 for an
  unstyled material, so the frame is unchanged by default), the hash seeded by the settings'
  seed; `--colour-speckle on|off` and a toggle with the banner.
- **Files touched:** `crates/svet-render/src/shaders/composite.wgsl`, the style table's module
  from Plan 0081, `crates/svet-render/src/settings.rs`, new `crates/svet-render/tests/gpu_colour.rs`,
  the fly-through's toggles and CLI, `docs/flythrough.md`.
- **Done when:**
  1. `the_speckle_is_a_function_of_the_voxel_alone` (`gpu_colour.rs`, about 3 s on lavapipe):
     a probe dispatch of the speckle function returns the same value for every hit point on
     one voxel's six faces and for frame indices 0 and 7; over 4096 voxels of one material at
     `a = 0.2` the four shades each occur between 20 % and 30 % of the time.
  2. With every amplitude 0 the composite's bytes equal the base commit's at one pose.
  3. Readings: the composite pass's time on and off, both adapters, the hills and the
     showcase; stills for the window.

### Phase 3: Layer 2, the drift and the AO ramp arm (exploration)
- **Owner skill:** dev
- **What:** the world-space drift (cell and per-material hue and brightness amounts in the
  style row); the AO ramp as an arm (`--colour-ao-ramp on|off`, default off), its corner AO
  from the 8 neighbours at the hit and a three-colour ramp per style row.
- **Files touched:** `composite.wgsl`, the style table, `settings.rs`, `gpu_colour.rs`, the
  fly-through's toggles and CLI.
- **Done when:**
  1. `the_drift_is_continuous_across_voxels` (`gpu_colour.rs`, about 2 s): along a 64-voxel
     row the drift factor between neighbours changes by at most `amount / (cell / 0.1 m)` times
     the noise's derivative bound (the bound computed in the test from the cell).
  2. `the_ao_ramp_reads_the_open_face_as_its_bright_colour` (about 2 s): a probe at the centre
     of a face with all 8 neighbours air returns the ramp's bright colour exactly.
  3. Readings: cost of the drift and of the ramp separately, both adapters (the ramp's 8
     lookups are the likely cost); stills.

### Phase 4: Layer 3, the grain, the rich form (exploration)
- **Owner skill:** dev
- **What:** the face-plane grain faded by projected size with Plan 0081's fade band; its
  amplitude a style field; `RICH_FIELDS` gains the grain's switch, on under Rich only.
- **Files touched:** `composite.wgsl`, the style table, `settings.rs`, `settings/tests.rs`,
  `gpu_colour.rs`, the fly-through.
- **Done when:**
  1. `the_grain_fades_to_nothing_below_the_band` (about 2 s): a probe at a projected size
     below the band's low end returns a grain factor of exactly 1.
  2. Phase 1's test holds with the grain's field listed.
  3. Readings: the grain's cost at Rich on the RTX; a stillness reading on Plan 0081's 3c rig
     (or Plan 0090's scenes if merged) at wind 0, grain on against off.

### Phase 5: The owner's window
- **Owner skill:** human
- **What:** the owner flies the ruins demo (Plan 0114, if its first phases have merged; else
  the hills and the showcase) at Floor and at Rich and picks: the amplitudes per material,
  the drift cell, the AO ramp on or off at Floor, the grain at Rich or dropped.
- **Done when:** the picks are written in the readings log, each with the profile it applies to.

### Phase 6: The picks as tests, and the cost per profile
- **Owner skill:** dev
- **What:** the picks as defaults and profile values; goldens `colour_floor.png` and
  `colour_rich.png` (the showcase's hero pose, lavapipe, SSIM floor 0.98); the cost per profile
  table in `docs/nfr/02-frame-budget.md`.
- **Files touched:** `settings.rs`, the style table's presets, `gpu_colour.rs`, the goldens,
  `docs/nfr/02-frame-budget.md`, `docs/flythrough.md`.
- **Done when:** both goldens pass at their floor; the table names each layer's form and its
  reading per profile on both adapters (the RENOIR at Portable).

## Data shapes

The Rich lines in `docs/nfr.md` section 2 (proposals until Phase 1 and the look plans read
them; the owner's question 2 of the 2026-11-28 batch):

| Rich, RTX 3080 Laptop, 1080p output, FSR 2 at 1.5 | ms |
|---|---|
| The Floor profile's work, as read on this card | 8.0 |
| Rich forms (each look plan names its share in its cost per profile table) | 6.0 |
| Headroom (entities, game work) | 2.6 |
| Total | 16.6 |

```rust
// illustrative: a style row gains, beside Plan 0081's element fields
pub struct ColourStyle {
    pub speckle: f32,          // layer 1 amplitude, 0..=0.3
    pub drift: [f32; 2],       // layer 2: hue (turns), brightness (fraction)
    pub ramp: Option<[[f32; 3]; 3]>, // the AO arm's dark, mid, bright
    pub grain: f32,            // layer 3 amplitude, rich form
}
```

## Budgets

| profile | form | projection (a guess) | how the close measures it |
|---|---|---|---|
| Portable | layer 1 only | under 0.05 ms RENOIR | Phase 6, RENOIR, Portable 1280 x 720 |
| Floor | layers 1 and 2 (the ramp if picked) | 0.05 to 0.3 ms RTX, inside the composite's delta | Phase 2 to 3 readings, both adapters |
| Rich | layers 1 to 3 | +0.1 to 0.4 ms RTX for the grain | Phase 4, RTX |

No VRAM beyond the style table's rows; no world or streaming cost.

## Risks & open questions

- **The speckle reads as Lego at a near distance** (each voxel a flat tile). The shades stay
  within `a <= 0.3`, and the window judges; the drift is the larger-scale fallback.
- **The grain sparkles under FSR 2** if the fade band is wrong; the stillness reading in
  Phase 4 gates it, and ADR-0090 rule 2 keeps it out of every default if it fails.
- **The AO ramp's 8 lookups in the composite** may cost on the RENOIR what 0081's 2b found in
  the hit pass (occupancy); it is an arm, default off, and read before the window.

## What this plan does NOT do

- No image textures, no atlas, no per-face UVs.
- No colour written into the world (ADR-0090 rule 1); overgrowth colour by shape is Plan 0113's.
- No hue-keeping tone map (the backlog's "Overlapping coloured lights go white").

## Implementation log

> Written by `dev`: one row per phase as its commit lands, then the close block. **The phases
> above are the contract. This section records what happened.** Readings, attempts and resume
> notes go to `docs/plans/logs/0112.md`.

| phase | owner | state | commit |
|---|---|---|---|
| 1: the rich profile, its budget lines and the banner | dev | not started | |
| 2: layer 1, the speckle | dev | not started | |
| 3: layer 2, the drift and the AO ramp arm | dev | not started | |
| 4: layer 3, the grain | dev | not started | |
| 5: the owner's window | human | not started | |
| 6: the picks as tests | dev | not started | |

### Notes

### Close triggers

## Followups
