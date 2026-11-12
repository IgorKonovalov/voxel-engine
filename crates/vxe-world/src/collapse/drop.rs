//! The drop: a rigid move of the region down by the smallest gap under it, crushing what
//! yields (ADR-0016).

use std::collections::{BTreeMap, BTreeSet};

use super::compose::{Compose, Composed, check_landings};
use super::snapshot::{Snapshot, Yield};
use super::{BE, DropResult, bit_of, counts_under, index_of, local_bounds, wide};
use crate::brick::{Brick, RawBrick};
use crate::coarse::CellCoord;
use crate::coords::VoxelCoord;
use crate::edit::EditError;
use crate::material::{AIR, MaterialId};
use crate::store::{ChunkReader, World, read};
use crate::tally::Tally;

/// The `z` of the lowest voxel of each run of solid region voxels in a column's `parts`,
/// bottom first: a solid voxel whose voxel below is not a solid region voxel (or lies in a
/// cell outside the region).
fn run_bottoms(parts: &[(i32, [MaterialId; 8])]) -> impl Iterator<Item = i32> + '_ {
    let mut below_solid = false;
    let mut prev_z: Option<i32> = None;
    parts.iter().flat_map(move |(cz, col)| {
        if prev_z != Some(cz - 1) {
            below_solid = false;
        }
        prev_z = Some(*cz);
        // At most 4 runs start in 8 voxels; the array has room for 8.
        let mut out = [0; 8];
        let mut n = 0;
        for (lz, &m) in col.iter().enumerate() {
            let solid = m != AIR;
            if solid && !below_solid {
                out[n] = cz * BE + lz as i32;
                n += 1;
            }
            below_solid = solid;
        }
        out.into_iter().take(n)
    })
}

impl World {
    /// Moves the solid voxels of `cells`' bricks straight down, as one rigid piece, by the
    /// smallest air gap under the region's columns, at most `max_drop`:
    /// [`World::drop_region_through`] with a yield threshold of 0, which crushes nothing.
    ///
    /// Per voxel column of the footprint, the gap is the run of air below the column's lowest
    /// region voxel down to the first solid voxel, capped at `max_drop`. A column whose region
    /// voxels are split by a solid voxel outside the region is also scanned below each upper
    /// run, the region's own voxels counting as free (they move too), so no landed voxel ever
    /// overwrites a solid voxel outside the region. Every solid voxel moves, an `INDESTRUCTIBLE`
    /// one included: a rigid move destroys nothing. Fails with `NotResident`, changing nothing,
    /// when a cell's chunk, or a voxel a scan reaches, is not resident.
    ///
    /// The op moves every solid voxel of the named cells' bricks, including voxels that were
    /// not part of the region the caller had in mind. A caller dropping a piece tick by tick
    /// names its cells again on every call: after a drop by a multiple of `BRICK_EDGE` the
    /// piece's voxels lie in its cells shifted down by `dropped / BRICK_EDGE`; after any other
    /// drop they straddle two cell layers, and the caller re-derives the cells (for example from
    /// a [`crate::CoarseGrid`] updated with [`DropResult::edit_result`]).
    ///
    /// Refused with `OutOfRange { what: RegionCells }`, changing nothing, when `cells` holds
    /// more than the world's `region_cells_max`.
    pub fn drop_region(&self, cells: &[CellCoord], max_drop: i32) -> Result<DropResult, EditError> {
        self.drop_region_with::<Brick>(cells, max_drop, 0)
    }

    /// [`World::drop_region`] composed over decoded voxels, written id by id: the reference the
    /// native path is tested against.
    #[doc(hidden)]
    pub fn drop_region_decoded(
        &self,
        cells: &[CellCoord],
        max_drop: i32,
    ) -> Result<DropResult, EditError> {
        self.drop_region_with::<RawBrick>(cells, max_drop, 0)
    }

