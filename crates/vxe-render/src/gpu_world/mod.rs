//! The GPU mirror of a world's bricks (ADR-0002, ADR-0004, ADR-0012): brick pool, chunk
//! records, chunk table and material rows.
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
//! The CPU keeps a copy of every record entry, so `sync` knows which slot a brick held before
//! an edit. A `Full` brick's slot is its packed CPU bytes (ADR-0012), a run of 64-byte *units*
//! in the pool, nothing narrowed:
//!
//! ```text
//! unit 0          the opaque mask: bit i % 32 of word i / 32 set when voxel i (x + 8 y + 64 z)
//!                 is neither air nor TRANSLUCENT; layer z is words 2 z (low) and 2 z + 1
//! unit 1          the header (HEADER_* words): the brick's palette of up to 16 u16 ids,
//!                 ascending, entry j in half j % 2 of word j / 2; the two translucent words
//!                 (bit bx + 4 by + 16 bz set when 2^3 block (bx, by, bz) holds a TRANSLUCENT
//!                 voxel); the band prefix and the band slot
//! units 2..       the payload: 0, 1, 2 or 4 bit planes in the mask's bit layout, plane k's
//!                 bits forming bit k of a voxel's palette index; or, for a brick of more than
//!                 16 materials (class Wide), 16 units of u16 ids, voxel i in half i % 2 of
//!                 word i / 2 (0 is air), and no palette
//! then            for a planar brick holding a TRANSLUCENT voxel, one unit of its translucent
//!                 voxels in the mask's layout (a translucent voxel at palette index 0 is
//!                 otherwise indistinguishable from air)
//! ```
//!
//! The record entry of a `Full` brick names its first unit and its class (`CLASS_SHIFT`), so
//! the slot's layout follows from the entry alone. Ahead of its 512 entries a record holds 512
//! brick bits, one per entry that is not uniform air, and the chunk table entry of a record
//! whose bits are all clear carries `EMPTY_BIT`, so the marcher skips an empty chunk on the
//! table read alone. Records carry no palette.
//!
//! Slots are handed out in *bins* of [`BINS`] units (ADR-0013's page kinds, one per bin): a
//! 4096-unit page takes a bin when it is first issued and holds `4096 / bin` slots of it; a
//! page whose last live slot leaves joins the free pages and is issued to whichever bin next
//! needs one, lowest first, before a page never handed out. A brick whose slot size changes
//! (a class or its translucent unit) moves to a slot of its new bin in the
//! same sync, and its entry is rewritten with it. No live brick moves otherwise. The pool is
//! sharded over `MAX_SHARDS` storage buffers (ADR-0004): unit `u` lives in shard
//! `u / units_per_shard`, a multiple of a page, so a slot never straddles two shards.
//!
//! The mirror reserves its VRAM once, at creation (ADR-0013): [`MirrorConfig::budget_bytes`]
//! is split into records for every chunk the window can hold (at most the binding allows),
//! then whole pages of units, filling shard 0 to its limit, then shard 1, and so on
//! ([`ShardLayout::with_budget`]). A shard holding no reserved unit is a one-unit placeholder,
//! so the marcher can always bind all of them. No pool or record buffer is created or copied
//! after `new`: a sync that would need more pages or records than are reserved fails before
//! writing anything. Pages are handed out from the bottom, so the pages handed out so far
//! ([`GpuWorld::pool_units`]) are the pool's high-water mark.
//!
//! A LOD-only chunk (ADR-0003) is mirrored from its mips as a LOD record: entries 0..64 hold
//! level 1's mip-bricks, 64..72 level 2's and 72 level 3's, encoded like brick entries. Its
//! table entry carries `LOD_BIT`. A chunk that changes tier gets a new record: its old one and
//! its slots are freed in the same sync.
//!
//! A mirror with the field surface on ([`GpuWorld::with_field_surface`],
//! [`GpuWorld::set_field_surface`], ADR-0041) gives every band brick of a full chunk (a brick
//! holding a voxel whose 27-count is neither 0 nor 27, [`crate::field::band_mask`]) a *band
//! slot*: one unit of its band mask (the opaque mask's layout), then `ceil(n / 64)` units of
//! its `n` band voxels' `u8` counts in band-index order (the band voxels before it in voxel
//! order), in a bin of exactly its size ([`SLOT_BINS`], [`band_bin_of`]). The brick's header
//! names it ([`HEADER_BAND_WORD`]) and holds its band prefix ([`HEADER_BAND_PREFIX_WORD`],
//! [`pack_prefix`] of its band mask); a uniform band brick gets a slot of class
//! [`CLASS_UNIFORM`] for that header, laid out as class 1. The CPU writes the mask and zeroed
//! counts and lists the slot for the field pass, which fills the counts on the GPU
//! ([`GpuWorld::update_field`], `field.wgsl`). A brick's band depends on the bricks around it,
//! across chunk faces: its upload takes it from the world, and each sync then places again the
//! band slots of the bricks of other chunks its writes reach (`fix_band`), the one way an
//! upload couples chunks. Band slots count in the live units, the pressure and `mirror_bytes`.
//!
//! Every `Full` brick of a full chunk that `sync` writes, and each of its neighbours in the
//! chunk, is scanned (when its voxels hold a material with non-zero emission) for its
//! *exposed* emissive voxels: an opaque voxel of such a material with at least one of its six
//! neighbours not opaque, read across brick faces inside the chunk; a face on the chunk's face
//! counts as exposed. Each becomes a [`LightSample`] (its world voxel and the faces it shows) in a
//! light-sample buffer reserved once at creation ([`MirrorConfig::light_samples`], ADR-0013's
//! rule). The lists stay on the CPU; a chunk's samples are one contiguous range of that buffer
//! only while it lies within [`MirrorConfig::light_radius`] (squared chunk distance) of the
//! focus chunk, released beyond the radius plus one (ADR-0025). Ranges are written nearest
//! first, at most [`MirrorConfig::light_sync_samples`] per sync (the rest wait), each from its
//! lists after they change; a chunk the buffer cannot fit evicts the farthest range beyond
//! it, and is refused only when none is farther. A chunk above
//! [`MirrorConfig::light_chunk_cap`] keeps every `k`-th sample by index, `k` the least that
//! fits. Freed ranges are zeroed, so a sample of material 0 is a hole the light pass skips.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::PoisonError;

