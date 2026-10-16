//! The edit budget (`nfr.md` §4): 100 carves of radius 10 voxels (1 m at 10 cm) at distinct
//! seeded positions, each iteration on a fresh solid world so every carve expands uniform
//! bricks, the worst case.

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use vxe_world::hash::mix64;
use vxe_world::*;

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

fn carves(c: &mut Criterion) {
    let ops: Vec<_> = centres()
        .into_iter()
        .map(|p| EditOp::sphere(p, 10 * FIXED_ONE, Mode::Carve))
        .collect();
    c.bench_function("100 radius-10 carves, fresh solid world", |b| {
        b.iter_batched(
            solid_world,
            |world| {
                for op in &ops {
                    std::hint::black_box(world.apply(op).unwrap());
                }
                world
            },
            BatchSize::LargeInput,
        )
    });
}

criterion_group!(benches, carves);
criterion_main!(benches);
