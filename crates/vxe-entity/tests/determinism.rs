//! Fixed vectors: seeded rays against a posed three-part model, and every tick of a three-part
//! clip, hash to committed constants, the same on every platform CI runs.

use std::sync::Arc;

use vxe_entity::{
    Clip, EntityHit, EntitySet, Key, Model, Part, PartPose, Pose, Quat, Track, blend, sample,
};
use vxe_world::hash::Fnv1a;
use vxe_world::{AIR, Face, Fixed3, MaterialId, Prefab};

/// A volume with a seeded pattern of holes and three materials, so a ray can pass between
/// voxels and the hit's material varies.
fn holed(size: [u32; 3], salt: u32) -> Arc<Prefab> {
    let mut voxels = Vec::new();
    for z in 0..size[2] {
        for y in 0..size[1] {
            for x in 0..size[0] {
                let h = (x * 7 + y * 13 + z * 31 + salt) % 5;
                voxels.push(if h == 0 { AIR } else { h as MaterialId });
            }
        }
    }
    Arc::new(Prefab::new(size, voxels, None))
}

fn model() -> Model {
    Model::new(vec![
        Part {
            volume: holed([8, 12, 6], 0),
            parent: None,
            pivot: [4 * 256, 6 * 256, 3 * 256],
            detail: 1,
        },
        Part {
            volume: holed([8, 8, 8], 1),
            parent: Some(0),
            pivot: [4 * 256, 0, 4 * 256],
            detail: 2,
        },
        Part {
            volume: holed([16, 4, 4], 2),
            parent: Some(0),
            pivot: [0, 2 * 256, 2 * 256],
            detail: 4,
        },
    ])
    .unwrap()
}

fn pose(model: &Model) -> Pose {
    let mut p = Pose::rest(model);
    p.root = Fixed3([1000 * 256 + 37, -20 * 256 + 3, 517 * 256 + 201]);
    // 45 degrees about z, then 30 degrees about x.
    p.root_q = Quat([15137, 0, 0, 6270]) * Quat([15826, 4240, 0, 0]);
    p.parts[1] = PartPose {
        t: [0, 6 * 256, 0],
        // 60 degrees about y.
        q: Quat([14189, 0, 8192, 0]),
    };
    p.parts[2] = PartPose {
        t: [4 * 256 + 77, 3 * 256, -100],
        // 90 degrees about z.
        q: Quat([11585, 0, 0, 11585]),
    };
    p
}

/// SplitMix64.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `[-span, span)`.
    fn within(&mut self, span: i64) -> i64 {
        (self.next() % (2 * span as u64)) as i64 - span
    }
}

fn face_index(f: Option<Face>) -> u8 {
    match f {
        None => 0,
        Some(Face::NegX) => 1,
        Some(Face::PosX) => 2,
        Some(Face::NegY) => 3,
        Some(Face::PosY) => 4,
        Some(Face::NegZ) => 5,
        Some(Face::PosZ) => 6,
    }
}

fn write_hit(h: &mut Fnv1a, hit: Option<EntityHit>) {
    match hit {
        None => h.write(&[0]),
        Some(e) => {
            h.write(&[1]);
            h.write(&e.entity.0.to_le_bytes());
            h.write(&[e.part]);
            h.write(&e.voxel);
            h.write(&[face_index(e.face)]);
            h.write(&e.material.to_le_bytes());
            h.write(&e.t.to_le_bytes());
        }
    }
}

/// The hit point lies on the hit voxel as `Pose::part_point` places it: within half the voxel's
/// diagonal of its centre, plus two units of rounding.
fn assert_near_the_voxel(set: &EntitySet, o: Fixed3, dir: [i64; 3], e: EntityHit) {
    let (model, pose) = set.get(e.entity).unwrap();
    let part = &model.parts()[usize::from(e.part)];
    let centre = pose
        .part_point(
            model,
            usize::from(e.part),
            e.voxel.map(|c| i32::from(c) * 256 + 128),
        )
        .unwrap();
    let at = [0, 1, 2].map(|k| o.0[k] as f64 + dir[k] as f64 * e.t as f64 / 256.0);
    let dist = (0..3)
        .map(|k| (at[k] - centre.0[k] as f64).powi(2))
        .sum::<f64>()
        .sqrt();
    let len = dir.iter().map(|&d| (d as f64).powi(2)).sum::<f64>().sqrt();
    // t is rounded down: up to one unit of t short of the face.
    let bound = 256.0 * 3f64.sqrt() / 2.0 / f64::from(part.detail) + 2.0 + len / 256.0;
    assert!(
        dist <= bound,
        "{e:?}: {dist} from the voxel centre, bound {bound}"
    );
}

