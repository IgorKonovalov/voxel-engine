//! The GPU mirror's allocation against a closure-source world: what `sync` uploads into which
//! slot, bin and page, what it frees, the shard and record layout the limits set, and that
//! the buffers hold what the CPU accounting says.

mod support;

use support::*;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use vxe_render::{GpuWorld, MirrorConfig, ShardLayout, Window, wgpu};
use vxe_render::{
    internals::BIN_COUNT, internals::BINS, internals::BITS_WORDS, internals::CLASS_SHIFT,
    internals::CLASS_UNIFORM, internals::CLASS_WIDE, internals::EMPTY_BIT, internals::ENTRY_WORD,
    internals::FULL_BIT, internals::HEADER_BAND_BIT, internals::HEADER_BAND_PREFIX_WORD,
    internals::HEADER_BAND_WORD, internals::HEADER_TRANSLUCENT_WORD, internals::PAGE_BYTES,
    internals::PAGE_UNITS, internals::PLACEHOLDER_BYTES, internals::RECORD_BYTES,
    internals::RECORD_WORDS, internals::TRANSLUCENT_BIT, internals::UNIT_BYTES,
    internals::UNIT_MASK, internals::UNIT_WORDS, internals::band_bin_of, internals::bin_of,
    internals::payload_units, internals::slots_per_page,
};
use vxe_world::packed::PackedClass;
use vxe_world::*;

/// The source `label` of the renderer's list, composed as its pipeline composes it.
fn composed(label: &str) -> String {
    vxe_render::internals::shader_sources()
        .into_iter()
        .find(|s| s.label == label)
        .unwrap_or_else(|| panic!("the renderer composes a {label} pass"))
        .source
}

/// Parses and validates `source` with naga, panicking with its name on a failure.
fn validate(name: &str, source: &str) {
    let module = wgpu::naga::front::wgsl::parse_str(source)
        .unwrap_or_else(|e| panic!("{name} parses: {}", e.emit_to_string(source)));
    wgpu::naga::valid::Validator::new(
        wgpu::naga::valid::ValidationFlags::all(),
        wgpu::naga::valid::Capabilities::default(),
    )
    .validate(&module)
    .unwrap_or_else(|e| panic!("{name} validates: {e:?}"));
}

/// The frame's shaders parse and validate headless, without a device: the hit pass and each
/// pass that reads the walk or the hit record, composed as the renderer composes them.
#[test]
fn march_wgsl_validates() {
    for name in [
        "hit",
        "light",
        "composite",
        "temporal",
        "atrous",
        "gradient",
    ] {
        validate(name, &composed(name));
    }
}

/// The field pass's shader parses and validates headless, after the prelude and the mirror's
/// decode.
#[test]
fn field_wgsl_validates() {
    validate("field", &composed("field"));
}

/// The units the slot of the `Full` brick at `e` takes, by its bin.
fn slot_units_of(gpu: &GpuWorld, e: u32) -> u32 {
    BINS[gpu
        .bin_of_unit(e & UNIT_MASK)
        .expect("a live slot's page has a bin")]
}

#[test]
fn sync_uploads_dirty_bricks_once() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = world();
    // 4 chunks x 64 bricks in the z = 0 layer.
    assert_eq!(world.stats().full_bricks, 256);

    let mut gpu = GpuWorld::with_config(&device, small(Window::Fixed(world.extent())));
    let first = gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(first.bricks, 256);
    assert_eq!(first.chunks, 4);
    assert_eq!(gpu.live_bricks(), 256);
    // One-material bricks: two units each, all in one page of the 2-unit bin.
    assert_eq!(gpu.live_units(), 512);
    assert_eq!(gpu.pool_units(), PAGE_UNITS);
    assert!(gpu.reserved_units() > PAGE_UNITS);
    // Allocation is the reserved buffers' sizes, and covers the live accounting.
    let allocated: u64 =
        (0..4).map(|k| gpu.pool_shard(k).size()).sum::<u64>() + gpu.records().size();
    assert_eq!(gpu.allocated_bytes(), allocated);
    assert_eq!(gpu.allocated_bytes(), gpu.shard_layout().reserved_bytes());
    assert!(gpu.allocated_bytes() >= gpu.mirror_bytes());
    for c in world.store().coords() {
        assert!(!world.store().get(c).unwrap().read().unwrap().any_dirty());
    }

    // The dirty-chunk set is empty: an idle sync visits no chunk and writes nothing.
    assert_eq!(world.store().dirty_len(), 0);
    let before = gpu.bytes_written();
    let second = gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(second.bytes, 0);
    assert_eq!(second.chunks_visited, 0);
    assert_eq!(second.writes, 0);
    assert_eq!(gpu.bytes_written(), before);

    // The GPU records equal the CPU entries, and carry no palette.
    let rec = gpu.record_index(ChunkCoord([1, 0, 0])).unwrap() as u64;
    let bytes = vxe_render::debug::read_buffer(
        &device,
        &queue,
        gpu.records(),
        rec * RECORD_BYTES,
        RECORD_BYTES,
    );
    let words: &[u32] = bytemuck::cast_slice(&bytes);
    assert_eq!(words.len(), RECORD_WORDS);
    for (slot, &e) in words[ENTRY_WORD..].iter().enumerate() {
        assert_eq!(Some(e), gpu.entry(ChunkCoord([1, 0, 0]), slot));
    }
    // Only the z = 0 layer of bricks is not uniform air: bricks 0..64, words 0 and 1.
    assert_eq!(
        words[..BITS_WORDS],
        [!0, !0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
    );
    let e = words[ENTRY_WORD];
    assert_eq!(e & FULL_BIT, FULL_BIT);
    assert_eq!(class_of(e), 1, "one material, no plane");
    // The slot: its opaque mask is the brick's solid bits, its header's palette the one
    // material, its translucent words clear.
    let u = u64::from(e & UNIT_MASK);
    let slot = vxe_render::debug::read_buffer(
        &device,
        &queue,
        gpu.pool_shard(0),
        u * UNIT_BYTES,
        2 * UNIT_BYTES,
    );
    let slot: Vec<u32> = bytemuck::pod_collect_to_vec(&slot);
    let bits = world
        .store()
        .get(ChunkCoord([1, 0, 0]))
        .unwrap()
        .read()
        .unwrap()
        .brick(0)
        .solid_bits();
    assert_eq!(bits[..3], [u64::MAX; 3]);
    assert_eq!(bits[3..], [0; 5]);
    assert_eq!(layers(&slot[..UNIT_WORDS]), bits);
    assert_eq!(slot[UNIT_WORDS], 1, "palette entry 0 is material 1");
    assert_eq!(slot[UNIT_WORDS + 1..UNIT_WORDS + 8], [0; 7]);
    let t = UNIT_WORDS + HEADER_TRANSLUCENT_WORD;
    assert_eq!(slot[t..t + 2], [0, 0]);
}

