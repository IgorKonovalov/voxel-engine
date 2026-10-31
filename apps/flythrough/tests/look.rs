//! The surface-look harness on the software adapter (lavapipe): normal error in a carved
//! sphere, silhouette error of a rock ball, and the look's flag bits leaving the goldens alone.
//! `VXE_BLESS=1` rewrites the look goldens.

mod support;

use support::sharpness::axis_fraction;
use support::*;
use vxe_render::gpu_world::PAGE_UNITS;
use vxe_render::{LightParams, LookParams, NormalMode, ViewMode};
use vxe_world::World;

/// Rock's detail in the look tests: the spike's `[scale, strength]`, pinned here so that the
/// fly-through's default can move without moving a golden or a metric.
const SPIKE_ROCK_DETAIL: [f32; 2] = [4.0, 0.3];

/// The cave table with rock at [`SPIKE_ROCK_DETAIL`].
fn spike_materials() -> vxe_world::MaterialTable {
    flythrough::look_materials_with(SPIKE_ROCK_DETAIL)
}

fn look(normal_mode: NormalMode, view: ViewMode) -> LookParams {
    LookParams {
        normal_mode,
        detail: false,
        view,
        ..LookParams::default()
    }
}

/// Cavity normal error (degrees) and pillar silhouette error (pixels) of `mode`, over mirrors
/// without contour attributes.
fn metrics(mode: NormalMode) -> Option<(f64, usize)> {
    metrics_with(mode, false)
}

/// [`metrics`], over mirrors with contour attributes when `attrs` is set.
fn metrics_with(mode: NormalMode, attrs: bool) -> Option<(f64, usize)> {
    metrics_of(mode, false, attrs)
}

/// [`metrics_with`], with `detail` on and rock (the cavity's row 3, the pillar's rock) at
/// [`SPIKE_ROCK_DETAIL`] when `detail` is set.
fn metrics_of(mode: NormalMode, detail: bool, attrs: bool) -> Option<(f64, usize)> {
    guardrails_of(mode, detail, attrs).map(|g| (g.normal_error, g.silhouette))
}

/// The guardrails of the "not a Lego look" bar: the cavity's normal error (degrees), the
/// pillar's silhouette error (pixels) and the cavity's axis fraction `A`, the share of its
/// hit pixels whose shading normal (detailed when `detail` is set) lies within 5 degrees of
/// a coordinate axis.
#[derive(Clone, Copy, Debug)]
struct Guardrails {
    normal_error: f64,
    silhouette: usize,
    axis_fraction: f64,
}

/// [`Guardrails`] of `mode` as [`metrics_of`] renders them.
fn guardrails_of(mode: NormalMode, detail: bool, attrs: bool) -> Option<Guardrails> {
    let mut cavity_table = look_table();
    let mut pillar_table = flythrough::cave_materials();
    if detail {
        // Row 3 is the cavity's rock; `look_table` keeps rows in id order.
        let mut t = vxe_world::MaterialTable::new();
        for (id, row) in cavity_table.rows().iter().enumerate().skip(1) {
            let mut row = row.clone();
            if id == 3 {
                row.detail = SPIKE_ROCK_DETAIL;
            }
            t.push(row);
        }
        cavity_table = t;
        pillar_table = spike_materials();
    }
    let look = |view| LookParams {
        normal_mode: mode,
        detail,
        view,
        ..LookParams::default()
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
    let axis = axis_fraction(&normals);

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
        "{}{}: cavity normal error {normal_error:.2} degrees, pillar silhouette error {silhouette} pixels, axis fraction {:.2} %",
        mode.name(),
        if detail { " + detail" } else { "" },
        axis * 100.0
    );
    Some(Guardrails {
        normal_error,
        silhouette,
        axis_fraction: axis,
    })
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
        translucent_layers: 4,
        cutaway: false,
        tuning: vxe_render::SurfaceTuning::default(),
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
    rig.frame.set_lod(lod_forced_1());
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

/// The axis fraction `A`: face mode's undetailed normals are all axes; contour's is printed,
/// and contour still meets the cavity normal error and silhouette guardrails.
#[test]
fn axis_fraction_of_face_and_contour() {
    let Some(face) = guardrails_of(NormalMode::Face, false, false) else {
        return;
    };
    let contour = guardrails_of(NormalMode::Contour, false, true).unwrap();
    println!(
        "axis fraction: face {:.2} %, contour {:.2} %",
        face.axis_fraction * 100.0,
        contour.axis_fraction * 100.0
    );
    assert_eq!(face.axis_fraction, 1.0, "face mode's normals are axes");
    assert!(
        contour.normal_error <= 12.0,
        "contour {:.2} degrees",
        contour.normal_error
    );
    assert!(
        2 * contour.silhouette <= face.silhouette,
        "contour silhouette {} pixels against face {}",
        contour.silhouette,
        face.silhouette
    );
}

/// A world, its goldens with their lights, and whether its LOD walk is forced to level 1.
type Scene<'a> = (&'a World, &'a [(&'a str, LightParams)], bool);

