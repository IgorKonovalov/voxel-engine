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
        lamp_bounce: true,
        exposure: Exposure::Manual(VEIN_POSE.exposure),
        ..Lighting::default()
    });
    frame.set_dt(1.0 / 60.0);
    frame
}

/// Switching the lamp on opens the probe volume's light-change window on its first frame and
/// holds it for exactly `WINDOW_FRAMES` (16) frames, every one updating probes (the channel
/// fills from zero); switching it off runs no probe pass, so no frame traces a ray or opens a
/// window.
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
            assert_eq!(
                frame.traced_probe_rays() > 0,
                !from,
                "probes update while the lamp is on, and only then"
            );
            open.push(frame.probe_light_window());
        }
        println!("lamp {from} -> {}: window per frame {open:?}", !from);
        let want: Vec<bool> = (0..32)
            .map(|i| !from && i < vxe_render::probes::WINDOW_FRAMES as usize)
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

/// The face keys of every opaque voxel's face toward air within the cube of `2 * half` voxels
/// an edge centred on `centre` in `world` (the keys a table around a pose can be asked to
/// hold: `cache_key`'s voxel and entry face, `2 axis + 1` where the air lies toward `+axis`),
/// sorted. A voxel outside the world reads as rock.
fn air_faces(world: &vxe_world::World, centre: [f32; 3], half: i32) -> Vec<([i32; 3], u32)> {
    use vxe_world::AIR;
    let lo = centre.map(|c| c.floor() as i32 - half);
    let n = (2 * half + 2) as usize;
    let table = world.materials();
    // 0 rock (opaque), 1 air, 2 translucent, over the cube and a one-voxel rim.
    let mut cls = vec![0u8; n * n * n];
    for z in 0..n {
        for y in 0..n {
            for x in 0..n {
                let v = [
                    lo[0] - 1 + x as i32,
                    lo[1] - 1 + y as i32,
                    lo[2] - 1 + z as i32,
                ];
                cls[(z * n + y) * n + x] = match world.material_at(v) {
                    Some(AIR) => 1,
                    Some(m) if table.is_translucent(m) => 2,
                    _ => 0,
                };
            }
        }
    }
    let at = |x: usize, y: usize, z: usize| cls[(z * n + y) * n + x];
    let mut keys = Vec::new();
    for z in 1..n - 1 {
        for y in 1..n - 1 {
            for x in 1..n - 1 {
                if at(x, y, z) != 0 {
                    continue;
                }
                let v = [
                    lo[0] - 1 + x as i32,
                    lo[1] - 1 + y as i32,
                    lo[2] - 1 + z as i32,
                ];
                let around = [
                    (at(x - 1, y, z), 0),
                    (at(x + 1, y, z), 1),
                    (at(x, y - 1, z), 2),
                    (at(x, y + 1, z), 3),
                    (at(x, y, z - 1), 4),
                    (at(x, y, z + 1), 5),
                ];
                for (c, face) in around {
                    if c == 1 {
                        keys.push((v, face));
                    }
                }
            }
        }
    }
    keys.sort_unstable();
    keys
}

/// The preload's census around `eye` at a radius of `r` voxels: the opaque faces toward air
/// whose face centre lies within `r` of the eye (`air_faces`), and over the sweep's bricks
/// (`marcher::preload_offsets` at `r / 8` bricks around the eye's brick) how many end at once
/// (every voxel air; or one opaque material throughout with six such neighbour bricks), how
/// many are left to visit, and the faces the visited bricks hold. A voxel outside the world
/// reads as rock, as the mirror's absent chunks do.
fn preload_census(world: &vxe_world::World, eye: [f32; 3], r: u32) -> String {
    use std::collections::HashMap;
    use vxe_world::AIR;
    let table = world.materials();
    let faces = air_faces(world, eye, r as i32 + 1);
    let within = faces
        .iter()
        .filter(|(v, f)| {
            let n = [
                (-1.0f32, 0),
                (1.0, 0),
                (-1.0, 1),
                (1.0, 1),
                (-1.0, 2),
                (1.0, 2),
            ][*f as usize];
            let mut c = v.map(|x| x as f32 + 0.5);
            c[n.1] += 0.5 * n.0;
            let d: f32 = (0..3).map(|k| (c[k] - eye[k]).powi(2)).sum();
            d <= (r * r) as f32
        })
        .count();
    // 0 every voxel air, 1 one opaque material throughout, 2 anything else.
    let mut class: HashMap<[i32; 3], u8> = HashMap::new();
    let mut class_of = |b: [i32; 3]| -> u8 {
        *class.entry(b).or_insert_with(|| {
            let mut first = None;
            let mut uniform = true;
            for i in 0..512 {
                let v = [b[0] * 8 + i % 8, b[1] * 8 + (i / 8) % 8, b[2] * 8 + i / 64];
                let m = world.material_at(v);
                if first.is_none() {
                    first = Some(m);
                } else if first != Some(m) {
                    uniform = false;
                    break;
                }
            }
            match first.flatten() {
                _ if !uniform => 2,
                Some(AIR) => 0,
                Some(m) if table.is_translucent(m) => 2,
                _ => 1,
            }
        })
    };
    let eb = eye.map(|c| (c / 8.0).floor() as i32);
    let offsets = vxe_render::marcher::preload_offsets(r / 8);
    let (mut air, mut solid, mut left) = (0usize, 0usize, Vec::new());
    for o in &offsets {
        let b = [eb[0] + o[0], eb[1] + o[1], eb[2] + o[2]];
        match class_of(b) {
            0 => air += 1,
            1 if [
                [-1, 0, 0],
                [1, 0, 0],
                [0, -1, 0],
                [0, 1, 0],
                [0, 0, -1],
                [0, 0, 1],
            ]
            .iter()
            .all(|d| class_of([b[0] + d[0], b[1] + d[1], b[2] + d[2]]) == 1) =>
            {
                solid += 1;
            }
            _ => left.push(b),
        }
    }
    let left_set: std::collections::HashSet<[i32; 3]> = left.iter().copied().collect();
    let in_bricks = air_faces(world, eye, r as i32 + 16)
        .iter()
        .filter(|(v, _)| left_set.contains(&v.map(|c| c.div_euclid(8))))
        .count();
    format!(
        "R {r}: faces within {within}; bricks {} (air {air}, enclosed solid {solid}, left {}), faces in the bricks left {in_bricks}",
        offsets.len(),
        left.len()
    )
}

/// The preload's census (Plan 0057 Phase 4 Part 2b, done-when 12): `preload_census` at 64 and
/// 96 voxels around `VEIN_POSE`'s, `PATCH_POSE`'s and the showcase hall's eye, on the level and
/// the showcase at seed 1. No GPU.
#[test]
#[ignore = "reading only: run with --ignored"]
fn preload_face_census() {
    use flythrough::showcase::{SHOTS, ShowcaseSource};
    let source = flythrough::cave::CaveSource::new(
        1,
        vxe_world::ChunkExtent::new([0, 0, 0], flythrough::cave::LEVEL_CHUNKS),
    );
    let level = vxe_world::World::generate(
        source.extent,
        &source,
        &flythrough::with_emissive_ore(&flythrough::cave_materials()),
    );
    let show = ShowcaseSource::new(1);
    let hall = vxe_world::World::generate(
        show.extent(),
        &show,
        &flythrough::with_showcase_rows(&flythrough::with_emissive_ore(
            &flythrough::cave_materials(),
        )),
    );
    let hall_eye = SHOTS[0].eye.0.map(|v| v as f32 + 0.5);
    for (name, world, eye) in [
        ("vein", &level, VEIN_POSE.position),
        ("patch", &level, stillness::PATCH_POSE.position),
        ("hall", &hall, hall_eye),
    ] {
        for r in [64, 96] {
            println!("reading: census {name} {}", preload_census(world, eye, r));
        }
    }
}

/// The keys of a table fixture under `tests/fixtures/` (`table-keys-<name>.bin`: the keys a
/// run's table held, sorted as `z << 43 | y << 23 | x << 3 | face` with each coordinate's 20
/// bits, stored as the varint deltas between them), as voxels and faces.
fn fixture_keys(name: &str) -> Vec<([i32; 3], u32)> {
    let path = format!(
        "{}/tests/fixtures/table-keys-{name}.bin",
        env!("CARGO_MANIFEST_DIR")
    );
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let (mut out, mut prev, mut d, mut shift) = (Vec::new(), 0u64, 0u64, 0);
    for b in bytes {
        d |= u64::from(b & 0x7F) << shift;
        shift += 7;
        if b & 0x80 == 0 {
            prev += d;
            let c = |at: u32| ((prev >> at) & 0xF_FFFF) as i32;
            out.push(([c(3), c(23), c(43)], (prev & 7) as u32));
            (d, shift) = (0, 0);
        }
    }
    out
}

/// The probe count holds a window's key sets (Plan 0057 Phase 7 Part A): the keys the table
/// held at the end of the hall's, the vein's and the patch's runs at the window's size (2530 x
/// 1378 from 1687 x 919, half-resolution lighting; `strip.rs`'s `light_table` at df5158b,
/// `tests/fixtures/`), their union in key order cut at a load of 0.25 of
/// `DEFAULT_TABLE_CELLS`, inserted by linear probing through `marcher::cache_slot` at
/// `CACHE_PROBES`: no key is left without a slot (`the_table_hash_spreads_a_levels_keys`).
fn the_probe_count_holds_a_windows_keys() {
    use vxe_render::marcher::{CACHE_PROBES, DEFAULT_TABLE_CELLS, cache_slot};
    let mut keys: Vec<([i32; 3], u32)> = ["hall", "vein", "patch"]
        .iter()
        .flat_map(|n| fixture_keys(n))
        .collect();
    let counts: Vec<usize> = ["hall", "vein", "patch"]
        .iter()
        .map(|n| fixture_keys(n).len())
        .collect();
    assert_eq!(counts, [789_657, 193_409, 128_127], "the fixtures' keys");
    keys.sort_unstable_by_key(|&(v, f)| vxe_render::marcher::cache_key(v, f));
    keys.dedup();
    let n = DEFAULT_TABLE_CELLS as usize / 4;
    assert!(keys.len() >= n, "{} keys", keys.len());
    keys.truncate(n);
    let bases: Vec<u32> = keys
        .iter()
        .map(|&(v, f)| cache_slot(v, f, DEFAULT_TABLE_CELLS))
        .collect();
    let (missed, runs, mean, max) = probe_insert(&bases, DEFAULT_TABLE_CELLS);
    println!(
        "reading: {n} keys at {DEFAULT_TABLE_CELLS} cells, {CACHE_PROBES} probes: no slot {missed}; runs {runs} mean {mean:.3} max {max}"
    );
    assert_eq!(missed, 0, "keys without a slot");
}

/// The table's hash spreads a level's keys (ADR-0043's table): the key set of every opaque
/// face toward air within a 128-voxel cube around `PATCH_POSE` and around `VEIN_POSE` on the
/// level at seed 1 (`air_faces`), inserted in key order by linear probing through
/// `marcher::cache_slot` at `CACHE_PROBES`: at `DEFAULT_TABLE_CELLS` no key lacks a slot, and
/// at a load of 0.9 of the largest power of two that takes it, where 16 probes first leave keys
/// out, the share of keys left without a slot is at most twice a seeded uniform assignment's of
/// the same keys at that size (`uniform_slot`, seed 0x0057); and the window's key sets find a slot (`the_probe_count_holds_a_windows_keys`).
/// No GPU.
#[test]
fn the_table_hash_spreads_a_levels_keys() {
    the_probe_count_holds_a_windows_keys();
    use vxe_render::marcher::{DEFAULT_TABLE_CELLS, cache_slot};
    let source = flythrough::cave::CaveSource::new(
        1,
        vxe_world::ChunkExtent::new([0, 0, 0], flythrough::cave::LEVEL_CHUNKS),
    );
    let world = vxe_world::World::generate(
        source.extent,
        &source,
        &flythrough::with_emissive_ore(&flythrough::cave_materials()),
    );
    for pose in [stillness::PATCH_POSE, VEIN_POSE] {
        let keys = air_faces(&world, pose.position, 64);
        let n = keys.len();
        assert!(n > 10_000, "{}: {n} keys", pose.name);
        let bases = |cells: u32, n: usize| -> Vec<u32> {
            keys[..n]
                .iter()
                .map(|&(v, f)| cache_slot(v, f, cells))
                .collect()
        };
        let (full_missed, ..) = probe_insert(&bases(DEFAULT_TABLE_CELLS, n), DEFAULT_TABLE_CELLS);
        // The largest power of two whose 0.9 the keys fill, and the keys that fill it.
        let dense_cells = 1u32 << ((n as f64 / 0.9).log2().floor() as u32);
        let dense = (f64::from(dense_cells) * 0.9) as usize;
        let (hash_missed, _, hmean, hmax) = probe_insert(&bases(dense_cells, dense), dense_cells);
        let ubases: Vec<u32> = (0..dense as u64)
            .map(|i| uniform_slot(0x0057, i, dense_cells))
            .collect();
        let (uni_missed, _, umean, umax) = probe_insert(&ubases, dense_cells);
        println!(
            "reading: {} keys {n}; at {DEFAULT_TABLE_CELLS} cells no slot {full_missed}; {dense} keys at {dense_cells} cells no slot: hash {hash_missed} ({:.5}, runs mean {hmean:.3} max {hmax}), uniform {uni_missed} ({:.5}, runs mean {umean:.3} max {umax})",
            pose.name,
            hash_missed as f64 / dense as f64,
            uni_missed as f64 / dense as f64
        );
        assert_eq!(full_missed, 0, "{}: keys without a slot", pose.name);
        assert!(
            uni_missed > 0,
            "{}: the uniform run misses nothing",
            pose.name
        );
        assert!(
            hash_missed <= 2 * uni_missed,
            "{}: the hash leaves {hash_missed} keys without a slot, the uniform {uni_missed}",
            pose.name
        );
    }
}

/// The preload's radius in voxels the visibility reading counts against (`Lighting`'s
/// default `preload_radius`).
const VIS_R: f32 = 64.0;
/// The frames of history a face must be absent from to count as entering view.
const VIS_HISTORY: usize = 60;
/// The reading's default resolution (16:9 at the rig's 70-degree vertical field of view; the
/// fly-through's half-tier lighting grid at 1080p from 720p); `VXE_VIS_SIZE` (`WxH`) overrides.
const VIS_SIZE: [usize; 2] = [640, 360];

fn vis_size() -> [usize; 2] {
    std::env::var("VXE_VIS_SIZE")
        .ok()
        .and_then(|v| {
            let (w, h) = v.split_once('x')?;
            Some([w.parse().ok()?, h.parse().ok()?])
        })
        .unwrap_or(VIS_SIZE)
}
/// The primary rays' reach in voxels.
const VIS_REACH: f32 = 512.0;
/// The distance histogram's upper edges in voxels (the last bin is open).
const VIS_BINS: [f32; 5] = [8.0, 16.0, 32.0, 64.0, 128.0];

type VisKey = (vxe_world::VoxelCoord, vxe_world::Face);

/// The first voxel and entry face the ray from `eye` along `dir` (unit) hits within `reach`
/// voxels, with the hit point; `None` on a miss or a ray the world refuses.
fn vis_cast(world: &vxe_world::World, eye: Vec3, dir: Vec3, reach: f32) -> Option<(VisKey, Vec3)> {
    let origin = vxe_world::Fixed3((eye * 256.0).round().to_array().map(|c| c as i64));
    let d = (dir * 256.0).round();
    let len = d.length();
    let df = d.to_array().map(|c| c as i64);
    let hit = world
        .raycast(origin, df, (reach * 256.0 * 256.0 / len) as i64)
        .ok()??;
    let face = hit.face?;
    let at = Vec3::from(origin.0.map(|c| c as f32 / 256.0)) + d * (hit.t as f32 / 65536.0);
    Some(((hit.voxel, face), at))
}

/// The centre of `key`'s face and its outward normal.
fn vis_face(key: VisKey) -> (Vec3, Vec3) {
    use vxe_world::Face;
    let v = Vec3::from(key.0.0.map(|c| c as f32)) + Vec3::splat(0.5);
    let n = match key.1 {
        Face::NegX => Vec3::NEG_X,
        Face::PosX => Vec3::X,
        Face::NegY => Vec3::NEG_Y,
        Face::PosY => Vec3::Y,
        Face::NegZ => Vec3::NEG_Z,
        Face::PosZ => Vec3::Z,
    };
    (v + n * 0.5, n)
}

