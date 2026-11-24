//! Highlight protection: the eye is capped so the view's bright tail stays under a white point.
//!
//! The composite writes, per meter tile, the largest `clamp(log2(max(max(r, g, b), 2^-16)),
//! -16, 16)` of its hit pixels' radiance before exposure into the tile's last word (the
//! brightest channel, as the tone curve maps each channel alone; sky pixels count as -16, so
//! the sky and the sun's disc, which the sky anchor exposes for, never set the cap). The exposure pass bins the tiles' maxima,
//! each weighted by its pixel count, into [`TAIL_BINS`] bins of a quarter EV over `[-16, 16)`, and reads the level `L` over which the share `tail` of the view's pixels lies
//! (the high percentile `P`, `tail = 1 - P / 100`), linear within its bin. That level is
//! smoothed by the eye's own step (`AutoExposure::step`: its time constants and its rate cap;
//! a reset or the first metered frame jumps), and the exposure in force is clamped under
//! `white / 2^L` besides the ceiling: `white` the exposed value the tone curve maps to
//! [`HIGHLIGHT_WHITE`] ([`white_exposed`]). A view with no bright tail gives a cap over the
//! ceiling, and the cap does nothing.

use crate::ToneMap;

/// The tone-mapped value (before the display encode) the bright tail is held at or under.
pub const HIGHLIGHT_WHITE: f32 = 0.95;

/// Bins of the tail's histogram over the meter's log2 range, `[-METER_LOG2_LIMIT,
/// METER_LOG2_LIMIT)` (the prelude's `TAIL_BINS`).
pub(crate) const TAIL_BINS: u32 = 128;

/// The exposed value `x` the tone curve `tone` maps to [`HIGHLIGHT_WHITE`]: the ACES fit's
/// inverse (the root of its quadratic, as `undisplay` in `composite.wgsl` takes it), or the
/// white point itself under the identity.
pub fn white_exposed(tone: ToneMap) -> f32 {
    let m = HIGHLIGHT_WHITE;
    match tone {
        ToneMap::Identity => m,
        ToneMap::Aces => {
            let a = 2.51 - 2.43 * m;
            let b = 0.03 - 0.59 * m;
            (-b + (b * b + 4.0 * a * 0.14 * m).sqrt()) / (2.0 * a)
        }
    }
}

/// `ExHighlight` in `exposure.wgsl`, field for field: 16 bytes, in a buffer of its own beside
/// the state (`Frame::exposure_highlight`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ExposureHighlight {
    /// The last metered frame's tail level, log2 of radiance before exposure.
    pub now_log2: f32,
    /// The tail level smoothed by the eye's step, log2.
    pub log2: f32,
    /// The cap the last pass clamped the exposure under, `white / 2^log2`; 0 with the cap off.
    pub cap: f32,
    /// 1 once a metered frame set `log2`; 0 with the cap off or before.
    pub seen: u32,
}

impl ExposureHighlight {
    /// The highlight from its 16 bytes.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        bytemuck::pod_read_unaligned(&bytes[..size_of::<Self>()])
    }

    /// The cap in force, `None` with the cap off or before its first metered frame.
    pub fn cap(&self) -> Option<f32> {
        (self.seen != 0 && self.cap > 0.0).then_some(self.cap)
    }
}

impl super::ExposureUniform {
    /// The highlight fields of `auto` under the tone curve `tone`: the tail's share `1 - P /
    /// 100` and the exposed white, both 0 with [`super::AutoExposure::highlight_percentile`]
    /// 0 (off).
    pub(crate) fn set_highlight(&mut self, auto: &super::AutoExposure, tone: ToneMap) {
        let p = auto.highlight_percentile;
        if p > 0.0 && p <= 100.0 {
            self.hi_tail = 1.0 - p / 100.0;
            self.hi_white = white_exposed(tone);
        } else {
            self.hi_tail = 0.0;
            self.hi_white = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exposure::{AutoExposure, ExposureUniform};

    /// The ACES white maps back to the white point through the fit; the identity's is the
    /// white point; the highlight's fields are the tail's share and the white, 0 when off.
    #[test]
    fn the_white_and_the_tail_reach_the_uniform() {
        let x = white_exposed(ToneMap::Aces);
        let aces = (x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14);
        assert!((aces - HIGHLIGHT_WHITE).abs() < 1e-5, "{x} maps to {aces}");
        assert!((x - 2.863).abs() < 1e-3, "{x}");
        assert_eq!(white_exposed(ToneMap::Identity), HIGHLIGHT_WHITE);
        let mut u = ExposureUniform::default();
        let on = AutoExposure {
            highlight_percentile: 98.0,
            ..AutoExposure::eye()
        };
        u.set_highlight(&on, ToneMap::Aces);
        assert!((u.hi_tail - 0.02).abs() < 1e-6);
        assert_eq!(u.hi_white, x);
        u.set_highlight(
            &AutoExposure {
                highlight_percentile: 0.0,
                ..on
            },
            ToneMap::Aces,
        );
        assert_eq!((u.hi_tail, u.hi_white), (0.0, 0.0));
        let h = ExposureHighlight {
            cap: 2.0,
            ..Default::default()
        };
        assert_eq!(h.cap(), None);
        assert_eq!(ExposureHighlight { seen: 1, ..h }.cap(), Some(2.0));
    }
}
