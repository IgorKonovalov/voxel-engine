//! Emissive voxels as light samples, on the software adapter (lavapipe).

mod support;

use flythrough::cave_materials;
use support::*;
use vxe_render::gpu_world::{LIGHT_SAMPLE_BYTES, LightSample};
use vxe_render::{GpuWorld, MirrorConfig, Window, readback};
use vxe_world::*;

/// The cave's table plus an emissive `glow` row and its non-emissive twin `dull`.
fn glow_table() -> (MaterialTable, MaterialId, MaterialId) {
    let mut t = cave_materials();
    let glow = t.push(Material {
        albedo: [0.3, 0.6, 0.9],
        emission: [1.0, 1.0, 1.0],
        ..Material::named("glow")
    });
    let dull = t.push(Material {
        albedo: [0.3, 0.6, 0.9],
        ..Material::named("dull")
    });
    (t, glow, dull)
}

fn box_op(min: [i64; 3], max: [i64; 3], mode: Mode) -> EditOp {
    EditOp::box_(
        Fixed3(min.map(|v| v * FIXED_ONE)),
        Fixed3(max.map(|v| v * FIXED_ONE)),
        mode,
    )
}

/// The samples of `range` read back from the buffer.
fn read_samples(
    device: &vxe_render::wgpu::Device,
    queue: &vxe_render::wgpu::Queue,
    gpu: &GpuWorld,
    (start, len): (u32, u32),
) -> Vec<LightSample> {
    let bytes = readback::read_buffer(
        device,
        queue,
        gpu.light_buffer(),
        u64::from(start) * LIGHT_SAMPLE_BYTES,
        u64::from(len) * LIGHT_SAMPLE_BYTES,
    );
    let word = |c: &[u8], i: usize| u32::from_le_bytes(c[4 * i..4 * i + 4].try_into().unwrap());
    bytes
        .chunks(LIGHT_SAMPLE_BYTES as usize)
        .map(|c| LightSample {
            voxel: [0, 1, 2].map(|i| word(c, i) as i32),
            material_faces: word(c, 3),
        })
        .collect()
}

/// A 4 x 4 x 4 cube of an emissive material at voxels 2..6 of brick 0 of one air chunk holds
/// 56 samples after a sync (64 voxels less the 8 interior ones), each a cube voxel with its
/// outward faces; carving the corner voxel (2, 2, 2) leaves 55; replacing the material by its
/// non-emissive twin leaves 0 (its range zeroed), and replacing it back refills a range that
/// reads back the samples, never the zeroes; and after the chunk leaves the mirror its range is
/// free and zeroed and `light_samples` reads 0.
#[test]
fn an_emissive_cube_holds_its_exposed_voxels_as_samples() {
    let Some((device, queue)) = software_device(64 << 20) else {
        return;
    };
    let (table, glow, dull) = glow_table();
    let coord = ChunkCoord([0, 0, 0]);
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [1, 1, 1]),
        &UniformSource(AIR),
        &table,
    );
    world
        .apply(&box_op([2; 3], [6; 3], Mode::Fill(glow)))
        .unwrap();
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig::new(Window::Fixed(ChunkExtent::new([0, 0, 0], [1, 1, 1]))),
    );
    let st = gpu.sync_now(&world, &device, &queue).unwrap();
    println!(
        "cube: {} samples, {} bricks scanned, {} us",
        gpu.light_samples(),
        st.light_bricks,
        st.light_us
    );
    assert_eq!(gpu.light_samples(), 56);
    assert_eq!(st.light_samples, 56);
    let range = gpu.light_range(coord).expect("the chunk has a range");
    assert_eq!(range.1, 56);
    let samples = read_samples(&device, &queue, &gpu, range);
    for s in &samples {
        assert_eq!(s.material(), glow);
        let v = s.voxel;
        assert!(v.iter().all(|&c| (2..6).contains(&c)), "{v:?}");
        // Bit 2 k for a voxel on the cube's low face of axis k, 2 k + 1 on its high face.
        let want = (0..3).fold(0, |f, k| {
            f | u32::from(v[k] == 2) << (2 * k) | u32::from(v[k] == 5) << (2 * k + 1)
        });
        assert_eq!(s.faces(), want, "{v:?}");
    }

    world.apply(&box_op([2; 3], [3; 3], Mode::Carve)).unwrap();
    gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(gpu.light_samples(), 55);
    // The old range was zeroed and the new one written in the same sync's encoder, in that
    // order: the samples read back, none of them zero.
    let rewritten = gpu.light_range(coord).expect("the chunk has a range");
    let samples = read_samples(&device, &queue, &gpu, rewritten);
    assert!(
        samples.iter().all(|s| s.material() == glow),
        "a sample reads zero"
    );

    world
        .apply(&box_op(
            [0; 3],
            [64; 3],
            Mode::Replace {
                from: glow,
                to: dull,
            },
        ))
        .unwrap();
    gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(gpu.light_samples(), 0);
    assert_eq!(gpu.light_range(coord), None);

    // Glow again, then the chunk leaves the mirror.
    world
        .apply(&box_op(
            [0; 3],
            [64; 3],
            Mode::Replace {
                from: dull,
                to: glow,
            },
        ))
        .unwrap();
    gpu.sync_now(&world, &device, &queue).unwrap();
    let range = gpu.light_range(coord).expect("the chunk has a range again");
    assert_eq!(range.1, 55);
    // A range zeroed by one sync and refilled by a later one reads back the new samples.
    let refilled = read_samples(&device, &queue, &gpu, range);
    assert!(
        refilled.iter().all(|s| s.material() == glow),
        "a sample reads zero"
    );
    gpu.apply_events(&StreamEvents {
        evicted: vec![coord],
        ..StreamEvents::default()
    });
    assert_eq!(gpu.light_samples(), 0);
    assert_eq!(gpu.light_range(coord), None);
    let _ = gpu.sync_now(&world, &device, &queue);
    assert_eq!(
        gpu.light_span(),
        0,
        "the freed range lowered the high-water mark"
    );
    let zeroed = read_samples(&device, &queue, &gpu, range);
    assert!(
        zeroed.iter().all(|s| s.material() == AIR),
        "the range is zeroed"
    );
}

