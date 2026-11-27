//! The light pre-roll as a steppable object (ADR-0059): the live queue stepped with a fixed
//! sample count until the chunks near the camera hold their light and the coarse pass has
//! covered the full chunks mirrored at that moment, boxed past the near chunks and capped. The
//! box and the cap read the work time the caller passes (its summed step time), never a clock
//! of their own, so the time between a sliced pre-roll's slices is not counted. A step's own
//! time, and the step at which a read-back is read, are the machine's: where the box or the cap
//! can end it, or a read-back can lag, the step it ends at is too. A caller that waits on the
//! device before each step and passes no box and no cap ends at the same step on any machine.

use svet_world::{ChunkCoord, World};

use crate::bake::{BakeError, QueueView};
use crate::frame::Frame;
use crate::gpu_world::GpuWorld;

/// The pre-roll's settings.
#[derive(Clone, Debug, PartialEq)]
pub struct PreRollConfig {
    /// The chunks that must hold their light before the coarse pass starts.
    pub near: Vec<ChunkCoord>,
    /// The coarse pass's sample count; 0 ends the pre-roll at the near chunks.
    pub coarse: u32,
    /// Work time the coarse pass runs at most past the near chunks, in ms.
    pub box_ms: f64,
    /// Work time the pre-roll runs at most, in ms.
    pub cap_ms: f64,
    /// Samples one step draws ([`QueueView::samples`]).
    pub samples: u64,
}

/// Where the pre-roll stands after a step.
#[derive(Clone, Debug, PartialEq)]
pub enum PreRollState {
    /// `settled` of the `of` near chunks hold their light.
    Near { settled: usize, of: usize },
    /// `passed` of the `of` chunks the coarse pass covers have had it.
    Coarse { passed: usize, of: usize },
    /// The pre-roll has ended.
    Done(PreRollSummary),
}

/// How the pre-roll ended: the work time, the chunks baked and the samples at the near chunks'
/// moment and at the end, the coarse pass's chunks, and whether the box or the cap ended it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PreRollSummary {
    pub steps: u32,
    pub near_ms: f64,
    pub near_baked: u64,
    pub near_samples: u64,
    pub total_ms: f64,
    pub baked: u64,
    pub samples: u64,
    pub coarse_chunks: usize,
    pub boxed: bool,
    pub capped: bool,
}

/// The pre-roll's state between steps; see the module's doc.
#[derive(Clone, Debug)]
pub struct PreRoll {
    cfg: PreRollConfig,
    steps: u32,
    /// The near chunks' moment: the work time, the chunks baked and the samples then.
    near_done: Option<(f64, u64, u64)>,
    /// The full chunks mirrored at the near chunks' moment.
    first_pass: Vec<ChunkCoord>,
    state: PreRollState,
}

impl PreRoll {
    pub fn new(cfg: PreRollConfig) -> Self {
        let of = cfg.near.len();
        Self {
            cfg,
            steps: 0,
            near_done: None,
            first_pass: Vec::new(),
            state: PreRollState::Near { settled: 0, of },
        }
    }

    pub fn config(&self) -> &PreRollConfig {
        &self.cfg
    }

    /// The state after the last step.
    pub fn state(&self) -> &PreRollState {
        &self.state
    }

    /// The work time at which the near chunks settled; `None` before.
    pub fn near_ms(&self) -> Option<f64> {
        self.near_done.map(|(ms, _, _)| ms)
    }

    /// Whether the pre-roll has ended.
    pub fn done(&self) -> bool {
        matches!(self.state, PreRollState::Done(_))
    }

