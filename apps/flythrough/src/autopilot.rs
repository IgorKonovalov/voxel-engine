//! The autopilot's flight: its legs, the near ring it waits on and the settle clock.

use crate::edits::{Failure, ball};
use crate::run::hud_feed::FieldSamples;
use std::time::Instant;
use vxe_app::Flow;
use vxe_render::glam::Vec3;
use vxe_world::{ChunkCoord, FIXED_ONE};

/// Autopilot: dig reach (30 m) and radius (1 m), flight speed (10 m/s) and leg length.
pub(crate) const PILOT_REACH: i64 = 300 * FIXED_ONE;

pub(crate) const PILOT_RADIUS: i64 = 10 * FIXED_ONE;

pub(crate) const PILOT_SPEED: f32 = 100.0;

pub(crate) const PILOT_LEG_SECS: f64 = 30.0;

/// The autopilot's legs (`--autopilot`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Leg {
    /// Waiting for full residency, then digging.
    Settle,
    Out,
    Back,
    /// Waiting for the dug chunks to be full and mirrored again.
    Return,
}

/// `--autopilot`: waits for full residency, carves 1 m at the first rock within 30 m along +x
/// and records the touched chunks' hash, flies +x at 10 m/s for 30 s (beyond the eviction
/// radius) and back, waits for the touched chunks to be full again, and compares the hash.
/// Over the flight it counts unready-near frames (a chunk wanted full within `r_full - 2`
/// chunks of the camera's not yet resident and mirrored full), and samples streaming CPU,
/// latency, memory and march-pass GPU time.
pub(crate) struct Autopilot {
    pub(crate) leg: Leg,
    pub(crate) leg_start: Instant,
    /// The frames spent in `Leg::Settle` so far, bounded.
    pub(crate) settle: SettleClock,
    pub(crate) origin: Vec3,
    pub(crate) touched: Vec<ChunkCoord>,
    pub(crate) hash: u64,
    pub(crate) near: NearRing,
    pub(crate) flight_frames: u64,
    /// The first and last flight frame's time, in milliseconds since the start.
    pub(crate) flight_span_ms: Option<(f64, f64)>,
    pub(crate) unready_frames: u64,
    pub(crate) gpu_ms: Vec<f64>,
    pub(crate) cpu_ms: Vec<f64>,
    /// The streaming CPU's stages per flight frame: `update`, `poll`, `apply_events`, `sync`.
    pub(crate) stages_ms: [Vec<f64>; 4],
    /// The live queue's step on the CPU per flight frame.
    pub(crate) queue_ms: Vec<f64>,
    /// The light extraction inside `sync`, per flight frame; the flight's peak of light
    /// samples in the mirror, its ranges evicted and refused, and the longest wait of a chunk
    /// inside the light radius, in milliseconds.
    pub(crate) light_ms: Vec<f64>,
    /// Each frame pass's GPU time per flight frame, `FRAME_PASSES` in order.
    pub(crate) pass_ms: Vec<Vec<f64>>,
    pub(crate) peak_light: u64,
    pub(crate) light_evicted: u64,
    pub(crate) light_refused: u64,
    pub(crate) light_wait_ms: f64,
    /// The field pass over the flight, with the field's band slots.
    pub(crate) field: FieldSamples,
    pub(crate) latency_from: usize,
    pub(crate) peak_mirror: u64,
    pub(crate) peak_allocated: u64,
    /// The highest mirror pressure over the flight, and of each of its terms: pool-slot
    /// and record occupancy.
    pub(crate) peak_pressure: f32,
    pub(crate) peak_terms: [f64; 2],
    pub(crate) peak_resident: u64,
    pub(crate) peak_persisted: u64,
    /// The most staging-belt chunks the mirror held over the flight.
    pub(crate) peak_belt: usize,
    pub(crate) generated_at_takeoff: u64,
}

