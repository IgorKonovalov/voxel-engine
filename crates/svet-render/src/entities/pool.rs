//! The pool's bookkeeping (ADR-0060): which words each volume takes, which models claim it,
//! and the model table behind the handles. No device: [`super::EntityMirror`] writes what an
//! upload returns.
//!
//! A volume is keyed by its prefab's id and claimed once by each model holding it; it is freed
//! when its last claimant is, into a [`RangeAlloc`] of the pool's words. A volume never moves.
//! A model's new volumes take one range, laid in part order. A handle is a slot in the model
//! table and the slot's generation, bumped at each free, so a freed handle matches nothing.

use std::collections::{BTreeSet, HashMap};

use svet_entity::Model;
use svet_world::{AIR, MaterialTable, Prefab};

use super::alloc::RangeAlloc;
use super::{EntityUploadError, PALETTE, POOL_WORD, VOLUME_HEADER_WORDS};

/// A model uploaded to an [`super::EntityMirror`]: a slot in its model table (the low 16
/// bits) and the slot's generation (the high 16). Opaque: a handle comes from an upload, and
/// once its model is freed it names nothing, even after the slot is reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ModelHandle(u32);

impl ModelHandle {
    fn new(slot: u16, generation: u16) -> Self {
        Self(u32::from(slot) | u32::from(generation) << 16)
    }

    fn slot(self) -> usize {
        (self.0 & 0xffff) as usize
    }

    fn generation(self) -> u16 {
        (self.0 >> 16) as u16
    }
}

/// The bytes a free returned to the pool: the volumes no other model claims.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FreedBytes(pub u64);

/// One volume's place in the pool: its first word in the mirror's buffer and its size in
/// model voxels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Volume {
    pub(crate) word: u32,
    pub(crate) size: [u32; 3],
}

/// A volume in the pool and the models claiming it.
#[derive(Clone, Copy, Debug)]
struct Claimed {
    volume: Volume,
    /// Its words, from `volume.word - POOL_WORD`.
    len: u32,
    claims: u32,
}

/// A slot of the model table: its generation, and the model's parts (each part's prefab id
/// and volume) while it holds one.
#[derive(Clone, Debug, Default)]
struct ModelSlot {
    generation: u16,
    parts: Option<Vec<(u64, Volume)>>,
}

/// What an upload asks the mirror to write: `(word, words)`, each new volume at its first
/// word in the mirror's buffer.
pub struct PoolUpload {
    pub handle: ModelHandle,
    pub writes: Vec<(u32, Vec<u32>)>,
}

/// The pool's bookkeeping: the volumes, their claims, the free list and the model table.
#[derive(Clone, Debug)]
pub struct VolumePool {
    alloc: RangeAlloc,
    /// Looked up by prefab id, never iterated: no decision depends on a hash order.
    volumes: HashMap<u64, Claimed>,
    models: Vec<ModelSlot>,
    /// The model table's free slots; the lowest is reused first.
    free_slots: BTreeSet<u16>,
}

impl VolumePool {
    /// An empty pool of `pool_bytes` (rounded down to whole words).
    pub fn new(pool_bytes: u64) -> Self {
        let words = u32::try_from(pool_bytes / 4).expect("an entity pool under 16 GiB");
        Self {
            alloc: RangeAlloc::new(words),
            volumes: HashMap::new(),
            models: Vec::new(),
            free_slots: BTreeSet::new(),
        }
    }

    /// The pool's bytes.
    pub fn total_bytes(&self) -> u64 {
        u64::from(self.alloc.total()) * 4
    }

    /// The bytes the live volumes take.
    pub fn used_bytes(&self) -> u64 {
        u64::from(self.alloc.total() - self.alloc.free_total()) * 4
    }

    /// The free bytes, over every hole.
    pub fn free_bytes(&self) -> u64 {
        u64::from(self.alloc.free_total()) * 4
    }

    /// The largest hole's bytes: the most one upload's new volumes can take.
    pub fn largest_free_bytes(&self) -> u64 {
        u64::from(self.alloc.largest()) * 4
    }

    /// The free list.
    pub fn free_list(&self) -> &RangeAlloc {
        &self.alloc
    }

    /// Each part's volume of the model `handle` names; `None` for a handle freed or not this
    /// pool's.
    pub(crate) fn model(&self, handle: ModelHandle) -> Option<&[(u64, Volume)]> {
        let s = self.models.get(handle.slot())?;
        if s.generation != handle.generation() {
            return None;
        }
        s.parts.as_deref()
    }

    /// Each part's first word in the pool (0 is the pool's first word) of the model `handle`
    /// names; `None` for a handle freed or not this pool's.
    pub fn part_words(&self, handle: ModelHandle) -> Option<Vec<u32>> {
        self.model(handle)
            .map(|p| p.iter().map(|(_, v)| v.word - POOL_WORD as u32).collect())
    }

