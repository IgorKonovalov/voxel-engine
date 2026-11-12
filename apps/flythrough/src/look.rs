//! The look presets the fly-through starts in and `N` cycles, and the light store's default
//! read and weights.

use vxe_render::{NormalMode, SurfaceTuning};

/// A named look of the window: a normal mode and the surface tuning. `--look` starts in one, `N` cycles them in [`LookPreset::ALL`] order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LookPreset {
    pub name: &'static str,
    pub normal_mode: NormalMode,
    pub tuning: SurfaceTuning,
    /// `LookParams::far_normal`, `[0.0, 0.0]` off.
    pub far_normal: [f32; 2],
}

impl LookPreset {
    /// A preset of `mode` with no face mix.
    const fn of(name: &'static str, normal_mode: NormalMode) -> Self {
        Self {
            name,
            normal_mode,
            tuning: SurfaceTuning { face_mix: 0.0 },
            far_normal: [0.0; 2],
        }
    }

    pub const FACE: Self = Self::of("face", NormalMode::Face);
    /// The field surface (ADR-0041, `NormalMode::Contour`), an opt-in look; a material marked
    /// `MaterialFlags::CRISP` in the table renders as cubes in it. It needs a mirror with the
    /// field's band slots.
    pub const FIELD: Self = Self::of("field", NormalMode::Contour);
    pub const TRILINEAR8: Self = Self::of("trilinear8", NormalMode::Trilinear8);
    pub const GRADIENT27: Self = Self::of("gradient27", NormalMode::Gradient27);

    /// `N`'s order: face, the field, then the two diagnostic normal modes.
    pub const ALL: [Self; 4] = [Self::FACE, Self::FIELD, Self::TRILINEAR8, Self::GRADIENT27];

    /// The look the fly-through starts in without `--look`: honest cubes (ADR-0041) with the
    /// far normal at `[2, 6]`, the owner's pick (Plan 0061 Phase 4), read with
    /// [`DEFAULT_STORE_READ`] and [`DEFAULT_LIGHT_WEIGHTS`]. `--look face` is the same cubes
    /// with the far normal off.
    pub const DEFAULT: Self = Self {
        far_normal: [2.0, 6.0],
        ..Self::FACE
    };

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name == name)
    }

    /// The names `--look` takes, comma-separated.
    pub fn names() -> String {
        Self::ALL.map(|p| p.name).join(", ")
    }

    /// The next preset in [`LookPreset::ALL`] order, wrapping, skipping the field unless
    /// `contour` is set (a mirror with the field's band slots). A preset not in the list
    /// (one with a part overridden) continues from the first of its normal mode.
    pub fn next(self, contour: bool) -> Self {
        let i = Self::ALL
            .iter()
            .position(|p| *p == self)
            .or_else(|| {
                Self::ALL
                    .iter()
                    .position(|p| p.normal_mode == self.normal_mode)
            })
            .unwrap_or(0);
        (1..=Self::ALL.len())
            .map(|k| Self::ALL[(i + k) % Self::ALL.len()])
            .find(|p| contour || p.normal_mode != NormalMode::Contour)
            .unwrap_or(Self::FACE)
    }

    /// The parts beyond the normal mode: `face mix 0`.
    pub fn parts(&self) -> String {
        format!("face mix {}", self.tuning.face_mix)
    }

    /// The far normal in words: `far normal 2..6` in face mode, the one mode that reads it, and
    /// `far normal off` in the others or with the knob off.
    pub fn far_normal_words(normal_mode: NormalMode, far_normal: [f32; 2]) -> String {
        let [lo, hi] = far_normal;
        if normal_mode == NormalMode::Face && hi > lo {
            format!("far normal {lo}..{hi}")
        } else {
            "far normal off".to_owned()
        }
    }
}

/// How the fly-through's cube faces read the baked store without `--store-read`: by the
/// shading normal (ADR-0049), so where the far normal leans a face's normal the stored light
/// is the mix of the cell's direction values it leans toward; near faces read their own
/// direction, as `StoreRead::Face`, the library's default, does everywhere.
pub const DEFAULT_STORE_READ: vxe_render::StoreRead = vxe_render::StoreRead::Normal;

/// The normal the fly-through's half-resolution light filters compare without
/// `--light-weights`: the hit's face normal (ADR-0049's weight rule), so a far riser's and
/// tread's taps stay apart under the far normal. The library's default is `Normal`.
pub const DEFAULT_LIGHT_WEIGHTS: vxe_render::LightWeights = vxe_render::LightWeights::Face;
