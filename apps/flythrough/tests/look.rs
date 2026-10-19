//! The surface-look harness on the software adapter (lavapipe): normal error in a carved
//! sphere, silhouette error of a rock ball, and the look's flag bits leaving the goldens alone.
//! `VXE_BLESS=1` rewrites the look goldens.

mod support;

use support::*;
use vxe_render::gpu_world::{ATTR_PAGE_BRICK_SLOTS, POOL_PAGE_SLOTS, SLOT_BYTES};
use vxe_render::{
    GpuWorld, LightParams, LookParams, Marcher, MirrorConfig, NormalMode, ViewMode, Window, wgpu,
};
use vxe_world::World;

fn look(normal_mode: NormalMode, view: ViewMode) -> LookParams {
    LookParams {
        normal_mode,
        detail: false,
        view,
    }
}

/// Pool bytes a mirror with attribute pages needs for `full` bricks, with a page to spare.
fn attr_pool_bytes(full: u64) -> u64 {
    (full.div_ceil(u64::from(ATTR_PAGE_BRICK_SLOTS)) + 1) * u64::from(POOL_PAGE_SLOTS) * SLOT_BYTES
}

/// `world` mirrored with attribute pages over a fixed window, shards capped at
/// `max_shard_slots`, its attributes filled.
fn attr_rig_on(
    device: wgpu::Device,
    queue: wgpu::Queue,
    world: &World,
    max_shard_slots: Option<u32>,
) -> LookRig {
    let cfg = MirrorConfig {
        max_shard_slots,
        ..MirrorConfig::new(Window::Fixed(world.extent()))
    };
    let mut gpu = GpuWorld::with_surface_attrs(&device, cfg);
    gpu.sync(world, &queue).expect("mirror fits the adapter");
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let bricks = gpu.update_attrs(&mut enc);
    queue.submit([enc.finish()]);
    assert_eq!(
        bricks,
        world.stats().full_bricks,
        "every Full brick has its attribute"
    );
    let marcher = Marcher::new(&device, WIDTH, HEIGHT);
    LookRig {
        device,
        queue,
        gpu,
        marcher,
    }
}

/// [`attr_rig_on`] on the software adapter; `None` with a printed skip when there is none.
fn attr_rig(world: &World) -> Option<LookRig> {
    let full = world.stats().full_bricks as u64;
    let (device, queue) = software_device(attr_pool_bytes(full).max(64 << 20))?;
    Some(attr_rig_on(device, queue, world, None))
}

/// Cavity normal error (degrees) and pillar silhouette error (pixels) of `mode`, over mirrors
/// without attribute pages.
fn metrics(mode: NormalMode) -> Option<(f64, usize)> {
    metrics_with(mode, false)
}

/// [`metrics`], over mirrors with attribute pages when `attrs` is set.
fn metrics_with(mode: NormalMode, attrs: bool) -> Option<(f64, usize)> {
    metrics_of(mode, false, attrs)
}

/// [`metrics_with`], with `detail` on and rock (the cavity's row 3, the pillar's rock) at
/// `ROCK_DETAIL` when `detail` is set.
fn metrics_of(mode: NormalMode, detail: bool, attrs: bool) -> Option<(f64, usize)> {
    let mut cavity_table = look_table();
    let mut pillar_table = flythrough::cave_materials();
    if detail {
        // Row 3 is the cavity's rock; `look_table` keeps rows in id order.
        let mut t = vxe_world::MaterialTable::new();
        for (id, row) in cavity_table.rows().iter().enumerate().skip(1) {
            let mut row = row.clone();
            if id == 3 {
                row.detail = flythrough::ROCK_DETAIL;
            }
            t.push(row);
        }
        cavity_table = t;
        pillar_table = flythrough::look_materials();
    }
    let look = |view| LookParams {
        normal_mode: mode,
        detail,
        view,
    };
    let cavity = cavity_world_with(cavity_table);
    let mut rig = if attrs {
        attr_rig(&cavity)?
    } else {
        LookRig::new(&cavity, None)?
    };
    let cam = cavity_camera();
    let normals = rig.render(&cam, &LightParams::at_eye(), look(ViewMode::Normals));
    let normal_error = cavity_normal_error(&normals, &cam);

    let pillar = pillar_world_with(pillar_table);
    let mut rig = if attrs {
        attr_rig_on(rig.device, rig.queue, &pillar, None)
    } else {
        LookRig::on(rig.device, rig.queue, &pillar, None)
    };
    let cam = pillar_camera();
    let mask = rig.render(&cam, &LightParams::at_eye(), look(ViewMode::HitMask));
    let silhouette = pillar_silhouette_error(&mask, &cam);
    println!(
        "{}{}: cavity normal error {normal_error:.2} degrees, pillar silhouette error {silhouette} pixels",
        mode.name(),
        if detail { " + detail" } else { "" }
    );
    Some((normal_error, silhouette))
}

