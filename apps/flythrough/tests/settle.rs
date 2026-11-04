//! How fast the light settles after a fly-in, a lamp toggle, a dig and a fast yaw, the dots a
//! turn, a flight or a dig leaves, the world-keyed flicker, the probes inside rock and the
//! frame's cost in flight (`support::settle`), on the level's ore veins at `stillness`'s poses,
//! on the fly-through's default look (`LookPreset::DEFAULT`, FSR 2 at 1.5).
//!
//! The asserted tests run on the software adapter (lavapipe). The `*_readings` tests are
//! reading-only and `#[ignore]`d: they print readings and assert nothing a gate runs, on the
//! adapter `VXE_TEST_ADAPTER` picks (`support::reading_device`). Their runs narrow by
//! environment variables, each a comma list: `VXE_SETTLE_POSES` (patch, vein),
//! `VXE_SETTLE_EVENTS` (flyin, flyin-pinned, toggle, dig, dig100, yaw10, yaw30, yaw118,
//! yaw180),
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
use vxe_render::marcher::LightingResolution;
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
            "dig100" => Event::DigFar,
            "yaw10" => Event::Yaw {
                frames: 10,
                degrees: 90,
            },
            "yaw30" => Event::Yaw {
                frames: 30,
                degrees: 90,
            },
            "yaw118" => Event::Yaw {
                frames: 118,
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
                if event.is_dig() {
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
                    if event.is_dig() {
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

/// The dots after the fast yaws, the slow yaw (90 degrees over 118 frames) and the two digs
/// (with their settle), split by cell age where the run reads the table.
#[test]
#[ignore = "reading only: run with --ignored"]
fn dot_readings() {
    run_settle(
        &["yaw10", "yaw30", "yaw118", "yaw180", "dig", "dig100"],
        false,
    );
}

/// The dug hole's emissive settle and young share (`dig_box_run`) at arm's length and down the
/// passage, at the vein unless `VXE_SETTLE_POSES` names others.
#[test]
#[ignore = "reading only: run with --ignored"]
fn dig_box_readings() {
    let Some(mut level) = reading_level() else {
        return;
    };
    let adapter = level.adapter.clone();
    let want = list("VXE_SETTLE_POSES", &["vein"]);
    for pose in POSES.iter().filter(|p| want.iter().any(|w| w == p.name)) {
        for event in events(&["dig", "dig100"]) {
            for setting in settings().into_iter().filter(|s| !s.auto) {
                let line = dig_box_run(&mut level, pose, setting, event == Event::DigFar);
                reset_world(&mut level);
                println!(
                    "reading[{adapter}]: hole {} {} {}: {line}",
                    pose.name,
                    event.name(),
                    setting.name()
                );
            }
        }
    }
}

/// The table's occupancy after the steady flight repeated `VXE_FLIGHTS` times (default 4) from
/// each pose, never cleared between (`occupancy_run`).
#[test]
#[ignore = "reading only: run with --ignored"]
fn occupancy_readings() {
    let Some(level) = reading_level() else {
        return;
    };
    let adapter = level.adapter.clone();
    let flights: usize = std::env::var("VXE_FLIGHTS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4);
    for pose in poses() {
        for setting in settings().into_iter().filter(|s| !s.auto) {
            for line in occupancy_run(&level, &pose, setting, flights) {
                println!(
                    "reading[{adapter}]: occupancy {} {}: {line}",
                    pose.name,
                    setting.name()
                );
            }
        }
    }
}

/// The table guard's reopens on a still camera: at each pose and setting the filters pick,
/// `VXE_STILL_FRAMES` (default 1024) still frames from a fresh frame, the guard's reopen count
/// since the start printed every 256 frames.
#[test]
#[ignore = "reading only: run with --ignored"]
fn still_reopen_readings() {
    let Some(level) = reading_level() else {
        return;
    };
    let adapter = level.adapter.clone();
    let frames: usize = std::env::var("VXE_STILL_FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1024);
    let light = LightParams::headlamp();
    for pose in poses() {
        for setting in settings() {
            let mut frame = make_frame(&level, &pose, setting);
            let camera = pose.camera();
            let mut line = Vec::new();
            for f in 1..=frames {
                frame.step(&level.device, &level.queue, &level.gpu, &camera, &light);
                if f % 256 == 0 || f == frames {
                    let n = frame.read_table_reopens(&level.device, &level.queue);
                    let (by, e) = frame.read_table_reopens_by(&level.device, &level.queue);
                    line.push(format!(
                        "{f}: {n} (re-shade {}, torn {}, dark {}; last re-shade e_old {:.5}, e_new {:.5})",
                        by[0], by[1], by[2], e[0], e[1]
                    ));
                }
            }
            println!(
                "reading[{adapter}]: still reopens {} {}: {}",
                pose.name,
                setting.name(),
                line.join(", ")
            );
        }
    }
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
                if event.is_dig() {
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

/// The eye's settled exposure in the constants' own state: 600 frames lamp on, then 900 off.
#[test]
#[ignore = "reading only: run with --ignored"]
fn eye_lamp_off_readings() {
    let Some(level) = reading_level() else {
        return;
    };
    let adapter = level.adapter.clone();
    for pose in POSES {
        println!(
            "reading[{adapter}]: eye {} 600 on, 900 off: {:.4} against the constant {:.4}",
            pose.name,
            eye_after_lamp_off(&level, &pose),
            pose.exposure
        );
    }
}

/// The strip flight's camera at each 12th frame: whether it is in air. CPU only.
#[test]
#[ignore = "reading only: run with --ignored"]
fn strip_path_readings() {
    let source = flythrough::cave::CaveSource::new(
        1,
        vxe_world::ChunkExtent::new([0, 0, 0], flythrough::cave::LEVEL_CHUNKS),
    );
    let world = vxe_world::World::generate(
        source.extent,
        &source,
        &flythrough::with_emissive_ore(&flythrough::cave_materials()),
    );
    for pose in [POSES[0], VEIN_POSE, spawn_pose(1.0)] {
        let mut camera = pose.camera();
        let mut line = Vec::new();
        for f in 0..STRIP_FRAMES_FLOWN {
            let (fwd, _, _) = camera.basis();
            camera.position += fwd * STRIP_SPEED / 60.0;
            camera.yaw += STRIP_YAW.to_radians() / 60.0;
            let v = camera.position.to_array().map(|c| c.floor() as i32);
            let air = world.material_at(v) == Some(vxe_world::AIR);
            if f % 12 == 0 {
                line.push(if air { "air" } else { "rock" });
            }
        }
        println!(
            "reading: strip path {} every 12 frames: {}",
            pose.name,
            line.join(" ")
        );
    }
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

/// The dots along the rig's steady flight, whose path has air around every step.
#[test]
#[ignore = "reading only: run with --ignored"]
fn path_flight_dot_readings() {
    let Some(level) = reading_level() else {
        return;
    };
    let adapter = level.adapter.clone();
    for pose in poses() {
        for setting in settings().into_iter().filter(|s| !s.auto) {
            let d = path_dots(&level, &pose, setting);
            println!(
                "reading[{adapter}]: path-flight dots {} {}: {}",
                pose.name,
                setting.name(),
                d.line()
            );
        }
    }
}

/// B's young share, occupancy and bytes along the yaw and the flight.
#[test]
#[ignore = "reading only: run with --ignored"]
fn cache_readings() {
    let Some(level) = reading_level() else {
        return;
    };
    let adapter = level.adapter.clone();
    for pose in poses() {
        for setting in settings().into_iter().filter(|s| !s.auto) {
            for run in k_runs() {
                println!(
                    "reading[{adapter}]: cache {} {} {}: {}",
                    pose.name,
                    run.name(),
                    setting.name(),
                    cache_run(&level, &pose, setting, run)
                );
            }
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
/// indirect as `VXE_SETTLE_INDIRECT` lists (default on), under `VXE_STEADY`'s presets.
#[test]
#[ignore = "reading only: run with --ignored"]
fn flight_cost_readings() {
    let Some(level) = reading_level() else {
        return;
    };
    let adapter = level.adapter.clone();
    let indirect: Vec<bool> = list("VXE_SETTLE_INDIRECT", &["on"])
        .iter()
        .map(|s| s == "on")
        .collect();
    for pose in poses() {
        for (steady, indirect) in list("VXE_STEADY", &["off"])
            .iter()
            .map(|s| Steady::named(s))
            .flat_map(|s| indirect.iter().map(move |&i| (s, i)))
        {
            let lighting = steady.apply(Lighting {
                lamp: true,
                indirect,
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
                "reading[{adapter}]: flight cost {} indirect {}{} 1920x1080 from 1280x720: total {total:.3} ms ({})",
                pose.name,
                if indirect { "on" } else { "off" },
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

// ---- Experiment B: the asserted tests ----

/// Air over chunks `[0, 2)^3`, a rock block at x, y in `[40, 64)`, z in `[40, 64)`, and an ore
/// plate over it at z `[80, 84)`: the block's corner faces the camera, its faces lit from above.
fn cache_box() -> (vxe_world::World, Camera) {
    cache_box_in(&flythrough::with_emissive_ore(&flythrough::cave_materials()))
}

/// The box (`cache_box`) over the material table `table`.
fn cache_box_in(table: &vxe_world::MaterialTable) -> (vxe_world::World, Camera) {
    use vxe_world::*;
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [2, 2, 2]),
        &UniformSource(AIR),
        table,
    );
    let fill = |min: [i64; 3], max: [i64; 3], m| {
        world
            .apply(&EditOp::box_(
                Fixed3(min.map(|v| v * FIXED_ONE)),
                Fixed3(max.map(|v| v * FIXED_ONE)),
                Mode::Fill(m),
            ))
            .expect("the box is resident");
    };
    fill([40, 40, 40], [64, 64, 64], flythrough::cave::ROCK);
    fill([20, 20, 80], [90, 90, 84], flythrough::cave::ORE);
    // At the block's lowest corner on x and y, above its top: three faces in view.
    let camera = Camera {
        position: [30.5, 28.5, 70.5].into(),
        yaw: 45f32.to_radians(),
        pitch: (-35f32).to_radians(),
        vfov: 70f32.to_radians(),
    };
    (world, camera)
}

struct CacheBox {
    level: stillness::VeinLevel,
    camera: Camera,
    frame: Frame,
}

/// The box under a frame at 64 x 36 (full-resolution lighting, no jitter, lamp and probes off),
/// the table's cap at `cap`.
fn cache_box_rig(cap: u32) -> Option<CacheBox> {
    cache_box_rig_sized(cap, [64, 36])
}

/// `cache_box_rig` at `size`.
fn cache_box_rig_sized(cap: u32, size: [u32; 2]) -> Option<CacheBox> {
    let (world, camera) = cache_box();
    cache_box_rig_on(world, camera, cap, size, None)
}

/// `cache_box_rig` over `world` from `camera` at `size`, with the mirror's light radius
/// `light_radius` when given.
fn cache_box_rig_on(
    world: vxe_world::World,
    camera: Camera,
    cap: u32,
    size: [u32; 2],
    light_radius: Option<i32>,
) -> Option<CacheBox> {
    let (device, queue) = support::software_device(64 << 20)?;
    let base = vxe_render::MirrorConfig::new(vxe_render::Window::Fixed(world.extent()));
    let cfg = vxe_render::MirrorConfig {
        budget_bytes: 64 << 20,
        light_radius: light_radius.unwrap_or(base.light_radius),
        ..base
    };
    let mut gpu = vxe_render::GpuWorld::with_config(&device, cfg);
    for _ in 0..2 {
        gpu.sync_now(&world, &device, &queue)
            .expect("the mirror fits");
    }
    let mut frame = Frame::new(&device, size[0], size[1]);
    frame.set_resolution(&device, vxe_render::LightingResolution::Full);
    frame.set_lighting(Lighting {
        lamp: false,
        indirect: false,
        exposure: Exposure::Manual(1.0),
        table_cap: cap,
        ..Lighting::default()
    });
    frame.set_dt(1.0 / 60.0);
    Some(CacheBox {
        level: stillness::VeinLevel {
            device,
            queue,
            gpu,
            world,
            adapter: "llvmpipe (lavapipe)".to_owned(),
        },
        camera,
        frame,
    })
}

impl CacheBox {
    fn step(&mut self, n: usize) {
        let light = LightParams::headlamp();
        for _ in 0..n {
            self.frame.step(
                &self.level.device,
                &self.level.queue,
                &self.level.gpu,
                &self.camera,
                &light,
            );
        }
    }

    /// Each face key in view with its pixel count this frame.
    fn keys(&self) -> std::collections::HashMap<FaceKey, u32> {
        let (d, q) = (&self.level.device, &self.level.queue);
        let pos = vxe_render::readback::read_float(d, q, self.frame.hit_positions());
        let info = vxe_render::readback::read_rgba16u(d, q, self.frame.hit_records());
        let mut out = std::collections::HashMap::new();
        for (p, i) in pos.iter().zip(&info) {
            if let Some(k) = face_key(self.camera.position, *p, *i) {
                *out.entry(k).or_insert(0u32) += 1;
            }
        }
        out
    }
}

/// A key is its voxel and face: on the box, one frame writes as many cells as the view has face
/// keys, each counting exactly its pixels; two pixels on one face share a cell and the same
/// voxel's other face has its own.
#[test]
fn a_key_is_its_voxel_and_face() {
    let Some(mut rig) = cache_box_rig(vxe_render::marcher::DEFAULT_TABLE_CAP) else {
        return;
    };
    rig.step(1);
    let keys = rig.keys();
    let words = cache_words(&rig.level, &rig.frame);
    let [used, _, _] = cache_census(&words, 16, 4096);
    assert_eq!(
        used,
        keys.len() as u64,
        "the occupancy counts each key once"
    );
    for (k, n) in &keys {
        assert_eq!(cache_count(&words, *k), *n, "key {k:?} counts its pixels");
    }
    assert!(
        keys.values().any(|&n| n >= 2),
        "two pixels on one face share a cell"
    );
    let two_faced = keys
        .keys()
        .find(|(v, f)| keys.keys().any(|(w, g)| w == v && g != f));
    let (v, f) = two_faced.expect("a voxel shows two faces");
    let other = keys
        .keys()
        .find(|(w, g)| w == v && g != f)
        .expect("its other face");
    assert_ne!(
        cache_cell(&words, (*v, *f)),
        cache_cell(&words, *other),
        "a voxel's two faces take two cells"
    );
}

/// Once every cell in view has reached the cap, the emissive part is bit-identical frame to
/// frame and those pixels' light pass draws nothing (its emissive sample reads 0): at `C_CAP`
/// 64, scale 1, no jitter, lamp and probes off.
#[test]
fn a_capped_cell_does_not_move() {
    let Some(mut rig) = cache_box_rig(64) else {
        return;
    };
    rig.step(66);
    let words = cache_words(&rig.level, &rig.frame);
    for k in rig.keys().keys() {
        assert!(cache_count(&words, *k) >= 64, "key {k:?} is capped");
    }
    let (d, q) = (&rig.level.device, &rig.level.queue);
    let info = vxe_render::readback::read_rgba16u(d, q, rig.frame.hit_records());
    let on_face: Vec<usize> = (0..info.len())
        .filter(|&i| info[i][3] & 1 == 1 && (info[i][3] >> 3) & 3 > 0)
        .collect();
    assert!(on_face.len() > 100, "{} pixels on faces", on_face.len());
    let read = |rig: &CacheBox| {
        (
            vxe_render::readback::read_float(d, q, rig.frame.accumulated()),
            vxe_render::readback::read_float(d, q, rig.frame.irradiance()),
        )
    };
    let (a0, _) = read(&rig);
    assert!(
        on_face.iter().any(|&i| a0[i][0] > 0.0),
        "the plate lights the block"
    );
    let light = LightParams::headlamp();
    for _ in 0..64 {
        rig.frame.step(d, q, &rig.level.gpu, &rig.camera, &light);
        let (a, r) = read(&rig);
        for &i in &on_face {
            assert_eq!(a[i][0..3], a0[i][0..3], "pixel {i}'s emissive part moved");
            assert_eq!(r[i][0..3], [0.0; 3], "pixel {i} drew an emissive sample");
        }
    }
}

/// An edit box over one voxel clears that voxel's cells, leaves a capped neighbour two voxels
/// away, and the cleared cells refill to 16 samples within 16 frames.
#[test]
fn a_dig_clears_its_cells() {
    let Some(mut rig) = cache_box_rig(64) else {
        return;
    };
    rig.step(66);
    let keys = rig.keys();
    let words = cache_words(&rig.level, &rig.frame);
    // A voxel with a face that takes pixels every frame, and a capped face in view at least two
    // voxels past the box's dilation on some axis.
    let (&(v, f), _) = keys
        .iter()
        .max_by_key(|(k, n)| (**n, k.0))
        .expect("faces in view");
    let far = keys
        .keys()
        .find(|(w, _)| (0..3).any(|a| (w[a] - v[a]).abs() == 2))
        .copied()
        .expect("a face two voxels away");
    assert!(cache_count(&words, (v, f)) >= 64);
    let far_before = cache_count(&words, far);
    assert!(far_before >= 64);
    rig.frame.invalidate(v, v);
    rig.step(1);
    let words = cache_words(&rig.level, &rig.frame);
    let n = keys[&(v, f)];
    assert_eq!(
        cache_count(&words, (v, f)),
        n,
        "the cell restarted this frame"
    );
    for face in (0..6u8).filter(|&g| g != f) {
        assert!(
            cache_count(&words, (v, face)) < 64,
            "face {face} of the box's voxel cleared"
        );
    }
    assert_eq!(
        cache_count(&words, far),
        far_before,
        "the neighbour two voxels away stays capped"
    );
    rig.step(15);
    let words = cache_words(&rig.level, &rig.frame);
    assert!(
        cache_count(&words, (v, f)) >= 16,
        "the cell refilled to 16 samples within 16 frames: {}",
        cache_count(&words, (v, f))
    );
}

/// The dots' split by cell age is a partition of the excess: on the box at the rig's `SIZE`,
/// scale 1, a fresh table at `C_MIN` 16, frames 1 to 8 read as the event's (young cells) and
/// frames 25 to 32 as the floor's, every frame's young and mature dots sum to its dots, and
/// the young and mature excess sum to the excess.
#[test]
fn the_dots_split_by_cell_age_sums_to_the_excess() {
    let Some(mut rig) = cache_box_rig_sized(
        vxe_render::marcher::DEFAULT_TABLE_CAP,
        support::stillness::SIZE,
    ) else {
        return;
    };
    rig.frame.set_composite_capture(&rig.level.device, true);
    let mut dots = Dots {
        cell_split: true,
        ..Dots::default()
    };
    let (mut floor, mut floor_cell, mut seen) = (0usize, [0usize; 2], 0usize);
    for f in 1..=32 {
        rig.step(1);
        if (9..25).contains(&f) {
            continue;
        }
        let px = read_dots(&rig.level, &rig.frame, rig.camera);
        let words = cache_words(&rig.level, &rig.frame);
        let (n, split) = frame_dot_split(&px, &words);
        assert_eq!(
            split[0] + split[1],
            n,
            "frame {f}: every dot is young or mature"
        );
        seen += n;
        if f <= 8 {
            dots.counts.push(n);
            for (a, b) in dots.by_cell.iter_mut().zip(split) {
                *a += b;
            }
        } else {
            floor += n;
            for (a, b) in floor_cell.iter_mut().zip(split) {
                *a += b;
            }
        }
    }
    dots.floor = floor as f64 / 8.0;
    dots.by_cell_floor = floor_cell.map(|c| c as f64 / 8.0);
    let [young, mature] = dots.cell_excess();
    println!(
        "dots {seen} over the 16 frames; excess {:.3}: young {young:.3}, mature {mature:.3}",
        dots.excess_sum()
    );
    assert!(seen > 0, "the box shows dots to split");
    assert!(
        (young + mature - dots.excess_sum()).abs() < 1e-9,
        "young {young} and mature {mature} sum to the excess {}",
        dots.excess_sum()
    );
}

/// Each lighting pixel of the rig's frame with a face key: its index and key.
fn keyed_pixels(rig: &CacheBox) -> Vec<(usize, FaceKey)> {
    let (d, q) = (&rig.level.device, &rig.level.queue);
    let pos = vxe_render::readback::read_float(d, q, rig.frame.hit_positions());
    let info = vxe_render::readback::read_rgba16u(d, q, rig.frame.hit_records());
    (0..pos.len())
        .filter_map(|i| face_key(rig.camera.position, pos[i], info[i]).map(|k| (i, k)))
        .collect()
}

/// A cell's mean irradiance from the table's words: the sums over the count (at most the cap).
fn cell_mean(words: &[u32], w: usize, cap: u32) -> [f64; 3] {
    let c = vxe_render::marcher::CACHE_COUNT_WORD;
    let n = f64::from(words[w + c].min(cap)) * f64::from(vxe_render::marcher::CACHE_Q);
    [1, 2, 3].map(|k| f64::from(words[w + c + k]) / n)
}

/// One `f16` step at `v`: the spacing of half-precision values around it.
fn f16_step(v: f64) -> f64 {
    2f64.powi(v.abs().max(6.1e-5).log2().floor() as i32 - 10)
}

/// A face reads its cell's mean from its first sample (ADR-0043): on the box at scale 1, no
/// jitter, lamp off, exposure 1, on the first frame every pixel on a face carries the same
/// emissive value, its cell's sums over its count within one `f16` step; once every cell in
/// view is capped (`C_CAP` 64, after 64 frames), the next 64 frames are bit-identical.
#[test]
fn a_face_reads_its_mean_from_the_first_sample() {
    let Some(mut rig) = cache_box_rig(64) else {
        return;
    };
    rig.step(1);
    let keyed = keyed_pixels(&rig);
    assert!(keyed.len() > 500, "{} keyed pixels", keyed.len());
    let words = cache_words(&rig.level, &rig.frame);
    let acc = vxe_render::readback::read_float(
        &rig.level.device,
        &rig.level.queue,
        rig.frame.accumulated(),
    );
    let mut per_face: std::collections::HashMap<FaceKey, [f32; 3]> = Default::default();
    let mut lit = 0;
    for &(i, k) in &keyed {
        let w = cache_cell(&words, k).unwrap_or_else(|| panic!("key {k:?} has a cell"));
        let mean = cell_mean(&words, w, 64);
        let e = [acc[i][0], acc[i][1], acc[i][2]];
        for c in 0..3 {
            let got = f64::from(e[c]);
            assert!(
                (got - mean[c]).abs() <= f16_step(mean[c]),
                "pixel {i} key {k:?} channel {c}: {got} against the cell's {}",
                mean[c]
            );
        }
        let first = *per_face.entry(k).or_insert(e);
        assert_eq!(first, e, "pixel {i}: face {k:?} carries one value");
        lit += usize::from(e[0] > 0.0);
    }
    assert!(lit > 100, "{lit} lit face pixels");
    rig.step(63);
    let words = cache_words(&rig.level, &rig.frame);
    for (_, k) in &keyed {
        assert!(cache_count(&words, *k) >= 64, "key {k:?} is capped");
    }
    let a0 = vxe_render::readback::read_float(
        &rig.level.device,
        &rig.level.queue,
        rig.frame.accumulated(),
    );
    for f in 0..64 {
        rig.step(1);
        let (d, q) = (&rig.level.device, &rig.level.queue);
        let a = vxe_render::readback::read_float(d, q, rig.frame.accumulated());
        for &(i, _) in &keyed {
            assert_eq!(a[i][0..3], a0[i][0..3], "frame {f}: pixel {i} moved");
        }
    }
}

/// A hit with no face (behind a translucent slab) takes the fallback: on the box with a glass
/// layer over the rock block's top, a translucent hit's emissive part after 8 frames equals the
/// closed-form blend of its own samples, `e += max(0.2, 1 / (age + 1)) (s - e)`, within 1 %,
/// and its key holds no cell.
#[test]
fn a_hit_with_no_face_takes_the_fallback() {
    use vxe_world::*;
    let table =
        flythrough::castle_materials(&flythrough::with_emissive_ore(&flythrough::cave_materials()));
    let (world, camera) = cache_box_in(&table);
    world
        .apply(&EditOp::box_(
            Fixed3([40, 40, 64].map(|v| v * FIXED_ONE)),
            Fixed3([64, 64, 65].map(|v| v * FIXED_ONE)),
            Mode::Fill(flythrough::castle::material(flythrough::castle::GLASS)),
        ))
        .expect("the slab is resident");
    let Some(mut rig) = cache_box_rig_on(world, camera, 4096, [64, 36], None) else {
        return;
    };
    let mut expect: Vec<[f64; 3]> = Vec::new();
    let mut glass: Vec<usize> = Vec::new();
    for f in 0..8 {
        rig.step(1);
        let (d, q) = (&rig.level.device, &rig.level.queue);
        let s = vxe_render::readback::read_float(d, q, rig.frame.irradiance());
        if f == 0 {
            let info = vxe_render::readback::read_rgba16u(d, q, rig.frame.hit_records());
            glass = (0..info.len())
                .filter(|&i| info[i][3] & 1 == 1 && info[i][3] & 4 != 0 && s[i][0] > 0.0)
                .collect();
            assert!(glass.len() > 20, "{} lit translucent hits", glass.len());
            expect = glass
                .iter()
                .map(|&i| [0, 1, 2].map(|c| f64::from(s[i][c])))
                .collect();
        } else {
            let a = (1.0f64 / (f as f64 + 1.0)).max(0.2);
            for (e, &i) in expect.iter_mut().zip(&glass) {
                for c in 0..3 {
                    e[c] += a * (f64::from(s[i][c]) - e[c]);
                }
            }
        }
    }
    let (d, q) = (&rig.level.device, &rig.level.queue);
    let acc = vxe_render::readback::read_float(d, q, rig.frame.accumulated());
    let pos = vxe_render::readback::read_float(d, q, rig.frame.hit_positions());
    let info = vxe_render::readback::read_rgba16u(d, q, rig.frame.hit_records());
    let words = cache_words(&rig.level, &rig.frame);
    for (e, &i) in expect.iter().zip(&glass) {
        for c in 0..3 {
            let got = f64::from(acc[i][c]);
            assert!(
                (got - e[c]).abs() <= 0.01 * e[c].abs().max(1e-4),
                "pixel {i} channel {c}: {got} against the blend's {}",
                e[c]
            );
        }
        let mut bare = info[i];
        bare[3] &= !4;
        let k = face_key(rig.camera.position, pos[i], bare).expect("a cube face under the glass");
        assert!(cache_cell(&words, k).is_none(), "key {k:?} holds a cell");
    }
}

/// The sweep frees the cells beyond the light radius: on the box with a second rock box in the
/// chunk the camera moves into and `light_radius` 1, after
/// the camera moves one chunk along +x, a face whose chunk lies beyond one chunk of the new
/// camera chunk counts only this frame's pixels (0 out of view), and one within it its old count
/// plus them.
#[test]
fn the_sweep_clears_cells_beyond_the_light_radius() {
    use vxe_world::*;
    let (world, camera) = cache_box();
    // A rock box in chunk (1, 0, 1), in view: within the radius of the camera's chunk after the
    // move, while the block's chunk (0, 0, 0) lies beyond it.
    world
        .apply(&EditOp::box_(
            Fixed3([66, 30, 64].map(|v| v * FIXED_ONE)),
            Fixed3([70, 40, 66].map(|v| v * FIXED_ONE)),
            Mode::Fill(flythrough::cave::ROCK),
        ))
        .expect("the box is resident");
    let Some(mut rig) = cache_box_rig_on(world, camera, 4096, [64, 36], Some(1)) else {
        return;
    };
    rig.step(4);
    let before = cache_words(&rig.level, &rig.frame);
    let keys: Vec<FaceKey> = rig.keys().into_keys().collect();
    rig.camera.position.x += 64.0;
    rig.step(1);
    let now = rig.keys();
    let words = cache_words(&rig.level, &rig.frame);
    let cam = rig
        .camera
        .position
        .to_array()
        .map(|v| (v / 64.0).floor() as i32);
    let (mut far, mut near) = (0, 0);
    for k in &keys {
        let d2: i32 = (0..3)
            .map(|a| (k.0[a].div_euclid(64) - cam[a]).pow(2))
            .sum();
        let here = now.get(k).copied().unwrap_or(0);
        if d2 > 1 {
            far += 1;
            assert_eq!(
                cache_count(&words, *k),
                here,
                "key {k:?} beyond the radius restarts"
            );
        } else {
            near += 1;
            assert_eq!(
                cache_count(&words, *k),
                cache_count(&before, *k) + here,
                "key {k:?} within the radius keeps its count"
            );
        }
    }
    assert!(
        far > 0 && near > 0,
        "{far} faces beyond the radius, {near} within"
    );
}

/// A full table counts its misses: at 64 cells, a view of more than 64 faces reads a miss
/// count equal to the lighting pixels whose key found no cell, and those pixels show their own
/// sample (the fallback, on the first frame the accumulation itself), some of it lit.
#[test]
fn a_full_table_counts_its_misses() {
    let Some(mut rig) = cache_box_rig(4096) else {
        return;
    };
    rig.frame.set_lighting(Lighting {
        table_cells: 64,
        ..rig.frame.lighting()
    });
    rig.step(1);
    let (d, q) = (&rig.level.device, &rig.level.queue);
    let keyed = keyed_pixels(&rig);
    let words = cache_words(&rig.level, &rig.frame);
    assert_eq!(cache_cells(&words), 64);
    let faces: std::collections::HashSet<FaceKey> = keyed.iter().map(|&(_, k)| k).collect();
    assert!(faces.len() > 64, "{} faces in view", faces.len());
    let missed: Vec<usize> = keyed
        .iter()
        .filter(|&&(_, k)| cache_cell(&words, k).is_none())
        .map(|&(i, _)| i)
        .collect();
    let missed_faces: std::collections::HashSet<FaceKey> = keyed
        .iter()
        .filter(|&&(_, k)| cache_cell(&words, k).is_none())
        .map(|&(_, k)| k)
        .collect();
    let stats = rig.frame.read_table_stats(d, q);
    println!(
        "full table: {} faces in view, {} occupied, {} missed pixels on {} faces, counter {}",
        faces.len(),
        stats.occupied,
        missed.len(),
        missed_faces.len(),
        stats.misses
    );
    assert_eq!(stats.cells, 64);
    assert_eq!(stats.occupied, 64, "every cell holds a key");
    assert_eq!(
        stats.misses as usize,
        missed.len(),
        "the counter counts the missed pixels"
    );
    let acc = vxe_render::readback::read_float(d, q, rig.frame.accumulated());
    let raw = vxe_render::readback::read_float(d, q, rig.frame.irradiance());
    for &i in &missed {
        assert_eq!(acc[i][0..3], raw[i][0..3], "pixel {i} shows its own sample");
    }
    assert!(
        missed.iter().any(|&i| acc[i][0] > 0.0),
        "the missed pixels' fallback is lit"
    );
}

/// The words of the table with `cells` cells after `keys` are each written with a count of 9 at
/// its first free probed slot.
fn table_with(cells: u32, keys: &[FaceKey]) -> Vec<u32> {
    use vxe_render::marcher::{
        CACHE_CELL_WORDS, CACHE_COUNT_WORD, CACHE_PROBES, cache_key, cache_slot,
    };
    let n = CACHE_CELL_WORDS as usize;
    let mut words = vec![0u32; cells as usize * n];
    for k in keys {
        let slot = cache_slot(k.0, u32::from(k.1), cells);
        let w = (0..CACHE_PROBES)
            .map(|i| (slot.wrapping_add(i) & (cells - 1)) as usize * n)
            .find(|&w| words[w + 1] == 0)
            .expect("a free slot");
        let key = cache_key(k.0, u32::from(k.1));
        words[w] = key[0];
        words[w + 1] = key[1];
        words[w + CACHE_COUNT_WORD] = 9;
    }
    words
}

/// An edit box of any size clears every cell: with the table's dispatch rows capped at 4
/// groups, a box of 10 x 10 x 10 voxels (12^3 dilated, 27 groups, so 7 rows) clears all 6000
/// cells of its voxels' faces, and a cell two voxels past the dilation keeps its count.
#[test]
fn a_large_edit_box_clears_every_cell() {
    let Some(mut rig) = cache_box_rig(4096) else {
        return;
    };
    rig.frame.set_table_dispatch_cap(4);
    // Out of view: behind and below the camera, in air.
    let lo = [100, 100, 10];
    let mut keys = Vec::new();
    for x in 0..10 {
        for y in 0..10 {
            for z in 0..10 {
                for f in 0..6u8 {
                    keys.push(([lo[0] + x, lo[1] + y, lo[2] + z], f));
                }
            }
        }
    }
    let outside = ([lo[0] + 12, lo[1], lo[2]], 0u8);
    let mut all = keys.clone();
    all.push(outside);
    rig.step(1);
    let cells = rig.frame.lighting().cells();
    let bytes: Vec<u8> = table_with(cells, &all)
        .iter()
        .flat_map(|w| w.to_le_bytes())
        .collect();
    rig.level
        .queue
        .write_buffer(rig.frame.emissive_table(), 0, &bytes);
    rig.frame.invalidate(lo, [lo[0] + 9, lo[1] + 9, lo[2] + 9]);
    rig.step(1);
    let words = cache_words(&rig.level, &rig.frame);
    assert_eq!(keys.len(), 6000);
    for k in &keys {
        assert_eq!(cache_count(&words, *k), 0, "key {k:?} cleared");
        assert!(cache_cell(&words, *k).is_some(), "key {k:?} keeps its slot");
    }
    assert_eq!(
        cache_count(&words, outside),
        9,
        "the cell past the box keeps its count"
    );
}

/// A frame with more than `MAX_EDITS` boxes clears the whole table: after 8 frames on the box,
/// `MAX_EDITS + 1` one-voxel boxes out of view and a frame looking out of the world from its
/// corner (no hit adds a sample) leave every cell at count 0.
#[test]
fn an_overflowed_edit_frame_clears_the_table() {
    let Some(mut rig) = cache_box_rig(4096) else {
        return;
    };
    rig.step(8);
    let words = cache_words(&rig.level, &rig.frame);
    assert!(
        cache_census(&words, 1, 4096)[0] > 100,
        "the view fills cells"
    );
    for i in 0..=vxe_render::MAX_EDITS as i32 {
        rig.frame.invalidate([100 + i, 100, 10], [100 + i, 100, 10]);
    }
    // In the world's corner, facing out of it: every ray leaves the window.
    rig.camera = Camera {
        position: [4.5, 4.5, 30.5].into(),
        yaw: 225f32.to_radians(),
        pitch: 0.0,
        ..rig.camera
    };
    rig.step(1);
    assert!(
        rig.keys().is_empty(),
        "the view out of the world holds no face"
    );
    let words = cache_words(&rig.level, &rig.frame);
    let n = vxe_render::marcher::CACHE_COUNT_WORD;
    let held = words
        .chunks_exact(vxe_render::marcher::CACHE_CELL_WORDS as usize)
        .filter(|c| c[n] != 0)
        .count();
    assert_eq!(held, 0, "{held} cells kept a count");
}

/// The fast yaw's figures ([`fast_yaw_excess`]): the excess dots over frames 1 to 60, the
/// floor's mean and spread, the floor's frame count, the dot frames run, and the excess split
/// by `own_class` (no cell, off its own cell, own), each class's floor mean taken off its count.
struct FastYaw {
    excess: f64,
    floor: f64,
    sd: f64,
    floor_frames: usize,
    ran: usize,
    by_own: [f64; 3],
}

/// The display's excess dots in frames 1 to 60 after a 90 degree yaw onto `VEIN_POSE` over 10
/// frames, lamp on, indirect off, exposure pinned, FSR 2 at 1.5 when `upscaler` (else scale
/// 1), lighting at `lighting`, the table's read `table`: settled 192 frames at the pose, its
/// floor read over 64 more; then 64 frames at the yaw's start, the yaw, and the 60 dot frames.
/// With `split` each read frame's dots are split by `own_class` too.
fn fast_yaw_excess(
    level: &stillness::VeinLevel,
    table: bool,
    upscaler: bool,
    lighting: LightingResolution,
    split: bool,
) -> FastYaw {
    let setting = Setting {
        lamp: true,
        indirect: false,
        auto: false,
        upscaler,
        bridge: false,
        steady: Steady {
            table,
            ..Steady::default()
        },
    };
    let mut frame = make_frame(level, &VEIN_POSE, setting);
    frame.set_resolution(&level.device, lighting);
    let light = LightParams::headlamp();
    let still = VEIN_POSE.camera();
    let step = |frame: &mut Frame, c: &Camera| {
        frame.step(&level.device, &level.queue, &level.gpu, c, &light)
    };
    let read = |frame: &Frame| -> (f64, [f64; 3]) {
        let px = read_dots(level, frame, still);
        if !split {
            return (dots_of(&px) as f64, [0.0; 3]);
        }
        let words = cache_words(level, frame);
        let (all, _) = crease_frame(&px, &words, VEIN_POSE.exposure);
        (all.iter().sum::<usize>() as f64, all.map(|v| v as f64))
    };
    for _ in 0..192 {
        step(&mut frame, &still);
    }
    let mut floor = Vec::new();
    let mut floor_own = [0.0f64; 3];
    for _ in 0..64 {
        step(&mut frame, &still);
        let (d, o) = read(&frame);
        floor.push(d);
        for k in 0..3 {
            floor_own[k] += o[k] / 64.0;
        }
    }
    for _ in 0..64 {
        step(&mut frame, &yawed(&VEIN_POSE, -90.0));
    }
    for i in 1..=10 {
        step(&mut frame, &yawed(&VEIN_POSE, -90.0 + 9.0 * i as f32));
    }
    let mean = floor.iter().sum::<f64>() / floor.len() as f64;
    let spread =
        (floor.iter().map(|f| (f - mean).powi(2)).sum::<f64>() / floor.len() as f64).sqrt();
    let mut excess = 0.0;
    let mut by_own = [0.0f64; 3];
    let mut ran = 0;
    for _ in 1..=DOT_FRAMES {
        step(&mut frame, &still);
        let (d, o) = read(&frame);
        excess += d - mean;
        for k in 0..3 {
            by_own[k] += o[k] - floor_own[k];
        }
        ran += 1;
    }
    FastYaw {
        excess,
        floor: mean,
        sd: spread,
        floor_frames: floor.len(),
        ran,
        by_own,
    }
}

/// The fast yaw's excess dots by the table's read, a reading: at `VEIN_POSE`, lamp on, indirect
/// off, FSR 2 at 1.5, exposure pinned, after a 90 degree yaw over 10 frames, frames 1 to 60,
/// the default and the read off (`table_read: false`) each print their excess, floor, floor
/// spread and noise line (60 floor spreads). It asserts only that each floor was measured (a
/// non-zero still-frame dot count over 64 frames, with its spread) and that the 60 dot frames
/// ran; no excess is asserted.
#[test]
fn the_fast_yaw_excess_by_table_read() {
    let Some(level) = vein_level() else {
        return;
    };
    for (name, table) in [("table", true), ("read off", false)] {
        let FastYaw {
            excess,
            floor,
            sd,
            floor_frames,
            ran,
            ..
        } = fast_yaw_excess(&level, table, true, LightingResolution::Half, false);
        println!(
            "reading: fast yaw excess dots 1-60, {name}: {excess:.1} (floor {floor:.1}, sd {sd:.1}, line {:.1})",
            DOT_FRAMES as f64 * sd
        );
        assert_eq!(floor_frames, 64, "{name}: the floor's frames");
        assert!(
            floor > 0.0 && sd.is_finite(),
            "{name}: the floor was measured: {floor} (sd {sd})"
        );
        assert_eq!(ran, DOT_FRAMES, "{name}: the dot frames run");
    }
}

/// The creases (Plan 0057 Phase 4 Part 1), a reading: Plan 0056's fast yaw (`crease_yaw_run`,
/// settle 512, 240 frames at 10 display pixels a frame) at each pose of `VXE_SETTLE_POSES`
/// (default both), FSR 2 at 1.5 and scale 1, lamp off, indirect off, pinned, lighting half (the
/// default) and full: the yellow dots a frame and their split by their own face's cell.
#[test]
#[ignore = "reading only: run with --ignored"]
fn crease_readings() {
    let Some(level) = reading_level() else {
        return;
    };
    let adapter = level.adapter.clone();
    let want = list("VXE_SETTLE_POSES", &["patch", "vein"]);
    for pose in POSES.iter().filter(|p| want.iter().any(|w| w == p.name)) {
        for (up, upscaler) in [("fsr2", true), ("native", false)] {
            for lighting in [LightingResolution::Half, LightingResolution::Full] {
                let setting = Setting {
                    lamp: false,
                    indirect: false,
                    auto: false,
                    upscaler,
                    bridge: false,
                    steady: Steady::default(),
                };
                let line = crease_yaw_run(&level, pose, setting, lighting, 512, 10.0, 240);
                println!(
                    "reading[{adapter}]: crease {} {up} lighting {}: {line}",
                    pose.name,
                    lighting.name()
                );
            }
        }
    }
}

/// The rig's row of the creases, a reading: `the_fast_yaw_excess_by_table_read`'s default run
/// (table read on) at FSR 2 at 1.5 and scale 1, lighting half and full, its excess dots split by
/// their own face's cell (`own_class`).
#[test]
#[ignore = "reading only: run with --ignored"]
fn fast_yaw_split_readings() {
    let Some(level) = reading_level() else {
        return;
    };
    let adapter = level.adapter.clone();
    for (up, upscaler) in [("up1.5", true), ("scale1", false)] {
        for lighting in [LightingResolution::Half, LightingResolution::Full] {
            let r = fast_yaw_excess(&level, true, upscaler, lighting, true);
            println!(
                "reading[{adapter}]: fast yaw split {up} lighting {}: excess {:.1} (floor {:.1}, sd {:.1}, line {:.1}, frames {}/{}); by own: {} {:.1}, {} {:.1}, {} {:.1}",
                lighting.name(),
                r.excess,
                r.floor,
                r.sd,
                DOT_FRAMES as f64 * r.sd,
                r.floor_frames,
                r.ran,
                OWN_CLASSES[0],
                r.by_own[0],
                OWN_CLASSES[1],
                r.by_own[1],
                OWN_CLASSES[2],
                r.by_own[2]
            );
        }
    }
}

/// The guard's box: in one rock chunk, an emitter cell at x 4..14 (y and z 20..40) with an ore
/// wall at x = 3 facing +x, a one-voxel rock partition at x = 14 when `plug`, and a far cell at
/// x 15..60 behind it; the camera 8 voxels from the far wall at x = 60, facing it (arm's
/// length: a voxel face covers about 200 pixels at the rig's 192 x 108). Returns the world, the
/// plug's carve, the emitter wall's replacement by rock and the camera.
fn guard_box(
    plug: bool,
) -> (
    vxe_world::World,
    vxe_world::EditOp,
    vxe_world::EditOp,
    Camera,
) {
    use flythrough::cave::{ORE, ROCK};
    use vxe_world::*;
    let table = flythrough::with_emissive_ore(&flythrough::cave_materials());
    let op = |min: [i64; 3], max: [i64; 3], mode| {
        EditOp::box_(
            Fixed3(min.map(|v| v * FIXED_ONE)),
            Fixed3(max.map(|v| v * FIXED_ONE)),
            mode,
        )
    };
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [1, 1, 1]),
        &UniformSource(ROCK),
        &table,
    );
    let carve = op([14, 20, 20], [15, 40, 40], Mode::Carve);
    let edits = [
        op([4, 20, 20], [14, 40, 40], Mode::Carve),
        op(
            [3, 20, 20],
            [4, 40, 40],
            Mode::Replace {
                from: ROCK,
                to: ORE,
            },
        ),
        op([15, 20, 20], [60, 40, 40], Mode::Carve),
    ];
    for e in &edits {
        world.apply(e).expect("the box is resident");
    }
    if !plug {
        world.apply(&carve).expect("the box is resident");
    }
    let to_rock = op(
        [3, 20, 20],
        [4, 40, 40],
        Mode::Replace {
            from: ORE,
            to: ROCK,
        },
    );
    let camera = Camera {
        position: [52.0, 30.0, 30.0].into(),
        yaw: 0.0,
        pitch: 0.0,
        vfov: 50f32.to_radians(),
    };
    (world, carve, to_rock, camera)
}

/// Cells in the guard box's table: few, so a frame's read of the table stays small.
const GUARD_CELLS: u32 = 1 << 14;

/// The guard box (`guard_box`) under a frame at the rig's 192 x 108, scale 1, lamp off, the
/// table at GUARD_CELLS cells and its cap `cap`.
fn guard_rig(plug: bool, cap: u32) -> Option<(CacheBox, vxe_world::EditOp, vxe_world::EditOp)> {
    let (world, carve, to_rock, camera) = guard_box(plug);
    let mut rig = cache_box_rig_on(world, camera, cap, [192, 108], None)?;
    rig.frame.set_lighting(Lighting {
        lamp: false,
        indirect: false,
        exposure: Exposure::Manual(1.0),
        table_cap: cap,
        table_cells: GUARD_CELLS,
        ..Lighting::default()
    });
    Some((rig, carve, to_rock))
}

/// The far wall's face keys in view with at least one gradient stratum's 9 pixels each.
fn far_wall_keys(rig: &CacheBox) -> Vec<FaceKey> {
    let mut keys: Vec<FaceKey> = rig
        .keys()
        .into_iter()
        .filter(|&((v, f), n)| v[0] == 60 && f == 0 && n >= 9)
        .map(|(k, _)| k)
        .collect();
    keys.sort_unstable();
    keys
}

/// A cell's mean luminance (`cell_mean`), 0 for a key with no cell or no sample.
fn cell_lum(words: &[u32], key: FaceKey, cap: u32) -> f64 {
    match cache_cell(words, key) {
        Some(w) if words[w + vxe_render::marcher::CACHE_COUNT_WORD] > 0 => {
            let m = cell_mean(words, w, cap);
            0.2126 * m[0] + 0.7152 * m[1] + 0.0722 * m[2]
        }
        _ => 0.0,
    }
}

/// The mean over `keys` of their cells' mean luminance.
fn wall_lum(words: &[u32], keys: &[FaceKey], cap: u32) -> f64 {
    keys.iter().map(|&k| cell_lum(words, k, cap)).sum::<f64>() / keys.len() as f64
}

/// Applies `op` to the rig's world, syncs the mirror and invalidates the edit's box.
fn guard_edit(rig: &mut CacheBox, op: &vxe_world::EditOp) {
    let r = rig.level.world.apply(op).expect("the box is resident");
    let aabb = r.aabb.expect("the edit changes voxels");
    rig.level
        .gpu
        .sync_now(&rig.level.world, &rig.level.device, &rig.level.queue)
        .expect("the mirror fits");
    rig.frame.invalidate(aabb.0.0, aabb.1.0);
}

/// Steps the rig `frames` frames after an edit, reading the table each frame: the frame (1 on)
/// each key's count first fell below the frame before's (a reopen), and the words after each
/// frame.
fn reopen_series(
    rig: &mut CacheBox,
    keys: &[FaceKey],
    frames: usize,
) -> (Vec<Option<usize>>, Vec<Vec<u32>>) {
    let mut prev: Vec<u32> = {
        let w = cache_words(&rig.level, &rig.frame);
        keys.iter().map(|&k| cache_count(&w, k)).collect()
    };
    let mut at = vec![None; keys.len()];
    let mut series = Vec::with_capacity(frames);
    for f in 1..=frames {
        rig.step(1);
        let w = cache_words(&rig.level, &rig.frame);
        for (i, &k) in keys.iter().enumerate() {
            let c = cache_count(&w, k);
            if c < prev[i] && at[i].is_none() {
                at[i] = Some(f);
            }
            prev[i] = c;
        }
        series.push(w);
    }
    (at, series)
}

/// The guard box's cap: the far wall's faces of about 200 pixels cap within two frames.
const GUARD_CAP: u32 = 256;

/// A revealed emitter reopens the cells it lights (ADR-0043's guard): on the guard box with
/// the plug in, lamp off, scale 1, the far wall's cells at arm's length (the faces with at
/// least 9 pixels) are capped and dark after 64 frames; after the plug's carve, which lies
/// outside every far-wall cell's edit box, every one of them reopens within 9 frames, and the
/// wall's mean luminance reads within 10 % of a fresh run's (the plug carved before the first
/// frame, 64 frames) by frame 16. About 2 x 80 frames.
#[test]
fn a_revealed_emitter_reopens_the_cells_it_lights() {
    let Some((mut fresh, _, _)) = guard_rig(false, GUARD_CAP) else {
        return;
    };
    fresh.step(64);
    let keys = far_wall_keys(&fresh);
    assert!(keys.len() >= 40, "{} far-wall keys", keys.len());
    let target = wall_lum(&cache_words(&fresh.level, &fresh.frame), &keys, GUARD_CAP);
    assert!(target > 0.0, "the fresh run lights the wall");
    drop(fresh);
    let (mut rig, carve, _) = guard_rig(true, GUARD_CAP).expect("the device opened once");
    rig.step(64);
    assert_eq!(far_wall_keys(&rig), keys, "the same faces in view");
    let words = cache_words(&rig.level, &rig.frame);
    for &k in &keys {
        assert!(cache_count(&words, k) >= GUARD_CAP, "key {k:?} is capped");
        assert_eq!(cell_lum(&words, k, GUARD_CAP), 0.0, "key {k:?} is dark");
    }
    guard_edit(&mut rig, &carve);
    let (at, series) = reopen_series(&mut rig, &keys, 16);
    let lums: Vec<f64> = series
        .iter()
        .map(|w| wall_lum(w, &keys, GUARD_CAP))
        .collect();
    let last = at.iter().map(|a| a.unwrap_or(17)).max().unwrap_or(0);
    let within = lums
        .iter()
        .position(|l| (l - target).abs() <= 0.1 * target)
        .map(|f| f + 1);
    println!(
        "reading: reveal at arm's length, {} keys: the last reopens at frame {last}; the wall's mean within 10 % of the fresh run's {target:.5} at frame {within:?}; means {lums:.5?}",
        keys.len()
    );
    assert!(
        last <= 9,
        "every far-wall cell reopens within 9 frames: {at:?}"
    );
    let l16 = lums[15];
    assert!(
        (l16 - target).abs() <= 0.1 * target,
        "the wall's mean at frame 16 {l16:.5} against the fresh run's {target:.5}"
    );
}

/// A removed emitter darkens the cells it lit: on the guard box without the plug, lamp off,
/// scale 1, after 64 frames the far wall's cells at arm's length are lit; replacing the emitter
/// wall with rock reopens every one of them within 9 frames, and by frame 16 each one's mean
/// reads under 1 % of its lit value. About 80 frames.
#[test]
fn a_removed_emitter_darkens_the_cells_it_lit() {
    let Some((mut rig, _, to_rock)) = guard_rig(false, GUARD_CAP) else {
        return;
    };
    rig.step(64);
    let keys = far_wall_keys(&rig);
    assert!(keys.len() >= 40, "{} far-wall keys", keys.len());
    let words = cache_words(&rig.level, &rig.frame);
    let lit: Vec<f64> = keys
        .iter()
        .map(|&k| cell_lum(&words, k, GUARD_CAP))
        .collect();
    for (k, l) in keys.iter().zip(&lit) {
        assert!(*l > 0.0, "key {k:?} is lit");
    }
    guard_edit(&mut rig, &to_rock);
    let (at, series) = reopen_series(&mut rig, &keys, 16);
    let last = at.iter().map(|a| a.unwrap_or(17)).max().unwrap_or(0);
    let worst = keys
        .iter()
        .zip(&lit)
        .map(|(&k, l)| cell_lum(&series[15], k, GUARD_CAP) / l)
        .fold(0.0, f64::max);
    println!(
        "reading: removed emitter at arm's length, {} keys: the last reopens at frame {last}; the largest share of its lit value at frame 16 {worst:.5}",
        keys.len()
    );
    assert!(
        last <= 9,
        "every far-wall cell reopens within 9 frames: {at:?}"
    );
    assert!(worst < 0.01, "a cell keeps {worst:.4} of its lit value");
}

/// A still scene reopens no cell: on the guard box without the plug, lamp off, scale 1, the
/// default cap, over 64 still frames from the first every cell in view's count only grows and
/// the table's reopen counter reads 0. About 64 frames.
#[test]
fn a_still_scene_reopens_no_cell() {
    let Some((mut rig, _, _)) = guard_rig(false, vxe_render::marcher::DEFAULT_TABLE_CAP) else {
        return;
    };
    rig.step(1);
    let mut keys: Vec<FaceKey> = rig.keys().into_keys().collect();
    keys.sort_unstable();
    assert!(keys.len() >= 100, "{} keys in view", keys.len());
    let (at, _) = reopen_series(&mut rig, &keys, 63);
    let fell: Vec<_> = keys.iter().zip(&at).filter(|(_, a)| a.is_some()).collect();
    assert!(fell.is_empty(), "counts fell: {fell:?}");
    let reopens = rig
        .frame
        .read_table_reopens(&rig.level.device, &rig.level.queue);
    assert_eq!(reopens, 0, "the guard reopened {reopens} cells");
}

/// The still hall by pixel class (Plan 0057 Phase 4 Part 1), a reading: the showcase's `hall`
/// shot as `hero --exposure 8` renders it, native and FSR 2 at 1.5 (`VXE_HALL_SCALES`, default
/// both), lamp off, the drift and the one frame split by `HALL_CLASSES` (`hall_class_run`).
#[test]
#[ignore = "reading only: run with --ignored"]
fn hall_class_readings() {
    use flythrough::showcase::{SHOTS, ShowcaseSource};
    use vxe_render::gpu_world::{GpuWorld, MirrorConfig};
    let source = ShowcaseSource::new(1);
    let world = vxe_world::World::generate(
        source.extent(),
        &source,
        &flythrough::with_showcase_rows(&flythrough::with_emissive_ore(
            &flythrough::cave_materials(),
        )),
    );
    let pool = flythrough::level_pool_units(&world, false) * vxe_render::gpu_world::UNIT_BYTES;
    let Some((device, queue, adapter)) = reading_device(pool) else {
        return;
    };
    let chunks = world.extent().len() as u64;
    let cfg = MirrorConfig {
        budget_bytes: flythrough::level_mirror_budget(&world, chunks, false),
        light_radius: 19,
        ..MirrorConfig::new(vxe_render::Window::Fixed(world.extent()))
    };
    let mut gpu = GpuWorld::with_config(&device, cfg);
    let shot = &SHOTS[0];
    gpu.set_focus(shot.eye);
    let mut last = None;
    for _ in 0..256 {
        gpu.sync_now(&world, &device, &queue)
            .expect("mirror fits the adapter");
        let n = gpu.light_samples();
        if last == Some(n) {
            break;
        }
        last = Some(n);
    }
    let camera = Camera {
        position: shot.eye.0.map(|v| v as f32 + 0.5).into(),
        yaw: (shot.yaw_deg as f32).to_radians(),
        pitch: (shot.pitch_deg as f32).to_radians(),
        vfov: 70f32.to_radians(),
    };
    for scale in list("VXE_HALL_SCALES", &["native", "up"]) {
        let line = hall_class_run(&device, &queue, &gpu, camera, scale == "native");
        println!("reading[{adapter}]: hall classes {scale}: {line}");
    }
}
