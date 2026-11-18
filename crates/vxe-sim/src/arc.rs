//! Integer direction arithmetic for the surface mover: lengths, projection, the shortest-arc
//! rotation as a Q14 quaternion and its application to a vector.
//!
//! `shortest_arc` is a copy of `vxe_entity::Quat::shortest_arc`'s rule, kept here so this
//! crate's only dependency stays `vxe-world`; a committed vector of direction pairs hashes to
//! one constant in a test of each crate, so the copies cannot drift.

use crate::body::isqrt;

/// The fixed length directions are normalised to.
pub const DIR_ONE: i64 = 256;

/// Q14 one: the scale of a quaternion's components.
const Q_ONE: i128 = 1 << 14;

pub(crate) fn dot(a: [i64; 3], b: [i64; 3]) -> i128 {
    (0..3).map(|k| i128::from(a[k]) * i128::from(b[k])).sum()
}

fn cross128(a: [i128; 3], b: [i128; 3]) -> [i128; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub(crate) fn cross(a: [i64; 3], b: [i64; 3]) -> [i64; 3] {
    cross128(a.map(i128::from), b.map(i128::from)).map(to_i64)
}

fn to_i64(v: i128) -> i64 {
    v.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}

/// `n / d` rounded to nearest, halves away from zero; `d > 0`.
pub(crate) fn rdiv(n: i128, d: i128) -> i128 {
    if n >= 0 {
        (2 * n + d) / (2 * d)
    } else {
        -((-2 * n + d) / (2 * d))
    }
}

/// The floor of `|v|`.
pub(crate) fn length(v: [i64; 3]) -> i64 {
    isqrt(dot(v, v) as u128) as i64
}

/// `v` scaled to length [`DIR_ONE`], each component rounded to nearest: exact for an axis
/// direction. The length is taken at 256 times the scale (the floor of the root of
/// `|v|² 2^16`), so a short vector such as a small gradient normalises to within a unit.
/// Zero for zero.
pub fn normalise(v: [i64; 3]) -> [i64; 3] {
    let v = reduce(v);
    let len = isqrt((dot(v, v) << 16) as u128) as i128;
    if len == 0 {
        return [0; 3];
    }
    v.map(|c| rdiv(i128::from(c) * i128::from(DIR_ONE) * 256, len) as i64)
}

/// `v` without its component along `n`: `v - n (v . n) / |n|²`, rounded to nearest. Zero
/// when `n` is zero.
pub(crate) fn project(v: [i64; 3], n: [i64; 3]) -> [i64; 3] {
    let nn = dot(n, n);
    if nn == 0 {
        return v;
    }
    let d = dot(v, n);
    [0, 1, 2].map(|k| to_i64(i128::from(v[k]) - rdiv(i128::from(n[k]) * d, nn)))
}

/// Shifts `v` right until every component is under `2^24` in magnitude, so the products of
/// the arc stay inside `i128`. Directions of the mover's scale are unchanged.
fn reduce(mut v: [i64; 3]) -> [i64; 3] {
    while v.iter().any(|c| c.unsigned_abs() >= 1 << 24) {
        v = v.map(|c| c >> 1);
    }
    v
}

/// The rotation taking direction `a` to direction `b`, as a Q14 quaternion `[w, x, y, z]`:
/// `w = |a||b| + a . b`, `(x, y, z) = a x b`, normalised to length 16384 with each component
/// rounded to nearest. For `a == -b` (the cross product zero and `w` zero) a half turn about
/// `e_k - a (a_k) / |a|²`, with `e_k` the axis direction of lowest index not parallel to `a`
/// (the axis itself when `a` is perpendicular to it). The identity when either is zero.
/// Components are first reduced below `2^24`.
pub fn shortest_arc(a: [i64; 3], b: [i64; 3]) -> [i16; 4] {
    let (a, b) = (reduce(a), reduce(b));
    let (la, lb) = (dot(a, a), dot(b, b));
    if la == 0 || lb == 0 {
        return [Q_ONE as i16, 0, 0, 0];
    }
    let ab = isqrt((la * lb) as u128) as i128;
    let w = ab + dot(a, b);
    let xyz = cross128(a.map(i128::from), b.map(i128::from));
    let q = if w <= 0 || xyz == [0; 3] && dot(a, b) < 0 {
        // Antiparallel: `w` is zero up to the root's rounding.
        let k = (0..3)
            .find(|&k| {
                let mut e = [0i64; 3];
                e[k] = 1;
                cross(a, e) != [0; 3]
            })
            .unwrap_or(0);
        let axis = [0, 1, 2].map(|j| {
            let e = if j == k { la } else { 0 };
            e - i128::from(a[j]) * i128::from(a[k])
        });
        [0, axis[0], axis[1], axis[2]]
    } else {
        [w, xyz[0], xyz[1], xyz[2]]
    };
    normalise_q(q)
}

/// `q` scaled to length 16384, components rounded to nearest.
fn normalise_q(q: [i128; 4]) -> [i16; 4] {
    // Bring the components under 2^60 so their squares sum inside `i128`.
    let mut q = q;
    while q.iter().any(|c| c.unsigned_abs() >= 1 << 60) {
        q = q.map(|c| c >> 1);
    }
    let len = isqrt(q.iter().map(|c| c * c).sum::<i128>() as u128) as i128;
    if len == 0 {
        return [Q_ONE as i16, 0, 0, 0];
    }
    q.map(|c| rdiv(c * Q_ONE, len) as i16)
}

/// `v` rotated by the Q14 quaternion `q`: `v + 2 w (u x v) + 2 u x (u x v)` with `u` the
/// vector part, each term at its scale and the sum rounded to nearest once.
pub fn rotate(q: [i16; 4], v: [i64; 3]) -> [i64; 3] {
    let w = i128::from(q[0]);
    let u = [q[1], q[2], q[3]].map(i128::from);
    let v128 = v.map(i128::from);
    // `t = 2 (u x v)`, at scale 2^14.
    let t = cross128(u, v128).map(|c| 2 * c);
    let ut = cross128(u, t);
    [0, 1, 2].map(|k| {
        let n = v128[k] * Q_ONE * Q_ONE + w * t[k] + ut[k];
        to_i64(rdiv(n, Q_ONE * Q_ONE))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_axis_arc_is_a_permutation() {
        let q = shortest_arc([0, 0, 256], [256, 0, 0]);
        assert_eq!(q, [11585, 0, 11585, 0]);
        assert_eq!(rotate(q, [0, 0, 256]), [256, 0, 0]);
        assert_eq!(rotate(q, [256, 0, 0]), [0, 0, -256]);
        let h = shortest_arc([0, 0, 256], [0, 0, -256]);
        assert_eq!(h, [0, 16384, 0, 0]);
        assert_eq!(rotate(h, [0, 0, 256]), [0, 0, -256]);
    }

    #[test]
    fn normalise_and_project_are_exact_on_axes() {
        assert_eq!(normalise([0, 0, 3]), [0, 0, 256]);
        assert_eq!(normalise([-7, 0, 0]), [-256, 0, 0]);
        assert_eq!(project([256, 0, 256], [0, 0, 256]), [256, 0, 0]);
    }
}
