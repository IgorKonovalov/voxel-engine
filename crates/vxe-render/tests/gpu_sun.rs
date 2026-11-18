//! The sun and the moon as the light pass's directional sample (ADR-0027): a floor of rock
//! under a key light, read from the light pass's direct target, and an absent chunk read as
//! sky by the lighting rays. Lavapipe, lamp off, no list
//! light, no sky ray, filter off, manual exposure 1.0, the full shadow walk, lighting at full
//! resolution.
//! The image bars (the penumbra, the settle after a jump, the upscaler's mask) are readings in
//! `mod reading` until the owner's window.

use std::f32::consts::FRAC_PI_2;

use vxe_render::sky::{Body, SUN_ANGULAR_RADIUS, Sky};
use vxe_render::{
    Camera, Exposure, Frame, GpuWorld, LightParams, Lighting, LightingResolution, MirrorConfig,
    ShadowWalk, Window, glam::Vec3, wgpu,
};
use vxe_world::*;

const ROCK: MaterialId = 1;
/// The floor's top: rock below voxel z 16, so the lit plane is z = 16, altitude 0.
const FLOOR: i32 = 16;
const SIZE: [u32; 2] = [64, 36];
/// The transmittance from altitude 0 to the zenith, `exp(-(0.0614, 0.1420, 0.2714))`.
const T_ZENITH: [f32; 3] = [0.9404, 0.8677, 0.7623];

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    vxe_render::test_support::any_device(32 << 20, wgpu::Features::empty())
}

fn table() -> MaterialTable {
    [Material::named("rock")].into_iter().collect()
}

/// Rock below z = FLOOR over `chunks`, plus `extra` solid voxels.
fn floor_world(chunks: [i32; 3], extra: impl Fn([i32; 3]) -> bool + Sync) -> World {
    let f = move |p: [i32; 3]| if p[2] < FLOOR || extra(p) { ROCK } else { AIR };
    World::generate(ChunkExtent::new([0, 0, 0], chunks), &FnSource(f), &table())
}

/// `world` mirrored over the fixed `window` (its extent when `None`).
fn mirror(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    world: &World,
    window: Option<ChunkExtent>,
) -> GpuWorld {
    let w = window.unwrap_or(world.extent());
    let mut gpu = GpuWorld::with_config(device, MirrorConfig::new(Window::Fixed(w)));
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    gpu.sync(world, queue, &mut enc).expect("the mirror fits");
    queue.submit([enc.finish()]);
    gpu
}

/// A sky of `sun` and `moon` (illuminance `e` each, their real radii unless given), altitude
/// 0 at the floor's top.
fn sky(sun: [f32; 3], moon: [f32; 3], e: [[f32; 3]; 2]) -> Sky {
    let body = |direction, illuminance| Body {
        direction,
        illuminance,
        angular_radius: SUN_ANGULAR_RADIUS,
    };
    Sky {
        altitude_zero_z: FLOOR as f32,
        ..Sky::new(body(sun, e[0]), body(moon, e[1]))
    }
}

fn frame(device: &wgpu::Device, size: [u32; 2], walk: ShadowWalk) -> Frame {
    let mut frame = Frame::new(device, size[0], size[1]);
    frame.set_resolution(device, LightingResolution::Full);
    frame.set_lighting(Lighting {
        lamp: false,
        filter: false,
        shadow_walk: walk,
        exposure: Exposure::Manual(1.0),
        sky_rays: 0,
        ..Lighting::default()
    });
    frame
}

/// A unit direction at elevation `el` and azimuth `az` (degrees, clockwise from north).
fn dir(el: f32, az: f32) -> [f32; 3] {
    let (e, a) = (el.to_radians(), az.to_radians());
    [e.cos() * a.sin(), e.cos() * a.cos(), e.sin()]
}

/// Over the floor's centre, looking down at 60 degrees.
fn camera(at: [f32; 2]) -> Camera {
    Camera {
        position: Vec3::new(at[0], at[1], FLOOR as f32 + 30.0),
        yaw: 0.0,
        pitch: -60f32.to_radians(),
        vfov: 60f32.to_radians(),
    }
}

/// The direct target's rgb at the centre pixel, mean over `frames` frames.
fn centre_direct(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    frame: &mut Frame,
    gpu: &GpuWorld,
    camera: &Camera,
    frames: u32,
) -> [f32; 3] {
    let mut sum = [0.0f64; 3];
    for _ in 0..frames {
        frame.step(device, queue, gpu, camera, &LightParams::headlamp());
        let d = vxe_render::debug::read_rgba16f(device, queue, frame.direct());
        let w = frame.direct().width() as usize;
        let h = frame.direct().height() as usize;
        let c = d[(h / 2) * w + w / 2];
        for k in 0..3 {
            sum[k] += f64::from(c[k]);
        }
    }
    sum.map(|v| (v / f64::from(frames)) as f32)
}

/// The light at the centre pixel, mean over `frames` frames: the temporal pass's accumulated
/// direct part's rgb (the key light's, plus the visibility pair's sky term where the sky rays
/// run, which the temporal pass adds to the direct part at every hit) plus the accumulated
/// stored part's (at a cube face the baked visibility's sky term, ADR-0070; no emissive light
/// here).
fn centre_lit(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    frame: &mut Frame,
    gpu: &GpuWorld,
    camera: &Camera,
    frames: u32,
) -> [f32; 3] {
    let mut sum = [0.0f64; 3];
    for _ in 0..frames {
        frame.step(device, queue, gpu, camera, &LightParams::headlamp());
        let w = frame.direct_accumulated().width() as usize;
        let h = frame.direct_accumulated().height() as usize;
        let d = vxe_render::debug::read_rgba16f(device, queue, frame.direct_accumulated());
        let a = vxe_render::debug::read_rgba16f(device, queue, frame.accumulated());
        let i = (h / 2) * w + w / 2;
        for k in 0..3 {
            sum[k] += f64::from(d[i][k]) + f64::from(a[i][k]);
        }
    }
    sum.map(|v| (v / f64::from(frames)) as f32)
}

fn within(got: [f32; 3], want: [f32; 3], rel: f32) -> bool {
    got.iter()
        .zip(&want)
        .all(|(g, w)| (g / w - 1.0).abs() <= rel)
}

/// Bars 1 and 2: with `E_sun = (1, 1, 1)` at the zenith a floor pixel reads the transmittance
/// from altitude 0, `(0.9404, 0.8677, 0.7623)`, within 1 %; at elevation 30 degrees
/// `0.5 exp(-2 (0.0614, 0.1420, 0.2714)) = (0.4422, 0.3764, 0.2906)` within 1 % (the flat
/// air mass of 2 is within 0.2 % of the curved one's transmittance). Mean over 16 frames, on
/// one frame whose history is dropped between the two suns.
#[test]
fn the_noon_sun_reads_the_transmittance() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = floor_world([2, 2, 1], |_| false);
    let gpu = mirror(&device, &queue, &world, None);
    let cam = camera([64.0, 40.0]);
    let below = dir(-45.0, 0.0);
    let mut f = frame(&device, SIZE, ShadowWalk::Full);
    for (sun, want) in [
        (dir(90.0, 0.0), T_ZENITH),
        (dir(30.0, 90.0), [0.4422, 0.3764, 0.2906]),
    ] {
        f.set_sky(Some(sky(sun, below, [[1.0; 3], [0.0; 3]])));
        f.reset_history();
        let got = centre_direct(&device, &queue, &mut f, &gpu, &cam, 16);
        println!("sun {sun:?}: {got:?} against {want:?}");
        assert!(within(got, want, 0.01), "{got:?} against {want:?}");
    }
}

/// Bar 4: with the sun at -30 degrees and the moon at the zenith at illuminance 1e-3 the floor
/// reads `1e-3 (0.9404, 0.8677, 0.7623)` within 1 %; with both below the horizon, 0.
#[test]
fn the_moon_lights_the_floor_at_night() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = floor_world([2, 2, 1], |_| false);
    let gpu = mirror(&device, &queue, &world, None);
    let cam = camera([64.0, 40.0]);
    let mut f = frame(&device, SIZE, ShadowWalk::Full);
    f.set_sky(Some(sky(
        dir(-30.0, 0.0),
        dir(90.0, 0.0),
        [[1.0; 3], [1e-3; 3]],
    )));
    let got = centre_direct(&device, &queue, &mut f, &gpu, &cam, 16);
    let want = T_ZENITH.map(|t| 1e-3 * t);
    println!("moon at the zenith: {got:?} against {want:?}");
    assert!(within(got, want, 0.01), "{got:?} against {want:?}");
    f.set_sky(Some(sky(
        dir(-30.0, 0.0),
        dir(-10.0, 180.0),
        [[1.0; 3], [1e-3; 3]],
    )));
    let dark = centre_direct(&device, &queue, &mut f, &gpu, &cam, 4);
    assert_eq!(dark, [0.0; 3], "both bodies down");
}

/// Plan 0032 Phase 10 bar 1, an absent chunk is sky, the sun's half: on a mirror whose window
/// holds the chunk above the floor's but whose world does not (absent, never synced), the floor
/// under a noon sun reads the noon value of bar 1 within 1 %, as it does where that chunk is
/// synced as all-air (`EMPTY_BIT`). 16-frame means.
#[test]
fn an_absent_chunk_is_sky_to_the_sun() {
    let Some((device, queue)) = device() else {
        return;
    };
    let noon = sky(dir(90.0, 0.0), dir(-90.0, 0.0), [[1.0; 3], [0.0; 3]]);
    let absent = upper_chunk_read(&device, &queue, &floor_world([1, 1, 1], |_| false), noon, 0);
    let synced = upper_chunk_read(&device, &queue, &floor_world([1, 1, 2], |_| false), noon, 0);
    println!("sun: absent {absent:?}, synced air {synced:?}");
    assert!(within(absent, T_ZENITH, 0.01), "{absent:?}");
    assert!(within(synced, T_ZENITH, 0.01), "{synced:?}");
}

/// Plan 0032 Phase 10 bar 1, an absent chunk is sky, the sky ray's half: under
/// `Sky::uniform(1.0)` with one sky ray and the key light off, the floor of
/// [`an_absent_chunk_is_sky_to_the_sun`] reads `pi` (the cosine integral of a radiance of 1)
/// within 10 % with the upper chunk absent. A 16-frame mean.
#[test]
fn an_absent_chunk_is_sky_to_the_sky_ray() {
    let Some((device, queue)) = device() else {
        return;
    };
    let uniform = Sky {
        altitude_zero_z: FLOOR as f32,
        ..Sky::uniform(1.0)
    };
    let half = floor_world([1, 1, 1], |_| false);
    let sky_ray = upper_chunk_read(&device, &queue, &half, uniform, 1);
    println!("sky ray, absent {sky_ray:?}");
    let pi = [std::f32::consts::PI; 3];
    assert!(within(sky_ray, pi, 0.10), "{sky_ray:?}");
}

