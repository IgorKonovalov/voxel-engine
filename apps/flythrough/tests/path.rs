//! The camera paths (`flythrough::path`): the sampler's exactness at keys, its shape between
//! them and its shortest-arc yaw, and the headless guard that every path flies in air.

use flythrough::path::{CameraPath, Keyframe, PATH_SEED, SHOWCASE, Scene, key};
use flythrough::showcase::ShowcaseSource;
use vxe_world::{AIR, MaterialId};

/// Samples the air test reads along a path, seconds apart.
const AIR_STEP_S: f32 = 0.25;
/// Voxels from a sample to the six it checks along the axes: 1 m.
const CLEARANCE: i32 = 10;

fn path_of(keys: &'static [Keyframe]) -> CameraPath {
    CameraPath {
        name: "test",
        scene: Scene::Cave,
        keys,
    }
}

#[test]
fn sampling_at_a_key_returns_its_eye_exactly() {
    for k in SHOWCASE.keys {
        let p = SHOWCASE.sample(k.t_s);
        assert_eq!(p.eye, k.eye, "key at {} s", k.t_s);
        assert_eq!(p.pitch_deg, k.pitch_deg, "key at {} s", k.t_s);
    }
    let last = SHOWCASE.keys.last().copied().expect("keys");
    assert_eq!(SHOWCASE.sample(SHOWCASE.duration_s()).eye, last.eye);
    assert_eq!(SHOWCASE.sample(SHOWCASE.duration_s() + 5.0).eye, last.eye);
    assert_eq!(SHOWCASE.sample(-1.0).eye, SHOWCASE.keys[0].eye);
}

/// Three keys on one line, unevenly spaced: the eye stays on the line and moves forward along
/// it at every step, through the middle key and on to the last.
#[test]
fn a_straight_path_moves_monotonically_along_its_chord() {
    static KEYS: [Keyframe; 3] = [
        key(0.0, [0.0, 0.0, 0.0], 0.0, 0.0),
        key(1.0, [10.0, 5.0, 2.0], 0.0, 0.0),
        key(3.0, [60.0, 30.0, 12.0], 0.0, 0.0),
    ];
    let path = path_of(&KEYS);
    let dir = [60.0f32, 30.0, 12.0];
    let len = dir.iter().map(|v| v * v).sum::<f32>().sqrt();
    let along = |e: [f32; 3]| (0..3).map(|k| e[k] * dir[k]).sum::<f32>() / len;
    let mut last = -1.0;
    for s in 0..=300 {
        let t = s as f32 * 0.01;
        let e = path.sample(t).eye;
        let a = along(e);
        // Off the line by the distance from the eye to its projection.
        let off = (0..3)
            .map(|k| (e[k] - dir[k] / len * a).powi(2))
            .sum::<f32>()
            .sqrt();
        assert!(off < 1e-3, "off the chord by {off} at {t} s");
        assert!(a > last, "along {a} after {last} at {t} s");
        last = a;
    }
}

#[test]
fn a_yaw_from_350_to_10_turns_through_20_degrees() {
    static KEYS: [Keyframe; 2] = [
        key(0.0, [0.0, 0.0, 0.0], 350.0, 0.0),
        key(2.0, [20.0, 0.0, 0.0], 10.0, 0.0),
    ];
    let path = path_of(&KEYS);
    let mut last = 350.0;
    for s in 0..=200 {
        let yaw = path.sample(s as f32 * 0.01).yaw_deg;
        assert!(
            (350.0..=370.0).contains(&yaw),
            "yaw {yaw} outside the 20-degree arc"
        );
        assert!(yaw >= last, "yaw went back from {last} to {yaw}");
        last = yaw;
    }
    assert_eq!(path.sample(2.0).yaw_deg, 370.0);
    assert_eq!(path.sample(2.0).yaw_deg.rem_euclid(360.0), 10.0);
}

/// Every key and every [`AIR_STEP_S`] sample of `path`: air at the sample's voxel and at the six
/// voxels [`CLEARANCE`] away along the axes. Returns the failures, `t eye` each.
fn rock_on(path: &CameraPath, material: impl Fn([i32; 3]) -> MaterialId) -> Vec<String> {
    let mut times: Vec<f32> = path.keys.iter().map(|k| k.t_s).collect();
    let steps = (path.duration_s() / AIR_STEP_S).ceil() as u32;
    times.extend((0..=steps).map(|s| (s as f32 * AIR_STEP_S).min(path.duration_s())));
    let mut bad = Vec::new();
    for t in times {
        let eye = path.sample(t).eye.map(|v| v.floor() as i32);
        let mut probes = vec![eye];
        for k in 0..3 {
            for s in [-CLEARANCE, CLEARANCE] {
                let mut p = eye;
                p[k] += s;
                probes.push(p);
            }
        }
        if let Some(p) = probes.iter().find(|&&p| material(p) != AIR) {
            bad.push(format!("{t:.2} s at {eye:?}: {p:?} is {}", material(*p)));
        }
    }
    bad
}

#[test]
fn the_showcase_path_flies_in_air() {
    let src = ShowcaseSource::new(PATH_SEED);
    let bad = rock_on(&SHOWCASE, |p| src.material(p));
    assert!(
        bad.is_empty(),
        "the showcase path meets rock:\n{}",
        bad.join("\n")
    );
}
