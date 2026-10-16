//! Edit operations: a shape and a mode in, the voxels that changed (by old material) out.
//!
//! A voxel is inside a shape when its fixed-point centre is. Every predicate compares
//! integers, so the same op on the same world changes the same voxels on every machine.
//! An op first checks that every chunk the shape may touch is resident and fails with the
//! first missing coordinate (ascending order) before changing anything; then it writes each
//! chunk in place under that chunk's write lock, in ascending coordinate order (ADR-0002).

use crate::brick::Brick;
use crate::coords::index_in_brick;
use crate::coords::{BRICK_EDGE, BRICK_VOXELS, CHUNK_EDGE, ChunkCoord, VoxelCoord, brick_slot};
use crate::fixed::{Fixed3, centre, first_centre_at_or_above, last_centre_at_or_below};
use crate::material::{AIR, MaterialId, MaterialTable};
use crate::store::{World, read, write};

/// A region of space in fixed point.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Shape {
    /// Centres within distance `r` of `c`, boundary included.
    Sphere { c: Fixed3, r: i64 },
    /// Centres in the half-open box `[min, max)`.
    Box_ { min: Fixed3, max: Fixed3 },
}

/// What an op does to the voxels inside its shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    /// Every non-air voxel becomes air.
    Carve,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EditOp {
    pub shape: Shape,
    pub mode: Mode,
}

/// What an op changed.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct EditResult {
    /// Voxels that changed, keyed by their old material, ascending by material.
    pub changed_from: Vec<(MaterialId, u32)>,
    /// Chunks holding at least one changed voxel, ascending.
    pub touched: Vec<ChunkCoord>,
    /// Inclusive bounds of the changed voxels; `None` when nothing changed.
    pub aabb: Option<(VoxelCoord, VoxelCoord)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EditError {
    /// The shape reaches this chunk and it is not resident. Nothing was changed.
    NotResident(ChunkCoord),
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotResident(c) => write!(f, "chunk {:?} is not resident", c.0),
        }
    }
}

impl std::error::Error for EditError {}

impl EditOp {
    pub fn sphere(c: Fixed3, r: i64, mode: Mode) -> Self {
        Self {
            shape: Shape::Sphere { c, r },
            mode,
        }
    }

    pub fn box_(min: Fixed3, max: Fixed3, mode: Mode) -> Self {
        Self {
            shape: Shape::Box_ { min, max },
            mode,
        }
    }
}

/// How a box of voxel centres relates to a shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Coverage {
    /// No centre in the box is inside.
    Outside,
    /// Every centre in the box is inside.
    Inside,
    /// Unknown; test each voxel.
    Partial,
}

impl Shape {
    /// Inclusive voxel bounds of every voxel whose centre may lie inside; `None` when empty.
    fn voxel_bounds(&self) -> Option<([i64; 3], [i64; 3])> {
        let (lo, hi) = match *self {
            Shape::Sphere { c, r } => {
                if r < 0 {
                    return None;
                }
                (
                    c.0.map(|v| first_centre_at_or_above(v - r)),
                    c.0.map(|v| last_centre_at_or_below(v + r)),
                )
            }
            Shape::Box_ { min, max } => (
                min.0.map(first_centre_at_or_above),
                max.0.map(|v| last_centre_at_or_below(v - 1)),
            ),
        };
        (0..3).all(|k| lo[k] <= hi[k]).then_some((lo, hi))
    }

    /// Whether the fixed-point point `p` is inside.
    fn contains(&self, p: [i64; 3]) -> bool {
        match *self {
            Shape::Sphere { c, r } => dist2(p, c.0) <= r * r,
            Shape::Box_ { min, max } => (0..3).all(|k| p[k] >= min.0[k] && p[k] < max.0[k]),
        }
    }

    /// Relation to the centres of voxels `lo..=hi` (fixed-point centre box `clo..=chi`).
    fn coverage(&self, clo: [i64; 3], chi: [i64; 3]) -> Coverage {
        match *self {
            Shape::Sphere { c, r } => {
                let mut near = 0;
                let mut far = 0;
                for k in 0..3 {
                    let n = c.0[k].clamp(clo[k], chi[k]) - c.0[k];
                    let f = (clo[k] - c.0[k]).abs().max((chi[k] - c.0[k]).abs());
                    near += n * n;
                    far += f * f;
                }
                if near > r * r {
                    Coverage::Outside
                } else if far <= r * r {
                    Coverage::Inside
                } else {
                    Coverage::Partial
                }
            }
            Shape::Box_ { min, max } => {
                if (0..3).any(|k| chi[k] < min.0[k] || clo[k] >= max.0[k]) {
                    Coverage::Outside
                } else if (0..3).all(|k| clo[k] >= min.0[k] && chi[k] < max.0[k]) {
                    Coverage::Inside
                } else {
                    Coverage::Partial
                }
            }
        }
    }
}

fn dist2(a: [i64; 3], b: [i64; 3]) -> i64 {
    (0..3).map(|k| (a[k] - b[k]) * (a[k] - b[k])).sum()
}

impl Mode {
    /// The material a voxel of `old` becomes, or `None` when the mode leaves it alone.
    fn target(self, old: MaterialId, _table: &MaterialTable) -> Option<MaterialId> {
        match self {
            Mode::Carve => (old != AIR).then_some(AIR),
        }
    }
}

/// Per-op accumulation of counts and bounds.
#[derive(Default)]
struct Tally {
    counts: Vec<(MaterialId, u32)>,
    lo: [i64; 3],
    hi: [i64; 3],
    any: bool,
}

