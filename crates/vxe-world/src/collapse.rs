//! The collapse ops (ADR-0008): a rigid vertical drop of a region of cells, and its removal with
//! rubble. Both take the region as cells (from [`crate::find_unsupported`] or the caller's own
//! choice) and act on every solid voxel of those cells' bricks, except that the removal spares
//! voxels of an `INDESTRUCTIBLE` material as `Mode::Carve` does.
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

use crate::brick::{Brick, SpareArrays};
use crate::chunk::Chunk;
use crate::coarse::CellCoord;
use crate::coords::{BRICK_EDGE, ChunkCoord, VoxelCoord, index_in_brick};
use crate::edit::{EditError, EditResult};
use crate::material::{AIR, MaterialId};
use crate::store::{World, read, write};
use crate::support::{CellKey, CellMap};

/// What [`World::drop_region`] did.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct DropResult {
    /// The region's solid voxels by material, ascending by material, when it moved; empty when
    /// `dropped == 0`.
    pub moved: Vec<(MaterialId, u32)>,
    /// How far every voxel moved down, in voxels.
    pub dropped: i32,
    /// Chunks holding a vacated or a landed voxel, ascending; empty when nothing moved.
    pub touched: Vec<ChunkCoord>,
    /// Inclusive bounds of the vacated and the landed voxels; `None` when nothing moved.
    pub aabb: Option<(VoxelCoord, VoxelCoord)>,
}

