//! Edit shapes in fixed point and their integer inside tests.
//!
//! A voxel is inside a shape when its fixed-point centre is. Sphere and box compare squared
//! distances in `i64`; the capsule multiplies two squared lengths and works in `i128`. Sphere
//! and box are exact for coordinates and radii within `±coord_max` of
//! [`WorldLimits`](crate::WorldLimits); the capsule also needs each axis of `b - a` within
//! `capsule_axis_max`. Both hold for points within the shape's voxel bounds; `contains` on a
//! point far outside them is not covered. [`Shape::check`] enforces the range.

use crate::fixed::{Fixed3, centre, first_centre_at_or_above, last_centre_at_or_below};

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
    /// An edit with this shape reads and writes no voxel outside them.
    pub fn voxel_bounds(&self) -> Option<([i64; 3], [i64; 3])> {
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

    /// Whether the fixed-point point `p` is inside. An edit changes a voxel only when this
    /// holds for its centre (`256 v + 128` per axis).
    pub fn contains(&self, p: [i64; 3]) -> bool {
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

    /// The inclusive range of voxel indices `x` in `x_lo..=x_hi` whose centres
    /// `(256 x + 128, 256 y + 128, 256 z + 128)` lie inside, or `None` when there are none.
    /// The set of such `x` is always one contiguous range, because every shape is convex.
    ///
    /// The sphere solves `dx² <= r² - dy² - dz²` with an integer square root: for a
    /// non-negative integer `rem`, `dx² <= rem` exactly when `|dx| <= isqrt(rem)`. The box
    /// clips its half-open range. The capsule takes the window's centre nearest the segment
    /// (see `CapsuleRow::nearest`), returns `None` when it is outside, and otherwise bisects
    /// each end with the test of [`Shape::contains`]: at most `2 + 2 ceil(log2(w))` tests for a
    /// window of `w` voxels, 2 when no centre is inside.
    pub fn row_span(&self, y: i64, z: i64, x_lo: i64, x_hi: i64) -> Option<(i64, i64)> {
        let (lo, hi) = match *self {
            Shape::Sphere { c, r } => {
                let dy = centre(y) - c.0[1];
                let dz = centre(z) - c.0[2];
                let rem = r * r - dy * dy - dz * dz;
                if r < 0 || rem < 0 {
                    return None;
                }
                let h = rem.isqrt();
                (
                    first_centre_at_or_above(c.0[0] - h).max(x_lo),
                    last_centre_at_or_below(c.0[0] + h).min(x_hi),
                )
            }
            Shape::Box_ { min, max } => {
                let (cy, cz) = (centre(y), centre(z));
                if cy < min.0[1] || cy >= max.0[1] || cz < min.0[2] || cz >= max.0[2] {
                    return None;
                }
                (
                    first_centre_at_or_above(min.0[0]).max(x_lo),
                    last_centre_at_or_below(max.0[0] - 1).min(x_hi),
                )
            }
            Shape::Capsule { a, b, r } => {
                if x_lo > x_hi {
                    return None;
                }
                let row = CapsuleRow::new(a.0, b.0, r, centre(y), centre(z));
                let m = row.nearest(x_lo, x_hi)?;
                // The inside centres of the row are one interval holding `m` (the capsule is
                // convex): bisect for its first centre in `x_lo..=m` and its last in
                // `m..=x_hi`.
                let inside = |x: i64| row.contains(centre(x));
                (first_true(x_lo, m, inside), last_true(m, x_hi, inside))
            }
        };
        (lo <= hi).then_some((lo, hi))
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

/// The spans of a sphere's voxel rows `y` in `ylo..=yhi`, `z` in `zlo..=zhi`, clipped to
/// `x_lo..=x_hi`, written to `out[(y - ylo) + ny (z - zlo)]` (`ny = yhi - ylo + 1`; `out`
/// holds at least `ny` times the `z` count) and empty as `(1, 0)`.
///
/// A row's span is the set of `x` with `(256 x + 128 - cx)² <= rem`, where
/// `rem = r² - dy² - dz²`; it is an interval about `cx` that grows with `rem`. The rows are
/// visited `z` ascending, `y` ascending within each `z`, and each is
/// walked from a neighbouring row's unclipped span (the previous row at the same `z`, or the
/// first row of the previous `z`): when `rem` grew, each end steps outward while the next
/// centre is inside; when it shrank, each end steps inward while its centre is outside. Every
/// step is one squared-distance comparison, so the walked span is exactly the span `isqrt`
/// solves. A row with no neighbouring span is solved with `isqrt` when it is the first row
/// or its span may be long, and otherwise walked outward from the centre nearest
/// `cx` (it is near the surface, so its span is short).
pub(crate) fn sphere_row_spans(
    c: Fixed3,
    r: i64,
    (ylo, yhi): (i64, i64),
    (zlo, zhi): (i64, i64),
    (x_lo, x_hi): (i64, i64),
    out: &mut [(i64, i64)],
) {
    let ny = yhi - ylo + 1;
    let [cx, cy, cz] = c.0;
    let r2 = r * r;
    // The nearest-centre start: the voxel whose centre is at or below `cx`, and the next.
    let below = last_centre_at_or_below(cx);
    // Spans of at most about 16 voxels start from the centre; longer ones are solved.
    const SHORT: i64 = (8 * 256) * (8 * 256);
    // The previous row at this `z`, and the first row of the previous `z`: (rem, lo, hi),
    // empty when lo > hi.
    let mut first = (0i64, 1i64, 0i64);
    let mut solved_first = false;
    for z in zlo..=zhi {
        let dz = centre(z) - cz;
        let rem_z = r2 - dz * dz;
        let mut prev = first;
        for y in ylo..=yhi {
            let dy = centre(y) - cy;
            let rem = rem_z - dy * dy;
            let (lo, hi) = if r < 0 || rem < 0 {
                (1, 0)
            } else if prev.1 <= prev.2 {
                walk_sphere_span(cx, rem, prev.0, prev.1, prev.2)
            } else if !solved_first || rem > SHORT {
                solve_sphere_span(cx, rem)
            } else {
                let inside = |x: i64| {
                    let d = centre(x) - cx;
                    d * d <= rem
                };
                // The inside centres are an interval about `cx`, so one of the two centres
                // nearest `cx` is inside unless none is.
                if inside(below) {
                    walk_sphere_span(cx, rem, -1, below, below)
                } else if inside(below + 1) {
                    walk_sphere_span(cx, rem, -1, below + 1, below + 1)
                } else {
                    (1, 0)
                }
            };
            solved_first = true;
            let row = (rem, lo, hi);
            if y == ylo {
                first = row;
            }
            prev = row;
            let (lo, hi) = (lo.max(x_lo), hi.min(x_hi));
            out[((y - ylo) + ny * (z - zlo)) as usize] = if lo <= hi { (lo, hi) } else { (1, 0) };
        }
    }
}

/// The unclipped sphere span of a row with `rem >= 0`, by integer square root: `d² <= rem`
/// exactly when `|d| <= isqrt(rem)`. Empty when `lo > hi`.
fn solve_sphere_span(cx: i64, rem: i64) -> (i64, i64) {
    let h = rem.isqrt();
    (
        first_centre_at_or_above(cx - h),
        last_centre_at_or_below(cx + h),
    )
}

/// The unclipped sphere span of a row with `rem >= 0`, walked from the non-empty span
/// `lo..=hi` of a row with `prev` (`prev <= rem` when `lo..=hi` is known to be inside).
/// Empty when `lo > hi`. Each end takes its first step without a branch (between
/// neighbouring rows an end moves by 0 or 1 voxel except near the poles) and loops only for
/// the rest.
fn walk_sphere_span(cx: i64, rem: i64, prev: i64, mut lo: i64, mut hi: i64) -> (i64, i64) {
    let inside = |x: i64| {
        let d = centre(x) - cx;
        d * d <= rem
    };
    if rem >= prev {
        // The span grows or stays: both old ends are still inside.
        lo -= i64::from(inside(lo - 1));
        hi += i64::from(inside(hi + 1));
        while inside(lo - 1) {
            lo -= 1;
        }
        while inside(hi + 1) {
            hi += 1;
        }
    } else {
        // The span shrinks or stays: it lies within the old one.
        lo += i64::from(!inside(lo));
        hi -= i64::from(!inside(hi));
        while lo <= hi && !inside(lo) {
            lo += 1;
        }
        while hi >= lo && !inside(hi) {
            hi -= 1;
        }
    }
    (lo, hi)
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

/// The capsule test of [`Shape::contains`] along one row (fixed `y`, `z`): the terms that do
/// not depend on `x` are summed once, so each point costs three products. Every sum is the
/// same exact integer `segment_dist2` forms, so the verdict is identical point for point.
struct CapsuleRow {
    ax: i128,
    bx: i128,
    dx: i128,
    len2: i128,
    r2: i128,
    /// `y`, `z` parts of `w·d`, `|w|²` and `|p - b|²`.
    t_yz: i128,
    ww_yz: i128,
    ee_yz: i128,
    /// `dy² + dz²`: the segment's squared length across the row.
    across2: i128,
}

impl CapsuleRow {
    fn new(a: [i64; 3], b: [i64; 3], r: i64, cy: i64, cz: i64) -> Self {
        let d = [0, 1, 2].map(|k| i128::from(b[k] - a[k]));
        let (wy, wz) = (i128::from(cy - a[1]), i128::from(cz - a[2]));
        let (ey, ez) = (i128::from(cy - b[1]), i128::from(cz - b[2]));
        Self {
            ax: i128::from(a[0]),
            bx: i128::from(b[0]),
            dx: d[0],
            len2: d[0] * d[0] + d[1] * d[1] + d[2] * d[2],
            r2: i128::from(r) * i128::from(r),
            t_yz: wy * d[1] + wz * d[2],
            ww_yz: wy * wy + wz * wz,
            ee_yz: ey * ey + ez * ez,
            across2: d[1] * d[1] + d[2] * d[2],
        }
    }

    /// A centre of `x_lo..=x_hi` (`x_lo <= x_hi`) at least as near the segment as every other
    /// centre of the window, if it is inside; `None` when no centre of the window is inside.
    ///
    /// The point of the segment `a + t d` nearest the row line minimises
    /// `(cy - ay - t dy)² + (cz - az - t dz)²` over `t` in `[0, 1]`: `t = t_yz / across2`
    /// clamped (any `t` when `across2 == 0`, the segment parallel to `x` or a point; `t = 0`
    /// is taken). The row point nearest the segment then has `x* = ax + t dx`, and the
    /// squared distance along the row is a convex function of `x` minimised there, so over the
    /// window's centres it is least at the centre just below or just above `x*`, clamped to
    /// the window. Both are tested: two capsule tests.
    ///
    /// The closed form stays inside `i128` for any `ax` in `i64` and any segment whose
    /// components differ by at most `2^31` (the module's ±2^30 bound): `td <= across2 <=
    /// 2^63`, so `|ax td| <= 2^126`, `|dx tn| <= 2^94` and `128 td <= 2^70`, and `num` and
    /// `256 td` are below `2^127`.
    fn nearest(&self, x_lo: i64, x_hi: i64) -> Option<i64> {
        // t = tn / td, in [0, 1].
        let (tn, td) = if self.across2 == 0 {
            (0, 1)
        } else {
            (self.t_yz.clamp(0, self.across2), self.across2)
        };
        // Below: the greatest voxel whose centre 256 i + 128 is at or below x* = ax + dx tn / td,
        // i = floor((ax td + dx tn - 128 td) / (256 td)).
        let num = self.ax * td + self.dx * tn - 128 * td;
        let below = num.div_euclid(256 * td);
        let clamp = |i: i128| i64::try_from(i.clamp(i128::from(x_lo), i128::from(x_hi)));
        let (below, above) = (clamp(below).ok()?, clamp(below + 1).ok()?);
        if self.contains(centre(below)) {
            Some(below)
        } else if above != below && self.contains(centre(above)) {
            Some(above)
        } else {
            None
        }
    }

    /// Whether the point `(cx, y, z)` is inside.
    #[inline]
    fn contains(&self, cx: i64) -> bool {
        #[cfg(test)]
        tests::CAPSULE_TESTS.with(|n| n.set(n.get() + 1));
        let wx = i128::from(cx) - self.ax;
        let t = wx * self.dx + self.t_yz;
        let ww = wx * wx + self.ww_yz;
        if self.len2 == 0 || t <= 0 {
            ww <= self.r2
        } else if t >= self.len2 {
            let ex = i128::from(cx) - self.bx;
            ex * ex + self.ee_yz <= self.r2
        } else {
            ww * self.len2 - t * t <= self.r2 * self.len2
        }
    }
}

/// The least `x` in `lo..=hi` with `f(x)`, for `f` false then true along the range and
/// `f(hi)` true; `ceil(log2(hi - lo + 1))` calls.
fn first_true(lo: i64, hi: i64, f: impl Fn(i64) -> bool) -> i64 {
    // Invariant: f(x) is false for x <= out (x >= lo) and true at x = in_.
    let (mut out, mut in_) = (lo - 1, hi);
    while in_ - out > 1 {
        let mid = out + (in_ - out) / 2;
        if f(mid) {
            in_ = mid;
        } else {
            out = mid;
        }
    }
    in_
}

/// The greatest `x` in `lo..=hi` with `f(x)`, for `f` true then false along the range and
/// `f(lo)` true; `ceil(log2(hi - lo + 1))` calls.
fn last_true(lo: i64, hi: i64, f: impl Fn(i64) -> bool) -> i64 {
    let (mut in_, mut out) = (lo, hi + 1);
    while out - in_ > 1 {
        let mid = in_ + (out - in_) / 2;
        if f(mid) {
            in_ = mid;
        } else {
            out = mid;
        }
    }
    in_
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
    use crate::hash::mix64;

    /// Uniform in `lo..=hi` from a SplitMix-style stream.
    fn range(state: &mut u64, lo: i64, hi: i64) -> i64 {
        *state = mix64(*state);
        lo + (*state % (hi - lo + 1) as u64) as i64
    }

    /// For 200 seeded spheres (radius 1 to 40 voxels, centres anywhere in fixed point within
    /// ±64 voxels, a quarter of them on a voxel centre with a whole-voxel radius so poles and
    /// other centres fall exactly on the surface), every voxel row walked by
    /// `sphere_row_spans` (`z` then `y` ascending), both over each brick row's part of the
    /// bounds and over the whole bounds in one walk, gives exactly the `isqrt` span of
    /// `row_span`.
    #[test]
    fn walked_spans_equal_solved_spans() {
        let mut state = 0x0014_0003;
        let (mut rows, mut spans, mut boundary) = (0, 0, 0);
        for i in 0..200 {
            let (c, r) = if i % 4 == 0 {
                let c = [0; 3].map(|_| range(&mut state, -64, 64) * 256 + 128);
                (c, range(&mut state, 1, 40) * 256)
            } else {
                let c = [0; 3].map(|_| range(&mut state, -64 * 256, 64 * 256));
                (c, range(&mut state, 256, 40 * 256))
            };
            let shape = Shape::Sphere { c: Fixed3(c), r };
            let (lo, hi) = shape.voxel_bounds().unwrap();
            // A window from the bounds, sometimes cut short on one side as a chunk edge
            // would cut it.
            let x_lo = lo[0] + range(&mut state, 0, 3) * i64::from(i % 3 == 1);
            let x_hi = hi[0] - range(&mut state, 0, 3) * i64::from(i % 3 == 2);
            let whole = ((lo[1], hi[1]), (lo[2], hi[2]));
            let bricks = (lo[2].div_euclid(8)..=hi[2].div_euclid(8)).flat_map(|bz| {
                (lo[1].div_euclid(8)..=hi[1].div_euclid(8)).map(move |by| {
                    (
                        ((by * 8).max(lo[1]), (by * 8 + 7).min(hi[1])),
                        ((bz * 8).max(lo[2]), (bz * 8 + 7).min(hi[2])),
                    )
                })
            });
            for (ys, zs) in std::iter::once(whole).chain(bricks) {
                let ny = ys.1 - ys.0 + 1;
                let mut out = vec![(i64::MIN, i64::MIN); (ny * (zs.1 - zs.0 + 1)) as usize];
                sphere_row_spans(Fixed3(c), r, ys, zs, (x_lo, x_hi), &mut out);
                for z in zs.0..=zs.1 {
                    for y in ys.0..=ys.1 {
                        let (a, b) = out[((y - ys.0) + ny * (z - zs.0)) as usize];
                        let walked = (a <= b).then_some((a, b));
                        assert_eq!(
                            walked,
                            shape.row_span(y, z, x_lo, x_hi),
                            "sphere {i} {shape:?} row {y} {z} x {x_lo}..={x_hi}"
                        );
                        rows += 1;
                        spans += usize::from(walked.is_some());
                        let rem = r * r - (centre(y) - c[1]).pow(2) - (centre(z) - c[2]).pow(2);
                        boundary += usize::from(
                            rem >= 0 && (x_lo..=x_hi).any(|x| (centre(x) - c[0]).pow(2) == rem),
                        );
                    }
                }
            }
        }
        println!("{rows} rows, {spans} with a span, {boundary} with a centre on the surface");
        assert!(spans >= rows / 2, "{spans} of {rows} rows hold a span");
        assert!(
            boundary >= 100,
            "{boundary} rows with a centre on the surface"
        );
    }

    thread_local! {
        /// `CapsuleRow::contains` calls on this thread.
        pub(super) static CAPSULE_TESTS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    }

    /// Over 1000 seeded capsule rows (radius 1 to 40 voxels, ends within ±64 voxels, a third
    /// of the segments parallel to `x` and a sixth of length 0), `row_span` makes at most
    /// `2 + 2 ceil(log2(w))` capsule tests for a window of `w` voxels, and at most 2 when no
    /// centre is inside.
    #[test]
    fn capsule_rows_cost_logarithmic_tests() {
        let mut state = 0x0014_0004;
        let (mut empty, mut full) = (0, 0);
        for i in 0..1000 {
            let a = [0; 3].map(|_| range(&mut state, -64 * 256, 64 * 256));
            let b = match i % 6 {
                0 => a,
                1 | 2 => [range(&mut state, -64 * 256, 64 * 256), a[1], a[2]],
                _ => [0; 3].map(|_| range(&mut state, -64 * 256, 64 * 256)),
            };
            let shape = Shape::Capsule {
                a: Fixed3(a),
                b: Fixed3(b),
                r: range(&mut state, 256, 40 * 256),
            };
            let (lo, hi) = shape.voxel_bounds().unwrap();
            let y = range(&mut state, lo[1] - 2, hi[1] + 2);
            let z = range(&mut state, lo[2] - 2, hi[2] + 2);
            let x_lo = range(&mut state, lo[0] - 4, hi[0]);
            let x_hi = x_lo + range(&mut state, 0, hi[0] - lo[0] + 8);
            let w = x_hi - x_lo + 1;
            CAPSULE_TESTS.with(|n| n.set(0));
            let span = shape.row_span(y, z, x_lo, x_hi);
            let tests = CAPSULE_TESTS.with(std::cell::Cell::get);
            let bound = 2 + 2 * u64::from(w.cast_unsigned().next_power_of_two().trailing_zeros());
            if span.is_none() {
                assert!(tests <= 2, "row {i}: {tests} tests and no centre inside");
                empty += 1;
            } else {
                assert!(tests <= bound, "row {i}: {tests} tests for a window of {w}");
                full += 1;
            }
        }
        println!("{full} rows with a span, {empty} without");
        assert!(
            full >= 300 && empty >= 100,
            "{full} with a span, {empty} without"
        );
    }

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
