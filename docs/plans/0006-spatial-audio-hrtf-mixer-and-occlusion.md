# 0006: Spatial audio: a headless HRTF mixer on the device callback, occlusion and early reflections through the material table

> **Status:** approved (2026-10-16)
> **Created:** 2026-10-16
> **Milestone:** M4 in `docs/roadmap.md`
> **Related ADRs:** [ADR-0006](../adrs/0006-hrtf-source-and-audio-thread-model.md) (HRTF source, rendering, thread model), ADR-0002 (access model: probes take short chunk read locks), ADR-0001 (stack)
> **Prerequisite:** Plan 0001 closed. Independent of Plans 0002 to 0005 and of their readings.
> **Amended 2026-10-16:** the HRIR pack keeps every 3rd azimuth of each SADIE II D2 ring
> (2950 directions, about 3.0 MB); header is 20 bytes. See "Amendment 2026-10-16" below.
> **Amended 2026-10-16 (second):** Phase 3's partial-cover pillar is 3 x 3 voxels; Phase 4 pins
> the reflection direction set, the hit point (the face) and the tap delay as excess over the
> direct path, so the expected echo is 1160 samples. See "Amendment 2026-10-16 (second)" below.

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
  README.md}`, `crates/vxe-audio/tests/hrtf.rs`, `Cargo.toml` (`sofar` dev pin).
- **Done when:**
  - `cargo run -p vxe-audio --example hrir_pack -- <D2 48 kHz SOFA path> assets/sadie2_d2_48k.hrir`
    (amended 2026-10-16: with `--azimuth-stride 3`, the default) reads the SADIE II subject D2 HRIR SOFA at 48 kHz (from the `D2_HRIR_SOFA.zip` archive on
    the Zenodo record, 38 MB, not committed), truncates each HRIR to 128 taps keeping the onset
    (the pack stores, per direction, the two 128-tap responses starting 8 samples before the
    earlier ear's onset), and writes the pack: a 20-byte header (`b"VXEH"`, version 1, sample
    rate, tap count, direction count), the direction table (azimuth, elevation in degrees as
    `f32`), then the taps. The log records the direction count and the pack's byte size
    (amended 2026-10-16: 2950 directions and 3,044,420 bytes at stride 3; the log records the packer's own counts, and `20 + dirs x 8 + dirs x
    1024` must equal the file size).
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
    path, with gain `(1 - A(m)) * (1 m / (|L - H| + |H - S|))` times the voice's own gain, fed
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
  plan.
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
| 1: `vxe-audio`, the mixer and the output stream | dev | not started | |
| 2: HRTF: the pack, the FIR, the crossfade | dev | not started | |
| 3: Material traversal and occlusion | dev | not started | |
| 4: Early reflections | dev | not started | |
| 5: Benches, readings and the fly-through's audio HUD | dev | not started | |

### Notes

_(Deviations from the plan, with the commit, stated without justification. Done-whens not
satisfiable as stated, and what was done instead. Followups noticed and not acted on. One line
each. Empty is fine.)_

### Close triggers

_(Facts for the architect. No recommendations, and no suggested version bump.)_

- **What shipped:**
- **Public API changed:**
- **Budget readings:**
- **Gate at the tip:**
- **Outstanding `human` phases:**

## Followups

- Reverb from the reflection fan's hit statistics; resonance-driven synthesis hooks.
- HRTF direction interpolation; a device-rate resampler; a probe worker thread.