/// The slot layout's arithmetic: units are 64-byte lines, a page 4096 of them; a slot takes
/// the smallest bin that holds it, and a page of bin `b` holds `4096 / b` slots.
#[test]
fn slots_take_the_smallest_bin_that_holds_them() {
    assert_eq!(UNIT_BYTES, 64);
    assert_eq!(PAGE_BYTES, 256 << 10);
    assert_eq!(RECORD_BYTES, 2112);
    assert_eq!(BINS, [2, 3, 4, 6, 8, 12, 16, 20, 24, 36]);
    for (units, bin) in [
        (2, 2),
        (3, 3),
        (4, 4),
        (5, 6),
        (6, 6),
        (18, 20),
        (19, 20),
        (22, 24),
        (34, 36),
    ] {
        assert_eq!(BINS[bin_of(units)], bin, "{units} units");
    }
    assert_eq!(slots_per_page(bin_of(3)), 1365);
    assert_eq!(slots_per_page(bin_of(20)), 204);
    // The payload by class code.
    assert_eq!(
        (1..=5).map(payload_units).collect::<Vec<_>>(),
        [0, 1, 2, 4, 16]
    );
}

#[test]
fn a_collapsed_brick_frees_its_slot_for_the_next_expansion() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = world();
    let mut gpu = GpuWorld::with_config(&device, small(Window::Fixed(world.extent())));
    gpu.sync_now(&world, &device, &queue).unwrap();
    let unit0 = gpu.entry(ChunkCoord([0, 0, 0]), 0).unwrap() & UNIT_MASK;
    let k = bin_of(2);

    // Carve the floor voxels of brick [0, 8)^3: it collapses to air.
    let r = world
        .apply(&EditOp::box_(
            Fixed3([0; 3]),
            Fixed3([2048; 3]),
            Mode::Carve,
        ))
        .unwrap();
    assert_eq!(r.changed_from, [(1, 192)]);
    let s = gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(s.bricks, 0);
    assert_eq!(gpu.entry(ChunkCoord([0, 0, 0]), 0), Some(0));
    assert_eq!(gpu.free_slots(k), [unit0]);
    assert_eq!(gpu.live_bricks(), 255);

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
    gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(
        gpu.entry(ChunkCoord([1, 0, 0]), target),
        Some(FULL_BIT | 1 << CLASS_SHIFT | unit0)
    );
    assert!(gpu.free_slots(k).is_empty());

    let rec = gpu.record_index(ChunkCoord([0, 0, 0])).unwrap() as u64;
    let at = rec * RECORD_BYTES + ENTRY_WORD as u64 * 4;
    let bytes = vxe_render::debug::read_buffer(&device, &queue, gpu.records(), at, 4);
    assert_eq!(bytemuck::cast_slice::<u8, u32>(&bytes), [0]);
}

/// The chunk table entry the GPU holds for `coord`.
fn table_entry(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    gpu: &GpuWorld,
    coord: ChunkCoord,
) -> u32 {
    let at = u64::from(gpu.table_index(coord)) * 4;
    let bytes = vxe_render::debug::read_buffer(device, queue, gpu.table(), at, 4);
    bytemuck::cast_slice::<u8, u32>(&bytes)[0]
}

/// A mirrored chunk of uniform-air bricks carries `EMPTY_BIT`; a one-voxel fill clears it at
/// the next sync, and carving the voxel away sets it again, the record index unchanged.
#[test]
fn an_edit_in_an_empty_chunk_toggles_its_empty_bit() {
    let Some((device, queue)) = device() else {
        return;
    };
    // The Floor world over two layers of chunks: the z = 1 layer is all air.
    let mut t = MaterialTable::new();
    t.push(Material::named("floor"));
    let world = World::generate(ChunkExtent::new([0, 0, 0], [1, 1, 2]), &Floor, &t);
    let mut gpu = GpuWorld::with_config(&device, small(Window::Fixed(world.extent())));
    gpu.sync_now(&world, &device, &queue).unwrap();
    let (floor, air) = (ChunkCoord([0, 0, 0]), ChunkCoord([0, 0, 1]));
    let rec = gpu.record_index(air).unwrap();
    assert_eq!(table_entry(&device, &queue, &gpu, air), rec | EMPTY_BIT);
    assert_eq!(
        table_entry(&device, &queue, &gpu, floor),
        gpu.record_index(floor).unwrap()
    );

    fill_voxel(&world, [5, 6, 70], 1);
    gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(table_entry(&device, &queue, &gpu, air), rec);

    let r = world
        .apply(&EditOp::box_(
            centre_of_voxel([5, 6, 70]),
            centre_of_voxel([6, 7, 71]),
            Mode::Carve,
        ))
        .unwrap();
    assert_eq!(r.changed_from, [(1, 1)]);
    gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(table_entry(&device, &queue, &gpu, air), rec | EMPTY_BIT);
    assert_eq!(gpu.record_index(air), Some(rec));
}

/// Chunk `[0, 0, 0]` cycles through 300 non-air materials voxel by voxel; chunk `[1, 0, 0]`
/// holds materials 7 and 9 below z = 4 and air above.
struct ManyAndTwo;

