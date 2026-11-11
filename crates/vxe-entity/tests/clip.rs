//! Clips sampled on computed keys: the translation, the rotation blend and its shorter arc, the
//! loop and the hold, the blend of two poses, `Clip::new`'s refusals and the playback tick.

use std::sync::Arc;

use vxe_entity::{
    Clip, ClipError, Key, Model, Part, PartPose, Playback, Pose, Quat, Track, blend, sample,
};
use vxe_world::Prefab;

/// A two-part model: a root cube and a child.
fn model() -> Model {
    let cube = Arc::new(Prefab::new([2; 3], vec![1; 8], None));
    let part = |parent| Part {
        volume: cube.clone(),
        parent,
        pivot: [256; 3],
        detail: 1,
    };
    Model::new(vec![part(None), part(Some(0))]).unwrap()
}

fn key(tick: u32, t: [i32; 3], q: Quat) -> Key {
    Key { tick, t, q }
}

fn clip(model: &Model, length: u32, looped: bool, keys: Vec<Key>) -> Clip {
    Clip::new(model, length, looped, vec![Track { part: 1, keys }]).unwrap()
}

fn within_one(got: Quat, want: [i16; 4]) {
    for k in 0..4 {
        assert!(
            (i32::from(got.0[k]) - i32::from(want[k])).abs() <= 1,
            "{got:?}, want {want:?} within 1"
        );
    }
}

const X180: Quat = Quat([0, 16384, 0, 0]);

#[test]
fn a_translation_key_pair_is_linear() {
    let m = model();
    let rest = Pose::rest(&m);
    let c = clip(
        &m,
        40,
        false,
        vec![
            key(0, [0; 3], Quat::IDENTITY),
            key(40, [256, 0, 0], Quat::IDENTITY),
        ],
    );
    assert_eq!(sample(&m, &rest, &c, 10).parts[1].t, [64, 0, 0]);
    assert_eq!(sample(&m, &rest, &c, 20).parts[1].t, [128, 0, 0]);
    // A part with no track keeps its rest pose, and the key adds to the rest translation.
    let mut rest = Pose::rest(&m);
    rest.parts[0].t = [5, 6, 7];
    rest.parts[1].t = [1000, 0, -3];
    let p = sample(&m, &rest, &c, 10);
    assert_eq!(p.parts[0], rest.parts[0]);
    assert_eq!(p.parts[1].t, [1064, 0, -3]);
    // Rounding toward zero: 1/3 of -256 is -85.33, read as -85; of 256, 85.
    let c = clip(
        &m,
        3,
        false,
        vec![
            key(0, [0; 3], Quat::IDENTITY),
            key(3, [-256, 256, 0], Quat::IDENTITY),
        ],
    );
    assert_eq!(sample(&m, &Pose::rest(&m), &c, 1).parts[1].t, [-85, 85, 0]);
}

#[test]
fn a_half_turn_key_pair_passes_through_a_quarter_turn() {
    let m = model();
    let rest = Pose::rest(&m);
    let c = clip(
        &m,
        40,
        false,
        vec![key(0, [0; 3], Quat::IDENTITY), key(40, [0; 3], X180)],
    );
    let mid = sample(&m, &rest, &c, 20).parts[1].q;
    within_one(mid, [11585, 11585, 0, 0]);
    // (0.75, 0.25) normalised: 36.9 degrees, the blend's uneven speed.
    let quarter = sample(&m, &rest, &c, 10).parts[1].q;
    within_one(quarter, [15543, 5181, 0, 0]);
    println!("tick 20 {mid:?}, tick 10 {quarter:?}");
}

#[test]
fn the_shorter_arc_is_taken() {
    let m = model();
    let rest = Pose::rest(&m);
    // 60 degrees about y, then a key of 100 degrees about y written as its negative (dot < 0).
    let a = Quat([14189, 0, 8192, 0]);
    let b = Quat([10531, 0, 12551, 0]);
    let neg_b = Quat(b.0.map(|c| -c));
    let dot: i32 = (0..4)
        .map(|k| i32::from(a.0[k]) * i32::from(neg_b.0[k]))
        .sum();
    assert!(dot < 0);
    let with_neg = clip(
        &m,
        40,
        false,
        vec![key(0, [0; 3], a), key(40, [0; 3], neg_b)],
    );
    let with_pos = clip(&m, 40, false, vec![key(0, [0; 3], a), key(40, [0; 3], b)]);
    let q_neg = sample(&m, &rest, &with_neg, 20).parts[1].q;
    let q_pos = sample(&m, &rest, &with_pos, 20).parts[1].q;
    assert_eq!(q_neg, q_pos);
    // 80 degrees about y: cos 40 and sin 40 in Q14.
    within_one(q_pos, [12551, 0, 10531, 0]);
}

