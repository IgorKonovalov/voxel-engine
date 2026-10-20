//! The mixer: voices in, interleaved stereo out, one block at a time.
//!
//! Per voice and block:
//! 1. read the voice's mono samples;
//! 2. scale them by the voice's gain times the distance gain `1 / max(d, 1 m)` times the
//!    acoustic gain multiplier, through a one-pole smoother of [`crate::SMOOTHING_SECONDS`],
//!    and low-pass them with a one-pole filter (`y += a (x - y)`, `a = 1 - exp(-2 pi fc / fs)`)
//!    at the acoustic cutoff `fc`. The coefficient `a` moves through the same smoother every
//!    sample; at [`OPEN_CUTOFF_HZ`] its target is 1, where the filter passes its input;
//! 3. send the early reflections to the direction buses: the voice's samples times its own gain
//!    (smoothed like the others, no distance gain; the acoustics fold the occlusion into each
//!    tap's gain) go into a [`MAX_DELAY_FRAMES`] delay line, and tap slot `i` adds the line's
//!    sample its delay back times its gain into bus `i`. A slot glides toward its
//!    [`crate::Tap`] target (see [`render_tap`]): the
//!    delay moves at most [`TAP_SLEW_FRAMES_PER_FRAME`] per frame, read between ring samples by
//!    linear interpolation, and the gain follows the one-pole smoother. A slot whose target
//!    moved more than [`TAP_JUMP_FRAMES`] (its ray now hits another wall) fades below
//!    [`TAP_SILENT`] first, then takes the new delay and fades in;
//! 4. spatialise. On the HRTF path the samples run through the two 128-tap FIRs of the measured
//!    direction nearest to the source (time-domain convolution over the voice's last `taps - 1`
//!    inputs and this block). When the nearest direction changes between blocks, the block is
//!    filtered with both the old and the new FIRs and crossfaded from old to new over the
//!    block along a raised cosine, so the switch neither steps nor kinks at the block edges. On
//!    the panning path a constant-power pan on the listener's right axis is smoothed like the
//!    gain.
//!
//! Each voice also sends its own-gained signal times its smoothed [`crate::AcousticTargets`]
//! `send` into the shared late tail (`reverb.rs`: two networks, one fed at a time), whose
//! stereo outputs add to the out after the buses.
//!
//! After every voice, each of the sixteen buses that took input or still rings is spatialised
//! the same way on the reflection direction `i` ([`crate::reflection_directions`], world axes)
//! in the listener's frame, so every echo is heard from its wall's side (ADR-0015).
//!
//! Nothing here allocates after construction: the voice list, the scratch and every voice's
//! buffers come from pools sized once for [`MAX_VOICES`]; a `Play` beyond it is dropped.

use std::sync::Arc;

use crate::acoustics::{BlockClock, reflection_directions};
use crate::command::Command;
use crate::hrtf::Hrtf;
use crate::reverb::{Reverb, ReverbLine, ReverbNetwork, ReverbSettings};
use crate::voice::{
    MAX_DELAY_FRAMES, MAX_TAPS, OPEN_CUTOFF_HZ, PlayParams, Source, Tap, TapState, Voice,
    VoiceBuffers, VoiceId,
};
use crate::{BLOCK, Listener, RATE, SMOOTHING_SECONDS, cross, dot, length, normalize_or, sub};

/// Voices the mixer holds at once.
pub const MAX_VOICES: usize = 64;
/// One tap slot as [`Mixer::tap_slots`] reads it.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TapSlot {
    /// Frames, fractional while gliding.
    pub delay: f32,
    pub gain: f32,
    pub target: Tap,
}

