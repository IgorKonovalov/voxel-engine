//! The sun and the moon as the light pass's directional sample (ADR-0027): a floor of rock
//! under a key light, read from the light pass's direct target, and an absent chunk read as
//! sky by the lighting rays. Lavapipe, lamp off, no list
//! light, no sky ray, filter off, manual exposure 1.0, the full shadow walk, lighting at full
//! resolution.
//! The image bars (the penumbra, the settle after a jump, the upscaler's mask) are readings in
//! `mod reading` until the owner's window.

use std::f32::consts::FRAC_PI_2;

use svet_render::sky::{Body, SUN_ANGULAR_RADIUS, Sky};
use svet_render::{
    Camera, Exposure, Frame, GpuWorld, LightParams, Lighting, LightingResolution, MirrorConfig,
    ShadowWalk, Window, glam::Vec3, wgpu,
};
use svet_world::*;

const ROCK: MaterialId = 1;
/// The floor's top: rock below voxel z 16, so the lit plane is z = 16, altitude 0.
const FLOOR: i32 = 16;
const SIZE: [u32; 2] = [64, 36];
/// The transmittance from altitude 0 to the zenith, `exp(-(0.0614, 0.1420, 0.2714))`.
const T_ZENITH: [f32; 3] = [0.9404, 0.8677, 0.7623];

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    svet_render::test_support::any_device(32 << 20, wgpu::Features::empty())
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

/// The rigs' frame: full-resolution lighting, the lamp and the filter off, the exposure 1, no
/// sky ray, and the light history on with the key light's far baked and blended: the
/// mechanisms these tests read (the running means, the key ray's penumbra, a record's fade)
/// ride the light history, which is off by default (ADR-0082). A test
/// of another configuration sets it over this one.
fn frame(device: &wgpu::Device, size: [u32; 2], walk: ShadowWalk) -> Frame {
    let mut frame = Frame::new(device, size[0], size[1]);
    frame.set_resolution(device, LightingResolution::Full);
    frame.set_lighting(Lighting {
        lamp: false,
        filter: false,
        shadow_walk: walk,
        exposure: Exposure::Manual(1.0),
        sky_rays: 0,
        light_history: true,
        key_far: svet_render::KeyFar::Baked,
        key_far_mix: svet_render::KeyFarMix::Blend,
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
        let d = svet_render::debug::read_rgba16f(device, queue, frame.direct_accumulated());
        let w = frame.direct_accumulated().width() as usize;
        let h = frame.direct_accumulated().height() as usize;
        let c = d[(h / 2) * w + w / 2];
        for k in 0..3 {
            sum[k] += f64::from(c[k]);
        }
    }
    sum.map(|v| (v / f64::from(frames)) as f32)
}

/// The light at the centre pixel, mean over `frames` frames: the temporal pass's accumulated
/// direct part's rgb (the key light's), plus the accumulated stored part's (at a cube face the
/// baked visibility's sky term, ADR-0070; no emissive light here), plus the visibility pair's
/// sky term where the sky ray was cast this frame (`pair.x * E_sky(+z) * exposure`: the
/// filter's input and the composite add it, not the direct part; every caller's centre pixel
/// is a +z floor face at manual exposure 1).
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
        let d = svet_render::debug::read_rgba16f(device, queue, frame.direct_accumulated());
        let a = svet_render::debug::read_rgba16f(device, queue, frame.accumulated());
        let i = (h / 2) * w + w / 2;
        let pair = pair_sky_term(device, queue, frame, i);
        for k in 0..3 {
            sum[k] += f64::from(d[i][k]) + f64::from(a[i][k]) + f64::from(pair[k]);
        }
    }
    sum.map(|v| (v / f64::from(frames)) as f32)
}

