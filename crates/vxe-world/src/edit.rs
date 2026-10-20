//! Edit operations: a shape and a mode in, the voxels that changed (by old material) out.
//!
//! A voxel is inside a shape when its fixed-point centre is (see `shape.rs`). Every predicate
//! compares integers, so the same op on the same world changes the same voxels on every
//! machine. An op first checks that every chunk holding a voxel centre inside the shape is
//! resident and fails with the first missing coordinate (ascending order) before changing
//! anything; then it writes each chunk in place under that chunk's write lock, in ascending
//! coordinate order (ADR-0002). The store map's read lock is held from before the check until
//! after the last write, so a streamer cannot evict a chunk the op has checked (ADR-0003).

use crate::brick::{Brick, SpareArrays};
use crate::coords::{
    BRICK_EDGE, BRICK_VOXELS, CHUNK_EDGE, CHUNK_EDGE_BRICKS, ChunkCoord, VoxelCoord, brick_slot,
};
use crate::fixed::{Fixed3, centre};
use crate::material::{AIR, MaterialFlags, MaterialId, MaterialTable};
use crate::raycast::point_at;
use crate::shape::{Coverage, Shape, sphere_row_spans};
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
    /// A stamp names a prefab id that is not registered where it is applied (ADR-0014).
    /// Nothing was changed.
    UnknownPrefab(u64),
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotResident(c) => write!(f, "chunk {:?} is not resident", c.0),
            Self::UnknownPrefab(id) => write!(f, "prefab {id:#018x} is not registered"),
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

/// What an op does to each old material, resolved once per op from the mode and the
/// material table, so the voxel loop reads a bit or compares one id instead of calling into
/// the table.
#[derive(Clone, Debug, PartialEq, Eq)]
enum TargetTable {
    /// Bit `m` of `carves` (word `m / 64`, bit `m % 64`) is set when row `m` exists, is not
    /// air and is not indestructible. An id with no row has no flags, so `Carve` turns it to
    /// air as well; that case is decided outside the bits.
    Carve { carves: Vec<u64>, rows: usize },
    /// Air becomes this material.
    Fill(MaterialId),
    /// `from` becomes `to`.
    Replace { from: MaterialId, to: MaterialId },
}

impl TargetTable {
    fn new(mode: Mode, table: &MaterialTable) -> Self {
        match mode {
            Mode::Carve => Self::carve(table),
            Mode::Fill(m) => Self::Fill(m),
            Mode::Replace { from, to } => Self::Replace { from, to },
        }
    }

    fn carve(table: &MaterialTable) -> Self {
        let rows = table.len();
        let mut carves = vec![0u64; rows.div_ceil(64)];
        for (m, row) in table.rows().iter().enumerate() {
            if m != usize::from(AIR) && !row.flags.contains(MaterialFlags::INDESTRUCTIBLE) {
                carves[m / 64] |= 1 << (m % 64);
            }
        }
        Self::Carve { carves, rows }
    }

    /// Bit `m` of a `Carve` table; clear for an id beyond the table's rows and for the other
    /// variants.
    #[cfg(test)]
    fn carve_bit(&self, m: MaterialId) -> bool {
        match self {
            Self::Carve { carves, .. } => {
                let m = usize::from(m);
                carves.get(m / 64).is_some_and(|w| w >> (m % 64) & 1 != 0)
            }
            _ => false,
        }
    }

    /// The material a voxel of `old` becomes, or `None` when the op leaves it unchanged.
    #[inline]
    fn target(&self, old: MaterialId) -> Option<MaterialId> {
        match *self {
            Self::Carve { ref carves, rows } => CarveBits { carves, rows }.target(old),
            Self::Fill(m) => FillAir(m).target(old),
            Self::Replace { from, to } => ReplaceOne { from, to }.target(old),
        }
    }
}

/// One variant of [`TargetTable`] as its own type, so the row loop is compiled once per
/// variant with the lookup inlined and no `match` per voxel.
trait Target: Copy {
    /// The material a voxel of `old` becomes, or `None` when the op leaves it unchanged.
    fn target(self, old: MaterialId) -> Option<MaterialId>;
}

