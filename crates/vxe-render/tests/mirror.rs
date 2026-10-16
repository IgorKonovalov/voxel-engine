//! The GPU mirror against a closure-source world: what `sync` uploads, what it frees, and
//! that the buffers hold what the CPU accounting says.

use vxe_render::gpu_world::{FULL_BIT, POOL_PAGE_SLOTS};
use vxe_render::{GpuWorld, MARCH_WGSL, readback, wgpu};
use vxe_world::*;

/// The shader parses and validates headless, without a device.
#[test]
fn march_wgsl_validates() {
    let module = wgpu::naga::front::wgsl::parse_str(MARCH_WGSL).expect("march.wgsl parses");
    wgpu::naga::valid::Validator::new(
        wgpu::naga::valid::ValidationFlags::all(),
        wgpu::naga::valid::Capabilities::default(),
    )
    .validate(&module)
    .expect("march.wgsl validates");
}

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

/// Material 1 below z = 3, air above: every brick at z in [0, 8) is `Full`.
struct Floor;

impl WorldSource for Floor {
    fn chunk(&self, coord: ChunkCoord, out: &mut Chunk) {
        let base = coord.min_voxel().0;
        for slot in 0..CHUNK_BRICKS {
            let b = brick_of_slot(slot);
            out.set_brick(
                slot,
                Brick::from_fn(|l| u16::from(base[2] + b[2] * BRICK_EDGE + l[2] < 3)),
            );
        }
    }
}

fn world() -> World {
    let mut t = MaterialTable::new();
    t.push(Material::named("floor"));
    World::generate(ChunkExtent::new([0, 0, 0], [2, 2, 1]), &Floor, &t)
}

#[test]
fn sync_uploads_dirty_bricks_once() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = world();
    // 4 chunks x 64 bricks in the z = 0 layer.
    assert_eq!(world.stats().full_bricks, 256);

    let mut gpu = GpuWorld::new(&device, world.extent());
    let first = gpu.sync(&world, &queue).unwrap();
    assert_eq!(first.bricks, 256);
    assert_eq!(first.chunks, 4);
    assert_eq!(gpu.live_slots(), 256);
    assert_eq!(gpu.pool_slots(), POOL_PAGE_SLOTS);
    for c in world.store().coords() {
        assert!(!world.store().get(c).unwrap().read().unwrap().any_dirty());
    }

    let before = gpu.bytes_written();
    let second = gpu.sync(&world, &queue).unwrap();
    assert_eq!(second.bytes, 0);
    assert_eq!(gpu.bytes_written(), before);

    // The GPU records equal the CPU entries, and a full brick's pool bytes are its voxels.
    let rec = gpu.record_index(ChunkCoord([1, 0, 0])).unwrap() as u64;
    let bytes = readback::read_buffer(&device, &queue, gpu.records(), rec * 2048, 2048);
    let entries: &[u32] = bytemuck::cast_slice(&bytes);
    for (slot, &e) in entries.iter().enumerate() {
        assert_eq!(Some(e), gpu.entry(ChunkCoord([1, 0, 0]), slot));
    }
    let e = entries[0];
    assert_ne!(e & FULL_BIT, 0);
    let pool = readback::read_buffer(
        &device,
        &queue,
        gpu.pool(),
        u64::from(e & !FULL_BIT) * 1024,
        1024,
    );
    let voxels: &[u16] = bytemuck::cast_slice(&pool);
    for (i, &v) in voxels.iter().enumerate() {
        let z = (i / 64) as i32;
        assert_eq!(v, u16::from(z < 3), "voxel {i}");
    }

    // The same slot's mask is the brick's occupancy mask, low word first.
    let mask = world
        .store()
        .get(ChunkCoord([1, 0, 0]))
        .unwrap()
        .read()
        .unwrap()
        .brick(0)
        .occupancy_mask();
    assert_ne!(mask, 0);
    let bytes = readback::read_buffer(
        &device,
        &queue,
        gpu.masks(),
        u64::from(e & !FULL_BIT) * 8,
        8,
    );
    assert_eq!(
        bytemuck::cast_slice::<u8, u32>(&bytes),
        [mask as u32, (mask >> 32) as u32]
    );
}

#[test]
fn a_collapsed_brick_frees_its_slot_for_the_next_expansion() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = world();
    let mut gpu = GpuWorld::new(&device, world.extent());
    gpu.sync(&world, &queue).unwrap();
    let slot0 = gpu.entry(ChunkCoord([0, 0, 0]), 0).unwrap() & !FULL_BIT;

    // Carve the floor voxels of brick [0, 8)^3: it collapses to air.
    let r = world
        .apply(&EditOp::box_(
            Fixed3([0; 3]),
            Fixed3([2048; 3]),
            Mode::Carve,
        ))
        .unwrap();
    assert_eq!(r.changed_from, [(1, 192)]);
    let s = gpu.sync(&world, &queue).unwrap();
    assert_eq!(s.bricks, 0);
    assert_eq!(gpu.entry(ChunkCoord([0, 0, 0]), 0), Some(0));
    assert_eq!(gpu.free_slots(), [slot0]);
    assert_eq!(gpu.live_slots(), 255);

    // Fill a voxel of an air brick in another chunk: it expands into the freed slot.
    let target = brick_slot([0, 0, 1]);
    let r = world
        .apply(&EditOp::box_(
            centre_of_voxel([64, 0, 8]),
            centre_of_voxel([65, 1, 9]),
            Mode::Fill(1),
        ))
        .unwrap();
    assert_eq!(r.changed_from, [(0, 1)]);
    gpu.sync(&world, &queue).unwrap();
    assert_eq!(
        gpu.entry(ChunkCoord([1, 0, 0]), target),
        Some(FULL_BIT | slot0)
    );
    assert!(gpu.free_slots().is_empty());

    let rec = gpu.record_index(ChunkCoord([0, 0, 0])).unwrap() as u64;
    let bytes = readback::read_buffer(&device, &queue, gpu.records(), rec * 2048, 4);
    assert_eq!(bytemuck::cast_slice::<u8, u32>(&bytes), [0]);
}
