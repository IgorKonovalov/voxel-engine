mod support;

use support::*;
use vxe_world::*;

#[test]
fn count_in_box_splits_the_carved_cube_by_material() {
    let world = carved_world();
    assert_eq!(
        world.count_in_box([-10, -10, -10], [10, 10, 10]),
        [(0, 4169), (3, 5092)]
    );
}

#[test]
fn count_in_box_over_uniform_bricks_reads_no_voxel_array() {
    let world = solid3_world();
    let (counts, stats) = world.count_in_box_with_stats([64, 64, 64], [127, 127, 127]);
    assert_eq!(counts, [(3, 262_144)]);
    assert_eq!(stats.voxel_arrays_read, 0);

    // The carved world's Full bricks are read, and only they.
    let world = carved_world();
    let (_, stats) = world.count_in_box_with_stats([-64, -64, -64], [63, 63, 63]);
    assert_eq!(stats.voxel_arrays_read, 32);
}

const UP: [i64; 3] = [0, 0, 256];

#[test]
fn raycast_hits_the_cavity_wall_at_an_exact_t() {
    let world = carved_world();
    assert_eq!(
        world.raycast(centre_of_voxel([0, 0, 0]), UP, 100 * 256),
        Some(Hit {
            voxel: VoxelCoord([0, 0, 11]),
            face: Some(Face::NegZ),
            t: 2688,
            material: 3,
        })
    );
    assert_eq!(
        world.raycast(centre_of_voxel([0, 0, 0]), UP, 2688),
        world.raycast(centre_of_voxel([0, 0, 0]), UP, 100 * 256)
    );
    assert_eq!(world.raycast(centre_of_voxel([0, 0, 0]), UP, 2687), None);
}

#[test]
fn raycast_from_inside_a_solid_voxel_hits_at_zero() {
    let world = carved_world();
    assert_eq!(
        world.raycast(centre_of_voxel([0, 0, 11]), UP, 100 * 256),
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
    let hit = |d| world.raycast(o, d, 100 * 256).unwrap();
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
        let hit = world.raycast(o, d, 64 * 256).map(|h| h.voxel);
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
        match world.raycast(o, d, max_t) {
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