/// The most a tap's delay moves toward its target per frame of audio: a pitch glide of at most
/// 5 percent on that echo while its path changes.
pub const TAP_SLEW_FRAMES_PER_FRAME: f32 = 0.05;
/// A tap whose target delay is more than this far from its current delay (10 ms; its ray hit
/// another wall) is faded out and back in at the new delay instead of gliding there.
pub const TAP_JUMP_FRAMES: f32 = 480.0;
/// A tap gain below this is silent: a jumping tap takes its new delay once its gain is below it.
pub const TAP_SILENT: f32 = 1e-3;

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
    /// The direction buses: one per reflection direction, every voice's tap slot `i` summed
    /// into bus `i` and spatialised once on the bus's direction.
    buses: [Bus; MAX_TAPS],
    /// The reflection directions in world axes, bus by bus.
    bus_dirs: [[f32; 3]; MAX_TAPS],
    /// The late tail every voice sends into.
    reverb: Reverb,
    /// One block of the voices' summed sends.
    sends: Vec<f32>,
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
            buses: std::array::from_fn(|_| Bus::new(taps)),
            bus_dirs: reflection_directions(),
            reverb: Reverb::new(),
            sends: vec![0.0; BLOCK],
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

    /// The tap slots of voice `id` as played now, for tests: each slot's delay and gain and
    /// the target it moves toward. `None` when `id` is not playing.
    #[doc(hidden)]
    pub fn tap_slots(&self, id: VoiceId) -> Option<[TapSlot; crate::MAX_TAPS]> {
        let v = self.voices.iter().find(|v| v.id == id)?;
        Some(std::array::from_fn(|i| TapSlot {
            delay: v.taps[i].delay,
            gain: v.taps[i].gain,
            target: v.acoustic.taps[i],
        }))
    }

    /// Sets how the late tail sounds beyond what the room gives it.
    pub fn set_reverb_settings(&mut self, settings: ReverbSettings) {
        self.reverb.set_settings(settings);
    }

    pub fn reverb_settings(&self) -> ReverbSettings {
        self.reverb.settings()
    }

    /// The fed tail network's lines, for tests: each line's current and target length.
    #[doc(hidden)]
    pub fn reverb_lines(&self) -> [ReverbLine; crate::REVERB_LINES] {
        self.reverb.lines()
    }

    /// Both tail networks, for tests: lines, whether each holds signal, which is fed and
    /// their input weights.
    #[doc(hidden)]
    pub fn reverb_networks(&self) -> [ReverbNetwork; 2] {
        self.reverb.networks()
    }

    /// Moves the fed tail network's line `line` read point by `frames` at once, for tests that
    /// check a jump is caught; the line then glides back to its target.
    #[doc(hidden)]
    pub fn jump_reverb_line(&mut self, line: usize, frames: f32) {
        self.reverb.jump(line, frames);
    }

    /// Bytes the tail's delay lines hold, both networks.
    pub fn reverb_bytes(&self) -> usize {
        Reverb::line_bytes()
    }

    /// Whether `id` is still playing.
    pub fn is_playing(&self, id: VoiceId) -> bool {
        self.voices.iter().any(|v| v.id == id)
    }

    /// Applies one command. A `Play` past [`MAX_VOICES`] or for an id already playing is
    /// dropped; a command naming a voice that is not playing does nothing.
    pub fn apply(&mut self, cmd: Command) {
        match cmd {
            Command::Play(id, sound, params) => self.start(id, Source::Sound(sound), params),
            Command::PlayStream(id, stream, params) => {
                self.start(id, Source::Stream(stream), params)
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
            Command::SetRoom(room) => self.reverb.set_room(room),
            Command::SetAcoustics(id, targets) => {
                if let Some(v) = self.voice_mut(id) {
                    v.acoustic = targets;
                    // A fresh voice starts at its targets; a playing one glides to them.
                    if !v.started {
                        v.lowpass_a = lowpass_coefficient(targets.cutoff_hz);
                        v.taps = targets.taps.map(TapState::at);
                        v.send = targets.send;
                    }
                }
            }
        }
    }

    fn start(&mut self, id: VoiceId, source: Source, params: PlayParams) {
        if !self.is_playing(id)
            && let Some(buffers) = self.spare.pop()
        {
            self.voices.push(Voice::new(id, source, params, buffers));
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
            // The taps read the delay line until their longest delay has passed; the direct
            // path's FIR rings for its length. The buses ring out on their own.
            let tail = v
                .tap_tail()
                .max(self.hrtf.as_ref().map_or(0, |h| h.taps() - 1));
            v.read(dry, tail);
            if !v.started {
                v.gain = target_gain;
                v.own_gain = v.params.gain;
                v.send = v.acoustic.send;
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
                v.own_gain = og;
                v.send = send;
                v.echo_pos = (echo_start + n) % len;
            }
            // The gain smoother's decay over this block.
            let decay = (1.0 - k).powi(n as i32);
            for ((state, target), bus) in
                v.taps.iter_mut().zip(&v.acoustic.taps).zip(&mut self.buses)
            {
                bus.live |= render_tap(
                    state,
                    *target,
                    &v.buffers.echo,
                    echo_start,
                    &mut bus.block[..n],
                    decay,
                );
            }
            // A voice without its direct path is heard only through its taps and its send.
            if !v.params.direct {
                v.started = true;
                continue;
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

/// One direction bus: a block of summed tap output and its own spatialisation state.
struct Bus {
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
    fn new(taps: usize) -> Self {
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
        for (bus, &world_dir) in self.buses.iter_mut().zip(&self.bus_dirs) {
            if !bus.live && !bus.ringing {
                continue;
            }
            let dir = normalize_or(frame.local(world_dir), [1.0, 0.0, 0.0]);
            let block = &mut bus.block[..n];
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

/// Plays one tap slot over a block into `out`, moving `state` toward `target`, and returns
/// whether it added anything. `ring` holds the voice's own-gained signal, at least
/// [`MAX_DELAY_FRAMES`] plus one block long, with this block written at `at`, so every read up
/// to [`MAX_DELAY_FRAMES`] back finds a sample not yet overwritten.
///
/// The delay moves toward the target by at most [`TAP_SLEW_FRAMES_PER_FRAME`] per frame,
/// linearly across the block, and is read at its fractional position by linear interpolation
/// between the two ring samples around it. While the target is more than [`TAP_JUMP_FRAMES`]
/// away the delay holds and the gain heads for 0; a gain below [`TAP_SILENT`] takes a new
/// delay at once. A silent target holds the delay where it is, so a lost hit fades out at its
/// last delay. The gain follows the one-pole smoother of [`crate::SMOOTHING_SECONDS`] at block
/// edges (`decay` is the smoother's factor over the block) and moves linearly between them;
/// within `1e-7` of its target it settles on it. A settled slot reads whole-frame samples with
/// a constant gain.
fn render_tap(
    state: &mut TapState,
    target: Tap,
    ring: &[f32],
    at: usize,
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
        let reach = TAP_SLEW_FRAMES_PER_FRAME * n as f32;
        d0 + (target_delay - d0).clamp(-reach, reach)
    };
    let gain_to = if jumping { 0.0 } else { target.gain };
    let mut g1 = gain_to + (g0 - gain_to) * decay;
    if (g1 - gain_to).abs() < 1e-7 {
        g1 = gain_to;
    }
    state.delay = d1;
    state.gain = g1;
    if g0 == 0.0 && g1 == 0.0 {
        return false;
    }
    let len = ring.len();
    let inv = 1.0 / n as f32;
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
        let (dd, dg) = (f64::from(d1 - d0) / n as f64, (g1 - g0) * inv);
        for (j, y) in out.iter_mut().enumerate() {
            let d = f64::from(d0) + dd * (j + 1) as f64;
            // The read position, made positive by a whole ring before the modulo.
            let pos = (at + j + len) as f64 - d;
            let base = pos.floor();
            let frac = (pos - base) as f32;
            let i0 = base as usize % len;
            let i1 = (i0 + 1) % len;
            let x = ring[i0] + (ring[i1] - ring[i0]) * frac;
            *y += (g0 + dg * (j + 1) as f32) * x;
        }
    }
    true
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
