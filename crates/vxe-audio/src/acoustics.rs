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
//! # The occlusion model
//!
//! Seven probe rays leave the listener: one to the source and six to the points 0.5 m from the
//! source along each world axis. Each ray crosses runs of material ([`World::trace_materials`]);
//! its gain is the product over its solid runs of `T(m) * (1 - A(m)) ^ (L / 1 m)`, with `L` the
//! run's length in metres. The source's gain multiplier is the mean of the seven gains, and its
//! cutoff is `20 000 Hz / (1 + 10 * thickness)`, with `thickness` the summed solid length in
//! metres along the direct ray. The mixer applies both (see `mixer.rs`).
//!
//! # Cadence
//!
//! [`Acoustics::update`] probes a voice when it or the listener moved more than 0.1 m since its
//! last probe, and at least once per 500 ms of audio time, counted in mixer blocks through a
//! [`BlockClock`] (no wall clock).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use vxe_world::{AIR, FIXED_ONE, Fixed3, MaterialTable, World};

use crate::voice::{AcousticTargets, OPEN_CUTOFF_HZ, VoiceId};
use crate::{BLOCK, Command, Listener, RATE, length, sub};

/// Movement, in metres, of a voice or the listener that triggers a fresh probe.
pub const PROBE_MOVE_METRES: f32 = 0.1;
/// Audio time after which a voice is probed even if nothing moved.
pub const PROBE_INTERVAL_SECONDS: f32 = 0.5;
/// Offset of the six side probes from the source, in metres.
const SIDE_PROBE_METRES: f32 = 0.5;

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

/// The occlusion of one source.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Occlusion {
    /// Mean of the seven probe gains.
    pub gain: f32,
    /// Summed solid length along the direct ray, metres.
    pub thickness_m: f32,
    /// `20 000 Hz / (1 + 10 * thickness_m)`.
    pub cutoff_hz: f32,
}

/// The gain of one ray from `from` to `to` (voxel units) and the solid length it crosses in
/// metres.
fn ray(
    world: &World,
    table: &MaterialTable,
    from: [f32; 3],
    to: [f32; 3],
    metres_per_voxel: f32,
) -> (f32, f32) {
    let span = sub(to, from);
    let len = length(span);
    if len < 1e-6 {
        return (1.0, 0.0);
    }
    let one = FIXED_ONE as f32;
    let origin = Fixed3(from.map(|c| (c * one).floor() as i64));
    // |dir| close to 256, so t counts fixed units (1/256 voxel) of distance.
    let dir = span.map(|c| (c / len * one).round() as i64);
    let dir_len = length(dir.map(|c| c as f32));
    let max_t = (len * one * one / dir_len).round() as i64;
    // Metres per unit of t: position moves |dir| * t / 256 fixed units.
    let metres_per_t = dir_len / one / one * metres_per_voxel;
    let mut gain = 1.0f32;
    let mut thickness = 0.0f32;
    for run in world.trace_materials(origin, dir, max_t) {
        if run.material == AIR {
            continue;
        }
        let l = (run.t1 - run.t0) as f32 * metres_per_t;
        let [a, t, _] = table.get(run.material).map_or([0.0; 3], |m| m.acoustic);
        gain *= t * (1.0 - a).max(0.0).powf(l);
        thickness += l;
    }
    (gain, thickness)
}

/// The occlusion of a source at `source` heard at `listener` (voxel units), probed through
/// `world` with the acoustic triples of `table`.
pub fn occlusion(
    world: &World,
    table: &MaterialTable,
    listener: [f32; 3],
    source: [f32; 3],
    metres_per_voxel: f32,
) -> Occlusion {
    let (direct, thickness_m) = ray(world, table, listener, source, metres_per_voxel);
    let side = SIDE_PROBE_METRES / metres_per_voxel;
    let mut sum = direct;
    for axis in 0..3 {
        for sign in [-1.0, 1.0] {
            let mut p = source;
            p[axis] += sign * side;
            sum += ray(world, table, listener, p, metres_per_voxel).0;
        }
    }
    Occlusion {
        gain: sum / 7.0,
        thickness_m,
        cutoff_hz: OPEN_CUTOFF_HZ / (1.0 + 10.0 * thickness_m),
    }
}

/// Where a voice and the listener were at the voice's last probe.
struct ProbeState {
    source: [f32; 3],
    listener: [f32; 3],
    block: u64,
}

/// Probes the world for every voice and sends the mixer its targets.
pub struct Acoustics {
    last: HashMap<VoiceId, ProbeState>,
    clock: BlockClock,
    probed: usize,
}

impl Acoustics {
    /// Probes on the audio time `clock` counts (from [`crate::Mixer::clock`]).
    pub fn new(clock: BlockClock) -> Self {
        Self {
            last: HashMap::new(),
            clock,
            probed: 0,
        }
    }

    /// Voices probed by the last [`Acoustics::update`].
    pub fn last_probed(&self) -> usize {
        self.probed
    }

    /// Probes each of `voices` (id and position, voxel units) that is due and sends a
    /// [`Command::SetAcoustics`] for it through `send`. Voices missing from `voices` are
    /// forgotten.
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
            send(Command::SetAcoustics(
                id,
                AcousticTargets {
                    gain: o.gain,
                    cutoff_hz: o.cutoff_hz,
                    ..AcousticTargets::default()
                },
            ));
            self.last.insert(
                id,
                ProbeState {
                    source,
                    listener: listener.position,
                    block: now,
                },
            );
            self.probed += 1;
        }
    }
}
