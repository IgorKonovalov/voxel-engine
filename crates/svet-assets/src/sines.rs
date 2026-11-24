//! The sines of 0 to 90 degrees in half-degree steps, in Q14, so a rig's whole-degree
//! rotations become quaternions without the platform's `sin` (its last bit may differ between
//! platforms; this table does not).

/// `SINES[i]` is `16384 sin(i / 2 degrees)` rounded to the nearest integer, `i` from 0 to 180.
/// Generated once in double precision; the nearest entry to a half is `i` = 94 (11982.499), so
/// every rounding is clear of the tie.
pub(crate) const SINES: [i16; 181] = [
    0, 143, 286, 429, 572, 715, 857, 1000, 1143, 1285, 1428, 1570, 1713, 1855, 1997, 2139, 2280,
    2422, 2563, 2704, 2845, 2986, 3126, 3266, 3406, 3546, 3686, 3825, 3964, 4102, 4240, 4378, 4516,
    4653, 4790, 4927, 5063, 5199, 5334, 5469, 5604, 5738, 5872, 6005, 6138, 6270, 6402, 6533, 6664,
    6794, 6924, 7053, 7182, 7311, 7438, 7565, 7692, 7818, 7943, 8068, 8192, 8316, 8438, 8561, 8682,
    8803, 8923, 9043, 9162, 9280, 9397, 9514, 9630, 9746, 9860, 9974, 10087, 10199, 10311, 10422,
    10531, 10641, 10749, 10856, 10963, 11069, 11174, 11278, 11381, 11484, 11585, 11686, 11786,
    11885, 11982, 12080, 12176, 12271, 12365, 12458, 12551, 12642, 12733, 12822, 12911, 12998,
    13085, 13170, 13255, 13338, 13421, 13502, 13583, 13662, 13741, 13818, 13894, 13970, 14044,
    14117, 14189, 14260, 14330, 14399, 14466, 14533, 14598, 14663, 14726, 14788, 14849, 14909,
    14968, 15025, 15082, 15137, 15191, 15244, 15296, 15346, 15396, 15444, 15491, 15537, 15582,
    15626, 15668, 15709, 15749, 15788, 15826, 15862, 15897, 15931, 15964, 15996, 16026, 16055,
    16083, 16110, 16135, 16159, 16182, 16204, 16225, 16244, 16262, 16279, 16294, 16309, 16322,
    16333, 16344, 16353, 16362, 16368, 16374, 16378, 16382, 16383, 16384,
];

/// `16384 sin` and `16384 cos` of `half_degrees / 2` degrees, for any integer.
pub(crate) fn sin_cos_half_degrees(half_degrees: i64) -> (i16, i16) {
    // Reduce to [0, 720) half degrees, one full turn.
    let a = half_degrees.rem_euclid(720) as usize;
    let sin = |a: usize| -> i16 {
        match a {
            0..=180 => SINES[a],
            181..=360 => SINES[360 - a],
            361..=540 => -SINES[a - 360],
            _ => -SINES[720 - a],
        }
    };
    (sin(a), sin((a + 180) % 720))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_the_rounded_sine() {
        for (i, &s) in SINES.iter().enumerate() {
            let want = (16384.0 * (i as f64 * 0.5).to_radians().sin()).round();
            assert_eq!(f64::from(s), want, "entry {i}");
        }
        assert_eq!(
            (SINES[0], SINES[60], SINES[90], SINES[180]),
            (0, 8192, 11585, 16384)
        );
    }

    #[test]
    fn every_quadrant_reads_the_table() {
        assert_eq!(sin_cos_half_degrees(0), (0, 16384));
        assert_eq!(sin_cos_half_degrees(30), (4240, 15826));
        assert_eq!(sin_cos_half_degrees(180), (16384, 0));
        assert_eq!(sin_cos_half_degrees(210), (15826, -4240));
        assert_eq!(sin_cos_half_degrees(390), (-4240, -15826));
        assert_eq!(sin_cos_half_degrees(-30), (-4240, 15826));
        assert_eq!(sin_cos_half_degrees(720 + 30), (4240, 15826));
    }
}
