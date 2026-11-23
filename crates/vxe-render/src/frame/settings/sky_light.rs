//! The sky's light at a hit: the key light's penumbra, where its far visibility comes from and
//! how that joins the ray, and the normal the baked sky term reads by.

/// How the key light's shadow ray makes its penumbra (Plan 0086 Phase 7's spike).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Penumbra {
    /// One ray toward a uniform point of the body's disc a pixel and frame: the penumbra is the
    /// mean over frames and pixels. The default.
    #[default]
    Disc,
    /// One ray toward the body's centre that keeps, along its walk, the smallest ratio of the
    /// distance from the ray to a solid 2^3 block beside it (probed two voxels across the ray's
    /// dominant axis, about every two voxels through cells of a brick's edge or less) to
    /// `t tan r`, `r` the body's angular radius: the visibility is that ratio clamped to 1, 0
    /// where the ray hits. The same every frame; a penumbra on the lit side of the edge only.
    Approach,
}

named_enum!(Penumbra {
    Disc => "disc",
    Approach => "approach",
});

/// Where the key light's visibility comes from where a voxel is small on screen (ADR-0079).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyFar {
    /// The far estimate at a cube face: the store's baked key visibility where its cell holds
    /// a fresh one (`KEY_BAKED`, not `SKY_STALE`), read bilinear, else the column map's horizon
    /// toward the sun softened by the disc's radius. Where the far normal's weight is 1 (at and
    /// over one half under [`KeyFarMix::Step`]) the estimate with no key ray and no screen
    /// history, a switch of its source (the record's landing, a re-bake) riding one fade;
    /// between the band's ends the ray's running mean and the estimate by [`KeyFarMix`]; and a
    /// reset key mean seeded from the estimate. Anywhere else the ray.
    Baked,
    /// As [`KeyFar::Baked`] with the column map's horizon at every far face, record or not.
    /// The default (ADR-0082).
    #[default]
    Column,
    /// The per-pixel disc ray and its running mean at every distance (ADR-0027).
    Ray,
}

named_enum!(KeyFar {
    Baked => "baked",
    Column => "column",
    Ray => "ray",
});

/// How [`KeyFar::Baked`] joins the ray's mean and the store's value inside the far normal's
/// band.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyFarMix {
    /// `mix(mean, store, w)`, `w` the far normal's weight.
    Blend,
    /// The store's value where `w` is at least one half, else the mean. The default
    /// (ADR-0082).
    #[default]
    Step,
}

named_enum!(KeyFarMix {
    Blend => "blend",
    Step => "step",
});

/// The normal a cube face's baked sky term (ADR-0070) reads its visibilities by (Plan 0086
/// Phase 13's arms).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SkyNormal {
    /// The shading normal: the face's own direction near, the far normal's blend far.
    Face,
    /// The 27-count gradient's normal `g` at every distance: the face's term is the mix over
    /// the directions `g` leans toward, `2a + (g_a > 0)` weighted `g_a^2`, of each direction's
    /// baked visibility times the sky's irradiance above the horizon about it, over the
    /// directions whose visibility is held; a tread and a riser of a one-voxel stair read the
    /// slope's blend. The surface and the key light keep the face normal; a face whose sky
    /// rays were cast (no fresh bake) keeps the shading normal. The default.
    #[default]
    Smoothed,
}

named_enum!(SkyNormal {
    Face => "face",
    Smoothed => "smoothed",
});
