//! The mixer: voices in, interleaved stereo out, one block at a time.
//!
//! Per voice and block:
//! 1. read the voice's mono samples;
//! 2. scale them by the voice's gain times the distance gain `1 / max(d, 1 m)` times the
//!    acoustic gain multiplier, through a one-pole smoother of [`crate::SMOOTHING_SECONDS`],
//!    and low-pass them with a one-pole filter (`y += a (x - y)`, `a = 1 - exp(-2 pi fc / fs)`)
//!    at the acoustic cutoff `fc`. The coefficient `a` moves through the same smoother every
//!    sample; at [`OPEN_CUTOFF_HZ`] its target is 1, where the filter passes its input;
//! 3. add the early reflections: the voice's samples times its own gain (smoothed like the
//!    others, no distance gain; the acoustics fold the occlusion into each tap's gain) go into a
//!    [`MAX_DELAY_FRAMES`] delay line, and each [`crate::Tap`] adds the line's sample
//!    `delay_frames` back times the tap's gain. When the acoustics replace the taps, the next
//!    block renders both sets and crossfades from old to new along the same raised cosine as a
//!    direction change;
//! 4. spatialise. On the HRTF path the samples run through the two 128-tap FIRs of the measured
//!    direction nearest to the source (time-domain convolution over the voice's last `taps - 1`
//!    inputs and this block). When the nearest direction changes between blocks, the block is
//!    filtered with both the old and the new FIRs and crossfaded from old to new over the
//!    block along a raised cosine, so the switch neither steps nor kinks at the block edges. On the panning path a constant-power pan on the
//!    listener's right axis is smoothed like the gain.
//!
//! Nothing here allocates after construction: the voice list, the scratch and every voice's
//! buffers come from pools sized once for [`MAX_VOICES`]; a `Play` beyond it is dropped.

use std::sync::Arc;

use crate::acoustics::BlockClock;
use crate::command::Command;
use crate::hrtf::Hrtf;
use crate::voice::{MAX_DELAY_FRAMES, OPEN_CUTOFF_HZ, Tap, Voice, VoiceBuffers, VoiceId};
use crate::{BLOCK, Listener, RATE, SMOOTHING_SECONDS, cross, dot, length, normalize_or, sub};

/// Voices the mixer holds at once.
pub const MAX_VOICES: usize = 64;

/// Mixes voices into interleaved stereo `f32` at [`RATE`].
pub struct Mixer {
    voices: Vec<Voice>,
    /// Buffers of voices not playing.
    spare: Vec<VoiceBuffers>,
    hrtf: Option<Arc<Hrtf>>,
    listener: Listener,
    metres_per_voxel: f32,
    /// One block of one voice's mono samples.
    dry: Vec<f32>,
    /// The FIR input of one voice: its history, then this block, gained.
    input: Vec<f32>,
    /// The old and new FIR outputs of one voice for a crossfade, per ear.
    wet: [[Vec<f32>; 2]; 2],
    /// Voices that ended since the last [`Mixer::drain_finished`].
    finished: Vec<VoiceId>,
    /// Per-sample step of the one-pole smoother.
    smooth_k: f32,
    /// Blocks rendered, for [`crate::Acoustics`]' cadence.
    clock: BlockClock,
}

impl Mixer {
    /// A binaural mixer over `hrtf` for a world of `metres_per_voxel` (0.1 for 10 cm voxels).
    pub fn new(hrtf: Arc<Hrtf>, metres_per_voxel: f32) -> Self {
        Self::build(Some(hrtf), metres_per_voxel)
    }

    /// A mixer that pans with constant power instead of filtering through an HRTF.
    pub fn panning(metres_per_voxel: f32) -> Self {
        Self::build(None, metres_per_voxel)
    }

    fn build(hrtf: Option<Arc<Hrtf>>, metres_per_voxel: f32) -> Self {
        let taps = hrtf.as_ref().map_or(1, |h| h.taps());
        Self {
            voices: Vec::with_capacity(MAX_VOICES),
            spare: (0..MAX_VOICES).map(|_| VoiceBuffers::new(taps)).collect(),
            hrtf,
            listener: Listener::default(),
            metres_per_voxel,
            dry: vec![0.0; BLOCK],
            input: vec![0.0; taps - 1 + BLOCK],
            wet: std::array::from_fn(|_| std::array::from_fn(|_| vec![0.0; BLOCK])),
            finished: Vec::with_capacity(MAX_VOICES),
            smooth_k: 1.0 - (-1.0 / (SMOOTHING_SECONDS * RATE as f32)).exp(),
            clock: BlockClock::default(),
        }
    }

    /// The count of blocks this mixer has rendered, shared with whoever holds a clone.
    pub fn clock(&self) -> BlockClock {
        self.clock.clone()
    }

    pub fn listener(&self) -> &Listener {
        &self.listener
    }

