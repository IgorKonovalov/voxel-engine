//! The showcase's integer fields: its noises, the hall's and the chasm's rules, the sides a
//! brick's bounds put it on, and the features ([`Feature`], [`Kind`]).

use super::*;

use vxe_world::hash::ValueNoise;

/// The noises of one source.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Noises {
    pub(crate) wall: ValueNoise,
    pub(crate) floor: ValueNoise,
    pub(crate) vein: ValueNoise,
    pub(crate) chasm: ValueNoise,
}

impl Noises {
    pub(crate) fn new(seed: u64) -> Self {
        Self {
            wall: ValueNoise::new(seed ^ 0x5A0C_A5E0_0000_0001, 5),
            floor: ValueNoise::new(seed ^ 0x5A0C_A5E0_0000_0002, 6),
            vein: ValueNoise::new(seed ^ 0x5A0C_A5E0_0000_0003, 4),
            chasm: ValueNoise::new(seed ^ 0x5A0C_A5E0_0000_0004, 6),
        }
    }
}

/// The noises' corner values for one brick: every lattice holding it is one cell.
pub(crate) struct Fields {
    pub(crate) n: Noises,
    /// The brick's minimum voxel.
    pub(crate) brick: [i32; 3],
    pub(crate) wall: [i64; 8],
    /// The wall noise's cell at the floor's mean height over the brick's footprint.
    pub(crate) wall_floor: [i64; 8],
    pub(crate) floor: [i64; 8],
    pub(crate) vein: [i64; 8],
    pub(crate) chasm: [i64; 8],
}

impl Fields {
    /// The corners of the cells holding the brick with minimum voxel `b`.
    pub(crate) fn of_brick(n: Noises, b: [i32; 3]) -> Self {
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
    pub(crate) fn wall(&self, p: [i32; 3]) -> i64 {
        self.n.wall.interpolate(&self.wall, p) >> 1
    }

    /// The wall noise at the floor's mean height over `(x, y)`.
    pub(crate) fn wall_floor(&self, x: i32, y: i32) -> i64 {
        self.n
            .wall
            .interpolate(&self.wall_floor, [x, y, FLOOR_Z as i32])
            >> 1
    }

    /// The floor's height at `(x, y)`: the highest rock voxel under the hall.
    pub(crate) fn floor(&self, x: i32, y: i32) -> i64 {
        FLOOR_Z + (self.n.floor.interpolate(&self.floor, [x, y, 0]) >> 11)
    }

    pub(crate) fn vein(&self, p: [i32; 3]) -> i64 {
        self.n.vein.interpolate(&self.vein, p)
    }

    /// The chasm's centre line's offset from [`CHASM_X`] at `y`.
    pub(crate) fn chasm(&self, y: i32) -> i64 {
        chasm_offset(self.n.chasm.interpolate(&self.chasm, [0, y, 0]))
    }
}

/// The chasm's offset from a raw noise value: `-13..13`, monotone in the value.
pub(crate) fn chasm_offset(raw: i64) -> i64 {
    (raw * CHASM_WANDER) >> 15
}

/// Where a brick lies against one rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    In,
    Out,
    Mixed,
}

impl Side {
    /// The side of the union of two regions.
    pub(crate) fn or(self, other: Side) -> Side {
        match (self, other) {
            (Side::In, _) | (_, Side::In) => Side::In,
            (Side::Out, Side::Out) => Side::Out,
            _ => Side::Mixed,
        }
    }

    /// The side of the intersection of two regions.
    pub(crate) fn and(self, other: Side) -> Side {
        match (self, other) {
            (Side::Out, _) | (_, Side::Out) => Side::Out,
            (Side::In, Side::In) => Side::In,
            _ => Side::Mixed,
        }
    }

    pub(crate) fn of(all: bool, none: bool) -> Side {
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
pub(crate) struct Bounds {
    /// The floor's heights.
    pub(crate) floor: (i64, i64),
    pub(crate) wall: (i64, i64),
    pub(crate) wall_floor: (i64, i64),
    pub(crate) chasm: (i64, i64),
}

impl Bounds {
    /// Over the brick `lo..=hi` that `f` holds the corners of.
    pub(crate) fn of_brick(f: &Fields, lo: [i32; 3], hi: [i32; 3]) -> Self {
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
    pub(crate) fn of_box(n: &Noises, lo: [i32; 3], hi: [i32; 3]) -> Self {
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
    pub(crate) fn cylinder(&self) -> Option<([i64; 2], i64, [i64; 2])> {
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
