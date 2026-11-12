//! The surface-look harness on the software adapter (lavapipe): normal error in a carved sphere,
//! silhouette error of a rock ball, the far normal, every mode, and the look's flag bits leaving
//! the goldens alone. `VXE_BLESS=1` rewrites the look goldens.

mod support;

use support::look::*;
use support::*;

use vxe_render::internals::PAGE_UNITS;
use vxe_render::{LightParams, LookParams, NormalMode, ViewMode};
use vxe_world::World;

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
/// default: shaded, face normals.
#[test]
fn the_default_look_leaves_the_goldens_byte_for_byte() {
    if std::env::var("VXE_BLESS").as_deref() == Ok("1") {
        return;
    }
    let default = LookParams {
        normal_mode: NormalMode::Face,
        view: ViewMode::Shaded,
        translucent_layers: 4,
        cutaway: false,
        tuning: vxe_render::SurfaceTuning::default(),
        far_normal: [0.0; 2],
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
#[ignore = "soak"]
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
#[ignore = "soak"]
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

/// The axis fraction `A`: face mode's normals are all axes; contour's is printed,
/// and contour still meets the cavity normal error and silhouette guardrails.
#[test]
#[ignore = "soak"]
fn axis_fraction_of_face_and_contour() {
    let Some(face) = guardrails_of(NormalMode::Face, false) else {
        return;
    };
    let contour = guardrails_of(NormalMode::Contour, true).unwrap();
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
#[ignore = "soak"]
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

/// The contour golden scene's dark specks with and without the headlamp's shadow ray, in the
/// field look (`NormalMode::Contour`) and in face mode, printed: the shadows-on field render is
/// `look_contour.png`. The field's shadow ray walks the voxels from the hit along its normal,
/// so a field hit inside a voxel's cube can start shadowed; its count against face's is a
/// reading (ADR-0041), not a bar.
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
    println!(
        "only with shadows: field {} against face {}",
        only_with[0], only_with[1]
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

/// `LookParams::far_normal [2, 6]` on the pillar, seen along +x at 320 x 180 with 70 degrees of
/// vertical field of view. A view's tangents span `2 tan(35 deg) 320 / 180 = 2.4896` across
/// its 320 pixels, so a voxel at distance `t` spans `128.53 / t` pixels. The camera `d` voxels
/// from the sphere's centre (radius 20) hits it from `d - 20` at the pole to `sqrt(d^2 - 400)`
/// at the silhouette. `d = 25`: `t` at most 15, at least 8.5 pixels a voxel, so the normals
/// view is `face`'s. `d = 90`: `t` at least 70, at most 1.84 pixels a voxel, so it is
/// `gradient27`'s. `d = 57`: `t` from 37 to 53.4, 3.47 to 2.41 pixels a voxel, inside the
/// band, so it differs from both.
#[test]
fn the_far_normal_is_face_up_close_and_gradient27_far() {
    let world = pillar_world_with(flythrough::cave_materials());
    let Some(mut rig) = LookRig::new(&world, None) else {
        return;
    };
    let camera = |d: f32| vxe_render::Camera {
        position: [PILLAR_CENTRE[0] - d, PILLAR_CENTRE[1], PILLAR_CENTRE[2]].into(),
        ..pillar_camera()
    };
    let far = LookParams {
        far_normal: [2.0, 6.0],
        ..look(NormalMode::Face, ViewMode::Normals)
    };
    let face = look(NormalMode::Face, ViewMode::Normals);
    let gradient = look(NormalMode::Gradient27, ViewMode::Normals);
    let mut views = |d: f32| {
        let cam = camera(d);
        let light = LightParams::at_eye();
        (
            rig.render(&cam, &light, far),
            rig.render(&cam, &light, face),
            rig.render(&cam, &light, gradient),
        )
    };
    let differing =
        |a: &[u8], b: &[u8]| a.chunks(4).zip(b.chunks(4)).filter(|(p, q)| p != q).count();
    // A miss is black in the normals view; each distance must show the sphere.
    let hits = |a: &[u8]| a.chunks(4).filter(|p| p[..3] != [0, 0, 0]).count();

    let (blend, f, g) = views(25.0);
    assert!(hits(&blend) > 1000, "{} hit pixels", hits(&blend));
    assert!(
        differing(&f, &g) > 1000,
        "face and gradient27 differ up close"
    );
    assert_eq!(differing(&blend, &f), 0, "up close it is face's");

    let (blend, f, g) = views(90.0);
    assert!(hits(&blend) > 100, "{} hit pixels", hits(&blend));
    assert!(
        differing(&f, &g) > 100,
        "face and gradient27 differ far off"
    );
    assert_eq!(differing(&blend, &g), 0, "far off it is gradient27's");

    let (blend, f, g) = views(57.0);
    assert!(hits(&blend) > 100, "{} hit pixels", hits(&blend));
    let (from_face, from_gradient) = (differing(&blend, &f), differing(&blend, &g));
    println!(
        "in the band, of {} hit pixels: {from_face} differ from face, {from_gradient} from gradient27",
        hits(&blend)
    );
    assert!(from_face > hits(&blend) / 2, "{from_face}");
    assert!(from_gradient > hits(&blend) / 4, "{from_gradient}");
}

/// The comparison table's image metrics, one line per mode, in one run:
/// `cargo nextest run -p flythrough --test gpu_look --run-ignored only --no-capture -E
/// 'test(=every_mode)'`.
#[test]
#[ignore = "soak"]
fn every_mode() {
    println!("| mode | cavity normal error (degrees) | pillar silhouette error (px) |");
    println!("|---|---|---|");
    for mode in NormalMode::ALL {
        let Some((error, silhouette)) = metrics_with(mode, mode == NormalMode::Contour) else {
            return;
        };
        println!("| {} | {error:.2} | {silhouette} |", mode.name());
    }
}
