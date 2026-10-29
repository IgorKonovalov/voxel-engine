//! Edit operations: a shape and a mode in, the voxels that changed (by old material) out.
//!
//! A voxel is inside a shape when its fixed-point centre is (see `shape.rs`). Every predicate
//! compares integers, so the same op on the same world changes the same voxels on every
//! machine. An op first checks that every chunk holding a voxel centre inside the shape is
//! resident and fails with the first missing coordinate (ascending order) before changing
//! anything; then it writes each chunk in place under that chunk's write lock, in ascending
//! coordinate order (ADR-0002). The store map's read lock is held from before the check until
//! after the last write, so a streamer cannot evict a chunk the op has checked (ADR-0003).

use crate::brick::{Brick, RawBrick, SpareArrays, Voxels};
use crate::coords::{
    BRICK_EDGE, BRICK_VOXELS, CHUNK_EDGE, CHUNK_EDGE_BRICKS, ChunkCoord, VoxelCoord, brick_slot,
};
use crate::fixed::{Fixed3, centre};
use crate::limits::Field;
use crate::material::{AIR, MaterialFlags, MaterialId, MaterialTable};
use crate::packed::{KernelMode, PackedStats, RowSpans, box_words, shape_words};
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
    /// A shape, ray, box query or region parameter lies outside the world's
    /// [`WorldLimits`](crate::WorldLimits).
    /// Nothing was changed.
    OutOfRange { what: Field },
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotResident(c) => write!(f, "chunk {:?} is not resident", c.0),
            Self::UnknownPrefab(id) => write!(f, "prefab {id:#018x} is not registered"),
            Self::OutOfRange { what } => write!(f, "{what} out of range"),
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

    /// The planar kernel's view of the table.
    fn kernel_mode(&self) -> KernelMode<'_> {
        match *self {
            Self::Carve { ref carves, rows } => KernelMode::Carve { carves, rows },
            Self::Fill(m) => KernelMode::Fill(m),
            Self::Replace { from, to } => KernelMode::Replace { from, to },
        }
    }

    /// [`write_rows`] under this table's target.
    fn write_rows(
        &self,
        v: &mut Voxels,
        bmin: [i64; 3],
        spans: &RowSpans,
        tally: &mut Tally,
    ) -> bool {
        match *self {
            Self::Carve { ref carves, rows } => {
                write_rows(v, bmin, spans, CarveBits { carves, rows }, tally)
            }
            Self::Fill(m) => write_rows(v, bmin, spans, FillAir(m), tally),
            Self::Replace { from, to } => {
                write_rows(v, bmin, spans, ReplaceOne { from, to }, tally)
            }
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

/// A voxel or chunk coordinate of a checked shape as `i32`. Within the limits every such
/// coordinate fits (`±2^30` fixed units plus a radius of as much is `±2^23` voxels), so the
/// clamp never moves one.
fn to_i32(v: [i64; 3]) -> [i32; 3] {
    v.map(|c| {
        debug_assert!(i32::try_from(c).is_ok(), "coordinate {c} beyond i32");
        c.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
    })
}

/// The op path's stages, in the order an op meets them. A stage's time runs from the previous
/// stamp to the stamp that names it, so every nanosecond between an op's first and last stamp
/// lands in exactly one stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stage {
    /// The shape check, the bounds, the residency scan and the per-op tables.
    Scan,
    /// One chunk's write lock.
    Lock,
    /// The op's row spans (`OpRows::new`; a shape other than the sphere solves its rows in
    /// `Clip`, at their first use).
    Spans,
    /// A brick row's coverage tests, and a brick's edit box and slot.
    Coverage,
    /// A brick's shape words from the op's row spans clipped to the brick.
    Clip,
    /// The empty-brick test on the shape words, and the capture when one is asked for.
    Words,
    /// The planar kernel, with its per-material counts into the tally.
    Kernel,
    /// The brick moved to its canonical class.
    Canon,
    /// The changed bounds, the dirty marks and the touched list.
    Tally,
    /// The result: the sorted `changed_from`, the aabb, the stats.
    Result,
}

/// A stamp sink threaded through the op path. The default build passes [`NoStages`], whose
/// stamps are empty and inline away, so the op path reads no clock.
pub(crate) trait Stamps {
    fn begin(&mut self) {}
    fn at(&mut self, _stage: Stage) {}
}

/// The default build's sink: no clock, no state.
pub(crate) struct NoStages;

impl Stamps for NoStages {}

/// The summed wall time per [`Stage`] over every op applied through [`World::apply_staged`]
/// (feature `edit-stages`, for the edit bench only; the default build has no clock in this
/// crate). `ns[s]` is indexed by the stage's position in [`EditStages::NAMES`]; `stamps`
/// counts the clock reads, one per stage interval, so a caller can subtract the clock's own
/// cost.
#[cfg(feature = "edit-stages")]
#[derive(Clone, Debug, Default)]
pub struct EditStages {
    pub ns: [u64; 10],
    pub stamps: [u64; 10],
    last: Option<std::time::Instant>,
}

#[cfg(feature = "edit-stages")]
impl EditStages {
    pub const NAMES: [&'static str; 10] = [
        "scan", "lock", "spans", "coverage", "clip", "words", "kernel", "canon", "tally", "result",
    ];
}

#[cfg(feature = "edit-stages")]
impl Stamps for EditStages {
    #[allow(
        clippy::disallowed_methods,
        reason = "feature edit-stages only: the bench's stage timer, never in the default build"
    )]
    fn begin(&mut self) {
        self.last = Some(std::time::Instant::now());
    }

    #[allow(
        clippy::disallowed_methods,
        reason = "feature edit-stages only: the bench's stage timer, never in the default build"
    )]
    fn at(&mut self, stage: Stage) {
        let now = std::time::Instant::now();
        if let Some(last) = self.last {
            let i = stage as usize;
            self.ns[i] += u64::try_from((now - last).as_nanos()).unwrap_or(u64::MAX);
            self.stamps[i] += 1;
        }
        self.last = Some(now);
    }
}

