//! The device: a `cpal` output stream whose callback owns the [`Mixer`] (ADR-0006).
//!
//! The callback drains every queued command at the top of each buffer, then renders the
//! buffer. It takes no lock and blocks on nothing; finished voices go back over a second
//! bounded channel, dropped if the game never reads them.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender};

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
        let stream = device
            .build_output_stream(
                config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    while let Some(cmd) = rx.try_next() {
                        mixer.apply(cmd);
                    }
                    mixer.render(data, data.len() / 2);
                    mixer.drain_finished(|id| {
                        let _ = finished_tx.try_send(id);
                    });
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
