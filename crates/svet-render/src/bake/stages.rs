//! The queue step's stage timers, regimes and tallies.

use super::*;

/// The stages a queue step is timed in ([`StepStages::ms`]), in the order they run: the
/// frame's setup before the step (the light hold, the parameters, the bind group); the mirror
/// filter in five (the store following the window, the mirror's change list and the blocks of
/// chunks it does not hold full, the per-chunk sets' removals, the reach verdicts dropped
/// near a change, the changes marked for the candidate pool); the edit boxes, the withdrawal,
/// the digs' jobs, then the opening's candidate build, census, sort, reach and light checks and
/// listing (each 0 on a step that does not open), the job tables, the device poll, the
/// read-back's copy and the blocks' placement, the submit, the fades and the light summaries.
pub const STEP_STAGES: [&str; 21] = [
    "setup",
    "rewindow",
    "gone",
    "forget",
    "drop_reach",
    "touched",
    "boxes",
    "withdraw",
    "digs",
    "candidates",
    "census",
    "sort",
    "checks",
    "list",
    "refresh",
    "poll",
    "collect",
    "place",
    "submit",
    "fades",
    "summaries",
];

pub(super) const S_SETUP: usize = 0;

pub(super) const S_REWINDOW: usize = 1;

pub(super) const S_GONE: usize = 2;

pub(super) const S_FORGET: usize = 3;

pub(super) const S_DROP_REACH: usize = 4;

pub(super) const S_TOUCHED: usize = 5;

pub(super) const S_BOXES: usize = 6;

pub(super) const S_WITHDRAW: usize = 7;

pub(super) const S_DIGS: usize = 8;

pub(super) const S_CANDIDATES: usize = 9;

pub(super) const S_CENSUS: usize = 10;

pub(super) const S_SORT: usize = 11;

pub(super) const S_CHECKS: usize = 12;

pub(super) const S_LIST: usize = 13;

pub(super) const S_REFRESH: usize = 14;

pub(super) const S_POLL: usize = 15;

pub(super) const S_COLLECT: usize = 16;

pub(super) const S_PLACE: usize = 17;

pub(super) const S_SUBMIT: usize = 18;

pub(super) const S_FADES: usize = 19;

pub(super) const S_SUMMARIES: usize = 20;

/// One queue step's CPU time by stage.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StepStages {
    /// Each stage's milliseconds, [`STEP_STAGES`] in order. The stages are laps of one clock,
    /// so they cover the step from the frame's call to the last stage's end.
    pub ms: [f64; STEP_STAGES.len()],
    /// The step's own milliseconds, from the frame's call to the step's return.
    pub total_ms: f64,
    /// Whether the step ran an opening.
    pub opened: bool,
}

impl StepStages {
    /// The stages' sum.
    pub fn sum(&self) -> f64 {
        self.ms.iter().sum()
    }
}

/// The laps of a step's stages: each lap adds the time since the last to a stage.
#[derive(Clone, Copy, Debug)]
pub(super) struct Laps {
    pub(super) at: Instant,
    pub(super) ms: [f64; STEP_STAGES.len()],
}

impl Laps {
    pub(super) fn new(at: Instant) -> Self {
        Self {
            at,
            ms: [0.0; STEP_STAGES.len()],
        }
    }

    pub(super) fn lap(&mut self, stage: usize) {
        let now = stopwatch();
        self.ms[stage] += now.duration_since(self.at).as_secs_f64() * 1e3;
        self.at = now;
    }
}

/// The live queue's operating point over a second of a flight, read from the samples a chunk
/// baked that second drew: [`Regime::KeepingUp`] (B) when the coarse front keeps up, every
/// opening a small coarse job; [`Regime::Starved`] (A) when the near shell's full jobs hold the
/// loop, few chunks baked a second at many samples each; [`Regime::Mixed`] between.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Regime {
    Starved,
    KeepingUp,
    Mixed,
}

/// Samples a baked chunk under which a second reads [`Regime::KeepingUp`].
pub(crate) const REGIME_B_MAX: f64 = 3e5;