/// Emissive voxels at the even layers z = 0, 2, 4 of one chunk, in index order, 10 000 of
/// them: every one is exposed (its z neighbours are air).
struct Layers(MaterialId);

impl WorldSource for Layers {
    fn chunk(&self, _coord: ChunkCoord, out: &mut Chunk) {
        for slot in 0..CHUNK_BRICKS {
            let b = brick_of_slot(slot);
            out.set_brick(
                slot,
                Brick::from_fn(|l| {
                    let v = [0, 1, 2].map(|k| (b[k] * BRICK_EDGE + l[k]) as i64);
                    let i = v[2] / 2 * 4096 + v[1] * 64 + v[0];
                    if v[2] % 2 == 0 && i < 10_000 {
                        self.0
                    } else {
                        AIR
                    }
                }),
            );
        }
    }
}

/// A chunk with 10 000 emissive surface voxels and a cap of 8192 holds 5000, every second by
/// index.
#[test]
fn a_chunk_over_the_cap_keeps_every_kth_sample() {
    let Some((device, queue)) = software_device(64 << 20) else {
        return;
    };
    let (table, glow, _) = glow_table();
    let extent = ChunkExtent::new([0, 0, 0], [1, 1, 1]);
    let world = World::generate(extent, &Layers(glow), &table);
    let mut uncapped = GpuWorld::with_config(
        &device,
        MirrorConfig {
            light_chunk_cap: 1 << 20,
            ..MirrorConfig::new(Window::Fixed(extent))
        },
    );
    uncapped.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(uncapped.light_samples(), 10_000);
    let all = read_samples(&device, &queue, &uncapped, (0, 10_000));

    let world = World::generate(extent, &Layers(glow), &table);
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            light_chunk_cap: 8192,
            ..MirrorConfig::new(Window::Fixed(extent))
        },
    );
    gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(gpu.light_samples(), 5000);
    let kept = read_samples(&device, &queue, &gpu, (0, 5000));
    let every_second: Vec<LightSample> = all.iter().step_by(2).copied().collect();
    assert_eq!(kept, every_second);
}

