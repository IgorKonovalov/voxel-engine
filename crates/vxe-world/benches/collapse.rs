//! The collapse budgets (`nfr.md` §4 edits): the support query after a radius-10 carve at the
//! centre of a solid 4³-chunk world, where every flood runs to the cap.

use criterion::{Criterion, criterion_group, criterion_main};
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

const PARAMS: SupportParams = SupportParams {
    solid_min: 64,
    max_region: 4096,
    boundary_anchors: true,
};

fn support(c: &mut Criterion) {
    let world = solid_world();
    let mut grid = CoarseGrid::build(&world);
    let r = world
        .apply(&EditOp::sphere(
            centre_of_voxel([0, 0, 0]),
            10 * FIXED_ONE,
            Mode::Carve,
        ))
        .unwrap();
    let changes = grid.update(&world, &r);
    let mut group = c.benchmark_group("collapse");
    group.bench_function(
        format!(
            "find_unsupported after a radius-10 carve, solid 4^3 chunks, {} changed cells",
            changes.len()
        ),
        |b| {
            b.iter(|| {
                let regions =
                    find_unsupported(&world, &grid, &PARAMS, &Anchors::default(), &changes);
                assert!(regions.is_empty());
                regions
            })
        },
    );
    group.finish();
}

criterion_group!(benches, support);
criterion_main!(benches);
