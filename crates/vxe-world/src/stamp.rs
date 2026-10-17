//! Stamping prefabs: at runtime into a resident world, and at generation time through a
//! source wrapper.
//!
//! A stamp writes every masked voxel of the rotated prefab whatever the material flags on
//! either side: it is an authoring operation, so an indestructible shell can replace
//! anything and anything can replace it. It reports what it changed exactly as an edit does
//! (see `edit.rs`): counts by old material for voxels whose material changed, the touched
//! chunks ascending, and the inclusive bounds of the changed voxels. Residency is checked for
//! every chunk holding a masked voxel before anything is written, and chunks are written one
//! write lock at a time in ascending order under the store map's read lock (ADR-0002,
//! ADR-0003).

use crate::brick::{Brick, SpareArrays};
use crate::chunk::Chunk;
use crate::coords::{BRICK_EDGE, CHUNK_EDGE, ChunkCoord, VoxelCoord, brick_slot, index_in_brick};
use crate::edit::{EditError, EditResult};
use crate::material::MaterialId;
use crate::prefab::{Prefab, Rot};
use crate::store::{World, read, write};

/// Counts by old material and bounds of the changed voxels of one stamp.
struct Tally {
    /// Indexed by material id, grown on demand.
    counts: Vec<u32>,
    lo: [i32; 3],
    hi: [i32; 3],
    any: bool,
}

impl Tally {
    fn new(rows: usize) -> Self {
        Self {
            counts: vec![0; rows],
            lo: [0; 3],
            hi: [0; 3],
            any: false,
        }
    }

    #[inline]
    fn count(&mut self, m: MaterialId, v: [i32; 3]) {
        let i = usize::from(m);
        if i >= self.counts.len() {
            self.counts.resize(i + 1, 0);
        }
        self.counts[i] += 1;
        if self.any {
            self.lo = [0, 1, 2].map(|k| self.lo[k].min(v[k]));
            self.hi = [0, 1, 2].map(|k| self.hi[k].max(v[k]));
        } else {
            (self.lo, self.hi, self.any) = (v, v, true);
        }
    }

    fn changed_from(&self) -> Vec<(MaterialId, u32)> {
        self.counts
            .iter()
            .enumerate()
            .filter(|&(_, &n)| n != 0)
            .map(|(m, &n)| (m as MaterialId, n))
            .collect()
    }
}

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
fn chunks_of(lo: [i64; 3], hi: [i64; 3]) -> impl Iterator<Item = [i64; 3]> {
    let edge = i64::from(CHUNK_EDGE);
    let clo = lo.map(|v| v.div_euclid(edge));
    let chi = hi.map(|v| v.div_euclid(edge));
    (clo[0]..=chi[0]).flat_map(move |x| {
        (clo[1]..=chi[1]).flat_map(move |y| (clo[2]..=chi[2]).map(move |z| [x, y, z]))
    })
}

/// The part of the box `lo..=hi` inside chunk `c`, inclusive.
fn clip_to_chunk(c: [i64; 3], lo: [i64; 3], hi: [i64; 3]) -> ([i64; 3], [i64; 3]) {
    let edge = i64::from(CHUNK_EDGE);
    let clo = [0, 1, 2].map(|k| (c[k] * edge).max(lo[k]));
    let chi = [0, 1, 2].map(|k| (c[k] * edge + edge - 1).min(hi[k]));
    (clo, chi)
}

/// Whether `prefab`, with its minimum corner at `origin`, writes any voxel in `lo..=hi` (a
/// sub-box of its own box).
fn writes_any(prefab: &Prefab, origin: [i64; 3], lo: [i64; 3], hi: [i64; 3]) -> bool {
    let s = prefab.size().map(|v| v as usize);
    for z in lo[2]..=hi[2] {
        for y in lo[1]..=hi[1] {
            let row = ((z - origin[2]) as usize * s[1] + (y - origin[1]) as usize) * s[0];
            for x in lo[0]..=hi[0] {
                if prefab.written(row + (x - origin[0]) as usize) {
                    return true;
                }
            }
        }
    }
    false
}

