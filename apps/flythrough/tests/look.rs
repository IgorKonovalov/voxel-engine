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
use vxe_render::{CrackClosing, CrackShares, Exposure, SurfaceFit, readback};

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
/// wall (4), on its own contour rig at the start look's fit under ADR-0011's old hit rule
/// (`CrackClosing::Old`), lit by the headlamp, at the look rigs' size, under the analytic
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
        closing_look(ViewMode::Cracks, CrackClosing::Old, false),
        Some(analytic_window(&world)),
    ))
}

/// The tilted slab in the contour look the fly-through starts in, under the old hit rule, has
/// cracked pixels under the analytic window rule: a ray that passes one surface voxel outside its plane and then hits a
/// voxel with no plane (class 1) or enters the next voxel already inside its plane (class 2).
/// The cracks view's colours decode to the same shares as the hit pass's layers.
#[test]
fn a_tilted_slab_has_cracks_today() {
    let Some(r) = slab_reading(0) else {
        return;
    };
    println!("{CRACK_HEADER}");
    println!("{}", r.row("tilted slab", "start look"));
    assert_eq!(
        CrackShares::from_rgba8(&r.cracks),
        r.all,
        "the cracks view's colours decode to the layers' classes"
    );
    let cracked = r.shares.classes[1] + r.shares.classes[2];
    assert!(cracked >= 1, "{:?}", r.shares);
}

/// A flat floor (rock at `z < 64`) and a flat vertical wall (rock at `x >= 80`), each seen
/// obliquely in the start look under the old hit rule, have no crack under the analytic window rule: their surface
/// voxels share one neighbourhood and the plane code holds the axis normal exactly
/// (ADR-0038), so they share one plane and no ray reads class 1 or 2. It pins that the
/// classification does not count a clean surface.
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

