//! The showcase cavern: one bounded hall under a domed roof over a noisy floor, with pillars,
//! stalactites and stalagmites, two ledges, a chasm and a natural bridge over it, lit by
//! emissive ore in its rock, inside the indestructible shell. Its named camera poses
//! ([`SHOTS`]) frame it for the hero still and the fly-through's `--shot`.
//!
//! Every value is a pure integer function of the seed and the voxel coordinate (`i64` with
//! floored division and integer square roots), so chunks can be generated in any order on any
//! thread and on any platform with identical bytes. The renderer's floats live in `lib.rs`.
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

use std::collections::BTreeMap;

use vxe_world::hash::{ValueNoise, hash3};
use vxe_world::{
    AIR, BRICK_EDGE, Brick, CHUNK_BRICKS, CHUNK_EDGE, Chunk, ChunkCoord, ChunkExtent, MaterialId,
    VoxelCoord, WorldSource, brick_of_slot,
};

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

/// The noises of one source.
#[derive(Clone, Copy, Debug)]
struct Noises {
    wall: ValueNoise,
    floor: ValueNoise,
    vein: ValueNoise,
    chasm: ValueNoise,
}

impl Noises {
    fn new(seed: u64) -> Self {
        Self {
            wall: ValueNoise::new(seed ^ 0x5A0C_A5E0_0000_0001, 5),
            floor: ValueNoise::new(seed ^ 0x5A0C_A5E0_0000_0002, 6),
            vein: ValueNoise::new(seed ^ 0x5A0C_A5E0_0000_0003, 4),
            chasm: ValueNoise::new(seed ^ 0x5A0C_A5E0_0000_0004, 6),
        }
    }
}

/// The noises' corner values for one brick: every lattice holding it is one cell.
struct Fields {
    n: Noises,
    /// The brick's minimum voxel.
    brick: [i32; 3],
    wall: [i64; 8],
    /// The wall noise's cell at the floor's mean height over the brick's footprint.
    wall_floor: [i64; 8],
    floor: [i64; 8],
    vein: [i64; 8],
    chasm: [i64; 8],
}

impl Fields {
    /// The corners of the cells holding the brick with minimum voxel `b`.
    fn of_brick(n: Noises, b: [i32; 3]) -> Self {
        Self {
            n,
            brick: b,
            wall: n.wall.corners(n.wall.cell_of(b)),
            wall_floor: n.wall.corners(n.wall.cell_of([b[0], b[1], FLOOR_Z as i32])),
            floor: n.floor.corners(n.floor.cell_of([b[0], b[1], 0])),
            vein: n.vein.corners(n.vein.cell_of(b)),
            chasm: n.chasm.corners(n.chasm.cell_of([0, b[1], 0])),
        }
    }

    /// The wall noise `n(p)`, `-K/4..K/4`.
    fn wall(&self, p: [i32; 3]) -> i64 {
        self.n.wall.interpolate(&self.wall, p) >> 1
    }

    /// The wall noise at the floor's mean height over `(x, y)`.
    fn wall_floor(&self, x: i32, y: i32) -> i64 {
        self.n
            .wall
            .interpolate(&self.wall_floor, [x, y, FLOOR_Z as i32])
            >> 1
    }

    /// The floor's height at `(x, y)`: the highest rock voxel under the hall.
    fn floor(&self, x: i32, y: i32) -> i64 {
        FLOOR_Z + (self.n.floor.interpolate(&self.floor, [x, y, 0]) >> 11)
    }

    fn vein(&self, p: [i32; 3]) -> i64 {
        self.n.vein.interpolate(&self.vein, p)
    }

    /// The chasm's centre line's offset from [`CHASM_X`] at `y`.
    fn chasm(&self, y: i32) -> i64 {
        chasm_offset(self.n.chasm.interpolate(&self.chasm, [0, y, 0]))
    }
}

/// The chasm's offset from a raw noise value: `-13..13`, monotone in the value.
fn chasm_offset(raw: i64) -> i64 {
    (raw * CHASM_WANDER) >> 15
}

/// Where a brick lies against one rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    In,
    Out,
    Mixed,
}

impl Side {
    /// The side of the union of two regions.
    fn or(self, other: Side) -> Side {
        match (self, other) {
            (Side::In, _) | (_, Side::In) => Side::In,
            (Side::Out, Side::Out) => Side::Out,
            _ => Side::Mixed,
        }
    }

    /// The side of the intersection of two regions.
    fn and(self, other: Side) -> Side {
        match (self, other) {
            (Side::Out, _) | (_, Side::Out) => Side::Out,
            (Side::In, Side::In) => Side::In,
            _ => Side::Mixed,
        }
    }

    fn of(all: bool, none: bool) -> Side {
        if all {
            Side::In
        } else if none {
            Side::Out
        } else {
            Side::Mixed
        }
    }
}

/// The raw noise ranges over a box, before each rule's shift.
struct Bounds {
    /// The floor's heights.
    floor: (i64, i64),
    wall: (i64, i64),
    wall_floor: (i64, i64),
    chasm: (i64, i64),
}

impl Bounds {
    /// Over the brick `lo..=hi` that `f` holds the corners of.
    fn of_brick(f: &Fields, lo: [i32; 3], hi: [i32; 3]) -> Self {
        let z = FLOOR_Z as i32;
        Self {
            floor: floor_from(corner_bounds([lo[0], lo[1], 0], [hi[0], hi[1], 0], |p| {
                f.n.floor.interpolate(&f.floor, p)
            })),
            wall: corner_bounds(lo, hi, |p| f.n.wall.interpolate(&f.wall, p)),
            wall_floor: corner_bounds([lo[0], lo[1], z], [hi[0], hi[1], z], |p| {
                f.n.wall.interpolate(&f.wall_floor, p)
            }),
            chasm: corner_bounds([0, lo[1], 0], [0, hi[1], 0], |p| {
                f.n.chasm.interpolate(&f.chasm, p)
            }),
        }
    }

    /// Over any box `lo..=hi`.
    fn of_box(n: &Noises, lo: [i32; 3], hi: [i32; 3]) -> Self {
        let z = FLOOR_Z as i32;
        Self {
            floor: floor_from(lattice_bounds(
                &n.floor,
                [lo[0], lo[1], 0],
                [hi[0], hi[1], 0],
            )),
            wall: lattice_bounds(&n.wall, lo, hi),
            wall_floor: lattice_bounds(&n.wall, [lo[0], lo[1], z], [hi[0], hi[1], z]),
            chasm: lattice_bounds(&n.chasm, [0, lo[1], 0], [0, hi[1], 0]),
        }
    }
}

