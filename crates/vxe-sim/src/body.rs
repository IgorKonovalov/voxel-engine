//! The body, its distance to an axis-aligned box, and the contact a box makes with it.
//!
//! Units are fixed point, 1/256 voxel; voxel `v` spans `[256 v, 256 v + 256)`. Squares are
//! taken in `i128`, so no product overflows for any position a `vxe-world` coordinate can name.

use std::sync::{PoisonError, RwLock, RwLockReadGuard};

use vxe_world::{
    AIR, BRICK_EDGE, Brick, CHUNK_EDGE, ChunkCoord, FIXED_ONE, Fixed3, MaterialId, QueryStats,
    VoxelCoord, World, brick_slot, index_in_brick,
};

/// A vertical cylinder: its axis is the vertical line through `foot`, it occupies `z` in
/// `[foot.z, foot.z + height)` and plan distance `< radius` from the axis (ADR-0005).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Body {
    pub foot: Fixed3,
    pub radius: i64,
    pub height: i64,
}

impl Body {
    /// The same body with its foot at `foot`.
    pub fn at(&self, foot: [i64; 3]) -> Self {
        Self {
            foot: Fixed3(foot),
            ..*self
        }
    }
}

/// Where a body touches the world.
///
/// `normal` is an integer direction, not a unit vector: `(h.x, h.y, 0)` for a wall, with `h`
/// the plan vector from the box's nearest point to the axis; `(0, 0, radius)` for a floor and
/// `(0, 0, -radius)` for a ceiling. A rim contact (the box is vertically separated but its
/// plan rectangle is not within the radius of the axis) counts as a wall.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Contact {
    pub voxel: VoxelCoord,
    pub material: MaterialId,
    pub normal: [i64; 3],
}

impl Contact {
    /// A floor contact: the box is below the body and under its base. The only walkable kind.
    pub fn is_floor(&self) -> bool {
        self.normal[2] > 0
    }

    pub fn is_ceiling(&self) -> bool {
        self.normal[2] < 0
    }

    pub fn is_wall(&self) -> bool {
        self.normal[2] == 0
    }
}

/// Floor of the square root.
pub fn isqrt(n: u128) -> u128 {
    if n < 2 {
        return n;
    }
    // Start at a power of two at or above the root; Newton's step then decreases monotonically
    // to the floor of the root.
    let mut x = 1u128 << (128 - n.leading_zeros()).div_ceil(2);
    loop {
        let y = (x + n / x) / 2;
        if y >= x {
            return x;
        }
        x = y;
    }
}

/// An axis-aligned solid box, `[min, max)` in fixed units: one voxel of a `Full` brick, or a
/// whole `Uniform` solid brick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SolidBox {
    pub min: [i64; 3],
    pub max: [i64; 3],
    pub material: MaterialId,
}

/// The ordering key that picks the contact among boxes: squared distance, then the squared
/// distance from the foot point to the box's nearest lattice point, then the voxel holding
/// that point. A `Uniform` brick and its 512 expanded voxels give the same minimum, because
/// the voxel holding the nearest lattice point carries the brick's distance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Key {
    pub d2: i128,
    pub tie: i128,
    pub voxel: VoxelCoord,
}

impl SolidBox {
    pub fn intersects(&self, lo: [i64; 3], hi: [i64; 3]) -> bool {
        (0..3).all(|k| self.min[k] < hi[k] && self.max[k] > lo[k])
    }

    /// `(dx, dy)`: the box's plan rectangle's nearest point to the axis, minus the axis.
    fn plan_delta(&self, b: &Body) -> [i64; 2] {
        [0, 1].map(|k| b.foot.0[k].clamp(self.min[k], self.max[k]) - b.foot.0[k])
    }

    /// The gap between `[foot.z, foot.z + height]` and `[min.z, max.z]`, 0 when they meet.
    fn gap_z(&self, b: &Body) -> i64 {
        let fz = b.foot.0[2];
        (self.min[2] - (fz + b.height)).max(fz - self.max[2]).max(0)
    }

