//! `--settled-start` (headless only): a start-up stage after the light pre-roll that holds
//! frame 0 back until the world no longer changes under a still camera. It steps the stream
//! (update, poll, apply, sync) until full residency (nothing queued or in flight on the
//! streamer, nothing pending on the mirror), then the stream and the live queue in lockstep
//! until the queue reads idle ([`drain_idle`]: no job open or pending, no coarse block left to
//! refine) on [`IDLE_STEPS`] steps in a row with residency still full. It ends by that state,
//! never by wall time; at [`SETTLE_STEPS_MAX`] steps it records the session's failure, so the
//! run exits non-zero and no unsettled frame is drawn. The queue bakes at the drain's
//! [`PRE_ROLL_DRAIN_MS`] a step while it settles, then the run's budget returns.

use crate::app::ms_since;
use crate::edits::voxel_of;
use crate::now;
use crate::run::Run;
use crate::run::queue::{PRE_ROLL_DRAIN_MS, drain_idle};
use std::time::{Duration, Instant};
use svet_app::Gpu;
use svet_render::wgpu;

/// Steps the settle takes at most, residency's and the queue's together: a bound on a queue
/// that never empties, not a reading of how long a settle takes.
pub(crate) const SETTLE_STEPS_MAX: u64 = 50_000;

/// Queue steps in a row that must read idle, residency full on each: a step that opens no job
/// may be followed by one that lists a chunk the last sync made ready.
pub(crate) const IDLE_STEPS: u32 = 3;

/// The settle between steps.
#[derive(Debug, Default)]
pub(crate) struct Settle {
    /// Steps while residency was not full, and steps of the queue after it was.
    residency_steps: u64,
    queue_steps: u64,
    /// Queue steps in a row read idle.
    idle_run: u32,
    began: Option<Instant>,
}

impl Settle {
    fn steps(&self) -> u64 {
        self.residency_steps + self.queue_steps
    }
}

/// Whether the settle ends after a queue step: `idle` (the step's queue read idle, or the run
/// has no queue) on [`IDLE_STEPS`] steps in a row, counted in `run`.
pub(crate) fn settled_after(run: &mut u32, idle: bool) -> bool {
    *run = if idle { *run + 1 } else { 0 };
    *run >= IDLE_STEPS
}

impl Run {
    /// Whether the streamed world is fully resident now (always on the bounded level, whose
    /// upload ended before the pre-roll).
    fn resident_now(&self) -> bool {
        let (Some(stream), Some(gfx)) = (self.stream.as_ref(), self.gfx.as_ref()) else {
            return true;
        };
        stream
            .stats
            .is_some_and(|st| st.in_flight == 0 && st.queued == 0)
            && gfx.mirror.pending_chunks() == 0
    }

    /// One step of the settle; `true` once it has ended, settled (the `settled:` line printed)
    /// or failed (the failure recorded).
    pub(crate) fn settle_step(&mut self, gpu: &Gpu, s: &mut Settle) -> bool {
        let began = *s.began.get_or_insert_with(|| {
            if let Some(gfx) = self.gfx.as_mut() {
                gfx.frame.set_bake_budget(svet_render::Budget::PerFrame {
                    ms: PRE_ROLL_DRAIN_MS,
                });
            }
            self.queue.settling = true;
            now()
        });
        if s.steps() >= SETTLE_STEPS_MAX {
            let line = format!(
                "settled start: not settled after {} steps ({} to residency, {} of the queue), {:.0} ms: {}",
                s.steps(),
                s.residency_steps,
                s.queue_steps,
                ms_since(began),
                self.settle_counts()
            );
            println!("{line}");
            self.failure.set(line);
            return self.end_settle();
        }
        let mut enc = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        if self.stream.is_some() {
            if let Err(e) = self.stream_step(gpu, &mut enc) {
                self.failure.set(format!("mirror sync failed: {e}"));
                return self.end_settle();
            }
        } else if let Some(gfx) = self.gfx.as_mut() {
            gfx.mirror.set_focus(voxel_of(self.fly.camera.position));
            if let Err(e) = gfx.mirror.sync(&self.world, &gpu.queue, &mut enc) {
                self.failure.set(format!("mirror sync failed: {e}"));
                return self.end_settle();
            }
        }
        if !self.resident_now() {
            gpu.queue.submit([enc.finish()]);
            s.residency_steps += 1;
            s.idle_run = 0;
            // The streamer's workers run meanwhile; the step yields to them as the near
            // residency's does.
            std::thread::sleep(Duration::from_millis(1));
            return false;
        }
        let enc = self.step_bake_queue(gpu, enc);
        gpu.queue.submit([enc.finish()]);
        s.queue_steps += 1;
        if self.queue.failed {
            self.failure
                .set("settled start: the bake queue failed".to_owned());
            return self.end_settle();
        }
        let has_queue = self
            .gfx
            .as_ref()
            .is_some_and(|g| g.frame.bake_queue().is_some());
        let idle = !has_queue || self.queue.stats.is_some_and(|st| drain_idle(&st));
        if !settled_after(&mut s.idle_run, idle && self.resident_now()) {
            return false;
        }
        println!(
            "settled: {} after {} steps ({} to residency, {} of the queue), {:.0} ms",
            self.settle_counts(),
            s.steps(),
            s.residency_steps,
            s.queue_steps,
            ms_since(began)
        );
        self.end_settle()
    }

    /// The settle's counts: the streamer's full and LOD-only chunks (the bounded level's
    /// mirrored chunks), the queue's jobs, chunks to refine and provisional chunks.
    fn settle_counts(&self) -> String {
        let (full, lod) = match (self.stream.as_ref(), self.gfx.as_ref()) {
            (Some(stream), _) => stream
                .stats
                .map_or((0, 0), |st| (st.resident_full, st.resident_lod)),
            (None, _) => (self.world.stats().chunks, 0),
        };
        let st = self.queue.stats.unwrap_or_default();
        format!(
            "{full} full, {lod} lod chunks, {} jobs, {} to refine, {} provisional",
            st.jobs, st.coarse, st.provisional
        )
    }

    /// The run's own bake budget back and the queue's lines on; `true` (the settle has ended).
    fn end_settle(&mut self) -> bool {
        if let Some(gfx) = self.gfx.as_mut() {
            gfx.frame.set_bake_budget(self.args.bake.budget);
        }
        self.queue.settling = false;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{IDLE_STEPS, settled_after};

    /// The settle ends on the third idle step in a row; a busy step starts the count again.
    #[test]
    fn the_settle_ends_after_idle_steps_in_a_row() {
        assert_eq!(IDLE_STEPS, 3);
        let mut run = 0;
        assert!(!settled_after(&mut run, true));
        assert!(!settled_after(&mut run, true));
        assert!(!settled_after(&mut run, false));
        assert_eq!(run, 0);
        assert!(!settled_after(&mut run, true));
        assert!(!settled_after(&mut run, true));
        assert!(settled_after(&mut run, true));
    }
}
