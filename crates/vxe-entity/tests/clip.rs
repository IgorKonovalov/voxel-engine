//! Clips sampled on computed keys: the translation, the rotation blend and its shorter arc, the
//! loop and the hold, the blend of two poses, `Clip::new`'s refusals, the playback tick and
//! the player's fade from one clip into the next.

use std::sync::Arc;

use vxe_entity::{
    Clip, ClipError, Key, Model, Part, PartPose, Playback, Player, Pose, Quat, Track, blend, sample,
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

/// Two clips for the fade tests: `a` slides part 1 ten voxels along x over 40 ticks; `b` holds
/// it a quarter turn about z, four voxels along y, sliding one voxel along z over 40 ticks.
fn two_clips(m: &Model) -> (Arc<Clip>, Arc<Clip>) {
    let a = clip(
        m,
        40,
        false,
        vec![
            key(0, [0; 3], Quat::IDENTITY),
            key(40, [2560, 0, 0], Quat::IDENTITY),
        ],
    );
    let z90 = Quat([11585, 0, 0, 11585]);
    let b = clip(
        m,
        40,
        false,
        vec![key(0, [0, 1024, 0], z90), key(40, [0, 1024, 256], z90)],
    );
    (Arc::new(a), Arc::new(b))
}

/// A switch at tick 20 with an 8-tick fade: at the switch the pose is the old clip's there;
/// from tick 28 it is the new clip's from its tick 0 (tick 8 and 20 of it); in between it is
/// `blend` of the held pose and the new clip's at `(now - 20) * 256 / 8`.
#[test]
fn a_switch_fades_from_the_old_pose_into_the_new_clip() {
    let m = model();
    let rest = Pose::rest(&m);
    let (a, b) = two_clips(&m);
    let mut p = Player::new(Playback {
        clip: a.clone(),
        start_tick: 0,
        rate: 256,
    });
    let old = p.pose(&m, &rest, 20);
    assert_eq!(old, sample(&m, &rest, &a, 20));
    assert_eq!(old.parts[1].t, [1280, 0, 0]);
    p.switch(&m, &rest, 20, b.clone(), 8);
    assert_eq!(p.pose(&m, &rest, 20), old, "no jump at the switch");
    let fade = p.fade.as_ref().expect("fading");
    assert_eq!(
        [20, 21, 22, 24, 27, 28, 99].map(|t| fade.weight(t)),
        [0, 32, 64, 128, 224, 256, 256]
    );
    for now in 21..28 {
        let want = blend(&old, &sample(&m, &rest, &b, now - 20), fade.weight(now));
        assert_eq!(p.pose(&m, &rest, now), want, "tick {now}");
    }
    let mid = p.pose(&m, &rest, 24);
    // Half of 1280 along x, half of 1024 along y and half of 4 ticks' 6.4 along z.
    assert_eq!(mid.parts[1].t, [640, 512, 12]);
    assert_eq!(p.pose(&m, &rest, 28), sample(&m, &rest, &b, 8));
    assert_eq!(p.pose(&m, &rest, 40), sample(&m, &rest, &b, 20));
    assert_eq!(p.playback.rate, 256, "the rate is kept");
}

/// A switch at tick 24, half way through a fade begun at 20, fades from the blend shown at 24
/// (not from either clip), into the new clip from its tick 0: no jump at the switch, and the
/// steps after it are the blend's.
#[test]
fn a_switch_during_a_fade_starts_from_the_blend_shown() {
    let m = model();
    let rest = Pose::rest(&m);
    let (a, b) = two_clips(&m);
    let mut p = Player::new(Playback {
        clip: a.clone(),
        start_tick: 0,
        rate: 256,
    });
    p.switch(&m, &rest, 20, b.clone(), 8);
    let shown = p.pose(&m, &rest, 24);
    assert_ne!(shown, sample(&m, &rest, &a, 24));
    assert_ne!(shown, sample(&m, &rest, &b, 4));
    p.switch(&m, &rest, 24, a.clone(), 8);
    assert_eq!(p.pose(&m, &rest, 24), shown, "no jump at the second switch");
    assert_eq!(
        p.pose(&m, &rest, 25),
        blend(&shown, &sample(&m, &rest, &a, 1), 32)
    );
    // One tick on, part 1 moves an eighth of the way from the blend ([640, 512, 12]) to
    // `a`'s tick 1 ([64, 0, 0]): 640 - 72 along x, 512 - 64 along y, 12 * 7 / 8 = 10.5
    // rounded toward zero along z.
    assert_eq!(p.pose(&m, &rest, 25).parts[1].t, [568, 448, 10]);
    assert_eq!(p.pose(&m, &rest, 32), sample(&m, &rest, &a, 8));
}

/// A fade of 0 ticks switches at once; `switch_from` fades from the pose it is given.
#[test]
fn a_zero_tick_fade_switches_at_once() {
    let m = model();
    let rest = Pose::rest(&m);
    let (a, b) = two_clips(&m);
    let mut p = Player::new(Playback {
        clip: a,
        start_tick: 0,
        rate: 256,
    });
    p.switch(&m, &rest, 10, b.clone(), 0);
    assert_eq!(p.fade, None);
    assert_eq!(p.pose(&m, &rest, 10), sample(&m, &rest, &b, 0));
    p.switch_from(rest.clone(), 12, b.clone(), 4);
    assert_eq!(p.pose(&m, &rest, 12), rest);
    assert_eq!(p.pose(&m, &rest, 16), sample(&m, &rest, &b, 4));
}
