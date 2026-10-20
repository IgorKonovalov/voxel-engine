//! The late tail: feedback delay networks shared by every voice (ADR-0015).
//!
//! [`REVERB_LINES`] delay lines are mixed by the Householder matrix `I - (2/N) 1 1^T` (an
//! orthogonal, energy-preserving feedback: each output is the line's input minus twice the
//! lines' mean), and each line's output runs through a first-order low-pass
//! `y = g (1 - a) x + a y` (Jot and Chaigne 1991; Smith, "Delay-Line Damping Filter Design").
//! For a line of length `M`, `g = 10 ^ (-3 M / (T60 f_s))` gives the decay `T60` at DC, and `a`
//! is solved so the gain at 4 kHz is the same expression at `T60 * hf_ratio`.
//!
//! Line `i`'s target length is the mean free path's travel time times
//! `spread_i = 0.5 * 3 ^ (i / 15)`, clamped to `[32, REVERB_MAX_DELAY_FRAMES]` and rounded to
//! the nearest prime, so the lines share no common period.
//!
//! # A change of room
//!
//! There are two networks, one of them fed. How a room with another mean free path reaches
//! them is [`TailLines`]:
//! - [`TailLines::Handover`] (the default) never moves a line that holds signal, since a
//!   recirculating signal read through a moving line is shifted in pitch on every pass. A room
//!   whose mean free path differs from the fed network's by more than [`TAIL_HANDOVER_RATIO`]
//!   either way goes to the other network: it is cleared, takes the new lengths at once, and
//!   the input crossfades to it over [`crate::SMOOTHING_SECONDS`] (equal power, the two being
//!   uncorrelated), while the old network keeps its lengths and rings out at its own decay with
//!   no input. If the other network still rings when a handover is due, it first fades to
//!   silence over [`crate::SMOOTHING_SECONDS`], then is cleared. A room within the ratio
//!   changes the fed network's decay and energy in place, its lengths unchanged.
//! - [`TailLines::Glide`] keeps one network and glides each line's length toward its target
//!   by at most [`REVERB_DELAY_SLEW_FRAMES_PER_FRAME`] per frame through a linearly
//!   interpolated fractional read.
//!
//! The first room sets the fed network's lengths and decay at once and its input gain rises
//! from 0 through the smoother. A decay changed in place moves through the smoother; the loop
//! gains follow the current lengths and the smoothed `T60` every block.
//!
//! # Pre-delay
//!
//! The voices' summed sends pass a mono line of [`REVERB_MAX_PREDELAY_FRAMES`] plus a block
//! before the networks, read [`ReverbSettings::predelay_s`] back. A new pre-delay never glides
//! (that would repitch the input): the read crossfades linearly from the old read point to the
//! new one over [`crate::SMOOTHING_SECONDS`]; a change asked for while one crossfades waits for
//! it to finish.
//!
//! # Level
//!
//! Every line takes the same input `b * x`, `x` the sum of the voices' sends. At frequency `w`
//! a pass through line `i` keeps `|H_i(w)|^2` of the energy and the Householder matrix
//! preserves it, so with `G(w)` the mean of `|H_i(w)|^2` over the lines, a unit impulse leaves
//! `N b^2 G (1 + G + G^2 + ...) = N b^2 G / (1 - G)` in the filtered line outputs at `w`. Each
//! ear takes half the lines (disjoint halves, alternating signs, so the ears decorrelate), so
//! `b^2 = 2 E / (N mean_w [G / (1 - G)])`, the mean over [`ENERGY_BANDS`] frequencies spread
//! evenly to Nyquist, gives each ear the tail energy `E` the room asks for.
//!
//! The steady output for a steady input is `b^2` times a loop factor that grows with `T60`.
//! So that a change in place never overshoots the new steady level, `b` is computed from the
//! larger of the smoothed and the target `T60` (the loop gains rise no faster than the
//! smoother lets them, `b` already at the longer decay's value), and `b` moves up through the
//! smoother but down to its target within the block.

use crate::command::RoomTargets;
use crate::{BLOCK, RATE, SMOOTHING_SECONDS, SPEED_OF_SOUND};

/// How a change of room reaches the tail's lines (module comment).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TailLines {
    /// Two networks: a room with another mean free path goes to the silent one, the input
    /// crossfades to it, and no line that holds signal moves.
    #[default]
    Handover,
    /// One network whose lines glide to their new lengths.
    Glide,
}

