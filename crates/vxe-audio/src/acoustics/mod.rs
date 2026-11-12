//! What the voxel world does to each voice: occlusion and transmission through the material
//! table, probed on the caller's thread (ADR-0006).
//!
//! # The acoustic triple
//!
//! A material row's `acoustic` is `[A, T, R]`:
//! - `A`, absorption per metre of path: a ray keeps `(1 - A)` of its amplitude per metre it
//!   travels inside the material;
//! - `T`, transmission: the amplitude factor of entering the material, applied once per run;
//! - `R`, resonance: unused here.
//!
//! Air contributes nothing; a row of zeros (the default) blocks completely.
//!
//! The probes are in `occlusion.rs`, `reflections.rs` and `room.rs`; [`Acoustics`] runs them
//! on the cadence below.
//!
//! # Cadence
//!
//! [`Acoustics::update`] probes a voice when it or the listener moved more than 0.1 m since its
//! last probe, and at least once per 500 ms of audio time, counted in mixer blocks through a
//! [`BlockClock`] (no wall clock). A room fan begins at the first update, and again once the
//! listener has moved more than [`ROOM_MOVE_METRES`] from where the last fan began or
//! [`ROOM_INTERVAL_SECONDS`] of audio time has passed since; otherwise no room ray is traced.
//! A fan in progress traces from where it began and starts over if the listener moves more
//! than [`ROOM_MOVE_METRES`] from there.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use vxe_world::{FIXED_ONE, Fixed3, MaterialTable, World};

use crate::voice::{AcousticTargets, MAX_TAPS, Tap, VoiceId};
use crate::{BLOCK, Command, Listener, RATE, length, sub};

mod occlusion;
mod reflections;
mod room;

pub use occlusion::{Occlusion, occlusion};
pub use reflections::{
    REFLECTION_RANGE_METRES, REFLECTION_RAYS, REFLECTION_SKIN_METRES, Reflection,
    reflection_directions, reflection_keep, reflections, taps,
};
use room::Fan;
pub use room::{
    MAX_T60_SECONDS, ROOM_COS_FLOOR, ROOM_INTERVAL_SECONDS, ROOM_MOVE_METRES, ROOM_RAYS,
    ROOM_RAYS_PER_UPDATE, RoomEstimate, room_directions,
};

/// Movement, in metres, of a voice or the listener that triggers a fresh probe.
pub const PROBE_MOVE_METRES: f32 = 0.1;
/// Audio time after which a voice is probed even if nothing moved.
pub const PROBE_INTERVAL_SECONDS: f32 = 0.5;

/// A ray from `from` (voxel units) along `unit` in fixed point: the origin, the direction of
/// length close to [`FIXED_ONE`] (so `t` counts fixed units of distance), and its length.
fn fixed_ray(from: [f32; 3], unit: [f32; 3]) -> (Fixed3, [i64; 3], f32) {
    let one = FIXED_ONE as f32;
    let origin = Fixed3(from.map(|c| (c * one).floor() as i64));
    let dir = unit.map(|c| (c * one).round() as i64);
    (origin, dir, length(dir.map(|c| c as f32)))
}

/// Mixer blocks rendered so far, shared between the mixer and the game thread.
#[derive(Clone, Debug, Default)]
pub struct BlockClock(Arc<AtomicU64>);

impl BlockClock {
    pub fn blocks(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }

