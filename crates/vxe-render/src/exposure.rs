//! The frame's exposure (ADR-0033): the composite's meter, the exposure pass that turns it into
//! the state the next frame reads, and a ring of read-back buffers that hands that state to the
//! CPU a few frames late without waiting.
//!
//! The meter: in the shaded view the composite adds, per 8 x 8 workgroup of the internal
//! target, `clamp(log2(max(Y, 2^-16)), -16, 16)` of each pixel's radiance before exposure and
//! before the tone map (Rec. 709 luminance `Y`), a count of 1 and a sky count of 1 for a pixel
//! that is sky (a miss), into one tile each (four `f32`, the last 0). The exposure pass
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
pub use around::SkyAround;

/// The exposure pass, compiled after the prelude ([`exposure_source`]).
pub const EXPOSURE_WGSL: &str = include_str!("shaders/exposure.wgsl");

/// The exposure pass's composed source: the prelude, then [`EXPOSURE_WGSL`].
pub(crate) fn exposure_source() -> String {
    crate::frame::compose(&[EXPOSURE_WGSL])
}

/// Pixels on a meter tile's edge: the composite's workgroup.
pub(crate) const METER_TILE: u32 = 8;
/// Bytes of one meter tile: the log2 sum, the count, the sky count and 0, as `f32`.
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
    ring: Mutex<Ring>,
    /// The pass's bind group, built again only when the tiles buffer is replaced.
    binds: crate::gpu_util::BindCache,
}

