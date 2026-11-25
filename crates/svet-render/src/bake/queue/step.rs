//! The queue's step: one frame's run over every stage.

use super::rule::zero_block;
use super::*;
use crate::bake::job_loop::JobCtx;

impl BakeQueue {
    /// One frame's step: follow the window, free the blocks of chunks the mirror dropped, open
    /// jobs while the open ones hold less than two slices, one submit within the slice, and the
    /// fades a frame on. `None` from the submit when nothing was dispatched. `t0` is when the
    /// frame's call began: the time to here is the step's setup stage ([`StepStages`]).
    ///
    /// The bake half's writes go into write lists ([`ListMaker`]), which the render half
    /// (`render`, ADR-0083) applies at the step's end, in lockstep the same frame.
    ///
    /// The step reads nothing of the frame but `msg` (ADR-0083): its arrivals and settings go
    /// into the books first, then its mirror notes into the bake half's `shadow`.
    pub(crate) fn step(
        &mut self,
        shadow: &mut Shadow,
        world: &World,
        store: &mut Store,
        msg: FrameMessage,
        half: &mut dyn RenderHalf,
    ) -> Result<QueueStats, BakeError> {
        let FrameMessage {
            start: t0,
            view,
            notes,
            sky,
            key,
            settings,
            arrivals,
        } = msg;
        self.take_message(settings, arrivals, world.materials());
        shadow.take(&notes);
        self.changes_in = notes.changed;
        let run = &BakeRun {
            mirror: shadow,
            world,
            sky,
            key,
        };
        let view = &view;
        self.laps = Laps::new(t0);
        // Summary time spent outside a step (a one-shot bake, a dig's ring) is not this step's.
        store.take_summary_ms();
        store.out = Some(ListMaker::new(half.cap(), self.steps + 1));
        let books = self.step_books(run, half, store, view, t0);
        let lists = store.out.take().map(|mut m| m.finish()).unwrap_or_default();
        self.apply_lists(half, store, lists, t0);
        books.map(|()| self.stats)
    }

    /// The render half's apply (lockstep): the step's lists into the pool, a first block's
    /// landing word the frame's, and the store's copy told what was written.
    fn apply_lists(
        &mut self,
        half: &mut dyn RenderHalf,
        store: &mut Store,
        lists: Vec<WriteList>,
        t0: StepStart,
    ) {
        // The landing word is the frame plus one ([`BakeQueue::landed_word`]); with the switch
        // off or in the pre-roll no write carries a mark.
        let frame = self
            .landed_word
            .filter(|w| *w != 0)
            .map_or(0, |w| u64::from(w) - 1);
        for (c, w) in half.apply(lists, frame) {
            store.landed(c, w);
        }
        self.laps.lap(S_APPLY);
        self.stats.stages.ms = self.laps.ms;
        self.stats.stages.render_ms = self.laps.render_ms();
        self.stats.stages.bytes = self.laps.bytes();
        self.stats.stages.total_ms = stopwatch().duration_since(t0.at).as_secs_f64() * 1e3;
    }

    /// The bake half of [`BakeQueue::step`]: every stage but the apply.
    fn step_books(
        &mut self,
        run: &BakeRun<'_>,
        half: &mut dyn RenderHalf,
        store: &mut Store,
        view: &QueueView,
        t0: StepStart,
    ) -> Result<(), BakeError> {
        self.laps.lap(S_SETUP);
        let (changes, unheld) = self.follow_window(run, store, view);
        let changed = changes.as_ref().is_none_or(|c| !c.is_empty());
        // The column tops are read again this step, after the mirror's sync: the memo of the
        // last step's opening would answer the rule from before an edit (a dig to the sky).
        self.columns_lowest.clear();
        self.relist_zeroed(run.mirror, store, changes.as_deref());
        let world = run.world.store();
        self.sealed.prune(run.mirror.mirrored_chunks(), |c| {
            world.contains(ChunkCoord(c))
        });
        self.forget_unheld(run.mirror, changes, &unheld);
        self.catch_up(run, view, changed);
        self.laps.lap(S_TOUCHED);
        // An edit's writes are one list, never carried (ADR-0083).
        if let Some(m) = store.out.as_mut() {
            m.editing = true;
        }
        let boxes = self.take_boxes(run, store);
        if let Some(m) = store.out.as_mut() {
            m.editing = false;
        }
        boxes?;
        self.laps.lap(S_BOXES);
        let (cap, opened_step) = self.open_step(run, store, view)?;
        half.refresh_tables(&self.lp.open_tables());
        half.write_params();
        self.laps.lap(S_REFRESH);
        self.submit(run, half, store, view, cap);
        self.close_step(run, store, view, t0, opened_step);
        Ok(())
    }

