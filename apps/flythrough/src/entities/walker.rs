//! The built-in walker (ADR-0050): a voxel model of four rigid parts made in code, a torso, a
//! head with two glowing eyes and two legs, all at detail 2. It is the entry placed when the
//! entities library is missing or empty (`super::library`), so a run from an unpacked tester
//! archive still places something; `walkers walk` (or `--entities-walk on`) sets the placed
//! built-in walkers walking, each on a circle from where it stands with its legs swinging,
//! and `eye lights` lights their eyes. The engine knows no walker: this is the fly-through's
//! entity.

use std::sync::Arc;

use vxe_entity::{Model, Part, PartPose, Pose, Q14_ONE, Quat};
use vxe_world::{AIR, Fixed3, Material, MaterialId, MaterialTable, Prefab};

/// The walker's body row's name in the table ([`with_walker`]).
pub const BODY_ROW: &str = "walker";
/// The eyes' row's name: it emits.
pub const EYE_ROW: &str = "walker eye";
/// The eyes' emission, radiance per channel: a warm glow.
pub const EYE_EMISSION: [f32; 3] = [3.0, 1.6, 0.5];
/// Model voxels per world voxel.
pub const DETAIL: u8 = 2;
/// The torso's size in model voxels: 8 wide (x), 6 deep (y), 12 tall (z).
pub const TORSO: [u32; 3] = [8, 6, 12];
/// The head's size: a 6-voxel cube; its eyes are on its front face (y 0).
pub const HEAD: [u32; 3] = [6, 6, 6];
/// A leg's size: 3 x 3, 8 tall.
pub const LEG: [u32; 3] = [3, 3, 8];
/// A walker's eye light: warm, intensity 150, casting shadows, not bouncing.
pub const EYE_LIGHT_COLOUR: [f32; 3] = [1.0, 0.7, 0.4];
pub const EYE_LIGHT_INTENSITY: f32 = 150.0;
/// Where the eye light sits in the head's space, in 1/256 model voxel: between the eyes'
/// centres in x and z, one model voxel in front of the face (a light inside the head would be
/// shadowed by it).
pub const EYE_LIGHT_AT: [i32; 3] = [3 * 256, -256, 896];
/// The ring `--entities` places walkers on: 3 m around the spawn.
pub const RING_RADIUS: f32 = 30.0;
/// How far down from the ring's points a walker looks for the floor, in voxels.
pub(crate) const FLOOR_SEARCH: i64 = 200;
/// The radius of the circle a walking walker walks from where it stands: 2 m.
pub const WALK_RADIUS: f32 = 20.0;
/// Its pace along the circle: 1 m/s, in voxels per second.
pub const WALK_SPEED: f32 = 10.0;
/// Its legs' swing either way: 30 degrees.
pub const WALK_SWING: f32 = std::f32::consts::FRAC_PI_6;
/// Its strides per second (one swing forward and back of each leg).
pub const WALK_STRIDES: f32 = 1.0;

/// `table` with the walker's two rows appended: its body and its emissive eyes.
pub fn with_walker(table: &MaterialTable) -> MaterialTable {
    let mut t = table.clone();
    t.push(Material {
        albedo: [0.55, 0.52, 0.48],
        ..Material::named(BODY_ROW)
    });
    t.push(Material {
        albedo: [0.9, 0.7, 0.4],
        emission: EYE_EMISSION,
        ..Material::named(EYE_ROW)
    });
    t
}

/// The ids of the walker's rows in `table`, when it has them.
pub fn rows(table: &MaterialTable) -> Option<(MaterialId, MaterialId)> {
    let find = |name: &str| {
        table
            .rows()
            .iter()
            .position(|m| m.name == name)
            .and_then(|i| MaterialId::try_from(i).ok())
    };
    Some((find(BODY_ROW)?, find(EYE_ROW)?))
}

fn solid(size: [u32; 3], m: MaterialId) -> Vec<MaterialId> {
    vec![m; (size[0] * size[1] * size[2]) as usize]
}

