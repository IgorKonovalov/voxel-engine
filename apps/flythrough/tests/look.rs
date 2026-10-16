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
    let cavity = cavity_world();
    let mut rig = if attrs {
        attr_rig(&cavity)?
    } else {
        LookRig::new(&cavity, None)?
    };
    let cam = cavity_camera();
    let normals = rig.render(&cam, &LightParams::at_eye(), look(mode, ViewMode::Normals));
    let normal_error = cavity_normal_error(&normals, &cam);

    let pillar = pillar_world();
    let mut rig = if attrs {
        attr_rig_on(rig.device, rig.queue, &pillar, None)
    } else {
        LookRig::on(rig.device, rig.queue, &pillar, None)
    };
    let cam = pillar_camera();
    let mask = rig.render(&cam, &LightParams::at_eye(), look(mode, ViewMode::HitMask));
    let silhouette = pillar_silhouette_error(&mask, &cam);
    println!(
        "{}: cavity normal error {normal_error:.2} degrees, pillar silhouette error {silhouette} pixels",
        mode.name()
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