    /// The step's window stages: the eye's cell, the store's window, and the blocks of the chunks
    /// the mirror does not hold full. Returns the mirror's changes since the last step, each
    /// chunk once (`None` untracked, the first step), and those of them the mirror does not
    /// hold full, read in the same walk (the forget stage's keys).
    fn follow_window(
        &mut self,
        run: &BakeRun<'_>,
        store: &mut Store,
        view: &QueueView,
    ) -> (Option<Vec<ChunkCoord>>, Vec<[i32; 3]>) {
        self.steps += 1;
        // In lockstep the message is this frame's, its number the step's.
        self.lp.stamp = self.steps;
        let eye = view.eye.map(|v| (v / 64.0).floor() as i32);
        if eye != self.eye {
            self.eye_moves += 1;
        }
        self.eye = eye;
        self.retier(store, view, eye);
        let gpu = run.mirror;
        self.stats.scanned = 0;
        self.stats.reach_checks = 0;
        self.stats.rebuilt = 0;
        let window = gpu.window();
        let dropped = store.rewindow(window);
        self.rewindowed(dropped);
        self.laps.lap(S_REWINDOW);
        let seen = (Some(window), gpu.mirrored_chunks());
        let moved = seen != self.mirror_seen;
        self.mirror_seen = seen;
        store.piece = Some(window);
        // The mirror's changes since the last step; untracked (the first step), everything is
        // read again.
        // The list repeats a chunk once per event (queued, uploaded, dropped); everything below
        // reads each changed chunk once.
        // The frame's message carries the mirror's change list (`BakeQueue::changes_in`).
        let changes = self.changes_in.take().map(|mut ch| {
            self.stats.scanned += ch.len() as u64;
            let mut seen = ChunkSet::with_capacity_and_hasher(ch.len(), Default::default());
            ch.retain(|c| seen.insert(c.0));
            self.stats.mirror_changes += ch.len() as u64;
            ch
        });
        let unheld = |c: &[i32; 3]| !gpu.is_mirrored(ChunkCoord(*c)) || gpu.is_lod(ChunkCoord(*c));
        // A chunk the mirror does not hold full (evicted, or LOD-only) frees its block. Every
        // such chunk is among the mirror's changes since the step that last read them.
        // In coordinate order, so the freed places return to the free list in the same order
        // run to run.
        // One walk of the changes reads which the mirror does not hold full, for this stage and
        // the forget stage's.
        let unheld_changes: Vec<[i32; 3]> = changes
            .iter()
            .flatten()
            .map(|c| c.0)
            .filter(unheld)
            .collect();
        let gone: Vec<[i32; 3]> = match &changes {
            Some(_) => {
                let mut g: Vec<[i32; 3]> = unheld_changes
                    .iter()
                    .copied()
                    .filter(|c| store.blocks.contains_key(c))
                    .collect();
                g.sort_unstable();
                g
            }
            None => {
                self.stats.scanned += store.blocks.len() as u64;
                store.blocks.keys().copied().filter(unheld).collect()
            }
        };
        if moved || !gone.is_empty() {
            // The mirror moved, gained or lost chunks: the depth is counted again this step.
            self.counted_idle = false;
            self.quiet_until = self.steps;
        }
        for c in gone {
            store.remove(ChunkCoord(c));
            self.zeroed.remove(&c);
            self.block_count.remove(&c);
        }
        self.laps.lap(S_GONE);
        (changes, unheld_changes)
    }

