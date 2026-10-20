//! The late tail: one feedback delay network shared by every voice (ADR-0015).
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
//! the nearest prime, so the lines share no common period. The current length glides toward it
//! by at most [`REVERB_DELAY_SLEW_FRAMES_PER_FRAME`] per frame through a linearly interpolated
//! fractional read, and the gains follow the current lengths and a smoothed `T60` every block,
//! so a change of room is a glide, never a swap.
//!
//! Every line takes the same input `b * x`, `x` the sum of the voices' sends. At frequency `w`
//! a pass through line `i` keeps `|H_i(w)|^2` of the energy and the Householder matrix
//! preserves it, so with `G(w)` the mean of `|H_i(w)|^2` over the lines, a unit impulse leaves
//! `N b^2 G (1 + G + G^2 + ...) = N b^2 G / (1 - G)` in the filtered line outputs at `w`. Each
//! ear takes half the lines (disjoint halves, alternating signs, so the ears decorrelate), so
//! `b^2 = 2 E / (N mean_w [G / (1 - G)])`, the mean over [`ENERGY_BANDS`] frequencies spread
//! evenly to Nyquist, gives each ear the tail energy `E` the room asks for.

use crate::command::RoomTargets;
use crate::{BLOCK, RATE, SMOOTHING_SECONDS, SPEED_OF_SOUND};

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
}

impl Default for ReverbSettings {
    fn default() -> Self {
        Self {
            hf_ratio: 0.5,
            level: 1.0,
            max_t60_s: 8.0,
        }
    }
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

/// The network and its state, allocated once.
pub(crate) struct Reverb {
    lines: [Vec<f32>; REVERB_LINES],
    /// Where the next frame is written, the same in every ring.
    write: usize,
    length: [f32; REVERB_LINES],
    target: [u32; REVERB_LINES],
    /// Each line's low-pass state.
    lowpass: [f32; REVERB_LINES],
    /// Each line's filter at the end of the last block, `(g (1 - a), a)`, which this block's
    /// coefficients ramp from, sample by sample, so a change of decay or length never steps
    /// the loop gain at a block edge.
    coef: [(f32, f32); REVERB_LINES],
    /// `T60` and input gain `b` as smoothed so far, and their targets.
    t60: f32,
    t60_target: f32,
    b: f32,
    b_target: f32,
    energy: f32,
    settings: ReverbSettings,
    /// `cos w` at the centre of each of the [`ENERGY_BANDS`] bands.
    band_cos: [f32; ENERGY_BANDS],
    /// A room has arrived; until then nothing is set and the network is silent.
    started: bool,
    /// The network may hold signal; cleared once it has rung out with no input.
    active: bool,
}

impl Reverb {
    pub fn new() -> Self {
        Self {
            lines: std::array::from_fn(|_| vec![0.0; RING]),
            write: 0,
            length: [REVERB_MIN_DELAY_FRAMES as f32; REVERB_LINES],
            target: [REVERB_MIN_DELAY_FRAMES as u32; REVERB_LINES],
            lowpass: [0.0; REVERB_LINES],
            coef: [(0.0, 0.0); REVERB_LINES],
            t60: 1.0,
            t60_target: 1.0,
            b: 0.0,
            b_target: 0.0,
            energy: 0.0,
            settings: ReverbSettings::default(),
            band_cos: std::array::from_fn(|k| {
                (std::f32::consts::PI * (k as f32 + 0.5) / ENERGY_BANDS as f32).cos()
            }),
            started: false,
            active: false,
        }
    }

    /// Bytes the lines hold.
    pub fn line_bytes() -> usize {
        REVERB_LINES * RING * 4
    }

    pub fn settings(&self) -> ReverbSettings {
        self.settings
    }

    pub fn set_settings(&mut self, settings: ReverbSettings) {
        self.settings = settings;
        self.t60_target = self.t60_target.min(settings.max_t60_s);
    }

    /// Takes a room. The first one sets the lengths and the decay at once, and the input gain
    /// rises from 0 through the smoother, so the tail fades in rather than starting on a step;
    /// later rooms glide.
    pub fn set_room(&mut self, room: RoomTargets) {
        for (i, t) in self.target.iter_mut().enumerate() {
            *t = target_length(i, room.mean_free_path_m.max(0.0));
        }
        self.t60_target = room.t60_s.clamp(0.05, self.settings.max_t60_s);
        self.energy = room.energy.max(0.0);
        if !self.started {
            self.started = true;
            self.length = self.target.map(|t| t as f32);
            self.t60 = self.t60_target;
            for (c, &m) in self.coef.iter_mut().zip(&self.length) {
                let (g, a) = line_filter(m, self.t60, self.settings.hf_ratio);
                *c = (g * (1.0 - a), a);
            }
        }
        if self.energy > 0.0 {
            self.active = true;
        }
    }