impl World {
    /// Applies `op` and reports what changed. Fails, changing nothing, when a parameter of the
    /// shape lies outside the world's [`WorldLimits`](crate::WorldLimits)
    /// ([`EditError::OutOfRange`]: coordinates, radius, capsule length, or voxel bounds wider
    /// than `edit_axis_max`) or the shape reaches a chunk that is not resident.
    pub fn apply(&self, op: &EditOp) -> Result<EditResult, EditError> {
        self.apply_with(op, &mut None, &mut NoStages, &mut 0)
    }

    /// [`World::apply`] and the row-span entries the op allocated: [`SLAB_ROWS`] (64) per
    /// brick row of its bounds that it reached, all of the bounds' for a sphere. For the
    /// span-allocation test.
    #[doc(hidden)]
    pub fn apply_counting_spans(&self, op: &EditOp) -> Result<(EditResult, usize), EditError> {
        let mut spans = 0;
        let r = self.apply_with(op, &mut None, &mut NoStages, &mut spans)?;
        Ok((r, spans))
    }

    /// [`World::apply`] with each stage's wall time added to `stages`. For the edit bench.
    #[cfg(feature = "edit-stages")]
    #[doc(hidden)]
    pub fn apply_staged(
        &self,
        op: &EditOp,
        stages: &mut EditStages,
    ) -> Result<EditResult, EditError> {
        self.apply_with(op, &mut None, stages, &mut 0)
    }

    /// [`World::apply`], and every brick-edit that reaches the row write pushed onto `out` in
    /// op order: the brick before the op and its row spans. For the kernel bench.
    #[doc(hidden)]
    pub fn apply_capturing(
        &self,
        op: &EditOp,
        out: &mut Vec<CapturedBrickEdit>,
    ) -> Result<EditResult, EditError> {
        self.apply_with(op, &mut Some(out), &mut NoStages, &mut 0)
    }

