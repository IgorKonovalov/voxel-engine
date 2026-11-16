//! The bodies (the cylinder and the sphere), their distance to an axis-aligned box, and the
//! contact a box makes with them.
//!
//! Units are fixed point, 1/256 voxel; voxel `v` spans `[256 v, 256 v + 256)`. Squares are
//! taken in `i128`, so no product overflows for any position a `vxe-world` coordinate can name.

use std::sync::{PoisonError, RwLock, RwLockReadGuard};

use vxe_world::{
    AIR, BRICK_EDGE, Brick, CHUNK_EDGE, ChunkCoord, FIXED_ONE, Fixed3, MaterialId, QueryStats,
    VoxelCoord, World, brick_slot,
};

/// A vertical cylinder: its axis is the vertical line through `foot`, it occupies `z` in
/// `[foot.z, foot.z + height)` and horizontal distance `< radius` from the axis (ADR-0005).
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
/// the horizontal (xy) vector from the box's nearest point to the axis; `(0, 0, radius)` for a
/// floor and `(0, 0, -radius)` for a ceiling. A rim contact (the box is vertically separated
/// but its xy rectangle is not within the radius of the axis) counts as a wall.
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

    /// `(dx, dy)`: the box's xy rectangle's nearest point to the axis, minus the axis.
    fn plan_delta(&self, b: &Body) -> [i64; 2] {
        [0, 1].map(|k| b.foot.0[k].clamp(self.min[k], self.max[k]) - b.foot.0[k])
    }

    /// The gap between `[foot.z, foot.z + height]` and `[min.z, max.z]`, 0 when they meet.
    fn gap_z(&self, b: &Body) -> i64 {
        let fz = b.foot.0[2];
        (self.min[2] - (fz + b.height)).max(fz - self.max[2]).max(0)
    }

    /// `max(0, d_xy - r)² + d_z²` with `d_xy` the floor of the horizontal distance: a lower bound
    /// on the squared Euclidean gap between body and box.
    pub fn distance2(&self, b: &Body) -> i128 {
        let [dx, dy] = self.plan_delta(b).map(i128::from);
        let dxy = isqrt((dx * dx + dy * dy) as u128) as i128;
        let t = (dxy - i128::from(b.radius)).max(0);
        let dz = i128::from(self.gap_z(b));
        t * t + dz * dz
    }

    /// Strict interior overlap: inside the radius in xy and overlapping in `z`. Touching is
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
    /// With the horizontal gap `t = max(0, d_xy - r)`, `toward = delta_xy . (dx, dy)` (positive
    /// toward the box in xy) and the vertical gap `g_z` closed at the rate `v_z` (`-delta.z`
    /// for a box under the body, `delta.z` for one over it), the move closes on the box iff
    /// `t * toward + g_z * v_z * d_xy > 0`: the rate at which the gap shrinks, times the
    /// positive `d_xy * gap`. A box under
    /// (over) the body's base and inside the radius in xy is closed on only by a move down
    /// (up). A box at floored distance zero (`t == 0`, `g_z == 0`: touching, or less than one
    /// unit away at an irrational horizontal distance) beside the body is closed on only by a move
    /// toward it in xy that does not leave its `z` range through the face it touches. An
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
        // `t > 0` iff the floored horizontal distance exceeds the radius; the root is taken only
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

/// A sphere: the points within distance `radius` of `centre`, in fixed units. The surface
/// mover's body (ADR-0062).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Sphere {
    pub centre: Fixed3,
    pub radius: i64,
}

impl Sphere {
    /// The same sphere centred at `centre`.
    pub fn at(&self, centre: [i64; 3]) -> Self {
        Self {
            centre: Fixed3(centre),
            ..*self
        }
    }
}

impl SolidBox {
    /// `c - p`, with `p` the point of the closed box `[min, max]` nearest the sphere's centre
    /// `c`: axis-aligned on a face, diagonal on an edge or a corner, zero inside.
    fn sphere_delta(&self, s: &Sphere) -> [i64; 3] {
        let c = s.centre.0;
        [0, 1, 2].map(|k| c[k] - c[k].clamp(self.min[k], self.max[k]))
    }

    /// `max(0, d - r)²` with `d` the floor of the root of the summed per-axis squared gaps:
    /// a lower bound on the squared Euclidean gap between the sphere and the box.
    pub fn sphere_distance2(&self, s: &Sphere) -> i128 {
        let g = self.sphere_delta(s).map(i128::from);
        let d = isqrt((g[0] * g[0] + g[1] * g[1] + g[2] * g[2]) as u128) as i128;
        let t = (d - i128::from(s.radius)).max(0);
        t * t
    }

