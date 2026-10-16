//! Edit shapes in fixed point and their integer inside tests.
//!
//! A voxel is inside a shape when its fixed-point centre is. Sphere and box compare squared
//! distances in `i64`; the capsule multiplies two squared lengths and works in `i128`. Both
//! stay exact for coordinates within ±2^30 fixed-point units (±4 M voxels) of each other.

use crate::fixed::{Fixed3, first_centre_at_or_above, last_centre_at_or_below};

/// A region of space in fixed point.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Shape {
    /// Centres within distance `r` of `c`, boundary included.
    Sphere { c: Fixed3, r: i64 },
    /// Centres in the half-open box `[min, max)`.
    Box_ { min: Fixed3, max: Fixed3 },
    /// Centres within distance `r` of the segment `a..b`, boundary included.
    Capsule { a: Fixed3, b: Fixed3, r: i64 },
}

/// How a box of voxel centres relates to a shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Coverage {
    /// No centre in the box is inside.
    Outside,
    /// Every centre in the box is inside.
    Inside,
    /// Unknown; test each voxel.
    Partial,
}

impl Shape {
    /// Inclusive voxel bounds of every voxel whose centre may lie inside; `None` when empty.
    pub(crate) fn voxel_bounds(&self) -> Option<([i64; 3], [i64; 3])> {
        let (lo, hi) = match *self {
            Shape::Sphere { c, r } => {
                if r < 0 {
                    return None;
                }
                (
                    c.0.map(|v| first_centre_at_or_above(v - r)),
                    c.0.map(|v| last_centre_at_or_below(v + r)),
                )
            }
            Shape::Box_ { min, max } => (
                min.0.map(first_centre_at_or_above),
                max.0.map(|v| last_centre_at_or_below(v - 1)),
            ),
            Shape::Capsule { a, b, r } => {
                if r < 0 {
                    return None;
                }
                (
                    [0, 1, 2].map(|k| first_centre_at_or_above(a.0[k].min(b.0[k]) - r)),
                    [0, 1, 2].map(|k| last_centre_at_or_below(a.0[k].max(b.0[k]) + r)),
                )
            }
        };
        (0..3).all(|k| lo[k] <= hi[k]).then_some((lo, hi))
    }

    /// Whether the fixed-point point `p` is inside.
    pub(crate) fn contains(&self, p: [i64; 3]) -> bool {
        match *self {
            Shape::Sphere { c, r } => dist2(p, c.0) <= r * r,
            Shape::Box_ { min, max } => (0..3).all(|k| p[k] >= min.0[k] && p[k] < max.0[k]),
            Shape::Capsule { a, b, r } => {
                let (num, den) = segment_dist2(p, a.0, b.0);
                let r = i128::from(r);
                num <= r * r * den
            }
        }
    }

    /// Relation to every fixed-point centre in the box `clo..=chi`. `Outside` and `Inside`
    /// are never wrong; `Partial` may be returned for a box that is in fact one or the other.
    pub(crate) fn coverage(&self, clo: [i64; 3], chi: [i64; 3]) -> Coverage {
        match *self {
            Shape::Sphere { c, r } => {
                let mut near = 0;
                let mut far = 0;
                for k in 0..3 {
                    let n = c.0[k].clamp(clo[k], chi[k]) - c.0[k];
                    let f = (clo[k] - c.0[k]).abs().max((chi[k] - c.0[k]).abs());
                    near += n * n;
                    far += f * f;
                }
                if near > r * r {
                    Coverage::Outside
                } else if far <= r * r {
                    Coverage::Inside
                } else {
                    Coverage::Partial
                }
            }
            Shape::Box_ { min, max } => {
                if (0..3).any(|k| chi[k] < min.0[k] || clo[k] >= max.0[k]) {
                    Coverage::Outside
                } else if (0..3).all(|k| clo[k] >= min.0[k] && chi[k] < max.0[k]) {
                    Coverage::Inside
                } else {
                    Coverage::Partial
                }
            }
            Shape::Capsule { a, b, r } => {
                // Outside if the box misses the capsule's bounding box, or if the box's
                // bounding sphere (centre m, radius h rounded up) misses the capsule.
                if (0..3)
                    .any(|k| chi[k] < a.0[k].min(b.0[k]) - r || clo[k] > a.0[k].max(b.0[k]) + r)
                {
                    return Coverage::Outside;
                }
                let m = [0, 1, 2].map(|k| (clo[k] + chi[k]).div_euclid(2));
                let h2: i128 = (0..3)
                    .map(|k| {
                        let e = i128::from((m[k] - clo[k]).max(chi[k] - m[k]));
                        e * e
                    })
                    .sum();
                let h = isqrt_ceil(h2);
                let (num, den) = segment_dist2(m, a.0, b.0);
                let reach = i128::from(r) + h;
                if num > reach * reach * den {
                    return Coverage::Outside;
                }
                // A capsule is convex: if every corner is inside, the whole box is.
                let corner =
                    |i: usize| [0, 1, 2].map(|k| if i >> k & 1 == 0 { clo[k] } else { chi[k] });
                if (0..8).all(|i| self.contains(corner(i))) {
                    Coverage::Inside
                } else {
                    Coverage::Partial
                }
            }
        }
    }
}

