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
