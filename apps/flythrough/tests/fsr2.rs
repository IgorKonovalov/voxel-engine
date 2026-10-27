//! FSR 2.2.1 ported to WGSL (ADR-0034), on the software adapter (lavapipe), through the rigs
//! of the upscaler tests (`support::rigs`).

mod support;

use support::rigs::*;
use support::*;
use vxe_render::{Frame, GpuWorld, LightParams, LookParams, UpscalerKind, View, readback, wgpu};

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
/// 0; switching the lamp on over a settled still camera, with indirect light on and off, every
/// pixel the lamp lights past the light-change threshold reads `reactive_light` and the rest
/// 0; behind the glass slab every pixel reads `reactive_translucent (1 - transmittance)`
/// within 1/255.
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
    let byte = |v: f32| (v * 255.0).round() as u8;
    let reactive = |frame: &Frame| -> Vec<u8> {
        readback::read_rgba8(&device, &queue, frame.fsr2().unwrap().reactive())
            .chunks(4)
            .map(|p| p[0])
            .collect()
    };
    // The lamp: off for the settle, on for one frame; with the probe volume's indirect light
    // on (the fly-through's default) and off. The mask follows the lamp's own term either way.
    let lamp_switched_on = |indirect: bool| -> Frame {
        let mut frame = frame_of(&device, INTERNAL, OUTPUT, false);
        frame.set_upscaler_kind(&device, UpscalerKind::Fsr2(settings));
        let lighting = vxe_render::Lighting {
            indirect,
            ..Default::default()
        };
        frame.set_lighting(vxe_render::Lighting {
            lamp: false,
            ..lighting
        });
        for _ in 0..SETTLE {
            frame.step(&device, &queue, &gpu, &camera, &light);
        }
        assert!(
            reactive(&frame).iter().all(|&r| r == 0),
            "a settled still frame is not reactive (indirect {indirect})"
        );
        frame.set_lighting(lighting);
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
        println!(
            "lamp on, indirect {indirect}: {lit} lit hits, {dark} unlit, {wrong} off their term"
        );
        assert!(lit > 1000 && wrong == 0, "{lit} lit, {wrong} wrong");
        frame
    };
    lamp_switched_on(true);
    let mut frame = lamp_switched_on(false);

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

/// The internal pixel whose sample lies nearest output pixel `(x, y)`'s centre in a frame
/// whose rays move by `j` (`View::jitter`, internal pixels).
fn nearest_sample(x: u32, y: u32, j: [f32; 2]) -> usize {
    let c = [x, y];
    let n = [0, 1].map(|k| {
        let centre = (c[k] as f32 + 0.5) * INTERNAL[k] as f32 / OUTPUT[k] as f32;
        ((centre - j[k]).floor() as i32).clamp(0, INTERNAL[k] as i32 - 1) as usize
    });
    n[1] * INTERNAL[0] as usize + n[0]
}

/// The mean and the count over 8/255 of the largest channel difference between `a` and `b`
/// over the output pixels `at`.
fn difference_over(a: &[u8], b: &[u8], at: &[usize]) -> (f64, usize) {
    let (mut sum, mut over) = (0u64, 0usize);
    for &o in at {
        let d = (0..3)
            .map(|k| a[4 * o + k].abs_diff(b[4 * o + k]))
            .max()
            .unwrap();
        sum += u64::from(d);
        over += usize::from(d > 8);
    }
    (sum as f64 / at.len().max(1) as f64, over)
}

/// No ghost after a carve: on the level at scale 1.5, after 32 still frames, a radius-6 carve
/// where the view axis meets rock; on the first frame after it, over the output pixels whose
/// nearest internal hit lies in the carve's box dilated by one voxel, FSR's image differs from
/// the same pipeline's frame with FSR's history reset (the same pose, jitter phase and
/// lighting history) by at most 2/255 mean. The count over 8/255 and the fourth frame's two
/// figures are readings.
#[test]
fn a_carve_leaves_no_ghost_under_fsr2() {
    let (world, camera) = level();
    let Some((device, queue, mut gpu)) = level_rig(&world, &camera) else {
        return;
    };
    let light = LightParams::headlamp();
    let mut kept = frame_of(&device, INTERNAL, OUTPUT, false);
    let mut reset = frame_of(&device, INTERNAL, OUTPUT, false);
    for f in [&mut kept, &mut reset] {
        f.set_upscaler_kind(&device, fsr2_kind());
    }
    for _ in 0..SETTLE {
        for f in [&mut kept, &mut reset] {
            f.step(&device, &queue, &gpu, &camera, &light);
        }
    }
    let (aabb, t) = carve_in_view(&world, &camera);
    gpu.sync_now(&world, &device, &queue).unwrap();
    for f in [&mut kept, &mut reset] {
        f.invalidate(aabb.0.0, aabb.1.0);
    }
    for frame in 1..=4 {
        reset.reset_upscaler_history();
        for f in [&mut kept, &mut reset] {
            f.step(&device, &queue, &gpu, &camera, &light);
        }
        let j = kept.params(&gpu, &camera, &light).views[0].jitter;
        let pos = readback::read_rgba32f(&device, &queue, kept.hit_positions());
        let mut at = Vec::new();
        for y in 0..OUTPUT[1] {
            for x in 0..OUTPUT[0] {
                let p = pos[nearest_sample(x, y, j)];
                if p[3] < 1e30 && in_dilated([p[0], p[1], p[2]], aabb) {
                    at.push((y * OUTPUT[0] + x) as usize);
                }
            }
        }
        let a = readback::read_rgba8(&device, &queue, kept.target());
        let b = readback::read_rgba8(&device, &queue, reset.target());
        let (mean, over) = difference_over(&a, &b, &at);
        println!(
            "carve at t = {t:.1}, frame {frame} after: {} output pixels in the dilated box, mean difference from the reset frame {mean:.3}/255, {over} over 8/255",
            at.len()
        );
        if frame == 1 {
            assert!(!at.is_empty(), "no output pixel hits inside the box");
            assert!(mean <= 2.0, "mean {mean:.3}/255 from the reset frame");
        }
    }
}

/// The cavity at scale 1.5 on the software adapter with FSR 2, lighting at full resolution.
fn cavity_rig() -> Option<LookRig> {
    let world = cavity_world();
    let (device, queue, gpu) = mirrored(&world)?;
    let mut frame = Frame::with_sizes(&device, INTERNAL, OUTPUT);
    frame.set_upscaler_kind(&device, fsr2_kind());
    Some(LookRig {
        device,
        queue,
        gpu,
        frame,
    })
}

