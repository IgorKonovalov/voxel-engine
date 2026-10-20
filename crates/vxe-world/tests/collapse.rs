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
fn boundary_anchors_off_lets_the_floor_go_and_a_missing_chunk_anchors() {
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
    let off = SupportParams {
        boundary_anchors: false,
        ..params()
    };
    let mut floor = Vec::new();
    for x in 0..8 {
        for y in 0..8 {
            floor.push(CellCoord([x, y, 0]));
        }
    }
    let mut slab = Vec::new();
    for x in 3..=8 {
        for y in 3..=5 {
            slab.push(CellCoord([x, y, 5]));
        }
    }
    let slab_region = Region {
        cells: slab,
        counts: vec![(3, 9216)],
        aabb: (VoxelCoord([24, 24, 40]), VoxelCoord([71, 47, 47])),
    };
    // Boundary anchors off: the chunks outside the extent are air, so the floor (first visited,
    // through cell (4, 4, 0) below the carve) and the slab (6 x 3 cells) are both unsupported.
    assert_eq!(
        query(&world, &grid, off, &changes),
        [
            Region {
                cells: floor,
                counts: vec![(3, 32768)],
                aabb: (VoxelCoord([0, 0, 0]), VoxelCoord([63, 63, 7])),
            },
            slab_region.clone(),
        ]
    );
    // Boundary anchors on: the floor lies on the extent's bottom face; the slab alone falls.
    assert_eq!(query(&world, &grid, params(), &changes), [slab_region]);
    // Once the grid drops the second chunk, inside the extent, its cells are unknown: anchors.
    // The floor reaches it through x = 8, the slab directly.
    grid.chunk_evicted(ChunkCoord([1, 0, 0]));
    assert_eq!(query(&world, &grid, off, &changes), []);
}

/// The world's hash after building `parts` on a fresh fixture chunk.
fn direct(parts: &[([i32; 3], [i32; 3], MaterialId)]) -> u64 {
    build(parts).voxel_hash()
}

#[test]
fn the_carved_slab_drops_32_voxels_onto_the_floor() {
    let (world, _, _) = carved();
    let r = world.drop_region(&slab_cells(), 64).unwrap();
    assert_eq!(
        r,
        DropResult {
            moved: vec![(3, 4608)],
            dropped: 32,
            touched: vec![ChunkCoord([0, 0, 0])],
            aabb: Some((VoxelCoord([24, 24, 8]), VoxelCoord([47, 47, 47]))),
        }
    );
    assert_eq!(
        world.voxel_hash(),
        direct(&[(FLOOR.0, FLOOR.1, 3), ([24, 24, 8], [48, 48, 16], 3)])
    );
    let again = world.drop_region(&slab_cells(), 64).unwrap();
    assert_eq!(again.dropped, 0);
    assert_eq!(again.moved, []);
    assert_eq!(again, DropResult::default());
}

#[test]
fn four_drops_of_ten_fall_10_10_10_2_and_end_where_one_drop_of_64_does() {
    let (world, _, _) = carved();
    // The slab's cells move with it: after each drop, the next call names the cells it fell to.
    // A drop of 10 leaves the slab straddling two cell layers, so the region is the cells
    // holding its voxels, read from the grid.
    let mut grid = CoarseGrid::build(&world);
    let mut cells = slab_cells();
    let mut falls = Vec::new();
    for _ in 0..5 {
        let r = world.drop_region(&cells, 10).unwrap();
        falls.push(r.dropped);
        grid.update(&world, &r.edit_result());
        cells = occupied_cells(&grid, [3, 3, 0], [5, 5, 7]);
        cells.retain(|c| c.0[2] >= 1);
    }
    assert_eq!(falls, [10, 10, 10, 2, 0]);

    let (reference, _, _) = carved();
    reference.drop_region(&slab_cells(), 64).unwrap();
    assert_eq!(world.voxel_hash(), reference.voxel_hash());
}