    pub(crate) fn tick(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

/// Where a voice and the listener were at the voice's last probe.
struct ProbeState {
    source: [f32; 3],
    listener: [f32; 3],
    block: u64,
    /// Taps sent at that probe.
    taps: usize,
}

/// Probes the world for every voice and sends the mixer its targets.
pub struct Acoustics {
    last: HashMap<VoiceId, ProbeState>,
    clock: BlockClock,
    probed: usize,
    directions: [[f32; 3]; REFLECTION_RAYS],
    reflections: bool,
    room_dirs: [[f32; 3]; ROOM_RAYS],
    /// The fan being traced.
    fan: Option<Fan>,
    /// Where and when the last fan began.
    fan_began: Option<([f32; 3], u64)>,
    room: Option<RoomEstimate>,
    reverb: bool,
    /// The room targets must go out again at the next update (the tail was switched).
    resend_room: bool,
    room_rays: usize,
    discarded: u64,
}

impl Acoustics {
    /// Probes on the audio time `clock` counts (from [`crate::Mixer::clock`]).
    pub fn new(clock: BlockClock) -> Self {
        Self {
            last: HashMap::new(),
            clock,
            probed: 0,
            directions: reflection_directions(),
            reflections: true,
            room_dirs: room_directions(),
            fan: None,
            fan_began: None,
            room: None,
            reverb: true,
            resend_room: false,
            room_rays: 0,
            discarded: 0,
        }
    }

    /// Switches the late tail on (the default) or off. Off, the room targets carry energy 0;
    /// a change sends the last estimate's targets again at the next [`Acoustics::update`].
    pub fn set_reverb(&mut self, on: bool) {
        self.resend_room |= on != self.reverb;
        self.reverb = on;
    }

    /// Whether the late tail is on.
    pub fn reverb_on(&self) -> bool {
        self.reverb
    }

    /// The room as the last complete fan saw it; `None` before the first completes.
    pub fn room(&self) -> Option<RoomEstimate> {
        self.room
    }

    /// Fans discarded so far because the listener's voxel was solid when they were due.
    #[doc(hidden)]
    pub fn discarded_fans(&self) -> u64 {
        self.discarded
    }

    /// Room rays traced by the last [`Acoustics::update`].
    pub fn last_room_rays(&self) -> usize {
        self.room_rays
    }

    /// Switches the early reflections on (the default) or off. Off, no reflection ray is
    /// traced and every probe sends silent taps, which the mixer fades out. A change makes
    /// every voice due at the next [`Acoustics::update`].
    pub fn set_reflections(&mut self, on: bool) {
        if on != self.reflections {
            self.last.clear();
        }
        self.reflections = on;
    }

    /// Whether the early reflections are on.
    pub fn reflections_on(&self) -> bool {
        self.reflections
    }

    /// Voices probed by the last [`Acoustics::update`].
    pub fn last_probed(&self) -> usize {
        self.probed
    }

    /// Tap slots with a nonzero gain the voices carry as of their last probes.
    pub fn active_taps(&self) -> usize {
        self.last.values().map(|s| s.taps).sum()
    }

    /// Probes each of `voices` (id and position, voxel units) that is due and sends a
    /// [`Command::SetAcoustics`] for it through `send`, then advances the room fan, sending a
    /// [`Command::SetRoom`] when a fan completes. Voices missing from `voices` are forgotten.
    pub fn update(
        &mut self,
        world: &World,
        table: &MaterialTable,
        listener: &Listener,
        voices: &[(VoiceId, [f32; 3])],
        metres_per_voxel: f32,
        send: &mut dyn FnMut(Command),
    ) {
        let now = self.clock.blocks();
        let interval = (PROBE_INTERVAL_SECONDS * RATE as f32 / BLOCK as f32).ceil() as u64;
        let moved = PROBE_MOVE_METRES / metres_per_voxel;
        self.last
            .retain(|id, _| voices.iter().any(|(v, _)| v == id));
        self.probed = 0;
        // The listener's reflections, traced once per update and only when a voice is due.
        let mut hits = None;
        for &(id, source) in voices {
            let due = self.last.get(&id).is_none_or(|s| {
                length(sub(source, s.source)) > moved
                    || length(sub(listener.position, s.listener)) > moved
                    || now.saturating_sub(s.block) >= interval
            });
            if !due {
                continue;
            }
            let o = occlusion(world, table, listener.position, source, metres_per_voxel);
            let taps = if self.reflections {
                let hits = hits.get_or_insert_with(|| {
                    reflections(
                        world,
                        table,
                        listener.position,
                        &self.directions,
                        metres_per_voxel,
                    )
                });
                taps(hits, listener.position, source, metres_per_voxel, o.gain)
            } else {
                [Tap::default(); MAX_TAPS]
            };
            send(Command::SetAcoustics(
                id,
                AcousticTargets {
                    gain: o.gain,
                    cutoff_hz: o.cutoff_hz,
                    taps,
                    send: o.gain,
                },
            ));
            self.last.insert(
                id,
                ProbeState {
                    source,
                    listener: listener.position,
                    block: now,
                    taps: taps.iter().filter(|t| t.gain != 0.0).count(),
                },
            );
            self.probed += 1;
        }
        self.step_room(world, table, listener.position, metres_per_voxel, send);
    }
}
