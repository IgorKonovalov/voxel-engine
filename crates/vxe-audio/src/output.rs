//! The device: a `cpal` output stream whose callback owns the [`Mixer`] (ADR-0006).
//!
//! The callback drains every queued command at the top of each buffer, then renders the
//! buffer. It takes no lock and blocks on nothing; finished voices go back over a second
//! bounded channel, dropped if the game never reads them.
//!
//! The callback also meters itself ([`OutputStats`]): its buffer sizes, how long it ran and
//! the gap since the previous one, the output's peak, clipped samples and largest fourth
//! difference (a step or kink in the signal), and on request it records what it handed the
//! device into a ring allocated up front ([`Output::record`]).

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::command::{CommandSender, channel};
use crate::mixer::{MAX_VOICES, Mixer};
use crate::voice::{PlayParams, Sound, VoiceId};
use crate::{AudioError, Command, Listener, RATE};

/// An open output stream playing a [`Mixer`].
pub struct Output {
    stream: cpal::Stream,
    commands: CommandSender,
    finished: Receiver<VoiceId>,
    next_id: u32,
    device: String,
    errors: Arc<AtomicU64>,
    meter: Arc<Meter>,
    recorder: Arc<Mutex<Recorder>>,
}

/// The callback's own readings since the last [`Output::take_stats`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OutputStats {
    pub callbacks: u64,
    /// Smallest and largest buffer, frames.
    pub frames_min: u64,
    pub frames_max: u64,
    /// Longest callback, start to return, milliseconds.
    pub busy_max_ms: f32,
    /// Longest gap between the starts of consecutive callbacks, milliseconds.
    pub gap_max_ms: f32,
    /// Gaps longer than two of the previous buffer's durations: the device's buffer ran dry
    /// unless it holds more than two of them.
    pub late: u64,
    /// Largest output magnitude.
    pub peak: f32,
    /// Output samples beyond `[-1, 1]`, which the device clips.
    pub clipped: u64,
    /// Largest `|x[n] - 4x[n-1] + 6x[n-2] - 4x[n-3] + x[n-4]|` over either channel, across
    /// callback edges too: about three times the size of a step in the signal.
    pub kink_max: f32,
}

/// The callback's side of [`OutputStats`]: atomics it updates without blocking.
struct Meter {
    callbacks: AtomicU64,
    frames_min: AtomicU64,
    frames_max: AtomicU64,
    busy_max_ns: AtomicU64,
    gap_max_ns: AtomicU64,
    late: AtomicU64,
    /// `f32` bits; for non-negative floats the bit order is the numeric order.
    peak: AtomicU32,
    clipped: AtomicU64,
    kink_max: AtomicU32,
}

impl Meter {
    fn new() -> Self {
        Self {
            callbacks: AtomicU64::new(0),
            frames_min: AtomicU64::new(u64::MAX),
            frames_max: AtomicU64::new(0),
            busy_max_ns: AtomicU64::new(0),
            gap_max_ns: AtomicU64::new(0),
            late: AtomicU64::new(0),
            peak: AtomicU32::new(0),
            clipped: AtomicU64::new(0),
            kink_max: AtomicU32::new(0),
        }
    }

    fn take(&self) -> OutputStats {
        let f = |a: &AtomicU32| f32::from_bits(a.swap(0, Ordering::Relaxed));
        let ms = |a: &AtomicU64| a.swap(0, Ordering::Relaxed) as f32 / 1e6;
        let frames_min = self.frames_min.swap(u64::MAX, Ordering::Relaxed);
        let callbacks = self.callbacks.swap(0, Ordering::Relaxed);
        OutputStats {
            callbacks,
            frames_min: if callbacks == 0 { 0 } else { frames_min },
            frames_max: self.frames_max.swap(0, Ordering::Relaxed),
            busy_max_ms: ms(&self.busy_max_ns),
            gap_max_ms: ms(&self.gap_max_ns),
            late: self.late.swap(0, Ordering::Relaxed),
            peak: f(&self.peak),
            clipped: self.clipped.swap(0, Ordering::Relaxed),
            kink_max: f(&self.kink_max),
        }
    }
}

/// The callback's state for metering: the last callback's start and length, and the last four
/// frames for the fourth difference across the edge.
struct MeterState {
    last_start: Option<Instant>,
    last_frames: usize,
    tail: [[f32; 4]; 2],
}