/// The attribute layout changes no pixel in face mode: `primary.png`, `headlamp.png` and
/// `lod.png` render byte for byte with contour attributes, over one shard and over two to
/// four.
#[test]
fn attributes_leave_the_face_goldens_byte_for_byte() {
    if std::env::var("VXE_BLESS").as_deref() == Ok("1") {
        return;
    }
    let face = LookParams::default();
    let carved = carved_world();
    let lod = lod_scene();
    let Some((device, queue)) = software_device(attr_pool_bytes(&carved).max(64 << 20)) else {
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
            let cap = several.then(|| pages.div_ceil(3) * PAGE_UNITS);
            let mut rig = attr_rig_on(device.clone(), queue.clone(), world, cap);
            let shards = rig.gpu.shards_in_use();
            pages = rig.gpu.pages_issued();
            println!("max_shard_units {cap:?}: {pages} pages over {shards} shards");
            if several {
                assert!((2..=4).contains(&shards), "{shards} shards");
            } else {
                assert_eq!(shards, 1);
            }
            if forced {
                rig.frame.set_lod(lod_forced_1());
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

/// The contour golden scene's dark specks that appear only with the headlamp's shadow ray:
/// contour's count is at most face mode's in the same run, so the shadow ray of a plane hit
/// starts clear of the rock the plane cuts.
#[test]
fn contour_dark_specks_with_and_without_shadows() {
    let world = carved_world();
    let Some(mut rig) = attr_rig(&world) else {
        return;
    };
    let off = LightParams {
        shadows: false,
        ..LightParams::headlamp()
    };
    if std::env::var("VXE_BLESS").as_deref() != Ok("1") {
        // The golden is drawn at the default walk.
        let on_img = rig.render(
            &scene_camera(),
            &LightParams::headlamp(),
            look(NormalMode::Contour, ViewMode::Shaded),
        );
        let golden = read_png(&golden_path("look_contour.png")).0;
        let differing = on_img.iter().zip(&golden).filter(|(a, b)| a != b).count();
        assert_eq!(
            differing, 0,
            "the shadows-on render is look_contour.png: {differing} bytes differ"
        );
    }
    // The full walk: the claim is the shadow ray's origin, and the coarse walk's own darkening
    // of grazing rays (any 2^3 block with rock is solid) confounds it.
    rig.frame.set_lighting(vxe_render::Lighting {
        shadow_walk: vxe_render::ShadowWalk::Full,
        ..rig.frame.lighting()
    });
    let mut only_with = Vec::new();
    for mode in [NormalMode::Contour, NormalMode::Face] {
        let render = |rig: &mut LookRig, light: &LightParams| {
            rig.render(&scene_camera(), light, look(mode, ViewMode::Shaded))
        };
        let on_img = render(&mut rig, &LightParams::headlamp());
        let off_img = render(&mut rig, &off);
        let on = dark_specks(&on_img);
        let off = dark_specks(&off_img);
        let only_on = on.iter().filter(|i| off.binary_search(i).is_err()).count();
        println!(
            "{} ({WIDTH}x{HEIGHT}, headlamp): dark specks with shadows {}, without {}, only with {only_on}",
            mode.name(),
            on.len(),
            off.len()
        );
        only_with.push(only_on);
    }
    assert!(
        only_with[0] <= only_with[1],
        "contour {} only-with-shadows specks against face {}",
        only_with[0],
        only_with[1]
    );
}

fn detail_look(normal_mode: NormalMode, view: ViewMode) -> LookParams {
    LookParams {
        normal_mode,
        detail: true,
        view,
        ..LookParams::default()
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
    rig.frame.set_lod(lod_forced_1());
    let img = rig.render(&scene_camera(), &LightParams::at_eye(), on);
    let golden = read_png(&golden_path("lod.png")).0;
    let differing = img.iter().zip(&golden).filter(|(a, b)| a != b).count();
    assert_eq!(differing, 0, "lod.png: {differing} bytes differ");
}

/// Rock at `detail = [4.0, 0.3]` changes at least 20 % of the headlamp scene's pixels by more
/// than 8/255 against face mode.
#[test]
fn rock_detail_golden() {
    let table = spike_materials();
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
    let world = pillar_world_with(spike_materials());
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

/// A material row is 64 B on the GPU, and the live mirror bytes are the slots' units, the
/// 2112-byte records, the table and those rows, nothing else.
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
        gpu.live_units() * 64 + gpu.mirrored_chunks() as u64 * 2112 + window * 4 + rows * 64
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

// ---- Crack classes: how each primary ray of the contour look reached its hit ----

use support::rigs::{
    POSE_SIZE, axis_slab_camera, axis_slab_world, corner_camera, corner_world,
    diagonal_slab_camera, diagonal_slab_world, fit_rig_on, floor_camera, floor_world, pose_level,
    pose_rig_on, tilted_slab_camera, tilted_slab_world, wall_camera, wall_world,
};
use vxe_render::{CrackShares, Exposure, SurfaceFit, readback};

/// The look the fly-through starts in (`LookPreset::DEFAULT`, detail on) in `view`.
fn start_look(view: ViewMode) -> LookParams {
    let p = flythrough::LookPreset::DEFAULT;
    LookParams {
        normal_mode: p.normal_mode,
        detail: true,
        view,
        tuning: p.tuning,
        ..LookParams::default()
    }
}

/// Frames a crack reading renders in each view before it reads the last: the lamp's term is
/// whole from the first frame; the level's emissive estimate needs a few.
const CRACK_FRAMES: u32 = 8;

/// The analytic window rule: an analytic scene's surface is cut where it meets the mirror
/// window's faces, so its bars count only hits whose hit point lies at least this many voxels
/// inside the window on every axis.
const WINDOW_MARGIN: f32 = 2.0;

/// `world`'s mirror window (the analytic rigs mirror the whole extent) as its `[min, max)`
/// voxel bounds per axis.
fn analytic_window(world: &World) -> [[f32; 3]; 2] {
    let e = world.extent();
    [e.min, e.max].map(|c| c.min_voxel().0.map(|v| v as f32))
}

/// A crack reading of one scene in one look: the cracks view's shares over the counted hits
/// (every hit, or under the analytic window rule those at least [`WINDOW_MARGIN`] inside the
/// window), the hits left out, the counted class 1 and 2 pixels whose shaded display luma is
/// dark (under 32/255) or bright (over its 5 x 5 median by more than 48/255), the shares over
/// every hit, and the images and per-pixel classes the side-by-sides draw.
struct CrackReading {
    shares: CrackShares,
    excluded: u64,
    all: CrackShares,
    dark: u64,
    bright: u64,
    shaded: Vec<u8>,
    cracks: Vec<u8>,
    /// Per pixel, the crack class of a hit, `None` on a miss.
    classes: Vec<Option<u8>>,
    size: [u32; 2],
}

impl CrackReading {
    /// `| scene | look | hits | left out | clean % | 1 % | 2 % | 3 % | 1+2 px | dark % |
    /// bright % | self-hit % |`, the dark and bright shares of the class 1 and 2 pixels.
    fn row(&self, scene: &str, look: &str) -> String {
        let s = &self.shares;
        let cracked = s.classes[1] + s.classes[2];
        let of = |n: u64| 100.0 * n as f64 / cracked.max(1) as f64;
        format!(
            "| {scene} | {look} | {} | {} | {:.2} | {:.3} | {:.3} | {:.3} | {cracked} | {:.1} | {:.1} | {:.3} |",
            s.hits,
            self.excluded,
            100.0 * s.share(0),
            100.0 * s.share(1),
            100.0 * s.share(2),
            100.0 * s.share(3),
            of(self.dark),
            of(self.bright),
            100.0 * s.self_hit_share()
        )
    }
}

const CRACK_HEADER: &str = "| scene | look | hits | left out | clean % | class 1 % | class 2 % | class 3 % | class 1+2 px | of them dark % | of them bright % | self-hit % |\n|---|---|---|---|---|---|---|---|---|---|---|---|";

/// Rec. 709 luma of pixel `i` of an RGBA image.
fn luma_at(rgba: &[u8], i: usize) -> f64 {
    let p = &rgba[i * 4..i * 4 + 3];
    0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2])
}

/// Renders `camera` lit by `light` in `look`'s shaded view and then its cracks view,
/// [`CRACK_FRAMES`] frames each, and reads the last frame of each. With `window` (its
/// `[min, max)` voxel bounds per axis, [`analytic_window`]) the shares count only the hits the analytic window
/// rule keeps; without, every hit.
fn crack_reading(
    rig: &mut LookRig,
    camera: &vxe_render::Camera,
    light: &LightParams,
    look: LookParams,
    window: Option<[[f32; 3]; 2]>,
) -> CrackReading {
    let size = rig.frame.internal_size();
    let mut run = |view: ViewMode| {
        rig.frame.set_look(LookParams { view, ..look });
        for _ in 0..CRACK_FRAMES {
            rig.frame
                .render(&rig.device, &rig.queue, &rig.gpu, camera, light);
        }
        readback::read_rgba8(&rig.device, &rig.queue, rig.frame.target())
    };
    let shaded = run(ViewMode::Shaded);
    let cracks = run(ViewMode::Cracks);
    let layers = readback::read_rgba16f(&rig.device, &rig.queue, rig.frame.layers());
    let records = readback::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
    let all = CrackShares::from_layers(&layers, &records);
    let positions = readback::read_rgba32f(&rig.device, &rig.queue, rig.frame.hit_positions());
    let classes: Vec<Option<u8>> = layers
        .iter()
        .zip(&records)
        .map(|(l, r)| (r[3] & 1 == 1).then(|| l[0].round().clamp(0.0, 3.0) as u8))
        .collect();
    let counted = |i: usize| {
        window.is_none_or(|[lo, hi]| {
            (0..3).all(|k| {
                let c = positions[i][k];
                c >= lo[k] + WINDOW_MARGIN && c <= hi[k] - WINDOW_MARGIN
            })
        })
    };
    let (w, h) = (size[0] as usize, size[1] as usize);
    let (mut shares, mut excluded) = (CrackShares::default(), 0);
    let (mut dark, mut bright) = (0, 0);
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let Some(class) = classes[i] else {
                continue;
            };
            if !counted(i) {
                excluded += 1;
                continue;
            }
            shares.hits += 1;
            shares.classes[usize::from(class)] += 1;
            shares.self_hits += u64::from(layers[i][1] > 0.5);
            if !matches!(class, 1 | 2) {
                continue;
            }
            let l = luma_at(&shaded, i);
            if l < 32.0 {
                dark += 1;
            }
            let mut n = Vec::with_capacity(25);
            for dy in -2i64..=2 {
                for dx in -2i64..=2 {
                    let (xx, yy) = (x as i64 + dx, y as i64 + dy);
                    if (0..w as i64).contains(&xx) && (0..h as i64).contains(&yy) {
                        n.push(luma_at(&shaded, yy as usize * w + xx as usize));
                    }
                }
            }
            n.sort_by(f64::total_cmp);
            if l > n[n.len() / 2] + 48.0 {
                bright += 1;
            }
        }
    }
    CrackReading {
        shares,
        excluded,
        all,
        dark,
        bright,
        shaded,
        cracks,
        classes,
        size,
    }
}

/// The tilted slab (0), the 45-degree slab (1), the diagonal slab (2), the floor (3) or the
/// wall (4), on its own contour rig in the start look, lit by the headlamp, at the look rigs' size, under the analytic
/// window rule; `None` without a software adapter.
fn slab_reading(slab: u32) -> Option<CrackReading> {
    let table = flythrough::LookPreset::DEFAULT.materials();
    let (world, camera) = match slab {
        0 => (tilted_slab_world(&table), tilted_slab_camera()),
        1 => (axis_slab_world(&table), axis_slab_camera()),
        2 => (diagonal_slab_world(&table), diagonal_slab_camera()),
        3 => (floor_world(&table), floor_camera()),
        _ => (wall_world(&table), wall_camera()),
    };
    let (device, queue) = software_device(attr_pool_bytes(&world).max(64 << 20))?;
    let fit = flythrough::LookPreset::DEFAULT.tuning.fit;
    let mut rig = fit_rig_on(device, queue, &world, fit, [WIDTH, HEIGHT]);
    Some(crack_reading(
        &mut rig,
        &camera,
        &LightParams::headlamp(),
        start_look(ViewMode::Cracks),
        Some(analytic_window(&world)),
    ))
}

/// A flat floor (rock at `z < 64`) and a flat vertical wall (rock at `x >= 80`), each seen
/// obliquely in the start look, have no crack under the analytic window rule: their surface
/// voxels share one neighbourhood and the plane code holds the axis normal exactly
/// (ADR-0038), so they share one plane and no ray reads class 1 or 2. The cracks view's
/// colours decode to the same shares as the hit pass's layers.
#[test]
fn axis_planes_have_no_cracks() {
    let Some(floor) = slab_reading(3) else {
        return;
    };
    let wall = slab_reading(4).expect("the floor's adapter");
    println!("{CRACK_HEADER}");
    for (name, r) in [("floor", &floor), ("wall", &wall)] {
        println!("{}", r.row(name, "start look"));
        assert!(r.shares.hits > 0, "{name}");
        assert_eq!(r.shares.classes[1], 0, "{name}: {:?}", r.shares);
        assert_eq!(r.shares.classes[2], 0, "{name}: {:?}", r.shares);
        assert_eq!(
            CrackShares::from_rgba8(&r.cracks),
            r.all,
            "{name}: the cracks view's colours decode to the layers' classes"
        );
    }
}

/// The crack readings' looks: the start look, and its tuning with one part swept.
fn crack_looks() -> Vec<(&'static str, SurfaceFit)> {
    use vxe_render::{PlaneFit, PlaneOffset};
    let start = flythrough::LookPreset::DEFAULT.tuning.fit;
    vec![
        ("start look", start),
        (
            "fit uniform27",
            SurfaceFit {
                fit: PlaneFit::Uniform27,
                ..start
            },
        ),
        (
            "offset volume",
            SurfaceFit {
                offset: PlaneOffset::VolumeMatched,
                ..start
            },
        ),
    ]
}

/// The look of a crack reading at `fit`: the start look with that plane fit.
fn look_at_fit(fit: SurfaceFit) -> LookParams {
    let mut look = start_look(ViewMode::Cracks);
    look.tuning.fit = fit;
    look
}

/// Where the side-by-sides go: the worktree's untracked `live/cracks/`.
fn cracks_dir() -> std::path::PathBuf {
    let d = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../live/cracks");
    std::fs::create_dir_all(&d).expect("live/cracks is writable");
    d
}

/// The density tile's edge in pixels.
const CRACK_TILE: usize = 24;

/// Writes `name.png`: the shaded image, the cracks view, and the shaded image with each 24 x
/// 24-pixel tile tinted red by its share of class 1 and 2 pixels (full red at 5 %), side by
/// side.
fn write_side_by_side(r: &CrackReading, name: &str) {
    let (w, h) = (r.size[0] as usize, r.size[1] as usize);
    let mut out = vec![0u8; 3 * w * h * 4];
    let tiles_x = w.div_ceil(CRACK_TILE);
    let mut density = vec![0.0f64; tiles_x * h.div_ceil(CRACK_TILE)];
    for y in 0..h {
        for x in 0..w {
            if matches!(r.classes[y * w + x], Some(1 | 2)) {
                density[(y / CRACK_TILE) * tiles_x + x / CRACK_TILE] += 1.0;
            }
        }
    }
    let area = (CRACK_TILE * CRACK_TILE) as f64;
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let a = (density[(y / CRACK_TILE) * tiles_x + x / CRACK_TILE] / area / 0.05).min(1.0);
            for (panel, src) in [(0, &r.shaded), (1, &r.cracks), (2, &r.shaded)] {
                let o = (y * 3 * w + panel * w + x) * 4;
                for k in 0..3 {
                    let v = f64::from(src[i * 4 + k]);
                    let tinted = if panel == 2 {
                        let target = if k == 0 { 255.0 } else { 0.0 };
                        v + (target - v) * 0.75 * a
                    } else {
                        v
                    };
                    out[o + k] = tinted.round() as u8;
                }
                out[o + 3] = 255;
            }
        }
    }
    let path = cracks_dir().join(format!("{name}.png"));
    write_png(&path, &out, 3 * w as u32, h as u32);
    println!("wrote {}", path.display());
}

