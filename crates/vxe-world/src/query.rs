//! Region queries over resident chunks.

use crate::brick::Brick;
use crate::coords::{BRICK_EDGE, CHUNK_EDGE, ChunkCoord, VoxelCoord, brick_slot, clip_box};
use crate::edit::EditError;
use crate::material::{AIR, MaterialId};
use crate::packed::box_words;
use crate::store::{World, read};

/// Work a region query did, for callers that measure it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct QueryStats {
    /// `Full` bricks whose voxels were read (their mask and planes). `Uniform` bricks are
    /// counted without a read.
    pub voxel_arrays_read: u64,
}

impl World {
    /// Voxels per material in the inclusive box `lo..=hi`, ascending by material. Voxels in
    /// chunks that are not resident are not counted. Refused with
    /// [`EditError::OutOfRange`] (`QueryAxis`, `QueryVolume`) when the box is larger than the
    /// world's [`WorldLimits`](crate::WorldLimits) allow; a reversed box is empty.
    pub fn count_in_box(
        &self,
        lo: impl Into<VoxelCoord>,
        hi: impl Into<VoxelCoord>,
    ) -> Result<Vec<(MaterialId, u64)>, EditError> {
        self.count_in_box_with_stats(lo, hi)
            .map(|(counts, _)| counts)
    }

    /// [`World::count_in_box`] plus the work it did.
    pub fn count_in_box_with_stats(
        &self,
        lo: impl Into<VoxelCoord>,
        hi: impl Into<VoxelCoord>,
    ) -> Result<(Vec<(MaterialId, u64)>, QueryStats), EditError> {
        let (lo, hi) = (lo.into().0, hi.into().0);
        self.limits().check_query(lo, hi)?;
        let (lo, hi) = (lo.map(i64::from), hi.map(i64::from));
        let mut counts: Vec<(MaterialId, u64)> = Vec::new();
        let mut stats = QueryStats::default();
        if (0..3).any(|k| lo[k] > hi[k]) {
            return Ok((counts, stats));
        }
        let mut add = |m: MaterialId, n: u64| match counts.iter_mut().find(|(k, _)| *k == m) {
            Some((_, c)) => *c += n,
            None => counts.push((m, n)),
        };
        let ce = i64::from(CHUNK_EDGE);
        let be = i64::from(BRICK_EDGE);
        let clo = lo.map(|v| v.div_euclid(ce));
        let chi = hi.map(|v| v.div_euclid(ce));
        for cx in clo[0]..=chi[0] {
            for cy in clo[1]..=chi[1] {
                for cz in clo[2]..=chi[2] {
                    let c = [cx, cy, cz];
                    let coord = ChunkCoord(c.map(|v| v as i32));
                    let Some(chunk) = self.store.get(coord) else {
                        continue;
                    };
                    let chunk = read(&chunk);
                    let (vlo, vhi) = clip_box(c, ce, lo, hi);
                    let blo = vlo.map(|v| v.div_euclid(be));
                    let bhi = vhi.map(|v| v.div_euclid(be));
                    for bz in blo[2]..=bhi[2] {
                        for by in blo[1]..=bhi[1] {
                            for bx in blo[0]..=bhi[0] {
                                let b = [bx, by, bz];
                                let (elo, ehi) = clip_box(b, be, vlo, vhi);
                                let slot = brick_slot([0, 1, 2].map(|k| (b[k] - c[k] * 8) as i32));
                                let brick = chunk.brick(slot);
                                if !brick.is_full() {
                                    let n = (0..3).map(|k| (ehi[k] - elo[k] + 1) as u64).product();
                                    add(brick.get(0), n);
                                    continue;
                                }
                                stats.voxel_arrays_read += 1;
                                let Brick::Full(p) = brick else {
                                    unreachable!("a Full brick")
                                };
                                // The box's voxel words: per-entry popcounts under them, and the
                                // clear mask bits under them for air.
                                let sel = box_words(
                                    elo.map(|v| v.rem_euclid(be) as u8),
                                    ehi.map(|v| v.rem_euclid(be) as u8),
                                );
                                let air: u32 = (0..8)
                                    .map(|z| (sel[z] & !p.mask_word(z)).count_ones())
                                    .sum();
                                if air > 0 {
                                    add(AIR, u64::from(air));
                                }
                                p.counts_under(&sel, &mut |m, n| add(m, u64::from(n)));
                            }
                        }
                    }
                }
            }
        }
        counts.sort_unstable();
        Ok((counts, stats))
    }

