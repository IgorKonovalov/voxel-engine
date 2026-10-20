//! The microphone: a `cpal` input stream whose callback writes the device's first channel into a
//! [`StreamWriter`], to be played as a streamed voice ([`crate::Command::PlayStream`]).
//!
//! The callback pushes without blocking; frames that do not fit the ring are dropped and counted
//! ([`Input::dropped_frames`]). Nothing here is exercised headlessly: opening a device is the
//! fly-through's job, as it is for [`crate::Output`].

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::stream::{StreamMonitor, StreamWriter};
use crate::{AudioError, RATE};

/// An open input stream feeding a [`StreamWriter`].
pub struct Input {
    _stream: cpal::Stream,
    device: String,
    period: Arc<AtomicU32>,
    errors: Arc<AtomicU64>,
    monitor: StreamMonitor,
}

/// The largest buffer the callback deinterleaves at once; a longer one is taken in pieces.
const CHUNK: usize = 1024;

impl Input {
    /// Opens the host's default input device at 48 kHz `f32` (mono, or the first channel of a
    /// wider format) and starts writing it into `writer`.
    pub fn open(mut writer: StreamWriter) -> Result<Input, AudioError> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or(AudioError::NoInputDevice)?;
        let name = device
            .description()
            .map_or_else(|e| format!("unnamed device ({e})"), |d| d.to_string());
        let configs: Vec<_> = device
            .supported_input_configs()
            .map_err(|e| AudioError::Stream(e.to_string()))?
            .collect();
        let fit = configs
            .iter()
            .filter(|c| {
                c.sample_format() == cpal::SampleFormat::F32
                    && (c.min_sample_rate()..=c.max_sample_rate()).contains(&RATE)
            })
            .min_by_key(|c| c.channels());
        let Some(fit) = fit else {
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
        };
        let channels = usize::from(fit.channels()).max(1);
        let config = cpal::StreamConfig {
            channels: fit.channels(),
            sample_rate: RATE,
            buffer_size: cpal::BufferSize::Default,
        };
        let monitor = writer.monitor();
        let period = Arc::new(AtomicU32::new(0));
        let callback_period = Arc::clone(&period);
        let errors = Arc::new(AtomicU64::new(0));
        let error_count = Arc::clone(&errors);
        let mut mono = [0.0f32; CHUNK];
        let stream = device
            .build_input_stream(
                config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    let frames = data.len() / channels;
                    callback_period.store(frames as u32, Ordering::Relaxed);
                    for piece in data.chunks(CHUNK * channels) {
                        let n = piece.len() / channels;
                        for (m, f) in mono.iter_mut().zip(piece.chunks_exact(channels)) {
                            *m = f[0];
                        }
                        writer.push(&mono[..n]);
                    }
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
        Ok(Input {
            _stream: stream,
            device: format!("{name}, 48000 Hz, {channels} ch, f32"),
            period,
            errors,
            monitor,
        })
    }

    /// The device line: its description and the stream format.
    pub fn device(&self) -> &str {
        &self.device
    }

    /// Frames in the device's last callback; 0 before the first.
    pub fn period_frames(&self) -> u32 {
        self.period.load(Ordering::Relaxed)
    }

    /// Frames dropped because the stream's ring was full.
    pub fn dropped_frames(&self) -> u64 {
        self.monitor.dropped()
    }

    /// Errors the backend reported on the running stream so far.
    pub fn stream_errors(&self) -> u64 {
        self.errors.load(Ordering::Relaxed)
    }
}
