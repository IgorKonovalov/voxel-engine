//! The audio budgets: one 256-frame mixer block with 32 HRTF voices, each low-passed and
//! carrying 16 reflection taps; and one `Acoustics::update` probing 32 sources (7 occlusion
//! rays each, 224, plus the listener's 16 reflection rays) in the fly-through's seed-1 level
//! cave. Prints the audio RAM the mixer and the pack hold.

use std::hint::black_box;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use flythrough::cave::{CaveSource, LEVEL_CHUNKS};
use flythrough::cave_materials;
use vxe_audio::*;
use vxe_world::{ChunkExtent, Material, MaterialTable, World};

const MPV: f32 = 0.1;
const SOURCES: usize = 32;

/// 32 positions (voxel units) on three rings around `centre`, 2 to 12 m out, at different
/// heights, so the voices take different HRTF directions and the probes different paths.
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

fn mixer_block(c: &mut Criterion) {
    let hrtf = Hrtf::sadie2_d2();
    let pack_bytes = hrtf.heap_bytes();
    let mut mixer = Mixer::new(hrtf, MPV);
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
    // Past the start, with the delay lines full and the low-pass and gains settled.
    for _ in 0..200 {
        mixer.render(&mut out, BLOCK);
    }
    assert_eq!(mixer.voice_count(), SOURCES);
    // Every voice owns its history and a delay line of MAX_DELAY_FRAMES plus one block,
    // pooled for MAX_VOICES at construction.
    let delay_line = (MAX_DELAY_FRAMES + BLOCK) * 4;
    println!(
        "audio RAM: pack {pack_bytes} B; delay line {delay_line} B per voice, {} B for the \
         {MAX_VOICES}-voice pool",
        delay_line * MAX_VOICES
    );
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

/// The cave's table with the fly-through's acoustic triple on every row but air.
fn acoustic_table() -> MaterialTable {
    let mut t = MaterialTable::new();
    for row in cave_materials().rows().iter().skip(1) {
        t.push(Material {
            acoustic: [0.3, 0.5, 0.0],
            ..row.clone()
        });
    }
    t
}

fn acoustics_update(c: &mut Criterion) {
    let source = CaveSource::new(1, ChunkExtent::new([0, 0, 0], LEVEL_CHUNKS));
    let world = World::generate(source.extent, &source, &cave_materials());
    let table = acoustic_table();
    let spawn = source.spawn().0.map(|v| v as f32 + 0.5);
    let listener = Listener {
        position: spawn,
        ..Listener::default()
    };
    let voices: Vec<(VoiceId, [f32; 3])> = sources(spawn)
        .into_iter()
        .enumerate()
        .map(|(i, p)| (VoiceId(i as u32 + 1), p))
        .collect();
    let clock = Mixer::new(Hrtf::sadie2_d2(), MPV).clock();
    // A fresh `Acoustics` each iteration has every voice due, so each call probes all 32.
    let mut probe = Acoustics::new(clock.clone());
    let mut sent = 0;
    probe.update(&world, &table, &listener, &voices, MPV, &mut |_| sent += 1);
    assert_eq!(probe.last_probed(), SOURCES);
    assert_eq!(sent, SOURCES);
    println!(
        "acoustics: {} sources probed at the seed-1 level spawn, {} taps",
        probe.last_probed(),
        probe.active_taps()
    );
    c.bench_function(
        "acoustics: update of 32 sources (224 occlusion + 16 reflection rays), seed-1 level",
        |b| {
            b.iter_batched(
                || Acoustics::new(clock.clone()),
                |mut a| {
                    a.update(&world, &table, &listener, &voices, MPV, &mut |cmd| {
                        black_box(cmd);
                    });
                    a
                },
                BatchSize::SmallInput,
            )
        },
    );
}

criterion_group!(benches, mixer_block, acoustics_update);
criterion_main!(benches);
