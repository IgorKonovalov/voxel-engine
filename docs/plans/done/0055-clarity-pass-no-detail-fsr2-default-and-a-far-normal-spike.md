# 0055: The clarity pass before Plan 0051: procedural detail removed, FSR 2 at 1.5 the default, and a spike for a footprint-blended far normal

> **Status:** done (2026-11-03, v0.40.0): closed clean after two fix passes, the last on the close review's
> two minors; procedural detail is gone, FSR 2 at 1.5 is the fly-through's default on the owner's word,
> and the far normal is on at `[2, 6]`, provisional, reopened after Plan 0051; ADR-0042 accepted
> **Created:** 2026-11-03
> **Milestone:** M2 in [`docs/roadmap.md`](../../roadmap.md) (the surface-look and upscaler
> bullets; Plan 0051 and the light plans after it read their bars on this plan's default look)
> **Related ADRs:** [ADR-0011](../../adrs/0011-contour-planes-in-attribute-pages-with-procedural-detail.md)
> (its procedural detail: an Outcome addendum at this plan's close says it is removed),
> [ADR-0034](../../adrs/0034-fsr-2-ported-to-wgsl-as-the-default-upscaler-candidate.md) (an
> Outcome addendum at the close: FSR 2 at 1.5 becomes the fly-through's default, our upscaler
> stays opt-in), [ADR-0024](../../adrs/0024-own-temporal-upscaler-on-the-hit-buffer.md) (our
> upscaler, kept opt-in), [ADR-0041](../../adrs/0041-the-surface-is-the-trilinear-zero-set-of-the-27-count-field.md)
> (its "tilted by ADR-0011's per-material detail" clause falls with the detail). If Phase 4
> accepts the far normal, the architect writes ADR-0042 at the close (the rejected
> alternatives are named under Decision); no ADR is written for a spike that fails.
> **Source:** the owner's baseline of 2026-11-03 (`renders/baseline-2026-11-03/HANDOFF.md`,
> `notes.md`, the window checks in `session-checks.log`) and the headless strips the same
> afternoon (`renders/strips/NOTES.md`, all local and gitignored). Owner decisions of
> 2026-11-03: this plan runs before Plan 0051 ("we cannot proceed until we fix it"); rock
> detail is removed, not defaulted off; the whole detail mechanism goes; our upscaler stays
> opt-in; the far normal starts from the 27-count gradient blended by footprint, judged from
> strips and then one windowed session; the Radeon reading of FSR 2 is informational.
> **Starts:** now, alone on main. Plan 0051 starts after this plan closes and reads its
> Phase 1 tables on this plan's default look.

## TL;DR

Three of the five faults the owner saw on 2026-11-03 are not light faults, and Plan 0051
would read its light bars through them. This plan removes them first. **Procedural detail
goes** (ADR-0011's per-material tilt and albedo noise, used by rock alone): it reads as a blur
under the headlamp and saves nothing when off. **FSR 2 at scale 1.5 becomes the
fly-through's default** (ADR-0034's candidate): our upscaler blends display-encoded colour
after the tone map and brightens and widens thin emissive lines, FSR 2 blends linear light
before it, and the owner's eye and the flying strips both prefer it. **A spike tests a far
shading normal:** the face normal where a voxel is big on screen, the existing 27-count
gradient where it is small, blended by the voxel's projected size, because the swirling
contour lines on every far wall are real risers of a 10 cm staircase seen at two pixels'
spacing, which no upscaler can remove and smooth shading up close turns to plastic. The owner
picks from headless strips, confirms in one window, and Phase 5 makes the pick the default
or removes the code. The first thing anyone sees: the fly-through starts at the vein pose
with no detail and FSR 2, the look of the baseline's runs B, E and 4 in one.

## Context & problem

The owner's windowed baseline (2026-11-03) named five faults: a blur under the headlamp,
two flickers after the lamp goes off, bright dots when digging or moving fast, thin lines
that read super bright, and swirling contour bands on far ceilings and walls. The window
checks and the strips split them by cause:

- **The blur** improved with rock detail off (run B) and much more with our upscaler off
  (run E). Removing detail saves about 0.08 ms at the showcase pose (8.785 against 8.706 ms,
  RTX 3080 Laptop, 2530 x 1378), so it is a look call, and the owner made it.
- **The bright thin lines** survived sharpening off (G) and full-resolution lighting (H) and
  faded at native resolution (I). `upscale.wgsl` blends the composite's display-encoded
  colour after the tone map; the FSR 2 path takes linear HDR before it (ADR-0034). A clipped
  one-pixel emissive seam averaged in display space over a 3 x 3 kernel and a Catmull-Rom
  history is wider and brighter than the same seam averaged in linear light. The owner's
  anti-aliasing check preferred FSR 2 at 1.5 (run 4); the flying strips (`renders/strips/fly-*`,
  4 m/s with a 12 degree per second yaw) show crisp ore edges and no trails under FSR 2,
  more far-wall speckle under ours, and raw aliasing with none.
- **The far lines** stayed under every upscaler setting and vanished under both smoothed
  normal modes (`gradient27`, `trilinear8`). A dome built of 10 cm cubes is a set of
  contour lines at 10 cm height spacing: each riser a dark line, each tread a lit strip.
  Near, they are cube edges; far, a topographic map; below about two pixels of spacing they
  alias into swirls. The a-trous filter never blurs across them (its normal weight is
  `max(n.n', 0)^128`). The fix is the shading normal, and only where the voxel is small on
  screen: whole-frame `gradient27` reads plastic up close (the owner's word) and costs
  +1.6 ms (12.032 against 10.442 ms under FSR 2 at the showcase pose, the hit pass 4.339
  against 2.818). Measured on the strips at the hall shot (1920 x 1080, FSR 2 at 1.5, RTX):

  | crop of the hall shot | face, mean neighbour luma difference (h / v) | gradient27 (h / v) |
  |---|---|---|
  | the dome, rows 0..250, columns 800..1500 | 0.00547 / 0.00725 | 0.00222 / 0.00223 |
  | the near pillar, rows 300..700, columns 560..760 | 0.00254 / 0.00249 | 0.00180 / 0.00183 |

  The dome loses 60 % of its high-frequency energy under `gradient27`; so does the near
  pillar lose 29 %, which is the plastic. A footprint blend should take the first and leave
  the second.
- **The two flickers and the dots** are light faults and stay with Plan 0051. The strips
  attribute the lamp-off settle to the probe volume (indirect off: settled in one frame;
  on: 1.5 to 2 s with a non-monotonic wander), which that plan's amendment takes up.
- **The far-wall blotches** in the showcase survive indirect off, the filter off,
  `gradient27`, the flat-list sampler, 32 candidates, the full shadow walk and 900 settled
  frames, over uniform albedo and visible wall relief: converged direct light, the soft
  shadows of the wall's own relief under a low cluster of small emitters. Not this plan's.

Plan 0051's Phase 1 reads "today on the default look", and its dots and shimmer readings
pass through the upscaler. Changing the look after it voids those tables, so this plan runs
first. The `nfr.md` figures touched are section 2's frame line and its upscale stage, and
section 3's `frame_bytes`.

## Decision

**Phase 1 removes ADR-0011's procedural detail entirely**: `Material::detail` and
`detail_fine`, the detail flag and the detail-after-lighting tuning, the tilt and the lamp
ratio in the shaders, the testbed's flags and key, and the sharpness harness that measured
them. The material row keeps its GPU layout; the detail lanes become spare. We rejected
defaulting it off because it saves nothing, is code to maintain, and nothing but rock used
it. **Phase 2 makes FSR 2 at scale 1.5 the fly-through's default** with RCAS at the
fly-through's sharpening 0.5; our upscaler stays selectable (`--upscaler own`, `F10`) and its
removal is the structure pass's (Plan 0045), not this plan's. We rejected removing it here
because the Radeon has no FSR 2 at 1.5 reading at walk speed yet. **Phase 3 builds the far
normal as a spike**: the march blends the face normal toward `gradient27_normal` by the
voxel's projected size in pixels, two thresholds as a knob, the 26 neighbour reads skipped
where the weight is zero. We rejected a mip-based normal first because it needs new storage
and a pass before anything is on screen (it stays ADR-0042's alternative if the blend
disappoints); we rejected supersampling or more temporal filtering because the lines are
geometry, not sampling error; and the field surface (ADR-0041) because the owner priced it
out. **Phase 4 is the owner's pick** from strips and one window, which also gives the
slow-pan verdict ADR-0034 is owed. **Phase 5 makes the picks the defaults, or removes the
spike's code**, re-blesses the goldens and re-shoots the hero.

## Architecture diagram

```mermaid
flowchart LR
    subgraph vxe-render
        H[hit pass: march.wgsl] -->|face normal nf, t| B{voxel size on screen\npx = W / ((tan_l + tan_r) t)}
        B -->|px >= hi| F[nf]
        B -->|px <= lo| G[gradient27_normal\n26 occupancy reads]
        B -->|between| M[normalize mix by smoothstep]
        F & G & M --> N[hit_info: octahedral normal]
        N --> L[light, temporal, a-trous, composite]
        L --> U[FSR 2 at 1.5: linear HDR in, tone after]
    end
    U --> P[present]
```

## Implementation phases

Each phase ships as its own commit. `dev` implements all phases in one session with no review
between phases; Phase 4 is the owner's, and dev stops there with a ping. The architect
reviews once at the end, in a fresh session.

### Phase 1: Procedural detail removed
- **Owner skill:** dev
- **What:** ADR-0011's per-material procedural detail is gone from the world, the renderer
  and the testbed; the fly-through starts in `face` with nothing tilting or modulating a
  cube face.
- **What goes (the names dev finds in the tree win):** `Material::detail`, `Material::detail_fine`
  (`crates/vxe-world/src/material.rs`); the material row's detail lanes in the mirror upload
  (`gpu_world.rs`, the lanes stay, written zero, named spare in `march.wgsl`'s binding
  comment); `detail`, `DETAIL_TILT`, `detail_after_lighting`, `detail_lamp_ratio`, flag bits
  4 and 13 (`march.wgsl`, `composite.wgsl`, `probe_gather.wgsl`); `LookParams::detail`,
  `SurfaceTuning::detail_after_lighting` (`marcher.rs`); `ROCK_DETAIL`, `look_materials`,
  `look_materials_with`, `LookPreset::rock_detail`, `LookPreset::detail_fine`, the preset
  `plain` as a distinct thing (`lib.rs`); `--detail`, `--rock-detail`, `--detail-fine`,
  `--detail-after`, `--look plain`, the `V` key and the banner's detail words (`main.rs`,
  `hud.rs`, `cave.rs`); `apps/flythrough/tests/sharpness.rs` whole (its candidates and
  detail-after-lighting claims have no subject) and its `binary_id` in every copy of the gpu
  filter in `.config/nextest.toml`; `every_mode_with_and_without_detail` becomes
  `every_mode` (`tests/look.rs`); the detail parts of `tests/support/mod.rs`,
  `tests/golden.rs`, `tests/fsr2.rs`; `VXE_STRIP_ROCK` in `tests/strip.rs`.
- **Check first:** whether `Material` rows reach a file format (the op log, a save, a prefab:
  ADR-0014). If they do, the format gains a version and a reader for the old rows, and the
  log says so; if not, the log says that.
- **Files touched:** the files above, `docs/plans/logs/0055.md` (new, from the readings-log
  template).
- **Done when:**
  1. `rg -n "detail" crates apps` finds only the LOD sense ("full detail", `r_full`) and the
     documentation of removed things in `docs/`; a one-line listing of the survivors is in the
     readings log.
  2. The goldens that changed (`golden.rs`'s showcase hall and any look golden with rock in
     frame) are re-blessed on lavapipe, each named in the readings log with its fraction of
     pixels changed over tolerance 2; the hall golden's hue-class test still holds.
  3. The fly-through's banner at start reads `look face` with no detail words, and `N` still
     cycles the looks (an arg-parse and banner test, under 1 s).
  4. The hit pass at the showcase start pose (headless, 2530 x 1378 from 1687 x 919, RTX,
     perf lock, median of 30 frames) is within 0.1 ms of 2.818 ms (the baseline's face hit);
     the reading goes to `docs/nfr/02-frame-budget.md`.
  5. `node scripts/check-test-tiers.mjs` exits 0 with `flythrough::sharpness` gone.

### Phase 2: FSR 2 at 1.5 the fly-through's default
- **Owner skill:** dev
- **What:** `flythrough` with no upscaler flag starts in FSR 2 at scale 1.5 with RCAS at
  sharpening 0.5; `--upscaler own` and `off` keep their meanings; `F10` cycles as before.
  The library's `Frame` default (Plan 0024's frame) does not change.
- **Files touched:** `apps/flythrough/src/main.rs` (the default mode, the doc comment), the
  arg-parse tests that assert the default, `apps/flythrough/tests/upscale.rs` or `fsr2.rs`
  where a test names the fly-through's default, `docs/nfr/02-frame-budget.md` and the memory
  readings file for `frame_bytes`.
- **Done when:**
  1. The parse tests assert: no flag gives `fsr2` at 1.5 with sharpening 0.5; `--upscaler own`
     gives ours at 1.5 jittered; `--internal-scale 1` alone gives `off` (under 1 s).
  2. **Cost readings, informational, in the readings log and `docs/nfr/02-frame-budget.md`:**
     `flythrough --headless --render 2530x1378 --frames 300` at the showcase start and at
     `--level --pose vein`, each under `--upscaler fsr2` and `--upscaler own`, GPU frame median
     of the last 30 frames, on the RTX 3080 Laptop and the RADV RENOIR iGPU (`--render
     1920x1080` there), perf lock, load under 1.5. The RTX showcase pair is expected near the
     baseline's 10.442 against 8.706 ms. The upscale stage's 1.5 ms line in `nfr.md` section 2
     is over on the RTX under FSR 2 (ADR-0034 read 1.9 ms); the reading records it, the
     default flips on the owner's eye.
  3. `frame_bytes` at 1.5 for `fsr2` and `own` read into section 3's readings file (ADR-0034
     read 305 574 892 against 181 154 464 B).
  4. Every existing FSR 2 and upscaler test passes unchanged; the goldens are bit-identical to
     Phase 1's (they build their own frames).

