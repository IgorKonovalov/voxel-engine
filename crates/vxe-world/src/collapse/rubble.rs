//! The rubble stage of the removal and the pile: the share each column takes, its ground, and
//! the upward write.

use std::collections::{BTreeMap, BTreeSet};

use super::compose::{Compose, Composed};
use super::snapshot::Snapshot;
use super::{bit_of, cell_of, wide};
use crate::coarse::CellCoord;
use crate::coords::{ChunkCoord, VoxelCoord};
use crate::edit::EditError;
use crate::material::{AIR, MaterialId};
use crate::store::ChunkReader;
use crate::tally::Tally;

#[cfg(doc)]
use crate::store::World;

/// The rubble stage of [`World::remove_region`] and [`World::pile_rubble`]: one integer
/// accumulator across the footprint's columns in their fixed order, a scan down each column
/// to its ground, and the upward write of each column's share.
pub(super) struct RubbleStage {
    rubble: MaterialId,
    /// `(numerator, denominator)` of the share; nothing piles when the denominator is 0 or
    /// `rubble` is air.
    fraction: (u64, u64),
    max_depth: i32,
    /// Weight times the numerator not yet piled, below the denominator between columns.
    acc: u64,
    /// Per column with a share and ground: the lowest air voxel above the ground, and the
    /// share.
    piles: Vec<([i32; 3], i32)>,
}

impl RubbleStage {
    pub(super) fn new(rubble: MaterialId, fraction: (u32, u32), max_depth: i32) -> Self {
        Self {
            rubble,
            fraction: (u64::from(fraction.0), u64::from(fraction.1)),
            max_depth,
            acc: 0,
            piles: Vec::new(),
        }
    }

    /// The next column `xy` of weight `n`: it adds `n` times the numerator to the accumulator
    /// and takes `h`, the accumulator's whole multiples of the denominator. Its ground is the
    /// first voxel below `z` (exclusive) that is solid once the op's carve is done (a region
    /// voxel is free unless `spare` holds), within `max_depth`; a column with no ground within
    /// reach loses its `h`, and the remainder still carries.
    pub(super) fn column(
        &mut self,
        snap: &Snapshot,
        reader: &mut ChunkReader<'_>,
        xy: [i32; 2],
        n: u64,
        z: i32,
        spare: &impl Fn(MaterialId) -> bool,
    ) -> Result<(), EditError> {
        let (num, den) = self.fraction;
        let h = if den != 0 && self.rubble != AIR {
            self.acc += n * num;
            let h = self.acc / den;
            self.acc -= h * den;
            h
        } else {
            0
        };
        let gap = snap
            .gap_below(reader, xy, z, self.max_depth, spare, None)
            .map_err(EditError::NotResident)?;
        if h > 0 && gap < self.max_depth {
            let h = i32::try_from(h).unwrap_or(i32::MAX);
            self.piles.push(([xy[0], xy[1], z - gap], h));
        }
        Ok(())
    }

    /// Writes each column's share upward from its lowest air voxel, fewer where the air (after
    /// the carve) runs out, into `compose`; returns the voxels written.
    pub(super) fn write<B: Composed>(
        self,
        snap: &Snapshot,
        reader: &mut ChunkReader<'_>,
        compose: &mut Compose<B>,
        spare: &impl Fn(MaterialId) -> bool,
        touched: &mut BTreeSet<ChunkCoord>,
        bounds: &mut Tally,
    ) -> u32 {
        let mut placed: u32 = 0;
        // Each column's run, as bits of its cells' shapes: the columns are distinct and the
        // air test reads the snapshot and the world, never the composed bricks, so each
        // brick then takes its rubble in one fill.
        let mut shapes: BTreeMap<CellCoord, [u64; 8]> = BTreeMap::new();
        for ([x, y, z0], h) in self.piles {
            let mut top = None;
            for z in z0..z0.saturating_add(h) {
                if !snap.air_after_carve(reader, [x, y, z], spare) {
                    break;
                }
                let (w, bit) = bit_of([x, y, z]);
                shapes.entry(cell_of([x, y, z])).or_default()[w] |= bit;
                touched.insert(VoxelCoord([x, y, z]).chunk());
                placed += 1;
                top = Some(z);
            }
            if let Some(top) = top {
                bounds.bound(wide([x, y, z0]), wide([x, y, top]));
            }
        }
        for (cell, shape) in shapes {
            compose.brick(reader, cell).fill(&shape, self.rubble);
        }
        placed
    }
}
