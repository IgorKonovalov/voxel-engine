//! The coarse grid: build, update from an edit result, ties, and streaming hooks.

mod support;

use support::*;
use vxe_world::*;

fn all_cells(g: &CoarseGrid) -> Vec<(ChunkCoord, usize, CoarseCell)> {
    let mut v = Vec::new();
    for c in g.chunks() {
        for (slot, cell) in g.chunk(c).unwrap().iter().enumerate() {
            v.push((c, slot, *cell));
        }
    }
    v
}

fn radius10_carve() -> EditOp {
    EditOp::sphere(centre_of_voxel([0, 0, 0]), 10 * FIXED_ONE, Mode::Carve)
}

#[test]
fn the_solid_world_is_full_cells_of_its_material_read_without_voxels() {
    let (g, stats) = CoarseGrid::build_with_stats(&solid3_world());
    let cells = all_cells(&g);
    assert_eq!(cells.len(), 32_768);
    assert!(cells.iter().all(|(_, _, c)| *c
        == CoarseCell {
            solid: 512,
            dominant: 3
        }));
    assert_eq!(stats.voxel_arrays_read, 0);
    assert_eq!(g.bytes(), 64 * 2048);
    assert_eq!(size_of::<CoarseCell>(), 4);
}

#[test]
fn the_carved_world_has_32_partial_cells_missing_4169_voxels() {
    let (g, stats) = CoarseGrid::build_with_stats(&carved_world());
    let cells = all_cells(&g);
    let partial: Vec<_> = cells.iter().filter(|(_, _, c)| c.solid < 512).collect();
    assert_eq!(partial.len(), 32);
    let missing: u64 = cells.iter().map(|(_, _, c)| u64::from(512 - c.solid)).sum();
    assert_eq!(missing, 4169);
    assert!(cells.iter().all(|(_, _, c)| c.dominant == 3));
    assert_eq!(
        g.cell(ChunkCoord([0, 0, 0]), 0),
        Some(CoarseCell {
            solid: 32,
            dominant: 3
        })
    );
    assert_eq!(stats.voxel_arrays_read, 32);
}

#[test]
fn an_update_reports_the_changed_cells_sorted() {
    let world = solid3_world();
    let mut g = CoarseGrid::build(&world);
    let r = world.apply(&radius10_carve()).unwrap();
    let (changes, stats) = g.update_with_stats(&world, &r);
    assert_eq!(changes.len(), 32);
    // The aabb spans 4³ bricks; the 32 `Full` ones are the only voxel arrays read.
    assert_eq!(stats.voxel_arrays_read, 32);
    let mut sorted = changes.clone();
    sorted.sort_by_key(|c| (c.chunk, c.slot));
    assert_eq!(changes, sorted);
    for c in &changes {
        assert_eq!(
            c.before,
            CoarseCell {
                solid: 512,
                dominant: 3
            }
        );
        assert!(c.after.solid < 512 && c.after.dominant == 3);
        assert_eq!(g.cell(c.chunk, usize::from(c.slot)), Some(c.after));
    }
    let missing: u32 = changes.iter().map(|c| u32::from(512 - c.after.solid)).sum();
    assert_eq!(missing, 4169);
    assert_eq!(g, CoarseGrid::build(&world));

    let again = world.apply(&radius10_carve()).unwrap();
    assert!(g.update(&world, &again).is_empty());

    let fill = world
        .apply(&EditOp::box_(
            Fixed3([0, 0, 0]),
            Fixed3([2048, 2048, 2048]),
            Mode::Fill(2),
        ))
        .unwrap();
    assert_eq!(fill.changed_from, [(AIR, 480)]);
    let changes = g.update(&world, &fill);
    assert_eq!(changes.len(), 1);
    assert_eq!(
        (changes[0].chunk, changes[0].slot),
        (ChunkCoord([0, 0, 0]), 0)
    );
    assert_eq!(
        changes[0].before,
        CoarseCell {
            solid: 32,
            dominant: 3
        }
    );
    assert_eq!(
        changes[0].after,
        CoarseCell {
            solid: 512,
            dominant: 2
        }
    );
    assert_eq!(g, CoarseGrid::build(&world));
}

