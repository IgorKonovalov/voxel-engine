//! The queue step's stage timers, regimes and tallies.

use super::*;

/// The stages a queue step is timed in ([`StepStages::ms`]), in the order they run: the
/// frame's setup before the step (the light hold, the parameters, the bind group); the mirror
/// filter in five (the store following the window, the mirror's change list and the blocks of
/// chunks it does not hold full, the per-chunk sets' removals, the reach verdicts dropped
/// near a change, the changes marked for the candidate pool); the edit boxes, the withdrawal,
/// the digs' jobs, then the opening's candidate build, census, sort, reach and light checks and
/// listing (each 0 on a step that does not open), the job tables, the device poll, the
/// read-back's copy and the blocks' placement, the submit, the fades and the light summaries, and the render half's apply of the step's
/// write lists (ADR-0083).
pub const STEP_STAGES: [&str; 22] = [
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
    "apply",
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

pub(crate) const S_APPLY: usize = 21;

/// The buffers a step's writes are counted by ([`StepStages::bytes`]): the pool's block words
/// (a block, a fade's shown words, a dig's sky-stale marks), the chunk table's words (a block's
/// table word, a removal's zero, the header and table a window move rewrites), the summaries'
/// parent records, and the job's inputs (the job uniform, the faces, the candidate tables, the
/// parameters).
pub const WRITE_KINDS: [&str; 4] = ["pool", "table", "summary", "job"];

pub(crate) const W_POOL: usize = 0;

pub(crate) const W_TABLE: usize = 1;

pub(crate) const W_SUMMARY: usize = 2;

pub(crate) const W_JOB: usize = 3;

/// The calling thread's time in `wgpu` calls and its bytes written, since the thread began:
/// what ADR-0083 leaves on the render thread (the writes, the encode, the submit, the poll,
/// the mapped read and unmap). A step reads it at each lap ([`Laps::lap`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct CallTally {
    pub(crate) ms: f64,
    pub(crate) bytes: [u64; WRITE_KINDS.len()],
}

thread_local! {
    static CALLS: std::cell::Cell<CallTally> = const {
        std::cell::Cell::new(CallTally { ms: 0.0, bytes: [0; WRITE_KINDS.len()] })
    };
}

/// The calling thread's [`CallTally`] now.
pub(crate) fn calls() -> CallTally {
    CALLS.with(std::cell::Cell::get)
}

/// Runs `f`, a run of `wgpu` calls, and adds its time to the thread's [`CallTally`].
pub(crate) fn wgpu_call<R>(f: impl FnOnce() -> R) -> R {
    let t = stopwatch();
    let r = f();
    add_call_ms(stopwatch().duration_since(t).as_secs_f64() * 1e3);
    r
}

/// Adds `ms` of `wgpu` calls timed by the caller to the thread's [`CallTally`].
pub(crate) fn add_call_ms(ms: f64) {
    CALLS.with(|c| {
        let mut v = c.get();
        v.ms += ms;
        c.set(v);
    });
}

/// `queue.write_buffer(buffer, offset, data)`, timed as a `wgpu` call and its bytes counted
/// under `kind` ([`WRITE_KINDS`]).
pub(crate) fn write(
    queue: &wgpu::Queue,
    buffer: &wgpu::Buffer,
    offset: u64,
    data: &[u8],
    kind: usize,
) {
    wgpu_call(|| queue.write_buffer(buffer, offset, data));
    CALLS.with(|c| {
        let mut v = c.get();
        v.bytes[kind] += data.len() as u64;
        c.set(v);
    });
}

/// Where a step starts: the frame's call ([`Laps::new`]), with the thread's [`CallTally`] then.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StepStart {
    pub(crate) at: Instant,
    pub(crate) calls: CallTally,
}

impl StepStart {
    pub(crate) fn now() -> Self {
        Self {
            at: stopwatch(),
            calls: calls(),
        }
    }
}