### Phase 3: The far-normal spike
- **Owner skill:** dev
- **What:** a shading normal blended by the voxel's size on screen, behind a knob, off by
  default, plus the strips the owner picks from.
- **Mechanism (illustrative; the names are dev's):** in `march.wgsl` where the shading normal
  is chosen (`normal_mode()`), a hit at distance `t` voxels in a view of `W` internal pixels
  across tangents `tan_l + tan_r` has a projected voxel size `px = W / ((tan_l + tan_r) t)`
  pixels. With thresholds `lo < hi` (pixels per voxel), `w = 1 - smoothstep(lo, hi, px)`;
  `w == 0` keeps `nf` and reads no neighbour; otherwise `n = normalize(mix(nf,
  gradient27_normal(h, nf), w))`. The octahedral encoding in `hit_info` already carries
  non-axis normals (the field's). The shadow ray's origin keeps the face normal
  (`shadow_normal_of`). `LookParams` gains `far_normal: [f32; 2]` (`[0, 0]` off); the
  fly-through gains `--far-normal LO,HI` and shows `far normal LO..HI` or `far normal off` on
  the banner and in the look line; `N` keeps cycling the presets. `tests/strip.rs` gains
  `VXE_STRIP_FAR=LO,HI`.
- **Files touched:** `crates/vxe-render/src/shaders/march.wgsl`, `crates/vxe-render/src/marcher.rs`,
  `apps/flythrough/src/main.rs`, `apps/flythrough/src/hud.rs`, `apps/flythrough/src/lib.rs`,
  `apps/flythrough/tests/look.rs`, `apps/flythrough/tests/strip.rs`, `docs/plans/logs/0055.md`.
- **Done when:**
  1. `the_far_normal_is_face_up_close_and_gradient27_far` (lavapipe, `support::cavity_world`
     or the pillar, 320 x 180, under 15 s): with `far_normal [2, 6]` and a camera at which
     every hit voxel spans at least 6 pixels, the normals view is bit-identical to `face`;
     with the camera far enough that every hit voxel spans at most 2 pixels, it is
     bit-identical to `gradient27`; and at an intermediate distance it differs from both. The
     distances are worked out from the camera's vertical tangent and written in the test.
  2. **The reading at the hall shot** (`strip_run`, RTX, 1920 x 1080, FSR 2 at 1.5, the crops
     of the Context table, read by the same neighbour-difference measure, which the strip
     tool prints per crop): under the setting dev proposes, the dome crop's energy is at most
     half of face's (today 0.00547 / 0.00725) and the near-pillar crop's is within 5 % of
     face's (today 0.00254 / 0.00249). Face and `gradient27` are re-read on the same tree
     beside it. A setting that cannot meet both is reported with the nearest it reaches; the
     owner picks anyway.
  3. **Cost:** the hit pass at the showcase start pose under the proposed setting, headless as
     in Phase 1, within +0.8 ms of face's (a guess at half the whole-frame `gradient27`
     delta of +1.52 ms; the reading decides, and the log states it against both).
  4. **The strips for Phase 4**, under `renders/plan-0055-strips/` (gitignored) and listed in
     the readings log: the hall shot, the hall eye pitched up 30 degrees at the dome, the
     level's vein pose and its spawn, and the flight the strips used (40 voxels/s, 12 degrees
     a second yaw, 180 frames), each at `face`, `[1, 3]`, `[2, 6]`, `[3, 10]` and whole-frame
     `gradient27`, with the dome and far-wall crops and the flicker map. Dev names the setting
     it would pick and why, in one line, in the log.
  5. The goldens are bit-identical with the knob off; `frame_bytes` is unchanged.