/// The crack class table for every scene, pose and swept part (reading only; it asserts
/// nothing): the tilted slab, the concave corner, the 45-degree slab,
/// the floor, the wall, the diagonal slab, the cavity and the pillar at the look rigs' size
/// under the analytic window rule, and the level at `VEIN_POSE` and `PATCH_POSE` at 384 x 216
/// with the pose's exposure, indirect light off, each in the start look and with its fit or
/// offset swept, lamp on, and the start look's poses with the lamp off too. The level's
/// side-by-sides go to `live/cracks/`. On the adapter `VXE_TEST_ADAPTER` names (lavapipe
/// unset):
/// `cargo nextest run -p flythrough --test look --run-ignored only --no-capture crack_classes`.
#[test]
#[ignore = "a reading: prints the crack class table and writes the side-by-sides"]
fn crack_classes() {
    let table = flythrough::LookPreset::DEFAULT.materials();
    let analytic: Vec<(&str, World, vxe_render::Camera)> = vec![
        (
            "tilted slab",
            tilted_slab_world(&table),
            tilted_slab_camera(),
        ),
        ("concave corner", corner_world(&table), corner_camera()),
        (
            "45-degree slab",
            axis_slab_world(&table),
            axis_slab_camera(),
        ),
        ("floor", floor_world(&table), floor_camera()),
        ("wall", wall_world(&table), wall_camera()),
        (
            "diagonal slab",
            diagonal_slab_world(&table),
            diagonal_slab_camera(),
        ),
        ("cavity", cavity_world(), cavity_camera()),
        ("pillar", pillar_world_with(table.clone()), pillar_camera()),
    ];
    let level = pose_level();
    let pool = attr_pool_bytes(&level).max(256 << 20);
    let Some((device, queue, adapter)) = reading_device(pool) else {
        return;
    };
    println!("adapter {adapter}");
    let mut rows = Vec::new();
    for (look_name, fit) in crack_looks() {
        for (name, world, camera) in &analytic {
            let mut rig = fit_rig_on(device.clone(), queue.clone(), world, fit, [WIDTH, HEIGHT]);
            let r = crack_reading(
                &mut rig,
                camera,
                &LightParams::headlamp(),
                look_at_fit(fit),
                Some(analytic_window(world)),
            );
            println!("{}", r.row(name, look_name));
            rows.push(r.row(name, look_name));
        }
        for pose in flythrough::POSES {
            // The start look's poses are read with the lamp off too: the owner saw the wedges
            // and the triangles with it off, where the shaded image is not lit to white.
            let lamps: &[bool] = if look_name == "start look" {
                &[true, false]
            } else {
                &[true]
            };
            for &lamp in lamps {
                let mut rig = pose_rig_on(device.clone(), queue.clone(), &level, &pose, fit);
                rig.frame.set_lighting(vxe_render::Lighting {
                    exposure: Exposure::Manual(pose.exposure),
                    indirect: false,
                    lamp,
                    ..rig.frame.lighting()
                });
                assert_eq!(rig.frame.internal_size(), POSE_SIZE);
                let r = crack_reading(
                    &mut rig,
                    &pose.camera(),
                    &LightParams::headlamp(),
                    look_at_fit(fit),
                    None,
                );
                let lamp_tag = if lamp { "" } else { " lamp off" };
                let name = format!("level {}{lamp_tag}", pose.name);
                println!("{}", r.row(&name, look_name));
                rows.push(r.row(&name, look_name));
                let tag = format!("{look_name}{lamp_tag}").replace(' ', "-");
                write_side_by_side(&r, &format!("{}-{tag}", pose.name));
            }
        }
    }
    println!("\nadapter {adapter}\n{CRACK_HEADER}");
    for r in rows {
        println!("{r}");
    }
}

/// The class 1 and 2 pixels the kept rule (snap) read at `VEIN_POSE` and `PATCH_POSE` on
/// lavapipe, lamp on, start look (Plan 0050's Phase 3 table; Phase 2's old rule read 2247 and
/// 5266).
const KEPT_POSE_CRACKS: [u64; 2] = [0, 0];

/// The look the fly-through starts in closes the contour surface's cracks: classes 1 and 2
/// read 0 on the tilted slab, the concave corner and the cavity under the analytic window rule,
/// and at most the kept rule's count at the vein and the patch.
#[test]
fn the_closing_rule_leaves_no_crack() {
    let table = flythrough::LookPreset::DEFAULT.materials();
    let level = pose_level();
    let Some((device, queue)) = software_device(attr_pool_bytes(&level).max(256 << 20)) else {
        return;
    };
    let look = start_look(ViewMode::Cracks);
    let fit = look.tuning.fit;
    println!("{CRACK_HEADER}");
    for (name, world, camera) in [
        (
            "tilted slab",
            tilted_slab_world(&table),
            tilted_slab_camera(),
        ),
        ("concave corner", corner_world(&table), corner_camera()),
        ("cavity", cavity_world(), cavity_camera()),
    ] {
        let mut rig = fit_rig_on(device.clone(), queue.clone(), &world, fit, [WIDTH, HEIGHT]);
        let r = crack_reading(
            &mut rig,
            &camera,
            &LightParams::headlamp(),
            look,
            Some(analytic_window(&world)),
        );
        println!("{}", r.row(name, "start look"));
        assert!(r.shares.hits > 0, "{name}");
        assert_eq!(r.shares.classes[1], 0, "{name}: {:?}", r.shares);
        assert_eq!(r.shares.classes[2], 0, "{name}: {:?}", r.shares);
    }
    for (pose, kept) in flythrough::POSES.into_iter().zip(KEPT_POSE_CRACKS) {
        let mut rig = pose_rig_on(device.clone(), queue.clone(), &level, &pose, fit);
        rig.frame.set_lighting(vxe_render::Lighting {
            exposure: Exposure::Manual(pose.exposure),
            indirect: false,
            ..rig.frame.lighting()
        });
        let r = crack_reading(
            &mut rig,
            &pose.camera(),
            &LightParams::headlamp(),
            look,
            None,
        );
        let name = format!("level {}", pose.name);
        println!("{}", r.row(&name, "start look"));
        assert!(r.shares.hits > 0, "{name}");
        let cracked = r.shares.classes[1] + r.shares.classes[2];
        assert!(cracked <= kept, "{name}: {cracked} cracked against {kept}");
    }
}

/// The output size of [`the_cracks_view_keeps_its_palette_through_the_window_chain`]: the
/// pose rigs' internal size at the window's scale 1.5.
const WINDOW_CHAIN_OUTPUT: [u32; 2] = [576, 324];

/// The cracks view's palette colour of an internal pixel of class `class` with self-hit
/// `self_hit`, for the classes the test reads (composite.wgsl `crack_colour`): class 3 blue,
/// its red at 128 under a self-hit; a class 0 self-hit magenta. `None` for the rest.
fn crack_palette(class: u8, self_hit: bool) -> Option<[u8; 3]> {
    match (class, self_hit) {
        (3, false) => Some([0, 0, 255]),
        (3, true) => Some([128, 0, 255]),
        (0, true) => Some([255, 0, 255]),
        _ => None,
    }
}

