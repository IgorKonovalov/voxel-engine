//! The HUD overlay on the software adapter (lavapipe): exact pixel counts of glyphs drawn onto
//! a cleared offscreen target.

use svet_render::internals::FONT;
use svet_render::{HudColour, HudOverlay, wgpu};

const CLEAR: [u8; 4] = [10, 20, 30, 255];
const WHITE: [u8; 3] = [255, 255, 255];

/// A device on the software Vulkan adapter, or `None` with a printed skip when the machine has
/// none. With `SVET_REQUIRE_SOFTWARE_ADAPTER=1` a missing adapter panics instead.
fn software_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    svet_render::test_support::software_device(0, wgpu::Features::empty())
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
    svet_render::debug::read_rgba8(device, queue, target)
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

/// `set_lines` draws what `set_rows` draws for the same lines and colours, a colour past
/// the end of the slice reading white.
#[test]
fn set_lines_draws_what_set_rows_draws() {
    let Some((device, queue)) = software_device() else {
        return;
    };
    let (w, h) = (64, 32);
    let mut hud = HudOverlay::new(&device, wgpu::TextureFormat::Rgba8Unorm, 1);
    hud.set_rows(
        &queue,
        &[
            (b"AB".as_slice(), HudColour::Yellow),
            (b"CD", HudColour::Red),
            (b"EF", HudColour::White),
        ],
    );
    let rows = draw(
        &device,
        &queue,
        &hud,
        &cleared_target(&device, &queue, w, h),
    );
    hud.set_lines(&queue, "AB\nCD\nEF\n", &[HudColour::Yellow, HudColour::Red]);
    let lines = draw(
        &device,
        &queue,
        &hud,
        &cleared_target(&device, &queue, w, h),
    );
    assert!(rows.chunks(4).any(|p| p[..3] == RED));
    assert_eq!(lines, rows);
}

/// 100 frames of twelve rows written with `set_lines`, 240 samples and the pass encoded and
/// submitted: the overlay allocates once, on the first frame, and frames 2 to 100 allocate
/// nothing of its own.
#[test]
fn the_overlay_allocates_nothing_after_its_first_frame() {
    let Some((device, queue)) = software_device() else {
        return;
    };
    let (w, h) = (320, 180);
    let target = cleared_target(&device, &queue, w, h);
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let mut hud = HudOverlay::new(&device, wgpu::TextureFormat::Rgba8Unorm, 1);
    let text = "fps 58.8  frame mean 17.00 ms\n".repeat(12);
    let colours = [HudColour::White; 12];
    let samples: Vec<f32> = (0..240).map(|i| (i % 40) as f32).collect();
    let mut after = Vec::new();
    for _ in 0..100 {
        hud.set_lines(&queue, &text, &colours);
        hud.set_samples(&queue, &samples, 33.3);
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        hud.encode(&mut enc, &view, w, h);
        queue.submit([enc.finish()]);
        after.push(hud.allocations());
    }
    assert_eq!(after, [1; 100]);
}

const RED: [u8; 3] = [255, 64, 64];

/// Samples `[0, c/2, c, 2c]` at scale 1 with `c = 32 ms`: bars of 0, 32, 64 and 64 white
/// pixels in columns 0 to 3, and the red line at `round(64 * (1000 / 60) / 32) = 33` rows
/// above the strip's bottom row wherever no bar covers it (columns 0 and 1).
#[test]
fn strip_bars_scale_with_the_ceiling_and_clamp() {
    let Some((device, queue)) = software_device() else {
        return;
    };
    let (w, h) = (16, 80);
    let c = 32.0;
    let mut hud = HudOverlay::new(&device, wgpu::TextureFormat::Rgba8Unorm, 1);
    hud.set_samples(&queue, &[0.0, c / 2.0, c, 2.0 * c], c);
    let img = draw(
        &device,
        &queue,
        &hud,
        &cleared_target(&device, &queue, w, h),
    );
    let px = |x: u32, y: u32| &img[((y * w + x) * 4) as usize..][..4];

    // The strip's bottom row sits above the last text row: y = 80 - 8 - 1 = 71.
    let bottom = h - 8 - 1;
    let line_y = bottom - 33;
    for (x, want) in [0u32, 32, 64, 64].into_iter().enumerate() {
        let x = x as u32;
        let white: Vec<u32> = (0..h).filter(|&y| px(x, y)[..3] == WHITE).collect();
        assert_eq!(white.len(), want as usize, "column {x}");
        // The bar is contiguous from the strip's bottom row up.
        assert!(white.iter().all(|&y| y <= bottom && y > bottom - want));
        let red: Vec<u32> = (0..h).filter(|&y| px(x, y)[..3] == RED).collect();
        if want <= 33 {
            assert_eq!(red, [line_y], "column {x}: the red line row");
        } else {
            assert!(red.is_empty(), "column {x}: the bar covers the line");
        }
    }
    // Nothing else is drawn.
    for y in 0..h {
        for x in 4..w {
            assert_eq!(px(x, y), CLEAR, "pixel ({x}, {y})");
        }
    }
    for x in 0..4 {
        for y in 0..h {
            let p = px(x, y);
            assert!(
                p[..3] == WHITE || p[..3] == RED || p == CLEAR,
                "pixel ({x}, {y}) is {p:?}"
            );
        }
    }
}