/// A feature of the hall, rock where it lies in air.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feature {
    /// A column from under the floor to over the roof: radius `r_end` at the floor's mean
    /// and the noiseless dome over its axis, `r_mid` at `mid` between them, quadratic in
    /// between (`half` is half the span).
    Pillar {
        axis: [i64; 2],
        r_end: i64,
        r_mid: i64,
        mid: i64,
        half: i64,
    },
    /// A stalactite (`tip < base`) or a stalagmite (`tip > base`): a cone from its tip to
    /// radius `r_base` at `base`, continued as a cylinder of that radius to `root` in the rock
    /// it hangs from or stands on.
    Cone {
        axis: [i64; 2],
        tip: i64,
        base: i64,
        root: i64,
        r_base: i64,
    },
    /// A shelf `LEDGE_THICKNESS` thick under `z_top`, over the 90 degree arc facing `dir`:
    /// the hall's air within `depth` voxels of the wall, measured outward from the centre.
    Ledge {
        z_top: i64,
        dir: [i64; 2],
        depth: i64,
    },
    /// The natural arch over the chasm (`BRIDGE_*`).
    Bridge,
    /// A crystal prism: [`CRYSTAL`] within `r` of the segment from `a` to `b`, wherever it
    /// lies.
    Capsule { a: [i64; 3], b: [i64; 3], r: i64 },
}

impl Feature {
    /// The feature's kind, its rank in the list's order.
    pub fn kind(&self) -> Kind {
        match self {
            Feature::Pillar { .. } => Kind::Pillar,
            Feature::Cone { tip, base, .. } if tip < base => Kind::Stalactite,
            Feature::Cone { .. } => Kind::Stalagmite,
            Feature::Ledge { .. } => Kind::Ledge,
            Feature::Bridge => Kind::Bridge,
            Feature::Capsule { .. } => Kind::Crystal,
        }
    }

    /// The vertical cylinder that bounds a pillar or a cone: axis, radius and heights.
    fn cylinder(&self) -> Option<([i64; 2], i64, [i64; 2])> {
        match *self {
            Feature::Pillar { axis, r_end, .. } => Some((axis, r_end, PILLAR_Z)),
            Feature::Cone {
                axis,
                tip,
                root,
                r_base,
                ..
            } => Some((axis, r_base, [tip.min(root), tip.max(root)])),
            _ => None,
        }
    }
}

/// The kinds of feature, in the list's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Pillar,
    Stalactite,
    Stalagmite,
    Ledge,
    Bridge,
    Crystal,
}

/// The showcase cavern of one seed over [`SHOWCASE_CHUNKS`].
#[derive(Clone, Debug)]
pub struct ShowcaseSource {
    pub seed: u64,
    noises: Noises,
    /// Sorted by kind, then by index.
    features: Vec<Feature>,
    /// Each feature's bounds, inclusive.
    boxes: Vec<([i32; 3], [i32; 3])>,
    /// Brick minimum voxel to the features whose bounds touch it.
    buckets: BTreeMap<[i32; 3], Vec<u32>>,
}

/// Which features a voxel is evaluated against.
#[derive(Clone, Copy)]
enum Scope<'a> {
    /// Every feature: the reference evaluation.
    All,
    /// The brick's bucket.
    Bucket(&'a [u32]),
}

impl ShowcaseSource {
    pub fn new(seed: u64) -> Self {
        let mut s = Self {
            seed,
            noises: Noises::new(seed),
            features: Vec::new(),
            boxes: Vec::new(),
            buckets: BTreeMap::new(),
        };
        s.place();
        s
    }

    /// The extent the source fills: chunks `[0, 0, 0]..SHOWCASE_CHUNKS`.
    pub fn extent(&self) -> ChunkExtent {
        ChunkExtent::new([0, 0, 0], SHOWCASE_CHUNKS)
    }

    /// The spawn: the `hall` shot's eye.
    pub fn spawn(&self) -> VoxelCoord {
        SHOTS[0].eye
    }

    /// The features, sorted by kind, then by index.
    pub fn features(&self) -> &[Feature] {
        &self.features
    }

    /// The chasm's centre line's offset from [`CHASM_X`] at `y`, `-13..13`.
    pub fn chasm_offset(&self, y: i32) -> i64 {
        chasm_offset(self.noises.chasm.sample([0, y, 0]))
    }

    /// The floor's height at `(x, y)`.
    pub fn floor_at(&self, x: i32, y: i32) -> i64 {
        self.fields_at([x, y, 0]).floor(x, y)
    }

    fn fields_at(&self, p: [i32; 3]) -> Fields {
        Fields::of_brick(self.noises, brick_min(p))
    }

    /// The voxel two metres in from the wall at the middle of ledge `i`'s arc, at its top
    /// height: the first voxel from the centre along the arc's direction that is not the
    /// hall's air, less 20 voxels.
    pub fn ledge_probe(&self, i: usize) -> [i32; 3] {
        let (z, dir) = LEDGES[i];
        let mut r = 0;
        loop {
            let p = [CENTRE[0] + dir[0] * r, CENTRE[1] + dir[1] * r, z].map(|v| v as i32);
            if !self.hall_air(&self.fields_at(p), p) {
                break;
            }
            r += 1;
        }
        let r = r - 20;
        [CENTRE[0] + dir[0] * r, CENTRE[1] + dir[1] * r, z].map(|v| v as i32)
    }

    /// Builds the feature list: the ledges, the pillars, the cones and the bridge, each
    /// kept clear of what came before it, then sorts it and buckets it by brick.
    fn place(&mut self) {
        let ledges: Vec<Feature> = LEDGES
            .iter()
            .enumerate()
            .map(|(i, &(z_top, dir))| {
                let h = hash3(self.seed, [Kind::Ledge as i32, i as i32, 0]);
                Feature::Ledge {
                    z_top,
                    dir,
                    depth: 30 + (h % 21) as i64,
                }
            })
            .collect();
        self.features = ledges;
        let probes: Vec<[i64; 2]> = (0..LEDGES.len())
            .map(|i| {
                let p = self.ledge_probe(i);
                [i64::from(p[0]), i64::from(p[1])]
            })
            .collect();
        let pillars = self.place_pillars(&probes);
        self.features.extend(pillars);
        let cones = self.place_cones(&probes);
        self.features.extend(cones);
        self.features.push(Feature::Bridge);
        let crystals = self.place_crystals();
        self.features.extend(crystals);
        // Stable: within a kind the order of placement, which is the index order.
        self.features.sort_by_key(Feature::kind);
        self.boxes = (0..self.features.len())
            .map(|i| self.bounds_of(i))
            .collect();
        self.buckets = BTreeMap::new();
        for i in 0..self.features.len() {
            for b in self.bricks_of(i) {
                self.buckets.entry(b).or_default().push(i as u32);
            }
        }
    }

    /// Whether a vertical cylinder (`axis`, radius `r`, heights `z`) keeps `clear` voxels from
    /// each shot's eye and two voxels from the first [`RAY_CLEAR`] voxels of its centre ray.
    fn clear_of_shots(axis: [i64; 2], r: i64, z: [i64; 2]) -> bool {
        SHOTS.iter().all(|s| {
            let eye = s.eye.0.map(i64::from);
            let near_eye = d2_xy(axis, [eye[0], eye[1]]) < (r + EYE_CLEAR).pow(2)
                && eye[2] >= z[0] - EYE_CLEAR
                && eye[2] <= z[1] + EYE_CLEAR;
            let on_ray = (0..=RAY_CLEAR).any(|t| {
                let p = s.ray_point(t);
                d2_xy(axis, [p[0], p[1]]) < (r + 2).pow(2) && p[2] >= z[0] - 2 && p[2] <= z[1] + 2
            });
            !near_eye && !on_ray
        })
    }

