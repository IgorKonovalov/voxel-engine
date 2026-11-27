//! The live queue's per-second and census lines (readings): the step, view and listing census
//! lines and the job classes' ns a sample.

use super::QueueRun;
use crate::percentile;

/// `--queue-census`'s `bake queue census:` line of a listing census.
pub(crate) fn listing_census_line(c: &svet_render::ListingCensus, depth: u64) -> String {
    let band = |k: usize| {
        format!(
            "{} {} ({} in view, {} reach-bound)",
            svet_render::Band::ALL[k].name(),
            c.surface[k][0] + c.surface[k][1],
            c.surface[k][1],
            c.reach_bound[k]
        )
    };
    format!(
        "bake queue census: depth {depth}, listed {}, no face {} ({} at the residency's edge), buried with no emitter in reach {} (+{} under a column not final), surface {}: {}, {}, {}; {:.1} ms",
        c.listed,
        c.no_face,
        c.no_face_edge,
        c.buried,
        c.buried_unfinal,
        c.surface.iter().flatten().sum::<u64>(),
        band(0),
        band(1),
        band(2),
        c.ms,
    )
}

/// The ns a sample of each job class over a line (`samples`, `ms` since the last), attributed
/// by the submits' sample shares: `ns a sample full 5.1 coarse 4.0 (attributed)`, `-` for a
/// class that drew nothing.
pub(crate) fn class_ns_text(samples: [u64; 2], ms: [f64; 2]) -> String {
    let ns = |k: usize| {
        if samples[k] == 0 {
            "-".to_owned()
        } else {
            format!("{:.1}", ms[k] * 1e6 / samples[k] as f64)
        }
    };
    format!("ns a sample full {} coarse {} (attributed)", ns(0), ns(1))
}

/// The per-second `bake queue step:` line: the second's regime from its samples a baked chunk
/// (its steps tagged with it in the run's tally), the bake's GPU ms a second, the steps that
/// opened, the step's p50 on opening and other steps, the in-view waiting chunks whose reach
/// crosses the full radius, the `reach` mode with its partial blocks, partial jobs opened and
/// re-queues since the start, and each stage's mean ms a step. Reads the second's counters from
/// `q`'s last line and takes its steps.
pub(crate) fn queue_step_line(
    q: &mut QueueRun,
    st: &svet_render::QueueStats,
    at_s: f64,
    secs: f64,
    bake_ms_per_s: f64,
    reach: svet_render::ReachMode,
) -> String {
    use std::fmt::Write as _;
    use svet_render::{debug::Regime, debug::STEP_STAGES, debug::StepStages, internals::VIEW_IN};
    let samples = st.total_samples - q.line_samples;
    let baked = st.baked - q.line_baked;
    let regime = Regime::of(samples, baked);
    q.tally.tag(regime);
    let steps = std::mem::take(&mut q.line_stages);
    let p50 = |opened: Option<bool>| {
        let mut v: Vec<f64> = steps
            .iter()
            .filter(|s| opened.is_none_or(|o| s.opened == o))
            .map(|s| s.total_ms)
            .collect();
        if v.is_empty() {
            0.0
        } else {
            percentile(&mut v, 50.0)
        }
    };
    q.line_step_p50 = p50(None);
    q.line_split_p50 = [
        StepStages::render as fn(&StepStages) -> f64,
        StepStages::movable,
    ]
    .map(|f| {
        let mut v: Vec<f64> = steps.iter().map(f).collect();
        if v.is_empty() {
            0.0
        } else {
            percentile(&mut v, 50.0)
        }
    });
    let opened = steps.iter().filter(|s| s.opened).count();
    let [blank, coarse] = st.view_waiting[VIEW_IN];
    let mut line = format!(
        "bake queue step: at {at_s:.1} s, regime {} ({} samples a baked chunk, +{baked} chunks in {secs:.1} s, jobs {}, coarse {}), bake {bake_ms_per_s:.1} ms/s, opened {opened} of {} steps ({} since the start, {} starved), step p50 {:.3} ms (open {:.3}, not {:.3}), reach-bound {} near (waiting), {} mid/far (partial) of {} in view waiting, reach {} ({} partial, {} opened partial, {} re-queued); {}; stage means ms (render share):",
        regime.map_or("idle", Regime::name),
        if baked > 0 {
            format!("{:.3e}", samples as f64 / baked as f64)
        } else {
            "n/a".to_owned()
        },
        st.jobs,
        st.coarse,
        steps.len(),
        st.openings - q.line_openings,
        st.starved_openings - q.line_starved,
        q.line_step_p50,
        p50(Some(true)),
        p50(Some(false)),
        st.reach_bound[0],
        st.reach_bound[1],
        blank + coarse,
        reach.name(),
        st.partial,
        st.partial_opened,
        st.partial_requeued,
        StepStages::split_text(&steps),
    );
    q.line_openings = st.openings;
    q.line_starved = st.starved_openings;
    let n = steps.len().max(1) as f64;
    for (k, stage) in STEP_STAGES.iter().enumerate() {
        let mean = steps.iter().map(|s| s.ms[k]).sum::<f64>() / n;
        let render = steps.iter().map(|s| s.render_ms[k]).sum::<f64>() / n;
        let _ = write!(line, " {stage} {mean:.3} ({render:.3})");
    }
    let _ = write!(
        line,
        "; mirror changes a step mean {:.1}",
        (st.mirror_changes - q.line_changes) as f64 / n
    );
    q.line_changes = st.mirror_changes;
    let _ = write!(
        line,
        "; candidates scored a step mean {:.1}",
        (st.visited_total - q.line_visited) as f64 / n
    );
    q.line_visited = st.visited_total;
    line
}

