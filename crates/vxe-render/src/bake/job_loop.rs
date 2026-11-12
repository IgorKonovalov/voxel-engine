//! The job loop: jobs in flight, their submission room and timing, and the job table.

use super::*;

/// Slots of the candidate tables a one-off bake's jobs take ([`run_jobs`]); the live queue's
/// take the rest ([`QUEUE_SLOTS`]), so a dig's re-bake between two of its submits leaves its open
/// jobs' tables.
pub(super) const LOOP_SLOTS: usize = OPEN_JOBS - QUEUE_SLOTS;

/// Slots the live queue's open jobs take, the last of [`OPEN_JOBS`].
pub(crate) const QUEUE_SLOTS: usize = 16;

/// What one submit of a [`JobLoop`] did.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SubmitDone {
    pub samples: u64,
    /// From the submit to the readback.
    pub seconds: f64,
    /// The dispatch's milliseconds on the GPU's timestamps, where the loop times its submits.
    pub gpu_ms: Option<f64>,
}

/// Lanes of a workgroup that share an entry's samples (`BAKE_LANES` in bake.wgsl).
pub(crate) const BAKE_LANES: u32 = 64;

/// Workgroups a dispatch row holds (`BAKE_ROW` in bake.wgsl): the entries span x then y.
pub(super) const BAKE_ROW: u32 = 32_768;

/// Submits a loop holds unread at most ([`JobLoop::dispatch`]).
pub(crate) const IN_FLIGHT: usize = 2;

/// Bytes at the head of a submit's readback: its dispatch's two timestamps.
pub(super) const STAMP_BYTES: u64 = 16;

/// A submit not read back yet: its entries in dispatch order (job slot, face, rounds) and the
/// buffer its results are copied to, mapped when `ready` reads 1.
pub(super) struct InFlight {
    pub(super) entries: Vec<(usize, usize, u32)>,
    pub(super) count: u64,
    pub(super) staging: wgpu::Buffer,
    pub(super) ready: Arc<std::sync::atomic::AtomicU8>,
    pub(super) timed: bool,
    pub(super) t: Instant,
}

/// The loop's own timestamps, resolved into each submit's readback.
pub(super) struct LoopTimer {
    pub(super) queries: wgpu::QuerySet,
    pub(super) resolve: wgpu::Buffer,
    pub(super) period_ns: f32,
}

/// The job loop, resumable: jobs pending and open, the table slots free, and what it did. Each
/// [`JobLoop::dispatch`] is one dispatch, read back by a later [`JobLoop::collect`] once the GPU
/// is done with it; [`JobLoop::submit`] does both, waited on.
pub(crate) struct JobLoop {
    pub(super) pending: VecDeque<Job>,
    pub(super) open: Vec<OpenJob>,
    /// Withdrawn open jobs with rounds still in flight ([`JobLoop::withdraw`]): read back into
    /// nothing, each slot freed once its rounds are.
    pub(super) withdrawn: Vec<OpenJob>,
    pub(super) free: Vec<usize>,
    pub(super) in_flight: VecDeque<InFlight>,
    pub(super) staging: Vec<wgpu::Buffer>,
    /// Whether the loop times its submits on its own timestamps, and the timer once made
    /// (`Some(None)`: the device has no timestamps).
    pub(super) timed: bool,
    pub(super) timer: Option<Option<LoopTimer>>,
    /// The samples the last [`JobLoop::dispatch`] planned: 0 when it dispatched nothing.
    pub last_samples: u64,
    /// The light epoch the open jobs' tables were last written at ([`JobLoop::refresh_tables`]).
    pub(super) tables_at: Option<u64>,
    /// Each ended job's chunk and the samples its faces drew, where the loop keeps them
    /// (`Some`); the owner drains it.
    pub ended: Option<Vec<(ChunkCoord, u64)>>,
    pub report: BakeReport,
    pub done: BakeProgress,
}

