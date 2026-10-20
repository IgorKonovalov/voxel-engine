//! The coarse grid: one cell per brick (8³ voxels), its solid count and dominant material.
//!
//! The grid is derived data the caller keeps beside the world. It is built from the resident
//! chunks, gains and drops a chunk's cells when the caller reports the chunk loaded or
//! evicted, and is updated from an edit's [`EditResult`]: only the bricks inside the result's
//! `aabb` are recomputed, and the cells that changed come back sorted by `(chunk, slot)`. The
//! edit itself never touches the grid, and the renderer's dirty bits are not read.

use std::collections::HashMap;

use crate::brick::Brick;
use crate::coords::{
    BRICK_EDGE, CHUNK_BRICKS, CHUNK_EDGE, CHUNK_EDGE_BRICKS, ChunkCoord, VoxelCoord, brick_of_slot,
    brick_slot,
};
use crate::edit::EditResult;
use crate::material::{AIR, MaterialId};
use crate::query::QueryStats;
use crate::store::{World, read};

/// One brick's summary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct CoarseCell {
    /// Non-air voxels in the brick, 0 to 512.
    pub solid: u16,
    /// The most frequent non-air material, the lower id on a tie; `AIR` when `solid == 0`.
    pub dominant: MaterialId,
}

impl CoarseCell {
    /// The cell of `brick`. A `Uniform` brick reads no voxel.
    pub fn of(brick: &Brick) -> Self {
        Self::of_counted(brick, &mut QueryStats::default())
    }

    /// [`CoarseCell::of`], counting a voxel-array read in `stats` for a `Full` brick.
    fn of_counted(brick: &Brick, stats: &mut QueryStats) -> Self {
        match brick {
            Brick::Uniform(AIR) => Self::default(),
            Brick::Uniform(m) => Self {
                solid: 512,
                dominant: *m,
            },
            Brick::Full(p) => {
                let mut v = [AIR; crate::coords::BRICK_VOXELS];
                p.decode(&mut v);
                stats.voxel_arrays_read += 1;
                // Few materials meet in one brick; a short list beats a map.
                let mut counts: Vec<(MaterialId, u16)> = Vec::new();
                let mut run = (v[0], 0u16);
                for &m in v.iter() {
                    if m == run.0 {
                        run.1 += 1;
                        continue;
                    }
                    tally(&mut counts, run);
                    run = (m, 1);
                }
                tally(&mut counts, run);
                let mut cell = Self::default();
                let mut best = 0;
                for (m, n) in counts {
                    if m == AIR {
                        continue;
                    }
                    cell.solid += n;
                    if n > best || (n == best && m < cell.dominant) {
                        best = n;
                        cell.dominant = m;
                    }
                }
                cell
            }
        }
    }
}

fn tally(counts: &mut Vec<(MaterialId, u16)>, (m, n): (MaterialId, u16)) {
    match counts.iter_mut().find(|(k, _)| *k == m) {
        Some((_, c)) => *c += n,
        None => counts.push((m, n)),
    }
}

/// A cell's global brick coordinate: `chunk * 8 + brick` on each axis. Cell `g` holds voxels
/// `[8 g, 8 g + 8)` along each axis.
///
/// The derived order (x, then y, then z) is the order every cell walk and cell list uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct CellCoord(pub [i32; 3]);

impl CellCoord {
    /// The cell of brick `slot` in chunk `chunk`.
    pub fn from(chunk: ChunkCoord, slot: usize) -> Self {
        let b = brick_of_slot(slot);
        Self([0, 1, 2].map(|k| chunk.0[k] * CHUNK_EDGE_BRICKS + b[k]))
    }

    /// The chunk holding this cell and the cell's brick slot in it.
    pub fn split(self) -> (ChunkCoord, usize) {
        let e = CHUNK_EDGE_BRICKS;
        (
            ChunkCoord(self.0.map(|v| v.div_euclid(e))),
            brick_slot(self.0.map(|v| v.rem_euclid(e))),
        )
    }

    /// The voxel at the cell's minimum corner.
    pub fn min_voxel(self) -> VoxelCoord {
        VoxelCoord(self.0.map(|v| v * BRICK_EDGE))
    }
}

impl PartialEq<[i32; 3]> for CellCoord {
    fn eq(&self, other: &[i32; 3]) -> bool {
        self.0 == *other
    }
}

/// A cell that an update changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CellChange {
    pub chunk: ChunkCoord,
    /// The brick slot, `x + 8 y + 64 z`.
    pub slot: u16,
    pub before: CoarseCell,
    pub after: CoarseCell,
}

impl CellChange {
    /// The global coordinate of the changed cell.
    pub fn cell(&self) -> CellCoord {
        CellCoord::from(self.chunk, usize::from(self.slot))
    }
}

/// One cell per brick of the chunks it holds; 4 bytes a cell, 2048 a chunk.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CoarseGrid {
    cells: HashMap<ChunkCoord, Box<[CoarseCell; CHUNK_BRICKS]>>,
}