/// Cells in `lo..=hi` whose grid cell holds a solid voxel, ascending.
fn occupied_cells(grid: &CoarseGrid, lo: [i32; 3], hi: [i32; 3]) -> Vec<CellCoord> {
    let mut v = Vec::new();
    for x in lo[0]..=hi[0] {
        for y in lo[1]..=hi[1] {
            for z in lo[2]..=hi[2] {
                let c = CellCoord([x, y, z]);
                if grid.cell_at(c).is_some_and(|cell| cell.solid > 0) {
                    v.push(c);
                }
            }
        }
    }
    v
}

#[test]
fn an_uneven_bottom_drops_by_the_stub_gap() {
    let world = fixture();
    let mut grid = CoarseGrid::build(&world);
    let changes = carve(&world, &mut grid, [32, 32, 8], [40, 40, 32]);
    assert_eq!(changes.len(), 3);
    let regions = query(&world, &grid, params(), &changes);
    assert_eq!(regions.len(), 1);
    let mut cells = slab_cells();
    cells.push(CellCoord([4, 4, 4]));
    cells.sort_unstable();
    assert_eq!(regions[0].cells, cells);
    assert_eq!(regions[0].counts, [(3, 5120)]);

    let r = world.drop_region(&regions[0].cells, 64).unwrap();
    assert_eq!(r.dropped, 24);
    assert_eq!(r.moved, [(3, 5120)]);
    assert_eq!(
        r.aabb,
        Some((VoxelCoord([24, 24, 8]), VoxelCoord([47, 47, 47])))
    );
    assert_eq!(
        world.voxel_hash(),
        direct(&[
            (FLOOR.0, FLOOR.1, 3),
            ([24, 24, 16], [48, 48, 24], 3),
            ([32, 32, 8], [40, 40, 16], 3),
        ])
    );
}

#[test]
fn a_drop_scanning_into_a_non_resident_chunk_changes_nothing() {
    // The floor's chunk is gone: chunk [0, 0, 0] alone stands on chunk [0, 0, -1], absent.
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [1, 1, 1]),
        &UniformSource(AIR),
        &test_table(),
    );
    world.apply(&box_op(SLAB.0, SLAB.1, Mode::Fill(3))).unwrap();
    let before = world.voxel_hash();
    assert_eq!(
        world.drop_region(&slab_cells(), 64),
        Err(EditError::NotResident(ChunkCoord([0, 0, -1])))
    );
    assert_eq!(world.voxel_hash(), before);
    // Within the chunk the slab falls 40 to the chunk's floor.
    assert_eq!(world.drop_region(&slab_cells(), 40).unwrap().dropped, 40);
    assert_eq!(
        world.drop_region(&[CellCoord([0, 0, 8])], 4),
        Err(EditError::NotResident(ChunkCoord([0, 0, 1])))
    );
}

#[test]
fn a_solid_voxel_between_two_region_runs_wedges_the_piece() {
    // Region cells (4, 4, 3) and (4, 4, 5), full of material 3, over a floor up to z = 20; the
    // cell between them, (4, 4, 4), is not in the region. With one voxel of material 5 at its
    // top, [32, 32, 39], the upper cell cannot move without landing on it.
    let lower = ([32, 32, 24], [40, 40, 32]);
    let upper = ([32, 32, 40], [40, 40, 48]);
    let cells = [CellCoord([4, 4, 3]), CellCoord([4, 4, 5])];
    let parts = [
        ([0, 0, 0], [64, 64, 20], 3),
        (lower.0, lower.1, 3),
        (upper.0, upper.1, 3),
    ];

    let world = build(&parts);
    world
        .apply(&box_op([32, 32, 39], [33, 33, 40], Mode::Fill(5)))
        .unwrap();
    let before = world.voxel_hash();
    assert_eq!(
        world.drop_region(&cells, 64).unwrap(),
        DropResult::default()
    );
    assert_eq!(world.voxel_hash(), before);

    // Without the voxel both fall by the lower cell's gap of 4.
    let world = build(&parts);
    let r = world.drop_region(&cells, 64).unwrap();
    assert_eq!(r.dropped, 4);
    assert_eq!(r.moved, [(3, 1024)]);
    assert_eq!(
        world.voxel_hash(),
        direct(&[
            ([0, 0, 0], [64, 64, 20], 3),
            ([32, 32, 20], [40, 40, 28], 3),
            ([32, 32, 36], [40, 40, 44], 3),
        ])
    );
}

