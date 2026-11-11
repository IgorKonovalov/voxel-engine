//! The entity ray query on hand-checked poses, and against a float reference on a tilted part.

use std::sync::Arc;

use vxe_entity::{EntitySet, Model, ModelError, Part, PartPose, Pose, Quat};
use vxe_world::{Face, Fixed3, MaterialId, Prefab};

const STONE: MaterialId = 3;
/// A half turn about y, `w, x, y, z` in Q14.
const Y180: Quat = Quat([0, 0, 16384, 0]);

fn solid(edge: u32) -> Arc<Prefab> {
    let n = (edge * edge * edge) as usize;
    Arc::new(Prefab::new([edge; 3], vec![STONE; n], None))
}

/// A part of a solid cube of `edge` model voxels, its pivot at the centre.
fn cube_part(edge: u32, detail: u8, parent: Option<u8>) -> Part {
    Part {
        volume: solid(edge),
        parent,
        pivot: [edge as i32 * 128; 3],
        detail,
    }
}

/// `model` placed with its root (the pivot of a root part) at voxel (104, 54, 104), so an
/// 8-world-voxel cube about it has its minimum corner at (100, 50, 100).
fn placed(model: Model, part_q: Quat) -> (EntitySet, Arc<Model>) {
    let model = Arc::new(model);
    let mut pose = Pose::rest(&model);
    pose.root = Fixed3([104 * 256, 54 * 256, 104 * 256]);
    pose.parts[0].q = part_q;
    let mut set = EntitySet::new();
    set.insert(model.clone(), pose).unwrap();
    (set, model)
}

/// (90.5, 54.25, 104.25) in 1/256 voxel.
const ORIGIN: Fixed3 = Fixed3([90 * 256 + 128, 54 * 256 + 64, 104 * 256 + 64]);
const PLUS_X: [i64; 3] = [256, 0, 0];
/// 9.5 voxels.
const NINE_AND_A_HALF: i64 = 9 * 256 + 128;

#[test]
fn a_ray_names_the_part_voxel_and_face() {
    let (set, _) = placed(
        Model::new(vec![cube_part(8, 1, None)]).unwrap(),
        Quat::IDENTITY,
    );
    let hit = set.raycast(ORIGIN, PLUS_X, 64 * 256).unwrap();
    assert_eq!(hit.part, 0);
    assert_eq!(hit.voxel, [0, 4, 4]);
    assert_eq!(hit.face, Some(Face::NegX));
    assert_eq!(hit.material, STONE);
    assert_eq!(hit.t, NINE_AND_A_HALF);
}

#[test]
fn a_half_turn_maps_the_hit_to_the_far_voxel() {
    let (set, _) = placed(Model::new(vec![cube_part(8, 1, None)]).unwrap(), Y180);
    let hit = set.raycast(ORIGIN, PLUS_X, 64 * 256).unwrap();
    assert_eq!(hit.part, 0);
    assert_eq!(hit.voxel, [7, 4, 3]);
    assert_eq!(hit.face, Some(Face::PosX));
    assert_eq!(hit.material, STONE);
    assert_eq!(hit.t, NINE_AND_A_HALF);
}

#[test]
fn detail_two_halves_the_voxel() {
    let (set, _) = placed(
        Model::new(vec![cube_part(16, 2, None)]).unwrap(),
        Quat::IDENTITY,
    );
    let hit = set.raycast(ORIGIN, PLUS_X, 64 * 256).unwrap();
    assert_eq!(hit.voxel, [0, 8, 8]);
    assert_eq!(hit.face, Some(Face::NegX));
    assert_eq!(hit.t, NINE_AND_A_HALF);
}