### Phase 4: The owner's pick
- **Owner skill:** human
- **What:** the owner looks at Phase 3's strips, names a setting or `face`, then one windowed
  session on the RTX: `flythrough --far-normal LO,HI` at the pick in the showcase and the
  level, the dome and the far walls at rest and in flight, and a slow pan across the near
  pillars and the ore under FSR 2 (the verdict ADR-0034 is owed). Dev sets the session up as
  a runner script that tees to a log and pings the owner (`renders/plan-0055-strips/session.sh`).
- **Done when:** the owner's words are under **Owner answers** below: (a) the far normal's
  setting, or `face` (the spike fails); (b) FSR 2 at 1.5 stands as the default after the slow
  pan, or does not.

### Phase 5: The picks become the defaults, or the spike's code goes
- **Owner skill:** dev
- **What:** if (a) names a setting, `LookPreset::DEFAULT` carries it, `--far-normal` keeps
  overriding it, and the banner shows it; if (a) is `face`, the knob, the blend and
  `VXE_STRIP_FAR` are removed and only the readings remain. If (b) is "does not", the
  fly-through's default returns to our upscaler at 1.5 and Phase 2's readings stand as
  readings. Either way the goldens that changed are re-blessed and the hero
  (`docs/images/hero.png`) is re-shot from the hall shot at its exposure with the new
  defaults.
