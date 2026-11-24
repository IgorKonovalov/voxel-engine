//! The collapse ops (ADR-0008): a rigid vertical drop of a region of cells, its removal with
//! rubble, and a pile of rubble on it. Each takes the region as cells (from
//! [`crate::find_unsupported`] or the caller's own choice) and acts on every solid voxel of
//! those cells' bricks, except that the removal spares voxels of an `INDESTRUCTIBLE` material
//! as `Mode::Carve` does.
//!
//! A drop may crush what it falls through (ADR-0016): with a yield threshold, a solid voxel
//! outside the region whose cell holds fewer solid voxels than the threshold, and whose
//! material is not `INDESTRUCTIBLE`, is passed through as if it were air and destroyed.
//!
//! An op first checks that every chunk holding a region cell is resident, then snapshots the
//! region's bricks and scans the columns under them, failing with `NotResident` before anything
//! changes when a scan reaches a chunk that is not resident. It then composes every brick it
//! changes in memory and writes them, one chunk write lock at a time in ascending chunk order,
//! under the store map's read lock held for the whole op (the access model of `World::apply`,
//! ADR-0002). Every count and coordinate is an integer, so the same op on the same world gives
//! the same voxels and result everywhere.

mod compose;
mod drop;
mod pile;
mod remove;
mod rubble;
mod snapshot;

use crate::brick::Brick;
use crate::coarse::CellCoord;
use crate::coords::{BRICK_EDGE, ChunkCoord, VoxelCoord, index_in_brick};
use crate::edit::EditResult;
use crate::material::{AIR, MaterialId};
use crate::packed::bounds_of;
use crate::tally::Tally;

#[cfg(doc)]
use crate::store::World;

/// What [`World::drop_region`] and [`World::drop_region_through`] did.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct DropResult {
    /// The region's solid voxels by material, ascending by material, when it moved; empty when
    /// `dropped == 0`.
    pub moved: Vec<(MaterialId, u32)>,
    /// How far every voxel moved down, in voxels.
    pub dropped: i32,
    /// Voxels outside the region that the fall destroyed, by material, ascending by material:
    /// the yielding voxels in the span each region run swept. Empty when `dropped == 0` or the
    /// yield threshold is 0.
    pub crushed: Vec<(MaterialId, u32)>,
    /// Chunks holding a vacated, a landed or a crushed voxel, ascending; empty when nothing
    /// moved.
    pub touched: Vec<ChunkCoord>,
    /// Inclusive bounds of the vacated, the landed and the crushed voxels; `None` when nothing
    /// moved.
    pub aabb: Option<(VoxelCoord, VoxelCoord)>,
}

impl DropResult {
    /// The result in the edit-result vocabulary, for [`crate::CoarseGrid::update`]:
    /// `changed_from` is `moved` and `crushed` merged, `touched` and `aabb` are the same.
    pub fn edit_result(&self) -> EditResult {
        let mut changed = Tally::new(0);
        for &(m, n) in self.moved.iter().chain(&self.crushed) {
            changed.count(m, n);
        }
        EditResult {
            changed_from: changed.changed_from(),
            touched: self.touched.clone(),
            aabb: self.aabb,
        }
    }
}

/// What [`World::remove_region`] did.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct RemoveResult {
    /// The region's solid voxels by material, ascending by material: all carved.
    pub removed: Vec<(MaterialId, u32)>,
    /// Rubble voxels written.
    pub placed: u32,
    /// Chunks holding a carved or a rubble voxel, ascending.
    pub touched: Vec<ChunkCoord>,
    /// Inclusive bounds of the carved and the rubble voxels; `None` when nothing changed.
    pub aabb: Option<(VoxelCoord, VoxelCoord)>,
}

impl RemoveResult {
    /// The result in the edit-result vocabulary, for [`crate::CoarseGrid::update`]:
    /// `changed_from` is `removed`, `touched` and `aabb` are the same.
    pub fn edit_result(&self) -> EditResult {
        EditResult {
            changed_from: self.removed.clone(),
            touched: self.touched.clone(),
            aabb: self.aabb,
        }
    }
}

/// What [`World::pile_rubble`] did.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PileResult {
    /// Rubble voxels written.
    pub placed: u32,
    /// Chunks holding a rubble voxel, ascending.
    pub touched: Vec<ChunkCoord>,
    /// Inclusive bounds of the rubble voxels; `None` when none was placed.
    pub aabb: Option<(VoxelCoord, VoxelCoord)>,
}

impl PileResult {
    /// The result in the edit-result vocabulary, for [`crate::CoarseGrid::update`]: every
    /// rubble voxel was air, so `changed_from` is `[(AIR, placed)]` (empty when none was
    /// placed); `touched` and `aabb` are the same.
    pub fn edit_result(&self) -> EditResult {
        EditResult {
            changed_from: if self.placed > 0 {
                vec![(AIR, self.placed)]
            } else {
                Vec::new()
            },
            touched: self.touched.clone(),
            aabb: self.aabb,
        }
    }
}

const BE: i32 = BRICK_EDGE;

/// The cell holding voxel `v`.
fn cell_of(v: [i32; 3]) -> CellCoord {
    CellCoord(v.map(|c| c.div_euclid(BE)))
}

/// The index in its brick of voxel `v`.
fn index_of(v: [i32; 3]) -> usize {
    index_in_brick(v.map(|c| c.rem_euclid(BE)))
}

/// The bit of voxel `v` in its brick's layer words: bit `x + 8 y` of word `z`.
fn bit_of(v: [i32; 3]) -> (usize, u64) {
    let i = index_of(v);
    (i / 64, 1 << (i % 64))
}

/// The 64 voxel columns of `b`, index `x + 8 y`, each bottom first: one decode of the brick.
fn brick_columns(b: &Brick) -> [[MaterialId; 8]; 64] {
    match b {
        Brick::Uniform(m) => [[*m; 8]; 64],
        Brick::Full(p) => {
            let mut v = [AIR; crate::coords::BRICK_VOXELS];
            p.decode(&mut v);
            std::array::from_fn(|xy| std::array::from_fn(|z| v[64 * z + xy]))
        }
    }
}

/// Calls `f(material, n)` for each non-air material of `b` under `sel`, ascending, `n > 0`.
fn counts_under(b: &Brick, sel: &[u64; 8], f: &mut impl FnMut(MaterialId, u32)) {
    match b {
        Brick::Uniform(AIR) => {}
        Brick::Uniform(m) => {
            let n = sel.iter().map(|w| w.count_ones()).sum();
            if n > 0 {
                f(*m, n);
            }
        }
        Brick::Full(p) => p.counts_under(sel, f),
    }
}

/// A voxel coordinate widened for [`Tally::bound`].
fn wide(v: [i32; 3]) -> [i64; 3] {
    v.map(i64::from)
}

/// The brick-local inclusive bounds of the set bits of `mask`, or `None`.
fn local_bounds(mask: &[u64; 8]) -> Option<([i32; 3], [i32; 3])> {
    bounds_of(mask).map(|(lo, hi)| (lo.map(i32::from), hi.map(i32::from)))
}