#[test]
fn removing_the_slab_piles_a_quarter_of_it_as_rubble_on_the_floor() {
    let (world, _, _) = carved();
    let r = world.remove_region(&slab_cells(), 7, (1, 4), 64).unwrap();
    assert_eq!(r.removed, [(3, 4608)]);
    // 576 columns of 8 carved voxels, 2 rubble voxels each.
    assert_eq!(r.placed, 1152);
    assert_eq!(r.touched, [ChunkCoord([0, 0, 0])]);
    assert_eq!(
        r.aabb,
        Some((VoxelCoord([24, 24, 8]), VoxelCoord([47, 47, 47])))
    );
    // `count_in_box` takes inclusive bounds: the half-open box [24, 24, 8]..[48, 48, 10].
    assert_eq!(
        world.count_in_box([24, 24, 8], [47, 47, 9]).unwrap(),
        [(7, 1152)]
    );
    assert_eq!(
        world.voxel_hash(),
        direct(&[(FLOOR.0, FLOOR.1, 3), ([24, 24, 8], [48, 48, 10], 7)])
    );
}

#[test]
fn remove_region_spares_indestructible_voxels() {
    // Fixture F with the slab voxel [30, 30, 40] of material 4 (indestructible).
    let world = fixture();
    world
        .apply(&box_op(
            [30, 30, 40],
            [31, 31, 41],
            Mode::Replace { from: 3, to: 4 },
        ))
        .unwrap();
    assert_eq!(world.material_at([30, 30, 40]), Some(4));
    let mut grid = CoarseGrid::build(&world);
    let changes = carve(&world, &mut grid, PILLAR.0, PILLAR.1);
    let regions = query(&world, &grid, params(), &changes);
    assert_eq!(
        regions,
        [Region {
            cells: slab_cells(),
            counts: vec![(3, 4607), (4, 1)],
            aabb: (VoxelCoord([24, 24, 40]), VoxelCoord([47, 47, 47])),
        }]
    );

    let r = world.remove_region(&slab_cells(), 7, (1, 4), 64).unwrap();
    assert_eq!(r.removed, [(3, 4607)]);
    assert_eq!(world.material_at([30, 30, 40]), Some(4));
    // Column (30, 30) carved 7 voxels: the accumulator, 0 before it, reaches 7, so h = 1,
    // landing on the spared voxel at z = 41, and 3 carries. Every other column carved 8:
    // 2 rubble voxels on the floor each, the 3 carrying through. floor(4607 / 4) in all.
    assert_eq!(r.placed, 575 * 2 + 1);
    assert_eq!(r.placed, 1151);
    assert_eq!(world.material_at([30, 30, 41]), Some(7));
    assert_eq!(world.material_at([30, 30, 8]), Some(AIR));

    let expected = build(&[
        (FLOOR.0, FLOOR.1, 3),
        ([24, 24, 8], [48, 48, 10], 7),
        ([30, 30, 40], [31, 31, 41], 4),
        ([30, 30, 41], [31, 31, 42], 7),
    ]);
    expected
        .apply(&box_op([30, 30, 8], [31, 31, 10], Mode::Carve))
        .unwrap();
    assert_eq!(world.voxel_hash(), expected.voxel_hash());
}

#[test]
fn no_fraction_leaves_the_floor_alone_and_a_whole_one_piles_the_slab() {
    let (world, _, _) = carved();
    let r = world.remove_region(&slab_cells(), 7, (0, 1), 64).unwrap();
    assert_eq!(r.removed, [(3, 4608)]);
    assert_eq!(r.placed, 0);
    assert_eq!(world.voxel_hash(), direct(&[(FLOOR.0, FLOOR.1, 3)]));

    let (world, _, _) = carved();
    let r = world.remove_region(&slab_cells(), 7, (1, 1), 64).unwrap();
    assert_eq!(r.placed, 4608);
    assert_eq!(
        world.voxel_hash(),
        direct(&[(FLOOR.0, FLOOR.1, 3), ([24, 24, 8], [48, 48, 16], 7)])
    );
}