- **Files touched:** the Phase 3 files, `apps/flythrough/tests/golden/`, `docs/images/hero.png`,
  `docs/plans/logs/0055.md`.
- **Done when:**
  1. A parse-and-look test asserts the fly-through's start look and upscaler equal the owner's
     picks (under 1 s).
  2. The hall golden under the new defaults is re-blessed and its hue-class test holds; the
     readings log names every re-blessed golden and its changed fraction.
  3. The hero is re-shot and its command line is in the readings log.
  4. The gate tier passes at the tip (`cargo nextest run --profile gate` under the perf lock),
     clippy, fmt and deny clean.

## Data shapes

```rust
// illustrative: the one new field
pub struct LookParams {
    // ... as today, minus `detail`
    /// The far shading normal: below `lo` pixels per voxel the 27-count gradient, above
    /// `hi` the face normal, a smoothstep between; `[0.0, 0.0]` is off (the face normal).
    pub far_normal: [f32; 2],
}
```

The material row's GPU layout (`materials`, four `vec4` per id) does not change; the two
detail lanes and the fine-octave lane are written zero and named spare.

## Budgets

- **`nfr.md` section 2, the frame line and the upscale stage.** FSR 2 at 1.5 against ours at
  the showcase pose on the RTX: 10.442 against 8.706 ms at 2530 x 1378 (the baseline's
  same-pose reading, c09aa5b); Phase 2 re-reads both poses on both adapters. The upscale
  stage's 1.5 ms line is over under FSR 2 on the RTX (1.9 ms in ADR-0034), accepted on the
  owner's eye; the close records it as a reading, not a budget change.
