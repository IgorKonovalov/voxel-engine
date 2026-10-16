//! The mirror on the level-size cave (seed 1, 20 x 20 x 8 chunks): how many queue writes the
//! first sync issues, and that a byte budget bounds every sync.

use flythrough::cave::{CaveSource, LEVEL_CHUNKS};
use flythrough::cave_materials;
use vxe_render::gpu_world::BRICK_BYTES;
use vxe_render::{GpuWorld, MirrorConfig, Window, wgpu};
use vxe_world::*;

fn level() -> (World, CaveSource) {
    let source = CaveSource::new(1, ChunkExtent::new([0, 0, 0], LEVEL_CHUNKS));
    (
        World::generate(source.extent, &source, &cave_materials()),
        source,
    )
}

/// A device on an adapter that can bind a pool of `pool_bytes` in one buffer, or `None` with
/// a printed skip when no adapter can.
fn device(pool_bytes: u64) -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN));
    let fits = |a: &wgpu::Adapter| {
        let l = a.limits();
        l.max_storage_buffer_binding_size.min(l.max_buffer_size) >= pool_bytes
    };
    let Some(adapter) = adapters.into_iter().find(fits) else {
        println!("skipped: no adapter binds a {pool_bytes} B pool");
        return None;
    };
    println!("adapter: {}", adapter.get_info().name);
    let desc = wgpu::DeviceDescriptor {
        required_limits: vxe_render::limits_for(&adapter, pool_bytes),
        ..Default::default()
    };
    Some(pollster::block_on(adapter.request_device(&desc)).expect("device"))
}

#[test]
fn the_first_sync_issues_at_most_three_writes_per_chunk() {
    let (world, source) = level();
    let s = world.stats();
    let Some((device, queue)) = device(s.full_bricks as u64 * BRICK_BYTES) else {
        return;
    };
    let mut gpu = GpuWorld::new(&device, world.extent());
    gpu.set_focus(source.spawn());
    let st = gpu.sync(&world, &queue).unwrap();
    println!(
        "first sync: {} writes, {} chunks, {} bricks, {} B",
        st.writes, st.chunks, st.bricks, st.bytes
    );
    assert_eq!(st.chunks, 3200);
    assert_eq!(st.bricks, s.full_bricks as u64);
    assert!(st.writes <= 3 * 3200 + 8, "{} writes", st.writes);
    assert_eq!(st.pending_chunks, 0);
}

#[test]
fn every_budgeted_sync_writes_at_most_the_budget_plus_one_chunk() {
    const BUDGET: u64 = 8 << 20;
    // A wide chunk's upload: 512 two-slot bricks and its record with palette.
    const LARGEST_CHUNK: u64 = 512 * 1024 + 2560;
    let (world, source) = level();
    let Some((device, queue)) = device(world.stats().full_bricks as u64 * BRICK_BYTES) else {
        return;
    };
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            window: Window::Fixed(world.extent()),
            sync_budget_bytes: BUDGET,
        },
    );
    gpu.set_focus(source.spawn());
    let spawn_chunk = source.spawn().chunk();
    let mut syncs = 0;
    let mut largest = 0;
    loop {
        let st = gpu.sync(&world, &queue).unwrap();
        syncs += 1;
        largest = largest.max(st.bytes);
        assert!(
            st.bytes <= BUDGET + LARGEST_CHUNK,
            "sync {syncs} wrote {} B",
            st.bytes
        );
        if syncs == 1 {
            assert!(gpu.is_mirrored(spawn_chunk), "the focus chunk comes first");
        }
        if st.pending_chunks == 0 {
            break;
        }
        assert!(syncs < 1000);
    }
    println!("{syncs} syncs, largest {largest} B");
    assert_eq!(gpu.mirrored_chunks(), 3200);
    assert!(syncs > 1);
}
