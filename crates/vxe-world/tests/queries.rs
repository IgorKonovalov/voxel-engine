mod support;

use support::*;
use vxe_world::*;

#[test]
fn count_in_box_splits_the_carved_cube_by_material() {
    let world = carved_world();
    assert_eq!(
        world.count_in_box([-10, -10, -10], [10, 10, 10]).unwrap(),
        [(0, 4169), (3, 5092)]
    );
}

#[test]
fn count_in_box_over_uniform_bricks_reads_no_voxel_array() {
    let world = solid3_world();
    let (counts, stats) = world
        .count_in_box_with_stats([64, 64, 64], [127, 127, 127])
        .unwrap();
    assert_eq!(counts, [(3, 262_144)]);
    assert_eq!(stats.voxel_arrays_read, 0);

    // The carved world's Full bricks are read, and only they.
    let world = carved_world();
    let (_, stats) = world
        .count_in_box_with_stats([-64, -64, -64], [63, 63, 63])
        .unwrap();
    assert_eq!(stats.voxel_arrays_read, 32);
}

const UP: [i64; 3] = [0, 0, 256];

#[test]
fn raycast_hits_the_cavity_wall_at_an_exact_t() {
    let world = carved_world();
    assert_eq!(
        world
            .raycast(centre_of_voxel([0, 0, 0]), UP, 100 * 256)
            .unwrap(),
        Some(Hit {
            voxel: VoxelCoord([0, 0, 11]),
            face: Some(Face::NegZ),
            t: 2688,
            material: 3,
        })
    );
    assert_eq!(
        world.raycast(centre_of_voxel([0, 0, 0]), UP, 2688).unwrap(),
        world
            .raycast(centre_of_voxel([0, 0, 0]), UP, 100 * 256)
            .unwrap()
    );
    assert_eq!(
        world.raycast(centre_of_voxel([0, 0, 0]), UP, 2687).unwrap(),
        None
    );
}

#[test]
fn raycast_from_inside_a_solid_voxel_hits_at_zero() {
    let world = carved_world();
    assert_eq!(
        world
            .raycast(centre_of_voxel([0, 0, 11]), UP, 100 * 256)
            .unwrap(),
        Some(Hit {
            voxel: VoxelCoord([0, 0, 11]),
            face: None,
            t: 0,
            material: 3,
        })
    );
}

#[test]
fn raycast_faces_follow_the_direction() {
    let world = carved_world();
    let o = centre_of_voxel([0, 0, 0]);
    let hit = |d| world.raycast(o, d, 100 * 256).unwrap().unwrap();
    assert_eq!(hit([-256, 0, 0]).face, Some(Face::PosX));
    assert_eq!(hit([-256, 0, 0]).voxel, VoxelCoord([-11, 0, 0]));
    assert_eq!(hit([0, 256, 0]).face, Some(Face::NegY));
    assert_eq!(hit([0, 0, -256]).voxel, VoxelCoord([0, 0, -11]));
}

/// The first non-air voxel met stepping the ray by at most 1/1024 voxel per axis.
fn brute_force(world: &World, o: Fixed3, d: [i64; 3], max_voxels: i64) -> Option<VoxelCoord> {
    let k = 4 * d.iter().map(|c| c.abs()).max().unwrap();
    // Position in fixed point at step s is o + d * s / k: at most 1/4 fixed unit per step.
    let steps = max_voxels * 256 * 4;
    let mut last = None;
    for s in 0..=steps {
        let v = [0, 1, 2].map(|i| {
            let p = i128::from(o.0[i]) * i128::from(k) + i128::from(d[i]) * i128::from(s);
            p.div_euclid(i128::from(k) * 256) as i32
        });
        if last == Some(v) {
            continue;
        }
        last = Some(v);
        if world.material_at(v).is_some_and(|m| m != AIR) {
            return Some(VoxelCoord(v));
        }
    }
    None
}

#[test]
fn raycast_agrees_with_a_brute_force_walk() {
    let world = carved_world();
    let o = centre_of_voxel([0, 0, 0]);
    let mut rng = SplitMix64(0x0001_ca57);
    let mut dirs = vec![[256, 256, 256]];
    while dirs.len() < 10 {
        dirs.push([0; 3].map(|_| rng.range(-400, 400)));
    }
    for d in dirs {
        let hit = world.raycast(o, d, 64 * 256).unwrap().map(|h| h.voxel);
        assert_eq!(hit, brute_force(&world, o, d, 30), "dir {d:?}");
        assert!(hit.is_some(), "dir {d:?}");
    }
}

