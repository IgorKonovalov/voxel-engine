//! The exact ray walk through one posed part.
//!
//! A world ray is `origin + dir * t / 256` in 1/256 voxel, as for `World::raycast`. Into a part's
//! space it maps with no rounding: a local coordinate is carried in units of `1 / (256 * 16384)`
//! model voxel, so `local = pivot * 16384 + detail * M^T (world - part pivot)` is an integer for
//! any integer world point, and the local ray is `L0 + D * t / 256` with the same parameter `t`.
//! `M^T` stands for the inverse rotation; this map is the definition of where a turned part's
//! voxels are, so the query needs no float and no division until it reports `t`.
//!
//! Crossing times are exact fractions compared by cross-multiplication in `i128`, as in the
//! world's walk. Within the documented domain (origins in the `i32` voxel range, `|dir|`
//! components below 2^40) no product overflows.

use vxe_world::{AIR, Face, MaterialId};

use crate::model::Part;
use crate::pose::PartFrame;
use crate::quat::{Q14_ONE, rotate_back_q14};

/// Local units per model voxel.
const LOCAL_ONE: i128 = 256 * Q14_ONE as i128;

/// A ray parameter `num / den`, `den > 0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Frac {
    pub num: i128,
    pub den: i128,
}

impl Frac {
    pub fn int(t: i64) -> Self {
        Self {
            num: i128::from(t),
            den: 1,
        }
    }

    pub fn lt(self, o: Self) -> bool {
        self.num * o.den < o.num * self.den
    }

    /// Rounded down, as `World::raycast` reports `t`.
    pub fn floor(self) -> i64 {
        self.num.div_euclid(self.den) as i64
    }
}

/// The face entered when stepping along `axis` toward `positive`.
fn entered(axis: usize, positive: bool) -> Face {
    match (axis, positive) {
        (0, true) => Face::NegX,
        (0, false) => Face::PosX,
        (1, true) => Face::NegY,
        (1, false) => Face::PosY,
        (2, true) => Face::NegZ,
        _ => Face::PosZ,
    }
}

/// Where the ray `o + d * t / 256`, `t >= 0`, is inside the half-open box `[lo, hi)`: the entry
/// and exit parameters and the axis of the entry face (`None` when the ray starts inside or on
/// the box at `t = 0`). `None` when it misses or only grazes an edge.
pub(crate) fn box_span(
    o: [i128; 3],
    d: [i128; 3],
    lo: [i128; 3],
    hi: [i128; 3],
) -> Option<(Frac, Frac, Option<usize>)> {
    let mut enter = Frac::int(0);
    let mut axis = None;
    let mut exit: Option<Frac> = None;
    for k in 0..3 {
        if d[k] == 0 {
            if o[k] < lo[k] || o[k] >= hi[k] {
                return None;
            }
            continue;
        }
        let (near, far) = if d[k] > 0 {
            (lo[k], hi[k])
        } else {
            (hi[k], lo[k])
        };
        let s = d[k].signum();
        let den = d[k].abs();
        let tn = Frac {
            num: (near - o[k]) * 256 * s,
            den,
        };
        let tf = Frac {
            num: (far - o[k]) * 256 * s,
            den,
        };
        if enter.lt(tn) {
            enter = tn;
            axis = Some(k);
        }
        if exit.is_none_or(|e| tf.lt(e)) {
            exit = Some(tf);
        }
    }
    match exit {
        Some(e) if !enter.lt(e) => None,
        _ => Some((enter, exit.unwrap_or(enter), axis)),
    }
}

/// The first solid voxel of one part along the world ray, at a parameter no greater than
/// `limit`.
pub(crate) struct PartHit {
    pub t: Frac,
    pub voxel: [u8; 3],
    pub face: Option<Face>,
    pub material: MaterialId,
}

/// Walks part `part`, placed at `frame`, along the world ray `origin + dir * t / 256`.
pub(crate) fn part_raycast(
    part: &Part,
    frame: &PartFrame,
    origin: [i64; 3],
    dir: [i64; 3],
    limit: Frac,
) -> Option<PartHit> {
    let detail = i128::from(part.detail);
    let rel = [0, 1, 2].map(|k| i128::from(origin[k]) - i128::from(frame.pivot[k]));
    let back = rotate_back_q14(&frame.m, rel);
    let l0 = [0, 1, 2].map(|k| i128::from(part.pivot[k]) * i128::from(Q14_ONE) + detail * back[k]);
    let d = rotate_back_q14(&frame.m, dir.map(i128::from)).map(|c| c * detail);
    let size = part.volume.size();
    let hi = size.map(|s| i128::from(s) * LOCAL_ONE);
    let (enter, _exit, axis) = box_span(l0, d, [0; 3], hi)?;
    let mut t = enter;
    let mut face = axis.map(|k| entered(k, d[k] > 0));
    loop {
        if limit.lt(t) {
            return None;
        }
        let v = voxel_at(l0, d, t);
        if (0..3).any(|k| v[k] < 0 || v[k] >= i128::from(size[k])) {
            return None;
        }
        let local = v.map(|c| c as i32);
        if let Some(material) = part.volume.get(local)
            && material != AIR
        {
            return Some(PartHit {
                t,
                voxel: v.map(|c| c as u8),
                face,
                material,
            });
        }
        // The earliest boundary crossing out of voxel `v`, lowest axis first on a tie.
        let mut best: Option<(Frac, usize)> = None;
        for k in 0..3 {
            if d[k] == 0 {
                continue;
            }
            let edge = if d[k] > 0 { v[k] + 1 } else { v[k] };
            let tk = Frac {
                num: (edge * LOCAL_ONE - l0[k]) * 256 * d[k].signum(),
                den: d[k].abs(),
            };
            if best.is_none_or(|(b, _)| tk.lt(b)) {
                best = Some((tk, k));
            }
        }
        let (tk, k) = best?;
        t = tk;
        face = Some(entered(k, d[k] > 0));
    }
}

/// The voxel holding the local point at parameter `t`; on a boundary, the voxel the ray moves
/// into.
fn voxel_at(l0: [i128; 3], d: [i128; 3], t: Frac) -> [i128; 3] {
    [0, 1, 2].map(|k| {
        // position * den * 256 = l0 * den * 256 + d * num, over den * 256 * LOCAL_ONE a voxel.
        let p = l0[k] * t.den * 256 + d[k] * t.num;
        let q = t.den * 256 * LOCAL_ONE;
        if d[k] < 0 {
            (p - 1).div_euclid(q)
        } else {
            p.div_euclid(q)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_box_span_names_its_entry_face() {
        let (enter, exit, axis) = box_span([-512, 10, 10], [256, 0, 0], [0; 3], [256; 3]).unwrap();
        assert_eq!((enter.floor(), exit.floor(), axis), (512, 768, Some(0)));
        // Starting inside: no entry face, entry at zero.
        let (enter, _, axis) = box_span([10, 10, 10], [0, -256, 0], [0; 3], [256; 3]).unwrap();
        assert_eq!((enter, axis), (Frac::int(0), None));
        // Beside the box, and on its far plane along a still axis.
        assert!(box_span([-512, 300, 10], [256, 0, 0], [0; 3], [256; 3]).is_none());
        assert!(box_span([-512, 256, 10], [256, 0, 0], [0; 3], [256; 3]).is_none());
        // Behind the origin.
        assert!(box_span([512, 10, 10], [256, 0, 0], [0; 3], [256; 3]).is_none());
    }
}
