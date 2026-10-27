//! The showcase cavern: one bounded hall under a domed roof over a noisy floor, lit by
//! emissive ore in its rock, inside the indestructible shell. Its named camera poses
//! ([`SHOTS`]) frame it for the hero still and the fly-through's `--shot`.
//!
//! Every value is a pure integer function of the seed and the voxel coordinate (`i64` with
//! floored division), so chunks can be generated in any order on any thread and on any
//! platform with identical bytes. The renderer's floats live in `lib.rs`.
//!
//! The hall is the air where `z > floor(x, y)` and `q(p) <= K + n(p)`, with `q` the
//! ellipsoid term `dx^2 K / a^2 + dy^2 K / b^2 + dz^2 K / c^2` around [`CENTRE`] (each term
//! floored on its own), `K = 1 << 16`, and `n` value noise at cell 32 in `-K/4..K/4`. The
//! floor is `FLOOR_Z` plus 2D value noise at cell 64 in `-16..16`.
//!
//! Every noise lattice is a multiple of the brick edge and aligned with bricks, so inside one
//! brick each noise is a trilinear function of one cell and lies within one unit of the range
//! its eight corner values span. Each ellipsoid term is monotone in `|d|`, so over a box it
//! takes its extremes at the box's nearest and farthest coordinates. A brick whose bounds put
//! it wholly on one side of every rule is emitted without evaluating its voxels; the rest are
//! evaluated per voxel from noise corners hashed once per brick.

use vxe_world::hash::ValueNoise;
use vxe_world::{
    AIR, BRICK_EDGE, Brick, CHUNK_BRICKS, CHUNK_EDGE, Chunk, ChunkCoord, ChunkExtent, MaterialId,
    VoxelCoord, WorldSource, brick_of_slot,
};

use crate::cave::{ORE, ROCK, SHELL};

/// The showcase's extent in chunks: 896 x 768 x 448 voxels (89.6 x 76.8 x 44.8 m).
pub const SHOWCASE_CHUNKS: [i32; 3] = [14, 12, 7];
/// Cyan crystal, the emissive row after the cave's four.
pub const CRYSTAL: MaterialId = 5;
/// Violet amethyst veins in the dome.
pub const AMETHYST: MaterialId = 6;
/// Deep red ember lining the chasm.
pub const EMBER: MaterialId = 7;
/// Amber is the cave's ore row.
pub const AMBER: MaterialId = ORE;

/// The fixed point of the ellipsoid term: `q == K` on the noiseless wall.
pub const K: i64 = 1 << 16;
/// The hall's centre in x and y, voxels.
pub const CENTRE: [i64; 2] = [448, 384];
/// The height `dz` is measured from: the floor's mean.
pub const FLOOR_Z: i64 = 128;
/// The hall's semi-axes `a`, `b` (footprint) and `c` (dome height over `FLOOR_Z`), voxels.
pub const AXES: [i64; 3] = [360, 288, 240];
/// The floor noise's amplitude, voxels: the floor lies in `FLOOR_Z - 16..FLOOR_Z + 16`.
pub const FLOOR_AMPLITUDE: i64 = 16;
/// The top of the amber zone: the lower two thirds of the dome, `dz < 2c/3`.
pub const DOME_THIRD_Z: i64 = FLOOR_Z + 2 * AXES[2] / 3;

/// Vein-noise value above which rock in the amber zone is amber: about 8 % of the noise's
/// values lie above it.
const AMBER_ABOVE: i64 = 14800;
/// Voxels of indestructible shell inside each face of the extent.
const SHELL_THICKNESS: i32 = 2;
/// Rounding slack for the per-brick bounds, in noise units.
const MARGIN: i64 = 4;

/// A named camera pose: the eye voxel, yaw and pitch in whole degrees, and the exposure in
/// thousandths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shot {
    pub name: &'static str,
    pub eye: VoxelCoord,
    pub yaw_deg: i32,
    pub pitch_deg: i32,
    pub exposure_milli: u32,
}

