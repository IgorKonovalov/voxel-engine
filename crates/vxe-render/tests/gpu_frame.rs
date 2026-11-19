//! What a `Frame` holds and decides between encodes: the probe volume exists only once
//! the lamp's bounce needs it, the lamp an encode compares is the one its `Params` carry, and
//! the pass timers and the store's counters read back without waiting on the GPU.

use vxe_render::{Camera, Frame, GpuWorld, LightParams, Lighting, glam::Vec3, wgpu};
use vxe_world::*;

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    vxe_render::test_support::any_device(16 << 20, wgpu::Features::empty())
}

/// As [`device`], with `TIMESTAMP_QUERY` where the adapter has it.
fn timed_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    vxe_render::test_support::any_device(16 << 20, wgpu::Features::TIMESTAMP_QUERY)
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

/// The lighting modes the bind-group tests cycle: the default, the filter off, two a-trous
/// iterations, the gradient's history kill, the lamp's bounce (the probe passes) and the lamp
/// off.
fn lighting_modes(base: Lighting) -> [(&'static str, Lighting); 6] {
    [
        ("default", base),
        (
            "filter off",
            Lighting {
                filter: false,
                ..base
            },
        ),
        (
            "two a-trous iterations",
            Lighting {
                atrous_iterations: 2,
                ..base
            },
        ),
        (
            "gradient",
            Lighting {
                history: vxe_render::HistoryKill::Gradient,
                ..base
            },
        ),
        (
            "lamp bounce",
            Lighting {
                lamp_bounce: true,
                ..base
            },
        ),
        (
            "lamp off",
            Lighting {
                lamp: false,
                ..base
            },
        ),
    ]
}

/// The frame's counts of bind groups and views created so far.
fn built(frame: &Frame) -> (u64, u64) {
    (frame.bind_groups_built(), frame.views_built())
}

/// Under each upscaler (none, the engine's own, FSR 2) and in each lighting mode, the two
/// frames after a switch may build the groups of both history parities; the two after those
/// build no bind group and no view.
#[test]
fn a_frame_after_the_second_builds_no_bind_group() {
    let Some((device, queue)) = device() else {
        return;
    };
    let gpu = mirrored(&device, &queue);
    let mut frame = Frame::with_sizes(&device, [48, 32], [64, 48]);
    let base = frame.lighting();
    let light = LightParams::headlamp();
    for kind in [
        vxe_render::UpscalerKind::Off,
        vxe_render::UpscalerKind::Own(vxe_render::Upscaler::default()),
        vxe_render::UpscalerKind::Fsr2(vxe_render::Fsr2Settings::default()),
    ] {
        frame.set_upscaler_kind(&device, kind);
        for (mode, lighting) in lighting_modes(base) {
            frame.set_lighting(lighting);
            for _ in 0..2 {
                frame.step(&device, &queue, &gpu, &camera(), &light);
            }
            let warm = built(&frame);
            for _ in 0..2 {
                frame.step(&device, &queue, &gpu, &camera(), &light);
            }
            let after = built(&frame);
            println!(
                "{} / {mode}: {} groups and {} views built so far",
                kind.name(),
                warm.0,
                warm.1
            );
            assert_eq!(
                after,
                warm,
                "{} / {mode}: frames 3 and 4 built (groups, views)",
                kind.name()
            );
        }
    }
}

/// A resize builds the groups of both parities once: the two frames after it build what a
/// new frame's first two build, and the next two build nothing.
#[test]
fn a_resize_builds_both_parities_once() {
    let Some((device, queue)) = device() else {
        return;
    };
    let gpu = mirrored(&device, &queue);
    let light = LightParams::headlamp();
    let mut frame = Frame::new(&device, 32, 32);
    let steps = |frame: &mut Frame, n: usize| {
        let before = built(frame);
        for _ in 0..n {
            frame.step(&device, &queue, &gpu, &camera(), &light);
        }
        let after = built(frame);
        (after.0 - before.0, after.1 - before.1)
    };
    let first = steps(&mut frame, 2);
    println!("a new frame's first two encodes: {first:?} (groups, views)");
    assert!(first.0 > 0 && first.1 > 0, "{first:?}");
    assert_eq!(steps(&mut frame, 2), (0, 0), "frames 3 and 4");
    frame.resize(&device, 48, 40);
    assert_eq!(
        steps(&mut frame, 2),
        first,
        "the two encodes after a resize"
    );
    assert_eq!(
        steps(&mut frame, 2),
        (0, 0),
        "frames 3 and 4 after a resize"
    );
}

/// The test devices' pipeline cache skips a software adapter: no cache file is named for it,
/// none is open after its device opens, the renderer builds through no cache, and a write-back
/// writes nothing (Plan 0094 Phase 2a bar 4, the gate's half; a software driver's cache holds
/// no compiled code, 84 B of data on llvmpipe).
#[test]
fn a_software_adapter_opens_no_pipeline_cache() {
    use vxe_render::test_support::{
        device_on, open_pipeline_cache_file, pipeline_cache_file, pipeline_cache_root,
        software_adapter, write_back_pipeline_cache,
    };
    let Some(adapter) = software_adapter() else {
        return;
    };
    let key = wgpu::util::pipeline_cache_key(&adapter.get_info());
    let dir = pipeline_cache_root().zip(key).map(|(r, k)| r.join(k));
    let files = || {
        dir.as_ref()
            .and_then(|d| std::fs::read_dir(d).ok())
            .map_or(0, Iterator::count)
    };
    let before = files();
    assert_eq!(pipeline_cache_file(&adapter), None);
    let (_device, _queue) = device_on(&adapter, 16 << 20, wgpu::Features::empty());
    assert_eq!(open_pipeline_cache_file(), None);
    assert!(vxe_render::debug::pipeline_cache_data().is_none());
    write_back_pipeline_cache();
    assert_eq!(files(), before, "files under {dir:?}");
}

/// The test devices' pipeline cache pays on a hardware adapter (Plan 0094 Phase 2a bar 4, the
/// hardware half): with the driver's own shader disk cache off, a frame's pipelines
/// (`Frame::with_sizes` at 853 x 480 to 1280 x 720 and `prepare_entities`) built cold with the
/// cache file deleted, then warm on a second device from the file the first wrote back: the
/// warm build is at most a quarter of the cold (the RTX 3080 Laptop read 0.01). A truncated
/// file is then a cold start that builds every pipeline, its wall printed. About a minute on
/// the RTX; skipped where no hardware adapter is found.
#[test]
#[ignore = "soak"]
#[allow(
    clippy::disallowed_methods,
    reason = "the cache's bar is a ratio of build walls"
)]
fn the_pipeline_cache_pays_on_a_hardware_adapter() {
    use vxe_render::test_support::{
        device_on, high_performance_adapter, open_pipeline_cache_file, pipeline_cache_file,
        write_back_pipeline_cache,
    };
    // SAFETY: set before this process loads a driver, on the test's only thread that reads
    // the environment; nextest runs each test in a process of its own.
    unsafe {
        std::env::set_var("__GL_SHADER_DISK_CACHE", "0");
        std::env::set_var("MESA_SHADER_CACHE_DISABLE", "true");
    }
    let Some(adapter) = high_performance_adapter() else {
        return;
    };
    let name = adapter.get_info().name;
    let Some(file) = pipeline_cache_file(&adapter) else {
        println!("skipped: {name} keeps no pipeline cache");
        return;
    };
    let build = || {
        let (device, _q) = device_on(&adapter, 16 << 20, wgpu::Features::empty());
        assert_eq!(open_pipeline_cache_file().as_ref(), Some(&file));
        let t = std::time::Instant::now();
        let f = Frame::with_sizes(&device, [853, 480], [1280, 720]);
        f.prepare_entities(&device);
        let ms = t.elapsed().as_secs_f64() * 1e3;
        write_back_pipeline_cache();
        ms
    };
    let _ = std::fs::remove_file(&file);
    let cold = build();
    let bytes = std::fs::metadata(&file).map_or(0, |m| m.len());
    let warm = build();
    let data = std::fs::read(&file).expect("the written cache");
    std::fs::write(&file, &data[..data.len() / 2]).expect("truncate the cache");
    let truncated = build();
    println!(
        "pipeline cache on {name}: cold {cold:.0} ms, warm {warm:.0} ms ({:.3} of cold), \
         truncated file {truncated:.0} ms; {bytes} B written",
        warm / cold
    );
    assert!(bytes > 0, "the cold build wrote no cache to {file:?}");
    assert!(
        warm <= cold / 4.0,
        "warm {warm:.0} ms against cold {cold:.0} ms"
    );
}

