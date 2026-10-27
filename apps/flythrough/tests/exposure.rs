//! The eye's exposure on the level (ADR-0033), headless on the software adapter (lavapipe):
//! the lamp switched off in the cave and the exposure's measured curve.

mod support;

use support::*;
use vxe_render::exposure::ExposureState;
use vxe_render::{
    AutoExposure, Exposure, Frame, GpuWorld, LightParams, Lighting, MirrorConfig, Window, readback,
    wgpu,
};
use vxe_world::*;

/// Frames with the lamp on, then with it off.
const LAMP_ON: usize = 600;
const LAMP_OFF: usize = 900;

/// A pose on the level out from an ore vein along its normal, looking at it: position, yaw and
/// pitch in degrees. Its lamp-off meter (about -4.9 EV) settles above `max`'s clamp (-6.47 EV).
const VEIN_POSE: ([f32; 3], f32, f32) = ([727.5, 644.3, 208.5], 153.4, 0.0);

/// The level (seed 1, the ore emissive) mirrored whole on the software adapter.
struct Level {
    device: wgpu::Device,
    queue: wgpu::Queue,
    gpu: GpuWorld,
    spawn: [f32; 3],
}

fn level() -> Option<Level> {
    let source = flythrough::cave::CaveSource::new(
        1,
        ChunkExtent::new([0, 0, 0], flythrough::cave::LEVEL_CHUNKS),
    );
    let world = World::generate(
        source.extent,
        &source,
        &flythrough::with_emissive_ore(&flythrough::cave_materials()),
    );
    let pool = flythrough::level_pool_units(&world, false) * vxe_render::gpu_world::UNIT_BYTES;
    let (device, queue) = software_device(pool)?;
    let cfg = MirrorConfig {
        budget_bytes: 256 << 20,
        ..MirrorConfig::new(Window::Fixed(world.extent()))
    };
    let mut gpu = GpuWorld::with_config(&device, cfg);
    gpu.set_focus(source.spawn());
    for _ in 0..4 {
        gpu.sync_now(&world, &device, &queue)
            .expect("mirror fits the adapter");
    }
    Some(Level {
        device,
        queue,
        gpu,
        spawn: source.spawn().0.map(|v| v as f32 + 0.5),
    })
}

fn camera(position: [f32; 3], yaw_deg: f32, pitch_deg: f32) -> vxe_render::Camera {
    vxe_render::Camera {
        position: position.into(),
        yaw: yaw_deg.to_radians(),
        pitch: pitch_deg.to_radians(),
        vfov: 70f32.to_radians(),
    }
}

/// The exposure state after each of `LAMP_ON` frames with the lamp on and `LAMP_OFF` with it
/// off (switched through `Frame::set_lighting`), at 128 x 72 with `dt` 1/60, the
/// fly-through's lighting (indirect light on) and `exposure`.
fn lamp_off_states(
    level: &Level,
    camera: &vxe_render::Camera,
    exposure: Exposure,
) -> Vec<ExposureState> {
    let lit = Lighting {
        indirect: true,
        exposure,
        ..Lighting::default()
    };
    let mut frame = Frame::new(&level.device, 128, 72);
    frame.set_lighting(lit);
    frame.set_dt(1.0 / 60.0);
    let light = LightParams::headlamp();
    let mut states = Vec::with_capacity(LAMP_ON + LAMP_OFF);
    for i in 0..LAMP_ON + LAMP_OFF {
        if i == LAMP_ON {
            frame.set_lighting(Lighting { lamp: false, ..lit });
        }
        frame.step(&level.device, &level.queue, &level.gpu, camera, &light);
        states.push(ExposureState::from_bytes(&readback::read_buffer(
            &level.device,
            &level.queue,
            frame.exposure_state(),
            0,
            32,
        )));
    }
    states
}

