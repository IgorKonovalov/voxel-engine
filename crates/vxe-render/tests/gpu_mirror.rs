//! The GPU mirror against a closure-source world: what `sync` uploads, what it frees, and
//! that the buffers hold what the CPU accounting says.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use vxe_render::gpu_world::{
    ABSENT, BIN_COUNT, BINS, BITS_WORDS, CLASS_MASK, CLASS_SHIFT, CLASS_UNIFORM, CLASS_WIDE,
    EMPTY_BIT, ENTRY_WORD, FULL_BIT, HEADER_BAND_BIT, HEADER_BAND_PREFIX_WORD, HEADER_BAND_WORD,
    HEADER_TRANSLUCENT_WORD, LOD_BIT, LOD_LEVEL_ENTRY, PAGE_BYTES, PAGE_UNITS, PLACEHOLDER_BYTES,
    RECORD_BYTES, RECORD_MASK, RECORD_WORDS, SLOT_BINS, TRANSLUCENT_BIT, UNIT_BYTES, UNIT_MASK,
    UNIT_WORDS, band_bin_of, bin_of, budget_for, payload_units, slots_per_page,
};
use vxe_render::{
    ATROUS_WGSL, COMPOSITE_WGSL, FIELD_WGSL, GRADIENT_WGSL, GpuWorld, LIGHT_WGSL, MARCH_WGSL,
    MirrorConfig, ShardLayout, SyncPoint, TEMPORAL_WGSL, Window, readback, wgpu,
};
use vxe_world::packed::PackedClass;
use vxe_world::*;

/// The frame's shaders parse and validate headless, without a device: the hit pass alone and
/// each pass compiled after it.
#[test]
fn march_wgsl_validates() {
    for (name, source) in [
        ("march.wgsl", MARCH_WGSL.to_owned()),
        ("light.wgsl", format!("{MARCH_WGSL}\n{LIGHT_WGSL}")),
        ("composite.wgsl", format!("{MARCH_WGSL}\n{COMPOSITE_WGSL}")),
        ("temporal.wgsl", format!("{MARCH_WGSL}\n{TEMPORAL_WGSL}")),
        ("atrous.wgsl", format!("{MARCH_WGSL}\n{ATROUS_WGSL}")),
        (
            "gradient.wgsl",
            format!("{MARCH_WGSL}\n{LIGHT_WGSL}\n{GRADIENT_WGSL}"),
        ),
    ] {
        let module = wgpu::naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("{name} parses: {}", e.emit_to_string(&source)));
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::default(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{name} validates: {e:?}"));
    }
}

/// The field pass's shader parses and validates headless.
#[test]
fn field_wgsl_validates() {
    let module = wgpu::naga::front::wgsl::parse_str(FIELD_WGSL).expect("field.wgsl parses");
    wgpu::naga::valid::Validator::new(
        wgpu::naga::valid::ValidationFlags::all(),
        wgpu::naga::valid::Capabilities::default(),
    )
    .validate(&module)
    .expect("field.wgsl validates");
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
    let limits = vxe_render::limits_for(&adapter, 16 << 20).expect("the frame's limits");
    let desc = wgpu::DeviceDescriptor {
        required_limits: limits,
        ..Default::default()
    };
    Some(pollster::block_on(adapter.request_device(&desc)).expect("device"))
}

