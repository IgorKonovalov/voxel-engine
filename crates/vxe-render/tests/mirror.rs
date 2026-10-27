//! The GPU mirror against a closure-source world: what `sync` uploads, what it frees, and
//! that the buffers hold what the CPU accounting says.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use vxe_render::attrib::{AttribBindings, AttribItem, AttribPass};
use vxe_render::gpu_world::{
    ABSENT, BINS, BITS_WORDS, CLASS_MASK, CLASS_SHIFT, CLASS_WIDE, EMPTY_BIT, ENTRY_WORD, FULL_BIT,
    HEADER_TRANSLUCENT_WORD, LOD_BIT, LOD_LEVEL_ENTRY, NO_PLANE, PAGE_BYTES, PAGE_UNITS,
    PLACEHOLDER_BYTES, RECORD_BYTES, RECORD_MASK, RECORD_WORDS, TRANSLUCENT_BIT, UNIT_BYTES,
    UNIT_MASK, UNIT_WORDS, attr_offset, bin_of, budget_for, chunk_surface, payload_units,
    slot_units, slots_per_page,
};
use vxe_render::{
    ATROUS_WGSL, ATTRIB_WGSL, COMPOSITE_WGSL, GRADIENT_WGSL, GpuWorld, LIGHT_WGSL, MARCH_WGSL,
    MirrorConfig, ShardLayout, TEMPORAL_WGSL, Window, readback, wgpu,
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
    // The payload by class code, and the attributes' offset past the translucent unit.
    assert_eq!(
        (1..=5).map(payload_units).collect::<Vec<_>>(),
        [0, 1, 2, 4, 16]
    );
    assert_eq!(attr_offset(FULL_BIT | 3 << CLASS_SHIFT), 4);
    assert_eq!(
        attr_offset(FULL_BIT | TRANSLUCENT_BIT | 3 << CLASS_SHIFT),
        5
    );
    assert_eq!(
        attr_offset(FULL_BIT | TRANSLUCENT_BIT | CLASS_WIDE << CLASS_SHIFT),
        18
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
            assert_eq!(
                set,
                *brick != Brick::Uniform(AIR),
                "{coord:?} brick {slot} bit"
            );
            let e = self.records[rec as usize * RECORD_WORDS + ENTRY_WORD + slot];
            match brick {
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
    let mut stats = Vec::new();

    let focus = VoxelCoord([0, 0, 0]);
    s.update(&world, &[focus]);
    let ev = s.wait_idle(&world);
    gpu.apply_events(&ev);
    gpu.set_focus(focus);
    let st = gpu.sync_now(&world, &device, &queue).unwrap();
    stats.push(st);
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
    assert_eq!(l.shards, 4);
    // RTX 3080 Laptop and RADV: 2 147 483 644 B per binding, the largest page multiple below.
    let l = ShardLayout::from_limits(&limits(2_147_483_644, 4_294_967_295, 9), None);
    assert_eq!(l.units_per_shard, 8191 * 4096);
    assert!(u64::from(l.units_per_shard) * 64 <= 2_147_483_644);
    // The reservation stops at what a 26-bit unit index addresses: 4 GiB of units.
    assert_eq!(l.reserved_units, 1 << 26);
    // The smaller of the two size limits binds.
    let l = ShardLayout::from_limits(&limits(1 << 30, 100 << 20, 8), None);
    assert_eq!(l.units_per_shard, 1_638_400);
    // Shards: storage buffers per stage less the five other bindings, at most four.
    assert_eq!(
        ShardLayout::from_limits(&limits(1 << 30, 1 << 30, 8), None).shards,
        3
    );
    assert_eq!(
        ShardLayout::from_limits(&limits(1 << 30, 1 << 30, 48), None).shards,
        4
    );
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

// ---- Contour attributes ----

/// With attributes, a full chunk's `Full` brick carries its chunk-local surface mask after its
/// CPU units and an attribute unit for each 32 surface voxels, "no plane" until the pass fills
/// them, its header holding the attribute units, the surface flag and each layer's prefix
/// popcount; a brick with no surface voxel carries none (a checkerboard chunk's inner bricks),
/// and a mip-brick none. Against the same world mirrored without attributes, the live units
/// differ by exactly the full chunks' bricks' bin growth.
#[test]
fn full_chunks_bricks_carry_their_surface_and_mip_bricks_no_attributes() {
    let Some((device, queue)) = device() else {
        return;
    };
    // A 300-material chunk, two checkerboard chunks and a LOD-only chunk of `Mixed` mips.
    struct ManyAndChecker;
    impl WorldSource for ManyAndChecker {
        fn chunk(&self, coord: ChunkCoord, out: &mut Chunk) {
            if coord.0[0] == 0 {
                ManyAndTwo.chunk(coord, out);
            } else {
                Checker.chunk(coord, out);
            }
        }
    }
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [4, 1, 1]),
        &ManyAndChecker,
        &numbered_table(300),
    );
    let lod = ChunkCoord([3, 0, 0]);
    world.store().remove(lod);
    let mixed = World::generate(world.extent(), &Mixed, &mixed_table());
    let mips = ChunkMips::build(&mixed.store().get(lod).unwrap().read().unwrap());
    world.insert_lod(lod, Arc::new(mips));
    let mut gpu = GpuWorld::with_surface_attrs(&device, small(Window::Fixed(world.extent())));
    gpu.sync_now(&world, &device, &queue).unwrap();
    assert!(gpu.surface_attrs());
    assert!(gpu.is_lod(lod));
    let rb = Readback::new(&device, &queue, &gpu);
    let (mut with_surface, mut without, mut growth) = (0, 0, 0u64);
    for c in world.store().coords() {
        rb.assert_chunk(&gpu, &world, c);
        let chunk = world.store().get(c).unwrap();
        let chunk = chunk.read().unwrap();
        for i in 0..CHUNK_BRICKS {
            let e = gpu.entry(c, i).unwrap();
            let Brick::Full(p) = chunk.brick(i) else {
                panic!("{c:?} brick {i} is Full")
            };
            let surface = chunk_surface(chunk.bricks(), i);
            let n: u32 = surface.iter().map(|w| w.count_ones()).sum();
            let units = slot_units(p, |_| false, Some(n));
            let plain = slot_units(p, |_| false, None);
            assert_eq!(
                slot_units_of(&gpu, e),
                BINS[bin_of(units)],
                "{c:?} brick {i}"
            );
            growth += u64::from(BINS[bin_of(units)] - BINS[bin_of(plain)]);
            let u = e & UNIT_MASK;
            let head = rb.word(u, UNIT_WORDS + 14);
            if n == 0 {
                assert_eq!(head, 0, "{c:?} brick {i}");
                without += 1;
                continue;
            }
            with_surface += 1;
            assert_eq!(head, n.div_ceil(32) | 1 << 16, "{c:?} brick {i}");
            let s = attr_offset(e) as usize * UNIT_WORDS;
            let mask: Vec<u32> = (0..16).map(|w| rb.word(u, s + w)).collect();
            assert_eq!(layers(&mask), surface, "{c:?} brick {i} surface");
            let mut below = 0;
            for (z, layer) in surface.iter().enumerate() {
                let prefix = rb.word(u, UNIT_WORDS + 10 + z / 2) >> (16 * (z % 2)) & 0xFFFF;
                assert_eq!(prefix, below, "{c:?} brick {i} prefix {z}");
                below += layer.count_ones();
            }
            for w in 0..n.div_ceil(32) as usize * UNIT_WORDS {
                assert_eq!(
                    rb.word(u, s + UNIT_WORDS + w),
                    u32::MAX,
                    "{c:?} brick {i} word {w}"
                );
            }
        }
    }
    assert!(with_surface > 0 && without > 0, "{with_surface} {without}");
    let mip_units = rb.assert_lod(&gpu, &world, lod);
    assert!(!mip_units.is_empty());
    for &u in &mip_units {
        assert!(BINS[gpu.bin_of_unit(u).unwrap()] <= 3, "mip unit {u}");
    }

    let mut plain = GpuWorld::with_config(&device, small(Window::Fixed(world.extent())));
    plain.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(plain.live_bricks(), gpu.live_bricks());
    assert_eq!(gpu.live_units() - plain.live_units(), growth);
    assert_eq!(
        gpu.mirror_bytes() - plain.mirror_bytes(),
        growth * UNIT_BYTES
    );
}

/// The attribute of voxel `i` of the slot `w` (its words from its first unit) of the `Full`
/// brick at entry `e`, read as the shader reads it: no plane off the surface mask, else the
/// attribute at the voxel's popcount index.
fn attr_in(w: &[u32], e: u32, i: usize) -> u16 {
    if w[UNIT_WORDS + 14] & 0xFFFF == 0 {
        return NO_PLANE;
    }
    let s = attr_offset(e) as usize * UNIT_WORDS;
    let (z, b) = (i / 64, i % 64);
    let layer = u64::from(w[s + 2 * z]) | u64::from(w[s + 2 * z + 1]) << 32;
    if layer >> b & 1 == 0 {
        return NO_PLANE;
    }
    let prefix = (w[UNIT_WORDS + 10 + z / 2] >> (16 * (z % 2)) & 0xFFFF) as usize;
    let k = prefix + (layer & ((1u64 << b) - 1)).count_ones() as usize;
    (w[s + UNIT_WORDS + k / 2] >> (16 * (k & 1))) as u16
}

/// The words of the slot at entry `e`, read back.
fn slot_words(device: &wgpu::Device, queue: &wgpu::Queue, gpu: &GpuWorld, e: u32) -> Vec<u32> {
    let units = u64::from(slot_units_of(gpu, e));
    let bytes = slot_bytes(device, queue, gpu, e & UNIT_MASK, 0, units * UNIT_BYTES);
    bytemuck::pod_collect_to_vec(&bytes)
}

/// The attribute words of the slot `w` at entry `e`: its header's attribute units' words.
fn attr_words(w: &[u32], e: u32) -> Vec<u32> {
    let n = (w[UNIT_WORDS + 14] & 0xFFFF) as usize * UNIT_WORDS;
    let s = (attr_offset(e) as usize + 1) * UNIT_WORDS;
    if n == 0 {
        Vec::new()
    } else {
        w[s..s + n].to_vec()
    }
}

/// A brick holding a 4 x 4 x 4 cube at voxels 2..6 of air has 56 surface voxels and two
/// attribute units; after the pass the attribute of voxel (2, 2, 2) is at index 0 and that of
/// (5, 5, 5) at index 55, each a plane facing out of the cube's corner. A carve of one cube
/// voxel re-uploads the brick with 55 surface voxels, and the pass's readback matches an
/// uncapped mirror's. A brick at a chunk's -x face whose face layer is solid counts those 64
/// voxels as surface.
#[test]
fn surface_attributes_are_indexed_by_popcount() {
    let Some((device, queue)) = device() else {
        return;
    };
    // The cube in brick (3, 3, 3) of one chunk: voxels 26..30 on each axis.
    struct Cube;
    impl WorldSource for Cube {
        fn chunk(&self, _: ChunkCoord, out: &mut Chunk) {
            out.set_brick(
                brick_slot([3, 3, 3]),
                Brick::from_fn(|l| u16::from(l.iter().all(|&v| (2..6).contains(&v)))),
            );
        }
    }
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [1, 1, 1]),
        &Cube,
        &numbered_table(1),
    );
    let mut gpu = GpuWorld::with_surface_attrs(&device, small(Window::Fixed(world.extent())));
    gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(update_attrs(&device, &queue, &mut gpu), 1);
    let c = ChunkCoord([0, 0, 0]);
    let slot = brick_slot([3, 3, 3]);
    let e = gpu.entry(c, slot).unwrap();
    let w = slot_words(&device, &queue, &gpu, e);
    let s = attr_offset(e) as usize * UNIT_WORDS;
    let surface: u32 = w[s..s + 16].iter().map(|x| x.count_ones()).sum();
    assert_eq!(surface, 56);
    assert_eq!(w[UNIT_WORDS + 14] & 0xFFFF, 2, "two attribute units");
    let attrs = attr_words(&w, e);
    assert_eq!(attrs.len(), 2 * UNIT_WORDS);
    let first = attrs[0] as u16;
    let last = (attrs[27] >> 16) as u16;
    let index = |l: [usize; 3]| l[0] + 8 * l[1] + 64 * l[2];
    assert_eq!(attr_in(&w, e, index([2, 2, 2])), first);
    assert_eq!(attr_in(&w, e, index([5, 5, 5])), last);
    assert!(attrs[28..].iter().all(|&x| x == u32::MAX), "past index 55");
    let (n0, _) = decode_attr(first).expect("(2, 2, 2) has a plane");
    let (n55, _) = decode_attr(last).expect("(5, 5, 5) has a plane");
    assert!(
        n0.iter().all(|&v| v < -0.5),
        "(2, 2, 2) faces -1, -1, -1: {n0:?}"
    );
    assert!(
        n55.iter().all(|&v| v > 0.5),
        "(5, 5, 5) faces +1, +1, +1: {n55:?}"
    );
    assert_eq!(attr_of(&device, &queue, &gpu, [26, 26, 26]), first);
    assert_eq!(
        attr_of(&device, &queue, &gpu, [27, 27, 27]),
        NO_PLANE,
        "buried"
    );

    // Carve the cube's corner (29, 29, 29): 55 surface voxels.
    let r = world
        .apply(&EditOp::box_(
            centre_of_voxel([29, 29, 29]),
            centre_of_voxel([30, 30, 30]),
            Mode::Carve,
        ))
        .unwrap();
    assert_eq!(r.changed_from, [(1, 1)]);
    gpu.sync_now(&world, &device, &queue).unwrap();
    let e = gpu.entry(c, slot).unwrap();
    let w = slot_words(&device, &queue, &gpu, e);
    let s = attr_offset(e) as usize * UNIT_WORDS;
    let surface: u32 = w[s..s + 16].iter().map(|x| x.count_ones()).sum();
    assert_eq!(surface, 55);
    update_attrs(&device, &queue, &mut gpu);
    assert_eq!(
        attr_bytes(&device, &queue, &gpu, &world),
        uncapped_attrs(&device, &queue, &world)
    );

    // Rock with one voxel of ore in brick (0, 3, 3): every voxel solid, its -x layer on the
    // chunk's face.
    struct Face;
    impl WorldSource for Face {
        fn chunk(&self, _: ChunkCoord, out: &mut Chunk) {
            for slot in 0..CHUNK_BRICKS {
                out.set_brick(slot, Brick::Uniform(1));
            }
            out.set_brick(
                brick_slot([0, 3, 3]),
                Brick::from_fn(|l| if l == [4, 4, 4] { 2 } else { 1 }),
            );
        }
    }
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [1, 1, 1]),
        &Face,
        &numbered_table(2),
    );
    let mut gpu = GpuWorld::with_surface_attrs(&device, small(Window::Fixed(world.extent())));
    gpu.sync_now(&world, &device, &queue).unwrap();
    let e = gpu.entry(c, brick_slot([0, 3, 3])).unwrap();
    let w = slot_words(&device, &queue, &gpu, e);
    let s = attr_offset(e) as usize * UNIT_WORDS;
    assert_eq!(layers(&w[s..s + 16]), [0x0101_0101_0101_0101; 8]);
    assert_eq!(w[UNIT_WORDS + 14] & 0xFFFF, 2);
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

