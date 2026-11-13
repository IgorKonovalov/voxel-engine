//! What a record's bricks become: the bricks a write covers, each brick's slot words and its
//! band slot, and the band's repair across chunk faces.

use super::*;

/// What the upload reads of each material, from the table at the first sync, and the GPU
/// rows written from it.
pub(super) struct MaterialFlags {
    /// Whether each material is `TRANSLUCENT`.
    pub(super) translucent: Vec<bool>,
    /// Whether each material has non-zero emission.
    pub(super) emissive: Vec<bool>,
    /// Each material's emitted luminance.
    pub(super) emit_lum: Vec<f32>,
    /// Whether each material is opaque (neither air nor `TRANSLUCENT`).
    pub(super) opaque: Vec<bool>,
    /// Whether the rows are written, and how many: the rows `mirror_bytes` counts.
    pub(super) written: bool,
    pub(super) rows: u64,
    /// [`GpuWorld::crisp_rows`].
    pub(super) crisp: bool,
    /// Rows written in place of the world's table's at the first sync, from
    /// [`GpuWorld::rewrite_materials`] before it.
    pub(super) rows_from: Option<MaterialTable>,
}

impl MaterialFlags {
    pub(super) fn new() -> Self {
        Self {
            translucent: vec![false; 1 << 16],
            emissive: vec![false; 1 << 16],
            emit_lum: vec![0.0; 1 << 16],
            opaque: vec![false; 1 << 16],
            written: false,
            rows: 0,
            crisp: false,
            rows_from: None,
        }
    }

    /// The flags of every row of `table`.
    pub(super) fn read(&mut self, table: &MaterialTable) {
        for (id, m) in table.rows().iter().enumerate() {
            self.translucent[id] = m.is_translucent();
            self.emissive[id] = m.emission.iter().any(|&e| e > 0.0);
            self.emit_lum[id] =
                0.2126 * m.emission[0] + 0.7152 * m.emission[1] + 0.0722 * m.emission[2];
            self.opaque[id] = id != usize::from(AIR) && !m.is_translucent();
        }
    }
}

/// Which of `len` bricks an upload writes: all when `every`, else the dirty ones and, with
/// `band` (a full chunk's bricks carrying band slots), the 26 neighbours of each dirty brick
/// within the chunk, whose band may have changed.
pub(super) fn upload_set(
    len: usize,
    every: bool,
    band: bool,
    is_dirty: impl Fn(usize) -> bool,
) -> Vec<bool> {
    if every {
        return vec![true; len];
    }
    let mut set: Vec<bool> = (0..len).map(&is_dirty).collect();
    if band {
        for i in 0..len {
            if !is_dirty(i) {
                continue;
            }
            let b = brick_of_slot(i);
            for k in 0..27 {
                let d = [k % 3 - 1, (k / 3) % 3 - 1, k / 9 - 1];
                let n = [0, 1, 2].map(|a| b[a] + d[a]);
                if n.iter().all(|v| (0..CHUNK_EDGE_BRICKS).contains(v)) {
                    set[brick_slot(n)] = true;
                }
            }
        }
    }
    set
}

/// The bricks one record is written from, read in place.
pub(super) enum Bricks<'a> {
    /// A full chunk's 512 bricks and its dirty bits.
    Chunk(&'a vxe_world::Chunk),
    /// A LOD record's mip-bricks, level 1 first; every one counts as dirty.
    Mips(&'a ChunkMips),
}

impl<'a> Bricks<'a> {
    /// A LOD record's bricks: the chunk's mips.
    pub(super) fn is_lod(&self) -> bool {
        matches!(self, Self::Mips(_))
    }

    pub(super) fn len(&self) -> usize {
        match self {
            Self::Chunk(_) => CHUNK_BRICKS,
            Self::Mips(m) => (1..=MIP_LEVELS).map(|n| m.level(n).len()).sum(),
        }
    }

    /// Entry `i`'s brick: a chunk's brick `i`, or the mip-brick at record entry `i`, each
    /// level starting at its `LOD_LEVEL_ENTRY`.
    pub(super) fn get(&self, i: usize) -> &'a Brick {
        match self {
            Self::Chunk(ch) => &ch.bricks()[i],
            Self::Mips(m) => {
                let k = LOD_LEVEL_ENTRY.iter().rposition(|&e| e <= i).unwrap_or(0);
                &m.level(k + 1)[i - LOD_LEVEL_ENTRY[k]]
            }
        }
    }

    pub(super) fn is_dirty(&self, i: usize) -> bool {
        match self {
            Self::Chunk(ch) => ch.is_dirty(i),
            Self::Mips(_) => true,
        }
    }
}

