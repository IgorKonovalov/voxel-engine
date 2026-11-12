//! The parser's tests: each flag's words, its default and its refusals.

use super::*;
use crate::audio::CAVE_ACOUSTIC;
use crate::castle;
use crate::cli::parse_args_from;
use crate::fly::CutawayMode;
use crate::hud::{self};
use crate::run::hud_feed::look_line;
use crate::setup::{default_workers, shell_config};
use crate::showcase::SHOTS;
use crate::test_support::argv;
use crate::{LogicChoice, LookPreset, RESIDENCY};
use vxe_app::Pacing;
use vxe_render::UpscalerKind;
use vxe_render::{
    AutoExposure, Exposure, HistoryKill, LightingResolution, NormalMode, RenderScale, ShadowWalk,
};

/// `--pace` takes `free` (no frame callback), the default, and `compositor`; another word
/// or a missing value is refused.
#[test]
fn the_pace_flag_parses_both_words() {
    let a = parse_args_from(argv(&[])).expect("no flags parse");
    assert_eq!(a.pace, Pacing::Unthrottled);
    let a = parse_args_from(argv(&["--pace", "free"])).expect("--pace free parses");
    assert_eq!(a.pace, Pacing::Unthrottled);
    assert!(!a.pace.frame_callback());
    let a = parse_args_from(argv(&["--pace", "compositor"])).expect("--pace compositor parses");
    assert_eq!(a.pace, Pacing::Compositor);
    assert!(a.pace.frame_callback());
    let e = parse_args_from(argv(&["--pace", "vsync"]))
        .err()
        .expect("vsync is refused");
    assert!(e.contains("free or compositor"), "{e}");
    assert!(parse_args_from(argv(&["--pace"])).is_err());
}

/// `--workers N` sets the streamer's workers (unset: `default_workers`, one fewer than
/// the logical cores and at least 1); 0, a non-number or a missing value is refused.
#[test]
fn the_workers_flag_parses_and_refuses_zero() {
    let a = parse_args_from(argv(&[])).expect("no flags parse");
    assert_eq!(a.workers, None);
    let cores = std::thread::available_parallelism().map_or(1, usize::from);
    assert_eq!(default_workers(), cores.saturating_sub(1).max(1));
    let a = parse_args_from(argv(&["--workers", "14"])).expect("--workers 14 parses");
    assert_eq!(a.workers, Some(14));
    let a = parse_args_from(argv(&["--workers", "1"])).expect("--workers 1 parses");
    assert_eq!(a.workers, Some(1));
    let e = parse_args_from(argv(&["--workers", "0"]))
        .err()
        .expect("0 workers is refused");
    assert!(e.contains("at least 1"), "{e}");
    assert!(parse_args_from(argv(&["--workers", "many"])).is_err());
    assert!(parse_args_from(argv(&["--workers"])).is_err());
}

/// `--sync-ms MS` sets the mirror's upload time per frame (default none); `--sync-ms 0`
/// is no time budget; a non-number or a missing value is refused.
#[test]
fn the_sync_ms_flag_parses_and_zero_is_no_budget() {
    let ms = std::time::Duration::from_millis;
    let a = parse_args_from(argv(&[])).expect("no flags parse");
    assert_eq!(a.sync_time, None);
    let a = parse_args_from(argv(&["--sync-ms", "5"])).expect("--sync-ms 5 parses");
    assert_eq!(a.sync_time, Some(ms(5)));
    let a = parse_args_from(argv(&["--sync-ms", "0"])).expect("--sync-ms 0 parses");
    assert_eq!(a.sync_time, None);
    assert!(parse_args_from(argv(&["--sync-ms", "-1"])).is_err());
    assert!(parse_args_from(argv(&["--sync-ms"])).is_err());
}

