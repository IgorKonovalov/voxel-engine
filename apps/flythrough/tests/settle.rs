//! How fast the light settles after a fly-in, a lamp toggle, a dig and a fast yaw, the dots a
//! turn, a flight or a dig leaves, the world-keyed flicker, the probes inside rock and the
//! frame's cost in flight (`support::settle`), on the level's ore veins at `stillness`'s poses,
//! on the fly-through's default look (`LookPreset::DEFAULT`, FSR 2 at 1.5).
//!
//! The asserted tests run on the software adapter (lavapipe). The `*_readings` tests are
//! reading-only and `#[ignore]`d: they print readings and assert nothing a gate runs, on the
//! adapter `VXE_TEST_ADAPTER` picks (`support::reading_device`). Their runs narrow by
//! environment variables, each a comma list: `VXE_SETTLE_POSES` (patch, vein),
//! `VXE_SETTLE_EVENTS` (flyin, flyin-pinned, toggle, dig, yaw10, yaw30, yaw180),
//! `VXE_SETTLE_LAMP` and `VXE_SETTLE_INDIRECT` (on, off), `VXE_SETTLE_AUTO` (pinned, auto),
//! `VXE_SETTLE_UPSCALER` (up, s1, bridge) and `VXE_K_RUNS` (yaw, flight). `settle_readings`
//! takes each run's `N_step` from the composite's `sigma_view` in a `wander_readings` log on
//! the same adapter (`VXE_WANDER_LOG`); `VXE_SETTLE_DRY` set prints the `N_step` of each run's
//! first seed and runs no more.

mod support;

use support::settle::*;
use support::stillness::{Pose, VEIN_POSE, vein_level, vein_level_on};
use support::{SyncNow, reading_device, stillness};
use vxe_render::glam::Vec3;
use vxe_render::{Camera, Exposure, Frame, LightParams, Lighting, RenderScale};

/// Frames the window tests settle before their change.
const WINDOW_SETTLE: usize = 64;

fn window_frame(level: &stillness::VeinLevel, lamp: bool) -> Frame {
    let mut frame = Frame::new(&level.device, 64, 36);
    frame.set_lighting(Lighting {
        lamp,
        indirect: true,
        exposure: Exposure::Manual(VEIN_POSE.exposure),
        ..Lighting::default()
    });
    frame.set_dt(1.0 / 60.0);
    frame
}

/// Switching the lamp opens the probe volume's light-change window on the first frame after
/// the switch (every frame with indirect light on updates probes) and holds it for exactly
/// `WINDOW_FRAMES` (16) such frames, both ways.
#[test]
fn the_lamp_toggle_opens_the_light_window() {
    let Some(level) = vein_level() else {
        return;
    };
    let light = LightParams::headlamp();
    let camera = VEIN_POSE.camera();
    for from in [true, false] {
        let mut frame = window_frame(&level, from);
        for _ in 0..WINDOW_SETTLE {
            frame.step(&level.device, &level.queue, &level.gpu, &camera, &light);
        }
        assert!(
            !frame.probe_light_window(),
            "the window is closed after {WINDOW_SETTLE} still frames"
        );
        frame.set_lighting(Lighting {
            lamp: !from,
            ..frame.lighting()
        });
        let mut open = Vec::new();
        for _ in 0..32 {
            frame.step(&level.device, &level.queue, &level.gpu, &camera, &light);
            assert!(frame.traced_probe_rays() > 0, "every frame updates probes");
            open.push(frame.probe_light_window());
        }
        println!("lamp {from} -> {}: window per frame {open:?}", !from);
        let want: Vec<bool> = (0..32)
            .map(|i| i < vxe_render::probes::WINDOW_FRAMES as usize)
            .collect();
        assert_eq!(open, want, "lamp {from} -> {}", !from);
    }
}

/// A 90-degree yaw over 10 frames with the lamp on leaves the light-change window closed: the
/// lamp's pose is not in the window's key (the gap the settle spike reads).
#[test]
fn a_turn_with_the_lamp_on_opens_no_window() {
    let Some(level) = vein_level() else {
        return;
    };
    let light = LightParams::headlamp();
    let mut frame = window_frame(&level, true);
    let at = |deg: f32| Camera {
        yaw: (VEIN_POSE.yaw_deg + deg).to_radians(),
        ..VEIN_POSE.camera()
    };
    for _ in 0..WINDOW_SETTLE {
        frame.step(&level.device, &level.queue, &level.gpu, &at(0.0), &light);
    }
    let mut open = Vec::new();
    for i in 1..=30 {
        let deg = 90.0 * (i.min(10) as f32) / 10.0;
        frame.step(&level.device, &level.queue, &level.gpu, &at(deg), &light);
        open.push(frame.probe_light_window());
    }
    println!("yaw with the lamp on: window per frame {open:?}");
    assert!(open.iter().all(|&o| !o), "{open:?}");
}

