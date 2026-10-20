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

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, RwLock, RwLockReadGuard};

use crate::brick::{Brick, RawBrick, SpareArrays};
use crate::chunk::Chunk;
use crate::coarse::CellCoord;
use crate::coords::{BRICK_EDGE, ChunkCoord, VoxelCoord, index_in_brick};
use crate::edit::{EditError, EditResult};
use crate::material::{AIR, MaterialId, MaterialTable};
use crate::packed::{KernelMode, PackedStats};
use crate::store::{World, read, write};
use crate::support::{CellKey, CellMap};

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
        let mut changed = Counts::default();
        for &(m, n) in self.moved.iter().chain(&self.crushed) {
            changed.add(m, n);
        }
        EditResult {
            changed_from: changed.sorted(),
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

type ChunkMap = HashMap<ChunkCoord, Arc<RwLock<Chunk>>>;

const BE: i32 = BRICK_EDGE;

/// The cell holding voxel `v`.
fn cell_of(v: [i32; 3]) -> CellCoord {
    CellCoord(v.map(|c| c.div_euclid(BE)))
}

/// The index in its brick of voxel `v`.
fn index_of(v: [i32; 3]) -> usize {
    index_in_brick(v.map(|c| c.rem_euclid(BE)))
}

/// Brick reads over the store map with the current chunk's read lock kept between reads.
struct Reader<'m> {
    map: &'m ChunkMap,
    cur: Option<(ChunkCoord, RwLockReadGuard<'m, Chunk>)>,
}

impl<'m> Reader<'m> {
    fn new(map: &'m ChunkMap) -> Self {
        Self { map, cur: None }
    }

    /// The brick of cell `c`, or its chunk's coordinate when that is not resident.
    fn brick(&mut self, c: CellCoord) -> Result<&Brick, ChunkCoord> {
        let (chunk, slot) = c.split();
        if self.cur.as_ref().is_none_or(|(k, _)| *k != chunk) {
            self.cur = None;
            let lock = self.map.get(&chunk).ok_or(chunk)?;
            self.cur = Some((chunk, read(lock)));
        }
        let (_, guard) = self.cur.as_ref().expect("set above");
        Ok(guard.brick(slot))
    }
}

/// The region's bricks as they were when the op started.
struct Snapshot {
    /// Decoded, for the voxel scans.
    bricks: CellMap<RawBrick>,
    /// As the world holds them, for the composed writes.
    packed: CellMap<Brick>,
    /// Per cell column `(x, y)` of the footprint, ascending, the region's cells' `z`,
    /// ascending.
    stacks: BTreeMap<[i32; 2], Vec<i32>>,
}

/// A voxel column of the region's footprint: its `(x, y)` and, per region cell it crosses
/// (ascending `z`), the cell's `z` and the column's 8 voxels in the cell's brick, bottom first.
type Column = ([i32; 2], Vec<(i32, [MaterialId; 8])>);

impl Snapshot {
    /// Checks residency (the least missing chunk) and copies the bricks.
    fn take(map: &ChunkMap, cells: &[CellCoord]) -> Result<Self, EditError> {
        let mut cells = cells.to_vec();
        cells.sort_unstable();
        cells.dedup();
        if let Some(m) = cells
            .iter()
            .map(|c| c.split().0)
            .filter(|c| !map.contains_key(c))
            .min()
        {
            return Err(EditError::NotResident(m));
        }
        let mut bricks = CellMap::default();
        let mut packed = CellMap::default();
        let mut stacks: BTreeMap<[i32; 2], Vec<i32>> = BTreeMap::new();
        let mut reader = Reader::new(map);
        for &c in &cells {
            let b = reader.brick(c).expect("residency checked above").clone();
            bricks.insert(CellKey(c), RawBrick::of(&b));
            packed.insert(CellKey(c), b);
            stacks.entry([c.0[0], c.0[1]]).or_default().push(c.0[2]);
        }
        Ok(Self {
            bricks,
            packed,
            stacks,
        })
    }

    fn brick(&self, c: CellCoord) -> &RawBrick {
        &self.bricks[&CellKey(c)]
    }

    /// Every voxel column of the footprint that holds a solid region voxel, ascending by
    /// `(x, y)`.
    fn columns(&self) -> Vec<Column> {
        let mut out = Vec::new();
        for (&[cx, cy], zs) in &self.stacks {
            for ly in 0..BE {
                for lx in 0..BE {
                    let mut parts = Vec::with_capacity(zs.len());
                    let mut any = false;
                    for &cz in zs {
                        let b = self.brick(CellCoord([cx, cy, cz]));
                        let col =
                            std::array::from_fn(|lz| b.get(index_in_brick([lx, ly, lz as i32])));
                        any |= col.iter().any(|&m| m != AIR);
                        parts.push((cz, col));
                    }
                    if any {
                        out.push(([cx * BE + lx, cy * BE + ly], parts));
                    }
                }
            }
        }
        out
    }

    /// Voxels below `z` (exclusive) in column `(x, y)` that are free to fall into, counted down
    /// to the first solid voxel outside the region that does not yield, at most `max`. A
    /// region voxel is free unless `spare` holds for its material (a voxel the op leaves in
    /// place); a voxel outside the region is free when it is air or `yields` says it yields
    /// (no voxel does when `yields` is `None`). Fails with the chunk of the first non-resident
    /// voxel reached.
    fn gap_below(
        &self,
        reader: &mut Reader<'_>,
        [x, y]: [i32; 2],
        z: i32,
        max: i32,
        spare: &impl Fn(MaterialId) -> bool,
        mut yields: Option<&mut Yield<'_>>,
    ) -> Result<i32, ChunkCoord> {
        let mut gap = 0;
        while gap < max {
            let top = z - gap - 1;
            let cell = cell_of([x, y, top]);
            // Voxels from `top` down to the bottom of its cell.
            let span = (top.rem_euclid(BE) + 1).min(max - gap);
            let brick = match self.bricks.get(&CellKey(cell)) {
                Some(b) => match b {
                    RawBrick::Uniform(m) if !spare(*m) => {
                        gap += span;
                        continue;
                    }
                    RawBrick::Uniform(_) => break,
                    RawBrick::Full(v) => {
                        for k in 0..span {
                            if spare(v[index_of([x, y, top - k])]) {
                                return Ok(gap);
                            }
                            gap += 1;
                        }
                        continue;
                    }
                },
                None => reader.brick(cell)?,
            };
            let mut free = |m: MaterialId| {
                m == AIR
                    || yields
                        .as_deref_mut()
                        .is_some_and(|y| y.yields(cell, brick, m))
            };
            match brick {
                Brick::Uniform(AIR) => gap += span,
                Brick::Uniform(m) => {
                    if !free(*m) {
                        break;
                    }
                    gap += span;
                }
                full @ Brick::Full(_) => {
                    for k in 0..span {
                        if !free(full.get(index_of([x, y, top - k]))) {
                            return Ok(gap);
                        }
                        gap += 1;
                    }
                }
            }
        }
        Ok(gap)
    }
}

/// A drop's yield rule (ADR-0016): a solid voxel outside the region yields when its cell
/// holds fewer than `below` solid voxels and its material is not `INDESTRUCTIBLE`. A cell's
/// count is its brick's solid-mask popcount, read when the op first reaches the cell and
/// kept for the op, so the rule sees the world as the op found it.
struct Yield<'t> {
    below: u32,
    materials: &'t MaterialTable,
    counts: HashMap<CellCoord, u32>,
}

