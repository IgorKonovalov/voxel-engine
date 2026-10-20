//! The accepted range of edit, ray, query and region parameters, checked before any
//! arithmetic.
//!
//! Each [`World`](crate::World) holds a [`WorldLimits`] ([`WorldLimits::DEFAULT`] unless built
//! [`with_limits`](crate::World::with_limits)). A parameter outside it is refused with
//! [`EditError::OutOfRange`] naming the [`Field`], and nothing is changed. Where each bound
//! comes from:
//!
//! - **Coordinates and radii** within `±coord_max` keep the sphere's `i64` squared distances
//!   below `3 x 2^60` for any point in its voxel bounds; the box multiplies nothing.
//! - **Capsule length.** The capsule forms `ww x len2` in `i128`, with `len2 = |b - a|^2` and
//!   `ww = |p - a|^2`. With `C = coord_max` and `L` bounding each axis of `b - a`, a point in
//!   the voxel bounds has `|p[k] - a[k]| <= L + C`, so `ww x len2 <= 9 L^2 (L + C)^2`; with
//!   `L = C = 2^30` that is `9 x 2^122`, about `2^125.2`, below `i128::MAX`. Without the
//!   per-axis cap, ends at `±2^30` are `2^31` apart and the product overflows.
//! - **Edit extent.** An edit whose voxel bounds span at most `edit_axis_max` (1 024) voxels
//!   per axis meets at most 17 chunks per axis, so the residency walk visits at most 4 913
//!   chunk coordinates.
//! - **Ray reach.** The ray moves `dir[k] x max_t / 256` fixed units along axis `k`; capping
//!   that per axis (formed in `i128`, exact) bounds the walk whatever `dir` is: `2^22` fixed
//!   units is 16 384 voxels, at most 258 cell steps along an axis through empty chunks and
//!   772 on a diagonal.
//! - **Box queries** visit every chunk coordinate in the box, so their extent and volume are
//!   capped.
//! - **Region cells.** `drop_region` and `remove_region` scan a column per cell; 4 096 is the
//!   largest region the default support query reports.

use crate::edit::EditError;
use crate::fixed::Fixed3;
use crate::shape::Shape;

/// Which parameter of a shape, ray, box query or region op was out of range. The wire field
/// byte is the index in [`Field::ALL`] plus 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Field {
    /// A position: a sphere's centre, a box corner, a capsule end, a ray's origin.
    Coord,
    /// A sphere's or a capsule's radius.
    Radius,
    /// A capsule's `b - a` on some axis.
    CapsuleLength,
    /// The voxel bounds of an edit's shape on some axis.
    EditExtent,
    /// A ray's reach, `|dir[k]| x max_t / 256`, on some axis.
    RayLength,
    /// One axis of a box query, in voxels.
    QueryAxis,
    /// The voxel count of a box query.
    QueryVolume,
    /// The cell count of a drop or a removal.
    RegionCells,
}

impl Field {
    /// Every field, in wire order.
    pub const ALL: [Field; 8] = [
        Field::Coord,
        Field::Radius,
        Field::CapsuleLength,
        Field::EditExtent,
        Field::RayLength,
        Field::QueryAxis,
        Field::QueryVolume,
        Field::RegionCells,
    ];
}

impl std::fmt::Display for Field {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Coord => "coordinate",
            Self::Radius => "radius",
            Self::CapsuleLength => "capsule length",
            Self::EditExtent => "edit extent",
            Self::RayLength => "ray length",
            Self::QueryAxis => "query axis",
            Self::QueryVolume => "query volume",
            Self::RegionCells => "region cell count",
        })
    }
}

/// The accepted range of edit, ray, query and region parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WorldLimits {
    /// The largest magnitude, inclusive, of every fixed-point coordinate and radius.
    pub coord_max: i64,
    /// The largest `|b[k] - a[k]|` of a capsule, inclusive, in fixed units.
    pub capsule_axis_max: i64,
    /// The most voxels, inclusive, an edit's voxel bounds span on one axis.
    pub edit_axis_max: u32,
    /// The largest reach of a ray on one axis, inclusive, in fixed units:
    /// `|dir[k]| x max(max_t, 0) <= 256 x ray_reach_max`.
    pub ray_reach_max: i64,
    /// The most voxels, inclusive, along one axis of a box query.
    pub query_axis_max: u32,
    /// The most voxels, inclusive, in a box query.
    pub query_voxels_max: u64,
    /// The most cells, inclusive, in a drop's or a removal's cell list.
    pub region_cells_max: u32,
}