/// The rig reads what the fly-through starts in: `LookPreset::DEFAULT`, FSR 2 at scale 1.5 with
/// sharpening 0.5, over the level's table with the ore emissive.
#[test]
fn the_rig_reads_the_default_look() {
    let Some(level) = vein_level() else {
        return;
    };
    let frame = make_frame(
        &level,
        &VEIN_POSE,
        Setting {
            lamp: true,
            indirect: true,
            auto: false,
            upscaler: true,
            bridge: false,
            steady: Steady::default(),
        },
    );
    let want = flythrough::LookPreset::DEFAULT;
    let look = frame.look();
    assert_eq!(look.normal_mode, want.normal_mode);
    assert_eq!(look.tuning.face_mix, want.tuning.face_mix);
    assert_eq!(look.far_normal, want.far_normal);
    assert_eq!(look.view, vxe_render::ViewMode::Shaded);
    assert_eq!(
        frame.upscaler_kind(),
        vxe_render::UpscalerKind::Fsr2(vxe_render::Fsr2Settings {
            sharpness: 0.5,
            ..vxe_render::Fsr2Settings::default()
        })
    );
    assert_eq!(frame.output_size(), OUTPUT);
    assert_eq!(frame.internal_size(), RenderScale::OneHalf.internal(OUTPUT));
    let ore = level
        .world
        .materials()
        .get(flythrough::cave::ORE)
        .expect("the level's table has the ore row");
    assert_eq!(ore.emission, flythrough::ORE_EMISSION);
}

/// `K` reads 0 over repeated frames, and one key's step reads its log2 over the keys and frames.
#[test]
fn the_flicker_metric_reads_zero_on_a_still_image() {
    let frame = |a: f32, b: f32| -> std::collections::HashMap<FaceKey, f32> {
        [(([1, 2, 3], 0u8), a), (([1, 2, 3], 1u8), b)]
            .into_iter()
            .collect()
    };
    let mut still = KeyFlicker::new();
    for _ in 0..4 {
        still.add(frame(0.25, 0.125), true);
    }
    assert_eq!(still.k(), 0.0, "repeated frames do not flicker");
    assert_eq!(still.p95(), 0.0);

    let mut step = KeyFlicker::new();
    step.add(frame(0.25, 0.125), true);
    step.add(frame(0.5, 0.125), true);
    // One of two keys moved one stop, over one pair of frames.
    assert_eq!(step.k(), 0.5);
    assert_eq!(step.p95(), 1.0);

    // A key absent from the frame before counts nothing.
    let mut new_key = KeyFlicker::new();
    new_key.add([(([0, 0, 0], 0u8), 0.5)].into_iter().collect(), true);
    new_key.add([(([9, 9, 9], 0u8), 0.5)].into_iter().collect(), true);
    assert_eq!(new_key.k(), 0.0);
    // Uncounted frames still set the previous frame.
    let mut windowed = KeyFlicker::new();
    windowed.add(frame(0.25, 0.125), false);
    windowed.add(frame(0.25, 0.25), true);
    assert_eq!(windowed.k(), 0.5);
}

/// A face key is the voxel behind the hit and its entry face, on each axis and sign.
#[test]
fn a_face_key_is_the_voxel_behind_the_hit() {
    let hit = |axis: u16| [0, 0, 0, 1 | ((axis + 1) << 3)];
    let p = [10.0, 20.25, 30.75, 5.0];
    let toward = Vec3::new(50.0, 0.0, 0.0);
    assert_eq!(
        face_key(toward, p, hit(0)),
        Some(([9, 20, 30], 1)),
        "a +x face: its voxel lies toward -x"
    );
    let against = Vec3::new(-50.0, 0.0, 0.0);
    assert_eq!(face_key(against, p, hit(0)), Some(([10, 20, 30], 0)));
    let q = [10.25, 20.75, 30.0, 5.0];
    assert_eq!(
        face_key(Vec3::new(0.0, 0.0, 0.0), q, hit(2)),
        Some(([10, 20, 30], 4))
    );
    assert_eq!(
        face_key(Vec3::new(0.0, 0.0, 99.0), q, hit(2)),
        Some(([10, 20, 29], 5))
    );
    assert_eq!(face_key(toward, p, [0, 0, 0, 0]), None, "a miss has no key");
    assert_eq!(
        face_key(toward, p, [0, 0, 0, 1]),
        None,
        "a ray from inside rock has none"
    );
}

/// Prints the adapter `VXE_TEST_ADAPTER` picks for the reading tests.
#[test]
#[ignore = "reading only: run with --ignored"]
fn reading_adapter() {
    let Some((_, _, name)) = reading_device(64 << 20) else {
        return;
    };
    println!("reading adapter: {name}");
}

fn list(var: &str, all: &[&str]) -> Vec<String> {
    match std::env::var(var) {
        Ok(v) if !v.is_empty() => v.split(',').map(str::to_owned).collect(),
        _ => all.iter().map(|s| (*s).to_owned()).collect(),
    }
}

fn poses() -> Vec<Pose> {
    let want = list("VXE_SETTLE_POSES", &["patch", "vein"]);
    POSES
        .into_iter()
        .filter(|p| want.iter().any(|w| w == p.name))
        .collect()
}

fn on_off(var: &str) -> Vec<bool> {
    list(var, &["on", "off"])
        .iter()
        .map(|s| s == "on")
        .collect()
}

fn settings() -> Vec<Setting> {
    let auto: Vec<bool> = list("VXE_SETTLE_AUTO", &["pinned", "auto"])
        .iter()
        .map(|s| s == "auto")
        .collect();
    let up = list("VXE_SETTLE_UPSCALER", &["up"]);
    let steady = list("VXE_STEADY", &["off"]);
    let mut out = Vec::new();
    for steady in steady.iter().map(|s| Steady::named(s)) {
        for up in &up {
            for &lamp in &on_off("VXE_SETTLE_LAMP") {
                for &indirect in &on_off("VXE_SETTLE_INDIRECT") {
                    for &a in &auto {
                        out.push(Setting {
                            lamp,
                            indirect,
                            auto: a,
                            upscaler: up != "s1",
                            bridge: up == "bridge",
                            steady,
                        });
                    }
                }
            }
        }
    }
    out
}