/// A sync outside a frame: records into an encoder of its own, submits it and waits for the
/// copies, so the belt's chunks come back before the next sync.
trait SyncNow {
    fn sync_now(
        &mut self,
        world: &World,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<vxe_render::SyncStats, vxe_render::MirrorError>;
}

impl SyncNow for GpuWorld {
    fn sync_now(
        &mut self,
        world: &World,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<vxe_render::SyncStats, vxe_render::MirrorError> {
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let r = self.sync(world, queue, &mut enc);
        queue.submit([enc.finish()]);
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("device poll after a sync");
        r
    }
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

/// The budget of `pages` whole pages and `records` records.
fn pages_budget(pages: u64, records: u64) -> u64 {
    budget_for(pages * u64::from(PAGE_UNITS), records)
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

/// Sixteen mask words as eight `u64` layers, low word first.
fn layers(w: &[u32]) -> [u64; 8] {
    std::array::from_fn(|z| u64::from(w[2 * z]) | u64::from(w[2 * z + 1]) << 32)
}

/// The class code of a `Full` entry.
fn class_of(e: u32) -> u32 {
    e >> CLASS_SHIFT & CLASS_MASK
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
    let bytes = readback::read_buffer(
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
    let slot = readback::read_buffer(
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
    /// The pool's words as far as the pages handed out reach, shards in order.
    pool: Vec<u32>,
}

impl Readback {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, gpu: &GpuWorld) -> Self {
        let all = |b: &wgpu::Buffer| readback::read_buffer(device, queue, b, 0, b.size());
        let units = u64::from(gpu.pool_units());
        let ups = u64::from(gpu.shard_layout().units_per_shard);
        let shard = |k: usize| {
            let n = units.saturating_sub(k as u64 * ups).min(ups);
            readback::read_buffer(device, queue, gpu.pool_shard(k), 0, n * UNIT_BYTES)
        };
        let pool: Vec<u8> = (0..gpu.shards_in_use()).flat_map(shard).collect();
        Self {
            table: bytemuck::cast_slice(&all(gpu.table())).to_vec(),
            records: bytemuck::cast_slice(&all(gpu.records())).to_vec(),
            pool: bytemuck::pod_collect_to_vec(&pool),
        }
    }

    /// Word `w` of the slot whose first unit is `u`.
    fn word(&self, u: u32, w: usize) -> u32 {
        self.pool[u as usize * UNIT_WORDS + w]
    }

    /// The band slot of brick `slot` of `c` (its band mask and count units, as its bin holds
    /// them), `None` for a brick without one.
    fn band(&self, gpu: &GpuWorld, c: ChunkCoord, slot: usize) -> Option<Vec<u32>> {
        let e = gpu.entry(c, slot).filter(|e| e & FULL_BIT != 0)?;
        let h = self.word(e & UNIT_MASK, UNIT_WORDS + HEADER_BAND_WORD);
        if h & HEADER_BAND_BIT == 0 {
            return None;
        }
        let u = h & UNIT_MASK;
        let units = SLOT_BINS[gpu.bin_of_unit(u).expect("a live band slot")] as usize;
        Some((0..units * UNIT_WORDS).map(|w| self.word(u, w)).collect())
    }

    /// The `Full` brick at entry `e` decoded as the shader decodes it: a Wide brick's ids;
    /// else a voxel set in the opaque mask, or in the translucent unit of a brick flagged
    /// translucent, reads its palette entry by the planes' bits.
    fn voxels(&self, e: u32) -> Vec<u16> {
        let u = e & UNIT_MASK;
        let code = class_of(e);
        let bit = |w: usize, i: usize| self.word(u, w + i / 32) >> (i % 32) & 1;
        let planes = payload_units(code) as usize;
        (0..BRICK_VOXELS)
            .map(|i| {
                if code == CLASS_WIDE {
                    return (self.word(u, 32 + i / 2) >> (16 * (i & 1))) as u16;
                }
                let translucent = e & TRANSLUCENT_BIT != 0 && bit(32 + 16 * planes, i) == 1;
                if bit(0, i) == 0 && !translucent {
                    return AIR;
                }
                let idx = (0..planes).fold(0, |a, k| a | (bit(32 + 16 * k, i) as usize) << k);
                (self.word(u, 16 + idx / 2) >> (16 * (idx & 1))) as u16
            })
            .collect()
    }

    /// Asserts the `Full` brick at entry `e` holds `brick`: its decoded voxels, its opaque
    /// mask (the solid voxels less the translucent ones), its header's translucent words, its
    /// translucent flag and its class.
    fn assert_full(&self, e: u32, brick: &Brick, table: &MaterialTable, what: &str) {
        let mut v = [AIR; BRICK_VOXELS];
        brick.decode(&mut v);
        assert_ne!(e & FULL_BIT, 0, "{what}");
        assert_eq!(self.voxels(e), &v[..], "{what}");
        let (mut opaque, mut glass) = ([0u64; 8], [0u32; 2]);
        for (i, &m) in v.iter().enumerate() {
            if m == AIR {
                continue;
            }
            if table.flags(m).contains(MaterialFlags::TRANSLUCENT) {
                let (x, y, z) = (i % 8, i / 8 % 8, i / 64);
                let block = x / 2 + 4 * (y / 2) + 16 * (z / 2);
                glass[block / 32] |= 1 << (block % 32);
            } else {
                opaque[i / 64] |= 1 << (i % 64);
            }
        }
        let u = e & UNIT_MASK;
        let mask: Vec<u32> = (0..16).map(|w| self.word(u, w)).collect();
        assert_eq!(layers(&mask), opaque, "{what} opaque mask");
        let t = UNIT_WORDS + HEADER_TRANSLUCENT_WORD;
        assert_eq!(
            [self.word(u, t), self.word(u, t + 1)],
            glass,
            "{what} translucent words"
        );
        assert_eq!(e & TRANSLUCENT_BIT != 0, glass != [0, 0], "{what} flag");
        let class = brick.class().expect("a Full brick has a class");
        assert_eq!(
            class_of(e),
            vxe_render::gpu_world::class_code(class),
            "{what} class"
        );
    }

    /// Asserts that the table, records and pool hold `coord`'s CPU bricks; returns the first
    /// units of the slots its `Full` bricks occupy.
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
        let mut units = Vec::new();
        for (slot, brick) in chunk.bricks().iter().enumerate() {
            let set = bits[slot / 32] >> (slot % 32) & 1 != 0;
            let e = self.records[rec as usize * RECORD_WORDS + ENTRY_WORD + slot];
            // With the field on, a uniform band brick has a slot of class CLASS_UNIFORM, its
            // header naming its band slot; its bit is set even for air.
            assert_eq!(
                set,
                *brick != Brick::Uniform(AIR) || e & FULL_BIT != 0,
                "{coord:?} brick {slot} bit"
            );
            match brick {
                Brick::Uniform(m) if e & FULL_BIT != 0 => {
                    let u = e & UNIT_MASK;
                    assert_eq!(
                        e >> CLASS_SHIFT & CLASS_MASK,
                        CLASS_UNIFORM,
                        "{coord:?} brick {slot}"
                    );
                    assert_eq!(
                        self.word(u, UNIT_WORDS) & 0xFFFF,
                        u32::from(*m),
                        "{coord:?} brick {slot}: the palette's entry 0"
                    );
                    assert_ne!(
                        self.word(u, UNIT_WORDS + HEADER_BAND_WORD) & HEADER_BAND_BIT,
                        0,
                        "{coord:?} brick {slot}: a uniform brick's slot names its band slot"
                    );
                    units.push(u);
                }
                Brick::Uniform(m) => assert_eq!(
                    e & !TRANSLUCENT_BIT,
                    u32::from(*m),
                    "{coord:?} brick {slot}"
                ),
                Brick::Full(_) => {
                    let what = format!("{coord:?} brick {slot}");
                    self.assert_full(e, brick, world.materials(), &what);
                    units.push(e & UNIT_MASK);
                }
            }
            if e & FULL_BIT != 0 {
                let h = self.word(e & UNIT_MASK, UNIT_WORDS + HEADER_BAND_WORD);
                if h & HEADER_BAND_BIT != 0 {
                    units.push(h & UNIT_MASK);
                }
            }
        }
        units
    }

    /// Asserts that `coord`'s table entry is a LOD record whose 73 entries decode to the
    /// world's mips for it; returns the first units of its `Full` mip-bricks' slots.
    fn assert_lod(&self, gpu: &GpuWorld, world: &World, coord: ChunkCoord) -> Vec<u32> {
        let t = self.table[gpu.table_index(coord) as usize];
        assert_ne!(t, ABSENT, "{coord:?} has no table entry");
        assert_eq!(t & LOD_BIT, LOD_BIT, "{coord:?} is not a LOD record");
        let rec = t & RECORD_MASK;
        let mips = world.lod(coord).expect("LOD-only in the world");
        let mut units = Vec::new();
        for n in 1..=MIP_LEVELS {
            for (j, brick) in mips.level(n).iter().enumerate() {
                let e = self.records
                    [rec as usize * RECORD_WORDS + ENTRY_WORD + LOD_LEVEL_ENTRY[n - 1] + j];
                match brick {
                    Brick::Uniform(m) => assert_eq!(e, u32::from(*m), "{coord:?} L{n} {j}"),
                    Brick::Full(_) => {
                        let what = format!("{coord:?} L{n} {j}");
                        self.assert_full(e, brick, world.materials(), &what);
                        units.push(e & UNIT_MASK);
                    }
                }
            }
        }
        units
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

/// The two terms of the pressure: unit occupancy and record occupancy.
fn pressure_terms(gpu: &GpuWorld) -> (f64, f64) {
    (
        gpu.live_units() as f64 / f64::from(gpu.reserved_units()),
        gpu.mirrored_chunks() as f64 / f64::from(gpu.reserved_records()),
    )
}

/// `pressure()` is the larger of its two terms within 1e-6, and between 0 and 1 here.
fn assert_pressure(gpu: &GpuWorld) {
    let (units, records) = pressure_terms(gpu);
    let want = units.max(records);
    let got = f64::from(gpu.pressure());
    assert!((got - want).abs() < 1e-6, "pressure {got} against {want}");
    assert!(want > 0.0 && want < 1.0, "{want}");
}

/// Every unit of the pages handed out is live, spare, a free page's, or the tail of a page
/// past its bin's last whole slot: none leaked.
fn assert_units_accounted(gpu: &GpuWorld) {
    let tails: u64 = gpu
        .bin_stats()
        .iter()
        .map(|b| u64::from(b.pages) * u64::from(PAGE_UNITS % b.units))
        .sum();
    assert_eq!(
        gpu.live_units()
            + gpu.spare_units()
            + tails
            + gpu.free_pages().len() as u64 * u64::from(PAGE_UNITS),
        u64::from(gpu.pool_units())
    );
}

/// Whether the slot at unit `u` is free: on its bin's free list, or in a free page.
fn is_free(gpu: &GpuWorld, u: u32) -> bool {
    let p = u / PAGE_UNITS;
    if gpu.free_pages().contains(&p) {
        return true;
    }
    gpu.bin_of_unit(u)
        .is_some_and(|k| gpu.free_slots(k).contains(&u))
}

/// The buffers a mirror reserved: its four shards and records.
fn reserved_buffers(gpu: &GpuWorld) -> Vec<wgpu::Buffer> {
    (0..4)
        .map(|k| gpu.pool_shard(k).clone())
        .chain([gpu.records().clone()])
        .collect()
}

/// The streamer's radius-2 scenario through a following window of side 7: after the move the
/// mirror holds the 42 resident chunks, each decoding to its CPU voxels, and the evicted
/// chunks' slots went to the free lists. The 64 MiB reservation is made once: its bytes are
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
    let mut gpu = GpuWorld::with_config(&device, small(Window::Follow { side: 7 }));
    // 343 records, then whole pages of what is left past the placeholders: 253 pages on one
    // shard (lavapipe's 2 097 152 units), the other three shards one-unit placeholders.
    assert_eq!(gpu.reserved_records(), 343);
    let pages = (SMALL_BUDGET - 343 * RECORD_BYTES - PLACEHOLDER_BYTES) / PAGE_BYTES;
    assert_eq!(pages, 253);
    assert_eq!(u64::from(gpu.reserved_units()), pages * 4096);
    let reserved = 343 * RECORD_BYTES + pages * PAGE_BYTES + 3 * UNIT_BYTES;
    assert_eq!(gpu.allocated_bytes(), reserved);
    assert!(SMALL_BUDGET - reserved < PAGE_BYTES + PLACEHOLDER_BYTES);
    let buffers = reserved_buffers(&gpu);

    let focus = VoxelCoord([0, 0, 0]);
    s.update(&world, &[focus]);
    let ev = s.wait_idle(&world);
    gpu.apply_events(&ev);
    gpu.set_focus(focus);
    let st = gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(gpu.allocated_bytes(), reserved);
    assert_pressure(&gpu);
    assert_eq!(st.chunks, 33);
    assert_eq!(gpu.mirrored_chunks(), 33);
    let first = world.store().coords();
    let rb = Readback::new(&device, &queue, &gpu);
    let mut old_units = HashSet::new();
    for &c in &first {
        old_units.extend(rb.assert_chunk(&gpu, &world, c));
    }

    let focus = VoxelCoord([192, 0, 0]);
    s.update(&world, &[focus]);
    let ev = s.wait_idle(&world);
    assert_eq!(ev.evicted.len(), 22);
    let evicted_units: HashSet<u32> = ev
        .evicted
        .iter()
        .flat_map(|&c| (0..512).map(move |slot| (c, slot)))
        .filter_map(|(c, slot)| gpu.entry(c, slot))
        .filter(|e| e & FULL_BIT != 0)
        .map(|e| e & UNIT_MASK)
        .collect();
    assert!(!evicted_units.is_empty());
    let (live_before, allocated_before) = (gpu.mirror_bytes(), gpu.allocated_bytes());
    gpu.apply_events(&ev);
    // Eviction lowers the live accounting; the reservation stays.
    assert!(gpu.mirror_bytes() < live_before);
    assert_eq!(gpu.allocated_bytes(), allocated_before);
    assert_eq!(gpu.allocated_bytes(), reserved);
    assert_pressure(&gpu);
    // Eviction returns exactly the evicted chunks' slots: to their bins' free lists, or with
    // a page that emptied, to the free pages.
    for &u in &evicted_units {
        assert!(is_free(&gpu, u), "unit {u}");
    }
    assert_units_accounted(&gpu);

    gpu.set_focus(focus);
    let st = gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(st.pending_chunks, 0);
    // The window moved by three chunks; nothing was created or resized.
    assert_eq!(gpu.allocated_bytes(), reserved);
    assert_eq!(reserved_buffers(&gpu), buffers);
    assert_pressure(&gpu);
    let resident = world.store().coords();
    assert_eq!(resident.len(), 42);
    assert_eq!(gpu.mirrored_chunks(), 42);
    for &c in &ev.evicted {
        assert!(!gpu.is_mirrored(c));
    }
    let rb = Readback::new(&device, &queue, &gpu);
    let mut live = HashSet::new();
    let mut units = 0u64;
    for &c in &resident {
        for u in rb.assert_chunk(&gpu, &world, c) {
            assert!(live.insert(u), "unit {u} held twice");
            assert!(!is_free(&gpu, u), "unit {u} is live and free");
            units += u64::from(BINS[gpu.bin_of_unit(u).unwrap()]);
        }
    }
    assert_eq!(gpu.live_bricks(), full_bricks(&world, &resident));
    assert_eq!(live.len() as u32, gpu.live_bricks());
    assert_eq!(units, gpu.live_units());
    assert_units_accounted(&gpu);
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
    assert!(old_units.len() > 100);
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

/// A first sync past the reservation uploads the `uploaded` nearest chunks that fit and
/// defers the other `deferred`, which stay pending with their dirty bits; the next sync,
/// with nothing freed, is empty: it fails with `err` and writes nothing.
fn partial_then_empty(
    gpu: &mut GpuWorld,
    world: &World,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    uploaded: usize,
    deferred: u64,
    err: vxe_render::MirrorError,
) {
    let st = gpu.sync_now(world, device, queue).unwrap();
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
    let (bytes, live) = (gpu.bytes_written(), gpu.live_units());
    assert_eq!(gpu.sync_now(world, device, queue).unwrap_err(), err);
    assert_eq!(gpu.bytes_written(), bytes);
    assert_eq!(gpu.mirrored_chunks(), uploaded);
    assert_eq!(gpu.live_units(), live);
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

/// Every brick of every chunk is `Full`: two materials in a checkerboard, one plane, three
/// units in the 3-unit bin (1365 slots a page).
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

/// The same 36 chunks under a budget of four pages and their records, on uncapped shards:
/// the budget is the limit; the first sync uploads 10 chunks, and the next, with nothing
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
    let budget = pages_budget(4, 36);
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            budget_bytes: budget,
            ..MirrorConfig::new(Window::Fixed(world.extent()))
        },
    );
    assert!(gpu.shard_layout().capacity() > 4 * 4096);
    assert_eq!(gpu.reserved_units(), 4 * 4096);
    assert_eq!(gpu.reserved_records(), 36);
    assert!(gpu.allocated_bytes() <= budget);
    assert_eq!(gpu.allocated_bytes(), gpu.shard_layout().reserved_bytes());
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

    // Exactly the reservation: 8 chunks of 512 `Full` bricks take 4096 slots, three pages of
    // 1365 and one slot of a fourth.
    let fits = World::generate(
        ChunkExtent::new([0, 0, 0], [4, 2, 1]),
        &Checker,
        &numbered_table(2),
    );
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            budget_bytes: pages_budget(4, 8),
            ..MirrorConfig::new(Window::Fixed(fits.extent()))
        },
    );
    let st = gpu.sync_now(&fits, &device, &queue).unwrap();
    assert_eq!(st.bricks, 4096);
    assert_eq!(gpu.pool_units(), gpu.reserved_units());
    assert_eq!(gpu.bin_stats()[bin_of(3)].spare, 4 * 1365 - 4096);
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
        gpu.sync_now(&world, &device, &queue).unwrap();
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
    let full_units: HashSet<u32> = (0..512)
        .filter_map(|slot| gpu.entry(demoted, slot))
        .filter(|e| e & FULL_BIT != 0)
        .map(|e| e & UNIT_MASK)
        .collect();
    assert!(!full_units.is_empty());

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
    assert_eq!(live.len() as u32, gpu.live_bricks());
    // Each slot the demoted chunk's full record held is free (alone or in a free page), or
    // reused by a live record; every unit handed out is accounted for.
    for &u in &full_units {
        assert!(is_free(&gpu, u) != live.contains(&u), "unit {u}");
    }
    assert!(full_units.iter().any(|&u| is_free(&gpu, u)));
    assert_units_accounted(&gpu);
    let lod_full = world.lod(demoted).unwrap().full_bricks();
    assert!(
        lod_full < full_units.len(),
        "{lod_full} mip-bricks for {} bricks",
        full_units.len()
    );
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
    readback::read_buffer(
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

/// Pressure on units: a fixed window whose `Full` bricks fill more than 0.9 of a six-page
/// reservation, with records far from full.
#[test]
fn pressure_follows_the_units_when_they_run_out() {
    let Some((device, queue)) = device() else {
        return;
    };
    // 15 chunks of 512 `Full` bricks of 3 units: 23 040 of 24 576 units.
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [5, 3, 1]),
        &Checker,
        &numbered_table(2),
    );
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            budget_bytes: pages_budget(6, 40),
            record_chunks: Some(40),
            ..MirrorConfig::new(Window::Fixed(world.extent()))
        },
    );
    assert_eq!(
        (gpu.reserved_units(), gpu.reserved_records()),
        (6 * 4096, 40)
    );
    gpu.sync_now(&world, &device, &queue).unwrap();
    let (units, records) = pressure_terms(&gpu);
    assert_eq!(units, 23_040.0 / 24_576.0);
    assert!(units > 0.9 && records < 0.5, "{units} {records}");
    assert_pressure(&gpu);
    assert!((f64::from(gpu.pressure()) - units).abs() < 1e-6);
}