- **The hit pass.** Phase 1 within 0.1 ms of 2.818 ms; Phase 3's blend within +0.8 ms of face
  at the showcase pose (a guess; whole-frame `gradient27` is +1.52 ms).
- **`nfr.md` section 3, `frame_bytes`.** FSR 2 at 1.5 holds about 124 MB more than ours
  (ADR-0034); Phase 2 re-reads it.
- The close measures all of these on the RTX 3080 Laptop and, where named, the RADV RENOIR
  iGPU, headless, under the perf lock, with the commit named.

## Risks & open questions

- **Material rows in a file format.** If `Material` is serialized anywhere (a save, a prefab),
  Phase 1 touches a format. Dev checks before deleting and the log says what it found.
- **The transition band in motion.** `w` changes with distance, so a surface approached at
  speed changes its shading smoothly across the band; a too-narrow band could read as a
  moving seam. The strips' flight and flicker map at the three settings read it; the owner
  decides.
- **Plastic at the near end.** `[3, 10]` smooths voxels up to ten pixels wide; the near-pillar
  bar (within 5 % of face) is the guard, and the owner's eye the gate.
- **The a-trous filter under blended normals.** Its normal weight (`n.n'^128`) lets the
  filter blur across far surfaces once their normals agree, which is the intent; it also
  blurs the far wall's shadow structure a little more. Read on the far-wall crop.