/// History survives a sideways step: the cavity at scale 1.5, still for 4 frames, then one
/// voxel sideways; on the next frame the accumulate pass's history weight (`1 - fAlpha`) is
/// above 0 in at least 95 % of the output pixels whose hit surface did not change (the nearest
/// sample's hit reprojected into the frame before lands on a hit of the same material, a
/// normal within 0.99 and 1.5 voxels).
#[test]
fn a_sideways_step_keeps_fsr2s_history() {
    let Some(mut rig) = cavity_rig() else {
        return;
    };
    rig.frame
        .fsr2_mut()
        .expect("FSR mode")
        .set_debug(&rig.device, true);
    let light = LightParams::headlamp();
    let mut camera = cavity_camera();
    for _ in 0..4 {
        rig.frame
            .step(&rig.device, &rig.queue, &rig.gpu, &camera, &light);
    }
    let read_hits = |rig: &LookRig, camera: &vxe_render::Camera| {
        (
            readback::read_rgba32f(&rig.device, &rig.queue, rig.frame.hit_positions()),
            readback::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records()),
            rig.frame.params(&rig.gpu, camera, &light).views[0],
        )
    };
    let (pos0, info0, view0) = read_hits(&rig, &camera);
    let (_, right, _) = camera.basis();
    camera.position += right;
    rig.frame
        .step(&rig.device, &rig.queue, &rig.gpu, &camera, &light);
    let (pos1, info1, view1) = read_hits(&rig, &camera);
    let weight = read_f32_texels(
        &rig.device,
        &rig.queue,
        rig.frame.fsr2().unwrap().history_weight(),
    );
    let (mut same, mut kept) = (0usize, 0usize);
    for y in 0..OUTPUT[1] {
        for x in 0..OUTPUT[0] {
            let i = nearest_sample(x, y, view1.jitter);
            if info1[i][3] & 1 == 0 {
                continue;
            }
            let p = [pos1[i][0], pos1[i][1], pos1[i][2]];
            let Some(q) = project(&view0, p) else {
                continue;
            };
            // The previous frame's pixel whose jittered sample lies nearest.
            let k = [0, 1].map(|c| q[c] - f64::from(view0.jitter[c]));
            if k[0] < 0.0
                || k[1] < 0.0
                || k[0] >= f64::from(INTERNAL[0])
                || k[1] >= f64::from(INTERNAL[1])
            {
                continue;
            }
            let k = k[1] as usize * INTERNAL[0] as usize + k[0] as usize;
            let (n0, n1) = (
                octa16_normal(info0[k][0], info0[k][1]),
                octa16_normal(info1[i][0], info1[i][1]),
            );
            let dist = (0..3)
                .map(|c| (pos0[k][c] - p[c]).powi(2))
                .sum::<f32>()
                .sqrt();
            let unchanged = info0[k][3] & 1 == 1
                && info0[k][2] == info1[i][2]
                && (0..3).map(|c| n0[c] * n1[c]).sum::<f32>() >= 0.99
                && dist <= 1.5;
            if unchanged {
                same += 1;
                kept += usize::from(weight[(y * OUTPUT[0] + x) as usize] > 0.0);
            }
        }
    }
    let share = kept as f64 / same.max(1) as f64;
    println!(
        "sideways step: {kept} of {same} unchanged output pixels keep history ({:.2} %)",
        share * 100.0
    );
    assert!(same > 0 && share >= 0.95, "{:.2} %", share * 100.0);
}

/// Still means no disocclusion: on the level at scale 1.5, a still camera, after 2 frames
/// the depth clip is 0 at every hit pixel not within one internal pixel of a silhouette (a
/// neighbour whose view depth differs by more than 5 %, or a miss).
#[test]
fn a_still_camera_has_no_disocclusion() {
    let Some((device, queue, gpu, mut frame, camera)) = fsr_level_frame() else {
        return;
    };
    let light = LightParams::headlamp();
    for _ in 0..2 {
        frame.step(&device, &queue, &gpu, &camera, &light);
    }
    let fsr = frame.fsr2().expect("FSR mode");
    let clip = readback::read_rgba16f(&device, &queue, fsr.prepared());
    let depth = read_f32_texels(&device, &queue, fsr.depth());
    let [w, h] = INTERNAL.map(|n| n as i32);
    let at = |x: i32, y: i32| depth[(y * w + x) as usize];
    let (mut checked, mut near_edge, mut clipped, mut worst) = (0usize, 0usize, 0usize, 0.0f32);
    for y in 0..h {
        for x in 0..w {
            let d = at(x, y);
            if d == 0.0 {
                continue;
            }
            let edge = (-1..=1).any(|dy| {
                (-1..=1).any(|dx| {
                    let (nx, ny) = ((x + dx).clamp(0, w - 1), (y + dy).clamp(0, h - 1));
                    let n = at(nx, ny);
                    n == 0.0 || (n / d - 1.0).abs() > 0.05
                })
            });
            if edge {
                near_edge += 1;
                continue;
            }
            checked += 1;
            let c = clip[(y * w + x) as usize][3];
            clipped += usize::from(c != 0.0);
            worst = worst.max(c);
        }
    }
    println!(
        "still camera: {checked} hit pixels away from silhouettes, {clipped} with a depth clip (largest {worst}); {near_edge} beside one"
    );
    assert!(
        checked > 1000 && clipped == 0,
        "{clipped} clipped, worst {worst}"
    );
}

