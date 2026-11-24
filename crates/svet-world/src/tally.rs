//! The count-and-bound accumulator an op's result is built from: the changed voxels counted
//! by material, and the inclusive bounds of the changed voxels.

use crate::coords::{VoxelCoord, to_i32};
use crate::material::MaterialId;

/// Per-op counts by material and bounds. `counts` is indexed by material id, sized to the
/// material table once per op and grown to reach an id beyond it; `seen` lists each id the
/// first time it is counted, so [`Tally::changed_from`] walks only the ids that occur.
pub(crate) struct Tally {
    counts: Vec<u32>,
    seen: Vec<MaterialId>,
    lo: [i64; 3],
    hi: [i64; 3],
    any: bool,
}

impl Tally {
    pub(crate) fn new(rows: usize) -> Self {
        Self {
            counts: vec![0; rows],
            seen: Vec::with_capacity(rows.min(16)),
            lo: [0; 3],
            hi: [0; 3],
            any: false,
        }
    }

    /// Counts `n` voxels of material `m`. An id beyond the table's rows (a voxel the table
    /// does not describe) grows `counts` to reach it. An id counted with `n == 0` is still
    /// listed, once, in [`Tally::changed_from`].
    #[inline]
    pub(crate) fn count(&mut self, m: MaterialId, n: u32) {
        let i = usize::from(m);
        if i >= self.counts.len() {
            self.counts.resize(i + 1, 0);
        }
        if self.counts[i] == 0 && !self.seen.contains(&m) {
            self.seen.push(m);
        }
        self.counts[i] += n;
    }

    /// Widens the bounds to hold `lo..=hi`.
    #[inline]
    pub(crate) fn bound(&mut self, lo: [i64; 3], hi: [i64; 3]) {
        if self.any {
            for k in 0..3 {
                self.lo[k] = self.lo[k].min(lo[k]);
                self.hi[k] = self.hi[k].max(hi[k]);
            }
        } else {
            (self.lo, self.hi, self.any) = (lo, hi, true);
        }
    }

    /// The bounds, `None` when nothing was bounded.
    pub(crate) fn bounds(&self) -> Option<([i64; 3], [i64; 3])> {
        self.any.then_some((self.lo, self.hi))
    }

    /// The bounds as voxel coordinates, `None` when nothing was bounded.
    pub(crate) fn aabb(&self) -> Option<(VoxelCoord, VoxelCoord)> {
        self.bounds()
            .map(|(lo, hi)| (VoxelCoord(to_i32(lo)), VoxelCoord(to_i32(hi))))
    }

    /// The counted ids ascending, each with its count.
    pub(crate) fn changed_from(mut self) -> Vec<(MaterialId, u32)> {
        self.seen.sort_unstable();
        self.seen
            .iter()
            .map(|&m| (m, self.counts[usize::from(m)]))
            .collect()
    }
}