/// The flags of the per-face table, the reference and the cuts are refused, each naming Plan
/// 0059 and the flag; with none of them the run parses and its frame takes the store.
#[test]
fn the_flags_of_the_deleted_table_are_refused_naming_plan_0059() {
    for flags in [
        &["--light", "table"][..],
        &["--light", "baked"],
        &["--light", "queue"],
        &["--cut", "lattice:64"],
        &["--show", "table"],
        &["--reference"],
        &["--bake-file", "a.bin"],
        &["--queue-file", "a.bin"],
        &["--bake-radius", "2"],
        &["--blend", "off"],
        &["--preload", "off"],
        &["--preload-claim", "16"],
    ] {
        let Err(e) = parse_args_from(argv(flags)) else {
            panic!("{flags:?} is refused");
        };
        assert!(e.contains("Plan 0059"), "{flags:?}: {e}");
        assert!(e.starts_with(flags[0]), "{flags:?}: {e}");
        let mut level = vec!["--level"];
        level.extend_from_slice(flags);
        assert!(parse_args_from(argv(&level)).is_err(), "{level:?}");
    }
    assert!(parse_args_from(argv(&["--level"])).is_ok());
}

#[test]
fn residency_parses_two_radii() {
    let r = parse_args_from(argv(&["--residency", "14,22"]))
        .unwrap()
        .residency;
    assert_eq!(
        (r.r_full, r.r_lod, r.hysteresis),
        (14, 22, RESIDENCY.hysteresis)
    );
    assert_eq!(parse_args_from(argv(&[])).unwrap().residency, RESIDENCY);
    for bad in ["14", "0,4", "8,6", "a,b"] {
        assert!(
            parse_args_from(argv(&["--residency", bad])).is_err(),
            "{bad}"
        );
    }
    assert_eq!(
        parse_args_from(argv(&["--mirror-budget", "1024"]))
            .unwrap()
            .mirror_budget,
        Some(1 << 30)
    );
}

#[test]
fn walk_mode_is_refused_while_streaming() {
    assert_eq!(
        parse_args_from(argv(&["--walk"])).err().as_deref(),
        Some("--walk: walk mode needs --level (the coarse grid is not wired to streaming)")
    );
    let a = parse_args_from(argv(&["--walk", "--level"])).expect("--walk --level parses");
    assert!(a.walk && a.level);
    assert_eq!(walk_allowed(true), Err(WALK_NEEDS_LEVEL));
    assert_eq!(walk_allowed(false), Ok(()));
}

/// `--save` and `--replay` take a path, need the bounded level, and do not combine.
#[test]
fn save_and_replay_need_the_level() {
    for flag in ["--save", "--replay"] {
        assert_eq!(
            parse_args_from(argv(&[flag, "a.log"])).err().as_deref(),
            Some(SAVE_NEEDS_LEVEL)
        );
    }
    let a = parse_args_from(argv(&["--level", "--bench", "1", "--census"]))
        .expect("--level --bench 1 --census parses");
    assert!(a.census && a.bench == 1);
    let a = parse_args_from(argv(&["--level", "--bench", "1", "--save", "a.log"]))
        .expect("--level --bench 1 --save parses");
    assert_eq!(a.save.as_deref(), Some(std::path::Path::new("a.log")));
    assert_eq!(a.bench, 1);
    assert!(parse_args_from(argv(&["--level", "--save", "a", "--replay", "b"])).is_err());
    assert!(parse_args_from(argv(&["--level", "--replay"])).is_err());
}

/// `--logic` takes `native`, `wasm` or `wasm=PATH`; without the `wasm` feature `wasm` is
/// refused with a message naming the feature.
#[test]
fn logic_selects_native_or_the_component() {
    let a = parse_args_from(argv(&[])).expect("no arguments parse");
    assert_eq!(a.logic, LogicChoice::Native);
    assert!(parse_args_from(argv(&["--logic", "lua"])).is_err());
    assert!(parse_args_from(argv(&["--logic"])).is_err());
    let wasm = parse_args_from(argv(&["--logic", "wasm=x.wasm"]));
    if cfg!(feature = "wasm") {
        assert_eq!(
            wasm.expect("parses").logic,
            LogicChoice::Wasm("x.wasm".into())
        );
    } else {
        let e = wasm.err().expect("refused without the feature");
        assert!(e.contains("--features wasm"), "{e}");
    }
}