    pub fn hrtf(&self) -> Option<&Arc<Hrtf>> {
        self.hrtf.as_ref()
    }

    /// Voices playing, finished ones excluded.
    pub fn voice_count(&self) -> usize {
        self.voices.len()
    }

    /// Whether `id` is still playing.
    pub fn is_playing(&self, id: VoiceId) -> bool {
        self.voices.iter().any(|v| v.id == id)
    }

    /// Applies one command. A `Play` past [`MAX_VOICES`] or for an id already playing is
    /// dropped; a command naming a voice that is not playing does nothing.
    pub fn apply(&mut self, cmd: Command) {
        match cmd {
            Command::Play(id, sound, params) => {
                if !self.is_playing(id)
                    && let Some(buffers) = self.spare.pop()
                {
                    self.voices.push(Voice::new(id, sound, params, buffers));
                }
            }
            Command::SetPosition(id, p) => {
                if let Some(v) = self.voice_mut(id) {
                    v.params.position = p;
                }
            }
            Command::SetGain(id, g) => {
                if let Some(v) = self.voice_mut(id) {
                    v.params.gain = g;
                }
            }
            Command::Stop(id) => {
                if let Some(i) = self.voices.iter().position(|v| v.id == id) {
                    let v = self.voices.swap_remove(i);
                    self.spare.push(v.buffers);
                }
            }
            Command::SetListener(l) => self.listener = l,
            Command::SetAcoustics(id, targets) => {
                if let Some(v) = self.voice_mut(id) {
                    let count = usize::from(targets.tap_count.min(crate::MAX_TAPS as u8));
                    let changed = count != usize::from(v.acoustic.tap_count)
                        || targets.taps[..count] != v.acoustic.taps[..count];
                    // A second change before the block renders keeps fading out from what
                    // was last heard.
                    if v.started && changed && !v.fading {
                        v.old_taps = v.acoustic.taps;
                        v.old_tap_count = v.acoustic.tap_count;
                        v.fading = true;
                    }
                    v.acoustic = targets;
                    v.acoustic.tap_count = count as u8;
                    if !v.started {
                        v.lowpass_a = lowpass_coefficient(targets.cutoff_hz);
                    }
                }
            }
        }
    }

    fn voice_mut(&mut self, id: VoiceId) -> Option<&mut Voice> {
        self.voices.iter_mut().find(|v| v.id == id)
    }

    /// Calls `f` with every voice that ended since the last call, oldest first.
    pub fn drain_finished(&mut self, mut f: impl FnMut(VoiceId)) {
        for id in self.finished.drain(..) {
            f(id);
        }
    }

    /// Renders `frames` frames of interleaved stereo into `out[..2 * frames]`, overwriting it.
    /// A shorter `out` renders as many whole frames as fit.
    pub fn render(&mut self, out: &mut [f32], frames: usize) {
        let frames = frames.min(out.len() / 2);
        let out = &mut out[..2 * frames];
        out.fill(0.0);
        for block in out.chunks_mut(2 * BLOCK) {
            self.render_block(block);
        }
    }