#[test]
fn face_normals_baseline() {
    let Some((normal_error, silhouette)) = metrics(NormalMode::Face) else {
        return;
    };
    assert!(
        (15.0..=45.0).contains(&normal_error),
        "face normal error {normal_error:.2} degrees"
    );
    assert!(
        (100..=800).contains(&silhouette),
        "face silhouette error {silhouette} pixels"
    );
}

/// The decoded `view = Normals` value at the image centre of the occluder scene, whose
/// centre ray meets the slab's -x face.
fn occluder_centre_normal(mode: NormalMode) -> Option<[f32; 3]> {
    let world = occluder_world();
    let mut rig = LookRig::new(&world, None)?;
    let img = rig.render(
        &occluder_camera(),
        &LightParams::at_eye(),
        look(mode, ViewMode::Normals),
    );
    let i = ((HEIGHT / 2) * WIDTH + WIDTH / 2) as usize * 4;
    let n = decode_normal(&img[i..i + 4]).to_array();
    println!("{}: occluder centre normal {n:?}", mode.name());
    Some(n)
}

#[test]
fn a_wall_facing_minus_x_reads_minus_x_in_face_mode() {
    let Some(n) = occluder_centre_normal(NormalMode::Face) else {
        return;
    };
    for (k, want) in [-1.0f32, 0.0, 0.0].into_iter().enumerate() {
        assert!(
            (n[k] - want).abs() <= 1.0 / 255.0 + 1e-6,
            "channel {k}: {} against {want}",
            n[k]
        );
    }
}

/// `primary.png`, `headlamp.png` and `lod.png` render byte for byte with the look at its
/// default: shaded, face normals, no detail.
#[test]
fn the_default_look_leaves_the_goldens_byte_for_byte() {
    if std::env::var("VXE_BLESS").as_deref() == Ok("1") {
        return;
    }
    let default = LookParams {
        normal_mode: NormalMode::Face,
        detail: false,
        view: ViewMode::Shaded,
    };
    assert_eq!(default, LookParams::default());
    let world = carved_world();
    let Some(mut rig) = LookRig::new(&world, None) else {
        return;
    };
    for (name, light) in [
        ("primary.png", LightParams::at_eye()),
        ("headlamp.png", LightParams::headlamp()),
    ] {
        let img = rig.render(&scene_camera(), &light, default);
        let golden = read_png(&golden_path(name)).0;
        let differing = img.iter().zip(&golden).filter(|(a, b)| a != b).count();
        assert_eq!(differing, 0, "{name}: {differing} bytes differ");
    }
    let lod = lod_scene();
    let mut rig = LookRig::on(rig.device, rig.queue, &lod, None);
    rig.marcher.set_lod(lod_forced_1());
    let img = rig.render(&scene_camera(), &LightParams::at_eye(), default);
    let golden = read_png(&golden_path("lod.png")).0;
    let differing = img.iter().zip(&golden).filter(|(a, b)| a != b).count();
    assert_eq!(differing, 0, "lod.png: {differing} bytes differ");
}

/// Mode A: both smoothed normals cut the cavity's normal error and move no hit.
#[test]
fn smoothed_normals_cut_the_normal_error_and_move_no_hit() {
    let Some((face_error, face_silhouette)) = metrics(NormalMode::Face) else {
        return;
    };
    let (tri_error, tri_silhouette) = metrics(NormalMode::Trilinear8).unwrap();
    let (g27_error, g27_silhouette) = metrics(NormalMode::Gradient27).unwrap();
    assert!(g27_error <= 12.0, "gradient27 {g27_error:.2} degrees");
    assert!(
        tri_error <= face_error / 2.0,
        "trilinear8 {tri_error:.2} degrees against face {face_error:.2}"
    );
    assert_eq!(tri_silhouette, face_silhouette);
    assert_eq!(g27_silhouette, face_silhouette);
}

#[test]
fn smoothing_does_not_bend_a_flat_wall() {
    for mode in [NormalMode::Trilinear8, NormalMode::Gradient27] {
        let Some(n) = occluder_centre_normal(mode) else {
            return;
        };
        for (k, want) in [-1.0f32, 0.0, 0.0].into_iter().enumerate() {
            assert!(
                (n[k] - want).abs() <= 2.0 / 255.0 + 1e-6,
                "{}: channel {k}: {} against {want}",
                mode.name(),
                n[k]
            );
        }
    }
}

