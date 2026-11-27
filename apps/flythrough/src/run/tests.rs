//! The run's tests on a bare run: the banner's rows, the keys and the edits it applies.

use super::*;
use crate::LookPreset;
use crate::app::session_outcome;
use crate::castle;
use crate::cave::ROCK;
use crate::cli::parse_args_from;
use crate::edits::Coarse;
use crate::fly::{CutawayMode, Fly};
use crate::hud::{self};
use crate::now;
use crate::run::hud_feed::{Hud, write_pace_banner};
use crate::run::hud_lines::look_line;
use crate::setup::{GfxSample, castle_camera, start_camera};
use crate::test_support::{applied, argv, bare_run, carved_rock, counting};
use crate::toggles::TOGGLES;
use svet_app::{Axis, Flow, Input, KeyCode, MouseButton, PadButton};
use svet_render::glam::Vec3;
use svet_render::{FRAME_PASSES, LightingResolution, ViewMode};
use svet_world::{ChunkCoord, EditOp, EditResult, FIXED_ONE, Fixed3, Mode, OpResult, VoxelCoord};

/// A plain start's banner names the pacing, `pace free`, until the banner's time is up;
/// `--pace compositor` names its word.
#[test]
fn a_plain_start_s_banner_names_the_pace() {
    let pace_banner = |pace, now_ms| {
        let mut out = String::new();
        write_pace_banner(pace, now_ms, &mut out).then_some(out)
    };
    let a = parse_args_from(argv(&[])).expect("no flags parse");
    assert_eq!(pace_banner(a.pace, 0.0).as_deref(), Some("pace free"));
    assert_eq!(pace_banner(a.pace, hud::BANNER_MS), None);
    let c = parse_args_from(argv(&["--pace", "compositor"])).unwrap();
    assert_eq!(
        pace_banner(c.pace, 10.0).as_deref(),
        Some("pace compositor")
    );
    let mut run = bare_run(carved_rock());
    run.start = now();
    let banner = run.banner().expect("the banner shows");
    assert!(
        banner.starts_with("pace free\nlight: live queue"),
        "{banner}"
    );
}

/// A shown HUD's whole CPU half (`Run::fill_hud`: the values refreshed with a lighting row
/// from a GPU sample and the frame's passes, the rows and strip formatted, and the banner's
/// rows under them, at the start where the banner shows its pace, its modes, the upscaler
/// and a pan) allocates on the first frame and 0 bytes on frames 2 to 100.
#[test]
fn a_shown_hud_s_refresh_and_banner_allocate_nothing_after_the_first_frame() {
    let mut run = bare_run(carved_rock());
    run.hud = Hud::new(true);
    run.hud.pass_ms = Some((0..FRAME_PASSES.len()).map(|i| 0.1 * i as f64).collect());
    run.hud.store = Some(svet_render::StoreStats::default());
    run.queue.stats = Some(svet_render::QueueStats::default());
    run.modes.pan = Some(hud::Pan::new(1.5, 0.0));
    run.modes.upscaler = hud::UpscalerBanner::shown_from(hud::UpscalerMode::Fsr2, "1.5", 0.0);
    let g = GfxSample {
        mirror_bytes: 3 << 20,
        pool_live: 1200,
        pool_free: 34,
        pressure: 0.25,
        budget_bytes: 64 << 20,
        light_samples: 4096,
        sizes: Some(hud::FrameSizes {
            internal: [640, 360],
            output: [960, 540],
            window: Some([1920, 1080]),
            upscaling: true,
        }),
    };
    run.start = now();
    let mut per_frame = Vec::with_capacity(100);
    for frame in 0..100u32 {
        let before = counting::bytes();
        run.hud.frames.record(0.016 + frame as f32 * 1e-5);
        run.fill_hud(Some(g));
        per_frame.push(counting::bytes() - before);
    }
    let text = &run.hud.text;
    assert!(text.contains(FRAME_PASSES[0]), "the lighting row: {text}");
    assert!(text.contains("pace free"), "the banner: {text}");
    assert!(
        text.contains("upscaler fsr2"),
        "the upscaler's banner: {text}"
    );
    assert!(text.contains("pan 1.5 deg/s"), "the pan's banner: {text}");
    println!("bytes per frame: {per_frame:?}");
    assert!(per_frame[0] > 0, "the first frame grows the buffers");
    assert_eq!(per_frame[1..], [0; 99]);
}

/// A mirror sync that fails ends the session with its failure recorded, and the headless
/// runner's outcome is that failure as `Err`; with none recorded it is `Ok`.
#[test]
fn a_failed_mirror_sync_ends_the_session_as_a_failure() {
    let mut run = bare_run(carved_rock());
    assert!(!run.failure.is_some());
    assert_eq!(session_outcome(&run.failure), Ok(()));
    let e = svet_render::MirrorError::PoolTooLarge {
        needed: 2 << 20,
        limit: 1 << 20,
    };
    let message = format!("mirror sync failed: {e}");
    assert_eq!(run.mirror_failed(e), Flow::Exit);
    assert!(run.failure.is_some());
    // The first failure is kept.
    run.failure.set("later".to_owned());
    assert_eq!(session_outcome(&run.failure), Err(message));
    assert_eq!(session_outcome(&run.failure), Ok(()));
}

/// An op applied in a tick with no logic line naming it (here one submitted to the server
/// directly; a hosted logic that applied ops and then trapped leaves the same) still
/// reaches the coarse grid by the end of the tick.
#[test]
fn an_applied_op_no_logic_line_names_still_updates_the_coarse_grid() {
    let mut run = bare_run(carved_rock());
    run.coarse = Some(Coarse::build(&run.world));
    let solid = |run: &Run| {
        let grid = &run.coarse.as_ref().unwrap().grid;
        grid.cell(ChunkCoord([0, 0, 0]), 0).unwrap().solid
    };
    assert_eq!(solid(&run), 0, "the carve emptied cell (0, 0, 0)");
    let fill = EditOp::box_(Fixed3([0; 3]), Fixed3([8 * FIXED_ONE; 3]), Mode::Fill(ROCK));
    run.net.as_ref().unwrap().server.submit(fill);
    let (benches, refused, _) = run.tick_server();
    assert_eq!((benches, refused), (0, 0));
    assert_eq!(solid(&run), 512);
}

