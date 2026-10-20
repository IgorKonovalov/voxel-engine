//! The GPU mirror against a closure-source world: what `sync` uploads, what it frees, and
//! that the buffers hold what the CPU accounting says.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use vxe_render::gpu_world::{
    ABSENT, ATTR_PAGE_BRICK_SLOTS, BITS_WORDS, EMPTY_BIT, ENTRY_WORD, FULL_BIT, LOD_BIT,
    LOD_LEVEL_ENTRY, MASK_BYTES, MASK_GROUP_SLOTS, MASK_GROUP_WORDS, NO_PLANE, PAGE_BYTES,
    PALETTE_WORD, PLACEHOLDER_BYTES, POOL_PAGE_SLOTS, RECORD_BYTES, RECORD_MASK, RECORD_WORDS,
    SLOT_BYTES, SLOT_MASK, WIDE_BIT, attr_slot, budget_for, opaque_mask_word,
    translucent_mask_word,
};
use vxe_render::{
    ATTRIB_WGSL, GpuWorld, MARCH_WGSL, MirrorConfig, ShardLayout, Window, readback, wgpu,
};
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

/// The attribute pass's shader parses and validates headless.
#[test]
fn attrib_wgsl_validates() {
    let module = wgpu::naga::front::wgsl::parse_str(ATTRIB_WGSL).expect("attrib.wgsl parses");
    wgpu::naga::valid::Validator::new(
        wgpu::naga::valid::ValidationFlags::all(),
        wgpu::naga::valid::Capabilities::default(),
    )
    .validate(&module)
    .expect("attrib.wgsl validates");
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

/// The budget the tests' mirrors reserve unless a test says otherwise: 64 MiB.
const SMALL_BUDGET: u64 = 64 << 20;

/// `window` with no sync budget and a 64 MiB reservation.
fn small(window: Window) -> MirrorConfig {
    MirrorConfig {
        budget_bytes: SMALL_BUDGET,
        ..MirrorConfig::new(window)
    }
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

    let mut gpu = GpuWorld::with_config(&device, small(Window::Fixed(world.extent())));
    let first = gpu.sync(&world, &queue).unwrap();
    assert_eq!(first.bricks, 256);
    assert_eq!(first.chunks, 4);
    assert_eq!(gpu.live_slots(), 256);
    // One page handed out, of the reservation's pages.
    assert_eq!(gpu.pool_slots(), POOL_PAGE_SLOTS);
    assert!(gpu.reserved_slots() > POOL_PAGE_SLOTS);
    // Allocation is the reserved buffers' sizes, and covers the live accounting.
    let allocated: u64 = (0..4).map(|k| gpu.pool_shard(k).size()).sum::<u64>()
        + gpu.masks().size()
        + gpu.records().size();
    assert_eq!(gpu.allocated_bytes(), allocated);
    assert_eq!(gpu.allocated_bytes(), gpu.shard_layout().reserved_bytes());
    assert!(gpu.allocated_bytes() >= gpu.mirror_bytes());
    for c in world.store().coords() {
        assert!(!world.store().get(c).unwrap().read().unwrap().any_dirty());
    }

    // The dirty-chunk set is empty: an idle sync visits no chunk and writes nothing.
    assert_eq!(world.store().dirty_len(), 0);
    let before = gpu.bytes_written();
    let second = gpu.sync(&world, &queue).unwrap();
    assert_eq!(second.bytes, 0);
    assert_eq!(second.chunks_visited, 0);
    assert_eq!(second.writes, 0);
    assert_eq!(gpu.bytes_written(), before);

    // The GPU records equal the CPU entries, and a full brick's pool bytes are its voxels as
    // indices into the record's palette.
    let rec = gpu.record_index(ChunkCoord([1, 0, 0])).unwrap() as u64;
    let bytes = readback::read_buffer(
        &device,
        &queue,
        gpu.records(),
        rec * RECORD_BYTES,
        RECORD_BYTES,
    );
    let words: &[u32] = bytemuck::cast_slice(&bytes);
    for (slot, &e) in words[ENTRY_WORD..PALETTE_WORD].iter().enumerate() {
        assert_eq!(Some(e), gpu.entry(ChunkCoord([1, 0, 0]), slot));
    }
    assert_eq!(gpu.palette(ChunkCoord([1, 0, 0])), Some(&[0, 1][..]));
    assert_eq!(
        words[PALETTE_WORD],
        1 << 16,
        "palette entries 0 (air) and 1"
    );
    // Only the z = 0 layer of bricks is not uniform air: bricks 0..64, words 0 and 1.
    assert_eq!(
        words[..BITS_WORDS],
        [!0, !0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
    );
    let e = words[ENTRY_WORD];
    assert_eq!(e & (FULL_BIT | WIDE_BIT), FULL_BIT);
    let pool = readback::read_buffer(
        &device,
        &queue,
        gpu.pool_shard(0),
        u64::from(e & SLOT_MASK) * SLOT_BYTES,
        SLOT_BYTES,
    );
    for (i, &index) in pool.iter().enumerate() {
        let z = (i / 64) as i32;
        assert_eq!(index, u8::from(z < 3), "voxel {i}");
    }

    // The same slot's opaque mask is the brick's solid bits, its translucent words clear.
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
    let s = e & SLOT_MASK;
    let opaque = readback::read_buffer(
        &device,
        &queue,
        gpu.masks(),
        u64::from(opaque_mask_word(s)) * 4,
        64,
    );
    let opaque: Vec<u32> = bytemuck::pod_collect_to_vec(&opaque);
    assert_eq!(layers(&opaque), bits);
    let translucent = readback::read_buffer(
        &device,
        &queue,
        gpu.masks(),
        u64::from(translucent_mask_word(s)) * 4,
        8,
    );
    assert_eq!(
        bytemuck::pod_collect_to_vec::<u8, u32>(&translucent),
        [0, 0]
    );
}

/// Sixteen opaque mask words as eight `u64` layers, low word first.
fn layers(w: &[u32]) -> [u64; 8] {
    std::array::from_fn(|z| u64::from(w[2 * z]) | u64::from(w[2 * z + 1]) << 32)
}

/// Slot s's opaque mask starts on a 64-byte line, and its two translucent words lie in its
/// group's ninth line, at 144 (s / 8) + 128 + 2 (s % 8).
#[test]
fn every_opaque_mask_starts_a_line_and_the_translucent_words_share_the_ninth() {
    assert_eq!(MASK_GROUP_SLOTS, 8);
    assert_eq!(MASK_GROUP_WORDS, 144);
    assert_eq!(MASK_BYTES, 72);
    let mut lines = HashSet::new();
    for s in 0..64u32 {
        let o = opaque_mask_word(s) * 4;
        assert_eq!(o % 64, 0, "slot {s}");
        assert_eq!(o, 576 * (s / 8) + 64 * (s % 8), "slot {s}");
        assert!(lines.insert(o / 64), "slot {s} shares a line");
        let t = translucent_mask_word(s);
        assert_eq!(t, 144 * (s / 8) + 128 + 2 * (s % 8), "slot {s}");
        // Both words in the group's ninth 64-byte line.
        assert_eq!(t * 4 / 64, 9 * (s / 8) + 8, "slot {s}");
        assert_eq!((t + 1) * 4 / 64, 9 * (s / 8) + 8, "slot {s}");
    }
}

#[test]
fn a_collapsed_brick_frees_its_slot_for_the_next_expansion() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = world();
    let mut gpu = GpuWorld::with_config(&device, small(Window::Fixed(world.extent())));
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
    let at = rec * RECORD_BYTES + ENTRY_WORD as u64 * 4;
    let bytes = readback::read_buffer(&device, &queue, gpu.records(), at, 4);
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
    let bytes = readback::read_buffer(device, queue, gpu.table(), at, 4);
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
    gpu.sync(&world, &queue).unwrap();
    let (floor, air) = (ChunkCoord([0, 0, 0]), ChunkCoord([0, 0, 1]));
    let rec = gpu.record_index(air).unwrap();
    assert_eq!(table_entry(&device, &queue, &gpu, air), rec | EMPTY_BIT);
    assert_eq!(
        table_entry(&device, &queue, &gpu, floor),
        gpu.record_index(floor).unwrap()
    );

    fill_voxel(&world, [5, 6, 70], 1);
    gpu.sync(&world, &queue).unwrap();
    assert_eq!(table_entry(&device, &queue, &gpu, air), rec);

    let r = world
        .apply(&EditOp::box_(
            centre_of_voxel([5, 6, 70]),
            centre_of_voxel([6, 7, 71]),
            Mode::Carve,
        ))
        .unwrap();
    assert_eq!(r.changed_from, [(1, 1)]);
    gpu.sync(&world, &queue).unwrap();
    assert_eq!(table_entry(&device, &queue, &gpu, air), rec | EMPTY_BIT);
    assert_eq!(gpu.record_index(air), Some(rec));
}

/// Each brick is air, material 1 or mixed by a hash of its brick coordinate; a mixed brick
/// draws each voxel from air, 1 and 2. Chunks hold `Uniform` and `Full` bricks.
struct Mixed;

impl WorldSource for Mixed {
    fn chunk(&self, coord: ChunkCoord, out: &mut Chunk) {
        let base = coord.min_voxel().0;
        for slot in 0..CHUNK_BRICKS {
            let b = brick_of_slot(slot);
            let bmin = [0, 1, 2].map(|k| base[k] + b[k] * BRICK_EDGE);
            let class = hash::hash3(0xB0B, bmin.map(|v| v >> 3)) % 16;
            out.set_brick(
                slot,
                Brick::from_fn(|l| match class {
                    0..6 => 0,
                    6..12 => 1,
                    _ => (hash::hash3(0xF00D, [0, 1, 2].map(|k| bmin[k] + l[k])) % 3) as u16,
                }),
            );
        }
    }
}

fn mixed_table() -> MaterialTable {
    let mut t = MaterialTable::new();
    t.push(Material::named("one"));
    t.push(Material::named("two"));
    t
}

/// Whole-buffer copies of the mirror, decoded the way the shader reads them.
struct Readback {
    table: Vec<u32>,
    records: Vec<u32>,
    pool: Vec<u8>,
    masks: Vec<u32>,
}

impl Readback {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, gpu: &GpuWorld) -> Self {
        let all = |b: &wgpu::Buffer| readback::read_buffer(device, queue, b, 0, b.size());
        // The reserved buffers are read as far as the pages handed out reach.
        let slots = u64::from(gpu.pool_slots());
        let sps = u64::from(gpu.shard_layout().slots_per_shard);
        let shard = |k: usize| {
            let n = slots.saturating_sub(k as u64 * sps).min(sps);
            readback::read_buffer(device, queue, gpu.pool_shard(k), 0, n * SLOT_BYTES)
        };
        Self {
            table: bytemuck::cast_slice(&all(gpu.table())).to_vec(),
            records: bytemuck::cast_slice(&all(gpu.records())).to_vec(),
            pool: (0..gpu.shards_in_use()).flat_map(shard).collect(),
            masks: bytemuck::cast_slice(&readback::read_buffer(
                device,
                queue,
                gpu.masks(),
                0,
                slots * MASK_BYTES,
            ))
            .to_vec(),
        }
    }
    /// The voxels of the `Full` brick in record entry `e` of record `rec`, decoded as the
    /// shader decodes them: through the palette when narrow, as `u16` pairs when wide.
    fn voxels(&self, rec: u32, e: u32) -> Vec<u16> {
        let s = (e & SLOT_MASK) as usize * SLOT_BYTES as usize;
        if e & WIDE_BIT != 0 {
            bytemuck::cast_slice(&self.pool[s..s + 1024]).to_vec()
        } else {
            let palette = &self.records[rec as usize * RECORD_WORDS + PALETTE_WORD..][..128];
            self.pool[s..s + 512]
                .iter()
                .map(|&i| (palette[usize::from(i) / 2] >> (16 * (i & 1))) as u16)
                .collect()
        }
    }

    /// Asserts that the table, records and pool hold `coord`'s CPU bricks; returns the slots
    /// its `Full` bricks occupy.
    fn assert_chunk(&self, gpu: &GpuWorld, world: &World, coord: ChunkCoord) -> Vec<u32> {
        let t = self.table[gpu.table_index(coord) as usize];
        assert_ne!(t, ABSENT, "{coord:?} has no table entry");
        let rec = t & RECORD_MASK;
        let chunk = world.store().get(coord).unwrap();
        let chunk = chunk.read().unwrap();
        let bits = &self.records[rec as usize * RECORD_WORDS..][..BITS_WORDS];
        assert_eq!(
            t & EMPTY_BIT != 0,
            bits.iter().all(|&w| w == 0),
            "{coord:?}"
        );
        let mut slots = Vec::new();
        for (slot, brick) in chunk.bricks().iter().enumerate() {
            let set = bits[slot / 32] >> (slot % 32) & 1 != 0;
            assert_eq!(
                set,
                *brick != Brick::Uniform(AIR),
                "{coord:?} brick {slot} bit"
            );
            let e = self.records[rec as usize * RECORD_WORDS + ENTRY_WORD + slot];
            match brick {
                Brick::Uniform(m) => assert_eq!(e, u32::from(*m), "{coord:?} brick {slot}"),
                Brick::Full(p) => {
                    let mut v = [AIR; BRICK_VOXELS];
                    p.decode(&mut v);
                    assert_ne!(e & FULL_BIT, 0, "{coord:?} brick {slot}");
                    assert_eq!(self.voxels(rec, e), &v[..], "{coord:?} brick {slot}");
                    let s = e & SLOT_MASK;
                    let o = opaque_mask_word(s) as usize;
                    assert_eq!(
                        layers(&self.masks[o..o + 16]),
                        brick.solid_bits(),
                        "{coord:?} brick {slot} mask"
                    );
                    let t = translucent_mask_word(s) as usize;
                    assert_eq!(self.masks[t..t + 2], [0, 0], "{coord:?} brick {slot}");
                    slots.push(s);
                    if e & WIDE_BIT != 0 {
                        assert_eq!(s % 2, 0, "{coord:?} brick {slot}: wide pair at odd slot");
                        slots.push(s + 1);
                    }
                }
            }
        }
        slots
    }
}