/// The golden scene under the headlamp in `look`.
fn cave_render(look: LookParams) -> Option<Vec<u8>> {
    let world = carved_world();
    let mut rig = LookRig::new(&world, None)?;
    Some(rig.render(&scene_camera(), &LightParams::headlamp(), look))
}

#[test]
fn smoothed_normal_goldens() {
    for (name, mode) in [
        ("look_trilinear8.png", NormalMode::Trilinear8),
        ("look_gradient27.png", NormalMode::Gradient27),
    ] {
        let Some(img) = cave_render(look(mode, ViewMode::Shaded)) else {
            return;
        };
        assert_golden(name, &img);
        let (face, _, _) = read_png(&golden_path("headlamp.png"));
        let f = fraction_differing(&img, &face, 8);
        println!(
            "{name} differs from headlamp.png at {:.2} % of pixels",
            f * 100.0
        );
        assert!(f >= 0.01, "{name}: {:.2} %", f * 100.0);
    }
}

/// Mode B: contour planes halve the pillar's silhouette error and bring the cavity's normal
/// error under 12 degrees.
#[test]
fn contour_planes_cut_the_silhouette_and_normal_errors() {
    let Some((_, face_silhouette)) = metrics(NormalMode::Face) else {
        return;
    };
    let (error, silhouette) = metrics_with(NormalMode::Contour, true).unwrap();
    assert!(error <= 12.0, "contour {error:.2} degrees");
    assert!(
        2 * silhouette <= face_silhouette,
        "contour silhouette {silhouette} pixels against face {face_silhouette}"
    );
}

/// A world, its goldens with their lights, and whether its LOD walk is forced to level 1.
type Scene<'a> = (&'a World, &'a [(&'a str, LightParams)], bool);

/// The attribute layout changes no pixel in face mode: `primary.png`, `headlamp.png` and
/// `lod.png` render byte for byte with attribute pages, over one shard and over two to four.
#[test]
fn attribute_pages_leave_the_face_goldens_byte_for_byte() {
    if std::env::var("VXE_BLESS").as_deref() == Ok("1") {
        return;
    }
    let face = LookParams::default();
    let carved = carved_world();
    let lod = lod_scene();
    let full = carved.stats().full_bricks as u64;
    let Some((device, queue)) = software_device(attr_pool_bytes(full).max(64 << 20)) else {
        return;
    };
    let scenes: [Scene<'_>; 2] = [
        (
            &carved,
            &[
                ("primary.png", LightParams::at_eye()),
                ("headlamp.png", LightParams::headlamp()),
            ],
            false,
        ),
        (&lod, &[("lod.png", LightParams::at_eye())], true),
    ];
    for (world, goldens, forced) in scenes {
        // One shard first; its pages size the cap that spreads the next mirror over several.
        let mut pages = 0u32;
        for several in [false, true] {
            let cap = several.then(|| pages.div_ceil(3) * POOL_PAGE_SLOTS);
            let mut rig = attr_rig_on(device.clone(), queue.clone(), world, cap);
            let shards = rig.gpu.shards_in_use();
            pages = rig.gpu.pool_slots() / POOL_PAGE_SLOTS;
            println!("max_shard_slots {cap:?}: {pages} pages over {shards} shards");
            if several {
                assert!((2..=4).contains(&shards), "{shards} shards");
            } else {
                assert_eq!(shards, 1);
            }
            if forced {
                rig.marcher.set_lod(lod_forced_1());
            }
            for (name, light) in goldens {
                let img = rig.render(&scene_camera(), light, face);
                let golden = read_png(&golden_path(name)).0;
                let differing = img.iter().zip(&golden).filter(|(a, b)| a != b).count();
                assert_eq!(
                    differing, 0,
                    "{name} over {shards} shards: {differing} bytes differ"
                );
            }
        }
    }
}

#[test]
fn contour_golden() {
    let world = carved_world();
    let Some(mut rig) = attr_rig(&world) else {
        return;
    };
    let img = rig.render(
        &scene_camera(),
        &LightParams::headlamp(),
        look(NormalMode::Contour, ViewMode::Shaded),
    );
    assert_golden("look_contour.png", &img);
    let (face, _, _) = read_png(&golden_path("headlamp.png"));
    let f = fraction_differing(&img, &face, 8);
    println!(
        "look_contour.png differs from headlamp.png at {:.2} % of pixels",
        f * 100.0
    );
    assert!(f >= 0.01, "look_contour.png: {:.2} %", f * 100.0);
}