/// The floor's centre light (`centre_lit`) under `s` at `rays` sky rays, a 16-frame mean, through a
/// mirror whose window holds the chunk above the floor's whether `world` has it or not.
fn upper_chunk_read(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    world: &World,
    s: Sky,
    rays: u32,
) -> [f32; 3] {
    let window = ChunkExtent::new([0, 0, 0], [1, 1, 2]);
    let cam = Camera {
        position: Vec3::new(32.0, 8.0, FLOOR as f32 + 30.0),
        ..camera([0.0, 0.0])
    };
    let gpu = mirror(device, queue, world, Some(window));
    let mut f = frame(device, SIZE, ShadowWalk::Full);
    f.set_lighting(Lighting {
        sky_rays: rays,
        ..f.lighting()
    });
    f.set_sky(Some(s));
    centre_lit(device, queue, &mut f, &gpu, &cam, 16)
}

/// Plan 0086 Phase 10 bar 2: a young chunk reads its neighbours' sky. An open floor of 3 x 3
/// chunks under `Sky::uniform(1.0)`, one sky ray, the key light off, baked at 64 samples a brick
/// direction with the centre chunk's faces withheld (the bake's face filter), so the centre
/// holds no block. On the first frame the centre floor's light is the neighbour floor's within
/// 10 % (both read at the centre pixel of a camera over each), the light pass casts no sky ray
/// at the centre's pixel (the missing-light chain's sky visibility stands in), and the store's
/// black count is 0.
#[test]
fn a_young_chunk_reads_its_neighbours_sky() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = floor_world([3, 3, 1], |_| false);
    let mut gpu = mirror(&device, &queue, &world, None);
    let uniform = Sky {
        altitude_zero_z: FLOOR as f32,
        ..Sky::uniform(1.0)
    };
    let mut f = frame(&device, SIZE, ShadowWalk::Full);
    f.set_lighting(Lighting {
        sky_rays: 1,
        ..f.lighting()
    });
    f.set_light_hooks(vxe_render::LightHooks {
        key_off: true,
        ..Default::default()
    });
    f.set_sky(Some(uniform));
    f.set_count_store(true);
    let settings = vxe_render::BakeSettings {
        brick_samples: 64,
        round: 64,
        filter: Some(std::sync::Arc::new(|v: [i32; 3], _f: u8| {
            !(v[0].div_euclid(64) == 1 && v[1].div_euclid(64) == 1)
        })),
        ..Default::default()
    };
    f.bake_until(
        &device,
        &queue,
        &mut gpu,
        &world,
        world.extent(),
        &settings,
        &mut |_| {},
    )
    .expect("the store fits");
    let mut read = |at: [f32; 2]| {
        let lit = centre_lit(&device, &queue, &mut f, &gpu, &camera(at), 1);
        let vis = vxe_render::debug::read_rgba8(&device, &queue, f.visibility_samples());
        let w = f.visibility_samples().width() as usize;
        let h = f.visibility_samples().height() as usize;
        let cast = vis[4 * ((h / 2) * w + w / 2) + 3] > 0;
        (lit, cast, f.read_store_stats(&device, &queue))
    };
    let (young, young_cast, st) = read([96.0, 96.0]);
    let (old, old_cast, _) = read([32.0, 96.0]);
    println!(
        "centre {young:?} (sky ray cast {young_cast}), neighbour {old:?} (cast {old_cast}); store missing {} fallback {} black {}",
        st.missing, st.fallback, st.still_black
    );
    assert!(within(young, old, 0.10), "{young:?} against {old:?}");
    assert!(!young_cast, "the chain stands in for the sky ray");
    assert!(st.missing > 0, "the centre's faces have no block");
    assert_eq!(st.still_black, 0);
}

/// Plan 0086 Phase 9 bar 4's twin: the floor of chunk (0, 0, 0) under a resident rock roof
/// (chunks (0, 0, 1) and (1, 0, 1)), the chunk beside the floor's, (1, 0, 0), held by the
/// window and absent from the world. Under `Sky::uniform(1.0)` with one sky ray and the key
/// light off, a sky ray into the absent chunk ends on its face below the roof, so it is rock
/// by the column map (ADR-0070's amendment): the floor reads under 1 % of `pi`. The same rig
/// with every end counted as sky (`LightHooks::sky_end_is_sky`) is printed. 16-frame means.
#[test]
fn an_absent_chunk_below_the_column_top_is_rock_to_the_sky_ray() {
    let Some((device, queue)) = device() else {
        return;
    };
    let f = |p: [i32; 3]| {
        if p[2] < FLOOR || p[2] >= 64 {
            ROCK
        } else {
            AIR
        }
    };
    let extent = ChunkExtent::new([0, 0, 0], [2, 1, 2]);
    let world = World::generate(extent, &FnSource(f), &table());
    world.store().remove(ChunkCoord([1, 0, 0]));
    let uniform = Sky {
        altitude_zero_z: FLOOR as f32,
        ..Sky::uniform(1.0)
    };
    let cam = Camera {
        position: Vec3::new(32.0, 8.0, FLOOR as f32 + 30.0),
        ..camera([0.0, 0.0])
    };
    let gpu = mirror(&device, &queue, &world, Some(extent));
    let read = |hooks: vxe_render::LightHooks| {
        let mut fr = frame(&device, SIZE, ShadowWalk::Full);
        fr.set_lighting(Lighting {
            sky_rays: 1,
            ..fr.lighting()
        });
        fr.set_light_hooks(vxe_render::LightHooks {
            key_off: true,
            ..hooks
        });
        fr.set_sky(Some(uniform));
        centre_lit(&device, &queue, &mut fr, &gpu, &cam, 16)
    };
    let rule = read(Default::default());
    let as_built = read(vxe_render::LightHooks {
        sky_end_is_sky: true,
        ..Default::default()
    });
    println!(
        "under a roof beside an absent chunk: column rule {rule:?}, every end sky {as_built:?}"
    );
    let bar = 0.01 * std::f32::consts::PI;
    assert!(rule.iter().all(|&v| v < bar), "{rule:?} against {bar}");
}

/// Plan 0086 Phase 9 bar 1: the column map is the top of opaque voxels. A 3 x 3 x 3-chunk
/// stepped terrain (rock below `h(x, y)`, a 4 x 4-voxel shaft of air through every z at
/// x, y 70..74), chunk (0, 0, 0) mirrored LOD-only from its mips. Each map texel (a 2 x 2
/// column) equals the highest rock voxel's z over its columns where that lies in a full chunk,
/// lies within one voxel above it in the LOD chunk, and is `COLUMN_NONE` over the shaft. A
/// 2 x 2 x 1 dig of a cell's top lowers its texel at the sync that uploads it, and a voxel
/// stamped 5 above raises it at that sync.
#[test]
fn the_column_map_is_the_top_of_solid() {
    let Some((device, queue)) = device() else {
        return;
    };
    let h = |x: i32, y: i32| 40 + 8 * (x / 24) + 5 * (y / 20);
    let shaft = |x: i32, y: i32| (70..74).contains(&x) && (70..74).contains(&y);
    let f = move |p: [i32; 3]| {
        if !shaft(p[0], p[1]) && p[2] < h(p[0], p[1]) {
            ROCK
        } else {
            AIR
        }
    };
    let extent = ChunkExtent::new([0, 0, 0], [3, 3, 3]);
    let world = World::generate(extent, &FnSource(f), &table());
    let lod = ChunkCoord([0, 0, 0]);
    let chunk = world.store().remove(lod).expect("generated");
    world.insert_lod(
        lod,
        std::sync::Arc::new(ChunkMips::build(&chunk.read().expect("lock"))),
    );
    let mut gpu = mirror(&device, &queue, &world, Some(extent));
    assert!(gpu.is_lod(lod), "chunk (0, 0, 0) is LOD-only");
    let side = 3 * vxe_render::COLUMN_CELLS;
    let read = |gpu: &GpuWorld| vxe_render::debug::read_r32i(&device, &queue, gpu.column_map());
    let map = read(&gpu);
    assert_eq!(map.len(), (side * side) as usize);
    let mut checked = [0u32; 3];
    for j in 0..side {
        for i in 0..side {
            let got = map[(i + side * j) as usize];
            let cols = [[0, 0], [1, 0], [0, 1], [1, 1]].map(|o| [2 * i + o[0], 2 * j + o[1]]);
            if cols.iter().all(|c| shaft(c[0], c[1])) {
                assert_eq!(got, vxe_render::COLUMN_NONE, "the shaft's cell {i} {j}");
                checked[2] += 1;
                continue;
            }
            let want = cols
                .iter()
                .filter(|c| !shaft(c[0], c[1]))
                .map(|c| h(c[0], c[1]) - 1)
                .max()
                .expect("a rock column");
            if want < 64 && i < 32 && j < 32 {
                assert!(
                    (want..=want + 1).contains(&got),
                    "LOD cell {i} {j}: {got} against {want}"
                );
                checked[1] += 1;
            } else {
                assert_eq!(got, want, "cell {i} {j}");
                checked[0] += 1;
            }
        }
    }
    println!(
        "cells checked: full {}, LOD {}, air {}",
        checked[0], checked[1], checked[2]
    );
    assert!(checked.iter().all(|&n| n > 0), "{checked:?}");
    let sync = |gpu: &mut GpuWorld| {
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        gpu.sync(&world, &queue, &mut enc).expect("the mirror fits");
        queue.submit([enc.finish()]);
    };
    // Cell (50, 50): voxels 100..102 in x and y, all at h(100, 100).
    let (cell, top) = ([50, 50], h(100, 100) - 1);
    let texel = |m: &[i32]| m[(cell[0] + side * cell[1]) as usize];
    assert_eq!(texel(&map), top);
    let centre = |v: [i32; 3]| centre_of_voxel(v);
    world
        .apply(&EditOp::box_(
            centre([100, 100, top]),
            centre([102, 102, top + 1]),
            Mode::Carve,
        ))
        .expect("the dig");
    sync(&mut gpu);
    let dug = texel(&read(&gpu));
    println!("cell {cell:?}: top {top}, dug {dug}");
    assert_eq!(dug, top - 1, "the dig lowers the texel at its sync");
    world
        .apply(&EditOp::box_(
            centre([100, 100, top + 5]),
            centre([101, 101, top + 6]),
            Mode::Fill(ROCK),
        ))
        .expect("the stamp");
    sync(&mut gpu);
    let stamped = texel(&read(&gpu));
    assert_eq!(stamped, top + 5, "the stamp raises the texel at its sync");
}