#[derive(Clone, Copy)]
struct CarveBits<'a> {
    carves: &'a [u64],
    rows: usize,
}

impl Target for CarveBits<'_> {
    #[inline]
    fn target(self, old: MaterialId) -> Option<MaterialId> {
        let m = usize::from(old);
        // `rows >= 1` (row 0 is air), so an id at or beyond `rows` is not air.
        (m >= self.rows || self.carves[m / 64] >> (m % 64) & 1 != 0).then_some(AIR)
    }
}

#[derive(Clone, Copy)]
struct FillAir(MaterialId);

impl Target for FillAir {
    #[inline]
    fn target(self, old: MaterialId) -> Option<MaterialId> {
        (old == AIR && self.0 != AIR).then_some(self.0)
    }
}

#[derive(Clone, Copy)]
struct ReplaceOne {
    from: MaterialId,
    to: MaterialId,
}

impl Target for ReplaceOne {
    #[inline]
    fn target(self, old: MaterialId) -> Option<MaterialId> {
        (old == self.from && self.from != self.to).then_some(self.to)
    }
}

/// Per-op accumulation of counts and bounds. `counts` is indexed by material id and sized to
/// the material table once per op; `seen` lists each id the first time its count leaves zero.
struct Tally {
    counts: Vec<u32>,
    seen: Vec<MaterialId>,
    lo: [i64; 3],
    hi: [i64; 3],
    any: bool,
}

impl Tally {
    fn new(rows: usize) -> Self {
        Self {
            counts: vec![0; rows],
            seen: Vec::with_capacity(rows.min(16)),
            lo: [0; 3],
            hi: [0; 3],
            any: false,
        }
    }

    /// Counts `n >= 1` changed voxels of old material `m`. An id beyond the table's rows (a
    /// voxel the table does not describe) grows `counts` to reach it.
    fn count(&mut self, m: MaterialId, n: u32) {
        let i = usize::from(m);
        if i >= self.counts.len() {
            self.counts.resize(i + 1, 0);
        }
        if self.counts[i] == 0 {
            self.seen.push(m);
        }
        self.counts[i] += n;
    }

    /// Widens the bounds of the changed voxels to hold `lo..=hi`.
    fn bound(&mut self, lo: [i64; 3], hi: [i64; 3]) {
        if self.any {
            for k in 0..3 {
                self.lo[k] = self.lo[k].min(lo[k]);
                self.hi[k] = self.hi[k].max(hi[k]);
            }
        } else {
            (self.lo, self.hi, self.any) = (lo, hi, true);
        }
    }

