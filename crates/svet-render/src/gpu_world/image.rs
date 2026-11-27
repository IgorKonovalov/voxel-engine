//! A full chunk's *slot image*: what the mirror writes of the chunk into a new record, made
//! without the mirror (ADR-0056). Per brick its entry word less the slot's unit, per `Full`
//! brick its slot words padded to its bin, the per-bin counts and bytes a new record takes, and
//! the chunk's exposed emitters. The per-brick packing here ([`MaterialFlags::pack_brick`] and
//! the functions under it) is the one the mirror's own upload calls, so an image's words are
//! the upload's words by construction.

use super::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

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

/// Where a new full record's slot words are packed (ADR-0056): on the render thread at the
/// upload (`Inline`), or on the stream workers into a [`SlotImage`] the upload copies
/// (`Workers`). The mirror's bytes are the same under either.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Packing {
    #[default]
    Inline,
    Workers,
}

impl Packing {
    /// `inline` or `workers`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Inline => "inline",
            Self::Workers => "workers",
        }
    }

    /// The packing [`Packing::name`] names.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "inline" => Some(Self::Inline),
            "workers" => Some(Self::Workers),
            _ => None,
        }
    }

    /// The other packing.
    pub fn next(self) -> Self {
        match self {
            Self::Inline => Self::Workers,
            Self::Workers => Self::Inline,
        }
    }
}

/// The bytes of slot images alive at once ([`ImageHook`]'s default cap): about 1 300 images
/// at ADR-0056's average of 50 KB, or 127 at the worst.
pub const DEFAULT_IMAGE_CAP_BYTES: usize = 64 << 20;

/// What the hook's images share: the flags they are packed under, the switch, the cap and the
/// counters.
struct HookShared {
    flags: MaterialFlags,
    workers: AtomicBool,
    cap: usize,
    held: AtomicUsize,
    peak: AtomicUsize,
    made: AtomicU64,
    refused: AtomicU64,
    ns: AtomicU64,
}

/// The stream workers' half of [`Packing::Workers`] (ADR-0056), a part of
/// [`crate::ChunkWork`]: under `Workers` it makes each full chunk's [`SlotImage`] under the
/// flags of the table it was made with, while the bytes of the images alive (made and not yet
/// dropped by the mirror) stay within its cap; under `Inline` it makes none. Clones share the
/// switch, the cap and the counters.
#[derive(Clone)]
pub struct ImageHook(Arc<HookShared>);

/// What an [`ImageHook`] has done: images made and refused by the cap, the bytes held now and
/// at most, and the nanoseconds spent making the images.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImageHookStats {
    pub made: u64,
    pub refused: u64,
    pub held_bytes: u64,
    pub peak_bytes: u64,
    pub ns: u64,
}

impl ImageHook {
    /// A hook packing under `table`'s flags, its switch at `packing`, holding at most `cap`
    /// bytes of images alive ([`DEFAULT_IMAGE_CAP_BYTES`]).
    pub fn new(table: &MaterialTable, packing: Packing, cap: usize) -> Self {
        let mut flags = MaterialFlags::new();
        flags.read(table);
        Self(Arc::new(HookShared {
            flags,
            workers: AtomicBool::new(packing == Packing::Workers),
            cap,
            held: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            made: AtomicU64::new(0),
            refused: AtomicU64::new(0),
            ns: AtomicU64::new(0),
        }))
    }

    /// Switches the hook from the next chunk on.
    pub fn set_packing(&self, packing: Packing) {
        self.0
            .workers
            .store(packing == Packing::Workers, Ordering::Relaxed);
    }

    pub fn packing(&self) -> Packing {
        if self.0.workers.load(Ordering::Relaxed) {
            Packing::Workers
        } else {
            Packing::Inline
        }
    }

    pub fn stats(&self) -> ImageHookStats {
        let s = &self.0;
        ImageHookStats {
            made: s.made.load(Ordering::Relaxed),
            refused: s.refused.load(Ordering::Relaxed),
            held_bytes: s.held.load(Ordering::Relaxed) as u64,
            peak_bytes: s.peak.load(Ordering::Relaxed) as u64,
            ns: s.ns.load(Ordering::Relaxed),
        }
    }