/// Pressure on records: uniform chunks take no slot, and 37 of 40 records read past 0.9
/// with units at 0. A mirror given `record_chunks` reserves exactly that many, and its
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
    let budget = pages_budget(1, 40);
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
    gpu.sync_now(&world, &device, &queue).unwrap();
    let (units, records) = pressure_terms(&gpu);
    assert_eq!(units, 0.0);
    assert_eq!(records, 37.0 / 40.0);
    assert!(records > 0.9);
    assert!((f64::from(gpu.pressure()) - records).abs() < 1e-6);
}

/// A chunk as the store holds it.
type SharedChunk = Arc<std::sync::RwLock<Chunk>>;

/// A fixed window over chunks x = 0..12 (y = z = 0) of `Checker`, emptied of every chunk; the
/// chunks are handed back to put in the store, and `Mixed` mips at the same coordinates.
fn tier_world() -> (World, Vec<SharedChunk>, Vec<Arc<ChunkMips>>) {
    let extent = ChunkExtent::new([0, 0, 0], [12, 1, 1]);
    let world = World::generate(extent, &Checker, &numbered_table(5));
    let mixed = World::generate(extent, &Mixed, &mixed_table());
    let mut chunks = Vec::new();
    let mut mips = Vec::new();
    for x in 0..12 {
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

/// Inserts chunk `x` of the tier world, full.
fn to_full(world: &World, x: i32, chunks: &[SharedChunk]) {
    world
        .store()
        .write_lock()
        .insert(ChunkCoord([x, 0, 0]), chunks[x as usize].clone());
}

/// A demotion that frees its pages in the same sync fits a full pool, with the field's band
/// slots: three `Checker` chunks take exactly the reserved pages (their bricks in the 3-unit
/// bin, their band bricks' band slots in the band bins); one sync demotes them to
/// their own mips (a checkerboard's mip-bricks are uniform, so they take no slot) and adds a
/// farther LOD-only chunk whose `Mixed` mips need 2- and 3-unit slots. The demotions empty
/// the pages first, so those slots come from freed pages. A pool check that credited no tier
/// change's frees would ask for a page past the reservation and fail the sync with
/// `PoolTooLarge`.
#[test]
fn a_demotion_that_empties_pages_fits_a_full_pool() {
    let Some((device, queue)) = device() else {
        return;
    };
    let (world, chunks, mixed) = tier_world();
    for x in 0..3 {
        to_full(&world, x, &chunks);
    }
    let pages = pages_taken(&device, &queue, &world, true);
    let mut gpu = GpuWorld::with_field_surface(
        &device,
        MirrorConfig {
            budget_bytes: pages_budget(u64::from(pages), 12),
            ..MirrorConfig::new(Window::Fixed(world.extent()))
        },
    );
    gpu.set_focus(VoxelCoord([1, 1, 1]));
    gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(gpu.reserved_units(), pages * 4096);
    // The bricks' slots and a band slot for each band brick (the bricks on the world's faces).
    assert_eq!(gpu.live_bricks(), 3 * 512 + gpu.band_bricks() as u32);
    assert_eq!(gpu.pages_issued(), pages);
    assert!(pages >= 4, "{pages} pages over four bins");

    for x in 0..3 {
        let own = Arc::new(ChunkMips::build(&chunks[x as usize].read().unwrap()));
        assert_eq!(own.full_bricks(), 0, "a checkerboard's mips are uniform");
        to_lod(&world, x, &own);
    }
    world.insert_lod(ChunkCoord([3, 0, 0]), mixed[3].clone());
    let need = mixed[3].full_bricks() as u32;
    assert!(need > 0);
    let st = gpu
        .sync_now(&world, &device, &queue)
        .expect("the demotions free the pages first");
    assert_eq!(st.chunks, 4);
    assert_eq!(gpu.live_bricks(), need);
    assert_eq!(gpu.pages_issued(), pages);
    let rb = Readback::new(&device, &queue, &gpu);
    for x in 0..4 {
        rb.assert_lod(&gpu, &world, ChunkCoord([x, 0, 0]));
    }
}

/// The same without band slots: eight `Checker` chunks fill a four-page reservation of
/// 3-unit slots; demoting them and adding a farther LOD-only chunk fits, its mip-bricks taking
/// what the demotions free. A pool check that credited no tier change's frees would fail it
/// with `PoolTooLarge`.
#[test]
fn a_plain_demotion_fits_a_full_pool() {
    let Some((device, queue)) = device() else {
        return;
    };
    let (world, chunks, mixed) = tier_world();
    for x in 0..8 {
        to_full(&world, x, &chunks);
    }
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            budget_bytes: pages_budget(4, 12),
            ..MirrorConfig::new(Window::Fixed(world.extent()))
        },
    );
    gpu.set_focus(VoxelCoord([1, 1, 1]));
    gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(gpu.live_bricks(), 4096);
    assert_eq!(gpu.pages_issued(), 4);
    assert_eq!(gpu.reserved_units(), 4 * 4096);

    for x in 0..8 {
        let own = Arc::new(ChunkMips::build(&chunks[x as usize].read().unwrap()));
        to_lod(&world, x, &own);
    }
    world.insert_lod(ChunkCoord([8, 0, 0]), mixed[8].clone());
    let st = gpu
        .sync_now(&world, &device, &queue)
        .expect("the demotions free slots first");
    assert_eq!(st.chunks, 9);
    assert_eq!(gpu.live_bricks(), mixed[8].full_bricks() as u32);
    let rb = Readback::new(&device, &queue, &gpu);
    for x in 0..9 {
        rb.assert_lod(&gpu, &world, ChunkCoord([x, 0, 0]));
    }
}