/// One chunk of rock with a closed box of air, interior x and y `[22, 42)`, z `[20, 30)`,
/// whose whole ceiling (the layer z = 30 above the interior) is `glow`.
fn ceiling_box(table: &MaterialTable, glow: MaterialId) -> World {
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [1, 1, 1]),
        &UniformSource(flythrough::cave::ROCK),
        table,
    );
    world
        .apply(&box_op([22, 22, 20], [42, 42, 30], Mode::Carve))
        .unwrap();
    world
        .apply(&box_op(
            [22, 22, 30],
            [42, 42, 31],
            Mode::Replace {
                from: flythrough::cave::ROCK,
                to: glow,
            },
        ))
        .unwrap();
    world
}

/// The closed box lit by its emissive ceiling (emission `[1, 1, 1]`), the lamp off and no
/// filter: the irradiance at the floor's centre, averaged over 64 frames read back one by one,
/// is within 5 % of `E = PI L F = 1.741`, the form factor of a point at the centre of a
/// 2h x 2h rectangle at distance h being `4 x 0.13853 = 0.5541`.
#[test]
fn an_emissive_ceiling_gives_the_analytic_irradiance_at_the_floor_centre() {
    let Some((device, queue)) = software_device(64 << 20) else {
        return;
    };
    let (table, glow, _) = glow_table();
    let world = ceiling_box(&table, glow);
    let mut gpu = GpuWorld::new(&device, world.extent());
    gpu.sync_now(&world, &device, &queue).unwrap();
    println!(
        "{} light samples over a span of {}",
        gpu.light_samples(),
        gpu.light_span()
    );
    // A 1 x 1 target: the pixel's ray is the camera's axis, straight down at (32, 32, 20).
    let camera = vxe_render::Camera {
        position: [32.0, 32.0, 29.0].into(),
        yaw: 0.0,
        pitch: -std::f32::consts::FRAC_PI_2,
        vfov: 0.5,
    };
    let mut frame = vxe_render::Frame::new(&device, 1, 1);
    frame.set_lighting(vxe_render::Lighting {
        lamp: false,
        ..vxe_render::Lighting::default()
    });
    let light = vxe_render::LightParams::headlamp();
    let mut sum = 0.0f64;
    let mut readings = Vec::new();
    for i in 0..64 {
        frame.set_frame_index(i);
        frame.render(&device, &queue, &gpu, &camera, &light);
        let hit = readback::read_buffer(
            &device,
            &queue,
            &hit_pos_buffer(&device, &queue, &frame),
            0,
            16,
        );
        if i == 0 {
            let p: Vec<f32> = hit
                .chunks(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .collect();
            println!("hit at {:?}", &p[..3]);
            assert!(
                (p[2] - 20.0).abs() < 1e-3 && (p[0] - 32.0).abs() < 1e-3,
                "{p:?}"
            );
        }
        let e = readback::read_rgba16f(&device, &queue, frame.irradiance())[0];
        assert!(
            (e[0] - e[1]).abs() < 1e-3 && (e[1] - e[2]).abs() < 1e-3,
            "{e:?}"
        );
        readings.push(e[0]);
        sum += f64::from(e[0]);
    }
    let mean = sum / 64.0;
    let analytic = std::f64::consts::PI * 4.0 * 0.13853;
    println!(
        "floor centre irradiance over 64 frames: mean {mean:.4} against {analytic:.4} ({:+.2} %); first frames {:?}",
        (mean / analytic - 1.0) * 100.0,
        &readings[..8]
    );
    assert!((mean / analytic - 1.0).abs() <= 0.05, "{mean:.4}");
}

/// The hit buffer's position texture copied into a buffer of one texel (the target is 1 x 1).
fn hit_pos_buffer(
    device: &vxe_render::wgpu::Device,
    queue: &vxe_render::wgpu::Queue,
    frame: &vxe_render::Frame,
) -> vxe_render::wgpu::Buffer {
    use vxe_render::wgpu;
    let buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 256,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        frame.hit_positions().as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(1),
            },
        },
        frame.hit_positions().size(),
    );
    queue.submit([enc.finish()]);
    buf
}

/// The hit positions, records and ages of `frame`'s last encoded frame.
struct FrameRead {
    pos: Vec<[f32; 4]>,
    info: Vec<[u16; 4]>,
    age: Vec<f32>,
}

