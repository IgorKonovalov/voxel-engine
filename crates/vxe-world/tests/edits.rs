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

#[test]
fn tool_along_ray_carves_a_capsule_to_the_hit_face() {
    let world = carved_world();
    let origin = centre_of_voxel([0, 0, 0]);
    let op = EditOp::tool_along_ray(
        &world,
        origin,
        [0, 0, 256],
        100 * 256,
        2 * FIXED_ONE,
        Mode::Carve,
    );
    assert_eq!(
        op.shape,
        Shape::Capsule {
            a: origin,
            b: Fixed3([128, 128, 2816]),
            r: 512
        }
    );
    let r = world.apply(&op).unwrap();
    assert_eq!(r.changed_from, [(3, 26)]);
    assert_eq!(r.aabb.unwrap().0.0[2], 10);
    assert_eq!(r.aabb.unwrap().1.0[2], 12);
}

#[test]
fn tool_along_ray_without_a_hit_ends_at_max_t() {
    let world = carved_world();
    let origin = centre_of_voxel([0, 0, 0]);
    let op = EditOp::tool_along_ray(
        &world,
        origin,
        [0, 0, 256],
        5 * 256,
        2 * FIXED_ONE,
        Mode::Carve,
    );
    assert_eq!(
        op.shape,
        Shape::Capsule {
            a: origin,
            b: Fixed3([128, 128, 128 + 5 * 256]),
            r: 512
        }
    );
    assert_eq!(world.apply(&op).unwrap().changed_from, []);
}

/// One op of the oracle run: shape and mode cycle with `i`, sizes and positions come from
/// `rng`. Every eighth op from 7 sits within 8 voxels of the world's +x, -y or +z face with a
/// radius of at least 10 voxels, so it reaches a non-resident chunk.
fn seeded_op(i: usize, rng: &mut SplitMix64) -> EditOp {
    let near_edge = i % 40 == 7;
    let mut c = [0; 3].map(|_| rng.range(-100, 100) * FIXED_ONE + rng.range(-128, 127));
    if near_edge {
        let axis = (i / 40) % 3;
        let side = if axis == 1 { -1 } else { 1 };
        c[axis] = side * rng.range(120, 127) * FIXED_ONE + rng.range(-128, 127);
    }
    let size = |rng: &mut SplitMix64| {
        if near_edge {
            rng.range(10 * FIXED_ONE, 12 * FIXED_ONE)
        } else {
            rng.range(FIXED_ONE, 12 * FIXED_ONE)
        }
    };
    let mode = match (i / 3) % 3 {
        0 => Mode::Carve,
        1 => Mode::Fill(rng.range(1, 5) as MaterialId),
        _ => Mode::Replace {
            from: rng.range(0, 5) as MaterialId,
            to: rng.range(0, 5) as MaterialId,
        },
    };
    match i % 3 {
        0 => EditOp::sphere(Fixed3(c), size(rng), mode),
        1 => {
            let lo = [0; 3].map(|_| size(rng));
            let hi = [0; 3].map(|_| size(rng));
            EditOp::box_(
                Fixed3([0, 1, 2].map(|k| c[k] - lo[k])),
                Fixed3([0, 1, 2].map(|k| c[k] + hi[k])),
                mode,
            )
        }
        _ => {
            let d = [0; 3].map(|_| rng.range(-12 * FIXED_ONE, 12 * FIXED_ONE));
            let r = size(rng);
            EditOp::capsule(Fixed3(c), Fixed3([0, 1, 2].map(|k| c[k] + d[k])), r, mode)
        }
    }
}

/// Runs the oracle's 200 seeded ops (seed `0x0003_0001`, [`seeded_op`]) in sequence on
/// `world`: each result or error, and every voxel in the op's bounds afterwards, equals what
/// the brute-force reference says. Returns how many ops changed a voxel and how many failed on
/// a non-resident chunk.
/// Every brick of the `touched` chunks holds the bytes `Brick::from_voxels` gives its
/// decoded voxels: the canonical form, so equal voxels are equal bricks.
fn assert_canonical(world: &World, touched: &[ChunkCoord], op: usize) {
    for &c in touched {
        let chunk = world.store().get(c).expect("touched chunks are resident");
        let chunk = chunk.read().unwrap();
        for (slot, b) in chunk.bricks().iter().enumerate() {
            let mut v = Box::new([AIR; BRICK_VOXELS]);
            b.decode(&mut v);
            assert_eq!(*b, Brick::from_voxels(v), "op {op}: {c:?} slot {slot}");
        }
    }
}