impl MeterState {
    /// Meters one buffer of interleaved stereo that took from `start` to now to render.
    fn record(&mut self, meter: &Meter, data: &[f32], start: Instant) {
        let frames = data.len() / 2;
        let mut peak = 0.0f32;
        let mut clipped = 0;
        let mut kink = 0.0f32;
        for f in data.chunks_exact(2) {
            for (c, &x) in f.iter().enumerate() {
                peak = peak.max(x.abs());
                clipped += u64::from(x.abs() > 1.0);
                let [a, b, d, e] = self.tail[c];
                kink = kink.max((x - 4.0 * e + 6.0 * d - 4.0 * b + a).abs());
                self.tail[c] = [b, d, e, x];
            }
        }
        meter.callbacks.fetch_add(1, Ordering::Relaxed);
        meter.frames_min.fetch_min(frames as u64, Ordering::Relaxed);
        meter.frames_max.fetch_max(frames as u64, Ordering::Relaxed);
        meter.peak.fetch_max(peak.to_bits(), Ordering::Relaxed);
        meter.clipped.fetch_add(clipped, Ordering::Relaxed);
        meter.kink_max.fetch_max(kink.to_bits(), Ordering::Relaxed);
        let busy = now().saturating_duration_since(start);
        meter
            .busy_max_ns
            .fetch_max(busy.as_nanos() as u64, Ordering::Relaxed);
        if let Some(last) = self.last_start {
            let gap = start.saturating_duration_since(last);
            meter
                .gap_max_ns
                .fetch_max(gap.as_nanos() as u64, Ordering::Relaxed);
            let period_ns = self.last_frames as u128 * 1_000_000_000 / u128::from(RATE);
            if gap.as_nanos() > 2 * period_ns {
                meter.late.fetch_add(1, Ordering::Relaxed);
            }
        }
        self.last_start = Some(start);
        self.last_frames = frames;
    }
}

#[allow(
    clippy::disallowed_methods,
    reason = "the output stream meters its own callback; this crate is the perceptual layer, not the deterministic world"
)]
fn now() -> Instant {
    Instant::now()
}

/// A ring of the last frames the callback handed the device, allocated by [`Output::record`].
#[derive(Default)]
struct Recorder {
    ring: Vec<f32>,
    /// Next sample to write.
    at: usize,
    /// The ring has wrapped.
    full: bool,
}

impl Recorder {
    fn push(&mut self, data: &[f32]) {
        let len = self.ring.len();
        if len == 0 {
            return;
        }
        for &x in data {
            self.ring[self.at] = x;
            self.at += 1;
            if self.at == len {
                self.at = 0;
                self.full = true;
            }
        }
    }

    fn take(&mut self) -> Vec<f32> {
        let mut out = Vec::with_capacity(self.ring.len());
        if self.full {
            out.extend_from_slice(&self.ring[self.at..]);
        }
        out.extend_from_slice(&self.ring[..self.at]);
        self.at = 0;
        self.full = false;
        out
    }
}

