mod support;

use std::collections::HashSet;
use std::sync::Arc;

use support::*;
use vxe_world::*;

/// The tripod: material 5 at (0,0,0), (1,0,0), (2,0,0), (0,1,0), (0,0,1), (0,0,2), (0,0,3),
/// masked to those seven voxels, size (3, 2, 4). Arms of lengths 2, 1 and 3 along x, y and z
/// give it no rotational symmetry.
fn tripod() -> Prefab {
    let size = [3u32, 2, 4];
    let solid = [
        [0, 0, 0],
        [1, 0, 0],
        [2, 0, 0],
        [0, 1, 0],
        [0, 0, 1],
        [0, 0, 2],
        [0, 0, 3],
    ];
    let n = 3 * 2 * 4;
    let mut voxels = vec![AIR; n];
    let mut mask = vec![0u64; 1];
    for p in solid {
        let i = (p[0] + 3 * (p[1] + 2 * p[2])) as usize;
        voxels[i] = 5;
        mask[0] |= 1 << i;
    }
    Prefab::new(size, voxels, Some(mask))
}

/// Every written voxel of `p` as (local, material), x fastest.
fn written(p: &Prefab) -> Vec<([i32; 3], MaterialId)> {
    let s = p.size().map(|v| v as i32);
    let mut out = Vec::new();
    for z in 0..s[2] {
        for y in 0..s[1] {
            for x in 0..s[0] {
                if let Some(m) = p.get([x, y, z]) {
                    out.push(([x, y, z], m));
                }
            }
        }
    }
    out
}

#[test]
fn prefab_get_follows_the_mask_and_id_follows_the_contents() {
    let t = tripod();
    assert_eq!(t.size(), [3, 2, 4]);
    assert_eq!(t.get([0, 0, 0]), Some(5));
    assert_eq!(t.get([0, 0, 3]), Some(5));
    assert_eq!(t.get([1, 1, 0]), None, "mask clear");
    assert_eq!(t.get([3, 0, 0]), None, "outside the box");
    assert_eq!(t.get([-1, 0, 0]), None, "outside the box");
    assert_eq!(t.written_count(), 7);
    assert_eq!(tripod().id(), t.id());

    // A `None` mask writes every voxel and equals the explicit all-ones mask.
    let all = Prefab::new([2, 2, 2], vec![7; 8], None);
    assert_eq!(all.get([1, 1, 1]), Some(7));
    assert_eq!(all.written_count(), 8);
    assert_eq!(
        all.id(),
        Prefab::new([2, 2, 2], vec![7; 8], Some(vec![0xff])).id()
    );

    // Contents that differ in one voxel, or in one mask bit, have different ids.
    let mut v = vec![7; 8];
    v[3] = 6;
    assert_ne!(Prefab::new([2, 2, 2], v, None).id(), all.id());
    assert_ne!(
        Prefab::new([2, 2, 2], vec![7; 8], Some(vec![0x7f])).id(),
        all.id()
    );
}

#[test]
fn rot_has_24_distinct_proper_rotations_closed_under_composition() {
    let all: Vec<Rot> = Rot::all().collect();
    assert_eq!(all.len(), 24);
    assert_eq!(all.iter().copied().collect::<HashSet<_>>().len(), 24);
    assert_eq!(all[0], Rot::IDENTITY);
    let set: HashSet<Rot> = all.iter().copied().collect();
    for &r in &all {
        assert_eq!(r.determinant(), 1, "{r:?}");
        assert_eq!(r.then(r.inverse()), Rot::IDENTITY, "{r:?}");
        assert_eq!(r.inverse().then(r), Rot::IDENTITY, "{r:?}");
        for &o in &all {
            let p = r.then(o);
            assert!(set.contains(&p), "{r:?} then {o:?}");
            let v = [1, 20, 300];
            assert_eq!(p.apply(v), o.apply(r.apply(v)));
        }
    }
    // Same order on every call.
    assert_eq!(Rot::all().collect::<Vec<_>>(), all);

    assert_eq!(Rot::Z90.apply([1, 2, 3]), [-2, 1, 3]);
    assert_eq!(Rot::IDENTITY.apply([1, 2, 3]), [1, 2, 3]);
    assert_eq!(Rot::Z90.then(Rot::Z90).apply([1, 2, 3]), [-1, -2, 3]);
    assert_eq!(Rot::from_rows([1, 0, 2], [false; 3]), None, "a reflection");
    assert_eq!(
        Rot::from_rows([0, 0, 2], [false; 3]),
        None,
        "not a permutation"
    );
    assert_eq!(
        Rot::from_rows([1, 0, 2], [true, false, false]),
        Some(Rot::Z90)
    );
}

