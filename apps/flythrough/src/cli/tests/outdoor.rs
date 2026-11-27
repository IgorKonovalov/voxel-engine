//! The outdoor section's lighting flags: the penumbra, the key light's far source, the defaults' picks,
//! the sky term's normal and bounce, the exposure's ceiling.

use crate::cli::parse_args_from;
use crate::test_support::argv;
use svet_render::Exposure;

/// `--penumbra`: disc unless named (Plan 0098 Phase 4, the fly-through's `DEFAULT_PENUMBRA`),
/// which the banner leaves out; approach reaches the lighting and the banner.
#[test]
fn penumbra_is_disc_unless_named() {
    let r = parse_args_from(argv(&["--source", "outdoor"])).unwrap();
    assert_eq!(r.start_lighting().penumbra, svet_render::Penumbra::Disc);
    assert!(!r.start_modes(0.0).line().contains("penumbra"));
    let r = parse_args_from(argv(&["--source", "outdoor", "--penumbra", "approach"])).unwrap();
    assert_eq!(r.start_lighting().penumbra, svet_render::Penumbra::Approach);
    assert!(r.start_modes(0.0).line().contains("penumbra approach, "));
    assert!(parse_args_from(argv(&["--penumbra", "soft"])).is_err());
}

/// Plan 0095 Phases 2 and 4, the default Plan 0098 Phase 4's: `--key-far` is column with the
/// step unless named, which the banner leaves out; `ray`, `baked` and `--key-far-mix blend`
/// reach the lighting and the banner with a sky set.
#[test]
fn key_far_is_column_step_unless_named() {
    use svet_render::{KeyFar, KeyFarMix};
    let r = parse_args_from(argv(&["--source", "outdoor"])).unwrap();
    let l = r.start_lighting();
    assert_eq!(
        (l.key_far, l.key_far_mix),
        (KeyFar::Column, KeyFarMix::Step)
    );
    assert!(!r.start_modes(0.0).line().contains("key far"));
    let r = parse_args_from(argv(&["--source", "outdoor", "--key-far-mix", "blend"])).unwrap();
    assert_eq!(r.start_lighting().key_far_mix, KeyFarMix::Blend);
    assert!(
        r.start_modes(0.0)
            .line()
            .contains("key far column, key far mix blend, ")
    );
    let r = parse_args_from(argv(&["--source", "outdoor", "--key-far", "ray"])).unwrap();
    assert_eq!(r.start_lighting().key_far, KeyFar::Ray);
    assert!(r.start_modes(0.0).line().contains("key far ray, "));
    let r = parse_args_from(argv(&["--source", "outdoor", "--key-far", "baked"])).unwrap();
    assert_eq!(r.start_lighting().key_far, KeyFar::Baked);
    assert!(r.start_modes(0.0).line().contains("key far baked, "));
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
        (svet_render::SkyNormal::Smoothed, 0.0)
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
        (svet_render::SkyNormal::Face, 0.3)
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
        svet_render::DEFAULT_EXPOSURE_RATE_EV_S
    );
    assert_eq!(svet_render::DEFAULT_EXPOSURE_RATE_EV_S, 3.75);
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
    let stats = svet_render::QueueStats {
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

/// Plan 0108 Phase 1: `--pre-roll` takes full, near and off, is full headless unless given
/// (the window's default by source is the test below), refuses a fourth value, and names itself on the banner's queue row beside the bake thread; near and
/// off refuse `--pre-roll-drain`, full takes it. Near and off box the coarse pass at 0 ms with
/// `--pre-roll-stepped` too (which then lifts only the cap), full keeps the 5 s box and the
/// 120 s cap, or neither with the stepped flag.
#[test]
fn the_pre_roll_takes_full_near_or_off_and_the_banner_names_it() {
    use crate::cli::PreRollMode;
    use crate::run::hud_feed::write_queue_banner_row;
    use crate::run::loading::pre_roll_limits;
    let mode = |a: &[&str]| parse_args_from(argv(a)).map(|r| r.pre_roll());
    assert_eq!(
        mode(&["--headless", "--source", "outdoor"]),
        Ok(PreRollMode::Full)
    );
    assert_eq!(mode(&["--headless"]), Ok(PreRollMode::Full));
    for (v, want) in [
        ("full", PreRollMode::Full),
        ("near", PreRollMode::Near),
        ("off", PreRollMode::Off),
    ] {
        assert_eq!(
            mode(&["--source", "outdoor", "--pre-roll", v]),
            Ok(want),
            "{v}"
        );
        let r = parse_args_from(argv(&["--headless", "--pre-roll", v])).unwrap();
        let mut row = String::new();
        write_queue_banner_row(&mut row, &r, "w");
        assert!(
            row.contains(&format!(", ring 3 on, pre-roll {v}, bake thread off; w")),
            "{row}"
        );
    }
    assert!(mode(&["--pre-roll", "coarse"]).is_err());
    assert!(mode(&["--pre-roll"]).is_err());

    let drain = |v: &str| parse_args_from(argv(&["--pre-roll", v, "--pre-roll-drain", "30"]));
    assert!(drain("full").is_ok());
    for v in ["near", "off"] {
        let Err(e) = drain(v) else {
            panic!("--pre-roll {v} with --pre-roll-drain is refused");
        };
        assert!(
            e.contains(&format!("--pre-roll {v}")) && e.contains("--pre-roll-drain"),
            "{e}"
        );
    }

    let limits = |a: &[&str]| pre_roll_limits(&parse_args_from(argv(a)).unwrap());
    assert_eq!(limits(&["--source", "outdoor"]), (0.0, 120_000.0));
    assert_eq!(limits(&["--source", "showcase"]), (5000.0, 120_000.0));
    assert_eq!(limits(&["--headless"]), (5000.0, 120_000.0));
    assert_eq!(
        limits(&["--headless", "--pre-roll", "near"]),
        (0.0, 120_000.0)
    );
    assert_eq!(
        limits(&["--headless", "--pre-roll", "off"]),
        (0.0, 120_000.0)
    );
    let stepped = |v: &str| limits(&["--headless", "--pre-roll-stepped", "--pre-roll", v]);
    assert_eq!(stepped("full"), (f64::INFINITY, f64::INFINITY));
    assert_eq!(stepped("near"), (0.0, f64::INFINITY));
    assert_eq!(stepped("off"), (0.0, f64::INFINITY));
}

/// Plan 0108 Phase 4, the owner's pick: unset, `--pre-roll` is `near` in a window over a
/// streamed source (`cave`, `outdoor`) and `full` over a bounded one (`level`, `showcase`);
/// `full` under `--headless` for all four, and `full` with `--pre-roll-drain` in a window. The
/// banner's queue row names the mode in effect beside the bake thread's. A given value wins
/// everywhere.
#[test]
fn the_pre_roll_defaults_to_near_in_a_window_over_a_streamed_source_only() {
    use crate::cli::PreRollMode;
    use crate::run::hud_feed::write_queue_banner_row;
    let parse = |a: &[&str]| parse_args_from(argv(a)).unwrap();
    for (source, window) in [
        ("cave", PreRollMode::Near),
        ("outdoor", PreRollMode::Near),
        ("level", PreRollMode::Full),
        ("showcase", PreRollMode::Full),
    ] {
        for (headless, want, thread) in [(false, window, "on"), (true, PreRollMode::Full, "off")] {
            let mut a = vec!["--source", source];
            if headless {
                a.push("--headless");
            }
            let r = parse(&a);
            assert_eq!(r.pre_roll(), want, "{a:?}");
            let mut row = String::new();
            write_queue_banner_row(&mut row, &r, "w");
            assert!(
                row.contains(&format!(
                    ", ring 3 on, pre-roll {}, bake thread {thread}; w",
                    want.name()
                )),
                "{a:?}: {row}"
            );
            for given in ["full", "near", "off"] {
                let mut g = a.clone();
                g.extend(["--pre-roll", given]);
                assert_eq!(parse(&g).pre_roll().name(), given, "{g:?}");
            }
        }
    }
    let drained = parse(&["--source", "outdoor", "--pre-roll-drain", "30"]);
    assert_eq!(drained.pre_roll(), PreRollMode::Full);
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
        svet_render::DEFAULT_SKY_ANCHOR_FLOOR
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
        Some(svet_render::DEFAULT_SKY_ANCHOR_LIFT)
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
        svet_render::DEFAULT_EXPOSURE_HIGHLIGHT
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
        svet_render::DEFAULT_SKY_ANCHOR_SHARE
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

/// Plan 0098 Phase 4 (ADR-0082, window W2's picks): with no flag the start's lighting and look
/// are the five picks (light history off, key far column, key far mix step, the disc penumbra,
/// far normal 4..8) and the arms not picked are off (`landing fade`, `source fade`, `far light
/// wide`, `light history far`); the lighting row and the banner name none of them. Each flag
/// moves its row alone and is named: `--light-history on` reads `light history on`, `--key-far
/// baked` and `--key-far-mix blend` read `key far baked, key far mix blend`, `--penumbra
/// approach` reads `penumbra approach`, `--far-normal 2,6` sets the band. `--calm` is gone (the
/// defaults are its combination).
#[test]
fn the_defaults_are_the_five_picks_and_a_flag_names_its_row() {
    use crate::hud::HudRow;
    use svet_render::{KeyFar, KeyFarMix, Penumbra};
    let base = parse_args_from(argv(&["--source", "outdoor"])).unwrap();
    let l = base.start_lighting();
    assert_eq!(
        (l.light_history, l.key_far, l.key_far_mix, l.penumbra),
        (false, KeyFar::Column, KeyFarMix::Step, Penumbra::Disc)
    );
    assert_eq!(base.start_look().far_normal, [4.0, 8.0]);
    assert_eq!(
        (
            l.landing_fade,
            l.source_fade,
            l.far_light_wide,
            l.light_history_far
        ),
        (false, false, false, false)
    );
    let m = base.start_modes(0.0);
    assert!(!m.row(HudRow::Lighting).contains("light history"));
    let banner = m.banner(0.0).unwrap().replace('\n', ", ");
    for word in [
        "calm",
        "light history",
        "key far",
        "penumbra",
        "landing fade",
    ] {
        assert!(!banner.contains(word), "{word}: {banner}");
    }

    let r = parse_args_from(argv(&["--source", "outdoor", "--light-history", "on"])).unwrap();
    assert_eq!(
        r.start_lighting(),
        svet_render::Lighting {
            light_history: true,
            ..l
        },
        "--light-history on moves its row alone"
    );
    let m = r.start_modes(0.0);
    let row = m.row(HudRow::Lighting);
    assert!(row.contains("light history on, "), "{row}");
    let banner = m.banner(0.0).unwrap().replace('\n', ", ");
    assert!(banner.contains("light history on, "), "{banner}");

    let r = parse_args_from(argv(&[
        "--source",
        "outdoor",
        "--key-far",
        "baked",
        "--key-far-mix",
        "blend",
        "--penumbra",
        "approach",
        "--far-normal",
        "2,6",
    ]))
    .unwrap();
    let k = r.start_lighting();
    assert_eq!(
        (k.light_history, k.key_far, k.key_far_mix, k.penumbra),
        (false, KeyFar::Baked, KeyFarMix::Blend, Penumbra::Approach)
    );
    assert_eq!(r.start_look().far_normal, [2.0, 6.0]);
    let banner = r.start_modes(0.0).banner(0.0).unwrap().replace('\n', ", ");
    assert!(
        banner.contains("key far baked, key far mix blend, "),
        "{banner}"
    );
    assert!(banner.contains("penumbra approach, "), "{banner}");

    assert!(parse_args_from(argv(&["--source", "outdoor", "--calm"])).is_err());
    assert!(parse_args_from(argv(&["--light-history", "maybe"])).is_err());
    let r = parse_args_from(argv(&["--source", "outdoor", "--landing-fade", "on"])).unwrap();
    assert!(r.start_lighting().landing_fade);
    let banner = r.start_modes(0.0).banner(0.0).unwrap().replace('\n', ", ");
    assert!(banner.contains("landing fade on, "), "{banner}");
    assert!(parse_args_from(argv(&["--landing-fade", "maybe"])).is_err());
}

/// Plan 0098 Phase 6b: `--source-fade on` sets `Lighting::source_fade` and the lighting row reads
/// `source fade on`; the defaults leave it off and the row does not name it.
#[test]
fn source_fade_parses_and_the_defaults_leave_it_off() {
    let base = parse_args_from(argv(&["--source", "outdoor"])).unwrap();
    assert!(!base.start_lighting().source_fade);
    let banner = base.start_modes(0.0).banner(0.0).unwrap();
    assert!(!banner.contains("source fade"), "{banner}");
    let r = parse_args_from(argv(&[
        "--source",
        "outdoor",
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

/// Plan 0098 Phases 8, 14 and 17: `Lighting::dig_hold` is `on` by default and the row does
/// not name it; `--dig-hold rays` and `--dig-hold off` set it and the lighting row and the
/// banner name it; `--dig-hold land` and `--dig-hold near` are refused.
#[test]
fn dig_hold_is_on_rays_and_off_names_itself() {
    use svet_render::DigHold;
    let base = parse_args_from(argv(&["--source", "outdoor"])).unwrap();
    assert_eq!(base.start_lighting().dig_hold, DigHold::On);
    let banner = base.start_modes(0.0).banner(0.0).unwrap();
    assert!(!banner.contains("dig hold"), "{banner}");
    for gone in ["land", "near"] {
        assert!(
            parse_args_from(argv(&["--source", "outdoor", "--dig-hold", gone])).is_err(),
            "{gone} is refused"
        );
    }
    for (v, d) in [("rays", DigHold::Rays), ("off", DigHold::Off)] {
        let r = parse_args_from(argv(&["--source", "outdoor", "--dig-hold", v])).unwrap();
        assert_eq!(r.start_lighting().dig_hold, d);
        let banner = r.start_modes(0.0).banner(0.0).unwrap().replace('\n', ", ");
        assert!(banner.contains(&format!("dig hold {v}, ")), "{banner}");
        let mut row = String::new();
        r.start_modes(0.0).write_diag(&mut row);
        assert!(row.contains(&format!("dig hold {v}, ")), "{row}");
    }
    assert!(parse_args_from(argv(&["--dig-hold", "maybe"])).is_err());
}

/// No `--key-at` or `--key-horizon` flag stands (unknown arguments), and
/// `--key-cover record` is refused by `--key-cover`'s own error, which names what it takes.
#[test]
fn the_removed_key_settings_fail_to_parse() {
    let err = |a: &[&str]| parse_args_from(argv(a)).err();
    assert_eq!(
        err(&["--key-at", "face"]).as_deref(),
        Some("unknown argument --key-at")
    );
    assert_eq!(
        err(&["--key-horizon", "walk"]).as_deref(),
        Some("unknown argument --key-horizon")
    );
    assert_eq!(
        err(&["--key-cover", "record"]).as_deref(),
        Some("--key-cover takes off or ray, not record")
    );
}

/// Plan 0100 Phase 6: `--key-cover` is `off` unless named, which the lighting row and the banner
/// leave out; `ray` reaches the lighting, the row and the banner; the withdrawn `ray-record`
/// and `record` are refused; the debug page's `key cover` cycles the two and its row reads the
/// value.
#[test]
fn key_cover_is_off_unless_named() {
    use crate::hud::HudRow;
    use svet_render::KeyCover;
    let r = parse_args_from(argv(&["--source", "outdoor"])).unwrap();
    assert_eq!(r.start_lighting().key_cover, KeyCover::Off);
    let m = r.start_modes(0.0);
    assert!(!m.row(HudRow::Lighting).contains("key cover"));
    assert!(!m.banner(0.0).unwrap().contains("key cover"));
    {
        let k = KeyCover::Ray;
        let r = parse_args_from(argv(&["--source", "outdoor", "--key-cover", k.name()])).unwrap();
        assert_eq!(r.start_lighting().key_cover, k);
        let m = r.start_modes(0.0);
        let want = format!("key cover {}, ", k.name());
        let row = m.row(HudRow::Lighting);
        assert!(row.contains(&want), "{row}");
        let banner = m.banner(0.0).unwrap().replace('\n', ", ");
        assert!(banner.contains(&want), "{banner}");
    }
    assert!(parse_args_from(argv(&["--key-cover", "both"])).is_err());
    assert!(parse_args_from(argv(&["--key-cover", "ray-record"])).is_err());
    assert!(parse_args_from(argv(&["--key-cover", "record"])).is_err());
    let tog = crate::toggles::TOGGLES
        .iter()
        .find(|t| t.name == "key cover")
        .unwrap();
    let mut m = r.start_modes(0.0);
    for want in ["key cover ray", "key cover off"] {
        (tog.flip)(&mut m, 0.0);
        assert_eq!((tog.get)(&m), want);
    }
}

/// Plan 0100 Phase 9: every profile keeps `key cover` at its default (`off`), and `--profile`
/// gives it as the profile holds it.
#[test]
fn the_profiles_keep_the_key_defaults() {
    use svet_render::{KeyCover, Profile, RenderSettings};
    for p in Profile::ALL {
        let want = RenderSettings::profile(p).lighting;
        assert_eq!(want.key_cover, KeyCover::Off, "{}", p.name());
        let r = parse_args_from(argv(&["--source", "outdoor", "--profile", p.name()])).unwrap();
        let l = r.render_settings().lighting;
        assert_eq!(l.key_cover, want.key_cover, "{}", p.name());
    }
}

/// `--eye`: the outdoor camera starts at the named point and view in place of the spawn's;
/// unset, the spawn's camera; a wrong count or a pitch past 89 degrees is refused.
#[test]
fn eye_places_the_outdoor_camera() {
    let r = parse_args_from(argv(&["--source", "outdoor"])).unwrap();
    assert_eq!(r.outdoor.eye, None);
    let r = parse_args_from(argv(&[
        "--source",
        "outdoor",
        "--eye",
        "496.0,-648.7,456.7,34.5,-46.2",
    ]))
    .unwrap();
    let cam = crate::setup::outdoor_camera(&r);
    assert_eq!(<[f32; 3]>::from(cam.position), [496.0, -648.7, 456.7]);
    assert_eq!(cam.yaw, 34.5f32.to_radians());
    assert_eq!(cam.pitch, (-46.2f32).to_radians());
    assert!(parse_args_from(argv(&["--eye", "1,2,3,4"])).is_err());
    assert!(parse_args_from(argv(&["--eye", "1,2,3,4,90"])).is_err());
    assert!(parse_args_from(argv(&["--eye", "1,2,x,4,5"])).is_err());
}