/// The showcase's shots: `hall` (the spawn), `chasm` and `crystals`.
pub const SHOTS: [Shot; 3] = [
    Shot {
        name: "hall",
        eye: VoxelCoord([190, 384, 218]),
        yaw_deg: 0,
        pitch_deg: -12,
        exposure_milli: 1000,
    },
    Shot {
        name: "chasm",
        eye: VoxelCoord([598, 384, 144]),
        yaw_deg: 90,
        pitch_deg: -30,
        exposure_milli: 1000,
    },
    Shot {
        name: "crystals",
        eye: VoxelCoord([720, 300, 160]),
        yaw_deg: 160,
        pitch_deg: 18,
        exposure_milli: 1000,
    },
];

impl Shot {
    /// The shot named `name`.
    pub fn named(name: &str) -> Option<Shot> {
        SHOTS.iter().copied().find(|s| s.name == name)
    }
}

/// `d^2 K / axis^2`, floored: one ellipsoid term.
fn term(d: i64, axis: i64) -> i64 {
    d * d * K / (axis * axis)
}

/// The footprint term `dx^2 K / a^2 + dy^2 K / b^2` at `(x, y)`.
pub fn q_xy(x: i32, y: i32) -> i64 {
    term(i64::from(x) - CENTRE[0], AXES[0]) + term(i64::from(y) - CENTRE[1], AXES[1])
}

/// The ellipsoid term `q` at `p`.
pub fn q(p: [i32; 3]) -> i64 {
    q_xy(p[0], p[1]) + term(i64::from(p[2]) - FLOOR_Z, AXES[2])
}

/// The smallest and largest of `term` over `lo..=hi` on one axis around `c`.
fn term_bounds(lo: i32, hi: i32, c: i64, axis: i64) -> (i64, i64) {
    let (lo, hi) = (i64::from(lo) - c, i64::from(hi) - c);
    let near = if lo <= 0 && hi >= 0 {
        0
    } else {
        lo.abs().min(hi.abs())
    };
    let far = lo.abs().max(hi.abs());
    (term(near, axis), term(far, axis))
}

/// The noises of one source.
#[derive(Clone, Copy, Debug)]
struct Noises {
    wall: ValueNoise,
    floor: ValueNoise,
    vein: ValueNoise,
}

impl Noises {
    fn new(seed: u64) -> Self {
        Self {
            wall: ValueNoise::new(seed ^ 0x5A0C_A5E0_0000_0001, 5),
            floor: ValueNoise::new(seed ^ 0x5A0C_A5E0_0000_0002, 6),
            vein: ValueNoise::new(seed ^ 0x5A0C_A5E0_0000_0003, 4),
        }
    }
}

/// The noises' corner values for one brick: every lattice holding it is one cell.
struct Fields {
    n: Noises,
    /// The brick's minimum voxel.
    brick: [i32; 3],
    wall: [i64; 8],
    floor: [i64; 8],
    vein: [i64; 8],
}

impl Fields {
    /// The corners of the cells holding the brick with minimum voxel `b`.
    fn of_brick(n: Noises, b: [i32; 3]) -> Self {
        Self {
            n,
            brick: b,
            wall: n.wall.corners(n.wall.cell_of(b)),
            floor: n.floor.corners(n.floor.cell_of([b[0], b[1], 0])),
            vein: n.vein.corners(n.vein.cell_of(b)),
        }
    }

    /// The wall noise `n(p)`, `-K/4..K/4`.
    fn wall(&self, p: [i32; 3]) -> i64 {
        self.n.wall.interpolate(&self.wall, p) >> 1
    }

    /// The floor's height at `(x, y)`: the highest rock voxel under the hall.
    fn floor(&self, x: i32, y: i32) -> i64 {
        FLOOR_Z + (self.n.floor.interpolate(&self.floor, [x, y, 0]) >> 11)
    }

    fn vein(&self, p: [i32; 3]) -> i64 {
        self.n.vein.interpolate(&self.vein, p)
    }
}

/// Where a brick lies against one rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    In,
    Out,
    Mixed,
}