    /// Five pillars: axes where `10 q_xy <= 6 K`, at least [`PILLAR_SPACING`] apart, 3 m from
    /// the chasm's reach, [`PILLAR_EYE_CLEAR`] from each shot's eye, clear of the shots' rays
    /// and of the ledges' probes, their mid-height probe off a ledge.
    fn place_pillars(&self, probes: &[[i64; 2]]) -> Vec<Feature> {
        let mut out: Vec<Feature> = Vec::new();
        let (ra, rb) = (AXES[0] * 775 / 1000, AXES[1] * 775 / 1000);
        let mut i = 0;
        while out.len() < PILLARS {
            assert!(i < ATTEMPTS, "the showcase places {PILLARS} pillars");
            let h = hash3(self.seed, [Kind::Pillar as i32, i, 0]);
            i += 1;
            let axis = [
                CENTRE[0] - ra + (h % (2 * ra as u64 + 1)) as i64,
                CENTRE[1] - rb + ((h >> 20) % (2 * rb as u64 + 1)) as i64,
            ];
            let r_end = 20 + ((h >> 40) % 21) as i64;
            let r_mid = r_end * 3 / 5;
            let q = q_xy(axis[0], axis[1]);
            let top = dome_z(q);
            let (mid, half) = ((FLOOR_Z + top) / 2, (top - FLOOR_Z) / 2);
            let p = Feature::Pillar {
                axis,
                r_end,
                r_mid,
                mid,
                half,
            };
            let spaced = out.iter().all(|o| match o {
                Feature::Pillar { axis: a, .. } => d2_xy(*a, axis) >= PILLAR_SPACING.pow(2),
                _ => true,
            });
            let off_chasm = (axis[0] - CHASM_X).abs() >= CHASM_HALF + CHASM_WANDER + r_end + 30;
            let off_probes = probes
                .iter()
                .all(|&pr| d2_xy(pr, axis) >= (r_end + 10).pow(2));
            let mid_probe = [axis[0] + r_mid + 1, axis[1], mid].map(|v| v as i32);
            let framed = SHOTS.iter().all(|s| {
                let e = [i64::from(s.eye.0[0]), i64::from(s.eye.0[1])];
                d2_xy(axis, e) >= (r_end + PILLAR_EYE_CLEAR).pow(2)
            });
            let ok = q * 10 <= K * 6
                && framed
                && spaced
                && off_chasm
                && off_probes
                && Self::clear_of_shots(axis, r_end, PILLAR_Z)
                && !self.in_ledge(mid_probe);
            if ok {
                out.push(p);
            }
        }
        out
    }

    /// Whether voxel `p` lies in one of the ledges placed so far.
    fn in_ledge(&self, p: [i32; 3]) -> bool {
        let f = self.fields_at(p);
        self.features
            .iter()
            .any(|ft| matches!(ft, Feature::Ledge { .. }) && self.feature_rock(ft, &f, p))
    }

    /// The highest of the hall's air over the floor at `(x, y)`, scanning up from the floor;
    /// `None` where the floor has no air over it.
    fn roof_at(&self, x: i64, y: i64) -> Option<i64> {
        let (x, y) = (x as i32, y as i32);
        let mut z = self.floor_at(x, y) as i32 + 1;
        let mut top = None;
        while z < SHOWCASE_CHUNKS[2] * CHUNK_EDGE {
            let p = [x, y, z];
            if !self.hall_air(&self.fields_at(p), p) {
                break;
            }
            top = Some(i64::from(z));
            z += 1;
        }
        top
    }

    /// One stalactite with probability 1/2 and one stalagmite with probability 1/3 per
    /// [`CONE_CELL`] floor cell whose centre has `10 q_xy <= 8 K`, each inside its cell, 1 m
    /// clear of every pillar and of the ledges, clear of the shots; stalactite tips keep 5 m
    /// over the floor and a stalagmite's tip 1 m under its cell's stalactite's; stalagmites
    /// keep 3 m off the chasm's reach and off the crystal floor.
    fn place_cones(&self, probes: &[[i64; 2]]) -> Vec<Feature> {
        let pillars: Vec<([i64; 2], i64)> = self
            .features
            .iter()
            .filter_map(|f| match *f {
                Feature::Pillar { axis, r_end, .. } => Some((axis, r_end)),
                _ => None,
            })
            .collect();
        let mut stalactites = Vec::new();
        let mut stalagmites = Vec::new();
        let cells = [SHOWCASE_CHUNKS[0], SHOWCASE_CHUNKS[1]].map(|c| c * CHUNK_EDGE / CONE_CELL);
        let edge = i64::from(CONE_CELL);
        for j in 0..cells[1] {
            for i in 0..cells[0] {
                let cell = [i64::from(i) * edge, i64::from(j) * edge];
                if q_xy(cell[0] + edge / 2, cell[1] + edge / 2) * 10 > K * 8 {
                    continue;
                }
                let idx = j * cells[0] + i;
                let clear = |axis: [i64; 2], r: i64, z: [i64; 2], tips: [[i64; 3]; 2]| {
                    pillars
                        .iter()
                        .all(|&(a, pr)| d2_xy(a, axis) >= (r + pr + 10).pow(2))
                        && probes.iter().all(|&pr| d2_xy(pr, axis) >= (r + 10).pow(2))
                        && Self::clear_of_shots(axis, r, z)
                        && tips.iter().all(|t| !self.in_ledge(t.map(|v| v as i32)))
                };
                // The draw for each kind, then placement attempts until one is clear.
                let draw = |kind: Kind, a: i32| hash3(self.seed, [kind as i32, idx, a]);
                let place = |h: u64| {
                    let r = 6 + ((h >> 8) % 9) as i64;
                    let span = (edge - 2 * r + 1) as u64;
                    let axis = [
                        cell[0] + r + ((h >> 24) % span) as i64,
                        cell[1] + r + ((h >> 40) % span) as i64,
                    ];
                    (r, axis, (h >> 16) & 0xFF)
                };
                let mut hung = None;
                if draw(Kind::Stalactite, 0) & 1 == 0 {
                    for a in 0..8 {
                        let (r, axis, len) = place(draw(Kind::Stalactite, a));
                        let Some(roof) = self.roof_at(axis[0], axis[1]) else {
                            continue;
                        };
                        let floor = self.floor_at(axis[0] as i32, axis[1] as i32);
                        let tip = (roof - 40 - (len % 81) as i64).max(floor + 50);
                        if roof - tip < 20 {
                            continue;
                        }
                        let (base, root) = (roof + 1, roof + 41);
                        let tips = [[axis[0], axis[1], tip], [axis[0], axis[1], tip - 1]];
                        if clear(axis, r, [tip, root], tips) {
                            hung = Some(tip);
                            stalactites.push(Feature::Cone {
                                axis,
                                tip,
                                base,
                                root,
                                r_base: r,
                            });
                            break;
                        }
                    }
                }
                if draw(Kind::Stalagmite, 0) % 3 == 1 {
                    for a in 0..8 {
                        let (r, axis, len) = place(draw(Kind::Stalagmite, a));
                        let off_chasm =
                            (axis[0] - CHASM_X).abs() >= CHASM_HALF + CHASM_WANDER + r + 30;
                        if !off_chasm || axis[0] + r >= CRYSTAL_FLOOR_X {
                            continue;
                        }
                        let base = self.floor_at(axis[0] as i32, axis[1] as i32);
                        let mut tip = base + 20 + (len % 41) as i64;
                        if let Some(over) = hung {
                            tip = tip.min(over - 10);
                        }
                        if tip - base < 20 {
                            continue;
                        }
                        let root = base - 30;
                        let tips = [[axis[0], axis[1], tip], [axis[0], axis[1], tip + 1]];
                        if clear(axis, r, [root, tip], tips) {
                            stalagmites.push(Feature::Cone {
                                axis,
                                tip,
                                base,
                                root,
                                r_base: r,
                            });
                            break;
                        }
                    }
                }
            }
        }
        stalactites.extend(stalagmites);
        stalactites
    }

