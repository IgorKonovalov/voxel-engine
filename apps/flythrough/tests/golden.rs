//! Golden images of the cave on the software adapter (lavapipe). `VXE_BLESS=1` rewrites them.

mod support;

use flythrough::cave::ROCK;
use flythrough::cave_materials;
use support::*;
use vxe_render::gpu_world::{PAGE_UNITS, UNIT_BYTES};
use vxe_render::{
    Camera, Frame, GpuWorld, LightParams, LodParams, MirrorConfig, ShadowWalk, Window, readback,
    wgpu,
};
use vxe_world::*;

/// The scene from the spawn pose under each of `lights`, on one device.
fn render(world: &World, lights: &[LightParams]) -> Option<Vec<Vec<u8>>> {
    render_from(world, &scene_camera(), lights)
}

/// `world` from `camera` under each of `lights`, on one device.
fn render_from(world: &World, camera: &Camera, lights: &[LightParams]) -> Option<Vec<Vec<u8>>> {
    render_lit(world, camera, lights, true)
}

/// [`render_from`] with the spatial filter on or off: the lamp's shadow geometry is checked
/// unfiltered, since the filter spreads irradiance across a shadow's edge.
fn render_lit(
    world: &World,
    camera: &Camera,
    lights: &[LightParams],
    filter: bool,
) -> Option<Vec<Vec<u8>>> {
    let full = world.stats().full_bricks as u64;
    let (device, queue) = software_device((full + 4096) * 512)?;
    let mut gpu = GpuWorld::new(&device, world.extent());
    gpu.sync_now(world, &device, &queue)
        .expect("mirror fits the adapter");
    let mut frame = Frame::new(&device, WIDTH, HEIGHT);
    frame.set_resolution(&device, vxe_render::LightingResolution::Full);
    frame.set_lighting(vxe_render::Lighting {
        filter,
        ..vxe_render::Lighting::default()
    });
    Some(
        lights
            .iter()
            .map(|light| {
                frame.render(&device, &queue, &gpu, camera, light);
                readback::read_rgba8(&device, &queue, frame.target())
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
    assert_golden("headlamp.png", &images[0]);
    let images = render_lit(&carved_world(), &scene_camera(), &[lit, unshadowed], false).unwrap();
    let (shadowed, plain) = (&images[0], &images[1]);
    assert_eq!(
        darker_pixels(shadowed, plain),
        0,
        "pixels darker without shadows, unfiltered"
    );
    let f = fraction_differing(plain, shadowed, 8);
    println!(
        "shadows darken {:.2} % of pixels by more than 8/255",
        f * 100.0
    );
}

/// Bytes at which `a` and `b` differ.
fn differing_bytes(a: &[u8], b: &[u8]) -> usize {
    assert_eq!(a.len(), b.len());
    a.iter().zip(b).filter(|(x, y)| x != y).count()
}

/// The walk is independent of its skips: the `primary.png`, `headlamp.png` and `lod.png`
/// scenes from their golden poses render byte-identically with every skip on, with every
/// skip off (the walk over single voxels, which stays under `MAX_STEPS` on these windows of
/// 384 voxels) and with the mask tests alone off, on one device, and the image with every
/// skip on is the golden file's bytes, `lod.png` included.
#[test]
fn the_walk_renders_the_same_bytes_with_every_skip_on_or_off() {
    let Some((device, queue)) = software_device(64 << 20) else {
        return;
    };
    let bless = std::env::var("VXE_BLESS").as_deref() == Ok("1");
    let carved = carved_world();
    let lod = lod_scene();
    let scenes = [
        (
            "primary.png",
            &carved,
            LightParams::at_eye(),
            LodParams::default(),
        ),
        (
            "headlamp.png",
            &carved,
            LightParams::headlamp(),
            LodParams::default(),
        ),
        ("lod.png", &lod, LightParams::at_eye(), forced(1)),
    ];
    for (name, world, light, lod) in scenes {
        let variants = [
            ("every skip on", light),
            (
                "every skip off",
                LightParams {
                    skips: false,
                    ..light
                },
            ),
            (
                "the mask tests off",
                LightParams {
                    masks: false,
                    ..light
                },
            ),
        ];
        // The full walk: the coarse walk is defined on the mask blocks the skips read, so
        // turning them off changes what it treats as solid, which is not the skips' claim.
        let images = render_walk(
            &device,
            &queue,
            world,
            &scene_camera(),
            &variants.map(|(_, light)| light),
            lod,
            ShadowWalk::Full,
        );
        for (i, (label, _)) in variants.iter().enumerate().skip(1) {
            let differing = differing_bytes(&images[0], &images[i]);
            assert_eq!(
                differing, 0,
                "{name} with {label}: {differing} bytes differ from every skip on"
            );
        }
        if !bless {
            // The golden is drawn at the default walk.
            let image = &render_with(&device, &queue, world, &scene_camera(), &[light], lod)[0];
            let differing = differing_bytes(image, &read_png(&golden_path(name)).0);
            assert_eq!(
                differing, 0,
                "{name}: {differing} bytes differ from the golden"
            );
        }
    }
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
    let Some(images) = render_lit(&world, &camera, &[lit, unshadowed], false) else {
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

/// The cave's table plus a translucent red glass row (opacity 0.5).
fn glass_table() -> (MaterialTable, MaterialId) {
    let mut t = cave_materials();
    let glass = t.push(Material {
        albedo: [0.9, 0.1, 0.1],
        flags: MaterialFlags::TRANSLUCENT,
        opacity: 0.5,
        ..Material::named("glass")
    });
    (t, glass)
}

/// The translucent scene: air over chunks `[0, 2)^3`, the rock wall at x `[44, 46)` and a
/// glass slab at x `[36, 38)`, y and z `[56, 72)`, seen from `(8.5, 64.5, 64.5)` along +x
/// under the headlamp. `with_glass` false leaves the slab out.
fn translucent_scene(with_glass: bool) -> World {
    let (table, glass) = glass_table();
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [2, 2, 2]),
        &UniformSource(0),
        &table,
    );
    fill_voxels(&world, [44, 0, 0], [46, 128, 128], ROCK);
    if with_glass {
        fill_voxels(&world, [36, 56, 56], [38, 72, 72], glass);
    }
    world
}

/// `translucent.png`: the glass slab composited over the wall, shaded under the headlamp. The
/// slab tints the wall behind it red (the centre pixel's red channel leads its green) and
/// changes a twentieth of the image or more against the same scene without it (the slab
/// subtends about 23 % of the width and 41 % of the height); the file matches byte for byte.
#[test]
fn translucent_golden_composites_the_slab_over_the_wall() {
    let camera = Camera {
        position: [8.5, 64.5, 64.5].into(),
        yaw: 0.0,
        pitch: 0.0,
        vfov: 70f32.to_radians(),
    };
    let Some((device, queue)) = software_device(64 << 20) else {
        return;
    };
    let light = [LightParams::headlamp()];
    let lod = LodParams::default();
    let glass = render_with(
        &device,
        &queue,
        &translucent_scene(true),
        &camera,
        &light,
        lod,
    );
    let bare = render_with(
        &device,
        &queue,
        &translucent_scene(false),
        &camera,
        &light,
        lod,
    );
    assert_golden("translucent.png", &glass[0]);
    if std::env::var("VXE_BLESS").as_deref() != Ok("1") {
        let differing = differing_bytes(&glass[0], &read_png(&golden_path("translucent.png")).0);
        assert_eq!(
            differing, 0,
            "translucent.png: {differing} bytes differ from the golden"
        );
    }
    let centre = (HEIGHT / 2 * WIDTH + WIDTH / 2) as usize * 4;
    let p = &glass[0][centre..centre + 3];
    println!("centre pixel through the glass: {p:?}");
    assert!(p[0] > p[1] + 16, "centre pixel {p:?} is not tinted red");
    let f = fraction_differing(&glass[0], &bare[0], 8);
    println!("the slab changes {:.2} % of pixels", f * 100.0);
    assert!(f >= 0.05, "{:.2} %", f * 100.0);
}

/// `primary.png` and `headlamp.png` match byte for byte with the pool in one shard and split
/// over two to four: the mirror's layout changes no pixel. One scene, one device.
#[test]
fn goldens_match_bit_exactly_over_one_and_several_shards() {
    if std::env::var("VXE_BLESS").as_deref() == Ok("1") {
        return;
    }
    let world = carved_world();
    let pool = flythrough::level_pool_units(&world, false) * UNIT_BYTES;
    let Some((device, queue)) = software_device(pool) else {
        return;
    };
    let goldens = [
        ("primary.png", LightParams::at_eye()),
        ("headlamp.png", LightParams::headlamp()),
    ]
    .map(|(name, light)| (name, light, read_png(&golden_path(name)).0));
    let mut frame = Frame::new(&device, WIDTH, HEIGHT);
    frame.set_resolution(&device, vxe_render::LightingResolution::Full);
    // One shard first; its pages size the cap that spreads the next mirror over several.
    let mut pages = 0u32;
    for several in [false, true] {
        let max_shard_units = several.then(|| pages.div_ceil(3) * PAGE_UNITS);
        let mut gpu = GpuWorld::with_config(
            &device,
            MirrorConfig {
                max_shard_units,
                ..MirrorConfig::new(Window::Fixed(world.extent()))
            },
        );
        gpu.sync_now(&world, &device, &queue)
            .expect("mirror fits the adapter");
        let shards = gpu.shards_in_use();
        pages = gpu.pages_issued();
        println!("max_shard_units {max_shard_units:?}: {pages} pages over {shards} shards");
        if several {
            assert!((2..=4).contains(&shards), "{shards} shards");
        } else {
            assert_eq!(shards, 1);
        }
        for (name, light, golden) in &goldens {
            frame.render(&device, &queue, &gpu, &scene_camera(), light);
            let img = readback::read_rgba8(&device, &queue, frame.target());
            let differing = img.iter().zip(golden).filter(|(a, b)| a != b).count();
            assert_eq!(
                differing, 0,
                "{name} over {shards} shards: {differing} bytes differ"
            );
        }
    }
}

/// Makes every chunk of `world` for which `lod` holds LOD-only: out of the store, into the
/// world's mips.
fn make_lod(world: &World, lod: impl Fn(ChunkCoord) -> bool) {
    for c in world.store().coords() {
        if lod(c) {
            let chunk = world.store().remove(c).unwrap();
            let mips = ChunkMips::build(&chunk.read().unwrap());
            world.insert_lod(c, std::sync::Arc::new(mips));
        }
    }
}

/// Renders `world` from `camera` under each of `lights` with `lod`, on `device`.
fn render_with(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    world: &World,
    camera: &Camera,
    lights: &[LightParams],
    lod: LodParams,
) -> Vec<Vec<u8>> {
    render_walk(
        device,
        queue,
        world,
        camera,
        lights,
        lod,
        ShadowWalk::default(),
    )
}

/// [`render_with`] with shadow rays taking `walk`.
fn render_walk(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    world: &World,
    camera: &Camera,
    lights: &[LightParams],
    lod: LodParams,
    walk: ShadowWalk,
) -> Vec<Vec<u8>> {
    let mut gpu = GpuWorld::new(device, world.extent());
    gpu.sync_now(world, device, queue)
        .expect("mirror fits the adapter");
    let mut frame = Frame::new(device, WIDTH, HEIGHT);
    frame.set_resolution(device, vxe_render::LightingResolution::Full);
    frame.set_lighting(vxe_render::Lighting {
        shadow_walk: walk,
        ..frame.lighting()
    });
    frame.set_lod(lod);
    lights
        .iter()
        .map(|light| {
            frame.render(device, queue, &gpu, camera, light);
            readback::read_rgba8(device, queue, frame.target())
        })
        .collect()
}

fn forced(level: u32) -> LodParams {
    LodParams {
        r_full: 1,
        forced_level: Some(level),
    }
}

/// Uniform chunks, solid material 3 below z = 0 and air above, render bit-identically when
/// every chunk but the camera's is LOD-only at any forced level: a uniform chunk's mips are
/// hit on the same entry face as its voxels.
#[test]
fn uniform_chunks_render_identically_from_their_mips() {
    let source = UniformBelow;
    let extent = ChunkExtent::new([-3, -3, -2], [3, 3, 2]);
    let full = World::generate(extent, &source, &cave_materials());
    let Some((device, queue)) = software_device(4096 * 512) else {
        return;
    };
    let camera = Camera {
        position: [32.5, 32.5, 20.5].into(),
        yaw: 0.6,
        pitch: -0.5,
        vfov: 70f32.to_radians(),
    };
    let cam_chunk = ChunkCoord([0, 0, 0]);
    let lights = [LightParams::at_eye(), LightParams::headlamp()];
    let want = render_with(&device, &queue, &full, &camera, &lights, forced(1));
    let lit = want[0].chunks(4).filter(|p| p[..3] != [0, 0, 0]).count();
    println!("{lit} lit pixels of {}", WIDTH * HEIGHT);
    assert!(lit > (WIDTH * HEIGHT / 4) as usize, "{lit} lit pixels");

    let lod = World::generate(extent, &source, &cave_materials());
    make_lod(&lod, |c| c != cam_chunk);
    assert_eq!(lod.store().coords(), [cam_chunk]);
    for level in 1..=3 {
        let got = render_with(&device, &queue, &lod, &camera, &lights, forced(level));
        for (i, (g, w)) in got.iter().zip(&want).enumerate() {
            let differing = g.iter().zip(w).filter(|(a, b)| a != b).count();
            assert_eq!(
                differing, 0,
                "level {level}, light {i}: {differing} bytes differ"
            );
        }
    }
}

/// Rock below z = 0, air above.
struct UniformBelow;

impl WorldSource for UniformBelow {
    fn chunk(&self, coord: ChunkCoord, out: &mut Chunk) {
        let m = if coord.0[2] < 0 { ROCK } else { AIR };
        for slot in 0..CHUNK_BRICKS {
            out.set_brick(slot, Brick::Uniform(m));
        }
    }
}

/// The carved golden scene with every chunk farther than squared distance 1 from the
/// camera's chunk LOD-only.
fn lod_scene() -> World {
    let world = carved_world();
    let cam = VoxelCoord(scene_source().spawn().0).chunk();
    make_lod(&world, |c| {
        (0..3).map(|k| (c.0[k] - cam.0[k]).pow(2)).sum::<i32>() > 1
    });
    world
}

/// `lod.png`: the golden scene from the spawn pose with chunks past squared distance 1 of the
/// camera's chunk walked at forced level 1. The LOD walk visibly changes the image.
#[test]
fn lod_golden_walks_the_ring_at_level_1() {
    let world = lod_scene();
    let Some((device, queue)) = software_device(64 << 20) else {
        return;
    };
    let images = render_with(
        &device,
        &queue,
        &world,
        &scene_camera(),
        &[LightParams::at_eye()],
        forced(1),
    );
    assert_golden("lod.png", &images[0]);
    let (primary, _, _) = read_png(&golden_path("primary.png"));
    let f = fraction_differing(&images[0], &primary, 8);
    println!("lod differs from primary at {:.2} % of pixels", f * 100.0);
    assert!(f >= 0.01, "{:.2} %", f * 100.0);
}

/// Without a forced level the frame picks level 1 within `(2 r_full)^2` of the camera's
/// chunk, level 2 within `(4 r_full)^2`, else 3. In the golden scene every chunk lies within
/// squared distance 27 of the camera's: with r_full = 3 the image equals forced level 1, with
/// r_full = 0 forced level 3. A chunk at squared distance 9 with r_full = 1 is walked at
/// level 2.
#[test]
fn the_lod_level_follows_the_distance_to_the_camera_chunk() {
    let world = lod_scene();
    let Some((device, queue)) = software_device(64 << 20) else {
        return;
    };
    let light = [LightParams::at_eye()];
    let auto = |r_full| LodParams {
        r_full,
        forced_level: None,
    };
    let differing = |a: &[u8], b: &[u8]| a.iter().zip(b).filter(|(x, y)| x != y).count();
    {
        let render = |lod| render_with(&device, &queue, &world, &scene_camera(), &light, lod);
        let (l1, l3) = (render(forced(1)), render(forced(3)));
        assert!(differing(&l1[0], &l3[0]) > 0, "levels 1 and 3 render alike");
        assert_eq!(differing(&render(auto(3))[0], &l1[0]), 0);
        assert_eq!(differing(&render(auto(0))[0], &l3[0]), 0);
    }

    // Columns one voxel wide every 16 voxels in chunk [3, 0, 0], air elsewhere; seen from the
    // middle of chunk [0, 0, 0] looking along +x. Squared distance 9: level 2 at r_full = 1.
    let columns = World::generate(
        ChunkExtent::new([0, -1, -1], [4, 2, 2]),
        &Columns,
        &cave_materials(),
    );
    make_lod(&columns, |c| c != ChunkCoord([0, 0, 0]));
    let camera = Camera {
        position: [32.5, 32.5, 32.5].into(),
        yaw: 0.0,
        pitch: 0.0,
        vfov: 70f32.to_radians(),
    };
    let render = |lod| render_with(&device, &queue, &columns, &camera, &light, lod);
    let levels: Vec<Vec<u8>> = (1..=3).map(|l| render(forced(l)).remove(0)).collect();
    assert!(differing(&levels[0], &levels[1]) > 0 && differing(&levels[1], &levels[2]) > 0);
    let picked = render(auto(1)).remove(0);
    assert_eq!(
        differing(&picked, &levels[1]),
        0,
        "r_full = 1 walks d2 = 9 at level 2"
    );
}

/// Rock columns along z at x and y multiples of 16 in chunks with x = 3, air elsewhere.
struct Columns;

impl WorldSource for Columns {
    fn chunk(&self, coord: ChunkCoord, out: &mut Chunk) {
        let base = coord.min_voxel().0;
        for slot in 0..CHUNK_BRICKS {
            let b = brick_of_slot(slot);
            out.set_brick(
                slot,
                Brick::from_fn(|l| {
                    let v = [0, 1, 2].map(|k| base[k] + b[k] * BRICK_EDGE + l[k]);
                    if coord.0[0] == 3 && v[0] % 16 == 0 && v[1].rem_euclid(16) == 0 {
                        ROCK
                    } else {
                        AIR
                    }
                }),
            );
        }
    }
}

/// The level the fly-through generates with `--level --seed 1`.
fn seed_1_level() -> World {
    let source = flythrough::cave::CaveSource::new(
        1,
        ChunkExtent::new([0, 0, 0], flythrough::cave::LEVEL_CHUNKS),
    );
    World::generate(source.extent, &source, &cave_materials())
}

/// 40 m (400 voxels) outside the level's -x face, level with its centre, looking +x.
fn outside_level_camera() -> Camera {
    let c = flythrough::cave::LEVEL_CHUNKS.map(|n| (n * 64) as f32 / 2.0 + 0.5);
    camera_looking_x([-400.0, c[1], c[2]])
}

/// The cutaway look: face normals, no detail, shaded.
fn cutaway_look() -> vxe_render::LookParams {
    vxe_render::LookParams {
        cutaway: true,
        ..vxe_render::LookParams::default()
    }
}

/// The level from outside its -x face in the cutaway view: the far walls of the caverns
/// behind the hull under the hemisphere light, against the flat sky where rock has no cavern
/// behind it. The sky's colour bin covers at least 20 % of the pixels, and at least 20 % are
/// lit rock (outside that bin and not black).
#[test]
fn cutaway_level_golden_shows_sky_and_lit_rock() {
    let world = seed_1_level();
    let pool = flythrough::level_pool_units(&world, false) * UNIT_BYTES;
    let Some((device, queue)) = software_device(pool) else {
        return;
    };
    let cfg = MirrorConfig {
        budget_bytes: 256 << 20,
        ..MirrorConfig::new(Window::Fixed(world.extent()))
    };
    let mut rig = LookRig::on(device, queue, &world, Some(cfg));
    let img = rig.render(
        &outside_level_camera(),
        &LightParams::headlamp(),
        cutaway_look(),
    );
    assert_golden("cutaway_level.png", &img);

    // The sky is the colour of a miss: read it off the hit mask.
    let mask = rig.render(
        &outside_level_camera(),
        &LightParams::headlamp(),
        vxe_render::LookParams {
            view: vxe_render::ViewMode::HitMask,
            ..cutaway_look()
        },
    );
    let bin = |p: &[u8]| (p[0] >> 5, p[1] >> 5, p[2] >> 5);
    let miss = img
        .chunks(4)
        .zip(mask.chunks(4))
        .find(|(_, m)| m[0] == 0)
        .map(|(p, _)| bin(p))
        .expect("a pixel misses");
    let n = (img.len() / 4) as f64;
    let sky = img.chunks(4).filter(|p| bin(p) == miss).count() as f64 / n;
    let lit = img
        .chunks(4)
        .filter(|p| bin(p) != miss && p[..3].iter().any(|&c| c >= 32))
        .count() as f64
        / n;
    let misses = mask.chunks(4).filter(|m| m[0] == 0).count() as f64 / n;
    println!(
        "cutaway_level.png: sky bin {miss:?} {:.2} %, lit rock {:.2} %, misses {:.2} %, bins covering 5 %: {}",
        sky * 100.0,
        lit * 100.0,
        misses * 100.0,
        bins_covering(&img, 0.05)
    );
    assert!(sky >= 0.20, "sky {:.2} %", sky * 100.0);
    assert!(lit >= 0.20, "lit rock {:.2} %", lit * 100.0);
}

/// The steps view from the outside camera at 1920 x 1080 on the high-performance adapter:
/// mean and p99 steps per ray (red channel, clamped at 255). A reading, not a check: run with
/// `--ignored` on the machine being measured.
#[test]
#[ignore = "a diagnostic reading on the high-performance adapter; run by hand with --ignored"]
fn cutaway_steps_from_outside_the_level_at_1080p() {
    let world = seed_1_level();
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .expect("an adapter");
    let pool_bytes = flythrough::level_pool_units(&world, false) * UNIT_BYTES;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: vxe_render::limits_for(&adapter, pool_bytes),
        ..Default::default()
    }))
    .expect("a device");
    let cfg = MirrorConfig {
        budget_bytes: 256 << 20,
        ..MirrorConfig::new(Window::Fixed(world.extent()))
    };
    let mut gpu = GpuWorld::with_config(&device, cfg);
    gpu.sync_now(&world, &device, &queue)
        .expect("mirror fits the adapter");
    let (w, h) = (1920, 1080);
    let mut frame = Frame::new(&device, w, h);
    frame.set_resolution(&device, vxe_render::LightingResolution::Full);
    for cutaway in [false, true] {
        frame.set_look(vxe_render::LookParams {
            view: vxe_render::ViewMode::Steps,
            cutaway,
            ..cutaway_look()
        });
        frame.render(
            &device,
            &queue,
            &gpu,
            &outside_level_camera(),
            &LightParams::headlamp(),
        );
        let px = readback::read_rgba8(&device, &queue, frame.target());
        let hist = readback::StepHistogram::from_rgba8(&px, w as usize);
        let saturated = px.chunks(4).filter(|p| p[0] == 255).count();
        println!(
            "{} at {w}x{h}, cutaway {cutaway}: steps per ray mean {:.2} p99 {}, rays at the 255 clamp {saturated} of {}",
            adapter.get_info().name,
            hist.mean_steps(),
            hist.p99_steps(),
            w * h
        );
    }
}

