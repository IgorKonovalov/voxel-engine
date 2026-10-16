//! Fixed-point positions at 1/256 voxel, the unit of every edit-shape parameter.
//!
//! The centre of voxel `i` is `256 i + 128`. Shape predicates compare squared distances in
//! integers, so no float ever decides which voxel an edit touches.

use crate::coords::VoxelCoord;

/// Fixed-point units per voxel.
pub const FIXED_ONE: i64 = 256;
/// Offset of a voxel's centre from its minimum corner, in fixed-point units.
pub const FIXED_HALF: i64 = FIXED_ONE / 2;

/// A position in fixed point, 1/256 voxel per unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Fixed3(pub [i64; 3]);

impl From<[i64; 3]> for Fixed3 {
    fn from(v: [i64; 3]) -> Self {
        Self(v)
    }
}

/// The fixed-point centre of voxel `v`.
pub fn centre_of_voxel(v: impl Into<VoxelCoord>) -> Fixed3 {
    let v = v.into();
    Fixed3(v.0.map(|c| i64::from(c) * FIXED_ONE + FIXED_HALF))
}

/// Fixed-point centre of voxel index `i` along one axis.
pub(crate) fn centre(i: i64) -> i64 {
    i * FIXED_ONE + FIXED_HALF
}

/// Smallest voxel index whose centre is `>= f`.
pub(crate) fn first_centre_at_or_above(f: i64) -> i64 {
    // centre(i) >= f  <=>  i >= (f - 128) / 256, rounded up.
    -(-(f - FIXED_HALF)).div_euclid(FIXED_ONE)
}

/// Largest voxel index whose centre is `<= f`.
pub(crate) fn last_centre_at_or_below(f: i64) -> i64 {
    (f - FIXED_HALF).div_euclid(FIXED_ONE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centre_bounds_round_toward_the_inside() {
        assert_eq!(first_centre_at_or_above(128), 0);
        assert_eq!(first_centre_at_or_above(129), 1);
        assert_eq!(first_centre_at_or_above(-128), -1);
        assert_eq!(first_centre_at_or_above(-129), -1);
        assert_eq!(last_centre_at_or_below(128), 0);
        assert_eq!(last_centre_at_or_below(127), -1);
        assert_eq!(last_centre_at_or_below(-128), -1);
        assert_eq!(last_centre_at_or_below(-129), -2);
    }
}