impl Tally {
    fn add(&mut self, m: MaterialId, n: u32, lo: [i64; 3], hi: [i64; 3]) {
        match self.counts.iter_mut().find(|(k, _)| *k == m) {
            Some((_, c)) => *c += n,
            None => self.counts.push((m, n)),
        }
        if self.any {
            for k in 0..3 {
                self.lo[k] = self.lo[k].min(lo[k]);
                self.hi[k] = self.hi[k].max(hi[k]);
            }
        } else {
            (self.lo, self.hi, self.any) = (lo, hi, true);
        }
    }
}

fn centres(lo: [i64; 3], hi: [i64; 3]) -> ([i64; 3], [i64; 3]) {
    (lo.map(centre), hi.map(centre))
}

fn to_i32(v: [i64; 3]) -> [i32; 3] {
    v.map(|c| c.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32)
}

impl World {
    /// Applies `op` and reports what changed. Fails, changing nothing, when the shape reaches a
    /// chunk that is not resident.
    pub fn apply(&self, op: &EditOp) -> Result<EditResult, EditError> {
        let Some((lo, hi)) = op.shape.voxel_bounds() else {
            return Ok(EditResult::default());
        };
        let edge = i64::from(CHUNK_EDGE);
        let clo = lo.map(|v| v.div_euclid(edge));
        let chi = hi.map(|v| v.div_euclid(edge));

        // Residency first: collect every chunk the shape may reach, ascending, or fail.
        let mut chunks = Vec::new();
        {
            let map = read(&self.store.chunks);
            for x in clo[0]..=chi[0] {
                for y in clo[1]..=chi[1] {
                    for z in clo[2]..=chi[2] {
                        let c = [x, y, z];
                        let vlo = c.map(|v| v * edge);
                        let vlo = [0, 1, 2].map(|k| vlo[k].max(lo[k]));
                        let vhi = [0, 1, 2].map(|k| (c[k] * edge + edge - 1).min(hi[k]));
                        let (a, b) = centres(vlo, vhi);
                        if op.shape.coverage(a, b) == Coverage::Outside {
                            continue;
                        }
                        let coord = ChunkCoord(to_i32(c));
                        match map.get(&coord) {
                            Some(chunk) => chunks.push((coord, vlo, vhi, chunk.clone())),
                            None => return Err(EditError::NotResident(coord)),
                        }
                    }
                }
            }
        }

        let mut tally = Tally::default();
        let mut touched = Vec::new();
        for (coord, vlo, vhi, chunk) in chunks {
            let mut chunk = write(&chunk);
            let base = coord.min_voxel().0.map(i64::from);
            let be = i64::from(BRICK_EDGE);
            let blo = [0, 1, 2].map(|k| (vlo[k] - base[k]) / be);
            let bhi = [0, 1, 2].map(|k| (vhi[k] - base[k]) / be);
            let mut chunk_changed = false;
            for bz in blo[2]..=bhi[2] {
                for by in blo[1]..=bhi[1] {
                    for bx in blo[0]..=bhi[0] {
                        let b = [bx, by, bz];
                        let bmin = [0, 1, 2].map(|k| base[k] + b[k] * be);
                        let elo = [0, 1, 2].map(|k| bmin[k].max(vlo[k]));
                        let ehi = [0, 1, 2].map(|k| (bmin[k] + be - 1).min(vhi[k]));
                        let (a, c) = centres(elo, ehi);
                        let cov = op.shape.coverage(a, c);
                        if cov == Coverage::Outside {
                            continue;
                        }
                        let slot = brick_slot(b.map(|v| v as i32));
                        let brick = chunk.brick_mut(slot);
                        if edit_brick(brick, op, &self.materials, cov, bmin, elo, ehi, &mut tally) {
                            chunk.mark_dirty(slot);
                            chunk_changed = true;
                        }
                    }
                }
            }
            if chunk_changed {
                touched.push(coord);
            }
        }

        tally.counts.sort_unstable();
        Ok(EditResult {
            changed_from: tally.counts,
            touched,
            aabb: tally
                .any
                .then(|| (VoxelCoord(to_i32(tally.lo)), VoxelCoord(to_i32(tally.hi)))),
        })
    }
}

/// Applies `op` to the voxels `elo..=ehi` of the brick whose minimum voxel is `bmin`.
/// Returns whether any voxel changed. Leaves the brick collapsed if it became uniform.
#[allow(
    clippy::too_many_arguments,
    reason = "the inner loop of apply; a struct would only rename these"
)]
fn edit_brick(
    brick: &mut Brick,
    op: &EditOp,
    table: &MaterialTable,
    cov: Coverage,
    bmin: [i64; 3],
    elo: [i64; 3],
    ehi: [i64; 3],
    tally: &mut Tally,
) -> bool {
    let whole = (0..3).all(|k| elo[k] == bmin[k] && ehi[k] == bmin[k] + i64::from(BRICK_EDGE) - 1);
    if let Brick::Uniform(m) = *brick {
        let Some(n) = op.mode.target(m, table) else {
            return false;
        };
        if whole && cov == Coverage::Inside {
            *brick = Brick::Uniform(n);
            tally.add(m, BRICK_VOXELS as u32, elo, ehi);
            return true;
        }
    }

    let mut changed = false;
    for z in elo[2]..=ehi[2] {
        for y in elo[1]..=ehi[1] {
            for x in elo[0]..=ehi[0] {
                let v = [x, y, z];
                if cov == Coverage::Partial && !op.shape.contains(v.map(centre)) {
                    continue;
                }
                let i = index_in_brick([0, 1, 2].map(|k| (v[k] - bmin[k]) as i32));
                let old = brick.get(i);
                let Some(new) = op.mode.target(old, table) else {
                    continue;
                };
                if new == old {
                    continue;
                }
                brick.expand()[i] = new;
                tally.add(old, 1, v, v);
                changed = true;
            }
        }
    }
    if changed {
        brick.collapse();
    }
    changed
}
