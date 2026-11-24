//! The region's bricks as an op found them, the scans down its columns, and a drop's yield
//! rule.

use std::collections::{BTreeMap, HashMap};

use super::{BE, brick_columns, cell_of, index_of};
use crate::brick::Brick;
use crate::coarse::CellCoord;
use crate::coords::ChunkCoord;
use crate::edit::EditError;
use crate::material::{AIR, MaterialId, MaterialTable};
use crate::store::{ChunkMap, ChunkReader};
use crate::support::{CellKey, CellMap};

/// The region's bricks as they were when the op started.
pub(super) struct Snapshot {
    /// As the world holds them: read for the scans, the counts and the composed writes.
    pub(super) packed: CellMap<Brick>,
    /// Per cell column `(x, y)` of the footprint, ascending, the region's cells' `z`,
    /// ascending.
    pub(super) stacks: BTreeMap<[i32; 2], Vec<i32>>,
}

/// A voxel column of the region's footprint: its `(x, y)` and, per region cell it crosses
/// (ascending `z`), the cell's `z` and the column's 8 voxels in the cell's brick, bottom first.
pub(super) type Column = ([i32; 2], Vec<(i32, [MaterialId; 8])>);

impl Snapshot {
    /// Checks residency (the least missing chunk) and copies the bricks.
    pub(super) fn take(map: &ChunkMap, cells: &[CellCoord]) -> Result<Self, EditError> {
        let mut cells = cells.to_vec();
        cells.sort_unstable();
        cells.dedup();
        if let Some(m) = cells
            .iter()
            .map(|c| c.split().0)
            .filter(|c| !map.contains_key(c))
            .min()
        {
            return Err(EditError::NotResident(m));
        }
        let mut packed = CellMap::default();
        let mut stacks: BTreeMap<[i32; 2], Vec<i32>> = BTreeMap::new();
        let mut reader = ChunkReader::new(map);
        for &c in &cells {
            let b = reader.brick(c).expect("residency checked above").clone();
            packed.insert(CellKey(c), b);
            stacks.entry([c.0[0], c.0[1]]).or_default().push(c.0[2]);
        }
        Ok(Self { packed, stacks })
    }

    pub(super) fn brick(&self, c: CellCoord) -> &Brick {
        &self.packed[&CellKey(c)]
    }

    /// Every voxel column of the footprint that holds a solid region voxel, ascending by
    /// `(x, y)`. Each brick is decoded once, into its 64 columns.
    pub(super) fn columns(&self) -> Vec<Column> {
        let mut out = Vec::new();
        let mut stack: Vec<(i32, [[MaterialId; 8]; 64])> = Vec::new();
        for (&[cx, cy], zs) in &self.stacks {
            stack.clear();
            stack.extend(
                zs.iter()
                    .map(|&cz| (cz, brick_columns(self.brick(CellCoord([cx, cy, cz]))))),
            );
            // `xy = lx + 8 ly`: local y, then local x, ascending.
            for xy in 0..64 {
                let parts: Vec<_> = stack.iter().map(|(cz, cols)| (*cz, cols[xy])).collect();
                if parts.iter().any(|(_, col)| col.iter().any(|&m| m != AIR)) {
                    let (lx, ly) = ((xy % 8) as i32, (xy / 8) as i32);
                    out.push(([cx * BE + lx, cy * BE + ly], parts));
                }
            }
        }
        out
    }