/// Readings run by hand: `cargo nextest run -p vxe-render --run-ignored only -E
/// 'test(=reading::<name>)'`; the soak and full tiers leave this module out by its name.
mod reading {
    use super::*;

    /// Plan 0094 Phase 2a bar 4, as a reading first: on the software adapter and on the
    /// high-performance one, a frame's pipelines (`Frame::with_sizes` at 853 x 480 to 1280 x
    /// 720 and `prepare_entities`, the light pass's two default variants) built on a device
    /// under an empty pipeline cache (`debug::set_pipeline_cache`), the cache's data taken,
    /// then built again on a second device under a cache made from that data: both walls, the
    /// data's size and the warm over the cold, printed.
    #[test]
    #[ignore = "a reading"]
    #[allow(
        clippy::disallowed_methods,
        reason = "a reading of the pipelines' build time"
    )]
    fn the_pipeline_cache_pays() {
        use vxe_render::test_support::{
            device_on_uncached, high_performance_adapter, software_adapter,
        };
        for adapter in [software_adapter(), high_performance_adapter()]
            .into_iter()
            .flatten()
        {
            let name = adapter.get_info().name;
            if !adapter.features().contains(wgpu::Features::PIPELINE_CACHE) {
                println!("reading: pipeline cache on {name}: the adapter has no PIPELINE_CACHE");
                continue;
            }
            let build = |data: Option<&[u8]>| {
                let (device, _q) =
                    device_on_uncached(&adapter, 16 << 20, wgpu::Features::PIPELINE_CACHE);
                // SAFETY: the data is this process's own `get_data` from the same adapter, or none;
                // `fallback` takes an empty cache for data the driver refuses.
                let cache = unsafe {
                    device.create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
                        label: Some("reading"),
                        data,
                        fallback: true,
                    })
                };
                vxe_render::debug::set_pipeline_cache(Some(cache));
                let t = std::time::Instant::now();
                let f = Frame::with_sizes(&device, [853, 480], [1280, 720]);
                f.prepare_entities(&device);
                let ms = t.elapsed().as_secs_f64() * 1e3;
                let out = vxe_render::debug::pipeline_cache_data();
                vxe_render::debug::set_pipeline_cache(None);
                (ms, out)
            };
            let (cold, data) = build(None);
            let Some(data) = data else {
                println!(
                    "reading: pipeline cache on {name}: no data from the backend; cold {cold:.0} ms"
                );
                continue;
            };
            let (warm, _) = build(Some(&data));
            let (again, _) = build(None);
            println!(
                "reading: pipeline cache on {name}: cold {cold:.0} ms, warm {warm:.0} ms ({:.2} of cold), cold again {again:.0} ms; {} B of data",
                warm / cold,
                data.len()
            );
        }
    }

    /// The CPU time of `Frame::encode` at the fly-through's default sizes (853 x 480 for a
    /// 1280 x 720 image under FSR 2, half-resolution lighting) over the one-chunk floor, on the
    /// high-performance adapter (`VK_DRIVER_FILES` picks another): 300 encodes after 20, each
    /// submitted and waited on, so the CPU times the encode alone. Prints p50 and p99.
    #[test]
    #[ignore = "a CPU timing reading; run by hand"]
    #[allow(
        clippy::disallowed_methods,
        reason = "a reading of the encode's wall time on the CPU"
    )]
    fn encode_cpu_time_at_the_default_sizes() {
        let Some((device, queue, adapter)) =
            vxe_render::test_support::high_performance_device(16 << 20, wgpu::Features::empty())
        else {
            return;
        };
        let gpu = mirrored(&device, &queue);
        let mut frame = Frame::with_sizes(&device, [853, 480], [1280, 720]);
        frame.set_upscaler_kind(
            &device,
            vxe_render::UpscalerKind::Fsr2(vxe_render::Fsr2Settings::default()),
        );
        let light = LightParams::headlamp();
        let mut ms = Vec::new();
        for i in 0..320 {
            frame.advance();
            let params = frame.params(&gpu, &camera(), &light);
            let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            let t = std::time::Instant::now();
            frame.encode(&device, &queue, &mut enc, &gpu, &params, None);
            let dt = t.elapsed().as_secs_f64() * 1e3;
            queue.submit([enc.finish()]);
            device
                .poll(wgpu::PollType::wait_indefinitely())
                .expect("the frame finishes");
            if i >= 20 {
                ms.push(dt);
            }
        }
        ms.sort_by(f64::total_cmp);
        println!(
            "reading[{adapter}]: encode CPU p50 {:.4} ms, p99 {:.4} ms over {} encodes",
            ms[ms.len() / 2],
            ms[ms.len() * 99 / 100],
            ms.len()
        );
    }
}