/// Runs the menu entry labelled `label` as `Enter` on it does.
fn enter(run: &mut Run, label: &str) {
    use crate::hud::menu::{Activated, EntryKind, entries};
    let e = entries()
        .find(|e| e.label == label)
        .unwrap_or_else(|| panic!("{label} is an entry"));
    run.menu_act(match e.kind {
        EntryKind::Toggle(t) => Activated::Toggle(t),
        EntryKind::Action(a) => Activated::Action(a.act),
    });
}

/// The edits one server tick of `run` applied.
fn tick_edits(run: &mut Run) -> Vec<EditResult> {
    let net = run.net.as_mut().unwrap();
    assert_eq!(net.server.tick().logic_error, None);
    applied(&net.client.poll())
        .into_iter()
        .map(|r| match r {
            OpResult::Edit(e) => e,
            other => panic!("{other:?}"),
        })
        .collect()
}

/// `handle_input` digs on a left click and on `R2` pulled through `Input`'s setters
/// alike: the dig reaches the server, which carves the same rock from the wall ahead;
/// `R2` held on the next frame does not dig again, and released and pulled again does.
/// `L2` fills.
#[test]
fn r2_digs_as_the_left_click_does_and_l2_fills() {
    let mut click = bare_run(carved_rock());
    let mut input = Input::default();
    input.button_down(MouseButton::Left);
    click.handle_input(&input);
    let by_click = tick_edits(&mut click);
    let [r] = &by_click[..] else {
        panic!("{by_click:?}")
    };
    assert_eq!(r.changed_from.len(), 1);
    assert_eq!(r.changed_from[0].0, ROCK);
    assert!(r.changed_from[0].1 > 0);

    let mut pad = bare_run(carved_rock());
    let mut input = Input::default();
    input.set_axis(Axis::TriggerR, 1.0);
    pad.handle_input(&input);
    assert_eq!(tick_edits(&mut pad), by_click);
    input.end_frame();
    pad.handle_input(&input);
    assert_eq!(tick_edits(&mut pad), [], "a held trigger digs once");
    input.set_axis(Axis::TriggerR, 0.0);
    pad.handle_input(&input);
    input.set_axis(Axis::TriggerR, 0.8);
    pad.handle_input(&input);
    assert_eq!(tick_edits(&mut pad).len(), 1, "pulled again, it digs again");

    let mut input = Input::default();
    input.set_axis(Axis::TriggerL, 1.0);
    pad.handle_input(&input);
    let filled = tick_edits(&mut pad);
    let [f] = &filled[..] else {
        panic!("{filled:?}")
    };
    assert_eq!(f.changed_from.len(), 1);
    assert_eq!(f.changed_from[0].0, 0, "the fill writes rock over air");
}

/// `--level --pose vein|patch` starts the camera at exactly the settle rig's pose (its
/// position, yaw and pitch) and leaves the exposure on auto; `--pose` without `--level` and
/// an unknown name are errors that list the names; the banner's first row reads `pose vein`
/// until the camera moves.
#[test]
fn pose_starts_the_level_at_the_rigs_still_camera() {
    for want in crate::START_POSES {
        let a = parse_args_from(argv(&["--level", "--pose", want.name])).expect("parses");
        assert_eq!(a.pose, Some(want));
        assert_eq!(a.light.exposure, None, "the pose's exposure is not applied");
        let c = start_camera(&a, VoxelCoord([0, 0, 0]));
        assert_eq!(c.position, Vec3::from(want.position));
        assert_eq!(c.yaw, want.yaw_deg.to_radians());
        assert_eq!(c.pitch, want.pitch_deg.to_radians());
        assert_eq!(c, want.camera());
    }
    let e = parse_args_from(argv(&["--pose", "vein"]))
        .err()
        .expect("needs --level");
    assert!(
        e.contains("needs --level") && e.contains("vein, patch"),
        "{e}"
    );
    let e = parse_args_from(argv(&["--level", "--pose", "cave"]))
        .err()
        .expect("unknown name");
    assert_eq!(e, "--pose takes vein, patch, far, vein-flight, not cave");
    assert!(parse_args_from(argv(&["--level", "--pose"])).is_err());

    let mut run = bare_run(carved_rock());
    let at = crate::VEIN_POSE.camera();
    run.fly = Fly::new(at);
    run.pose = Some((hud::pose_banner(crate::VEIN_POSE.name), at));
    run.forget_pose_once_moved();
    let banner = run.banner().expect("the pose's row shows");
    assert_eq!(banner.lines().next(), Some("pose vein"), "{banner}");
    run.fly.camera.position.x += 0.25;
    run.forget_pose_once_moved();
    assert!(
        run.banner().is_none_or(|b| !b.contains("pose vein")),
        "the row goes once the camera moves"
    );
}

