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
//!    [`crate::Tap`] target (see [`render_tap`]): the delay covers the step to a new target
//!    over as many frames as passed since the voice's previous targets, at most
//!    [`TAP_SLEW_FRAMES_PER_FRAME`] per frame, read between ring samples by linear
//!    interpolation, and the gain follows the one-pole smoother. A slot whose target
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
//! With the direct delay on ([`MixSettings::direct_delay`]) step 2 reads the voice's delay line
//! instead, `d / c` frames back (`d` the source's distance this block, at most
//! [`MAX_DELAY_FRAMES`]) through the same fractional read, and every tap reads that much
//! further back than its excess: the direct path and the echoes carry the Doppler of the
//! listener's and the source's motion. The listener arrives once per game frame, so the delay
//! moves toward `d / c` through the one-pole smoother of [`crate::SMOOTHING_SECONDS`], at
//! most [`TAP_SLEW_FRAMES_PER_FRAME`] per frame. Switched off, the delay glides back to 0 the
//! same way and the direct path then reads the voice's samples as before.
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
//!
//! Nothing here frees or zeroes a voice's memory on the callback either. A voice that ends or
//! is stopped, and a `Play` that is dropped, hand their source (which may hold the last
//! reference to its samples) and the voice's buffers to the game thread over a bounded channel
//! of [`MAX_VOICES`] slots. The game thread's [`Recycler`] drops the source there, clears the
//! buffers and sends them back over a second channel, from which a `Play` takes them. While
//! the first channel is full the callback holds what it retires and retries every block, in
//! two stores of [`MAX_VOICES`] slots each:
//! - a retired voice (source and buffers) waits in the voice hold. It cannot overflow: there
//!   are [`MAX_VOICES`] buffer sets, and each is in exactly one place (spare, playing, held,
//!   in a channel, or with the game thread), so a voice's buffers are never dropped and the
//!   pool never shrinks. Held voices go out before held sources;
//! - a dropped `Play`'s source waits in the source hold. `Play`s are unbounded, so past
//!   [`MAX_VOICES`] waiting sources a further one is dropped on the callback; that frees only
//!   when the `Play` carried the last reference to its samples.
//!
//! With the [`Recycler`] dropped (the channel disconnected) no thread is left to recycle: the
//! callback keeps a retired voice's buffers on the mixer and clears them when a `Play` takes
//! them, so the pool still never shrinks, and drops the source itself.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};

use crate::acoustics::{BlockClock, reflection_directions};
use crate::command::Command;
use crate::hrtf::Hrtf;
use crate::reverb::{Reverb, ReverbClears, ReverbLine, ReverbNetwork, ReverbSettings};
use crate::room_curve::{ECHO_KNOTS, RoomCurve, curve_of};
use crate::voice::{
    MAX_DELAY_FRAMES, MAX_TAPS, PlayParams, Source, Tap, TapState, Voice, VoiceBuffers, VoiceId,
};
use crate::{BLOCK, Listener, RATE, SMOOTHING_SECONDS};

mod render;
mod spatialiser;

use render::{Bus, lowpass_coefficient};

/// Voices the mixer holds at once.
pub const MAX_VOICES: usize = 64;

/// The balance a listener sets over everything the acoustics give: applied by
/// [`Mixer::set_mix`] or, on a running output, [`Command::SetMix`].
///
/// The early reflections' amplitude is `echo_gain` times `10^(echo_by_room.db_at(mfp) / 20)`,
/// `mfp` the mean free path of the latest [`Command::SetRoom`] (no room before the first,
/// which the curve reads as its last knot). The mixer recomputes it on [`Command::SetMix`] and
/// on `SetRoom`, never per block, and moves the buses' input to it through the one-pole
/// smoother of [`crate::SMOOTHING_SECONDS`], so a room change ramps the echoes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MixSettings {
    /// Amplitude trim of the early reflections, on top of `echo_by_room`.
    pub echo_gain: f32,
    /// The early reflections' level in dB by the room's mean free path.
    pub echo_by_room: RoomCurve,
    /// The direct path's propagation delay (module comment).
    pub direct_delay: bool,
    /// The late tail's settings.
    pub reverb: ReverbSettings,
}