/// A one-voxel-thick 8 x 8 sheet of material 3 in cell `[1, 1, 2]` (`z = 18`), 10 voxels over
/// fixture F's floor (top at `z = 7`): 64 columns of `n = 1`. At a quarter the accumulator
/// reaches 4 at every fourth column of the cell's order (`y`, then `x`), so 16 rubble voxels
/// land on the floor at local `x` 3 and 7 of every row; per-column rounding piled none.
#[test]
fn a_thin_sheet_piles_a_quarter_by_carrying_the_remainder() {
    let world = build(&[(FLOOR.0, FLOOR.1, 3), ([8, 8, 18], [16, 16, 19], 3)]);
    let r = world
        .remove_region(&[CellCoord([1, 1, 2])], 7, (1, 4), 64)
        .unwrap();
    assert_eq!(r.removed, [(3, 64)]);
    assert_eq!(r.placed, 16);
    let rubble: Vec<[i32; 3]> = (8..16)
        .flat_map(|y| (8..16).map(move |x| [x, y, 8]))
        .filter(|&v| world.material_at(v) == Some(7))
        .collect();
    assert_eq!(
        rubble,
        [
            [11, 8, 8],
            [15, 8, 8],
            [11, 9, 8],
            [15, 9, 8],
            [11, 10, 8],
            [15, 10, 8],
            [11, 11, 8],
            [15, 11, 8],
            [11, 12, 8],
            [15, 12, 8],
            [11, 13, 8],
            [15, 13, 8],
            [11, 14, 8],
            [15, 14, 8],
            [11, 15, 8],
            [15, 15, 8],
        ]
    );
    let expected = build(&[
        (FLOOR.0, FLOOR.1, 3),
        ([11, 8, 8], [12, 16, 9], 7),
        ([15, 8, 8], [16, 16, 9], 7),
    ]);
    assert_eq!(world.voxel_hash(), expected.voxel_hash());
}

#[test]
fn rubble_needs_ground_within_max_depth_and_a_resident_scan() {
    // The floor is 32 below the slab: within 33 voxels, not within 32.
    let (world, _, _) = carved();
    assert_eq!(
        world
            .remove_region(&slab_cells(), 7, (1, 4), 32)
            .unwrap()
            .placed,
        0
    );
    let (world, _, _) = carved();
    assert_eq!(
        world
            .remove_region(&slab_cells(), 7, (1, 4), 33)
            .unwrap()
            .placed,
        1152
    );

    // No floor: the scan leaves the chunk through its bottom face.
    let world = build(&[(SLAB.0, SLAB.1, 3)]);
    let before = world.voxel_hash();
    assert_eq!(
        world.remove_region(&slab_cells(), 7, (1, 4), 64),
        Err(EditError::NotResident(ChunkCoord([0, 0, -1])))
    );
    assert_eq!(world.voxel_hash(), before);
}

/// Feeds a result's counts, chunks and bounds to `h`.
fn feed(
    h: &mut Fnv1a,
    counts: &[(MaterialId, u32)],
    touched: &[ChunkCoord],
    aabb: Option<(VoxelCoord, VoxelCoord)>,
) {
    for &(m, n) in counts {
        h.write(&m.to_le_bytes());
        h.write(&n.to_le_bytes());
    }
    for c in touched {
        for v in c.0 {
            h.write(&v.to_le_bytes());
        }
    }
    if let Some((a, b)) = aabb {
        for v in a.0.into_iter().chain(b.0) {
            h.write(&v.to_le_bytes());
        }
    }
}

fn feed_regions(h: &mut Fnv1a, regions: &[Region]) {
    h.write(&(regions.len() as u32).to_le_bytes());
    for r in regions {
        for c in &r.cells {
            for v in c.0 {
                h.write(&v.to_le_bytes());
            }
        }
        feed(h, &r.counts, &[], Some(r.aabb));
    }
}

