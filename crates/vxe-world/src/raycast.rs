//! Integer DDA raycast over the brick map.
//!
//! A ray is `origin + dir * t / 256` for `t >= 0`, all in fixed point (1/256 voxel). With
//! `|dir| == 256` the parameter `t` is a distance in fixed units. Crossing times are exact
//! fractions `num / den` compared by cross-multiplication in `i128`; nothing is divided until
//! the hit's `t` is reported, rounded down.
//!
//! The walk steps through cells of three sizes: a missing chunk is skipped as one 64³ cell, a
//! `Uniform` air brick as one 8³ cell, and voxels of a `Full` brick one at a time. The first
//! non-air voxel is the hit.

use crate::coords::{BRICK_EDGE, CHUNK_EDGE, VoxelCoord};
use crate::fixed::{FIXED_ONE, Fixed3};
use crate::material::{AIR, MaterialId};
use crate::store::{World, read};

/// The face of the hit voxel the ray entered through: `NegZ` is the face at the voxel's
/// minimum z, entered by a ray travelling toward +z.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Face {
    NegX,
    PosX,
    NegY,
    PosY,
    NegZ,
    PosZ,
}

impl Face {
    /// The face crossed when stepping along `axis` in the direction of `sign`.
    fn entered(axis: usize, sign: i64) -> Self {
        match (axis, sign > 0) {
            (0, true) => Self::NegX,
            (0, false) => Self::PosX,
            (1, true) => Self::NegY,
            (1, false) => Self::PosY,
            (2, true) => Self::NegZ,
            _ => Self::PosZ,
        }
    }
}

/// The first non-air voxel along a ray.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Hit {
    pub voxel: VoxelCoord,
    /// `None` when the ray starts inside the voxel.
    pub face: Option<Face>,
    /// Ray parameter where the ray enters the voxel, rounded down.
    pub t: i64,
    pub material: MaterialId,
}

/// A hit plus its exact entry parameter `t_num / t_den` (`t_den > 0`).
pub(crate) struct ExactHit {
    pub hit: Hit,
    pub t_num: i128,
    pub t_den: i128,
}

/// What the walk finds at a voxel: a hit, or the edge of the empty cell holding it.
enum Cell {
    Solid(MaterialId),
    Empty(i64),
}

impl World {
    /// The first non-air voxel the ray `origin + dir * t / 256`, `0 <= t <= max_t`, enters.
    pub fn raycast(&self, origin: Fixed3, dir: [i64; 3], max_t: i64) -> Option<Hit> {
        self.raycast_exact(origin, dir, max_t).map(|h| h.hit)
    }

    pub(crate) fn raycast_exact(
        &self,
        origin: Fixed3,
        dir: [i64; 3],
        max_t: i64,
    ) -> Option<ExactHit> {
        let o = origin.0.map(i128::from);
        let d = dir.map(i128::from);
        let one = i128::from(FIXED_ONE);
        // Current parameter t = num / den, and the face we came through.
        let (mut num, mut den): (i128, i128) = (0, 1);
        let mut face = None;
        loop {
            if num > i128::from(max_t) * den {
                return None;
            }
            let v = voxel_at(o, d, num, den);
            let size = match self.cell(v) {
                Cell::Solid(material) => {
                    return Some(ExactHit {
                        hit: Hit {
                            voxel: VoxelCoord(v.map(|c| c as i32)),
                            face,
                            t: num.div_euclid(den) as i64,
                            material,
                        },
                        t_num: num,
                        t_den: den,
                    });
                }
                Cell::Empty(size) => i128::from(size),
            };
            // Leave the `size`-aligned cell holding `v`: the earliest boundary crossing, lowest
            // axis first on a tie.
            let mut best: Option<(i128, i128, usize)> = None;
            for k in 0..3 {
                if d[k] == 0 {
                    continue;
                }
                let cell_min = i128::from(v[k]).div_euclid(size) * size;
                let edge = if d[k] > 0 { cell_min + size } else { cell_min };
                // t_k = (edge * 256 - o) * 256 / d, as a fraction over |d|.
                let n = (edge * one - o[k]) * one * d[k].signum();
                let m = d[k].abs();
                if best.is_none_or(|(bn, bm, _)| n * bm < bn * m) {
                    best = Some((n, m, k));
                }
            }
            let (n, m, k) = best?;
            (num, den) = (n, m);
            face = Some(Face::entered(k, dir[k]));
        }
    }

    /// The material at `v` if solid, else the edge of the empty cell to skip.
    fn cell(&self, v: [i64; 3]) -> Cell {
        // Beyond the i32 lattice no chunk can be resident.
        let [Ok(x), Ok(y), Ok(z)] = v.map(i32::try_from) else {
            return Cell::Empty(i64::from(CHUNK_EDGE));
        };
        let v = VoxelCoord([x, y, z]);
        let Some(chunk) = self.store.get(v.chunk()) else {
            return Cell::Empty(i64::from(CHUNK_EDGE));
        };
        let chunk = read(&chunk);
        let brick = chunk.brick(v.brick_slot());
        if !brick.is_full() && brick.get(0) == AIR {
            return Cell::Empty(i64::from(BRICK_EDGE));
        }
        match brick.get(v.index_in_brick()) {
            AIR => Cell::Empty(1),
            m => Cell::Solid(m),
        }
    }
}

/// The voxel the ray occupies just after parameter `num / den`: on an axis it travels down, a
/// position exactly on a voxel boundary belongs to the voxel below.
fn voxel_at(o: [i128; 3], d: [i128; 3], num: i128, den: i128) -> [i64; 3] {
    let one = i128::from(FIXED_ONE);
    [0, 1, 2].map(|k| {
        // position * den * 256 = o * den * 256 + d * num, over den * 256 * 256 per voxel.
        let p = o[k] * den * one + d[k] * num;
        let q = den * one * one;
        let v = if d[k] < 0 {
            (p - 1).div_euclid(q)
        } else {
            p.div_euclid(q)
        };
        v as i64
    })
}

/// The fixed-point point at exact parameter `num / den`, rounded down.
pub(crate) fn point_at(origin: Fixed3, dir: [i64; 3], num: i128, den: i128) -> Fixed3 {
    let one = i128::from(FIXED_ONE);
    Fixed3([0, 1, 2].map(|k| {
        let p = i128::from(origin.0[k]) * den * one + i128::from(dir[k]) * num;
        p.div_euclid(den * one) as i64
    }))
}