/// A selection whose net need exceeds the reservation writes what fits net of its frees: ten
/// `Checker` chunks on four pages of 3-unit slots (340 spare), one of them demoted (freeing
/// 512) while two new full chunks need 1024. The demotion and the nearer new chunk upload, the
/// farther waits; the next sync, its nearest chunk alone not fitting and no demotion behind
/// it, is empty and writes nothing.
#[test]
fn a_selection_past_the_reservation_net_of_its_frees_still_fails_untouched() {
    let Some((device, queue)) = device() else {
        return;
    };
    let (world, chunks, _) = tier_world();
    for x in 0..10 {
        to_full(&world, x, &chunks);
    }
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            budget_bytes: pages_budget(4, 12),
            ..MirrorConfig::new(Window::Fixed(world.extent()))
        },
    );
    gpu.set_focus(VoxelCoord([1, 1, 1]));
    gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(gpu.bin_stats()[bin_of(3)].spare, 4 * 1365 - 5120);
    let own = Arc::new(ChunkMips::build(&chunks[0].read().unwrap()));
    to_lod(&world, 0, &own);
    for x in 10..12 {
        to_full(&world, x, &chunks);
    }
    let st = gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(
        (st.chunks, st.deferred_chunks, st.pending_chunks),
        (2, 1, 1)
    );
    assert!(gpu.is_lod(ChunkCoord([0, 0, 0])));
    assert!(gpu.is_mirrored(ChunkCoord([10, 0, 0])));
    assert!(!gpu.is_mirrored(ChunkCoord([11, 0, 0])));
    assert_eq!(gpu.live_bricks(), 5120);
    let (bytes, mirrored, live) = (gpu.bytes_written(), gpu.mirrored_chunks(), gpu.live_units());
    let err = gpu.sync_now(&world, &device, &queue).unwrap_err();
    assert_eq!(
        err,
        vxe_render::MirrorError::PoolTooLarge {
            needed: 5 * PAGE_BYTES,
            limit: 4 * PAGE_BYTES,
        }
    );
    assert_eq!(gpu.bytes_written(), bytes);
    assert_eq!(gpu.mirrored_chunks(), mirrored);
    assert_eq!(gpu.live_units(), live);
    assert_eq!(gpu.pending_chunks(), 1);
}

