//! The live queue's time budget, its slice constants and its price of a submit.

use std::collections::VecDeque;

use super::JobClass;

/// The live queue's slice by default, in milliseconds of a frame.
pub const DEFAULT_BAKE_SLICE_MS: f64 = 1.5;

/// The least slice a per-second budget gives a step, in milliseconds: what a step with no
/// interval yet (the first) or a very short one bakes.
pub(crate) const BAKE_SLICE_FLOOR_MS: f64 = 0.25;

/// The live queue's bake budget (ADR-0052): a slice a frame, or milliseconds of GPU time a
/// second of wall time spent in slices sized by the frame's interval.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Budget {
    /// Milliseconds of bake a frame, whatever the frame's length.
    PerFrame { ms: f64 },
    /// Milliseconds of bake a second: a step's slice is `ms_per_s` times the last frame's
    /// interval in seconds, at most `cap` of that interval and at least
    /// [`BAKE_SLICE_FLOOR_MS`].
    PerSecond { ms_per_s: f64, cap: f64 },
}

impl Default for Budget {
    fn default() -> Self {
        Budget::PerFrame {
            ms: DEFAULT_BAKE_SLICE_MS,
        }
    }
}

impl Budget {
    /// The step's slice in milliseconds after a frame of `interval_s` seconds.
    pub fn slice_ms(&self, interval_s: f64) -> f64 {
        match *self {
            Budget::PerFrame { ms } => ms,
            Budget::PerSecond { ms_per_s, cap } => {
                let interval_ms = interval_s.max(0.0) * 1e3;
                (ms_per_s * interval_s.max(0.0))
                    .min(cap * interval_ms)
                    .max(BAKE_SLICE_FLOOR_MS)
            }
        }
    }

    /// Parses `frame:MS` or `second:MS,CAP` (the form [`std::fmt::Display`] writes).
    pub fn parse(s: &str) -> Result<Self, String> {
        let num = |v: &str| {
            v.trim()
                .parse::<f64>()
                .ok()
                .filter(|x| x.is_finite() && *x >= 0.0)
                .ok_or_else(|| format!("bake budget: {v:?} is not a non-negative number"))
        };
        match s.split_once(':') {
            Some(("frame", ms)) => Ok(Budget::PerFrame { ms: num(ms)? }),
            Some(("second", rest)) => {
                let (ms, cap) = rest
                    .split_once(',')
                    .ok_or_else(|| format!("bake budget: {s:?} wants second:MS,CAP"))?;
                let cap = num(cap)?;
                if cap <= 0.0 || cap > 1.0 {
                    return Err(format!("bake budget: the cap {cap} is not in (0, 1]"));
                }
                Ok(Budget::PerSecond {
                    ms_per_s: num(ms)?,
                    cap,
                })
            }
            _ => Err(format!(
                "bake budget: {s:?} is neither frame:MS nor second:MS,CAP"
            )),
        }
    }
}

impl std::fmt::Display for Budget {
    /// `frame:1.5` or `second:150,0.25`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Budget::PerFrame { ms } => write!(f, "frame:{ms}"),
            Budget::PerSecond { ms_per_s, cap } => write!(f, "second:{ms_per_s},{cap}"),
        }
    }
}

/// The slice's two values (ADR-0080 rule 3): `drain_ms` a frame while a chunk in view waits
/// for its tier's record, at least the [`Budget`]'s slice; `line_ms` a frame once none does
/// (the `docs/nfr.md` bake line).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drain {
    pub drain_ms: f64,
    pub line_ms: f64,
}

/// The floor profile's drain: 1.5 ms a frame while in-view work waits (the floor budget's own
/// slice: a longer slice landed the forest's view sooner but at a slower frame, Plan 0096
/// Phase 9's sweep), 1.0 once landed.
pub const DEFAULT_DRAIN: Drain = Drain {
    drain_ms: 1.5,
    line_ms: 1.0,
};

impl Drain {
    /// The step's slice after a frame of `interval_s` seconds under `budget`, with a chunk in
    /// view `waiting` or not.
    pub fn slice_ms(&self, budget: &Budget, interval_s: f64, waiting: bool) -> f64 {
        if waiting {
            self.drain_ms.max(budget.slice_ms(interval_s))
        } else {
            self.line_ms
        }
    }

