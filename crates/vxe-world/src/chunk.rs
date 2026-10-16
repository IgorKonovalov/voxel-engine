//! A chunk: 8 × 8 × 8 brick slots (64³ voxels) plus a dirty bit per slot (ADR-0002).
//!
//! The chunk is the unit of generation, upload and dirty tracking. It does not know its own
//! coordinate; the store's key is the coordinate.

use crate::brick::Brick;
use crate::coords::CHUNK_BRICKS;
use crate::material::{AIR, MaterialId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk {
    bricks: [Brick; CHUNK_BRICKS],
    /// Bit `s` set: slot `s` changed since the mirror last cleared it.
    dirty: [u64; CHUNK_BRICKS / 64],
    /// Set by any edit that changed a voxel, never cleared: an evicted edited chunk is
    /// persisted instead of dropped (ADR-0003).
    edited: bool,
}

impl Default for Chunk {
    fn default() -> Self {
        Self::uniform(AIR)
    }
}

impl Chunk {
    /// A chunk of one material with every slot marked dirty, so a fresh chunk is uploaded
    /// whole on the mirror's first sync.
    pub fn uniform(m: MaterialId) -> Self {
        Self {
            bricks: std::array::from_fn(|_| Brick::Uniform(m)),
            dirty: [u64::MAX; CHUNK_BRICKS / 64],
            edited: false,
        }
    }

    pub fn brick(&self, slot: usize) -> &Brick {
        &self.bricks[slot]
    }

    pub fn bricks(&self) -> &[Brick; CHUNK_BRICKS] {
        &self.bricks
    }

    /// Replaces slot `slot` and marks it dirty.
    pub fn set_brick(&mut self, slot: usize, brick: Brick) {
        self.bricks[slot] = brick;
        self.mark_dirty(slot);
    }

    /// Writable access for edits; the caller marks the slot dirty if it changed anything.
    pub(crate) fn brick_mut(&mut self, slot: usize) -> &mut Brick {
        &mut self.bricks[slot]
    }

    pub(crate) fn mark_dirty(&mut self, slot: usize) {
        self.dirty[slot / 64] |= 1 << (slot % 64);
    }

    pub fn is_dirty(&self, slot: usize) -> bool {
        self.dirty[slot / 64] & (1 << (slot % 64)) != 0
    }

    pub fn any_dirty(&self) -> bool {
        self.dirty.iter().any(|&w| w != 0)
    }

    /// Dirty slots in ascending order.
    pub fn dirty_slots(&self) -> impl Iterator<Item = usize> + '_ {
        (0..CHUNK_BRICKS).filter(|&s| self.is_dirty(s))
    }

    /// Clears every dirty bit; the mirror calls it after copying the dirty slots out.
    pub fn clear_dirty(&mut self) {
        self.dirty = [0; CHUNK_BRICKS / 64];
    }

    /// Marks every slot dirty, so a chunk coming back into residency is uploaded whole.
    pub(crate) fn mark_all_dirty(&mut self) {
        self.dirty = [u64::MAX; CHUNK_BRICKS / 64];
    }

    /// Whether an edit has ever changed a voxel of this chunk.
    pub fn is_edited(&self) -> bool {
        self.edited
    }

    pub(crate) fn mark_edited(&mut self) {
        self.edited = true;
    }
}
