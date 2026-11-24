//! `chapel.png`: the chapel of the structures library placed in the dark cave ahead of the
//! spawn (`--vox`'s path), lit by its own lamps and the headlamp, on the software adapter
//! (lavapipe).
//! `VXE_BLESS=1` rewrites it.

mod support;

use flythrough::cave::CaveSource;
use flythrough::structures::{self, CHAPEL_GLASS, CHAPEL_LAMP};
use svet_render::{Camera, LightParams, Lighting};
use svet_world::{ChunkExtent, FIXED_ONE, Fixed3, MaterialId, MaterialTable, VoxelCoord, World};

/// The chapel's nave: 40 x 16 voxels in plan, its door in the east wall.
const NAVE: [u32; 2] = [40, 16];

/// The eye in the nave's own voxels: by its south wall, 0.95 m up, looking north-west across
/// the nave at the north wall's window bays and the east lamp over the aisle.
const EYE_IN_NAVE: [f32; 3] = [30.5, 3.5, 9.5];

/// Frames the golden settles over.
const FRAMES: u32 = 64;

/// The cave's table (no emissive ore: the chapel's lamps are the only emitters) with the chapel's
/// palette rows, the chapel's scene placed ahead of the cave's spawn with its box cleared, and
/// a world of the chunks its box and margin meet; with the glass and lamp rows' ids.
fn chapel_world() -> (World, [i32; 3], MaterialId, MaterialId) {
    let path = std::path::PathBuf::from(structures::LIBRARY).join("chapel/chapel.vox");
    let file = structures::read(&path).expect("reads");
    let mut table: MaterialTable = flythrough::cave_materials();
    structures::add_rows(&file, &mut table);
    let scene = structures::scene(&file, &table, 1).expect("loads");
    let cave = CaveSource::unbounded(1);
    let offset = structures::offset(&scene, cave.spawn(), None);
    let (lo, hi) = scene.bounds;
    let m = structures::CLEAR_MARGIN;
    let extent = ChunkExtent::new(
        [0, 1, 2].map(|k| (lo[k] + offset[k] - m).div_euclid(64) - 1),
        [0, 1, 2].map(|k| (hi[k] + offset[k] + m).div_euclid(64) + 2),
    );
    let (_, nave_at, _) = scene
        .placements
        .iter()
        .find(|(i, _, _)| scene.prefabs[*i].size()[..2] == NAVE)
        .expect("the nave");
    let eye = [0, 1, 2].map(|k| nave_at.0[k] + offset[k]);
    let row = |i: u8| {
        let name = svet_assets::palette_row(&file, i, 0.0).name;
        table
            .rows()
            .iter()
            .position(|m| m.name == name)
            .expect("a chapel row") as MaterialId
    };
    let (glass, lamp) = (row(CHAPEL_GLASS), row(CHAPEL_LAMP));
    let source = structures::source(cave, &scene, offset, true);
    (World::generate(extent, &source, &table), eye, glass, lamp)
}

/// The golden's camera: [`EYE_IN_NAVE`] from the nave's corner `nave`, looking north-west
/// (yaw 120 degrees from +x), 8 degrees up.
fn chapel_camera(nave: [i32; 3]) -> Camera {
    Camera {
        position: [0, 1, 2].map(|k| nave[k] as f32 + EYE_IN_NAVE[k]).into(),
        yaw: 120f32.to_radians(),
        pitch: 8f32.to_radians(),
        vfov: 70f32.to_radians(),
    }
}

/// The material of the first voxel pixel (`col`, `row`)'s primary ray meets within 40 m.
fn first_material(world: &World, camera: &Camera, col: u32, row: u32) -> Option<MaterialId> {
    let d = support::pixel_ray(camera, col, row);
    let one = FIXED_ONE as f32;
    let o = Fixed3(camera.position.to_array().map(|v| (v * one).floor() as i64));
    let dir = d.to_array().map(|v| (v * one).round() as i64);
    world
        .raycast(o, dir, 400 * FIXED_ONE)
        .ok()
        .flatten()
        .map(|h| h.material)
}