    /// Each line's current and target length.
    pub fn lines(&self) -> [ReverbLine; REVERB_LINES] {
        std::array::from_fn(|i| ReverbLine {
            length: self.length[i],
            target: self.target[i],
        })
    }

    /// `b` for the room's energy at decay `t60` and the current lengths:
    /// `b^2 = 2 E level^2 / (N mean_w [G / (1 - G)])` (module comment).
    fn input_gain(&self, t60: f32) -> f32 {
        let e = self.energy * self.settings.level * self.settings.level;
        if e <= 0.0 {
            return 0.0;
        }
        let filters = self
            .length
            .map(|m| line_filter(m, t60, self.settings.hf_ratio));
        let mut sum = 0.0f32;
        for &c in &self.band_cos {
            let g2 = filters
                .iter()
                .map(|&(g, a)| g * g * (1.0 - a) * (1.0 - a) / (1.0 - 2.0 * a * c + a * a))
                .sum::<f32>()
                / REVERB_LINES as f32;
            sum += g2 / (1.0 - g2).max(1e-6);
        }
        let mean = sum / ENERGY_BANDS as f32;
        (2.0 * e / (REVERB_LINES as f32 * mean.max(1e-9))).sqrt()
    }

    /// Runs one block: `input` (the voices' summed sends, `n` frames) in, both ears added to
    /// interleaved `out`.
    pub fn render(&mut self, input: &[f32], out: &mut [f32]) {
        if !self.active {
            return;
        }
        let n = input.len();
        let k_block = 1.0 - (-(n as f32) / (SMOOTHING_SECONDS * RATE as f32)).exp();
        self.t60 += (self.t60_target - self.t60) * k_block;
        self.b_target = self.input_gain(self.t60);
        let b0 = self.b;
        self.b += (self.b_target - self.b) * k_block;
        let b1 = self.b;
        let reach = REVERB_DELAY_SLEW_FRAMES_PER_FRAME * n as f32;
        let mut l0 = [0.0f32; REVERB_LINES];
        let mut dl = [0.0f32; REVERB_LINES];
        let c0 = self.coef;
        let mut dc = [(0.0f32, 0.0f32); REVERB_LINES];
        for i in 0..REVERB_LINES {
            let m0 = self.length[i];
            let m1 = m0 + (self.target[i] as f32 - m0).clamp(-reach, reach);
            let (g, a) = line_filter(m1, self.t60, self.settings.hf_ratio);
            self.coef[i] = (g * (1.0 - a), a);
            dc[i] = (
                (self.coef[i].0 - c0[i].0) / n as f32,
                (self.coef[i].1 - c0[i].1) / n as f32,
            );
            l0[i] = m0;
            dl[i] = (m1 - m0) / n as f32;
            self.length[i] = m1;
        }
        let inv_n = 1.0 / n as f32;
        let mut peak = 0.0f32;
        for (j, (&x, f)) in input.iter().zip(out.chunks_exact_mut(2)).enumerate() {
            let bx = (b0 + (b1 - b0) * (j + 1) as f32 * inv_n) * x;
            let mut y = [0.0f32; REVERB_LINES];
            let mut sum = 0.0f32;
            for i in 0..REVERB_LINES {
                let m = l0[i] + dl[i] * (j + 1) as f32;
                // Read `m` frames behind the write position; `m` is at least 32, so the frame
                // read was written in an earlier frame.
                let pos = (self.write + RING) as f32 - m;
                let base = pos.floor();
                let frac = pos - base;
                let i0 = base as usize % RING;
                let i1 = (i0 + 1) % RING;
                let line = &self.lines[i];
                let raw = line[i0] + (line[i1] - line[i0]) * frac;
                let t = (j + 1) as f32;
                let (gain, pole) = (c0[i].0 + dc[i].0 * t, c0[i].1 + dc[i].1 * t);
                let lp = gain * raw + pole * self.lowpass[i];
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
            f[0] += left;
            f[1] += right;
            peak = peak.max(left.abs()).max(right.abs()).max(bx.abs());
            let feedback = 2.0 * sum / REVERB_LINES as f32;
            for (line, &yi) in self.lines.iter_mut().zip(&y) {
                line[self.write] = yi - feedback + bx;
            }
            self.write = (self.write + 1) % RING;
        }
        // Rung out with nothing coming in and nothing asked for: clear and stop.
        if self.energy == 0.0 && b1 < 1e-9 && peak < 1e-9 {
            for line in &mut self.lines {
                line.fill(0.0);
            }
            self.lowpass = [0.0; REVERB_LINES];
            self.active = false;
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
