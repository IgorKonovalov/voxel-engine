//! The spot lamp the marcher shades with: placed relative to the camera, cone falloff,
//! inverse-square attenuation, and optionally one shadow ray per pixel.

/// Voxels per metre at the first consumer's 10 cm scale; the lamp offsets are given in metres.
const VOXELS_PER_METRE: f32 = 10.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightParams {
    /// Lamp offset from the eye along the camera's right and down vectors, in voxels.
    pub offset_right: f32,
    pub offset_down: f32,
    /// Radiance on the cone axis at one voxel of distance.
    pub intensity: f32,
    /// Cone half-angles in radians: full intensity inside `inner`, none outside `outer`.
    pub inner: f32,
    pub outer: f32,
    /// Trace one shadow ray from each lit hit toward the lamp. Off only for tests and the
    /// fly-through's debug toggle.
    pub shadows: bool,
    /// Skip the empty 4 × 4 × 4 and 2 × 2 × 2 blocks and the empty voxels of a full brick
    /// through its opaque mask. Off only for tests and debugging: the walk then reads every
    /// voxel of a full brick through the pool.
    pub masks: bool,
    /// Skip every empty cell larger than a voxel: absent and empty chunks, clear brick groups,
    /// uniform-air bricks and the masked blocks. Off only for tests: the walk then steps one
    /// voxel (one mip cell of a LOD-only chunk) at a time, the comparison the hierarchy is
    /// held byte-identical to.
    pub skips: bool,
    /// The lamp's cone turned from the camera's forward toward its right, in radians.
    pub turn: f32,
}

impl LightParams {
    /// A lamp at the eye with no shadow ray: from the eye, every lit point is visible from the
    /// lamp, so a shadow ray would change nothing.
    pub fn at_eye() -> Self {
        Self {
            offset_right: 0.0,
            offset_down: 0.0,
            intensity: 1200.0,
            inner: 0.35,
            outer: 0.70,
            shadows: false,
            masks: true,
            skips: true,
            turn: 0.0,
        }
    }

    /// A headlamp 0.25 m right of and 0.15 m below the eye, casting shadows.
    pub fn headlamp() -> Self {
        Self {
            offset_right: 0.25 * VOXELS_PER_METRE,
            offset_down: 0.15 * VOXELS_PER_METRE,
            shadows: true,
            ..Self::at_eye()
        }
    }
}

impl Default for LightParams {
    fn default() -> Self {
        Self::headlamp()
    }
}