/// `V` steps the far normal from the default `[4, 8]` through `[8, 12]`, `[2, 6]`, `[1, 2]`, off
/// and `[4, 8]`, `Z` the sharpening through
/// 0.25, 0 and 0.5, and `Period` stops and starts `--pan`'s yaw; each press changes the
/// banner's text to the new mode, and the stats HUD's aliasing row names all four.
/// `--pan` parses a number of degrees a second and refuses a non-number and 0; FSR 2's
/// candidate knobs parse, refuse 0 and a non-word, and show in the fsr2 banner line.
#[test]
fn the_aliasing_toggles_show_in_the_banner() {
    let mut run = bare_run(carved_rock());
    run.modes.upscaler = hud::UpscalerBanner::new(hud::UpscalerMode::Fsr2, "1.5");
    run.modes.pan = Some(hud::Pan::new(1.1, -1.0e9));
    let press = |run: &mut Run, label: &str| {
        enter(run, label);
        run.banner().expect("a press shows the banner")
    };
    for want in [
        "far normal 8..12",
        "far normal 2..6",
        "far normal 1..2",
        "far normal off",
        "far normal 4..8",
    ] {
        let banner = press(&mut run, "far normal");
        assert!(banner.contains(want), "{want}: {banner}");
    }
    for want in ["sharpen 0.25", "sharpen 0", "sharpen 0.5"] {
        let banner = press(&mut run, "sharpen");
        assert!(
            banner
                .lines()
                .any(|l| l == format!("upscaler fsr2, scale 1.5, {want}")),
            "{want}: {banner}"
        );
        assert_eq!(run.args.look.sharpen.to_string(), want["sharpen ".len()..]);
    }
    for (want, on) in [("pan 1.1 deg/s stopped", false), ("pan 1.1 deg/s", true)] {
        let banner = press(&mut run, "pan");
        assert!(banner.lines().any(|l| l == want), "{want}: {banner}");
        assert_eq!(run.modes.pan.map(|p| p.on), Some(on));
    }
    assert_eq!(
        hud::aliasing_row(
            &run.modes.upscaler,
            "far normal 4..8",
            &run.modes,
            run.modes.pan.as_ref()
        ),
        "upscaler fsr2, scale 1.5, sharpen 0.5; far normal 4..8; store read normal, weights face, lighting half; pan 1.1 deg/s"
    );
    let knobs = parse_args_from(argv(&["--jitter-phases", "8", "--dither", "on"]))
        .expect("the candidate knobs parse");
    assert_eq!(
        (knobs.look.jitter_phases, knobs.look.dither),
        (Some(8), true)
    );
    let banner = hud::UpscalerBanner::new(hud::UpscalerMode::Fsr2, "1.5").with_fsr2_knobs(
        hud::fsr2_knob_words(knobs.look.jitter_phases, knobs.look.dither),
    );
    assert_eq!(
        banner.line(),
        "upscaler fsr2, scale 1.5, sharpen 0.5, phases 8, dither"
    );
    for (flag, bad) in [("--jitter-phases", "0"), ("--dither", "x")] {
        assert!(
            parse_args_from(argv(&[flag, bad])).is_err(),
            "{flag} {bad} parses"
        );
    }
    let p = parse_args_from(argv(&["--pan", "1.1"])).expect("--pan 1.1 parses");
    assert_eq!(p.look.pan, Some(1.1));
    for bad in ["x", "0", "-0", "inf"] {
        assert_eq!(
            parse_args_from(argv(&["--pan", bad])).err().as_deref(),
            Some(format!("--pan takes degrees a second other than 0, not {bad}").as_str()),
            "{bad}"
        );
    }
}

/// The fly-through starts in the owner's picks of Plan 0061 Phase 4 and Plan 0098 (ADR-0082):
/// far normal `[4, 8]`,
/// the store read by the shading normal, the light's filters weighed by face, the light at
/// half resolution; the start banner's mode rows and the aliasing row name all four.
#[test]
fn the_default_look_is_the_windows_pick() {
    use svet_render::{LightWeights, StoreRead};
    assert_eq!(LookPreset::DEFAULT.far_normal, [4.0, 8.0]);
    let d = parse_args_from(argv(&[])).expect("parses");
    let l = d.start_lighting();
    assert_eq!(
        (l.store_read, l.light_weights, d.light.resolution),
        (
            StoreRead::Normal,
            LightWeights::Face,
            LightingResolution::Half
        )
    );
    assert_eq!(d.start_look().far_normal, [4.0, 8.0]);
    let banner = d
        .start_modes(0.0)
        .banner(0.0)
        .expect("the banner shows at the start");
    let rows: Vec<&str> = banner.lines().collect();
    assert_eq!(
        rows[..3],
        [
            "look face, far normal 4..8",
            "source cave, lighting half",
            "store read normal, weights face"
        ]
    );
    let run = bare_run(carved_rock());
    assert_eq!(
        hud::aliasing_row(
            &run.modes.upscaler,
            &LookPreset::far_normal_words(run.modes.look.normal_mode, run.modes.look.far_normal),
            &run.modes,
            None
        ),
        "upscaler off, scale 1; far normal 4..8; store read normal, weights face, lighting half; no pan"
    );
}

/// `--store-read` and `--light-weights` parse their two modes and refuse anything else;
/// `Comma` and `Semicolon` change the banner's text and the aliasing row's to each mode, and
/// the row names the light's resolution, which `F6` changes.
#[test]
fn the_store_read_toggle_shows_in_the_banner() {
    use svet_render::{LightWeights, StoreRead};
    for (flag, modes) in [
        ("--store-read", ["face", "normal"]),
        ("--light-weights", ["normal", "face"]),
    ] {
        for m in modes {
            let a = parse_args_from(argv(&[flag, m])).expect("a mode parses");
            let l = a.start_lighting();
            assert_eq!(
                if flag == "--store-read" {
                    l.store_read.name()
                } else {
                    l.light_weights.name()
                },
                m
            );
        }
        let what = if flag == "--store-read" {
            "face or normal"
        } else {
            "normal or face"
        };
        for bad in ["x", "Face", ""] {
            assert_eq!(
                parse_args_from(argv(&[flag, bad])).err().as_deref(),
                Some(format!("{flag} takes {what}, not {bad}").as_str()),
                "{flag} {bad}"
            );
        }
    }
    let d = parse_args_from(argv(&[])).expect("parses");
    assert_eq!(
        (
            d.start_lighting().store_read,
            d.start_lighting().light_weights
        ),
        (StoreRead::Normal, LightWeights::Face)
    );
    let mut run = bare_run(carved_rock());
    run.modes.upscaler = hud::UpscalerBanner::new(hud::UpscalerMode::Fsr2, "1.5");
    let press = |run: &mut Run, key: &str| {
        enter(run, key);
        let row = hud::aliasing_row(&run.modes.upscaler, "far normal 2..6", &run.modes, None);
        (run.banner().expect("a press shows the banner"), row)
    };
    for (key, banner_word, row_words) in [
        (
            "store read",
            "store read face",
            "store read face, weights face, lighting half",
        ),
        (
            "light weights",
            "weights normal",
            "store read face, weights normal, lighting half",
        ),
        (
            "lighting resolution",
            "lighting full",
            "store read face, weights normal, lighting full",
        ),
        (
            "store read",
            "store read normal",
            "store read normal, weights normal, lighting full",
        ),
        (
            "light weights",
            "weights face",
            "store read normal, weights face, lighting full",
        ),
    ] {
        let (banner, row) = press(&mut run, key);
        assert!(
            banner.replace('\n', " ").contains(banner_word),
            "{key:?}: {banner}"
        );
        assert_eq!(
            row,
            format!("upscaler fsr2, scale 1.5, sharpen 0.5; far normal 2..6; {row_words}; no pan"),
            "{key:?}"
        );
    }
}

