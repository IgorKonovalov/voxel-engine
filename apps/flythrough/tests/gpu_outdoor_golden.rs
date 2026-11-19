//! Plan 0032's goldens: the sky of an empty world at noon, sunset and night, the outdoor
//! hills of seed 1 at noon, sunset and night and in the cave mouth, and the forest at sunset,
//! each at the testbed's picks (both bodies at `daylight::BODY_RADIUS_DEG`, one sky ray, the
//! half lighting tier, the outdoor auto exposure, `--trees sparse` at `LEAF_FILL` unless a
//! test names another). Lavapipe, 320 x 180, blessed with `VXE_BLESS=1`.

mod support;

use flythrough::daylight::{BODY_RADIUS_DEG, Clock};
use flythrough::outdoor::{BASE_Z, GRASS, OutdoorSource, SOIL, Spawn};
use flythrough::trees::{ForestSource, LEAF_FILL, TreeDensity};
use support::{HEIGHT, SyncNow, WIDTH, assert_golden, fraction_differing, software_device};
use vxe_render::sky::{Body, Sky};
use vxe_render::{
    AutoExposure, Camera, Exposure, Frame, GpuWorld, LightParams, Lighting, LightingResolution,
    MirrorConfig, Window, glam::Vec3, wgpu,
};
use vxe_world::*;

/// A unit direction at elevation `el` and azimuth `az` (degrees, clockwise from north).
fn dir(el: f32, az: f32) -> [f32; 3] {
    let (e, a) = (el.to_radians(), az.to_radians());
    [e.cos() * a.sin(), e.cos() * a.cos(), e.sin()]
}

/// The camera at `p` facing azimuth `az` (degrees, clockwise from north) at pitch `pitch`.
fn facing(p: [f32; 3], az: f32, pitch: f32) -> Camera {
    Camera {
        position: Vec3::from(p),
        yaw: (90.0 - az).to_radians(),
        pitch: pitch.to_radians(),
        vfov: 70f32.to_radians(),
    }
}

/// `x` rounded to two significant digits, so a pinned exposure does not carry a readback's
/// last bits.
fn two_digits(x: f32) -> f32 {
    let e = 10f32.powf(x.log10().floor() - 1.0);
    (x / e).round() * e
}

/// The empty world's frame: a fixed window over one all-air chunk, the lamp off, `exposure`.
struct Empty {
    device: wgpu::Device,
    queue: wgpu::Queue,
    gpu: GpuWorld,
    frame: Frame,
}

impl Empty {
    fn new() -> Option<Self> {
        let world = World::generate(
            ChunkExtent::new([0, 0, 0], [1, 1, 1]),
            &FnSource(|_| AIR),
            &flythrough::outdoor_materials(),
        );
        let (device, queue) = software_device(64 << 20)?;
        let mut gpu =
            GpuWorld::with_config(&device, MirrorConfig::new(Window::Fixed(world.extent())));
        gpu.sync_now(&world, &device, &queue)
            .expect("the mirror fits");
        let frame = Frame::new(&device, WIDTH, HEIGHT);
        Some(Self {
            device,
            queue,
            gpu,
            frame,
        })
    }

    /// The exposure the outdoor-free auto exposure (instant, its floor and ceiling wide open)
    /// settles at looking straight up under `sky`: the zenith metered to mid-grey, two digits.
    fn zenith_exposure(&mut self, sky: Sky) -> f32 {
        self.frame.set_lighting(Lighting {
            lamp: false,
            exposure: Exposure::Auto(AutoExposure {
                min: 1e-12,
                max: 1e12,
                ..AutoExposure::instant()
            }),
            ..flythrough::default_lighting()
        });
        self.frame.set_sky(Some(sky));
        self.frame.set_dt(1.0 / 60.0);
        let up = facing([32.0, 32.0, 32.0], 0.0, 89.9);
        for _ in 0..8 {
            self.frame.step(
                &self.device,
                &self.queue,
                &self.gpu,
                &up,
                &LightParams::headlamp(),
            );
        }
        let e = self
            .frame
            .exposure_readout(&self.device)
            .expect("the exposure read back")
            .exposure;
        two_digits(e)
    }