/// `--exposure` takes `auto` (the default) or a positive number; `--adaptation` a preset
/// (eye by default); `--adapt` four numbers replacing the preset's constants and floor;
/// `--metering` centre (the default) or average. Each refuses anything else: `--adapt`
/// with three numbers, a negative constant or a negative floor.
#[test]
fn exposure_flags_parse_and_refuse_bad_values() {
    let d = parse_args_from(argv(&[])).unwrap();
    assert_eq!(
        d.start_lighting().exposure,
        Exposure::Auto(AutoExposure::eye())
    );
    let a = parse_args_from(argv(&[
        "--exposure",
        "2.5",
        "--adaptation",
        "symmetric",
        "--adapt",
        "0.1,0.5,2,0.01",
        "--metering",
        "average",
    ]))
    .unwrap();
    assert_eq!(a.start_lighting().exposure, Exposure::Manual(2.5));
    assert_eq!(a.adaptation, hud::Adaptation::Symmetric);
    let auto = a.start_auto();
    assert_eq!(
        [auto.tau_light, auto.tau_cone, auto.tau_rod, auto.cone_floor],
        [0.1, 0.5, 2.0, 0.01]
    );
    assert_eq!(auto.metering, vxe_render::Metering::Average);
    assert_eq!(auto.key, 0.18);
    let i = parse_args_from(argv(&["--exposure", "auto", "--adaptation", "instant"])).unwrap();
    assert_eq!(
        i.start_lighting().exposure,
        Exposure::Auto(AutoExposure::instant())
    );
    for (args, err) in [
        (
            &["--exposure", "bright"][..],
            "--exposure takes auto or a positive number, not bright",
        ),
        (
            &["--exposure", "0"],
            "--exposure takes auto or a positive number, not 0",
        ),
        (
            &["--adaptation", "cat"],
            "--adaptation takes eye, symmetric or instant, not cat",
        ),
        (
            &["--adapt", "0.15,1,4"],
            "--adapt takes LIGHT,CONE,ROD,FLOOR: four numbers, none negative, not 0.15,1,4",
        ),
        (
            &["--adapt", "0.15,-1,4,0.01"],
            "--adapt takes LIGHT,CONE,ROD,FLOOR: four numbers, none negative, not 0.15,-1,4,0.01",
        ),
        (
            &["--adapt", "0.15,1,4,-0.01"],
            "--adapt takes LIGHT,CONE,ROD,FLOOR: four numbers, none negative, not 0.15,1,4,-0.01",
        ),
        (
            &["--metering", "spot"],
            "--metering takes centre or average, not spot",
        ),
    ] {
        assert_eq!(
            parse_args_from(argv(args)).err().as_deref(),
            Some(err),
            "{args:?}"
        );
    }
}

/// `--pad` takes `on` or `off`, defaults on, and refuses anything else; headless and
/// the autopilot never read a pad.
#[test]
fn pad_parses_on_and_off() {
    let a = parse_args_from(argv(&[])).expect("no arguments parse");
    assert!(a.pad && a.wants_pad());
    let on = parse_args_from(argv(&["--pad", "on"])).expect("on parses");
    assert!(on.pad);
    let off = parse_args_from(argv(&["--pad", "off"])).expect("off parses");
    assert!(!off.pad && !off.wants_pad());
    let e = parse_args_from(argv(&["--pad", "x"]))
        .err()
        .expect("x refused");
    assert_eq!(e, "--pad takes on or off, not x");
    assert!(parse_args_from(argv(&["--pad"])).is_err());
    let amd = parse_args_from(argv(&["--headless", "--adapter", "AMD"])).unwrap();
    assert_eq!(amd.adapter.as_deref(), Some("AMD"));
    assert!(parse_args_from(argv(&["--adapter"])).is_err());
}

