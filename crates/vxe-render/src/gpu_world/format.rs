//! The mirror's GPU contract: the pool's units, pages and bins, the record words and entry
//! bits, a slot's units by class, and the shard layout the budget reserves.

use super::*;

/// Bytes of one unit of the pool: one 64-byte line, the opaque mask of a brick.
pub const UNIT_BYTES: u64 = 64;
/// `u32` words of one unit.
pub const UNIT_WORDS: usize = 16;
/// Units per page: the unit the pool is reserved and handed out in, and the unit
/// `units_per_shard` is a multiple of. 256 KiB.
pub const PAGE_UNITS: u32 = 4096;
/// The slot sizes, in units, that pages are binned by; a brick's slot takes the smallest that
/// holds it.
pub const BINS: [u32; 10] = [2, 3, 4, 6, 8, 12, 16, 20, 24, 36];
/// Every bin's slot size: [`BINS`], then the sizes a band slot takes that `BINS` lacks. A band
/// slot takes the bin of its exact size ([`band_bin_of`]).
pub const SLOT_BINS: [u32; 13] = [2, 3, 4, 6, 8, 12, 16, 20, 24, 36, 5, 7, 9];
/// How many bins there are.
pub const BIN_COUNT: usize = SLOT_BINS.len();
/// Storage buffers the pool is sharded over: every frame shader binds all four.
pub(crate) const MAX_SHARDS: u32 = 4;
/// Storage buffers the frame's passes bind besides the pool shards: records, table,
/// materials, light samples and the emission scale, plus the face store the hit pass reads
/// for a translucent face's reflected light (ADR-0047).
pub(crate) const OTHER_STORAGE_BUFFERS: u32 = 6;
/// `u32` words of a slot's opaque mask (its unit 0): one bit per voxel.
pub(crate) const OPAQUE_WORDS: usize = 16;
/// The header unit's words: the palette (16 `u16`), the two translucent words (one bit per
/// 2^3 block), two words held at 0 (10 and 11), the band prefix (two words,
/// [`pack_prefix`]: the band voxels below each layer), word 14 held at 0, then the band slot.
pub(crate) const HEADER_PALETTE_WORD: usize = 0;
pub const HEADER_TRANSLUCENT_WORD: usize = 8;
/// The band prefix of a band brick (ADR-0041), [`pack_prefix`] of its band mask.
pub const HEADER_BAND_PREFIX_WORD: usize = 12;
/// The header word naming a band brick's band slot (ADR-0041): [`HEADER_BAND_BIT`] and the
/// band slot's first unit; 0 for a brick without one.
pub const HEADER_BAND_WORD: usize = 15;
pub const HEADER_BAND_BIT: u32 = 1 << 31;
/// The bit offset and width of layer `z`'s prefix (`z` in 1..8) inside the two words of a
/// packed prefix: layers 1 to 4 in the first word at bits 0, 7, 15 and 23 (7, 8, 8 and 9 bits
/// wide), layers 5 to 7 in the second at bits 0, 9 and 18 (9 bits each). Layer `z`'s prefix is
/// at most `64 z`, so each fits its width; layer 0's is 0 and is not stored.
pub(super) const PREFIX_FIELDS: [(usize, u32, u32); 8] = [
    (0, 0, 0),
    (0, 0, 7),
    (0, 7, 8),
    (0, 15, 8),
    (0, 23, 9),
    (1, 0, 9),
    (1, 9, 9),
    (1, 18, 9),
];

/// The packed prefix of `mask` (bit `x + 8 y` of word `z` for voxel `(x, y, z)`): for each
/// layer `z`, the set bits of the layers below it, in two words ([`PREFIX_FIELDS`]). The
/// shaders read it back with `layer_prefix`.
pub(crate) fn pack_prefix(mask: &[u64; 8]) -> [u32; 2] {
    let mut out = [0u32; 2];
    let mut below = 0u32;
    for (z, w) in mask.iter().enumerate() {
        let (word, shift, _) = PREFIX_FIELDS[z];
        if z > 0 {
            out[word] |= below << shift;
        }
        below += w.count_ones();
    }
    out
}

/// Layer `z`'s prefix in the packed prefix `words` ([`pack_prefix`]).
pub fn layer_prefix(words: [u32; 2], z: usize) -> u32 {
    let (word, shift, width) = PREFIX_FIELDS[z];
    (words[word] >> shift) & ((1u32 << width) - 1)
}