fn full_bricks(world: &World, coords: &[ChunkCoord]) -> u32 {
    coords
        .iter()
        .map(|&c| {
            let chunk = world.store().get(c).unwrap();
            let chunk = chunk.read().unwrap();
            chunk.bricks().iter().filter(|b| b.is_full()).count() as u32
        })
        .sum()
}

/// The two terms of the pressure: slot occupancy (a brick in an attribute page counting its
/// two attribute slots) and record occupancy.
fn pressure_terms(gpu: &GpuWorld) -> (f64, f64) {
    let slots = f64::from(gpu.live_slots()) + 2.0 * f64::from(gpu.live_attr_slots());
    (
        slots / f64::from(gpu.reserved_slots()),
        gpu.mirrored_chunks() as f64 / f64::from(gpu.reserved_records()),
    )
}

/// `pressure()` is the larger of its two terms within 1e-6, and between 0 and 1 here.
fn assert_pressure(gpu: &GpuWorld) {
    let (slots, records) = pressure_terms(gpu);
    let want = slots.max(records);
    let got = f64::from(gpu.pressure());
    assert!((got - want).abs() < 1e-6, "pressure {got} against {want}");
    assert!(want > 0.0 && want < 1.0, "{want}");
}

/// The buffers a mirror reserved: its four shards, masks and records.
fn reserved_buffers(gpu: &GpuWorld) -> Vec<wgpu::Buffer> {
    (0..4)
        .map(|k| gpu.pool_shard(k).clone())
        .chain([gpu.masks().clone(), gpu.records().clone()])
        .collect()
}

/// The streamer's radius-2 scenario through a following window of side 7: after the move the
/// mirror holds the 42 resident chunks, each decoding to its CPU voxels, and the evicted
/// chunks' slots went to the free list. The 64 MiB reservation is made once: its bytes are
/// the budget rounded down to whole pages and the window's 343 records, and no sync, eviction
/// or window move creates, copies or resizes a buffer.
#[test]
fn a_following_window_mirrors_the_streamed_residency() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = World::new(&mixed_table());
    let mut s = Streamer::new(
        Arc::new(Mixed),
        StreamConfig {
            residency: Residency {
                r_full: 2,
                r_lod: 2,
                hysteresis: 1,
            },
            workers: 4,
            persist: PersistConfig::default(),
        },
    );
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            window: Window::Follow { side: 7 },
            sync_budget_bytes: u64::MAX,
            max_shard_slots: None,
            budget_bytes: SMALL_BUDGET,
            record_chunks: None,
        },
    );
    // 343 records, then whole pages of what is left past the placeholders: 27 pages on one
    // shard (lavapipe's 262 144 slots), the other three shards one-slot placeholders.
    assert_eq!(gpu.reserved_records(), 343);
    let pages = (SMALL_BUDGET - 343 * RECORD_BYTES - PLACEHOLDER_BYTES) / PAGE_BYTES;
    assert_eq!(pages, 27);
    assert_eq!(u64::from(gpu.reserved_slots()), pages * 4096);
    let reserved = 343 * RECORD_BYTES + pages * PAGE_BYTES + 3 * SLOT_BYTES;
    assert_eq!(gpu.allocated_bytes(), reserved);
    assert!(SMALL_BUDGET - reserved < PAGE_BYTES + PLACEHOLDER_BYTES);
    let buffers = reserved_buffers(&gpu);
    let mut stats = Vec::new();

    let focus = VoxelCoord([0, 0, 0]);
    s.update(&world, &[focus]);
    let ev = s.wait_idle(&world);
    gpu.apply_events(&ev);
    gpu.set_focus(focus);
    let st = gpu.sync(&world, &queue).unwrap();
    stats.push(st);
    assert_eq!(gpu.allocated_bytes(), reserved);
    assert_pressure(&gpu);
    assert_eq!(st.chunks, 33);
    assert_eq!(gpu.mirrored_chunks(), 33);
    let first = world.store().coords();
    let rb = Readback::new(&device, &queue, &gpu);
    let mut old_slots = HashSet::new();
    for &c in &first {
        old_slots.extend(rb.assert_chunk(&gpu, &world, c));
    }

    let focus = VoxelCoord([192, 0, 0]);
    s.update(&world, &[focus]);
    let ev = s.wait_idle(&world);
    assert_eq!(ev.evicted.len(), 22);
    let evicted_slots: HashSet<u32> = ev
        .evicted
        .iter()
        .flat_map(|&c| (0..512).map(move |slot| (c, slot)))
        .filter_map(|(c, slot)| gpu.entry(c, slot))
        .filter(|e| e & FULL_BIT != 0)
        .map(|e| e & !FULL_BIT)
        .collect();
    assert!(!evicted_slots.is_empty());
    let (live_before, allocated_before) = (gpu.mirror_bytes(), gpu.allocated_bytes());
    gpu.apply_events(&ev);
    // Eviction lowers the live accounting; the reservation stays.
    assert!(gpu.mirror_bytes() < live_before);
    assert_eq!(gpu.allocated_bytes(), allocated_before);
    assert_eq!(gpu.allocated_bytes(), reserved);
    assert_pressure(&gpu);
    // Eviction returns exactly the evicted chunks' slots: to the free list, or with a page
    // that emptied, to the free pages.
    let free: HashSet<u32> = gpu.free_slots().iter().copied().collect();
    let free_pages: HashSet<u32> = gpu.free_pages().into_iter().collect();
    let freed: HashSet<u32> = (0..gpu.pool_slots())
        .filter(|s| free.contains(s) || free_pages.contains(&(s / POOL_PAGE_SLOTS)))
        .collect();
    assert!(
        free.iter()
            .all(|s| !free_pages.contains(&(s / POOL_PAGE_SLOTS)))
    );
    assert!(evicted_slots.is_subset(&freed));
    assert_eq!(
        u64::from(gpu.live_slots()) + gpu.spare_slots(),
        u64::from(gpu.pool_slots())
    );

    gpu.set_focus(focus);
    let st = gpu.sync(&world, &queue).unwrap();
    stats.push(st);
    assert_eq!(st.pending_chunks, 0);
    // The window moved by three chunks; nothing was created, copied or resized.
    assert_eq!(gpu.allocated_bytes(), reserved);
    assert_eq!(reserved_buffers(&gpu), buffers);
    assert!(
        stats
            .iter()
            .all(|s| s.buffers_created == 0 && s.copies == 0)
    );
    assert_pressure(&gpu);
    let resident = world.store().coords();
    assert_eq!(resident.len(), 42);
    assert_eq!(gpu.mirrored_chunks(), 42);
    for &c in &ev.evicted {
        assert!(!gpu.is_mirrored(c));
    }
    let rb = Readback::new(&device, &queue, &gpu);
    let mut live = HashSet::new();
    for &c in &resident {
        for s in rb.assert_chunk(&gpu, &world, c) {
            assert!(live.insert(s), "slot {s} held twice");
        }
    }
    assert_eq!(gpu.live_slots(), full_bricks(&world, &resident));
    assert_eq!(live.len() as u32, gpu.live_slots());
    // No free slot is live, and every slot handed out is live or spare: none leaked.
    let free: HashSet<u32> = gpu.free_slots().iter().copied().collect();
    let free_pages: HashSet<u32> = gpu.free_pages().into_iter().collect();
    for s in &live {
        assert!(!free.contains(s), "slot {s}");
        assert!(!free_pages.contains(&(s / POOL_PAGE_SLOTS)), "slot {s}");
    }
    assert_eq!(
        u64::from(gpu.live_slots()) + gpu.spare_slots(),
        u64::from(gpu.pool_slots())
    );
    // The evicted chunks' table entries are gone: their wrapped indices now hold new chunks
    // or nothing.
    for &c in &ev.evicted {
        let t = gpu.table_index(c) as usize;
        let holder = resident.iter().find(|&&r| gpu.table_index(r) as usize == t);
        match holder {
            Some(&r) => assert_eq!(Some(rb.table[t]), gpu.record_index(r)),
            None => assert_eq!(rb.table[t], ABSENT),
        }
    }
    assert!(old_slots.len() > 100);
}