/// Bar 6's flags: turning the sun by 10 degrees in one frame raises the sky-change bit on that
/// frame and no other; a drift of 0.15 of its radius a frame over 64 frames raises it on none;
/// the list's change is raised on no frame of either. No adapter time to speak of: a 16 x 16
/// frame of the floor.
#[test]
fn a_sun_jump_sets_the_change_bit_once_and_a_drift_never() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = floor_world([1, 1, 1], |_| false);
    let gpu = mirror(&device, &queue, &world, None);
    let cam = camera([32.0, 8.0]);
    let mut f = frame(&device, [16, 16], ShadowWalk::Coarse);
    let light = LightParams::headlamp();
    let at = |el: f32| sky(dir(el, 180.0), dir(-el, 0.0), [[1.0; 3], [0.0; 3]]);
    let mut raised = Vec::new();
    let mut step = |f: &mut Frame, s: Sky| {
        f.set_sky(Some(s));
        f.step(&device, &queue, &gpu, &cam, &light);
        assert!(!f.lights_changed(), "the list never changes");
        raised.push(f.sky_changed());
    };
    for _ in 0..4 {
        step(&mut f, at(50.0));
    }
    step(&mut f, at(60.0));
    for _ in 0..4 {
        step(&mut f, at(60.0));
    }
    let r = SUN_ANGULAR_RADIUS.to_degrees();
    for i in 1..=64 {
        step(&mut f, at(60.0 + 0.15 * r * i as f32));
    }
    let frames: Vec<usize> = (0..raised.len()).filter(|&i| raised[i]).collect();
    assert_eq!(frames, [4], "the jump's frame alone");
}

/// Phase 4 bar 2: a sun of illuminance 3000 at the zenith over the floor, the exposure
/// automatic (`AutoExposure::instant()`), the filter on: after 64 frames no texel of the
/// irradiance target, the direct target, its accumulation or the moments is infinite or NaN.
#[test]
fn a_noon_sun_overflows_no_target() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = floor_world([2, 2, 1], |_| false);
    let gpu = mirror(&device, &queue, &world, None);
    let cam = camera([64.0, 40.0]);
    let mut f = frame(&device, SIZE, ShadowWalk::Coarse);
    f.set_lighting(Lighting {
        filter: true,
        exposure: Exposure::Auto(vxe_render::AutoExposure::instant()),
        ..f.lighting()
    });
    f.set_sky(Some(sky(
        dir(90.0, 0.0),
        dir(-90.0, 0.0),
        [[3000.0; 3], [0.0; 3]],
    )));
    for _ in 0..64 {
        f.step(&device, &queue, &gpu, &cam, &LightParams::headlamp());
    }
    for (name, t) in [
        ("irradiance", f.irradiance()),
        ("direct", f.direct()),
        ("direct accumulated", f.direct_accumulated()),
        ("moments", f.moments()),
    ] {
        let texels = vxe_render::debug::read_rgba16f(&device, &queue, t);
        let bad = texels.iter().flatten().filter(|v| !v.is_finite()).count();
        let top = texels.iter().flatten().fold(0.0f32, |a, &b| a.max(b));
        println!("{name}: largest {top}, {bad} not finite");
        assert_eq!(bad, 0, "{name}");
    }
}

/// The image bars of Phase 5, printed for the readings log (Phase 10 asserts them against the
/// owner's picks).
/// The floor's direct value along a row across a roof slab's shadow edge under `walk`: a 16 x
/// 16 voxel slab 20 voxels above the floor under a zenith sun of radius 5 degrees, the camera
/// 8 voxels over the floor straight down at 4 pixels a voxel, a 64-frame mean, as a share of
/// the open floor's value. `None` without an adapter.
fn roof_slab_row(walk: ShadowWalk) -> Option<SlabRow> {
    let (device, queue) = device()?;
    let slab =
        |p: [i32; 3]| p[2] == FLOOR + 20 && (24..40).contains(&p[0]) && (24..40).contains(&p[1]);
    let world = floor_world([1, 1, 1], slab);
    let gpu = mirror(&device, &queue, &world, None);
    // Between the floor and the slab, 8 voxels up, straight down with screen right +x:
    // 128 x 128 pixels over 32 voxels, 4 pixels a voxel at the floor.
    let size = [128u32, 128];
    let cam = Camera {
        position: Vec3::new(40.0, 32.0, FLOOR as f32 + 8.0),
        yaw: FRAC_PI_2,
        pitch: -FRAC_PI_2 + 1e-4,
        vfov: 2.0 * 2f32.atan(),
    };
    let mut f = frame(&device, size, walk);
    let mut s = sky(dir(90.0, 0.0), dir(-90.0, 0.0), [[1.0; 3], [0.0; 3]]);
    s.sun.angular_radius = 5f32.to_radians();
    f.set_sky(Some(s));
    let w = size[0] as usize;
    let mut sum = vec![0.0f64; w];
    let row = size[1] as usize / 2;
    let frames = 64;
    let mut xs = vec![0.0f32; w];
    for _ in 0..frames {
        f.step(&device, &queue, &gpu, &cam, &LightParams::headlamp());
        let d = vxe_render::debug::read_rgba16f(&device, &queue, f.direct());
        let p = vxe_render::debug::read_rgba32f(&device, &queue, f.hit_positions());
        for x in 0..w {
            sum[x] += f64::from(d[row * w + x][0]);
            xs[x] = p[row * w + x][0];
        }
    }
    let open = f64::from(T_ZENITH[0]);
    let v: Vec<f64> = sum.iter().map(|s| s / f64::from(frames) / open).collect();
    // The edge at x = 40 (the slab's +x side); the rise from 5 % to 95 % past it.
    let first = |t: f64| (0..w).find(|&x| xs[x] > 30.0 && v[x] >= t).map(|x| xs[x]);
    // The value at x = 40, linear between the two pixels either side of it.
    let b = (1..w)
        .find(|&x| xs[x] >= 40.0)
        .expect("the row crosses the edge");
    let t = f64::from((40.0 - xs[b - 1]) / (xs[b] - xs[b - 1]));
    Some(SlabRow {
        rise_from: first(0.05).expect("a 5 % pixel"),
        rise_to: first(0.95).expect("a 95 % pixel"),
        at_edge: v[b - 1] + t * (v[b] - v[b - 1]),
        inside_max: (0..w)
            .filter(|&x| xs[x] > 26.0 && xs[x] < 38.0)
            .map(|x| v[x])
            .fold(0.0, f64::max),
        outside_min: (0..w)
            .filter(|&x| xs[x] > 42.0)
            .map(|x| v[x])
            .fold(f64::INFINITY, f64::min),
    })
}

/// [`roof_slab_row`]'s figures, as shares of the open floor's value.
#[derive(Debug)]
struct SlabRow {
    /// The first x past the slab's middle at 5 % and at 95 %.
    rise_from: f32,
    rise_to: f32,
    /// At the geometric edge, x = 40.
    at_edge: f64,
    /// The brightest pixel more than 2 voxels inside the edge.
    inside_max: f64,
    /// The darkest pixel more than 2 voxels outside it.
    outside_min: f64,
}

/// Bar 3: a 16 x 16 voxel roof slab 20 voxels above the floor under a zenith sun of the test
/// radius of 5 degrees, full walk: floor pixels more than 2 voxels inside the shadow's
/// geometric edge read 0, those more than 2 voxels outside read the open value within 1 %, the
/// edge reads half within 10 %, and the 5 %-to-95 % rise spans 2.8 +- 0.6 voxels (the disc's
/// radius on the floor is `20 tan 5 = 1.75` voxels, and a disc segment holds 5 % at a depth of
/// 0.2 radii: `2 x 1.75 - 2 x 0.2 x 1.75 = 2.8`).
#[test]
fn a_roof_slab_casts_a_soft_shadow() {
    let Some(r) = roof_slab_row(ShadowWalk::Full) else {
        return;
    };
    let rise = r.rise_to - r.rise_from;
    println!("full walk: {r:?}, rise {rise} voxels");
    assert_eq!(r.inside_max, 0.0, "{r:?}");
    assert!((r.outside_min - 1.0).abs() <= 0.01, "{r:?}");
    assert!((r.at_edge - 0.5).abs() <= 0.05, "{r:?}");
    assert!((rise - 2.8).abs() <= 0.6, "rise {rise}");
}

/// Bar 6's settle: the floor beside a pillar under a sun turned by 10 degrees in one frame,
/// filter on, the real radius: every pixel that left the shadow reads at least 90 % of its
/// settled lit value (32 more frames under the turned sun) on frame 1 after the turn, the
/// reset taking the frame's sample, and on frame 11, the bar (without the reset the blend
/// leaves `0.8^11 = 0.086` of the old value). Both shares are printed.
#[test]
fn a_sun_jump_resets_the_direct_part() {
    let Some((device, queue)) = device() else {
        return;
    };
    let pillar =
        |p: [i32; 3]| (30..34).contains(&p[0]) && (30..34).contains(&p[1]) && p[2] < FLOOR + 30;
    let world = floor_world([1, 1, 1], pillar);
    let gpu = mirror(&device, &queue, &world, None);
    let cam = Camera {
        position: Vec3::new(32.0, 32.0, FLOOR as f32 + 60.0),
        yaw: 0.0,
        pitch: -FRAC_PI_2 + 1e-4,
        vfov: 60f32.to_radians(),
    };
    let mut f = frame(&device, [64, 64], ShadowWalk::Full);
    f.set_lighting(Lighting {
        filter: true,
        ..f.lighting()
    });
    let at = |el: f32| sky(dir(el, 180.0), dir(-el, 0.0), [[1.0; 3], [0.0; 3]]);
    let light = LightParams::headlamp();
    f.set_sky(Some(at(50.0)));
    for _ in 0..32 {
        f.step(&device, &queue, &gpu, &cam, &light);
    }
    let before = vxe_render::debug::read_rgba16f(&device, &queue, f.direct_accumulated());
    f.set_sky(Some(at(60.0)));
    let mut reads = Vec::new();
    for _ in 0..11 {
        f.step(&device, &queue, &gpu, &cam, &light);
        reads.push(vxe_render::debug::read_rgba16f(
            &device,
            &queue,
            f.direct_accumulated(),
        ));
    }
    for _ in 0..32 {
        f.step(&device, &queue, &gpu, &cam, &light);
    }
    let settled = vxe_render::debug::read_rgba16f(&device, &queue, f.direct_accumulated());
    let lit = settled.iter().map(|c| c[0]).fold(0.0f32, f32::max);
    let left: Vec<usize> = (0..before.len())
        .filter(|&i| before[i][0] < 0.1 * lit && settled[i][0] > 0.9 * lit)
        .collect();
    assert!(!left.is_empty(), "the turn moves the shadow");
    let counts = [(1, &reads[0]), (11, &reads[10])].map(|(k, r)| {
        let ok = left
            .iter()
            .filter(|&&i| r[i][0] >= 0.9 * settled[i][0])
            .count();
        println!(
            "frame {k} after the turn: {ok} of {} pixels that left the shadow at 90 %",
            left.len()
        );
        (k, ok)
    });
    for (k, ok) in counts {
        assert_eq!(ok, left.len(), "frame {k}");
    }
}

