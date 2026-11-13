//! The streamed world's frame budget: the radius rule over the mirror's pressure, the
//! deferral streaks and the streamer with its readings.

use crate::hud::{self};
use std::collections::HashMap;
use vxe_render::GpuWorld;

use crate::report::SyncSeries;
use vxe_world::{ChunkCoord, Streamer, Tier};

/// The pool size a streaming device's limits are requested for; capped at the adapter.
pub(crate) const STREAM_POOL_BYTES: u64 = 2 << 30;

/// The first frame waits for the chunks within this squared distance of the spawn chunk.
pub(crate) const NEAR_D2: i32 = 4;

/// Default mirror upload budget per frame.
pub(crate) const SYNC_BUDGET_MIB: u64 = 2;

/// Mirror pressure (`GpuWorld::pressure`, the fuller of pool slots and records) above which the full radius falls by a
/// chunk once it has held for `PRESSURE_HIGH_FRAMES` frames, and below which it rises by one
/// once it has held for `PRESSURE_LOW_FRAMES` (ADR-0013).
pub(crate) const PRESSURE_HIGH: f32 = hud::PRESSURE_HIGH;

pub(crate) const PRESSURE_HIGH_FRAMES: u32 = 30;

pub(crate) const PRESSURE_LOW: f32 = crate::PRESSURE_LOW;

pub(crate) const PRESSURE_LOW_FRAMES: u32 = 300;

/// The full radius the pressure rule never lowers below, in chunks.
pub(crate) const R_FULL_FLOOR: i32 = 6;

/// The two terms of `GpuWorld::pressure`: pool-unit occupancy and record occupancy.
pub(crate) fn pressure_terms(m: &GpuWorld) -> [f64; 2] {
    [
        m.live_units() as f64 / f64::from(m.reserved_units().max(1)),
        m.mirrored_chunks() as f64 / f64::from(m.reserved_records().max(1)),
    ]
}

/// The fly-through's answer to mirror pressure: the full radius between a floor and the
/// configured radius, a chunk down after `PRESSURE_HIGH_FRAMES` frames above
/// `PRESSURE_HIGH`, a chunk up after `PRESSURE_LOW_FRAMES` frames below `PRESSURE_LOW`. A
/// change restarts both counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RadiusRule {
    pub(crate) r_full: i32,
    pub(crate) max: i32,
    pub(crate) above: u32,
    pub(crate) below: u32,
}

impl RadiusRule {
    pub(crate) fn new(max: i32) -> Self {
        Self {
            r_full: max,
            max,
            above: 0,
            below: 0,
        }
    }

    /// The floor: `R_FULL_FLOOR`, or the configured radius when that is smaller.
    pub(crate) fn floor(&self) -> i32 {
        R_FULL_FLOOR.min(self.max)
    }

    /// Takes one frame's pressure; returns the new full radius when it changes.
    pub(crate) fn step(&mut self, pressure: f32) -> Option<i32> {
        self.above = if pressure > PRESSURE_HIGH {
            self.above + 1
        } else {
            0
        };
        self.below = if pressure < PRESSURE_LOW {
            self.below + 1
        } else {
            0
        };
        let r = if self.above >= PRESSURE_HIGH_FRAMES && self.r_full > self.floor() {
            self.r_full - 1
        } else if self.below >= PRESSURE_LOW_FRAMES && self.r_full < self.max {
            self.r_full + 1
        } else {
            return None;
        };
        self.r_full = r;
        self.above = 0;
        self.below = 0;
        Some(r)
    }
}

/// The mirror's deferrals (ADR-0013): a sync that defers a chunk, or an empty sync that
/// uploads none, extends a deferral streak; a sync that defers nothing ends it. Keeps the
/// deferred-chunk total (a chunk deferred in five syncs counts five), the syncs with a
/// deferral and the empty ones among them, the lowest and highest pressure and terms at a
/// deferral, and the current and longest streak in syncs, an open streak included.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Deferrals {
    pub(crate) chunks: u64,
    pub(crate) syncs: u64,
    pub(crate) empty: u64,
    /// Lowest and highest: pressure, slot term, record term.
    pub(crate) low: [f64; 3],
    pub(crate) high: [f64; 3],
    pub(crate) streak: u64,
    pub(crate) longest: u64,
}

impl Deferrals {
    /// Notes one sync's `deferred` chunks, `empty` when it uploaded none, with the mirror's
    /// pressure and terms `v`. Returns the length of a streak this sync ends.
    pub(crate) fn note(&mut self, deferred: u64, empty: bool, v: [f64; 3]) -> Option<u64> {
        if deferred == 0 && !empty {
            let ended = (self.streak > 0).then_some(self.streak);
            self.streak = 0;
            return ended;
        }
        if self.syncs == 0 {
            (self.low, self.high) = (v, v);
        }
        self.low = [0, 1, 2].map(|k| self.low[k].min(v[k]));
        self.high = [0, 1, 2].map(|k| self.high[k].max(v[k]));
        self.chunks += deferred;
        self.syncs += 1;
        self.empty += u64::from(empty);
        self.streak += 1;
        self.longest = self.longest.max(self.streak);
        None
    }
}