/// The live queue's view line, a second: the candidates by where they lie (blank / coarse),
/// those the cone adds over the widened frustum, the samples of ended jobs by where their chunk
/// lay (since the start, and since the last line with `line_view` and `line_passed` then), the
/// in-view ages in frames over the run, and the weight log: the angle weights `needs` read
/// since the last line at p50 and p90, the refine weight at which the in-view median coarse
/// chunk ranks with the median blank one, and the census by class, distance band and angle
/// band.
pub(crate) fn queue_view_line(
    st: &svet_render::QueueStats,
    line_view: [u64; 3],
    line_passed: u64,
    needs: &[f32],
    first: &[u32],
    full: &[u32],
) -> String {
    let w = st.view_waiting;
    let total: u64 = st.view_samples.iter().sum();
    let d: [u64; 3] = std::array::from_fn(|k| st.view_samples[k] - line_view[k]);
    let d_total: u64 = d.iter().sum();
    let d_passed = st.passed_samples - line_passed;
    let share = |v: u64, of: u64| 100.0 * v as f64 / of.max(1) as f64;
    let pct = |v: &[u32], p: f64| {
        let mut f: Vec<f64> = v.iter().map(|&x| f64::from(x)).collect();
        if f.is_empty() {
            "-".to_owned()
        } else {
            format!("{:.0}", percentile(&mut f, p))
        }
    };
    let mut n: Vec<f64> = needs.iter().map(|&x| f64::from(x)).collect();
    let need = |n: &mut Vec<f64>, p: f64| {
        if n.is_empty() {
            "-".to_owned()
        } else {
            format!("{:.2}", percentile(n, p))
        }
    };
    let refine = if st.view_median[1] > 0.0 && st.view_median[0] > 0.0 {
        format!("{:.2}", st.view_median[0] / st.view_median[1])
    } else {
        "-".to_owned()
    };
    let census = |c: usize| {
        st.weight_census[c]
            .iter()
            .map(|a| format!("{} {} {} {}", a[0], a[1], a[2], a[3]))
            .collect::<Vec<_>>()
            .join(" | ")
    };
    format!(
        "bake queue view: waiting (blank/coarse) shell {}/{} in view {}/{} out {}/{} (reach not ready {} of {} read, census {:.3} ms), cone only {}, in-view median {:.1}/{:.1} chunks; samples shell {:.1} view {:.1} out {:.1} passed {:.1} % of {:.3e} since the start, last line shell {:.1} view {:.1} out {:.1} passed {:.1} % of {:.3e}; in-view ages (frames) first p50 {} p95 {} of {}, full p50 {} p95 {} of {}; angle weight needed p50 {} p90 {} of {}, refine {} (3 at 5..6: 1.67..2.00); census blank [{}] coarse [{}] (distance 0-2 | 3-5 | 6-10 | >10, a 0 <0.2 0.2-0.4 >0.4); withdrawn {} jobs, {:.3e} samples",
        w[0][0],
        w[0][1],
        w[1][0],
        w[1][1],
        w[2][0],
        w[2][1],
        st.view_reach[1],
        st.view_reach[0],
        st.census_ms,
        st.cone_only,
        st.view_median[0],
        st.view_median[1],
        share(st.view_samples[0], total),
        share(st.view_samples[1], total),
        share(st.view_samples[2], total),
        share(st.passed_samples, total),
        total as f64,
        share(d[0], d_total),
        share(d[1], d_total),
        share(d[2], d_total),
        share(d_passed, d_total),
        d_total as f64,
        pct(first, 50.0),
        pct(first, 95.0),
        first.len(),
        pct(full, 50.0),
        pct(full, 95.0),
        full.len(),
        need(&mut n, 50.0),
        need(&mut n, 90.0),
        n.len(),
        refine,
        census(0),
        census(1),
        st.withdrawn,
        st.withdrawn_samples as f64,
    )
}
