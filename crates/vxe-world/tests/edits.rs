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

#[test]
fn box_carve_counts_the_centres_in_the_half_open_box() {
    let world = solid3_world();
    let r = world
        .apply(&EditOp::box_(
            Fixed3([0, 0, 0]),
            Fixed3([1024, 1280, 1536]),
            Mode::Carve,
        ))
        .unwrap();
    assert_eq!(r.changed_from, [(3, 120)]);
    assert_eq!(r.aabb, Some((VoxelCoord([0, 0, 0]), VoxelCoord([3, 4, 5]))));
}

#[test]
fn capsule_carve_reports_exact_counts() {
    let world = solid3_world();
    let r = world
        .apply(&EditOp::capsule(
            centre_of_voxel([0, 0, 0]),
            centre_of_voxel([10, 0, 0]),
            3 * FIXED_ONE,
            Mode::Carve,
        ))
        .unwrap();
    assert_eq!(r.changed_from, [(3, 413)]);
    assert_eq!(
        r.aabb,
        Some((VoxelCoord([-3, -3, -3]), VoxelCoord([13, 3, 3])))
    );
    assert_eq!(world.material_at([13, 0, 0]), Some(0));
    assert_eq!(world.material_at([14, 0, 0]), Some(3));
    assert_eq!(world.material_at([5, 3, 0]), Some(0));
    assert_eq!(world.material_at([5, 3, 1]), Some(3));
}

/// `x >= 0` is material 4 (indestructible), `x < 0` is material 3.
fn split_world() -> World {
    World::generate(
        test_extent(),
        &FnSource(|v: VoxelCoord| if v.0[0] >= 0 { 4 } else { 3 }),
        &test_table(),
    )
}

#[test]
fn carve_leaves_indestructible_voxels() {
    let world = split_world();
    let r = world.apply(&sphere10(Mode::Carve)).unwrap();
    assert_eq!(r.changed_from, [(3, 1926)]);
    assert_eq!(world.material_at([0, 0, 0]), Some(4));
    assert_eq!(world.material_at([-1, 0, 0]), Some(0));
    assert_eq!(
        r.aabb,
        Some((VoxelCoord([-10, -9, -9]), VoxelCoord([-1, 9, 9])))
    );
}

#[test]
fn replace_ignores_the_indestructible_flag() {
    let world = split_world();
    let r = world
        .apply(&sphere10(Mode::Replace { from: 4, to: 5 }))
        .unwrap();
    assert_eq!(r.changed_from, [(4, 2243)]);
    assert_eq!(world.material_at([0, 0, 0]), Some(5));
    assert_eq!(world.material_at([-1, 0, 0]), Some(3));
    assert_eq!(world.material_at([11, 0, 0]), Some(4));
}

#[test]
fn fill_writes_only_air() {
    let world = solid3_world();
    world.apply(&sphere10(Mode::Carve)).unwrap();
    // Fixed [-256, 512) holds the centres of voxels -1..=1.
    let cube = EditOp::box_(Fixed3([-256; 3]), Fixed3([512; 3]), Mode::Fill(2));
    let r = world.apply(&cube).unwrap();
    assert_eq!(r.changed_from, [(0, 27)]);
    assert_eq!(world.material_at([1, 1, 1]), Some(2));
    assert_eq!(world.material_at([2, 0, 0]), Some(0));

    let again = world.apply(&cube).unwrap();
    assert!(again.changed_from.is_empty());
    assert!(again.touched.is_empty());

    // A larger fill reaches the rock and the filled cube, and writes only the remaining air.
    let r = world
        .apply(&EditOp::sphere(
            centre_of_voxel([0, 0, 0]),
            12 * FIXED_ONE,
            Mode::Fill(1),
        ))
        .unwrap();
    assert_eq!(r.changed_from, [(0, 4169 - 27)]);
    assert_eq!(world.material_at([0, 0, 0]), Some(2));
    assert_eq!(world.material_at([0, 0, 10]), Some(1));
    assert_eq!(world.material_at([0, 0, 11]), Some(3));
}

