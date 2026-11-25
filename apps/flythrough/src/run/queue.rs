//! The live queue in the run: the pre-roll, the per-frame step, its watch on digs and its
//! printed lines.

use crate::app::ms_since;
use crate::now;
use crate::percentile;
use crate::run::Run;
use crate::run::hud_feed::write_queue_banner_row;
use svet_app::Gpu;
use svet_render::wgpu;
use svet_world::ChunkCoord;

/// Samples a step of the live queue's pre-roll draws: about one chunk's brick level at 4096, so
/// the jobs it opens stay near the camera.
pub(crate) const PRE_ROLL_SAMPLES: u64 = 2_000_000;

/// Milliseconds the pre-roll's coarse pass runs at most past the near chunks: a streamed world's
/// mirror holds about 17 000 full chunks within its full radius, which the live queue then takes
/// coarse first; the level's 3168 take about 4 s on the RTX.
pub(crate) const COARSE_PRE_ROLL_MS: f64 = 5000.0;

/// The live queue's budget a frame while `--pre-roll-drain` drains, in ms.
pub(crate) const PRE_ROLL_DRAIN_MS: f64 = 40.0;

/// Wall ms the live queue must stay idle ([`drain_idle`]) before `--pre-roll-drain` calls the
/// view landed.
const DRAIN_LANDED_HOLD_MS: f64 = 1500.0;

/// The distance past which a lighting pixel counts as far in the drain's closing line (the
/// wobble rig's far class).
const DRAIN_FAR_M: f32 = 15.0;

/// `--pre-roll-drain`'s state: not started, draining since a frame-loop ms (with the ms the
/// queue first read idle), or ended at a ms after its seconds, landed or timed out.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) enum Drain {
    #[default]
    Off,
    Draining {
        from_ms: f64,
        clear_ms: Option<f64>,
    },
    Done {
        at_ms: f64,
        secs: f64,
        landed: bool,
    },
}

/// Whether the live queue has nothing left it can bake while the camera stands: no job open or
/// pending and no coarse block waiting for its refinement. The chunks still without a block are
/// those it cannot take (their reach crosses the streamer's full radius, or they are not
/// resident full).
pub(crate) fn drain_idle(st: &svet_render::QueueStats) -> bool {
    st.jobs == 0 && st.coarse == 0
}

/// Appends `--pre-roll-drain`'s banner row to `out`: while it drains the queue's jobs and coarse
/// chunks left and the seconds to its limit (`secs`), then for `hud::BANNER_MS` after it ends how it ended.
pub(crate) fn write_drain_banner(
    out: &mut String,
    drain: Drain,
    stats: Option<&svet_render::QueueStats>,
    secs: f64,
    now_ms: f64,
) {
    use std::fmt::Write as _;
    match drain {
        Drain::Off => {}
        Drain::Draining { from_ms, .. } => {
            crate::hud::start_row(out);
            let left = (secs - (now_ms - from_ms) / 1e3).max(0.0);
            match stats {
                Some(st) => {
                    let _ = write!(
                        out,
                        "pre-roll: draining {} jobs, {} coarse chunks to refine, {left:.0} s at most; HOLD STILL",
                        st.jobs, st.coarse,
                    );
                }
                None => {
                    let _ = write!(out, "pre-roll: draining, {left:.0} s at most; HOLD STILL");
                }
            }
        }
        Drain::Done {
            at_ms,
            secs: took,
            landed,
        } => {
            if now_ms - at_ms < crate::hud::BANNER_MS {
                crate::hud::start_row(out);
                let _ = write!(
                    out,
                    "pre-roll: {} after {took:.0} s; move now",
                    if landed { "landed" } else { "timed out" },
                );
            }
        }
    }
}