/// Whether `eye` has a clear line within `VIS_R` to `key`'s face: a ray toward the face's
/// centre, or toward `at` (a point on the face), enters that voxel through that face.
fn vis_clear(world: &vxe_world::World, eye: Vec3, key: VisKey, at: Vec3) -> bool {
    let (centre, n) = vis_face(key);
    if (eye - centre).dot(n) <= 0.0 {
        return false;
    }
    [centre, at].into_iter().any(|target| {
        let to = target - n * 0.01 - eye;
        let dist = to.length();
        dist <= VIS_R && vis_cast(world, eye, to / dist, dist + 1.0).is_some_and(|(k, _)| k == key)
    })
}

/// One frame's primary hits at `VIS_SIZE`: per pixel, the face key, its distance and hit point.
fn vis_frame(world: &vxe_world::World, camera: &Camera) -> Vec<Option<(VisKey, f32, Vec3)>> {
    let [w, h] = vis_size();
    let (fwd, right, up) = camera.basis();
    let tv = (camera.vfov * 0.5).tan();
    let ta = tv * w as f32 / h as f32;
    let threads = std::thread::available_parallelism().map_or(1, usize::from);
    let rows: Vec<usize> = (0..h).collect();
    let chunk = h.div_ceil(threads);
    std::thread::scope(|s| {
        let jobs: Vec<_> = rows
            .chunks(chunk)
            .map(|rows| {
                s.spawn(move || {
                    let mut out = Vec::with_capacity(rows.len() * w);
                    for &y in rows {
                        for x in 0..w {
                            let u = (x as f32 + 0.5) / w as f32 * 2.0 - 1.0;
                            let v = 1.0 - (y as f32 + 0.5) / h as f32 * 2.0;
                            let dir = (fwd + right * (u * ta) + up * (v * tv)).normalize();
                            out.push(
                                vis_cast(world, camera.position, dir, VIS_REACH)
                                    .map(|(k, at)| (k, (at - camera.position).length(), at)),
                            );
                        }
                    }
                    out
                })
            })
            .collect();
        jobs.into_iter()
            .flat_map(|j| j.join().expect("vis rows"))
            .collect()
    })
}

/// One visibility run: `cameras` in order after `VIS_HISTORY` frames at the first; prints
/// the entering pixels (a face not in view in the previous `VIS_HISTORY` frames), the share
/// within `VIS_R`, the share of those with a clear line from an eye of the previous
/// `VIS_HISTORY` frames, and their distance histogram. Returns (entering, within, clear).
fn vis_run(world: &vxe_world::World, name: &str, cameras: &[Camera]) -> [usize; 3] {
    use std::collections::HashMap;
    let mut last: HashMap<VisKey, usize> = HashMap::new();
    let mut eyes: Vec<Vec3> = Vec::new();
    let first = cameras[0];
    let history: Vec<Camera> = std::iter::repeat_n(first, VIS_HISTORY).collect();
    let (mut entering, mut within, mut clear) = (0usize, 0usize, 0usize);
    let mut hist = [0usize; VIS_BINS.len() + 1];
    let mut hist_clear = [0usize; VIS_BINS.len() + 1];
    let mut hit_pixels = 0usize;
    let mut peak = (0usize, 0usize);
    for (f, camera) in history.iter().chain(cameras.iter()).enumerate() {
        let hits = vis_frame(world, camera);
        let counted = f >= VIS_HISTORY;
        let mut frame_entering = Vec::new();
        for (key, dist, at) in hits.iter().flatten() {
            let fresh = last.get(key).is_none_or(|&seen| seen + VIS_HISTORY < f);
            if counted && fresh {
                frame_entering.push((*key, *dist, *at));
            }
        }
        if counted {
            // The distinct eyes of the previous `VIS_HISTORY` frames, newest first.
            let mut prev: Vec<Vec3> = Vec::new();
            for e in eyes[f - VIS_HISTORY..f].iter().rev() {
                if prev.iter().all(|p| p.distance(*e) > 0.25) {
                    prev.push(*e);
                }
            }
            let flags: Vec<bool> = std::thread::scope(|s| {
                let threads = std::thread::available_parallelism().map_or(1, usize::from);
                let chunk = frame_entering.len().div_ceil(threads).max(1);
                let jobs: Vec<_> = frame_entering
                    .chunks(chunk)
                    .map(|part| {
                        let prev = &prev;
                        s.spawn(move || {
                            part.iter()
                                .map(|&(key, dist, at)| {
                                    dist <= VIS_R
                                        && prev.iter().any(|&e| vis_clear(world, e, key, at))
                                })
                                .collect::<Vec<_>>()
                        })
                    })
                    .collect();
                jobs.into_iter()
                    .flat_map(|j| j.join().expect("vis clear"))
                    .collect()
            });
            for (&(_, dist, _), ok) in frame_entering.iter().zip(flags) {
                entering += 1;
                within += usize::from(dist <= VIS_R);
                clear += usize::from(ok);
                let bin = VIS_BINS
                    .iter()
                    .position(|&e| dist < e)
                    .unwrap_or(VIS_BINS.len());
                hist[bin] += 1;
                hist_clear[bin] += usize::from(ok);
            }
            hit_pixels += hits.iter().flatten().count();
            if frame_entering.len() > peak.1 {
                peak = (f - VIS_HISTORY, frame_entering.len());
            }
        }
        for (key, ..) in hits.iter().flatten() {
            last.insert(*key, f);
        }
        eyes.push(camera.position);
    }
    let share = |a: usize, b: usize| if b == 0 { 0.0 } else { a as f64 / b as f64 };
    println!(
        "reading: preload visibility {name} at {}x{}, {} frames: entering pixels {entering}, within R {within} ({:.3}), clear within R from an eye of the previous {VIS_HISTORY} frames {clear} ({:.3} of within R, {:.3} of entering); distance <8 / 8-16 / 16-32 / 32-64 / 64-128 / 128+: {:?}, of them clear {:?}; entering {:.4} of the hit pixels; peak frame {} with {}",
        vis_size()[0],
        vis_size()[1],
        cameras.len(),
        share(within, entering),
        share(clear, within),
        share(clear, entering),
        hist,
        hist_clear,
        share(entering, hit_pixels),
        peak.0,
        peak.1
    );
    [entering, within, clear]
}

