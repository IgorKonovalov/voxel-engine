//! The showcase cavern: one bounded hall under a domed roof over a noisy floor, with pillars,
//! stalactites and stalagmites, two ledges, a chasm and a natural bridge over it, lit by
//! emissive ore in its rock, inside the indestructible shell. Its named camera poses
//! ([`SHOTS`]) frame it for the hero still and the fly-through's `--shot`.
//!
//! Every value is a pure integer function of the seed and the voxel coordinate (`i64` with
//! floored division and integer square roots), so chunks can be generated in any order on any
//! thread and on any platform with identical bytes. The renderer's floats live in `scene.rs`.
//!
//! The hall is the air where `z > floor(x, y)` and `q(p) <= K + n(p)`, with `q` the
//! ellipsoid term `dx^2 K / a^2 + dy^2 K / b^2 + dz^2 K / c^2` around [`CENTRE`] (each term
//! floored on its own), `K = 1 << 16`, and `n` value noise at cell 32 in `-K/4..K/4`. The
//! floor is `FLOOR_Z` plus 2D value noise at cell 64 in `-16..16`. The chasm is the air from
//! [`CHASM_BOTTOM`] up to the floor where `|x - CHASM_X - w(y)| <= CHASM_HALF` (`w` 1D noise
//! at cell 64 in `-13..13`) under the hall's footprint at the floor's mean height.
//!
//! The features (pillars, cones, ledges and the bridge) are a list built once from
//! `hash3(seed, [kind, i, attempt])` with rejection rules, sorted by kind and index. Each is
//! rock where it lies in the hall's or the chasm's air, and each is bucketed by the bricks its
//! bounds touch.
//!
//! Every noise lattice is a multiple of the brick edge and aligned with bricks, so inside one
//! brick each noise is a trilinear function of one cell and lies within one unit of the range
//! its eight corner values span; over a box of several cells it lies within the range of the
//! lattice values of those cells. Each ellipsoid term is monotone in `|d|`, so over a box it
//! takes its extremes at the box's nearest and farthest coordinates. A brick that no feature
//! touches and whose bounds put it wholly on one side of every rule is emitted without
//! evaluating its voxels; the rest are evaluated per voxel against only their bucket, from
//! noise corners hashed once per brick.

use vxe_world::{MaterialId, VoxelCoord};

mod field;
mod source;

pub use field::*;
pub use source::*;

use crate::cave::{ORE, ROCK, SHELL};

/// The showcase's extent in chunks: 896 x 768 x 448 voxels (about 90 x 77 x 45 m).
pub const SHOWCASE_CHUNKS: [i32; 3] = [14, 12, 7];
/// Cyan crystal, the emissive row after the cave's four.
pub const CRYSTAL: MaterialId = 5;
/// Violet amethyst veins in the east walls.
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

/// The chasm's centre line before its noise, its half-width and its lowest air voxel.
pub const CHASM_X: i64 = 598;
pub const CHASM_HALF: i64 = 25;
pub const CHASM_BOTTOM: i64 = 32;
/// The chasm's centre line moves by at most this much either way.
pub const CHASM_WANDER: i64 = 13;
/// The bridge: rock over the chasm for `y` in `BRIDGE_Y` (inclusive) and `z <= BRIDGE_DECK`,
/// above a half-ellipse underside `BRIDGE_HALF_SPAN` either side of [`CHASM_X`] with its
/// springing at `BRIDGE_SPRING` and its crown at `BRIDGE_CROWN`.
pub const BRIDGE_Y: [i64; 2] = [364, 404];
pub const BRIDGE_DECK: i64 = 128;
pub const BRIDGE_CROWN: i64 = 116;
pub const BRIDGE_SPRING: i64 = 86;
pub const BRIDGE_HALF_SPAN: i64 = 40;
/// The ledges' top heights and the directions their arcs face, from the centre.
pub const LEDGES: [(i64, [i64; 2]); 2] = [(188, [-1, 0]), (248, [0, 1])];
/// A ledge's thickness, voxels.
pub const LEDGE_THICKNESS: i64 = 12;
/// The pillars the hall holds, and the least distance between two axes.
pub const PILLARS: usize = 5;
pub const PILLAR_SPACING: i64 = 80;
/// The heights a pillar spans: under the floor's lowest point to over the dome's highest.
pub const PILLAR_Z: [i64; 2] = [96, 400];
/// The edge of the floor cells that hold one stalactite and one stalagmite at most.
pub const CONE_CELL: i32 = 64;
/// The east floor the crystals stand on, `x >= CRYSTAL_FLOOR_X`; no stalagmite reaches it.
pub const CRYSTAL_FLOOR_X: i64 = 650;
/// The clearance every pillar, cone and ledge keeps from each shot's eye, voxels (4 m).
pub const EYE_CLEAR: i64 = 40;
/// How far along each shot's centre ray pillars and cones keep clear of it, voxels.
pub const RAY_CLEAR: i64 = 110;
/// The horizontal distance a pillar's surface keeps from each shot's eye, voxels (12 m), so
/// the shots frame the pillars rather than look into one.
pub const PILLAR_EYE_CLEAR: i64 = 120;

