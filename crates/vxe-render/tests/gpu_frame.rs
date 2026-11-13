//! What a `Frame` holds and decides between encodes: the probe volume exists only once
//! the lamp's bounce needs it, the lamp an encode compares is the one its `Params` carry, and
//! the pass timers and the store's counters read back without waiting on the GPU.

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

/// As [`device`], with `TIMESTAMP_QUERY` where the adapter has it.
fn timed_device() -> Option<(wgpu::Device, wgpu::Queue)> {
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
        required_features: adapter.features() & wgpu::Features::TIMESTAMP_QUERY,
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

/// A frame with the lamp's bounce off (the default) holds no probe volume, before any encode
/// and after one; the first encode with it on (the lamp on) builds it, and `frame_bytes` grows
/// by exactly the volume at its default side of 8 probes (the lamp's reach of 64 voxels at the
/// spacing of 16), 5 015 600 B: its atlases and state (512 x 385 words, 788 480 B), its scratch
/// (8192 + 8192 x 128 words, 4 227 072 B) and its 48-byte uniform.
#[test]
fn the_probe_volume_is_built_on_the_first_encode_with_the_lamp_bounce() {
    let Some((device, queue)) = device() else {
        return;
    };
    let gpu = mirrored(&device, &queue);
    let mut frame = Frame::new(&device, 64, 64);
    assert!(
        !frame.lighting().lamp_bounce,
        "the lamp's bounce is off by default"
    );
    let before = frame.frame_bytes();
    frame.step(&device, &queue, &gpu, &camera(), &LightParams::headlamp());
    assert_eq!(
        frame.frame_bytes(),
        before,
        "an encode without the lamp's bounce"
    );
    frame.set_lighting(Lighting {
        lamp_bounce: true,
        ..frame.lighting()
    });
    frame.step(&device, &queue, &gpu, &camera(), &LightParams::headlamp());
    let uniform = size_of::<vxe_render::internals::ProbeUniform>() as u64;
    assert_eq!(uniform, 48);
    let grown = frame.frame_bytes() - before;
    println!(
        "frame_bytes {before} B with the lamp's bounce off, {} B after an encode with it on (+{grown} B)",
        frame.frame_bytes()
    );
    assert_eq!(frame.probes().axis(), 8);
    assert_eq!(grown, 5_015_600);
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
    let encode = |frame: &Frame, p: &vxe_render::Params| {
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

/// Five frames, each encoded with the frame's pass timer and a one-pass timer (the HUD's and
/// the field pass's form) resolved and the store's counters copied, submitted, then read with
/// `try_read_all_ms`, `try_read_ms` and `try_store_stats`: the reads are `None` until a set has
/// finished, then per-pass values (one per `FRAME_PASSES` entry, finite, not all zero) and the
/// store's counts at most 2 frames old; across all of it neither timer nor the store counts a
/// blocking read. One `read_all_ms` after counts exactly one, and the store's non-blocking read
/// then equals its blocking one.
#[test]
fn the_frame_s_timers_and_store_counts_read_back_without_waiting() {
    let Some((device, queue)) = timed_device() else {
        return;
    };
    let passes = vxe_render::FRAME_PASSES.len() as u32;
    let (Some(timer), Some(one)) = (
        vxe_render::PassTimer::with_passes(&device, &queue, passes),
        vxe_render::PassTimer::new(&device, &queue),
    ) else {
        println!("skipped: the adapter has no TIMESTAMP_QUERY");
        return;
    };
    let gpu = mirrored(&device, &queue);
    let mut frame = Frame::new(&device, 16, 16);
    frame.set_count_store(true);
    let light = LightParams::headlamp();
    let mut sets = Vec::new();
    let mut ones = Vec::new();
    let mut stores = Vec::new();
    for _ in 0..5 {
        frame.advance();
        let p = frame.params(&gpu, &camera(), &light);
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        frame.encode(&device, &queue, &mut enc, &gpu, &p, Some(&timer));
        timer.resolve(&mut enc);
        {
            // An empty compute pass timed as the one-pass timer's pass 0.
            let _pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("one-pass timer"),
                timestamp_writes: Some(one.writes()),
            });
        }
        one.resolve(&mut enc);
        frame.copy_store_stats(&mut enc);
        queue.submit([enc.finish()]);
        sets.push(timer.try_read_all_ms(&device));
        ones.push(one.try_read_ms(&device));
        stores.push(frame.try_store_stats(&device));
    }
    println!("sets {sets:?}\none-pass {ones:?}\nstore {stores:?}");
    let first = sets
        .iter()
        .position(Option::is_some)
        .expect("a set finishes within 5 frames");
    assert!(sets[..first].iter().all(Option::is_none));
    for set in sets.iter().flatten() {
        assert_eq!(set.len(), passes as usize, "one value per pass");
        assert!(set.iter().all(|ms| ms.is_finite() && *ms >= 0.0), "{set:?}");
        assert!(
            set.iter().sum::<f64>() > 0.0,
            "the passes took time: {set:?}"
        );
    }
    assert!(
        ones.iter().any(Option::is_some),
        "the one-pass timer read a set"
    );
    let store_first = stores
        .iter()
        .position(Option::is_some)
        .expect("the store's counts finish within 5 frames");
    assert!(stores[..store_first].iter().all(Option::is_none));
    for (_, age) in stores.iter().flatten() {
        assert!(*age <= 2, "the store's counts {age} frames old");
    }
    assert_eq!(timer.blocking_reads(), 0);
    assert_eq!(one.blocking_reads(), 0);
    assert_eq!(frame.store_blocking_reads(), 0);
    let waited = timer.read_all_ms(&device);
    assert_eq!(waited.len(), passes as usize);
    assert_eq!(timer.blocking_reads(), 1, "a read that waits counts once");
    let blocking = frame.read_store_stats(&device, &queue);
    assert_eq!(frame.store_blocking_reads(), 1);
    let (latest, age) = frame
        .try_store_stats(&device)
        .expect("the store's counts were read back");
    assert_eq!(age, 0, "the GPU is idle: the last copy is in");
    assert_eq!(latest, blocking);
}