/// Pixels of a `WIDTH` x `HEIGHT` RGBA image whose Rec. 709 luma is below half the median luma
/// of their 8 neighbours; border pixels are not counted.
fn dark_specks(rgba: &[u8]) -> Vec<usize> {
    let (w, h) = (WIDTH as usize, HEIGHT as usize);
    let luma = |i: usize| {
        let p = &rgba[i * 4..i * 4 + 3];
        0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2])
    };
    let mut specks = Vec::new();
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let mut n = [0.0; 8];
            let mut k = 0;
            for dy in [-1, 0, 1] {
                for dx in [-1, 0, 1] {
                    if (dx, dy) != (0, 0) {
                        n[k] = luma((y as isize + dy) as usize * w + (x as isize + dx) as usize);
                        k += 1;
                    }
                }
            }
            n.sort_by(f64::total_cmp);
            // The median of eight: the mean of the middle two.
            let median = 0.5 * (n[3] + n[4]);
            if luma(y * w + x) < 0.5 * median {
                specks.push(y * w + x);
            }
        }
    }
    specks
}

/// A diagnostic, not a gate: the dark specks of the contour golden scene with the headlamp's
/// shadow ray on and off, and in face mode for comparison. Run with `--ignored` and read the
/// printed counts.
#[test]
#[ignore = "diagnostic: prints speck counts, asserts nothing about them"]
fn contour_dark_specks_with_and_without_shadows() {
    let world = carved_world();
    let Some(mut rig) = attr_rig(&world) else {
        return;
    };
    let off = LightParams {
        shadows: false,
        ..LightParams::headlamp()
    };
    for mode in [NormalMode::Contour, NormalMode::Face] {
        let render = |rig: &mut LookRig, light: &LightParams| {
            rig.render(&scene_camera(), light, look(mode, ViewMode::Shaded))
        };
        let on_img = render(&mut rig, &LightParams::headlamp());
        let off_img = render(&mut rig, &off);
        if mode == NormalMode::Contour {
            let golden = read_png(&golden_path("look_contour.png")).0;
            assert_eq!(on_img, golden, "the shadows-on render is look_contour.png");
        }
        let on = dark_specks(&on_img);
        let off = dark_specks(&off_img);
        let only_on = on.iter().filter(|i| off.binary_search(i).is_err()).count();
        println!(
            "{} ({WIDTH}x{HEIGHT}, headlamp): dark specks with shadows {}, without {}, only with {only_on}",
            mode.name(),
            on.len(),
            off.len()
        );
    }
}

fn detail_look(normal_mode: NormalMode, view: ViewMode) -> LookParams {
    LookParams {
        normal_mode,
        detail: true,
        view,
    }
}

/// The golden scene with its carve, under `table`.
fn carved_world_with(table: &vxe_world::MaterialTable) -> World {
    let source = scene_source();
    let world = World::generate(source.extent, &source, table);
    let r = world.apply(&scene_carve(&world)).unwrap();
    assert!(!r.changed_from.is_empty(), "the carve removes rock");
    world
}

/// With every row's strength at 0, the detail term is exactly zero: `primary.png`,
/// `headlamp.png` and `lod.png` render byte for byte with detail on.
#[test]
fn zero_strength_detail_leaves_the_goldens_byte_for_byte() {
    if std::env::var("VXE_BLESS").as_deref() == Ok("1") {
        return;
    }
    let world = carved_world();
    assert!(
        world.materials().rows().iter().all(|m| m.detail[1] == 0.0),
        "the cave table carries no detail"
    );
    let Some(mut rig) = LookRig::new(&world, None) else {
        return;
    };
    let on = detail_look(NormalMode::Face, ViewMode::Shaded);
    for (name, light) in [
        ("primary.png", LightParams::at_eye()),
        ("headlamp.png", LightParams::headlamp()),
    ] {
        let img = rig.render(&scene_camera(), &light, on);
        let golden = read_png(&golden_path(name)).0;
        let differing = img.iter().zip(&golden).filter(|(a, b)| a != b).count();
        assert_eq!(differing, 0, "{name}: {differing} bytes differ");
    }
    let lod = lod_scene();
    let mut rig = LookRig::on(rig.device, rig.queue, &lod, None);
    rig.marcher.set_lod(lod_forced_1());
    let img = rig.render(&scene_camera(), &LightParams::at_eye(), on);
    let golden = read_png(&golden_path("lod.png")).0;
    let differing = img.iter().zip(&golden).filter(|(a, b)| a != b).count();
    assert_eq!(differing, 0, "lod.png: {differing} bytes differ");
}