#[test]
fn an_op_reaching_a_non_resident_chunk_changes_nothing() {
    let world = solid3_world();
    world.apply(&sphere10(Mode::Carve)).unwrap();
    let before = world_hash(&world);
    let stats = world.stats();

    let op = EditOp::sphere(centre_of_voxel([127, 0, 0]), 10 * FIXED_ONE, Mode::Carve);
    assert_eq!(
        world.apply(&op),
        Err(EditError::NotResident(ChunkCoord([2, -1, -1])))
    );
    assert_eq!(world_hash(&world), before);
    assert_eq!(world.stats(), stats);
    assert_eq!(world.material_at([127, 0, 0]), Some(3));
}

#[test]
fn a_shape_whose_bounds_but_no_centre_reach_a_missing_chunk_succeeds() {
    // The sphere's bounding box reaches x = 128 (chunk 2), but only its pole voxel [128, 0, 0]
    // would lie there; at radius 10 voxels minus one fixed unit that pole falls outside.
    let world = solid3_world();
    let op = EditOp::sphere(
        centre_of_voxel([118, 0, 0]),
        10 * FIXED_ONE - 1,
        Mode::Carve,
    );
    let r = world.apply(&op).unwrap();
    assert_eq!(r.aabb.unwrap().1, VoxelCoord([127, 9, 9]));
}

/// Every shape and mode, with one op that fails on a non-resident chunk.
fn mixed_ops() -> Vec<EditOp> {
    let c = centre_of_voxel;
    vec![
        EditOp::sphere(c([3, -5, 7]), 9 * FIXED_ONE + 40, Mode::Carve),
        EditOp::box_(
            Fixed3([-2000, -900, 300]),
            Fixed3([1500, 2600, 4100]),
            Mode::Fill(2),
        ),
        EditOp::capsule(
            c([-20, -3, 1]),
            c([17, 9, -12]),
            4 * FIXED_ONE + 99,
            Mode::Replace { from: 1, to: 5 },
        ),
        EditOp::capsule(c([30, 30, 30]), c([-31, 20, 2]), 2 * FIXED_ONE, Mode::Carve),
        EditOp::sphere(c([120, 0, 0]), 12 * FIXED_ONE, Mode::Carve),
        EditOp::box_(
            Fixed3([-8000, -8000, -8000]),
            Fixed3([-3000, -2500, -1000]),
            Mode::Replace { from: 4, to: 3 },
        ),
        EditOp::sphere(c([0, 0, 0]), 14 * FIXED_ONE, Mode::Fill(1)),
        EditOp::capsule(
            c([-60, 60, -60]),
            c([60, -60, 60]),
            3 * FIXED_ONE + 128,
            Mode::Carve,
        ),
    ]
}

#[test]
fn results_do_not_depend_on_generation_thread_count() {
    let source = FnSource(|v: VoxelCoord| {
        let [x, y, z] = v.0;
        (x.div_euclid(5) + y.div_euclid(7) + z.div_euclid(3)).rem_euclid(6) as MaterialId
    });
    let run = |threads| {
        let world = World::generate_with_threads(test_extent(), &source, &test_table(), threads);
        let results: Vec<_> = mixed_ops().iter().map(|op| world.apply(op)).collect();
        (results, world_hash(&world))
    };
    let (one, h1) = run(1);
    let (sixteen, h16) = run(16);
    assert_eq!(one, sixteen);
    assert_eq!(h1, h16);
    // The list exercises every mode and the failure path.
    assert_eq!(one[4], Err(EditError::NotResident(ChunkCoord([2, -1, -1]))));
    assert!(
        one.iter()
            .filter(|r| matches!(r, Ok(r) if !r.changed_from.is_empty()))
            .count()
            >= 6
    );
}
