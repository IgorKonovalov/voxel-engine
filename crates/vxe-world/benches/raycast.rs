//! The world's ray walks: 256 seeded rays of up to 200 voxels from four origins in the air
//! of a rock world. A hall of radius 150 voxels is carved around the origin, rock boulders
//! (spheres of radius 4 to 12) are filled back into it and caves (radius 6 to 20) carved
//! around it, so a ray crosses air bricks, chunk edges (the hall spans about five chunks across) and
//! the boulders' `Full` bricks and clear blocks before it hits, through `World::raycast` and
//! through `World::trace_materials`, the walk the audio probes take.

use criterion::{Criterion, criterion_group, criterion_main};
use vxe_world::hash::mix64;
use vxe_world::*;

/// A seeded point within `[-span, span]` on every axis.
fn point(state: &mut u64, span: u64) -> [i32; 3] {
    [0; 3].map(|_| {
        *state = mix64(*state);
        (*state % (2 * span + 1)) as i32 - span as i32
    })
}

fn sphere(world: &World, v: [i32; 3], r: i64, mode: Mode) {
    world
        .apply(&EditOp::sphere(centre_of_voxel(v), r * FIXED_ONE, mode))
        .expect("inside the generated extent");
}

/// The rays' origins: in the hall, each at least 20 voxels from every boulder.
const ORIGINS: [[i32; 3]; 4] = [[0, 0, 0], [30, -20, 10], [-25, 35, -15], [10, 15, 40]];

fn cave_world() -> World {
    let mut table = MaterialTable::new();
    let rock = table.push(Material::named("rock"));
    let world = World::generate(
        ChunkExtent::new([-4, -4, -4], [4, 4, 4]),
        &UniformSource(rock),
        &table,
    );
    sphere(&world, [0; 3], 150, Mode::Carve);
    let mut state = 0x5EED_0044u64;
    let mut boulders = 0;
    while boulders < 40 {
        let v = point(&mut state, 120);
        state = mix64(state);
        let r = 4 + (state % 9) as i64;
        let clear = ORIGINS.iter().all(|o| {
            let d2: i64 = (0..3).map(|k| i64::from(v[k] - o[k]).pow(2)).sum();
            d2 > (r + 20) * (r + 20)
        });
        if clear {
            sphere(&world, v, r, Mode::Fill(rock));
            boulders += 1;
        }
    }
    for _ in 0..120 {
        let v = point(&mut state, 220);
        state = mix64(state);
        sphere(&world, v, 6 + (state % 15) as i64, Mode::Carve);
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
    let origins = ORIGINS.map(centre_of_voxel);
    let dirs = directions();
    let max_t = 200 * FIXED_ONE;
    let mut group = c.benchmark_group("raycast");
    group.bench_function(
        "256 rays of up to 200 voxels through a carved rock world",
        |b| {
            b.iter(|| {
                dirs.iter()
                    .enumerate()
                    .filter(|(i, d)| world.raycast(origins[i % 4], **d, max_t).unwrap().is_some())
                    .count()
            })
        },
    );
    group.bench_function("trace_materials along the same 256 rays", |b| {
        b.iter(|| {
            dirs.iter()
                .enumerate()
                .map(|(i, d)| {
                    world
                        .trace_materials(origins[i % 4], *d, max_t)
                        .unwrap()
                        .len()
                })
                .sum::<usize>()
        })
    });
    group.finish();
}

criterion_group!(benches, rays);
criterion_main!(benches);