    /// `max(0, d_xy - r)² + d_z²` with `d_xy` the floor of the plan distance: a lower bound
    /// on the squared Euclidean gap between body and box.
    pub fn distance2(&self, b: &Body) -> i128 {
        let [dx, dy] = self.plan_delta(b).map(i128::from);
        let dxy = isqrt((dx * dx + dy * dy) as u128) as i128;
        let t = (dxy - i128::from(b.radius)).max(0);
        let dz = i128::from(self.gap_z(b));
        t * t + dz * dz
    }

    /// Strict interior overlap: inside the radius in plan and overlapping in `z`. Touching is
    /// not overlapping.
    pub fn overlaps(&self, b: &Body) -> bool {
        let [dx, dy] = self.plan_delta(b).map(i128::from);
        let r = i128::from(b.radius);
        let fz = b.foot.0[2];
        dx * dx + dy * dy < r * r && fz < self.max[2] && fz + b.height > self.min[2]
    }

    /// Whether a move along `delta` from the body's position starts to close the gap to this
    /// box. The set of foot positions where the body overlaps the box is convex (the box grown
    /// by the cylinder), so the gap is a convex function along a line: a move that does not
    /// start to close it never closes it, and the box cannot stop that move.
    ///
    /// With the plan gap `t = max(0, d_xy - r)`, `toward = delta_xy . (dx, dy)` (positive
    /// toward the box in plan) and the vertical gap `g_z` closed at the rate `v_z` (`-delta.z`
    /// for a box under the body, `delta.z` for one over it), the move closes on the box iff
    /// `t * toward + g_z * v_z * d_xy > 0`: the rate at which the gap shrinks, times the
    /// positive `d_xy * gap`. A box under
    /// (over) the body's base and inside the radius in plan is closed on only by a move down
    /// (up). A box at floored distance zero (`t == 0`, `g_z == 0`: touching, or less than one
    /// unit away at an irrational plan distance) beside the body is closed on only by a move
    /// toward it in plan that does not leave its `z` range through the face it touches. An
    /// overlapping box is always closed on.
    pub fn closes_on(&self, b: &Body, delta: [i64; 3]) -> bool {
        let [dx, dy] = self.plan_delta(b).map(i128::from);
        let r = i128::from(b.radius);
        let fz = b.foot.0[2];
        let top = fz + b.height;
        let below = self.max[2] <= fz;
        let above = self.min[2] >= top;
        let vz = i128::from(delta[2]);
        let d2 = dx * dx + dy * dy;
        if d2 < r * r {
            return (below && vz < 0) || (above && vz > 0) || (!below && !above);
        }
        let toward = i128::from(delta[0]) * dx + i128::from(delta[1]) * dy;
        // `t > 0` iff the floored plan distance exceeds the radius; the root is taken only
        // when both gaps are open and the sign needs it.
        let t_open = d2 >= (r + 1) * (r + 1);
        let (gz, vz_close) = if below {
            (i128::from(fz - self.max[2]), -vz)
        } else if above {
            (i128::from(self.min[2] - top), vz)
        } else {
            (0, 0)
        };
        match (t_open, gz > 0) {
            (false, false) => toward > 0 && (!below || vz < 0) && (!above || vz > 0),
            (true, false) => toward > 0,
            (false, true) => vz_close > 0,
            (true, true) => {
                let dxy = isqrt(d2 as u128) as i128;
                (dxy - r) * toward + gz * vz_close * dxy > 0
            }
        }
    }

    /// The box's lattice point nearest the foot point.
    fn nearest_point(&self, b: &Body) -> [i64; 3] {
        [0, 1, 2].map(|k| b.foot.0[k].clamp(self.min[k], self.max[k] - 1))
    }

    pub fn key(&self, b: &Body) -> Key {
        let p = self.nearest_point(b);
        let tie = (0..3)
            .map(|k| {
                let d = i128::from(p[k] - b.foot.0[k]);
                d * d
            })
            .sum();
        Key {
            d2: self.distance2(b),
            tie,
            voxel: VoxelCoord(p.map(|v| v.div_euclid(FIXED_ONE) as i32)),
        }
    }