use vxe_world::packed::{Packed, PackedClass};
use vxe_world::{
    AIR, BRICK_EDGE, BRICK_VOXELS, Brick, CHUNK_BRICKS, CHUNK_EDGE_BRICKS, ChunkCoord, ChunkExtent,
    ChunkMips, MIP_LEVELS, MaterialId, MaterialTable, StreamEvents, VoxelCoord, World,
    brick_of_slot, brick_slot,
};

use crate::field::FieldBindings;

mod alloc;
mod format;
mod lights;
mod slot;
mod stage;
mod window;

pub use self::alloc::*;
pub use self::format::*;
pub use self::lights::*;
use self::slot::*;
pub use self::stage::*;
pub use self::window::*;

/// Where a sync reads its clock ([`SyncClock`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncPoint {
    /// Once, at the sync's start.
    Start,
    /// After selection sized a pending chunk (one without a kept size).
    Sized,
    /// After a chunk's record was written.
    Uploaded,
}

/// A sync clock: the time since some fixed start, read at each [`SyncPoint`]. The point lets
/// a test advance its clock for one kind of work only.
pub type SyncClock = Box<dyn FnMut(SyncPoint) -> std::time::Duration + Send + Sync>;

/// The default sync clock: wall time since its creation.
#[allow(
    clippy::disallowed_methods,
    reason = "the render thread bounds its upload time per frame; no simulation reads it"
)]
fn wall_clock() -> SyncClock {
    let start = std::time::Instant::now();
    Box::new(move |_| start.elapsed())
}

/// The render thread's light-extraction timer (`SyncStats::light_us`); no simulation reads it.
#[allow(
    clippy::disallowed_methods,
    reason = "the render thread times its light extraction for the streaming line's readings"
)]
fn stopwatch() -> std::time::Instant {
    std::time::Instant::now()
}

#[allow(
    clippy::disallowed_methods,
    reason = "the render thread times its light extraction for the streaming line's readings"
)]
fn micros_since(t: std::time::Instant) -> u64 {
    t.elapsed().as_micros() as u64
}

/// Why a sync could not complete.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MirrorError {
    /// The pool would need `needed` bytes of slots; the reservation holds `limit`.
    PoolTooLarge { needed: u64, limit: u64 },
    /// The chunk records would need `needed` bytes; the reservation holds `limit`, a whole
    /// number of records.
    RecordsTooLarge { needed: u64, limit: u64 },
}

impl std::fmt::Display for MirrorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PoolTooLarge { needed, limit } => write!(
                f,
                "brick pool needs {needed} bytes; the reservation holds {limit}"
            ),
            Self::RecordsTooLarge { needed, limit } => write!(
                f,
                "chunk records need {needed} bytes; the reservation holds {limit}"
            ),
        }
    }
}