/// The cells of the resident chunk `c`, or `None` when it is not resident.
fn chunk_cells(
    world: &World,
    c: ChunkCoord,
    stats: &mut QueryStats,
) -> Option<Box<[CoarseCell; CHUNK_BRICKS]>> {
    let chunk = world.store.get(c)?;
    let chunk = read(&chunk);
    let mut cells = Box::new([CoarseCell::default(); CHUNK_BRICKS]);
    for (cell, brick) in cells.iter_mut().zip(chunk.bricks()) {
        *cell = CoarseCell::of_counted(brick, stats);
    }
    Some(cells)
}

impl CoarseGrid {
    /// An empty grid.
    pub fn new() -> Self {
        Self::default()
    }

    /// The grid of every resident chunk.
    pub fn build(world: &World) -> Self {
        Self::build_with_stats(world).0
    }

    /// [`CoarseGrid::build`] plus the voxel arrays it read: one per `Full` brick.
    pub fn build_with_stats(world: &World) -> (Self, QueryStats) {
        let mut g = Self::new();
        let mut stats = QueryStats::default();
        for c in world.store.coords() {
            if let Some(cells) = chunk_cells(world, c, &mut stats) {
                g.cells.insert(c, cells);
            }
        }
        (g, stats)
    }

    /// Adds (or recomputes) chunk `c`'s 512 cells; nothing when `c` is not resident.
    pub fn chunk_loaded(&mut self, world: &World, c: ChunkCoord) {
        if let Some(cells) = chunk_cells(world, c, &mut QueryStats::default()) {
            self.cells.insert(c, cells);
        }
    }

    /// Drops chunk `c`'s cells.
    pub fn chunk_evicted(&mut self, c: ChunkCoord) {
        self.cells.remove(&c);
    }

    /// The cell of brick `slot` in chunk `chunk`, if the grid holds that chunk.
    pub fn cell(&self, chunk: ChunkCoord, slot: usize) -> Option<CoarseCell> {
        self.cells.get(&chunk).map(|c| c[slot])
    }

    /// The cell at global coordinate `c`, if the grid holds its chunk.
    pub fn cell_at(&self, c: CellCoord) -> Option<CoarseCell> {
        let (chunk, slot) = c.split();
        self.cell(chunk, slot)
    }

    /// The chunk's 512 cells, if the grid holds it.
    pub fn chunk(&self, chunk: ChunkCoord) -> Option<&[CoarseCell; CHUNK_BRICKS]> {
        self.cells.get(&chunk).map(|c| &**c)
    }

    pub fn contains(&self, chunk: ChunkCoord) -> bool {
        self.cells.contains_key(&chunk)
    }

    /// Chunks the grid holds, ascending.
    pub fn chunks(&self) -> Vec<ChunkCoord> {
        let mut v: Vec<_> = self.cells.keys().copied().collect();
        v.sort_unstable();
        v
    }

    /// Chunks the grid holds.
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Bytes of cell storage: 4 per brick.
    pub fn bytes(&self) -> usize {
        self.cells.len() * CHUNK_BRICKS * size_of::<CoarseCell>()
    }

    /// Recomputes the cells of the bricks inside `result.aabb`, in chunks the grid holds, and
    /// returns those that changed, sorted by `(chunk, slot)`.
    pub fn update(&mut self, world: &World, result: &EditResult) -> Vec<CellChange> {
        self.update_with_stats(world, result).0
    }

    /// [`CoarseGrid::update`] plus the voxel arrays it read: one per `Full` brick recomputed.
    pub fn update_with_stats(
        &mut self,
        world: &World,
        result: &EditResult,
    ) -> (Vec<CellChange>, QueryStats) {
        let mut stats = QueryStats::default();
        let mut out = Vec::new();
        let Some((lo, hi)) = result.aabb else {
            return (out, stats);
        };
        let (ce, be) = (CHUNK_EDGE, BRICK_EDGE);
        let clo = lo.0.map(|v| v.div_euclid(ce));
        let chi = hi.0.map(|v| v.div_euclid(ce));
        for cx in clo[0]..=chi[0] {
            for cy in clo[1]..=chi[1] {
                for cz in clo[2]..=chi[2] {
                    let c = [cx, cy, cz];
                    let coord = ChunkCoord(c);
                    let Some(cells) = self.cells.get_mut(&coord) else {
                        continue;
                    };
                    let Some(chunk) = world.store.get(coord) else {
                        continue;
                    };
                    let chunk = read(&chunk);
                    let blo = [0, 1, 2].map(|k| (lo.0[k].max(c[k] * ce) - c[k] * ce) / be);
                    let bhi = [0, 1, 2].map(|k| (hi.0[k].min(c[k] * ce + ce - 1) - c[k] * ce) / be);
                    // z, then y, then x: ascending slot order.
                    let slots = (blo[2]..=bhi[2]).flat_map(|bz| {
                        (blo[1]..=bhi[1]).flat_map(move |by| {
                            (blo[0]..=bhi[0]).map(move |bx| brick_slot([bx, by, bz]))
                        })
                    });
                    for slot in slots {
                        let after = CoarseCell::of_counted(chunk.brick(slot), &mut stats);
                        let before = cells[slot];
                        if after != before {
                            cells[slot] = after;
                            out.push(CellChange {
                                chunk: coord,
                                slot: slot as u16,
                                before,
                                after,
                            });
                        }
                    }
                }
            }
        }
        (out, stats)
    }
}
