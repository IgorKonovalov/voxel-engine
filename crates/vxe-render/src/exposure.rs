//! The frame's exposure (ADR-0033): the composite's meter, the exposure pass that turns it into
//! the state the next frame reads, and a ring of read-back buffers that hands that state to the
//! CPU a few frames late without waiting.
//!
//! The meter: in the shaded view the composite adds, per 8 x 8 workgroup of the internal
//! target, `clamp(log2(max(Y, 2^-16)), -16, 16)` of each pixel's radiance before exposure and
//! before the tone map (Rec. 709 luminance `Y`), a count of 1 and a sky count of 1 for a pixel
//! that is sky (a miss), into one tile each (four `f32`, the last its hits' brightest channel
//! in log2, for highlight protection: `highlight.rs`). The exposure pass
//! (`exposure.wgsl`, one workgroup of 256, run last in the frame) weights tile `i` by
//! [`Metering::weight`] and writes `sum(w S) / sum(w n)` into the state's `metered_log2` and
//! `sum(w k) / sum(w n)`, the share of the view that is sky, into its `sky`.
//!
//! The exposure is [`Exposure::Manual`] or [`Exposure::Auto`]. In auto the pass steps an adapted
//! level `a` (log2 luminance) toward the meter `t` by [`AutoExposure::step`] over the caller's
//! `dt` (`Frame::set_dt`), and the next frame's exposure is [`AutoExposure::exposure`] of it.
//!
//! **Pre-exposure.** Lighting math is in absolute units. Every value a pass stores into an
//! `rgba16float` HDR target is multiplied by the frame's exposure (`exposure_now` in
//! `common.wgsl`) and clamped at 60 000; a pass that reads a value an earlier frame stored
//! multiplies it by `current / previous` (`exposure_ratio`) first, a second moment by its
//! square, and a second moment is of the luminance clamped at 255. Terms that stay absolute
//! (emission, translucent layers, the probe atlases and their gathered irradiance, reservoirs)
//! are multiplied by the exposure where they join a pre-exposed value. A new pass that keeps HDR
//! history follows the same rule, or its history pulses on every exposure change.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::gpu_util::{Io, compute_pipeline, io_layout};

mod around;
mod highlight;
pub use around::SkyAround;
pub(crate) use highlight::TAIL_BINS;
pub use highlight::{ExposureHighlight, HIGHLIGHT_WHITE, white_exposed};

/// The exposure pass, compiled after the prelude ([`exposure_source`]).
pub const EXPOSURE_WGSL: &str = include_str!("shaders/exposure.wgsl");

/// The exposure pass's composed source: the prelude, then [`EXPOSURE_WGSL`].
pub(crate) fn exposure_source() -> String {
    crate::frame::compose(&[EXPOSURE_WGSL])
}

/// Pixels on a meter tile's edge: the composite's workgroup.
pub(crate) const METER_TILE: u32 = 8;
/// Bytes of one meter tile: the log2 sum, the count, the sky count and the brightest
/// channel's log2, as `f32`.
pub(crate) const METER_TILE_BYTES: u64 = 16;
/// The log2 luminance a pixel adds to its tile is clamped to `[-METER_LOG2_LIMIT,
/// METER_LOG2_LIMIT]`.
pub const METER_LOG2_LIMIT: f32 = 16.0;

/// How the exposure pass weights the meter's tiles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Metering {
    /// Tile `i` weighs `1 + strength exp(-r_i^2 / (2 sigma^2))`, `r_i` its centre's distance
    /// from its view's centre in units of half the view's height.
    CentreWeighted { strength: f32, sigma: f32 },
    /// Every tile weighs 1 (`strength` 0).
    Average,
}

impl Default for Metering {
    /// Centre-weighted, `strength` 3 and `sigma` 0.5.
    fn default() -> Self {
        Self::CentreWeighted {
            strength: 3.0,
            sigma: 0.5,
        }
    }
}

impl Metering {
    /// `strength` and `sigma`; `Average` is strength 0.
    pub fn params(self) -> (f32, f32) {
        match self {
            Self::CentreWeighted { strength, sigma } => (strength, sigma),
            Self::Average => (0.0, 1.0),
        }
    }