impl WorldSource for ManyAndTwo {
    fn chunk(&self, coord: ChunkCoord, out: &mut Chunk) {
        let base = coord.min_voxel().0;
        for slot in 0..CHUNK_BRICKS {
            let b = brick_of_slot(slot);
            out.set_brick(
                slot,
                Brick::from_fn(|l| {
                    let v = [0, 1, 2].map(|k| base[k] + b[k] * BRICK_EDGE + l[k]);
                    if coord.0[0] == 0 {
                        1 + ((v[0] + 64 * v[1] + 4096 * v[2]) % 300) as u16
                    } else if v[2] < 4 {
                        if v[0] % 2 == 0 { 7 } else { 9 }
                    } else {
                        0
                    }
                }),
            );
        }
    }
}

/// A chunk of 300 materials and its two-material neighbour need no chunk palette: every
/// brick of the first holds more than 16 materials and stores its `u16` ids (class Wide, 18
/// units in the 20-unit bin), every `Full` brick of the second one plane (3 units), and both
/// decode to their voxels.
#[test]
fn bricks_past_sixteen_materials_store_their_ids_beside_planar_neighbours() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [2, 1, 1]),
        &ManyAndTwo,
        &numbered_table(300),
    );
    let (many, two) = (ChunkCoord([0, 0, 0]), ChunkCoord([1, 0, 0]));
    let mut gpu = GpuWorld::with_config(&device, small(Window::Fixed(world.extent())));
    gpu.sync_now(&world, &device, &queue).unwrap();
    let rb = Readback::new(&device, &queue, &gpu);
    assert_eq!(rb.assert_chunk(&gpu, &world, many).len(), 512);
    for slot in 0..512 {
        let e = gpu.entry(many, slot).unwrap();
        assert_eq!(class_of(e), CLASS_WIDE, "brick {slot}");
        assert_eq!(slot_units_of(&gpu, e), 20, "brick {slot}");
    }
    let n = full_bricks(&world, &[two]);
    assert_eq!(rb.assert_chunk(&gpu, &world, two).len() as u32, n);
    for slot in 0..512 {
        let e = gpu.entry(two, slot).unwrap();
        if e & FULL_BIT != 0 {
            assert_eq!(class_of(e), 2, "brick {slot}");
            assert_eq!(slot_units_of(&gpu, e), 3, "brick {slot}");
        }
    }
    assert_eq!(gpu.live_units(), 512 * 20 + u64::from(n) * 3);
}

/// Fills one voxel `v` with `m`.
fn fill_voxel(world: &World, v: [i32; 3], m: MaterialId) {
    let r = world
        .apply(&EditOp::box_(
            centre_of_voxel(v),
            centre_of_voxel(v.map(|c| c + 1)),
            Mode::Fill(m),
        ))
        .unwrap();
    assert_eq!(r.changed_from, [(0, 1)], "fill at {v:?}");
}

/// Syncs whose encoders are submitted and waited for reuse one belt chunk; syncs recorded
/// before any of them is submitted each take a chunk of their own.
#[test]
fn submitted_syncs_reuse_one_belt_chunk() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = world();
    let mut gpu = GpuWorld::with_config(&device, small(Window::Fixed(world.extent())));
    assert_eq!(gpu.belt_chunks(), 0);
    gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(gpu.belt_chunks(), 1);
    for x in 0..8 {
        fill_voxel(&world, [x * 3, 1, 3], 1);
        let st = gpu.sync_now(&world, &device, &queue).unwrap();
        assert_eq!(st.bricks, 1, "edit {x}");
        assert_eq!(gpu.belt_chunks(), 1, "edit {x}");
    }
    let mut encs = Vec::new();
    for x in 0..2 {
        fill_voxel(&world, [x * 3, 5, 3], 1);
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        gpu.sync(&world, &queue, &mut enc).unwrap();
        encs.push(enc.finish());
    }
    assert_eq!(gpu.belt_chunks(), 2);
    queue.submit(encs);
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    assert_eq!(gpu.belt_trims(), 0);
}

/// An edit that adds a third material to a two-material brick moves it, in one sync, from
/// the 3-unit bin to the 4-unit bin: its entry names the new slot and its class, the old
/// slot's unit is on the 3-unit free list, and the slot decodes to the edited voxels.
#[test]
fn a_brick_gaining_a_material_moves_to_the_next_bin_in_one_sync() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [1, 1, 1]),
        &Floor,
        &numbered_table(3),
    );
    let c = ChunkCoord([0, 0, 0]);
    // Material 2 above the floor of bricks 0 and 1: two materials, one plane; brick 1 keeps
    // the 3-unit page live.
    let r = world
        .apply(&EditOp::box_(
            centre_of_voxel([0, 0, 4]),
            centre_of_voxel([16, 8, 6]),
            Mode::Fill(2),
        ))
        .unwrap();
    assert_eq!(r.changed_from, [(0, 16 * 8 * 2)]);
    let mut gpu = GpuWorld::with_config(&device, small(Window::Fixed(world.extent())));
    gpu.sync_now(&world, &device, &queue).unwrap();
    let before = gpu.entry(c, 0).unwrap();
    assert_eq!(class_of(before), 2);
    assert_eq!(slot_units_of(&gpu, before), 3);
    let (three, four) = (bin_of(3), bin_of(4));
    assert!(gpu.free_slots(three).is_empty());

    fill_voxel(&world, [3, 3, 7], 3);
    let st = gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!((st.chunks, st.bricks), (1, 1));
    let after = gpu.entry(c, 0).unwrap();
    assert_eq!(class_of(after), 3, "two planes");
    assert_eq!(slot_units_of(&gpu, after), 4);
    assert_eq!(gpu.bin_of_unit(after & UNIT_MASK), Some(four));
    assert_ne!(after & UNIT_MASK, before & UNIT_MASK);
    assert_eq!(gpu.free_slots(three), [before & UNIT_MASK]);
    assert_eq!(gpu.bin_stats()[three].live, 1);
    assert_eq!(gpu.bin_stats()[four].live, 1);
    let rb = Readback::new(&device, &queue, &gpu);
    rb.assert_chunk(&gpu, &world, c);
}

