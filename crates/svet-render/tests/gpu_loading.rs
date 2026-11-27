//! The engine's loading frame on the software adapter (lavapipe): the bar fills to its
//! fraction, a second frame with new rows allocates nothing, and the frame matches its golden
//! (`tests/golden/loading_screen.png`; `SVET_BLESS=1` rewrites it).

use svet_render::{
    LoadingScreen, LoadingView, loading_bar_inner, loading_bar_rect, loading_layout, wgpu,
};

/// A device on the software Vulkan adapter, or `None` with a printed skip when the machine has
/// none. With `SVET_REQUIRE_SOFTWARE_ADAPTER=1` a missing adapter panics instead.
fn software_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        force_fallback_adapter: true,
        ..Default::default()
    }));
    let adapter = match adapter {
        Ok(a) if a.get_info().device_type == wgpu::DeviceType::Cpu => a,
        other => {
            let found = other.map_or_else(|e| format!("no adapter ({e})"), |a| a.get_info().name);
            if std::env::var("SVET_REQUIRE_SOFTWARE_ADAPTER").as_deref() == Ok("1") {
                panic!("SVET_REQUIRE_SOFTWARE_ADAPTER=1 but the fallback is {found}");
            }
            println!("skipped: no software adapter ({found})");
            return None;
        }
    };
    println!("adapter: {}", adapter.get_info().name);
    Some(
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .expect("device on the software adapter"),
    )
}

/// An `rgba8unorm` target of `size`.
fn target(device: &wgpu::Device, size: [u32; 2]) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("loading test target"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

fn view(progress: f32, stage: &str) -> LoadingView<'_> {
    LoadingView {
        title: "castle - benchmark",
        stage,
        detail: "31 / 49 chunks",
        progress,
        seconds: 0.4,
        alpha: 1.0,
    }
}

/// Encodes one loading frame onto `tex` and reads it back.
fn draw(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    screen: &mut LoadingScreen,
    tex: &wgpu::Texture,
    v: &LoadingView<'_>,
) -> Vec<u8> {
    let target = tex.create_view(&wgpu::TextureViewDescriptor::default());
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    screen.encode(queue, &mut enc, &target, [tex.width(), tex.height()], v);
    queue.submit([enc.finish()]);
    svet_render::debug::read_rgba8(device, queue, tex)
}

/// At 1280 x 720, progress 0.5, alpha 1: along the bar's inner middle row each pixel is
/// nearer the fill colour left of the inner rect's midpoint and nearer the empty colour right
/// of it, but for at most one pixel each side of the midpoint, the edge's ramp.
#[test]
fn the_bar_fills_to_its_fraction() {
    let Some((device, queue)) = software_device() else {
        return;
    };
    let size = [1280, 720];
    let tex = target(&device, size);
    let mut screen = LoadingScreen::new(&device, wgpu::TextureFormat::Rgba8Unorm, 2);
    let img = draw(&device, &queue, &mut screen, &tex, &view(0.5, "lighting"));
    let inner = loading_bar_inner(size);
    let (outer, _) = loading_bar_rect(size);
    println!("bar outer {outer:?}, inner {inner:?}");
    let y = ((inner[1] + inner[3]) * 0.5) as usize;
    let px = |x: usize| {
        let i = (y * size[0] as usize + x) * 4;
        [img[i], img[i + 1], img[i + 2]]
    };
    let x0 = inner[0].ceil() as usize;
    let x1 = inner[2].floor() as usize - 1;
    let fill = px(x0 + 2);
    let empty = px(x1 - 2);
    println!("fill {fill:?}, empty {empty:?}");
    let dist = |a: [u8; 3], b: [u8; 3]| {
        (0..3)
            .map(|k| (i32::from(a[k]) - i32::from(b[k])).abs())
            .sum::<i32>()
    };
    assert!(
        dist(fill, empty) > 100,
        "fill and empty differ: {fill:?} {empty:?}"
    );
    let mid = (inner[0] + inner[2]) * 0.5;
    let mut wrong = Vec::new();
    for x in x0..=x1 {
        let c = (x as f32) + 0.5;
        let filled = dist(px(x), fill) < dist(px(x), empty);
        if filled != (c < mid) && (c - mid).abs() > 1.0 {
            wrong.push(x);
        }
    }
    assert!(
        wrong.is_empty(),
        "pixels off their side of {mid}: {wrong:?}"
    );
}

