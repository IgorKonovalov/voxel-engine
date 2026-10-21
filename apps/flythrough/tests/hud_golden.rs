//! The stats HUD over the golden scene on the software adapter (lavapipe): `primary.png`'s
//! view after its carve, the overlay drawn from `HudStats::fixed()` and a fixed 240-sample
//! strip at scale 1, compared pixel-exact with `hud.png`. `VXE_BLESS=1` rewrites it.
//!
//! The overlay is drawn onto a copy of the march target, the way the fly-through draws it
//! onto the surface after the blit. Integer scale and no filtering make every pixel exact.

mod support;

use flythrough::hud::{CEILING_MS, HudStats, RING};
use support::*;
use vxe_render::{GpuWorld, HudColour, HudOverlay, LightParams, readback, wgpu};

/// Glyph cells are 8 pixels; the overlay is drawn at scale 1.
const CELL: u32 = 8;
/// The strip's height at scale 1 (`vxe_render::hud::STRIP_PX`).
const STRIP: u32 = 64;

/// A fixed strip: a 12 to 23.5 ms sawtooth with a 40 ms spike (clamped at the ceiling)
/// every 60 frames, so bars of many heights, the red line and the clamp all show.
fn fixed_strip() -> Vec<f32> {
    (0..RING)
        .map(|i| {
            if i % 60 == 59 {
                40.0
            } else {
                12.0 + (i % 24) as f32 * 0.5
            }
        })
        .collect()
}

/// The march target and a copy of it with the overlay drawn when `hud` is true, both read
/// back.
struct Frame {
    march: Vec<u8>,
    composed: Vec<u8>,
}

fn render(hud: bool) -> Option<Frame> {
    let world = carved_world();
    let full = world.stats().full_bricks as u64;
    let (device, queue) = software_device((full + 4096) * 512)?;
    let mut gpu = GpuWorld::new(&device, world.extent());
    gpu.sync(&world, &queue).expect("mirror fits the adapter");
    let marcher = vxe_render::Frame::new(&device, WIDTH, HEIGHT);
    marcher.render(
        &device,
        &queue,
        &gpu,
        &scene_camera(),
        &LightParams::at_eye(),
    );
    let march = readback::read_rgba8(&device, &queue, marcher.target());

    let size = wgpu::Extent3d {
        width: WIDTH,
        height: HEIGHT,
        depth_or_array_layers: 1,
    };
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("hud golden target"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let mut overlay = HudOverlay::new(&device, wgpu::TextureFormat::Rgba8Unorm, 1);
    let (mut text, mut colours) = (String::new(), Vec::new());
    HudStats::fixed().write_rows(&mut text, &mut colours);
    let rows: Vec<(&[u8], HudColour)> = text
        .lines()
        .map(str::as_bytes)
        .zip(colours.iter().copied())
        .collect();
    overlay.set_rows(&queue, &rows);
    overlay.set_samples(&queue, &fixed_strip(), CEILING_MS);

    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_texture(
        marcher.target().as_image_copy(),
        target.as_image_copy(),
        size,
    );
    if hud {
        let view = target.create_view(&Default::default());
        overlay.encode(&mut encoder, &view, WIDTH, HEIGHT);
    }
    queue.submit([encoder.finish()]);
    let composed = readback::read_rgba8(&device, &queue, &target);
    Some(Frame { march, composed })
}

/// Bytes at which `a` and `b` differ.
fn differing_bytes(a: &[u8], b: &[u8]) -> usize {
    assert_eq!(a.len(), b.len());
    a.iter().zip(b).filter(|(x, y)| x != y).count()
}

#[test]
fn hud_golden_over_the_carved_view_is_pixel_exact() {
    let Some(frame) = render(true) else {
        return;
    };
    let img = &frame.composed;
    let path = golden_path("hud.png");
    if std::env::var("VXE_BLESS").as_deref() == Ok("1") {
        write_png(&path, img, WIDTH, HEIGHT);
        println!("blessed {path:?}");
        return;
    }
    let (golden, w, h) = read_png(&path);
    assert_eq!((w, h), (WIDTH, HEIGHT));
    let differing = differing_bytes(img, &golden);
    assert_eq!(differing, 0, "hud.png: {differing} bytes differ");

    // The overlay touches only the text rows (plus their one-pixel shadow) and the strip;
    // every other pixel is the march target's.
    let rows = HudStats::fixed().to_string().lines().count() as u32;
    let text_bottom = rows * CELL + 1;
    let strip_top = HEIGHT - CELL - STRIP;
    let strip_cols = RING as u32;
    let mut outside = 0;
    for (i, (p, q)) in img.chunks(4).zip(frame.march.chunks(4)).enumerate() {
        let (x, y) = (i as u32 % WIDTH, i as u32 / WIDTH);
        let in_text = y < text_bottom;
        let in_strip = (strip_top..HEIGHT - CELL).contains(&y) && x < strip_cols;
        if !in_text && !in_strip {
            outside += 1;
            assert_eq!(p, q, "pixel ({x}, {y}) outside the overlay changed");
        }
    }
    println!("{rows} rows; {outside} pixels outside the overlay match the march target");
    assert!(outside > 0);

    // The overlay drew: white glyph and bar pixels, yellow rows, the red line (`hud.wgsl`'s
    // colours (1, 0.8, 0) and (1, 0.25, 0.25) in `Rgba8Unorm`).
    let count = |rgb: [u8; 3]| img.chunks(4).filter(|p| p[..3] == rgb).count();
    let (white, yellow, red) = (
        count([255, 255, 255]),
        count([255, 204, 0]),
        count([255, 64, 64]),
    );
    println!("white {white}, yellow {yellow}, red {red} pixels");
    assert!(white > 0 && yellow > 0 && red > 0);
}

/// With the HUD off the image is `primary.png` byte for byte, both read straight off the
/// march target and through the copy the HUD is drawn onto.
#[test]
fn hud_off_is_the_primary_golden_byte_for_byte() {
    if std::env::var("VXE_BLESS").as_deref() == Ok("1") {
        return;
    }
    let Some(frame) = render(false) else {
        return;
    };
    let (primary, _, _) = read_png(&golden_path("primary.png"));
    assert_eq!(differing_bytes(&frame.march, &primary), 0, "march target");
    assert_eq!(
        differing_bytes(&frame.composed, &primary),
        0,
        "copied target"
    );
}
