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

/// `world` mirrored over its extent on the software adapter; `None` without the adapter.
fn mirrored(world: &vxe_world::World) -> Option<(wgpu::Device, wgpu::Queue, GpuWorld)> {
    let full = world.stats().full_bricks as u64;
    let (device, queue) = software_device(((full + 4096) * 512).max(64 << 20))?;
    let mut gpu = GpuWorld::new(&device, world.extent());
    gpu.sync_now(world, &device, &queue)
        .expect("mirror fits the adapter");
    Some((device, queue, gpu))
}

/// The tone move is lossless: on the primary, headlamp, translucent and LOD golden poses at
/// scale 1, jitter off, the composite's
/// linear target put through the tone pass (the upscaler bypassed) is the composite's direct
/// rgba8 image byte for byte.
#[test]
fn the_tone_pass_reproduces_the_composites_bytes_on_every_golden_pose() {
    let carved = support::carved_world();
    let poses: [(
        &str,
        vxe_world::World,
        vxe_render::Camera,
        LightParams,
        vxe_render::LodParams,
    ); 4] = [
        (
            "primary",
            carved,
            scene_camera(),
            LightParams::at_eye(),
            Default::default(),
        ),
        (
            "headlamp",
            support::carved_world(),
            scene_camera(),
            LightParams::headlamp(),
            Default::default(),
        ),
        (
            "translucent",
            translucent_world(),
            translucent_camera(),
            LightParams::headlamp(),
            Default::default(),
        ),
        (
            "lod",
            lod_scene(),
            scene_camera(),
            LightParams::at_eye(),
            lod_forced_1(),
        ),
    ];
    for (name, world, camera, light, lod) in poses {
        let Some((device, queue, gpu)) = mirrored(&world) else {
            return;
        };
        let mut plain = Frame::new(&device, WIDTH, HEIGHT);
        let mut toned = Frame::new(&device, WIDTH, HEIGHT);
        toned.set_upscaler_kind(&device, fsr2_kind());
        toned.fsr2_mut().expect("FSR mode").set_bypass(true);
        for f in [&mut plain, &mut toned] {
            f.set_resolution(&device, vxe_render::LightingResolution::Full);
            f.set_lod(lod);
            f.render(&device, &queue, &gpu, &camera, &light);
        }
        let want = readback::read_rgba8(&device, &queue, plain.target());
        let got = readback::read_rgba8(&device, &queue, toned.target());
        let differing = want.iter().zip(&got).filter(|(a, b)| a != b).count();
        let lit = want.chunks(4).filter(|p| p[..3] != [0, 0, 0]).count();
        println!("{name}: {differing} bytes differ, {lit} lit pixels");
        assert!(lit > 0, "{name} renders something");
        assert_eq!(
            differing, 0,
            "{name}: the tone pass differs from the composite"
        );
    }
}