fn read_frame(
    device: &vxe_render::wgpu::Device,
    queue: &vxe_render::wgpu::Queue,
    frame: &vxe_render::Frame,
) -> FrameRead {
    FrameRead {
        pos: readback::read_rgba32f(device, queue, frame.hit_positions()),
        info: readback::read_rgba16u(device, queue, frame.hit_records()),
        age: readback::read_rgba16f(device, queue, frame.moments())
            .iter()
            .map(|m| m[2])
            .collect(),
    }
}

/// Whether `p` lies in the voxels `min ..= max` dilated by one voxel, faces included.
fn in_dilated(p: [f32; 4], (min, max): (VoxelCoord, VoxelCoord)) -> bool {
    (0..3).all(|k| p[k] >= (min.0[k] - 1) as f32 && p[k] <= (max.0[k] + 2) as f32)
}

/// A static camera over the cavity for 16 frames, then a radius-4 carve where the view
/// axis meets the wall: the age reads 0 in every pixel whose hit lies in the carve's `aabb`
/// dilated by one voxel and 16 in every other hit pixel; one more frame reads 1 and 17.
#[test]
fn history_dies_in_the_edit_box_and_ages_elsewhere() {
    let world = cavity_world();
    let Some(mut rig) = LookRig::new(&world, None) else {
        return;
    };
    let camera = cavity_camera();
    let light = vxe_render::LightParams::headlamp();
    for _ in 0..16 {
        rig.frame
            .step(&rig.device, &rig.queue, &rig.gpu, &camera, &light);
    }
    let r = world
        .apply(&EditOp::sphere(
            centre_of_voxel([40, 0, 0]),
            4 * FIXED_ONE,
            Mode::Carve,
        ))
        .unwrap();
    let aabb = r.aabb.expect("the carve removes rock");
    rig.gpu.sync_now(&world, &rig.device, &rig.queue).unwrap();
    rig.frame.invalidate(aabb.0.0, aabb.1.0);
    for (k, (inside_age, outside_age)) in [(0.0, 16.0), (1.0, 17.0)].into_iter().enumerate() {
        rig.frame
            .step(&rig.device, &rig.queue, &rig.gpu, &camera, &light);
        let f = read_frame(&rig.device, &rig.queue, &rig.frame);
        let (mut inside, mut outside) = (0, 0);
        for i in 0..f.pos.len() {
            if f.info[i][3] & 1 == 0 {
                continue;
            }
            let want = if in_dilated(f.pos[i], aabb) {
                inside += 1;
                inside_age
            } else {
                outside += 1;
                outside_age
            };
            assert_eq!(f.age[i], want, "pixel {i} at {:?}", f.pos[i]);
        }
        println!(
            "frame {}: {inside} hit pixels in the dilated box at age {inside_age}, {outside} outside at {outside_age}",
            17 + k
        );
        assert!(inside > 0 && outside > 0);
    }
}

/// A camera that moves one voxel sideways between two frames keeps age 1 or more in at least
/// 95 % of the pixels that hit the cavity's unchanged material, and every pixel whose hit
/// changed material (a `Replace` the frame is not told of) reads 0.
#[test]
fn reprojection_holds_across_a_sideways_step_and_a_material_change_resets() {
    let world = cavity_world();
    let Some(mut rig) = LookRig::new(&world, None) else {
        return;
    };
    let light = vxe_render::LightParams::headlamp();
    let mut camera = cavity_camera();
    rig.frame
        .step(&rig.device, &rig.queue, &rig.gpu, &camera, &light);
    // Material 3 becomes 4 in a slab of the wall ahead, left of the axis.
    world
        .apply(&EditOp::box_(
            Fixed3([30 * FIXED_ONE, 2 * FIXED_ONE, -10 * FIXED_ONE]),
            Fixed3([64 * FIXED_ONE, 12 * FIXED_ONE, 10 * FIXED_ONE]),
            Mode::Replace { from: 3, to: 4 },
        ))
        .unwrap();
    rig.gpu.sync_now(&world, &rig.device, &rig.queue).unwrap();
    let (_, right, _) = camera.basis();
    camera.position += right;
    rig.frame
        .step(&rig.device, &rig.queue, &rig.gpu, &camera, &light);
    let f = read_frame(&rig.device, &rig.queue, &rig.frame);
    let (mut same, mut kept, mut changed) = (0, 0, 0);
    for i in 0..f.pos.len() {
        if f.info[i][3] & 1 == 0 {
            continue;
        }
        match f.info[i][2] {
            3 => {
                same += 1;
                kept += usize::from(f.age[i] >= 1.0);
            }
            4 => {
                changed += 1;
                assert_eq!(f.age[i], 0.0, "pixel {i} changed material");
            }
            m => panic!("material {m}"),
        }
    }
    let share = kept as f64 / same as f64;
    println!(
        "sideways step: {kept} of {same} unchanged pixels keep history ({:.2} %), {changed} changed pixels reset",
        share * 100.0
    );
    assert!(changed > 0);
    assert!(share >= 0.95, "{:.2} %", share * 100.0);
}

