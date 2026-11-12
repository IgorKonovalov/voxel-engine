//! One feedback delay network: its line lengths (primes over the mean free path), its loop
//! filters, its input gain for the room's energy, its block render, and the spare line sets a
//! cleared network swaps with.

use super::{
    CLEAR_FRAMES_PER_BLOCK, HF_HZ, REVERB_DELAY_SLEW_FRAMES_PER_FRAME, REVERB_LINES,
    REVERB_MAX_DELAY_FRAMES, REVERB_MIN_DELAY_FRAMES, RING, ReverbClears, ReverbSettings,
};
use crate::command::RoomTargets;
use crate::{RATE, SMOOTHING_SECONDS, SPEED_OF_SOUND};

/// `spread_i = 0.5 * 3 ^ (i / 15)`.
fn spread(i: usize) -> f32 {
    0.5 * 3f32.powf(i as f32 / (REVERB_LINES - 1) as f32)
}

/// [`PRIMES`] holds every prime below this: past [`REVERB_MAX_DELAY_FRAMES`] by more than the
/// gap after the last prime under it, so [`nearest_prime`] of any length up to the cap finds
/// the prime above it too.
const PRIME_LIMIT: usize = REVERB_MAX_DELAY_FRAMES + 64;

/// Sieve of Eratosthenes: `true` at every composite below [`PRIME_LIMIT`], and at 0 and 1.
const fn composites() -> [bool; PRIME_LIMIT] {
    let mut c = [false; PRIME_LIMIT];
    c[0] = true;
    c[1] = true;
    let mut i = 2;
    while i * i < PRIME_LIMIT {
        if !c[i] {
            let mut j = i * i;
            while j < PRIME_LIMIT {
                c[j] = true;
                j += i;
            }
        }
        i += 1;
    }
    c
}

const fn prime_count() -> usize {
    let c = composites();
    let mut n = 0;
    let mut i = 0;
    while i < PRIME_LIMIT {
        if !c[i] {
            n += 1;
        }
        i += 1;
    }
    n
}

/// Every prime below [`PRIME_LIMIT`], ascending, built at compile time.
static PRIMES: [u32; prime_count()] = {
    let c = composites();
    let mut p = [0u32; prime_count()];
    let (mut i, mut k) = (0, 0);
    while i < PRIME_LIMIT {
        if !c[i] {
            p[k] = i as u32;
            k += 1;
        }
        i += 1;
    }
    p
};

/// The prime nearest to `n`, the lower one on a tie. `n` is at most
/// [`REVERB_MAX_DELAY_FRAMES`], so the prime above it is in [`PRIMES`].
fn nearest_prime(n: u32) -> u32 {
    let i = PRIMES.partition_point(|&p| p < n);
    match (i.checked_sub(1).map(|j| PRIMES[j]), PRIMES.get(i)) {
        (_, Some(&above)) if above == n => n,
        (Some(below), Some(&above)) => {
            if n - below <= above - n {
                below
            } else {
                above
            }
        }
        (Some(below), None) => below,
        (None, Some(&above)) => above,
        (None, None) => 2,
    }
}

/// Line `i`'s target length for mean free path `mfp` metres.
fn target_length(i: usize, mfp: f32) -> u32 {
    let frames = (mfp / SPEED_OF_SOUND * RATE as f32 * spread(i)).clamp(
        REVERB_MIN_DELAY_FRAMES as f32,
        REVERB_MAX_DELAY_FRAMES as f32,
    );
    let p = nearest_prime(frames.round() as u32);
    if p <= REVERB_MAX_DELAY_FRAMES as u32 {
        p
    } else {
        largest_prime_at_most(REVERB_MAX_DELAY_FRAMES as u32)
    }
}