/// The hit buffer's hitmask and normals views render, byte for byte, what the single-pass
/// march rendered at ae58f0c on the three opaque scenes (the carved golden scene, its LOD
/// ring at level 1, the slab-and-wall occluder scene): the `split_*.png` files, never
/// re-blessed.
#[test]
fn hit_buffer_views_match_the_single_pass_march() {
    let carved = carved_world();
    let lod = lod_scene();
    let occluder = occluder_world();
    let scenes: [(&str, &World, Camera, Option<LodParams>); 3] = [
        ("carve", &carved, scene_camera(), None),
        ("lod", &lod, scene_camera(), Some(lod_forced_1())),
        ("occluder", &occluder, occluder_camera(), None),
    ];
    for (scene, world, camera, lod) in scenes {
        let Some(mut rig) = LookRig::new(world, None) else {
            return;
        };
        if let Some(lod) = lod {
            rig.frame.set_lod(lod);
        }
        for view in [vxe_render::ViewMode::HitMask, vxe_render::ViewMode::Normals] {
            let look = vxe_render::LookParams {
                view,
                ..vxe_render::LookParams::default()
            };
            let img = rig.render(&camera, &LightParams::headlamp(), look);
            let name = format!("split_{}_{scene}.png", view.name());
            let differing = differing_bytes(&img, &read_png(&golden_path(&name)).0);
            println!("{name}: {differing} bytes differ");
            assert_eq!(differing, 0, "{name}: {differing} bytes differ");
        }
    }
}

