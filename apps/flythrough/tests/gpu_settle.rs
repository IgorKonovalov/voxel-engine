//! How the lamp channel's light-change window opens, how a still lamp freezes the channel and a
//! lamp step moves only the probes in reach, and the rig's default look, on the level's ore
//! veins at `stillness`'s poses (`support::settle`), plus the flicker metric's and the face
//! key's arithmetic.
//!
//! The tests run on the software adapter (lavapipe). The settle, dot, flicker, cost and probe
//! readings on the same rig are the `settle` example (`cargo run -p flythrough --example
//! settle -- --help`).

mod support;

use support::settle::*;
use support::stillness;
use support::stillness::{VEIN_POSE, vein_level};
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
            .map(|i| !from && i < vxe_render::internals::WINDOW_FRAMES as usize)
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

/// The rig reads what the fly-through starts in: `LookPreset::DEFAULT` with the fly-through's
/// store read and filter weights, FSR 2 at scale 1.5 with sharpening 0.5, over the level's
/// table with the ore emissive.
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
            probe_freeze: None,
            probe_move: None,
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
    let lighting = frame.lighting();
    assert_eq!(
        (lighting.store_read, lighting.light_weights),
        (
            flythrough::DEFAULT_STORE_READ,
            flythrough::DEFAULT_LIGHT_WEIGHTS
        )
    );
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
    vxe_render::debug::read_buffer(&level.device, &level.queue, data, 0, data.size())
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
