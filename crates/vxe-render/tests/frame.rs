//! What a `Frame` holds and decides between encodes: the probe volume exists only once
//! indirect light needs it, and the lamp an encode compares is the one its `Params` carry.

use vxe_render::{Camera, Frame, GpuWorld, LightParams, Lighting, glam::Vec3, wgpu};
use vxe_world::*;

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        force_fallback_adapter: true,
        ..Default::default()
    }))
    .or_else(|_| pollster::block_on(instance.request_adapter(&Default::default())));
    let Ok(adapter) = adapter else {
        println!("skipped: no adapter");
        return None;
    };
    let desc = wgpu::DeviceDescriptor {
        required_limits: vxe_render::limits_for(&adapter, 16 << 20).expect("the frame's limits"),
        ..Default::default()
    };
    Some(pollster::block_on(adapter.request_device(&desc)).expect("device"))
}

/// One chunk, rock below z = 32 and air above, mirrored.
fn mirrored(device: &wgpu::Device, queue: &wgpu::Queue) -> GpuWorld {
    struct Floor;
    impl WorldSource for Floor {
        fn chunk(&self, coord: ChunkCoord, out: &mut Chunk) {
            let base = coord.min_voxel().0;
            for slot in 0..CHUNK_BRICKS {
                let b = brick_of_slot(slot);
                out.set_brick(
                    slot,
                    Brick::from_fn(|l| {
                        if base[2] + b[2] * BRICK_EDGE + l[2] < 32 {
                            1
                        } else {
                            AIR
                        }
                    }),
                );
            }
        }
    }
    let mut t = MaterialTable::new();
    t.push(Material::named("rock"));
    let world = World::generate(ChunkExtent::new([0, 0, 0], [1, 1, 1]), &Floor, &t);
    let mut gpu = GpuWorld::new(device, world.extent());
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    gpu.sync(&world, queue, &mut enc).expect("sync");
    queue.submit([enc.finish()]);
    gpu
}

fn camera() -> Camera {
    Camera {
        position: Vec3::new(32.0, 32.0, 48.0),
        yaw: 0.0,
        pitch: -0.5,
        vfov: 1.0,
    }
}

/// A frame with indirect light off (the default) holds no probe volume, before any encode
/// and after one; the first encode with indirect light on builds it, and `frame_bytes` grows
/// by exactly the volume: its atlases, state and scratch (12 615 680 + 1 056 768 words,
/// 54 689 792 B) and its 32-byte uniform.
#[test]
fn the_probe_volume_is_built_on_the_first_encode_with_indirect_light() {
    let Some((device, queue)) = device() else {
        return;
    };
    let gpu = mirrored(&device, &queue);
    let mut frame = Frame::new(&device, 64, 64);
    assert!(
        !frame.lighting().indirect,
        "indirect light is off by default"
    );
    let before = frame.frame_bytes();
    frame.step(&device, &queue, &gpu, &camera(), &LightParams::headlamp());
    assert_eq!(
        frame.frame_bytes(),
        before,
        "an encode without indirect light"
    );
    frame.set_lighting(Lighting {
        indirect: true,
        ..frame.lighting()
    });
    frame.step(&device, &queue, &gpu, &camera(), &LightParams::headlamp());
    let uniform = size_of::<vxe_render::probes::ProbeUniform>() as u64;
    assert_eq!(uniform, 32);
    let grown = frame.frame_bytes() - before;
    println!(
        "frame_bytes {before} B with indirect off, {} B after an encode with it on (+{grown} B)",
        frame.frame_bytes()
    );
    assert_eq!(grown, 54_689_792 + uniform);
    assert_eq!(grown, frame.probes().bytes());
}

/// The lamp an encode compares against the last one's is the lamp of the `Params` it is
/// given: building a second `Params` for another lamp in between changes nothing.
#[test]
fn a_params_built_between_encodes_does_not_move_the_lamp_the_encode_compares() {
    let Some((device, queue)) = device() else {
        return;
    };
    let gpu = mirrored(&device, &queue);
    let mut frame = Frame::new(&device, 16, 16);
    let a = LightParams::headlamp();
    let b = LightParams {
        intensity: 2.0 * a.intensity,
        ..a
    };
    let encode = |frame: &Frame, p: &vxe_render::marcher::Params| {
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        frame.encode(&device, &queue, &mut enc, &gpu, p, None);
        queue.submit([enc.finish()]);
    };
    frame.step(&device, &queue, &gpu, &camera(), &a);
    frame.advance();
    let p = frame.params(&gpu, &camera(), &a);
    let _other = frame.params(&gpu, &camera(), &b);
    encode(&frame, &p);
    assert!(!frame.lamp_changed(), "lamp a after lamp a");
    frame.advance();
    let p = frame.params(&gpu, &camera(), &b);
    encode(&frame, &p);
    assert!(frame.lamp_changed(), "lamp b after lamp a");
    frame.advance();
    let p = frame.params(&gpu, &camera(), &b);
    let _other = frame.params(&gpu, &camera(), &a);
    encode(&frame, &p);
    assert!(!frame.lamp_changed(), "lamp b after lamp b");
}
