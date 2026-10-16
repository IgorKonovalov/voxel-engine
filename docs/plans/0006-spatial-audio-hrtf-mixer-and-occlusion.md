# 0006: Spatial audio: a headless HRTF mixer on the device callback, occlusion and early reflections through the material table

> **Status:** in-progress
> **Created:** 2026-10-16
> **Milestone:** M4 in `docs/roadmap.md`
> **Related ADRs:** [ADR-0006](../adrs/0006-hrtf-source-and-audio-thread-model.md) (HRTF source, rendering, thread model), ADR-0002 (access model: probes take short chunk read locks), ADR-0001 (stack)
> **Prerequisite:** Plan 0001 closed. Independent of Plans 0002 to 0005 and of their readings.
> **Amended 2026-10-16:** the HRIR pack keeps every 3rd azimuth of each SADIE II D2 ring
> (2950 directions, about 3.0 MB); header is 20 bytes. See "Amendment 2026-10-16" below.
> **Amended 2026-10-16 (second):** Phase 3's partial-cover pillar is 3 x 3 voxels; Phase 4 pins
> the reflection direction set, the hit point (the face) and the tap delay as excess over the
> direct path, so the expected echo is 1160 samples. See "Amendment 2026-10-16 (second)" below.
> **Amended 2026-10-16 (third):** the HRIR pack uses one window start for every direction
> (sample 54 of 256), not one per direction; each reflection tap is scaled by its source's
> occlusion multiplier; Phase 3's rendered ratios are stated with reflections off, and the
> figures with reflections on move to Phase 4. See "Amendment 2026-10-16 (third)" below.

## TL;DR

A new crate, `vxe-audio`, mixes positional sources into binaural stereo with head-related
transfer functions and plays them through `cpal`. The mixer renders into a buffer without a
device, so every claim is a headless test. Occlusion and transmission come from the voxel world:
a new `vxe-world` query walks a ray through solids and returns the materials and lengths it
crossed, and the acoustic triple in the material table (absorption, transmission, resonance)
turns that into a gain and a low-pass cutoff per source. Early reflections come from a fixed fan
of probe rays around the listener, turned into delay taps. The first thing a reader sees is a
headless test: a 100 Hz tone 10 m away behind 5 m of a material with transmission 0.5 and
absorption 0.3 per metre renders at 0.0814 of its unoccluded level. The first thing the user
hears is a knock on the wall they just dug, muffled when they walk around the corner.

## Amendment 2026-10-16

Phase 2 stopped on the pack size. SADIE II subject D2 is a KEMAR measured at 8802 directions
(22 elevation rings of 400 azimuths at 0.9 degrees, plus the two poles), not the about 2800 the
draft assumed. Packed whole, the file is 9,083,684 bytes, three times the repo's current size,
and a committed blob never leaves history.

**Decision:** the packer takes `--azimuth-stride N` (default 3) and keeps ring azimuth indices
divisible by N, plus both poles. At stride 3 that is 22 rings x 134 (indices 0, 3, ..., 399)
+ 2 poles = 2950 directions, and 20 + 2950 x 8 + 2950 x 2 x 128 x 4 = 3,044,420 bytes. Azimuth
spacing becomes 2.7 degrees, every elevation ring stays. Nearest-direction lookup with the
one-block crossfade (ADR-0006) is unchanged; 2.7 degrees sits inside the frontal localisation
blur, and the moving-source done-when still has to hold.