- **FSR 2 on the Radeon.** ADR-0034 read 34.7 to 35.0 ms frames at 1080p against ours at
  28.1 to 28.8. Informational by the owner's call; a walk-speed spike on the Radeon is owed
  separately.
- **Determinism, memory, threading:** nothing here touches `vxe-world`'s edit math; the
  material fields removed are render properties. No new buffers but a two-float uniform
  field.

## What this plan does NOT do

- Remove our upscaler (ADR-0024): the structure pass, Plan 0045.
- Amend Plan 0051: a separate architect pass, with the strips' finding that the lamp-off
  settle is the probe volume (its experiment B does not touch that case, A does, and a
  "headlamp does not feed the probes" arm is a candidate).
- The far-wall blotches: converged direct light on the evidence; the owner says whether they
  remain a look item.
- A mip-based far normal: ADR-0042's alternative if Phase 4 keeps the blend, a later plan if
  it does not.
- The Radeon walk-speed spike, the dig dots, the shader toolchain spike (parked 2026-10-31).

## Owner answers

2026-11-03, after one windowed session of the six variants on the showcase on the RTX
(`renders/plan-0055-strips/variants.sh`; the level was not among them): (a) "i think variant 4 is the best. difficuilt to say with problematic lighting"
(variant 4 is `[2, 6]`), then "keep 2,6 as provisional". (b) "fsr2 stays".

2026-11-03, at the close. The scripted slow pan under FSR 2 did not run: `session.sh` was never
started, and `variants.sh` has no separate pan. The close review asked whether (b) stands as
ADR-0034's slow-pan verdict, and the architect recommended recording it as still owed. The owner
decided otherwise: "fsr2 stays counts". So (b) is that verdict, given in the owner's words on the
six-variant session, and no slow-pan verdict is owed anywhere.

## Implementation log

> Written by `dev`: one row per phase as its commit lands, then the close block. **The phases
> above are the contract. This section records what happened.** Observations, never conclusions:
> no pass list, no self-assessment. Deviations and unmet done-whens are **always** disclosed.
> Keep it shorter than `## Implementation phases`. Readings, gate counts, attempts, sweeps and
> resume notes go to the plan's readings log, `docs/plans/logs/0055.md` (started from
> `templates/readings-log.md`), and this section links it once it exists.

Readings: [`logs/0055.md`](../logs/0055.md).

