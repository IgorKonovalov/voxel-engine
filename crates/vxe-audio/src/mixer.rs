//! The mixer: voices in, interleaved stereo out, one block at a time.
//!
//! Per voice and block: read the voice's mono samples, derive its per-channel gain from the
//! listener (distance gain `1 / max(d, 1 m)`, constant-power pan on the listener's right axis),
//! and add the samples into the output through a one-pole gain smoother of
//! [`crate::SMOOTHING_SECONDS`]. Nothing here allocates after construction: the voice list and
//! the scratch are sized once, a `Play` beyond [`MAX_VOICES`] is dropped.

use crate::command::Command;
use crate::voice::{Voice, VoiceId};
use crate::{BLOCK, Listener, RATE, SMOOTHING_SECONDS, cross, dot, length, normalize_or, sub};

/// Voices the mixer holds at once.
pub const MAX_VOICES: usize = 256;

/// Mixes voices into interleaved stereo `f32` at [`RATE`].
pub struct Mixer {
    voices: Vec<Voice>,
    listener: Listener,
    metres_per_voxel: f32,
    /// One block of one voice's mono samples.
    scratch: Vec<f32>,
    /// Voices that ended since the last [`Mixer::drain_finished`].
    finished: Vec<VoiceId>,
    /// Per-sample step of the one-pole smoother.
    smooth_k: f32,
}

impl Mixer {
    /// A mixer for a world of `metres_per_voxel` (0.1 for 10 cm voxels).
    pub fn new(metres_per_voxel: f32) -> Self {
        Self {
            voices: Vec::with_capacity(MAX_VOICES),
            listener: Listener::default(),
            metres_per_voxel,
            scratch: vec![0.0; BLOCK],
            finished: Vec::with_capacity(MAX_VOICES),
            smooth_k: 1.0 - (-1.0 / (SMOOTHING_SECONDS * RATE as f32)).exp(),
        }
    }

    pub fn listener(&self) -> &Listener {
        &self.listener
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
                if self.voices.len() < MAX_VOICES && !self.is_playing(id) {
                    self.voices.push(Voice::new(id, sound, params));
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
            Command::Stop(id) => self.voices.retain(|v| v.id != id),
            Command::SetListener(l) => self.listener = l,
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
        let k = self.smooth_k;
        let listener = self.listener;
        let right = normalize_or(cross(listener.forward, listener.up), [0.0, -1.0, 0.0]);
        let scratch = &mut self.scratch[..n];
        for v in &mut self.voices {
            v.read(scratch);
            let target = channel_gains(&listener, right, self.metres_per_voxel, v);
            if !v.started {
                v.gain = target;
                v.started = true;
            }
            let [mut gl, mut gr] = v.gain;
            for (frame, &s) in out.chunks_exact_mut(2).zip(scratch.iter()) {
                gl += (target[0] - gl) * k;
                gr += (target[1] - gr) * k;
                frame[0] += s * gl;
                frame[1] += s * gr;
            }
            v.gain = [gl, gr];
        }
        let finished = &mut self.finished;
        self.voices.retain(|v| {
            if v.finished && finished.len() < finished.capacity() {
                finished.push(v.id);
            }
            !v.finished
        });
    }
}

/// Left and right gain of `v` heard by `listener`: its own gain, the distance gain and a
/// constant-power pan (`cos`, `sin` of an angle from 0 at hard left to pi/2 at hard right).
fn channel_gains(
    listener: &Listener,
    right: [f32; 3],
    metres_per_voxel: f32,
    v: &Voice,
) -> [f32; 2] {
    let rel = sub(v.params.position, listener.position);
    let d = length(rel) * metres_per_voxel;
    let g = v.params.gain / d.max(1.0);
    let pan = if d > 1e-6 {
        dot(normalize_or(rel, [0.0; 3]), right).clamp(-1.0, 1.0)
    } else {
        0.0
    };
    let theta = (pan + 1.0) * std::f32::consts::FRAC_PI_4;
    [g * theta.cos(), g * theta.sin()]
}
