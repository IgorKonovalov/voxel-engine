//! The pile: rubble spread over the region's columns with nothing carved.

use std::collections::BTreeSet;

use super::compose::{Compose, Composed};
use super::rubble::RubbleStage;
use super::snapshot::Snapshot;
use super::{BE, PileResult};
use crate::brick::Brick;
#[cfg(feature = "bench-internals")]
use crate::brick::RawBrick;
use crate::coarse::CellCoord;
use crate::edit::EditError;
use crate::material::{AIR, MaterialId};
use crate::store::{ChunkReader, World, read};
use crate::tally::Tally;

impl World {
    /// Spreads `count` rubble voxels of material `rubble` over the footprint columns of
    /// `cells`, on top of what the cells hold: [`World::remove_region`]'s rubble stage without
    /// its carve.
    ///
    /// The `C` voxel columns of the footprint that hold a solid voxel of the named cells are
    /// taken in `remove_region`'s order (cell stacks ascending by `(cx, cy)`, then local `y`,
    /// then local `x`), each of weight 1 with the fraction `(count, C)`: one integer
    /// accumulator adds `count` per column and the column takes `h` = the accumulator's whole
    /// multiples of `C`, so the columns pile `count` voxels in all, less only what finds no
    /// ground or no air. Each column scans down from the top voxel of its stack's highest named
    /// cell, through air, to the first solid voxel within `max_depth`, and writes `h` rubble
    /// voxels upward into air from the voxel above it. A column with no solid voxel within
    /// `max_depth` loses its `h`; the remainder still carries. `count == 0`, or `rubble` air,
    /// places nothing. Fails with `NotResident`, changing nothing, when a cell's chunk, or a
    /// voxel a scan reaches, is not resident, and with `OutOfRange { what: RegionCells }` when
    /// `cells` holds more than the world's `region_cells_max`.
    pub fn pile_rubble(
        &self,
        cells: &[CellCoord],
        rubble: MaterialId,
        count: u32,
        max_depth: i32,
    ) -> Result<PileResult, EditError> {
        self.pile_rubble_with::<Brick>(cells, rubble, count, max_depth)
    }

    /// [`World::pile_rubble`] composed over decoded voxels, written id by id: the reference
    /// the native path is tested against. Feature `bench-internals`.
    #[cfg(feature = "bench-internals")]
    #[doc(hidden)]
    pub fn pile_rubble_decoded(
        &self,
        cells: &[CellCoord],
        rubble: MaterialId,
        count: u32,
        max_depth: i32,
    ) -> Result<PileResult, EditError> {
        self.pile_rubble_with::<RawBrick>(cells, rubble, count, max_depth)
    }

    fn pile_rubble_with<B: Composed>(
        &self,
        cells: &[CellCoord],
        rubble: MaterialId,
        count: u32,
        max_depth: i32,
    ) -> Result<PileResult, EditError> {
        self.limits().check_region(cells.len())?;
        let map = read(&self.store.chunks);
        let snap = Snapshot::take(&map, cells)?;
        let columns = snap.columns();
        if columns.is_empty() || count == 0 || rubble == AIR {
            return Ok(PileResult::default());
        }
        // Nothing is carved: every solid voxel of the named cells stays.
        let spare = |m: MaterialId| m != AIR;
        let c = u32::try_from(columns.len()).expect("at most region_cells_max * 64 columns");

        let mut reader = ChunkReader::new(&map);
        let mut stage = RubbleStage::new(rubble, (count, c), max_depth);
        for (xy, parts) in &columns {
            let (top_cell, _) = parts.last().expect("a column crosses a named cell");
            stage.column(&snap, &mut reader, *xy, 1, (top_cell + 1) * BE, &spare)?;
        }

        let mut compose = Compose::<B>::new(&snap, &spare);
        let mut bounds = Tally::new(0);
        let mut touched = BTreeSet::new();
        let placed = stage.write(
            &snap,
            &mut reader,
            &mut compose,
            &spare,
            &mut touched,
            &mut bounds,
        );
        drop(reader);
        compose.commit(self, &map);
        drop(map);
        Ok(PileResult {
            placed,
            touched: touched.into_iter().collect(),
            aabb: bounds.aabb(),
        })
    }
}