/// The window's shell carries `--adapter`'s substring, and the selector picks the first
/// adapter whose name contains it.
#[test]
fn the_window_takes_the_adapter_flag() {
    let args = parse_args_from(argv(&["--adapter", "Radeon"])).unwrap();
    let config = shell_config(&args, [640, 360], vxe_app::Pacing::Unthrottled);
    assert_eq!(config.adapter.as_deref(), Some("Radeon"));
    assert_eq!(config.size, [640, 360]);
    let none = parse_args_from(argv(&[])).unwrap();
    assert_eq!(
        shell_config(&none, [640, 360], vxe_app::Pacing::Unthrottled).adapter,
        None
    );
    let names = [
        "NVIDIA GeForce RTX 3080 Laptop GPU",
        "AMD Radeon Graphics (RADV RENOIR)",
        "llvmpipe (LLVM 19.1.7, 256 bits)",
        "AMD Radeon Graphics (RADV RENOIR) copy",
    ]
    .map(str::to_owned);
    assert_eq!(vxe_app::pick_adapter(&names, "Radeon"), Some(1));
    assert_eq!(vxe_app::pick_adapter(&names, "NVIDIA"), Some(0));
    assert_eq!(vxe_app::pick_adapter(&names, "llvmpipe"), Some(2));
    assert_eq!(vxe_app::pick_adapter(&names, "Intel"), None);
    let headless = parse_args_from(argv(&["--pad", "on", "--headless"])).unwrap();
    assert!(!headless.wants_pad());
    let pilot = parse_args_from(argv(&["--autopilot"])).unwrap();
    assert!(!pilot.wants_pad());
}

/// `--castle` streams: the bounded level is too short for it, the vault's origin is
/// inside its box, and `--room` builds its own world. `--castle-out` implies it.
#[test]
fn castle_refuses_the_level_the_vault_and_the_room() {
    assert!(parse_args_from(argv(&["--castle"])).unwrap().castle);
    for other in [&["--level"][..], &["--vault"], &["--room", "12"]] {
        let mut a = vec!["--castle"];
        a.extend_from_slice(other);
        assert!(parse_args_from(argv(&a)).is_err(), "{a:?}");
    }
    let a = parse_args_from(argv(&["--castle-shot", "nave"])).unwrap();
    assert!(a.castle);
    assert_eq!(a.castle_shot, castle::CastleShot::Nave);
    assert!(parse_args_from(argv(&["--castle-shot", "keep"])).is_err());
    let a = parse_args_from(argv(&["--castle-out", "c.vox"])).unwrap();
    assert!(a.castle);
    assert_eq!(a.castle_out.as_deref(), Some(std::path::Path::new("c.vox")));
}

/// Every edit is in the log now, so `--vault` combines with `--save` and `--replay` (the
/// room placed at generation is not an op; `--replay` takes `--vault` to regenerate it).
#[test]
fn vault_combines_with_save_and_replay() {
    for flag in ["--save", "--replay"] {
        let a = parse_args_from(argv(&["--level", "--vault", flag, "a.log"]))
            .unwrap_or_else(|e| panic!("--level --vault {flag} parses: {e}"));
        assert!(a.level && a.vault);
    }
}

/// `--room` takes metres and implies the room line; `--mic` implies `--audio`;
/// `--absorption` is checked; a room builds its own world.
#[test]
fn room_mic_and_absorption_parse() {
    let a = parse_args_from(argv(&["--room", "12"])).expect("--room 12 parses");
    assert_eq!(a.room, Some(120));
    assert!(a.room_line && !a.audio);
    assert_eq!(a.absorption, CAVE_ACOUSTIC[0]);
    let a =
        parse_args_from(argv(&["--mic", "--absorption", "0.5", "--room-line"])).expect("parses");
    assert!(a.mic && a.audio && a.room_line);
    assert_eq!(a.absorption, 0.5);
    assert!(parse_args_from(argv(&["--absorption", "1.5"])).is_err());
    assert!(parse_args_from(argv(&["--room", "0.5"])).is_err());
    assert!(parse_args_from(argv(&["--room", "12", "--level"])).is_err());
    assert!(parse_args_from(argv(&["--room", "4", "--walk"])).is_ok());
}

/// FSR 2's emitter cap is gone (Plan 0056's verdict: a fault): `--emitter-cap` is an
/// unknown argument.
#[test]
fn the_emitter_cap_is_gone() {
    assert_eq!(
        parse_args_from(argv(&["--emitter-cap", "1"]))
            .err()
            .as_deref(),
        Some("unknown argument --emitter-cap")
    );
}

