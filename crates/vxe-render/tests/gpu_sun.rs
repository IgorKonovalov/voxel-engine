//! The sun and the moon as the light pass's directional sample (ADR-0027): a floor of rock
//! under a key light, read from the light pass's direct target, and the lighting rays' rule
//! that an absent chunk inside the residency ball is not sky. Lavapipe, lamp off, no list
//! light, no sky ray, filter off, manual exposure 1.0, the full shadow walk, lighting at full
//! resolution.
//! The image bars (the penumbra, the settle after a jump, the upscaler's mask) are readings in
//! `mod reading` until the owner's window.

use std::f32::consts::FRAC_PI_2;

use vxe_render::sky::{Body, SUN_ANGULAR_RADIUS, Sky};
use vxe_render::{
    Camera, Exposure, Frame, GpuWorld, LightParams, Lighting, LightingResolution, LodParams,
    MirrorConfig, ShadowWalk, Window, glam::Vec3, wgpu,
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
/// air mass of 2 is within 0.2 % of the curved one's transmittance). Mean over 16 frames.
#[test]
fn the_noon_sun_reads_the_transmittance() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = floor_world([2, 2, 1], |_| false);
    let gpu = mirror(&device, &queue, &world, None);
    let cam = camera([64.0, 40.0]);
    let below = dir(-45.0, 0.0);
    for (sun, want) in [
        (dir(90.0, 0.0), T_ZENITH),
        (dir(30.0, 90.0), [0.4422, 0.3764, 0.2906]),
    ] {
        let mut f = frame(&device, SIZE, ShadowWalk::Full);
        f.set_sky(Some(sky(sun, below, [[1.0; 3], [0.0; 3]])));
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

/// Bar 5: on a mirror whose window holds the chunk above the floor's but whose world does not
/// (absent, never synced), with the residency ball set (`LodParams::r_lod`) the floor under a
/// noon sun reads 0; with no ball it reads the noon value of bar 1, as it does where that
/// chunk is synced as all-air (`EMPTY_BIT`) with the ball set.
#[test]
fn an_absent_chunk_blocks_the_sun() {
    let Some((device, queue)) = device() else {
        return;
    };
    let window = ChunkExtent::new([0, 0, 0], [1, 1, 2]);
    let cam = Camera {
        position: Vec3::new(32.0, 8.0, FLOOR as f32 + 30.0),
        ..camera([0.0, 0.0])
    };
    let noon = sky(dir(90.0, 0.0), dir(-90.0, 0.0), [[1.0; 3], [0.0; 3]]);
    let ball = LodParams {
        r_full: 1,
        r_lod: Some(1),
        ..LodParams::default()
    };
    let read = |world: &World, lod: LodParams| {
        let gpu = mirror(&device, &queue, world, Some(window));
        let mut f = frame(&device, SIZE, ShadowWalk::Full);
        f.set_lod(lod);
        f.set_sky(Some(noon));
        centre_direct(&device, &queue, &mut f, &gpu, &cam, 4)
    };
    let half = floor_world([1, 1, 1], |_| false);
    let whole = floor_world([1, 1, 2], |_| false);
    let absent = read(&half, ball);
    let no_ball = read(&half, LodParams::default());
    let synced = read(&whole, ball);
    println!("absent {absent:?}, no ball {no_ball:?}, synced air {synced:?}");
    assert_eq!(absent, [0.0; 3], "an absent chunk in the ball is not sky");
    assert!(within(no_ball, T_ZENITH, 0.01), "{no_ball:?}");
    assert!(within(synced, T_ZENITH, 0.01), "{synced:?}");
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
mod reading {
    use super::*;

    /// Bar 3: a 16 x 16 voxel roof slab 20 voxels above the floor under a zenith sun of radius
    /// 5 degrees, the camera straight down at 4 or more pixels a voxel; the floor's direct
    /// value along a row across the shadow's edge, a 64-frame mean, and the 5 %-to-95 % rise
    /// in voxels (2.8 +- 0.6 by the disc's geometry), under the full and the coarse walk.
    #[test]
    #[ignore = "reading"]
    fn a_roof_slab_casts_a_soft_shadow() {
        let Some((device, queue)) = device() else {
            return;
        };
        let slab = |p: [i32; 3]| {
            p[2] == FLOOR + 20 && (24..40).contains(&p[0]) && (24..40).contains(&p[1])
        };
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
        for walk in [ShadowWalk::Full, ShadowWalk::Coarse] {
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
            let open = T_ZENITH[0] as f64;
            let v: Vec<f64> = sum.iter().map(|s| s / f64::from(frames) / open).collect();
            // The edge at x = 40 (the slab's +x side); the rise from 5 % to 95 % past it.
            let first = |t: f64| (0..w).find(|&x| xs[x] > 30.0 && v[x] >= t).map(|x| xs[x]);
            let (lo, hi) = (first(0.05), first(0.95));
            let at_edge = (0..w)
                .min_by(|&a, &b| (xs[a] - 40.0).abs().total_cmp(&(xs[b] - 40.0).abs()))
                .map(|x| v[x]);
            println!(
                "{walk:?}: 5 % at x {lo:?}, 95 % at x {hi:?}, rise {:?} voxels; at the edge {at_edge:?}",
                lo.zip(hi).map(|(a, b)| b - a)
            );
            let inside = (0..w)
                .filter(|&x| xs[x] > 26.0 && xs[x] < 38.0)
                .map(|x| v[x])
                .fold(0.0, f64::max);
            let near: Vec<String> = (0..w)
                .filter(|&x| (xs[x] - 40.0).abs() < 0.6)
                .map(|x| format!("{:.3} at {:.3}", v[x], xs[x]))
                .collect();
            println!("{walk:?}: around the edge {near:?}");
            let outside = (0..w)
                .filter(|&x| xs[x] > 43.0)
                .map(|x| v[x])
                .fold(1.0, f64::min);
            println!(
                "{walk:?}: brightest 3 voxels inside {inside:.4}, darkest 3 outside {outside:.4}"
            );
        }
    }

    /// Bar 6's settle: the floor beside a pillar under a sun turned by 10 degrees in one frame,
    /// filter on, the real radius: the share of pixels that left the shadow reading at least
    /// 90 % of their lit value on frames 1 and 11 after the turn.
    #[test]
    #[ignore = "reading"]
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
        let lit = reads[10].iter().map(|c| c[0]).fold(0.0f32, f32::max);
        let left: Vec<usize> = (0..before.len())
            .filter(|&i| before[i][0] < 0.1 * lit && reads[10][i][0] > 0.9 * lit)
            .collect();
        for (k, r) in [(1, &reads[0]), (11, &reads[10])] {
            let ok = left
                .iter()
                .filter(|&&i| r[i][0] >= 0.9 * reads[10][i][0])
                .count();
            println!(
                "frame {k} after the turn: {ok} of {} pixels that left the shadow at 90 %",
                left.len()
            );
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

    /// Bar 7: FSR 2 at 1.5 over the floor beside a pillar: on the frame of a 10-degree turn
    /// the share of sunlit floor pixels marked in the input pass's reactive mask; over 64
    /// frames of a drift of 0.15 radius a frame, the frames whose mask differs from the
    /// still sun's (which is all zero on a settled still frame).
    #[test]
    #[ignore = "reading"]
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
        let mut f = frame(&device, [320, 180], ShadowWalk::Coarse);
        f.set_upscaler_kind(
            &device,
            vxe_render::UpscalerKind::Fsr2(vxe_render::Fsr2Settings::default()),
        );
        let at = |el: f32| sky(dir(el, 180.0), dir(-el, 0.0), [[1.0; 3], [0.0; 3]]);
        let light = LightParams::headlamp();
        let mask = |f: &Frame| -> Vec<u8> {
            vxe_render::debug::read_rgba8(&device, &queue, f.fsr2().unwrap().reactive())
                .chunks(4)
                .map(|p| p[0])
                .collect()
        };
        f.set_sky(Some(at(50.0)));
        for _ in 0..32 {
            f.step(&device, &queue, &gpu, &cam, &light);
        }
        let still = mask(&f);
        println!(
            "settled still frame: {} marked of {}",
            still.iter().filter(|&&m| m > 0).count(),
            still.len()
        );
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
        let r = SUN_ANGULAR_RADIUS.to_degrees();
        let mut differing = 0;
        for i in 1..=64 {
            f.set_sky(Some(at(60.0 + 0.15 * r * i as f32)));
            f.step(&device, &queue, &gpu, &cam, &light);
            differing += usize::from(mask(&f).iter().any(|&m| m > 0));
        }
        println!("drift: {differing} of 64 frames with a marked pixel");
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
        let light = LightParams::headlamp();
        let run = |rays: Option<u32>, fsr: bool, exposure: f32| -> (f64, f64, f64) {
            let mut f = if fsr {
                let mut f = Frame::with_sizes(&device, [320, 180], [480, 270]);
                f.set_upscaler_kind(
                    &device,
                    vxe_render::UpscalerKind::Fsr2(vxe_render::Fsr2Settings::default()),
                );
                f
            } else {
                Frame::new(&device, 320, 180)
            };
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
            for _ in 0..64 {
                f.step(&device, &queue, &gpu, &cam, &light);
            }
            let read = |f: &Frame| vxe_render::debug::read_rgba8(&device, &queue, f.target());
            let mut last = read(&f);
            let mean_level =
                last.chunks(4).map(|p| f64::from(p[1])).sum::<f64>() / (last.len() / 4) as f64;
            let mut changes = Vec::new();
            for _ in 0..64 {
                f.step(&device, &queue, &gpu, &cam, &light);
                let now = read(&f);
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

    /// Plan 0032 Phase 6 bars 2 and 3 under `Sky::uniform(1.0)`, one sky ray, filter off,
    /// exposure 1, 64-frame mean of the direct target at the centre pixel: the open floor 40
    /// and about 400 voxels from the camera (pi is the bar), and the floor inside a closed rock
    /// box with 5-voxel walls, the camera inside it (under 2 % of pi is the bar).
    #[test]
    #[ignore = "reading"]
    fn the_open_floor_and_a_closed_box_under_a_uniform_sky() {
        let Some((device, queue)) = device() else {
            return;
        };
        let walls = |p: [i32; 3]| {
            let inside = |k: usize, lo: i32, hi: i32| (lo..hi).contains(&p[k]);
            let outer = inside(0, 100, 140) && inside(1, 100, 140) && inside(2, FLOOR, FLOOR + 30);
            let inner = inside(0, 105, 135) && inside(1, 105, 135) && inside(2, FLOOR, FLOOR + 25);
            outer && !inner
        };
        let world = floor_world([16, 16, 1], walls);
        let gpu = mirror(&device, &queue, &world, None);
        let at = |pos: [f32; 3], pitch: f32| Camera {
            position: Vec3::from(pos),
            yaw: FRAC_PI_2,
            pitch: pitch.to_radians(),
            vfov: 60f32.to_radians(),
        };
        for (name, cam) in [
            (
                "open floor, 57 voxels",
                at([512.0, 300.0, FLOOR as f32 + 40.0], -45.0),
            ),
            (
                "open floor, 400 voxels",
                at([512.0, 300.0, FLOOR as f32 + 40.0], -5.71),
            ),
            (
                "inside the closed box",
                at([120.0, 110.0, FLOOR as f32 + 12.0], -45.0),
            ),
        ] {
            let mut f = frame(&device, SIZE, ShadowWalk::Coarse);
            f.set_lighting(Lighting {
                sky_rays: 1,
                ..f.lighting()
            });
            f.set_sky(Some(Sky {
                altitude_zero_z: FLOOR as f32,
                ..Sky::uniform(1.0)
            }));
            let got = centre_direct(&device, &queue, &mut f, &gpu, &cam, 64);
            println!(
                "{name}: {got:?}, {:.4} of pi",
                got[1] / std::f32::consts::PI
            );
        }
    }
}