/// Phase 3's closed box in chunk 0 of a rock row of chunks `[0, 5)` along x, plus a flat
/// `glow` wall of 64 x 64 voxels at x = 257 (chunk 4, whose box lies 224 voxels from the
/// camera, inside the light radius; one voxel in from the chunk's face, so its -x faces
/// against rock are not counted exposed) facing +x into air, hidden from the box by rock.
fn box_and_far_wall(table: &MaterialTable, glow: MaterialId) -> World {
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [5, 1, 1]),
        &UniformSource(flythrough::cave::ROCK),
        table,
    );
    world
        .apply(&box_op([22, 22, 20], [42, 42, 30], Mode::Carve))
        .unwrap();
    let ceiling = Mode::Replace {
        from: flythrough::cave::ROCK,
        to: glow,
    };
    world
        .apply(&box_op([22, 22, 30], [42, 42, 31], ceiling))
        .unwrap();
    world
        .apply(&box_op([258, 0, 0], [320, 64, 64], Mode::Carve))
        .unwrap();
    world
        .apply(&box_op([257, 0, 0], [258, 64, 64], ceiling))
        .unwrap();
    world
}

/// The floor centre's irradiance over 64 frames, one read per frame, lamp off, no filter.
fn floor_centre_series(
    device: &vxe_render::wgpu::Device,
    queue: &vxe_render::wgpu::Queue,
    gpu: &GpuWorld,
    draw: vxe_render::CandidateDraw,
) -> Vec<f64> {
    let camera = vxe_render::Camera {
        position: [32.0, 32.0, 29.0].into(),
        yaw: 0.0,
        pitch: -std::f32::consts::FRAC_PI_2,
        vfov: 0.5,
    };
    let mut frame = vxe_render::Frame::new(device, 1, 1);
    frame.set_lighting(vxe_render::Lighting {
        lamp: false,
        draw,
        ..vxe_render::Lighting::default()
    });
    (0..64)
        .map(|i| {
            frame.set_frame_index(i);
            frame.render(
                device,
                queue,
                gpu,
                &camera,
                &vxe_render::LightParams::headlamp(),
            );
            f64::from(readback::read_rgba16f(device, queue, frame.irradiance())[0][0])
        })
        .collect()
}

/// Near emitters win, unbiased inside the radius: with a far emissive wall of 4096 samples
/// in the light radius beside the box's 400-sample ceiling, the floor centre's mean over 64
/// frames is within 5 % of 1.741, and one frame's relative RMSE against 1.741 is at most half
/// of the uniform draw's on the same scene.
#[test]
fn the_candidate_table_favours_near_emitters_without_bias() {
    let Some((device, queue)) = software_device(64 << 20) else {
        return;
    };
    let (table, glow, _) = glow_table();
    let world = box_and_far_wall(&table, glow);
    let mut gpu = GpuWorld::with_config(&device, MirrorConfig::new(Window::Fixed(world.extent())));
    gpu.set_focus(VoxelCoord([32, 32, 25]));
    gpu.sync_now(&world, &device, &queue).unwrap();
    let chunks: Vec<_> = gpu.light_chunks().collect();
    println!("light chunks: {chunks:?}");
    assert_eq!(gpu.light_samples(), 400 + 4096);
    let analytic = std::f64::consts::PI * 4.0 * 0.13853;
    let stats = |v: &[f64]| {
        let mean = v.iter().sum::<f64>() / v.len() as f64;
        let rmse = (v.iter().map(|x| (x - analytic).powi(2)).sum::<f64>() / v.len() as f64).sqrt()
            / analytic;
        (mean, rmse)
    };
    let (mean, rmse) = stats(&floor_centre_series(
        &device,
        &queue,
        &gpu,
        vxe_render::CandidateDraw::Table,
    ));
    let (umean, urmse) = stats(&floor_centre_series(
        &device,
        &queue,
        &gpu,
        vxe_render::CandidateDraw::Uniform,
    ));
    println!(
        "table: mean {mean:.4} ({:+.2} %), relative RMSE {rmse:.4}; uniform: mean {umean:.4}, relative RMSE {urmse:.4}; ratio {:.3}",
        (mean / analytic - 1.0) * 100.0,
        rmse / urmse
    );
    assert!((mean / analytic - 1.0).abs() <= 0.05, "{mean:.4}");
    assert!(rmse <= 0.5 * urmse, "{rmse:.4} against {urmse:.4}");
}