/// Writes the masked voxels of `prefab` (already rotated, minimum corner at `origin`) that
/// fall inside `lo..=hi`, a box within chunk `coord` and within the prefab's box. Marks
/// changed slots dirty and collapses bricks that became uniform, pushing their arrays onto
/// `spare`. Returns whether any voxel changed. A `Uniform` brick that must hold mixed voxels
/// expands into an array from `spare`, or a new one when `spare` is empty.
#[allow(
    clippy::too_many_arguments,
    reason = "the blit's inputs plus the caller's spare arrays"
)]
fn blit(
    chunk: &mut Chunk,
    coord: ChunkCoord,
    prefab: &Prefab,
    origin: [i64; 3],
    lo: [i64; 3],
    hi: [i64; 3],
    mut tally: Option<&mut Tally>,
    spare: &mut SpareArrays,
) -> bool {
    let be = i64::from(BRICK_EDGE);
    let base = coord.min_voxel().0.map(i64::from);
    let s = prefab.size().map(|v| v as usize);
    let blo = [0, 1, 2].map(|k| (lo[k] - base[k]) / be);
    let bhi = [0, 1, 2].map(|k| (hi[k] - base[k]) / be);
    let mut changed_any = false;
    for bz in blo[2]..=bhi[2] {
        for by in blo[1]..=bhi[1] {
            for bx in blo[0]..=bhi[0] {
                let b = [bx, by, bz];
                let bmin = [0, 1, 2].map(|k| base[k] + b[k] * be);
                let elo = [0, 1, 2].map(|k| bmin[k].max(lo[k]));
                let ehi = [0, 1, 2].map(|k| (bmin[k] + be - 1).min(hi[k]));
                let slot = brick_slot(b.map(|v| v as i32));
                let brick = chunk.brick_mut(slot);
                // A uniform brick expands only when a written voxel differs from it.
                if let Brick::Uniform(m) = *brick {
                    let mut differs = false;
                    'scan: for z in elo[2]..=ehi[2] {
                        for y in elo[1]..=ehi[1] {
                            let row =
                                ((z - origin[2]) as usize * s[1] + (y - origin[1]) as usize) * s[0];
                            for x in elo[0]..=ehi[0] {
                                let i = row + (x - origin[0]) as usize;
                                if prefab.written(i) && prefab.raw(i) != m {
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
                let voxels = brick.expand(spare);
                let mut changed = false;
                for z in elo[2]..=ehi[2] {
                    for y in elo[1]..=ehi[1] {
                        let row =
                            ((z - origin[2]) as usize * s[1] + (y - origin[1]) as usize) * s[0];
                        let local = [0, (y - bmin[1]) as i32, (z - bmin[2]) as i32];
                        let brow = index_in_brick(local);
                        for x in elo[0]..=ehi[0] {
                            let i = row + (x - origin[0]) as usize;
                            if !prefab.written(i) {
                                continue;
                            }
                            let new = prefab.raw(i);
                            let cell = &mut voxels[brow + (x - bmin[0]) as usize];
                            let old = *cell;
                            if old == new {
                                continue;
                            }
                            *cell = new;
                            changed = true;
                            if let Some(t) = tally.as_deref_mut() {
                                t.count(old, [x as i32, y as i32, z as i32]);
                            }
                        }
                    }
                }
                if changed {
                    brick.collapse(spare);
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
    let (clo, chi) = clip_to_chunk(c, lo, hi);
    if (0..3).any(|k| clo[k] > chi[k]) {
        return false;
    }
    // A source holds no world pool: expansions allocate, and collapsed arrays are freed.
    blit(
        chunk,
        coord,
        prefab,
        lo,
        clo,
        chi,
        None,
        &mut SpareArrays::new(),
    )
}

impl World {
    /// Writes `prefab` turned by `rot`, with the rotated volume's minimum corner at `origin`,
    /// and reports what changed. Every masked voxel is written whatever the material flags.
    /// Fails, changing nothing, with the first chunk (ascending) that holds a masked voxel
    /// and is not resident.
    pub fn stamp(
        &self,
        prefab: &Prefab,
        origin: impl Into<VoxelCoord>,
        rot: Rot,
    ) -> Result<EditResult, EditError> {
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
        let o = origin.0.map(i64::from);

        // Residency first, ascending; the map guard lives to the end so no checked chunk can
        // be evicted before its write.
        let map = read(&self.store.chunks);
        let mut chunks = Vec::new();
        for c in chunks_of(lo, hi) {
            let (clo, chi) = clip_to_chunk(c, lo, hi);
            let coord = ChunkCoord(c.map(|v| v as i32));
            match map.get(&coord) {
                Some(chunk) => chunks.push((coord, clo, chi, chunk)),
                None if writes_any(prefab, o, clo, chi) => {
                    return Err(EditError::NotResident(coord));
                }
                None => {}
            }
        }

        // One batch of pooled arrays for every brick the stamp may expand: at most the bricks
        // its box overlaps.
        let be = i64::from(BRICK_EDGE);
        let bound = (0..3)
            .map(|k| usize::try_from(hi[k].div_euclid(be) - lo[k].div_euclid(be) + 1).unwrap_or(0))
            .fold(1usize, usize::saturating_mul);
        let mut spare = self.brick_arrays.take(bound);
        let mut tally = Tally::new(self.materials.len());
        let mut touched = Vec::new();
        for (coord, clo, chi, chunk) in chunks {
            let mut chunk = write(chunk);
            let changed = blit(
                &mut chunk,
                coord,
                prefab,
                o,
                clo,
                chi,
                Some(&mut tally),
                &mut spare,
            );
            if changed {
                chunk.mark_edited();
                self.store.mark_dirty(coord);
                touched.push(coord);
            }
        }
        drop(map);
        self.brick_arrays.give(spare);
        Ok(EditResult {
            changed_from: tally.changed_from(),
            touched,
            aabb: tally
                .any
                .then_some((VoxelCoord(tally.lo), VoxelCoord(tally.hi))),
        })
    }
}