/// Syncs the per-second `sync:` line keeps: past this, its newest.
pub(crate) const SYNC_LINE_SYNCS: usize = 4096;

/// The streaming state of the default mode.
pub(crate) struct Stream {
    pub(crate) streamer: Streamer,
    /// The full radius under mirror pressure, and the changes it made.
    pub(crate) rule: RadiusRule,
    pub(crate) radius_changes: u32,
    /// Chunks the mirror deferred for space, and the streaks of syncs that deferred.
    pub(crate) deferrals: Deferrals,
    /// Milliseconds since start at each `update` call, `tick_ms[t - 1]` for tick `t`.
    pub(crate) tick_ms: Vec<f64>,
    /// Chunks loaded full and not yet mirrored, with the tick that first wanted them.
    pub(crate) awaiting: HashMap<ChunkCoord, u64>,
    /// Wanted-to-mirrored latency of each chunk mirrored full, in milliseconds.
    pub(crate) latency_ms: Vec<f64>,
    /// Render-thread CPU of `update + poll + sync`, per frame.
    pub(crate) cpu_ms: Vec<f64>,
    /// The last frame's `update`, `poll`, `apply_events` and `sync` CPU in milliseconds.
    pub(crate) stages_ms: [f64; 4],
    /// The last frame's light extraction inside `sync` (`SyncStats::light_us`), in
    /// milliseconds, its ranges evicted and refused, and how long the oldest chunk inside the
    /// light radius has waited for its range, in milliseconds.
    pub(crate) light_ms: f64,
    pub(crate) light_evicted: u64,
    pub(crate) light_refused: u64,
    pub(crate) light_wait_ms: f64,
    /// The last frame's sync, `None` when it failed.
    pub(crate) last_sync: Option<vxe_render::SyncStats>,
    /// The syncs since the last `sync:` line ([`SYNC_LINE_SYNCS`] kept), and that line's time in
    /// milliseconds since start and frame.
    pub(crate) sync_line: SyncSeries,
    pub(crate) sync_line_ms: f64,
    pub(crate) sync_line_frame: u32,
    pub(crate) full_residency: bool,
    pub(crate) workers: usize,
    /// The streamer's counters as `stream_step` last read them, once a frame: what the
    /// autopilot and the HUD read after it (and the autopilot's settle, before it, a frame
    /// late). `None` before the first step.
    pub(crate) stats: Option<vxe_world::StreamStats>,
}

impl Stream {
    /// A stream over `streamer` with `workers` generation workers: the radius rule at its
    /// configured `r_full`, and every reading empty.
    pub(crate) fn new(streamer: Streamer, workers: usize) -> Self {
        Stream {
            rule: RadiusRule::new(streamer.config().residency.r_full),
            radius_changes: 0,
            deferrals: Deferrals::default(),
            streamer,
            tick_ms: Vec::new(),
            awaiting: HashMap::new(),
            latency_ms: Vec::new(),
            cpu_ms: Vec::new(),
            stages_ms: [0.0; 4],
            light_ms: 0.0,
            light_evicted: 0,
            light_refused: 0,
            light_wait_ms: 0.0,
            last_sync: None,
            sync_line: SyncSeries::with_capacity(SYNC_LINE_SYNCS),
            sync_line_ms: 0.0,
            sync_line_frame: 0,
            full_residency: false,
            workers,
            stats: None,
        }
    }

