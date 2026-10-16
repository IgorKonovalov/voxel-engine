//! The GPU mirror against a closure-source world: what `sync` uploads, what it frees, and
//! that the buffers hold what the CPU accounting says.

use std::collections::HashSet;
use std::sync::Arc;

use vxe_render::gpu_world::{
    ABSENT, FULL_BIT, POOL_PAGE_SLOTS, RECORD_BYTES, RECORD_WORDS, SLOT_BYTES, SLOT_MASK, WIDE_BIT,
};
use vxe_render::{GpuWorld, MARCH_WGSL, MirrorConfig, Window, readback, wgpu};
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
    for (slot, &e) in words[..512].iter().enumerate() {
        assert_eq!(Some(e), gpu.entry(ChunkCoord([1, 0, 0]), slot));
    }
    assert_eq!(gpu.palette(ChunkCoord([1, 0, 0])), Some(&[0, 1][..]));
    assert_eq!(words[512], 1 << 16, "palette entries 0 (air) and 1");
    let e = words[0];
    assert_eq!(e & (FULL_BIT | WIDE_BIT), FULL_BIT);
    let pool = readback::read_buffer(
        &device,
        &queue,
        gpu.pool(),
        u64::from(e & SLOT_MASK) * SLOT_BYTES,
        SLOT_BYTES,
    );
    for (i, &index) in pool.iter().enumerate() {
        let z = (i / 64) as i32;
        assert_eq!(index, u8::from(z < 3), "voxel {i}");
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
        u64::from(e & SLOT_MASK) * 8,
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
    let bytes = readback::read_buffer(&device, &queue, gpu.records(), rec * RECORD_BYTES, 4);
    assert_eq!(bytemuck::cast_slice::<u8, u32>(&bytes), [0]);
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
        Self {
            table: bytemuck::cast_slice(&all(gpu.table())).to_vec(),
            records: bytemuck::cast_slice(&all(gpu.records())).to_vec(),
            pool: all(gpu.pool()),
            masks: bytemuck::cast_slice(&all(gpu.masks())).to_vec(),
        }
    }
    /// The voxels of the `Full` brick in record entry `e` of record `rec`, decoded as the
    /// shader decodes them: through the palette when narrow, as `u16` pairs when wide.
    fn voxels(&self, rec: u32, e: u32) -> Vec<u16> {
        let s = (e & SLOT_MASK) as usize * SLOT_BYTES as usize;
        if e & WIDE_BIT != 0 {
            bytemuck::cast_slice(&self.pool[s..s + 1024]).to_vec()
        } else {
            let palette = &self.records[rec as usize * RECORD_WORDS + 512..][..128];
            self.pool[s..s + 512]
                .iter()
                .map(|&i| (palette[usize::from(i) / 2] >> (16 * (i & 1))) as u16)
                .collect()
        }
    }

    /// Asserts that the table, records and pool hold `coord`'s CPU bricks; returns the slots
    /// its `Full` bricks occupy.
    fn assert_chunk(&self, gpu: &GpuWorld, world: &World, coord: ChunkCoord) -> Vec<u32> {
        let rec = self.table[gpu.table_index(coord) as usize];
        assert_ne!(rec, ABSENT, "{coord:?} has no table entry");
        let chunk = world.store().get(coord).unwrap();
        let chunk = chunk.read().unwrap();
        let mut slots = Vec::new();
        for (slot, brick) in chunk.bricks().iter().enumerate() {
            let e = self.records[rec as usize * RECORD_WORDS + slot];
            match brick {
                Brick::Uniform(m) => assert_eq!(e, u32::from(*m), "{coord:?} brick {slot}"),
                Brick::Full(v) => {
                    assert_ne!(e & FULL_BIT, 0, "{coord:?} brick {slot}");
                    assert_eq!(self.voxels(rec, e), &v[..], "{coord:?} brick {slot}");
                    let s = e & SLOT_MASK;
                    let mask = brick.occupancy_mask();
                    assert_eq!(
                        self.masks[s as usize * 2..s as usize * 2 + 2],
                        [mask as u32, (mask >> 32) as u32]
                    );
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

/// The streamer's radius-2 scenario through a following window of side 7: after the move the
/// mirror holds the 42 resident chunks, each decoding to its CPU voxels, and the evicted
/// chunks' slots went to the free list.
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
        },
    );

    let focus = VoxelCoord([0, 0, 0]);
    s.update(&world, &[focus]);
    let ev = s.wait_idle(&world);
    gpu.apply_events(&ev);
    gpu.set_focus(focus);
    let st = gpu.sync(&world, &queue).unwrap();
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
    gpu.apply_events(&ev);
    // Eviction returns exactly the evicted chunks' slots to the free list.
    let free: HashSet<u32> = gpu.free_slots().iter().copied().collect();
    assert_eq!(free, evicted_slots);

    gpu.set_focus(focus);
    let st = gpu.sync(&world, &queue).unwrap();
    assert_eq!(st.pending_chunks, 0);
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
    // Every evicted slot is either reused by a new chunk or still free; none leaked.
    let free: HashSet<u32> = gpu.free_slots().iter().copied().collect();
    for s in &evicted_slots {
        assert!(live.contains(s) != free.contains(s), "slot {s}");
    }
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
    let mut gpu = GpuWorld::new(&device, world.extent());
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
    let mut gpu = GpuWorld::new(&device, world.extent());
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