fn events(all: &[&str]) -> Vec<Event> {
    list("VXE_SETTLE_EVENTS", all)
        .iter()
        .map(|e| match e.as_str() {
            "flyin" => Event::FlyIn { pinned: false },
            "flyin-pinned" => Event::FlyIn { pinned: true },
            "toggle" => Event::Toggle,
            "dig" => Event::Dig,
            "yaw10" => Event::Yaw {
                frames: 10,
                degrees: 90,
            },
            "yaw30" => Event::Yaw {
                frames: 30,
                degrees: 90,
            },
            "yaw180" => Event::Yaw {
                frames: 20,
                degrees: 180,
            },
            other => panic!("unknown event {other}"),
        })
        .collect()
}

fn strip_tag() -> Option<String> {
    std::env::var("STILL_FILMSTRIP")
        .ok()
        .filter(|t| !t.is_empty())
}

fn reading_level() -> Option<stillness::VeinLevel> {
    vein_level_on(reading_device)
}

/// The composite's `sigma_view` per (pose, lamp, indirect, upscaler) from a `wander_readings`
/// log (`VXE_WANDER_LOG`), on the same adapter: the step settle's `N_step` comes from it.
fn wander_sigmas(adapter: &str) -> Vec<(String, f32)> {
    let path = std::env::var("VXE_WANDER_LOG")
        .expect("VXE_WANDER_LOG names a wander_readings log on this adapter");
    let text = std::fs::read_to_string(&path).expect("the wander log reads");
    let head = format!("reading[{adapter}]: wander ");
    text.lines()
        .filter_map(|l| {
            let rest = l.strip_prefix(&head)?;
            let (name, v) = rest.split_once(" composite: sigma_view ")?;
            let v: f32 = v.split_whitespace().next()?.parse().ok()?;
            Some((name.replace(" pinned", ""), v))
        })
        .collect()
}

fn sigma_of(sigmas: &[(String, f32)], pose: &Pose, lamp: bool, setting: Setting) -> f32 {
    let on = |b: bool| if b { "on" } else { "off" };
    let key = format!(
        "{} lamp {} indirect {} {}{}",
        pose.name,
        on(lamp),
        on(setting.indirect),
        setting.upscaler_name(),
        setting.steady.words()
    );
    sigmas
        .iter()
        .find(|(k, _)| *k == key)
        .unwrap_or_else(|| panic!("no wander reading for {key} in VXE_WANDER_LOG"))
        .1
}

/// The settle runs of `all` events (narrowed by the filters). `seeded`: each pinned run
/// repeats over `N_step` seeds and its Auto run over the same; otherwise one seed (the dots'
/// reading, with no wander log needed).
fn run_settle(all: &[&str], seeded: bool) {
    let Some(mut level) = reading_level() else {
        return;
    };
    let adapter = level.adapter.clone();
    // `VXE_SETTLE_DRY`: print each pinned run's `N_step` from its first seed, run no more.
    let dry = std::env::var("VXE_SETTLE_DRY").is_ok_and(|v| !v.is_empty());
    let sigmas = if seeded {
        wander_sigmas(&adapter)
    } else {
        Vec::new()
    };
    let tag = strip_tag();
    for pose in poses() {
        for event in events(all) {
            let mut pinned_strip: Option<Vec<Vec<u8>>> = None;
            for setting in settings() {
                #[allow(
                    clippy::disallowed_methods,
                    reason = "a reading test's frames a second, not simulation"
                )]
                let t0 = std::time::Instant::now();
                let r = settle_run(&mut level, &pose, setting, event, 0, tag.as_deref());
                if event == Event::Dig {
                    reset_world(&mut level);
                }
                let name = format!("{} {} {}", pose.name, event.name(), setting.name());
                let mut sum = SeedSum::new();
                sum.add(&r);
                let lamp_after = if event == Event::Toggle {
                    !setting.lamp
                } else {
                    setting.lamp
                };
                // The Auto run is one seed: it reads the exposure's own settle, which the eye's
                // smooth adaptation carries with little seed noise.
                let n = if !seeded || setting.auto {
                    Some(1)
                } else {
                    let (parts, _) = sum.settles();
                    let d = parts[0].1.d;
                    let sigma = sigma_of(&sigmas, &pose, lamp_after, setting);
                    let n = n_step(sigma, d);
                    println!(
                        "reading[{adapter}]: n_step {name}: composite single-seed D {d:+.4}, sigma_view {sigma:.4}, (30 sigma / |D|)^2 {:.2}, N_step {}",
                        (30.0 * sigma / d.abs().max(1e-9)).powi(2),
                        n.map_or("under the wander".into(), |n| n.to_string())
                    );
                    n
                };
                let runs = if dry { 1 } else { n.unwrap_or(1) };
                for k in 1..runs {
                    let rk = settle_run(&mut level, &pose, setting, event, k * SEED_STRIDE, None);
                    if event == Event::Dig {
                        reset_world(&mut level);
                    }
                    sum.add(&rk);
                }
                #[allow(
                    clippy::disallowed_methods,
                    reason = "a reading test's frames a second, not simulation"
                )]
                let secs = t0.elapsed().as_secs_f64();
                let (steps, exposure) = sum.settles();
                let under = if n.is_none() {
                    " (under the wander: one seed)"
                } else {
                    ""
                };
                let parts: Vec<String> = steps
                    .iter()
                    .map(|(p, st, _)| format!("{} {}", p.name(), st.cell()))
                    .collect();
                println!(
                    "reading[{adapter}]: step {name}, {} seeds{under}: {}; exposure {}; {:.0} s",
                    sum.n,
                    parts.join(", "),
                    exposure.cell(),
                    secs
                );
                let tiles: Vec<String> = steps
                    .iter()
                    .map(|(p, _, s)| format!("{} {}", p.name(), s.cell()))
                    .collect();
                println!(
                    "reading[{adapter}]: per-tile {name}, {} seeds: {}; {} counted tiles",
                    sum.n,
                    tiles.join(", "),
                    steps[0].2.counted
                );
                for (p, st, s) in &steps {
                    println!(
                        "reading[{adapter}]: curve {name} {}: M every 8 frames {}",
                        p.name(),
                        st.curve_line()
                    );
                    println!(
                        "reading[{adapter}]: tile-curve {name} {}: settled % {}",
                        p.name(),
                        s.curve_line()
                    );
                }
                if setting.auto {
                    println!(
                        "reading[{adapter}]: curve {name} exposure: log2 every 8 frames {}",
                        exposure.curve_line()
                    );
                }
                if event == Event::Toggle {
                    let open = r.window.iter().take_while(|&&o| o).count();
                    let later = r.window.iter().skip(open).filter(|&&o| o).count();
                    println!(
                        "reading[{adapter}]: window {name}: open frames 1 to {open}, {later} more open later in 1 to 64"
                    );
                }
                if let Some(d) = &r.dots {
                    println!("reading[{adapter}]: dots {name}: {}", d.line());
                }
                if setting.auto {
                    let e: Vec<String> =
                        r.strip_exposure.iter().map(|e| format!("{e:.3}")).collect();
                    println!(
                        "reading[{adapter}]: auto exposure {name} at frames {STRIP_FRAMES:?}: {}",
                        e.join(" ")
                    );
                    if let (Some(tag), Some(p)) = (&tag, &pinned_strip) {
                        let size = if setting.upscaler {
                            OUTPUT
                        } else {
                            stillness::SIZE
                        };
                        auto_sheet(
                            tag,
                            &format!(
                                "{}-{}-lamp{}-ind{}-auto-vs-pinned",
                                pose.name,
                                event.name().replace(' ', "-"),
                                u8::from(setting.lamp),
                                u8::from(setting.indirect)
                            ),
                            size,
                            p,
                            &r.strip,
                        );
                    }
                } else {
                    pinned_strip = Some(r.strip);
                }
            }
        }
    }
}