/// The longest pre-delay: 200 ms.
pub const REVERB_MAX_PREDELAY_FRAMES: usize = 9_600;
/// The pre-delay line holds the longest pre-delay plus a block.
const PRE_RING: usize = REVERB_MAX_PREDELAY_FRAMES + BLOCK;

/// A room whose mean free path differs from the fed network's by more than this factor,
/// either way, is handed to the other network ([`TailLines::Handover`]).
pub const TAIL_HANDOVER_RATIO: f32 = 1.15;

/// Delay lines in the network.
pub const REVERB_LINES: usize = 16;
/// The longest line: 100 ms.
pub const REVERB_MAX_DELAY_FRAMES: usize = 4_800;
/// The shortest line.
pub const REVERB_MIN_DELAY_FRAMES: usize = 32;
/// The most a line's length moves toward its target per frame of audio.
pub const REVERB_DELAY_SLEW_FRAMES_PER_FRAME: f32 = 0.02;
/// The frequency at which a line's decay is `T60 * hf_ratio`.
const HF_HZ: f32 = 4_000.0;
/// Frequencies the input gain's energy average samples, evenly from 0 to Nyquist.
const ENERGY_BANDS: usize = 64;
/// Each ring holds the longest line plus a block.
const RING: usize = REVERB_MAX_DELAY_FRAMES + BLOCK;

/// How the tail sounds beyond what the room gives it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReverbSettings {
    /// The decay time at 4 kHz as a fraction of the decay time at DC.
    pub hf_ratio: f32,
    /// Amplitude scale of the whole tail.
    pub level: f32,
    /// The longest decay time the tail takes.
    pub max_t60_s: f32,
    /// How a change of room reaches the lines.
    pub lines: TailLines,
    /// Seconds the voices' sends wait before they enter the networks, 0 to 0.2.
    pub predelay_s: f32,
}

impl ReverbSettings {
    /// The pre-delay in whole frames, clamped to `[0, REVERB_MAX_PREDELAY_FRAMES]`.
    pub fn predelay_frames(&self) -> usize {
        (self.predelay_s.max(0.0) * RATE as f32)
            .round()
            .min(REVERB_MAX_PREDELAY_FRAMES as f32) as usize
    }
}

impl Default for ReverbSettings {
    fn default() -> Self {
        Self {
            hf_ratio: 0.5,
            level: 1.0,
            max_t60_s: 8.0,
            lines: TailLines::Handover,
            predelay_s: 0.0,
        }
    }
}

/// One network as [`crate::Mixer::reverb_networks`] reads it.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReverbNetwork {
    pub lines: [ReverbLine; REVERB_LINES],
    /// Some line or filter state is nonzero.
    pub holds_signal: bool,
    /// The network is the fed one.
    pub fed: bool,
    /// Its input weight now, 0 to 1.
    pub input: f32,
}

/// One line as [`crate::Mixer::reverb_lines`] reads it.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReverbLine {
    /// Frames, fractional while gliding.
    pub length: f32,
    pub target: u32,
}

/// `spread_i = 0.5 * 3 ^ (i / 15)`.
fn spread(i: usize) -> f32 {
    0.5 * 3f32.powf(i as f32 / (REVERB_LINES - 1) as f32)
}

fn is_prime(n: u32) -> bool {
    n >= 2
        && (2..)
            .take_while(|d| d * d <= n)
            .all(|d| !n.is_multiple_of(d))
}

/// The prime nearest to `n`, the lower one on a tie.
fn nearest_prime(n: u32) -> u32 {
    (0..)
        .find_map(|k| {
            [n.saturating_sub(k), n + k]
                .into_iter()
                .find(|&c| is_prime(c))
        })
        .unwrap_or(2)
}

/// Line `i`'s target length for mean free path `mfp` metres.
fn target_length(i: usize, mfp: f32) -> u32 {
    let frames = (mfp / SPEED_OF_SOUND * RATE as f32 * spread(i)).clamp(
        REVERB_MIN_DELAY_FRAMES as f32,
        REVERB_MAX_DELAY_FRAMES as f32,
    );
    nearest_prime(frames.round() as u32).min(REVERB_MAX_DELAY_FRAMES as u32)
}