    /// Voxels below `z` (exclusive) in column `(x, y)` that are free to fall into, counted down
    /// to the first solid voxel outside the region that does not yield, at most `max`. A
    /// region voxel is free unless `spare` holds for its material (a voxel the op leaves in
    /// place); a voxel outside the region is free when it is air or `yields` says it yields
    /// (no voxel does when `yields` is `None`). Fails with the chunk of the first non-resident
    /// voxel reached.
    pub(super) fn gap_below(
        &self,
        reader: &mut ChunkReader<'_>,
        [x, y]: [i32; 2],
        z: i32,
        max: i32,
        spare: &impl Fn(MaterialId) -> bool,
        mut yields: Option<&mut Yield<'_>>,
    ) -> Result<i32, ChunkCoord> {
        let mut gap = 0;
        while gap < max {
            let top = z - gap - 1;
            let cell = cell_of([x, y, top]);
            // Voxels from `top` down to the bottom of its cell.
            let span = (top.rem_euclid(BE) + 1).min(max - gap);
            let brick = match self.packed.get(&CellKey(cell)) {
                Some(b) => match b {
                    Brick::Uniform(m) if !spare(*m) => {
                        gap += span;
                        continue;
                    }
                    Brick::Uniform(_) => break,
                    full @ Brick::Full(_) => {
                        for k in 0..span {
                            if spare(full.get(index_of([x, y, top - k]))) {
                                return Ok(gap);
                            }
                            gap += 1;
                        }
                        continue;
                    }
                },
                None => reader.brick(cell)?,
            };
            let mut free = |m: MaterialId| {
                m == AIR
                    || yields
                        .as_deref_mut()
                        .is_some_and(|y| y.yields(cell, brick, m))
            };
            match brick {
                Brick::Uniform(AIR) => gap += span,
                Brick::Uniform(m) => {
                    if !free(*m) {
                        break;
                    }
                    gap += span;
                }
                full @ Brick::Full(_) => {
                    for k in 0..span {
                        if !free(full.get(index_of([x, y, top - k]))) {
                            return Ok(gap);
                        }
                        gap += 1;
                    }
                }
            }
        }
        Ok(gap)
    }
}

/// A drop's yield rule (ADR-0016): a solid voxel outside the region yields when its cell
/// holds fewer than `below` solid voxels and its material is not `INDESTRUCTIBLE`. A cell's
/// count is its brick's solid-mask popcount, read when the op first reaches the cell and
/// kept for the op, so the rule sees the world as the op found it.
pub(super) struct Yield<'t> {
    below: u32,
    materials: &'t MaterialTable,
    /// Per cell reached, its solid count and whether one of its voxels has yielded.
    counts: HashMap<CellCoord, (u32, bool)>,
    /// The cells a voxel has yielded in, in the order they first did: the only cells a drop
    /// can crush voxels of.
    pub(super) yielded: Vec<CellCoord>,
}

impl<'t> Yield<'t> {
    pub(super) fn new(below: u16, materials: &'t MaterialTable) -> Self {
        Self {
            below: u32::from(below),
            materials,
            counts: HashMap::new(),
            yielded: Vec::new(),
        }
    }

    /// Whether solid voxel material `m` of cell `cell`, whose brick is `brick`, yields.
    pub(super) fn yields(&mut self, cell: CellCoord, brick: &Brick, m: MaterialId) -> bool {
        if self.materials.is_indestructible(m) {
            return false;
        }
        // A uniform solid brick holds 512 solid voxels: no count to look up below that.
        if matches!(brick, Brick::Uniform(_)) && self.below <= 512 {
            return false;
        }
        let (n, seen) = self.counts.entry(cell).or_insert_with(|| {
            let n = brick.solid_bits().iter().map(|w| w.count_ones()).sum();
            (n, false)
        });
        let yields = *n < self.below;
        if yields && !*seen {
            *seen = true;
            self.yielded.push(cell);
        }
        yields
    }
}

impl Snapshot {
    /// Whether voxel `v` is air once the region's solid voxels are carved: a region voxel is
    /// unless `spare` holds for its material; another is when the world holds air there. A
    /// voxel in a chunk that is not resident is not.
    pub(super) fn air_after_carve(
        &self,
        reader: &mut ChunkReader<'_>,
        v: [i32; 3],
        spare: &impl Fn(MaterialId) -> bool,
    ) -> bool {
        let cell = cell_of(v);
        match self.packed.get(&CellKey(cell)) {
            Some(b) => !spare(b.get(index_of(v))),
            None => reader.brick(cell).is_ok_and(|b| b.get(index_of(v)) == AIR),
        }
    }
}