/// The still-camera wander (run (vi)) of every part at each pose and setting, pinned exposure,
/// and the gather's share of the composite's and the display's view-mean variance where both
/// indirect states are read: `(sigma_on^2 - sigma_off^2) / sigma_on^2`, which assumes the
/// gather's wander independent of the direct parts'.
#[test]
#[ignore = "reading only: run with --ignored"]
fn wander_readings() {
    let Some(level) = reading_level() else {
        return;
    };
    let adapter = level.adapter.clone();
    for pose in poses() {
        let mut by: Vec<(Setting, Wanders)> = Vec::new();
        for setting in settings().into_iter().filter(|s| !s.auto) {
            let (w, fps) = wander_run(&level, &pose, setting, 0);
            let name = format!("{} {}", pose.name, setting.name());
            println!("reading[{adapter}]: wander {name}: {fps:.1} frames a second");
            for (p, w) in &w {
                println!(
                    "reading[{adapter}]: wander {name} {}: {}",
                    p.name(),
                    w.map_or("no counted tiles".into(), |w| w.line())
                );
            }
            by.push((setting, w));
        }
        for (s_on, w_on) in by.iter().filter(|(s, _)| s.indirect) {
            let Some((_, w_off)) = by
                .iter()
                .find(|(s, _)| !s.indirect && s.lamp == s_on.lamp && s.upscaler == s_on.upscaler)
            else {
                continue;
            };
            for part in [Part::Composite, Part::Display] {
                let get = |w: &[(Part, Option<Wander>)]| {
                    w.iter().find(|(p, _)| *p == part).and_then(|(_, w)| *w)
                };
                if let (Some(on), Some(off)) = (get(w_on), get(w_off)) {
                    let share =
                        (on.sigma_view.powi(2) - off.sigma_view.powi(2)) / on.sigma_view.powi(2);
                    println!(
                        "reading[{adapter}]: wander {} {}: gather's share of the {} view-mean variance {share:.3}",
                        pose.name,
                        s_on.name().replace("indirect on ", ""),
                        part.name()
                    );
                }
            }
        }
    }
}

/// The settle of every part after each event, pose and setting.
#[test]
#[ignore = "reading only: run with --ignored"]
fn settle_readings() {
    run_settle(&["toggle", "dig"], true);
}

/// The dots after the fast yaws and the dig (with their settle).
#[test]
#[ignore = "reading only: run with --ignored"]
fn dot_readings() {
    run_settle(&["yaw10", "yaw30", "yaw180", "dig"], false);
}

