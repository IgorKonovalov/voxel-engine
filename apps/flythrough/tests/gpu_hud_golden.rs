//! The stats HUD over the golden scene on the software adapter (lavapipe): `primary.png`'s
//! view after its carve, the overlay drawn from [`fixed_stats`] (`HudStats::fixed()` with the
//! start's mode block) and a fixed 240-sample strip at scale 1, compared pixel-exact with
//! `hud.png`. `VXE_BLESS=1` rewrites it.
//!
//! The overlay is drawn onto a copy of the march target, the way the fly-through draws it
//! onto the surface after the blit. Integer scale and no filtering make every pixel exact.

mod support;

use flythrough::hud::{CEILING_MS, HudStats, RING};
use support::*;
use vxe_render::{GpuWorld, HudBox, HudColour, HudOverlay, LightParams, wgpu};

/// Glyph cells are 8 pixels; the overlay is drawn at scale 1.
const CELL: u32 = 8;
/// The strip's height at scale 1 (`STRIP_PX` of `vxe-render`'s HUD).
const STRIP: u32 = 64;

/// `HudStats::fixed()` with the mode block the fly-through shows at its start: the five rows
/// of `Modes::write_rows` at the default lighting and resolution, as the banner golden builds
/// the start's modes.
fn fixed_stats() -> HudStats {
    let mut stats = HudStats::fixed();
    flythrough::hud::Modes::new(
        vxe_render::Lighting::default(),
        vxe_render::LightingResolution::default(),
        0.0,
    )
    .write_rows(&mut stats.modes);
    stats
}

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
    render_overlay(hud, None, None)
}

/// The menu's text as the overlay draws it: the rows, their colours, the paint laid over
/// them (the backdrop's box, spans and tiles) and the tiles' pixels by atlas index.
struct MenuText<'a> {
    text: &'a str,
    colours: &'a [HudColour],
    paint: vxe_render::HudPaint<'a>,
    tiles: &'a [(u32, Vec<u8>)],
}

