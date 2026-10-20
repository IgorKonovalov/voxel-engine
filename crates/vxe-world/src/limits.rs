//! The documented range of shape, ray and query parameters, checked before any arithmetic.
//!
//! Shape math squares coordinate differences in `i64` and multiplies squared lengths in
//! `i128`; both stay exact while every coordinate and radius lies within `±coord_max` fixed
//! units (`shape.rs`). A parameter outside the range is refused with
//! [`EditError::OutOfRange`], naming the [`Field`], and the world is left unchanged.

use crate::edit::EditError;
use crate::fixed::Fixed3;
use crate::shape::Shape;

/// Which parameter of a shape, ray or box query was out of range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Field {
    /// A position: a sphere's centre, a box corner, a capsule end, a ray's origin.
    Coord,
    /// A sphere's or a capsule's radius.
    Radius,
    /// A ray's `max_t`.
    RayLength,
    /// A ray's direction.
    Direction,
    /// One axis of a box query, in voxels.
    QueryAxis,
    /// The voxel count of a box query.
    QueryVolume,
}

impl Field {
    /// Every field, in wire tag order (tag = index + 1).
    pub const ALL: [Field; 6] = [
        Field::Coord,
        Field::Radius,
        Field::RayLength,
        Field::Direction,
        Field::QueryAxis,
        Field::QueryVolume,
    ];
}

impl std::fmt::Display for Field {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Coord => "coordinate",
            Self::Radius => "radius",
            Self::RayLength => "ray length",
            Self::Direction => "ray direction",
            Self::QueryAxis => "query axis",
            Self::QueryVolume => "query volume",
        })
    }
}

/// The accepted range of edit and query parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WorldLimits {
    /// The largest magnitude, inclusive, of every fixed-point coordinate and radius.
    pub coord_max: i64,
    /// The largest ray `max_t`, inclusive, in fixed units.
    pub ray_max_t: i64,
    /// The most voxels, inclusive, along one axis of a box query.
    pub query_axis_max: u32,
    /// The most voxels, inclusive, in a box query.
    pub query_voxels_max: u64,
}

impl WorldLimits {
    /// `±2^30` fixed units (`±4 M` voxels) for coordinates and radii, rays of at most `2^22`
    /// fixed units (16 384 voxels at `|dir| = 256`), box queries of at most 1 024 voxels per
    /// axis and `2^30` voxels.
    pub const DEFAULT: Self = Self {
        coord_max: 1 << 30,
        ray_max_t: 1 << 22,
        query_axis_max: 1024,
        query_voxels_max: 1 << 30,
    };

    fn coord(&self, p: Fixed3) -> Result<(), EditError> {
        if p.0
            .iter()
            .all(|v| v.unsigned_abs() <= self.coord_max.unsigned_abs())
        {
            Ok(())
        } else {
            Err(EditError::OutOfRange { what: Field::Coord })
        }
    }

    fn radius(&self, r: i64) -> Result<(), EditError> {
        if r.unsigned_abs() <= self.coord_max.unsigned_abs() {
            Ok(())
        } else {
            Err(EditError::OutOfRange {
                what: Field::Radius,
            })
        }
    }
}

impl Default for WorldLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl Shape {
    /// `Ok` when every coordinate and radius lies within `±limits.coord_max`; otherwise the
    /// first field out of range, positions before radii.
    pub fn check(&self, limits: &WorldLimits) -> Result<(), EditError> {
        match *self {
            Shape::Sphere { c, r } => {
                limits.coord(c)?;
                limits.radius(r)
            }
            Shape::Box_ { min, max } => {
                limits.coord(min)?;
                limits.coord(max)
            }
            Shape::Capsule { a, b, r } => {
                limits.coord(a)?;
                limits.coord(b)?;
                limits.radius(r)
            }
        }
    }
}