    /// `changed_from`: the seen ids ascending, each with its count.
    fn changed_from(mut self) -> Vec<(MaterialId, u32)> {
        self.seen.sort_unstable();
        self.seen
            .iter()
            .map(|&m| (m, self.counts[usize::from(m)]))
            .collect()
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

        // One batch of pooled arrays for every brick the op may expand: at most the bricks
        // its voxel bounds overlap.
        let be = i64::from(BRICK_EDGE);
        let bound = (0..3)
            .map(|k| usize::try_from(hi[k].div_euclid(be) - lo[k].div_euclid(be) + 1).unwrap_or(0))
            .fold(1usize, usize::saturating_mul);
        let mut spare = self.brick_arrays.take(bound);
        let targets = TargetTable::new(op.mode, &self.materials);
        let mut tally = Tally::new(self.materials.len());
        let mut touched = Vec::new();
        for (coord, vlo, vhi, chunk) in chunks {
            let mut chunk = write(chunk);
            let base = coord.min_voxel().0.map(i64::from);
            let blo = [0, 1, 2].map(|k| (vlo[k] - base[k]) / be);
            let bhi = [0, 1, 2].map(|k| (vhi[k] - base[k]) / be);
            let mut chunk_changed = false;
            let mut covs = [Coverage::Outside; CHUNK_EDGE_BRICKS as usize];
            for bz in blo[2]..=bhi[2] {
                for by in blo[1]..=bhi[1] {
                    // One brick row (fixed `by`, `bz`): the bricks share their `y`, `z` voxel
                    // rows, so each row's span is solved once over the x window of the row's
                    // `Partial` bricks and clipped per brick.
                    let mut window: Option<(i64, i64)> = None;
                    for bx in blo[0]..=bhi[0] {
                        let bmin = [0, 1, 2].map(|k| base[k] + [bx, by, bz][k] * be);
                        let elo = [0, 1, 2].map(|k| bmin[k].max(vlo[k]));
                        let ehi = [0, 1, 2].map(|k| (bmin[k] + be - 1).min(vhi[k]));
                        let (a, c) = centres(elo, ehi);
                        let cov = op.shape.coverage(a, c);
                        covs[bx as usize] = cov;
                        if cov == Coverage::Partial {
                            window = Some(match window {
                                Some((x0, _)) => (x0, ehi[0]),
                                None => (elo[0], ehi[0]),
                            });
                        }
                    }
                    let ys = (
                        (base[1] + by * be).max(vlo[1]),
                        (base[1] + by * be + be - 1).min(vhi[1]),
                    );
                    let zs = (
                        (base[2] + bz * be).max(vlo[2]),
                        (base[2] + bz * be + be - 1).min(vhi[2]),
                    );
                    let mut rows = RowCache::new(&op.shape, window, ys, zs);
                    for bx in blo[0]..=bhi[0] {
                        let cov = covs[bx as usize];
                        if cov == Coverage::Outside {
                            continue;
                        }
                        let b = [bx, by, bz];
                        let bmin = [0, 1, 2].map(|k| base[k] + b[k] * be);
                        let elo = [0, 1, 2].map(|k| bmin[k].max(vlo[k]));
                        let ehi = [0, 1, 2].map(|k| (bmin[k] + be - 1).min(vhi[k]));
                        let slot = brick_slot(b.map(|v| v as i32));
                        let brick = chunk.brick_mut(slot);
                        let edit = BrickEdit {
                            cov,
                            bmin,
                            elo,
                            ehi,
                        };
                        if edit_brick(brick, &targets, edit, &mut rows, &mut spare, &mut tally) {
                            chunk.mark_dirty(slot);
                            chunk_changed = true;
                        }
                    }
                }
            }
            if chunk_changed {
                chunk.mark_edited();
                self.store.mark_dirty(coord);
                touched.push(coord);
            }
        }

        drop(map);
        self.brick_arrays.give(spare);
        let aabb = tally
            .any
            .then(|| (VoxelCoord(to_i32(tally.lo)), VoxelCoord(to_i32(tally.hi))));
        Ok(EditResult {
            changed_from: tally.changed_from(),
            touched,
            aabb,
        })
    }
}

/// Where an op meets one brick: the brick's coverage, its minimum voxel, and the voxels
/// `elo..=ehi` of it inside the op's bounds.
#[derive(Clone, Copy)]
struct BrickEdit {
    cov: Coverage,
    bmin: [i64; 3],
    elo: [i64; 3],
    ehi: [i64; 3],
}

