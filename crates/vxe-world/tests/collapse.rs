//! Collapse: the support query on the coarse grid, and the drop and remove ops (ADR-0008).
//!
//! Fixture `F` is one chunk `[0, 0, 0]..[1, 1, 1]` of air with three boxes of material 3: a
//! floor `[0, 0, 0]..[64, 64, 8]`, a pillar `[32, 32, 8]..[40, 40, 40]` and a slab
//! `[24, 24, 40]..[48, 48, 48]` (voxel bounds half-open).

mod support;

use support::*;
use vxe_world::*;

const FLOOR: ([i32; 3], [i32; 3]) = ([0, 0, 0], [64, 64, 8]);
const PILLAR: ([i32; 3], [i32; 3]) = ([32, 32, 8], [40, 40, 40]);
const SLAB: ([i32; 3], [i32; 3]) = ([24, 24, 40], [48, 48, 48]);

/// A box op over the half-open voxel bounds `lo..hi`.
fn box_op(lo: [i32; 3], hi: [i32; 3], mode: Mode) -> EditOp {
    let f = |v: [i32; 3]| Fixed3(v.map(|c| i64::from(c) * FIXED_ONE));
    EditOp::box_(f(lo), f(hi), mode)
}

/// One chunk of air with each part filled by a box op, in order.
fn build(parts: &[([i32; 3], [i32; 3], MaterialId)]) -> World {
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [1, 1, 1]),
        &UniformSource(AIR),
        &test_table(),
    );
    for &(lo, hi, m) in parts {
        world.apply(&box_op(lo, hi, Mode::Fill(m))).unwrap();
    }
    world
}

/// Fixture `F`.
fn fixture() -> World {
    build(&[
        (FLOOR.0, FLOOR.1, 3),
        (PILLAR.0, PILLAR.1, 3),
        (SLAB.0, SLAB.1, 3),
    ])
}

fn params() -> SupportParams {
    SupportParams {
        solid_min: 64,
        max_region: 4096,
        boundary_anchors: true,
    }
}

fn carve(world: &World, grid: &mut CoarseGrid, lo: [i32; 3], hi: [i32; 3]) -> Vec<CellChange> {
    let r = world.apply(&box_op(lo, hi, Mode::Carve)).unwrap();
    grid.update(world, &r)
}

fn query(
    world: &World,
    grid: &CoarseGrid,
    p: SupportParams,
    changed: &[CellChange],
) -> Vec<Region> {
    find_unsupported(world, grid, &p, &Anchors::default(), changed)
}

/// The nine slab cells, ascending.
fn slab_cells() -> Vec<CellCoord> {
    let mut v = Vec::new();
    for x in 3..=5 {
        for y in 3..=5 {
            v.push(CellCoord([x, y, 5]));
        }
    }
    v
}

/// The state after the pillar carve, its grid and the changes.
fn carved() -> (World, CoarseGrid, Vec<CellChange>) {
    let world = fixture();
    let mut grid = CoarseGrid::build(&world);
    let changes = carve(&world, &mut grid, PILLAR.0, PILLAR.1);
    (world, grid, changes)
}

#[test]
fn cell_coords_round_trip_through_chunk_and_slot() {
    let c = CellCoord::from(ChunkCoord([-1, 2, 0]), brick_slot([7, 0, 3]));
    assert_eq!(c, [-1, 16, 3]);
    assert_eq!(c.split(), (ChunkCoord([-1, 2, 0]), brick_slot([7, 0, 3])));
    assert_eq!(
        CellCoord([-9, 0, 63]).split(),
        (ChunkCoord([-2, 0, 7]), brick_slot([7, 0, 7]))
    );
    assert_eq!(c.min_voxel(), [-8, 128, 24]);
}

#[test]
fn the_pillar_carve_changes_four_cells_from_512_to_0() {
    let (world, _, changes) = carved();
    let _ = world;
    let cells: Vec<_> = changes.iter().map(CellChange::cell).collect();
    assert_eq!(
        cells,
        (1..=4).map(|z| CellCoord([4, 4, z])).collect::<Vec<_>>()
    );
    for ch in &changes {
        assert_eq!(
            ch.before,
            CoarseCell {
                solid: 512,
                dominant: 3
            }
        );
        assert_eq!(
            ch.after,
            CoarseCell {
                solid: 0,
                dominant: AIR
            }
        );
    }
}

#[test]
fn an_initial_sweep_of_the_fixture_finds_no_region() {
    let world = fixture();
    let grid = CoarseGrid::build(&world);
    let mut all = Vec::new();
    for c in grid.chunks() {
        for (slot, cell) in grid.chunk(c).unwrap().iter().enumerate() {
            all.push(CellChange {
                chunk: c,
                slot: slot as u16,
                before: *cell,
                after: *cell,
            });
        }
    }
    assert_eq!(all.len(), 512);
    assert_eq!(query(&world, &grid, params(), &all), []);
}

#[test]
fn the_pillar_carve_leaves_the_slab_as_one_region_of_4608_voxels() {
    let (world, grid, changes) = carved();
    let regions = query(&world, &grid, params(), &changes);
    assert_eq!(
        regions,
        [Region {
            cells: slab_cells(),
            counts: vec![(3, 4608)],
            aabb: (VoxelCoord([24, 24, 40]), VoxelCoord([47, 47, 47])),
        }]
    );
}

