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
#[ignore = "soak"]
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
#[ignore = "soak"]
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
#[ignore = "soak"]
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
#[ignore = "soak"]
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
#[ignore = "soak"]
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

/// Air over chunks `[0, 2)^3`, a rock block at x, y in `[40, 64)`, z in `[40, 64)`, and an ore
/// plate over it at z `[80, 84)`: the block's corner faces the camera, its faces lit from above.
fn ore_box() -> (vxe_world::World, Camera) {
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

/// Readings: measurement tests that print numbers and assert nothing a tier runs. Each is
/// `#[ignore]`d and run by hand, `cargo nextest run -p <package> --run-ignored only -E
/// 'test(=reading::<name>)'`; the soak and full tiers leave this module out by its name.
mod reading {
    use super::*;

    /// The dots after the fast yaws, the slow yaw (90 degrees over 118 frames) and the two digs
    /// (with their settle).
    #[test]
    #[ignore = "reading only: run with --ignored"]
    fn dot_readings() {
        run_settle(
            &["yaw10", "yaw30", "yaw118", "yaw180", "dig", "dig100"],
            false,
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
    /// `ore_box` camera and at the level's spawn facing +x, the direct lamp irradiance `D` at the
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
        let (box_world, box_camera) = ore_box();
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
            ("ore_box", &box_gpu, box_camera),
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

    /// Prints the adapter `VXE_TEST_ADAPTER` picks for the reading tests.
    #[test]
    #[ignore = "reading only: run with --ignored"]
    fn reading_adapter() {
        let Some((_, _, name)) = reading_device(64 << 20) else {
            return;
        };
        println!("reading adapter: {name}");
    }

    /// The settle of every part after each event, pose and setting.
    #[test]
    #[ignore = "reading only: run with --ignored"]
    fn settle_readings() {
        run_settle(&["toggle", "dig"], true);
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
            let mut step_max = 0u8;
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
                last = now;
            }
            println!(
                "reading[{adapter}]: still lamp {} bounce {bounce}: after {settle} frames, pixels moved per frame over 64 {moved:?} (of {}); probe rays dispatched over them {rays} (a frozen probe returns at once); largest channel step {step_max}",
                pose.name,
                last.len() / 4
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
                let Some((_, w_off)) = by.iter().find(|(s, _)| {
                    !s.indirect && s.lamp == s_on.lamp && s.upscaler == s_on.upscaler
                }) else {
                    continue;
                };
                for part in [Part::Composite, Part::Display] {
                    let get = |w: &[(Part, Option<Wander>)]| {
                        w.iter().find(|(p, _)| *p == part).and_then(|(_, w)| *w)
                    };
                    if let (Some(on), Some(off)) = (get(w_on), get(w_off)) {
                        let share = (on.sigma_view.powi(2) - off.sigma_view.powi(2))
                            / on.sigma_view.powi(2);
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
}