/// A ceiling at or below one 60 Hz frame draws no line, and an empty slice hides the strip.
#[test]
fn no_line_at_a_low_ceiling_and_no_strip_without_samples() {
    let Some((device, queue)) = software_device() else {
        return;
    };
    let (w, h) = (8, 80);
    let mut hud = HudOverlay::new(&device, wgpu::TextureFormat::Rgba8Unorm, 1);
    hud.set_samples(&queue, &[8.0], 16.0);
    let img = draw(
        &device,
        &queue,
        &hud,
        &cleared_target(&device, &queue, w, h),
    );
    assert_eq!(img.chunks(4).filter(|p| p[..3] == WHITE).count(), 32);
    assert_eq!(img.chunks(4).filter(|p| p[..3] == RED).count(), 0);
    hud.set_samples(&queue, &[], 16.0);
    let img = draw(
        &device,
        &queue,
        &hud,
        &cleared_target(&device, &queue, w, h),
    );
    assert!(img.chunks(4).all(|p| p == CLEAR));
}

/// The grey palette entry, `0.6` of white.
const GREY: [u8; 3] = [153, 153, 153];

/// A 4 x 2 block of cells with the backdrop and `ab` in grey on its first row, at scale 1
/// and then at scale 3 after `set_scale` with the cells unchanged: a block pixel no glyph or
/// shadow reaches (the middle of cell (3, 1)) is `mix(clear, black, 0.7)` within 1/255, each
/// set bit of `a` is the grey entry, and every pixel right of or below the block is the clear
/// colour.
#[test]
fn a_backdrop_cell_dims_and_a_grey_row_reads() {
    let Some((device, queue)) = software_device() else {
        return;
    };
    let (w, h) = (192, 96);
    let mut hud = HudOverlay::new(&device, wgpu::TextureFormat::Rgba8Unorm, 1);
    let block = svet_render::HudBox {
        col: 0,
        row: 0,
        cols: 4,
        rows: 2,
    };
    hud.set_lines_over(&queue, "ab", &[HudColour::Grey], block);
    let a = FONT[usize::from(b'a' - 32)];
    for scale in [1u32, 3] {
        hud.set_scale(&queue, scale);
        assert_eq!(hud.scale(), scale);
        let img = draw(
            &device,
            &queue,
            &hud,
            &cleared_target(&device, &queue, w, h),
        );
        let px = |x: u32, y: u32| &img[((y * w + x) * 4) as usize..][..4];
        let cell = 8 * scale;
        let dim = px(3 * cell + cell / 2, cell + cell / 2);
        for k in 0..3 {
            let want = f32::from(CLEAR[k]) * (1.0 - svet_render::BACKDROP_ALPHA);
            assert!(
                (f32::from(dim[k]) - want).abs() <= 1.0,
                "scale {scale}: backdrop pixel {dim:?}, channel {k} wants {want}"
            );
        }
        let mut glyph = 0;
        for (gy, row) in a.iter().enumerate() {
            for gx in 0..8u32 {
                if row >> gx & 1 == 1 {
                    let p = px(gx * scale, gy as u32 * scale);
                    assert_eq!(p[..3], GREY, "scale {scale}: bit ({gx}, {gy}) of a");
                    glyph += 1;
                }
            }
        }
        assert!(glyph > 0);
        for y in 0..h {
            for x in 0..w {
                if x >= 4 * cell || y >= 2 * cell {
                    assert_eq!(px(x, y), CLEAR, "scale {scale}: pixel ({x}, {y})");
                }
            }
        }
    }
}