    /// Eight clusters of three to six crystal prisms (radius 2..4, length 16..36, a hashed
    /// tilt) standing in the east floor at `x` 690..749 where `10 q_xy <= 7 K`: centres
    /// [`CLUSTER_SPACING`] apart and in distinct chunks, 4 m clear of every pillar's reach,
    /// every prism [`CRYSTAL_EYE_CLEAR`] from each shot's eye and two voxels off the first
    /// [`RAY_CLEAR`] voxels of its centre ray.
    fn place_crystals(&self) -> Vec<Feature> {
        let pillars: Vec<([i64; 2], i64)> = self
            .features
            .iter()
            .filter_map(|f| match *f {
                Feature::Pillar { axis, r_end, .. } => Some((axis, r_end)),
                _ => None,
            })
            .collect();
        let mut centres: Vec<[i64; 3]> = Vec::new();
        let mut out = Vec::new();
        let mut i = 0;
        while centres.len() < CLUSTERS {
            assert!(
                i < ATTEMPTS,
                "the showcase places {CLUSTERS} crystal clusters"
            );
            let h = hash3(self.seed, [Kind::Crystal as i32, i, 0]);
            let cluster = i;
            i += 1;
            let c = [690 + (h % 60) as i64, 241 + ((h >> 16) % 287) as i64];
            if q_xy(c[0], c[1]) * 10 > K * 7 {
                continue;
            }
            let centre = [c[0], c[1], self.floor_at(c[0] as i32, c[1] as i32)];
            let chunk = centre.map(|v| v.div_euclid(i64::from(CHUNK_EDGE)));
            let apart = centres.iter().all(|o| {
                o.map(|v| v.div_euclid(i64::from(CHUNK_EDGE))) != chunk
                    && d2_xy([o[0], o[1]], c) >= CLUSTER_SPACING.pow(2)
            });
            let off_pillars = pillars
                .iter()
                .all(|&(a, r)| d2_xy(a, c) >= (r + EYE_CLEAR).pow(2));
            if !apart || !off_pillars {
                continue;
            }
            let prisms: Vec<Feature> = (0..3 + (h >> 32) % 4)
                .map(|k| {
                    let h = hash3(self.seed, [Kind::Crystal as i32, cluster, 1 + k as i32]);
                    let r = 2 + (h % 3) as i64;
                    let len = 16 + ((h >> 8) % 21) as i64;
                    let jitter = |shift: u32| ((h >> shift) % 21) as i64 - 10;
                    let tilt = |shift: u32| ((h >> shift) % (len as u64 + 1)) as i64 - len / 2;
                    let foot = [c[0] + jitter(16), c[1] + jitter(24)];
                    let a = [
                        foot[0],
                        foot[1],
                        self.floor_at(foot[0] as i32, foot[1] as i32) - 4,
                    ];
                    let b = [a[0] + tilt(32), a[1] + tilt(44), a[2] + len];
                    Feature::Capsule { a, b, r }
                })
                .collect();
            let clear = prisms.iter().all(|p| {
                let Feature::Capsule { a, b, r } = *p else {
                    return false;
                };
                SHOTS.iter().all(|s| {
                    let eye = s.eye.0.map(i64::from);
                    !in_capsule(a, b, r + CRYSTAL_EYE_CLEAR, eye)
                        && (0..=RAY_CLEAR).all(|t| !in_capsule(a, b, r + 2, s.ray_point(t)))
                })
            });
            if clear {
                centres.push(centre);
                out.extend(prisms);
            }
        }
        out
    }

    /// Feature `i`'s bounds, inclusive.
    fn bounds_of(&self, i: usize) -> ([i32; 3], [i32; 3]) {
        let f = &self.features[i];
        if let Some((axis, r, z)) = f.cylinder() {
            return (
                [axis[0] - r, axis[1] - r, z[0]].map(|v| v as i32),
                [axis[0] + r, axis[1] + r, z[1]].map(|v| v as i32),
            );
        }
        match *f {
            Feature::Bridge => (
                [CHASM_X - BRIDGE_HALF_SPAN, BRIDGE_Y[0], BRIDGE_SPRING].map(|v| v as i32),
                [CHASM_X + BRIDGE_HALF_SPAN, BRIDGE_Y[1], BRIDGE_DECK].map(|v| v as i32),
            ),
            Feature::Capsule { a, b, r } => (
                [0, 1, 2].map(|k| (a[k].min(b[k]) - r) as i32),
                [0, 1, 2].map(|k| (a[k].max(b[k]) + r) as i32),
            ),
            _ => {
                let bricks = self.bricks_of(i);
                let lo = bricks
                    .iter()
                    .fold([i32::MAX; 3], |a, b| [0, 1, 2].map(|k| a[k].min(b[k])));
                let hi = bricks.iter().fold([i32::MIN; 3], |a, b| {
                    [0, 1, 2].map(|k| a[k].max(b[k] + BRICK_EDGE - 1))
                });
                (lo, hi)
            }
        }
    }

    /// The bricks (minimum voxels) feature `i` can hold rock in.
    fn bricks_of(&self, i: usize) -> Vec<[i32; 3]> {
        let f = self.features[i];
        let (lo, hi) = match f {
            Feature::Ledge { z_top, .. } => (
                [0, 0, (z_top - LEDGE_THICKNESS + 1) as i32],
                [
                    SHOWCASE_CHUNKS[0] * CHUNK_EDGE - 1,
                    SHOWCASE_CHUNKS[1] * CHUNK_EDGE - 1,
                    z_top as i32,
                ],
            ),
            _ => self.boxes[i],
        };
        let (lo, hi) = (brick_min(lo), brick_min(hi));
        let mut out = Vec::new();
        for z in (lo[2]..=hi[2]).step_by(BRICK_EDGE as usize) {
            for y in (lo[1]..=hi[1]).step_by(BRICK_EDGE as usize) {
                for x in (lo[0]..=hi[0]).step_by(BRICK_EDGE as usize) {
                    let b = [x, y, z];
                    if !matches!(f, Feature::Ledge { .. }) || self.ledge_brick(&f, b) {
                        out.push(b);
                    }
                }
            }
        }
        out
    }