#[test]
fn a_child_follows_its_parent() {
    let model = Model::new(vec![cube_part(8, 1, None), cube_part(4, 1, Some(0))]).unwrap();
    let child_pivot = model.parts()[1].pivot;
    let mut pose = Pose::rest(&model);
    pose.root = Fixed3([104 * 256, 54 * 256, 104 * 256]);
    // The child's pivot 8 voxels along +x from the parent's.
    pose.parts[1] = PartPose {
        t: [8 * 256, 0, 0],
        q: Quat::IDENTITY,
    };
    let parent_pivot = pose.part_point(&model, 0, model.parts()[0].pivot).unwrap();
    assert_eq!(parent_pivot, pose.root);
    assert_eq!(
        pose.part_point(&model, 1, child_pivot).unwrap(),
        Fixed3([112 * 256, 54 * 256, 104 * 256])
    );
    pose.parts[0].q = Y180;
    assert_eq!(
        pose.part_point(&model, 0, model.parts()[0].pivot).unwrap(),
        parent_pivot
    );
    assert_eq!(
        pose.part_point(&model, 1, child_pivot).unwrap(),
        Fixed3([96 * 256, 54 * 256, 104 * 256])
    );
    // A point one voxel along the child's +x lands one voxel along world -x.
    let ahead = [child_pivot[0] + 256, child_pivot[1], child_pivot[2]];
    assert_eq!(
        pose.part_point(&model, 1, ahead).unwrap(),
        Fixed3([95 * 256, 54 * 256, 104 * 256])
    );
}

/// SplitMix64: the seeded stream the ray tests draw from.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `[-1, 1)`.
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 52) as f64 - 1.0
    }
}

/// The parameter where the ray `o + d * t / 256` (1/256 voxel) enters the box `[0, 8)^3` in the
/// space of a cube turned `angle` about z through its centre at `c`, by slabs in `f64`.
fn float_entry(o: [f64; 3], d: [f64; 3], c: [f64; 3], angle: f64) -> Option<f64> {
    let (s, co) = angle.sin_cos();
    // R^T v for R a turn about z.
    let back = |v: [f64; 3]| [co * v[0] + s * v[1], -s * v[0] + co * v[1], v[2]];
    let rel = back([0, 1, 2].map(|k| (o[k] - c[k]) / 256.0));
    let lo = rel.map(|x| x + 4.0);
    // Voxels per unit of t: the world point is (o + d * t / 256) / 256.
    let ld = back(d.map(|x| x / 65536.0));
    let (mut enter, mut exit) = (0.0f64, f64::INFINITY);
    for k in 0..3 {
        if ld[k] == 0.0 {
            if !(0.0..8.0).contains(&lo[k]) {
                return None;
            }
            continue;
        }
        let a = (0.0 - lo[k]) / ld[k];
        let b = (8.0 - lo[k]) / ld[k];
        enter = enter.max(a.min(b));
        exit = exit.min(a.max(b));
    }
    (enter < exit).then_some(enter)
}

