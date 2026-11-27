//! A full chunk's *slot image*: what the mirror writes of the chunk into a new record, made
//! without the mirror (ADR-0056). Per brick its entry word less the slot's unit, per `Full`
//! brick its slot words padded to its bin, the per-bin counts and bytes a new record takes, and
//! the chunk's exposed emitters. The per-brick packing here ([`MaterialFlags::pack_brick`] and
//! the functions under it) is the one the mirror's own upload calls, so an image's words are
//! the upload's words by construction.

use super::*;

/// A brick's bin in [`SlotImage::bins`] when it takes no slot (a uniform brick).
pub(super) const NO_BIN: u8 = u8::MAX;

/// What [`MaterialFlags::pack_brick`] made of a brick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BrickWords {
    /// No slot: the brick's record entry is this word (its material and translucent bit).
    Uniform(u32),
    /// A slot of bin `bin`, its words in the caller's buffer padded to the bin; the entry is
    /// `FULL_BIT | flag | code << CLASS_SHIFT` and the slot's first unit.
    Slot { flag: u32, code: u32, bin: usize },
}

impl BrickWords {
    /// The record entry without the slot's unit.
    pub(super) fn entry(self) -> u32 {
        match self {
            Self::Uniform(e) => e,
            Self::Slot { flag, code, .. } => FULL_BIT | flag | code << CLASS_SHIFT,
        }
    }
}

impl MaterialFlags {
    /// The units of `p`'s slot ([`slot_units`]): mask, header, payload, and a translucent unit
    /// for a planar brick holding a translucent voxel.
    pub(super) fn slot_units(&self, p: &Packed) -> u32 {
        slot_units(p, |m| self.translucent[usize::from(m)])
    }

    /// Whether `p`'s palette holds a material with non-zero emission: the bricks a record's
    /// emitter scan visits.
    pub(super) fn emits(&self, p: &Packed) -> bool {
        materials_of(p, |m| self.emissive[usize::from(m)])
    }

    /// The record entry of a uniform brick of `m` that takes no slot.
    pub(super) fn uniform_entry(&self, m: MaterialId) -> u32 {
        u32::from(m)
            | if self.translucent[usize::from(m)] {
                TRANSLUCENT_BIT
            } else {
                0
            }
    }

    /// The exposed emitters of brick `slot` of a full chunk `coord` ([`scan_emitters`]),
    /// `bricks(i)` the chunk's brick at slot `i`: `None` for a brick that is not `Full` or
    /// whose palette holds no emissive material.
    pub(super) fn scan<'a>(
        &self,
        coord: ChunkCoord,
        slot: usize,
        bricks: impl Fn(usize) -> &'a Brick,
    ) -> Option<Vec<LightSample>> {
        let b = brick_of_slot(slot);
        let neighbour = |k: usize, dir: i32| {
            let mut n = b;
            n[k] += dir;
            (0..CHUNK_EDGE_BRICKS)
                .contains(&n[k])
                .then(|| bricks(brick_slot(n)))
        };
        scan_emitters(
            coord,
            slot,
            bricks(slot),
            &self.emissive,
            &self.opaque,
            neighbour,
        )
    }

    /// Packs `brick` into `out`: a `Full` brick as its slot ([`MaterialFlags::fill_slot`]), a
    /// uniform one as a [`CLASS_UNIFORM`] slot when `uniform_slot` (a band brick), else as its
    /// entry alone. `band` (a band slot's first unit and its mask) goes into the header. A
    /// slot's words are padded with `u32::MAX` to its bin, padding no reader reads.
    pub(super) fn pack_brick(
        &self,
        brick: &Brick,
        band: Option<(u32, &[u64; 8])>,
        uniform_slot: bool,
        out: &mut Vec<u32>,
    ) -> BrickWords {
        let (flag, code) = match brick {
            Brick::Full(p) => (self.fill_slot(p, band, out), class_code(p.class())),
            Brick::Uniform(m) if uniform_slot => {
                (self.fill_uniform_slot(*m, band, out), CLASS_UNIFORM)
            }
            Brick::Uniform(m) => return BrickWords::Uniform(self.uniform_entry(*m)),
        };
        let bin = bin_of((out.len() / UNIT_WORDS) as u32);
        out.resize(BINS[bin] as usize * UNIT_WORDS, u32::MAX);
        BrickWords::Slot { flag, code, bin }
    }

    /// Fills `out` with the CPU units of a [`CLASS_UNIFORM`] slot of material `m`: an opaque
    /// mask all set for an opaque material and clear for air or a translucent one, the header
    /// with `m` as palette entry 0 (none for air), its translucent words all set for a
    /// translucent material, and `band` in [`HEADER_BAND_WORD`]; then, for a translucent
    /// material, a translucent unit all set. Returns `TRANSLUCENT_BIT` for a translucent `m`.
    pub(super) fn fill_uniform_slot(
        &self,
        m: MaterialId,
        band: Option<(u32, &[u64; 8])>,
        out: &mut Vec<u32>,
    ) -> u32 {
        out.clear();
        out.resize(2 * UNIT_WORDS, 0);
        let translucent = self.translucent[usize::from(m)];
        if m != AIR && !translucent {
            out[..OPAQUE_WORDS].fill(!0);
        }
        let header = UNIT_WORDS;
        out[header + HEADER_PALETTE_WORD] = u32::from(m);
        if let Some((u, mask)) = band {
            band_header(&mut out[header..header + UNIT_WORDS], u, mask);
        }
        if translucent {
            out[header + HEADER_TRANSLUCENT_WORD] = !0;
            out[header + HEADER_TRANSLUCENT_WORD + 1] = !0;
            out.resize(3 * UNIT_WORDS, !0);
            return TRANSLUCENT_BIT;
        }
        0
    }

    /// Fills `out` with the CPU units of `p`'s slot (module docs of the mirror): the opaque
    /// mask (the packed mask less the translucent voxels), the header, the planes or raw ids
    /// copied from the packed words, the translucent unit of a planar brick holding a
    /// translucent voxel, and `band`, a band slot's first unit and its band mask, in the header
    /// ([`band_header`]). Returns `TRANSLUCENT_BIT` when the brick holds a translucent voxel,
    /// else 0.
    pub(super) fn fill_slot(
        &self,
        p: &Packed,
        band: Option<(u32, &[u64; 8])>,
        out: &mut Vec<u32>,
    ) -> u32 {
        out.clear();
        out.resize(2 * UNIT_WORDS, 0);
        if let Some((u, mask)) = band {
            band_header(&mut out[UNIT_WORDS..2 * UNIT_WORDS], u, mask);
        }
        let glass = self.glass_of(p);
        let code = class_code(p.class());
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
        if glass != [0; 8] { TRANSLUCENT_BIT } else { 0 }
    }
}

