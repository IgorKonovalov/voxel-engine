//! Replay throughput (`nfr.md` §4): a log file of 10 000 radius-1 m carves at seeded centres
//! inside a solid 4³-chunk world, decoded and replayed. Each iteration replays onto a fresh
//! world; only the decode and the replay are timed.

use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use vxe_world::hash::mix64;
use vxe_world::*;

const CARVES: usize = 10_000;

/// Chunks `[-2, 2)³`: voxels `[-128, 128)` on every axis.
fn extent() -> ChunkExtent {
    ChunkExtent::new([-2, -2, -2], [2, 2, 2])
}

fn solid_world() -> World {
    let mut table = MaterialTable::new();
    let rock = table.push(Material::named("rock"));
    World::generate(extent(), &UniformSource(rock), &table)
}

/// The log file: `CARVES` 1 m carves, one per tick, centres within ±117 voxels so every
/// sphere stays inside the world, pushed with `OpLog::push` (no boundary).
fn log_file() -> LogFile {
    let mut ops = OpLog::new();
    let mut state = 0x5EED_0007u64;
    for tick in 0..CARVES as u64 {
        let v = [0; 3].map(|_| {
            state = mix64(state);
            (state % 235) as i32 - 117
        });
        ops.push(
            tick,
            EditOp::sphere(centre_of_voxel(v), 10 * FIXED_ONE, Mode::Carve),
        );
    }
    LogFile {
        seed: 7,
        source_version: 1,
        extent: extent(),
        ops,
        inputs: InputLog::new(),
    }
}

#[allow(
    clippy::disallowed_methods,
    reason = "the bench times decode and replay alone, between untimed world generation"
)]
fn timed<T>(f: impl FnOnce() -> T) -> (T, Duration) {
    let t = Instant::now();
    let out = f();
    (out, t.elapsed())
}

fn replay(c: &mut Criterion) {
    let bytes = log_file().encode();
    assert_eq!(bytes.len(), 60 + 470_000, "47 bytes per sphere carve entry");
    println!(
        "replay: log file {} bytes, {CARVES} op entries",
        bytes.len()
    );

    let mut group = c.benchmark_group("replay");
    group.sample_size(10);
    group.bench_function(
        "decode + replay 10000 radius-1 m carves, solid 4^3 chunks",
        |b| {
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let world = solid_world();
                    let (results, dt) = timed(|| {
                        let file = LogFile::decode(&bytes).unwrap();
                        world.replay(&file.ops)
                    });
                    assert_eq!(results.len(), CARVES);
                    assert!(results.iter().all(Result::is_ok));
                    std::hint::black_box(results);
                    total += dt;
                }
                total
            })
        },
    );
    group.finish();
}

criterion_group!(benches, replay);
criterion_main!(benches);