/// Samples a baked chunk over which a second reads [`Regime::Starved`].
pub(crate) const REGIME_A_MIN: f64 = 6e5;

impl Regime {
    /// A second's regime from the samples it read back and the chunks it baked; `None` for a
    /// second that read no sample (an idle queue). A second that drew samples and baked no
    /// chunk is starved.
    pub fn of(samples: u64, chunks: u64) -> Option<Self> {
        if samples == 0 {
            return None;
        }
        if chunks == 0 {
            return Some(Self::Starved);
        }
        let per = samples as f64 / chunks as f64;
        Some(if per < REGIME_B_MAX {
            Self::KeepingUp
        } else if per > REGIME_A_MIN {
            Self::Starved
        } else {
            Self::Mixed
        })
    }

    /// `A`, `B` or `mixed`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Starved => "A",
            Self::KeepingUp => "B",
            Self::Mixed => "mixed",
        }
    }
}

/// A run's queue steps, each tagged with its second's regime once the second ends: the
/// per-regime summary of the step's time, its opening share and its stages.
#[derive(Clone, Debug, Default)]
pub struct StepTally {
    /// The steps of the second not yet tagged.
    pub(super) pending: Vec<StepStages>,
    /// Tagged steps: A, B, mixed, and idle (a second that read no sample).
    pub(super) tagged: [Vec<StepStages>; 4],
}

/// The tally's buckets in order, by name.
pub(super) const TALLY_NAMES: [&str; 4] = ["A", "B", "mixed", "idle"];

pub(super) fn tally_index(r: Option<Regime>) -> usize {
    match r {
        Some(Regime::Starved) => 0,
        Some(Regime::KeepingUp) => 1,
        Some(Regime::Mixed) => 2,
        None => 3,
    }
}

/// The `p`th percentile of `v` by nearest rank (0 for none), `v` sorted in place.
pub(super) fn nearest_rank(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    let i = ((p / 100.0) * v.len() as f64).ceil() as usize;
    v[i.clamp(1, v.len()) - 1]
}

impl StepTally {
    /// A step of the current second.
    pub fn push(&mut self, s: StepStages) {
        self.pending.push(s);
    }

    /// Ends the second: its steps take `regime`.
    pub fn tag(&mut self, regime: Option<Regime>) {
        let i = tally_index(regime);
        self.tagged[i].append(&mut self.pending);
    }

    /// The steps tagged `regime`.
    pub fn count(&self, regime: Option<Regime>) -> usize {
        self.tagged[tally_index(regime)].len()
    }

    /// The step time's `p`th percentile over the steps tagged `regime`, of those that opened
    /// (`Some(true)`), did not (`Some(false)`) or all (`None`).
    pub fn percentile(&self, regime: Option<Regime>, opened: Option<bool>, p: f64) -> f64 {
        let mut v: Vec<f64> = self.tagged[tally_index(regime)]
            .iter()
            .filter(|s| opened.is_none_or(|o| s.opened == o))
            .map(|s| s.total_ms)
            .collect();
        nearest_rank(&mut v, p)
    }

    /// The summary: a line per bucket that holds a step, then the pooled line, each `queue
    /// step <bucket>: N steps, p50 X ms, p99 Y ms, opened K (share), open p50 X ms, not p50 Y
    /// ms; stages p50/p99 ms (open p50 / not p50): setup a/b (c/d), ...`.
    pub fn summary(&self) -> Vec<String> {
        let all: Vec<StepStages> = self.tagged.iter().flatten().copied().collect();
        let mut out = Vec::new();
        for (name, steps) in TALLY_NAMES.iter().zip(&self.tagged) {
            if !steps.is_empty() {
                out.push(Self::line(name, steps));
            }
        }
        if !all.is_empty() {
            out.push(Self::line("pooled", &all));
        }
        out
    }