    /// The weight of a tile whose centre lies `r` half-heights from its view's centre.
    pub fn weight(self, r: f32) -> f32 {
        let (strength, sigma) = self.params();
        let s = sigma.max(1e-6);
        1.0 + strength * (-r * r / (2.0 * s * s)).exp()
    }

    /// The lower-case name the fly-through prints and parses.
    pub fn name(self) -> &'static str {
        match self {
            Self::CentreWeighted { .. } => "centre",
            Self::Average => "average",
        }
    }
}

/// The frame's exposure: the radiance the tone map sees is the scene's times it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Exposure {
    /// A fixed multiplier (the default, 1.0). It applies from the frame it is set on.
    Manual(f32),
    /// Adapted on the GPU from the frame's meter, one frame late (ADR-0033).
    Auto(AutoExposure),
}

impl Default for Exposure {
    fn default() -> Self {
        Self::Manual(1.0)
    }
}

/// The automatic exposure's model (ADR-0033). In log2 luminance, with `k(tau) = 1 - exp(-dt /
/// tau)` (1 for `tau` 0): toward bright `a += (t - a) k(tau_light)`; toward dark `a` decays
/// toward `t` at `tau_cone` while `a > log2 cone_floor` and at `tau_rod` once below, a step that
/// crosses the floor spending `tau_cone ln((a - t) / (f - t))` of its `dt` reaching it. The
/// exposure is `key 2^compensation_ev / 2^a` clamped to `[min, ceiling]`, the ceiling `max`,
/// or under [`AutoExposure::sky_anchor_ev`] while the frame holds a sky the one in force,
/// which steps toward the anchored one ([`AutoExposure::ceiling`]) as far as the sky in view
/// holds it ([`AutoExposure::ceiling_target`], [`AutoExposure::step_ceiling`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutoExposure {
    /// The frame luminance a settled scene is exposed to (0.18).
    pub key: f32,
    /// The game's bias in EV (0).
    pub compensation_ev: f32,
    pub min: f32,
    pub max: f32,
    /// Time constants in seconds: toward bright, toward dark above the floor, below it.
    pub tau_light: f32,
    pub tau_cone: f32,
    pub tau_rod: f32,
    /// The frame luminance where dark adaptation turns from `tau_cone` to `tau_rod`; 0 is one
    /// dark stage at `tau_cone`.
    pub cone_floor: f32,
    pub metering: Metering,
    /// The ceiling anchored to the sky, `N` EV: while the frame holds a sky, the exposure
    /// opens at most `2^N` times the one that puts a sunlit horizontal face of albedo 0.5 at
    /// the key ([`AutoExposure::ceiling`]); `None` keeps `max`. The default is
    /// [`DEFAULT_SKY_ANCHOR_EV`]; with no sky set the ceiling is `max` whatever it holds.
    pub sky_anchor_ev: Option<f32>,
    /// The share of the view that is sky (`F`, under the meter's weights) at and over which
    /// the anchor holds in full; under it the anchor releases toward `max`, log2-linear in
    /// the share, to `max` with no sky in view ([`AutoExposure::ceiling_target`]). 0 holds the
    /// anchor whatever is in view. The default is [`DEFAULT_SKY_ANCHOR_SHARE`].
    pub sky_anchor_share: f32,
    /// The share of the hemisphere over the camera's floor that is open sky (`F_floor`, the
    /// baked `+z` sky visibility of the record under the eye, ADR-0081) at and over which the
    /// anchor holds in full; the anchor holds by the larger of this share and the view's
    /// ([`AutoExposure::ceiling_target`]). 0 drops the term (the view's share alone). The
    /// default is [`DEFAULT_SKY_ANCHOR_FLOOR`].
    pub sky_anchor_floor: f32,
    /// The time constant in seconds the floor's share is smoothed at before it reaches the
    /// exposure ([`SkyAround::step`]); 0 jumps. The default is [`DEFAULT_SKY_AROUND_TAU`].
    pub sky_around_tau: f32,
    /// Highlight protection's percentile `P` in `(0, 100]`: the exposure is also capped so
    /// the level over which `100 - P` percent of the view's pixels lie (their brightest
    /// channel, per meter tile) maps at or under [`HIGHLIGHT_WHITE`] after the tone curve
    /// (`highlight.rs`); 0 is off. The default is [`DEFAULT_EXPOSURE_HIGHLIGHT`].
    pub highlight_percentile: f32,
    /// The most the adapted level, and under the sky anchor the ceiling in force, move toward
    /// their targets per second, in EV (ADR-0076 rule 5): a frame's step is clamped to
    /// `rate_ev_s * min(dt, MAX_RATE_DT)`, so a hitch does not step further than a 15 fps
    /// frame. 0 is no cap (ADR-0033's speed alone, and the ceiling jumps).
    pub rate_ev_s: f32,
}

