//! The removal: the region carved, and a share of it piled as rubble.

use std::collections::BTreeSet;

use super::compose::{Compose, Composed};
use super::rubble::RubbleStage;
use super::snapshot::Snapshot;
use super::{BE, RemoveResult, counts_under, local_bounds, wide};
use crate::brick::{Brick, RawBrick};
use crate::coarse::CellCoord;
use crate::edit::EditError;
use crate::material::{AIR, MaterialId};
use crate::store::{ChunkReader, World, read};
use crate::tally::Tally;

impl World {
    /// Carves every solid voxel of `cells`' bricks and piles a fraction of the carved volume as
    /// rubble of material `rubble`, column by column, on the first solid voxel below each.
    ///
    /// As `Mode::Carve` does, the carve spares voxels of an `INDESTRUCTIBLE` material: they stay
    /// in place, are not in `removed` and do not count toward a column's `n`, and they are solid
    /// to the rubble scan and the rubble run. The voxel columns of the footprint are taken in a
    /// fixed order (cell stacks ascending by `(cx, cy)`, then local `y`, then local `x`), and one
    /// integer accumulator runs across them: a column with `n` carved voxels adds
    /// `n * fraction.0`, takes `h` = the accumulator's whole multiples of `fraction.1`, and
    /// keeps the remainder for the next column (`h` is 0 when `fraction.1 == 0` or `rubble` is
    /// air). The region so piles `floor(N * fraction.0 / fraction.1)` voxels for `N` carved,
    /// less only what finds no ground or no air. The scan goes down from the column's lowest
    /// carved voxel through air to the first solid voxel within `max_depth`; `h` rubble voxels
    /// are written upward from it, fewer where the air above it (after the carve) runs out. A
    /// column with no solid voxel within `max_depth` loses its `h`; the remainder still
    /// carries. Fails with `NotResident`, changing nothing, when a cell's chunk, or a
    /// voxel a scan reaches, is not resident, and with `OutOfRange { what: RegionCells }`
    /// when `cells` holds more than the world's `region_cells_max`.
    pub fn remove_region(
        &self,
        cells: &[CellCoord],
        rubble: MaterialId,
        fraction: (u32, u32),
        max_depth: i32,
    ) -> Result<RemoveResult, EditError> {
        self.remove_region_with::<Brick>(cells, rubble, fraction, max_depth)
    }

    /// [`World::remove_region`] composed over decoded voxels, written id by id: the reference
    /// the native path is tested against.
    #[doc(hidden)]
    pub fn remove_region_decoded(
        &self,
        cells: &[CellCoord],
        rubble: MaterialId,
        fraction: (u32, u32),
        max_depth: i32,
    ) -> Result<RemoveResult, EditError> {
        self.remove_region_with::<RawBrick>(cells, rubble, fraction, max_depth)
    }

    fn remove_region_with<B: Composed>(
        &self,
        cells: &[CellCoord],
        rubble: MaterialId,
        fraction: (u32, u32),
        max_depth: i32,
    ) -> Result<RemoveResult, EditError> {
        self.limits().check_region(cells.len())?;
        let map = read(&self.store.chunks);
        let snap = Snapshot::take(&map, cells)?;
        let columns = snap.columns();
        if columns.is_empty() {
            return Ok(RemoveResult::default());
        }
        let spare = |m: MaterialId| self.materials.is_indestructible(m);
        let carved = |m: MaterialId| m != AIR && !spare(m);

        // Scan every column first, so a non-resident chunk fails the op before any write.
        let mut reader = ChunkReader::new(&map);
        let mut stage = RubbleStage::new(rubble, fraction, max_depth);
        for (xy, parts) in &columns {
            let mut n: u64 = 0;
            let mut lowest: Option<i32> = None;
            for (cz, col) in parts {
                for (lz, &m) in col.iter().enumerate() {
                    if carved(m) {
                        n += 1;
                        lowest.get_or_insert(cz * BE + lz as i32);
                    }
                }
            }
            let Some(lowest) = lowest else { continue };
            stage.column(&snap, &mut reader, *xy, n, lowest, &spare)?;
        }

        let mut compose = Compose::<B>::new(&snap, &spare);
        // `removed` also holds the bounds of the carved and the rubble voxels.
        let mut removed = Tally::new(self.materials.len());
        let mut touched = BTreeSet::new();
        for (&[cx, cy], zs) in &snap.stacks {
            for &cz in zs {
                let c = CellCoord([cx, cy, cz]);
                let min = c.min_voxel().0;
                let (lo, hi) = match snap.brick(c) {
                    Brick::Uniform(m) if !carved(*m) => continue,
                    Brick::Uniform(m) => {
                        removed.count(*m, 512);
                        ([0; 3], [BE - 1; 3])
                    }
                    full @ Brick::Full(_) => {
                        // The carved voxels: the solid ones less those the carve spares.
                        let mut kept = full.clone();
                        kept.keep_only(spare);
                        let kept = kept.solid_bits();
                        let mut sel = full.solid_bits();
                        for (w, k) in sel.iter_mut().zip(kept) {
                            *w &= !k;
                        }
                        counts_under(full, &sel, &mut |m, n| removed.count(m, n));
                        let Some(b) = local_bounds(&sel) else {
                            continue;
                        };
                        b
                    }
                };
                removed.bound(
                    wide([0, 1, 2].map(|k| min[k] + lo[k])),
                    wide([0, 1, 2].map(|k| min[k] + hi[k])),
                );
                touched.insert(c.split().0);
            }
        }
        let placed = stage.write(
            &snap,
            &mut reader,
            &mut compose,
            &spare,
            &mut touched,
            &mut removed,
        );
        drop(reader);
        compose.commit(self, &map);
        drop(map);
        let aabb = removed.aabb();
        Ok(RemoveResult {
            removed: removed.changed_from(),
            placed,
            touched: touched.into_iter().collect(),
            aabb,
        })
    }
}