#[test]
fn the_shard_layout_follows_the_limits_alone() {
    // lavapipe: 128 MiB per binding.
    let l = ShardLayout::from_limits(&limits(128 << 20, 2_147_483_647, 10), None);
    assert_eq!(l.units_per_shard, 2_097_152);
    assert_eq!(l.capacity(), 4 * 2_097_152);
    // RTX 3080 Laptop and RADV: 2 147 483 644 B per binding, the largest page multiple below.
    let l = ShardLayout::from_limits(&limits(2_147_483_644, 4_294_967_295, 9), None);
    assert_eq!(l.units_per_shard, 8191 * 4096);
    assert!(u64::from(l.units_per_shard) * 64 <= 2_147_483_644);
    // The reservation stops at what a 26-bit unit index addresses: 4 GiB of units.
    assert_eq!(l.reserved_units, 1 << 26);
    // The smaller of the two size limits binds.
    let l = ShardLayout::from_limits(&limits(1 << 30, 100 << 20, 8), None);
    assert_eq!(l.units_per_shard, 1_638_400);
    // The test knob caps units per shard, rounded down to a page.
    let l = ShardLayout::from_limits(&limits(128 << 20, 1 << 31, 10), Some(10_000));
    assert_eq!(l.units_per_shard, 8192);
    assert_eq!(l.capacity(), 4 * 8192);
}

#[test]
fn the_record_capacity_follows_the_limits() {
    // lavapipe: 128 MiB per binding holds 63 550 records of 2112 B.
    assert_eq!(RECORD_BYTES, 2112);
    let l = ShardLayout::from_limits(&limits(128 << 20, 2_147_483_647, 8), None);
    assert_eq!(l.record_capacity, 63_550);
    let l = ShardLayout::from_limits(&limits(4 << 20, 2_147_483_647, 8), None);
    assert_eq!(l.record_capacity, 1985);
    // The smaller of the two size limits binds.
    let l = ShardLayout::from_limits(&limits(1 << 30, 4 << 20, 8), None);
    assert_eq!(l.record_capacity, 1985);
}

/// More chunks than a 4 MiB binding holds records for: the first sync uploads the records that
/// fit, nearest first, and the next, with nothing freed, fails before writing anything.
#[test]
fn records_past_the_binding_fail_and_leave_the_mirror_untouched() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        force_fallback_adapter: true,
        ..Default::default()
    }))
    .or_else(|_| pollster::block_on(instance.request_adapter(&Default::default())));
    let Ok(adapter) = adapter else {
        println!("skipped: no adapter");
        return;
    };
    let mut required = vxe_render::limits_for(&adapter, 16 << 20).expect("the frame's limits");
    required.max_storage_buffer_binding_size = 4 << 20;
    let desc = wgpu::DeviceDescriptor {
        required_limits: required,
        ..Default::default()
    };
    let Ok((device, queue)) = pollster::block_on(adapter.request_device(&desc)) else {
        println!("skipped: the adapter refuses a 4 MiB storage binding limit");
        return;
    };
    assert_eq!(device.limits().max_storage_buffer_binding_size, 4 << 20);

    // 13 x 13 x 13 = 2197 uniform chunks: no `Full` brick, one record each.
    let mut t = MaterialTable::new();
    let rock = t.push(Material::named("rock"));
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [13, 13, 13]),
        &UniformSource(rock),
        &t,
    );
    assert_eq!(world.store().coords().len(), 2197);
    let mut gpu = GpuWorld::new(&device, world.extent());
    assert_eq!(gpu.shard_layout().record_capacity, 1985);
    partial_then_empty(
        &mut gpu,
        &world,
        &device,
        &queue,
        1985,
        212,
        vxe_render::MirrorError::RecordsTooLarge {
            needed: 1986 * RECORD_BYTES,
            limit: 1985 * RECORD_BYTES,
        },
    );

    // A budget holding 1000 records, and no page, binds before the device does.
    let budget = 1000 * RECORD_BYTES + PLACEHOLDER_BYTES;
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            budget_bytes: budget,
            ..MirrorConfig::new(Window::Fixed(world.extent()))
        },
    );
    assert_eq!(gpu.reserved_records(), 1000);
    assert_eq!(gpu.reserved_units(), 0);
    assert!(gpu.allocated_bytes() <= budget);
    partial_then_empty(
        &mut gpu,
        &world,
        &device,
        &queue,
        1000,
        1197,
        vxe_render::MirrorError::RecordsTooLarge {
            needed: 1001 * RECORD_BYTES,
            limit: 1000 * RECORD_BYTES,
        },
    );
}

