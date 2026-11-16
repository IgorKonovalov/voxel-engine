//! The engine's loading frame on the software adapter (lavapipe): the bar fills to its
//! fraction, and a second frame with new rows allocates nothing.

use vxe_render::{LoadingScreen, LoadingView, loading_bar_inner, loading_bar_rect, wgpu};

/// A device on the software Vulkan adapter, or `None` with a printed skip when the machine has
/// none. With `VXE_REQUIRE_SOFTWARE_ADAPTER=1` a missing adapter panics instead.
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
            if std::env::var("VXE_REQUIRE_SOFTWARE_ADAPTER").as_deref() == Ok("1") {
                panic!("VXE_REQUIRE_SOFTWARE_ADAPTER=1 but the fallback is {found}");
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
    vxe_render::debug::read_rgba8(device, queue, tex)
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

/// Readings, run by hand: `--run-ignored only -E 'test(=reading::loading_pass_gpu_time)'`.
mod reading {
    use super::*;

    /// The loading pass's GPU time at 1920 x 1080: 200 frames (the cube turning at 60 Hz),
    /// the first 20 dropped, p50 and max printed. `VXE_TEST_ADAPTER=rtx` reads the first
    /// discrete adapter, unset lavapipe.
    #[test]
    #[ignore = "reading"]
    fn loading_pass_gpu_time() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let want_rtx = std::env::var("VXE_TEST_ADAPTER").as_deref() == Ok("rtx");
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
        let Some(timer) = vxe_render::PassTimer::new(&device, &queue) else {
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