impl Default for MixSettings {
    /// Echo trim 1 over the default echo curve (`(2.67 m, +30 dB)`, `(8.0, +15)`,
    /// `(24.0, +12)`, `(69.0, +6)`), the direct delay off, the tail's defaults (trim 1 over
    /// `(2.67 m, -15 dB)`, `(8.0, 0)`, no pre-delay, [`crate::TailLines::Handover`]).
    fn default() -> Self {
        Self {
            echo_gain: 1.0,
            echo_by_room: curve_of(&ECHO_KNOTS),
            direct_delay: false,
            reverb: ReverbSettings::default(),
        }
    }
}

impl MixSettings {
    /// The defaults with both room curves flat at 0 dB: echoes and tail at the levels the
    /// acoustics give, scaled by the trims alone.
    pub fn flat() -> Self {
        let mut mix = Self {
            echo_by_room: RoomCurve::flat(0.0),
            ..Self::default()
        };
        mix.reverb.level_by_room = RoomCurve::flat(0.0);
        mix
    }

    /// The echoes' amplitude at mean free path `mfp_m`: the trim times the curve's factor.
    pub fn echo_amplitude(&self, mfp_m: f32) -> f32 {
        self.echo_gain * self.echo_by_room.gain_at(mfp_m)
    }
}

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
/// 5 percent on that echo while its path changes. Below it, a slot covers each new target's
/// step over the frames that passed since the voice's previous targets: a listener probed
/// every few blocks while walking moves each echo at the walk's own rate.
pub const TAP_SLEW_FRAMES_PER_FRAME: f32 = 0.05;
/// A tap whose target delay is more than this far from its current delay (10 ms; its ray hit
/// another wall) is faded out and back in at the new delay instead of gliding there.
pub const TAP_JUMP_FRAMES: f32 = 480.0;
/// A tap gain below this is silent: a jumping tap takes its new delay once its gain is below it.
pub const TAP_SILENT: f32 = 1e-3;

/// What the callback hands to the game thread: a source, which may hold the last reference
/// to its samples, and the buffers of the voice that played it, when one did.
pub(crate) struct Retired {
    source: Source,
    buffers: Option<VoiceBuffers>,
}

/// The game thread's end of a [`Mixer`]'s recycling (module comment), taken with
/// [`Mixer::take_recycler`].
pub struct Recycler {
    retired: Receiver<Retired>,
    cleared: SyncSender<VoiceBuffers>,
}

impl Recycler {
    /// Drops every source the mixer retired since the last call, clears the buffers that came
    /// with them and returns those to the mixer's pool. Returns how many items it took. Runs
    /// on the caller's thread and never blocks.
    pub fn recycle(&self) -> usize {
        let mut n = 0;
        while let Ok(Retired { source, buffers }) = self.retired.try_recv() {
            n += 1;
            drop(source);
            if let Some(mut b) = buffers {
                b.clear();
                // The pool holds every voice's buffers at most once, so the channel, sized for
                // all of them, has room.
                let _ = self.cleared.try_send(b);
            }
        }
        n
    }
}