/// The attribute of world voxel `v`, which lies in a `Full` brick of a mirrored full chunk.
fn attr_of(device: &wgpu::Device, queue: &wgpu::Queue, gpu: &GpuWorld, v: [i32; 3]) -> u16 {
    let voxel = VoxelCoord(v);
    let e = gpu.entry(voxel.chunk(), voxel.brick_slot()).unwrap();
    assert_ne!(e & FULL_BIT, 0, "{v:?} is in a Full brick");
    let w = slot_words(device, queue, gpu, e);
    attr_in(&w, e, voxel.index_in_brick())
}

#[test]
fn the_attribute_pass_fills_the_floor_and_reaches_a_carve_s_neighbours() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = world();
    let mut gpu = GpuWorld::with_surface_attrs(&device, small(Window::Fixed(world.extent())));
    gpu.sync_now(&world, &device, &queue).unwrap();
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
    let s = gpu.sync_now(&world, &device, &queue).unwrap();
    // The carved brick and its two `Full` face neighbours in the chunk, whose surface masks
    // may have changed; its third in-chunk face neighbour is uniform air.
    assert_eq!(s.bricks, 3, "the carved brick and its Full face neighbours");
    // The carved brick and its 26 neighbours: those in the mirror that are Full.
    let n = update_attrs(&device, &queue, &mut gpu);
    assert!(n > 1 && n <= 27, "{n} bricks");
    let after = attr_of(&device, &queue, &gpu, beside);
    assert_ne!(after, NO_PLANE);

    // A mirror built from scratch computes every attribute: the neighbour pass agrees.
    let mut fresh = GpuWorld::with_surface_attrs(&device, small(Window::Fixed(world.extent())));
    fresh.sync_now(&world, &device, &queue).unwrap();
    update_attrs(&device, &queue, &mut fresh);
    assert_eq!(after, attr_of(&device, &queue, &fresh, beside));
    assert_eq!(
        attr_of(&device, &queue, &gpu, [5, 3, 1]),
        attr_of(&device, &queue, &fresh, [5, 3, 1])
    );
}