    /// [`World::drop_region`] through the voxels that yield (ADR-0016): a solid voxel outside
    /// the region yields when its cell held fewer than `yield_below` solid voxels when the op
    /// started and its material is not `INDESTRUCTIBLE`.
    ///
    /// The gap scan passes through a yielding voxel as if it were air, and stops at a voxel of
    /// a cell at or above the threshold, at an `INDESTRUCTIBLE` voxel and at a chunk that is not
    /// resident, as [`World::drop_region`] does. Once the drop `dropped` is known, every
    /// yielding voxel in the span a region run swept (in each column of the footprint, from
    /// each run's lowest voxel down by `dropped`) is crushed: it becomes air, or the landed
    /// region overwrites it. The crushed voxels are [`DropResult::crushed`], and `touched` and
    /// `aabb` include them. A drop crushes at most `dropped` voxels per run.
    ///
    /// `yield_below == 0` yields nothing and is exactly [`World::drop_region`]. With
    /// `yield_below` equal to the support query's `solid_min`, the drop agrees with the query:
    /// a cell the query calls load-free does not stop it.
    pub fn drop_region_through(
        &self,
        cells: &[CellCoord],
        max_drop: i32,
        yield_below: u16,
    ) -> Result<DropResult, EditError> {
        self.drop_region_with::<Brick>(cells, max_drop, yield_below)
    }

    /// [`World::drop_region_through`] composed over decoded voxels, written id by id: the
    /// reference the native path is tested against.
    #[doc(hidden)]
    pub fn drop_region_through_decoded(
        &self,
        cells: &[CellCoord],
        max_drop: i32,
        yield_below: u16,
    ) -> Result<DropResult, EditError> {
        self.drop_region_with::<RawBrick>(cells, max_drop, yield_below)
    }

