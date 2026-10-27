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