/// Plan 0077 Phase 7 done-when 1 (ADR-0075): a 64 x 64 tile of a known pattern at the 4 x 4
/// cell block from cell (2, 1). At scale 2 the block's 64 x 64 pixels are the tile's, exact;
/// a glyph on a cell of the block draws its set bits white over the tile; every pixel outside
/// the block and away from the glyph is the clear colour. At scale 3 each tile pixel covers
/// 1.5 target pixels by nearest sampling: the 96 x 96 block's corners and centre read tile
/// pixels (0, 0), (63, 0), (0, 63), (63, 63) and (32, 32).
#[test]
fn a_tile_draws_in_its_cells() {
    let Some((device, queue)) = software_device() else {
        return;
    };
    let pattern = |x: u32, y: u32| [(x * 4) as u8, (y * 4) as u8, ((x ^ y) * 4) as u8, 255];
    let rgba: Vec<u8> = (0..64 * 64).flat_map(|i| pattern(i % 64, i / 64)).collect();
    let mut hud = HudOverlay::new(&device, wgpu::TextureFormat::Rgba8Unorm, 2);
    hud.set_tile(&queue, 7, &rgba);
    let tiles = [svet_render::HudTile {
        col: 2,
        row: 1,
        index: 7,
    }];
    let paint = svet_render::HudPaint {
        tiles: &tiles,
        ..Default::default()
    };
    let (w, h) = (192, 128);
    hud.set_lines_painted(&queue, "", &[], paint);
    let img = draw(
        &device,
        &queue,
        &hud,
        &cleared_target(&device, &queue, w, h),
    );
    let px = |img: &[u8], x: u32, y: u32| -> [u8; 4] {
        img[((y * w + x) * 4) as usize..][..4].try_into().unwrap()
    };
    for y in 0..h {
        for x in 0..w {
            let inside = (32..96).contains(&x) && (16..80).contains(&y);
            let want = if inside {
                pattern(x - 32, y - 16)
            } else {
                CLEAR
            };
            assert_eq!(px(&img, x, y), want, "scale 2: pixel ({x}, {y})");
        }
    }
    // A glyph on the block's top-left cell.
    hud.set_lines_painted(&queue, "\n  A", &[], paint);
    let img = draw(
        &device,
        &queue,
        &hud,
        &cleared_target(&device, &queue, w, h),
    );
    let a = FONT[usize::from(b'A' - 32)];
    let mut bits = 0;
    for (gy, row) in a.iter().enumerate() {
        for gx in 0..8u32 {
            if row >> gx & 1 == 1 {
                let p = px(&img, 32 + gx * 2, 16 + gy as u32 * 2);
                assert_eq!(p[..3], WHITE, "bit ({gx}, {gy}) of A over the tile");
                bits += 1;
            }
        }
    }
    assert!(bits > 0);
    assert_eq!(px(&img, 0, 0), CLEAR);
    assert_eq!(px(&img, 95, 79), pattern(63, 63), "the block's far corner");
    hud.set_scale(&queue, 3);
    hud.set_lines_painted(&queue, "", &[], paint);
    let (w3, h3) = (288, 192);
    let img = draw(
        &device,
        &queue,
        &hud,
        &cleared_target(&device, &queue, w3, h3),
    );
    let px3 =
        |x: u32, y: u32| -> [u8; 4] { img[((y * w3 + x) * 4) as usize..][..4].try_into().unwrap() };
    let (x0, y0) = (2 * 24, 24);
    for ((bx, by), (tx, ty)) in [
        ((0, 0), (0, 0)),
        ((95, 0), (63, 0)),
        ((0, 95), (0, 63)),
        ((95, 95), (63, 63)),
        ((48, 48), (32, 32)),
    ] {
        assert_eq!(
            px3(x0 + bx, y0 + by),
            pattern(tx, ty),
            "scale 3: block pixel ({bx}, {by})"
        );
    }
    assert_eq!(px3(x0 + 96, y0), CLEAR, "past the block");
}

/// Readings: measurement tests that print numbers and assert nothing a tier runs. Each is
/// `#[ignore]`d and run by hand, `cargo nextest run -p <package> --run-ignored only -E
/// 'test(=reading::<name>)'`; the soak and full tiers leave this module out by its name.
mod reading {
    use super::*;

    /// The HUD pass's GPU time at 1920 x 1080: 24 rows of 100 characters and 240 samples, scale
    /// 2, on the high-performance adapter with timestamp queries; median of 300 frames. A reading,
    /// not a check: run with `--ignored` on the machine being measured.
    #[test]
    #[ignore = "a GPU timing reading; run by hand with --ignored"]
    fn hud_pass_reading_at_1080p() {
        let Some((device, queue, adapter)) =
            svet_render::test_support::high_performance_device(0, wgpu::Features::TIMESTAMP_QUERY)
        else {
            return;
        };
        let Some(timer) = svet_render::PassTimer::new(&device, &queue) else {
            println!("skipped: {} has no TIMESTAMP_QUERY", adapter);
            return;
        };
        let (w, h) = (1920, 1080);
        let target = cleared_target(&device, &queue, w, h);
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut hud = HudOverlay::new(&device, wgpu::TextureFormat::Rgba8Unorm, 2);
        let rows: Vec<Vec<u8>> = (0..24)
            .map(|r| {
                (0..100)
                    .map(|c| b' ' + 1 + ((r * 7 + c) % 94) as u8)
                    .collect()
            })
            .collect();
        let rows: Vec<&[u8]> = rows.iter().map(Vec::as_slice).collect();
        let samples: Vec<f32> = (0..240).map(|i| (i % 40) as f32).collect();
        let mut ms = Vec::new();
        for _ in 0..300 {
            hud.set_text(&queue, &rows);
            hud.set_samples(&queue, &samples, 33.3);
            let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            hud.encode_timed(&mut enc, &view, w, h, Some(timer.render_writes()));
            timer.resolve(&mut enc);
            queue.submit([enc.finish()]);
            ms.push(timer.read_ms(&device));
        }
        ms.sort_by(f64::total_cmp);
        println!(
            "hud pass on {} at {w}x{h}, 24 rows x 100 chars, 240 samples, scale 2: median {:.4} ms, \
         p99 {:.4} ms, max {:.4} ms over {} frames",
            adapter,
            ms[ms.len() / 2],
            ms[ms.len() * 99 / 100],
            ms[ms.len() - 1],
            ms.len()
        );
    }
}