    /// Parses `off` (`None`) or `DRAIN,LINE` in ms (the form [`std::fmt::Display`] writes).
    pub fn parse(s: &str) -> Result<Option<Self>, String> {
        if s == "off" {
            return Ok(None);
        }
        let num = |v: &str| {
            v.trim()
                .parse::<f64>()
                .ok()
                .filter(|x| x.is_finite() && (0.1..=100.0).contains(x))
                .ok_or_else(|| format!("bake drain: {v:?} is not 0.1 to 100 ms"))
        };
        let (d, l) = s
            .split_once(',')
            .ok_or_else(|| format!("bake drain: {s:?} is neither off nor DRAIN,LINE"))?;
        Ok(Some(Drain {
            drain_ms: num(d)?,
            line_ms: num(l)?,
        }))
    }
}

impl std::fmt::Display for Drain {
    /// `4,1`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{},{}", self.drain_ms, self.line_ms)
    }
}

/// Milliseconds a frame the live queue's slice grows by while a dig's bricks wait (ADR-0046's
/// edit cap, by time on the slice's own ns a sample).
pub const DEFAULT_BAKE_EDIT_MS: f64 = 3.0;

/// The live queue's ring-1 stretch ([`crate::BakeQueue::ring1_stretch`]): 1.5, a dig's ring 1
/// planned whole in a submit up to one and a half times the priced cap (the owner's pick, Plan
/// 0099 question 7 (f), on the slow adapter's drained readings of Phase 2e); 1.0 is the cap
/// alone. A setting takes 1.0 to 2.0 ([`RING1_STRETCH_RANGE`]).
pub const DEFAULT_RING1_STRETCH: f64 = 1.5;

/// Submits the live queue's steps hold unread at most on the bake thread (ADR-0083): a slot
/// turns once in two frames at best there, issued at a frame's take and read in a later
/// frame's message, so two slots cannot feed a submit every frame. The depth changes no record:
/// a face's samples are drawn in sample order and a round past its stop is discarded.
pub const THREAD_DEPTH: usize = 3;

/// The values the ring-1 stretch takes; a value outside is clamped into it.
pub const RING1_STRETCH_RANGE: std::ops::RangeInclusive<f64> = 1.0..=2.0;

/// The least submit the live queue's cap sends, in samples.
pub(crate) const CAP_FLOOR_SAMPLES: u64 = 16_384;

/// Read-backs the fit of [`SampleCost`] spans: about a second, so a dig's ring and the small
/// submits after it fall in one span.
const COST_FIT: usize = 64;

/// The spread of the dispatches' sizes in the fit's span (the largest over the smallest) from
/// which a fit is taken at all: submits of one size cannot tell a dispatch's fixed part from
/// its samples' cost.
const COST_SPREAD: f64 = 1.25;

/// The spread at which a fit replaces one taken over a wider span: the small submits under a
/// steady slice spread by up to about 2 with the estimate's noise, and their local slope is
/// not the cost of a grown slice; a slice's change (a dig, the drain's end) spreads them by 5
/// or more.
const COST_WIDE: f64 = 3.0;

/// The most a fit's slope may be off by its standard error, as a share of the slope.
const COST_SLOPE_ERROR: f64 = 0.1;

/// The decay a read-back of the sums the line's level is read from, as the mean's.
const COST_DECAY: f64 = 0.8;

/// The live queue's price of a submit on the GPU (Plan 0099 Phase 2b): a dispatch's fixed
/// milliseconds and the nanoseconds a sample, both from the submits read back. A cap sized by
/// the mean ns a sample alone prices a slice of another size by the old submits' share of the
/// fixed part: where that part is near the slice (a slow adapter under a 1 ms line), a slice
/// that grows (a dig's `edit_ms`) buys a fraction of what it could, and one that shrinks (the
/// drain's end) overdraws. The steady state is the same under either.
///
/// The ns a sample is the slope of a least-squares line over the last [`COST_FIT`]
/// read-backs' dispatches, taken when their sizes have spread by [`COST_SPREAD`] (and by the
/// last taken fit's spread up to [`COST_WIDE`]) and its standard error is within
/// [`COST_SLOPE_ERROR`]; between such spreads it is kept. The fixed part is then where a line
/// of that slope through the recent read-backs (decayed [`COST_DECAY`] a read-back) meets no
/// samples: under a steady slice the cap draws what the mean draws, and the level the noisy
/// small submits read well is re-read every step. Where that level is under 0 the price is the
/// recent read-backs' ms over their samples, and with no slope yet it is the mean.
#[derive(Clone, Debug)]
pub(crate) struct SampleCost {
    /// The mean ns a sample of the read-backs, `0.8 * old + 0.2 * new`: the cap's price while
    /// no slope is known, and the queue's printed `ns a sample`.
    pub(crate) mean_ns: f64,
    /// The last taken fit's ns a sample and its span's spread.
    slope_ns: Option<f64>,
    spread: f64,
    /// The recent read-backs' dispatches, samples and ms, decayed [`COST_DECAY`] a read-back.
    recent: [f64; 3],
    /// The fit's span: a read-back's samples and ms a dispatch.
    fit: VecDeque<(f64, f64)>,
}