/// A table with rows 1..=n named by number.
fn numbered_table(n: u16) -> MaterialTable {
    let mut t = MaterialTable::new();
    for i in 1..=n {
        assert_eq!(t.push(Material::named(format!("m{i}"))), i);
    }
    t
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

#[test]
fn a_chunk_of_300_materials_is_wide_and_its_neighbour_narrow() {
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
    gpu.sync(&world, &queue).unwrap();

    assert!(gpu.is_wide(many));
    assert!(!gpu.is_wide(two));
    assert_eq!(gpu.wide_chunks(), 1);
    assert_eq!(gpu.palette(two), Some(&[0, 7, 9][..]));
    let rb = Readback::new(&device, &queue, &gpu);
    // `assert_chunk` decodes every brick and checks each wide pair starts on an even slot.
    let wide_slots = rb.assert_chunk(&gpu, &world, many);
    assert_eq!(wide_slots.len(), 2 * full_bricks(&world, &[many]) as usize);
    assert_eq!(full_bricks(&world, &[many]), 512);
    for slot in 0..512 {
        let e = gpu.entry(many, slot).unwrap();
        assert_eq!(
            e & (FULL_BIT | WIDE_BIT),
            FULL_BIT | WIDE_BIT,
            "brick {slot}"
        );
        assert_eq!((e & SLOT_MASK) % 2, 0, "brick {slot}");
    }
    let narrow_slots = rb.assert_chunk(&gpu, &world, two);
    assert_eq!(narrow_slots.len(), full_bricks(&world, &[two]) as usize);
    assert_eq!(
        gpu.live_slots(),
        2 * 512 + full_bricks(&world, &[two]),
        "a wide brick holds two slots"
    );
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

#[test]
fn a_new_material_appends_to_the_palette_and_an_overflow_turns_the_chunk_wide() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [1, 1, 1]),
        &Floor,
        &numbered_table(256),
    );
    let c = ChunkCoord([0, 0, 0]);
    let mut gpu = GpuWorld::with_config(&device, small(Window::Fixed(world.extent())));
    gpu.sync(&world, &queue).unwrap();
    assert_eq!(gpu.palette(c), Some(&[0, 1][..]));
    let floor_bricks = full_bricks(&world, &[c]);
    assert_eq!(floor_bricks, 64);

    // Material 2 into the air above the floor in two of the floor's Full bricks.
    let r = world
        .apply(&EditOp::box_(
            centre_of_voxel([0, 0, 4]),
            centre_of_voxel([16, 8, 6]),
            Mode::Fill(2),
        ))
        .unwrap();
    assert_eq!(r.changed_from, [(0, 16 * 8 * 2)]);
    let st = gpu.sync(&world, &queue).unwrap();
    assert_eq!(st.chunks, 1);
    assert_eq!(st.bricks, 2, "only the two touched bricks are rewritten");
    assert_eq!(gpu.palette(c), Some(&[0, 1, 2][..]));
    assert!(!gpu.is_wide(c));
    Readback::new(&device, &queue, &gpu).assert_chunk(&gpu, &world, c);

    // Materials 3..=255 one voxel each, in the air brick layer z in [8, 16): the palette
    // fills to 255 non-air entries and the chunk stays narrow.
    for m in 3..=255u16 {
        let k = i32::from(m);
        fill_voxel(&world, [k % 64, k / 64 * 8, 9], m);
    }
    gpu.sync(&world, &queue).unwrap();
    assert_eq!(gpu.palette(c).unwrap().len(), 256);
    assert!(!gpu.is_wide(c));
    let full_before = full_bricks(&world, &[c]);

    // One more material: the chunk turns wide and every Full brick is uploaded again.
    fill_voxel(&world, [5, 40, 20], 256);
    let st = gpu.sync(&world, &queue).unwrap();
    assert!(gpu.is_wide(c));
    let full = full_bricks(&world, &[c]);
    assert_eq!(full, full_before + 1);
    assert_eq!(st.bricks, u64::from(full));
    let slots = Readback::new(&device, &queue, &gpu).assert_chunk(&gpu, &world, c);
    assert_eq!(slots.len() as u32, 2 * full);
    assert_eq!(gpu.live_slots(), 2 * full);
}

fn limits(binding: u64, buffer: u64, storage_buffers: u32) -> wgpu::Limits {
    wgpu::Limits {
        max_storage_buffer_binding_size: binding,
        max_buffer_size: buffer,
        max_storage_buffers_per_shader_stage: storage_buffers,
        ..wgpu::Limits::default()
    }
}

#[test]
fn the_shard_layout_follows_the_limits_alone() {
    // lavapipe: 128 MiB per binding.
    let l = ShardLayout::from_limits(&limits(128 << 20, 2_147_483_647, 8), None);
    assert_eq!(l.slots_per_shard, 262_144);
    assert_eq!(l.shards, 4);
    // RTX 3080 Laptop and RADV: 2 147 483 644 B per binding, the largest page multiple below.
    let l = ShardLayout::from_limits(&limits(2_147_483_644, 4_294_967_295, 8), None);
    assert_eq!(l.slots_per_shard, 1023 * 4096);
    assert!(u64::from(l.slots_per_shard) * 512 <= 2_147_483_644);
    // The smaller of the two size limits binds.
    let l = ShardLayout::from_limits(&limits(1 << 30, 100 << 20, 8), None);
    assert_eq!(l.slots_per_shard, 204_800);
    // Shards: storage buffers per stage less the four other bindings, at most four.
    assert_eq!(
        ShardLayout::from_limits(&limits(1 << 30, 1 << 30, 6), None).shards,
        2
    );
    assert_eq!(
        ShardLayout::from_limits(&limits(1 << 30, 1 << 30, 48), None).shards,
        4
    );
    // The test knob caps slots per shard, rounded down to a page.
    let l = ShardLayout::from_limits(&limits(128 << 20, 1 << 31, 8), Some(10_000));
    assert_eq!(l.slots_per_shard, 8192);
    assert_eq!(l.capacity(), 4 * 8192);
}

#[test]
fn the_record_capacity_follows_the_limits() {
    // lavapipe: 128 MiB per binding holds 51 150 records of 2624 B.
    assert_eq!(RECORD_BYTES, 2624);
    let l = ShardLayout::from_limits(&limits(128 << 20, 2_147_483_647, 8), None);
    assert_eq!(l.record_capacity, 51_150);
    let l = ShardLayout::from_limits(&limits(4 << 20, 2_147_483_647, 8), None);
    assert_eq!(l.record_capacity, 1598);
    // The smaller of the two size limits binds.
    let l = ShardLayout::from_limits(&limits(1 << 30, 4 << 20, 8), None);
    assert_eq!(l.record_capacity, 1598);
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
    let mut required = vxe_render::limits_for(&adapter, 16 << 20);
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

    // 12 x 12 x 12 = 1728 uniform chunks: no `Full` brick, one record each.
    let mut t = MaterialTable::new();
    let rock = t.push(Material::named("rock"));
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [12, 12, 12]),
        &UniformSource(rock),
        &t,
    );
    assert_eq!(world.store().coords().len(), 1728);
    let mut gpu = GpuWorld::new(&device, world.extent());
    assert_eq!(gpu.shard_layout().record_capacity, 1598);
    partial_then_empty(
        &mut gpu,
        &world,
        &queue,
        1598,
        130,
        vxe_render::MirrorError::RecordsTooLarge {
            needed: 1599 * RECORD_BYTES,
            limit: 1598 * RECORD_BYTES,
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
    assert_eq!(gpu.reserved_slots(), 0);
    assert!(gpu.allocated_bytes() <= budget);
    partial_then_empty(
        &mut gpu,
        &world,
        &queue,
        1000,
        728,
        vxe_render::MirrorError::RecordsTooLarge {
            needed: 1001 * RECORD_BYTES,
            limit: 1000 * RECORD_BYTES,
        },
    );
}

/// A first sync past the reservation uploads the `uploaded` nearest chunks that fit and
/// defers the other `deferred`, which stay pending with their dirty bits; the next sync,
/// with nothing freed, is empty: it fails with `err` and writes nothing.
fn partial_then_empty(
    gpu: &mut GpuWorld,
    world: &World,
    queue: &wgpu::Queue,
    uploaded: usize,
    deferred: u64,
    err: vxe_render::MirrorError,
) {
    let st = gpu.sync(world, queue).unwrap();
    assert_eq!(gpu.mirrored_chunks(), uploaded);
    assert_eq!(st.chunks, uploaded as u64);
    assert_eq!(st.deferred_chunks, deferred);
    assert_eq!(st.pending_chunks, deferred);
    let waiting: Vec<ChunkCoord> = world
        .store()
        .coords()
        .into_iter()
        .filter(|&c| !gpu.is_mirrored(c))
        .collect();
    assert_eq!(waiting.len() as u64, deferred);
    for c in &waiting {
        let chunk = world.store().get(*c).unwrap();
        if chunk.read().unwrap().bricks().iter().any(Brick::is_full) {
            assert!(
                chunk.read().unwrap().any_dirty(),
                "{c:?} keeps its dirty bits"
            );
        }
    }
    let (bytes, live) = (gpu.bytes_written(), gpu.live_slots());
    assert_eq!(gpu.sync(world, queue).unwrap_err(), err);
    assert_eq!(gpu.bytes_written(), bytes);
    assert_eq!(gpu.mirrored_chunks(), uploaded);
    assert_eq!(gpu.live_slots(), live);
    assert_eq!(gpu.pending_chunks() as u64, deferred);
}

/// The mirror's sizing code names no backend, vendor or adapter: only limits decide.
#[test]
fn the_mirror_never_reads_the_backend_or_vendor() {
    for (name, text) in [
        ("gpu_world.rs", include_str!("../src/gpu_world.rs")),
        ("marcher.rs", include_str!("../src/marcher.rs")),
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

/// Every brick of every chunk is `Full`: two materials in a checkerboard.
struct Checker;

impl WorldSource for Checker {
    fn chunk(&self, _coord: ChunkCoord, out: &mut Chunk) {
        for slot in 0..CHUNK_BRICKS {
            out.set_brick(
                slot,
                Brick::from_fn(|l| 1 + ((l[0] + l[1] + l[2]) % 2) as u16),
            );
        }
    }
}

#[test]
fn a_world_past_four_shards_fails_and_leaves_the_mirror_untouched() {
    let Some((device, queue)) = device() else {
        return;
    };
    // 36 chunks of 512 Full bricks: 18 432 slots, past four shards of 4096.
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [4, 3, 3]),
        &Checker,
        &numbered_table(2),
    );
    assert_eq!(world.stats().full_bricks, 18_432);
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            max_shard_slots: Some(4096),
            ..MirrorConfig::new(Window::Fixed(world.extent()))
        },
    );
    assert_eq!(gpu.shard_layout().capacity(), 16_384);
    // 32 chunks fill the four shards; the 4 farthest wait, and the next sync is empty.
    partial_then_empty(
        &mut gpu,
        &world,
        &queue,
        32,
        4,
        vxe_render::MirrorError::PoolTooLarge {
            needed: (16_384 + 512) * SLOT_BYTES,
            limit: 16_384 * SLOT_BYTES,
        },
    );
    assert_eq!(gpu.live_slots(), 16_384);
    assert_eq!(gpu.shards_in_use(), 4);
}

