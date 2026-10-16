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

use std::collections::{BTreeSet, HashMap};
use std::sync::PoisonError;

use vxe_world::{
    BRICK_VOXELS, Brick, CHUNK_BRICKS, ChunkCoord, ChunkExtent, MaterialTable, StreamEvents,
    VoxelCoord, World,
};

/// Pool slots per growth step.
pub const POOL_PAGE_SLOTS: u32 = 4096;
/// Bytes of one brick's voxels in the pool.
pub const BRICK_BYTES: u64 = (BRICK_VOXELS * 2) as u64;
/// Bytes of one brick's occupancy mask: 64 bits, low word first.
pub const MASK_BYTES: u64 = 8;
/// `u32` words of one chunk record.
pub const RECORD_WORDS: usize = CHUNK_BRICKS;
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
}

/// A chunk chosen for this sync's upload.
struct Selected {
    coord: ChunkCoord,
    chunk: std::sync::Arc<std::sync::RwLock<vxe_world::Chunk>>,
}

pub struct GpuWorld {
    device: wgpu::Device,
    cfg: MirrorConfig,
    window_min: [i32; 3],
    window_size: [i32; 3],
    focus: ChunkCoord,
    initialized: bool,
    pool: wgpu::Buffer,
    masks: wgpu::Buffer,
    pool_slots: u32,
    records: wgpu::Buffer,
    record_cap: u32,
    table: wgpu::Buffer,
    materials: wgpu::Buffer,
    materials_written: bool,
    /// Record index of every mirrored chunk.
    mirrored: HashMap<ChunkCoord, u32>,
    /// CPU copy of every record's entries, `record_cap * RECORD_WORDS`.
    entries: Vec<u32>,
    free_records: Vec<u32>,
    next_record: u32,
    /// CPU copy of the chunk table, and the indices changed since the last write.
    table_cpu: Vec<u32>,
    table_changed: Vec<u32>,
    free: Vec<u32>,
    next_slot: u32,
    /// Chunks waiting for an upload, keyed by squared distance to `pending_focus`.
    pending: BTreeSet<(i64, ChunkCoord)>,
    pending_d2: HashMap<ChunkCoord, i64>,
    pending_focus: ChunkCoord,
    bytes_written: u64,
    /// The run of consecutive pool slots staged for one write: its first slot, its voxels
    /// and its masks. Kept across syncs so staging allocates only while it first grows.
    stage_start: u32,
    stage_voxels: Vec<u16>,
    stage_masks: Vec<u32>,
    /// The run of consecutive records staged for one write.
    rec_stage_start: u32,
    rec_stage: Vec<u32>,
}

impl GpuWorld {
    /// A mirror of a bounded world: the window is `extent`, fixed, with no sync budget.
    pub fn new(device: &wgpu::Device, extent: ChunkExtent) -> Self {
        Self::with_config(
            device,
            MirrorConfig {
                window: Window::Fixed(extent),
                sync_budget_bytes: u64::MAX,
            },
        )
    }