    fn apply_with<S: Stamps>(
        &self,
        op: &EditOp,
        capture: &mut Option<&mut Vec<CapturedBrickEdit>>,
        st: &mut S,
        spans: &mut usize,
    ) -> Result<EditResult, EditError> {
        st.begin();
        op.shape.check(&self.limits())?;
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

        let be = i64::from(BRICK_EDGE);
        let targets = TargetTable::new(op.mode, &self.materials);
        let mut tally = Tally::new(self.materials.len());
        let mut stats = PackedStats::default();
        let mut touched = Vec::new();
        st.at(Stage::Scan);
        let mut op_rows = OpRows::new(&op.shape, lo, hi);
        st.at(Stage::Spans);
        for (coord, vlo, vhi, chunk) in chunks {
            let mut chunk = write(chunk);
            st.at(Stage::Lock);
            let base = coord.min_voxel().0.map(i64::from);
            let blo = [0, 1, 2].map(|k| (vlo[k] - base[k]) / be);
            let bhi = [0, 1, 2].map(|k| (vhi[k] - base[k]) / be);
            let mut chunk_changed = false;
            let mut covs = [Coverage::Outside; CHUNK_EDGE_BRICKS as usize];
            for bz in blo[2]..=bhi[2] {
                for by in blo[1]..=bhi[1] {
                    // One brick row (fixed `by`, `bz`): every brick's coverage first, then
                    // the bricks it reaches.
                    for bx in blo[0]..=bhi[0] {
                        let bmin = [0, 1, 2].map(|k| base[k] + [bx, by, bz][k] * be);
                        let elo = [0, 1, 2].map(|k| bmin[k].max(vlo[k]));
                        let ehi = [0, 1, 2].map(|k| (bmin[k] + be - 1).min(vhi[k]));
                        let (a, c) = centres(elo, ehi);
                        let cov = op.shape.coverage(a, c);
                        covs[bx as usize] = cov;
                    }
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
                        let cx = BrickCx {
                            targets: &targets,
                            tally: &mut tally,
                            stats: &mut stats,
                            capture,
                        };
                        if edit_brick(brick, edit, &mut op_rows, cx, st) {
                            chunk.mark_dirty(slot);
                            chunk_changed = true;
                        }
                        st.at(Stage::Tally);
                    }
                }
            }
            if chunk_changed {
                chunk.mark_edited();
                self.store.mark_dirty(coord);
                touched.push(coord);
            }
            drop(chunk);
            st.at(Stage::Tally);
        }

        drop(map);
        *spans = op_rows.entries;
        self.add_packed_stats(&stats);
        let aabb = tally
            .any
            .then(|| (VoxelCoord(to_i32(tally.lo)), VoxelCoord(to_i32(tally.hi))));
        let result = EditResult {
            changed_from: tally.changed_from(),
            touched,
            aabb,
        };
        st.at(Stage::Result);
        Ok(result)
    }
}

/// The per-op state an [`edit_brick`] call writes through.
struct BrickCx<'a, 'c> {
    targets: &'a TargetTable,
    tally: &'a mut Tally,
    stats: &'a mut PackedStats,
    capture: &'a mut Option<&'c mut Vec<CapturedBrickEdit>>,
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
/// Returns whether any voxel changed. The brick is decoded, written row by row and encoded
/// again in canonical form. Stamps `Coverage` through `Canon` on `st`; the caller stamps the
/// tally that follows.
fn edit_brick<S: Stamps>(
    brick: &mut Brick,
    edit: BrickEdit,
    rows: &mut OpRows<'_>,
    cx: BrickCx<'_, '_>,
    st: &mut S,
) -> bool {
    let BrickCx {
        targets,
        tally,
        stats,
        capture,
    } = cx;
    st.at(Stage::Coverage);
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
            st.at(Stage::Kernel);
            tally.bound(elo, ehi);
            return true;
        }
    }

    // The edit's voxels of this brick as eight shape words: the whole edit box when the brick
    // is `Inside`, else the brick row's spans clipped to the brick's `x` extent. Either way
    // no bit lies outside the edit box `elo..=ehi`.
    let shape = if cov == Coverage::Inside {
        let lo = [0, 1, 2].map(|k| (elo[k] - bmin[k]) as u8);
        let hi = [0, 1, 2].map(|k| (ehi[k] - bmin[k]) as u8);
        box_words(lo, hi)
    } else {
        rows.words(bmin, elo, ehi)
    };
    st.at(Stage::Clip);
    if shape.iter().all(|&w| w == 0) {
        return false;
    }
    if let Some(out) = capture {
        out.push(CapturedBrickEdit {
            before: brick.clone(),
            spans: spans_of(&shape),
        });
    }
    st.at(Stage::Words);
    let bounds = brick.edit_planar_uncanonical(
        targets.kernel_mode(),
        &shape,
        &mut |m, n| tally.count(m, n),
        stats,
    );
    st.at(Stage::Kernel);
    brick.canonicalize(stats);
    st.at(Stage::Canon);
    match bounds {
        Some((lo, hi)) => {
            tally.bound(
                [0, 1, 2].map(|k| bmin[k] + i64::from(lo[k])),
                [0, 1, 2].map(|k| bmin[k] + i64::from(hi[k])),
            );
            true
        }
        None => false,
    }
}