/// With exposure 1, the identity tone map and the filter off the composite is the march's
/// former display
/// encoding, so the carve golden matches the single-pass march's (`primary_pre_split.png`,
/// ae58f0c's `primary.png`) within 2/255 in every pixel: the split moved no light.
#[test]
fn the_identity_tone_map_reproduces_the_single_pass_carve() {
    let Some(mut rig) = LookRig::new(&carved_world(), None) else {
        return;
    };
    rig.frame.set_lighting(vxe_render::Lighting {
        exposure: vxe_render::Exposure::Manual(1.0),
        tone_map: vxe_render::ToneMap::Identity,
        filter: false,
        ..vxe_render::Lighting::default()
    });
    let img = rig.render(
        &scene_camera(),
        &LightParams::at_eye(),
        vxe_render::LookParams::default(),
    );
    let (old, _, _) = read_png(&golden_path("primary_pre_split.png"));
    let worst = img
        .chunks(4)
        .zip(old.chunks(4))
        .map(|(p, q)| (0..3).map(|k| p[k].abs_diff(q[k])).max().unwrap_or(0))
        .max()
        .unwrap_or(0);
    println!("largest channel difference from the single-pass carve: {worst}/255");
    assert!(worst <= 2, "{worst}/255");
}

/// An asymmetric view with tangents (left `t`, right 0, up and down `t_y`) over 160 x 180
/// pixels renders the left half of the symmetric 320 x 180 view, `t = t_y 320 / 180`, with no
/// pixel differing by more than 2/255.
#[test]
fn an_asymmetric_view_renders_the_left_half_of_the_symmetric_one() {
    let world = carved_world();
    let full = world.stats().full_bricks as u64;
    let Some((device, queue)) = software_device((full + 4096) * 512) else {
        return;
    };
    let mut gpu = GpuWorld::new(&device, world.extent());
    gpu.sync_now(&world, &device, &queue)
        .expect("mirror fits the adapter");
    let camera = scene_camera();
    let light = LightParams::headlamp();
    // Unfiltered: the filter's neighbourhoods differ along the half view's right edge.
    let unfiltered = vxe_render::Lighting {
        filter: false,
        ..vxe_render::Lighting::default()
    };
    let mut whole = Frame::new(&device, WIDTH, HEIGHT);
    whole.set_resolution(&device, vxe_render::LightingResolution::Full);
    whole.set_lighting(unfiltered);
    whole.render(&device, &queue, &gpu, &camera, &light);
    let whole_px = readback::read_rgba8(&device, &queue, whole.target());

    let half_w = WIDTH / 2;
    let mut half = Frame::new(&device, half_w, HEIGHT);
    half.set_resolution(&device, vxe_render::LightingResolution::Full);
    half.set_lighting(unfiltered);
    let ty = (camera.vfov * 0.5).tan();
    let t = ty * (WIDTH as f32 / HEIGHT as f32);
    let aspect = WIDTH as f32 / HEIGHT as f32;
    let view =
        vxe_render::View::with_tangents(&camera, [t, 0.0, ty, ty], aspect, [0, 0, half_w, HEIGHT]);
    let params = half.params_for(&gpu, &[view], &camera, &light);
    half.submit(&device, &queue, &gpu, &params);
    let half_px = readback::read_rgba8(&device, &queue, half.target());

    let mut worst = 0;
    for row in 0..HEIGHT as usize {
        for col in 0..half_w as usize {
            let a = &half_px[(row * half_w as usize + col) * 4..][..3];
            let b = &whole_px[(row * WIDTH as usize + col) * 4..][..3];
            worst = worst.max((0..3).map(|k| a[k].abs_diff(b[k])).max().unwrap_or(0));
        }
    }
    let lit = half_px.chunks(4).filter(|p| p[..3] != [0, 0, 0]).count();
    println!("asymmetric half: largest channel difference {worst}/255, {lit} lit pixels");
    assert!(lit > (half_w * HEIGHT / 4) as usize, "{lit} lit pixels");
    assert!(worst <= 2, "{worst}/255");
}

