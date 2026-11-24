//! The outdoor testbed: integer hills of grass over soil over rock, caverns from the cave's
//! noise deep under them, and a tunnel from a hillside mouth down to a roofed chamber. Every
//! value is an integer function of the seed and the voxel coordinate, so chunks generate in
//! any order on any thread with identical bytes. The engine sees voxels and materials only.
//!
//! The surface height of column `(x, y)` is `BASE_Z + (A1 n9 + A2 n7 + A3 n5) >> SHIFT`, `nk`
//! the value noise of cell `2^k` at `(x, y, 0)`: hills of about +-20 m. The top voxel is
//! [`GRASS`], the [`SOIL_DEPTH`] below it [`SOIL`], then rock; the cave's density carves
//! caverns only more than [`CAVE_DEPTH`] voxels under the surface, and none within
//! [`CHAMBER_CLEAR`] voxels in xy of the chamber's column or [`TUNNEL_CLEAR`] of the tunnel's
//! axis, so no cavern opens the roof or a second way in. The tunnel is a capsule of radius
//! [`TUNNEL_RADIUS`] from the surface at [`MOUTH`] to the chamber's centre, [`CHAMBER_RUN`]
//! voxels along +x and [`CHAMBER_DEPTH`] under the surface there, a slope of about 1 in 4.

use svet_world::hash::ValueNoise;
use svet_world::{
    AIR, BRICK_EDGE, Brick, CHUNK_BRICKS, CHUNK_EDGE, Chunk, ChunkCoord, MaterialId, VoxelCoord,
    WorldSource, brick_of_slot,
};

use crate::cave::{CaveSource, ROCK};

/// The surface's mean height in voxels.
pub const BASE_Z: i32 = 512;
/// The amplitudes of the cell-512, cell-128 and cell-32 octaves before the shift.
const AMPLITUDE: [i64; 3] = [140, 50, 10];
const SHIFT: u32 = 15;
/// The outdoor rows, appended after the cave's ([`crate::outdoor_materials`]).
pub const GRASS: MaterialId = 5;
pub const SOIL: MaterialId = 6;
/// The trees' rows ([`crate::trees`]), appended after the soil.
pub const BARK: MaterialId = 7;
pub const LEAF: MaterialId = 8;
/// The trees' other leaf rows, after the first: a yellower and a bluer green.
pub const LEAF_2: MaterialId = 9;
pub const LEAF_3: MaterialId = 10;
/// The leaf rows a tree variant picks from ([`crate::trees::leaf_row`]).
pub const LEAF_ROWS: [MaterialId; 3] = [LEAF, LEAF_2, LEAF_3];

/// Whether `m` is one of [`LEAF_ROWS`].
pub fn is_leaf(m: MaterialId) -> bool {
    LEAF_ROWS.contains(&m)
}
/// Soil voxels under the grass.
pub const SOIL_DEPTH: i32 = 8;
/// Caverns open only deeper than this under the surface.
pub const CAVE_DEPTH: i32 = 40;
/// The mouth's column.
pub const MOUTH: [i32; 2] = [0, 0];
/// The tunnel's radius in voxels.
pub const TUNNEL_RADIUS: i64 = 25;
/// The chamber's radius in voxels.
pub const CHAMBER_RADIUS: i64 = 60;
/// The chamber's centre: this many voxels along +x from the mouth's column ...
pub const CHAMBER_RUN: i32 = 600;
/// ... and this many under the surface of its own column.
pub const CHAMBER_DEPTH: i32 = 150;
/// No cavern within this many voxels in xy of the chamber's centre column.
pub const CHAMBER_CLEAR: i64 = 100;
/// No cavern within this many voxels of the tunnel's axis.
pub const TUNNEL_CLEAR: i64 = 60;
/// The tunnel's axis starts this far over the mouth's surface, so its opening is cut whole.
const MOUTH_LIFT: i32 = 10;
/// `--spawn outside`: voxels toward -x from the mouth's column (30 m) ...
pub const OUTSIDE_RUN: i32 = 300;
/// ... and over the ground there (2 m).
pub const OUTSIDE_LIFT: i32 = 20;
/// `--spawn forest`'s column, 640 voxels north of the mouth.
pub const FOREST: [i32; 2] = [0, 640];
/// `--spawn forest`: over the ground there (2 m).
pub const FOREST_LIFT: i32 = 20;
/// `--spawn inside`: voxels along x into the hill past the tunnel's entrance (about 3 m along
/// the axis at its slope of about 1 in 4).
pub const INSIDE_RUN: i32 = 29;

/// Where `--spawn` stands the outdoor camera ([`OutdoorSource::spawn`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Spawn {
    /// In front of the mouth, facing it.
    #[default]
    Outside,
    /// In the tunnel, facing out.
    Inside,
    /// At [`FOREST`], facing north, pitched 10 degrees down.
    Forest,
}

impl Spawn {
    pub const ALL: [Self; 3] = [Self::Outside, Self::Inside, Self::Forest];

