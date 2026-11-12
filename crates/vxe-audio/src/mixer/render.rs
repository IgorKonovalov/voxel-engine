//! The mixer's block: every voice read, gained, delayed, filtered and spatialised, its taps
//! summed into the direction buses, then the buses, the tail and the voices that ended.

use super::spatialiser::{ListenerFrame, pan, pan_gains, spatialise};
use super::{Mixer, Retired, TAP_JUMP_FRAMES, TAP_SILENT, TAP_SLEW_FRAMES_PER_FRAME};
use crate::reverb::flush;
use crate::voice::{MAX_DELAY_FRAMES, OPEN_CUTOFF_HZ, Tap, TapState};
use crate::{BLOCK, RATE, SPEED_OF_SOUND, length, normalize_or, sub};

/// How near its target a one-pole smoother settles on it, at a block edge. Left alone, a
/// smoother heading for 0 decays geometrically into subnormal floats within seconds and stays
/// there, and subnormal arithmetic costs several times normal arithmetic on every later
/// sample; a step of `1e-7` is 140 dB under a unit gain.
const SETTLE_WITHIN: f32 = 1e-7;

/// `x`, or `target` when `x` is within [`SETTLE_WITHIN`] of it.
#[inline]
fn settle(x: f32, target: f32) -> f32 {
    if (x - target).abs() < SETTLE_WITHIN {
        target
    } else {
        x
    }
}