/// One brick-edit on the row-write path, as [`World::apply_capturing`] saw it: the brick
/// before the op and the x span of each of its 64 rows (row `y + 8 z`, empty when
/// `x0 > x1`).
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapturedBrickEdit {
    pub before: Brick,
    pub spans: RowSpans,
}

/// What a writer counted: `changed_from` (ascending by material) and the brick-local bounds
/// of the changed voxels, `None` when nothing changed.
#[doc(hidden)]
pub type WriterResult = (Vec<(MaterialId, u32)>, Option<([i64; 3], [i64; 3])>);

/// The row write on raw ids, the reference the planar kernel is measured against:
/// `write_rows` on a [`RawBrick`] (a `Uniform` brick expands into a pooled, touched array),
/// then the collapse check. For the kernel bench.
#[doc(hidden)]
pub struct RowWriter {
    targets: TargetTable,
    tally: Tally,
    spare: SpareArrays,
}

impl RowWriter {
    pub fn new(mode: Mode, table: &MaterialTable) -> Self {
        Self {
            targets: TargetTable::new(mode, table),
            tally: Tally::new(table.len()),
            spare: Vec::new(),
        }
    }

    /// Pools `n` arrays, written so their pages are touched, as a warm world's pool is.
    pub fn reserve(&mut self, n: usize) {
        self.spare
            .extend((0..n).map(|_| Box::new([MaterialId::MAX; BRICK_VOXELS])));
    }

    /// Writes `spans` into `brick`; returns whether a voxel changed.
    pub fn write(&mut self, brick: &mut RawBrick, spans: &RowSpans) -> bool {
        if let RawBrick::Uniform(m) = *brick
            && self.targets.target(m).is_none()
        {
            return false;
        }
        let v = brick.expand(&mut self.spare);
        let changed = self.targets.write_rows(v, [0; 3], spans, &mut self.tally);
        if changed {
            brick.collapse(&mut self.spare);
        }
        changed
    }

    pub fn finish(self) -> WriterResult {
        finish(self.tally)
    }
}

/// The planar kernel of `packed.rs` on a [`Brick`]: the spans folded into eight shape
/// words, then the word operations. For the kernel bench.
#[doc(hidden)]
pub struct PlanarWriter {
    targets: TargetTable,
    tally: Tally,
    stats: PackedStats,
}

impl PlanarWriter {
    pub fn new(mode: Mode, table: &MaterialTable) -> Self {
        Self {
            targets: TargetTable::new(mode, table),
            tally: Tally::new(table.len()),
            stats: PackedStats::default(),
        }
    }

    /// Writes `spans` into `brick`; returns whether a voxel changed.
    pub fn write(&mut self, brick: &mut Brick, spans: &RowSpans) -> bool {
        let shape = shape_words(spans);
        let tally = &mut self.tally;
        match brick.edit_planar(
            self.targets.kernel_mode(),
            &shape,
            &mut |m, n| tally.count(m, n),
            &mut self.stats,
        ) {
            Some((lo, hi)) => {
                tally.bound(lo.map(i64::from), hi.map(i64::from));
                true
            }
            None => false,
        }
    }