impl<'t> Yield<'t> {
    fn new(below: u16, materials: &'t MaterialTable) -> Self {
        Self {
            below: u32::from(below),
            materials,
            counts: HashMap::new(),
        }
    }

    /// Whether solid voxel material `m` of cell `cell`, whose brick is `brick`, yields.
    fn yields(&mut self, cell: CellCoord, brick: &Brick, m: MaterialId) -> bool {
        if self.materials.is_indestructible(m) {
            return false;
        }
        let n = *self
            .counts
            .entry(cell)
            .or_insert_with(|| brick.solid_bits().iter().map(|w| w.count_ones()).sum());
        n < self.below
    }
}

impl Snapshot {
    /// Whether voxel `v` is air once the region's solid voxels are carved: a region voxel is
    /// unless `spare` holds for its material; another is when the world holds air there. A
    /// voxel in a chunk that is not resident is not.
    fn air_after_carve(
        &self,
        reader: &mut Reader<'_>,
        v: [i32; 3],
        spare: &impl Fn(MaterialId) -> bool,
    ) -> bool {
        let cell = cell_of(v);
        match self.bricks.get(&CellKey(cell)) {
            Some(b) => !spare(b.get(index_of(v))),
            None => reader.brick(cell).is_ok_and(|b| b.get(index_of(v)) == AIR),
        }
    }
}