/// Record words and table entry the GPU holds for `coord`.
fn record_and_entry(rb: &Readback, gpu: &GpuWorld, coord: ChunkCoord) -> (Vec<u32>, u32) {
    let t = rb.table[gpu.table_index(coord) as usize];
    let rec = (t & RECORD_MASK) as usize;
    (rb.records[rec * RECORD_WORDS..][..RECORD_WORDS].to_vec(), t)
}

/// The pages a mirror of `world`, with the field's band slots when `field`, takes on its
/// first sync, focus at x = 0.
fn pages_taken(device: &wgpu::Device, queue: &wgpu::Queue, world: &World, field: bool) -> u32 {
    let cfg = small(Window::Fixed(world.extent()));
    let mut probe = if field {
        GpuWorld::with_field_surface(device, cfg)
    } else {
        GpuWorld::with_config(device, cfg)
    };
    probe.set_focus(VoxelCoord([1, 1, 1]));
    probe.sync_now(world, device, queue).unwrap();
    probe.pages_issued()
}

/// Five materials in turn, `1 + (x + 2 y + 3 z) % 5`: every brick four planes, 6 units, 682
/// slots a page.
struct Five;

impl WorldSource for Five {
    fn chunk(&self, _: ChunkCoord, out: &mut Chunk) {
        for slot in 0..CHUNK_BRICKS {
            out.set_brick(
                slot,
                Brick::from_fn(|l| 1 + ((l[0] + 2 * l[1] + 3 * l[2]) % 5) as u16),
            );
        }
    }
}

/// One page short: eight LOD-only chunks (their `Mixed` mips in 2- and 3-unit pages) are
/// promoted, nearest first, to `Five` chunks onto a reservation of those pages and four more.
/// Five chunks' 2560 bricks of 6 units take the four pages (682 a page); the sixth would need a
/// fifth. The sync returns `Ok`: the five nearest read back to their voxels, the other three
/// are deferred and pending, and nothing of theirs was written (their LOD records and table
/// entries read back as before, still decoding to their mips; their chunks keep their dirty
/// bits). After three demotions free their slots, the next sync uploads them.
#[test]
fn one_page_short_uploads_the_nearest_and_defers_the_rest_untouched() {
    let Some((device, queue)) = device() else {
        return;
    };
    let (world, checker, mixed) = tier_world();
    let five = World::generate(world.extent(), &Five, &numbered_table(5));
    let chunks: Vec<SharedChunk> = (0..12)
        .map(|x| five.store().remove(ChunkCoord([x, 0, 0])).unwrap())
        .collect();
    for x in 0..8 {
        world.insert_lod(ChunkCoord([x, 0, 0]), mixed[x as usize].clone());
    }
    let mip_pages = pages_taken(&device, &queue, &world, false);
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            budget_bytes: pages_budget(u64::from(mip_pages) + 4, 12),
            ..MirrorConfig::new(Window::Fixed(world.extent()))
        },
    );
    gpu.set_focus(VoxelCoord([1, 1, 1]));
    gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(gpu.pages_issued(), mip_pages);
    let mips_before = gpu.live_bricks();
    let rb = Readback::new(&device, &queue, &gpu);
    let before: Vec<_> = (5..8)
        .map(|x| record_and_entry(&rb, &gpu, ChunkCoord([x, 0, 0])))
        .collect();

    for x in 0..8 {
        world.remove_lod(ChunkCoord([x, 0, 0]));
        to_full(&world, x, &chunks);
    }
    let st = gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(st.chunks, 5);
    assert_eq!(st.deferred_chunks, 3);
    assert_eq!(st.pending_chunks, 3);
    assert_eq!(gpu.bin_stats()[bin_of(6)].live, 5 * 512);
    let freed: u32 = (0..5).map(|x| mixed[x].full_bricks() as u32).sum();
    assert_eq!(gpu.live_bricks(), mips_before - freed + 5 * 512);
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

    // Demote the three nearest (to a checkerboard's uniform mips): their slots free.
    for x in 0..3 {
        let own = Arc::new(ChunkMips::build(&checker[x as usize].read().unwrap()));
        to_lod(&world, x, &own);
    }
    let st = gpu.sync_now(&world, &device, &queue).unwrap();
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