/// [`AutoExposure::rate_ev_s`]'s default: 1/16 EV a frame at 60 Hz, a step that moves a pixel
/// at 180 of 255 by about 8 levels; a 6 EV change takes about 1.6 s.
pub const DEFAULT_EXPOSURE_RATE_EV_S: f32 = 3.75;

/// The longest frame step the rate cap pays for, in seconds (a 15 fps frame).
pub(crate) const MAX_RATE_DT: f32 = 1.0 / 15.0;

/// [`AutoExposure::sky_anchor_ev`]'s default, in EV over the sunlit ground's exposure.
pub const DEFAULT_SKY_ANCHOR_EV: f32 = 4.0;

/// [`AutoExposure::sky_anchor_share`]'s default: the anchor holds in full while 1/256 of the
/// metered view is sky (Plan 0094 Phase 7b bar 1's reading: a forest floor under the canopy
/// meters under 1/128 of sky, a deep tunnel 0).
pub const DEFAULT_SKY_ANCHOR_SHARE: f32 = 1.0 / 256.0;

/// [`AutoExposure::sky_anchor_floor`]'s default: the anchor holds in full while 1/8 of the
/// floor's hemisphere is open sky (Plan 0097 Phase 1's reading: the forest floor at window W2's
/// two poses 0.14 and 0.20, the deep noon tunnel's 0.012; the largest power of two between).
pub const DEFAULT_SKY_ANCHOR_FLOOR: f32 = 1.0 / 8.0;

/// [`AutoExposure::sky_around_tau`]'s default in seconds (ADR-0081's guess).
pub const DEFAULT_SKY_AROUND_TAU: f32 = 1.0;

/// [`AutoExposure::highlight_percentile`]'s default (Plan 0097 Phase 3a's tunnel reading).
pub const DEFAULT_EXPOSURE_HIGHLIGHT: f32 = 99.0;

impl Default for AutoExposure {
    fn default() -> Self {
        Self::eye()
    }
}

impl AutoExposure {
    /// The eye: key 0.18, limits 2^-6 and 2^4, constants 0.15, 2 and 10 s, floor 2^-6,
    /// centre-weighted, the step capped at [`DEFAULT_EXPOSURE_RATE_EV_S`].
    pub fn eye() -> Self {
        Self {
            key: 0.18,
            compensation_ev: 0.0,
            min: 2f32.powi(-6),
            max: 2f32.powi(4),
            tau_light: 0.15,
            tau_cone: 2.0,
            tau_rod: 10.0,
            cone_floor: 2f32.powi(-6),
            metering: Metering::default(),
            sky_anchor_ev: Some(DEFAULT_SKY_ANCHOR_EV),
            sky_anchor_share: DEFAULT_SKY_ANCHOR_SHARE,
            sky_anchor_floor: DEFAULT_SKY_ANCHOR_FLOOR,
            sky_around_tau: DEFAULT_SKY_AROUND_TAU,
            highlight_percentile: DEFAULT_EXPOSURE_HIGHLIGHT,
            rate_ev_s: DEFAULT_EXPOSURE_RATE_EV_S,
        }
    }

    /// One rate both ways: every constant `1 / speed` seconds, no floor, no rate cap.
    pub fn symmetric(speed: f32) -> Self {
        let tau = 1.0 / speed;
        Self {
            tau_light: tau,
            tau_cone: tau,
            tau_rod: tau,
            cone_floor: 0.0,
            rate_ev_s: 0.0,
            ..Self::eye()
        }
    }

    /// Every constant 0: the exposure follows the meter at once.
    pub fn instant() -> Self {
        Self::symmetric(f32::INFINITY)
    }

    /// `log2 cone_floor`, or minus infinity for a floor of 0.
    pub fn floor_log2(&self) -> f64 {
        if self.cone_floor > 0.0 {
            f64::from(self.cone_floor).log2()
        } else {
            f64::NEG_INFINITY
        }
    }

