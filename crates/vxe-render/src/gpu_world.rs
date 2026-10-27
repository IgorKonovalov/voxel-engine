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
//!                 voxel); the surface prefix, attribute units and flags
//! units 2..       the payload: 0, 1, 2 or 4 bit planes in the mask's bit layout, plane k's
//!                 bits forming bit k of a voxel's palette index; or, for a brick of more than
//!                 16 materials (class Wide), 16 units of u16 ids, voxel i in half i % 2 of
//!                 word i / 2 (0 is air), and no palette
//! then            for a planar brick holding a TRANSLUCENT voxel, one unit of its translucent
//!                 voxels in the mask's layout (a translucent voxel at palette index 0 is
//!                 otherwise indistinguishable from air)
//! then            for a full chunk's brick in a mirror with contour attributes and with `n > 0`
//!                 surface voxels: one unit of its surface mask, then ceil(n / 32) units of
//!                 their u16 attributes, two per word, NO_PLANE past the n-th
//! ```
//!
//! A brick's *surface* is chunk-local ([`chunk_surface`]): its non-air voxels with a face
//! neighbour that is air inside the same chunk, or on a face of the chunk; no other chunk is
//! read, so an upload never couples chunks. The attribute of surface voxel `(x, y, z)` is at
//! index `prefix[z] + popcount(surface[z] & ((1 << (x + 8 y)) - 1))`, the header carrying
//! `prefix[z]`, the surface voxels of the layers below `z`. An upload of a dirty brick also
//! re-uploads its face neighbours within the chunk, whose surface masks may have changed.
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
//! (a class, its translucent unit or its attributes) moves to a slot of its new bin in the
//! same sync, and its entry is rewritten with it. No live brick moves otherwise. The pool is
//! sharded over up to `MAX_SHARDS` storage buffers (ADR-0004): unit `u` lives in shard
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
//! A mirror made with [`GpuWorld::with_surface_attrs`] keeps a contour attribute for each
//! surface voxel of the `Full` bricks of full chunks inside their slots, after the surface
//! mask. The CPU writes the attribute units as "no plane" and never fills them:
//! [`GpuWorld::update_attrs`] does, on the GPU (`attrib.wgsl`), for the bricks `sync` wrote
//! and their neighbours, so a brick written into a slot is always followed by its attribute.
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

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::PoisonError;

use vxe_world::packed::{Packed, PackedClass};
use vxe_world::{
    AIR, BRICK_EDGE, BRICK_VOXELS, Brick, CHUNK_BRICKS, CHUNK_EDGE_BRICKS, ChunkCoord, ChunkExtent,
    ChunkMips, MIP_LEVELS, MaterialId, MaterialTable, StreamEvents, VoxelCoord, World,
    brick_of_slot, brick_slot, solid_mask, surface_mask,
};

use crate::attr_work::{self, AttrWorkStats, MirrorView};
use crate::attrib::{AttribBindings, AttribItem, AttribPass, SurfaceFit};

/// Light samples [`MirrorConfig::new`] reserves, and the samples one chunk keeps at most.
pub const DEFAULT_LIGHT_SAMPLES: u32 = 1 << 20;
pub const DEFAULT_LIGHT_CHUNK_CAP: u32 = 8192;
/// The light radius in chunks and the light samples one sync writes, by default.
pub const DEFAULT_LIGHT_RADIUS: i32 = 4;
pub const DEFAULT_LIGHT_SYNC_SAMPLES: u32 = 16_384;

/// One exposed emissive voxel in the light-sample buffer, 16 bytes: its world voxel
/// coordinate, then its material in bits 0..16 and the faces it shows in bits 16..22 (bit
/// `16 + 2 k` the face toward -k, the next toward +k). Material 0 is no sample.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LightSample {
    pub voxel: [i32; 3],
    pub material_faces: u32,
}

impl LightSample {
    pub fn material(&self) -> MaterialId {
        (self.material_faces & 0xFFFF) as MaterialId
    }

    /// The faces the voxel shows: bit `2 k` toward -k, bit `2 k + 1` toward +k.
    pub fn faces(&self) -> u32 {
        self.material_faces >> 16
    }
}

/// Bytes of one [`LightSample`].
pub const LIGHT_SAMPLE_BYTES: u64 = 16;

/// Sample-sized slots past the reservation that hold the frame's candidate table: a header
/// and up to 1024 entries (`Frame` writes them; ADR-0025).
pub const LIGHT_TABLE_SLOTS: u64 = 1025;