/// The loop gain `g` and low-pass pole `a` of a line of `m` frames for decay `t60` seconds:
/// DC gain `g`, gain at [`HF_HZ`] that of decay `t60 * hf_ratio`.
fn line_filter(m: f32, t60: f32, hf_ratio: f32) -> (f32, f32) {
    let fs = RATE as f32;
    let g = 10f32.powf(-3.0 * m / (t60 * fs));
    let g_hf = 10f32.powf(-3.0 * m / (t60 * hf_ratio.max(1e-3) * fs));
    let r = ((g_hf / g) * (g_hf / g)).min(1.0);
    if r >= 1.0 - 1e-9 {
        return (g, 0.0);
    }
    // |g (1 - a) / (1 - a e^-jw)|^2 = g_hf^2 at w: (1 - r) a^2 - 2 (1 - r c) a + (1 - r) = 0,
    // the root inside the unit circle.
    let c = (std::f32::consts::TAU * HF_HZ / fs).cos();
    let p = 1.0 - r * c;
    let q = 1.0 - r;
    let a = (p - (p * p - q * q).max(0.0).sqrt()) / q;
    (g, a.clamp(0.0, 0.999))
}

/// Below this magnitude the network's state is set to exactly 0. Left alone, a tail ringing
/// out in silence decays into subnormal floats and stays there (round-to-nearest holds the
/// smallest subnormal under the low-pass pole), and subnormal arithmetic costs several times
/// normal arithmetic on every sample of every later block. `1e-20` is 400 dB under full
/// scale, far under audibility and far above `f32::MIN_POSITIVE` (about `1.2e-38`), so no
/// product of a flushed value with a gain under 1 reaches the subnormal range either; the
/// network rings out to exact zeros. Portable: no flush-to-zero mode of the FPU is set.
const FLUSH_BELOW: f32 = 1e-20;

/// `i` in `[0, 2 RING)` brought into the ring, without a division.
#[inline]
fn wrap(i: usize) -> usize {
    if i >= RING { i - RING } else { i }
}

/// `x`, or 0 when `|x| < FLUSH_BELOW`. A select, not a branch, in the compiled loop.
#[inline]
fn flush(x: f32) -> f32 {
    if x.abs() < FLUSH_BELOW { 0.0 } else { x }
}

/// One network and its state, allocated once.
struct Network {
    lines: [Vec<f32>; REVERB_LINES],
    /// Where the next frame is written, the same in every ring.
    write: usize,
    length: [f32; REVERB_LINES],
    target: [u32; REVERB_LINES],
    /// The mean free path the targets were set for.
    mfp: f32,
    /// Each line's low-pass state.
    lowpass: [f32; REVERB_LINES],
    /// Each line's filter at the end of the last block, `(g (1 - a), a)`, which this block's
    /// coefficients ramp from, sample by sample, so a change of decay or length never steps
    /// the loop gain at a block edge.
    coef: [(f32, f32); REVERB_LINES],
    /// `T60` and input gain `b` as smoothed so far, and the `T60` target.
    t60: f32,
    t60_target: f32,
    b: f32,
    energy: f32,
    /// Frames the decay holds at `t60` before it moves toward `t60_target` (a longer decay
    /// with less energy waits for the stored tail to fall to the new level).
    hold: f32,
    /// Lengths and decay have been set.
    started: bool,
    /// The network may hold signal; cleared once it has rung out with no input.
    active: bool,
    /// Output gain while fading out before a clear: `Some(g)`, `g` falling to 0.
    fade: Option<f32>,
}

impl Network {
    fn new() -> Self {
        Self {
            lines: std::array::from_fn(|_| vec![0.0; RING]),
            write: 0,
            length: [REVERB_MIN_DELAY_FRAMES as f32; REVERB_LINES],
            target: [REVERB_MIN_DELAY_FRAMES as u32; REVERB_LINES],
            mfp: 0.0,
            lowpass: [0.0; REVERB_LINES],
            coef: [(0.0, 0.0); REVERB_LINES],
            t60: 1.0,
            t60_target: 1.0,
            b: 0.0,
            energy: 0.0,
            hold: 0.0,
            started: false,
            active: false,
            fade: None,
        }
    }

    /// Silences every line and filter.
    fn clear(&mut self) {
        for line in &mut self.lines {
            line.fill(0.0);
        }
        self.lowpass = [0.0; REVERB_LINES];
        self.active = false;
        self.fade = None;
    }