    /// The adapted level after `dt` seconds toward the meter `metered_log2` from
    /// `adapted_log2`, the step clamped by [`AutoExposure::rate_ev_s`]; `dt` 0 returns it
    /// unchanged.
    pub fn step(&self, adapted_log2: f64, metered_log2: f64, dt: f64) -> f64 {
        let a = adapted_log2;
        let next = self.step_uncapped(adapted_log2, metered_log2, dt);
        if self.rate_ev_s <= 0.0 || dt <= 0.0 {
            return next;
        }
        let r = f64::from(self.rate_ev_s) * dt.min(f64::from(MAX_RATE_DT));
        a + (next - a).clamp(-r, r)
    }

    /// [`AutoExposure::step`] at ADR-0033's speed alone.
    fn step_uncapped(&self, adapted_log2: f64, metered_log2: f64, dt: f64) -> f64 {
        let (a, t) = (adapted_log2, metered_log2);
        if dt <= 0.0 {
            return a;
        }
        let decay = |dt: f64, tau: f32| {
            if tau > 0.0 {
                (-dt / f64::from(tau)).exp()
            } else {
                0.0
            }
        };
        if t >= a {
            return t + (a - t) * decay(dt, self.tau_light);
        }
        let f = self.floor_log2();
        if a > f {
            if t >= f {
                return t + (a - t) * decay(dt, self.tau_cone);
            }
            let to_floor = if self.tau_cone > 0.0 {
                f64::from(self.tau_cone) * ((a - t) / (f - t)).ln()
            } else {
                0.0
            };
            if dt <= to_floor {
                return t + (a - t) * decay(dt, self.tau_cone);
            }
            return t + (f - t) * decay(dt - to_floor, self.tau_rod);
        }
        t + (a - t) * decay(dt, self.tau_rod)
    }

    /// The exposure at the adapted level `adapted_log2`, under the ceiling `max`.
    pub fn exposure(&self, adapted_log2: f64) -> f64 {
        self.exposure_under(adapted_log2, self.max)
    }

    /// The exposure at the adapted level `adapted_log2` under the ceiling `ceiling` (the
    /// exposure pass's, which the state's `ceiling` reads back): `key 2^compensation_ev / 2^a`
    /// clamped to `[min, max(ceiling, min)]`.
    pub fn exposure_under(&self, adapted_log2: f64, ceiling: f32) -> f64 {
        let e = f64::from(self.key) * f64::from(self.compensation_ev).exp2() / adapted_log2.exp2();
        let lo = f64::from(self.min);
        e.clamp(lo, f64::from(ceiling).max(lo))
    }

    /// The ceiling in force over a frame whose sunlit ground's irradiance is `ground`
    /// ([`ground_irradiance`]; `None` with no sky set): with [`AutoExposure::sky_anchor_ev`]
    /// `N` and a ground over 0, `2^N key 2^compensation_ev / (0.5 ground / pi)`, the exposure
    /// that puts a horizontal face of albedo 0.5 under that irradiance at the key, opened `N`
    /// EV, clamped to `max`: the anchor lowers the ceiling and never lifts it (a dark sky, as
    /// at night, leaves `max` exactly); else `max`. The exposure pass computes the same in f32
    /// from `E_sky`.
    pub fn ceiling(&self, ground: Option<f32>) -> f32 {
        match (self.sky_anchor_ev, ground) {
            (Some(n), Some(g)) if g > 0.0 => (n.exp2() * self.key * self.compensation_ev.exp2()
                / (0.5 * g / std::f32::consts::PI))
                .min(self.max),
            _ => self.max,
        }
    }

