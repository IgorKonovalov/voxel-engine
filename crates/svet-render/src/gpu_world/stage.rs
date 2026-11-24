//! The writes: the mirror's buffers, the staged runs of slots, records and table entries,
//! and the staging belt they are copied through (ADR-0022).

use super::*;

/// Most units `sync` stages on the CPU before writing a run: 2 MiB.
pub(super) const STAGE_UNITS: u32 = 8 * PAGE_UNITS;
/// Most records staged before writing a run: 4 MiB.
pub(super) const STAGE_RECORDS: u32 = 2048;
/// Changed table entries closer than this many entries are written in one run.
pub(super) const TABLE_GAP: u32 = 64;
/// Bytes of one staging-belt chunk (ADR-0022): above a 2 MiB sync budget plus a frame's
/// records, so a budgeted sync normally fills one chunk.
pub const BELT_CHUNK_BYTES: u64 = 4 << 20;
/// Chunks the belt keeps across syncs. A sync that leaves it holding more (an unbudgeted
/// first sync of a large world) replaces it with an empty one, so the chunks of that burst
/// are released once their copies complete.
pub const BELT_KEEP_CHUNKS: usize = 3;

/// The mirror's buffers and the staged runs written into them through the belt.
pub(super) struct Uploader {
    /// One buffer per shard, created at its reserved size; a shard holding no reserved unit
    /// is a one-unit placeholder.
    pub(super) shards: Vec<wgpu::Buffer>,
    pub(super) records: wgpu::Buffer,
    pub(super) table: wgpu::Buffer,
    pub(super) materials: wgpu::Buffer,
    pub(super) belt: Belt,
    /// Per bin, the run of consecutive pool units staged for one write: its first unit and
    /// its words. A fresh chunk's bricks interleave bins but take consecutive slots of each,
    /// so each bin's run stays long. Kept across syncs so staging allocates only while it
    /// first grows.
    pub(super) stage: [(u32, Vec<u32>); BIN_COUNT],
    /// One slot's words as `upload_brick` builds them.
    pub(super) slot_words: Vec<u32>,
    /// The run of consecutive records staged for one write.
    pub(super) rec_start: u32,
    pub(super) rec_stage: Vec<u32>,
    pub(super) bytes_written: u64,
}

impl Uploader {
    /// The buffers of `layout`'s reservation beside the chunk `table`, one material row.
    pub(super) fn new(device: &wgpu::Device, layout: ShardLayout, table: wgpu::Buffer) -> Self {
        let ups = u64::from(layout.units_per_shard);
        let reserved = u64::from(layout.reserved_units);
        Self {
            shards: (0..u64::from(MAX_SHARDS))
                .map(|k| GpuWorld::create_pool(device, shard_units(reserved, ups, k) as u32))
                .collect(),
            records: GpuWorld::create_records(device, layout.reserved_records),
            table,
            materials: GpuWorld::create_materials(device, 1),
            belt: Belt::new(device),
            stage: Default::default(),
            slot_words: Vec::new(),
            rec_start: 0,
            rec_stage: Vec::new(),
            bytes_written: 0,
        }
    }
}

/// The field surface's band slots while it is on ([`GpuWorld::set_field_surface`]), and what
/// changed for it in this sync: chunks written whole, dirty bricks (world brick coordinates)
/// and chunks whose record was written new, dropped or replaced.
#[derive(Default)]
pub(super) struct FieldState {
    pub(super) band: Option<Band>,
    pub(super) chunks: HashSet<ChunkCoord>,
    pub(super) bricks: HashSet<[i32; 3]>,
    pub(super) shells: HashSet<ChunkCoord>,
    /// The band masks of the chunk being selected or uploaded, by brick slot
    /// ([`GpuWorld::need`]).
    pub(super) bands: Vec<Option<[u64; 8]>>,
}

/// The field surface's band slots (ADR-0041, module docs) and the pass that fills them.
pub(super) struct Band {
    pub(super) pass: crate::field::FieldPass,
    /// Each full chunk's band slots by brick slot: the slot's first unit, or `ABSENT`.
    pub(super) slots: HashMap<ChunkCoord, Box<[u32; CHUNK_BRICKS]>>,
    /// The field pass's work list: band slots to fill, with their brick's origin in voxels.
    pub(super) items: HashMap<u32, [i32; 3]>,
    /// The bricks of each chunk written in this sync, whose band is current.
    pub(super) fresh: HashMap<ChunkCoord, [u64; 8]>,
    /// Bricks (world brick coordinates) whose band slot could not be placed, tried again at
    /// the next sync.
    pub(super) retry: HashSet<[i32; 3]>,
}