    /// The step's forget, reach-drop and touched stages over the mirror's changes.
    fn forget_unheld(
        &mut self,
        gpu: &Shadow,
        changes: Option<Vec<ChunkCoord>>,
        unheld: &[[i32; 3]],
    ) {
        self.columns_final.clear();
        // What the queue knows of a chunk the mirror does not hold full goes with it, block or
        // none (a faceless or blocked chunk has none): the chunk is listed afresh if it returns.
        // A job in flight for it ends in the sink, which drops its block.
        let held = |c: &[i32; 3]| gpu.is_mirrored(ChunkCoord(*c)) && !gpu.is_lod(ChunkCoord(*c));
        // The sets hold only held chunks, and only a changed chunk can have stopped being held,
        // so removing the changed unheld chunks and keeping the held entries leave the same sets.
        // A key costs two lookups and six removals, an entry two lookups: the cheaper goes. In a
        // flight the changes a step outnumber the sets' entries.
        let entries = self.fades.len()
            + self.faceless.len()
            + self.blocked.len()
            + self.digs.len()
            + self.reach.len()
            + self.listed.len()
            + self.partial.len()
            + self.partial_due.len()
            + self.provisional.len()
            + self.provisional_due.len();
        let by_keys = changes.as_ref().is_some_and(|ch| 4 * ch.len() < entries);
        if !by_keys {
            self.fades.retain(|c, _| held(c));
            self.faceless.retain(held);
            self.blocked.retain(|c, _| held(c));
            self.digs.retain(|c, _| held(c));
            self.reach.retain(held);
            self.listed.retain(|c, _| held(c));
            self.partial.retain(held);
            self.partial_due.retain(held);
            self.provisional.retain(held);
            self.provisional_due.retain(held);
            self.keys.retain(held);
        }
        match changes {
            Some(ch) => {
                if by_keys {
                    for &c in unheld {
                        self.fades.remove(&c);
                        self.faceless.remove(&c);
                        self.blocked.remove(&c);
                        self.digs.remove(&c);
                        self.reach.remove(&c);
                        self.listed.remove(&c);
                        // Empty unless a partial reach baked (`ReachMode::Partial`).
                        if !self.partial.is_empty() || !self.partial_due.is_empty() {
                            self.partial.remove(&c);
                            self.partial_due.remove(&c);
                        }
                        if !self.provisional.is_empty() {
                            self.provisional.remove(&c);
                            self.provisional_due.remove(&c);
                        }
                        self.keys.retain(|k| *k != c);
                    }
                }
                self.laps.lap(S_FORGET);
                self.drop_reach(&ch, gpu.light_radius());
                self.laps.lap(S_DROP_REACH);
                self.touched.extend(ch.iter().map(|c| c.0));
                self.requeue_partial(gpu, Some(ch.as_slice()));
            }
            None => {
                self.laps.lap(S_FORGET);
                self.reach_cache.clear();
                self.laps.lap(S_DROP_REACH);
                self.pool_stale = true;
                self.requeue_partial(gpu, None);
            }
        }
    }

    /// The step's withdrawal, digs and listing stages: the submit's sample cap, and whether jobs
    /// were opened this step.
    fn open_step(
        &mut self,
        run: &BakeRun<'_>,
        store: &mut Store,
        view: &QueueView,
    ) -> Result<(u64, bool), BakeError> {
        let dig = !self.digs.is_empty() || !self.dig_open.is_empty();
        self.refresh_waiting(store, view, false);
        let cap = view.samples.unwrap_or_else(|| {
            let slice = self.step_slice(view.interval_s);
            self.stats.slice_ms = slice;
            let ms = slice + if dig { self.edit_ms } else { 0.0 };
            ((ms * 1e6 / self.stats.ns_per_sample.max(0.1)) as u64).clamp(16_384, SUBMIT_SAMPLES)
        });
        self.withdraw_passed(run, store, view);
        self.laps.lap(S_WITHDRAW);
        // The key-only jobs push at the front before the digs, which go ahead of them.
        self.mark_keys(run, store);
        self.open_keys(run, store, view, cap)?;
        self.open_digs(run, store, cap)?;
        self.laps.lap(S_DIGS);
        let mut opened_step = false;
        if self.lp.samples_left() < 2 * cap && self.steps >= self.quiet_until {
            opened_step = true;
            self.stats.openings += 1;
            let opened = self.lp.chunks().count();
            self.open(run, store, view, cap)?;
            if self.lp.chunks().count() == opened && self.lp.idle() {
                // Nothing was ready: the candidates are sorted again a few frames on.
                self.quiet_until = self.steps + QUEUE_QUIET;
            }
            self.laps.lap(S_LIST);
        }
        self.stats.visited = self.ordered.take_visited();
        self.stats.visited_total += self.stats.visited;
        Ok((cap, opened_step))
    }

