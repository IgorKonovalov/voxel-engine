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
/// without the field's band slots.
fn metrics(mode: NormalMode) -> Option<(f64, usize)> {
    metrics_with(mode, false)
}

/// [`metrics`], over mirrors with the field's band slots when `attrs` is set.
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
        field_mirror_rig(&cavity)?
    } else {
        LookRig::new(&cavity, None)?
    };
    let cam = cavity_camera();
    let normals = rig.render(&cam, &LightParams::at_eye(), look(ViewMode::Normals));
    let normal_error = cavity_normal_error(&normals, &cam);
    let axis = axis_fraction(&normals);

    let pillar = pillar_world_with(pillar_table);
    let mut rig = if attrs {
        field_mirror_rig_on(rig.device, rig.queue, &pillar, None)
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

/// The band slots change no pixel in face mode: `primary.png`, `headlamp.png` and `lod.png`
/// render byte for byte over a mirror with the field's band slots, over one shard and over two
/// to four.
#[test]
fn band_slots_leave_the_face_goldens_byte_for_byte() {
    if std::env::var("VXE_BLESS").as_deref() == Ok("1") {
        return;
    }
    let face = LookParams::default();
    let carved = carved_world();
    let lod = lod_scene();
    let Some((device, queue)) = software_device(field_pool_bytes(&carved).max(64 << 20)) else {
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
            let mut rig = field_mirror_rig_on(device.clone(), queue.clone(), world, cap);
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
    let Some(mut rig) = field_mirror_rig(&world) else {
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
    let Some(mut rig) = field_mirror_rig(&world) else {
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

// ---- Leaks: the cracks view of the field look ----

use support::rigs::{
    POSE_SIZE, axis_slab_camera, axis_slab_world, diagonal_slab_camera, diagonal_slab_world,
    field_rig_on, floor_camera, floor_world, pose_level, pose_rig_on, tilted_slab_camera,
    tilted_slab_world, wall_camera, wall_world,
};
use vxe_render::{CrackShares, Exposure, readback};

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

/// A crack reading of one scene in one look: the cracks view's leaks over the counted hits
/// (every hit, or under the analytic window rule those at least [`WINDOW_MARGIN`] inside the
/// window), the hits left out, the shaded image and the per-pixel leaks.
struct CrackReading {
    shares: CrackShares,
    excluded: u64,
    shaded: Vec<u8>,
    /// Per pixel, whether a hit is a leak, `None` on a miss.
    pixels: Vec<Option<bool>>,
    size: [u32; 2],
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
    run(ViewMode::Cracks);
    let layers = readback::read_rgba16f(&rig.device, &rig.queue, rig.frame.layers());
    let records = readback::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
    let positions = readback::read_rgba32f(&rig.device, &rig.queue, rig.frame.hit_positions());
    let pixels: Vec<Option<bool>> = layers
        .iter()
        .zip(&records)
        .map(|(l, r)| (r[3] & 1 == 1).then_some(l[0] > 0.5))
        .collect();
    let counted = |i: usize| {
        window.is_none_or(|[lo, hi]| {
            (0..3).all(|k| {
                let c = positions[i][k];
                c >= lo[k] + WINDOW_MARGIN && c <= hi[k] - WINDOW_MARGIN
            })
        })
    };
    let (mut shares, mut excluded) = (CrackShares::default(), 0);
    for (i, px) in pixels.iter().enumerate() {
        let Some(leak) = *px else {
            continue;
        };
        if !counted(i) {
            excluded += 1;
            continue;
        }
        shares.hits += 1;
        shares.leaks += u64::from(leak);
    }
    CrackReading {
        shares,
        excluded,
        shaded,
        pixels,
        size,
    }
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
    box_px: u64,
    by_count: [u64; 2],
    by_air: [u64; 3],
    pairs: u64,
    edges: u64,
}

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
    let field = look.normal_mode == NormalMode::Contour;
    let cos_box = FIELD_BOX_DEGREES.to_radians().cos();
    let [w, h] = crack.size.map(|v| v as usize);
    let counted: Vec<bool> = (0..w * h)
        .map(|i| {
            crack.pixels[i].is_some()
                && window.is_none_or(|[lo, hi]| {
                    (0..3).all(|k| {
                        let c = positions[i][k];
                        c >= lo[k] + WINDOW_MARGIN && c <= hi[k] - WINDOW_MARGIN
                    })
                })
        })
        .collect();
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
        box_px,
        by_count,
        by_air,
        pairs,
        edges,
    }
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
    let mut rig = pose_rig_on(device.clone(), queue.clone(), level, pose);
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

/// Whether the hit at `position` (a miss reads `t = BIG`) is kept by the analytic window rule
/// in `window`: a miss, or a hit at least [`WINDOW_MARGIN`] inside the window on every axis.
fn kept_by_window(position: [f32; 4], [lo, hi]: [[f32; 3]; 2]) -> bool {
    position[3] > 1e20
        || (0..3)
            .all(|k| position[k] >= lo[k] + WINDOW_MARGIN && position[k] <= hi[k] - WINDOW_MARGIN)
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

// ---- The field surface (ADR-0040 Alternative B, Plan 0054's scratch) ----

/// The field look (`LookPreset::FIELD`, detail on) in `view`.
fn field_look(view: ViewMode) -> LookParams {
    preset_look(flythrough::LookPreset::FIELD, view)
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
    let mut rig = field_rig_on(device.clone(), queue.clone(), world, [WIDTH, HEIGHT]);
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
/// (rock at `x >= 80`) within 1e-3 of `x = 80`, and neither reads a leak in the cracks view.
/// The hit counts print beside the start look's.
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
            field_look(ViewMode::Shaded),
        );
        let mut worst = 0f32;
        for (i, p) in positions.iter().enumerate() {
            if field.pixels[i].is_none() || !kept_by_window(*p, window) {
                continue;
            }
            worst = worst.max((p[axis] - plane).abs());
        }
        println!(
            "{name}: field {} hits ({} left out), {} leaks, worst {worst:.2e} voxel off the face; start look {} hits ({} left out)",
            field.shares.hits,
            field.excluded,
            field.shares.leaks,
            plane_look.shares.hits,
            plane_look.excluded
        );
        assert!(field.shares.hits > 0, "{name}");
        assert!(worst <= 1e-3, "{name}: a hit {worst} voxel off the face");
        assert_eq!(field.shares.leaks, 0, "{name}: leaks");
    }
}

/// A carve's band slot (ADR-0041): in `floor` (rock below `z = 64`) with the field look's band
/// slots on, a sphere carve of radius 7 voxels about the centre of brick (4, 4, 4), deep in
/// uniform rock, leaves that brick uniform air with rock diagonally beyond each of its corners.
/// The one sync that writes the edit gives it a band slot (a `CLASS_UNIFORM` entry where a
/// uniform rock entry was), and the field look from the hollow's centre reads every pixel a
/// hit and 0 leaks on lavapipe.
#[test]
fn a_carve_gives_the_exposed_air_brick_a_band_slot() {
    use vxe_render::gpu_world::{CLASS_MASK, CLASS_SHIFT, CLASS_UNIFORM, FULL_BIT};
    use vxe_world::{Brick, ChunkCoord, EditOp, FIXED_ONE, Fixed3, Mode, brick_slot};
    let Some((device, queue)) = software_device(64 << 20) else {
        return;
    };
    let table = flythrough::LookPreset::DEFAULT.materials();
    let world = floor_world(&table);
    let look = field_look(ViewMode::Shaded);
    let mut rig = field_rig_on(device, queue, &world, [WIDTH, HEIGHT]);
    let (c, slot) = (ChunkCoord([0, 0, 0]), brick_slot([4, 4, 4]));
    let before = rig.gpu.entry(c, slot).unwrap();
    assert_eq!(before & FULL_BIT, 0, "uniform rock before the carve");
    assert_ne!(before & 0xFFFF, 0, "uniform rock before the carve");
    world
        .apply(&EditOp::sphere(
            Fixed3([36, 36, 36].map(|v| v * FIXED_ONE)),
            7 * FIXED_ONE,
            Mode::Carve,
        ))
        .unwrap();
    {
        let chunk = world.store().get(c).unwrap();
        let chunk = chunk.read().unwrap();
        assert!(matches!(chunk.brick(slot), Brick::Uniform(vxe_world::AIR)));
    }
    rig.gpu
        .sync_now(&world, &rig.device, &rig.queue)
        .expect("the edit's sync");
    let after = rig.gpu.entry(c, slot).unwrap();
    assert_ne!(after & FULL_BIT, 0);
    assert_eq!(after >> CLASS_SHIFT & CLASS_MASK, CLASS_UNIFORM);
    support::rigs::drain_field(&rig.device, &rig.queue, &mut rig.gpu);
    let camera = vxe_render::Camera {
        position: [36.0, 36.0, 36.0].into(),
        yaw: 0.3,
        pitch: -0.2,
        vfov: 70f32.to_radians(),
    };
    let r = crack_reading(&mut rig, &camera, &LightParams::headlamp(), look, None);
    let [w, h] = r.size;
    println!(
        "carve: {} hits of {} pixels, {} leaks",
        r.shares.hits,
        w * h,
        r.shares.leaks
    );
    assert_eq!(r.shares.hits, u64::from(w * h));
    assert_eq!(r.shares.leaks, 0, "leaks");
}

/// The field readings' look: the field, shaded with its gradient (N1).
fn field_looks() -> [(&'static str, LookParams); 1] {
    [("field N1", field_look(ViewMode::Shaded))]
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
    let mut rig = field_rig_on(device.clone(), queue.clone(), world, [WIDTH, HEIGHT]);
    box_reading(
        &mut rig,
        world,
        camera,
        &LightParams::headlamp(),
        look,
        Some(analytic_window(world)),
    )
}

/// A row of the field table: the box row with the leaks in the cracks view.
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
            s.leaks.to_string()
        } else {
            "-".to_owned()
        },
    )
}

const FIELD_HEADER: &str = "| scene | look | hits | left out | box px | count < 14 | count >= 14 | 2 air | 3 air | 4+ air | pairs | tile edges % | leaks |\n|---|---|---|---|---|---|---|---|---|---|---|---|---|";

/// The field surface has no box and no leak: on lavapipe under the analytic window rule the
/// rod, the lone cube, the groove, the wall pillar and the ridge read 0 box pixels in the field
/// look, every analytic scene ([`field_scenes`]) and the level at `VEIN_POSE` and `PATCH_POSE`
/// read 0 leaks under N1; the tile-contrast share is printed for every scene. A box pixel in
/// the field look also needs its shading normal within [`FIELD_BOX_DEGREES`] of the face's axis
/// normal (the field surface crosses face planes without showing a cube face there).
#[test]
fn the_field_surface_has_no_boxes_and_no_leaks() {
    let level = pose_level();
    let Some((device, queue)) = software_device(field_pool_bytes(&level).max(256 << 20)) else {
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
            if r.crack.shares.leaks != 0 {
                failures.push(format!("{scene} {name}: {} leaks", r.crack.shares.leaks));
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
            if r.crack.shares.leaks != 0 {
                failures.push(format!(
                    "{} {name}: {} leaks",
                    pose.name, r.crack.shares.leaks
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
}

// ---- Thin features and the crisp-material look (Plan 0054 Phase 3a) ----

use flythrough::thin_rig::{FEATURES, ThinRig};

/// The thin rig's render size: about five pixels to a voxel at the features from its camera.
const THIN_SIZE: [u32; 2] = [320, 320];
/// Voxels of each feature nearest the trunk that its reading leaves out (the trunk's own
/// surface, and its fillet under the field, stay out of the feature's box).
const THIN_GAP: i32 = 2;
/// How far, in voxels, a hit may lie outside a feature's measured voxel box and still count
/// as the feature's.
const THIN_SLACK: f32 = 0.5;

/// The thin rig ([`flythrough::thin_rig`]) under the start look's table with the rig's wood,
/// its row carrying `flags` (`MaterialFlags::CRISP` as the fly-through has it).
fn thin_rig_world(flags: vxe_world::MaterialFlags) -> World {
    let table = flythrough::with_wood_flags(&flythrough::LookPreset::DEFAULT.materials(), flags);
    let source = ThinRig {
        wood: flythrough::THIN_RIG_WOOD,
    };
    World::generate(source.extent(), &source, &table)
}

/// One look's reading of the thin rig: its crack reading (every hit counted, its leaks), the
/// hit positions and the hit records of its last frame.
struct ThinReading {
    crack: CrackReading,
    positions: Vec<[f32; 4]>,
    records: Vec<[u16; 4]>,
}

fn thin_reading(
    device: &vxe_render::wgpu::Device,
    queue: &vxe_render::wgpu::Queue,
    world: &World,
    look: LookParams,
) -> ThinReading {
    let mut rig = field_rig_on(device.clone(), queue.clone(), world, THIN_SIZE);
    let crack = crack_reading(
        &mut rig,
        &flythrough::thin_rig_camera(),
        &LightParams::headlamp(),
        look,
        None,
    );
    let positions = readback::read_rgba32f(&rig.device, &rig.queue, rig.frame.hit_positions());
    let records = readback::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
    ThinReading {
        crack,
        positions,
        records,
    }
}

impl ThinReading {
    /// The hit pixels whose hit point lies in feature `f`'s measured box
    /// ([`flythrough::thin_rig::Feature::measured`], [`THIN_GAP`]) grown by [`THIN_SLACK`].
    fn feature_px(&self, f: &flythrough::thin_rig::Feature) -> u64 {
        let (lo, hi) = f.measured(THIN_GAP);
        self.crack
            .pixels
            .iter()
            .zip(&self.positions)
            .filter(|(c, p)| {
                c.is_some()
                    && (0..3).all(|k| {
                        p[k] >= lo[k] as f32 - THIN_SLACK && p[k] <= hi[k] as f32 + THIN_SLACK
                    })
            })
            .count() as u64
    }

    /// Whether pixel `i` hit within the crisp voxels' one-voxel neighbourhood: a corner of the
    /// dual cell holding its hit point lies within one voxel (Chebyshev) of a voxel of a feature,
    /// where the crisp wood's cube, corner and material rules may move the hit.
    fn near_crisp(&self, i: usize) -> bool {
        let p = self.positions[i];
        let base = [0, 1, 2].map(|k| (p[k] - 0.5).floor() as i32);
        (-1..3).any(|dz| {
            (-1..3).any(|dy| {
                (-1..3).any(|dx| {
                    let v = [base[0] + dx, base[1] + dy, base[2] + dz];
                    FEATURES.iter().any(|f| f.holds(v))
                })
            })
        })
    }
}

/// Where the thin rig's side-by-sides go: the worktree's untracked `live/thin/`.
fn thin_dir() -> std::path::PathBuf {
    let d = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../live/thin");
    std::fs::create_dir_all(&d).expect("live/thin is writable");
    d
}

/// The field look's hit pixels per feature of the thin rig with smooth wood, in
/// [`FEATURES`]' order (Plan 0054 Phase 3a's `field` column): a feature under about three
/// voxels across renders as nothing.
const THIN_SMOOTH_PX: [u64; 6] = [0, 0, 913, 0, 0, 2666];

/// Thin features in the field and per material (Plan 0054 Phases 3a and 4c), on lavapipe: the
/// thin rig from its camera in `face`, and in `field` with the rig wood's row crisp
/// (`MaterialFlags::CRISP`, no renderer setting) and with the flag cleared. Per feature it
/// prints the hit pixels whose hit point lies in the feature's box (less the two voxels nearest
/// the trunk, grown by half a voxel) in each, and asserts that every feature reads hit pixels in
/// `face`; with the wood crisp every feature reads within 2 % of `face`'s pixels; with it
/// smooth each feature reads [`THIN_SMOOTH_PX`]; both field readings and the level at
/// `VEIN_POSE` and `PATCH_POSE` read 0 leaks; and every pixel whose hit lies, in both field
/// readings, outside the wood's one-voxel neighbourhood (no corner of its dual cell within one
/// voxel of the wood) reads the same hit position and hit record bit for bit in both, a hit in
/// one and a miss in the other counting as a difference. The shaded side-by-side (face, field
/// smooth wood, field crisp wood) goes to `live/thin/rig.png`.
#[test]
fn thin_features_in_the_field_and_per_material() {
    let level = pose_level();
    let Some((device, queue)) = software_device(field_pool_bytes(&level).max(256 << 20)) else {
        return;
    };
    let crisp = thin_rig_world(vxe_world::MaterialFlags::CRISP);
    let smooth = thin_rig_world(vxe_world::MaterialFlags::empty());
    let face_look = preset_look(flythrough::LookPreset::FACE, ViewMode::Shaded);
    let field_look = preset_look(flythrough::LookPreset::FIELD, ViewMode::Shaded);
    let names = ["face", "field smooth wood", "field crisp wood"];
    let readings = [
        thin_reading(&device, &queue, &crisp, face_look),
        thin_reading(&device, &queue, &smooth, field_look),
        thin_reading(&device, &queue, &crisp, field_look),
    ];
    let [face, field, cubes] = [&readings[0], &readings[1], &readings[2]];
    let mut failures = Vec::new();
    println!(
        "| feature | face px | smooth px | smooth / face % | crisp px | crisp / face % |\n|---|---|---|---|---|---|"
    );
    for (f, want) in FEATURES.iter().zip(THIN_SMOOTH_PX) {
        let [a, b, c] = [face, field, cubes].map(|r| r.feature_px(f));
        let pct = |n: u64| 100.0 * n as f64 / a.max(1) as f64;
        println!(
            "| {} | {a} | {b} | {:.1} | {c} | {:.1} |",
            f.name,
            pct(b),
            pct(c)
        );
        if a == 0 {
            failures.push(format!("{}: no face hit in its box", f.name));
        }
        if (c as f64 - a as f64).abs() > 0.02 * a as f64 {
            failures.push(format!("{}: crisp wood {c} px against face {a}", f.name));
        }
        if b != want {
            failures.push(format!("{}: smooth wood {b} px against {want}", f.name));
        }
    }
    for (name, r) in names.iter().zip(&readings) {
        println!(
            "{name}: {} hits, {} leaks",
            r.crack.shares.hits, r.crack.shares.leaks
        );
    }
    for (name, r) in names[1..].iter().zip(&readings[1..]) {
        if r.crack.shares.leaks != 0 {
            failures.push(format!("rig {name}: {} leaks", r.crack.shares.leaks));
        }
    }
    let (mut compared, mut near, mut differ) = (0u64, 0u64, 0u64);
    for i in 0..field.positions.len() {
        let (hf, hc) = (
            field.crack.pixels[i].is_some(),
            cubes.crack.pixels[i].is_some(),
        );
        if (hf && field.near_crisp(i)) || (hc && cubes.near_crisp(i)) {
            near += 1;
            continue;
        }
        if !hf && !hc {
            continue;
        }
        compared += 1;
        let same = hf == hc
            && field.positions[i].map(f32::to_bits) == cubes.positions[i].map(f32::to_bits)
            && field.records[i] == cubes.records[i];
        differ += u64::from(!same);
    }
    println!(
        "smooth against crisp wood away from the wood: {compared} hit pixels compared, {differ} differ, {near} left out near the wood"
    );
    if differ != 0 {
        failures.push(format!("{differ} hit pixels differ away from the wood"));
    }
    let [w, h] = THIN_SIZE.map(|v| v as usize);
    let mut out = vec![0u8; 3 * w * h * 4];
    for y in 0..h {
        for (k, r) in readings.iter().enumerate() {
            for x in 0..w {
                let (i, o) = (y * w + x, (y * 3 * w + k * w + x) * 4);
                out[o..o + 3].copy_from_slice(&r.crack.shaded[i * 4..i * 4 + 3]);
                out[o + 3] = 255;
            }
        }
    }
    let path = thin_dir().join("rig.png");
    write_png(&path, &out, (3 * w) as u32, h as u32);
    println!("wrote {}", path.display());
    for pose in flythrough::POSES {
        let r = pose_box_reading(&device, &queue, &level, &pose, field_look, true);
        println!(
            "level {} field: {} hits, {} leaks",
            pose.name, r.crack.shares.hits, r.crack.shares.leaks
        );
        if r.crack.shares.leaks != 0 {
            failures.push(format!(
                "{} field: {} leaks",
                pose.name, r.crack.shares.leaks
            ));
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
}

/// The thin rig's hit positions and records in the hit-mask view, `look` with or without the
/// split walk (`Frame::set_field_split`).
fn thin_records(
    device: &vxe_render::wgpu::Device,
    queue: &vxe_render::wgpu::Queue,
    world: &World,
    look: LookParams,
    split: bool,
) -> (Vec<[f32; 4]>, Vec<[u16; 4]>) {
    let mut rig = field_rig_on(device.clone(), queue.clone(), world, THIN_SIZE);
    rig.frame.set_field_split(split);
    let look = LookParams {
        view: ViewMode::HitMask,
        ..look
    };
    let _ = rig.render(
        &flythrough::thin_rig_camera(),
        &LightParams::headlamp(),
        look,
    );
    (
        readback::read_rgba32f(&rig.device, &rig.queue, rig.frame.hit_positions()),
        readback::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records()),
    )
}

/// The pixels at which two readings differ in hit position (bits) or hit record.
fn records_differ(a: &(Vec<[f32; 4]>, Vec<[u16; 4]>), b: &(Vec<[f32; 4]>, Vec<[u16; 4]>)) -> usize {
    (0..a.0.len())
        .filter(|&i| a.0[i].map(f32::to_bits) != b.0[i].map(f32::to_bits) || a.1[i] != b.1[i])
        .count()
}

/// The split walk moves no hit: on lavapipe, the thin rig in the field look with its wood crisp
/// reads the same hit position and record at every pixel split and unsplit; with every row that
/// is not translucent crisp, the field look reads `face`'s at every pixel, split and unsplit.
#[test]
fn the_split_walk_moves_no_hit_record() {
    let Some((device, queue)) = software_device(256 << 20) else {
        return;
    };
    let field = preset_look(flythrough::LookPreset::FIELD, ViewMode::Shaded);
    let face = preset_look(flythrough::LookPreset::FACE, ViewMode::Shaded);
    let wood = thin_rig_world(vxe_world::MaterialFlags::CRISP);
    let split = thin_records(&device, &queue, &wood, field, true);
    let unsplit = thin_records(&device, &queue, &wood, field, false);
    let hits = split.1.iter().filter(|r| r[3] & 1 != 0).count();
    let wood_differ = records_differ(&split, &unsplit);
    println!("crisp wood: {hits} hits, split against unsplit {wood_differ} pixels differ");
    let base = flythrough::LookPreset::DEFAULT.materials();
    let mut table = vxe_world::MaterialTable::new();
    for row in base.rows().iter().skip(1) {
        let mut row = row.clone();
        if !row.is_translucent() {
            row.flags = row.flags | vxe_world::MaterialFlags::CRISP;
        }
        table.push(row);
    }
    let table = flythrough::with_wood_flags(&table, vxe_world::MaterialFlags::CRISP);
    let source = ThinRig {
        wood: flythrough::THIN_RIG_WOOD,
    };
    let all = World::generate(source.extent(), &source, &table);
    let face_records = thin_records(&device, &queue, &all, face, true);
    let all_split = records_differ(
        &thin_records(&device, &queue, &all, field, true),
        &face_records,
    );
    let all_unsplit = records_differ(
        &thin_records(&device, &queue, &all, field, false),
        &face_records,
    );
    println!(
        "every row crisp: against face, split {all_split} and unsplit {all_unsplit} pixels differ"
    );
    assert!(hits > 0, "the rig reads no hit");
    assert_eq!(wood_differ, 0, "split against unsplit, crisp wood");
    assert_eq!(all_split, 0, "split against face, every row crisp");
    assert_eq!(all_unsplit, 0, "unsplit against face, every row crisp");
}