/// The bound on pages ever issued for each bin's peak live slots: per bin with a slot, the
/// pages its peak fills and one more.
fn page_bound(peak: &[u32; 10]) -> u32 {
    peak.iter()
        .enumerate()
        .filter(|(_, n)| **n > 0)
        .map(|(k, &n)| n.div_ceil(slots_per_page(k)) + 1)
        .sum()
}

/// Bin churn in a following window: 8 chunks alternate 20 times between full with
/// attributes (20-unit slots) and LOD-only (2- and 3-unit slots), beside 2 chunks that stay
/// full. The pages ever issued stay within the per-bin bound, the stable chunks' bricks never
/// change slot, and after the churn every live brick and mip-brick decodes to its CPU voxels
/// and the churned chunks' attributes equal a fresh mirror's.
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
    let mut gpu = GpuWorld::with_surface_attrs(&device, small(Window::Follow { side: 7 }));
    gpu.set_focus(VoxelCoord([2 * 64 + 1, 64 + 1, 1]));
    gpu.sync_now(&world, &device, &queue).unwrap();
    update_attrs(&device, &queue, &mut gpu);
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
        update_attrs(&device, &queue, &mut gpu);
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
    let mut fresh = GpuWorld::with_surface_attrs(&device, small(Window::Follow { side: 7 }));
    fresh.set_focus(VoxelCoord([2 * 64 + 1, 64 + 1, 1]));
    fresh.sync_now(&world, &device, &queue).unwrap();
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

/// A demotion that frees its pages in the same sync fits a full pool, with attributes: three
/// `Checker` chunks take exactly the reserved pages (their inner bricks in the 3-unit bin,
/// their chunk-face bricks with surface attributes in larger bins); one sync demotes them to
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
    let mut gpu = GpuWorld::with_surface_attrs(
        &device,
        MirrorConfig {
            budget_bytes: pages_budget(u64::from(pages), 12),
            ..MirrorConfig::new(Window::Fixed(world.extent()))
        },
    );
    gpu.set_focus(VoxelCoord([1, 1, 1]));
    gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(gpu.reserved_units(), pages * 4096);
    assert_eq!(gpu.live_bricks(), 3 * 512);
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

/// The same without attributes: eight `Checker` chunks fill a four-page reservation of
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