    /// The ceiling's target over a frame whose sunlit ground's irradiance is `ground`, whose
    /// metered share of sky is `sky` in `[0, 1]` and whose camera's floor sees the share
    /// `floor` of its hemisphere open (`None`: no record answered, ADR-0081): with the anchor
    /// in force ([`AutoExposure::ceiling`] `c_a`, a ground over 0) and
    /// [`AutoExposure::sky_anchor_share`] `F` over 0, `log2 c = mix(log2 max, log2 c_a,
    /// min(s, 1))` with `s` the larger of `sky / F` and, with
    /// [`AutoExposure::sky_anchor_floor`] `F_floor` over 0 and a floor share, `floor /
    /// F_floor` ([`AutoExposure::floor_term`]): the anchored ceiling at a share of `F` and
    /// over or a floor share of `F_floor` and over, `max` with neither (`c_a` is never over
    /// `max`); else `c_a`.
    pub fn ceiling_target(&self, ground: Option<f32>, sky: f32, floor: Option<f32>) -> f32 {
        let a = self.ceiling(ground);
        let around = self.floor_term(floor);
        if !self.anchors(ground)
            || self.sky_anchor_share <= 0.0
            || sky >= self.sky_anchor_share
            || around.is_some_and(|s| s >= 1.0)
            || a >= self.max
        {
            return a;
        }
        let open = f64::from(self.max).log2();
        let view = sky / self.sky_anchor_share;
        let t = f64::from(around.map_or(view, |s| view.max(s)));
        let l = open + (f64::from(a).log2() - open) * t;
        l.exp2() as f32
    }

    /// The floor's normalised share `floor / F_floor` in `[0, 1]`, as the exposure uniform's
    /// `sky_around` carries it: `None` with [`AutoExposure::sky_anchor_floor`] 0 or no floor
    /// share.
    pub fn floor_term(&self, floor: Option<f32>) -> Option<f32> {
        if self.sky_anchor_floor <= 0.0 {
            return None;
        }
        floor.map(|f| (f.max(0.0) / self.sky_anchor_floor).min(1.0))
    }

    /// The anchor sets the ceiling: [`AutoExposure::sky_anchor_ev`] and a ground over 0.
    fn anchors(&self, ground: Option<f32>) -> bool {
        self.sky_anchor_ev.is_some() && ground.is_some_and(|g| g > 0.0)
    }

    /// The ceiling in force after `dt` seconds from `ceiling` toward `target`: under the
    /// anchor (`ground` as [`AutoExposure::ceiling`] takes it) with a share and a rate over 0,
    /// a step of at most `rate_ev_s * min(dt, MAX_RATE_DT)` EV in log2 (ADR-0076 rule 5); else,
    /// or from a ceiling of 0 (none yet), the target.
    pub fn step_ceiling(&self, ground: Option<f32>, ceiling: f32, target: f32, dt: f64) -> f32 {
        if !self.anchors(ground)
            || self.sky_anchor_share <= 0.0
            || self.rate_ev_s <= 0.0
            || ceiling <= 0.0
        {
            return target;
        }
        let r = f64::from(self.rate_ev_s) * dt.max(0.0).min(f64::from(MAX_RATE_DT));
        let c = f64::from(ceiling).log2();
        (c + (f64::from(target).log2() - c).clamp(-r, r)).exp2() as f32
    }
}

/// The irradiance a sunlit horizontal face at the sky's base altitude receives, in luminance
/// (Rec. 709): the sky's `E_sky(+z)` (`e_sky_up`) and the key light's `key_horizontal` (its
/// illuminance times the transmittance toward it times the cosine of its zenith angle, 0 with
/// no key light). The irradiance pass writes the key light's share into `E_sky(+z)`'s `w`.
pub fn ground_irradiance(e_sky_up: [f32; 3], key_horizontal: f32) -> f32 {
    0.2126 * e_sky_up[0] + 0.7152 * e_sky_up[1] + 0.0722 * e_sky_up[2] + key_horizontal
}

/// `ExState` in `exposure.wgsl`, field for field: 32 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ExposureState {
    /// The exposure the next frame's passes use.
    pub current: f32,
    /// The exposure the last frame's passes used.
    pub previous: f32,
    /// The adapted level, log2 of frame luminance.
    pub adapted_log2: f32,
    /// The last metered level, log2 of frame luminance.
    pub metered_log2: f32,
    /// Exposure passes run since the frame was made.
    pub frame: u32,
    /// [`STATE_HELD`].
    pub flags: u32,
    /// The ceiling the last pass clamped `current` under ([`AutoExposure::ceiling`]); in
    /// manual, the auto model's `max` of the uniform (0 when none was set).
    pub ceiling: f32,
    /// The last metered share of the view that is sky, under the meter's weights.
    pub sky: f32,
}

/// `ExposureState::flags` bit: the last pass metered nothing (another view than the shaded
/// one) and held the adapted and metered levels.
pub const STATE_HELD: u32 = 1;

