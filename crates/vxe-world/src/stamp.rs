//! Stamping prefabs: at runtime into a resident world, and at generation time through a
//! source wrapper.
//!
//! A stamp writes every masked voxel of the rotated prefab whatever the material flags on
//! either side: it is an authoring operation, so an indestructible shell can replace
//! anything and anything can replace it. It reports what it changed exactly as an edit does
//! (see `edit.rs`): counts by old material for voxels whose material changed, the touched
//! chunks ascending, and the inclusive bounds of the changed voxels. Chunks are written one
//! write lock at a time in ascending order under the store map's read lock (ADR-0002,
//! ADR-0003). A chunk the box meets that is not resident, and that the prefab writes into, is
//! left to a placement the store keeps: the streamer writes it into the chunk when it inserts
//! it, generated or restored from persistence, and drops the placement once every such chunk
//! has been written (ADR-0066).

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::brick::Brick;
use crate::chunk::Chunk;
use crate::coords::{
    BRICK_EDGE, CHUNK_EDGE, ChunkCoord, VoxelCoord, brick_slot, clip_at, clip_box, index_in_brick,
};
use crate::edit::{EditError, EditResult};
use crate::fixed::{FIXED_ONE, Fixed3};
use crate::limits::Field;
use crate::material::MaterialId;
use crate::prefab::{BrickRef, PREFAB_BRICK_EDGE, Prefab, Rot};
use crate::store::{World, read, write};
use crate::tally::Tally;

/// The inclusive voxel box `lo..=hi` a prefab of `size` covers with its minimum corner at
/// `origin`, in `i64` so the far corner cannot overflow; `None` for an empty prefab.
pub(crate) fn voxel_box(origin: VoxelCoord, size: [u32; 3]) -> Option<([i64; 3], [i64; 3])> {
    if size.contains(&0) {
        return None;
    }
    let lo = origin.0.map(i64::from);
    let hi = [0, 1, 2].map(|k| lo[k] + i64::from(size[k]) - 1);
    Some((lo, hi))
}

/// The chunks a voxel box meets, ascending.
pub(crate) fn chunks_of(lo: [i64; 3], hi: [i64; 3]) -> impl Iterator<Item = [i64; 3]> {
    let edge = i64::from(CHUNK_EDGE);
    let clo = lo.map(|v| v.div_euclid(edge));
    let chi = hi.map(|v| v.div_euclid(edge));
    (clo[0]..=chi[0]).flat_map(move |x| {
        (clo[1]..=chi[1]).flat_map(move |y| (clo[2]..=chi[2]).map(move |z| [x, y, z]))
    })
}

/// The prefab bricks the voxel box `lo..=hi` (inside the prefab's box, minimum corner at
/// `origin`) meets, as an inclusive range of brick coordinates, with the box in local voxels.
fn prefab_span(
    origin: [i64; 3],
    lo: [i64; 3],
    hi: [i64; 3],
) -> ([u32; 3], [u32; 3], [u32; 3], [u32; 3]) {
    let e = PREFAB_BRICK_EDGE;
    let plo = [0, 1, 2].map(|k| (lo[k] - origin[k]) as u32);
    let phi = [0, 1, 2].map(|k| (hi[k] - origin[k]) as u32);
    (plo.map(|v| v / e), phi.map(|v| v / e), plo, phi)
}

/// The one `(material, written)` every voxel of `prefab` in the voxel box `lo..=hi` holds,
/// when the prefab bricks the box meets are all uniform alike.
fn uniform_over(
    prefab: &Prefab,
    origin: [i64; 3],
    lo: [i64; 3],
    hi: [i64; 3],
) -> Option<(MaterialId, bool)> {
    let (blo, bhi, _, _) = prefab_span(origin, lo, hi);
    let mut seen = None;
    for z in blo[2]..=bhi[2] {
        for y in blo[1]..=bhi[1] {
            for x in blo[0]..=bhi[0] {
                match prefab.brick([x, y, z]) {
                    BrickRef::Uniform(m, w) if seen.is_none_or(|s| s == (m, w)) => {
                        seen = Some((m, w));
                    }
                    _ => return None,
                }
            }
        }
    }
    seen
}