    /// `sky` seen by `camera` at the zenith's mid-grey exposure, one frame.
    fn render(&mut self, sky: Sky, camera: &Camera) -> Vec<u8> {
        let exposure = self.zenith_exposure(sky);
        println!("zenith exposure {exposure:e}");
        self.frame.set_lighting(Lighting {
            lamp: false,
            exposure: Exposure::Manual(exposure),
            ..flythrough::default_lighting()
        });
        self.frame.render(
            &self.device,
            &self.queue,
            &self.gpu,
            camera,
            &LightParams::headlamp(),
        );
        vxe_render::debug::read_rgba8(&self.device, &self.queue, self.frame.target())
    }
}

/// A sky of `sun` and `moon` at the testbed's radius, their default illuminances, altitude 0
/// at voxel z 0.
fn empty_sky(sun: [f32; 3], moon: [f32; 3]) -> Sky {
    let r = BODY_RADIUS_DEG.to_radians();
    Sky {
        altitude_zero_z: 0.0,
        ..Sky::new(
            Body {
                angular_radius: r,
                ..Body::sun(sun)
            },
            Body {
                angular_radius: r,
                ..Body::moon(moon)
            },
        )
    }
}

/// Plan 0032 Phase 1 bar 4: `sky_empty_noon.png`, an empty world, the camera looking south at
/// the horizon, the sun at elevation 45 degrees behind it (north), the zenith metered to
/// mid-grey.
#[test]
fn sky_empty_noon_golden() {
    let Some(mut e) = Empty::new() else {
        return;
    };
    let img = e.render(
        empty_sky(dir(45.0, 0.0), dir(-45.0, 180.0)),
        &facing([32.0, 32.0, 32.0], 180.0, 0.0),
    );
    assert_golden("sky_empty_noon.png", &img);
}

/// Plan 0032 Phase 2 bar 5: `sky_empty_sunset.png` (the sun at elevation 2.65 degrees in the
/// west, the camera facing it) and `sky_empty_night.png` (the sun at -45 degrees, the moon at
/// 45 in the south at `MOON_OVER_SUN` of the sun, the camera facing it), each with the zenith
/// metered to mid-grey.
#[test]
fn sky_empty_sunset_and_night_goldens() {
    let Some(mut e) = Empty::new() else {
        return;
    };
    let sunset = e.render(
        empty_sky(dir(2.65, 270.0), dir(-2.65, 90.0)),
        &facing([32.0, 32.0, 32.0], 270.0, 0.0),
    );
    assert_golden("sky_empty_sunset.png", &sunset);
    let night = e.render(
        empty_sky(dir(-45.0, 0.0), dir(45.0, 180.0)),
        &facing([32.0, 32.0, 32.0], 180.0, 20.0),
    );
    assert_golden("sky_empty_night.png", &night);
}

/// The outdoor hills of seed 1 with `trees` at `LEAF_FILL` over the chunks within `reach` of
/// `spawn`'s chunk (half of it in z), mirrored whole on lavapipe.
struct Hills {
    device: wgpu::Device,
    queue: wgpu::Queue,
    gpu: GpuWorld,
    world: World,
    eye: VoxelCoord,
    facing: [i64; 3],
}

impl Hills {
    fn new(spawn: Spawn, trees: TreeDensity, reach: i32) -> Option<Self> {
        let r = [reach, reach, reach / 2];
        Self::spanning(spawn, trees, r.map(|v| -v), r)
    }

    /// The chunks from `lo` to `hi` (inclusive, chunk offsets from the spawn's chunk).
    fn spanning(spawn: Spawn, trees: TreeDensity, lo: [i32; 3], hi: [i32; 3]) -> Option<Self> {
        let source = ForestSource::new(OutdoorSource::new(1), trees, LEAF_FILL);
        let (eye, facing) = source.base().spawn(spawn);
        let c = eye.chunk().0;
        let extent = ChunkExtent::new(
            [0, 1, 2].map(|k| c[k] + lo[k]),
            [0, 1, 2].map(|k| c[k] + hi[k] + 1),
        );
        let world = World::generate(extent, &source, &flythrough::outdoor_materials());
        let pool = flythrough::level_pool_units(&world, false) * vxe_render::internals::UNIT_BYTES;
        let (device, queue) = software_device(pool)?;
        let cfg = MirrorConfig {
            budget_bytes: 256 << 20,
            ..MirrorConfig::new(Window::Fixed(world.extent()))
        };
        let mut gpu = GpuWorld::with_config(&device, cfg);
        gpu.set_focus(eye);
        // Until every chunk is mirrored: an absent chunk is air, and sky to a lighting ray.
        while gpu
            .sync_now(&world, &device, &queue)
            .expect("the mirror fits")
            .pending_chunks
            > 0
        {}
        Some(Self {
            device,
            queue,
            gpu,
            world,
            eye,
            facing,
        })
    }