impl ExposureState {
    /// The state as the frame starts: exposure 1.0, nothing metered.
    pub fn initial() -> Self {
        Self {
            current: 1.0,
            previous: 1.0,
            flags: STATE_HELD,
            ..Self::default()
        }
    }

    /// The state from its 32 bytes, as a blocking read of [`crate::Frame::exposure_state`]
    /// returns them.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        bytemuck::pod_read_unaligned(&bytes[..size_of::<Self>()])
    }

    pub fn readout(&self) -> ExposureReadout {
        ExposureReadout {
            exposure: self.current,
            adapted_log2: self.adapted_log2,
            metered_log2: self.metered_log2,
            frame: self.frame,
            held: self.flags & STATE_HELD != 0,
            ceiling: self.ceiling,
            sky_share: self.sky,
            sky_floor: None,
            highlight: None,
        }
    }
}

/// The exposure state as the CPU last read it back ([`crate::Frame::exposure_readout`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExposureReadout {
    /// The exposure the frame after the read one uses.
    pub exposure: f32,
    pub adapted_log2: f32,
    pub metered_log2: f32,
    /// Exposure passes run up to the read one.
    pub frame: u32,
    /// The read pass metered nothing and held the levels.
    pub held: bool,
    /// The ceiling the read pass clamped the exposure under ([`ExposureState::ceiling`]).
    pub ceiling: f32,
    /// The read pass's metered share of sky ([`ExposureState::sky`]).
    pub sky_share: f32,
    /// The sky share over the camera's floor the frame last fed the exposure, smoothed
    /// ([`SkyAround`]), `None` while absent or off: set by the frame on the CPU, not read
    /// back.
    pub sky_floor: Option<f32>,
    /// The highlight cap the read pass clamped the exposure under
    /// ([`ExposureHighlight::cap`]), `None` with the cap off.
    pub highlight: Option<f32>,
}

/// `ExUniform` in `exposure.wgsl`, field for field.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ExposureUniform {
    pub tiles: [u32; 2],
    pub view_count: u32,
    pub mode: u32,
    pub rects: [[f32; 4]; 2],
    pub manual: f32,
    pub strength: f32,
    pub sigma: f32,
    pub dt: f32,
    /// `key 2^compensation_ev`, the limits, the time constants and `log2 cone_floor` (a large
    /// negative number for no floor).
    pub key: f32,
    pub min: f32,
    pub max: f32,
    pub tau_light: f32,
    pub tau_cone: f32,
    pub tau_rod: f32,
    pub floor_log2: f32,
    /// `2^sky_anchor_ev` while the frame holds a sky and the model anchors its ceiling, else 0
    /// (the ceiling is `max`).
    pub anchor: f32,
    /// [`AutoExposure::rate_ev_s`] (0: no cap) and the longest step it pays for.
    pub rate_ev_s: f32,
    pub max_rate_dt: f32,
    /// [`AutoExposure::sky_anchor_share`], 0 without the anchor.
    pub sky_share: f32,
    /// The floor's share over `F_floor` in `[0, 1]` ([`AutoExposure::floor_term`]), read
    /// under [`EX_AROUND`] alone.
    pub sky_around: f32,
    /// Highlight protection: the tail's share of the view `1 - P / 100` (0: off) and the
    /// exposed value the tail is held at or under (`highlight.rs`).
    pub hi_tail: f32,
    pub hi_white: f32,
    pub _pad0: f32,
    pub _pad1: f32,
}

/// `ExUniform::mode` bit: the exposure is automatic.
pub(crate) const EX_AUTO: u32 = 1;
/// `ExUniform::mode` bit: the shaded view ran and its tiles hold the meter.
pub(crate) const EX_METER: u32 = 2;
/// `ExUniform::mode` bit: the adapted level jumps to the meter.
pub(crate) const EX_RESET: u32 = 4;
/// `ExUniform::mode` bit: `sky_around` holds the floor's share (ADR-0081); clear, the term is
/// absent and the view's share alone holds the anchor.
pub(crate) const EX_AROUND: u32 = 8;