impl JobLoop {
    /// A loop over `jobs`, its open jobs' tables in `slots`.
    pub(crate) fn new(jobs: Vec<Job>, slots: std::ops::Range<usize>) -> Self {
        let faces: u64 = jobs.iter().map(|j| j.faces.faces() as u64).sum();
        Self {
            report: BakeReport {
                chunks: jobs.len() as u64,
                ..BakeReport::default()
            },
            done: BakeProgress {
                chunks: jobs.len() as u64,
                faces,
                ..BakeProgress::default()
            },
            pending: jobs.into(),
            open: Vec::new(),
            withdrawn: Vec::new(),
            free: slots.rev().collect(),
            in_flight: VecDeque::new(),
            staging: Vec::new(),
            timed: false,
            timer: None,
            last_samples: 0,
            tables_at: None,
            ended: None,
        }
    }

    /// The loop, timing each submit on its own GPU timestamps where the device has them
    /// ([`SubmitDone::gpu_ms`]).
    pub(crate) fn timed(mut self) -> Self {
        self.timed = true;
        self
    }

    /// Adds a job behind the pending ones.
    pub(crate) fn push(&mut self, job: Job) {
        self.report.chunks += 1;
        self.done.chunks += 1;
        self.done.faces += job.faces.faces() as u64;
        self.pending.push_back(job);
    }

    /// Adds a job ahead of the pending ones.
    pub(crate) fn push_front(&mut self, job: Job) {
        self.report.chunks += 1;
        self.done.chunks += 1;
        self.done.faces += job.faces.faces() as u64;
        self.pending.push_front(job);
    }

    /// Moves the open jobs `first` holds ahead of the others, each part in its order: a dispatch
    /// plans the open jobs' rounds in order. A submit in flight finds its jobs by slot.
    pub(crate) fn promote(&mut self, first: impl Fn(ChunkCoord) -> bool) {
        self.open.sort_by_key(|j| !first(j.coord));
    }

    /// No job pending or open.
    pub(crate) fn idle(&self) -> bool {
        self.pending.is_empty() && self.open.is_empty()
    }

    /// Submits not read back yet.
    pub(crate) fn in_flight(&self) -> usize {
        self.in_flight.len()
    }

