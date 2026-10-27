//! The exposure (ADR-0033): the composite's meter and the exposure pass's state, read back from
//! closed scenes whose walls have albedo 0 and emit a grey radiance, so every pixel's luminance
//! is its wall's emission exactly. The lamp is off, indirect light off, the ACES tone map.

use vxe_render::exposure::ExposureState;
use vxe_render::{
    Camera, Frame, GpuWorld, LightParams, Lighting, LightingResolution, Metering, glam::Vec3,
    readback, wgpu,
};
use vxe_world::*;

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        force_fallback_adapter: true,
        ..Default::default()
    }))
    .or_else(|_| pollster::block_on(instance.request_adapter(&Default::default())));
    let Ok(adapter) = adapter else {
        println!("skipped: no adapter");
        return None;
    };
    let limits = vxe_render::limits_for(&adapter, 16 << 20);
    let desc = wgpu::DeviceDescriptor {
        required_limits: limits,
        ..Default::default()
    };
    Some(pollster::block_on(adapter.request_device(&desc)).expect("device"))
}

/// A world whose voxel at world coordinate `v` is `f(v)`.
struct Voxels<F>(F);

impl<F: Fn([i32; 3]) -> MaterialId + Sync> WorldSource for Voxels<F> {
    fn chunk(&self, coord: ChunkCoord, out: &mut Chunk) {
        let base = coord.min_voxel().0;
        for slot in 0..CHUNK_BRICKS {
            let b = brick_of_slot(slot);
            out.set_brick(
                slot,
                Brick::from_fn(|l| (self.0)([0, 1, 2].map(|k| base[k] + b[k] * BRICK_EDGE + l[k]))),
            );
        }
    }
}

/// A material of albedo 0 emitting grey `l`.
fn emitter(name: &str, l: f32) -> Material {
    Material {
        albedo: [0.0; 3],
        emission: [l; 3],
        ..Material::named(name)
    }
}

/// Materials 1 and 2: albedo 0, emitting grey `a` and `b`.
fn two_emitters(a: f32, b: f32) -> MaterialTable {
    let mut t = MaterialTable::new();
    assert_eq!(t.push(emitter("a", a)), 1);
    assert_eq!(t.push(emitter("b", b)), 2);
    t
}

/// Chunks x in -1..1, y and z in 0..1: rock of material `f(v)` at y >= `WALL_Y` and wherever
/// `v` lies outside the box `x in -60..60, y in 1.., z in 1..63` (so the box is closed),
/// air elsewhere.
fn walled(f: impl Fn([i32; 3]) -> MaterialId + Sync, table: &MaterialTable) -> World {
    World::generate(
        ChunkExtent::new([-1, 0, 0], [1, 1, 1]),
        &Voxels(move |v: [i32; 3]| {
            let inside = (-60..60).contains(&v[0]) && v[1] >= 1 && (1..63).contains(&v[2]);
            if v[1] >= WALL_Y || !inside { f(v) } else { AIR }
        }),
        table,
    )
}

/// The wall's near face, in voxels along y.
const WALL_Y: i32 = 52;
/// The eye: x = 0, centred in z, `WALL_Y - EYE[1]` voxels from the wall.
const EYE: [f32; 3] = [0.0, 32.0, 32.0];
const SIZE: [u32; 2] = [256, 144];

/// The camera looking +y at the wall; `vfov` 2 atan(1/2), so the view is the eye's distance
/// from the wall tall at the wall.
fn camera() -> Camera {
    Camera {
        position: Vec3::from_array(EYE),
        yaw: std::f32::consts::FRAC_PI_2,
        pitch: 0.0,
        vfov: 2.0 * 0.5f32.atan(),
    }
}

/// A frame at `SIZE`, full-resolution lighting, the lamp off, indirect light off.
fn frame(device: &wgpu::Device) -> Frame {
    let mut frame = Frame::new(device, SIZE[0], SIZE[1]);
    frame.set_resolution(device, LightingResolution::Full);
    frame.set_lighting(Lighting {
        lamp: false,
        ..Lighting::default()
    });
    frame
}

fn mirror(device: &wgpu::Device, queue: &wgpu::Queue, world: &World) -> GpuWorld {
    let mut gpu = GpuWorld::new(device, world.extent());
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    gpu.sync(world, queue, &mut enc).expect("sync");
    queue.submit([enc.finish()]);
    gpu
}