    /// The spawn's camera: at the eye's voxel centre, facing the spawn's direction.
    fn camera(&self) -> Camera {
        let d = self.facing.map(|v| v as f32);
        let p = self.eye.0.map(|v| v as f32 + 0.5);
        flythrough::camera_looking_at(p, [p[0] + d[0], p[1] + d[1], p[2] + d[2]])
    }

    /// A frame at the picks: the default lighting at the half tier, the outdoor auto
    /// exposure with instant adaptation, `lighting` applied over it, the sky of `clock`.
    fn frame(&mut self, clock: Clock, lighting: impl Fn(Lighting) -> Lighting) -> Frame {
        let mut frame = Frame::new(&self.device, WIDTH, HEIGHT);
        frame.set_resolution(&self.device, LightingResolution::Half);
        frame.set_lighting(lighting(Lighting {
            exposure: Exposure::Auto(AutoExposure {
                min: flythrough::cli::OUTDOOR_MIN,
                max: flythrough::cli::OUTDOOR_MAX,
                ..AutoExposure::instant()
            }),
            ..flythrough::default_lighting()
        }));
        frame.set_sky(Some(clock.sky(BASE_Z as f32)));
        frame.set_dt(1.0 / 60.0);
        frame
    }

    /// The store baked around the eye (one chunk), then `frames` frames of `camera`.
    fn settle(&mut self, frame: &mut Frame, camera: &Camera, frames: u32) {
        support::bake_around(
            &self.device,
            &self.queue,
            &mut self.gpu,
            frame,
            &self.world,
            self.eye.0,
            1,
            64,
        );
        for _ in 0..frames {
            frame.step(
                &self.device,
                &self.queue,
                &self.gpu,
                camera,
                &LightParams::headlamp(),
            );
        }
    }

    /// The image of `clock`'s sky after the bake and 64 frames at the picks.
    fn image(&mut self, clock: Clock) -> Vec<u8> {
        let camera = self.camera();
        let mut frame = self.frame(clock, |l| l);
        self.settle(&mut frame, &camera, 64);
        vxe_render::debug::read_rgba8(&self.device, &self.queue, frame.target())
    }
}

fn at(hhmm: [u32; 2]) -> Clock {
    Clock {
        minutes: f64::from(hhmm[0] * 60 + hhmm[1]),
        ..Clock::default()
    }
}

/// The mean of `f` over the pixels of `rect` (`[x0, y0, x1, y1)`) of a `WIDTH`-wide image of
/// `stride` channels.
fn rect_mean<T: Copy>(px: &[T], stride: usize, rect: [u32; 4], f: impl Fn(&[T]) -> f64) -> f64 {
    let mut s = 0.0;
    for y in rect[1]..rect[3] {
        for x in rect[0]..rect[2] {
            let i = (y * WIDTH + x) as usize * stride;
            s += f(&px[i..i + stride]);
        }
    }
    s / f64::from((rect[2] - rect[0]) * (rect[3] - rect[1]))
}

fn luma8(p: &[u8]) -> f64 {
    0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2])
}

/// The `inside` spawn's hills: 3 chunks around its chunk, and on to 10 chunks along +x and 4
/// down, so the whole tunnel and the chamber are resident (a lighting ray that leaves the
/// world is sky, so a cut-off tunnel would be open at its far end).
fn cave_mouth_hills() -> Option<Hills> {
    Hills::spanning(Spawn::Inside, TreeDensity::Sparse, [-3, -3, -4], [10, 3, 1])
}

/// The cave mouth's regions in the `inside` spawn's view facing out: the opening's sky over
/// the far end of the cut, and the tunnel's shaded left wall (the right wall near the mouth
/// faces the noon sun).
const OPENING: [u32; 4] = [120, 5, 200, 40];
const WALL: [u32; 4] = [0, 100, 60, 170];