impl DropResult {
    /// The result in the edit-result vocabulary, for [`crate::CoarseGrid::update`]:
    /// `changed_from` is `moved`, `touched` and `aabb` are the same.
    pub fn edit_result(&self) -> EditResult {
        EditResult {
            changed_from: self.moved.clone(),
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
    bricks: CellMap<Brick>,
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
        let mut stacks: BTreeMap<[i32; 2], Vec<i32>> = BTreeMap::new();
        let mut reader = Reader::new(map);
        for &c in &cells {
            let b = reader.brick(c).expect("residency checked above").clone();
            bricks.insert(CellKey(c), b);
            stacks.entry([c.0[0], c.0[1]]).or_default().push(c.0[2]);
        }
        Ok(Self { bricks, stacks })
    }

    fn brick(&self, c: CellCoord) -> &Brick {
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
    /// to the first solid voxel outside the region, at most `max`. A region voxel is free
    /// unless `spare` holds for its material (a voxel the op leaves in place). Fails with the
    /// chunk of the first non-resident voxel reached.
    fn gap_below(
        &self,
        reader: &mut Reader<'_>,
        [x, y]: [i32; 2],
        z: i32,
        max: i32,
        spare: &impl Fn(MaterialId) -> bool,
    ) -> Result<i32, ChunkCoord> {
        let mut gap = 0;
        while gap < max {
            let top = z - gap - 1;
            let cell = cell_of([x, y, top]);
            // Voxels from `top` down to the bottom of its cell.
            let span = (top.rem_euclid(BE) + 1).min(max - gap);
            let brick = match self.bricks.get(&CellKey(cell)) {
                Some(b) => match b {
                    Brick::Uniform(m) if !spare(*m) => {
                        gap += span;
                        continue;
                    }
                    Brick::Uniform(_) => break,
                    Brick::Full(v) => {
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
            match brick {
                Brick::Uniform(AIR) => gap += span,
                Brick::Uniform(_) => break,
                Brick::Full(v) => {
                    for k in 0..span {
                        if v[index_of([x, y, top - k])] != AIR {
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

/// The bricks an op will write, composed in memory: each starts as the world's brick (or, for
/// a region cell, as its spared voxels in air, since every other solid voxel leaves) and takes
/// voxel writes. Bricks it expands take arrays from `spare`, a batch of the world's pool, and
/// bricks that collapse push theirs back onto it; [`Compose::commit`] hands the batch back.
struct Compose {
    bricks: CellMap<Brick>,
    spare: SpareArrays,
}

impl Compose {
    /// Every region cell, holding only the voxels `spare` keeps. `arrays` is a batch of the
    /// world's brick-array pool (any length, empty included: an empty batch makes `expand`
    /// allocate).
    fn new(snap: &Snapshot, spare: &impl Fn(MaterialId) -> bool, mut arrays: SpareArrays) -> Self {
        let kept = |m: MaterialId| if spare(m) { m } else { AIR };
        let bricks = snap
            .bricks
            .iter()
            .map(|(&k, b)| {
                let b = match b {
                    Brick::Uniform(m) => Brick::Uniform(kept(*m)),
                    Brick::Full(v) => {
                        let mut b = Brick::Full(Box::new(v.map(kept)));
                        b.collapse(&mut arrays);
                        b
                    }
                };
                (k, b)
            })
            .collect();
        Self {
            bricks,
            spare: arrays,
        }
    }

    /// The composed brick of `c` in `bricks`, read from the world on first use. It takes the
    /// map rather than `self` so a caller can expand the brick into `self.spare`.
    fn brick<'a>(
        bricks: &'a mut CellMap<Brick>,
        reader: &mut Reader<'_>,
        c: CellCoord,
    ) -> &'a mut Brick {
        bricks
            .entry(CellKey(c))
            .or_insert_with(|| reader.brick(c).expect("scanned resident").clone())
    }

    /// Writes the solid voxels of `src`, the brick of region cell `c`, `dropped` voxels lower.
    /// They land in at most two bricks: layers `r..8` of `src` (with `r = dropped mod 8`) in
    /// the cell `dropped / 8` below `c`, layers `0..r` in the cell under that. Each destination
    /// brick is looked up once and takes whole 64-voxel layers.
    fn place_dropped(&mut self, reader: &mut Reader<'_>, c: CellCoord, src: &Brick, dropped: i32) {
        const LAYER: usize = (BE * BE) as usize;
        let (q, r) = (dropped.div_euclid(BE), dropped.rem_euclid(BE));
        let [cx, cy, cz] = c.0;
        // (destination cell, source layers, destination layer of the first source layer)
        let upper = (CellCoord([cx, cy, cz - q]), r..BE, 0);
        let lower = (CellCoord([cx, cy, cz - q - 1]), 0..r, BE - r);
        for (to, layers, first) in [upper, lower] {
            if layers.is_empty() {
                continue;
            }
            let span = layers.start as usize * LAYER..layers.end as usize * LAYER;
            let at = first as usize * LAYER;
            let Self { bricks, spare } = self;
            let b = Self::brick(bricks, reader, to);
            match (src, &*b) {
                (Brick::Uniform(m), Brick::Uniform(d)) if m == d => {}
                (Brick::Uniform(m), _) => b.expand(spare)[at..at + span.len()].fill(*m),
                (Brick::Full(v), _) => {
                    let from = &v[span];
                    if let Brick::Uniform(d) = *b
                        && from.iter().all(|&m| m == AIR || m == d)
                    {
                        continue;
                    }
                    let dst = &mut b.expand(spare)[at..at + from.len()];
                    for (d, &m) in dst.iter_mut().zip(from) {
                        if m != AIR {
                            *d = m;
                        }
                    }
                }
            }
        }
    }

    fn set(&mut self, reader: &mut Reader<'_>, v: [i32; 3], m: MaterialId) {
        let Self { bricks, spare } = self;
        let b = Self::brick(bricks, reader, cell_of(v));
        if b.get(index_of(v)) != m {
            b.expand(spare)[index_of(v)] = m;
        }
    }

    /// Writes every composed brick that differs from the world's, ascending by chunk, and
    /// returns the batch of pool arrays, for the caller to give back once the map lock is
    /// released.
    fn commit(self, world: &World, map: &ChunkMap) -> SpareArrays {
        let Self { bricks, mut spare } = self;
        let mut by_chunk: BTreeMap<ChunkCoord, Vec<(usize, Brick)>> = BTreeMap::new();
        for (CellKey(c), mut b) in bricks {
            b.collapse(&mut spare);
            let (chunk, slot) = c.split();
            by_chunk.entry(chunk).or_default().push((slot, b));
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
        spare
    }
}

impl World {
    /// Moves the solid voxels of `cells`' bricks straight down, as one rigid piece, by the
    /// smallest air gap under the region's columns, at most `max_drop`.
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
        self.limits().check_region(cells.len())?;
        let map = read(&self.store.chunks);
        let snap = Snapshot::take(&map, cells)?;
        let columns = snap.columns();
        if columns.is_empty() || max_drop <= 0 {
            return Ok(DropResult::default());
        }

        let mut reader = Reader::new(&map);
        let mut dropped = max_drop;
        for (xy, parts) in &columns {
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
                            .gap_below(&mut reader, *xy, z, max_drop, &|_| false)
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

        let mut moved = Counts::default();
        let mut bounds = Bounds::default();
        // Each region brick lands in at most two bricks, so at most two expands per cell.
        let arrays = self.brick_arrays.take(2 * cells.len());
        let mut compose = Compose::new(&snap, &|_| false, arrays);
        let mut touched = BTreeSet::new();
        for (&[cx, cy], zs) in &snap.stacks {
            for &cz in zs {
                let c = CellCoord([cx, cy, cz]);
                let min = c.min_voxel().0;
                match snap.brick(c) {
                    Brick::Uniform(AIR) => {}
                    Brick::Uniform(m) => {
                        moved.add(*m, 512);
                        let hi = min.map(|v| v + BE - 1);
                        bounds.add([min[0], min[1], min[2] - dropped], hi);
                        touched.insert(c.split().0);
                        touched.insert(VoxelCoord([min[0], min[1], min[2] - dropped]).chunk());
                        touched.insert(VoxelCoord([min[0], min[1], hi[2] - dropped]).chunk());
                        if dropped % BE == 0 {
                            let to = CellCoord([cx, cy, cz - dropped / BE]);
                            let b = Compose::brick(&mut compose.bricks, &mut reader, to);
                            if *b == Brick::Uniform(AIR) {
                                *b = Brick::Uniform(*m);
                                continue;
                            }
                        }
                        compose.place_dropped(&mut reader, c, &Brick::Uniform(*m), dropped);
                    }
                    Brick::Full(vox) => {
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
                            compose.place_dropped(&mut reader, c, snap.brick(c), dropped);
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
        let arrays = compose.commit(self, &map);
        drop(map);
        self.brick_arrays.give(arrays);
        Ok(DropResult {
            moved: moved.sorted(),
            dropped,
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
        let mut piles: Vec<([i32; 3], i32)> = Vec::new();
        let whole = fraction.1 != 0 && rubble != AIR;
        // Carved voxels times `fraction.0` not yet piled, below `fraction.1` between columns.
        let mut acc: u64 = 0;
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
            let h = if whole {
                acc += n * u64::from(fraction.0);
                let h = acc / u64::from(fraction.1);
                acc -= h * u64::from(fraction.1);
                h
            } else {
                0
            };
            let gap = snap
                .gap_below(&mut reader, *xy, lowest, max_depth, &spare)
                .map_err(EditError::NotResident)?;
            if h > 0 && gap < max_depth {
                let h = i32::try_from(h).unwrap_or(i32::MAX);
                piles.push(([xy[0], xy[1], lowest - gap], h));
            }
        }

        // A batch sized to the region; rubble beyond it expands into fresh arrays.
        let arrays = self.brick_arrays.take(cells.len());
        let mut compose = Compose::new(&snap, &spare, arrays);
        let mut removed = Counts::default();
        let mut bounds = Bounds::default();
        let mut touched = BTreeSet::new();
        for (&[cx, cy], zs) in &snap.stacks {
            for &cz in zs {
                let c = CellCoord([cx, cy, cz]);
                let min = c.min_voxel().0;
                let (lo, hi) = match snap.brick(c) {
                    Brick::Uniform(m) if !carved(*m) => continue,
                    Brick::Uniform(m) => {
                        removed.add(*m, 512);
                        ([0; 3], [BE - 1; 3])
                    }
                    Brick::Full(vox) => {
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
        let mut placed: u32 = 0;
        for ([x, y, z0], h) in piles {
            let mut top = None;
            for z in z0..z0.saturating_add(h) {
                if !snap.air_after_carve(&mut reader, [x, y, z], &spare) {
                    break;
                }
                compose.set(&mut reader, [x, y, z], rubble);
                touched.insert(VoxelCoord([x, y, z]).chunk());
                placed += 1;
                top = Some(z);
            }
            if let Some(top) = top {
                bounds.add([x, y, z0], [x, y, top]);
            }
        }
        drop(reader);
        let arrays = compose.commit(self, &map);
        drop(map);
        self.brick_arrays.give(arrays);
        Ok(RemoveResult {
            removed: removed.sorted(),
            placed,
            touched: touched.into_iter().collect(),
            aabb: bounds.get(),
        })
    }
}