impl Autopilot {
    /// An autopilot starting at `start` from `origin`, settling first, with its near ring over
    /// `r_full` and every reading empty.
    pub(crate) fn new(start: Instant, origin: Vec3, r_full: i32) -> Self {
        Autopilot {
            leg: Leg::Settle,
            leg_start: start,
            settle: SettleClock::default(),
            origin,
            touched: Vec::new(),
            hash: 0,
            near: NearRing::new(r_full),
            flight_frames: 0,
            flight_span_ms: None,
            unready_frames: 0,
            peak_belt: 0,
            gpu_ms: Vec::new(),
            cpu_ms: Vec::new(),
            stages_ms: Default::default(),
            queue_ms: Vec::new(),
            light_ms: Vec::new(),
            pass_ms: Vec::new(),
            peak_light: 0,
            light_evicted: 0,
            light_refused: 0,
            light_wait_ms: 0.0,
            field: FieldSamples::default(),
            latency_from: 0,
            peak_mirror: 0,
            peak_allocated: 0,
            peak_pressure: 0.0,
            peak_terms: [0.0; 2],
            peak_resident: 0,
            peak_persisted: 0,
            generated_at_takeoff: 0,
        }
    }
}

/// The chunk offsets the unready-near count checks: every offset within `r_full - 2` chunks,
/// recomputed when the streamer's `r_full` changes (the pressure rule's `set_radii`).
pub(crate) struct NearRing {
    pub(crate) r_full: i32,
    pub(crate) offsets: Vec<[i32; 3]>,
    pub(crate) recomputes: u32,
}

impl NearRing {
    pub(crate) fn new(r_full: i32) -> Self {
        Self {
            r_full,
            offsets: ball((r_full - 2).max(0).pow(2)),
            recomputes: 0,
        }
    }

    /// Recomputes the offsets when `r_full` differs from the ring's.
    pub(crate) fn follow(&mut self, r_full: i32) {
        if r_full != self.r_full {
            let recomputes = self.recomputes + 1;
            *self = Self::new(r_full);
            self.recomputes = recomputes;
        }
    }

    /// Whether any chunk of the ring around `cam` is not `ready`.
    pub(crate) fn unready(&self, cam: ChunkCoord, ready: impl Fn(ChunkCoord) -> bool) -> bool {
        self.offsets
            .iter()
            .any(|o| !ready(ChunkCoord([0, 1, 2].map(|k| cam.0[k] + o[k]))))
    }
}

/// Frames the autopilot waits in `Leg::Settle` for full residency before it gives up: 10
/// minutes at 60 frames a second, far past any settle the testbed has read.
pub(crate) const SETTLE_FRAMES_MAX: u64 = 36_000;

/// The autopilot's wait for full residency, counted in frames.
#[derive(Default)]
pub(crate) struct SettleClock {
    pub(crate) frames: u64,
}

impl SettleClock {
    /// One settle frame: `Continue` once residency is full or while under
    /// [`SETTLE_FRAMES_MAX`] frames; at the bound, prints the timeout line with the `pending`
    /// chunks (streamed or mirrored), records it in `failure` and returns `Exit`.
    pub(crate) fn step(&mut self, full_residency: bool, pending: u64, failure: &Failure) -> Flow {
        if full_residency {
            return Flow::Continue;
        }
        self.frames += 1;
        if self.frames < SETTLE_FRAMES_MAX {
            return Flow::Continue;
        }
        let line = format!(
            "autopilot: settle timed out at tick {}, pending {pending}",
            self.frames
        );
        println!("{line}");
        failure.set(line);
        Flow::Exit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use vxe_app::Flow;

    use crate::edits::Failure;

    /// The autopilot's settle step continues while full residency is under way and below the
    /// bound, and at the bound prints, records the failure and exits.
    #[test]
    fn the_settle_step_times_out_at_its_bound() {
        let failure = Failure::default();
        let mut clock = SettleClock::default();
        assert_eq!(clock.step(true, 0, &failure), Flow::Continue);
        for _ in 1..SETTLE_FRAMES_MAX {
            assert_eq!(clock.step(false, 7, &failure), Flow::Continue);
        }
        assert!(!failure.is_some());
        assert_eq!(clock.step(false, 7, &failure), Flow::Exit);
        assert_eq!(
            failure.take().as_deref(),
            Some("autopilot: settle timed out at tick 36000, pending 7")
        );
    }
}