/// Locks form on thin features: on the stripe wall at 88 voxels (1.753 output pixels a voxel),
/// after 32 still frames at scale 1.5, the share of the stripe-edge output pixels (the 3 x 3
/// spans 16/255 in a channel) in the middle half holding a lock is above zero; the share on a
/// flat patch of one material is a reading.
#[test]
fn locks_form_on_the_stripe_edges() {
    let Some(rig) = stripe_rig() else {
        return;
    };
    let (device, queue) = (&rig.device, &rig.queue);
    let light = LightParams::headlamp();
    let mut f = frame_of(device, INTERNAL, OUTPUT, false);
    f.set_upscaler_kind(device, fsr2_kind());
    f.set_lighting(vxe_render::Lighting {
        exposure: stripe_exposure(FAR),
        ..Default::default()
    });
    let camera = stripe_camera(FAR, 0);
    for _ in 0..SETTLE {
        f.step(device, queue, &rig.gpu, &camera, &light);
    }
    let img = readback::read_rgba8(device, queue, f.target());
    let fsr = f.fsr2().expect("FSR mode");
    let lock = read_f32_texels(device, queue, fsr.lock_status(f.current_copy()));
    let [w, h] = OUTPUT.map(|n| n as usize);
    let (mut edges, mut locked) = (0usize, 0usize);
    for y in h / 4..3 * h / 4 {
        for x in w / 4..3 * w / 4 {
            let spans = (0..3).any(|k| {
                let (mut lo, mut hi) = (255u8, 0u8);
                for yy in y - 1..=y + 1 {
                    for xx in x - 1..=x + 1 {
                        let v = img[(yy * w + xx) * 4 + k];
                        lo = lo.min(v);
                        hi = hi.max(v);
                    }
                }
                hi - lo >= 16
            });
            if spans {
                edges += 1;
                locked += usize::from(lock[2 * (y * w + x)] > 0.0);
            }
        }
    }
    let share = locked as f64 / edges.max(1) as f64;
    // The flat patch: the same distance, one material.
    let flat = vxe_world::World::generate(
        vxe_world::ChunkExtent::new([0, 0, 0], [3, 3, 3]),
        &vxe_world::UniformSource(vxe_world::AIR),
        &{
            let mut t = vxe_world::MaterialTable::new();
            t.push(vxe_world::Material {
                albedo: [0.8; 3],
                ..vxe_world::Material::named("flat")
            });
            t
        },
    );
    flat.apply(&vxe_world::EditOp::box_(
        vxe_world::Fixed3([WALL_X * vxe_world::FIXED_ONE, 0, 0]),
        vxe_world::Fixed3([
            (WALL_X + 2) * vxe_world::FIXED_ONE,
            192 * vxe_world::FIXED_ONE,
            192 * vxe_world::FIXED_ONE,
        ]),
        vxe_world::Mode::Fill(1),
    ))
    .expect("the wall is resident");
    let Some((device, queue, gpu)) = mirrored(&flat) else {
        return;
    };
    let mut g = frame_of(&device, INTERNAL, OUTPUT, false);
    g.set_upscaler_kind(&device, fsr2_kind());
    g.set_lighting(vxe_render::Lighting {
        exposure: stripe_exposure(FAR),
        ..Default::default()
    });
    for _ in 0..SETTLE {
        g.step(&device, &queue, &gpu, &camera, &light);
    }
    let lock = read_f32_texels(
        &device,
        &queue,
        g.fsr2().unwrap().lock_status(g.current_copy()),
    );
    let middle: Vec<usize> = (h / 4..3 * h / 4)
        .flat_map(|y| (w / 4..3 * w / 4).map(move |x| y * w + x))
        .collect();
    let flat_locked = middle.iter().filter(|&&i| lock[2 * i] > 0.0).count();
    println!(
        "locks at {FAR} voxels: {locked} of {edges} stripe-edge pixels ({:.2} %); flat patch {flat_locked} of {} ({:.2} %)",
        share * 100.0,
        middle.len(),
        flat_locked as f64 * 100.0 / middle.len() as f64
    );
    assert!(locked > 0, "no stripe-edge pixel holds a lock");
}

/// FSR 2 at `sharpness`.
fn fsr2_sharp(sharpness: f32) -> UpscalerKind {
    UpscalerKind::Fsr2(vxe_render::Fsr2Settings {
        sharpness,
        ..Default::default()
    })
}

/// A bar FSR 2's algorithm misses, printed for the owner (Phase 7 of Plan 0041): the reading,
/// the bar as written, and who judges it.
fn reading(what: &str, value: &str, bar: &str) {
    println!("reading: {what} {value}; bar: {bar}; owner judges, Phase 7");
}