/// A blocking read of the exposure state.
fn state(device: &wgpu::Device, queue: &wgpu::Queue, frame: &Frame) -> ExposureState {
    ExposureState::from_bytes(&readback::read_buffer(
        device,
        queue,
        frame.exposure_state(),
        0,
        32,
    ))
}

/// The meter's tiles as `(sum, count)` pairs.
fn tiles(device: &wgpu::Device, queue: &wgpu::Queue, frame: &Frame) -> Vec<[f32; 2]> {
    let n = SIZE.map(|v| v.div_ceil(8));
    let bytes = readback::read_buffer(
        device,
        queue,
        frame.exposure_tiles(),
        0,
        8 * u64::from(n[0] * n[1]),
    );
    bytes
        .chunks(8)
        .map(|p| [0, 1].map(|k| f32::from_le_bytes(p[4 * k..4 * k + 4].try_into().unwrap())))
        .collect()
}

/// `sum(w S) / sum(w n)` over the tiles of one full-target view under `metering`, the weight
/// at each 8 x 8 tile's centre (exposure.wgsl's formula), in f64.
fn cpu_meter(tiles: &[[f32; 2]], metering: Metering) -> f64 {
    let tw = SIZE[0].div_ceil(8) as usize;
    let (cx, cy, half) = (
        f64::from(SIZE[0]) / 2.0,
        f64::from(SIZE[1]) / 2.0,
        f64::from(SIZE[1]) / 2.0,
    );
    let (mut ws, mut wn) = (0.0, 0.0);
    for (i, t) in tiles.iter().enumerate() {
        let x = ((i % tw) as f64 + 0.5) * 8.0;
        let y = ((i / tw) as f64 + 0.5) * 8.0;
        let r = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt() / half;
        let w = f64::from(metering.weight(r as f32));
        ws += w * f64::from(t[0]);
        wn += w * f64::from(t[1]);
    }
    ws / wn
}

const CENTRE: Metering = Metering::CentreWeighted {
    strength: 3.0,
    sigma: 0.5,
};

/// A wall whose half at x < 0 emits `2^-3` and whose half at x >= 0 emits `2^-9`, filling the
/// 256 x 144 view from 20 voxels: no pixel centre lies on x = 0, so each holds 128 columns.
/// The meter reads `(-3 - 9) / 2 = -6` within 1e-3 under the average and under the centre
/// weight, which is symmetric about the split; every tile counts its 64 pixels.
#[test]
fn a_wall_split_in_half_meters_the_mean_of_its_halves() {
    let Some((device, queue)) = device() else {
        return;
    };
    let table = two_emitters(2f32.powi(-3), 2f32.powi(-9));
    let world = walled(|v| if v[0] < 0 { 1 } else { 2 }, &table);
    let gpu = mirror(&device, &queue, &world);
    for metering in [Metering::Average, CENTRE] {
        let mut frame = frame(&device);
        frame.set_metering(metering);
        frame.render(&device, &queue, &gpu, &camera(), &LightParams::at_eye());
        let s = state(&device, &queue, &frame);
        let t = tiles(&device, &queue, &frame);
        println!("{metering:?}: metered {} log2", s.metered_log2);
        assert!(
            t.iter().all(|p| p[1] == 64.0),
            "every tile counts 64 pixels"
        );
        assert!(
            (s.metered_log2 + 6.0).abs() <= 1e-3,
            "{metering:?} meters {}",
            s.metered_log2
        );
        assert_eq!(s.flags & vxe_render::exposure::STATE_HELD, 0);
        assert_eq!((s.current, s.previous), (1.0, 1.0));
    }
}