/// The cracks view survives the window's post chain: at `VEIN_POSE` in the start look, the
/// pose's exposure, the window's upscaler (ADR-0024's, scale 1.5, jittered, sharpening 0.5)
/// and the default tone map, every output pixel whose nearest internal sample (the one the
/// upscale pass copies in a view other than shaded, at this frame's jitter) is a class 3 hit
/// reads blue and every one whose sample is a self-hit over class 0 reads magenta, within
/// 32/255 per channel of the palette; both kinds are present.
#[test]
fn the_cracks_view_keeps_its_palette_through_the_window_chain() {
    let level = pose_level();
    let Some((device, queue)) = software_device(attr_pool_bytes(&level).max(256 << 20)) else {
        return;
    };
    let pose = flythrough::VEIN_POSE;
    let look = start_look(ViewMode::Cracks);
    let mut rig = pose_rig_on(device, queue, &level, &pose, look.tuning.fit);
    rig.frame = vxe_render::Frame::with_sizes(&rig.device, POSE_SIZE, WINDOW_CHAIN_OUTPUT);
    rig.frame.set_jitter(&rig.device, true);
    rig.frame
        .set_upscaler(vxe_render::marcher::Upscaler::default());
    rig.frame.set_sharpness(&rig.queue, 0.5);
    rig.frame.set_lighting(vxe_render::Lighting {
        exposure: Exposure::Manual(pose.exposure),
        ..rig.frame.lighting()
    });
    assert_eq!(
        rig.frame.lighting().tone_map,
        vxe_render::ToneMap::default()
    );
    rig.frame.set_look(look);
    let camera = pose.camera();
    let light = LightParams::headlamp();
    let mut last = 0;
    for _ in 0..CRACK_FRAMES {
        last = rig.frame.frame_index();
        rig.frame
            .render(&rig.device, &rig.queue, &rig.gpu, &camera, &light);
    }
    let out = readback::read_rgba8(&rig.device, &rig.queue, rig.frame.target());
    let layers = readback::read_rgba16f(&rig.device, &rig.queue, rig.frame.layers());
    let records = readback::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
    let [iw, ih] = POSE_SIZE.map(|v| v as usize);
    let [ow, oh] = WINDOW_CHAIN_OUTPUT.map(|v| v as usize);
    assert_eq!(out.len(), ow * oh * 4, "the target is the output size");
    let jitter = vxe_render::jitter_at(last, vxe_render::marcher::JITTER_PHASES);
    let ratio = [iw as f32 / ow as f32, ih as f32 / oh as f32];
    let (mut blue, mut magenta, mut worst) = (0, 0, 0u8);
    for y in 0..oh {
        for x in 0..ow {
            let near = |o: usize, k: usize, n: usize| {
                ((o as f32 + 0.5) * ratio[k] - jitter[k])
                    .floor()
                    .clamp(0.0, n as f32 - 1.0) as usize
            };
            let i = near(y, 1, ih) * iw + near(x, 0, iw);
            if records[i][3] & 1 != 1 {
                continue;
            }
            let class = layers[i][0].round().clamp(0.0, 3.0) as u8;
            let Some(want) = crack_palette(class, layers[i][1] > 0.5) else {
                continue;
            };
            if class == 3 {
                blue += 1;
            } else {
                magenta += 1;
            }
            let o = (y * ow + x) * 4;
            for k in 0..3 {
                let d = out[o + k].abs_diff(want[k]);
                worst = worst.max(d);
                assert!(
                    d <= 32,
                    "output pixel ({x}, {y}): {:?} against the palette's {want:?}",
                    &out[o..o + 3]
                );
            }
        }
    }
    println!(
        "window chain at the vein: {blue} blue and {magenta} magenta output pixels, worst channel off by {worst}/255"
    );
    assert!(blue > 0 && magenta > 0, "blue {blue}, magenta {magenta}");
}

// ---- One-voxel boxes: the box metric and the tile-contrast metric ----

use support::rigs::Feature;

/// A hit point lies on a cube face when one of its coordinates is within this of an integer.
const FACE_EPS: f32 = 1e-3;

/// Under the field look a hit on a cube face is a box only when its shading normal (the hit
/// record's, detail off) lies within this many degrees of the face's outward axis normal. The
/// field surface is curved and crosses voxel face planes: a hit on it that happens to lie
/// within [`FACE_EPS`] of a face plane, with a normal far from that face's, shows no cube face.
/// The plane looks keep the position-only test.
const FIELD_BOX_DEGREES: f32 = 15.0;

/// Adjacent hit pixels whose hit points lie within this many voxels of each other are on one
/// surface (no depth edge) for the tile-contrast metric.
const TILE_REACH: f32 = 1.5;

/// A pair of such pixels is a tile edge when their shading normals differ by more than this
/// many degrees.
const TILE_DEGREES: f64 = 15.0;

/// Whether voxel `v` of `world` is solid (not air; outside the world is air).
fn solid_at(world: &World, v: [i32; 3]) -> bool {
    world
        .material_at(vxe_world::VoxelCoord(v))
        .is_some_and(|m| m != vxe_world::AIR)
}

/// The face neighbours of `v` that are air in `world`.
fn air_faces(world: &World, v: [i32; 3]) -> u32 {
    let mut n = 0;
    for k in 0..3 {
        for s in [-1, 1] {
            let mut u = v;
            u[k] += s;
            n += u32::from(!solid_at(world, u));
        }
    }
    n
}

/// The solid voxels of the 27 around `v` in `world`, `v` itself included.
fn count27(world: &World, v: [i32; 3]) -> u32 {
    let mut n = 0;
    for z in -1..=1 {
        for y in -1..=1 {
            for x in -1..=1 {
                n += u32::from(solid_at(world, [v[0] + x, v[1] + y, v[2] + z]));
            }
        }
    }
    n
}

/// The box voxel hit point `p` lies on, if any: `p` lies on a cube face (a coordinate within
/// [`FACE_EPS`] of an integer) between a solid voxel and an air voxel of `world`, the solid one
/// has two or more air face neighbours, and `p` lies on none of that voxel's faces toward a
/// solid neighbour. A flat floor's face hits have one air face; a hit between two solid voxels
/// lies on no exposed face; a hit on the edge where an exposed face meets a face between two
/// solid voxels (a plane hit along the outline of a feature's footprint, on the surface under
/// it) is the surface's. `facing` is asked of the face's outward axis normal (from the solid
/// voxel toward the air one) and must hold too: [`box_reading`] passes the field look's
/// normal condition through it, which holds for every face in every other look.
fn box_voxel(world: &World, p: [f32; 3], facing: impl Fn([f32; 3]) -> bool) -> Option<[i32; 3]> {
    let near = |c: f32| (c - c.round()).abs() <= FACE_EPS;
    for k in 0..3 {
        if !near(p[k]) {
            continue;
        }
        let r = p[k].round();
        let mut lo = p.map(|c| c.floor() as i32);
        lo[k] = r as i32 - 1;
        let mut hi = lo;
        hi[k] = r as i32;
        let (sl, sh) = (solid_at(world, lo), solid_at(world, hi));
        if sl == sh {
            continue;
        }
        let v = if sl { lo } else { hi };
        let mut outward = [0.0; 3];
        outward[k] = if sl { 1.0 } else { -1.0 };
        if !facing(outward) {
            continue;
        }
        let on_buried_face = (0..3).filter(|&j| j != k && near(p[j])).any(|j| {
            let r = p[j].round() as i32;
            if r != v[j] && r != v[j] + 1 {
                return false;
            }
            let mut u = v;
            u[j] += if r == v[j] { -1 } else { 1 };
            solid_at(world, u)
        });
        if !on_buried_face && air_faces(world, v) >= 2 {
            return Some(v);
        }
    }
    None
}

/// A box reading of one scene in one look: the crack reading, the box pixels among the counted
/// hits sorted by their voxel's 27-count (under 14, 14 and over) and by its air faces (2, 3,
/// 4 or more), and the tile-contrast share: of the counted adjacent hit-pixel pairs on one
/// surface, those whose normals-view normals (detail off) differ by more than
/// [`TILE_DEGREES`].
struct BoxReading {
    crack: CrackReading,
    normals: Vec<u8>,
    /// Per pixel, the box class: 0 none, 1 a box voxel with a 27-count under 14, 2 one of 14
    /// or more.
    boxes: Vec<u8>,
    box_px: u64,
    by_count: [u64; 2],
    by_air: [u64; 3],
    pairs: u64,
    edges: u64,
}

impl BoxReading {
    fn row(&self, scene: &str, look: &str) -> String {
        let s = &self.crack.shares;
        format!(
            "| {scene} | {look} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {:.3} | {:.3} | {:.3} | {:.3} |",
            s.hits,
            self.crack.excluded,
            self.box_px,
            self.by_count[0],
            self.by_count[1],
            self.by_air[0],
            self.by_air[1],
            self.by_air[2],
            self.pairs,
            100.0 * self.edges as f64 / self.pairs.max(1) as f64,
            100.0 * s.share(1),
            100.0 * s.share(2),
            100.0 * s.share(3),
        )
    }
}

const BOX_HEADER: &str = "| scene | look | hits | left out | box px | count < 14 | count >= 14 | 2 air | 3 air | 4+ air | pairs | tile edges % | class 1 % | class 2 % | class 3 % |\n|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|";

/// The angle in degrees between two normals-view pixels' normals.
fn normal_angle(a: &[u8], b: &[u8]) -> f64 {
    let (a, b) = (decode_normal(a), decode_normal(b));
    if a.length() < 0.5 || b.length() < 0.5 {
        return 0.0;
    }
    f64::from(a.normalize().dot(b.normalize()).clamp(-1.0, 1.0))
        .acos()
        .to_degrees()
}