    pub fn finish(self) -> WriterResult {
        finish(self.tally)
    }
}

fn finish(tally: Tally) -> WriterResult {
    let bounds = tally.any.then_some((tally.lo, tally.hi));
    (tally.changed_from(), bounds)
}

/// Rows per slab: one brick row's `8 x 8` voxel rows `(y, z)`.
const SLAB_ROWS: usize = (BRICK_EDGE * BRICK_EDGE) as usize;

/// The span of every voxel row `(y, z)` of one op's voxel bounds `lo..=hi`, over the bounds'
/// whole `x` extent, each solved at most once per op: a sphere's all walked up front by
/// [`sphere_row_spans`], another shape's solved by [`Shape::row_span`] at its first use.
/// Rows are kept in slabs, one per brick row (fixed brick `y`, `z`) of the bounds, each of
/// [`SLAB_ROWS`] entries with row `(y, z)` at `y mod 8 + 8 (z mod 8)`, empty as `(1, 0)`. A
/// slab is allocated when one of its rows is first asked for (the sphere's all up front), so
/// an op holds spans only for the brick rows it reaches: a long diagonal capsule's bounds
/// span a million rows, and it reaches a few thousand. A row's span is the same whichever
/// chunk or brick asks: the inside centres of a row are one interval within the bounds, and
/// each brick clips it to its own `x` extent.
struct OpRows<'a> {
    shape: &'a Shape,
    lo: [i64; 3],
    hi: [i64; 3],
    /// The bounds' first brick `y` and `z`, and their brick-row count along `y`.
    by0: i64,
    bz0: i64,
    nby: i64,
    /// Per brick row `(by - by0) + nby (bz - bz0)`, its slab once allocated.
    slabs: Vec<Option<Box<Slab>>>,
    /// Span entries allocated so far, [`SLAB_ROWS`] per slab.
    entries: usize,
}

/// One brick row's spans, and which are solved: bit `i` of `solved` for entry `i`; all set
/// for a sphere's slabs, which are walked when allocated.
struct Slab {
    span: [(i64, i64); SLAB_ROWS],
    solved: u64,
}

impl<'a> OpRows<'a> {
    fn new(shape: &'a Shape, lo: [i64; 3], hi: [i64; 3]) -> Self {
        let be = i64::from(BRICK_EDGE);
        let (by0, bz0) = (lo[1].div_euclid(be), lo[2].div_euclid(be));
        let nby = hi[1].div_euclid(be) - by0 + 1;
        let nbz = hi[2].div_euclid(be) - bz0 + 1;
        let mut rows = Self {
            shape,
            lo,
            hi,
            by0,
            bz0,
            nby,
            slabs: (0..nby * nbz).map(|_| None).collect(),
            entries: 0,
        };
        if let Shape::Sphere { .. } = *shape {
            for bz in bz0..bz0 + nbz {
                for by in by0..by0 + nby {
                    rows.slab(by, bz);
                }
            }
        }
        rows
    }

    /// The slab of brick row `(by, bz)`, a brick row of the bounds, allocated on first use;
    /// a sphere's is walked then, row by row within the bounds.
    fn slab(&mut self, by: i64, bz: i64) -> &mut Slab {
        let i = ((by - self.by0) + self.nby * (bz - self.bz0)) as usize;
        if self.slabs[i].is_none() {
            let mut slab = Box::new(Slab {
                span: [(1, 0); SLAB_ROWS],
                solved: 0,
            });
            if let Shape::Sphere { c, r } = *self.shape {
                let be = i64::from(BRICK_EDGE);
                let (ylo, yhi) = (
                    (by * be).max(self.lo[1]),
                    (by * be + be - 1).min(self.hi[1]),
                );
                let (zlo, zhi) = (
                    (bz * be).max(self.lo[2]),
                    (bz * be + be - 1).min(self.hi[2]),
                );
                let ny = yhi - ylo + 1;
                let mut walked = [(1, 0); SLAB_ROWS];
                let n = (ny * (zhi - zlo + 1)) as usize;
                sphere_row_spans(
                    c,
                    r,
                    (ylo, yhi),
                    (zlo, zhi),
                    (self.lo[0], self.hi[0]),
                    &mut walked[..n],
                );
                for z in zlo..=zhi {
                    for y in ylo..=yhi {
                        slab.span[slab_index(y, z)] = walked[((y - ylo) + ny * (z - zlo)) as usize];
                    }
                }
                slab.solved = u64::MAX;
            }
            self.entries += SLAB_ROWS;
            self.slabs[i] = Some(slab);
        }
        self.slabs[i].as_mut().expect("allocated above")
    }