fn run_seeded_ops_against_the_reference(world: &World) -> (usize, usize) {
    let mut rng = SplitMix64(0x0003_0001);
    let mut changed = 0;
    let mut failed = 0;
    for i in 0..200 {
        let op = seeded_op(i, &mut rng);
        let expected = reference_apply(world, &op);
        assert_eq!(world.apply(&op), expected.result, "op {i}: {op:?}");
        if let Some((lo, hi)) = expected.bounds {
            assert!(
                materials_in(world, lo, hi) == expected.after,
                "op {i}: voxels differ from the reference after {op:?}"
            );
        }
        if let Ok(r) = &expected.result {
            assert_canonical(world, &r.touched, i);
        }
        match &expected.result {
            Ok(r) if !r.changed_from.is_empty() => changed += 1,
            Ok(_) => {}
            Err(_) => failed += 1,
        }
    }
    (changed, failed)
}

/// 200 seeded ops of every shape and mode on a world where every brick is `Full` and all six
/// materials occur: `World::apply` returns exactly what the brute-force reference says, and
/// leaves every voxel in the op's bounds as the reference says.
#[test]
fn apply_matches_the_reference_over_seeded_ops() {
    let world = World::generate(
        test_extent(),
        &FnSource(|v: VoxelCoord| (hash::hash3(0x0003_0001, v.0) % 6) as MaterialId),
        &test_table(),
    );
    let (changed, failed) = run_seeded_ops_against_the_reference(&world);
    println!("{changed} of 200 ops changed voxels; {failed} failed on a non-resident chunk");
    assert!(changed >= 150, "{changed} ops changed voxels");
    assert!(failed >= 5, "{failed} ops reached a non-resident chunk");
}

/// The same 200 seeded ops on two worlds of `Uniform` bricks only, the edit bench's cubes
/// world and the solid material-3 world, so the ops expand, write and collapse uniform
/// bricks: every result, error and voxel equals the reference.
#[test]
fn apply_matches_the_reference_on_uniform_worlds() {
    for (name, world) in [("cubes", cubes_world()), ("solid", solid3_world())] {
        let stats = world.stats();
        assert_eq!(
            stats.full_bricks, 0,
            "{name}: a Full brick before the first op"
        );
        assert_eq!(stats.uniform_bricks, 64 * CHUNK_BRICKS, "{name}");
        let (changed, failed) = run_seeded_ops_against_the_reference(&world);
        println!(
            "{name}: {changed} of 200 ops changed voxels; {failed} failed on a non-resident chunk"
        );
        assert!(
            failed >= 5,
            "{name}: {failed} ops reached a non-resident chunk"
        );
        assert!(
            world.stats().full_bricks > 0,
            "{name}: no brick was expanded"
        );
    }
}

/// A carve over bricks that hold only indestructible materials, `Full` ones (4 and a second
/// indestructible row 6 in a checkerboard) and `Uniform` ones (all 4), writes nothing: no
/// result, no dirty brick or chunk, no edited flag, no voxel or representation change.
#[test]
fn carving_only_indestructible_bricks_writes_nothing() {
    let mut table = test_table();
    let mut m6 = Material::named("m6");
    m6.flags = MaterialFlags::INDESTRUCTIBLE;
    assert_eq!(table.push(m6), 6);
    let world = World::generate(
        test_extent(),
        &FnSource(|v: VoxelCoord| {
            let [x, y, z] = v.0;
            match (x >= 0, (x + y + z).rem_euclid(2)) {
                (false, 0) => 4,
                (false, _) => 6,
                (true, _) => 4,
            }
        }),
        &table,
    );
    let _ = world.store().take_dirty();
    for c in world.store().coords() {
        world.store().get(c).unwrap().write().unwrap().clear_dirty();
    }
    let before = world_hash(&world);
    let stats = world.stats();
    assert!(stats.full_bricks > 0 && stats.uniform_bricks > 0);

    let r = world.apply(&sphere10(Mode::Carve)).unwrap();
    assert_eq!(r, EditResult::default());
    assert_eq!(world.store().dirty_len(), 0);
    for c in world.store().coords() {
        let chunk = world.store().get(c).unwrap();
        let chunk = chunk.read().unwrap();
        assert!(!chunk.any_dirty(), "{c:?}");
        assert!(!chunk.is_edited(), "{c:?}");
    }
    assert_eq!(world_hash(&world), before);
    assert_eq!(world.stats(), stats);
}