/// The pixels whose primary ray, and the rays of their four neighbours, first meet `want`:
/// a material's pixels away from its silhouette.
fn inner_pixels(hits: &[Option<MaterialId>], want: MaterialId) -> Vec<usize> {
    use support::{HEIGHT, WIDTH};
    let (w, h) = (WIDTH as usize, HEIGHT as usize);
    let mut out = Vec::new();
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let i = y * w + x;
            if [i, i - 1, i + 1, i - w, i + w]
                .iter()
                .all(|&j| hits[j] == Some(want))
            {
                out.push(i);
            }
        }
    }
    out
}

/// Done-when 1 of Plan 0083 Phase 8: `chapel.png`, the chapel in the dark cave from inside its
/// nave after [`FRAMES`] frames, the headlamp on (the cave's only white light: lit by the
/// amber lamps alone the glass reads amber) and the store baked around the eye, within
/// the goldens' floor (0.1 % of pixels over 2/255) of its blessed image on lavapipe; the glass
/// pixels (those whose ray and its neighbours' first meet the glass row) are bluer than red
/// on the mean, and the lamps' pixels' mean luma is over 0.5.
#[test]
fn chapel_golden() {
    use support::*;
    let (world, nave, glass, lamp) = chapel_world();
    let camera = chapel_camera(nave);
    let eye = camera.position.to_array().map(|v| v.floor() as i32);
    assert_eq!(
        world.material_at(VoxelCoord(eye)),
        Some(svet_world::AIR),
        "the eye in the nave's air"
    );
    let Some(mut rig) = LookRig::new(&world, None) else {
        return;
    };
    rig.frame.set_lighting(Lighting {
        lamp: true,
        ..Lighting::default()
    });
    rig.gpu.set_focus(VoxelCoord(eye));
    for _ in 0..4 {
        rig.gpu
            .sync_now(&world, &rig.device, &rig.queue)
            .expect("mirror fits the adapter");
    }
    rig.bake_around(&world, eye, 2, 64);
    for _ in 0..FRAMES {
        rig.frame.step(
            &rig.device,
            &rig.queue,
            &rig.gpu,
            &camera,
            &LightParams::headlamp(),
        );
    }
    let img = svet_render::debug::read_rgba8(&rig.device, &rig.queue, rig.frame.target());

    let hits: Vec<_> = (0..HEIGHT)
        .flat_map(|r| (0..WIDTH).map(move |c| (c, r)))
        .map(|(c, r)| first_material(&world, &camera, c, r))
        .collect();
    let mean = |px: &[usize], f: &dyn Fn(&[u8]) -> f64| {
        px.iter().map(|&i| f(&img[i * 4..i * 4 + 4])).sum::<f64>() / px.len().max(1) as f64
    };
    let glass_px = inner_pixels(&hits, glass);
    let lamp_px = inner_pixels(&hits, lamp);
    let (blue, red) = (
        mean(&glass_px, &|p| f64::from(p[2])),
        mean(&glass_px, &|p| f64::from(p[0])),
    );
    let luma = |p: &[u8]| {
        (0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2])) / 255.0
    };
    let lamp_luma = mean(&lamp_px, &luma);
    println!(
        "chapel.png: {} glass pixels, mean blue {blue:.1} red {red:.1}; {} lamp pixels, mean \
         luma {lamp_luma:.3}; {} light samples",
        glass_px.len(),
        lamp_px.len(),
        rig.gpu.light_samples()
    );
    assert_golden("chapel.png", &img);
    assert!(glass_px.len() >= 100, "{} glass pixels", glass_px.len());
    assert!(blue > red, "glass: blue {blue:.1}, red {red:.1}");
    assert!(lamp_px.len() >= 4, "{} lamp pixels", lamp_px.len());
    assert!(lamp_luma > 0.5, "lamps: mean luma {lamp_luma:.3}");
}