impl ExposureUniform {
    /// The auto fields of `auto`, over a frame that holds a sky when `sky`.
    pub(crate) fn set_auto(&mut self, auto: &AutoExposure, sky: bool) {
        self.key = auto.key * auto.compensation_ev.exp2();
        self.min = auto.min;
        self.max = auto.max;
        self.tau_light = auto.tau_light;
        self.tau_cone = auto.tau_cone;
        self.tau_rod = auto.tau_rod;
        self.floor_log2 = auto.floor_log2().max(-1e30) as f32;
        self.anchor = match auto.sky_anchor_ev {
            Some(n) if sky => n.exp2(),
            _ => 0.0,
        };
        self.rate_ev_s = auto.rate_ev_s.max(0.0);
        self.max_rate_dt = MAX_RATE_DT;
        self.sky_share = if self.anchor > 0.0 {
            auto.sky_anchor_share.max(0.0)
        } else {
            0.0
        };
    }

    /// The floor's share `floor` under `auto` ([`AutoExposure::floor_term`]): the
    /// [`EX_AROUND`] bit and `sky_around` where the term is present, both clear where it is
    /// absent.
    pub(crate) fn set_around(&mut self, auto: &AutoExposure, floor: Option<f32>) {
        match auto.floor_term(floor) {
            Some(s) => {
                self.mode |= EX_AROUND;
                self.sky_around = s;
            }
            None => {
                self.mode &= !EX_AROUND;
                self.sky_around = 0.0;
            }
        }
    }
}

/// Read-back buffers in the ring.
const RING: usize = 3;
/// Bytes of one read-back buffer.
const RING_BYTES: u64 = 256;

/// A ring slot's progress.
enum Phase {
    Idle,
    /// The state was copied in by the encode of this index; not yet asked to map.
    Copied(u64),
    /// Asked to map: 0 pending, 1 mapped, 2 failed.
    Mapping(Arc<AtomicU8>),
}

struct Slot {
    buffer: wgpu::Buffer,
    phase: Phase,
}

struct Ring {
    slots: Vec<Slot>,
    /// Encodes so far.
    encodes: u64,
    latest: Option<ExposureReadout>,
}

/// The exposure pass, its state and its read-back ring.
pub(crate) struct ExposurePass {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    /// The state the pass writes (`STORAGE | COPY_SRC`).
    state: wgpu::Buffer,
    /// The copy of the state every frame pass reads through group 0 (`UNIFORM | COPY_DST`).
    state_uniform: wgpu::Buffer,
    /// The highlight's state ([`ExposureHighlight`], `STORAGE | COPY_SRC`).
    highlight: wgpu::Buffer,
    ring: Mutex<Ring>,
    /// The pass's bind group, built again only when the tiles buffer is replaced.
    binds: crate::gpu_util::BindCache,
}