    /// Takes `room`'s lengths and decay at once, the input gain `b` from `b`.
    fn set_at_once(&mut self, room: RoomTargets, settings: &ReverbSettings, b: f32) {
        let mfp = room.mean_free_path_m.max(0.0);
        for (i, t) in self.target.iter_mut().enumerate() {
            *t = target_length(i, mfp);
        }
        self.mfp = mfp;
        self.length = self.target.map(|t| t as f32);
        self.t60_target = room.t60_s.clamp(0.05, settings.max_t60_s);
        self.t60 = self.t60_target;
        self.energy = room.energy.max(0.0);
        self.b = b;
        self.hold = 0.0;
        for (c, &m) in self.coef.iter_mut().zip(&self.length) {
            let (g, a) = line_filter(m, self.t60, settings.hf_ratio);
            *c = (g * (1.0 - a), a);
        }
        self.started = true;
        self.active |= self.energy > 0.0;
    }

    /// Takes `room`'s decay and energy in place; `glide` also retargets the lengths.
    fn set_in_place(&mut self, room: RoomTargets, settings: &ReverbSettings, glide: bool) {
        if glide {
            let mfp = room.mean_free_path_m.max(0.0);
            for (i, t) in self.target.iter_mut().enumerate() {
                *t = target_length(i, mfp);
            }
            self.mfp = mfp;
        }
        let t60 = room.t60_s.clamp(0.05, settings.max_t60_s);
        let energy = room.energy.max(0.0);
        // A longer decay with less energy: the stored tail, fed at the old level, would ring
        // on at the new decay above the new steady level. Hold the old decay for as long as it
        // takes to fall by the energy ratio, then let it rise.
        self.hold = if t60 > self.t60 && energy < self.energy {
            let db = if energy > 0.0 {
                (10.0 * (self.energy / energy).log10()).min(60.0)
            } else {
                60.0
            };
            self.t60 * db / 60.0 * RATE as f32
        } else {
            0.0
        };
        self.t60_target = t60;
        self.energy = energy;
        self.active |= self.energy > 0.0;
    }

    /// Whether any line or filter state is nonzero.
    fn holds_signal(&self) -> bool {
        self.lowpass.iter().any(|&x| x != 0.0)
            || self.lines.iter().any(|l| l.iter().any(|&x| x != 0.0))
    }

    /// `b` for the network's energy at decay `t60` and its current lengths:
    /// `b^2 = 2 E level^2 / (N mean_w [G / (1 - G)])` (module comment).
    fn input_gain(&self, t60: f32, settings: &ReverbSettings, band_cos: &[f32]) -> f32 {
        let e = self.energy * settings.level * settings.level;
        if e <= 0.0 {
            return 0.0;
        }
        let filters = self.length.map(|m| line_filter(m, t60, settings.hf_ratio));
        let mut sum = 0.0f32;
        for &c in band_cos {
            let g2 = filters
                .iter()
                .map(|&(g, a)| g * g * (1.0 - a) * (1.0 - a) / (1.0 - 2.0 * a * c + a * a))
                .sum::<f32>()
                / REVERB_LINES as f32;
            sum += g2 / (1.0 - g2).max(1e-6);
        }
        let mean = sum / band_cos.len() as f32;
        (2.0 * e / (REVERB_LINES as f32 * mean.max(1e-9))).sqrt()
    }