/// Rock at `detail = [4.0, 0.3]` changes at least 20 % of the headlamp scene's pixels by more
/// than 8/255 against face mode.
#[test]
fn rock_detail_golden() {
    let table = flythrough::look_materials();
    let rock = table.get(flythrough::cave::ROCK).unwrap();
    assert_eq!(rock.detail, [4.0, 0.3]);
    let world = carved_world_with(&table);
    let Some(mut rig) = LookRig::new(&world, None) else {
        return;
    };
    let img = rig.render(
        &scene_camera(),
        &LightParams::headlamp(),
        detail_look(NormalMode::Face, ViewMode::Shaded),
    );
    assert_golden("look_detail.png", &img);
    let face = rig.render(
        &scene_camera(),
        &LightParams::headlamp(),
        look(NormalMode::Face, ViewMode::Shaded),
    );
    let f = fraction_differing(&img, &face, 8);
    println!(
        "look_detail.png differs from face mode at {:.2} % of pixels by more than 8/255",
        f * 100.0
    );
    assert!(f >= 0.20, "{:.2} %", f * 100.0);
}

/// The detail noise is a function of world position alone: the pillar's front pole decodes
/// the same detailed normal from two camera positions on one line of sight.
#[test]
fn detail_is_a_function_of_world_position() {
    let world = pillar_world_with(flythrough::look_materials());
    let Some(mut rig) = LookRig::new(&world, None) else {
        return;
    };
    let near = pillar_camera();
    let (col, row) = (WIDTH / 2, HEIGHT / 2);
    let d = pixel_ray(&near, col, row);
    let far = vxe_render::Camera {
        position: near.position - d * 30.0,
        ..near
    };
    let on = detail_look(NormalMode::Face, ViewMode::Normals);
    let i = (row * WIDTH + col) as usize * 4;
    let a = rig.render(&near, &LightParams::at_eye(), on)[i..i + 4].to_vec();
    let b = rig.render(&far, &LightParams::at_eye(), on)[i..i + 4].to_vec();
    let flat = rig.render(
        &near,
        &LightParams::at_eye(),
        look(NormalMode::Face, ViewMode::Normals),
    )[i..i + 4]
        .to_vec();
    println!(
        "pole normal {:?} from both cameras {:?} / {:?}, without detail {:?}",
        decode_normal(&a),
        a,
        b,
        flat
    );
    for k in 0..3 {
        assert!(
            a[k].abs_diff(b[k]) <= 2,
            "channel {k}: {} against {}",
            a[k],
            b[k]
        );
    }
    assert!(
        (0..3).any(|k| a[k].abs_diff(flat[k]) > 4),
        "detail tilts the pole's normal: {a:?} against {flat:?}"
    );
}

/// A material row is 64 B on the GPU, and the live mirror bytes are the slots, records,
/// table and those rows, nothing else.
#[test]
fn material_rows_are_64_bytes() {
    let world = carved_world();
    let Some(rig) = LookRig::new(&world, None) else {
        return;
    };
    let gpu = &rig.gpu;
    let rows = world.materials().len() as u64;
    assert_eq!(gpu.materials().size(), rows * 64);
    let window = world
        .extent()
        .size()
        .iter()
        .map(|&v| v as u64)
        .product::<u64>();
    assert_eq!(
        gpu.mirror_bytes(),
        u64::from(gpu.live_slots()) * (512 + 72)
            + gpu.mirrored_chunks() as u64 * 2624
            + window * 4
            + rows * 64
    );
}

/// The comparison table's image metrics, one line per mode with and without detail, in one
/// run: `cargo nextest run -p flythrough --test look --no-capture every_mode`.
#[test]
fn every_mode_with_and_without_detail() {
    let mut rows = Vec::new();
    for mode in NormalMode::ALL {
        for detail in [false, true] {
            let Some(m) = metrics_of(mode, detail, mode == NormalMode::Contour) else {
                return;
            };
            rows.push((mode, detail, m));
        }
    }
    println!("| mode | detail | cavity normal error (degrees) | pillar silhouette error (px) |");
    println!("|---|---|---|---|");
    for (mode, detail, (error, silhouette)) in &rows {
        println!(
            "| {} | {} | {error:.2} | {silhouette} |",
            mode.name(),
            if *detail { "on" } else { "off" }
        );
    }
    // Detail moves no hit: each mode's silhouette is the same with and without it.
    for pair in rows.chunks(2) {
        assert_eq!(pair[0].2.1, pair[1].2.1, "{}", pair[0].0.name());
    }
}