/// The golden scene with its carve under `table`.
fn carved_world_under(table: &MaterialTable) -> World {
    let source = scene_source();
    let world = World::generate(source.extent, &source, table);
    let r = world.apply(&scene_carve(&world)).unwrap();
    assert!(!r.changed_from.is_empty(), "the carve removes rock");
    world
}

/// A pose in the golden scene facing an ore vein: voxel (264, 120, 376), 15 degrees of yaw
/// (found by casting rays from a 16-voxel grid of air voxels for the most ore within 60
/// voxels and a 70-degree cone).
fn vein_camera() -> Camera {
    Camera {
        position: [264.5, 120.5, 376.5].into(),
        yaw: 15f32.to_radians(),
        pitch: 0.0,
        vfov: 70f32.to_radians(),
    }
}

/// Frames the dark cave accumulates before its golden.
const SETTLE_FRAMES: u32 = 32;

/// The carved scene's rig under `table`, the lamp off, after [`SETTLE_FRAMES`] frames from
/// [`vein_camera`] with history; `None` without a software adapter.
fn settled_dark_cave(table: &MaterialTable) -> Option<(World, LookRig)> {
    settled_dark_cave_at(table, vxe_render::LightingResolution::Full)
}

/// [`settled_dark_cave`] with the lighting passes at `resolution`.
fn settled_dark_cave_at(
    table: &MaterialTable,
    resolution: vxe_render::LightingResolution,
) -> Option<(World, LookRig)> {
    let world = carved_world_under(table);
    let mut rig = LookRig::new(&world, None)?;
    rig.frame.set_resolution(&rig.device, resolution);
    rig.frame.set_lighting(vxe_render::Lighting {
        lamp: false,
        ..vxe_render::Lighting::default()
    });
    // The light radius around the camera, its ranges written over the syncs the budget needs.
    rig.gpu.set_focus(VoxelCoord(
        vein_camera().position.to_array().map(|c| c.floor() as i32),
    ));
    for _ in 0..4 {
        rig.gpu
            .sync_now(&world, &rig.device, &rig.queue)
            .expect("mirror fits the adapter");
    }
    for _ in 0..SETTLE_FRAMES {
        rig.frame.step(
            &rig.device,
            &rig.queue,
            &rig.gpu,
            &vein_camera(),
            &LightParams::headlamp(),
        );
    }
    Some((world, rig))
}

/// The pixel rectangle (`col0, row0, col1, row1`) the voxels `min ..= max` dilated by one
/// voxel project to from `camera` in a `WIDTH` x `HEIGHT` target.
fn box_projection(camera: &Camera, (min, max): (VoxelCoord, VoxelCoord)) -> [f32; 4] {
    let (fwd, right, up) = camera.basis();
    let tan = (camera.vfov * 0.5).tan();
    let aspect = WIDTH as f32 / HEIGHT as f32;
    let mut r = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for c in 0..8 {
        let corner = vxe_render::glam::Vec3::new(
            if c & 1 == 0 {
                min.0[0] - 1
            } else {
                max.0[0] + 2
            } as f32,
            if c & 2 == 0 {
                min.0[1] - 1
            } else {
                max.0[1] + 2
            } as f32,
            if c & 4 == 0 {
                min.0[2] - 1
            } else {
                max.0[2] + 2
            } as f32,
        );
        let d = corner - camera.position;
        let z = d.dot(fwd).max(1e-3);
        let col = (d.dot(right) / z / (tan * aspect) + 1.0) * 0.5 * WIDTH as f32;
        let row = (1.0 - d.dot(up) / z / tan) * 0.5 * HEIGHT as f32;
        r = [r[0].min(col), r[1].min(row), r[2].max(col), r[3].max(row)];
    }
    r
}

/// What the frame right after a carve shows: hit pixels in the dilated box and those not at
/// age 0; pixels whose pre-carve hit voxel (non-emissive) the carve removed and those within
/// 2/255 of their pre-carve value; pixels farther than 16 pixels from the box's projection
/// and those changed by more than 8/255.
struct CarveReading {
    in_box: usize,
    in_box_aged: usize,
    removed: usize,
    removed_stale: usize,
    far: usize,
    far_over: usize,
}

/// The unit normal a hit record's snorm16 octahedral pair encodes.
fn octa16_decode(u: u16, v: u16) -> [f32; 3] {
    let p = [u as i16 as f32 / 32767.0, v as i16 as f32 / 32767.0];
    let mut n = [p[0], p[1], 1.0 - p[0].abs() - p[1].abs()];
    if n[2] < 0.0 {
        let s = |x: f32| if x >= 0.0 { 1.0 } else { -1.0 };
        n = [
            (1.0 - n[1].abs()) * s(n[0]),
            (1.0 - n[0].abs()) * s(n[1]),
            n[2],
        ];
    }
    let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    n.map(|c| c / l)
}