    /// Strict interior overlap: the box's nearest point closer than the radius. Touching is
    /// not overlapping.
    pub fn sphere_overlaps(&self, s: &Sphere) -> bool {
        let g = self.sphere_delta(s).map(i128::from);
        let r = i128::from(s.radius);
        g[0] * g[0] + g[1] * g[1] + g[2] * g[2] < r * r
    }

    /// Whether a move along `delta` starts to close the gap to this box. The distance from a
    /// point to a convex set is convex along a line, and its derivative at a point outside
    /// the box is `g . delta / |g|` with `g` the vector from the box's nearest point to the
    /// centre: the move closes on the box iff `g . delta < 0`. An overlapping box is always
    /// closed on.
    pub fn sphere_closes_on(&self, s: &Sphere, delta: [i64; 3]) -> bool {
        if self.sphere_overlaps(s) {
            return true;
        }
        let g = self.sphere_delta(s).map(i128::from);
        let d = delta.map(i128::from);
        g[0] * d[0] + g[1] * d[1] + g[2] * d[2] < 0
    }

    /// The ordering key of the box for a sphere, as [`SolidBox::key`] is for a cylinder: the
    /// squared gap, the squared distance from the centre to the box's nearest lattice point,
    /// then the voxel holding that point.
    pub fn sphere_key(&self, s: &Sphere) -> Key {
        let c = s.centre.0;
        let p = [0, 1, 2].map(|k| c[k].clamp(self.min[k], self.max[k] - 1));
        let tie = (0..3)
            .map(|k| {
                let d = i128::from(p[k] - c[k]);
                d * d
            })
            .sum();
        Key {
            d2: self.sphere_distance2(s),
            tie,
            voxel: VoxelCoord(p.map(|v| v.div_euclid(FIXED_ONE) as i32)),
        }
    }

    /// The contact the box makes with a sphere: the normal is the integer vector from the
    /// box's nearest point to the centre (not normalised); `(0, 0, radius)` when the centre is
    /// inside the box.
    pub fn sphere_contact(&self, s: &Sphere, voxel: VoxelCoord) -> Contact {
        let g = self.sphere_delta(s);
        Contact {
            voxel,
            material: self.material,
            normal: if g == [0; 3] { [0, 0, s.radius] } else { g },
        }
    }
}

/// A body the sweep moves: the cylinder or the sphere. Each method is the body's own rule
/// against one box; the sweep's loop is shared, monomorphised per body.
pub(crate) trait Shape: Copy {
    /// The reference point: the cylinder's foot, the sphere's centre.
    fn pos(&self) -> [i64; 3];
    fn moved_to(&self, p: [i64; 3]) -> Self;
    /// The half-open box the body occupies, extended along `delta`.
    fn region(&self, delta: [i64; 3]) -> ([i64; 3], [i64; 3]);
    fn overlaps(&self, b: &SolidBox) -> bool;
    fn closes_on(&self, b: &SolidBox, delta: [i64; 3]) -> bool;
    fn key(&self, b: &SolidBox) -> Key;
    fn contact(&self, b: &SolidBox, voxel: VoxelCoord) -> Contact;
}

impl Shape for Body {
    fn pos(&self) -> [i64; 3] {
        self.foot.0
    }
    fn moved_to(&self, p: [i64; 3]) -> Self {
        self.at(p)
    }
    fn region(&self, delta: [i64; 3]) -> ([i64; 3], [i64; 3]) {
        let f = self.foot.0;
        let lo0 = [f[0] - self.radius, f[1] - self.radius, f[2]];
        let hi0 = [f[0] + self.radius, f[1] + self.radius, f[2] + self.height];
        extend(lo0, hi0, delta)
    }
    fn overlaps(&self, b: &SolidBox) -> bool {
        b.overlaps(self)
    }
    fn closes_on(&self, b: &SolidBox, delta: [i64; 3]) -> bool {
        b.closes_on(self, delta)
    }
    fn key(&self, b: &SolidBox) -> Key {
        b.key(self)
    }
    fn contact(&self, b: &SolidBox, voxel: VoxelCoord) -> Contact {
        b.contact(self, voxel)
    }
}

impl Shape for Sphere {
    fn pos(&self) -> [i64; 3] {
        self.centre.0
    }
    fn moved_to(&self, p: [i64; 3]) -> Self {
        self.at(p)
    }
    fn region(&self, delta: [i64; 3]) -> ([i64; 3], [i64; 3]) {
        let c = self.centre.0;
        extend(
            c.map(|v| v - self.radius),
            c.map(|v| v + self.radius),
            delta,
        )
    }
    fn overlaps(&self, b: &SolidBox) -> bool {
        b.sphere_overlaps(self)
    }
    fn closes_on(&self, b: &SolidBox, delta: [i64; 3]) -> bool {
        b.sphere_closes_on(self, delta)
    }
    fn key(&self, b: &SolidBox) -> Key {
        b.sphere_key(self)
    }
    fn contact(&self, b: &SolidBox, voxel: VoxelCoord) -> Contact {
        b.sphere_contact(self, voxel)
    }
}