#[test]
fn a_tilted_part_agrees_with_the_float_reference() {
    // 45 degrees about z: cos and sin of 22.5 degrees in Q14.
    let q = Quat([15137, 0, 0, 6270]);
    let (set, _) = placed(Model::new(vec![cube_part(8, 1, None)]).unwrap(), q);
    let centre = [104.0 * 256.0, 54.0 * 256.0, 104.0 * 256.0];
    let max_t = 64 * 256;
    let mut rng = Rng(0x0062_0005);
    let (mut both, mut disagree, mut worst) = (0, 0, 0.0f64);
    for _ in 0..256 {
        // From a shell 16 to 24 voxels out, toward a point within 6 voxels of the centre.
        let mut dir = [rng.unit(), rng.unit(), rng.unit()];
        let len = dir.iter().map(|x| x * x).sum::<f64>().sqrt().max(1e-9);
        let r = 16.0 + 8.0 * (rng.unit() + 1.0) / 2.0;
        dir = dir.map(|x| x / len * r);
        let o = [0, 1, 2].map(|k| (centre[k] + dir[k] * 256.0).round());
        let aim = [0, 1, 2].map(|k| centre[k] + 6.0 * 256.0 * rng.unit());
        let to = [0, 1, 2].map(|k| aim[k] - o[k]);
        let n = to.iter().map(|x| x * x).sum::<f64>().sqrt();
        let d = to.map(|x| (x / n * 256.0).round());
        let origin = Fixed3(o.map(|x| x as i64));
        let di = d.map(|x| x as i64);
        let int = set.raycast(origin, di, max_t);
        let flt =
            float_entry(o, d, centre, std::f64::consts::FRAC_PI_4).filter(|&t| t <= max_t as f64);
        match (int, flt) {
            (Some(h), Some(t)) => {
                both += 1;
                // Distance in voxels: t * |d| / 256 / 256.
                let dlen = d.iter().map(|x| x * x).sum::<f64>().sqrt();
                let err = (h.t as f64 - t).abs() * dlen / 256.0 / 256.0;
                worst = worst.max(err);
                assert!(err <= 1.0 / 64.0, "ray {origin:?} {di:?}: {} vs {t}", h.t);
            }
            (None, None) => {}
            _ => disagree += 1,
        }
    }
    println!("hits both {both}, disagree {disagree}, worst {worst:.6} voxel");
    assert!(both >= 64, "too few hits to judge: {both}");
    assert!(disagree <= 2, "{disagree} rays disagree on hit or miss");
}

#[test]
fn a_ray_past_max_t_or_beside_the_box_hits_nothing() {
    let (set, _) = placed(
        Model::new(vec![cube_part(8, 1, None)]).unwrap(),
        Quat::IDENTITY,
    );
    assert!(set.raycast(ORIGIN, PLUS_X, NINE_AND_A_HALF - 1).is_none());
    assert_eq!(
        set.raycast(ORIGIN, PLUS_X, NINE_AND_A_HALF).map(|h| h.t),
        Some(NINE_AND_A_HALF)
    );
    assert!(set.raycast(ORIGIN, PLUS_X, -1).is_none());
    // Half a voxel above the cube's top face at y = 58, and pointing away from it.
    let above = Fixed3([ORIGIN.0[0], 58 * 256 + 128, ORIGIN.0[2]]);
    assert!(set.raycast(above, PLUS_X, 64 * 256).is_none());
    assert!(set.raycast(ORIGIN, [-256, 0, 0], 64 * 256).is_none());
    // An empty set.
    assert!(EntitySet::new().raycast(ORIGIN, PLUS_X, 64 * 256).is_none());
}

#[test]
fn model_new_refuses_a_cycle_a_seventeenth_part_and_an_oversized_volume() {
    assert_eq!(
        Model::new(vec![cube_part(8, 1, Some(0))]),
        Err(ModelError::ParentCycle { part: 0, parent: 0 })
    );
    assert_eq!(
        Model::new(vec![
            cube_part(4, 1, None),
            cube_part(4, 1, Some(2)),
            cube_part(4, 1, Some(1))
        ]),
        Err(ModelError::ParentCycle { part: 1, parent: 2 })
    );
    let seventeen = (0..17).map(|_| cube_part(2, 1, None)).collect();
    assert_eq!(
        Model::new(seventeen),
        Err(ModelError::TooManyParts { count: 17 })
    );
    let sixteen: Vec<Part> = (0..16).map(|_| cube_part(2, 1, None)).collect();
    assert!(Model::new(sixteen).is_ok());
    let long = Arc::new(Prefab::new([65, 1, 1], vec![STONE; 65], None));
    let part = Part {
        volume: long,
        parent: None,
        pivot: [0; 3],
        detail: 1,
    };
    assert_eq!(
        Model::new(vec![part]),
        Err(ModelError::OversizedVolume {
            part: 0,
            size: [65, 1, 1]
        })
    );
    assert!(Model::new(vec![cube_part(64, 4, None)]).is_ok());
}