#[test]
fn a_loop_wraps_and_a_one_shot_holds() {
    let m = model();
    let rest = Pose::rest(&m);
    let keys = vec![
        key(0, [0; 3], Quat::IDENTITY),
        key(20, [512, 0, 0], X180),
        key(40, [0, 256, 0], Quat::IDENTITY),
    ];
    let looped = clip(&m, 40, true, keys.clone());
    let once = clip(&m, 40, false, keys);
    assert_eq!(
        sample(&m, &rest, &looped, 50),
        sample(&m, &rest, &looped, 10)
    );
    assert_ne!(
        sample(&m, &rest, &looped, 10),
        sample(&m, &rest, &looped, 40)
    );
    assert_eq!(sample(&m, &rest, &once, 50), sample(&m, &rest, &once, 40));
    assert_eq!(sample(&m, &rest, &once, 40).parts[1].t, [0, 256, 0]);
    // Before the first key and after the last, the nearest key holds.
    let late = clip(
        &m,
        40,
        false,
        vec![key(10, [7, 0, 0], X180), key(30, [9, 0, 0], X180)],
    );
    assert_eq!(sample(&m, &rest, &late, 3).parts[1].t, [7, 0, 0]);
    assert_eq!(sample(&m, &rest, &late, 35).parts[1].t, [9, 0, 0]);
    // The root is left at the origin, unturned.
    let p = sample(&m, &rest, &late, 3);
    assert_eq!((p.root.0, p.root_q), ([0; 3], Quat::IDENTITY));
}

#[test]
fn blend_at_half_is_the_midpoint() {
    let m = model();
    let a = Pose::rest(&m);
    let mut b = Pose::rest(&m);
    b.parts[1] = PartPose {
        t: [256, 0, 0],
        q: Quat([14189, 0, 8192, 0]),
    };
    b.root.0 = [512, -256, 0];
    let half = blend(&a, &b, 128);
    assert_eq!(half.parts[1].t, [128, 0, 0]);
    assert_eq!(half.root.0, [256, -128, 0]);
    // 30 degrees about y.
    within_one(half.parts[1].q, [15826, 0, 4240, 0]);
    assert_eq!(blend(&a, &b, 0), a);
    assert_eq!(blend(&a, &b, 256), b);
}

#[test]
fn clip_new_refuses_unsorted_past_the_length_and_unknown_parts() {
    let m = model();
    let k = |tick| key(tick, [0; 3], Quat::IDENTITY);
    let track = |part, keys| Track { part, keys };
    assert_eq!(
        Clip::new(&m, 40, false, vec![track(1, vec![k(0), k(20), k(10)])]),
        Err(ClipError::UnsortedTrack { part: 1, key: 2 })
    );
    assert_eq!(
        Clip::new(&m, 40, false, vec![track(1, vec![k(0), k(41)])]),
        Err(ClipError::KeyPastLength {
            part: 1,
            tick: 41,
            length: 40
        })
    );
    assert_eq!(
        Clip::new(&m, 40, false, vec![track(2, vec![k(0)])]),
        Err(ClipError::UnknownPart { part: 2, parts: 2 })
    );
    // Equal ticks are unsorted too; the other refusals.
    assert_eq!(
        Clip::new(&m, 40, false, vec![track(0, vec![k(5), k(5)])]),
        Err(ClipError::UnsortedTrack { part: 0, key: 1 })
    );
    assert_eq!(Clip::new(&m, 0, false, vec![]), Err(ClipError::ZeroLength));
    assert_eq!(
        Clip::new(&m, 40, false, vec![track(0, vec![])]),
        Err(ClipError::EmptyTrack { part: 0 })
    );
    assert_eq!(
        Clip::new(
            &m,
            40,
            false,
            vec![track(1, vec![k(0)]), track(1, vec![k(1)])]
        ),
        Err(ClipError::DuplicateTrack { part: 1 })
    );
}

#[test]
fn playback_ticks_are_integer() {
    let m = model();
    let c = Arc::new(clip(&m, 40, true, vec![key(0, [0; 3], Quat::IDENTITY)]));
    let p = Playback {
        clip: c,
        start_tick: 100,
        rate: 384,
    };
    assert_eq!(p.tick_at(90), 0);
    assert_eq!(p.tick_at(100), 0);
    assert_eq!(p.tick_at(101), 1);
    assert_eq!(p.tick_at(102), 3);
    assert_eq!(p.tick_at(110), 15);
    assert_eq!(p.pose(&m, &Pose::rest(&m), 110).parts.len(), 2);
}