    /// One block of at most [`BLOCK`] frames, added into `out`.
    fn render_block(&mut self, out: &mut [f32]) {
        let n = out.len() / 2;
        self.clock.tick();
        let k = self.smooth_k;
        let frame = ListenerFrame::new(&self.listener);
        for v in &mut self.voices {
            let rel = frame.local(sub(v.params.position, self.listener.position));
            let d = length(rel) * self.metres_per_voxel;
            let target_gain = v.params.gain / d.max(1.0) * v.acoustic.gain;
            let dir = normalize_or(rel, [1.0, 0.0, 0.0]);
            let dry = &mut self.dry[..n];
            let tail = v.tap_tail() + self.hrtf.as_ref().map_or(0, |h| h.taps() - 1);
            v.read(dry, tail);
            if !v.started {
                v.gain = target_gain;
                v.own_gain = v.params.gain;
            }
            // The delay line takes the voice's own-gained signal before distance and occlusion.
            let echo_start = v.echo_pos;
            {
                let ring = &mut v.buffers.echo;
                let len = ring.len();
                let mut og = v.own_gain;
                for (i, &s) in dry.iter().enumerate() {
                    og += (v.params.gain - og) * k;
                    ring[(echo_start + i) % len] = s * og;
                }
                v.own_gain = og;
                v.echo_pos = (echo_start + n) % len;
            }
            let mut g = v.gain;
            for s in dry.iter_mut() {
                g += (target_gain - g) * k;
                *s *= g;
            }
            v.gain = g;
            // The coefficient moves every sample, so neither a probe's new cutoff nor the step
            // into or out of the bypass (a = 1) changes the output's slope at a block edge.
            let target_a = lowpass_coefficient(v.acoustic.cutoff_hz);
            if v.lowpass_a < 1.0 || target_a < 1.0 {
                let mut a = v.lowpass_a;
                let mut y = v.lowpass;
                for s in dry.iter_mut() {
                    a += (target_a - a) * k;
                    y += a * (*s - y);
                    *s = y;
                }
                // Within float reach of the bypass, take it.
                v.lowpass_a = if target_a == 1.0 && a > 1.0 - 1e-6 {
                    1.0
                } else {
                    a
                };
                v.lowpass = y;
            } else if let Some(&last) = dry.last() {
                v.lowpass = last;
            }
            let taps = &v.acoustic.taps[..usize::from(v.acoustic.tap_count)];
            let ring = &v.buffers.echo;
            if v.fading {
                let old = &v.old_taps[..usize::from(v.old_tap_count)];
                for (i, s) in dry.iter_mut().enumerate() {
                    let a = tap_sum(old, ring, echo_start + i);
                    let b = tap_sum(taps, ring, echo_start + i);
                    *s += a + (b - a) * crossfade_weight(i, n);
                }
                v.fading = false;
            } else if !taps.is_empty() {
                for (i, s) in dry.iter_mut().enumerate() {
                    *s += tap_sum(taps, ring, echo_start + i);
                }
            }
            match &self.hrtf {
                Some(hrtf) => {
                    let to = hrtf.nearest(dir);
                    if !v.started {
                        v.dir = to;
                    }
                    let from = v.dir;
                    v.dir = to;
                    let taps = hrtf.taps();
                    let input = &mut self.input[..taps - 1 + n];
                    input[..taps - 1].copy_from_slice(&v.buffers.history);
                    input[taps - 1..].copy_from_slice(dry);
                    let [new, old] = &mut self.wet;
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
                    v.buffers.history.copy_from_slice(&input[n..]);
                }
                None => {
                    let target = pan_gains(dir, d);
                    if !v.started {
                        v.pan = target;
                    }
                    let [mut gl, mut gr] = v.pan;
                    for (f, &s) in out.chunks_exact_mut(2).zip(dry.iter()) {
                        gl += (target[0] - gl) * k;
                        gr += (target[1] - gr) * k;
                        f[0] += s * gl;
                        f[1] += s * gr;
                    }
                    v.pan = [gl, gr];
                }
            }
            v.started = true;
        }
        let mut i = 0;
        while i < self.voices.len() {
            if self.voices[i].finished {
                let v = self.voices.swap_remove(i);
                if self.finished.len() < self.finished.capacity() {
                    self.finished.push(v.id);
                }
                self.spare.push(v.buffers);
            } else {
                i += 1;
            }
        }
    }
}

/// The one-pole low-pass coefficient `a = 1 - exp(-2 pi fc / fs)` of `y += a (x - y)` for
/// cutoff `fc`; 1 (the filter passes its input) at or above [`OPEN_CUTOFF_HZ`].
fn lowpass_coefficient(cutoff_hz: f32) -> f32 {
    if cutoff_hz >= OPEN_CUTOFF_HZ {
        1.0
    } else {
        1.0 - (-2.0 * std::f32::consts::PI * cutoff_hz / RATE as f32).exp()
    }
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

/// The taps' sum at ring position `at`: each tap's gain times the sample `delay_frames` before
/// it. `ring` is at least [`MAX_DELAY_FRAMES`] plus one block long and `at` was written this
/// block, so every delay up to [`MAX_DELAY_FRAMES`] reads a sample not yet overwritten.
fn tap_sum(taps: &[Tap], ring: &[f32], at: usize) -> f32 {
    let len = ring.len();
    taps.iter()
        .map(|t| {
            let d = (t.delay_frames as usize).min(MAX_DELAY_FRAMES);
            t.gain * ring[(at + len - d) % len]
        })
        .sum()
}

/// `out[n] = sum_j h_reversed[j] * input[n + j]`: a causal FIR over `input`, which holds
/// `h_reversed.len() - 1` samples of history before the block.
fn convolve(input: &[f32], h_reversed: &[f32], out: &mut [f32]) {
    let taps = h_reversed.len();
    for (n, y) in out.iter_mut().enumerate() {
        let x = &input[n..n + taps];
        *y = x.iter().zip(h_reversed).map(|(a, b)| a * b).sum();
    }
}

/// The listener's axes, for turning world offsets into its frame: x front, y left, z up.
struct ListenerFrame {
    front: [f32; 3],
    left: [f32; 3],
    up: [f32; 3],
}

impl ListenerFrame {
    fn new(l: &Listener) -> Self {
        let front = normalize_or(l.forward, [1.0, 0.0, 0.0]);
        let right = normalize_or(cross(front, l.up), [0.0, -1.0, 0.0]);
        let up = cross(right, front);
        Self {
            front,
            left: right.map(|c| -c),
            up,
        }
    }

    fn local(&self, v: [f32; 3]) -> [f32; 3] {
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