    /// The chunks of the jobs pending or open.
    pub(crate) fn chunks(&self) -> impl Iterator<Item = ChunkCoord> + '_ {
        self.pending
            .iter()
            .map(|j| j.coord)
            .chain(self.open.iter().map(|j| j.coord))
    }

    /// Takes out the pending and open jobs whose chunks `pred` holds: a pending job dropped, an
    /// open job closed with no block, its rounds in flight read back into nothing and its table
    /// slot freed once they are. Each job's chunk, place (first word and words, its block's
    /// layout at `record` words a brick record) and the samples its faces drew (read back or
    /// in flight).
    pub(crate) fn withdraw(
        &mut self,
        pred: impl Fn(ChunkCoord) -> bool,
        record: usize,
    ) -> Vec<Withdrawn> {
        let mut out = Vec::new();
        let pending = std::mem::take(&mut self.pending);
        for job in pending {
            if pred(job.coord) {
                out.push(Withdrawn {
                    coord: job.coord,
                    first_word: job.first_word,
                    words: layout(&job.faces, record).1,
                    drawn: 0,
                });
            } else {
                self.pending.push_back(job);
            }
        }
        let mut k = 0;
        while k < self.open.len() {
            if !pred(self.open[k].coord) {
                k += 1;
                continue;
            }
            let job = self.open.remove(k);
            out.push(Withdrawn {
                coord: job.coord,
                first_word: job.first_word,
                words: layout(&job.faces, record).1,
                drawn: job
                    .states
                    .iter()
                    .map(|s| u64::from(s.n) + u64::from(s.queued))
                    .sum(),
            });
            if job.states.iter().all(|s| s.queued == 0) {
                self.free.push(job.slot);
            } else {
                self.withdrawn.push(job);
            }
        }
        out
    }

    /// The pool words the pending and open jobs' places hold, at `record` words a brick record.
    pub(crate) fn place_words(&self, record: usize) -> u64 {
        let pending: u64 = self
            .pending
            .iter()
            .map(|j| layout(&j.faces, record).1)
            .sum();
        let open: u64 = self.open.iter().map(|j| layout(&j.faces, record).1).sum();
        pending + open
    }

    /// Samples the pending and open jobs still have to submit, at most.
    pub(crate) fn samples_left(&self) -> u64 {
        let live = |s: &FaceState| {
            if s.done {
                0
            } else {
                u64::from(s.n_max.saturating_sub(s.n + s.queued))
            }
        };
        let open: u64 = self
            .open
            .iter()
            .map(|j| j.states.iter().map(live).sum::<u64>())
            .sum();
        open + self.pending.iter().map(|j| j.samples).sum::<u64>()
    }

    /// [`JobLoop::samples_left`] of the jobs whose chunks `of` holds.
    pub(super) fn dig_samples_left<V>(&self, of: &std::collections::HashMap<[i32; 3], V>) -> u64 {
        let live = |s: &FaceState| {
            if s.done {
                0
            } else {
                u64::from(s.n_max.saturating_sub(s.n + s.queued))
            }
        };
        let open: u64 = self
            .open
            .iter()
            .filter(|j| of.contains_key(&j.coord.0))
            .map(|j| j.states.iter().map(live).sum::<u64>())
            .sum();
        let pending: u64 = self
            .pending
            .iter()
            .filter(|j| of.contains_key(&j.coord.0))
            .map(|j| j.samples)
            .sum();
        open + pending
    }

    /// Writes each open job's candidate table again from the ranges `run.gpu` holds now, when a
    /// sync since the last write moved, wrote or dropped a range ([`GpuWorld::light_epoch`]). A
    /// job opened since then wrote its own table from the same ranges.
    pub(crate) fn refresh_tables(&mut self, run: &BakeRun<'_>) {
        let epoch = run.gpu.light_epoch();
        if self.tables_at == Some(epoch) {
            return;
        }
        self.tables_at = Some(epoch);
        for job in &self.open {
            write_job_table(run, job.coord, job.slot);
        }
    }

    /// One submit of at most about `sample_cap` samples (and at least one entry), waited on and
    /// read back, with any submit before it: see [`JobLoop::dispatch`] and [`JobLoop::collect`].
    /// `None` when no entry was left to dispatch.
    #[allow(
        clippy::too_many_arguments,
        reason = "the frame's parts, the store, the settings, the cap and the two callbacks are independent"
    )]
    pub(crate) fn submit(
        &mut self,
        run: &BakeRun<'_>,
        bind: &wgpu::BindGroup,
        store: &mut Store,
        settings: &BakeSettings,
        sample_cap: u64,
        sink: &mut BlockSink<'_>,
        progress: &mut dyn FnMut(&BakeProgress),
    ) -> Option<SubmitDone> {
        self.collect(run, store, settings, sink, progress, 0);
        if !self.dispatch(run, bind, store, settings, sample_cap, sink, progress) {
            return None;
        }
        Some(self.collect(run, store, settings, sink, progress, 0))
    }

    /// Plans and submits one dispatch of at most about `sample_cap` samples (and at least one
    /// entry): open jobs first, then pending ones opened into free slots, each face's rounds
    /// from the first sample no submit has drawn yet. Its results are copied to a readback
    /// buffer and mapped; nothing waits. A job with nothing to draw ends here, its block to
    /// `sink`. False when no entry was left to dispatch.
    #[allow(
        clippy::too_many_arguments,
        reason = "the frame's parts, the store, the settings, the cap and the two callbacks are independent"
    )]
    pub(crate) fn dispatch(
        &mut self,
        run: &BakeRun<'_>,
        bind: &wgpu::BindGroup,
        store: &mut Store,
        settings: &BakeSettings,
        sample_cap: u64,
        sink: &mut BlockSink<'_>,
        progress: &mut dyn FnMut(&BakeProgress),
    ) -> bool {
        self.start_timer(run);
        let round = settings.round.max(1);
        let per_submit = (SUBMIT_SAMPLES / u64::from(round)).max(1) as usize;
        let batch = settings
            .batch_faces
            .clamp(1, BAKE_BATCH_FACES)
            .min(per_submit);
        let ctx = JobCtx {
            run,
            settings,
            round,
            rounds_cap: if settings.submit_rounds == 0 {
                u32::MAX
            } else {
                settings.submit_rounds
            },
        };
        let (budget, plan) = self.plan_rounds(&ctx, store, batch, sample_cap, sink, progress);
        if budget.entries == 0 {
            self.last_samples = 0;
            return false;
        }
        self.last_samples = budget.samples;
        let (input, entries, jobs_in) = self.fill_entries(&plan, round, budget.entries);
        self.send(run, bind, settings, &input, entries, jobs_in);
        true
    }

    /// The loop's own timestamp queries, made at its first timed dispatch where the device
    /// has them.
    fn start_timer(&mut self, run: &BakeRun<'_>) {
        let (device, queue) = (run.device, run.queue);
        if self.timed && self.timer.is_none() {
            self.timer = Some(
                device
                    .features()
                    .contains(wgpu::Features::TIMESTAMP_QUERY)
                    .then(|| LoopTimer {
                        queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                            label: Some("bake loop timer"),
                            ty: wgpu::QueryType::Timestamp,
                            count: 2,
                        }),
                        resolve: device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some("bake loop timer resolve"),
                            size: STAMP_BYTES,
                            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                            mapped_at_creation: false,
                        }),
                        period_ns: queue.get_timestamp_period(),
                    }),
            );
        }
    }

    /// The rounds each live face takes in a submit of at most about `sample_cap` samples and
    /// `batch` entries: the open jobs' first, then pending jobs opened into free slots (a job
    /// with nothing to draw ends into `sink`), then further rounds while room is left.
    fn plan_rounds(
        &mut self,
        ctx: &JobCtx<'_, '_>,
        store: &mut Store,
        batch: usize,
        sample_cap: u64,
        sink: &mut BlockSink<'_>,
        progress: &mut dyn FnMut(&BakeProgress),
    ) -> (SubmitRoom, Vec<Vec<(usize, u32)>>) {
        let (settings, round) = (ctx.settings, ctx.round);
        let report = &mut self.report;
        let done = &mut self.done;
        let open = &mut self.open;
        // The rounds each live face takes in this submit: first those that cannot end it.
        let mut budget = SubmitRoom {
            entries: 0,
            batch,
            samples: 0,
            cap: sample_cap,
        };
        let mut plan: Vec<Vec<(usize, u32)>> =
            open.iter().map(|job| ctx.plan(job, &mut budget)).collect();
        while budget.room() {
            let Some(slot) = self.free.pop() else { break };
            let Some(next) = self.pending.pop_front() else {
                self.free.push(slot);
                break;
            };
            let job = ctx.open(next, slot);
            if job.live.is_empty() {
                self.free.push(slot);
                let (coord, first, block) = ctx.close(job, report);
                sink(store, coord, first, block);
                done.chunks_done += 1;
                progress(done);
                continue;
            }
            plan.push(ctx.plan(&job, &mut budget));
            open.push(job);
        }
        // Room left and no job to open: further rounds, a layer at a time, in order, each face
        // up to the rounds its standard error now expects it to need.
        while budget.room() && self.pending.is_empty() {
            let before = budget.entries;
            for (job, p) in open.iter().zip(&mut plan) {
                for (i, r) in p.iter_mut() {
                    if !budget.room() {
                        break;
                    }
                    let s = &job.states[*i];
                    let expected = s.expected_rounds(round, settings.clamp, settings.se).max(1);
                    if *r < s.rounds_left(round).min(ctx.rounds_cap).min(expected) {
                        budget.take(s.round_samples(*r, round));
                        *r += 1;
                    }
                }
            }
            if budget.entries == before {
                break;
            }
        }
        (budget, plan)
    }

    /// The dispatch's entries by the plan: each names its job's slot and its face's samples
    /// from the first no submit has drawn. Returns the input rows, the entries'
    /// `(slot, face, rounds)` and the count of jobs with an entry.
    fn fill_entries(
        &mut self,
        plan: &[Vec<(usize, u32)>],
        round: u32,
        capacity: usize,
    ) -> (Vec<[u32; 20]>, Vec<(usize, usize, u32)>, u64) {
        let report = &mut self.report;
        // One dispatch: each entry names its job's slot, and its face's samples from the first
        // no submit has drawn.
        let mut input: Vec<[u32; 20]> = Vec::with_capacity(capacity);
        let mut entries = Vec::new();
        let mut jobs_in = 0u64;
        for (job, p) in self.open.iter_mut().zip(plan) {
            jobs_in += u64::from(p.iter().any(|&(_, r)| r > 0));
            for &(i, r) in p {
                if r == 0 {
                    continue;
                }
                let s = &mut job.states[i];
                report.max_rounds_in_submit = report.max_rounds_in_submit.max(r);
                for _ in 0..r {
                    let first = s.n + s.queued;
                    let samples = round.min(s.n_max - first);
                    input.push(s.input(first, samples, job.slot));
                    s.queued += samples;
                }
                entries.push((job.slot, i, r));
            }
        }
        (input, entries, jobs_in)
    }

    /// Writes the dispatch's input, records the pass and its readback, submits and maps it,
    /// and puts it in flight.
    fn send(
        &mut self,
        run: &BakeRun<'_>,
        bind: &wgpu::BindGroup,
        settings: &BakeSettings,
        input: &[[u32; 20]],
        entries: Vec<(usize, usize, u32)>,
        jobs_in: u64,
    ) {
        let (device, queue) = (run.device, run.queue);
        let report = &mut self.report;
        let count = input.len() as u32;
        let uniform = [
            count,
            settings.seed,
            ((run.gpu.light_radius() * 64) as f32).to_bits(),
            u32::from(settings.clamp),
            0,
            0,
            0,
            0,
        ];
        queue.write_buffer(&run.pipeline.job, 0, bytemuck::cast_slice(&uniform));
        queue.write_buffer(&run.pipeline.faces, 0, bytemuck::cast_slice(input));
        let own = self.timer.as_ref().and_then(Option::as_ref);
        let size = STAMP_BYTES + u64::from(count) * FACE_BYTES;
        let staging = match self.staging.iter().position(|b| b.size() >= size) {
            Some(k) => self.staging.swap_remove(k),
            None => device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("bake readback"),
                size: STAMP_BYTES + BAKE_BATCH_FACES as u64 * FACE_BYTES,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
        };
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let writes = match own {
                Some(t) => Some(wgpu::ComputePassTimestampWrites {
                    query_set: &t.queries,
                    beginning_of_pass_write_index: Some(0),
                    end_of_pass_write_index: Some(1),
                }),
                None => settings.timer.as_ref().map(|t| t.writes()),
            };
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("bake"),
                timestamp_writes: writes,
            });
            pass.set_pipeline(&run.pipeline.pipeline);
            pass.set_bind_group(0, run.world_bind, &[]);
            pass.set_bind_group(3, bind, &[]);
            pass.dispatch_workgroups(count.min(BAKE_ROW), count.div_ceil(BAKE_ROW), 1);
        }
        match own {
            Some(t) => {
                enc.resolve_query_set(&t.queries, 0..2, &t.resolve, 0);
                enc.copy_buffer_to_buffer(&t.resolve, 0, &staging, 0, STAMP_BYTES);
            }
            None => {
                if let Some(timer) = &settings.timer {
                    timer.resolve(&mut enc);
                }
            }
        }
        enc.copy_buffer_to_buffer(
            &run.pipeline.faces,
            0,
            &staging,
            STAMP_BYTES,
            u64::from(count) * FACE_BYTES,
        );
        report.max_jobs_in_submit = report.max_jobs_in_submit.max(jobs_in);
        let t = stopwatch();
        queue.submit([enc.finish()]);
        let ready = Arc::new(std::sync::atomic::AtomicU8::new(0));
        let flag = ready.clone();
        staging.map_async(wgpu::MapMode::Read, ..size, move |r| {
            flag.store(
                if r.is_ok() { 1 } else { 2 },
                std::sync::atomic::Ordering::Release,
            );
        });
        self.in_flight.push_back(InFlight {
            entries,
            count: u64::from(count),
            staging,
            ready,
            timed: own.is_some(),
            t,
        });
    }

    /// Reads back the submits whose results are mapped, oldest first, stopping at the first
    /// that is not, after polling the device until at most `unread` are left unread: each face's
    /// rounds added in sample order, its stop read after each, and each ended job's block to
    /// `sink`. The sum of what the submits read did; zero samples when none was read.
    pub(crate) fn collect(
        &mut self,
        run: &BakeRun<'_>,
        store: &mut Store,
        settings: &BakeSettings,
        sink: &mut BlockSink<'_>,
        progress: &mut dyn FnMut(&BakeProgress),
        unread: usize,
    ) -> SubmitDone {
        use std::sync::atomic::Ordering;
        let device = run.device;
        let mut out = SubmitDone::default();
        while let Some(front) = self.in_flight.front() {
            if front.ready.load(Ordering::Acquire) == 0 {
                if self.in_flight.len() <= unread {
                    break;
                }
                let _ = device.poll(wgpu::PollType::wait_indefinitely());
                continue;
            }
            let f = self.in_flight.pop_front().expect("front read above");
            let mapped = f.ready.load(Ordering::Acquire) == 1;
            let seconds = seconds_since(f.t);
            let size = STAMP_BYTES + f.count * FACE_BYTES;
            let bytes: Vec<u8> = if mapped {
                let b = f
                    .staging
                    .get_mapped_range(..size)
                    .expect("bake readback mapped")
                    .to_vec();
                f.staging.unmap();
                b
            } else {
                vec![0; size as usize]
            };
            self.staging.push(f.staging);
            let gpu_ms = (f.timed && mapped).then(|| {
                let t: &[u64] = bytemuck::cast_slice(&bytes[..STAMP_BYTES as usize]);
                let period = self
                    .timer
                    .as_ref()
                    .and_then(Option::as_ref)
                    .map_or(1.0, |t| t.period_ns);
                t[1].saturating_sub(t[0]) as f64 * f64::from(period) / 1e6
            });
            let report = &mut self.report;
            report.gpu_seconds += seconds;
            if !f.timed
                && let Some(timer) = &settings.timer
            {
                report.dispatch_seconds += timer.read_ms(device) / 1e3;
            }
            report.submits += 1;
            let samples_before = report.samples;
            // Each face's rounds in order: the stop is read after each, and a round past it is
            // waste.
            let out_words: &[u32] = bytemuck::cast_slice(&bytes[STAMP_BYTES as usize..]);
            let mut e = 0usize;
            let mut at = 0usize;
            for &(slot, i, r) in &f.entries {
                if self.open.get(at).is_none_or(|j| j.slot != slot) {
                    match self.open.iter().position(|j| j.slot == slot) {
                        Some(k) => at = k,
                        None => {
                            // A withdrawn job's rounds: read into nothing.
                            let z = self
                                .withdrawn
                                .iter_mut()
                                .find(|j| j.slot == slot)
                                .expect("a job with entries in flight stays open or withdrawn");
                            for _ in 0..r {
                                let samples = out_words[e * 20 + 5];
                                e += 1;
                                z.states[i].queued -= samples;
                                report.wasted += u64::from(samples);
                            }
                            continue;
                        }
                    }
                }
                let s = &mut self.open[at].states[i];
                for _ in 0..r {
                    let words = &out_words[e * 20..(e + 1) * 20];
                    e += 1;
                    let samples = words[5];
                    s.queued -= samples;
                    // A readback that failed to map draws its rounds again in a later submit,
                    // from the face's first sample not drawn.
                    if s.done || !mapped {
                        report.wasted += u64::from(samples);
                        continue;
                    }
                    report.rays += u64::from(words[15]);
                    report.ray_len += f64::from(f32::from_bits(words[16]));
                    report.samples += u64::from(samples);
                    s.add(words, samples);
                    if settings.snapshots.contains(&s.n) {
                        let (a, b) = (s.mean(s.sum), s.mean(s.clamped));
                        s.snapshots.push((s.n, a, b));
                    }
                    let reached = s.n >= s.n_min && s.se(settings.clamp) <= f64::from(settings.se);
                    if reached || s.n >= s.n_max {
                        s.done = true;
                        report.faces_at_min += u64::from(s.n <= s.n_min);
                        report.faces_at_max += u64::from(s.n >= s.n_max);
                        self.done.faces_done += 1;
                    }
                }
            }
            // A withdrawn job whose rounds are all read back frees its slot.
            let free = &mut self.free;
            self.withdrawn.retain(|j| {
                let drained = j.states.iter().all(|s| s.queued == 0);
                if drained {
                    free.push(j.slot);
                }
                !drained
            });
            self.done.samples = report.samples;
            self.done.gpu_seconds = report.gpu_seconds;
            out.samples += report.samples - samples_before;
            out.seconds += seconds;
            if let Some(ms) = gpu_ms {
                *out.gpu_ms.get_or_insert(0.0) += ms;
            }
            self.close_ended(run, store, settings, sink);
            progress(&self.done);
        }
        out
    }

    /// Closes each open job whose faces are all done and none of whose rounds is in flight.
    pub(super) fn close_ended(
        &mut self,
        run: &BakeRun<'_>,
        store: &mut Store,
        settings: &BakeSettings,
        sink: &mut BlockSink<'_>,
    ) {
        let ctx = JobCtx {
            run,
            settings,
            round: settings.round.max(1),
            rounds_cap: 0,
        };
        for job in self.open.iter_mut() {
            let states = &job.states;
            job.live.retain(|&i| !states[i].done);
        }
        let mut k = 0;
        while k < self.open.len() {
            let j = &self.open[k];
            if j.live.is_empty() && j.states.iter().all(|s| s.queued == 0) {
                let job = self.open.remove(k);
                self.free.push(job.slot);
                if let Some(ended) = self.ended.as_mut() {
                    let drawn = job.states.iter().map(|s| u64::from(s.n)).sum();
                    ended.push((job.coord, drawn));
                }
                let (coord, first, block) = ctx.close(job, &mut self.report);
                sink(store, coord, first, block);
                self.done.chunks_done += 1;
            } else {
                k += 1;
            }
        }
    }
}