    pub fn name(self) -> &'static str {
        match self {
            Self::Outside => "outside",
            Self::Inside => "inside",
            Self::Forest => "forest",
        }
    }
}

/// The outdoor source of `seed`, unbounded in x and y.
#[derive(Clone, Copy, Debug)]
pub struct OutdoorSource {
    pub seed: u64,
    cave: CaveSource,
    /// The tunnel's axis, from over the mouth to the chamber's centre.
    mouth: [i64; 3],
    chamber: [i64; 3],
}

impl OutdoorSource {
    pub fn new(seed: u64) -> Self {
        let mut s = Self {
            seed,
            cave: CaveSource::unbounded(seed),
            mouth: [0; 3],
            chamber: [0; 3],
        };
        let [mx, my] = MOUTH;
        let cx = mx + CHAMBER_RUN;
        s.mouth = [mx, my, s.height(mx, my) + MOUTH_LIFT].map(i64::from);
        s.chamber = [cx, my, s.height(cx, my) - CHAMBER_DEPTH].map(i64::from);
        s
    }

    fn octaves(&self) -> [ValueNoise; 3] {
        [9, 7, 5].map(|k| ValueNoise::new(self.seed ^ 0x0D00_0000_0000_0000, k))
    }

    /// The surface's height at column `(x, y)`: the grass voxel's z.
    pub fn height(&self, x: i32, y: i32) -> i32 {
        let n = self.octaves();
        let sum: i64 = (0..3).map(|k| AMPLITUDE[k] * n[k].sample([x, y, 0])).sum();
        BASE_Z + (sum >> SHIFT) as i32
    }

    /// The tunnel's axis: over the mouth's surface, and the chamber's centre.
    pub fn tunnel(&self) -> [[i64; 3]; 2] {
        [self.mouth, self.chamber]
    }

    /// The chamber's centre voxel.
    pub fn chamber(&self) -> VoxelCoord {
        VoxelCoord(self.chamber.map(|v| v as i32))
    }

    /// The eye voxel of `spawn` and the integer direction it faces: [`Spawn::Outside`]
    /// [`OUTSIDE_RUN`] voxels toward -x from the mouth's column, [`OUTSIDE_LIFT`] over the
    /// ground there, facing +x; [`Spawn::Inside`] on the tunnel's axis [`INSIDE_RUN`] voxels
    /// along x past the first axis voxel under its column's surface, facing out along the axis;
    /// [`Spawn::Forest`] [`FOREST_LIFT`] over the ground at [`FOREST`], facing +y pitched 10
    /// degrees down (`tan 10 deg` is 1763 / 10000).
    pub fn spawn(&self, spawn: Spawn) -> (VoxelCoord, [i64; 3]) {
        let [mx, my] = MOUTH;
        match spawn {
            Spawn::Outside => {
                let x = mx - OUTSIDE_RUN;
                let eye = VoxelCoord([x, my, self.height(x, my) + OUTSIDE_LIFT]);
                (eye, [1, 0, 0])
            }
            Spawn::Inside => {
                let d = [0, 1, 2].map(|k| self.chamber[k] - self.mouth[k]);
                let axis_z = |i: i64| self.mouth[2] + d[2] * i / d[0];
                let x = |i: i64| (self.mouth[0] + i) as i32;
                let enter = (0..d[0])
                    .find(|&i| axis_z(i) < i64::from(self.height(x(i), my)))
                    .unwrap_or(0);
                let i = (enter + i64::from(INSIDE_RUN)).min(d[0]);
                let eye = VoxelCoord([x(i), my, axis_z(i) as i32]);
                (eye, d.map(|v| -v))
            }
            Spawn::Forest => {
                let [fx, fy] = FOREST;
                let eye = VoxelCoord([fx, fy, self.height(fx, fy) + FOREST_LIFT]);
                (eye, [0, 10_000, -1_763])
            }
        }
    }

    /// The squared distance from `p` to the tunnel's axis, compared exactly: whether it is at
    /// most `r^2` (the capsule edit op's `i64` segment test).
    fn near_axis(&self, p: [i64; 3], r: i64) -> bool {
        let (a, b) = (self.mouth, self.chamber);
        let d = [0, 1, 2].map(|k| b[k] - a[k]);
        let w = [0, 1, 2].map(|k| p[k] - a[k]);
        let dd: i64 = (0..3).map(|k| d[k] * d[k]).sum();
        let wd: i64 = (0..3).map(|k| w[k] * d[k]).sum();
        let ww: i64 = (0..3).map(|k| w[k] * w[k]).sum();
        if wd <= 0 {
            return ww <= r * r;
        }
        if wd >= dd {
            let e: i64 = (0..3).map(|k| (p[k] - b[k]).pow(2)).sum();
            return e <= r * r;
        }
        i128::from(ww) * i128::from(dd) - i128::from(wd) * i128::from(wd)
            <= i128::from(r * r) * i128::from(dd)
    }

    fn in_chamber(&self, p: [i64; 3]) -> bool {
        (0..3).map(|k| (p[k] - self.chamber[k]).pow(2)).sum::<i64>()
            <= CHAMBER_RADIUS * CHAMBER_RADIUS
    }

