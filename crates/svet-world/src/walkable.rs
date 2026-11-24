//! Walkable space: a bit per coarse cell, derived from a [`CoarseGrid`] and kept current from
//! the grid's [`CellChange`]s.
//!
//! Cell `c` is walkable when it is open (`solid <= max_solid`), the cell directly below is a
//! floor (`solid >= min_floor`), and the `clearance` cells above are open. A cell in a chunk the
//! grid does not hold is neither a floor nor open. So `c`'s bit depends on the cells from one
//! below it to `clearance` above it, and a change to cell `g` can flip only the cells from
//! `clearance` below `g` to one above it; an update recomputes exactly those.

use std::collections::HashMap;

use crate::coarse::{CellChange, CoarseCell, CoarseGrid};
use crate::coords::{CHUNK_BRICKS, CHUNK_EDGE_BRICKS, ChunkCoord, brick_of_slot, brick_slot};

/// What makes a cell walkable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WalkableParams {
    /// The most solid voxels an open cell holds.
    pub max_solid: u16,
    /// The fewest solid voxels a floor cell holds.
    pub min_floor: u16,
    /// Open cells needed above a walkable cell.
    pub clearance: u8,
}

/// Work an update did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct WalkableStats {
    /// Cells whose walkability was evaluated.
    pub cells_recomputed: u64,
}

/// One bit per coarse cell of the chunks the grid held when built or updated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WalkableMap {
    params: WalkableParams,
    masks: HashMap<ChunkCoord, [u64; CHUNK_BRICKS / 64]>,
}

/// Global brick coordinate of `slot` in chunk `c`.
fn global(c: ChunkCoord, slot: usize) -> [i32; 3] {
    let b = brick_of_slot(slot);
    [0, 1, 2].map(|k| c.0[k] * CHUNK_EDGE_BRICKS + b[k])
}

/// Chunk and slot of global brick `g`.
fn split(g: [i32; 3]) -> (ChunkCoord, usize) {
    let e = CHUNK_EDGE_BRICKS;
    (
        ChunkCoord(g.map(|v| v.div_euclid(e))),
        brick_slot(g.map(|v| v.rem_euclid(e))),
    )
}

fn cell_at(grid: &CoarseGrid, g: [i32; 3]) -> Option<CoarseCell> {
    let (c, slot) = split(g);
    grid.cell(c, slot)
}

impl WalkableParams {
    /// Whether global cell `g` of `grid` is walkable.
    fn walkable(&self, grid: &CoarseGrid, g: [i32; 3]) -> bool {
        let at = |dz: i32| cell_at(grid, [g[0], g[1], g[2] + dz]);
        let open = |c: Option<CoarseCell>| c.is_some_and(|c| c.solid <= self.max_solid);
        open(at(0))
            && at(-1).is_some_and(|c| c.solid >= self.min_floor)
            && (1..=i32::from(self.clearance)).all(|k| open(at(k)))
    }
}

impl WalkableMap {
    /// The walkable bits of every chunk `grid` holds.
    pub fn build(grid: &CoarseGrid, params: WalkableParams) -> Self {
        let mut masks = HashMap::new();
        for c in grid.chunks() {
            let mut m = [0u64; CHUNK_BRICKS / 64];
            for slot in 0..CHUNK_BRICKS {
                if params.walkable(grid, global(c, slot)) {
                    m[slot / 64] |= 1 << (slot % 64);
                }
            }
            masks.insert(c, m);
        }
        Self { params, masks }
    }

    pub fn params(&self) -> WalkableParams {
        self.params
    }

    /// Recomputes the cells `changes` can affect and returns those whose bit flipped, sorted
    /// by `(chunk, slot)`.
    pub fn update(&mut self, grid: &CoarseGrid, changes: &[CellChange]) -> Vec<(ChunkCoord, u16)> {
        self.update_with_stats(grid, changes).0
    }

    /// [`WalkableMap::update`] plus the work it did.
    pub fn update_with_stats(
        &mut self,
        grid: &CoarseGrid,
        changes: &[CellChange],
    ) -> (Vec<(ChunkCoord, u16)>, WalkableStats) {
        let mut stats = WalkableStats::default();
        let below = i32::from(self.params.clearance);
        let mut cells: Vec<(ChunkCoord, usize)> = Vec::new();
        for ch in changes {
            let g = global(ch.chunk, usize::from(ch.slot));
            for dz in -below..=1 {
                cells.push(split([g[0], g[1], g[2] + dz]));
            }
        }
        cells.sort_unstable();
        cells.dedup();
        let mut flipped = Vec::new();
        for (c, slot) in cells {
            if !grid.contains(c) {
                // The grid does not hold this chunk: it has no cells to be walkable.
                if let Some(m) = self.masks.get_mut(&c) {
                    *m = [0; CHUNK_BRICKS / 64];
                }
                continue;
            }
            stats.cells_recomputed += 1;
            let now = self.params.walkable(grid, global(c, slot));
            let m = self.masks.entry(c).or_insert([0; CHUNK_BRICKS / 64]);
            let bit = 1u64 << (slot % 64);
            if (m[slot / 64] & bit != 0) != now {
                m[slot / 64] ^= bit;
                flipped.push((c, slot as u16));
            }
        }
        (flipped, stats)
    }

    pub fn is_walkable(&self, chunk: ChunkCoord, slot: usize) -> bool {
        self.masks
            .get(&chunk)
            .is_some_and(|m| m[slot / 64] & (1 << (slot % 64)) != 0)
    }

    /// Walkable cells in all chunks.
    pub fn count(&self) -> usize {
        self.masks
            .values()
            .flat_map(|m| m.iter())
            .map(|w| w.count_ones() as usize)
            .sum()
    }

    /// The walkable cells, sorted by `(chunk, slot)`.
    pub fn cells(&self) -> Vec<(ChunkCoord, u16)> {
        let mut chunks: Vec<_> = self.masks.keys().copied().collect();
        chunks.sort_unstable();
        let mut out = Vec::new();
        for c in chunks {
            let m = &self.masks[&c];
            for slot in 0..CHUNK_BRICKS {
                if m[slot / 64] & (1 << (slot % 64)) != 0 {
                    out.push((c, slot as u16));
                }
            }
        }
        out
    }
}