/// A full chunk's slot image (module docs): everything a new record of the chunk, with no band
/// slots, takes from the chunk, made by [`SlotImage::of`] with no mirror and no device.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SlotImage {
    /// [`MaterialFlags::key`] of the flags it was packed under.
    pub(super) key: u64,
    /// Per brick slot its record entry without the slot's unit ([`BrickWords::entry`]).
    pub(super) entries: Box<[u32; CHUNK_BRICKS]>,
    /// Per brick slot its slot's bin, [`NO_BIN`] for a uniform brick.
    pub(super) bins: Box<[u8; CHUNK_BRICKS]>,
    /// Each `Full` brick's slot words padded to its bin, in ascending brick slot.
    pub(super) words: Vec<u32>,
    /// What [`need_of`] gives a new record with no band: new slots per bin, and slot bytes.
    pub(super) new: [u64; BIN_COUNT],
    pub(super) bytes: u64,
    /// Whether a brick's palette holds an emissive material ([`MaterialFlags::emits`]).
    pub(super) emits: bool,
    /// The exposed emitters of each brick that has any, by ascending brick slot; empty when
    /// not `emits`.
    pub(super) lights: Vec<(u16, Vec<LightSample>)>,
}

impl SlotImage {
    /// The image of full chunk `chunk` at `coord` under `flags`.
    pub(super) fn of(coord: ChunkCoord, chunk: &svet_world::Chunk, flags: &MaterialFlags) -> Self {
        let bricks = chunk.bricks();
        let mut entries = Box::new([0u32; CHUNK_BRICKS]);
        let mut bins = Box::new([NO_BIN; CHUNK_BRICKS]);
        let mut words = Vec::new();
        let mut new = [0u64; BIN_COUNT];
        let mut bytes = 0;
        let mut emits = false;
        let mut buf = Vec::new();
        for (i, brick) in bricks.iter().enumerate() {
            let packed = flags.pack_brick(brick, None, false, &mut buf);
            entries[i] = packed.entry();
            if let BrickWords::Slot { bin, .. } = packed {
                bins[i] = bin as u8;
                new[bin] += 1;
                bytes += u64::from(BINS[bin]) * UNIT_BYTES;
                words.extend_from_slice(&buf);
            }
            emits |= matches!(brick, Brick::Full(p) if flags.emits(p));
        }
        let lights = if emits {
            (0..CHUNK_BRICKS)
                .filter_map(|i| {
                    let found = flags.scan(coord, i, |s| &bricks[s])?;
                    (!found.is_empty()).then_some((i as u16, found))
                })
                .collect()
        } else {
            Vec::new()
        };
        Self {
            key: flags.key,
            entries,
            bins,
            words,
            new,
            bytes,
            emits,
            lights,
        }
    }

    /// The bytes the image holds while it waits for the mirror: its words, its entries and
    /// bins, and its samples.
    pub(crate) fn held_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.words.capacity() * 4
            + CHUNK_BRICKS * 5
            + self.lights.capacity() * std::mem::size_of::<(u16, Vec<LightSample>)>()
            + self
                .lights
                .iter()
                .map(|(_, l)| l.capacity() * std::mem::size_of::<LightSample>())
                .sum::<usize>()
    }
}

/// A bench's way in (not API): makes the images of full chunks under `table`'s flags and
/// returns each one's [`SlotImage::held_bytes`].
#[doc(hidden)]
pub fn slot_image_maker(table: &MaterialTable) -> impl Fn(ChunkCoord, &svet_world::Chunk) -> usize {
    let mut flags = MaterialFlags::new();
    flags.read(table);
    move |coord, chunk| SlotImage::of(coord, chunk, &flags).held_bytes()
}

#[cfg(test)]
mod tests;