/// Plan 0040 Phase 4 bar 2 on one pose's lamp-off series under `AutoExposure::eye()`. It prints
/// the metered EV before and after and `log2 e` at 0, 0.25, 0.5, 1, 2, 4, 8, 12 and 15 s, and
/// asserts: after the switch `log2 e` falls by no more than 0.01 EV between two frames; by
/// 15 s it is within 0.25 EV of its settled value (the key's exposure for the meter, or `max`);
/// from 1 s on it is within 0.1 EV of `AutoExposure::step` stepped over each frame's measured
/// meter (a); and, when `closed_form` is set, within 0.1 EV of the closed form for the
/// measured meters (b). Without it the closed form's distance is a printed reading.
fn check_curve(name: &str, states: &[ExposureState], closed_form: bool) {
    let eye = AutoExposure::eye();
    // The exposure frame LAMP_ON + k uses: the state after the frame before it.
    let used = |k: usize| f64::from(states[LAMP_ON - 1 + k].current).log2();
    let before = f64::from(states[LAMP_ON - 1].metered_log2);
    let after = f64::from(states[LAMP_ON + LAMP_OFF - 1].metered_log2);
    let a0 = f64::from(states[LAMP_ON - 1].adapted_log2);
    println!(
        "{name}: metered EV with the lamp on {before:.3}, off {after:.3} (cone floor {:.3}, max's meter {:.3}); adapted at the switch {a0:.3}",
        eye.floor_log2(),
        f64::from(eye.key).log2() - f64::from(eye.max).log2()
    );
    let meters: Vec<String> = (0..30)
        .map(|k| format!("{:.3}", states[LAMP_ON + k].metered_log2))
        .collect();
    println!(
        "{name}: metered EV over the first 30 frames off: {}",
        meters.join(" ")
    );
    let closed = |k: usize| eye.exposure(eye.step(a0, after, k as f64 / 60.0)).log2();
    // (a): the f64 model stepped over the measured meter of every frame.
    let mut a = a0;
    let mut stepped = vec![eye.exposure(a).log2()];
    for k in 0..LAMP_OFF {
        a = eye.step(a, f64::from(states[LAMP_ON + k].metered_log2), 1.0 / 60.0);
        stepped.push(eye.exposure(a).log2());
    }
    for t in [0.0, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 12.0, 15.0] {
        let k = ((t * 60.0) as usize).min(LAMP_OFF);
        println!(
            "{name}: t {t:>5.2} s: log2 e {:.3}, closed form {:.3}, stepped model {:.3}",
            used(k),
            closed(k),
            stepped[k]
        );
    }
    for k in 0..LAMP_OFF {
        assert!(
            used(k + 1) >= used(k) - 0.01,
            "{name}: frame {k} after the switch: {} then {}",
            used(k),
            used(k + 1)
        );
    }
    let settled = eye.exposure(after).log2();
    println!(
        "{name}: settled log2 e {settled:.3}; at 15 s {:.3}",
        used(LAMP_OFF)
    );
    assert!(
        (used(LAMP_OFF) - settled).abs() <= 0.25,
        "{name}: {} {settled}",
        used(LAMP_OFF)
    );
    let worst = |f: &dyn Fn(usize) -> f64| {
        (60..=LAMP_OFF)
            .map(|k| (used(k) - f(k)).abs())
            .fold(0.0, f64::max)
    };
    let worst_stepped = worst(&|k| stepped[k]);
    let worst_closed = worst(&closed);
    println!(
        "{name}: from 1 s on, largest |log2 e - stepped model| {worst_stepped:.4} EV, |log2 e - closed form| {worst_closed:.4} EV"
    );
    assert!(
        worst_stepped <= 0.1,
        "{name}: stepped model {worst_stepped}"
    );
    if closed_form {
        assert!(worst_closed <= 0.1, "{name}: closed form {worst_closed}");
    } else {
        println!("reading: {name} closed-form distance from 1 s on {worst_closed:.4} EV");
    }
}

/// On the level at its spawn pose, which sees no vein: the dark meter sits far under the clamp,
/// so the closed-form clause is a reading here and the stepped model is asserted.
#[test]
fn the_caves_curve_after_the_lamp_goes_off() {
    let Some(level) = level() else {
        return;
    };
    let cam = camera(level.spawn, 0.0, 0.0);
    let states = lamp_off_states(&level, &cam, Exposure::Auto(AutoExposure::eye()));
    check_curve("spawn", &states, false);
}

/// From a pose facing a vein (VEIN_POSE), whose lamp-off meter settles above `max`'s clamp:
/// both the stepped model and the closed form hold from 1 s on.
#[test]
fn the_curve_facing_a_vein_follows_the_closed_form() {
    let Some(level) = level() else {
        return;
    };
    let (p, yaw, pitch) = VEIN_POSE;
    let states = lamp_off_states(
        &level,
        &camera(p, yaw, pitch),
        Exposure::Auto(AutoExposure::eye()),
    );
    check_curve("vein", &states, true);
}
