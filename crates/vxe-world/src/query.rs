//! Region queries over resident chunks.

use crate::brick::Brick;
use crate::coords::{BRICK_EDGE, CHUNK_EDGE, ChunkCoord, VoxelCoord, brick_slot};
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
    /// chunks that are not resident are not counted.
    pub fn count_in_box(
        &self,
        lo: impl Into<VoxelCoord>,
        hi: impl Into<VoxelCoord>,
    ) -> Vec<(MaterialId, u64)> {
        self.count_in_box_with_stats(lo, hi).0
    }

    /// [`World::count_in_box`] plus the work it did.
    pub fn count_in_box_with_stats(
        &self,
        lo: impl Into<VoxelCoord>,
        hi: impl Into<VoxelCoord>,
    ) -> (Vec<(MaterialId, u64)>, QueryStats) {
        let (lo, hi) = (lo.into().0.map(i64::from), hi.into().0.map(i64::from));
        let mut counts: Vec<(MaterialId, u64)> = Vec::new();
        let mut stats = QueryStats::default();
        if (0..3).any(|k| lo[k] > hi[k]) {
            return (counts, stats);
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
                    let vlo = [0, 1, 2].map(|k| (c[k] * ce).max(lo[k]));
                    let vhi = [0, 1, 2].map(|k| (c[k] * ce + ce - 1).min(hi[k]));
                    let blo = vlo.map(|v| v.div_euclid(be));
                    let bhi = vhi.map(|v| v.div_euclid(be));
                    for bz in blo[2]..=bhi[2] {
                        for by in blo[1]..=bhi[1] {
                            for bx in blo[0]..=bhi[0] {
                                let b = [bx, by, bz];
                                let elo = [0, 1, 2].map(|k| (b[k] * be).max(vlo[k]));
                                let ehi = [0, 1, 2].map(|k| (b[k] * be + be - 1).min(vhi[k]));
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
        (counts, stats)
    }
}