fn feed_world(h: &mut Fnv1a, world: &World) {
    h.write(&world.voxel_hash().to_le_bytes());
}

#[test]
fn the_collapse_course_hashes_to_its_pinned_constant() {
    let mut h = Fnv1a::default();

    // Carve the pillar, query, drop the slab in four steps of 10 (and a fifth that lands).
    let world = fixture();
    let mut grid = CoarseGrid::build(&world);
    let r = world
        .apply(&box_op(PILLAR.0, PILLAR.1, Mode::Carve))
        .unwrap();
    feed(&mut h, &r.changed_from, &r.touched, r.aabb);
    feed_world(&mut h, &world);
    let changes = grid.update(&world, &r);
    let regions = query(&world, &grid, params(), &changes);
    feed_regions(&mut h, &regions);
    let mut cells = regions[0].cells.clone();
    let mut falls = Vec::new();
    for _ in 0..4 {
        let d = world.drop_region(&cells, 10).unwrap();
        h.write(&d.dropped.to_le_bytes());
        feed(&mut h, &d.moved, &d.touched, d.aabb);
        feed_world(&mut h, &world);
        falls.push(d.dropped);
        grid.update(&world, &d.edit_result());
        cells = occupied_cells(&grid, [3, 3, 1], [5, 5, 7]);
    }
    assert_eq!(falls, [10, 10, 10, 2]);

    // On a fresh fixture: carve the pillar, query, remove the slab with rubble at 1/4.
    let world = fixture();
    let mut grid = CoarseGrid::build(&world);
    let r = world
        .apply(&box_op(PILLAR.0, PILLAR.1, Mode::Carve))
        .unwrap();
    feed(&mut h, &r.changed_from, &r.touched, r.aabb);
    feed_world(&mut h, &world);
    let changes = grid.update(&world, &r);
    let regions = query(&world, &grid, params(), &changes);
    feed_regions(&mut h, &regions);
    let rm = world
        .remove_region(&regions[0].cells, 7, (1, 4), 64)
        .unwrap();
    h.write(&rm.placed.to_le_bytes());
    feed(&mut h, &rm.removed, &rm.touched, rm.aabb);
    feed_world(&mut h, &world);

    // Blessed on the dev box (Linux); asserted on the Windows CI arm as well.
    assert_eq!(h.0, 0x3178_0a15_61c1_3041, "course hash {:#018x}", h.0);
}

/// The cubes world of the edit tests, dressed by seeded edits so its bricks hold one to four
/// materials (an indestructible 4 among them), built twice for the two paths.
fn dressed_cubes() -> World {
    let world = cubes_world();
    let mut rng = SplitMix64(0x0021_0008);
    for i in 0..120 {
        let c = [0; 3].map(|_| rng.range(-100, 99) * FIXED_ONE);
        let r = rng.range(2 * FIXED_ONE, 9 * FIXED_ONE);
        let mode = match i % 4 {
            0 => Mode::Carve,
            1 => Mode::Fill(2),
            2 => Mode::Replace { from: 1, to: 4 },
            _ => Mode::Fill(3),
        };
        world.apply(&EditOp::sphere(Fixed3(c), r, mode)).unwrap();
    }
    world
}

fn assert_same_bricks(a: &World, b: &World, what: &str) {
    assert_eq!(a.store().coords(), b.store().coords(), "{what}");
    for c in a.store().coords() {
        let x = a.store().get(c).unwrap();
        let y = b.store().get(c).unwrap();
        let (x, y) = (x.read().unwrap(), y.read().unwrap());
        for slot in 0..CHUNK_BRICKS {
            assert_eq!(x.brick(slot), y.brick(slot), "{what}: {c:?} slot {slot}");
        }
    }
}

