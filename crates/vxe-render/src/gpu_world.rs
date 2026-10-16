//! The GPU mirror of a world's bricks (ADR-0002, ADR-0004): brick pool, brick occupancy
//! masks, chunk records, chunk table and material rows.
//!
//! The mirror holds the resident chunks inside a box-shaped *window* of chunks. A fixed
//! window is a bounded world's extent; a following window is a cube of side `W` centred on
//! the focus chunk, and moves with it. The chunk table is indexed by the chunk coordinate
//! wrapped into the window, so a window that moves by one chunk reuses the table entries of
//! the chunks it leaves; those are cleared in the same sync that moves the window.
//!
//! `sync` visits only chunks that need an upload: chunks the world reports in its dirty-chunk
//! set, and resident chunks the window has just reached. They wait in a pending set ordered
//! by squared chunk distance to the focus chunk, and each sync uploads them nearest first
//! until its byte budget is spent; the chunk that crosses the budget is still uploaded whole,
//! so a sync writes at most the budget plus one chunk.
//!
//! The CPU keeps a copy of every record entry, so `sync` knows which pool slot a brick held
//! before an edit. A brick that becomes `Full` takes a slot from the free list or the next
//! never-used slot; a brick that collapses to `Uniform`, and every `Full` brick of a chunk
//! that leaves the mirror, return their slots to the free list. `Full` bricks are staged and
//! written in runs of consecutive pool slots, records in runs of consecutive records, and
//! table entries in runs of nearby indices, so a first sync issues a few writes per page.
//!
//! Voxels are narrowed to 8 bits (ADR-0004). A `Full` brick takes one 512-byte slot of palette
//! indices, and each record carries its chunk's palette of 256 `u16` materials after its 512
//! entries; index 0 is air in every palette. A palette only grows while its chunk is
//! mirrored, so bricks already uploaded keep their indices. A chunk that needs more than 255
//! non-air materials is *wide*: each `Full` brick takes two slots from an even slot and holds
//! `u16` voxels, and a chunk that turns wide uploads every `Full` brick again.
//!
//! The pool is sharded over up to `MAX_SHARDS` storage buffers (ADR-0004). Slot `s` lives in
//! shard `s / slots_per_shard`; `slots_per_shard` is the largest multiple of a page that fits
//! the device's binding and buffer size limits, so a wide brick's even-aligned pair never
//! straddles two shards. Shards fill in order and only the last one in use grows; shards not
//! yet in use are one-slot placeholders, so the marcher can always bind all of them. Masks,
//! records, table and materials stay one buffer each, the masks indexed by global slot.
//!
//! A LOD-only chunk (ADR-0003) is mirrored from its mips as a LOD record: entries 0..64 hold
//! level 1's mip-bricks, 64..72 level 2's and 72 level 3's, encoded like brick entries and
//! narrowed through the record's palette like voxel bricks. Its table entry carries
//! `LOD_BIT`. A chunk that changes tier gets a new record: its old one and its slots are
//! freed in the same sync.

use std::collections::{BTreeSet, HashMap};
use std::sync::PoisonError;

use vxe_world::{
    AIR, BRICK_VOXELS, Brick, CHUNK_BRICKS, ChunkCoord, ChunkExtent, ChunkMips, MIP_LEVELS,
    MaterialId, MaterialTable, StreamEvents, VoxelCoord, World,
};

/// Pool slots per growth step, and the unit `slots_per_shard` is a multiple of.
pub const POOL_PAGE_SLOTS: u32 = 4096;
/// Most storage buffers the pool is sharded over.
pub const MAX_SHARDS: u32 = 4;
/// Storage buffers the marcher binds besides the pool shards: masks, records, table,
/// materials.
pub const OTHER_STORAGE_BUFFERS: u32 = 4;
/// Bytes of one pool slot: a narrow brick's 512 palette indices, or half a wide brick.
pub const SLOT_BYTES: u64 = BRICK_VOXELS as u64;
/// Bytes of one brick's occupancy mask: 64 bits, low word first.
pub const MASK_BYTES: u64 = 8;
/// Entries in a palette, index 0 being air.
pub const PALETTE_ENTRIES: usize = 256;
/// `u32` words of one chunk record: 512 brick entries, then the palette as `u16` pairs.
pub const RECORD_WORDS: usize = CHUNK_BRICKS + PALETTE_ENTRIES / 2;
/// Bytes of one chunk record.
pub const RECORD_BYTES: u64 = (RECORD_WORDS * 4) as u64;
/// Records per growth step of the record buffer.
const RECORD_PAGE: u32 = 1024;
/// Most slots `sync` stages on the CPU before writing a run: one pool page.
const STAGE_SLOTS: u32 = POOL_PAGE_SLOTS;
/// Most records staged before writing a run: 4 MiB.
const STAGE_RECORDS: u32 = 2048;
/// Changed table entries closer than this many entries are written in one run.
const TABLE_GAP: u32 = 64;
/// Chunk table entry of a chunk with no record.
pub const ABSENT: u32 = u32::MAX;
/// Chunk record entry bit: set, the low bits are a pool slot; clear, they are a material.
pub const FULL_BIT: u32 = 1 << 31;
/// Chunk record entry bit, with `FULL_BIT`: the brick is wide, two slots of `u16` voxels.
pub const WIDE_BIT: u32 = 1 << 30;
/// The pool slot in a `Full` entry.
pub const SLOT_MASK: u32 = WIDE_BIT - 1;
/// Chunk table entry bit: the record is a LOD record. The record index is in the low 30 bits.
pub const LOD_BIT: u32 = 1 << 30;
/// First entry of each mip level in a LOD record, level 1 first.
pub const LOD_LEVEL_ENTRY: [usize; MIP_LEVELS] = [0, 64, 72];
/// `rev` value of a material not in the palette being built.
const NO_INDEX: u16 = u16::MAX;
/// `f32`s per material row: albedo rgb, roughness, emission rgb, flags as bits.
pub const MATERIAL_FLOATS: usize = 8;