/// The first frame after the device (stage `starting`, no count) and a stage frame (`lighting
/// the near chunks: 31 / 49 chunks`), both at progress 0 and `seconds` 0, at 640 x 360 and at
/// 1024 x 562 (the 1280 x 720 window and a tiled 2048 x 1124 one, halved): below the cube's
/// top (the text rows above it) the two frames are the same pixels, and the cube's drawn
/// extent is centred on half the width to within a tenth of its radius (its silhouette under
/// the pinhole eye is not exactly symmetric).
#[test]
fn the_first_and_a_stage_frame_are_one_centred_picture() {
    let Some((device, queue)) = software_device() else {
        return;
    };
    let mut screen = LoadingScreen::new(&device, wgpu::TextureFormat::Rgba8Unorm, 1);
    for size in [[640, 360], [1024, 562]] {
        let tex = target(&device, size);
        let frame = |screen: &mut LoadingScreen, stage: &str, detail: &str| {
            let v = LoadingView {
                title: "loading castle",
                stage,
                detail,
                progress: 0.0,
                seconds: 0.0,
                alpha: 1.0,
            };
            draw(&device, &queue, screen, &tex, &v)
        };
        let first = frame(&mut screen, "starting", "");
        let stage = frame(&mut screen, "lighting the near chunks", "31 / 49 chunks");
        let l = loading_layout(size);
        let w = size[0] as usize;
        let top = (l.cube_centre[1] - l.cube_px).floor() as usize;
        let row = w * 4;
        let differing = first[top * row..]
            .iter()
            .zip(&stage[top * row..])
            .filter(|(a, b)| a != b)
            .count();
        assert_eq!(differing, 0, "{size:?}: bytes differ below row {top}");
        // The cube: pixels in its band that differ from the background at the row's left edge
        // (the background varies with the row only).
        let bottom = (l.cube_centre[1] + l.cube_px).ceil() as usize;
        let (mut x0, mut x1) = (usize::MAX, 0);
        for y in top..bottom {
            let bg = &first[y * row..y * row + 3];
            for x in 0..w {
                let i = y * row + x * 4;
                let d: i32 = (0..3)
                    .map(|k| (i32::from(first[i + k]) - i32::from(bg[k])).abs())
                    .sum();
                if d > 24 {
                    x0 = x0.min(x);
                    x1 = x1.max(x);
                }
            }
        }
        assert!(x0 < x1, "{size:?}: no cube drawn");
        let mid = (x0 + x1 + 1) as f32 * 0.5;
        println!(
            "{size:?}: cube x {x0}..={x1}, middle {mid}, half width {}, radius {}",
            size[0] as f32 / 2.0,
            l.cube_px
        );
        assert!(
            (mid - size[0] as f32 / 2.0).abs() <= 0.1 * l.cube_px,
            "{size:?}: the cube's middle {mid} off half the width"
        );
    }
}

/// A second frame with different rows grows none of the overlay's CPU buffers.
#[test]
fn the_loading_frame_allocates_nothing_after_the_first() {
    let Some((device, queue)) = software_device() else {
        return;
    };
    let size = [320, 180];
    let tex = target(&device, size);
    let mut screen = LoadingScreen::new(&device, wgpu::TextureFormat::Rgba8Unorm, 1);
    draw(
        &device,
        &queue,
        &mut screen,
        &tex,
        &view(0.2, "uploading the level"),
    );
    let first = screen.allocations();
    draw(
        &device,
        &queue,
        &mut screen,
        &tex,
        &view(0.4, "lighting the coarse pass"),
    );
    assert_eq!(screen.allocations(), first, "the second frame allocated");
}

