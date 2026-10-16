mod support;

use support::*;
use vxe_world::*;

fn sphere10(mode: Mode) -> EditOp {
    EditOp::sphere(centre_of_voxel([0, 0, 0]), 10 * FIXED_ONE, mode)
}

#[test]
fn generated_solid_world_is_all_uniform() {
    let world = solid3_world();
    assert_eq!(
        world.stats(),
        WorldStats {
            chunks: 64,
            uniform_bricks: 32_768,
            full_bricks: 0
        }
    );
    for c in world.store().coords() {
        let chunk = world.store().get(c).unwrap();
        let chunk = chunk.read().unwrap();
        assert!(chunk.bricks().iter().all(|b| *b == Brick::Uniform(3)));
    }
}

#[test]
fn sphere_carve_reports_exact_counts_and_touched_chunks() {
    let world = solid3_world();
    let r = world.apply(&sphere10(Mode::Carve)).unwrap();
    assert_eq!(r.changed_from, [(3, 4169)]);

    let mut expected = Vec::new();
    for x in [-1, 0] {
        for y in [-1, 0] {
            for z in [-1, 0] {
                expected.push(ChunkCoord([x, y, z]));
            }
        }
    }
    expected.sort();
    assert_eq!(r.touched, expected);
    assert_eq!(
        r.aabb,
        Some((VoxelCoord([-10, -10, -10]), VoxelCoord([10, 10, 10])))
    );
}

#[test]
fn sphere_carve_leaves_the_boundary_and_expands_only_entered_bricks() {
    let world = solid3_world();
    world.apply(&sphere10(Mode::Carve)).unwrap();
    assert_eq!(world.material_at([0, 0, 10]), Some(0));
    assert_eq!(world.material_at([5, 5, 7]), Some(0));
    assert_eq!(world.material_at([0, 0, 11]), Some(3));
    assert_eq!(world.material_at([6, 6, 6]), Some(3));
    assert_eq!(world.stats().full_bricks, 32);
    assert_eq!(world.stats().uniform_bricks, 32_768 - 32);
}

#[test]
fn repeating_a_carve_changes_nothing() {
    let world = solid3_world();
    world.apply(&sphere10(Mode::Carve)).unwrap();
    let again = world.apply(&sphere10(Mode::Carve)).unwrap();
    assert!(again.changed_from.is_empty());
    assert!(again.touched.is_empty());
    assert_eq!(again.aabb, None);
}

#[test]
fn a_brick_carved_to_air_collapses() {
    // Fixed [0, 2048)^3 holds exactly the centres of voxels [0, 8)^3.
    let carve_brick = |world: &World| {
        let r = world
            .apply(&EditOp::box_(
                Fixed3([0; 3]),
                Fixed3([8 * FIXED_ONE; 3]),
                Mode::Carve,
            ))
            .unwrap();
        r.changed_from.iter().map(|&(_, c)| c).sum::<u32>()
    };

    // Fresh solid world: the brick expands, empties, and collapses back to Uniform(0).
    let world = solid3_world();
    assert_eq!(carve_brick(&world), 512);
    assert_eq!(world.stats().full_bricks, 0);
    let chunk = world.store().get(ChunkCoord([0, 0, 0])).unwrap();
    assert_eq!(*chunk.read().unwrap().brick(0), Brick::Uniform(0));

    // After the sphere the brick is Full; emptying it collapses it and the count drops by one.
    let world = solid3_world();
    world.apply(&sphere10(Mode::Carve)).unwrap();
    assert_eq!(world.stats().full_bricks, 32);
    // The sphere emptied the 480 voxels of the brick with x^2 + y^2 + z^2 <= 100; the box
    // carves the other 32.
    assert_eq!(carve_brick(&world), 32);
    assert_eq!(world.stats().full_bricks, 31);
    let chunk = world.store().get(ChunkCoord([0, 0, 0])).unwrap();
    assert_eq!(*chunk.read().unwrap().brick(0), Brick::Uniform(0));
}