/// Voxels whose ids have no table row (6, 7 and 8 against a six-row table) are carved and
/// counted like any other: 60 seeded carves and replaces of every shape match the reference,
/// and the results count ids beyond the table.
#[test]
fn ids_beyond_the_table_are_counted_like_the_reference() {
    let world = World::generate(
        test_extent(),
        &FnSource(|v: VoxelCoord| (hash::hash3(0x0003_0004, v.0) % 9) as MaterialId),
        &test_table(),
    );
    assert_eq!(world.materials().len(), 6);
    let mut rng = SplitMix64(0x0003_0004);
    let mut beyond = 0;
    for i in 0..60 {
        let c = [0; 3].map(|_| rng.range(-60, 60) * FIXED_ONE + rng.range(-128, 127));
        let r = rng.range(FIXED_ONE, 10 * FIXED_ONE);
        let mode = if i % 2 == 0 {
            Mode::Carve
        } else {
            Mode::Replace {
                from: rng.range(6, 8) as MaterialId,
                to: rng.range(0, 5) as MaterialId,
            }
        };
        let op = match i % 3 {
            0 => EditOp::sphere(Fixed3(c), r, mode),
            1 => EditOp::box_(Fixed3(c.map(|v| v - r)), Fixed3(c.map(|v| v + r)), mode),
            _ => EditOp::capsule(Fixed3(c), Fixed3(c.map(|v| v + 3 * r)), r / 2 + 1, mode),
        };
        let expected = reference_apply(&world, &op);
        let got = world.apply(&op);
        assert_eq!(got, expected.result, "op {i}: {op:?}");
        if let Some((lo, hi)) = expected.bounds {
            assert!(
                materials_in(&world, lo, hi) == expected.after,
                "op {i}: voxels differ from the reference after {op:?}"
            );
        }
        let r = got.unwrap();
        beyond += r.changed_from.iter().filter(|(m, _)| *m >= 6).count();
    }
    assert!(beyond >= 30, "{beyond} counts of ids beyond the table");
}

/// 2000 seeded ops on the cubes source over chunks `[-1, 1)^3`, mixing carve, fill of
/// materials 1 to 3 and replace between them, so palette entries are inserted and removed at
/// every position. After every op the result equals the reference's (`changed_from`, `aabb`,
/// `touched`), the voxels in its bounds equal the reference's, and every brick of the touched
/// chunks is canonical.
#[test]
fn the_planar_kernel_matches_the_reference_over_2000_mixed_ops() {
    let world = World::generate(
        ChunkExtent::new([-1, -1, -1], [1, 1, 1]),
        &CubesSource,
        &test_table(),
    );
    let mut rng = SplitMix64(0x0021_0004);
    let mut changed = 0;
    for i in 0..2000 {
        let c = [0; 3].map(|_| rng.range(-60, 59) * FIXED_ONE + rng.range(-128, 127));
        let r = rng.range(FIXED_ONE, 7 * FIXED_ONE);
        let m = |rng: &mut SplitMix64| rng.range(1, 3) as MaterialId;
        let mode = match rng.range(0, 3) {
            0 => Mode::Carve,
            1 => Mode::Fill(m(&mut rng)),
            _ => Mode::Replace {
                from: m(&mut rng),
                to: m(&mut rng),
            },
        };
        let op = if rng.range(0, 3) == 0 {
            let h = [0; 3].map(|_| rng.range(0, 6 * FIXED_ONE));
            EditOp::box_(
                Fixed3([0, 1, 2].map(|k| c[k] - h[k])),
                Fixed3([0, 1, 2].map(|k| c[k] + h[k])),
                mode,
            )
        } else {
            EditOp::sphere(Fixed3(c), r, mode)
        };
        let expected = reference_apply(&world, &op);
        assert_eq!(world.apply(&op), expected.result, "op {i}: {op:?}");
        if let Some((lo, hi)) = expected.bounds {
            assert!(
                materials_in(&world, lo, hi) == expected.after,
                "op {i}: voxels differ from the reference after {op:?}"
            );
        }
        if let Ok(r) = &expected.result {
            assert_canonical(&world, &r.touched, i);
            changed += usize::from(!r.changed_from.is_empty());
        }
    }
    let st = world.packed_stats();
    println!("{changed} of 2000 ops changed voxels; {st:?}");
    assert!(changed >= 1000, "{changed} ops changed voxels");
    assert!(
        st.class_changes > 0 && st.entries_removed > 0 && st.renames > 0,
        "{st:?}"
    );
}

