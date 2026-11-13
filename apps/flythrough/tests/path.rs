//! The camera paths (`flythrough::path`): the sampler's exactness at keys, its shape between
//! them and its shortest-arc yaw, and the headless guard that every path flies in air.

use std::cell::RefCell;
use std::collections::HashMap;

use flythrough::castle;
use flythrough::cave::CaveSource;
use flythrough::path::{
    CASTLE, CAVE, CameraPath, Controls, Drive, Keyframe, PATH_SEED, PATHS, PathMode, PathPlayer,
    SHOWCASE, Scene, TesterAction, TesterInput, key, tester_action,
};
use flythrough::report::{Recorder, RunReport, json};
use flythrough::showcase::ShowcaseSource;
use vxe_app::{KeyCode, MouseButton};
use vxe_world::{AIR, Chunk, ChunkCoord, MaterialId, VoxelCoord, WorldSource};

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

/// The streamed cave at the path's seed, voxel by voxel (its material is a pure function).
#[test]
fn the_cave_path_flies_in_air() {
    let src = CaveSource::unbounded(PATH_SEED);
    let bad = rock_on(&CAVE, |p| src.material(p));
    assert!(
        bad.is_empty(),
        "the cave path meets rock:\n{}",
        bad.join("\n")
    );
}

/// The castle the way `--castle` builds it over the cave of the path's seed, read chunk by
/// chunk through its stamped source.
#[test]
fn the_castle_path_flies_in_air() {
    let file = castle::castle_vox();
    let src = castle::castle_source(
        PATH_SEED,
        castle::castle_prefabs(&file).expect("every index is mapped"),
    );
    let chunks: RefCell<HashMap<ChunkCoord, Chunk>> = RefCell::default();
    let bad = rock_on(&CASTLE, |p| {
        let v = VoxelCoord(p);
        let mut chunks = chunks.borrow_mut();
        let c = chunks.entry(v.chunk()).or_insert_with(|| {
            let mut c = Chunk::default();
            src.chunk(v.chunk(), &mut c);
            c
        });
        c.brick(v.brick_slot()).get(v.index_in_brick())
    });
    assert!(
        bad.is_empty(),
        "the castle path meets stone:\n{}",
        bad.join("\n")
    );
}

/// Each path names its scene, and the castle's cut keys hold the leg's last pose until the cut.
#[test]
fn every_scene_has_one_path_and_a_cut_holds_the_pose_before_it() {
    for scene in Scene::ALL {
        let n = PATHS.iter().filter(|p| p.scene == scene).count();
        assert_eq!(n, 1, "{scene:?}");
        assert_eq!(
            CameraPath::named(scene.name()).map(|p| p.scene),
            Some(scene)
        );
    }
    let cut = CASTLE.keys.iter().position(|k| k.cut).expect("a cut");
    let before = CASTLE.keys[cut - 1];
    let mid = (before.t_s + CASTLE.keys[cut].t_s) / 2.0;
    assert_eq!(CASTLE.sample(mid).eye, before.eye);
    assert_eq!(
        CASTLE.sample(CASTLE.keys[cut].t_s).eye,
        CASTLE.keys[cut].eye
    );
}

const MOVE: Controls = Controls {
    moved: true,
    looked: false,
    dug: false,
    resume: false,
    escape: false,
};
const NONE: Controls = Controls {
    moved: false,
    looked: false,
    dug: false,
    resume: false,
    escape: false,
};

#[test]
fn a_move_in_a_tour_hands_over_at_the_pose_and_t_resumes_at_the_nearest_time() {
    let mut p = PathPlayer::new(&SHOWCASE, PathMode::Tour);
    for _ in 0..60 {
        assert!(p.step(1.0 / 60.0, NONE, [0.0; 3]).is_some());
    }
    let Drive::Path { t_s } = p.drive else {
        panic!("touring: {:?}", p.drive)
    };
    let at = SHOWCASE.sample(t_s);
    // The move takes the camera: no pose from the path, so it stays where the path left it.
    assert_eq!(p.step(1.0 / 60.0, MOVE, at.eye), None);
    assert_eq!(p.drive, Drive::Free);
    assert_eq!(p.step(1.0 / 60.0, NONE, at.eye), None);
    // The tester flew to the key at 13.8 s; T resumes there (the search's step is 50 ms).
    let near = SHOWCASE.sample(13.8).eye;
    let resume = Controls {
        resume: true,
        ..NONE
    };
    let pose = p.step(1.0 / 60.0, resume, near).expect("the tour resumes");
    let Drive::Path { t_s } = p.drive else {
        panic!("resumed: {:?}", p.drive)
    };
    assert!((t_s - 13.8).abs() <= 0.05, "resumed at {t_s} s");
    assert_eq!(pose, SHOWCASE.sample(t_s));
    assert_eq!(p.step(0.0, NONE, near), Some(SHOWCASE.sample(t_s)));
}