/// `tests/golden/loading_screen.png`: at 1280 x 720, progress 0.37, `seconds` 0.4 (the cube
/// part-way through its turn), alpha 1, titled `castle - benchmark`, the stage and its count
/// below; byte for byte. Blessed on lavapipe with `SVET_BLESS=1`.
#[test]
fn loading_screen_golden() {
    let Some((device, queue)) = software_device() else {
        return;
    };
    let size = [1280, 720];
    let tex = target(&device, size);
    let mut screen = LoadingScreen::new(&device, wgpu::TextureFormat::Rgba8Unorm, 2);
    let img = draw(
        &device,
        &queue,
        &mut screen,
        &tex,
        &view(0.37, "lighting the near chunks"),
    );
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join("loading_screen.png");
    if std::env::var("SVET_BLESS").as_deref() == Ok("1") {
        std::fs::create_dir_all(path.parent().expect("a golden folder")).expect("golden folder");
        let file = std::fs::File::create(&path).expect("create golden");
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), size[0], size[1]);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header()
            .expect("png header")
            .write_image_data(&img)
            .expect("png data");
        println!("blessed {path:?}");
        return;
    }
    let file = std::fs::File::open(&path).unwrap_or_else(|e| panic!("open {path:?}: {e}"));
    let mut reader = png::Decoder::new(std::io::BufReader::new(file))
        .read_info()
        .expect("png info");
    let mut golden = vec![0; reader.output_buffer_size().expect("png size")];
    let info = reader.next_frame(&mut golden).expect("png frame");
    assert_eq!(info.color_type, png::ColorType::Rgba);
    assert_eq!([info.width, info.height], size, "the golden's size");
    golden.truncate(info.buffer_size());
    let differing = img.iter().zip(&golden).filter(|(a, b)| a != b).count();
    assert_eq!(differing, 0, "loading_screen.png: {differing} bytes differ");
}