impl std::error::Error for MirrorError {}

/// What one `sync` did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SyncStats {
    /// `Full` bricks scanned for exposed emissive voxels, light samples written, and the
    /// render thread's time in that scan and the ranges' rebuild, in microseconds.
    pub light_bricks: u64,
    pub light_samples: u64,
    pub light_us: u64,
    /// Ranges evicted to fit a nearer chunk, chunks left without a range because nothing
    /// farther could be evicted, and the syncs the oldest chunk inside the light radius has
    /// waited for its range.
    pub light_evicted: u64,
    pub light_refused: u64,
    pub light_oldest_wait: u64,
    /// Bytes copied into the mirror's buffers.
    pub bytes: u64,
    /// `Full` bricks written.
    pub bricks: u64,
    /// Chunks whose record was written.
    pub chunks: u64,
    /// Chunks examined: the pending chunks looked at, uploaded or not.
    pub chunks_visited: u64,
    /// Staged runs written: one belt copy command each.
    pub writes: u64,
    /// Chunks still waiting for an upload after this sync.
    pub pending_chunks: u64,
    /// Chunks selected this sync but deferred for space; they stay pending. 0 when everything
    /// fit.
    pub deferred_chunks: u64,
    /// Chunks selected this sync but left for the next because the sync's time budget
    /// ([`MirrorConfig::sync_budget_time`]) was spent; they stay pending, nearest first.
    /// Pending chunks selection did not reach once the budget was spent are not counted.
    pub time_deferred_chunks: u64,
    /// Pending chunks selection sized this sync, and those it took a kept size for instead:
    /// a chunk sized and not uploaded keeps its size until it uploads or changes.
    pub sized_chunks: u64,
    pub kept_sizes: u64,
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
    /// Time a sync spends, selection and upload together, read on the mirror's sync clock
    /// ([`GpuWorld::set_sync_clock`]) after each chunk sized and each chunk uploaded; `None`
    /// for no budget. Selection stops sizing once it is spent, and the upload stops after the
    /// chunk that crosses it, uploaded whole; a sync uploads at least one chunk that fits.
    /// What is left stays pending, first in the next sync.
    pub sync_budget_time: Option<std::time::Duration>,
    /// Caps the units per pool shard below what the device allows; for tests.
    pub max_shard_units: Option<u32>,
    /// VRAM reserved for the pool shards and records at creation, split by the
    /// device's limits ([`ShardLayout::with_budget`]). The chunk table and material rows are
    /// not part of it.
    pub budget_bytes: u64,
    /// Chunk records to reserve; `None` reserves one for every chunk of the window. A
    /// following window's residency is a ball, so a caller that knows it passes fewer.
    pub record_chunks: Option<u64>,
    /// Light samples the light-sample buffer holds, reserved at creation.
    pub light_samples: u32,
    /// Most light samples one chunk keeps.
    pub light_chunk_cap: u32,
    /// Chunks within this many (squared chunk distance `r^2`) of the focus chunk hold a range
    /// of the light-sample buffer; beyond `r + 1` they release it (ADR-0025).
    pub light_radius: i32,
    /// Light samples one sync writes before carrying the rest over (at least one chunk).
    pub light_sync_samples: u32,
}

impl MirrorConfig {
    /// A window with no sync budget, no shard cap and the default budget,
    /// [`DEFAULT_BUDGET_BYTES`], clamped to what the device can bind.
    pub fn new(window: Window) -> Self {
        Self {
            window,
            sync_budget_bytes: u64::MAX,
            sync_budget_time: None,
            max_shard_units: None,
            budget_bytes: DEFAULT_BUDGET_BYTES,
            record_chunks: None,
            light_samples: DEFAULT_LIGHT_SAMPLES,
            light_chunk_cap: DEFAULT_LIGHT_CHUNK_CAP,
            light_radius: DEFAULT_LIGHT_RADIUS,
            light_sync_samples: DEFAULT_LIGHT_SYNC_SAMPLES,
        }
    }
}

/// The mirror: a composition of its parts, each owning its own state.
pub struct GpuWorld {
    device: wgpu::Device,
    cfg: MirrorConfig,
    /// The queue of the last `sync`, which writes the field pass's uniform and work list.
    queue: Option<wgpu::Queue>,
    clock: SyncTimer,
    window: WindowQueue,
    pool: PoolAlloc,
    recs: Records,
    up: Uploader,
    mats: MaterialFlags,
    lights: LightMirror,
    field: FieldState,
}