/// A job taken out of a [`JobLoop`] ([`JobLoop::withdraw`]): its chunk, its place and the
/// samples its faces drew.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Withdrawn {
    pub coord: ChunkCoord,
    pub first_word: u64,
    pub words: u64,
    pub drawn: u64,
}

/// Where an ended job's block goes: the store, its chunk, its place and its words.
pub(crate) type BlockSink<'a> = dyn FnMut(&mut Store, ChunkCoord, u64, Vec<u32>) + 'a;

/// A submit's room: its entries against the batch, its samples against the cap.
pub(super) struct SubmitRoom {
    pub(super) entries: usize,
    pub(super) batch: usize,
    pub(super) samples: u64,
    pub(super) cap: u64,
}

impl SubmitRoom {
    /// Another entry fits: under the batch, and under the cap or with none taken yet.
    pub(super) fn room(&self) -> bool {
        self.entries < self.batch && (self.samples < self.cap || self.entries == 0)
    }

    pub(super) fn take(&mut self, samples: u32) {
        self.entries += 1;
        self.samples += u64::from(samples);
    }
}

/// Stages `coord`'s candidate table and its chunk in table slot `slot`.
pub(super) fn write_job_table(run: &BakeRun<'_>, coord: ChunkCoord, slot: usize) {
    let table = job_table(run.gpu, coord);
    let used = (1 + table.count as usize) * 16;
    let at = slot as u64 * TABLE_SLOT_BYTES;
    let tables = &run.pipeline.tables;
    run.queue
        .write_buffer(tables, at, &bytemuck::bytes_of(&table)[..used]);
    let chunk = [coord.0[0], coord.0[1], coord.0[2], 0];
    run.queue.write_buffer(
        tables,
        at + (TABLE_SLOT_WORDS - 1) * 16,
        bytemuck::cast_slice(&chunk),
    );
}