/// Eight chunks in a row along x, each with 2048 exposed emissive voxels (the z = 0 layer's
/// half y < 32).
struct HalfLayers(MaterialId);

impl WorldSource for HalfLayers {
    fn chunk(&self, _coord: ChunkCoord, out: &mut Chunk) {
        for slot in 0..CHUNK_BRICKS {
            let b = brick_of_slot(slot);
            out.set_brick(
                slot,
                Brick::from_fn(|l| {
                    let v = [0, 1, 2].map(|k| b[k] * BRICK_EDGE + l[k]);
                    if v[2] == 0 && v[1] < 32 { self.0 } else { AIR }
                }),
            );
        }
    }
}

/// The pool stays bounded and nothing is dropped silently: a mirror reserving 4096 samples
/// over eight chunks of 2048, light radius 2. From chunk 7 the two nearest chunks hold their
/// samples in full and the third is refused; moving to chunk 4 evicts the farthest ranges
/// first (7, then 6) for 4 and 3; moving to chunk 0 releases chunk 4 (beyond the radius plus
/// one) on that sync; and with a budget of one chunk per sync, chunks 0 and 1 have their
/// ranges on the first and second syncs, nearest first.
#[test]
fn light_ranges_follow_the_radius_and_evict_the_farthest() {
    let Some((device, queue)) = software_device(64 << 20) else {
        return;
    };
    let (table, glow, _) = glow_table();
    let extent = ChunkExtent::new([0, 0, 0], [8, 1, 1]);
    let world = World::generate(extent, &HalfLayers(glow), &table);
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            light_samples: 4096,
            light_radius: 2,
            light_sync_samples: 2048,
            ..MirrorConfig::new(Window::Fixed(extent))
        },
    );
    let ranged = |gpu: &GpuWorld| {
        let mut v: Vec<i32> = gpu.light_chunks().map(|(c, ..)| c.0[0]).collect();
        v.sort_unstable();
        v
    };
    let at = |x: i32| VoxelCoord([x * 64 + 32, 32, 32]);
    gpu.set_focus(at(7));
    let st = gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(ranged(&gpu), [7]);
    assert_eq!(st.light_oldest_wait, 0);
    let st = gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(ranged(&gpu), [6, 7]);
    assert_eq!(st.light_oldest_wait, 1, "chunk 5 has waited one sync");
    assert_eq!(gpu.light_samples(), 4096);
    let st2 = gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(ranged(&gpu), [6, 7]);
    assert_eq!(
        st2.light_refused, 1,
        "chunk 5 is refused: nothing farther to evict"
    );
    let _ = st;

    gpu.set_focus(at(4));
    let mut evicted = 0;
    for _ in 0..4 {
        evicted += gpu.sync_now(&world, &device, &queue).unwrap().light_evicted;
    }
    assert_eq!(ranged(&gpu), [3, 4]);
    assert_eq!(evicted, 2, "7, then 6");

    gpu.set_focus(at(0));
    gpu.sync_now(&world, &device, &queue).unwrap();
    let r = ranged(&gpu);
    println!("from chunk 0, one sync: {r:?}");
    assert!(!r.contains(&4), "chunk 4 lies beyond the radius plus one");
    // One chunk per sync, nearest first: chunk 0 had its range on that sync, chunk 1 (one
    // nearer chunk waiting before it) has it on the next, evicting chunk 3.
    assert!(r.contains(&0), "{r:?}");
    gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(ranged(&gpu), [0, 1]);
}

