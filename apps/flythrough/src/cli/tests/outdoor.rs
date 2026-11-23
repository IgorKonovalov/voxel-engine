//! The outdoor section's lighting flags: the penumbra, the key light's far source, `--calm`,
//! the sky term's normal and bounce, the exposure's ceiling.

use crate::cli::parse_args_from;
use crate::test_support::argv;
use vxe_render::Exposure;

/// `--penumbra`: approach unless named (Plan 0095 Phase 5, the fly-through's
/// `DEFAULT_PENUMBRA`), which the banner leaves out; disc reaches the lighting and the banner.
#[test]
fn penumbra_is_approach_unless_named() {
    let r = parse_args_from(argv(&["--source", "outdoor"])).unwrap();
    assert_eq!(r.start_lighting().penumbra, vxe_render::Penumbra::Approach);
    assert!(!r.start_modes(0.0).line().contains("penumbra"));
    let r = parse_args_from(argv(&["--source", "outdoor", "--penumbra", "disc"])).unwrap();
    assert_eq!(r.start_lighting().penumbra, vxe_render::Penumbra::Disc);
    assert!(r.start_modes(0.0).line().contains("penumbra disc, "));
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
/// pick one at start, `--leaf-shadow absorb`, and not otherwise: the approach penumbra (the
/// default outdoors) is in the default light pipelines.
#[test]
fn an_arm_picked_at_start_is_built_while_loading() {
    let stages = |r: &crate::cli::Args| {
        crate::run::loading::load_stages(true, false, r.light_arms_at_start())
            .into_iter()
            .map(|(n, _)| n)
            .collect::<Vec<_>>()
    };
    let r = parse_args_from(argv(&["--source", "outdoor", "--penumbra", "disc"])).unwrap();
    assert!(!r.light_arms_at_start());
    assert!(!stages(&r).contains(&crate::run::loading::ARMS));
    let r = parse_args_from(argv(&["--source", "outdoor", "--sky", "off"])).unwrap();
    assert!(!r.light_arms_at_start());
    for unpicked in [
        &["--source", "outdoor", "--penumbra", "approach"][..],
        &["--source", "outdoor"],
    ] {
        let r = parse_args_from(argv(unpicked)).unwrap();
        assert!(!r.light_arms_at_start(), "{unpicked:?}");
    }
    for picked in [
        &[
            "--source",
            "outdoor",
            "--penumbra",
            "disc",
            "--leaf-shadow",
            "absorb",
        ][..],
        &["--source", "outdoor", "--leaf-shadow", "absorb"],
    ] {
        let r = parse_args_from(argv(picked)).unwrap();
        assert!(r.light_arms_at_start(), "{picked:?}");
        assert_eq!(stages(&r)[1], crate::run::loading::ARMS, "{picked:?}");
    }
    let r = parse_args_from(argv(&[
        "--source",
        "outdoor",
        "--penumbra",
        "disc",
        "--leaf-shadow",
        "voxel",
    ]))
    .unwrap();
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

/// `--sky-anchor-floor` reaches the auto exposure, 1/8 by default and unnamed; another value is
/// named `sky anchor floor X` and `off` (0) `sky anchor floor off` with a sky set; out of (0,
/// 1] it is refused; `--check-args` accepts it.
#[test]
fn the_sky_anchor_floor_reaches_the_run() {
    let r = parse_args_from(argv(&["--source", "outdoor"])).unwrap();
    assert_eq!(r.start_auto().sky_anchor_floor, 0.125);
    assert_eq!(
        r.start_auto().sky_anchor_floor,
        vxe_render::DEFAULT_SKY_ANCHOR_FLOOR
    );
    assert!(!r.start_modes(0.0).line().contains("sky anchor floor"));
    for (v, floor, word) in [
        ("0.25", 0.25f32, "sky anchor floor 0.25, "),
        ("off", 0.0, "sky anchor floor off, "),
    ] {
        let r = parse_args_from(argv(&["--source", "outdoor", "--sky-anchor-floor", v])).unwrap();
        assert_eq!(r.start_auto().sky_anchor_floor, floor);
        let line = r.start_modes(0.0).line();
        assert!(line.contains(word), "{line}");
    }
    let r = parse_args_from(argv(&[
        "--source",
        "outdoor",
        "--sky-anchor-floor",
        "0.125",
    ]))
    .unwrap();
    assert!(!r.start_modes(0.0).line().contains("sky anchor floor"));
    for bad in ["0", "-0.5", "1.5", "most"] {
        assert!(
            parse_args_from(argv(&["--sky-anchor-floor", bad])).is_err(),
            "{bad}"
        );
    }
    assert!(parse_args_from(argv(&["--check-args", "--sky-anchor-floor", "off"])).is_ok());
}

/// `--sky-anchor-lift` reaches the auto exposure, 4 by default and unnamed; another value is
/// named `sky anchor lift L` and `off` (`None`) `sky anchor lift off` with a sky set; out of
/// [0, 12] it is refused; `--check-args` accepts `4` and `off`.
#[test]
fn the_sky_anchor_lift_reaches_the_run() {
    let r = parse_args_from(argv(&["--source", "outdoor"])).unwrap();
    assert_eq!(r.start_auto().sky_anchor_lift, Some(4.0));
    assert_eq!(
        r.start_auto().sky_anchor_lift,
        Some(vxe_render::DEFAULT_SKY_ANCHOR_LIFT)
    );
    assert!(!r.start_modes(0.0).line().contains("sky anchor lift"));
    for (v, lift, word) in [
        ("6", Some(6.0f32), "sky anchor lift 6, "),
        ("2", Some(2.0), "sky anchor lift 2, "),
        ("off", None, "sky anchor lift off, "),
    ] {
        let r = parse_args_from(argv(&["--source", "outdoor", "--sky-anchor-lift", v])).unwrap();
        assert_eq!(r.start_auto().sky_anchor_lift, lift);
        let line = r.start_modes(0.0).line();
        assert!(line.contains(word), "{line}");
    }
    let r = parse_args_from(argv(&["--source", "outdoor", "--sky-anchor-lift", "4"])).unwrap();
    assert!(!r.start_modes(0.0).line().contains("sky anchor lift"));
    for bad in ["-1", "12.5", "most"] {
        assert!(
            parse_args_from(argv(&["--sky-anchor-lift", bad])).is_err(),
            "{bad}"
        );
    }
    for ok in ["4", "off"] {
        assert!(parse_args_from(argv(&["--check-args", "--sky-anchor-lift", ok])).is_ok());
    }
}

/// `--exposure-highlight` reaches the auto exposure, 99 by default and unnamed; another value
/// is named `exposure highlight P` and `off` (0) `exposure highlight off`; out of [50, 100] it
/// is refused; `--check-args` accepts it.
#[test]
fn the_exposure_highlight_reaches_the_run() {
    let r = parse_args_from(argv(&["--source", "outdoor"])).unwrap();
    assert_eq!(r.start_auto().highlight_percentile, 99.0);
    assert_eq!(
        r.start_auto().highlight_percentile,
        vxe_render::DEFAULT_EXPOSURE_HIGHLIGHT
    );
    assert!(!r.start_modes(0.0).line().contains("exposure highlight"));
    for (v, p, word) in [
        ("97.5", 97.5f32, "exposure highlight 97.5, "),
        ("off", 0.0, "exposure highlight off, "),
    ] {
        let r = parse_args_from(argv(&["--source", "outdoor", "--exposure-highlight", v])).unwrap();
        assert_eq!(r.start_auto().highlight_percentile, p);
        let line = r.start_modes(0.0).line();
        assert!(line.contains(word), "{line}");
    }
    for bad in ["0", "49", "100.5", "most"] {
        assert!(
            parse_args_from(argv(&["--exposure-highlight", bad])).is_err(),
            "{bad}"
        );
    }
    assert!(parse_args_from(argv(&["--check-args", "--exposure-highlight", "off"])).is_ok());
}

/// `--sky-anchor-share` reaches the auto exposure, 1/256 by default and unnamed; another value is
/// named `sky anchor share X` and `off` (0) `sky anchor share off` with a sky set; out of (0, 1]
/// it is refused.
#[test]
fn the_sky_anchor_share_reaches_the_run() {
    let r = parse_args_from(argv(&["--source", "outdoor"])).unwrap();
    assert_eq!(r.start_auto().sky_anchor_share, 1.0 / 256.0);
    assert_eq!(
        r.start_auto().sky_anchor_share,
        vxe_render::DEFAULT_SKY_ANCHOR_SHARE
    );
    assert!(!r.start_modes(0.0).line().contains("sky anchor share"));
    for (v, share, word) in [
        ("0.25", 0.25f32, "sky anchor share 0.25, "),
        ("off", 0.0, "sky anchor share off, "),
    ] {
        let r = parse_args_from(argv(&["--source", "outdoor", "--sky-anchor-share", v])).unwrap();
        assert_eq!(r.start_auto().sky_anchor_share, share);
        let line = r.start_modes(0.0).line();
        assert!(line.contains(word), "{line}");
    }
    let r = parse_args_from(argv(&[
        "--source",
        "outdoor",
        "--sky-anchor-share",
        "0.00390625",
    ]))
    .unwrap();
    assert!(!r.start_modes(0.0).line().contains("sky anchor share"));
    for bad in ["0", "-0.5", "1.5", "most"] {
        assert!(
            parse_args_from(argv(&["--sky-anchor-share", bad])).is_err(),
            "{bad}"
        );
    }
}

/// Plan 0098 Phase 6 (Phase 1's test, its four rows become window W1's five picks): `--calm`
/// sets exactly light history off, key far column, key far mix step, the disc penumbra and far
/// normal 4..8, and nothing else of the start's lighting; a later flag of the five overrides
/// its part (`--calm --far-normal 2,6` keeps 2..6 and is not calm, `--calm --penumbra approach`
/// is not calm), an earlier one is overridden. With all five the lighting row reads `light
/// history off` and the banner names `calm`; the default names neither. `--landing-fade`
/// reaches the lighting and the banner, and `--calm` leaves it as it is.
#[test]
fn calm_sets_the_five_picks_and_a_later_flag_wins() {
    use crate::hud::HudRow;
    use vxe_render::{KeyFar, KeyFarMix, Penumbra};
    let base = parse_args_from(argv(&["--source", "outdoor"])).unwrap();
    assert!(base.start_lighting().light_history);
    assert!(!base.start_lighting().landing_fade);
    let m = base.start_modes(0.0);
    assert!(!m.row(HudRow::Lighting).contains("light history"));
    assert!(!m.banner(0.0).unwrap().contains("calm"));
    assert!(!m.banner(0.0).unwrap().contains("landing fade"));

    let r = parse_args_from(argv(&["--source", "outdoor", "--calm"])).unwrap();
    let l = r.start_lighting();
    assert_eq!(
        (l.light_history, l.key_far, l.key_far_mix, l.penumbra),
        (false, KeyFar::Column, KeyFarMix::Step, Penumbra::Disc)
    );
    assert_eq!(r.start_look().far_normal, [4.0, 8.0]);
    assert_eq!(
        vxe_render::Lighting {
            light_history: true,
            key_far: base.start_lighting().key_far,
            key_far_mix: base.start_lighting().key_far_mix,
            penumbra: base.start_lighting().penumbra,
            ..l
        },
        base.start_lighting(),
        "--calm changes no other lighting row"
    );
    let m = r.start_modes(0.0);
    let row = m.row(HudRow::Lighting);
    assert!(row.contains("light history off, "), "{row}");
    let banner = m.banner(0.0).unwrap().replace('\n', ", ");
    assert!(banner.contains("calm, "), "{banner}");
    assert!(
        banner.contains("key far column, key far mix step, "),
        "{banner}"
    );

    let r = parse_args_from(argv(&[
        "--source",
        "outdoor",
        "--calm",
        "--far-normal",
        "2,6",
    ]))
    .unwrap();
    assert_eq!(r.start_look().far_normal, [2.0, 6.0]);
    assert!(!r.start_lighting().light_history);
    assert!(!r.start_modes(0.0).line().contains("calm"));
    let r = parse_args_from(argv(&[
        "--source",
        "outdoor",
        "--calm",
        "--penumbra",
        "approach",
    ]))
    .unwrap();
    assert_eq!(r.start_lighting().penumbra, Penumbra::Approach);
    assert!(!r.start_modes(0.0).line().contains("calm"));
    let r = parse_args_from(argv(&[
        "--source",
        "outdoor",
        "--light-history",
        "on",
        "--calm",
    ]))
    .unwrap();
    assert!(!r.start_lighting().light_history);
    let r = parse_args_from(argv(&[
        "--source",
        "outdoor",
        "--calm",
        "--light-history",
        "on",
    ]))
    .unwrap();
    assert!(r.start_lighting().light_history);
    assert!(parse_args_from(argv(&["--light-history", "maybe"])).is_err());

    let r = parse_args_from(argv(&[
        "--source",
        "outdoor",
        "--landing-fade",
        "on",
        "--calm",
    ]))
    .unwrap();
    assert!(r.start_lighting().landing_fade);
    let banner = r.start_modes(0.0).banner(0.0).unwrap().replace('\n', ", ");
    assert!(banner.contains("calm, "), "{banner}");
    assert!(banner.contains("landing fade on, "), "{banner}");
    assert!(parse_args_from(argv(&["--landing-fade", "maybe"])).is_err());
}

/// Plan 0098 Phase 6b: `--source-fade on` sets `Lighting::source_fade` and the lighting row reads
/// `source fade on`; `--calm` alone leaves it off and the row does not name it.
#[test]
fn source_fade_parses_and_calm_leaves_it_off() {
    let calm = parse_args_from(argv(&["--source", "outdoor", "--calm"])).unwrap();
    assert!(!calm.start_lighting().source_fade);
    let banner = calm.start_modes(0.0).banner(0.0).unwrap();
    assert!(!banner.contains("source fade"), "{banner}");
    let r = parse_args_from(argv(&[
        "--source",
        "outdoor",
        "--calm",
        "--landing-fade",
        "on",
        "--source-fade",
        "on",
    ]))
    .unwrap();
    assert!(r.start_lighting().source_fade);
    let banner = r.start_modes(0.0).banner(0.0).unwrap().replace('\n', ", ");
    assert!(banner.contains("source fade on, "), "{banner}");
    assert!(banner.contains("landing fade on, "), "{banner}");
    let mut row = String::new();
    r.start_modes(0.0).write_diag(&mut row);
    assert!(row.contains("source fade on, "), "{row}");
    assert!(parse_args_from(argv(&["--source-fade", "maybe"])).is_err());
}

/// Plan 0098 Phase 8: `Lighting::dig_hold` is on by default and the row does not name it;
/// `--dig-hold off` clears it and the lighting row and the banner read `dig hold off`.
#[test]
fn dig_hold_is_on_and_off_names_itself() {
    let calm = parse_args_from(argv(&["--source", "outdoor", "--calm"])).unwrap();
    assert!(calm.start_lighting().dig_hold);
    let banner = calm.start_modes(0.0).banner(0.0).unwrap();
    assert!(!banner.contains("dig hold"), "{banner}");
    let r = parse_args_from(argv(&[
        "--source",
        "outdoor",
        "--calm",
        "--dig-hold",
        "off",
    ]))
    .unwrap();
    assert!(!r.start_lighting().dig_hold);
    let banner = r.start_modes(0.0).banner(0.0).unwrap().replace('\n', ", ");
    assert!(banner.contains("dig hold off, "), "{banner}");
    let mut row = String::new();
    r.start_modes(0.0).write_diag(&mut row);
    assert!(row.contains("dig hold off, "), "{row}");
    assert!(parse_args_from(argv(&["--dig-hold", "maybe"])).is_err());
}
