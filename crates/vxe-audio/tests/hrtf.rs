//! The HRIR pack and binaural rendering: validation, nearest-direction lookup, interaural
//! level differences, and the direction-change crossfade.

use std::f32::consts::PI;

use vxe_audio::*;

static PACK: &[u8] = include_bytes!("../assets/sadie2_d2_48k.hrir");

const MPV: f32 = 0.1;
const FRAMES: usize = 24_000;
const TAIL: usize = 4_800;

/// SplitMix64: a seeded stream for test inputs.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
}

#[test]
fn the_shipped_pack_loads() {
    let h = Hrtf::load(PACK).unwrap();
    assert_eq!(h.len(), 2950);
    assert_eq!(h.taps(), 128);
    assert_eq!(PACK.len(), 20 + h.len() * 8 + h.len() * 1024);
}

#[test]
fn a_bad_pack_is_rejected() {
    let mut magic = PACK.to_vec();
    magic[0] = b'X';
    assert_eq!(
        Hrtf::load(&magic).unwrap_err(),
        AudioError::Pack("wrong magic")
    );
    let mut rate = PACK.to_vec();
    rate[8..12].copy_from_slice(&44_100u32.to_le_bytes());
    assert_eq!(
        Hrtf::load(&rate).unwrap_err(),
        AudioError::Pack("sample rate is not 48000")
    );
    assert_eq!(
        Hrtf::load(&PACK[..PACK.len() - 1]).unwrap_err(),
        AudioError::Pack("size does not match the header")
    );
    let mut longer = PACK.to_vec();
    longer.extend_from_slice(&[0; 4]);
    assert_eq!(
        Hrtf::load(&longer).unwrap_err(),
        AudioError::Pack("size does not match the header")
    );
    assert_eq!(
        Hrtf::load(&PACK[..10]).unwrap_err(),
        AudioError::Pack("shorter than the header")
    );
}

#[test]
fn the_bin_table_agrees_with_a_brute_force_search() {
    let h = Hrtf::sadie2_d2();
    let mut rng = SplitMix64(0x0006_0002);
    for _ in 0..1000 {
        // Uniform on the sphere: z uniform in [-1, 1], azimuth uniform.
        let z = 2.0 * rng.unit() - 1.0;
        let a = 2.0 * PI * rng.unit();
        let r = (1.0 - z * z).max(0.0).sqrt();
        let d = [r * a.cos(), r * a.sin(), z];
        assert_eq!(h.nearest(d), h.nearest_brute_force(d), "direction {d:?}");
    }
    // The poles and the seams of the table.
    for d in [
        [0.0, 0.0, 1.0],
        [0.0, 0.0, -1.0],
        [-1.0, 0.0, 0.0],
        [-1.0, -1e-7, 0.0],
        [1.0, 0.0, 0.0],
    ] {
        assert_eq!(h.nearest(d), h.nearest_brute_force(d), "direction {d:?}");
    }
}

fn tone(freq: f32, amplitude: f32, len: usize) -> Sound {
    Sound::new(
        (0..len)
            .map(|n| amplitude * (2.0 * PI * freq * n as f32 / RATE as f32).sin())
            .collect::<Vec<_>>(),
    )
}

/// RMS of the left and right channels over the last `TAIL` frames of `out`.
fn tail_rms(out: &[f32]) -> [f32; 2] {
    let tail = &out[out.len() - 2 * TAIL..];
    [0, 1].map(|c| {
        let sum: f64 = tail
            .iter()
            .skip(c)
            .step_by(2)
            .map(|&s| f64::from(s) * f64::from(s))
            .sum();
        (sum / TAIL as f64).sqrt() as f32
    })
}

/// Voxel position at `distance_m` and azimuth `deg` (0 in front, +90 on the listener's right)
/// for the default listener (facing +x, +z up, so the right is -y).
fn at_azimuth(deg: f32, distance_m: f32) -> [f32; 3] {
    let (s, c) = deg.to_radians().sin_cos();
    let r = distance_m / MPV;
    [r * c, -r * s, 0.0]
}