/// The largest prime at or below `n` (`2 <= n < PRIME_LIMIT`).
fn largest_prime_at_most(n: u32) -> u32 {
    let i = PRIMES.partition_point(|&p| p <= n);
    i.checked_sub(1).map_or(2, |j| PRIMES[j])
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
/// network rings out to exact zeros. Portable: no flush-to-zero mode of the FPU is set. The
/// mixer's voice low-pass states flush by the same rule.
pub(crate) const FLUSH_BELOW: f32 = 1e-20;

/// `i` in `[0, 2 RING)` brought into the ring, without a division.
#[inline]
fn wrap(i: usize) -> usize {
    if i >= RING { i - RING } else { i }
}

/// `x`, or 0 when `|x| < FLUSH_BELOW`. A select, not a branch, in the compiled loop.
#[inline]
pub(crate) fn flush(x: f32) -> f32 {
    if x.abs() < FLUSH_BELOW { 0.0 } else { x }
}

/// One network and its state, allocated once.
pub(super) struct Network {
    pub(super) lines: [Vec<f32>; REVERB_LINES],
    /// Where the next frame is written, the same in every ring.
    pub(super) write: usize,
    pub(super) length: [f32; REVERB_LINES],
    pub(super) target: [u32; REVERB_LINES],
    /// The mean free path the targets were set for.
    pub(super) mfp: f32,
    /// The mean free path of the room taken last, as it arrived.
    pub(super) room_mfp: f32,
    /// `level_by_room` at `room_mfp`, as an amplitude factor.
    pub(super) room_level: f32,
    /// Each line's low-pass state.
    pub(super) lowpass: [f32; REVERB_LINES],
    /// Each line's filter at the end of the last block, `(g (1 - a), a)`, which this block's
    /// coefficients ramp from, sample by sample, so a change of decay or length never steps
    /// the loop gain at a block edge.
    pub(super) coef: [(f32, f32); REVERB_LINES],
    /// `T60` and input gain `b` as smoothed so far, and the `T60` target.
    pub(super) t60: f32,
    pub(super) t60_target: f32,
    pub(super) b: f32,
    pub(super) energy: f32,
    /// Frames the decay holds at `t60` before it moves toward `t60_target` (a longer decay
    /// with less energy waits for the stored tail to fall to the new level).
    pub(super) hold: f32,
    /// Lengths and decay have been set.
    pub(super) started: bool,
    /// The network may hold signal; cleared once it has rung out with no input.
    pub(super) active: bool,
    /// Output gain while fading out before a clear: `Some(g)`, `g` falling to 0.
    pub(super) fade: Option<f32>,
    /// Frames written since the network was made, of which `write` is the ring position.
    pub(super) written: u64,
    /// Since the lines were last all zero, the first frame of the first block that wrote a
    /// nonzero sample and the end of the last one: every nonzero sample lies in that run of
    /// frames (its last [`RING`] of them).
    pub(super) nonzero: Option<(u64, u64)>,
    /// The decay, the HF ratio and each line's length `coef` was computed for, as bits; `None`
    /// until a block computes them.
    pub(super) coef_for: Option<(u32, u32, [u32; REVERB_LINES])>,
    /// [`Network::input_gain`]'s inputs as bits (the decay, the HF ratio, the level trim, the
    /// room level, the energy, each line's length) and its value for them.
    pub(super) gain_for: Option<([u32; 5 + REVERB_LINES], f32)>,
}

impl Network {
    pub(super) fn new() -> Self {
        Self {
            lines: std::array::from_fn(|_| vec![0.0; RING]),
            write: 0,
            length: [REVERB_MIN_DELAY_FRAMES as f32; REVERB_LINES],
            target: [REVERB_MIN_DELAY_FRAMES as u32; REVERB_LINES],
            mfp: 0.0,
            room_mfp: f32::NAN,
            room_level: 1.0,
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
            written: 0,
            nonzero: None,
            coef_for: None,
            gain_for: None,
        }
    }

    /// Silences every line and filter: lines that may hold signal swap with `spare`'s clean
    /// ones, or, while the spare is dirty, wait parked (module comment, "Clearing").
    pub(super) fn clear(&mut self, spare: &mut SpareLines) {
        if let Some((first, end)) = self.nonzero {
            if !spare.has_clean() {
                if !self.parked() {
                    spare.stats.deferred += 1;
                }
            } else {
                self.nonzero = None;
                let len = (end - first).min(RING as u64) as usize;
                spare.take(&mut self.lines, (end % RING as u64) as usize, len);
            }
        }
        self.lowpass = [0.0; REVERB_LINES];
        self.active = false;
        self.fade = None;
    }

    /// Takes `room`'s lengths and decay at once, the input gain `b` from `b`.
    pub(super) fn set_at_once(&mut self, room: RoomTargets, settings: &ReverbSettings, b: f32) {
        let mfp = room.mean_free_path_m.max(0.0);
        for (i, t) in self.target.iter_mut().enumerate() {
            *t = target_length(i, mfp);
        }
        self.mfp = mfp;
        self.take_room_level(room.mean_free_path_m, settings);
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
        self.coef_for = Some((
            self.t60.to_bits(),
            settings.hf_ratio.to_bits(),
            self.length.map(f32::to_bits),
        ));
        self.started = true;
        self.active |= self.energy > 0.0;
    }

    /// Takes `room`'s decay and energy in place; `glide` also retargets the lengths.
    pub(super) fn set_in_place(
        &mut self,
        room: RoomTargets,
        settings: &ReverbSettings,
        glide: bool,
    ) {
        if glide {
            let mfp = room.mean_free_path_m.max(0.0);
            for (i, t) in self.target.iter_mut().enumerate() {
                *t = target_length(i, mfp);
            }
            self.mfp = mfp;
        }
        self.take_room_level(room.mean_free_path_m, settings);
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

    /// Records the room's mean free path and the tail curve's level there.
    pub(super) fn take_room_level(&mut self, mfp: f32, settings: &ReverbSettings) {
        self.room_mfp = mfp;
        self.room_level = settings.level_by_room.gain_at(mfp);
    }

    /// Cleared while the spare was dirty: silent, its lines not yet swapped.
    pub(super) fn parked(&self) -> bool {
        !self.active && self.nonzero.is_some()
    }

    /// Whether any line or filter state is nonzero.
    pub(super) fn holds_signal(&self) -> bool {
        self.lowpass.iter().any(|&x| x != 0.0)
            || self.lines.iter().any(|l| l.iter().any(|&x| x != 0.0))
    }

    /// `b` for the network's energy at decay `t60` and its current lengths:
    /// `b^2 = 2 E level^2 / (N mean_w [G / (1 - G)])` (module comment), `level` the trim
    /// times the room curve's factor at the network's room.
    pub(super) fn input_gain(&self, t60: f32, settings: &ReverbSettings, band_cos: &[f32]) -> f32 {
        let level = settings.level * self.room_level;
        let e = self.energy * level * level;
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

    /// [`Network::input_gain`], reused while its inputs hold their values bit for bit.
    pub(super) fn input_gain_cached(
        &mut self,
        t60: f32,
        settings: &ReverbSettings,
        band_cos: &[f32],
    ) -> f32 {
        let mut key = [0u32; 5 + REVERB_LINES];
        key[..5].copy_from_slice(&[
            t60.to_bits(),
            settings.hf_ratio.to_bits(),
            settings.level.to_bits(),
            self.room_level.to_bits(),
            self.energy.to_bits(),
        ]);
        for (k, m) in key[5..].iter_mut().zip(&self.length) {
            *k = m.to_bits();
        }
        if let Some((k, b)) = self.gain_for
            && k == key
        {
            return b;
        }
        let b = self.input_gain(t60, settings, band_cos);
        self.gain_for = Some((key, b));
        b
    }

    /// Runs one block: `input` (the voices' summed sends, `n` frames) times an input weight
    /// moving linearly from `w0` to `w1` across the block, both ears added to interleaved
    /// `out` times the fade's gain. Returns whether the network has faded out or rung out
    /// with nothing coming in, and so is to be cleared.
    pub(super) fn render(
        &mut self,
        input: &[f32],
        out: &mut [f32],
        (w0, w1): (f32, f32),
        settings: &ReverbSettings,
        band_cos: &[f32],
    ) -> bool {
        if !self.active {
            return false;
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
        let b_target = self.input_gain_cached(self.t60.max(self.t60_target), settings, band_cos);
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
        // A line whose length, decay and HF ratio are those of the last block keeps its filter.
        let (t60_bits, hf_bits) = (self.t60.to_bits(), settings.hf_ratio.to_bits());
        let mut coef_for = match self.coef_for {
            Some((t, h, m)) if t == t60_bits && h == hf_bits => m,
            // No length is NaN, so every line recomputes.
            _ => [f32::NAN.to_bits(); REVERB_LINES],
        };
        for i in 0..REVERB_LINES {
            let m0 = self.length[i];
            let m1 = m0 + (self.target[i] as f32 - m0).clamp(-reach, reach);
            if coef_for[i] != m1.to_bits() {
                let (g, a) = line_filter(m1, self.t60, settings.hf_ratio);
                self.coef[i] = (g * (1.0 - a), a);
                coef_for[i] = m1.to_bits();
            }
            dc[i] = (
                (self.coef[i].0 - c0[i].0) * inv_n,
                (self.coef[i].1 - c0[i].1) * inv_n,
            );
            l0[i] = m0;
            dl[i] = (m1 - m0) * inv_n;
            self.length[i] = m1;
        }
        self.coef_for = Some((t60_bits, hf_bits, coef_for));
        // A line that holds a whole length over the block reads one ring sample per frame.
        let whole: [Option<usize>; REVERB_LINES] = std::array::from_fn(|i| {
            (dl[i] == 0.0 && l0[i].fract() == 0.0 && l0[i] >= 1.0).then_some(l0[i] as usize)
        });
        let (bw0, bw1) = (b0 * w0, b1 * w1);
        let mut peak = 0.0f32;
        let mut wrote_nonzero = false;
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
                let w = flush(yi - feedback + bx);
                line[self.write] = w;
                wrote_nonzero |= w != 0.0;
            }
            self.write = wrap(self.write + 1);
        }
        let start = self.written;
        self.written += n as u64;
        if wrote_nonzero {
            let first = self.nonzero.map_or(start, |(first, _)| first);
            self.nonzero = Some((first, self.written));
        }
        // Faded out, or rung out with nothing coming in: clear and stop.
        self.fade == Some(0.0) || (bw1 < 1e-9 && peak < 1e-9)
    }
}

/// Spare line sets a cleared network swaps with (module comment, "Clearing").
pub(crate) const SPARE_SETS: usize = 1;

/// One spare set: all zero except the run of `len` ring frames ending before index `end`.
struct SpareSet {
    lines: [Vec<f32>; REVERB_LINES],
    end: usize,
    len: usize,
}

impl SpareSet {
    /// Zeroes the oldest `frames` frames of the dirty run, at most all of it, in every line;
    /// returns how many frames it zeroed.
    fn zero(&mut self, frames: usize) -> usize {
        let k = frames.min(self.len);
        if k == 0 {
            return 0;
        }
        let start = (self.end + RING - self.len) % RING;
        let first = (RING - start).min(k);
        for line in &mut self.lines {
            line[start..start + first].fill(0.0);
            line[..k - first].fill(0.0);
        }
        self.len -= k;
        k
    }
}

/// The [`SPARE_SETS`] spare sets and how their clears ran.
pub(super) struct SpareLines {
    sets: [SpareSet; SPARE_SETS],
    pub(super) stats: ReverbClears,
    /// Samples zeroed in the block running now.
    zeroed: usize,
}

impl SpareLines {
    pub(super) fn new() -> Self {
        Self {
            sets: std::array::from_fn(|_| SpareSet {
                lines: std::array::from_fn(|_| vec![0.0; RING]),
                end: 0,
                len: 0,
            }),
            stats: ReverbClears::default(),
            zeroed: 0,
        }
    }

    /// Whether a set is clean, so a clear can swap at once.
    pub(super) fn has_clean(&self) -> bool {
        self.sets.iter().any(|s| s.len == 0)
    }

    /// Swaps `lines`, nonzero only over the `dirty` frames ending before ring index `end`,
    /// with a clean set's lines. With no set clean (a caller that did not check
    /// [`SpareLines::has_clean`]), the first set finishes zeroing itself first.
    fn take(&mut self, lines: &mut [Vec<f32>; REVERB_LINES], end: usize, dirty: usize) {
        let i = match self.sets.iter().position(|s| s.len == 0) {
            Some(i) => i,
            None => {
                self.stats.forced += 1;
                let k = self.sets[0].zero(usize::MAX);
                self.zeroed += k * REVERB_LINES;
                0
            }
        };
        let set = &mut self.sets[i];
        std::mem::swap(&mut set.lines, lines);
        set.end = end;
        set.len = dirty;
        self.stats.swaps += 1;
    }

    /// Ends a block: zeroes up to [`CLEAR_FRAMES_PER_BLOCK`] more frames of each line in all,
    /// the set with the shortest dirty run first so it comes clean soonest, and records the
    /// block's total.
    pub(super) fn end_block(&mut self) {
        let mut budget = CLEAR_FRAMES_PER_BLOCK;
        while budget > 0 {
            let Some(set) = self
                .sets
                .iter_mut()
                .filter(|s| s.len > 0)
                .min_by_key(|s| s.len)
            else {
                break;
            };
            let k = set.zero(budget);
            budget -= k;
            self.zeroed += k * REVERB_LINES;
        }
        self.stats.max_zeroed = self.stats.max_zeroed.max(self.zeroed);
        self.zeroed = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_prime(n: u32) -> bool {
        n >= 2
            && (2..)
                .take_while(|d| d * d <= n)
                .all(|d| !n.is_multiple_of(d))
    }

    /// The trial-division search the table replaces.
    fn nearest_prime_by_trial(n: u32) -> u32 {
        (0..)
            .find_map(|k| {
                [n.saturating_sub(k), n + k]
                    .into_iter()
                    .find(|&c| is_prime(c))
            })
            .unwrap_or(2)
    }

    #[test]
    fn the_prime_table_matches_trial_division_up_to_the_cap() {
        assert!(PRIMES.iter().all(|&p| is_prime(p)));
        assert_eq!(
            PRIMES.len(),
            (0..PRIME_LIMIT as u32).filter(|&n| is_prime(n)).count()
        );
        for n in 0..=REVERB_MAX_DELAY_FRAMES as u32 {
            assert_eq!(nearest_prime(n), nearest_prime_by_trial(n), "n = {n}");
        }
        for n in 2..=REVERB_MAX_DELAY_FRAMES as u32 {
            let by_trial = (2..=n).rev().find(|&c| is_prime(c)).unwrap();
            assert_eq!(largest_prime_at_most(n), by_trial, "n = {n}");
        }
    }

    #[test]
    fn primes_and_lengths() {
        assert_eq!(nearest_prime(1120), 1117);
        assert_eq!(nearest_prime(4800), 4799);
        assert_eq!(nearest_prime(28_000), 28_001);
        assert_eq!(largest_prime_at_most(28_000), 27_997);
        assert_eq!(nearest_prime(32), 31);
        assert!(is_prime(target_length(0, 8.0)));
        // 8 m: 1119.5 frames of travel, half of it for line 0, one and a half for line 15.
        assert_eq!(target_length(0, 8.0), nearest_prime(560));
        assert_eq!(target_length(15, 8.0), nearest_prime(1679));
    }

    /// The cap covers line 15 at the largest mean free path a fan can report, `(4/3)` of the
    /// ray range, so a longer range fails here instead of collapsing the tail.
    #[test]
    fn the_cap_covers_the_fans_bound() {
        let bound = 1.5 * (4.0 / 3.0) * f64::from(crate::REFLECTION_RANGE_METRES)
            / f64::from(SPEED_OF_SOUND)
            * f64::from(RATE);
        assert!(
            REVERB_MAX_DELAY_FRAMES as f64 >= bound,
            "{REVERB_MAX_DELAY_FRAMES} < {bound}"
        );
        let at_bound: Vec<u32> = (0..REVERB_LINES)
            .map(|i| target_length(i, 133.334))
            .collect();
        println!("lines at mfp 133.334 m: {at_bound:?}");
        assert!(at_bound.iter().all(|&m| is_prime(m)));
        assert!(at_bound.windows(2).all(|w| w[0] < w[1]), "{at_bound:?}");
        assert!(at_bound.iter().all(|&m| m < REVERB_MAX_DELAY_FRAMES as u32));
        assert_eq!(at_bound[15], 27_983);
        assert_eq!(target_length(15, 1000.0), 27_997);
    }

    /// Rooms whose lines never reached the old 4 800-frame cap keep their lengths; the 36 m
    /// room's line 15 now takes its own length instead of the cap.
    #[test]
    fn small_rooms_keep_their_lengths() {
        let lines = |mfp: f32| {
            (0..REVERB_LINES)
                .map(|i| target_length(i, mfp))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            lines(2.666),
            [
                191, 199, 211, 233, 251, 269, 293, 311, 337, 359, 389, 419, 449, 479, 521, 557
            ]
        );
        assert_eq!(
            lines(7.999),
            [
                557, 601, 647, 701, 751, 809, 863, 937, 1009, 1087, 1163, 1249, 1361, 1451, 1559,
                1669
            ]
        );
        let big = lines(23.998);
        println!("lines at mfp 23.998 m: {big:?}");
        assert!(big[..15].iter().all(|&m| m < 4_800));
        assert_eq!(big[15], 5_039);
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