    /// A mirror with an explicit window and sync budget. The pool starts empty.
    pub fn with_config(device: &wgpu::Device, cfg: MirrorConfig) -> Self {
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
            pool: Self::create_pool(device, 0),
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
            next_slot: 0,
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
        Self::storage_buffer(device, "brick_pool", u64::from(slots.max(1)) * BRICK_BYTES)
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

    pub fn pool(&self) -> &wgpu::Buffer {
        &self.pool
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

    /// Slots currently holding a `Full` brick.
    pub fn live_slots(&self) -> u32 {
        self.next_slot - self.free.len() as u32
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
        let r = *self.mirrored.get(&coord)?;
        Some(self.entries[r as usize * RECORD_WORDS + slot])
    }

    /// Record index of `coord`, if it is mirrored.
    pub fn record_index(&self, coord: ChunkCoord) -> Option<u32> {
        self.mirrored.get(&coord).copied()
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
        u64::from(self.live_slots()) * (BRICK_BYTES + MASK_BYTES)
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
        let Some(rec) = self.mirrored.remove(&coord) else {
            return;
        };
        let base = rec as usize * RECORD_WORDS;
        let mut slots: Vec<u32> = self.entries[base..base + RECORD_WORDS]
            .iter()
            .filter(|&&e| e & FULL_BIT != 0)
            .map(|&e| e & !FULL_BIT)
            .collect();
        self.entries[base..base + RECORD_WORDS].fill(0);
        // Pushed highest first, so the next allocations pop them in ascending order and the
        // chunk that reuses them is written in long runs.
        slots.sort_unstable_by(|a, b| b.cmp(a));
        self.free.extend(slots);
        self.free_records.push(rec);
        if self.in_window(coord) {
            let i = self.table_index(coord);
            if self.table_cpu[i as usize] == rec {
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
            for c in world.store().coords() {
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
            if world.store().contains(c) {
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
                needed: peak * BRICK_BYTES,
                limit: limit * BRICK_BYTES,
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
            let Some(chunk) = world.store().get(coord) else {
                stale.push(coord);
                continue;
            };
            let rec = self.mirrored.get(&coord).copied();
            {
                let ch = chunk.read().unwrap_or_else(PoisonError::into_inner);
                if rec.is_some() && !ch.any_dirty() {
                    stale.push(coord);
                    continue;
                }
                let mut chunk_bytes = RECORD_BYTES;
                if rec.is_none() {
                    chunk_bytes += 4;
                }
                for slot in 0..CHUNK_BRICKS {
                    if rec.is_some() && !ch.is_dirty(slot) {
                        continue;
                    }
                    if ch.brick(slot).is_full() {
                        chunk_bytes += BRICK_BYTES + MASK_BYTES;
                        let old = rec.map_or(0, |r| self.entries[r as usize * RECORD_WORDS + slot]);
                        if old & FULL_BIT == 0 {
                            new_full += 1;
                        }
                    }
                }
                bytes += chunk_bytes;
            }
            selected.push(Selected { coord, chunk });
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
        for Selected { coord, chunk } in selected {
            let mut chunk = chunk.write().unwrap_or_else(PoisonError::into_inner);
            let (rec, new) = match self.mirrored.get(&coord) {
                Some(&r) => (r, false),
                None => (self.alloc_record(queue, st), true),
            };
            let base = rec as usize * RECORD_WORDS;
            if new {
                self.entries[base..base + RECORD_WORDS].fill(0);
            }
            for slot in 0..CHUNK_BRICKS {
                if !new && !chunk.is_dirty(slot) {
                    continue;
                }
                let i = base + slot;
                let old = self.entries[i];
                self.entries[i] = match chunk.brick(slot) {
                    Brick::Full(v) => {
                        let s = if old & FULL_BIT != 0 {
                            old & !FULL_BIT
                        } else {
                            self.alloc(queue, st)?
                        };
                        let mask = chunk.brick(slot).occupancy_mask();
                        self.stage(s, &v[..], mask, queue, st);
                        st.bytes += BRICK_BYTES + MASK_BYTES;
                        st.bricks += 1;
                        FULL_BIT | s
                    }
                    Brick::Uniform(m) => {
                        if old & FULL_BIT != 0 {
                            self.free.push(old & !FULL_BIT);
                        }
                        u32::from(*m)
                    }
                };
            }
            self.stage_record(rec, queue, st);
            st.bytes += RECORD_BYTES;
            if new {
                self.mirrored.insert(coord, rec);
                let t = self.table_index(coord);
                self.table_cpu[t as usize] = rec;
                self.table_changed.push(t);
            }
            st.chunks += 1;
            chunk.clear_dirty();
            self.unqueue(coord);
        }
        Ok(())
    }

    /// Appends slot `s` to the staged run, first writing the run out if `s` does not extend it
    /// or it is full.
    fn stage(
        &mut self,
        s: u32,
        voxels: &[u16],
        mask: u64,
        queue: &wgpu::Queue,
        st: &mut SyncStats,
    ) {
        let len = self.stage_masks.len() as u32 / 2;
        if len > 0 && (s != self.stage_start + len || len >= STAGE_SLOTS) {
            self.flush_stage(queue, st);
        }
        if self.stage_masks.is_empty() {
            self.stage_start = s;
        }
        self.stage_voxels.extend_from_slice(voxels);
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
        queue.write_buffer(
            &self.pool,
            s * BRICK_BYTES,
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
            let cap = (self.next_record + 1).div_ceil(RECORD_PAGE) * RECORD_PAGE;
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

    /// Slots the device can bind in one pool buffer.
    fn pool_limit_slots(&self) -> u64 {
        let limits = self.device.limits();
        limits
            .max_storage_buffer_binding_size
            .min(limits.max_buffer_size)
            / BRICK_BYTES
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

    /// Grows the pool to hold at least `slots`, in whole pages capped at the device limit,
    /// keeping its contents.
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
                needed: slots * BRICK_BYTES,
                limit: limit * BRICK_BYTES,
            });
        }
        let page = u64::from(POOL_PAGE_SLOTS);
        let new_slots = (slots.div_ceil(page).max(1) * page).min(limit);
        let pool = Self::create_pool(&self.device, new_slots as u32);
        let masks = Self::create_masks(&self.device, new_slots as u32);
        if self.pool_slots > 0 {
            let mut enc = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("grow pool"),
                });
            enc.copy_buffer_to_buffer(
                &self.pool,
                0,
                &pool,
                0,
                u64::from(self.pool_slots) * BRICK_BYTES,
            );
            enc.copy_buffer_to_buffer(
                &self.masks,
                0,
                &masks,
                0,
                u64::from(self.pool_slots) * MASK_BYTES,
            );
            queue.submit([enc.finish()]);
            st.writes += 2;
        }
        self.pool = pool;
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