impl Mixer {
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
            let dist_gain = v.acoustic.gain / d.max(1.0);
            let target_gain = v.params.gain * dist_gain;
            let dir = normalize_or(rel, [1.0, 0.0, 0.0]);
            let dry = &mut self.dry[..n];
            // The taps read the delay line until their longest delay has passed; the direct
            // path's FIR rings for its length. The buses ring out on their own.
            let tail = v
                .tap_tail()
                .max(self.hrtf.as_ref().map_or(0, |h| h.taps() - 1));
            v.read(dry, tail);
            v.since_acoustics = v.since_acoustics.saturating_add(n as u32);
            let delay_to = if self.mix.direct_delay {
                (d / SPEED_OF_SOUND * RATE as f32).min(MAX_DELAY_FRAMES as f32)
            } else {
                0.0
            };
            if !v.started {
                v.gain = target_gain;
                v.dist_gain = dist_gain;
                v.own_gain = v.params.gain;
                v.send = v.acoustic.send;
                v.direct_delay = delay_to;
            }
            // The delay line takes the voice's own-gained signal before distance and occlusion.
            let echo_start = v.echo_pos;
            {
                let ring = &mut v.buffers.echo;
                let len = ring.len();
                // The block lands in at most two runs: up to the ring's end, then from its start.
                let first = (len - echo_start).min(n);
                let (head, wrap) = ring.split_at_mut(echo_start);
                let runs = wrap[..first].iter_mut().chain(&mut head[..n - first]);
                // The tail's send: the same own-gained signal times the smoothed send.
                let (mut og, mut send) = (v.own_gain, v.send);
                let send_to = v.acoustic.send;
                for ((r, &s), t) in runs.zip(dry.iter()).zip(self.sends.iter_mut()) {
                    og += (v.params.gain - og) * k;
                    send += (send_to - send) * k;
                    *r = s * og;
                    *t += s * og * send;
                }
                // Settled at the block's edge, as every smoother is: none rests in subnormals.
                v.own_gain = settle(og, v.params.gain);
                v.send = settle(send, send_to);
                v.echo_pos = (echo_start + n) % len;
            }
            // The gain smoother's decay over this block.
            let decay = (1.0 - k).powi(n as i32);
            // The direct delay: the smoother's share of the way to its target, at most the
            // slew, linear across the block; back at 0 it settles there exactly.
            let dd0 = v.direct_delay;
            let reach = TAP_SLEW_FRAMES_PER_FRAME * n as f32;
            let mut dd1 = dd0 + ((delay_to - dd0) * (1.0 - decay)).clamp(-reach, reach);
            if delay_to == 0.0 && dd1 < 1e-3 {
                dd1 = 0.0;
            }
            v.direct_delay = dd1;
            for ((state, target), bus) in
                v.taps.iter_mut().zip(&v.acoustic.taps).zip(&mut self.buses)
            {
                bus.live |= render_tap(
                    state,
                    *target,
                    &v.buffers.echo,
                    echo_start,
                    (dd0, dd1),
                    &mut bus.block[..n],
                    decay,
                );
            }
            // A voice without its direct path is heard only through its taps and its send.
            if !v.params.direct {
                v.started = true;
                continue;
            }
            if dd0 == 0.0 && dd1 == 0.0 {
                let mut g = v.gain;
                for s in dry.iter_mut() {
                    g += (target_gain - g) * k;
                    *s *= g;
                }
                v.gain = settle(g, target_gain);
                v.dist_gain = settle(dist_gain + (v.dist_gain - dist_gain) * decay, dist_gain);
            } else {
                // The delay line carries the own gain; the distance and acoustic gain follow.
                let mut g = v.dist_gain;
                for_each_delayed(&v.buffers.echo, echo_start, (dd0, dd1), n, |j, x| {
                    g += (dist_gain - g) * k;
                    dry[j] = x * g;
                });
                v.dist_gain = settle(g, dist_gain);
                v.gain = settle(target_gain + (v.gain - target_gain) * decay, target_gain);
            }
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
                v.lowpass = flush(y);
            } else if let Some(&last) = dry.last() {
                v.lowpass = flush(last);
            }
            match &self.hrtf {
                Some(hrtf) => {
                    let to = hrtf.nearest(dir);
                    if !v.started {
                        v.dir = to;
                    }
                    let from = v.dir;
                    v.dir = to;
                    let scratch = (&mut self.input, &mut self.wet);
                    spatialise(hrtf, &mut v.buffers.history, dry, from, to, scratch, out);
                }
                None => {
                    let target = pan_gains(dir, d);
                    if !v.started {
                        v.pan = target;
                    }
                    pan(&mut v.pan, target, k, dry, out);
                }
            }
            v.started = true;
        }
        self.render_buses(&frame, out);
        let sends = &mut self.sends[..n];
        self.reverb.render(sends, out);
        sends.fill(0.0);
        self.flush_held();
        let mut i = 0;
        while i < self.voices.len() {
            if self.voices[i].finished {
                let v = self.voices.swap_remove(i);
                if self.finished.len() < self.finished.capacity() {
                    self.finished.push(v.id);
                }
                self.retire(Retired {
                    source: v.source,
                    buffers: Some(v.buffers),
                });
            } else {
                i += 1;
            }
        }
    }
}

/// One direction bus: a block of summed tap output and its own spatialisation state.
pub(super) struct Bus {
    /// This block's input, zeroed after each block it held anything.
    block: Vec<f32>,
    /// Some voice added into `block` this block.
    live: bool,
    /// HRTF path: the last `taps - 1` inputs, oldest first.
    history: Vec<f32>,
    /// `history` holds a nonzero sample, so the FIR still rings with no new input.
    ringing: bool,
    /// HRTF path: the measurement the last block was filtered with.
    dir: usize,
    /// Panning path: left and right gain as smoothed so far.
    pan: [f32; 2],
    /// Whether the bus has rendered; it starts at its direction instead of fading to it.
    started: bool,
}

impl Bus {
    pub(super) fn new(taps: usize) -> Self {
        Self {
            block: vec![0.0; BLOCK],
            live: false,
            history: vec![0.0; taps - 1],
            ringing: false,
            dir: 0,
            pan: [0.0; 2],
            started: false,
        }
    }
}