/// Bar 7: FSR 2 at 1.5 over the floor beside a pillar, the coarse walk: on the frame of a
/// 10-degree turn every sunlit floor pixel is marked in the input pass's reactive mask; over
/// 64 frames of a drift of 0.15 radius a frame, each frame's mask equals the same frame's with
/// the sun held still. Over the soak line on lavapipe.
#[test]
#[ignore = "soak"]
fn a_sun_jump_marks_the_reactive_mask_and_a_drift_does_not() {
    let Some((device, queue)) = device() else {
        return;
    };
    let pillar =
        |p: [i32; 3]| (30..34).contains(&p[0]) && (30..34).contains(&p[1]) && p[2] < FLOOR + 30;
    let world = floor_world([1, 1, 1], pillar);
    let gpu = mirror(&device, &queue, &world, None);
    let cam = Camera {
        position: Vec3::new(32.0, 20.0, FLOOR as f32 + 40.0),
        yaw: FRAC_PI_2,
        pitch: -50f32.to_radians(),
        vfov: 60f32.to_radians(),
    };
    let at = |el: f32| sky(dir(el, 180.0), dir(-el, 0.0), [[1.0; 3], [0.0; 3]]);
    let light = LightParams::headlamp();
    let mask = |f: &Frame| -> Vec<u8> {
        vxe_render::debug::read_rgba8(&device, &queue, f.fsr2().unwrap().reactive())
            .chunks(4)
            .map(|p| p[0])
            .collect()
    };
    let settled = || {
        let mut f = frame(&device, [320, 180], ShadowWalk::Coarse);
        f.set_upscaler_kind(
            &device,
            vxe_render::UpscalerKind::Fsr2(vxe_render::Fsr2Settings::default()),
        );
        f.set_sky(Some(at(50.0)));
        for _ in 0..32 {
            f.step(&device, &queue, &gpu, &cam, &light);
        }
        f
    };
    let mut f = settled();
    f.set_sky(Some(at(60.0)));
    f.step(&device, &queue, &gpu, &cam, &light);
    let jump = mask(&f);
    let d = vxe_render::debug::read_rgba16f(&device, &queue, f.direct());
    let lit: Vec<usize> = (0..d.len()).filter(|&i| d[i][0] > 0.5).collect();
    let unmarked = lit.iter().filter(|&&i| jump[i] == 0).count();
    println!(
        "jump frame: {} marked of {}; {} sunlit pixels, {unmarked} of them unmarked",
        jump.iter().filter(|&&m| m > 0).count(),
        jump.len(),
        lit.len()
    );
    assert!(!lit.is_empty());
    assert_eq!(unmarked, 0);
    // The drift from 60 degrees beside the same frames held at 60.
    let mut still = settled();
    still.set_sky(Some(at(60.0)));
    still.step(&device, &queue, &gpu, &cam, &light);
    let r = SUN_ANGULAR_RADIUS.to_degrees();
    let mut differing = 0;
    for i in 1..=64 {
        f.set_sky(Some(at(60.0 + 0.15 * r * i as f32)));
        f.step(&device, &queue, &gpu, &cam, &light);
        still.step(&device, &queue, &gpu, &cam, &light);
        differing += usize::from(mask(&f) != mask(&still));
    }
    println!("drift: {differing} of 64 frames whose mask differs from the still sun's");
    assert_eq!(differing, 0);
}

/// Step A's frame: native 320 x 180, or (`fsr`) FSR 2 at 1.5 to 480 x 270.
fn still_frame(device: &wgpu::Device, fsr: bool) -> Frame {
    if fsr {
        let mut f = Frame::with_sizes(device, [320, 180], [480, 270]);
        f.set_upscaler_kind(
            device,
            vxe_render::UpscalerKind::Fsr2(vxe_render::Fsr2Settings::default()),
        );
        f
    } else {
        Frame::new(device, 320, 180)
    }
}

/// Step A's rig on `f` ([`still_frame`]) at `rays` sky rays under `Sky::uniform(1.0)` (`None`:
/// the headlamp and no sky), filter on, the default lighting tier, manual `exposure`, the
/// history and the upscaler's dropped first: after `frames` settle frames, the mean and the
/// 99th percentile of the per-pixel change of the displayed image (the largest channel, in
/// levels of 255) between consecutive frames over `frames` frames, and the first frame's mean
/// green level.
#[allow(
    clippy::too_many_arguments,
    reason = "the rig's parts, its frame and the run's three settings"
)]
fn still_change(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    gpu: &GpuWorld,
    cam: &Camera,
    f: &mut Frame,
    rays: Option<u32>,
    exposure: f32,
    frames: u32,
) -> (f64, f64, f64) {
    let light = LightParams::headlamp();
    f.set_lighting(Lighting {
        lamp: rays.is_none(),
        exposure: Exposure::Manual(exposure),
        sky_rays: rays.unwrap_or(0),
        ..Lighting::default()
    });
    f.set_sky(rays.map(|_| Sky {
        altitude_zero_z: FLOOR as f32,
        ..Sky::uniform(1.0)
    }));
    f.reset_history();
    f.reset_upscaler_history();
    for _ in 0..frames {
        f.step(device, queue, gpu, cam, &light);
    }
    let read = |f: &Frame| vxe_render::debug::read_rgba8(device, queue, f.target());
    let mut last = read(f);
    let mean_level = last.chunks(4).map(|p| f64::from(p[1])).sum::<f64>() / (last.len() / 4) as f64;
    let mut changes = Vec::new();
    for _ in 0..frames {
        f.step(device, queue, gpu, cam, &light);
        let now = read(f);
        for (a, b) in now.chunks(4).zip(last.chunks(4)) {
            let d = (0..3).map(|k| a[k].abs_diff(b[k])).max().unwrap_or(0);
            changes.push(f64::from(d));
        }
        last = now;
    }
    changes.sort_by(f64::total_cmp);
    let mean = changes.iter().sum::<f64>() / changes.len() as f64;
    let p99 = changes[changes.len() * 99 / 100];
    (mean, p99, mean_level)
}

/// Step A's camera: 40 voxels over the 1024 x 1024 floor, looking down at 45 degrees.
fn still_camera() -> Camera {
    Camera {
        position: Vec3::new(512.0, 400.0, FLOOR as f32 + 40.0),
        yaw: FRAC_PI_2,
        pitch: -45f32.to_radians(),
        vfov: 60f32.to_radians(),
    }
}

/// Plan 0032 Phase 6 bar 4, the stop rule as a regression test on the open floor (Step A's
/// rig) at the default one sky ray under FSR 2 at 1.5: the 99th-percentile frame-to-frame
/// change of the sky-lit floor is at or under the headlamp-lit floor's. The exposures are
/// Step A's, each putting its floor near display level 118 (the readings log, Phase 6). 64
/// settle and 64 read frames; over the soak line, so the gate keeps
/// `the_sky_lit_floor_is_as_still_as_the_lamp_lit_one_natively`.
#[test]
#[ignore = "soak"]
fn the_sky_lit_floor_is_as_still_as_the_lamp_lit_one() {
    sky_lit_against_lamp_lit(true, [0.2456, 5.3589], 64);
}

/// The gate's variant of the stillness bar: native 320 x 180, 32 settle and 32 read frames,
/// at Step A's native exposures.
#[test]
fn the_sky_lit_floor_is_as_still_as_the_lamp_lit_one_natively() {
    sky_lit_against_lamp_lit(false, [0.2456, 5.2761], 32);
}

/// The stillness bar on Step A's rig, under FSR 2 at 1.5 (`fsr`) or native, at the sky's and
/// the lamp's `exposure`, `frames` settle and read frames.
fn sky_lit_against_lamp_lit(fsr: bool, exposure: [f32; 2], frames: u32) {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = floor_world([16, 16, 1], |_| false);
    let gpu = mirror(&device, &queue, &world, None);
    let cam = still_camera();
    let rays = vxe_render::DEFAULT_SKY_RAYS;
    // One frame for both arms, the lamp's first: a leftover of the lamp's run in the sky's
    // could only raise the sky's change.
    let mut f = still_frame(&device, fsr);
    let (lamp_mean, lamp_p99, lamp_level) = still_change(
        &device,
        &queue,
        &gpu,
        &cam,
        &mut f,
        None,
        exposure[1],
        frames,
    );
    let (sky_mean, sky_p99, sky_level) = still_change(
        &device,
        &queue,
        &gpu,
        &cam,
        &mut f,
        Some(rays),
        exposure[0],
        frames,
    );
    println!(
        "{rays} sky ray: level {sky_level:.1}, change mean {sky_mean:.3}, p99 {sky_p99}; headlamp: level {lamp_level:.1}, change mean {lamp_mean:.3}, p99 {lamp_p99}"
    );
    assert!(sky_p99 <= lamp_p99, "{sky_p99} against {lamp_p99}");
}

/// Under `Sky::uniform(1.0)`, one sky ray, filter off, exposure 1, the coarse walk: the
/// 64-frame mean of the light at the centre pixel of `cam` over `world` (`centre_lit`).
fn under_uniform_sky(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    world: &World,
    cam: &Camera,
) -> [f32; 3] {
    let gpu = mirror(device, queue, world, None);
    let mut f = frame(device, SIZE, ShadowWalk::Coarse);
    f.set_lighting(Lighting {
        sky_rays: 1,
        ..f.lighting()
    });
    f.set_sky(Some(Sky {
        altitude_zero_z: FLOOR as f32,
        ..Sky::uniform(1.0)
    }));
    centre_lit(device, queue, &mut f, &gpu, cam, 64)
}

/// A camera at `pos` facing +y at `pitch` degrees.
fn north_at(pos: [f32; 3], pitch: f32) -> Camera {
    Camera {
        position: Vec3::from(pos),
        yaw: FRAC_PI_2,
        pitch: pitch.to_radians(),
        vfov: 60f32.to_radians(),
    }
}

/// Plan 0032 Phase 6 bar 2: on the open 1024 x 1024 floor under `Sky::uniform(1.0)` (one sky
/// ray, a 64-frame mean) a floor pixel 57 voxels from the camera, and one about 400 voxels
/// from it, read a direct irradiance within 10 % of `pi`, the upper hemisphere's cosine
/// integral of a radiance of 1: one mechanism at every distance.
#[test]
fn the_open_floor_reads_pi_under_a_uniform_sky() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = floor_world([16, 16, 1], |_| false);
    let pi = [std::f32::consts::PI; 3];
    for (name, pitch) in [("57 voxels", -45.0), ("about 400 voxels", -5.71)] {
        let cam = north_at([512.0, 300.0, FLOOR as f32 + 40.0], pitch);
        let got = under_uniform_sky(&device, &queue, &world, &cam);
        println!("open floor, {name}: {got:?}, {:.4} of pi", got[1] / pi[1]);
        assert!(within(got, pi, 0.10), "{name}: {got:?}");
    }
}