impl SampleCost {
    /// An estimate that prices a sample at `ns` until the first read-back.
    pub(crate) fn new(ns: f64) -> Self {
        Self {
            mean_ns: ns,
            slope_ns: None,
            spread: 0.0,
            recent: [0.0; 3],
            fit: VecDeque::with_capacity(COST_FIT + 1),
        }
    }

    /// One step's read-back: `dispatches` submits that drew `samples` in `ms` on the GPU.
    pub(crate) fn read(&mut self, dispatches: u64, samples: u64, ms: f64) {
        if dispatches == 0 || samples == 0 {
            return;
        }
        self.mean_ns = 0.8 * self.mean_ns + 0.2 * (ms * 1e6 / samples as f64);
        let n = dispatches as f64;
        let r = &mut self.recent;
        let d = COST_DECAY;
        *r = [d * r[0] + n, d * r[1] + samples as f64, d * r[2] + ms];
        self.fit.push_back((samples as f64 / n, ms / n));
        if self.fit.len() > COST_FIT {
            self.fit.pop_front();
        }
        let (lo, hi) = self
            .fit
            .iter()
            .fold((f64::MAX, 0.0f64), |(lo, hi), p| (lo.min(p.0), hi.max(p.0)));
        let spread = hi / lo;
        if self.fit.len() < 3 || spread < COST_SPREAD.max(self.spread.min(COST_WIDE)) {
            return;
        }
        let k = self.fit.len() as f64;
        let (mx, my) = self
            .fit
            .iter()
            .fold((0.0, 0.0), |(x, y), p| (x + p.0 / k, y + p.1 / k));
        let (mut sxx, mut sxy, mut syy) = (0.0, 0.0, 0.0);
        for &(x, y) in &self.fit {
            sxx += (x - mx) * (x - mx);
            sxy += (x - mx) * (y - my);
            syy += (y - my) * (y - my);
        }
        let slope = sxy / sxx;
        let residual = (syy - slope * sxy).max(0.0) / (k - 2.0);
        if slope <= 0.0 || (residual / sxx).sqrt() > COST_SLOPE_ERROR * slope {
            return;
        }
        self.slope_ns = Some(slope * 1e6);
        self.spread = spread;
    }

    /// The price a cap is sized by: a dispatch's fixed ms and the ns a sample beyond it.
    pub(crate) fn price(&self) -> (f64, f64) {
        let [n, s, ms] = self.recent;
        match self.slope_ns {
            Some(per) if s > 0.0 => {
                let fixed = (ms - per * 1e-6 * s) / n;
                if fixed > 0.0 {
                    (fixed, per)
                } else {
                    (0.0, ms * 1e6 / s)
                }
            }
            _ => (0.0, self.mean_ns),
        }
    }

    /// The samples a submit draws in `ms` of the GPU: `(ms - fixed) / ns a sample`, at least
    /// [`CAP_FLOOR_SAMPLES`] and at most [`crate::bake::SUBMIT_SAMPLES`].
    pub(crate) fn cap(&self, ms: f64) -> u64 {
        let (fixed, per) = self.price();
        (((ms - fixed).max(0.0) * 1e6 / per.max(0.1)) as u64)
            .clamp(CAP_FLOOR_SAMPLES, super::SUBMIT_SAMPLES)
    }
}

/// The share of a read-back's samples drawn that makes it one class's reading in [`DigCost`].
const DIG_SHARE: f64 = 0.9;

/// The weight a new reading of a [`DigCost`] figure takes.
const DIG_DECAY: f64 = 0.8;

/// The ns a sample all in (a dispatch's whole ms over the samples it drew) of ring-1 dig work
/// and of other dig work (Plan 0099 Phase 2d). A sample costs by whose it is (a ring-2 sample
/// about twice another job's on a slow adapter), so a dig's cap priced by the general
/// [`SampleCost`] draws by the work read last. All in, not net of a fixed part: a dig's submits
/// all go out under one grant, where a figure read at that grant gives a cap that costs it.
/// A read-back at least [`DIG_SHARE`] of one class's samples moves that class's figure, `0.8
/// old + 0.2 new`; the first sets it. Kept between digs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct DigCost {
    pub(crate) ring1_ns: Option<f64>,
    pub(crate) dig_ns: Option<f64>,
}