    /// The step's poll, collect, place and submit stages, and the stats they read: the submits
    /// read back go to the sink, and one new submit goes out within `cap` samples.
    fn submit(
        &mut self,
        run: &BakeRun<'_>,
        half: &mut dyn RenderHalf,
        store: &mut Store,
        view: &QueueView,
        cap: u64,
    ) {
        // Nothing waits on the GPU in a sliced step: the submits read back are those whose
        // results are mapped by now, and a new one goes out while fewer than IN_FLIGHT are
        // unread. A step without a slice (the pre-roll) waits for its oldest submit once
        // IN_FLIGHT are out, so the CPU lists and plans while the GPU draws.
        let unread = if view.samples.is_some() {
            IN_FLIGHT - 1
        } else {
            IN_FLIGHT
        };
        half.poll();
        self.laps.lap(S_POLL);
        // The sink's own time, the blocks' placement, taken out of the collect's and the
        // submit's laps.
        let place_ms = std::cell::Cell::new(0.0f64);
        // Of it, the `wgpu` calls' (the store's writes).
        let place_render = std::cell::Cell::new(0.0f64);
        let ruled = std::mem::take(&mut self.ruled);
        // The ring-1 dig jobs ([`BakeQueue::open_digs`]): opened into the slots the read back
        // frees, and planned ahead of every other job.
        let ring1: ChunkSet = self
            .dig_open
            .iter()
            .filter(|(_, d)| d.near)
            .map(|(c, _)| *c)
            .collect();
        let (cell, clamp) = (self.settings.cell, self.settings.clamp);
        let mut placement = Placement {
            gpu: run.mirror,
            stale: &mut self.stale,
            fades: &mut self.fades,
            stats: &mut self.stats,
            job_coarse: &mut self.job_coarse,
            job_count: &mut self.job_count,
            block_count: &mut self.block_count,
            job_eye: &mut self.job_eye,
            dig_open: &mut self.dig_open,
            keys_open: &mut self.keys.open,
            reach: &mut self.reach,
            partial: &mut self.partial,
            job_partial: &mut self.job_partial,
            provisional: &mut self.provisional,
            job_provisional: &mut self.job_provisional,
            touched: &mut self.touched,
            entered: &mut self.entered,
            ages_first: &mut self.ages_first,
            ages_full: &mut self.ages_full,
            steps: self.steps,
            record: record_words(self.settings.cell),
            landing_fade: self.landing_fade,
            dig_store_hold: self.dig_store_hold,
            landed_word: self.landed_word,
        };
        let mut sink =
            |store: &mut Store, coord: ChunkCoord, first: u64, block: Vec<u32>, stamp| {
                let (t, c) = (stopwatch(), calls().ms);
                placement.place(store, coord, first, block, stamp);
                place_ms.set(place_ms.get() + seconds_since(t) * 1e3);
                place_render.set(place_render.get() + calls().ms - c);
            };
        // The rule's zero records land through the same placement as a job's block.
        let sky = run.sky & BAKE_SKY_ON != 0;
        let key = run.key.map(|(d, _)| d);
        let now = self.lp.stamp;
        for (coord, first, faces, tier) in ruled {
            sink(
                store,
                coord,
                first,
                zero_block(coord, &faces, cell, clamp, sky, key, tier),
                now,
            );
        }
        let settings = BakeSettings {
            round: if view.samples.is_some() {
                UNSLICED_ROUND
            } else {
                self.settings.round
            },
            ..self.settings.clone()
        };
        let mut d = self.lp.collect(
            run,
            half,
            store,
            &settings,
            &mut sink,
            &mut |_| {},
            IN_FLIGHT,
        );
        // Each lap's placement moves from its stage to the placement's, its `wgpu` calls with
        // it.
        let (mut placed, mut placed_render) = (0.0, 0.0);
        let mut lap = |laps: &mut Laps, stage: usize| {
            laps.lap(stage);
            let p = place_ms.get() - placed;
            let r = place_render.get() - placed_render;
            placed = place_ms.get();
            placed_render = place_render.get();
            // The placement ran inside the lap; the clamp only absorbs rounding.
            laps.shift(stage, S_PLACE, p, r);
        };
        lap(&mut self.laps, S_COLLECT);
        if !ring1.is_empty() {
            let ctx = JobCtx {
                run,
                settings: &settings,
                round: settings.round.max(1),
                rounds_cap: 0,
            };
            while let Some(k) = self
                .lp
                .pending
                .iter()
                .position(|j| ring1.contains(&j.coord.0))
            {
                let Some(slot) = self.lp.free.pop() else {
                    break;
                };
                let job = self.lp.pending.remove(k).expect("found above");
                self.lp.new_tables.push((slot, job.coord));
                self.lp.open.push(ctx.open(job, slot));
            }
            self.lp.promote(|c| ring1.contains(&c.0));
            // A ring-1 job opened with no live face (every brick it re-bakes left faceless by
            // the edit) has nothing to submit, so no read back would close it: it ends here,
            // its block to the sink, as a pending job with nothing to draw ends at its opening.
            self.lp.close_ended(run, store, &settings, &mut sink);
        }
        let room = self.lp.in_flight() < IN_FLIGHT;
        let in_flight_full = !room && !self.lp.idle();
        let sent = room
            && self
                .lp
                .dispatch(run, half, store, &settings, cap, &mut sink, &mut |_| {});
        lap(&mut self.laps, S_SUBMIT);
        let submitted = if sent { self.lp.last_samples } else { 0 };
        if unread < IN_FLIGHT {
            // With nothing left to send, the pre-roll waits out what is in flight.
            let left = if sent { unread } else { 0 };
            let w = self
                .lp
                .collect(run, half, store, &settings, &mut sink, &mut |_| {}, left);
            d.samples += w.samples;
            d.seconds += w.seconds;
            if let Some(ms) = w.gpu_ms {
                *d.gpu_ms.get_or_insert(0.0) += ms;
            }
            d.class_samples = [0, 1].map(|k| d.class_samples[k] + w.class_samples[k]);
            d.class_ms = [0, 1].map(|k| d.class_ms[k] + w.class_ms[k]);
            lap(&mut self.laps, S_COLLECT);
        }
        // The slice is sized by the GPU's own time for the submits read back, a frame or two
        // late; without timestamps it stays at the first guess, a fixed count.
        self.stats.cap = cap;
        self.stats.submitted = submitted;
        self.stats.plan_stops = self.lp.plan_stops;
        self.stats.in_flight_full += u64::from(in_flight_full);
        self.stats.samples = d.samples;
        self.stats.ms = d.gpu_ms.unwrap_or(d.seconds * 1e3);
        self.stats.total_samples += d.samples;
        let st = &mut self.stats;
        st.class_samples = [0, 1].map(|k| st.class_samples[k] + d.class_samples[k]);
        st.class_ms = [0, 1].map(|k| st.class_ms[k] + d.class_ms[k]);
        if let Some(ms) = d.gpu_ms
            && d.samples >= 4096
        {
            let ns = ms * 1e6 / d.samples as f64;
            self.stats.ns_per_sample = 0.8 * self.stats.ns_per_sample + 0.2 * ns;
        }
    }

