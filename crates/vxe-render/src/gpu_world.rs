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
//! entries; index 0 is air in every palette. Ahead of its entries a record holds 512 brick
//! bits, one per entry that is not uniform air, and the chunk table entry of a record whose
//! bits are all clear carries `EMPTY_BIT`, so the marcher skips an empty chunk on the table
//! read alone. A palette only grows while its chunk is
//! mirrored, so bricks already uploaded keep their indices. A chunk that needs more than 255
//! non-air materials is *wide*: each `Full` brick takes two slots from an even slot and holds
//! `u16` voxels, and a chunk that turns wide uploads every `Full` brick again.
//!
//! The pool is sharded over up to `MAX_SHARDS` storage buffers (ADR-0004). Slot `s` lives in
//! shard `s / slots_per_shard`; `slots_per_shard` is the largest multiple of a page that fits
//! the device's binding and buffer size limits, so a wide brick's even-aligned pair never
//! straddles two shards. Masks, records, table and materials stay one buffer each, the masks
//! indexed by global slot.
//!
//! The mirror reserves its VRAM once, at creation (ADR-0013): [`MirrorConfig::budget_bytes`]
//! is split into records for every chunk the window can hold (at most the binding allows),
//! then whole pages of pool slots with their masks, filling shard 0 to its limit, then shard
//! 1, and so on ([`ShardLayout::with_budget`]). A shard holding no reserved slot is a one-slot
//! placeholder, so the marcher can always bind all of them. No pool, mask or record buffer is
//! created or copied after `new`: a sync that would need more slots or records than are
//! reserved fails before writing anything. Pages are handed out from the bottom, so the pages
//! handed out so far ([`GpuWorld::pool_slots`]) are the pool's high-water mark.
//!
//! A LOD-only chunk (ADR-0003) is mirrored from its mips as a LOD record: entries 0..64 hold
//! level 1's mip-bricks, 64..72 level 2's and 72 level 3's, encoded like brick entries and
//! narrowed through the record's palette like voxel bricks. Its table entry carries
//! `LOD_BIT`. A chunk that changes tier gets a new record: its old one and its slots are
//! freed in the same sync.
//!
//! A mirror made with [`GpuWorld::with_surface_attrs`] keeps a per-voxel contour attribute for
//! the `Full` bricks of full chunks, in *attribute pages* of the pool (ADR-0002's reserved
//! attribute, packed into the existing shards). Each 4096-slot page is then of one kind: a
//! *plain page* holds mip-bricks as above; an attribute page holds bricks of full chunks in its
//! slots `0..ATTR_PAGE_BRICK_SLOTS`, and brick slot `i` of the page owns the two slots
//! `ATTR_PAGE_BRICK_SLOTS + 2i` and the one after, which hold its 512 `u16` attributes
//! ([`attr_slot`]). Each kind has its own cursor and free lists. The CPU never writes an
//! attribute slot: [`GpuWorld::update_attrs`] fills them on the GPU (`attrib.wgsl`) for the
//! bricks `sync` wrote and their neighbours, so a brick placed in a page is always followed by
//! its attribute. Without attribute pages every page is plain, and the allocator hands out
//! slots exactly as a single plain kind.
//!
//! A page is of one kind while it holds a live slot. Each page counts its live brick slots
//! (two for a wide brick); a page whose count falls to zero leaves its kind's free lists and
//! cursor and joins the free pages, and a kind that needs a page takes the lowest free page,
//! of either former kind, before one never handed out. So plain and attribute slots share one
//! high-water mark, and no live brick ever moves: a page changes kind only while it is empty.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::PoisonError;

use vxe_world::{
    AIR, BRICK_EDGE, BRICK_VOXELS, Brick, CHUNK_BRICKS, CHUNK_EDGE_BRICKS, ChunkCoord, ChunkExtent,
    ChunkMips, MIP_LEVELS, MaterialId, MaterialTable, StreamEvents, VoxelCoord, World,
    brick_of_slot, brick_slot,
};

use crate::attrib::{AttribBindings, AttribItem, AttribPass};

/// Pool slots per page: the unit the pool is reserved and handed out in, and the unit
/// `slots_per_shard` is a multiple of.
pub const POOL_PAGE_SLOTS: u32 = 4096;
/// Most storage buffers the pool is sharded over.
pub const MAX_SHARDS: u32 = 4;
/// Storage buffers the marcher binds besides the pool shards: masks, records, table,
/// materials.
pub const OTHER_STORAGE_BUFFERS: u32 = 4;
/// Bytes of one pool slot: a narrow brick's 512 palette indices, or half a wide brick.
pub const SLOT_BYTES: u64 = BRICK_VOXELS as u64;
/// Pool slots per group of the mask buffer.
pub const MASK_GROUP_SLOTS: u32 = 8;
/// `u32` words of a slot's opaque mask: one bit per voxel, one 64-byte line.
pub const OPAQUE_WORDS: usize = 16;
/// `u32` words of a slot's translucent mask: one bit per 2³ block.
pub const TRANSLUCENT_WORDS: usize = 2;
/// `u32` words of one mask group: its eight slots' opaque masks, each on its own 64-byte line,
/// then one line holding the eight slots' translucent masks.
pub const MASK_GROUP_WORDS: u32 = MASK_GROUP_SLOTS * (OPAQUE_WORDS + TRANSLUCENT_WORDS) as u32;
/// Bytes of one slot's masks, a group's bytes over its slots: 576 / 8.
pub const MASK_BYTES: u64 = MASK_GROUP_WORDS as u64 * 4 / MASK_GROUP_SLOTS as u64;
/// Entries in a palette, index 0 being air.
pub const PALETTE_ENTRIES: usize = 256;
/// `u32` words of brick bits at the head of a chunk record: bit `b` of word `b / 32` is set
/// when record entry `b` is not uniform air.
pub const BITS_WORDS: usize = CHUNK_BRICKS / 32;
/// The record word of brick entry 0.
pub const ENTRY_WORD: usize = BITS_WORDS;
/// The record word where the palette starts.
pub const PALETTE_WORD: usize = ENTRY_WORD + CHUNK_BRICKS;
/// `u32` words of one chunk record: the brick bits, 512 brick entries, then the palette as
/// `u16` pairs.
pub const RECORD_WORDS: usize = PALETTE_WORD + PALETTE_ENTRIES / 2;
/// Bytes of one chunk record.
pub const RECORD_BYTES: u64 = (RECORD_WORDS * 4) as u64;
/// Bytes one page of the pool reserves: its slots and their masks.
pub const PAGE_BYTES: u64 = POOL_PAGE_SLOTS as u64 * (SLOT_BYTES + MASK_BYTES);
/// Bytes a reservation sets aside for placeholders before it counts pages: four one-slot
/// shards and one mask group, so the buffers never exceed the budget.
pub const PLACEHOLDER_BYTES: u64 = MAX_SHARDS as u64 * SLOT_BYTES + MASK_GROUP_WORDS as u64 * 4;
/// The default budget of the pool, masks and records: the world line of `nfr.md` §3, 3 GiB.
pub const DEFAULT_BUDGET_BYTES: u64 = 3 << 30;
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
/// Brick entry bit: the brick holds a `TRANSLUCENT` voxel (a uniform brick's material is one).
/// The opaque walk reads it from the entry it loads anyway, so the translucent walk runs only
/// for a ray that passed such a brick.
pub const TRANSLUCENT_BIT: u32 = 1 << 29;
/// The material of a uniform brick's entry.
pub const MATERIAL_MASK: u32 = 0xFFFF;
/// The pool slot in a `Full` entry.
pub const SLOT_MASK: u32 = TRANSLUCENT_BIT - 1;
/// Chunk table entry bit: the record is a LOD record.
pub const LOD_BIT: u32 = 1 << 30;
/// Chunk table entry bit: every entry of the record is uniform air, so its brick bits are
/// all clear. Kept equal to the record's bits on every write of the record.
pub const EMPTY_BIT: u32 = 1 << 29;
/// The record index in a chunk table entry that is not `ABSENT`.
pub const RECORD_MASK: u32 = EMPTY_BIT - 1;
/// First entry of each mip level in a LOD record, level 1 first.
pub const LOD_LEVEL_ENTRY: [usize; MIP_LEVELS] = [0, 64, 72];
/// `rev` value of a material not in the palette being built.
const NO_INDEX: u16 = u16::MAX;
/// `f32`s per material row: albedo rgb, roughness, emission rgb, flags as bits, detail scale
/// and strength, opacity, 5 spare.
pub const MATERIAL_FLOATS: usize = 16;
/// Brick slots of an attribute page; the rest of the page holds their attributes, two slots
/// per brick slot, and its last 4 slots are unused.
pub const ATTR_PAGE_BRICK_SLOTS: u32 = 1364;
/// The attribute of a voxel with no contour plane.
pub const NO_PLANE: u16 = 0xFFFF;

