//! Sounds and the voices that play them.

use std::sync::Arc;

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
}

impl PlayParams {
    /// Gain 1, playing once, at `position`.
    pub fn at(position: [f32; 3]) -> Self {
        Self {
            position,
            gain: 1.0,
            looping: false,
        }
    }
}

/// One playing sound and its per-voice mixer state.
pub(crate) struct Voice {
    pub id: VoiceId,
    pub sound: Sound,
    pub params: PlayParams,
    /// Next sample of `sound` to play.
    pub cursor: usize,
    /// The sound has ended (never set for a looping voice).
    pub finished: bool,
    /// Per-channel gain as smoothed so far, left then right.
    pub gain: [f32; 2],
    /// Whether any frame has rendered; a fresh voice starts at its targets instead of
    /// smoothing in from zero.
    pub started: bool,
}

impl Voice {
    pub fn new(id: VoiceId, sound: Sound, params: PlayParams) -> Self {
        Self {
            id,
            sound,
            params,
            cursor: 0,
            finished: false,
            gain: [0.0; 2],
            started: false,
        }
    }

    /// Fills `out` with the voice's next samples; zeros once a non-looping sound has ended.
    pub fn read(&mut self, out: &mut [f32]) {
        let src = self.sound.samples();
        if src.is_empty() {
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
                    self.finished = true;
                    return;
                }
            }
            let n = (out.len() - i).min(src.len() - self.cursor);
            out[i..i + n].copy_from_slice(&src[self.cursor..self.cursor + n]);
            self.cursor += n;
            i += n;
        }
        if !self.params.looping && self.cursor >= src.len() {
            self.finished = true;
        }
    }
}