#[test]
fn tripod_rotations_are_all_distinct_and_z90_lands_where_expected() {
    let t = tripod();
    let ids: HashSet<u64> = Rot::all().map(|r| t.rotated(r).id()).collect();
    assert_eq!(ids.len(), 24);

    let z = t.rotated(Rot::Z90);
    assert_eq!(z.size(), [2, 3, 4]);
    assert_eq!(
        written(&z),
        vec![
            ([0, 0, 0], 5),
            ([1, 0, 0], 5),
            ([1, 1, 0], 5),
            ([1, 2, 0], 5),
            ([1, 0, 1], 5),
            ([1, 0, 2], 5),
            ([1, 0, 3], 5),
        ]
    );
    assert_eq!(t.rotated(Rot::IDENTITY), t);
    for r in Rot::all() {
        assert_eq!(t.rotated(r).rotated(r.inverse()), t, "{r:?}");
    }
}

#[test]
fn vault_is_a_shell_around_air() {
    let v = Prefab::vault(16, 4);
    assert_eq!(v.size(), [16, 16, 16]);
    assert_eq!(v.written_count(), 4096);
    let w = written(&v);
    assert_eq!(w.iter().filter(|(_, m)| *m == 4).count(), 1352);
    assert_eq!(w.iter().filter(|(_, m)| *m == AIR).count(), 2744);
    assert_eq!(v.get([0, 5, 5]), Some(4));
    assert_eq!(v.get([15, 15, 15]), Some(4));
    assert_eq!(v.get([1, 1, 1]), Some(AIR));
}

#[test]
fn vault_stamp_into_solid_rock_reports_exact_counts() {
    let world = solid3_world();
    let vault = Prefab::vault(16, 4);
    let r = world.stamp(&vault, [8, 8, 8], Rot::IDENTITY).unwrap();
    assert_eq!(r.changed_from, [(3, 4096)]);
    assert_eq!(r.touched, [ChunkCoord([0, 0, 0])]);
    assert_eq!(
        r.aabb,
        Some((VoxelCoord([8, 8, 8]), VoxelCoord([23, 23, 23])))
    );

    // The vault's own 16³ box (inclusive bounds 8..=23).
    assert_eq!(
        world.count_in_box([8, 8, 8], [23, 23, 23]),
        [(0, 2744), (4, 1352)]
    );
    // The box with inclusive upper corner 24 also holds the 17³ - 16³ = 817 rock voxels of
    // the layer just outside the vault.
    assert_eq!(
        world.count_in_box([8, 8, 8], [24, 24, 24]),
        [(0, 2744), (3, 817), (4, 1352)]
    );
    assert_eq!(world.material_at([8, 8, 8]), Some(4));
    assert_eq!(world.material_at([15, 15, 15]), Some(0));
    assert_eq!(world.material_at([23, 23, 23]), Some(4));
    assert_eq!(world.material_at([24, 8, 8]), Some(3));

    // Stamping the same prefab again changes nothing.
    let again = world.stamp(&vault, [8, 8, 8], Rot::IDENTITY).unwrap();
    assert_eq!(again.changed_from, []);
    assert_eq!(again.touched, Vec::<ChunkCoord>::new());
    assert_eq!(again.aabb, None);
}