/// Plan 0032 Phase 6 bar 3: a closed rock box with 5-voxel walls on the floor under
/// `Sky::uniform(1.0)` (one sky ray, a 64-frame mean), once near the floor's origin and once
/// 400 voxels along +x from it, the camera inside each: its interior floor reads under 2 % of
/// `pi`. Both boxes are resident, so their walls, not any absent chunk, keep the sky out.
#[test]
fn a_closed_box_takes_no_sky_light() {
    let Some((device, queue)) = device() else {
        return;
    };
    let walls = |p: [i32; 3]| {
        let inside = |k: usize, lo: i32, hi: i32| (lo..hi).contains(&p[k]);
        [100, 500].iter().any(|&x0| {
            let outer =
                inside(0, x0, x0 + 40) && inside(1, 100, 140) && inside(2, FLOOR, FLOOR + 30);
            let inner =
                inside(0, x0 + 5, x0 + 35) && inside(1, 105, 135) && inside(2, FLOOR, FLOOR + 25);
            outer && !inner
        })
    };
    let world = floor_world([16, 16, 1], walls);
    for x0 in [100.0, 500.0] {
        let cam = north_at([x0 + 20.0, 110.0, FLOOR as f32 + 12.0], -45.0);
        let got = under_uniform_sky(&device, &queue, &world, &cam);
        println!("inside the box at x {x0}: {got:?}");
        assert!(
            got.iter().all(|&c| c < 0.02 * std::f32::consts::PI),
            "{got:?}"
        );
    }
}

mod reading {
    use super::*;

    /// Plan 0086 Phase 4 bar 3: the baked sky term (ADR-0070) against the ray. Under
    /// `Sky::uniform(1.0)`, filter off, exposure 1, one sky ray, the coarse walk: `world`'s
    /// bricks within two of `brick` baked alone at 256 samples a brick direction (the bake's
    /// face filter), then four frames: the centre pixel's emissive part (`accumulated`, the
    /// store's term with no history) and direct part (the ray's sample where the store holds
    /// no visibility). Returns `(baked, direct)`.
    fn baked_term(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        world: &World,
        cam: &Camera,
        brick: [i32; 3],
    ) -> ([f32; 4], [f32; 4]) {
        let mut gpu = mirror(device, queue, world, None);
        let mut f = frame(device, SIZE, ShadowWalk::Coarse);
        f.set_lighting(Lighting {
            sky_rays: 1,
            ..f.lighting()
        });
        f.set_sky(Some(Sky {
            altitude_zero_z: FLOOR as f32,
            ..Sky::uniform(1.0)
        }));
        let settings = vxe_render::BakeSettings {
            round: 64,
            brick_samples: 256,
            filter: Some(std::sync::Arc::new(move |v: [i32; 3], _| {
                (0..3).all(|k| (v[k].div_euclid(8) - brick[k]).abs() <= 2)
            })),
            ..vxe_render::BakeSettings::default()
        };
        let report = f
            .bake_until(
                device,
                queue,
                &mut gpu,
                world,
                world.extent(),
                &settings,
                &mut |_| {},
            )
            .expect("the store fits");
        println!(
            "  bake: {} faces, {} sky rays",
            report.faces, report.sky_rays
        );
        for _ in 0..4 {
            f.step(device, queue, &gpu, cam, &LightParams::headlamp());
        }
        let at = |t: &wgpu::Texture| {
            let d = vxe_render::debug::read_rgba16f(device, queue, t);
            let (w, h) = (t.width() as usize, t.height() as usize);
            d[(h / 2) * w + w / 2]
        };
        (at(f.accumulated()), at(f.direct()))
    }

    /// Plan 0086 Phase 4 bar 3: the open floor's baked sky term against `pi` (Plan 0032 Phase 6
    /// bar 2's ray read 3.1406), a wall's outward face in the open against `pi / 2`, the closed
    /// box's floor against 0; each beside the ray's 64-frame mean on the same pixel with no
    /// bake. Printed.
    #[test]
    #[ignore = "reading"]
    fn the_baked_sky_term_matches_the_ray() {
        let Some((device, queue)) = device() else {
            return;
        };
        let walls = |p: [i32; 3]| {
            let inside = |k: usize, lo: i32, hi: i32| (lo..hi).contains(&p[k]);
            let outer = inside(0, 100, 140) && inside(1, 100, 140) && inside(2, FLOOR, FLOOR + 30);
            let inner = inside(0, 105, 135) && inside(1, 105, 135) && inside(2, FLOOR, FLOOR + 25);
            let wall = inside(0, 192, 200) && inside(1, 40, 216) && inside(2, FLOOR, FLOOR + 80);
            (outer && !inner) || wall
        };
        let world = floor_world([4, 4, 1], walls);
        let pi = std::f32::consts::PI;
        let rigs = [
            // About 40 voxels ahead of the camera on the open floor.
            (
                "open floor",
                north_at([40.0, 20.0, FLOOR as f32 + 40.0], -45.0),
                [5, 7, 1],
                pi,
            ),
            // The wall's +x face at x 200, looking -x at z FLOOR + 30.
            (
                "wall +x",
                Camera {
                    position: Vec3::new(230.0, 128.0, FLOOR as f32 + 30.0),
                    yaw: std::f32::consts::PI,
                    pitch: 0.0,
                    vfov: 60f32.to_radians(),
                },
                [24, 16, 5],
                pi / 2.0,
            ),
            (
                "closed box",
                north_at([120.0, 110.0, FLOOR as f32 + 12.0], -45.0),
                [15, 14, 1],
                0.0,
            ),
        ];
        for (name, cam, brick, want) in rigs {
            let (baked, direct) = baked_term(&device, &queue, &world, &cam, brick);
            let ray = under_uniform_sky(&device, &queue, &world, &cam);
            println!(
                "{name}: baked {:.4} ({:.4} of the expected {want:.4}), direct with the bake \
                 {:.4}, the ray's 64-frame mean {:.4}",
                baked[1],
                baked[1] / want.max(1e-6),
                direct[1],
                ray[1]
            );
        }
    }

    /// Bar 3 under the coarse walk, printed (the bar is on the full walk).
    #[test]
    #[ignore = "reading"]
    fn a_roof_slab_casts_a_soft_shadow_under_the_coarse_walk() {
        if let Some(r) = super::roof_slab_row(ShadowWalk::Coarse) {
            println!("coarse: {r:?}");
        }
    }

    /// Bars 1, 2 and 4 under the default coarse walk, printed.
    #[test]
    #[ignore = "reading"]
    fn the_floor_values_under_the_coarse_walk() {
        let Some((device, queue)) = device() else {
            return;
        };
        let world = floor_world([2, 2, 1], |_| false);
        let gpu = mirror(&device, &queue, &world, None);
        let cam = camera([64.0, 40.0]);
        for (name, s) in [
            (
                "noon",
                sky(dir(90.0, 0.0), dir(-45.0, 0.0), [[1.0; 3], [0.0; 3]]),
            ),
            (
                "slant",
                sky(dir(30.0, 90.0), dir(-45.0, 0.0), [[1.0; 3], [0.0; 3]]),
            ),
            (
                "moon",
                sky(dir(-30.0, 0.0), dir(90.0, 0.0), [[1.0; 3], [1e-3; 3]]),
            ),
        ] {
            let mut f = frame(&device, SIZE, ShadowWalk::Coarse);
            f.set_sky(Some(s));
            let got = centre_direct(&device, &queue, &mut f, &gpu, &cam, 16);
            println!("coarse, {name}: {got:?}");
        }
    }

    /// Plan 0032 Phase 6 Step A: a still camera 40 voxels over a 1024 x 1024 rock floor,
    /// looking down at 45 degrees, filter on, the default lighting tier, native 320 x 180 and
    /// FSR 2 at 1.5 to 480 x 270. Under `Sky::uniform(1.0)` at 1, 2 and 4 sky rays, and with
    /// the headlamp and no sky: after 64 settle frames, the mean and the 99th percentile of
    /// the per-pixel change of the displayed image (the largest channel, in levels of 255)
    /// between consecutive frames over 64 frames. The exposure is pinned so the floor's mean
    /// display level is near 118 (mid-grey, 0.18 linear).
    #[test]
    #[ignore = "reading"]
    fn sky_ray_stillness() {
        let Some((device, queue)) = device() else {
            return;
        };
        let world = floor_world([16, 16, 1], |_| false);
        let gpu = mirror(&device, &queue, &world, None);
        let cam = Camera {
            position: Vec3::new(512.0, 400.0, FLOOR as f32 + 40.0),
            yaw: FRAC_PI_2,
            pitch: -45f32.to_radians(),
            vfov: 60f32.to_radians(),
        };
        let run = |rays, fsr, exposure| {
            let mut f = still_frame(&device, fsr);
            still_change(&device, &queue, &gpu, &cam, &mut f, rays, exposure, 64)
        };
        for fsr in [false, true] {
            let name = if fsr { "FSR 2 at 1.5" } else { "native" };
            for rays in [None, Some(1), Some(2), Some(4)] {
                // Two passes: the second's exposure scales the first's toward level 118 by the
                // display's gamma of 2.2.
                let mut ex = if rays.is_some() { 0.5 } else { 1.0 };
                let (_, _, level) = run(rays, fsr, ex);
                ex *= ((118.0 / level.max(1.0)) as f32).powf(2.2);
                let (mean, p99, level) = run(rays, fsr, ex);
                let what = rays.map_or("the headlamp, no sky".into(), |n| format!("{n} sky rays"));
                println!(
                    "{name}, {what}: exposure {ex:.4}, mean level {level:.1}, change mean {mean:.3}, p99 {p99}"
                );
            }
        }
    }

    /// The canopy rig's leaf material.
    const LEAF: MaterialId = 2;
    /// The canopy's centre column and height above the floor, and its radius.
    const CANOPY: [i32; 3] = [64, 64, 80];
    const CANOPY_R: i32 = 12;
    /// The canopy's kept voxels, of 256 (the testbed's `LEAF_FILL`).
    const FILL: u64 = 64;