/// The exposed emissive voxels of `Full` brick `brick` at brick slot `slot` of chunk `coord`,
/// in voxel-index order (x fastest); `emissive` and `opaque` are by material id. A voxel on
/// the brick's face looks across it into `neighbour(k, dir)`, the chunk's brick beside this
/// one on axis `k` toward `dir` (-1 or 1); with `None` (the chunk's face) that face counts
/// as exposed.
pub fn exposed_emitters<'a>(
    coord: ChunkCoord,
    slot: usize,
    brick: &Brick,
    emissive: &[bool],
    opaque: &[bool],
    neighbour: impl Fn(usize, i32) -> Option<&'a Brick>,
) -> Vec<LightSample> {
    let Brick::Full(p) = brick else {
        return Vec::new();
    };
    if !materials_of(p, |m| emissive[usize::from(m)]) {
        return Vec::new();
    }
    let mut v = [AIR; BRICK_VOXELS];
    p.decode(&mut v);
    let b = brick_of_slot(slot);
    let base = [0, 1, 2].map(|k| coord.0[k] * CHUNK_EDGE_BRICKS * BRICK_EDGE + b[k] * BRICK_EDGE);
    let e = BRICK_EDGE as usize;
    let beside = [0, 1, 2].map(|k| [neighbour(k, -1), neighbour(k, 1)]);
    // Whether the voxel across the brick's face from local `l` on axis `k` toward `dir` lets
    // light out: no brick there (the chunk's face) or a voxel that is not opaque.
    let across = |l: [usize; 3], k: usize, dir: usize| match beside[k][dir] {
        None => true,
        Some(b) => {
            let mut q = l;
            q[k] = if dir == 0 { e - 1 } else { 0 };
            !opaque[usize::from(b.get(q[0] + e * (q[1] + e * q[2])))]
        }
    };
    let mut out = Vec::new();
    for (i, &m) in v.iter().enumerate() {
        if !emissive[usize::from(m)] || !opaque[usize::from(m)] {
            continue;
        }
        let l = [i % e, i / e % e, i / (e * e)];
        let mut faces = 0u32;
        for k in 0..3 {
            let stride = [1, e, e * e][k];
            let low = if l[k] == 0 {
                across(l, k, 0)
            } else {
                !opaque[usize::from(v[i - stride])]
            };
            let high = if l[k] == e - 1 {
                across(l, k, 1)
            } else {
                !opaque[usize::from(v[i + stride])]
            };
            faces |= u32::from(low) << (2 * k) | u32::from(high) << (2 * k + 1);
        }
        if faces != 0 {
            out.push(LightSample {
                voxel: [0, 1, 2].map(|k| base[k] + l[k] as i32),
                material_faces: u32::from(m) | faces << 16,
            });
        }
    }
    out
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

/// `samples` thinned to at most `cap` by keeping every `k`-th by index, `k` the least that
/// fits.
pub fn thin_samples(samples: Vec<LightSample>, cap: u32) -> Vec<LightSample> {
    let n = samples.len();
    let cap = cap.max(1) as usize;
    if n <= cap {
        return samples;
    }
    let k = n.div_ceil(cap);
    samples.into_iter().step_by(k).collect()
}

/// A chunk's light samples: each emissive brick's list by brick slot, and its range of the
/// light-sample buffer.
#[derive(Default)]
struct ChunkLights {
    bricks: BTreeMap<u16, Vec<LightSample>>,
    range: Option<(u32, u32)>,
    /// The power of the samples last written ([`GpuWorld::light_chunks`]).
    power: f32,
}

/// Bytes of one unit of the pool: one 64-byte line, the opaque mask of a brick.
pub const UNIT_BYTES: u64 = 64;
/// `u32` words of one unit.
pub const UNIT_WORDS: usize = 16;
/// Units per page: the unit the pool is reserved and handed out in, and the unit
/// `units_per_shard` is a multiple of. 256 KiB.
pub const PAGE_UNITS: u32 = 4096;
/// The slot sizes, in units, that pages are binned by; a slot takes the smallest that holds
/// it.
pub const BINS: [u32; 10] = [2, 3, 4, 6, 8, 12, 16, 20, 24, 36];
/// How many bins there are.
pub const BIN_COUNT: usize = BINS.len();
/// Most storage buffers the pool is sharded over.
pub const MAX_SHARDS: u32 = 4;
/// Storage buffers the frame's passes bind besides the pool shards: records, table,
/// materials, light samples and the emission scale.
pub const OTHER_STORAGE_BUFFERS: u32 = 5;
/// `u32` words of a slot's opaque mask (its unit 0): one bit per voxel.
pub const OPAQUE_WORDS: usize = 16;
/// The header unit's words: the palette (16 `u16`), the two translucent words (one bit per
/// 2^3 block), the surface prefix (8 `u16`, the surface voxels below each layer), then the
/// attribute units in the low half of a word and the flags in its high half.
pub const HEADER_PALETTE_WORD: usize = 0;
pub const HEADER_TRANSLUCENT_WORD: usize = 8;
pub const HEADER_PREFIX_WORD: usize = 10;
pub const HEADER_ATTR_WORD: usize = 14;
/// Most units a full chunk's brick's contour attributes take: its surface mask and 512 `u16`.
pub const MAX_ATTR_UNITS: u32 = 17;
/// The header's flag (in the high half of `HEADER_ATTR_WORD`): the slot carries a surface
/// mask.
pub const HEADER_HAS_SURFACE: u32 = 1;
/// Palette entries a slot's header holds.
pub const PALETTE_ENTRIES: usize = 16;
/// `u32` words of brick bits at the head of a chunk record: bit `b` of word `b / 32` is set
/// when record entry `b` is not uniform air.
pub const BITS_WORDS: usize = CHUNK_BRICKS / 32;
/// The record word of brick entry 0.
pub const ENTRY_WORD: usize = BITS_WORDS;
/// `u32` words of one chunk record: the brick bits, then 512 brick entries.
pub const RECORD_WORDS: usize = ENTRY_WORD + CHUNK_BRICKS;
/// Bytes of one chunk record.
pub const RECORD_BYTES: u64 = (RECORD_WORDS * 4) as u64;
/// Bytes one page of the pool reserves.
pub const PAGE_BYTES: u64 = PAGE_UNITS as u64 * UNIT_BYTES;
/// Bytes a reservation sets aside for placeholders before it counts pages: four one-unit
/// shards, so the buffers never exceed the budget.
pub const PLACEHOLDER_BYTES: u64 = MAX_SHARDS as u64 * UNIT_BYTES;
/// The default budget of the pool and records: the world line of `nfr.md` §3, 3 GiB.
pub const DEFAULT_BUDGET_BYTES: u64 = 3 << 30;
/// Most units `sync` stages on the CPU before writing a run: 2 MiB.
const STAGE_UNITS: u32 = 8 * PAGE_UNITS;
/// Most records staged before writing a run: 4 MiB.
const STAGE_RECORDS: u32 = 2048;
/// Changed table entries closer than this many entries are written in one run.
const TABLE_GAP: u32 = 64;
/// Bytes of one staging-belt chunk (ADR-0022): above a 2 MiB sync budget plus a frame's
/// records, so a budgeted sync normally fills one chunk.
pub const BELT_CHUNK_BYTES: u64 = 4 << 20;
/// Chunks the belt keeps across syncs. A sync that leaves it holding more (an unbudgeted
/// first sync of a large world) replaces it with an empty one, so the chunks of that burst
/// are released once their copies complete.
pub const BELT_KEEP_CHUNKS: usize = 3;
/// Chunk table entry of a chunk with no record.
pub const ABSENT: u32 = u32::MAX;
/// Chunk record entry bit: set, the entry names a slot (its class and first unit); clear,
/// the low bits are a uniform brick's material.
pub const FULL_BIT: u32 = 1 << 31;
/// Brick entry bit: the brick holds a `TRANSLUCENT` voxel (a uniform brick's material is one).
/// The opaque walk reads it from the entry it loads anyway, so the translucent walk runs only
/// for a ray that passed such a brick. A planar `Full` brick with it carries a translucent
/// unit.
pub const TRANSLUCENT_BIT: u32 = 1 << 29;
/// The material of a uniform brick's entry.
pub const MATERIAL_MASK: u32 = 0xFFFF;
/// Where a `Full` entry's class lies: bits 26..29, [`class_code`].
pub const CLASS_SHIFT: u32 = 26;
pub const CLASS_MASK: u32 = 7;
/// The first unit of the slot in a `Full` entry: 26 bits, 4 GiB of units.
pub const UNIT_MASK: u32 = (1 << CLASS_SHIFT) - 1;
/// The class code of a `Wide` brick: 16 units of raw ids, no palette.
pub const CLASS_WIDE: u32 = 5;
/// Chunk table entry bit: the record is a LOD record.
pub const LOD_BIT: u32 = 1 << 30;
/// Chunk table entry bit: every entry of the record is uniform air, so its brick bits are
/// all clear. Kept equal to the record's bits on every write of the record.
pub const EMPTY_BIT: u32 = 1 << 29;
/// The record index in a chunk table entry that is not `ABSENT`.
pub const RECORD_MASK: u32 = EMPTY_BIT - 1;
/// First entry of each mip level in a LOD record, level 1 first.
pub const LOD_LEVEL_ENTRY: [usize; MIP_LEVELS] = [0, 64, 72];
/// `f32`s per material row: albedo rgb, roughness, emission rgb, flags as bits, detail scale
/// and strength, opacity, the fine detail octave's weight, 4 spare.
pub const MATERIAL_FLOATS: usize = 16;
/// The attribute of a voxel with no contour plane.
pub const NO_PLANE: u16 = 0xFFFF;

/// The entry code of packed class `c`: 1 to 4 for 0, 1, 2 and 4 planes, [`CLASS_WIDE`].
pub fn class_code(c: PackedClass) -> u32 {
    match c {
        PackedClass::Mask => 1,
        PackedClass::Plane1 => 2,
        PackedClass::Plane2 => 3,
        PackedClass::Plane4 => 4,
        PackedClass::Wide => CLASS_WIDE,
    }
}

/// Units of a slot's payload after its mask and header, by entry class code: its planes, or
/// 16 units of raw ids.
pub fn payload_units(code: u32) -> u32 {
    match code {
        2 => 1,
        3 => 2,
        4 => 4,
        CLASS_WIDE => 16,
        _ => 0,
    }
}

/// The unit offset, from a slot's first unit, of the units after its payload and its
/// translucent unit: where its attributes start. Read from the `Full` entry alone.
pub fn attr_offset(entry: u32) -> u32 {
    let code = entry >> CLASS_SHIFT & CLASS_MASK;
    let translucent = entry & TRANSLUCENT_BIT != 0 && code != CLASS_WIDE;
    2 + payload_units(code) + u32::from(translucent)
}

/// Units of the contour attributes of a brick with `n` surface voxels: none for `n` 0, else
/// its surface mask and `ceil(n / 32)` units of `u16` attributes.
pub fn surface_attr_units(n: u32) -> u32 {
    if n == 0 { 0 } else { 1 + n.div_ceil(32) }
}

/// The units of the slot of `p` (module docs): mask, header, payload, a translucent unit for a
/// planar brick holding a voxel of a material `translucent` names, and with `surface` (its
/// surface voxels, for a brick carrying contour attributes) their attributes.
pub fn slot_units(
    p: &Packed,
    translucent: impl Fn(MaterialId) -> bool,
    surface: Option<u32>,
) -> u32 {
    let code = class_code(p.class());
    let t = code != CLASS_WIDE && materials_of(p, translucent);
    2 + payload_units(code) + u32::from(t) + surface.map_or(0, surface_attr_units)
}

/// The chunk-local surface of brick `slot` of a chunk's `bricks` ([`vxe_world::surface_mask`]):
/// its non-air voxels with an air face neighbour inside the chunk, or on the chunk's face.
pub fn chunk_surface(bricks: &[Brick], slot: usize) -> [u64; 8] {
    let b = brick_of_slot(slot);
    let beside: [[Option<[u64; 8]>; 2]; 3] = std::array::from_fn(|k| {
        std::array::from_fn(|d| {
            let mut n = b;
            n[k] += if d == 0 { -1 } else { 1 };
            (0..CHUNK_EDGE_BRICKS)
                .contains(&n[k])
                .then(|| solid_mask(&bricks[brick_slot(n)]))
        })
    });
    surface_mask(&solid_mask(&bricks[slot]), &beside)
}

/// Which of `len` bricks an upload writes: all when `every`, else the dirty ones and, with
/// `neighbours` (a full chunk's bricks carrying attributes), the face neighbours of each
/// dirty brick within the chunk, whose surface may have changed.
fn upload_set(
    len: usize,
    every: bool,
    neighbours: bool,
    is_dirty: impl Fn(usize) -> bool,
) -> Vec<bool> {
    if every {
        return vec![true; len];
    }
    let mut set: Vec<bool> = (0..len).map(&is_dirty).collect();
    if neighbours {
        for i in 0..len {
            if !is_dirty(i) {
                continue;
            }
            let b = brick_of_slot(i);
            for k in 0..3 {
                for dir in [-1, 1] {
                    let mut n = b;
                    n[k] += dir;
                    if (0..CHUNK_EDGE_BRICKS).contains(&n[k]) {
                        set[brick_slot(n)] = true;
                    }
                }
            }
        }
    }
    set
}

/// Set bits of a surface mask.
fn popcount(m: &[u64; 8]) -> u32 {
    m.iter().map(|w| w.count_ones()).sum()
}

/// The bin index of a slot of `units` units.
pub fn bin_of(units: u32) -> usize {
    BINS.iter()
        .position(|&b| b >= units)
        .expect("a slot fits the largest bin")
}

/// Slots of bin `k` one page holds.
pub fn slots_per_page(k: usize) -> u32 {
    PAGE_UNITS / BINS[k]
}

/// The slot allocator of one bin: a cursor through its current page and a free list.
#[derive(Debug, Default)]
struct BinAlloc {
    /// The first unit of the next never-used slot of the current page, and the end of the
    /// page's last slot.
    next: u32,
    end: u32,
    /// Slots of this bin holding a brick.
    live: u32,
    /// First units of free slots in pages of this bin that still hold a live slot.
    free: Vec<u32>,
}

impl BinAlloc {
    /// Slots this bin can hand out without a new page.
    fn available(&self, k: usize) -> u64 {
        u64::from((self.end - self.next) / BINS[k]) + self.free.len() as u64
    }
}

/// One bin's occupancy ([`GpuWorld::bin_stats`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BinStats {
    /// Units per slot.
    pub units: u32,
    /// Slots holding a brick.
    pub live: u32,
    /// Slots handed out to no brick: the free list and the rest of the cursor's page.
    pub spare: u64,
    /// Pages of this bin.
    pub pages: u32,
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
    /// Pool, mask or record buffers this sync created: 0, since `new` reserves them all.
    pub buffers_created: u32,
    /// Copies between mirror buffers this sync issued: 0, since no buffer is ever replaced
    /// (the belt's copies are counted in `writes`).
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
    /// Caps the units per pool shard below what the device allows; for tests.
    pub max_shard_units: Option<u32>,
    /// VRAM reserved for the pool shards and records at creation, split by the
    /// device's limits ([`ShardLayout::with_budget`]). The chunk table and material rows are
    /// not part of it.
    pub budget_bytes: u64,
    /// Chunk records to reserve; `None` reserves one for every chunk of the window. A
    /// following window's residency is a ball, so a caller that knows it passes fewer.
    pub record_chunks: Option<u64>,
    /// Most bricks one attribute pass computes from its carry-over queue
    /// ([`GpuWorld::prepare_attrs`]); `usize::MAX` for no cap.
    pub attr_cap: usize,
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
    /// A window with no sync budget, no shard cap, no attribute cap and the default budget,
    /// [`DEFAULT_BUDGET_BYTES`], clamped to what the device can bind.
    pub fn new(window: Window) -> Self {
        Self {
            window,
            sync_budget_bytes: u64::MAX,
            max_shard_units: None,
            budget_bytes: DEFAULT_BUDGET_BYTES,
            record_chunks: None,
            attr_cap: usize::MAX,
            light_samples: DEFAULT_LIGHT_SAMPLES,
            light_chunk_cap: DEFAULT_LIGHT_CHUNK_CAP,
            light_radius: DEFAULT_LIGHT_RADIUS,
            light_sync_samples: DEFAULT_LIGHT_SYNC_SAMPLES,
        }
    }
}

/// The smallest budget whose split reserves at least `units` pool units (rounded up to whole
/// pages) and a record for each of `window_chunks` chunks, when the device can bind them.
pub fn budget_for(units: u64, window_chunks: u64) -> u64 {
    window_chunks * RECORD_BYTES
        + units.div_ceil(u64::from(PAGE_UNITS)) * PAGE_BYTES
        + PLACEHOLDER_BYTES
}

/// How the pool is split over storage buffers on a device, and what a budget reserves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShardLayout {
    /// The largest multiple of `PAGE_UNITS` whose bytes fit one binding and one buffer, at
    /// most `max_shard_units` rounded down to a page.
    pub units_per_shard: u32,
    /// `min(MAX_SHARDS, max_storage_buffers_per_shader_stage - OTHER_STORAGE_BUFFERS)`.
    pub shards: u32,
    /// Chunk records the one record buffer can hold:
    /// `min(max_storage_buffer_binding_size, max_buffer_size) / RECORD_BYTES`.
    pub record_capacity: u32,
    /// Pool units reserved, a whole number of pages, at most [`ShardLayout::capacity`] and at
    /// most what an entry's 26-bit unit index addresses.
    pub reserved_units: u32,
    /// Chunk records reserved, at most `record_capacity`.
    pub reserved_records: u32,
}

impl ShardLayout {
    /// The layout `limits` allow, reserving all of it. Reads only the limits, never the
    /// backend or vendor.
    pub fn from_limits(limits: &wgpu::Limits, max_shard_units: Option<u32>) -> Self {
        let bytes = limits
            .max_storage_buffer_binding_size
            .min(limits.max_buffer_size);
        let page = u64::from(PAGE_UNITS);
        let mut units = bytes / UNIT_BYTES;
        if let Some(cap) = max_shard_units {
            units = units.min(u64::from(cap));
        }
        let units_per_shard = ((units / page) * page).clamp(page, u64::from(UNIT_MASK) + 1) as u32;
        let shards = limits
            .max_storage_buffers_per_shader_stage
            .saturating_sub(OTHER_STORAGE_BUFFERS)
            .clamp(1, MAX_SHARDS);
        let record_capacity = (bytes / RECORD_BYTES).min(u64::from(u32::MAX)) as u32;
        let capacity = u64::from(units_per_shard) * u64::from(shards);
        Self {
            units_per_shard,
            shards,
            record_capacity,
            reserved_units: capacity.min(u64::from(UNIT_MASK) + 1) as u32,
            reserved_records: record_capacity,
        }
    }

    /// Units the pool can hold over all its shards.
    pub fn capacity(&self) -> u64 {
        u64::from(self.units_per_shard) * u64::from(self.shards)
    }

    /// This layout reserving `budget_bytes`: past [`PLACEHOLDER_BYTES`], a record for each of
    /// `window_chunks` chunks, at most `record_capacity` and at most what the budget holds;
    /// then whole pages ([`PAGE_BYTES`] each) in what is left, at most the reservation's cap.
    /// The reserved bytes never exceed the budget unless it is smaller than one record and the
    /// placeholders.
    pub fn with_budget(self, budget_bytes: u64, window_chunks: u64) -> Self {
        let budget = budget_bytes.saturating_sub(PLACEHOLDER_BYTES);
        let records = window_chunks
            .min(u64::from(self.record_capacity))
            .min(budget / RECORD_BYTES)
            .max(1);
        let rest = budget.saturating_sub(records * RECORD_BYTES);
        let page = u64::from(PAGE_UNITS);
        let units = (rest / PAGE_BYTES * page).min(u64::from(self.reserved_units));
        Self {
            reserved_units: units as u32,
            reserved_records: records as u32,
            ..self
        }
    }