impl Mixer {
    /// Spatialises every bus that took input this block or still rings, each on the
    /// measurement nearest to its reflection direction in the listener's frame, and adds it to
    /// `out`. A bus whose nearest direction changed crossfades like a voice.
    fn render_buses(&mut self, frame: &ListenerFrame, out: &mut [f32]) {
        let n = out.len() / 2;
        // The echo gain's one-pole over the block; exactly 1, it is not applied.
        let (g0, to) = (self.echo_gain, self.echo_target);
        let mut g = g0;
        for r in &mut self.echo_ramp[..n] {
            g += (to - g) * self.smooth_k;
            *r = g;
        }
        self.echo_gain = settle(g, to);
        let g = self.echo_gain;
        let scaled = !(g0 == 1.0 && g == 1.0);
        for (bus, &world_dir) in self.buses.iter_mut().zip(&self.bus_dirs) {
            if !bus.live && !bus.ringing {
                continue;
            }
            let dir = normalize_or(frame.local(world_dir), [1.0, 0.0, 0.0]);
            let block = &mut bus.block[..n];
            if scaled && bus.live {
                for (x, r) in block.iter_mut().zip(&self.echo_ramp) {
                    *x *= r;
                }
            }
            match &self.hrtf {
                Some(hrtf) => {
                    let to = hrtf.nearest(dir);
                    if !bus.started {
                        bus.dir = to;
                    }
                    let from = bus.dir;
                    bus.dir = to;
                    let scratch = (&mut self.input, &mut self.wet);
                    spatialise(hrtf, &mut bus.history, block, from, to, scratch, out);
                    bus.ringing = bus.history.iter().any(|&x| x != 0.0);
                }
                None => {
                    let target = pan_gains(dir, 1.0);
                    if !bus.started {
                        bus.pan = target;
                    }
                    pan(&mut bus.pan, target, self.smooth_k, block, out);
                }
            }
            bus.started = true;
            if bus.live {
                block.fill(0.0);
                bus.live = false;
            }
        }
    }
}

/// The one-pole low-pass coefficient `a = 1 - exp(-2 pi fc / fs)` of `y += a (x - y)` for
/// cutoff `fc`; 1 (the filter passes its input) at or above [`OPEN_CUTOFF_HZ`].
pub(super) fn lowpass_coefficient(cutoff_hz: f32) -> f32 {
    if cutoff_hz >= OPEN_CUTOFF_HZ {
        1.0
    } else {
        1.0 - (-2.0 * std::f32::consts::PI * cutoff_hz / RATE as f32).exp()
    }
}

/// Calls `f(j, x)` for each frame `j` of an `n`-frame block, `x` the sample of `ring` read
/// `d0 + (d1 - d0) (j + 1) / n` frames (fractional, at most [`MAX_DELAY_FRAMES`]) before
/// position `at + j` by linear interpolation between the two ring samples around it. `at` is
/// under a ring past the ring's start. The read position moves by `1 - (d1 - d0) / n` per
/// frame, a step near 1 since a delay moves at most a few hundredths of a frame per frame,
/// carried in 32.32 fixed point: no frame rounds a float or divides.
#[inline]
fn for_each_delayed(
    ring: &[f32],
    at: usize,
    (d0, d1): (f32, f32),
    n: usize,
    mut f: impl FnMut(usize, f32),
) {
    const ONE: f64 = (1u64 << 32) as f64;
    let len = ring.len();
    let dd = f64::from(d1 - d0) / n as f64;
    // Frame 0 reads at delay `d0 + dd`; a whole ring ahead keeps the position positive.
    let pos0 = (at + len) as f64 - f64::from(d0) - dd;
    let ring_fx = (len as u64) << 32;
    let mut p = (pos0 * ONE) as u64;
    while p >= ring_fx {
        p -= ring_fx;
    }
    let step = ((1.0 - dd) * ONE) as u64;
    let scale = 1.0 / ONE as f32;
    for j in 0..n {
        let i0 = (p >> 32) as usize;
        let frac = (p & 0xffff_ffff) as f32 * scale;
        let i1 = if i0 + 1 == len { 0 } else { i0 + 1 };
        f(j, ring[i0] + (ring[i1] - ring[i0]) * frac);
        p += step;
        if p >= ring_fx {
            p -= ring_fx;
        }
    }
}