/// `--queue-order` parses `score` and `distance` and refuses the rest, `--queue-weights` `A,R`
/// and `A,tier`; a plain start reads the score with the class as the tier; `F` cycles the
/// order, the menu's halving halves the angle weight and `Slash` doubles it, each changing the
/// banner and the HUD's queue row in the same frame.
#[test]
fn the_banner_names_the_queue_order_and_its_weights() {
    use svet_render::{QUEUE_ANGLE_WEIGHT as A, QUEUE_REFINE_WEIGHT as R};
    let mut run = bare_run(carved_rock());
    let shown = |run: &mut Run, label: Option<&str>| {
        if let Some(label) = label {
            enter(run, label);
        }
        run.refresh_hud_stats(None);
        let banner = run.banner().expect("the queue's row is on the banner");
        let row = run.hud.stats.queue.clone().expect("the HUD's queue row");
        (banner, row)
    };
    let check = |(banner, row): (String, String), want: String| {
        assert!(
            banner.contains(&format!(
                "; {want}; withdraw on (Tab: the light queue page)"
            )),
            "{want}: {banner}"
        );
        assert_eq!(
            row,
            format!(
                "in view: 0 waiting (0 coarse), drain 0.0 s, withdrawn 0, step 0.00 ms (render 0.00, movable 0.00); pre-roll full; bake thread on; {want}; withdraw on; full-first 2; reach full; budget frame:1.5"
            )
        );
    };
    check(
        shown(&mut run, None),
        format!("order score, angle {A}, refine tier"),
    );
    for want in [
        "order distance".to_owned(),
        format!("order score, angle {A}, refine {R}"),
        format!("order score, angle {A}, refine tier"),
        "order distance".to_owned(),
        format!("order score, angle {A}, refine {R}"),
    ] {
        check(shown(&mut run, Some("queue order")), want);
    }
    // `queue angle` is a number: one step down halves the weight, one up doubles it.
    let angle = crate::toggles::TOGGLES
        .iter()
        .find(|t| t.name == "queue angle")
        .expect("queue angle is a toggle");
    run.menu_act(crate::hud::menu::Activated::Step(angle, -1));
    check(
        shown(&mut run, None),
        format!("order score, angle {}, refine {R}", A / 2.0),
    );
    for a in [A, A * 2.0] {
        run.menu_act(crate::hud::menu::Activated::Step(angle, 1));
        check(
            shown(&mut run, None),
            format!("order score, angle {a}, refine {R}"),
        );
    }
    use svet_render::{QueueOrder, Refine};
    let order = |args: &[&str]| parse_args_from(argv(args)).map(|a| a.queue_order());
    assert_eq!(
        order(&["--queue-order", "distance"]),
        Ok(QueueOrder::Distance)
    );
    assert_eq!(
        order(&["--queue-order", "score", "--queue-weights", "1.5,3"]),
        Ok(QueueOrder::Score {
            angle: 1.5,
            refine: Refine::Weight(3.0)
        })
    );
    assert_eq!(
        order(&["--queue-weights", "4,tier"]),
        Ok(QueueOrder::Score {
            angle: 4.0,
            refine: Refine::Tier
        })
    );
    assert_eq!(
        order(&[]),
        Ok(QueueOrder::Score {
            angle: A,
            refine: Refine::Tier
        })
    );
    assert_eq!(order(&[]), Ok(QueueOrder::default()));
    for bad in [
        &["--queue-order", "near"][..],
        &["--queue-weights", "4"],
        &["--queue-weights", "x,2"],
        &["--queue-weights", "-1,2"],
        &["--queue-weights", "4,0"],
        &["--queue-weights", "4,flat"],
    ] {
        assert!(order(bad).is_err(), "{bad:?}");
    }
}

/// `--queue-withdraw` parses `on` and `off` and refuses the rest; `Quote` toggles the
/// withdrawal, the banner and the HUD's queue row saying so in the same frame; the row
/// shows the withdrawn count from the queue's counters.
#[test]
fn the_banner_names_the_withdrawal() {
    let mut run = bare_run(carved_rock());
    for want in ["withdraw off", "withdraw on", "withdraw off"] {
        enter(&mut run, "queue withdraw");
        run.refresh_hud_stats(None);
        let banner = run.banner().expect("the queue's row is on the banner");
        assert!(
            banner.contains(&format!("; {want} (Tab: the light queue page)")),
            "{want}: {banner}"
        );
        let row = run.hud.stats.queue.clone().expect("the HUD's queue row");
        assert!(
            row.ends_with(&format!(
                "; {want}; full-first 2; reach full; budget frame:1.5"
            )),
            "{want}: {row}"
        );
    }
    run.queue.stats = Some(svet_render::QueueStats {
        withdrawn: 7,
        ..Default::default()
    });
    run.refresh_hud_stats(None);
    let row = run.hud.stats.queue.clone().expect("the HUD's queue row");
    assert!(
        row.starts_with("in view: 0 waiting (0 coarse), drain 0.0 s, withdrawn 7, step 0.00 ms (render 0.00, movable 0.00); "),
        "{row}"
    );
    let w =
        |v: &str| parse_args_from(argv(&["--queue-withdraw", v])).map(|a| a.bake.queue_withdraw);
    assert_eq!(w("on"), Ok(true));
    assert_eq!(w("off"), Ok(false));
    assert!(w("yes").is_err());
    assert!(parse_args_from(argv(&[])).unwrap().bake.queue_withdraw);
}