/// `--source` picks the cave, the level (as `--level` does) or the showcase; the showcase
/// starts with the lamp off and a light radius of 19 unless the flags set them, and
/// refuses what it cannot take.
#[test]
fn source_showcase_parses_with_the_lamp_off_and_refuses_what_it_cannot_take() {
    let a = parse_args_from(argv(&["--source", "showcase"])).expect("parses");
    assert!(a.showcase && !a.level && !a.lamp);
    assert_eq!(a.light_radius, 19);
    let a = parse_args_from(argv(&[
        "--source",
        "showcase",
        "--lamp",
        "on",
        "--light-radius",
        "6",
    ]))
    .expect("parses");
    assert!(a.lamp);
    assert_eq!(a.light_radius, 6);
    let a = parse_args_from(argv(&["--source", "showcase", "--walk"])).expect("walks");
    assert!(a.walk && a.showcase);
    assert_eq!(a.shot, SHOTS[0]);
    for s in SHOTS {
        let a = parse_args_from(argv(&["--source", "showcase", "--shot", s.name])).unwrap();
        assert_eq!(a.shot, s);
        assert_eq!(
            a.start_lighting().exposure,
            Exposure::Manual(s.exposure_milli as f32 / 1000.0)
        );
    }
    assert_eq!(
        parse_args_from(argv(&["--source", "showcase", "--shot", "keep"]))
            .err()
            .as_deref(),
        Some("--shot takes hall, chasm, crystals, not keep")
    );
    assert!(parse_args_from(argv(&["--level", "--shot", "hall"])).is_err());
    let level = parse_args_from(argv(&["--source", "level"])).expect("parses");
    assert!(level.level && !level.showcase && level.lamp);
    assert_eq!(level.light_radius, vxe_render::DEFAULT_LIGHT_RADIUS);
    let cave = parse_args_from(argv(&["--level", "--source", "cave"])).expect("parses");
    assert!(!cave.level && !cave.showcase && cave.lamp);
    assert_eq!(
        parse_args_from(argv(&["--source", "outdoor"]))
            .err()
            .as_deref(),
        Some("--source takes cave, level or showcase, not outdoor")
    );
    for extra in [
        &["--save", "a.log"][..],
        &["--replay", "a.log"],
        &["--vault"],
        &["--autopilot"],
        &["--room", "12"],
        &["--castle"],
    ] {
        let mut v = vec!["--source", "showcase"];
        v.extend_from_slice(extra);
        let e = parse_args_from(argv(&v)).err().expect("refused");
        assert!(e.starts_with("--source showcase builds"), "{extra:?}: {e}");
    }
}

#[test]
fn cutaway_parses_auto_on_and_off() {
    assert_eq!(
        parse_args_from(argv(&[])).unwrap().cutaway,
        CutawayMode::Auto
    );
    for (name, mode) in [
        ("auto", CutawayMode::Auto),
        ("on", CutawayMode::On),
        ("off", CutawayMode::Off),
    ] {
        assert_eq!(
            parse_args_from(argv(&["--cutaway", name])).unwrap().cutaway,
            mode
        );
    }
    assert_eq!(
        parse_args_from(argv(&["--cutaway", "x"])).err().as_deref(),
        Some("--cutaway takes auto, on or off, not x")
    );
}