    /// Runs one block: `input` (the voices' summed sends, `n` frames) times an input weight
    /// moving linearly from `w0` to `w1` across the block, both ears added to interleaved
    /// `out` times the fade's gain. A finished fade-out clears the network.
    fn render(
        &mut self,
        input: &[f32],
        out: &mut [f32],
        (w0, w1): (f32, f32),
        settings: &ReverbSettings,
        band_cos: &[f32],
    ) {
        if !self.active {
            return;
        }
        let n = input.len();
        let inv_n = 1.0 / n as f32;
        let k_block = 1.0 - (-(n as f32) / (SMOOTHING_SECONDS * RATE as f32)).exp();
        if self.hold > 0.0 {
            self.hold = (self.hold - n as f32).max(0.0);
        } else {
            self.t60 += (self.t60_target - self.t60) * k_block;
        }
        // From the longer of the two decays, so the input never runs ahead of the loop gains.
        let b_target = self.input_gain(self.t60.max(self.t60_target), settings, band_cos);
        let b0 = self.b;
        self.b = if b_target < b0 {
            b_target
        } else {
            b0 + (b_target - b0) * k_block
        };
        let b1 = self.b;
        // A fade-out runs linearly to 0 over the smoother's time.
        let f0 = self.fade.unwrap_or(1.0);
        let f1 = self.fade.map_or(1.0, |f| {
            (f - n as f32 / (SMOOTHING_SECONDS * RATE as f32)).max(0.0)
        });
        if self.fade.is_some() {
            self.fade = Some(f1);
        }
        let reach = REVERB_DELAY_SLEW_FRAMES_PER_FRAME * n as f32;
        let mut l0 = [0.0f32; REVERB_LINES];
        let mut dl = [0.0f32; REVERB_LINES];
        let c0 = self.coef;
        let mut dc = [(0.0f32, 0.0f32); REVERB_LINES];
        for i in 0..REVERB_LINES {
            let m0 = self.length[i];
            let m1 = m0 + (self.target[i] as f32 - m0).clamp(-reach, reach);
            let (g, a) = line_filter(m1, self.t60, settings.hf_ratio);
            self.coef[i] = (g * (1.0 - a), a);
            dc[i] = (
                (self.coef[i].0 - c0[i].0) * inv_n,
                (self.coef[i].1 - c0[i].1) * inv_n,
            );
            l0[i] = m0;
            dl[i] = (m1 - m0) * inv_n;
            self.length[i] = m1;
        }
        // A line that holds a whole length over the block reads one ring sample per frame.
        let whole: [Option<usize>; REVERB_LINES] = std::array::from_fn(|i| {
            (dl[i] == 0.0 && l0[i].fract() == 0.0 && l0[i] >= 1.0).then_some(l0[i] as usize)
        });
        let (bw0, bw1) = (b0 * w0, b1 * w1);
        let mut peak = 0.0f32;
        for (j, (&x, f)) in input.iter().zip(out.chunks_exact_mut(2)).enumerate() {
            let t = (j + 1) as f32;
            let bx = (bw0 + (bw1 - bw0) * t * inv_n) * x;
            let gain_out = f0 + (f1 - f0) * t * inv_n;
            let mut y = [0.0f32; REVERB_LINES];
            let mut sum = 0.0f32;
            for i in 0..REVERB_LINES {
                let line = &self.lines[i];
                // Read `m` frames behind the write position. A target is at least 32 frames
                // and a test's jump leaves at least 1, so the earlier of the two frames read
                // was written in an earlier frame; at `m = 1` the later one is this frame's
                // slot, still holding its old value, and takes weight 0.
                let raw = match whole[i] {
                    Some(m) => line[wrap(self.write + RING - m)],
                    None => {
                        let m = l0[i] + dl[i] * t;
                        let pos = (self.write + RING) as f32 - m;
                        let base = pos.floor();
                        let frac = pos - base;
                        let i0 = wrap(base as usize);
                        let i1 = wrap(i0 + 1);
                        line[i0] + (line[i1] - line[i0]) * frac
                    }
                };
                let (gain, pole) = (c0[i].0 + dc[i].0 * t, c0[i].1 + dc[i].1 * t);
                let lp = flush(gain * raw + pole * self.lowpass[i]);
                self.lowpass[i] = lp;
                y[i] = lp;
                sum += lp;
            }
            let half = REVERB_LINES / 2;
            let (mut left, mut right) = (0.0f32, 0.0f32);
            for i in 0..half {
                let s = if i % 2 == 0 { 1.0 } else { -1.0 };
                left += s * y[i];
                right += s * y[half + i];
            }
            f[0] += left * gain_out;
            f[1] += right * gain_out;
            peak = peak.max(left.abs()).max(right.abs()).max(bx.abs());
            let feedback = 2.0 * sum / REVERB_LINES as f32;
            for (line, &yi) in self.lines.iter_mut().zip(&y) {
                line[self.write] = flush(yi - feedback + bx);
            }
            self.write = wrap(self.write + 1);
        }
        // Faded out, or rung out with nothing coming in: clear and stop.
        if self.fade == Some(0.0) || (bw1 < 1e-9 && peak < 1e-9) {
            self.clear();
        }
    }
}

/// The tail: two networks, one fed, and how a change of room moves between them.
pub(crate) struct Reverb {
    nets: [Network; 2],
    /// The network the input goes to.
    fed: usize,
    /// The input crossfade from the other network to `fed`, 0 to 1; 1 when done.
    xfade: f32,
    /// A room waiting for the other network to fade out and clear.
    pending: Option<RoomTargets>,
    settings: ReverbSettings,
    /// `cos w` at the centre of each of the [`ENERGY_BANDS`] bands.
    band_cos: [f32; ENERGY_BANDS],
    pre: PreDelay,
}