/// Why a sync could not complete.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MirrorError {
    /// The pool would need `needed` bytes; the device allows `limit`.
    PoolTooLarge { needed: u64, limit: u64 },
}

impl std::fmt::Display for MirrorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PoolTooLarge { needed, limit } => write!(
                f,
                "brick pool needs {needed} bytes; the device binds at most {limit}"
            ),
        }
    }
}

impl std::error::Error for MirrorError {}

/// What one `sync` did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SyncStats {
    /// Bytes handed to the queue.
    pub bytes: u64,
    /// `Full` bricks written.
    pub bricks: u64,
    /// Chunks whose record was written.
    pub chunks: u64,
    /// Chunks examined: the pending chunks looked at, uploaded or not.
    pub chunks_visited: u64,
    /// Queue writes and copy commands issued.
    pub writes: u64,
    /// Chunks still waiting for an upload after this sync.
    pub pending_chunks: u64,
}

/// Which chunks the mirror holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Window {
    /// A fixed box of chunks, such as a bounded world's extent.
    Fixed(ChunkExtent),
    /// A cube of `side` chunks around the focus chunk, its minimum corner at
    /// `focus - side / 2`.
    Follow { side: i32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MirrorConfig {
    pub window: Window,
    /// Bytes a sync writes before it stops taking chunks; `u64::MAX` for no budget.
    pub sync_budget_bytes: u64,
    /// Caps the slots per pool shard below what the device allows; for tests.
    pub max_shard_slots: Option<u32>,
}

impl MirrorConfig {
    /// A window with no sync budget and no shard cap.
    pub fn new(window: Window) -> Self {
        Self {
            window,
            sync_budget_bytes: u64::MAX,
            max_shard_slots: None,
        }
    }
}

/// How the pool is split over storage buffers on a device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShardLayout {
    /// The largest multiple of `POOL_PAGE_SLOTS` whose bytes fit one binding and one buffer,
    /// at most `max_shard_slots` rounded down to a page.
    pub slots_per_shard: u32,
    /// `min(MAX_SHARDS, max_storage_buffers_per_shader_stage - OTHER_STORAGE_BUFFERS)`.
    pub shards: u32,
}

impl ShardLayout {
    /// The layout `limits` allow. Reads only the limits, never the backend or vendor.
    pub fn from_limits(limits: &wgpu::Limits, max_shard_slots: Option<u32>) -> Self {
        let bytes = limits
            .max_storage_buffer_binding_size
            .min(limits.max_buffer_size);
        let page = u64::from(POOL_PAGE_SLOTS);
        let mut slots = bytes / SLOT_BYTES;
        if let Some(cap) = max_shard_slots {
            slots = slots.min(u64::from(cap));
        }
        let slots_per_shard = ((slots / page) * page).clamp(page, u64::from(u32::MAX / 2)) as u32;
        let shards = limits
            .max_storage_buffers_per_shader_stage
            .saturating_sub(OTHER_STORAGE_BUFFERS)
            .clamp(1, MAX_SHARDS);
        Self {
            slots_per_shard,
            shards,
        }
    }

    /// Slots the pool can hold over all its shards.
    pub fn capacity(&self) -> u64 {
        u64::from(self.slots_per_shard) * u64::from(self.shards)
    }
}

/// What the mirror keeps for a mirrored chunk besides its record entries.
struct Mirrored {
    record: u32,
    palette: Vec<MaterialId>,
    wide: bool,
    /// A LOD record, from the chunk's mips.
    lod: bool,
}

/// What a chosen chunk is uploaded from.
enum Src {
    Full(std::sync::Arc<std::sync::RwLock<vxe_world::Chunk>>),
    Lod(std::sync::Arc<ChunkMips>),
}

/// A chunk chosen for this sync's upload.
struct Selected {
    coord: ChunkCoord,
    src: Src,
}

pub struct GpuWorld {
    device: wgpu::Device,
    cfg: MirrorConfig,
    window_min: [i32; 3],
    window_size: [i32; 3],
    focus: ChunkCoord,
    initialized: bool,
    layout: ShardLayout,
    /// One buffer per shard; a shard not in use is a one-slot placeholder.
    shards: Vec<wgpu::Buffer>,
    /// Slots allocated in each shard, 0 for a placeholder.
    shard_slots: Vec<u32>,
    masks: wgpu::Buffer,
    pool_slots: u32,
    records: wgpu::Buffer,
    record_cap: u32,
    table: wgpu::Buffer,
    materials: wgpu::Buffer,
    materials_written: bool,
    mirrored: HashMap<ChunkCoord, Mirrored>,
    /// CPU copy of every record's entries, `record_cap * RECORD_WORDS`.
    entries: Vec<u32>,
    free_records: Vec<u32>,
    next_record: u32,
    /// CPU copy of the chunk table, and the indices changed since the last write.
    table_cpu: Vec<u32>,
    table_changed: Vec<u32>,
    free: Vec<u32>,
    /// First slots of free slot pairs, from wide bricks.
    free_pairs: Vec<u32>,
    next_slot: u32,
    /// Palette index of every material while one chunk is narrowed, else `NO_INDEX`.
    rev: Vec<u16>,
    /// Chunks waiting for an upload, keyed by squared distance to `pending_focus`.
    pending: BTreeSet<(i64, ChunkCoord)>,
    pending_d2: HashMap<ChunkCoord, i64>,
    pending_focus: ChunkCoord,
    bytes_written: u64,
    /// The run of consecutive pool slots staged for one write: its first slot, its voxels
    /// and its masks. Kept across syncs so staging allocates only while it first grows.
    stage_start: u32,
    stage_voxels: Vec<u8>,
    stage_masks: Vec<u32>,
    /// The run of consecutive records staged for one write.
    rec_stage_start: u32,
    rec_stage: Vec<u32>,
}