impl ExposurePass {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let layout = io_layout(
            device,
            "exposure",
            &[Io::Uniform, Io::Storage, Io::ReadStorage, Io::ReadStorage],
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

    /// Bind groups and texture views the pass created so far.
    pub(crate) fn binds_built(&self) -> (u64, u64) {
        self.binds.built()
    }

    /// Bytes the state, its uniform copy, the pass's parameters and the ring occupy.
    pub(crate) fn bytes(&self) -> u64 {
        2 * size_of::<ExposureState>() as u64
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
                        let bytes = slot
                            .buffer
                            .get_mapped_range(..size_of::<ExposureState>() as u64)
                            .expect("exposure readback mapped");
                        ExposureState::from_bytes(&bytes).readout()
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
mod tests {
    use super::*;

    /// The exposure pass parses and validates headless, after the prelude.
    #[test]
    fn exposure_wgsl_validates_through_naga() {
        let source = exposure_source();
        let module = wgpu::naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("exposure.wgsl parses: {}", e.emit_to_string(&source)));
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::default(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("exposure.wgsl validates: {e:?}"));
    }

    /// The state and the uniform are the shader's sizes: 32 bytes and 112; a meter tile is 16.
    #[test]
    fn the_state_and_the_uniform_have_the_shaders_layout() {
        assert_eq!(size_of::<ExposureState>(), 32);
        assert_eq!(size_of::<ExposureUniform>(), 112);
        assert_eq!(METER_TILE_BYTES, 16);
    }

    /// The centre weight is 1 + strength at the centre and falls to 1 far out; the average
    /// weighs 1 everywhere.
    #[test]
    fn the_centre_weight_peaks_at_one_plus_strength() {
        let c = Metering::default();
        assert_eq!(c.weight(0.0), 4.0);
        assert!((c.weight(0.5) - (1.0 + 3.0 * (-0.5f32).exp())).abs() < 1e-6);
        assert!((c.weight(5.0) - 1.0).abs() < 1e-6);
        assert_eq!(Metering::Average.weight(0.0), 1.0);
    }

    /// The reference model's worked numbers (Plan 0040): from `a = -4` toward `t = -8` the
    /// floor `-6` is reached at `2 ln 2` s, and the rod stage leaves `2 exp(-(t - 2 ln 2) / 10)`;
    /// toward bright the step is `4 exp(-t / 0.15)`; a step of 0 holds; `instant` jumps.
    #[test]
    fn the_reference_step_follows_the_two_stage_curve() {
        let e = AutoExposure {
            rate_ev_s: 0.0,
            ..AutoExposure::eye()
        };
        let a = |t: f64| e.step(-4.0, -8.0, t);
        let floor_at = 2.0 * std::f64::consts::LN_2;
        assert!((a(floor_at) + 6.0).abs() < 1e-12);
        assert!((a(0.5) - (-8.0 + 4.0 * (-0.25f64).exp())).abs() < 1e-12);
        for t in [2.0, 4.0, 16.0] {
            let want = -8.0 + 2.0 * (-(t - floor_at) / 10.0).exp();
            assert!((a(t) - want).abs() < 1e-12, "{t}: {} {want}", a(t));
        }
        // Stepping in frames lands on the closed form.
        let mut x = -4.0;
        for _ in 0..600 {
            x = e.step(x, -8.0, 1.0 / 60.0);
        }
        assert!((x - a(10.0)).abs() < 1e-9, "{x} {}", a(10.0));
        assert!(
            (e.step(-8.0, -4.0, 0.15) - (-4.0 - 4.0 * (-1f64).exp())).abs() < 1e-6,
            "tau_light is 0.15 in f32"
        );
        assert_eq!(e.step(-5.3, -8.0, 0.0), -5.3);
        assert_eq!(AutoExposure::instant().step(-5.3, -8.0, 1.0 / 60.0), -8.0);
        assert!((e.exposure(-6.0) - f64::from(0.18f32) * 64.0).abs() < 1e-9);
        assert_eq!(e.exposure(-8.0), 16.0);
        assert_eq!(e.exposure(-12.0), 16.0);
        let s = AutoExposure::symmetric(2.0);
        assert!((s.step(-1.0, -3.0, 0.5) - (-3.0 + 2.0 * (-1f64).exp())).abs() < 1e-12);
    }

    /// The frame after a step of `d` EV toward a brighter meter by which the capped eye is within
    /// 1/64 EV of its target, from the model's constants: with `r` the per-frame cap and `tau`
    /// ADR-0033's light constant in frames, the cap binds while the gap exceeds `tau r`, then the
    /// exponential tail decays from there, plus 2 frames of rounding.
    pub(crate) fn landing_frame(d: f64, r: f64, tau: f64) -> u32 {
        ((d - tau * r) / r).ceil() as u32 + (tau * (64.0 * tau * r).ln()).ceil() as u32 + 2
    }

    /// ADR-0076 rule 5 (Plan 0094 Phase 4, the rate of Phase 7a): at the default rate and `dt`
    /// 1/60 a 2 EV step toward a brighter meter moves the adapted level by at most the rate's
    /// frame step (1/16 EV at 3.75 EV/s) within 1/256, monotonically, the first step being the
    /// cap, and is within 1/64 EV of the target by [`landing_frame`] (the frame it reaches printed);
    /// a long `dt` steps no further than a 15 fps frame's; with the rate 0 the step is
    /// ADR-0033's; a still meter holds the level.
    #[test]
    fn the_rate_caps_a_step() {
        let e = AutoExposure::eye();
        assert_eq!(e.rate_ev_s, DEFAULT_EXPOSURE_RATE_EV_S);
        let dt = 1.0 / 60.0;
        let r_f = f64::from(DEFAULT_EXPOSURE_RATE_EV_S) * dt;
        let landing = landing_frame(2.0, r_f, f64::from(e.tau_light) / dt);
        let (a0, t) = (-4.0, -2.0);
        let mut a = a0;
        let mut trace = vec![a];
        let mut reached = None;
        for k in 1..=160 {
            let next = e.step(a, t, dt);
            assert!(
                next >= a && next <= t,
                "monotonic toward the target: {a} -> {next}"
            );
            assert!((next - a) <= r_f + 1.0 / 256.0, "{a} -> {next}");
            a = next;
            trace.push(a);
            if reached.is_none() && (a - t).abs() <= 1.0 / 64.0 {
                reached = Some(k);
            }
        }
        println!(
            "reading: a 2 EV step at {DEFAULT_EXPOSURE_RATE_EV_S} EV/s is within 1/64 EV at frame \
             {reached:?}, the derived landing frame {landing}"
        );
        assert!(
            reached.is_some_and(|k| k <= landing),
            "{reached:?} {landing}"
        );
        assert!((trace[1] - (a0 + r_f)).abs() <= 1.0 / 256.0, "{trace:?}");
        let long = e.step(a0, t, 1.0);
        assert!((long - a0 - f64::from(DEFAULT_EXPOSURE_RATE_EV_S * MAX_RATE_DT)).abs() < 1e-6);
        let free = AutoExposure {
            rate_ev_s: 0.0,
            ..e
        };
        assert!(
            free.step(a0, t, dt) > a0 + r_f,
            "ADR-0033's bright step is faster"
        );
        assert_eq!(e.step(-3.0, -3.0, dt), -3.0);
        assert!((AutoExposure::eye().step(-3.0, -3.0 + 1.0 / 128.0, dt) + 3.0).abs() <= 1.0 / 64.0);
    }

    /// Plan 0086 Phase 17: the auto exposure's default anchors its ceiling 4 EV over the
    /// sunlit ground, and with no sky that ceiling is `max`.
    #[test]
    fn the_default_anchor_is_4() {
        assert_eq!(AutoExposure::default().sky_anchor_ev, Some(4.0));
        assert_eq!(
            AutoExposure::eye().sky_anchor_ev,
            Some(DEFAULT_SKY_ANCHOR_EV)
        );
        assert_eq!(
            AutoExposure::default().ceiling(None),
            AutoExposure::default().max
        );
    }

    /// Plan 0086 Phase 12 bar 1: the anchored ceiling follows the sky. Under `Sky::uniform(L)`
    /// with no key light `E_sky(+z)` is `pi L`, so at `N` 0 the ceiling is `key pi / (0.5 pi
    /// L)`; doubling `L` halves it, `N` 2 quadruples it; with no sky the ceiling is `max`, and
    /// the pass's uniform is byte for byte the one without the anchor, so the state it writes
    /// is too.
    #[test]
    fn the_ceiling_follows_the_sky() {
        use std::f32::consts::PI;
        let at = |n: f32| AutoExposure {
            sky_anchor_ev: Some(n),
            ..AutoExposure::eye()
        };
        let ground = |l: f32| Some(ground_irradiance([PI * l; 3], 0.0));
        for l in [0.5f32, 1.0, 37.0, 2400.0] {
            let want = 0.18 * PI / (0.5 * PI * l);
            let c0 = at(0.0).ceiling(ground(l));
            assert!((c0 - want).abs() <= 1e-6 * want, "{l}: {c0} {want}");
            let half = at(0.0).ceiling(ground(2.0 * l));
            assert!((half - 0.5 * c0).abs() <= 1e-6 * c0, "{l}: {half} {c0}");
            let four = at(2.0).ceiling(ground(l));
            assert!((four - 4.0 * c0).abs() <= 1e-6 * c0, "{l}: {four} {c0}");
        }
        // The key light's horizontal share adds to the sky's.
        assert_eq!(ground_irradiance([1.0, 1.0, 1.0], 2.0), 3.0);
        // No sky, or no anchor: `max`.
        let none = AutoExposure {
            sky_anchor_ev: None,
            ..AutoExposure::eye()
        };
        assert_eq!(at(2.0).ceiling(None), none.max);
        assert_eq!(at(2.0).ceiling(Some(0.0)), none.max);
        assert_eq!(none.ceiling(ground(1.0)), none.max);
        // A ground so dim the anchored exposure is over `max` (as at night): the anchor never
        // lifts the ceiling, it is `max` exactly, and the target is `max` at every share.
        let dim = ground(1e-3);
        let open = 4.0 * 0.18 * PI / (0.5 * PI * 1e-3);
        assert!(open > at(2.0).max, "{open}");
        assert_eq!(at(2.0).ceiling(dim), at(2.0).max);
        for k in 0..=64 {
            let sky = k as f32 / 64.0;
            assert_eq!(
                at(2.0).ceiling_target(dim, sky, None),
                at(2.0).max,
                "share {sky}"
            );
        }
        let uniform = |a: &AutoExposure, sky: bool| {
            let mut u = ExposureUniform::default();
            u.set_auto(a, sky);
            bytemuck::bytes_of(&u).to_vec()
        };
        assert_eq!(uniform(&at(2.0), false), uniform(&none, false));
        assert_eq!(uniform(&none, true), uniform(&none, false));
        assert_ne!(uniform(&at(2.0), true), uniform(&none, true));
        // The settled exposure clamps under the ceiling in force.
        let e = AutoExposure::eye();
        assert_eq!(e.exposure_under(-12.0, 0.25), 0.25);
        assert_eq!(e.exposure_under(-12.0, 1e-9), f64::from(e.min));
    }

    /// Plan 0094 Phase 7b: the ceiling's target is the anchored one at a sky share of `F` and
    /// over, `max` with no sky in view and log2-linear between; where the anchored exposure is
    /// over `max` (the night) the anchored ceiling and the target are `max` at any share; a
    /// share `F` of 0, no anchor or no sky keep today's ceiling.
    #[test]
    fn the_ceiling_target_follows_the_sky_in_view() {
        use std::f32::consts::PI;
        let e = AutoExposure {
            max: 4.0,
            ..AutoExposure::eye()
        };
        let f = e.sky_anchor_share;
        assert_eq!(f, DEFAULT_SKY_ANCHOR_SHARE);
        // Noon: a bright ground, the anchored ceiling far under `max`.
        let day = Some(ground_irradiance([PI * 2400.0; 3], 0.0));
        let a = e.ceiling(day);
        assert!(a < e.max, "{a}");
        let near = |x: f32, y: f32| (x.log2() - y.log2()).abs() < 1e-5;
        assert_eq!(e.ceiling_target(day, 1.0, None), a);
        assert_eq!(e.ceiling_target(day, f, None), a);
        assert!(near(e.ceiling_target(day, 0.0, None), e.max));
        assert!(near(
            e.ceiling_target(day, 0.5 * f, None),
            (a * e.max).sqrt()
        ));
        let mut last = e.ceiling_target(day, 0.0, None);
        for k in 1..=32 {
            let c = e.ceiling_target(day, f * k as f32 / 16.0, None);
            assert!(c <= last, "monotonic in the share: {k}");
            last = c;
        }
        // Night: the anchored exposure over `max`; the ceiling and its target are `max`.
        let night = Some(ground_irradiance([PI * 1e-3; 3], 0.0));
        assert_eq!(e.ceiling(night), e.max);
        for sky in [0.0, 0.5 * f, f, 1.0] {
            assert_eq!(e.ceiling_target(night, sky, None), e.max);
        }
        // The before arm, no anchor, no sky.
        let before = AutoExposure {
            sky_anchor_share: 0.0,
            ..e
        };
        assert_eq!(before.ceiling_target(day, 0.0, None), a);
        let none = AutoExposure {
            sky_anchor_ev: None,
            ..e
        };
        assert_eq!(none.ceiling_target(day, 0.0, None), e.max);
        assert_eq!(e.ceiling_target(None, 0.0, None), e.max);
        // The uniform carries the share under the anchor alone.
        let share = |a: &AutoExposure, sky: bool| {
            let mut u = ExposureUniform::default();
            u.set_auto(a, sky);
            u.sky_share
        };
        assert_eq!(share(&e, true), f);
        assert_eq!(share(&e, false), 0.0);
        assert_eq!(share(&none, true), 0.0);
    }

    /// Plan 0097 Phase 2 bar 1 (ADR-0081): the target holds by the larger of the view's share
    /// over `F` and the floor's over `F_floor`. At a day ground: a floor share of `F_floor`
    /// alone, or a view share of `F` alone, is `c_a` exactly; neither is `max`; half of
    /// `F_floor` is `sqrt(c_a max)`; half of each equals half of the view's alone. With
    /// `F_floor` 0 every pair reads as the floor absent (Phase 7b's target); at night `max`
    /// at every pair. The uniform carries the term under its bit alone.
    #[test]
    fn the_ceiling_target_follows_the_floor_too() {
        use std::f32::consts::PI;
        let e = AutoExposure {
            max: 4.0,
            ..AutoExposure::eye()
        };
        let (f, ff) = (e.sky_anchor_share, e.sky_anchor_floor);
        assert_eq!(ff, DEFAULT_SKY_ANCHOR_FLOOR);
        let day = Some(ground_irradiance([PI * 2400.0; 3], 0.0));
        let a = e.ceiling(day);
        assert!(a < e.max, "{a}");
        let ev = |x: f32, y: f32| (f64::from(x).log2() - f64::from(y).log2()).abs();
        assert_eq!(e.ceiling_target(day, 0.0, Some(ff)), a);
        assert_eq!(e.ceiling_target(day, 0.0, Some(1.0)), a);
        assert_eq!(e.ceiling_target(day, f, Some(0.0)), a);
        assert!(ev(e.ceiling_target(day, 0.0, Some(0.0)), e.max) <= 1.0 / 256.0);
        assert!(ev(e.ceiling_target(day, 0.0, None), e.max) <= 1.0 / 256.0);
        let half = e.ceiling_target(day, 0.0, Some(ff / 2.0));
        assert!(ev(half, (a * e.max).sqrt()) <= 1.0 / 256.0, "{half}");
        assert_eq!(
            e.ceiling_target(day, f / 2.0, Some(ff / 2.0)),
            e.ceiling_target(day, f / 2.0, Some(0.0))
        );
        assert_eq!(
            e.ceiling_target(day, f / 2.0, Some(0.0)),
            e.ceiling_target(day, f / 2.0, None)
        );
        // The larger share decides.
        assert_eq!(
            e.ceiling_target(day, f / 4.0, Some(ff / 2.0)),
            e.ceiling_target(day, 0.0, Some(ff / 2.0))
        );
        // `F_floor` 0: the floor's share is ignored, Phase 7b's target at every pair.
        let before = AutoExposure {
            sky_anchor_floor: 0.0,
            ..e
        };
        for sky in [0.0, f / 2.0, f, 1.0] {
            for floor in [None, Some(0.0), Some(ff / 2.0), Some(ff), Some(1.0)] {
                assert_eq!(
                    before.ceiling_target(day, sky, floor),
                    e.ceiling_target(day, sky, None),
                    "sky {sky} floor {floor:?}"
                );
            }
        }
        // Night: `max` at every pair.
        let night = Some(ground_irradiance([PI * 1e-3; 3], 0.0));
        for sky in [0.0, f / 2.0, f, 1.0] {
            for floor in [None, Some(0.0), Some(ff / 2.0), Some(ff), Some(1.0)] {
                assert_eq!(e.ceiling_target(night, sky, floor), e.max);
            }
        }
        // The uniform: the bit and the normalised share, clear when absent or off.
        let around = |a: &AutoExposure, floor: Option<f32>| {
            let mut u = ExposureUniform::default();
            u.set_auto(a, true);
            u.set_around(a, floor);
            (u.mode & EX_AROUND, u.sky_around)
        };
        assert_eq!(around(&e, Some(ff / 2.0)), (EX_AROUND, 0.5));
        assert_eq!(around(&e, Some(0.9)), (EX_AROUND, 1.0));
        assert_eq!(around(&e, None), (0, 0.0));
        assert_eq!(around(&before, Some(0.9)), (0, 0.0));
    }

    /// Plan 0094 Phase 7b bar 4's model (ADR-0076 rule 5 on the ceiling): from the anchored
    /// ceiling toward `max` and back the ceiling in force moves by at most the rate's frame
    /// step, monotonically, and lands on its target; at rate 0, from no ceiling, with no anchor
    /// or with the share 0 it jumps.
    #[test]
    fn the_ceiling_steps_at_the_rate() {
        use std::f32::consts::PI;
        let e = AutoExposure {
            max: 4.0,
            ..AutoExposure::eye()
        };
        let dt = 1.0 / 60.0;
        let r_f = f64::from(e.rate_ev_s) * dt;
        let day = Some(ground_irradiance([PI * 2400.0; 3], 0.0));
        let a = e.ceiling(day);
        let gap = f64::from(e.max.log2() - a.log2());
        let frames = (gap / r_f).ceil() as usize;
        let mut c = a;
        let mut trace = vec![c.log2()];
        for target in [e.max, a] {
            for _ in 0..frames + 2 {
                let next = e.step_ceiling(day, c, target, dt);
                let d = f64::from(next.log2() - c.log2());
                assert!(d.abs() <= r_f + 1.0 / 256.0, "{c} -> {next}");
                assert!(
                    (target >= c && next >= c) || (target <= c && next <= c),
                    "monotonic: {c} -> {next} toward {target}"
                );
                c = next;
                trace.push(c.log2());
            }
            assert!((c.log2() - target.log2()).abs() < 1e-4, "{trace:?}");
        }
        println!("reading: the ceiling's trace in EV {trace:?}");
        let jump = |m: &AutoExposure, g: Option<f32>, from: f32| m.step_ceiling(g, from, e.max, dt);
        let free = AutoExposure {
            rate_ev_s: 0.0,
            ..e
        };
        assert_eq!(jump(&free, day, a), e.max);
        assert_eq!(jump(&e, day, 0.0), e.max);
        assert_eq!(jump(&e, None, a), e.max);
        let before = AutoExposure {
            sky_anchor_share: 0.0,
            ..e
        };
        assert_eq!(jump(&before, day, a), e.max);
    }
}