/// Phase 3's closed box (interior x and y `[22, 42)`, z `[20, 30)`, rock all round) with a
/// vein in its +y wall instead of an emissive ceiling: a staircase band of `glow` in the rock
/// layer y = 42, x `[24, 40)`, two voxels tall rising one voxel every four, seen from inside
/// the box looking +x with the lamp off, no filter.
fn vein_rig(sampler: vxe_render::Sampler) -> Option<(LookRig, vxe_render::Camera, MaterialId)> {
    let (table, glow, _) = glow_table();
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [1, 1, 1]),
        &UniformSource(flythrough::cave::ROCK),
        &table,
    );
    world
        .apply(&box_op([22, 22, 20], [42, 42, 30], Mode::Carve))
        .unwrap();
    for x in 24..40 {
        let z = 22 + (x - 24) / 4;
        world
            .apply(&box_op(
                [x, 42, z],
                [x + 1, 43, z + 2],
                Mode::Replace {
                    from: flythrough::cave::ROCK,
                    to: glow,
                },
            ))
            .unwrap();
    }
    let mut rig = LookRig::new(&world, None)?;
    rig.frame.set_lighting(vxe_render::Lighting {
        lamp: false,
        filter: false,
        sampler,
        ..vxe_render::Lighting::default()
    });
    let camera = vxe_render::Camera {
        position: [23.0, 28.0, 25.0].into(),
        yaw: 0.35,
        pitch: -0.2,
        vfov: 70f32.to_radians(),
    };
    Some((rig, camera, glow))
}

/// The luminance of each pixel of `frame`'s raw irradiance.
fn irradiance_lum(rig: &LookRig) -> Vec<f64> {
    readback::read_rgba16f(&rig.device, &rig.queue, rig.frame.irradiance())
        .iter()
        .map(|e| 0.2126 * f64::from(e[0]) + 0.7152 * f64::from(e[1]) + 0.0722 * f64::from(e[2]))
        .collect()
}

/// Reservoir reuse against the flat list on the box with a vein at 320 x 180: one frame's
/// relative RMSE of the irradiance (over the hit pixels that do not show the vein, whose own
/// faces light each other at contact) against a 1024-frame average of the flat list is lower
/// with reuse on (after 16 frames of history) than with it off, at one shadow ray per pixel.
#[test]
fn reservoir_reuse_lowers_one_frames_error() {
    let Some((list, camera, glow)) = vein_rig(vxe_render::Sampler::List) else {
        return;
    };
    let light = vxe_render::LightParams::headlamp();
    let mut list = list;
    let mut reference = vec![0.0f64; (WIDTH * HEIGHT) as usize];
    let frames = 1024;
    for i in 0..frames {
        list.frame.set_frame_index(i);
        list.frame
            .render(&list.device, &list.queue, &list.gpu, &camera, &light);
        for (r, v) in reference.iter_mut().zip(irradiance_lum(&list)) {
            *r += v / f64::from(frames);
        }
    }
    let info = readback::read_rgba16u(&list.device, &list.queue, list.frame.hit_records());
    let lit: Vec<usize> = (0..info.len())
        .filter(|&i| info[i][3] & 1 == 1 && info[i][2] != glow)
        .collect();
    let mean_ref = lit.iter().map(|&i| reference[i]).sum::<f64>() / lit.len() as f64;
    let rel_rmse = |x: &[f64]| {
        (lit.iter()
            .map(|&i| (x[i] - reference[i]).powi(2))
            .sum::<f64>()
            / lit.len() as f64)
            .sqrt()
            / mean_ref
    };
    list.frame.set_frame_index(frames + 16);
    list.frame
        .render(&list.device, &list.queue, &list.gpu, &camera, &light);
    let off = rel_rmse(&irradiance_lum(&list));

    let (mut reuse, _, _) = vein_rig(vxe_render::Sampler::Reuse).unwrap();
    reuse.frame.set_frame_index(frames);
    for _ in 0..=16 {
        reuse
            .frame
            .step(&reuse.device, &reuse.queue, &reuse.gpu, &camera, &light);
    }
    let on = rel_rmse(&irradiance_lum(&reuse));
    println!(
        "reference mean irradiance {mean_ref:.4} over {frames} frames and {} pixels; one frame's relative RMSE: list {off:.4}, reuse {on:.4}, ratio {:.3}",
        lit.len(),
        on / off
    );
    assert!(on < off, "{on:.4} against {off:.4}");
}