impl Output {
    /// Opens the host's default output device at 48 kHz stereo `f32` and starts `mixer` on
    /// its callback.
    pub fn open(mut mixer: Mixer) -> Result<Output, AudioError> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or(AudioError::NoDevice)?;
        let name = device
            .description()
            .map_or_else(|e| format!("unnamed device ({e})"), |d| d.to_string());
        let configs: Vec<_> = device
            .supported_output_configs()
            .map_err(|e| AudioError::Stream(e.to_string()))?
            .collect();
        let fits = configs.iter().any(|c| {
            c.channels() == 2
                && c.sample_format() == cpal::SampleFormat::F32
                && (c.min_sample_rate()..=c.max_sample_rate()).contains(&RATE)
        });
        if !fits {
            let offered = configs
                .iter()
                .map(|c| {
                    format!(
                        "{} ch {} {}..{} Hz",
                        c.channels(),
                        c.sample_format(),
                        c.min_sample_rate(),
                        c.max_sample_rate()
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            return Err(AudioError::Format(if offered.is_empty() {
                "nothing".to_owned()
            } else {
                offered
            }));
        }
        let config = cpal::StreamConfig {
            channels: 2,
            sample_rate: RATE,
            buffer_size: cpal::BufferSize::Default,
        };
        let (commands, rx) = channel();
        let (finished_tx, finished): (SyncSender<VoiceId>, _) =
            std::sync::mpsc::sync_channel(MAX_VOICES);
        let errors = Arc::new(AtomicU64::new(0));
        let error_count = Arc::clone(&errors);
        let meter = Arc::new(Meter::new());
        let callback_meter = Arc::clone(&meter);
        let mut meter_state = MeterState {
            last_start: None,
            last_frames: 0,
            tail: [[0.0; 4]; 2],
        };
        let recorder = Arc::new(Mutex::new(Recorder::default()));
        let callback_recorder = Arc::clone(&recorder);
        let stream = device
            .build_output_stream(
                config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    let start = now();
                    while let Some(cmd) = rx.try_next() {
                        mixer.apply(cmd);
                    }
                    mixer.render(data, data.len() / 2);
                    mixer.drain_finished(|id| {
                        let _ = finished_tx.try_send(id);
                    });
                    // Only `Output::record` and `Output::take_recording` hold this lock, never
                    // across a callback's worth of time; a contended callback skips recording.
                    if let Ok(mut r) = callback_recorder.try_lock() {
                        r.push(data);
                    }
                    meter_state.record(&callback_meter, data, start);
                },
                move |_err: cpal::Error| {
                    error_count.fetch_add(1, Ordering::Relaxed);
                },
                None,
            )
            .map_err(|e| AudioError::Stream(e.to_string()))?;
        stream
            .play()
            .map_err(|e| AudioError::Stream(e.to_string()))?;
        Ok(Output {
            stream,
            commands,
            finished,
            next_id: 0,
            device: format!("{name}, 48000 Hz, 2 ch, f32"),
            errors,
            meter,
            recorder,
        })
    }

    /// The device line: its description and the stream format.
    pub fn device(&self) -> &str {
        &self.device
    }

    /// Errors the backend reported on the running stream so far.
    pub fn stream_errors(&self) -> u64 {
        self.errors.load(Ordering::Relaxed)
    }

    /// The callback's readings since the last call, and resets them.
    pub fn take_stats(&self) -> OutputStats {
        self.meter.take()
    }

    /// From now on keeps the last `seconds` of what the callback hands the device (interleaved
    /// stereo at [`RATE`]), dropping anything kept so far. The ring is allocated here, on the
    /// caller's thread.
    pub fn record(&self, seconds: f32) {
        let ring = vec![0.0; 2 * (seconds.max(0.0) * RATE as f32) as usize];
        let mut r = self
            .recorder
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *r = Recorder {
            ring,
            at: 0,
            full: false,
        };
    }

    /// What was recorded since [`Output::record`], oldest first, and starts over.
    pub fn take_recording(&self) -> Vec<f32> {
        self.recorder
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    /// Queues any command.
    pub fn send(&self, cmd: Command) -> Result<(), AudioError> {
        self.commands.send(cmd)
    }

    /// Starts `sound` and returns the voice's id.
    pub fn play(&mut self, sound: &Sound, params: PlayParams) -> Result<VoiceId, AudioError> {
        let id = VoiceId(self.next_id);
        self.send(Command::Play(id, sound.clone(), params))?;
        self.next_id = self.next_id.wrapping_add(1);
        Ok(id)
    }

    pub fn set_position(&self, id: VoiceId, position: [f32; 3]) -> Result<(), AudioError> {
        self.send(Command::SetPosition(id, position))
    }

    pub fn set_gain(&self, id: VoiceId, gain: f32) -> Result<(), AudioError> {
        self.send(Command::SetGain(id, gain))
    }

    pub fn stop(&self, id: VoiceId) -> Result<(), AudioError> {
        self.send(Command::Stop(id))
    }

    pub fn set_listener(&self, listener: Listener) -> Result<(), AudioError> {
        self.send(Command::SetListener(listener))
    }

    /// Voices that ended since the last call.
    pub fn finished(&self) -> impl Iterator<Item = VoiceId> + '_ {
        self.finished.try_iter()
    }

    /// Pauses the stream; the mixer keeps its state.
    pub fn pause(&self) -> Result<(), AudioError> {
        self.stream
            .pause()
            .map_err(|e| AudioError::Stream(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_meter_sees_a_step_across_a_callback_edge_and_clipping() {
        let meter = Meter::new();
        let mut state = MeterState {
            last_start: None,
            last_frames: 0,
            tail: [[0.0; 4]; 2],
        };
        // A constant 0.5 on both channels, then 0.6 from the next callback on: a step of 0.1
        // at the edge, which the fourth difference shows as 0.3 three frames later.
        state.record(&meter, &[0.5; 64], now());
        let first = meter.take();
        assert_eq!(first.callbacks, 1);
        assert_eq!((first.frames_min, first.frames_max), (32, 32));
        state.record(&meter, &[0.6; 64], now());
        let s = meter.take();
        assert!((s.kink_max - 0.3).abs() < 1e-6, "{}", s.kink_max);
        assert_eq!(s.clipped, 0);
        assert!((s.peak - 0.6).abs() < 1e-6);
        let mut loud = [0.6f32; 64];
        loud[10] = 1.5;
        loud[11] = -1.25;
        state.record(&meter, &loud, now());
        let s = meter.take();
        assert_eq!(s.clipped, 2);
        assert_eq!(s.peak, 1.5);
    }

    #[test]
    fn the_recorder_keeps_the_last_frames_oldest_first() {
        let mut r = Recorder {
            ring: vec![0.0; 6],
            at: 0,
            full: false,
        };
        r.push(&[1.0, 2.0, 3.0, 4.0]);
        r.push(&[5.0, 6.0, 7.0, 8.0]);
        assert_eq!(r.take(), vec![3.0, 4.0, 5.0, 6.0, 7.0, 8.0]);
        r.push(&[9.0, 10.0]);
        assert_eq!(r.take(), vec![9.0, 10.0]);
    }
}