/// The stripe wall's sharpness, Plan 0030 Phase 6's images beside FSR 2's, in one run: N, R,
/// B, our upscaler (sharpening 0 and 0.5) and FSR 2 (0 and 0.5) at scale 1 and 1.5, at both
/// distances, still and panning 64 frames. Asserted, at sharpening 0, still, at arm's length:
/// FSR at scale 1 has ES at most N's plus 0.53 px and GE at least 0.85 of R's. Printed as
/// `reading:` lines: scale 1.5 at arm's length (GE at least `(GE(R) + GE(B)) / 2`, ES at most
/// `(ES(R) + ES(B)) / 2`), the 88-voxel GE at both scales, the arm's-length pans at 0.37 px a
/// frame against ours, and the 88-voxel pans at 0.37 and 0.5 px a frame. Sharpening 0.5 and the
/// rim gradients are readings.
#[test]
fn fsr2_at_scale_1_keeps_stripe_edges_sharp_at_arms_length() {
    let Some(rig) = stripe_rig() else {
        return;
    };
    let own = UpscalerKind::Own(vxe_render::marcher::Upscaler::default());
    let mut failures = Vec::new();
    for dist in [NEAR, FAR] {
        let near = dist == NEAR;
        let still = |kind| Shot {
            dist,
            kind,
            pan: 0,
            phase: 0,
        };
        let read = |image, shot| stripe_sharpness(&rig, image, shot);
        println!(
            "stripes at {dist} voxels ({:.3} px per voxel):",
            px_per_voxel(dist)
        );
        let [n, _] = read(Image::N, still(own));
        let [r, _] = read(Image::R, still(own));
        let [b, _] = read(Image::B, still(own));
        println!("  N    {n}\n  R    {r}\n  B    {b}");
        let mut fsr0 = [n; 2];
        for (k, (name, image)) in [("1", Image::U1), ("1.5", Image::U15)]
            .into_iter()
            .enumerate()
        {
            let ours = read(image, still(own));
            let [f0, _] = read(image, still(fsr2_sharp(0.0)));
            let [f5, _] = read(image, still(fsr2_sharp(0.5)));
            println!("  ours {name}: sharpen 0 {}; 0.5 {}", ours[0], ours[1]);
            println!("  FSR  {name}: sharpen 0 {f0}; 0.5 {f5}");
            fsr0[k] = f0;
        }
        let [f1, f15] = fsr0;
        let (es1_bar, ge1_bar) = (n.es + 0.53, 0.85 * r.ge);
        let (ge_bar, es_bar) = ((r.ge + b.ge) / 2.0, (r.es + b.es) / 2.0);
        println!(
            "  bars: FSR 1 ES <= {es1_bar:.3}, GE >= {ge1_bar:.5}; FSR 1.5 GE >= {ge_bar:.5}, ES <= {es_bar:.3}"
        );
        if near {
            if !at_most(f1.es, es1_bar) {
                failures.push(format!("{dist}: FSR 1 ES {:.3} over {es1_bar:.3}", f1.es));
            }
            if !at_least(f1.ge, ge1_bar) {
                failures.push(format!("{dist}: FSR 1 GE {:.5} under {ge1_bar:.5}", f1.ge));
            }
            reading(
                &format!("{dist} voxels, still, FSR 1.5 ES"),
                &format!("{:.3} px", f15.es),
                &format!("at most (ES(R) + ES(B)) / 2 = {es_bar:.3} px"),
            );
        } else {
            reading(
                &format!("{dist} voxels, still, FSR 1 GE"),
                &format!("{:.5}", f1.ge),
                &format!("at least 0.85 GE(R) = {ge1_bar:.5}"),
            );
        }
        reading(
            &format!("{dist} voxels, still, FSR 1.5 GE"),
            &format!("{:.5}", f15.ge),
            &format!("at least (GE(R) + GE(B)) / 2 = {ge_bar:.5}"),
        );
        let pan = |kind| Shot {
            pan: 64,
            ..still(kind)
        };
        let speeds: &[f32] = if near { &[PAN_PX] } else { &[PAN_PX, 0.5] };
        for &px in speeds {
            for (name, image) in [("1", Image::U1), ("1.5", Image::U15)] {
                let read_at = |kind| stripe_sharpness_at(&rig, image, pan(kind), px);
                let [ours, ours5] = read_at(own);
                let [f0, _] = read_at(fsr2_sharp(0.0));
                let [f5, _] = read_at(fsr2_sharp(0.5));
                println!(
                    "  panning 64 frames at {px} px, scale {name}: ours sharpen 0 {ours}; 0.5 {ours5}"
                );
                println!(
                    "  panning 64 frames at {px} px, scale {name}: FSR  sharpen 0 {f0}; 0.5 {f5}"
                );
                let at = format!("{dist} voxels, panning at {px} px a frame, FSR {name}");
                if near {
                    reading(
                        &format!("{at} ES"),
                        &format!("{:.3} px", f0.es),
                        &format!("below ours {:.3} px", ours.es),
                    );
                }
                reading(
                    &format!("{at} GE"),
                    &format!("{:.5}", f0.ge),
                    &format!("at least ours {:.5}", ours.ge),
                );
            }
        }
        let [np, _] = read(Image::N, pan(own));
        println!("  N panning 64 frames: {np}");
    }
    for f in &failures {
        println!("over a bar: {f}");
    }
    assert!(failures.is_empty(), "{failures:?}");
}

/// FSR 2's still-frame GE and its last panning frame's GE in one run of the stripe wall at
/// `dist`, scale 1.5, sharpening 0: 32 still frames, then 64 translating at `pan_px` output
/// pixels a frame; the mean over the [`PHASES`] sub-pixel phases.
fn fsr2_pan_ge(rig: &StripeRig, dist: f32, pan_px: f32) -> (f64, f64) {
    let (device, queue) = (&rig.device, &rig.queue);
    let light = LightParams::headlamp();
    let (mut still, mut last) = (0.0, 0.0);
    for phase in 0..PHASES {
        let mut f = frame_of(device, INTERNAL, OUTPUT, true);
        f.set_upscaler_kind(device, fsr2_sharp(0.0));
        f.set_lighting(vxe_render::Lighting {
            exposure: stripe_exposure(dist),
            ..Default::default()
        });
        let mut camera = stripe_camera(dist, phase);
        let (_, right, _) = camera.basis();
        let step = right * (pan_px / px_per_voxel(dist));
        for i in 0..SETTLE + 64 {
            if i >= SETTLE {
                camera.position += step;
            }
            f.step(device, queue, &rig.gpu, &camera, &light);
            if i + 1 == SETTLE {
                let img = readback::read_rgba8(device, queue, f.target());
                still += sharpness(&img, &camera, dist).ge / f64::from(PHASES);
            }
        }
        let img = readback::read_rgba8(device, queue, f.target());
        last += sharpness(&img, &camera, dist).ge / f64::from(PHASES);
    }
    (still, last)
}

/// Alignment in motion: on the stripe wall at 88 voxels, scale 1.5, sharpening 0, a pan of one
/// output pixel a frame (each history fetch lands on a texel centre) for 64 frames after 32
/// still ones keeps the last frame's GE at least 0.85 of the same run's still GE at frame 32. A
/// motion-vector or jitter misalignment blurs the history on every frame and drops it.
#[test]
fn fsr2_keeps_its_still_sharpness_on_a_whole_pixel_pan() {
    let Some(rig) = stripe_rig() else {
        return;
    };
    let (still, last) = fsr2_pan_ge(&rig, FAR, 1.0);
    println!(
        "{FAR} voxels, scale 1.5, 1 px a frame: still GE {still:.5}, last panning GE {last:.5} ({:.3} of still)",
        last / still
    );
    assert!(
        at_least(last, 0.85 * still),
        "panning GE {last:.5} under 0.85 of still {still:.5}"
    );
}

/// The 2 x 2 downsample of `big` (twice `OUTPUT`) averaged in linear light: each channel's
/// bytes decoded by 2.2, the four averaged, encoded again.
fn downsample_2x2_linear(big: &[u8]) -> Vec<u8> {
    let [w, h] = OUTPUT.map(|n| n as usize);
    (0..w * h * 4)
        .map(|i| {
            let (px, k) = (i / 4, i % 4);
            if k == 3 {
                return 255;
            }
            let (x, y) = (px % w, px / w);
            let at = |xx: usize, yy: usize| {
                (f64::from(big[(yy * 2 * w + xx) * 4 + k]) / 255.0).powf(2.2)
            };
            let m = (at(2 * x, 2 * y)
                + at(2 * x + 1, 2 * y)
                + at(2 * x, 2 * y + 1)
                + at(2 * x + 1, 2 * y + 1))
                / 4.0;
            (m.powf(1.0 / 2.2) * 255.0).round() as u8
        })
        .collect()
}

