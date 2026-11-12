//! The late tail: feedback delay networks shared by every voice (ADR-0015).
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
//! the nearest prime, so the lines share no common period. The cap covers every mean free path
//! a room fan can report (`4V / S <= (4/3) x` the ray range), so no fan's room clamps a line;
//! a larger one from a caller clamps to the largest prime under the cap. Lines that clamped to
//! one length would carry equal signals, which the ears' alternating-sign sums cancel.
//!
//! # A change of room
//!
//! There are two networks, one of them fed. How a room with another mean free path reaches
//! them is [`TailLines`]:
//! - [`TailLines::Handover`] (the default) never moves a line that holds signal, since a
//!   recirculating signal read through a moving line is shifted in pitch on every pass. A room
//!   whose mean free path differs from the fed network's by more than [`TAIL_HANDOVER_RATIO`]
//!   either way goes to the other network: it is cleared, takes the new lengths at once, and
//!   the input crossfades to it over [`crate::SMOOTHING_SECONDS`] (equal power, the two being
//!   uncorrelated), while the old network keeps its lengths and rings out at its own decay with
//!   no input. If the other network still rings when a handover is due, it first fades to
//!   silence over [`crate::SMOOTHING_SECONDS`], then is cleared. A room within the ratio
//!   changes the fed network's decay and energy in place, its lengths unchanged.
//! - [`TailLines::Glide`] keeps one network and glides each line's length toward its target
//!   by at most [`REVERB_DELAY_SLEW_FRAMES_PER_FRAME`] per frame through a linearly
//!   interpolated fractional read.
//!
//! The first room sets the fed network's lengths and decay at once and its input gain rises
//! from 0 through the smoother. A decay changed in place moves through the smoother; the loop
//! gains follow the current lengths and the smoothed `T60` every block.
//!
//! # Pre-delay
//!
//! The voices' summed sends pass a mono line of [`REVERB_MAX_PREDELAY_FRAMES`] plus a block
//! before the networks, read [`ReverbSettings::predelay_s`] back. A new pre-delay never glides
//! (that would repitch the input): the read crossfades linearly from the old read point to the
//! new one over [`crate::SMOOTHING_SECONDS`]; a change asked for while one crossfades waits for
//! it to finish.
//!
//! # Clearing
//!
//! A network that rings out, or finishes a fade-out, is cleared, and so is the network a
//! handover takes. Its lines are not zeroed there: they swap with a third, spare set of lines
//! that is already zero; lines that never took a nonzero sample since they were last zero stay.
//! The spare then has zeroed, a block at a time, only the run of ring frames that took nonzero
//! samples, at most [`CLEAR_FRAMES_PER_BLOCK`] (two blocks) frames of each line per block; a
//! run over the whole ring is clean after `RING / CLEAR_FRAMES_PER_BLOCK` blocks, rounded up. A
//! clear that finds the spare not yet clean (a second handover within the spare's cleaning time)
//! parks the network instead: it goes silent and inactive with its lines as they are, and swaps
//! them at the end of the first block that leaves the spare clean. A handover that falls to a
//! parked network waits for that swap; a room taken in place by a parked network wakes it on its
//! own lines, the tail it rang out to. So no block zeroes more than [`CLEAR_FRAMES_PER_BLOCK`]
//! frames of each line.
//!
//! # Level
//!
//! Every line takes the same input `b * x`, `x` the sum of the voices' sends. At frequency `w`
//! a pass through line `i` keeps `|H_i(w)|^2` of the energy and the Householder matrix
//! preserves it, so with `G(w)` the mean of `|H_i(w)|^2` over the lines, a unit impulse leaves
//! `N b^2 G (1 + G + G^2 + ...) = N b^2 G / (1 - G)` in the filtered line outputs at `w`. Each
//! ear takes half the lines (disjoint halves, alternating signs, so the ears decorrelate), so
//! `b^2 = 2 E / (N mean_w [G / (1 - G)])`, the mean over [`ENERGY_BANDS`] frequencies spread
//! evenly to Nyquist, gives each ear the tail energy `E` the room asks for. `E` is the room's
//! energy times `level^2`, `level` the [`ReverbSettings::level`] trim times
//! [`ReverbSettings::level_by_room`] at the mean free path of the room that network took last;
//! a handover's new network carries its own room's level, and what a network already holds
//! keeps the level it was fed at.
//!
//! The steady output for a steady input is `b^2` times a loop factor that grows with `T60`.
//! So that a change in place never overshoots the new steady level, `b` is computed from the
//! larger of the smoothed and the target `T60` (the loop gains rise no faster than the
//! smoother lets them, `b` already at the longer decay's value), and `b` moves up through the
//! smoother but down to its target within the block.