/// The pages a mirror of `world`, with attributes when `attrs`, takes on its first sync, focus
/// at x = 0.
fn pages_taken(device: &wgpu::Device, queue: &wgpu::Queue, world: &World, attrs: bool) -> u32 {
    let cfg = small(Window::Fixed(world.extent()));
    let mut probe = if attrs {
        GpuWorld::with_surface_attrs(device, cfg)
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

/// The work list of every `Full` brick `gpu` mirrors for `world`'s chunks, in chunk then slot
/// order.
fn all_items(gpu: &GpuWorld, world: &World) -> Vec<AttribItem> {
    let mut coords = world.store().coords();
    coords.sort_by_key(|c| c.0);
    let mut items = Vec::new();
    for c in coords {
        for slot in 0..CHUNK_BRICKS {
            let e = gpu.entry(c, slot).unwrap();
            if e & FULL_BIT != 0 {
                let b = brick_of_slot(slot);
                items.push(AttribItem {
                    slot: e & !FULL_BIT,
                    origin: [0, 1, 2].map(|k| (c.0[k] * 8 + b[k]) * BRICK_EDGE),
                });
            }
        }
    }
    items
}

/// The attribute bytes of the brick with work item word `item` (an entry less `FULL_BIT`).
fn item_attrs(device: &wgpu::Device, queue: &wgpu::Queue, gpu: &GpuWorld, item: u32) -> Vec<u8> {
    let e = item | FULL_BIT;
    let w = slot_words(device, queue, gpu, e);
    bytemuck::cast_slice::<u32, u8>(&attr_words(&w, e)).to_vec()
}

/// The kept work buffer grows to the next power of two of a list that outgrows it, at least
/// 256 items, and only then: a 46-item list encoded 1000 times and a 2777-item list once
/// (the two carve sizes) build two buffers, and the attributes they write are the ones the
/// mirror's own pass writes.
#[test]
fn the_attribute_pass_keeps_its_work_buffer_until_a_list_outgrows_it() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = world();
    let cfg = || small(Window::Fixed(world.extent()));
    let mut gpu = GpuWorld::with_surface_attrs(&device, cfg());
    gpu.sync_now(&world, &device, &queue).unwrap();
    let items = all_items(&gpu, &world);
    assert_eq!(items.len(), 256);
    let e = gpu.window();
    let mut pass = AttribPass::new(&device);
    assert_eq!((pass.buffer_builds(), pass.work_capacity()), (0, 0));
    let run = |pass: &mut AttribPass, list: &[AttribItem]| {
        let b = AttribBindings {
            shards: [0, 1, 2, 3].map(|k| gpu.pool_shard(k)),
            records: gpu.records(),
            table: gpu.table(),
            window: (e.min.0, e.size()),
            units_per_shard: gpu.shard_layout().units_per_shard,
        };
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        pass.encode(&device, &queue, &mut enc, &b, list, None);
        queue.submit([enc.finish()]);
    };
    for _ in 0..1000 {
        run(&mut pass, &items[..46]);
    }
    assert_eq!((pass.buffer_builds(), pass.work_capacity()), (1, 256));
    let long: Vec<AttribItem> = items.iter().copied().cycle().take(2777).collect();
    run(&mut pass, &long);
    assert_eq!(pass.buffer_builds(), 2);
    assert_eq!(pass.work_capacity(), 4096);

    // The same attribute bytes as a mirror whose own pass filled every brick once.
    let mut fresh = GpuWorld::with_surface_attrs(&device, cfg());
    fresh.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(update_attrs(&device, &queue, &mut fresh), 256);
    let attrs = |g: &GpuWorld| -> Vec<u8> {
        items
            .iter()
            .flat_map(|item| item_attrs(&device, &queue, g, item.slot))
            .collect()
    };
    let (kept, per_mirror) = (attrs(&gpu), attrs(&fresh));
    // Each floor brick's top layer and its chunk-face bottom layer: at least 128 surface
    // voxels, four attribute units.
    assert!(kept.len() >= 256 * 4 * 64, "{} B", kept.len());
    assert!(kept.iter().any(|&b| b != 0xFF), "some voxel has a plane");
    assert!(
        kept == per_mirror,
        "the kept-buffer pass writes the same attributes"
    );
}

/// Rock filling chunk `(0, 0, 0)` except an air voxel at `(60, 4, 4)`, so its brick
/// `(7, 0, 0)` is `Full` and every other brick is uniform rock.
fn holed_rock() -> Chunk {
    let mut c = Chunk::uniform(1);
    c.set_brick(
        brick_slot([7, 0, 0]),
        Brick::from_fn(|l| u16::from(l != [4, 4, 4])),
    );
    c
}

/// The attribute of voxel `(63, 4, 4)` decodes to a `+x` plane while chunk `(1, 0, 0)` is
/// absent, to no plane while it is solid rock, and to the `+x` plane again once it leaves:
/// the boundary bricks next to a chunk whose record arrives or goes are recomputed.
#[test]
fn a_neighbour_chunk_arriving_and_leaving_recomputes_the_shell() {
    let Some((device, queue)) = device() else {
        return;
    };
    let mut t = MaterialTable::new();
    t.push(Material::named("rock"));
    let world = World::new(&t);
    let (here, east) = (ChunkCoord([0, 0, 0]), ChunkCoord([1, 0, 0]));
    world.store().insert(here, holed_rock());
    let mut gpu = GpuWorld::with_surface_attrs(
        &device,
        small(Window::Fixed(ChunkExtent::new([0, 0, 0], [2, 1, 1]))),
    );
    let v = [63, 4, 4];
    let plus_x = |a: u16, when: &str| {
        let (n, _) = decode_attr(a).unwrap_or_else(|| panic!("{when}: no plane"));
        assert!(n[0] > 0.9, "{when}: normal {n:?}");
    };

    gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(update_attrs(&device, &queue, &mut gpu), 1);
    plus_x(attr_of(&device, &queue, &gpu, v), "east absent");

    world.store().insert(east, Chunk::uniform(1));
    let st = gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(st.chunks, 1);
    assert!(gpu.is_mirrored(east));
    assert_eq!(
        update_attrs(&device, &queue, &mut gpu),
        1,
        "the shell brick"
    );
    assert_eq!(
        attr_of(&device, &queue, &gpu, v),
        NO_PLANE,
        "east solid: six solid face neighbours"
    );

    world.store().remove(east);
    gpu.apply_events(&StreamEvents {
        evicted: vec![east],
        ..Default::default()
    });
    gpu.sync_now(&world, &device, &queue).unwrap();
    assert!(!gpu.is_mirrored(east));
    assert_eq!(
        update_attrs(&device, &queue, &mut gpu),
        1,
        "the shell brick"
    );
    plus_x(attr_of(&device, &queue, &gpu, v), "east evicted");
}

/// The work-list lengths `update_attrs` returns over a scene that exercises each change set:
/// a first sync (chunks taken whole), tier churn (records written new and dropped, so shells),
/// a carve (re-uploaded bricks and their neighbourhoods), an eviction (a shell alone) and a
/// window shift. Pinned to the lengths the builder gave before it was factored out of
/// `GpuWorld` (ae2e088); the lists are deduplicated sets, so their lengths do not depend on
/// hash order.
#[test]
fn the_work_list_lengths_match_the_unfactored_builder() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [5, 2, 1]),
        &Mixed,
        &mixed_table(),
    );
    let churn: Vec<ChunkCoord> = world
        .store()
        .coords()
        .into_iter()
        .filter(|c| c.0[0] >= 3)
        .collect();
    let mips: HashMap<ChunkCoord, Arc<ChunkMips>> = churn
        .iter()
        .map(|&c| {
            let chunk = world.store().get(c).unwrap();
            (c, Arc::new(ChunkMips::build(&chunk.read().unwrap())))
        })
        .collect();
    let mut gpu = GpuWorld::with_surface_attrs(&device, small(Window::Follow { side: 7 }));
    gpu.set_focus(VoxelCoord([2 * 64 + 1, 64 + 1, 1]));
    let mut lengths = Vec::new();
    gpu.sync_now(&world, &device, &queue).unwrap();
    lengths.push(update_attrs(&device, &queue, &mut gpu));
    let mut held = HashMap::new();
    for round in 0..4 {
        for &c in &churn {
            if round % 2 == 0 {
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
        gpu.sync_now(&world, &device, &queue).unwrap();
        lengths.push(update_attrs(&device, &queue, &mut gpu));
    }
    world
        .apply(&EditOp::sphere(
            Fixed3([64 * FIXED_ONE, 60 * FIXED_ONE, 30 * FIXED_ONE]),
            12 * FIXED_ONE,
            Mode::Carve,
        ))
        .unwrap();
    gpu.sync_now(&world, &device, &queue).unwrap();
    lengths.push(update_attrs(&device, &queue, &mut gpu));
    let gone = ChunkCoord([2, 1, 0]);
    world.store().remove(gone);
    gpu.apply_events(&StreamEvents {
        evicted: vec![gone],
        ..Default::default()
    });
    gpu.sync_now(&world, &device, &queue).unwrap();
    lengths.push(update_attrs(&device, &queue, &mut gpu));
    gpu.set_focus(VoxelCoord([3 * 64 + 1, 64 + 1, 1]));
    gpu.sync_now(&world, &device, &queue).unwrap();
    lengths.push(update_attrs(&device, &queue, &mut gpu));
    println!("work-list lengths {lengths:?}");
    assert_eq!(lengths, [1343, 34, 568, 34, 568, 46, 56, 0]);
}

/// [`small`] with the attribute pass capped at `cap` compute items per call.
fn capped(window: Window, cap: usize) -> MirrorConfig {
    MirrorConfig {
        attr_cap: cap,
        ..small(window)
    }
}

/// Runs one attribute pass and waits for it; returns its items, every one a compute item:
/// no item carries bit 31, the clear flag the pass once had.
fn attr_pass(device: &wgpu::Device, queue: &wgpu::Queue, gpu: &mut GpuWorld) -> Vec<AttribItem> {
    gpu.prepare_attrs();
    let items = gpu.attr_list().to_vec();
    assert!(
        items.iter().all(|it| it.slot & 1 << 31 == 0),
        "a clear item"
    );
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    gpu.encode_attrs(&mut enc, None);
    queue.submit([enc.finish()]);
    items
}

/// The world brick of a work item.
fn brick_of(it: &AttribItem) -> [i32; 3] {
    it.origin.map(|v| v.div_euclid(8))
}

/// The attribute bytes of every `Full` brick of every mirrored full chunk of `world`, by world
/// brick coordinate: its attribute units' words, none for a brick without surface voxels.
fn attr_bytes(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    gpu: &GpuWorld,
    world: &World,
) -> HashMap<[i32; 3], Vec<u8>> {
    let rb = Readback::new(device, queue, gpu);
    let mut out = HashMap::new();
    for c in world.store().coords() {
        if !gpu.is_mirrored(c) || gpu.is_lod(c) {
            continue;
        }
        for i in 0..CHUNK_BRICKS {
            let Some(e) = gpu.entry(c, i).filter(|e| e & FULL_BIT != 0) else {
                continue;
            };
            let at = (e & UNIT_MASK) as usize * UNIT_WORDS;
            let words = attr_words(&rb.pool[at..], e);
            let l = brick_of_slot(i);
            let b = [0, 1, 2].map(|k| c.0[k] * 8 + l[k]);
            out.insert(b, bytemuck::cast_slice::<u32, u8>(&words).to_vec());
        }
    }
    out
}

/// An uncapped mirror of `world` after one sync and one pass: the reference a capped mirror
/// converges on.
fn uncapped_attrs(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    world: &World,
) -> HashMap<[i32; 3], Vec<u8>> {
    let mut gpu = GpuWorld::with_surface_attrs(device, small(Window::Fixed(world.extent())));
    gpu.sync_now(world, device, queue).unwrap();
    update_attrs(device, queue, &mut gpu);
    attr_bytes(device, queue, &gpu, world)
}

/// Runs passes until the carry-over queue is empty; returns how many ran.
fn drain_attrs(device: &wgpu::Device, queue: &wgpu::Queue, gpu: &mut GpuWorld) -> usize {
    let mut calls = 0;
    while gpu.attr_queue_age().1.is_some() {
        attr_pass(device, queue, gpu);
        calls += 1;
        assert!(calls < 1000, "the queue does not drain");
    }
    calls
}

/// A capped pass lists its cap of compute items and no clear item; a brick uploaded while its
/// item waits behind the cap reads every attribute word as `0xFFFFFFFF` (two `NO_PLANE`), as
/// its upload wrote it; the queue drains onto the uncapped mirror's attributes.
#[test]
fn a_capped_pass_emits_no_clear_item_and_what_waits_reads_no_plane() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = world();
    let mut gpu = GpuWorld::with_surface_attrs(&device, capped(Window::Fixed(world.extent()), 64));
    let s = gpu.sync_now(&world, &device, &queue).unwrap();
    let k = s.bricks as usize;
    println!("first sync wrote K = {k} Full bricks");
    assert!(k > 64);
    let computes = attr_pass(&device, &queue, &mut gpu);
    assert_eq!(computes.len(), 64, "the cap, and no clear item");
    assert_eq!(gpu.attr_stats().items, 64);
    let computed: HashSet<[i32; 3]> = computes.iter().map(brick_of).collect();
    let attrs = attr_bytes(&device, &queue, &gpu, &world);
    assert_eq!(attrs.len(), k);
    let mut waiting = 0;
    for (b, bytes) in &attrs {
        if !computed.contains(b) {
            waiting += 1;
            assert!(
                bytes.iter().all(|&x| x == 0xFF),
                "brick {b:?} reads a plane"
            );
        }
    }
    assert_eq!(waiting, k - 64);
    let calls = 1 + drain_attrs(&device, &queue, &mut gpu);
    assert_eq!(calls, k.div_ceil(64));
    assert_eq!(
        attr_bytes(&device, &queue, &gpu, &world),
        uncapped_attrs(&device, &queue, &world)
    );
}

