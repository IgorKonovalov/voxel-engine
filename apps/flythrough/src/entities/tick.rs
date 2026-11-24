//! The movers' tick: a 60 Hz accumulator over the frames' time, at most
//! [`MAX_TICKS_PER_FRAME`] ticks a frame (past that the sim falls behind the clock rather than
//! spiral), that ticks every placed entity's mover against the world, wander on or off; the
//! fraction of a tick left over, for drawing between the last two ticks; and the CPU each tick
//! took over the last second, for the HUD's `tick <us>`.

use std::collections::VecDeque;

use svet_entity::EntityId;
use svet_world::World;

use super::Walkers;
use super::mover::{MoverStep, TICK_HZ};
use crate::percentile;

/// At most this many ticks catch up in one frame, as walk mode's (`crate::fly`).
pub const MAX_TICKS_PER_FRAME: u32 = 8;

/// The accumulator and the last second's tick times.
#[derive(Clone, Debug, Default)]
pub struct Ticker {
    /// Seconds not yet spent on ticks.
    pending: f64,
    /// The last [`TICK_HZ`] ticks' CPU, in microseconds.
    times_us: VecDeque<f64>,
}

impl Ticker {
    /// The ticks `dt` more seconds owe, at most [`MAX_TICKS_PER_FRAME`] (the rest dropped).
    pub fn owe(&mut self, dt: f32) -> u32 {
        let step = 1.0 / f64::from(TICK_HZ);
        self.pending += f64::from(dt);
        // A frame of exactly one tick's time ticks once despite the sum's rounding.
        let n = ((self.pending + 1e-9) / step).floor() as u32;
        if n > MAX_TICKS_PER_FRAME {
            self.pending = 0.0;
            return MAX_TICKS_PER_FRAME;
        }
        self.pending = (self.pending - f64::from(n) * step).max(0.0);
        n
    }

    /// The fraction of a tick the clock is past the last tick, in `[0, 1)`.
    pub fn alpha(&self) -> f32 {
        (self.pending * f64::from(TICK_HZ)).clamp(0.0, 1.0) as f32
    }

    /// Records one tick's CPU, keeping the last second's.
    pub fn record(&mut self, us: f64) {
        if self.times_us.len() == TICK_HZ as usize {
            self.times_us.pop_front();
        }
        self.times_us.push_back(us);
    }

    /// The median of the last second's tick times, in microseconds; `None` with none.
    pub fn p50_us(&self) -> Option<f64> {
        if self.times_us.is_empty() {
            return None;
        }
        let mut v: Vec<f64> = self.times_us.iter().copied().collect();
        Some(percentile(&mut v, 50.0))
    }
}

impl Walkers {
    /// Runs the movers' ticks `dt` seconds owe ([`Ticker::owe`]); with no mover placed the
    /// clock runs on and no tick is timed.
    pub fn tick_movers(&mut self, dt: f32, world: &World) {
        let n = self.ticker.owe(dt);
        if !self.placed.iter().any(|p| p.mover.is_some()) {
            return;
        }
        for _ in 0..n {
            let t = crate::now();
            self.tick(world);
            self.ticker.record(crate::app::ms_since(t) * 1e3);
        }
    }

    /// One tick of every placed entity's mover (the ghost's never), wander on or off, each
    /// posed in the set: what each did, in placement order.
    pub fn tick(&mut self, world: &World) -> Vec<(EntityId, MoverStep)> {
        let mut out = Vec::new();
        for p in &mut self.placed {
            let Some(m) = p.mover.as_mut() else { continue };
            let Some(l) = p
                .entry
                .and_then(|i| self.library.entries.get(i))
                .and_then(|e| e.viewer())
                .and_then(|v| v.loaded.as_ref())
            else {
                continue;
            };
            out.push((p.id, m.tick(world, self.walking, (l, &p.play))));
            // The pose is built on the entry's model, so it fits.
            let _ = self.set.set_pose(p.id, m.now.clone());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Frames of one tick's time tick once each; a long frame ticks at most eight and drops
    /// the rest; the fraction left reads between 0 and 1; the p50 is the last second's.
    #[test]
    fn the_accumulator_ticks_at_sixty_and_caps() {
        let mut t = Ticker::default();
        let ticks: Vec<u32> = (0..600).map(|_| t.owe(1.0 / 60.0)).collect();
        assert!(ticks.iter().all(|&n| n == 1), "{ticks:?}");
        assert_eq!(t.owe(1.0), MAX_TICKS_PER_FRAME);
        assert_eq!(t.alpha(), 0.0);
        assert_eq!(t.owe(1.0 / 120.0), 0);
        assert!((t.alpha() - 0.5).abs() < 1e-4);
        assert_eq!(t.p50_us(), None);
        for us in 0..120 {
            t.record(f64::from(us));
        }
        // The last 60 of 0..120 are 60..119; their nearest-rank median is the 30th, 89.
        assert_eq!(t.p50_us(), Some(89.0));
    }

    /// Plan 0079 Phase 4 done-when 6: once a mover has ticked the entities row gains `tick
    /// <us>` (the last second's median, rounded), then `wander` while the wander is on.
    #[test]
    fn the_row_reads_the_tick_and_the_wander() {
        let mut w = Walkers::default();
        let row = |w: &Walkers| {
            let mut s = String::new();
            w.write_row(&mut s);
            s
        };
        assert_eq!(row(&w), "entities 0/64 parts 0/512 over 0");
        for us in [11.0, 12.4, 30.0] {
            w.ticker.record(us);
        }
        assert_eq!(row(&w), "entities 0/64 parts 0/512 over 0 tick 12");
        assert_eq!(w.toggle_walk(), "wander on");
        assert_eq!(row(&w), "entities 0/64 parts 0/512 over 0 tick 12 wander");
    }
}