#[test]
fn a_floor_hole_finds_no_region_because_the_floor_is_boundary_anchored() {
    let world = fixture();
    let mut grid = CoarseGrid::build(&world);
    let changes = carve(&world, &mut grid, [8, 8, 0], [16, 16, 8]);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].cell(), [1, 1, 0]);
    assert_eq!(query(&world, &grid, params(), &changes), []);
}

#[test]
fn a_second_pillar_holds_the_slab_until_it_is_carved_too() {
    let second = ([24, 24, 8], [32, 32, 40]);
    let world = build(&[
        (FLOOR.0, FLOOR.1, 3),
        (PILLAR.0, PILLAR.1, 3),
        (SLAB.0, SLAB.1, 3),
        (second.0, second.1, 3),
    ]);
    let mut grid = CoarseGrid::build(&world);
    let first = carve(&world, &mut grid, PILLAR.0, PILLAR.1);
    assert_eq!(first.len(), 4);
    assert_eq!(query(&world, &grid, params(), &first), []);
    let both = carve(&world, &mut grid, second.0, second.1);
    assert_eq!(both.len(), 4);
    let regions = query(&world, &grid, params(), &both);
    assert_eq!(
        regions,
        [Region {
            cells: slab_cells(),
            counts: vec![(3, 4608)],
            aabb: (VoxelCoord([24, 24, 40]), VoxelCoord([47, 47, 47])),
        }]
    );
}

#[test]
fn a_region_over_the_cap_counts_as_supported() {
    let (world, grid, changes) = carved();
    let p = SupportParams {
        max_region: 8,
        ..params()
    };
    assert_eq!(query(&world, &grid, p, &changes), []);
    let p = SupportParams {
        max_region: 9,
        ..params()
    };
    assert_eq!(query(&world, &grid, p, &changes).len(), 1);
}

#[test]
fn a_thinned_pillar_cell_holds_or_drops_by_the_threshold() {
    let world = fixture();
    let mut grid = CoarseGrid::build(&world);
    let r = world
        .apply(&box_op([32, 32, 16], [40, 36, 24], Mode::Carve))
        .unwrap();
    assert_eq!(r.changed_from, [(3, 256)]);
    let changes = grid.update(&world, &r);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].cell(), [4, 4, 2]);
    assert_eq!(
        changes[0].after,
        CoarseCell {
            solid: 256,
            dominant: 3
        }
    );

    let p = SupportParams {
        solid_min: 300,
        ..params()
    };
    let regions = query(&world, &grid, p, &changes);
    let mut cells = slab_cells();
    cells.extend([CellCoord([4, 4, 3]), CellCoord([4, 4, 4])]);
    cells.sort_unstable();
    assert_eq!(cells.len(), 11);
    assert_eq!(
        regions,
        [Region {
            cells,
            counts: vec![(3, 5632)],
            aabb: (VoxelCoord([24, 24, 24]), VoxelCoord([47, 47, 47])),
        }]
    );

    let p = SupportParams {
        solid_min: 200,
        ..params()
    };
    assert_eq!(query(&world, &grid, p, &changes), []);
}

#[test]
fn explicit_and_indestructible_anchors_hold_the_slab() {
    let (world, grid, changes) = carved();
    let anchor = [CellCoord([5, 5, 5])];
    let regions = find_unsupported(
        &world,
        &grid,
        &params(),
        &Anchors { explicit: &anchor },
        &changes,
    );
    assert_eq!(regions, []);

    // Material 4 is indestructible: one slab cell of it anchors the rest.
    let world = build(&[
        (FLOOR.0, FLOOR.1, 3),
        (PILLAR.0, PILLAR.1, 3),
        (SLAB.0, SLAB.1, 3),
    ]);
    world
        .apply(&box_op(
            [40, 40, 40],
            [48, 48, 48],
            Mode::Replace { from: 3, to: 4 },
        ))
        .unwrap();
    let mut grid = CoarseGrid::build(&world);
    let changes = carve(&world, &mut grid, PILLAR.0, PILLAR.1);
    assert_eq!(query(&world, &grid, params(), &changes), []);
}

#[test]
fn a_cell_in_a_chunk_the_grid_does_not_hold_anchors() {
    // Two chunks side by side along x; the slab reaches into the second.
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [2, 1, 1]),
        &UniformSource(AIR),
        &test_table(),
    );
    for (lo, hi) in [FLOOR, PILLAR, ([24, 24, 40], [72, 48, 48])] {
        world.apply(&box_op(lo, hi, Mode::Fill(3))).unwrap();
    }
    let mut grid = CoarseGrid::build(&world);
    let changes = carve(&world, &mut grid, PILLAR.0, PILLAR.1);
    let p = SupportParams {
        boundary_anchors: false,
        ..params()
    };
    // Boundary anchors off: the floor still touches the non-resident chunks outside the
    // extent, and the slab (6 x 3 cells) is unsupported.
    let regions = query(&world, &grid, p, &changes);
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].cells.len(), 18);
    assert_eq!(regions[0].counts, [(3, 9216)]);
    // Once the grid drops the second chunk, the slab's cells there are unknown: anchors.
    grid.chunk_evicted(ChunkCoord([1, 0, 0]));
    assert_eq!(query(&world, &grid, p, &changes), []);
}