/// The pre-delay line and its crossfade between two read points (module comment).
struct PreDelay {
    ring: Vec<f32>,
    write: usize,
    /// The read point being left and the one being taken, frames back.
    from: usize,
    to: usize,
    /// Crossfade progress from `from` to `to`, 0 to 1; 1 when done.
    x: f32,
    /// A pre-delay waiting for the crossfade in progress.
    pending: Option<usize>,
    /// One block of the delayed input.
    out: Vec<f32>,
}

impl PreDelay {
    fn new() -> Self {
        Self {
            ring: vec![0.0; PRE_RING],
            write: 0,
            from: 0,
            to: 0,
            x: 1.0,
            pending: None,
            out: vec![0.0; BLOCK],
        }
    }

    /// Asks for a read point `frames` back.
    fn set(&mut self, frames: usize) {
        if self.x < 1.0 {
            self.pending = (frames != self.to).then_some(frames);
        } else if frames != self.to {
            self.from = self.to;
            self.to = frames;
            self.x = 0.0;
        }
    }

    /// Writes `input` into the line and returns it delayed.
    fn process(&mut self, input: &[f32]) -> &[f32] {
        let n = input.len();
        if self.x >= 1.0 && self.pending.is_none() && self.to == 0 {
            // No pre-delay: the line still records, so a later one has history to read.
            for &v in input {
                self.ring[self.write] = v;
                self.write = if self.write + 1 == PRE_RING {
                    0
                } else {
                    self.write + 1
                };
            }
            self.out[..n].copy_from_slice(input);
            return &self.out[..n];
        }
        let step = 1.0 / (SMOOTHING_SECONDS * RATE as f32);
        let back = |w: usize, d: usize| if w >= d { w - d } else { w + PRE_RING - d };
        for (j, &v) in input.iter().enumerate() {
            self.ring[self.write] = v;
            let a = self.ring[back(self.write, self.from)];
            let b = self.ring[back(self.write, self.to)];
            if self.x < 1.0 {
                self.x = (self.x + step).min(1.0);
            }
            self.out[j] = a + (b - a) * self.x;
            self.write = if self.write + 1 == PRE_RING {
                0
            } else {
                self.write + 1
            };
            if self.x >= 1.0
                && let Some(next) = self.pending.take()
            {
                self.from = self.to;
                self.to = next;
                self.x = 0.0;
            }
        }
        &self.out[..n]
    }
}

impl Reverb {
    pub fn new() -> Self {
        Self {
            nets: [Network::new(), Network::new()],
            fed: 0,
            xfade: 1.0,
            pending: None,
            settings: ReverbSettings::default(),
            band_cos: std::array::from_fn(|k| {
                (std::f32::consts::PI * (k as f32 + 0.5) / ENERGY_BANDS as f32).cos()
            }),
            pre: PreDelay::new(),
        }
    }

    /// Bytes the lines of both networks hold.
    pub fn line_bytes() -> usize {
        2 * REVERB_LINES * RING * 4
    }

    /// Bytes the pre-delay line holds.
    pub fn predelay_bytes() -> usize {
        PRE_RING * 4
    }

    pub fn set_settings(&mut self, settings: ReverbSettings) {
        self.settings = settings;
        self.pre.set(settings.predelay_frames());
        for net in &mut self.nets {
            net.t60_target = net.t60_target.min(settings.max_t60_s);
        }
    }

    /// Takes a room (module comment, "A change of room").
    pub fn set_room(&mut self, room: RoomTargets) {
        let settings = self.settings;
        let fed = &mut self.nets[self.fed];
        if !fed.started {
            fed.set_at_once(room, &settings, 0.0);
            return;
        }
        match settings.lines {
            TailLines::Glide => {
                self.pending = None;
                fed.set_in_place(room, &settings, true);
            }
            TailLines::Handover => {
                let (a, b) = (fed.mfp, room.mean_free_path_m.max(0.0));
                let within = (a == 0.0 && b == 0.0)
                    || (a > 0.0 && b > 0.0 && a.max(b) <= TAIL_HANDOVER_RATIO * a.min(b));
                if within {
                    self.pending = None;
                    fed.set_in_place(room, &settings, false);
                } else {
                    self.pending = Some(room);
                    let other = &mut self.nets[1 - self.fed];
                    if other.active && other.fade.is_none() {
                        other.fade = Some(1.0);
                    }
                    self.take_pending();
                }
            }
        }
    }