    /// Whether `p` is a kept canopy voxel: inside the sphere and its hash under FILL of 256.
    fn canopy_at(p: [i32; 3]) -> bool {
        let d = [0, 1, 2].map(|k| i64::from(p[k] - [CANOPY[0], CANOPY[1], FLOOR + CANOPY[2]][k]));
        if d.iter().map(|v| v * v).sum::<i64>() > i64::from(CANOPY_R * CANOPY_R) {
            return false;
        }
        let mut h = (p[0] as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (p[1] as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
            ^ (p[2] as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
        h ^= h >> 31;
        h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^= h >> 29;
        h & 255 < FILL
    }

    /// Per floor pixel of the canopy rig (a floating canopy of radius 12 at `leaf_fill` 64, 80
    /// voxels over a 128 x 128 rock floor; the camera 40 voxels over the floor under it looking
    /// straight down, 256 x 256 at 4 pixels a voxel; a zenith sun of `E = (1, 1, 1)` at the real
    /// radius, the key light alone, the coarse walk, lighting at full resolution): the direct
    /// target's luminance, a 16-frame mean, and the hit's floor voxel (x, y). `canopy` false
    /// drops the canopy; `styled` sets SHADOW_AS_BRICK on its material.
    fn canopy_floor(canopy: bool, styled: bool) -> Option<Vec<(f32, [i32; 2])>> {
        let (device, queue) = device()?;
        let table: MaterialTable = [Material::named("rock"), Material::named("leaf")]
            .into_iter()
            .collect();
        let f = move |p: [i32; 3]| {
            if p[2] < FLOOR {
                ROCK
            } else if canopy && canopy_at(p) {
                LEAF
            } else {
                AIR
            }
        };
        let world = World::generate(ChunkExtent::new([0, 0, 0], [2, 2, 2]), &FnSource(f), &table);
        let gpu = mirror(&device, &queue, &world, None);
        let mut frame = super::frame(&device, [256, 256], ShadowWalk::Coarse);
        frame.set_sky(Some(sky(
            dir(90.0, 0.0),
            dir(-90.0, 0.0),
            [[1.0; 3], [0.0; 3]],
        )));
        if styled {
            let mut styles = vec![0; usize::from(LEAF) + 1];
            styles[usize::from(LEAF)] = vxe_render::SHADOW_AS_BRICK;
            frame.set_material_styles(&queue, &styles);
        }
        // tan(hfov / 2) = 32 / 40: 64 voxels across 256 pixels at the floor.
        let cam = Camera {
            position: Vec3::new(64.0, 64.0, FLOOR as f32 + 40.0),
            yaw: 0.0,
            pitch: -FRAC_PI_2 + 1e-4,
            vfov: 2.0 * 0.8f32.atan(),
        };
        let mut sum: Vec<f64> = Vec::new();
        for _ in 0..16 {
            frame.step(&device, &queue, &gpu, &cam, &LightParams::headlamp());
            let d = vxe_render::debug::read_rgba16f(&device, &queue, frame.direct());
            sum.resize(d.len(), 0.0);
            for (s, p) in sum.iter_mut().zip(&d) {
                *s += f64::from(p[3]) / 16.0;
            }
        }
        let hits = vxe_render::debug::read_rgba32f(&device, &queue, frame.light_hit_positions());
        Some(
            sum.iter()
                .zip(&hits)
                .filter(|(_, h)| h[3] < 1.0e30 && (h[2] - FLOOR as f32).abs() < 1e-2)
                .map(|(s, h)| (*s as f32, [h[0].floor() as i32, h[1].floor() as i32]))
                .collect(),
        )
    }

    /// Plan 0086 Phase 2 bars 1 and 2, as readings: on the canopy rig, the floor pixels inside
    /// the projection of the canopy's occupied bricks, with SHADOW_AS_BRICK set and clear: the
    /// share over 1 % and over 50 % of the open value (the same pose with no canopy); and with
    /// it set, of the floor pixels more than 8 voxels outside that projection, the share off the
    /// open value by more than 1 %. Prints them.
    #[test]
    #[ignore = "reading"]
    fn a_flagged_canopy_shadows_as_its_bricks() {
        let Some(open) = canopy_floor(false, false) else {
            return;
        };
        let open_v = open.iter().map(|p| f64::from(p.0)).sum::<f64>() / open.len() as f64;
        // The canopy's occupied bricks, projected down: (x >> 3, y >> 3).
        let mut bricks = std::collections::HashSet::new();
        for z in CANOPY[2] - CANOPY_R..=CANOPY[2] + CANOPY_R {
            for y in CANOPY[1] - CANOPY_R..=CANOPY[1] + CANOPY_R {
                for x in CANOPY[0] - CANOPY_R..=CANOPY[0] + CANOPY_R {
                    if canopy_at([x, y, FLOOR + z]) {
                        bricks.insert([x >> 3, y >> 3]);
                    }
                }
            }
        }
        let inside = |v: [i32; 2]| bricks.contains(&[v[0] >> 3, v[1] >> 3]);
        // Over 8 voxels from every projected brick's square.
        let far_out = |v: [i32; 2]| {
            bricks.iter().all(|b| {
                let dx = (b[0] * 8 - v[0]).max(v[0] - (b[0] * 8 + 7)).max(0);
                let dy = (b[1] * 8 - v[1]).max(v[1] - (b[1] * 8 + 7)).max(0);
                dx.max(dy) > 8
            })
        };
        println!(
            "reading: canopy rig, {} occupied brick columns, open value {open_v:.4} over {} px",
            bricks.len(),
            open.len()
        );
        for styled in [true, false] {
            let px = canopy_floor(true, styled).expect("the adapter");
            let ins: Vec<f64> = px
                .iter()
                .filter(|p| inside(p.1))
                .map(|p| f64::from(p.0) / open_v)
                .collect();
            let over =
                |r: f64| ins.iter().filter(|&&v| v > r).count() as f64 / ins.len().max(1) as f64;
            let outs: Vec<f64> = px
                .iter()
                .filter(|p| far_out(p.1))
                .map(|p| f64::from(p.0) / open_v)
                .collect();
            let off = outs.iter().filter(|&&v| (v - 1.0).abs() > 0.01).count();
            println!(
                "reading: SHADOW_AS_BRICK {}: inside {} px, over 1 % {:.4}, over 50 % {:.4}; outside by 8 {} px, off open by 1 % {off}",
                if styled { "set" } else { "clear" },
                ins.len(),
                over(0.01),
                over(0.5),
                outs.len(),
            );
        }
    }

    /// Per frame the history counters, then the direct accumulation's luminance and the lighting
    /// grid's hit positions.
    type FarRun = (Vec<[u32; 4]>, Vec<f32>, Vec<[f32; 4]>);

    /// A far frame of `world` at `cam`: 320 x 180 under FSR 2 at 1.5 (480 x 270), half lighting,
    /// the store counters on, the key light alone (lamp off, no sky ray, filter on), manual
    /// exposure 1, `sky`, the stair rules hooked off with `no_stair`; `frames` frames, then per
    /// frame of 64 more the history counters and, at the last, the direct accumulation's
    /// luminance and the lighting grid's hit positions.
    fn far_run(world: &World, cam: &Camera, sky_: Sky, no_stair: bool) -> Option<FarRun> {
        let (device, queue) = device()?;
        let gpu = mirror(&device, &queue, world, None);
        let mut frame = Frame::with_sizes(&device, [320, 180], [480, 270]);
        frame.set_upscaler_kind(
            &device,
            vxe_render::UpscalerKind::Fsr2(vxe_render::Fsr2Settings::default()),
        );
        frame.set_lighting(Lighting {
            lamp: false,
            exposure: Exposure::Manual(1.0),
            sky_rays: 0,
            ..Lighting::default()
        });
        frame.set_light_hooks(vxe_render::LightHooks {
            no_stair,
            ..Default::default()
        });
        frame.set_count_store(true);
        frame.set_resolution(&device, LightingResolution::Full);
        // The fly-through's default far normal.
        frame.set_look(vxe_render::LookParams {
            far_normal: [2.0, 6.0],
            ..vxe_render::LookParams::default()
        });
        frame.set_sky(Some(sky_));
        let light = LightParams::headlamp();
        for _ in 0..16 {
            frame.step(&device, &queue, &gpu, cam, &light);
        }
        let mut counts = Vec::new();
        for _ in 0..64 {
            frame.step(&device, &queue, &gpu, cam, &light);
            counts.push(frame.read_history_rejections(&device, &queue));
        }
        let lum = vxe_render::debug::read_rgba16f(&device, &queue, frame.direct_accumulated())
            .iter()
            .map(|p| p[3])
            .collect();
        let hits = vxe_render::debug::read_rgba32f(&device, &queue, frame.light_hit_positions());
        Some((counts, lum, hits))
    }

    /// A camera at `eye` looking at `at`.
    fn looking(eye: [f32; 3], at: [f32; 3]) -> Camera {
        let d = Vec3::from(at) - Vec3::from(eye);
        Camera {
            position: Vec3::from(eye),
            yaw: d.y.atan2(d.x),
            pitch: (d.z / d.length()).asin(),
            vfov: 60f32.to_radians(),
        }
    }

    /// Plan 0086 Phase 3 bar 1, as a reading: a staircase of 64 one-voxel steps up along +x
    /// (rock under `z = FLOOR + clamp(x - 160, 0, 64)` over a 384 x 192 floor) seen from about
    /// 220 voxels (0.7 pixels a voxel at 320 wide, the far normal's full weight), FSR 2 at 1.5,
    /// a still camera, a zenith sun: the share of the history test's hits whose history reset per
    /// frame over 64 frames, by cause, with the stair rules hooked off and on. Prints them.
    #[test]
    #[ignore = "reading"]
    fn a_far_staircase_keeps_its_history() {
        let f = |p: [i32; 3]| {
            if p[2] < FLOOR + (p[0] - 160).clamp(0, 64) {
                ROCK
            } else {
                AIR
            }
        };
        let world = World::generate(
            ChunkExtent::new([0, 0, 0], [6, 3, 2]),
            &FnSource(f),
            &table(),
        );
        let cam = looking(
            [40.0, 96.0, FLOOR as f32 + 120.0],
            [192.0, 96.0, FLOOR as f32 + 32.0],
        );
        let s = sky(dir(90.0, 0.0), dir(-90.0, 0.0), [[1.0; 3], [0.0; 3]]);
        for no_stair in [true, false] {
            let Some((counts, _, _)) = far_run(&world, &cam, s, no_stair) else {
                return;
            };
            let t: [u64; 4] = std::array::from_fn(|k| counts.iter().map(|w| u64::from(w[k])).sum());
            let max = counts
                .iter()
                .map(|w| f64::from(w[1] + w[2] + w[3]) / f64::from(w[0].max(1)))
                .fold(0.0, f64::max);
            println!(
                "reading: far staircase, stair rules {}: tested {} a frame, reset {:.4} (material {:.4} normal {:.4} other {:.4}), worst frame {max:.4}",
                if no_stair { "off" } else { "on" },
                t[0] / 64,
                (t[1] + t[2] + t[3]) as f64 / t[0].max(1) as f64,
                t[1] as f64 / t[0].max(1) as f64,
                t[2] as f64 / t[0].max(1) as f64,
                t[3] as f64 / t[0].max(1) as f64,
            );
        }
    }

    /// Plan 0086 Phase 3 bar 2, as a reading: a one-voxel wall 40 voxels tall (x = 256, y 32 to
    /// 160) on a 384 x 192 floor, seen from about 220 voxels on its -x side (0.7 pixels a voxel),
    /// the sun 20 degrees up on its +x side (its shadow falls toward the camera), lighting at
    /// full resolution: of the floor pixels 1 to 3 rows under the wall's lowest pixel in each
    /// column, the share whose key light reads over 10 % of the open floor's (the floor behind
    /// the wall, x over 262), with the stair rules (the lift) hooked off and on. Prints them.
    #[test]
    #[ignore = "reading"]
    fn a_thin_far_wall_does_not_leak() {
        let f = |p: [i32; 3]| {
            let wall = p[0] == 256 && (32..160).contains(&p[1]) && p[2] < FLOOR + 40;
            if p[2] < FLOOR || wall { ROCK } else { AIR }
        };
        let world = World::generate(
            ChunkExtent::new([0, 0, 0], [6, 3, 2]),
            &FnSource(f),
            &table(),
        );
        let cam = looking(
            [56.0, 96.0, FLOOR as f32 + 90.0],
            [256.0, 96.0, FLOOR as f32],
        );
        let s = sky(dir(20.0, 90.0), dir(-20.0, 270.0), [[1.0; 3], [0.0; 3]]);
        for no_stair in [true, false] {
            let Some((_, lum, hits)) = far_run(&world, &cam, s, no_stair) else {
                return;
            };
            let floor = |h: &[f32; 4]| h[3] < 1.0e30 && (h[2] - FLOOR as f32).abs() < 1e-2;
            let wall = |h: &[f32; 4]| h[3] < 1.0e30 && h[2] > FLOOR as f32 + 0.5 && h[0] < 257.5;
            // Open: the lit floor behind the wall.
            let open: Vec<f32> = lum
                .iter()
                .zip(&hits)
                .filter(|(_, h)| floor(h) && h[0] > 262.0)
                .map(|(l, _)| *l)
                .collect();
            let open_v = open.iter().sum::<f32>() / open.len().max(1) as f32;
            // Per column, the floor pixels 1 to 3 rows under the wall's lowest pixel.
            let (w, h) = (320usize, 180usize);
            let mut band: Vec<f32> = Vec::new();
            for x in 0..w {
                let Some(base) = (0..h).rev().find(|&y| wall(&hits[y * w + x])) else {
                    continue;
                };
                for y in base + 1..=(base + 3).min(h - 1) {
                    let i = y * w + x;
                    if floor(&hits[i]) && hits[i][0] < 256.0 {
                        band.push(lum[i] / open_v);
                    }
                }
            }
            let leak = band.iter().filter(|&&v| v > 0.1).count();
            println!(
                "reading: thin far wall, stair rules {}: open {open_v:.4} over {} px; band {} px, over 10 % {leak} ({:.4})",
                if no_stair { "off" } else { "on" },
                open.len(),
                band.len(),
                leak as f64 / band.len().max(1) as f64
            );
        }
    }

    /// One frame of [`pair_run`]: the pair and the visibility sample per lighting pixel.
    type PairFrame = (Vec<[f32; 4]>, Vec<[u8; 4]>);

    /// [`pair_run`]'s edit: per frame index, the box it changed in the world, if any.
    type PairEdit<'a> = dyn FnMut(u32, &World) -> Option<[[i32; 3]; 2]> + 'a;

    /// The visibility pair's rig (ADR-0070): `world` mirrored, a frame of SIZE at full lighting
    /// resolution with no jitter, filter off, exposure 1, one sky ray, the coarse walk,
    /// `hooks`, under `Sky::uniform(1.0)` (no key light). `frames` frames of `cam`, `edit`
    /// called before each with the frame's index, the world and the mirror (it returns an edit
    /// box to invalidate); per frame, the pair and the visibility sample at every lighting pixel.
    fn pair_run(
        world: &World,
        cam: &Camera,
        frames: u32,
        hooks: vxe_render::LightHooks,
        edit: &mut PairEdit<'_>,
    ) -> Option<Vec<PairFrame>> {
        let (device, queue) = device()?;
        let mut gpu = mirror(&device, &queue, world, None);
        let mut f = frame(&device, SIZE, ShadowWalk::Coarse);
        f.set_jitter(&device, false);
        f.set_lighting(Lighting {
            sky_rays: 1,
            ..f.lighting()
        });
        f.set_light_hooks(hooks);
        f.set_sky(Some(Sky {
            altitude_zero_z: FLOOR as f32,
            ..Sky::uniform(1.0)
        }));
        let mut out = Vec::new();
        for i in 0..frames {
            if let Some([min, max]) = edit(i, world) {
                let mut enc =
                    device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
                gpu.sync(world, &queue, &mut enc).expect("the mirror fits");
                queue.submit([enc.finish()]);
                f.invalidate(min, max);
            }
            f.step(&device, &queue, &gpu, cam, &LightParams::headlamp());
            let pair = vxe_render::debug::read_rgba16f(&device, &queue, f.visibility_pair());
            let vis = vxe_render::debug::read_rgba8(&device, &queue, f.visibility_samples());
            let vis = vis.chunks(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
            out.push((pair, vis));
        }
        Some(out)
    }

    /// The centre lighting pixel's index of a SIZE frame.
    fn centre() -> usize {
        let [w, h] = SIZE.map(|v| v as usize);
        (h / 2) * w + w / 2
    }

    /// Plan 0086 Phase 5 bar 1: a floor point at the foot of a half-plane roof's edge 20 voxels
    /// up (the roof over y >= 512 across the 1024 x 1024 floor, a hit rate of 0.5), under
    /// `Sky::uniform(1.0)`, the ray forced on: the pair's mean at the centre pixel after frame
    /// 64, and its largest frame-to-frame change over frames 65 to 128 against the running
    /// mean's step at the cap, 1 / 64. Printed.
    #[test]
    #[ignore = "reading"]
    fn the_visibility_pairs_mean_holds_still() {
        let roof = |p: [i32; 3]| p[2] == FLOOR + 20 && p[1] >= 512;
        let world = floor_world([16, 16, 1], roof);
        let cam = looking(
            [512.0, 470.0, FLOOR as f32 + 10.0],
            [512.0, 512.0, FLOOR as f32],
        );
        let hooks = vxe_render::LightHooks {
            sky_ray_always: true,
            ..Default::default()
        };
        let Some(run) = pair_run(&world, &cam, 128, hooks, &mut |_, _| None) else {
            return;
        };
        let c = centre();
        let r: Vec<f32> = run.iter().map(|(p, _)| p[c][0]).collect();
        let hits = run.iter().filter(|(_, v)| v[c][0] > 127).count();
        let late = &r[64..];
        let mean = late.iter().sum::<f32>() / late.len() as f32;
        let step = late
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0f32, f32::max);
        println!(
            "reading: half-plane edge: hit rate over 128 frames {:.3}; pair mean at frame 64 \
             {:.4}, over 65..128 {mean:.4}; largest change a frame {step:.4} (1/64 = {:.4}); age \
             {}",
            hits as f32 / 128.0,
            r[63],
            1.0 / 64.0,
            run[127].0[c][3]
        );
    }

    /// Plan 0086 Phase 5 bar 2: a floor point under a roof 20 voxels up with a 6 x 6 hole
    /// straight above it (the form factor `36 / (400 pi) = 0.029`), the ray forced on: the
    /// pair's largest mean over frames 9 to 128 and its mean over them (the clamp's dark bias
    /// against 0.029), with the clamp and without it. Printed.
    #[test]
    #[ignore = "reading"]
    fn a_rare_hit_is_clamped() {
        let (cx, cy) = (512, 512);
        let roof = move |p: [i32; 3]| {
            p[2] == FLOOR + 20
                && !((cx - 3..cx + 3).contains(&p[0]) && (cy - 3..cy + 3).contains(&p[1]))
        };
        let world = floor_world([16, 16, 1], roof);
        let cam = looking(
            [512.0, 480.0, FLOOR as f32 + 10.0],
            [512.0, 512.0, FLOOR as f32],
        );
        for no_clamp in [false, true] {
            let hooks = vxe_render::LightHooks {
                sky_ray_always: true,
                no_clamp,
                ..Default::default()
            };
            let Some(run) = pair_run(&world, &cam, 128, hooks, &mut |_, _| None) else {
                return;
            };
            let c = centre();
            let r: Vec<f32> = run.iter().map(|(p, _)| p[c][0]).collect();
            let hits = run.iter().filter(|(_, v)| v[c][0] > 127).count();
            let tail = &r[8..];
            let max = tail.iter().copied().fold(0f32, f32::max);
            let mean = tail.iter().sum::<f32>() / tail.len() as f32;
            println!(
                "reading: 6 x 6 hole, clamp {}: {hits} hits in 128 frames; pair over frames 9..128: \
                 largest {max:.4}, mean {mean:.4} ({:.3} of 0.029)",
                if no_clamp { "off" } else { "on" },
                mean / 0.029
            );
        }
    }

    /// Plan 0086 Phase 5 bar 3: Plan 0032 Phase 5's 4 x 4 pillar, 30 voxels tall, moved one
    /// voxel a frame along +x across the open floor under `Sky::uniform(1.0)` (an edit a frame,
    /// its box invalidated), the ray forced on: for every lighting pixel the pillar's sky
    /// shadow leaves (its pair mean under 0.9 at frame 32 and its sample open from frame 33 on
    /// within the open band), the frames until the mean reads at least 0.9 of the open floor's.
    /// The distribution, printed.
    #[test]
    #[ignore = "reading"]
    fn a_moving_shadow_edge_resets_the_history() {
        let world = floor_world([4, 4, 1], |_| false);
        let x0 = 100i64;
        let pillar = |x: i64, m: MaterialId| {
            EditOp::box_(
                Fixed3([x, 126, i64::from(FLOOR)].map(|v| v * FIXED_ONE)),
                Fixed3([x + 3, 129, i64::from(FLOOR) + 29].map(|v| v * FIXED_ONE)),
                Mode::Fill(m),
            )
        };
        world.apply(&pillar(x0, ROCK)).expect("resident");
        let cam = looking(
            [120.0, 80.0, FLOOR as f32 + 60.0],
            [120.0, 128.0, FLOOR as f32],
        );
        let hooks = vxe_render::LightHooks {
            sky_ray_always: true,
            ..Default::default()
        };
        let mut edit = |i: u32, w: &World| {
            if !(32..64).contains(&i) {
                return None;
            }
            let x = x0 + i64::from(i - 32);
            w.apply(&pillar(x, AIR)).expect("resident");
            let r = w.apply(&pillar(x + 1, ROCK)).expect("resident");
            let (min, max) = r.aabb.expect("a change");
            Some([[min.0[0] - 1, min.0[1], min.0[2]], max.0])
        };
        let Some(run) = pair_run(&world, &cam, 128, hooks, &mut edit) else {
            return;
        };
        let n = run[0].0.len();
        let mut frames = Vec::new();
        let mut never = 0;
        for i in 0..n {
            // Shadowed at frame 31 (the pillar still) and open at the end.
            let before = run[31].0[i][0];
            let after = run[127].0[i][0];
            if !(before < 0.9 && after > 0.97) {
                continue;
            }
            // The first frame from which the sample stays open.
            let Some(open) = (32..128)
                .rev()
                .find(|&f| run[f].1[i][0] < 255)
                .map(|f| f + 1)
            else {
                continue;
            };
            if open >= 120 {
                continue;
            }
            match (open..128).find(|&f| run[f].0[i][0] >= 0.9) {
                Some(f) => frames.push(f - open + 1),
                None => never += 1,
            }
        }
        frames.sort_unstable();
        let q = |p: usize| frames.get(frames.len() * p / 100).copied().unwrap_or(0);
        println!(
            "reading: moving pillar: {} pixels the shadow left; frames to 0.9 of open: median {}, \
             p90 {}, max {}, within 12 {}; never {never}",
            frames.len(),
            q(50),
            q(90),
            frames.last().copied().unwrap_or(0),
            frames.iter().filter(|&&f| f <= 12).count()
        );
    }

    /// The slab rig's frame-to-frame change in the penumbra band (Plan 0086 Phase 6's
    /// reading): Plan 0032 Phase 5 bar 3's 16 x 16 slab 20 voxels over the floor, the camera 8
    /// voxels up looking straight down, 128 x 128 internal under FSR 2 at 1.5 to 192 x 192 (6
    /// display pixels a voxel), filter on, the defaults otherwise. Under a zenith sun of
    /// `radius_deg` (`Some`), or the headlamp and no sky (`None`), at manual `exposure`: after 64
    /// settle frames, over 64 frames, the mean and 99th percentile of the display's per-pixel
    /// change (the largest channel, levels of 255) over the band within 1.5 voxels of the
    /// slab's +x edge (x 40) along its middle 12 voxels, and the band's mean green level.
    fn penumbra_band(radius_deg: Option<f32>, exposure: f32) -> Option<(f64, f64, f64)> {
        let (device, queue) = device()?;
        let slab = |p: [i32; 3]| {
            p[2] == FLOOR + 20 && (24..40).contains(&p[0]) && (24..40).contains(&p[1])
        };
        let world = floor_world([1, 1, 1], slab);
        let gpu = mirror(&device, &queue, &world, None);
        let cam = Camera {
            position: Vec3::new(40.0, 32.0, FLOOR as f32 + 8.0),
            yaw: FRAC_PI_2,
            pitch: -FRAC_PI_2 + 1e-4,
            vfov: 2.0 * 2f32.atan(),
        };
        let mut f = Frame::with_sizes(&device, [128, 128], [192, 192]);
        f.set_upscaler_kind(
            &device,
            vxe_render::UpscalerKind::Fsr2(vxe_render::Fsr2Settings::default()),
        );
        f.set_lighting(Lighting {
            lamp: radius_deg.is_none(),
            exposure: Exposure::Manual(exposure),
            sky_rays: 0,
            ..Lighting::default()
        });
        f.set_sky(radius_deg.map(|r| {
            let mut s = sky(dir(90.0, 0.0), dir(-90.0, 0.0), [[1.0; 3], [0.0; 3]]);
            s.sun.angular_radius = r.to_radians();
            s
        }));
        let light = LightParams::headlamp();
        for _ in 0..64 {
            f.step(&device, &queue, &gpu, &cam, &light);
        }
        // The band's display pixels: 6 a voxel about the centre (x 40, y 32).
        let band: Vec<usize> = (0..192usize * 192)
            .filter(|&i| {
                let (x, y) = ((i % 192) as f32, (i / 192) as f32);
                let (vx, vy) = (40.0 + (x + 0.5 - 96.0) / 6.0, 32.0 + (y + 0.5 - 96.0) / 6.0);
                (vx - 40.0).abs() <= 1.5 && (26.0..38.0).contains(&vy)
            })
            .collect();
        let read = |f: &Frame| vxe_render::debug::read_rgba8(&device, &queue, f.target());
        let mut last = read(&f);
        let level = band
            .iter()
            .map(|&i| f64::from(last[4 * i + 1]))
            .sum::<f64>()
            / band.len() as f64;
        let mut changes = Vec::new();
        for _ in 0..64 {
            f.step(&device, &queue, &gpu, &cam, &light);
            let now = read(&f);
            for &i in &band {
                let d = (0..3)
                    .map(|k| now[4 * i + k].abs_diff(last[4 * i + k]))
                    .max()
                    .unwrap_or(0);
                changes.push(f64::from(d));
            }
            last = now;
        }
        changes.sort_by(f64::total_cmp);
        let mean = changes.iter().sum::<f64>() / changes.len() as f64;
        Some((mean, changes[changes.len() * 99 / 100], level))
    }

    /// Plan 0086 Phase 6's reading: the penumbra band's change at 3 degrees (and 5) against
    /// the lamp's on the same floor, each exposure pinned so the band's mean level is near 118
    /// (two passes, the second scaled by the display's gamma of 2.2). Printed.
    #[test]
    #[ignore = "reading"]
    fn the_penumbra_against_the_lamp() {
        for r in [Some(3.0), Some(5.0), None] {
            let mut ex = 1.0f32;
            let Some((_, _, level)) = penumbra_band(r, ex) else {
                return;
            };
            ex *= ((118.0 / level.max(1.0)) as f32).powf(2.2);
            let Some((mean, p99, level)) = penumbra_band(r, ex) else {
                return;
            };
            let what = r.map_or("the headlamp, no sky".to_string(), |d| {
                format!("sun {d} degrees")
            });
            println!(
                "reading: penumbra band, {what}: exposure {ex:.4}, mean level {level:.1}, change mean {mean:.3}, p99 {p99}"
            );
        }
    }

    /// The key light's visibility across the slab rig's +x edge (`penumbra_band`'s slab and
    /// camera, native 128 x 128, the lighting at full resolution, the filter off, exposure 1,
    /// no sky rays) under a zenith sun of `radius_deg`: per bin of a quarter voxel in x over
    /// 36..46 (the lighting hits with y in 30..34), the light pass's direct sample over the open
    /// floor's (x 46..50), with `penumbra`; the disc's as a mean over 64 frames, the approach's
    /// at frame 1, and the approach's largest change of a bin from frame 1 to frame 2.
    fn edge_profile(radius_deg: f32, penumbra: vxe_render::Penumbra) -> Option<(Vec<f64>, f64)> {
        let (device, queue) = device()?;
        let slab = |p: [i32; 3]| {
            p[2] == FLOOR + 20 && (24..40).contains(&p[0]) && (24..40).contains(&p[1])
        };
        let world = floor_world([1, 1, 1], slab);
        let gpu = mirror(&device, &queue, &world, None);
        let cam = Camera {
            position: Vec3::new(40.0, 32.0, FLOOR as f32 + 8.0),
            yaw: FRAC_PI_2,
            pitch: -FRAC_PI_2 + 1e-4,
            vfov: 2.0 * 2f32.atan(),
        };
        let mut f = frame(&device, [128, 128], ShadowWalk::Coarse);
        f.set_lighting(Lighting {
            penumbra,
            ..f.lighting()
        });
        let mut s = sky(dir(90.0, 0.0), dir(-90.0, 0.0), [[1.0; 3], [0.0; 3]]);
        s.sun.angular_radius = radius_deg.to_radians();
        f.set_sky(Some(s));
        const BINS: usize = 40;
        let frames = if penumbra == vxe_render::Penumbra::Disc {
            64
        } else {
            2
        };
        let mut runs = Vec::new();
        let light = LightParams::headlamp();
        for _ in 0..frames {
            f.step(&device, &queue, &gpu, &cam, &light);
            let d = vxe_render::debug::read_rgba16f(&device, &queue, f.direct());
            let h = vxe_render::debug::read_rgba32f(&device, &queue, f.light_hit_positions());
            let (mut sum, mut n) = ([0.0f64; BINS], [0u32; BINS]);
            let (mut open, mut on) = (0.0f64, 0u32);
            for (p, q) in h.iter().zip(&d) {
                if p[3] > 1.0e30 || !(30.0..34.0).contains(&p[1]) {
                    continue;
                }
                if (46.0..50.0).contains(&p[0]) {
                    open += f64::from(q[3]);
                    on += 1;
                }
                let b = ((p[0] - 36.0) * 4.0).floor();
                if (0.0..BINS as f32).contains(&b) {
                    sum[b as usize] += f64::from(q[3]);
                    n[b as usize] += 1;
                }
            }
            let open = open / f64::from(on.max(1));
            runs.push(
                (0..BINS)
                    .map(|i| sum[i] / f64::from(n[i].max(1)) / open.max(1e-9))
                    .collect::<Vec<f64>>(),
            );
        }
        let mean: Vec<f64> = (0..BINS)
            .map(|i| runs.iter().map(|r| r[i]).sum::<f64>() / runs.len() as f64)
            .collect();
        if penumbra == vxe_render::Penumbra::Disc {
            return Some((mean, 0.0));
        }
        let change = (0..BINS)
            .map(|i| (runs[1][i] - runs[0][i]).abs())
            .fold(0.0, f64::max);
        Some((runs[0].clone(), change))
    }

    /// The first x (voxels) at which `profile` (bins of a quarter voxel from x 36) reaches
    /// `level`, linear between bins' centres.
    fn crossing(profile: &[f64], level: f64) -> Option<f64> {
        let x = |i: usize| 36.0 + (i as f64 + 0.5) / 4.0;
        (1..profile.len()).find_map(|i| {
            let (a, b) = (profile[i - 1], profile[i]);
            (a < level && b >= level).then(|| x(i - 1) + (level - a) / (b - a) * 0.25)
        })
    }

    /// Plan 0086 Phase 7 bar 1: the approach penumbra's profile across the slab's edge at one
    /// frame against the disc's 64-frame mean, at 5 and 3 degrees: the 5 %, 50 % and 95 %
    /// crossings of each, the rise's width (5 % to 95 %), the largest gap between the profiles
    /// and the approach's change from frame 1 to 2. Printed; lavapipe.
    #[test]
    #[ignore = "reading"]
    fn approach_penumbra_against_the_disc() {
        use vxe_render::Penumbra;
        for r in [5.0, 3.0] {
            let Some((disc, _)) = edge_profile(r, Penumbra::Disc) else {
                return;
            };
            let Some((approach, change)) = edge_profile(r, Penumbra::Approach) else {
                return;
            };
            for (name, p) in [("disc", &disc), ("approach", &approach)] {
                let c: Vec<String> = [0.05, 0.5, 0.95]
                    .iter()
                    .map(|&l| crossing(p, l).map_or("-".into(), |x| format!("{x:.2}")))
                    .collect();
                let width = crossing(p, 0.05)
                    .zip(crossing(p, 0.95))
                    .map_or("-".into(), |(a, b)| format!("{:.2}", b - a));
                println!(
                    "reading: edge at {r} degrees, {name}: crossings 5/50/95 % at x {}, width {width}",
                    c.join(" / ")
                );
            }
            let gap = disc
                .iter()
                .zip(&approach)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0, f64::max);
            let row = |p: &[f64]| {
                p.iter()
                    .step_by(2)
                    .map(|v| format!("{v:.2}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            println!(
                "reading: edge at {r} degrees: largest gap {gap:.3}, approach change frame 1 to 2 {change:.4}\n  disc     {}\n  approach {}",
                row(&disc),
                row(&approach)
            );
        }
    }
}