    /// Claims each part's volume of `model`, laying the volumes not yet in the pool into one
    /// range, first fit from the lowest word. Every material must have a row in `table`. On a
    /// refusal nothing changes.
    pub fn upload(
        &mut self,
        model: &Model,
        table: &MaterialTable,
    ) -> Result<PoolUpload, EntityUploadError> {
        let mut fresh: Vec<(u64, Vec<u32>)> = Vec::new();
        for (part, p) in model.parts().iter().enumerate() {
            let id = p.volume.id();
            if self.volumes.contains_key(&id) || fresh.iter().any(|(f, _)| *f == id) {
                continue;
            }
            fresh.push((id, volume_words(&p.volume, table, part)?));
        }
        let slot = match self.free_slots.first() {
            Some(&s) => s,
            None => {
                u16::try_from(self.models.len()).map_err(|_| EntityUploadError::TooManyModels)?
            }
        };
        let needed: usize = fresh.iter().map(|(_, w)| w.len()).sum();
        let mut at = if needed == 0 {
            0
        } else {
            let refused = EntityUploadError::PoolFull {
                needed: (needed * 4) as u64,
                free: self.free_bytes(),
                largest: self.largest_free_bytes(),
            };
            let n = u32::try_from(needed).map_err(|_| refused)?;
            self.alloc.alloc(n).ok_or(refused)?
        };
        let mut writes = Vec::with_capacity(fresh.len());
        for (id, words) in fresh {
            let len = words.len() as u32;
            let volume = Volume {
                word: POOL_WORD as u32 + at,
                size: [words[0], words[1], words[2]],
            };
            self.volumes.insert(
                id,
                Claimed {
                    volume,
                    len,
                    claims: 0,
                },
            );
            writes.push((volume.word, words));
            at += len;
        }
        let mut parts = Vec::with_capacity(model.parts().len());
        for (k, p) in model.parts().iter().enumerate() {
            let id = p.volume.id();
            let c = self.volumes.get_mut(&id).expect("claimed or laid above");
            if !model.parts()[..k].iter().any(|q| q.volume.id() == id) {
                c.claims += 1;
            }
            parts.push((id, c.volume));
        }
        if usize::from(slot) == self.models.len() {
            self.models.push(ModelSlot::default());
        }
        self.free_slots.remove(&slot);
        let s = &mut self.models[usize::from(slot)];
        s.parts = Some(parts);
        Ok(PoolUpload {
            handle: ModelHandle::new(slot, s.generation),
            writes,
        })
    }

    /// Drops the model's claim on each of its volumes; a volume nobody claims returns its
    /// words to the free list, in part order. The handle names nothing after.
    pub fn free_model(&mut self, handle: ModelHandle) -> Result<FreedBytes, EntityUploadError> {
        if self.model(handle).is_none() {
            return Err(EntityUploadError::UnknownModel);
        }
        let slot = handle.slot();
        let s = &mut self.models[slot];
        let parts = s.parts.take().expect("checked above");
        s.generation = s.generation.wrapping_add(1);
        self.free_slots.insert(slot as u16);
        let mut freed = 0u64;
        for (k, (id, _)) in parts.iter().enumerate() {
            if parts[..k].iter().any(|(q, _)| q == id) {
                continue;
            }
            let c = self
                .volumes
                .get_mut(id)
                .expect("a claimed volume is in the pool");
            c.claims -= 1;
            if c.claims == 0 {
                let c = self.volumes.remove(id).expect("present");
                self.alloc.free(c.volume.word - POOL_WORD as u32, c.len);
                freed += u64::from(c.len) * 4;
            }
        }
        Ok(FreedBytes(freed))
    }

    /// Frees every model: the pool is one free range and every handle names nothing.
    pub fn clear(&mut self) {
        self.alloc = RangeAlloc::new(self.alloc.total());
        self.volumes.clear();
        for (i, s) in self.models.iter_mut().enumerate() {
            if s.parts.take().is_some() {
                s.generation = s.generation.wrapping_add(1);
            }
            self.free_slots.insert(i as u16);
        }
    }
}

/// The words of `prefab` as a pool volume, or the part's refusal.
pub(crate) fn volume_words(
    prefab: &Prefab,
    table: &MaterialTable,
    part: usize,
) -> Result<Vec<u32>, EntityUploadError> {
    let size = prefab.size();
    let n = (size[0] * size[1] * size[2]) as usize;
    let blocks = size.map(|s| s.div_ceil(4));
    let nb = (blocks[0] * blocks[1] * blocks[2]) as usize;
    let occ = VOLUME_HEADER_WORDS;
    let blk = occ + n.div_ceil(32);
    let idx = blk + nb.div_ceil(32);
    let pal = idx + n.div_ceil(4);
    let mut w = vec![0u32; pal + PALETTE];
    w[..VOLUME_HEADER_WORDS].copy_from_slice(&[
        size[0], size[1], size[2], occ as u32, blk as u32, idx as u32, pal as u32, 0,
    ]);
    let mut palette: Vec<u16> = Vec::new();
    let mut i = 0usize;
    for z in 0..size[2] {
        for y in 0..size[1] {
            for x in 0..size[0] {
                let m = prefab.get([x as i32, y as i32, z as i32]).unwrap_or(AIR);
                if m != AIR {
                    if table.get(m).is_none() {
                        return Err(EntityUploadError::UnknownMaterial { part, material: m });
                    }
                    let k = match palette.iter().position(|&p| p == m) {
                        Some(k) => k,
                        None => {
                            if palette.len() == PALETTE - 1 {
                                return Err(EntityUploadError::Palette { part });
                            }
                            palette.push(m);
                            palette.len() - 1
                        }
                    };
                    w[occ + i / 32] |= 1 << (i % 32);
                    let b = (x / 4 + blocks[0] * (y / 4 + blocks[1] * (z / 4))) as usize;
                    w[blk + b / 32] |= 1 << (b % 32);
                    w[idx + i / 4] |= (k as u32) << (8 * (i % 4));
                }
                i += 1;
            }
        }
    }
    for (k, m) in palette.iter().enumerate() {
        w[pal + k] = u32::from(*m);
    }
    Ok(w)
}