/// The same 36 chunks under a budget of four pages and their records, on uncapped shards:
/// the budget is the limit; the first sync uploads 32 chunks, and the next, with nothing
/// freed, writes nothing.
#[test]
fn a_world_past_the_budget_fails_and_leaves_the_mirror_untouched() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [4, 3, 3]),
        &Checker,
        &numbered_table(2),
    );
    let budget = budget_for(16_384, 36);
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            budget_bytes: budget,
            ..MirrorConfig::new(Window::Fixed(world.extent()))
        },
    );
    assert!(gpu.shard_layout().capacity() > 16_384);
    assert_eq!(gpu.reserved_slots(), 16_384);
    assert_eq!(gpu.reserved_records(), 36);
    assert!(gpu.allocated_bytes() <= budget);
    assert_eq!(gpu.allocated_bytes(), gpu.shard_layout().reserved_bytes());
    partial_then_empty(
        &mut gpu,
        &world,
        &queue,
        32,
        4,
        vxe_render::MirrorError::PoolTooLarge {
            needed: (16_384 + 512) * SLOT_BYTES,
            limit: 16_384 * SLOT_BYTES,
        },
    );

    // Exactly the reservation fits: 32 chunks of 512 `Full` bricks take all 16 384 slots.
    let fits = World::generate(
        ChunkExtent::new([0, 0, 0], [4, 4, 2]),
        &Checker,
        &numbered_table(2),
    );
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            budget_bytes: budget_for(16_384, 32),
            ..MirrorConfig::new(Window::Fixed(fits.extent()))
        },
    );
    let st = gpu.sync(&fits, &queue).unwrap();
    assert_eq!(st.bricks, 16_384);
    assert_eq!(gpu.pool_slots(), gpu.reserved_slots());
}

impl Readback {
    /// Asserts that `coord`'s table entry is a LOD record whose 73 entries decode to the
    /// world's mips for it; returns the slots its `Full` mip-bricks occupy.
    fn assert_lod(&self, gpu: &GpuWorld, world: &World, coord: ChunkCoord) -> Vec<u32> {
        let t = self.table[gpu.table_index(coord) as usize];
        assert_ne!(t, ABSENT, "{coord:?} has no table entry");
        assert_eq!(t & LOD_BIT, LOD_BIT, "{coord:?} is not a LOD record");
        let rec = t & RECORD_MASK;
        let mips = world.lod(coord).expect("LOD-only in the world");
        let mut slots = Vec::new();
        for n in 1..=MIP_LEVELS {
            for (j, brick) in mips.level(n).iter().enumerate() {
                let e = self.records
                    [rec as usize * RECORD_WORDS + ENTRY_WORD + LOD_LEVEL_ENTRY[n - 1] + j];
                match brick {
                    Brick::Uniform(m) => assert_eq!(e, u32::from(*m), "{coord:?} L{n} {j}"),
                    Brick::Full(p) => {
                        let mut v = [AIR; BRICK_VOXELS];
                        p.decode(&mut v);
                        assert_ne!(e & FULL_BIT, 0, "{coord:?} L{n} {j}");
                        assert_eq!(self.voxels(rec, e), &v[..], "{coord:?} L{n} {j}");
                        slots.push(e & SLOT_MASK);
                    }
                }
            }
        }
        slots
    }
}

/// A two-tier residency through the mirror: LOD records decode to the world's mips, and a
/// demoted chunk trades its full record for a LOD one, its slots going back to the pool.
#[test]
fn lod_records_decode_to_the_mips_and_demotion_frees_the_full_slots() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = World::new(&mixed_table());
    let mut s = Streamer::new(
        Arc::new(Mixed),
        StreamConfig {
            residency: Residency {
                r_full: 1,
                r_lod: 2,
                hysteresis: 0,
            },
            workers: 4,
            persist: PersistConfig::default(),
        },
    );
    let mut gpu = GpuWorld::with_config(&device, small(Window::Follow { side: 7 }));
    let step = |s: &mut Streamer, gpu: &mut GpuWorld, focus: VoxelCoord| {
        s.update(&world, &[focus]);
        let ev = s.wait_idle(&world);
        gpu.apply_events(&ev);
        gpu.set_focus(focus);
        gpu.sync(&world, &queue).unwrap();
        ev
    };
    step(&mut s, &mut gpu, VoxelCoord([0, 0, 0]));
    assert_eq!(world.lod_coords().len(), 26);
    let rb = Readback::new(&device, &queue, &gpu);
    for c in world.store().coords() {
        assert!(!gpu.is_lod(c));
        rb.assert_chunk(&gpu, &world, c);
    }
    let mut lod_slots = 0;
    for c in world.lod_coords() {
        assert!(gpu.is_lod(c));
        lod_slots += rb.assert_lod(&gpu, &world, c).len();
    }
    assert!(
        lod_slots > 26,
        "the ring holds Full mip-bricks: {lod_slots}"
    );

    let demoted = ChunkCoord([1, 0, 0]);
    let full_slots: HashSet<u32> = (0..512)
        .filter_map(|slot| gpu.entry(demoted, slot))
        .filter(|e| e & FULL_BIT != 0)
        .map(|e| e & SLOT_MASK)
        .collect();
    assert!(!full_slots.is_empty());

    let ev = step(&mut s, &mut gpu, VoxelCoord([-64, 0, 0]));
    assert!(ev.demoted.contains(&demoted));
    assert!(gpu.is_lod(demoted));
    let rb = Readback::new(&device, &queue, &gpu);
    let mut live = HashSet::new();
    for c in world.store().coords() {
        live.extend(rb.assert_chunk(&gpu, &world, c));
    }
    for c in world.lod_coords() {
        live.extend(rb.assert_lod(&gpu, &world, c));
    }
    assert_eq!(live.len() as u32, gpu.live_slots());
    // Each slot the demoted chunk's full record held is free (alone or in a free page), or
    // reused by a live record; every slot handed out is live or spare.
    let free: HashSet<u32> = gpu.free_slots().iter().copied().collect();
    let free_pages: HashSet<u32> = gpu.free_pages().into_iter().collect();
    let is_free = |s: &u32| free.contains(s) || free_pages.contains(&(s / POOL_PAGE_SLOTS));
    for slot in &full_slots {
        assert!(is_free(slot) != live.contains(slot), "slot {slot}");
    }
    assert!(full_slots.iter().any(is_free));
    assert_eq!(
        u64::from(gpu.live_slots()) + gpu.spare_slots(),
        u64::from(gpu.pool_slots())
    );
    let lod_full = world.lod(demoted).unwrap().full_bricks();
    assert!(
        lod_full < full_slots.len(),
        "{lod_full} mip-bricks for {} bricks",
        full_slots.len()
    );
}

// ---- Attribute pages (contour attributes) ----

/// Chunk `[0, 0, 0]` cycles through 300 materials (wide) and chunk `[3, 0, 0]` holds
/// materials 7 and 9 below z = 4 (as in `ManyAndTwo`); every other chunk is a two-material
/// checkerboard of `Full` bricks.
struct WideAndChecker;

impl WorldSource for WideAndChecker {
    fn chunk(&self, coord: ChunkCoord, out: &mut Chunk) {
        if coord.0[0] == 0 || coord.0[0] == 3 {
            ManyAndTwo.chunk(coord, out);
        } else {
            Checker.chunk(coord, out);
        }
    }
}

/// With attribute pages, full chunks' bricks take brick slots of attribute pages, mip-bricks
/// slots of plain pages, and each brick's attribute slots lie in its own shard.
#[test]
fn attribute_pages_hold_full_bricks_and_plain_pages_mip_bricks() {
    let Some((device, queue)) = device() else {
        return;
    };
    // A wide chunk (1024 slots), two narrow chunks (1024 slots) and a LOD-only chunk.
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [4, 1, 1]),
        &WideAndChecker,
        &numbered_table(300),
    );
    let lod = ChunkCoord([3, 0, 0]);
    let chunk = world.store().remove(lod).unwrap();
    let mips = ChunkMips::build(&chunk.read().unwrap());
    world.insert_lod(lod, Arc::new(mips));
    let cfg = MirrorConfig {
        max_shard_slots: Some(POOL_PAGE_SLOTS),
        ..MirrorConfig::new(Window::Fixed(world.extent()))
    };
    let mut gpu = GpuWorld::with_surface_attrs(&device, cfg);
    gpu.sync(&world, &queue).unwrap();
    assert!(gpu.surface_attrs());
    assert!(gpu.is_wide(ChunkCoord([0, 0, 0])));
    assert!(gpu.is_lod(lod));
    let sps = gpu.shard_layout().slots_per_shard;
    assert_eq!(sps, POOL_PAGE_SLOTS);
    assert!(gpu.shards_in_use() >= 3, "{} shards", gpu.shards_in_use());

    let rb = Readback::new(&device, &queue, &gpu);
    let mut brick_pages = HashSet::new();
    let mut brick_slots = 0;
    for c in world.store().coords() {
        for s in rb.assert_chunk(&gpu, &world, c) {
            brick_slots += 1;
            assert!(gpu.in_attr_page(s), "{c:?} slot {s}");
            assert!(
                s % POOL_PAGE_SLOTS < ATTR_PAGE_BRICK_SLOTS,
                "{c:?} slot {s}"
            );
            brick_pages.insert(s / POOL_PAGE_SLOTS);
        }
        for i in 0..CHUNK_BRICKS {
            let e = gpu.entry(c, i).unwrap();
            assert_ne!(e & FULL_BIT, 0);
            let s = e & SLOT_MASK;
            if e & WIDE_BIT != 0 {
                assert_eq!(s % 2, 0, "{c:?} brick {i}: wide pair at odd slot {s}");
            }
            let a = attr_slot(s);
            assert_eq!(a, s - s % 4096 + 1364 + 2 * (s % 4096));
            for k in [a, a + 1] {
                assert_eq!(k / sps, s / sps, "{c:?} brick {i}: attribute slot {k}");
                // Attribute slots carry no occupancy mask: the CPU never writes them.
                let (o, t) = (
                    opaque_mask_word(k) as usize,
                    translucent_mask_word(k) as usize,
                );
                assert!(rb.masks[o..o + 16].iter().all(|&w| w == 0));
                assert_eq!(rb.masks[t..t + 2], [0, 0]);
            }
        }
    }
    assert_eq!(brick_slots, 2 * 512 + 2 * 512);
    assert_eq!(gpu.live_attr_slots(), 2048);
    let mip_slots = rb.assert_lod(&gpu, &world, lod);
    assert!(!mip_slots.is_empty());
    for s in &mip_slots {
        assert!(!gpu.in_attr_page(*s), "mip slot {s}");
        assert!(
            !brick_pages.contains(&(s / POOL_PAGE_SLOTS)),
            "mip slot {s}"
        );
    }
    assert_eq!(gpu.live_slots(), 2048 + mip_slots.len() as u32);

    // The live accounting adds the attribute slots and their masks of each brick slot in an
    // attribute page, against the same world mirrored without attribute pages.
    let mut plain = GpuWorld::with_config(&device, small(Window::Fixed(world.extent())));
    // Re-dirty nothing: a new mirror uploads every resident chunk on its first sync.
    plain.sync(&world, &queue).unwrap();
    assert_eq!(plain.live_slots(), gpu.live_slots());
    assert_eq!(
        gpu.mirror_bytes() - plain.mirror_bytes(),
        2048 * 2 * (SLOT_BYTES + MASK_BYTES)
    );
}

/// The decoded attribute: the octahedral normal and the offset code, or `None` for no plane.
fn decode_attr(a: u16) -> Option<([f32; 3], u16)> {
    if a == NO_PLANE {
        return None;
    }
    let q = [f32::from(a & 63), f32::from((a >> 6) & 63)].map(|v| v / 63.0 * 2.0 - 1.0);
    let mut n = [q[0], q[1], 1.0 - q[0].abs() - q[1].abs()];
    if n[2] < 0.0 {
        let (x, y) = (n[0], n[1]);
        n[0] = (1.0 - y.abs()) * if x >= 0.0 { 1.0 } else { -1.0 };
        n[1] = (1.0 - x.abs()) * if y >= 0.0 { 1.0 } else { -1.0 };
    }
    let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    Some((n.map(|v| v / l), a >> 12))
}