/// The showcase cavern of one seed over [`SHOWCASE_CHUNKS`].
#[derive(Clone, Debug)]
pub struct ShowcaseSource {
    pub seed: u64,
    noises: Noises,
}

impl ShowcaseSource {
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            noises: Noises::new(seed),
        }
    }

    /// The extent the source fills: chunks `[0, 0, 0]..SHOWCASE_CHUNKS`.
    pub fn extent(&self) -> ChunkExtent {
        ChunkExtent::new([0, 0, 0], SHOWCASE_CHUNKS)
    }

    /// The spawn: the `hall` shot's eye.
    pub fn spawn(&self) -> VoxelCoord {
        SHOTS[0].eye
    }

    fn in_shell(&self, p: [i32; 3]) -> bool {
        (0..3).any(|k| {
            p[k] < SHELL_THICKNESS || p[k] >= SHOWCASE_CHUNKS[k] * CHUNK_EDGE - SHELL_THICKNESS
        })
    }

    /// Whether `p` is the hall's air: above the floor and inside the noisy ellipsoid.
    fn hall_air(&self, f: &Fields, p: [i32; 3]) -> bool {
        i64::from(p[2]) > f.floor(p[0], p[1]) && q(p) <= K + f.wall(p)
    }

    /// Whether `p` is air before the shell: the hall's.
    fn base_air(&self, f: &Fields, p: [i32; 3]) -> bool {
        self.hall_air(f, p)
    }

    /// Whether voxel `p` is air in the final world, from `f` when `p` lies in its brick and
    /// from fresh corners otherwise.
    fn air_at(&self, f: &Fields, p: [i32; 3]) -> bool {
        if self.in_shell(p) {
            return false;
        }
        if (0..3).all(|k| p[k].div_euclid(BRICK_EDGE) * BRICK_EDGE == f.brick[k]) {
            self.base_air(f, p)
        } else {
            self.base_air(&Fields::of_brick(self.noises, brick_min(p)), p)
        }
    }

    /// The ore rock at `p` takes, before the rule that it borders air: the zone's ore where
    /// the vein noise lies above its threshold, else `None`.
    fn vein_ore(&self, f: &Fields, p: [i32; 3]) -> Option<MaterialId> {
        let [x, _, z] = p.map(i64::from);
        let v = f.vein(p);
        (x < CENTRE[0] && z < DOME_THIRD_Z && z > f.floor(p[0], p[1]) && v > AMBER_ABOVE)
            .then_some(AMBER)
    }

    /// The material of voxel `p` from the brick's fields: ore is rock in its zone, above its
    /// vein threshold, with a six-neighbour of air.
    fn eval(&self, f: &Fields, p: [i32; 3]) -> MaterialId {
        if self.in_shell(p) {
            return SHELL;
        }
        if self.base_air(f, p) {
            return AIR;
        }
        match self.vein_ore(f, p) {
            Some(ore) if self.borders_air(f, p) => ore,
            _ => ROCK,
        }
    }

    fn borders_air(&self, f: &Fields, p: [i32; 3]) -> bool {
        (0..6).any(|i| {
            let mut n = p;
            n[i / 2] += if i % 2 == 0 { -1 } else { 1 };
            self.air_at(f, n)
        })
    }

    /// The final material of voxel `p`.
    pub fn material(&self, p: [i32; 3]) -> MaterialId {
        self.eval(&Fields::of_brick(self.noises, brick_min(p)), p)
    }

    /// The brick with minimum voxel `b`: uniform when its bounds prove it, else evaluated
    /// per voxel.
    pub fn brick(&self, b: [i32; 3]) -> Brick {
        let hi = b.map(|v| v + BRICK_EDGE - 1);
        let f = Fields::of_brick(self.noises, b);
        if let Some(m) = self.uniform(&f, b, hi) {
            return Brick::Uniform(m);
        }
        Brick::from_fn(|l| self.eval(&f, [0, 1, 2].map(|k| b[k] + l[k])))
    }

    /// The material of every voxel of the brick `lo..=hi` when its bounds prove it uniform:
    /// air inside the hall, rock where no voxel of the brick grown by one is air (so none
    /// borders air) or no ore can lie.
    fn uniform(&self, f: &Fields, lo: [i32; 3], hi: [i32; 3]) -> Option<MaterialId> {
        let near_shell = (0..3).any(|k| {
            lo[k] < SHELL_THICKNESS || hi[k] >= SHOWCASE_CHUNKS[k] * CHUNK_EDGE - SHELL_THICKNESS
        });
        if near_shell {
            return None;
        }
        let floor = corner_floor(&f.n.floor, &f.floor, lo, hi);
        let wall = corner_bounds(lo, hi, |p| f.n.wall.interpolate(&f.wall, p));
        match hall_side(lo, hi, floor, wall) {
            Side::In => Some(AIR),
            Side::Mixed => None,
            Side::Out => {
                let (glo, ghi) = (lo.map(|v| v - 1), hi.map(|v| v + 1));
                let grown = hall_side(
                    glo,
                    ghi,
                    lattice_floor(&f.n.floor, glo, ghi),
                    lattice_bounds(&f.n.wall, glo, ghi),
                );
                if grown == Side::Out || !self.ore_possible(f, lo, hi, floor) {
                    Some(ROCK)
                } else {
                    None
                }
            }
        }
    }

    /// Whether some voxel of the solid brick `lo..=hi` can lie in an ore zone above its vein
    /// threshold.
    fn ore_possible(&self, f: &Fields, lo: [i32; 3], hi: [i32; 3], floor: (i64, i64)) -> bool {
        let (_, vein_hi) = corner_bounds(lo, hi, |p| f.vein(p));
        let amber_zone = i64::from(lo[0]) < CENTRE[0]
            && i64::from(lo[2]) < DOME_THIRD_Z
            && i64::from(hi[2]) > floor.0;
        amber_zone && vein_hi > AMBER_ABOVE - MARGIN
    }
}