#[test]
fn stamp_writes_over_indestructible_and_collapses_uniform_bricks() {
    let world = solid3_world();
    world
        .stamp(&Prefab::vault(16, 4), [8, 8, 8], Rot::IDENTITY)
        .unwrap();
    // A solid 16³ block of rock over the vault restores the solid world, shell included.
    let rock = Prefab::new([16; 3], vec![3; 4096], None);
    let r = world.stamp(&rock, [8, 8, 8], Rot::IDENTITY).unwrap();
    assert_eq!(r.changed_from, [(0, 2744), (4, 1352)]);
    assert_eq!(world.voxel_hash(), solid3_world().voxel_hash());
    assert_eq!(
        world.stats(),
        solid3_world().stats(),
        "bricks collapsed back"
    );
}

#[test]
fn rotated_tripod_stamp_writes_only_the_masked_voxels() {
    let world = solid3_world();
    let r = world.stamp(&tripod(), [100, 100, 100], Rot::Z90).unwrap();
    assert_eq!(r.changed_from, [(3, 7)]);
    assert_eq!(world.material_at([101, 102, 100]), Some(5));
    assert_eq!(world.material_at([100, 100, 100]), Some(5));
    assert_eq!(world.material_at([101, 100, 103]), Some(5));
    assert_eq!(
        world.material_at([100, 101, 100]),
        Some(3),
        "unmasked, untouched"
    );
    assert_eq!(
        r.aabb,
        Some((VoxelCoord([100, 100, 100]), VoxelCoord([101, 102, 103])))
    );
}

#[test]
fn stamp_outside_the_extent_fails_with_the_first_missing_chunk_and_changes_nothing() {
    let world = solid3_world();
    let before = world.voxel_hash();
    // x 120..=135 and y -8..=7 meet chunks [1, -1, 0], [1, 0, 0] (resident) and
    // [2, -1, 0], [2, 0, 0] (outside test_extent); the first missing one ascending is
    // [2, -1, 0].
    let e = world
        .stamp(&Prefab::vault(16, 4), [120, -8, 0], Rot::IDENTITY)
        .unwrap_err();
    assert_eq!(e, EditError::NotResident(ChunkCoord([2, -1, 0])));
    assert_eq!(world.voxel_hash(), before);
}

fn vault_at(origin: [i32; 3], rot: Rot) -> (Arc<Prefab>, VoxelCoord, Rot) {
    (Arc::new(Prefab::vault(16, 4)), VoxelCoord(origin), rot)
}

#[test]
fn a_generated_placement_equals_the_runtime_stamp() {
    let runtime = solid3_world();
    runtime
        .stamp(&Prefab::vault(16, 4), [8, 8, 8], Rot::IDENTITY)
        .unwrap();

    // One chunk: compared as chunk [0, 0, 0], the only chunk the vault touches.
    let source = StampedSource::new(UniformSource(3), vec![vault_at([8, 8, 8], Rot::IDENTITY)]);
    let one = World::generate(
        ChunkExtent::new([0, 0, 0], [1, 1, 1]),
        &source,
        &test_table(),
    );
    let c0 = [ChunkCoord([0, 0, 0])];
    assert_eq!(chunks_hash(&one, &c0), chunks_hash(&runtime, &c0));
    assert_eq!(one.material_at([8, 8, 8]), Some(4));
    assert_eq!(one.material_at([15, 15, 15]), Some(0));
    assert_eq!(
        one.count_in_box([0, 0, 0], [63, 63, 63]),
        [(0, 2744), (3, 262_144 - 4096), (4, 1352)]
    );

    // The whole test extent: the full world hash.
    let all = World::generate(test_extent(), &source, &test_table());
    assert_eq!(all.voxel_hash(), runtime.voxel_hash());
}

