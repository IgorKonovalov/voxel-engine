# 0058: Dynamic lights: a list of eight analytic lights the consumer hands the frame, shaded exactly, dropped and carried in the fly-through, and a bounce flag per source

> **Status:** approved (owner go 2026-11-10; the lane starts after Plan 0061 merges and the
> architect's re-read below), was draft (redrafted 2026-11-10 over the baked store of ADR-0046; the emission-scale
> phase and ADR-0045 are gone, Plan 0059 built that; amended 2026-11-10 for a light fired in
> flight, the owner's projectile: Phase 1's `6` and done-whens 11 and 12, Phase 4's run 5)
> **Created:** 2026-11-04
> **Milestone:** M2 in [`docs/roadmap.md`](../roadmap.md) (the consumer's "headlamp, flares,
> glowing crystal"; the per-source switch on feeding the probes, parked 2026-10-22)
> **Related ADRs:** [ADR-0044](../adrs/0044-direct-light-from-a-fixed-list-of-analytic-lights-one-shadow-ray-each.md)
> (proposed; the list, exact shading, reach, history, the bounce flag), accepted at this plan's
> close. Rests on [ADR-0046](../adrs/0046-static-emissive-light-is-baked-per-chunk-when-a-chunk-loads-or-changes.md)
> (accepted: emissive light on a cube face is the baked store's, which this plan leaves alone),
> on the lamp channel of [ADR-0043](../adrs/0043-emissive-light-from-a-world-space-per-face-table-and-a-lamp-bounce-probe-channel.md)
> (accepted; its table is superseded by ADR-0046, its lamp channel stands) and on
> [ADR-0018](../adrs/0018-direct-light-per-pixel-one-sample-from-a-hit-buffer-with-svgf-history-killed-by-edits.md)
> (the lamp's exact estimate and its reset).
> [ADR-0045](../adrs/0045-the-per-face-table-stores-irradiance-at-unit-emission-and-the-read-applies-the-scale-now.md)
> is withdrawn: the store is baked at unit emission and read times the scale now since Plan 0059.
> **Source:** the owner, 2026-11-04: "I want to be able to for example light cavern by dropping
> some light source inside, and see lighting change in real time", and the answers under Owner
> answers.
> **Starts after:** [Plan 0061](0061-antialiasing-far-bands-under-the-baked-light-and-the-edges-left.md)'s
> merge. The owner's order of 2026-11-10 is Plan 0060, Plan 0061, this plan, then
> [Plan 0044](0044-hot-path-and-defect-pass.md) and
> [Plan 0045](0045-structure-pass-split-the-mega-files-share-the-fixtures.md). Plan 0061 edits
> `temporal.wgsl` (the store read), `composite.wgsl` (the emitter cap) and `Lighting` (one bit),
> the files Phases 1 and 2 here change. **Before the lane starts the architect re-reads Phases 1 and 2
> against Plan 0061's tip** and refreshes their paths; bit and binding numbers in this plan are
> "the next free at the start commit", which dev reads from the tree.

> **Amended 2026-11-10 (test cost, owner).** Phases 1 to 3 are **exploration** (lane rules 2b):
> the owner's window in Phase 4 is their gate. Each lands its CPU tests of data, determinism
> and API; every test it names that judges the image (goldens, SSIM or pixel floors, stillness,
> settle and difference bars) runs as a reading from `live/`, a `mod reading` or the strip tool,
> recorded in the readings log, and is written as a test in Phase 5, after the window, against
> the picks. Per phase the gpu check runs only the `gpu_*` binaries the phase names
> (`test-changed.mjs --binaries`, Plan 0066); a named test over 10 s on lavapipe is born
> `#[ignore = "soak"]` (`docs/nfr.md` section 7). The mechanisms and bars stand; only when they
> become tests moves.

## TL;DR

The renderer shades one light that is not a voxel, the headlamp, and it is bound to the camera.
This plan makes it one entry of **a list of eight analytic lights** (point or spot, colour,
intensity, reach, flags) that the consumer hands the frame every frame. A lit pixel shades every
light in reach **exactly, with one shadow ray each**: no random number, no settle, no dots. The
fly-through drops a light where you look (`1`), carries the newest one in front of the camera
(`2`), cycles its colour (`3`), removes it (`4`), flags it for bounce (`5`) and fires one along
the view ray (`6`), and shows the
count and each mode in the window. The direct part of the frame widens from a white scalar to
rgb, in a target of its own. Then **a `bounce` flag per light** feeds ADR-0043's lamp channel
from the flagged lights, off for dropped lights by default. The first thing anyone sees: press
`1` in the hall with the lamp off, and the floor, the pillars and the far wall are lit that
frame and hold still.

## Context & problem

`project-context.md` (Rendering, P0): "dark scenes with few lights; headlamp, flares, glowing
crystal; emissive voxels as light sources, with emission changeable at runtime". The flare has no
path to the pixels: `LightParams` describes one lamp placed from the camera, `Params` carries its
position and cone (`light_pos`, `light_intensity`, `light_dir`, `cos_inner`, `cos_outer`), and
`light.wgsl`'s `lamp_lit` shades it. The runtime emission change the P0 names is already met:
the store is baked at unit emission and the read multiplies by the dominant material's scale
now (ADR-0046; `a_scale_change_moves_no_bake` in `apps/flythrough/tests/gpu_bake.rs`).

**What the tree gives this plan.** The lamp's estimate is exact up to its shadow ray; its screen
history exists for the half tier's jittered grid and takes this frame's sample as it is when the
lamp changed (`LIGHTING_LAMP_CHANGED`). The owner flew the headlamp in every session without a
flicker complaint about it. The irradiance texture holds the emissive part in rgb (the store's
value on a cube face, a fallback's running mean elsewhere) and the lamp's white part in `a`; the
temporal pass blends only the lamp's part and a fallback's, the a-trous passes store pixels
through, the composite reads the accumulation. The lamp channel (`Lighting::lamp_bounce`, off by
default) shades its probe rays with the headlamp alone, lists the probes within
`Lighting::lamp_reach` of the lamp, and freezes a probe at `probe_freeze` visits while the lamp
is still.

**Every reader of the lamp fields, at 82ea836** (all move to the lights uniform in Phase 1):

| File | What it reads |
|---|---|
| `light.wgsl` | `lamp_lit`, and the lamp's position for the pass's own cull |
| `march.wgsl` | `lamp(p)`, the unshadowed lamp on a glass hit's reflection |
| `gradient.wgsl` | `lamp_lit`, re-shaded against the previous frame's `a` (the gradient mode, not the default) |
| `upscale.wgsl` | `up_lamp` now against `up_tuning.old_*`: the change mask |
| `fsr2/input.wgsl` | `lamp_term` now against `in_lamp.old_*`: the reactive and change mask |
| `probe_update.wgsl` | `lamp_lit` at the probe rays' hits |
| `probe_gather.wgsl` | `probe_beyond_reach`, the lamp's position against `probe.reach` |

**What a lamp shadow ray costs** (`docs/nfr/02-frame-budget.md`; `docs/plans/logs/0051.md`,
loaded, indicative): the light pass with the lamp on and every emissive face capped read 0.113 ms
on the RTX 3080 Laptop at 720p internal on the half tier; the Plan 0041 table's light pass reads
about eight times the RTX's on the Cezanne iGPU. So one light in reach of every lighting pixel is
about 0.1 ms on the dev reference and about 0.8 ms on the slow adapter, projections this plan's
readings replace. Under the store the light pass draws no emissive sample on a cube face, so its
cost at the start commit is the lamp's ray and the fallback hits'.

**The `nfr.md` figures touched:** section 2's lighting line (light, gradient, probes, gather) and
denoise line (temporal, a-trous), and section 3's `Frame::frame_bytes`. The readings go to
`docs/nfr/02-frame-budget.md` and `docs/nfr/03-memory.md`.

## Decision

Build ADR-0044 in four phases. **The list** (Phase 1) replaces the single lamp in every reader,
white only, with its reach, its cull, the upscalers' change mask over the list, and the
fly-through's drop and carry: a walking skeleton that lights a cavern on the frame the key is
pressed. **Colour** (Phase 2) moves the direct part into an rgb target of its own through the
temporal, a-trous and composite passes. **The bounce flag** (Phase 3) generalises the lamp
channel to the flagged lights. **The window** (Phase 4) is the owner's, with lights dropped in
the hall and the level.

We rejected lights as emissive voxels (ADR-0044 A), a stochastic pick among lights (B), hardware
ray queries (C) and baking a still light into the store (D). We rejected a colour-first Phase 1,
because the white list exercises the whole path (upload, cull, shadow rays, history, the change
mask, the window's rows) with no target added, and the layout change is then one phase on its
own.

**The slot rule** (the owner, 2026-11-10). The list has eight slots. While `Lighting::lamp` is
on, the headlamp takes slot 0 and `Frame::set_lights` keeps the first seven lights it is given
in slots 1 to 7. While the lamp is off, `set_lights` keeps the first eight in slots 0 to 7. The
lights past the cap are counted (`Frame::lights_over_cap()`) and not shaded. Turning the lamp on
or off moves every list light by one slot, which the change detection sees as a changed list:
the direct part takes that frame's sample as it is, the same reset the lamp's own switch causes
today.

## Architecture diagram

```mermaid
flowchart LR
    subgraph consumer
        G[game or fly-through: lights this frame; 8 slots, slot 0 the headlamp while it is on]
    end
    subgraph vxe-render
        G --> U[(lights uniform: 8 x Light now, 8 x Light of the older frame, counts, change bit)]
        U --> LP[light pass: per lit pixel, each light in reach: exact term x window, one shadow ray]
        LP --> D[(direct target rgb, lighting grid)]
        D --> TP[temporal: blend as the lamp's part; this frame's sample as it is when the list changed]
        TP --> AT[a-trous over the direct rgb] --> C[composite]
        ST[(baked store, ADR-0046: untouched)] --> TP
        U --> RM[upscalers: change and reactive mask from the list now against the older list]
        U -- bounce flag --> PU[lamp channel: probe rays shade the flagged lights; freeze when all are still] --> PG[gather] --> C
    end
```

## Implementation phases

Each phase ships as its own commit. `dev` implements all dev phases in one session with no
review between phases. The architect reviews once at the end, in a fresh session. Lane rule 9c
applies: exploratory runs on the RTX 3080 Laptop through `VXE_TEST_ADAPTER=rtx`, labelled; every
asserted reading on lavapipe.

**The gates.** Per phase: `node scripts/test-changed.mjs --profile quick`, then `--profile gpu`
under the perf lock (every phase touches rendering), clippy and fmt. At the tip: `cargo nextest
run --workspace --profile gate` under the perf lock, doctests, clippy, fmt, deny, and the doc and
tier gates. The soak tier (`--profile soak --run-ignored only`) and the full tier (`--profile
full --run-ignored all`) run only when the owner asks. **Tables, sweeps and attempts go to
`docs/plans/logs/0058.md`**, started from the readings-log template; the Implementation log
below keeps one row per phase.

**Test files and tiers** (`.config/nextest.toml`, by marks). New GPU tests go in
`apps/flythrough/tests/gpu_dynamic_lights.rs`; the `gpu_` prefix is the classification and
`node scripts/check-test-tiers.mjs` checks it. A test over 30 s on lavapipe (60 s on CI) is born
`#[ignore = "soak"]` and is named as such in its phase. A measurement written as a test goes
`#[ignore]`d inside a `mod reading`; the readings below are fly-through runs, not tests. The
HUD and banner tests are unit tests in `apps/flythrough/src` with no device.

**Test times are estimates**, none measured for this plan: from Plan 0051's and 0057's tests on
lavapipe, a hand-built box renders 2 to 3 s for a few dozen frames at 192 x 108 in the gate's
build, about 4 s for 64 frames, about 16 s for 150; the level loads in about 5 s. Each named
test carries its estimate; `node scripts/test-times.mjs` on the phase's log replaces it.

**The analytic figures the tests use.** The shader's term is `PI * intensity * cone * max(n . l,
0) / d^2`, times ADR-0044's window `(1 - (d / r)^4)^2`. A point light of intensity 1200 at 20
voxels above a floor, reach `sqrt(1200 / 0.1)` = 109.5 voxels: directly below, `E = PI * 1200 /
400 = 9.425`, window `(1 - 160 000 / 1.44e8)^2` = 0.998, so **9.40**; 20 voxels to the side,
`d^2 = 800`, `cos = 0.7071`, `E = PI * 1200 * 0.7071 / 800 = 3.332`, window `(1 - 640 000 /
1.44e8)^2` = 0.991, so **3.30**. At manual exposure 1 the direct part stores these as they are
(under `HDR_MAX`).

### Phase 1: The list, white, with the fly-through's drop and carry
- **Owner skill:** dev
- **What:** ADR-0044's list in every reader of the lamp, white only, the slot rule, the reach
  and the per-pixel cull, the change detection, the upscalers' change mask over the list, and
  the fly-through's `1`, `2`, `6`, HUD row and `--lights N`. No target is added: each light's term is
  summed as luminance into the irradiance's `a`, where the lamp's part is today, so the
  temporal, a-trous and composite passes are untouched.
- **The list:**
  - **`Light`** (`vxe_render::Light`, `#[repr(C)]`, 64 B): `pos: [f32; 3]`, `intensity`,
    `dir: [f32; 3]`, `cos_inner`, `colour: [f32; 3]`, `cos_outer`, `flags: u32` (`ON`,
    `SHADOWS`, `BOUNCE`), three pad words. `MAX_LIGHTS` 8. `Frame::set_lights(&mut self, queue,
    &[Light])` fills the slots by the slot rule above; the headlamp's slot is filled from
    `LightParams` and the camera, as the `Params` lamp fields are today, which leave `Params`.
  - **The uniform** holds the list now, the list of the older frame the upscalers compare
    against (the frame `up_tuning.old_*` and `in_lamp.old_*` name today, which leave those
    structs), both counts, a `changed` word and a `lamp` word (1 when slot 0 is the headlamp):
    `2 x 8 x 64 + 16` = 1040 B. It is written when any field changed (a hash of the list against
    the last frame's). `LIGHTING_LAMP_CHANGED` becomes "the list changed" and keeps its name in
    the shader for Phase 2 to rename.
  - **Shading.** `lamp_lit` becomes `light_lit(i, p, n, ns)` over slot `i`: the term above,
    `cone` from the cosines as today (a point light has `cos_outer` -1 and `cone` 1), the window,
    zero beyond the reach, zero where the shadow ray toward the light finds rock (skipped when
    `SHADOWS` is clear). The light pass loops over the count and sums the lit lights' luminance
    into `a`.
  - **Reach.** `Lighting::light_floor: f32`, default 0.1 (ADR-0044); `reach(i) = sqrt(intensity /
    light_floor)`. A floor of 0 means no reach: the window is 1 and no light is culled, which is
    the lamp's arithmetic at the start commit. The `steps` view counts the shadow rays cast per
    pixel in its `a` as it counts the lamp's today, so the cull is readable.
  - **The other readers.** `march.wgsl`'s `lamp(p)` sums the list in reach, unshadowed as today.
    `probe_update.wgsl` and `probe_gather.wgsl` read the headlamp's slot through the `lamp` word
    (the channel stays the lamp's until Phase 3). `gradient.wgsl` re-shades the summed list
    against the previous frame's `a`, so the gradient mode's lambda is 0 on a still list of any
    length.
  - **The upscalers' change mask.** `up_light_changed` (`upscale.wgsl`) and `light_changed`
    (`fsr2/input.wgsl`) compare the summed unshadowed luminance of the list now against the
    older list's at the pixel, each light inside its own reach and window, on today's two
    thresholds (`change_abs`, `change_rel`). A pixel a dropped, moved, recoloured or removed
    light reaches is marked as a pixel under a stepped lamp is today; a pixel out of every
    changed light's reach is not. With the `changed` word clear for both frames the loop is
    skipped.
- **The fly-through:** `1` adds a white point light of intensity 1200 at the looked-at hit, 2
  voxels off the surface along its normal (a miss drops it 20 voxels ahead); `2` toggles **carry**
  of the newest light: its position is the eye plus 2 m forward and 0.3 m down each frame until
  `2` again, which leaves it where it is. `6` **fires** a white point light of intensity 1200
  from the eye along the view ray at 200 voxels a second (20 m/s; 3.3 voxels a frame at 60 fps),
  its `BOUNCE` flag clear as a dropped light's: its position advances by the frame's time step
  each frame, it stops 2 voxels short of the first surface on that ray (the drop's ray, cast
  once at the key press; an edit on its path afterwards is not seen) or at 400 voxels with no
  hit, and it stays there as a dropped light. Several may fly at once; `2` on a light in flight
  ends the flight and carries it; `4` removes it. The light has no visible body (What this plan
  does NOT do). `--lights N` (headless or windowed) places `N` white
  lights of intensity 600 on a ring of radius 3 m around the spawn pose at eye height, for the
  readings. The HUD's lighting row gains `lights <n>/8 [carry] [fly <m>] over <k>`, where `m` counts the lights in flight and `n` counts the
  headlamp while it is on; the banner names `lights <n>/8` on `1` and `carry on` or `carry off`
  on `2` and `fire` with `lights <n>/8` on `6`. A drop or a shot into a full list (eight with the lamp off, seven list lights with it on) prints
  a line and adds nothing. `Digit1` to `Digit6` are unbound in `apps/flythrough/src` and
  `crates/vxe-app/src` at a8b1a2c and no active plan claims them; `C`, `V` and `Z` are bound
  (fly down, far normal, sharpen).
- **The readings** (fly-through runs): the light pass and the frame at the level's spawn and the
  vein, lamp off at `--lights` 0, 1, 2, 4, 8 and lamp on at 0, 1, 2, 4, 7 (the slot rule), both
  adapters, three quiet rounds where the machine allows, with the pass's shadow-ray count per
  pixel from the `steps` view; the same at `light_floor` 0.1 and 0.025 (the ring's intensity 600
  reaches 77 and 155 voxels; the ring's radius is 30 voxels); the upscale pass under FSR 2 with
  a still list of 0 and 8 (the mask's loop); the hall at exposure 8 with one light at the floor's
  centre, native and FSR 2 at 1.5, 64 against 512 frames (the prediction: 0 difference at
  native).
- **Files touched:** `crates/vxe-render/src/light.rs`, `marcher.rs`, `fsr2.rs`, `lib.rs`,
  `crates/vxe-render/src/shaders/march.wgsl` (`Params`, the lights uniform, `lamp`),
  `light.wgsl`, `gradient.wgsl`, `upscale.wgsl`, `fsr2/input.wgsl`, `probe_update.wgsl`,
  `probe_gather.wgsl`, `apps/flythrough/src/main.rs`, `hud.rs`,
  `apps/flythrough/tests/gpu_dynamic_lights.rs` (new), `gpu_fsr2.rs`, `gpu_hud_golden.rs` and
  the lamp-lit baselines under `tests/golden/` (done-when 3), `docs/plans/logs/0058.md` (new).
- **Done when:**
  1. `a_point_light_gives_the_analytic_irradiance_on_the_floor` asserts, on a closed dark box
     (albedo 0.5, no emitter, lamp off, `--lighting full`, manual exposure 1, no jitter) with one
     list light of intensity 1200 at 20 voxels above the floor's centre, that the direct part
     at the floor pixel under the light reads 9.40 within 2 % and at the pixel 20 voxels to the
     side 3.30 within 2 %, on the first frame. About 3 s (an estimate).
  2. `a_slab_between_the_light_and_the_floor_casts_a_hard_shadow` asserts, on the same box with
     a 4-voxel slab 10 voxels under the light covering half the floor, that a floor pixel under
     the slab reads 0 direct and a pixel one voxel past its edge reads its analytic value within
     2 %, and that with `SHADOWS` clear the shadowed pixel reads its analytic value too. About
     3 s (an estimate).
  3. `the_list_at_no_reach_is_the_lamps_arithmetic`: at `light_floor` 0 with the lamp the only
     light, every gate golden passes against its baseline as blessed at the start commit, with
     no re-bless. At the default floor the lamp is windowed and cut at 109.5 voxels, so the
     lamp-lit baselines are re-blessed once in this phase on lavapipe, and the log names, per
     baseline, the share of pixels that moved and the greatest level. No new test time: the
     golden tests run twice in one binary, about double their time today (an estimate).
  4. `a_light_beyond_its_reach_casts_no_shadow_ray` asserts, in the `steps` view, that a floor
     pixel at 150 voxels from a light of intensity 1200 (reach 109.5) counts 0 shadow rays toward
     it and a pixel at 50 voxels counts 1, and that at `light_floor` 0.025 (reach 219) both count
     1. About 3 s (an estimate).
  5. `the_slots_follow_the_lamp` asserts the slot rule: with the lamp on, `set_lights` with 8
     given keeps 7, `lights_over_cap()` reads 1 and the eighth light's floor pixel reads 0
     direct; with the lamp off the same 8 are all shaded and `lights_over_cap()` reads 0; a
     ninth is over the cap in both. Under 5 s (an estimate).
  6. `a_changed_light_takes_this_frames_sample` asserts that after 32 still frames a light moved
     by 10 voxels gives, on the next frame, a direct part equal to the new analytic value within
     2 % (no blend), that an unchanged list leaves the change bit clear (`lights_changed()`
     false) over 32 frames, and that turning the lamp on sets it for one frame. About 4 s (an
     estimate).
  7. `fsr2_marks_a_changed_list_light_and_keeps_pace` (`gpu_fsr2.rs`) asserts, under FSR 2 at
     1.5 with the lamp off: a still list light leaves the reactive mask as it is with no list
     light; on the frame a light is dropped, every floor pixel within half its reach is marked
     and a pixel beyond its reach is not; and the output at the pixel under the light reaches its
     settled value within the frame count and tolerance that
     `fsr2_keeps_pace_with_a_sweeping_headlamp_and_a_lamp_step` allows a lamp step. The same
     mask assertion runs under `own` in `gpu_upscale.rs`. About 15 s (an estimate from the lamp
     test's size; over 30 s it is marked `#[ignore = "soak"]` and a 16-frame mask-only variant
     stays in the gate).
  8. `the_gradient_mode_holds_still_under_a_still_list` asserts, in the gradient mode with two
     list lights and the lamp off, that the strata's lambda reads 0 at every stratum over 16
     still frames. About 3 s (an estimate).
  9. `the_banner_and_hud_name_the_lights` (`hud.rs`, no device, under 1 s): the lighting row
     reads `lights 3/8 carry over 0` for three lights with carry on and the lamp off, and
     `lights 4/8` for the same with the lamp on; the banner reads `lights 3/8` after `1` and
     `carry on` after `2`; `--lights 3` parses; `1` on a full list is refused with a line that
     names this plan's number. With two lights, one of them in flight, the row reads `lights 2/8
     fly 1 over 0`; the banner reads `fire` and `lights 2/8` after `6`; `6` on a full list is
     refused with the same line.
  10. The readings are in the log, with the shadow-ray count per pixel beside each cost. The
      prediction: the light pass grows by 0.08 to 0.12 ms per light in reach on the RTX and by
      0.6 to 1.0 ms on the Cezanne; a figure outside that range is a reading, not a failure.
  11. `a_light_in_flight_leaves_no_trail_in_the_direct_part` asserts, on done-when 1's box
      (lamp off, no jitter) with one list light of intensity 1200 at 20 voxels above the floor,
      held still for 32 frames and then moved 4 voxels a frame along the floor for 16 frames:
      on every frame of the flight the direct part at the floor pixel under the start position
      and at the pixel under the light's position that frame equals the analytic value for that
      frame's position within 2 %, computed in the test from the term above; on the last frame
      of the flight the start pixel reads **0.185** within 2 %, down from 9.40 (64 voxels to
      the side: `d^2` = 4496, `cos` = 0.2983, `E = PI * 1200 * 0.2983 / 4496` = 0.2501, window
      `(1 - 0.6121^4)^2` = 0.739); `lights_changed()` is true on each frame of the flight and
      false on the second frame after it. The box is at least 100 voxels long. About 4 s (an
      estimate).
  12. `fsr2_leaves_no_trail_behind_a_light_in_flight` (`gpu_fsr2.rs`) asserts, under FSR 2 at
      1.5 with the lamp off and done-when 11's flight: the output's luminance at the pixel under
      the start position never rises from one frame of the flight to the next by more than the
      tolerance `fsr2_keeps_pace_with_a_sweeping_headlamp_and_a_lamp_step` allows, and within
      that test's frame count after the light stops it is within that tolerance of a reference
      settled with the light still at the end position; the floor pixels within half the reach
      of the light's position are marked reactive on each frame of the flight. **The trail's
      length is a reading:** the frames from the light's last move until the start pixel is
      within the tolerance, in the log, on lavapipe and labelled on the RTX. About 15 s (an
      estimate; over 30 s it is marked `#[ignore = "soak"]` and an 8-frame flight stays in the
      gate).

### Phase 2: Colour: the direct part in a target of its own
- **Owner skill:** dev
- **What:** the direct part becomes rgb through the light, temporal, gradient, a-trous and
  composite passes; the fly-through's `3` and `4`; a golden with two coloured lights;
  `frame_bytes`.
- **The direct target** (the contract Plan 0032 builds on; see Data shapes for the same text):
  - **Shape.** The light pass writes a `direct` texture, `rgba16float` on the lighting grid: rgb
    the pre-exposed sum of the frame's exact, analytic direct terms (each list light's term
    times its colour), `a` that sum's luminance. The temporal pass keeps a **direct
    accumulation pair** of the same format and grid. The irradiance texture's `a` goes unused.
  - **Blend.** The lamp's rule of ADR-0018, unchanged: `max(ALPHA, 1 / (age + 1))` on the
    reprojected history, with today's reprojection, edit-box and no-history resets; in the
    gradient mode the weight rises with the strata's lambda as today, and `gradient.wgsl`
    compares its re-shade against the direct target's `a`.
  - **Change rule.** On a frame where `LIGHTING_LIGHTS_CHANGED` is set (the bit
    `LIGHTING_LAMP_CHANGED` holds today, renamed) every pixel takes this frame's sample as it
    is. The bit is set by the list's change detection and by nothing else in this plan. A term
    added to the target that does not raise the bit is blended on the rule above.
  - **Reactive rule.** The upscalers mark a pixel when the unshadowed luminance of the terms
    now and of the older frame's differ past `change_abs` and `change_rel` (Phase 1's mask). A
    term outside the list needs its own now-and-then pair in that comparison to be marked.
  - **Filter and composite.** The moments' luminance is the direct's `a` plus the emissive's as
    today. The a-trous filters the direct rgb (its variance from the same moments) and passes
    the store's emissive through as today. The composite adds the filtered direct rgb to the
    emissive before the albedo.
- **Arithmetic** at 1280 x 720 internal on the half tier (640 x 360 lighting pixels, 8 B a
  pixel): the direct target 1 843 200 B, the pair 3 686 400 B, **5 529 600 B** more in
  `frame_bytes`.
- **The fly-through:** `3` cycles the newest light's colour through white, warm (1.0, 0.7, 0.4),
  red, green, blue; `4` removes the newest light. The HUD row names the newest light's colour;
  the banner names the colour on `3` and `lights <n>/8` on `4`.
- **Files touched:** `crates/vxe-render/src/shaders/light.wgsl`, `temporal.wgsl`,
  `gradient.wgsl`, `atrous.wgsl`, `composite.wgsl`, `march.wgsl` (the bit's name, `lamp` in
  rgb), `crates/vxe-render/src/marcher.rs` (targets, `frame_bytes`), `apps/flythrough/src/main.rs`,
  `hud.rs`, `apps/flythrough/tests/gpu_dynamic_lights.rs`, `gpu_golden.rs` and a new baseline
  under `tests/golden/`, `gpu_upscale.rs` and `gpu_fsr2.rs` (`frame_bytes`),
  `docs/plans/logs/0058.md`.
- **Done when:**
  1. `two_coloured_lights_sum_per_channel` asserts, on Phase 1's box with a red light (1, 0, 0)
     and a blue light (0, 0, 1) of intensity 1200 each at 20 voxels above the floor and 20 voxels
     apart, that the floor pixel under the red light reads direct `(9.40, 0, 3.30)` within 2 % per
     channel and the pixel under the blue one `(3.30, 0, 9.40)`. About 3 s (an estimate).
  2. `lights_box_golden_holds_two_coloured_lights`: a golden of the box at 192 x 108 with the
     red and the blue light, lamp off, `--lighting half`, blessed once on lavapipe, on the
     harness's floor (at most 0.1 % of pixels differ by more than 2/255 in any channel). About
     3 s (an estimate).
  3. `a_dropped_light_holds_still_while_the_camera_moves` asserts, on the box with the lamp off
     and one still list light, that over a 32-frame sideways step of 0.5 voxel a frame the direct
     accumulation at a reprojected floor pixel never differs from its analytic value by more
     than 2 % (the blend never sees a sample that differs). About 4 s (an estimate).
  4. `frame_bytes_hold_the_direct_pair` in `gpu_upscale.rs` asserts that the frame's inventory
     at 1280 x 720 internal on the half tier counts the direct target and its pair at 5 529 600 B
     and that `frame_bytes` equals the inventory's sum; `gpu_fsr2.rs`'s inventory test gains the
     same three targets. Under 5 s each (an estimate).
  5. Phase 1's done-whens 6, 7, 8, 11 and 12 hold on the direct target (the bit renamed), and no gate
     golden without a list light is re-blessed in this phase.
  6. The HUD and banner tests cover `3` and `4` (no device, under 1 s): the banner reads the
     colour's name after `3` and `lights 2/8` after `4` on three lights.
  7. The readings: the temporal and a-trous passes' cost at the spawn, lamp on, `--lights` 0 and
     4, both adapters, against Phase 1's (the prediction: the a-trous grows by under 0.1 ms on
     the RTX for three channels against one, a guess); the hall's 64 against 512 frames with two
     coloured lights, native (the prediction: 0).

### Phase 3: The bounce flag
- **Owner skill:** dev
- **What:** the lamp channel generalised to the lights with `BOUNCE` set, and two of the
  backlog's lamp-channel faults (the furnace's `D`, the one-probe step test).
- **Two reaches, named.** A light's **direct reach** is ADR-0044's `sqrt(intensity /
  light_floor)`: it windows the term wherever the light is shaded, at a pixel or at a probe
  ray's hit. The **bounce reach** is `Lighting::lamp_reach` (64 voxels by default), the same
  for every flagged light: it decides which probes a light lists and it sizes the volume
  (`probes_per_axis`), as it does for the lamp today.
- **The channel:** the probe rays shade their hits with every flagged light whose direct reach
  covers the hit, summing the terms as the light pass does (rgb after Phase 2); the probes
  listed are those within the bounce reach of any flagged light (the union;
  `probe_gather.wgsl`'s `probe_beyond_reach` tests every flagged light); a probe's mean freezes
  at `probe_freeze` visits while every flagged light within its bounce reach is still, and
  blends at `Lighting::probe_move_blend` while one moves; with no flagged light on, no probe
  pass is encoded. `Lighting::lamp_bounce` stays the channel's switch and the headlamp's flag.
  **The volume still scrolls with the camera** (`ProbeVolume::origin_for`): a flagged light farther
  from the camera than the volume's half-side plus the bounce reach lists no probe and adds no
  bounce, and its direct light is unchanged. The fly-through's `5` toggles the newest light's
  flag; the HUD row marks a flagged light `*`; the banner reads `bounce on` or `bounce off`.
- **Files touched:** `crates/vxe-render/src/shaders/probe_update.wgsl`, `probe_gather.wgsl`,
  `crates/vxe-render/src/probes.rs`, `marcher.rs`, `apps/flythrough/src/main.rs`, `hud.rs`,
  `apps/flythrough/tests/gpu_dynamic_lights.rs`, `gpu_indirect.rs`, `docs/plans/logs/0058.md`.
- **Done when:**
  1. `a_flagged_light_feeds_the_channel_and_an_unflagged_one_does_not` asserts, on a closed box
     of albedo 0.5 with the lamp off and one list light at the centre: with `BOUNCE` clear every
     probe texel reads exactly 0 over 64 frames and no probe pass is encoded; with `BOUNCE` set
     the centre probe's texel mean is above 0 after 16 frames. About 4 s (an estimate).
  2. `a_flagged_lights_furnace_reads_its_albedo_times_the_analytic_direct_mean`, born
     `#[ignore = "soak"]` (256 frames, as the lamp furnace): on the same box with `BOUNCE` set
     and `light_floor` 0, the centre probe's texel mean after 256 frames reads within 4 % of
     `0.5 D_ref`. `D_ref` is computed on the CPU, in the test, from the box's geometry alone:
     the analytic term `PI * intensity * cos / d^2` at the wall point each direction from the
     probe meets, cosine-weighted over a quadrature of at least 4096 directions, in the
     normalisation `the_lamp_furnace_reads_its_albedo_times_the_direct_mean` documents. No
     value read back from the shader enters `D_ref`. The lamp furnace takes the same CPU
     reference for a spot light in this phase.
  3. `the_channel_freezes_when_every_flagged_light_is_still` asserts, at `probe_freeze` 8 with a
     flagged list light and the lamp off, that after 8 visits the texels of every probe within
     the bounce reach are bit-identical over 64 frames, that moving the light changes every
     probe within the bounce reach of its old or new position within 8 frames and leaves the
     others bit-identical, and that they freeze again once it is still. About 16 s (an
     estimate).
  4. `a_light_beyond_the_bounce_reach_lists_no_probe` asserts that a flagged light of intensity
     1200 placed 80 voxels from a probe (beyond the bounce reach of 64, inside the direct reach
     of 109.5) leaves that probe at 0 and unvisited, and that a flagged light placed outside the
     volume by more than the bounce reach encodes no probe pass. About 4 s (an estimate).
  5. The HUD and banner tests cover `5` and the `*` (no device, under 1 s): the banner reads
     `bounce on` after `5` and `bounce off` after a second.
  6. The readings: the probe update's cost at the spawn with the lamp on and 0, 1 and 4 flagged
     lights in reach, both adapters, against the channel's cost at the start commit (the
     prediction: the update grows with the flagged lights' shadow rays per probe ray, about
     linearly); the count of probes inside rock traced while a light moves (the backlog's third
     fault, a reading); **the bounce's lag behind a light in flight:** with a flagged light on
     Phase 1's done-when 11 flight, the frames from its last move until the mean of the probe
     nearest the end position is within 10 % of its frozen value (a reading at the default
     `probe_move_blend`, no bar).

### Phase 4: The owner's window
- **Owner skill:** human
- **What:** the release fly-through on the RTX, one sitting, Phase 1 to 3's tables at hand.
  Every toggle shows in the banner and the HUD.

  ```
  cargo build --release -p flythrough

  # 1. The hall, lamp off: drop a light at the floor's centre (1), count to five, a fast turn,
  #    a slow pan; carry it (2) down the hall and set it down; a second and third light; colours (3).
  target/release/flythrough --source showcase --shot hall --exposure 8 --hud --lamp off

  # 2. The level: drop a light in the vein's passage, dig next to it, dig a hole and drop a
  #    light into it; carry one through the passage with the lamp on and off.
  target/release/flythrough --level --seed 1 --pose vein --hud

  # 3. Bounce: the same light with its flag on (5), beside it off; the lamp's bounce beside it.
  target/release/flythrough --level --seed 1 --pose vein --hud --lamp-bounce on

  # 4. Eight lights on the ring, lamp off, for the cost on screen.
  target/release/flythrough --level --seed 1 --lights 8 --lamp off --hud

  # 5. A light in flight (6): down the hall's length with the lamp off; along the vein's passage
  #    and across the cavern, camera still and then following it; three in a row; one with the
  #    bounce flag set after it lands (5).
  target/release/flythrough --source showcase --shot hall --exposure 8 --hud --lamp off
  target/release/flythrough --level --seed 1 --pose vein --hud --lamp off
  ```
- **Files touched:** this plan's log.
- **Done when:** the log records the owner's words on: (a) whether anything moves with a dropped
  light and a still camera, hall and level, under FSR 2 at 1.5; (b) the carried light in motion,
  against the headlamp; (c) the frame of the drop (does the cavern light "that frame"); (d) hard
  shadows and the reach's edge at `light_floor` 0.1, the headlamp's included; (e) the bounce
  flag's look and lag, and whether dropped lights should default to it; (f) eight lights' cost on
  screen; (g) whether the defaults are accepted; (h) the light in flight under FSR 2 at 1.5:
  whether the walls it passes are lit that frame, whether a trail stays behind it, and whether
  the lit walls shimmer while it flies. **Stop rule:** a "no" on (a), (b) or (h) that names
  shimmer or a trail under FSR 2 stops the close and goes to the architect with Phase 1's mask
  readings and done-when 12's trail length; the narrowed reset under Risks is the first amendment. A "no" on (g) names the run
  and goes to the architect with the tables. The plan closes with that record.

### Phase 5: The picks as tests

- **Owner skill:** dev
- **What:** the image tests, goldens and bars Phases 1 to 3 name, written now with the
  values the Phase 4 window settled, and the readings they replace cited from the log.
- **Files touched:** the `gpu_*` test files and `golden/` images Phases 1 to 3 name,
  `docs/plans/logs/0058.md`.
- **Done when:** every image test Phases 1 to 3 name exists, asserts the picked value or
  the bar the plan states (a bar the window moved says so in the log, with the owner's words),
  and passes in the gpu tier under the lock; each one over 10 s on lavapipe is
  `#[ignore = "soak"]` and passes once by name. A test for a mechanism the window dropped
  is not written, and the log names it.

## Data shapes

```rust
// illustrative: vxe-render
#[repr(C)]
pub struct Light {
    pub pos: [f32; 3],      // voxels
    pub intensity: f32,     // radiance on the axis at one voxel, as LightParams::intensity
    pub dir: [f32; 3],      // unit; ignored for a point light
    pub cos_inner: f32,
    pub colour: [f32; 3],   // linear, multiplies the term
    pub cos_outer: f32,     // -1 for a point light
    pub flags: u32,         // ON | SHADOWS | BOUNCE
    pub _pad: [u32; 3],
}
pub const MAX_LIGHTS: usize = 8;
impl Frame {
    /// Lamp on: the first 7 go to slots 1..=7. Lamp off: the first 8 go to slots 0..=7.
    pub fn set_lights(&mut self, queue: &wgpu::Queue, lights: &[Light]);
    pub fn lights_over_cap(&self) -> u32;
    pub fn lights_changed(&self) -> bool;
}
pub struct Lighting { /* as today, plus: */ pub light_floor: f32 /* 0.1; 0 is no reach */ }
```

```wgsl
// illustrative: the lights uniform, 2 x 8 x 64 B + 16 B = 1040 B
struct Lights {
    l: array<Light, 8>,     // this frame
    old: array<Light, 8>,   // the older frame the upscalers compare against
    count: u32, old_count: u32, changed: u32, lamp: u32,
};
```

**The direct target, for the plans that add a term to it.**
[Plan 0032](0032-sun-and-sky-hillaire-atmosphere-time-of-day-and-an-outdoor-testbed.md)'s sun is
not a list entry (the list has no directional kind and ADR-0044 adds none); its sun and sky
terms are added to this plan's direct target. What that target is at this plan's close:

> `direct` is `rgba16float` on the lighting grid: rgb the pre-exposed sum of the frame's exact
> direct terms, `a` its luminance; a direct accumulation pair of the same format and grid holds
> its history. The temporal pass blends it at `max(ALPHA, 1 / (age + 1))` with the lamp's
> reprojection, edit-box and no-history resets; on a frame where `LIGHTING_LIGHTS_CHANGED` is
> set every pixel takes this frame's sample as it is, and only the list's change detection sets
> that bit. The upscalers mark a pixel changed and reactive when the unshadowed luminance of the
> list now and of the older list differ at it past `change_abs` and `change_rel`. The a-trous
> filters the rgb on the shared moments; the composite adds it to the store's emissive before
> the albedo. A term added without raising the bit is blended on the same rule and is not in
> the upscalers' comparison until its writer adds a now-and-then pair there.

## Budgets

The base for every row is a reading at this plan's start commit (Plans 0060 and 0061 land
first), taken with the command in the table before Phase 1's first change and written to the
log. Plan 0059's close readings are the last on record and say what to expect of that base:
`docs/nfr/02-frame-budget.md` and `docs/nfr/03-memory.md` (the loaded-light rows), with the runs
in `docs/plans/logs/0059.md`. `nfr.md` section 2's upscale line is already over under the FSR 2
default; this plan's reading there is its delta.

| `nfr.md` figure | Base | Expected | How the close measures it |
|---|---|---|---|
| §2 lighting line: light + gradient + probes + gather | the start commit's reading | light pass **a projection:** + 0.08 to 0.12 ms per light in reach on the RTX 3080 Laptop, + 0.6 to 1.0 ms on the Cezanne RADV, read lamp off at `--lights` 0, 1, 2, 4, 8; probes grow with the flagged lights (Phase 3's reading) | `flythrough --level --seed 1 --headless --render 1920x1080 --frames 2700 --adapter NVIDIA\|AMD --lights N [--pose vein]`, three quiet rounds, the perf lock, the memory cap, per pass |
| §2 denoise line: temporal + a-trous | the start commit's reading | a-trous over three channels: **a guess** of under + 0.1 ms on the RTX | the same runs |
| §2 upscale line (over at the base) | the start commit's reading | the mask's loop over a still list: **a guess** of under + 0.05 ms on the RTX at 8 lights, skipped at 0 | the same runs, `--lights` 0 and 8 |
| §3 `Frame::frame_bytes` | the start commit's figure at the run's resolution | **arithmetic:** + 8 B x 3 per lighting pixel (5 529 600 B at 1280 x 720 internal, half tier); the lights uniform + 1040 B | `frame_bytes` asserted in `gpu_upscale.rs` and `gpu_fsr2.rs`, printed at the close |

**Review note (2026-11-10, the static performance review; estimates, nothing was run).** The
per-light projection above comes from one loaded reading of the headlamp: a spot cone at the
eye, whose shadow rays retrace the open path the primary ray crossed. A dropped point light
has no cone and its shadow rays are incoherent across the cave, so its cost may sit nearer a
share of the hit pass per light than 0.1 ms: **a guess of 0.15 to 1.1 ms each in the hall on
the RTX**. The close's readings therefore add one arm to the table's command: `--lights` 1, 4
and 8 as **dropped point lights at the hall**, beside the vein. A reading over 0.3 ms per
light on the RTX is not a failure; it goes to the owner with the cap the render-profile plan
would set (`docs/backlog.md`). If
[Plan 0065](0065-the-baked-lights-waste-atomics-a-filter-of-zeros-and-a-scan-per-frame.md)
has landed, the a-trous row's base is its early-out, which a moving light switches off.

Determinism is not touched: everything is in `vxe-render` and the fly-through. The list's
shading draws no random number, so two runs of the same still frame are bit-identical in the
direct part, which the close reads on lavapipe.

## Risks & open questions

- **Eight lights on the slow adapter.** Eight in reach of every pixel project to about 7 ms of
  shadow rays on the Cezanne. The reach, the window and the readings at `--lights 8` bound it;
  the portable profile may cap the count, which is a settings row, not this plan.
- **The reset on a change exposes the half tier's grid.** A carried or flickering light resets
  the direct blend on every pixel each frame, as the flying headlamp does today. If the owner
  sees shimmer in Phase 4, the reset narrows to the changed lights' reach (a per-pixel test of
  the changed lights' windows), the amendment Phase 4's stop rule names.
- **A carried light under FSR 2.** The change mask marks every pixel a moving light reaches,
  each frame it moves; FSR 2's history is clipped there as under the sweeping headlamp, which the
  owner accepted for one lamp. Several carried lights widen that area. Phase 1's done-when 7
  bounds one light; Phase 4 (a) and (b) judge the rest.
- **A light in flight is the worst case of the two risks above.** It changes the list on every
  frame for a second or two and sweeps its reach across the walls: the direct part there is one
  unblended sample a frame on the half tier's grid, and FSR 2 holds no history under it. Exact
  shading means no trail can come from the direct part (Phase 1 done-when 11); a trail or a
  shimmer can only come from the upscaler and the half grid, which done-when 12 reads and Phase
  4 (h) judges. The bounce of a flagged light in flight lags by the channel's blend (Phase 3's
  reading); the default leaves a fired light unflagged.
- **The headlamp gains a reach.** At the default floor its light ends at 109.5 voxels where
  today it falls off without end; Phase 1 re-blesses the lamp-lit goldens for it and Phase 4 (d)
  judges it. If the owner refuses the edge, the lamp's slot takes floor 0 and the list keeps its
  reach.
- **Hard shadows.** A point light's shadow has no penumbra. The owner judges it in Phase 4; a
  soft shadow is stochastic and is not offered here (ADR-0044).
- **Glass.** `march.wgsl`'s `lamp(p)` sums the list unshadowed on a glass hit's reflection, as
  it does the lamp today; a light behind a wall therefore shows in a pane's reflection. No bar
  here; a shadowed term there is a followup if the window shows it.
- **The lights uniform and `Params`.** Moving the lamp fields out of `Params` touches the seven
  readers in the Context's table; the gate's goldens at floor 0 (Phase 1 done-when 3) catch a
  slip.
- **Plan 0061's tip.** Its store read by the far normal and its removed emitter cap change
  `temporal.wgsl` and `composite.wgsl` under Phases 1 and 2; the header's re-read is the guard.

## What this plan does NOT do

- More than eight lights, a stochastic pick among lights, or a tiled cull: ADR-0044 B, when a
  consumer asks.
- A directional light. The sun is Plan 0032's, added to the direct target, not to the list.
- Soft shadows or area lights.
- Hardware ray queries: ADR-0044 C, an opt-in tier with its own ADR if a reading asks for it.
- Anything in the baked store, its bake or its queue (ADR-0046, Plans 0059 and 0060). A list
  light is never baked. The emission scale over the store is built (Plan 0059); a per-emitter
  scale is a later ADR.
- Indirect light from emissive materials: the flag per emissive material into the channel is
  the next step after this plan, when the owner asks (`docs/backlog.md`, "Per-source indirect
  light").
- A channel volume that follows a light instead of the camera, or a second volume.
- Light persistence, replication or physics: the consumer's; a light that is part of the world
  is an emissive voxel through an op, as today. The testbed flies a light on a straight line
  at one speed (`6`); gravity, an arc, a ricochet, a light that dies on impact and collision
  with anything that is not a voxel are the consumer's motion.
- A visible body for a light. A list light lights surfaces and is itself not drawn; a glowing
  projectile is an entity with an emissive model (M6) that also writes a slot.
- Entities (M6) carrying lights: they write a slot each frame; the entity itself is M6.
- Volumetrics (M8): a light in fog is M8's.

## Owner answers

2026-11-04, at the draft:

1. **The cap:** 8 lights, one shadow ray each, exact (not 32 with a reservoir pick, not 4).
2. **The bounce:** a phase of this plan, the flag off for dropped lights by default.
3. **The testbed:** drop at the looked-at point and a carry mode; no thrown arc.

2026-11-10, at the review of the active plans:

4. **The slots:** eight, and the headlamp takes slot 0 only while it is on: eight list lights
   fit with the lamp off, seven with it on.
5. **The order:** Plan 0060, Plan 0061, this plan, then Plans 0044 and 0045.

2026-11-10, after the review ("I want for example to fire a projectile and this projectile
will be lighting up walls of cave"; "yes, amend the plan"):

6. **A light in flight:** the fly-through fires a light along the view ray (`6`), and the plan
   reads the trail behind it. This replaces "no thrown arc" in answer 3 as far as a straight
   flight; an arc stays the consumer's.

The "go" is given (owner, 2026-11-10: "move all draft plans to approved"); the lane starts
after the architect's re-read against Plan 0061's tip. The keys (`1` to `6`)
are the architect's picks from the unbound ones and dev may move them if a binding collides.

## Implementation log

> Written by `dev`: one row per phase as its commit lands, then the close block. **The phases
> above are the contract. This section records what happened.** Observations, never conclusions:
> no pass list, no self-assessment. Deviations and unmet done-whens are **always** disclosed.
> Keep it shorter than `## Implementation phases`. Readings, gate counts, attempts, sweeps and
> resume notes go to `docs/plans/logs/0058.md`.

| phase | owner | state | commit |
|---|---|---|---|
| 1: The list, white, with the fly-through's drop and carry | dev | not started | |
| 2: Colour: the direct part in a target of its own | dev | not started | |
| 3: The bounce flag | dev | not started | |
| 4: The owner's window | human | not started | |
| 5: The picks as tests | dev | not started | |

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