    /// The step's fades and summaries stages, and its stats: the ended jobs' samples by where
    /// their chunk lies, the counts, and the stage times.
    fn close_step(
        &mut self,
        run: &BakeRun<'_>,
        store: &mut Store,
        view: &QueueView,
        t0: StepStart,
        opened_step: bool,
    ) {
        let record = record_words(self.settings.cell);
        // The ended jobs' samples by where their chunk lies now.
        let ended = self
            .lp
            .ended
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default();
        for (c, drawn) in ended {
            let class = self.view_class(c.0, self.eye, view);
            self.stats.view_samples[class] += drawn;
            let centre = [0, 1, 2].map(|k| (c.0[k] * 64 + 32) as f32 - view.eye[k]);
            let cos: f32 = (0..3).map(|k| centre[k] * view.forward[k]).sum();
            if class == VIEW_OUT && cos < 0.0 {
                self.stats.passed_samples += drawn;
            }
        }
        self.advance_fades(store, record);
        self.land_waiting(run.mirror, store);
        if self.stats.rule_missed.landed_at.is_none() {
            if !self.view_waiting.is_empty() {
                self.waiting_armed = true;
            } else if self.waiting_armed {
                self.stats.rule_missed.landed_at = Some(self.steps);
            }
        }
        self.stats.jobs = self.lp.chunks().count() as u64;
        self.stats.fading = self.fades.len() as u64;
        self.stats.coarse = store.coarse.len() as u64;
        self.stats.digs = (self.digs.len() + self.dig_open.len()) as u64;
        self.stats.reach = self.reach.len() as u64;
        self.stats.partial = self.partial.len() as u64;
        self.stats.provisional = self.provisional.len() as u64;
        self.stats.keys = self.keys.count();
        self.laps.lap(S_FADES);
        // The summaries' and parents' time ran inside the placement, the fades, the evictions
        // and the window's move: it moves to its own stage, from those in that order.
        let (mut summaries, mut summaries_render) = store.take_summary_ms();
        for stage in [S_PLACE, S_FADES, S_GONE, S_REWINDOW] {
            let (m, r) = self
                .laps
                .shift(stage, S_SUMMARIES, summaries, summaries_render);
            summaries -= m;
            summaries_render -= r;
        }
        self.stats.stages = StepStages {
            ms: self.laps.ms,
            render_ms: self.laps.render_ms(),
            bytes: self.laps.bytes(),
            total_ms: stopwatch().duration_since(t0.at).as_secs_f64() * 1e3,
            opened: opened_step,
        };
    }
}