/// A still camera on the level (Plan 0024's defaults, 32 frames, scale 1.5), against the 2 x 2
/// supersampled render averaged in linear light: our upscaler's image is nearer it than the
/// bilinear blit is, in mean absolute difference per channel. Printed: FSR 2's distance as a
/// `reading:` line (the bar: below the blit's), and every image's distance from the render
/// averaged in display bytes, with its mean signed difference.
#[test]
fn a_still_camera_under_our_upscaler_is_nearer_the_linear_supersampled_render_than_a_blit() {
    let (world, camera) = level();
    let Some((device, queue, gpu)) = level_rig(&world, &camera) else {
        return;
    };
    let light = LightParams::headlamp();
    let big = OUTPUT.map(|n| 2 * n);
    let mut fsr = frame_of(&device, INTERNAL, OUTPUT, false);
    fsr.set_upscaler_kind(&device, fsr2_kind());
    let mut ours = frame_of(&device, INTERNAL, OUTPUT, true);
    let mut low = frame_of(&device, INTERNAL, INTERNAL, false);
    let mut ssaa = frame_of(&device, big, big, false);
    for _ in 0..SETTLE {
        for f in [&mut fsr, &mut ours, &mut low, &mut ssaa] {
            f.step(&device, &queue, &gpu, &camera, &light);
        }
    }
    let big_img = readback::read_rgba8(&device, &queue, ssaa.target());
    let truth = downsample_2x2_linear(&big_img);
    let display_truth = downsample_2x2(&big_img, OUTPUT);
    let mean = |m: [f64; 3]| m.iter().sum::<f64>() / 3.0;
    let images = [
        ("FSR", readback::read_rgba8(&device, &queue, fsr.target())),
        ("ours", readback::read_rgba8(&device, &queue, ours.target())),
        (
            "bilinear",
            blit_texture(&device, &queue, low.target(), OUTPUT),
        ),
    ];
    let [f, o, b] = [0, 1, 2].map(|i| mean(mad(&images[i].1, &truth)));
    println!(
        "level, {SETTLE} frames, from 2 x 2 supersampled in linear light: FSR {f:.3}, ours {o:.3}, bilinear {b:.3}"
    );
    for (name, img) in &images {
        let signed: f64 = img
            .chunks(4)
            .zip(display_truth.chunks(4))
            .map(|(p, q)| {
                (0..3)
                    .map(|k| f64::from(p[k]) - f64::from(q[k]))
                    .sum::<f64>()
                    / 3.0
            })
            .sum::<f64>()
            / (img.len() / 4) as f64;
        println!(
            "  {name}: from it averaged in display bytes {:.3}, mean signed difference {signed:+.3}",
            mean(mad(img, &display_truth))
        );
    }
    reading(
        "level, still, FSR 1.5 from the linear supersampled render",
        &format!("{f:.3}"),
        &format!("below the bilinear blit's {b:.3}"),
    );
    assert!(o < b, "ours {o:.3} not below the blit's {b:.3}");
}

/// Per-pass readings of FSR 2 on a pan of the stripe wall: after 32 still frames, 64 frames
/// translating at `PAN_PX` output pixels a frame, at `dist` and scale 1.5, sharpening 0;
/// over the middle half, frame by frame: the output's gradient energy, the depth clip and the
/// dilated masks (render size), the share of output pixels holding a lock and their mean
/// lifetime, the history's share of the blend, and the temporal reactivity.
fn pan_per_pass_readings(dist: f32, pan_px: f32) -> Vec<String> {
    let Some(rig) = stripe_rig() else {
        return Vec::new();
    };
    let (device, queue) = (&rig.device, &rig.queue);
    let light = LightParams::headlamp();
    let mut f = frame_of(device, INTERNAL, OUTPUT, true);
    f.set_upscaler_kind(device, fsr2_sharp(0.0));
    f.fsr2_mut().expect("FSR mode").set_debug(device, true);
    f.set_lighting(vxe_render::Lighting {
        exposure: stripe_exposure(dist),
        ..Default::default()
    });
    f.set_sharpness(queue, 0.5);
    let mut camera = stripe_camera(dist, 0);
    let (_, right, _) = camera.basis();
    let step = right * (pan_px / px_per_voxel(dist));
    let [w, h] = OUTPUT.map(|n| n as usize);
    let [rw, rh] = INTERNAL.map(|n| n as usize);
    let middle = |w: usize, h: usize| -> Vec<usize> {
        (h / 4..3 * h / 4)
            .flat_map(|y| (w / 4..3 * w / 4).map(move |x| y * w + x))
            .collect()
    };
    let (mid, rmid) = (middle(w, h), middle(rw, rh));
    let mut lines = Vec::new();
    for i in 0..SETTLE + 64 {
        if i >= SETTLE {
            camera.position += step;
        }
        f.step(device, queue, &rig.gpu, &camera, &light);
        let n = i + 1;
        if !(n == SETTLE || n == SETTLE + 1 || n == SETTLE + 2 || n == SETTLE + 4 || n % 8 == 0) {
            continue;
        }
        let img = readback::read_rgba8(device, queue, f.target());
        let ge = sharpness(&img, &camera, dist).ge;
        let fsr = f.fsr2().unwrap();
        let cur = f.current_copy();
        let prepared = readback::read_rgba16f(device, queue, fsr.prepared());
        let masks = readback::read_rgba8(device, queue, fsr.dilated_reactive());
        let lock = read_f32_texels(device, queue, fsr.lock_status(cur));
        let weight = read_f32_texels(device, queue, fsr.history_weight());
        let up = readback::read_rgba16f(device, queue, fsr.upscaled(cur));
        let rn = rmid.len() as f64;
        let on = mid.len() as f64;
        let clip = rmid.iter().map(|&k| f64::from(prepared[k][3])).sum::<f64>() / rn;
        let clipped = rmid.iter().filter(|&&k| prepared[k][3] > 0.0).count() as f64 / rn;
        let reactive = rmid.iter().map(|&k| f64::from(masks[4 * k])).sum::<f64>() / rn / 255.0;
        let accum = rmid
            .iter()
            .map(|&k| f64::from(masks[4 * k + 1]))
            .sum::<f64>()
            / rn
            / 255.0;
        let locked = mid.iter().filter(|&&k| lock[2 * k] > 0.0).count() as f64 / on;
        let life = mid.iter().map(|&k| f64::from(lock[2 * k])).sum::<f64>() / on;
        let hist = mid.iter().map(|&k| f64::from(weight[k])).sum::<f64>() / on;
        let tr = mid.iter().map(|&k| f64::from(up[k][3].abs())).sum::<f64>() / on;
        lines.push(format!(
            "frame {n:3}: GE {ge:.5}; clip mean {clip:.4} share {:.2} %; dilated reactive {reactive:.4}, accumulation mask {accum:.4}; locked {:.2} %, lifetime {life:.3}; history share {hist:.4}; temporal reactive {tr:.4}",
            clipped * 100.0,
            locked * 100.0
        ));
    }
    lines
}

