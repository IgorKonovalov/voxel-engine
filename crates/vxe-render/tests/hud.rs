//! The HUD overlay on the software adapter (lavapipe): exact pixel counts of glyphs drawn onto
//! a cleared offscreen target.

use vxe_render::font8x8::FONT;
use vxe_render::{HudOverlay, readback, wgpu};

const CLEAR: [u8; 4] = [10, 20, 30, 255];
const WHITE: [u8; 3] = [255, 255, 255];

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

/// An `rgba8unorm` target of `width` x `height` cleared to [`CLEAR`].
fn cleared_target(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    width: u32,
    height: u32,
) -> wgpu::Texture {
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("hud test target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let [r, g, b, a] = CLEAR.map(|c| f64::from(c) / 255.0);
    enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("clear"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a }),
                store: wgpu::StoreOp::Store,
            },
        })],
        ..Default::default()
    });
    queue.submit([enc.finish()]);
    target
}

/// Draws `hud` onto `target` and reads it back.
fn draw(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    hud: &HudOverlay,
    target: &wgpu::Texture,
) -> Vec<u8> {
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    hud.encode(&mut enc, &view, target.width(), target.height());
    queue.submit([enc.finish()]);
    readback::read_rgba8(device, queue, target)
}

/// Whether `p` is black at alpha 0.75 over [`CLEAR`]: each channel a quarter of the clear
/// colour's, within one step of rounding.
fn is_shadow(p: &[u8]) -> bool {
    (0..3).all(|k| (f32::from(p[k]) - f32::from(CLEAR[k]) * 0.25).abs() <= 1.0)
}

/// `A` and a space at scale 2: every set bit of `A` becomes a 2 x 2 white block, nothing
/// reaches the space's cell, and every pixel is white, shadow or untouched.
#[test]
fn glyph_a_at_scale_2_draws_four_pixels_per_set_bit() {
    let Some((device, queue)) = software_device() else {
        return;
    };
    let (w, h) = (64, 16);
    let target = cleared_target(&device, &queue, w, h);
    let mut hud = HudOverlay::new(&device, wgpu::TextureFormat::Rgba8Unorm, 2);
    hud.set_text(&queue, &[b"A "]);
    let img = draw(&device, &queue, &hud, &target);

    let bits: u32 = FONT[usize::from(b'A' - 32)]
        .iter()
        .map(|r| r.count_ones())
        .sum();
    let white = img.chunks(4).filter(|p| p[..3] == WHITE).count();
    println!("A: {bits} set bits, {white} white pixels");
    assert_eq!(white, 4 * bits as usize);
    let mut shadows = 0;
    for (i, p) in img.chunks(4).enumerate() {
        let (x, y) = (i as u32 % w, i as u32 / w);
        if x >= 17 {
            assert_eq!(p, CLEAR, "pixel ({x}, {y}) right of cell 0's reach");
        }
        if p[..3] != WHITE && p != CLEAR {
            assert!(is_shadow(p), "pixel ({x}, {y}) is {p:?}");
            shadows += 1;
        }
    }
    println!("{shadows} shadow pixels");
    assert!(shadows > 0, "the glyph casts a shadow");

    // The glyph bit at (gx, gy) is white at pixels (2 gx .. 2 gx + 2, 2 gy .. 2 gy + 2).
    for (gy, row) in FONT[usize::from(b'A' - 32)].iter().enumerate() {
        for gx in 0..8 {
            let set = row >> gx & 1 == 1;
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (x, y) = (2 * gx + dx, 2 * gy + dy);
                let p = &img[(y * w as usize + x) * 4..][..4];
                assert_eq!(
                    p[..3] == WHITE,
                    set,
                    "glyph bit ({gx}, {gy}) at pixel ({x}, {y})"
                );
            }
        }
    }
}

/// A second `set_text` with fewer rows clears the rows the first one wrote.
#[test]
fn fewer_rows_clear_the_old_ones() {
    let Some((device, queue)) = software_device() else {
        return;
    };
    let (w, h) = (32, 32);
    let mut hud = HudOverlay::new(&device, wgpu::TextureFormat::Rgba8Unorm, 1);
    hud.set_text(&queue, &[b"AB", b"CD", b"EF"]);
    hud.set_text(&queue, &[b"A"]);
    let img = draw(
        &device,
        &queue,
        &hud,
        &cleared_target(&device, &queue, w, h),
    );
    let bits: u32 = FONT[usize::from(b'A' - 32)]
        .iter()
        .map(|r| r.count_ones())
        .sum();
    let white = img.chunks(4).filter(|p| p[..3] == WHITE).count();
    assert_eq!(white, bits as usize);
}
