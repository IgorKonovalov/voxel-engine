//! The outdoor section's lighting flags: the penumbra, the sky term's normal and bounce, the
//! exposure's ceiling.

use crate::cli::parse_args_from;
use crate::test_support::argv;
use vxe_render::Exposure;

/// `--penumbra`: disc unless named; approach reaches the lighting and the banner.
#[test]
fn penumbra_is_disc_unless_named() {
    // Disc by default and unnamed; approach reaches the lighting and the banner.
    let r = parse_args_from(argv(&["--source", "outdoor"])).unwrap();
    assert_eq!(r.start_lighting().penumbra, vxe_render::Penumbra::Disc);
    assert!(!r.start_modes(0.0).line().contains("penumbra"));
    let r = parse_args_from(argv(&["--source", "outdoor", "--penumbra", "approach"])).unwrap();
    assert_eq!(r.start_lighting().penumbra, vxe_render::Penumbra::Approach);
    assert!(r.start_modes(0.0).line().contains("penumbra approach, "));
    assert!(parse_args_from(argv(&["--penumbra", "soft"])).is_err());
}

/// `--sky-anchor` and `--exposure-max` reach the auto exposure; unset is the default anchor
/// 4, none the constant ceiling, and the banner names an anchor other than 4 (`none`
/// included) with a sky set, and nothing for the default.
#[test]
fn the_ceiling_flags_reach_the_auto_exposure() {
    let auto = |v: &[&str]| match parse_args_from(argv(v)).unwrap().start_lighting().exposure {
        Exposure::Auto(a) => a,
        Exposure::Manual(_) => panic!("auto"),
    };
    let a = auto(&["--source", "outdoor"]);
    assert_eq!((a.sky_anchor_ev, a.max), (Some(4.0), 4.0));
    let a = auto(&[
        "--source",
        "outdoor",
        "--sky-anchor",
        "4",
        "--exposure-max",
        "-1",
    ]);
    assert_eq!((a.sky_anchor_ev, a.max), (Some(4.0), 0.5));
    assert_eq!(
        auto(&["--source", "outdoor", "--sky-anchor", "none"]).sky_anchor_ev,
        None
    );
    let r = parse_args_from(argv(&["--source", "outdoor", "--sky-anchor", "2"])).unwrap();
    assert!(r.start_modes(0.0).line().contains("sky anchor 2, "));
    let r = parse_args_from(argv(&["--source", "outdoor", "--sky-anchor", "none"])).unwrap();
    assert!(r.start_modes(0.0).line().contains("sky anchor none, "));
    for v in [
        &["--source", "outdoor"][..],
        &["--source", "outdoor", "--sky-anchor", "4"],
    ] {
        let r = parse_args_from(argv(v)).unwrap();
        assert!(!r.start_modes(0.0).line().contains("sky anchor"), "{v:?}");
    }
    for bad in [
        &["--sky-anchor", "13"][..],
        &["--sky-anchor", "x"],
        &["--exposure-max", "9"],
    ] {
        assert!(parse_args_from(argv(bad)).is_err(), "{bad:?}");
    }
}

/// `--sky-normal` and `--sky-bounce` reach the lighting and the banner; the defaults,
/// smoothed and 0, are unnamed, and `face` is named.
#[test]
fn the_sky_term_flags_reach_the_lighting() {
    let r = parse_args_from(argv(&["--source", "outdoor"])).unwrap();
    let l = r.start_lighting();
    assert_eq!(
        (l.sky_normal, l.sky_bounce),
        (vxe_render::SkyNormal::Smoothed, 0.0)
    );
    assert!(!r.start_modes(0.0).line().contains("sky normal"));
    assert!(!r.start_modes(0.0).line().contains("sky bounce"));
    let r = parse_args_from(argv(&[
        "--source",
        "outdoor",
        "--sky-normal",
        "face",
        "--sky-bounce",
        "0.3",
    ]))
    .unwrap();
    let l = r.start_lighting();
    assert_eq!(
        (l.sky_normal, l.sky_bounce),
        (vxe_render::SkyNormal::Face, 0.3)
    );
    let line = r.start_modes(0.0).line();
    assert!(line.contains("sky normal face, sky bounce 0.3, "), "{line}");
    for bad in [&["--sky-normal", "bent"][..], &["--sky-bounce", "1.5"]] {
        assert!(parse_args_from(argv(bad)).is_err(), "{bad:?}");
    }
}

/// `--column-wait` reaches the lighting, off by default and unnamed; on is named. `--dump-black`
/// is off by default, named when on, and turns the store's counters on.
#[test]
fn the_column_wait_and_the_dump_reach_the_run() {
    let r = parse_args_from(argv(&["--source", "outdoor"])).unwrap();
    assert!(!r.start_lighting().column_wait);
    assert!(!r.outdoor.dump_black);
    let line = r.start_modes(0.0).line();
    assert!(
        !line.contains("column wait") && !line.contains("dump black"),
        "{line}"
    );
    let r = parse_args_from(argv(&[
        "--source",
        "outdoor",
        "--column-wait",
        "on",
        "--dump-black",
    ]))
    .unwrap();
    assert!(r.start_lighting().column_wait);
    assert!(r.outdoor.dump_black);
    let line = r.start_modes(0.0).line();
    assert!(line.contains("column wait on, dump black, "), "{line}");
    let r = parse_args_from(argv(&["--source", "outdoor", "--column-wait", "off"])).unwrap();
    assert!(!r.start_lighting().column_wait);
    assert!(parse_args_from(argv(&["--column-wait", "maybe"])).is_err());
}

/// `--settle` reaches the lighting, off by default and unnamed; on is named `settle on`.
#[test]
fn the_settle_reaches_the_run() {
    let r = parse_args_from(argv(&["--source", "outdoor"])).unwrap();
    assert!(!r.start_lighting().settle);
    assert!(!r.start_modes(0.0).line().contains("settle"));
    let r = parse_args_from(argv(&["--source", "outdoor", "--settle", "off"])).unwrap();
    assert!(!r.start_lighting().settle);
    assert!(!r.start_modes(0.0).line().contains("settle"));
    let r = parse_args_from(argv(&["--source", "outdoor", "--settle", "on"])).unwrap();
    assert!(r.start_lighting().settle);
    let line = r.start_modes(0.0).line();
    assert!(line.contains("settle on, "), "{line}");
    assert!(parse_args_from(argv(&["--settle", "maybe"])).is_err());
}