impl Band {
    pub(super) fn slot(&self, c: ChunkCoord, i: usize) -> u32 {
        self.slots.get(&c).map_or(ABSENT, |s| s[i])
    }

    pub(super) fn is_fresh(&self, c: ChunkCoord, i: usize) -> bool {
        self.fresh
            .get(&c)
            .is_some_and(|m| m[i / 64] >> (i % 64) & 1 != 0)
    }
}

/// The mirror's staging belt: `wgpu`'s mapped chunks sub-allocated per staged run, each run
/// copied to its destination by a command recorded into the sync's encoder (ADR-0022).
pub(super) struct Belt {
    pub(super) device: wgpu::Device,
    pub(super) belt: wgpu::util::StagingBelt,
    /// Every chunk buffer the current belt has handed out a slice of: the belt never drops a
    /// chunk, so this is its chunk count, active, closed, in flight and free.
    pub(super) chunks: Vec<wgpu::Buffer>,
    /// Times the belt was replaced for holding more than [`BELT_KEEP_CHUNKS`].
    pub(super) trims: u64,
}

impl Belt {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        Self {
            device: device.clone(),
            belt: wgpu::util::StagingBelt::new(device.clone(), BELT_CHUNK_BYTES),
            chunks: Vec::new(),
            trims: 0,
        }
    }

    /// Copies `bytes` into a belt slice and records the slice's copy to `target` at `offset`.
    /// `offset` and the length are multiples of `COPY_BUFFER_ALIGNMENT`, as every mirror run
    /// is (whole `u32` words).
    pub(super) fn write(
        &mut self,
        enc: &mut wgpu::CommandEncoder,
        target: &wgpu::Buffer,
        offset: u64,
        bytes: &[u8],
    ) {
        let Some(size) = wgpu::BufferSize::new(bytes.len() as u64) else {
            return;
        };
        let align = wgpu::BufferSize::new(wgpu::COPY_BUFFER_ALIGNMENT).expect("non-zero");
        let slice = self.belt.allocate(size, align);
        let chunk = slice.buffer();
        if !self.chunks.contains(chunk) {
            self.chunks.push(chunk.clone());
        }
        enc.copy_buffer_to_buffer(chunk, slice.offset(), target, offset, size.get());
        slice
            .get_mapped_range_mut()
            .expect("a fresh belt slice is mapped")
            .copy_from_slice(bytes);
    }

    /// Closes the chunks written since the last call; they are re-mapped for the belt once
    /// `enc`'s submission completes and the device is polled. Then, past
    /// [`BELT_KEEP_CHUNKS`], replaces the belt: the old one's free chunks drop now and its
    /// in-flight ones when their copies complete.
    pub(super) fn finish(&mut self, enc: &wgpu::CommandEncoder) {
        self.belt.finish_and_recall_on_submit(enc);
        if self.chunks.len() > BELT_KEEP_CHUNKS {
            self.belt = wgpu::util::StagingBelt::new(self.device.clone(), BELT_CHUNK_BYTES);
            self.chunks.clear();
            self.trims += 1;
        }
    }
}

/// The GPU rows of `table`, [`MATERIAL_FLOATS`] each.
pub(super) fn material_rows(table: &MaterialTable) -> Vec<[f32; MATERIAL_FLOATS]> {
    table
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
                0.0,
                0.0,
                m.opacity,
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
            ]
        })
        .collect()
}

