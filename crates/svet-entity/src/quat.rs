//! Unit quaternions in Q14 and the integer arithmetic on them.
//!
//! A component is an `i16` with [`Q14_ONE`] = 16384 standing for 1, in `w, x, y, z` order. Every
//! operation is integer arithmetic with round-half-away-from-zero, so a pose composes to the same
//! bits on every platform. A product is renormalised with an integer square root, so a chain of
//! products stays a unit quaternion to within one Q14 step per component.

/// The Q14 fixed-point one.
pub const Q14_ONE: i32 = 1 << 14;

/// A rotation as a quaternion in Q14, `w, x, y, z`. A quaternion that is not unit length is
/// read as its normalised self; the zero quaternion reads as the identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Quat(pub [i16; 4]);

impl Default for Quat {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// A rotation matrix in Q14, rows of columns: `m[r][c]`.
pub(crate) type Mat3 = [[i32; 3]; 3];

impl Quat {
    pub const IDENTITY: Self = Self([Q14_ONE as i16, 0, 0, 0]);

    /// The same rotation scaled to unit length in Q14 (a zero quaternion becomes the identity).
    pub fn normalised(self) -> Self {
        normalise(self.0.map(i128::from))
    }

    /// The rotation matrix in Q14. Each entry divides by the squared length, so a quaternion
    /// a few Q14 steps off unit length still gives a rotation; the axis quarter and half turns
    /// give entries of exactly 0 and +-16384.
    pub(crate) fn matrix(self) -> Mat3 {
        let [w, x, y, z] = self.0.map(i64::from);
        let n2 = w * w + x * x + y * y + z * z;
        if n2 == 0 {
            return Self::IDENTITY.matrix();
        }
        let one = i64::from(Q14_ONE);
        let e = |num: i64| div_round(i128::from(num) * i128::from(one), i128::from(n2)) as i32;
        [
            [
                e(n2 - 2 * (y * y + z * z)),
                e(2 * (x * y - w * z)),
                e(2 * (x * z + w * y)),
            ],
            [
                e(2 * (x * y + w * z)),
                e(n2 - 2 * (x * x + z * z)),
                e(2 * (y * z - w * x)),
            ],
            [
                e(2 * (x * z - w * y)),
                e(2 * (y * z + w * x)),
                e(n2 - 2 * (x * x + y * y)),
            ],
        ]
    }
}

impl Quat {
    /// `v` rotated by this quaternion, through its Q14 matrix ([`Quat::matrix`]'s rounding).
    pub fn rotate_vector(self, v: [i64; 3]) -> [i64; 3] {
        rotate(&self.matrix(), v)
    }

    /// The rotation taking the model's rest frame (facing `-y`, `+z` up) to the frame whose up
    /// is `up` and whose forward is `forward` made orthogonal to `up`. Built from the rotation
    /// matrix whose columns are the images of `+x` (`up x forward`), `+y` (`-forward`) and
    /// `+z` (`up`), each scaled to Q14 with the integer square root, and converted with the
    /// branch on the largest of the trace and the diagonal; `w >= 0`. `None` when either
    /// vector is zero or they are parallel. Components are first reduced below `2^24`.
    pub fn from_frame(forward: [i64; 3], up: [i64; 3]) -> Option<Quat> {
        let (f, u) = (reduce(forward), reduce(up));
        if dot(f, f) == 0 || dot(u, u) == 0 || cross(f, u) == [0; 3] {
            return None;
        }
        let one = i64::from(Q14_ONE);
        let u = unit(u, one);
        let f = unit(project(f, u), one);
        let x = unit(cross(u, f), one);
        if f == [0; 3] || x == [0; 3] {
            return None;
        }
        // Columns: the images of +x, +y and +z.
        let cols = [x, f.map(|c| -c), u];
        let m = |r: usize, c: usize| i128::from(cols[c][r]);
        let s = i128::from(one);
        let trace = m(0, 0) + m(1, 1) + m(2, 2);
        let q = if trace >= m(0, 0) && trace >= m(1, 1) && trace >= m(2, 2) {
            [
                s + trace,
                m(2, 1) - m(1, 2),
                m(0, 2) - m(2, 0),
                m(1, 0) - m(0, 1),
            ]
        } else if m(0, 0) >= m(1, 1) && m(0, 0) >= m(2, 2) {
            [
                m(2, 1) - m(1, 2),
                s + m(0, 0) - m(1, 1) - m(2, 2),
                m(0, 1) + m(1, 0),
                m(0, 2) + m(2, 0),
            ]
        } else if m(1, 1) >= m(2, 2) {
            [
                m(0, 2) - m(2, 0),
                m(0, 1) + m(1, 0),
                s - m(0, 0) + m(1, 1) - m(2, 2),
                m(1, 2) + m(2, 1),
            ]
        } else {
            [
                m(1, 0) - m(0, 1),
                m(0, 2) + m(2, 0),
                m(1, 2) + m(2, 1),
                s - m(0, 0) - m(1, 1) + m(2, 2),
            ]
        };
        let q = if q[0] < 0 { q.map(|c| -c) } else { q };
        Some(normalise(q))
    }

