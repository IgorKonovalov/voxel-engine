//! FSR 2.2.1 ported to WGSL (ADR-0034), on the software adapter (lavapipe), through the rigs
//! of the upscaler tests (`support::rigs`).

mod support;

use support::rigs::*;
use support::*;
use vxe_render::{Frame, GpuWorld, LightParams, UpscalerKind, View, readback, wgpu};

/// FSR 2 on the stripe wall at arm's length, a still camera at scale 1.5 for 32 frames,
/// recovers sub-pixel detail a blit cannot: its gradient energy is above the bilinear blit's
/// and its edge spread below it, both read in this run.
#[test]
fn fsr2_upscales_the_stripe_wall_sharper_than_a_bilinear_blit() {
    let Some(rig) = stripe_rig() else {
        return;
    };
    let shot = Shot {
        dist: NEAR,
        kind: fsr2_kind(),
        pan: 0,
        phase: 0,
    };
    let [b, _] = stripe_sharpness(&rig, Image::B, shot);
    let [f, _] = stripe_sharpness(&rig, Image::U15, shot);
    println!("stripes at {NEAR} voxels: B {b}; FSR 2 at 1.5 {f}");
    assert!(f.ge > b.ge, "FSR GE {:.5} not above B's {:.5}", f.ge, b.ge);
    assert!(f.es < b.es, "FSR ES {:.3} not below B's {:.3}", f.es, b.es);
}

/// A frame of FSR 2 at scale 1.5 on the level, its rig and camera.
fn fsr_level_frame() -> Option<(
    wgpu::Device,
    wgpu::Queue,
    GpuWorld,
    Frame,
    vxe_render::Camera,
)> {
    let (world, camera) = level();
    let (device, queue, gpu) = level_rig(&world, &camera)?;
    let mut frame = frame_of(&device, INTERNAL, OUTPUT, false);
    frame.set_upscaler_kind(&device, fsr2_kind());
    Some((device, queue, gpu, frame, camera))
}

/// The input pass's motion vectors: 0 at every pixel (misses included) on a still camera; after
/// a sideways step of one voxel, at every hit pixel the analytic reprojection of the hit
/// through the previous view less this frame's jitter and the pixel's centre, within 1e-3
/// internal pixels.
#[test]
fn motion_vectors_match_the_analytic_reprojection() {
    let Some((device, queue, gpu, mut frame, camera)) = fsr_level_frame() else {
        return;
    };
    let light = LightParams::headlamp();
    for _ in 0..3 {
        frame.step(&device, &queue, &gpu, &camera, &light);
    }
    let fsr = frame.fsr2().expect("FSR mode");
    let still = read_f32_texels(&device, &queue, fsr.motion());
    let moving = still.iter().filter(|&&m| m != 0.0).count();
    assert_eq!(
        moving, 0,
        "{moving} motion components not 0 on a still camera"
    );

    let rect = [0, 0, INTERNAL[0], INTERNAL[1]];
    let prev = View::of_camera(&camera, rect);
    let mut stepped = camera;
    let (_, right, _) = camera.basis();
    stepped.position += right;
    frame.step(&device, &queue, &gpu, &stepped, &light);
    let jitter = frame.params(&gpu, &stepped, &light).views[0].jitter;
    let fsr = frame.fsr2().expect("FSR mode");
    let motion = read_f32_texels(&device, &queue, fsr.motion());
    let pos = readback::read_rgba32f(&device, &queue, frame.hit_positions());
    let (mut hits, mut worst) = (0usize, 0.0f64);
    for (i, p) in pos.iter().enumerate() {
        if p[3] >= 1e30 {
            continue;
        }
        let (x, y) = (i as u32 % INTERNAL[0], i as u32 / INTERNAL[0]);
        let Some(q) = project(&prev, [p[0], p[1], p[2]]) else {
            continue;
        };
        hits += 1;
        for k in 0..2 {
            let centre = f64::from([x, y][k]) + 0.5;
            let want = q[k] - f64::from(jitter[k]) - centre;
            let got = f64::from(motion[2 * i + k]) * f64::from(INTERNAL[k]);
            worst = worst.max((got - want).abs());
        }
    }
    println!("{hits} hit pixels, largest motion error {worst:.2e} internal pixels");
    assert!(
        hits > (INTERNAL[0] * INTERNAL[1] / 4) as usize,
        "{hits} hits"
    );
    assert!(worst <= 1e-3, "motion off by {worst} internal pixels");
}

/// `--upscaler off` leaves Plan 0024's frame alone: a frame switched to FSR 2 and back to off
/// renders the primary and headlamp goldens byte for byte the same as a frame never switched
/// (the FSR path is not dispatched), and within the goldens' tolerance of the blessed files.
#[test]
fn off_after_fsr2_renders_the_goldens_byte_for_byte() {
    let world = scene_world();
    world
        .apply(&scene_carve(&world))
        .expect("the carve applies");
    let full = world.stats().full_bricks as u64;
    let Some((device, queue)) = software_device((full + 4096) * 512) else {
        return;
    };
    let mut gpu = GpuWorld::new(&device, world.extent());
    gpu.sync_now(&world, &device, &queue)
        .expect("mirror fits the adapter");
    let mut plain = Frame::new(&device, WIDTH, HEIGHT);
    plain.set_resolution(&device, vxe_render::LightingResolution::Full);
    let mut frame = Frame::new(&device, WIDTH, HEIGHT);
    frame.set_resolution(&device, vxe_render::LightingResolution::Full);
    frame.set_upscaler_kind(&device, fsr2_kind());
    assert!(frame.fsr2().is_some() && frame.upscaling());
    frame.render(
        &device,
        &queue,
        &gpu,
        &scene_camera(),
        &LightParams::at_eye(),
    );
    frame.set_upscaler_kind(&device, UpscalerKind::Off);
    assert!(frame.fsr2().is_none() && !frame.upscaling());
    for (name, light) in [
        ("primary.png", LightParams::at_eye()),
        ("headlamp.png", LightParams::headlamp()),
    ] {
        frame.render(&device, &queue, &gpu, &scene_camera(), &light);
        plain.render(&device, &queue, &gpu, &scene_camera(), &light);
        let img = readback::read_rgba8(&device, &queue, frame.target());
        let want = readback::read_rgba8(&device, &queue, plain.target());
        assert!(img == want, "{name} differs after FSR 2 and back");
        assert_golden(name, &img);
    }
}