/// Whether a material of `p` satisfies `f`: its palette entries, or a `Wide` brick's ids.
pub(super) fn materials_of(p: &Packed, f: impl Fn(MaterialId) -> bool) -> bool {
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

impl GpuWorld {
    /// The units of `p`'s slot ([`slot_units`]): mask, header, payload, and a translucent unit
    /// for a planar brick holding a translucent voxel.
    pub(super) fn slot_units(&self, p: &Packed) -> u32 {
        slot_units(p, |m| self.mats.translucent[usize::from(m)])
    }

    /// What uploading `bricks` into record `rec` (`None`: a new record) takes: the new slots
    /// of each bin (a brick keeps its slot only when its bin is unchanged) and the slot bytes
    /// written, over the bricks [`upload_set`] names, whose band masks, for a full chunk's
    /// bricks carrying band slots, are left in `bands` by brick slot. Reads only; `sync`
    /// calls it to select a chunk and again, under the chunk's write lock, as the upload's
    /// backstop, whose bands the upload then writes.
    pub(super) fn need(
        &self,
        world: &World,
        coord: ChunkCoord,
        bricks: &Bricks<'_>,
        rec: Option<u32>,
        bands: &mut Vec<Option<[u64; 8]>>,
    ) -> ([u64; BIN_COUNT], u64) {
        let band = self.field.band.as_ref().filter(|_| !bricks.is_lod());
        let set = upload_set(bricks.len(), rec.is_none(), band.is_some(), |i| {
            bricks.is_dirty(i)
        });
        bands.clear();
        bands.resize(bricks.len(), None);
        if band.is_some()
            && let Bricks::Chunk(ch) = bricks
        {
            self.with_around(world, coord, ch, |around| {
                for (i, &up) in set.iter().enumerate() {
                    if up {
                        let m = crate::field::band_mask(around, i);
                        bands[i] = (m != [0; 8]).then_some(m);
                    }
                }
            });
        }
        let mut new = [0u64; BIN_COUNT];
        let mut bytes = 0;
        for (i, &up) in set.iter().enumerate() {
            if !up {
                continue;
            }
            let old = rec.map_or(0, |r| {
                self.recs.entries[r as usize * RECORD_WORDS + ENTRY_WORD + i]
            });
            if let (Some(b), Some(m)) = (band, bands[i]) {
                let k = band_bin_of(crate::field::band_slot_units(crate::field::band_count(&m)));
                bytes += u64::from(SLOT_BINS[k]) * UNIT_BYTES;
                let had = b.slot(coord, i);
                if had == ABSENT || self.bin_of_unit(had) != Some(k) {
                    new[k] += 1;
                }
            }
            let units = match bricks.get(i) {
                Brick::Full(p) => self.slot_units(p),
                Brick::Uniform(m) if bands[i].is_some() => {
                    2 + u32::from(self.mats.translucent[usize::from(*m)])
                }
                Brick::Uniform(_) => continue,
            };
            let k = bin_of(units);
            bytes += u64::from(BINS[k]) * UNIT_BYTES;
            if old & FULL_BIT == 0 || self.bin_of_unit(old & UNIT_MASK) != Some(k) {
                new[k] += 1;
            }
        }
        (new, bytes)
    }

    /// Runs `f` on the 27 chunks around `coord` as the band reads them ([`crate::field`]):
    /// `centre` at the centre, every other chunk inside the window the world holds full by a
    /// read lock, one it holds as mips only by its mips, the rest as air.
    pub(super) fn with_around<R>(
        &self,
        world: &World,
        coord: ChunkCoord,
        centre: &vxe_world::Chunk,
        f: impl FnOnce(&[crate::field::Around<'_>; 27]) -> R,
    ) -> R {
        use crate::field::Around;
        let mut arcs = Vec::with_capacity(27);
        let mut lods = Vec::with_capacity(27);
        for k in 0..27 {
            let d = [k % 3 - 1, (k / 3) % 3 - 1, k / 9 - 1];
            let n = ChunkCoord([0, 1, 2].map(|a| coord.0[a] + d[a]));
            if k == 13 || !self.in_window(n) {
                arcs.push(None);
                lods.push(None);
            } else if let Some(a) = world.store().get(n) {
                arcs.push(Some(a));
                lods.push(None);
            } else {
                arcs.push(None);
                lods.push(world.lod(n));
            }
        }
        let guards: Vec<_> = arcs
            .iter()
            .map(|a| {
                a.as_ref()
                    .map(|a| a.read().unwrap_or_else(PoisonError::into_inner))
            })
            .collect();
        let around: [Around<'_>; 27] = std::array::from_fn(|k| {
            if k == 13 {
                Around::Full(centre)
            } else if let Some(g) = &guards[k] {
                Around::Full(g)
            } else if let Some(m) = &lods[k] {
                Around::Lod(m)
            } else {
                Around::Air
            }
        });
        f(&around)
    }

    /// Writes one chunk's record: `bricks` into its entries (512 voxel bricks, or a LOD
    /// record's 73 mip-bricks): every brick when `coord` had no record before this call, else
    /// the dirty ones; then, for a record allocated here, its table entry.
    pub(super) fn upload_record(
        &mut self,
        coord: ChunkCoord,
        bricks: Bricks<'_>,
        enc: &mut wgpu::CommandEncoder,
        st: &mut SyncStats,
    ) -> Result<(), MirrorError> {
        let lod = bricks.is_lod();
        let (rec, kept_emits, new) = match self.recs.mirrored.remove(&coord) {
            Some(m) => (m.record, m.emits, false),
            None => (
                self.recs.alloc(self.pool.layout.reserved_records)?,
                false,
                true,
            ),
        };
        let base = rec as usize * RECORD_WORDS;
        if new {
            self.recs.entries[base..base + RECORD_WORDS].fill(0);
        }
        let every = new;
        // The record has held an emissive material while mirrored: without one no brick of it
        // has an emitter.
        let left = self.lap(Stage::Light);
        let emits = !lod
            && (kept_emits
                || (0..bricks.len()).any(|i| {
                    (every || bricks.is_dirty(i))
                        && matches!(bricks.get(i), Brick::Full(p)
                            if materials_of(p, |m| self.mats.emissive[usize::from(m)]))
                }));
        self.lap(left);
        let mut rescan = BTreeSet::new();
        if !emits && !lod {
            self.drop_lights(coord);
        }
        if every && let Some(l) = self.lights.chunks.get_mut(&coord) {
            l.bricks.clear();
        }
        if self.field.band.is_some() {
            if every || lod {
                self.field.shells.insert(coord);
            }
            if every && !lod {
                self.field.chunks.insert(coord);
            }
        }
        let mut result = Ok(());
        let band_on = self.field.band.is_some() && !lod;
        let set = upload_set(bricks.len(), every, band_on, |i| bricks.is_dirty(i));
        for (i, &up) in set.iter().enumerate() {
            if !up {
                continue;
            }
            if self.field.band.is_some() && !every && !lod && bricks.is_dirty(i) {
                let b = brick_of_slot(i);
                self.field
                    .bricks
                    .insert([0, 1, 2].map(|k| coord.0[k] * CHUNK_EDGE_BRICKS + b[k]));
            }
            // The upload's backstop computed the bands under the same write lock.
            let band = band_on.then(|| self.field.bands.get(i).copied().flatten());
            result = self.upload_brick(
                coord,
                i,
                base + ENTRY_WORD + i,
                bricks.get(i),
                band,
                enc,
                st,
            );
            if result.is_err() {
                break;
            }
            if band_on {
                let fresh = self
                    .field
                    .band
                    .as_mut()
                    .expect("band_on")
                    .fresh
                    .entry(coord);
                fresh.or_default()[i / 64] |= 1 << (i % 64);
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
            let left = self.lap(Stage::Light);
            for &i in &rescan {
                self.note_lights(coord, i, &bricks, st);
            }
            self.lap(left);
            if self.lights.chunks.contains_key(&coord) {
                self.lights.dirty.insert(coord);
            }
        }
        let empty = self.recs.write_bits(base);
        self.note_change(coord);
        self.recs.mirrored.insert(
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
        if new || self.window.table[t] != want {
            self.window.table[t] = want;
            self.window.table_changed.push(t as u32);
        }
        result?;
        st.chunks += 1;
        self.unqueue(coord);
        Ok(())
    }

    /// Brings the band slots of the bricks this sync's writes reach across chunk faces up to
    /// date (ADR-0041): the bricks within one brick of a chunk written whole, written new as a
    /// LOD record or dropped, and the 3 x 3 x 3 bricks around each dirty brick written, less
    /// the bricks written in this sync, whose band is current, and the dirty bricks, which
    /// their chunk's upload writes with their band. Each such brick of a chunk
    /// mirrored full takes its band from the world ([`GpuWorld::with_around`]): its band slot
    /// is placed again ([`GpuWorld::place_band`]), a `Full` brick's header restaged when the
    /// slot moved, a uniform brick's entry rewritten when it gains or loses its band. A brick
    /// whose band slot finds no room keeps what it had and is tried again at the next sync.
    pub(super) fn fix_band(
        &mut self,
        world: &World,
        enc: &mut wgpu::CommandEncoder,
        st: &mut SyncStats,
    ) -> Result<(), MirrorError> {
        let Some(band) = &mut self.field.band else {
            return Ok(());
        };
        let retry = std::mem::take(&mut band.retry);
        let whole = std::mem::take(&mut self.field.chunks);
        let dirty = std::mem::take(&mut self.field.bricks);
        let shells = std::mem::take(&mut self.field.shells);
        let e = CHUNK_EDGE_BRICKS;
        let mut touched: BTreeMap<[i32; 3], [u64; 8]> = BTreeMap::new();
        let mut touch = |b: [i32; 3]| {
            let c = b.map(|v| v.div_euclid(e));
            let i = brick_slot(b.map(|v| v.rem_euclid(e)));
            touched.entry([c[2], c[1], c[0]]).or_default()[i / 64] |= 1 << (i % 64);
        };
        for c in whole.iter().chain(&shells) {
            let lo = c.0.map(|v| v * e - 1);
            for z in lo[2]..lo[2] + e + 2 {
                for y in lo[1]..lo[1] + e + 2 {
                    for x in lo[0]..lo[0] + e + 2 {
                        let b = [x, y, z];
                        if (0..3).any(|k| b[k] == lo[k] || b[k] == lo[k] + e + 1) {
                            touch(b);
                        }
                    }
                }
            }
        }
        for b in &dirty {
            for k in 0..27 {
                let d = [k % 3 - 1, (k / 3) % 3 - 1, k / 9 - 1];
                touch([0, 1, 2].map(|a| b[a] + d[a]));
            }
        }
        for &b in &retry {
            touch(b);
        }
        let mut retry = HashSet::new();
        for (key, bits) in touched {
            let c = ChunkCoord([key[2], key[1], key[0]]);
            let Some(rec) = self
                .recs
                .mirrored
                .get(&c)
                .filter(|m| !m.lod)
                .map(|m| m.record)
            else {
                continue;
            };
            let Some(arc) = world.store().get(c) else {
                continue;
            };
            let ch = arc.read().unwrap_or_else(PoisonError::into_inner);
            let band = self.field.band.as_ref().expect("on");
            // A dirty brick's slot waits for its chunk's upload, which takes its band then.
            let slots: Vec<usize> = (0..CHUNK_BRICKS)
                .filter(|&i| {
                    bits[i / 64] >> (i % 64) & 1 != 0 && !band.is_fresh(c, i) && !ch.is_dirty(i)
                })
                .collect();
            if slots.is_empty() {
                continue;
            }
            let masks: Vec<(usize, Option<[u64; 8]>)> = self.with_around(world, c, &ch, |around| {
                slots
                    .iter()
                    .map(|&i| {
                        let m = crate::field::band_mask(around, i);
                        (i, (m != [0; 8]).then_some(m))
                    })
                    .collect()
            });
            let base = rec as usize * RECORD_WORDS;
            let mut rewritten = false;
            for (i, mask) in masks {
                let had = self.field.band.as_ref().expect("on").slot(c, i);
                if had == ABSENT && mask.is_none() {
                    continue;
                }
                let at = base + ENTRY_WORD + i;
                let entry = self.recs.entries[at];
                let r = match ch.brick(i) {
                    Brick::Full(p) => self.place_band(c, i, mask, enc, st).map(|ptr| {
                        // The slot keeps its unit and bin: only its header names the band and
                        // holds its prefix, which a new mask changes even in the same slot.
                        let mut words = std::mem::take(&mut self.up.slot_words);
                        self.fill_slot(p, ptr.zip(mask.as_ref()), &mut words);
                        st.packed_bricks += 1;
                        let u = entry & UNIT_MASK;
                        let k = self.bin_of_unit(u).expect("a live slot");
                        self.stage(k, u + 1, &words[UNIT_WORDS..2 * UNIT_WORDS], enc, st);
                        st.bytes += UNIT_BYTES;
                        self.up.slot_words = words;
                    }),
                    b @ Brick::Uniform(_) => {
                        let r = self.upload_brick(c, i, at, b, Some(mask), enc, st);
                        rewritten |= self.recs.entries[at] != entry;
                        r
                    }
                };
                if r.is_err() {
                    retry.insert(crate::field::world_brick(c, i));
                }
            }
            if rewritten {
                let empty = self.recs.write_bits(base);
                self.stage_record(rec, enc, st);
                st.bytes += RECORD_BYTES;
                let t = self.table_index(c) as usize;
                let want = rec | if empty { EMPTY_BIT } else { 0 };
                if self.window.table[t] != want {
                    self.window.table[t] = want;
                    self.window.table_changed.push(t as u32);
                }
            }
        }
        let band = self.field.band.as_mut().expect("on");
        band.fresh.clear();
        band.retry = retry;
        Ok(())
    }

    /// Writes brick `slot` of `coord`, `brick`, into record word `i`, a brick entry: a `Full`
    /// brick as its slot, staged from its packed words ([`GpuWorld::fill_slot`]); a uniform one
    /// as its material. With `band`
    /// (a full chunk's brick in a mirror with the field on), first its band slot for the band
    /// mask it holds ([`GpuWorld::place_band`]), and a uniform band brick as a
    /// [`CLASS_UNIFORM`] slot. A brick whose slot changes bin takes a slot of the new bin,
    /// then returns the old one.
    #[allow(
        clippy::too_many_arguments,
        reason = "the brick's place, its contents and its band mask"
    )]
    pub(super) fn upload_brick(
        &mut self,
        coord: ChunkCoord,
        slot: usize,
        i: usize,
        brick: &Brick,
        band: Option<Option<[u64; 8]>>,
        enc: &mut wgpu::CommandEncoder,
        st: &mut SyncStats,
    ) -> Result<(), MirrorError> {
        let old = self.recs.entries[i];
        let old_unit = (old & FULL_BIT != 0).then_some(old & UNIT_MASK);
        let mask = band.flatten();
        let ptr = match band {
            Some(mask) => self.place_band(coord, slot, mask, enc, st)?,
            None => None,
        };
        let header = ptr.zip(mask.as_ref());
        let mut words = std::mem::take(&mut self.up.slot_words);
        let left = self.lap(Stage::Pack);
        let packed = match brick {
            Brick::Full(p) => Some((self.fill_slot(p, header, &mut words), class_code(p.class()))),
            Brick::Uniform(m) if ptr.is_some() => Some((
                self.fill_uniform_slot(*m, header, &mut words),
                CLASS_UNIFORM,
            )),
            Brick::Uniform(_) => None,
        };
        self.lap(left);
        st.packed_bricks += u64::from(packed.is_some());
        let (flag, code) = match (packed, brick) {
            (Some(p), _) => p,
            (None, Brick::Full(_)) => unreachable!("a full brick is packed above"),
            (None, Brick::Uniform(m)) => {
                self.up.slot_words = words;
                if let Some(u) = old_unit {
                    self.pool.free_slot(u);
                }
                self.recs.entries[i] = u32::from(*m)
                    | if self.mats.translucent[usize::from(*m)] {
                        TRANSLUCENT_BIT
                    } else {
                        0
                    };
                return Ok(());
            }
        };
        let k = bin_of((words.len() / UNIT_WORDS) as u32);
        // The rest of the bin's slot is padding that no reader reads.
        words.resize(BINS[k] as usize * UNIT_WORDS, u32::MAX);
        let u = match old_unit {
            Some(u) if self.bin_of_unit(u) == Some(k) => u,
            _ => match self.pool.alloc(k) {
                Ok(u) => {
                    if let Some(o) = old_unit {
                        self.pool.free_slot(o);
                    }
                    u
                }
                Err(e) => {
                    self.up.slot_words = words;
                    return Err(e);
                }
            },
        };
        self.stage(k, u, &words, enc, st);
        st.bytes += (words.len() * 4) as u64;
        self.recs.entries[i] = FULL_BIT | flag | code << CLASS_SHIFT | u;
        self.up.slot_words = words;
        st.bricks += 1;
        Ok(())
    }

    /// The band slot of brick `slot` of `coord` for its band `mask` (ADR-0041): with a band,
    /// its slot kept when it is of the mask's exact size, else a new one taken and the old one
    /// freed, its mask staged with zeroed counts and the slot listed for the field pass; without
    /// one, its slot freed. Returns the slot's first unit.
    pub(super) fn place_band(
        &mut self,
        coord: ChunkCoord,
        slot: usize,
        mask: Option<[u64; 8]>,
        enc: &mut wgpu::CommandEncoder,
        st: &mut SyncStats,
    ) -> Result<Option<u32>, MirrorError> {
        let left = self.lap(Stage::Pack);
        let r = self.place_band_words(coord, slot, mask, enc, st);
        self.lap(left);
        r
    }

    /// [`GpuWorld::place_band`]'s work, on the stage clock's [`Stage::Pack`].
    fn place_band_words(
        &mut self,
        coord: ChunkCoord,
        slot: usize,
        mask: Option<[u64; 8]>,
        enc: &mut wgpu::CommandEncoder,
        st: &mut SyncStats,
    ) -> Result<Option<u32>, MirrorError> {
        let had = self
            .field
            .band
            .as_ref()
            .expect("the field is on")
            .slot(coord, slot);
        let drop_old = |gw: &mut Self| {
            if had != ABSENT {
                gw.field.band.as_mut().expect("on").items.remove(&had);
                gw.pool.free_slot(had);
            }
        };
        let Some(m) = mask else {
            drop_old(self);
            if had != ABSENT {
                self.field
                    .band
                    .as_mut()
                    .expect("on")
                    .slots
                    .get_mut(&coord)
                    .expect("held")[slot] = ABSENT;
            }
            return Ok(None);
        };
        let units = crate::field::band_slot_units(crate::field::band_count(&m));
        let k = band_bin_of(units);
        let u = if had != ABSENT && self.bin_of_unit(had) == Some(k) {
            had
        } else {
            let u = self.pool.alloc(k)?;
            drop_old(self);
            u
        };
        let band = self.field.band.as_mut().expect("on");
        band.slots
            .entry(coord)
            .or_insert_with(|| Box::new([ABSENT; CHUNK_BRICKS]))[slot] = u;
        band.items.insert(
            u,
            crate::field::world_brick(coord, slot).map(|v| v * BRICK_EDGE),
        );
        let mut words = vec![0u32; units as usize * UNIT_WORDS];
        for (z, w) in m.iter().enumerate() {
            words[2 * z] = *w as u32;
            words[2 * z + 1] = (*w >> 32) as u32;
        }
        self.stage(k, u, &words, enc, st);
        st.bytes += (words.len() * 4) as u64;
        Ok(Some(u))
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
        let translucent = self.mats.translucent[usize::from(m)];
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

    /// Fills `out` with the CPU units of `p`'s slot (module docs): the opaque mask (the packed
    /// mask less the translucent voxels), the header, the planes or raw ids copied from the
    /// packed words, the translucent unit of a planar brick holding a translucent voxel, and
    /// `band`, a band slot's first unit and its band mask, in the header
    /// ([`band_header`]). Returns `TRANSLUCENT_BIT`
    /// when the brick holds a translucent voxel, else 0.
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
        let translucent = |m: MaterialId| self.mats.translucent[usize::from(m)];
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