/// `drop_region` and `remove_region` on 64 seeded regions of the dressed cubes world, each
/// through the native path (layer words and palette merges) on one copy and through decoded
/// voxels on another: equal results and equal bricks after every op.
#[test]
fn the_native_collapse_matches_the_decoded_one() {
    let (native, decoded) = (dressed_cubes(), dressed_cubes());
    assert_same_bricks(&native, &decoded, "before");
    let mut rng = SplitMix64(0x0021_0009);
    let mut moved = 0;
    for i in 0..64 {
        let at = [0; 3].map(|_| rng.range(-14, 12) as i32);
        let size = [0; 3].map(|_| rng.range(1, 3) as i32);
        let mut cells = Vec::new();
        for z in 0..size[2] {
            for y in 0..size[1] {
                for x in 0..size[0] {
                    cells.push(CellCoord([at[0] + x, at[1] + y, at[2] + z]));
                }
            }
        }
        if i % 2 == 0 {
            let drop = rng.range(1, 20) as i32;
            let a = native.drop_region(&cells, drop);
            let b = decoded.drop_region_decoded(&cells, drop);
            assert_eq!(a, b, "op {i}: drop {cells:?} by {drop}");
            moved += usize::from(a.is_ok_and(|r| r.dropped > 0));
        } else {
            let a = native.remove_region(&cells, 5, (1, 4), 24);
            let b = decoded.remove_region_decoded(&cells, 5, (1, 4), 24);
            assert_eq!(a, b, "op {i}: remove {cells:?}");
            moved += usize::from(a.is_ok_and(|r| !r.removed.is_empty()));
        }
        assert_same_bricks(&native, &decoded, &format!("after op {i}"));
    }
    println!("{moved} of 64 ops moved or removed voxels");
    assert!(moved >= 30, "{moved} ops did something");
}

/// `n` distinct cells: the slab's cells first, then cells counting up along x past them.
fn many_cells(n: usize) -> Vec<CellCoord> {
    (0..n)
        .map(|i| {
            let i = i as i32;
            CellCoord([i % 16, (i / 16) % 16, i / 256])
        })
        .collect()
}

/// A drop or a removal naming more than `region_cells_max` (4 096) cells is refused before
/// any scan and changes nothing; 4 096 cells pass the check (and here fail on residency, the
/// cells reaching past the fixture's one chunk).
#[test]
fn region_ops_over_the_cell_cap_are_refused() {
    let world = fixture();
    assert_eq!(world.limits().region_cells_max, 4096);
    let before = world.voxel_hash();
    let refused = Err(EditError::OutOfRange {
        what: Field::RegionCells,
    });
    assert_eq!(world.drop_region(&many_cells(4097), 8), refused);
    assert_eq!(
        world.remove_region(&many_cells(4097), 5, (1, 4), 16),
        refused.map(|_: DropResult| RemoveResult::default())
    );
    assert_eq!(world.voxel_hash(), before);
    assert!(matches!(
        world.drop_region(&many_cells(4096), 8),
        Err(EditError::NotResident(_))
    ));
    assert!(matches!(
        world.remove_region(&many_cells(4096), 5, (1, 4), 16),
        Err(EditError::NotResident(_))
    ));
    assert_eq!(world.voxel_hash(), before);
}

/// The query's step count after the pillar carve. The 4 changed cells `(4, 4, 1..=4)` and
/// their neighbours deduplicate to 22 candidates: the column `(4, 4, 0..=5)` (6) and the four
/// side neighbours of each changed cell (16). Ascending, the first load-bearing one is the
/// floor cell `(4, 4, 0)`, on the extent's face: its flood takes on 1 cell and is anchored.
/// The next is the slab cell `(4, 4, 5)`, whose flood takes on the 9 slab cells and ends as
/// the region. The rest are air or settled: `22 + 1 + 9 = 32`.
#[test]
fn the_support_query_counts_its_candidates_and_flood_cells() {
    let (world, grid, changes) = carved();
    assert_eq!(changes.len(), 4);
    let (regions, steps) =
        find_unsupported_with_stats(&world, &grid, &params(), &Anchors::default(), &changes);
    assert_eq!(regions, query(&world, &grid, params(), &changes));
    assert_eq!(regions[0].cells, slab_cells());
    assert_eq!(steps, 32);
}