/// The look of a crack reading at `fit`: the start look with that plane fit, under the old hit
/// rule.
fn look_at_fit(fit: SurfaceFit) -> LookParams {
    let mut look = closing_look(ViewMode::Cracks, CrackClosing::Old, false);
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

/// The crack class table for every scene, pose and swept part under ADR-0011's old hit rule
/// (reading only; it asserts nothing): the tilted slab, the concave corner, the 45-degree slab,
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
    assert_eq!(look.tuning.closing, CrackClosing::Snap);
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

// ---- The crack-closing candidates ----

/// The start look in `view` with crack-closing rule `closing` and shadow push `push`.
fn closing_look(view: ViewMode, closing: CrackClosing, push: bool) -> LookParams {
    let mut look = start_look(view);
    look.tuning.closing = closing;
    look.tuning.shadow_push = push;
    look
}

/// Plan 0039's guardrails of the start look under `closing`, as
/// `sharpness::the_default_look_meets_the_guardrails` reads them: the cavity's normal error
/// (degrees) and axis fraction `A` at full lighting resolution, and the pillar's silhouette
/// error (pixels), detail off.
fn closing_guardrails(
    device: &vxe_render::wgpu::Device,
    queue: &vxe_render::wgpu::Queue,
    closing: CrackClosing,
) -> (f64, f64, usize) {
    let fit = flythrough::LookPreset::DEFAULT.tuning.fit;
    let look = |view| LookParams {
        detail: false,
        ..closing_look(view, closing, false)
    };
    let cavity = cavity_world();
    let mut rig = fit_rig_on(device.clone(), queue.clone(), &cavity, fit, [WIDTH, HEIGHT]);
    let cam = cavity_camera();
    let normals = rig.render(&cam, &LightParams::at_eye(), look(ViewMode::Normals));
    let error = cavity_normal_error(&normals, &cam);
    let axis = axis_fraction(&normals);
    let pillar = pillar_world();
    let mut rig = fit_rig_on(device.clone(), queue.clone(), &pillar, fit, [WIDTH, HEIGHT]);
    let cam = pillar_camera();
    let mask = rig.render(&cam, &LightParams::at_eye(), look(ViewMode::HitMask));
    (error, axis, pillar_silhouette_error(&mask, &cam))
}

/// The speck test's reading of the contour look (the golden scene's tuning) under `closing`
/// and shadow push `push`: dark specks with the headlamp's shadows, without, and only with,
/// on the full shadow walk, as `contour_dark_specks_with_and_without_shadows` reads them.
fn closing_specks(
    device: &vxe_render::wgpu::Device,
    queue: &vxe_render::wgpu::Queue,
    closing: CrackClosing,
    push: bool,
) -> (usize, usize, usize) {
    let world = carved_world();
    let mut rig = attr_rig_on(device.clone(), queue.clone(), &world, None);
    rig.frame.set_lighting(vxe_render::Lighting {
        shadow_walk: vxe_render::ShadowWalk::Full,
        ..rig.frame.lighting()
    });
    let mut l = look(NormalMode::Contour, ViewMode::Shaded);
    l.tuning.closing = closing;
    l.tuning.shadow_push = push;
    let off = LightParams {
        shadows: false,
        ..LightParams::headlamp()
    };
    let on = dark_specks(&rig.render(&scene_camera(), &LightParams::headlamp(), l));
    let off = dark_specks(&rig.render(&scene_camera(), &off, l));
    let only = on.iter().filter(|i| off.binary_search(i).is_err()).count();
    (on.len(), off.len(), only)
}

/// The crack-closing candidates' readings (reading only; it asserts nothing): per rule, the
/// crack class table of `crack_classes`' scenes and poses at the start look, Plan 0039's
/// guardrails and the speck test's reading; and per rule with the shadow push, the poses'
/// self-hit shares and the speck reading. On the adapter `VXE_TEST_ADAPTER` names (lavapipe
/// unset): `cargo nextest run -p flythrough --test look --run-ignored only --no-capture
/// closing_candidates`.
#[test]
#[ignore = "a reading: prints the closing candidates' tables"]
fn closing_candidates() {
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
    let fit = flythrough::LookPreset::DEFAULT.tuning.fit;
    let face = {
        let pillar = pillar_world();
        let mut rig = LookRig::on(device.clone(), queue.clone(), &pillar, None);
        let cam = pillar_camera();
        let look = LookParams {
            view: ViewMode::HitMask,
            ..LookParams::default()
        };
        pillar_silhouette_error(&rig.render(&cam, &LightParams::at_eye(), look), &cam)
    };
    let mut rows = Vec::new();
    let mut extra = Vec::new();
    for closing in CrackClosing::ALL {
        let name = closing.name();
        for (scene, world, camera) in &analytic {
            let mut rig = fit_rig_on(device.clone(), queue.clone(), world, fit, [WIDTH, HEIGHT]);
            let r = crack_reading(
                &mut rig,
                camera,
                &LightParams::headlamp(),
                closing_look(ViewMode::Cracks, closing, false),
                Some(analytic_window(world)),
            );
            println!("{}", r.row(scene, name));
            rows.push(r.row(scene, name));
        }
        for push in [false, true] {
            let look_name = if push {
                format!("{name} + push")
            } else {
                name.to_owned()
            };
            for pose in flythrough::POSES {
                for lamp in [true, false] {
                    if push && !lamp {
                        continue;
                    }
                    let mut rig = pose_rig_on(device.clone(), queue.clone(), &level, &pose, fit);
                    rig.frame.set_lighting(vxe_render::Lighting {
                        exposure: Exposure::Manual(pose.exposure),
                        indirect: false,
                        lamp,
                        ..rig.frame.lighting()
                    });
                    let r = crack_reading(
                        &mut rig,
                        &pose.camera(),
                        &LightParams::headlamp(),
                        closing_look(ViewMode::Cracks, closing, push),
                        None,
                    );
                    let lamp_tag = if lamp { "" } else { " lamp off" };
                    let scene = format!("level {}{lamp_tag}", pose.name);
                    println!("{}", r.row(&scene, &look_name));
                    rows.push(r.row(&scene, &look_name));
                    let tag = format!("{look_name}{lamp_tag}").replace([' ', '+'], "-");
                    write_side_by_side(&r, &format!("{}-closing-{tag}", pose.name));
                }
            }
            let (on, off, only) = closing_specks(&device, &queue, closing, push);
            extra.push(format!(
                "{look_name}: specks with shadows {on}, without {off}, only with {only}"
            ));
        }
        let (error, axis, silhouette) = closing_guardrails(&device, &queue, closing);
        extra.push(format!(
            "{name}: cavity normal error {error:.2} degrees, silhouette {silhouette} px against face {face}, axis fraction {:.2} %",
            axis * 100.0
        ));
        println!("{}", extra[extra.len() - 1]);
    }
    println!("\nadapter {adapter}\n{CRACK_HEADER}");
    for r in rows {
        println!("{r}");
    }
    for e in extra {
        println!("{e}");
    }
}
