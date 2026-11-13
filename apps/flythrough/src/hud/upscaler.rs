//! The upscaler's banner: [`UpscalerMode`], [`UpscalerBanner`], FSR 2's knob words, the pan
//! and the aliasing row.

use super::*;
use std::fmt::Write as _;

/// The upscaler the window's `F10` cycles (`--upscaler`): Plan 0024's frame, the upscaler of
/// ADR-0024, or FSR 2 (ADR-0034).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpscalerMode {
    Off,
    Own,
    Fsr2,
}

impl UpscalerMode {
    pub const ALL: [Self; 3] = [Self::Off, Self::Own, Self::Fsr2];

    /// The name `--upscaler` parses and the banner shows.
    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Own => "own",
            Self::Fsr2 => "fsr2",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == name)
    }

    /// The mode `F10` switches to: off, own, fsr2, then off again.
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Own,
            Self::Own => Self::Fsr2,
            Self::Fsr2 => Self::Off,
        }
    }
}

/// The upscaler's banner: one line naming the mode and its internal scale, shown for
/// [`BANNER_MS`] after each change, by the frame loop's clock, beside the stats HUD or alone.
#[derive(Clone, Debug, PartialEq)]
pub struct UpscalerBanner {
    pub mode: UpscalerMode,
    /// The internal scale's name (`RenderScale::name`).
    pub scale: &'static str,
    /// The upscaler's RCAS sharpening (`--sharpen`, `Z`); unnamed while the upscaler is off.
    pub sharpen: f32,
    /// FSR 2's candidate knobs as words (`, phases 8, dither`), empty when none
    /// is set; named only while the mode is fsr2.
    pub fsr2_knobs: String,
    shown_at: Option<f64>,
}

/// FSR 2's candidate knobs (`--jitter-phases`, `--dither`) as the banner's suffix:
/// `, phases 8, dither`, each only when set; empty when none is.
pub fn fsr2_knob_words(phases: Option<u32>, dither: bool) -> String {
    let mut s = String::new();
    if let Some(n) = phases {
        s += &format!(", phases {n}");
    }
    if dither {
        s += ", dither";
    }
    s
}

/// The sharpening `Z` steps through: 0.5, 0.25, 0, then 0.5 again; any other value steps to
/// 0.5.
pub fn next_sharpen(s: f32) -> f32 {
    if s == 0.5 {
        0.25
    } else if s == 0.25 {
        0.0
    } else {
        0.5
    }
}

impl UpscalerBanner {
    /// The banner of the starting mode, hidden until the first change, at `--sharpen`'s
    /// default.
    pub fn new(mode: UpscalerMode, scale: &'static str) -> Self {
        Self {
            mode,
            scale,
            sharpen: 0.5,
            fsr2_knobs: String::new(),
            shown_at: None,
        }
    }

    /// The same banner at sharpening `sharpen`.
    pub fn with_sharpen(self, sharpen: f32) -> Self {
        Self { sharpen, ..self }
    }

    /// The same banner naming FSR 2's candidate knobs, `knobs` as [`fsr2_knob_words`] writes
    /// them.
    pub fn with_fsr2_knobs(self, knobs: String) -> Self {
        Self {
            fsr2_knobs: knobs,
            ..self
        }
    }

    /// The sharpening is now `sharpen` (`Z`); the banner shows from `now_ms`. Under FSR 2 the
    /// change is a settings change that keeps its history (`Frame::set_upscaler_kind`), and
    /// under our upscaler a uniform write, so the image shows no history drop at the key.
    pub fn set_sharpen(&mut self, sharpen: f32, now_ms: f64) {
        self.sharpen = sharpen;
        self.shown_at = Some(now_ms);
    }

    /// The banner of the starting mode, shown from `now_ms` as the lighting modes' banner is
    /// at the start.
    pub fn shown_from(mode: UpscalerMode, scale: &'static str, now_ms: f64) -> Self {
        Self {
            shown_at: Some(now_ms),
            ..Self::new(mode, scale)
        }
    }

    /// The mode is now `mode` at `scale`; the banner shows from `now_ms`.
    pub fn change(&mut self, mode: UpscalerMode, scale: &'static str, now_ms: f64) {
        self.mode = mode;
        self.scale = scale;
        self.shown_at = Some(now_ms);
    }

    /// `upscaler fsr2, scale 1.5, sharpen 0.5`; `upscaler off, scale 1` with no upscaler.
    pub fn line(&self) -> String {
        let mut out = String::new();
        self.write_line(&mut out);
        out
    }

    /// Appends [`UpscalerBanner::line`] to `out`.
    pub fn write_line(&self, out: &mut String) {
        if self.mode == UpscalerMode::Off {
            let _ = write!(out, "upscaler off, scale {}", self.scale);
        } else {
            let _ = write!(
                out,
                "upscaler {}, scale {}, sharpen {}{}",
                self.mode.name(),
                self.scale,
                self.sharpen,
                if self.mode == UpscalerMode::Fsr2 {
                    self.fsr2_knobs.as_str()
                } else {
                    ""
                }
            );
        }
    }

    /// The line within [`BANNER_MS`] of the last change, else `None`.
    pub fn banner(&self, now_ms: f64) -> Option<String> {
        let mut out = String::new();
        self.write_banner(now_ms, &mut out).then_some(out)
    }