#[test]
fn ties_go_to_the_lower_id_and_air_is_not_counted() {
    // Brick [0, 8)³: 256 voxels of 5 (z < 4) then 256 of 2. Brick [8, 16) x [0, 8)²: 255
    // voxels of 5, the rest air.
    let source = FnSource(|v: VoxelCoord| {
        let [x, y, z] = v.0;
        if (0..8).contains(&x) && (0..8).contains(&y) && (0..8).contains(&z) {
            if z < 4 { 5 } else { 2 }
        } else if (8..16).contains(&x) && (0..8).contains(&y) && (0..8).contains(&z) {
            let i = (x - 8) + 8 * y + 64 * z;
            if i < 255 { 5 } else { AIR }
        } else {
            AIR
        }
    });
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [1, 1, 1]),
        &source,
        &test_table(),
    );
    let g = CoarseGrid::build(&world);
    let c = ChunkCoord([0, 0, 0]);
    assert_eq!(
        g.cell(c, 0),
        Some(CoarseCell {
            solid: 512,
            dominant: 2
        })
    );
    assert_eq!(
        g.cell(c, 1),
        Some(CoarseCell {
            solid: 255,
            dominant: 5
        })
    );
    assert_eq!(
        g.cell(c, 2),
        Some(CoarseCell {
            solid: 0,
            dominant: AIR
        })
    );
}

#[test]
fn loading_every_chunk_equals_building_and_eviction_drops_cells() {
    let world = carved_world();
    let built = CoarseGrid::build(&world);
    let mut g = CoarseGrid::new();
    for c in test_extent().coords().into_iter().rev() {
        g.chunk_loaded(&world, c);
    }
    assert_eq!(g, built);
    assert_eq!(g.len(), 64);
    let c = ChunkCoord([0, 0, 0]);
    g.chunk_evicted(c);
    assert_eq!(g.len(), 63);
    assert!((0..CHUNK_BRICKS).all(|s| g.cell(c, s).is_none()));
    assert_eq!(g.bytes(), 63 * 2048);
    // A chunk that is not resident adds nothing.
    g.chunk_loaded(&world, ChunkCoord([9, 9, 9]));
    assert_eq!(g.len(), 63);
    g.chunk_loaded(&world, c);
    assert_eq!(g, built);
}

/// The cell by today's rule from the decoded voxels: the non-air count, and the most frequent
/// non-air material with the lower id on a tie.
fn decoded_cell(b: &Brick) -> CoarseCell {
    let mut v = [AIR; BRICK_VOXELS];
    b.decode(&mut v);
    let mut counts = std::collections::BTreeMap::<MaterialId, u16>::new();
    for m in v {
        if m != AIR {
            *counts.entry(m).or_default() += 1;
        }
    }
    let mut cell = CoarseCell::default();
    let mut best = 0;
    for (m, n) in counts {
        cell.solid += n;
        if n > best {
            best = n;
            cell.dominant = m;
        }
    }
    cell
}

/// `CoarseCell::of` from popcounts equals the decoded rule on the census chunk's six kinds of
/// brick and on 256 seeded bricks of 1 to 20 materials, ties included.
#[test]
fn the_cell_from_popcounts_equals_the_decoded_rule() {
    let half = |a: MaterialId, b: MaterialId| Brick::from_fn(|l| if l[2] < 4 { a } else { b });
    let six = [
        Brick::Uniform(1),
        half(AIR, 1),
        half(1, 2),
        Brick::from_fn(|l| 4 + (index_in_brick(l) % 17) as MaterialId),
        half(1, 3),
        half(AIR, 3),
    ];
    for (i, b) in six.iter().enumerate() {
        assert_eq!(CoarseCell::of(b), decoded_cell(b), "slot {i}");
    }
    let mut rng = SplitMix64(0x0021_0007);
    for i in 0..256 {
        let materials = rng.range(1, 20);
        let solid = rng.range(0, 8);
        let b = Brick::from_fn(|_| {
            if rng.range(0, 7) < solid {
                1 + rng.range(0, materials - 1) as MaterialId
            } else {
                AIR
            }
        });
        assert_eq!(CoarseCell::of(&b), decoded_cell(&b), "brick {i}");
    }
}