/// No ore lies at or above this height: the dome's upper half over the floor plane is plain
/// rock, so the roof carries no light.
pub const ORE_TOP_Z: i64 = FLOOR_Z + AXES[2] / 2;
/// Vein-noise values above which rock in the amber (west wall) and amethyst (east wall) zones
/// is ore.
const AMBER_ABOVE: i64 = 20000;
const AMETHYST_ABOVE: i64 = 20000;
/// The amber and amethyst zones start this many voxels over the floor (4 m), so the veins
/// light the walls and leave the floor dark.
const WALL_ORE_OVER_FLOOR: i64 = 40;
/// Vein-noise value above which rock in the ember zone is ember.
const EMBER_ABOVE: i64 = 9000;
/// The ember zone reaches this many voxels past the chasm's band and under its bottom.
const EMBER_REACH: i64 = 3;
/// The ember zone's top: the chasm's lowest 24 voxels glow, too deep to light the hall's floor.
pub const EMBER_TOP_Z: i64 = CHASM_BOTTOM + 24;
/// The ember zone's slack on the hall's footprint at the floor, about 4 voxels at its edge.
const EMBER_FOOT: i64 = K / 32;
/// The crystal clusters, the capsules in each, and the least distance between two centres.
pub const CLUSTERS: usize = 8;
pub const CLUSTER_SPACING: i64 = 30;
/// The clearance every crystal voxel keeps from each shot's eye, voxels (2 m).
pub const CRYSTAL_EYE_CLEAR: i64 = 20;
/// Voxels of indestructible shell inside each face of the extent.
const SHELL_THICKNESS: i32 = 2;
/// Rounding slack for the per-brick bounds, in noise units.
const MARGIN: i64 = 4;
/// Candidates the placement draws at most before it gives up on a feature.
const ATTEMPTS: i32 = 10_000;

/// A named camera pose: the eye voxel, yaw and pitch in whole degrees, the exposure in
/// thousandths, and the view's centre ray as a unit vector in thousandths (the yaw and
/// pitch's, so the integer placement can keep features off it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shot {
    pub name: &'static str,
    pub eye: VoxelCoord,
    pub yaw_deg: i32,
    pub pitch_deg: i32,
    pub exposure_milli: u32,
    pub ray_milli: [i32; 3],
}

/// The showcase's shots: `hall` (the spawn), `chasm` and `crystals`.
pub const SHOTS: [Shot; 3] = [
    Shot {
        name: "hall",
        eye: VoxelCoord([190, 384, 218]),
        yaw_deg: 0,
        pitch_deg: -12,
        exposure_milli: 1500,
        ray_milli: [978, 0, -208],
    },
    Shot {
        name: "chasm",
        eye: VoxelCoord([598, 384, 144]),
        yaw_deg: 90,
        pitch_deg: -30,
        exposure_milli: 1500,
        ray_milli: [0, 866, -500],
    },
    Shot {
        name: "crystals",
        eye: VoxelCoord([720, 300, 160]),
        yaw_deg: 160,
        pitch_deg: 18,
        exposure_milli: 1500,
        ray_milli: [-894, 325, 309],
    },
];

impl Shot {
    /// The shot named `name`.
    pub fn named(name: &str) -> Option<Shot> {
        SHOTS.iter().copied().find(|s| s.name == name)
    }

    /// The voxel `t` voxels along the centre ray from the eye.
    pub fn ray_point(&self, t: i64) -> [i64; 3] {
        [0, 1, 2].map(|k| i64::from(self.eye.0[k]) + t * i64::from(self.ray_milli[k]) / 1000)
    }
}

/// `d^2 K / axis^2`, floored: one ellipsoid term.
fn term(d: i64, axis: i64) -> i64 {
    d * d * K / (axis * axis)
}

/// The footprint term `dx^2 K / a^2 + dy^2 K / b^2` at `(x, y)`.
pub fn q_xy(x: i64, y: i64) -> i64 {
    term(x - CENTRE[0], AXES[0]) + term(y - CENTRE[1], AXES[1])
}

/// The ellipsoid term `q` at `p`.
pub fn q(p: [i32; 3]) -> i64 {
    q_xy(i64::from(p[0]), i64::from(p[1])) + term(i64::from(p[2]) - FLOOR_Z, AXES[2])
}

/// The noiseless dome's height over the footprint point whose term is `q_xy`.
pub fn dome_z(q_xy: i64) -> i64 {
    FLOOR_Z + ((K - q_xy).max(0) * AXES[2] * AXES[2] / K).isqrt()
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

/// The squared horizontal distance between two points.
fn d2_xy(a: [i64; 2], b: [i64; 2]) -> i64 {
    (a[0] - b[0]).pow(2) + (a[1] - b[1]).pow(2)
}

#[cfg(test)]
mod tests;