#[test]
fn raycast_hit_list_is_pinned() {
    let world = carved_world();
    let mut rng = SplitMix64(0x5EED_0003);
    let mut h = Fnv1a::default();
    let (mut misses, mut inside, mut walls) = (0, 0, 0);
    for _ in 0..1000 {
        let o = Fixed3([0; 3].map(|_| rng.range(-9 * 256, 9 * 256)));
        let d = [0; 3].map(|_| rng.range(-1024, 1024));
        let max_t = rng.range(0, 40 * 256);
        match world.raycast(o, d, max_t).unwrap() {
            None => {
                misses += 1;
                h.write(&[0]);
            }
            Some(hit) => {
                if hit.face.is_none() {
                    inside += 1;
                } else {
                    walls += 1;
                }
                h.write(&[1]);
                for c in hit.voxel.0 {
                    h.write(&c.to_le_bytes());
                }
                h.write(&[hit.face.map_or(0, |f| f as u8 + 1)]);
                h.write(&hit.t.to_le_bytes());
                h.write(&hit.material.to_le_bytes());
            }
        }
    }
    // The set exercises misses within max_t, starts inside rock, and wall hits.
    assert!(
        misses > 20 && inside > 200 && walls > 500,
        "{misses} {inside} {walls}"
    );
    // Blessed on Linux x86_64; a change here is a change in raycast results.
    assert_eq!(h.0, 0xa0f2_c477_8892_c8fb);
}

#[test]
fn trace_materials_reports_the_cavity_and_the_rock() {
    let world = carved_world();
    assert_eq!(
        world.trace_materials(centre_of_voxel([0, 0, 0]), UP, 100 * 256),
        [
            Run {
                material: 0,
                t0: 0,
                t1: 2688
            },
            Run {
                material: 3,
                t0: 2688,
                t1: 25600
            },
        ]
    );
}

#[test]
fn trace_materials_splits_at_a_layer_of_another_material() {
    let world = carved_world();
    // z in [20, 30) becomes material 2 over the whole extent.
    world
        .apply(&EditOp::box_(
            Fixed3([-128 * 256, -128 * 256, 20 * 256]),
            Fixed3([128 * 256, 128 * 256, 30 * 256]),
            Mode::Replace { from: 3, to: 2 },
        ))
        .unwrap();
    let runs = world.trace_materials(centre_of_voxel([0, 0, 0]), UP, 100 * 256);
    // Faces at z = 20 and z = 30 seen from z = 0.5: 19.5 and 29.5 voxels.
    assert_eq!(
        runs,
        [
            Run {
                material: 0,
                t0: 0,
                t1: 2688
            },
            Run {
                material: 3,
                t0: 2688,
                t1: 4992
            },
            Run {
                material: 2,
                t0: 4992,
                t1: 7552
            },
            Run {
                material: 3,
                t0: 7552,
                t1: 25600
            },
        ]
    );
}

#[test]
fn trace_materials_covers_missing_chunks_as_air() {
    let world = carved_world();
    // From inside the cavity out past the extent's +z edge at z = 128.
    let runs = world.trace_materials(centre_of_voxel([0, 0, 0]), UP, 200 * 256);
    assert_eq!(
        runs.last(),
        Some(&Run {
            material: 0,
            t0: 128 * 256 - 128,
            t1: 200 * 256
        })
    );
    assert_eq!(runs.len(), 3);
}

#[test]
fn trace_materials_agrees_with_the_raycast_and_the_voxels_it_crosses() {
    let world = carved_world();
    let mut rng = SplitMix64(0x0006_0003);
    for _ in 0..200 {
        let o = Fixed3([0; 3].map(|_| rng.range(-9 * 256, 9 * 256)));
        let d = [0; 3].map(|_| rng.range(-1024, 1024));
        let max_t = rng.range(0, 40 * 256);
        let runs = world.trace_materials(o, d, max_t);
        // The runs tile [0, max_t] and alternate material.
        assert_eq!(runs.first().map(|r| r.t0), Some(0));
        assert_eq!(runs.last().map(|r| r.t1), Some(max_t));
        for w in runs.windows(2) {
            assert_eq!(w[0].t1, w[1].t0);
            assert_ne!(w[0].material, w[1].material);
        }
        // The first solid run starts where the raycast hits.
        let first_solid = runs.iter().find(|r| r.material != AIR).map(|r| r.t0);
        assert_eq!(
            first_solid,
            world.raycast(o, d, max_t).unwrap().map(|h| h.t),
            "{o:?} {d:?}"
        );
        // Sampled along the ray away from run boundaries, each point's voxel holds its run's
        // material.
        for t in (0..=max_t).step_by(37) {
            let Some(run) = runs.iter().find(|r| r.t0 + 2 <= t && t + 2 <= r.t1) else {
                continue;
            };
            let v = [0, 1, 2].map(|k| {
                (i128::from(o.0[k]) * 256 + i128::from(d[k]) * i128::from(t)).div_euclid(256 * 256)
                    as i32
            });
            assert_eq!(
                world.material_at(v).unwrap_or(AIR),
                run.material,
                "{o:?} {d:?} t {t}"
            );
        }
    }
}