/// The hash of 1024 rays: from points within 24 voxels of the root toward points within 8
/// voxels of it, `dir` the whole segment, `max_t` twice its length.
const ENTITY_HITS_HASH: u64 = 0xdb3d_3b37_c198_6f36;

#[test]
fn entity_hits_match_the_fixed_vectors() {
    let model = Arc::new(model());
    let pose = pose(&model);
    let root = pose.root.0;
    let mut set = EntitySet::new();
    set.insert(model, pose).unwrap();
    let mut rng = Rng(0x0062_0006);
    let mut h = Fnv1a::default();
    let mut hits = 0;
    let mut parts = [0; 3];
    for _ in 0..1024 {
        let o = root.map(|c| c + rng.within(24 * 256));
        let aim = root.map(|c| c + rng.within(8 * 256));
        let dir = [0, 1, 2].map(|k| aim[k] - o[k]);
        let hit = set.raycast(Fixed3(o), dir, 512);
        if let Some(e) = hit {
            hits += 1;
            parts[usize::from(e.part)] += 1;
            assert_near_the_voxel(&set, Fixed3(o), dir, e);
        }
        write_hit(&mut h, hit);
    }
    println!("hits {hits}, per part {parts:?}, hash {:#018x}", h.finish());
    assert!(parts.iter().all(|&n| n > 0), "every part is hit: {parts:?}");
    assert_eq!(h.finish(), ENTITY_HITS_HASH);
}

fn write_pose(h: &mut Fnv1a, p: &Pose) {
    for c in p.root.0 {
        h.write(&c.to_le_bytes());
    }
    for c in p.root_q.0 {
        h.write(&c.to_le_bytes());
    }
    for pp in &p.parts {
        for c in pp.t {
            h.write(&c.to_le_bytes());
        }
        for c in pp.q.0 {
            h.write(&c.to_le_bytes());
        }
    }
}

/// A 48-tick looped clip on the three-part model: the root part sways, part 1 turns through
/// keys whose neighbours include a negative dot product, part 2 slides and turns about two
/// axes. Keys sit at uneven ticks so the divisions round.
fn three_part_clip(model: &Model) -> Clip {
    let k = |tick, t, q| Key { tick, t, q };
    Clip::new(
        model,
        48,
        true,
        vec![
            Track {
                part: 0,
                keys: vec![
                    k(0, [0, 0, 0], Quat::IDENTITY),
                    k(17, [-301, 77, 5], Quat([15826, 4240, 0, 0])),
                    k(48, [0, 0, 0], Quat::IDENTITY),
                ],
            },
            Track {
                part: 1,
                keys: vec![
                    k(3, [0, 0, 0], Quat([14189, 0, 8192, 0])),
                    // 100 degrees about y, negated: dot < 0 with the key before.
                    k(25, [13, -7, 0], Quat([-10531, 0, -12551, 0])),
                    k(41, [0, 100, 0], Quat([0, 0, 0, 16384])),
                ],
            },
            Track {
                part: 2,
                keys: vec![
                    k(0, [256, 0, -256], Quat([11585, 0, 0, 11585])),
                    k(11, [-256, 3, 255], Quat([15137, 6270, 0, 0])),
                    k(29, [1, 1, 1], Quat([8192, 8192, 8192, 8192])),
                    k(47, [256, 0, -256], Quat([11585, 0, 0, 11585])),
                ],
            },
        ],
    )
    .unwrap()
}

/// The hash of the clip's poses at ticks 0 to 96 (two loops) from the posed rest of [`pose`],
/// then of each tick's pose blended at w 77 with tick 0's.
const CLIP_POSES_HASH: u64 = 0xd1aa_d34b_0a23_3d09;

#[test]
fn clip_poses_match_the_fixed_vectors() {
    let model = model();
    let rest = pose(&model);
    let clip = three_part_clip(&model);
    let mut h = Fnv1a::default();
    let first = sample(&model, &rest, &clip, 0);
    for tick in 0..=96 {
        let p = sample(&model, &rest, &clip, tick);
        write_pose(&mut h, &p);
        write_pose(&mut h, &blend(&first, &p, 77));
    }
    println!("clip hash {:#018x}", h.finish());
    assert_eq!(h.finish(), CLIP_POSES_HASH);
}