/// The pre-roll's rig: a rock floor below voxel z 16 over 6 x 6 x 2 chunks, mirrored whole with
/// the light radius 1 chunk, the live queue under a uniform sky with a coarse pass of 16 beyond
/// the near radius 1, 64 samples a brick direction.
fn pre_roll_rig(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> (svet_world::World, svet_render::GpuWorld, svet_render::Frame) {
    use svet_render::{Frame, GpuWorld, MirrorConfig, Window, sky::Sky};
    use svet_world::{AIR, ChunkExtent, FnSource, Material, MaterialTable, World};
    let table: MaterialTable = [Material::named("rock")].into_iter().collect();
    let extent = ChunkExtent::new([0, 0, 0], [6, 6, 2]);
    let world = World::generate(
        extent,
        &FnSource(|p: [i32; 3]| if p[2] < 16 { 1 } else { AIR }),
        &table,
    );
    let mut gpu = GpuWorld::with_config(
        device,
        MirrorConfig {
            light_radius: 1,
            ..MirrorConfig::new(Window::Fixed(extent))
        },
    );
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    gpu.sync(&world, queue, &mut enc).expect("the mirror fits");
    queue.submit([enc.finish()]);
    let mut frame = Frame::new(device, 64, 36);
    frame.set_sky(Some(Sky {
        altitude_zero_z: 16.0,
        ..Sky::uniform(1.0)
    }));
    let settings = svet_render::BakeSettings {
        brick_samples: 64,
        round: 64,
        ..Default::default()
    };
    frame.start_bake_queue(queue, &gpu, settings, 1.0e9);
    frame.set_bake_coarse(16, 1);
    (world, gpu, frame)
}

/// A pre-roll with the coarse box at 0 ms (`--pre-roll near`, and `off` with `near` empty) on
/// [`pre_roll_rig`], stepped until it ends or 400 steps, each step after a device wait and with
/// the step's index as its work time (no clock). Returns the step at the near chunks' moment
/// (the first state past `Near`), the full chunks mirrored then, and the end's summary.
fn boxed_at_zero(
    near: Vec<svet_world::ChunkCoord>,
) -> Option<(u32, usize, svet_render::PreRollSummary)> {
    use svet_render::{PreRoll, PreRollConfig, PreRollState, QueueView};
    let (device, queue) = svet_render::test_support::any_device(32 << 20, wgpu::Features::empty())?;
    let (world, mut gpu, mut frame) = pre_roll_rig(&device, &queue);
    let mut pre = PreRoll::new(PreRollConfig {
        near,
        coarse: 16,
        box_ms: 0.0,
        cap_ms: f64::INFINITY,
        samples: 4096,
    });
    let view = QueueView {
        eye: [128.0, 128.0, 40.0],
        forward: [0.0, 1.0, 0.0],
        resident: None,
        samples: None,
        interval_s: 0.0,
        hold: None,
        half_angle: 1.0,
        frustum: None,
        full_radius: None,
        lod_radius: None,
        focal_px: None,
    };
    let mut near_at = None;
    for step in 1..=400u32 {
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("device poll");
        let state = pre
            .step(
                &mut frame,
                &device,
                &queue,
                &mut gpu,
                &world,
                &view,
                f64::from(step),
            )
            .expect("the pre-roll's step");
        if near_at.is_none() && !matches!(state, PreRollState::Near { .. }) {
            near_at = Some((step, gpu.full_chunks().count()));
        }
        if let PreRollState::Done(s) = state {
            let (at, full) = near_at.expect("the near chunks' moment came first");
            println!(
                "near chunks' moment at step {at} of {}, {full} full chunks: {s:?}",
                s.steps
            );
            for &c in &pre.config().near {
                assert!(frame.bake_held_for_start(c, false), "{c:?} holds its light");
            }
            return Some((at, full, s));
        }
    }
    panic!("the pre-roll boxed at 0 ms did not end in 400 steps");
}

/// Plan 0108 Phase 1: a pre-roll whose coarse box is 0 ms ends at the step after its near
/// chunks' moment (on this rig the coarse pass has not covered its chunks by then), every near
/// chunk then holding its light, boxed, its coarse pass's cover the full chunks mirrored at
/// that moment (the rig's 72; the summary's `coarse_chunks` counts that cover, not the chunks
/// it baked); with no near chunk it is past its near stage at its first step and ends at its
/// second. The work time is the step's index: counts and states only.
#[test]
fn a_pre_roll_boxed_at_zero_ends_at_the_near_chunks() {
    use svet_world::ChunkCoord;
    let near: Vec<ChunkCoord> = (0..2)
        .flat_map(|z| (1..3).flat_map(move |y| (1..3).map(move |x| ChunkCoord([x, y, z]))))
        .collect();
    let Some((at, full, s)) = boxed_at_zero(near) else {
        return;
    };
    assert_eq!(
        full, 72,
        "the full chunks mirrored at the near chunks' moment"
    );
    assert_eq!(
        s.steps,
        at + 1,
        "the end against the near chunks' moment at step {at}"
    );
    assert_eq!((s.near_ms, s.coarse_chunks), (f64::from(at), full));
    assert!(s.boxed && !s.capped, "{s:?}");

    let Some((at, full, s)) = boxed_at_zero(Vec::new()) else {
        return;
    };
    assert_eq!((at, full, s.steps), (1, 72, 2), "{s:?}");
    assert_eq!((s.near_ms, s.near_baked, s.coarse_chunks), (1.0, 0, 72));
    assert!(s.boxed && !s.capped, "{s:?}");
}

/// Readings, run by hand: `--run-ignored only -E 'test(=reading::loading_pass_gpu_time)'`.
mod reading {
    use super::*;

    /// The loading pass's GPU time at 1920 x 1080: 200 frames (the cube turning at 60 Hz),
    /// the first 20 dropped, p50 and max printed. `SVET_TEST_ADAPTER=rtx` reads the first
    /// discrete adapter, unset lavapipe.
    #[test]
    #[ignore = "reading"]
    fn loading_pass_gpu_time() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let want_rtx = std::env::var("SVET_TEST_ADAPTER").as_deref() == Ok("rtx");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: !want_rtx,
            ..Default::default()
        }))
        .expect("an adapter");
        let name = adapter.get_info().name;
        let features = adapter.features() & wgpu::Features::TIMESTAMP_QUERY;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: features,
            ..Default::default()
        }))
        .expect("a device");
        let Some(timer) = svet_render::PassTimer::new(&device, &queue) else {
            println!("skipped: {name} has no timestamp queries");
            return;
        };
        let size = [1920, 1080];
        let tex = target(&device, size);
        let tv = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let mut screen = LoadingScreen::new(&device, wgpu::TextureFormat::Rgba8Unorm, 2);
        let mut ms = Vec::new();
        for i in 0..200 {
            let mut v = view(i as f32 / 200.0, "lighting the near chunks");
            v.seconds = i as f32 / 60.0;
            let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            screen.encode_timed(&queue, &mut enc, &tv, size, &v, Some(timer.render_writes()));
            timer.resolve(&mut enc);
            queue.submit([enc.finish()]);
            let t = timer.read_ms(&device);
            if i >= 20 {
                ms.push(t);
            }
        }
        ms.sort_by(f64::total_cmp);
        println!(
            "loading pass at 1920x1080 on {name}: p50 {:.4} ms, max {:.4} ms over {}",
            ms[ms.len() / 2],
            ms[ms.len() - 1],
            ms.len()
        );
    }
}