#[test]
fn a_placement_straddling_two_chunks_generates_like_the_runtime_stamp() {
    let extent = ChunkExtent::new([0, 0, 0], [2, 1, 1]);
    let runtime = World::generate(extent, &UniformSource(3), &test_table());
    let r = runtime
        .stamp(&Prefab::vault(16, 4), [56, 8, 8], Rot::IDENTITY)
        .unwrap();
    assert_eq!(r.touched, [ChunkCoord([0, 0, 0]), ChunkCoord([1, 0, 0])]);
    assert_eq!(r.changed_from, [(3, 4096)]);

    let source = StampedSource::new(UniformSource(3), vec![vault_at([56, 8, 8], Rot::IDENTITY)]);
    let generated = World::generate(extent, &source, &test_table());
    assert_eq!(generated.voxel_hash(), runtime.voxel_hash());
    assert_eq!(generated.material_at([63, 8, 8]), Some(4));
    assert_eq!(generated.material_at([64, 9, 9]), Some(0));
    assert_eq!(generated.material_at([71, 8, 8]), Some(4));
    assert_eq!(generated.material_at([72, 8, 8]), Some(3));
}

#[test]
fn generation_with_placements_does_not_depend_on_the_thread_count() {
    let extent = ChunkExtent::new([-1, -1, -1], [2, 2, 1]);
    let placements = vec![
        vault_at([56, 8, 8], Rot::IDENTITY),
        vault_at([-8, -8, -8], Rot::Z90),
        (Arc::new(tripod()), VoxelCoord([63, 63, -2]), Rot::Z90),
        (
            Arc::new(tripod()),
            VoxelCoord([-1, 62, 60]),
            Rot::Z90.inverse(),
        ),
        // Reaches past the extent on +x and -y: clipped.
        vault_at([120, -70, 0], Rot::IDENTITY),
    ];
    let source = StampedSource::new(cave_like_source(), placements.clone());
    let one = World::generate_with_threads(extent, &source, &test_table(), 1);
    let seven = World::generate_with_threads(extent, &source, &test_table(), 7);
    assert_eq!(one.voxel_hash(), seven.voxel_hash());

    // And it equals the base world with each placement stamped in order, where the stamp
    // fits inside the extent.
    let runtime = World::generate(extent, &cave_like_source(), &test_table());
    for (p, origin, rot) in &placements[..4] {
        runtime.stamp(p, *origin, *rot).unwrap();
    }
    let clipped = StampedSource::new(cave_like_source(), placements[..4].to_vec());
    assert_eq!(
        World::generate_with_threads(extent, &clipped, &test_table(), 3).voxel_hash(),
        runtime.voxel_hash()
    );
    // The fifth placement only writes inside the extent.
    assert_eq!(one.material_at([120, -64, 0]), Some(4));
    assert_eq!(one.material_at([127, -55, 0]), Some(4));
}

#[test]
fn later_placements_win_where_they_overlap() {
    let extent = ChunkExtent::new([0, 0, 0], [1, 1, 1]);
    let t = (Arc::new(tripod()), VoxelCoord([8, 8, 8]), Rot::IDENTITY);
    let six = (
        Arc::new(Prefab::new([1, 1, 1], vec![6], None)),
        VoxelCoord([8, 8, 8]),
        Rot::IDENTITY,
    );
    let w = World::generate(
        extent,
        &StampedSource::new(UniformSource(3), vec![t.clone(), six.clone()]),
        &test_table(),
    );
    assert_eq!(w.material_at([8, 8, 8]), Some(6));
    assert_eq!(w.material_at([9, 8, 8]), Some(5));
    let w = World::generate(
        extent,
        &StampedSource::new(UniformSource(3), vec![six, t]),
        &test_table(),
    );
    assert_eq!(w.material_at([8, 8, 8]), Some(5));
}

#[test]
fn rot_apply_is_total_at_the_i32_extremes() {
    for r in Rot::all() {
        let v = r.apply([i32::MIN, i32::MAX, 0]);
        // A signed permutation: each component is one input component, negated or not,
        // and i32::MIN negates to itself.
        for c in v {
            assert!(
                [i32::MIN, i32::MAX, -i32::MAX, 0].contains(&c),
                "{r:?} {v:?}"
            );
        }
    }
}