/// `--full-first` parses `off` and 0 to 8 and refuses the rest, and follows `--near` when
/// absent; a plain start reads `2`; `Backslash` toggles `off` and `2`, each press changing the
/// banner and the HUD's queue row in the same frame.
#[test]
fn the_banner_names_the_full_first_radius() {
    let mut run = bare_run(carved_rock());
    run.refresh_hud_stats(None);
    let banner = run.banner().expect("the queue's row is on the banner");
    assert!(banner.contains(", full-first 2, budget "), "{banner}");
    assert!(banner.contains("(Tab: the light queue page)"), "{banner}");
    let row = run.hud.stats.queue.clone().expect("the HUD's queue row");
    assert!(
        row.ends_with("; full-first 2; reach full; budget frame:1.5"),
        "{row}"
    );
    for want in ["off", "2", "off"] {
        enter(&mut run, "full-first");
        run.refresh_hud_stats(None);
        let banner = run.banner().expect("the queue's row is on the banner");
        assert!(
            banner.contains(&format!(", full-first {want}, budget ")),
            "{want}: {banner}"
        );
        let row = run.hud.stats.queue.clone().expect("the HUD's queue row");
        assert!(
            row.ends_with(&format!(
                "; full-first {want}; reach full; budget frame:1.5"
            )),
            "{want}: {row}"
        );
    }
    let f = |args: &[&str]| parse_args_from(argv(args)).map(|a| a.full_first());
    assert_eq!(f(&[]), Ok(Some(2)));
    assert_eq!(f(&["--near", "0"]), Ok(Some(0)));
    assert_eq!(f(&["--full-first", "off"]), Ok(None));
    assert_eq!(f(&["--near", "0", "--full-first", "3"]), Ok(Some(3)));
    for bad in ["9", "-1", "on", "x"] {
        assert!(f(&["--full-first", bad]).is_err(), "{bad}");
    }
}

/// `--missing-light` parses the four modes and refuses the rest, a plain start reads
/// `chain`, and `7` cycles black, neighbour, parent and back to chain, each press changing
/// the banner's queue row in the same frame.
#[test]
fn the_banner_names_the_missing_light() {
    use svet_render::MissingLight;
    let mut run = bare_run(carved_rock());
    let banner = run.banner().expect("the queue's row is on the banner");
    assert!(
        banner.contains(", missing light chain, reach full, digs "),
        "{banner}"
    );
    assert!(banner.contains("(Tab: the light queue page)"), "{banner}");
    for want in ["black", "neighbour", "parent", "chain"] {
        enter(&mut run, "missing light");
        let banner = run.banner().expect("the queue's row is on the banner");
        assert!(
            banner.contains(&format!(", missing light {want}, reach full, digs ")),
            "{want}: {banner}"
        );
    }
    let f = |args: &[&str]| parse_args_from(argv(args)).map(|a| a.bake.missing_light);
    assert_eq!(f(&[]), Ok(MissingLight::Chain));
    for m in MissingLight::ALL {
        assert_eq!(f(&["--missing-light", m.name()]), Ok(m));
    }
    for bad in ["neighbor", "parents", "on", ""] {
        assert!(f(&["--missing-light", bad]).is_err(), "{bad}");
    }
}

/// `--reach` parses `full` and `partial` and refuses the rest; a plain start reads `full`;
/// `8` toggles `partial` and `full`, each press changing the banner and the HUD's queue row
/// in the same frame, the row adding the partial blocks under `partial`.
#[test]
fn the_banner_names_the_reach() {
    use svet_render::ReachMode;
    let mut run = bare_run(carved_rock());
    run.refresh_hud_stats(None);
    let banner = run.banner().expect("the queue's row is on the banner");
    assert!(banner.contains(", reach full, digs "), "{banner}");
    assert!(banner.contains("(Tab: the light queue page)"), "{banner}");
    let row = run.hud.stats.queue.clone().expect("the HUD's queue row");
    assert!(row.contains("; reach full; budget "), "{row}");
    for (want, note) in [("partial", "partial (0 partial)"), ("full", "full")] {
        enter(&mut run, "reach");
        run.refresh_hud_stats(None);
        let banner = run.banner().expect("the queue's row is on the banner");
        assert!(
            banner.contains(&format!(", reach {want}, digs ")),
            "{want}: {banner}"
        );
        let row = run.hud.stats.queue.clone().expect("the HUD's queue row");
        assert!(
            row.contains(&format!("; reach {note}; budget ")),
            "{want}: {row}"
        );
    }
    let f = |args: &[&str]| parse_args_from(argv(args)).map(|a| a.bake.reach);
    assert_eq!(f(&[]), Ok(ReachMode::Full));
    for m in ReachMode::ALL {
        assert_eq!(f(&["--reach", m.name()]), Ok(m));
    }
    for bad in ["partials", "on", "chain", ""] {
        assert!(f(&["--reach", bad]).is_err(), "{bad}");
    }
}