    /// The solid-count gradient around voxel `v`: over the cube `v - half ..= v + half`, per
    /// axis the count of solid (non-air) voxels on the minus side of `v`'s plane less the
    /// count on the plus side; the plane through `v` counts for neither. It points from solid
    /// toward air, and is a surface's normal smoothed at the scale of `half`: the face normal
    /// on a flat face, the slope on a staircase. Read through the bricks' masks: a `Uniform`
    /// brick by arithmetic, a `Full` brick by its mask bits. Chunks that are not resident read
    /// as air. Refused as [`World::count_in_box`] is when the cube is larger than the world's
    /// limits allow.
    pub fn solid_gradient(
        &self,
        v: impl Into<VoxelCoord>,
        half: u32,
    ) -> Result<[i64; 3], EditError> {
        self.solid_gradient_with_stats(v, half).map(|(g, _)| g)
    }

    /// [`World::solid_gradient`] plus the work it did.
    pub fn solid_gradient_with_stats(
        &self,
        v: impl Into<VoxelCoord>,
        half: u32,
    ) -> Result<([i64; 3], QueryStats), EditError> {
        let c = v.into().0.map(i64::from);
        let h = i64::from(half);
        let clamp = |x: i64| x.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
        self.limits()
            .check_query(c.map(|x| clamp(x - h)), c.map(|x| clamp(x + h)))?;
        let (lo, hi) = (c.map(|x| x - h), c.map(|x| x + h));
        let mut g = [0i64; 3];
        let mut stats = QueryStats::default();
        let ce = i64::from(CHUNK_EDGE);
        let be = i64::from(BRICK_EDGE);
        let clo = lo.map(|x| x.div_euclid(ce));
        let chi = hi.map(|x| x.div_euclid(ce));
        for cx in clo[0]..=chi[0] {
            for cy in clo[1]..=chi[1] {
                for cz in clo[2]..=chi[2] {
                    let cc = [cx, cy, cz];
                    let Some(chunk) = self.store.get(ChunkCoord(cc.map(|x| x as i32))) else {
                        continue;
                    };
                    let chunk = read(&chunk);
                    let (vlo, vhi) = clip_box(cc, ce, lo, hi);
                    let blo = vlo.map(|x| x.div_euclid(be));
                    let bhi = vhi.map(|x| x.div_euclid(be));
                    for bz in blo[2]..=bhi[2] {
                        for by in blo[1]..=bhi[1] {
                            for bx in blo[0]..=bhi[0] {
                                let b = [bx, by, bz];
                                let (elo, ehi) = clip_box(b, be, vlo, vhi);
                                let slot = brick_slot([0, 1, 2].map(|k| (b[k] - cc[k] * 8) as i32));
                                let brick = chunk.brick(slot);
                                let mask = match brick {
                                    Brick::Uniform(AIR) => continue,
                                    Brick::Uniform(_) => None,
                                    Brick::Full(p) => {
                                        stats.voxel_arrays_read += 1;
                                        Some(p.mask())
                                    }
                                };
                                // The solid voxels of the brick's part of `slo..=shi`.
                                let solid = |slo: [i64; 3], shi: [i64; 3]| -> i64 {
                                    if (0..3).any(|k| slo[k] > shi[k]) {
                                        return 0;
                                    }
                                    match &mask {
                                        None => (0..3).map(|k| shi[k] - slo[k] + 1).product(),
                                        Some(m) => {
                                            let sel = box_words(
                                                slo.map(|x| x.rem_euclid(be) as u8),
                                                shi.map(|x| x.rem_euclid(be) as u8),
                                            );
                                            (0..8)
                                                .map(|z| i64::from((sel[z] & m[z]).count_ones()))
                                                .sum()
                                        }
                                    }
                                };
                                for k in 0..3 {
                                    let mut minus_hi = ehi;
                                    minus_hi[k] = ehi[k].min(c[k] - 1);
                                    let mut plus_lo = elo;
                                    plus_lo[k] = elo[k].max(c[k] + 1);
                                    g[k] += solid(elo, minus_hi) - solid(plus_lo, ehi);
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok((g, stats))
    }
}