/// The visibility pair's sky term at lighting pixel `i` of the last encoded frame, for a +z
/// face at manual exposure 1: `pair.x * E_sky(+z)` where the visibility sample says the sky ray
/// was cast, else 0 (`sky_pair_term` in sky.wgsl).
fn pair_sky_term(device: &wgpu::Device, queue: &wgpu::Queue, frame: &Frame, i: usize) -> [f32; 3] {
    let vis = svet_render::debug::read_rgba8(device, queue, frame.visibility_samples());
    if vis[4 * i + 3] <= 127 || frame.sky().is_none() {
        return [0.0; 3];
    }
    let pair = svet_render::debug::read_rgba16f(device, queue, frame.visibility_pair())[i];
    let bytes = svet_render::debug::read_buffer(device, queue, frame.sky_irradiance(), 0, 96);
    let e = |k: usize| {
        let b = &bytes[4 * k..4 * k + 4];
        f32::from_le_bytes([b[0], b[1], b[2], b[3]])
    };
    [e(20), e(21), e(22)].map(|c| pair[0] * c)
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

/// The roof of [`a_face_with_no_record_reads_its_own_summary_or_the_column`]: a one-voxel slab
/// [`ROOF_UP`] voxels over the floor, over the covered half of the centre chunk (x 64..96,
/// y 64..128).
const ROOF_UP: i32 = 4;
fn under_roof(p: [i32; 3]) -> bool {
    p[2] == FLOOR + ROOF_UP && (64..96).contains(&p[0]) && (64..128).contains(&p[1])
}

/// The sky term's source word in a moments' `w` with the store counted (temporal.wgsl's
/// `SOURCE_*`: the word over 16 added to the kind).
fn source_of(w: f32) -> u8 {
    ((w - w.floor()) * 16.0).round() as u8
}
const SOURCE_OWN_SUMMARY: u8 = 3;

/// Plan 0094 Phase 5 bar 1 (ADR-0070's no-record prior; this rig is Plan 0086 Phase 10 bar 2's,
/// whose neighbour rule it replaces): a no-record face reads no neighbour. An open floor of
/// 3 x 3 chunks under `Sky::uniform(1.0)`, the key light off, one sky ray, a roof
/// ([`under_roof`]) over half the centre chunk, baked at 16 samples a brick direction with the
/// centre chunk's faces withheld, so its four open neighbours' summaries read about 1 (what the
/// neighbour rule gave, printed as the before) and the column map reads 0 under the roof and 1
/// beside it. A camera low in the open half looks back under the roof (160 x 90); the read
/// pixels are the floor's under the roof at least 8 voxels from its edges, where the column
/// reads 0. (a) No summary for the centre: after 16 frames their mean visibility (the light
/// over `pi`) is within `1 / (SEED_AGE + 1)` of the column's 0 under arm A (the ray, seeded
/// from the column) and exactly 0 under arm B (`LightHooks::column_prior`). (b) The covered
/// half's faces alone withheld, so the centre's summary holds the open half's records: every
/// read pixel's source word is `own summary` and their mean light is the summary's `+z`
/// visibility times `pi` within 10 %.
#[test]
fn a_face_with_no_record_reads_its_own_summary_or_the_column() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = floor_world([3, 3, 1], under_roof);
    let mut gpu = mirror(&device, &queue, &world, None);
    let uniform = Sky {
        altitude_zero_z: FLOOR as f32,
        ..Sky::uniform(1.0)
    };
    let cam = Camera {
        position: Vec3::new(102.0, 96.0, FLOOR as f32 + 2.5),
        yaw: std::f32::consts::PI,
        pitch: -12f32.to_radians(),
        vfov: 60f32.to_radians(),
    };
    let pi = std::f32::consts::PI;
    let summary_up = |f: &Frame, c: [i32; 2]| -> Option<f32> {
        let block = f.store().block_words(ChunkCoord([c[0], c[1], 0]))?;
        let at = svet_render::internals::SUMMARY_AT as usize;
        let w = svet_render::internals::BRICK_WORDS;
        svet_render::internals::record_visibility(&block[at..at + w], 5)
    };
    // Bakes `world` with the centre chunk's voxels that `withheld` names left out, then per
    // arm (B with `column_prior`) steps `frames` frames after a reset and reads, over the
    // read pixels, the mean light over `pi` and the source words.
    let mut f = frame(&device, DIG_SIZE, ShadowWalk::Full);
    f.set_lighting(Lighting {
        sky_rays: 1,
        ..f.lighting()
    });
    f.set_sky(Some(uniform));
    f.set_count_store(true);
    let mut run = |withheld: fn([i32; 3]) -> bool, frames: u32| {
        let settings = svet_render::BakeSettings {
            brick_samples: 16,
            round: 16,
            filter: Some(std::sync::Arc::new(move |v: [i32; 3], _f: u8| {
                !(v[0].div_euclid(64) == 1 && v[1].div_euclid(64) == 1 && withheld(v))
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
        let neighbours: Vec<Option<f32>> = [[0, 1], [2, 1], [1, 0], [1, 2]]
            .iter()
            .map(|&c| summary_up(&f, c))
            .collect();
        let own = summary_up(&f, [1, 1]);
        let mut arms = Vec::new();
        for column_prior in [false, true] {
            f.set_light_hooks(svet_render::LightHooks {
                key_off: true,
                column_prior,
                ..Default::default()
            });
            f.reset_history();
            for _ in 0..frames {
                f.step(&device, &queue, &gpu, &cam, &LightParams::headlamp());
            }
            let hits = svet_render::debug::read_rgba32f(&device, &queue, f.light_hit_positions());
            let d = svet_render::debug::read_rgba16f(&device, &queue, f.direct_accumulated());
            let a = svet_render::debug::read_rgba16f(&device, &queue, f.accumulated());
            let m = svet_render::debug::read_rgba16f(&device, &queue, f.moments());
            let vis = svet_render::debug::read_rgba8(&device, &queue, f.visibility_samples());
            let pair = svet_render::debug::read_rgba16f(&device, &queue, f.visibility_pair());
            // `pair_sky_term`'s E_sky(+z), green.
            let bytes = svet_render::debug::read_buffer(&device, &queue, f.sky_irradiance(), 0, 96);
            let e_up = f32::from_le_bytes([bytes[84], bytes[85], bytes[86], bytes[87]]);
            let (mut sum, mut n, mut words) = (0.0f64, 0usize, [0usize; 7]);
            for (i, h) in hits.iter().enumerate() {
                let deep = h[3] < 1e30
                    && (h[2] - FLOOR as f32).abs() < 1e-3
                    && (72.0..88.0).contains(&h[0])
                    && (72.0..120.0).contains(&h[1]);
                if !deep {
                    continue;
                }
                let cast = vis[4 * i + 3] > 127;
                let ray = if cast { pair[i][0] * e_up } else { 0.0 };
                let lit = f64::from(d[i][1]) + f64::from(a[i][1]) + f64::from(ray);
                sum += lit / f64::from(pi);
                n += 1;
                words[usize::from(source_of(m[i][3])).min(6)] += 1;
            }
            arms.push((sum / n.max(1) as f64, n, words));
        }
        (neighbours, own, arms)
    };
    let (neighbours, own, arms) = run(|_| true, 16);
    println!(
        "(a) no centre summary (own {own:?}); the neighbours' +z summaries (the before) {neighbours:?}; \
         under the roof: arm A mean {:.4} over {} px, words {:?}; arm B mean {:.4} over {} px, words {:?}",
        arms[0].0, arms[0].1, arms[0].2, arms[1].0, arms[1].1, arms[1].2
    );
    assert!(own.is_none(), "the centre holds a summary: {own:?}");
    assert!(
        arms[0].1 > 100 && arms[1].1 > 100,
        "rig fault: few read pixels"
    );
    assert!(
        arms[0].0 <= 1.0 / 9.0,
        "arm A: {} against the column's 0",
        arms[0].0
    );
    assert!(
        arms[1].0 <= 1e-4,
        "arm B: {} against the column's 0",
        arms[1].0
    );
    let (_, own, arms) = run(|v| v[0] < 96, 4);
    let own = own.expect("the centre's open half summarises");
    println!(
        "(b) the centre's summary {own:.4}: under the roof arm A mean {:.4} over {} px, words {:?}",
        arms[0].0, arms[0].1, arms[0].2
    );
    assert!(arms[0].1 > 100, "rig fault: few read pixels");
    assert_eq!(
        arms[0].2[usize::from(SOURCE_OWN_SUMMARY)],
        arms[0].1,
        "{:?}",
        arms[0].2
    );
    assert!(
        (arms[0].0 / f64::from(own) - 1.0).abs() <= 0.10,
        "{} against the summary's {own}",
        arms[0].0
    );
}

/// Plan 0086 Phase 9 bar 4's twin: the floor of chunk (0, 0, 0) under a resident rock roof
/// (chunks (0, 0, 1) and (1, 0, 1)), the chunk beside the floor's, (1, 0, 0), held by the
/// window and absent from the world. Under `Sky::uniform(1.0)` with one sky ray and the key
/// light off, a sky ray into the absent chunk ends on its face below the roof, so it is rock
/// by the column map (ADR-0070's amendment): the floor reads under 1 % of `pi`. The same rig
/// with every end counted as sky (`LightHooks::sky_end_is_sky`) is printed. 16-frame means.
/// Soak: 11.0 s on lavapipe in Plan 0086 Phase 24i's quiet gate.
#[test]
#[ignore = "soak"]
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
    let read = |hooks: svet_render::LightHooks| {
        let mut fr = frame(&device, SIZE, ShadowWalk::Full);
        fr.set_lighting(Lighting {
            sky_rays: 1,
            ..fr.lighting()
        });
        fr.set_light_hooks(svet_render::LightHooks {
            key_off: true,
            ..hooks
        });
        fr.set_sky(Some(uniform));
        let lit = centre_lit(&device, &queue, &mut fr, &gpu, &cam, 16);
        let pair = centre_pair(&device, &queue, &fr);
        println!(
            "the centre pixel's visibility pair at the last frame: mean {:.4} at age {}",
            pair[0], pair[3]
        );
        lit
    };
    let rule = read(Default::default());
    let as_built = read(svet_render::LightHooks {
        sky_end_is_sky: true,
        ..Default::default()
    });
    println!(
        "under a roof beside an absent chunk: column rule {rule:?}, every end sky {as_built:?}"
    );
    let bar = 0.01 * std::f32::consts::PI;
    assert!(rule.iter().all(|&v| v < bar), "{rule:?} against {bar}");
}

/// The column map texture's texels for a window `tiles` chunks wide in x and y: the map's
/// `COLUMN_CELLS` per tile on each axis.
fn column_map_texels(tiles: [i32; 2]) -> usize {
    let cells = svet_render::COLUMN_CELLS;
    (tiles[0] * cells) as usize * (tiles[1] * cells) as usize
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
    let side = 3 * svet_render::COLUMN_CELLS;
    let read = |gpu: &GpuWorld| {
        let map = svet_render::debug::read_r32i(&device, &queue, gpu.column_map());
        assert_eq!(gpu.column_tiles(), [3, 3], "the window's tiles");
        assert_eq!(
            map.len(),
            column_map_texels(gpu.column_tiles()),
            "the map's rows"
        );
        map
    };
    let map = read(&gpu);
    let mut checked = [0u32; 3];
    for j in 0..side {
        for i in 0..side {
            let got = map[(i + side * j) as usize];
            let cols = [[0, 0], [1, 0], [0, 1], [1, 1]].map(|o| [2 * i + o[0], 2 * j + o[1]]);
            if cols.iter().all(|c| shaft(c[0], c[1])) {
                assert_eq!(got, svet_render::COLUMN_NONE, "the shaft's cell {i} {j}");
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

/// `column_top_max` is the column map's maximum. Read back after each sync, it equals the
/// largest texel of the map's tiles: after the first sync, after a dig and a stamp, and at
/// every sync of a following window's move across a 6 x 6 x 3-chunk terrain.
#[test]
fn the_column_top_max_is_the_maps_maximum() {
    let Some((device, queue)) = device() else {
        return;
    };
    let h = |x: i32, y: i32| 20 + (x * 7 + y * 13).rem_euclid(37) + 40 * ((x / 64 + y / 64) % 2);
    let f = move |p: [i32; 3]| if p[2] < h(p[0], p[1]) { ROCK } else { AIR };
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [6, 6, 3]),
        &FnSource(f),
        &table(),
    );
    let mut gpu = GpuWorld::with_config(&device, MirrorConfig::new(Window::Follow { side: 3 }));
    gpu.set_focus(VoxelCoord([96, 96, 64]));
    let sync = |gpu: &mut GpuWorld| {
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        gpu.sync(&world, &queue, &mut enc).expect("the mirror fits");
        queue.submit([enc.finish()]);
    };
    let check = |gpu: &GpuWorld, when: &str| -> i32 {
        let map = svet_render::debug::read_r32i(&device, &queue, gpu.column_map());
        let tiles = gpu.column_tiles();
        assert_eq!(
            map.len(),
            column_map_texels(tiles),
            "{when}: the texture's texels"
        );
        let top = map
            .iter()
            .copied()
            .max()
            .unwrap_or(svet_render::COLUMN_NONE);
        assert_eq!(
            gpu.column_top_max(),
            top,
            "{when}: the window's highest top"
        );
        top
    };
    sync(&mut gpu);
    let first = check(&gpu, "first sync");
    assert!(first > 0, "the terrain tops the window: {first}");
    let centre = |v: [i32; 3]| centre_of_voxel(v);
    let (x, y) = (100, 100);
    let top = h(x, y) - 1;
    world
        .apply(&EditOp::box_(
            centre([x - 8, y - 8, top - 6]),
            centre([x + 8, y + 8, top + 1]),
            Mode::Carve,
        ))
        .expect("the dig");
    sync(&mut gpu);
    check(&gpu, "the dig");
    world
        .apply(&EditOp::box_(
            centre([x, y, 150]),
            centre([x + 1, y + 1, 151]),
            Mode::Fill(ROCK),
        ))
        .expect("the stamp");
    sync(&mut gpu);
    assert_eq!(
        check(&gpu, "the stamp"),
        150,
        "the stamp (voxel z 150) tops the window"
    );
    gpu.set_focus(VoxelCoord([300, 280, 64]));
    for i in 0..4 {
        sync(&mut gpu);
        check(&gpu, &format!("the move, sync {i}"));
    }
    println!("column map: {} bytes", gpu.column_map_bytes());
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
        exposure: Exposure::Auto(svet_render::AutoExposure::instant()),
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
        let texels = svet_render::debug::read_rgba16f(&device, &queue, t);
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
        let d = svet_render::debug::read_rgba16f(&device, &queue, f.direct_accumulated());
        let p = svet_render::debug::read_rgba32f(&device, &queue, f.hit_positions());
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
    let before = svet_render::debug::read_rgba16f(&device, &queue, f.direct_accumulated());
    f.set_sky(Some(at(60.0)));
    let mut reads = Vec::new();
    for _ in 0..11 {
        f.step(&device, &queue, &gpu, &cam, &light);
        reads.push(svet_render::debug::read_rgba16f(
            &device,
            &queue,
            f.direct_accumulated(),
        ));
    }
    for _ in 0..32 {
        f.step(&device, &queue, &gpu, &cam, &light);
    }
    let settled = svet_render::debug::read_rgba16f(&device, &queue, f.direct_accumulated());
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
        svet_render::debug::read_rgba8(&device, &queue, f.fsr2().unwrap().reactive())
            .chunks(4)
            .map(|p| p[0])
            .collect()
    };
    let settled = || {
        let mut f = frame(&device, [320, 180], ShadowWalk::Coarse);
        f.set_upscaler_kind(
            &device,
            svet_render::UpscalerKind::Fsr2(svet_render::Fsr2Settings::default()),
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
    let d = svet_render::debug::read_rgba16f(&device, &queue, f.direct_accumulated());
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
            svet_render::UpscalerKind::Fsr2(svet_render::Fsr2Settings::default()),
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
    let read = |f: &Frame| svet_render::debug::read_rgba8(device, queue, f.target());
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
    let rays = svet_render::DEFAULT_SKY_RAYS;
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
    let lit = centre_lit(device, queue, &mut f, &gpu, cam, 64);
    println!(
        "the centre pixel's visibility pair at the last frame: mean {:.4} at age {}",
        centre_pair(device, queue, &f)[0],
        centre_pair(device, queue, &f)[3]
    );
    lit
}

/// The visibility pair at the centre lighting pixel of `f`'s last frame.
fn centre_pair(device: &wgpu::Device, queue: &wgpu::Queue, f: &Frame) -> [f32; 4] {
    let w = f.visibility_pair().width() as usize;
    let h = f.visibility_pair().height() as usize;
    svet_render::debug::read_rgba16f(device, queue, f.visibility_pair())[(h / 2) * w + w / 2]
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

// The readings' module, `reading`, kept in its own file for the file-size cap.
include!("gpu_sun/reading.rs");
include!("gpu_sun/dig.rs");
include!("gpu_sun/history.rs");
include!("gpu_sun/key.rs");
include!("gpu_sun/rebake.rs");
include!("gpu_sun/shaft.rs");

/// The mean `+z` sky visibility (face 5) of chunk (1, 1, 0)'s records in `block` whose brick
/// lies under `over`'s footprint (read at the slab's height).
fn up_visibility_under(block: &[u32], over: impl Fn([i32; 3]) -> bool) -> f32 {
    let w = svet_render::internals::BRICK_WORDS;
    let (mut sum, mut n) = (0.0, 0u32);
    for slot in 0..512usize {
        let dir = block[slot];
        if dir == 0 {
            continue;
        }
        let rec = &block[(dir >> 11) as usize - w..(dir >> 11) as usize];
        let b = brick_of_slot(slot);
        let at = [64 + b[0] * 8 + 4, 64 + b[1] * 8 + 4, 201];
        if over(at)
            && let Some(v) = svet_render::internals::record_visibility(rec, 5)
        {
            sum += v;
            n += 1;
        }
    }
    sum / n.max(1) as f32
}

/// The live queue on `world` under `Sky::uniform(1.0)` with the column wait `wait`, the light
/// radius 1 chunk, 64 samples a brick direction, no coarse pass, every chunk the world holds
/// (and every one outside `extent`) resident full and every chunk of the window wanted by the
/// streamer (`lod_radius` 8 about the
/// floor's centre): stepped until `until` says so or 600 steps, `between` run after each step.
fn caught_up_queue(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    world: &World,
    extent: ChunkExtent,
    wait: bool,
    mut between: impl FnMut(usize, &mut GpuWorld, &mut Frame) -> bool,
) -> (GpuWorld, Frame) {
    let mut gpu = GpuWorld::with_config(
        device,
        MirrorConfig {
            light_radius: 1,
            ..MirrorConfig::new(Window::Fixed(extent))
        },
    );
    let mut f = frame(device, SIZE, ShadowWalk::Full);
    f.set_lighting(Lighting {
        column_wait: wait,
        ..f.lighting()
    });
    f.set_sky(Some(Sky {
        altitude_zero_z: FLOOR as f32,
        ..Sky::uniform(1.0)
    }));
    let settings = svet_render::BakeSettings {
        brick_samples: 64,
        round: 64,
        ..Default::default()
    };
    f.start_bake_queue(queue, &gpu, settings, 1.0e9);
    f.set_bake_coarse(0, 0);
    let eye = [96.0, 96.0, 32.0];
    for i in 0..600 {
        let mut enc = device.create_command_encoder(&Default::default());
        gpu.sync(world, queue, &mut enc).expect("the mirror fits");
        queue.submit([enc.finish()]);
        // Every chunk the world holds now is resident full, and one outside the extent too (the
        // world's edge: nothing to wait for there): a snapshot taken each step, as a chunk may
        // arrive between steps.
        let held: std::collections::HashSet<ChunkCoord> =
            world.store().coords().into_iter().collect();
        let resident =
            svet_render::Residency::of_rule(move |c| held.contains(&c) || !extent.contains(c));
        let view = svet_render::QueueView {
            eye,
            forward: [0.0, 1.0, 0.0],
            resident: Some(resident),
            samples: None,
            interval_s: 1.0 / 60.0,
            hold: None,
            half_angle: 1.0,
            frustum: None,
            full_radius: Some(8),
            lod_radius: Some(8),
            focal_px: None,
        };
        f.bake_queue_step(device, queue, &mut gpu, world, &view)
            .expect("the queue runs")
            .expect("the queue's step");
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("device poll");
        if between(i, &mut gpu, &mut f) {
            break;
        }
    }
    (gpu, f)
}

/// Plan 0086 Phase 18 bar 3: a provisional bake is caught up. A floor of 3 x 3 chunks over a row
/// of rock (chunk z -1) under a rock slab in the top row (chunk row z 3, voxels 200..204 over x
/// and y 40..150), 3 x 3 x 5 chunks, mirrored
/// over the whole extent with the light radius 1 chunk (the slab's chunks lie outside the floor
/// chunk's reach and inside its columns), the slab's chunk (1, 1, 3) held back from the world
/// and so from the mirror. With the column wait off the floor chunk (1, 1, 0) bakes before the
/// slab arrives, its block provisional; once the slab's chunk is inserted and mirrored, the
/// queue re-bakes it once, and its record then equals bit for bit the same world's baked with
/// the wait on and the slab present from the start. The provisional block's `+z` visibility
/// under the slab is printed (the leak the catch-up closes).
#[test]
fn a_provisional_bake_is_caught_up() {
    let Some((device, queue)) = device() else {
        return;
    };
    let slab = |p: [i32; 3]| {
        (200..204).contains(&p[2]) && (40..150).contains(&p[0]) && (40..150).contains(&p[1])
    };
    // A chunk row of rock under the floor (z -1), so the floor chunk's reach is resident.
    let extent = ChunkExtent::new([0, 0, -1], [3, 3, 5]);
    let world = || {
        let f = move |p: [i32; 3]| if p[2] < FLOOR || slab(p) { ROCK } else { AIR };
        World::generate(extent, &FnSource(f), &table())
    };
    let full = world();
    let (floor, high) = (ChunkCoord([1, 1, 0]), ChunkCoord([1, 1, 3]));
    let (_, want) = caught_up_queue(&device, &queue, &full, extent, true, |_, _, f| {
        f.store().block_words(floor).is_some()
            && f.bake_queue().is_some_and(|q| q.tracks(floor).is_empty())
    });
    if let Some(q) = want.bake_queue() {
        let st = q.stats();
        println!(
            "the wait on: baked {}, depth {}, jobs {}, waiting reach {}, the floor's tracks {:?}",
            st.baked,
            st.depth,
            st.jobs,
            st.waiting_reach,
            q.tracks(floor)
        );
    }
    let want = want.store().block_words(floor).expect("baked").to_vec();
    let held = world();
    let slab_chunk = held.store().remove(high).expect("generated");
    let slab_chunk = slab_chunk.read().expect("lock").clone();
    let mut provisional = None;
    let mut arrived = None;
    let (mut due, mut was_due, mut requeued) = (0u32, false, 0u64);
    let (_, got) = caught_up_queue(&device, &queue, &held, extent, false, |i, _, f| {
        let q = f.bake_queue().expect("the queue");
        let tracks = q.tracks(floor);
        if arrived.is_none() {
            if tracks.contains(&"provisional") {
                provisional = f.store().block_words(floor).map(<[u32]>::to_vec);
                held.store().insert(high, slab_chunk.clone());
                arrived = Some(i);
            }
            return false;
        }
        let is_due = tracks.contains(&"provisional_due");
        due += u32::from(is_due && !was_due);
        was_due = is_due;
        requeued = q.stats().provisional_requeued;
        due > 0
            && !tracks
                .iter()
                .any(|t| t.starts_with("provisional") || *t == "job_provisional")
            && f.store().block_words(floor).is_some()
    });
    let arrived = arrived.expect("the floor bakes before the slab arrives");
    let provisional = provisional.expect("a provisional block");
    let got = got.store().block_words(floor).expect("baked").to_vec();
    println!(
        "the slab arrived at step {arrived}; the floor re-queued {due} times ({requeued} catch-ups over every chunk); the provisional block differs in {} words of {}; mean +z sky visibility under the slab: provisional {:.4}, caught up {:.4}, the wait's {:.4}",
        provisional
            .iter()
            .zip(&want)
            .filter(|(a, b)| a != b)
            .count(),
        want.len(),
        up_visibility_under(&provisional, slab),
        up_visibility_under(&got, slab),
        up_visibility_under(&want, slab),
    );
    assert_eq!(due, 1, "one catch-up of the floor");
    assert_eq!(got, want, "the end state is the wait's");
}

/// Plan 0086 Phase 19 bar 1 (Phase 6 bar 2): the key light's visibility on the pair's running
/// mean holds the penumbra still. `reading::penumbra_band_pinned` (the 16 x 16 slab 20 voxels
/// over the floor, FSR 2 at 1.5 over 128 x 128, each run's exposure pinned so the band's mean
/// level is near 118): after 64 frames, the p99 frame-to-frame change over the penumbra band's
/// display pixels under a zenith sun of 3 degrees is at or under 4 levels (ADR-0076's starting
/// `T_still`, the bar as re-stated 2026-11-18; the reading of record p99 2, mean 0.355), the
/// headlamp's on the same floor with no sky printed beside it (p99 1, mean 0.076).
/// Soak: 24.8 s on lavapipe in Plan 0086 Phase 24i's quiet gate.
#[test]
#[ignore = "soak"]
fn the_penumbra_holds_still_on_the_long_history() {
    let Some((ex_sun, mean_sun, p99_sun, level_sun)) = reading::penumbra_band_pinned(Some(3.0))
    else {
        return;
    };
    let Some((ex_lamp, mean_lamp, p99_lamp, level_lamp)) = reading::penumbra_band_pinned(None)
    else {
        return;
    };
    println!(
        "penumbra band, sun 3 degrees: exposure {ex_sun:.4}, level {level_sun:.1}, change mean {mean_sun:.3}, p99 {p99_sun}; the headlamp: exposure {ex_lamp:.4}, level {level_lamp:.1}, change mean {mean_lamp:.3}, p99 {p99_lamp}"
    );
    assert!(
        p99_sun <= 4.0,
        "p99 {p99_sun} against T_still 4 (the lamp's {p99_lamp})"
    );
}

/// Plan 0086 Phase 19 bar 6: `Frame::reset_history` between two `Frame::step` calls restarts
/// every pixel. The open floor under the noon sun, 8 frames stepped (every hit pixel's age 7),
/// then a reset: the next step's moments read age 0 at every pixel, and `history_restarts`
/// counts the reset; the step after reads age 1.
#[test]
fn a_reset_between_steps_restarts_the_history() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = floor_world([2, 2, 1], |_| false);
    let gpu = mirror(&device, &queue, &world, None);
    let cam = camera([64.0, 40.0]);
    let mut f = frame(&device, SIZE, ShadowWalk::Full);
    f.set_sky(Some(sky(
        dir(90.0, 0.0),
        dir(-45.0, 0.0),
        [[1.0; 3], [0.0; 3]],
    )));
    let ages = |f: &Frame| -> Vec<f32> {
        svet_render::debug::read_rgba16f(&device, &queue, f.moments())
            .iter()
            .map(|m| m[2])
            .collect()
    };
    let light = LightParams::headlamp();
    for _ in 0..8 {
        f.step(&device, &queue, &gpu, &cam, &light);
    }
    let before = ages(&f);
    let centre = (SIZE[1] as usize / 2) * SIZE[0] as usize + SIZE[0] as usize / 2;
    println!("centre age before the reset: {}", before[centre]);
    assert_eq!(before[centre], 7.0, "the history runs before the reset");
    let restarts = f.history_restarts();
    f.reset_history();
    f.step(&device, &queue, &gpu, &cam, &light);
    let after = ages(&f);
    let max = after.iter().copied().fold(0.0f32, f32::max);
    println!("largest age after the reset: {max}");
    assert!(after.iter().all(|&a| a == 0.0), "largest age {max}");
    assert_eq!(f.history_restarts(), restarts + 1);
    f.step(&device, &queue, &gpu, &cam, &light);
    assert_eq!(ages(&f)[centre], 1.0, "the history runs again");
}
