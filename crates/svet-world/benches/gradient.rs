//! The solid-count gradient (`nfr.md`, collision and coarse grids): `World::solid_gradient`
//! at `half` 2 and `half` 6 on a world of `Full` bricks (every third voxel along each axis
//! diagonal solid, so no brick is uniform), at a point off the brick lattice so the cube
//! spans bricks on every axis.

use criterion::{Criterion, criterion_group, criterion_main};
use svet_world::*;

fn full_world() -> World {
    World::generate(
        ChunkExtent::new([-1, -1, -1], [1, 1, 1]),
        &FnSource(|[x, y, z]: [i32; 3]| {
            if (x + y + z).rem_euclid(3) == 0 {
                1
            } else {
                AIR
            }
        }),
        &[Material::named("rock")].into_iter().collect(),
    )
}

fn gradients(c: &mut Criterion) {
    let world = full_world();
    for half in [2u32, 6] {
        c.bench_function(
            &format!("solid_gradient at half {half} on Full bricks"),
            |b| b.iter(|| std::hint::black_box(world.solid_gradient([5, 6, 7], half).unwrap())),
        );
    }
}

criterion_group!(benches, gradients);
criterion_main!(benches);
