//! A level in dB as a function of the listener's room size, read off its mean free path.
//!
//! A [`RoomCurve`] holds up to [`ROOM_CURVE_POINTS`] knots `(mfp_m, db)` with strictly
//! increasing mean free paths. Between two knots `(m_a, d_a)` and `(m_b, d_b)` it is linear in
//! `log(mfp)`: `d = d_a + (d_b - d_a) log(mfp / m_a) / log(m_b / m_a)`. Below the first knot it
//! holds the first value, above the last the last. A mean free path that is not finite or is at
//! or below 0 (no room yet, or a degenerate fan) reads the **last** knot, the largest room's
//! value. The knots live in a fixed array, so the curve is `Copy` and never allocates.

use crate::AudioError;

/// The most knots a [`RoomCurve`] holds.
pub const ROOM_CURVE_POINTS: usize = 8;

/// A dB level by room size (module comment).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoomCurve {
    knots: [(f32, f32); ROOM_CURVE_POINTS],
    count: usize,
}

impl RoomCurve {
    /// A curve through `knots`, `(mean free path in metres, dB)`. Refuses zero or more than
    /// [`ROOM_CURVE_POINTS`] knots, a non-finite value, a mean free path at or below 0, and mean
    /// free paths that do not strictly increase.
    pub fn new(knots: &[(f32, f32)]) -> Result<Self, AudioError> {
        if knots.is_empty() {
            return Err(AudioError::Curve("a room curve needs at least one knot"));
        }
        if knots.len() > ROOM_CURVE_POINTS {
            return Err(AudioError::Curve("a room curve holds at most 8 knots"));
        }
        if knots.iter().any(|&(m, d)| !m.is_finite() || !d.is_finite()) {
            return Err(AudioError::Curve("a room curve knot is not finite"));
        }
        if knots.iter().any(|&(m, _)| m <= 0.0) {
            return Err(AudioError::Curve(
                "a room curve knot's mean free path is not above 0",
            ));
        }
        if knots.windows(2).any(|w| w[1].0 <= w[0].0) {
            return Err(AudioError::Curve(
                "a room curve's mean free paths do not strictly increase",
            ));
        }
        let mut out = [(0.0, 0.0); ROOM_CURVE_POINTS];
        out[..knots.len()].copy_from_slice(knots);
        Ok(Self {
            knots: out,
            count: knots.len(),
        })
    }

    /// The same `db` for every room.
    pub fn flat(db: f32) -> Self {
        let mut knots = [(0.0, 0.0); ROOM_CURVE_POINTS];
        knots[0] = (1.0, db);
        Self { knots, count: 1 }
    }

    /// The knots, in order.
    pub fn knots(&self) -> &[(f32, f32)] {
        &self.knots[..self.count]
    }

    /// Whether the curve reads one value for every room.
    pub fn is_flat(&self) -> bool {
        let k = self.knots();
        k.iter().all(|&(_, d)| d == k[0].1)
    }

    /// The level at mean free path `mfp_m`, in dB (module comment).
    pub fn db_at(&self, mfp_m: f32) -> f32 {
        let k = self.knots();
        let last = k[k.len() - 1];
        if !mfp_m.is_finite() || mfp_m <= 0.0 || mfp_m >= last.0 {
            return last.1;
        }
        if mfp_m <= k[0].0 {
            return k[0].1;
        }
        for w in k.windows(2) {
            let ((ma, da), (mb, db)) = (w[0], w[1]);
            if mfp_m == mb {
                return db;
            }
            if mfp_m < mb {
                return da + (db - da) * (mfp_m / ma).ln() / (mb / ma).ln();
            }
        }
        last.1
    }

    /// The amplitude factor at `mfp_m`: `10^(db_at / 20)`.
    pub fn gain_at(&self, mfp_m: f32) -> f32 {
        10f32.powf(self.db_at(mfp_m) / 20.0)
    }
}

/// The default echo gain by room size: `(2.67 m, +30 dB)`, `(8.0, +15)`, `(24.0, +12)`,
/// `(69.0, +6)`.
pub(crate) const ECHO_KNOTS: [(f32, f32); 4] =
    [(2.67, 30.0), (8.0, 15.0), (24.0, 12.0), (69.0, 6.0)];

