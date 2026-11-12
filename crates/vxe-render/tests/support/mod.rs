//! The GPU mirror tests' shared fixtures: the device, the source worlds, the read-back of the
//! mirror's buffers and the accounting asserts the `gpu_mirror_*` binaries share.

#![allow(dead_code, reason = "shared across test binaries; each uses a subset")]

use std::sync::Arc;
use vxe_render::{GpuWorld, MirrorConfig, SyncPoint, Window, wgpu};
use vxe_render::{
    internals::ABSENT, internals::BITS_WORDS, internals::CLASS_MASK, internals::CLASS_SHIFT,
    internals::CLASS_UNIFORM, internals::CLASS_WIDE, internals::EMPTY_BIT, internals::ENTRY_WORD,
    internals::FULL_BIT, internals::HEADER_BAND_BIT, internals::HEADER_BAND_WORD,
    internals::HEADER_TRANSLUCENT_WORD, internals::LOD_BIT, internals::LOD_LEVEL_ENTRY,
    internals::PAGE_UNITS, internals::RECORD_MASK, internals::RECORD_WORDS, internals::SLOT_BINS,
    internals::TRANSLUCENT_BIT, internals::UNIT_BYTES, internals::UNIT_MASK, internals::UNIT_WORDS,
    internals::budget_for, internals::payload_units,
};
use vxe_world::*;

pub fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
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
pub trait SyncNow {
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
pub const SMALL_BUDGET: u64 = 64 << 20;

/// `window` with no sync budget and a 64 MiB reservation.
pub fn small(window: Window) -> MirrorConfig {
    MirrorConfig {
        budget_bytes: SMALL_BUDGET,
        ..MirrorConfig::new(window)
    }
}

/// The budget of `pages` whole pages and `records` records.
pub fn pages_budget(pages: u64, records: u64) -> u64 {
    budget_for(pages * u64::from(PAGE_UNITS), records)
}

/// Material 1 below z = 3, air above: every brick at z in [0, 8) is `Full`.
pub struct Floor;

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

pub fn world() -> World {
    let mut t = MaterialTable::new();
    t.push(Material::named("floor"));
    World::generate(ChunkExtent::new([0, 0, 0], [2, 2, 1]), &Floor, &t)
}

/// Sixteen mask words as eight `u64` layers, low word first.
pub fn layers(w: &[u32]) -> [u64; 8] {
    std::array::from_fn(|z| u64::from(w[2 * z]) | u64::from(w[2 * z + 1]) << 32)
}

/// The class code of a `Full` entry.
pub fn class_of(e: u32) -> u32 {
    e >> CLASS_SHIFT & CLASS_MASK
}

/// Each brick is air, material 1 or mixed by a hash of its brick coordinate; a mixed brick
/// draws each voxel from air, 1 and 2. Chunks hold `Uniform` and `Full` bricks.
pub struct Mixed;

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

pub fn mixed_table() -> MaterialTable {
    let mut t = MaterialTable::new();
    t.push(Material::named("one"));
    t.push(Material::named("two"));
    t
}

/// Whole-buffer copies of the mirror, decoded the way the shader reads them.
pub struct Readback {
    pub table: Vec<u32>,
    pub records: Vec<u32>,
    /// The pool's words as far as the pages handed out reach, shards in order.
    pub pool: Vec<u32>,
}

impl Readback {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, gpu: &GpuWorld) -> Self {
        let all = |b: &wgpu::Buffer| vxe_render::debug::read_buffer(device, queue, b, 0, b.size());
        let units = u64::from(gpu.pool_units());
        let ups = u64::from(gpu.shard_layout().units_per_shard);
        let shard = |k: usize| {
            let n = units.saturating_sub(k as u64 * ups).min(ups);
            vxe_render::debug::read_buffer(device, queue, gpu.pool_shard(k), 0, n * UNIT_BYTES)
        };
        let pool: Vec<u8> = (0..gpu.shards_in_use()).flat_map(shard).collect();
        Self {
            table: bytemuck::cast_slice(&all(gpu.table())).to_vec(),
            records: bytemuck::cast_slice(&all(gpu.records())).to_vec(),
            pool: bytemuck::pod_collect_to_vec(&pool),
        }
    }

    /// Word `w` of the slot whose first unit is `u`.
    pub fn word(&self, u: u32, w: usize) -> u32 {
        self.pool[u as usize * UNIT_WORDS + w]
    }