/// Applies `carve` to `world`, syncs `rig`, tells the frame its box (or, with `flush`, more
/// boxes than a frame takes, flushing all history), steps one frame and reads it against the
/// pre-carve image `before`, hit positions `pos` and records `info`.
#[allow(
    clippy::too_many_arguments,
    reason = "the carve, the rig and the pre-carve frame"
)]
fn carve_reading(
    world: &World,
    rig: &mut LookRig,
    carve: &EditOp,
    before: &[u8],
    pos: &[[f32; 4]],
    info: &[[u16; 4]],
    table: &MaterialTable,
    flush: bool,
) -> CarveReading {
    let r = world.apply(carve).expect("the carve applies");
    let aabb = r.aabb.expect("the carve removes rock");
    rig.gpu.sync_now(world, &rig.device, &rig.queue).unwrap();
    let boxes = if flush { vxe_render::MAX_EDITS + 1 } else { 1 };
    for _ in 0..boxes {
        rig.frame.invalidate(aabb.0.0, aabb.1.0);
    }
    let camera = vein_camera();
    rig.frame.step(
        &rig.device,
        &rig.queue,
        &rig.gpu,
        &camera,
        &LightParams::headlamp(),
    );
    let after = readback::read_rgba8(&rig.device, &rig.queue, rig.frame.target());
    let now = readback::read_rgba32f(&rig.device, &rig.queue, rig.frame.hit_positions());
    let age = readback::read_rgba16f(&rig.device, &rig.queue, rig.frame.moments());
    let rect = box_projection(&camera, aabb);
    let mut out = CarveReading {
        in_box: 0,
        in_box_aged: 0,
        removed: 0,
        removed_stale: 0,
        far: 0,
        far_over: 0,
    };
    let in_box = |p: [f32; 4]| {
        p[3] < 1e30
            && (0..3).all(|k| p[k] >= (aabb.0.0[k] - 1) as f32 && p[k] <= (aabb.1.0[k] + 2) as f32)
    };
    for i in 0..now.len() {
        let diff = (0..3)
            .map(|k| after[4 * i + k].abs_diff(before[4 * i + k]))
            .max()
            .unwrap_or(0);
        if in_box(now[i]) {
            out.in_box += 1;
            out.in_box_aged += usize::from(age[i][2] != 0.0);
        }
        let emits = table
            .get(info[i][2])
            .is_some_and(|m| m.emission.iter().any(|&e| e > 0.0));
        if info[i][3] & 1 == 1 && !emits {
            let n = octa16_decode(info[i][0], info[i][1]);
            let v = VoxelCoord([0, 1, 2].map(|k| (pos[i][k] - 0.01 * n[k]).floor() as i32));
            if world.material_at(v) == Some(AIR) {
                out.removed += 1;
                out.removed_stale += usize::from(diff <= 2);
            }
        }
        let (col, row) = (
            (i as u32 % WIDTH) as f32 + 0.5,
            (i as u32 / WIDTH) as f32 + 0.5,
        );
        let dx = (rect[0] - col).max(col - rect[2]).max(0.0);
        let dy = (rect[1] - row).max(row - rect[3]).max(0.0);
        if dx.max(dy) > 16.0 {
            out.far += 1;
            out.far_over += usize::from(diff > 8);
        }
    }
    out
}

/// `dark_cave.png`: the carved scene from a pose facing an ore vein, the lamp off, lit only by
/// the ore, after 32 accumulated frames. With the ore's emission off the same view is black
/// in every pixel. On the frame right after a radius-6 carve into the wall the vein lights (its
/// brightest rock pixel 15 to 25 voxels away), every hit pixel in the carve's dilated `aabb`
/// reads age 0 (the test also prints how many pixels whose non-emissive pre-carve hit voxel
/// the carve removed stay within 2/255 of their pre-carve value), and at most 0.1 % of the pixels farther than
/// 16 pixels from the box's projection change by more than 8/255; the same carve with all
/// history flushed exceeds that bar.
#[test]
fn dark_cave_golden_is_lit_by_its_ore_alone() {
    let Some((world, mut rig)) =
        settled_dark_cave(&flythrough::with_emissive_ore(&cave_materials()))
    else {
        return;
    };
    let before = readback::read_rgba8(&rig.device, &rig.queue, rig.frame.target());
    assert_golden("dark_cave.png", &before);
    let samples = rig.gpu.light_samples();
    let bright = before
        .chunks(4)
        .filter(|p| p[..3].iter().any(|&c| c >= 16))
        .count();
    println!(
        "dark_cave.png: {samples} light samples, {bright} of {} pixels at 16/255 or more, bins covering 1 %: {}",
        WIDTH * HEIGHT,
        bins_covering(&before, 0.01)
    );
    assert!(samples > 0);
    assert!(
        bright > (WIDTH * HEIGHT / 20) as usize,
        "{bright} lit pixels"
    );

    // The carve at the brightest rock pixel 15 to 25 voxels away, and the frame right after
    // it; the control makes the same carve on a second settled rig with every pixel's history
    // flushed.
    let pos = readback::read_rgba32f(&rig.device, &rig.queue, rig.frame.hit_positions());
    let info = readback::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records());
    let target = (0..pos.len())
        .filter(|&i| {
            // The radius-6 sphere stays inside the scene's 384-voxel extent.
            info[i][3] & 1 == 1
                && info[i][2] == ROCK
                && (15.0..25.0).contains(&pos[i][3])
                && pos[i][..3].iter().all(|c| (7.0..377.0).contains(c))
        })
        .max_by_key(|&i| u32::from(before[4 * i]) + u32::from(before[4 * i + 1]))
        .expect("a lit rock pixel");
    let at = pos[target];
    println!(
        "carve at pixel ({}, {}), {:?}, t {:.1}",
        target as u32 % WIDTH,
        target as u32 / WIDTH,
        &at[..3],
        at[3]
    );
    let carve = EditOp::sphere(
        Fixed3([0, 1, 2].map(|k| (at[k] * FIXED_ONE as f32) as i64)),
        6 * FIXED_ONE,
        Mode::Carve,
    );
    let table = flythrough::with_emissive_ore(&cave_materials());
    let reading = carve_reading(
        &world, &mut rig, &carve, &before, &pos, &info, &table, false,
    );
    let (cworld, mut control) = settled_dark_cave(&table).unwrap();
    let control = carve_reading(
        &cworld,
        &mut control,
        &carve,
        &before,
        &pos,
        &info,
        &table,
        true,
    );
    println!(
        "carve: {} hit pixels in the dilated box, {} not at age 0; {} pixels whose non-emissive pre-carve voxel was carved, {} within 2/255 of before; far: {} of {} over 8/255 (control, history flushed: {} of {})",
        reading.in_box,
        reading.in_box_aged,
        reading.removed,
        reading.removed_stale,
        reading.far_over,
        reading.far,
        control.far_over,
        control.far
    );
    assert!(reading.in_box > 0 && reading.removed > 0);
    assert_eq!(reading.in_box_aged, 0, "pixels in the box kept history");
    // `removed_stale` is a reading: rock revealed behind the removed rock, most of it as dark,
    // reads within 2/255 of it in about a fifth of those pixels (Plan 0024's log).
    let bar = |r: &CarveReading| r.far_over * 1000 <= r.far;
    assert!(bar(&reading), "the carve disturbs pixels far from it");
    assert!(
        !bar(&control),
        "the flushed control meets the far bar, which then bites on nothing"
    );

    let (_, off) = settled_dark_cave(&cave_materials()).unwrap();
    assert_eq!(off.gpu.light_samples(), 0);
    let off = readback::read_rgba8(&off.device, &off.queue, off.frame.target());
    let lit_off = off.chunks(4).filter(|p| p[..3] != [0, 0, 0]).count();
    assert_eq!(
        lit_off, 0,
        "{lit_off} pixels are not black with the ore dark"
    );
}

/// The slab scene of `headlamp_slab_casts_the_computed_shadow`, with a one-voxel hole through
/// the slab along x at `(y, z)` when given.
fn slab_world(hole: Option<[i64; 2]>) -> World {
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [2, 2, 2]),
        &UniformSource(0),
        &cave_materials(),
    );
    fill_voxels(&world, [44, 0, 0], [46, 128, 128], ROCK);
    fill_voxels(&world, [28, 56, 56], [30, 68, 68], ROCK);
    if let Some([y, z]) = hole {
        let op = EditOp::box_(
            Fixed3([28 * FIXED_ONE, y * FIXED_ONE, z * FIXED_ONE]),
            Fixed3([30 * FIXED_ONE, (y + 1) * FIXED_ONE, (z + 1) * FIXED_ONE]),
            Mode::Carve,
        );
        world.apply(&op).expect("the slab is resident");
    }
    world
}