/// The live queue's state in the fly-through: the workers' results before the frame exists, and
/// what its stdout line reads.
#[derive(Default)]
pub(crate) struct QueueRun {
    /// `--pre-roll-drain`'s state.
    pub(crate) drain: Drain,
    pub(crate) prepared: Vec<(ChunkCoord, svet_world::Prepared)>,
    /// Ms since the start at the last line, and the chunks baked and samples then.
    pub(crate) line_ms: f64,
    pub(crate) line_frame: u32,
    pub(crate) line_baked: u64,
    pub(crate) line_samples: u64,
    pub(crate) line_prepared: u64,
    /// The submits' ms since the last line.
    pub(crate) slices: Vec<f64>,
    /// The projection over the cap at each draw since the last line, with the bake thread's
    /// listing pool (`QueueStats::projected`).
    pub(crate) projected: Vec<f64>,
    pub(crate) emptied: bool,
    pub(crate) stats: Option<svet_render::QueueStats>,
    pub(crate) failed: bool,
    /// Ms since the start at the last step, and the dig being watched.
    pub(crate) step_ms: Option<f64>,
    pub(crate) dig: Option<DigWatch>,
    /// The CPU ms of each `bake_queue_step` since the last line, and of the last one.
    pub(crate) step_cpu: Vec<f64>,
    pub(crate) last_step_cpu: f64,
    /// The steps' stages since the last line, and over the run by the regime of their second
    /// (the `frames:` summary's `queue step` lines).
    pub(crate) line_stages: Vec<svet_render::debug::StepStages>,
    /// The queue's candidates scored at the last step line ([`svet_render::QueueStats::visited_total`]).
    pub(crate) line_visited: u64,
    pub(crate) tally: svet_render::debug::StepTally,
    /// The jobs and the openings at the last line, and the last line's step p50 (the HUD's).
    pub(crate) line_openings: u64,
    /// [`svet_render::QueueStats::starved_openings`] at the last line.
    pub(crate) line_starved: u64,
    pub(crate) line_step_p50: f64,
    /// The last line's `render` and `movable` p50s (the HUD's).
    pub(crate) line_split_p50: [f64; 2],
    /// The samples by view class and passed at the last line.
    pub(crate) line_view: [u64; 3],
    pub(crate) line_passed: u64,
    /// The samples a second over the last line's second: the in-view drain's rate.
    pub(crate) line_rate: f64,
    /// [`svet_render::QueueStats::class_samples`] and `class_ms` at the last line.
    pub(crate) line_class: ([u64; 2], [f64; 2]),
    /// [`svet_render::QueueStats::mirror_changes`] at the last line.
    pub(crate) line_changes: u64,
    /// Whether the next step prints `--queue-census`'s listing census (set once at the hand-
    /// over to the live queue).
    pub(crate) listing_due: bool,
    /// The frames' wall and CPU ms, by whether the last step left chunks in view waiting for
    /// their tier (the drain, `[0]`) or none (`[1]`): the `frames: queue split` line.
    pub(crate) split_frames: [[Vec<f64>; 2]; 2],
}

impl QueueRun {
    /// Files frame `frame`'s wall (`dt` s, not on the first frame, whose delta spans the
    /// start-up) and CPU ms under the queue's state at its last step; nothing before the live
    /// queue's first step.
    pub(crate) fn push_frame(&mut self, frame: u32, dt: f32, cpu_ms: f64) {
        let Some(st) = &self.stats else {
            return;
        };
        let k = usize::from(!st.draining);
        if frame > 0 {
            self.split_frames[k][0].push(f64::from(dt) * 1e3);
        }
        self.split_frames[k][1].push(cpu_ms);
    }

    /// The `frames: queue split` line: the frames' wall and CPU p50/p99 while chunks in view
    /// wait for their tier and after.
    pub(crate) fn split_line(&self) -> Option<String> {
        self.stats?;
        let part = |k: usize| {
            let pick = |v: &[f64], p: f64| {
                let mut v = v.to_vec();
                if v.is_empty() {
                    0.0
                } else {
                    percentile(&mut v, p)
                }
            };
            let [w, c] = &self.split_frames[k];
            format!(
                "{} frames, wall p50/p99 {:.2}/{:.2} ms, frame CPU p50/p99 {:.2}/{:.2} ms",
                c.len(),
                pick(w, 50.0),
                pick(w, 99.0),
                pick(c, 50.0),
                pick(c, 99.0)
            )
        };
        Some(format!(
            "queue split: while in view waits {}; after {}",
            part(0),
            part(1)
        ))
    }
}