/// A chunk's job while its faces run.
pub(super) struct OpenJob {
    pub(super) coord: ChunkCoord,
    pub(super) faces: ChunkFaces,
    pub(super) first_word: u64,
    pub(super) kept_bricks: KeptBricks,
    pub(super) states: Vec<FaceState>,
    /// The faces still running.
    pub(super) live: Vec<usize>,
    pub(super) slot: usize,
}

/// What opening, planning and closing a job read of the bake.
pub(super) struct JobCtx<'a, 'b> {
    pub(super) run: &'a BakeRun<'b>,
    pub(super) settings: &'a BakeSettings,
    pub(super) round: u32,
    pub(super) rounds_cap: u32,
}

impl JobCtx<'_, '_> {
    /// The job at `coord`: its faces' states, and its parameters and table staged in `slot`.
    pub(super) fn open(&self, job: Job, slot: usize) -> OpenJob {
        let Job {
            coord,
            faces,
            first_word,
            kept_bricks,
            samples: _,
            brick_samples,
        } = job;
        write_job_table(self.run, coord, slot);
        // A job's own count (the live queue's coarse pass) on cells of a brick draws each
        // direction's samples on `ceil(count / BAKE_LANES)` of its faces, spread evenly through
        // them in key order, so each entry fills its workgroup; the record is their mean.
        let subset = brick_samples.is_some() && self.settings.cell == 8;
        let brick_samples = brick_samples.unwrap_or(self.settings.brick_samples);
        let mut states: Vec<FaceState> = Vec::with_capacity(faces.faces());
        for (brick_at, keys) in &faces.bricks {
            let b = brick_of_slot(usize::from(*brick_at));
            let record_kept = kept_bricks.contains_key(brick_at);
            // A brick-level direction's samples, spread over its faces, a translucent voxel's
            // among them.
            let mut per_dir = [0u32; 6];
            for &key in keys {
                per_dir[usize::from(key & 7)] += 1;
            }
            // Under `subset`, a direction's `m` faces drawn of its `k`: the i-th face is
            // drawn where `(i m) mod k < m`, which holds for exactly `m` of them.
            let drawn = per_dir.map(|k| {
                if subset {
                    brick_samples.div_ceil(BAKE_LANES).min(k)
                } else {
                    k
                }
            });
            let mut at_dir = [0u32; 6];
            for &key in keys {
                let i = i32::from(key >> 3);
                let l = [i % 8, i / 8 % 8, i / 64];
                let voxel = [0, 1, 2].map(|k| coord.0[k] * 64 + b[k] * 8 + l[k]);
                let face = (key & 7) as u8;
                let d = usize::from(face);
                let n = brick_samples.div_ceil(drawn[d].max(1));
                let in_subset = {
                    let i = at_dir[d];
                    at_dir[d] += 1;
                    u64::from(i) * u64::from(drawn[d]) % u64::from(per_dir[d].max(1))
                        < u64::from(drawn[d])
                };
                let wanted = in_subset
                    && !record_kept
                    && self.settings.filter.as_ref().is_none_or(|f| f(voxel, face));
                states.push(FaceState {
                    voxel,
                    face,
                    n_min: n,
                    n_max: n,
                    n: 0,
                    queued: 0,
                    sum: [0.0; 3],
                    clamped: [0.0; 3],
                    lum2: 0.0,
                    best: 0.0,
                    material: 0,
                    snapshots: Vec::new(),
                    done: !wanted,
                });
            }
        }
        let live = (0..states.len()).filter(|&i| !states[i].done).collect();
        OpenJob {
            coord,
            faces,
            first_word,
            kept_bricks,
            states,
            live,
            slot,
        }
    }

