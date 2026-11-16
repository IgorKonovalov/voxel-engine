//! The render settings as one value (ADR-0055): a frame's own settings applied back change
//! nothing, a `Settle` field applies on the next frame, and a `Rebuild` field waits for the
//! caller.

use vxe_render::{Camera, Frame, GpuWorld, LightParams, RenderScale, glam::Vec3, wgpu};
use vxe_world::*;

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    vxe_render::test_support::any_device(16 << 20, wgpu::Features::empty())
}

/// A hand-built box: one chunk, rock walls one brick thick around air, a rock pillar in the
/// middle, mirrored.
fn mirrored(device: &wgpu::Device, queue: &wgpu::Queue) -> GpuWorld {
    struct BoxRoom;
    impl WorldSource for BoxRoom {
        fn chunk(&self, coord: ChunkCoord, out: &mut Chunk) {
            let base = coord.min_voxel().0;
            for slot in 0..CHUNK_BRICKS {
                let b = brick_of_slot(slot);
                out.set_brick(
                    slot,
                    Brick::from_fn(|l| {
                        let v: [i32; 3] =
                            std::array::from_fn(|k| base[k] + b[k] * BRICK_EDGE + l[k]);
                        let wall = v.iter().any(|&c| !(4..60).contains(&c));
                        let pillar = (28..36).contains(&v[0]) && (28..36).contains(&v[1]);
                        if wall || pillar { 1 } else { AIR }
                    }),
                );
            }
        }
    }
    let mut t = MaterialTable::new();
    t.push(Material::named("rock"));
    let world = World::generate(ChunkExtent::new([0, 0, 0], [1, 1, 1]), &BoxRoom, &t);
    let mut gpu = GpuWorld::new(device, world.extent());
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    gpu.sync(&world, queue, &mut enc).expect("sync");
    queue.submit([enc.finish()]);
    gpu
}

fn camera() -> Camera {
    Camera {
        position: Vec3::new(12.0, 12.0, 30.0),
        yaw: 0.7,
        pitch: -0.2,
        vfov: 1.2,
    }
}

const SIZE: [u32; 2] = [192, 108];

/// Steps `n` frames and returns the last one's image.
fn frames(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    frame: &mut Frame,
    gpu: &GpuWorld,
    n: usize,
) -> Vec<Vec<u8>> {
    (0..n)
        .map(|_| {
            frame.step(device, queue, gpu, &camera(), &LightParams::default());
            vxe_render::debug::read_rgba8(device, queue, frame.target())
        })
        .collect()
}

/// A default frame at 192 x 108 over the box: applying its own settings returns an empty
/// change, and its next 8 frames' bytes equal those of a frame never applied to.
#[test]
fn applying_the_frames_own_settings_changes_nothing() {
    let Some((device, queue)) = device() else {
        return;
    };
    let gpu = mirrored(&device, &queue);
    let mut plain = Frame::new(&device, SIZE[0], SIZE[1]);
    let mut applied = Frame::new(&device, SIZE[0], SIZE[1]);
    let own = applied.settings();
    let change = applied.apply_settings(&device, &queue, &own);
    assert!(change.is_empty(), "{change:?}");
    assert_eq!(applied.settings(), own);
    let a = frames(&device, &queue, &mut plain, &gpu, 8);
    let b = frames(&device, &queue, &mut applied, &gpu, 8);
    for (i, (a, b)) in a.iter().zip(&b).enumerate() {
        assert!(a == b, "frame {i} differs");
    }
}

/// Changing the scale from 1.5 to 2 is reported as `Settle`, and `settings().scale` reads 2
/// after the next frame; changing the mirror budget is reported in `rebuild`, and
/// `settings()` still reads the old budget.
#[test]
fn a_scale_change_settles_and_a_budget_change_waits_for_a_rebuild() {
    let Some((device, queue)) = device() else {
        return;
    };
    let gpu = mirrored(&device, &queue);
    let internal = RenderScale::OneHalf.internal(SIZE);
    let mut frame = Frame::with_sizes(&device, internal, SIZE);
    assert_eq!(frame.settings().scale, RenderScale::OneHalf);
    frames(&device, &queue, &mut frame, &gpu, 2);
    let mut s = frame.settings();
    s.scale = RenderScale::Two;
    let budget = s.build.mirror_budget_bytes;
    s.build.mirror_budget_bytes = budget / 2;
    let change = frame.apply_settings(&device, &queue, &s);
    assert_eq!(change.settled, ["scale"]);
    assert_eq!(change.rebuild, ["build.mirror_budget_bytes"]);
    assert!(change.live.is_empty(), "{change:?}");
    frames(&device, &queue, &mut frame, &gpu, 1);
    assert_eq!(frame.settings().scale, RenderScale::Two);
    assert_eq!(frame.internal_size(), RenderScale::Two.internal(SIZE));
    assert_eq!(frame.settings().build.mirror_budget_bytes, budget);
}
