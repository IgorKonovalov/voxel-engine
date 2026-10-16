//! Spatial audio: positional voices mixed to binaural stereo, played through `cpal`.
//!
//! The [`Mixer`] filters each voice through the head-related impulse responses of the
//! measured direction nearest to it ([`Hrtf`], SADIE II subject D2 shipped in `assets/`) and
//! renders blocks of interleaved stereo `f32` at [`RATE`] into a caller's buffer,
//! so every behaviour is testable without a device. [`Output`] moves a mixer onto the device
//! callback (ADR-0006): the callback owns it, drains a bounded command channel at the top of
//! each block and never blocks or allocates for a command.
//!
//! Positions are in voxel units; the mixer turns them into metres with the world's
//! `metres_per_voxel`. A [`Sound`] is mono PCM at [`RATE`] that the consumer decodes; the engine
//! reads no audio files.
//!
//! This crate is the perceptual layer and is not deterministic by contract.
//!
//! ```
//! use vxe_audio::*;
//!
//! let mut mixer = Mixer::new(Hrtf::sadie2_d2(), 0.1);
//! let tone: Vec<f32> = (0..4800).map(|n| (n as f32 * 0.1).sin()).collect();
//! mixer.apply(Command::Play(VoiceId(1), Sound::new(tone), PlayParams::at([10.0, 0.0, 0.0])));
//! let mut out = vec![0.0; 2 * BLOCK];
//! mixer.render(&mut out, BLOCK);
//! assert!(out.iter().any(|&s| s != 0.0));
//! ```

mod acoustics;
mod command;
mod hrtf;
mod mixer;
mod output;
mod voice;

pub use acoustics::{
    Acoustics, BlockClock, Occlusion, REFLECTION_RANGE_METRES, REFLECTION_RAYS, Reflection,
    occlusion, reflection_directions, reflections, taps,
};
pub use command::{COMMAND_CAPACITY, Command};
pub use hrtf::Hrtf;
pub use mixer::{MAX_VOICES, Mixer};
pub use output::{Output, OutputStats};
pub use voice::{
    AcousticTargets, MAX_DELAY_FRAMES, MAX_TAPS, OPEN_CUTOFF_HZ, PlayParams, Sound, Tap, VoiceId,
};

/// The mixer's sample rate in frames per second.
pub const RATE: u32 = 48_000;
/// Frames per mixer block: the unit of command draining and of filter crossfades.
pub const BLOCK: usize = 256;
/// Metres per second.
pub const SPEED_OF_SOUND: f32 = 343.0;
/// Time constant of the one-pole smoother every per-voice gain moves through: 50 ms.
pub const SMOOTHING_SECONDS: f32 = 0.05;

/// The listener: a position and an orientation in voxel units. `right` is `forward x up`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Listener {
    pub position: [f32; 3],
    pub forward: [f32; 3],
    pub up: [f32; 3],
}

impl Default for Listener {
    /// At the origin, facing +x with +z up.
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            forward: [1.0, 0.0, 0.0],
            up: [0.0, 0.0, 1.0],
        }
    }
}

/// What can go wrong opening or talking to the output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AudioError {
    /// The host has no default output device.
    NoDevice,
    /// The device cannot open 48 kHz stereo `f32`; names what it offered.
    Format(String),
    /// The command channel is full; the command was not sent.
    Busy,
    /// An HRIR pack failed validation.
    Pack(&'static str),
    /// The backend failed to build, start or keep the stream.
    Stream(String),
}

impl std::fmt::Display for AudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoDevice => write!(f, "no default output device"),
            Self::Format(offered) => {
                write!(
                    f,
                    "the device cannot open 48 kHz stereo f32; it offers {offered}"
                )
            }
            Self::Busy => write!(f, "the audio command channel is full"),
            Self::Pack(why) => write!(f, "bad HRIR pack: {why}"),
            Self::Stream(why) => write!(f, "audio stream: {why}"),
        }
    }
}

impl std::error::Error for AudioError {}

pub(crate) fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub(crate) fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(crate) fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub(crate) fn length(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}

/// `a / |a|`, or `fallback` for a zero vector.
pub(crate) fn normalize_or(a: [f32; 3], fallback: [f32; 3]) -> [f32; 3] {
    let l = length(a);
    if l > 1e-12 {
        a.map(|c| c / l)
    } else {
        fallback
    }
}