/// The slab scene's renders under the headlamp in `walk`, shadows on and off.
fn slab_renders(world: &World, walk: vxe_render::ShadowWalk) -> Option<[Vec<u8>; 2]> {
    let mut rig = LookRig::new(world, None)?;
    rig.frame.set_lighting(vxe_render::Lighting {
        shadow_walk: walk,
        ..vxe_render::Lighting::default()
    });
    let camera = occluder_camera();
    let lit = LightParams::headlamp();
    let look = vxe_render::LookParams::default();
    let shadowed = rig.render(&camera, &lit, look);
    let plain = rig.render(
        &camera,
        &LightParams {
            shadows: false,
            ..lit
        },
        look,
    );
    Some([shadowed, plain])
}

/// Pixels darker than the unshadowed render by more than 8/255 in some channel.
fn darkened(shadowed: &[u8], plain: &[u8]) -> usize {
    shadowed
        .chunks(4)
        .zip(plain.chunks(4))
        .filter(|(s, p)| (0..3).any(|k| p[k] > s[k].saturating_add(8)))
        .count()
}

/// The coarse shadow walk treats any 2^3 block with a set bit as solid: through a one-voxel
/// hole along x in the slab's edge row (y 67, z 62) the full walk lights wall pixels the
/// hole-less slab shadows (the same surface seen in both, so the unshadowed renders agree
/// there), and the coarse walk lights none of them; on the slab without a hole the coarse
/// walk darkens at least as many pixels as the full walk. An interior hole's light lands on
/// wall the slab hides from the camera beside the lamp.
#[test]
fn the_coarse_shadow_walk_closes_a_one_voxel_hole() {
    use vxe_render::ShadowWalk::{Coarse, Full};
    let Some([full_solid, plain]) = slab_renders(&slab_world(None), Full) else {
        return;
    };
    let [coarse_solid, _] = slab_renders(&slab_world(None), Coarse).unwrap();
    let (dark_full, dark_coarse) = (
        darkened(&full_solid, &plain),
        darkened(&coarse_solid, &plain),
    );
    println!("slab: full walk darkens {dark_full} pixels, coarse walk {dark_coarse}");
    assert!(dark_full > 0);
    assert!(dark_coarse >= dark_full, "{dark_coarse} < {dark_full}");

    let hole = slab_world(Some([67, 62]));
    let [full_hole, plain_hole] = slab_renders(&hole, Full).unwrap();
    let [coarse_hole, _] = slab_renders(&hole, Coarse).unwrap();
    // Pixels the hole lets the lamp reach in the full walk.
    let through: Vec<usize> = (0..full_hole.len() / 4)
        .filter(|&i| {
            plain_hole[4 * i..4 * i + 3] == plain[4 * i..4 * i + 3]
                && (0..3).any(|k| full_hole[4 * i + k] > full_solid[4 * i + k].saturating_add(8))
        })
        .collect();
    let coarse_lit = through
        .iter()
        .filter(|&&i| {
            (0..3).any(|k| coarse_hole[4 * i + k] > full_solid[4 * i + k].saturating_add(2))
        })
        .count();
    println!(
        "hole: the full walk lights {} pixels through it, the coarse walk {coarse_lit} of them",
        through.len()
    );
    assert!(
        !through.is_empty(),
        "the full walk lights no pixel through the hole"
    );
    assert_eq!(coarse_lit, 0);
}

/// `dark_cave_half.png`: the dark cave with the lighting passes at half resolution, after 32
/// accumulated frames; its mean absolute difference from `dark_cave.png` is under 6/255 in
/// every channel.
#[test]
fn half_resolution_dark_cave_golden_stays_near_the_full_one() {
    let table = flythrough::with_emissive_ore(&cave_materials());
    let Some((_, rig)) = settled_dark_cave_at(&table, vxe_render::LightingResolution::Half) else {
        return;
    };
    let half = readback::read_rgba8(&rig.device, &rig.queue, rig.frame.target());
    assert_golden("dark_cave_half.png", &half);
    let (full, _, _) = read_png(&golden_path("dark_cave.png"));
    let n = (WIDTH * HEIGHT) as f64;
    let mad: Vec<f64> = (0..3)
        .map(|k| {
            half.chunks(4)
                .zip(full.chunks(4))
                .map(|(a, b)| f64::from(a[k].abs_diff(b[k])))
                .sum::<f64>()
                / n
        })
        .collect();
    println!(
        "dark_cave_half.png: mean absolute difference from dark_cave.png per channel {mad:.3?} (of 255)"
    );
    assert!(mad.iter().all(|&d| d < 6.0), "{mad:?}");
}

/// The cavity's table with row 6 an ore vein emitting eight times the fly-through's ore.
fn vein_cavity_table() -> (MaterialTable, MaterialId) {
    let mut t = look_table();
    let vein = t.push(Material {
        albedo: [0.6, 0.45, 0.3],
        emission: flythrough::ORE_EMISSION.map(|c| 8.0 * c),
        ..Material::named("vein")
    });
    (t, vein)
}

/// The cavity split by a rock partition at x in [-4, 0) up to z = 20, open above, whose
/// whole -x face is an emissive vein facing -x: from the camera at (24.5, 0.5,
/// 0.5) looking -x the vein is around the partition's corner, and no voxel the camera sees
/// takes its light directly.
fn vein_cavity() -> World {
    let (table, vein) = vein_cavity_table();
    let world = cavity_world_with(table);
    let fill = |min: [i64; 3], max: [i64; 3], mode: Mode| {
        world
            .apply(&EditOp::box_(
                Fixed3(min.map(|v| v * FIXED_ONE)),
                Fixed3(max.map(|v| v * FIXED_ONE)),
                mode,
            ))
            .unwrap();
    };
    fill([-4, -40, -40], [0, 40, 20], Mode::Fill(3));
    fill(
        [-4, -40, -40],
        [-3, 40, 20],
        Mode::Replace { from: 3, to: vein },
    );
    world
}

fn vein_cavity_camera() -> Camera {
    Camera {
        position: [24.5, 0.5, 0.5].into(),
        yaw: std::f32::consts::PI,
        pitch: 0.0,
        vfov: 70f32.to_radians(),
    }
}

/// Frames the indirect golden settles over.
const INDIRECT_FRAMES: u32 = 64;

/// The vein cavity after [`INDIRECT_FRAMES`] frames with the lamp off and indirect light
/// `indirect`, read back.
fn vein_cavity_render(world: &World, indirect: bool) -> Option<(Vec<u8>, Vec<[u16; 4]>)> {
    vein_cavity_render_under(world, indirect, None)
}

/// [`vein_cavity_render`] under FSR 2 at scale 1 when `fsr2` is `Some`: `Some(true)` with
/// the upscaler bypassed (the composite's linear target put through the tone pass alone),
/// `Some(false)` with every pass running.
fn vein_cavity_render_under(
    world: &World,
    indirect: bool,
    fsr2: Option<bool>,
) -> Option<(Vec<u8>, Vec<[u16; 4]>)> {
    let mut rig = LookRig::new(world, None)?;
    if let Some(bypass) = fsr2 {
        rig.frame.set_upscaler_kind(
            &rig.device,
            vxe_render::UpscalerKind::Fsr2(Default::default()),
        );
        rig.frame.fsr2_mut().expect("FSR mode").set_bypass(bypass);
    }
    rig.frame.set_lighting(vxe_render::Lighting {
        lamp: false,
        indirect,
        ..vxe_render::Lighting::default()
    });
    for _ in 0..INDIRECT_FRAMES {
        rig.frame.step(
            &rig.device,
            &rig.queue,
            &rig.gpu,
            &vein_cavity_camera(),
            &LightParams::headlamp(),
        );
    }
    Some((
        readback::read_rgba8(&rig.device, &rig.queue, rig.frame.target()),
        readback::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records()),
    ))
}

