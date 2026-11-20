//! The outdoor section's lighting flags: the penumbra, the key light's far source, the sky
//! term's normal and bounce, the exposure's ceiling.

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

/// Plan 0095 Phases 2 and 4: `--key-far` is baked with the blend unless named, which the banner
/// leaves out; `ray`, `column` and `--key-far-mix step` reach the lighting and the banner with a
/// sky set.
#[test]
fn key_far_is_baked_blend_unless_named() {
    use vxe_render::{KeyFar, KeyFarMix};
    let r = parse_args_from(argv(&["--source", "outdoor"])).unwrap();
    let l = r.start_lighting();
    assert_eq!(
        (l.key_far, l.key_far_mix),
        (KeyFar::Baked, KeyFarMix::Blend)
    );
    assert!(!r.start_modes(0.0).line().contains("key far"));
    let r = parse_args_from(argv(&["--source", "outdoor", "--key-far-mix", "step"])).unwrap();
    assert_eq!(r.start_lighting().key_far_mix, KeyFarMix::Step);
    assert!(
        r.start_modes(0.0)
            .line()
            .contains("key far baked, key far mix step, ")
    );
    let r = parse_args_from(argv(&["--source", "outdoor", "--key-far", "ray"])).unwrap();
    assert_eq!(r.start_lighting().key_far, KeyFar::Ray);
    assert!(r.start_modes(0.0).line().contains("key far ray, "));
    let r = parse_args_from(argv(&["--source", "outdoor", "--key-far", "column"])).unwrap();
    assert_eq!(r.start_lighting().key_far, KeyFar::Column);
    assert!(r.start_modes(0.0).line().contains("key far column, "));
    assert!(parse_args_from(argv(&["--key-far", "far"])).is_err());
    assert!(parse_args_from(argv(&["--key-far-mix", "fade"])).is_err());
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

/// `--exposure-rate` reaches the auto exposure, 3.75 EV/s by default and unnamed; another
/// value, 0 among them, is named `exposure rate N EV/s`; out of 0 to 64 it is refused.
#[test]
fn the_exposure_rate_reaches_the_run() {
    let r = parse_args_from(argv(&["--source", "outdoor"])).unwrap();
    assert_eq!(
        r.start_auto().rate_ev_s,
        vxe_render::DEFAULT_EXPOSURE_RATE_EV_S
    );
    assert_eq!(vxe_render::DEFAULT_EXPOSURE_RATE_EV_S, 3.75);
    assert!(!r.start_modes(0.0).line().contains("exposure rate"));
    let r = parse_args_from(argv(&["--source", "outdoor", "--exposure-rate", "3.75"])).unwrap();
    assert!(!r.start_modes(0.0).line().contains("exposure rate"));
    for (v, rate) in [("0", 0.0f32), ("1.875", 1.875), ("7.5", 7.5)] {
        let r = parse_args_from(argv(&["--source", "outdoor", "--exposure-rate", v])).unwrap();
        assert_eq!(r.start_auto().rate_ev_s, rate);
        let line = r.start_modes(0.0).line();
        assert!(
            line.contains(&format!("exposure rate {rate} EV/s, ")),
            "{line}"
        );
    }
    for bad in ["-1", "65", "fast"] {
        assert!(
            parse_args_from(argv(&["--exposure-rate", bad])).is_err(),
            "{bad}"
        );
    }
}

/// The light pass arms are built in the loading stage (`run::loading::ARMS`) when the flags
/// pick one at start, `--leaf-shadow absorb` or `--penumbra approach`, and not otherwise.
#[test]
fn an_arm_picked_at_start_is_built_while_loading() {
    let stages = |r: &crate::cli::Args| {
        crate::run::loading::load_stages(true, false, r.light_arms_at_start())
            .into_iter()
            .map(|(n, _)| n)
            .collect::<Vec<_>>()
    };
    let r = parse_args_from(argv(&["--source", "outdoor"])).unwrap();
    assert!(!r.light_arms_at_start());
    assert!(!stages(&r).contains(&crate::run::loading::ARMS));
    for picked in [
        &["--source", "outdoor", "--leaf-shadow", "absorb"][..],
        &["--source", "outdoor", "--penumbra", "approach"],
    ] {
        let r = parse_args_from(argv(picked)).unwrap();
        assert!(r.light_arms_at_start(), "{picked:?}");
        assert_eq!(stages(&r)[1], crate::run::loading::ARMS, "{picked:?}");
    }
    let r = parse_args_from(argv(&["--source", "outdoor", "--leaf-shadow", "voxel"])).unwrap();
    assert!(!r.light_arms_at_start());
}

/// The settle is gone (Plan 0094 Phase 7a, ADR-0076's Alternative F): `--settle` is refused
/// and the banner names no settle.
#[test]
fn the_settle_is_refused() {
    let r = parse_args_from(argv(&["--source", "outdoor"])).unwrap();
    assert!(!r.start_modes(0.0).line().contains("settle"));
    for v in ["on", "off"] {
        assert!(
            parse_args_from(argv(&["--source", "outdoor", "--settle", v])).is_err(),
            "{v}"
        );
    }
}

#[test]
fn the_pre_roll_drain_takes_seconds_and_its_banner_names_the_state() {
    use crate::run::queue::{Drain, write_drain_banner};
    let drain = |a: &[&str]| parse_args_from(argv(a)).map(|r| r.bake.pre_roll_drain);
    assert_eq!(drain(&["--source", "outdoor"]), Ok(None));
    assert_eq!(drain(&["--pre-roll-drain", "90"]), Ok(Some(90.0)));
    assert!(drain(&["--pre-roll-drain", "0"]).is_err());
    assert!(drain(&["--pre-roll-drain", "601"]).is_err());
    let stats = vxe_render::QueueStats {
        jobs: 12,
        coarse: 340,
        ..Default::default()
    };
    let row = |d: Drain, now: f64| {
        let mut out = String::new();
        write_drain_banner(&mut out, d, Some(&stats), 120.0, now);
        out
    };
    let draining = Drain::Draining {
        from_ms: 1000.0,
        clear_ms: None,
    };
    assert_eq!(
        row(draining, 31_000.0).trim(),
        "pre-roll: draining 12 jobs, 340 coarse chunks to refine, 90 s at most; HOLD STILL"
    );
    let done = Drain::Done {
        at_ms: 40_000.0,
        secs: 39.0,
        landed: true,
    };
    assert_eq!(
        row(done, 41_000.0).trim(),
        "pre-roll: landed after 39 s; move now"
    );
    assert_eq!(row(done, 40_000.0 + crate::hud::BANNER_MS), "");
    assert_eq!(row(Drain::Off, 0.0), "");
}
