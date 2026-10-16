//! Edit operations: a shape and a mode in, the voxels that changed (by old material) out.
//!
//! A voxel is inside a shape when its fixed-point centre is (see `shape.rs`). Every predicate
//! compares integers, so the same op on the same world changes the same voxels on every
//! machine. An op first checks that every chunk holding a voxel centre inside the shape is
//! resident and fails with the first missing coordinate (ascending order) before changing
//! anything; then it writes each chunk in place under that chunk's write lock, in ascending
//! coordinate order (ADR-0002). The store map's read lock is held from before the check until
//! after the last write, so a streamer cannot evict a chunk the op has checked (ADR-0003).

use crate::brick::Brick;
use crate::coords::index_in_brick;
use crate::coords::{BRICK_EDGE, BRICK_VOXELS, CHUNK_EDGE, ChunkCoord, VoxelCoord, brick_slot};
use crate::fixed::{Fixed3, centre};
use crate::material::{AIR, MaterialId, MaterialTable};
use crate::raycast::point_at;
use crate::shape::{Coverage, Shape};
use crate::store::{World, read, write};

/// What an op does to the voxels inside its shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    /// Every non-air voxel becomes air, except voxels of an `INDESTRUCTIBLE` material.
    Carve,
    /// Every air voxel becomes this material; non-air voxels are left alone.
    Fill(MaterialId),
    /// Every voxel of `from` becomes `to`, whatever the flags of either.
    Replace { from: MaterialId, to: MaterialId },
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

    pub fn capsule(a: Fixed3, b: Fixed3, r: i64, mode: Mode) -> Self {
        Self {
            shape: Shape::Capsule { a, b, r },
            mode,
        }
    }

    /// A capsule of radius `r` from `origin` to where the ray `origin + dir * t / 256` enters
    /// the first non-air voxel of `world`, or to `t = max_t` when nothing is hit. The end point
    /// is exact fixed point rounded down.
    pub fn tool_along_ray(
        world: &World,
        origin: Fixed3,
        dir: [i64; 3],
        max_t: i64,
        r: i64,
        mode: Mode,
    ) -> Self {
        let end = match world.raycast_exact(origin, dir, max_t) {
            Some(h) => point_at(origin, dir, h.t_num, h.t_den),
            None => point_at(origin, dir, i128::from(max_t), 1),
        };
        Self::capsule(origin, end, r, mode)
    }
}

impl Mode {
    /// The material a voxel of `old` becomes, or `None` when the mode leaves it unchanged.
    fn target(self, old: MaterialId, table: &MaterialTable) -> Option<MaterialId> {
        let new = match self {
            Mode::Carve => (old != AIR && !table.is_indestructible(old)).then_some(AIR),
            Mode::Fill(m) => (old == AIR).then_some(m),
            Mode::Replace { from, to } => (old == from).then_some(to),
        }?;
        (new != old).then_some(new)
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

/// Whether any voxel centre in `lo..=hi` (one chunk's part of the shape bounds) is inside,
/// decided brick by brick so only bricks on the surface test single voxels.
fn reaches(shape: &Shape, lo: [i64; 3], hi: [i64; 3]) -> bool {
    let be = i64::from(BRICK_EDGE);
    let blo = lo.map(|v| v.div_euclid(be));
    let bhi = hi.map(|v| v.div_euclid(be));
    for bz in blo[2]..=bhi[2] {
        for by in blo[1]..=bhi[1] {
            for bx in blo[0]..=bhi[0] {
                let b = [bx, by, bz];
                let elo = [0, 1, 2].map(|k| (b[k] * be).max(lo[k]));
                let ehi = [0, 1, 2].map(|k| (b[k] * be + be - 1).min(hi[k]));
                let (a, c) = centres(elo, ehi);
                match shape.coverage(a, c) {
                    Coverage::Outside => {}
                    Coverage::Inside => return true,
                    Coverage::Partial => {
                        for z in elo[2]..=ehi[2] {
                            for y in elo[1]..=ehi[1] {
                                for x in elo[0]..=ehi[0] {
                                    if shape.contains([x, y, z].map(centre)) {
                                        return true;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    false
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

        // Residency first: collect every chunk the shape may reach, ascending, or fail. The map
        // guard lives to the end of the function: insertion and eviction wait for the op.
        let map = read(&self.store.chunks);
        let mut chunks = Vec::new();
        {
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
                            Some(chunk) => chunks.push((coord, vlo, vhi, chunk)),
                            None if reaches(&op.shape, vlo, vhi) => {
                                return Err(EditError::NotResident(coord));
                            }
                            None => {}
                        }
                    }
                }
            }
        }

        let mut tally = Tally::default();
        let mut touched = Vec::new();
        for (coord, vlo, vhi, chunk) in chunks {
            let mut chunk = write(chunk);
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
                chunk.mark_edited();
                touched.push(coord);
            }
        }

        drop(map);
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
