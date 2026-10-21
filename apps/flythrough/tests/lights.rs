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