**Rejected:** the full 8802-direction pack (9.08 MB in history for steps below hearing on the
first consumer's use); stride 2 (4402 directions, 4.54 MB) as the default, kept as the fallback
if the owner hears azimuth stepping at the Phase 5 listening check (re-run the packer, one new
blob); a human SADIE subject (about 2800 directions, but the plan pins D2 and a re-pin needs a
new licence and attribution check); `f16` or fewer taps (changes the format and the 128-tap
argument in ADR-0006 for a saving stride already gives).

Also corrected: the header is a magic plus four `u32`, 20 bytes; D2 is a KEMAR, so the
+90 degree expectation no longer quotes a KU100 figure.

## Amendment 2026-10-16 (second)

Phase 3 stopped on the partial-cover done-when (`plan-0006` at 1b8a894); Phase 4 has two
conventions the draft left open. The owner chose the recommendations below on 2026-10-16.

**Phase 3, partial cover.** The draft's pillar (`x in [100, 150)`, `y in [123, 133)`, all `z`)
lies on all seven probe rays and measures 0.083982, the full-wall figure. The draft's arithmetic
was wrong: the direct ray and the two `±x` offset rays are collinear, so anything on the direct
ray covers those three, and the `±y` and `±z` offset rays stay within 1.65 to 3.3 voxels of the
axis over `x in [100, 150)`, inside a 10-voxel pillar. **Decision:** the pillar is
`x in [100, 150)`, `y in [127, 130)`, `z in [127, 130)`. It holds the axis (`y = z = 128.5`);
the `±y` and `±z` offset rays aim 5 voxels off the axis at the source, which is 150 voxels
beyond the listener, so at `x = 100` they sit 1.65 voxels off the axis, at `y` or `z` 130.15 or
126.85, outside it. It covers exactly the direct and `±x` rays, and the expected multiplier
stays `(3 * 0.08404 + 4) / 7 = 0.6074` within 1 % (dev measured 0.607444). Also corrected: the
source at `(200, 128, 128)` is 15 m from the listener at `x = 50.5`, not 10 m; every asserted
figure is a ratio to the same scene without the wall and is unchanged. The 5 m multiplier is
asserted within 1 % of 0.08404 (dev measured 0.083982: the four off-axis rays cross 5.003 m).
Rejected: a slab `y in [127, 130)`, all `z` (five rays, 0.345713), which tests the same model
with a less legible figure.

**Phase 4, the direction set.** The sixteen directions are a Fibonacci sphere with its poles on
the world `x` axis, in world axes (not turned with the listener):
`x_i = 1 - 2i / 15`, `r_i = sqrt(1 - x_i²)`, `phi_i = i * pi * (3 - sqrt 5)`,
`d_i = (x_i, r_i cos phi_i, r_i sin phi_i)` for `i = 0..15`, in `f32`. Then `d_0` is exactly
`(1, 0, 0)` and `d_15` exactly `(-1, 0, 0)`; the test lists `d_0`, `d_1 = (0.8667, -0.3679,
0.3370)` and `d_2 = (0.7333, 0.0594, -0.6773)` to four places. Rejected: the textbook
half-offset Fibonacci set (`x_i = 1 - (2i + 1) / 16`), which puts no ray on the axis the test
scene uses and turns the expected echo into an off-axis calculation.

**Phase 4, the hit point and the delay.** The hit point `H` is where the ray enters the solid:
`origin + dir * t0` of the first solid run `trace_materials` returns, which is exact in fixed
point. We take the face, not the hit voxel's centre, because the face is where the surface is,
the run's `t0` is already the query's output, and the centre would add a half-voxel bias that
depends on the incidence angle. The tap delay is the **excess** over the direct path,
`(|L - H| + |H - S| - |L - S|) / 343 m/s`, rounded to whole frames, because this mixer gives the
direct path no propagation delay (no Doppler, Phase 1). The tap feeds the voice's mono signal
ahead of its HRIR FIR, so the direct sound and its echoes share the voice's HRIR and onset; its
gain multiplies the voice's own gain, not the direct path's distance gain. In the Phase 4 scene
the listener at `x = 0.5` is 4.95 m from the face at `x = 50`: the `+x` ray hits
`H = (50, 128.5, 128.5)`, `|L - H| = 4.95 m`, `|H - S| = sqrt(4.95² + 2²) = 5.3388 m`, and the
echo trails the direct peak by `(10.2888 - 2) / 343 s = 24.166 ms`, **1160 samples** (1159.94),
within ±2 samples (frame rounding plus the shared FIR's peak; the hit is exact, so the draft's
0.1 m quantisation allowance no longer applies). Its amplitude relative to the direct peak is
`0.7 * 2 / 10.2888 = 0.1361`, within 10 %. Five more rays hit the wall inside the extent, at
1458, 1634, 1922, 2938 and 3661 samples, with relative amplitudes 0.113 to 0.050; the first echo
is the earliest peak after the direct one, more than 290 samples clear of the next. Rejected:
the voxel centre (dev's flagged alternative; 1173 samples by the draft's arithmetic, which
also placed the listener 5 m from the face).

**Listening checks.** A `dev` fix pass is in flight on `plan-0006` for two defects the owner
heard on the dev box: left and right swapped, and clicks when turning or when a source's
distance changes. Phase 3's listening check (the `J` hum behind a wall) is re-run after that
pass lands, and so is Phase 1's `H` knock; the log records both.

## Amendment 2026-10-16 (third)

The fix pass on `plan-0006` stopped on two blocks (the log's Notes at 23ccbf2 and fa7c920). The
owner decided both on 2026-10-16, after an A/B listen of the first.

**Phase 2, the pack window.** One window start for every direction replaces the per-direction
rule. The packer computes each ear's onset as before (the first sample reaching a tenth of that
ear's own peak), takes the earliest onset over every kept direction and both ears, and starts
every direction's two 128-tap responses 8 samples before it. For D2 at stride 3 that is sample
54 of the source's 256. The format, the direction count and the size (3,044,420 bytes) do not
change; the file keeps its name, `assets/sadie2_d2_48k.hrir`, and its bytes become those of
dev's candidate `sadie2_d2_48k_global_window.hrir` (sha256 `86375f1c8ff0...`, the full hash in
the log), packed from the same `D2_HRIR_SOFA.zip` (md5 `f555491d1b6b1a108bb4d26b63ad84a4`).

Why: the per-direction rule caused the turning clicks. A tenth of the far ear's peak is often
reached by its faint early response, so neighbouring directions' windows start up to 23 samples
apart and their 440 Hz phases differ by up to 1.822 rad; the one-block crossfade between them
is a 5 ms phase jump. With one window the largest ring-neighbour phase step is 0.078 rad, the
least energy any ear keeps in its window rises from 0.934 to 0.971, and every Phase 2 and 3
check holds (2 kHz at +90 degrees 7.10 dB, front 0.71 dB, -90 degrees 7.60 dB; moving max step
0.0576 against static 0.0519; the fly-through's side test 19.6 and 17.9 dB). The owner A/B'd it
live with `--audio-debug`: blocks with a 440 Hz phase jump over 0.3 rad fell from 378 of 2722 to
14 of 3611, level steps over 0.5 dB per 5 ms from 11.4/s to 2.6/s, and the clicks were gone by
ear. The replaced pack stays in history as a second blob of about 3 MB; that is accepted.

What remains is unevenness, not clicks: about 7 interaural level steps over 0.5 dB per second
while turning, from nearest-direction switching at the pack's spacing. That is ADR-0006's
direction-interpolation follow-up, which this measurement now motivates; it is not in this plan.

Rejected: a per-direction window with one onset threshold for both ears (a tenth of the louder
ear's peak), which still steps 1.032 rad between neighbours; interpolation or a separated
fractional ITD now, which is the follow-up above and a larger change than the defect needs.

ADR-0006 does not state the window rule (it is a packer detail), so its decision is unchanged;
its nearest-direction consequence gains a dated line pointing at this measurement.

**Phase 4 against Phase 3, option (c).** With reflections on, the listener's rays hit the
occluding wall's near face, and a tap gain of `(1 - A) / path` carries no occlusion for the
hit-to-source leg, which runs through the wall: the echo was louder than the occluded direct
sound (0.6623 and 1.2332 against 0.0814 and 0.358). **Decision:** every tap's gain is also
multiplied by its source's occlusion multiplier (the mean of the seven probe rays, Phase 3). No
extra rays; the 240-ray budget stands. It is an approximation: a source occluded from the
listener is taken as equally occluded from every reflector.

Phase 3's rendered-ratio done-whens are stated **with reflections off**, and the figures with
reflections on become a Phase 4 check. That keeps Phase 3's figures analytic (they follow from
the occlusion model and the one-pole response alone) and keeps a change to the reflection fan
from moving a Phase 3 test. `Acoustics` gains a reflections switch (on by default; the name is
dev's) that sends no taps when off; Phase 3's rendered tests turn it off.

Under (c), dev measured the Phase 3 scenes with reflections on at 0.1136 (5 m wall, 100 Hz) and
0.6452 (0.5 m wall, 2 kHz). These are measured, not derived: they depend on where the sixteen
rays hit the walls of those scenes. Phase 4 asserts them within 2 % as a regression guard, and
asserts the analytic property behind them on the targets: in each scene every tap's gain equals
the source's multiplier times the same tap's gain with the wall's transmission set to 1, within
`1e-6` relative. Phase 4's own echo scene has nothing between listener and source (multiplier
1), so its figures (1160 samples, 0.1361) do not change.

Rejected: (a) Phase 3's tests drop the taps and nothing else changes (0.081379 and 0.359709),
which leaves the echo louder than the direct sound behind a wall in the product; (b) occlude
the hit-to-source leg with one more ray per hit (0.1040 and 0.6481), which is the better model
but 16 more rays per probed voice, 752 rays for 32 sources against the 240 the Budgets row
counts. (b) is the upgrade if the owner hears (c)'s approximation; it needs a new budget.

**Recorded from the fix pass.** The owner's left/right swap was the headset's system channel
map (`speaker-test` played "Front Left" in the right ear), not the engine; no change. These fix
pass changes on `plan-0006` are accepted as deviations outside the phases' Files touched: the
raised-cosine direction crossfade (2f497aa; still one block, as ADR-0006 says), `vxe-audio` at
opt-level 2 in dev builds (`Cargo.toml`, 2f497aa), the per-sample low-pass coefficient glide
(8a0d6c5), the callback meter and recorder behind `--audio-debug` and the `FLYTHROUGH_HRIR`
pack override (96de82d), and the two fourth-difference tests that guard the crossfade and the glide.

## Context & problem

Sound is the first consumer's core mechanic (`project-context.md`, Audio): P0 positional
playback with HRTF, and occlusion and transmission through voxels by material, so that a knock
behind 50 cm of rock sounds different from one behind a thin wall in front of a void; P1 early
reflections and reverb from the surrounding geometry, and material-driven synthesis hooks. The
consumer keeps two layers apart: perceptual audio (this plan, approximate and not required to
be deterministic) and gameplay noise (the consumer's own model on Plan 0004's coarse grid,
deterministic, never dependent on audio rays). The material table already carries the acoustic
triple (`Material::acoustic`, Plan 0001), unused.

The roadmap's M4 names the HRTF source and the audio thread model as ADR material; ADR-0006
decides them. `nfr.md` has no audio figure; this plan proposes them under "Budgets".

## Decision

We implement ADR-0006: a `Mixer` that renders blocks of interleaved stereo `f32` at 48 kHz
from a set of voices, each a mono `f32` sample buffer with position, gain and looping; per
voice, nearest-direction HRTF from a packed SADIE II head through a 128-tap time-domain FIR per
ear with a one-block crossfade on direction change; a `cpal` output stream whose callback owns
the mixer and drains a command channel; an `Acoustics` model the caller updates each frame with
the world, which probes occlusion (seven rays per source) and reflections (sixteen listener
rays) through a new `World::trace_materials` and publishes per-voice gain, cutoff and taps that
the mixer smooths. The engine decodes no audio files: a `Sound` is PCM the consumer provides.

Rejected (ADR-0006): the `hrtf` crate with IRCAM spheres, `sofar` at runtime, Steam Audio, and
mixing on the game thread. Rejected here: a reverb (P1, a follow-up once the owner hears the
early reflections), and probes on a dedicated thread (the caller's thread with a bounded probe
count is simpler and lets the game set the cadence).

## Architecture diagram

```mermaid
flowchart LR
    subgraph consumer["game / apps/flythrough"]
        G[game thread: play, move,<br/>update(world, listener)]
    end
    subgraph audio["vxe-audio"]
        AC[Acoustics<br/>7 occlusion rays/source<br/>16 reflection rays/listener]
        CMD[command channel<br/>bounded, no alloc on drain]
        MX[Mixer on the cpal callback<br/>voices, HRTF FIR, LPF, taps]
        OUT[cpal output stream<br/>48 kHz stereo f32]
        PACK[(assets: SADIE II D2 pack<br/>128 taps x 2 x N dirs)]
    end
    subgraph world["vxe-world"]
        TM[trace_materials<br/>runs of material, t0..t1]
        MT[MaterialTable.acoustic]
    end
    G --> AC --> TM
    AC --> MT
    AC --> CMD
    G --> CMD --> MX --> OUT
    PACK --> MX
```

## Implementation phases

Each phase ships as its own commit. `dev` implements all phases in one session with no review
between phases. The architect reviews once at the end, in a fresh session. Phase 1 is the
walking skeleton.

Shared conventions for every phase:

- The mixer rate is 48 000 Hz; blocks are 256 frames; all samples are `f32`; stereo is
  interleaved left, right. Distances in metres come from positions in voxel units through the
  world's voxel size, 0.1 m, passed in as `metres_per_voxel: f32`; the speed of sound is 343
  m/s.
- **Distance gain** is `1 / max(d, 1 m)`; a source at 1 m or closer plays at its gain.
- **Headless renders** in tests use `Mixer::render(&mut [f32], frames)` directly; no test opens
  a device. The RMS of a stereo buffer is the root mean square over both channels unless a
  channel is named. Tolerances are stated per test.
- **Smoothing:** a voice's target gain and cutoff move toward their targets with a one-pole
  smoother of 50 ms time constant; tests that assert steady values render 0.5 s and measure the
  last 0.1 s.
- Test tones are generated in the test with `sin` from `std` (this crate is not deterministic
  by contract; `vxe-world` stays integer).
- New dependencies, pinned in `[workspace.dependencies]` at the then-latest release: `cpal`
  (0.18.2 on 2026-10-16, default features) for `vxe-audio`; `sofar` (0.4.0) as a
  dev-dependency of the packer example only. CI's Linux arm installs `libasound2-dev` for
  `cpal`'s ALSA backend; the Windows arm needs nothing.

### Phase 1: `vxe-audio`, the mixer and the output stream
- **Owner skill:** dev
- **What:** the crate; `Sound`, `Voice`, `Listener`, `Mixer::render` with distance gain and
  constant-power stereo panning (no HRTF yet); `Output` over `cpal` with the command channel;
  a knock in the fly-through.
- **Files touched:** `Cargo.toml` (workspace member, `cpal` pin, `vxe-audio` row),
  `crates/vxe-audio/Cargo.toml`, `crates/vxe-audio/src/{lib.rs, mixer.rs, voice.rs, output.rs,
  command.rs}`, `crates/vxe-audio/tests/mixer.rs`, `apps/flythrough/src/main.rs` (`--audio`,
  key `H`), `apps/flythrough/Cargo.toml`, `.github/workflows/ci.yml` (`libasound2-dev`).
- **Done when:**
  - `Mixer::render` of a 1 kHz tone of amplitude 0.5 placed 1 m in front of the listener for
    0.5 s gives, over the last 0.1 s, an RMS per channel of `0.5 / sqrt(2) * (1 / sqrt(2))`
    (constant-power pan at centre) within 1 %; at 2 m, half of that within 1 %; at 0.5 m, the
    same as at 1 m. A source 1 m to the right gives a right channel RMS at least 20 dB above
    the left.
  - A looping voice of 1000 samples renders 48 000 frames without a discontinuity larger than
    the tone's own largest sample step; a non-looping voice ends and reports `finished`.
  - Sixty-four voices render one block without a heap allocation after the first block (a
    counting global allocator in the test, as the plan's "no allocation in the callback" rule;
    the first block may allocate scratch).
  - `Output::open(mixer) -> Result<Output, AudioError>` opens the default device at 48 kHz
    stereo `f32`, or returns `AudioError::Format` naming what the device offered. Commands
    (`play`, `set_position`, `set_gain`, `stop`, `set_listener`) go over a bounded channel of
    1024 entries; the callback drains them at the top of each block without blocking; a full
    channel makes the sender return `AudioError::Busy` rather than block.
  - `cargo run --release -p flythrough -- --seed 1 --audio`: `H` plays a synthesised knock (a
    50 ms noise burst from `SplitMix64(0x0006_0001)` with an exponential decay, generated once
    at startup) at the dig point of the last left click. The log records that the knock was
    heard on the dev box, and the device line `cpal` reported.

### Phase 2: HRTF: the pack, the FIR, the crossfade
- **Owner skill:** dev
- **What:** the `hrir_pack` example that reads a SADIE II SOFA file with `sofar` and writes the
  engine's pack format; the committed pack and licence; nearest-direction binaural rendering
  per voice.
- **Files touched:** `crates/vxe-audio/examples/hrir_pack.rs`, `crates/vxe-audio/src/{hrtf.rs,
  mixer.rs, voice.rs}`, `crates/vxe-audio/assets/{sadie2_d2_48k.hrir, LICENSE-SADIE-II.txt,
  README.md}` (third amendment: the README's modifications line states the one window), `crates/vxe-audio/tests/hrtf.rs`, `Cargo.toml` (`sofar` dev pin).
- **Done when:**
  - `cargo run -p vxe-audio --example hrir_pack -- <D2 48 kHz SOFA path> assets/sadie2_d2_48k.hrir`
    (amended 2026-10-16: with `--azimuth-stride 3`, the default) reads the SADIE II subject D2 HRIR SOFA at 48 kHz (from the `D2_HRIR_SOFA.zip` archive on
    the Zenodo record, 38 MB, not committed), truncates each HRIR to 128 taps keeping the onset
    (amended 2026-10-16, third: every direction's two 128-tap responses start at one sample,
    8 samples before the earliest onset of any kept direction and either ear, sample 54 for D2;
    the packer prints that start and the least fraction of an ear's energy kept, at least 0.95),
    and writes the pack: a 20-byte header (`b"VXEH"`, version 1, sample
    rate, tap count, direction count), the direction table (azimuth, elevation in degrees as
    `f32`), then the taps. The log records the direction count and the pack's byte size
    (amended 2026-10-16: 2950 directions and 3,044,420 bytes at stride 3; the log records the packer's own counts, and `20 + dirs x 8 + dirs x
    1024` must equal the file size; third amendment: the log records the pack's sha256, which
    begins `86375f1c`).
  - (Amended 2026-10-16, third.) Read from the pack at test start: between neighbours on every
    elevation ring, the largest change in either ear's 440 Hz phase is below 0.15 rad (0.078
    measured; the per-direction pack measured 1.822); printed and recorded. A 440 Hz hum 1 m
    ahead, the listener turning at 90 deg/s with the listener set once per 256-frame block:
    within any one block, either ear's 440 Hz phase moves by less than 0.2 rad and its level by
    less than 0.25 dB (0.058 rad and 0.04 dB measured; the per-direction pack 0.811 rad and
    0.90 dB; static 0.001 rad).
  - `assets/README.md` names the source (SADIE II, University of York, Zenodo DOI
    10.5281/zenodo.10886409), the subject, the licence (Apache 2.0) and the attribution text
    SADIE II asks for; `LICENSE-SADIE-II.txt` is the licence copy.
  - `Hrtf::load(bytes)` validates the header and rejects a pack with a wrong magic, rate or
    size with `AudioError::Pack`; `nearest(direction)` returns the index of the measured
    direction with the greatest dot product, through a precomputed 32 x 64 azimuth-elevation
    bin table, and equals a brute-force search over all directions for 1000 seeded directions.
  - A 2 kHz tone at azimuth +90 degrees (listener's right), elevation 0, 2 m: the right channel
    RMS exceeds the left by at least 6 dB over the last 0.1 s of a 0.5 s render (the measured
    difference is printed and recorded; D2 is a KEMAR). The same
    tone in front: the channels differ by at most 1 dB. The same tone at -90 degrees mirrors
    the +90 result within 1 dB.
  - A source moving from -90 to +90 degrees over 1 s at 2 m renders without any sample-to-sample
    step larger than twice the largest step of the static front render (the one-block
    crossfade removes the switch click); the maximum step is printed and recorded.
  - The Phase 1 panning tests now run through the HRTF path with their tolerances widened to
    the recorded front-direction gain of the pack (stated in the test as a constant read from
    the pack at test start, not hard-coded).

### Phase 3: Material traversal and occlusion
- **Owner skill:** dev
- **What:** `World::trace_materials` in `vxe-world`; `Acoustics` with the occlusion model;
  per-voice gain and one-pole low-pass in the mixer.
- **Files touched:** `crates/vxe-world/src/{raycast.rs, lib.rs}`, `crates/vxe-world/tests/queries.rs`,
  `crates/vxe-audio/src/{acoustics.rs, mixer.rs, voice.rs, lib.rs}`,
  `crates/vxe-audio/tests/occlusion.rs`, `apps/flythrough/src/main.rs` (key `J`, `update` call).
- **Done when:**
  - `World::trace_materials(origin, dir, max_t) -> Vec<Run { material, t0, t1 }>` walks the
    integer DDA of the raycast through solids as well as air, merging consecutive voxels of
    one material into a run with exact fixed-point entry and exit parameters, skipping
    `Uniform` bricks and missing chunks as whole runs (a missing chunk is a run of `AIR`). In
    the carved solid material-3 world, the ray from the centre of voxel `[0, 0, 0]` along
    `[0, 0, 256]` with `max_t = 100 * 256` returns `[Run { 0, 0, 2688 }, Run { 3, 2688,
    25600 }]`. The same ray in a world where `z in [20, 30)` is material 2 returns three runs
    with boundaries at `t = 2688`, `t = 4992` and `t = 7552` (faces at `z = 20` and `z = 30`
    from `z = 0.5`: 19.5 and 29.5 voxels). A pinned FNV-1a hash over 500 seeded rays in the
    carved world is blessed on Linux and asserted on both arms.
  - **Occlusion model** (documented in `acoustics.rs`): for a source, seven probe rays from
    the listener, to the source and to six points offset ±0.5 m along each axis; for each ray,
    `g = product over solid runs of T(m) * (1 - A(m)) ^ (L / 1 m)` where `T` is the material's
    transmission, `A` its absorption per metre of path and `L` the run length in metres;
    `thickness = sum of solid run lengths`; the source's gain multiplier is the mean of the
    seven `g`, and its cutoff is `20 000 Hz / (1 + 10 * thickness_m)` using the thickness of
    the direct ray. The mixer applies the gain and a one-pole low-pass at the cutoff per voice.
  - (Amended 2026-10-16, third.) The two rendered-ratio checks below run with `Acoustics`'
    reflections switched off, so they measure the direct path; Phase 4 states the same scenes
    with reflections on.
  - Headless: an air world over chunks `[0, 4)³` with a wall of material `W` (`acoustic =
    [0.3, 0.5, 0.0]`) filling `x in [100, 150)` (5 m), listener at voxel centre `(50, 128,
    128)`, a 100 Hz source of amplitude 0.5 at `(150 + 50, 128, 128)` (10 m away, 5 m of wall
    on every one of the seven rays). The gain multiplier is `0.5 * 0.7^5 = 0.08404`; the cutoff
    is `392.16 Hz`; the one-pole response at 100 Hz is `0.9690`; the rendered RMS over the last
    0.1 s equals the same scene without the wall times `0.0814`, within 2 %. Without the wall
    the multiplier is exactly 1 and the cutoff 20 000 Hz. (Amended 2026-10-16: the source is
    15 m from the listener, not 10 m; the multiplier is asserted within 1 %.)
  - A thin wall: 0.5 m of `W` gives the multiplier `0.5 * 0.7^0.5 = 0.4183` and the cutoff
    `3333 Hz`; a 2 kHz tone behind it renders at `0.4183 * (1 / sqrt(1 + (2000 / 3333)²)) =
    0.358` of the open level, within 2 %.
  - Partial cover (amended 2026-10-16): a pillar of `W` covering only the direct ray and the
    two `±x` offset rays, which are collinear with it (`x in [100, 150)`, `y in [127, 130)`,
    `z in [127, 130)`), gives the multiplier `(3 * 0.08404 + 4) / 7 = 0.6074` within 1 %.
  - `Acoustics::update(&world, listener, metres_per_voxel)` probes every voice whose position
    or the listener moved by more than 0.1 m since its last probe, or at least once per 500 ms
    of audio time (counted in blocks, no clock), and sends the targets over the command
    channel. The fly-through's `J` places a looping 440 Hz hum (synthesised) at the dig point;
    walking behind a wall muffles it. The log records it was heard on the dev box.

### Phase 4: Early reflections
- **Owner skill:** dev
- **What:** sixteen fixed listener rays, turned into per-voice delay taps with material gain.
- **Files touched:** `crates/vxe-audio/src/{acoustics.rs, mixer.rs, voice.rs}`,
  `crates/vxe-audio/tests/reflections.rs`.
- **Done when:**
  - The sixteen directions are a fixed Fibonacci-sphere set computed in `f32` once at startup,
    poles on the world `x` axis as "Amendment 2026-10-16 (second)" constructs it (listed in a
    test by their first three values so a change is visible). For each ray that hits within
    50 m at point `H` (the entry of the first solid run) with material `m`, every voice gets a
    tap at delay `(|L - H| + |H - S| - |L - S|) / 343 m/s` seconds, the excess over the direct
    path, with gain `(1 - A(m)) * (1 m / (|L - H| + |H - S|))` times the voice's own gain and
    times the voice's occlusion multiplier from Phase 3 (amended 2026-10-16, third), fed
    ahead of the voice's HRIR FIR, at most 16 taps per voice, in a 200 ms delay line per voice;
    taps are replaced with a one-block crossfade when the acoustics update changes them.
  - Headless: an air world over chunks `[0, 4)³`; a wall of material `W` (`A = 0.3`) filling
    `x in [50, 60)`; listener at voxel centre `(0, 128, 128)`... (that voxel is in the extent;
    `[0, 4)³` is voxels `[0, 256)³`), source at `(0, 148, 128)` (2 m along `+y`), both 4.95 m
    from the wall face at `x = 50` (amended 2026-10-16). A one-sample impulse of amplitude 1.0
    rendered for 100 ms shows its direct peak and a second peak, from the `+x` ray, whose delay
    after the direct peak is `(4.95 + sqrt(4.95² + 2²) - 2) / 343 s = 24.166 ms`, that is
    `1160` samples at 48 kHz, within ±2 samples (the hit is the exact face entry; the HRIR
    onset shifts both peaks equally). The second peak's amplitude relative to the first is
    `0.7 * (1 / 10.2888) / (1 / 2) = 0.1361`, within 10 % (the HRIR shapes both).
  - With the wall removed, no second peak above 1 % of the first exists within 100 ms.
  - (Amended 2026-10-16, third.) Phase 3's 5 m and 0.5 m wall scenes with reflections on:
    every tap's gain equals the source's occlusion multiplier times that tap's gain in the same
    scene with the wall's transmission set to 1, within `1e-6` relative; the rendered ratios to
    the open scene are 0.1136 (100 Hz) and 0.6452 (2 kHz), each within 2 % (measured under this
    rule, not derived). With reflections off, `Acoustics` sends no taps.
  - The `--audio` fly-through's `F1` line prints the number of active taps and the last
    acoustics update's time.

### Phase 5: Benches, readings and the fly-through's audio HUD
- **Owner skill:** dev
- **What:** the callback and acoustics cost readings against the proposals, and the proposals
  into `nfr.md`.
- **Files touched:** `crates/vxe-audio/benches/mixer.rs`, `crates/vxe-audio/Cargo.toml`,
  `apps/flythrough/src/main.rs`, `docs/nfr.md`.
- **Done when:**
  - `cargo bench -p vxe-audio --bench mixer`: one 256-frame block with 32 HRTF voices, each
    with low-pass and 16 taps; and one `Acoustics::update` of 32 sources (224 occlusion rays
    plus 16 reflection rays) against the seed-1 level-size cave. The log records both on the
    dev box against the proposals under "Budgets".
  - In the `--audio` fly-through, the callback measures its own block time with
    `Instant::now` behind `#[allow(clippy::disallowed_methods, reason = ...)]` (this crate is
    the perceptual layer; `vxe-world` keeps the ban) and `F1` prints the maximum block time
    over the last second and the number of voices; the log records a reading with 8 voices
    playing.
  - `nfr.md` gains an audio section carrying the proposals and the readings (the close writes
    it from the log).

## Data shapes

```rust
// illustrative: vxe-audio
pub struct Sound(Arc<[f32]>);                      // mono at 48 kHz; the consumer decodes
pub struct VoiceId(u32);
pub struct Listener { pub position: [f32; 3], pub forward: [f32; 3], pub up: [f32; 3] }  // voxel units
pub struct PlayParams { pub position: [f32; 3], pub gain: f32, pub looping: bool }
pub enum Command { Play(VoiceId, Sound, PlayParams), SetPosition(VoiceId, [f32; 3]), SetGain(VoiceId, f32),
                   Stop(VoiceId), SetListener(Listener), SetAcoustics(VoiceId, AcousticTargets) }
pub struct AcousticTargets { pub gain: f32, pub cutoff_hz: f32, pub taps: [Tap; 16], pub tap_count: u8 }
pub struct Tap { pub delay_frames: u32, pub gain: f32 }

pub struct Mixer { voices: Vec<Voice>, hrtf: Arc<Hrtf>, listener: Listener, scratch: Vec<f32> }
impl Mixer { pub fn render(&mut self, out: &mut [f32], frames: usize); pub fn apply(&mut self, cmd: Command); }

pub struct Hrtf { rate: u32, taps: u32, dirs: Vec<[f32; 3]>, fir: Vec<f32> /* dirs x 2 x taps */, bins: Vec<u16> }
pub struct Acoustics { last: HashMap<VoiceId, ProbeState>, directions: [[f32; 3]; 16] }
impl Acoustics {
    pub fn update(&mut self, world: &World, table: &MaterialTable, listener: &Listener,
                  voices: &[(VoiceId, [f32; 3])], metres_per_voxel: f32, send: &mut dyn FnMut(Command));
}
pub enum AudioError { NoDevice, Format(String), Busy, Pack(&'static str), Stream(String) }

// vxe-world
pub struct Run { pub material: MaterialId, pub t0: i64, pub t1: i64 }
impl World { pub fn trace_materials(&self, origin: Fixed3, dir: [i64; 3], max_t: i64) -> Vec<Run>; }
```

```text
Pack format (little-endian):
  magic b"VXEH", u32 version = 1, u32 rate = 48000, u32 taps = 128, u32 dirs
  dirs x (f32 azimuth_deg, f32 elevation_deg)
  dirs x 2 ears x taps x f32
```

## Budgets

The figures below are **proposals** this plan introduces (`nfr.md` has no audio section); the
close writes them in with the dev-box readings.

| Figure (proposed) | Proposal | How the close measures it |
|---|---|---|
| Mixer callback, 32 HRTF voices, one 256-frame block (5.33 ms of audio) | 1.0 ms on the baseline CPU; the arithmetic is 32 x 2 x 128 x 256 = 2.1 million multiply-adds per block plus filters and taps, so well under on the dev box | `bench mixer`; the fly-through's `F1` maximum block time |
| `Acoustics::update`, 32 sources | 0.5 ms per call (240 integer rays through the brick map) | `bench mixer` |
| Audio RAM | the pack (about 3.0 MB at stride 3), 200 ms delay lines (38 KB per voice), scratch | printed by the bench |
| Startup | pack load under 10 ms | the fly-through's startup line |
| `nfr.md` §6 | `trace_materials` is integer and pinned on both arms; nothing else here is deterministic by contract | the pinned hash in `queries.rs` |

The baseline CPU is not on hand; readings are dev reference and say so.

## Risks & open questions

- **ALSA on CI.** `cpal` links `libasound` on Linux; the CI arm installs the dev package. No
  test opens a device; `Output::open` is exercised only by the fly-through.
- **Devices without 48 kHz.** `Output::open` fails with `AudioError::Format` in this plan; a
  resampler is a follow-up (ADR-0006).
- **Nearest-direction stepping** may be audible on slow pans (ADR-0006). The moving-source test
  bounds the click, not the step; the owner's ear decides whether interpolation is the next
  plan. (2026-10-16: with the one-window pack, the owner's `--audio-debug` recording still shows
  about 7 interaural level steps over 0.5 dB per second while turning; that measurement is the
  case for interpolation.)
- **Probe cost on the game thread.** 240 rays per update at most; each ray is a brick-map DDA
  of up to 50 m. The bench reads it. If it crowds the frame, the follow-up is a worker thread
  fed by a snapshot of positions, which ADR-0006 already names.
- **Locks.** Probes take one chunk read lock at a time and hold none across a ray. A burst of
  edits delays a probe by microseconds, never the callback.
- **The acoustic triple's semantics** (transmission as an entry factor, absorption per metre,
  resonance unused here) are this plan's definition; the consumer's rows are authored against
  it, so the definition is documented in `acoustics.rs` and in the material table's doc
  comment. Resonance waits for the synthesis hooks (P1, follow-up).
- **Licence hygiene.** The pack is derived data from an Apache 2.0 dataset; the attribution file
  ships beside it, and the repository's own licence is unaffected.

## What this plan does NOT do

- **Decoding** (WAV, OGG): the consumer hands over PCM.
- **Reverb, late reflections, material-driven synthesis**: P1 follow-ups after the owner hears
  the early reflections.
- **Direction interpolation, a resampler, a worker thread for probes**: ADR-0006's named
  follow-ups.
- **Doppler, air absorption with distance, source directivity.**
- **Voice chat as a positional source**: M9.
- **The consumer's gameplay noise model**: it lives on Plan 0004's coarse grid, in the game.

## Implementation log

> Written by `dev`: one row per phase as its commit lands, then the close block. **The phases
> above are the contract. This section records what happened.** Observations, never conclusions:
> no pass list, no self-assessment. Deviations and unmet done-whens are **always** disclosed.
> Keep it shorter than `## Implementation phases`.

| phase | owner | state | commit |
|---|---|---|---|
| 1: `vxe-audio`, the mixer and the output stream | dev | done: `vxe-audio` with `Mixer`, `Output` over cpal 0.18.2, bounded command channel (1024); 6 crate tests; `--audio` + `H` knock in the fly-through; `--audio --headless --frames 10` (seed 1, no key pressed, silent stream) printed `audio: Default Audio Device, 48000 Hz, 2 ch, f32` | 5a6697a |
| 2: HRTF: the pack, the FIR, the crossfade | dev | done: packer printed `8802 directions in 24 rings, 256 taps in the file; azimuth stride 3 keeps 2950; 128 taps from samples 44..=97 onward; wrote 3044420 bytes` (= 20 + 2950 x 8 + 2950 x 1024); kept rings are 134 directions with gaps of 1.0 to 3.0 degrees; 2 kHz at 2 m: +90 right over left 6.81 dB, front 0.89 dB, -90 left over right 7.33 dB; moving -90 to +90 over 1 s: max step 0.0589 vs static front 0.0523; front gain at 1 kHz read from the pack: left 0.5187, right 0.5585; pack load 7.1 to 8.9 ms (release test build, shared lock) | bd04e56 |
| 3: Material traversal and occlusion | dev | done: amended partial-cover pillar (`y, z in [127, 130)`) measures 0.6074435 against 0.6074457 (1b8a894 blocked on the earlier pillar; finished in the commit with this row); `trace_materials` gives `[Run{0,0,2688}, Run{3,2688,25600}]` and the layer runs at 2688, 4992, 7552; 500-ray hash `0xbae9_1463_c336_8745` (Linux x86_64); 5 m wall: multiplier 0.083982, cutoff 392.157 Hz, rendered 100 Hz ratio 0.081379; open: multiplier 1, cutoff 20 000 Hz; 0.5 m wall: multiplier 0.418304, cutoff 3333.33 Hz, 2 kHz ratio 0.359709; probe cadence: 0.11 m move reprobes, 0.10 m does not, block 94 reprobes and 93 does not | 79b3d87, 5a7b226 |
| 4: Early reflections | dev | done except one done-when (block in Notes): `d_0 = (1, 0, 0)`, `d_1 = (0.8667, -0.3679, 0.3370)`, `d_2 = (0.7333, 0.0594, -0.6773)`, `d_15 = (-1, 0, 0)`; `+x` ray hits `(50, 128.5, 128.5)`; echo 1160 samples after the direct peak, ratio 0.13607; taps `[1160, 1457, 1631, 1922, 2947, 3684]`; no wall: 0 taps, 0.0 after the HRIR; Phase 3 scenes with reflections on: 0.11360 (100 Hz, 5 m) and 0.64517 (2 kHz, 0.5 m); with reflections off 0.081379 and 0.359709, 0 taps; each tap equals the occlusion multiplier (0.083982, 0.418304) times its unoccluded gain within 1e-6 relative; the worded transmission-1 form measures 0.5 per tap (ignored test) | committed with this row |
| 5: Benches, readings and the fly-through's audio HUD | dev | not started | |

### Notes

- Phase 1, awaits the owner: hearing the `H` knock on the dev box (no sound was played in this session; only a silent stream was opened for the device line).
- Phase 1: the loop-discontinuity bound is the largest step of the same tone rendered as one 48 000-sample non-looping buffer through the same mixer path, so the bound holds through the HRTF FIR of Phase 2.
- Phase 1: `Output` also carries `device()`, `stream_errors()`, `finished()` and `pause()`; `Mixer` carries `drain_finished`, `is_playing`, `voice_count`, `MAX_VOICES = 256` (a `Play` beyond it is dropped).
- Phase 2 stopped at 11f6ce5 on the pack size (D2 is 8802 directions, 9 083 684 bytes packed whole); resolved by the plan's Amendment 2026-10-16 (6f47757, azimuth stride 3). 11f6ce5 also truncated the plan after its Notes; 202851b restored the Close triggers template and Followups.
- Phase 2 source: `D2_HRIR_SOFA.zip` downloaded from Zenodo record 10886409 (DOI 10.5281/zenodo.10886409), md5 `f555491d1b6b1a108bb4d26b63ad84a4` matching the record; the record's licence is `apache2.0` and its `LICENSE.txt` (md5 `0ba5044c64ef53cb0189c9546081e228`) is the Apache 2.0 text; the SOFA's `License` attribute reads "Copyright 2018, University of York, Licensed under the Apache License, Version 2.0"; York's page asks that "the original dataset must be referenced whenever used in original or modified form" and that academic work cite DOI 10.3390/app8112029. The file used is `D2_48K_24bit_256tap_FIR_SOFA.sofa` (256 taps, zero per-filter delays); onsets put the 128-tap windows' starts at samples 44 to 97.
- Phase 2: the kept rings are not uniform: D2's 400-direction rings are 360 directions at 1 degree plus 40 others, so stride 3 leaves gaps of 1.0 to 3.0 degrees, not a uniform 2.7.
- Phase 2: the `sofar` dev-dependency is also declared in `crates/vxe-audio/Cargo.toml` (not in Files touched), with `default-features = false` (the reader needs neither its renderer nor its resampler).
- Phase 2: `crates/vxe-audio/src/lib.rs` (module and re-export) and `apps/flythrough/src/main.rs` (`Mixer::new` now takes the `Hrtf`; prints the pack load time) changed; neither is in Files touched.
- Phase 2: the hard-right 20 dB check stays on the constant-power panner (`Mixer::panning`); through the HRTF the right-hand check is the done-when's +90 degree 2 kHz test. The centre and distance checks run through the HRTF against the front measurement's `|H(1 kHz)|` per ear computed from the pack at test start, within the original 1 %.
- Phase 2: `MAX_VOICES` went from 256 to 64; every voice's buffers come from a pool built in `Mixer::new`, so `Play` does not allocate on the callback.
- Phase 2: a non-looping voice finishes once its last sample has left the FIR (`taps - 1` zeros after the end), not at the end of the sound.
- Phase 2: the moving-source test fails with the crossfade disabled (max step 0.351 against 0.0523), checked by hand and reverted.
- Phase 2: the bin table holds 64 904 candidate entries (about 32 per bin); `Hrtf` heap 3 194 204 bytes.
- Phase 3 BLOCK, partial cover: the pillar as specified (`x in [100, 150)`, `y in [123, 133)`, all z) lies on all seven probe rays and measures a multiplier of 0.083982, not 0.6074. The direct ray and the two +-x side rays are collinear, so any wall on the direct ray covers three rays; the +-y and +-z side rays stay within 1.65 to 3.3 voxels of the axis over `x in [100, 150)`, inside the pillar's y range and its all-z extent. Measured alternatives: a 3 x 3 pillar `y, z in [127, 130)` covers exactly the direct and +-x rays and measures 0.607444 (the plan's `(3 * 0.08404 + 4) / 7`); a slab `y in [127, 130)`, all z, measures 0.345713 (five rays). The test `a_pillar_covering_three_rays_gives_0_6074` holds the specified geometry and the plan's figure under `#[ignore]` until the plan is amended. Resolved by the plan's second amendment (153377f, merged into `plan-0006` at 0ce7891); the test now holds the amended pillar, not ignored.
- Phase 3: the scene prose says the source at `(200, 128, 128)` is 10 m away; from the listener at `x = 50.5` it is 15 m. The asserted figures are ratios to the same scene without the wall and do not depend on it.
- Phase 3: the 5 m multiplier is 0.083982 against the plan's 0.08404: the four side rays off the x axis cross 5.003 m of wall. The test's tolerance is 1 % (the done-when states none for the multiplier).
- Phase 3: `apps/flythrough/src/main.rs` gives every solid cave row the acoustic triple `[0.3, 0.5, 0.0]` in a table it passes to `Acoustics::update`; the cave's own table (`apps/flythrough/src/lib.rs`) leaves it zero, which blocks all sound. Values chosen here, not in the plan.
- Phase 3: `vxe-audio` depends on `vxe-world` (`crates/vxe-audio/Cargo.toml`, `Cargo.lock`; not in Phase 3's Files touched). `command.rs` (the `SetAcoustics` command) changed; not in Files touched. `Acoustics::new` takes the mixer's `BlockClock` (block count shared through an atomic) for the 500 ms cadence; the public `occlusion()` function exposes the seven-ray model to tests.
- Phase 3: the material table's doc comment does not yet carry the acoustic triple's definition (Risks names it; `material.rs` is not in Files touched); `acoustics.rs` carries it.
- Phase 3, awaits the owner: `cargo run --release -p flythrough -- --seed 1 --audio`, left-click a wall to set the dig point, `J` to start the hum there, walk around the corner: the hum turns duller and quieter behind rock and clears in line of sight. Not run here: no sound was played and no window opened.
- Fix pass, owner's headphone check 2026-10-16 at 1b8a894, finding 1 ("left and right swapped" on the `H` knock): no swap found in code; committed with this line. Pack, per ear at +-90 degrees: the +y (left, SOFA azimuth +90) entry has ear 0 energy 1.225 vs ear 1 0.024 and ear 1 lagging ear 0 by 35 frames (cross-correlation); the -y entry has ear 1 1.387 vs ear 0 0.021, ear 0 lagging by 37; every 30 degrees around the horizon the louder ear is also the earlier one. Camera to mixer: `Camera::basis` right is `forward x Z` (world -y at yaw 0), the marcher's `+ndc.x` and the `D` strafe use it, and the mixer's right is `forward x up` = the same vector. Output: interleaved left, right; cpal 0.18.2's ALSA host (default features) does no channel remap; the default sink (Blackwire C3220, via PipeWire) reports channel map `front-left,front-right`. A burst 2 m along the camera's right renders right over left 19.5 dB with the right onset 132 frames earlier; on the left, 17.9 dB and 75 frames; front 0.98 dB and back -1.01 dB R/L, front over back 4.61 dB, the same at five yaw/pitch poses. `flythrough::listener` (lib.rs) now builds the listener from the camera (moved out of `main.rs`, not in Files touched), and `apps/flythrough/tests/audio_sides.rs` (new, not in Files touched) pins it: >= 10 dB and >= 10 frames lead on the source's side, both mirrors, at five poses; world -y is the screen's right at yaw 0; front and back each within 3 dB L/R and front >= 3 dB over back.
- Fix pass, finding 1, awaits the owner: `speaker-test -D default -c 2 -t wav -l 1` speaks "Front Left" then "Front Right"; each should come from that ear of the headset. If it does, re-listen with `cargo run --release -p flythrough -- --seed 1 --audio`: dig a wall ahead (left click), press `H`, turn right with the mouse until the hole is at the screen's left edge, press `H`: the knock is on the left.
- Fix pass, finding 2 ("clicks when turning or walking closer and further"): two causes measured; committed with this line. (a) The direction crossfade was linear, so at every block edge where the nearest direction changed the output changed slope by `(new - old) / n`. Measured on a clean 440 Hz hum (f64 phase) as the largest |fourth difference| (gain 1.1e-5 at 440 Hz, 2.2 at 10 kHz): static at 1 m and 2 m, 24 azimuths, 4.95e-6; turning 90 deg/s with the listener set per 256-frame callback, 1.124e-3 (227 x static), peaks on block starts; turning 360 deg/s with 64-frame callbacks, 4.4e-3. Now a raised cosine (`crossfade_weight` in `mixer.rs`): turning 90 and 360 deg/s at 256-, 512- and 1024-frame callbacks 5.85e-6 to 6.59e-6 (1.2 to 1.3 x static); walking past at 3 m/s, 1 m to the side at the closest, 8.94e-6 to 1.04e-5 (1.8 to 2.1 x). Walking straight at the source with the linear fade measured 1.5e-6 to 2.2e-6 (the gain smoother does not kink). The existing moving-source max step is unchanged (0.0589 vs static 0.0523). With 64-frame callbacks the fade spans 64 frames and the raised cosine measures 8.8e-5 (about 18 x static); cpal 0.18.2 opens the dev box's default device (PipeWire ALSA) at a 512-frame period and a 1024-frame buffer (read with a scratch ALSA program that negotiated the same parameters and wrote no frame), so blocks there are whole. (b) `vxe-audio` built at opt-level 0 in dev builds (`[profile.dev.package."*"]` covers non-members only): one HRTF voice measured 1.25 ms per 256-frame block static and 2.52 ms (worst 3.97 ms) while its direction changed, against 5.33 ms of audio and a 10.7 ms period of two blocks; a second voice puts a turning period over its deadline, and an underrun is a click. `Cargo.toml` (not in Files touched) gives `vxe-audio` opt-level 2 in dev: 0.045 ms static, 0.090 ms turning (worst 0.15 ms); release 0.044 and 0.087 ms. Timings on the dev box under the exclusive lock, load average 11 to 15 from other lanes' builds. The fly-through (`main.rs`) now prints `audio: N stream errors so far` whenever cpal reports a stream error (underruns arrive there).
- Fix pass, finding 2: `crates/vxe-audio/tests/hrtf.rs` `turning_and_walking_past_a_tone_add_no_kinks_at_block_edges` asserts the largest fourth difference while turning (90 and 360 deg/s) and walking past (3 m/s) at 256-, 512- and 1024-frame callbacks is at most 4 x the static maximum; with the linear fade it fails at 227 x.
- Fix pass, finding 1 closed by the owner 2026-10-16 at 2f497aa (release, headphones): left and right are correct; the swap was the headset's system channel map (`speaker-test` played "Front Left" in the right ear). No engine change.
- Fix pass, finding 2 re-heard by the owner at 2f497aa (release): clicks only while the listener turns (yaw clearly, pitch less), none while walking straight at the source; no `stream errors` line. Measured cause: the pack's per-direction window. The packer starts each direction's 128 taps 8 samples before the earlier ear's onset, the onset being a tenth of that ear's own peak; the far ear's faint early response often passes, so neighbouring directions' windows start up to 23 samples apart (on the horizontal ring at -90 to -107 degrees the far, left ear is taken as leading and the near ear's peak sits at sample 33 to 34; at -99, -87 and -84 at 17). Between ring neighbours the 440 Hz phase of the pack steps by up to 1.822 rad (median 0.081, 99th percentile 1.167). The one-block crossfade between two such windows is a 5 ms phase jump: a 440 Hz hum turning at 90 deg/s changes phase by up to 0.811 rad within 256 frames with a level dip of 0.90 dB (static 0.001 rad), at 360 deg/s with callbacks of 480 to 1024 frames and listener updates at 60 and 144 Hz 0.82 to 0.84 rad and 1.00 to 1.15 dB. The fourth-difference test does not see it (1.0 x static RMS, max kink 1.3e-5): a phase glide over a block has no high-frequency step.
- Fix pass, finding 2, separately measured and fixed (8a0d6c5): the low-pass cutoff moved once per block and the filter entered and left its bypass hard; a hum whose probe cutoff alternated 20 000 / 13 333 Hz measured a max kink of 101 x static and 20 000 / 5000 Hz 780 x; with the coefficient gliding per sample, 1.1 x and 2.0 x. `tests/hrtf.rs` `probe_updates_add_no_kinks` asserts at most 4 x static. Steady figures unchanged (5 m wall 0.081379, 0.5 m wall 0.359709).
- Fix pass, finding 2, BLOCK on the pack's window rule: the plan's Phase 2 pins "per direction, the two 128-tap responses starting 8 samples before the earlier ear's onset", which is the cause. Measured candidates, packed from the same `D2_48K_24bit_256tap_FIR_SOFA.sofa` (re-downloaded, md5 of the zip `f555491d1b6b1a108bb4d26b63ad84a4`; the HEAD packer reproduces the shipped pack byte for byte): (a) one window for every direction, starting 8 samples before the earliest onset of any direction (sample 54 of 256): ring-neighbour 440 Hz phase step max 0.078 rad (median 0.026, 99th 0.062); least energy any ear keeps in its window 0.971 (shipped pack 0.934); turning at 90 deg/s 0.058 rad and 0.04 dB, at 360 deg/s 0.16 to 0.23 rad and 0.09 to 0.11 dB; the Phase 2 checks hold with it (+90 deg 2 kHz 7.10 dB, front 0.71 dB, -90 deg 7.60 dB, moving max step 0.0576 against static 0.0519, front gain at 1 kHz 0.528 / 0.547; `audio_sides` 19.6 and 17.9 dB) and so do Phase 3's; it is a new 3,044,420-byte blob in history. (b) per direction with one threshold for both ears (a tenth of the louder ear's peak): max 1.032 rad, 99th 0.849, not enough. (c) direction interpolation or a separated fractional ITD: ADR-0006's followup. Nothing of the pack or packer is committed; the candidate (a) pack (sha256 `86375f1c8ff076131ccdd8cd4b8ae6f7cac4dc5d2224f90376a013b9149719b7`), the packer and test diff that made it, the pack checker and A/B renders are under `target/hrir-candidates/` in this worktree (not tracked). Resolved by the third amendment (863df84, merged at 95bd845): option (a), committed in 6275612.
- Fix pass, 96de82d (`output.rs`, `lib.rs`, `apps/flythrough/src/main.rs`; not in any phase's Files touched): the output callback meters itself (`Output::take_stats`, `OutputStats`: buffer sizes, run time, gap between callbacks, gaps over two buffers counted as late, peak, clipped samples, max fourth difference across callback edges) and records into a ring allocated up front (`Output::record`, `take_recording`); `--audio-debug` prints a line a second and writes the last 60 s to `flythrough-audio-debug.wav`; `FLYTHROUGH_HRIR=<pack>` loads another pack. Not run here (it opens the default device).
- Fix pass, finding 2, awaits the owner: (1) offline A/B, any player: `/home/igor/Work/voxel-engine/.claude/worktrees/plan-0006/target/hrir-candidates/wav/old/turn_0fps_90dps.wav` against `.../wav/global/turn_0fps_90dps.wav` (also `turn_60fps_360dps`, `turn_144fps_360dps`, `turn_144fps_1440dps`, `static`): a 440 Hz hum 1 m ahead, the head turning back and forth over +-90 deg; the `old` files should tick on every switch and the `global` ones not. (2) live A/B, from `/home/igor/Work/voxel-engine/.claude/worktrees/plan-0006`: `cargo run --release -p flythrough -- --seed 1 --audio-debug`, dig a wall (left click), `J`, turn left and right; then the same with `FLYTHROUGH_HRIR=/home/igor/Work/voxel-engine/.claude/worktrees/plan-0006/target/hrir-candidates/sadie2_d2_48k_global_window.hrir` in front. Listen for the ticks while turning; read the `audio-debug:` lines: `late` above 0 or `gap max` above 21.3 ms is the device starving, `clipped` above 0 is clipping, and `kink max` above about 1e-4 with only the hum playing is a step in what the mixer handed the device. The wav it writes on exit holds the last 60 s. Done by the owner 2026-10-16 (third amendment): clicks gone with the one-window pack.
- Phase 3: the merge of `main` (0ce7891) took main's amended plan body and the branch's log; `Cargo.toml` kept both `vxe-audio` and `vxe-world` rows at main's `=0.4.0`; `apps/flythrough/src/main.rs` kept both `--audio` and main's `--look`/`--detail`/`--rock-detail`/`--view`; b9e9a4b records `vxe-audio` 0.4.0 in `Cargo.lock`.
- Phase 4 BLOCK, reflections against Phase 3's rendered ratios. Implemented per the amended Phase 4 (Fibonacci set, face hit, excess delay, taps ahead of the HRIR in a 9600-frame delay line, one-block raised-cosine tap crossfade). Its own done-whens measure: `d_0 = (1, 0, 0)`, `d_1`, `d_2` as listed, `d_15 = (-1, 0, 0)`; the `+x` ray hits `(50, 128.5, 128.5)`; echo 1160 samples after the direct peak, amplitude ratio 0.13607 (expected 0.1361); taps `[1160, 1457, 1631, 1922, 2947, 3684]` (the amendment lists 1458, 1634, 1922, 2938, 3661 for the other five); without the wall 0 taps and 0.0 after the HRIR. But with taps on, Phase 3's rendered scenes measure 0.6623 (5 m wall, 100 Hz; done-when 0.0814 within 2 %) and 1.2332 (0.5 m wall, 2 kHz; done-when 0.358 within 2 %): the listener's rays hit the occluding wall's near face, and the tap gain `(1 - A) / path` carries no occlusion for the `H -> S` leg, which runs through the wall, so the echo is louder than the occluded direct sound. Measured alternatives in the same tests: (a) Phase 3's rendered tests drop the taps (direct path only): 0.081379 and 0.359709, Phase 4 unchanged; (b) the tap gain also times the occlusion gain of a ray from `H` (stepped 0.5 voxel back toward the listener) to `S`: 0.1040 and 0.6481, Phase 4 unchanged, 16 more rays per probed voice (32 sources: 752 rays against the 240 the Budgets row counts); (c) the tap gain also times the source's occlusion multiplier: 0.1136 and 0.6452, no extra rays. Under (b) and (c) the Phase 3 rendered figures need new expected values. The Phase 4 work (`acoustics.rs`, `mixer.rs`, `voice.rs`, `lib.rs` re-exports, `tests/reflections.rs`) is uncommitted, held in `git stash` ("plan-0006 phase 4 WIP") on this repository; nothing of Phase 4 or 5 is committed. Resolved by the third amendment (863df84): option (c), committed with the Phase 4 row.
- Fix pass, finding 2, awaits the owner: `cargo run --release -p flythrough -- --seed 1 --audio`, dig a wall (left click), `J` for the hum, then turn on the spot fast and slow and walk toward and away from it: no clicks; the console prints no `stream errors` line. The same in a dev build (`cargo run -p flythrough -- --seed 1 --audio`). A remaining slow "stepping" of the image while turning is the nearest-direction switch ADR-0006 names (interpolation is a Followup), not a click.
- Phase 2, third amendment (pack window), committed with this line: the packer starts every kept direction's two 128-tap responses at one sample, 8 before the earliest onset over all kept directions and both ears. Run from `D2_HRIR_SOFA.zip` (md5 `f555491d1b6b1a108bb4d26b63ad84a4`), member `D2_48K_24bit_256tap_FIR_SOFA.sofa`, `--azimuth-stride 3`, it printed `8802 directions in 24 rings, 256 taps in the file; azimuth stride 3 keeps 2950; window start 62 (earliest onset 70), 128 taps; least energy kept in a window 0.9714; wrote 3044420 bytes`. `assets/sadie2_d2_48k.hrir` sha256 `86375f1c8ff076131ccdd8cd4b8ae6f7cac4dc5d2224f90376a013b9149719b7`, byte-identical to the candidate the owner A/B'd. `tests/hrtf.rs`: largest ring-neighbour 440 Hz phase step 0.0782 rad (elevation -25.0, azimuth 33.0); turning 90 deg/s with the listener set per 256-frame block, 1 m: largest 440 Hz phase change within 256 frames 0.0579 rad, deepest level dip 0.0424 dB (static 0.0017 rad, 0.0114 dB); with the previous pack swapped in, both tests fail (1.8222 rad; 0.8069 rad and 0.9724 dB). Phase 2 checks with this pack: +90 deg 2 kHz 7.10 dB, front 0.71 dB, -90 deg 7.60 dB, moving max step 0.0576 against static 0.0519.
- Phase 2, third amendment, deviation from its text: the window start the committed (sha-pinned) pack uses is sample 62, not 54, and its onset is the first sample reaching a tenth of the direction's **louder** ear's peak, not of that ear's own peak. With the rule as the amendment words it (own-peak onsets) the packer gives window start 44, least kept energy 0.9667, and sha256 `80a9e3fd448722bfe3186648d0d80c980f79709e9abfeb05bb488e260cbceb9a`, a pack the owner has not heard; the candidate's packer (the experiment patch under `target/hrir-candidates/`) used the louder-ear threshold, and aligning the shipped per-direction pack against the candidate confirms start 62 (shipped starts 44..97 sit -17..+35 from it). The committed packer reproduces the candidate byte for byte; the 54 recorded at 23ccbf2 was a misreport.
- Phase 2, third amendment: the turning test's level figure is the deepest dip of the 440 Hz magnitude inside a 256-frame span below the lower of the span's two ends (the measure behind the amendment's 0.04 and 0.90 dB); the plain max-over-min level change inside 256 frames measures 0.2806 dB with this pack, which includes the nearest-direction level step the amendment leaves to interpolation.
- Phase 4: popped `git stash` entry "plan-0006 phase 4 WIP: reflections, blocked on the phase 3 rendered ratios" (the only entry); conflicts in `lib.rs`, `mixer.rs`, `voice.rs` against 8a0d6c5 and 96de82d resolved by keeping the per-sample coefficient glide and `OutputStats` and adding the tap fields; the stash entry is left in `git stash list` (pop kept it on conflict).
- Phase 4: each tap's gain is `(1 - A) / max(path, 1 m)` times the source's occlusion multiplier (third amendment, option (c)); `taps()` takes the multiplier as an argument. `Acoustics::set_reflections(bool)` / `reflections_on()` is the switch, on by default; off, no reflection ray is traced and every probe sends `tap_count = 0`.
- Phase 4 BLOCK, the worded tap check: "every tap's gain equals the source's occlusion multiplier times that tap's gain in the same scene with the wall's transmission set to 1, within 1e-6 relative" does not hold under the amended rule. Transmission 1 leaves the wall's absorption in the multiplier (`0.7^5 = 0.168` for 5 m, `0.7^0.5 = 0.837` for 0.5 m), so each tap over its transmission-1 gain is `mult(T = 0.5) / mult(T = 1) = 0.5`, measured 0.5 on every tap in both scenes, not the multiplier (0.083982, 0.418304). `tests/reflections.rs` holds the check as worded in `every_tap_is_the_multiplier_times_its_transmission_1_gain` under `#[ignore]`; `every_tap_carries_the_source_occlusion_multiplier` asserts each sent tap equals the multiplier times the same scene's tap computed with multiplier 1 (`taps(..., 1.0)`), within 1e-6 relative, and passes. Options: (a) restate the check against the unoccluded tap gain (what the passing test does); (b) restate it against transmission 1 **and** absorption 0 for the occlusion only, which the material table cannot express without also changing the reflection's `1 - A`; (c) keep the worded form, which needs a different tap rule.
- Phase 4: `tests/occlusion.rs` (Phase 3's file, not in Phase 4's Files touched) sets reflections off in its rendered tests and asserts 0 active taps there, as the third amendment directs. `apps/flythrough/src/main.rs` (not in Phase 4's Files touched) prints `audio: N active taps, last acoustics update X ms` after the `F1` frame line; `Acoustics::active_taps()` sums the taps of each voice's last probe. Not run in a window here.
- Phase 4: the reflection fan reuses the occlusion probe's fixed-point ray setup (`fixed_ray`); `reflections()`, `taps()`, `reflection_directions()`, `Reflection`, `REFLECTION_RAYS`, `REFLECTION_RANGE_METRES`, `MAX_DELAY_FRAMES` are public for the tests.

### Close triggers

_(Facts for the architect. No recommendations, and no suggested version bump.)_

- **What shipped:** Phases 1, 2 and 3. Phase 4 blocked (see Notes), uncommitted; Phase 5 not started.
- **Public API changed:** `vxe-world`: `World::trace_materials`, `Run`. `vxe-audio`: `Acoustics`, `BlockClock`, `Occlusion`, `occlusion()`, `AcousticTargets`, `Tap`, `MAX_TAPS`, `OPEN_CUTOFF_HZ`, `Command::SetAcoustics`, `Mixer::clock`, `OutputStats`, `Output::{take_stats, record, take_recording}`.
- **Budget readings:** none yet (Phase 5).
- **Gate at the tip (96de82d, dev box, CARGO_BUILD_JOBS=3, load average 17 to 35 from other lanes):** `cargo build --workspace` exit 0; `cargo nextest run --workspace` under the exclusive lock on the NVIDIA GeForce RTX 3080 Laptop GPU: 146 passed, 1 skipped (`flythrough::look`'s diagnostic speck count, ignored on `main`); `cargo test --workspace --doc` 2 passed; `cargo clippy --workspace --all-targets -- -D warnings` exit 0; `cargo fmt --all --check` exit 0; `cargo deny check` advisories, bans, licenses, sources ok; `node scripts/check-doc-links.mjs` 76 links resolve.
- **Outstanding `human` phases:** none in the plan; listening checks awaiting the owner are in Notes (Phase 1 `H`, Phase 3 `J`, the fix pass's side and click checks).

## Followups

- Reverb from the reflection fan's hit statistics; resonance-driven synthesis hooks.
- HRTF direction interpolation; a device-rate resampler; a probe worker thread.
