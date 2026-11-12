//! The translucent class: translucent voxels are air to the opaque walk and are composited by
//! the bounded second walk, read back from one-pixel targets on closure worlds.

use vxe_render::{
    Camera, Frame, GpuWorld, LightParams, Lighting, LookParams, ToneMap, ViewMode, glam::Vec3,
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
    let limits = vxe_render::limits_for(&adapter, 16 << 20).expect("the frame's limits");
    let desc = wgpu::DeviceDescriptor {
        required_limits: limits,
        ..Default::default()
    };
    Some(pollster::block_on(adapter.request_device(&desc)).expect("device"))
}

/// Material 1: opaque grey, albedo 0.5. Material 2: translucent red, opacity 0.5.
const GREY: MaterialId = 1;
const RED: MaterialId = 2;

/// Rows carry properties, so the table is built here: `GREY` plain, `RED` translucent red at
/// opacity 0.5.
fn table() -> MaterialTable {
    let mut t = MaterialTable::new();
    assert_eq!(t.push(Material::named("grey")), GREY);
    assert_eq!(
        t.push(Material {
            albedo: [1.0, 0.0, 0.0],
            flags: MaterialFlags::TRANSLUCENT,
            opacity: 0.5,
            ..Material::named("red")
        }),
        RED
    );
    t
}

/// One chunk: a one-voxel-thick wall of `RED` at each x in `red`, and one of `GREY` at
/// x = 40, each spanning the chunk in y and z.
fn walls(red: &'static [i32]) -> World {
    World::generate(
        ChunkExtent::new([0, 0, 0], [1, 1, 1]),
        &FnSource(move |v: [i32; 3]| {
            if red.contains(&v[0]) {
                RED
            } else if v[0] == 40 {
                GREY
            } else {
                AIR
            }
        }),
        &table(),
    )
}

/// The one pixel of a 1 × 1 target for the +x ray from `origin`, in `view` with `layers`
/// translucent layers, lit by a lamp at the eye.
fn pixel(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    world: &World,
    origin: [f32; 3],
    view: ViewMode,
    layers: u32,
) -> [u8; 4] {
    let mut gpu = GpuWorld::new(device, world.extent());
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    gpu.sync(world, queue, &mut enc).expect("sync");
    queue.submit([enc.finish()]);
    let mut frame = Frame::new(device, 1, 1);
    frame.set_resolution(device, vxe_render::LightingResolution::Full);
    // The identity tone map: the expected values below are `pow(clamp(c), 1 / 2.2)`.
    frame.set_lighting(Lighting {
        tone_map: ToneMap::Identity,
        ..Lighting::default()
    });
    frame.set_look(LookParams {
        view,
        translucent_layers: layers,
        ..LookParams::default()
    });
    let camera = Camera {
        position: Vec3::from_array(origin),
        yaw: 0.0,
        pitch: 0.0,
        vfov: 1.0,
    };
    frame.render(device, queue, &gpu, &camera, &LightParams::at_eye());
    let px = readback::read_rgba8(device, queue, frame.target());
    [px[0], px[1], px[2], px[3]]
}

/// `rgb` in [0, 1] as the view's bytes, within `1 / 255` per channel.
fn assert_rgb(px: [u8; 4], rgb: [f32; 3], what: &str) {
    for k in 0..3 {
        let want = rgb[k] * 255.0;
        assert!(
            (px[k] as f32 - want).abs() <= 1.0,
            "{what}: channel {k} reads {} for {want:.2}",
            px[k]
        );
    }
}

const EYE: [f32; 3] = [10.5, 32.5, 32.5];

/// The red row's transmittance through one voxel (ADR-0047), `0.8 * (0.5 + 0.5 * albedo /
/// max(albedo))` of its albedo `[1, 0, 0]`.
const TINT: [f32; 3] = [0.8, 0.4, 0.4];

/// Schlick's reflected share of the entry face at normal view, `F0` 0.04: the +x ray meets the
/// face at `cos = 1`, so `R = F0`.
const R: f32 = 0.04;

/// The albedo view's pixel behind `len` voxels of red in front of the grey wall (albedo 0.5):
/// `R * face_light + T(len) * grey` with `T(len) = TINT^len` per channel. The albedo view runs
/// the filter unlit, so `face_light` is 1 there, not the lamp's term.
fn filtered(len: i32) -> [f32; 3] {
    TINT.map(|t| R + t.powi(len) * 0.5)
}

