//! Spatialisation: the HRTF's FIR pair with a crossfade between measurements, the
//! constant-power pan, and the listener's frame.

use std::sync::Arc;

use crate::hrtf::Hrtf;
use crate::{BLOCK, Listener, cross, dot, normalize_or};

/// One source as the [`Spatialiser`] places it, borrowed from a voice or a bus.
pub(super) struct Placement<'a> {
    /// HRTF path: the last `taps - 1` inputs, oldest first.
    pub history: &'a mut [f32],
    /// HRTF path: the measurement the last block was filtered with.
    pub dir: &'a mut usize,
    /// Panning path: left and right gain as smoothed so far.
    pub pan: &'a mut [f32; 2],
    /// Whether the source has rendered; a fresh one starts at its direction instead of fading
    /// to it.
    pub started: bool,
}

/// Places voices and buses alike in the listener's frame: through the two FIRs of the
/// measurement nearest to a direction, crossfading when that measurement changes, or, without
/// an HRTF, by a constant-power pan. Holds the scratch the FIRs run in, sized once.
pub(super) struct Spatialiser {
    hrtf: Option<Arc<Hrtf>>,
    /// The FIR input of one source: its history, then this block.
    input: Vec<f32>,
    /// The old and new FIR outputs of one source for a crossfade, per ear.
    wet: [[Vec<f32>; 2]; 2],
}

impl Spatialiser {
    pub fn new(hrtf: Option<Arc<Hrtf>>) -> Self {
        let taps = hrtf.as_ref().map_or(1, |h| h.taps());
        Self {
            hrtf,
            input: vec![0.0; taps - 1 + BLOCK],
            wet: std::array::from_fn(|_| std::array::from_fn(|_| vec![0.0; BLOCK])),
        }
    }

    pub fn hrtf(&self) -> Option<&Arc<Hrtf>> {
        self.hrtf.as_ref()
    }

    /// Frames a FIR rings for after its input stops: `taps - 1`, 0 when panning.
    pub fn ring_frames(&self) -> usize {
        self.hrtf.as_ref().map_or(0, |h| h.taps() - 1)
    }

    /// Adds `block`, heard from unit direction `dir` in the listener's frame at `d` metres, to
    /// interleaved `out`, moving `at` on. On the HRTF path a changed nearest measurement
    /// crossfades ([`spatialise`]); on the panning path the gains of [`pan_gains`] move through
    /// the one-pole of step `k`.
    pub fn render(
        &mut self,
        at: Placement<'_>,
        dir: [f32; 3],
        d: f32,
        k: f32,
        block: &[f32],
        out: &mut [f32],
    ) {
        match &self.hrtf {
            Some(hrtf) => {
                let to = hrtf.nearest(dir);
                if !at.started {
                    *at.dir = to;
                }
                let from = *at.dir;
                *at.dir = to;
                let scratch = (&mut self.input, &mut self.wet);
                spatialise(hrtf, at.history, block, from, to, scratch, out);
            }
            None => {
                let target = pan_gains(dir, d);
                if !at.started {
                    *at.pan = target;
                }
                pan(at.pan, target, k, block, out);
            }
        }
    }
}

/// Filters `block` through measurement `to`'s two FIRs, continuing from `history` (the last
/// `taps - 1` inputs, updated here), and adds the result to interleaved `out`. When `from`
/// differs from `to` the block is filtered with both and crossfaded from old to new along
/// [`crossfade_weight`]. `scratch` is the mixer's FIR input buffer and its two pairs of ear
/// outputs.
fn spatialise(
    hrtf: &Hrtf,
    history: &mut [f32],
    block: &[f32],
    from: usize,
    to: usize,
    scratch: (&mut Vec<f32>, &mut [[Vec<f32>; 2]; 2]),
    out: &mut [f32],
) {
    let n = block.len();
    let taps = hrtf.taps();
    let (input, wet) = scratch;
    let input = &mut input[..taps - 1 + n];
    input[..taps - 1].copy_from_slice(history);
    input[taps - 1..].copy_from_slice(block);
    let [new, old] = wet;
    for (ear, y) in new.iter_mut().enumerate() {
        convolve(input, hrtf.reversed(to, ear), &mut y[..n]);
    }
    if from == to {
        for (i, f) in out.chunks_exact_mut(2).enumerate() {
            f[0] += new[0][i];
            f[1] += new[1][i];
        }
    } else {
        for (ear, y) in old.iter_mut().enumerate() {
            convolve(input, hrtf.reversed(from, ear), &mut y[..n]);
        }
        for (i, f) in out.chunks_exact_mut(2).enumerate() {
            let w = crossfade_weight(i, n);
            f[0] += old[0][i] + (new[0][i] - old[0][i]) * w;
            f[1] += old[1][i] + (new[1][i] - old[1][i]) * w;
        }
    }
    history.copy_from_slice(&input[n..]);
}

/// Adds `block` to interleaved `out` through left and right gains `gains`, which move toward
/// `target` through the one-pole of step `k` every sample.
fn pan(gains: &mut [f32; 2], target: [f32; 2], k: f32, block: &[f32], out: &mut [f32]) {
    let [mut gl, mut gr] = *gains;
    for (f, &s) in out.chunks_exact_mut(2).zip(block) {
        gl += (target[0] - gl) * k;
        gr += (target[1] - gr) * k;
        f[0] += s * gl;
        f[1] += s * gr;
    }
    *gains = [gl, gr];
}