/// What the step's sink writes of the queue while the job loop is borrowed: a read-back block's
/// placement in the store, its fade, and the queue's books on its chunk.
struct Placement<'a> {
    gpu: &'a Shadow,
    /// [`BakeQueue::stale`].
    stale: &'a mut std::collections::HashMap<[i32; 3], u64>,
    fades: &'a mut BTreeMap<[i32; 3], Fade>,
    stats: &'a mut QueueStats,
    job_coarse: &'a mut std::collections::HashMap<[i32; 3], bool>,
    job_count: &'a mut std::collections::HashMap<[i32; 3], u32>,
    block_count: &'a mut std::collections::HashMap<[i32; 3], u32>,
    job_eye: &'a mut std::collections::HashMap<[i32; 3], u64>,
    dig_open: &'a mut std::collections::HashMap<[i32; 3], DigChunk>,
    keys_open: &'a mut std::collections::HashSet<[i32; 3]>,
    reach: &'a mut std::collections::BTreeSet<[i32; 3]>,
    partial: &'a mut ChunkSet,
    job_partial: &'a mut ChunkSet,
    provisional: &'a mut ChunkSet,
    job_provisional: &'a mut ChunkSet,
    /// [`BakeQueue::touched`]: a chunk whose job's block is thrown away is a candidate again.
    touched: &'a mut ChunkSet,
    entered: &'a mut std::collections::HashMap<[i32; 3], (u64, bool)>,
    ages_first: &'a mut Vec<u32>,
    ages_full: &'a mut Vec<u32>,
    steps: u64,
    record: usize,
    /// [`BakeQueue::landing_fade`].
    landing_fade: bool,
    /// [`BakeQueue::dig_store_hold`]: a dig's landing fades its sky visibilities too, from the
    /// held record.
    dig_store_hold: bool,
    /// [`BakeQueue::landed_word`].
    landed_word: Option<u32>,
}

