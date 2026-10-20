//! Sounds and the voices that play them.

use std::sync::Arc;

use crate::stream::StreamReader;

/// Mono PCM at [`crate::RATE`], shared read-only between the game and the callback.
#[derive(Clone, Debug, PartialEq)]
pub struct Sound(Arc<[f32]>);

impl Sound {
    pub fn new(samples: impl Into<Arc<[f32]>>) -> Self {
        Self(samples.into())
    }

    pub fn samples(&self) -> &[f32] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Names a playing voice. The [`crate::Output`] hands them out; a headless caller picks its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VoiceId(pub u32);

/// How a voice starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayParams {
    /// Voxel units.
    pub position: [f32; 3],
    /// Linear gain applied before distance and acoustics.
    pub gain: f32,
    pub looping: bool,
    /// Whether the voice's direct path sounds. Off, only its early reflections and its send
    /// into the tail do: a voice at the listener heard only as it comes back off the walls.
    pub direct: bool,
}

impl PlayParams {
    /// Gain 1, playing once, at `position`, its direct path on.
    pub fn at(position: [f32; 3]) -> Self {
        Self {
            position,
            gain: 1.0,
            looping: false,
            direct: true,
        }
    }
}

/// Taps a voice carries: one slot per reflection direction ([`crate::REFLECTION_RAYS`]).
pub const MAX_TAPS: usize = 16;
/// The longest tap delay: 200 ms at [`crate::RATE`], the length of a voice's delay line.
pub const MAX_DELAY_FRAMES: usize = 9_600;
/// The cutoff of an unobstructed voice; at or above it the low-pass is bypassed.
pub const OPEN_CUTOFF_HZ: f32 = 20_000.0;

/// One delayed copy of a voice's signal: an early reflection. Gain 0 is a slot with no hit.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tap {
    pub delay_frames: u32,
    pub gain: f32,
}

/// What the world does to a voice, from [`crate::Acoustics`]: a gain multiplier and a low-pass
/// cutoff for the direct path, the reflection taps and the send into the shared tail.
///
/// `taps` is slot-indexed: slot `i` is the reflection off the wall reflection ray `i` hits
/// ([`crate::reflection_directions`]), so a slot keeps its meaning from one probe to the next
/// and the mixer glides each slot to its new delay and gain instead of swapping the set.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AcousticTargets {
    pub gain: f32,
    pub cutoff_hz: f32,
    pub taps: [Tap; MAX_TAPS],
    /// The voice's send into the late tail: the occlusion multiplier.
    pub send: f32,
}

impl Default for AcousticTargets {
    /// Unobstructed: gain 1, cutoff [`OPEN_CUTOFF_HZ`], every tap silent, send 1.
    fn default() -> Self {
        Self {
            gain: 1.0,
            cutoff_hz: OPEN_CUTOFF_HZ,
            taps: [Tap::default(); MAX_TAPS],
            send: 1.0,
        }
    }
}

/// A tap slot as the mixer plays it now: the delay in frames (fractional while it glides) and
/// the gain, both moving toward the slot's [`Tap`] target.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct TapState {
    pub delay: f32,
    pub gain: f32,
}

impl TapState {
    pub fn at(t: Tap) -> Self {
        Self {
            delay: t.delay_frames.min(MAX_DELAY_FRAMES as u32) as f32,
            gain: t.gain,
        }
    }
}

/// What a voice plays.
pub(crate) enum Source {
    Sound(Sound),
    /// A stream, read a block at a time; it never ends by itself.
    Stream(StreamReader),
}