/// The minimum voxel of the brick holding `p`.
fn brick_min(p: [i32; 3]) -> [i32; 3] {
    p.map(|v| v.div_euclid(BRICK_EDGE) * BRICK_EDGE)
}

/// Where the box `lo..=hi` lies against the hall's air, given the floor's height bounds over
/// its footprint and the wall noise's raw bounds (before the shift to `-K/4..K/4`) over it.
fn hall_side(lo: [i32; 3], hi: [i32; 3], floor: (i64, i64), wall: (i64, i64)) -> Side {
    let (n_lo, n_hi) = ((wall.0 - MARGIN) >> 1, (wall.1 + MARGIN) >> 1);
    let mut q_lo = 0;
    let mut q_hi = 0;
    for (k, c) in [CENTRE[0], CENTRE[1], FLOOR_Z].into_iter().enumerate() {
        let (a, b) = term_bounds(lo[k], hi[k], c, AXES[k]);
        q_lo += a;
        q_hi += b;
    }
    let (z_lo, z_hi) = (i64::from(lo[2]), i64::from(hi[2]));
    if z_lo > floor.1 && q_hi <= K + n_lo {
        Side::In
    } else if z_hi <= floor.0 || q_lo > K + n_hi {
        Side::Out
    } else {
        Side::Mixed
    }
}

/// The floor's height bounds over a footprint from a raw 2D noise range.
fn floor_from(raw: (i64, i64)) -> (i64, i64) {
    (
        FLOOR_Z + ((raw.0 - MARGIN) >> 11),
        FLOOR_Z + ((raw.1 + MARGIN) >> 11),
    )
}

/// The floor's height bounds over the footprint of `lo..=hi`, inside one cell of `noise`
/// whose corners are `corners`.
fn corner_floor(noise: &ValueNoise, corners: &[i64; 8], lo: [i32; 3], hi: [i32; 3]) -> (i64, i64) {
    floor_from(corner_bounds([lo[0], lo[1], 0], [hi[0], hi[1], 0], |p| {
        noise.interpolate(corners, p)
    }))
}

