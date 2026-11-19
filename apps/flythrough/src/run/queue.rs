//! The live queue in the run: the pre-roll, the per-frame step, its watch on digs and its
//! printed lines.

use crate::app::ms_since;
use crate::now;
use crate::percentile;
use crate::run::Run;
use crate::run::hud_feed::write_queue_banner_row;
use vxe_app::Gpu;
use vxe_render::wgpu;
use vxe_world::ChunkCoord;

/// Samples a step of the live queue's pre-roll draws: about one chunk's brick level at 4096, so
/// the jobs it opens stay near the camera.
pub(crate) const PRE_ROLL_SAMPLES: u64 = 2_000_000;

/// Milliseconds the pre-roll's coarse pass runs at most past the near chunks: a streamed world's
/// mirror holds about 17 000 full chunks within its full radius, which the live queue then takes
/// coarse first; the level's 3168 take about 4 s on the RTX.
pub(crate) const COARSE_PRE_ROLL_MS: f64 = 5000.0;

/// The live queue's state in the fly-through: the workers' results before the frame exists, and
/// what its stdout line reads.
#[derive(Default)]
pub(crate) struct QueueRun {
    pub(crate) prepared: Vec<(ChunkCoord, vxe_world::Prepared)>,
    /// Ms since the start at the last line, and the chunks baked and samples then.
    pub(crate) line_ms: f64,
    pub(crate) line_frame: u32,
    pub(crate) line_baked: u64,
    pub(crate) line_samples: u64,
    pub(crate) line_prepared: u64,
    /// The submits' ms since the last line.
    pub(crate) slices: Vec<f64>,
    pub(crate) emptied: bool,
    pub(crate) stats: Option<vxe_render::QueueStats>,
    pub(crate) failed: bool,
    /// Ms since the start at the last step, and the dig being watched.
    pub(crate) step_ms: Option<f64>,
    pub(crate) dig: Option<DigWatch>,
    /// The CPU ms of each `bake_queue_step` since the last line, and of the last one.
    pub(crate) step_cpu: Vec<f64>,
    pub(crate) last_step_cpu: f64,
    /// The steps' stages since the last line, and over the run by the regime of their second
    /// (the `frames:` summary's `queue step` lines).
    pub(crate) line_stages: Vec<vxe_render::debug::StepStages>,
    pub(crate) tally: vxe_render::debug::StepTally,
    /// The jobs and the openings at the last line, and the last line's step p50 (the HUD's).
    pub(crate) line_openings: u64,
    /// [`vxe_render::QueueStats::starved_openings`] at the last line.
    pub(crate) line_starved: u64,
    pub(crate) line_step_p50: f64,
    /// The samples by view class and passed at the last line.
    pub(crate) line_view: [u64; 3],
    pub(crate) line_passed: u64,
}

/// The per-second `bake queue step:` line: the second's regime from its samples a baked chunk
/// (its steps tagged with it in the run's tally), the bake's GPU ms a second, the steps that
/// opened, the step's p50 on opening and other steps, the in-view waiting chunks whose reach
/// crosses the full radius, the `reach` mode with its partial blocks, partial jobs opened and
/// re-queues since the start, and each stage's mean ms a step. Reads the second's counters from
/// `q`'s last line and takes its steps.
pub(crate) fn queue_step_line(
    q: &mut QueueRun,
    st: &vxe_render::QueueStats,
    at_s: f64,
    secs: f64,
    bake_ms_per_s: f64,
    reach: vxe_render::ReachMode,
) -> String {
    use std::fmt::Write as _;
    use vxe_render::{debug::Regime, debug::STEP_STAGES, internals::VIEW_IN};
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
    let opened = steps.iter().filter(|s| s.opened).count();
    let [blank, coarse] = st.view_waiting[VIEW_IN];
    let mut line = format!(
        "bake queue step: at {at_s:.1} s, regime {} ({} samples a baked chunk, +{baked} chunks in {secs:.1} s, jobs {}, coarse {}), bake {bake_ms_per_s:.1} ms/s, opened {opened} of {} steps ({} since the start, {} starved), step p50 {:.3} ms (open {:.3}, not {:.3}), reach-bound {} of {} in view waiting, reach {} ({} partial, {} opened partial, {} re-queued); stage means ms:",
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
        st.reach_bound,
        blank + coarse,
        reach.name(),
        st.partial,
        st.partial_opened,
        st.partial_requeued,
    );
    q.line_openings = st.openings;
    q.line_starved = st.starved_openings;
    let n = steps.len().max(1) as f64;
    for (k, stage) in STEP_STAGES.iter().enumerate() {
        let mean = steps.iter().map(|s| s.ms[k]).sum::<f64>() / n;
        let _ = write!(line, " {stage} {mean:.3}");
    }
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
    pub(crate) from: vxe_render::QueueStats,
}

/// The camera the run starts at: the castle's shot, the showcase's shot, `--pose`'s still
/// camera, or the spawn facing +x.
/// The live queue's view angles for a camera of `vfov` drawn at `size` (output pixels): the
/// cone's half-angle (the wider of the two) and the frustum's horizontal and vertical ones.
pub(crate) fn queue_view_angles(vfov: f32, size: [u32; 2]) -> (f32, [f32; 2]) {
    let aspect = size[0].max(1) as f32 / size[1].max(1) as f32;
    (
        vxe_render::internals::view_half_angle(vfov, aspect),
        vxe_render::internals::view_half_angles(vfov, aspect),
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
    st: &vxe_render::QueueStats,
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
        if gfx.frame.bake_queue().is_none() {
            return enc;
        }
        gpu.queue.submit([enc.finish()]);
        let (forward, _, _) = self.fly.camera.basis();
        let stream = self.stream.as_ref();
        let resident_fn = |c: ChunkCoord| {
            stream.is_some_and(|s| s.streamer.tier(c) == Some(vxe_world::Tier::Full))
        };
        let resident: Option<&dyn Fn(ChunkCoord) -> bool> = stream
            .is_some()
            .then_some(&resident_fn as &dyn Fn(ChunkCoord) -> bool);
        let (half_angle, frustum) =
            queue_view_angles(self.fly.camera.vfov, gfx.frame.output_size());
        let t_step = ms_since(self.start);
        let dt = self.queue.step_ms.replace(t_step).map(|at| t_step - at);
        let view = vxe_render::QueueView {
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
        match stepped {
            Some(Ok(st)) => {
                if st.samples > 0 {
                    self.queue.slices.push(st.ms);
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
                if !self.queue.emptied
                    && gfx
                        .frame
                        .bake_queue()
                        .is_some_and(vxe_render::BakeQueue::empty)
                {
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
        self.print_queue_line(gpu);
        gpu.device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default())
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
                .map_or((vxe_render::StoreStats::default(), None), |(s, age)| {
                    (s, Some(age))
                });
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
            println!(
                "bake queue: at {:.1} s, frame {}, depth {}, jobs {}, waiting reach {} lights {}, baked {} (+{} in {:.1} s, {} from the workers), {:.3e} samples/s, slice p50 {:.2} max {:.2} ms over {} submits, {:.1} ns a sample, fading {}, coarse {} chunks, {}, eye {:.0} {:.0} {:.0}, step CPU p50 {:.3} max {:.3} ms, provisional {} (opened {}, catch-ups {})",
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
            );
            let needs = gfx
                .frame
                .bake_queue_mut()
                .map(|q| {
                    // The next second's line and the HUD's row read the next opening's census.
                    q.request_census();
                    q.take_angle_needs()
                })
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