/// One queue step's CPU time by stage.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StepStages {
    /// Each stage's milliseconds, [`STEP_STAGES`] in order. The stages are laps of one clock,
    /// so they cover the step from the frame's call to the last stage's end.
    pub ms: [f64; STEP_STAGES.len()],
    /// Of each stage's milliseconds, those spent in `wgpu` calls: what stays on the render
    /// thread (ADR-0083). At most the stage's own.
    pub render_ms: [f64; STEP_STAGES.len()],
    /// The bytes the step wrote, by [`WRITE_KINDS`].
    pub bytes: [u64; WRITE_KINDS.len()],
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

    /// The stages' time in `wgpu` calls: the render thread's share.
    pub fn render(&self) -> f64 {
        self.render_ms.iter().sum()
    }

    /// The stages' time outside `wgpu` calls: what can leave the render thread.
    pub fn movable(&self) -> f64 {
        self.ms
            .iter()
            .zip(&self.render_ms)
            .map(|(m, r)| m - r)
            .sum()
    }

    /// The bytes the step wrote, every buffer.
    pub fn bytes_total(&self) -> u64 {
        self.bytes.iter().sum()
    }

    /// The step's split as the `bake queue step:` line prints it over `steps`: `render p50/p99
    /// a/b ms, movable p50/p99 c/d ms (means e + f = g ms, stages h ms), bytes a step p50/p99
    /// i/j (pool k, table l, summary m, job n mean)`. The means' sum equals the stages' mean.
    pub fn split_text(steps: &[StepStages]) -> String {
        let pick = |f: &dyn Fn(&StepStages) -> f64, p: f64| {
            let mut v: Vec<f64> = steps.iter().map(f).collect();
            nearest_rank(&mut v, p)
        };
        let n = steps.len().max(1) as f64;
        let mean = |f: &dyn Fn(&StepStages) -> f64| steps.iter().map(f).sum::<f64>() / n;
        let kinds: Vec<String> = WRITE_KINDS
            .iter()
            .enumerate()
            .map(|(k, name)| format!("{name} {:.0}", mean(&|s| s.bytes[k] as f64)))
            .collect();
        format!(
            "render p50/p99 {:.3}/{:.3} ms, movable p50/p99 {:.3}/{:.3} ms (means {:.3} + {:.3} = {:.3} ms, stages {:.3} ms), bytes a step p50/p99 {:.0}/{:.0} ({} mean)",
            pick(&StepStages::render, 50.0),
            pick(&StepStages::render, 99.0),
            pick(&StepStages::movable, 50.0),
            pick(&StepStages::movable, 99.0),
            mean(&StepStages::render),
            mean(&StepStages::movable),
            mean(&StepStages::render) + mean(&StepStages::movable),
            mean(&StepStages::sum),
            pick(&|s| s.bytes_total() as f64, 50.0),
            pick(&|s| s.bytes_total() as f64, 99.0),
            kinds.join(", "),
        )
    }
}

/// The laps of a step's stages: each lap adds the time since the last to a stage, and the
/// thread's `wgpu` calls since the last to the stage's render share.
#[derive(Clone, Copy, Debug)]
pub(super) struct Laps {
    pub(super) at: Instant,
    pub(super) ms: [f64; STEP_STAGES.len()],
    pub(super) render: [f64; STEP_STAGES.len()],
    /// The thread's tally at the last lap, and at the step's start.
    calls: CallTally,
    start: CallTally,
}

impl Laps {
    pub(super) fn new(start: StepStart) -> Self {
        Self {
            at: start.at,
            ms: [0.0; STEP_STAGES.len()],
            render: [0.0; STEP_STAGES.len()],
            calls: start.calls,
            start: start.calls,
        }
    }

    pub(super) fn lap(&mut self, stage: usize) {
        let now = stopwatch();
        let c = calls();
        self.ms[stage] += now.duration_since(self.at).as_secs_f64() * 1e3;
        self.render[stage] += c.ms - self.calls.ms;
        self.at = now;
        self.calls = c;
    }

    /// Moves `ms` of `from`'s time, `render` of it in `wgpu` calls, to stage `to`, each
    /// clamped to what `from` holds; returns what moved.
    pub(super) fn shift(&mut self, from: usize, to: usize, ms: f64, render: f64) -> (f64, f64) {
        let m = ms.min(self.ms[from]).max(0.0);
        let r = render.min(self.render[from]).min(m).max(0.0);
        self.ms[from] -= m;
        self.ms[to] += m;
        self.render[from] -= r;
        self.render[to] += r;
        (m, r)
    }

    /// The step's bytes written since its start, by [`WRITE_KINDS`].
    pub(super) fn bytes(&self) -> [u64; WRITE_KINDS.len()] {
        std::array::from_fn(|k| self.calls.bytes[k] - self.start.bytes[k])
    }

