//! The mixer budget: one 256-frame block with 32 HRTF voices, each low-passed and carrying 16
//! reflection taps into the 16 direction buses and a send into the late tail, with the
//! listener still (every voice and bus on one FIR pair) and turning (every voice and bus
//! crossfading between two FIR pairs, the worst case). The tail's room alternates every 0.33 s
//! between an 8 m room (2.26 s) and a 10 m one (2.83 s), as a walk through a cave hands it
//! rooms, so both of its networks run: one fed, the other ringing out or fading. Each case
//! runs again with the direct path's propagation delay on (every voice and tap reading the
//! delay line at the source's distance). Prints the audio RAM the
//! mixer and the pack hold. The `Acoustics::update` budget is the fly-through's `acoustics`
//! bench, which owns the cave it probes.

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
fn loaded_mixer(direct_delay: bool) -> Mixer {
    let mut mixer = Mixer::new(Hrtf::sadie2_d2(), MPV);
    mixer.set_mix(MixSettings {
        direct_delay,
        ..MixSettings::default()
    });
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
                send: 0.5,
            },
        ));
    }
    let mut out = vec![0.0f32; 2 * BLOCK];
    let mut rooms = Rooms(0);
    for _ in 0..200 {
        rooms.step(&mut mixer);
        mixer.render(&mut out, BLOCK);
    }
    assert_eq!(mixer.voice_count(), SOURCES);
    assert!(
        mixer.reverb_networks().iter().all(|n| n.holds_signal),
        "both tail networks run"
    );
    mixer
}

/// The two rooms the tail alternates between.
const ROOMS: [RoomTargets; 2] = [
    RoomTargets {
        t60_s: 2.26,
        mean_free_path_m: 8.0,
        energy: 0.379,
    },
    RoomTargets {
        t60_s: 2.83,
        mean_free_path_m: 10.0,
        energy: 0.243,
    },
];
/// Blocks between room changes: 0.33 s.
const ROOM_BLOCKS: usize = 62;

/// Blocks rendered; sends the next room every [`ROOM_BLOCKS`].
struct Rooms(usize);

impl Rooms {
    fn step(&mut self, mixer: &mut Mixer) {
        if self.0.is_multiple_of(ROOM_BLOCKS) {
            mixer.apply(Command::SetRoom(ROOMS[self.0 / ROOM_BLOCKS % 2]));
        }
        self.0 += 1;
    }
}

/// Prints the median time of the blocks that clear a tail network (a fade-out ending or a
/// handover taking a network) against the median of the other blocks, over 40 room changes.
/// Two passes over the same deterministic sequence: the first finds the clearing blocks (a
/// network that held signal holds none after), the second times every block.
#[allow(
    clippy::disallowed_methods,
    reason = "a bench times single blocks on the wall clock; nothing here is the deterministic world"
)]
fn clearing_block() {
    const BLOCKS: usize = 40 * ROOM_BLOCKS;
    let mut mixer = loaded_mixer(false);
    let mut rooms = Rooms(200);
    let mut out = vec![0.0f32; 2 * BLOCK];
    let mut clears = vec![false; BLOCKS];
    for c in clears.iter_mut() {
        let before = mixer.reverb_networks().map(|n| n.holds_signal);
        rooms.step(&mut mixer);
        mixer.render(&mut out, BLOCK);
        let after = mixer.reverb_networks().map(|n| n.holds_signal);
        *c = before.iter().zip(&after).any(|(b, a)| *b && !*a);
    }
    let mut mixer = loaded_mixer(false);
    let mut rooms = Rooms(200);
    let (mut clearing, mut other) = (Vec::new(), Vec::new());
    for &c in &clears {
        rooms.step(&mut mixer);
        let t = std::time::Instant::now();
        mixer.render(&mut out, BLOCK);
        let ms = t.elapsed().as_secs_f64() * 1e3;
        black_box(&out);
        if c { clearing.push(ms) } else { other.push(ms) }
    }
    let median = |v: &mut Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v.get(v.len() / 2).copied().unwrap_or(f64::NAN)
    };
    println!(
        "tail: {} blocks clear a network, median {:.4} ms; the other {} blocks, median {:.4} ms",
        clearing.len(),
        median(&mut clearing),
        other.len(),
        median(&mut other)
    );
}

fn mixer_block(c: &mut Criterion) {
    clearing_block();
    let mut mixer = loaded_mixer(false);
    let pack_bytes = mixer.hrtf().map_or(0, |h| h.heap_bytes());
    // Every voice owns its history and a delay line of MAX_DELAY_FRAMES plus one block,
    // pooled for MAX_VOICES at construction.
    let delay_line = (MAX_DELAY_FRAMES + BLOCK) * 4;
    // Each bus holds one block of input and the HRIR's history; each voice its 16 slot states.
    let taps = mixer.hrtf().map_or(1, |h| h.taps());
    let buses = MAX_TAPS * (BLOCK + taps - 1) * 4;
    let slots = MAX_TAPS * 8;
    println!(
        "audio RAM: pack {pack_bytes} B; delay line {delay_line} B per voice, {} B for the \
         {MAX_VOICES}-voice pool; buses {buses} B; tap slots {slots} B per voice, {} B for the \
         pool; tail lines {} B (two networks); pre-delay line {} B",
        delay_line * MAX_VOICES,
        slots * MAX_VOICES,
        mixer.reverb_bytes(),
        mixer.predelay_bytes()
    );
    let mut out = vec![0.0f32; 2 * BLOCK];
    let mut rooms = Rooms(1);
    c.bench_function(
        "mixer: one 256-frame block, 32 HRTF voices, low-pass and 16 taps each, 16 buses, tail \
         handing over",
        |b| {
            b.iter(|| {
                rooms.step(&mut mixer);
                mixer.render(&mut out, BLOCK);
                black_box(&out);
            })
        },
    );
    let mut mixer = loaded_mixer(true);
    c.bench_function(
        "mixer: one 256-frame block, 32 HRTF voices, low-pass and 16 taps each, 16 buses, tail \
         handing over, direct delay",
        |b| {
            b.iter(|| {
                rooms.step(&mut mixer);
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
    for direct_delay in [false, true] {
        mixer_block_turning_with(c, direct_delay);
    }
}

fn mixer_block_turning_with(c: &mut Criterion, direct_delay: bool) {
    let mut mixer = loaded_mixer(direct_delay);
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
    let mut rooms = Rooms(1);
    let name = format!(
        "mixer: one 256-frame block, 32 HRTF voices, low-pass and 16 taps each, 16 buses, \
         tail handing over, listener turning (every voice crossfades){}",
        if direct_delay { ", direct delay" } else { "" }
    );
    c.bench_function(&name, |b| {
        b.iter(|| {
            rooms.step(&mut mixer);
            k = (k + 1) % steps;
            mixer.apply(Command::SetListener(facing(k as f32 * YAW_STEP_DEG)));
            mixer.render(&mut out, BLOCK);
            black_box(&out);
        })
    });
}

criterion_group!(benches, mixer_block, mixer_block_turning);
criterion_main!(benches);