/// A long series after an event: the gather's and the composite's mean tile log2 over the
/// full tiles every 128 frames to `VXE_LONG_FRAMES` (default 8192), still camera, at each
/// pose and setting the filters pick (events toggle or dig).
#[test]
#[ignore = "reading only: run with --ignored"]
fn long_series() {
    let Some(mut level) = reading_level() else {
        return;
    };
    let adapter = level.adapter.clone();
    let frames: usize = std::env::var("VXE_LONG_FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8192);
    for pose in poses() {
        for event in events(&["toggle"]) {
            for setting in settings().into_iter().filter(|s| !s.auto) {
                let rows = long_run(&mut level, &pose, setting, event, frames);
                if event == Event::Dig {
                    reset_world(&mut level);
                }
                let line: Vec<String> = rows
                    .iter()
                    .map(|(f, g, c, e)| format!("{f}:{g:.3}/{c:.3}/{e:.3}"))
                    .collect();
                println!(
                    "reading[{adapter}]: long {} {} {}: frame:gather/composite/emissive mean log2 {}",
                    pose.name,
                    event.name(),
                    setting.name(),
                    line.join(" ")
                );
            }
        }
    }
}

fn k_runs() -> Vec<KRun> {
    list("VXE_K_RUNS", &["yaw", "flight"])
        .iter()
        .map(|r| match r.as_str() {
            "yaw" => KRun::Yaw,
            "flight" => KRun::Path,
            other => panic!("unknown flicker run {other}"),
        })
        .collect()
}

/// The world-keyed flicker `K` at the yaw and the flight, each pose and setting: over frames
/// `SHIMMER_FROM ..` of the run and over the frames before them.
#[test]
#[ignore = "reading only: run with --ignored"]
fn flicker_readings() {
    let Some(level) = reading_level() else {
        return;
    };
    let adapter = level.adapter.clone();
    for pose in poses() {
        for setting in settings().into_iter().filter(|s| !s.auto) {
            for run in k_runs() {
                let (late, early) = flicker_run(&level, &pose, setting, run);
                println!(
                    "reading[{adapter}]: flicker {} {} {}: from {SHIMMER_FROM} {}; before {SHIMMER_FROM} {}",
                    pose.name,
                    run.name(),
                    setting.name(),
                    late.cell(),
                    early.cell()
                );
            }
        }
    }
}

/// The poses and the spawn at the eye's settled exposure: the eye's value at each against the
/// pose's constant, lamp on.
fn eye_at(level: &stillness::VeinLevel, pose: &Pose) -> f32 {
    eye_exposure(
        level,
        pose,
        Setting {
            lamp: true,
            indirect: true,
            auto: true,
            upscaler: true,
            bridge: false,
            steady: Steady::default(),
        },
        1800,
    )
}

/// The eye's settled exposure at each pose and the spawn, 1800 frames lamp on, against the
/// pose's constant.
#[test]
#[ignore = "reading only: run with --ignored"]
fn eye_readings() {
    let Some(level) = reading_level() else {
        return;
    };
    let adapter = level.adapter.clone();
    for pose in POSES {
        println!(
            "reading[{adapter}]: eye {}: {:.4} after 1800 frames against the constant {:.4}",
            pose.name,
            eye_at(&level, &pose),
            pose.exposure
        );
    }
    let spawn = spawn_pose(1.0);
    println!(
        "reading[{adapter}]: eye spawn: {:.4} after 1800 frames",
        eye_at(&level, &spawn)
    );
}

/// The strip tool's flight's dots, each pose and the spawn (at the eye's settled exposure).
#[test]
#[ignore = "reading only: run with --ignored"]
fn strip_flight_dot_readings() {
    let Some(level) = reading_level() else {
        return;
    };
    let adapter = level.adapter.clone();
    let spawn = spawn_pose(eye_at(&level, &spawn_pose(1.0)));
    let all = [POSES[0], VEIN_POSE, spawn];
    let want = list("VXE_SETTLE_POSES", &["patch", "vein", "spawn"]);
    for pose in all.iter().filter(|p| want.iter().any(|w| w == p.name)) {
        for setting in settings().into_iter().filter(|s| !s.auto) {
            let d = strip_dots(&level, pose, setting);
            println!(
                "reading[{adapter}]: strip-flight dots {} {}: {}",
                pose.name,
                setting.name(),
                d.line()
            );
        }
    }
}

/// The share of traced probes inside rock at frames 8 .. 1024 at rest, indirect on.
#[test]
#[ignore = "reading only: run with --ignored"]
fn probe_census_readings() {
    let Some(level) = reading_level() else {
        return;
    };
    let adapter = level.adapter.clone();
    for pose in poses() {
        let setting = Setting {
            lamp: true,
            indirect: true,
            auto: false,
            upscaler: true,
            bridge: false,
            steady: Steady::default(),
        };
        let mut frame = make_frame(&level, &pose, setting);
        let light = LightParams::headlamp();
        let cam = pose.camera();
        let mut done = 0;
        for at in [8usize, 64, 256, 1024] {
            for _ in done..at {
                frame.step(&level.device, &level.queue, &level.gpu, &cam, &light);
            }
            done = at;
            let (traced, inside) = probe_census(&level, &frame);
            println!(
                "reading[{adapter}]: probes {} frame {at}: {traced} traced, {inside} inside rock ({:.4} of traced)",
                pose.name,
                f64::from(inside) / f64::from(traced.max(1))
            );
        }
    }
}

/// The frame's per-pass cost at 1080p from 720p in the steady flight, each pose, lamp on,
/// indirect on, under `VXE_STEADY`'s presets.
#[test]
#[ignore = "reading only: run with --ignored"]
fn flight_cost_readings() {
    let Some(level) = reading_level() else {
        return;
    };
    let adapter = level.adapter.clone();
    for pose in poses() {
        for steady in list("VXE_STEADY", &["off"])
            .iter()
            .map(|s| Steady::named(s))
        {
            let lighting = steady.apply(Lighting {
                lamp: true,
                indirect: true,
                exposure: Exposure::Manual(pose.exposure),
                ..Lighting::default()
            });
            let Some(ms) = flight_cost(&level, &pose, lighting, KRun::Path, 120) else {
                println!("reading[{adapter}]: no TIMESTAMP_QUERY");
                return;
            };
            let line: Vec<String> = ms
                .iter()
                .filter(|(_, v)| *v > 0.0)
                .map(|(n, v)| format!("{n} {v:.3}"))
                .collect();
            let total: f64 = ms.iter().map(|(_, v)| v).sum();
            println!(
                "reading[{adapter}]: flight cost {}{} 1920x1080 from 1280x720: total {total:.3} ms ({})",
                pose.name,
                steady.words(),
                line.join(", ")
            );
        }
    }
}

/// A's frozen share at rest: the volume's and the view's traced, inside and frozen probes every
/// 64 frames to 1024, at `VXE_STEADY`'s presets' freeze counts, and the first frame all of the
/// view's traced probes outside rock read frozen.
#[test]
#[ignore = "reading only: run with --ignored"]
fn frozen_share_readings() {
    let Some(level) = reading_level() else {
        return;
    };
    let adapter = level.adapter.clone();
    for pose in poses() {
        for setting in settings().into_iter().filter(|s| !s.auto) {
            let freeze = setting
                .steady
                .a1
                .unwrap_or(vxe_render::marcher::DEFAULT_PROBE_FREEZE);
            let mut frame = make_frame(&level, &pose, setting);
            let light = LightParams::headlamp();
            let cam = pose.camera();
            let mut curve = Vec::new();
            let mut first = None;
            for f in 1..=1024usize {
                frame.step(&level.device, &level.queue, &level.gpu, &cam, &light);
                if f % 8 == 0 {
                    let c = freeze_census(&level, &frame, &cam, freeze);
                    if first.is_none()
                        && c.in_view[0] > 0
                        && c.in_view[2] + c.in_view[1] == c.in_view[0]
                    {
                        first = Some(f);
                    }
                    if f % 64 == 0 {
                        curve.push(format!(
                            "{f}:{:.3}/{:.3}",
                            f64::from(c.all[2]) / f64::from((c.all[0] - c.all[1]).max(1)),
                            f64::from(c.in_view[2])
                                / f64::from((c.in_view[0] - c.in_view[1]).max(1)),
                        ));
                    }
                }
            }
            println!(
                "reading[{adapter}]: frozen {} {}: view frozen at {first:?}; frame:share of volume/view {}",
                pose.name,
                setting.name(),
                curve.join(" ")
            );
        }
    }
}

// ---- Experiments A and C: the asserted tests ----

/// `freeze`'s frame for the lavapipe tests: 64 x 36, indirect on, 8 slices, at the vein.
fn steady_frame(level: &stillness::VeinLevel, lighting: Lighting) -> Frame {
    let mut frame = Frame::new(&level.device, 64, 36);
    frame.set_lighting(Lighting {
        indirect: true,
        probe_slices: 8,
        exposure: Exposure::Manual(VEIN_POSE.exposure),
        ..lighting
    });
    frame.set_dt(1.0 / 60.0);
    frame
}

fn probe_words(level: &stillness::VeinLevel, frame: &Frame) -> Vec<u8> {
    let data = frame.probes().data();
    vxe_render::readback::read_buffer(&level.device, &level.queue, data, 0, data.size())
}

/// The slot of a traced probe outside rock, with light in it if `lit`, nearest the eye, of the
/// probes `keep` accepts.
fn traced_probe(
    level: &stillness::VeinLevel,
    frame: &Frame,
    camera: &Camera,
    lit: bool,
    keep: impl Fn(Vec3) -> bool,
) -> u32 {
    use vxe_render::probes::{PROBES_PER_AXIS, probe_slot};
    let origin = frame.probes().origin_for(camera.position.to_array());
    let n = PROBES_PER_AXIS as i32;
    let mut all = Vec::new();
    for z in 0..n {
        for y in 0..n {
            for x in 0..n {
                let w = [origin[0] + x, origin[1] + y, origin[2] + z];
                let p = Vec3::from(frame.probes().probe_position(w));
                if keep(p) {
                    all.push(((p - camera.position).length(), probe_slot(w)));
                }
            }
        }
    }
    all.sort_by(|a, b| a.0.total_cmp(&b.0));
    all.iter()
        .map(|&(_, s)| s)
        .find(|&s| {
            let r = frame.probes().read_probe(&level.device, &level.queue, s);
            !r.inside()
                && r.age() > 0
                && (!lit
                    || r.interior_irradiance()
                        .iter()
                        .any(|t| t[0] + t[1] + t[2] > 0.0))
        })
        .expect("a traced probe outside rock")
}

/// A probe that has reached its freeze count is not traced: at `probe_freeze` 8 and 8 slices,
/// after 8 visits (64 frames) with no restart every probe's words are bit-identical across the
/// next 64 encoded frames, and a lamp toggle restarts a lit probe on the first visit after.
#[test]
fn a_frozen_probe_is_not_traced() {
    let Some(level) = vein_level() else {
        return;
    };
    let light = LightParams::headlamp();
    let camera = VEIN_POSE.camera();
    let mut frame = steady_frame(
        &level,
        Lighting {
            probe_freeze: Some(8),
            ..Lighting::default()
        },
    );
    let step = |frame: &mut Frame, n: usize| {
        for _ in 0..n {
            frame.step(&level.device, &level.queue, &level.gpu, &camera, &light);
        }
    };
    step(&mut frame, 64);
    let frozen = probe_words(&level, &frame);
    step(&mut frame, 64);
    assert!(
        probe_words(&level, &frame) == frozen,
        "the atlases and states moved over 64 frames after the freeze"
    );
    let slot = traced_probe(&level, &frame, &camera, true, |_| true);
    let before = frame.probes().read_probe(&level.device, &level.queue, slot);
    frame.set_lighting(Lighting {
        lamp: false,
        ..frame.lighting()
    });
    step(&mut frame, 4);
    assert!(frame.probe_light_window(), "the toggle opened the window");
    let after = frame.probes().read_probe(&level.device, &level.queue, slot);
    assert_ne!(
        after.irradiance, before.irradiance,
        "the window's first visit moved the texels"
    );
}

/// With a frozen volume pinned so that the far corner is out of `lamp_reach`, a lamp moved each
/// frame changes a near probe's texels within 8 frames, keeps a probe beyond the reach
/// bit-identical, and the near probe freezes again once the lamp holds still.
#[test]
fn a_moving_lamp_unfreezes_the_probes_it_reaches() {
    let Some(level) = vein_level() else {
        return;
    };
    let light = LightParams::headlamp();
    let reach = vxe_render::probes::lamp_reach(light.intensity);
    let camera = VEIN_POSE.camera();
    let mut frame = steady_frame(
        &level,
        Lighting {
            probe_freeze: Some(8),
            ..Lighting::default()
        },
    );
    // The eye in the volume's lowest cell on every axis.
    let mut origin = frame.probes().origin_for(VEIN_POSE.position);
    for o in &mut origin {
        *o += vxe_render::probes::PROBES_PER_AXIS as i32 / 2 - 1;
    }
    frame.probes_mut().set_origin(Some(origin));
    let step = |frame: &mut Frame, cam: &Camera, n: usize| {
        for _ in 0..n {
            frame.step(&level.device, &level.queue, &level.gpu, cam, &light);
        }
    };
    step(&mut frame, &camera, 64);
    let near = traced_probe(&level, &frame, &camera, true, |_| true);
    // Beyond the reach no emitter need be in range: its state word still shows a probe held
    // open, whose age the moving lamp would cap.
    let far = traced_probe(&level, &frame, &camera, false, |p| {
        (p - camera.position).length() > reach + 8.0
    });
    let read = |frame: &Frame, s: u32| frame.probes().read_probe(&level.device, &level.queue, s);
    let (near0, far0) = (read(&frame, near), read(&frame, far));
    assert!(near0.age() >= 8 && far0.age() >= 8, "both froze");
    // A turn moves the lamp's position and direction together; it opens no window.
    for i in 1..=8 {
        let c = Camera {
            yaw: camera.yaw + 0.002 * i as f32,
            ..camera
        };
        step(&mut frame, &c, 1);
        assert!(!frame.probe_light_window());
    }
    let near1 = read(&frame, near);
    assert_ne!(near1.irradiance, near0.irradiance, "the near probe moved");
    assert!(
        near1.age() <= 4,
        "the moving lamp caps its age: {}",
        near1.age()
    );
    assert_eq!(read(&frame, far), far0, "the far probe stayed frozen");
    // The lamp holds still: the rest of the visits, then frozen again.
    let held = Camera {
        yaw: camera.yaw + 0.016,
        ..camera
    };
    step(&mut frame, &held, 40);
    let settled = read(&frame, near);
    assert!(settled.age() >= 8, "it refroze at age {}", settled.age());
    step(&mut frame, &held, 16);
    assert_eq!(read(&frame, near), settled, "and holds");
}

/// With A2 a probe inside rock keeps its texels and its state bit-identical over 64 frames.
#[test]
fn an_inside_probe_is_not_traced() {
    let Some(level) = vein_level() else {
        return;
    };
    let light = LightParams::headlamp();
    let camera = VEIN_POSE.camera();
    let mut frame = steady_frame(
        &level,
        Lighting {
            skip_inside_probes: true,
            ..Lighting::default()
        },
    );
    for _ in 0..8 {
        frame.step(&level.device, &level.queue, &level.gpu, &camera, &light);
    }
    let (traced, inside) = probe_census(&level, &frame);
    assert!(
        inside > 0 && traced > inside,
        "{traced} traced, {inside} inside"
    );
    use vxe_render::probes::PROBE_COUNT;
    let slots: Vec<u32> = (0..PROBE_COUNT)
        .filter(|&s| {
            frame
                .probes()
                .read_probe(&level.device, &level.queue, s)
                .inside()
        })
        .take(64)
        .collect();
    let before: Vec<_> = slots
        .iter()
        .map(|&s| frame.probes().read_probe(&level.device, &level.queue, s))
        .collect();
    for _ in 0..64 {
        frame.step(&level.device, &level.queue, &level.gpu, &camera, &light);
    }
    for (&s, b) in slots.iter().zip(&before) {
        assert_eq!(
            &frame.probes().read_probe(&level.device, &level.queue, s),
            b,
            "slot {s}"
        );
    }
}

/// With A3 and A4 at threshold 64 and indirect off, a still pixel at the threshold keeps its
/// emissive value bit-identical for 64 frames with its reservoir copied through, and a `Replace`
/// of the emitters around its kept point lowers its age within 9 frames and resumes the draw.
#[test]
fn a_converged_pixel_keeps_its_sample() {
    let Some(mut level) = vein_level() else {
        return;
    };
    let light = LightParams::headlamp();
    let camera = VEIN_POSE.camera();
    let mut frame = Frame::new(&level.device, 64, 36);
    frame.set_lighting(Lighting {
        lamp: false,
        indirect: false,
        emissive_history: vxe_render::marcher::HISTORY_AGE_MAX,
        skip_converged_draw: Some(64),
        exposure: Exposure::Manual(VEIN_POSE.exposure),
        ..Lighting::default()
    });
    frame.set_dt(1.0 / 60.0);
    let read = |level: &stillness::VeinLevel, frame: &Frame| {
        (
            vxe_render::readback::read_float(&level.device, &level.queue, frame.accumulated()),
            vxe_render::readback::read_float(&level.device, &level.queue, frame.moments()),
            vxe_render::readback::read_texels_u32(&level.device, &level.queue, frame.reservoirs()),
        )
    };
    for _ in 0..66 {
        frame.step(&level.device, &level.queue, &level.gpu, &camera, &light);
    }
    let (accum0, moments0, res0) = read(&level, &frame);
    // The pixels at the threshold with a reservoir, keeping light.
    let kept =
        |i: usize| moments0[i][2] >= 64.0 && res0[i][3] & 0x800_0000 != 0 && accum0[i][1] > 0.0;
    let pixels: Vec<usize> = (0..accum0.len()).filter(|&i| kept(i)).collect();
    assert!(pixels.len() > 100, "{} converged pixels", pixels.len());
    for f in 0..64 {
        frame.step(&level.device, &level.queue, &level.gpu, &camera, &light);
        let (a, m, r) = read(&level, &frame);
        for &i in &pixels {
            assert_eq!(
                a[i], accum0[i],
                "pixel {i} moved at frame {f}, moments {:?}",
                m[i]
            );
            assert_eq!(r[i], res0[i], "pixel {i}'s reservoir moved");
        }
    }
    // The emitters around a kept point go dark. A pixel's kept voxel lies behind its face.
    let i = *pixels
        .iter()
        .max_by(|&&a, &&b| moments0[a][3].total_cmp(&moments0[b][3]))
        .expect("pixels");
    let x = [res0[i][0], res0[i][1], res0[i][2]].map(f32::from_bits);
    let face = (res0[i][3] >> 24) & 7;
    let mut v = x.map(|c| c.floor() as i64);
    let axis = (face / 2) as usize;
    if face & 1 == 1 {
        v[axis] -= 1;
    }
    let (min, max) = (
        v.map(|c| (c - 12) * vxe_world::FIXED_ONE),
        v.map(|c| (c + 13) * vxe_world::FIXED_ONE),
    );
    let r = level
        .world
        .apply(&vxe_world::EditOp::box_(
            vxe_world::Fixed3(min),
            vxe_world::Fixed3(max),
            vxe_world::Mode::Replace {
                from: flythrough::cave::ORE,
                to: flythrough::cave::ROCK,
            },
        ))
        .expect("the replace applies");
    assert!(
        r.changed_from.iter().map(|c| c.1).sum::<u32>() > 0,
        "the box holds ore"
    );
    level
        .gpu
        .sync_now(&level.world, &level.device, &level.queue)
        .expect("mirror fits");
    let start = frame_age(&level, &frame, i);
    let mut ages = Vec::new();
    for _ in 1..=30 {
        frame.step(&level.device, &level.queue, &level.gpu, &camera, &light);
        ages.push(frame_age(&level, &frame, i));
    }
    println!("the pixel's age from {start}, per frame after the replace: {ages:?}");
    let lowered = ages.iter().position(|&a| a < start).map(|f| f + 1);
    let resumed = ages.iter().position(|&a| a < 64.0).map(|f| f + 1);
    println!("lowered at frame {lowered:?}, under the threshold at {resumed:?}");
    assert!(
        lowered.is_some_and(|f| f <= 9),
        "the guard lowered the pixel's age within 9 frames"
    );
    assert!(
        resumed.is_some_and(|f| f <= 9),
        "the age fell under the threshold within 9 frames, so the draw resumed"
    );
    let (a, _, _) = read(&level, &frame);
    assert_ne!(a[i], accum0[i], "the draw resumed and the value moved");
}

/// Lighting pixel `i`'s history age.
fn frame_age(level: &stillness::VeinLevel, frame: &Frame, i: usize) -> f32 {
    vxe_render::readback::read_float(&level.device, &level.queue, frame.moments())[i][2]
}

/// With C on, a lamp toggle both ways and a lamp moved each frame leave every probe's words
/// bit-identical to a run with no lamp change at the same frames (the rays' random numbers go
/// with the frame, so nothing else differs); an emission scale change still restarts them.
#[test]
fn the_lamp_leaves_the_probes_alone_under_c() {
    let Some(level) = vein_level() else {
        return;
    };
    let camera = VEIN_POSE.camera();
    let lighting = Lighting {
        probe_lamp: false,
        ..Lighting::default()
    };
    let run = |changes: bool| {
        let mut frame = steady_frame(&level, lighting);
        let mut light = LightParams::headlamp();
        for f in 0..64 {
            if changes {
                if f == 8 || f == 32 {
                    let l = frame.lighting();
                    frame.set_lighting(Lighting { lamp: !l.lamp, ..l });
                }
                light.offset_right += 0.01;
            }
            let c = if changes {
                Camera {
                    yaw: camera.yaw + 0.001 * f as f32,
                    ..camera
                }
            } else {
                camera
            };
            frame.step(&level.device, &level.queue, &level.gpu, &c, &light);
            assert!(!frame.probe_light_window(), "frame {f} opened a window");
        }
        (frame, light)
    };
    let (still, _) = run(false);
    let (mut moved, light) = run(true);
    assert!(
        probe_words(&level, &still) == probe_words(&level, &moved),
        "the lamp reached the probes"
    );
    let before = probe_words(&level, &moved);
    moved.set_emission_scale(&level.queue, flythrough::cave::ORE, 0.5);
    moved.set_emission_scale(&level.queue, flythrough::cave::ORE, 0.5);
    let mut opened = false;
    for _ in 0..8 {
        moved.step(&level.device, &level.queue, &level.gpu, &camera, &light);
        opened |= moved.probe_light_window();
    }
    assert!(opened, "an emission change still opens the window");
    assert!(probe_words(&level, &moved) != before);
}
