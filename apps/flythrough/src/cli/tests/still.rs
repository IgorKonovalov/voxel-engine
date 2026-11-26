//! `--still-at`, `--still-dir` and `--eye`: the frames a run writes, where, under what name,
//! and the camera's start on every source.

use crate::cli::parse_args_from;
use crate::run::still::still_stem;
use crate::setup::start_camera;
use crate::test_support::argv;
use std::path::Path;
use svet_world::VoxelCoord;

/// `--still-at` takes comma-separated frames and `--still-dir` a directory; unset, the
/// directory is `target/stills`; a word that is not a frame is refused.
#[test]
fn still_at_and_still_dir_parse() {
    let a = parse_args_from(argv(&["--still-at", "1,63", "--still-dir", "out/s"])).unwrap();
    assert_eq!(a.still.at, vec![1, 63]);
    assert_eq!(a.still.dir(), Path::new("out/s"));
    let a = parse_args_from(argv(&["--still-at", "5"])).unwrap();
    assert_eq!(a.still.at, vec![5]);
    assert_eq!(a.still.dir(), Path::new("target/stills"));
    assert!(!a.still.any(), "--still-at needs no --headless or --frames");
    let a = parse_args_from(argv(&[])).unwrap();
    assert!(a.still.at.is_empty() && a.still.dir.is_none());
    assert!(parse_args_from(argv(&["--still-at", "1,x"])).is_err());
    assert!(parse_args_from(argv(&["--still-at"])).is_err());
    let help = crate::cli::help();
    assert!(help.contains("--still-at F,..") && help.contains("--still-dir DIR"));
    assert!(help.contains("--eye X,Y,Z,YAW,PITCH"));
}

/// Under `--frames 64` the run draws frames 0 to 63: `--still-at 63` parses, `--still-at 64`
/// (alone or after a frame in range) is refused naming both flags; `--still-dir` without
/// `--still-at` is refused.
#[test]
fn a_still_past_the_last_frame_is_refused() {
    let ok = [
        "--headless",
        "--source",
        "showcase",
        "--frames",
        "64",
        "--still-at",
        "63",
    ];
    assert_eq!(parse_args_from(argv(&ok)).unwrap().still.at, vec![63]);
    for at in ["64", "10,64"] {
        let e = parse_args_from(argv(&["--frames", "64", "--still-at", at]))
            .err()
            .expect("refused");
        assert_eq!(
            e,
            "--still-at 64 is past the run's last frame: --frames 64 draws frames 0 to 63"
        );
    }
    let e = parse_args_from(argv(&["--still-dir", "d"]))
        .err()
        .expect("refused");
    assert_eq!(
        e,
        "--still-dir places --still-at's files; it needs --still-at"
    );
}

/// `--eye 100,100,40,90,-10` starts the camera at that point, facing 90 degrees and 10 down,
/// on the showcase (in place of its shot), the cave, the castle (in place of its shot) and
/// the room; unset, the showcase starts at its shot.
#[test]
fn eye_places_the_camera_on_every_source() {
    let eye = ["--eye", "100,100,40,90,-10"];
    for source in [
        &["--source", "showcase"][..],
        &["--source", "cave"],
        &["--castle", "--castle-shot", "nave"],
        &["--room", "4"],
        &["--level"],
    ] {
        let mut v = source.to_vec();
        v.extend_from_slice(&eye);
        let a = parse_args_from(argv(&v)).unwrap_or_else(|e| panic!("{v:?}: {e}"));
        assert_eq!(a.eye, Some([100.0, 100.0, 40.0, 90.0, -10.0]), "{v:?}");
        let cam = start_camera(&a, VoxelCoord([0, 0, 0]));
        assert_eq!(
            <[f32; 3]>::from(cam.position),
            [100.0, 100.0, 40.0],
            "{v:?}"
        );
        assert_eq!(cam.yaw, 90f32.to_radians(), "{v:?}");
        assert_eq!(cam.pitch, (-10f32).to_radians(), "{v:?}");
    }
    let a = parse_args_from(argv(&["--source", "showcase"])).unwrap();
    let cam = start_camera(&a, VoxelCoord([0, 0, 0]));
    assert_eq!(cam.position, crate::setup::shot_camera(&a.shot).position);
}

/// `--path` drives the camera: with `--eye` it is refused naming both.
#[test]
fn eye_with_a_path_is_refused() {
    for v in [
        &["--source", "cave", "--path", "cave", "--eye", "1,2,3,4,5"][..],
        &[
            "--source",
            "showcase",
            "--path",
            "showcase",
            "--eye",
            "1,2,3,4,5",
        ],
    ] {
        assert_eq!(
            parse_args_from(argv(v)).err().as_deref(),
            Some("--path drives the camera; it does not take --eye"),
            "{v:?}"
        );
    }
}

/// The still's file stem names the scene: the showcase's shot, the outdoor start's clock, the
/// castle's shot, the cave.
#[test]
fn the_still_stem_names_the_scene() {
    let stem = |v: &[&str]| still_stem(&parse_args_from(argv(v)).unwrap());
    assert_eq!(stem(&["--source", "showcase"]), "showcase-hall");
    assert_eq!(
        stem(&["--source", "showcase", "--shot", "chasm"]),
        "showcase-chasm"
    );
    assert_eq!(
        stem(&["--source", "outdoor", "--time", "15:00"]),
        "outdoor-15-00"
    );
    assert_eq!(
        stem(&["--source", "outdoor", "--time", "07:05"]),
        "outdoor-07-05"
    );
    assert_eq!(stem(&["--castle", "--castle-shot", "nave"]), "castle-nave");
    assert_eq!(stem(&["--source", "cave"]), "cave");
    assert_eq!(stem(&["--room", "4"]), "room-40", "the edge in voxels");
}

/// A source switch drops `--eye` (the first source's pose) and counts `--still-at`'s frames on
/// from the frames drawn, as `--switch-at`'s are.
#[test]
fn a_switch_drops_the_eye_and_counts_the_stills_on() {
    let a = parse_args_from(argv(&[
        "--source",
        "cave",
        "--eye",
        "1,2,3,4,5",
        "--still-at",
        "5,20,40",
    ]))
    .unwrap();
    let next = crate::run::switch::switched_args(&a, 20).expect("the next source parses");
    assert_eq!(next.eye, None);
    assert_eq!(next.still.at, vec![0, 20]);
}