impl ExposurePass {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let layout = io_layout(
            device,
            "exposure",
            &[
                Io::Uniform,
                Io::Storage,
                Io::ReadStorage,
                Io::ReadStorage,
                Io::Storage,
            ],
        );
        let pipeline = compute_pipeline(
            device,
            "exposure",
            exposure_source(),
            &[Some(&layout)],
            "exposure",
        );
        let init = |label, usage| {
            wgpu::util::DeviceExt::create_buffer_init(
                device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents: bytemuck::bytes_of(&ExposureState::initial()),
                    usage,
                },
            )
        };
        let state = init(
            "exposure state",
            wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
        );
        let state_uniform = init(
            "exposure state uniform",
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let highlight = wgpu::util::DeviceExt::create_buffer_init(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("exposure highlight"),
                contents: bytemuck::bytes_of(&ExposureHighlight::default()),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            },
        );
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("exposure params"),
            size: size_of::<ExposureUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let slots = (0..RING)
            .map(|_| Slot {
                buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("exposure readback"),
                    size: RING_BYTES,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                phase: Phase::Idle,
            })
            .collect();
        Self {
            pipeline,
            layout,
            uniform,
            state,
            state_uniform,
            highlight,
            binds: crate::gpu_util::BindCache::default(),
            ring: Mutex::new(Ring {
                slots,
                encodes: 0,
                latest: None,
            }),
        }
    }

    pub(crate) fn state(&self) -> &wgpu::Buffer {
        &self.state
    }

    pub(crate) fn state_uniform(&self) -> &wgpu::Buffer {
        &self.state_uniform
    }

    pub(crate) fn highlight(&self) -> &wgpu::Buffer {
        &self.highlight
    }

    /// Bind groups and texture views the pass created so far.
    pub(crate) fn binds_built(&self) -> (u64, u64) {
        self.binds.built()
    }

    /// Bytes the state, its uniform copy, the pass's parameters and the ring occupy.
    pub(crate) fn bytes(&self) -> u64 {
        2 * size_of::<ExposureState>() as u64
            + size_of::<ExposureHighlight>() as u64
            + size_of::<ExposureUniform>() as u64
            + RING as u64 * RING_BYTES
    }

    /// Writes the pass's parameters for this frame and records it over `tiles` and `E_sky`
    /// (`sky_e`, read for the anchored ceiling; its group built again only for other
    /// buffers), then the state's copies: into the uniform the next frame reads, and into a
    /// free ring slot.
    /// Slots copied by an earlier encode are asked to map first: every encode's commands are
    /// submitted before the next encode.
    pub(crate) fn encode(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        [tiles, sky_e]: [&wgpu::Buffer; 2],
        uniform: &ExposureUniform,
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        use crate::gpu_util::Res;
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(uniform));
        self.binds.tick();
        let bind = self.binds.io(
            device,
            &self.layout,
            &[
                Res::Buf(&self.uniform),
                Res::Buf(&self.state),
                Res::Buf(tiles),
                Res::Buf(sky_e),
                Res::Buf(&self.highlight),
            ],
        );
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("exposure"),
                timestamp_writes,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        let bytes = size_of::<ExposureState>() as u64;
        encoder.copy_buffer_to_buffer(&self.state, 0, &self.state_uniform, 0, bytes);
        let mut ring = self
            .ring
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = ring.encodes;
        ring.request_maps(now);
        if let Some(slot) = ring
            .slots
            .iter_mut()
            .find(|s| matches!(s.phase, Phase::Idle))
        {
            encoder.copy_buffer_to_buffer(&self.state, 0, &slot.buffer, 0, bytes);
            let hi = size_of::<ExposureHighlight>() as u64;
            encoder.copy_buffer_to_buffer(&self.highlight, 0, &slot.buffer, bytes, hi);
            slot.phase = Phase::Copied(now);
        }
        ring.encodes += 1;
    }

    /// The newest state a ring slot has finished mapping, after a non-blocking poll; slots
    /// copied before the last encode are asked to map first.
    pub(crate) fn readout(&self, device: &wgpu::Device) -> Option<ExposureReadout> {
        let mut ring = self
            .ring
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let last = ring.encodes.saturating_sub(1);
        ring.request_maps(last);
        let _ = device.poll(wgpu::PollType::Poll);
        let mut newest: Option<ExposureReadout> = ring.latest;
        for slot in &mut ring.slots {
            let Phase::Mapping(done) = &slot.phase else {
                continue;
            };
            match done.load(Ordering::Acquire) {
                0 => continue,
                1 => {
                    let r = {
                        let n = size_of::<ExposureState>();
                        let bytes = slot
                            .buffer
                            .get_mapped_range(..(n + size_of::<ExposureHighlight>()) as u64)
                            .expect("exposure readback mapped");
                        ExposureReadout {
                            highlight: ExposureHighlight::from_bytes(&bytes[n..]).cap(),
                            ..ExposureState::from_bytes(&bytes).readout()
                        }
                    };
                    slot.buffer.unmap();
                    if newest.is_none_or(|n| r.frame.wrapping_sub(n.frame) as i32 > 0) {
                        newest = Some(r);
                    }
                }
                _ => {}
            }
            slot.phase = Phase::Idle;
        }
        ring.latest = newest;
        newest
    }
}

impl Ring {
    /// Asks every slot copied by an encode before index `before` to map.
    fn request_maps(&mut self, before: u64) {
        for slot in &mut self.slots {
            if let Phase::Copied(at) = slot.phase
                && at < before
            {
                let done = Arc::new(AtomicU8::new(0));
                let flag = done.clone();
                slot.buffer.map_async(wgpu::MapMode::Read, .., move |r| {
                    flag.store(if r.is_ok() { 1 } else { 2 }, Ordering::Release);
                });
                slot.phase = Phase::Mapping(done);
            }
        }
    }
}

#[cfg(test)]
mod tests;