    /// Pages reserved.
    pub fn reserved_pages(&self) -> u32 {
        self.reserved_units / PAGE_UNITS
    }

    /// Bytes of the buffers this layout reserves: the pool shards (a placeholder unit for a
    /// shard holding none) and the records.
    pub fn reserved_bytes(&self) -> u64 {
        let ups = u64::from(self.units_per_shard);
        let units = u64::from(self.reserved_units);
        let shards: u64 = (0..u64::from(MAX_SHARDS))
            .map(|k| shard_units(units, ups, k).max(1) * UNIT_BYTES)
            .sum();
        shards + u64::from(self.reserved_records) * RECORD_BYTES
    }
}

/// The units shard `k` holds of `units` reserved, shards filling in order at `ups` each.
fn shard_units(units: u64, ups: u64, k: u64) -> u64 {
    units.saturating_sub(k * ups).min(ups)
}

/// What the mirror keeps for a mirrored chunk besides its record entries.
struct Mirrored {
    record: u32,
    /// A LOD record, from the chunk's mips.
    lod: bool,
    /// Whether a brick uploaded into this record has held a material with emission: its
    /// bricks are then scanned for light samples whenever they or a neighbour change.
    emits: bool,
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
    /// New slots of each bin its upload takes.
    new: [u64; BIN_COUNT],
}

/// The pool and records as a sync's selection leaves them, replayed chunk by chunk in the
/// order `upload` places them. A chunk changing tier first frees its old record's slots:
/// credited to their bin's spare slots, or, for a page those frees empty, as a free page less
/// the page's spare slots. Then its new slots take spare slots of their bin, then free pages,
/// then pages never handed out. No free is credited before the chunk that makes it. Once a
/// bin has taken spare slots (its free list or cursor), a later free of that bin is credited
/// as a slot, never as an emptied page: the allocator hands just-freed slots back first, so an
/// earlier allocation may sit on the page. A brick that moves bins inside a record frees its
/// old slot after taking its new one; that free is never credited.
struct Fit {
    /// Spare slots of each bin.
    avail: [u64; BIN_COUNT],
    free_pages: u64,
    /// Pages past the pages handed out.
    fresh: u64,
    records: u64,
    /// Slots the placed chunks free, per page.
    freed: HashMap<u32, u32>,
    /// Whether each bin has taken spare slots.
    took: [bool; BIN_COUNT],
}

impl Fit {
    fn new(gw: &GpuWorld) -> Self {
        Self {
            avail: std::array::from_fn(|k| gw.bins[k].available(k)),
            free_pages: gw.free_pages.len() as u64,
            fresh: 0,
            records: gw.mirrored.len() as u64,
            freed: HashMap::new(),
            took: [false; BIN_COUNT],
        }
    }