/// `indirect.png`: the cavity lit only by a vein around the partition's corner, after 64
/// frames with indirect light; with indirect light off the same view differs in at least 20 %
/// of its hit pixels by more than 8/255 (the vein's bounce light is what the golden shows).
#[test]
fn indirect_golden_shows_the_vein_s_bounce_light() {
    let world = vein_cavity();
    let Some((lit, info)) = vein_cavity_render(&world, true) else {
        return;
    };
    assert_golden("indirect.png", &lit);
    let (off, _) = vein_cavity_render(&world, false).expect("the adapter rendered once");
    let (mut hits, mut differ) = (0, 0);
    for (i, rec) in info.iter().enumerate() {
        if rec[3] & 1 == 0 {
            continue;
        }
        hits += 1;
        let d = (0..3)
            .map(|k| lit[4 * i + k].abs_diff(off[4 * i + k]))
            .max()
            .unwrap_or(0);
        differ += usize::from(d > 8);
    }
    let share = differ as f64 / hits as f64;
    println!(
        "indirect.png: {differ} of {hits} hit pixels ({:.1} %) differ from indirect off by more than 8/255; centre {:?} against {:?}",
        share * 100.0,
        centre_pixel(&lit),
        centre_pixel(&off)
    );
    assert!(share >= 0.20, "{:.1} %", share * 100.0);
}

/// The share of hit pixels (`info`'s flag 1) whose largest channel differs between `a` and
/// `b` by more than 8/255, and the mean absolute byte difference over those hits.
fn hit_difference(a: &[u8], b: &[u8], info: &[[u16; 4]]) -> (f64, f64) {
    let (mut hits, mut differ, mut sum) = (0usize, 0usize, 0u64);
    for (i, rec) in info.iter().enumerate() {
        if rec[3] & 1 == 0 {
            continue;
        }
        hits += 1;
        let d: Vec<u8> = (0..3)
            .map(|k| a[4 * i + k].abs_diff(b[4 * i + k]))
            .collect();
        differ += usize::from(d.iter().copied().max().unwrap_or(0) > 8);
        sum += d.iter().map(|&v| u64::from(v)).sum::<u64>();
    }
    (differ as f64 / hits as f64, sum as f64 / (3 * hits) as f64)
}

/// FSR 2 carries the probe volume's indirect light: on `indirect.png`'s view, FSR 2's
/// colour input put through the tone pass alone (the upscaler bypassed) is the plain frame's
/// image byte for byte; with every FSR pass running at scale 1, the image with indirect light
/// differs from the one without in at least 20 % of its hit pixels by more than 8/255, as the
/// plain frame's does. FSR's mean distance from the plain indirect image is a `reading:` line.
#[test]
fn fsr2_carries_the_indirect_golden_s_bounce_light() {
    let world = vein_cavity();
    let Some((plain, info)) = vein_cavity_render(&world, true) else {
        return;
    };
    let (toned, _) =
        vein_cavity_render_under(&world, true, Some(true)).expect("the adapter rendered once");
    let differing = plain.iter().zip(&toned).filter(|(a, b)| a != b).count();
    println!("indirect, FSR 2 bypassed: {differing} bytes differ from the plain frame");
    assert_eq!(differing, 0, "the tone pass changes the indirect image");
    let (fsr_lit, _) =
        vein_cavity_render_under(&world, true, Some(false)).expect("the adapter rendered once");
    let (fsr_off, _) =
        vein_cavity_render_under(&world, false, Some(false)).expect("the adapter rendered once");
    let (share, _) = hit_difference(&fsr_lit, &fsr_off, &info);
    let (far, mean) = hit_difference(&fsr_lit, &plain, &info);
    println!(
        "reading: FSR 2 at scale 1 with indirect light against without: {:.1} % of hits over 8/255; against the plain indirect image: {:.1} % over 8/255, mean {mean:.3}/255",
        share * 100.0,
        far * 100.0
    );
    assert!(share >= 0.20, "{:.1} %", share * 100.0);
}

/// The upscaler's golden scene: the cavity from its centre under the headlamp at 256 x 144
/// internal and 384 x 216 output, jittered, after 32 frames, at each of `sharpness`; with the
/// upscale pass's history (rgba16float) of the first.
fn upscaled_cavity(sharpness: &[f32]) -> Option<(Vec<Vec<u8>>, History16)> {
    let world = cavity_world();
    let rig = LookRig::new(&world, None)?;
    let mut out = Vec::new();
    let mut history = Vec::new();
    for &s in sharpness {
        let mut frame = Frame::with_sizes(&rig.device, UPSCALED_INTERNAL, UPSCALED_OUTPUT);
        frame.set_jitter(&rig.device, true);
        frame.set_sharpness(&rig.queue, s);
        for _ in 0..32 {
            frame.step(
                &rig.device,
                &rig.queue,
                &rig.gpu,
                &cavity_camera(),
                &LightParams::headlamp(),
            );
        }
        if history.is_empty() {
            history = readback::read_rgba16f(
                &rig.device,
                &rig.queue,
                frame.upscale_history().expect("the upscaler runs"),
            );
        }
        out.push(readback::read_rgba8(
            &rig.device,
            &rig.queue,
            frame.target(),
        ));
    }
    Some((out, history))
}

/// An rgba16float texture read back, texel by texel.
type History16 = Vec<[f32; 4]>;

const UPSCALED_INTERNAL: [u32; 2] = [256, 144];
const UPSCALED_OUTPUT: [u32; 2] = [384, 216];

/// `upscaled.png`: the upscaler's golden at the fly-through's default sharpening (0.5). At
/// sharpening 0 the pass writes the upscale pass's output unchanged: each byte is its history's
/// value times 255, rounded. The sharpened image's edge contrast (the mean absolute luminance
/// gradient over the pixels whose unsharpened 3 x 3 spans at least 16/255, the cavity's voxel
/// rims) is higher than the unsharpened one's, and no channel of a sharpened pixel exceeds its
/// unsharpened 3 x 3 neighbourhood's maximum by more than 1/255.
#[test]
fn upscaled_golden_sharpens_within_the_neighbourhood() {
    let Some((images, history)) = upscaled_cavity(&[0.0, 0.5]) else {
        return;
    };
    let (plain, sharp) = (&images[0], &images[1]);
    let [w, h] = UPSCALED_OUTPUT.map(|n| n as usize);
    let path = golden_path("upscaled.png");
    if std::env::var("VXE_BLESS").as_deref() == Ok("1") {
        write_png(&path, sharp, w as u32, h as u32);
        println!("blessed {path:?}");
    } else {
        let (golden, gw, gh) = read_png(&path);
        assert_eq!((gw, gh), (w as u32, h as u32));
        let f = fraction_differing(sharp, &golden, 2);
        println!(
            "upscaled.png: {:.4} % of pixels differ by more than 2/255",
            f * 100.0
        );
        assert!(f <= 0.001, "{:.4} % of pixels differ", f * 100.0);
    }
    let identity = plain
        .chunks(4)
        .zip(&history)
        .filter(|(p, t)| (0..3).any(|k| f32::from(p[k]) != (t[k] * 255.0).round()))
        .count();
    assert_eq!(identity, 0, "sharpening 0 changed {identity} pixels");
    let lum = |img: &[u8], x: usize, y: usize| {
        let p = &img[(y * w + x) * 4..];
        0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2])
    };
    let (mut rim, mut g_plain, mut g_sharp, mut over) = (0usize, 0.0f64, 0.0f64, 0usize);
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let mut lo = [255u8; 3];
            let mut hi = [0u8; 3];
            for dy in 0..3 {
                for dx in 0..3 {
                    let p = &plain[((y + dy - 1) * w + x + dx - 1) * 4..];
                    for k in 0..3 {
                        lo[k] = lo[k].min(p[k]);
                        hi[k] = hi[k].max(p[k]);
                    }
                }
            }
            let s = &sharp[(y * w + x) * 4..];
            over += usize::from((0..3).any(|k| s[k] > hi[k].saturating_add(1)));
            if (0..3).any(|k| hi[k] - lo[k] >= 16) {
                rim += 1;
                let grad = |img: &[u8]| {
                    (lum(img, x + 1, y) - lum(img, x, y)).abs()
                        + (lum(img, x, y + 1) - lum(img, x, y)).abs()
                };
                g_plain += grad(plain);
                g_sharp += grad(sharp);
            }
        }
    }
    println!(
        "rim pixels {rim}: mean absolute gradient unsharpened {:.3}, sharpened {:.3}; {over} pixels over their neighbourhood's maximum",
        g_plain / rim as f64,
        g_sharp / rim as f64
    );
    assert!(rim > 0);
    assert!(g_sharp > g_plain, "{g_sharp} against {g_plain}");
    assert_eq!(over, 0);
}

/// The level's spawn pose as the fly-through starts there: the spawn voxel's centre, yaw 0,
/// 70 degrees of vertical field of view.
fn level_spawn_camera() -> Camera {
    let source = flythrough::cave::CaveSource::new(
        1,
        ChunkExtent::new([0, 0, 0], flythrough::cave::LEVEL_CHUNKS),
    );
    Camera {
        position: source.spawn().0.map(|v| v as f32 + 0.5).into(),
        yaw: 0.0,
        pitch: 0.0,
        vfov: 70f32.to_radians(),
    }
}