/// [`crack_reading`] of `camera` in `look`, then its normals view with detail off, and the box
/// and tile-contrast metrics over the hits it counts (`window` as there), `world` the CPU world
/// the box metric reads.
fn box_reading(
    rig: &mut LookRig,
    world: &World,
    camera: &vxe_render::Camera,
    light: &LightParams,
    look: LookParams,
    window: Option<[[f32; 3]; 2]>,
) -> BoxReading {
    let crack = crack_reading(rig, camera, light, look, window);
    rig.frame.set_look(LookParams {
        view: ViewMode::Normals,
        detail: false,
        ..look
    });
    for _ in 0..CRACK_FRAMES {
        rig.frame
            .render(&rig.device, &rig.queue, &rig.gpu, camera, light);
    }
    let normals = readback::read_rgba8(&rig.device, &rig.queue, rig.frame.target());
    let positions = readback::read_rgba32f(&rig.device, &rig.queue, rig.frame.hit_positions());
    let records = readback::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
    let field = look.tuning.model == vxe_render::SurfaceModel::Field;
    let cos_box = FIELD_BOX_DEGREES.to_radians().cos();
    let [w, h] = crack.size.map(|v| v as usize);
    let counted: Vec<bool> = (0..w * h)
        .map(|i| {
            crack.classes[i].is_some()
                && window.is_none_or(|[lo, hi]| {
                    (0..3).all(|k| {
                        let c = positions[i][k];
                        c >= lo[k] + WINDOW_MARGIN && c <= hi[k] - WINDOW_MARGIN
                    })
                })
        })
        .collect();
    let mut boxes = vec![0u8; w * h];
    let (mut box_px, mut by_count, mut by_air) = (0, [0; 2], [0; 3]);
    for i in 0..w * h {
        if !counted[i] {
            continue;
        }
        let p = [positions[i][0], positions[i][1], positions[i][2]];
        let n = support::rigs::octa16_normal(records[i][0], records[i][1]);
        let facing = |a: [f32; 3]| !field || (0..3).map(|k| a[k] * n[k]).sum::<f32>() >= cos_box;
        let Some(v) = box_voxel(world, p, facing) else {
            continue;
        };
        let high = count27(world, v) >= 14;
        boxes[i] = 1 + u8::from(high);
        box_px += 1;
        by_count[usize::from(high)] += 1;
        by_air[(air_faces(world, v) as usize).clamp(2, 4) - 2] += 1;
    }
    let (mut pairs, mut edges) = (0, 0);
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if !counted[i] {
                continue;
            }
            for j in [(x + 1 < w).then_some(i + 1), (y + 1 < h).then_some(i + w)]
                .into_iter()
                .flatten()
            {
                if !counted[j] {
                    continue;
                }
                let d: f32 = (0..3)
                    .map(|k| (positions[i][k] - positions[j][k]).powi(2))
                    .sum();
                if d.sqrt() > TILE_REACH {
                    continue;
                }
                pairs += 1;
                if normal_angle(&normals[i * 4..i * 4 + 3], &normals[j * 4..j * 4 + 3])
                    > TILE_DEGREES
                {
                    edges += 1;
                }
            }
        }
    }
    BoxReading {
        crack,
        normals,
        boxes,
        box_px,
        by_count,
        by_air,
        pairs,
        edges,
    }
}

/// Where the box side-by-sides go: the worktree's untracked `live/boxes/`.
fn boxes_dir() -> std::path::PathBuf {
    let d = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../live/boxes");
    std::fs::create_dir_all(&d).expect("live/boxes is writable");
    d
}

/// Writes `name.png`: the shaded image, the normals view, and the shaded image with every box
/// pixel painted, side by side: red where the box voxel's 27-count is 14 or more, yellow
/// where it is under 14 (a protrusion).
fn write_box_side_by_side(r: &BoxReading, name: &str) {
    let [w, h] = r.crack.size.map(|v| v as usize);
    let mut out = vec![0u8; 3 * w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            for (panel, src) in [(0, &r.crack.shaded), (1, &r.normals), (2, &r.crack.shaded)] {
                let o = (y * 3 * w + panel * w + x) * 4;
                if panel == 2 && r.boxes[i] != 0 {
                    let c = if r.boxes[i] == 2 {
                        [255, 0, 0]
                    } else {
                        [255, 255, 0]
                    };
                    out[o..o + 3].copy_from_slice(&c);
                } else {
                    out[o..o + 3].copy_from_slice(&src[i * 4..i * 4 + 3]);
                }
                out[o + 3] = 255;
            }
        }
    }
    let path = boxes_dir().join(format!("{name}.png"));
    write_png(&path, &out, 3 * w as u32, h as u32);
    println!("wrote {}", path.display());
}

/// The box reading of `feature` in `look` on `device`, at the look rigs' size under the
/// analytic window rule, lit by the headlamp.
fn feature_reading(
    device: &vxe_render::wgpu::Device,
    queue: &vxe_render::wgpu::Queue,
    feature: Feature,
    look: LookParams,
) -> BoxReading {
    let table = flythrough::LookPreset::DEFAULT.materials();
    let world = feature.world(&table);
    let mut rig = fit_rig_on(
        device.clone(),
        queue.clone(),
        &world,
        look.tuning.fit,
        [WIDTH, HEIGHT],
    );
    with_field_pool(&mut rig, &world, &look);
    box_reading(
        &mut rig,
        &world,
        &feature.camera(),
        &LightParams::headlamp(),
        look,
        Some(analytic_window(&world)),
    )
}

/// The box reading of the level at `pose` in `look` (384 x 216, the pose's exposure, indirect
/// off, lamp on or off), every hit counted.
fn pose_box_reading(
    device: &vxe_render::wgpu::Device,
    queue: &vxe_render::wgpu::Queue,
    level: &World,
    pose: &flythrough::Pose,
    look: LookParams,
    lamp: bool,
) -> BoxReading {
    let mut rig = pose_rig_on(device.clone(), queue.clone(), level, pose, look.tuning.fit);
    with_field_pool(&mut rig, level, &look);
    rig.frame.set_lighting(vxe_render::Lighting {
        exposure: Exposure::Manual(pose.exposure),
        indirect: false,
        lamp,
        ..rig.frame.lighting()
    });
    assert_eq!(rig.frame.internal_size(), POSE_SIZE);
    box_reading(
        &mut rig,
        level,
        &pose.camera(),
        &LightParams::headlamp(),
        look,
        None,
    )
}

/// One-voxel features render as boxes in the look the fly-through starts in: on lavapipe under
/// the analytic window rule, the rod, the lone cube, the groove and the wall pillar each read at
/// least one box pixel (a hit on an exposed cube face of a voxel with two or more air faces)
/// and the bare floor reads none.
#[test]
fn one_voxel_features_render_as_boxes_today() {
    let Some((device, queue)) = software_device(64 << 20) else {
        return;
    };
    let look = start_look(ViewMode::Shaded);
    println!("{BOX_HEADER}");
    for feature in [
        Feature::Floor,
        Feature::Rod,
        Feature::LoneCube,
        Feature::Groove,
        Feature::WallPillar,
    ] {
        let r = feature_reading(&device, &queue, feature, look);
        println!("{}", r.row(feature.name(), "start look"));
        assert!(r.crack.shares.hits > 0, "{}", feature.name());
        if feature == Feature::Floor {
            assert_eq!(r.box_px, 0, "the bare floor has no box");
        } else {
            assert!(r.box_px >= 1, "{}: no box pixel", feature.name());
        }
    }
}