/// The brick at slot 0 of a one-chunk world, and its palette.
fn brick0(world: &World) -> (Option<vxe_world::packed::PackedClass>, Vec<MaterialId>) {
    let chunk = world.store().get(ChunkCoord([0, 0, 0])).unwrap();
    let chunk = chunk.read().unwrap();
    let b = chunk.brick(0);
    let palette = match b {
        Brick::Full(p) => p.palette(),
        Brick::Uniform(m) => vec![*m],
    };
    (b.class(), palette)
}

/// The voxel box `lo..=hi` as an op.
fn voxel_box(lo: [i32; 3], hi: [i32; 3], mode: Mode) -> EditOp {
    EditOp::box_(
        Fixed3(lo.map(|v| i64::from(v) * FIXED_ONE)),
        Fixed3(hi.map(|v| i64::from(v) * FIXED_ONE + FIXED_ONE - 1)),
        mode,
    )
}

/// A fill inserts a palette entry and grows the class, a replace whose `from` voxels all lie
/// in the shape renames the entry with no plane written, and a carve of every voxel of an
/// entry removes it and shrinks the class.
#[test]
fn entries_are_inserted_renamed_and_removed() {
    let mut table = MaterialTable::new();
    for i in 1..=7u16 {
        table.push(Material::named(format!("m{i}")));
    }
    // Brick 0: layers z 0..4 of material 3, the rest air.
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [1, 1, 1]),
        &FnSource(|v: VoxelCoord| {
            if v.0.iter().all(|&c| c < 8) && v.0[2] < 4 {
                3
            } else {
                AIR
            }
        }),
        &table,
    );
    assert_eq!(
        brick0(&world),
        (Some(vxe_world::packed::PackedClass::Mask), vec![3])
    );

    // Fill 7 into layer 4: 64 air voxels.
    let r = world
        .apply(&voxel_box([0, 0, 4], [7, 7, 4], Mode::Fill(7)))
        .unwrap();
    assert_eq!(r.changed_from, [(AIR, 64)]);
    assert_eq!(
        brick0(&world),
        (Some(vxe_world::packed::PackedClass::Plane1), vec![3, 7])
    );
    let s0 = world.packed_stats();
    assert_eq!(s0.class_changes, 1);

    // Replace 3 by 2 over the whole brick: a rename, the palette [2, 7].
    let r = world
        .apply(&voxel_box(
            [0, 0, 0],
            [7, 7, 7],
            Mode::Replace { from: 3, to: 2 },
        ))
        .unwrap();
    assert_eq!(r.changed_from, [(3, 256)]);
    assert_eq!(
        brick0(&world),
        (Some(vxe_world::packed::PackedClass::Plane1), vec![2, 7])
    );
    let s1 = world.packed_stats();
    assert_eq!(s1.renames - s0.renames, 1);
    assert_eq!(s1.class_changes, s0.class_changes);

    // Carve every 7: the entry goes and the brick is mask-only again.
    let r = world
        .apply(&voxel_box([0, 0, 4], [7, 7, 4], Mode::Carve))
        .unwrap();
    assert_eq!(r.changed_from, [(7, 64)]);
    assert_eq!(
        brick0(&world),
        (Some(vxe_world::packed::PackedClass::Mask), vec![2])
    );
    let s2 = world.packed_stats();
    assert_eq!(s2.entries_removed - s1.entries_removed, 1);
    assert_eq!(s2.class_changes - s1.class_changes, 1);
}