/// Every lighting flag parses its values and refuses an unknown one.
#[test]
fn lighting_flags_parse_and_refuse_unknown_values() {
    let parse = |a: &[&str]| parse_args_from(argv(a));
    let a = parse(&[
        "--emissive-ore",
        "off",
        "--lamp",
        "off",
        "--filter",
        "off",
        "--lamp-bounce",
        "off",
    ])
    .unwrap();
    assert!(!a.emissive_ore && !a.lamp && !a.filter && !a.lamp_bounce);
    let a = parse(&[
        "--shadow-walk",
        "coarse",
        "--history",
        "gradient",
        "--lighting",
        "full",
        "--light-radius",
        "6",
    ])
    .unwrap();
    assert_eq!(a.shadow_walk, ShadowWalk::Coarse);
    assert_eq!(a.history, HistoryKill::Gradient);
    assert_eq!(a.lighting, LightingResolution::Full);
    assert_eq!(a.light_radius, 6);
    let d = parse(&[]).unwrap();
    assert!(d.emissive_ore && d.lamp && d.filter && d.lamp_bounce);
    assert!(
        d.start_lighting().lamp_bounce,
        "the lamp's bounce is on by default"
    );
    assert_eq!(d.probe_spacing, 16);
    assert_eq!(parse(&["--probe-spacing", "8"]).unwrap().probe_spacing, 8);
    let p = parse(&["--probe-rays", "32", "--probe-slices", "16"]).unwrap();
    assert_eq!(
        (p.start_lighting().rays(), p.start_lighting().slices()),
        (32, 16)
    );
    assert!(parse(&["--probe-slices", "12"]).is_err());
    assert_eq!(
        (d.shadow_walk, d.history, d.lighting, d.light_radius),
        (
            ShadowWalk::Coarse,
            HistoryKill::Edits,
            LightingResolution::Half,
            4
        )
    );
    // The line the fly-through prints at the start with no lighting flags, and under the table.
    assert_eq!(
        d.start_modes(0.0).line(),
        "lighting half, store read normal, weights face, shadow walk coarse, history edits, lamp on, lamp bounce on at 64 rays 1/8, probes 16, filter on, ore pulse off, exposure auto eye +0.00 EV"
    );
    for (flag, err) in [
        ("--emissive-ore", "--emissive-ore takes on or off, not x"),
        ("--lamp", "--lamp takes on or off, not x"),
        ("--lamp-bounce", "--lamp-bounce takes on or off, not x"),
        ("--indirect", "--indirect takes on or off, not x"),
        (
            "--probe-spacing",
            "--probe-spacing takes 4 to 64 voxels, not x",
        ),
        ("--probe-rays", "--probe-rays takes 1 to 64 rays, not x"),
        ("--probe-slices", "--probe-slices takes 8, 16 or 32, not x"),
        ("--filter", "--filter takes on or off, not x"),
        ("--shadow-walk", "--shadow-walk takes full or coarse, not x"),
        ("--history", "--history takes edits or gradient, not x"),
        ("--lighting", "--lighting takes full or half, not x"),
        (
            "--light-radius",
            "--light-radius takes 0 to 64 chunks, not x",
        ),
    ] {
        assert_eq!(parse(&[flag, "x"]).err().as_deref(), Some(err), "{flag}");
    }
    // The steady spike's flags and the screen path's are refused, naming Plan 0057.
    for flag in [
        "--steady",
        "--cache-read",
        "--emissive-guard",
        "--reuse-m-cap",
        "--sampler",
        "--emissive-history",
    ] {
        let err = parse(&[flag, "b"]).err().unwrap_or_default();
        assert!(
            err.starts_with(flag) && err.contains("Plan 0057"),
            "{flag}: {err}"
        );
    }
}

/// `--internal-scale`, `--jitter` and `--sharpen` parse their values, default to 1.5, off
/// and 0.5, and refuse an unknown value.
#[test]
fn internal_scale_and_jitter_parse_and_refuse_unknown_values() {
    let parse = |a: &[&str]| parse_args_from(argv(a));
    let d = parse(&[]).unwrap();
    assert_eq!(
        (d.internal_scale, d.jitter, d.jitter_set),
        (RenderScale::OneHalf, false, false)
    );
    assert_eq!(d.sharpen, 0.5);
    assert_eq!(parse(&["--sharpen", "0"]).unwrap().sharpen, 0.0);
    assert_eq!(parse(&["--sharpen", "0.8"]).unwrap().sharpen, 0.8);
    for (name, scale) in [
        ("1", RenderScale::One),
        ("1.5", RenderScale::OneHalf),
        ("2", RenderScale::Two),
    ] {
        let a = parse(&["--internal-scale", name, "--jitter", "on"]).unwrap();
        assert_eq!((a.internal_scale, a.jitter), (scale, true));
    }
    assert!(!parse(&["--jitter", "off"]).unwrap().jitter);
    assert!(parse(&["--jitter", "on"]).unwrap().jitter);
    assert_eq!(
        RenderScale::OneHalf.internal([1920, 1080]),
        [1280, 720],
        "1.5 renders 720p for 1080p"
    );
    assert_eq!(RenderScale::OneHalf.internal([384, 216]), [256, 144]);
    assert_eq!(RenderScale::Two.internal([1920, 1080]), [960, 540]);
    for (flag, v, err) in [
        (
            "--internal-scale",
            "3",
            "--internal-scale takes 1, 1.5 or 2, not 3",
        ),
        (
            "--internal-scale",
            "1.0",
            "--internal-scale takes 1, 1.5 or 2, not 1.0",
        ),
        ("--jitter", "x", "--jitter takes on or off, not x"),
        ("--sharpen", "1.5", "--sharpen takes 0 to 1, not 1.5"),
        ("--sharpen", "x", "--sharpen takes 0 to 1, not x"),
        (
            "--upscaler",
            "fsr",
            "--upscaler takes off, own or fsr2, not fsr",
        ),
    ] {
        assert_eq!(parse(&[flag, v]).err().as_deref(), Some(err), "{flag} {v}");
    }
}