#[test]
fn a_tour_loops_and_esc_ends_it() {
    let mut p = PathPlayer::new(&SHOWCASE, PathMode::Tour);
    let end = SHOWCASE.duration_s();
    p.step(end - 0.5, NONE, [0.0; 3]);
    p.step(1.0, NONE, [0.0; 3]);
    let Drive::Path { t_s } = p.drive else {
        panic!("looping: {:?}", p.drive)
    };
    assert!((t_s - 0.5).abs() < 1e-3, "looped to {t_s}");
    let esc = Controls {
        escape: true,
        ..NONE
    };
    assert_eq!(p.step(0.0, esc, [0.0; 3]), None);
    assert_eq!(p.drive, Drive::Done);
    assert_eq!(p.ended_word(), "completed");
}

#[test]
fn esc_in_a_benchmark_aborts_it_and_its_end_completes_it() {
    let esc = Controls {
        escape: true,
        ..NONE
    };
    let mut p = PathPlayer::new(&SHOWCASE, PathMode::Bench);
    p.step(1.0, MOVE, [0.0; 3]);
    assert!(
        matches!(p.drive, Drive::Path { .. }),
        "a move does not stop a benchmark"
    );
    p.step(0.0, esc, [0.0; 3]);
    assert_eq!(p.drive, Drive::Aborted);
    assert!(p.ended());
    assert_eq!(p.ended_word(), "aborted");

    let mut p = PathPlayer::new(&SHOWCASE, PathMode::Bench);
    p.step(SHOWCASE.duration_s(), NONE, [0.0; 3]);
    assert_eq!(p.drive, Drive::Done);
    assert_eq!(p.ended_word(), "completed");
}

#[test]
fn the_tester_filter_leaves_movement_look_help_and_esc() {
    for k in [
        KeyCode::F2,
        KeyCode::F7,
        KeyCode::F10,
        KeyCode::KeyB,
        KeyCode::KeyQ,
        KeyCode::KeyV,
        KeyCode::KeyX,
        KeyCode::KeyZ,
        KeyCode::KeyG,
        KeyCode::KeyP,
        KeyCode::F9,
        KeyCode::Comma,
        KeyCode::Semicolon,
    ] {
        assert_eq!(tester_action(TesterInput::Key(k)), None, "{k:?}");
    }
    assert_eq!(tester_action(TesterInput::Click(MouseButton::Right)), None);
    assert_eq!(
        tester_action(TesterInput::Key(KeyCode::KeyW)),
        Some(TesterAction::Move)
    );
    assert_eq!(
        tester_action(TesterInput::MouseLook),
        Some(TesterAction::Look)
    );
    assert_eq!(
        tester_action(TesterInput::Key(KeyCode::F1)),
        Some(TesterAction::Help)
    );
    assert_eq!(
        tester_action(TesterInput::Key(KeyCode::Escape)),
        Some(TesterAction::End)
    );
    assert_eq!(
        tester_action(TesterInput::Key(KeyCode::KeyT)),
        Some(TesterAction::Resume)
    );
    assert_eq!(
        tester_action(TesterInput::Click(MouseButton::Left)),
        Some(TesterAction::Dig)
    );
}

/// The window's input reads into the player's controls: a held `W` is a move, mouse motion a
/// look, `Esc` the end.
#[test]
fn the_window_input_reads_into_controls() {
    let mut input = vxe_app::Input::default();
    assert_eq!(Controls::read(&input), NONE);
    input.key_down(KeyCode::KeyW);
    assert!(Controls::read(&input).moved);
    input.release_all();
    input.end_frame();
    input.add_mouse_delta(3.0, 0.0);
    assert!(Controls::read(&input).looked);
    input.end_frame();
    input.key_down(KeyCode::Escape);
    assert!(Controls::read(&input).escape);
}