    /// The span of row `(y, z)`, a row of the bounds.
    fn span(&mut self, y: i64, z: i64) -> (i64, i64) {
        let be = i64::from(BRICK_EDGE);
        let (shape, x_lo, x_hi) = (self.shape, self.lo[0], self.hi[0]);
        let slab = self.slab(y.div_euclid(be), z.div_euclid(be));
        let i = slab_index(y, z);
        if slab.solved >> i & 1 == 0 {
            slab.solved |= 1 << i;
            slab.span[i] = shape.row_span(y, z, x_lo, x_hi).unwrap_or((1, 0));
        }
        slab.span[i]
    }

    /// The shape words of the brick whose minimum voxel is `bmin` over its edit box
    /// `elo..=ehi` (rows of the bounds): each row's span clipped to the brick's `x` extent,
    /// bit `x + 8 y` (brick-local) of word `z`; rows outside the box are empty. Branch-free per
    /// row: the clipped span is the bits below `x1 + 1 - bmin.x` less the bits below
    /// `x0 - bmin.x`, both clamped to `0..=8`; an empty span `(1, 0)` has equal bounds and sets
    /// nothing.
    fn words(&mut self, bmin: [i64; 3], elo: [i64; 3], ehi: [i64; 3]) -> [u64; 8] {
        const ONES: [u64; 9] = [0, 1, 3, 7, 15, 31, 63, 127, 255];
        let be = i64::from(BRICK_EDGE);
        let mut w = [0u64; 8];
        for z in elo[2]..=ehi[2] {
            let mut layer = 0u64;
            for y in elo[1]..=ehi[1] {
                let (a, b) = self.span(y, z);
                let lo = (a - bmin[0]).clamp(0, be) as usize;
                let hi = (b + 1 - bmin[0]).clamp(0, be) as usize;
                layer |= (ONES[hi] & !ONES[lo]) << (8 * (y - bmin[1]));
            }
            w[(z - bmin[2]) as usize] = layer;
        }
        w
    }
}

/// Row `(y, z)`'s entry in its slab: `y mod 8 + 8 (z mod 8)`.
fn slab_index(y: i64, z: i64) -> usize {
    let be = i64::from(BRICK_EDGE);
    (y.rem_euclid(be) + be * z.rem_euclid(be)) as usize
}

/// The row spans of eight shape words whose rows are each one contiguous run (as every
/// shape's are): row `y + 8 z` is the run in byte `y` of word `z`, `(1, 0)` when it is empty.
fn spans_of(shape: &[u64; 8]) -> RowSpans {
    std::array::from_fn(|r| {
        let byte = (shape[r / 8] >> (8 * (r % 8))) as u8;
        if byte == 0 {
            (1, 0)
        } else {
            (byte.trailing_zeros() as u8, 7 - byte.leading_zeros() as u8)
        }
    })
}

/// Writes every row span of `spans` (brick-local, row `y + 8 z`, empty when `x0 > x1`) into
/// `brick`: per voxel, read the old value, look up its target, write it, count the old value.
/// Counts go to `tally` per run of equal old values, and the bounds once per brick, from the
/// first and last changed voxel of each row. Returns whether any voxel changed.
fn write_rows<T: Target>(
    voxels: &mut Voxels,
    bmin: [i64; 3],
    spans: &[(u8, u8); BRICK_EDGE as usize * BRICK_EDGE as usize],
    t: T,
    tally: &mut Tally,
) -> bool {
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
