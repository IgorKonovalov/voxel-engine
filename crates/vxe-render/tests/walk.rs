//! The walk's cost on fixed rays: steps and storage reads read back from the steps view of a
//! one-pixel target, on closure worlds with axis-aligned rays from voxel centres so that
//! every `t` of the walk is exact.

use vxe_render::{
    Camera, GpuWorld, LightParams, LookParams, Marcher, ViewMode, glam::Vec3, readback, wgpu,
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

const ROCK: MaterialId = 1;

fn table() -> MaterialTable {
    let mut t = MaterialTable::new();
    t.push(Material::named("rock"));
    t
}

fn world(chunks: [i32; 3], f: impl Fn([i32; 3]) -> MaterialId + Sync) -> World {
    World::generate(ChunkExtent::new([0, 0, 0], chunks), &Voxels(f), &table())
}

/// What the steps view reads for one ray: loop iterations and the green channel's count.
#[derive(Debug, PartialEq, Eq)]
struct Cost {
    steps: u8,
    reads: u8,
}

/// The steps view's pixel for the ray from `origin` along +x, the one pixel of a 1 × 1
/// target (its centre is the camera's axis), lit by a lamp at the eye.
fn cost_x(device: &wgpu::Device, queue: &wgpu::Queue, world: &World, origin: [f32; 3]) -> Cost {
    cost(device, queue, world, origin, LightParams::at_eye())
}

fn cost(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    world: &World,
    origin: [f32; 3],
    light: LightParams,
) -> Cost {
    let mut gpu = GpuWorld::new(device, world.extent());
    gpu.sync(world, queue).expect("sync");
    let mut marcher = Marcher::new(device, 1, 1);
    marcher.set_look(LookParams {
        view: ViewMode::Steps,
        ..LookParams::default()
    });
    let camera = Camera {
        position: Vec3::from_array(origin),
        yaw: 0.0,
        pitch: 0.0,
        vfov: 1.0,
    };
    marcher.render(device, queue, &gpu, &camera, &light);
    let px = readback::read_rgba8(device, queue, marcher.target());
    Cost {
        steps: px[0],
        reads: px[1],
    }
}

/// 4 × 1 × 1 chunks: the first three uniform air, the fourth uniform rock.
fn three_air_then_rock() -> World {
    world([4, 1, 1], |v| if v[0] >= 192 { ROCK } else { AIR })
}

/// From the centre of the first chunk's -x face, the +x ray crosses three air chunks of eight
/// uniform-air bricks each and hits the rock chunk's face on its 25th step, reading no pool.
#[test]
fn a_ray_through_three_air_chunks_takes_25_steps() {
    let Some((device, queue)) = device() else {
        return;
    };
    let w = three_air_then_rock();
    assert_eq!(
        cost_x(&device, &queue, &w, [0.0, 32.5, 32.5]),
        Cost {
            steps: 25,
            reads: 0
        }
    );
}