/// Runs the attribute pass over what changed and waits for it.
fn update_attrs(device: &wgpu::Device, queue: &wgpu::Queue, gpu: &mut GpuWorld) -> usize {
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let n = gpu.update_attrs(&mut enc);
    queue.submit([enc.finish()]);
    n
}

/// The attribute of world voxel `v`, which lies in a `Full` brick of a mirrored full chunk.
fn attr_of(device: &wgpu::Device, queue: &wgpu::Queue, gpu: &GpuWorld, v: [i32; 3]) -> u16 {
    let voxel = VoxelCoord(v);
    let e = gpu.entry(voxel.chunk(), voxel.brick_slot()).unwrap();
    assert_ne!(e & FULL_BIT, 0, "{v:?} is in a Full brick");
    let a = attr_slot(e & SLOT_MASK);
    let sps = gpu.shard_layout().slots_per_shard;
    let i = voxel.index_in_brick() as u64;
    let bytes = readback::read_buffer(
        device,
        queue,
        gpu.pool_shard((a / sps) as usize),
        u64::from(a % sps) * SLOT_BYTES + (i / 2) * 4,
        4,
    );
    let word = u32::from_le_bytes(bytes.try_into().unwrap());
    (word >> (16 * (i & 1))) as u16
}

#[test]
fn the_attribute_pass_fills_the_floor_and_reaches_a_carve_s_neighbours() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = world();
    let mut gpu = GpuWorld::with_surface_attrs(&device, small(Window::Fixed(world.extent())));
    gpu.sync(&world, &queue).unwrap();
    assert_eq!(update_attrs(&device, &queue, &mut gpu), 256);
    assert_eq!(
        update_attrs(&device, &queue, &mut gpu),
        0,
        "nothing changed"
    );

    // The top solid layer: 18 of the 27 voxels solid, offset (18/27 - 0.5) * 3 = +1/2, code 11.
    let (n, code) = decode_attr(attr_of(&device, &queue, &gpu, [3, 3, 2])).expect("a plane");
    let angle = n[2].clamp(-1.0, 1.0).acos().to_degrees();
    assert!(angle <= 3.0, "normal {n:?} is {angle} degrees off +z");
    assert_eq!(code, 11);
    // Six solid face neighbours: no plane.
    assert_eq!(attr_of(&device, &queue, &gpu, [3, 3, 1]), NO_PLANE);
    // Interior beside brick [0, 8)^3's +x face, in the unchanged brick [8, 16) x [0, 8)^2.
    let beside = [8, 3, 1];
    assert_eq!(attr_of(&device, &queue, &gpu, beside), NO_PLANE);

    // Carve x in [6, 8) of the floor in brick [0, 8)^3: the voxel beside it gains a plane.
    let r = world
        .apply(&EditOp::box_(
            Fixed3([6 * FIXED_ONE, 0, 0]),
            Fixed3([8 * FIXED_ONE, 8 * FIXED_ONE, 3 * FIXED_ONE]),
            Mode::Carve,
        ))
        .unwrap();
    assert_eq!(r.changed_from, [(1, 2 * 8 * 3)]);
    let s = gpu.sync(&world, &queue).unwrap();
    assert_eq!(s.bricks, 1, "only the carved brick is written");
    // The carved brick and its 26 neighbours: those in the mirror that are Full.
    let n = update_attrs(&device, &queue, &mut gpu);
    assert!(n > 1 && n <= 27, "{n} bricks");
    let after = attr_of(&device, &queue, &gpu, beside);
    assert_ne!(after, NO_PLANE);

    // A mirror built from scratch computes every attribute: the neighbour pass agrees.
    let mut fresh = GpuWorld::with_surface_attrs(&device, small(Window::Fixed(world.extent())));
    fresh.sync(&world, &queue).unwrap();
    update_attrs(&device, &queue, &mut fresh);
    assert_eq!(after, attr_of(&device, &queue, &fresh, beside));
    assert_eq!(
        attr_of(&device, &queue, &gpu, [5, 3, 1]),
        attr_of(&device, &queue, &fresh, [5, 3, 1])
    );
}

/// The bound on pages ever issued for the peak live brick slots of each kind: per kind, the
/// pages its peak fills (1364 brick slots an attribute page, 4096 a plain one) and one more.
fn page_bound(peak_plain: u32, peak_attr: u32) -> u32 {
    peak_plain.div_ceil(POOL_PAGE_SLOTS) + 1 + peak_attr.div_ceil(ATTR_PAGE_BRICK_SLOTS) + 1
}

/// Kind churn in a following window: 8 chunks alternate 20 times between full with
/// attributes and LOD-only, beside 2 chunks that stay full. The pages ever issued stay within
/// the per-kind bound, the stable chunks' bricks never change slot, and after the churn every
/// live brick and mip-brick decodes to its CPU voxels and the churned chunks' attributes equal
/// a fresh mirror's.
#[test]
fn kind_churn_reuses_empty_pages_and_moves_no_live_brick() {
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
    let mut gpu = GpuWorld::with_surface_attrs(&device, small(Window::Follow { side: 7 }));
    gpu.set_focus(VoxelCoord([2 * 64 + 1, 64 + 1, 1]));
    gpu.sync(&world, &queue).unwrap();
    update_attrs(&device, &queue, &mut gpu);
    let entries = |gpu: &GpuWorld| -> Vec<Option<u32>> {
        stable
            .iter()
            .flat_map(|&c| (0..CHUNK_BRICKS).map(move |i| (c, i)))
            .map(|(c, i)| gpu.entry(c, i))
            .collect()
    };
    let stable_entries = entries(&gpu);
    let mut peak = [0u32; 2];
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
        let st = gpu.sync(&world, &queue).unwrap();
        assert_eq!(st.pending_chunks, 0);
        update_attrs(&device, &queue, &mut gpu);
        for &c in &churn {
            assert_eq!(gpu.is_lod(c), lod, "round {round} {c:?}");
        }
        let attr = gpu.live_attr_slots();
        peak = [peak[0].max(gpu.live_slots() - attr), peak[1].max(attr)];
        let bound = page_bound(peak[0], peak[1]);
        assert!(
            gpu.pages_issued() <= bound,
            "round {round}: {} pages issued, bound {bound} for peaks {peak:?}",
            gpu.pages_issued()
        );
        assert_eq!(entries(&gpu), stable_entries, "round {round}");
    }
    println!(
        "{} pages issued for peaks {peak:?}, {} free",
        gpu.pages_issued(),
        gpu.free_pages().len()
    );

    let rb = Readback::new(&device, &queue, &gpu);
    let mut live = HashSet::new();
    for c in world.store().coords() {
        for s in rb.assert_chunk(&gpu, &world, c) {
            assert!(gpu.in_attr_page(s), "{c:?} slot {s}");
            assert!(live.insert(s), "slot {s} held twice");
        }
    }
    assert_eq!(live.len() as u32, gpu.live_slots());
    let mut fresh = GpuWorld::with_surface_attrs(&device, small(Window::Follow { side: 7 }));
    fresh.set_focus(VoxelCoord([2 * 64 + 1, 64 + 1, 1]));
    fresh.sync(&world, &queue).unwrap();
    update_attrs(&device, &queue, &mut fresh);
    let mut compared = 0;
    for &c in &churn {
        let base = c.min_voxel().0;
        for slot in (0..CHUNK_BRICKS).step_by(7) {
            if gpu.entry(c, slot).is_some_and(|e| e & FULL_BIT != 0) {
                let b = brick_of_slot(slot);
                let v = [0, 1, 2].map(|k| base[k] + b[k] * BRICK_EDGE + 3);
                assert_eq!(
                    attr_of(&device, &queue, &gpu, v),
                    attr_of(&device, &queue, &fresh, v),
                    "{c:?} voxel {v:?}"
                );
                compared += 1;
            }
        }
    }
    assert!(compared > 20, "{compared} attributes compared");
}

/// An attribute page whose bricks all leave becomes free and serves plain slots: 8 full
/// chunks of `Full` bricks fill attribute pages 0 to 3; evicted, the 4 pages are free; mips
/// at the same coordinates (from `Mixed`, whose mip-bricks are partly `Full`; a checkerboard's
/// are uniform) then take plain slots from page 0 with no new page, and the full chunks coming
/// back take pages 1 to 3, then page 0 again once the last chunk's mips leave it.
#[test]
fn an_empty_attribute_page_serves_plain_slots() {
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
    let mut gpu = GpuWorld::with_surface_attrs(&device, small(Window::Fixed(world.extent())));
    gpu.sync(&world, &queue).unwrap();
    assert_eq!(gpu.live_attr_slots(), 8 * 512);
    assert_eq!(gpu.pages_issued(), 4);
    assert!(gpu.free_pages().is_empty());

    let mut held = Vec::new();
    for &c in &coords {
        held.push((c, world.store().remove(c).unwrap()));
    }
    gpu.apply_events(&StreamEvents {
        evicted: coords.clone(),
        ..StreamEvents::default()
    });
    gpu.sync(&world, &queue).unwrap();
    assert_eq!(gpu.live_slots(), 0);
    assert_eq!(gpu.free_pages(), [0, 1, 2, 3]);
    assert_eq!(gpu.spare_slots(), 4 * u64::from(POOL_PAGE_SLOTS));

    let mixed = World::generate(world.extent(), &Mixed, &mixed_table());
    for &c in &coords {
        let chunk = mixed.store().get(c).unwrap();
        world.insert_lod(c, Arc::new(ChunkMips::build(&chunk.read().unwrap())));
    }
    gpu.sync(&world, &queue).unwrap();
    let rb = Readback::new(&device, &queue, &gpu);
    let mut mip_slots = 0;
    for &c in &coords {
        for s in rb.assert_lod(&gpu, &world, c) {
            assert_eq!(s / POOL_PAGE_SLOTS, 0, "{c:?} mip slot {s}");
            assert!(!gpu.in_attr_page(s));
            mip_slots += 1;
        }
    }
    assert!(mip_slots > 0);
    assert_eq!(gpu.live_slots(), mip_slots);
    assert_eq!(
        gpu.pages_issued(),
        4,
        "no page past the four already issued"
    );
    assert_eq!(gpu.free_pages(), [1, 2, 3]);

    for (c, chunk) in held {
        world.remove_lod(c);
        world.store().write_lock().insert(c, chunk);
    }
    gpu.sync(&world, &queue).unwrap();
    assert_eq!(gpu.live_attr_slots(), 8 * 512);
    assert_eq!(gpu.live_slots(), 8 * 512);
    assert_eq!(
        gpu.pages_issued(),
        4,
        "page 0 changed kind twice, no page was added"
    );
    assert!(gpu.free_pages().is_empty());
    let rb = Readback::new(&device, &queue, &gpu);
    let mut in_page_0 = 0;
    for &c in &coords {
        for s in rb.assert_chunk(&gpu, &world, c) {
            assert!(gpu.in_attr_page(s), "{c:?} slot {s}");
            in_page_0 += usize::from(s / POOL_PAGE_SLOTS == 0);
        }
    }
    assert!(
        in_page_0 > 0,
        "the last bricks took page 0 back as an attribute page"
    );
}