impl GpuWorld {
    /// A mirror of a bounded world: the window is `extent`, fixed, with no sync budget.
    pub fn new(device: &wgpu::Device, extent: ChunkExtent) -> Self {
        Self::with_config(device, MirrorConfig::new(Window::Fixed(extent)))
    }

    /// A mirror with an explicit window, sync budget and VRAM budget. Creates every pool and
    /// record buffer at its reserved size; the pool starts empty.
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
        let layout = ShardLayout::from_limits(&device.limits(), cfg.max_shard_units)
            .with_budget(cfg.budget_bytes, cfg.record_chunks.unwrap_or(n as u64));
        let storage = wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC;
        // Every entry starts ABSENT, as `table_cpu` does: `flush_table` writes only entries
        // that change, so an entry no chunk ever took must not read as record 0.
        let table = wgpu::util::DeviceExt::create_buffer_init(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("chunk_table"),
                contents: &vec![0xFF; n * 4],
                usage: storage,
            },
        );
        Self {
            device: device.clone(),
            lights: LightMirror::new(device, cfg.light_samples),
            cfg,
            queue: None,
            clock: SyncTimer::new(),
            window: WindowQueue::new(window_min, window_size, focus, n),
            pool: PoolAlloc::new(layout),
            recs: Records::new(layout.reserved_records),
            up: Uploader::new(device, layout, table),
            mats: MaterialFlags::new(),
            field: FieldState::default(),
        }
    }

    /// Turns the field surface's band slots on or off (ADR-0041): on, every band brick of a
    /// full chunk carries a band slot, which the field pass ([`GpuWorld::update_field`])
    /// fills. Every chunk mirrored full is dropped and queued again, so
    /// the next syncs write it whole in the new layout.
    pub fn set_field_surface(&mut self, on: bool) {
        if on == self.field.band.is_some() {
            return;
        }
        let full: Vec<ChunkCoord> = self
            .recs
            .mirrored
            .iter()
            .filter(|(_, m)| !m.lod)
            .map(|(&c, _)| c)
            .collect();
        for &c in &full {
            self.drop_record(c);
        }
        self.field.chunks.clear();
        self.field.bricks.clear();
        self.field.shells.clear();
        // The band's slots are part of a chunk's size.
        self.window.sizes.clear();
        self.field.band = on.then(|| Band {
            pass: crate::field::FieldPass::new(&self.device),
            slots: HashMap::new(),
            items: HashMap::new(),
            fresh: HashMap::new(),
            retry: HashSet::new(),
        });
        for c in full {
            self.enqueue(c);
        }
    }

    /// Whether band bricks carry band slots.
    pub fn field_surface(&self) -> bool {
        self.field.band.is_some()
    }

    /// Whether the table the mirror holds has a crisp row that is not translucent
    /// ([`vxe_world::MaterialTable::is_crisp`]): one the field surface draws as cubes. False
    /// before the first sync writes the rows.
    pub fn crisp_rows(&self) -> bool {
        self.mats.crisp
    }

    /// Band slots held, one per band brick of the chunks mirrored full; 0 without the field.
    pub fn band_bricks(&self) -> u64 {
        self.field.band.as_ref().map_or(0, |b| {
            b.slots
                .values()
                .map(|s| s.iter().filter(|&&u| u != ABSENT).count() as u64)
                .sum()
        })
    }

    /// Units of the band slots held (each slot's whole bin) and of the slots of uniform band
    /// bricks ([`CLASS_UNIFORM`]); 0 without the field.
    pub fn band_units(&self) -> u64 {
        let Some(b) = &self.field.band else {
            return 0;
        };
        let mut n = 0u64;
        for (c, slots) in &b.slots {
            let rec = self.recs.mirrored.get(c).map(|m| m.record);
            for (i, &u) in slots.iter().enumerate() {
                if u == ABSENT {
                    continue;
                }
                n += u64::from(SLOT_BINS[self.bin_of_unit(u).expect("a live band slot")]);
                let e = rec.map_or(0, |r| {
                    self.recs.entries[r as usize * RECORD_WORDS + ENTRY_WORD + i]
                });
                if e & FULL_BIT != 0 && e >> CLASS_SHIFT & CLASS_MASK == CLASS_UNIFORM {
                    n += u64::from(SLOT_BINS[self.bin_of_unit(e & UNIT_MASK).expect("live")]);
                }
            }
        }
        n
    }

    /// Band slots waiting for the field pass at the next `update_field`.
    pub fn field_items(&self) -> usize {
        self.field.band.as_ref().map_or(0, |b| b.items.len())
    }

    /// A mirror under `cfg` with the field surface's band slots on
    /// ([`GpuWorld::set_field_surface`]).
    pub fn with_field_surface(device: &wgpu::Device, cfg: MirrorConfig) -> Self {
        let mut m = Self::with_config(device, cfg);
        m.set_field_surface(true);
        m
    }

    /// Rewrites the material rows from `table`, a table of the same rows as the world's at the
    /// first sync with other renderer fields (albedo, roughness, opacity): the
    /// translucent, emissive and opaque flags the upload reads stay the first sync's. Before
    /// the first sync the rows wait for it.
    ///
    /// # Panics
    ///
    /// If the rows are written and `table` has a different number of rows.
    pub fn rewrite_materials(&mut self, table: &MaterialTable, queue: &wgpu::Queue) {
        if !self.mats.written {
            self.mats.rows_from = Some(table.clone());
            return;
        }
        assert_eq!(
            table.len() as u64,
            self.mats.rows,
            "a rewrite keeps the table's rows"
        );
        queue.write_buffer(
            &self.up.materials,
            0,
            bytemuck::cast_slice(&material_rows(table)),
        );
    }

    /// The chunks the mirror can hold now: the window's minimum corner and size.
    pub fn window(&self) -> ChunkExtent {
        ChunkExtent::new(
            self.window.min,
            [0, 1, 2].map(|k| self.window.min[k] + self.window.size[k]),
        )
    }

    /// The focus chunk: uploads run nearest it first, and a following window centres on it.
    pub fn focus(&self) -> ChunkCoord {
        self.window.focus
    }

    /// Moves the focus to the chunk holding `v`. Takes effect on the next `sync`.
    pub fn set_focus(&mut self, v: VoxelCoord) {
        self.window.focus = v.chunk();
    }

    /// Pool shard `k`, for `k < MAX_SHARDS`; a shard holding no reserved unit is a one-unit
    /// placeholder.
    pub fn pool_shard(&self, k: usize) -> &wgpu::Buffer {
        &self.up.shards[k]
    }

    /// How the pool is split on this device, and what the budget reserved.
    pub fn shard_layout(&self) -> ShardLayout {
        self.pool.layout
    }

    /// Shards holding a page handed out so far.
    pub fn shards_in_use(&self) -> usize {
        self.pool_units()
            .div_ceil(self.pool.layout.units_per_shard)
            .try_into()
            .unwrap_or(usize::MAX)
    }

    /// Pool units reserved, a whole number of pages.
    pub fn reserved_units(&self) -> u32 {
        self.pool.layout.reserved_units
    }

    /// Chunk records reserved.
    pub fn reserved_records(&self) -> u32 {
        self.pool.layout.reserved_records
    }

    pub fn records(&self) -> &wgpu::Buffer {
        &self.up.records
    }

    pub fn table(&self) -> &wgpu::Buffer {
        &self.up.table
    }

    pub fn materials(&self) -> &wgpu::Buffer {
        &self.up.materials
    }

    /// Units of the pages handed out so far, from unit 0: the pool's high-water mark, at most
    /// `reserved_units`.
    pub fn pool_units(&self) -> u32 {
        self.pool.next_page * PAGE_UNITS
    }

    /// Slots currently holding a `Full` brick or a band, over every bin.
    pub fn live_bricks(&self) -> u32 {
        self.pool.bins.iter().map(|b| b.live).sum()
    }

    /// Units of the slots currently holding a `Full` brick or a band: each slot's whole bin.
    pub fn live_units(&self) -> u64 {
        self.pool
            .bins
            .iter()
            .zip(SLOT_BINS)
            .map(|(b, n)| u64::from(b.live) * u64::from(n))
            .sum()
    }

    /// Units of the slots handed out to no brick: every bin's free slots and the rest of its
    /// cursor's page. The units of free pages, and the tail of each page past its last whole
    /// slot, are not counted.
    pub fn spare_units(&self) -> u64 {
        self.pool
            .bins
            .iter()
            .enumerate()
            .map(|(k, b)| b.available(k) * u64::from(SLOT_BINS[k]))
            .sum()
    }

    /// Slots handed out to no brick, over every bin: free slots and the rest of each cursor's
    /// page.
    pub fn spare_slots(&self) -> u64 {
        self.pool
            .bins
            .iter()
            .enumerate()
            .map(|(k, b)| b.available(k))
            .sum()
    }

    /// Each bin's slot size, live and spare slots, and pages.
    pub fn bin_stats(&self) -> [BinStats; BIN_COUNT] {
        std::array::from_fn(|k| BinStats {
            units: SLOT_BINS[k],
            live: self.pool.bins[k].live,
            spare: self.pool.bins[k].available(k),
            pages: self
                .pool
                .page_bin
                .iter()
                .filter(|b| **b == Some(k as u8))
                .count() as u32,
        })
    }

    /// The bin of the page holding unit `u`, `None` for a free or never-issued page.
    pub fn bin_of_unit(&self, u: u32) -> Option<usize> {
        self.pool.bin_of_unit(u)
    }

    /// The free slots of bin `k`, in the order they will be reused last-first.
    pub fn free_slots(&self, k: usize) -> &[u32] {
        &self.pool.bins[k].free
    }

    /// Pages ever handed out from the reservation, free ones included: the pool's high-water
    /// mark in pages.
    pub fn pages_issued(&self) -> u32 {
        self.pool.next_page
    }

    /// Pages handed out whose live slots fell to zero, ascending: free for any bin.
    pub fn free_pages(&self) -> Vec<u32> {
        self.pool.free_pages.iter().copied().collect()
    }

    /// Bytes written to the GPU over the mirror's life.
    pub fn bytes_written(&self) -> u64 {
        self.up.bytes_written
    }

    /// The staging belt's chunks, active, closed, in flight and free, each
    /// [`BELT_CHUNK_BYTES`] (or one run's size, past it): host memory held mapped for the
    /// mirror's uploads.
    pub fn belt_chunks(&self) -> usize {
        self.up.belt.chunks.len()
    }

    /// Times a sync left the belt holding more than [`BELT_KEEP_CHUNKS`] chunks and replaced
    /// it.
    pub fn belt_trims(&self) -> u64 {
        self.up.belt.trims
    }

    /// Whether `coord` is mirrored from its mips.
    pub fn is_lod(&self, coord: ChunkCoord) -> bool {
        self.recs.mirrored.get(&coord).is_some_and(|m| m.lod)
    }

    /// Chunks holding a record.
    pub fn mirrored_chunks(&self) -> usize {
        self.recs.mirrored.len()
    }

    pub fn is_mirrored(&self, coord: ChunkCoord) -> bool {
        self.recs.mirrored.contains_key(&coord)
    }

    /// Chunks waiting for an upload.
    pub fn pending_chunks(&self) -> usize {
        self.window.pending.len()
    }

    /// Whether `coord` waits for an upload: its mirrored record, if any, is not its content.
    pub fn is_pending(&self, coord: ChunkCoord) -> bool {
        self.window.pending_d2.contains_key(&coord)
    }

    /// The chunks holding a full record (not LOD-only), in no order.
    pub fn full_chunks(&self) -> impl Iterator<Item = ChunkCoord> + '_ {
        self.recs
            .mirrored
            .iter()
            .filter(|(_, m)| !m.lod)
            .map(|(c, _)| *c)
    }

    /// Whether every chunk with lights whose box lies within `r` chunks of `coord`'s box (the
    /// bake's candidate table's reach, `bake::job_table`) holds a current range of the
    /// light-sample buffer.
    pub fn lights_ready_near(&self, coord: ChunkCoord, r: i32) -> bool {
        let rr = i64::from(r).pow(2);
        self.lights.chunks.iter().all(|(c, l)| {
            let gap2: i64 = (0..3)
                .map(|k| i64::from(((c.0[k] - coord.0[k]).abs() - 1).max(0)).pow(2))
                .sum();
            gap2 >= rr || (l.range.is_some() && !self.lights.dirty.contains(c))
        })
    }

    /// The record entry the mirror holds for brick `slot` of chunk `coord`.
    pub fn entry(&self, coord: ChunkCoord, slot: usize) -> Option<u32> {
        let r = self.recs.mirrored.get(&coord)?.record;
        Some(self.recs.entries[r as usize * RECORD_WORDS + ENTRY_WORD + slot])
    }

    /// Record index of `coord`, if it is mirrored.
    pub fn record_index(&self, coord: ChunkCoord) -> Option<u32> {
        self.recs.mirrored.get(&coord).map(|m| m.record)
    }

    /// Index of `coord` in the chunk table: the coordinate wrapped into the window size.
    pub fn table_index(&self, c: ChunkCoord) -> u32 {
        let w = self.window.size.map(|v| v.max(1));
        let wr = [0, 1, 2].map(|k| c.0[k].rem_euclid(w[k]) as u32);
        let w = w.map(|v| v as u32);
        wr[0] + w[0] * (wr[1] + w[1] * wr[2])
    }

    fn in_window(&self, c: ChunkCoord) -> bool {
        (0..3).all(|k| {
            c.0[k] >= self.window.min[k] && c.0[k] < self.window.min[k] + self.window.size[k]
        })
    }

    /// Bytes the mirror occupies on the GPU by the uploader's live accounting: the units of
    /// live slots (each slot's whole bin), the records of mirrored chunks, chunk table and
    /// material rows. Spare slots, free pages and free records are not counted;
    /// `allocated_bytes` is what the reserved buffers hold.
    pub fn mirror_bytes(&self) -> u64 {
        self.live_units() * UNIT_BYTES
            + self.recs.mirrored.len() as u64 * RECORD_BYTES
            + self.window.table.len() as u64 * 4
            + self.mats.rows * MATERIAL_FLOATS as u64 * 4
    }

    /// Bytes of the reserved buffers: every pool shard (placeholders included) and the
    /// records, which is the budget rounded down to whole pages and records
    /// ([`ShardLayout::reserved_bytes`]). Fixed from creation: no sync, eviction or window
    /// move changes it. The chunk table and material rows are outside the reservation.
    pub fn allocated_bytes(&self) -> u64 {
        self.up.shards.iter().map(wgpu::Buffer::size).sum::<u64>() + self.up.records.size()
    }

    /// The mirror's pressure, the larger of its two occupancies (ADR-0013): pool units in
    /// use, `live_units() / reserved_units()`, and records in use, `mirrored_chunks() /
    /// reserved_records()`. The application decides what to do as it nears 1; a sync that
    /// would pass the reservation fails instead of growing it. The pool can fail a little
    /// below 1, at each bin's partly filled page and each page's tail.
    pub fn pressure(&self) -> f32 {
        let unit_term =
            self.live_units() as f64 / f64::from(self.pool.layout.reserved_units.max(1));
        let record_term =
            self.recs.mirrored.len() as f64 / f64::from(self.pool.layout.reserved_records.max(1));
        unit_term.max(record_term) as f32
    }

    /// Records into `encoder` the field pass over the band slots `sync` placed since the last
    /// call ([`GpuWorld::field_items`]), and empties that list. Returns the slots filled; 0
    /// records nothing, and so does a mirror without the field. The pass reads the pool,
    /// records, table and material rows as earlier submissions and the `sync` copies recorded
    /// ahead of it in `encoder` leave them.
    pub fn update_field(&mut self, encoder: &mut wgpu::CommandEncoder) -> usize {
        self.update_field_timed(encoder, None)
    }

    /// [`GpuWorld::update_field`], writing the pass's start and end timestamps when
    /// `timestamps` is given and the work list is not empty.
    pub fn update_field_timed(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        timestamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) -> usize {
        let n = self.field_items();
        let e = ChunkExtent::new(
            self.window.min,
            [0, 1, 2].map(|k| self.window.min[k] + self.window.size[k]),
        );
        let b = FieldBindings {
            shards: [0, 1, 2, 3].map(|k| &self.up.shards[k]),
            records: &self.up.records,
            table: &self.up.table,
            materials: &self.up.materials,
            window: (e.min.0, e.size()),
            units_per_shard: self.pool.layout.units_per_shard,
        };
        if let (Some(band), Some(queue)) = (&mut self.field.band, &self.queue) {
            let mut items: Vec<crate::field::FieldItem> = band
                .items
                .drain()
                .map(|(unit, origin)| crate::field::FieldItem { unit, origin })
                .collect();
            items.sort_unstable_by_key(|it| it.unit);
            band.pass
                .encode(&self.device, queue, encoder, &b, &items, timestamps);
        }
        n
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
        let Some(Mirrored { record: rec, .. }) = self.recs.mirrored.remove(&coord) else {
            return;
        };
        self.note_change(coord);
        if let Some(slots) = self
            .field
            .band
            .as_mut()
            .and_then(|b| b.slots.remove(&coord))
        {
            for &u in slots.iter().filter(|&&u| u != ABSENT) {
                self.field
                    .band
                    .as_mut()
                    .expect("held above")
                    .items
                    .remove(&u);
                self.pool.free_slot(u);
            }
        }
        if self.field.band.is_some() {
            self.field.shells.insert(coord);
        }
        self.drop_lights(coord);
        let base = rec as usize * RECORD_WORDS;
        let mut slots = Vec::new();
        let mut emptied = Vec::new();
        for i in base + ENTRY_WORD..base + RECORD_WORDS {
            let e = self.recs.entries[i];
            if e & FULL_BIT == 0 {
                continue;
            }
            let u = e & UNIT_MASK;
            if self.pool.unlive(u) {
                emptied.push(u / PAGE_UNITS);
            }
            slots.push(u);
        }
        self.recs.entries[base..base + RECORD_WORDS].fill(0);
        for p in emptied {
            self.pool.release_page(p);
        }
        // Slots of a released page are not listed. The rest are pushed highest first, so the
        // next allocations pop them in ascending order and the chunk that reuses them is
        // written in long runs.
        slots.retain(|u| self.pool.page_bin[(u / PAGE_UNITS) as usize].is_some());
        slots.sort_unstable_by(|a, b| b.cmp(a));
        for u in slots {
            let k = self.pool.page_bin[(u / PAGE_UNITS) as usize].expect("held above") as usize;
            self.pool.bins[k].free.push(u);
        }
        self.recs.free.push(rec);
        if self.in_window(coord) {
            let i = self.table_index(coord);
            let t = self.window.table[i as usize];
            if t != ABSENT && t & RECORD_MASK == rec {
                self.window.table[i as usize] = ABSENT;
                self.window.table_changed.push(i);
            }
        }
    }

    /// Replaces the clock the sync's time budget reads (wall time by default), for a test
    /// that advances it by a fixed step per read.
    pub fn set_sync_clock(&mut self, clock: SyncClock) {
        self.clock.read = clock;
    }

    /// Uploads what the pending chunks need, nearest the focus first, until the byte budget
    /// or the time budget ([`MirrorConfig::sync_budget_time`]) is spent, and clears their
    /// dirty bits. Takes one chunk lock at a time.
    ///
    /// Writes what fits the reservation, nearest first, and never part of a chunk (ADR-0013):
    /// the first chunk whose slots or record do not fit is deferred, and so is every later
    /// chunk except a demotion (a full chunk now LOD-only) that still fits. Deferred chunks
    /// stay pending with their dirty bits and their mirrored record, and are counted in
    /// `SyncStats::deferred_chunks`. A sync that defers chunks and uploads none fails with
    /// `PoolTooLarge` or `RecordsTooLarge`, writing nothing.
    ///
    /// Every write is a staging-belt copy recorded into `encoder` (ADR-0022), which the
    /// caller submits before anything reads the mirror: the frame's own encoder ahead of the
    /// field pass and the march, or one of its own outside a frame. A sync whose encoder is
    /// never submitted keeps its belt chunks. `queue` is kept for the field pass's own
    /// writes.
    pub fn sync(
        &mut self,
        world: &World,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<SyncStats, MirrorError> {
        self.clock.start = (self.clock.read)(SyncPoint::Start);
        let result = self.sync_into(world, queue, encoder);
        self.up.belt.finish(encoder);
        result
    }

    fn sync_into(
        &mut self,
        world: &World,
        queue: &wgpu::Queue,
        enc: &mut wgpu::CommandEncoder,
    ) -> Result<SyncStats, MirrorError> {
        let mut st = SyncStats::default();
        if self.field.band.is_some() && self.queue.is_none() {
            self.queue = Some(queue.clone());
        }
        self.reconcile_window(world);
        self.flush_dropped_lights(enc);
        for c in world.store().take_dirty() {
            self.forget_size(c);
            if self.in_window(c) {
                self.enqueue(c);
            }
        }

        let short = self.select(world, &mut st);
        if self.window.selected.is_empty()
            && let Some(e) = short
        {
            // An empty sync: the nearest chunk does not fit and no demotion behind it does.
            return Err(e);
        }

        if !self.mats.written {
            // The flags are read while staging slots; the rows are written below, once a sync
            // writes anything.
            self.mats.read(world.materials());
        }
        let mut short = short;
        let mut result = self.upload(world, &mut short, enc, &mut st);
        if result.is_ok() {
            result = self.fix_band(world, enc, &mut st);
        }
        if result.is_ok()
            && st.chunks == 0
            && let Some(e) = short
        {
            // The upload's check deferred every chosen chunk before writing any of it: an
            // empty sync. Queued table changes and the material rows wait for the next sync
            // that writes.
            return Err(e);
        }
        if !self.mats.written {
            let table = self.mats.rows_from.take();
            self.write_materials(table.as_ref().unwrap_or(world.materials()), enc, &mut st);
            self.mats.written = true;
        }
        let t = stopwatch();
        self.place_lights(enc, &mut st);
        st.light_us += micros_since(t);
        // Staged runs are written even after a failed allocation: their records name them.
        for k in 0..BIN_COUNT {
            self.flush_stage(k, enc, &mut st);
        }
        self.flush_records(enc, &mut st);
        self.flush_table(enc, &mut st);
        st.pending_chunks = self.window.pending.len() as u64;
        self.up.bytes_written += st.bytes;
        result.map(|()| st)
    }
}