    /// Hands the pending room to the other network if it is silent: it takes the room's
    /// lengths and decay at once and the input starts crossfading to it.
    fn take_pending(&mut self) {
        let other = 1 - self.fed;
        if self.nets[other].active {
            return;
        }
        let Some(room) = self.pending.take() else {
            return;
        };
        let settings = self.settings;
        let net = &mut self.nets[other];
        net.clear();
        // Its input weight starts at 0, so its input gain starts at its target.
        net.set_at_once(room, &settings, 0.0);
        net.b = net.input_gain(net.t60, &settings, &self.band_cos);
        self.fed = other;
        self.xfade = 0.0;
    }

    /// Moves the fed network's line `i` current length by `frames` at once, within the ring.
    pub fn jump(&mut self, i: usize, frames: f32) {
        let net = &mut self.nets[self.fed];
        net.length[i] = (net.length[i] + frames).clamp(1.0, REVERB_MAX_DELAY_FRAMES as f32);
    }

    /// The fed network's current and target length per line.
    pub fn lines(&self) -> [ReverbLine; REVERB_LINES] {
        let net = &self.nets[self.fed];
        std::array::from_fn(|i| ReverbLine {
            length: net.length[i],
            target: net.target[i],
        })
    }

    /// Both networks, for tests.
    pub fn networks(&self) -> [ReverbNetwork; 2] {
        let weights = self.weights(self.xfade);
        std::array::from_fn(|k| {
            let net = &self.nets[k];
            ReverbNetwork {
                lines: std::array::from_fn(|i| ReverbLine {
                    length: net.length[i],
                    target: net.target[i],
                }),
                holds_signal: net.holds_signal(),
                fed: k == self.fed,
                input: weights[k],
            }
        })
    }

    /// Each network's input weight at crossfade position `x`: equal power, `sin` for the fed
    /// network and `cos` for the other.
    fn weights(&self, x: f32) -> [f32; 2] {
        let (fed, other) = if x >= 1.0 {
            (1.0, 0.0)
        } else if x <= 0.0 {
            (0.0, 1.0)
        } else {
            (std::f32::consts::FRAC_PI_2 * x).sin_cos()
        };
        let mut w = [0.0; 2];
        w[self.fed] = fed;
        w[1 - self.fed] = other;
        w
    }

    /// Runs one block: `input` (the voices' summed sends, `n` frames) in, both ears of both
    /// networks added to interleaved `out`.
    pub fn render(&mut self, input: &[f32], out: &mut [f32]) {
        self.take_pending();
        let n = input.len();
        let x0 = self.xfade;
        let x1 = (x0 + n as f32 / (SMOOTHING_SECONDS * RATE as f32)).min(1.0);
        self.xfade = x1;
        let (w0, w1) = (self.weights(x0), self.weights(x1));
        let input = self.pre.process(input);
        for (k, net) in self.nets.iter_mut().enumerate() {
            net.render(input, out, (w0[k], w1[k]), &self.settings, &self.band_cos);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primes_and_lengths() {
        assert_eq!(nearest_prime(1120), 1117);
        assert_eq!(nearest_prime(4800), 4799);
        assert_eq!(nearest_prime(32), 31);
        assert!(is_prime(target_length(0, 8.0)));
        // 8 m: 1119.5 frames of travel, half of it for line 0, one and a half for line 15.
        assert_eq!(target_length(0, 8.0), nearest_prime(560));
        assert_eq!(target_length(15, 8.0), nearest_prime(1679));
    }

    #[test]
    fn the_line_filter_meets_its_two_gains() {
        let (m, t60, ratio) = (1117.0f32, 2.26f32, 0.5f32);
        let (g, a) = line_filter(m, t60, ratio);
        let fs = RATE as f32;
        assert!((g - 10f32.powf(-3.0 * m / (t60 * fs))).abs() < 1e-6);
        let w = std::f32::consts::TAU * HF_HZ / fs;
        let mag = g * (1.0 - a) / (1.0 - 2.0 * a * w.cos() + a * a).sqrt();
        let want = 10f32.powf(-3.0 * m / (t60 * ratio * fs));
        assert!(
            (mag - want).abs() < 1e-4 * want.max(1e-3),
            "{mag} vs {want}"
        );
    }
}