/// A central square of `2^-3`, half the view's height on a side, in a field of `2^-9`: the
/// meter matches the CPU's `sum(w S) / sum(w n)` over the read-back tiles within 1e-3 under
/// both meterings, which differ by more than 0.5 EV (about -8.16 and -7.35: the square is
/// 14.1 % of the view, the centre weight gives it more).
#[test]
fn the_tile_weights_are_the_documented_ones() {
    let Some((device, queue)) = device() else {
        return;
    };
    let table = two_emitters(2f32.powi(-3), 2f32.powi(-9));
    // The view is 20 voxels tall at the wall: the square spans x -5..5 and z 27..37.
    let world = walled(
        |v| {
            if (-5..5).contains(&v[0]) && (27..37).contains(&v[2]) {
                1
            } else {
                2
            }
        },
        &table,
    );
    let gpu = mirror(&device, &queue, &world);
    let mut read = Vec::new();
    for metering in [Metering::Average, CENTRE] {
        let mut frame = frame(&device);
        frame.set_metering(metering);
        frame.render(&device, &queue, &gpu, &camera(), &LightParams::at_eye());
        let s = state(&device, &queue, &frame);
        let cpu = cpu_meter(&tiles(&device, &queue, &frame), metering);
        println!("{metering:?}: gpu {} cpu {cpu}", s.metered_log2);
        assert!(
            (f64::from(s.metered_log2) - cpu).abs() <= 1e-3,
            "{metering:?}: gpu {} cpu {cpu}",
            s.metered_log2
        );
        read.push(s.metered_log2);
    }
    assert!(
        read[1] - read[0] > 0.5,
        "centre {} against average {}",
        read[1],
        read[0]
    );
    assert!((read[0] + 8.16).abs() < 0.1, "average {}", read[0]);
    assert!((read[1] + 7.35).abs() < 0.15, "centre {}", read[1]);
}

/// The read-back: right after the first frame's submission it has nothing and returns at once;
/// in a loop that waits for each frame before the next (a presented frame's pace) it returns the
/// state within 4 frames of the first, the wall's meter among it.
#[test]
fn the_readout_arrives_within_four_frames_without_waiting() {
    let Some((device, queue)) = device() else {
        return;
    };
    let table = two_emitters(2f32.powi(-3), 2f32.powi(-9));
    let world = walled(|v| if v[0] < 0 { 1 } else { 2 }, &table);
    let gpu = mirror(&device, &queue, &world);
    let mut frame = frame(&device);
    let light = LightParams::at_eye();
    let mut got = None;
    for i in 0..4 {
        frame.step(&device, &queue, &gpu, &camera(), &light);
        let r = frame.exposure_readout(&device);
        println!("frame {i}: {r:?}");
        if i == 0 {
            assert_eq!(
                r, None,
                "nothing has mapped before the first frame finished"
            );
        }
        if r.is_some() {
            got = r;
            break;
        }
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
    }
    let r = got.expect("a readout within 4 frames");
    assert!((r.metered_log2 + 6.0).abs() <= 1e-3, "{r:?}");
    assert_eq!(r.exposure, 1.0);
    assert!(!r.held);
}

/// Material 1: rock, albedo 0.5. Material 2: an emitter of albedo 0.5 and emission
/// `[1, 1, 1]`.
fn rock_and_glow() -> MaterialTable {
    let mut t = MaterialTable::new();
    assert_eq!(
        t.push(Material {
            albedo: [0.5; 3],
            ..Material::named("rock")
        }),
        1
    );
    assert_eq!(
        t.push(Material {
            albedo: [0.5; 3],
            emission: [1.0; 3],
            ..Material::named("glow")
        }),
        2
    );
    t
}

/// One chunk of rock with a closed box of air, interior x and y `[22, 42)`, z `[20, 30)`, whose
/// whole ceiling (the layer z = 30) glows.
fn ceiling_box() -> World {
    World::generate(
        ChunkExtent::new([0, 0, 0], [1, 1, 1]),
        &Voxels(|v: [i32; 3]| {
            let inside_xy = (22..42).contains(&v[0]) && (22..42).contains(&v[1]);
            if inside_xy && (20..30).contains(&v[2]) {
                AIR
            } else if inside_xy && v[2] == 30 {
                2
            } else {
                1
            }
        }),
        &rock_and_glow(),
    )
}

/// The camera at (32, 32, 29) looking straight down at the floor's centre.
fn floor_camera() -> Camera {
    Camera {
        position: Vec3::new(32.0, 32.0, 29.0),
        yaw: 0.0,
        pitch: -std::f32::consts::FRAC_PI_2,
        vfov: 1.0,
    }
}

/// A 32 x 32 frame over the ceiling box, full-resolution lighting, the lamp and the filter
/// off, at manual exposure `e`.
fn ceiling_frame(device: &wgpu::Device, e: f32) -> Frame {
    let mut frame = Frame::new(device, 32, 32);
    frame.set_resolution(device, LightingResolution::Full);
    frame.set_lighting(Lighting {
        lamp: false,
        filter: false,
        exposure: e,
        ..Lighting::default()
    });
    frame
}