/// The mirror's sizing code names no backend, vendor or adapter: only limits decide.
#[test]
fn the_mirror_never_reads_the_backend_or_vendor() {
    for (name, text) in [
        ("gpu_world/mod.rs", include_str!("../src/gpu_world/mod.rs")),
        (
            "gpu_world/alloc.rs",
            include_str!("../src/gpu_world/alloc.rs"),
        ),
        (
            "gpu_world/format.rs",
            include_str!("../src/gpu_world/format.rs"),
        ),
        (
            "gpu_world/lights.rs",
            include_str!("../src/gpu_world/lights.rs"),
        ),
        (
            "gpu_world/slot.rs",
            include_str!("../src/gpu_world/slot.rs"),
        ),
        (
            "gpu_world/stage.rs",
            include_str!("../src/gpu_world/stage.rs"),
        ),
        (
            "gpu_world/window.rs",
            include_str!("../src/gpu_world/window.rs"),
        ),
        ("frame/baked.rs", include_str!("../src/frame/baked.rs")),
        ("frame/compose.rs", include_str!("../src/frame/compose.rs")),
        ("frame/encode.rs", include_str!("../src/frame/encode.rs")),
        ("frame/marcher.rs", include_str!("../src/frame/marcher.rs")),
        ("frame/mod.rs", include_str!("../src/frame/mod.rs")),
        (
            "frame/pipelines.rs",
            include_str!("../src/frame/pipelines.rs"),
        ),
        (
            "frame/probe_channel.rs",
            include_str!("../src/frame/probe_channel.rs"),
        ),
        (
            "frame/readback_ring.rs",
            include_str!("../src/frame/readback_ring.rs"),
        ),
        (
            "frame/settings.rs",
            include_str!("../src/frame/settings.rs"),
        ),
        ("frame/state.rs", include_str!("../src/frame/state.rs")),
        ("frame/targets.rs", include_str!("../src/frame/targets.rs")),
        ("frame/timer.rs", include_str!("../src/frame/timer.rs")),
        ("frame/upscale.rs", include_str!("../src/frame/upscale.rs")),
        ("gpu_util.rs", include_str!("../src/gpu_util.rs")),
        ("lib.rs", include_str!("../src/lib.rs")),
    ] {
        for word in ["get_info", "Backend", "backend(", "vendor", "AdapterInfo"] {
            let hits = text
                .lines()
                .filter(|l| !l.trim_start().starts_with("//"))
                .filter(|l| l.contains(word))
                .count();
            assert_eq!(hits, 0, "{name} uses {word}");
        }
    }
}

#[test]
fn a_world_past_four_shards_fails_and_leaves_the_mirror_untouched() {
    let Some((device, queue)) = device() else {
        return;
    };
    // 36 chunks of 512 Full bricks; four one-page shards hold 4 x 1365 slots, 10 chunks.
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [4, 3, 3]),
        &Checker,
        &numbered_table(2),
    );
    assert_eq!(world.stats().full_bricks, 18_432);
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            max_shard_units: Some(PAGE_UNITS),
            ..MirrorConfig::new(Window::Fixed(world.extent()))
        },
    );
    assert_eq!(gpu.shard_layout().capacity(), 4 * 4096);
    // 10 chunks fill the four shards; the 26 farthest wait, and the next sync is empty.
    partial_then_empty(
        &mut gpu,
        &world,
        &device,
        &queue,
        10,
        26,
        vxe_render::MirrorError::PoolTooLarge {
            needed: 5 * PAGE_BYTES,
            limit: 4 * PAGE_BYTES,
        },
    );
    assert_eq!(gpu.live_bricks(), 5120);
    assert_eq!(gpu.live_units(), 5120 * 3);
    assert_eq!(gpu.shards_in_use(), 4);
}

/// Bytes `len` at word `w` of the slot whose first unit is `u`.
fn slot_bytes(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    gpu: &GpuWorld,
    u: u32,
    w: u64,
    len: u64,
) -> Vec<u8> {
    let ups = gpu.shard_layout().units_per_shard;
    vxe_render::debug::read_buffer(
        device,
        queue,
        gpu.pool_shard((u / ups) as usize),
        u64::from(u % ups) * UNIT_BYTES + w * 4,
        len,
    )
}

/// The bound on pages ever issued for each bin's peak live slots: per bin with a slot, the
/// pages its peak fills and one more.
fn page_bound(peak: &[u32; BIN_COUNT]) -> u32 {
    peak.iter()
        .enumerate()
        .filter(|(_, n)| **n > 0)
        .map(|(k, &n)| n.div_ceil(slots_per_page(k)) + 1)
        .sum()
}

/// Bin churn in a following window: 8 chunks alternate 20 times between full with the
/// field's band slots and LOD-only (2- and 3-unit slots), beside 2 chunks that stay full. The
/// pages ever issued stay within the per-bin bound, the stable chunks' bricks never change
/// slot, and after the churn every live brick and mip-brick decodes to its CPU voxels and the
/// churned chunks' band slots (mask and counts) equal a fresh mirror's.
#[test]
fn bin_churn_reuses_empty_pages_and_moves_no_live_brick() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [5, 2, 1]),
        &Mixed,
        &mixed_table(),
    );
    let (stable, churn): (Vec<ChunkCoord>, Vec<ChunkCoord>) = world
        .store()
        .coords()
        .into_iter()
        .partition(|c| c.0[0] == 0);
    assert_eq!((stable.len(), churn.len()), (2, 8));
    let mips: HashMap<ChunkCoord, Arc<ChunkMips>> = churn
        .iter()
        .map(|&c| {
            let chunk = world.store().get(c).unwrap();
            let mips = ChunkMips::build(&chunk.read().unwrap());
            (c, Arc::new(mips))
        })
        .collect();
    let mut gpu = GpuWorld::with_field_surface(&device, small(Window::Follow { side: 7 }));
    gpu.set_focus(VoxelCoord([2 * 64 + 1, 64 + 1, 1]));
    gpu.sync_now(&world, &device, &queue).unwrap();
    update_field(&device, &queue, &mut gpu);
    let entries = |gpu: &GpuWorld| -> Vec<Option<u32>> {
        stable
            .iter()
            .flat_map(|&c| (0..CHUNK_BRICKS).map(move |i| (c, i)))
            .map(|(c, i)| gpu.entry(c, i))
            .collect()
    };
    let stable_entries = entries(&gpu);
    let mut peak = gpu.bin_stats().map(|b| b.live);
    let mut held = HashMap::new();
    for round in 0..20 {
        let lod = round % 2 == 0;
        for &c in &churn {
            if lod {
                held.insert(c, world.store().remove(c).unwrap());
                world.insert_lod(c, mips[&c].clone());
            } else {
                world.remove_lod(c);
                world
                    .store()
                    .write_lock()
                    .insert(c, held.remove(&c).unwrap());
            }
        }
        let st = gpu.sync_now(&world, &device, &queue).unwrap();
        assert_eq!(st.pending_chunks, 0);
        update_field(&device, &queue, &mut gpu);
        for &c in &churn {
            assert_eq!(gpu.is_lod(c), lod, "round {round} {c:?}");
        }
        for (p, b) in peak.iter_mut().zip(gpu.bin_stats()) {
            *p = (*p).max(b.live);
        }
        let bound = page_bound(&peak);
        assert!(
            gpu.pages_issued() <= bound,
            "round {round}: {} pages issued, bound {bound} for peaks {peak:?}",
            gpu.pages_issued()
        );
        assert_eq!(entries(&gpu), stable_entries, "round {round}");
        assert_units_accounted(&gpu);
    }
    println!(
        "{} pages issued for peaks {peak:?}, {} free",
        gpu.pages_issued(),
        gpu.free_pages().len()
    );

    let rb = Readback::new(&device, &queue, &gpu);
    let mut live = HashSet::new();
    for c in world.store().coords() {
        for u in rb.assert_chunk(&gpu, &world, c) {
            assert!(live.insert(u), "unit {u} held twice");
        }
    }
    assert_eq!(live.len() as u32, gpu.live_bricks());
    let mut fresh = GpuWorld::with_field_surface(&device, small(Window::Follow { side: 7 }));
    fresh.set_focus(VoxelCoord([2 * 64 + 1, 64 + 1, 1]));
    fresh.sync_now(&world, &device, &queue).unwrap();
    update_field(&device, &queue, &mut fresh);
    let rb_fresh = Readback::new(&device, &queue, &fresh);
    let mut compared = 0;
    for &c in &churn {
        for slot in 0..CHUNK_BRICKS {
            let band = rb.band(&gpu, c, slot);
            assert_eq!(band, rb_fresh.band(&fresh, c, slot), "{c:?} brick {slot}");
            compared += usize::from(band.is_some());
        }
    }
    assert!(compared > 20, "{compared} band slots compared");
}