/// Mixes voices into interleaved stereo `f32` at [`RATE`].
pub struct Mixer {
    voices: Vec<Voice>,
    /// Cleared buffers of voices not playing, from construction; later ones arrive on
    /// `cleared`.
    spare: Vec<VoiceBuffers>,
    /// Retired sources and buffers, to the game thread.
    retire: SyncSender<Retired>,
    /// Retired voices `retire` had no room for, retried every block before `held_sources`.
    /// Never overflows (module comment).
    held_voices: VecDeque<Retired>,
    /// Sources of dropped `Play`s `retire` had no room for, retried every block.
    held_sources: VecDeque<Source>,
    /// Buffers of voices retired after the game's end was dropped, cleared when taken.
    dirty: Vec<VoiceBuffers>,
    /// Buffers the game thread cleared.
    cleared: Receiver<VoiceBuffers>,
    /// The game thread's end, until [`Mixer::take_recycler`] takes it. While the mixer holds
    /// it, a `Play` that finds no cleared buffers recycles on the caller's thread: the
    /// headless use, where the caller is the game thread.
    recycler: Option<Recycler>,
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
    /// The balance applied last.
    mix: MixSettings,
    /// The mean free path of the latest `SetRoom`; NaN before the first.
    room_mfp: f32,
    /// The echoes' amplitude the smoother moves toward ([`MixSettings`]).
    echo_target: f32,
    /// The echo gain as smoothed so far, at the last block's end.
    echo_gain: f32,
    /// One block of the echo gain's ramp.
    echo_ramp: Vec<f32>,
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
        let (retire, retired) = std::sync::mpsc::sync_channel(MAX_VOICES);
        let (cleared_tx, cleared) = std::sync::mpsc::sync_channel(MAX_VOICES);
        Self {
            voices: Vec::with_capacity(MAX_VOICES),
            spare: (0..MAX_VOICES).map(|_| VoiceBuffers::new(taps)).collect(),
            retire,
            held_voices: VecDeque::with_capacity(MAX_VOICES),
            held_sources: VecDeque::with_capacity(MAX_VOICES),
            dirty: Vec::with_capacity(MAX_VOICES),
            cleared,
            recycler: Some(Recycler {
                retired,
                cleared: cleared_tx,
            }),
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
            mix: MixSettings::default(),
            room_mfp: f32::NAN,
            echo_target: MixSettings::default().echo_amplitude(f32::NAN),
            echo_gain: MixSettings::default().echo_amplitude(f32::NAN),
            echo_ramp: vec![0.0; BLOCK],
        }
    }

    /// Sets the balance: the echo trim and curve, the direct delay and the tail's settings.
    /// Before the first block the echoes start at their new amplitude; after it they ramp.
    pub fn set_mix(&mut self, mix: MixSettings) {
        self.mix = mix;
        self.reverb.set_settings(mix.reverb);
        self.retarget_echoes();
    }

    /// Recomputes the echoes' target from the mix and the latest room.
    fn retarget_echoes(&mut self) {
        self.echo_target = self.mix.echo_amplitude(self.room_mfp);
        if self.clock.blocks() == 0 {
            self.echo_gain = self.echo_target;
        }
    }

    /// The game thread's end of the recycling (module comment), once: `None` after the first
    /// call. Whoever takes it calls [`Recycler::recycle`] on a thread other than the
    /// callback's, often enough that the [`MAX_VOICES`] buffers come back for new voices.
    pub fn take_recycler(&mut self) -> Option<Recycler> {
        self.recycler.take()
    }

    /// The balance applied last.
    pub fn mix(&self) -> MixSettings {
        self.mix
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

    /// The smoothed state of voice `id`, for tests: its own gain, its send, its gain, its
    /// distance gain and its low-pass state, in that order. `None` when `id` is not playing.
    #[doc(hidden)]
    pub fn voice_smoothers(&self, id: VoiceId) -> Option<[f32; 5]> {
        let v = self.voices.iter().find(|v| v.id == id)?;
        Some([v.own_gain, v.send, v.gain, v.dist_gain, v.lowpass])
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

    /// How the tail's clears ran so far, for tests.
    #[doc(hidden)]
    pub fn reverb_clears(&self) -> ReverbClears {
        self.reverb.clears()
    }

    /// Bytes the tail's delay lines hold: both networks and the spare set they swap with.
    pub fn reverb_bytes(&self) -> usize {
        Reverb::line_bytes()
    }

    /// Bytes the tail's pre-delay line holds.
    pub fn predelay_bytes(&self) -> usize {
        Reverb::predelay_bytes()
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
                    self.retire(Retired {
                        source: v.source,
                        buffers: Some(v.buffers),
                    });
                }
            }
            Command::SetListener(l) => self.listener = l,
            Command::SetRoom(room) => {
                self.room_mfp = room.mean_free_path_m;
                self.retarget_echoes();
                self.reverb.set_room(room);
            }
            Command::SetMix(mix) => self.set_mix(mix),
            Command::SetAcoustics(id, targets) => {
                if let Some(v) = self.voice_mut(id) {
                    v.acoustic = targets;
                    // A fresh voice starts at its targets; a playing one glides to them, each
                    // slot's step spread over the frames since the previous targets.
                    if !v.started {
                        v.lowpass_a = lowpass_coefficient(targets.cutoff_hz);
                        v.taps = targets.taps.map(TapState::at);
                        v.send = targets.send;
                    } else {
                        let frames = v.since_acoustics.max(BLOCK as u32) as f32;
                        for (s, t) in v.taps.iter_mut().zip(&targets.taps) {
                            let to = t.delay_frames.min(MAX_DELAY_FRAMES as u32) as f32;
                            s.rate = ((to - s.delay).abs() / frames).min(TAP_SLEW_FRAMES_PER_FRAME);
                        }
                    }
                    v.since_acoustics = 0;
                }
            }
        }
    }

    fn start(&mut self, id: VoiceId, source: Source, params: PlayParams) {
        let buffers = if self.is_playing(id) {
            None
        } else {
            self.spare
                .pop()
                .or_else(|| self.cleared.try_recv().ok())
                .or_else(|| {
                    self.recycler.as_ref()?.recycle();
                    self.cleared.try_recv().ok()
                })
                .or_else(|| {
                    let mut b = self.dirty.pop()?;
                    b.clear();
                    Some(b)
                })
        };
        match buffers {
            Some(buffers) => self.voices.push(Voice::new(id, source, params, buffers)),
            None => self.retire(Retired {
                source,
                buffers: None,
            }),
        }
    }

    /// Hands `r` to the game thread, or holds it while the channel is full: a voice in the
    /// voice hold, which has room for every buffer set, a source alone in the source hold,
    /// dropped here when that is full. With the game's end gone, buffers stay on the mixer
    /// (module comment).
    fn retire(&mut self, r: Retired) {
        let r = if self.held_voices.is_empty() && self.held_sources.is_empty() {
            match self.retire.try_send(r) {
                Ok(()) => return,
                Err(TrySendError::Full(r)) => r,
                Err(TrySendError::Disconnected(r)) => return self.keep(r),
            }
        } else {
            r
        };
        match r.buffers {
            Some(_) => {
                debug_assert!(self.held_voices.len() < MAX_VOICES);
                self.held_voices.push_back(r);
            }
            None if self.held_sources.len() < MAX_VOICES => self.held_sources.push_back(r.source),
            None => {}
        }
    }

    /// Keeps `r`'s buffers on the mixer once the game's end is gone; the source drops here.
    fn keep(&mut self, r: Retired) {
        if let Some(b) = r.buffers {
            self.dirty.push(b);
        }
    }

    /// Retries what [`Mixer::retire`] held, voices first, oldest first, until the channel is
    /// full.
    fn flush_held(&mut self) {
        while let Some(r) = self.held_voices.pop_front() {
            match self.retire.try_send(r) {
                Ok(()) => {}
                Err(TrySendError::Disconnected(r)) => self.keep(r),
                Err(TrySendError::Full(r)) => {
                    self.held_voices.push_front(r);
                    return;
                }
            }
        }
        while let Some(source) = self.held_sources.pop_front() {
            let r = Retired {
                source,
                buffers: None,
            };
            match self.retire.try_send(r) {
                Ok(()) | Err(TrySendError::Disconnected(_)) => {}
                Err(TrySendError::Full(r)) => {
                    self.held_sources.push_front(r.source);
                    return;
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
}