/// `[lo, hi)` and its copy moved by `delta`, and everything between.
fn extend(lo0: [i64; 3], hi0: [i64; 3], delta: [i64; 3]) -> ([i64; 3], [i64; 3]) {
    let lo = [0, 1, 2].map(|k| lo0[k].min(lo0[k] + delta[k]));
    let hi = [0, 1, 2].map(|k| hi0[k].max(hi0[k] + delta[k]));
    (lo, hi)
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
    collect_into(world, lo, hi, stats, &mut out);
    out
}

/// [`collect`] appending to `out`.
fn collect_into(
    world: &World,
    lo: [i64; 3],
    hi: [i64; 3],
    stats: &mut QueryStats,
    out: &mut Vec<SolidBox>,
) {
    if (0..3).any(|k| lo[k] >= hi[k]) {
        return;
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
                                    let elo = [0, 1, 2].map(|k| vlo[k].max(bv[k]) - bv[k]);
                                    let ehi = [0, 1, 2].map(|k| vhi[k].min(bv[k] + be - 1) - bv[k]);
                                    // The mask's set bits under the box, in z, y, x order.
                                    full.for_each_solid_in(
                                        elo.map(|v| v as u8),
                                        ehi.map(|v| v as u8),
                                        |i, m| {
                                            let l = [i % 8, i / 8 % 8, i / 64].map(|v| v as i64);
                                            let v = [0, 1, 2].map(|k| bv[k] + l[k]);
                                            out.push(SolidBox {
                                                min: v.map(|v| v * one),
                                                max: v.map(|v| (v + 1) * one),
                                                material: m,
                                            });
                                        },
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The solid boxes of a region, collected once and read by every query inside it.
///
/// The boxes of a region `R` inside the collected one are, in order, the collected boxes that
/// meet `R`: [`collect`]'s order (ascending chunk, brick, voxel) is the same order restricted.
/// A query over the larger list that takes only the boxes meeting its region, and the first
/// box in order of any it looks for, reads what [`collect`] over `R` would have given.
pub(crate) struct Boxes {
    lo: [i64; 3],
    hi: [i64; 3],
    list: Vec<SolidBox>,
    /// The boxes of the last region asked for, taken from `list`.
    sub: Vec<SolidBox>,
    /// Collections made, for tests.
    pub(crate) collections: u64,
}

impl Boxes {
    pub(crate) const fn new() -> Self {
        Self {
            lo: [0; 3],
            hi: [0; 3],
            list: Vec::new(),
            sub: Vec::new(),
            collections: 0,
        }
    }

    /// Collects the boxes of `[lo, hi)` afresh, reusing the list's storage.
    pub(crate) fn collect(&mut self, world: &World, lo: [i64; 3], hi: [i64; 3]) {
        self.list.clear();
        collect_into(world, lo, hi, &mut QueryStats::default(), &mut self.list);
        (self.lo, self.hi) = (lo, hi);
        self.collections += 1;
    }

    /// The boxes meeting `[lo, hi)`, in [`collect`]'s order: [`Boxes::cover`]'s list
    /// restricted to them, so a query scans only its own region's boxes.
    pub(crate) fn select(&mut self, world: &World, lo: [i64; 3], hi: [i64; 3]) -> &[SolidBox] {
        self.cover(world, lo, hi);
        self.sub.clear();
        self.sub
            .extend(self.list.iter().filter(|x| x.intersects(lo, hi)).copied());
        &self.sub
    }

    /// The boxes of a region holding `[lo, hi)`: the held list, or, when `[lo, hi)` reaches
    /// past it, a list collected anew over the smallest box holding both.
    pub(crate) fn cover(&mut self, world: &World, lo: [i64; 3], hi: [i64; 3]) -> &[SolidBox] {
        let empty = (0..3).any(|k| lo[k] >= hi[k]);
        let inside = (0..3).all(|k| lo[k] >= self.lo[k] && hi[k] <= self.hi[k]);
        if !empty && !inside {
            let held = (0..3).all(|k| self.lo[k] < self.hi[k]);
            let (ulo, uhi) = if held {
                (
                    [0, 1, 2].map(|k| lo[k].min(self.lo[k])),
                    [0, 1, 2].map(|k| hi[k].max(self.hi[k])),
                )
            } else {
                (lo, hi)
            };
            self.collect(world, ulo, uhi);
        }
        &self.list
    }
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