/// Prints `--queue-census`'s listing census of the live queue under `view` when `q` has one
/// due ([`QueueRun::listing_due`]), once.
fn print_listing_census(
    gfx: &mut crate::run::Gfx,
    world: &svet_world::World,
    view: &svet_render::QueueView,
    q: &mut QueueRun,
) {
    if !std::mem::take(&mut q.listing_due) {
        return;
    }
    if let Some(q) = gfx.frame.bake_queue_mut() {
        let st = q.stats();
        let (shells, shell_bytes) = q.shells();
        let c = q.listing_census(&gfx.mirror, world, view);
        println!("{}", listing_census_line(&c, st.depth));
        println!(
            "bake queue census: sealed by the workers {} of {} chunks, the verdict mean {:.1} us max {:.1} us a chunk; {} shells, {} B",
            st.sealed,
            st.hook_chunks,
            st.seal_ns as f64 / st.hook_chunks.max(1) as f64 / 1e3,
            st.seal_max_ns as f64 / 1e3,
            shells,
            shell_bytes,
        );
    }
}

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

impl QueueRun {
    /// The in-view drain of `st` at the last line's rate.
    pub(crate) fn drain_of(&self, st: &svet_render::QueueStats) -> crate::report::QueueDrain {
        crate::report::QueueDrain {
            depth: st.depth,
            waiting: st.in_view,
            samples_per_s: self.line_rate,
            rule_missed: (st.rule_missed != svet_render::RuleMissed::default())
                .then_some(st.rule_missed),
        }
    }
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

/// A dig in the live queue, from the step that first read its box until its rings are baked:
/// the frames, the longest wall time between two steps, and the queue's counters at the start.
pub(crate) struct DigWatch {
    pub(crate) start_ms: f64,
    pub(crate) frames: u32,
    pub(crate) longest_ms: f64,
    pub(crate) over_33: u32,
    pub(crate) reach_max: u64,
    /// The largest share of the lighting pixels reading no value, read back each frame until
    /// rings 1 and 2 are baked (`--read-missing` only).
    pub(crate) missing_max: f64,
    pub(crate) rings_ms: Option<(f64, u32)>,
    pub(crate) from: svet_render::QueueStats,
}

/// The camera the run starts at: the castle's shot, the showcase's shot, `--pose`'s still
/// camera, or the spawn facing +x.
/// The live queue's view angles for a camera of `vfov` drawn at `size` (output pixels): the
/// cone's half-angle (the wider of the two) and the frustum's horizontal and vertical ones;
/// and the output's focal length in pixels ([`svet_render::Bands::focal_px`]).
pub(crate) fn queue_view_angles(vfov: f32, size: [u32; 2]) -> (f32, [f32; 2], f32) {
    let aspect = size[0].max(1) as f32 / size[1].max(1) as f32;
    (
        svet_render::internals::view_half_angle(vfov, aspect),
        svet_render::internals::view_half_angles(vfov, aspect),
        svet_render::Bands::focal_px(size[1], vfov),
    )
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

impl Run {
    /// One frame of the live queue, ahead of the encode: `enc` (the sync's copies) is submitted
    /// first so the bake reads this frame's mirror, and a new encoder returned. Prints a line a
    /// second, and the time the queue first empties.
    pub(crate) fn step_bake_queue(
        &mut self,
        gpu: &Gpu,
        enc: wgpu::CommandEncoder,
    ) -> wgpu::CommandEncoder {
        let Some(gfx) = self.gfx.as_mut() else {
            return enc;
        };
        // The debug page's `queue freeze`: no step, so no record lands or starts.
        if gfx.frame.bake_queue().is_none() || self.modes.diag.queue_frozen {
            return enc;
        }
        gpu.queue.submit([enc.finish()]);
        self.drain_begin();
        let Some(gfx) = self.gfx.as_mut() else {
            return gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        };
        let (forward, _, _) = self.fly.camera.basis();
        let stream = self.stream.as_ref();
        let resident = stream.map(|s| svet_render::Residency::of_chunks(s.streamer.full_chunks()));
        let (half_angle, frustum, focal) =
            queue_view_angles(self.fly.camera.vfov, gfx.frame.output_size());
        let t_step = ms_since(self.start);
        let dt = self.queue.step_ms.replace(t_step).map(|at| t_step - at);
        let view = svet_render::QueueView {
            eye: self.fly.camera.position.to_array(),
            forward: forward.to_array(),
            resident,
            samples: None,
            interval_s: dt.unwrap_or(0.0) * 1e-3,
            hold: stream.map(|s| s.rule.r_full + self.args.light.radius() + 1),
            half_angle,
            frustum: Some(frustum),
            full_radius: stream.map(|s| s.rule.r_full),
            lod_radius: stream.map(|s| {
                let r = s.streamer.config().residency;
                r.r_lod.max(r.r_full)
            }),
            focal_px: Some(focal),
        };
        let dug = gfx.frame.bake_queue().is_some_and(|q| !q.digs_settled());
        let new_box = gfx.frame.bake_queue().is_some_and(|q| q.boxes_waiting());
        // A dig after the last one's rings 1 and 2 starts its own watch; one before them joins it.
        if new_box && let Some(w) = self.queue.dig.take_if(|w| w.rings_ms.is_some()) {
            println!(
                "dig settled: ring 3 not yet ({} chunks at most) after {} frames, {:.0} ms, at the next dig; frames: longest {:.1} ms, {} over 33 ms",
                w.reach_max,
                w.frames,
                t_step - w.start_ms,
                w.longest_ms,
                w.over_33,
            );
        }
        if dug && self.queue.dig.is_none() {
            self.queue.dig = self.queue.stats.map(|from| DigWatch {
                start_ms: t_step,
                frames: 0,
                longest_ms: 0.0,
                over_33: 0,
                reach_max: 0,
                missing_max: 0.0,
                rings_ms: None,
                from,
            });
        }
        let t_cpu = now();
        let stepped =
            gfx.frame
                .bake_queue_step(&gpu.device, &gpu.queue, &mut gfx.mirror, &self.world, &view);
        self.queue.last_step_cpu = ms_since(t_cpu);
        self.queue.step_cpu.push(self.queue.last_step_cpu);
        // After the step: the queue holds the frame's far-normal band by then.
        print_listing_census(gfx, &self.world, &view, &mut self.queue);
        match stepped {
            Some(Ok(st)) => {
                if st.samples > 0 {
                    self.queue.slices.push(st.ms);
                }
                if st.projected > 0.0 {
                    self.queue.projected.push(st.projected);
                }
                self.queue.line_stages.push(st.stages);
                self.queue.tally.push(st.stages);
                if let Some(w) = self.queue.dig.as_mut() {
                    w.frames += 1;
                    if let Some(dt) = dt {
                        w.longest_ms = w.longest_ms.max(dt);
                        w.over_33 += u32::from(dt > 33.0);
                    }
                    w.reach_max = w.reach_max.max(st.reach);
                    if w.rings_ms.is_none() && self.args.bake.read_missing {
                        // The store's counters, read back blocking: the wait it adds to each
                        // frame is in the frames this line reads.
                        let missing = gfx.frame.read_store_stats(&gpu.device, &gpu.queue).missing;
                        let share = f64::from(missing) / f64::from(gfx.frame.light_pixels().max(1));
                        w.missing_max = w.missing_max.max(share);
                    }
                    if w.rings_ms.is_none() && st.digs == 0 {
                        w.rings_ms = Some((t_step - w.start_ms, w.frames));
                        println!(
                            "dig settled: rings 1 and 2 in {} frames, {:.0} ms ({} dig jobs, {} bricks, {:.3e} samples); frames from the dig: longest {:.1} ms, {} over 33 ms; no value at most {}; ring 3 {} chunks waiting",
                            w.frames,
                            t_step - w.start_ms,
                            st.digs_baked - w.from.digs_baked,
                            st.dig_bricks - w.from.dig_bricks,
                            (st.total_samples - w.from.total_samples) as f64,
                            w.longest_ms,
                            w.over_33,
                            if self.args.bake.read_missing {
                                format!("{:.4} of the pixels", w.missing_max)
                            } else {
                                "not read (--read-missing)".to_owned()
                            },
                            st.reach,
                        );
                    }
                    if w.rings_ms.is_some() && st.reach == 0 {
                        println!(
                            "dig settled: ring 3 ({} chunks at most) in {} frames, {:.0} ms from the dig; frames: longest {:.1} ms, {} over 33 ms",
                            w.reach_max,
                            w.frames,
                            t_step - w.start_ms,
                            w.longest_ms,
                            w.over_33,
                        );
                        self.queue.dig = None;
                    }
                }
                self.queue.stats = Some(st);
                if !self.queue.emptied && gfx.frame.bake_queue().is_some_and(|q| q.empty()) {
                    self.queue.emptied = true;
                    println!(
                        "bake queue: empty after {:.1} s since the start ({} chunks baked, {:.3e} samples)",
                        ms_since(self.start) / 1e3,
                        st.baked,
                        st.total_samples as f64
                    );
                }
            }
            Some(Err(e)) => {
                if !self.queue.failed {
                    println!("bake queue: {e}");
                }
                self.queue.failed = true;
            }
            None => {}
        }
        self.drain_check(gpu);
        self.print_queue_line(gpu);
        gpu.device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default())
    }

    /// `--pre-roll-drain`'s start, at the first queue step: the budget to `PRE_ROLL_DRAIN_MS` a
    /// frame and the camera held from this frame.
    fn drain_begin(&mut self) {
        if self.args.bake.pre_roll_drain.is_none() || self.queue.drain != Drain::Off {
            return;
        }
        if let Some(gfx) = self.gfx.as_mut() {
            gfx.frame.set_bake_budget(svet_render::Budget::PerFrame {
                ms: PRE_ROLL_DRAIN_MS,
            });
        }
        self.queue.drain = Drain::Draining {
            from_ms: ms_since(self.start),
            clear_ms: None,
        };
        println!(
            "pre-roll drain: the camera held, the queue at {PRE_ROLL_DRAIN_MS} ms a frame for {} s at most",
            self.args.bake.pre_roll_drain.unwrap_or_default()
        );
    }

    /// `--pre-roll-drain`'s end, after the frame's queue step: landed once the queue has been
    /// idle ([`drain_idle`]) for `DRAIN_LANDED_HOLD_MS`, timed out at its seconds;
    /// either returns the budget to `--bake-budget`'s (the menu's) and prints the line and the
    /// far share ([`Run::print_drain_share`]).
    fn drain_check(&mut self, gpu: &Gpu) {
        let (Drain::Draining { from_ms, clear_ms }, Some(st)) =
            (self.queue.drain, self.queue.stats)
        else {
            return;
        };
        let t = ms_since(self.start);
        let limit = self.args.bake.pre_roll_drain.unwrap_or_default();
        let clear_ms = (drain_idle(&st) && st.openings > 0).then(|| clear_ms.unwrap_or(t));
        let landed = clear_ms.is_some_and(|c| t - c >= DRAIN_LANDED_HOLD_MS);
        let secs = (t - from_ms) / 1e3;
        if !landed && secs < limit {
            self.queue.drain = Drain::Draining { from_ms, clear_ms };
            return;
        }
        if let Some(gfx) = self.gfx.as_mut() {
            gfx.frame.set_bake_budget(self.args.bake.budget);
        }
        self.queue.drain = Drain::Done {
            at_ms: t,
            secs,
            landed,
        };
        println!(
            "pre-roll drain: {} after {secs:.1} s (frame {}), {} jobs, {} coarse, depth {}, {} baked; the budget back to {}",
            if landed { "landed" } else { "timed out" },
            self.frame,
            st.jobs,
            st.coarse,
            st.depth,
            st.baked,
            self.args.bake.budget,
        );
        self.print_drain_share(gpu);
    }

    /// Whether `--pre-roll-drain` holds the camera this frame.
    pub(crate) fn draining(&self) -> bool {
        matches!(self.queue.drain, Drain::Draining { .. })
    }

    /// `--pre-roll-drain`'s closing share line: of the last frame's lighting pixels, those whose
    /// hit lies over `DRAIN_FAR_M` from the eye (far), and of them those whose sky source word
    /// is `record` (a fresh store record: far-baked). Reads the hit positions and the moments
    /// back blocking, once; the source words are written while the store's reads are counted,
    /// which the drain turns on.
    fn print_drain_share(&self, gpu: &Gpu) {
        let Some(gfx) = self.gfx.as_ref() else {
            return;
        };
        let (d, q) = (&gpu.device, &gpu.queue);
        let hits = svet_render::debug::read_rgba32f(d, q, gfx.frame.light_hit_positions());
        let moments = svet_render::debug::read_rgba16f(d, q, gfx.frame.moments());
        let eye = self.fly.camera.position.to_array();
        let far_voxels = DRAIN_FAR_M / crate::audio::METRES_PER_VOXEL;
        let (mut far, mut far_baked) = (0usize, 0usize);
        for (h, m) in hits.iter().zip(&moments) {
            if !h[3].is_finite() || h[3] > 1.0e30 {
                continue;
            }
            let d2: f32 = (0..3).map(|k| (h[k] - eye[k]).powi(2)).sum();
            if d2.sqrt() >= far_voxels {
                far += 1;
                // The moments' w: the kind plus the source word over 16; word 1 is `record`.
                let word = ((m[3] - m[3].floor()) * 16.0).round() as u8;
                far_baked += usize::from(word == 1);
            }
        }
        let n = hits.len().max(1) as f64;
        println!(
            "pre-roll drain: far (over {DRAIN_FAR_M} m) {:.1} % of the lighting pixels, far-baked {:.1} % ({:.1} % of far)",
            100.0 * far as f64 / n,
            100.0 * far_baked as f64 / n,
            100.0 * far_baked as f64 / far.max(1) as f64,
        );
    }

    /// The live queue's lines, once a second of wall time in a window or once every 60 frames
    /// headless: the step line with the store's counters read without waiting, the view line
    /// and the stage line; then the next line's starting counts.
    fn print_queue_line(&mut self, gpu: &Gpu) {
        let Some(gfx) = self.gfx.as_mut() else {
            return;
        };
        // A line a second: of wall time in a window, of 60 frames headless (a headless frame is
        // 1/60 s of the glide and the exposure).
        let t = ms_since(self.start);
        let due = if self.args.headless {
            self.frame.is_multiple_of(60)
        } else {
            t - self.queue.line_ms >= 1000.0
        };
        if due && let Some(st) = self.queue.stats {
            // The store's counters without waiting: the newest copy finished on the GPU, its
            // age in frames on the line.
            let (store_stats, store_age) = gfx
                .frame
                .try_store_stats(&gpu.device)
                .map_or((svet_render::StoreStats::default(), None), |(s, age)| {
                    (s, Some(age))
                });
            // The workers' shells under their lock, once a line.
            let (shells, shell_bytes) = gfx.frame.bake_queue().map_or((0, 0), |q| q.shells());
            let pixels = gfx.frame.light_pixels().max(1);
            let reads = if gfx.frame.counts_store() {
                format!(
                    "no value {:.4} fallback {:.4} still black {:.4} coarse {:.4} of {} pixels (missing light {}; store {} B, summaries {} B, parents {} B; {})",
                    f64::from(store_stats.missing) / f64::from(pixels),
                    f64::from(store_stats.fallback) / f64::from(pixels),
                    f64::from(store_stats.still_black) / f64::from(pixels),
                    f64::from(store_stats.coarse_reads) / f64::from(pixels),
                    pixels,
                    self.args.bake.missing_light.name(),
                    store_stats.bytes,
                    store_stats.summary_bytes,
                    store_stats.parent_bytes,
                    store_age.map_or_else(
                        || "not read back yet".to_owned(),
                        |a| format!("read {a} frames old")
                    ),
                )
            } else {
                "store reads not counted (the HUD or --read-missing counts them)".to_owned()
            };
            let secs = if self.args.headless {
                f64::from(self.frame - self.queue.line_frame) / 60.0
            } else {
                (t - self.queue.line_ms) / 1e3
            }
            .max(1e-3);
            let mut sl = std::mem::take(&mut self.queue.slices);
            sl.sort_by(f64::total_cmp);
            let p50 = sl.get(sl.len() / 2).copied().unwrap_or(0.0);
            let max = sl.last().copied().unwrap_or(0.0);
            let mut cpu = std::mem::take(&mut self.queue.step_cpu);
            self.queue.line_rate = (st.total_samples - self.queue.line_samples) as f64 / secs;
            let (cs, cm) = self.queue.line_class;
            let class_ns = class_ns_text(
                [0, 1].map(|k| st.class_samples[k] - cs[k]),
                [0, 1].map(|k| st.class_ms[k] - cm[k]),
            );
            self.queue.line_class = (st.class_samples, st.class_ms);
            println!(
                "bake queue: at {:.1} s, frame {}, depth {}, jobs {}, waiting reach {} lights {}, baked {} (+{} in {:.1} s, {} from the workers), {:.3e} samples/s, slice p50 {:.2} max {:.2} ms over {} submits, {:.1} ns a sample (price {:.2} ms a dispatch + {:.1} ns), fading {}, coarse {} chunks, {}, eye {:.0} {:.0} {:.0}, step CPU p50 {:.3} max {:.3} ms, provisional {} (opened {}, catch-ups {}), key {} (landed {}), zero by rule {} (re-listed {}), sealed by the workers {} ({} shells, {} B), {}, {}",
                t / 1e3,
                self.frame,
                st.depth,
                st.jobs,
                st.waiting_reach,
                st.waiting_lights,
                st.baked,
                st.baked - self.queue.line_baked,
                secs,
                st.prepared - self.queue.line_prepared,
                (st.total_samples - self.queue.line_samples) as f64 / secs,
                p50,
                max,
                sl.len(),
                st.ns_per_sample,
                st.fixed_ms,
                st.cost_ns,
                st.fading,
                st.coarse,
                reads,
                self.fly.camera.position.x,
                self.fly.camera.position.y,
                self.fly.camera.position.z,
                percentile(&mut cpu, 50.0),
                percentile(&mut cpu, 100.0),
                st.provisional,
                st.provisional_opened,
                st.provisional_requeued,
                st.keys,
                st.keys_baked,
                st.zeroed,
                st.zero_relisted,
                st.sealed,
                shells,
                shell_bytes,
                self.queue.drain_of(&st).text(),
                class_ns,
            );
            let mut projected = std::mem::take(&mut self.queue.projected);
            print_slice_line(t, &st, p50, percentile(&mut projected, 50.0));
            // The next second's line and the HUD's row read the next opening's census: asked
            // through the frame's message, so the bake thread's queue takes it too.
            gfx.frame
                .set_bake_queue(svet_render::BakeQueue::request_census);
            let needs = gfx
                .frame
                .bake_queue_mut()
                .map(svet_render::BakeQueue::take_angle_needs)
                .unwrap_or_default();
            let (first, full) = gfx
                .frame
                .bake_queue()
                .map(|q| {
                    let (a, b) = q.view_ages();
                    (a.to_vec(), b.to_vec())
                })
                .unwrap_or_default();
            println!(
                "{}",
                queue_view_line(
                    &st,
                    self.queue.line_view,
                    self.queue.line_passed,
                    &needs,
                    &first,
                    &full
                )
            );
            println!(
                "{}",
                queue_step_line(
                    &mut self.queue,
                    &st,
                    t / 1e3,
                    secs,
                    sl.iter().sum::<f64>() / secs,
                    self.args.bake.reach,
                )
            );
            self.queue.line_view = st.view_samples;
            self.queue.line_passed = st.passed_samples;
            self.queue.line_ms = t;
            self.queue.line_frame = self.frame;
            self.queue.line_baked = st.baked;
            self.queue.line_samples = st.total_samples;
            self.queue.line_prepared = st.prepared;
            if self.args.outdoor.dump_black && store_stats.still_black > 0 {
                self.dump_black(gpu, store_stats.still_black);
            }
        }
    }

    /// The live queue's banner row: the mode, its settings and slice, its order and keys.
    pub(crate) fn queue_row(&self) -> String {
        let mut out = String::new();
        write_queue_banner_row(&mut out, &self.args, &self.args.queue_words());
        out
    }
}

/// The per-second `bake queue slice:` line: the slice granted and whether it was the drain's
/// value (ADR-0080 rule 3), the chunks in view still waiting for their tier, the slices drawn,
/// why the submits' plans stopped, and the reads of the in-view set with their mean and longest
/// cost; with the bake thread's listing pool, the projection over the cap at the draws (p50),
/// the listing bounds' two fills (from the world, from prepared masks) and the openings the
/// pool's guard stopped.
fn print_slice_line(t: f64, st: &svet_render::QueueStats, p50: f64, projected: f64) {
    println!(
        "bake queue slice: at {:.1} s, granted {:.2} ms ({}, {} in view waiting for their tier), drawn p50 {:.2} ms, plans stopped at the cap {} at the batch {} out of rounds {} ({} with no table slot free), steps with every submit slot unread {} of {}, the in-view set read {} times, mean {:.3} ms, max {:.3} ms, projected at the draw p50 {:.2} caps, fill world {:.3} masks {:.3}, pool guard {}, wasted samples {} of {} drawn",
        t / 1e3,
        st.slice_ms,
        if st.draining { "drain" } else { "line" },
        st.drain_waiting,
        p50,
        st.plan_stops[0],
        st.plan_stops[1],
        st.plan_stops[2],
        st.slotless_stops,
        st.in_flight_full,
        st.steps,
        st.waiting_reads,
        st.waiting_ms / st.waiting_reads.max(1) as f64,
        st.waiting_max_ms,
        projected,
        st.fill[0],
        st.fill[1],
        st.pool_guard,
        st.read_back[1],
        st.read_back[0] + st.read_back[1],
    );
}

/// The bake thread's submits on the render side (a reading, the teardown's `bake round trip:`
/// line): the submits issued, their round trip in frames from the issue to the message that
/// carried the read-back (p50 / p99 / max), the frames that carried a submit past them, and the
/// issuer's peak readback buffers. `None` when no submit was issued on the thread.
pub(crate) fn round_trip_line(r: &svet_render::BakeRoundTrip) -> Option<String> {
    if r.issued == 0 {
        return None;
    }
    let read: u64 = r.frames.iter().sum();
    let max = r.frames.iter().rposition(|n| *n > 0).unwrap_or(0);
    Some(format!(
        "bake round trip: {} submits issued, {read} read, frames p50 {} p99 {} max {max}, frames that carried a submit {}, issuer peak buffers {}",
        r.issued,
        r.percentile(50.0),
        r.percentile(99.0),
        r.carried,
        r.peak_buffers,
    ))
}

/// The render thread's laps in the bake thread's frames (a reading, the teardown's
/// `bake thread render laps:` line): each lap's p50 / p99 / max, then the frames whose whole
/// call took over 4.0 ms with the lap that was largest in each and each lap's mean over them.
pub(crate) fn render_laps_line(laps: &[[f32; 7]]) -> Option<String> {
    const NAMES: [&str; 7] = [
        "build", "take", "issue", "apply", "collect", "send", "total",
    ];
    if laps.is_empty() {
        return None;
    }
    let mut out = format!("bake thread render laps: {} frames;", laps.len());
    for (k, name) in NAMES.iter().enumerate() {
        let mut v: Vec<f64> = laps.iter().map(|l| f64::from(l[k])).collect();
        let p50 = crate::report::percentile(&mut v, 50.0);
        let p99 = crate::report::percentile(&mut v, 99.0);
        let max = v.last().copied().unwrap_or(0.0);
        out += &format!(" {name} {p50:.3}/{p99:.3}/{max:.3}");
    }
    let over: Vec<&[f32; 7]> = laps.iter().filter(|l| l[6] > 4.0).collect();
    out += &format!("; over 4.0 ms {} frames, largest lap", over.len());
    for (k, name) in NAMES[..6].iter().enumerate() {
        let n = over.iter().filter(|l| (0..6).all(|j| l[k] >= l[j])).count();
        let mean = over.iter().map(|l| f64::from(l[k])).sum::<f64>() / over.len().max(1) as f64;
        out += &format!(" {name} {n} (mean {mean:.3})");
    }
    Some(out)
}