/// Plan 0032 Phase 7 bar 2: `cave_mouth.png` (the `inside` spawn facing out at 12:00) matches, and
/// the opening reads brighter than the tunnel's shaded wall. The seal clause (Phase 9 bar 5,
/// re-derived by Plan 0086 Phase 17: a sealed cave takes no sky light once its history has run
/// out) is Plan 0094's to write back on the pair's sky term.
#[test]
#[ignore = "soak"]
fn the_cave_mouth_is_lit_from_outside() {
    let Some(mut h) = cave_mouth_hills() else {
        return;
    };
    let img = h.image(at([12, 0]));
    assert_golden("cave_mouth.png", &img);
    let opening = rect_mean(&img, 4, OPENING, luma8);
    let wall = rect_mean(&img, 4, WALL, luma8);
    println!("cave mouth: opening luma {opening:.1}, wall {wall:.1}");
    assert!(opening > wall, "{opening} against {wall}");

    // The seal (rock across the covered bore takes the sky out of the walls' light) is owed
    // by Plan 0094: the pair's sky term joins at the filter and the composite, so the direct
    // accumulation holds no sky and the claim is re-derived there on the pair's term or the
    // lit image.
}

/// Plan 0032 Phase 7 bar 1: `outdoor_noon.png`, `outdoor_sunset.png` and `outdoor_night.png`
/// (the `outside` spawn facing the mouth at 12:00, 17:45 and 00:00, the moon at 45 degrees in
/// the south at midnight) match, and with `cave_mouth.png`'s image each of the four differs
/// from every other in more than 20 % of its pixels by more than 8/255.
#[test]
#[ignore = "soak"]
fn outdoor_goldens_differ_by_time_of_day() {
    let Some(mut out) = Hills::new(Spawn::Outside, TreeDensity::Sparse, 6) else {
        return;
    };
    let mut images = Vec::new();
    for (name, clock) in [
        ("outdoor_noon.png", at([12, 0])),
        ("outdoor_sunset.png", at([17, 45])),
        ("outdoor_night.png", at([0, 0])),
    ] {
        let img = out.image(clock);
        assert_golden(name, &img);
        images.push((name, img));
    }
    drop(out);
    let mut inside = cave_mouth_hills().expect("the adapter");
    images.push(("cave_mouth.png", inside.image(at([12, 0]))));
    for (i, (a, x)) in images.iter().enumerate() {
        for (b, y) in &images[i + 1..] {
            let f = fraction_differing(x, y, 8);
            println!(
                "{a} against {b}: {:.1} % differ by more than 8/255",
                f * 100.0
            );
            assert!(f > 0.20, "{a} against {b}: {f}");
        }
    }
}

/// Plan 0032 Phase 8 bar 6: `outdoor_forest_sunset.png`, the `forest` spawn with `--trees
/// forest` at 17:45, matches. The same pose with `--trees none` and the forest's are then each
/// settled 64 frames at one manual exposure (the bare pose's auto exposure after its 64
/// frames), so the display compares the light and not the meter: the forest's ground pixels
/// (hit material grass or soil) read a lower mean luminance than the bare hills', and the
/// ratio is printed.
#[test]
#[ignore = "soak"]
fn outdoor_forest_sunset_golden() {
    let Some(mut forest) = Hills::new(Spawn::Forest, TreeDensity::Forest, 6) else {
        return;
    };
    assert_golden("outdoor_forest_sunset.png", &forest.image(at([17, 45])));
    let mut bare = Hills::new(Spawn::Forest, TreeDensity::None, 6).expect("the adapter");
    let camera = bare.camera();
    let mut frame = bare.frame(at([17, 45]), |l| l);
    bare.settle(&mut frame, &camera, 64);
    let exposure = frame
        .exposure_readout(&bare.device)
        .expect("the exposure read back")
        .exposure;
    let ground_luma = |h: &mut Hills| {
        let camera = h.camera();
        let mut frame = h.frame(at([17, 45]), |l| Lighting {
            exposure: Exposure::Manual(exposure),
            ..l
        });
        h.settle(&mut frame, &camera, 64);
        let img = vxe_render::debug::read_rgba8(&h.device, &h.queue, frame.target());
        let rec = vxe_render::debug::read_rgba16u(&h.device, &h.queue, frame.hit_records());
        let (mut s, mut n) = (0.0, 0usize);
        for (p, r) in img.chunks(4).zip(&rec) {
            if r[3] & 1 == 1 && (r[2] == GRASS || r[2] == SOIL) {
                s += luma8(p);
                n += 1;
            }
        }
        (s / n.max(1) as f64, n)
    };
    let (open, no) = ground_luma(&mut bare);
    drop(bare);
    let (in_forest, nf) = ground_luma(&mut forest);
    println!(
        "forest sunset ground luma at exposure {exposure:.3e}: forest {in_forest:.2} over {nf} pixels, bare {open:.2} over {no}, ratio {:.3}",
        in_forest / open
    );
    assert!(nf > 0 && no > 0);
    assert!(in_forest < open, "{in_forest} against {open}");
}