    /// Each stage's render share, clamped to the stage's own time (the clock reads around a
    /// call's own reads can put a hair more in a call than in its lap).
    pub(super) fn render_ms(&self) -> [f64; STEP_STAGES.len()] {
        std::array::from_fn(|k| self.render[k].min(self.ms[k]).max(0.0))
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
            "queue step {name}: {} steps, p50 {:.3} ms, p99 {:.3} ms, opened {opened} ({:.3}), open p50 {:.3} ms, not p50 {:.3} ms; {}; stages p50/p99 ms (open p50 / not p50):",
            steps.len(),
            pick(None, &|s| s.total_ms, 50.0),
            pick(None, &|s| s.total_ms, 99.0),
            opened as f64 / steps.len() as f64,
            pick(Some(true), &|s| s.total_ms, 50.0),
            pick(Some(false), &|s| s.total_ms, 50.0),
            StepStages::split_text(steps),
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

    /// A step of known laps: 22 stages of `k + 0.1` ms, each with `k * 0.03` ms in `wgpu`
    /// calls (at most its own), 3 000 bytes over the four kinds. Its line's `render` and
    /// `movable` sums (the means of one step) equal the stages' sum within 0.001 ms, and its
    /// p50s read the step's own split; two steps' means sum to the stages' mean.
    #[test]
    fn the_step_line_splits_render_and_movable_and_they_sum_to_the_step() {
        let mut s = StepStages::default();
        for k in 0..STEP_STAGES.len() {
            s.ms[k] = k as f64 + 0.1;
            s.render_ms[k] = k as f64 * 0.03;
        }
        s.bytes = [2048, 512, 256, 184];
        s.total_ms = s.sum() + 0.5;
        // 22 stages: the sum of k is 231, so the stages' sum is 231 + 22 x 0.1 = 233.2 and the
        // render share 231 x 0.03 = 6.93.
        assert!((s.sum() - 233.2).abs() < 1e-9);
        assert!((s.render() - 6.93).abs() < 1e-9);
        assert!((s.render() + s.movable() - s.sum()).abs() < 1e-9);
        let num = |text: &str, key: &str| -> f64 {
            let at = text.find(key).unwrap_or_else(|| panic!("{key} in {text}")) + key.len();
            text[at..]
                .split([' ', '/', ','])
                .next()
                .and_then(|v| v.parse().ok())
                .unwrap_or_else(|| panic!("a number after {key} in {text}"))
        };
        let line = StepStages::split_text(&[s]);
        let (render, movable) = (num(&line, "(means "), num(&line, " + "));
        assert!(
            (render + movable - s.sum()).abs() <= 0.001,
            "{render} + {movable} against {}: {line}",
            s.sum()
        );
        assert!(
            (num(&line, "render p50/p99 ") - 6.93).abs() <= 0.0005,
            "{line}"
        );
        assert!(
            (num(&line, "movable p50/p99 ") - 226.27).abs() <= 0.0005,
            "{line}"
        );
        assert!((num(&line, ", stages ") - 233.2).abs() <= 0.0005, "{line}");
        assert!(line.contains("bytes a step p50/p99 3000/3000"), "{line}");
        assert!(
            line.contains("(pool 2048, table 512, summary 256, job 184 mean)"),
            "{line}"
        );
        // Two steps: the means' sum is the stages' mean.
        let mut t = s;
        t.ms = t.ms.map(|m| m * 2.0);
        let line = StepStages::split_text(&[s, t]);
        let mean = (s.sum() + t.sum()) / 2.0;
        let (render, movable) = (num(&line, "(means "), num(&line, " + "));
        assert!((render + movable - mean).abs() <= 0.001, "{line}");
        // The tally's line carries the split.
        let mut tally = StepTally::default();
        tally.push(s);
        tally.tag(Some(Regime::KeepingUp));
        let lines = tally.summary();
        assert!(
            lines[0].contains("; render p50/p99 6.930/6.930 ms,"),
            "{}",
            lines[0]
        );
    }

    /// Laps move a call's time into the stage it ran in; a shift moves time and its render
    /// share together, the share never over the time moved.
    #[test]
    fn a_lap_takes_the_calls_inside_it() {
        let start = StepStart::now();
        let mut laps = Laps::new(start);
        laps.lap(S_SETUP);
        let before = calls();
        let buf_bytes = before.bytes;
        wgpu_call(|| std::thread::sleep(std::time::Duration::from_millis(2)));
        laps.lap(S_SUBMIT);
        assert!(laps.render[S_SUBMIT] >= 2.0, "{:?}", laps.render);
        assert!(laps.render[S_SUBMIT] <= laps.ms[S_SUBMIT] + 1e-9);
        assert!(laps.render[S_SETUP] < 0.5);
        assert_eq!(laps.bytes(), [0; 4]);
        assert_eq!(calls().bytes, buf_bytes);
        let (m, r) = laps.shift(S_SUBMIT, S_PLACE, 1.0, 5.0);
        assert!((m - 1.0).abs() < 1e-12 && (r - 1.0).abs() < 1e-12);
        let rm = laps.render_ms();
        assert!(rm.iter().zip(&laps.ms).all(|(r, m)| r <= m));
    }

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