/// 0.5 s of a 2 kHz tone of amplitude 0.5 at azimuth `deg`, 2 m.
fn render_2k(deg: f32) -> Vec<f32> {
    let mut m = Mixer::new(Hrtf::sadie2_d2(), MPV);
    m.apply(Command::Play(
        VoiceId(1),
        tone(2000.0, 0.5, FRAMES),
        PlayParams::at(at_azimuth(deg, 2.0)),
    ));
    let mut out = vec![0.0; 2 * FRAMES];
    m.render(&mut out, FRAMES);
    out
}

fn db(a: f32, b: f32) -> f32 {
    20.0 * (a / b).log10()
}

#[test]
fn a_tone_on_the_right_is_louder_on_the_right() {
    let [l, r] = tail_rms(&render_2k(90.0));
    let right = db(r, l);
    println!("2 kHz at +90 degrees, 2 m: right over left {right:.2} dB (left {l}, right {r})");
    assert!(right >= 6.0, "right over left {right} dB");

    let [l, r] = tail_rms(&render_2k(0.0));
    let front = db(r, l);
    println!("2 kHz in front, 2 m: right over left {front:.2} dB");
    assert!(front.abs() <= 1.0, "front channels differ by {front} dB");

    let [l, r] = tail_rms(&render_2k(-90.0));
    let left = db(l, r);
    println!("2 kHz at -90 degrees, 2 m: left over right {left:.2} dB");
    assert!(
        (left - right).abs() <= 1.0,
        "-90 gives {left} dB, +90 gives {right} dB"
    );
}

fn max_step(out: &[f32]) -> f32 {
    [0, 1]
        .map(|c| {
            let ch: Vec<f32> = out.iter().skip(c).step_by(2).copied().collect();
            ch.windows(2)
                .map(|w| (w[1] - w[0]).abs())
                .fold(0.0, f32::max)
        })
        .into_iter()
        .fold(0.0, f32::max)
}

#[test]
fn a_moving_source_switches_filters_without_a_click() {
    let static_step = max_step(&render_2k(0.0));
    let blocks = RATE as usize / BLOCK + 1;
    let mut m = Mixer::new(Hrtf::sadie2_d2(), MPV);
    m.apply(Command::Play(
        VoiceId(1),
        tone(2000.0, 0.5, blocks * BLOCK),
        PlayParams::at(at_azimuth(-90.0, 2.0)),
    ));
    let mut out = vec![0.0; 2 * blocks * BLOCK];
    for (b, block) in out.chunks_mut(2 * BLOCK).enumerate() {
        let t = (b * BLOCK) as f32 / RATE as f32;
        let deg = -90.0 + 180.0 * t.min(1.0);
        m.apply(Command::SetPosition(VoiceId(1), at_azimuth(deg, 2.0)));
        m.render(block, BLOCK);
    }
    let moving_step = max_step(&out);
    println!(
        "moving -90 to +90 over 1 s at 2 m: max step {moving_step}, static front max step {static_step}"
    );
    assert!(
        moving_step <= 2.0 * static_step,
        "moving step {moving_step} vs static {static_step}"
    );
}

/// One second of 440 Hz at amplitude 0.3 (a whole number of cycles, so it loops cleanly), the
/// phase computed in `f64`: an `f32` phase grows to 2765 rad and jitters by its rounding, which
/// would mask the kinks measured below.
fn clean_hum() -> Sound {
    Sound::new(
        (0..RATE as usize)
            .map(|n| {
                (0.3 * (std::f64::consts::TAU * 440.0 * n as f64 / f64::from(RATE)).sin()) as f32
            })
            .collect::<Vec<_>>(),
    )
}