#[test]
fn a_carve_is_computed_in_the_next_pass_ahead_of_the_queue() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = world();
    let mut gpu = GpuWorld::with_surface_attrs(&device, capped(Window::Fixed(world.extent()), 64));
    let k = gpu.sync_now(&world, &device, &queue).unwrap().bricks as usize;
    attr_pass(&device, &queue, &mut gpu);
    assert!(k - 64 > 64, "{} bricks wait", k - 64);
    world
        .apply(&EditOp::box_(
            Fixed3([6 * FIXED_ONE, 0, 0]),
            Fixed3([8 * FIXED_ONE, 8 * FIXED_ONE, 3 * FIXED_ONE]),
            Mode::Carve,
        ))
        .unwrap();
    // The bricks the next sync re-uploads, and their 26 neighbours.
    let mut reuploaded = Vec::new();
    for c in world.store().coords() {
        let chunk = world.store().get(c).unwrap();
        for i in chunk.read().unwrap().dirty_slots() {
            let l = brick_of_slot(i);
            reuploaded.push([0, 1, 2].map(|k| c.0[k] * 8 + l[k]));
        }
    }
    assert_eq!(reuploaded, [[0, 0, 0]]);
    let s = gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(s.bricks, 3, "the carved brick and its Full face neighbours");
    let computes = attr_pass(&device, &queue, &mut gpu);
    let computed: HashSet<[i32; 3]> = computes.iter().map(brick_of).collect();
    for b in &reuploaded {
        for d in [-1, 0, 1] {
            for e in [-1, 0, 1] {
                for f in [-1, 0, 1] {
                    let n = [b[0] + d, b[1] + e, b[2] + f];
                    let v = VoxelCoord(n.map(|x| x * 8));
                    let full = gpu.is_mirrored(v.chunk())
                        && gpu
                            .entry(v.chunk(), v.brick_slot())
                            .is_some_and(|e| e & FULL_BIT != 0);
                    if full {
                        assert!(computed.contains(&n), "{n:?} waits");
                    }
                }
            }
        }
    }
    assert!(computed.contains(&[0, 0, 0]));
    drain_attrs(&device, &queue, &mut gpu);
    assert_eq!(
        attr_bytes(&device, &queue, &gpu, &world),
        uncapped_attrs(&device, &queue, &world)
    );
}