/// Per-material counts, ascending by material, from a small list.
#[derive(Default)]
struct Counts(Vec<(MaterialId, u32)>);

impl Counts {
    fn add(&mut self, m: MaterialId, n: u32) {
        match self.0.iter_mut().find(|(k, _)| *k == m) {
            Some((_, c)) => *c += n,
            None => self.0.push((m, n)),
        }
    }

    fn sorted(mut self) -> Vec<(MaterialId, u32)> {
        self.0.sort_unstable();
        self.0
    }
}

/// Inclusive voxel bounds grown one box at a time.
#[derive(Default)]
struct Bounds(Option<([i32; 3], [i32; 3])>);

impl Bounds {
    fn add(&mut self, lo: [i32; 3], hi: [i32; 3]) {
        self.0 = Some(match self.0 {
            None => (lo, hi),
            Some((a, b)) => (
                [0, 1, 2].map(|k| a[k].min(lo[k])),
                [0, 1, 2].map(|k| b[k].max(hi[k])),
            ),
        });
    }

    fn get(&self) -> Option<(VoxelCoord, VoxelCoord)> {
        self.0.map(|(a, b)| (VoxelCoord(a), VoxelCoord(b)))
    }
}

/// A brick as [`Compose`] holds it while an op writes it: the packed [`Brick`] itself (the
/// native path: palette merges and layer words) or decoded [`RawBrick`] ids (the reference
/// path, written voxel by voxel).
trait Composed: Sized {
    /// The world's brick.
    fn of_world(b: &Brick) -> Self;
    /// A region cell's brick holding only the voxels `spare` keeps.
    fn kept(b: &Brick, spare: &impl Fn(MaterialId) -> bool) -> Self;
    fn is_air(&self) -> bool;
    fn set_uniform(&mut self, m: MaterialId);
    /// Writes the non-air voxels of `src`'s z-layers `layers` into layers `first..`.
    fn place(&mut self, src: &Brick, layers: std::ops::Range<usize>, first: usize);
    /// Sets voxel `i` to `m`.
    fn set_voxel(&mut self, i: usize, m: MaterialId);
    /// Sets the voxels at `indices` to air.
    fn clear(&mut self, indices: &[usize]);
    fn into_brick(self) -> Brick;
}

impl Composed for Brick {
    fn of_world(b: &Brick) -> Self {
        b.clone()
    }

    fn kept(b: &Brick, spare: &impl Fn(MaterialId) -> bool) -> Self {
        let mut b = b.clone();
        b.keep_only(spare);
        b
    }

    fn is_air(&self) -> bool {
        *self == Brick::Uniform(AIR)
    }

    fn set_uniform(&mut self, m: MaterialId) {
        *self = Brick::Uniform(m);
    }

    fn place(&mut self, src: &Brick, layers: std::ops::Range<usize>, first: usize) {
        self.overlay_layers(src, layers, first);
    }

    fn set_voxel(&mut self, i: usize, m: MaterialId) {
        let old = self.get(i);
        if old == m {
            return;
        }
        if old == AIR {
            // A fill of the one voxel: a mask bit and the entry's plane bits.
            let mut shape = [0u64; 8];
            shape[i / 64] = 1 << (i % 64);
            self.edit_planar(
                KernelMode::Fill(m),
                &shape,
                &mut |_, _| {},
                &mut PackedStats::default(),
            );
        } else {
            self.modify(|v| v[i] = m);
        }
    }

    fn clear(&mut self, indices: &[usize]) {
        self.modify(|v| {
            for &i in indices {
                v[i] = AIR;
            }
        });
    }

    fn into_brick(self) -> Brick {
        self
    }
}

impl Composed for RawBrick {
    fn of_world(b: &Brick) -> Self {
        RawBrick::of(b)
    }

