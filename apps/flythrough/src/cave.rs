//! A seeded cave: integer value noise thresholded into rock and air, ore veins from a second
//! noise, an indestructible shell around the extent, and an air pocket at the spawn point.
//!
//! Every value is a pure integer function of the seed and the voxel coordinate, so chunks can
//! be generated in any order on any thread and on any platform with identical bytes.
//!
//! Density is the sum of two octaves, caverns at cell 64 and detail at cell 16 (one eighth
//! weight); ore veins come from a third noise at cell 16. All lattices align with bricks, so inside one brick each octave is a trilinear function of one cell, and a
//! trilinear function over a box takes its extremes at the box's corners. A brick whose eight
//! corner densities all lie beyond `MARGIN` on one side of a threshold is therefore uniform
//! on that side and is emitted without evaluating its voxels. `MARGIN` covers the integer
//! rounding in `ValueNoise::sample` (under one unit per octave, plus the detail shift).

use vxe_world::hash::ValueNoise;
use vxe_world::{
    AIR, BRICK_EDGE, Brick, CHUNK_BRICKS, CHUNK_EDGE, Chunk, ChunkCoord, ChunkExtent, MaterialId,
    VoxelCoord, WorldSource, brick_of_slot,
};

pub const ROCK: MaterialId = 1;
pub const ORE: MaterialId = 2;
pub const SHELL: MaterialId = 3;

/// Density above which a voxel is air.
const AIR_ABOVE: i64 = 6000;
/// Ore-noise value above which rock is ore.
const ORE_ABOVE: i64 = 28000;
/// The detail octave is divided by `1 << DETAIL_SHIFT` before it is added to the caverns.
const DETAIL_SHIFT: u32 = 3;
/// Rounding slack for the per-brick early-out.
const MARGIN: i64 = 4;
/// Voxels of indestructible shell inside each face of the extent.
const SHELL_THICKNESS: i32 = 2;
/// Radius of the spawn pocket, in voxels (3 m at 10 cm).
pub const SPAWN_POCKET_RADIUS: i64 = 30;

/// The cave over a bounded extent of chunks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaveSource {
    pub seed: u64,
    pub extent: ChunkExtent,
}

impl CaveSource {
    pub fn new(seed: u64, extent: ChunkExtent) -> Self {
        Self { seed, extent }
    }

    /// The voxel at the centre of the extent, where the spawn pocket is carved.
    pub fn spawn(&self) -> VoxelCoord {
        VoxelCoord(
            [0, 1, 2].map(|k| (self.extent.min.0[k] + self.extent.max.0[k]) * CHUNK_EDGE / 2),
        )
    }

    fn density_noise(&self) -> (ValueNoise, ValueNoise) {
        (
            ValueNoise::new(self.seed, 6),
            ValueNoise::new(self.seed ^ 0xD3A5_17C0_0000_0001, 4),
        )
    }

    fn ore_noise(&self) -> ValueNoise {
        ValueNoise::new(self.seed ^ 0x0EE0_0EE0_0000_0002, 4)
    }

    fn density(&self, p: [i32; 3]) -> i64 {
        let (a, b) = self.density_noise();
        a.sample(p) + (b.sample(p) >> DETAIL_SHIFT)
    }

    /// Rock, ore or air from the noise alone, before shell and pocket.
    fn base(&self, p: [i32; 3]) -> MaterialId {
        if self.density(p) > AIR_ABOVE {
            AIR
        } else if self.ore_noise().sample(p) > ORE_ABOVE {
            ORE
        } else {
            ROCK
        }
    }

    fn in_shell(&self, p: [i32; 3]) -> bool {
        (0..3).any(|k| {
            p[k] < self.extent.min.0[k] * CHUNK_EDGE + SHELL_THICKNESS
                || p[k] >= self.extent.max.0[k] * CHUNK_EDGE - SHELL_THICKNESS
        })
    }

    fn in_pocket(&self, p: [i32; 3]) -> bool {
        let s = self.spawn().0;
        let d2: i64 = (0..3)
            .map(|k| (i64::from(p[k]) - i64::from(s[k])).pow(2))
            .sum();
        d2 <= SPAWN_POCKET_RADIUS * SPAWN_POCKET_RADIUS
    }

    /// The final material of voxel `p`.
    pub fn material(&self, p: [i32; 3]) -> MaterialId {
        if self.in_shell(p) {
            SHELL
        } else if self.in_pocket(p) {
            AIR
        } else {
            self.base(p)
        }
    }