impl GpuWorld {
    pub(super) fn storage_buffer(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
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

    pub(super) fn create_pool(device: &wgpu::Device, units: u32) -> wgpu::Buffer {
        Self::storage_buffer(device, "brick_pool", u64::from(units.max(1)) * UNIT_BYTES)
    }

    pub(super) fn create_records(device: &wgpu::Device, records: u32) -> wgpu::Buffer {
        Self::storage_buffer(
            device,
            "chunk_records",
            u64::from(records.max(1)) * RECORD_BYTES,
        )
    }

    pub(super) fn create_materials(device: &wgpu::Device, rows: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("materials"),
            size: (rows.max(1) * MATERIAL_FLOATS * 4) as u64,
            // COPY_SRC for readback, as the mirror's other storage buffers.
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })
    }

    /// Appends the slot at unit `u` of bin `k`, whose words are `words` (whole units), to the
    /// bin's staged run, first writing the run out if `u` does not extend it, it is full, or
    /// `u` starts a shard.
    pub(super) fn stage(
        &mut self,
        k: usize,
        u: u32,
        words: &[u32],
        enc: &mut wgpu::CommandEncoder,
        st: &mut SyncStats,
    ) {
        let (start, run) = &self.up.stage[k];
        let len = (run.len() / UNIT_WORDS) as u32;
        let boundary = u.is_multiple_of(self.pool.layout.units_per_shard);
        if len > 0 && (u != start + len || len >= STAGE_UNITS || boundary) {
            self.flush_stage(k, enc, st);
        }
        let (start, run) = &mut self.up.stage[k];
        if run.is_empty() {
            *start = u;
        }
        run.extend_from_slice(words);
    }

    /// Writes bin `k`'s staged run into its pool shard, and empties it.
    pub(super) fn flush_stage(
        &mut self,
        k: usize,
        enc: &mut wgpu::CommandEncoder,
        st: &mut SyncStats,
    ) {
        let (start, run) = &mut self.up.stage[k];
        if run.is_empty() {
            return;
        }
        let ups = self.pool.layout.units_per_shard;
        let shard = (*start / ups) as usize;
        self.up.belt.write(
            enc,
            &self.up.shards[shard],
            u64::from(*start % ups) * UNIT_BYTES,
            bytemuck::cast_slice(run),
        );
        st.writes += 1;
        run.clear();
    }

    /// Appends record `rec`'s CPU entries to the staged run of records.
    pub(super) fn stage_record(
        &mut self,
        rec: u32,
        enc: &mut wgpu::CommandEncoder,
        st: &mut SyncStats,
    ) {
        let left = self.lap(Stage::Copy);
        let len = (self.up.rec_stage.len() / RECORD_WORDS) as u32;
        if len > 0 && (rec != self.up.rec_start + len || len >= STAGE_RECORDS) {
            self.flush_records(enc, st);
        }
        if self.up.rec_stage.is_empty() {
            self.up.rec_start = rec;
        }
        let base = rec as usize * RECORD_WORDS;
        self.up
            .rec_stage
            .extend_from_slice(&self.recs.entries[base..base + RECORD_WORDS]);
        self.lap(left);
    }

    pub(super) fn flush_records(&mut self, enc: &mut wgpu::CommandEncoder, st: &mut SyncStats) {
        if self.up.rec_stage.is_empty() {
            return;
        }
        self.up.belt.write(
            enc,
            &self.up.records,
            u64::from(self.up.rec_start) * RECORD_BYTES,
            bytemuck::cast_slice(&self.up.rec_stage),
        );
        st.writes += 1;
        self.up.rec_stage.clear();
    }

    /// Writes the changed table entries in runs; entries closer than `TABLE_GAP` share a run.
    pub(super) fn flush_table(&mut self, enc: &mut wgpu::CommandEncoder, st: &mut SyncStats) {
        if self.window.table_changed.is_empty() {
            return;
        }
        let mut idx = std::mem::take(&mut self.window.table_changed);
        idx.sort_unstable();
        idx.dedup();
        let mut start = idx[0];
        let mut end = idx[0];
        for &i in &idx[1..] {
            if i - end > TABLE_GAP {
                self.write_table_run(start, end, enc, st);
                start = i;
            }
            end = i;
        }
        self.write_table_run(start, end, enc, st);
        idx.clear();
        self.window.table_changed = idx;
    }

    pub(super) fn write_table_run(
        &mut self,
        start: u32,
        end: u32,
        enc: &mut wgpu::CommandEncoder,
        st: &mut SyncStats,
    ) {
        let run = &self.window.table[start as usize..=end as usize];
        self.up.belt.write(
            enc,
            &self.up.table,
            u64::from(start) * 4,
            bytemuck::cast_slice(run),
        );
        st.writes += 1;
        st.bytes += run.len() as u64 * 4;
    }

    pub(super) fn write_materials(
        &mut self,
        table: &MaterialTable,
        enc: &mut wgpu::CommandEncoder,
        st: &mut SyncStats,
    ) {
        let rows = material_rows(table);
        // The material rows are outside the reservation: sized to the table at the first sync.
        if self.up.materials.size() < (rows.len() * MATERIAL_FLOATS * 4) as u64 {
            self.up.materials = Self::create_materials(&self.device, rows.len());
        }
        self.mats.rows = rows.len() as u64;
        self.mats.crisp = (0..table.len()).any(|i| {
            let id = i as MaterialId;
            table.is_crisp(id) && !table.is_translucent(id)
        });
        let bytes: &[u8] = bytemuck::cast_slice(&rows);
        self.up.belt.write(enc, &self.up.materials, 0, bytes);
        st.writes += 1;
        st.bytes += bytes.len() as u64;
    }
}