/// `--bake-budget` parses `frame:MS` and `second:MS,CAP` and refuses the rest, `--bake-slice
/// MS` reads `frame:MS`; a plain start reads `frame:1.5`; `Backspace` steps the budget through
/// `frame:1.5`, `second:150,0.25` and `second:300,0.25`, each press changing the banner and
/// the HUD's queue row in the same frame, the row adding the last slice under a per-second
/// budget.
#[test]
fn the_banner_names_the_budget() {
    use svet_render::Budget;
    let mut run = bare_run(carved_rock());
    run.refresh_hud_stats(None);
    let banner = run.banner().expect("the queue's row is on the banner");
    assert!(
        banner.contains(", budget frame:1.5, missing light chain, reach full, digs +"),
        "{banner}"
    );
    let row = run.hud.stats.queue.clone().expect("the HUD's queue row");
    assert!(row.ends_with("; budget frame:1.5"), "{row}");
    run.queue.stats = Some(svet_render::QueueStats {
        slice_ms: 2.3125,
        ..Default::default()
    });
    for (want, slice) in [
        ("second:150,0.25", " (slice 2.31 ms)"),
        ("second:300,0.25", " (slice 2.31 ms)"),
        ("frame:1.5", ""),
        ("second:150,0.25", " (slice 2.31 ms)"),
    ] {
        enter(&mut run, "bake budget");
        run.refresh_hud_stats(None);
        let banner = run.banner().expect("the queue's row is on the banner");
        assert!(
            banner.contains(&format!(
                ", budget {want}, missing light chain, reach full, digs +"
            )),
            "{want}: {banner}"
        );
        assert!(banner.contains("(Tab: the light queue page)"), "{banner}");
        let row = run.hud.stats.queue.clone().expect("the HUD's queue row");
        assert!(
            row.ends_with(&format!("; budget {want}{slice}")),
            "{want}: {row}"
        );
    }
    let b = |args: &[&str]| parse_args_from(argv(args)).map(|a| a.bake.budget);
    assert_eq!(b(&[]), Ok(Budget::PerFrame { ms: 1.5 }));
    assert_eq!(b(&["--bake-slice", "3"]), Ok(Budget::PerFrame { ms: 3.0 }));
    assert_eq!(
        b(&["--bake-budget", "second:150,0.25"]),
        Ok(Budget::PerSecond {
            ms_per_s: 150.0,
            cap: 0.25
        })
    );
    assert_eq!(
        b(&["--bake-budget", "frame:0.75"]),
        Ok(Budget::PerFrame { ms: 0.75 })
    );
    for bad in ["second:150", "second:150,2", "frame:x", "150"] {
        assert!(b(&["--bake-budget", bad]).is_err(), "{bad}");
    }
}

/// `F9` cycles the views shaded, normals, hitmask, steps, cracks, albedo, shaded; each
/// press shows the banner, which names the view (`view cracks`) unless it is shaded;
/// `--view cracks` parses.
#[test]
fn f9_cycles_the_views_and_the_banner_names_them() {
    let mut run = bare_run(carved_rock());
    let mut input = Input::default();
    let mut seen = Vec::new();
    for _ in 0..ViewMode::ALL.len() {
        input.end_frame();
        input.key_up(KeyCode::F9);
        input.key_down(KeyCode::F9);
        run.handle_input(&input);
        let banner = run.banner().expect("a press shows the banner");
        let named = banner
            .lines()
            .any(|l| l.split(", ").any(|p| p == "view cracks"));
        assert_eq!(named, run.modes.look.view == ViewMode::Cracks, "{banner}");
        if run.modes.look.view != ViewMode::Shaded {
            let want = format!("view {}", run.modes.look.view.name());
            assert!(banner.contains(&want), "{banner}");
        } else {
            assert!(!banner.contains("view "), "{banner}");
        }
        seen.push(run.modes.look.view.name());
    }
    assert_eq!(
        seen,
        ["normals", "hitmask", "steps", "cracks", "albedo", "shaded"]
    );
    let a = parse_args_from(argv(&["--view", "cracks"])).expect("parses");
    assert_eq!(a.look.view, ViewMode::Cracks);
}

/// With the castle, the menu's `next castle shot` moves the camera to the next shot's pose
/// and puts its name on the banner: gate, nave, window, dungeon, gate. Without it, it does
/// nothing.
#[test]
fn the_castle_shot_entry_cycles_the_shots() {
    let mut run = bare_run(carved_rock());
    enter(&mut run, "next castle shot");
    assert!(run.scene.is_none() && run.fly.camera.position == Vec3::splat(0.5));
    run.castle_shot = Some(castle::CastleShot::Gate);
    for want in ["nave", "window", "dungeon", "gate"] {
        enter(&mut run, "next castle shot");
        let shot = run.castle_shot.unwrap();
        assert_eq!(shot.name(), want);
        let (eye, _, _) = shot.pose();
        assert_eq!(
            run.fly.camera.position,
            Vec3::from(eye.map(|v| v as f32 + 0.5))
        );
        assert_eq!(run.scene.as_ref().unwrap().0, format!("castle {want}"));
        // Under the default light the queue's row comes first.
        assert!(
            run.banner()
                .unwrap()
                .lines()
                .any(|l| l == format!("castle {want}"))
        );
    }
    let (_, yaw, pitch) = castle::CastleShot::Nave.pose();
    let nave = castle_camera(castle::CastleShot::Nave);
    assert_eq!(
        (nave.yaw, nave.pitch),
        ((yaw as f32).to_radians(), (pitch as f32).to_radians())
    );
}

/// `X` and the pad's `West` (Square) cycle the cutaway alike: auto, on, off, auto.
#[test]
fn x_and_west_cycle_the_cutaway() {
    for press in [
        (|i: &mut Input| i.key_down(KeyCode::KeyX)) as fn(&mut Input),
        |i: &mut Input| i.pad_down(PadButton::West),
    ] {
        let mut run = bare_run(carved_rock());
        let mut input = Input::default();
        let mut seen = Vec::new();
        for _ in 0..3 {
            press(&mut input);
            run.handle_input(&input);
            seen.push(run.modes.cutaway);
            input.end_frame();
            input.key_up(KeyCode::KeyX);
            input.pad_up(PadButton::West);
        }
        assert_eq!(seen, [CutawayMode::On, CutawayMode::Off, CutawayMode::Auto]);
    }
}

/// With `--source showcase` the banner's look row is followed by a line that starts
/// `source showcase` and reads `lamp off`; `F7` turns it to `lamp on`. Other sources name none.
#[test]
fn the_showcase_banner_names_its_source_and_the_lamp() {
    let a = parse_args_from(argv(&["--source", "showcase"])).expect("parses");
    let mut m = a.start_modes(0.0);
    let line = m.line();
    assert!(line.starts_with("source showcase, "), "{line}");
    assert!(line.contains("lamp off"), "{line}");
    let banner = m.banner(0.0).expect("the banner shows at the start");
    let row = |b: &str| b.lines().nth(1).unwrap_or("").to_owned();
    assert!(banner.starts_with("look face, "), "{banner}");
    assert!(row(&banner).starts_with("source showcase"), "{banner}");
    assert!(banner.contains("lamp off"), "{banner}");
    assert!(banner.lines().all(|l| l.len() <= hud::BANNER_COLS));
    m.press(hud::ModeKey::Lamp, 1000.0);
    let banner = m.banner(1000.0).expect("F7 shows the banner");
    assert!(row(&banner).starts_with("source showcase"), "{banner}");
    assert!(
        banner.contains("lamp on") && !banner.contains("lamp off"),
        "{banner}"
    );
    let level = parse_args_from(argv(&["--level"])).expect("parses");
    assert!(
        level
            .start_modes(0.0)
            .line()
            .starts_with("source level, lighting ")
    );
}

