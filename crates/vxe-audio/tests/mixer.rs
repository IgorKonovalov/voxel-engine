//! Headless mixer renders: distance gain, direction, looping, ending, and no allocation per
//! block. Everything runs through the HRTF path except the hard-right panning check, which
//! holds the constant-power panner to its 20 dB.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::f32::consts::{PI, SQRT_2};
use std::sync::Arc;

use vxe_audio::*;

/// Counts heap allocations made on a thread that has switched counting on.
struct Counting;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
}

// SAFETY: forwards every call to `System` unchanged; the thread-locals are `const`-initialised
// `Cell`s, which never allocate.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.with(Cell::get) {
            ALLOCATIONS.with(|a| a.set(a.get() + 1));
        }
        // SAFETY: the caller upholds `GlobalAlloc::alloc`'s contract, passed through.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` came from `System.alloc` with this layout.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if COUNTING.with(Cell::get) {
            ALLOCATIONS.with(|a| a.set(a.get() + 1));
        }
        // SAFETY: as `alloc` and `dealloc`, passed through.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

const MPV: f32 = 0.1;
const FRAMES: usize = 24_000;
const TAIL: usize = 4_800;

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

/// The HRTF path's mixer over the shipped pack.
fn mixer() -> Mixer {
    Mixer::new(Hrtf::sadie2_d2(), MPV)
}

/// `|H(freq)|` of measurement `i`'s left and right impulse responses: the gain a steady tone
/// of `freq` gets through them.
fn gain_at(hrtf: &Arc<Hrtf>, i: usize, freq: f32) -> [f32; 2] {
    hrtf.impulse(i).map(|h| {
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (n, &x) in h.iter().enumerate() {
            let w = 2.0 * std::f64::consts::PI * f64::from(freq) * n as f64 / f64::from(RATE);
            re += f64::from(x) * w.cos();
            im -= f64::from(x) * w.sin();
        }
        re.hypot(im) as f32
    })
}

/// 0.5 s of a 1 kHz tone of amplitude 0.5 at `position` (voxels), listener at the origin
/// facing +x with +z up.
fn render_at(mut m: Mixer, position: [f32; 3]) -> [f32; 2] {
    m.apply(Command::Play(
        VoiceId(1),
        tone(1000.0, 0.5, FRAMES),
        PlayParams::at(position),
    ));
    let mut out = vec![0.0; 2 * FRAMES];
    m.render(&mut out, FRAMES);
    tail_rms(&out)
}

fn assert_within(actual: f32, expected: f32, rel: f32, what: &str) {
    assert!(
        (actual - expected).abs() <= rel * expected,
        "{what}: {actual} vs {expected} (tolerance {rel})"
    );
}

#[test]
fn distance_gain_in_front() {
    // The tone's RMS through the front measurement's two responses, read from the pack.
    let hrtf = Hrtf::sadie2_d2();
    let front = gain_at(&hrtf, hrtf.nearest([1.0, 0.0, 0.0]), 1000.0);
    println!(
        "front-direction gain at 1 kHz: left {}, right {}",
        front[0], front[1]
    );
    let one_m = render_at(mixer(), [10.0, 0.0, 0.0]);
    for (c, rms) in one_m.iter().enumerate() {
        assert_within(
            *rms,
            0.5 / SQRT_2 * front[c],
            0.01,
            &format!("1 m, channel {c}"),
        );
    }
    let two_m = render_at(mixer(), [20.0, 0.0, 0.0]);
    for (c, rms) in two_m.iter().enumerate() {
        assert_within(
            *rms,
            0.5 / SQRT_2 * front[c] / 2.0,
            0.01,
            &format!("2 m, channel {c}"),
        );
    }
    let half_m = render_at(mixer(), [5.0, 0.0, 0.0]);
    for (c, rms) in half_m.iter().enumerate() {
        assert_within(*rms, one_m[c], 0.01, &format!("0.5 m, channel {c}"));
    }
}

#[test]
fn the_panner_at_centre_gives_each_channel_half_the_power() {
    let centre = 0.5 / SQRT_2 * (1.0 / SQRT_2);
    for (c, rms) in render_at(Mixer::panning(MPV), [10.0, 0.0, 0.0])
        .iter()
        .enumerate()
    {
        assert_within(*rms, centre, 0.01, &format!("panned 1 m, channel {c}"));
    }
}

#[test]
fn a_source_on_the_right_is_20_db_louder_on_the_right_when_panned() {
    // Facing +x with +z up, the listener's right is -y.
    let [l, r] = render_at(Mixer::panning(MPV), [0.0, -10.0, 0.0]);
    let db = 20.0 * (r / l.max(1e-12)).log10();
    assert!(db >= 20.0, "right over left {db} dB (left {l}, right {r})");
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
fn a_looping_voice_wraps_without_a_discontinuity() {
    // Ten periods of 480 Hz fill the 1000 samples exactly, so the loop is seamless.
    let short = tone(480.0, 0.5, 1000);
    let long = tone(480.0, 0.5, 48_000);
    let render = |sound: Sound, looping: bool| {
        let mut m = mixer();
        m.apply(Command::Play(
            VoiceId(7),
            sound,
            PlayParams {
                position: [10.0, 0.0, 0.0],
                gain: 1.0,
                looping,
            },
        ));
        let mut out = vec![0.0; 2 * 48_000];
        m.render(&mut out, 48_000);
        (out, m.is_playing(VoiceId(7)))
    };
    let (looped, playing) = render(short, true);
    let (reference, _) = render(long, false);
    assert!(playing, "a looping voice keeps playing");
    let step = max_step(&looped);
    let own = max_step(&reference);
    assert!(
        step <= own * 1.001,
        "loop step {step} vs the tone's own {own}"
    );
    assert!(own > 0.0);
}

#[test]
fn a_one_shot_voice_ends_and_reports_finished() {
    let mut m = mixer();
    m.apply(Command::Play(
        VoiceId(3),
        tone(480.0, 0.5, 1000),
        PlayParams::at([10.0, 0.0, 0.0]),
    ));
    let mut out = vec![0.0; 2 * 2048];
    m.render(&mut out, 2048);
    let mut finished = Vec::new();
    m.drain_finished(|id| finished.push(id));
    assert_eq!(finished, [VoiceId(3)]);
    assert!(!m.is_playing(VoiceId(3)));
    assert_eq!(m.voice_count(), 0);
    // Silence once the sound's 1000 samples have left the 128-tap filter.
    let taps = Hrtf::sadie2_d2().taps();
    assert!(out[2 * (1000 + taps - 1)..].iter().all(|&s| s == 0.0));
}

#[test]
fn sixty_four_voices_render_a_block_without_allocating() {
    let mut m = mixer();
    let sound = tone(440.0, 0.1, 48_000);
    for i in 0..64u32 {
        let a = i as f32 * 0.1;
        m.apply(Command::Play(
            VoiceId(i),
            sound.clone(),
            PlayParams {
                position: [30.0 * a.cos(), 30.0 * a.sin(), 0.0],
                gain: 1.0,
                looping: true,
            },
        ));
    }
    let mut out = vec![0.0; 2 * BLOCK];
    m.render(&mut out, BLOCK);
    COUNTING.with(|c| c.set(true));
    for _ in 0..16 {
        m.render(&mut out, BLOCK);
    }
    COUNTING.with(|c| c.set(false));
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
    assert_eq!(m.voice_count(), 64);
}