    fn kept(b: &Brick, spare: &impl Fn(MaterialId) -> bool) -> Self {
        let kept = |m: MaterialId| if spare(m) { m } else { AIR };
        match RawBrick::of(b) {
            RawBrick::Uniform(m) => RawBrick::Uniform(kept(m)),
            RawBrick::Full(v) => {
                let mut b = RawBrick::Full(Box::new(v.map(kept)));
                b.collapse(&mut SpareArrays::new());
                b
            }
        }
    }

    fn is_air(&self) -> bool {
        *self == RawBrick::Uniform(AIR)
    }

    fn set_uniform(&mut self, m: MaterialId) {
        *self = RawBrick::Uniform(m);
    }

    fn place(&mut self, src: &Brick, layers: std::ops::Range<usize>, first: usize) {
        const LAYER: usize = (BE * BE) as usize;
        let src = RawBrick::of(src);
        let span = layers.start * LAYER..layers.end * LAYER;
        let at = first * LAYER;
        match (&src, &*self) {
            (RawBrick::Uniform(m), RawBrick::Uniform(d)) if m == d => {}
            (RawBrick::Uniform(m), _) => {
                self.expand(&mut SpareArrays::new())[at..at + span.len()].fill(*m)
            }
            (RawBrick::Full(v), _) => {
                let from = &v[span];
                if let RawBrick::Uniform(d) = *self
                    && from.iter().all(|&m| m == AIR || m == d)
                {
                    return;
                }
                let dst = &mut self.expand(&mut SpareArrays::new())[at..at + from.len()];
                for (d, &m) in dst.iter_mut().zip(from) {
                    if m != AIR {
                        *d = m;
                    }
                }
            }
        }
    }

    fn set_voxel(&mut self, i: usize, m: MaterialId) {
        if self.get(i) != m {
            self.expand(&mut SpareArrays::new())[i] = m;
        }
    }

    fn clear(&mut self, indices: &[usize]) {
        for &i in indices {
            self.set_voxel(i, AIR);
        }
    }

    fn into_brick(self) -> Brick {
        RawBrick::into_brick(self)
    }
}

/// The bricks an op will write, composed in memory: each starts as the world's brick (or, for
/// a region cell, as its spared voxels in air, since every other solid voxel leaves) and takes
/// writes; [`Compose::commit`] stores each in canonical form.
struct Compose<B> {
    bricks: CellMap<B>,
}

impl<B: Composed> Compose<B> {
    /// Every region cell, holding only the voxels `spare` keeps.
    fn new(snap: &Snapshot, spare: &impl Fn(MaterialId) -> bool) -> Self {
        let bricks = snap
            .packed
            .iter()
            .map(|(&k, b)| (k, B::kept(b, spare)))
            .collect();
        Self { bricks }
    }

    /// The composed brick of `c`, read from the world on first use.
    fn brick(&mut self, reader: &mut Reader<'_>, c: CellCoord) -> &mut B {
        self.bricks
            .entry(CellKey(c))
            .or_insert_with(|| B::of_world(reader.brick(c).expect("scanned resident")))
    }

    /// Writes the solid voxels of `src`, the brick of region cell `c`, `dropped` voxels lower.
    /// They land in at most two bricks: layers `r..8` of `src` (with `r = dropped mod 8`) in
    /// the cell `dropped / 8` below `c`, layers `0..r` in the cell under that. Each destination
    /// brick is looked up once and takes whole z-layers.
    fn place_dropped(&mut self, reader: &mut Reader<'_>, c: CellCoord, src: &Brick, dropped: i32) {
        let (q, r) = (dropped.div_euclid(BE), dropped.rem_euclid(BE));
        let [cx, cy, cz] = c.0;
        let (r, be) = (r as usize, BE as usize);
        // (destination cell, source layers, destination layer of the first source layer)
        let upper = (CellCoord([cx, cy, cz - q]), r..be, 0);
        let lower = (CellCoord([cx, cy, cz - q - 1]), 0..r, be - r);
        for (to, layers, first) in [upper, lower] {
            if !layers.is_empty() {
                self.brick(reader, to).place(src, layers, first);
            }
        }
    }

    fn set(&mut self, reader: &mut Reader<'_>, v: [i32; 3], m: MaterialId) {
        self.brick(reader, cell_of(v)).set_voxel(index_of(v), m);
    }