| phase | owner | state | commit |
|---|---|---|---|
| 1: Procedural detail removed | dev | done | 4ec2b34 |
| 2: FSR 2 at 1.5 the default | dev | done | a2c4dc5 |
| 3: The far-normal spike | dev | done | e9f0d55 |
| 4: The owner's pick | human | done | owner answers above |
| 5: The picks become the defaults | dev | done | b829b21 |

### Notes

- Phase 1 touched files outside its list: `apps/flythrough/examples/hero.rs`,
  `apps/flythrough/tests/shutdown.rs` (`--look plain` gone), `apps/flythrough/tests/support/rigs.rs`,
  `apps/flythrough/tests/support/sharpness.rs` (deleted; `axis_fraction` moved to `tests/look.rs`),
  `README.md` (flag and key tables), and renamed `cave.rs`'s `DETAIL_SHIFT` to `FINE_SHIFT`.
- Phase 1 deleted `look_detail.png` and the three `look_wall_*.png` goldens with their tests.
- Followup, not acted on: `GpuWorld::rewrite_materials` has no caller left in the workspace.
- Followup, not acted on: `docs/nfr.md:386` says the shutdown test runs `--look plain`; the test
  now runs the default look. `docs/nfr.md` and `docs/nfr/` name `--look plain` and `--detail` in
  historical readings, left as written.
- Phase 2: with no `--upscaler`, `--jitter` given at any scale starts in `own` (only `own` takes it).
- Phase 1's done-when 3 was not met at 4ec2b34: `start_modes` never showed the look, and the test called `show_look` by hand. Phase 3's commit fixes `start_modes` and the test reads its banner.
- Phase 3's done-when 2: no setting meets both the dome and the near-pillar bars; the nearest are in the readings log.
- Phase 3 moved `the_showcase_banner_names_its_source_and_the_lamp`'s source check to the banner's second row: the look row now comes first.
- Phase 3 added setting [1, 2] to the strips, and read five more bands at the hall shot.
- `LookPreset` gained `far_normal`; `N` leaves the run's far normal alone.
- Phase 4 ran `variants.sh` (six variants, showcase) in place of `session.sh`; the level run and the separate slow-pan run did not happen as scripted. The owner named `[2, 6]` provisional because the lighting made it hard to judge.
- Phase 5 touched files outside its list: `apps/flythrough/examples/hero.rs` (FSR 2 above scale 1 without `--jitter`), `apps/flythrough/src/lib.rs`, `apps/flythrough/src/main.rs`, `apps/flythrough/tests/golden.rs` (the hall test builds FSR 2), `README.md`, `docs/nfr/02-frame-budget.md`.
- Phase 5 changed `the_default_look_is_face` to compare the default look over both mirrors, not against `face`: the default's normals differ from `face` on far surfaces. It also asserts `DEFAULT` is `FACE` plus `[2, 6]`.
- `--look face` is the far normal off; `N` leaves the run's far normal on when it returns to `face`.
- Followup, not acted on: flythrough::look `the_default_look_is_face` reads 173.5 s on lavapipe, over the soak line, and is not on the soak list.
- Followup, not acted on: RADV showcase frame reads 55 ms under the defaults (about 18 fps); a slower adapter has no far-normal off switch but `--look face`.
- The plan's TL;DR says the fly-through starts at the vein pose; no phase changes the start pose
  and none was changed (owner told, 2026-11-03).
- Review fix pass on d548cdb's findings. Major, the gate over 12 min: 5c95af6, five tests moved to the soak list. Minor, the far normal named where nothing reads it: 6a62ea4; it also touched `README.md` and adds `next_look`. Minor, the strip tool's default: 16036af. Nit, the test's name: 02b7443. Nit, `live/`: 86eecb1. Gate at 86eecb1: 640.669 s; `docs/plans/logs/0055.md`.
- Close review's findings at aaa6516. Minor, the README's `--look plain` clause: 7b06065. Minor, `a_smooth_surface_meets_a_crisp_cube_without_a_gap` over CI's 60 s line (86.7 s, run 41614241239), on the soak list: 0546d18; `check-test-tiers` exits 0, the gate list no longer holds it, no gate rerun.

### Close triggers