/// `Floor` at x = 0 (64 `Full` bricks of 2 units), `Checker` elsewhere (512 of 3).
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

/// A dug chunk goes first: on a four-page pool holding a `Floor` chunk at the focus (a page
/// of 2-unit slots) and four far `Checker` chunks (2048 3-unit slots, two pages, 682 spare),
/// five nearer chunks are promoted from LOD-only (their checkerboard mips take no slot) while
/// a fill at the focus gains `Full` bricks. The dug chunk uploads and reads back to its
/// edited voxels; the promotions that fit (682 spare and a fourth page of 1365: three of 512)
/// follow nearest first, and only chunks farther than every uploaded one are deferred.
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
            budget_bytes: pages_budget(4, 10),
            ..MirrorConfig::new(Window::Fixed(extent))
        },
    );
    gpu.set_focus(VoxelCoord([1, 1, 1]));
    gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(gpu.live_bricks(), 64 + 4 * 512);
    assert_eq!(gpu.pages_issued(), 3);

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
    let st = gpu.sync_now(&world, &device, &queue).unwrap();
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

/// A demotion is not blocked: the nearest selected chunk, a promotion of 512 bricks, does not
/// fit a two-page pool with 170 spare 3-unit slots; two farther demotions behind it do (they
/// need nothing), upload and read back to their mips, and free 1024 slots, so the promotion
/// uploads in the next sync.
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
        to_full(&world, x, &chunks);
    }
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            budget_bytes: pages_budget(2, 12),
            ..MirrorConfig::new(Window::Fixed(world.extent()))
        },
    );
    gpu.set_focus(VoxelCoord([1, 1, 1]));
    gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(gpu.live_bricks(), 5 * 512);
    assert_eq!(gpu.pages_issued(), 2);
    assert_eq!(gpu.bin_stats()[bin_of(3)].spare, 2 * 1365 - 5 * 512);

    world.remove_lod(ChunkCoord([0, 0, 0]));
    to_full(&world, 0, &chunks);
    for x in 4..6 {
        to_lod(&world, x, &own[x as usize]);
    }
    let st = gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!((st.chunks, st.deferred_chunks), (2, 1));
    assert!(gpu.is_lod(ChunkCoord([0, 0, 0])), "the promotion waits");
    let rb = Readback::new(&device, &queue, &gpu);
    for x in 4..6 {
        rb.assert_lod(&gpu, &world, ChunkCoord([x, 0, 0]));
    }
    let st = gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!((st.chunks, st.deferred_chunks), (1, 0));
    let rb = Readback::new(&device, &queue, &gpu);
    rb.assert_chunk(&gpu, &world, ChunkCoord([0, 0, 0]));
}

/// A chunk whose bricks move bins past the pool is deferred whole: on a two-page pool (a page
/// of 2-unit slots holding a `Floor` chunk, a page of 3-unit slots holding two `Checker`
/// chunks), two new materials above the floor turn the floor's 64 bricks into 4-unit bricks,
/// for which no page is left. The sync is empty; the floor chunk's record and table entry
/// read back as before and still decode to the voxels it had, and it keeps its dirty bits.
#[test]
fn a_chunk_whose_bricks_move_past_the_pool_is_deferred_whole() {
    let Some((device, queue)) = device() else {
        return;
    };
    let extent = ChunkExtent::new([0, 0, 0], [3, 1, 1]);
    let world = World::generate(extent, &FloorAndChecker, &numbered_table(3));
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            budget_bytes: pages_budget(2, 3),
            ..MirrorConfig::new(Window::Fixed(extent))
        },
    );
    let c = ChunkCoord([0, 0, 0]);
    gpu.set_focus(VoxelCoord([1, 1, 1]));
    gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(gpu.pages_issued(), 2);
    let live = gpu.live_units();
    assert_eq!(live, 64 * 2 + 1024 * 3);
    let snapshot = |gpu: &GpuWorld| {
        let rb = Readback::new(&device, &queue, gpu);
        let voxels: Vec<Vec<u16>> = (0..CHUNK_BRICKS)
            .filter_map(|i| gpu.entry(c, i).filter(|e| e & FULL_BIT != 0))
            .map(|e| rb.voxels(e))
            .collect();
        (record_and_entry(&rb, gpu, c), voxels)
    };
    let before = snapshot(&gpu);

    for (m, z) in [(2, 4), (3, 5)] {
        let r = world
            .apply(&EditOp::box_(
                centre_of_voxel([0, 0, z]),
                centre_of_voxel([64, 64, z + 1]),
                Mode::Fill(m),
            ))
            .unwrap();
        assert_eq!(r.changed_from, [(0, 64 * 64)]);
    }
    let written = gpu.bytes_written();
    let err = gpu.sync_now(&world, &device, &queue).unwrap_err();
    assert_eq!(gpu.bytes_written(), written, "an empty sync writes nothing");
    assert_eq!(
        err,
        vxe_render::MirrorError::PoolTooLarge {
            needed: 3 * PAGE_BYTES,
            limit: 2 * PAGE_BYTES,
        }
    );
    assert_eq!(snapshot(&gpu), before);
    assert_eq!(gpu.pending_chunks(), 1);
    assert_eq!(gpu.live_units(), live);
    let chunk = world.store().get(c).unwrap();
    assert!(
        chunk.read().unwrap().any_dirty(),
        "the deferred chunk keeps its dirty bits"
    );
}