#[test]
fn trace_materials_list_is_pinned() {
    let world = carved_world();
    let mut rng = SplitMix64(0x5EED_0006);
    let mut h = Fnv1a::default();
    let mut runs_total = 0;
    for _ in 0..500 {
        // From inside the cavity, through the rock and out of the extent into missing chunks.
        let o = Fixed3([0; 3].map(|_| rng.range(-9 * 256, 9 * 256)));
        let d = [0; 3].map(|_| rng.range(-1024, 1024));
        let max_t = rng.range(0, 200 * 256);
        let runs = world.trace_materials(o, d, max_t);
        runs_total += runs.len();
        h.write(&(runs.len() as u32).to_le_bytes());
        for r in runs {
            h.write(&r.material.to_le_bytes());
            h.write(&r.t0.to_le_bytes());
            h.write(&r.t1.to_le_bytes());
        }
    }
    assert!(runs_total > 1000, "{runs_total}");
    // Blessed on Linux x86_64; a change here is a change in trace_materials results.
    assert_eq!(h.0, 0xbae9_1463_c336_8745);
}

/// A ray's reach is capped per axis at `|dir[k]| x max_t <= 256 x 2^22`, whatever `dir`; its
/// origin is a coordinate like any other.
#[test]
fn rays_past_the_reach_limit_are_refused() {
    let world = World::new(&test_table());
    let o = centre_of_voxel([0, 0, 0]);
    let max = 1i64 << 22;
    let refused = |what| Err(EditError::OutOfRange { what });
    assert_eq!(world.raycast(o, [256, 0, 0], max), Ok(None));
    assert_eq!(
        world.raycast(o, [256, 0, 0], max + 1),
        refused(Field::RayLength)
    );
    assert_eq!(
        world.raycast(o, [1 << 40, 0, 0], 1),
        refused(Field::RayLength)
    );
    assert_eq!(world.raycast(o, [256; 3], max), Ok(None));
    assert_eq!(
        world.raycast(o, [-256, 0, i64::MIN], 2),
        refused(Field::RayLength)
    );
    assert_eq!(
        world.raycast(Fixed3([0, 0, (1 << 30) + 1]), [256, 0, 0], 256),
        refused(Field::Coord)
    );
}

/// A box query is capped at 1 024 voxels per axis and `2^30` voxels; a world built
/// `with_limits` takes its own caps.
#[test]
fn box_queries_past_the_limits_are_refused() {
    let world = carved_world();
    assert_eq!(
        world.count_in_box([0, 0, 0], [1024, 0, 0]),
        Err(EditError::OutOfRange {
            what: Field::QueryAxis
        })
    );
    assert_eq!(
        world
            .count_in_box_with_stats([0, 0, -1024], [0, 0, 0])
            .map(|(c, _)| c),
        Err(EditError::OutOfRange {
            what: Field::QueryAxis
        })
    );
    let (counts, _) = world
        .count_in_box_with_stats([-512; 3], [511; 3])
        .expect("1 024^3 is within the limits");
    // Every voxel of the 256³-voxel resident extent lies in the box.
    assert_eq!(counts.iter().map(|&(_, n)| n).sum::<u64>(), 256 * 256 * 256);
    let volume = WorldLimits {
        query_voxels_max: 1023,
        ..WorldLimits::DEFAULT
    };
    let small = carved_world().with_limits(volume);
    assert_eq!(small.limits(), volume);
    assert_eq!(
        small.count_in_box([0, 0, 0], [1023, 0, 0]),
        Err(EditError::OutOfRange {
            what: Field::QueryVolume
        })
    );
    let wide = carved_world().with_limits(WorldLimits {
        query_axis_max: 2048,
        ..WorldLimits::DEFAULT
    });
    assert!(wide.count_in_box([0, 0, 0], [1024, 0, 0]).is_ok());
    // A reversed box is empty, whatever its size.
    assert_eq!(world.count_in_box([5000, 0, 0], [0, 0, 0]), Ok(vec![]));
}