/// The default tail level by room size: `(2.67 m, -15 dB)`, `(8.0, 0)`.
pub(crate) const TAIL_KNOTS: [(f32, f32); 2] = [(2.67, -15.0), (8.0, 0.0)];

/// A curve from knots known to be well formed.
pub(crate) fn curve_of(knots: &[(f32, f32)]) -> RoomCurve {
    match RoomCurve::new(knots) {
        Ok(c) => c,
        Err(e) => panic!("built-in room curve: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(got: f32, want: f32) {
        assert!((got - want).abs() <= 0.01, "{got} vs {want}");
    }

    #[test]
    fn the_default_curves_read_their_knots_exactly() {
        let echo = curve_of(&ECHO_KNOTS);
        let tail = curve_of(&TAIL_KNOTS);
        assert_eq!(echo.db_at(2.67), 30.0);
        assert_eq!(echo.db_at(8.0), 15.0);
        assert_eq!(echo.db_at(24.0), 12.0);
        assert_eq!(echo.db_at(69.0), 6.0);
        assert_eq!(tail.db_at(2.67), -15.0);
        assert_eq!(tail.db_at(8.0), 0.0);
    }

    #[test]
    fn between_knots_the_curve_is_linear_in_log_mfp() {
        let echo = curve_of(&ECHO_KNOTS);
        let tail = curve_of(&TAIL_KNOTS);
        near(echo.db_at(4.0), 24.475);
        near(tail.db_at(4.0), -9.475);
        near(echo.db_at(16.0), 13.107);
        near(echo.db_at(40.0), 9.098);
        // The 100 m cube's centre fan; the figure only resembles 2 pi.
        #[expect(clippy::approx_constant, reason = "a dB figure, not 2 pi")]
        let at_100_m = 6.283;
        near(echo.db_at(65.648), at_100_m);
        assert_eq!(tail.db_at(16.0), 0.0);
    }

    #[test]
    fn outside_the_knots_the_curve_holds_and_a_degenerate_room_reads_the_last() {
        let echo = curve_of(&ECHO_KNOTS);
        let tail = curve_of(&TAIL_KNOTS);
        assert_eq!(echo.db_at(1.0), 30.0);
        assert_eq!(tail.db_at(1.0), -15.0);
        assert_eq!(echo.db_at(133.33), 6.0);
        assert_eq!(tail.db_at(133.33), 0.0);
        for mfp in [0.0, -1.0, f32::NAN] {
            assert_eq!(echo.db_at(mfp), 6.0, "{mfp}");
            assert_eq!(tail.db_at(mfp), 0.0, "{mfp}");
        }
    }

    #[test]
    fn a_malformed_curve_is_refused() {
        let bad: [&[(f32, f32)]; 9] = [
            &[],
            &[(1.0, 0.0); 9],
            &[(f32::NAN, 0.0)],
            &[(1.0, f32::INFINITY)],
            &[(f32::INFINITY, 0.0)],
            &[(0.0, 0.0)],
            &[(-2.0, 0.0), (1.0, 0.0)],
            &[(2.0, 0.0), (2.0, 1.0)],
            &[(3.0, 0.0), (2.0, 1.0)],
        ];
        for knots in bad {
            assert!(
                matches!(RoomCurve::new(knots), Err(AudioError::Curve(_))),
                "{knots:?}"
            );
        }
        let eight: Vec<(f32, f32)> = (1..=8).map(|i| (i as f32, i as f32)).collect();
        assert_eq!(RoomCurve::new(&eight).map(|c| c.knots().len()), Ok(8));
    }

    #[test]
    fn a_flat_curve_reads_one_value() {
        let flat = RoomCurve::flat(-3.0);
        for mfp in [0.0, 0.5, 1.0, 8.0, 1000.0, f32::NAN] {
            assert_eq!(flat.db_at(mfp), -3.0);
        }
        assert!(flat.is_flat());
        assert!(!curve_of(&ECHO_KNOTS).is_flat());
        assert_eq!(RoomCurve::flat(0.0).gain_at(4.0), 1.0);
    }
}