    pub(super) fn line(name: &str, steps: &[StepStages]) -> String {
        use std::fmt::Write as _;
        let pick = |opened: Option<bool>, f: &dyn Fn(&StepStages) -> f64, p: f64| {
            let mut v: Vec<f64> = steps
                .iter()
                .filter(|s| opened.is_none_or(|o| s.opened == o))
                .map(f)
                .collect();
            nearest_rank(&mut v, p)
        };
        let opened = steps.iter().filter(|s| s.opened).count();
        let mut line = format!(
            "queue step {name}: {} steps, p50 {:.3} ms, p99 {:.3} ms, opened {opened} ({:.3}), open p50 {:.3} ms, not p50 {:.3} ms; stages p50/p99 ms (open p50 / not p50):",
            steps.len(),
            pick(None, &|s| s.total_ms, 50.0),
            pick(None, &|s| s.total_ms, 99.0),
            opened as f64 / steps.len() as f64,
            pick(Some(true), &|s| s.total_ms, 50.0),
            pick(Some(false), &|s| s.total_ms, 50.0),
        );
        for (k, stage) in STEP_STAGES.iter().enumerate() {
            let _ = write!(
                line,
                " {stage} {:.3}/{:.3} ({:.3}/{:.3})",
                pick(None, &|s| s.ms[k], 50.0),
                pick(None, &|s| s.ms[k], 99.0),
                pick(Some(true), &|s| s.ms[k], 50.0),
                pick(Some(false), &|s| s.ms[k], 50.0),
            );
        }
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Seconds of 1.4e7 samples over 190 chunks, 1.5e7 over 12 and 1.5e7 over 35 read B, A and
    /// mixed; 20 tagged steps, 12 in B and 8 in A, count 12 and 8 and take each regime's p50
    /// from its own steps alone.
    #[test]
    fn the_regime_line_reads_fixed_seconds() {
        assert_eq!(Regime::of(14_000_000, 190), Some(Regime::KeepingUp));
        assert_eq!(Regime::of(15_000_000, 12), Some(Regime::Starved));
        assert_eq!(Regime::of(15_000_000, 35), Some(Regime::Mixed));
        assert_eq!(Regime::of(0, 0), None);
        assert_eq!(Regime::of(1_000, 0), Some(Regime::Starved));
        let step = |total_ms: f64, opened: bool| StepStages {
            total_ms,
            opened,
            ..StepStages::default()
        };
        let mut t = StepTally::default();
        // B's 12 steps: 9 to 9.11 ms, all opening but the last two at 1 ms.
        for i in 0..12 {
            let open = i < 10;
            t.push(step(
                if open { 9.0 + f64::from(i) * 0.01 } else { 1.0 },
                open,
            ));
        }
        t.tag(Some(Regime::KeepingUp));
        // A's 8 steps: 0.5 to 0.57 ms, none opening.
        for i in 0..8 {
            t.push(step(0.5 + f64::from(i) * 0.01, false));
        }
        t.tag(Some(Regime::Starved));
        assert_eq!(t.count(Some(Regime::KeepingUp)), 12);
        assert_eq!(t.count(Some(Regime::Starved)), 8);
        assert_eq!(t.count(Some(Regime::Mixed)), 0);
        // Nearest rank: the 6th of B's 12 sorted (1, 1, 9.00 .. 9.09) is 9.03; of A's 8 the
        // 4th, 0.53.
        assert!((t.percentile(Some(Regime::KeepingUp), None, 50.0) - 9.03).abs() < 1e-9);
        assert!((t.percentile(Some(Regime::Starved), None, 50.0) - 0.53).abs() < 1e-9);
        assert!((t.percentile(Some(Regime::KeepingUp), Some(false), 50.0) - 1.0).abs() < 1e-9);
        let lines = t.summary();
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(
            lines[0].starts_with("queue step A: 8 steps, p50 0.530 ms,"),
            "{}",
            lines[0]
        );
        assert!(
            lines[1].starts_with(
                "queue step B: 12 steps, p50 9.030 ms, p99 9.090 ms, opened 10 (0.833), open p50 9.040 ms, not p50 1.000 ms;"
            ),
            "{}",
            lines[1]
        );
        assert!(
            lines[2].starts_with("queue step pooled: 20 steps,"),
            "{}",
            lines[2]
        );
    }
}