impl DigCost {
    /// One step's read-backs: `drawn` samples in `ms`, `dig[0]` of them ring-1 dig jobs' and
    /// `dig[1]` other dig jobs'.
    pub(crate) fn read(&mut self, drawn: u64, dig: [u64; 2], ms: f64) {
        if drawn == 0 {
            return;
        }
        let n = drawn as f64;
        let figure = if dig[0] as f64 >= DIG_SHARE * n {
            &mut self.ring1_ns
        } else if (dig[0] + dig[1]) as f64 >= DIG_SHARE * n {
            &mut self.dig_ns
        } else {
            return;
        };
        let ns = ms * 1e6 / n;
        *figure = Some(figure.map_or(ns, |old| DIG_DECAY * old + (1.0 - DIG_DECAY) * ns));
    }

    /// The ns a sample a step whose first job is of `class` is priced by: its class's figure,
    /// else the other dig class's; `None` for a job that is not a dig's or no figure read.
    pub(crate) fn ns(&self, class: JobClass) -> Option<f64> {
        match class {
            JobClass::Ring1 => self.ring1_ns.or(self.dig_ns),
            JobClass::Dig => self.dig_ns.or(self.ring1_ns),
            JobClass::Other => None,
        }
    }

    /// The samples a submit whose first job is of `class` draws in `ms`: `ms` over the class's
    /// figure ([`DigCost::ns`]) under [`SampleCost::cap`]'s clamp, else `cost`'s cap.
    pub(crate) fn cap(&self, class: JobClass, cost: &SampleCost, ms: f64) -> u64 {
        match self.ns(class) {
            Some(ns) => ((ms.max(0.0) * 1e6 / ns.max(0.1)) as u64)
                .clamp(CAP_FLOOR_SAMPLES, super::SUBMIT_SAMPLES),
            None => cost.cap(ms),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// At 150 ms a second a 10 ms interval gives a 1.5 ms slice and a 66 ms one 9.9 ms; at 300
    /// ms a second and a cap of 0.25 the 66 ms interval gives the cap's 16.5 ms; a 0 interval
    /// gives the floor. A per-frame budget ignores the interval; both forms parse back from
    /// their text.
    #[test]
    fn a_per_second_budget_sizes_the_slice_by_the_interval() {
        let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
        let s150 = Budget::PerSecond {
            ms_per_s: 150.0,
            cap: 0.25,
        };
        assert!(near(s150.slice_ms(0.010), 1.5), "{}", s150.slice_ms(0.010));
        assert!(near(s150.slice_ms(0.066), 9.9), "{}", s150.slice_ms(0.066));
        let s300 = Budget::PerSecond {
            ms_per_s: 300.0,
            cap: 0.25,
        };
        assert!(near(s300.slice_ms(0.066), 16.5), "{}", s300.slice_ms(0.066));
        assert_eq!(s150.slice_ms(0.0), BAKE_SLICE_FLOOR_MS);
        assert_eq!(s300.slice_ms(0.0), BAKE_SLICE_FLOOR_MS);
        let f = Budget::PerFrame { ms: 1.5 };
        assert_eq!(f.slice_ms(0.0), 1.5);
        assert_eq!(f.slice_ms(0.066), 1.5);
        assert_eq!(f.to_string(), "frame:1.5");
        assert_eq!(s150.to_string(), "second:150,0.25");
        assert_eq!(Budget::parse("frame:1.5"), Ok(f));
        assert_eq!(Budget::parse("second:150,0.25"), Ok(s150));
        assert!(Budget::parse("second:150").is_err());
        assert!(Budget::parse("second:150,0").is_err());
        assert!(Budget::parse("minute:3").is_err());
    }

    /// Plan 0096 Phase 4 (ADR-0080 rule 3): the drain value while a chunk in view waits, at
    /// least the budget's slice, and the line's once none does; `off` and `DRAIN,LINE` parse.
    #[test]
    fn the_drain_slice_is_the_drain_value_while_waiting_and_the_line_after() {
        let d = Drain {
            drain_ms: 4.0,
            line_ms: 1.0,
        };
        assert_eq!(DEFAULT_DRAIN.to_string(), "1.5,1");
        let f = Budget::PerFrame { ms: 1.5 };
        assert_eq!(d.slice_ms(&f, 1.0 / 60.0, true), 4.0);
        assert_eq!(d.slice_ms(&f, 1.0 / 60.0, false), 1.0);
        let big = Budget::PerFrame { ms: 8.0 };
        assert_eq!(
            d.slice_ms(&big, 1.0 / 60.0, true),
            8.0,
            "the budget's floor"
        );
        assert_eq!(Drain::parse("off"), Ok(None));
        assert_eq!(Drain::parse("4,1"), Ok(Some(d)));
        assert_eq!(d.to_string(), "4,1");
        assert!(Drain::parse("4").is_err());
        assert!(Drain::parse("0,1").is_err());
    }
}
