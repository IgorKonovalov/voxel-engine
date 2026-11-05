//! The surface-look harness on the software adapter (lavapipe): normal error in a carved
//! sphere, silhouette error of a rock ball, and the look's flag bits leaving the goldens alone.
//! `VXE_BLESS=1` rewrites the look goldens.

mod support;

use support::*;
use vxe_render::gpu_world::PAGE_UNITS;
use vxe_render::{LightParams, LookParams, NormalMode, ViewMode};
use vxe_world::World;

fn look(normal_mode: NormalMode, view: ViewMode) -> LookParams {
    LookParams {
        normal_mode,
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
    guardrails_of(mode, attrs).map(|g| (g.normal_error, g.silhouette))
}

/// The guardrails of the "not a Lego look" bar: the cavity's normal error (degrees), the
/// pillar's silhouette error (pixels) and the cavity's axis fraction `A`, the share of its
/// hit pixels whose shading normal lies within 5 degrees of a coordinate axis.
#[derive(Clone, Copy, Debug)]
struct Guardrails {
    normal_error: f64,
    silhouette: usize,
    axis_fraction: f64,
}

/// [`Guardrails`] of `mode`, over mirrors with the field's band slots when `attrs` is set.
fn guardrails_of(mode: NormalMode, attrs: bool) -> Option<Guardrails> {
    let look = |view| LookParams {
        normal_mode: mode,
        view,
        ..LookParams::default()
    };
    let cavity = cavity_world_with(look_table());
    let mut rig = if attrs {
        field_mirror_rig(&cavity)?
    } else {
        LookRig::new(&cavity, None)?
    };
    let cam = cavity_camera();
    let normals = rig.render(&cam, &LightParams::at_eye(), look(ViewMode::Normals));
    let normal_error = cavity_normal_error(&normals, &cam);
    let axis = axis_fraction(&normals);

    let pillar = pillar_world_with(flythrough::cave_materials());
    let mut rig = if attrs {
        field_mirror_rig_on(rig.device, rig.queue, &pillar, None)
    } else {
        LookRig::on(rig.device, rig.queue, &pillar, None)
    };
    let cam = pillar_camera();
    let mask = rig.render(&cam, &LightParams::at_eye(), look(ViewMode::HitMask));
    let silhouette = pillar_silhouette_error(&mask, &cam);
    println!(
        "{}: cavity normal error {normal_error:.2} degrees, pillar silhouette error {silhouette} pixels, axis fraction {:.2} %",
        mode.name(),
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

/// The axis fraction `A`: face mode's normals are all axes; contour's is printed,
/// and contour still meets the cavity normal error and silhouette guardrails.
#[test]
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

/// The share of the hit pixels of a `view = Normals` render whose decoded normal lies within
/// 5 degrees of a coordinate axis, a miss (a decoded length under 0.5) not counted.
fn axis_fraction(rgba: &[u8]) -> f64 {
    let cos5 = 5f32.to_radians().cos();
    let (mut hits, mut axial) = (0usize, 0usize);
    for p in rgba.chunks(4) {
        let n = decode_normal(p);
        if n.length() < 0.5 {
            continue;
        }
        hits += 1;
        let n = n.normalize();
        if n.x.abs().max(n.y.abs()).max(n.z.abs()) >= cos5 {
            axial += 1;
        }
    }
    axial as f64 / hits.max(1) as f64
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
/// `cargo nextest run -p flythrough --test look --no-capture every_mode`.
#[test]
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

// ---- Leaks: the cracks view of the field look ----

use support::rigs::{
    POSE_SIZE, axis_slab_camera, axis_slab_world, diagonal_slab_camera, diagonal_slab_world,
    field_rig_on, floor_camera, floor_world, pose_level, pose_rig_on, pose_rig_with,
    tilted_slab_camera, tilted_slab_world, wall_camera, wall_world,
};
use vxe_render::{CrackShares, Exposure, readback};

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
/// record's) lies within this many degrees of the face's outward axis normal. The
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
/// surface, those whose normals-view normals differ by more than
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

/// [`crack_reading`] of `camera` in `look`, then its normals view, and the box
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
        lamp_bounce: false,
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

/// Preset `p`'s look in `view`.
fn preset_look(p: flythrough::LookPreset, view: ViewMode) -> LookParams {
    LookParams {
        normal_mode: p.normal_mode,
        view,
        tuning: p.tuning,
        far_normal: p.far_normal,
        ..LookParams::default()
    }
}

// ---- The field surface (ADR-0040 Alternative B, Plan 0054's scratch) ----

/// The field look (`LookPreset::FIELD`) in `view`.
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
    let table = flythrough::cave_materials();
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
            field_look(ViewMode::Shaded),
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
    let table = flythrough::cave_materials();
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
    let table = flythrough::cave_materials();
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
    let table = flythrough::with_wood_flags(&flythrough::cave_materials(), flags);
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
    let base = flythrough::cave_materials();
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

/// `Frame::frame_bytes` of the default frame at 1080p output from 1280 x 720 internal, after
/// a render in its default look over the thin rig mirrored without band slots: main 28948a5's
/// figure (DEFAULT_MAIN) plus the split walk's 32-byte stub list and 16-byte dispatch words,
/// no hand-off list, and the emissive table of ADR-0043 in place of the screen's emissive
/// history and reservoirs (TABLE_DELTA: + lavapipe's 2^21 cells' 100 663 296 + the blend list's 921 616 + the walk's 92 - 3 686 400 -
/// 7 372 800, `upscale.rs`'s `frame_bytes_hold_the_table_and_no_emissive_history`), the reopened fill's list and
/// dispatch words (+ 65 564), and the preload's store, uniform, dispatch words and idle
/// readback slots (+ 2 271 860).
const DEFAULT_FRAME_BYTES: u64 = 223_130_444;
const DEFAULT_MAIN: u64 = 130_267_168;
const TABLE_DELTA: u64 = 100_663_296 + 921_616 + 92 - 3_686_400 - 7_372_800;
const FILL_BYTES: u64 = 16 + 4 * 16_384 + 12;
const PRELOAD_BYTES: u64 = 152
    + 512
    + 4 * (4 * 32_768 + 17_077 + 4 * 16_384 + 2 * 32_768 + 33 * 33 * 33 * 8)
    + 4_176
    + 24
    + 4 * 32;

/// The split walk's hand-off list (16 B of header and 16 B a pixel) is allocated only by a
/// frame that splits: on lavapipe the default frame (internal 1280 x 720, output 1920 x 1080)
/// rendered in its default look over a mirror without band slots reads `DEFAULT_FRAME_BYTES`,
/// as before the render; the thin rig's frame in the field look with its wood crisp grows by
/// exactly the list at its size on its first render, and not again on its second.
#[test]
fn only_a_split_frame_allocates_the_split_list() {
    let Some((device, queue)) = software_device(256 << 20) else {
        return;
    };
    let wood = thin_rig_world(vxe_world::MaterialFlags::CRISP);
    let mut gpu = vxe_render::GpuWorld::with_config(
        &device,
        vxe_render::MirrorConfig::new(vxe_render::Window::Fixed(wood.extent())),
    );
    gpu.sync_now(&wood, &device, &queue)
        .expect("mirror fits the adapter");
    assert!(!gpu.field_surface(), "a mirror without band slots");
    let frame = vxe_render::Frame::with_sizes(&device, [1280, 720], [1920, 1080]);
    let before = frame.frame_bytes();
    frame.render(
        &device,
        &queue,
        &gpu,
        &flythrough::thin_rig_camera(),
        &LightParams::headlamp(),
    );
    let default = frame.frame_bytes();
    println!(
        "default frame_bytes at 1080p from 1280x720: {before} B before a render, {default} B after; main 28948a5 {DEFAULT_MAIN} B"
    );
    assert_eq!(before, default, "a render without the split walk");
    assert_eq!(default, DEFAULT_FRAME_BYTES);
    assert_eq!(
        default,
        DEFAULT_MAIN + 32 + 16 + TABLE_DELTA + FILL_BYTES + PRELOAD_BYTES
    );

    let mut rig = field_rig_on(device.clone(), queue.clone(), &wood, THIN_SIZE);
    let field = preset_look(flythrough::LookPreset::FIELD, ViewMode::Shaded);
    let unsplit = rig.frame.frame_bytes();
    let _ = rig.render(
        &flythrough::thin_rig_camera(),
        &LightParams::headlamp(),
        field,
    );
    let split = rig.frame.frame_bytes();
    let _ = rig.render(
        &flythrough::thin_rig_camera(),
        &LightParams::headlamp(),
        field,
    );
    let list = 16 + 16 * u64::from(THIN_SIZE[0]) * u64::from(THIN_SIZE[1]);
    println!(
        "thin rig frame_bytes: {unsplit} B before the split walk, {split} B after (+{} B, list {list} B)",
        split - unsplit
    );
    assert_eq!(
        split - unsplit,
        list,
        "the first split frame builds the list"
    );
    assert_eq!(
        rig.frame.frame_bytes(),
        split,
        "a second split frame reuses it"
    );
}

// ---- The default look and the opt-in field look (Plan 0054 Phase 4c) ----

/// The fly-through's default look is `face` (ADR-0041): `LookPreset::DEFAULT` is
/// `LookPreset::FACE` with the far normal at `[2, 6]`, and on lavapipe the level (`--level --seed 1`) at the spawn (looking
/// +x), `VEIN_POSE` and `PATCH_POSE` (384 x 216, the hit-mask view) reads in the default look
/// over a mirror with no band slot (the fly-through's default mirror) the same hit position
/// (bits) and hit record at every pixel as over a mirror with the field's band slots.
#[test]
fn the_default_look_reads_the_same_hit_records_with_and_without_band_slots() {
    assert_eq!(
        flythrough::LookPreset::DEFAULT,
        flythrough::LookPreset {
            far_normal: [2.0, 6.0],
            ..flythrough::LookPreset::FACE
        },
        "the default look is face with the far normal"
    );
    let level = pose_level();
    let Some((device, queue)) = software_device(field_pool_bytes(&level).max(256 << 20)) else {
        return;
    };
    let source = flythrough::cave::CaveSource::new(
        1,
        vxe_world::ChunkExtent::new([0, 0, 0], flythrough::cave::LEVEL_CHUNKS),
    );
    let spawn = source.spawn().0.map(|v| v as f32 + 0.5);
    let spawn_pose = flythrough::Pose {
        name: "spawn",
        position: spawn,
        yaw_deg: 0.0,
        pitch_deg: 0.0,
        exposure: 1.0,
    };
    let records = |field: bool, pose: &flythrough::Pose, look: LookParams| {
        let mut rig = pose_rig_with(device.clone(), queue.clone(), &level, pose, field);
        assert_eq!(rig.gpu.field_surface(), field);
        let camera = if pose.name == "spawn" {
            camera_looking_x(spawn)
        } else {
            pose.camera()
        };
        let look = LookParams {
            view: ViewMode::HitMask,
            ..look
        };
        let _ = rig.render(&camera, &LightParams::headlamp(), look);
        (
            readback::read_rgba32f(&rig.device, &rig.queue, rig.frame.hit_positions()),
            readback::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records()),
        )
    };
    let default = preset_look(flythrough::LookPreset::DEFAULT, ViewMode::HitMask);
    let mut failures = Vec::new();
    for pose in [spawn_pose, flythrough::VEIN_POSE, flythrough::PATCH_POSE] {
        let a = records(false, &pose, default);
        let b = records(true, &pose, default);
        let hits = a.1.iter().filter(|r| r[3] & 1 != 0).count();
        let differ = records_differ(&a, &b);
        println!(
            "{}: {hits} hits in the default look, {differ} pixels differ over band slots",
            pose.name
        );
        if hits == 0 || differ != 0 {
            failures.push(format!("{}: {hits} hits, {differ} differ", pose.name));
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
}

/// Plan 0054 Phase 1's N1 tile-contrast share per scene (`tile edges %`, its log's field table at
/// three decimals, 2026-10-30, lavapipe): the field look's bar, at most this plus half the
/// last printed digit.
const PHASE1_TILE_EDGES_PCT: [(&str, f64); 15] = [
    ("bare floor", 0.000),
    ("rod", 0.649),
    ("lone cube", 0.000),
    ("groove", 0.644),
    ("wall pillar", 0.272),
    ("ridge", 0.645),
    ("free pillar", 0.004),
    ("sheet", 0.773),
    ("tilted slab", 0.000),
    ("45-degree slab", 0.000),
    ("diagonal slab", 0.000),
    ("cavity", 0.188),
    ("pillar", 0.125),
    ("level vein", 0.397),
    ("level patch", 0.633),
];

/// The tile-contrast bar of `scene` (`None` for a scene the table does not name).
fn tile_bar(scene: &str) -> Option<f64> {
    PHASE1_TILE_EDGES_PCT
        .iter()
        .find(|(s, _)| *s == scene)
        .map(|(_, v)| v + 0.0005)
}

/// The opt-in field look has no boxes and no leaks: on lavapipe, in the field look
/// (`LookPreset::FIELD`), every hit on `floor` lies within 1e-3 voxel of `z = 64`
/// and every hit on `wall` within 1e-3 of `x = 80` under the analytic window rule; the rod, the
/// lone cube, the groove, the wall pillar and the ridge read 0 box pixels; every analytic scene
/// and the level at `VEIN_POSE` and `PATCH_POSE`, lamp on and off, read 0 leaks; each scene's
/// tile-contrast share is at most Phase 1's N1 reading ([`PHASE1_TILE_EDGES_PCT`], the poses
/// lamp on); and Plan 0039's guardrails hold: cavity normal error at most 12
/// degrees, pillar silhouette error at most half of face mode's, axis fraction at most 10 %.
#[test]
fn the_field_look_has_no_boxes_and_no_leaks() {
    let level = pose_level();
    let Some((device, queue)) = software_device(field_pool_bytes(&level).max(256 << 20)) else {
        return;
    };
    let look = field_look(ViewMode::Shaded);
    let mut failures = Vec::new();
    let table = flythrough::cave_materials();
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
        let (r, positions) = field_crack_reading(&device, &queue, &world, &camera, look);
        let worst = positions
            .iter()
            .enumerate()
            .filter(|&(i, p)| r.pixels[i].is_some() && kept_by_window(*p, window))
            .map(|(_, p)| (p[axis] - plane).abs())
            .fold(0f32, f32::max);
        println!(
            "{name}: {} hits ({} left out), {} leaks, worst {worst:.2e} voxel off the face",
            r.shares.hits, r.excluded, r.shares.leaks
        );
        if r.shares.hits == 0 || worst > 1e-3 || r.shares.leaks != 0 {
            failures.push(format!(
                "{name}: {} hits, worst {worst}, {} leaks",
                r.shares.hits, r.shares.leaks
            ));
        }
    }
    println!("{FIELD_HEADER}");
    let boxed = ["rod", "lone cube", "groove", "wall pillar", "ridge"];
    let tiles = |scene: &str, r: &BoxReading, failures: &mut Vec<String>| {
        let pct = 100.0 * r.edges as f64 / r.pairs.max(1) as f64;
        match tile_bar(scene) {
            Some(bar) if pct > bar => {
                failures.push(format!("{scene}: tile edges {pct:.3} % over {bar:.4}"));
            }
            Some(_) => {}
            None => failures.push(format!("{scene}: no tile bar")),
        }
    };
    for (scene, world, camera) in field_scenes() {
        let r = scene_box_reading(&device, &queue, &world, &camera, look);
        println!("{}", field_row(&r, &scene, "field"));
        if r.crack.shares.hits == 0 {
            failures.push(format!("{scene}: no hit"));
        }
        if r.crack.shares.leaks != 0 {
            failures.push(format!("{scene}: {} leaks", r.crack.shares.leaks));
        }
        if boxed.contains(&scene.as_str()) && r.box_px != 0 {
            failures.push(format!("{scene}: {} box px", r.box_px));
        }
        tiles(&scene, &r, &mut failures);
    }
    for pose in flythrough::POSES {
        for lamp in [true, false] {
            let r = pose_box_reading(&device, &queue, &level, &pose, look, lamp);
            let scene = format!("level {}", pose.name);
            let label = if lamp { "lamp on" } else { "lamp off" };
            println!("{}", field_row(&r, &scene, &format!("field, {label}")));
            if r.crack.shares.leaks != 0 {
                failures.push(format!("{scene} {label}: {} leaks", r.crack.shares.leaks));
            }
            if lamp {
                tiles(&scene, &r, &mut failures);
            }
        }
    }
    let face = guardrails_of(NormalMode::Face, false).expect("a device");
    let field = guardrails_of(flythrough::LookPreset::FIELD.normal_mode, true).expect("a device");
    println!(
        "guardrails: cavity normal error {:.2} degrees, silhouette {} px against face {}, axis fraction {:.2} %",
        field.normal_error,
        field.silhouette,
        face.silhouette,
        100.0 * field.axis_fraction
    );
    if field.normal_error > 12.0 {
        failures.push(format!("cavity normal error {:.2}", field.normal_error));
    }
    if 2 * field.silhouette > face.silhouette {
        failures.push(format!(
            "silhouette {} against face {}",
            field.silhouette, face.silhouette
        ));
    }
    if field.axis_fraction > 0.10 {
        failures.push(format!(
            "axis fraction {:.2} %",
            100.0 * field.axis_fraction
        ));
    }
    assert!(failures.is_empty(), "{failures:?}");
}

// ---- The seam: a smooth surface meeting crisp cubes (Plan 0054 Phase 4c) ----

use flythrough::thin_rig::{SeamRig, Shape};

/// A seam reading: the crack reading (shaded, then cracks view) of one view at `WIDTH` x
/// `HEIGHT`, with the hit positions and records of its last frame.
struct SeamReading {
    crack: CrackReading,
    positions: Vec<[f32; 4]>,
    records: Vec<[u16; 4]>,
}

fn seam_reading(
    device: &vxe_render::wgpu::Device,
    queue: &vxe_render::wgpu::Queue,
    world: &World,
    camera: &vxe_render::Camera,
) -> SeamReading {
    let mut rig = field_rig_on(device.clone(), queue.clone(), world, [WIDTH, HEIGHT]);
    let crack = crack_reading(
        &mut rig,
        camera,
        &LightParams::headlamp(),
        field_look(ViewMode::Shaded),
        None,
    );
    SeamReading {
        crack,
        positions: readback::read_rgba32f(&rig.device, &rig.queue, rig.frame.hit_positions()),
        records: readback::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records()),
    }
}

impl SeamReading {
    /// With `SEAM_DEBUG` set, prints pixel `i`'s hit position, record and leak flag under
    /// `tag` (a reading aid for a failing seam bar).
    fn dbg(&self, tag: &str, i: usize) {
        if std::env::var_os("SEAM_DEBUG").is_some() {
            let r = self.records[i];
            println!(
                "DBG {tag} px {} {}: p {:?} flags {:#x} mat {} n {:?} leak {:?}",
                i as u32 % WIDTH,
                i as u32 / WIDTH,
                self.positions[i],
                r[3],
                r[2],
                support::rigs::octa16_normal(r[0], r[1]),
                self.crack.pixels[i]
            );
        }
    }

    fn hit(&self, i: usize) -> Option<[f32; 3]> {
        self.crack.pixels[i].map(|_| {
            let p = self.positions[i];
            [p[0], p[1], p[2]]
        })
    }

    /// Whether pixel `i`'s hit is a cube hit of `crisp`: not a field hit (record flag bit 1),
    /// material `crisp`, its normal along an axis.
    fn cube_hit(&self, i: usize, crisp: vxe_world::MaterialId) -> bool {
        let r = self.records[i];
        let n = support::rigs::octa16_normal(r[0], r[1]);
        r[3] & 2 == 0 && r[2] == crisp && n.iter().any(|c| c.abs() >= 0.9999)
    }

    /// The leaks and misses among the pixels whose primary ray meets `aabb` (a box `[lo, hi]`
    /// lying on or just behind the seam's surface, so every such ray hits).
    fn leaks_and_misses(
        &self,
        camera: &vxe_render::Camera,
        aabb: [[f32; 3]; 2],
    ) -> (u64, u64, u64) {
        let (mut n, mut leaks, mut misses) = (0, 0, 0);
        for i in 0..self.crack.pixels.len() {
            let (col, row) = (i as u32 % WIDTH, i as u32 / WIDTH);
            if !ray_meets(
                camera.position.to_array(),
                pixel_ray(camera, col, row).to_array(),
                aabb,
            ) {
                continue;
            }
            n += 1;
            if self.crack.pixels[i] != Some(false) {
                self.dbg("box", i);
            }
            match self.crack.pixels[i] {
                None => misses += 1,
                Some(true) => leaks += 1,
                Some(false) => {}
            }
        }
        (n, leaks, misses)
    }
}

/// Whether the ray `o + d t`, `t > 0`, meets the closed box `[lo, hi]` (the slab test).
fn ray_meets(o: [f32; 3], d: [f32; 3], [lo, hi]: [[f32; 3]; 2]) -> bool {
    let (mut t0, mut t1) = (0f32, f32::INFINITY);
    for k in 0..3 {
        if d[k].abs() < 1e-9 {
            if o[k] < lo[k] || o[k] > hi[k] {
                return false;
            }
            continue;
        }
        let (a, b) = ((lo[k] - o[k]) / d[k], (hi[k] - o[k]) / d[k]);
        t0 = t0.max(a.min(b));
        t1 = t1.min(a.max(b));
    }
    t0 <= t1
}

/// Whether `p` lies within `eps` of the closed box `[lo, hi]`.
fn on_box(p: [f32; 3], lo: [i32; 3], hi: [i32; 3], eps: f32) -> bool {
    (0..3).all(|k| p[k] >= lo[k] as f32 - eps && p[k] <= hi[k] as f32 + eps)
}

/// The voxel boxes `[lo, hi)` of a thin-rig feature (one box, or one per diagonal voxel).
fn feature_boxes(f: &flythrough::thin_rig::Feature) -> Vec<([i32; 3], [i32; 3])> {
    match f.shape {
        Shape::Box { lo, hi } => vec![(lo, hi)],
        Shape::Diagonal { start } => (0..flythrough::thin_rig::LENGTH)
            .map(|k| {
                let v = [start[0] + k, start[1], start[2] + k];
                (v, [v[0] + 1, v[1] + 1, v[2] + 1])
            })
            .collect(),
    }
}

/// A seam view: its name and camera.
fn seam_view(
    name: &'static str,
    eye: [f32; 3],
    target: [f32; 3],
) -> (&'static str, vxe_render::Camera) {
    (name, flythrough::camera_looking_at(eye, target))
}

/// The worked collar heights off the face (voxels) of the one-voxel branch, the 2x2 rod and
/// the 3x3 rod (Phase 4c's seam worked from the counts).
const WORKED_COLLAR: [(&str, f32); 3] = [
    ("one-voxel branch", 0.19),
    ("2x2 rod", 0.43),
    ("3x3 rod", 0.75),
];

/// A smooth surface meets a crisp cube without a gap, as far as the opt-in field look meets
/// it (Plan 0054 Phase 4c's close: the seam is a reading, its met bars asserted): on lavapipe,
/// in the field look, the seam rig ([`SeamRig`], the rig wood crisp by its row) and the thin
/// rig ([`ThinRig`], its wood crisp), each seam seen head-on and at 45 degrees. Every view
/// reads 0 misses among the pixels whose ray meets the seam's box, and the inlay's and the
/// L's boxes 0 leaks. The inlay, head-on: every hit with `x` in `[12, 52]` and `z` in
/// `[68, 108]` (the wall's face, the inlay included, four voxels clear of its edges) lies
/// within 1e-3 voxel of `y = 100`, and every hit more than one voxel inside the inlay's border
/// is a cube hit of the wood. The trunk: among the hits on its branch face (`x >= 66.5`, `y`
/// in `[63.5, 65.5]` clear of the trunk's edges, `z` in `[70, 102]`) that are not on a
/// feature, every one more than 2 voxels (Chebyshev, in `y` and `z`) from every feature's
/// footprint lies within 1e-3 of `x = 67`; every hit on a feature (within 1e-3 of its voxels)
/// more than 2 voxels from the trunk is a cube hit. Printed, not asserted (the opt-in look's
/// disclosed defects): the trunk box's leaks, the collar hits more than a voxel off the plane
/// or not of the rock (those of material 0 counted apart), each feature's collar height (the
/// highest collar hit off the face) against the worked 0.19, 0.43 and 0.75 voxel, the L's hits
/// within 1.5 voxels of its bounding box above `z = 66` that are not cube hits on a slab, and
/// the dark specks on the trunk face head-on with the branches crisp and smooth.
#[test]
fn a_smooth_surface_meets_a_crisp_cube_without_a_gap() {
    use flythrough::thin_rig::{INLAY_X, INLAY_Z, L_THICK, L_TOP, L_X, L_Y, TRUNK_LO, WALL_Y};
    let Some((device, queue)) = software_device(256 << 20) else {
        return;
    };
    let wood = flythrough::THIN_RIG_WOOD;
    let rock = flythrough::cave::ROCK;
    let table = flythrough::with_wood_flags(
        &flythrough::cave_materials(),
        vxe_world::MaterialFlags::CRISP,
    );
    let source = SeamRig { crisp: wood };
    let seam = World::generate(source.extent(), &source, &table);
    let thin = thin_rig_world(vxe_world::MaterialFlags::CRISP);
    let mut failures = Vec::new();
    // `leaks_barred`: whether the box's leaks are asserted or only printed.
    let check_box = |name: &str,
                     r: &SeamReading,
                     cam: &vxe_render::Camera,
                     aabb: [[f32; 3]; 2],
                     leaks_barred: bool,
                     failures: &mut Vec<String>| {
        let (n, leaks, misses) = r.leaks_and_misses(cam, aabb);
        println!(
            "{name}: {n} px in the seam's box, {leaks} leaks{}, {misses} misses; {} hits, {} leaks in the frame",
            if leaks_barred { "" } else { " (printed)" },
            r.crack.shares.hits,
            r.crack.shares.leaks
        );
        if n == 0 || (leaks_barred && leaks != 0) || misses != 0 {
            failures.push(format!("{name}: {n} px, {leaks} leaks, {misses} misses"));
        }
    };

    // The inlay: a flat rock wall, face y = 100, the wood 8 x 8 and 3 deep flush in it.
    let wall_y = WALL_Y[0] as f32;
    let centre = [32.0f32, wall_y, 88.0];
    let inlay_box = [[26.0, wall_y - 0.5, 82.0], [38.0, wall_y + 0.5, 94.0]];
    for (name, cam) in [
        seam_view("inlay head-on", [32.0, 70.0, 88.0], centre),
        seam_view("inlay 45", [32.0 - 21.2, wall_y - 21.2, 88.0], centre),
    ] {
        let r = seam_reading(&device, &queue, &seam, &cam);
        check_box(name, &r, &cam, inlay_box, true, &mut failures);
        if name != "inlay head-on" {
            continue;
        }
        let (mut on_wall, mut worst, mut inner, mut not_cube) = (0u64, 0f32, 0u64, 0u64);
        for i in 0..r.positions.len() {
            let Some(p) = r.hit(i) else { continue };
            if (12.0..=52.0).contains(&p[0]) && (68.0..=108.0).contains(&p[2]) {
                on_wall += 1;
                worst = worst.max((p[1] - wall_y).abs());
            }
            let inside = p[0] > (INLAY_X[0] + 1) as f32
                && p[0] < (INLAY_X[1] - 1) as f32
                && p[2] > (INLAY_Z[0] + 1) as f32
                && p[2] < (INLAY_Z[1] - 1) as f32
                && p[1] < wall_y + 0.5;
            if inside {
                inner += 1;
                if !r.cube_hit(i, wood) {
                    not_cube += 1;
                    r.dbg("inlay", i);
                }
            }
        }
        println!(
            "{name}: {on_wall} hits on the wall, worst {worst:.2e} voxel off its plane; {inner} inside the inlay, {not_cube} not a cube hit"
        );
        if on_wall == 0 || worst > 1e-3 {
            failures.push(format!("{name}: {on_wall} wall hits, worst {worst}"));
        }
        if inner == 0 || not_cube != 0 {
            failures.push(format!("{name}: {inner} inlay hits, {not_cube} not cubes"));
        }
    }

    // The trunk: its +x face x = 67 carries the branches' roots.
    let face_x = (TRUNK_LO[0] + 5) as f32;
    let trunk_target = [face_x, 64.5, 86.5];
    let trunk_box = [[face_x - 0.5, 63.0, 70.0], [face_x, 65.0, 100.0]];
    let plus_x: Vec<_> = FEATURES
        .iter()
        .filter(|f| f.measured(0).0[0] >= TRUNK_LO[0] + 5)
        .collect();
    let footprint_gap = |f: &flythrough::thin_rig::Feature, p: [f32; 3]| -> f32 {
        // Chebyshev distance in y and z from p to the feature's voxels within one voxel of
        // the face (the layers the face's and its front air's counts read).
        feature_boxes(f)
            .iter()
            .filter(|(lo, _)| lo[0] <= TRUNK_LO[0] + 6)
            .map(|(lo, hi)| {
                (1..3)
                    .map(|k| (lo[k] as f32 - p[k]).max(p[k] - hi[k] as f32).max(0.0))
                    .fold(0f32, f32::max)
            })
            .fold(f32::INFINITY, f32::min)
    };
    let on_feature = |p: [f32; 3]| {
        FEATURES.iter().any(|f| {
            feature_boxes(f)
                .iter()
                .any(|&(lo, hi)| on_box(p, lo, hi, 1e-3))
        })
    };
    for (name, cam) in [
        seam_view("trunk head-on", [face_x + 33.5, 64.5, 86.5], trunk_target),
        seam_view("trunk 45", [face_x + 24.0, 64.5 - 24.0, 86.5], trunk_target),
    ] {
        let r = seam_reading(&device, &queue, &thin, &cam);
        check_box(name, &r, &cam, trunk_box, false, &mut failures);
        let mut collar = vec![0f32; plus_x.len()];
        let (mut flat, mut flat_worst, mut near, mut near_bad, mut near_air) =
            (0u64, 0f32, 0u64, 0u64, 0u64);
        let (mut feat, mut feat_bad) = (0u64, 0u64);
        for i in 0..r.positions.len() {
            let Some(p) = r.hit(i) else { continue };
            if on_feature(p) {
                let from_trunk = if p[0] >= face_x {
                    p[0] - face_x
                } else {
                    TRUNK_LO[0] as f32 - p[0]
                };
                if from_trunk > 2.0 {
                    feat += 1;
                    feat_bad += u64::from(!r.cube_hit(i, wood));
                }
                continue;
            }
            if p[0] < face_x - 0.5
                || !(63.5..=65.5).contains(&p[1])
                || !(70.0..=102.0).contains(&p[2])
            {
                continue;
            }
            let gaps: Vec<f32> = plus_x.iter().map(|f| footprint_gap(f, p)).collect();
            let off = p[0] - face_x;
            if gaps.iter().all(|&g| g > 2.0) {
                flat += 1;
                flat_worst = flat_worst.max(off.abs());
                if off.abs() > 1e-3 {
                    r.dbg("flat", i);
                }
            } else {
                near += 1;
                let bad = !(-1e-3..=1.0).contains(&off) || r.records[i][2] != rock;
                if bad {
                    r.dbg("collar", i);
                }
                near_bad += u64::from(bad);
                near_air += u64::from(r.records[i][2] == 0);
                for (k, &g) in gaps.iter().enumerate() {
                    if g <= 2.0 {
                        collar[k] = collar[k].max(off);
                    }
                }
            }
        }
        println!(
            "{name}: {flat} face hits clear of the roots, worst {flat_worst:.2e} off the plane; {near} collar hits, {near_bad} off by more than a voxel or not rock (printed; {near_air} of material 0); {feat} feature hits clear of the trunk, {feat_bad} not a cube hit"
        );
        for (f, h) in plus_x.iter().zip(&collar) {
            let worked = WORKED_COLLAR
                .iter()
                .find(|(n, _)| *n == f.name)
                .map(|w| format!("{:.2}", w.1));
            println!(
                "  {name} collar {}: {h:.3} voxel off the face (worked {})",
                f.name,
                worked.unwrap_or_else(|| "-".to_owned())
            );
        }
        if flat == 0 || flat_worst > 1e-3 {
            failures.push(format!("{name}: {flat} flat hits, worst {flat_worst}"));
        }
        if feat == 0 || feat_bad != 0 {
            failures.push(format!("{name}: {feat} feature hits, {feat_bad} not cubes"));
        }
    }

    // The specks on the trunk face head-on, branches crisp and smooth.
    let smooth = thin_rig_world(vxe_world::MaterialFlags::empty());
    let head_on = flythrough::camera_looking_at([face_x + 33.5, 64.5, 86.5], trunk_target);
    for (label, world) in [("crisp", &thin), ("smooth", &smooth)] {
        let r = seam_reading(&device, &queue, world, &head_on);
        let specks = dark_specks(&r.crack.shaded)
            .into_iter()
            .filter(|&i| {
                r.hit(i).is_some_and(|p| {
                    p[0] >= face_x - 0.5
                        && p[0] <= face_x + 1.0
                        && (TRUNK_LO[1] as f32..=(TRUNK_LO[1] + 5) as f32).contains(&p[1])
                        && r.records[i][2] == rock
                })
            })
            .count();
        println!("trunk face head-on, branches {label}: {specks} dark specks");
    }

    // The L: two wood slabs 3 thick meeting at 90 degrees on the rock floor.
    let slabs = [
        ([L_X[0], L_Y[0], 64], [L_X[1], L_Y[0] + L_THICK, L_TOP]),
        ([L_X[0], L_Y[0], 64], [L_X[0] + L_THICK, L_Y[1], L_TOP]),
    ];
    let corner = [(L_X[0] + L_THICK) as f32, (L_Y[0] + L_THICK) as f32];
    let l_box = [
        [corner[0] - 0.5, corner[1] - 0.5, 70.0],
        [corner[0] + 6.0, corner[1] + 6.0, 90.0],
    ];
    for (name, cam) in [
        seam_view("L head-on", [88.0, 50.0, 80.5], [88.0, corner[1], 80.5]),
        seam_view(
            "L 45",
            [corner[0] + 20.0, corner[1] + 20.0, 80.5],
            [corner[0], corner[1], 80.5],
        ),
    ] {
        let r = seam_reading(&device, &queue, &seam, &cam);
        check_box(name, &r, &cam, l_box, true, &mut failures);
        let (mut n, mut bad, mut corner_px) = (0u64, 0u64, 0u64);
        for i in 0..r.positions.len() {
            let Some(p) = r.hit(i) else { continue };
            let near_l = p[2] > 66.0
                && p[2] <= L_TOP as f32 + 1.5
                && p[0] >= L_X[0] as f32 - 1.5
                && p[0] <= L_X[1] as f32 + 1.5
                && p[1] >= L_Y[0] as f32 - 1.5
                && p[1] <= L_Y[1] as f32 + 1.5;
            if !near_l {
                continue;
            }
            n += 1;
            if (p[0] - corner[0]).abs() <= 1.0 && (p[1] - corner[1]).abs() <= 1.0 {
                corner_px += 1;
            }
            let on = slabs.iter().any(|&(lo, hi)| on_box(p, lo, hi, 1e-3));
            if !on || !r.cube_hit(i, wood) {
                bad += 1;
                r.dbg("L", i);
            }
        }
        println!(
            "{name}: {n} hits on the L, {corner_px} within a voxel of its concave corner, {bad} not a cube hit on a slab (printed)"
        );
        if n == 0 {
            failures.push(format!("{name}: no hit on the L"));
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
}