/// The pan's per-pass readings at both distances (readings, no bar).
#[test]
fn fsr2_pan_per_pass_readings() {
    for (dist, px) in [(NEAR, PAN_PX), (FAR, PAN_PX), (FAR, 1.0), (FAR, 0.5)] {
        println!("pan at {dist} voxels, {px} px a frame, scale 1.5:");
        for l in pan_per_pass_readings(dist, px) {
            println!("  {l}");
        }
    }
}

// ---- Lights, flicker and shadow motion ----

/// FSR 2 with the reactive mask's light-change value `light` and translucent weight
/// `translucent`, sharpening 0.
fn fsr2_reactive(light: f32, translucent: f32) -> UpscalerKind {
    UpscalerKind::Fsr2(vxe_render::Fsr2Settings {
        reactive_light: light,
        reactive_translucent: translucent,
        ..Default::default()
    })
}

/// A sweeping headlamp and a lamp switched off reach FSR 2's image about as fast as Plan
/// 0024's frame: on the level at the spawn pose with a still camera, at scale 1 and 1.5 (384 x
/// 216 output), the lamp's centroid lags the ideal by at most 2 frames more than Plan 0024's
/// frame does over the sweep's last 16 frames, and the lamp step settles within 10 % of the
/// lamp's settled contribution in at most 2 frames more than that frame takes. The trail, the
/// emission step and the whole-image wobble are readings.
#[test]
fn fsr2_keeps_pace_with_a_sweeping_headlamp_and_a_lamp_step() {
    let Some(failures) = lamp_readings(fsr2_kind(), "fsr2", true) else {
        return;
    };
    assert!(failures.is_empty(), "{failures:?}");
}

/// FSR 2's jitter adds little flicker: on the dark cave's still rig at scale 1.5 (320 x 180
/// output), FSR 2's frames' mean step is at most 0.40/255 above our upscaler's with the jitter
/// off, and its share of values over 4/255 at most 1.15 percentage points above it (FSR 2
/// always jitters; the jitter-off image is the stillest one Plan 0030 compared against).
#[test]
fn fsr2_adds_little_flicker_on_a_still_vein() {
    let own = UpscalerKind::Own(vxe_render::marcher::Upscaler::default());
    let Some((off_mean, off_over)) = still_vein_flicker(false, own) else {
        return;
    };
    let (on_mean, on_over) = still_vein_flicker(true, fsr2_kind()).unwrap();
    let (own_mean, own_over) = still_vein_flicker(true, own).unwrap();
    println!(
        "still vein at scale 1.5: jitter off (ours) mean {off_mean:.4}/255, {:.4} % over 4/255; FSR 2 mean {on_mean:.4}/255, {:.4} %; ours jittered {own_mean:.4}/255, {:.4} %",
        off_over * 100.0,
        on_over * 100.0,
        own_over * 100.0
    );
    assert!(
        on_mean <= off_mean + 0.40,
        "mean {on_mean:.4} against {off_mean:.4}"
    );
    assert!(
        on_over * 100.0 <= off_over * 100.0 + 1.15,
        "{:.4} % against {:.4} %",
        on_over * 100.0,
        off_over * 100.0
    );
}

/// The output pixels within 2 pixels (Chebyshev) of a lamp-shadow boundary on the level at
/// its spawn: in Plan 0024's frame (scale 1, jitter off, the face look) settled 48 frames with
/// the lamp on and off, a pair of 4-neighbours on one surface (both hit, the same entry face
/// axis, `t` within 5 %) whose lamp contribution (lit less off, luminance) differs by more
/// than 16/255.
fn shadow_boundary(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    gpu: &GpuWorld,
    camera: &vxe_render::Camera,
) -> Vec<bool> {
    let light = LightParams::headlamp();
    let settled = |lamp: bool| {
        let mut f = Pipe::Reference.frame(device, UpscalerKind::Off, lamp);
        for _ in 0..48 {
            f.step(device, queue, gpu, camera, &light);
        }
        f
    };
    let lit_frame = settled(true);
    let lit = lumas(&readback::read_rgba8(device, queue, lit_frame.target()));
    let pos = readback::read_rgba32f(device, queue, lit_frame.hit_positions());
    let rec = readback::read_rgba16u(device, queue, lit_frame.hit_records());
    let off = lumas(&readback::read_rgba8(
        device,
        queue,
        settled(false).target(),
    ));
    let [w, h] = OUTPUT.map(|n| n as usize);
    let c: Vec<f64> = lit
        .iter()
        .zip(&off)
        .map(|(a, b)| (a - b).max(0.0))
        .collect();
    let surface = |a: usize, b: usize| {
        let hit = |i: usize| rec[i][3] & 1 == 1;
        let axis = |i: usize| (rec[i][3] >> 3) & 7;
        let (ta, tb) = (pos[a][3], pos[b][3]);
        hit(a) && hit(b) && axis(a) == axis(b) && (ta - tb).abs() <= 0.05 * ta.min(tb)
    };
    let mut mask = vec![false; w * h];
    let mut mark = |x: usize, y: usize| {
        for yy in y.saturating_sub(2)..(y + 3).min(h) {
            for xx in x.saturating_sub(2)..(x + 3).min(w) {
                mask[yy * w + xx] = true;
            }
        }
    };
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            for (j, ok) in [(i + 1, x + 1 < w), (i + w, y + 1 < h)] {
                if ok && surface(i, j) && (c[i] - c[j]).abs() > 16.0 {
                    mark(x, y);
                    mark(j % w, j / w);
                }
            }
        }
    }
    mask
}