/// The reactive mask, pixel by pixel, on the level at scale 1.5: on the frame of a radius-6
/// carve every hit in the carve's box dilated by one voxel reads 1 and every other opaque hit
/// 0; switching the lamp on over a settled still camera, every pixel the lamp lights past the
/// light-change threshold reads `reactive_light` and the rest 0; behind the glass slab every
/// pixel reads `reactive_translucent (1 - transmittance)` within 1/255.
#[test]
fn the_reactive_mask_marks_edits_light_changes_and_translucency() {
    let (world, camera) = level();
    let Some((device, queue, mut gpu)) = level_rig(&world, &camera) else {
        return;
    };
    let light = LightParams::headlamp();
    let settings = vxe_render::Fsr2Settings {
        reactive_light: 0.6,
        reactive_translucent: 0.8,
        ..Default::default()
    };
    let mut frame = frame_of(&device, INTERNAL, OUTPUT, false);
    frame.set_upscaler_kind(&device, UpscalerKind::Fsr2(settings));
    let byte = |v: f32| (v * 255.0).round() as u8;
    let reactive = |frame: &Frame| -> Vec<u8> {
        readback::read_rgba8(&device, &queue, frame.fsr2().unwrap().reactive())
            .chunks(4)
            .map(|p| p[0])
            .collect()
    };
    // The lamp: off for the settle, on for one frame.
    frame.set_lighting(vxe_render::Lighting {
        lamp: false,
        ..Default::default()
    });
    for _ in 0..SETTLE {
        frame.step(&device, &queue, &gpu, &camera, &light);
    }
    assert!(
        reactive(&frame).iter().all(|&r| r == 0),
        "a settled still frame is not reactive"
    );
    frame.set_lighting(vxe_render::Lighting::default());
    frame.step(&device, &queue, &gpu, &camera, &light);
    let params = frame.params(&gpu, &camera, &light);
    let pos = readback::read_rgba32f(&device, &queue, frame.hit_positions());
    let info = readback::read_rgba16u(&device, &queue, frame.hit_records());
    let r = reactive(&frame);
    let (mut lit, mut dark, mut wrong) = (0usize, 0usize, 0usize);
    for i in 0..r.len() {
        if pos[i][3] >= 1e30 {
            continue;
        }
        let p = [pos[i][0], pos[i][1], pos[i][2]];
        let term = lamp_term(&params, p, octa16_normal(info[i][0], info[i][1]));
        if term > 0.0105 {
            lit += 1;
            wrong += usize::from(r[i] != byte(0.6));
        } else if term < 0.0095 {
            dark += 1;
            wrong += usize::from(r[i] != 0);
        }
    }
    println!("lamp on: {lit} lit hits, {dark} unlit, {wrong} off their term");
    assert!(lit > 1000 && wrong == 0, "{lit} lit, {wrong} wrong");

    // The carve, the lamp steady.
    for _ in 0..SETTLE {
        frame.step(&device, &queue, &gpu, &camera, &light);
    }
    let (aabb, t) = carve_in_view(&world, &camera);
    gpu.sync_now(&world, &device, &queue).unwrap();
    frame.invalidate(aabb.0.0, aabb.1.0);
    frame.step(&device, &queue, &gpu, &camera, &light);
    let pos = readback::read_rgba32f(&device, &queue, frame.hit_positions());
    let r = reactive(&frame);
    let (mut inside, mut outside, mut wrong) = (0usize, 0usize, 0usize);
    for i in 0..r.len() {
        if pos[i][3] >= 1e30 {
            continue;
        }
        if in_dilated([pos[i][0], pos[i][1], pos[i][2]], aabb) {
            inside += 1;
            wrong += usize::from(r[i] != 255);
        } else {
            outside += 1;
            wrong += usize::from(r[i] != 0);
        }
    }
    println!(
        "carve at t = {t:.1}: {inside} hits in the dilated box, {outside} outside, {wrong} wrong"
    );
    assert!(
        inside > 0 && outside > 0 && wrong == 0,
        "{inside} in, {wrong} wrong"
    );

    // The glass slab, in its own scene.
    let world = translucent_world();
    let Some((device, queue, gpu)) = mirrored(&world) else {
        return;
    };
    let mut frame = frame_of(&device, INTERNAL, OUTPUT, false);
    frame.set_upscaler_kind(&device, UpscalerKind::Fsr2(settings));
    frame.step(&device, &queue, &gpu, &translucent_camera(), &light);
    let r: Vec<u8> = readback::read_rgba8(&device, &queue, frame.fsr2().unwrap().reactive())
        .chunks(4)
        .map(|p| p[0])
        .collect();
    let info = readback::read_rgba16u(&device, &queue, frame.hit_records());
    let pos = readback::read_rgba32f(&device, &queue, frame.hit_positions());
    let layer = readback::read_rgba16f(&device, &queue, frame.layers());
    let (mut behind, mut worst) = (0usize, 0i32);
    for i in 0..r.len() {
        if info[i][3] & 4 == 0 || pos[i][3] >= 1e30 {
            continue;
        }
        behind += 1;
        let want = (0.8 * (1.0 - layer[i][3])).min(1.0);
        worst = worst.max((i32::from(r[i]) - i32::from(byte(want))).abs());
    }
    println!("glass: {behind} pixels behind it, largest error {worst}/255");
    assert!(
        behind > 1000 && worst <= 1,
        "{behind} behind, worst {worst}/255"
    );
}

/// The exposure seam: doubling `params.exposure` changes FSR's exposure texel on that frame,
/// and once the history settles the tone pass's output on the stripe wall at scale 1 is Plan
/// 0024's frame's at the new exposure within 2/255 mean.
#[test]
fn doubling_the_exposure_reaches_fsr_the_same_frame() {
    let Some(rig) = stripe_rig() else {
        return;
    };
    let (device, queue) = (&rig.device, &rig.queue);
    let camera = stripe_camera(NEAR, 0);
    let light = LightParams::headlamp();
    let e = stripe_exposure(NEAR);
    let lighting = |exposure| vxe_render::Lighting {
        exposure,
        ..vxe_render::Lighting::default()
    };
    let mut fsr = frame_of(device, OUTPUT, OUTPUT, false);
    fsr.set_upscaler_kind(device, fsr2_kind());
    fsr.set_lighting(lighting(e));
    for _ in 0..SETTLE {
        fsr.step(device, queue, &rig.gpu, &camera, &light);
    }
    let texel = |f: &Frame| read_f32_texels(device, queue, f.fsr2().unwrap().exposure())[0];
    assert_eq!(texel(&fsr), e);
    fsr.set_lighting(lighting(2.0 * e));
    fsr.step(device, queue, &rig.gpu, &camera, &light);
    assert_eq!(texel(&fsr), 2.0 * e, "the texel follows the same frame");
    for _ in 0..SETTLE {
        fsr.step(device, queue, &rig.gpu, &camera, &light);
    }
    let mut native = frame_of(device, OUTPUT, OUTPUT, false);
    native.set_lighting(lighting(2.0 * e));
    for _ in 0..SETTLE {
        native.step(device, queue, &rig.gpu, &camera, &light);
    }
    let a = readback::read_rgba8(device, queue, fsr.target());
    let b = readback::read_rgba8(device, queue, native.target());
    let m = mad(&a, &b);
    let mean = m.iter().sum::<f64>() / 3.0;
    println!("FSR at 2x exposure against Plan 0024's frame: mean {mean:.3}/255 ({m:.3?})");
    assert!(mean <= 2.0, "mean {mean:.3}/255");
}