/// A page the sync allocates into is not credited free by a later demotion's frees. Two pages
/// of 3-unit slots: x = 0 LOD-only with uniform mips, x = 1..4 full (1536 bricks; page 1 holds
/// 171 of x = 3's, its cursor short of the page end). One sync promotes x = 0 to a chunk of 64
/// two-material bricks (onto page 1's cursor), demotes x = 3 (freeing its slots, page 1's 171
/// among them) and adds a LOD-only x = 4 whose `Mixed` mips need a page of 2-unit slots. The
/// bin took spare slots first, so the demotion's frees are credited as slots, not as an
/// emptied page 1: x = 4 is deferred whole, and the sync is `Ok`.
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
        to_full(&world, x, &chunks);
    }
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            budget_bytes: pages_budget(2, 12),
            ..MirrorConfig::new(Window::Fixed(world.extent()))
        },
    );
    gpu.set_focus(VoxelCoord([1, 1, 1]));
    gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(gpu.live_bricks(), 3 * 512);
    assert_eq!(gpu.pages_issued(), 2);

    // Material 1 below z = 2 and 2 at z = 2: 64 two-material bricks.
    struct TwoLayer;
    impl WorldSource for TwoLayer {
        fn chunk(&self, _: ChunkCoord, out: &mut Chunk) {
            for slot in 0..CHUNK_BRICKS {
                let b = brick_of_slot(slot);
                out.set_brick(
                    slot,
                    Brick::from_fn(|l| match b[2] * BRICK_EDGE + l[2] {
                        0 | 1 => 1,
                        2 => 2,
                        _ => AIR,
                    }),
                );
            }
        }
    }
    let two = World::generate(
        ChunkExtent::new([0, 0, 0], [1, 1, 1]),
        &TwoLayer,
        &numbered_table(2),
    );
    world.remove_lod(ChunkCoord([0, 0, 0]));
    world.store().write_lock().insert(
        ChunkCoord([0, 0, 0]),
        two.store().get(ChunkCoord([0, 0, 0])).unwrap(),
    );
    to_lod(&world, 3, &own[3]);
    world.insert_lod(ChunkCoord([4, 0, 0]), mixed[4].clone());
    let small_mips = (1..=MIP_LEVELS)
        .flat_map(|n| mixed[4].level(n))
        .filter(|b| b.class() == Some(PackedClass::Mask))
        .count();
    assert!(small_mips > 0, "x = 4's mips need 2-unit slots");
    let st = gpu
        .sync_now(&world, &device, &queue)
        .expect("x = 4 waits; the rest uploads");
    assert_eq!((st.chunks, st.deferred_chunks), (2, 1));
    assert!(!gpu.is_mirrored(ChunkCoord([4, 0, 0])));
    assert_eq!(gpu.pending_chunks(), 1);
    assert_eq!(gpu.live_bricks(), 2 * 512 + 64);
    let rb = Readback::new(&device, &queue, &gpu);
    rb.assert_chunk(&gpu, &world, ChunkCoord([0, 0, 0]));
    rb.assert_lod(&gpu, &world, ChunkCoord([3, 0, 0]));
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
    let pool: Vec<u32> = bytemuck::pod_collect_to_vec(&readback::read_buffer(
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

/// A sync clock, from 0, that advances `step` on every read at `on` and holds at the others.
fn stepping_clock(step: std::time::Duration, on: SyncPoint) -> vxe_render::SyncClock {
    let mut t = std::time::Duration::ZERO;
    Box::new(move |at| {
        if at == on {
            t += step;
        }
        t
    })
}

/// The 64 chunks of an 8 x 8 x 1 floor, each holding 64 `Full` bricks, mirrored under a time
/// budget of 8 steps of a clock that advances one step per uploaded chunk: the first sync sizes
/// all 64, uploads exactly 8 and leaves 56 pending as `time_deferred_chunks`; the second sizes
/// none (56 kept sizes) and uploads the 9th to the 16th of the unbudgeted sync's order (its
/// record indices 8 to 15). With the budget unset the same world uploads all 64 in one sync.
/// Once the budgeted mirror has drained in 8 syncs, every chunk has the same record index in
/// both, and the pool shards and the records read back byte-identical: the slot words do not
/// depend on how the uploads were split.
#[test]
fn a_sync_stops_at_its_time_budget() {
    let Some((device, queue)) = device() else {
        return;
    };
    let mut t = MaterialTable::new();
    t.push(Material::named("floor"));
    let extent = ChunkExtent::new([0, 0, 0], [8, 8, 1]);
    let fresh = || World::generate(extent, &Floor, &t);
    let step = std::time::Duration::from_micros(250);

    let whole_world = fresh();
    let mut whole = GpuWorld::with_config(&device, small(Window::Fixed(extent)));
    let s = whole.sync_now(&whole_world, &device, &queue).unwrap();
    assert_eq!(s.chunks, 64);
    assert_eq!(s.time_deferred_chunks, 0);
    assert_eq!(s.pending_chunks, 0);
    let coords = whole_world.store().coords();
    assert_eq!(coords.len(), 64);
    let order = |gpu: &GpuWorld| -> Vec<Option<u32>> {
        coords.iter().map(|&c| gpu.record_index(c)).collect()
    };
    let whole_order = order(&whole);
    let first_n = |n: u32| -> Vec<bool> {
        whole_order
            .iter()
            .map(|r| r.expect("mirrored") < n)
            .collect()
    };

    let split_world = fresh();
    let mut split = GpuWorld::with_config(
        &device,
        MirrorConfig {
            sync_budget_time: Some(8 * step),
            ..small(Window::Fixed(extent))
        },
    );
    split.set_sync_clock(stepping_clock(step, SyncPoint::Uploaded));
    let s = split.sync_now(&split_world, &device, &queue).unwrap();
    assert_eq!(s.chunks, 8, "8 chunks in 8 steps");
    assert_eq!(s.time_deferred_chunks, 56);
    assert_eq!(s.deferred_chunks, 0, "none deferred for space");
    assert_eq!(s.pending_chunks, 56);
    assert_eq!((s.sized_chunks, s.kept_sizes), (64, 0));
    let held: Vec<bool> = order(&split).iter().map(Option::is_some).collect();
    assert_eq!(
        held,
        first_n(8),
        "the nearest 8, as the whole sync ordered them"
    );
    let s = split.sync_now(&split_world, &device, &queue).unwrap();
    assert_eq!(s.chunks, 8);
    assert_eq!(s.time_deferred_chunks, 48);
    assert_eq!(
        (s.sized_chunks, s.kept_sizes),
        (0, 56),
        "the deferred chunks are not sized again"
    );
    let held: Vec<bool> = order(&split).iter().map(Option::is_some).collect();
    assert_eq!(held, first_n(16), "the second sync starts with the 9th");
    let mut syncs = 2;
    while split.pending_chunks() > 0 {
        split.sync_now(&split_world, &device, &queue).unwrap();
        syncs += 1;
    }
    assert_eq!(syncs, 8);
    assert_eq!(order(&split), whole_order);
    let read = |b: &wgpu::Buffer| readback::read_buffer(&device, &queue, b, 0, b.size());
    for k in 0..4 {
        assert!(
            read(split.pool_shard(k)) == read(whole.pool_shard(k)),
            "pool shard {k} differs"
        );
    }
    assert!(read(split.records()) == read(whole.records()));
}

/// The floor of [`a_sync_stops_at_its_time_budget`] under the same budget of 8 steps, with a
/// clock that advances one step per chunk selection sizes and holds while chunks upload: the
/// first sync sizes 8 chunks, stops sizing once the 8th spends the budget, writes the first
/// of them (a sync writes at least one chunk) and leaves the 7 others sized and pending
/// (`time_deferred_chunks`), the 56 unsized behind them. Every later sync takes the kept
/// sizes without sizing them again and sizes at most 8 new chunks, so the 64 chunks are sized
/// 64 times over the whole drain; the sync after the last sizing writes the 56 left. The
/// mirror's records and pool read back byte-identical to the unbudgeted sync's.
#[test]
fn a_sync_spent_while_sizing_keeps_what_it_sized() {
    let Some((device, queue)) = device() else {
        return;
    };
    let mut t = MaterialTable::new();
    t.push(Material::named("floor"));
    let extent = ChunkExtent::new([0, 0, 0], [8, 8, 1]);
    let fresh = || World::generate(extent, &Floor, &t);
    let step = std::time::Duration::from_micros(250);

    let whole_world = fresh();
    let mut whole = GpuWorld::with_config(&device, small(Window::Fixed(extent)));
    let s = whole.sync_now(&whole_world, &device, &queue).unwrap();
    assert_eq!((s.chunks, s.sized_chunks, s.kept_sizes), (64, 64, 0));
    let coords = whole_world.store().coords();
    let order = |gpu: &GpuWorld| -> Vec<Option<u32>> {
        coords.iter().map(|&c| gpu.record_index(c)).collect()
    };

    let split_world = fresh();
    let mut split = GpuWorld::with_config(
        &device,
        MirrorConfig {
            sync_budget_time: Some(8 * step),
            ..small(Window::Fixed(extent))
        },
    );
    split.set_sync_clock(stepping_clock(step, SyncPoint::Sized));
    let s = split.sync_now(&split_world, &device, &queue).unwrap();
    assert_eq!(s.sized_chunks, 8, "sizing stops once the budget is spent");
    assert_eq!(s.kept_sizes, 0);
    assert_eq!(s.chunks, 1, "the first sized chunk is written");
    assert_eq!(s.time_deferred_chunks, 7, "the rest of what was sized");
    assert_eq!(s.deferred_chunks, 0);
    assert_eq!(s.pending_chunks, 63);
    let s = split.sync_now(&split_world, &device, &queue).unwrap();
    assert_eq!(
        (s.sized_chunks, s.kept_sizes),
        (8, 7),
        "the 7 left are not sized again"
    );
    assert_eq!((s.chunks, s.time_deferred_chunks), (1, 14));
    let mut sized = 16;
    let mut syncs = 2;
    let mut last = s;
    while split.pending_chunks() > 0 {
        last = split.sync_now(&split_world, &device, &queue).unwrap();
        assert!(last.sized_chunks <= 8);
        sized += last.sized_chunks;
        syncs += 1;
    }
    assert_eq!(sized, 64, "each chunk sized once over the drain");
    assert_eq!(syncs, 9);
    assert_eq!(
        (last.sized_chunks, last.kept_sizes, last.chunks),
        (0, 56, 56)
    );
    assert_eq!(order(&split), order(&whole));
    let read = |b: &wgpu::Buffer| readback::read_buffer(&device, &queue, b, 0, b.size());
    for k in 0..4 {
        assert!(
            read(split.pool_shard(k)) == read(whole.pool_shard(k)),
            "pool shard {k} differs"
        );
    }
    assert!(read(split.records()) == read(whole.records()));
}

/// A chunk edited while it waits with a kept size uploads its edited words: the floor of
/// [`a_sync_stops_at_its_time_budget`] under its budget of 8 uploads a sync; after the first
/// sync the farthest chunk (the unbudgeted order's last) gains a 3 x 3 x 2 block of the floor
/// material above the floor. The next sync sizes that chunk again and only it (55 kept), and
/// once drained the mirror's records and pool read back byte-identical to an unbudgeted sync
/// of a world edited the same way before its first sync.
#[test]
fn a_chunk_edited_while_deferred_uploads_its_edited_words() {
    let Some((device, queue)) = device() else {
        return;
    };
    let mut t = MaterialTable::new();
    t.push(Material::named("floor"));
    let extent = ChunkExtent::new([0, 0, 0], [8, 8, 1]);
    let fresh = || World::generate(extent, &Floor, &t);
    let step = std::time::Duration::from_micros(250);

    // The unbudgeted order's last chunk, from an unedited mirror.
    let probe_world = fresh();
    let mut probe = GpuWorld::with_config(&device, small(Window::Fixed(extent)));
    probe.sync_now(&probe_world, &device, &queue).unwrap();
    let last = *probe_world
        .store()
        .coords()
        .iter()
        .max_by_key(|&&c| probe.record_index(c))
        .unwrap();
    let edit = |w: &mut World| {
        let v = last.min_voxel().0;
        let r = w
            .apply(&EditOp::box_(
                centre_of_voxel([v[0] + 1, v[1] + 1, v[2] + 3]),
                centre_of_voxel([v[0] + 4, v[1] + 4, v[2] + 5]),
                Mode::Fill(1),
            ))
            .unwrap();
        assert_eq!(r.changed_from, [(0, 18)]);
    };

    let mut whole_world = fresh();
    edit(&mut whole_world);
    let mut whole = GpuWorld::with_config(&device, small(Window::Fixed(extent)));
    whole.sync_now(&whole_world, &device, &queue).unwrap();

    let mut split_world = fresh();
    let mut split = GpuWorld::with_config(
        &device,
        MirrorConfig {
            sync_budget_time: Some(8 * step),
            ..small(Window::Fixed(extent))
        },
    );
    split.set_sync_clock(stepping_clock(step, SyncPoint::Uploaded));
    let s = split.sync_now(&split_world, &device, &queue).unwrap();
    assert_eq!((s.chunks, s.sized_chunks, s.pending_chunks), (8, 64, 56));
    assert_eq!(
        split.record_index(last),
        None,
        "the edited chunk is still deferred"
    );
    edit(&mut split_world);
    let s = split.sync_now(&split_world, &device, &queue).unwrap();
    assert_eq!(
        (s.sized_chunks, s.kept_sizes),
        (1, 55),
        "only the edited chunk is sized again"
    );
    while split.pending_chunks() > 0 {
        split.sync_now(&split_world, &device, &queue).unwrap();
    }
    let coords = whole_world.store().coords();
    let order = |gpu: &GpuWorld| -> Vec<Option<u32>> {
        coords.iter().map(|&c| gpu.record_index(c)).collect()
    };
    assert_eq!(order(&split), order(&whole));
    let read = |b: &wgpu::Buffer| readback::read_buffer(&device, &queue, b, 0, b.size());
    for k in 0..4 {
        assert!(
            read(split.pool_shard(k)) == read(whole.pool_shard(k)),
            "pool shard {k} differs"
        );
    }
    assert!(read(split.records()) == read(whole.records()));
}