    /// Whether ledge `f` can hold rock in the brick with minimum voxel `b`: the brick reaches
    /// into the arc's wedge, holds some of the hall's air, and the brick grown outward by the
    /// ledge's depth holds some rock.
    fn ledge_brick(&self, f: &Feature, b: [i32; 3]) -> bool {
        let Feature::Ledge { dir, depth, .. } = *f else {
            return false;
        };
        let hi = b.map(|v| v + BRICK_EDGE - 1);
        // The wedge is `dot >= |cross|`: both `dot - cross` and `dot + cross` non-negative,
        // linear forms whose largest values lie at the box's corners.
        let reaches = |sign: i64| {
            (0..4).any(|c| {
                let x = i64::from(if c & 1 == 0 { b[0] } else { hi[0] }) - CENTRE[0];
                let y = i64::from(if c & 2 == 0 { b[1] } else { hi[1] }) - CENTRE[1];
                x * dir[0] + y * dir[1] + sign * (x * dir[1] - y * dir[0]) >= 0
            })
        };
        if !reaches(1) || !reaches(-1) {
            return false;
        }
        let air = hall_side(b, hi, &Bounds::of_box(&self.noises, b, hi));
        let g = (depth + 1) as i32;
        let (glo, ghi) = ([b[0] - g, b[1] - g, b[2]], [hi[0] + g, hi[1] + g, hi[2]]);
        let grown = hall_side(glo, ghi, &Bounds::of_box(&self.noises, glo, ghi));
        air != Side::Out && grown != Side::In
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

    /// Whether `(x, y)` lies in the chasm's band.
    fn in_band(&self, f: &Fields, x: i32, y: i32) -> bool {
        (i64::from(x) - CHASM_X - f.chasm(y)).abs() <= CHASM_HALF
    }

    /// Whether `p` is the chasm's air: in its band, from its bottom up to the floor, under
    /// the hall's footprint at the floor's mean height.
    fn chasm_air(&self, f: &Fields, p: [i32; 3]) -> bool {
        let [x, y, z] = p;
        let z = i64::from(z);
        z >= CHASM_BOTTOM
            && self.in_band(f, x, y)
            && z <= f.floor(x, y)
            && q_xy(i64::from(x), i64::from(y)) <= K + f.wall_floor(x, y)
    }

    /// Whether `p` is air before the shell and the features: the hall's or the chasm's.
    fn base_air(&self, f: &Fields, p: [i32; 3]) -> bool {
        self.hall_air(f, p) || self.chasm_air(f, p)
    }

    /// Whether feature `ft` is rock at `p` (`f` the fields of `p`'s brick), where `p` lies in
    /// air.
    fn feature_rock(&self, ft: &Feature, f: &Fields, p: [i32; 3]) -> bool {
        let [x, y, z] = p.map(i64::from);
        match *ft {
            Feature::Pillar {
                axis,
                r_end,
                r_mid,
                mid,
                half,
            } => {
                let dz = (z - mid).abs().min(half);
                let r = r_mid + (r_end - r_mid) * dz * dz / (half * half);
                z >= PILLAR_Z[0] && z <= PILLAR_Z[1] && d2_xy(axis, [x, y]) <= r * r
            }
            Feature::Cone {
                axis,
                tip,
                base,
                root,
                r_base,
            } => {
                if z < tip.min(root) || z > tip.max(root) {
                    return false;
                }
                let d2 = d2_xy(axis, [x, y]);
                let (along, span) = ((z - tip).abs(), (base - tip).abs());
                if along <= span {
                    d2 * span * span <= r_base * r_base * along * along
                } else {
                    d2 <= r_base * r_base
                }
            }
            Feature::Ledge { z_top, dir, depth } => {
                if z > z_top || z <= z_top - LEDGE_THICKNESS {
                    return false;
                }
                let (dx, dy) = (x - CENTRE[0], y - CENTRE[1]);
                let dot = dx * dir[0] + dy * dir[1];
                if dot <= 0 || dot < (dx * dir[1] - dy * dir[0]).abs() {
                    return false;
                }
                if !self.hall_air(f, p) {
                    return false;
                }
                let near_eye = SHOTS.iter().any(|s| {
                    let e = s.eye.0.map(i64::from);
                    (0..3).map(|k| (e[k] - [x, y, z][k]).pow(2)).sum::<i64>() < EYE_CLEAR.pow(2)
                });
                if near_eye {
                    return false;
                }
                let r = (dx * dx + dy * dy).isqrt();
                let out = [
                    CENTRE[0] + dx * (r + depth) / r,
                    CENTRE[1] + dy * (r + depth) / r,
                    z,
                ]
                .map(|v| v as i32);
                !self.hall_air(&self.fields_at(out), out)
            }
            Feature::Bridge => {
                let dx = x - CHASM_X;
                let rise = BRIDGE_CROWN - BRIDGE_SPRING;
                let s2 = BRIDGE_HALF_SPAN * BRIDGE_HALF_SPAN;
                y >= BRIDGE_Y[0]
                    && y <= BRIDGE_Y[1]
                    && (BRIDGE_SPRING..=BRIDGE_DECK).contains(&z)
                    && dx.abs() <= BRIDGE_HALF_SPAN
                    && self.in_band(f, p[0], p[1])
                    && (z - BRIDGE_SPRING).pow(2) * s2 >= rise * rise * (s2 - dx * dx)
            }
            Feature::Capsule { .. } => false,
        }
    }

    /// Whether a crystal of `scope` holds `p`.
    fn crystal_at(&self, scope: Scope<'_>, p: [i32; 3]) -> bool {
        let hit = |i: usize| {
            let (lo, hi) = self.boxes[i];
            match self.features[i] {
                Feature::Capsule { a, b, r } => {
                    (0..3).all(|k| p[k] >= lo[k] && p[k] <= hi[k])
                        && in_capsule(a, b, r, p.map(i64::from))
                }
                _ => false,
            }
        };
        match scope {
            Scope::All => (0..self.features.len()).any(hit),
            Scope::Bucket(b) => b.iter().any(|&i| hit(i as usize)),
        }
    }

    /// Whether a stalactite of `scope` is rock at `p`.
    fn in_stalactite(&self, scope: Scope<'_>, f: &Fields, p: [i32; 3]) -> bool {
        let hit = |i: usize| {
            let (lo, hi) = self.boxes[i];
            let ft = &self.features[i];
            ft.kind() == Kind::Stalactite
                && (0..3).all(|k| p[k] >= lo[k] && p[k] <= hi[k])
                && self.feature_rock(ft, f, p)
        };
        match scope {
            Scope::All => (0..self.features.len()).any(hit),
            Scope::Bucket(b) => b.iter().any(|&i| hit(i as usize)),
        }
    }

    /// Whether a feature of `scope` is rock at `p`.
    fn rock_feature(&self, scope: Scope<'_>, f: &Fields, p: [i32; 3]) -> bool {
        let hit = |i: usize| {
            let (lo, hi) = self.boxes[i];
            (0..3).all(|k| p[k] >= lo[k] && p[k] <= hi[k])
                && self.feature_rock(&self.features[i], f, p)
        };
        match scope {
            Scope::All => (0..self.features.len()).any(hit),
            Scope::Bucket(b) => b.iter().any(|&i| hit(i as usize)),
        }
    }

    /// The bucket of the brick with minimum voxel `b`.
    fn bucket(&self, b: [i32; 3]) -> &[u32] {
        self.buckets.get(&b).map_or(&[], Vec::as_slice)
    }

    /// Whether voxel `p` is air in the final world (`f` the fields of the brick `scope` was
    /// drawn for), from `f` when `p` lies in that brick and from its own brick otherwise.
    fn air_at(&self, f: &Fields, scope: Scope<'_>, p: [i32; 3]) -> bool {
        if self.in_shell(p) {
            return false;
        }
        let b = brick_min(p);
        if b == f.brick {
            !self.crystal_at(scope, p) && self.base_air(f, p) && !self.rock_feature(scope, f, p)
        } else {
            let g = Fields::of_brick(self.noises, b);
            let scope = match scope {
                Scope::All => Scope::All,
                Scope::Bucket(_) => Scope::Bucket(self.bucket(b)),
            };
            !self.crystal_at(scope, p) && self.base_air(&g, p) && !self.rock_feature(scope, &g, p)
        }
    }

    /// The ore rock at `p` takes, before the rule that it borders air: the zone's ore where
    /// the vein noise lies above its threshold, else `None`. Every zone lies under
    /// [`ORE_TOP_Z`]: amber in the west walls over the floor, ember around the chasm, amethyst
    /// in the east walls over the floor.
    fn vein_ore(&self, f: &Fields, p: [i32; 3]) -> Option<MaterialId> {
        let [x, _, z] = p.map(i64::from);
        if z >= ORE_TOP_Z {
            return None;
        }
        let v = f.vein(p);
        let over_floor = z > f.floor(p[0], p[1]) + WALL_ORE_OVER_FLOOR;
        if x < CENTRE[0] && over_floor {
            (v > AMBER_ABOVE).then_some(AMBER)
        } else if v > EMBER_ABOVE && self.ember_zone(f, p) {
            Some(EMBER)
        } else if x >= CENTRE[0] && over_floor {
            (v > AMETHYST_ABOVE).then_some(AMETHYST)
        } else {
            None
        }
    }

    /// Whether rock at `p` lies in the ember zone: within [`EMBER_REACH`] of the chasm's band
    /// and bottom, under the floor's mean height, under the hall's footprint with
    /// [`EMBER_FOOT`] of slack.
    fn ember_zone(&self, f: &Fields, p: [i32; 3]) -> bool {
        let [x, y, z] = p;
        let zz = i64::from(z);
        (CHASM_BOTTOM - EMBER_REACH..EMBER_TOP_Z).contains(&zz)
            && (i64::from(x) - CHASM_X - f.chasm(y)).abs() <= CHASM_HALF + EMBER_REACH
            && q_xy(i64::from(x), i64::from(y)) <= K + f.wall_floor(x, y) + EMBER_FOOT
    }

    /// The material of voxel `p` from its brick's fields against `scope`'s features: ore is
    /// rock in its zone, above its vein threshold, outside every stalactite, with air beside
    /// it or above it (air only below is a ceiling face, which stays rock).
    fn eval(&self, f: &Fields, scope: Scope<'_>, p: [i32; 3]) -> MaterialId {
        if self.in_shell(p) {
            return SHELL;
        }
        if self.crystal_at(scope, p) {
            // A crystal voxel lit only from below would light the floor as a ceiling face.
            let mut below = p;
            below[2] -= 1;
            let under_only = self.air_at(f, scope, below) && !self.faces_air(f, scope, p);
            return if under_only { ROCK } else { CRYSTAL };
        }
        if self.base_air(f, p) && !self.rock_feature(scope, f, p) {
            return AIR;
        }
        match self.vein_ore(f, p) {
            Some(ore) if self.faces_air(f, scope, p) && !self.in_stalactite(scope, f, p) => ore,
            _ => ROCK,
        }
    }

    /// Whether a six-neighbour of `p` other than the one below is air.
    fn faces_air(&self, f: &Fields, scope: Scope<'_>, p: [i32; 3]) -> bool {
        (0..6).filter(|&i| i != 4).any(|i| {
            let mut n = p;
            n[i / 2] += if i % 2 == 0 { -1 } else { 1 };
            self.air_at(f, scope, n)
        })
    }

    /// The final material of voxel `p`, against every feature.
    pub fn material(&self, p: [i32; 3]) -> MaterialId {
        self.eval(&self.fields_at(p), Scope::All, p)
    }

    /// The brick with minimum voxel `b`: uniform when its bounds prove it, else evaluated
    /// per voxel against its bucket.
    pub fn brick(&self, b: [i32; 3]) -> Brick {
        let hi = b.map(|v| v + BRICK_EDGE - 1);
        let f = Fields::of_brick(self.noises, b);
        let bucket = self.bucket(b);
        if let Some(m) = self.uniform(&f, bucket, b, hi) {
            return Brick::Uniform(m);
        }
        Brick::from_fn(|l| self.eval(&f, Scope::Bucket(bucket), [0, 1, 2].map(|k| b[k] + l[k])))
    }

    /// The material of every voxel of the brick `lo..=hi` when its bounds prove it uniform:
    /// air where it lies in the hall's or the chasm's air and no feature touches it, rock
    /// where no voxel of the brick grown by one is air (so none borders air) or no ore can
    /// lie. The features only turn air to rock, so they leave a solid brick solid.
    fn uniform(
        &self,
        f: &Fields,
        bucket: &[u32],
        lo: [i32; 3],
        hi: [i32; 3],
    ) -> Option<MaterialId> {
        let near_shell = (0..3).any(|k| {
            lo[k] < SHELL_THICKNESS || hi[k] >= SHOWCASE_CHUNKS[k] * CHUNK_EDGE - SHELL_THICKNESS
        });
        if near_shell {
            return None;
        }
        let crystal = bucket
            .iter()
            .any(|&i| self.features[i as usize].kind() == Kind::Crystal);
        if crystal {
            return None;
        }
        let bounds = Bounds::of_brick(f, lo, hi);
        match base_side(lo, hi, &bounds) {
            Side::In => bucket.is_empty().then_some(AIR),
            Side::Mixed => None,
            Side::Out => {
                let (glo, ghi) = (lo.map(|v| v - 1), hi.map(|v| v + 1));
                let grown = base_side(glo, ghi, &Bounds::of_box(&self.noises, glo, ghi));
                if grown == Side::Out || !self.ore_possible(f, lo, hi, &bounds) {
                    Some(ROCK)
                } else {
                    None
                }
            }
        }
    }

    /// Whether some voxel of the solid brick `lo..=hi` can lie in an ore zone above its vein
    /// threshold.
    fn ore_possible(&self, f: &Fields, lo: [i32; 3], hi: [i32; 3], b: &Bounds) -> bool {
        let (_, vein) = corner_bounds(lo, hi, |p| f.vein(p));
        let (z_lo, z_hi) = (i64::from(lo[2]), i64::from(hi[2]));
        let walls = z_lo < ORE_TOP_Z && z_hi > b.floor.0 + WALL_ORE_OVER_FLOOR;
        let amethyst = walls && i64::from(hi[0]) >= CENTRE[0] && vein > AMETHYST_ABOVE - MARGIN;
        let amber = walls && i64::from(lo[0]) < CENTRE[0] && vein > AMBER_ABOVE - MARGIN;
        let w = (
            chasm_offset(b.chasm.0 - MARGIN),
            chasm_offset(b.chasm.1 + MARGIN),
        );
        let reach = CHASM_HALF + EMBER_REACH;
        let band_out =
            i64::from(hi[0]) < CHASM_X + w.0 - reach || i64::from(lo[0]) > CHASM_X + w.1 + reach;
        let height_out = z_hi < CHASM_BOTTOM - EMBER_REACH || z_lo >= EMBER_TOP_Z;
        let foot_out = q_xy_bounds(lo, hi).0 > K + wall_from(b.wall_floor).1 + EMBER_FOOT;
        let ember = !band_out && !height_out && !foot_out && vein > EMBER_ABOVE - MARGIN;
        amethyst || amber || ember
    }
}

/// Whether `p` lies within `r` of the segment from `a` to `b`: with `d = b - a`, `w = p - a`
/// and `t` the clamped `w . d` over `L2 = d . d`, `|w L2 - d t|^2 <= r^2 L2^2`.
fn in_capsule(a: [i64; 3], b: [i64; 3], r: i64, p: [i64; 3]) -> bool {
    let d = [0, 1, 2].map(|k| b[k] - a[k]);
    let w = [0, 1, 2].map(|k| p[k] - a[k]);
    let l2: i64 = d.iter().map(|v| v * v).sum();
    if l2 == 0 {
        return w.iter().map(|v| v * v).sum::<i64>() <= r * r;
    }
    let t = (0..3).map(|k| w[k] * d[k]).sum::<i64>().clamp(0, l2);
    let e: i64 = (0..3).map(|k| (w[k] * l2 - d[k] * t).pow(2)).sum();
    e <= r * r * l2 * l2
}

/// The minimum voxel of the brick holding `p`.
fn brick_min(p: [i32; 3]) -> [i32; 3] {
    p.map(|v| v.div_euclid(BRICK_EDGE) * BRICK_EDGE)
}

/// The footprint term's bounds over the box's footprint.
fn q_xy_bounds(lo: [i32; 3], hi: [i32; 3]) -> (i64, i64) {
    let (a0, b0) = term_bounds(lo[0], hi[0], CENTRE[0], AXES[0]);
    let (a1, b1) = term_bounds(lo[1], hi[1], CENTRE[1], AXES[1]);
    (a0 + a1, b0 + b1)
}

/// The wall noise's bounds in `-K/4..K/4` from its raw range.
fn wall_from(raw: (i64, i64)) -> (i64, i64) {
    ((raw.0 - MARGIN) >> 1, (raw.1 + MARGIN) >> 1)
}

/// Where the box `lo..=hi` lies against the hall's air.
fn hall_side(lo: [i32; 3], hi: [i32; 3], b: &Bounds) -> Side {
    let (n_lo, n_hi) = wall_from(b.wall);
    let (qxy_lo, qxy_hi) = q_xy_bounds(lo, hi);
    let (qz_lo, qz_hi) = term_bounds(lo[2], hi[2], FLOOR_Z, AXES[2]);
    let (z_lo, z_hi) = (i64::from(lo[2]), i64::from(hi[2]));
    Side::of(
        z_lo > b.floor.1 && qxy_hi + qz_hi <= K + n_lo,
        z_hi <= b.floor.0 || qxy_lo + qz_lo > K + n_hi,
    )
}

/// Where the box `lo..=hi` lies against the chasm's air.
fn chasm_side(lo: [i32; 3], hi: [i32; 3], b: &Bounds) -> Side {
    let w = (
        chasm_offset(b.chasm.0 - MARGIN),
        chasm_offset(b.chasm.1 + MARGIN),
    );
    let (x_lo, x_hi) = (i64::from(lo[0]), i64::from(hi[0]));
    let band = Side::of(
        x_lo >= CHASM_X + w.1 - CHASM_HALF && x_hi <= CHASM_X + w.0 + CHASM_HALF,
        x_hi < CHASM_X + w.0 - CHASM_HALF || x_lo > CHASM_X + w.1 + CHASM_HALF,
    );
    let (z_lo, z_hi) = (i64::from(lo[2]), i64::from(hi[2]));
    let height = Side::of(
        z_lo >= CHASM_BOTTOM && z_hi <= b.floor.0,
        z_hi < CHASM_BOTTOM || z_lo > b.floor.1,
    );
    let (n_lo, n_hi) = wall_from(b.wall_floor);
    let (qxy_lo, qxy_hi) = q_xy_bounds(lo, hi);
    let foot = Side::of(qxy_hi <= K + n_lo, qxy_lo > K + n_hi);
    band.and(height).and(foot)
}

/// Where the box `lo..=hi` lies against the air before the features.
fn base_side(lo: [i32; 3], hi: [i32; 3], b: &Bounds) -> Side {
    hall_side(lo, hi, b).or(chasm_side(lo, hi, b))
}

/// The floor's height bounds over a footprint from a raw 2D noise range.
fn floor_from(raw: (i64, i64)) -> (i64, i64) {
    (
        FLOOR_Z + ((raw.0 - MARGIN) >> 11),
        FLOOR_Z + ((raw.1 + MARGIN) >> 11),
    )
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

    fn solid(m: MaterialId) -> bool {
        m != AIR && m != SHELL
    }

    /// The worked points of the hall: air in the middle, rock under the floor's lowest point,
    /// rock past the noise's reach in x and z, the shell at the face, and air at `hall`'s eye.
    #[test]
    fn the_hall_is_the_worked_shape() {
        let s = ShowcaseSource::new(1);
        assert_eq!(s.material([448, 384, 200]), AIR);
        assert_eq!(s.material([448, 384, 100]), ROCK);
        assert!(matches!(s.material([40, 384, 200]), ROCK | AMBER));
        assert_eq!(s.material([448, 384, 400]), ROCK);
        assert_eq!(s.material([0, 384, 200]), SHELL);
        assert_eq!(s.material(SHOTS[0].eye.0), AIR);
        assert_eq!(s.spawn(), SHOTS[0].eye);
        assert_eq!(Shot::named("chasm").map(|s| s.eye), Some(SHOTS[1].eye));
        assert_eq!(Shot::named("keep"), None);
    }

    /// Compares `brick` with per-voxel evaluation against every feature; true when the brick
    /// is `Full`.
    fn early_out_agrees(s: &ShowcaseSource, b: [i32; 3]) -> bool {
        let fast = s.brick(b);
        let slow = Brick::from_fn(|l| s.material([0, 1, 2].map(|k| b[k] + l[k])));
        assert_eq!(fast, slow, "brick at {b:?}");
        fast.is_full()
    }

    /// The early-out never disagrees with evaluating every voxel: over a 12^3 brick sample
    /// spanning the west wall, the floor and the shell, a coarser one across the west half,
    /// and bricks that touch each kind of feature. Each kind's sample holds `Full` bricks.
    #[test]
    fn the_brick_early_out_matches_per_voxel_evaluation() {
        let s = ShowcaseSource::new(1);
        let (mut uniform, mut full) = (0, 0);
        // x 0..96 crosses the shell and the west wall; z 96..192 the floor and the air.
        for (origin, step) in [([0, 340, 96], 8), ([40, 300, 100], 24)] {
            for bz in 0..12 {
                for by in 0..12 {
                    for bx in 0..12 {
                        let b = [0, 1, 2].map(|k| origin[k] + [bx, by, bz][k] * step);
                        if early_out_agrees(&s, brick_min(b)) {
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
        let mut by_kind: BTreeMap<Kind, Vec<[i32; 3]>> = BTreeMap::new();
        for (b, list) in &s.buckets {
            for &i in list {
                by_kind
                    .entry(s.features()[i as usize].kind())
                    .or_default()
                    .push(*b);
            }
        }
        assert_eq!(by_kind.len(), 6, "every kind is bucketed");
        for (kind, bricks) in by_kind {
            let step = bricks.len() / 48 + 1;
            let full = bricks
                .iter()
                .step_by(step)
                .filter(|&&b| early_out_agrees(&s, b))
                .count();
            println!(
                "{kind:?}: {} bricks bucketed, {full} full in the sample",
                bricks.len()
            );
            assert!(full > 0, "{kind:?}");
        }
    }

    /// Seed 1's features follow their rules, and each shows at its worked voxels.
    #[test]
    fn seed_1_places_the_features_by_their_rules() {
        let s = ShowcaseSource::new(1);
        let of = |k: Kind| -> Vec<Feature> {
            s.features()
                .iter()
                .filter(|f| f.kind() == k)
                .copied()
                .collect()
        };
        let kinds: Vec<Kind> = s.features().iter().map(Feature::kind).collect();
        assert!(kinds.is_sorted(), "sorted by kind");
        let pillars = of(Kind::Pillar);
        assert_eq!(pillars.len(), 5);
        for (i, a) in pillars.iter().enumerate() {
            let Feature::Pillar {
                axis, r_mid, mid, ..
            } = *a
            else {
                unreachable!()
            };
            for b in &pillars[i + 1..] {
                let Feature::Pillar { axis: other, .. } = *b else {
                    unreachable!()
                };
                assert!(d2_xy(axis, other) >= 80 * 80, "{axis:?} {other:?}");
            }
            let at = |dx: i64| [axis[0] + dx, axis[1], mid].map(|v| v as i32);
            assert!(solid(s.material(at(0))), "pillar {axis:?} at mid-height");
            assert_eq!(s.material(at(r_mid + 1)), AIR, "beside pillar {axis:?}");
        }
        let (hung, standing) = (of(Kind::Stalactite), of(Kind::Stalagmite));
        println!(
            "{} pillars, {} stalactites, {} stalagmites, {} ledges",
            pillars.len(),
            hung.len(),
            standing.len(),
            of(Kind::Ledge).len()
        );
        assert!(hung.len() >= 16, "{} stalactites", hung.len());
        assert!(standing.len() >= 8, "{} stalagmites", standing.len());
        for c in hung.iter().chain(&standing) {
            let Feature::Cone {
                axis, tip, base, ..
            } = *c
            else {
                unreachable!()
            };
            let past = tip + (tip - base).signum();
            let at = |z: i64| [axis[0], axis[1], z].map(|v| v as i32);
            assert!(solid(s.material(at(tip))), "cone tip {:?}", at(tip));
            assert_eq!(s.material(at(past)), AIR, "past the tip {:?}", at(past));
        }
        let x = (CHASM_X + s.chasm_offset(384)) as i32;
        assert_eq!(s.material([x, 384, 32]), AIR, "the chasm's bottom");
        assert!(solid(s.material([x, 384, 31])), "under the chasm");
        assert!(solid(s.material([598, 384, 127])), "the bridge's deck");
        assert_eq!(s.material([598, 384, 129]), AIR, "over the deck");
        assert_eq!(s.material([598, 384, 90]), AIR, "under the arch");
        for i in 0..LEDGES.len() {
            let p = s.ledge_probe(i);
            assert!(solid(s.material(p)), "ledge {i} at {p:?}");
            assert_eq!(s.material([p[0], p[1], p[2] + 1]), AIR, "over ledge {i}");
        }
    }

    /// No pillar, cone or ledge voxel lies within 4 m of a shot's eye. The bridge is the
    /// `chasm` shot's subject, 16 voxels under its eye.
    #[test]
    fn no_feature_lies_within_four_metres_of_a_shot() {
        let s = ShowcaseSource::new(1);
        let r = EYE_CLEAR as i32;
        for shot in SHOTS {
            let e = shot.eye.0;
            let (lo, hi) = (brick_min(e.map(|v| v - r)), brick_min(e.map(|v| v + r)));
            let mut near = 0;
            for bz in (lo[2]..=hi[2]).step_by(BRICK_EDGE as usize) {
                for by in (lo[1]..=hi[1]).step_by(BRICK_EDGE as usize) {
                    for bx in (lo[0]..=hi[0]).step_by(BRICK_EDGE as usize) {
                        let f = Fields::of_brick(s.noises, [bx, by, bz]);
                        for l in 0..BRICK_EDGE.pow(3) {
                            let p = [
                                bx + l % BRICK_EDGE,
                                by + l / BRICK_EDGE % BRICK_EDGE,
                                bz + l / (BRICK_EDGE * BRICK_EDGE),
                            ];
                            let d2: i32 = (0..3).map(|k| (p[k] - e[k]).pow(2)).sum();
                            if d2 >= r * r {
                                continue;
                            }
                            near +=
                                usize::from(s.features().iter().any(|ft| {
                                    ft.kind() != Kind::Bridge && s.feature_rock(ft, &f, p)
                                }));
                        }
                    }
                }
            }
            assert_eq!(
                near, 0,
                "{} feature voxels within 4 m of {}",
                near, shot.name
            );
        }
    }
}
