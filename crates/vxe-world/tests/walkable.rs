//! Walkable space over the coarse grid: build, incremental update, and equivalence with a
//! full rebuild.

mod support;

use support::*;
use vxe_world::*;

const PARAMS: WalkableParams = WalkableParams {
    max_solid: 64,
    min_floor: 448,
    clearance: 2,
};

/// F1: material 1 solid iff `z < 0`, over chunks `[-2, 2)³`.
fn floor_world() -> World {
    World::generate(
        test_extent(),
        &FnSource(|v: VoxelCoord| if v.0[2] < 0 { 1 } else { AIR }),
        &test_table(),
    )
}

/// Chunk and slot of global brick `g`.
fn cell(g: [i32; 3]) -> (ChunkCoord, u16) {
    (
        ChunkCoord(g.map(|v| v.div_euclid(8))),
        brick_slot(g.map(|v| v.rem_euclid(8))) as u16,
    )
}

#[test]
fn the_floor_world_is_walkable_on_one_layer() {
    let world = floor_world();
    let map = WalkableMap::build(&CoarseGrid::build(&world), PARAMS);
    assert_eq!(map.count(), 1024);
    let cells = map.cells();
    assert_eq!(cells.len(), 1024);
    let mut columns: Vec<[i32; 2]> = Vec::new();
    for (c, slot) in cells {
        let b = brick_of_slot(usize::from(slot));
        let g = [0, 1, 2].map(|k| c.0[k] * 8 + b[k]);
        assert_eq!(g[2], 0, "walkable cell at brick z {}", g[2]);
        assert!((-16..16).contains(&g[0]) && (-16..16).contains(&g[1]));
        columns.push([g[0], g[1]]);
    }
    columns.sort_unstable();
    columns.dedup();
    assert_eq!(columns.len(), 1024);
}

#[test]
fn a_pillar_moves_the_walkable_cell_to_its_top() {
    let world = floor_world();
    let mut grid = CoarseGrid::build(&world);
    let mut map = WalkableMap::build(&grid, PARAMS);
    let r = world
        .apply(&EditOp::box_(
            Fixed3([0, 0, 0]),
            Fixed3([2048, 2048, 6144]),
            Mode::Fill(1),
        ))
        .unwrap();
    assert_eq!(r.changed_from, [(AIR, 3 * 512)]);
    let changes = grid.update(&world, &r);
    assert_eq!(changes.len(), 3);
    let (flipped, stats) = map.update_with_stats(&grid, &changes);
    assert_eq!(flipped, [cell([0, 0, 0]), cell([0, 0, 3])]);
    let (c0, s0) = cell([0, 0, 0]);
    let (c3, s3) = cell([0, 0, 3]);
    assert!(!map.is_walkable(c0, usize::from(s0)));
    assert!(map.is_walkable(c3, usize::from(s3)));
    assert_eq!(map.count(), 1024);
    // Each change at brick z recomputes z - clearance ..= z + 1; for z in 0..3 that is the
    // six distinct cells z = -2 ..= 3 of the column.
    assert_eq!(stats.cells_recomputed, 6);
    assert_eq!(map.cells(), WalkableMap::build(&grid, PARAMS).cells());
}

#[test]
fn an_update_recomputes_only_the_cells_a_change_can_flip() {
    let world = floor_world();
    let mut grid = CoarseGrid::build(&world);
    let mut map = WalkableMap::build(&grid, PARAMS);
    // One isolated changed cell: a sphere carved inside brick (5, 5, -1).
    let r = world
        .apply(&EditOp::sphere(
            centre_of_voxel([44, 44, -4]),
            2 * FIXED_ONE,
            Mode::Carve,
        ))
        .unwrap();
    let changes = grid.update(&world, &r);
    assert_eq!(changes.len(), 1);
    let (_, stats) = map.update_with_stats(&grid, &changes);
    assert_eq!(stats.cells_recomputed, u64::from(PARAMS.clearance) + 2);
}

#[test]
fn incremental_updates_equal_a_full_rebuild_on_seeded_sequences() {
    let mut flips = 0;
    for seed in 0..32u64 {
        let world = floor_world();
        let mut grid = CoarseGrid::build(&world);
        let mut map = WalkableMap::build(&grid, PARAMS);
        let mut rng = SplitMix64(seed);
        for _ in 0..8 {
            let c = [
                rng.range(-100, 100),
                rng.range(-100, 100),
                rng.range(-24, 40),
            ];
            let r = rng.range(2, 20) * FIXED_ONE;
            let mode = if rng.range(0, 1) == 0 {
                Mode::Carve
            } else {
                Mode::Fill(rng.range(1, 2) as MaterialId)
            };
            let centre = Fixed3(c.map(|v| v * FIXED_ONE + rng.range(0, 255)));
            let result = world.apply(&EditOp::sphere(centre, r, mode)).unwrap();
            let changes = grid.update(&world, &result);
            flips += map.update(&grid, &changes).len();
            assert_eq!(
                map.cells(),
                WalkableMap::build(&grid, PARAMS).cells(),
                "seed {seed}"
            );
        }
        assert_eq!(grid, CoarseGrid::build(&world), "seed {seed}");
        assert_eq!(map.count(), WalkableMap::build(&grid, PARAMS).count());
    }
    // The sequences do move walkable cells, so the comparison is not vacuous.
    assert!(flips > 100, "{flips} flips");
}
