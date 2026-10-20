//! What the game tells the mixer, and the bounded channel it travels on.
//!
//! The channel is `std::sync::mpsc::sync_channel` with [`COMMAND_CAPACITY`] slots: a fixed ring
//! allocated once, so neither `try_send` nor `try_recv` allocates. The sender never blocks; a
//! full ring is [`crate::AudioError::Busy`].

use std::sync::mpsc::{Receiver, SyncSender, TrySendError};

use crate::voice::{AcousticTargets, PlayParams, Sound, VoiceId};
use crate::{AudioError, Listener};

/// Slots in the command channel.
pub const COMMAND_CAPACITY: usize = 1024;

/// The late tail's targets from a completed room fan (see `acoustics.rs`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoomTargets {
    /// Decay time to 60 dB down, seconds.
    pub t60_s: f32,
    /// `4V / S`, metres.
    pub mean_free_path_m: f32,
    /// The tail's energy relative to a unit-energy direct impulse; 0 silences it.
    pub energy: f32,
}

/// One instruction for the mixer.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Play(VoiceId, Sound, PlayParams),
    SetPosition(VoiceId, [f32; 3]),
    SetGain(VoiceId, f32),
    Stop(VoiceId),
    SetListener(Listener),
    SetAcoustics(VoiceId, AcousticTargets),
    SetRoom(RoomTargets),
}

pub(crate) struct CommandSender(SyncSender<Command>);
pub(crate) struct CommandReceiver(Receiver<Command>);

pub(crate) fn channel() -> (CommandSender, CommandReceiver) {
    let (tx, rx) = std::sync::mpsc::sync_channel(COMMAND_CAPACITY);
    (CommandSender(tx), CommandReceiver(rx))
}

impl CommandSender {
    pub fn send(&self, cmd: Command) -> Result<(), AudioError> {
        self.0.try_send(cmd).map_err(|e| match e {
            TrySendError::Full(_) => AudioError::Busy,
            TrySendError::Disconnected(_) => {
                AudioError::Stream("the audio callback has stopped".to_owned())
            }
        })
    }
}

impl CommandReceiver {
    /// The next queued command, without blocking.
    pub fn try_next(&self) -> Option<Command> {
        self.0.try_recv().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_channel_returns_busy_without_blocking() {
        let (tx, rx) = channel();
        for _ in 0..COMMAND_CAPACITY {
            tx.send(Command::Stop(VoiceId(1))).unwrap();
        }
        assert_eq!(tx.send(Command::Stop(VoiceId(2))), Err(AudioError::Busy));
        assert_eq!(rx.try_next(), Some(Command::Stop(VoiceId(1))));
        tx.send(Command::Stop(VoiceId(3))).unwrap();
        let mut n = 0;
        while rx.try_next().is_some() {
            n += 1;
        }
        assert_eq!(n, COMMAND_CAPACITY);
        assert_eq!(rx.try_next(), None);
    }
}