impl WorldLimits {
    /// `±2^30` fixed units (`±4 M` voxels) for coordinates and radii, capsules at most `2^30`
    /// fixed units long per axis, edits at most 1 024 voxels per axis, rays reaching at most
    /// `2^22` fixed units (16 384 voxels) per axis, box queries of at most 1 024 voxels per
    /// axis and `2^30` voxels, region ops of at most 4 096 cells.
    pub const DEFAULT: Self = Self {
        coord_max: 1 << 30,
        capsule_axis_max: 1 << 30,
        edit_axis_max: 1024,
        ray_reach_max: 1 << 22,
        query_axis_max: 1024,
        query_voxels_max: 1 << 30,
        region_cells_max: 4096,
    };

    pub(crate) fn coord(&self, p: Fixed3) -> Result<(), EditError> {
        refuse_unless(
            p.0.iter()
                .all(|v| v.unsigned_abs() <= self.coord_max.unsigned_abs()),
            Field::Coord,
        )
    }

    fn radius(&self, r: i64) -> Result<(), EditError> {
        refuse_unless(
            r.unsigned_abs() <= self.coord_max.unsigned_abs(),
            Field::Radius,
        )
    }

    /// `Ok` when `origin` is in range and the ray `origin + dir x t / 256`, `t <= max_t`,
    /// reaches at most `ray_reach_max` fixed units on every axis.
    pub fn check_ray(&self, origin: Fixed3, dir: [i64; 3], max_t: i64) -> Result<(), EditError> {
        self.coord(origin)?;
        // |dir[k]| <= 2^63 and max_t <= 2^63: the product is exact in i128.
        let t = i128::from(max_t.max(0));
        let reach = 256 * i128::from(self.ray_reach_max.max(0));
        refuse_unless(
            dir.iter()
                .all(|&d| i128::from(d.unsigned_abs()) * t <= reach),
            Field::RayLength,
        )
    }

    /// `Ok` when the inclusive voxel box `lo..=hi` (empty when reversed on an axis) spans at
    /// most `query_axis_max` voxels per axis and `query_voxels_max` voxels.
    pub fn check_query(&self, lo: [i32; 3], hi: [i32; 3]) -> Result<(), EditError> {
        if (0..3).any(|k| lo[k] > hi[k]) {
            return Ok(());
        }
        let span = [0, 1, 2].map(|k| (i64::from(hi[k]) - i64::from(lo[k]) + 1).cast_unsigned());
        refuse_unless(
            span.iter().all(|&n| n <= u64::from(self.query_axis_max)),
            Field::QueryAxis,
        )?;
        // Each span is at most 2^32, so the product of three fits u128.
        let volume: u128 = span.iter().map(|&n| u128::from(n)).product();
        refuse_unless(
            volume <= u128::from(self.query_voxels_max),
            Field::QueryVolume,
        )
    }

    /// `Ok` when a region op's cell list holds at most `region_cells_max` cells.
    pub fn check_region(&self, cells: usize) -> Result<(), EditError> {
        refuse_unless(
            u64::try_from(cells).unwrap_or(u64::MAX) <= u64::from(self.region_cells_max),
            Field::RegionCells,
        )
    }
}

fn refuse_unless(ok: bool, what: Field) -> Result<(), EditError> {
    if ok {
        Ok(())
    } else {
        Err(EditError::OutOfRange { what })
    }
}

impl Default for WorldLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl Shape {
    /// `Ok` when the shape lies within `limits`; otherwise the first field out of range, in
    /// the order: positions, radius, capsule length, then the edit extent (its voxel bounds,
    /// computed only once the rest is in range, on a reversed or empty shape passes).
    pub fn check(&self, limits: &WorldLimits) -> Result<(), EditError> {
        match *self {
            Shape::Sphere { c, r } => {
                limits.coord(c)?;
                limits.radius(r)?;
            }
            Shape::Box_ { min, max } => {
                limits.coord(min)?;
                limits.coord(max)?;
            }
            Shape::Capsule { a, b, r } => {
                limits.coord(a)?;
                limits.coord(b)?;
                limits.radius(r)?;
                let cap = limits.capsule_axis_max.unsigned_abs();
                refuse_unless(
                    (0..3).all(|k| a.0[k].abs_diff(b.0[k]) <= cap),
                    Field::CapsuleLength,
                )?;
            }
        }
        let Some((lo, hi)) = self.voxel_bounds() else {
            return Ok(());
        };
        refuse_unless(
            (0..3).all(|k| hi[k].abs_diff(lo[k]) < u64::from(limits.edit_axis_max)),
            Field::EditExtent,
        )
    }
}