    /// The brick with minimum voxel `b`: uniform from its corners when they allow it, else
    /// evaluated per voxel.
    pub fn brick(&self, b: [i32; 3]) -> Brick {
        let hi = b.map(|v| v + BRICK_EDGE - 1);
        let near_shell = (0..3).any(|k| {
            b[k] < self.extent.min.0[k] * CHUNK_EDGE + SHELL_THICKNESS
                || hi[k] >= self.extent.max.0[k] * CHUNK_EDGE - SHELL_THICKNESS
        });
        let s = self.spawn().0;
        let pocket_near: i64 = (0..3)
            .map(|k| (i64::from(s[k].clamp(b[k], hi[k])) - i64::from(s[k])).pow(2))
            .sum();
        let near_pocket = pocket_near <= SPAWN_POCKET_RADIUS * SPAWN_POCKET_RADIUS;
        if !near_shell
            && !near_pocket
            && let Some(m) = self.uniform_from_corners(b, hi)
        {
            return Brick::Uniform(m);
        }
        // Every octave's lattice cell is at least a brick and brick-aligned, so one set of
        // corners per noise serves all 512 voxels.
        let (dn, dd) = self.density_noise();
        let on = self.ore_noise();
        let (dc, ddc, oc) = (
            dn.corners(dn.cell_of(b)),
            dd.corners(dd.cell_of(b)),
            on.corners(on.cell_of(b)),
        );
        Brick::from_fn(|l| {
            let p = [0, 1, 2].map(|k| b[k] + l[k]);
            if self.in_shell(p) {
                SHELL
            } else if self.in_pocket(p)
                || dn.interpolate(&dc, p) + (dd.interpolate(&ddc, p) >> DETAIL_SHIFT) > AIR_ABOVE
            {
                AIR
            } else if on.interpolate(&oc, p) > ORE_ABOVE {
                ORE
            } else {
                ROCK
            }
        })
    }

    /// The material of every voxel in `lo..=hi` when the corner values prove it uniform.
    fn uniform_from_corners(&self, lo: [i32; 3], hi: [i32; 3]) -> Option<MaterialId> {
        let corners: [[i32; 3]; 8] =
            std::array::from_fn(|i| [0, 1, 2].map(|k| if i >> k & 1 == 0 { lo[k] } else { hi[k] }));
        let d = corners.map(|c| self.density(c));
        if d.iter().all(|&v| v > AIR_ABOVE + MARGIN) {
            return Some(AIR);
        }
        if !d.iter().all(|&v| v <= AIR_ABOVE - MARGIN) {
            return None;
        }
        let ore = self.ore_noise();
        corners
            .iter()
            .all(|&c| ore.sample(c) <= ORE_ABOVE - MARGIN)
            .then_some(ROCK)
    }
}

impl WorldSource for CaveSource {
    fn chunk(&self, coord: ChunkCoord, out: &mut Chunk) {
        let base = coord.min_voxel().0;
        for slot in 0..CHUNK_BRICKS {
            let b = brick_of_slot(slot);
            let b = [0, 1, 2].map(|k| base[k] + b[k] * BRICK_EDGE);
            out.set_brick(slot, self.brick(b));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cave(seed: u64) -> CaveSource {
        CaveSource::new(seed, ChunkExtent::new([0, 0, 0], [4, 4, 4]))
    }

    /// The early-out never disagrees with evaluating every voxel.
    #[test]
    fn corner_early_out_matches_per_voxel_evaluation() {
        let c = cave(0x5EED_0001);
        let mut uniform = 0;
        for bz in 0..12 {
            for by in 0..12 {
                for bx in 0..12 {
                    let b = [bx, by, bz].map(|v| 8 + v * BRICK_EDGE);
                    let fast = c.brick(b);
                    let slow = Brick::from_fn(|l| c.material([0, 1, 2].map(|k| b[k] + l[k])));
                    assert_eq!(fast, slow, "brick at {b:?}");
                    uniform += i32::from(!fast.is_full());
                }
            }
        }
        // Most of the sample is uniform, so the early-out is doing work.
        assert!(uniform > 12 * 12 * 12 / 2, "{uniform}");
    }

    #[test]
    fn shell_is_indestructible_and_the_pocket_is_air() {
        let c = cave(1);
        assert!(crate::cave_materials().is_indestructible(SHELL));
        assert_eq!(c.material([0, 100, 100]), SHELL);
        assert_eq!(c.material([255, 100, 100]), SHELL);
        assert_eq!(c.material([128, 128, 128]), AIR);
        assert_eq!(c.material([128 + 30, 128, 128]), AIR);
        assert_eq!(c.spawn(), VoxelCoord([128, 128, 128]));
    }
}