impl GpuWorld {
    /// A mirror of a bounded world: the window is `extent`, fixed, with no sync budget.
    pub fn new(device: &wgpu::Device, extent: ChunkExtent) -> Self {
        Self::with_config(device, MirrorConfig::new(Window::Fixed(extent)))
    }

    /// A mirror with an explicit window and sync budget. The pool starts empty.
    pub fn with_config(device: &wgpu::Device, cfg: MirrorConfig) -> Self {
        let layout = ShardLayout::from_limits(&device.limits(), cfg.max_shard_slots);
        let (window_min, window_size, focus) = match cfg.window {
            Window::Fixed(e) => {
                let s = e.size();
                let centre = [0, 1, 2].map(|k| e.min.0[k] + s[k] / 2);
                (e.min.0, s, ChunkCoord(centre))
            }
            Window::Follow { side } => {
                let side = side.max(1);
                ([-(side / 2); 3], [side; 3], ChunkCoord([0; 3]))
            }
        };
        let n = window_size
            .iter()
            .map(|&v| v.max(1) as usize)
            .product::<usize>();
        let storage = wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC;
        let table = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chunk_table"),
            size: (n * 4) as u64,
            usage: storage,
            mapped_at_creation: false,
        });
        Self {
            device: device.clone(),
            cfg,
            window_min,
            window_size,
            focus,
            initialized: false,
            layout,
            shards: (0..MAX_SHARDS)
                .map(|_| Self::create_pool(device, 0))
                .collect(),
            shard_slots: vec![0; MAX_SHARDS as usize],
            masks: Self::create_masks(device, 0),
            pool_slots: 0,
            records: Self::create_records(device, 0),
            record_cap: 0,
            table,
            materials: Self::create_materials(device, 1),
            materials_written: false,
            mirrored: HashMap::new(),
            entries: Vec::new(),
            free_records: Vec::new(),
            next_record: 0,
            table_cpu: vec![ABSENT; n],
            table_changed: Vec::new(),
            free: Vec::new(),
            free_pairs: Vec::new(),
            next_slot: 0,
            rev: vec![NO_INDEX; 1 << 16],
            pending: BTreeSet::new(),
            pending_d2: HashMap::new(),
            pending_focus: focus,
            bytes_written: 0,
            stage_start: 0,
            stage_voxels: Vec::new(),
            stage_masks: Vec::new(),
            rec_stage_start: 0,
            rec_stage: Vec::new(),
        }
    }

    fn storage_buffer(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            // A zero-sized binding is invalid.
            size: size.max(4),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })
    }

    fn create_pool(device: &wgpu::Device, slots: u32) -> wgpu::Buffer {
        Self::storage_buffer(device, "brick_pool", u64::from(slots.max(1)) * SLOT_BYTES)
    }

    fn create_masks(device: &wgpu::Device, slots: u32) -> wgpu::Buffer {
        Self::storage_buffer(device, "brick_masks", u64::from(slots.max(1)) * MASK_BYTES)
    }

    fn create_records(device: &wgpu::Device, records: u32) -> wgpu::Buffer {
        Self::storage_buffer(
            device,
            "chunk_records",
            u64::from(records.max(1)) * RECORD_BYTES,
        )
    }

    fn create_materials(device: &wgpu::Device, rows: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("materials"),
            size: (rows.max(1) * MATERIAL_FLOATS * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// The chunks the mirror can hold now: the window's minimum corner and size.
    pub fn window(&self) -> ChunkExtent {
        ChunkExtent::new(
            self.window_min,
            [0, 1, 2].map(|k| self.window_min[k] + self.window_size[k]),
        )
    }

    /// The focus chunk: uploads run nearest it first, and a following window centres on it.
    pub fn focus(&self) -> ChunkCoord {
        self.focus
    }

    /// Moves the focus to the chunk holding `v`. Takes effect on the next `sync`.
    pub fn set_focus(&mut self, v: VoxelCoord) {
        self.focus = v.chunk();
    }

    /// Pool shard `k`, for `k < MAX_SHARDS`; a shard not in use is a one-slot placeholder.
    pub fn pool_shard(&self, k: usize) -> &wgpu::Buffer {
        &self.shards[k]
    }

    /// How the pool is split on this device.
    pub fn shard_layout(&self) -> ShardLayout {
        self.layout
    }

    /// Shards holding allocated slots.
    pub fn shards_in_use(&self) -> usize {
        self.shard_slots.iter().filter(|&&n| n > 0).count()
    }

    /// Occupancy mask of each pool slot, `[slot][2 x u32]`.
    pub fn masks(&self) -> &wgpu::Buffer {
        &self.masks
    }

    pub fn records(&self) -> &wgpu::Buffer {
        &self.records
    }

    pub fn table(&self) -> &wgpu::Buffer {
        &self.table
    }

    pub fn materials(&self) -> &wgpu::Buffer {
        &self.materials
    }

    /// Pool capacity in slots.
    pub fn pool_slots(&self) -> u32 {
        self.pool_slots
    }

    /// Slots currently holding a `Full` brick or half of a wide one.
    pub fn live_slots(&self) -> u32 {
        self.next_slot - self.free.len() as u32 - 2 * self.free_pairs.len() as u32
    }

    /// Bytes of the pool slots in use.
    pub fn live_pool_bytes(&self) -> u64 {
        u64::from(self.live_slots()) * SLOT_BYTES
    }

    /// The palette the mirror holds for `coord`, index 0 first.
    pub fn palette(&self, coord: ChunkCoord) -> Option<&[MaterialId]> {
        self.mirrored.get(&coord).map(|m| m.palette.as_slice())
    }

    /// Whether `coord` is mirrored wide.
    pub fn is_wide(&self, coord: ChunkCoord) -> bool {
        self.mirrored.get(&coord).is_some_and(|m| m.wide)
    }

    /// Whether `coord` is mirrored from its mips.
    pub fn is_lod(&self, coord: ChunkCoord) -> bool {
        self.mirrored.get(&coord).is_some_and(|m| m.lod)
    }

    /// Mirrored chunks that are wide.
    pub fn wide_chunks(&self) -> usize {
        self.mirrored.values().filter(|m| m.wide).count()
    }

    /// Slots returned and not yet reused, in the order they will be reused last-first.
    pub fn free_slots(&self) -> &[u32] {
        &self.free
    }

    /// Bytes written to the GPU over the mirror's life.
    pub fn bytes_written(&self) -> u64 {
        self.bytes_written
    }

    /// Chunks holding a record.
    pub fn mirrored_chunks(&self) -> usize {
        self.mirrored.len()
    }

    pub fn is_mirrored(&self, coord: ChunkCoord) -> bool {
        self.mirrored.contains_key(&coord)
    }

    /// Chunks waiting for an upload.
    pub fn pending_chunks(&self) -> usize {
        self.pending.len()
    }

    /// The record entry the mirror holds for brick `slot` of chunk `coord`.
    pub fn entry(&self, coord: ChunkCoord, slot: usize) -> Option<u32> {
        let r = self.mirrored.get(&coord)?.record;
        Some(self.entries[r as usize * RECORD_WORDS + slot])
    }

    /// Record index of `coord`, if it is mirrored.
    pub fn record_index(&self, coord: ChunkCoord) -> Option<u32> {
        self.mirrored.get(&coord).map(|m| m.record)
    }

    /// Index of `coord` in the chunk table: the coordinate wrapped into the window size.
    pub fn table_index(&self, c: ChunkCoord) -> u32 {
        let w = self.window_size.map(|v| v.max(1));
        let wr = [0, 1, 2].map(|k| c.0[k].rem_euclid(w[k]) as u32);
        let w = w.map(|v| v as u32);
        wr[0] + w[0] * (wr[1] + w[1] * wr[2])
    }

    fn in_window(&self, c: ChunkCoord) -> bool {
        (0..3).all(|k| {
            c.0[k] >= self.window_min[k] && c.0[k] < self.window_min[k] + self.window_size[k]
        })
    }

    /// Bytes the mirror occupies on the GPU by the uploader's accounting: pool slots in use
    /// with their masks, the records of mirrored chunks, chunk table and material rows.
    pub fn mirror_bytes(&self) -> u64 {
        u64::from(self.live_slots()) * (SLOT_BYTES + MASK_BYTES)
            + self.mirrored.len() as u64 * RECORD_BYTES
            + self.table_cpu.len() as u64 * 4
            + self.materials.size()
    }

    /// Drops the chunks `events` evicted from the mirror. Loaded chunks need nothing here:
    /// the world's dirty-chunk set carries them to the next `sync`.
    pub fn apply_events(&mut self, events: &StreamEvents) {
        for &c in &events.evicted {
            self.drop_chunk(c);
        }
    }

    /// Forgets `coord`: its slots and record go to the free lists and its table entry is
    /// cleared on the next sync.
    fn drop_chunk(&mut self, coord: ChunkCoord) {
        self.unqueue(coord);
        self.drop_record(coord);
    }

    /// Frees `coord`'s record and slots and clears its table entry, leaving it queued.
    fn drop_record(&mut self, coord: ChunkCoord) {
        let Some(Mirrored { record: rec, .. }) = self.mirrored.remove(&coord) else {
            return;
        };
        let base = rec as usize * RECORD_WORDS;
        let mut slots = Vec::new();
        for &e in &self.entries[base..base + CHUNK_BRICKS] {
            if e & FULL_BIT == 0 {
                continue;
            }
            if e & WIDE_BIT != 0 {
                self.free_pairs.push(e & SLOT_MASK);
            } else {
                slots.push(e & SLOT_MASK);
            }
        }
        self.entries[base..base + RECORD_WORDS].fill(0);
        // Pushed highest first, so the next allocations pop them in ascending order and the
        // chunk that reuses them is written in long runs.
        slots.sort_unstable_by(|a, b| b.cmp(a));
        self.free.extend(slots);
        self.free_records.push(rec);
        if self.in_window(coord) {
            let i = self.table_index(coord);
            if self.table_cpu[i as usize] & !LOD_BIT == rec {
                self.table_cpu[i as usize] = ABSENT;
                self.table_changed.push(i);
            }
        }
    }

    fn enqueue(&mut self, c: ChunkCoord) {
        if self.pending_d2.contains_key(&c) {
            return;
        }
        let d = d2(c, self.pending_focus);
        self.pending_d2.insert(c, d);
        self.pending.insert((d, c));
    }

    fn unqueue(&mut self, c: ChunkCoord) {
        if let Some(d) = self.pending_d2.remove(&c) {
            self.pending.remove(&(d, c));
        }
    }

    /// Moves the window to follow the focus, and re-keys the pending set when the focus
    /// chunk changed. Chunks leaving the window are dropped; resident chunks entering it are
    /// queued.
    fn reconcile_window(&mut self, world: &World) {
        if self.pending_focus != self.focus {
            self.pending_focus = self.focus;
            let keys: Vec<ChunkCoord> = self.pending_d2.keys().copied().collect();
            self.pending.clear();
            self.pending_d2.clear();
            for c in keys {
                self.enqueue(c);
            }
        }
        let new_min = match self.cfg.window {
            Window::Fixed(e) => e.min.0,
            Window::Follow { side } => self.focus.0.map(|v| v - side.max(1) / 2),
        };
        if !self.initialized {
            self.initialized = true;
            self.window_min = new_min;
            for c in world.store().coords().into_iter().chain(world.lod_coords()) {
                if self.in_window(c) {
                    self.enqueue(c);
                }
            }
            return;
        }
        if new_min == self.window_min {
            return;
        }
        let old = self.window();
        let new = ChunkExtent::new(new_min, [0, 1, 2].map(|k| new_min[k] + self.window_size[k]));
        let leaving: Vec<ChunkCoord> = self
            .mirrored
            .keys()
            .chain(self.pending_d2.keys())
            .copied()
            .filter(|&c| !new.contains(c))
            .collect();
        // Dropped while the window is still the old one, so their table entries are cleared;
        // the entering chunks wrap onto the same entries.
        for c in leaving {
            self.drop_chunk(c);
        }
        self.window_min = new_min;
        for c in box_difference(new, old) {
            if world.store().contains(c) || world.lod(c).is_some() {
                self.enqueue(c);
            }
        }
    }

    /// Uploads what the pending chunks need, nearest the focus first, until the byte budget
    /// is spent, and clears their dirty bits. Takes one chunk lock at a time.
    ///
    /// Fails with `PoolTooLarge`, writing nothing, when the chosen chunks' new `Full` bricks
    /// would not fit the largest pool the device can bind.
    pub fn sync(&mut self, world: &World, queue: &wgpu::Queue) -> Result<SyncStats, MirrorError> {
        let mut st = SyncStats::default();
        self.reconcile_window(world);
        for c in world.store().take_dirty() {
            if self.in_window(c) {
                self.enqueue(c);
            }
        }

        let (selected, new_full) = self.select(world, &mut st);
        let limit = self.pool_limit_slots();
        let peak = u64::from(self.live_slots()) + new_full;
        if peak > limit {
            return Err(MirrorError::PoolTooLarge {
                needed: peak * SLOT_BYTES,
                limit: limit * SLOT_BYTES,
            });
        }

        // Grow once to what the chosen chunks can need, rather than page by page.
        let reach = u64::from(self.next_slot) + new_full.saturating_sub(self.free.len() as u64);
        self.grow_to(reach.min(limit), queue, &mut st)?;
        if !self.materials_written {
            self.write_materials(world.materials(), queue, &mut st);
            self.materials_written = true;
        }
        let result = self.upload(selected, queue, &mut st);
        // Staged runs are written even after a failed allocation: their records name them.
        self.flush_stage(queue, &mut st);
        self.flush_records(queue, &mut st);
        self.flush_table(queue, &mut st);
        st.pending_chunks = self.pending.len() as u64;
        self.bytes_written += st.bytes;
        result.map(|()| st)
    }

    /// Walks the pending set nearest first and picks chunks until the estimated bytes reach
    /// the budget. Returns them with the count of bricks among them that need a new slot.
    fn select(&mut self, world: &World, st: &mut SyncStats) -> (Vec<Selected>, u64) {
        let budget = self.cfg.sync_budget_bytes;
        // Writes that happen whatever is chosen count against the budget first.
        let mut bytes = self.table_changed.len() as u64 * 4;
        if !self.materials_written {
            bytes += (world.materials().len() * MATERIAL_FLOATS * 4) as u64;
        }
        let mut selected = Vec::new();
        let mut new_full = 0u64;
        let mut stale = Vec::new();
        for &(_, coord) in &self.pending {
            if bytes >= budget && !selected.is_empty() {
                break;
            }
            st.chunks_visited += 1;
            // The full chunk wins over mips: a promoted chunk replaces its LOD record.
            let src = match world.store().get(coord) {
                Some(chunk) => Src::Full(chunk),
                None => match world.lod(coord) {
                    Some(mips) => Src::Lod(mips),
                    None => {
                        stale.push(coord);
                        continue;
                    }
                },
            };
            let mirrored = self.mirrored.get(&coord);
            // A record of the other tier is replaced whole.
            let (rec, wide) = match mirrored {
                Some(m) if m.lod == matches!(src, Src::Lod(_)) => (Some(m.record), m.wide),
                _ => (None, false),
            };
            // A chunk that turns wide during upload is estimated narrow here: its upload, and
            // the slots it needs, can exceed this estimate (ADR-0004's rare path).
            let (slots, slot_bytes) = if wide { (2, 2) } else { (1, 1) };
            let mut chunk_bytes = RECORD_BYTES + if rec.is_none() { 4 } else { 0 };
            match &src {
                Src::Full(chunk) => {
                    let ch = chunk.read().unwrap_or_else(PoisonError::into_inner);
                    if rec.is_some() && !ch.any_dirty() {
                        stale.push(coord);
                        continue;
                    }
                    for slot in 0..CHUNK_BRICKS {
                        if rec.is_some() && !ch.is_dirty(slot) {
                            continue;
                        }
                        if ch.brick(slot).is_full() {
                            chunk_bytes += slot_bytes * (SLOT_BYTES + MASK_BYTES);
                            let old =
                                rec.map_or(0, |r| self.entries[r as usize * RECORD_WORDS + slot]);
                            if old & FULL_BIT == 0 {
                                new_full += slots;
                            }
                        }
                    }
                }
                Src::Lod(mips) => {
                    // Mips never change: a chunk already mirrored LOD-only needs nothing.
                    if rec.is_some() {
                        stale.push(coord);
                        continue;
                    }
                    let full = mips.full_bricks() as u64;
                    chunk_bytes += full * (SLOT_BYTES + MASK_BYTES);
                    new_full += full;
                }
            }
            bytes += chunk_bytes;
            selected.push(Selected { coord, src });
        }
        for c in stale {
            self.unqueue(c);
        }
        (selected, new_full)
    }

    fn upload(
        &mut self,
        selected: Vec<Selected>,
        queue: &wgpu::Queue,
        st: &mut SyncStats,
    ) -> Result<(), MirrorError> {
        for Selected { coord, src } in selected {
            let lod = matches!(src, Src::Lod(_));
            if self.mirrored.get(&coord).is_some_and(|m| m.lod != lod) {
                self.drop_record(coord);
            }
            match src {
                Src::Full(chunk) => {
                    let mut ch = chunk.write().unwrap_or_else(PoisonError::into_inner);
                    {
                        let bricks: Vec<&Brick> = ch.bricks().iter().collect();
                        let dirty: Vec<bool> = (0..CHUNK_BRICKS).map(|s| ch.is_dirty(s)).collect();
                        self.upload_record(coord, &bricks, &dirty, false, queue, st)?;
                    }
                    ch.clear_dirty();
                }
                Src::Lod(mips) => {
                    let bricks: Vec<&Brick> =
                        (1..=MIP_LEVELS).flat_map(|n| mips.level(n)).collect();
                    let dirty = vec![true; bricks.len()];
                    self.upload_record(coord, &bricks, &dirty, true, queue, st)?;
                }
            }
        }
        Ok(())
    }

    /// Writes one chunk's record: `bricks` into its entries (512 voxel bricks, or a LOD
    /// record's 73 mip-bricks), the ones `dirty` marks unless the record is new or turns
    /// wide, then its palette and, for a new record, its table entry.
    fn upload_record(
        &mut self,
        coord: ChunkCoord,
        bricks: &[&Brick],
        dirty: &[bool],
        lod: bool,
        queue: &wgpu::Queue,
        st: &mut SyncStats,
    ) -> Result<(), MirrorError> {
        let (rec, mut palette, was_wide, new) = match self.mirrored.remove(&coord) {
            Some(m) => (m.record, m.palette, m.wide, false),
            None => (self.alloc_record(queue, st), vec![AIR], false, true),
        };
        let base = rec as usize * RECORD_WORDS;
        if new {
            self.entries[base..base + RECORD_WORDS].fill(0);
        }
        for (i, &m) in palette.iter().enumerate() {
            self.rev[usize::from(m)] = i as u16;
        }
        // Append the uploaded bricks' new materials; past 255 non-air the chunk is wide.
        let mut wide = was_wide;
        if !wide {
            'scan: for (i, brick) in bricks.iter().enumerate() {
                if !new && !dirty[i] {
                    continue;
                }
                let Brick::Full(v) = brick else {
                    continue;
                };
                for &m in v.iter() {
                    if self.rev[usize::from(m)] == NO_INDEX {
                        if palette.len() == PALETTE_ENTRIES {
                            wide = true;
                            break 'scan;
                        }
                        self.rev[usize::from(m)] = palette.len() as u16;
                        palette.push(m);
                    }
                }
            }
        }
        let every = new || wide != was_wide;
        let mut result = Ok(());
        for (i, brick) in bricks.iter().enumerate() {
            if !every && !dirty[i] {
                continue;
            }
            result = self.upload_brick(base + i, brick, wide, queue, st);
            if result.is_err() {
                break;
            }
        }
        for &m in &palette {
            self.rev[usize::from(m)] = NO_INDEX;
        }
        for (k, w) in self.entries[base + CHUNK_BRICKS..base + RECORD_WORDS]
            .iter_mut()
            .enumerate()
        {
            let at = |i: usize| u32::from(palette.get(i).copied().unwrap_or(AIR));
            *w = at(2 * k) | at(2 * k + 1) << 16;
        }
        self.mirrored.insert(
            coord,
            Mirrored {
                record: rec,
                palette,
                wide,
                lod,
            },
        );
        self.stage_record(rec, queue, st);
        st.bytes += RECORD_BYTES;
        if new {
            let t = self.table_index(coord);
            self.table_cpu[t as usize] = if lod { rec | LOD_BIT } else { rec };
            self.table_changed.push(t);
        }
        result?;
        st.chunks += 1;
        self.unqueue(coord);
        Ok(())
    }
    /// Writes `brick` into record entry `i`: a narrow brick as palette indices through `rev`,
    /// a wide one as two slots of `u16` voxels, a uniform one as its material. A slot of the
    /// wrong width is returned and a new one taken.
    fn upload_brick(
        &mut self,
        i: usize,
        brick: &Brick,
        wide: bool,
        queue: &wgpu::Queue,
        st: &mut SyncStats,
    ) -> Result<(), MirrorError> {
        let old = self.entries[i];
        let old_slot = (old & FULL_BIT != 0).then_some((old & SLOT_MASK, old & WIDE_BIT != 0));
        let Brick::Full(v) = brick else {
            if let Some(s) = old_slot {
                self.free_slot(s);
            }
            self.entries[i] = u32::from(brick.get(0));
            return Ok(());
        };
        let mask = brick.occupancy_mask();
        let s = match old_slot {
            Some((s, w)) if w == wide => s,
            _ => {
                let s = if wide {
                    self.alloc_pair(queue, st)?
                } else {
                    self.alloc(queue, st)?
                };
                if let Some(old) = old_slot {
                    self.free_slot(old);
                }
                s
            }
        };
        if wide {
            let bytes: &[u8] = bytemuck::cast_slice(&v[..]);
            let (lo, hi) = bytes.split_at(SLOT_BYTES as usize);
            self.stage(s, lo, mask, queue, st);
            self.stage(s + 1, hi, 0, queue, st);
            st.bytes += 2 * (SLOT_BYTES + MASK_BYTES);
            self.entries[i] = FULL_BIT | WIDE_BIT | s;
        } else {
            let mut idx = [0u8; BRICK_VOXELS];
            for (d, &m) in idx.iter_mut().zip(v.iter()) {
                *d = self.rev[usize::from(m)] as u8;
            }
            self.stage(s, &idx, mask, queue, st);
            st.bytes += SLOT_BYTES + MASK_BYTES;
            self.entries[i] = FULL_BIT | s;
        }
        st.bricks += 1;
        Ok(())
    }

    /// Returns a slot, or a wide brick's slot pair, to its free list.
    fn free_slot(&mut self, (s, wide): (u32, bool)) {
        if wide {
            self.free_pairs.push(s);
        } else {
            self.free.push(s);
        }
    }

    /// Appends slot `s` to the staged run, first writing the run out if `s` does not extend it
    /// or it is full.
    fn stage(&mut self, s: u32, bytes: &[u8], mask: u64, queue: &wgpu::Queue, st: &mut SyncStats) {
        let len = self.stage_masks.len() as u32 / 2;
        let boundary = s.is_multiple_of(self.layout.slots_per_shard);
        if len > 0 && (s != self.stage_start + len || len >= STAGE_SLOTS || boundary) {
            self.flush_stage(queue, st);
        }
        if self.stage_masks.is_empty() {
            self.stage_start = s;
        }
        self.stage_voxels.extend_from_slice(bytes);
        self.stage_masks
            .extend_from_slice(&[mask as u32, (mask >> 32) as u32]);
    }

    /// Writes the staged run into the current pool and masks buffers and empties it. A pool
    /// grown since the run began holds its slots too, so the run always targets the live
    /// buffers.
    fn flush_stage(&mut self, queue: &wgpu::Queue, st: &mut SyncStats) {
        if self.stage_masks.is_empty() {
            return;
        }
        let s = u64::from(self.stage_start);
        let sps = self.layout.slots_per_shard;
        let shard = (self.stage_start / sps) as usize;
        queue.write_buffer(
            &self.shards[shard],
            u64::from(self.stage_start % sps) * SLOT_BYTES,
            bytemuck::cast_slice(&self.stage_voxels),
        );
        queue.write_buffer(
            &self.masks,
            s * MASK_BYTES,
            bytemuck::cast_slice(&self.stage_masks),
        );
        st.writes += 2;
        self.stage_voxels.clear();
        self.stage_masks.clear();
    }

    /// Appends record `rec`'s CPU entries to the staged run of records.
    fn stage_record(&mut self, rec: u32, queue: &wgpu::Queue, st: &mut SyncStats) {
        let len = (self.rec_stage.len() / RECORD_WORDS) as u32;
        if len > 0 && (rec != self.rec_stage_start + len || len >= STAGE_RECORDS) {
            self.flush_records(queue, st);
        }
        if self.rec_stage.is_empty() {
            self.rec_stage_start = rec;
        }
        let base = rec as usize * RECORD_WORDS;
        self.rec_stage
            .extend_from_slice(&self.entries[base..base + RECORD_WORDS]);
    }

    fn flush_records(&mut self, queue: &wgpu::Queue, st: &mut SyncStats) {
        if self.rec_stage.is_empty() {
            return;
        }
        queue.write_buffer(
            &self.records,
            u64::from(self.rec_stage_start) * RECORD_BYTES,
            bytemuck::cast_slice(&self.rec_stage),
        );
        st.writes += 1;
        self.rec_stage.clear();
    }

    /// Writes the changed table entries in runs; entries closer than `TABLE_GAP` share a run.
    fn flush_table(&mut self, queue: &wgpu::Queue, st: &mut SyncStats) {
        if self.table_changed.is_empty() {
            return;
        }
        let mut idx = std::mem::take(&mut self.table_changed);
        idx.sort_unstable();
        idx.dedup();
        let mut start = idx[0];
        let mut end = idx[0];
        for &i in &idx[1..] {
            if i - end > TABLE_GAP {
                self.write_table_run(start, end, queue, st);
                start = i;
            }
            end = i;
        }
        self.write_table_run(start, end, queue, st);
        idx.clear();
        self.table_changed = idx;
    }

    fn write_table_run(&self, start: u32, end: u32, queue: &wgpu::Queue, st: &mut SyncStats) {
        let run = &self.table_cpu[start as usize..=end as usize];
        queue.write_buffer(&self.table, u64::from(start) * 4, bytemuck::cast_slice(run));
        st.writes += 1;
        st.bytes += run.len() as u64 * 4;
    }

    fn write_materials(&mut self, table: &MaterialTable, queue: &wgpu::Queue, st: &mut SyncStats) {
        let rows: Vec<[f32; MATERIAL_FLOATS]> = table
            .rows()
            .iter()
            .map(|m| {
                [
                    m.albedo[0],
                    m.albedo[1],
                    m.albedo[2],
                    m.roughness,
                    m.emission[0],
                    m.emission[1],
                    m.emission[2],
                    f32::from_bits(m.flags.0),
                ]
            })
            .collect();
        if self.materials.size() < (rows.len() * MATERIAL_FLOATS * 4) as u64 {
            self.materials = Self::create_materials(&self.device, rows.len());
        }
        let bytes: &[u8] = bytemuck::cast_slice(&rows);
        queue.write_buffer(&self.materials, 0, bytes);
        st.writes += 1;
        st.bytes += bytes.len() as u64;
    }

    fn alloc_record(&mut self, queue: &wgpu::Queue, st: &mut SyncStats) -> u32 {
        if let Some(r) = self.free_records.pop() {
            return r;
        }
        if self.next_record >= self.record_cap {
            // Half again at least, as the pool grows.
            let want = (self.next_record + 1).max(self.record_cap * 3 / 2);
            let cap = want.div_ceil(RECORD_PAGE) * RECORD_PAGE;
            let records = Self::create_records(&self.device, cap);
            if self.record_cap > 0 {
                let mut enc = self
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("grow records"),
                    });
                enc.copy_buffer_to_buffer(
                    &self.records,
                    0,
                    &records,
                    0,
                    u64::from(self.record_cap) * RECORD_BYTES,
                );
                queue.submit([enc.finish()]);
                st.writes += 1;
            }
            self.records = records;
            self.record_cap = cap;
            self.entries.resize(cap as usize * RECORD_WORDS, 0);
        }
        self.next_record += 1;
        self.next_record - 1
    }

    /// Slots the pool can hold over all its shards.
    fn pool_limit_slots(&self) -> u64 {
        self.layout.capacity()
    }

    fn alloc(&mut self, queue: &wgpu::Queue, st: &mut SyncStats) -> Result<u32, MirrorError> {
        if let Some(s) = self.free.pop() {
            return Ok(s);
        }
        if self.next_slot >= self.pool_slots {
            self.grow_to(u64::from(self.next_slot) + 1, queue, st)?;
        }
        self.next_slot += 1;
        Ok(self.next_slot - 1)
    }

    /// Two adjacent slots starting on an even slot, for a wide brick. An odd next slot is
    /// skipped onto the single free list.
    fn alloc_pair(&mut self, queue: &wgpu::Queue, st: &mut SyncStats) -> Result<u32, MirrorError> {
        if let Some(s) = self.free_pairs.pop() {
            return Ok(s);
        }
        let start = self.next_slot + self.next_slot % 2;
        if start + 2 > self.pool_slots {
            self.grow_to(u64::from(start) + 2, queue, st)?;
        }
        if start != self.next_slot {
            self.free.push(self.next_slot);
        }
        self.next_slot = start + 2;
        Ok(start)
    }

    /// Grows the pool to hold at least `slots` and at least half again its size, in whole
    /// pages capped at the device limit, keeping its contents.
    fn grow_to(
        &mut self,
        slots: u64,
        queue: &wgpu::Queue,
        st: &mut SyncStats,
    ) -> Result<(), MirrorError> {
        if slots <= u64::from(self.pool_slots) {
            return Ok(());
        }
        let limit = self.pool_limit_slots();
        if slots > limit {
            return Err(MirrorError::PoolTooLarge {
                needed: slots * SLOT_BYTES,
                limit: limit * SLOT_BYTES,
            });
        }
        // Half again at least, so a pool growing every sync copies its contents a logarithmic
        // number of times rather than once per page.
        let page = u64::from(POOL_PAGE_SLOTS);
        let want = slots.max(u64::from(self.pool_slots) * 3 / 2);
        let new_slots = (want.div_ceil(page).max(1) * page).min(limit);
        let sps = u64::from(self.layout.slots_per_shard);
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("grow pool"),
            });
        let mut copies = 0;
        for k in 0..self.layout.shards as usize {
            let want = new_slots.saturating_sub(k as u64 * sps).min(sps) as u32;
            let have = self.shard_slots[k];
            if want <= have {
                continue;
            }
            let shard = Self::create_pool(&self.device, want);
            if have > 0 {
                enc.copy_buffer_to_buffer(
                    &self.shards[k],
                    0,
                    &shard,
                    0,
                    u64::from(have) * SLOT_BYTES,
                );
                copies += 1;
            }
            self.shards[k] = shard;
            self.shard_slots[k] = want;
        }
        let masks = Self::create_masks(&self.device, new_slots as u32);
        if self.pool_slots > 0 {
            enc.copy_buffer_to_buffer(
                &self.masks,
                0,
                &masks,
                0,
                u64::from(self.pool_slots) * MASK_BYTES,
            );
            copies += 1;
        }
        if copies > 0 {
            queue.submit([enc.finish()]);
            st.writes += copies;
        }
        self.masks = masks;
        self.pool_slots = new_slots as u32;
        Ok(())
    }
}

