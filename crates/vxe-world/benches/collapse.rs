//! The collapse budgets (`nfr.md` §4 edits): the support query after a radius-10 carve at the
//! centre of a solid 4³-chunk world, where every flood runs to the cap; the drop of the test
//! fixture's 9-cell slab and its removal with rubble; the slab's drop through a 32-voxel sliver
//! with a yield threshold of 64 (ADR-0016) and a pile of 32 rubble voxels on it; and a 64-voxel
//! drop of a 4096-cell cube, plainly and with a yield threshold of 64.
//! The drops by 10 and by 60 are not multiples of the brick edge, so every voxel moves one at a
//! time; the drops by 32, 64 and 8 move whole bricks. The ops mutate the world, so each
//! iteration builds a fresh one and only the op is timed.

use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use vxe_world::*;

#[allow(
    clippy::disallowed_methods,
    reason = "the bench times the op alone, between untimed world builds"
)]
fn timed<T>(f: impl FnOnce() -> T) -> (T, Duration) {
    let t = Instant::now();
    let out = f();
    (out, t.elapsed())
}

/// A box op over the half-open voxel bounds `lo..hi`.
fn box_op(lo: [i32; 3], hi: [i32; 3], mode: Mode) -> EditOp {
    let f = |v: [i32; 3]| Fixed3(v.map(|c| i64::from(c) * FIXED_ONE));
    EditOp::box_(f(lo), f(hi), mode)
}

/// Air over `extent` with each part filled with material 1, in order.
fn build(extent: ChunkExtent, parts: &[([i32; 3], [i32; 3])]) -> World {
    let mut table = MaterialTable::new();
    let rock = table.push(Material::named("rock"));
    let world = World::generate(extent, &UniformSource(AIR), &table);
    for &(lo, hi) in parts {
        world.apply(&box_op(lo, hi, Mode::Fill(rock))).unwrap();
    }
    world
}

/// The test fixture after the pillar carve: a floor and the 9-cell slab 32 voxels above it.
fn carved_fixture() -> World {
    build(
        ChunkExtent::new([0, 0, 0], [1, 1, 1]),
        &[([0, 0, 0], [64, 64, 8]), ([24, 24, 40], [48, 48, 48])],
    )
}

/// The test fixture with the pillar carved but a 32-voxel sliver, `[32, 32, 39]..[36, 40, 40]`,
/// under the slab: half the 64-voxel threshold.
fn sliver_fixture() -> World {
    build(
        ChunkExtent::new([0, 0, 0], [1, 1, 1]),
        &[
            ([0, 0, 0], [64, 64, 8]),
            ([32, 32, 39], [36, 40, 40]),
            ([24, 24, 40], [48, 48, 48]),
        ],
    )
}

fn slab_cells() -> Vec<CellCoord> {
    let mut v = Vec::new();
    for x in 3..=5 {
        for y in 3..=5 {
            v.push(CellCoord([x, y, 5]));
        }
    }
    v
}

/// A floor and a 16³-cell cube (128³ voxels) 64 voxels above it, in 2 x 2 x 4 chunks.
fn cube_world() -> (World, Vec<CellCoord>) {
    let world = build(
        ChunkExtent::new([0, 0, 0], [2, 2, 4]),
        &[([0, 0, 0], [128, 128, 64]), ([0, 0, 128], [128, 128, 256])],
    );
    let mut cells = Vec::new();
    for x in 0..16 {
        for y in 0..16 {
            for z in 16..32 {
                cells.push(CellCoord([x, y, z]));
            }
        }
    }
    (world, cells)
}

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

fn drops(c: &mut Criterion) {
    let mut group = c.benchmark_group("collapse");
    let slab = slab_cells();
    group.bench_function("drop_region of the fixture's 9-cell slab, 32 voxels", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let world = carved_fixture();
                let (r, dt) = timed(|| world.drop_region(&slab, 64).unwrap());
                assert_eq!(r.dropped, 32);
                total += dt;
            }
            total
        })
    });
    group.bench_function("drop_region of the fixture's 9-cell slab, 10 voxels", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let world = carved_fixture();
                let (r, dt) = timed(|| world.drop_region(&slab, 10).unwrap());
                assert_eq!(r.dropped, 10);
                assert_eq!(r.moved, [(1, 4608)]);
                total += dt;
            }
            total
        })
    });
    group.bench_function(
        "remove_region of the fixture's 9-cell slab, rubble 1/4",
        |b| {
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let world = carved_fixture();
                    let (r, dt) = timed(|| world.remove_region(&slab, 2, (1, 4), 64).unwrap());
                    assert_eq!(r.placed, 1152);
                    total += dt;
                }
                total
            })
        },
    );
    group.bench_function(
        "drop_region_through of the 9-cell slab through a 32-voxel sliver, 32 voxels",
        |b| {
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let world = sliver_fixture();
                    let (r, dt) = timed(|| world.drop_region_through(&slab, 64, 64).unwrap());
                    assert_eq!(r.dropped, 32);
                    assert_eq!(r.crushed, [(1, 32)]);
                    total += dt;
                }
                total
            })
        },
    );
    group.bench_function("pile_rubble of 32 on the 9-cell slab", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let world = carved_fixture();
                let (r, dt) = timed(|| world.pile_rubble(&slab, 2, 32, 64).unwrap());
                assert_eq!(r.placed, 32);
                total += dt;
            }
            total
        })
    });
    group.sample_size(10);
    group.bench_function(
        "drop_region_through of a 4096-cell cube, 64 voxels, yield 64",
        |b| {
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let (world, cells) = cube_world();
                    let (r, dt) = timed(|| world.drop_region_through(&cells, 64, 64).unwrap());
                    assert_eq!(r.dropped, 64);
                    assert_eq!(r.moved, [(1, 2_097_152)]);
                    assert_eq!(r.crushed, []);
                    total += dt;
                }
                total
            })
        },
    );
    group.bench_function("drop_region of a 4096-cell cube, 64 voxels", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let (world, cells) = cube_world();
                let (r, dt) = timed(|| world.drop_region(&cells, 64).unwrap());
                assert_eq!(r.dropped, 64);
                assert_eq!(r.moved, [(1, 2_097_152)]);
                total += dt;
            }
            total
        })
    });
    for by in [60, 8] {
        group.bench_function(
            format!("drop_region of a 4096-cell cube, {by} voxels"),
            |b| {
                b.iter_custom(|iters| {
                    let mut total = Duration::ZERO;
                    for _ in 0..iters {
                        let (world, cells) = cube_world();
                        let (r, dt) = timed(|| world.drop_region(&cells, by).unwrap());
                        assert_eq!(r.dropped, by);
                        assert_eq!(r.moved, [(1, 2_097_152)]);
                        total += dt;
                    }
                    total
                })
            },
        );
    }
    group.finish();
}

criterion_group!(benches, support, drops);
criterion_main!(benches);
