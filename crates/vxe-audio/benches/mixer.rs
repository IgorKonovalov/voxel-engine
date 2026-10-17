//! The mixer budget: one 256-frame block with 32 HRTF voices, each low-passed and carrying 16
//! reflection taps, with the listener still (every voice on one FIR pair) and turning (every
//! voice crossfading between two FIR pairs, the worst case). Prints the audio RAM the mixer and
//! the pack hold. The `Acoustics::update` budget is the fly-through's `acoustics` bench, which
//! owns the cave it probes.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use vxe_audio::*;

const MPV: f32 = 0.1;
const SOURCES: usize = 32;

/// 32 positions (voxel units) on three rings around `centre`, 2 to 12 m out, at different
/// heights, so the voices take different HRTF directions.
fn sources(centre: [f32; 3]) -> Vec<[f32; 3]> {
    (0..SOURCES)
        .map(|i| {
            let a = i as f32 * std::f32::consts::TAU / SOURCES as f32 * 3.0;
            let r = 20.0 + 50.0 * (i % 3) as f32;
            let z = 10.0 * ((i % 5) as f32 - 2.0);
            [
                centre[0] + r * a.cos(),
                centre[1] + r * a.sin(),
                centre[2] + z,
            ]
        })
        .collect()
}

/// One second of noise from a seeded SplitMix64 stream: broadband, so no filter is idle.
fn noise() -> Sound {
    let mut s = 0x0006_0005u64;
    Sound::new(
        (0..RATE as usize)
            .map(|_| {
                s = s.wrapping_add(0x9e37_79b9_7f4a_7c15);
                let mut z = s;
                z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
                z ^= z >> 31;
                ((z >> 40) as f32 / (1u64 << 24) as f32 - 0.5) * 0.2
            })
            .collect::<Vec<_>>(),
    )
}

/// A mixer at the origin with the 32 looping noise voices, each with 16 taps and a low-pass,
/// rendered past the start so the delay lines are full and the gains and filters settled.
fn loaded_mixer() -> Mixer {
    let mut mixer = Mixer::new(Hrtf::sadie2_d2(), MPV);
    let sound = noise();
    for (i, p) in sources([0.0; 3]).into_iter().enumerate() {
        let id = VoiceId(i as u32 + 1);
        mixer.apply(Command::Play(
            id,
            sound.clone(),
            PlayParams {
                looping: true,
                ..PlayParams::at(p)
            },
        ));
        let mut taps = [Tap::default(); MAX_TAPS];
        for (k, t) in taps.iter_mut().enumerate() {
            *t = Tap {
                delay_frames: 300 + 550 * k as u32 + 7 * i as u32,
                gain: 0.05,
            };
        }
        mixer.apply(Command::SetAcoustics(
            id,
            AcousticTargets {
                gain: 0.5,
                cutoff_hz: 2_000.0 + 100.0 * i as f32,
                taps,
                tap_count: MAX_TAPS as u8,
            },
        ));
    }
    let mut out = vec![0.0f32; 2 * BLOCK];
    for _ in 0..200 {
        mixer.render(&mut out, BLOCK);
    }
    assert_eq!(mixer.voice_count(), SOURCES);
    mixer
}

fn mixer_block(c: &mut Criterion) {
    let mut mixer = loaded_mixer();
    let pack_bytes = mixer.hrtf().map_or(0, |h| h.heap_bytes());
    // Every voice owns its history and a delay line of MAX_DELAY_FRAMES plus one block,
    // pooled for MAX_VOICES at construction.
    let delay_line = (MAX_DELAY_FRAMES + BLOCK) * 4;
    println!(
        "audio RAM: pack {pack_bytes} B; delay line {delay_line} B per voice, {} B for the \
         {MAX_VOICES}-voice pool",
        delay_line * MAX_VOICES
    );
    let mut out = vec![0.0f32; 2 * BLOCK];
    c.bench_function(
        "mixer: one 256-frame block, 32 HRTF voices, low-pass and 16 taps each",
        |b| {
            b.iter(|| {
                mixer.render(&mut out, BLOCK);
                black_box(&out);
            })
        },
    );
}

/// Yaw per block in the turning variant.
const YAW_STEP_DEG: f32 = 90.0;

/// The listener at the origin facing `yaw_deg` about +z.
fn facing(yaw_deg: f32) -> Listener {
    let (s, c) = yaw_deg.to_radians().sin_cos();
    Listener {
        forward: [c, s, 0.0],
        ..Listener::default()
    }
}

/// `p` in the frame of a listener at the origin facing `yaw_deg`: x front, y left, z up.
fn local(p: [f32; 3], yaw_deg: f32) -> [f32; 3] {
    let (s, c) = yaw_deg.to_radians().sin_cos();
    let v = [p[0] * c + p[1] * s, -p[0] * s + p[1] * c, p[2]];
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    v.map(|x| x / len)
}

fn mixer_block_turning(c: &mut Criterion) {
    let mut mixer = loaded_mixer();
    // Each step of YAW_STEP_DEG moves every voice to another measured direction, so every
    // block renders two FIR pairs per voice and crossfades them.
    let hrtf = mixer.hrtf().cloned().expect("the HRTF mixer");
    let steps = (360.0 / YAW_STEP_DEG) as usize;
    for p in sources([0.0; 3]) {
        for k in 0..steps {
            let yaw = k as f32 * YAW_STEP_DEG;
            assert_ne!(
                hrtf.nearest(local(p, yaw)),
                hrtf.nearest(local(p, yaw + YAW_STEP_DEG)),
                "source {p:?} keeps its direction from yaw {yaw}"
            );
        }
    }
    let mut out = vec![0.0f32; 2 * BLOCK];
    let mut k = 0usize;
    c.bench_function(
        "mixer: one 256-frame block, 32 HRTF voices, low-pass and 16 taps each, \
         listener turning (every voice crossfades)",
        |b| {
            b.iter(|| {
                k = (k + 1) % steps;
                mixer.apply(Command::SetListener(facing(k as f32 * YAW_STEP_DEG)));
                mixer.render(&mut out, BLOCK);
                black_box(&out);
            })
        },
    );
}

criterion_group!(benches, mixer_block, mixer_block_turning);
criterion_main!(benches);