    /// The band slot of brick `slot` of `c` (its band mask and count units, as its bin holds
    /// them), `None` for a brick without one.
    pub fn band(&self, gpu: &GpuWorld, c: ChunkCoord, slot: usize) -> Option<Vec<u32>> {
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
    pub fn voxels(&self, e: u32) -> Vec<u16> {
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
    pub fn assert_full(&self, e: u32, brick: &Brick, table: &MaterialTable, what: &str) {
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
            vxe_render::internals::class_code(class),
            "{what} class"
        );
    }

    /// Asserts that the table, records and pool hold `coord`'s CPU bricks; returns the first
    /// units of the slots its `Full` bricks occupy.
    pub fn assert_chunk(&self, gpu: &GpuWorld, world: &World, coord: ChunkCoord) -> Vec<u32> {
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
    pub fn assert_lod(&self, gpu: &GpuWorld, world: &World, coord: ChunkCoord) -> Vec<u32> {
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

pub fn full_bricks(world: &World, coords: &[ChunkCoord]) -> u32 {
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
pub fn pressure_terms(gpu: &GpuWorld) -> (f64, f64) {
    (
        gpu.live_units() as f64 / f64::from(gpu.reserved_units()),
        gpu.mirrored_chunks() as f64 / f64::from(gpu.reserved_records()),
    )
}

/// `pressure()` is the larger of its two terms within 1e-6, and between 0 and 1 here.
pub fn assert_pressure(gpu: &GpuWorld) {
    let (units, records) = pressure_terms(gpu);
    let want = units.max(records);
    let got = f64::from(gpu.pressure());
    assert!((got - want).abs() < 1e-6, "pressure {got} against {want}");
    assert!(want > 0.0 && want < 1.0, "{want}");
}

/// Every unit of the pages handed out is live, spare, a free page's, or the tail of a page
/// past its bin's last whole slot: none leaked.
pub fn assert_units_accounted(gpu: &GpuWorld) {
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

/// A table with rows 1..=n named by number.
pub fn numbered_table(n: u16) -> MaterialTable {
    let mut t = MaterialTable::new();
    for i in 1..=n {
        assert_eq!(t.push(Material::named(format!("m{i}"))), i);
    }
    t
}

pub fn limits(binding: u64, buffer: u64, storage_buffers: u32) -> wgpu::Limits {
    wgpu::Limits {
        max_storage_buffer_binding_size: binding,
        max_buffer_size: buffer,
        max_storage_buffers_per_shader_stage: storage_buffers,
        ..wgpu::Limits::default()
    }
}

/// A first sync past the reservation uploads the `uploaded` nearest chunks that fit and
/// defers the other `deferred`, which stay pending with their dirty bits; the next sync,
/// with nothing freed, is empty: it fails with `err` and writes nothing.
pub fn partial_then_empty(
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

/// Every brick of every chunk is `Full`: two materials in a checkerboard, one plane, three
/// units in the 3-unit bin (1365 slots a page).
pub struct Checker;

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

/// A chunk as the store holds it.
pub type SharedChunk = Arc<std::sync::RwLock<Chunk>>;

/// A fixed window over chunks x = 0..12 (y = z = 0) of `Checker`, emptied of every chunk; the
/// chunks are handed back to put in the store, and `Mixed` mips at the same coordinates.
pub fn tier_world() -> (World, Vec<SharedChunk>, Vec<Arc<ChunkMips>>) {
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
pub fn to_lod(world: &World, x: i32, mips: &Arc<ChunkMips>) {
    let c = ChunkCoord([x, 0, 0]);
    world.store().remove(c);
    world.insert_lod(c, mips.clone());
}

/// Inserts chunk `x` of the tier world, full.
pub fn to_full(world: &World, x: i32, chunks: &[SharedChunk]) {
    world
        .store()
        .write_lock()
        .insert(ChunkCoord([x, 0, 0]), chunks[x as usize].clone());
}

/// The pages a mirror of `world`, with the field's band slots when `field`, takes on its
/// first sync, focus at x = 0.
pub fn pages_taken(device: &wgpu::Device, queue: &wgpu::Queue, world: &World, field: bool) -> u32 {
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

/// `Floor` at x = 0 (64 `Full` bricks of 2 units), `Checker` elsewhere (512 of 3).
pub struct FloorAndChecker;

impl WorldSource for FloorAndChecker {
    fn chunk(&self, coord: ChunkCoord, out: &mut Chunk) {
        if coord.0[0] == 0 {
            Floor.chunk(coord, out);
        } else {
            Checker.chunk(coord, out);
        }
    }
}

/// A sync clock, from 0, that advances `step` on every read at `on` and holds at the others.
pub fn stepping_clock(step: std::time::Duration, on: SyncPoint) -> vxe_render::SyncClock {
    let mut t = std::time::Duration::ZERO;
    Box::new(move |at| {
        if at == on {
            t += step;
        }
        t
    })
}