    /// `chunk`'s image at `coord`, or `None` under `Inline` or when it would take the bytes
    /// alive past the cap (counted refused). The time is taken here, on the worker.
    pub(crate) fn make(&self, coord: ChunkCoord, chunk: &svet_world::Chunk) -> Option<HeldImage> {
        let s = &self.0;
        if !s.workers.load(Ordering::Relaxed) {
            return None;
        }
        if s.held.load(Ordering::Relaxed) >= s.cap {
            s.refused.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        let t = stopwatch();
        let image = SlotImage::of(coord, chunk, &s.flags);
        let ns = stopwatch().duration_since(t).as_nanos() as u64;
        let bytes = image.held_bytes();
        let before = s.held.fetch_add(bytes, Ordering::Relaxed);
        if before + bytes > s.cap {
            s.held.fetch_sub(bytes, Ordering::Relaxed);
            s.refused.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        s.peak.fetch_max(before + bytes, Ordering::Relaxed);
        s.made.fetch_add(1, Ordering::Relaxed);
        s.ns.fetch_add(ns, Ordering::Relaxed);
        Some(HeldImage {
            image,
            bytes,
            hook: self.0.clone(),
        })
    }
}

/// A slot image alive: its bytes count against its hook's cap until it is dropped.
pub(crate) struct HeldImage {
    pub(super) image: SlotImage,
    bytes: usize,
    hook: Arc<HookShared>,
}

impl Drop for HeldImage {
    fn drop(&mut self) {
        self.hook.held.fetch_sub(self.bytes, Ordering::Relaxed);
    }
}

/// The images handed to the mirror, each waiting by coordinate for its chunk's upload, its
/// eviction or its leaving the window.
#[derive(Default)]
pub(super) struct Images {
    pub(super) held: HashMap<ChunkCoord, HeldImage>,
}

impl GpuWorld {
    /// Takes the images among the stream workers' results ([`crate::ChunkWork`]) to wait for
    /// their chunks' uploads; under [`Packing::Inline`] they are dropped here. A chunk's newer
    /// image replaces an older one.
    pub fn take_images(&mut self, prepared: &[(ChunkCoord, svet_world::Prepared)]) {
        for (c, p) in prepared {
            let Ok(p) = p.0.clone().downcast::<crate::bake::ChunkPrepared>() else {
                continue;
            };
            let image = p
                .image
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            if let Some(image) = image
                && self.cfg.packing == Packing::Workers
            {
                self.images.held.insert(*c, image);
            }
        }
    }

    /// Switches the mirror's packing from the next sync on; `Inline` drops the images held.
    /// The hook that makes them ([`ImageHook::set_packing`]) is switched by its owner.
    pub fn set_packing(&mut self, packing: Packing) {
        self.cfg.packing = packing;
        if packing == Packing::Inline {
            self.images.held.clear();
        }
    }

    pub fn packing(&self) -> Packing {
        self.cfg.packing
    }

    /// Images waiting for their chunks' uploads.
    pub fn held_images(&self) -> usize {
        self.images.held.len()
    }

    /// Drops the images of chunks outside the window, which no upload takes.
    pub(super) fn prune_images(&mut self) {
        if self.images.held.is_empty() {
            return;
        }
        let w = self.window();
        self.images.held.retain(|c, _| w.contains(*c));
    }

    /// `coord`'s image for a new full record of `chunk`, taken out of the held ones, when it
    /// may stand in for the chunk (ADR-0056, Plan 0107's four conditions; the caller checked
    /// that no full record of the chunk exists): the field surface off, packed under the mirror's flags,
    /// and the chunk never edited. A refused image is dropped and counted by its reason.
    pub(super) fn usable_image(
        &mut self,
        coord: ChunkCoord,
        chunk: &svet_world::Chunk,
        st: &mut SyncStats,
    ) -> Option<HeldImage> {
        let image = self.images.held.remove(&coord)?;
        if self.field.band.is_some() {
            st.images_field += 1;
            return None;
        }
        if image.image.key != self.mats.key {
            st.images_other_flags += 1;
            return None;
        }
        if chunk.is_edited() {
            st.images_edited += 1;
            return None;
        }
        st.images_used += 1;
        Some(image)
    }

    /// Writes brick `slot` of a new record from `img` into entry `i`: a uniform brick's entry,
    /// or a `Full` brick's words, at `at` in the image's words, copied into a new slot of its
    /// bin. With `debug_assertions` the brick is packed inline as well and must match.
    #[allow(
        clippy::too_many_arguments,
        reason = "the image, the brick's place and the image's read offset"
    )]
    pub(super) fn upload_image_brick(
        &mut self,
        img: &SlotImage,
        slot: usize,
        i: usize,
        at: &mut usize,
        brick: &Brick,
        enc: &mut wgpu::CommandEncoder,
        st: &mut SyncStats,
    ) -> Result<(), MirrorError> {
        debug_assert_eq!(self.recs.entries[i], 0, "a new record's entry");
        let entry = img.entries[slot];
        let bin = img.bins[slot];
        let n = if bin == NO_BIN {
            0
        } else {
            BINS[usize::from(bin)] as usize * UNIT_WORDS
        };
        let words = &img.words[*at..*at + n];
        *at += n;
        #[cfg(debug_assertions)]
        {
            let mut inline = Vec::new();
            let packed = self.mats.pack_brick(brick, None, false, &mut inline);
            assert_eq!(packed.entry(), entry, "brick {slot}: the image's entry");
            if bin != NO_BIN {
                assert_eq!(words, &inline[..], "brick {slot}: the image's words");
            }
        }
        #[cfg(not(debug_assertions))]
        let _ = brick;
        if bin == NO_BIN {
            self.recs.entries[i] = entry;
            return Ok(());
        }
        let k = usize::from(bin);
        let u = self.pool.alloc(k)?;
        self.stage(k, u, words, enc, st);
        st.bytes += (words.len() * 4) as u64;
        self.recs.entries[i] = entry | u;
        st.bricks += 1;
        Ok(())
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