fn d2(a: ChunkCoord, b: ChunkCoord) -> i64 {
    (0..3)
        .map(|k| (i64::from(a.0[k]) - i64::from(b.0[k])).pow(2))
        .sum()
}

/// The chunks of `a` that are not in `b`.
fn box_difference(a: ChunkExtent, b: ChunkExtent) -> Vec<ChunkCoord> {
    let inside = |k: usize, v: i32| v >= b.min.0[k] && v < b.max.0[k];
    let mut out = Vec::new();
    for x in a.min.0[0]..a.max.0[0] {
        for y in a.min.0[1]..a.max.0[1] {
            if inside(0, x) && inside(1, y) {
                for z in a.min.0[2]..a.max.0[2] {
                    if !inside(2, z) {
                        out.push(ChunkCoord([x, y, z]));
                    }
                }
            } else {
                for z in a.min.0[2]..a.max.0[2] {
                    out.push(ChunkCoord([x, y, z]));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_difference_is_the_entering_slab() {
        let a = ChunkExtent::new([1, 0, 0], [4, 3, 3]);
        let b = ChunkExtent::new([0, 0, 0], [3, 3, 3]);
        let d = box_difference(a, b);
        assert_eq!(d.len(), 9);
        assert!(d.iter().all(|c| c.0[0] == 3));
        assert!(box_difference(b, b).is_empty());
        assert_eq!(
            box_difference(a, ChunkExtent::new([9, 9, 9], [10, 10, 10])).len(),
            27
        );
    }
}