/// The dark cave of `golden.rs` (the golden scene with its carve, the ore emissive, the pose
/// facing a vein) with the lamp on and `history`, settled over 32 frames.
fn lamp_cave(history: vxe_render::HistoryKill) -> Option<(LookRig, vxe_render::Camera)> {
    let source = scene_source();
    let world = World::generate(
        source.extent,
        &source,
        &flythrough::with_emissive_ore(&cave_materials()),
    );
    world.apply(&scene_carve(&world)).unwrap();
    let mut rig = LookRig::new(&world, None)?;
    rig.gpu.set_focus(VoxelCoord([264, 120, 376]));
    for _ in 0..4 {
        rig.gpu.sync_now(&world, &rig.device, &rig.queue).unwrap();
    }
    rig.frame.set_lighting(vxe_render::Lighting {
        history,
        ..vxe_render::Lighting::default()
    });
    let camera = vxe_render::Camera {
        position: [264.5, 120.5, 376.5].into(),
        yaw: 15f32.to_radians(),
        pitch: 0.0,
        vfov: 70f32.to_radians(),
    };
    for _ in 0..32 {
        rig.frame.step(
            &rig.device,
            &rig.queue,
            &rig.gpu,
            &camera,
            &vxe_render::LightParams::headlamp(),
        );
    }
    Some((rig, camera))
}

/// The luminance of `frame`'s filtered irradiance per pixel: what the composite shades with.
fn accumulated_lum(rig: &LookRig) -> Vec<f64> {
    readback::read_rgba16f(&rig.device, &rig.queue, rig.frame.filtered())
        .iter()
        .map(|e| 0.2126 * f64::from(e[0]) + 0.7152 * f64::from(e[1]) + 0.0722 * f64::from(e[2]))
        .collect()
}

/// A static camera in the dark cave and the headlamp's cone turned 30 degrees in one frame:
/// in the gradient mode every pixel that left the cone (its filtered irradiance at 0.5 or
/// more before, under 5 % of that 40 frames after) falls under 10 % of its lit value within 2
/// frames, where plain accumulation at alpha 0.2 needs about 11 (0.8^11 = 0.086); the frames
/// each mode takes go in the log.
#[test]
fn the_gradient_drops_light_that_left_the_cone_within_two_frames() {
    let turned = vxe_render::LightParams {
        turn: 30f32.to_radians(),
        ..vxe_render::LightParams::headlamp()
    };
    let mut frames_to_drop = Vec::new();
    for history in [
        vxe_render::HistoryKill::Gradient,
        vxe_render::HistoryKill::Edits,
    ] {
        let Some((mut rig, camera)) = lamp_cave(history) else {
            return;
        };
        let before = accumulated_lum(&rig);
        let mut after = Vec::new();
        for _ in 0..40 {
            rig.frame
                .step(&rig.device, &rig.queue, &rig.gpu, &camera, &turned);
            after.push(accumulated_lum(&rig));
        }
        let settled = &after[39];
        let left: Vec<usize> = (0..before.len())
            .filter(|&i| before[i] >= 0.5 && settled[i] < 0.05 * before[i])
            .collect();
        // The first frame after the turn by which every such pixel reads under 10 % of before.
        let k = (0..40)
            .find(|&k| left.iter().all(|&i| after[k][i] < 0.1 * before[i]))
            .map_or(41, |k| k + 1);
        println!(
            "{history:?}: {} pixels left the cone; all under 10 % after {k} frames",
            left.len()
        );
        let share: Vec<String> = (0..12)
            .map(|k| {
                let n = left
                    .iter()
                    .filter(|&&i| after[k][i] < 0.1 * before[i])
                    .count();
                format!("{:.3}", n as f64 / left.len() as f64)
            })
            .collect();
        println!(
            "{history:?}: share under 10 % after 1..12 frames: {}",
            share.join(" ")
        );
        assert!(!left.is_empty());
        frames_to_drop.push(k);
    }
    assert!(
        frames_to_drop[0] <= 2,
        "gradient: {} frames",
        frames_to_drop[0]
    );
}