#[test]
fn a_slot_reused_while_its_old_brick_waits_ends_with_the_new_brick_s_attributes() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = world();
    let mut gpu = GpuWorld::with_surface_attrs(&device, capped(Window::Fixed(world.extent()), 64));
    gpu.sync_now(&world, &device, &queue).unwrap();
    let computed: HashSet<[i32; 3]> = attr_pass(&device, &queue, &mut gpu)
        .iter()
        .map(brick_of)
        .collect();
    // A waiting floor brick A (z brick 0), and the air brick B above it in the same chunk.
    let a = all_items(&gpu, &world)
        .iter()
        .map(brick_of)
        .filter(|b| !computed.contains(b) && b[2] == 0)
        .min()
        .expect("a floor brick waits");
    let b = [a[0], a[1], 1];
    let entry = |gpu: &GpuWorld, br: [i32; 3]| {
        let v = VoxelCoord(br.map(|x| x * 8));
        gpu.entry(v.chunk(), v.brick_slot()).unwrap()
    };
    let a_unit = entry(&gpu, a) & UNIT_MASK;
    assert_eq!(entry(&gpu, b) & FULL_BIT, 0, "B starts uniform air");
    let lo = a.map(|x| i64::from(x) * 8 * FIXED_ONE);
    // A filled solid turns uniform and frees its slot; B gains two one-voxel slabs (its layers
    // 0 and 2), 128 surface voxels as A had with its top and chunk-face bottom layers, so it
    // takes a slot of A's bin.
    world
        .apply(&EditOp::box_(
            Fixed3(lo),
            Fixed3([
                lo[0] + 8 * FIXED_ONE,
                lo[1] + 8 * FIXED_ONE,
                lo[2] + 8 * FIXED_ONE,
            ]),
            Mode::Fill(1),
        ))
        .unwrap();
    for z in [8, 10] {
        world
            .apply(&EditOp::box_(
                Fixed3([lo[0], lo[1], lo[2] + z * FIXED_ONE]),
                Fixed3([
                    lo[0] + 8 * FIXED_ONE,
                    lo[1] + 8 * FIXED_ONE,
                    lo[2] + (z + 1) * FIXED_ONE,
                ]),
                Mode::Fill(1),
            ))
            .unwrap();
    }
    gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(entry(&gpu, a) & FULL_BIT, 0, "A is uniform");
    assert_eq!(entry(&gpu, b) & UNIT_MASK, a_unit, "B took A's slot");
    drain_attrs(&device, &queue, &mut gpu);
    let got = attr_bytes(&device, &queue, &gpu, &world);
    let want = uncapped_attrs(&device, &queue, &world);
    assert_eq!(got[&b], want[&b]);
    assert_eq!(got, want);
}

/// One chunk holding five `Full` bricks along x at bricks (1..6, 3, 3): the 4 x 4 x 4 cube at
/// voxels 2..6 (56 surface voxels), a 3D checkerboard (256 solid voxels, every one surface),
/// and three floor bricks (material 1 below local z = 3); everything else air.
struct FiveBricks;

impl WorldSource for FiveBricks {
    fn chunk(&self, _: ChunkCoord, out: &mut Chunk) {
        out.set_brick(
            brick_slot([1, 3, 3]),
            Brick::from_fn(|l| u16::from(l.iter().all(|&v| (2..6).contains(&v)))),
        );
        out.set_brick(
            brick_slot([2, 3, 3]),
            Brick::from_fn(|l| u16::from((l[0] + l[1] + l[2]) % 2 == 0)),
        );
        for x in 3..6 {
            out.set_brick(
                brick_slot([x, 3, 3]),
                Brick::from_fn(|l| u16::from(l[2] < 3)),
            );
        }
    }
}

/// A work list of five bricks (one partial workgroup) holding the cube brick and the
/// checkerboard brick reads back the same attribute words as an uncapped mirror's own pass,
/// and the words the one-brick workgroups of c6df852 wrote for them (pinned as a hash): the
/// cube's 56 surface voxels in 2 units, the checkerboard's 256 in 8.
#[test]
fn a_five_brick_list_matches_an_uncapped_mirror_and_the_one_brick_pass() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [1, 1, 1]),
        &FiveBricks,
        &numbered_table(1),
    );
    let mut gpu = GpuWorld::with_surface_attrs(&device, small(Window::Fixed(world.extent())));
    gpu.sync_now(&world, &device, &queue).unwrap();
    let items = all_items(&gpu, &world);
    assert_eq!(items.len(), 5);
    let c = ChunkCoord([0, 0, 0]);
    let heads: Vec<u32> = [1, 2]
        .iter()
        .map(|&x| {
            let e = gpu.entry(c, brick_slot([x, 3, 3])).unwrap();
            slot_words(&device, &queue, &gpu, e)[UNIT_WORDS + 14] & 0xFFFF
        })
        .collect();
    assert_eq!(
        heads,
        [2, 8],
        "the cube's and the checkerboard's attribute units"
    );
    let e = gpu.window();
    let mut pass = AttribPass::new(&device);
    let b = AttribBindings {
        shards: [0, 1, 2, 3].map(|k| gpu.pool_shard(k)),
        records: gpu.records(),
        table: gpu.table(),
        window: (e.min.0, e.size()),
        units_per_shard: gpu.shard_layout().units_per_shard,
    };
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    pass.encode(&device, &queue, &mut enc, &b, &items, None);
    queue.submit([enc.finish()]);
    let got: Vec<u8> = items
        .iter()
        .flat_map(|it| item_attrs(&device, &queue, &gpu, it.slot))
        .collect();
    let units: u32 = items
        .iter()
        .map(|it| slot_words(&device, &queue, &gpu, it.slot | FULL_BIT)[UNIT_WORDS + 14] & 0xFFFF)
        .sum();
    assert_eq!(got.len(), units as usize * UNIT_BYTES as usize);
    assert!(got.iter().any(|&b| b != 0xFF), "some voxel has a plane");
    let mut fresh = GpuWorld::with_surface_attrs(&device, small(Window::Fixed(world.extent())));
    fresh.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(update_attrs(&device, &queue, &mut fresh), 5);
    let want: Vec<u8> = items
        .iter()
        .flat_map(|it| item_attrs(&device, &queue, &fresh, it.slot))
        .collect();
    assert!(got == want, "the list's words equal the uncapped mirror's");
    let mut h = hash::Fnv1a::default();
    h.write(&got);
    println!("five-brick attribute hash {:#018x}", h.finish());
    assert_eq!(
        h.finish(),
        FIVE_BRICK_HASH,
        "c6df852's one-brick workgroups"
    );
}