use crate::room_curve::{RoomCurve, TAIL_KNOTS, curve_of};
use crate::{BLOCK, RATE};

/// How a change of room reaches the tail's lines (module comment).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TailLines {
    /// Two networks: a room with another mean free path goes to the silent one, the input
    /// crossfades to it, and no line that holds signal moves.
    #[default]
    Handover,
    /// One network whose lines glide to their new lengths.
    Glide,
}

/// The longest pre-delay: 200 ms.
pub const REVERB_MAX_PREDELAY_FRAMES: usize = 9_600;
/// The pre-delay line holds the longest pre-delay plus a block.
const PRE_RING: usize = REVERB_MAX_PREDELAY_FRAMES + BLOCK;

/// A room whose mean free path differs from the fed network's by more than this factor,
/// either way, is handed to the other network ([`TailLines::Handover`]).
pub const TAIL_HANDOVER_RATIO: f32 = 1.15;

/// Delay lines in the network.
pub const REVERB_LINES: usize = 16;
/// The longest line: 583 ms, line 15's length at the largest mean free path a room fan can
/// report (`1.5 x (4/3) x 100 m / 343 m/s x 48 000 = 27 988` frames).
pub const REVERB_MAX_DELAY_FRAMES: usize = 28_000;
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

/// Frames of each spare line zeroed at most per block (module comment, "Clearing").
const CLEAR_FRAMES_PER_BLOCK: usize = 2 * BLOCK;

/// How the tail sounds beyond what the room gives it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReverbSettings {
    /// The decay time at 4 kHz as a fraction of the decay time at DC.
    pub hf_ratio: f32,
    /// Amplitude scale of the whole tail, a trim on top of `level_by_room`.
    pub level: f32,
    /// The tail's level in dB by the mean free path of the room a network takes; it multiplies
    /// `level` where the level enters the network's input gain.
    pub level_by_room: RoomCurve,
    /// The longest decay time the tail takes.
    pub max_t60_s: f32,
    /// How a change of room reaches the lines.
    pub lines: TailLines,
    /// Seconds the voices' sends wait before they enter the networks, 0 to 0.2.
    pub predelay_s: f32,
}

impl ReverbSettings {
    /// The pre-delay in whole frames, clamped to `[0, REVERB_MAX_PREDELAY_FRAMES]`.
    pub fn predelay_frames(&self) -> usize {
        (self.predelay_s.max(0.0) * RATE as f32)
            .round()
            .min(REVERB_MAX_PREDELAY_FRAMES as f32) as usize
    }
}

impl Default for ReverbSettings {
    fn default() -> Self {
        Self {
            hf_ratio: 0.5,
            level: 1.0,
            level_by_room: curve_of(&TAIL_KNOTS),
            max_t60_s: 8.0,
            lines: TailLines::Handover,
            predelay_s: 0.0,
        }
    }
}

/// One network as [`crate::Mixer::reverb_networks`] reads it.
#[cfg(feature = "test-hooks")]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReverbNetwork {
    pub lines: [ReverbLine; REVERB_LINES],
    /// Some line or filter state is nonzero.
    pub holds_signal: bool,
    /// The network is the fed one.
    pub fed: bool,
    /// Its input weight now, 0 to 1.
    pub input: f32,
}

/// One line as [`crate::Mixer::reverb_lines`] reads it.
#[cfg(feature = "test-hooks")]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReverbLine {
    /// Frames, fractional while gliding.
    pub length: f32,
    pub target: u32,
}

/// How the tail's clears ran, for tests (`Mixer::reverb_clears`, behind the `test-hooks`
/// feature).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReverbClears {
    /// Clears that swapped a network's lines with the spare.
    pub swaps: u64,
    /// Of those, the ones that found the spare not yet clean and finished zeroing it first.
    /// A clear defers instead, so this stays 0.
    pub forced: u64,
    /// Clears that found the spare dirty and parked their network until it was clean.
    pub deferred: u64,
    /// The most samples zeroed in one block.
    pub max_zeroed: usize,
}

mod handover;
mod network;

pub(crate) use handover::Reverb;
pub(crate) use network::flush;