/// Pressure on slots: a fixed window whose `Full` bricks fill more than 0.9 of a two-page
/// reservation, with records far from full.
#[test]
fn pressure_follows_the_slots_when_they_run_out() {
    let Some((device, queue)) = device() else {
        return;
    };
    // 15 chunks of 512 `Full` bricks: 7680 of 8192 slots.
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [5, 3, 1]),
        &Checker,
        &numbered_table(2),
    );
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            budget_bytes: budget_for(8192, 40),
            record_chunks: Some(40),
            ..MirrorConfig::new(Window::Fixed(world.extent()))
        },
    );
    assert_eq!((gpu.reserved_slots(), gpu.reserved_records()), (8192, 40));
    gpu.sync(&world, &queue).unwrap();
    let (slots, records) = pressure_terms(&gpu);
    assert_eq!(slots, 7680.0 / 8192.0);
    assert!(slots > 0.9 && records < 0.5, "{slots} {records}");
    assert_pressure(&gpu);
    assert!((f64::from(gpu.pressure()) - slots).abs() < 1e-6);
}

/// Pressure on records: uniform chunks take no slot, and 37 of 40 records read past 0.9
/// with slots at 0. A mirror given `record_chunks` reserves exactly that many, and its
/// reservation is still the budget rounded down.
#[test]
fn pressure_follows_the_records_when_they_run_out() {
    let Some((device, queue)) = device() else {
        return;
    };
    let mut t = MaterialTable::new();
    let rock = t.push(Material::named("rock"));
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [37, 1, 1]),
        &UniformSource(rock),
        &t,
    );
    let budget = budget_for(4096, 40);
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            budget_bytes: budget,
            record_chunks: Some(40),
            ..MirrorConfig::new(Window::Fixed(world.extent()))
        },
    );
    assert_eq!(gpu.reserved_records(), 40);
    assert_eq!(gpu.allocated_bytes(), gpu.shard_layout().reserved_bytes());
    assert!(gpu.allocated_bytes() <= budget);
    assert!(budget - gpu.allocated_bytes() < PAGE_BYTES + PLACEHOLDER_BYTES);
    gpu.sync(&world, &queue).unwrap();
    let (slots, records) = pressure_terms(&gpu);
    assert_eq!(slots, 0.0);
    assert_eq!(records, 37.0 / 40.0);
    assert!(records > 0.9);
    assert!((f64::from(gpu.pressure()) - records).abs() < 1e-6);
}

/// A chunk as the store holds it.
type SharedChunk = Arc<std::sync::RwLock<Chunk>>;

/// A fixed window over chunks x = 0..10 (y = z = 0) of `Checker`, emptied of every chunk; the
/// chunks are handed back to put in the store, and `Mixed` mips at the same coordinates.
fn tier_world() -> (World, Vec<SharedChunk>, Vec<Arc<ChunkMips>>) {
    let extent = ChunkExtent::new([0, 0, 0], [10, 1, 1]);
    let world = World::generate(extent, &Checker, &numbered_table(2));
    let mixed = World::generate(extent, &Mixed, &mixed_table());
    let mut chunks = Vec::new();
    let mut mips = Vec::new();
    for x in 0..10 {
        let c = ChunkCoord([x, 0, 0]);
        chunks.push(world.store().remove(c).unwrap());
        let m = mixed.store().get(c).unwrap();
        mips.push(Arc::new(ChunkMips::build(&m.read().unwrap())));
    }
    (world, chunks, mips)
}

/// Makes chunk `x` of the tier world LOD-only with `mips`.
fn to_lod(world: &World, x: i32, mips: &Arc<ChunkMips>) {
    let c = ChunkCoord([x, 0, 0]);
    world.store().remove(c);
    world.insert_lod(c, mips.clone());
}

/// A demotion that frees its pages in the same sync fits a full pool, with attribute pages:
/// three `Checker` chunks fill both reserved pages with attribute slots (512, 512, then 340 of
/// page 0 and 172 of page 1); one sync demotes them to their own mips (a checkerboard's
/// mip-bricks are uniform, so they take no slot) and adds a farther LOD-only chunk whose
/// `Mixed` mips need plain slots. The demotions empty both pages first, so the plain slots
/// come from a freed page. A pool check that credited no tier change's frees would ask for a
/// third page past the two-page reservation and fail the sync with `PoolTooLarge`.
#[test]
fn a_demotion_that_empties_attribute_pages_fits_a_full_pool() {
    let Some((device, queue)) = device() else {
        return;
    };
    let (world, chunks, mixed) = tier_world();
    for x in 0..3 {
        world
            .store()
            .write_lock()
            .insert(ChunkCoord([x, 0, 0]), chunks[x as usize].clone());
    }
    let mut gpu = GpuWorld::with_surface_attrs(
        &device,
        MirrorConfig {
            budget_bytes: budget_for(2 * 4096, 10),
            ..MirrorConfig::new(Window::Fixed(world.extent()))
        },
    );
    gpu.set_focus(VoxelCoord([1, 1, 1]));
    gpu.sync(&world, &queue).unwrap();
    assert_eq!(gpu.reserved_slots(), 2 * 4096);
    assert_eq!(gpu.live_attr_slots(), 3 * 512);
    assert_eq!(gpu.pages_issued(), 2);

    for x in 0..3 {
        let own = Arc::new(ChunkMips::build(&chunks[x as usize].read().unwrap()));
        assert_eq!(own.full_bricks(), 0, "a checkerboard's mips are uniform");
        to_lod(&world, x, &own);
    }
    world.insert_lod(ChunkCoord([3, 0, 0]), mixed[3].clone());
    let need = mixed[3].full_bricks() as u32;
    assert!(need > 0);
    let st = gpu
        .sync(&world, &queue)
        .expect("the demotions free the pages first");
    assert_eq!(st.chunks, 4);
    assert_eq!(gpu.live_attr_slots(), 0);
    assert_eq!(gpu.live_slots(), need);
    assert_eq!(gpu.pages_issued(), 2);
    let rb = Readback::new(&device, &queue, &gpu);
    for x in 0..4 {
        rb.assert_lod(&gpu, &world, ChunkCoord([x, 0, 0]));
    }
}

/// The same without attribute pages: eight `Checker` chunks fill a one-page reservation;
/// demoting them and adding a farther LOD-only chunk fits, its mip-bricks taking slots the
/// first demotion frees. A pool check that credited no tier change's frees would fail it with
/// `PoolTooLarge` (4096 slots plus the new mips past 4096).
#[test]
fn a_plain_demotion_fits_a_full_pool() {
    let Some((device, queue)) = device() else {
        return;
    };
    let (world, chunks, mixed) = tier_world();
    for x in 0..8 {
        world
            .store()
            .write_lock()
            .insert(ChunkCoord([x, 0, 0]), chunks[x as usize].clone());
    }
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            budget_bytes: budget_for(4096, 10),
            ..MirrorConfig::new(Window::Fixed(world.extent()))
        },
    );
    gpu.set_focus(VoxelCoord([1, 1, 1]));
    gpu.sync(&world, &queue).unwrap();
    assert_eq!(gpu.live_slots(), 4096);
    assert_eq!(gpu.reserved_slots(), 4096);

    for x in 0..8 {
        let own = Arc::new(ChunkMips::build(&chunks[x as usize].read().unwrap()));
        to_lod(&world, x, &own);
    }
    world.insert_lod(ChunkCoord([8, 0, 0]), mixed[8].clone());
    let st = gpu
        .sync(&world, &queue)
        .expect("the first demotion frees slots first");
    assert_eq!(st.chunks, 9);
    assert_eq!(gpu.live_slots(), mixed[8].full_bricks() as u32);
    let rb = Readback::new(&device, &queue, &gpu);
    for x in 0..9 {
        rb.assert_lod(&gpu, &world, ChunkCoord([x, 0, 0]));
    }
}

/// A selection whose net need exceeds the reservation writes what fits net of its frees: a
/// full one-page pool of eight `Checker` chunks, one of them demoted (freeing 512 slots)
/// while two new full chunks need 1024. The demotion and the nearer new chunk upload, the
/// farther waits; the next sync, its nearest chunk alone not fitting and no demotion behind
/// it, is empty and writes nothing.
#[test]
fn a_selection_past_the_reservation_net_of_its_frees_still_fails_untouched() {
    let Some((device, queue)) = device() else {
        return;
    };
    let (world, chunks, _) = tier_world();
    for x in 0..8 {
        world
            .store()
            .write_lock()
            .insert(ChunkCoord([x, 0, 0]), chunks[x as usize].clone());
    }
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            budget_bytes: budget_for(4096, 10),
            ..MirrorConfig::new(Window::Fixed(world.extent()))
        },
    );
    gpu.set_focus(VoxelCoord([1, 1, 1]));
    gpu.sync(&world, &queue).unwrap();
    let own = Arc::new(ChunkMips::build(&chunks[0].read().unwrap()));
    to_lod(&world, 0, &own);
    for x in 8..10 {
        world
            .store()
            .write_lock()
            .insert(ChunkCoord([x, 0, 0]), chunks[x as usize].clone());
    }
    let st = gpu.sync(&world, &queue).unwrap();
    assert_eq!(
        (st.chunks, st.deferred_chunks, st.pending_chunks),
        (2, 1, 1)
    );
    assert!(gpu.is_lod(ChunkCoord([0, 0, 0])));
    assert!(gpu.is_mirrored(ChunkCoord([8, 0, 0])));
    assert!(!gpu.is_mirrored(ChunkCoord([9, 0, 0])));
    assert_eq!(gpu.live_slots(), 4096);
    let (bytes, mirrored, live) = (gpu.bytes_written(), gpu.mirrored_chunks(), gpu.live_slots());
    let err = gpu.sync(&world, &queue).unwrap_err();
    assert_eq!(
        err,
        vxe_render::MirrorError::PoolTooLarge {
            needed: (4096 + 512) * SLOT_BYTES,
            limit: 4096 * SLOT_BYTES,
        }
    );
    assert_eq!(gpu.bytes_written(), bytes);
    assert_eq!(gpu.mirrored_chunks(), mirrored);
    assert_eq!(gpu.live_slots(), live);
    assert_eq!(gpu.pending_chunks(), 1);
}

/// Record words, palette included, and table entry the GPU holds for `coord`.
fn record_and_entry(rb: &Readback, gpu: &GpuWorld, coord: ChunkCoord) -> (Vec<u32>, u32) {
    let t = rb.table[gpu.table_index(coord) as usize];
    let rec = (t & RECORD_MASK) as usize;
    (rb.records[rec * RECORD_WORDS..][..RECORD_WORDS].to_vec(), t)
}