fn dist2(a: [i64; 3], b: [i64; 3]) -> i64 {
    (0..3).map(|k| (a[k] - b[k]) * (a[k] - b[k])).sum()
}

/// Squared distance from `p` to the segment `a..b` as the exact fraction `num / den`, `den > 0`.
fn segment_dist2(p: [i64; 3], a: [i64; 3], b: [i64; 3]) -> (i128, i128) {
    let d = [0, 1, 2].map(|k| i128::from(b[k] - a[k]));
    let w = [0, 1, 2].map(|k| i128::from(p[k] - a[k]));
    let len2: i128 = (0..3).map(|k| d[k] * d[k]).sum();
    let t: i128 = (0..3).map(|k| w[k] * d[k]).sum();
    let ww: i128 = (0..3).map(|k| w[k] * w[k]).sum();
    if len2 == 0 || t <= 0 {
        return (ww, 1);
    }
    if t >= len2 {
        let e = [0, 1, 2].map(|k| i128::from(p[k] - b[k]));
        return ((0..3).map(|k| e[k] * e[k]).sum(), 1);
    }
    // |w|² - t² / |d|², over the common denominator |d|².
    (ww * len2 - t * t, len2)
}

/// Smallest `s >= 0` with `s * s >= n`.
fn isqrt_ceil(n: i128) -> i128 {
    let n = n.max(0).cast_unsigned();
    let s = n.isqrt();
    (if s * s < n { s + 1 } else { s }).cast_signed()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isqrt_ceil_rounds_up() {
        assert_eq!(isqrt_ceil(0), 0);
        assert_eq!(isqrt_ceil(1), 1);
        assert_eq!(isqrt_ceil(2), 2);
        assert_eq!(isqrt_ceil(16), 4);
        assert_eq!(isqrt_ceil(17), 5);
    }

    #[test]
    fn segment_distance_is_exact_in_the_middle_and_at_the_caps() {
        let a = [0, 0, 0];
        let b = [10, 0, 0];
        assert_eq!(segment_dist2([5, 3, 4], a, b), (25 * 100, 100));
        assert_eq!(segment_dist2([-3, 4, 0], a, b), (25, 1));
        assert_eq!(segment_dist2([13, 0, 4], a, b), (25, 1));
        assert_eq!(segment_dist2([1, 1, 0], a, a), (2, 1));
    }

    /// The capsule's coverage verdicts agree with the per-centre test on every brick-sized
    /// box around a diagonal capsule.
    #[test]
    fn capsule_coverage_never_lies() {
        let s = Shape::Capsule {
            a: Fixed3([128, 128, 128]),
            b: Fixed3([128 + 9 * 256, 128 + 5 * 256, 128 - 7 * 256]),
            r: 3 * 256 + 17,
        };
        let centre = |i: i64| i * 256 + 128;
        for bx in -3..3 {
            for by in -3..3 {
                for bz in -3..3 {
                    let lo = [bx * 4, by * 4, bz * 4];
                    let hi = lo.map(|v| v + 3);
                    let cov = s.coverage(lo.map(centre), hi.map(centre));
                    let mut inside = 0;
                    for x in lo[0]..=hi[0] {
                        for y in lo[1]..=hi[1] {
                            for z in lo[2]..=hi[2] {
                                inside += i32::from(s.contains([x, y, z].map(centre)));
                            }
                        }
                    }
                    match cov {
                        Coverage::Outside => assert_eq!(inside, 0, "{lo:?}"),
                        Coverage::Inside => assert_eq!(inside, 64, "{lo:?}"),
                        Coverage::Partial => {}
                    }
                }
            }
        }
    }
}