- **What shipped:** feature
- **Public API changed:** `vxe_world::Material` loses `detail`, `detail_fine`; `vxe_render`: `LookParams` loses `detail`, gains `far_normal`; `SurfaceTuning` loses `detail_after_lighting`; `FLAG_DETAIL`, `FLAG_DETAIL_AFTER_LIGHTING` gone; `Params` gains `far_normal` in spare words, same size; material row layout unchanged. `flythrough`: `ROCK_DETAIL`, `look_materials`, `look_materials_with`, `LookPreset::rock_detail`, `detail_fine`, `materials` gone; `LookPreset::far_normal`, `far_normal_words` added. CLI: `--detail`, `--rock-detail`, `--detail-fine`, `--detail-after`, `--look plain`, the `V` key gone; `--far-normal LO,HI` added; the default upscaler is `fsr2`.
- **Budget readings:** `docs/plans/logs/0055.md`, and `docs/nfr/02-frame-budget.md`, `docs/nfr/03-memory.md`. Frame (RTX 3080 Laptop, 2530 x 1378 from 1687 x 919, e9f0d55's tree): showcase start 11.799, 11.936 ms; vein 7.482, 7.519 ms. RADV RENOIR 1080p: showcase 55.063, 54.998 ms; vein 33.107, 33.206 ms. Upscale stage under FSR 2: RTX 2.932 to 2.958 ms, RADV 9.967 to 9.983 ms, over the 1.5 ms line (a2c4dc5's parent tree). Hit pass at Phase 1: 2.793 to 2.846 ms. `frame_bytes` at 1.5, fly-through's line: 1080p 309 377 468 B against ours 184 957 040 B; 2530 x 1378 483 147 556 B against 273 897 312 B.
- **Gate at the tip:** b829b21's tree: `cargo build` 0; `cargo nextest run --workspace --profile gate --no-fail-fast` 0, 694 run, 694 passed, 46 skipped; `cargo test --doc --workspace` 0; clippy 0; fmt 0; `cargo deny check` 0; doc and tier scripts 0.
- **Outstanding `human` phases:** none

## Close

2026-11-03, architect, fresh session. Verdict: clean. The first review's findings, on d548cdb,
had their fix pass (Notes). The close review of aaa6516 then found two minors and no blocker or
major; 7b06065 and 0546d18 fix them, and this close read both diffs and f7d77dc.

- **What the close relied on without rerunning.** The gate was not rerun after the fix pass (the
  owner's rule of 2026-11-03: the gate tier runs once per plan, a fix pass reruns what its findings
  touched). The review reported 690 passed, 51 skipped, 636.8 s at aaa6516; that figure reached this
  close through the orchestrator's brief, and neither this plan nor its log holds it. The log's
  own reading is 690 passed, 51 skipped, 640.669 s at 86eecb1, and only docs differ between the two
  commits. Both counts include `a_smooth_surface_meets_a_crisp_cube_without_a_gap`, which 0546d18
  moves to the soak list, so a rerun lists one test fewer (not read). 0546d18 adds it to all five
  copies of the filter, and `check-test-tiers` exits 0 on that tree. The doctests, clippy, fmt and
  deny readings are 86eecb1's, in the log.
- **Done-whens not met, recorded as readings.** Phase 3's near-pillar bar (no setting meets it and
  the dome bar together) and its +0.8 ms cost bar at the owner's pick: `[2, 6]` reads about +1.5 ms
  at the showcase start, the same as whole-frame `gradient27`. The setting is the owner's call
  (ADR-0042). Phase 2's upscale stage is over `docs/nfr.md` section 2's 1.5 ms line, as the plan
  said it would be: 2.932 to 2.958 ms on the RTX 3080 Laptop.
- **Not run:** Phase 4's level run and the scripted slow pan (above). The far normal was never
  judged in a window on the level.
- **The stale line at `docs/nfr.md` section 7** that said the shutdown test runs `--look plain` is
  corrected at this close. The other followups under Notes stay followups.

## Followups

- The far normal's setting: reopen `[2, 6]` after Plan 0051, when the light no longer gets in the
  way of judging it (the owner's "difficuilt to say with problematic lighting"; ADR-0042).

- The strip tool (`apps/flythrough/tests/strip.rs`) prints its per-crop neighbour-difference
  measure from Phase 3 on; a later plan may fold it into the sharpness readings' home.