/// Writes into `header` (a slot's header unit) the band slot `unit` of a band brick of band
/// mask `mask`: [`HEADER_BAND_WORD`], and its band prefix in [`HEADER_BAND_PREFIX_WORD`].
pub(crate) fn band_header(header: &mut [u32], unit: u32, mask: &[u64; 8]) {
    header[HEADER_BAND_WORD] = HEADER_BAND_BIT | unit;
    let p = pack_prefix(mask);
    header[HEADER_BAND_PREFIX_WORD] = p[0];
    header[HEADER_BAND_PREFIX_WORD + 1] = p[1];
}

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
/// The class code of a uniform band brick's slot (ADR-0041): laid out as class 1 (a mask and a
/// header with the brick's material as palette entry 0, the translucent unit for a
/// translucent material), so that every reader of a `Full` entry reads its voxels; its header
/// names its band slot.
pub const CLASS_UNIFORM: u32 = 6;
/// Chunk table entry bit: the record is a LOD record.
pub const LOD_BIT: u32 = 1 << 30;
/// Chunk table entry bit: every entry of the record is uniform air, so its brick bits are
/// all clear. Kept equal to the record's bits on every write of the record.
pub const EMPTY_BIT: u32 = 1 << 29;
/// The record index in a chunk table entry that is not `ABSENT`.
pub const RECORD_MASK: u32 = EMPTY_BIT - 1;
/// First entry of each mip level in a LOD record, level 1 first.
pub const LOD_LEVEL_ENTRY: [usize; MIP_LEVELS] = [0, 64, 72];
/// `f32`s per material row: albedo rgb, roughness, emission rgb, flags as bits, 2 spare,
/// opacity, 5 spare.
pub const MATERIAL_FLOATS: usize = 16;
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
        // CLASS_UNIFORM and class 1 have no payload.
        2 => 1,
        3 => 2,
        4 => 4,
        CLASS_WIDE => 16,
        _ => 0,
    }
}

/// The units of the slot of `p` (module docs): mask, header, payload, and a translucent unit
/// for a planar brick holding a voxel of a material `translucent` names.
pub fn slot_units(p: &Packed, translucent: impl Fn(MaterialId) -> bool) -> u32 {
    let code = class_code(p.class());
    let t = code != CLASS_WIDE && materials_of(p, translucent);
    2 + payload_units(code) + u32::from(t)
}

/// The bin index of a slot of `units` units.
pub fn bin_of(units: u32) -> usize {
    BINS.iter()
        .position(|&b| b >= units)
        .expect("a slot fits the largest bin")
}

/// The bin index of a band slot of `units` units, the bin of exactly that size.
pub fn band_bin_of(units: u32) -> usize {
    SLOT_BINS
        .iter()
        .position(|&b| b == units)
        .expect("every band slot size has a bin")
}

/// Slots of bin `k` one page holds.
pub fn slots_per_page(k: usize) -> u32 {
    PAGE_UNITS / SLOT_BINS[k]
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
        let record_capacity = (bytes / RECORD_BYTES).min(u64::from(u32::MAX)) as u32;
        let capacity = u64::from(units_per_shard) * u64::from(MAX_SHARDS);
        Self {
            units_per_shard,
            record_capacity,
            reserved_units: capacity.min(u64::from(UNIT_MASK) + 1) as u32,
            reserved_records: record_capacity,
        }
    }

    /// Units the pool can hold over its [`MAX_SHARDS`] shards.
    pub fn capacity(&self) -> u64 {
        u64::from(self.units_per_shard) * u64::from(MAX_SHARDS)
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
pub(super) fn shard_units(units: u64, ups: u64, k: u64) -> u64 {
    units.saturating_sub(k * ups).min(ups)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The packed prefix reads back every layer's set bits below it, for a full mask (the
    /// widest values: 64 z) and for a seeded mix.
    #[test]
    fn a_packed_prefix_reads_back_each_layer() {
        let mut z = 0x0054_003a_u64;
        let mut masks = vec![[!0u64; 8], [0; 8]];
        for _ in 0..64 {
            masks.push(std::array::from_fn(|_| {
                z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
                vxe_world::hash::mix64(z) & vxe_world::hash::mix64(z ^ 1)
            }));
        }
        for m in masks {
            let p = pack_prefix(&m);
            let mut below = 0;
            for (layer, w) in m.iter().enumerate() {
                assert_eq!(layer_prefix(p, layer), below, "{m:x?} layer {layer}");
                below += w.count_ones();
            }
        }
    }
}