/// [`render`], or with `banner` the overlay drawing that one line alone, yellow, with no
/// strip: the fly-through's mode banner with the stats HUD off; with `menu` the overlay
/// drawing the menu's rows over their backdrop, with no strip.
fn render_overlay(hud: bool, banner: Option<&str>, menu: Option<MenuText<'_>>) -> Option<Frame> {
    let world = carved_world();
    let full = world.stats().full_bricks as u64;
    let (device, queue) = software_device((full + 4096) * 512)?;
    let mut gpu = GpuWorld::new(&device, world.extent());
    gpu.sync_now(&world, &device, &queue)
        .expect("mirror fits the adapter");
    let mut marcher = vxe_render::Frame::new(&device, WIDTH, HEIGHT);
    marcher.set_resolution(&device, vxe_render::LightingResolution::Full);
    marcher.render(
        &device,
        &queue,
        &gpu,
        &scene_camera(),
        &LightParams::at_eye(),
    );
    let march = vxe_render::debug::read_rgba8(&device, &queue, marcher.target());

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
    fixed_stats().write_rows(&mut text, &mut colours);
    let rows: Vec<(&[u8], HudColour)> = text
        .lines()
        .map(str::as_bytes)
        .zip(colours.iter().copied())
        .collect();
    overlay.set_rows(&queue, &rows);
    overlay.set_samples(&queue, &fixed_strip(), CEILING_MS);
    if let Some(b) = banner {
        overlay.set_lines(
            &queue,
            b,
            &[HudColour::Yellow; flythrough::hud::BANNER_ROWS],
        );
        overlay.set_samples(&queue, &[], 0.0);
    }
    if let Some(m) = &menu {
        for (i, px) in m.tiles {
            overlay.set_tile(&queue, *i, px);
        }
        overlay.set_lines_painted(&queue, m.text, m.colours, m.paint);
        overlay.set_samples(&queue, &[], 0.0);
    }

    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_texture(
        marcher.target().as_image_copy(),
        target.as_image_copy(),
        size,
    );
    if hud || banner.is_some() || menu.is_some() {
        let view = target.create_view(&Default::default());
        overlay.encode(&mut encoder, &view, WIDTH, HEIGHT);
    }
    queue.submit([encoder.finish()]);
    let composed = vxe_render::debug::read_rgba8(&device, &queue, &target);
    Some(Frame { march, composed })
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
    let rows = fixed_stats().to_string().lines().count() as u32;
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

/// `mode_banner.png`: the mode banner the fly-through draws for 3 s after the start and after
/// every mode change, with the stats HUD off: the start's modes over the carved view, wrapped
/// into rows. Only the banner's rows (and their one-pixel shadow) differ from the march target.
#[test]
fn mode_banner_golden_shows_the_modes_with_the_hud_off() {
    let line = flythrough::hud::Modes::new(
        vxe_render::Lighting::default(),
        vxe_render::LightingResolution::default(),
        0.0,
    )
    .banner(0.0)
    .expect("the banner shows at the start");
    let Some(frame) = render_overlay(false, Some(&line), None) else {
        return;
    };
    let img = &frame.composed;
    let path = golden_path("mode_banner.png");
    if std::env::var("VXE_BLESS").as_deref() == Ok("1") {
        write_png(&path, img, WIDTH, HEIGHT);
        println!("blessed {path:?}");
    } else {
        let differing = differing_bytes(img, &read_png(&path).0);
        assert_eq!(differing, 0, "mode_banner.png: {differing} bytes differ");
    }
    let rows = line.lines().count() as u32;
    let mut changed = 0;
    for (i, (p, q)) in img.chunks(4).zip(frame.march.chunks(4)).enumerate() {
        let y = i as u32 / WIDTH;
        if p != q {
            changed += 1;
            assert!(
                y <= rows * CELL,
                "pixel at row {y} outside the banner's rows changed"
            );
        }
    }
    println!("{line}: {changed} pixels drawn");
    assert!(changed > 0);
}

/// `menu.png`: the menu's root page at the start's modes over the carved view, at scale 1
/// (40 x 22 cells), placed by `menu::place` at the window's top-left with a fixed readout
/// on its title row (the `audio` group grey: the run has no audio): the backdrop dims the
/// page's rows plus one row below the footer, the
/// whole width. Only those rows change; on the padding row below the footer (clear of the
/// footer's shadow) every pixel is the march target's at `1 - BACKDROP_ALPHA` within 1/255.
#[test]
fn menu_over_the_carved_view() {
    use flythrough::hud::menu::{Menu, Panel, RunView, place, step};
    let m = flythrough::hud::Modes::new(
        vxe_render::Lighting::default(),
        vxe_render::LightingResolution::default(),
        0.0,
    );
    let mut menu = Menu::default();
    let mut input = vxe_app::Input::default();
    input.key_down(vxe_app::KeyCode::Tab);
    step(&mut menu, &input, &m, &RunView::default());
    let (mut page, mut page_colours) = (String::new(), Vec::new());
    menu.write(&m, &RunView::default(), &mut page, &mut page_colours);
    let panel = Panel {
        scale: 1,
        col: 0,
        row: 0,
        cols: (WIDTH / CELL) as usize,
        rows: (HEIGHT / CELL) as usize,
    };
    let (mut text, mut colours) = (String::new(), Vec::new());
    let backdrop = place(
        &page,
        &page_colours,
        panel,
        "fps 59.9  frame 16.7 ms p99 16.7",
        &mut text,
        &mut colours,
    );
    let rows = page.lines().count() as u32;
    assert_eq!(
        backdrop,
        HudBox {
            col: 0,
            row: 0,
            cols: WIDTH / CELL,
            rows: rows + 1,
        }
    );
    let menu = MenuText {
        text: &text,
        colours: &colours,
        paint: vxe_render::HudPaint {
            backdrops: std::slice::from_ref(&backdrop),
            ..Default::default()
        },
        tiles: &[],
    };
    let Some(frame) = render_overlay(false, None, Some(menu)) else {
        return;
    };
    let img = &frame.composed;
    let path = golden_path("menu.png");
    if std::env::var("VXE_BLESS").as_deref() == Ok("1") {
        write_png(&path, img, WIDTH, HEIGHT);
        println!("blessed {path:?}");
    } else {
        let differing = differing_bytes(img, &read_png(&path).0);
        assert_eq!(differing, 0, "menu.png: {differing} bytes differ");
    }
    let pad = rows * CELL + 1..(rows + 1) * CELL;
    let (mut outside, mut dimmed) = (0, 0);
    for (i, (p, q)) in img.chunks(4).zip(frame.march.chunks(4)).enumerate() {
        let y = i as u32 / WIDTH;
        if y >= (rows + 1) * CELL {
            assert_eq!(p, q, "pixel row {y} below the backdrop changed");
            outside += 1;
        } else if pad.contains(&y) {
            for k in 0..3 {
                let want = f32::from(q[k]) * (1.0 - vxe_render::BACKDROP_ALPHA);
                assert!(
                    (f32::from(p[k]) - want).abs() <= 1.0,
                    "row {y}: {p:?} over {q:?}"
                );
            }
            dimmed += 1;
        }
    }
    println!("{rows} rows; {dimmed} dimmed padding pixels, {outside} untouched");
    assert!(outside > 0 && dimmed > 0);
    let yellow = img.chunks(4).filter(|p| p[..3] == [255, 204, 0]).count();
    assert!(yellow > 0, "the selected group is yellow");
    let grey = img.chunks(4).filter(|p| p[..3] == [153, 153, 153]).count();
    assert!(
        grey > 0,
        "the audio group, with no audio in the run, is grey"
    );
}

/// Plan 0077 Phase 7 done-when 6: `chooser.png`, the chooser (the menu's entities page) over
/// the carved view at scale 1 with both library entries' icons (the showcase's table with
/// the walker's rows, so the spider builds), placed at the window's top-left; and `belt.png`,
/// the belt in placement mode at slot 1 with its icons in the slots and slot 1's frame
/// yellow, the grid's 40 x 22 cells (the belt cut at the window's right edge). Pixel exact
/// against goldens blessed on lavapipe.
#[test]
fn the_belt_and_the_chooser_over_the_carved_view() {
    use flythrough::entities::{self, belt, library};
    use flythrough::hud::menu::{Menu, PageId, Panel, RunView, place};
    let table = flythrough::walker::with_walker(&flythrough::showcase_materials());
    let mut m = flythrough::hud::Modes::new(
        vxe_render::Lighting::default(),
        vxe_render::LightingResolution::default(),
        0.0,
    );
    let lib = library::scan(std::path::Path::new(library::DEFAULT_DIR));
    m.walkers = entities::Walkers::default().with_library(lib);
    assert_eq!(m.walkers.poll_viewer(0.0, &table), None, "both build");
    m.walkers.refresh_icons(&table);
    m.walkers.icons.wait();
    let order = m.walkers.library.order();
    m.walkers.belt.fill(&order, &[]);
    let pixels: Vec<(u32, Vec<u8>)> = m.walkers.icons.uploads().collect();
    assert_eq!(pixels.len(), 2, "both icons");
    let grid = ((WIDTH / CELL) as usize, (HEIGHT / CELL) as usize);

    let mut menu = Menu::default();
    menu.open_at(PageId::Entities);
    let (mut page, mut page_colours) = (String::new(), Vec::new());
    let mut page_paint = belt::Paint::default();
    menu.write_with(
        &m,
        &RunView::default(),
        None,
        &mut page,
        &mut page_colours,
        &mut page_paint,
    );
    assert_eq!(page_paint.tiles.len(), 2, "a tile per entry");
    let panel = Panel {
        scale: 1,
        col: 0,
        row: 0,
        cols: grid.0,
        rows: grid.1,
    };
    let (mut text, mut colours) = (String::new(), Vec::new());
    let backdrop = place(&page, &page_colours, panel, "", &mut text, &mut colours);
    let chooser = MenuText {
        text: &text,
        colours: &colours,
        paint: vxe_render::HudPaint {
            backdrops: std::slice::from_ref(&backdrop),
            tiles: &page_paint.tiles,
            ..Default::default()
        },
        tiles: &pixels,
    };
    let (mut belt_text, mut belt_colours) = (String::new(), Vec::new());
    let mut paint = belt::Paint::default();
    let belt_box = belt::append_belt(
        &mut belt_text,
        &mut belt_colours,
        grid,
        &m,
        0.0,
        flythrough::hud::BANNER_MS,
        &mut paint,
    );
    assert_eq!(
        paint.tiles.len(),
        2,
        "the two filled slots show their icons"
    );
    let belt = MenuText {
        text: &belt_text,
        colours: &belt_colours,
        paint: paint.with(std::slice::from_ref(&belt_box)),
        tiles: &pixels,
    };
    for (name, overlay) in [("chooser.png", chooser), ("belt.png", belt)] {
        let Some(frame) = render_overlay(false, None, Some(overlay)) else {
            return;
        };
        let img = &frame.composed;
        let path = golden_path(name);
        if std::env::var("VXE_BLESS").as_deref() == Ok("1") {
            write_png(&path, img, WIDTH, HEIGHT);
            println!("blessed {path:?}");
        } else {
            let differing = differing_bytes(img, &read_png(&path).0);
            assert_eq!(differing, 0, "{name}: {differing} bytes differ");
        }
        let yellow = img.chunks(4).filter(|p| p[..3] == [255, 204, 0]).count();
        assert!(yellow > 0, "{name}: the selection or the frame is yellow");
    }
}

/// Plan 0083 Phase 8 (the Phase 6 amendment's addition): `structures.png`, the menu's
/// Structures page over the carved view at scale 1, placed at the window's top-left: the
/// committed library's chapel with its icon and its rows, the castle before its build in grey
/// (`castle: building`), against the cave's table with the walker's and the library's rows.
/// Pixel exact against the golden blessed on lavapipe.
#[test]
fn the_structures_chooser() {
    use flythrough::entities::{self, belt, library};
    use flythrough::hud::menu::{Menu, PageId, Panel, RunView, place};
    use flythrough::structures as st;
    let base = flythrough::walker::with_walker(&flythrough::cave_materials());
    let mut table = base.clone();
    let dir = std::path::Path::new(st::LIBRARY);
    st::add_library_rows(dir, None, &mut table);
    let mut m = flythrough::hud::Modes::new(
        vxe_render::Lighting::default(),
        vxe_render::LightingResolution::default(),
        0.0,
    );
    let lib = library::scan(std::path::Path::new(library::DEFAULT_DIR));
    m.walkers = entities::Walkers::default().with_library(lib);
    m.structures = st::Library::load(dir, &table, &base);
    let names: Vec<&str> = m
        .structures
        .entries
        .iter()
        .map(|e| e.name.as_str())
        .collect();
    assert_eq!(names, ["chapel", st::CASTLE_NAME]);
    st::refresh_icons(&mut m, &table);
    m.walkers.icons.wait();
    let chapel_tile = belt::tile_of(&m, belt::Item::Structure(0));
    assert!(m.walkers.icons.ready(chapel_tile), "the chapel's icon");
    let pixels: Vec<(u32, Vec<u8>)> = m.walkers.icons.uploads().collect();
    let grid = ((WIDTH / CELL) as usize, (HEIGHT / CELL) as usize);

    let mut menu = Menu::default();
    menu.open_at(PageId::Structures);
    let (mut page, mut page_colours) = (String::new(), Vec::new());
    let mut page_paint = belt::Paint::default();
    menu.write_with(
        &m,
        &RunView::default(),
        None,
        &mut page,
        &mut page_colours,
        &mut page_paint,
    );
    assert!(page.contains("castle: building"), "{page}");
    assert!(
        page_paint
            .tiles
            .iter()
            .any(|t| t.index == chapel_tile as u32),
        "the chapel's tile on the page"
    );
    let panel = Panel {
        scale: 1,
        col: 0,
        row: 0,
        cols: grid.0,
        rows: grid.1,
    };
    let (mut text, mut colours) = (String::new(), Vec::new());
    let backdrop = place(&page, &page_colours, panel, "", &mut text, &mut colours);
    let chooser = MenuText {
        text: &text,
        colours: &colours,
        paint: vxe_render::HudPaint {
            backdrops: std::slice::from_ref(&backdrop),
            tiles: &page_paint.tiles,
            ..Default::default()
        },
        tiles: &pixels,
    };
    let Some(frame) = render_overlay(false, None, Some(chooser)) else {
        return;
    };
    let img = &frame.composed;
    let path = golden_path("structures.png");
    if std::env::var("VXE_BLESS").as_deref() == Ok("1") {
        write_png(&path, img, WIDTH, HEIGHT);
        println!("blessed {path:?}");
    } else {
        let differing = differing_bytes(img, &read_png(&path).0);
        assert_eq!(differing, 0, "structures.png: {differing} bytes differ");
    }
    let grey = img.chunks(4).filter(|p| p[..3] == [153, 153, 153]).count();
    assert!(grey > 0, "the castle before its build is grey");
}

/// Readings run by hand (`--run-ignored only -E 'test(=reading::<name>)'`).
mod reading {
    use super::*;
    use flythrough::hud::menu::{Menu, PageId, Panel, RunView, place};

    /// The open menu's rows through the overlay over 61 frames, on the root page and on the
    /// palette with one letter typed, placed in a 2530 x 1378 window's panel with the
    /// readout and drawn over their backdrop at the panel's scale: the overlay's allocation
    /// count after the first frame and after the last, and each page's formatting time
    /// (`Menu::write` and `menu::place`, as the HUD's `format_us` times them) as the median
    /// of the 60 later frames, in microseconds.
    #[test]
    #[ignore = "reading: the menu's overlay allocations and format time"]
    fn menu_allocations_and_format_time() {
        let Some((device, queue, adapter)) = reading_device(1 << 20) else {
            return;
        };
        let mut overlay = HudOverlay::new(&device, wgpu::TextureFormat::Rgba8Unorm, 2);
        let m = flythrough::hud::Modes::new(
            vxe_render::Lighting::default(),
            vxe_render::LightingResolution::Half,
            0.0,
        );
        let v = RunView::default();
        let (mut text, mut colours) = (String::new(), Vec::new());
        let (mut placed, mut placed_colours) = (String::new(), Vec::new());
        let panel = Panel::of(2530, 1378);
        overlay.set_scale(&queue, panel.scale);
        let readout = "fps 59.9  frame 16.7 ms p99 16.7";
        for (name, filter) in [("root", ""), ("palette l", "l")] {
            let mut menu = Menu::default();
            menu.open_at(PageId::Root);
            for c in filter.chars() {
                menu.input(flythrough::hud::menu::MenuInput::Type(c), &m, &v);
            }
            let mut us = Vec::with_capacity(60);
            let mut first = 0;
            for frame in 0..61 {
                let t = flythrough::now();
                menu.write(&m, &v, &mut text, &mut colours);
                let backdrop = place(
                    &text,
                    &colours,
                    panel,
                    readout,
                    &mut placed,
                    &mut placed_colours,
                );
                let took = (flythrough::now() - t).as_secs_f64() * 1e6;
                overlay.set_lines_over(&queue, &placed, &placed_colours, backdrop);
                if frame == 0 {
                    first = overlay.allocations();
                } else {
                    us.push(took);
                }
            }
            us.sort_by(f64::total_cmp);
            println!(
                "menu {name} on {adapter}: overlay allocations {first} after the first frame, \
                 {} after 61; format median {:.1} us, max {:.1} us, {} rows",
                overlay.allocations(),
                us[us.len() / 2],
                us[us.len() - 1],
                text.lines().count()
            );
        }
    }
}