/// One pipeline's wobble on the level's spawn, lamp on, still, after 32 frames: the mean
/// over pixels of each output pixel's luminance range over one jitter cycle, over the whole
/// image and over `boundary`; under FSR 2 also the composite's input (FSR's colour input, the
/// jittered lighting before the upscaler, put through the tone arithmetic) over the internal
/// pixels nearest the boundary pixels' centres.
fn wobble(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    gpu: &GpuWorld,
    camera: &vxe_render::Camera,
    pipe: Pipe,
    kind: UpscalerKind,
    boundary: &[bool],
) -> (f64, f64, Option<f64>) {
    let light = LightParams::headlamp();
    let mut f = pipe.frame(device, kind, true);
    f.set_look(LookParams {
        normal_mode: vxe_render::NormalMode::Face,
        ..f.look()
    });
    for _ in 0..SETTLE {
        f.step(device, queue, gpu, camera, &light);
    }
    let n = boundary.len();
    let internal = f.internal_size();
    let ni = (internal[0] * internal[1]) as usize;
    let (mut lo, mut hi) = (vec![f64::MAX; n], vec![f64::MIN; n]);
    let (mut ilo, mut ihi) = (vec![f64::MAX; ni], vec![f64::MIN; ni]);
    let fsr = kind.is_fsr2() && pipe != Pipe::Reference;
    for _ in 0..pipe.phases(kind) {
        f.step(device, queue, gpu, camera, &light);
        for (i, v) in lumas(&readback::read_rgba8(device, queue, f.target()))
            .into_iter()
            .enumerate()
        {
            lo[i] = lo[i].min(v);
            hi[i] = hi[i].max(v);
        }
        if fsr {
            let lighting = f.lighting();
            let colour = read_f32_texels(device, queue, f.fsr2().unwrap().colour_in());
            for (i, c) in colour.chunks(4).enumerate() {
                let [r, g, b] = tone_bytes([c[0], c[1], c[2], c[3]], &lighting).map(f64::from);
                let v = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                ilo[i] = ilo[i].min(v);
                ihi[i] = ihi[i].max(v);
            }
        }
    }
    let range = |i: usize| hi[i] - lo[i];
    let all = (0..n).map(range).sum::<f64>() / n as f64;
    let at: Vec<usize> = (0..n).filter(|&i| boundary[i]).collect();
    let edge = at.iter().map(|&i| range(i)).sum::<f64>() / at.len().max(1) as f64;
    let input = fsr.then(|| {
        let [w, _] = OUTPUT.map(|n| n as usize);
        let [iw, ih] = internal.map(|n| n as usize);
        at.iter()
            .map(|&i| {
                let (x, y) = (i % w, i / w);
                let ix = ((x as f64 + 0.5) * iw as f64 / OUTPUT[0] as f64) as usize;
                let iy = ((y as f64 + 0.5) * ih as f64 / OUTPUT[1] as f64) as usize;
                let k = iy.min(ih - 1) * iw + ix.min(iw - 1);
                ihi[k] - ilo[k]
            })
            .sum::<f64>()
            / at.len().max(1) as f64
    });
    (all, edge, input)
}

/// Shadow wobble no worse than ours at scale 1.5: on the level's spawn, lamp on, a still
/// camera in the face look, over one jitter cycle (each pipeline's own), FSR 2's mean
/// per-pixel luminance range at scale 1.5 is at most our upscaler's, over the whole image and
/// over the pixels within 2 output pixels of a lamp-shadow boundary. Scale 1 against ours is a
/// `reading:` line (the same bar, missed with no port defect found); Plan 0024's frame and the
/// composite's input under FSR 2 (the jittered lighting before the upscaler) are readings.
#[test]
fn fsr2_at_scale_1_5_moves_shadows_no_more_than_ours_on_a_still_camera() {
    let (world, camera) = level();
    let Some((device, queue, gpu)) = level_rig(&world, &camera) else {
        return;
    };
    let boundary = shadow_boundary(&device, &queue, &gpu, &camera);
    println!(
        "lamp-shadow boundary: {} of {} output pixels",
        boundary.iter().filter(|&&b| b).count(),
        boundary.len()
    );
    let own = UpscalerKind::Own(vxe_render::marcher::Upscaler::default());
    let (ra, re, _) = wobble(
        &device,
        &queue,
        &gpu,
        &camera,
        Pipe::Reference,
        own,
        &boundary,
    );
    println!("reference: wobble {ra:.3}/255, at the boundary {re:.3}/255");
    let mut failures = Vec::new();
    for pipe in [Pipe::U1, Pipe::U15] {
        let (oa, oe, _) = wobble(&device, &queue, &gpu, &camera, pipe, own, &boundary);
        let (fa, fe, fi) = wobble(&device, &queue, &gpu, &camera, pipe, fsr2_kind(), &boundary);
        println!(
            "{pipe:?}: ours wobble {oa:.3}/255 over {} frames, at the boundary {oe:.3}/255; FSR 2 {fa:.3}/255 over {} frames, at the boundary {fe:.3}/255, its input there {:.3}/255",
            pipe.phases(own),
            pipe.phases(fsr2_kind()),
            fi.unwrap_or(f64::NAN)
        );
        if pipe == Pipe::U1 {
            reading(
                "level, still, FSR 1 wobble",
                &format!("{fa:.3}/255"),
                &format!("at most ours {oa:.3}/255"),
            );
            reading(
                "level, still, FSR 1 wobble at the lamp-shadow boundary",
                &format!("{fe:.3}/255"),
                &format!("at most ours {oe:.3}/255"),
            );
            continue;
        }
        if !at_most(fa, oa) {
            failures.push(format!("{pipe:?}: wobble {fa:.3} over ours {oa:.3}"));
        }
        if !at_most(fe, oe) {
            failures.push(format!(
                "{pipe:?}: boundary wobble {fe:.3} over ours {oe:.3}"
            ));
        }
    }
    for f in &failures {
        println!("over a bar: {f}");
    }
    assert!(failures.is_empty(), "{failures:?}");
}

/// A reading, no bar: the lamp, flicker and wobble rigs under FSR 2 for each reactive-mask
/// tuning tried (the light-change value, the translucent weight). Run with `--ignored`.
#[test]
#[ignore = "a diagnostic reading of the reactive mask's tuning; run by hand with --ignored"]
fn fsr2_reactive_tuning_readings() {
    let (world, camera) = level();
    let Some((device, queue, gpu)) = level_rig(&world, &camera) else {
        return;
    };
    let boundary = shadow_boundary(&device, &queue, &gpu, &camera);
    for (light, translucent) in [(1.0, 1.0), (0.6, 1.0), (0.3, 1.0), (0.0, 1.0), (1.0, 0.6)] {
        let kind = fsr2_reactive(light, translucent);
        let label = format!("light {light}, translucent {translucent}");
        lamp_readings(kind, &label, false);
        let (m, o) = still_vein_flicker(true, kind).unwrap();
        println!(
            "[{label}] flicker at 1.5: {m:.4}/255, {:.4} % over 4/255",
            o * 100.0
        );
        for pipe in [Pipe::U1, Pipe::U15] {
            let (a, e, i) = wobble(&device, &queue, &gpu, &camera, pipe, kind, &boundary);
            println!(
                "[{label}] {pipe:?}: wobble {a:.3}/255, at the boundary {e:.3}/255, its input there {:.3}/255",
                i.unwrap_or(f64::NAN)
            );
        }
    }
}

