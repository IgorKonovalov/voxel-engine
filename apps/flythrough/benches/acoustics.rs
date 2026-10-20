//! The acoustics budget: one `Acoustics::update` probing 32 sources (7 occlusion rays each,
//! 224, plus the listener's 16 reflection rays) in the fly-through's seed-1 level cave. It
//! lives with the fly-through because the cave does; the mixer's budget is in `vxe-audio`'s
//! own `mixer` bench.

use std::hint::black_box;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use flythrough::cave::{CaveSource, LEVEL_CHUNKS};
use flythrough::cave_materials;
use vxe_audio::*;
use vxe_world::{ChunkExtent, Material, MaterialTable, World};

const MPV: f32 = 0.1;
const SOURCES: usize = 32;

/// 32 positions (voxel units) on three rings around `centre`, 2 to 12 m out, at different
/// heights, so the probes take different paths. The same layout as the mixer bench's voices.
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

criterion_group!(benches, acoustics_update);
criterion_main!(benches);
