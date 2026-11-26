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

/// What a dig shows until its re-bake lands with no light history (Plan 0098 Phases 8, 14 and
/// 17, ADR-0082): every edit box is held from its frame until the live queue holds no dig and a
/// fade's frames more.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DigHold {
    /// The store holds the value: at the edit each touched brick takes a held record made on the
    /// CPU against the world (the removed surface's sky visibility times a near openness, at
    /// least a floor; `bake::held`), which the light pass reads as any record (no ray cast, no
    /// pair kept) and the store fades to the job's record at its landing (`DIG_FADE`). A stale
    /// ring face holds its record's term, as under `Rays`. The default.
    #[default]
    On,
    /// A stale ring face holds its record's term; a face in a held box shows its sky rays'
    /// running mean, seeded by its stale record or the column's answer.
    Rays,
    /// Neither: the cast rays' seed until the landing.
    Off,
}

named_enum!(DigHold {
    On => "on",
    Rays => "rays",
    Off => "off",
});

impl DigHold {
    /// The value after this one in [`DigHold::ALL`], the first after the last.
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|&d| d == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

/// The estimate a dig's held record takes under [`DigHold::On`] (`bake::held`, ADR-0082): a
/// dug face's sky visibility is its base (the nearest record at or above the cell, else the
/// chunk summary's, capped by the cell's own) times the square root of its near openness.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DigEstimate {
    /// The value at least a floor above black (`HELD_FLOOR`), a cell with no face its base at
    /// least the floor. The default.
    #[default]
    Surface,
    /// No floor on any path: a face holds its base times the root of its openness, 0
    /// included, and a cell with no face its base.
    Depth,
}

named_enum!(DigEstimate {
    Surface => "surface",
    Depth => "depth",
});

/// Where on a cube face the key light's column horizon (`column_key_visibility`) is read
/// (Plan 0100 Phase 1's spike, ADR-0084).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyAt {
    /// At the hit: a point that moves with the frame's jitter across a face. The default.
    #[default]
    Hit,
    /// At the centre of the hit's voxel face (the voxel's centre moved half a voxel along the
    /// face normal): one value a face, fixed in the world.
    Face,
    /// At the four face centres of the face's plane nearest the hit, blended bilinearly by the
    /// hit's place among them: a ramp one voxel wide, fixed in the world.
    FaceSmooth,
}

named_enum!(KeyAt {
    Hit => "hit",
    Face => "face",
    FaceSmooth => "face-smooth",
});

impl KeyAt {
    /// The value after this one in [`KeyAt::ALL`], the first after the last.
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|&k| k == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

/// How the key light's column horizon reads the map (Plan 0100 Phase 5b, ADR-0084).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyHorizon {
    /// ADR-0079's march: the map cell at each of 12 distances to 512 voxels, then every cell
    /// (at most 8) of the one interval whose far cell stands highest. The default.
    #[default]
    March,
    /// Every map cell along the sun's horizontal direction to the window's side, over the
    /// coarse levels (the largest top of each 32 x 32 and 8 x 8 cells): a level's block is
    /// passed whole where its top cannot raise the horizon into the disc, and the walk ends
    /// where the visibility is 0 or no top of the window can reach the disc.
    Walk,
}

named_enum!(KeyHorizon {
    March => "march",
    Walk => "walk",
});

impl KeyHorizon {
    /// The value after this one in [`KeyHorizon::ALL`], the first after the last.
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|&k| k == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

/// The source that adds the key light back on a cube face where the column horizon reads under
/// 1 (Plan 0100 Phase 6, ADR-0084): the shown visibility is the larger of the horizon and the
/// source, so a source only adds. Entities, field surfaces, `light history on` and `key far ray`
/// are as without it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyCover {
    /// The horizon alone (ADR-0082's picture). The default.
    #[default]
    Off,
    /// Near (the far normal's weight under one half), one ray from the face's centre toward the
    /// key light's disc centre, times its transmittance, cast every frame with no random number
    /// and shown with no mean; under [`crate::Penumbra::Approach`] its closest-approach score.
    /// Its length is `key_cap`'s. Past the half, the horizon alone.
    Ray,
    /// At every distance, the store's baked key visibility where the face's own cell holds a
    /// fresh one, else the horizon alone; a landing rides the key slot's fade.
    Record,
}

named_enum!(KeyCover {
    Off => "off",
    Ray => "ray",
    Record => "record",
});

impl KeyCover {
    /// The value after this one in [`KeyCover::ALL`], the first after the last.
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|&k| k == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}