/// A page whose last brick leaves joins the free pages and is next issued to a different bin
/// on demand: 8 checkerboard chunks fill pages 0 to 3 of the 3-unit bin; evicted, the 4 pages
/// are free; `Mixed` mips at the same coordinates then take 2- and 3-unit slots from those
/// pages with no new page, one of them now in the 2-unit bin; and the checkerboard chunks
/// coming back take the pages again with none past the four.
#[test]
fn an_emptied_page_is_issued_to_another_bin() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [4, 2, 1]),
        &Checker,
        &numbered_table(2),
    );
    let coords = world.store().coords();
    assert_eq!(coords.len(), 8);
    let mut gpu = GpuWorld::with_config(&device, small(Window::Fixed(world.extent())));
    gpu.sync_now(&world, &device, &queue).unwrap();
    let three = bin_of(3);
    assert_eq!(gpu.bin_stats()[three].live, 8 * 512);
    assert_eq!(gpu.pages_issued(), 4);
    for p in 0..4 {
        assert_eq!(gpu.bin_of_unit(p * PAGE_UNITS), Some(three), "page {p}");
    }
    assert!(gpu.free_pages().is_empty());

    let mut held = Vec::new();
    for &c in &coords {
        held.push((c, world.store().remove(c).unwrap()));
    }
    gpu.apply_events(&StreamEvents {
        evicted: coords.clone(),
        ..StreamEvents::default()
    });
    gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(gpu.live_bricks(), 0);
    assert_eq!(gpu.free_pages(), [0, 1, 2, 3]);
    assert_eq!(gpu.spare_units(), 0);
    for p in 0..4 {
        assert_eq!(gpu.bin_of_unit(p * PAGE_UNITS), None, "page {p} is free");
    }

    let mixed = World::generate(world.extent(), &Mixed, &mixed_table());
    for &c in &coords {
        let chunk = mixed.store().get(c).unwrap();
        world.insert_lod(c, Arc::new(ChunkMips::build(&chunk.read().unwrap())));
    }
    gpu.sync_now(&world, &device, &queue).unwrap();
    let rb = Readback::new(&device, &queue, &gpu);
    let mut mip_slots = 0;
    for &c in &coords {
        mip_slots += rb.assert_lod(&gpu, &world, c).len() as u32;
    }
    assert!(mip_slots > 0);
    assert_eq!(gpu.live_bricks(), mip_slots);
    assert_eq!(
        gpu.pages_issued(),
        4,
        "no page past the four already issued"
    );
    let bins: HashSet<usize> = (0..4)
        .filter_map(|p| gpu.bin_of_unit(p * PAGE_UNITS))
        .collect();
    assert!(
        bins.contains(&bin_of(2)),
        "a page of the 3-unit bin now serves the 2-unit bin: {bins:?}"
    );

    for (c, chunk) in held {
        world.remove_lod(c);
        world.store().write_lock().insert(c, chunk);
    }
    gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(gpu.live_bricks(), 8 * 512);
    assert_eq!(gpu.pages_issued(), 4, "no page was added");
    assert!(gpu.free_pages().is_empty());
    let rb = Readback::new(&device, &queue, &gpu);
    for &c in &coords {
        rb.assert_chunk(&gpu, &world, c);
    }
    assert_units_accounted(&gpu);
}

/// Slots 0..256 of one chunk hold seeded bricks of 1 to 24 materials from ids 1 to 30, ids
/// 25 to 30 translucent; the rest are air.
struct Seeded256;

impl WorldSource for Seeded256 {
    fn chunk(&self, _: ChunkCoord, out: &mut Chunk) {
        for slot in 0..256 {
            let h = hash::hash3(0x0021_0701, [slot as i32, 0, 0]);
            let n = 1 + h % 24;
            let first = 1 + (h >> 8) % (31 - n);
            let solid = (h >> 16) % 9;
            let mut s = h;
            out.set_brick(
                slot,
                Brick::from_fn(|_| {
                    s = hash::mix64(s);
                    if s % 8 < solid {
                        (first + (s >> 8) % n) as MaterialId
                    } else {
                        AIR
                    }
                }),
            );
        }
    }
}

