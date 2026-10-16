//! Golden images of the cave on the software adapter (lavapipe). `VXE_BLESS=1` rewrites them.

mod support;

use flythrough::cave::ROCK;
use flythrough::cave_materials;
use support::*;
use vxe_render::{Camera, GpuWorld, LightParams, Marcher, readback};
use vxe_world::*;

/// The scene from the spawn pose under each of `lights`, on one device.
fn render(world: &World, lights: &[LightParams]) -> Option<Vec<Vec<u8>>> {
    render_from(world, &scene_camera(), lights)
}

/// `world` from `camera` under each of `lights`, on one device.
fn render_from(world: &World, camera: &Camera, lights: &[LightParams]) -> Option<Vec<Vec<u8>>> {
    let full = world.stats().full_bricks as u64;
    let (device, queue) = software_device((full + 4096) * 1024)?;
    let mut gpu = GpuWorld::new(&device, world.extent());
    gpu.sync(world, &queue).expect("mirror fits the adapter");
    let marcher = Marcher::new(&device, WIDTH, HEIGHT);
    Some(
        lights
            .iter()
            .map(|light| {
                marcher.render(&device, &queue, &gpu, camera, light);
                readback::read_rgba8(&device, &queue, marcher.target())
            })
            .collect(),
    )
}

fn carved_world() -> World {
    let world = scene_world();
    let r = world.apply(&scene_carve(&world)).unwrap();
    assert!(!r.changed_from.is_empty(), "the carve removes rock");
    world
}

#[test]
fn primary_golden_shows_the_carve() {
    let Some(images) = render(&carved_world(), &[LightParams::at_eye()]) else {
        return;
    };
    assert_golden("primary.png", &images[0]);

    // The golden holds structure: at least two colour bins each cover 5 % of the pixels.
    let (golden, _, _) = read_png(&golden_path("primary.png"));
    let bins = bins_covering(&golden, 0.05);
    println!("bins covering 5 %: {bins}");
    assert!(bins >= 2, "{bins} bins");

    // The carve is visible: the uncarved scene differs from the golden.
    let plain = render(&scene_world(), &[LightParams::at_eye()]).unwrap();
    let f = fraction_differing(&plain[0], &golden, 8);
    println!(
        "uncarved differs from golden at {:.2} % of pixels",
        f * 100.0
    );
    assert!(f >= 0.02, "{:.2} %", f * 100.0);
}

#[test]
fn headlamp_shadows_only_remove_light() {
    let lit = LightParams::headlamp();
    let unshadowed = LightParams {
        shadows: false,
        ..lit
    };
    let Some(images) = render(&carved_world(), &[lit, unshadowed]) else {
        return;
    };
    let (shadowed, plain) = (&images[0], &images[1]);
    assert_golden("headlamp.png", shadowed);

    assert_eq!(
        darker_pixels(shadowed, plain),
        0,
        "pixels darker without shadows"
    );
    let f = fraction_differing(plain, shadowed, 8);
    println!(
        "shadows darken {:.2} % of pixels by more than 8/255",
        f * 100.0
    );
}

#[test]
fn masks_move_only_isolated_pixels() {
    for (name, lit) in [
        ("at_eye", LightParams::at_eye()),
        ("headlamp", LightParams::headlamp()),
    ] {
        let unmasked = LightParams {
            masks: false,
            ..lit
        };
        let Some(images) = render(&carved_world(), &[lit, unmasked]) else {
            return;
        };
        let differing = differing_pixels(&images[0], &images[1], 2);
        println!(
            "{name}: masks on and off differ at {} pixels by more than 2/255",
            differing.len()
        );
        // The golden floor: 0.1 % of 57 600 pixels.
        assert!(differing.len() <= 57, "{name}: {} pixels", differing.len());
        // A skip past an occupied block opens a hole several pixels wide; a corner tie broken
        // the other way moves one pixel.
        for (i, &(c, r)) in differing.iter().enumerate() {
            for &(c2, r2) in &differing[i + 1..] {
                assert!(
                    c.abs_diff(c2) > 1 || r.abs_diff(r2) > 1,
                    "{name}: differing pixels ({c}, {r}) and ({c2}, {r2}) touch"
                );
            }
        }
    }
}

/// Pixels (column, row) whose largest channel difference between `a` and `b` exceeds
/// `tolerance`.
fn differing_pixels(a: &[u8], b: &[u8], tolerance: u8) -> Vec<(u32, u32)> {
    a.chunks(4)
        .zip(b.chunks(4))
        .enumerate()
        .filter(|(_, (p, q))| (0..3).any(|k| p[k].abs_diff(q[k]) > tolerance))
        .map(|(i, _)| (i as u32 % WIDTH, i as u32 / WIDTH))
        .collect()
}

/// Pixels of `plain` darker than `shadowed` in any channel.
fn darker_pixels(shadowed: &[u8], plain: &[u8]) -> usize {
    shadowed
        .chunks(4)
        .zip(plain.chunks(4))
        .filter(|(s, p)| (0..3).any(|k| p[k] < s[k]))
        .count()
}

/// Fills voxels `[min, max)` with `material`.
fn fill_voxels(world: &World, min: [i64; 3], max: [i64; 3], material: MaterialId) {
    let op = EditOp::box_(
        Fixed3(min.map(|v| v * FIXED_ONE)),
        Fixed3(max.map(|v| v * FIXED_ONE)),
        Mode::Fill(material),
    );
    world.apply(&op).expect("the occluder scene is resident");
}

#[test]
fn headlamp_slab_casts_the_computed_shadow() {
    // Air over chunks [0, 2)^3, a rock wall at x [44, 46) and a slab at x [28, 30), y and z
    // [56, 68). Eye and lamp both sit inside the slab's y and z ranges, so the slab's near
    // face shadows an L-shaped band of the wall along the slab's left and top edges.
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [2, 2, 2]),
        &UniformSource(0),
        &cave_materials(),
    );
    fill_voxels(&world, [44, 0, 0], [46, 128, 128], ROCK);
    fill_voxels(&world, [28, 56, 56], [30, 68, 68], ROCK);
    let camera = Camera {
        position: [8.5, 64.5, 64.5].into(),
        yaw: 0.0,
        pitch: 0.0,
        vfov: 70f32.to_radians(),
    };
    let lit = LightParams::headlamp();
    let unshadowed = LightParams {
        shadows: false,
        ..lit
    };
    let Some(images) = render_from(&world, &camera, &[lit, unshadowed]) else {
        return;
    };
    let (shadowed, plain) = (&images[0], &images[1]);

    assert_eq!(
        darker_pixels(shadowed, plain),
        0,
        "pixels darker without shadows"
    );
    let mut brighter = 0;
    for (i, (s, p)) in shadowed.chunks(4).zip(plain.chunks(4)).enumerate() {
        let (col, row) = (i as u32 % WIDTH, i as u32 / WIDTH);
        if (0..3).any(|k| p[k].abs_diff(s[k]) > 8) {
            brighter += 1;
            assert!(
                (130..=207).contains(&col) && (63..=140).contains(&row),
                "shadowed pixel at column {col}, row {row} lies outside the computed band"
            );
        } else {
            assert_eq!(
                s, p,
                "column {col}, row {row} differs without being shadowed"
            );
        }
    }
    println!("slab shadow: {brighter} pixels brighter by more than 8/255 without shadows");
    assert!(
        (820..=840).contains(&brighter),
        "{brighter} shadowed pixels, 830 computed"
    );
}