    fn drop_region_with<B: Composed>(
        &self,
        cells: &[CellCoord],
        max_drop: i32,
        yield_below: u16,
    ) -> Result<DropResult, EditError> {
        self.limits().check_region(cells.len())?;
        let map = read(&self.store.chunks);
        let snap = Snapshot::take(&map, cells)?;
        let columns = snap.columns();
        if columns.is_empty() || max_drop <= 0 {
            return Ok(DropResult::default());
        }

        let mut reader = ChunkReader::new(&map);
        let mut yields = (yield_below > 0).then(|| Yield::new(yield_below, &self.materials));
        let mut dropped = max_drop;
        for (xy, parts) in &columns {
            // The bottom of each run of solid region voxels, as `run_bottoms` yields them.
            let mut below_solid = false;
            let mut prev_z: Option<i32> = None;
            for (cz, col) in parts {
                if prev_z != Some(cz - 1) {
                    below_solid = false;
                }
                for (lz, &m) in col.iter().enumerate() {
                    let solid = m != AIR;
                    if solid && !below_solid {
                        let z = cz * BE + lz as i32;
                        let gap = snap
                            .gap_below(&mut reader, *xy, z, max_drop, &|_| false, yields.as_mut())
                            .map_err(EditError::NotResident)?;
                        dropped = dropped.min(gap);
                    }
                    below_solid = solid;
                }
                prev_z = Some(*cz);
            }
        }
        if dropped == 0 {
            return Ok(DropResult::default());
        }

        check_landings(&snap, &mut reader, dropped).map_err(EditError::NotResident)?;

        // `moved` also holds the bounds of the vacated, landed and crushed voxels.
        let mut moved = Tally::new(self.materials.len());
        let mut crushed = Tally::new(self.materials.len());
        let mut compose = Compose::<B>::new(&snap, &|_| false);
        let mut touched = BTreeSet::new();
        if let Some(y) = yields.as_mut().filter(|y| !y.yielded.is_empty()) {
            // Every solid voxel outside the region in a swept span yielded when the gap scan
            // passed it, so it lies in a cell of `yielded`, and each is crushed. Per column, the
            // spans of its runs (which may overlap) are merged, and each meets the yielded
            // cells of its cell stack.
            y.yielded.sort_unstable();
            let mut by_cell: BTreeMap<CellCoord, [u64; 8]> = BTreeMap::new();
            let mut spans: Vec<(i32, i32)> = Vec::new();
            let mut stack: Option<[i32; 2]> = None;
            let mut stack_cells: Vec<CellCoord> = Vec::new();
            for (xy, parts) in &columns {
                let [x, y_] = *xy;
                let here = [x.div_euclid(BE), y_.div_euclid(BE)];
                if stack != Some(here) {
                    stack = Some(here);
                    stack_cells.clear();
                    stack_cells.extend(
                        y.yielded
                            .iter()
                            .filter(|c| [c.0[0], c.0[1]] == here)
                            .copied(),
                    );
                }
                if stack_cells.is_empty() {
                    continue;
                }
                spans.clear();
                for z in run_bottoms(parts) {
                    let (lo, hi) = (z - dropped, z - 1);
                    match spans.last_mut() {
                        Some(last) if lo <= last.1 + 1 => last.1 = last.1.max(hi),
                        _ => spans.push((lo, hi)),
                    }
                }
                for &cell in &stack_cells {
                    let (clo, chi) = (cell.0[2] * BE, cell.0[2] * BE + BE - 1);
                    for &(lo, hi) in &spans {
                        let (from, to) = (lo.max(clo), hi.min(chi));
                        if from > to {
                            continue;
                        }
                        let brick = reader.brick(cell).expect("scanned resident");
                        for z in from..=to {
                            let v = [x, y_, z];
                            let m = brick.get(index_of(v));
                            if m != AIR {
                                crushed.count(m, 1);
                                moved.bound(wide(v), wide(v));
                                touched.insert(VoxelCoord(v).chunk());
                                let (w, bit) = bit_of(v);
                                by_cell.entry(cell).or_default()[w] |= bit;
                            }
                        }
                    }
                }
            }
            for (cell, shape) in by_cell {
                compose.brick(&mut reader, cell).clear(&shape);
            }
        }
        for (&[cx, cy], zs) in &snap.stacks {
            for &cz in zs {
                let c = CellCoord([cx, cy, cz]);
                let min = c.min_voxel().0;
                match snap.brick(c) {
                    Brick::Uniform(AIR) => {}
                    Brick::Uniform(m) => {
                        moved.count(*m, 512);
                        let hi = min.map(|v| v + BE - 1);
                        moved.bound(wide([min[0], min[1], min[2] - dropped]), wide(hi));
                        touched.insert(c.split().0);
                        touched.insert(VoxelCoord([min[0], min[1], min[2] - dropped]).chunk());
                        touched.insert(VoxelCoord([min[0], min[1], hi[2] - dropped]).chunk());
                        if dropped % BE == 0 {
                            let to = CellCoord([cx, cy, cz - dropped / BE]);
                            let b = compose.brick(&mut reader, to);
                            if b.is_air() {
                                b.set_uniform(*m);
                                continue;
                            }
                        }
                        compose.place_dropped(&mut reader, c, &Brick::Uniform(*m), dropped);
                    }
                    full @ Brick::Full(_) => {
                        let solid = full.solid_bits();
                        counts_under(full, &solid, &mut |m, n| moved.count(m, n));
                        if let Some((lo, hi)) = local_bounds(&solid) {
                            compose.place_dropped(&mut reader, c, full, dropped);
                            let lo = [0, 1, 2].map(|k| min[k] + lo[k]);
                            let hi = [0, 1, 2].map(|k| min[k] + hi[k]);
                            moved.bound(wide([lo[0], lo[1], lo[2] - dropped]), wide(hi));
                            touched.insert(c.split().0);
                            touched.insert(VoxelCoord([lo[0], lo[1], lo[2] - dropped]).chunk());
                            touched.insert(VoxelCoord([lo[0], lo[1], hi[2] - dropped]).chunk());
                        }
                    }
                }
            }
        }
        drop(reader);
        compose.commit(self, &map);
        drop(map);
        let aabb = moved.aabb();
        Ok(DropResult {
            moved: moved.changed_from(),
            dropped,
            crushed: crushed.changed_from(),
            touched: touched.into_iter().collect(),
            aabb,
        })
    }
}
