// `gpu_sun`'s far staircase bar (Plan 0086 Phase 3), included by `gpu_sun.rs` (kept apart for
// the file-size cap), with the rigs it shares with `mod reading`.

/// Per frame the history counters, then the direct accumulation's luminance and the lighting
/// grid's hit positions.
type FarRun = (Vec<[u32; 4]>, Vec<f32>, Vec<[f32; 4]>);

/// A far frame of `world` at `cam`: 320 x 180 under FSR 2 at 1.5 (480 x 270), half lighting,
/// the store counters on, the key light alone (lamp off, no sky ray, filter on), manual
/// exposure 1, `sky`, the stair rules hooked off with `no_stair`; `frames` frames, then per
/// frame of 64 more the history counters and, at the last, the direct accumulation's
/// luminance and the lighting grid's hit positions.
fn far_run(world: &World, cam: &Camera, sky_: Sky, no_stair: bool) -> Option<FarRun> {
    let (device, queue) = device()?;
    let gpu = mirror(&device, &queue, world, None);
    let mut frame = Frame::with_sizes(&device, [320, 180], [480, 270]);
    frame.set_upscaler_kind(
        &device,
        vxe_render::UpscalerKind::Fsr2(vxe_render::Fsr2Settings::default()),
    );
    frame.set_lighting(Lighting {
        lamp: false,
        exposure: Exposure::Manual(1.0),
        sky_rays: 0,
        ..Lighting::default()
    });
    frame.set_light_hooks(vxe_render::LightHooks {
        no_stair,
        ..Default::default()
    });
    frame.set_count_store(true);
    frame.set_resolution(&device, LightingResolution::Full);
    // The fly-through's default far normal.
    frame.set_look(vxe_render::LookParams {
        far_normal: [2.0, 6.0],
        ..vxe_render::LookParams::default()
    });
    frame.set_sky(Some(sky_));
    let light = LightParams::headlamp();
    for _ in 0..16 {
        frame.step(&device, &queue, &gpu, cam, &light);
    }
    let mut counts = Vec::new();
    for _ in 0..64 {
        frame.step(&device, &queue, &gpu, cam, &light);
        counts.push(frame.read_history_rejections(&device, &queue));
    }
    let lum = vxe_render::debug::read_rgba16f(&device, &queue, frame.direct_accumulated())
        .iter()
        .map(|p| p[3])
        .collect();
    let hits = vxe_render::debug::read_rgba32f(&device, &queue, frame.light_hit_positions());
    Some((counts, lum, hits))
}

/// A camera at `eye` looking at `at`.
fn looking(eye: [f32; 3], at: [f32; 3]) -> Camera {
    let d = Vec3::from(at) - Vec3::from(eye);
    Camera {
        position: Vec3::from(eye),
        yaw: d.y.atan2(d.x),
        pitch: (d.z / d.length()).asin(),
        vfov: 60f32.to_radians(),
    }
}

/// One frame of [`pair_run`]: the pair and the visibility sample per lighting pixel.
type PairFrame = (Vec<[f32; 4]>, Vec<[u8; 4]>);

/// [`pair_run`]'s edit: per frame index, the box it changed in the world, if any.
type PairEdit<'a> = dyn FnMut(u32, &World) -> Option<[[i32; 3]; 2]> + 'a;

/// The visibility pair's rig (ADR-0070): `world` mirrored, a frame of SIZE at full lighting
/// resolution with no jitter, filter off, exposure 1, one sky ray, the coarse walk,
/// `hooks`, under `Sky::uniform(1.0)` (no key light). `frames` frames of `cam`, `edit`
/// called before each with the frame's index, the world and the mirror (it returns an edit
/// box to invalidate); per frame, the pair and the visibility sample at every lighting pixel.
fn pair_run(
    world: &World,
    cam: &Camera,
    frames: u32,
    hooks: vxe_render::LightHooks,
    edit: &mut PairEdit<'_>,
) -> Option<Vec<PairFrame>> {
    let (device, queue) = device()?;
    let mut gpu = mirror(&device, &queue, world, None);
    let mut f = frame(&device, SIZE, ShadowWalk::Coarse);
    f.set_jitter(&device, false);
    f.set_lighting(Lighting {
        sky_rays: 1,
        ..f.lighting()
    });
    f.set_light_hooks(hooks);
    f.set_sky(Some(Sky {
        altitude_zero_z: FLOOR as f32,
        ..Sky::uniform(1.0)
    }));
    let mut out = Vec::new();
    for i in 0..frames {
        if let Some([min, max]) = edit(i, world) {
            let mut enc =
                device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            gpu.sync(world, &queue, &mut enc).expect("the mirror fits");
            queue.submit([enc.finish()]);
            f.invalidate(min, max);
        }
        f.step(&device, &queue, &gpu, cam, &LightParams::headlamp());
        let pair = vxe_render::debug::read_rgba16f(&device, &queue, f.visibility_pair());
        let vis = vxe_render::debug::read_rgba8(&device, &queue, f.visibility_samples());
        let vis = vis.chunks(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
        out.push((pair, vis));
    }
    Some(out)
}

/// The centre lighting pixel's index of a SIZE frame.
fn centre() -> usize {
    let [w, h] = SIZE.map(|v| v as usize);
    (h / 2) * w + w / 2
}

/// Plan 0086 Phase 3 bar 1: a staircase of 64 one-voxel steps up along +x
/// (rock under `z = FLOOR + clamp(x - 160, 0, 64)` over a 384 x 192 floor) seen from about
/// 220 voxels (0.7 pixels a voxel at 320 wide, the far normal's full weight), FSR 2 at 1.5,
/// a still camera, a zenith sun: the share of the history test's hits whose history reset per
/// frame over 64 frames, by cause, with the stair rules hooked off (printed: the rig before
/// Phase 3) and on (the default): with them on, no frame resets 1 % of the tested hits.
#[test]
#[ignore = "soak"]
fn a_far_staircase_keeps_its_history() {
    let f = |p: [i32; 3]| {
        if p[2] < FLOOR + (p[0] - 160).clamp(0, 64) {
            ROCK
        } else {
            AIR
        }
    };
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [6, 3, 2]),
        &FnSource(f),
        &table(),
    );
    let cam = looking(
        [40.0, 96.0, FLOOR as f32 + 120.0],
        [192.0, 96.0, FLOOR as f32 + 32.0],
    );
    let s = sky(dir(90.0, 0.0), dir(-90.0, 0.0), [[1.0; 3], [0.0; 3]]);
    for no_stair in [true, false] {
        let Some((counts, _, _)) = far_run(&world, &cam, s, no_stair) else {
            return;
        };
        let t: [u64; 4] = std::array::from_fn(|k| counts.iter().map(|w| u64::from(w[k])).sum());
        let max = counts
            .iter()
            .map(|w| f64::from(w[1] + w[2] + w[3]) / f64::from(w[0].max(1)))
            .fold(0.0, f64::max);
        println!(
            "far staircase, stair rules {}: tested {} a frame, reset {:.4} (material {:.4} normal {:.4} other {:.4}), worst frame {max:.4}",
            if no_stair { "off" } else { "on" },
            t[0] / 64,
            (t[1] + t[2] + t[3]) as f64 / t[0].max(1) as f64,
            t[1] as f64 / t[0].max(1) as f64,
            t[2] as f64 / t[0].max(1) as f64,
            t[3] as f64 / t[0].max(1) as f64,
        );
        if !no_stair {
            assert!(t[0] > 0, "the staircase is tested");
            assert!(max < 0.01, "the worst frame resets {max:.4} of the tested hits");
        }
    }
}