/// While the lamp's bounce is on (the default) the banner names it and the probe spacing,
/// `probes 16` by default and `probes 32` with `--probe-spacing 32`, in every mode and
/// source; with it off (`--lamp-bounce off`, or its alias `--indirect off`) it names none.
#[test]
fn the_banner_names_the_probe_spacing_while_the_lamp_bounce_is_on() {
    // The table's own `probes` (the slots a key probes) is not the spacing.
    let spacing = |b: String| b.replace("table 2^22 probes 16", "table");
    for source in [&["--level"][..], &["--source", "showcase"][..], &[][..]] {
        let banner = |extra: &[&str]| {
            let mut v = source.to_vec();
            v.extend_from_slice(extra);
            let a = parse_args_from(argv(&v)).expect("parses");
            spacing(
                a.start_modes(0.0)
                    .banner(0.0)
                    .expect("the banner shows at the start"),
            )
        };
        let default = banner(&[]);
        assert!(
            default.contains("lamp bounce on") && default.contains("probes 16"),
            "the lamp's bounce is on by default: {source:?}: {default}"
        );
        for flag in ["--lamp-bounce", "--indirect"] {
            let off = banner(&[flag, "off"]);
            assert!(
                off.contains("lamp bounce off") && !off.contains("probes"),
                "{flag}: {off}"
            );
        }
        let wide = banner(&["--probe-spacing", "32", "--lamp-bounce", "on"]);
        assert!(
            wide.contains("probes 32") && !wide.contains("probes 16"),
            "{source:?}: {wide}"
        );
        assert!(wide.lines().all(|l| l.len() <= hud::BANNER_COLS));
        let off = banner(&["--probe-spacing", "32", "--lamp-bounce", "off"]);
        assert!(!off.contains("probes"), "{source:?}: {off}");
    }
    let a = parse_args_from(argv(&["--probe-spacing", "32"])).expect("parses");
    let mut m = a.start_modes(0.0);
    m.press(hud::ModeKey::LampBounce, 1000.0);
    let off = spacing(m.banner(1000.0).unwrap());
    assert!(off.contains("lamp bounce off") && !off.contains("probes"));
    m.press(hud::ModeKey::LampBounce, 2000.0);
    assert!(spacing(m.banner(2000.0).unwrap()).contains("probes 32"));
}

/// The start's banner is `look face, far normal 4..8` over the lighting modes,
/// `--far-normal` shows and takes the look's far normal, and `N`'s cycle reaches every
/// preset from there.
#[test]
fn the_start_banner_names_the_look_and_n_cycles() {
    let a = parse_args_from(argv(&[])).unwrap();
    let banner = a.start_modes(0.0).banner(0.0).expect("the start shows it");
    let mut bare = hud::Modes::new(a.start_lighting(), a.light.resolution, 0.0)
        .with_source(a.mode.source_name())
        .with_probe_spacing(a.light.probe_spacing);
    bare.set_auto(a.light.adaptation, a.start_auto());
    let lighting = bare.banner(0.0).expect("the start shows it");
    assert_eq!(banner, format!("look face, far normal 4..8\n{lighting}"));
    let knob = parse_args_from(argv(&["--far-normal", "3,10"])).unwrap();
    assert_eq!(knob.start_look().far_normal, [3.0, 10.0]);
    assert_eq!(
        look_line(&knob.start_look(), &knob.look.preset),
        "look face (normals face, face mix 0, far normal 3..10), view shaded"
    );
    let banner = knob.start_modes(0.0).banner(0.0).unwrap();
    assert!(
        banner.starts_with("look face, far normal 3..10\n"),
        "{banner}"
    );
    for bad in ["6,2", "2,2", "-1,2", "2", "x,6"] {
        assert!(
            parse_args_from(argv(&["--far-normal", bad])).is_err(),
            "{bad}"
        );
    }
    let mut seen = vec![a.look.preset.name];
    let mut p = a.look.preset.next(true);
    while p.name != a.look.preset.name {
        seen.push(p.name);
        p = p.next(true);
    }
    assert_eq!(seen, ["face", "field", "trilinear8", "gradient27"]);
}

/// The banner and the look line name the far normal in face mode only, the one mode that
/// reads it: from the default `N` reads `far normal off` in trilinear8 and gradient27 and
/// `far normal 4..8` back in face, and `--look field --far-normal 2,6` starts off.
#[test]
fn the_far_normal_is_named_in_face_mode_only() {
    let a = parse_args_from(argv(&[])).unwrap();
    let mut modes = a.start_modes(0.0);
    let n = TOGGLES
        .iter()
        .find(|t| t.name == "look")
        .expect("N is a toggle");
    let mut t = 5000.0;
    for (name, far) in [
        ("trilinear8", "off"),
        ("gradient27", "off"),
        ("face", "4..8"),
    ] {
        (n.flip)(&mut modes, t);
        let (preset, look) = (modes.preset, modes.look);
        assert_eq!(look.far_normal, [4.0, 8.0], "{name}");
        let banner = modes.banner(t).expect("a press shows the banner");
        assert!(
            banner.starts_with(&format!("look {name}, far normal {far}\n")),
            "{banner}"
        );
        assert_eq!(
            look_line(&look, &preset),
            format!("look {name} (normals {name}, face mix 0, far normal {far}), view shaded")
        );
        t += 5000.0;
    }
    let field = parse_args_from(argv(&["--look", "field", "--far-normal", "2,6"])).unwrap();
    assert_eq!(field.start_look().far_normal, [2.0, 6.0]);
    assert_eq!(
        look_line(&field.start_look(), &field.look.preset),
        "look field (normals contour, face mix 0, far normal off), view shaded"
    );
    let banner = field.start_modes(0.0).banner(0.0).unwrap();
    assert!(
        banner.starts_with("look field, far normal off\n"),
        "{banner}"
    );
}