/// `--upscaler` picks the frame's upscaler, scale and jitter: `off` is scale 1 with the
/// jitter off whatever `--internal-scale` says; `own` jitters unless `--jitter off`; FSR 2
/// takes `--internal-scale` and `--sharpen`. Without it the scale and `--jitter` decide:
/// no flag is `fsr2` at 1.5 with sharpening 0.5; `--internal-scale 1` is Plan 0024's frame;
/// `--upscaler own` is ours at 1.5, jittered; `--jitter` with no `--upscaler` is ours.
#[test]
fn upscaler_picks_the_kind_scale_and_jitter() {
    let parse = |a: &[&str]| parse_args_from(argv(a));
    let settings = |a: &[&str]| {
        let args = parse(a).unwrap();
        let (kind, scale, jitter) = args.upscaler_settings(args.start_upscaler());
        (kind.name(), scale, jitter)
    };
    assert_eq!(settings(&[]), ("fsr2", RenderScale::OneHalf, false));
    let none = parse(&[]).unwrap();
    match none.upscaler_settings(none.start_upscaler()).0 {
        UpscalerKind::Fsr2(s) => assert_eq!(s.sharpness, 0.5),
        k => panic!("{k:?}"),
    }
    assert_eq!(
        settings(&["--upscaler", "own"]),
        ("own", RenderScale::OneHalf, true)
    );
    assert_eq!(
        settings(&["--internal-scale", "2"]),
        ("fsr2", RenderScale::Two, false)
    );
    assert_eq!(
        settings(&["--internal-scale", "1"]),
        ("off", RenderScale::One, false)
    );
    assert_eq!(
        settings(&["--upscaler", "off"]),
        ("off", RenderScale::One, false)
    );
    assert_eq!(
        settings(&["--internal-scale", "1", "--jitter", "on"]),
        ("own", RenderScale::One, true)
    );
    assert_eq!(
        settings(&["--internal-scale", "1.5", "--jitter", "off"]),
        ("own", RenderScale::OneHalf, false)
    );
    assert_eq!(
        settings(&["--upscaler", "off", "--internal-scale", "1.5"]),
        ("off", RenderScale::One, false)
    );
    assert_eq!(
        settings(&["--upscaler", "own", "--internal-scale", "1.5"]),
        ("own", RenderScale::OneHalf, true)
    );
    assert_eq!(
        settings(&["--upscaler", "own", "--jitter", "off"]),
        ("own", RenderScale::OneHalf, false)
    );
    assert_eq!(
        settings(&["--upscaler", "fsr2", "--internal-scale", "1.5"]),
        ("fsr2", RenderScale::OneHalf, false)
    );
    let args = parse(&["--upscaler", "fsr2", "--sharpen", "0.2"]).unwrap();
    match args.upscaler_settings(hud::UpscalerMode::Fsr2).0 {
        UpscalerKind::Fsr2(s) => assert_eq!(s.sharpness, 0.2),
        k => panic!("{k:?}"),
    }
    // Cycled to with `F10` from `off`, our upscaler jitters.
    let args = parse(&["--upscaler", "off"]).unwrap();
    assert!(args.upscaler_settings(hud::UpscalerMode::Own).2);
}

