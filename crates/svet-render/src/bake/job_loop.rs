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
    /// Of `samples`, those of jobs at the full count and those at a job's own count (the
    /// coarse pass), and `gpu_ms` split between them by those samples: an attribution of the
    /// dispatch's time by its share of the samples, not a measurement of each class.
    pub class_samples: [u64; 2],
    pub class_ms: [f64; 2],
}

/// Lanes of a workgroup that share an entry's samples (`BAKE_LANES` in bake.wgsl).
pub(crate) const BAKE_LANES: u32 = 64;

/// Workgroups a dispatch row holds (`BAKE_ROW` in bake.wgsl): the entries span x then y.
pub(crate) const BAKE_ROW: u32 = 32_768;

/// Submits a loop holds unread at most ([`JobLoop::dispatch`]).
pub(crate) const IN_FLIGHT: usize = 2;

/// Bytes at the head of a submit's readback: its dispatch's two timestamps.
pub(super) const STAMP_BYTES: u64 = 16;

/// A submit's entries: each `(job slot, face, rounds)`.
pub(super) type Entries = Vec<(usize, usize, u32)>;

/// A submit planned and not read back yet: its entries in dispatch order (job slot, face,
/// rounds); the render half holds its buffers ([`Issuer`]).
pub(super) struct Planned {
    pub(super) entries: Entries,
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
    pub(super) in_flight: VecDeque<Planned>,
    /// The jobs opened since the last submit went out, `(slot, chunk)`: the render half writes
    /// their candidate tables ahead of it.
    pub(super) new_tables: Vec<(usize, ChunkCoord)>,
    /// The newest frame message the bake half has read: a job is stamped with it as it joins
    /// the loop ([`JobLoop::stamps`]).
    pub stamp: u64,
    /// Each pending or open job's stamp, by chunk: a change in a later message stales its
    /// result.
    pub(super) stamps: std::collections::HashMap<[i32; 3], u64>,
    /// The samples the last [`JobLoop::dispatch`] planned: 0 when it dispatched nothing.
    pub last_samples: u64,
    /// The dispatches' plans that stopped at the sample cap, at the entries' batch, and out of
    /// rounds to plan, since the loop started.
    pub plan_stops: [u64; 3],
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
            new_tables: Vec::new(),
            stamp: 0,
            stamps: Default::default(),
            last_samples: 0,
            plan_stops: [0; 3],
            ended: None,
        }
    }

    /// Adds a job behind the pending ones.
    pub(crate) fn push(&mut self, job: Job) {
        self.report.chunks += 1;
        self.done.chunks += 1;
        self.done.faces += job.faces.faces() as u64;
        self.stamps.insert(job.coord.0, self.stamp);
        self.pending.push_back(job);
    }

    /// Adds a job ahead of the pending ones.
    pub(crate) fn push_front(&mut self, job: Job) {
        self.report.chunks += 1;
        self.done.chunks += 1;
        self.done.faces += job.faces.faces() as u64;
        self.stamps.insert(job.coord.0, self.stamp);
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

    /// The chunks of the jobs pending, not yet opened.
    pub(crate) fn pending_chunks(&self) -> impl Iterator<Item = ChunkCoord> + '_ {
        self.pending.iter().map(|j| j.coord)
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
                self.stamps.remove(&job.coord.0);
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
            self.stamps.remove(&job.coord.0);
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
    pub(super) fn dig_samples_left(&self, of: impl Fn(&[i32; 3]) -> bool) -> u64 {
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
            .filter(|j| of(&j.coord.0))
            .map(|j| j.states.iter().map(live).sum::<u64>())
            .sum();
        let pending: u64 = self
            .pending
            .iter()
            .filter(|j| of(&j.coord.0))
            .map(|j| j.samples)
            .sum();
        open + pending
    }

    /// The open jobs' slots and chunks, for the render half's table refresh.
    pub(crate) fn open_tables(&self) -> Vec<(usize, ChunkCoord)> {
        self.open.iter().map(|j| (j.slot, j.coord)).collect()
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
        half: &mut dyn RenderHalf,
        store: &mut Store,
        settings: &BakeSettings,
        sample_cap: u64,
        sink: &mut BlockSink<'_>,
        progress: &mut dyn FnMut(&BakeProgress),
    ) -> Option<SubmitDone> {
        self.collect(run, half, store, settings, sink, progress, 0);
        if !self.dispatch(run, half, store, settings, sample_cap, sink, progress) {
            return None;
        }
        Some(self.collect(run, half, store, settings, sink, progress, 0))
    }

    /// Plans one dispatch of at most about `sample_cap` samples (and at least one entry): open
    /// jobs first, then pending ones opened into free slots, each face's rounds from the first
    /// sample no submit has drawn yet; the render half issues it ([`RenderHalf::issue`]), the
    /// opened jobs' tables first. A job with nothing to draw ends here, its block to `sink`.
    /// False when no entry was left to dispatch.
    #[allow(
        clippy::too_many_arguments,
        reason = "the frame's parts, the store, the settings, the cap and the two callbacks are independent"
    )]
    pub(crate) fn dispatch(
        &mut self,
        run: &BakeRun<'_>,
        half: &mut dyn RenderHalf,
        store: &mut Store,
        settings: &BakeSettings,
        sample_cap: u64,
        sink: &mut BlockSink<'_>,
        progress: &mut dyn FnMut(&BakeProgress),
    ) -> bool {
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
        half.tables(&std::mem::take(&mut self.new_tables));
        if budget.entries == 0 {
            self.last_samples = 0;
            return false;
        }
        self.last_samples = budget.samples;
        let stop = if budget.samples >= budget.cap {
            0
        } else if budget.entries >= budget.batch {
            1
        } else {
            2
        };
        self.plan_stops[stop] += 1;
        let (input, entries, jobs_in) = self.fill_entries(&plan, round, budget.entries);
        self.report.max_jobs_in_submit = self.report.max_jobs_in_submit.max(jobs_in);
        half.issue(Prepared {
            input,
            seed: settings.seed,
            clamp: settings.clamp,
            sky: run.sky,
            key: run.key,
            timer: settings.timer.clone(),
        });
        self.in_flight.push_back(Planned { entries });
        true
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
            self.new_tables.push((slot, job.coord));
            if job.live.is_empty() {
                self.free.push(slot);
                let (coord, first, block) = ctx.close(job, report);
                let stamp = self.stamps.remove(&coord.0).unwrap_or(self.stamp);
                sink(store, coord, first, block, stamp);
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

    /// The dispatch's entries from `plan`'s rounds per face: each names its job's slot and its
    /// face's samples from the first no submit has drawn. Returns the input rows, the entries'
    /// `(slot, face, rounds)` and the count of jobs with an entry.
    fn fill_entries(
        &mut self,
        plan: &[Vec<(usize, u32)>],
        round: u32,
        capacity: usize,
    ) -> (Vec<[u32; FACE_WORDS]>, Entries, u64) {
        let report = &mut self.report;
        // One dispatch: each entry names its job's slot, and its face's samples from the first
        // no submit has drawn.
        let mut input: Vec<[u32; FACE_WORDS]> = Vec::with_capacity(capacity);
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

    /// Reads back the submits whose results are mapped, oldest first, stopping at the first
    /// that is not, after the render half waits until at most `unread` are left unread: each
    /// face's rounds added in sample order, its stop read after each, and each ended job's block
    /// to `sink`. The sum of what the submits read did; zero samples when none was read.
    #[allow(
        clippy::too_many_arguments,
        reason = "the frame's parts, the store, the settings, the two callbacks and the wait are independent"
    )]
    pub(crate) fn collect(
        &mut self,
        run: &BakeRun<'_>,
        half: &mut dyn RenderHalf,
        store: &mut Store,
        settings: &BakeSettings,
        sink: &mut BlockSink<'_>,
        progress: &mut dyn FnMut(&BakeProgress),
        unread: usize,
    ) -> SubmitDone {
        let mut out = SubmitDone::default();
        while !self.in_flight.is_empty() {
            let Some(rb) = half.collect(unread) else {
                break;
            };
            let f = self
                .in_flight
                .pop_front()
                .expect("one planned a submit read back");
            let (mapped, seconds, gpu_ms) = (rb.mapped, rb.seconds, rb.gpu_ms);
            let report = &mut self.report;
            report.gpu_seconds += seconds;
            if let Some(ms) = rb.timer_ms {
                report.dispatch_seconds += ms / 1e3;
            }
            report.submits += 1;
            let samples_before = report.samples;
            // Each face's rounds in order: the stop is read after each, and a round past it is
            // waste.
            let out_words: &[u32] = &rb.words;
            let mut e = 0usize;
            let mut at = 0usize;
            let mut class_samples = [0u64; 2];
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
                                let samples = out_words[e * FACE_WORDS + 5];
                                e += 1;
                                z.states[i].queued -= samples;
                                report.wasted += u64::from(samples);
                            }
                            continue;
                        }
                    }
                }
                let class = usize::from(self.open[at].own_count);
                let s = &mut self.open[at].states[i];
                for _ in 0..r {
                    let words = &out_words[e * FACE_WORDS..(e + 1) * FACE_WORDS];
                    e += 1;
                    let samples = words[5];
                    s.queued -= samples;
                    // A readback that failed to map draws its rounds again in a later submit,
                    // from the face's first sample not drawn.
                    if s.done || !mapped {
                        report.wasted += u64::from(samples);
                        continue;
                    }
                    class_samples[class] += u64::from(samples);
                    report.rays += u64::from(words[15]);
                    report.ray_len += f64::from(f32::from_bits(words[16]));
                    report.samples += u64::from(samples);
                    report.sky_rays += u64::from(samples.min(s.sky_n.saturating_sub(words[4])));
                    report.key_rays += u64::from(samples.min(s.key_n.saturating_sub(words[4])));
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
                let all = class_samples[0] + class_samples[1];
                for ((s, m), &n) in out
                    .class_samples
                    .iter_mut()
                    .zip(&mut out.class_ms)
                    .zip(&class_samples)
                {
                    *s += n;
                    if all > 0 {
                        *m += ms * n as f64 / all as f64;
                    }
                }
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
                let stamp = self.stamps.remove(&coord.0).unwrap_or(self.stamp);
                sink(store, coord, first, block, stamp);
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

/// Where an ended job's block goes: the store, its chunk, its place, its words, and its job's
/// stamp ([`JobLoop::stamps`]).
pub(crate) type BlockSink<'a> = dyn FnMut(&mut Store, ChunkCoord, u64, Vec<u32>, u64) + 'a;

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

/// A chunk's job while its faces run.
pub(super) struct OpenJob {
    pub(super) coord: ChunkCoord,
    pub(super) faces: ChunkFaces,
    pub(super) first_word: u64,
    pub(super) kept_bricks: KeptBricks,
    /// [`Job::key_only`].
    pub(super) key_only: bool,
    /// Whether the job draws its own count ([`Job::brick_samples`], the coarse pass): its
    /// samples' class in [`SubmitDone::class_samples`].
    pub(super) own_count: bool,
    /// [`Job::tier`].
    pub(super) tier: u8,
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
    /// The job at `coord` in `slot`: its faces' states. The caller hands `(slot, coord)` to the
    /// render half, which writes its candidate table ([`JobLoop::new_tables`]).
    pub(super) fn open(&self, job: Job, slot: usize) -> OpenJob {
        let Job {
            coord,
            faces,
            first_word,
            kept_bricks,
            samples: _,
            brick_samples,
            key_only,
            tier,
        } = job;
        // A job's own count (the live queue's coarse pass) on cells of a brick draws each
        // direction's samples on `ceil(count / BAKE_LANES)` of its faces, spread evenly through
        // them in key order, so each entry fills its workgroup; the record is their mean.
        let own_count = brick_samples.is_some();
        let subset = brick_samples.is_some() && self.settings.cell == 8;
        let brick_samples = brick_samples.unwrap_or(self.settings.brick_samples);
        let mut states: Vec<FaceState> = Vec::with_capacity(faces.faces());
        for (brick_at, keys) in &faces.bricks {
            let b = brick_of_slot(usize::from(*brick_at));
            // A key-only job keeps every record's other words and draws every face's key rays.
            let record_kept = !key_only && kept_bricks.contains_key(brick_at);
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
                    && (!key_only || self.run.key.is_some())
                    && self.settings.filter.as_ref().is_none_or(|f| f(voxel, face));
                let key_n = if self.run.key.is_some() {
                    KEY_BAKE_RAYS.div_ceil(drawn[d].max(1)).min(n)
                } else {
                    0
                };
                // A key-only face draws its key rays and stops.
                let n = if key_only { key_n } else { n };
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
                    sky_n: if self.run.sky & BAKE_SKY_ON != 0 && !key_only {
                        let rays = if tier == TIER_FAR {
                            SKY_FAR_RAYS
                        } else {
                            SKY_BAKE_RAYS
                        };
                        rays.div_ceil(drawn[d].max(1)).min(n)
                    } else {
                        0
                    },
                    sky_open: 0,
                    key_n,
                    key_open: 0,
                    key_only,
                });
            }
        }
        let live = (0..states.len()).filter(|&i| !states[i].done).collect();
        OpenJob {
            coord,
            faces,
            first_word,
            kept_bricks,
            key_only,
            own_count,
            tier,
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
            let baked = || {
                brick_record(
                    states,
                    clamp,
                    cell,
                    self.run.sky & BAKE_SKY_ON != 0,
                    self.run.key.is_some(),
                    job.tier,
                )
            };
            let record = match job.kept_bricks.get(slot) {
                Some(kept) if job.key_only && self.run.key.is_some() => key_rebaked(kept, &baked()),
                Some(kept) => kept.clone(),
                None => baked(),
            };
            block.extend(record);
        }
        debug_assert_eq!(block.len() as u64, words);
        // The sun its key words were baked under: a ring job's kept bricks may be older.
        block[KEY_SUN_AT as usize] = self.run.key.map_or(0, |(d, _)| pack_direction(d));
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