/// The floor's height bounds over the footprint of `lo..=hi`, any number of cells.
fn lattice_floor(noise: &ValueNoise, lo: [i32; 3], hi: [i32; 3]) -> (i64, i64) {
    floor_from(lattice_bounds(noise, [lo[0], lo[1], 0], [hi[0], hi[1], 0]))
}

/// The smallest and largest lattice value of every cell of `noise` the box `lo..=hi`
/// touches: interpolation is a convex combination of them, so the noise over the box lies in
/// that range up to its one unit of rounding.
fn lattice_bounds(noise: &ValueNoise, lo: [i32; 3], hi: [i32; 3]) -> (i64, i64) {
    let (a, b) = (noise.cell_of(lo), noise.cell_of(hi));
    let mut r = (i64::MAX, i64::MIN);
    for z in a[2]..=b[2] + 1 {
        for y in a[1]..=b[1] + 1 {
            for x in a[0]..=b[0] + 1 {
                let v = noise.lattice([x, y, z]);
                r = (r.0.min(v), r.1.max(v));
            }
        }
    }
    r
}

/// The smallest and largest of `f` over the eight corners of `lo..=hi`.
fn corner_bounds(lo: [i32; 3], hi: [i32; 3], f: impl Fn([i32; 3]) -> i64) -> (i64, i64) {
    (0..8).fold((i64::MAX, i64::MIN), |(a, b), i| {
        let c = [0, 1, 2].map(|k| if i >> k & 1 == 0 { lo[k] } else { hi[k] });
        let v = f(c);
        (a.min(v), b.max(v))
    })
}

impl WorldSource for ShowcaseSource {
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

    /// The worked points of the hall: air in the middle, rock under the floor's lowest point,
    /// rock past the noise's reach in x and z, the shell at the face, and air at `hall`'s eye.
    #[test]
    fn the_hall_is_the_worked_shape() {
        let s = ShowcaseSource::new(1);
        assert_eq!(s.material([448, 384, 200]), AIR);
        assert_eq!(s.material([448, 384, 100]), ROCK);
        assert_ne!(s.material([40, 384, 200]), AIR);
        assert_ne!(s.material([448, 384, 400]), AIR);
        assert!(matches!(s.material([40, 384, 200]), ROCK | AMBER));
        assert_eq!(s.material([448, 384, 400]), ROCK);
        assert_eq!(s.material([0, 384, 200]), SHELL);
        assert_eq!(s.material(SHOTS[0].eye.0), AIR);
        assert_eq!(s.spawn(), SHOTS[0].eye);
        assert_eq!(Shot::named("chasm").map(|s| s.eye), Some(SHOTS[1].eye));
        assert_eq!(Shot::named("keep"), None);
    }

    /// The early-out never disagrees with evaluating every voxel, over a 12^3 brick sample
    /// spanning the west wall, the floor and the shell, and is doing work.
    #[test]
    fn the_brick_early_out_matches_per_voxel_evaluation() {
        let s = ShowcaseSource::new(1);
        let mut uniform = 0;
        let mut full = 0;
        // x 0..96 crosses the shell and the west wall; z 96..192 the floor and the air.
        for (origin, step) in [([0, 340, 96], 8), ([40, 300, 100], 24)] {
            for bz in 0..12 {
                for by in 0..12 {
                    for bx in 0..12 {
                        let b = [0, 1, 2].map(|k| origin[k] + [bx, by, bz][k] * step);
                        let b = b.map(|v: i32| v - v.rem_euclid(BRICK_EDGE));
                        let fast = s.brick(b);
                        let slow = Brick::from_fn(|l| s.material([0, 1, 2].map(|k| b[k] + l[k])));
                        assert_eq!(fast, slow, "brick at {b:?}");
                        if fast.is_full() {
                            full += 1;
                        } else {
                            uniform += 1;
                        }
                    }
                }
            }
        }
        assert!(full > 0, "the sample crosses a surface");
        assert!(uniform > full, "{uniform} uniform, {full} full");
    }
}