/// The lighting targets are scale-free (ADR-0033): on the level at the spawn pose with the
/// headlamp on and the emissive ore, 64 frames with the lamp's intensity and every emission
/// scaled by `s` under a manual exposure of `1 / s` land, for `s` = 1/64 and 64, within 1/255
/// of `s` = 1 in every pixel.
#[test]
fn the_lighting_targets_are_scale_free() {
    let source = flythrough::cave::CaveSource::new(
        1,
        ChunkExtent::new([0, 0, 0], flythrough::cave::LEVEL_CHUNKS),
    );
    let world = World::generate(
        source.extent,
        &source,
        &flythrough::with_emissive_ore(&cave_materials()),
    );
    let pool = flythrough::level_pool_units(&world, false) * UNIT_BYTES;
    let Some((device, queue)) = software_device(pool) else {
        return;
    };
    let cfg = MirrorConfig {
        budget_bytes: 256 << 20,
        ..MirrorConfig::new(Window::Fixed(world.extent()))
    };
    let mut gpu = GpuWorld::with_config(&device, cfg);
    gpu.sync_now(&world, &device, &queue)
        .expect("mirror fits the adapter");
    let emitters: Vec<MaterialId> = world
        .materials()
        .rows()
        .iter()
        .enumerate()
        .filter(|(_, m)| m.emission != [0.0; 3])
        .map(|(i, _)| i as MaterialId)
        .collect();
    assert!(emitters.contains(&flythrough::cave::ORE), "{emitters:?}");
    let camera = level_spawn_camera();
    let shot = |s: f32| {
        let mut frame = Frame::new(&device, 192, 108);
        frame.set_lighting(vxe_render::Lighting {
            exposure: vxe_render::Exposure::Manual(1.0 / s),
            ..vxe_render::Lighting::default()
        });
        for &m in &emitters {
            frame.set_emission_scale(&queue, m, s);
        }
        let light = LightParams {
            intensity: LightParams::headlamp().intensity * s,
            ..LightParams::headlamp()
        };
        for _ in 0..64 {
            frame.step(&device, &queue, &gpu, &camera, &light);
        }
        readback::read_rgba8(&device, &queue, frame.target())
    };
    let base = shot(1.0);
    let lit = base
        .chunks(4)
        .filter(|p| p[..3].iter().any(|&c| c >= 16))
        .count();
    println!("s = 1: {lit} of {} pixels lit", base.len() / 4);
    assert!(lit > base.len() / 4 / 10, "{lit} lit pixels");
    for s in [1.0 / 64.0, 64.0] {
        let img = shot(s);
        let worst = img
            .chunks(4)
            .zip(base.chunks(4))
            .map(|(p, q)| (0..3).map(|k| p[k].abs_diff(q[k])).max().unwrap_or(0))
            .max()
            .unwrap_or(0);
        let off = img
            .chunks(4)
            .zip(base.chunks(4))
            .filter(|(p, q)| (0..3).any(|k| p[k].abs_diff(q[k]) > 1))
            .count();
        println!("s = {s}: largest channel difference {worst}/255, {off} pixels past 1/255");
        assert!(worst <= 1, "s = {s}: {worst}/255 at {off} pixels");
    }
}

/// One frame of FSR 2's golden scene at a sharpness: the frame's image, the accumulate pass's
/// output (rgba16float, the internal upscaled colour), whether RCAS was dispatched, and the
/// frame's lighting.
struct Fsr2Shot {
    image: Vec<u8>,
    accumulated: History16,
    sharpened: bool,
    lighting: vxe_render::Lighting,
}

/// FSR 2's golden scene: the cavity from its centre under the headlamp at 256 x 144 internal
/// and 384 x 216 output after 32 frames, at each of `sharpness`.
fn fsr2_cavity(sharpness: &[f32]) -> Option<Vec<Fsr2Shot>> {
    let world = cavity_world();
    let rig = LookRig::new(&world, None)?;
    let mut out = Vec::new();
    for &s in sharpness {
        let mut frame = Frame::with_sizes(&rig.device, UPSCALED_INTERNAL, UPSCALED_OUTPUT);
        frame.set_upscaler_kind(
            &rig.device,
            vxe_render::UpscalerKind::Fsr2(vxe_render::Fsr2Settings {
                sharpness: s,
                ..Default::default()
            }),
        );
        for _ in 0..32 {
            frame.step(
                &rig.device,
                &rig.queue,
                &rig.gpu,
                &cavity_camera(),
                &LightParams::headlamp(),
            );
        }
        let fsr = frame.fsr2().expect("FSR mode");
        out.push(Fsr2Shot {
            image: readback::read_rgba8(&rig.device, &rig.queue, frame.target()),
            accumulated: readback::read_rgba16f(
                &rig.device,
                &rig.queue,
                fsr.upscaled(frame.current_copy()),
            ),
            sharpened: fsr.sharpens(),
            lighting: frame.lighting(),
        });
    }
    Some(out)
}

/// `fsr2.png`: FSR 2's golden at the fly-through's default sharpening (0.5) matches. At
/// sharpening 0 RCAS is not dispatched and the image is the accumulate pass's output put
/// through the tone arithmetic, texel for texel; RCAS leaves that output alone (the same at 0
/// and 0.5). AMD's RCAS has no clamp: the sharpened pixels over their unsharpened 3 x 3
/// neighbourhood's maximum by more than 1/255, and the largest excess, are a `reading:` line.
#[test]
fn fsr2_golden_matches_and_rcas_is_bypassed_at_zero() {
    let Some(shots) = fsr2_cavity(&[0.0, 0.5]) else {
        return;
    };
    let (zero, half) = (&shots[0], &shots[1]);
    let (plain, sharp) = (&zero.image, &half.image);
    let [w, h] = UPSCALED_OUTPUT.map(|n| n as usize);
    let path = golden_path("fsr2.png");
    if std::env::var("VXE_BLESS").as_deref() == Ok("1") {
        write_png(&path, sharp, w as u32, h as u32);
        println!("blessed {path:?}");
    } else {
        let (golden, gw, gh) = read_png(&path);
        assert_eq!((gw, gh), (w as u32, h as u32));
        let f = fraction_differing(sharp, &golden, 2);
        println!(
            "fsr2.png: {:.4} % of pixels differ by more than 2/255",
            f * 100.0
        );
        assert!(f <= 0.001, "{:.4} % of pixels differ", f * 100.0);
    }
    assert!(!zero.sharpened, "RCAS dispatched at sharpening 0");
    assert!(half.sharpened, "RCAS not dispatched at sharpening 0.5");
    let moved = zero
        .accumulated
        .iter()
        .zip(&half.accumulated)
        .filter(|(a, b)| a[..3] != b[..3])
        .count();
    assert_eq!(moved, 0, "sharpening changed {moved} accumulated texels");
    let toned = zero
        .accumulated
        .iter()
        .zip(plain.chunks(4))
        .filter(|(c, p)| support::rigs::tone_bytes(**c, &zero.lighting) != [p[0], p[1], p[2]])
        .count();
    println!(
        "sharpening 0: {toned} of {} pixels differ from the toned accumulate output",
        w * h
    );
    assert_eq!(toned, 0, "{toned} pixels are not the accumulate output");
    let (mut over, mut changed, mut most) = (0usize, 0usize, 0u8);
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let mut hi = [0u8; 3];
            for dy in 0..3 {
                for dx in 0..3 {
                    let p = &plain[((y + dy - 1) * w + x + dx - 1) * 4..];
                    for k in 0..3 {
                        hi[k] = hi[k].max(p[k]);
                    }
                }
            }
            let i = (y * w + x) * 4;
            let excess = (0..3)
                .map(|k| sharp[i + k].saturating_sub(hi[k]))
                .max()
                .unwrap_or(0);
            over += usize::from(excess > 1);
            most = most.max(excess);
            changed += usize::from(sharp[i..i + 3] != plain[i..i + 3]);
        }
    }
    println!("fsr2.png: {changed} pixels sharpened");
    assert!(changed > 0, "sharpening changed nothing");
    println!(
        "reading: fsr2.png at sharpening 0.5, sharpened pixels over their unsharpened 3 x 3 maximum by more than 1/255 {over} of {changed}, largest excess {most}/255; bar: none over; owner judges, Phase 7"
    );
}