/// Whether `prefab`, with its minimum corner at `origin`, writes any voxel in `lo..=hi` (a
/// sub-box of its own box). A brick that writes nothing is passed over whole.
fn writes_any(prefab: &Prefab, origin: [i64; 3], lo: [i64; 3], hi: [i64; 3]) -> bool {
    let (blo, bhi, plo, phi) = prefab_span(origin, lo, hi);
    let e = PREFAB_BRICK_EDGE;
    for bz in blo[2]..=bhi[2] {
        for by in blo[1]..=bhi[1] {
            for bx in blo[0]..=bhi[0] {
                match prefab.brick([bx, by, bz]) {
                    BrickRef::Uniform(_, w) => {
                        if w {
                            return true;
                        }
                    }
                    BrickRef::Full(_) => {
                        let b = [bx, by, bz];
                        let clo = [0, 1, 2].map(|k| plo[k].max(b[k] * e));
                        let chi = [0, 1, 2].map(|k| phi[k].min(b[k] * e + e - 1));
                        for z in clo[2]..=chi[2] {
                            for y in clo[1]..=chi[1] {
                                for x in clo[0]..=chi[0] {
                                    if prefab.cell([x, y, z]).1 {
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

/// Writes the masked voxels of `prefab` (already rotated, minimum corner at `origin`) that
/// fall inside `lo..=hi`, a box within chunk `coord` and within the prefab's box. Marks
/// changed slots dirty; a brick is written through `Brick::modify`, so one that became
/// uniform is stored `Uniform`. Per world brick, the prefab bricks it meets decide the path:
/// all writing nothing, the brick is passed over; all writing one material, a uniform world
/// brick of it is passed over and a whole world brick is replaced by one word; otherwise each
/// voxel is read. Returns whether any voxel changed.
fn blit(
    chunk: &mut Chunk,
    coord: ChunkCoord,
    prefab: &Prefab,
    origin: [i64; 3],
    lo: [i64; 3],
    hi: [i64; 3],
    mut tally: Option<&mut Tally>,
) -> bool {
    let be = i64::from(BRICK_EDGE);
    let base = coord.min_voxel().0.map(i64::from);
    let blo = [0, 1, 2].map(|k| (lo[k] - base[k]) / be);
    let bhi = [0, 1, 2].map(|k| (hi[k] - base[k]) / be);
    let mut changed_any = false;
    for bz in blo[2]..=bhi[2] {
        for by in blo[1]..=bhi[1] {
            for bx in blo[0]..=bhi[0] {
                let b = [bx, by, bz];
                let bmin = [0, 1, 2].map(|k| base[k] + b[k] * be);
                let (elo, ehi) = clip_at(bmin, be, lo, hi);
                let slot = brick_slot(b.map(|v| v as i32));
                let one = uniform_over(prefab, origin, elo, ehi);
                if matches!(one, Some((_, false))) {
                    continue;
                }
                let whole = (0..3).all(|k| elo[k] == bmin[k] && ehi[k] == bmin[k] + be - 1);
                let uniform = match chunk.brick(slot) {
                    Brick::Uniform(m) => Some(*m),
                    Brick::Full(_) => None,
                };
                if let (Some((m, true)), Some(old)) = (one, uniform) {
                    if old == m {
                        continue;
                    }
                    if whole {
                        if let Some(t) = tally.as_deref_mut() {
                            t.count(old, BRICK_EDGE.pow(3) as u32);
                            t.bound(elo, ehi);
                        }
                        chunk.set_brick(slot, Brick::Uniform(m));
                        changed_any = true;
                        continue;
                    }
                }
                // A uniform brick expands only when a written voxel differs from it.
                if let (None, Some(m)) = (one, uniform) {
                    let mut differs = false;
                    'scan: for z in elo[2]..=ehi[2] {
                        for y in elo[1]..=ehi[1] {
                            for x in elo[0]..=ehi[0] {
                                let l = [x - origin[0], y - origin[1], z - origin[2]];
                                let (p, w) = prefab.cell(l.map(|v| v as u32));
                                if w && p != m {
                                    differs = true;
                                    break 'scan;
                                }
                            }
                        }
                    }
                    if !differs {
                        continue;
                    }
                }
                let changed = chunk.brick_mut(slot).modify(|voxels| {
                    let mut changed = false;
                    for z in elo[2]..=ehi[2] {
                        for y in elo[1]..=ehi[1] {
                            let local = [0, (y - bmin[1]) as i32, (z - bmin[2]) as i32];
                            let brow = index_in_brick(local);
                            for x in elo[0]..=ehi[0] {
                                let new = match one {
                                    Some((m, _)) => m,
                                    None => {
                                        let l = [x - origin[0], y - origin[1], z - origin[2]];
                                        match prefab.cell(l.map(|v| v as u32)) {
                                            (m, true) => m,
                                            (_, false) => continue,
                                        }
                                    }
                                };
                                let cell = &mut voxels[brow + (x - bmin[0]) as usize];
                                let old = *cell;
                                if old == new {
                                    continue;
                                }
                                *cell = new;
                                changed = true;
                                if let Some(t) = tally.as_deref_mut() {
                                    // Each changed voxel bounds itself, at its `i32` coordinate.
                                    let v = [x as i32, y as i32, z as i32].map(i64::from);
                                    t.count(old, 1);
                                    t.bound(v, v);
                                }
                            }
                        }
                    }
                    changed
                });
                if changed {
                    chunk.mark_dirty(slot);
                    changed_any = true;
                }
            }
        }
    }
    changed_any
}

/// Writes the masked voxels of `prefab` (already rotated, covering the voxel box `lo..=hi`
/// with its minimum corner at `lo`) that fall inside chunk `coord`. For sources: counts
/// nothing and leaves the chunk's edited flag alone. Returns whether any voxel changed.
pub(crate) fn blit_into(
    chunk: &mut Chunk,
    coord: ChunkCoord,
    prefab: &Prefab,
    lo: [i64; 3],
    hi: [i64; 3],
) -> bool {
    let c = coord.0.map(i64::from);
    let (clo, chi) = clip_box(c, i64::from(CHUNK_EDGE), lo, hi);
    if (0..3).any(|k| clo[k] > chi[k]) {
        return false;
    }
    blit(chunk, coord, prefab, lo, clo, chi, None)
}

/// A stamp's part the world has not written: the rotated prefab, its inclusive voxel box
/// (minimum corner first), and the chunks it still has to write.
#[derive(Debug)]
pub(crate) struct Placement {
    prefab: Arc<Prefab>,
    lo: [i64; 3],
    hi: [i64; 3],
    remaining: BTreeSet<ChunkCoord>,
}

/// The store's pending placements in stamp order (a later one wins where two overlap, as at
/// the stamp), and the chunk writes made from them.
#[derive(Debug, Default)]
pub(crate) struct Pending {
    pub(crate) list: Vec<Placement>,
    pub(crate) writes: u64,
}

impl Pending {
    /// Writes every pending placement that still has to write `coord` into `chunk`, in stamp
    /// order, and forgets `coord` for each; drops the placements left with nothing to write.
    /// Marks the chunk edited when a voxel changed, so it is persisted, not regenerated, when it
    /// leaves residency. Returns whether a voxel changed.
    pub(crate) fn apply(&mut self, coord: ChunkCoord, chunk: &mut Chunk) -> bool {
        let mut changed = false;
        for p in self.list.iter_mut() {
            if !p.remaining.remove(&coord) {
                continue;
            }
            changed |= blit_into(chunk, coord, &p.prefab, p.lo, p.hi);
            self.writes += 1;
        }
        self.list.retain(|p| !p.remaining.is_empty());
        if changed {
            chunk.mark_edited();
        }
        changed
    }
}

impl World {
    /// Writes `prefab` turned by `rot`, with the rotated volume's minimum corner at `origin`,
    /// and reports what changed. Every masked voxel is written whatever the material flags.
    /// The resident chunks the box meets are written now and reported; the rest the prefab
    /// writes into are kept as one pending placement, written when the streamer inserts each
    /// (ADR-0066), and not reported.
    ///
    /// Refused with `OutOfRange`, changing nothing, under the world's limits like the other op
    /// kinds: `EditExtent` when the prefab spans more than `edit_axis_max` voxels on an axis
    /// (checked before it is turned; a turn permutes the axes), `Coord` when the rotated
    /// volume's minimum or maximum corner lies past `coord_max`.
    pub fn stamp(
        &self,
        prefab: &Prefab,
        origin: impl Into<VoxelCoord>,
        rot: Rot,
    ) -> Result<EditResult, EditError> {
        let limits = self.limits();
        if prefab.size().iter().any(|&n| n > limits.edit_axis_max) {
            return Err(EditError::OutOfRange {
                what: Field::EditExtent,
            });
        }
        let rotated;
        let prefab = if rot == Rot::IDENTITY {
            prefab
        } else {
            rotated = prefab.rotated(rot);
            &rotated
        };
        let origin = origin.into();
        let Some((lo, hi)) = voxel_box(origin, prefab.size()) else {
            return Ok(EditResult::default());
        };
        for corner in [lo, hi] {
            limits.coord(Fixed3(corner.map(|v| v * FIXED_ONE)))?;
        }
        let o = origin.0.map(i64::from);

        // Resident chunks and the missing ones the prefab writes into, ascending; the map guard
        // lives to the end so no chunk can be inserted or evicted before its write or before the
        // placement that will write it is kept.
        let map = read(&self.store.chunks);
        let mut chunks = Vec::new();
        let mut missing = BTreeSet::new();
        for c in chunks_of(lo, hi) {
            let (clo, chi) = clip_box(c, i64::from(CHUNK_EDGE), lo, hi);
            let coord = ChunkCoord(c.map(|v| v as i32));
            match map.get(&coord) {
                Some(chunk) => chunks.push((coord, clo, chi, chunk)),
                None if writes_any(prefab, o, clo, chi) => {
                    missing.insert(coord);
                }
                None => {}
            }
        }

        let mut tally = Tally::new(self.materials.len());
        let mut touched = Vec::new();
        for (coord, clo, chi, chunk) in chunks {
            let mut chunk = write(chunk);
            let changed = blit(&mut chunk, coord, prefab, o, clo, chi, Some(&mut tally));
            if changed {
                chunk.mark_edited();
                self.store.mark_dirty(coord);
                touched.push(coord);
            }
        }
        if !missing.is_empty() {
            crate::store::lock(&self.store.pending)
                .list
                .push(Placement {
                    prefab: Arc::new(prefab.clone()),
                    lo,
                    hi,
                    remaining: missing,
                });
        }
        drop(map);
        let aabb = tally.aabb();
        Ok(EditResult {
            changed_from: tally.changed_from(),
            touched,
            aabb,
        })
    }
}