/// One playing sound and its per-voice mixer state.
pub(crate) struct Voice {
    pub id: VoiceId,
    pub source: Source,
    pub params: PlayParams,
    /// Next sample of `sound` to play.
    pub cursor: usize,
    /// The sound has ended (never set for a looping voice).
    pub finished: bool,
    /// Gain before spatialisation (the voice's own times distance), as smoothed so far.
    pub gain: f32,
    /// Panning path: left and right pan gain as smoothed so far.
    pub pan: [f32; 2],
    /// The acoustics' targets for this voice.
    pub acoustic: AcousticTargets,
    /// Each tap slot's delay and gain as played so far, gliding toward `acoustic.taps`.
    pub taps: [TapState; MAX_TAPS],
    /// The voice's own gain, as smoothed so far, for the delay line's input.
    pub own_gain: f32,
    /// The send into the late tail, as smoothed so far.
    pub send: f32,
    /// Where the next block goes in [`VoiceBuffers::echo`].
    pub echo_pos: usize,
    /// The low-pass coefficient as smoothed so far, 1 when open (the acoustic gain smooths
    /// inside `gain`).
    pub lowpass_a: f32,
    /// The one-pole low-pass's state.
    pub lowpass: f32,
    /// HRTF path: the measurement the last block was filtered with.
    pub dir: usize,
    /// HRTF path: the last `taps - 1` filter inputs, oldest first.
    pub buffers: VoiceBuffers,
    /// Whether any frame has rendered; a fresh voice starts at its targets instead of
    /// smoothing in from zero.
    pub started: bool,
}

/// A voice's heap state, taken from the mixer's pool on `Play` and returned when the voice
/// ends, so the callback never allocates.
pub(crate) struct VoiceBuffers {
    pub history: Vec<f32>,
    /// The delay line the taps read: a ring of the voice's own-gained signal,
    /// [`MAX_DELAY_FRAMES`] plus one block long so a whole block can be written before any tap
    /// of it is read.
    pub echo: Vec<f32>,
}

impl VoiceBuffers {
    pub fn new(taps: usize) -> Self {
        Self {
            history: vec![0.0; taps.saturating_sub(1)],
            echo: vec![0.0; MAX_DELAY_FRAMES + crate::BLOCK],
        }
    }

    pub fn clear(&mut self) {
        self.history.fill(0.0);
        self.echo.fill(0.0);
    }
}

impl Voice {
    pub fn new(id: VoiceId, source: Source, params: PlayParams, mut buffers: VoiceBuffers) -> Self {
        buffers.clear();
        Self {
            id,
            source,
            params,
            cursor: 0,
            finished: false,
            gain: 0.0,
            pan: [0.0; 2],
            acoustic: AcousticTargets::default(),
            taps: [TapState::default(); MAX_TAPS],
            own_gain: 0.0,
            send: 0.0,
            echo_pos: 0,
            lowpass_a: 1.0,
            lowpass: 0.0,
            dir: 0,
            buffers,
            started: false,
        }
    }

    /// The longest delay, in whole frames rounded up, among the tap slots this voice plays now
    /// or is moving toward.
    pub fn tap_tail(&self) -> usize {
        self.taps
            .iter()
            .zip(&self.acoustic.taps)
            .filter(|(s, t)| s.gain != 0.0 || t.gain != 0.0)
            .map(|(s, t)| (s.delay.ceil() as usize).max(t.delay_frames as usize))
            .max()
            .unwrap_or(0)
            .min(MAX_DELAY_FRAMES)
    }

    /// Fills `out` with the voice's next samples, zeros past the end of a non-looping sound or
    /// where a stream has run dry.
    /// The voice finishes once `tail` zeros have followed the end, so a filter of `tail + 1`
    /// taps rings out before the voice is dropped.
    pub fn read(&mut self, out: &mut [f32], tail: usize) {
        let sound = match &mut self.source {
            Source::Sound(sound) => sound,
            Source::Stream(stream) => {
                stream.read(out);
                return;
            }
        };
        let src = sound.samples();
        if src.is_empty() && self.params.looping {
            out.fill(0.0);
            self.finished = true;
            return;
        }
        let mut i = 0;
        while i < out.len() {
            if self.cursor >= src.len() {
                if self.params.looping {
                    self.cursor = 0;
                } else {
                    out[i..].fill(0.0);
                    self.cursor += out.len() - i;
                    break;
                }
            }
            let n = (out.len() - i).min(src.len() - self.cursor);
            out[i..i + n].copy_from_slice(&src[self.cursor..self.cursor + n]);
            self.cursor += n;
            i += n;
        }
        if !self.params.looping && self.cursor >= src.len() + tail {
            self.finished = true;
        }
    }
}
