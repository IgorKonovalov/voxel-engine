//! FSR 2's settings and tuning, the upscaler a frame runs, and the jitter sequence.

use super::*;

/// FSR 2's knobs in the engine ([`UpscalerKind::Fsr2`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fsr2Settings {
    /// RCAS's sharpness, 0 to 1 (FSR's `sharpness`: `2 - 2 sharpness` stops); 0 skips the
    /// pass (FSR's `enableSharpening` off).
    pub sharpness: f32,
    /// The reactive mask's weight on a translucent layer's `1 - transmittance`.
    pub reactive_translucent: f32,
    /// The reactive mask's value where the light at a hit changed.
    pub reactive_light: f32,
    /// FSR's `viewSpaceToMetersFactor`: the world's voxel edge in metres.
    pub metres_per_voxel: f32,
    /// The jitter sequence's phase count; `None` is FSR's own, [`jitter_phases`].
    pub jitter_phases: Option<u32>,
    /// The tone pass adds a static blue-noise dither of one 8-bit level to the shaded view: the
    /// frame's blue-noise texel at the pixel (tiled), less a half, over 255. It reads no frame
    /// index, so it holds still on a still image.
    pub dither: bool,
}

impl Fsr2Settings {
    /// The phase count for a `render_width` image shown `display_width` wide: the setting's,
    /// at least 1, else [`jitter_phases`].
    pub fn phases(&self, render_width: u32, display_width: u32) -> u32 {
        self.jitter_phases
            .map_or_else(|| jitter_phases(render_width, display_width), |n| n.max(1))
    }
}

impl Default for Fsr2Settings {
    fn default() -> Self {
        Self {
            sharpness: 0.0,
            reactive_translucent: 1.0,
            reactive_light: 1.0,
            metres_per_voxel: 0.1,
            jitter_phases: None,
            dither: false,
        }
    }
}

/// Which upscaler a [`crate::Frame`] runs ([`crate::Frame::set_upscaler_kind`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UpscalerKind {
    /// No upscaler: the jitter off, and at equal sizes the composite's image is the frame's.
    Off,
    /// The upscaler of ADR-0024 with this tuning.
    Own(Upscaler),
    /// FSR 2 with these settings.
    Fsr2(Fsr2Settings),
}

impl UpscalerKind {
    /// The name the fly-through prints and parses.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Own(_) => "own",
            Self::Fsr2(_) => "fsr2",
        }
    }

    pub fn is_fsr2(&self) -> bool {
        matches!(self, Self::Fsr2(_))
    }
}

/// FSR's jitter phase count for a `render_width` image shown `display_width` wide
/// (`ffxFsr2GetJitterPhaseCount`): `int(8 (display / render)^2)`.
pub fn jitter_phases(render_width: u32, display_width: u32) -> u32 {
    let ratio = display_width as f32 / render_width as f32;
    (8.0 * ratio.powi(2)) as u32
}

/// FSR's jitter offset of frame `index` in a sequence of `phases`, in render pixels
/// (`ffxFsr2GetJitterOffset`): Halton (2, 3) at `index % phases + 1`, minus 0.5. FSR's sample
/// of render pixel `p` lies at `p + 0.5 - offset` in the same y-down pixel space our rays use,
/// so a view's [`crate::View::jitter`] is the offset negated on both axes.
pub fn jitter_offset(index: u32, phases: u32) -> [f32; 2] {
    crate::frame::jitter_at(index, phases)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FSR's phase counts: 8 at scale 1, 18 at 1.5 and 32 at 2; and its first four offsets.
    #[test]
    fn the_jitter_sequence_is_fsrs() {
        assert_eq!(jitter_phases(384, 384), 8);
        assert_eq!(jitter_phases(256, 384), 18);
        assert_eq!(jitter_phases(192, 384), 32);
        assert_eq!(jitter_phases(1280, 1920), 18);
        let want = [
            [0.0, -1.0 / 6.0],
            [-0.25, 1.0 / 6.0],
            [0.25, -7.0 / 18.0],
            [-0.375, -1.0 / 18.0],
        ];
        for (i, w) in want.iter().enumerate() {
            let j = jitter_offset(i as u32, 18);
            for k in 0..2 {
                assert!(
                    (j[k] - w[k]).abs() < 1e-6,
                    "offset {i}: {j:?} against {w:?}"
                );
            }
        }
        assert_eq!(jitter_offset(18, 18), jitter_offset(0, 18));
    }
}