/// The first of the two pool slots holding the attributes of brick slot `s`, which lies in an
/// attribute page: `p + ATTR_PAGE_BRICK_SLOTS + 2 (s - p)` with `p = s - s % POOL_PAGE_SLOTS`.
/// Always in the same page, so in the same shard, as `s`.
pub fn attr_slot(s: u32) -> u32 {
    let p = s - s % POOL_PAGE_SLOTS;
    p + ATTR_PAGE_BRICK_SLOTS + 2 * (s - p)
}

/// The mask word where slot `s`'s opaque mask starts: word `i / 32` bit `i % 32` is voxel `i`
/// of the brick (`x + 8 y + 64 z`), set when the voxel is not air. A multiple of 16 words, so
/// the mask is one 64-byte line.
pub fn opaque_mask_word(s: u32) -> u32 {
    MASK_GROUP_WORDS * (s / MASK_GROUP_SLOTS) + OPAQUE_WORDS as u32 * (s % MASK_GROUP_SLOTS)
}

/// The mask word where slot `s`'s two translucent words start, in its group's ninth line: bit
/// `bx + 4 by + 16 bz` is set when 2³ block `(bx, by, bz)` holds a translucent voxel.
pub fn translucent_mask_word(s: u32) -> u32 {
    MASK_GROUP_WORDS * (s / MASK_GROUP_SLOTS)
        + MASK_GROUP_SLOTS * OPAQUE_WORDS as u32
        + TRANSLUCENT_WORDS as u32 * (s % MASK_GROUP_SLOTS)
}

/// A slot's masks as staged: the opaque words, then the translucent words.
type SlotMasks = [u32; OPAQUE_WORDS + TRANSLUCENT_WORDS];

/// The kind of a pool page, and of the slots it hands out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// Every slot a brick slot: mip-bricks, or every brick without attribute pages.
    Plain = 0,
    /// Brick slots of full chunks' `Full` bricks, then their attributes.
    Attr = 1,
}

impl Kind {
    /// Brick slots a page of this kind hands out.
    fn brick_slots(self) -> u32 {
        match self {
            Self::Plain => POOL_PAGE_SLOTS,
            Self::Attr => ATTR_PAGE_BRICK_SLOTS,
        }
    }
}

/// The slot allocator of one page kind: a cursor through its current page and free lists.
#[derive(Debug, Default)]
struct SlotAlloc {
    /// The next never-used slot of the current page, and the end of the page's brick slots.
    next: u32,
    end: u32,
    /// Brick slots of this kind holding a brick, two for a wide brick.
    live: u32,
    /// Free single slots in pages of this kind that still hold a live slot.
    free: Vec<u32>,
    /// First slots of free slot pairs, from wide bricks.
    free_pairs: Vec<u32>,
}

impl SlotAlloc {
    fn live(&self) -> u32 {
        self.live
    }

    /// Brick slots this kind can hand out without a new page.
    fn available(&self) -> u64 {
        u64::from(self.end - self.next) + self.free.len() as u64 + 2 * self.free_pairs.len() as u64
    }
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
    /// Chunks selected this sync but deferred for space; they stay pending. 0 when everything
    /// fit.
    pub deferred_chunks: u64,
    /// Pool, mask or record buffers this sync created: 0, since `new` reserves them all.
    pub buffers_created: u32,
    /// GPU buffer-to-buffer copies this sync issued: 0, since no buffer is ever replaced.
    pub copies: u32,
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
    /// VRAM reserved for the pool shards, masks and records at creation, split by the
    /// device's limits ([`ShardLayout::with_budget`]). The chunk table and material rows are
    /// not part of it.
    pub budget_bytes: u64,
    /// Chunk records to reserve; `None` reserves one for every chunk of the window. A
    /// following window's residency is a ball, so a caller that knows it passes fewer.
    pub record_chunks: Option<u64>,
}

impl MirrorConfig {
    /// A window with no sync budget, no shard cap and the default budget,
    /// [`DEFAULT_BUDGET_BYTES`], clamped to what the device can bind.
    pub fn new(window: Window) -> Self {
        Self {
            window,
            sync_budget_bytes: u64::MAX,
            max_shard_slots: None,
            budget_bytes: DEFAULT_BUDGET_BYTES,
            record_chunks: None,
        }
    }
}

/// The smallest budget whose split reserves at least `slots` pool slots and a record for each
/// of `window_chunks` chunks, when the device can bind them.
pub fn budget_for(slots: u64, window_chunks: u64) -> u64 {
    window_chunks * RECORD_BYTES
        + slots.div_ceil(u64::from(POOL_PAGE_SLOTS)) * PAGE_BYTES
        + PLACEHOLDER_BYTES
}

/// How the pool is split over storage buffers on a device, and what a budget reserves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShardLayout {
    /// The largest multiple of `POOL_PAGE_SLOTS` whose bytes fit one binding and one buffer,
    /// at most `max_shard_slots` rounded down to a page.
    pub slots_per_shard: u32,
    /// `min(MAX_SHARDS, max_storage_buffers_per_shader_stage - OTHER_STORAGE_BUFFERS)`.
    pub shards: u32,
    /// Chunk records the one record buffer can hold:
    /// `min(max_storage_buffer_binding_size, max_buffer_size) / RECORD_BYTES`.
    pub record_capacity: u32,
    /// Pool slots reserved, a whole number of pages, at most [`ShardLayout::capacity`].
    pub reserved_slots: u32,
    /// Chunk records reserved, at most `record_capacity`.
    pub reserved_records: u32,
}

impl ShardLayout {
    /// The layout `limits` allow, reserving all of it. Reads only the limits, never the
    /// backend or vendor.
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
        let record_capacity = (bytes / RECORD_BYTES).min(u64::from(u32::MAX)) as u32;
        let capacity = u64::from(slots_per_shard) * u64::from(shards);
        Self {
            slots_per_shard,
            shards,
            record_capacity,
            reserved_slots: capacity
                .min(u64::from(u32::MAX) / u64::from(POOL_PAGE_SLOTS) * u64::from(POOL_PAGE_SLOTS))
                as u32,
            reserved_records: record_capacity,
        }
    }

    /// Slots the pool can hold over all its shards.
    pub fn capacity(&self) -> u64 {
        u64::from(self.slots_per_shard) * u64::from(self.shards)
    }

    /// This layout reserving `budget_bytes`: past [`PLACEHOLDER_BYTES`], a record for each of
    /// `window_chunks` chunks, at most `record_capacity` and at most what the budget holds;
    /// then whole pages of slots with their masks ([`PAGE_BYTES`] each) in what is left, at
    /// most the shards' capacity. The reserved bytes never exceed the budget unless it is
    /// smaller than one record and the placeholders.
    pub fn with_budget(self, budget_bytes: u64, window_chunks: u64) -> Self {
        let budget = budget_bytes.saturating_sub(PLACEHOLDER_BYTES);
        let records = window_chunks
            .min(u64::from(self.record_capacity))
            .min(budget / RECORD_BYTES)
            .max(1);
        let rest = budget.saturating_sub(records * RECORD_BYTES);
        let page = u64::from(POOL_PAGE_SLOTS);
        let slots = (rest / PAGE_BYTES * page).min(u64::from(self.reserved_slots));
        Self {
            reserved_slots: slots as u32,
            reserved_records: records as u32,
            ..self
        }
    }

    /// Bytes of the buffers this layout reserves: the pool shards (a placeholder slot for a
    /// shard holding none), the masks (one group when there is no slot) and the records.
    pub fn reserved_bytes(&self) -> u64 {
        let sps = u64::from(self.slots_per_shard);
        let slots = u64::from(self.reserved_slots);
        let shards: u64 = (0..u64::from(MAX_SHARDS))
            .map(|k| shard_slots(slots, sps, k).max(1) * SLOT_BYTES)
            .sum();
        let groups = slots.div_ceil(u64::from(MASK_GROUP_SLOTS)).max(1);
        shards
            + groups * u64::from(MASK_GROUP_WORDS) * 4
            + u64::from(self.reserved_records) * RECORD_BYTES
    }
}

/// The slots shard `k` holds of `slots` reserved, shards filling in order at `sps` each.
fn shard_slots(slots: u64, sps: u64, k: u64) -> u64 {
    slots.saturating_sub(k * sps).min(sps)
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
    /// Brick slots of each page kind its upload takes new, narrow.
    new: [u64; 2],
}