/// One page short, with attribute pages: eight LOD-only chunks (their `Mixed` mips in plain
/// page 0) are promoted, nearest first, onto a three-page reservation. Pages 1 and 2 take five
/// chunks' 2560 attribute bricks; the sixth would need a fourth page. The sync returns `Ok`:
/// the five nearest read back to their voxels, the other three are deferred and pending, and
/// nothing of theirs was written (their LOD records, palettes and table entries read back as
/// before, still decoding to their mips; their chunks keep their dirty bits). After three
/// demotions free page 1, the next sync uploads them.
#[test]
fn one_page_short_uploads_the_nearest_and_defers_the_rest_untouched() {
    let Some((device, queue)) = device() else {
        return;
    };
    let (world, chunks, mixed) = tier_world();
    for x in 0..8 {
        world.insert_lod(ChunkCoord([x, 0, 0]), mixed[x as usize].clone());
    }
    let mut gpu = GpuWorld::with_surface_attrs(
        &device,
        MirrorConfig {
            budget_bytes: budget_for(3 * 4096, 10),
            ..MirrorConfig::new(Window::Fixed(world.extent()))
        },
    );
    gpu.set_focus(VoxelCoord([1, 1, 1]));
    gpu.sync(&world, &queue).unwrap();
    assert_eq!(gpu.pages_issued(), 1);
    let mips_before = gpu.live_slots();
    let rb = Readback::new(&device, &queue, &gpu);
    let before: Vec<_> = (5..8)
        .map(|x| record_and_entry(&rb, &gpu, ChunkCoord([x, 0, 0])))
        .collect();

    for x in 0..8 {
        let c = ChunkCoord([x, 0, 0]);
        world.remove_lod(c);
        world
            .store()
            .write_lock()
            .insert(c, chunks[x as usize].clone());
    }
    let st = gpu.sync(&world, &queue).unwrap();
    assert_eq!(st.chunks, 5);
    assert_eq!(st.deferred_chunks, 3);
    assert_eq!(st.pending_chunks, 3);
    assert_eq!(gpu.pages_issued(), 3);
    let freed: u32 = (0..5).map(|x| mixed[x].full_bricks() as u32).sum();
    assert_eq!(gpu.live_slots(), mips_before - freed + 5 * 512);
    let rb = Readback::new(&device, &queue, &gpu);
    for x in 0..5 {
        rb.assert_chunk(&gpu, &world, ChunkCoord([x, 0, 0]));
    }
    for (i, x) in (5..8).enumerate() {
        let c = ChunkCoord([x, 0, 0]);
        assert!(gpu.is_lod(c), "{c:?} keeps its LOD record");
        assert_eq!(record_and_entry(&rb, &gpu, c), before[i], "{c:?}");
        // The world holds it full now; its record still decodes to the mips it was built from.
        let lod_world = World::new(&mixed_table());
        lod_world.insert_lod(c, mixed[x as usize].clone());
        rb.assert_lod(&gpu, &lod_world, c);
        assert!(chunks[x as usize].read().unwrap().any_dirty(), "{c:?}");
    }

    // Demote the three nearest (to their own uniform mips): page 1 empties.
    for x in 0..3 {
        let own = Arc::new(ChunkMips::build(&chunks[x as usize].read().unwrap()));
        to_lod(&world, x, &own);
    }
    let st = gpu.sync(&world, &queue).unwrap();
    assert_eq!(st.deferred_chunks, 0);
    assert_eq!(st.pending_chunks, 0);
    let rb = Readback::new(&device, &queue, &gpu);
    for x in 3..8 {
        rb.assert_chunk(&gpu, &world, ChunkCoord([x, 0, 0]));
    }
    for x in 0..3 {
        rb.assert_lod(&gpu, &world, ChunkCoord([x, 0, 0]));
    }
}

/// `Floor` at x = 0 and every 4th chunk from 6 (64 `Full` bricks each), `Checker` elsewhere.
struct FloorAndChecker;

impl WorldSource for FloorAndChecker {
    fn chunk(&self, coord: ChunkCoord, out: &mut Chunk) {
        if coord.0[0] == 0 {
            Floor.chunk(coord, out);
        } else {
            Checker.chunk(coord, out);
        }
    }
}

/// A dug chunk goes first: on a one-page pool holding a `Floor` chunk at the focus and four
/// far `Checker` chunks (2112 slots), five nearer chunks are promoted from LOD-only (their
/// checkerboard mips take no slot) while a fill at the focus gains `Full` bricks. The dug
/// chunk uploads and reads back to its edited voxels; the promotions that fit follow nearest
/// first, and only chunks farther than every uploaded one are deferred.
#[test]
fn a_dug_chunk_uploads_ahead_of_farther_promotions() {
    let Some((device, queue)) = device() else {
        return;
    };
    let extent = ChunkExtent::new([0, 0, 0], [10, 1, 1]);
    let world = World::generate(extent, &FloorAndChecker, &numbered_table(2));
    let mut held = Vec::new();
    for x in 1..6 {
        let c = ChunkCoord([x, 0, 0]);
        let chunk = world.store().remove(c).unwrap();
        world.insert_lod(c, Arc::new(ChunkMips::build(&chunk.read().unwrap())));
        held.push((c, chunk));
    }
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            budget_bytes: budget_for(4096, 10),
            ..MirrorConfig::new(Window::Fixed(extent))
        },
    );
    gpu.set_focus(VoxelCoord([1, 1, 1]));
    gpu.sync(&world, &queue).unwrap();
    assert_eq!(gpu.live_slots(), 64 + 4 * 512);

    let r = world
        .apply(&EditOp::box_(
            centre_of_voxel([0, 0, 8]),
            centre_of_voxel([16, 8, 10]),
            Mode::Fill(1),
        ))
        .unwrap();
    assert_eq!(r.touched, [ChunkCoord([0, 0, 0])]);
    for (c, chunk) in held {
        world.remove_lod(c);
        world.store().write_lock().insert(c, chunk);
    }
    let st = gpu.sync(&world, &queue).unwrap();
    // 4096 - 2112 = 1984 spare: 2 dug bricks, then three promotions of 512 (1536).
    assert_eq!(st.chunks, 4);
    assert_eq!(st.deferred_chunks, 2);
    let rb = Readback::new(&device, &queue, &gpu);
    rb.assert_chunk(&gpu, &world, ChunkCoord([0, 0, 0]));
    assert_eq!(full_bricks(&world, &[ChunkCoord([0, 0, 0])]), 66);
    for x in 1..4 {
        rb.assert_chunk(&gpu, &world, ChunkCoord([x, 0, 0]));
    }
    for x in 4..6 {
        assert!(gpu.is_lod(ChunkCoord([x, 0, 0])), "x {x} deferred");
    }
}

/// A demotion is not blocked: the nearest selected chunk, a promotion of 512 attribute
/// bricks, does not fit a two-page pool with 168 spare; two farther demotions behind it do
/// (they need nothing), upload and read back to their mips, and free 1024 slots, so the
/// promotion uploads in the next sync.
#[test]
fn a_demotion_behind_a_deferred_chunk_still_uploads() {
    let Some((device, queue)) = device() else {
        return;
    };
    let (world, chunks, _) = tier_world();
    let own: Vec<Arc<ChunkMips>> = chunks
        .iter()
        .map(|c| Arc::new(ChunkMips::build(&c.read().unwrap())))
        .collect();
    world.insert_lod(ChunkCoord([0, 0, 0]), own[0].clone());
    for x in 1..6 {
        world
            .store()
            .write_lock()
            .insert(ChunkCoord([x, 0, 0]), chunks[x as usize].clone());
    }
    let mut gpu = GpuWorld::with_surface_attrs(
        &device,
        MirrorConfig {
            budget_bytes: budget_for(2 * 4096, 10),
            ..MirrorConfig::new(Window::Fixed(world.extent()))
        },
    );
    gpu.set_focus(VoxelCoord([1, 1, 1]));
    gpu.sync(&world, &queue).unwrap();
    assert_eq!(gpu.live_attr_slots(), 5 * 512);
    assert_eq!(gpu.pages_issued(), 2);

    world.remove_lod(ChunkCoord([0, 0, 0]));
    world
        .store()
        .write_lock()
        .insert(ChunkCoord([0, 0, 0]), chunks[0].clone());
    for x in 4..6 {
        to_lod(&world, x, &own[x as usize]);
    }
    let st = gpu.sync(&world, &queue).unwrap();
    assert_eq!((st.chunks, st.deferred_chunks), (2, 1));
    assert!(gpu.is_lod(ChunkCoord([0, 0, 0])), "the promotion waits");
    let rb = Readback::new(&device, &queue, &gpu);
    for x in 4..6 {
        rb.assert_lod(&gpu, &world, ChunkCoord([x, 0, 0]));
    }
    let st = gpu.sync(&world, &queue).unwrap();
    assert_eq!((st.chunks, st.deferred_chunks), (1, 0));
    let rb = Readback::new(&device, &queue, &gpu);
    rb.assert_chunk(&gpu, &world, ChunkCoord([0, 0, 0]));
}

/// A chunk that turns wide in the sync, on a pool that can take its new bricks narrow but not
/// its every brick as a slot pair, is deferred whole: its narrow record, palette and table
/// entry read back as before and still decode to the voxels it had.
#[test]
fn a_chunk_turning_wide_past_the_pool_is_deferred_whole() {
    let Some((device, queue)) = device() else {
        return;
    };
    // x 0..6 `Floor` (64 `Full` bricks each), x 6..13 `Checker`: 3968 of 4096 slots.
    struct Fillers;
    impl WorldSource for Fillers {
        fn chunk(&self, coord: ChunkCoord, out: &mut Chunk) {
            if coord.0[0] < 6 {
                Floor.chunk(coord, out);
            } else {
                Checker.chunk(coord, out);
            }
        }
    }
    let extent = ChunkExtent::new([0, 0, 0], [13, 1, 1]);
    let world = World::generate(extent, &Fillers, &numbered_table(300));
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            budget_bytes: budget_for(4096, 13),
            ..MirrorConfig::new(Window::Fixed(extent))
        },
    );
    let c = ChunkCoord([0, 0, 0]);
    gpu.set_focus(VoxelCoord([1, 1, 1]));
    gpu.sync(&world, &queue).unwrap();
    assert_eq!(gpu.live_slots(), 6 * 64 + 7 * 512);
    let snapshot = |gpu: &GpuWorld| {
        let rb = Readback::new(&device, &queue, gpu);
        let voxels: Vec<Vec<u16>> = (0..CHUNK_BRICKS)
            .filter_map(|i| gpu.entry(c, i).filter(|e| e & FULL_BIT != 0))
            .map(|e| {
                let rec = gpu.record_index(c).unwrap();
                rb.voxels(rec, e)
            })
            .collect();
        (record_and_entry(&rb, gpu, c), voxels)
    };
    let before = snapshot(&gpu);

    // 255 new materials, one voxel each, in the air above the floor: the chunk turns wide.
    for m in 2..=256u16 {
        let k = i32::from(m);
        fill_voxel(&world, [k % 64, k / 64 * 8, 9 + (k / 32) % 2 * 8], m);
    }
    let full = full_bricks(&world, &[c]);
    assert!(
        2 * full > 128,
        "{full} bricks as pairs past the 128 spare slots"
    );
    let written = gpu.bytes_written();
    let st = gpu.sync(&world, &queue).unwrap_err();
    assert_eq!(gpu.bytes_written(), written, "an empty sync writes nothing");
    assert!(
        matches!(st, vxe_render::MirrorError::PoolTooLarge { .. }),
        "{st:?}"
    );
    assert!(!gpu.is_wide(c));
    assert_eq!(snapshot(&gpu), before);
    assert_eq!(gpu.pending_chunks(), 1);
    assert_eq!(gpu.live_slots(), 6 * 64 + 7 * 512);
    let chunk = world.store().get(c).unwrap();
    assert!(
        chunk.read().unwrap().any_dirty(),
        "the deferred chunk keeps its dirty bits"
    );
}