/// The ages (moments alpha) of the last encoded frame.
fn ages(device: &wgpu::Device, queue: &wgpu::Queue, frame: &Frame) -> Vec<f32> {
    readback::read_rgba16f(device, queue, frame.moments())
        .iter()
        .map(|m| m[2])
        .collect()
}

/// The centre pixel's display bytes.
fn centre(device: &wgpu::Device, queue: &wgpu::Queue, frame: &Frame) -> [u8; 3] {
    let px = readback::read_rgba8(device, queue, frame.target());
    let i = (16 * 32 + 16) * 4;
    [px[i], px[i + 1], px[i + 2]]
}

/// On the emissive-ceiling box (filter off, lamp off), a manual step from 1.0 to 4.0 at frame
/// 64 kills no history: at frames 64 and 65 every hit pixel's age is the frame before's plus
/// one, and the floor-centre pixel at frame 65 lies within 2/255 of a run at 4.0 throughout.
#[test]
fn history_survives_an_exposure_step() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = ceiling_box();
    let gpu = mirror(&device, &queue, &world);
    let light = LightParams::at_eye();
    let mut stepped = ceiling_frame(&device, 1.0);
    let mut prev_ages = Vec::new();
    for i in 0..66 {
        if i == 64 {
            stepped.set_lighting(Lighting {
                exposure: 4.0,
                ..stepped.lighting()
            });
        }
        stepped.step(&device, &queue, &gpu, &floor_camera(), &light);
        if i >= 63 {
            let a = ages(&device, &queue, &stepped);
            if i >= 64 {
                for (k, (now, before)) in a.iter().zip(&prev_ages).enumerate() {
                    assert_eq!(
                        *now,
                        before + 1.0,
                        "frame {i}, pixel {k}: age {now} after {before}"
                    );
                }
            }
            println!("frame {i}: centre age {}", a[16 * 32 + 16]);
            prev_ages = a;
        }
    }
    let got = centre(&device, &queue, &stepped);
    let mut at_four = ceiling_frame(&device, 4.0);
    for _ in 0..66 {
        at_four.step(&device, &queue, &gpu, &floor_camera(), &light);
    }
    let want = centre(&device, &queue, &at_four);
    println!("floor centre at frame 65: stepped {got:?}, at 4.0 throughout {want:?}");
    for k in 0..3 {
        assert!(got[k].abs_diff(want[k]) <= 2, "{got:?} against {want:?}");
    }
}

/// In a rock box with the lamp on at the eye, `Manual(64)`, and the camera 3 voxels from a wall
/// on the lamp's axis (irradiance `PI 1200 / 9 = 419`, pre-exposed 26 800), every texel of the
/// irradiance, the accumulation, the moments and the filter is finite after 64 frames.
#[test]
fn no_target_overflows_at_the_glare() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = ceiling_box();
    let gpu = mirror(&device, &queue, &world);
    // From (32, 25, 25) looking -y: the wall's face at y = 22 is 3 voxels away.
    let camera = Camera {
        position: Vec3::new(32.0, 25.0, 25.0),
        yaw: -std::f32::consts::FRAC_PI_2,
        pitch: 0.0,
        vfov: 1.0,
    };
    let mut frame = Frame::new(&device, 64, 36);
    frame.set_resolution(&device, LightingResolution::Full);
    frame.set_lighting(Lighting {
        exposure: 64.0,
        ..Lighting::default()
    });
    let light = LightParams::at_eye();
    for _ in 0..64 {
        frame.step(&device, &queue, &gpu, &camera, &light);
    }
    let mut peak = 0.0f32;
    for (name, tex) in [
        ("irradiance", frame.irradiance()),
        ("accumulated", frame.accumulated()),
        ("moments", frame.moments()),
        ("filtered", frame.filtered()),
    ] {
        let texels = readback::read_rgba16f(&device, &queue, tex);
        let bad = texels.iter().flatten().filter(|v| !v.is_finite()).count();
        let max = texels.iter().flatten().fold(0.0f32, |m, &v| m.max(v));
        println!("{name}: largest {max}, {bad} non-finite");
        assert_eq!(bad, 0, "{name}: {bad} non-finite values");
        if name == "irradiance" {
            peak = max;
        }
    }
    assert!(
        peak > 20_000.0,
        "the wall's pre-exposed irradiance reads {peak}"
    );
}