    /// One step: the mirror's sync submitted in an encoder of its own, then one live-queue
    /// step of [`PreRollConfig::samples`] along `view` (its `samples` is replaced), then the
    /// decision on `work_ms`, the caller's work time spent on the pre-roll before this step.
    /// A near chunk counts as settled by [`Frame::bake_held_for_start`]: a block at the full
    /// count does, a catch-up due or in a job on it or not. A near chunk outside a bounded world
    /// (`view.resident` is `None`) that holds no chunk counts as settled. A step after the end does nothing and returns the end.
    #[allow(
        clippy::too_many_arguments,
        reason = "the frame, the mirror, the world and the view the step reads, as the queue's own step takes them"
    )]
    pub fn step(
        &mut self,
        frame: &mut Frame,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mirror: &mut GpuWorld,
        world: &World,
        view: &QueueView,
        work_ms: f64,
    ) -> Result<PreRollState, PreRollError> {
        if self.done() {
            return Ok(self.state.clone());
        }
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("pre-roll sync"),
        });
        mirror
            .sync(world, queue, &mut enc)
            .map_err(|e| PreRollError::Sync(e.to_string()))?;
        queue.submit([enc.finish()]);
        let view = QueueView {
            eye: view.eye,
            forward: view.forward,
            resident: view.resident.clone(),
            samples: Some(self.cfg.samples),
            interval_s: view.interval_s,
            hold: view.hold,
            half_angle: view.half_angle,
            frustum: view.frustum,
            full_radius: view.full_radius,
            lod_radius: view.lod_radius,
            focal_px: view.focal_px,
        };
        if let Some(Err(e)) = frame.bake_queue_step(device, queue, mirror, world, &view) {
            return Err(PreRollError::Bake(e));
        }
        let bounded = view.resident.is_none();
        let settled = self
            .cfg
            .near
            .iter()
            .filter(|&&c| frame.bake_held_for_start(c, bounded && world.store().get(c).is_none()))
            .count();
        let st = frame.bake_queue().map(|q| q.stats()).unwrap_or_default();
        Ok(self.decide(
            work_ms,
            settled,
            (st.baked, st.total_samples),
            || mirror.full_chunks().collect(),
            |chunks| chunks.iter().filter(|&&c| frame.bake_passed(&[c])).count(),
        ))
    }

    /// The decision after a step, a function of its arguments: `settled` near chunks, the
    /// queue's chunks baked and samples so far, the full chunks mirrored (read once, at the near
    /// chunks' moment) and how many of them have passed. Boxes the coarse pass once `work_ms` is
    /// over [`PreRollConfig::box_ms`] past the near chunks' moment, and caps the pre-roll at
    /// [`PreRollConfig::cap_ms`].
    pub fn decide(
        &mut self,
        work_ms: f64,
        settled: usize,
        (baked, samples): (u64, u64),
        full_chunks: impl FnOnce() -> Vec<ChunkCoord>,
        passed: impl FnOnce(&[ChunkCoord]) -> usize,
    ) -> PreRollState {
        if self.done() {
            return self.state.clone();
        }
        self.steps += 1;
        let of = self.cfg.near.len();
        if self.near_done.is_none() && settled >= of {
            self.near_done = Some((work_ms, baked, samples));
            self.first_pass = full_chunks();
        }
        let mut end = None;
        self.state = match self.near_done {
            None => PreRollState::Near { settled, of },
            Some((near_ms, _, _)) => {
                let passed = if self.cfg.coarse == 0 {
                    self.first_pass.len()
                } else {
                    passed(&self.first_pass)
                };
                if passed >= self.first_pass.len() {
                    end = Some(false);
                } else if work_ms - near_ms > self.cfg.box_ms {
                    end = Some(true);
                }
                PreRollState::Coarse {
                    passed,
                    of: self.first_pass.len(),
                }
            }
        };
        let capped = end.is_none() && work_ms > self.cfg.cap_ms;
        if end.is_some() || capped {
            let (near_ms, near_baked, near_samples) =
                self.near_done.unwrap_or((work_ms, baked, samples));
            self.state = PreRollState::Done(PreRollSummary {
                steps: self.steps,
                near_ms,
                near_baked,
                near_samples,
                total_ms: work_ms,
                baked,
                samples,
                coarse_chunks: self.first_pass.len(),
                boxed: end == Some(true),
                capped,
            });
        }
        self.state.clone()
    }
}

/// Why a pre-roll step failed.
#[derive(Debug)]
pub enum PreRollError {
    /// The mirror's sync failed; the message names why.
    Sync(String),
    /// The live queue's step failed.
    Bake(BakeError),
}

impl std::fmt::Display for PreRollError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PreRollError::Sync(e) => write!(f, "mirror sync failed: {e}"),
            PreRollError::Bake(e) => write!(f, "bake queue: {e}"),
        }
    }
}