/// Plays one tap slot over a block into `out`, moving `state` toward `target`, and returns
/// whether it added anything. `ring` holds the voice's own-gained signal, at least
/// [`MAX_DELAY_FRAMES`] plus one block long, with this block written at `at`, so every read up
/// to [`MAX_DELAY_FRAMES`] back finds a sample not yet overwritten. The slot reads `offset`
/// (the direct path's delay at the block's start and end, linear between) further back than
/// its own delay, the sum at most [`MAX_DELAY_FRAMES`].
///
/// The delay moves toward the target by at most the slot's `rate` (itself at most
/// [`TAP_SLEW_FRAMES_PER_FRAME`]) per frame,
/// linearly across the block, and is read at its fractional position by linear interpolation
/// between the two ring samples around it. While the target is more than [`TAP_JUMP_FRAMES`]
/// away the delay holds and the gain heads for 0; a gain below [`TAP_SILENT`] takes a new
/// delay at once. A silent target holds the delay where it is, so a lost hit fades out at its
/// last delay. The gain follows the one-pole smoother of [`crate::SMOOTHING_SECONDS`] at block
/// edges (`decay` is the smoother's factor over the block) and moves linearly between them;
/// within [`SETTLE_WITHIN`] of its target it settles on it. A settled slot reads whole-frame samples with
/// a constant gain.
fn render_tap(
    state: &mut TapState,
    target: Tap,
    ring: &[f32],
    at: usize,
    offset: (f32, f32),
    out: &mut [f32],
    decay: f32,
) -> bool {
    let n = out.len();
    let target_delay = target.delay_frames.min(MAX_DELAY_FRAMES as u32) as f32;
    if target.gain != 0.0 && state.delay != target_delay && state.gain.abs() < TAP_SILENT {
        state.delay = target_delay;
    }
    let jumping = target.gain != 0.0 && (target_delay - state.delay).abs() > TAP_JUMP_FRAMES;
    let (d0, g0) = (state.delay, state.gain);
    let d1 = if jumping || target.gain == 0.0 {
        d0
    } else {
        let reach = state.rate * n as f32;
        d0 + (target_delay - d0).clamp(-reach, reach)
    };
    let gain_to = if jumping { 0.0 } else { target.gain };
    let g1 = settle(gain_to + (g0 - gain_to) * decay, gain_to);
    state.delay = d1;
    state.gain = g1;
    if g0 == 0.0 && g1 == 0.0 {
        return false;
    }
    let len = ring.len();
    let inv = 1.0 / n as f32;
    let max = MAX_DELAY_FRAMES as f32;
    let (d0, d1) = ((d0 + offset.0).min(max), (d1 + offset.1).min(max));
    if d0 == d1 && d0.fract() == 0.0 {
        let d = d0 as usize;
        let start = (at + len - d) % len;
        let first = (len - start).min(n);
        let (a, b) = out.split_at_mut(first);
        let src = ring[start..start + first].iter().chain(&ring[..b.len()]);
        if g0 == g1 {
            for (y, &x) in a.iter_mut().chain(b.iter_mut()).zip(src) {
                *y += g0 * x;
            }
        } else {
            let step = (g1 - g0) * inv;
            for (j, (y, &x)) in a.iter_mut().chain(b.iter_mut()).zip(src).enumerate() {
                *y += (g0 + step * (j + 1) as f32) * x;
            }
        }
    } else {
        let dg = (g1 - g0) * inv;
        for_each_delayed(ring, at, (d0, d1), n, |j, x| {
            out[j] += (g0 + dg * (j + 1) as f32) * x;
        });
    }
    true
}