/// Which faces entering view the preload's eye rays could have reached (Plan 0057 Phase 4
/// Part 2b): the rig's steady flight at both poses at 0.5 voxel a frame (`KRun::Path`, 361
/// frames), and `yaw 90 10 f` and `yaw 180 20 f` onto `VEIN_POSE`, each after 60 still frames
/// at its start, on the level at seed 1 through `vxe-world`'s raycast at `vis_size()`. No GPU.
#[test]
#[ignore = "reading only: run with --ignored"]
fn preload_visibility_readings() {
    let source = flythrough::cave::CaveSource::new(
        1,
        vxe_world::ChunkExtent::new([0, 0, 0], flythrough::cave::LEVEL_CHUNKS),
    );
    let world = vxe_world::World::generate(
        source.extent,
        &source,
        &flythrough::with_emissive_ore(&flythrough::cave_materials()),
    );
    for pose in [stillness::PATCH_POSE, VEIN_POSE] {
        let cameras: Vec<Camera> = (0..=360).map(|f| KRun::Path.camera(&pose, f)).collect();
        vis_run(&world, &format!("{} walk", pose.name), &cameras);
    }
    for (frames, degrees) in [(10usize, 90.0f32), (20, 180.0)] {
        let pose = VEIN_POSE;
        let cameras: Vec<Camera> = (0..=frames + 10)
            .map(|i| {
                let yaw = pose.yaw_deg - degrees + degrees * (i.min(frames) as f32) / frames as f32;
                camera_at(pose.position, yaw, pose.pitch_deg)
            })
            .collect();
        vis_run(&world, &format!("vein yaw {degrees} {frames}f"), &cameras);
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
                lamp_bounce: indirect,
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

/// The lamp channel's reach, read: under the lamp (on, no emissive light, the channel over the
/// whole 32-probe volume with no reach cut, its freeze 64), still for 600 frames at the
/// `cache_box` camera and at the level's spawn facing +x, the direct lamp irradiance `D` at the
/// wall the lamp lights at arm's length (the light pass's lamp part over the central 8 x 8
/// lighting pixels), then by 16-voxel band of the lamp-to-probe distance the brightest interior texel of
/// any traced probe outside rock, and by band of the lamp-to-hit distance the brightest gather
/// over the hit pixels, each over `D`. `probes::LAMP_REACH` is the band edge past which every
/// probe band reads under 0.01, rounded up to whole spacings. `VXE_TEST_ADAPTER` picks the
/// adapter.
#[test]
#[ignore = "reading only: run with --ignored"]
fn lamp_reach_readings() {
    use vxe_render::readback;
    let Some(level) = reading_level() else {
        return;
    };
    let adapter = level.adapter.clone();
    let (device, queue) = (&level.device, &level.queue);
    let (box_world, box_camera) = cache_box();
    let mut box_gpu = vxe_render::GpuWorld::with_config(
        device,
        vxe_render::MirrorConfig {
            budget_bytes: 64 << 20,
            ..vxe_render::MirrorConfig::new(vxe_render::Window::Fixed(box_world.extent()))
        },
    );
    for _ in 0..2 {
        box_gpu
            .sync_now(&box_world, device, queue)
            .expect("the mirror fits");
    }
    let spawn = spawn_pose(VEIN_POSE.exposure).camera();
    let light = LightParams::headlamp();
    const BAND: f32 = 16.0;
    const BANDS: usize = 16;
    for (name, gpu, camera) in [
        ("cache_box", &box_gpu, box_camera),
        ("spawn", &level.gpu, spawn),
    ] {
        let size = [160u32, 90];
        let mut frame = Frame::new(device, size[0], size[1]);
        frame.set_resolution(device, vxe_render::LightingResolution::Full);
        frame.set_lighting(Lighting {
            lamp: true,
            candidates: 0,
            lamp_bounce: true,
            lamp_reach: 0.0,
            exposure: Exposure::Manual(1.0),
            ..Lighting::default()
        });
        frame.set_dt(1.0 / 60.0);
        for _ in 0..600 {
            frame.step(device, queue, gpu, &camera, &light);
        }
        let params = frame.params(gpu, &camera, &light);
        let lamp = Vec3::from(params.light_pos);
        let irr = readback::read_float(device, queue, frame.irradiance());
        let (cx, cy) = (size[0] / 2, size[1] / 2);
        let mut d_sum = 0.0f64;
        for y in cy - 4..cy + 4 {
            for x in cx - 4..cx + 4 {
                // The light pass's lamp part is its alpha (the emissive part is its rgb).
                d_sum += f64::from(irr[(y * size[0] + x) as usize][3]);
            }
        }
        let d = d_sum / 64.0;
        let v = frame.probes();
        let origin = v.origin_for(camera.position.to_array());
        let mut probe_band = [0.0f64; BANDS];
        let mut probe_count = [0u32; BANDS];
        for s in 0..v.count() {
            let w = v.slot_probe(origin, s);
            let p = v.read_probe(device, queue, s);
            if p.inside() || p.age() == 0 {
                continue;
            }
            let r = (Vec3::from(v.probe_position(w)) - lamp).length();
            let b = ((r / BAND) as usize).min(BANDS - 1);
            let hi = p
                .interior_irradiance()
                .iter()
                .map(|t| f64::from(t[0]))
                .fold(0.0, f64::max);
            probe_band[b] = probe_band[b].max(hi);
            probe_count[b] += 1;
        }
        let pos = readback::read_float(device, queue, frame.hit_positions());
        let info = readback::read_rgba16u(device, queue, frame.hit_records());
        let gather = readback::read_float(device, queue, frame.indirect_irradiance());
        let mut hit_band = [0.0f64; BANDS];
        for ((p, i), g) in pos.iter().zip(&info).zip(&gather) {
            if i[3] & 1 == 0 {
                continue;
            }
            let r = (Vec3::new(p[0], p[1], p[2]) - lamp).length();
            let b = ((r / BAND) as usize).min(BANDS - 1);
            hit_band[b] = hit_band[b].max(f64::from(g[0]));
        }
        let under = (0..BANDS)
            .find(|&b| probe_band[b..].iter().all(|&e| e < 0.01 * d))
            .map(|b| b as f32 * BAND);
        let fmt = |v: &[f64]| {
            v.iter()
                .map(|e| format!("{:.4}", e / d))
                .collect::<Vec<_>>()
                .join(" ")
        };
        println!(
            "reading[{adapter}]: lamp reach {name}: D {d:.4} at the central pixels; probe bands of {BAND} voxels (brightest texel / D) {}; probes per band {probe_count:?}; gather bands (brightest hit / D) {}; every probe band under 0.01 from {under:?} voxels",
            fmt(&probe_band),
            fmt(&hit_band)
        );
    }
}

/// The rig's setting for the lamp channel's readings: the lamp and its bounce on, the pose's
/// pinned exposure, scale 1 with no upscaler.
fn channel_setting() -> Setting {
    Setting {
        lamp: true,
        indirect: true,
        auto: false,
        upscaler: false,
        bridge: false,
        steady: Steady::default(),
    }
}

/// The lamp step at each `h_move` (`VXE_PROBE_MOVES`, default 0.5, 0.8 and 0.9): at each pose
/// (`VXE_SETTLE_POSES`) with the lamp channel on, 600 still frames (every probe in reach
/// frozen), then the camera (and the lamp with it) stepped 2 voxels to its right in one frame
/// and held, for 600 frames (a step of the lamp's offset instead is a change of its parameters,
/// which restarts the channel): the gather's mean over the hit pixels each frame, the frames until it has moved 90 %
/// of the way from its value before the step to its value at the end, and the largest
/// overshoot past the end value as a share of the step.
#[test]
#[ignore = "reading only: run with --ignored"]
fn lamp_step_readings() {
    let Some(level) = reading_level() else {
        return;
    };
    let adapter = level.adapter.clone();
    let moves: Vec<f32> = list("VXE_PROBE_MOVES", &["0.5", "0.8", "0.9"])
        .iter()
        .map(|s| s.parse().expect("an h_move"))
        .collect();
    let mean_gather = |frame: &Frame| -> f64 {
        let (d, q) = (&level.device, &level.queue);
        let g = vxe_render::readback::read_float(d, q, frame.indirect_irradiance());
        let info = vxe_render::readback::read_rgba16u(d, q, frame.hit_records());
        let (mut sum, mut n) = (0.0f64, 0u32);
        for (g, i) in g.iter().zip(&info) {
            if i[3] & 1 != 0 {
                sum += f64::from(g[0] + g[1] + g[2]) / 3.0;
                n += 1;
            }
        }
        sum / f64::from(n.max(1))
    };
    for pose in poses() {
        for &h in &moves {
            let mut frame = make_frame(&level, &pose, channel_setting());
            frame.set_lighting(Lighting {
                probe_move_blend: h,
                ..frame.lighting()
            });
            let mut cam = pose.camera();
            let light = LightParams::headlamp();
            for _ in 0..600 {
                frame.step(&level.device, &level.queue, &level.gpu, &cam, &light);
            }
            let before = mean_gather(&frame);
            let freeze = frame.lighting().probe_freeze;
            let frozen =
                |frame: &Frame, cam: &Camera| freeze_census(&level, frame, cam, freeze).all[2];
            let mut census = vec![(0, frozen(&frame, &cam))];
            let (_, right, _) = cam.basis();
            cam.position += right * 2.0;
            let mut series = Vec::new();
            for f in 1..=600 {
                frame.step(&level.device, &level.queue, &level.gpu, &cam, &light);
                series.push(mean_gather(&frame));
                if [1, 8, 16, 64, 600].contains(&f) {
                    census.push((f, frozen(&frame, &cam)));
                }
            }
            let end = *series.last().expect("frames ran");
            let step = end - before;
            let to90 = series
                .iter()
                .position(|&v| (v - before) / step >= 0.9)
                .map(|i| i + 1);
            let over = series
                .iter()
                .map(|&v| (v - end) / step)
                .fold(0.0f64, f64::max);
            println!(
                "reading[{adapter}]: lamp step {} h_move {h}: gather mean {before:.5} -> {end:.5} ({:+.1} %); frames to 90 % {to90:?}; overshoot {:.3} of the step; frozen probes at frames {census:?}; frames 1..=24 {:?}",
                pose.name,
                step / before * 100.0,
                over,
                series[..24]
                    .iter()
                    .map(|v| format!("{v:.5}"))
                    .collect::<Vec<_>>()
            );
        }
    }
}

/// The still image through the channel: at each pose with the lamp on and still and its bounce
/// on and off (scale 1, pinned exposure), after `VXE_STILL_SETTLE` frames (default 1200: the freeze's 64
/// visits at 8 slices with room), the output's pixels that differ from the last frame's over
/// the next 64 frames, per frame, and the probe rays dispatched then.
#[test]
#[ignore = "reading only: run with --ignored"]
fn still_lamp_composite_readings() {
    let Some(level) = reading_level() else {
        return;
    };
    let adapter = level.adapter.clone();
    let settle: usize = std::env::var("VXE_STILL_SETTLE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1200);
    for (pose, bounce) in poses()
        .into_iter()
        .flat_map(|p| [true, false].map(move |b| (p, b)))
    {
        let setting = Setting {
            indirect: bounce,
            ..channel_setting()
        };
        let mut frame = make_frame(&level, &pose, setting);
        let cam = pose.camera();
        let light = LightParams::headlamp();
        for _ in 0..settle {
            frame.step(&level.device, &level.queue, &level.gpu, &cam, &light);
        }
        let read = |frame: &Frame| {
            vxe_render::readback::read_rgba8(&level.device, &level.queue, frame.target())
        };
        let mut last = read(&frame);
        let mut moved = Vec::new();
        let mut rays = 0u64;
        // The moved pixels by class (`VXE_STILL_CLASSES`): no face key (a miss, the field, a
        // hit behind translucent layers: the fallback), a face with no cell (a missed key),
        // a cell under the cap (young), a capped cell; each split interior / edge (a 4-neighbour
        // pixel's face key differs from its own). The largest step of a moved pixel's channels.
        let classes = std::env::var("VXE_STILL_CLASSES").is_ok();
        let mut tally = [[0usize; 2]; 4];
        let mut step_max = 0u8;
        let cap = frame.lighting().table_cap;
        let [w, h] = frame.internal_size();
        assert_eq!(
            frame.internal_size(),
            frame.output_size(),
            "scale 1: a hit pixel is an output pixel"
        );
        for _ in 0..64 {
            frame.step(&level.device, &level.queue, &level.gpu, &cam, &light);
            rays += frame.traced_probe_rays();
            let now = read(&frame);
            let changed: Vec<usize> = now
                .chunks(4)
                .zip(last.chunks(4))
                .enumerate()
                .filter(|(_, (a, b))| a[..3] != b[..3])
                .map(|(i, (a, b))| {
                    for c in 0..3 {
                        step_max = step_max.max(a[c].abs_diff(b[c]));
                    }
                    i
                })
                .collect();
            moved.push(changed.len());
            if classes && !changed.is_empty() {
                let (d, q) = (&level.device, &level.queue);
                let pos = vxe_render::readback::read_float(d, q, frame.hit_positions());
                let info = vxe_render::readback::read_rgba16u(d, q, frame.hit_records());
                let keys: Vec<Option<FaceKey>> = pos
                    .iter()
                    .zip(&info)
                    .map(|(p, i)| face_key(cam.position, *p, *i))
                    .collect();
                let words = cache_words(&level, &frame);
                for &i in &changed {
                    let (x, y) = ((i as u32 % w) as i64, (i as u32 / w) as i64);
                    let k = keys[i];
                    let edge = [(-1, 0), (1, 0), (0, -1), (0, 1)].iter().any(|(dx, dy)| {
                        let (nx, ny) = (x + dx, y + dy);
                        nx >= 0
                            && ny >= 0
                            && nx < i64::from(w)
                            && ny < i64::from(h)
                            && keys[(ny * i64::from(w) + nx) as usize] != k
                    });
                    let class = match k {
                        None => 0,
                        Some(k) => match cache_count(&words, k) {
                            0 => 1,
                            n if n < cap => 2,
                            _ => 3,
                        },
                    };
                    tally[class][usize::from(edge)] += 1;
                }
            }
            last = now;
        }
        println!(
            "reading[{adapter}]: still lamp {} bounce {bounce}: after {settle} frames, pixels moved per frame over 64 {moved:?} (of {}); probe rays dispatched over them {rays} (a frozen probe returns at once); largest channel step {step_max}",
            pose.name,
            last.len() / 4
        );
        if classes {
            println!(
                "reading[{adapter}]: still lamp {} bounce {bounce}: moved pixels by class [interior, edge]: no face key {:?}, no cell {:?}, cell under the cap {:?}, capped cell {:?}",
                pose.name, tally[0], tally[1], tally[2], tally[3]
            );
        }
    }
}

// ---- The lamp channel's blend: the asserted tests ----

/// `freeze`'s frame for the lavapipe tests: 64 x 36, the lamp channel on, 8 slices, at the vein.
fn steady_frame(level: &stillness::VeinLevel, lighting: Lighting) -> Frame {
    let mut frame = Frame::new(&level.device, 64, 36);
    frame.set_lighting(Lighting {
        lamp_bounce: true,
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
    let v = frame.probes();
    let origin = v.origin_for(camera.position.to_array());
    let mut all = Vec::new();
    for s in 0..v.count() {
        let p = Vec3::from(v.probe_position(v.slot_probe(origin, s)));
        if keep(p) {
            all.push(((p - camera.position).length(), s));
        }
    }
    all.sort_by(|a, b| a.0.total_cmp(&b.0));
    all.iter()
        .map(|&(_, s)| s)
        .find(|&s| {
            let r = v.read_probe(&level.device, &level.queue, s);
            !r.inside()
                && r.age() > 0
                && (!lit
                    || r.interior_irradiance()
                        .iter()
                        .any(|t| t[0] + t[1] + t[2] > 0.0))
        })
        .expect("a traced probe outside rock")
}

/// A still lamp freezes the channel: at `probe_freeze` 8 and 8 slices with the lamp on and still
/// at the vein, after the channel's start and 8 visits of every probe in reach (72 frames)
/// every probe's texels and state are bit-identical across the next 64 encoded frames, and a
/// probe in reach with light in it reached the freeze.
#[test]
fn a_still_lamp_freezes_the_channel() {
    let Some(level) = vein_level() else {
        return;
    };
    let light = LightParams::headlamp();
    let camera = VEIN_POSE.camera();
    let mut frame = steady_frame(
        &level,
        Lighting {
            probe_freeze: 8,
            ..Lighting::default()
        },
    );
    let step = |frame: &mut Frame, n: usize| {
        for _ in 0..n {
            frame.step(&level.device, &level.queue, &level.gpu, &camera, &light);
        }
    };
    step(&mut frame, 72);
    let frozen = probe_words(&level, &frame);
    let slot = traced_probe(&level, &frame, &camera, true, |_| true);
    let age = frame
        .probes()
        .read_probe(&level.device, &level.queue, slot)
        .age();
    step(&mut frame, 64);
    println!("lamp still: the nearest lit probe (slot {slot}) at age {age} after 72 frames");
    assert!(age >= 8, "age {age}");
    assert!(
        probe_words(&level, &frame) == frozen,
        "the atlases and states moved over 64 frames after the freeze"
    );
}

/// A lamp step moves the probes in reach only: with the volume pinned so that the eye sits in
/// its lowest cell (its far corner beyond `lamp_reach`), after the channel froze at
/// `probe_freeze` 8, a lamp moved each frame for 8 frames changes a near probe's texels within
/// them (blending at `h_move`, its age restarting at 1) and keeps a probe beyond the reach
/// bit-identical (never listed), and the near probe freezes again once the lamp holds still.
#[test]
fn a_lamp_step_moves_the_probes_in_reach_only() {
    let Some(level) = vein_level() else {
        return;
    };
    let light = LightParams::headlamp();
    let camera = VEIN_POSE.camera();
    let mut frame = steady_frame(
        &level,
        Lighting {
            probe_freeze: 8,
            ..Lighting::default()
        },
    );
    let reach = frame.lighting().lamp_reach;
    // The eye in the volume's lowest cell on every axis.
    let mut origin = frame.probes().origin_for(VEIN_POSE.position);
    let axis = frame.probes().axis() as i32;
    for o in &mut origin {
        *o += axis / 2 - 1;
    }
    frame.probes_mut().set_origin(Some(origin));
    let step = |frame: &mut Frame, cam: &Camera, n: usize| {
        for _ in 0..n {
            frame.step(&level.device, &level.queue, &level.gpu, cam, &light);
        }
    };
    step(&mut frame, &camera, 72);
    let near = traced_probe(&level, &frame, &camera, true, |_| true);
    let v = frame.probes();
    let far = (0..v.count())
        .find(|&s| {
            let p = Vec3::from(v.probe_position(v.slot_probe(origin, s)));
            (p - camera.position).length() > reach + 8.0
        })
        .expect("a probe beyond the reach");
    let read = |frame: &Frame, s: u32| frame.probes().read_probe(&level.device, &level.queue, s);
    let (near0, far0) = (read(&frame, near), read(&frame, far));
    assert!(near0.age() >= 8, "the near probe froze: {}", near0.age());
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
    println!(
        "near probe slot {near}: age {} -> {}; far probe slot {far}: age {}",
        near0.age(),
        near1.age(),
        far0.age()
    );
    assert_ne!(near1.irradiance, near0.irradiance, "the near probe moved");
    assert_eq!(near1.age(), 1, "the moving lamp restarts its age");
    assert_eq!(read(&frame, far), far0, "the far probe stayed as it was");
    // The lamp holds still: 8 more visits, then frozen again.
    let held = Camera {
        yaw: camera.yaw + 0.016,
        ..camera
    };
    step(&mut frame, &held, 72);
    let settled = read(&frame, near);
    assert!(settled.age() >= 8, "it refroze at age {}", settled.age());
    step(&mut frame, &held, 16);
    assert_eq!(read(&frame, near), settled, "and holds");
}

/// A probe inside rock keeps its texels and its state bit-identical over 64 frames.
#[test]
fn an_inside_probe_is_not_traced() {
    let Some(level) = vein_level() else {
        return;
    };
    let light = LightParams::headlamp();
    let camera = VEIN_POSE.camera();
    let mut frame = steady_frame(&level, Lighting::default());
    for _ in 0..8 {
        frame.step(&level.device, &level.queue, &level.gpu, &camera, &light);
    }
    let (traced, inside) = probe_census(&level, &frame);
    assert!(
        inside > 0 && traced > inside,
        "{traced} traced, {inside} inside"
    );
    let count = frame.probes().count();
    let slots: Vec<u32> = (0..count)
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
        lamp_bounce: false,
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
    /// Turns the preload off, for a test of the light pass's own cells.
    fn preload_off(&mut self) {
        let l = self.frame.lighting();
        self.frame.set_lighting(Lighting {
            preload_radius: 0,
            ..l
        });
    }

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
/// voxel's other face has its own. The light pass's cells alone: the preload is off.
#[test]
fn a_key_is_its_voxel_and_face() {
    let Some(mut rig) = cache_box_rig(vxe_render::marcher::DEFAULT_TABLE_CAP) else {
        return;
    };
    rig.preload_off();
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
/// away, and the cleared cells refill to 16 samples within 16 frames; on the frame after the
/// box, the cell of the voxel's most-seen face counts its pixels plus the reopened fill's F.
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
    // Its pixels' samples and the reopened fill's F.
    let n = keys[&(v, f)] + vxe_render::marcher::DEFAULT_FILL_SAMPLES;
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

/// The box with a glass layer over the rock block's top (`cache_box_in` with the castle's
/// rows), at 64 x 36, scale 1, lamp off.
fn slab_rig() -> Option<CacheBox> {
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
    cache_box_rig_on(world, camera, 4096, [64, 36], None)
}

/// A still fallback pixel holds still: on the slab box (`slab_rig`), after 64 frames every
/// translucent hit's accumulated emissive part is bit-identical over the next 64 (its running
/// mean held at 64 samples); a one-voxel edit box over the slab at the median translucent hit
/// restarts the pixels whose hit lies in its dilated span (each one's value changes within 2
/// frames). About 130 frames.
#[test]
fn a_still_fallback_pixel_holds_still() {
    let Some(mut rig) = slab_rig() else {
        return;
    };
    rig.step(64);
    let read = |rig: &CacheBox| {
        let (d, q) = (&rig.level.device, &rig.level.queue);
        (
            vxe_render::readback::read_float(d, q, rig.frame.accumulated()),
            vxe_render::readback::read_float(d, q, rig.frame.hit_positions()),
            vxe_render::readback::read_rgba16u(d, q, rig.frame.hit_records()),
        )
    };
    let (held, pos, info) = read(&rig);
    let glass: Vec<usize> = (0..info.len())
        .filter(|&i| info[i][3] & 1 == 1 && info[i][3] & 4 != 0 && held[i][0] > 0.0)
        .collect();
    assert!(glass.len() > 20, "{} lit translucent hits", glass.len());
    let rgb = |a: &[[f32; 4]], i: usize| [a[i][0], a[i][1], a[i][2]].map(f32::to_bits);
    for f in 1..=64 {
        rig.step(1);
        let (acc, _, _) = read(&rig);
        let moved: Vec<usize> = glass
            .iter()
            .copied()
            .filter(|&i| rgb(&acc, i) != rgb(&held, i))
            .collect();
        assert!(
            moved.is_empty(),
            "frame {f} after 64: {} of {} translucent hits moved, first {:?}",
            moved.len(),
            glass.len(),
            moved.first().map(|&i| (acc[i], held[i]))
        );
    }
    // The median translucent hit's voxel, under the glass.
    let mut by_x: Vec<usize> = glass.clone();
    by_x.sort_by(|&a, &b| {
        pos[a][0]
            .total_cmp(&pos[b][0])
            .then(pos[a][1].total_cmp(&pos[b][1]))
    });
    let m = pos[by_x[by_x.len() / 2]];
    let v = [m[0].floor() as i32, m[1].floor() as i32, 64];
    let inside: Vec<usize> = glass
        .iter()
        .copied()
        .filter(|&i| {
            (0..3).all(|k| pos[i][k] >= (v[k] - 1) as f32 && pos[i][k] <= (v[k] + 2) as f32)
        })
        .collect();
    assert!(!inside.is_empty(), "no translucent hit in the box at {v:?}");
    rig.frame.invalidate(v, v);
    let mut changed = vec![false; inside.len()];
    for _ in 0..2 {
        rig.step(1);
        let (acc, _, _) = read(&rig);
        for (c, &i) in changed.iter_mut().zip(&inside) {
            *c |= rgb(&acc, i) != rgb(&held, i);
        }
    }
    println!(
        "reading: slab, {} translucent hits held over 64 frames; the edit box at {v:?} restarted {} of {} inside it",
        glass.len(),
        changed.iter().filter(|&&c| c).count(),
        inside.len()
    );
    assert!(
        changed.iter().all(|&c| c),
        "a pixel in the edit box kept its value"
    );
}

/// A hit with no face (behind a translucent slab) takes the fallback: on the box with a glass
/// layer over the rock block's top, a translucent hit's emissive part after 8 frames equals the
/// closed-form running mean of its own samples, `e += (s - e) / (age + 1)`, within 1 %, and
/// its key holds no cell.
#[test]
fn a_hit_with_no_face_takes_the_fallback() {
    let Some(mut rig) = slab_rig() else {
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
            let a = 1.0f64 / (f as f64 + 1.0);
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
/// plus them. The light pass's cells alone: the preload is off.
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
    rig.preload_off();
    // The walk visits a table of 2^20 cells in one frame.
    rig.frame.set_lighting(Lighting {
        table_cells: vxe_render::marcher::TABLE_WALK_SLICE,
        ..rig.frame.lighting()
    });
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

/// The voxel and face of a cell's key words (`cache_key_voxel` in `march.wgsl`).
fn cell_key(cell: &[u32]) -> FaceKey {
    let sign = |c: u32| ((c << 12) as i32) >> 12;
    let (x, y) = (cell[0] & 0xF_FFFF, cell[0] >> 20 | (cell[1] & 0xFF) << 12);
    (
        [sign(x), sign(y), sign((cell[1] >> 8) & 0xF_FFFF)],
        ((cell[1] >> 28) & 7) as u8,
    )
}

/// The keys of every occupied cell in the table's words, duplicates kept.
fn cell_keys(words: &[u32]) -> Vec<FaceKey> {
    words
        .chunks_exact(vxe_render::marcher::CACHE_CELL_WORDS as usize)
        .filter(|c| c[1] != 0)
        .map(cell_key)
        .collect()
}

/// The age-out's eye, its radius and its table (test sizes).
const AGE_EYE: [f32; 3] = [64.5, 64.5, 64.5];
const AGE_R: i32 = 16;
const AGE_CELLS: u32 = 16_384;

/// Whether a voxel's centre lies within the age-out's radius of its eye.
fn age_near(v: [i32; 3]) -> bool {
    let d: f32 = (0..3)
        .map(|a| (v[a] as f32 + 0.5 - AGE_EYE[a]).powi(2))
        .sum();
    d <= (AGE_R * AGE_R) as f32
}

/// The age-out's box under its rig, at the eye looking at the sky: walls at x 105 and x 22, a
/// floor at z 22, a 6-voxel rock block within `AGE_R` beside the views and an ore plate above
/// them; 192 x 108, full-resolution lighting, cap 64, `AGE_CELLS` cells, age 8 down to 4.
fn age_rig(age_out: bool) -> Option<CacheBox> {
    use vxe_world::*;
    let table = flythrough::with_emissive_ore(&flythrough::cave_materials());
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [2, 2, 2]),
        &UniformSource(AIR),
        &table,
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
    fill([105, 0, 0], [108, 128, 128], flythrough::cave::ROCK);
    fill([20, 0, 0], [23, 128, 128], flythrough::cave::ROCK);
    fill([0, 0, 20], [128, 128, 23], flythrough::cave::ROCK);
    fill([64, 73, 58], [70, 79, 64], flythrough::cave::ROCK);
    fill([40, 40, 118], [50, 50, 120], flythrough::cave::ORE);
    let mut rig = cache_box_rig_on(world, age_view(89.0, 0.0), 64, [192, 108], None)?;
    rig.frame.set_lighting(Lighting {
        table_cells: AGE_CELLS,
        table_age_out: age_out,
        table_age: 8,
        table_age_min: 4,
        preload_radius: AGE_R as u32,
        preload_claim_radius: AGE_R as u32,
        ..rig.frame.lighting()
    });
    Some(rig)
}

/// The age-out's eye at `pitch` and `yaw`, in degrees.
fn age_view(pitch: f32, yaw: f32) -> Camera {
    Camera {
        position: AGE_EYE.into(),
        yaw: yaw.to_radians(),
        pitch: pitch.to_radians(),
        vfov: 70f32.to_radians(),
    }
}

/// The three views the age-out's test holds in turn: x 105's wall, x 22's and the floor.
const AGE_VIEWS: [(f32, f32); 3] = [(0.0, 0.0), (0.0, 180.0), (-89.0, 0.0)];

/// The age-out (ADR-0043's table): three views of about 6000 faces each beyond `AGE_R`, held 32
/// frames in turn in `AGE_CELLS` cells, after 16 frames of looking at the sky in which the
/// preload claims the block's faces within `AGE_R`: after the third view the table holds only
/// that view's cells, those within `AGE_R` and those a kernel of its cells reads (two faces
/// along the plane, one across it); every cell within `AGE_R` keeps its count; no key has two
/// cells; and with the age-out off the third view does not fit and counts misses. The second
/// view's cells are the re-arm's: one frame old at the third view's first pass, they go in the
/// pass `age` frames after it.
#[test]
fn an_unseen_far_cell_ages_out_and_a_near_one_stays() {
    let Some(mut rig) = age_rig(true) else {
        return;
    };
    rig.step(16);
    let near: std::collections::HashMap<FaceKey, u32> = {
        let words = cache_words(&rig.level, &rig.frame);
        cell_keys(&words)
            .into_iter()
            .filter(|k| age_near(k.0))
            .map(|k| (k, cache_count(&words, k)))
            .collect()
    };
    let mut faces = Vec::new();
    let mut view3 = std::collections::HashSet::new();
    for (pitch, yaw) in AGE_VIEWS {
        rig.camera = age_view(pitch, yaw);
        rig.step(32);
        let keys = rig.keys();
        faces.push(keys.keys().filter(|k| !age_near(k.0)).count());
        view3 = keys.into_keys().collect();
    }
    let words = cache_words(&rig.level, &rig.frame);
    let cells = cell_keys(&words);
    let unique: std::collections::HashSet<FaceKey> = cells.iter().copied().collect();
    let (d, q) = (&rig.level.device, &rig.level.queue);
    let age = rig.frame.read_age_stats(d, q);
    let kernel = |k: &FaceKey| {
        let axis = usize::from(k.1 / 2);
        view3.iter().any(|o| {
            o.1 == k.1 && (0..3).all(|a| (k.0[a] - o.0[a]).abs() <= if a == axis { 1 } else { 2 })
        })
    };
    let beyond: Vec<&FaceKey> = unique
        .iter()
        .filter(|k| !view3.contains(k) && !age_near(k.0) && !kernel(k))
        .collect();
    println!(
        "age-out: faces beyond R by view {faces:?}, within R {} (seen by preload {}), view 3 {}, occupied {}, unexplained {}, {age:?}",
        unique.iter().filter(|k| age_near(k.0)).count(),
        near.len(),
        view3.len(),
        cells.len(),
        beyond.len(),
    );
    assert!(
        faces.iter().all(|&n| (4000..8000).contains(&n)),
        "about 6000 faces a view: {faces:?}"
    );
    assert!(near.len() >= 100, "{} cells within R", near.len());
    assert_eq!(cells.len(), unique.len(), "no key has two cells");
    assert_eq!(
        beyond.len(),
        0,
        "cells left by the first two views: {beyond:?}"
    );
    for (k, n) in &near {
        assert!(*n > 0, "the preload's cell {k:?} holds samples");
        assert!(
            cache_count(&words, *k) >= *n,
            "the cell {k:?} within R keeps its count"
        );
    }
    assert!(age.evicted > 0, "{age:?}");

    let Some(mut rig) = age_rig(false) else {
        return;
    };
    rig.step(16);
    for (pitch, yaw) in AGE_VIEWS {
        rig.camera = age_view(pitch, yaw);
        rig.step(32);
    }
    let (d, q) = (&rig.level.device, &rig.level.queue);
    let stats = rig.frame.read_table_stats(d, q);
    println!("age-out off: {stats:?}");
    assert!(stats.misses > 0, "the third view does not fit: {stats:?}");
}

/// A bench press in budget clears only its boxes' cells: 63 boxes of 21^3 voxels, whose dilated
/// voxels' faces outnumber the table's cells, out of the view, leave the view's cells their
/// counts and open no whole-table clear, while a cell written inside the first box is cleared
/// with its key kept.
#[test]
fn a_bench_press_clears_its_boxes_not_the_table() {
    use vxe_render::marcher::{CACHE_CELL_WORDS, CACHE_COUNT_WORD, cache_key, cache_slot};
    let Some(mut rig) = cache_box_rig(64) else {
        return;
    };
    rig.preload_off();
    rig.step(80);
    let keys: Vec<FaceKey> = rig.keys().into_keys().collect();
    let before = cache_words(&rig.level, &rig.frame);
    let clears = rig.frame.table_clears_and_sweeps().0;
    let v = [10_010, 10_010, 10_010];
    let word = u64::from(cache_slot(v, 0, rig.frame.table_cells())) * CACHE_CELL_WORDS * 4;
    let k = cache_key(v, 0);
    let cell: Vec<u8> = [k[0], k[1], 64]
        .iter()
        .flat_map(|w| w.to_le_bytes())
        .collect();
    rig.level
        .queue
        .write_buffer(rig.frame.emissive_table(), word, &cell);
    for i in 0..63 {
        let min = [10_000 + 30 * i, 10_000, 10_000];
        rig.frame.invalidate(min, min.map(|c| c + 20));
    }
    rig.step(1);
    let words = cache_words(&rig.level, &rig.frame);
    assert_eq!(
        rig.frame.table_clears_and_sweeps().0,
        clears,
        "no whole-table clear"
    );
    for k in &keys {
        assert_eq!(cache_count(&before, *k), 64, "key {k:?} is capped before");
        assert_eq!(cache_count(&words, *k), 64, "key {k:?} keeps its count");
    }
    let w = cache_cell(&words, (v, 0)).expect("the written cell keeps its key");
    assert_eq!(
        words[w + CACHE_COUNT_WORD],
        0,
        "the written cell is cleared"
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
    let cells = rig.frame.table_cells();
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
    Some((guard_rig_on(world, camera, cap)?, carve, to_rock))
}

/// `world` from `camera` under the guard box's frame (`guard_rig`).
fn guard_rig_on(world: vxe_world::World, camera: Camera, cap: u32) -> Option<CacheBox> {
    let mut rig = cache_box_rig_on(world, camera, cap, [192, 108], None)?;
    rig.frame.set_lighting(Lighting {
        lamp: false,
        lamp_bounce: false,
        exposure: Exposure::Manual(1.0),
        table_cap: cap,
        table_cells: GUARD_CELLS,
        ..Lighting::default()
    });
    Some(rig)
}

/// The guard box with the plug in and a hole through it of one 2^3 block of the coarse shadow
/// walk (`ShadowWalk::Coarse`, the default), y 30..32 and z 38..40, near the plug's top edge:
/// from the far wall's rows above z 31.5 in view the hole's shadow on the emitter wall
/// (x = 4) reaches its top row of faces, two or three of its 400, so a pixel's emissive sample
/// finds light about once in 150 to 200 and a cell of `GUARD_CAP` samples caps dark with a
/// share near `(1 - 1/170)^256`, about 0.2; the rows below see no emitter and cap dark.
fn gap_rig(cap: u32) -> Option<CacheBox> {
    use vxe_world::*;
    let (world, _, _, camera) = guard_box(true);
    world
        .apply(&EditOp::box_(
            Fixed3([14, 30, 38].map(|v| v * FIXED_ONE)),
            Fixed3([15, 32, 40].map(|v| v * FIXED_ONE)),
            Mode::Carve,
        ))
        .expect("the box is resident");
    guard_rig_on(world, camera, cap)
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

/// The dig box: two rock chunks along x (128 x 64 x 64) with a passage carved at x 2..108,
/// y and z 20..44, its ceiling (z = 44) set with four 4 x 4 ore patches at x 16, 40, 64 and 88
/// (y 30..34). Returns the world.
fn dig_box() -> vxe_world::World {
    use flythrough::cave::{ORE, ROCK};
    use vxe_world::*;
    let table = flythrough::with_emissive_ore(&flythrough::cave_materials());
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [2, 1, 1]),
        &UniformSource(ROCK),
        &table,
    );
    let op = |min: [i64; 3], max: [i64; 3], mode| {
        world
            .apply(&EditOp::box_(
                Fixed3(min.map(|v| v * FIXED_ONE)),
                Fixed3(max.map(|v| v * FIXED_ONE)),
                mode,
            ))
            .expect("the box is resident");
    };
    op([2, 20, 20], [108, 44, 44], Mode::Carve);
    for x in [16, 40, 64, 88] {
        op(
            [x, 30, 44],
            [x + 4, 34, 45],
            Mode::Replace {
                from: ROCK,
                to: ORE,
            },
        );
    }
    world
}

/// The dig box's two digs of radius 10: at arm's length, the camera at (60, 25, 32) facing the
/// passage's -y wall with the dig centred on it 5 voxels away; and at range, the camera at
/// (8, 32, 32) facing +x down the passage with the dig centred on its end wall 100 voxels away.
fn dig_case(far: bool) -> (Camera, [i64; 3]) {
    if far {
        (
            Camera {
                position: [8.0, 32.0, 32.0].into(),
                yaw: 0.0,
                pitch: 0.0,
                vfov: 50f32.to_radians(),
            },
            [108, 32, 32],
        )
    } else {
        (
            Camera {
                position: [60.0, 25.0, 32.0].into(),
                yaw: (-90f32).to_radians(),
                pitch: 0.0,
                vfov: 50f32.to_radians(),
            },
            [60, 20, 32],
        )
    }
}

/// Frames a dug hole's run reads after the dig; its pixels' reference is their mean over the
/// last `DIG_REF` of them, and `S90` is read up to `DIG_FRAMES - DIG_REF`.
const DIG_FRAMES: usize = 300;
const DIG_REF: usize = 60;

/// The dug hole's `S90` on the dig box (`dig_box`, `dig_case`) at 192 x 108, scale 1, lamp off,
/// manual exposure 1, the fill at `fill_samples` (and the default C_FILL): 16 frames, the
/// carve (synced and its box invalidated), then `DIG_FRAMES` frames, each reading the
/// accumulated emissive part. The hole's pixels are those of the first frame after the dig
/// whose face key's voxel lies in the carve's box dilated by one and whose reference luminance
/// is above 1e-6; a pixel is settled at a frame when its luminance lies within a factor 1.1 of
/// its reference. `S90` is the frame after the last at which under 90 % of the hole's pixels
/// are settled (0 when none), `None` when that frame is past `DIG_FRAMES - DIG_REF`. Returns
/// `S90` and the hole's pixel count.
fn dug_hole_s90(far: bool, fill_samples: u32) -> Option<(Option<usize>, usize)> {
    dug_hole_run(far, |l| Lighting { fill_samples, ..l }).map(|h| (h.s90, h.lit))
}

/// A dug hole's run (`dug_hole_s90`): `S90`, the lit hole pixels, the settled share at frames
/// 1, 4 and 16 after the dig, and the preload's counts on the dig's frame.
struct DugHole {
    s90: Option<usize>,
    lit: usize,
    shares: [f64; 3],
    dig_frame: vxe_render::marcher::PreloadStats,
}

/// `dug_hole_s90`'s run with the rig's lighting changed by `adjust`, at 192 x 108.
fn dug_hole_run(far: bool, adjust: impl Fn(Lighting) -> Lighting) -> Option<DugHole> {
    dug_hole_run_at(far, [192, 108], adjust)
}

/// `dug_hole_run` at `size`.
fn dug_hole_run_at(
    far: bool,
    size: [u32; 2],
    adjust: impl Fn(Lighting) -> Lighting,
) -> Option<DugHole> {
    use vxe_world::*;
    let (camera, centre) = dig_case(far);
    let mut rig = cache_box_rig_on(
        dig_box(),
        camera,
        vxe_render::marcher::DEFAULT_TABLE_CAP,
        size,
        None,
    )?;
    let l = rig.frame.lighting();
    rig.frame.set_lighting(adjust(l));
    let fill_samples = rig.frame.lighting().fill_samples;
    rig.step(16);
    let r = rig
        .level
        .world
        .apply(&EditOp::sphere(
            Fixed3(centre.map(|v| v * FIXED_ONE + FIXED_ONE / 2)),
            10 * FIXED_ONE,
            Mode::Carve,
        ))
        .expect("the dig applies");
    let (min, max) = r.aabb.expect("the dig changes voxels");
    rig.level
        .gpu
        .sync_now(&rig.level.world, &rig.level.device, &rig.level.queue)
        .expect("the mirror fits");
    rig.frame.invalidate(min.0, max.0);
    let inside = |k: FaceKey| (0..3).all(|a| k.0[a] >= min.0[a] - 1 && k.0[a] <= max.0[a] + 1);
    let mut pixels: Vec<usize> = Vec::new();
    let mut series: Vec<Vec<f64>> = Vec::with_capacity(DIG_FRAMES);
    let mut dig_frame = vxe_render::marcher::PreloadStats::default();
    for f in 1..=DIG_FRAMES {
        rig.step(1);
        let (d, q) = (&rig.level.device, &rig.level.queue);
        if f == 1 {
            dig_frame = rig.frame.read_preload_stats(d, q);
            let pos = vxe_render::readback::read_float(d, q, rig.frame.hit_positions());
            let info = vxe_render::readback::read_rgba16u(d, q, rig.frame.hit_records());
            pixels = (0..pos.len())
                .filter(|&i| face_key(camera.position, pos[i], info[i]).is_some_and(inside))
                .collect();
        }
        let acc = vxe_render::readback::read_float(d, q, rig.frame.accumulated());
        series.push(
            pixels
                .iter()
                .map(|&i| {
                    let c = acc[i];
                    0.2126 * f64::from(c[0]) + 0.7152 * f64::from(c[1]) + 0.0722 * f64::from(c[2])
                })
                .collect(),
        );
    }
    let reference: Vec<f64> = (0..pixels.len())
        .map(|i| {
            series[DIG_FRAMES - DIG_REF..]
                .iter()
                .map(|s| s[i])
                .sum::<f64>()
                / DIG_REF as f64
        })
        .collect();
    let lit: Vec<usize> = (0..pixels.len()).filter(|&i| reference[i] > 1e-6).collect();
    let until = DIG_FRAMES - DIG_REF;
    let settled = |f: usize| {
        lit.iter()
            .filter(|&&i| (series[f - 1][i] / reference[i]).log2().abs() <= 1.1f64.log2())
            .count() as f64
            / lit.len().max(1) as f64
    };
    let s90 = match (1..=until).rev().find(|&f| settled(f) < 0.9) {
        Some(f) if f == until => None,
        Some(f) => Some(f + 1),
        None => Some(0),
    };
    let (by, _) = rig
        .frame
        .read_table_reopens_by(&rig.level.device, &rig.level.queue);
    let preload = rig.frame.lighting().preload_radius;
    println!(
        "reading: dug hole far {far} F {fill_samples} preload {preload}: reopens by branch {by:?}; settled share at 1/4/8/16/32/64/120/240: {:?}; dig frame's preload {dig_frame:?}",
        [1usize, 4, 8, 16, 32, 64, 120, 240].map(|f| (settled(f) * 1000.0).round() / 1000.0)
    );
    Some(DugHole {
        s90,
        lit: lit.len(),
        shares: [settled(1), settled(4), settled(16)],
        dig_frame,
    })
}

/// A dug hole settles within the bar (Plan 0057 Phase 4's dug-hole bar, on the dig box): with
/// the reopened fill at its default, the hole's `S90` reads at most 60 frames at arm's length
/// and at most 120 at 100 voxels. With the fill off (F 0) the 100-voxel dig is read and printed,
/// not asserted: on this box it settles within 120 frames without the fill (its faces cover
/// more than a lighting pixel each), so the fill's need rests on the level's reading
/// (`dig_box_readings`). About 3 x 316 frames.
#[test]
fn a_dug_hole_settles_within_the_bar() {
    let f = vxe_render::marcher::DEFAULT_FILL_SAMPLES;
    let Some((near, near_px)) = dug_hole_s90(false, f) else {
        return;
    };
    let (far, far_px) = dug_hole_s90(true, f).expect("the device opened once");
    let (off, _) = dug_hole_s90(true, 0).expect("the device opened once");
    println!(
        "reading: dug hole, F {f}: S90 at arm's length {near:?} over {near_px} lit hole pixels, at 100 voxels {far:?} over {far_px}; F 0 at 100 voxels {off:?}"
    );
    assert!(
        near_px >= 100 && far_px >= 10,
        "hole pixels {near_px}, {far_px}"
    );
    assert!(
        near.is_some_and(|s| s <= 60),
        "S90 at arm's length {near:?}"
    );
    assert!(far.is_some_and(|s| s <= 120), "S90 at 100 voxels {far:?}");
}

/// A reopened face fills and a new face does not: on the dig box from 100 voxels (`dig_case`)
/// at 96 x 54, scale 1, lamp off, where an end-wall face covers about a third of a lighting
/// pixel, a face seen by one pixel: after an edit box over its voxel clears its cell (64 frames
/// in), its count reads at least F after the next frame and at least 2 F after the second; in
/// a fresh run, the same face, first seen with no edit, reads at most 2 after frame 2.
/// About 2 x 70 frames.
#[test]
fn a_reopened_face_fills_and_a_new_face_does_not() {
    let f = vxe_render::marcher::DEFAULT_FILL_SAMPLES;
    let (camera, _) = dig_case(true);
    let rig_of = || {
        cache_box_rig_on(
            dig_box(),
            camera,
            vxe_render::marcher::DEFAULT_TABLE_CAP,
            [96, 54],
            None,
        )
    };
    let Some(mut fresh) = rig_of() else {
        return;
    };
    fresh.step(1);
    // An end-wall face (x = 108, entry face -x) seen by exactly one pixel.
    let mut ones: Vec<FaceKey> = fresh
        .keys()
        .into_iter()
        .filter(|&((v, face), n)| v[0] == 108 && face == 0 && n == 1)
        .map(|(k, _)| k)
        .collect();
    ones.sort_unstable();
    assert!(ones.len() >= 10, "{} one-pixel end-wall faces", ones.len());
    let key = ones[ones.len() / 2];
    fresh.step(1);
    let new_count = cache_count(&cache_words(&fresh.level, &fresh.frame), key);
    drop(fresh);
    let mut rig = rig_of().expect("the device opened once");
    rig.step(64);
    rig.frame.invalidate(key.0, key.0);
    rig.step(1);
    let c1 = cache_count(&cache_words(&rig.level, &rig.frame), key);
    rig.step(1);
    let c2 = cache_count(&cache_words(&rig.level, &rig.frame), key);
    println!(
        "reading: face {key:?}, F {f}: after the edit box {c1} then {c2}; first seen with no edit {new_count} after frame 2"
    );
    assert!(c1 >= f, "count {c1} after the first frame");
    assert!(c2 >= 2 * f, "count {c2} after the second frame");
    assert!(new_count <= 2, "the new face counts {new_count}");
}

/// [`gap_still_run`]'s keys in view, dark capped keys, keys whose count fell, reopens and
/// uncapped keys.
type GapRun = (Vec<FaceKey>, usize, Vec<FaceKey>, u32, Vec<FaceKey>);

/// The gap box's still run (`a_still_world_never_reopens_a_cell`): `GUARD_WINDOW + 8` frames
/// from the first (the window the first upload opened has closed and every cell in view has
/// capped), then 512 still frames read frame by frame. Returns the keys in view, the dark
/// capped ones among them at the start of the 512, the keys whose count fell in them, and the
/// reopen counter's growth over them.
fn gap_still_run(forced: bool) -> Option<GapRun> {
    let mut rig = gap_rig(GUARD_CAP)?;
    rig.frame.force_guard_window(forced);
    rig.step(vxe_render::marcher::GUARD_WINDOW as usize + 8);
    let mut keys: Vec<FaceKey> = rig.keys().into_keys().collect();
    keys.sort_unstable();
    let words = cache_words(&rig.level, &rig.frame);
    let uncapped: Vec<FaceKey> = keys
        .iter()
        .copied()
        .filter(|&k| cache_count(&words, k) < GUARD_CAP)
        .collect();
    let dark = keys
        .iter()
        .filter(|&&k| cache_count(&words, k) >= GUARD_CAP && cell_lum(&words, k, GUARD_CAP) == 0.0)
        .count();
    let (d, q) = (&rig.level.device, &rig.level.queue);
    let before = rig.frame.read_table_reopens(d, q);
    let (at, _) = reopen_series(&mut rig, &keys, 512);
    let after = rig
        .frame
        .read_table_reopens(&rig.level.device, &rig.level.queue);
    let fell = keys
        .iter()
        .zip(&at)
        .filter(|(_, a)| a.is_some())
        .map(|(k, _)| *k)
        .collect();
    Some((keys, dark, fell, after - before, uncapped))
}

/// A still world never reopens a cell (ADR-0043's guard, within its window only): on the gap
/// box (`gap_rig`: the far wall lit through a small hole, so some of its capped cells
/// drew no light in `GUARD_CAP` samples and the guard's fresh sample may find some), scale 1,
/// lamp off, `C_CAP` 256, once the window the first upload opened has closed, 512 still frames
/// leave every count in view monotone and the reopen counter unchanged; with the window
/// forced open for the whole run the counter grows. About 2 x 780 frames.
#[test]
fn a_still_world_never_reopens_a_cell() {
    let Some((keys, dark, fell, reopens, uncapped)) = gap_still_run(false) else {
        return;
    };
    let (_, dark_forced, _, reopens_forced, _) =
        gap_still_run(true).expect("the device opened once");
    println!(
        "reading: gap box, {} keys in view; dark capped at the start {dark} (forced {dark_forced}); reopens over 512 still frames {reopens}, window forced open {reopens_forced}",
        keys.len()
    );
    assert!(keys.len() >= 100, "{} keys in view", keys.len());
    assert!(uncapped.is_empty(), "uncapped cells: {uncapped:?}");
    assert!(
        dark >= 1,
        "no capped cell is dark: the gap lights every one"
    );
    assert!(dark < keys.len(), "no cell is lit through the gap");
    assert!(fell.is_empty(), "counts fell: {fell:?}");
    assert_eq!(reopens, 0, "the guard reopened {reopens} cells");
    assert!(
        reopens_forced > 0,
        "with the window forced open the guard reopened no cell"
    );
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

// ---- Plan 0057 Phase 4 Part 2b: the camera-radius preload ----

/// A world of air over chunks `[0, 2)^3` with each `(min, max, material)` box filled in turn.
fn air_box_with(fills: &[([i64; 3], [i64; 3], vxe_world::MaterialId)]) -> vxe_world::World {
    use vxe_world::*;
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [2, 2, 2]),
        &UniformSource(AIR),
        &flythrough::with_emissive_ore(&flythrough::cave_materials()),
    );
    for &(min, max, m) in fills {
        world
            .apply(&EditOp::box_(
                Fixed3(min.map(|v| v * FIXED_ONE)),
                Fixed3(max.map(|v| v * FIXED_ONE)),
                Mode::Fill(m),
            ))
            .expect("the box is resident");
    }
    world
}

/// Each pixel's accumulated luminance (the emissive part with the lamp off, at exposure 1).
fn accumulated_lum(rig: &CacheBox) -> Vec<f64> {
    let (d, q) = (&rig.level.device, &rig.level.queue);
    vxe_render::readback::read_float(d, q, rig.frame.accumulated())
        .iter()
        .map(|c| 0.2126 * f64::from(c[0]) + 0.7152 * f64::from(c[1]) + 0.0722 * f64::from(c[2]))
        .collect()
}

/// The pixels whose face key `inside` takes, with their keys, from `eye` this frame.
fn pixels_where(
    rig: &CacheBox,
    eye: Vec3,
    inside: impl Fn(FaceKey) -> bool,
) -> Vec<(usize, FaceKey)> {
    let (d, q) = (&rig.level.device, &rig.level.queue);
    let pos = vxe_render::readback::read_float(d, q, rig.frame.hit_positions());
    let info = vxe_render::readback::read_rgba16u(d, q, rig.frame.hit_records());
    (0..pos.len())
        .filter_map(|i| {
            face_key(eye, pos[i], info[i])
                .filter(|&k| inside(k))
                .map(|k| (i, k))
        })
        .collect()
}

/// Steps `rig` from `camera` until every cell of `keys` reads `cap` samples (checked every 16
/// frames, at most `limit` frames), then returns each pixel's accumulated luminance: the
/// capped cells' settled means, which no later sample moves. `None` when the limit passed.
fn settled_reference(
    rig: &mut CacheBox,
    camera: Camera,
    keys: &[FaceKey],
    cap: u32,
    limit: usize,
) -> Option<Vec<f64>> {
    rig.camera = camera;
    for _ in 0..limit / 16 {
        rig.step(16);
        let words = cache_words(&rig.level, &rig.frame);
        if keys.iter().all(|&k| cache_count(&words, k) >= cap) {
            return Some(accumulated_lum(rig));
        }
    }
    None
}

/// The share of `pixels` (with a reference above 1e-6) whose `now` lies within a factor 1.1 of
/// `reference`, and how many pixels that is.
fn share_within(pixels: &[usize], now: &[f64], reference: &[f64]) -> (f64, usize) {
    let lit: Vec<usize> = pixels
        .iter()
        .copied()
        .filter(|&i| reference[i] > 1e-6)
        .collect();
    let ok = lit
        .iter()
        .filter(|&&i| (now[i] / reference[i]).log2().abs() <= 1.1f64.log2())
        .count();
    (ok as f64 / lit.len().max(1) as f64, lit.len())
}

/// The turn box (done-when 13): a rock floor at z 40..44 and a wall at x 50..54 over x and y
/// 40..90, and an ore strip at the wall's top at x 54..57, y 60..69, z 64..66, whose underside
/// lights the wall's face at x 54 from close above and the corner where it meets the floor, 20
/// voxels below, dimly. The camera at (62.5, 64.5, 50.5), 8.5 voxels from the wall,
/// faces +x (away) and turned faces the corner (yaw 180, pitch -30).
fn turn_box() -> (vxe_world::World, Camera, Camera) {
    use flythrough::cave::{ORE, ROCK};
    let world = air_box_with(&[
        ([40, 40, 40], [90, 90, 44], ROCK),
        ([50, 40, 44], [54, 90, 66], ROCK),
        ([54, 60, 64], [57, 69, 66], ORE),
    ]);
    let away = camera_at([62.5, 64.5, 50.5], 0.0, 0.0);
    let turned = camera_at([62.5, 64.5, 50.5], 180.0, -30.0);
    (world, away, turned)
}

/// The turn box's corner: the wall's faces toward +x at z 44..48 and the floor's toward +z at
/// x 54..58, over y 55..74.
fn turn_corner(k: FaceKey) -> bool {
    let [x, y, z] = k.0;
    (55..74).contains(&y)
        && ((x == 53 && (44..48).contains(&z) && k.1 == 1)
            || ((54..58).contains(&x) && z == 43 && k.1 == 5))
}

/// The mean luminance of the cells of `keys` that hold samples, from the table's words.
fn cells_mean(words: &[u32], keys: &[FaceKey]) -> f64 {
    use vxe_render::marcher::{CACHE_COUNT_WORD, CACHE_Q};
    let means: Vec<f64> = keys
        .iter()
        .filter_map(|&k| cache_cell(words, k))
        .filter(|&w| words[w + CACHE_COUNT_WORD] > 0)
        .map(|w| {
            let n = f64::from(words[w + CACHE_COUNT_WORD].min(1024));
            let c =
                |i: usize| f64::from(words[w + CACHE_COUNT_WORD + 1 + i]) / f64::from(CACHE_Q) / n;
            0.2126 * c(0) + 0.7152 * c(1) + 0.0722 * c(2)
        })
        .collect();
    means.iter().sum::<f64>() / means.len().max(1) as f64
}

/// The turn box's run at `preload_radius`: 64 still frames facing away, a 180-degree yaw in one
/// frame, and on the first frame after it the corner's settled share against the run continued
/// facing the corner until every corner cell reads C_CAP (1024). C_PRE 256, 64 x 36, scale 1,
/// lamp off. Returns the share, the corner's lit pixels, and the corner's and the near wall's
/// (the wall's faces toward +x at z 60..64, over y 55..74, out of view) mean cell luminance.
fn turn_corner_run(preload_radius: u32) -> Option<(f64, usize, f64, f64)> {
    let (world, away, turned) = turn_box();
    let mut rig = cache_box_rig_on(world, away, 1024, [64, 36], None)?;
    let l = rig.frame.lighting();
    rig.frame.set_lighting(Lighting {
        preload_radius,
        preload_until: 256,
        ..l
    });
    rig.step(64);
    let near_keys: Vec<FaceKey> = (55..74)
        .flat_map(|y| (60..64).map(move |z| ([53, y, z], 1u8)))
        .collect();
    assert!(
        pixels_where(&rig, away.position, turn_corner).is_empty(),
        "the corner is out of view facing away"
    );
    rig.camera = turned;
    rig.step(1);
    let corner = pixels_where(&rig, turned.position, turn_corner);
    let now = accumulated_lum(&rig);
    let mut keys: Vec<FaceKey> = corner.iter().map(|&(_, k)| k).collect();
    keys.sort_unstable();
    keys.dedup();
    let reference = settled_reference(&mut rig, turned, &keys, 1024, 4096)
        .expect("the corner's cells reach C_CAP");
    let px: Vec<usize> = corner.iter().map(|&(i, _)| i).collect();
    let (share, lit) = share_within(&px, &now, &reference);
    let words = cache_words(&rig.level, &rig.frame);
    Some((
        share,
        lit,
        cells_mean(&words, &keys),
        cells_mean(&words, &near_keys),
    ))
}

/// A face behind the camera is settled before the turn (Plan 0057 Phase 4 Part 2b, done-when
/// 13): on the turn box (`turn_box`), after 64 still frames facing away with the preload at its
/// default radius, on the first frame after a 180-degree yaw at least 90 % of the dim corner's
/// lit pixels read within 10 % of their cell's settled mean; with the preload off under 50 %
/// do. The corner is lit at under a tenth of the near wall's irradiance. About 2 x 140 frames.
#[test]
fn a_face_behind_the_camera_is_settled_before_the_turn() {
    let r = vxe_render::marcher::DEFAULT_PRELOAD_RADIUS;
    let Some((on, lit, corner, near)) = turn_corner_run(r) else {
        return;
    };
    let (off, lit_off, ..) = turn_corner_run(0).expect("the device opened once");
    println!(
        "reading: turn box: corner within 10 % on the first frame: preload {r} {on:.3} of {lit}, off {off:.3} of {lit_off}; corner mean {corner:.5}, near wall {near:.5}"
    );
    assert!(lit >= 50, "corner pixels {lit}");
    assert!(
        corner > 0.0 && corner < 0.1 * near,
        "corner {corner}, near wall {near}"
    );
    assert!(on >= 0.9, "preload on: {on}");
    assert!(off < 0.5, "preload off: {off}");
}

/// The claim box (done-when 14's asserted half): a rock wall at x 70..72 over all of y and z,
/// and behind it an enclosure (x 72..96, y and z 52..78, walls 4 thick) split at x 80..81 into
/// a pocket (x 72..80) and an emitter chamber (x 81..92) by a divider whose only opening is the
/// one voxel (80, 64, 64); an ore block at x 88..92, y and z 56..74, faces the gap. The eye at
/// (63.5, 64.5, 64.5) faces -x, away from the wall. Face A, the pocket's back wall at the gap's
/// height ([71, 64, 64] toward +x), lies 8.0 voxels from the eye and is lit only through the
/// gap; face B ([70, 64, 60] toward -x, 8.1 voxels) and face C ([70, 104, 64] toward -x, 40.1
/// voxels) lie on the wall's open side, which no emitter reaches.
fn claim_box() -> (vxe_world::World, Camera) {
    use flythrough::cave::{ORE, ROCK};
    let world = air_box_with(&[
        ([88, 56, 56], [92, 74, 74], ORE),
        ([70, 0, 0], [72, 128, 128], ROCK),
        ([72, 52, 52], [96, 56, 78], ROCK),
        ([72, 74, 52], [96, 78, 78], ROCK),
        ([72, 56, 52], [96, 74, 56], ROCK),
        ([72, 56, 74], [96, 74, 78], ROCK),
        ([92, 56, 56], [96, 74, 74], ROCK),
        ([80, 56, 56], [81, 64, 74], ROCK),
        ([80, 65, 56], [81, 74, 74], ROCK),
        ([80, 64, 56], [81, 65, 64], ROCK),
        ([80, 64, 65], [81, 65, 74], ROCK),
    ]);
    (world, camera_at([63.5, 64.5, 64.5], 180.0, 0.0))
}

const CLAIM_GAP_LIT: FaceKey = ([71, 64, 64], 1);
const CLAIM_NEAR_UNLIT: FaceKey = ([70, 64, 60], 0);
const CLAIM_FAR_UNLIT: FaceKey = ([70, 104, 64], 0);

/// Whether the segment from `p` to `q` first meets the claim box's ore (x 88..92), on the CPU.
fn claim_clear(world: &vxe_world::World, p: Vec3, q: Vec3) -> bool {
    let d = q - p;
    vis_cast(world, p, d.normalize(), d.length() + 1.0)
        .is_some_and(|((v, _), _)| (88..92).contains(&v.0[0]))
}

/// The claim box's run: the preload at its default radius, C_PRE 256 and the claim radius
/// `r0`, the random numbers from frame `seed`, one sweep (`ceil(offsets / B)` frames) and 32
/// still frames; returns the three faces' cells as (count, mean luminance), `None` for no cell.
fn claim_run(rig: &mut CacheBox, r0: u32, seed: u32) -> [Option<(u32, f64)>; 3] {
    use vxe_render::marcher::{DEFAULT_PRELOAD_BRICKS, DEFAULT_PRELOAD_RADIUS, preload_offsets};
    let l = rig.frame.lighting();
    rig.frame.reset_history();
    rig.frame.set_frame_index(seed);
    rig.frame.set_lighting(Lighting {
        preload_radius: DEFAULT_PRELOAD_RADIUS,
        preload_until: 256,
        preload_claim_radius: r0,
        ..l
    });
    let sweep = preload_offsets(DEFAULT_PRELOAD_RADIUS / 8)
        .len()
        .div_ceil(DEFAULT_PRELOAD_BRICKS as usize);
    rig.step(sweep + 32);
    let words = cache_words(&rig.level, &rig.frame);
    [CLAIM_GAP_LIT, CLAIM_NEAR_UNLIT, CLAIM_FAR_UNLIT].map(|k| {
        cache_cell(&words, k).map(|w| {
            (
                words[w + vxe_render::marcher::CACHE_COUNT_WORD],
                cells_mean(&words, &[k]),
            )
        })
    })
}

/// A face near the eye is claimed lit or not (Plan 0057 Phase 4 Part 2b, done-when 14's
/// asserted half): on the claim box (`claim_box`), r0 24, C_PRE 256, scale 1, lamp off, after
/// one sweep and 32 still frames facing away from the three faces, the gap-lit face A and the
/// unlit face B, both 8 voxels from the eye, hold cells of C_PRE samples, B's mean 0, and the
/// unlit face C, 40 voxels away, holds no cell; with the claim radius 0 face A holds a cell in
/// under half of 16 seeds (the 8-probe rule alone). Face A's lit share: the share of uniform
/// pairs of points on it and on the ore's face toward it that are clear on the CPU
/// (`World::raycast`), under one in 16; face B's and C's: no ray from them reaches any ore voxel.
#[test]
fn a_face_near_the_eye_is_claimed_lit_or_not() {
    let (world, camera) = claim_box();
    // The faces' lit shares on the CPU.
    let mut rng = 0x2545_F491_u32;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        f32::from(u16::try_from(rng >> 16).expect("16 bits")) / 65536.0
    };
    let pairs = 4096;
    let clear = (0..pairs)
        .filter(|_| {
            let p = Vec3::new(72.01, 64.0 + next(), 64.0 + next());
            let q = Vec3::new(87.99, 56.0 + 18.0 * next(), 56.0 + 18.0 * next());
            claim_clear(&world, p, q)
        })
        .count();
    let lit_share = clear as f64 / f64::from(pairs);
    let ore: Vec<Vec3> = (88..92)
        .flat_map(|x| {
            (56..74).flat_map(move |y| {
                (56..74).map(move |z| Vec3::new(x as f32, y as f32, z as f32) + Vec3::splat(0.5))
            })
        })
        .collect();
    for (name, p) in [
        ("B", Vec3::new(69.99, 64.5, 60.5)),
        ("C", Vec3::new(69.99, 104.5, 64.5)),
    ] {
        assert!(
            ore.iter().all(|&q| !claim_clear(&world, p, q)),
            "no emitter reaches face {name}"
        );
    }
    let Some(mut rig) = cache_box_rig_on(world, camera, 1024, [64, 36], None) else {
        return;
    };
    let near = claim_run(&mut rig, 24, 1);
    let keys = rig.keys();
    assert!(
        [CLAIM_GAP_LIT, CLAIM_NEAR_UNLIT, CLAIM_FAR_UNLIT]
            .iter()
            .all(|k| !keys.contains_key(k)),
        "the three faces are out of view"
    );
    let off: Vec<bool> = (0..16)
        .map(|seed| claim_run(&mut rig, 0, 1 + 7919 * seed)[0].is_some())
        .collect();
    let claimed_off = off.iter().filter(|&&c| c).count();
    println!(
        "reading: claim box: face A lit share {lit_share:.4} ({clear} of {pairs} pairs clear); r0 24: A {:?}, B {:?}, C {:?}; r0 0: A claimed in {claimed_off} of 16 seeds",
        near[0], near[1], near[2]
    );
    assert!(lit_share > 0.0 && lit_share < 1.0 / 16.0, "{lit_share}");
    let [a, b, c] = near;
    assert!(a.is_some_and(|(n, _)| n >= 256), "face A: {a:?}");
    assert!(
        b.is_some_and(|(n, m)| n >= 256 && m == 0.0),
        "face B: {b:?}"
    );
    assert!(c.is_none(), "face C: {c:?}");
    assert!(
        claimed_off < 8,
        "r0 0: face A claimed in {claimed_off} of 16"
    );
}

/// The near dig settles from the preload alone (Plan 0057 Phase 4 Part 2b, done-when 15): on
/// the dig box with the camera 5 voxels from the dig (`dig_case`), the reopened fill off (F 0)
/// and the preload at its default, the dig's frame visits the edit box's bricks and lists
/// faces, at least 90 % of the hole's lit pixels read within 10 % of their settled mean by frame
/// 16 after the dig and its `S90` is at most 60 frames; with both off, `S90` is read beside it
/// (the pixels settle the hole alone at this size). At `NEAR_DIG_SMALL`, where a hole face
/// covers about one lighting pixel, the preload's `S90` is at most 60 and both off read over 60
/// (the test fails first). About 4 x 316 frames.
/// The near dig's small frame: a hole face 5 to 20 voxels away covers about one pixel.
const NEAR_DIG_SMALL: [u32; 2] = [24, 14];

#[test]
fn a_near_dig_settles_from_the_preload_alone() {
    let Some(on) = dug_hole_run(false, |l| Lighting {
        fill_samples: 0,
        ..l
    }) else {
        return;
    };
    let off = dug_hole_run(false, |l| Lighting {
        fill_samples: 0,
        preload_radius: 0,
        ..l
    })
    .expect("the device opened once");
    println!(
        "reading: near dig, F 0: preload on S90 {:?} shares at 1/4/16 {:?} over {}; both off S90 {:?} shares {:?}",
        on.s90, on.shares, on.lit, off.s90, off.shares
    );
    // At 24 x 14 a hole face covers about one lighting pixel, so the pixels alone do not
    // settle it in a few frames (at 192 x 108 they cover hundreds, and both off read `S90` 4).
    let small = |preload_radius| {
        dug_hole_run_at(false, NEAR_DIG_SMALL, |l| Lighting {
            fill_samples: 0,
            preload_radius,
            ..l
        })
        .expect("the device opened once")
    };
    let (on_small, off_small) = (small(vxe_render::marcher::DEFAULT_PRELOAD_RADIUS), small(0));
    println!(
        "reading: near dig at {NEAR_DIG_SMALL:?}, F 0: preload on S90 {:?} shares at 1/4/16 {:?} over {}; both off S90 {:?} shares {:?} over {}",
        on_small.s90, on_small.shares, on_small.lit, off_small.s90, off_small.shares, off_small.lit
    );
    assert!(
        on_small.s90.is_some_and(|s| s <= 60),
        "at {NEAR_DIG_SMALL:?}: S90 {:?}",
        on_small.s90
    );
    assert!(
        off_small.s90.is_none_or(|s| s > 60),
        "both off at {NEAR_DIG_SMALL:?}: S90 {:?}",
        off_small.s90
    );
    assert!(
        on.dig_frame.edit_bricks > 0 && on.dig_frame.listed > 0,
        "the dig's frame: {:?}",
        on.dig_frame
    );
    assert!(on.lit >= 100, "hole pixels {}", on.lit);
    assert!(on.shares[2] >= 0.9, "frame 16: {}", on.shares[2]);
    assert!(on.s90.is_some_and(|s| s <= 60), "S90 {:?}", on.s90);
}

/// The pillar box (done-when 16): a rock floor at z 30..34 over x and y 0..128, a wall at y
/// 66..68, x 0..88, z 34..50 beside the path, and behind its end a rock block at x 92..96, y
/// 70..74, z 34..46 whose faces toward -x and -y (the hidden face) no eye on the path sees
/// until it passes x 80; an ore plate at x 84..100, y 60..80, z 52..54 lights them from above
/// the wall. The path runs along +x at y 64, z 40, heading 20 degrees toward +y, from x 4.
fn pillar_box() -> vxe_world::World {
    use flythrough::cave::{ORE, ROCK};
    air_box_with(&[
        ([0, 0, 30], [128, 128, 34], ROCK),
        ([0, 66, 34], [88, 68, 50], ROCK),
        ([92, 70, 34], [96, 74, 46], ROCK),
        ([84, 60, 52], [100, 80, 54], ORE),
    ])
}

fn pillar_hidden(k: FaceKey) -> bool {
    let [x, y, z] = k.0;
    (34..46).contains(&z)
        && ((x == 92 && (70..74).contains(&y) && k.1 == 0)
            || (y == 70 && (92..96).contains(&x) && k.1 == 2))
}

fn pillar_camera(frame: usize, speed: f32) -> Camera {
    camera_at([4.0 + speed * frame as f32, 64.0, 40.0], 20.0, 0.0)
}

/// The first path frame from whose eye some point of the hidden face is clear on the CPU: a
/// 4 x 4 grid of points on each of its faces, a ray from the eye toward each entering the
/// face's voxel through that face (`vis_cast`).
fn pillar_first_clear(world: &vxe_world::World, speed: f32, frames: usize) -> Option<usize> {
    use vxe_world::Face;
    (0..frames).find(|&f| {
        let eye = pillar_camera(f, speed).position;
        (34..46).any(|z| {
            (0..4).any(|i| {
                let a = i as f32 * 0.25 + 0.125;
                let targets = [
                    (
                        Vec3::new(92.0 - 0.01, 70.0 + 4.0 * a, z as f32 + 0.5),
                        Face::NegX,
                    ),
                    (
                        Vec3::new(92.0 + 4.0 * a, 70.0 - 0.01, z as f32 + 0.5),
                        Face::NegY,
                    ),
                ];
                targets.iter().any(|&(p, face)| {
                    let d = (p - eye).normalize();
                    vis_cast(world, eye, d, 200.0).is_some_and(|((_, f), _)| f == face)
                        && vis_cast(world, eye, d, 200.0)
                            .is_some_and(|((v, _), _)| pillar_hidden((v.0, face_index(face))))
                })
            })
        })
    })
}

/// `vxe_world::Face`'s index in the table's key (`2 axis + 1` toward `+axis`).
fn face_index(f: vxe_world::Face) -> u8 {
    use vxe_world::Face;
    match f {
        Face::NegX => 0,
        Face::PosX => 1,
        Face::NegY => 2,
        Face::PosY => 3,
        Face::NegZ => 4,
        Face::PosZ => 5,
    }
}

/// The pillar box's walk at `speed` voxels a frame with the preload at `preload_radius`: the
/// first frame the hidden face is in view, and its settled share there and `late` frames on
/// (or at its last frame in view, if sooner) against each pixel's cell mean once every cell of
/// the face in view at the late read reads C_CAP (1024), the camera held at that pose; a first
/// frame's pixel whose key is out of view there is not counted. C_PRE 256, 128 x 72, scale 1,
/// lamp off.
fn pillar_run(speed: f32, frames: usize, preload_radius: u32, late: usize) -> Option<PillarRun> {
    let world = pillar_box();
    let first_clear = pillar_first_clear(&world, speed, frames);
    let mut rig = cache_box_rig_on(world, pillar_camera(0, speed), 1024, [128, 72], None)?;
    let l = rig.frame.lighting();
    rig.frame.set_lighting(Lighting {
        preload_radius,
        preload_until: 256,
        ..l
    });
    // The first frame read in view, and the last at most `late` frames after it: the frame,
    // the face's pixels with their keys, every pixel's luminance and the pose.
    type Read = (usize, Vec<(usize, FaceKey)>, Vec<f64>, Camera);
    let mut seen: Option<Read> = None;
    let mut held: Option<Read> = None;
    for f in 0..frames {
        let camera = pillar_camera(f, speed);
        rig.camera = camera;
        rig.step(1);
        let px = pillar_px(&rig, camera);
        if seen.is_none() {
            if !px.is_empty() {
                let lum = accumulated_lum(&rig);
                seen = Some((f, px.clone(), lum.clone(), camera));
                held = Some((f, px, lum, camera));
            }
        } else if px.is_empty() {
            break;
        } else {
            held = Some((f, px, accumulated_lum(&rig), camera));
        }
        if seen.as_ref().is_some_and(|s| f >= s.0 + late) {
            break;
        }
    }
    let (first, px, now, _) = seen.expect("the hidden face comes into view on the path");
    let (late_frame, late_px, late_now, hold) = held.expect("read with the first");
    let mut keys: Vec<FaceKey> = late_px.iter().map(|&(_, k)| k).collect();
    keys.sort_unstable();
    keys.dedup();
    let reference = settled_reference(&mut rig, hold, &keys, 1024, 4096)
        .expect("the face's cells in view reach C_CAP");
    // Each pixel takes its own key's cell mean at the held pose.
    let reference_px = pillar_px(&rig, hold);
    let by_key: std::collections::HashMap<FaceKey, f64> = reference_px
        .iter()
        .map(|&(i, k)| (k, reference[i]))
        .collect();
    let share = |p: &[(usize, FaceKey)], v: &[f64]| {
        let pairs: Vec<(f64, f64)> = p
            .iter()
            .filter_map(|&(i, k)| by_key.get(&k).map(|&r| (v[i], r)))
            .filter(|&(_, r)| r > 1e-6)
            .collect();
        let ok = pairs
            .iter()
            .filter(|&&(a, r)| (a / r).log2().abs() <= 1.1f64.log2())
            .count();
        (ok as f64 / pairs.len().max(1) as f64, pairs.len())
    };
    let (at_first, n_first) = share(&px, &now);
    let (at_late, n_late) = share(&late_px, &late_now);
    Some(PillarRun {
        first_clear,
        first,
        late: late_frame - first,
        at_first,
        n_first,
        at_late,
        n_late,
    })
}

struct PillarRun {
    first_clear: Option<usize>,
    first: usize,
    /// Frames from the first in view to the late read (`late`, or fewer when the face left
    /// the view first).
    late: usize,
    at_first: f64,
    n_first: usize,
    at_late: f64,
    n_late: usize,
}

fn pillar_px(rig: &CacheBox, camera: Camera) -> Vec<(usize, FaceKey)> {
    pixels_where(rig, camera.position, pillar_hidden)
}

/// A face hidden from every eye is settled before it is seen (Plan 0057 Phase 4 Part 2b,
/// done-when 16): on the pillar box (`pillar_box`; the hiding checked on the CPU, no eye
/// before the face's first frame in view sees it), walking at 0.5 voxel a frame from 80 voxels
/// out, on the first frame any of the hidden face's pixels is in view at least 90 % of them read
/// within 10 % of their settled mean, and with the preload off under 50 %; at 2 voxels a frame
/// (Shift) at least 90 % by the tenth frame in view. About 3 runs of 170 and 50 frames.
#[test]
fn a_face_hidden_from_every_eye_is_settled_before_it_is_seen() {
    let r = vxe_render::marcher::DEFAULT_PRELOAD_RADIUS;
    let Some(walk) = pillar_run(0.5, 200, r, 0) else {
        return;
    };
    let off = pillar_run(0.5, 200, 0, 0).expect("the device opened once");
    let shift = pillar_run(2.0, 60, r, 10).expect("the device opened once");
    for (name, run) in [
        ("walk", &walk),
        ("walk, preload off", &off),
        ("shift", &shift),
    ] {
        println!(
            "reading: pillar box {name}: first clear {:?}, first in view {} at x {:.1}: within 10 % {:.3} of {}; {} frames on {:.3} of {}",
            run.first_clear,
            run.first,
            4.0 + if name == "shift" { 2.0 } else { 0.5 } * run.first as f32,
            run.at_first,
            run.n_first,
            run.late,
            run.at_late,
            run.n_late
        );
    }
    for run in [&walk, &off, &shift] {
        assert!(
            run.first_clear.is_some_and(|c| c + 2 >= run.first),
            "the face is hidden from every eye before its first frame in view: clear {:?}, seen {}",
            run.first_clear,
            run.first
        );
    }
    assert!(
        walk.first >= 120,
        "within R for the walk's frames before: {}",
        walk.first
    );
    assert!(walk.n_first >= 1, "the face's first pixels");
    assert!(walk.at_first >= 0.9, "walking: {}", walk.at_first);
    assert!(off.at_first < 0.5, "preload off: {}", off.at_first);
    assert!(shift.n_late >= 1, "the face in view at Shift");
    assert!(
        shift.at_late >= 0.9,
        "shift, by the tenth frame: {}",
        shift.at_late
    );
}

/// The preload readings' lighting at `pose`: lamp off, indirect off, the pose's exposure,
/// the preload at `radius` voxels and `VXE_PRELOAD_BRICKS`, `VXE_PRELOAD_UNTIL`,
/// `VXE_PRELOAD_SAMPLES` and `VXE_PRELOAD_CLAIM` (r0; else the defaults).
fn preload_lighting(pose: &Pose, radius: u32) -> Lighting {
    let env = |name: &str, default: u32| {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };
    use vxe_render::marcher::{
        DEFAULT_PRELOAD_BRICKS, DEFAULT_PRELOAD_CLAIM_RADIUS, DEFAULT_PRELOAD_SAMPLES,
        DEFAULT_PRELOAD_UNTIL,
    };
    Lighting {
        lamp: false,
        lamp_bounce: false,
        exposure: Exposure::Manual(pose.exposure),
        preload_radius: radius,
        preload_bricks: env("VXE_PRELOAD_BRICKS", DEFAULT_PRELOAD_BRICKS),
        preload_until: env("VXE_PRELOAD_UNTIL", DEFAULT_PRELOAD_UNTIL),
        preload_samples: env("VXE_PRELOAD_SAMPLES", DEFAULT_PRELOAD_SAMPLES),
        preload_claim_radius: env("VXE_PRELOAD_CLAIM", DEFAULT_PRELOAD_CLAIM_RADIUS),
        ..Lighting::default()
    }
}

/// A fresh frame for the preload readings: `stillness::SIZE` (or `VXE_PRELOAD_SIZE`, as
/// `WxH`), full-resolution lighting (so a lighting pixel is a hit pixel; half with
/// `VXE_PRELOAD_COST_ONLY`), scale 1, the default look.
fn preload_frame(level: &stillness::VeinLevel, lighting: Lighting) -> Frame {
    let [w, h] = preload_size();
    let mut f = Frame::new(&level.device, w, h);
    f.set_look(look_of(flythrough::LookPreset::DEFAULT));
    // Cost-only readings take the default half-resolution lighting (the Budgets' frame).
    f.set_resolution(
        &level.device,
        if preload_cost_only() {
            LightingResolution::Half
        } else {
            LightingResolution::Full
        },
    );
    f.set_lighting(lighting);
    f.set_dt(1.0 / 60.0);
    f
}

/// One frame's pixels as the preload readings take them: each hit pixel's face key (if it has
/// one), its distance from the eye and its accumulated luminance.
fn preload_pixels(
    level: &stillness::VeinLevel,
    frame: &Frame,
    eye: Vec3,
) -> Vec<Option<(FaceKey, f32, f64)>> {
    let (d, q) = (&level.device, &level.queue);
    let pos = vxe_render::readback::read_float(d, q, frame.hit_positions());
    let info = vxe_render::readback::read_rgba16u(d, q, frame.hit_records());
    let acc = vxe_render::readback::read_float(d, q, frame.accumulated());
    (0..pos.len())
        .map(|i| {
            face_key(eye, pos[i], info[i]).map(|k| {
                let c = acc[i];
                let lum =
                    0.2126 * f64::from(c[0]) + 0.7152 * f64::from(c[1]) + 0.0722 * f64::from(c[2]);
                (
                    k,
                    (Vec3::new(pos[i][0], pos[i][1], pos[i][2]) - eye).length(),
                    lum,
                )
            })
        })
        .collect()
}

/// The preload readings' frame size: `VXE_PRELOAD_SIZE` as `WxH`, else `stillness::SIZE`.
fn preload_size() -> [u32; 2] {
    std::env::var("VXE_PRELOAD_SIZE")
        .ok()
        .and_then(|v| {
            let (w, h) = v.split_once('x')?;
            Some([w.parse().ok()?, h.parse().ok()?])
        })
        .unwrap_or(stillness::SIZE)
}

/// Whether the preload readings read only the passes' cost (`VXE_PRELOAD_COST_ONLY`): no
/// reference run and no pixel readback, so the shares read 0 of 0.
fn preload_cost_only() -> bool {
    std::env::var("VXE_PRELOAD_COST_ONLY").is_ok()
}

/// The settled cell means a run's faces read (the reference of done-whens 14 and 16).
struct PreloadRef {
    /// The table's mean luminance of each cell at C_CAP, times the pose's exposure, by key
    /// (0 for a face no emitter reaches).
    means: std::collections::HashMap<FaceKey, f64>,
    /// The nine decile edges of the lit (above 1e-6) means.
    deciles: Vec<f64>,
    /// Frames held, and the claims the load and miss guards refused over the run.
    held: usize,
    refused: u32,
}

/// The reference: a fresh frame walks `cameras` with the preload at `radius` (the largest the
/// readings read), every face within it claimed whatever its probes read (the claim radius at
/// the same radius; the load and miss guards still apply, and their refusals are counted), C_PRE
/// at C_CAP (4096) and S_PRE at 524 288, holding every 16th camera and the last until the
/// preload's lists read empty (at most 4096 frames each), so every face within `radius` of the
/// path is filled to C_CAP. A reference cell is one at C_CAP; a face without one (beyond the
/// radius of every held camera, or refused by a guard) has none, and the readings count its
/// pixels as not settled.
fn preload_reference(
    level: &stillness::VeinLevel,
    pose: &Pose,
    cameras: &[Camera],
    radius: u32,
) -> PreloadRef {
    preload_reference_seeded(level, pose, cameras, radius, None)
}

/// `preload_reference` with its random numbers from frame `seed` when given.
fn preload_reference_seeded(
    level: &stillness::VeinLevel,
    pose: &Pose,
    cameras: &[Camera],
    radius: u32,
    seed: Option<u32>,
) -> PreloadRef {
    use vxe_render::marcher::{CACHE_CELL_WORDS, CACHE_COUNT_WORD, CACHE_Q, DEFAULT_TABLE_CAP};
    let mut frame = preload_frame(
        level,
        Lighting {
            preload_until: DEFAULT_TABLE_CAP,
            preload_samples: 1 << 19,
            preload_claim_radius: radius,
            ..preload_lighting(pose, radius)
        },
    );
    if let Some(seed) = seed {
        frame.set_frame_index(seed);
    }
    let light = LightParams::headlamp();
    let mut held = 0;
    let mut refused = 0;
    let mut step = |frame: &mut Frame, c: &Camera| {
        frame.step(&level.device, &level.queue, &level.gpu, c, &light);
        let s = frame.read_preload_stats(&level.device, &level.queue);
        refused += s.refused_load + s.refused_miss;
        s
    };
    for (i, c) in cameras.iter().enumerate() {
        step(&mut frame, c);
        let last = i + 1 == cameras.len();
        if i % 16 != 15 && !last {
            continue;
        }
        for f in 0..4096 {
            let s = step(&mut frame, c);
            held += 1;
            if f > 16 && s.list == 0 && s.probes == 0 && s.listed == 0 {
                break;
            }
        }
    }
    let words = cache_words(level, &frame);
    let w = CACHE_CELL_WORDS as usize;
    let mut means = std::collections::HashMap::new();
    for cell in words.chunks_exact(w) {
        if cell[1] & 0x8000_0000 == 0 || cell[CACHE_COUNT_WORD] < DEFAULT_TABLE_CAP {
            continue;
        }
        let k = (cell[0], cell[1]);
        let x = (k.0 & 0xF_FFFF) as i32;
        let y = ((k.0 >> 20) | ((k.1 & 0xFF) << 12)) as i32;
        let z = ((k.1 >> 8) & 0xF_FFFF) as i32;
        let sx = |c: i32| (c << 12) >> 12;
        let face = ((k.1 >> 28) & 7) as u8;
        let n = f64::from(DEFAULT_TABLE_CAP);
        let c = |i: usize| f64::from(cell[CACHE_COUNT_WORD + 1 + i]) / f64::from(CACHE_Q) / n;
        let lum = 0.2126 * c(0) + 0.7152 * c(1) + 0.0722 * c(2);
        means.insert(
            ([sx(x), sx(y), sx(z)], face),
            lum * f64::from(pose.exposure),
        );
    }
    let mut lit: Vec<f64> = means.values().copied().filter(|&v| v > 1e-6).collect();
    lit.sort_by(f64::total_cmp);
    let deciles = (1..10)
        .map(|d| lit.get(lit.len() * d / 10).copied().unwrap_or(0.0))
        .collect();
    PreloadRef {
        means,
        deciles,
        held,
        refused,
    }
}

/// Whether `now` lies within a factor 1.1 of `reference` (above 1e-6), or reads 0 (at most
/// 1e-6) where the reference does.
fn within_tenth(now: f64, reference: f64) -> bool {
    if reference <= 1e-6 {
        return now <= 1e-6;
    }
    (now / reference).log2().abs() <= 1.1f64.log2()
}

/// A band's tally of pixels against the reference: pixels, within 10 %, pixels with no
/// reference cell (counted as not settled), unlit-reference pixels and those reading 0, and the
/// lit pixels by the decile of their reference mean (pixels, within 10 %).
#[derive(Clone, Default)]
struct RefTally {
    n: usize,
    ok: usize,
    noref: usize,
    unlit: [usize; 2],
    deciles: [[usize; 2]; 10],
}

impl RefTally {
    fn add(&mut self, r: &PreloadRef, k: &FaceKey, lum: f64) {
        self.n += 1;
        let Some(&rf) = r.means.get(k) else {
            self.noref += 1;
            return;
        };
        let ok = within_tenth(lum, rf);
        self.ok += usize::from(ok);
        if rf <= 1e-6 {
            self.unlit[0] += 1;
            self.unlit[1] += usize::from(ok);
        } else {
            let d = r.deciles.iter().filter(|&&e| rf >= e).count().min(9);
            self.deciles[d][0] += 1;
            self.deciles[d][1] += usize::from(ok);
        }
    }

    fn line(&self) -> String {
        let deciles: Vec<String> = self
            .deciles
            .iter()
            .map(|d| format!("{:.2}/{}", d[1] as f64 / d[0].max(1) as f64, d[0]))
            .collect();
        format!(
            "{:.3} of {} (no reference {}{}; unlit {}/{}; lit by decile {})",
            self.ok as f64 / self.n.max(1) as f64,
            self.n,
            self.noref,
            if self.noref * 100 > self.n {
                ", over 1 %: not a reading"
            } else {
                ""
            },
            self.unlit[1],
            self.unlit[0],
            deciles.join(" ")
        )
    }
}

/// The preload readings' bands of a face's voxel centre's distance from the eye: within the
/// claim radius r0, r0..64, 64..96, 96..128 and beyond.
fn preload_band_of(k: &FaceKey, eye: Vec3, r0: f32) -> usize {
    let c = Vec3::new(k.0[0] as f32, k.0[1] as f32, k.0[2] as f32) + Vec3::splat(0.5);
    let d = (c - eye).length();
    [r0, 64.0, 96.0, 128.0]
        .iter()
        .position(|&e| d <= e)
        .unwrap_or(4)
}

/// The five bands' tallies in a line.
fn band_lines(t: &[RefTally; 5], r0: f32) -> String {
    let names = [
        format!("within r0 {r0}"),
        "r0..64".to_owned(),
        "64..96".to_owned(),
        "96..128".to_owned(),
        "beyond".to_owned(),
    ];
    names
        .iter()
        .zip(t)
        .map(|(n, t)| format!("{n}: {}", t.line()))
        .collect::<Vec<_>>()
        .join("; ")
}

/// The preload on the level (Plan 0057 Phase 4 Part 2b, done-whens 12, 14, 16 and 17), lamp
/// off, scale 1, full-resolution lighting at `stillness::SIZE`, each pose of
/// `VXE_SETTLE_POSES`, each radius of `VXE_PRELOAD_RADII` (default 64 and 0) and C_PRE of
/// `VXE_PRELOAD_UNTILS` (default `VXE_PRELOAD_UNTIL`, else 256), the claim radius
/// `VXE_PRELOAD_CLAIM` (r0, default 32), the runs of `VXE_PRELOAD_RUNS` (default all). The
/// shares are against `preload_reference` at the largest radius read (at least 64; one reference
/// a run, shared by its radii and C_PRE; cells at C_CAP; a pixel whose face has none counts as
/// not settled), by band of the face's voxel centre's distance from the eye (within r0, r0..64,
/// 64..96, 96..128, beyond) and, for lit faces, by decile of the reference mean:
/// - `fill`: from a fresh table at the pose, still: frames until the list first reads empty,
///   the cells claimed by band (within r0 lit and unlit, r0 to R) and refused (load, miss,
///   unlit) and the list's drops over them, the occupancy and misses then, and the passes'
///   median GPU time over 120 frames at rest after.
/// - `turn`: 120 still frames, then the rig's `yaw 90 10 f` and 10 still frames: of the pixels
///   whose face was not in view in the 120 frames, the share within 10 % of the face's
///   reference on the face's first frame in view, and on the tenth frame after the yaw ends.
/// - `walk` and `shift`: the steady flight at 0.5 and 2 voxels a frame: of the entering pixels
///   (a face no pixel hit in the 60 frames before), the share within 10 % on their first
///   frame; the passes' median GPU time over the flight.
/// - `corner`: `VEIN_CORNER` (the lit faces within 24 voxels of the eye and above it in the
///   lowest tenth of their settled means, vein only): the 60 voxels of the path into the pose
///   at 0.5 voxel a frame heading 180 degrees from the corner, then `yaw 180 20 f`; on each
///   frame the corner is in view, the share of its pixels within 10 %, and the frames each
///   corner face lay within r0 of the eye before its first frame in view.
#[test]
#[ignore = "reading only: run with --ignored"]
fn preload_level_readings() {
    use std::collections::{HashMap, HashSet};
    let Some(level) = reading_level() else {
        return;
    };
    let adapter = level.adapter.clone();
    let light = LightParams::headlamp();
    let radii: Vec<u32> = list("VXE_PRELOAD_RADII", &["64", "0"])
        .iter()
        .map(|r| r.parse().expect("a radius"))
        .collect();
    let until_default = std::env::var("VXE_PRELOAD_UNTIL").unwrap_or_else(|_| "256".to_owned());
    let untils: Vec<u32> = list("VXE_PRELOAD_UNTILS", &[until_default.as_str()])
        .iter()
        .map(|c| c.parse().expect("a count"))
        .collect();
    let ref_radius = radii.iter().copied().max().unwrap_or(64).max(64);
    let combos: Vec<(u32, u32)> = radii
        .iter()
        .flat_map(|&r| {
            // Off reads once: C_PRE does not apply.
            let cs: Vec<u32> = if r == 0 {
                untils[..1].to_vec()
            } else {
                untils.clone()
            };
            cs.into_iter().map(move |c| (r, c))
        })
        .collect();
    let runs = list(
        "VXE_PRELOAD_RUNS",
        &["fill", "turn", "walk", "shift", "corner"],
    );
    let timer = vxe_render::PassTimer::with_passes(
        &level.device,
        &level.queue,
        vxe_render::marcher::FRAME_PASSES.len() as u32,
    );
    let costs = |ms: &[Vec<f64>]| -> String {
        vxe_render::marcher::FRAME_PASSES
            .iter()
            .zip(ms)
            .filter(|(n, _)| ["preload", "light", "fill", "temporal"].contains(n))
            .map(|(n, v)| {
                let mut v = v.clone();
                v.sort_by(f64::total_cmp);
                format!("{n} {:.4}", v.get(v.len() / 2).copied().unwrap_or(0.0))
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let setup = |pose: &Pose, r: u32, c: u32| {
        let lighting = Lighting {
            preload_until: c,
            ..preload_lighting(pose, r)
        };
        let r0 = lighting.preload_claim_radius.min(r) as f32;
        let tag = format!(
            "{} R {r} r0 {r0} C {c} {}",
            pose.name,
            std::env::var("VXE_PRELOAD_TAG").unwrap_or_default()
        );
        (lighting, r0, tag)
    };
    for pose in poses() {
        if runs.iter().any(|x| x == "fill") {
            for &(r, c) in &combos {
                let (lighting, _, tag) = setup(&pose, r, c);
                let mut frame = preload_frame(&level, lighting);
                let cam = pose.camera();
                let mut sum = vxe_render::marcher::PreloadStats::default();
                let mut empty_at = None;
                for f in 1..=4096 {
                    frame.step(&level.device, &level.queue, &level.gpu, &cam, &light);
                    let s = frame.read_preload_stats(&level.device, &level.queue);
                    sum.claimed += s.claimed;
                    sum.claimed_near_lit += s.claimed_near_lit;
                    sum.claimed_near_unlit += s.claimed_near_unlit;
                    sum.refused_load += s.refused_load;
                    sum.refused_miss += s.refused_miss;
                    sum.refused_unlit += s.refused_unlit;
                    sum.drops += s.drops;
                    sum.samples += s.samples;
                    if f > 1 && s.list == 0 && s.listed == 0 && s.probes == 0 {
                        empty_at = Some(f);
                        break;
                    }
                    if r == 0 && f >= 1 {
                        break;
                    }
                }
                let t = frame.read_table_stats(&level.device, &level.queue);
                // The cells against the reference (`VXE_PRELOAD_CELLCHECK`): of the lit
                // reference cells within r0 that the frame holds, by decile of the reference
                // mean, the median ratio of the frame's mean to the reference's and the share
                // within 10 %; and two references' agreement, a second at another seed.
                if r > 0 && std::env::var("VXE_PRELOAD_CELLCHECK").is_ok() {
                    let r0 = lighting.preload_claim_radius.min(r) as f32;
                    let reference =
                        preload_reference(&level, &pose, std::slice::from_ref(&cam), ref_radius);
                    let words = cache_words(&level, &frame);
                    let eye = cam.position;
                    let mut by: Vec<Vec<f64>> = vec![Vec::new(); 10];
                    for (k, &rf) in &reference.means {
                        if rf <= 1e-6 || preload_band_of(k, eye, r0) != 0 {
                            continue;
                        }
                        let Some(w) = cache_cell(&words, *k) else {
                            continue;
                        };
                        let n = words[w + vxe_render::marcher::CACHE_COUNT_WORD]
                            .min(vxe_render::marcher::DEFAULT_TABLE_CAP);
                        if n == 0 {
                            continue;
                        }
                        let q = |i: usize| {
                            f64::from(words[w + vxe_render::marcher::CACHE_COUNT_WORD + 1 + i])
                                / f64::from(vxe_render::marcher::CACHE_Q)
                                / f64::from(n)
                        };
                        let m = (0.2126 * q(0) + 0.7152 * q(1) + 0.0722 * q(2))
                            * f64::from(pose.exposure);
                        let d = reference
                            .deciles
                            .iter()
                            .filter(|&&e| rf >= e)
                            .count()
                            .min(9);
                        by[d].push(m / rf);
                    }
                    let line: Vec<String> = by
                        .iter_mut()
                        .map(|v| {
                            v.sort_by(f64::total_cmp);
                            let within = v.iter().filter(|&&x| within_tenth(x, 1.0)).count();
                            format!(
                                "{:.3}/{:.2}/{}",
                                v.get(v.len() / 2).copied().unwrap_or(0.0),
                                within as f64 / v.len().max(1) as f64,
                                v.len()
                            )
                        })
                        .collect();
                    let second = preload_reference_seeded(
                        &level,
                        &pose,
                        std::slice::from_ref(&cam),
                        ref_radius,
                        Some(0x5EED_0001),
                    );
                    let mut agree: Vec<Vec<f64>> = vec![Vec::new(); 10];
                    for (k, &rf) in &reference.means {
                        if let Some(&r2) = second.means.get(k)
                            && rf > 1e-6
                            && preload_band_of(k, eye, r0) == 0
                        {
                            let d = reference
                                .deciles
                                .iter()
                                .filter(|&&e| rf >= e)
                                .count()
                                .min(9);
                            agree[d].push(r2 / rf);
                        }
                    }
                    let agree: Vec<String> = agree
                        .iter_mut()
                        .map(|v| {
                            v.sort_by(f64::total_cmp);
                            let within = v.iter().filter(|&&x| within_tenth(x, 1.0)).count();
                            format!(
                                "{:.3}/{:.2}/{}",
                                v.get(v.len() / 2).copied().unwrap_or(0.0),
                                within as f64 / v.len().max(1) as f64,
                                v.len()
                            )
                        })
                        .collect();
                    println!(
                        "reading[{adapter}]: preload cells {tag}: lit cells within r0 by reference decile (median ratio / share within 10 % / cells): {}; a second reference against the first: {}",
                        line.join(" "),
                        agree.join(" ")
                    );
                }
                // At rest: 256 more still frames after the list first reads empty.
                if empty_at.is_some() {
                    for _ in 0..256 {
                        frame.step(&level.device, &level.queue, &level.gpu, &cam, &light);
                    }
                }
                let skips_before = frame.preload_skips();
                let mut ms = vec![Vec::new(); vxe_render::marcher::FRAME_PASSES.len()];
                if let Some(timer) = &timer {
                    for _ in 0..120 {
                        let v = timed_step(&level, &mut frame, timer, &cam);
                        for (a, x) in ms.iter_mut().zip(v) {
                            a.push(x);
                        }
                    }
                }
                println!(
                    "reading[{adapter}]: preload fill {tag}: list empty at frame {empty_at:?}; claimed within r0 lit {} unlit {}, r0 to R {}; refused load {} miss {} unlit {}, drops {}, samples {}; occupied {} of {} ({:.4}), misses {}; at rest: {} (idle skips {} of 120)",
                    sum.claimed_near_lit,
                    sum.claimed_near_unlit,
                    sum.claimed - sum.claimed_near_lit - sum.claimed_near_unlit,
                    sum.refused_load,
                    sum.refused_miss,
                    sum.refused_unlit,
                    sum.drops,
                    sum.samples,
                    t.occupied,
                    t.cells,
                    f64::from(t.occupied) / f64::from(t.cells),
                    t.misses,
                    costs(&ms),
                    frame.preload_skips() - skips_before
                );
            }
        }
        if runs.iter().any(|x| x == "turn") {
            let mut cams: Vec<Camera> = vec![KRun::Yaw.camera(&pose, 0); 120];
            cams.extend((1..=20).map(|f| KRun::Yaw.camera(&pose, f)));
            let reference = preload_reference(&level, &pose, &cams, ref_radius);
            for &(r, c) in &combos {
                let (lighting, r0, tag) = setup(&pose, r, c);
                let mut frame = preload_frame(&level, lighting);
                let mut seen: HashSet<FaceKey> = HashSet::new();
                let mut first: HashSet<FaceKey> = HashSet::new();
                let mut at1: [RefTally; 5] = Default::default();
                let mut at10: [RefTally; 5] = Default::default();
                for (f, c) in cams.iter().enumerate() {
                    frame.step(&level.device, &level.queue, &level.gpu, c, &light);
                    let px = preload_pixels(&level, &frame, c.position);
                    if f < 120 {
                        seen.extend(px.iter().flatten().map(|p| p.0));
                        continue;
                    }
                    let mut new_keys = HashSet::new();
                    for &(k, _, lum) in px.iter().flatten() {
                        if seen.contains(&k) {
                            continue;
                        }
                        let b = preload_band_of(&k, c.position, r0);
                        if !first.contains(&k) {
                            new_keys.insert(k);
                            at1[b].add(&reference, &k, lum);
                        }
                        if f == 139 {
                            at10[b].add(&reference, &k, lum);
                        }
                    }
                    first.extend(new_keys);
                }
                println!(
                    "reading[{adapter}]: preload turn {tag}: entering pixels within 10 % on the face's first frame: {}",
                    band_lines(&at1, r0)
                );
                println!(
                    "reading[{adapter}]: preload turn {tag}: on the tenth frame after the yaw: {} (reference R {ref_radius} held {}, refused {})",
                    band_lines(&at10, r0),
                    reference.held,
                    reference.refused
                );
            }
        }
        for (run, speed) in [("walk", 0.5f32), ("shift", 2.0)] {
            if !runs.iter().any(|x| x == run) {
                continue;
            }
            let (start, end) = flight_path(&pose);
            let (start, end) = (Vec3::from(start), Vec3::from(end));
            let dir = (end - start).normalize();
            let len = (end - start).length();
            let frames = (len / speed) as usize;
            let cams: Vec<Camera> = (0..=frames)
                .map(|f| {
                    let mut c = KRun::Path.camera(&pose, 0);
                    c.position = start + dir * (speed * f as f32).min(len);
                    c
                })
                .collect();
            let reference = if preload_cost_only() {
                PreloadRef {
                    means: HashMap::new(),
                    deciles: Vec::new(),
                    held: 0,
                    refused: 0,
                }
            } else {
                preload_reference(&level, &pose, &cams, ref_radius)
            };
            for &(r, c) in &combos {
                let (lighting, r0, tag) = setup(&pose, r, c);
                let mut frame = preload_frame(&level, lighting);
                let mut last_seen: HashMap<FaceKey, usize> = HashMap::new();
                let mut bands: [RefTally; 5] = Default::default();
                let mut ms = vec![Vec::new(); vxe_render::marcher::FRAME_PASSES.len()];
                // The preload's samples, faces probed, faces listed and cells claimed, summed.
                let mut pre_sum = [0u64; 4];
                for (f, c) in cams.iter().enumerate() {
                    match &timer {
                        Some(t) if f > 0 => {
                            for (a, x) in ms.iter_mut().zip(timed_step(&level, &mut frame, t, c)) {
                                a.push(x);
                            }
                        }
                        _ => frame.step(&level.device, &level.queue, &level.gpu, c, &light),
                    }
                    let st = frame.read_preload_stats(&level.device, &level.queue);
                    pre_sum[0] += u64::from(st.samples);
                    pre_sum[1] += u64::from(st.probed);
                    pre_sum[2] += u64::from(st.listed);
                    pre_sum[3] += u64::from(st.claimed);
                    if preload_cost_only() {
                        continue;
                    }
                    let px = preload_pixels(&level, &frame, c.position);
                    for &(k, _, lum) in px.iter().flatten() {
                        let entering = f >= 60 && last_seen.get(&k).is_none_or(|&s| f - s > 60);
                        if entering {
                            bands[preload_band_of(&k, c.position, r0)].add(&reference, &k, lum);
                        }
                    }
                    for &(k, ..) in px.iter().flatten() {
                        last_seen.insert(k, f);
                    }
                }
                println!(
                    "reading[{adapter}]: preload {run} {tag} at {speed} voxels a frame: entering pixels within 10 % on their first frame: {} (reference R {ref_radius} held {}, refused {}); in flight: {}; a frame: {:.0} samples, {:.1} faces probed, {:.1} listed, {:.1} claimed",
                    if preload_cost_only() {
                        "not read (cost only)".to_owned()
                    } else {
                        band_lines(&bands, r0)
                    },
                    reference.held,
                    reference.refused,
                    costs(&ms),
                    pre_sum[0] as f64 / cams.len() as f64,
                    pre_sum[1] as f64 / cams.len() as f64,
                    pre_sum[2] as f64 / cams.len() as f64,
                    pre_sum[3] as f64 / cams.len() as f64,
                );
            }
        }
        if runs.iter().any(|x| x == "corner") && pose.name == VEIN_POSE.name {
            let eye = Vec3::from(pose.position);
            let probe = preload_reference(&level, &pose, &[pose.camera()], ref_radius);
            let mut lit: Vec<(FaceKey, f64)> = probe
                .means
                .iter()
                .filter(|(k, v)| {
                    let c =
                        Vec3::new(k.0[0] as f32, k.0[1] as f32, k.0[2] as f32) + Vec3::splat(0.5);
                    **v > 1e-6 && (c - eye).length() <= 24.0 && c.z > eye.z
                })
                .map(|(k, v)| (*k, *v))
                .collect();
            lit.sort_by(|a, b| a.1.total_cmp(&b.1));
            let corner: HashSet<FaceKey> = lit[..lit.len() / 10].iter().map(|p| p.0).collect();
            let lo = corner
                .iter()
                .fold([i32::MAX; 3], |a, k| [0, 1, 2].map(|i| a[i].min(k.0[i])));
            let hi = corner
                .iter()
                .fold([i32::MIN; 3], |a, k| [0, 1, 2].map(|i| a[i].max(k.0[i])));
            let centre: Vec3 = corner
                .iter()
                .map(|k| Vec3::new(k.0[0] as f32, k.0[1] as f32, k.0[2] as f32))
                .sum::<Vec3>()
                / corner.len().max(1) as f32;
            let to = centre - eye;
            let corner_yaw = to.y.atan2(to.x).to_degrees();
            let corner_pitch = (to.z / to.length()).asin().to_degrees();
            let away = corner_yaw + 180.0;
            let back = Vec3::new(away.to_radians().cos(), away.to_radians().sin(), 0.0);
            let mut cams: Vec<Camera> = (0..=120)
                .map(|f| camera_at((eye - back * (60.0 - 0.5 * f as f32)).to_array(), away, 0.0))
                .collect();
            cams.extend((1..=30).map(|f| {
                let t = (f.min(20) as f32) / 20.0;
                camera_at(pose.position, away - 180.0 * t, corner_pitch * t)
            }));
            let reference = preload_reference(&level, &pose, &cams, ref_radius);
            for &(r, c) in &combos {
                let (lighting, r0, tag) = setup(&pose, r, c);
                let mut frame = preload_frame(&level, lighting);
                let mut line = Vec::new();
                // Each corner face's frames within r0 of the eye before its first in view.
                let mut lead: HashMap<FaceKey, usize> = HashMap::new();
                let mut leads: Vec<usize> = Vec::new();
                for (f, c) in cams.iter().enumerate() {
                    frame.step(&level.device, &level.queue, &level.gpu, c, &light);
                    let px = (f >= 120).then(|| preload_pixels(&level, &frame, c.position));
                    let in_view: HashSet<FaceKey> = px
                        .iter()
                        .flatten()
                        .flatten()
                        .map(|p| p.0)
                        .filter(|k| corner.contains(k))
                        .collect();
                    for k in &corner {
                        if lead.get(k) == Some(&usize::MAX) {
                            continue;
                        }
                        if in_view.contains(k) {
                            leads.push(lead.get(k).copied().unwrap_or(0));
                            lead.insert(*k, usize::MAX);
                        } else if preload_band_of(k, c.position, r0) == 0 {
                            *lead.entry(*k).or_insert(0) += 1;
                        }
                    }
                    let Some(px) = px else {
                        continue;
                    };
                    let mut t = RefTally::default();
                    for &(k, _, lum) in px.iter().flatten() {
                        if corner.contains(&k) {
                            t.add(&reference, &k, lum);
                        }
                    }
                    if t.n > 0 {
                        line.push(format!(
                            "{}: {:.3}/{} (no ref {})",
                            f - 120,
                            t.ok as f64 / t.n as f64,
                            t.n,
                            t.noref
                        ));
                    }
                }
                leads.sort_unstable();
                println!(
                    "reading[{adapter}]: preload corner {tag}: VEIN_CORNER {} faces, box {lo:?}..{hi:?}, yaw {corner_yaw:.1} pitch {corner_pitch:.1}; per frame in view (frame after the walk: share/pixels): {}; frames within r0 before the first in view, of {} faces seen: least {}, median {}, most {} (reference R {ref_radius} held {}, refused {})",
                    corner.len(),
                    line.join(" "),
                    leads.len(),
                    leads.first().copied().unwrap_or(0),
                    leads.get(leads.len() / 2).copied().unwrap_or(0),
                    leads.last().copied().unwrap_or(0),
                    reference.held,
                    reference.refused
                );
            }
        }
    }
}