/// The owner's picks (Plan 0055 Phase 4): no flag starts in face normals with the far
/// normal at `[2, 6]`, in FSR 2 at scale 1.5 with sharpening 0.5.
#[test]
fn the_start_look_and_upscaler_are_the_owners_picks() {
    let a = parse_args_from(argv(&[])).unwrap();
    let look = a.start_look();
    assert_eq!(look.normal_mode, NormalMode::Face);
    assert_eq!(look.far_normal, [2.0, 6.0]);
    assert_eq!(
        look_line(&look, &a.look),
        "look face (normals face, face mix 0, far normal 2..6), view shaded"
    );
    let (kind, scale, jitter) = a.upscaler_settings(a.start_upscaler());
    assert_eq!(
        (kind.name(), scale, jitter),
        ("fsr2", RenderScale::OneHalf, false)
    );
    match kind {
        UpscalerKind::Fsr2(s) => assert_eq!(s.sharpness, 0.5),
        k => panic!("{k:?}"),
    }
    let over = parse_args_from(argv(&["--far-normal", "1,2"])).unwrap();
    assert_eq!(over.start_look().far_normal, [1.0, 2.0]);
}

/// No flag starts in the face look with no band slot, as `--look face` does, whose far
/// normal is off; `--look field` alone builds the band slots.
#[test]
fn the_default_look_is_face() {
    let a = parse_args_from(argv(&[])).unwrap();
    assert_eq!(a.look, LookPreset::DEFAULT);
    assert_eq!(a.look.normal_mode, NormalMode::Face);
    assert!(!a.field_surface());
    assert!(
        parse_args_from(argv(&["--look", "field"]))
            .unwrap()
            .field_surface()
    );
    for name in ["face", "trilinear8", "gradient27"] {
        assert!(
            !parse_args_from(argv(&["--look", name]))
                .unwrap()
                .field_surface(),
            "{name}"
        );
    }
    let c = parse_args_from(argv(&["--look", "face"])).unwrap();
    assert_eq!(
        look_line(&c.start_look(), &c.look),
        "look face (normals face, face mix 0, far normal off), view shaded"
    );
    assert_eq!(c.look, LookPreset::FACE);
    assert_eq!(
        parse_args_from(argv(&["--look", "plain"])).err().as_deref(),
        Some("--look takes face, field, trilinear8, gradient27, not plain")
    );
}

/// `--look` pins each preset by name with its parts on the startup line, an unknown name
/// lists the valid ones, and the part flag overrides the preset's part.
#[test]
fn look_presets_parse_by_name_with_their_parts() {
    let line = |v: &[&str]| {
        let a = parse_args_from(argv(v)).unwrap();
        look_line(&a.start_look(), &a.look)
    };
    assert_eq!(
        line(&["--look", "field"]),
        "look field (normals contour, face mix 0, far normal off), view shaded"
    );
    for p in LookPreset::ALL {
        let a = parse_args_from(argv(&["--look", p.name])).unwrap();
        assert_eq!(a.look, p, "{}", p.name);
        assert_eq!(a.start_look().normal_mode, p.normal_mode, "{}", p.name);
        assert_eq!(a.start_look().tuning, p.tuning, "{}", p.name);
    }
    assert_eq!(
        parse_args_from(argv(&["--look", "contour-soft"]))
            .err()
            .as_deref(),
        Some("--look takes face, field, trilinear8, gradient27, not contour-soft")
    );
    assert_eq!(
        line(&["--look", "field", "--face-mix", "0.25"]),
        "look field (normals contour, face mix 0.25, far normal off), view shaded"
    );
    // The plane attribute's settings are gone: their flags are unknown arguments.
    for flag in [
        "--plane-fit",
        "--plane-offset",
        "--fit-filter",
        "--shadow-push",
        "--normal-field",
        "--field-normal",
        "--attr-cap",
    ] {
        assert_eq!(
            parse_args_from(argv(&[flag, "on"])).err(),
            Some(format!("unknown argument {flag}")),
            "{flag}"
        );
    }
    assert!(parse_args_from(argv(&["--face-mix", "0.6"])).is_err());
    assert!(parse_args_from(argv(&["--look", "cubes-field"])).is_err());
}