    /// Places one chunk: `frees` (the record of a tier change's old record), `new` slots of
    /// each bin, and a new record when `new_record`. Changes nothing when the chunk does not
    /// fit, and returns what it would have needed.
    fn place(
        &mut self,
        gw: &GpuWorld,
        frees: Option<u32>,
        new: [u64; BIN_COUNT],
        new_record: bool,
    ) -> Result<(), MirrorError> {
        let reserved = u64::from(gw.layout.reserved_records);
        if new_record && self.records + 1 > reserved {
            return Err(MirrorError::RecordsTooLarge {
                needed: (self.records + 1) * RECORD_BYTES,
                limit: reserved * RECORD_BYTES,
            });
        }
        let (mut avail, mut free_pages, mut fresh) = (self.avail, self.free_pages, self.fresh);
        let mut took = self.took;
        let mut local: HashMap<u32, u32> = HashMap::new();
        if let Some(rec) = frees {
            let base = rec as usize * RECORD_WORDS;
            for &e in &gw.entries[base + ENTRY_WORD..base + RECORD_WORDS] {
                if e & FULL_BIT == 0 {
                    continue;
                }
                let p = (e & UNIT_MASK) / PAGE_UNITS;
                let k = gw.page_bin[p as usize].expect("a live slot's page has a bin") as usize;
                let f = local.entry(p).or_insert(0);
                *f += 1;
                let total = *f + self.freed.get(&p).copied().unwrap_or(0);
                if !self.took[k] && u32::from(gw.page_live[p as usize]) == total {
                    // The page empties: its spare slots, and the ones credited from it so
                    // far, leave the bin; the page becomes free for any bin.
                    let credited = total - 1;
                    avail[k] =
                        avail[k].saturating_sub(gw.spare_in_page(k, p) + u64::from(credited));
                    free_pages += 1;
                } else {
                    avail[k] += 1;
                }
            }
        }
        for (k, &n) in new.iter().enumerate() {
            let take = n.min(avail[k]);
            avail[k] -= take;
            took[k] |= take > 0;
            let mut rest = n - take;
            let per = u64::from(slots_per_page(k));
            while rest > 0 {
                if free_pages > 0 {
                    free_pages -= 1;
                } else {
                    fresh += 1;
                }
                let used = rest.min(per);
                avail[k] += per - used;
                rest -= used;
            }
        }
        let need = u64::from(gw.next_page) + fresh;
        let limit = u64::from(gw.layout.reserved_pages());
        if need > limit {
            return Err(MirrorError::PoolTooLarge {
                needed: need * PAGE_BYTES,
                limit: limit * PAGE_BYTES,
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
    /// The device's layout and the reservation: `reserved_units` and `reserved_records`.
    layout: ShardLayout,
    /// One buffer per shard, created at its reserved size; a shard holding no reserved unit
    /// is a one-unit placeholder.
    shards: Vec<wgpu::Buffer>,
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
    /// Whether full chunks' `Full` bricks carry contour attributes.
    surface_attrs: bool,
    /// The allocator of each bin.
    bins: [BinAlloc; BIN_COUNT],
    /// The bin of every page handed out, `None` while it is free, and its live slots.
    page_bin: Vec<Option<u8>>,
    page_live: Vec<u16>,
    /// Pages handed out whose live slots fell to zero, taken lowest first by any bin.
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
    /// The work list `prepare_attrs` built and `encode_attrs` records, kept for its capacity,
    /// and the counters of its build.
    attr_items: Vec<AttribItem>,
    attr_stats: AttrWorkStats,
    /// The attribute carry-over queue, oldest first, each entry with the `prepare_attrs` call
    /// that queued it; the chunk part-way expanded, its next brick slot and its call; and
    /// what the queue holds, for deduplication.
    attr_queue: VecDeque<(AttrWait, u64)>,
    attr_cursor: Option<(ChunkCoord, usize, u64)>,
    queued_chunks: HashSet<ChunkCoord>,
    queued_bricks: HashSet<[i32; 3]>,
    /// The bricks of the current list, compute and front items, for deduplication.
    attr_taken: HashSet<[i32; 3]>,
    /// `prepare_attrs` calls so far.
    attr_calls: u64,
    /// The `prepare_attrs` call before the last fit change: every brick queued at or before it
    /// waits for the new fit.
    refit_call: Option<u64>,
    /// Rows written in place of the world's table's at the first sync, from
    /// [`GpuWorld::rewrite_materials`] before it.
    material_rows_from: Option<MaterialTable>,
    /// The queue of the last `sync`, which writes the attribute pass's uniform and work list.
    queue: Option<wgpu::Queue>,
    /// Whether each material is `TRANSLUCENT`, from the table at the first sync.
    translucent: Vec<bool>,
    /// Whether each material has non-zero emission, from the table at the first sync.
    emissive: Vec<bool>,
    /// The light-sample buffer, its free ranges (start to length) below `light_high`, the
    /// first sample never handed out, and each chunk's samples.
    light_buf: wgpu::Buffer,
    light_free: BTreeMap<u32, u32>,
    light_high: u32,
    lights: HashMap<ChunkCoord, ChunkLights>,
    light_live: u64,
    /// Ranges of chunks dropped outside a sync, zeroed and freed at the next one.
    light_dropped: Vec<(u32, u32)>,
    /// Chunks whose lists changed since their range was written; chunks inside the radius
    /// waiting for a range, with the sync they started waiting at; syncs so far.
    light_dirty: HashSet<ChunkCoord>,
    light_wait: HashMap<ChunkCoord, u64>,
    light_syncs: u64,
    /// Each material's emitted luminance, from the table at the first sync.
    emit_lum: Vec<f32>,
    /// Whether each material is opaque (neither air nor `TRANSLUCENT`).
    opaque: Vec<bool>,
    /// Chunks waiting for an upload, keyed by squared distance to `pending_focus`.
    pending: BTreeSet<(i64, ChunkCoord)>,
    pending_d2: HashMap<ChunkCoord, i64>,
    pending_focus: ChunkCoord,
    bytes_written: u64,
    belt: Belt,
    /// Per bin, the run of consecutive pool units staged for one write: its first unit and
    /// its words. A fresh chunk's bricks interleave bins but take consecutive slots of each,
    /// so each bin's run stays long. Kept across syncs so staging allocates only while it
    /// first grows.
    stage: [(u32, Vec<u32>); BIN_COUNT],
    /// One slot's words as `upload_brick` builds them.
    slot_words: Vec<u32>,
    /// The surface masks of the chunk being selected or uploaded, by brick slot
    /// ([`GpuWorld::need`]).
    surfaces: Vec<Option<[u64; 8]>>,
    /// The run of consecutive records staged for one write.
    rec_stage_start: u32,
    rec_stage: Vec<u32>,
    /// The chunks `select` chose and the stale ones it unqueues, cleared at the start of
    /// each `select` and kept for their capacity.
    selected: Vec<Selected>,
    stale: Vec<ChunkCoord>,
}

/// The mirror's staging belt: `wgpu`'s mapped chunks sub-allocated per staged run, each run
/// copied to its destination by a command recorded into the sync's encoder (ADR-0022).
struct Belt {
    device: wgpu::Device,
    belt: wgpu::util::StagingBelt,
    /// Every chunk buffer the current belt has handed out a slice of: the belt never drops a
    /// chunk, so this is its chunk count, active, closed, in flight and free.
    chunks: Vec<wgpu::Buffer>,
    /// Times the belt was replaced for holding more than [`BELT_KEEP_CHUNKS`].
    trims: u64,
}

impl Belt {
    fn new(device: &wgpu::Device) -> Self {
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
    fn write(
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
    fn finish(&mut self, enc: &wgpu::CommandEncoder) {
        self.belt.finish_and_recall_on_submit(enc);
        if self.chunks.len() > BELT_KEEP_CHUNKS {
            self.belt = wgpu::util::StagingBelt::new(self.device.clone(), BELT_CHUNK_BYTES);
            self.chunks.clear();
            self.trims += 1;
        }
    }
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
        let ups = u64::from(layout.units_per_shard);
        let reserved = u64::from(layout.reserved_units);
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
            cfg,
            window_min,
            window_size,
            focus,
            initialized: false,
            layout,
            shards: (0..u64::from(MAX_SHARDS))
                .map(|k| Self::create_pool(device, shard_units(reserved, ups, k) as u32))
                .collect(),
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
            bins: Default::default(),
            page_bin: Vec::new(),
            page_live: Vec::new(),
            free_pages: BTreeSet::new(),
            next_page: 0,
            attr_chunks: HashSet::new(),
            attr_bricks: HashSet::new(),
            shell_chunks: HashSet::new(),
            attrib: None,
            attr_items: Vec::new(),
            attr_stats: AttrWorkStats::default(),
            attr_queue: VecDeque::new(),
            attr_cursor: None,
            queued_chunks: HashSet::new(),
            queued_bricks: HashSet::new(),
            attr_taken: HashSet::new(),
            attr_calls: 0,
            refit_call: None,
            material_rows_from: None,
            queue: None,
            translucent: vec![false; 1 << 16],
            emissive: vec![false; 1 << 16],
            light_buf: Self::storage_buffer(
                device,
                "light_samples",
                (u64::from(cfg.light_samples) + LIGHT_TABLE_SLOTS) * LIGHT_SAMPLE_BYTES,
            ),
            light_free: BTreeMap::new(),
            light_high: 0,
            lights: HashMap::new(),
            light_live: 0,
            light_dropped: Vec::new(),
            light_dirty: HashSet::new(),
            light_wait: HashMap::new(),
            light_syncs: 0,
            emit_lum: vec![0.0; 1 << 16],
            opaque: vec![false; 1 << 16],
            pending: BTreeSet::new(),
            pending_d2: HashMap::new(),
            pending_focus: focus,
            bytes_written: 0,
            belt: Belt::new(device),
            stage: Default::default(),
            slot_words: Vec::new(),
            surfaces: Vec::new(),
            rec_stage_start: 0,
            rec_stage: Vec::new(),
            selected: Vec::new(),
            stale: Vec::new(),
        }
    }

    /// A mirror under `cfg` whose full chunks' `Full` bricks carry contour attributes in
    /// their slots, with the pass that fills them ([`GpuWorld::update_attrs`]).
    pub fn with_surface_attrs(device: &wgpu::Device, cfg: MirrorConfig) -> Self {
        let mut m = Self::with_config(device, cfg);
        m.surface_attrs = true;
        let mut pass = AttribPass::new(device);
        if cfg.attr_cap != usize::MAX && cfg.sync_budget_bytes != u64::MAX {
            // A capped list: the cap, then a clear item for every brick one sync writes, its
            // budget's worth of the smallest slots and the one chunk it may finish past it.
            let bricks =
                cfg.sync_budget_bytes / (u64::from(BINS[0]) * UNIT_BYTES) + CHUNK_BRICKS as u64;
            pass.reserve(cfg.attr_cap.saturating_add(bricks as usize));
        }
        m.attrib = Some(pass);
        m
    }

    /// Whether full chunks' bricks carry contour attributes.
    pub fn surface_attrs(&self) -> bool {
        self.surface_attrs
    }

    /// The plane fit the attribute pass computes from its next call on. A change re-queues
    /// every chunk mirrored full through the carry-over queue, nearest the focus first, so
    /// the refit spreads under [`MirrorConfig::attr_cap`]; until its turn a brick keeps the
    /// attribute it was computed with, valid under the previous fit
    /// ([`GpuWorld::refitting`]). A mirror without contour attributes ignores it.
    pub fn set_surface_fit(&mut self, fit: SurfaceFit) {
        let Some(pass) = &mut self.attrib else {
            return;
        };
        if pass.fit() == fit {
            return;
        }
        pass.set_fit(fit);
        let f = self.focus.0;
        let mut chunks: Vec<ChunkCoord> = self
            .mirrored
            .iter()
            .filter(|(_, m)| !m.lod)
            .map(|(&c, _)| c)
            .collect();
        chunks.sort_by_key(|c| {
            let d = [0, 1, 2].map(|k| i64::from(c.0[k] - f[k]));
            (d[0] * d[0] + d[1] * d[1] + d[2] * d[2], c.0)
        });
        let call = self.attr_calls;
        for c in chunks {
            if self.queued_chunks.insert(c) {
                self.attr_queue.push_back((AttrWait::Chunk(c), call));
            }
        }
        // A mirror with nothing filled yet has nothing to refit.
        if self.mirrored.values().any(|m| !m.lod) {
            self.refit_call = Some(call);
        }
    }

    /// Whether a brick queued by the last fit change still waits for the new fit.
    pub fn refitting(&self) -> bool {
        let (_, oldest) = self.attr_queue_age();
        self.refit_call
            .is_some_and(|r| oldest.is_some_and(|o| o <= r))
    }

    /// Rewrites the material rows from `table`, a table of the same rows as the world's at the
    /// first sync with other renderer fields (albedo, roughness, detail, opacity): the
    /// translucent, emissive and opaque flags the upload reads stay the first sync's. Before
    /// the first sync the rows wait for it.
    ///
    /// # Panics
    ///
    /// If the rows are written and `table` has a different number of rows.
    pub fn rewrite_materials(&mut self, table: &MaterialTable, queue: &wgpu::Queue) {
        if !self.materials_written {
            self.material_rows_from = Some(table.clone());
            return;
        }
        assert_eq!(
            table.len() as u64,
            self.material_rows,
            "a rewrite keeps the table's rows"
        );
        queue.write_buffer(
            &self.materials,
            0,
            bytemuck::cast_slice(&material_rows(table)),
        );
    }

    /// The attribute pass's plane fit; the default for a mirror without contour attributes.
    pub fn surface_fit(&self) -> SurfaceFit {
        self.attrib
            .as_ref()
            .map(AttribPass::fit)
            .unwrap_or_default()
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

    fn create_pool(device: &wgpu::Device, units: u32) -> wgpu::Buffer {
        Self::storage_buffer(device, "brick_pool", u64::from(units.max(1)) * UNIT_BYTES)
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
            // COPY_SRC for readback, as the mirror's other storage buffers.
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
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

    /// Pool shard `k`, for `k < MAX_SHARDS`; a shard holding no reserved unit is a one-unit
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
        self.pool_units()
            .div_ceil(self.layout.units_per_shard)
            .try_into()
            .unwrap_or(usize::MAX)
    }

    /// Pool units reserved, a whole number of pages.
    pub fn reserved_units(&self) -> u32 {
        self.layout.reserved_units
    }

    /// Chunk records reserved.
    pub fn reserved_records(&self) -> u32 {
        self.layout.reserved_records
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

    /// Units of the pages handed out so far, from unit 0: the pool's high-water mark, at most
    /// `reserved_units`.
    pub fn pool_units(&self) -> u32 {
        self.next_page * PAGE_UNITS
    }

    /// Slots currently holding a `Full` brick, over every bin.
    pub fn live_bricks(&self) -> u32 {
        self.bins.iter().map(|b| b.live).sum()
    }

    /// Units of the slots currently holding a `Full` brick: each slot's whole bin.
    pub fn live_units(&self) -> u64 {
        self.bins
            .iter()
            .zip(BINS)
            .map(|(b, n)| u64::from(b.live) * u64::from(n))
            .sum()
    }

    /// Units of the slots handed out to no brick: every bin's free slots and the rest of its
    /// cursor's page. The units of free pages, and the tail of each page past its last whole
    /// slot, are not counted.
    pub fn spare_units(&self) -> u64 {
        self.bins
            .iter()
            .enumerate()
            .map(|(k, b)| b.available(k) * u64::from(BINS[k]))
            .sum()
    }

    /// Slots handed out to no brick, over every bin: free slots and the rest of each cursor's
    /// page.
    pub fn spare_slots(&self) -> u64 {
        self.bins
            .iter()
            .enumerate()
            .map(|(k, b)| b.available(k))
            .sum()
    }

    /// Each bin's slot size, live and spare slots, and pages.
    pub fn bin_stats(&self) -> [BinStats; BIN_COUNT] {
        std::array::from_fn(|k| BinStats {
            units: BINS[k],
            live: self.bins[k].live,
            spare: self.bins[k].available(k),
            pages: self
                .page_bin
                .iter()
                .filter(|b| **b == Some(k as u8))
                .count() as u32,
        })
    }

    /// The bin of the page holding unit `u`, `None` for a free or never-issued page.
    pub fn bin_of_unit(&self, u: u32) -> Option<usize> {
        self.page_bin
            .get((u / PAGE_UNITS) as usize)
            .copied()
            .flatten()
            .map(usize::from)
    }

    /// The free slots of bin `k`, in the order they will be reused last-first.
    pub fn free_slots(&self, k: usize) -> &[u32] {
        &self.bins[k].free
    }

    /// Pages ever handed out from the reservation, free ones included: the pool's high-water
    /// mark in pages.
    pub fn pages_issued(&self) -> u32 {
        self.next_page
    }

    /// Pages handed out whose live slots fell to zero, ascending: free for any bin.
    pub fn free_pages(&self) -> Vec<u32> {
        self.free_pages.iter().copied().collect()
    }

    /// Bytes written to the GPU over the mirror's life.
    pub fn bytes_written(&self) -> u64 {
        self.bytes_written
    }

    /// The staging belt's chunks, active, closed, in flight and free, each
    /// [`BELT_CHUNK_BYTES`] (or one run's size, past it): host memory held mapped for the
    /// mirror's uploads.
    pub fn belt_chunks(&self) -> usize {
        self.belt.chunks.len()
    }

    /// Times a sync left the belt holding more than [`BELT_KEEP_CHUNKS`] chunks and replaced
    /// it.
    pub fn belt_trims(&self) -> u64 {
        self.belt.trims
    }

    /// Whether `coord` is mirrored from its mips.
    pub fn is_lod(&self, coord: ChunkCoord) -> bool {
        self.mirrored.get(&coord).is_some_and(|m| m.lod)
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

    /// Bytes the mirror occupies on the GPU by the uploader's live accounting: the units of
    /// live slots (each slot's whole bin), the records of mirrored chunks, chunk table and
    /// material rows. Spare slots, free pages and free records are not counted;
    /// `allocated_bytes` is what the reserved buffers hold.
    pub fn mirror_bytes(&self) -> u64 {
        self.live_units() * UNIT_BYTES
            + self.mirrored.len() as u64 * RECORD_BYTES
            + self.table_cpu.len() as u64 * 4
            + self.material_rows * MATERIAL_FLOATS as u64 * 4
    }

    /// Bytes of the reserved buffers: every pool shard (placeholders included) and the
    /// records, which is the budget rounded down to whole pages and records
    /// ([`ShardLayout::reserved_bytes`]). Fixed from creation: no sync, eviction or window
    /// move changes it. The chunk table and material rows are outside the reservation.
    pub fn allocated_bytes(&self) -> u64 {
        self.shards.iter().map(wgpu::Buffer::size).sum::<u64>() + self.records.size()
    }

    /// The mirror's pressure, the larger of its two occupancies (ADR-0013): pool units in
    /// use, `live_units() / reserved_units()`, and records in use, `mirrored_chunks() /
    /// reserved_records()`. The application decides what to do as it nears 1; a sync that
    /// would pass the reservation fails instead of growing it. The pool can fail a little
    /// below 1, at each bin's partly filled page and each page's tail.
    pub fn pressure(&self) -> f32 {
        let unit_term = self.live_units() as f64 / f64::from(self.layout.reserved_units.max(1));
        let record_term =
            self.mirrored.len() as f64 / f64::from(self.layout.reserved_records.max(1));
        unit_term.max(record_term) as f32
    }

    /// Records into `encoder` the attribute pass over what changed since the last call: every
    /// `Full` brick of a full chunk that `sync` wrote, the bricks of full chunks among their 26
    /// neighbours, and the bricks of full chunks next to a chunk whose record was written new,
    /// dropped or replaced. Under [`MirrorConfig::attr_cap`] part of that waits for later
    /// calls ([`GpuWorld::prepare_attrs`]). Returns the items in the work list, compute and
    /// clear; 0 records nothing, and so does a mirror without contour attributes. The pass reads
    /// the pool, records and table as earlier submissions and the `sync` copies recorded
    /// ahead of it in `encoder` leave them.
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
        self.prepare_attrs();
        self.encode_attrs(encoder, timestamps)
    }

    /// The first half of [`GpuWorld::update_attrs_timed`]: builds the work list from what
    /// changed since the last one and clears the change sets. The list waits for
    /// [`GpuWorld::encode_attrs`]; a second call before it replaces the list. Returns the
    /// build's counters, also kept for [`GpuWorld::attr_stats`].
    ///
    /// The list, in order:
    /// - the re-uploaded bricks and their 26 neighbours, never deferred;
    /// - bricks from the carry-over queue, oldest first, until the list holds
    ///   [`MirrorConfig::attr_cap`] compute items. Chunks written whole are queued as their
    ///   coordinate and expanded as they are taken, the shells' bricks one by one. A queued
    ///   brick is resolved to its slot when it is taken, and skipped when its chunk is no
    ///   longer mirrored full or it is not `Full`.
    ///
    /// A brick `sync` wrote whose item waits reads "no plane" (face mode) until its turn: its
    /// upload wrote its attribute units so, whatever its slot held before.
    /// Each brick is listed once per call.
    pub fn prepare_attrs(&mut self) -> AttrWorkStats {
        self.attr_calls += 1;
        let call = self.attr_calls;
        let chunks = std::mem::take(&mut self.attr_chunks);
        let bricks = std::mem::take(&mut self.attr_bricks);
        let shells = std::mem::take(&mut self.shell_chunks);
        let mut items = std::mem::take(&mut self.attr_items);
        let mut taken = std::mem::take(&mut self.attr_taken);
        items.clear();
        taken.clear();
        if !self.surface_attrs {
            self.attr_items = items;
            self.attr_taken = taken;
            self.attr_stats = AttrWorkStats::default();
            return self.attr_stats;
        }
        let mut front = Vec::new();
        let mut rest = Vec::new();
        let mut st =
            attr_work::build_split(&*self, &chunks, &bricks, &shells, &mut front, &mut rest);
        for &c in &chunks {
            if self.queued_chunks.insert(c) {
                self.attr_queue.push_back((AttrWait::Chunk(c), call));
            }
        }
        for it in &rest {
            let b = it.origin.map(|v| v.div_euclid(BRICK_EDGE));
            if self.queued_bricks.insert(b) {
                self.attr_queue.push_back((AttrWait::Brick(b), call));
            }
        }
        for it in front {
            if taken.insert(it.origin.map(|v| v.div_euclid(BRICK_EDGE))) {
                items.push(it);
            }
        }
        let cap = self.cfg.attr_cap;
        let e = CHUNK_EDGE_BRICKS;
        while items.len() < cap {
            if let Some((c, mut i, since)) = self.attr_cursor.take() {
                if !self.is_full_chunk(c) {
                    continue;
                }
                while i < CHUNK_BRICKS && items.len() < cap {
                    let l = brick_of_slot(i);
                    let b = [0, 1, 2].map(|k| c.0[k] * e + l[k]);
                    i += 1;
                    st.item_calls += 1;
                    if !taken.contains(&b)
                        && let Some(slot) = self.full_slot(b)
                    {
                        taken.insert(b);
                        items.push(AttribItem {
                            slot,
                            origin: b.map(|v| v * BRICK_EDGE),
                        });
                    }
                }
                // Past the bricks with nothing to compute, so a cursor waits only on work.
                while i < CHUNK_BRICKS {
                    let l = brick_of_slot(i);
                    let b = [0, 1, 2].map(|k| c.0[k] * e + l[k]);
                    if !taken.contains(&b) && self.full_slot(b).is_some() {
                        break;
                    }
                    i += 1;
                }
                if i < CHUNK_BRICKS {
                    self.attr_cursor = Some((c, i, since));
                }
                continue;
            }
            let Some((wait, since)) = self.attr_queue.pop_front() else {
                break;
            };
            match wait {
                AttrWait::Chunk(c) => {
                    self.queued_chunks.remove(&c);
                    self.attr_cursor = Some((c, 0, since));
                }
                AttrWait::Brick(b) => {
                    self.queued_bricks.remove(&b);
                    if !taken.contains(&b)
                        && let Some(slot) = self.full_slot(b)
                    {
                        taken.insert(b);
                        items.push(AttribItem {
                            slot,
                            origin: b.map(|v| v * BRICK_EDGE),
                        });
                    }
                }
            }
        }
        st.items = items.len() as u32;
        self.attr_items = items;
        self.attr_taken = taken;
        self.attr_stats = st;
        st
    }

    /// The list [`GpuWorld::prepare_attrs`] built, until [`GpuWorld::encode_attrs`] records
    /// it: compute items only. A brick written into a slot whose item waits reads "no plane"
    /// until its turn, from the attribute units its upload wrote.
    pub fn attr_list(&self) -> &[AttribItem] {
        &self.attr_items
    }

    /// `prepare_attrs` calls so far, and the call that queued the oldest brick still waiting
    /// in the carry-over queue (`None` when it is empty). A brick waits `calls - oldest`
    /// calls.
    pub fn attr_queue_age(&self) -> (u64, Option<u64>) {
        let oldest = self
            .attr_cursor
            .map(|c| c.2)
            .or_else(|| self.attr_queue.front().map(|w| w.1));
        (self.attr_calls, oldest)
    }

    /// The attribute pass's work buffers built so far ([`AttribPass::buffer_builds`]); 0
    /// without contour attributes.
    pub fn attr_buffer_builds(&self) -> u32 {
        self.attrib.as_ref().map_or(0, AttribPass::buffer_builds)
    }

    /// The second half of [`GpuWorld::update_attrs_timed`]: records the pass over the list
    /// [`GpuWorld::prepare_attrs`] built, and empties it. Returns the items in the list.
    pub fn encode_attrs(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        timestamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) -> usize {
        let n = self.attr_items.len();
        if let (Some(pass), Some(queue)) = (&mut self.attrib, &self.queue) {
            let e = ChunkExtent::new(
                self.window_min,
                [0, 1, 2].map(|k| self.window_min[k] + self.window_size[k]),
            );
            let b = AttribBindings {
                shards: [0, 1, 2, 3].map(|k| &self.shards[k]),
                records: &self.records,
                table: &self.table,
                window: (e.min.0, e.size()),
                units_per_shard: self.layout.units_per_shard,
            };
            pass.encode(
                &self.device,
                queue,
                encoder,
                &b,
                &self.attr_items,
                timestamps,
            );
        }
        self.attr_items.clear();
        n
    }

    /// The counters of the last work-list build.
    pub fn attr_stats(&self) -> AttrWorkStats {
        self.attr_stats
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
        if self.surface_attrs {
            self.shell_chunks.insert(coord);
        }
        self.drop_lights(coord);
        let base = rec as usize * RECORD_WORDS;
        let mut slots = Vec::new();
        let mut emptied = Vec::new();
        for i in base + ENTRY_WORD..base + RECORD_WORDS {
            let e = self.entries[i];
            if e & FULL_BIT == 0 {
                continue;
            }
            let u = e & UNIT_MASK;
            if self.unlive(u) {
                emptied.push(u / PAGE_UNITS);
            }
            slots.push(u);
        }
        self.entries[base..base + RECORD_WORDS].fill(0);
        for p in emptied {
            self.release_page(p);
        }
        // Slots of a released page are not listed. The rest are pushed highest first, so the
        // next allocations pop them in ascending order and the chunk that reuses them is
        // written in long runs.
        slots.retain(|u| self.page_bin[(u / PAGE_UNITS) as usize].is_some());
        slots.sort_unstable_by(|a, b| b.cmp(a));
        for u in slots {
            let k = self.page_bin[(u / PAGE_UNITS) as usize].expect("held above") as usize;
            self.bins[k].free.push(u);
        }
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
        let leaving = leaving_window(
            old,
            new,
            self.mirrored.len() + self.pending_d2.len(),
            |c| self.mirrored.contains_key(&c) || self.pending_d2.contains_key(&c),
            || self.mirrored.keys().chain(self.pending_d2.keys()).copied(),
        );
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
    ///
    /// Every write is a staging-belt copy recorded into `encoder` (ADR-0022), which the
    /// caller submits before anything reads the mirror: the frame's own encoder ahead of the
    /// attribute pass and the march, or one of its own outside a frame. A sync whose encoder
    /// is never submitted keeps its belt chunks. `queue` is kept for the attribute pass's
    /// own writes.
    pub fn sync(
        &mut self,
        world: &World,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<SyncStats, MirrorError> {
        let result = self.sync_into(world, queue, encoder);
        self.belt.finish(encoder);
        result
    }

    fn sync_into(
        &mut self,
        world: &World,
        queue: &wgpu::Queue,
        enc: &mut wgpu::CommandEncoder,
    ) -> Result<SyncStats, MirrorError> {
        let mut st = SyncStats::default();
        if self.attrib.is_some() && self.queue.is_none() {
            self.queue = Some(queue.clone());
        }
        self.reconcile_window(world);
        self.flush_dropped_lights(enc);
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
            // The flags are read while staging slots; the rows are written below, once a sync
            // writes anything.
            for (id, m) in world.materials().rows().iter().enumerate() {
                self.translucent[id] = m.is_translucent();
                self.emissive[id] = m.emission.iter().any(|&e| e > 0.0);
                self.emit_lum[id] =
                    0.2126 * m.emission[0] + 0.7152 * m.emission[1] + 0.0722 * m.emission[2];
                self.opaque[id] = id != usize::from(AIR) && !m.is_translucent();
            }
        }
        let mut short = short;
        let result = self.upload(&mut short, enc, &mut st);
        if result.is_ok()
            && st.chunks == 0
            && let Some(e) = short
        {
            // The upload's check deferred every chosen chunk before writing any of it: an
            // empty sync. Queued table changes and the material rows wait for the next sync
            // that writes.
            return Err(e);
        }
        if !self.materials_written {
            let table = self.material_rows_from.take();
            self.write_materials(table.as_ref().unwrap_or(world.materials()), enc, &mut st);
            self.materials_written = true;
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
        st.pending_chunks = self.pending.len() as u64;
        self.bytes_written += st.bytes;
        result.map(|()| st)
    }

    /// Whether a `Full` brick of a record of this tier carries contour attributes.
    fn attrs_for(&self, lod: bool) -> bool {
        self.surface_attrs && !lod
    }

    /// The units of `p`'s slot with `surface` voxels' attributes ([`slot_units`]): mask,
    /// header, payload, a translucent unit for a planar brick holding a translucent voxel,
    /// and the attributes.
    fn slot_units(&self, p: &Packed, surface: Option<u32>) -> u32 {
        slot_units(p, |m| self.translucent[usize::from(m)], surface)
    }

    /// What uploading `bricks` into record `rec` (`None`: a new record) takes: the new slots
    /// of each bin (a brick keeps its slot only when its bin is unchanged) and the slot bytes
    /// written, over the bricks [`upload_set`] names, whose surface masks, for a full chunk's
    /// bricks carrying attributes, are left in `surfaces` by brick slot. Reads only; `sync`
    /// calls it to select a chunk and again, under the chunk's write lock, as the upload's
    /// backstop, whose surfaces the upload then writes.
    fn need(
        &self,
        bricks: &Bricks<'_>,
        rec: Option<u32>,
        lod: bool,
        surfaces: &mut Vec<Option<[u64; 8]>>,
    ) -> ([u64; BIN_COUNT], u64) {
        let attrs = self.attrs_for(lod);
        let set = upload_set(bricks.len(), rec.is_none(), attrs, |i| bricks.is_dirty(i));
        surfaces.clear();
        surfaces.resize(bricks.len(), None);
        let mut new = [0u64; BIN_COUNT];
        let mut bytes = 0;
        for (i, &up) in set.iter().enumerate() {
            let Brick::Full(p) = bricks.get(i) else {
                continue;
            };
            if !up {
                continue;
            }
            let surface = match bricks {
                Bricks::Chunk(ch) if attrs => {
                    let s = chunk_surface(ch.bricks(), i);
                    surfaces[i] = Some(s);
                    Some(popcount(&s))
                }
                _ => None,
            };
            let k = bin_of(self.slot_units(p, surface));
            bytes += u64::from(BINS[k]) * UNIT_BYTES;
            let old = rec.map_or(0, |r| {
                self.entries[r as usize * RECORD_WORDS + ENTRY_WORD + i]
            });
            if old & FULL_BIT == 0 || self.bin_of_unit(old & UNIT_MASK) != Some(k) {
                new[k] += 1;
            }
        }
        (new, bytes)
    }

    /// Spare slots of bin `k` in page `p`: its free slots, and the rest of the bin's cursor
    /// when that is in `p`.
    fn spare_in_page(&self, k: usize, p: u32) -> u64 {
        let a = &self.bins[k];
        let free = a.free.iter().filter(|&&u| u / PAGE_UNITS == p).count() as u64;
        let cursor = if a.next < a.end && (a.end - 1) / PAGE_UNITS == p {
            u64::from((a.end - a.next) / BINS[k])
        } else {
            0
        };
        free + cursor
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
        let mut surfaces = std::mem::take(&mut self.surfaces);
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
            let rec = mirrored.filter(|m| m.lod == lod).map(|m| m.record);
            let demotion = mirrored.is_some_and(|m| !m.lod) && lod;
            let mut chunk_bytes = RECORD_BYTES + if rec.is_none() { 4 } else { 0 };
            let new = match &src {
                Src::Full(chunk) => {
                    let ch = chunk.read().unwrap_or_else(PoisonError::into_inner);
                    if rec.is_some() && !ch.any_dirty() {
                        self.stale.push(coord);
                        continue;
                    }
                    let (new, bytes) = self.need(&Bricks::Chunk(&ch), rec, false, &mut surfaces);
                    chunk_bytes += bytes;
                    new
                }
                Src::Lod(mips) => {
                    // Mips never change: a chunk already mirrored LOD-only needs nothing.
                    if rec.is_some() {
                        self.stale.push(coord);
                        continue;
                    }
                    let (new, bytes) = self.need(&Bricks::Mips(mips), None, true, &mut surfaces);
                    chunk_bytes += bytes;
                    new
                }
            };
            // The frees of a tier change: the old record's slots.
            let frees = mirrored.filter(|m| m.lod != lod).map(|m| m.record);
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
        self.surfaces = surfaces;
        let mut stale = std::mem::take(&mut self.stale);
        for &c in &stale {
            self.unqueue(c);
        }
        stale.clear();
        self.stale = stale;
        short
    }

    /// Uploads the chunks `select` chose, stopping at the first failed allocation, and
    /// empties `self.selected`. Each chunk is checked against the allocator as it is then
    /// before anything of it is written or its old record dropped: one that does not fit is
    /// deferred whole, and so is every later chunk but a demotion.
    fn upload(
        &mut self,
        short: &mut Option<MirrorError>,
        enc: &mut wgpu::CommandEncoder,
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
            let frees = mirrored.filter(|m| m.lod != lod).map(|m| m.record);
            let new_record = mirrored.is_none();
            let rec = mirrored.filter(|m| m.lod == lod).map(|m| m.record);
            // A full chunk is held under its write lock from the backstop to its upload, so
            // the need checked is the need written.
            let mut guard = match &src {
                Src::Full(chunk) => Some(chunk.write().unwrap_or_else(PoisonError::into_inner)),
                Src::Lod(_) => None,
            };
            let mut surfaces = std::mem::take(&mut self.surfaces);
            let need = match &guard {
                Some(ch) => self.need(&Bricks::Chunk(ch), rec, false, &mut surfaces).0,
                None => {
                    surfaces.clear();
                    new
                }
            };
            self.surfaces = surfaces;
            // The backstop: this chunk alone against the allocator as it is now. It must be
            // exact for every need it admits, so that no admitted chunk fails part-way: the
            // state is exact since a chunk's own tier-change frees precede its allocations,
            // and a moved brick's free, which follows its allocation, is never credited.
            if let Err(e) = Fit::new(self).place(self, frees, need, new_record) {
                short.get_or_insert(e);
                st.deferred_chunks += 1;
                defer_rest = true;
                continue;
            }
            if self.mirrored.get(&coord).is_some_and(|m| m.lod != lod) {
                self.drop_record(coord);
            }
            result = match (&src, guard.as_mut()) {
                (_, Some(ch)) => {
                    let r = self.upload_record(coord, Bricks::Chunk(ch), false, enc, st);
                    if r.is_ok() {
                        ch.clear_dirty();
                    }
                    r
                }
                (Src::Lod(mips), None) => {
                    self.upload_record(coord, Bricks::Mips(mips), true, enc, st)
                }
                (Src::Full(_), None) => unreachable!("a full chunk is locked above"),
            };
            if result.is_err() {
                break;
            }
        }
        self.selected = selected;
        result
    }

    /// Writes one chunk's record: `bricks` into its entries (512 voxel bricks, or a LOD
    /// record's 73 mip-bricks): every brick when `coord` had no record before this call, else
    /// the dirty ones; then, for a record allocated here, its table entry.
    fn upload_record(
        &mut self,
        coord: ChunkCoord,
        bricks: Bricks<'_>,
        lod: bool,
        enc: &mut wgpu::CommandEncoder,
        st: &mut SyncStats,
    ) -> Result<(), MirrorError> {
        let (rec, kept_emits, new) = match self.mirrored.remove(&coord) {
            Some(m) => (m.record, m.emits, false),
            None => (self.alloc_record()?, false, true),
        };
        let base = rec as usize * RECORD_WORDS;
        if new {
            self.entries[base..base + RECORD_WORDS].fill(0);
        }
        let every = new;
        // The record has held an emissive material while mirrored: without one no brick of it
        // has an emitter.
        let emits = !lod
            && (kept_emits
                || (0..bricks.len()).any(|i| {
                    (every || bricks.is_dirty(i))
                        && matches!(bricks.get(i), Brick::Full(p)
                            if materials_of(p, |m| self.emissive[usize::from(m)]))
                }));
        let attrs = self.attrs_for(lod);
        let mut light_us = 0u64;
        let mut rescan = BTreeSet::new();
        if !emits && !lod {
            self.drop_lights(coord);
        }
        if every && let Some(l) = self.lights.get_mut(&coord) {
            l.bricks.clear();
        }
        if self.surface_attrs {
            if every || lod {
                self.shell_chunks.insert(coord);
            }
            if every && !lod {
                self.attr_chunks.insert(coord);
            }
        }
        let mut result = Ok(());
        let set = upload_set(bricks.len(), every, attrs, |i| bricks.is_dirty(i));
        for (i, &up) in set.iter().enumerate() {
            if !up {
                continue;
            }
            if self.surface_attrs && !every && !lod && bricks.is_dirty(i) {
                let b = brick_of_slot(i);
                self.attr_bricks
                    .insert([0, 1, 2].map(|k| coord.0[k] * CHUNK_EDGE_BRICKS + b[k]));
            }
            // The upload's backstop computed the surfaces under the same write lock.
            let surface = self.surfaces.get(i).copied().flatten();
            result = self.upload_brick(base + ENTRY_WORD + i, bricks.get(i), surface, enc, st);
            if result.is_err() {
                break;
            }
            if emits {
                rescan.insert(i);
                let b = brick_of_slot(i);
                for k in 0..3 {
                    for dir in [-1, 1] {
                        let mut n = b;
                        n[k] += dir;
                        if (0..CHUNK_EDGE_BRICKS).contains(&n[k]) {
                            rescan.insert(brick_slot(n));
                        }
                    }
                }
            }
        }
        if !lod {
            // A written brick changes its own exposed voxels and those of its neighbours'
            // faces toward it.
            let t = stopwatch();
            for &i in &rescan {
                self.note_lights(coord, i, &bricks, st);
            }
            light_us += micros_since(t);
            if self.lights.contains_key(&coord) {
                self.light_dirty.insert(coord);
            }
        }
        st.light_us += light_us;
        let empty = self.write_bits(base);
        self.mirrored.insert(
            coord,
            Mirrored {
                record: rec,
                lod,
                emits,
            },
        );
        self.stage_record(rec, enc, st);
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
        let (bits, rest) = self.entries[base..base + RECORD_WORDS].split_at_mut(ENTRY_WORD);
        for (w, entries) in bits.iter_mut().zip(rest.chunks_exact(32)) {
            *w = entries
                .iter()
                .enumerate()
                .fold(0, |acc, (j, &e)| acc | u32::from(e != 0) << j);
        }
        bits.iter().all(|&w| w == 0)
    }

    /// Writes `brick` into record word `i`, a brick entry: a `Full` brick as its slot, staged
    /// from its packed words and, for a brick carrying attributes, its `surface` mask
    /// ([`GpuWorld::fill_slot`]); a uniform one as its material. A brick whose slot changes
    /// bin takes a slot of the new bin, then returns the old one.
    fn upload_brick(
        &mut self,
        i: usize,
        brick: &Brick,
        surface: Option<[u64; 8]>,
        enc: &mut wgpu::CommandEncoder,
        st: &mut SyncStats,
    ) -> Result<(), MirrorError> {
        let old = self.entries[i];
        let old_unit = (old & FULL_BIT != 0).then_some(old & UNIT_MASK);
        let Brick::Full(p) = brick else {
            if let Some(u) = old_unit {
                self.free_slot(u);
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
        let mut words = std::mem::take(&mut self.slot_words);
        let flag = self.fill_slot(p, surface, &mut words);
        let k = bin_of((words.len() / UNIT_WORDS) as u32);
        // The rest of the bin's slot reads as "no plane" and is never read as anything else.
        words.resize(BINS[k] as usize * UNIT_WORDS, u32::MAX);
        let u = match old_unit {
            Some(u) if self.bin_of_unit(u) == Some(k) => u,
            _ => match self.alloc(k) {
                Ok(u) => {
                    if let Some(o) = old_unit {
                        self.free_slot(o);
                    }
                    u
                }
                Err(e) => {
                    self.slot_words = words;
                    return Err(e);
                }
            },
        };
        self.stage(k, u, &words, enc, st);
        st.bytes += (words.len() * 4) as u64;
        self.entries[i] = FULL_BIT | flag | class_code(p.class()) << CLASS_SHIFT | u;
        self.slot_words = words;
        st.bricks += 1;
        Ok(())
    }

    /// Fills `out` with the CPU units of `p`'s slot (module docs): the opaque mask (the packed
    /// mask less the translucent voxels), the header, the planes or raw ids copied from the
    /// packed words, the translucent unit of a planar brick holding a translucent voxel, and
    /// for a brick carrying attributes with a non-empty `surface`, its surface mask and its
    /// attribute units as "no plane", the header holding their prefix popcounts and count.
    /// Returns `TRANSLUCENT_BIT` when the brick holds a translucent voxel, else 0.
    fn fill_slot(&self, p: &Packed, surface: Option<[u64; 8]>, out: &mut Vec<u32>) -> u32 {
        out.clear();
        out.resize(2 * UNIT_WORDS, 0);
        let translucent = |m: MaterialId| self.translucent[usize::from(m)];
        let mut glass = [0u64; 8];
        let code = class_code(p.class());
        if materials_of(p, translucent) {
            match (p.entry_matches(), p.wide_ids()) {
                (Some(entries), _) => {
                    for (m, w) in entries {
                        if translucent(m) {
                            for z in 0..8 {
                                glass[z] |= w[z];
                            }
                        }
                    }
                }
                (None, Some(ids)) => {
                    for (i, &m) in ids.iter().enumerate() {
                        if m != AIR && translucent(m) {
                            glass[i / 64] |= 1 << (i % 64);
                        }
                    }
                }
                (None, None) => unreachable!("a packed brick is planar or wide"),
            }
        }
        let mask = p.mask();
        for z in 0..8 {
            let opaque = mask[z] & !glass[z];
            out[2 * z] = opaque as u32;
            out[2 * z + 1] = (opaque >> 32) as u32;
        }
        let header = UNIT_WORDS;
        for (z, &w) in glass.iter().enumerate() {
            let mut bits = w;
            while bits != 0 {
                let b = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let (x, y) = (b % 8, b / 8);
                let block = x / 2 + 4 * (y / 2) + 16 * (z / 2);
                out[header + HEADER_TRANSLUCENT_WORD + block / 32] |= 1 << (block % 32);
            }
        }
        let surface = surface.filter(|s| popcount(s) > 0);
        if let Some(s) = &surface {
            let mut below = 0u32;
            for (z, w) in s.iter().enumerate() {
                out[header + HEADER_PREFIX_WORD + z / 2] |= below << (16 * (z % 2));
                below += w.count_ones();
            }
            out[header + HEADER_ATTR_WORD] =
                (surface_attr_units(below) - 1) | (HEADER_HAS_SURFACE << 16);
        }
        match p.words() {
            Some(w) => {
                let planes = p.class().planes();
                for (q, &pal) in w[8 + 8 * planes..].iter().enumerate() {
                    out[header + HEADER_PALETTE_WORD + 2 * q] = pal as u32;
                    out[header + HEADER_PALETTE_WORD + 2 * q + 1] = (pal >> 32) as u32;
                }
                for &plane in &w[8..8 + 8 * planes] {
                    out.extend([plane as u32, (plane >> 32) as u32]);
                }
                if glass != [0; 8] {
                    for g in glass {
                        out.extend([g as u32, (g >> 32) as u32]);
                    }
                }
            }
            None => {
                let ids = p.wide_ids().expect("a packed brick is planar or wide");
                out.extend(
                    ids.chunks_exact(2)
                        .map(|v| u32::from(v[0]) | u32::from(v[1]) << 16),
                );
            }
        }
        debug_assert_eq!(
            out.len(),
            2 * UNIT_WORDS
                + payload_units(code) as usize * UNIT_WORDS
                + usize::from(glass != [0; 8] && code != CLASS_WIDE) * UNIT_WORDS
        );
        if let Some(s) = surface {
            for w in s {
                out.extend([w as u32, (w >> 32) as u32]);
            }
            let attrs = surface_attr_units(popcount(&s)) - 1;
            out.resize(out.len() + attrs as usize * UNIT_WORDS, u32::MAX);
        }
        if glass != [0; 8] { TRANSLUCENT_BIT } else { 0 }
    }

    /// Replaces brick `slot` of `coord`'s light lists with its exposed emissive voxels.
    fn note_lights(
        &mut self,
        coord: ChunkCoord,
        slot: usize,
        bricks: &Bricks<'_>,
        st: &mut SyncStats,
    ) {
        let brick = bricks.get(slot);
        let found = if brick.is_full() {
            st.light_bricks += 1;
            let opaque: &[bool] = &self.opaque;
            let b = brick_of_slot(slot);
            let neighbour = |k: usize, dir: i32| {
                let mut n = b;
                n[k] += dir;
                (0..CHUNK_EDGE_BRICKS)
                    .contains(&n[k])
                    .then(|| bricks.get(brick_slot(n)))
            };
            exposed_emitters(coord, slot, brick, &self.emissive, opaque, neighbour)
        } else {
            Vec::new()
        };
        match self.lights.get_mut(&coord) {
            Some(l) => {
                if found.is_empty() {
                    l.bricks.remove(&(slot as u16));
                } else {
                    l.bricks.insert(slot as u16, found);
                }
            }
            None if !found.is_empty() => {
                let mut l = ChunkLights::default();
                l.bricks.insert(slot as u16, found);
                self.lights.insert(coord, l);
            }
            None => {}
        }
    }

    /// The ranges of the light-sample buffer by the light radius (ADR-0025): a range more
    /// than `light_radius + 1` chunks from the focus chunk is released; a chunk within
    /// `light_radius` whose lists changed or that holds no range is written, nearest first,
    /// until the sync's sample budget is spent (each chunk at most once; the rest wait for the
    /// next sync). A chunk the buffer cannot fit evicts the farthest range beyond itself.
    fn place_lights(&mut self, enc: &mut wgpu::CommandEncoder, st: &mut SyncStats) {
        self.light_syncs += 1;
        let r = i64::from(self.cfg.light_radius);
        let f = self.focus;
        let d2 = |c: ChunkCoord| -> i64 { (0..3).map(|k| i64::from(c.0[k] - f.0[k]).pow(2)).sum() };
        let far: Vec<ChunkCoord> = self
            .lights
            .iter()
            .filter(|(c, l)| l.range.is_some() && d2(**c) > (r + 1) * (r + 1))
            .map(|(c, _)| *c)
            .collect();
        for c in far {
            self.release_lights(c, enc);
        }
        let mut wanted: Vec<(i64, ChunkCoord)> = self
            .lights
            .iter()
            .filter(|(c, l)| {
                d2(**c) <= r * r && (l.range.is_none() || self.light_dirty.contains(*c))
            })
            .map(|(c, _)| (d2(*c), *c))
            .collect();
        wanted.sort_unstable_by_key(|&(d, c)| (d, c.0));
        let mut written = 0u64;
        for (d, c) in wanted {
            if written >= u64::from(self.cfg.light_sync_samples) {
                self.light_wait.entry(c).or_insert(self.light_syncs);
                continue;
            }
            written += self.write_chunk_lights(c, d, enc, st);
            self.light_dirty.remove(&c);
            self.light_wait.remove(&c);
        }
        // A dirty chunk outside the radius is rewritten when it comes back inside it.
        self.light_wait.retain(|c, _| d2(*c) <= r * r);
        st.light_oldest_wait = self
            .light_wait
            .values()
            .map(|&since| self.light_syncs - since)
            .max()
            .unwrap_or(0);
    }

    /// Writes `coord`'s samples (thinned to the cap) into a new range, its old range zeroed and
    /// freed; `d2` is its squared chunk distance to the focus. When the buffer has no run, the
    /// farthest range beyond `d2` is evicted until one fits; with none left the chunk is
    /// refused and holds no range. The samples written.
    fn write_chunk_lights(
        &mut self,
        coord: ChunkCoord,
        d2: i64,
        enc: &mut wgpu::CommandEncoder,
        st: &mut SyncStats,
    ) -> u64 {
        self.release_lights(coord, enc);
        let Some(l) = self.lights.get(&coord) else {
            return 0;
        };
        if l.bricks.is_empty() {
            self.lights.remove(&coord);
            return 0;
        }
        let all: Vec<LightSample> = l.bricks.values().flatten().copied().collect();
        let power: f32 = all
            .iter()
            .map(|s| self.emit_lum[usize::from(s.material())] * s.faces().count_ones() as f32)
            .sum();
        let samples = thin_samples(all, self.cfg.light_chunk_cap);
        let n = samples.len() as u32;
        let start = loop {
            if let Some(start) = self.alloc_light_range(n) {
                break Some(start);
            }
            let f = self.focus;
            let victim = self
                .lights
                .iter()
                .filter(|(c, l)| **c != coord && l.range.is_some())
                .map(|(c, _)| {
                    let d: i64 = (0..3).map(|k| i64::from(c.0[k] - f.0[k]).pow(2)).sum();
                    (d, c.0, *c)
                })
                .filter(|&(d, _, _)| d > d2)
                .max();
            match victim {
                Some((_, _, v)) => {
                    self.release_lights(v, enc);
                    self.light_wait.entry(v).or_insert(self.light_syncs);
                    st.light_evicted += 1;
                }
                None => break None,
            }
        };
        let Some(start) = start else {
            st.light_refused += 1;
            return 0;
        };
        self.belt.write(
            enc,
            &self.light_buf,
            u64::from(start) * LIGHT_SAMPLE_BYTES,
            bytemuck::cast_slice(&samples),
        );
        st.writes += 1;
        st.bytes += u64::from(n) * LIGHT_SAMPLE_BYTES;
        st.light_samples += u64::from(n);
        self.light_live += u64::from(n);
        if let Some(l) = self.lights.get_mut(&coord) {
            l.range = Some((start, n));
            l.power = power;
        }
        u64::from(n)
    }

    /// Zeroes and frees `coord`'s range, keeping its lists.
    fn release_lights(&mut self, coord: ChunkCoord, enc: &mut wgpu::CommandEncoder) {
        if let Some(r) = self.lights.get_mut(&coord).and_then(|l| l.range.take()) {
            self.light_live -= u64::from(r.1);
            self.zero_light_range(r, enc);
            self.free_light_range(r);
        }
    }

    /// The chunks holding a range of the light-sample buffer: coordinate, start, length and
    /// power (the sum over the chunk's samples, before thinning, of its material's emitted
    /// luminance times its exposed faces; the render-side scale left out).
    pub fn light_chunks(&self) -> impl Iterator<Item = (ChunkCoord, u32, u32, f32)> + '_ {
        self.lights
            .iter()
            .filter_map(|(c, l)| l.range.map(|(s, n)| (*c, s, n, l.power)))
    }

    /// The first slot of the candidate table in the light-sample buffer: just past the
    /// reservation.
    pub fn light_table_base(&self) -> u32 {
        self.cfg.light_samples
    }

    /// The light radius in chunks ([`MirrorConfig::light_radius`]).
    pub fn light_radius(&self) -> i32 {
        self.cfg.light_radius
    }

    /// Forgets `coord`'s light samples; their range is zeroed and freed on the next sync.
    fn drop_lights(&mut self, coord: ChunkCoord) {
        self.light_dirty.remove(&coord);
        self.light_wait.remove(&coord);
        if let Some(l) = self.lights.remove(&coord)
            && let Some(r) = l.range
        {
            self.light_live -= u64::from(r.1);
            self.light_dropped.push(r);
        }
    }

    /// Zeroes and frees the ranges `drop_lights` set aside.
    fn flush_dropped_lights(&mut self, enc: &mut wgpu::CommandEncoder) {
        for r in std::mem::take(&mut self.light_dropped) {
            self.zero_light_range(r, enc);
            self.free_light_range(r);
        }
    }

    /// Zeroes a range of the light-sample buffer in the sync's encoder, ordered with its belt
    /// copies (a queue write would run before every command buffer of the submission and
    /// could zero a range a later copy refilled).
    fn zero_light_range(&self, (start, len): (u32, u32), enc: &mut wgpu::CommandEncoder) {
        enc.clear_buffer(
            &self.light_buf,
            u64::from(start) * LIGHT_SAMPLE_BYTES,
            Some(u64::from(len) * LIGHT_SAMPLE_BYTES),
        );
    }

    /// The start of a free run of `n` samples: the lowest free range that holds it, else the
    /// samples above the high-water mark; `None` when the buffer has no such run.
    fn alloc_light_range(&mut self, n: u32) -> Option<u32> {
        if let Some((&start, &len)) = self.light_free.iter().find(|&(_, &len)| len >= n) {
            self.light_free.remove(&start);
            if len > n {
                self.light_free.insert(start + n, len - n);
            }
            return Some(start);
        }
        let start = self.light_high;
        if u64::from(start) + u64::from(n) > u64::from(self.cfg.light_samples) {
            return None;
        }
        self.light_high += n;
        Some(start)
    }

    /// Returns a range to the free ranges, merged with its neighbours; a free range that
    /// reaches the high-water mark lowers it instead.
    fn free_light_range(&mut self, (mut start, mut len): (u32, u32)) {
        if let Some((&s, &l)) = self.light_free.range(..start).next_back()
            && s + l == start
        {
            self.light_free.remove(&s);
            start = s;
            len += l;
        }
        if let Some(&l) = self.light_free.get(&(start + len)) {
            self.light_free.remove(&(start + len));
            len += l;
        }
        if start + len == self.light_high {
            self.light_high = start;
        } else {
            self.light_free.insert(start, len);
        }
    }

    /// The light-sample buffer: [`LightSample`]s, holes of material 0 between the chunks'
    /// ranges.
    pub fn light_buffer(&self) -> &wgpu::Buffer {
        &self.light_buf
    }

    /// Light samples live in the buffer: the sum of the chunks' ranges.
    pub fn light_samples(&self) -> u64 {
        self.light_live
    }

    /// The samples from 0 to the high-water mark: what a uniform draw over the buffer covers.
    pub fn light_span(&self) -> u32 {
        self.light_high
    }

    /// `coord`'s range of the light-sample buffer, start and length.
    pub fn light_range(&self, coord: ChunkCoord) -> Option<(u32, u32)> {
        self.lights.get(&coord).and_then(|l| l.range)
    }

    /// Returns the slot at unit `u` to its bin's free list, or releases its page when it held
    /// the page's last live slot.
    fn free_slot(&mut self, u: u32) {
        let p = u / PAGE_UNITS;
        let k = self.page_bin[p as usize].expect("a live slot's page has a bin") as usize;
        if self.unlive(u) {
            self.release_page(p);
            return;
        }
        self.bins[k].free.push(u);
    }

    /// Takes the slot at unit `u` off its page's and its bin's live counts; true when the page
    /// holds no live slot after it.
    fn unlive(&mut self, u: u32) -> bool {
        let p = (u / PAGE_UNITS) as usize;
        let k = self.page_bin[p].expect("a live slot's page has a bin") as usize;
        self.page_live[p] -= 1;
        self.bins[k].live -= 1;
        self.page_live[p] == 0
    }

    /// Frees page `p`, which holds no live slot: its slots leave its bin's free list, the
    /// bin's cursor leaves it, and it joins the free pages.
    fn release_page(&mut self, p: u32) {
        let k = self.page_bin[p as usize]
            .take()
            .expect("a page released once") as usize;
        let a = &mut self.bins[k];
        a.free.retain(|&u| u / PAGE_UNITS != p);
        if a.next < a.end && (a.end - 1) / PAGE_UNITS == p {
            a.next = a.end;
        }
        self.free_pages.insert(p);
    }

    /// Appends the slot at unit `u` of bin `k`, whose words are `words` (whole units), to the
    /// bin's staged run, first writing the run out if `u` does not extend it, it is full, or
    /// `u` starts a shard.
    fn stage(
        &mut self,
        k: usize,
        u: u32,
        words: &[u32],
        enc: &mut wgpu::CommandEncoder,
        st: &mut SyncStats,
    ) {
        let (start, run) = &self.stage[k];
        let len = (run.len() / UNIT_WORDS) as u32;
        let boundary = u.is_multiple_of(self.layout.units_per_shard);
        if len > 0 && (u != start + len || len >= STAGE_UNITS || boundary) {
            self.flush_stage(k, enc, st);
        }
        let (start, run) = &mut self.stage[k];
        if run.is_empty() {
            *start = u;
        }
        run.extend_from_slice(words);
    }

    /// Writes bin `k`'s staged run into its pool shard, and empties it.
    fn flush_stage(&mut self, k: usize, enc: &mut wgpu::CommandEncoder, st: &mut SyncStats) {
        let (start, run) = &mut self.stage[k];
        if run.is_empty() {
            return;
        }
        let ups = self.layout.units_per_shard;
        let shard = (*start / ups) as usize;
        self.belt.write(
            enc,
            &self.shards[shard],
            u64::from(*start % ups) * UNIT_BYTES,
            bytemuck::cast_slice(run),
        );
        st.writes += 1;
        run.clear();
    }

    /// Appends record `rec`'s CPU entries to the staged run of records.
    fn stage_record(&mut self, rec: u32, enc: &mut wgpu::CommandEncoder, st: &mut SyncStats) {
        let len = (self.rec_stage.len() / RECORD_WORDS) as u32;
        if len > 0 && (rec != self.rec_stage_start + len || len >= STAGE_RECORDS) {
            self.flush_records(enc, st);
        }
        if self.rec_stage.is_empty() {
            self.rec_stage_start = rec;
        }
        let base = rec as usize * RECORD_WORDS;
        self.rec_stage
            .extend_from_slice(&self.entries[base..base + RECORD_WORDS]);
    }

    fn flush_records(&mut self, enc: &mut wgpu::CommandEncoder, st: &mut SyncStats) {
        if self.rec_stage.is_empty() {
            return;
        }
        self.belt.write(
            enc,
            &self.records,
            u64::from(self.rec_stage_start) * RECORD_BYTES,
            bytemuck::cast_slice(&self.rec_stage),
        );
        st.writes += 1;
        self.rec_stage.clear();
    }

    /// Writes the changed table entries in runs; entries closer than `TABLE_GAP` share a run.
    fn flush_table(&mut self, enc: &mut wgpu::CommandEncoder, st: &mut SyncStats) {
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
                self.write_table_run(start, end, enc, st);
                start = i;
            }
            end = i;
        }
        self.write_table_run(start, end, enc, st);
        idx.clear();
        self.table_changed = idx;
    }

    fn write_table_run(
        &mut self,
        start: u32,
        end: u32,
        enc: &mut wgpu::CommandEncoder,
        st: &mut SyncStats,
    ) {
        let run = &self.table_cpu[start as usize..=end as usize];
        self.belt.write(
            enc,
            &self.table,
            u64::from(start) * 4,
            bytemuck::cast_slice(run),
        );
        st.writes += 1;
        st.bytes += run.len() as u64 * 4;
    }

    fn write_materials(
        &mut self,
        table: &MaterialTable,
        enc: &mut wgpu::CommandEncoder,
        st: &mut SyncStats,
    ) {
        let rows = material_rows(table);
        // The material rows are outside the reservation: sized to the table at the first sync.
        if self.materials.size() < (rows.len() * MATERIAL_FLOATS * 4) as u64 {
            self.materials = Self::create_materials(&self.device, rows.len());
        }
        self.material_rows = rows.len() as u64;
        let bytes: &[u8] = bytemuck::cast_slice(&rows);
        self.belt.write(enc, &self.materials, 0, bytes);
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

    /// Moves bin `k`'s cursor onto a page: the lowest free page, of any former bin, else the
    /// next page never handed out, failing with `PoolTooLarge` when that one lies past the
    /// reservation. The current page's unused slots go to the free list.
    fn take_page(&mut self, k: usize) -> Result<(), MirrorError> {
        let page = match self.free_pages.pop_first() {
            Some(page) => page,
            None => {
                let limit = self.layout.reserved_pages();
                if self.next_page + 1 > limit {
                    return Err(MirrorError::PoolTooLarge {
                        needed: u64::from(self.next_page + 1) * PAGE_BYTES,
                        limit: u64::from(limit) * PAGE_BYTES,
                    });
                }
                self.page_bin.push(None);
                self.page_live.push(0);
                self.next_page += 1;
                self.next_page - 1
            }
        };
        self.page_bin[page as usize] = Some(k as u8);
        let b = BINS[k];
        let a = &mut self.bins[k];
        // Pushed highest first, so they pop ascending.
        a.free.extend((a.next..a.end).step_by(b as usize).rev());
        a.next = page * PAGE_UNITS;
        a.end = a.next + slots_per_page(k) * b;
        Ok(())
    }

    /// One slot of bin `k`: a free one, else the cursor, else a new page.
    fn alloc(&mut self, k: usize) -> Result<u32, MirrorError> {
        let a = &mut self.bins[k];
        let u = if let Some(u) = a.free.pop() {
            u
        } else {
            if a.next >= a.end {
                self.take_page(k)?;
            }
            let a = &mut self.bins[k];
            a.next += BINS[k];
            a.next - BINS[k]
        };
        self.page_live[(u / PAGE_UNITS) as usize] += 1;
        self.bins[k].live += 1;
        Ok(u)
    }
}

/// The GPU rows of `table`, [`MATERIAL_FLOATS`] each.
fn material_rows(table: &MaterialTable) -> Vec<[f32; MATERIAL_FLOATS]> {
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
                m.detail[0],
                m.detail[1],
                m.opacity,
                m.detail_fine,
                0.0,
                0.0,
                0.0,
                0.0,
            ]
        })
        .collect()
}

/// An entry of the attribute carry-over queue.
#[derive(Clone, Copy, Debug)]
enum AttrWait {
    /// Every brick of a chunk written whole.
    Chunk(ChunkCoord),
    /// One brick, in world brick coordinates.
    Brick([i32; 3]),
}

impl MirrorView for GpuWorld {
    fn is_full_chunk(&self, c: ChunkCoord) -> bool {
        self.mirrored.get(&c).is_some_and(|m| !m.lod)
    }

    /// The brick's entry less `FULL_BIT`: its first unit, class and translucent flag, which
    /// locate its attributes ([`attr_offset`]).
    fn full_slot(&self, b: [i32; 3]) -> Option<u32> {
        let e = CHUNK_EDGE_BRICKS;
        let c = ChunkCoord(b.map(|v| v.div_euclid(e)));
        let m = self.mirrored.get(&c).filter(|m| !m.lod)?;
        let local = b.map(|v| v.rem_euclid(e));
        let entry = self.entries[m.record as usize * RECORD_WORDS + ENTRY_WORD + brick_slot(local)];
        (entry & FULL_BIT != 0).then_some(entry & !FULL_BIT)
    }
}

fn d2(a: ChunkCoord, b: ChunkCoord) -> i64 {
    (0..3)
        .map(|k| (i64::from(a.0[k]) - i64::from(b.0[k])).pow(2))
        .sum()
}

/// The chunks held (`held(c)`) that leave the window when it moves from `old` to `new`. Every
/// held chunk lies in `old`. When the slab `old - new` has no more chunks than the `count`
/// held, it is looked up chunk by chunk; otherwise the held chunks (`keys`, possibly with
/// repeats) are filtered. Either way the result holds each leaving chunk once.
fn leaving_window<K: Iterator<Item = ChunkCoord>>(
    old: ChunkExtent,
    new: ChunkExtent,
    count: usize,
    held: impl Fn(ChunkCoord) -> bool,
    keys: impl FnOnce() -> K,
) -> Vec<ChunkCoord> {
    let overlap: i64 = (0..3)
        .map(|k| {
            let lo = old.min.0[k].max(new.min.0[k]);
            let hi = old.max.0[k].min(new.max.0[k]);
            i64::from((hi - lo).max(0))
        })
        .product();
    let volume: i64 = old.size().iter().map(|&v| i64::from(v)).product();
    if (volume - overlap) as u64 <= count as u64 {
        box_difference(old, new)
            .into_iter()
            .filter(|&c| held(c))
            .collect()
    } else {
        let set: HashSet<ChunkCoord> = keys().filter(|&c| !new.contains(c)).collect();
        set.into_iter().collect()
    }
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

/// Whether a material of `p` satisfies `f`: its palette entries, or a `Wide` brick's ids.
fn materials_of(p: &Packed, f: impl Fn(MaterialId) -> bool) -> bool {
    match p.words() {
        Some(w) => w[8 + 8 * p.class().planes()..].iter().any(|&q| {
            (0..4).any(|j| {
                let m = (q >> (16 * j)) as MaterialId;
                m != AIR && f(m)
            })
        }),
        None => p
            .wide_ids()
            .is_some_and(|ids| ids.iter().any(|&m| m != AIR && f(m))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 50 seeded shifts of -2..=2 chunks per axis of a 7-chunk window holding a seeded mix
    /// of mirrored and pending chunks: the slab lookup leaves the chunks the filter over every
    /// held key leaves, and clears the same wrapped table entries.
    #[test]
    fn the_window_slab_finds_the_chunks_the_key_filter_finds() {
        let side = 7;
        let mut z = 0x0023_0004_u64;
        let mut next = |n: u64| {
            z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
            vxe_world::hash::mix64(z) % n
        };
        let wrap = |c: ChunkCoord| {
            let r = c.0.map(|v| v.rem_euclid(side) as usize);
            r[0] + 7 * (r[1] + 7 * r[2])
        };
        let mut min = [0i32; 3];
        let mut slab_used = 0;
        for shift in 0..50 {
            let old = ChunkExtent::new(min, min.map(|v| v + side));
            let mut mirrored = HashSet::new();
            let mut pending = HashSet::new();
            for x in old.min.0[0]..old.max.0[0] {
                for y in old.min.0[1]..old.max.0[1] {
                    for zz in old.min.0[2]..old.max.0[2] {
                        let c = ChunkCoord([x, y, zz]);
                        match next(4) {
                            0 => {}
                            1 => {
                                mirrored.insert(c);
                            }
                            2 => {
                                pending.insert(c);
                            }
                            _ => {
                                mirrored.insert(c);
                                pending.insert(c);
                            }
                        }
                    }
                }
            }
            let d = [0; 3].map(|_| next(5) as i32 - 2);
            min = [0, 1, 2].map(|k| min[k] + d[k]);
            let new = ChunkExtent::new(min, min.map(|v| v + side));
            let count = mirrored.len() + pending.len();
            let got = leaving_window(
                old,
                new,
                count,
                |c| mirrored.contains(&c) || pending.contains(&c),
                || mirrored.iter().chain(pending.iter()).copied(),
            );
            let slab = box_difference(old, new).len();
            if slab <= count {
                slab_used += 1;
            }
            let got_set: HashSet<ChunkCoord> = got.iter().copied().collect();
            assert_eq!(got.len(), got_set.len(), "shift {shift}: a repeat");
            let want: HashSet<ChunkCoord> = mirrored
                .iter()
                .chain(pending.iter())
                .copied()
                .filter(|&c| !new.contains(c))
                .collect();
            assert_eq!(got_set, want, "shift {shift} by {d:?}");
            let table = |leave: &HashSet<ChunkCoord>| {
                let mut t = vec![None; 343];
                for &c in mirrored.iter().chain(pending.iter()) {
                    t[wrap(c)] = Some(c);
                }
                for &c in leave {
                    t[wrap(c)] = None;
                }
                t
            };
            assert_eq!(table(&got_set), table(&want), "shift {shift}");
        }
        assert!(slab_used >= 25, "the slab path ran {slab_used} of 50 times");
    }

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