/// The hash of the five bricks' attribute words as c6df852's pass wrote them.
const FIVE_BRICK_HASH: u64 = 0xd795_d691_eff6_7ad3;

/// A `Mixed` full chunk at (1, 0, 0) whose five neighbours in a 3 x 2 x 1 window are LOD-only
/// (their own mips): its bricks' attribute words, by world brick in ascending order, as an
/// uncapped pass writes them, pinned as a hash. The pass reads a LOD-only neighbour at its
/// level-1 mip cell, so the words of the bricks on the chunk's faces toward those neighbours
/// differ from the same chunk's with the neighbours absent, and the inner bricks' words do
/// not.
#[test]
fn attributes_beside_lod_only_chunks_read_their_mips() {
    let Some((device, queue)) = device() else {
        return;
    };
    let extent = ChunkExtent::new([0, 0, 0], [3, 2, 1]);
    let full = ChunkCoord([1, 0, 0]);
    let lod_world = World::generate(extent, &Mixed, &mixed_table());
    for c in lod_world.store().coords() {
        if c != full {
            let m = Arc::new(ChunkMips::build(
                &lod_world.store().get(c).unwrap().read().unwrap(),
            ));
            lod_world.store().remove(c);
            lod_world.insert_lod(c, m);
        }
    }
    let alone = World::generate(extent, &Mixed, &mixed_table());
    for c in alone.store().coords() {
        if c != full {
            alone.store().remove(c);
        }
    }
    let attrs = |world: &World| {
        let mut gpu = GpuWorld::with_surface_attrs(&device, small(Window::Fixed(extent)));
        gpu.sync_now(world, &device, &queue).unwrap();
        assert!(update_attrs(&device, &queue, &mut gpu) > 0);
        let mut v: Vec<_> = attr_bytes(&device, &queue, &gpu, world)
            .into_iter()
            .collect();
        v.sort_unstable();
        v
    };
    let beside = attrs(&lod_world);
    let without = attrs(&alone);
    assert_eq!(beside.len(), without.len());
    let mut faces = 0;
    for ((b, got), (b2, other)) in beside.iter().zip(&without) {
        assert_eq!(b, b2);
        let l = b.map(|v| v.rem_euclid(8));
        // The chunk's faces toward its LOD-only neighbours: x = 0, x = 7, y = 7.
        let toward = l[0] == 0 || l[0] == 7 || l[1] == 7;
        if !toward {
            assert_eq!(got, other, "inner brick {b:?}");
        } else if toward && got != other {
            faces += 1;
        }
    }
    println!("face bricks whose words a LOD-only neighbour changed: {faces}");
    assert!(
        faces > 20,
        "{faces} face bricks read their LOD-only neighbour"
    );
    let mut h = hash::Fnv1a::default();
    for (_, w) in &beside {
        h.write(w);
    }
    println!("beside-LOD attribute hash {:#018x}", h.finish());
    assert_eq!(h.finish(), BESIDE_LOD_HASH, "the pass's words at the tip");
}

/// The hash of the full chunk's attribute words beside its LOD-only neighbours.
const BESIDE_LOD_HASH: u64 = 0xda7d_5f1f_ace6_9921;

// ---- Plane fits ----

/// The four plane fits: the uniform and weighted gradients, each with the fraction and the
/// volume-matched offset.
fn fits() -> [vxe_render::SurfaceFit; 4] {
    use vxe_render::{PlaneFit, PlaneOffset, SurfaceFit};
    [
        (PlaneFit::Uniform27, PlaneOffset::Fraction),
        (PlaneFit::Weighted27, PlaneOffset::Fraction),
        (PlaneFit::Uniform27, PlaneOffset::VolumeMatched),
        (PlaneFit::Weighted27, PlaneOffset::VolumeMatched),
    ]
    .map(|(fit, offset)| SurfaceFit { fit, offset })
}

/// `world` mirrored with attributes under `fit`, its attributes filled.
fn fitted(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    world: &World,
    fit: vxe_render::SurfaceFit,
) -> GpuWorld {
    let mut gpu = GpuWorld::with_surface_attrs(device, small(Window::Fixed(world.extent())));
    gpu.set_surface_fit(fit);
    assert_eq!(gpu.surface_fit(), fit);
    gpu.sync_now(world, device, queue).unwrap();
    assert!(update_attrs(device, queue, &mut gpu) > 0);
    gpu
}

/// The decoded attribute of voxel `v` (its plane normal and offset code), read back.
fn attr_at(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    gpu: &GpuWorld,
    v: [i32; 3],
) -> Option<([f32; 3], u16)> {
    let vc = VoxelCoord(v);
    let c = vc.chunk();
    let l = vc.local();
    let b = l.map(|x| x / BRICK_EDGE);
    let e = gpu
        .entry(c, brick_slot(b))
        .expect("the voxel's chunk is mirrored");
    assert!(e & FULL_BIT != 0, "voxel {v:?} lies in a Full brick");
    let w = slot_words(device, queue, gpu, e);
    decode_attr(attr_in(&w, e, index_in_brick(l.map(|x| x % BRICK_EDGE))))
}

fn angle_deg(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d: f32 = (0..3).map(|k| a[k] * b[k]).sum();
    d.clamp(-1.0, 1.0).acos().to_degrees()
}

/// Material 1 at `x >= 63, y >= 63, z < 40` of chunks `[0, 2)^2 x [0, 1)`: a convex vertical
/// edge off the brick grid, its column `x = y = 63`; and material 1 below a one-voxel stair,
/// `z < 3` for `x < 16` and `z < 4` from `x = 16`, under `y < 32`.
fn edge_and_stair() -> World {
    let mut t = MaterialTable::new();
    t.push(Material::named("rock"));
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [2, 2, 1]),
        &UniformSource(AIR),
        &t,
    );
    let f = |v: [i64; 3]| Fixed3(v.map(|x| x * FIXED_ONE));
    for (min, max) in [
        ([63, 63, 0], [128, 128, 40]),
        ([0, 0, 0], [16, 32, 3]),
        ([16, 0, 0], [40, 32, 4]),
    ] {
        world
            .apply(&EditOp::box_(f(min), f(max), Mode::Fill(1)))
            .expect("resident");
    }
    world
}