    /// Writes every composed brick that differs from the world's, in canonical form, ascending
    /// by chunk.
    fn commit(self, world: &World, map: &ChunkMap) {
        let mut by_chunk: BTreeMap<ChunkCoord, Vec<(usize, Brick)>> = BTreeMap::new();
        for (CellKey(c), b) in self.bricks {
            let (chunk, slot) = c.split();
            by_chunk
                .entry(chunk)
                .or_default()
                .push((slot, b.into_brick()));
        }
        for (coord, bricks) in by_chunk {
            let mut chunk = write(&map[&coord]);
            let mut changed = false;
            for (slot, b) in bricks {
                if *chunk.brick(slot) != b {
                    chunk.set_brick(slot, b);
                    changed = true;
                }
            }
            if changed {
                chunk.mark_edited();
                world.store.mark_dirty(coord);
            }
        }
    }
}

/// The rubble stage of [`World::remove_region`] and [`World::pile_rubble`]: one integer
/// accumulator across the footprint's columns in their fixed order, a scan down each column
/// to its ground, and the upward write of each column's share.
struct RubbleStage {
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
    fn new(rubble: MaterialId, fraction: (u32, u32), max_depth: i32) -> Self {
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
    fn column(
        &mut self,
        snap: &Snapshot,
        reader: &mut Reader<'_>,
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
    fn write<B: Composed>(
        self,
        snap: &Snapshot,
        reader: &mut Reader<'_>,
        compose: &mut Compose<B>,
        spare: &impl Fn(MaterialId) -> bool,
        touched: &mut BTreeSet<ChunkCoord>,
        bounds: &mut Bounds,
    ) -> u32 {
        let mut placed: u32 = 0;
        for ([x, y, z0], h) in self.piles {
            let mut top = None;
            for z in z0..z0.saturating_add(h) {
                if !snap.air_after_carve(reader, [x, y, z], spare) {
                    break;
                }
                compose.set(reader, [x, y, z], self.rubble);
                touched.insert(VoxelCoord([x, y, z]).chunk());
                placed += 1;
                top = Some(z);
            }
            if let Some(top) = top {
                bounds.add([x, y, z0], [x, y, top]);
            }
        }
        placed
    }
}

/// The `z` of the lowest voxel of each run of solid region voxels in a column's `parts`,
/// bottom first: a solid voxel whose voxel below is not a solid region voxel (or lies in a
/// cell outside the region).
fn run_bottoms(parts: &[(i32, [MaterialId; 8])]) -> Vec<i32> {
    let mut out = Vec::new();
    let mut below_solid = false;
    let mut prev_z: Option<i32> = None;
    for (cz, col) in parts {
        if prev_z != Some(cz - 1) {
            below_solid = false;
        }
        for (lz, &m) in col.iter().enumerate() {
            let solid = m != AIR;
            if solid && !below_solid {
                out.push(cz * BE + lz as i32);
            }
            below_solid = solid;
        }
        prev_z = Some(*cz);
    }
    out
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

        let mut reader = Reader::new(&map);
        let mut yields = (yield_below > 0).then(|| Yield::new(yield_below, &self.materials));
        let mut dropped = max_drop;
        for (xy, parts) in &columns {
            for z in run_bottoms(parts) {
                let gap = snap
                    .gap_below(&mut reader, *xy, z, max_drop, &|_| false, yields.as_mut())
                    .map_err(EditError::NotResident)?;
                dropped = dropped.min(gap);
            }
        }
        if dropped == 0 {
            return Ok(DropResult::default());
        }

        let mut moved = Counts::default();
        let mut crushed = Counts::default();
        let mut bounds = Bounds::default();
        let mut compose = Compose::<B>::new(&snap, &|_| false);
        let mut touched = BTreeSet::new();
        if yields.is_some() {
            // Every solid voxel outside the region in a swept span yields (the gap reached
            // past it), so each is crushed; spans of two runs of one column may overlap.
            let mut by_cell: BTreeMap<CellCoord, Vec<usize>> = BTreeMap::new();
            let mut swept: Vec<i32> = Vec::new();
            for (xy, parts) in &columns {
                let [x, y] = *xy;
                swept.clear();
                for z in run_bottoms(parts) {
                    swept.extend(z - dropped..z);
                }
                swept.sort_unstable();
                swept.dedup();
                for &z in &swept {
                    let v = [x, y, z];
                    let cell = cell_of(v);
                    if snap.bricks.contains_key(&CellKey(cell)) {
                        continue;
                    }
                    let m = reader
                        .brick(cell)
                        .expect("scanned resident")
                        .get(index_of(v));
                    if m != AIR {
                        crushed.add(m, 1);
                        bounds.add(v, v);
                        touched.insert(VoxelCoord(v).chunk());
                        by_cell.entry(cell).or_default().push(index_of(v));
                    }
                }
            }
            for (cell, indices) in by_cell {
                compose.brick(&mut reader, cell).clear(&indices);
            }
        }
        for (&[cx, cy], zs) in &snap.stacks {
            for &cz in zs {
                let c = CellCoord([cx, cy, cz]);
                let min = c.min_voxel().0;
                match snap.brick(c) {
                    RawBrick::Uniform(AIR) => {}
                    RawBrick::Uniform(m) => {
                        moved.add(*m, 512);
                        let hi = min.map(|v| v + BE - 1);
                        bounds.add([min[0], min[1], min[2] - dropped], hi);
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
                    RawBrick::Full(vox) => {
                        let (mut lo, mut hi) = ([BE; 3], [-1; 3]);
                        for (i, &m) in vox.iter().enumerate() {
                            if m == AIR {
                                continue;
                            }
                            moved.add(m, 1);
                            let l = [i % 8, i / 8 % 8, i / 64].map(|x| x as i32);
                            lo = [0, 1, 2].map(|k| lo[k].min(l[k]));
                            hi = [0, 1, 2].map(|k| hi[k].max(l[k]));
                        }
                        if hi[0] >= 0 {
                            compose.place_dropped(
                                &mut reader,
                                c,
                                &snap.packed[&CellKey(c)],
                                dropped,
                            );
                            let lo = [0, 1, 2].map(|k| min[k] + lo[k]);
                            let hi = [0, 1, 2].map(|k| min[k] + hi[k]);
                            bounds.add([lo[0], lo[1], lo[2] - dropped], hi);
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
        Ok(DropResult {
            moved: moved.sorted(),
            dropped,
            crushed: crushed.sorted(),
            touched: touched.into_iter().collect(),
            aabb: bounds.get(),
        })
    }

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
        let mut reader = Reader::new(&map);
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
        let mut removed = Counts::default();
        let mut bounds = Bounds::default();
        let mut touched = BTreeSet::new();
        for (&[cx, cy], zs) in &snap.stacks {
            for &cz in zs {
                let c = CellCoord([cx, cy, cz]);
                let min = c.min_voxel().0;
                let (lo, hi) = match snap.brick(c) {
                    RawBrick::Uniform(m) if !carved(*m) => continue,
                    RawBrick::Uniform(m) => {
                        removed.add(*m, 512);
                        ([0; 3], [BE - 1; 3])
                    }
                    RawBrick::Full(vox) => {
                        let (mut lo, mut hi) = ([BE; 3], [-1; 3]);
                        for (i, &m) in vox.iter().enumerate() {
                            if carved(m) {
                                removed.add(m, 1);
                                let l = [i % 8, i / 8 % 8, i / 64].map(|x| x as i32);
                                lo = [0, 1, 2].map(|k| lo[k].min(l[k]));
                                hi = [0, 1, 2].map(|k| hi[k].max(l[k]));
                            }
                        }
                        if hi[0] < 0 {
                            continue;
                        }
                        (lo, hi)
                    }
                };
                bounds.add(
                    [0, 1, 2].map(|k| min[k] + lo[k]),
                    [0, 1, 2].map(|k| min[k] + hi[k]),
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
            &mut bounds,
        );
        drop(reader);
        compose.commit(self, &map);
        drop(map);
        Ok(RemoveResult {
            removed: removed.sorted(),
            placed,
            touched: touched.into_iter().collect(),
            aabb: bounds.get(),
        })
    }

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
    /// the native path is tested against.
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

        let mut reader = Reader::new(&map);
        let mut stage = RubbleStage::new(rubble, (count, c), max_depth);
        for (xy, parts) in &columns {
            let (top_cell, _) = parts.last().expect("a column crosses a named cell");
            stage.column(&snap, &mut reader, *xy, 1, (top_cell + 1) * BE, &spare)?;
        }

        let mut compose = Compose::<B>::new(&snap, &spare);
        let mut bounds = Bounds::default();
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
            aabb: bounds.get(),
        })
    }
}