/// The box readings' looks: the start look, its tuning with the volume-matched offset, with
/// the majority filter, with the normal field, with both, and the `cubes-field` preset.
fn box_looks() -> Vec<(&'static str, LookParams)> {
    let mut looks: Vec<(&'static str, LookParams)> = crack_looks()
        .into_iter()
        .filter(|(name, _)| *name != "fit uniform27")
        .map(|(name, fit)| {
            let mut look = start_look(ViewMode::Shaded);
            look.tuning.fit = fit;
            (name, look)
        })
        .collect();
    let majority = with_filter(
        start_look(ViewMode::Shaded),
        vxe_render::OccupancyFilter::Majority,
    );
    looks.push(("majority", majority));
    looks.push(("field", field_on(start_look(ViewMode::Shaded))));
    looks.push(("majority + field", field_on(majority)));
    looks.push((
        "cubes-field",
        preset_look(flythrough::LookPreset::CUBES_FIELD, ViewMode::Shaded),
    ));
    looks
}

/// The box and tile-contrast table for every one-voxel scene and both poses in each of
/// [`box_looks`] (reading only; it asserts nothing): the scenes at the look rigs' size under
/// the analytic window rule, the level at `VEIN_POSE` and `PATCH_POSE` at 384 x 216 with the
/// pose's exposure and indirect light off, lamp on and off. Every reading's side-by-side goes
/// to `live/boxes/`. `VXE_BOX_TAG` (optional) suffixes the look names, for a scratch build. On
/// the adapter `VXE_TEST_ADAPTER` names (lavapipe unset):
/// `cargo nextest run -p flythrough --test look --run-ignored only --no-capture box_readings`.
#[test]
#[ignore = "a reading: prints the box table and writes the side-by-sides"]
fn box_readings() {
    let level = pose_level();
    let pool = attr_pool_bytes(&level).max(256 << 20);
    let Some((device, queue, adapter)) = reading_device(pool) else {
        return;
    };
    let tag = std::env::var("VXE_BOX_TAG").unwrap_or_default();
    println!("adapter {adapter}");
    let mut rows = Vec::new();
    for (look_name, look) in box_looks() {
        let look_name = format!("{look_name}{tag}");
        for feature in Feature::ALL {
            let r = feature_reading(&device, &queue, feature, look);
            println!("{}", r.row(feature.name(), &look_name));
            rows.push(r.row(feature.name(), &look_name));
            let file = format!("{}-{look_name}", feature.name()).replace(' ', "-");
            write_box_side_by_side(&r, &file);
        }
        for pose in flythrough::POSES {
            for lamp in [true, false] {
                let r = pose_box_reading(&device, &queue, &level, &pose, look, lamp);
                let lamp_tag = if lamp { "" } else { " lamp off" };
                let name = format!("level {}{lamp_tag}", pose.name);
                println!("{}", r.row(&name, &look_name));
                rows.push(r.row(&name, &look_name));
                let file = format!("{}-{look_name}{lamp_tag}", pose.name).replace(' ', "-");
                write_box_side_by_side(&r, &file);
            }
        }
    }
    println!("\nadapter {adapter}\n{BOX_HEADER}");
    for r in rows {
        println!("{r}");
    }
}

// ---- The plane fit on a majority-filtered occupancy (ADR-0040) ----

/// `look` with the plane fit's occupancy filter set to `filter`.
fn with_filter(look: LookParams, filter: vxe_render::OccupancyFilter) -> LookParams {
    let mut look = look;
    look.tuning.fit.filter = filter;
    look
}

/// The hit mask and the normals view (detail off) of `world` from `camera` in `look`, one frame
/// each, lit by the headlamp, and the hit positions.
fn mask_and_normals(
    device: &vxe_render::wgpu::Device,
    queue: &vxe_render::wgpu::Queue,
    world: &World,
    camera: &vxe_render::Camera,
    look: LookParams,
) -> (Vec<u8>, Vec<u8>, Vec<[f32; 4]>) {
    let mut rig = fit_rig_on(
        device.clone(),
        queue.clone(),
        world,
        look.tuning.fit,
        [WIDTH, HEIGHT],
    );
    with_field_pool(&mut rig, world, &look);
    let light = LightParams::headlamp();
    let mask = rig.render(
        camera,
        &light,
        LookParams {
            view: ViewMode::HitMask,
            ..look
        },
    );
    let normals = rig.render(
        camera,
        &light,
        LookParams {
            view: ViewMode::Normals,
            detail: false,
            ..look
        },
    );
    let positions = readback::read_rgba32f(&rig.device, &rig.queue, rig.frame.hit_positions());
    (mask, normals, positions)
}

/// Whether the hit at `position` (a miss reads `t = BIG`) is kept by the analytic window rule
/// in `window`: a miss, or a hit at least [`WINDOW_MARGIN`] inside the window on every axis.
fn kept_by_window(position: [f32; 4], [lo, hi]: [[f32; 3]; 2]) -> bool {
    position[3] > 1e20
        || (0..3)
            .all(|k| position[k] >= lo[k] + WINDOW_MARGIN && position[k] <= hi[k] - WINDOW_MARGIN)
}

/// The plane fit on the majority-filtered occupancy melts one-voxel features into the surface
/// under them: on lavapipe under the analytic window rule the rod, the lone cube and the wall
/// pillar read no box pixel in the start look with the filter; the bare floor and Plan 0050's
/// floor and wall read the same hit mask and normals view byte for byte with and without it at
/// every pixel the analytic window rule keeps in both (every voxel beside their surfaces
/// counts 18 there, so nothing is filtered; the pixels left out are printed); and the tilted and
/// 45-degree slabs, whose images may move under it, read no class 1 or 2 crack.
#[test]
fn the_majority_fit_melts_one_voxel_features() {
    let Some((device, queue)) = software_device(64 << 20) else {
        return;
    };
    let start = start_look(ViewMode::Shaded);
    let majority = with_filter(start, vxe_render::OccupancyFilter::Majority);
    println!("{BOX_HEADER}");
    for feature in [Feature::Rod, Feature::LoneCube, Feature::WallPillar] {
        let r = feature_reading(&device, &queue, feature, majority);
        println!("{}", r.row(feature.name(), "majority"));
        assert!(r.crack.shares.hits > 0, "{}", feature.name());
        assert_eq!(
            r.box_px,
            0,
            "{}: box pixels under the filter",
            feature.name()
        );
    }
    let table = flythrough::LookPreset::DEFAULT.materials();
    for (name, world, camera) in [
        (
            "bare floor",
            Feature::Floor.world(&table),
            Feature::Floor.camera(),
        ),
        ("floor", floor_world(&table), floor_camera()),
        ("wall", wall_world(&table), wall_camera()),
    ] {
        let plain = mask_and_normals(&device, &queue, &world, &camera, start);
        let filtered = mask_and_normals(&device, &queue, &world, &camera, majority);
        let window = analytic_window(&world);
        let (mut kept, mut left_out) = (0, 0);
        for i in 0..plain.2.len() {
            if !kept_by_window(plain.2[i], window) || !kept_by_window(filtered.2[i], window) {
                left_out += 1;
                continue;
            }
            kept += usize::from(plain.0[i * 4] > 127);
            let px = i * 4..i * 4 + 4;
            assert_eq!(
                plain.0[px.clone()],
                filtered.0[px.clone()],
                "{name}: the hit mask moved under the filter at pixel {i}"
            );
            assert_eq!(
                plain.1[px.clone()],
                filtered.1[px],
                "{name}: the normals view moved under the filter at pixel {i}"
            );
        }
        println!("{name}: {kept} hits the same with and without the filter, {left_out} left out");
        assert!(kept > 0, "{name}: hits");
    }
    println!("{CRACK_HEADER}");
    for (name, world, camera) in [
        (
            "tilted slab",
            tilted_slab_world(&table),
            tilted_slab_camera(),
        ),
        (
            "45-degree slab",
            axis_slab_world(&table),
            axis_slab_camera(),
        ),
    ] {
        let mut rig = fit_rig_on(
            device.clone(),
            queue.clone(),
            &world,
            majority.tuning.fit,
            [WIDTH, HEIGHT],
        );
        let r = crack_reading(
            &mut rig,
            &camera,
            &LightParams::headlamp(),
            LookParams {
                view: ViewMode::Cracks,
                ..majority
            },
            Some(analytic_window(&world)),
        );
        println!("{}", r.row(name, "majority"));
        assert!(r.shares.hits > 0, "{name}");
        assert_eq!(r.shares.classes[1], 0, "{name}: {:?}", r.shares);
        assert_eq!(r.shares.classes[2], 0, "{name}: {:?}", r.shares);
    }
}

/// The guardrails of the "not a Lego look" bar in `look`, detail off: the cavity's normal
/// error (degrees) and axis fraction `A`, and the pillar's silhouette error (pixels), each
/// scene mirrored with contour attributes under the look's fit.
fn look_guardrails(
    device: &vxe_render::wgpu::Device,
    queue: &vxe_render::wgpu::Queue,
    look: LookParams,
) -> Guardrails {
    let cavity = cavity_world();
    let cam = cavity_camera();
    let (_, normals, _) = mask_and_normals(device, queue, &cavity, &cam, look);
    let pillar = pillar_world();
    let pcam = pillar_camera();
    let (mask, _, _) = mask_and_normals(device, queue, &pillar, &pcam, look);
    Guardrails {
        normal_error: cavity_normal_error(&normals, &cam),
        silhouette: pillar_silhouette_error(&mask, &pcam),
        axis_fraction: axis_fraction(&normals),
    }
}

/// Face mode's pillar silhouette error, the silhouette guardrail's reference.
fn face_silhouette_on(device: &vxe_render::wgpu::Device, queue: &vxe_render::wgpu::Queue) -> usize {
    let pillar = pillar_world();
    let mut rig = LookRig::on(device.clone(), queue.clone(), &pillar, None);
    let cam = pillar_camera();
    let mask = rig.render(
        &cam,
        &LightParams::at_eye(),
        look(NormalMode::Face, ViewMode::HitMask),
    );
    pillar_silhouette_error(&mask, &cam)
}

/// The start look with the majority filter meets Plan 0039's guardrails: cavity normal error
/// at most 12 degrees, pillar silhouette error at most half of face mode's, `A` at most 10 %.
/// The start look's own are printed beside them.
#[test]
fn the_majority_fit_meets_the_guardrails() {
    let Some((device, queue)) = software_device(attr_pool_bytes(&cavity_world()).max(64 << 20))
    else {
        return;
    };
    let start = start_look(ViewMode::Shaded);
    let face = face_silhouette_on(&device, &queue);
    for (name, look) in [
        ("start look", start),
        (
            "majority",
            with_filter(start, vxe_render::OccupancyFilter::Majority),
        ),
    ] {
        let g = look_guardrails(&device, &queue, look);
        println!(
            "{name}: cavity normal error {:.2} degrees, silhouette {} px against face {face}, axis fraction {:.2} %",
            g.normal_error,
            g.silhouette,
            g.axis_fraction * 100.0
        );
        if name == "majority" {
            assert!(g.normal_error <= 12.0, "{g:?}");
            assert!(2 * g.silhouette <= face, "{g:?} against face {face}");
            assert!(g.axis_fraction <= 0.10, "{g:?}");
        }
    }
}

// ---- The normal field and the cubes-shaded-by-the-field preset (ADR-0040) ----

/// `look` shading with the planes' normal field.
fn field_on(look: LookParams) -> LookParams {
    let mut look = look;
    look.tuning.normal_field = true;
    look
}

/// Preset `p`'s look, detail on, in `view`.
fn preset_look(p: flythrough::LookPreset, view: ViewMode) -> LookParams {
    LookParams {
        normal_mode: p.normal_mode,
        detail: true,
        view,
        tuning: p.tuning,
        ..LookParams::default()
    }
}

/// The normal field leaves no tile edge: on lavapipe under the analytic window rule the rod,
/// the lone cube, the groove and the ridge read no adjacent same-surface pair whose normals
/// differ by more than 15 degrees with the field on, the plane fit's filter off and on.
#[test]
fn the_normal_field_has_no_tile_edges() {
    let Some((device, queue)) = software_device(64 << 20) else {
        return;
    };
    let start = start_look(ViewMode::Shaded);
    let majority = with_filter(start, vxe_render::OccupancyFilter::Majority);
    println!("{BOX_HEADER}");
    for (name, look) in [
        ("field", field_on(start)),
        ("majority + field", field_on(majority)),
    ] {
        for feature in [
            Feature::Rod,
            Feature::LoneCube,
            Feature::Groove,
            Feature::Ridge,
        ] {
            let r = feature_reading(&device, &queue, feature, look);
            println!("{}", r.row(feature.name(), name));
            assert!(r.pairs > 0, "{}: pairs", feature.name());
            assert_eq!(r.edges, 0, "{} under {name}: tile edges", feature.name());
        }
    }
}

/// The normal field moves no hit: every one-voxel scene's hit mask is byte for byte the same
/// with the field on and off, the plane fit's filter off and on, on lavapipe.
#[test]
fn the_normal_field_moves_no_hit() {
    let Some((device, queue)) = software_device(64 << 20) else {
        return;
    };
    let start = start_look(ViewMode::Shaded);
    let majority = with_filter(start, vxe_render::OccupancyFilter::Majority);
    let table = flythrough::LookPreset::DEFAULT.materials();
    for base in [start, majority] {
        for feature in Feature::ALL {
            let world = feature.world(&table);
            let camera = feature.camera();
            let off = mask_and_normals(&device, &queue, &world, &camera, base);
            let on = mask_and_normals(&device, &queue, &world, &camera, field_on(base));
            assert!(
                off.0.chunks(4).any(|p| p[0] > 127),
                "{}: hits",
                feature.name()
            );
            assert!(
                off.0 == on.0,
                "{}: the hit mask moved under the field",
                feature.name()
            );
        }
    }
}

/// Dark specks of the contour golden scene in `look` that appear only with the headlamp's
/// shadow ray, with the full shadow walk: `(with, without, only with)`.
fn specks_in(rig: &mut LookRig, look: LookParams) -> (usize, usize, usize) {
    rig.frame.set_lighting(vxe_render::Lighting {
        shadow_walk: vxe_render::ShadowWalk::Full,
        ..rig.frame.lighting()
    });
    let off_light = LightParams {
        shadows: false,
        ..LightParams::headlamp()
    };
    let on = dark_specks(&rig.render(&scene_camera(), &LightParams::headlamp(), look));
    let off = dark_specks(&rig.render(&scene_camera(), &off_light, look));
    let only = on.iter().filter(|i| off.binary_search(i).is_err()).count();
    (on.len(), off.len(), only)
}

/// The normal field meets Plan 0039's guardrails: in the start look with the field (the plane
/// fit's filter off and on), the cavity's normal error is at most the start look's, `A` at
/// most 10 % and the pillar's silhouette the start look's. `cubes-field` reads face mode's
/// silhouette and a cavity normal error of at most 12 degrees.
#[test]
fn the_normal_field_meets_the_guardrails() {
    let Some((device, queue)) = software_device(attr_pool_bytes(&cavity_world()).max(64 << 20))
    else {
        return;
    };
    let start = start_look(ViewMode::Shaded);
    let face = face_silhouette_on(&device, &queue);
    let today = look_guardrails(&device, &queue, start);
    let mut rows = Vec::new();
    for (name, look) in [
        ("start look", start),
        ("field", field_on(start)),
        (
            "majority + field",
            field_on(with_filter(start, vxe_render::OccupancyFilter::Majority)),
        ),
        (
            "cubes-field",
            preset_look(flythrough::LookPreset::CUBES_FIELD, ViewMode::Shaded),
        ),
    ] {
        let g = look_guardrails(&device, &queue, look);
        println!(
            "{name}: cavity normal error {:.2} degrees, silhouette {} px against face {face}, axis fraction {:.2} %",
            g.normal_error,
            g.silhouette,
            g.axis_fraction * 100.0
        );
        rows.push((name, g));
    }
    for (name, g) in &rows[1..3] {
        assert!(
            g.normal_error <= today.normal_error,
            "{name}: {g:?} against {today:?}"
        );
        assert!(g.axis_fraction <= 0.10, "{name}: {g:?}");
        assert_eq!(g.silhouette, today.silhouette, "{name}: {g:?}");
    }
    let cubes = rows[3].1;
    assert_eq!(cubes.silhouette, face, "cubes-field: {cubes:?}");
    assert!(cubes.normal_error <= 12.0, "cubes-field: {cubes:?}");
}

/// The contour golden scene's dark specks that appear only with the headlamp's shadow ray (the
/// speck test's reading, full shadow walk) are with the normal field at most without it, on
/// lavapipe.
#[test]
fn the_normal_field_adds_no_dark_speck() {
    let world = carved_world();
    let Some((device, queue)) = software_device(attr_pool_bytes(&world).max(64 << 20)) else {
        return;
    };
    let mut rig = attr_rig_on(device, queue, &world, None);
    let contour = look(NormalMode::Contour, ViewMode::Shaded);
    let plain = specks_in(&mut rig, contour);
    let field = specks_in(&mut rig, field_on(contour));
    println!(
        "contour golden scene specks (with, without, only with): {plain:?}, with the field {field:?}"
    );
    assert!(field.2 <= plain.2, "{field:?} against {plain:?}");
}

// ---- The field surface (ADR-0040 Alternative B, Plan 0054's scratch) ----

use support::rigs::with_field_pool;

/// The field look (`LookPreset::FIELD`, detail on) in `view` under `normal`.
fn field_look(view: ViewMode, normal: vxe_render::FieldNormal) -> LookParams {
    let mut look = preset_look(flythrough::LookPreset::FIELD, view);
    look.tuning.field_normal = normal;
    look
}

/// A crack reading of `world` from `camera` in `look` on its own fit rig at the look rigs'
/// size under the analytic window rule, the field's pool built when `look` walks the field,
/// with the hit positions of its last frame.
fn field_crack_reading(
    device: &vxe_render::wgpu::Device,
    queue: &vxe_render::wgpu::Queue,
    world: &World,
    camera: &vxe_render::Camera,
    look: LookParams,
) -> (CrackReading, Vec<[f32; 4]>) {
    let mut rig = fit_rig_on(
        device.clone(),
        queue.clone(),
        world,
        look.tuning.fit,
        [WIDTH, HEIGHT],
    );
    with_field_pool(&mut rig, world, &look);
    let r = crack_reading(
        &mut rig,
        camera,
        &LightParams::headlamp(),
        look,
        Some(analytic_window(world)),
    );
    let positions = readback::read_rgba32f(&rig.device, &rig.queue, rig.frame.hit_positions());
    (r, positions)
}

/// The field surface lies on the flat faces: on lavapipe under the analytic window rule every
/// hit on `floor` (rock at `z < 64`) lies within 1e-3 voxel of `z = 64` and every hit on `wall`
/// (rock at `x >= 80`) within 1e-3 of `x = 80`, and neither reads a leak (the cracks view's
/// class 1 in the field look). The hit counts print beside the start look's.
#[test]
fn the_field_surface_lies_on_the_flat_faces() {
    let Some((device, queue)) = software_device(64 << 20) else {
        return;
    };
    let table = flythrough::LookPreset::DEFAULT.materials();
    for (name, world, camera, axis, plane) in [
        (
            "floor",
            floor_world(&table),
            floor_camera(),
            2usize,
            64.0f32,
        ),
        ("wall", wall_world(&table), wall_camera(), 0, 80.0),
    ] {
        let window = analytic_window(&world);
        let (plane_look, _) = field_crack_reading(
            &device,
            &queue,
            &world,
            &camera,
            start_look(ViewMode::Shaded),
        );
        let (field, positions) = field_crack_reading(
            &device,
            &queue,
            &world,
            &camera,
            field_look(ViewMode::Shaded, vxe_render::FieldNormal::Analytic),
        );
        let mut worst = 0f32;
        for (i, p) in positions.iter().enumerate() {
            if field.classes[i].is_none() || !kept_by_window(*p, window) {
                continue;
            }
            worst = worst.max((p[axis] - plane).abs());
        }
        println!(
            "{name}: field {} hits ({} left out), {} leaks, worst {worst:.2e} voxel off the face; start look {} hits ({} left out)",
            field.shares.hits,
            field.excluded,
            field.shares.classes[1],
            plane_look.shares.hits,
            plane_look.excluded
        );
        assert!(field.shares.hits > 0, "{name}");
        assert!(worst <= 1e-3, "{name}: a hit {worst} voxel off the face");
        assert_eq!(field.shares.classes[1], 0, "{name}: leaks");
    }
}

/// The field readings' two normal sources: N1, the field's gradient, and N2, the corners'
/// blend.
fn field_looks() -> [(&'static str, LookParams); 2] {
    [
        (
            "field N1",
            field_look(ViewMode::Shaded, vxe_render::FieldNormal::Analytic),
        ),
        (
            "field N2",
            field_look(ViewMode::Shaded, vxe_render::FieldNormal::Blend),
        ),
    ]
}

/// The analytic scenes of the field readings: the one-voxel scenes (the box metric's), the
/// tilted, 45-degree and diagonal slabs, the cavity and the pillar, each with its camera.
fn field_scenes() -> Vec<(String, World, vxe_render::Camera)> {
    let table = flythrough::LookPreset::DEFAULT.materials();
    let mut scenes: Vec<(String, World, vxe_render::Camera)> = Feature::ALL
        .into_iter()
        .map(|f| (f.name().to_owned(), f.world(&table), f.camera()))
        .collect();
    scenes.push((
        "tilted slab".into(),
        tilted_slab_world(&table),
        tilted_slab_camera(),
    ));
    scenes.push((
        "45-degree slab".into(),
        axis_slab_world(&table),
        axis_slab_camera(),
    ));
    scenes.push((
        "diagonal slab".into(),
        diagonal_slab_world(&table),
        diagonal_slab_camera(),
    ));
    scenes.push(("cavity".into(), cavity_world(), cavity_camera()));
    scenes.push(("pillar".into(), pillar_world(), pillar_camera()));
    scenes
}

/// The box reading of `world` from `camera` in `look` at the look rigs' size under the
/// analytic window rule, lit by the headlamp, the field's pool built when `look` walks it.
fn scene_box_reading(
    device: &vxe_render::wgpu::Device,
    queue: &vxe_render::wgpu::Queue,
    world: &World,
    camera: &vxe_render::Camera,
    look: LookParams,
) -> BoxReading {
    let mut rig = fit_rig_on(
        device.clone(),
        queue.clone(),
        world,
        look.tuning.fit,
        [WIDTH, HEIGHT],
    );
    with_field_pool(&mut rig, world, &look);
    box_reading(
        &mut rig,
        world,
        camera,
        &LightParams::headlamp(),
        look,
        Some(analytic_window(world)),
    )
}

/// A row of the field table: the box row with the leaks (the cracks view's class 1 in the
/// field look) in place of the crack classes.
fn field_row(r: &BoxReading, scene: &str, look: &str) -> String {
    let s = &r.crack.shares;
    format!(
        "| {scene} | {look} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {:.3} | {} |",
        s.hits,
        r.crack.excluded,
        r.box_px,
        r.by_count[0],
        r.by_count[1],
        r.by_air[0],
        r.by_air[1],
        r.by_air[2],
        r.pairs,
        100.0 * r.edges as f64 / r.pairs.max(1) as f64,
        if look.starts_with("field") {
            s.classes[1].to_string()
        } else {
            "-".to_owned()
        },
    )
}

const FIELD_HEADER: &str = "| scene | look | hits | left out | box px | count < 14 | count >= 14 | 2 air | 3 air | 4+ air | pairs | tile edges % | leaks |\n|---|---|---|---|---|---|---|---|---|---|---|---|---|";

/// The field surface has no box and no leak: on lavapipe under the analytic window rule the
/// rod, the lone cube, the groove, the wall pillar and the ridge read 0 box pixels in the field
/// look, every analytic scene ([`field_scenes`]) and the level at `VEIN_POSE` and `PATCH_POSE`
/// read 0 leaks, under N1 and N2; the tile-contrast share under each is printed for every
/// scene. A box pixel in the field look also needs its shading normal within
/// [`FIELD_BOX_DEGREES`] of the face's axis normal (the field surface crosses face planes
/// without showing a cube face there).
#[test]
fn the_field_surface_has_no_boxes_and_no_leaks() {
    let level = pose_level();
    let Some((device, queue)) = software_device(attr_pool_bytes(&level).max(256 << 20)) else {
        return;
    };
    println!("{FIELD_HEADER}");
    let boxed = ["rod", "lone cube", "groove", "wall pillar", "ridge"];
    let mut failures = Vec::new();
    for (scene, world, camera) in field_scenes() {
        for (name, look) in field_looks() {
            let r = scene_box_reading(&device, &queue, &world, &camera, look);
            println!("{}", field_row(&r, &scene, name));
            if r.crack.shares.hits == 0 {
                failures.push(format!("{scene} {name}: no hit"));
            }
            if r.crack.shares.classes[1] != 0 {
                failures.push(format!(
                    "{scene} {name}: {} leaks",
                    r.crack.shares.classes[1]
                ));
            }
            if boxed.contains(&scene.as_str()) && r.box_px != 0 {
                failures.push(format!("{scene} {name}: {} box px", r.box_px));
            }
        }
    }
    for pose in flythrough::POSES {
        for (name, look) in field_looks() {
            let r = pose_box_reading(&device, &queue, &level, &pose, look, true);
            println!("{}", field_row(&r, &format!("level {}", pose.name), name));
            if r.crack.shares.classes[1] != 0 {
                failures.push(format!(
                    "{} {name}: {} leaks",
                    pose.name, r.crack.shares.classes[1]
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
}

/// Where the field side-by-sides go: the worktree's untracked `live/field/`.
fn field_dir() -> std::path::PathBuf {
    let d = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../live/field");
    std::fs::create_dir_all(&d).expect("live/field is writable");
    d
}

/// Writes `name.png`: the plane look's shaded image and normals view, then each field
/// reading's, side by side.
fn write_field_side_by_side(plane: &BoxReading, fields: &[&BoxReading], name: &str) {
    let [w, h] = plane.crack.size.map(|v| v as usize);
    let mut panels: Vec<&[u8]> = vec![&plane.crack.shaded, &plane.normals];
    for f in fields {
        panels.push(&f.crack.shaded);
        panels.push(&f.normals);
    }
    let n = panels.len();
    let mut out = vec![0u8; n * w * h * 4];
    for y in 0..h {
        for (k, src) in panels.iter().enumerate() {
            for x in 0..w {
                let i = y * w + x;
                let o = (y * n * w + k * w + x) * 4;
                out[o..o + 3].copy_from_slice(&src[i * 4..i * 4 + 3]);
                out[o + 3] = 255;
            }
        }
    }
    let path = field_dir().join(format!("{name}.png"));
    write_png(&path, &out, (n * w) as u32, h as u32);
    println!("wrote {}", path.display());
}

/// The field readings (reading only; it asserts nothing): the field table of every analytic
/// scene and both poses (lamp on and off) under the start look, N1 and N2; Plan 0039's
/// guardrails and the speck test under each; the side-by-sides of the poses and of the scenes
/// under `live/field/` (the start look's shaded and normals view, then N1's and N2's). On the
/// adapter `VXE_TEST_ADAPTER` names (lavapipe unset):
/// `cargo nextest run -p flythrough --test look --run-ignored only --no-capture field_readings`.
#[test]
#[ignore = "a reading: prints the field table and writes the side-by-sides"]
fn field_readings() {
    let level = pose_level();
    let Some((device, queue, adapter)) = reading_device(attr_pool_bytes(&level).max(256 << 20))
    else {
        return;
    };
    println!("adapter {adapter}");
    let start = start_look(ViewMode::Shaded);
    let mut rows = Vec::new();
    let mut emit = |row: String| {
        println!("{row}");
        rows.push(row);
    };
    for (scene, world, camera) in field_scenes() {
        let plane = scene_box_reading(&device, &queue, &world, &camera, start);
        emit(field_row(&plane, &scene, "start look"));
        let mut fields = Vec::new();
        for (name, look) in field_looks() {
            let r = scene_box_reading(&device, &queue, &world, &camera, look);
            emit(field_row(&r, &scene, name));
            fields.push(r);
        }
        let refs: Vec<&BoxReading> = fields.iter().collect();
        write_field_side_by_side(&plane, &refs, &scene.replace(' ', "-"));
    }
    for pose in flythrough::POSES {
        for lamp in [true, false] {
            let lamp_tag = if lamp { "" } else { " lamp off" };
            let scene = format!("level {}{lamp_tag}", pose.name);
            let plane = pose_box_reading(&device, &queue, &level, &pose, start, lamp);
            emit(field_row(&plane, &scene, "start look"));
            let mut fields = Vec::new();
            for (name, look) in field_looks() {
                let r = pose_box_reading(&device, &queue, &level, &pose, look, lamp);
                emit(field_row(&r, &scene, name));
                fields.push(r);
            }
            let refs: Vec<&BoxReading> = fields.iter().collect();
            write_field_side_by_side(
                &plane,
                &refs,
                &format!("{}{lamp_tag}", pose.name).replace(' ', "-"),
            );
        }
    }
    let face = face_silhouette_on(&device, &queue);
    let mut guard = Vec::new();
    let world = carved_world();
    let contour = look(NormalMode::Contour, ViewMode::Shaded);
    for (name, l) in [("start look", start)].into_iter().chain(field_looks()) {
        let g = look_guardrails(&device, &queue, l);
        let mut rig = attr_rig_on(device.clone(), queue.clone(), &world, None);
        let speck_look = LookParams {
            tuning: vxe_render::SurfaceTuning {
                model: l.tuning.model,
                field_normal: l.tuning.field_normal,
                ..contour.tuning
            },
            ..contour
        };
        with_field_pool(&mut rig, &world, &speck_look);
        let specks = specks_in(&mut rig, speck_look);
        guard.push(format!(
            "| {name} | {:.2} | {} | {face} | {:.2} | {} | {} | {} |",
            g.normal_error,
            g.silhouette,
            g.axis_fraction * 100.0,
            specks.0,
            specks.1,
            specks.2
        ));
    }
    println!("\nadapter {adapter}\n{FIELD_HEADER}");
    for r in rows {
        println!("{r}");
    }
    println!(
        "\n| look | cavity normal error (deg) | silhouette px | face px | A % | specks with | without | only with |\n|---|---|---|---|---|---|---|---|"
    );
    for g in guard {
        println!("{g}");
    }
}
