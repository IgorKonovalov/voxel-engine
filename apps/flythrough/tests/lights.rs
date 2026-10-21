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
/// non-emissive twin leaves 0; and after the chunk leaves the mirror its range is free and
/// zeroed and `light_samples` reads 0.
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
    let st = gpu.sync(&world, &queue).unwrap();
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
    gpu.sync(&world, &queue).unwrap();
    assert_eq!(gpu.light_samples(), 55);

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
    gpu.sync(&world, &queue).unwrap();
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
    gpu.sync(&world, &queue).unwrap();
    let range = gpu.light_range(coord).expect("the chunk has a range again");
    assert_eq!(range.1, 55);
    gpu.apply_events(&StreamEvents {
        evicted: vec![coord],
        ..StreamEvents::default()
    });
    assert_eq!(gpu.light_samples(), 0);
    assert_eq!(gpu.light_range(coord), None);
    let _ = gpu.sync(&world, &queue);
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
    uncapped.sync(&world, &queue).unwrap();
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
    gpu.sync(&world, &queue).unwrap();
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
    gpu.sync(&world, &queue).unwrap();
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