/// The largest magnitude of the fourth difference over both channels. It is a high-pass of
/// gain `(2 sin(pi f / RATE))^4`: 1.1e-5 at 440 Hz, 2.2 at 10 kHz. A smooth tone shows almost
/// nothing; a step or a change of slope at a block edge shows as a spike of about its size.
fn max_kink(out: &[f32]) -> f32 {
    [0, 1]
        .map(|c| {
            let ch: Vec<f32> = out.iter().skip(c).step_by(2).copied().collect();
            ch.windows(5)
                .map(|w| (w[4] - 4.0 * w[3] + 6.0 * w[2] - 4.0 * w[1] + w[0]).abs())
                .fold(0.0, f32::max)
        })
        .into_iter()
        .fold(0.0, f32::max)
}

/// Frames skipped before measuring, so every render is past its start.
const SETTLE: usize = 9_600;

/// The looping hum at voxel position `source`, rendered for `seconds` in device callbacks of
/// `period` frames, with the listener set from `listener_at(t)` (`t` in seconds) at the top of
/// each callback as the fly-through's frames do.
fn render_hum(
    source: [f32; 3],
    period: usize,
    seconds: f32,
    listener_at: impl Fn(f32) -> Listener,
) -> Vec<f32> {
    let mut m = Mixer::new(Hrtf::sadie2_d2(), MPV);
    m.apply(Command::Play(
        VoiceId(1),
        clean_hum(),
        PlayParams {
            looping: true,
            ..PlayParams::at(source)
        },
    ));
    let frames = (seconds * RATE as f32) as usize;
    let mut out = vec![0.0; 2 * frames];
    for (k, chunk) in out.chunks_mut(2 * period).enumerate() {
        m.apply(Command::SetListener(listener_at(
            (k * period) as f32 / RATE as f32,
        )));
        m.render(chunk, period);
    }
    out
}

fn facing(yaw_deg: f32, position: [f32; 3]) -> Listener {
    let (s, c) = yaw_deg.to_radians().sin_cos();
    Listener {
        position,
        forward: [c, s, 0.0],
        up: [0.0, 0.0, 1.0],
    }
}

#[test]
fn turning_and_walking_past_a_tone_add_no_kinks_at_block_edges() {
    // The static reference: the hum 1 m and 2 m away (the nearest and farthest the moving
    // renders come) at 24 azimuths each, the loudest kink of any of them.
    let mut static_kink = 0.0f32;
    for r in [10.0, 20.0] {
        for k in 0..24 {
            let a = (15.0 * k as f32).to_radians();
            let source = [r * a.cos(), r * a.sin(), 0.0];
            let out = render_hum(source, BLOCK, 1.0, |_| Listener::default());
            static_kink = static_kink.max(max_kink(&out[2 * SETTLE..]));
        }
    }
    println!("static hum at 1 m and 2 m, 24 azimuths: max kink {static_kink:e}");
    let bound = 4.0 * static_kink;
    // 512 frames is the period cpal opens on the dev box's PipeWire default device.
    for period in [BLOCK, 2 * BLOCK, 4 * BLOCK] {
        for deg_per_s in [90.0, 360.0] {
            let out = render_hum([20.0, 0.0, 0.0], period, 2.0, |t| {
                facing(deg_per_s * t, [0.0; 3])
            });
            let kink = max_kink(&out[2 * SETTLE..]);
            println!(
                "turning {deg_per_s} deg/s, {period}-frame callbacks: max kink {kink:e} ({:.1} x static)",
                kink / static_kink
            );
            assert!(kink <= bound, "turning: {kink} against 4 x {static_kink}");
        }
        // Walking at 3 m/s past the hum, 1 m to the side at the closest: distance and
        // direction both change.
        let out = render_hum([0.0, 10.0, 0.0], period, 4.0, |t| {
            facing(0.0, [-60.0 + 30.0 * t, 0.0, 0.0])
        });
        let kink = max_kink(&out[2 * SETTLE..]);
        println!(
            "walking past at 3 m/s, {period}-frame callbacks: max kink {kink:e} ({:.1} x static)",
            kink / static_kink
        );
        assert!(kink <= bound, "walking: {kink} against 4 x {static_kink}");
    }
}