/// The pool and records as a sync's selection leaves them, replayed chunk by chunk in the
/// order `upload` places them. A chunk changing tier first frees its old record's slots:
/// credited to that kind's spare slots, or, for a page those frees empty, as a free page less
/// the page's spare slots. Then its new slots take spare slots of their kind, then free
/// pages, then pages never handed out. No free is credited before the chunk that makes it.
/// Once a kind has taken spare slots (its free lists or cursor), a later free of that kind is
/// credited as a slot, never as an emptied page: the allocator hands just-freed slots back
/// first, so an earlier allocation may sit on the page.
struct Fit {
    /// Spare brick slots of each kind.
    avail: [u64; 2],
    free_pages: u64,
    /// Slots past the pages handed out: whole pages with attribute pages, else slots, since
    /// a plain page never handed out continues the cursor's page.
    fresh: u64,
    records: u64,
    /// Slots the placed chunks free, per page.
    freed: HashMap<u32, u32>,
    /// Whether each kind has taken spare slots.
    took: [bool; 2],
}

impl Fit {
    fn new(gw: &GpuWorld) -> Self {
        Self {
            avail: [0, 1].map(|k| gw.allocs[k].available()),
            free_pages: gw.free_pages.len() as u64,
            fresh: 0,
            records: gw.mirrored.len() as u64,
            freed: HashMap::new(),
            took: [false; 2],
        }
    }

    /// Places one chunk: `frees` (the kind and record of a tier change's old record), `new`
    /// slots of each kind, and a new record when `new_record`. Changes nothing when the chunk
    /// does not fit, and returns what it would have needed.
    fn place(
        &mut self,
        gw: &GpuWorld,
        frees: Option<(usize, u32)>,
        new: [u64; 2],
        new_record: bool,
    ) -> Result<(), MirrorError> {
        let reserved = u64::from(gw.layout.reserved_records);
        if new_record && self.records + 1 > reserved {
            return Err(MirrorError::RecordsTooLarge {
                needed: (self.records + 1) * RECORD_BYTES,
                limit: reserved * RECORD_BYTES,
            });
        }
        let page = u64::from(POOL_PAGE_SLOTS);
        let (mut avail, mut free_pages, mut fresh) = (self.avail, self.free_pages, self.fresh);
        let mut took = self.took;
        let mut local: HashMap<u32, u32> = HashMap::new();
        if let Some((kind, rec)) = frees {
            let base = rec as usize * RECORD_WORDS;
            for &e in &gw.entries[base + ENTRY_WORD..base + PALETTE_WORD] {
                if e & FULL_BIT == 0 {
                    continue;
                }
                let s = e & SLOT_MASK;
                let n = if e & WIDE_BIT != 0 { 2 } else { 1 };
                let p = s / POOL_PAGE_SLOTS;
                let f = local.entry(p).or_insert(0);
                *f += n;
                let total = *f + self.freed.get(&p).copied().unwrap_or(0);
                if !self.took[kind] && u32::from(gw.page_live[p as usize]) == total {
                    // The page empties: its spare slots, and the ones credited from it so
                    // far, leave the kind; the page becomes free for either kind.
                    let credited = total - n;
                    avail[kind] =
                        avail[kind].saturating_sub(gw.spare_in_page(kind, p) + u64::from(credited));
                    free_pages += 1;
                } else {
                    avail[kind] += u64::from(n);
                }
            }
        }
        for (k, &n) in new.iter().enumerate() {
            let take = n.min(avail[k]);
            avail[k] -= take;
            took[k] |= take > 0;
            let mut rest = n - take;
            let bs = u64::from(if k == Kind::Attr as usize {
                ATTR_PAGE_BRICK_SLOTS
            } else {
                POOL_PAGE_SLOTS
            });
            while rest > 0 {
                if free_pages > 0 {
                    free_pages -= 1;
                } else if gw.surface_attrs {
                    fresh += page;
                } else {
                    // Plain pages continue the cursor: count the slots, not the page.
                    fresh += rest;
                    break;
                }
                let used = rest.min(bs);
                avail[k] += bs - used;
                rest -= used;
            }
        }
        let need = u64::from(gw.next_page) * page + fresh;
        let limit = u64::from(gw.layout.reserved_slots);
        if need > limit {
            return Err(MirrorError::PoolTooLarge {
                needed: need * SLOT_BYTES,
                limit: limit * SLOT_BYTES,
            });
        }
        (self.avail, self.free_pages, self.fresh, self.took) = (avail, free_pages, fresh, took);
        for (p, n) in local {
            *self.freed.entry(p).or_insert(0) += n;
        }
        self.records += u64::from(new_record);
        Ok(())
    }
}

/// The bricks one record is written from, read in place.
enum Bricks<'a> {
    /// A full chunk's 512 bricks and its dirty bits.
    Chunk(&'a vxe_world::Chunk),
    /// A LOD record's mip-bricks, level 1 first; every one counts as dirty.
    Mips(&'a ChunkMips),
}

impl<'a> Bricks<'a> {
    fn len(&self) -> usize {
        match self {
            Self::Chunk(_) => CHUNK_BRICKS,
            Self::Mips(m) => (1..=MIP_LEVELS).map(|n| m.level(n).len()).sum(),
        }
    }

    /// Entry `i`'s brick: a chunk's brick `i`, or the mip-brick at record entry `i`, each
    /// level starting at its `LOD_LEVEL_ENTRY`.
    fn get(&self, i: usize) -> &'a Brick {
        match self {
            Self::Chunk(ch) => &ch.bricks()[i],
            Self::Mips(m) => {
                let k = LOD_LEVEL_ENTRY.iter().rposition(|&e| e <= i).unwrap_or(0);
                &m.level(k + 1)[i - LOD_LEVEL_ENTRY[k]]
            }
        }
    }

    fn is_dirty(&self, i: usize) -> bool {
        match self {
            Self::Chunk(ch) => ch.is_dirty(i),
            Self::Mips(_) => true,
        }
    }
}

pub struct GpuWorld {
    device: wgpu::Device,
    cfg: MirrorConfig,
    window_min: [i32; 3],
    window_size: [i32; 3],
    focus: ChunkCoord,
    initialized: bool,
    /// The device's layout and the reservation: `reserved_slots` and `reserved_records`.
    layout: ShardLayout,
    /// One buffer per shard, created at its reserved size; a shard holding no reserved slot
    /// is a one-slot placeholder.
    shards: Vec<wgpu::Buffer>,
    masks: wgpu::Buffer,
    records: wgpu::Buffer,
    table: wgpu::Buffer,
    materials: wgpu::Buffer,
    materials_written: bool,
    /// Material rows written: the rows `mirror_bytes` counts.
    material_rows: u64,
    mirrored: HashMap<ChunkCoord, Mirrored>,
    /// CPU copy of every reserved record's entries, `reserved_records * RECORD_WORDS`, zeroed
    /// at creation (untouched pages cost no RAM until a record is written).
    entries: Vec<u32>,
    free_records: Vec<u32>,
    next_record: u32,
    /// CPU copy of the chunk table, and the indices changed since the last write.
    table_cpu: Vec<u32>,
    table_changed: Vec<u32>,
    /// Whether full chunks' `Full` bricks take slots from attribute pages.
    surface_attrs: bool,
    /// The allocator of each page kind, indexed by `Kind`.
    allocs: [SlotAlloc; 2],
    /// The kind of every page handed out, `None` while it is free, and its live brick slots
    /// (two for a wide brick).
    page_kind: Vec<Option<Kind>>,
    page_live: Vec<u16>,
    /// Pages handed out whose live slots fell to zero, taken lowest first by either kind.
    free_pages: BTreeSet<u32>,
    /// The next page never handed out: the pool's high-water mark in pages.
    next_page: u32,
    /// What changed since the last `update_attrs`: chunks whose every brick needs its
    /// attribute, bricks (world brick coordinates) whose 3 x 3 x 3 neighbourhood does, and
    /// chunks whose neighbours' bricks next to them do.
    attr_chunks: HashSet<ChunkCoord>,
    attr_bricks: HashSet<[i32; 3]>,
    shell_chunks: HashSet<ChunkCoord>,
    attrib: Option<AttribPass>,
    /// Palette index of every material while one chunk is narrowed, else `NO_INDEX`.
    rev: Vec<u16>,
    /// Whether each material is `TRANSLUCENT`, from the table at the first sync.
    translucent: Vec<bool>,
    /// Chunks waiting for an upload, keyed by squared distance to `pending_focus`.
    pending: BTreeSet<(i64, ChunkCoord)>,
    pending_d2: HashMap<ChunkCoord, i64>,
    pending_focus: ChunkCoord,
    bytes_written: u64,
    /// The run of consecutive pool slots staged for one write: its first slot, its voxels
    /// and its masks, and the run's masks rearranged into group layout when written. Kept
    /// across syncs so staging allocates only while it first grows.
    stage_start: u32,
    stage_voxels: Vec<u8>,
    stage_masks: Vec<SlotMasks>,
    stage_groups: Vec<u32>,
    /// The run of consecutive records staged for one write.
    rec_stage_start: u32,
    rec_stage: Vec<u32>,
    /// The chunks `select` chose and the stale ones it unqueues, cleared at the start of
    /// each `select` and kept for their capacity.
    selected: Vec<Selected>,
    stale: Vec<ChunkCoord>,
}