    /// Whether the cave's caverns are kept out of `p`.
    fn cavern_excluded(&self, p: [i64; 3]) -> bool {
        let dxy = (p[0] - self.chamber[0]).pow(2) + (p[1] - self.chamber[1]).pow(2);
        dxy <= CHAMBER_CLEAR * CHAMBER_CLEAR || self.near_axis(p, TUNNEL_CLEAR)
    }

    /// Whether `p` lies in the tunnel's capsule or the chamber.
    pub fn carved(&self, p: [i32; 3]) -> bool {
        let q = p.map(i64::from);
        self.near_axis(q, TUNNEL_RADIUS) || self.in_chamber(q)
    }

    /// The material of voxel `p` over a column of surface height `h` ([`OutdoorSource::height`]
    /// of its column): what [`OutdoorSource::material`] reads, with the height given.
    pub fn material_at(&self, p: [i32; 3], h: i32) -> MaterialId {
        let q = p.map(i64::from);
        if self.near_axis(q, TUNNEL_RADIUS) || self.in_chamber(q) || p[2] > h {
            AIR
        } else if p[2] == h {
            GRASS
        } else if p[2] >= h - SOIL_DEPTH {
            SOIL
        } else if p[2] < h - CAVE_DEPTH && !self.cavern_excluded(q) && self.cave.material(p) == AIR
        {
            AIR
        } else {
            ROCK
        }
    }

    /// The final material of voxel `p`.
    pub fn material(&self, p: [i32; 3]) -> MaterialId {
        self.material_at(p, self.height(p[0], p[1]))
    }

    /// Whether the box `lo..=hi` may meet the tunnel's or the chamber's air, or the caverns'
    /// exclusion: a test against their bounding boxes grown by the reach.
    fn near_carve(&self, lo: [i32; 3], hi: [i32; 3]) -> bool {
        let r = TUNNEL_CLEAR.max(CHAMBER_CLEAR) + 1;
        (0..3).all(|k| {
            let a = self.mouth[k].min(self.chamber[k]) - r;
            let b = self.mouth[k].max(self.chamber[k]) + r;
            i64::from(hi[k]) >= a && i64::from(lo[k]) <= b
        }) || {
            let c = self.chamber;
            (0..2).all(|k| {
                i64::from(hi[k]) >= c[k] - CHAMBER_CLEAR && i64::from(lo[k]) <= c[k] + CHAMBER_CLEAR
            })
        }
    }

    /// The brick with minimum voxel `b`, the surface heights of its columns in `heights`
    /// (x fastest).
    fn brick(&self, b: [i32; 3], heights: &[i32; 64]) -> Brick {
        let hi = b.map(|v| v + BRICK_EDGE - 1);
        let (min_h, max_h) = heights
            .iter()
            .fold((i32::MAX, i32::MIN), |(a, c), &h| (a.min(h), c.max(h)));
        let near = self.near_carve(b, hi);
        if !near && b[2] > max_h {
            return Brick::Uniform(AIR);
        }
        if !near && hi[2] < min_h - SOIL_DEPTH && hi[2] >= min_h - CAVE_DEPTH {
            return Brick::Uniform(ROCK);
        }
        if !near && hi[2] < min_h - CAVE_DEPTH {
            // The cave's own brick, its pocket and ore read as rock: deep under every column,
            // where only its caverns reach.
            return match self.cave.brick(b) {
                Brick::Uniform(m) => Brick::Uniform(if m == AIR { AIR } else { ROCK }),
                _ => Brick::from_fn(|l| {
                    let p = [0, 1, 2].map(|k| b[k] + l[k]);
                    if self.cave.material(p) == AIR {
                        AIR
                    } else {
                        ROCK
                    }
                }),
            };
        }
        Brick::from_fn(|l| {
            let p = [0, 1, 2].map(|k| b[k] + l[k]);
            self.material_at(p, heights[(l[0] + BRICK_EDGE * l[1]) as usize])
        })
    }
}

impl WorldSource for OutdoorSource {
    fn chunk(&self, coord: ChunkCoord, out: &mut Chunk) {
        let base = coord.min_voxel().0;
        let edge = CHUNK_EDGE as usize;
        let mut cols = vec![0i32; edge * edge];
        for y in 0..edge {
            for x in 0..edge {
                cols[x + edge * y] = self.height(base[0] + x as i32, base[1] + y as i32);
            }
        }
        for slot in 0..CHUNK_BRICKS {
            let s = brick_of_slot(slot);
            let mut heights = [0i32; 64];
            for ly in 0..BRICK_EDGE {
                for lx in 0..BRICK_EDGE {
                    let (x, y) = (s[0] * BRICK_EDGE + lx, s[1] * BRICK_EDGE + ly);
                    heights[(lx + BRICK_EDGE * ly) as usize] = cols[x as usize + edge * y as usize];
                }
            }
            let b = [0, 1, 2].map(|k| base[k] + s[k] * BRICK_EDGE);
            out.set_brick(slot, self.brick(b, &heights));
        }
    }
}