impl std::error::Error for PreRollError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn pre_roll(near: usize, coarse: u32) -> PreRoll {
        PreRoll::new(PreRollConfig {
            near: (0..near as i32).map(|x| ChunkCoord([x, 0, 0])).collect(),
            coarse,
            box_ms: 5000.0,
            cap_ms: 120_000.0,
            samples: 1,
        })
    }

    fn chunks(n: i32) -> Vec<ChunkCoord> {
        (0..n).map(|x| ChunkCoord([x, 9, 0])).collect()
    }

    /// The near chunks settle at work 1000 ms; the coarse pass never passes. The pre-roll is
    /// in the coarse pass at 6000 ms of work (5000 past the near chunks, not over) and boxed
    /// at 6000.5, however the calls are spaced in wall time (here none); a pre-roll whose near
    /// chunks never settle is capped just past 120 000 ms, not at it.
    #[test]
    fn the_box_and_the_cap_count_work_time_only() {
        let mut p = pre_roll(2, 16);
        assert_eq!(
            p.decide(500.0, 1, (1, 10), || unreachable!(), |_| unreachable!()),
            PreRollState::Near { settled: 1, of: 2 }
        );
        assert_eq!(
            p.decide(1000.0, 2, (2, 20), || chunks(4), |_| 1),
            PreRollState::Coarse { passed: 1, of: 4 }
        );
        assert_eq!(
            p.decide(6000.0, 2, (3, 30), || unreachable!(), |_| 3),
            PreRollState::Coarse { passed: 3, of: 4 }
        );
        let PreRollState::Done(s) = p.decide(6000.5, 2, (4, 40), || unreachable!(), |_| 3) else {
            panic!("boxed past 5000 ms of work after the near chunks");
        };
        assert_eq!(
            s,
            PreRollSummary {
                steps: 4,
                near_ms: 1000.0,
                near_baked: 2,
                near_samples: 20,
                total_ms: 6000.5,
                baked: 4,
                samples: 40,
                coarse_chunks: 4,
                boxed: true,
                capped: false,
            }
        );
        assert!(p.done());

        let mut p = pre_roll(2, 16);
        assert!(matches!(
            p.decide(120_000.0, 0, (0, 0), Vec::new, |_| 0),
            PreRollState::Near { .. }
        ));
        let PreRollState::Done(s) = p.decide(120_000.1, 0, (0, 0), Vec::new, |_| 0) else {
            panic!("capped past 120 s of work");
        };
        assert!(s.capped && !s.boxed);
    }

    /// A box of 0 ms (`--pre-roll near`): a near list of 2 is `Near` until it settles,
    /// `Coarse` on that step (0 ms past the near chunks' moment is not over the box), and
    /// `Done` on the next, boxed, with the cover of 4 unpassed as its `coarse_chunks` and
    /// `steps` the near step plus 1. An empty near list settles at step 1 and ends at step 2
    /// with `near_baked` 0.
    #[test]
    fn a_zero_box_ends_the_pre_roll_the_step_after_the_near_moment() {
        let mut p = pre_roll(2, 16);
        p.cfg.box_ms = 0.0;
        assert_eq!(
            p.decide(40.0, 0, (0, 10), || unreachable!(), |_| unreachable!()),
            PreRollState::Near { settled: 0, of: 2 }
        );
        assert_eq!(
            p.decide(80.0, 1, (1, 20), || unreachable!(), |_| unreachable!()),
            PreRollState::Near { settled: 1, of: 2 }
        );
        assert_eq!(
            p.decide(120.0, 2, (2, 30), || chunks(4), |_| 0),
            PreRollState::Coarse { passed: 0, of: 4 }
        );
        let PreRollState::Done(s) = p.decide(160.0, 2, (3, 40), || unreachable!(), |_| 0) else {
            panic!("a 0 ms box ends the pre-roll the step after the near chunks' moment");
        };
        assert_eq!(
            s,
            PreRollSummary {
                steps: 4,
                near_ms: 120.0,
                near_baked: 2,
                near_samples: 30,
                total_ms: 160.0,
                baked: 3,
                samples: 40,
                coarse_chunks: 4,
                boxed: true,
                capped: false,
            }
        );

        let mut p = pre_roll(0, 16);
        p.cfg.box_ms = 0.0;
        assert_eq!(
            p.decide(40.0, 0, (0, 10), || chunks(4), |_| 0),
            PreRollState::Coarse { passed: 0, of: 4 }
        );
        let PreRollState::Done(s) = p.decide(80.0, 0, (1, 20), || unreachable!(), |_| 0) else {
            panic!("an empty near list with a 0 ms box ends at step 2");
        };
        assert_eq!((s.steps, s.near_baked, s.coarse_chunks), (2, 0, 4));
        assert!(s.boxed && !s.capped);
    }

    /// The coarse pass ends the pre-roll once every chunk it covers has passed; with coarse 0
    /// it ends at the near chunks.
    #[test]
    fn the_pre_roll_ends_when_the_coarse_pass_has_passed() {
        let mut p = pre_roll(1, 16);
        assert!(matches!(
            p.decide(10.0, 1, (1, 1), || chunks(2), |_| 2),
            PreRollState::Done(PreRollSummary { boxed: false, .. })
        ));
        let mut p = pre_roll(1, 0);
        assert!(matches!(
            p.decide(10.0, 1, (1, 1), || chunks(2), |_| unreachable!()),
            PreRollState::Done(PreRollSummary {
                coarse_chunks: 2,
                ..
            })
        ));
    }
}