/// Through the red wall at x = 20 to the grey wall at x = 40, the albedo view filters by the
/// one voxel of path: `0.04 + 0.8 * 0.5 = 0.44` red, `0.04 + 0.4 * 0.5 = 0.24` green and blue,
/// and the hit mask reads the opaque hit. With no layers the grey wall alone is drawn. With
/// two red walls (x = 20 and 30) the layer bound caps the path in voxels of length: one layer
/// reads the one-wall value, two read `0.04 + 0.64 * 0.5 = 0.36` and `0.04 + 0.16 * 0.5 =
/// 0.12`. The reflected share is the first entry face's alone.
#[test]
#[ignore = "soak"]
fn translucent_layers_composite_front_to_back_up_to_the_bound() {
    let Some((device, queue)) = device() else {
        return;
    };
    let one = walls(&[20]);
    assert_rgb(
        pixel(&device, &queue, &one, EYE, ViewMode::Albedo, 4),
        filtered(1),
        "one red wall",
    );
    assert_eq!(
        pixel(&device, &queue, &one, EYE, ViewMode::HitMask, 4),
        [255, 255, 255, 255],
        "the opaque wall is the hit"
    );
    assert_rgb(
        pixel(&device, &queue, &one, EYE, ViewMode::Albedo, 0),
        [0.5, 0.5, 0.5],
        "no layers",
    );
    let two = walls(&[20, 30]);
    assert_eq!(
        pixel(&device, &queue, &two, EYE, ViewMode::HitMask, 4),
        [255, 255, 255, 255],
        "the opaque wall is the hit behind two"
    );
    assert_rgb(
        pixel(&device, &queue, &two, EYE, ViewMode::Albedo, 1),
        filtered(1),
        "two red walls, one layer",
    );
    assert_rgb(
        pixel(&device, &queue, &two, EYE, ViewMode::Albedo, 2),
        filtered(2),
        "two red walls, two layers",
    );
}

/// The opaque walk's pool reads on the ray through the red wall equal its pool reads without
/// it (the steps view's blue channel: one, the grey wall's voxel). Its steps and loads differ
/// by the red wall's brick, which is not uniform air: without the wall the ray skips the 16³
/// group of bricks 2 and 3 (1 table, 2 bits, brick 4's entry, brick 5's entry, 4 mask words,
/// and the hit's one pool word, the header's palette entry of a one-material brick: 10 loads
/// in 4 steps); with it the group is occupied, so brick 2's entry and its mask line are read
/// and the two empty 4³ blocks and brick 3's uniform-air entry are stepped (16 loads in 6
/// steps). The translucent walk does not run in the steps view.
#[test]
fn a_translucent_wall_costs_the_opaque_walk_no_pool_read() {
    let Some((device, queue)) = device() else {
        return;
    };
    let with = pixel(&device, &queue, &walls(&[20]), EYE, ViewMode::Steps, 4);
    let without = pixel(&device, &queue, &walls(&[]), EYE, ViewMode::Steps, 4);
    assert_eq!(
        with[2], without[2],
        "pool reads with {with:?}, without {without:?}"
    );
    assert_eq!(without, [4, 10, 1, 255]);
    assert_eq!(with, [6, 16, 1, 255]);
}

/// A uniform translucent brick is air to the opaque walk and filters in the second: with bricks
/// 2 and 3 along the ray (x in [16, 32)) all red, the path through them is 16 voxels, capped at
/// the 4 layers' length, so `T(4) = [0.4096, 0.0256, 0.0256]` of the grey wall remains under the
/// entry face's `R`: `[0.2448, 0.0528, 0.0528]`.
#[test]
fn a_uniform_translucent_brick_is_air_to_the_opaque_walk_and_layers_to_the_second() {
    let Some((device, queue)) = device() else {
        return;
    };
    let slab = World::generate(
        ChunkExtent::new([0, 0, 0], [1, 1, 1]),
        &FnSource(|v: [i32; 3]| {
            if (16..32).contains(&v[0]) {
                RED
            } else if v[0] == 40 {
                GREY
            } else {
                AIR
            }
        }),
        &table(),
    );
    assert_eq!(
        pixel(&device, &queue, &slab, EYE, ViewMode::HitMask, 4),
        [255, 255, 255, 255]
    );
    assert_rgb(
        pixel(&device, &queue, &slab, EYE, ViewMode::Albedo, 4),
        filtered(4),
        "a 16-voxel red slab, 4 layers",
    );
}