/// Per-pass readings of FSR 2 on the wobble rig (readings, no bar): the level's spawn, lamp
/// on, still, face look, at scale 1 and 1.5; over one jitter cycle after 32 frames, at the
/// lamp-shadow boundary's output pixels, frame by frame: the history's share of the blend,
/// the share holding a lock, the temporal reactivity (the upscaled colour's alpha), and at the
/// internal pixels nearest them the dilated reactive and accumulation masks and the depth clip.
#[test]
fn fsr2_wobble_per_pass_readings() {
    let (world, camera) = level();
    let Some((device, queue, gpu)) = level_rig(&world, &camera) else {
        return;
    };
    let boundary = shadow_boundary(&device, &queue, &gpu, &camera);
    let at: Vec<usize> = (0..boundary.len()).filter(|&i| boundary[i]).collect();
    let light = LightParams::headlamp();
    for pipe in [Pipe::U1, Pipe::U15] {
        let kind = fsr2_kind();
        let mut f = pipe.frame(&device, kind, true);
        f.fsr2_mut().expect("FSR mode").set_debug(&device, true);
        for _ in 0..SETTLE {
            f.step(&device, &queue, &gpu, &camera, &light);
        }
        let internal = f.internal_size().map(|n| n as usize);
        let [w, _] = OUTPUT.map(|n| n as usize);
        let inner: Vec<usize> = at
            .iter()
            .map(|&i| {
                let (x, y) = (i % w, i / w);
                let ix = (x as f64 + 0.5) * internal[0] as f64 / OUTPUT[0] as f64;
                let iy = (y as f64 + 0.5) * internal[1] as f64 / OUTPUT[1] as f64;
                (iy as usize).min(internal[1] - 1) * internal[0]
                    + (ix as usize).min(internal[0] - 1)
            })
            .collect();
        println!("{pipe:?}, {} boundary pixels:", at.len());
        let n = at.len() as f64;
        for k in 0..pipe.phases(kind) {
            f.step(&device, &queue, &gpu, &camera, &light);
            let fsr = f.fsr2().unwrap();
            let cur = f.current_copy();
            let weight = read_f32_texels(&device, &queue, fsr.history_weight());
            let lock = read_f32_texels(&device, &queue, fsr.lock_status(cur));
            let up = readback::read_rgba16f(&device, &queue, fsr.upscaled(cur));
            let masks = readback::read_rgba8(&device, &queue, fsr.dilated_reactive());
            let prepared = readback::read_rgba16f(&device, &queue, fsr.prepared());
            let hist = at.iter().map(|&i| f64::from(weight[i])).sum::<f64>() / n;
            let locked = at.iter().filter(|&&i| lock[2 * i] > 0.0).count() as f64 / n;
            let tr = at.iter().map(|&i| f64::from(up[i][3].abs())).sum::<f64>() / n;
            let reactive = inner.iter().map(|&i| f64::from(masks[4 * i])).sum::<f64>() / n / 255.0;
            let accum = inner
                .iter()
                .map(|&i| f64::from(masks[4 * i + 1]))
                .sum::<f64>()
                / n
                / 255.0;
            let clip = inner
                .iter()
                .map(|&i| f64::from(prepared[i][3]))
                .sum::<f64>()
                / n;
            println!(
                "  frame {k:2}: history share {hist:.4}; locked {:.2} %; temporal reactive {tr:.4}; dilated reactive {reactive:.4}, accumulation mask {accum:.4}; clip {clip:.4}",
                locked * 100.0
            );
        }
    }
}

// ---- The readings on both adapters ----

/// `frame_bytes` in FSR mode at scale 1.5 and 1080p output is our upscaler's frame at those
/// sizes, less its two output-size rgba16float histories (not allocated in FSR mode), plus
/// FSR 2's inventory, resource by resource, and no more: at render size (1280 x 720) the
/// rgba32float colour input, the r32float depth, the rg32float motion, the rgba8unorm masks,
/// the reconstructed-depth `u32` buffer, the r32float dilated depth, two rg32float dilated
/// motions, the r32float lock luma, the rgba16float prepared colour and the rgba8unorm dilated
/// masks; at output size two rgba16float upscaled colours (RCAS writes the older one), two
/// rg32float lock statuses, two rgba8unorm luma histories and the new-locks `f32` buffer; the
/// luminance pyramid's r32float mips 4 (40 x 22) and 5 (20 x 11); two rg32float 1 x 1
/// auto-exposure texels, the rg32float exposure texel and the 1 x 1 r32float history weight.
#[test]
fn frame_bytes_under_fsr2_swap_the_histories_for_fsr2s_inventory() {
    let Some((device, _queue)) = software_device(64 << 20) else {
        return;
    };
    let output = [1920u32, 1080];
    let internal = vxe_render::RenderScale::OneHalf.internal(output);
    assert_eq!(internal, [1280, 720]);
    let ours = Frame::with_sizes(&device, internal, output).frame_bytes();
    let mut f = Frame::with_sizes(&device, internal, output);
    f.set_upscaler_kind(&device, fsr2_kind());
    let fsr = f.frame_bytes();
    let px = |[w, h]: [u32; 2]| u64::from(w) * u64::from(h);
    let (r, d) = (px(internal), px(output));
    let histories = 2 * 8 * d;
    let render = (16 + 4 + 8 + 4 + 4 + 4 + 2 * 8 + 4 + 8 + 4) * r;
    let display = (2 * 8 + 2 * 8 + 2 * 4 + 4) * d;
    let tiny = 4 * px([40, 22]) + 4 * px([20, 11]) + 2 * 8 + 8 + 4;
    let inventory = render + display + tiny;
    println!(
        "frame_bytes at 1080p output, scale 1.5: ours {ours} B, FSR 2 {fsr} B; histories {histories} B; FSR 2's inventory {inventory} B (render size {render}, output size {display}, the rest {tiny})"
    );
    assert_eq!(fsr, ours - histories + inventory);
}