/// A report written from a fixed series of syncs: 3 in the warm-up with every stage at 9000
/// us, then 100 whose `pack_us` runs 1 to 100 in a shuffled order (`(37 k mod 100) + 1`),
/// `size_us` 2 k + 2 for the same k, the other four stages 1 us each and `packed_bricks`
/// 10 k, `lock_us` k mod 7. `run.json`'s `streaming` holds the 100 measured syncs only: `pack_us` p50 50, p95 95,
/// p99 99, max 100; `size_us` p50 100 (the 50th of 2..=200 step 2); `sync_us` p50 154 (the
/// sum's 50th: 3 k + 4 of k = 50; the lock wait is inside the stages, not added);
/// `packed_bricks` p50 500 and max 1000; `lock_us` max 6. The fields past the six stages
/// (not in `sync_us`'s sum): `light_place_us` k mod 3 (max 2), `light_note_us` 3 k (p50 150),
/// `rescan_inserts` 7 k (p99 693, max 700), `clock_reads` 40 + k mod 5 (p50 42, max 44).
#[test]
fn run_json_carries_the_streaming_stages() {
    let mut rec = Recorder::for_path(10.0);
    let warm = vxe_render::SyncStats {
        select_us: 9000,
        size_us: 9000,
        pack_us: 9000,
        light_us: 9000,
        copy_us: 9000,
        alloc_us: 9000,
        packed_bricks: 9000,
        ..Default::default()
    };
    for i in 0..3 {
        rec.push_sync(i as f32 * 0.5, &warm);
    }
    for i in 0..100u64 {
        let k = (37 * i) % 100 + 1;
        let s = vxe_render::SyncStats {
            select_us: 1,
            size_us: 2 * k,
            pack_us: k,
            light_us: 1,
            copy_us: 1,
            alloc_us: 1,
            packed_bricks: 10 * k,
            lock_us: k % 7,
            light_place_us: k % 3,
            light_note_us: 3 * k,
            rescan_inserts: 7 * k,
            clock_reads: 40 + k % 5,
            ..Default::default()
        };
        rec.push_sync(3.0 + i as f32 * 0.01, &s);
    }
    let mut r = RunReport {
        version: "0.0.0",
        commit: "abc",
        scene: "cave",
        path: "cave",
        mode: "bench",
        output: [1280, 720],
        internal: [853, 480],
        internal_scale: "1.5",
        upscaler: "fsr2",
        adapter: None,
        path_s: 10.0,
        frames: 0,
        warmup_frames: 0,
        warmup_s: 0.0,
        wall_ms: None,
        cpu_ms: None,
        gpu_ms: None,
        over_33ms: 0,
        over_100ms: 0,
        peak_rss_bytes: None,
        streaming: None,
        ended: "completed",
    };
    rec.fill(&mut r);
    let v = json::parse(&r.to_json()).expect("run.json parses");
    let num = |path: &[&str]| -> f64 {
        let mut x = &v;
        for k in path {
            x = x.get(k).unwrap_or_else(|| panic!("no {path:?}"));
        }
        x.as_f64()
            .unwrap_or_else(|| panic!("{path:?} is not a number"))
    };
    assert_eq!(num(&["streaming", "frames"]), 100.0);
    assert_eq!(num(&["streaming", "pack_us", "p50"]), 50.0);
    assert_eq!(num(&["streaming", "pack_us", "p95"]), 95.0);
    assert_eq!(num(&["streaming", "pack_us", "p99"]), 99.0);
    assert_eq!(num(&["streaming", "pack_us", "max"]), 100.0);
    assert_eq!(num(&["streaming", "size_us", "p50"]), 100.0);
    assert_eq!(num(&["streaming", "select_us", "max"]), 1.0);
    assert_eq!(num(&["streaming", "light_us", "p99"]), 1.0);
    assert_eq!(num(&["streaming", "copy_us", "p50"]), 1.0);
    assert_eq!(num(&["streaming", "alloc_us", "max"]), 1.0);
    assert_eq!(num(&["streaming", "sync_us", "p50"]), 154.0);
    assert_eq!(num(&["streaming", "sync_us", "max"]), 304.0);
    assert_eq!(num(&["streaming", "packed_bricks", "p50"]), 500.0);
    assert_eq!(num(&["streaming", "packed_bricks", "max"]), 1000.0);
    assert_eq!(num(&["streaming", "lock_us", "max"]), 6.0);
    assert_eq!(num(&["streaming", "light_place_us", "max"]), 2.0);
    assert_eq!(num(&["streaming", "light_note_us", "p50"]), 150.0);
    assert_eq!(num(&["streaming", "rescan_inserts", "p99"]), 693.0);
    assert_eq!(num(&["streaming", "rescan_inserts", "max"]), 700.0);
    assert_eq!(num(&["streaming", "clock_reads", "p50"]), 42.0);
    assert_eq!(num(&["streaming", "clock_reads", "max"]), 44.0);
}