impl Placement<'_> {
    /// Whether a first block's landing word is the apply frame's: the switch on and not the
    /// pre-roll (whose word is 0).
    fn marks_landing(&self) -> bool {
        self.landed_word.is_some_and(|w| w != 0)
    }

    /// Places chunk `coord`'s block read back at `first`, or releases it when the chunk went
    /// stale after its job's `stamp` (a change in a later frame message) or left the mirror.
    fn place(
        &mut self,
        store: &mut Store,
        coord: ChunkCoord,
        first: u64,
        block: Vec<u32>,
        stamp: u64,
    ) {
        let coarse = self.job_coarse.remove(&coord.0).unwrap_or(false);
        let count = self.job_count.remove(&coord.0);
        self.job_eye.remove(&coord.0);
        let from_partial = self.job_partial.remove(&coord.0);
        let from_provisional = self.job_provisional.remove(&coord.0);
        let dug = self.dig_open.remove(&coord.0);
        let keyed = self.keys_open.remove(&coord.0);
        let stale = self.stale.remove(&coord.0).is_some_and(|at| at > stamp);
        if stale || !self.gpu.is_mirrored(coord) {
            store.release(first, block.len() as u64);
            // The pool reads its candidacy again: a job opened and ended between two of its
            // reads is in neither job set it compares, and a chunk with no block is a
            // candidate once its job is gone.
            self.touched.insert(coord.0);
            // A dig job's bricks went back to the waiting digs at the edit; a whole chunk's
            // job that would have replaced a full block re-bakes whole.
            if dug.is_none()
                && !keyed
                && self.gpu.is_mirrored(coord)
                && store.blocks.contains_key(&coord.0)
                && !store.is_coarse(coord)
            {
                self.reach.insert(coord.0);
            }
            return;
        }
        let mut block = block;
        if keyed {
            // A key-only job keeps the block's words but its key visibilities: its place,
            // reach and provisional flags stay; the light pass fades the key words' change.
            self.stats.keys_baked += 1;
            if let Some((f, from)) = store
                .blocks
                .get(&coord.0)
                .and_then(|&(f, _)| store.words.get(&coord.0).map(|w| (f, w.clone())))
                .filter(|(_, w)| {
                    w.len() == block.len() && w[..CHUNK_BRICKS] == block[..CHUNK_BRICKS]
                })
            {
                block[LANDED_AT as usize] = landed_word(Some(&from), self.landed_word);
                self.refade(store, coord, (first, f), from, block, false);
            } else {
                store.release(first, block.len() as u64);
            }
            return;
        }
        if let Some(e) = self.entered.get_mut(&coord.0) {
            let age = u32::try_from(self.steps - e.0).unwrap_or(u32::MAX);
            if !e.1 {
                e.1 = true;
                self.ages_first.push(age);
            }
            if !coarse {
                self.ages_full.push(age);
                self.entered.remove(&coord.0);
            }
        }
        if let Some(d) = &dug {
            self.stats.digs_baked += 1;
            self.stats.dig_bricks += d.slots.len() as u64;
        }
        // A whole chunk's block carries its job's reach; a dig's keeps the rest of the
        // block's, and with it the flag.
        if dug.is_none() {
            if from_partial {
                self.partial.insert(coord.0);
            } else {
                self.partial.remove(&coord.0);
            }
            if from_provisional {
                self.provisional.insert(coord.0);
            } else {
                self.provisional.remove(&coord.0);
            }
        }
        let dug = dug.is_some();
        self.stats.baked += 1;
        // The table word's flag follows at the put below.
        store.mark_coarse(coord, coarse);
        if let Some(n) = count {
            self.block_count.insert(coord.0, n);
        }
        let old = store
            .blocks
            .get(&coord.0)
            .and_then(|&(f, _)| store.words.get(&coord.0).map(|w| (f, w.clone())));
        // The chunk's landing word: kept from its block, else this frame's (a first block).
        block[LANDED_AT as usize] =
            landed_word(old.as_ref().map(|(_, w)| w.as_slice()), self.landed_word);
        match old {
            Some((f, from))
                if from.len() == block.len() && from[..CHUNK_BRICKS] == block[..CHUNK_BRICKS] =>
            {
                self.refade(store, coord, (first, f), from, block, dug);
            }
            Some((f, from)) => {
                self.stats.landings[LANDED_UNFADED] += 1;
                store.release(f, from.len() as u64);
                self.fades.remove(&coord.0);
                store.put(coord, first, block);
            }
            None => match store.light.chain(coord.0, store.missing) {
                // A first block under a fallback mode fades from the chain's value, read
                // before the block's own summary joins the chain (ADR-0053).
                Some(chain) => {
                    self.stats.landings[LANDED_FALLBACK] += 1;
                    let mut from = fallback_block(&block, &chain, self.record, self.landing_fade);
                    count_landing(&mut self.stats.landing_cells, &from, &block, self.record);
                    // The block and what it shows land in one frame, their landing word that
                    // frame's.
                    store.begin_unit();
                    store.mark_landing = self.marks_landing();
                    store.put(coord, first, block);
                    store.show(coord, first, &mut from);
                    store.mark_landing = false;
                    store.end_unit();
                    self.fades.insert(
                        coord.0,
                        Fade {
                            first,
                            from,
                            start: self.steps,
                            frame: 0,
                            frames: FADE,
                            vis: self.landing_fade,
                            dig: false,
                        },
                    );
                }
                None => {
                    self.stats.landings[LANDED_BARE] += 1;
                    store.mark_landing = self.marks_landing();
                    store.put(coord, first, block);
                    store.mark_landing = false;
                }
            },
        }
    }

    /// The same layout: the block takes its old place `f`, its words read back at `first` are
    /// released, it shows its old values `from` and fades to the new ones.
    fn refade(
        &mut self,
        store: &mut Store,
        coord: ChunkCoord,
        (first, f): (u64, u64),
        from: Vec<u32>,
        block: Vec<u32>,
        dug: bool,
    ) {
        // Under a running fade the old values are what that fade shows now, not its target: a
        // refinement landing while a first block fades from its fallback goes on from where the
        // pool is.
        let mut from = match self.fades.get(&coord.0) {
            Some(fd)
                if fd.first == f
                    && fd.frame < fd.frames
                    && fd.from.len() == from.len()
                    && fd.from[..CHUNK_BRICKS] == from[..CHUNK_BRICKS] =>
            {
                let mut now = Vec::new();
                let t = fd.frame as f32 / fd.frames as f32;
                fade_mix(&mut now, &fd.from, &from, t, self.record, fd.vis);
                now
            }
            _ => from,
        };
        self.stats.landings[LANDED_REFADED] += 1;
        count_landing(&mut self.stats.landing_cells, &from, &block, self.record);
        store.release(first, block.len() as u64);
        store.begin_unit();
        store.put(coord, f, block);
        store.show(coord, f, &mut from);
        store.end_unit();
        self.fades.insert(
            coord.0,
            Fade {
                first: f,
                from,
                start: self.steps,
                frame: 0,
                frames: if dug { DIG_FADE } else { FADE },
                vis: self.landing_fade || (dug && self.dig_store_hold),
                dig: dug,
            },
        );
    }
}