/// Weight of the new filter at frame `i` of an `n`-frame crossfade: a raised cosine from
/// `0.5 - 0.5 cos(pi / (n + 1))` up to `0.5 + 0.5 cos(pi / (n + 1))`, symmetric about the
/// block's middle. Its slope is near zero at both ends, so a crossfade that follows another
/// in the next block (a source whose nearest direction changes every block) joins it without
/// a kink; a linear ramp changes slope by `(new - old) / n` at every block boundary.
fn crossfade_weight(i: usize, n: usize) -> f32 {
    let x = (i + 1) as f32 / (n + 1) as f32;
    0.5 - 0.5 * (std::f32::consts::PI * x).cos()
}

/// Independent partial sums in [`convolve`]: one dependent chain of `f32` adds runs at one add
/// latency per multiply-add, and the compiler may not reassociate it; eight chains vectorise.
const LANES: usize = 8;

/// `out[n] = sum_j h_reversed[j] * input[n + j]`: a causal FIR over `input`, which holds
/// `h_reversed.len() - 1` samples of history before the block. The products accumulate in
/// [`LANES`] partial sums (lane `l` takes every tap `j` with `j % LANES == l`), added together
/// at the end, then any taps past the last whole group of [`LANES`].
fn convolve(input: &[f32], h_reversed: &[f32], out: &mut [f32]) {
    let taps = h_reversed.len();
    let (h_lanes, h_rest) = h_reversed.as_chunks::<LANES>();
    for (n, y) in out.iter_mut().enumerate() {
        let (x_lanes, x_rest) = input[n..n + taps].as_chunks::<LANES>();
        let mut acc = [0.0f32; LANES];
        for (xs, hs) in x_lanes.iter().zip(h_lanes) {
            for ((a, x), h) in acc.iter_mut().zip(xs).zip(hs) {
                *a += x * h;
            }
        }
        let mut sum = acc.iter().sum::<f32>();
        for (x, h) in x_rest.iter().zip(h_rest) {
            sum += x * h;
        }
        *y = sum;
    }
}

/// The listener's axes, for turning world offsets into its frame: x front, y left, z up.
pub(super) struct ListenerFrame {
    front: [f32; 3],
    left: [f32; 3],
    up: [f32; 3],
}

impl ListenerFrame {
    pub(super) fn new(l: &Listener) -> Self {
        let front = normalize_or(l.forward, [1.0, 0.0, 0.0]);
        let right = normalize_or(cross(front, l.up), [0.0, -1.0, 0.0]);
        let up = cross(right, front);
        Self {
            front,
            left: right.map(|c| -c),
            up,
        }
    }

    pub(super) fn local(&self, v: [f32; 3]) -> [f32; 3] {
        [dot(v, self.front), dot(v, self.left), dot(v, self.up)]
    }
}

/// Constant-power pan gains for unit direction `dir` in the listener's frame (`cos`, `sin` of
/// an angle from 0 at hard left to pi/2 at hard right); centre for a source at the listener.
fn pan_gains(dir: [f32; 3], d: f32) -> [f32; 2] {
    let right = if d > 1e-6 {
        (-dir[1]).clamp(-1.0, 1.0)
    } else {
        0.0
    };
    let theta = (right + 1.0) * std::f32::consts::FRAC_PI_4;
    [theta.cos(), theta.sin()]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A seeded SplitMix64 stream of `f32` in [-1, 1).
    fn noise(seed: u64, count: usize) -> Vec<f32> {
        let mut s = seed;
        (0..count)
            .map(|_| {
                s = s.wrapping_add(0x9e37_79b9_7f4a_7c15);
                let mut z = s;
                z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
                z ^= z >> 31;
                (z >> 40) as f32 / (1u64 << 23) as f32 - 1.0
            })
            .collect()
    }

    /// Relative to the sum of the products' magnitudes, the scale a dot product's rounding
    /// error is bounded by: a random signed filter cancels to near zero at some samples, where
    /// an error relative to the output itself measures the cancellation, not the sum's order.
    #[test]
    fn convolve_matches_a_naive_f64_convolution() {
        const TAPS: usize = 128;
        let h = noise(0x0006_0001, TAPS);
        let input = noise(0x0006_0002, TAPS - 1 + BLOCK);
        let mut out = vec![0.0f32; BLOCK];
        convolve(&input, &h, &mut out);
        let mut worst = 0.0f64;
        for (n, &y) in out.iter().enumerate() {
            let products = (0..TAPS).map(|j| f64::from(h[j]) * f64::from(input[n + j]));
            let want: f64 = products.clone().sum();
            let scale: f64 = products.map(f64::abs).sum();
            let rel = (f64::from(y) - want).abs() / scale;
            assert!(
                rel < 1e-5,
                "out[{n}] = {y}, naive f64 {want} over magnitude {scale}: relative error {rel}"
            );
            worst = worst.max(rel);
        }
        println!("convolve against f64, {TAPS} taps x {BLOCK}: worst relative error {worst:e}");
    }
}