/// Where the geometry is simple every fit agrees: the floor's top solid voxel decodes to +z
/// within 3 degrees and offset code 11 (`(18/27 - 0.5) x 3 = 0.5`, and the volume-matched
/// plane of 18 solid voxels in the block lies at +0.5), and a voxel of a convex edge's column
/// (12 of 27 solid) to `(-1, -1, 0) / sqrt 2` within 3 degrees and code 6 (`-0.167` and
/// `-0.121`, both rounded to `-1/8`). A one-voxel stair is where they differ: its codes are
/// printed. Every fit leaves the mirror's bytes as the default's.
#[test]
fn plane_fits_agree_where_the_geometry_is_simple() {
    let Some((device, queue)) = device() else {
        return;
    };
    let floor = world();
    let edges = edge_and_stair();
    let r = std::f32::consts::FRAC_1_SQRT_2;
    let mut bytes = None;
    for fit in fits() {
        let gpu = fitted(&device, &queue, &floor, fit);
        let (n, k) = attr_at(&device, &queue, &gpu, [10, 10, 2]).expect("the floor has a plane");
        println!("{}: floor top {n:?} code {k}", fit.name());
        assert!(
            angle_deg(n, [0.0, 0.0, 1.0]) <= 3.0,
            "{}: {n:?}",
            fit.name()
        );
        assert_eq!(k, 11, "{}: floor code", fit.name());
        let b = (gpu.mirror_bytes(), gpu.allocated_bytes());
        assert_eq!(*bytes.get_or_insert(b), b, "{}: mirror bytes", fit.name());

        let gpu = fitted(&device, &queue, &edges, fit);
        let (n, k) = attr_at(&device, &queue, &gpu, [63, 63, 20]).expect("the edge has a plane");
        println!("{}: edge column {n:?} code {k}", fit.name());
        assert!(angle_deg(n, [-r, -r, 0.0]) <= 3.0, "{}: {n:?}", fit.name());
        assert_eq!(k, 6, "{}: edge code", fit.name());
        for v in [[15, 16, 2], [16, 16, 3], [17, 16, 3], [14, 16, 2]] {
            let a = attr_at(&device, &queue, &gpu, v);
            println!(
                "{}: stair voxel {v:?}: {}",
                fit.name(),
                a.map_or("no plane".to_owned(), |(n, k)| format!(
                    "normal {n:.3?}, code {k} ({:+.3} voxel)",
                    (f32::from(k) - 7.0) / 8.0
                ))
            );
        }
    }
}

/// [`edge_and_stair`] with a second one-voxel stair on top of the block, in the far chunk
/// `(1, 1, 0)`: `z < 41` for `x < 96` and `z < 42` from `x = 96`, over `y` in `[64, 128)`.
fn far_stair() -> World {
    let world = edge_and_stair();
    let f = |v: [i64; 3]| Fixed3(v.map(|x| x * FIXED_ONE));
    for (min, max) in [
        ([64, 64, 40], [96, 128, 41]),
        ([96, 64, 40], [128, 128, 42]),
    ] {
        world
            .apply(&EditOp::box_(f(min), f(max), Mode::Fill(1)))
            .expect("resident");
    }
    world
}

/// A fit change re-queues every full chunk through the attribute pass under its cap: until a
/// brick's turn its voxels keep the previous fit's attribute (never "no plane") and the mirror
/// reports `refitting`; once it drains, every probed voxel equals a fresh fill's under the new
/// fit, among them a stair voxel of the far chunk whose attribute the change moved. Neither
/// the refit nor a material rewrite moves the mirror's bytes, and the rewrite reaches the
/// material rows.
#[test]
fn a_fit_change_refits_every_brick_under_the_cap() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = far_stair();
    let [old, new] = [fits()[0], fits()[3]];
    let fresh = fitted(&device, &queue, &world, new);
    let cap = 64;
    let mut gpu = GpuWorld::with_surface_attrs(
        &device,
        MirrorConfig {
            attr_cap: cap,
            ..small(Window::Fixed(world.extent()))
        },
    );
    assert_eq!(gpu.surface_fit(), old);
    gpu.sync_now(&world, &device, &queue).unwrap();
    while update_attrs(&device, &queue, &mut gpu) > 0 {}
    assert!(!gpu.refitting());
    let bytes = (gpu.mirror_bytes(), gpu.allocated_bytes());
    // The far stair's lower and upper voxels, the near stair's, the edge column, the top face.
    let probes = [
        [95, 90, 40],
        [96, 90, 41],
        [15, 16, 2],
        [63, 63, 20],
        [100, 100, 41],
    ];
    let at = |gpu: &GpuWorld, v| attr_at(&device, &queue, gpu, v);
    let before: Vec<_> = probes.iter().map(|&v| at(&gpu, v)).collect();
    let want: Vec<_> = probes.iter().map(|&v| at(&fresh, v)).collect();
    for (v, (b, w)) in probes.iter().zip(before.iter().zip(&want)) {
        println!("{v:?}: {} {b:?}, {} {w:?}", old.name(), new.name());
        assert!(b.is_some() && w.is_some(), "{v:?} has a plane");
    }
    assert_ne!(before[0], want[0], "the far stair's lower voxel moves");

    gpu.set_surface_fit(new);
    assert!(gpu.refitting());
    let mut calls = 0;
    while gpu.refitting() {
        let n = update_attrs(&device, &queue, &mut gpu);
        assert!(n > 0 && n <= cap, "call {calls}: {n} items");
        calls += 1;
        assert!(calls < 100_000, "the refit drains");
        let now: Vec<_> = probes.iter().map(|&v| at(&gpu, v)).collect();
        for (i, a) in now.iter().enumerate() {
            assert!(
                *a == before[i] || *a == want[i],
                "call {calls}: {:?} reads {a:?}",
                probes[i]
            );
        }
    }
    println!(
        "refit of {} bricks in {calls} calls under a cap of {cap}",
        world.stats().full_bricks
    );
    assert!(calls > 1, "the refit spreads over calls");
    for (i, &v) in probes.iter().enumerate() {
        assert_eq!(at(&gpu, v), want[i], "{v:?} after the drain");
    }
    assert_eq!((gpu.mirror_bytes(), gpu.allocated_bytes()), bytes);

    let mut t = MaterialTable::new();
    for row in world.materials().rows().iter().skip(1) {
        let mut row = row.clone();
        row.detail = [2.0, 0.3];
        row.detail_fine = 0.5;
        t.push(row);
    }
    gpu.rewrite_materials(&t, &queue);
    let rows = readback::read_buffer(&device, &queue, gpu.materials(), 0, 2 * 64);
    let rock: &[f32] = bytemuck::cast_slice(&rows[64..]);
    assert_eq!([rock[8], rock[9], rock[11]], [2.0, 0.3, 0.5]);
    assert_eq!((gpu.mirror_bytes(), gpu.allocated_bytes()), bytes);
}