/// The landing word ([`LANDED_AT`]) of a block landing on a chunk whose block was `old`: under
/// the switch (`word` the queue's [`BakeQueue::landed_word`]) the old block's where the chunk
/// held one (a refinement, a key re-bake, a dig), else `word` (a first block over the
/// missing-light chain or a bare one: the frame plus one, 0 in the pre-roll); 0 with the switch
/// off.
pub(super) fn landed_word(old: Option<&[u32]>, word: Option<u32>) -> u32 {
    let Some(w) = word else {
        return 0;
    };
    old.map_or(w, |b| b.get(LANDED_AT as usize).copied().unwrap_or(0))
}

/// Counts into `cells` ([`QueueStats::landing_cells`]) the cells of a landing block `to` whose
/// words differ from `from`, what the pool shows as the fade starts, by read.
fn count_landing(cells: &mut [u64; 7], from: &[u32], to: &[u32], record: usize) {
    for &dir in &to[..CHUNK_BRICKS] {
        if dir == 0 {
            continue;
        }
        let run = (dir >> 11) as usize;
        for cell in (run - record..run).step_by(BRICK_WORDS) {
            let (a, b) = (
                &from[cell..cell + BRICK_WORDS],
                &to[cell..cell + BRICK_WORDS],
            );
            let (v, k) = (VIS_WORD, KEY_WORD);
            let flags = |w: u32| w & !0xFFFF;
            cells[LANDING_DIRECTIONS] += u64::from(a[..6] != b[..6]);
            if flags(a[v + 1]) != flags(b[v + 1]) {
                cells[LANDING_VIS_FLAG] += 1;
            } else if a[v..v + 2] != b[v..v + 2] {
                cells[LANDING_VIS] += 1;
            }
            let keyed = a[k + 1] & KEY_BAKED != 0 && b[k + 1] & KEY_BAKED != 0;
            if a[k..k + 2] != b[k..k + 2] {
                cells[if keyed { LANDING_KEY } else { LANDING_KEY_FLAG }] += 1;
            }
            cells[LANDING_TAIL] += u64::from(a[6] != b[6]);
            cells[LANDING_RECORDS] += 1;
        }
    }
}