/// A page the sync allocates into is not credited free by a later demotion's frees. Two
/// attribute pages: x = 0 LOD-only with uniform mips, x = 1..4 full (page 0 full, page 1
/// holding 172 of x = 3's slots, its cursor short of the page end). One sync promotes x = 0
/// (512 attribute bricks onto page 1's cursor), demotes x = 3 (freeing its 172 on page 1,
/// which x = 0's bricks keep live) and adds a LOD-only x = 4 whose `Mixed` mips need a plain
/// page. Crediting page 1 as emptied would place x = 4 there and fail inside the upload; x = 4
/// is deferred whole instead, and the sync is `Ok`.
#[test]
fn a_page_the_sync_allocated_into_is_not_credited_free() {
    let Some((device, queue)) = device() else {
        return;
    };
    let (world, chunks, mixed) = tier_world();
    let own: Vec<Arc<ChunkMips>> = chunks
        .iter()
        .map(|c| Arc::new(ChunkMips::build(&c.read().unwrap())))
        .collect();
    world.insert_lod(ChunkCoord([0, 0, 0]), own[0].clone());
    for x in 1..4 {
        world
            .store()
            .write_lock()
            .insert(ChunkCoord([x, 0, 0]), chunks[x as usize].clone());
    }
    let mut gpu = GpuWorld::with_surface_attrs(
        &device,
        MirrorConfig {
            budget_bytes: budget_for(2 * 4096, 10),
            ..MirrorConfig::new(Window::Fixed(world.extent()))
        },
    );
    gpu.set_focus(VoxelCoord([1, 1, 1]));
    gpu.sync(&world, &queue).unwrap();
    assert_eq!(gpu.live_attr_slots(), 3 * 512);
    assert_eq!(gpu.pages_issued(), 2);

    world.remove_lod(ChunkCoord([0, 0, 0]));
    world
        .store()
        .write_lock()
        .insert(ChunkCoord([0, 0, 0]), chunks[0].clone());
    to_lod(&world, 3, &own[3]);
    world.insert_lod(ChunkCoord([4, 0, 0]), mixed[4].clone());
    assert!(mixed[4].full_bricks() > 0);
    let st = gpu
        .sync(&world, &queue)
        .expect("x = 4 waits; the rest uploads");
    assert_eq!((st.chunks, st.deferred_chunks), (2, 1));
    assert!(!gpu.is_mirrored(ChunkCoord([4, 0, 0])));
    assert_eq!(gpu.pending_chunks(), 1);
    assert_eq!(gpu.live_attr_slots(), 3 * 512);
    assert_eq!(gpu.live_slots(), 3 * 512);
    let rb = Readback::new(&device, &queue, &gpu);
    rb.assert_chunk(&gpu, &world, ChunkCoord([0, 0, 0]));
    rb.assert_lod(&gpu, &world, ChunkCoord([3, 0, 0]));
}

/// The upload's backstop: a chunk turning wide takes the room the selection counted on for
/// a later chunk. On a one-page pool with 512 spare slots, `Floor` chunk x = 0 gains 255
/// materials inside its 64 `Full` bricks (no new brick, so the selection counts no slot for
/// it) and a new `Checker` chunk x = 14 needs 512: the selection fits both. In the upload
/// x = 0 turns wide (64 slot pairs from the spare, its 64 narrow slots freed after), which
/// leaves 448; x = 14 is checked against the allocator as it then is and deferred whole.
#[test]
fn a_widening_chunk_defers_the_later_chunk_it_crowds_out() {
    let Some((device, queue)) = device() else {
        return;
    };
    // x 0..8 `Floor` (64 `Full` bricks each), x 8..15 `Checker` (512 each).
    struct Layout;
    impl WorldSource for Layout {
        fn chunk(&self, coord: ChunkCoord, out: &mut Chunk) {
            if coord.0[0] < 8 {
                Floor.chunk(coord, out);
            } else {
                Checker.chunk(coord, out);
            }
        }
    }
    let extent = ChunkExtent::new([0, 0, 0], [15, 1, 1]);
    let world = World::generate(extent, &Layout, &numbered_table(300));
    let late = ChunkCoord([14, 0, 0]);
    let held = world.store().remove(late).unwrap();
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            budget_bytes: budget_for(4096, 15),
            ..MirrorConfig::new(Window::Fixed(extent))
        },
    );
    gpu.set_focus(VoxelCoord([1, 1, 1]));
    gpu.sync(&world, &queue).unwrap();
    assert_eq!(gpu.live_slots(), 8 * 64 + 6 * 512);
    assert_eq!(gpu.live_slots(), 4096 - 512);

    let c = ChunkCoord([0, 0, 0]);
    for m in 2..=256u16 {
        let k = i32::from(m);
        // Air above the floor, inside the `Full` bricks of the bottom layer.
        fill_voxel(&world, [k % 64, (k / 64) * 8 + 1, 5], m);
    }
    assert_eq!(full_bricks(&world, &[c]), 64);
    world.store().write_lock().insert(late, held);
    let st = gpu.sync(&world, &queue).expect("x = 0 uploads");
    assert_eq!((st.chunks, st.deferred_chunks), (1, 1));
    assert!(gpu.is_wide(c));
    assert!(!gpu.is_mirrored(late));
    assert_eq!(gpu.pending_chunks(), 1);
    assert_eq!(gpu.live_slots(), 8 * 64 + 6 * 512 + 64);
    let rb = Readback::new(&device, &queue, &gpu);
    rb.assert_chunk(&gpu, &world, c);
    let chunk = world.store().get(late).unwrap();
    assert!(
        chunk.read().unwrap().any_dirty(),
        "the deferred chunk keeps its dirty bits"
    );
}

/// `ManyAndTwo`'s 300-material chunk at x = 0, `Checker` elsewhere.
struct WideThenChecker;

impl WorldSource for WideThenChecker {
    fn chunk(&self, coord: ChunkCoord, out: &mut Chunk) {
        if coord.0[0] == 0 {
            ManyAndTwo.chunk(coord, out);
        } else {
            Checker.chunk(coord, out);
        }
    }
}

/// Free pairs serve narrow bricks. One plain page: wide x = 0 (512 bricks, 1024 slots) and
/// x = 1..6 `Checker` (2560) leave the cursor 512 short of the page end. One sync demotes
/// x = 0 to uniform mips, which returns its 512 pairs to the free pair list, and adds
/// `Checker` x = 6 and x = 7 (1024 narrow slots). x = 6 takes the cursor's 512 and x = 7
/// splits 256 pairs; nothing is deferred, and no chunk fails part-way.
#[test]
fn free_pairs_serve_narrow_bricks() {
    let Some((device, queue)) = device() else {
        return;
    };
    let extent = ChunkExtent::new([0, 0, 0], [8, 1, 1]);
    let world = World::generate(extent, &WideThenChecker, &numbered_table(300));
    let (c0, c6, c7) = (
        ChunkCoord([0, 0, 0]),
        ChunkCoord([6, 0, 0]),
        ChunkCoord([7, 0, 0]),
    );
    let held6 = world.store().remove(c6).unwrap();
    let held7 = world.store().remove(c7).unwrap();
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            budget_bytes: budget_for(4096, 8),
            ..MirrorConfig::new(Window::Fixed(extent))
        },
    );
    gpu.set_focus(VoxelCoord([1, 1, 1]));
    gpu.sync(&world, &queue).unwrap();
    assert!(gpu.is_wide(c0));
    assert_eq!(gpu.live_slots(), 1024 + 5 * 512);

    let checker = World::generate(extent, &Checker, &numbered_table(2));
    let mips = Arc::new(ChunkMips::build(
        &checker.store().get(c0).unwrap().read().unwrap(),
    ));
    assert_eq!(mips.full_bricks(), 0);
    world.store().remove(c0);
    world.insert_lod(c0, mips);
    world.store().write_lock().insert(c6, held6);
    world.store().write_lock().insert(c7, held7);
    let st = gpu.sync(&world, &queue).expect("every chunk fits whole");
    assert_eq!(
        (st.chunks, st.deferred_chunks, st.pending_chunks),
        (3, 0, 0)
    );
    assert_eq!(gpu.live_slots(), 7 * 512);
    assert_eq!(gpu.pages_issued(), 1);
    let rb = Readback::new(&device, &queue, &gpu);
    rb.assert_lod(&gpu, &world, c0);
    for x in 1..8 {
        rb.assert_chunk(&gpu, &world, ChunkCoord([x, 0, 0]));
    }
}

/// An already-wide chunk's new bricks need pairs, not singles. One plain page: `Floor` x = 0
/// made wide by 255 materials (64 bricks, 128 slots) and x = 1..8 `Checker` (3584) leave a
/// 384-slot cursor. One sync demotes x = 1 (focus) to uniform mips, freeing 512 singles, and
/// fills one voxel in 250 air bricks of x = 0, which then needs 250 pairs (500 slots). Pair
/// room is the cursor's 384: x = 0 is deferred whole, its record unchanged, and the sync is
/// `Ok` with the demotion written.
#[test]
fn an_already_wide_chunk_is_not_counted_free_singles() {
    let Some((device, queue)) = device() else {
        return;
    };
    let extent = ChunkExtent::new([0, 0, 0], [8, 1, 1]);
    let world = World::generate(extent, &FloorAndChecker, &numbered_table(300));
    let (c0, c1) = (ChunkCoord([0, 0, 0]), ChunkCoord([1, 0, 0]));
    for m in 2..=256u16 {
        let k = i32::from(m);
        fill_voxel(&world, [k % 64, (k / 64) * 8 + 1, 5], m);
    }
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            budget_bytes: budget_for(4096, 8),
            ..MirrorConfig::new(Window::Fixed(extent))
        },
    );
    gpu.set_focus(VoxelCoord([65, 1, 1]));
    gpu.sync(&world, &queue).unwrap();
    assert!(gpu.is_wide(c0));
    assert_eq!(gpu.live_slots(), 128 + 7 * 512);
    let snapshot = |gpu: &GpuWorld| {
        let rb = Readback::new(&device, &queue, gpu);
        let rec = gpu.record_index(c0).unwrap();
        let voxels: Vec<Vec<u16>> = (0..CHUNK_BRICKS)
            .filter_map(|i| gpu.entry(c0, i).filter(|e| e & FULL_BIT != 0))
            .map(|e| rb.voxels(rec, e))
            .collect();
        (record_and_entry(&rb, gpu, c0), voxels)
    };
    let before = snapshot(&gpu);

    let checker = World::generate(extent, &Checker, &numbered_table(2));
    let mips = Arc::new(ChunkMips::build(
        &checker.store().get(c1).unwrap().read().unwrap(),
    ));
    world.store().remove(c1);
    world.insert_lod(c1, mips);
    let mut n = 0;
    'fill: for bz in 1..8 {
        for by in 0..8 {
            for bx in 0..8 {
                if n == 250 {
                    break 'fill;
                }
                fill_voxel(&world, [bx * 8, by * 8, bz * 8], 1);
                n += 1;
            }
        }
    }
    assert_eq!(full_bricks(&world, &[c0]), 64 + 250);
    let st = gpu.sync(&world, &queue).expect("the demotion uploads");
    assert_eq!(
        (st.chunks, st.deferred_chunks, st.pending_chunks),
        (1, 1, 1)
    );
    assert_eq!(gpu.live_slots(), 128 + 6 * 512);
    assert_eq!(snapshot(&gpu), before);
    let chunk = world.store().get(c0).unwrap();
    assert!(
        chunk.read().unwrap().any_dirty(),
        "the deferred chunk keeps its dirty bits"
    );
    let rb = Readback::new(&device, &queue, &gpu);
    rb.assert_lod(&gpu, &world, c1);
}