impl GpuWorld {
    /// A mirror of a bounded world: the window is `extent`, fixed, with no sync budget.
    pub fn new(device: &wgpu::Device, extent: ChunkExtent) -> Self {
        Self::with_config(device, MirrorConfig::new(Window::Fixed(extent)))
    }

    /// A mirror with an explicit window, sync budget and VRAM budget. Creates every pool,
    /// mask and record buffer at its reserved size; the pool starts empty.
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
        let layout = ShardLayout::from_limits(&device.limits(), cfg.max_shard_slots)
            .with_budget(cfg.budget_bytes, cfg.record_chunks.unwrap_or(n as u64));
        let sps = u64::from(layout.slots_per_shard);
        let reserved = u64::from(layout.reserved_slots);
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
            shards: (0..u64::from(MAX_SHARDS))
                .map(|k| Self::create_pool(device, shard_slots(reserved, sps, k) as u32))
                .collect(),
            masks: Self::create_masks(device, layout.reserved_slots),
            records: Self::create_records(device, layout.reserved_records),
            table,
            materials: Self::create_materials(device, 1),
            materials_written: false,
            material_rows: 0,
            mirrored: HashMap::new(),
            entries: vec![0; layout.reserved_records as usize * RECORD_WORDS],
            free_records: Vec::new(),
            next_record: 0,
            table_cpu: vec![ABSENT; n],
            table_changed: Vec::new(),
            surface_attrs: false,
            allocs: Default::default(),
            page_kind: Vec::new(),
            page_live: Vec::new(),
            free_pages: BTreeSet::new(),
            next_page: 0,
            attr_chunks: HashSet::new(),
            attr_bricks: HashSet::new(),
            shell_chunks: HashSet::new(),
            attrib: None,
            rev: vec![NO_INDEX; 1 << 16],
            translucent: vec![false; 1 << 16],
            pending: BTreeSet::new(),
            pending_d2: HashMap::new(),
            pending_focus: focus,
            bytes_written: 0,
            stage_start: 0,
            stage_voxels: Vec::new(),
            stage_masks: Vec::new(),
            stage_groups: Vec::new(),
            rec_stage_start: 0,
            rec_stage: Vec::new(),
            selected: Vec::new(),
            stale: Vec::new(),
        }
    }

    /// A mirror under `cfg` whose full chunks' `Full` bricks take slots from attribute pages,
    /// with the pass that fills their contour attributes ([`GpuWorld::update_attrs`]).
    pub fn with_surface_attrs(device: &wgpu::Device, cfg: MirrorConfig) -> Self {
        let mut m = Self::with_config(device, cfg);
        m.surface_attrs = true;
        m.attrib = Some(AttribPass::new(device));
        m
    }

    /// Whether the mirror holds attribute pages.
    pub fn surface_attrs(&self) -> bool {
        self.surface_attrs
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

    /// A mask buffer of whole groups holding at least `slots` slots.
    fn create_masks(device: &wgpu::Device, slots: u32) -> wgpu::Buffer {
        let groups = slots.div_ceil(MASK_GROUP_SLOTS).max(1);
        Self::storage_buffer(
            device,
            "brick_masks",
            u64::from(groups * MASK_GROUP_WORDS) * 4,
        )
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

    /// Pool shard `k`, for `k < MAX_SHARDS`; a shard holding no reserved slot is a one-slot
    /// placeholder.
    pub fn pool_shard(&self, k: usize) -> &wgpu::Buffer {
        &self.shards[k]
    }

    /// How the pool is split on this device, and what the budget reserved.
    pub fn shard_layout(&self) -> ShardLayout {
        self.layout
    }

    /// Shards holding a page handed out so far.
    pub fn shards_in_use(&self) -> usize {
        self.pool_slots()
            .div_ceil(self.layout.slots_per_shard)
            .try_into()
            .unwrap_or(usize::MAX)
    }

    /// Pool slots reserved, a whole number of pages.
    pub fn reserved_slots(&self) -> u32 {
        self.layout.reserved_slots
    }

    /// Chunk records reserved.
    pub fn reserved_records(&self) -> u32 {
        self.layout.reserved_records
    }

    /// The masks of every pool slot, in groups of eight slots: [`opaque_mask_word`] and
    /// [`translucent_mask_word`] say where a slot's words lie.
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

    /// Slots of the pages handed out so far, from slot 0: the pool's high-water mark, at most
    /// `reserved_slots`.
    pub fn pool_slots(&self) -> u32 {
        self.next_page * POOL_PAGE_SLOTS
    }

    /// Brick slots currently holding a `Full` brick or half of a wide one; attribute slots
    /// are not counted.
    pub fn live_slots(&self) -> u32 {
        self.allocs.iter().map(SlotAlloc::live).sum()
    }

    /// Brick slots in attribute pages currently holding a `Full` brick or half of a wide one.
    pub fn live_attr_slots(&self) -> u32 {
        self.allocs[Kind::Attr as usize].live()
    }

    /// Whether pool slot `s` lies in an attribute page.
    pub fn in_attr_page(&self, s: u32) -> bool {
        self.page_kind.get((s / POOL_PAGE_SLOTS) as usize) == Some(&Some(Kind::Attr))
    }

    /// Pages ever handed out from the reservation, free ones included: the pool's high-water
    /// mark in pages.
    pub fn pages_issued(&self) -> u32 {
        self.next_page
    }

    /// Pages handed out whose live slots fell to zero, ascending: free for either kind.
    pub fn free_pages(&self) -> Vec<u32> {
        self.free_pages.iter().copied().collect()
    }

    /// Brick slots the mirror can hand out before it takes a page never handed out: each
    /// kind's free slots and the rest of its current page, and every slot of the free pages.
    /// Without attribute pages, `live_slots() + spare_slots() == pool_slots()`.
    pub fn spare_slots(&self) -> u64 {
        self.allocs.iter().map(SlotAlloc::available).sum::<u64>()
            + self.free_pages.len() as u64 * u64::from(POOL_PAGE_SLOTS)
    }

    /// Bytes of the masks of the pool slots in use, attribute slots included: `MASK_BYTES` a
    /// slot.
    pub fn live_mask_bytes(&self) -> u64 {
        (u64::from(self.live_slots()) + 2 * u64::from(self.live_attr_slots())) * MASK_BYTES
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

    /// Single slots of plain pages (every page, without attribute pages) returned and not yet
    /// reused, in the order they will be reused last-first.
    pub fn free_slots(&self) -> &[u32] {
        &self.allocs[Kind::Plain as usize].free
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
        Some(self.entries[r as usize * RECORD_WORDS + ENTRY_WORD + slot])
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

    /// Bytes the mirror occupies on the GPU by the uploader's live accounting: pool slots in
    /// use with their masks, the two attribute slots and their masks of each live brick slot
    /// in an attribute page, the records of mirrored chunks, chunk table and material rows.
    /// Free slots and free records are not counted; `allocated_bytes` is what the reserved
    /// buffers hold.
    pub fn mirror_bytes(&self) -> u64 {
        u64::from(self.live_slots()) * (SLOT_BYTES + MASK_BYTES)
            + u64::from(self.live_attr_slots()) * 2 * (SLOT_BYTES + MASK_BYTES)
            + self.mirrored.len() as u64 * RECORD_BYTES
            + self.table_cpu.len() as u64 * 4
            + self.material_rows * MATERIAL_FLOATS as u64 * 4
    }

    /// Bytes of the reserved buffers: every pool shard (placeholders included), the masks
    /// and the records, which is the budget rounded down to whole pages and records
    /// ([`ShardLayout::reserved_bytes`]). Fixed from creation: no sync, eviction or window
    /// move changes it. The chunk table and material rows are outside the reservation.
    pub fn allocated_bytes(&self) -> u64 {
        self.shards.iter().map(wgpu::Buffer::size).sum::<u64>()
            + self.masks.size()
            + self.records.size()
    }

    /// The mirror's pressure, the larger of its two occupancies (ADR-0013): pool slots in
    /// use, `(live_slots() + 2 live_attr_slots()) / reserved_slots()` (a brick in an attribute
    /// page holds its two attribute slots too), and records in use, `mirrored_chunks() /
    /// reserved_records()`. The application decides what to do as it nears 1; a sync that
    /// would pass the reservation fails instead of growing it. With attribute pages the pool
    /// can fail a little below 1, at each kind's partly filled page.
    pub fn pressure(&self) -> f32 {
        let slots = u64::from(self.live_slots()) + 2 * u64::from(self.live_attr_slots());
        let slot_term = slots as f64 / f64::from(self.layout.reserved_slots.max(1));
        let record_term =
            self.mirrored.len() as f64 / f64::from(self.layout.reserved_records.max(1));
        slot_term.max(record_term) as f32
    }

    /// Records into `encoder` the attribute pass over what changed since the last call: every
    /// `Full` brick of a full chunk that `sync` wrote, the bricks of full chunks among their 26
    /// neighbours, and the bricks of full chunks next to a chunk whose record was written new,
    /// dropped or replaced. Returns the bricks in the work list; 0 records nothing, and so
    /// does a mirror without attribute pages. The pass reads the pool, records and table as
    /// the queue holds them when `encoder` is submitted.
    pub fn update_attrs(&mut self, encoder: &mut wgpu::CommandEncoder) -> usize {
        self.update_attrs_timed(encoder, None)
    }

    /// [`GpuWorld::update_attrs`], writing the pass's start and end timestamps when
    /// `timestamps` is given and the work list is not empty.
    pub fn update_attrs_timed(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        timestamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) -> usize {
        let items = self.attr_work();
        if let Some(pass) = &self.attrib {
            let e = self.window();
            let b = AttribBindings {
                shards: [0, 1, 2, 3].map(|k| &self.shards[k]),
                records: &self.records,
                table: &self.table,
                window: (e.min.0, e.size()),
                slots_per_shard: self.layout.slots_per_shard,
            };
            pass.encode(&self.device, encoder, &b, &items, timestamps);
        }
        items.len()
    }

    /// The attribute pass's work list from what changed since the last one, deduplicated,
    /// and clears the change sets.
    fn attr_work(&mut self) -> Vec<AttribItem> {
        let chunks = std::mem::take(&mut self.attr_chunks);
        let bricks = std::mem::take(&mut self.attr_bricks);
        let shells = std::mem::take(&mut self.shell_chunks);
        let mut items = Vec::new();
        if !self.surface_attrs {
            return items;
        }
        let e = CHUNK_EDGE_BRICKS;
        for &c in &chunks {
            for i in 0..CHUNK_BRICKS {
                let b = brick_of_slot(i);
                if let Some(item) = self.attr_item([0, 1, 2].map(|k| c.0[k] * e + b[k])) {
                    items.push(item);
                }
            }
        }
        // Bricks outside the chunks taken whole, each once.
        let mut seen = HashSet::new();
        let mut visit = |gw: &Self, b: [i32; 3], items: &mut Vec<AttribItem>| {
            let c = ChunkCoord(b.map(|v| v.div_euclid(e)));
            if !chunks.contains(&c)
                && seen.insert(b)
                && let Some(item) = gw.attr_item(b)
            {
                items.push(item);
            }
        };
        for &b in &bricks {
            for z in -1..=1 {
                for y in -1..=1 {
                    for x in -1..=1 {
                        visit(self, [b[0] + x, b[1] + y, b[2] + z], &mut items);
                    }
                }
            }
        }
        // The neighbours' bricks within one brick of each shell chunk's box.
        for &c in &shells {
            let lo = c.0.map(|v| v * e - 1);
            let hi = c.0.map(|v| v * e + e);
            for z in lo[2]..=hi[2] {
                for y in lo[1]..=hi[1] {
                    for x in lo[0]..=hi[0] {
                        let on_shell = [x, y, z]
                            .iter()
                            .enumerate()
                            .any(|(k, &v)| v == lo[k] || v == hi[k]);
                        if on_shell {
                            visit(self, [x, y, z], &mut items);
                        }
                    }
                }
            }
        }
        items
    }

    /// The work-list entry of world brick `b`, if it is a `Full` brick of a mirrored full
    /// chunk.
    fn attr_item(&self, b: [i32; 3]) -> Option<AttribItem> {
        let e = CHUNK_EDGE_BRICKS;
        let c = ChunkCoord(b.map(|v| v.div_euclid(e)));
        let m = self.mirrored.get(&c).filter(|m| !m.lod)?;
        let local = b.map(|v| v.rem_euclid(e));
        let entry = self.entries[m.record as usize * RECORD_WORDS + ENTRY_WORD + brick_slot(local)];
        (entry & FULL_BIT != 0).then(|| AttribItem {
            slot: entry & SLOT_MASK,
            origin: b.map(|v| v * BRICK_EDGE),
        })
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
        let Some(Mirrored {
            record: rec, lod, ..
        }) = self.mirrored.remove(&coord)
        else {
            return;
        };
        if self.surface_attrs {
            self.shell_chunks.insert(coord);
        }
        // A record's slots are all of one kind.
        let kind = self.kind_for(lod);
        let base = rec as usize * RECORD_WORDS;
        let mut slots = Vec::new();
        let mut pairs = Vec::new();
        let mut emptied = Vec::new();
        for i in base + ENTRY_WORD..base + PALETTE_WORD {
            let e = self.entries[i];
            if e & FULL_BIT == 0 {
                continue;
            }
            let (s, wide) = (e & SLOT_MASK, e & WIDE_BIT != 0);
            if self.unlive(s, wide) {
                emptied.push(s / POOL_PAGE_SLOTS);
            }
            if wide {
                pairs.push(s);
            } else {
                slots.push(s);
            }
        }
        self.entries[base..base + RECORD_WORDS].fill(0);
        for p in emptied {
            self.release_page(p);
        }
        // Slots of a released page are not listed. The rest are pushed highest first, so the
        // next allocations pop them in ascending order and the chunk that reuses them is
        // written in long runs.
        let held = |gw: &Self, s: &u32| gw.page_kind[(s / POOL_PAGE_SLOTS) as usize].is_some();
        slots.retain(|s| held(self, s));
        pairs.retain(|s| held(self, s));
        slots.sort_unstable_by(|a, b| b.cmp(a));
        let a = &mut self.allocs[kind as usize];
        a.free.extend(slots);
        a.free_pairs.extend(pairs);
        self.free_records.push(rec);
        if self.in_window(coord) {
            let i = self.table_index(coord);
            let t = self.table_cpu[i as usize];
            if t != ABSENT && t & RECORD_MASK == rec {
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
    /// Writes what fits the reservation, nearest first, and never part of a chunk (ADR-0013):
    /// the first chunk whose slots or record do not fit is deferred, and so is every later
    /// chunk except a demotion (a full chunk now LOD-only) that still fits. Deferred chunks
    /// stay pending with their dirty bits and their mirrored record, and are counted in
    /// `SyncStats::deferred_chunks`. A sync that defers chunks and uploads none fails with
    /// `PoolTooLarge` or `RecordsTooLarge`, writing nothing.
    pub fn sync(&mut self, world: &World, queue: &wgpu::Queue) -> Result<SyncStats, MirrorError> {
        let mut st = SyncStats::default();
        self.reconcile_window(world);
        for c in world.store().take_dirty() {
            if self.in_window(c) {
                self.enqueue(c);
            }
        }

        let short = self.select(world, &mut st);
        if self.selected.is_empty()
            && let Some(e) = short
        {
            // An empty sync: the nearest chunk does not fit and no demotion behind it does.
            return Err(e);
        }

        if !self.materials_written {
            self.write_materials(world.materials(), queue, &mut st);
            for (id, m) in world.materials().rows().iter().enumerate() {
                self.translucent[id] = m.is_translucent();
            }
            self.materials_written = true;
        }
        let may_widen = world.materials().len() > PALETTE_ENTRIES;
        let mut short = short;
        let result = self.upload(may_widen, &mut short, queue, &mut st);
        // Staged runs are written even after a failed allocation: their records name them.
        self.flush_stage(queue, &mut st);
        self.flush_records(queue, &mut st);
        self.flush_table(queue, &mut st);
        st.pending_chunks = self.pending.len() as u64;
        self.bytes_written += st.bytes;
        result?;
        match short {
            // Every chosen chunk turned wide past the pool: an empty sync.
            Some(e) if st.chunks == 0 => Err(e),
            _ => Ok(st),
        }
    }

    /// The page kind of a record's slots.
    fn kind_for(&self, lod: bool) -> Kind {
        if self.surface_attrs && !lod {
            Kind::Attr
        } else {
            Kind::Plain
        }
    }

    /// The `Full` bricks of `bricks` when uploading them turns `coord`'s record wide (more
    /// non-air materials than a palette holds): every one then takes a slot pair. `None` when
    /// the record stays narrow or already is wide. Reads only; the upload's own scan builds
    /// the palette.
    fn turns_wide(&self, coord: ChunkCoord, bricks: &Bricks<'_>, lod: bool) -> Option<u64> {
        let kept = self.mirrored.get(&coord).filter(|m| m.lod == lod);
        if kept.is_some_and(|m| m.wide) {
            return None;
        }
        let palette: HashSet<MaterialId> = match kept {
            Some(m) => m.palette.iter().copied().collect(),
            None => HashSet::from([AIR]),
        };
        let mut added = HashSet::new();
        let mut full = 0u64;
        for i in 0..bricks.len() {
            let Brick::Full(v) = bricks.get(i) else {
                continue;
            };
            full += 1;
            if kept.is_some() && !bricks.is_dirty(i) {
                continue;
            }
            for m in v.iter() {
                if !palette.contains(m) {
                    added.insert(*m);
                }
            }
        }
        (palette.len() + added.len() > PALETTE_ENTRIES).then_some(full)
    }

    /// Slots of `kind` that can still take slot pairs: free pairs, the rest of the kind's
    /// current page, free pages and the pages the reservation has not handed out.
    fn pair_room(&self, kind: Kind) -> u64 {
        let a = &self.allocs[kind as usize];
        let page = u64::from(POOL_PAGE_SLOTS);
        let unissued = u64::from(self.layout.reserved_slots) / page - u64::from(self.next_page);
        // `alloc_pair` skips an odd cursor slot; pages and their brick slots are even.
        let start = a.next + a.next % 2;
        let cursor = u64::from(a.end.saturating_sub(start)) / 2 * 2;
        2 * a.free_pairs.len() as u64
            + cursor
            + (self.free_pages.len() as u64 + unissued) * u64::from(kind.brick_slots())
    }

    /// Spare brick slots of `kind` in page `p`: its free singles and pairs, and the rest of
    /// the kind's current page when that is `p`.
    fn spare_in_page(&self, kind: usize, p: u32) -> u64 {
        let a = &self.allocs[kind];
        let free = a.free.iter().filter(|&&s| s / POOL_PAGE_SLOTS == p).count() as u64;
        let pairs = a
            .free_pairs
            .iter()
            .filter(|&&s| s / POOL_PAGE_SLOTS == p)
            .count() as u64;
        let cursor = if a.next < a.end && (a.end - 1) / POOL_PAGE_SLOTS == p {
            u64::from(a.end - a.next)
        } else {
            0
        };
        free + 2 * pairs + cursor
    }

    /// Walks the pending set nearest first and picks chunks into `self.selected` until the
    /// estimated bytes reach the budget, replaying the fit as it goes ([`Fit`]): a chunk
    /// changing tier is credited its old record's frees, then its new slots and record are
    /// checked against what is left. The first chunk that does not fit is deferred, with every
    /// later chunk but a demotion that fits; deferred chunks stay pending and do not count
    /// against the budget. Returns the error of the first deferral, if any.
    fn select(&mut self, world: &World, st: &mut SyncStats) -> Option<MirrorError> {
        let budget = self.cfg.sync_budget_bytes;
        // Writes that happen whatever is chosen count against the budget first.
        let mut bytes = self.table_changed.len() as u64 * 4;
        if !self.materials_written {
            bytes += (world.materials().len() * MATERIAL_FLOATS * 4) as u64;
        }
        self.selected.clear();
        self.stale.clear();
        let mut fit = Fit::new(self);
        let mut short = None;
        let attr = self.kind_for(false) as usize;
        for &(_, coord) in &self.pending {
            if bytes >= budget && !self.selected.is_empty() {
                break;
            }
            st.chunks_visited += 1;
            // The full chunk wins over mips: a promoted chunk replaces its LOD record.
            let src = match world.store().get(coord) {
                Some(chunk) => Src::Full(chunk),
                None => match world.lod(coord) {
                    Some(mips) => Src::Lod(mips),
                    None => {
                        self.stale.push(coord);
                        continue;
                    }
                },
            };
            let mirrored = self.mirrored.get(&coord);
            let lod = matches!(src, Src::Lod(_));
            // A record of the other tier is replaced whole.
            let (rec, wide) = match mirrored {
                Some(m) if m.lod == lod => (Some(m.record), m.wide),
                _ => (None, false),
            };
            let demotion = mirrored.is_some_and(|m| !m.lod) && lod;
            // A chunk that turns wide during upload is estimated narrow here; `upload`
            // re-checks it before its first slot (ADR-0004's rare path).
            let (slots, slot_bytes) = if wide { (2, 2) } else { (1, 1) };
            let mut chunk_bytes = RECORD_BYTES + if rec.is_none() { 4 } else { 0 };
            let mut new = [0u64; 2];
            match &src {
                Src::Full(chunk) => {
                    let ch = chunk.read().unwrap_or_else(PoisonError::into_inner);
                    if rec.is_some() && !ch.any_dirty() {
                        self.stale.push(coord);
                        continue;
                    }
                    for slot in 0..CHUNK_BRICKS {
                        if rec.is_some() && !ch.is_dirty(slot) {
                            continue;
                        }
                        if ch.brick(slot).is_full() {
                            chunk_bytes += slot_bytes * (SLOT_BYTES + MASK_BYTES);
                            let old = rec.map_or(0, |r| {
                                self.entries[r as usize * RECORD_WORDS + ENTRY_WORD + slot]
                            });
                            if old & FULL_BIT == 0 {
                                new[attr] += slots;
                            }
                        }
                    }
                }
                Src::Lod(mips) => {
                    // Mips never change: a chunk already mirrored LOD-only needs nothing.
                    if rec.is_some() {
                        self.stale.push(coord);
                        continue;
                    }
                    let full = mips.full_bricks() as u64;
                    chunk_bytes += full * (SLOT_BYTES + MASK_BYTES);
                    new[Kind::Plain as usize] += full;
                }
            }
            // The frees of a tier change: the old record's slots, of the old record's kind.
            let frees = mirrored
                .filter(|m| m.lod != lod)
                .map(|m| (self.kind_for(m.lod) as usize, m.record));
            if short.is_some() && !demotion {
                st.deferred_chunks += 1;
                continue;
            }
            match fit.place(self, frees, new, mirrored.is_none()) {
                Ok(()) => {
                    bytes += chunk_bytes;
                    self.selected.push(Selected { coord, src, new });
                }
                Err(e) => {
                    st.deferred_chunks += 1;
                    short.get_or_insert(e);
                }
            }
        }
        let mut stale = std::mem::take(&mut self.stale);
        for &c in &stale {
            self.unqueue(c);
        }
        stale.clear();
        self.stale = stale;
        short
    }

    /// Uploads the chunks `select` chose, stopping at the first failed allocation, and
    /// empties `self.selected`. When the table has more materials than a palette holds
    /// (`may_widen`), a chunk about to turn wide is re-checked first: if its bricks do not fit
    /// as slot pairs it is deferred whole, before anything of it is written or its old record
    /// dropped, and so is every later chunk but a demotion.
    fn upload(
        &mut self,
        may_widen: bool,
        short: &mut Option<MirrorError>,
        queue: &wgpu::Queue,
        st: &mut SyncStats,
    ) -> Result<(), MirrorError> {
        let mut selected = std::mem::take(&mut self.selected);
        let mut result = Ok(());
        let mut defer_rest = false;
        for Selected { coord, src, new } in selected.drain(..) {
            let lod = matches!(src, Src::Lod(_));
            let mirrored = self.mirrored.get(&coord);
            let demotion = lod && mirrored.is_some_and(|m| !m.lod);
            if defer_rest && !demotion {
                st.deferred_chunks += 1;
                continue;
            }
            let frees = mirrored
                .filter(|m| m.lod != lod)
                .map(|m| (self.kind_for(m.lod) as usize, m.record));
            let new_record = mirrored.is_none();
            // A chunk turning wide takes a slot pair for every `Full` brick; its old narrow
            // slots are freed only after their pairs are taken, so none is credited.
            let pairs = if may_widen {
                match &src {
                    Src::Full(chunk) => {
                        let ch = chunk.read().unwrap_or_else(PoisonError::into_inner);
                        self.turns_wide(coord, &Bricks::Chunk(&ch), false)
                    }
                    Src::Lod(mips) => self.turns_wide(coord, &Bricks::Mips(mips), true),
                }
            } else {
                None
            };
            let kind = self.kind_for(lod);
            let mut need = new;
            if let Some(n) = pairs {
                need[kind as usize] = 2 * n;
            }
            // An already-wide record's new bricks take pairs too (`select` counts two slots
            // each).
            let kept_wide = mirrored.is_some_and(|m| m.lod == lod && m.wide);
            let pairs = pairs.or(kept_wide.then(|| new[kind as usize] / 2));
            // The backstop: this chunk alone against the allocator as it is now. It must be
            // exact for every need it admits, so that no admitted chunk fails part-way: the
            // state is exact since a chunk's own frees precede its allocations, every slot
            // `Fit` counts serves a narrow brick (`alloc` splits free pairs), and a wide need
            // is checked against pair room only (free pairs, the even-aligned cursor, free
            // and unissued pages), never free singles. A chunk that does not fit is deferred
            // whole, before anything of it is dropped or written.
            let mut fit = Fit::new(self).place(self, frees, need, new_record);
            if let Some(n) = pairs
                && fit.is_ok()
            {
                let room = self.pair_room(kind);
                if 2 * n > room {
                    let limit = u64::from(self.layout.reserved_slots);
                    fit = Err(MirrorError::PoolTooLarge {
                        needed: (limit + 2 * n - room) * SLOT_BYTES,
                        limit: limit * SLOT_BYTES,
                    });
                }
            }
            if let Err(e) = fit {
                short.get_or_insert(e);
                st.deferred_chunks += 1;
                defer_rest = true;
                continue;
            }
            if self.mirrored.get(&coord).is_some_and(|m| m.lod != lod) {
                self.drop_record(coord);
            }
            result = match src {
                Src::Full(chunk) => {
                    let mut ch = chunk.write().unwrap_or_else(PoisonError::into_inner);
                    let r = self.upload_record(coord, Bricks::Chunk(&ch), false, queue, st);
                    if r.is_ok() {
                        ch.clear_dirty();
                    }
                    r
                }
                Src::Lod(mips) => self.upload_record(coord, Bricks::Mips(&mips), true, queue, st),
            };
            if result.is_err() {
                break;
            }
        }
        self.selected = selected;
        result
    }

    /// Writes one chunk's record: `bricks` into its entries (512 voxel bricks, or a LOD
    /// record's 73 mip-bricks): every brick when `coord` had no record before this call or
    /// its width changes, else the dirty ones; then its palette and, for a record allocated
    /// here, its table entry.
    fn upload_record(
        &mut self,
        coord: ChunkCoord,
        bricks: Bricks<'_>,
        lod: bool,
        queue: &wgpu::Queue,
        st: &mut SyncStats,
    ) -> Result<(), MirrorError> {
        let (rec, mut palette, was_wide, new) = match self.mirrored.remove(&coord) {
            Some(m) => (m.record, m.palette, m.wide, false),
            None => (self.alloc_record()?, vec![AIR], false, true),
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
            'scan: for i in 0..bricks.len() {
                if !new && !bricks.is_dirty(i) {
                    continue;
                }
                let Brick::Full(v) = bricks.get(i) else {
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
        let kind = self.kind_for(lod);
        if self.surface_attrs {
            if every || lod {
                self.shell_chunks.insert(coord);
            }
            if every && !lod {
                self.attr_chunks.insert(coord);
            }
        }
        let mut result = Ok(());
        for i in 0..bricks.len() {
            if !every && !bricks.is_dirty(i) {
                continue;
            }
            if self.surface_attrs && !every && !lod {
                let b = brick_of_slot(i);
                self.attr_bricks
                    .insert([0, 1, 2].map(|k| coord.0[k] * CHUNK_EDGE_BRICKS + b[k]));
            }
            result = self.upload_brick(base + ENTRY_WORD + i, bricks.get(i), wide, kind, queue, st);
            if result.is_err() {
                break;
            }
        }
        for &m in &palette {
            self.rev[usize::from(m)] = NO_INDEX;
        }
        for (k, w) in self.entries[base + PALETTE_WORD..base + RECORD_WORDS]
            .iter_mut()
            .enumerate()
        {
            let at = |i: usize| u32::from(palette.get(i).copied().unwrap_or(AIR));
            *w = at(2 * k) | at(2 * k + 1) << 16;
        }
        let empty = self.write_bits(base);
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
        // A re-synced record can turn empty or stop being so: its table entry follows it.
        let t = self.table_index(coord) as usize;
        let want = rec | if lod { LOD_BIT } else { 0 } | if empty { EMPTY_BIT } else { 0 };
        if new || self.table_cpu[t] != want {
            self.table_cpu[t] = want;
            self.table_changed.push(t as u32);
        }
        result?;
        st.chunks += 1;
        self.unqueue(coord);
        Ok(())
    }

    /// Sets the brick bits of the record at word `base` from its entries; true when they are
    /// all clear.
    fn write_bits(&mut self, base: usize) -> bool {
        let (bits, rest) = self.entries[base..base + PALETTE_WORD].split_at_mut(ENTRY_WORD);
        for (w, entries) in bits.iter_mut().zip(rest.chunks_exact(32)) {
            *w = entries
                .iter()
                .enumerate()
                .fold(0, |acc, (j, &e)| acc | u32::from(e != 0) << j);
        }
        bits.iter().all(|&w| w == 0)
    }

    /// Writes `brick` into record word `i`, a brick entry: a narrow brick as palette indices through `rev`,
    /// a wide one as two slots of `u16` voxels, a uniform one as its material. A slot of the
    /// wrong width is returned and a new one taken.
    fn upload_brick(
        &mut self,
        i: usize,
        brick: &Brick,
        wide: bool,
        kind: Kind,
        queue: &wgpu::Queue,
        st: &mut SyncStats,
    ) -> Result<(), MirrorError> {
        let old = self.entries[i];
        let old_slot = (old & FULL_BIT != 0).then_some((old & SLOT_MASK, old & WIDE_BIT != 0));
        let Brick::Full(v) = brick else {
            if let Some(s) = old_slot {
                self.free_slot(s);
            }
            let m = brick.get(0);
            self.entries[i] = u32::from(m)
                | if self.translucent[usize::from(m)] {
                    TRANSLUCENT_BIT
                } else {
                    0
                };
            return Ok(());
        };
        let s = match old_slot {
            Some((s, w)) if w == wide => s,
            _ => {
                let s = if wide {
                    self.alloc_pair(kind)?
                } else {
                    self.alloc(kind)?
                };
                if let Some(old) = old_slot {
                    self.free_slot(old);
                }
                s
            }
        };
        // The bits are set in the pass that narrows the voxels: an opaque voxel sets its bit
        // of the opaque mask, a translucent one its 2^3 block's bit (bx + 4 by + 16 bz) of the
        // two translucent words, and flags the entry.
        let mut masks: SlotMasks = [0; OPAQUE_WORDS + TRANSLUCENT_WORDS];
        let mut flag = 0;
        let mut set_bits = |i: usize, m: MaterialId| {
            if m == AIR {
                return;
            }
            if self.translucent[usize::from(m)] {
                let (x, y, z) = (i % 8, i / 8 % 8, i / 64);
                let block = x / 2 + 4 * (y / 2) + 16 * (z / 2);
                masks[OPAQUE_WORDS + block / 32] |= 1 << (block % 32);
                flag = TRANSLUCENT_BIT;
            } else {
                masks[i / 32] |= 1 << (i % 32);
            }
        };
        if wide {
            for (i, &m) in v.iter().enumerate() {
                set_bits(i, m);
            }
            let bytes: &[u8] = bytemuck::cast_slice(&v[..]);
            let (lo, hi) = bytes.split_at(SLOT_BYTES as usize);
            self.stage(s, lo, &masks, queue, st);
            self.stage(s + 1, hi, &[0; OPAQUE_WORDS + TRANSLUCENT_WORDS], queue, st);
            st.bytes += 2 * (SLOT_BYTES + MASK_BYTES);
            self.entries[i] = FULL_BIT | WIDE_BIT | flag | s;
        } else {
            let mut idx = [0u8; BRICK_VOXELS];
            for (i, (d, &m)) in idx.iter_mut().zip(v.iter()).enumerate() {
                *d = self.rev[usize::from(m)] as u8;
                set_bits(i, m);
            }
            self.stage(s, &idx, &masks, queue, st);
            st.bytes += SLOT_BYTES + MASK_BYTES;
            self.entries[i] = FULL_BIT | flag | s;
        }
        st.bricks += 1;
        Ok(())
    }

    /// Returns a slot, or a wide brick's slot pair, to the free list of its page's kind, or
    /// releases the page when it held the page's last live slot.
    fn free_slot(&mut self, (s, wide): (u32, bool)) {
        let p = s / POOL_PAGE_SLOTS;
        let kind = self.page_kind[p as usize].expect("a live slot's page has a kind");
        if self.unlive(s, wide) {
            self.release_page(p);
            return;
        }
        let a = &mut self.allocs[kind as usize];
        if wide {
            a.free_pairs.push(s);
        } else {
            a.free.push(s);
        }
    }

    /// Takes slot `s` (two slots from `s` when `wide`) off its page's and its kind's live
    /// counts; true when the page holds no live slot after it.
    fn unlive(&mut self, s: u32, wide: bool) -> bool {
        let p = (s / POOL_PAGE_SLOTS) as usize;
        let kind = self.page_kind[p].expect("a live slot's page has a kind");
        let n = if wide { 2 } else { 1 };
        self.page_live[p] -= n;
        self.allocs[kind as usize].live -= u32::from(n);
        self.page_live[p] == 0
    }

    /// Adds slot `s` (two from `s` when `wide`) to its page's and its kind's live counts.
    fn make_live(&mut self, s: u32, wide: bool, kind: Kind) {
        let n = if wide { 2 } else { 1 };
        self.page_live[(s / POOL_PAGE_SLOTS) as usize] += n;
        self.allocs[kind as usize].live += u32::from(n);
    }

    /// Frees page `p`, which holds no live slot: its slots leave its kind's free lists, the
    /// kind's cursor leaves it, and it joins the free pages.
    fn release_page(&mut self, p: u32) {
        let kind = self.page_kind[p as usize]
            .take()
            .expect("a page released once");
        let a = &mut self.allocs[kind as usize];
        a.free.retain(|&s| s / POOL_PAGE_SLOTS != p);
        a.free_pairs.retain(|&s| s / POOL_PAGE_SLOTS != p);
        if a.next < a.end && (a.end - 1) / POOL_PAGE_SLOTS == p {
            a.next = a.end;
        }
        self.free_pages.insert(p);
    }

    /// Appends slot `s` to the staged run, first writing the run out if `s` does not extend it
    /// or it is full.
    fn stage(
        &mut self,
        s: u32,
        bytes: &[u8],
        masks: &SlotMasks,
        queue: &wgpu::Queue,
        st: &mut SyncStats,
    ) {
        let len = self.stage_masks.len() as u32;
        let boundary = s.is_multiple_of(self.layout.slots_per_shard);
        if len > 0 && (s != self.stage_start + len || len >= STAGE_SLOTS || boundary) {
            self.flush_stage(queue, st);
        }
        if self.stage_masks.is_empty() {
            self.stage_start = s;
        }
        self.stage_voxels.extend_from_slice(bytes);
        self.stage_masks.push(*masks);
    }

    /// Writes the staged run into its pool shard and the masks buffer, and empties it.
    fn flush_stage(&mut self, queue: &wgpu::Queue, st: &mut SyncStats) {
        if self.stage_masks.is_empty() {
            return;
        }
        let sps = self.layout.slots_per_shard;
        let shard = (self.stage_start / sps) as usize;
        queue.write_buffer(
            &self.shards[shard],
            u64::from(self.stage_start % sps) * SLOT_BYTES,
            bytemuck::cast_slice(&self.stage_voxels),
        );
        st.writes += 1;
        self.write_masks(queue, st);
        self.stage_voxels.clear();
        self.stage_masks.clear();
    }

    /// Writes the staged run's masks: the groups it covers whole as one write of group
    /// layout, and the slots of a group it covers in part as one write of their opaque lines
    /// and one of their translucent words, so no write touches a slot outside the run.
    fn write_masks(&mut self, queue: &wgpu::Queue, st: &mut SyncStats) {
        let g = MASK_GROUP_SLOTS;
        let start = self.stage_start;
        let end = start + self.stage_masks.len() as u32;
        let mut s = start;
        while s < end {
            let first = s % g;
            let whole = if first == 0 { (end - s) / g } else { 0 };
            if whole > 0 {
                let groups = &mut self.stage_groups;
                groups.clear();
                for k in 0..whole {
                    let slots = &self.stage_masks[(s - start + k * g) as usize..][..g as usize];
                    for m in slots {
                        groups.extend_from_slice(&m[..OPAQUE_WORDS]);
                    }
                    for m in slots {
                        groups.extend_from_slice(&m[OPAQUE_WORDS..]);
                    }
                }
                queue.write_buffer(
                    &self.masks,
                    u64::from(opaque_mask_word(s)) * 4,
                    bytemuck::cast_slice(groups),
                );
                st.writes += 1;
                s += whole * g;
                continue;
            }
            let n = (g - first).min(end - s);
            let slots = &self.stage_masks[(s - start) as usize..][..n as usize];
            let groups = &mut self.stage_groups;
            groups.clear();
            for m in slots {
                groups.extend_from_slice(&m[..OPAQUE_WORDS]);
            }
            let opaque = groups.len();
            for m in slots {
                groups.extend_from_slice(&m[OPAQUE_WORDS..]);
            }
            queue.write_buffer(
                &self.masks,
                u64::from(opaque_mask_word(s)) * 4,
                bytemuck::cast_slice(&groups[..opaque]),
            );
            queue.write_buffer(
                &self.masks,
                u64::from(translucent_mask_word(s)) * 4,
                bytemuck::cast_slice(&groups[opaque..]),
            );
            st.writes += 2;
            s += n;
        }
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
                    m.detail[0],
                    m.detail[1],
                    m.opacity,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                ]
            })
            .collect();
        // The material rows are outside the reservation: sized to the table at the first sync.
        if self.materials.size() < (rows.len() * MATERIAL_FLOATS * 4) as u64 {
            self.materials = Self::create_materials(&self.device, rows.len());
        }
        self.material_rows = rows.len() as u64;
        let bytes: &[u8] = bytemuck::cast_slice(&rows);
        queue.write_buffer(&self.materials, 0, bytes);
        st.writes += 1;
        st.bytes += bytes.len() as u64;
    }

    /// A free record, or the next never-used one of the reservation. `sync` checks the
    /// reservation before uploading, so the error here is a backstop.
    fn alloc_record(&mut self) -> Result<u32, MirrorError> {
        if let Some(r) = self.free_records.pop() {
            return Ok(r);
        }
        let limit = self.layout.reserved_records;
        if self.next_record >= limit {
            return Err(MirrorError::RecordsTooLarge {
                needed: (u64::from(self.next_record) + 1) * RECORD_BYTES,
                limit: u64::from(limit) * RECORD_BYTES,
            });
        }
        self.next_record += 1;
        Ok(self.next_record - 1)
    }

    /// Moves `kind`'s cursor onto a page: the lowest free page, of either former kind, else
    /// the next page never handed out, failing with `PoolTooLarge` when that one lies past
    /// the reservation. A page right after the cursor's end continues it; otherwise the
    /// current page's unused brick slots go to the free list.
    fn take_page(&mut self, kind: Kind) -> Result<(), MirrorError> {
        let page = match self.free_pages.pop_first() {
            Some(page) => page,
            None => {
                let end = u64::from(self.next_page + 1) * u64::from(POOL_PAGE_SLOTS);
                let limit = u64::from(self.layout.reserved_slots);
                if end > limit {
                    return Err(MirrorError::PoolTooLarge {
                        needed: end * SLOT_BYTES,
                        limit: limit * SLOT_BYTES,
                    });
                }
                self.page_kind.push(None);
                self.page_live.push(0);
                self.next_page += 1;
                self.next_page - 1
            }
        };
        self.page_kind[page as usize] = Some(kind);
        let p = page * POOL_PAGE_SLOTS;
        let a = &mut self.allocs[kind as usize];
        if p != a.end {
            a.free.extend((a.next..a.end).rev());
            a.next = p;
        }
        a.end = p + kind.brick_slots();
        Ok(())
    }

    /// One slot: a free single, else the cursor, else half of a free pair (the other half
    /// becomes a single), else a new page. So every slot `SlotAlloc::available` counts can
    /// serve a narrow brick.
    fn alloc(&mut self, kind: Kind) -> Result<u32, MirrorError> {
        let a = &mut self.allocs[kind as usize];
        let s = if let Some(s) = a.free.pop() {
            s
        } else if a.next < a.end {
            a.next += 1;
            a.next - 1
        } else if let Some(s) = a.free_pairs.pop() {
            a.free.push(s + 1);
            s
        } else {
            self.take_page(kind)?;
            let a = &mut self.allocs[kind as usize];
            a.next += 1;
            a.next - 1
        };
        self.make_live(s, false, kind);
        Ok(s)
    }

    /// Two adjacent slots starting on an even slot, for a wide brick. An odd next slot is
    /// skipped onto the single free list.
    fn alloc_pair(&mut self, kind: Kind) -> Result<u32, MirrorError> {
        let s = match self.allocs[kind as usize].free_pairs.pop() {
            Some(s) => s,
            None => {
                let a = &self.allocs[kind as usize];
                // Pages and their brick-slot counts are even, so a pair never straddles a
                // page end unless the next page continues this one.
                if a.next + a.next % 2 + 2 > a.end {
                    self.take_page(kind)?;
                }
                let a = &mut self.allocs[kind as usize];
                let next = a.next;
                let start = next + next % 2;
                if start != next {
                    a.free.push(next);
                }
                a.next = start + 2;
                start
            }
        };
        self.make_live(s, true, kind);
        Ok(s)
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