    /// The rotation taking direction `a` to direction `b`: `w = |a||b| + a . b`,
    /// `(x, y, z) = a x b`, normalised to Q14 with each component rounded half away from zero.
    /// For `a == -b` a half turn about `e_k - a (a_k) / |a|²`, with `e_k` the axis direction of
    /// lowest index not parallel to `a` (the axis itself when `a` is perpendicular to it). The
    /// identity when either is zero. Components are first reduced below `2^24`.
    ///
    /// `svet_sim`'s surface mover carries a copy of this rule (its only dependency is
    /// `svet-world`); a committed vector of direction pairs hashes to one constant in a test of
    /// each crate, so the two cannot drift.
    pub fn shortest_arc(a: [i64; 3], b: [i64; 3]) -> Quat {
        let (a, b) = (reduce(a), reduce(b));
        let (la, lb) = (dot(a, a), dot(b, b));
        if la == 0 || lb == 0 {
            return Self::IDENTITY;
        }
        let ab = isqrt((la * lb).cast_unsigned()).cast_signed();
        let w = ab + dot(a, b);
        let xyz = cross128(a.map(i128::from), b.map(i128::from));
        let q = if w <= 0 || xyz == [0; 3] && dot(a, b) < 0 {
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
        normalise(q)
    }
}

fn dot(a: [i64; 3], b: [i64; 3]) -> i128 {
    (0..3).map(|k| i128::from(a[k]) * i128::from(b[k])).sum()
}

fn cross128(a: [i128; 3], b: [i128; 3]) -> [i128; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn cross(a: [i64; 3], b: [i64; 3]) -> [i64; 3] {
    cross128(a.map(i128::from), b.map(i128::from)).map(|c| c as i64)
}

/// Shifts `v` right until every component is under `2^24` in magnitude.
fn reduce(mut v: [i64; 3]) -> [i64; 3] {
    while v.iter().any(|c| c.unsigned_abs() >= 1 << 24) {
        v = v.map(|c| c >> 1);
    }
    v
}

/// `v` scaled to length `len`, rounded half away from zero; zero for zero.
fn unit(v: [i64; 3], len: i64) -> [i64; 3] {
    let n = isqrt(dot(v, v).cast_unsigned()).cast_signed();
    if n == 0 {
        return [0; 3];
    }
    v.map(|c| div_round(i128::from(c) * i128::from(len), n) as i64)
}

/// `v` without its component along the unit-scaled `n`, rounded half away from zero.
fn project(v: [i64; 3], n: [i64; 3]) -> [i64; 3] {
    let nn = dot(n, n);
    let d = dot(v, n);
    [0, 1, 2].map(|k| (i128::from(v[k]) - div_round(i128::from(n[k]) * d, nn)) as i64)
}

impl std::ops::Mul for Quat {
    type Output = Self;

    /// The product `self * rhs`: rotate by `rhs`, then by `self`. Renormalised to unit length in
    /// Q14; a zero product reads as the identity.
    fn mul(self, rhs: Self) -> Self {
        let [aw, ax, ay, az] = self.0.map(i64::from);
        let [bw, bx, by, bz] = rhs.0.map(i64::from);
        // Q28 components.
        let p = [
            aw * bw - ax * bx - ay * by - az * bz,
            aw * bx + ax * bw + ay * bz - az * by,
            aw * by - ax * bz + ay * bw + az * bx,
            aw * bz + ax * by - ay * bx + az * bw,
        ];
        normalise(p.map(i128::from))
    }
}

/// The normalised linear blend `a * (den - num) + b * num`, along the shorter arc: `b` is negated
/// first when its dot product with `a` is negative (`q` and `-q` are the same rotation, and the
/// blend toward the nearer of the two turns the short way). `0 <= num <= den`, `den > 0`.
///
/// The blend's speed is uneven: it turns faster in the middle than at the ends, by more the
/// farther apart the keys are (a half turn sampled at a quarter of the way is 36.9 degrees, not
/// 45).
pub(crate) fn nlerp(a: Quat, b: Quat, num: i64, den: i64) -> Quat {
    debug_assert!(den > 0 && (0..=den).contains(&num));
    let a = a.0.map(i64::from);
    let mut b = b.0.map(i64::from);
    let dot: i64 = (0..4).map(|k| a[k] * b[k]).sum();
    if dot < 0 {
        b = b.map(|c| -c);
    }
    normalise([0, 1, 2, 3].map(|k| i128::from(a[k] * (den - num) + b[k] * num)))
}

/// Scales a quaternion given at any fixed-point scale to unit length in Q14.
fn normalise(q: [i128; 4]) -> Quat {
    // Bring the components under 2^60 so their squares sum inside `i128`.
    let mut q = q;
    while q.iter().any(|c| c.unsigned_abs() >= 1 << 60) {
        q = q.map(|c| c >> 1);
    }
    let n2: i128 = q.iter().map(|c| c * c).sum();
    if n2 == 0 {
        return Quat::IDENTITY;
    }
    let n = isqrt(n2.cast_unsigned()).cast_signed();
    let one = i128::from(Q14_ONE);
    // |c| <= n, so every component lands in [-16384, 16384].
    Quat(q.map(|c| div_round(c * one, n) as i16))
}

/// `m * v / 16384`, rounded half away from zero.
pub(crate) fn rotate(m: &Mat3, v: [i64; 3]) -> [i64; 3] {
    let one = i128::from(Q14_ONE);
    [0, 1, 2].map(|r| {
        let s: i128 = (0..3).map(|c| i128::from(m[r][c]) * i128::from(v[c])).sum();
        div_round(s, one) as i64
    })
}

/// `m^T * v`, unscaled (the result carries the extra factor 16384).
pub(crate) fn rotate_back_q14(m: &Mat3, v: [i128; 3]) -> [i128; 3] {
    [0, 1, 2].map(|c| (0..3).map(|r| i128::from(m[r][c]) * v[r]).sum())
}

/// `n / d` for `d > 0`, rounded half away from zero.
pub(crate) fn div_round(n: i128, d: i128) -> i128 {
    debug_assert!(d > 0);
    let q = (2 * n.abs() + d) / (2 * d);
    if n < 0 { -q } else { q }
}

/// The integer square root, rounded down.
pub(crate) fn isqrt(n: u128) -> u128 {
    if n < 2 {
        return n;
    }
    // Newton's iteration from an over-estimate converges down to floor(sqrt(n)).
    let mut x = 1u128 << (128 - n.leading_zeros()).div_ceil(2);
    loop {
        let y = (x + n / x) / 2;
        if y >= x {
            return x;
        }
        x = y;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isqrt_rounds_down() {
        for n in 0u128..2000 {
            let r = isqrt(n);
            assert!(r * r <= n && (r + 1) * (r + 1) > n, "{n}");
        }
        assert_eq!(isqrt(u128::from(u64::MAX)), u128::from(u32::MAX));
        assert_eq!(isqrt(1u128 << 120), 1u128 << 60);
    }

    #[test]
    fn half_turns_compose_exactly() {
        let y180 = Quat([0, 0, Q14_ONE as i16, 0]);
        // Two half turns about y are a full turn: -1, the same rotation as the identity.
        assert_eq!(y180 * y180, Quat([-(Q14_ONE as i16), 0, 0, 0]));
        assert_eq!(y180 * Quat::IDENTITY, y180);
        assert_eq!(
            y180.matrix(),
            [[-16384, 0, 0], [0, 16384, 0], [0, 0, -16384]]
        );
        assert_eq!(rotate(&y180.matrix(), [2048, 3, 5]), [-2048, 3, -5]);
    }

    #[test]
    fn a_product_stays_unit_length() {
        // 45 degrees about z, cos and sin of 22.5 degrees in Q14.
        let q = Quat([15137, 0, 0, 6270]);
        let mut acc = Quat::IDENTITY;
        for _ in 0..64 {
            acc = acc * q;
            let n2: i64 = acc.0.iter().map(|&c| i64::from(c) * i64::from(c)).sum();
            let one = i64::from(Q14_ONE) * i64::from(Q14_ONE);
            assert!((n2 - one).abs() <= 4 * i64::from(Q14_ONE), "{acc:?}");
        }
    }

    #[test]
    fn div_round_is_symmetric() {
        assert_eq!(div_round(5, 2), 3);
        assert_eq!(div_round(-5, 2), -3);
        assert_eq!(div_round(4, 3), 1);
        assert_eq!(div_round(-4, 3), -1);
    }
}