    /// The rounds of `job`'s live faces that cannot end them, while `budget` has room.
    pub(super) fn plan(&self, job: &OpenJob, budget: &mut SubmitRoom) -> Vec<(usize, u32)> {
        let mut p = Vec::new();
        for &i in &job.live {
            if !budget.room() {
                break;
            }
            let s = &job.states[i];
            let want = s
                .certain_rounds(self.round)
                .min(s.rounds_left(self.round))
                .min(self.rounds_cap);
            let mut r = 0;
            while r < want && budget.room() {
                budget.take(s.round_samples(r, self.round));
                r += 1;
            }
            p.push((i, r));
        }
        p
    }

    /// The ended job's chunk, place and block: the directory, then each surface brick's record (a
    /// kept one, or the mean of its baked faces).
    pub(super) fn close(
        &self,
        job: OpenJob,
        report: &mut BakeReport,
    ) -> (ChunkCoord, u64, Vec<u32>) {
        let clamp = self.settings.clamp;
        let cell = self.settings.cell;
        let (dir, words) = layout(&job.faces, record_words(cell));
        let mut block: Vec<u32> = dir;
        block.reserve(words as usize);
        let mut at = 0usize;
        for (slot, keys) in &job.faces.bricks {
            let states = &job.states[at..at + keys.len()];
            at += keys.len();
            let record = job
                .kept_bricks
                .get(slot)
                .cloned()
                .unwrap_or_else(|| brick_record(states, clamp, cell));
            block.extend(record);
        }
        debug_assert_eq!(block.len() as u64, words);
        report.faces += job.states.iter().filter(|s| s.n > 0).count() as u64;
        if self.settings.keep_faces {
            report
                .outcomes
                .extend(job.states.iter().filter(|s| s.n > 0).map(|s| FaceOutcome {
                    voxel: s.voxel,
                    face: s.face,
                    samples: s.n,
                    mean: [s.mean(s.sum), s.mean(s.clamped)],
                    se: s.se(self.settings.clamp) as f32,
                    material: s.material,
                    snapshots: s.snapshots.clone(),
                }));
        }
        (job.coord, job.first_word, block)
    }
}