    /// Appends the line to `out` as a banner row ([`start_row`]) within [`BANNER_MS`] of the
    /// last change; `false`, and nothing written, otherwise.
    pub fn write_banner(&self, now_ms: f64, out: &mut String) -> bool {
        let shows = self.shown_at.is_some_and(|at| now_ms - at < BANNER_MS);
        if shows {
            start_row(out);
            self.write_line(out);
        }
        shows
    }
}

/// `--pan DEG_PER_S`'s constant yaw, which `Period` stops and starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pan {
    /// Degrees a second, positive to the left.
    pub deg_per_s: f32,
    pub on: bool,
    shown_at: Option<f64>,
}

impl Pan {
    /// The pan at `deg_per_s`, running, its banner shown from `now_ms`.
    pub fn new(deg_per_s: f32, now_ms: f64) -> Self {
        Self {
            deg_per_s,
            on: true,
            shown_at: Some(now_ms),
        }
    }

    /// `Period`: stops or starts the pan; the banner shows from `now_ms`.
    pub fn toggle(&mut self, now_ms: f64) {
        self.on = !self.on;
        self.shown_at = Some(now_ms);
    }

    /// The yaw in radians the pan adds over `dt` seconds.
    pub fn yaw(&self, dt: f32) -> f32 {
        if self.on {
            self.deg_per_s.to_radians() * dt
        } else {
            0.0
        }
    }

    /// `pan 1.1 deg/s`, or `pan 1.1 deg/s stopped`.
    pub fn line(&self) -> String {
        let mut out = String::new();
        self.write_line(&mut out);
        out
    }

    /// Appends [`Pan::line`] to `out`.
    pub fn write_line(&self, out: &mut String) {
        let _ = write!(
            out,
            "pan {} deg/s{}",
            self.deg_per_s,
            if self.on { "" } else { " stopped" }
        );
    }

    /// The line within [`BANNER_MS`] of the last change, else `None`.
    pub fn banner(&self, now_ms: f64) -> Option<String> {
        let mut out = String::new();
        self.write_banner(now_ms, &mut out).then_some(out)
    }

    /// Appends the line to `out` as a banner row ([`start_row`]) within [`BANNER_MS`] of the
    /// last change; `false`, and nothing written, otherwise.
    pub fn write_banner(&self, now_ms: f64, out: &mut String) -> bool {
        let shows = self.shown_at.is_some_and(|at| now_ms - at < BANNER_MS);
        if shows {
            start_row(out);
            self.write_line(out);
        }
        shows
    }
}

/// The stats HUD's aliasing row: the upscaler, its scale and sharpening, the far normal, the
/// store read, the filters' weights and the light's resolution ([`Modes::write_filter_words`]),
/// and the pan, shown the whole time.
pub fn aliasing_row(
    upscaler: &UpscalerBanner,
    far_normal: &str,
    modes: &Modes,
    pan: Option<&Pan>,
) -> String {
    let mut out = String::new();
    write_aliasing_row(&mut out, upscaler, far_normal, modes, pan);
    out
}

/// Appends [`aliasing_row`] to `out`.
pub fn write_aliasing_row(
    out: &mut String,
    upscaler: &UpscalerBanner,
    far_normal: &str,
    modes: &Modes,
    pan: Option<&Pan>,
) {
    upscaler.write_line(out);
    out.push_str("; ");
    out.push_str(far_normal);
    out.push_str("; ");
    modes.write_filter_words(out);
    out.push_str("; ");
    match pan {
        Some(p) => p.write_line(out),
        None => out.push_str("no pan"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `F10` steps the banner's text through `off`, `own` and `fsr2`, each line fitting the
    /// banner's width; it shows after a change, or from the start when built with
    /// [`UpscalerBanner::shown_from`], for [`BANNER_MS`].
    #[test]
    fn the_upscaler_banner_names_each_mode_it_cycles_through() {
        let mut b = UpscalerBanner::new(UpscalerMode::Off, "1");
        assert_eq!(b.banner(0.0), None, "no banner before a change");
        let mut mode = UpscalerMode::Off;
        let mut seen = Vec::new();
        for (i, scale) in ["1.5", "1.5", "1"].into_iter().enumerate() {
            mode = mode.next();
            let t = 1000.0 * i as f64;
            b.change(
                mode,
                if mode == UpscalerMode::Off {
                    "1"
                } else {
                    scale
                },
                t,
            );
            let text = b.banner(t).expect("a change shows the banner");
            assert!(text.len() <= BANNER_COLS, "{text}");
            seen.push(text);
        }
        assert_eq!(
            seen,
            [
                "upscaler own, scale 1.5, sharpen 0.5",
                "upscaler fsr2, scale 1.5, sharpen 0.5",
                "upscaler off, scale 1"
            ]
        );
        assert!(b.banner(2000.0 + BANNER_MS - 1.0).is_some());
        assert_eq!(b.banner(2000.0 + BANNER_MS), None);
        let start = UpscalerBanner::shown_from(UpscalerMode::Fsr2, "1.5", 0.0);
        assert_eq!(
            start.banner(BANNER_MS - 1.0).as_deref(),
            Some("upscaler fsr2, scale 1.5, sharpen 0.5"),
            "the starting mode shows at the start"
        );
        assert_eq!(start.banner(BANNER_MS), None);
        for m in UpscalerMode::ALL {
            assert_eq!(UpscalerMode::from_name(m.name()), Some(m));
        }
        assert_eq!(UpscalerMode::from_name("fsr"), None);
    }
}
