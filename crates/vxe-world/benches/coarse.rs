//! The coarse grid budget (`nfr.md`, collision and coarse grids): 100 grid updates, each after
//! a radius-10 carve at a seeded position on a solid world, and the 100 walkable-map updates
//! from those changes. Each iteration carves a fresh world; only the updates are timed.

use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use vxe_world::hash::mix64;
use vxe_world::*;

/// Walkable cells: at most 64 solid voxels, on a floor cell of at least 448, under two open
/// cells.
const WALKABLE: WalkableParams = WalkableParams {
    max_solid: 64,
    min_floor: 448,
    clearance: 2,
};

fn solid_world() -> World {
    let mut table = MaterialTable::new();
    let rock = table.push(Material::named("rock"));
    World::generate(
        ChunkExtent::new([-2, -2, -2], [2, 2, 2]),
        &UniformSource(rock),
        &table,
    )
}

/// 100 distinct centres within ±100 voxels of the origin, so every sphere stays resident.
fn centres() -> Vec<Fixed3> {
    let mut out: Vec<Fixed3> = Vec::new();
    let mut state = 0x5EED_0004u64;
    while out.len() < 100 {
        let v = [0; 3].map(|_| {
            state = mix64(state);
            (state % 201) as i32 - 100
        });
        let c = centre_of_voxel(v);
        if !out.contains(&c) {
            out.push(c);
        }
    }
    out
}

#[allow(
    clippy::disallowed_methods,
    reason = "the bench times the grid updates alone, between untimed carves"
)]
fn timed<T>(f: impl FnOnce() -> T) -> (T, Duration) {
    let t = Instant::now();
    let out = f();
    (out, t.elapsed())
}

fn updates(c: &mut Criterion) {
    let ops: Vec<_> = centres()
        .into_iter()
        .map(|p| EditOp::sphere(p, 10 * FIXED_ONE, Mode::Carve))
        .collect();
    let mut group = c.benchmark_group("coarse");
    group.bench_function(
        "100 grid updates after radius-10 carves, solid world",
        |b| {
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let world = solid_world();
                    let mut grid = CoarseGrid::build(&world);
                    for op in &ops {
                        let r = world.apply(op).unwrap();
                        let (changes, dt) = timed(|| grid.update(&world, &r));
                        std::hint::black_box(changes);
                        total += dt;
                    }
                }
                total
            })
        },
    );
    group.bench_function("100 walkable updates from those changes", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let world = solid_world();
                let mut grid = CoarseGrid::build(&world);
                let mut map = WalkableMap::build(&grid, WALKABLE);
                for op in &ops {
                    let r = world.apply(op).unwrap();
                    let changes = grid.update(&world, &r);
                    let (flipped, dt) = timed(|| map.update(&grid, &changes));
                    std::hint::black_box(flipped);
                    total += dt;
                }
            }
            total
        })
    });
    group.finish();
}

criterion_group!(benches, updates);
criterion_main!(benches);