    pub fn contact(&self, b: &Body, voxel: VoxelCoord) -> Contact {
        let fz = b.foot.0[2];
        let vz = if self.max[2] <= fz {
            1
        } else if self.min[2] >= fz + b.height {
            -1
        } else {
            0
        };
        let [dx, dy] = self.plan_delta(b);
        let h = [-dx, -dy];
        let (r, w) = (i128::from(b.radius), [dx, dy].map(i128::from));
        let normal = if vz != 0 && w[0] * w[0] + w[1] * w[1] < r * r {
            [0, 0, vz * b.radius]
        } else {
            [h[0], h[1], 0]
        };
        Contact {
            voxel,
            material: self.material,
            normal,
        }
    }
}

fn read<T>(l: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    l.read().unwrap_or_else(PoisonError::into_inner)
}

/// The solid boxes meeting the half-open fixed-point region `[lo, hi)`, in ascending chunk,
/// brick and voxel order. A `Uniform` solid brick is one box covering the whole brick, read
/// without touching a voxel array; a `Full` brick yields its non-air voxels inside the region
/// and counts one voxel-array read in `stats`. Chunks that are not resident hold no solid.
/// One chunk read lock is held at a time.
pub(crate) fn collect(
    world: &World,
    lo: [i64; 3],
    hi: [i64; 3],
    stats: &mut QueryStats,
) -> Vec<SolidBox> {
    let mut out = Vec::new();
    if (0..3).any(|k| lo[k] >= hi[k]) {
        return out;
    }
    let one = FIXED_ONE;
    let (ce, be) = (i64::from(CHUNK_EDGE), i64::from(BRICK_EDGE));
    let vlo = lo.map(|v| v.div_euclid(one));
    let vhi = hi.map(|v| (v - 1).div_euclid(one));
    let clo = vlo.map(|v| v.div_euclid(ce));
    let chi = vhi.map(|v| v.div_euclid(ce));
    for cx in clo[0]..=chi[0] {
        for cy in clo[1]..=chi[1] {
            for cz in clo[2]..=chi[2] {
                let c = [cx, cy, cz];
                // The sweep bounds a delta to 2^31 fixed units, so chunk indices fit an i32.
                let Some(chunk) = world.store().get(ChunkCoord(c.map(|v| v as i32))) else {
                    continue;
                };
                let chunk = read(&chunk);
                let cv = c.map(|v| v * ce);
                let blo = [0, 1, 2].map(|k| (vlo[k].max(cv[k]) - cv[k]) / be);
                let bhi = [0, 1, 2].map(|k| (vhi[k].min(cv[k] + ce - 1) - cv[k]) / be);
                for bz in blo[2]..=bhi[2] {
                    for by in blo[1]..=bhi[1] {
                        for bx in blo[0]..=bhi[0] {
                            let b = [bx, by, bz];
                            let slot = brick_slot(b.map(|v| v as i32));
                            let bv = [0, 1, 2].map(|k| cv[k] + b[k] * be);
                            match chunk.brick(slot) {
                                Brick::Uniform(AIR) => {}
                                Brick::Uniform(m) => out.push(SolidBox {
                                    min: bv.map(|v| v * one),
                                    max: bv.map(|v| (v + be) * one),
                                    material: *m,
                                }),
                                full @ Brick::Full(_) => {
                                    stats.voxel_arrays_read += 1;
                                    let elo = [0, 1, 2].map(|k| vlo[k].max(bv[k]));
                                    let ehi = [0, 1, 2].map(|k| vhi[k].min(bv[k] + be - 1));
                                    for z in elo[2]..=ehi[2] {
                                        for y in elo[1]..=ehi[1] {
                                            for x in elo[0]..=ehi[0] {
                                                let l = [x - bv[0], y - bv[1], z - bv[2]];
                                                let m =
                                                    full.get(index_in_brick(l.map(|v| v as i32)));
                                                if m != AIR {
                                                    let v = [x, y, z];
                                                    out.push(SolidBox {
                                                        min: v.map(|v| v * one),
                                                        max: v.map(|v| (v + 1) * one),
                                                        material: m,
                                                    });
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isqrt_is_the_floor_of_the_root() {
        for n in 0..10_000u128 {
            let r = isqrt(n);
            assert!(r * r <= n && (r + 1) * (r + 1) > n, "{n}");
        }
        let big = (1u128 << 100) + 12345;
        let r = isqrt(big);
        assert!(r * r <= big && (r + 1) * (r + 1) > big);
        assert_eq!(isqrt(u128::MAX), u64::MAX as u128);
    }
}