/// Ids 1 to 30, 25 to 30 translucent.
fn seeded_table() -> MaterialTable {
    let mut t = MaterialTable::new();
    for i in 1..=30u16 {
        let mut m = Material::named(format!("m{i}"));
        if i >= 25 {
            m.flags = MaterialFlags::TRANSLUCENT;
        }
        t.push(m);
    }
    t
}

/// For a chunk of 256 seeded bricks, translucent and above-16-material ones among them, every
/// slot decodes to its voxels, its opaque mask is its opaque voxels, its header's translucent
/// words its translucent voxels' 2^3 blocks, and its class and slot size follow the brick: the
/// class's units, plus a translucent unit for a planar brick holding a translucent voxel.
#[test]
fn every_staged_slot_decodes_to_its_brick() {
    let Some((device, queue)) = device() else {
        return;
    };
    let t = seeded_table();
    let world = World::generate(ChunkExtent::new([0, 0, 0], [1, 1, 1]), &Seeded256, &t);
    let mut gpu = GpuWorld::with_config(&device, small(Window::Fixed(world.extent())));
    gpu.sync_now(&world, &device, &queue).unwrap();
    let rb = Readback::new(&device, &queue, &gpu);
    let coord = ChunkCoord([0, 0, 0]);
    rb.assert_chunk(&gpu, &world, coord);
    let chunk = world.store().get(coord).unwrap();
    let chunk = chunk.read().unwrap();
    let (mut translucent_bricks, mut over_16, mut glass_units) = (0, 0, 0);
    for (slot, brick) in chunk.bricks().iter().enumerate().take(256) {
        let Brick::Full(p) = brick else {
            continue;
        };
        let e = gpu.entry(coord, slot).unwrap();
        let wide = p.class() == PackedClass::Wide;
        let glass = e & TRANSLUCENT_BIT != 0;
        let units = 2 + payload_units(class_of(e)) + u32::from(glass && !wide);
        assert_eq!(slot_units_of(&gpu, e), BINS[bin_of(units)], "slot {slot}");
        translucent_bricks += usize::from(glass);
        over_16 += usize::from(wide);
        glass_units += usize::from(glass && !wide);
    }
    assert!(
        translucent_bricks > 20 && over_16 > 10 && glass_units > 10,
        "{translucent_bricks} {over_16} {glass_units}"
    );
}

/// One brick at slot 0 of a chunk, the rest air.
struct OneBrick(Brick);

impl WorldSource for OneBrick {
    fn chunk(&self, _: ChunkCoord, out: &mut Chunk) {
        out.set_brick(0, self.0.clone());
    }
}

/// The slot words of brick 0 of a one-chunk world mirrored from `brick`, and its entry.
fn one_slot(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    brick: Brick,
    table: &MaterialTable,
) -> (Vec<u32>, u32) {
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [1, 1, 1]),
        &OneBrick(brick),
        table,
    );
    let mut gpu = GpuWorld::with_config(device, small(Window::Fixed(world.extent())));
    gpu.sync_now(&world, device, queue).unwrap();
    let e = gpu.entry(ChunkCoord([0, 0, 0]), 0).unwrap();
    let units = u64::from(slot_units_of(&gpu, e));
    let bytes = slot_bytes(device, queue, &gpu, e & UNIT_MASK, 0, units * UNIT_BYTES);
    (bytemuck::pod_collect_to_vec(&bytes), e)
}

/// A slot is the packed brick's own words, read back from the pool: a two-material opaque
/// brick's unit 0 is the CPU mask, its header's palette `Packed::palette` and its plane unit
/// `Packed::words`'s plane; the same brick with one material swapped for glass has unit 0
/// clear at the glass voxels and the header's 2^3 words set at their blocks, and a
/// translucent unit holding them; a brick of 17 materials reads its `u16` ids back.
#[test]
fn a_slot_holds_the_packed_brick_s_own_words() {
    let Some((device, queue)) = device() else {
        return;
    };
    let t = seeded_table();
    let pattern = |a: MaterialId, b: MaterialId| {
        Brick::from_fn(move |l| match (l[0] + 2 * l[1] + 3 * l[2]) % 5 {
            0 => AIR,
            1 | 2 => a,
            _ => b,
        })
    };
    // Rock-like 3 and 7: one plane.
    let brick = pattern(3, 7);
    let Brick::Full(p) = &brick else {
        panic!("a mixed brick is Full")
    };
    assert_eq!(p.class(), PackedClass::Plane1);
    let (slot, e) = one_slot(&device, &queue, brick.clone(), &t);
    assert_eq!(class_of(e), 2);
    assert_eq!(e & TRANSLUCENT_BIT, 0);
    assert_eq!(slot.len(), 3 * UNIT_WORDS);
    assert_eq!(layers(&slot[..16]), p.mask(), "unit 0 is the CPU mask");
    let palette: Vec<MaterialId> = (0..16)
        .map(|j| (slot[16 + j / 2] >> (16 * (j % 2))) as MaterialId)
        .take_while(|&m| m != AIR)
        .collect();
    assert_eq!(palette, p.palette());
    assert_eq!(palette, [3, 7]);
    let words = p.words().expect("a planar brick has words");
    assert_eq!(layers(&slot[32..48]), words[8..16], "the plane unit");
    assert_eq!(slot[16 + 8..16 + 10], [0, 0], "no translucent block");

    // Glass (id 26) for material 7.
    let glass = pattern(3, 26);
    let (slot, e) = one_slot(&device, &queue, glass.clone(), &t);
    assert_ne!(e & TRANSLUCENT_BIT, 0);
    assert_eq!(class_of(e), 2);
    let mut v = [AIR; BRICK_VOXELS];
    glass.decode(&mut v);
    let (mut opaque, mut panes, mut blocks) = ([0u64; 8], [0u64; 8], [0u32; 2]);
    for (i, &m) in v.iter().enumerate() {
        match m {
            3 => opaque[i / 64] |= 1 << (i % 64),
            26 => {
                panes[i / 64] |= 1 << (i % 64);
                let (x, y, z) = (i % 8, i / 8 % 8, i / 64);
                let b = x / 2 + 4 * (y / 2) + 16 * (z / 2);
                blocks[b / 32] |= 1 << (b % 32);
            }
            _ => {}
        }
    }
    assert_ne!(panes, [0; 8]);
    assert_eq!(
        layers(&slot[..16]),
        opaque,
        "unit 0 is clear at the glass voxels"
    );
    assert_eq!(slot[16 + 8..16 + 10], blocks, "the header's 2^3 words");
    assert_eq!(
        slot.len(),
        4 * UNIT_WORDS,
        "mask, header, plane, translucent unit"
    );
    assert_eq!(layers(&slot[48..64]), panes, "the translucent unit");

    // Seventeen materials, 1 to 17, one per voxel in turn: raw ids.
    let many = Brick::from_fn(|l| 1 + ((l[0] + 8 * l[1] + 64 * l[2]) % 17) as MaterialId);
    let (slot, e) = one_slot(&device, &queue, many.clone(), &t);
    assert_eq!(class_of(e), CLASS_WIDE);
    let mut v = [AIR; BRICK_VOXELS];
    many.decode(&mut v);
    let ids: Vec<MaterialId> = slot[32..32 + 256]
        .iter()
        .flat_map(|&w| [w as MaterialId, (w >> 16) as MaterialId])
        .collect();
    assert_eq!(ids, v);
}