/// Applies the op to the voxels `elo..=ehi` of the brick whose minimum voxel is `bmin`.
/// Returns whether any voxel changed. Expands into an array from `spare` and leaves the
/// brick collapsed, its array pushed onto `spare`, if it became uniform.
fn edit_brick(
    brick: &mut Brick,
    targets: &TargetTable,
    edit: BrickEdit,
    rows: &mut RowCache<'_>,
    spare: &mut SpareArrays,
    tally: &mut Tally,
) -> bool {
    let BrickEdit {
        cov,
        bmin,
        elo,
        ehi,
    } = edit;
    let whole = (0..3).all(|k| elo[k] == bmin[k] && ehi[k] == bmin[k] + i64::from(BRICK_EDGE) - 1);
    if let Brick::Uniform(m) = *brick {
        let Some(n) = targets.target(m) else {
            return false;
        };
        if whole && cov == Coverage::Inside {
            *brick = Brick::Uniform(n);
            tally.count(m, BRICK_VOXELS as u32);
            tally.bound(elo, ehi);
            return true;
        }
    }

    // The span of every row of the edit box, in brick-local `x`, clipped to the box; rows
    // with no inside centre are empty (`x0 > x1`).
    let mut spans = [(1u8, 0u8); BRICK_EDGE as usize * BRICK_EDGE as usize];
    let mut any = false;
    for z in elo[2]..=ehi[2] {
        for y in elo[1]..=ehi[1] {
            let (x0, x1) = if cov == Coverage::Inside {
                (elo[0], ehi[0])
            } else {
                let (x0, x1) = rows.span(y, z);
                (x0.max(elo[0]), x1.min(ehi[0]))
            };
            if x0 <= x1 {
                let r = local(y, bmin[1]) + BRICK_EDGE as usize * local(z, bmin[2]);
                spans[r] = ((x0 - bmin[0]) as u8, (x1 - bmin[0]) as u8);
                any = true;
            }
        }
    }
    if !any {
        return false;
    }
    let changed = match *targets {
        TargetTable::Carve {
            ref carves,
            rows: n,
        } => write_rows(
            brick,
            bmin,
            &spans,
            CarveBits { carves, rows: n },
            spare,
            tally,
        ),
        TargetTable::Fill(m) => write_rows(brick, bmin, &spans, FillAir(m), spare, tally),
        TargetTable::Replace { from, to } => {
            write_rows(brick, bmin, &spans, ReplaceOne { from, to }, spare, tally)
        }
    };
    if changed {
        brick.collapse(spare);
    }
    changed
}

/// The brick-local index (`0..8`) of voxel coordinate `v` in the brick starting at `min`.
fn local(v: i64, min: i64) -> usize {
    (v - min) as usize
}

/// The spans of one brick row's voxel rows (fixed `y`, `z`) over the x window of the brick
/// row's `Partial` bricks, kept for every brick of the brick row and indexed by `y` and `z`
/// modulo the brick edge. A sphere's are all walked up front by [`sphere_row_spans`]; another
/// shape's are solved by [`Shape::row_span`] at their first use.
struct RowCache<'a> {
    shape: &'a Shape,
    /// The x window of the brick row's `Partial` bricks; `None` when it has none.
    window: Option<(i64, i64)>,
    /// Bit `y % 8 + 8 (z % 8)` set when that row's span is solved.
    solved: u64,
    span: [(i64, i64); BRICK_EDGE as usize * BRICK_EDGE as usize],
}

impl<'a> RowCache<'a> {
    /// The cache of the brick row whose rows are `ys` by `zs` (the edit box's `y` and `z`
    /// extents, the same for every brick of the row).
    fn new(shape: &'a Shape, window: Option<(i64, i64)>, ys: (i64, i64), zs: (i64, i64)) -> Self {
        let mut cache = Self {
            shape,
            window,
            solved: 0,
            span: [(1, 0); BRICK_EDGE as usize * BRICK_EDGE as usize],
        };
        if let (&Shape::Sphere { c, r }, Some(window)) = (shape, window) {
            sphere_row_spans(c, r, ys, zs, window, &mut cache.span);
            cache.solved = u64::MAX;
        }
        cache
    }

    /// The inclusive span of row `(y, z)` within the window, empty (`x0 > x1`) when no centre
    /// is inside. Called only for rows of a `Partial` brick, so the window is set.
    fn span(&mut self, y: i64, z: i64) -> (i64, i64) {
        let be = i64::from(BRICK_EDGE);
        let r = (y.rem_euclid(be) + be * z.rem_euclid(be)) as usize;
        if self.solved >> r & 1 == 0 {
            let (lo, hi) = self.window.expect("a Partial brick sets the window");
            self.span[r] = self.shape.row_span(y, z, lo, hi).unwrap_or((1, 0));
            self.solved |= 1 << r;
        }
        self.span[r]
    }
}