/// The walker's model: torso (part 0, hung from the root), head (1) on the torso, left and
/// right legs (2, 3) from the torso's hips, the legs sharing one prefab. Each part's pivot is
/// where it joins its parent: the torso's and head's bottom centres, the legs' top centres.
pub fn model(body: MaterialId, eye: MaterialId) -> Model {
    let mut head = solid(HEAD, body);
    for x in [1usize, 4] {
        // Voxel (x, 0, 3): the front face, a little above the middle.
        head[x + 6 * 6 * 3] = eye;
    }
    let leg = Arc::new(Prefab::new(LEG, solid(LEG, body), None));
    let parts = vec![
        Part {
            volume: Arc::new(Prefab::new(TORSO, solid(TORSO, body), None)),
            parent: None,
            pivot: [4 * 256, 3 * 256, 0],
            detail: DETAIL,
            coarse: 1,
        },
        Part {
            volume: Arc::new(Prefab::new(HEAD, head, None)),
            parent: Some(0),
            pivot: [3 * 256, 3 * 256, 0],
            detail: DETAIL,
            coarse: 1,
        },
        Part {
            volume: leg.clone(),
            parent: Some(0),
            pivot: [384, 384, 8 * 256],
            detail: DETAIL,
            coarse: 1,
        },
        Part {
            volume: leg,
            parent: Some(0),
            pivot: [384, 384, 8 * 256],
            detail: DETAIL,
            coarse: 1,
        },
    ];
    Model::new(parts).expect("the walker's parts are valid")
}

/// A turn of `angle` radians about axis `axis` (0 x, 1 y, 2 z) in Q14.
pub fn turn(axis: usize, angle: f32) -> Quat {
    let (s, c) = (0.5 * angle).sin_cos();
    let mut q = [(c * Q14_ONE as f32).round() as i16, 0, 0, 0];
    q[1 + axis] = (s * Q14_ONE as f32).round() as i16;
    Quat(q)
}

/// The walker standing with its feet's centre at `feet` (world voxels), turned `yaw` radians
/// about z from facing -y, its legs swung `swing` radians about x, opposite ways.
pub fn pose(feet: [f32; 3], yaw: f32, swing: f32) -> Pose {
    let fixed = |v: f32| (v * 256.0).round() as i64;
    // Each offset in 1/256 world voxel: a model voxel is half a world voxel.
    let leg_top = (LEG[2] as i32) * 128;
    Pose {
        root: Fixed3(feet.map(fixed)),
        root_q: turn(2, yaw),
        parts: vec![
            PartPose {
                t: [0, 0, leg_top],
                q: Quat::IDENTITY,
            },
            PartPose {
                t: [0, 0, TORSO[2] as i32 * 128],
                q: Quat::IDENTITY,
            },
            PartPose {
                t: [-256, 0, 0],
                q: turn(0, swing),
            },
            PartPose {
                t: [256, 0, 0],
                q: turn(0, -swing),
            },
        ],
    }
}

/// The yaw that turns a walker at `at` to face `eye` (its front is -y at yaw 0).
pub fn facing(at: [f32; 3], eye: [f32; 3]) -> f32 {
    let (dx, dy) = (eye[0] - at[0], eye[1] - at[1]);
    // -y turned by yaw about z is (sin yaw, -cos yaw).
    dx.atan2(-dy)
}

/// Voxels of the walker's model that are not air, per part, for the readings.
pub fn solid_voxels(model: &Model) -> Vec<usize> {
    model
        .parts()
        .iter()
        .map(|p| {
            let s = p.volume.size();
            let mut n = 0;
            for z in 0..s[2] as i32 {
                for y in 0..s[1] as i32 {
                    for x in 0..s[0] as i32 {
                        n += usize::from(p.volume.get([x, y, z]).is_some_and(|m| m != AIR));
                    }
                }
            }
            n
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_walker_has_four_parts_and_two_glowing_eyes() {
        let table = with_walker(&MaterialTable::new());
        let (body, eye) = rows(&table).unwrap();
        let m = model(body, eye);
        assert_eq!(m.parts().len(), 4);
        let head = &m.parts()[1].volume;
        assert_eq!(head.get([1, 0, 3]), Some(eye));
        assert_eq!(head.get([4, 0, 3]), Some(eye));
        assert_eq!(head.get([2, 0, 3]), Some(body));
        assert_eq!(table.get(eye).unwrap().emission, EYE_EMISSION);
        assert_eq!(m.parts()[2].volume.id(), m.parts()[3].volume.id());
    }

    #[test]
    fn a_walker_faces_the_eye() {
        // At yaw 0 the front is -y: an eye at -y gives yaw 0, one at +x a quarter turn.
        assert!(facing([0.0; 3], [0.0, -5.0, 0.0]).abs() < 1e-6);
        let q = facing([0.0; 3], [5.0, 0.0, 0.0]);
        assert!((q - std::f32::consts::FRAC_PI_2).abs() < 1e-6, "{q}");
    }
}