/// A benchmark path runs with the headlamp off and its `run.json` says so: the tester's
/// arguments for each benchmark scene (the scene's flags, the mode's `--lamp off`, `--tester`,
/// the path) start the lamp off, the help panel's first row and the plain banner name `lamp
/// off`, and the report written at exit reads `"lamp": "off"`. A tour keeps the lamp on in the
/// cave and the castle, and `--lamp on` turns it back on for a benchmark.
#[test]
fn a_benchmark_path_runs_with_the_lamp_off_and_its_report_says_so() {
    use crate::path::{CameraPath, PathMode, Scene};
    use crate::report::json::{self, Value};
    let path_args = |scene: Scene, mode: PathMode, extra: &[&str]| {
        let mut a: Vec<&str> = scene.flags().iter().chain(mode.flags()).copied().collect();
        a.extend(["--path", scene.name(), "--path-mode", mode.name()]);
        a.extend(extra);
        parse_args_from(argv(&a)).unwrap_or_else(|e| panic!("{a:?}: {e}"))
    };
    let lamp_of = |a: &Args| a.start_modes(0.0).lighting.lamp;
    for scene in Scene::ALL {
        let bench = path_args(scene, PathMode::Bench, &["--tester"]);
        assert!(!bench.light.lamp() && !lamp_of(&bench), "{scene:?} bench");
        let tour = path_args(scene, PathMode::Tour, &["--tester"]);
        assert_eq!(
            tour.light.lamp(),
            scene != Scene::Showcase,
            "{scene:?} tour"
        );
        assert_eq!(lamp_of(&tour), scene != Scene::Showcase, "{scene:?} tour");
    }
    // The default holds without the mode's flags too, and `--lamp on` wins over it.
    let bare = parse_args_from(argv(&["--path", "cave"])).expect("parses");
    assert!(!bare.light.lamp());
    let lit = parse_args_from(argv(&["--path", "cave", "--lamp", "on"])).expect("parses");
    assert!(lit.light.lamp());

    let dir = std::env::temp_dir().join(format!("svet-bench-lamp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let report = dir.display().to_string();
    let args = path_args(
        Scene::Cave,
        PathMode::Bench,
        &["--tester", "--report", &report],
    );
    let path = CameraPath::named("cave").expect("the cave path");
    let mut run = bare_run(carved_rock());
    run.modes = args.start_modes(0.0);
    run.modes.upscaler = hud::UpscalerBanner::new(hud::UpscalerMode::Off, "1");
    run.args = args;
    run.player = Some(PathPlayer::new(path, PathMode::Bench));
    run.recorder = Some(Recorder::for_path(path.duration_s()));
    run.start = now();
    run.help = true;
    let banner = run.banner().expect("the banner shows");
    let panel = banner
        .lines()
        .find(|l| l.starts_with("scene "))
        .unwrap_or("");
    assert_eq!(panel, "scene cave, benchmark, lamp off", "{banner}");
    run.args.tester = false;
    let banner = run.banner().expect("the banner shows");
    assert!(
        banner.contains("lamp off") && !banner.contains("lamp on"),
        "{banner}"
    );

    run.write_report();
    let text = std::fs::read_to_string(dir.join("run.json")).expect("run.json written");
    let _ = std::fs::remove_dir_all(&dir);
    let v = json::parse(&text).expect("run.json parses");
    assert_eq!(v.get("lamp").and_then(Value::as_str), Some("off"), "{text}");
    assert_eq!(v.get("scene").and_then(Value::as_str), Some("cave"));
    assert_eq!(v.get("mode").and_then(Value::as_str), Some("bench"));
}

/// Under `--model`, the chooser hands the model to placement mode, a click places it, the
/// right click leaves the mode, and `Right` and `Left` cycle the placed entity's clips and
/// `Down` pauses it, each named in the banner; the HUD's mode block carries the entity's clip
/// row and the viewer's own row, which the keys leave on its first clip.
#[test]
fn the_clip_keys_name_the_clip_and_the_pause() {
    let mut run = bare_run(carved_rock());
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/entities/walker/walker.rig");
    let mut viewer = crate::viewer::Viewer::open(&path, None).expect("walker.rig opens");
    let table = crate::walker::with_walker(&crate::cave_materials());
    assert_eq!(viewer.build_pending(&table), None);
    run.modes.walkers = std::mem::take(&mut run.modes.walkers).with_viewer(Some(viewer));
    let mut input = Input::default();
    let mut press = |run: &mut Run, key: KeyCode| {
        input.end_frame();
        input.key_up(key);
        input.key_down(key);
        run.handle_input(&input);
        run.banner().expect("a press shows the banner")
    };
    run.act(crate::toggles::Act::Picker);
    run.menu_act(crate::hud::menu::Activated::Choose {
        item: crate::entities::belt::Item::Entity(0),
    });
    let mut click = Input::default();
    click.button_down(svet_app::MouseButton::Left);
    run.handle_input(&click);
    assert!(run.banner().unwrap().contains("placed walker #1 (idle)"));
    let mut click = Input::default();
    click.button_down(svet_app::MouseButton::Right);
    run.handle_input(&click);
    assert!(!run.modes.walkers.belt.on);
    assert!(press(&mut run, KeyCode::ArrowRight).contains("walker #1 clip walk"));
    assert!(press(&mut run, KeyCode::ArrowDown).contains("walker #1 paused"));
    let mut rows = String::new();
    run.modes.write_rows(&mut rows);
    assert!(rows.contains("\nwalker #1 walk 0/30 paused\n"), "{rows}");
    assert!(rows.contains("model walker clip idle 0/60\n"), "{rows}");
    assert!(press(&mut run, KeyCode::ArrowLeft).contains("clip idle"));
    assert!(press(&mut run, KeyCode::ArrowDown).contains("walker #1 playing"));
}