/// Writes every row span of `spans` (brick-local, row `y + 8 z`, empty when `x0 > x1`) into
/// `brick`: per voxel, read the old value, look up its target, write it, count the old value.
/// Counts go to `tally` per run of equal old values, and the bounds once per brick, from the
/// first and last changed voxel of each row. Expands the brick once, into an array from
/// `spare` when it has one. Returns whether any voxel changed; does not collapse.
fn write_rows<T: Target>(
    brick: &mut Brick,
    bmin: [i64; 3],
    spans: &[(u8, u8); BRICK_EDGE as usize * BRICK_EDGE as usize],
    t: T,
    spare: &mut SpareArrays,
    tally: &mut Tally,
) -> bool {
    let voxels = brick.expand(spare);
    // The run of equal old values being counted, and the brick-local bounds of the changes.
    let (mut run_m, mut run_n) = (AIR, 0u32);
    let (mut lo, mut hi) = ([usize::MAX; 3], [0usize; 3]);
    for (r, &(x0, x1)) in spans.iter().enumerate() {
        if x0 > x1 {
            continue;
        }
        let (x0, x1) = (usize::from(x0), usize::from(x1));
        let row = r * BRICK_EDGE as usize;
        let (mut first, mut last) = (usize::MAX, 0);
        for (x, cell) in (x0..=x1).zip(&mut voxels[row + x0..=row + x1]) {
            let old = *cell;
            let Some(new) = t.target(old) else {
                continue;
            };
            *cell = new;
            if old != run_m {
                if run_n > 0 {
                    tally.count(run_m, run_n);
                }
                (run_m, run_n) = (old, 0);
            }
            run_n += 1;
            first = first.min(x);
            last = x;
        }
        if first != usize::MAX {
            let (y, z) = (r % BRICK_EDGE as usize, r / BRICK_EDGE as usize);
            lo = [lo[0].min(first), lo[1].min(y), lo[2].min(z)];
            hi = [hi[0].max(last), hi[1].max(y), hi[2].max(z)];
        }
    }
    if run_n > 0 {
        tally.count(run_m, run_n);
    }
    if lo[0] == usize::MAX {
        return false;
    }
    tally.bound(
        [0, 1, 2].map(|k| bmin[k] + lo[k] as i64),
        [0, 1, 2].map(|k| bmin[k] + hi[k] as i64),
    );
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::material::Material;

    /// Rows 0 (air), 1..=5, with 4 indestructible: the integration tests' table.
    fn test_table() -> MaterialTable {
        let mut t = MaterialTable::new();
        for i in 1..=5u16 {
            let mut m = Material::named(format!("m{i}"));
            if i == 4 {
                m.flags = MaterialFlags::INDESTRUCTIBLE;
            }
            t.push(m);
        }
        t
    }

    #[test]
    fn carve_table_sets_a_bit_per_carvable_row() {
        let t = TargetTable::carve(&test_table());
        for m in [1, 2, 3, 5] {
            assert!(t.carve_bit(m), "bit {m}");
        }
        for m in [0, 4, 6, 64, 1000, MaterialId::MAX] {
            assert!(!t.carve_bit(m), "bit {m}");
        }
    }

    #[test]
    fn targets_agree_with_the_mode_over_every_low_id() {
        let table = test_table();
        let modes = [
            Mode::Carve,
            Mode::Fill(2),
            Mode::Fill(AIR),
            Mode::Replace { from: 3, to: 5 },
            Mode::Replace { from: 4, to: 4 },
            Mode::Replace { from: 7, to: 0 },
        ];
        for mode in modes {
            let t = TargetTable::new(mode, &table);
            for old in 0..300u16 {
                let expected = match mode {
                    Mode::Carve => (old != AIR && !table.is_indestructible(old)).then_some(AIR),
                    Mode::Fill(m) => (old == AIR).then_some(m),
                    Mode::Replace { from, to } => (old == from).then_some(to),
                }
                .filter(|&n| n != old);
                assert_eq!(t.target(old), expected, "{mode:?} on {old}");
            }
        }
    }
}