    /// Records every chunk `loaded` full with its wanted tick, and moves the ones the mirror
    /// now holds full into `latency_ms`.
    pub(crate) fn track(&mut self, ev: &vxe_world::StreamEvents, mirror: &GpuWorld) {
        let tick = self.streamer.ticks();
        let full = ev.loaded.iter().filter(|(_, t)| *t == Tier::Full);
        for (&(c, _), &lat) in full.zip(&ev.latency_ticks) {
            self.awaiting.insert(c, tick.saturating_sub(u64::from(lat)));
        }
        let now_ms = self.tick_ms.last().copied().unwrap_or(0.0);
        let tick_ms = &self.tick_ms;
        let latency = &mut self.latency_ms;
        self.awaiting.retain(|&c, &mut wanted| {
            if mirror.is_mirrored(c) && !mirror.is_lod(c) {
                let at = tick_ms
                    .get(wanted.max(1) as usize - 1)
                    .copied()
                    .unwrap_or(now_ms);
                latency.push(now_ms - at);
                false
            } else {
                true
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use crate::cave::CaveSource;

    use crate::RESIDENCY;

    use vxe_world::{ChunkCoord, PersistConfig, StreamConfig, Streamer};

    use crate::autopilot::NearRing;
    use crate::edits::ball;

    /// `--walk` and `G` refuse walk mode alike while streaming; the bounded level allows it.
    /// The unready-near ring follows the streamer's `r_full`: with chunks within 10 of the
    /// camera ready, the ring at the default 16 (offsets within 14) finds one unready; after the
    /// pressure rule's `set_radii` lowers `r_full` to 12, the recomputed ring (within 10) finds
    /// none, and a repeated radius recomputes nothing.
    #[test]
    fn the_unready_ring_is_recomputed_on_set_radii() {
        let mut streamer = Streamer::new(
            Arc::new(CaveSource::unbounded(1)),
            StreamConfig {
                residency: RESIDENCY,
                workers: 1,
                persist: PersistConfig::default(),
            },
        );
        let mut ring = NearRing::new(streamer.config().residency.r_full);
        assert_eq!(ring.offsets, ball(14 * 14));
        let cam = ChunkCoord([3, -2, 5]);
        let ready = |c: ChunkCoord| {
            (0..3)
                .map(|k| i64::from(c.0[k] - cam.0[k]).pow(2))
                .sum::<i64>()
                <= 100
        };
        assert!(ring.unready(cam, ready));
        streamer.set_radii(12, RESIDENCY.r_lod);
        ring.follow(streamer.config().residency.r_full);
        assert_eq!(ring.recomputes, 1);
        assert_eq!(ring.r_full, 12);
        assert_eq!(ring.offsets, ball(10 * 10));
        assert!(!ring.unready(cam, ready));
        ring.follow(streamer.config().residency.r_full);
        assert_eq!(ring.recomputes, 1);
    }

    /// The radius falls a chunk after 30 frames above 0.9, not before, and not below the
    /// floor; it rises a chunk after 300 frames below 0.7, not past the configured radius; a
    /// frame between the lines restarts both counts.
    #[test]
    fn the_radius_follows_the_pressure_between_its_floor_and_the_configured_radius() {
        let mut r = RadiusRule::new(8);
        for _ in 0..29 {
            assert_eq!(r.step(0.95), None);
        }
        assert_eq!(r.step(0.95), Some(7));
        assert_eq!(r.step(0.95), None, "the count restarts after a change");
        for _ in 0..28 {
            assert_eq!(r.step(0.95), None);
        }
        assert_eq!(r.step(0.95), Some(6));
        for _ in 0..100 {
            assert_eq!(r.step(1.0), None, "6 is the floor");
        }
        for _ in 0..299 {
            assert_eq!(r.step(0.5), None);
        }
        assert_eq!(
            r.step(0.8),
            None,
            "a frame between the lines restarts the count"
        );
        for _ in 0..299 {
            assert_eq!(r.step(0.5), None);
        }
        assert_eq!(r.step(0.5), Some(7));
        for _ in 0..299 {
            assert_eq!(r.step(0.5), None);
        }
        assert_eq!(r.step(0.5), Some(8));
        for _ in 0..1000 {
            assert_eq!(r.step(0.0), None, "8 is the configured radius");
        }
        // A configured radius under the floor is its own floor.
        let mut small = RadiusRule::new(4);
        for _ in 0..100 {
            assert_eq!(small.step(1.0), None);
        }
    }

    /// A deferral extends a streak, an empty sync counts, a sync with none ends it, and a
    /// streak open at the end counts toward the longest.
    #[test]
    fn deferral_streaks_count_empty_syncs_and_an_open_end() {
        let mut d = Deferrals::default();
        assert_eq!(d.note(3, false, [0.91, 0.91, 0.6]), None);
        assert_eq!(d.note(5, true, [0.95, 0.95, 0.5]), None);
        assert_eq!(d.note(0, false, [0.8, 0.8, 0.5]), Some(2));
        assert_eq!(d.note(0, false, [0.8, 0.8, 0.5]), None);
        for _ in 0..3 {
            d.note(1, false, [0.92, 0.92, 0.7]);
        }
        assert_eq!((d.chunks, d.syncs, d.empty), (11, 5, 1));
        assert_eq!(d.low, [0.91, 0.91, 0.5]);
        assert_eq!(d.high, [0.95, 0.95, 0.7]);
        assert_eq!(
            (d.streak, d.longest),
            (3, 3),
            "the open streak is the longest"
        );
    }

    #[test]
    fn the_first_frame_waits_for_33_chunks() {
        assert_eq!(ball(NEAR_D2).len(), 33);
    }
}