/// A band-air brick's slot, bit-exact (ADR-0041): in a one-chunk world whose one solid brick
/// is uniform, with the field surface on, the uniform air brick beside the solid brick's +x
/// face is a band brick. Its entry is `Full` of class [`CLASS_UNIFORM`] and not translucent;
/// its slot is two units, an opaque mask with no bit set and a header holding air as palette
/// entry 0, its band slot in [`HEADER_BAND_WORD`] and its band prefix (8 band voxels a layer)
/// in [`HEADER_BAND_PREFIX_WORD`]; the band slot is two units of an exact
/// bin, its band mask the face layer `x = 0` (64 bits), and its first count unit holds, after
/// the field pass, the counts of the face layer in band-index order (voxel `(0, y, z)` at
/// index `y + 8 z`): 4 at a corner of the face, 6 along an edge, 9 inside it.
#[test]
fn a_band_air_brick_s_slot_is_pinned() {
    let Some((device, queue)) = device() else {
        return;
    };
    let mut t = MaterialTable::new();
    t.push(Material::named("rock"));
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [1, 1, 1]),
        &OneBrick(Brick::Uniform(1)),
        &t,
    );
    let mut gpu = GpuWorld::with_config(&device, small(Window::Fixed(world.extent())));
    gpu.set_field_surface(true);
    gpu.sync_now(&world, &device, &queue).unwrap();
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    assert!(gpu.update_field(&mut enc) > 0);
    queue.submit([enc.finish()]);
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();

    let c = ChunkCoord([0, 0, 0]);
    let e = gpu.entry(c, brick_slot([1, 0, 0])).unwrap();
    assert_eq!(e & !UNIT_MASK, FULL_BIT | CLASS_UNIFORM << CLASS_SHIFT);
    let pool: Vec<u32> = bytemuck::pod_collect_to_vec(&vxe_render::debug::read_buffer(
        &device,
        &queue,
        gpu.pool_shard(0),
        0,
        gpu.pool_shard(0).size(),
    ));
    let o = (e & UNIT_MASK) as usize * UNIT_WORDS;
    assert_eq!(gpu.bin_of_unit(e & UNIT_MASK), Some(bin_of(2)));
    assert_eq!(pool[o..o + UNIT_WORDS], [0; UNIT_WORDS]);
    let header = &pool[o + UNIT_WORDS..o + 2 * UNIT_WORDS];
    let band = header[HEADER_BAND_WORD] & UNIT_MASK;
    let mut want = [0u32; UNIT_WORDS];
    want[HEADER_BAND_WORD] = HEADER_BAND_BIT | band;
    // The band prefix: eight band voxels in each layer below, layers 1 to 4 at bits 0, 7, 15
    // and 23 of the first word, 5 to 7 at bits 0, 9 and 18 of the second.
    want[HEADER_BAND_PREFIX_WORD] = 8 | 16 << 7 | 24 << 15 | 32 << 23;
    want[HEADER_BAND_PREFIX_WORD + 1] = 40 | 48 << 9 | 56 << 18;
    assert_eq!(header, want);

    assert_eq!(gpu.bin_of_unit(band), Some(band_bin_of(2)));
    let b = band as usize * UNIT_WORDS;
    assert_eq!(pool[b..b + UNIT_WORDS], [0x0101_0101; UNIT_WORDS]);
    let counts: Vec<u8> = pool[b + UNIT_WORDS..b + 2 * UNIT_WORDS]
        .iter()
        .flat_map(|w| w.to_le_bytes())
        .collect();
    let want: Vec<u8> = (0..64)
        .map(|k| {
            let edge = |v: usize| if v == 0 || v == 7 { 2 } else { 3 };
            (edge(k % 8) * edge(k / 8)) as u8
        })
        .collect();
    assert_eq!(counts[..8], [4, 6, 6, 6, 6, 6, 6, 4]);
    assert_eq!(counts[8..16], [6, 9, 9, 9, 9, 9, 9, 6]);
    assert_eq!(counts, want);
    // The solid brick is a band brick too, its band its outer layer.
    let s = gpu.entry(c, 0).unwrap();
    assert_eq!(s & !UNIT_MASK, FULL_BIT | CLASS_UNIFORM << CLASS_SHIFT);
    // The solid brick and the seven air bricks of the chunk around its corner.
    assert_eq!(gpu.band_bricks(), 8);
}

/// Runs the field pass over the band slots written since the last call and waits for it.
fn update_field(device: &wgpu::Device, queue: &wgpu::Queue, gpu: &mut GpuWorld) -> usize {
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let n = gpu.update_field(&mut enc);
    queue.submit([enc.finish()]);
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    n
}
