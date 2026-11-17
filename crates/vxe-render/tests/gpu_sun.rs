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

/// The floor's centre direct value under `s` at `rays` sky rays, a 16-frame mean, through a
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
    centre_direct(device, queue, &mut f, &gpu, &cam, 16)
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
/// 64-frame mean of the direct target at the centre pixel of `cam` over `world`.
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
    centre_direct(device, queue, &mut f, &gpu, cam, 64)
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
}
