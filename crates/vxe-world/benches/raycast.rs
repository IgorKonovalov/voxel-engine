//! The world's ray walks: 256 seeded rays of up to 200 voxels from the middle of a rock world
//! carved into caves (spheres of radius 6 to 20 voxels, so the walk crosses `Full` bricks,
//! clear blocks, air bricks and chunk edges), through `World::raycast` and through
//! `World::trace_materials`, the walk the audio probes take.

use criterion::{Criterion, criterion_group, criterion_main};
use vxe_world::hash::mix64;
use vxe_world::*;

fn cave_world() -> World {
    let mut table = MaterialTable::new();
    let rock = table.push(Material::named("rock"));
    let world = World::generate(
        ChunkExtent::new([-3, -3, -3], [3, 3, 3]),
        &UniformSource(rock),
        &table,
    );
    let mut state = 0x5EED_0044u64;
    for _ in 0..120 {
        let v = [0; 3].map(|_| {
            state = mix64(state);
            (state % 301) as i32 - 150
        });
        state = mix64(state);
        let r = 6 + (state % 15) as i64;
        world
            .apply(&EditOp::sphere(
                centre_of_voxel(v),
                r * FIXED_ONE,
                Mode::Carve,
            ))
            .expect("inside the generated extent");
    }
    world
}

/// 256 seeded directions of length 256 (fixed units per unit of `t`) or near it.
fn directions() -> Vec<[i64; 3]> {
    let mut state = 0x5EED_0045u64;
    (0..256)
        .map(|_| {
            [0; 3].map(|_| {
                state = mix64(state);
                (state % 513) as i64 - 256
            })
        })
        .filter(|d| *d != [0; 3])
        .collect()
}

fn rays(c: &mut Criterion) {
    let world = cave_world();
    let origin = centre_of_voxel([0, 0, 0]);
    let dirs = directions();
    let max_t = 200 * FIXED_ONE;
    let mut group = c.benchmark_group("raycast");
    group.bench_function(
        "256 rays of up to 200 voxels through a carved rock world",
        |b| {
            b.iter(|| {
                dirs.iter()
                    .filter(|d| world.raycast(origin, **d, max_t).unwrap().is_some())
                    .count()
            })
        },
    );
    group.bench_function("trace_materials along the same 256 rays", |b| {
        b.iter(|| {
            dirs.iter()
                .map(|d| world.trace_materials(origin, *d, max_t).unwrap().len())
                .sum::<usize>()
        })
    });
    group.finish();
}

criterion_group!(benches, rays);
criterion_main!(benches);
