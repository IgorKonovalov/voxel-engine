//! The queue's step: one frame's run over every stage.

use super::*;

impl BakeQueue {
    /// One frame's step: follow the window, free the blocks of chunks the mirror dropped, open
    /// jobs while the open ones hold less than two slices, one submit within the slice, and the
    /// fades a frame on. `None` from the submit when nothing was dispatched. `t0` is when the
    /// frame's call began: the time to here is the step's setup stage ([`StepStages`]).
    pub(crate) fn step(
        &mut self,
        run: &BakeRun<'_>,
        store: &mut Store,
        view: &QueueView<'_>,
        t0: Instant,
    ) -> Result<QueueStats, BakeError> {
        self.laps = Laps::new(t0);
        // Summary time spent outside a step (a one-shot bake, a dig's ring) is not this step's.
        store.take_summary_ms();
        self.laps.lap(S_SETUP);
        self.steps += 1;
        let eye = view.eye.map(|v| (v / 64.0).floor() as i32);
        if eye != self.eye {
            self.eye_moves += 1;
        }
        self.eye = eye;
        let gpu = run.gpu;
        self.stats.scanned = 0;
        self.stats.reach_checks = 0;
        self.stats.rebuilt = 0;
        let window = gpu.window();
        let dropped = store.rewindow(run.queue, window);
        self.touched.extend(dropped);
        self.laps.lap(S_REWINDOW);
        let seen = (Some(window), gpu.mirrored_chunks());
        let moved = seen != self.mirror_seen;
        self.mirror_seen = seen;
        store.piece = Some(window);
        // The mirror's changes since the last step; untracked (the first step), everything is
        // read again.
        // The list repeats a chunk once per event (queued, uploaded, dropped); everything below
        // reads each changed chunk once.
        let changes = gpu.take_changes().map(|mut ch| {
            self.stats.scanned += ch.len() as u64;
            let mut seen = ChunkSet::with_capacity_and_hasher(ch.len(), Default::default());
            ch.retain(|c| seen.insert(c.0));
            ch
        });
        let unheld = |c: &[i32; 3]| !gpu.is_mirrored(ChunkCoord(*c)) || gpu.is_lod(ChunkCoord(*c));
        // A chunk the mirror does not hold full (evicted, or LOD-only) frees its block. Every
        // such chunk is among the mirror's changes since the step that last read them.
        // In coordinate order, so the freed places return to the free list in the same order
        // run to run.
        let gone: Vec<[i32; 3]> = match &changes {
            Some(ch) => {
                let mut g: Vec<[i32; 3]> = ch
                    .iter()
                    .map(|c| c.0)
                    .filter(|c| store.blocks.contains_key(c) && unheld(c))
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
            store.remove(run.queue, ChunkCoord(c));
        }
        self.laps.lap(S_GONE);
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
            + self.partial_due.len();
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
        }
        match changes {
            Some(ch) => {
                if by_keys {
                    for c in ch.iter().map(|c| c.0).filter(|c| !held(c)) {
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
        self.laps.lap(S_TOUCHED);
        self.take_boxes(run, store)?;
        self.laps.lap(S_BOXES);
        let dig = !self.digs.is_empty() || !self.dig_open.is_empty();
        let cap = view.samples.unwrap_or_else(|| {
            let slice = self.budget.slice_ms(view.interval_s);
            self.stats.slice_ms = slice;
            let ms = slice + if dig { self.edit_ms } else { 0.0 };
            ((ms * 1e6 / self.stats.ns_per_sample.max(0.1)) as u64).clamp(16_384, SUBMIT_SAMPLES)
        });
        self.withdraw_passed(run, store, view);
        self.laps.lap(S_WITHDRAW);
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
        self.lp.refresh_tables(run);
        let bind = run.pipeline.bind();
        run.queue
            .write_buffer(run.params_buffer, 0, bytemuck::bytes_of(&run.params));
        self.laps.lap(S_REFRESH);
        // Nothing waits on the GPU in a sliced step: the submits read back are those whose
        // results are mapped by now, and a new one goes out while fewer than IN_FLIGHT are
        // unread. A step without a slice (the pre-roll) waits for its oldest submit once
        // IN_FLIGHT are out, so the CPU lists and plans while the GPU draws.
        let unread = if view.samples.is_some() {
            IN_FLIGHT - 1
        } else {
            IN_FLIGHT
        };
        let _ = run.device.poll(wgpu::PollType::Poll);
        self.laps.lap(S_POLL);
        // The sink's own time, the blocks' placement, taken out of the collect's and the
        // submit's laps.
        let place_ms = std::cell::Cell::new(0.0f64);
        let queue = run.queue;
        let stale = &mut self.stale;
        let fades = &mut self.fades;
        let stats = &mut self.stats;
        let job_coarse = &mut self.job_coarse;
        let job_eye = &mut self.job_eye;
        let dig_open = &mut self.dig_open;
        let reach = &mut self.reach;
        let partial = &mut self.partial;
        let job_partial = &mut self.job_partial;
        let entered = &mut self.entered;
        let ages_first = &mut self.ages_first;
        let ages_full = &mut self.ages_full;
        let steps = self.steps;
        let record = record_words(self.settings.cell);
        let mut place = |store: &mut Store, coord: ChunkCoord, first: u64, block: Vec<u32>| {
            let coarse = job_coarse.remove(&coord.0).unwrap_or(false);
            job_eye.remove(&coord.0);
            let from_partial = job_partial.remove(&coord.0);
            let dug = dig_open.remove(&coord.0);
            if stale.remove(&coord.0) || !run.gpu.is_mirrored(coord) {
                store.release(first, block.len() as u64);
                // A dig job's bricks went back to the waiting digs at the edit; a whole chunk's
                // job that would have replaced a full block re-bakes whole.
                if dug.is_none()
                    && run.gpu.is_mirrored(coord)
                    && store.blocks.contains_key(&coord.0)
                    && !store.is_coarse(coord)
                {
                    reach.insert(coord.0);
                }
                return;
            }
            if let Some(e) = entered.get_mut(&coord.0) {
                let age = u32::try_from(steps - e.0).unwrap_or(u32::MAX);
                if !e.1 {
                    e.1 = true;
                    ages_first.push(age);
                }
                if !coarse {
                    ages_full.push(age);
                    entered.remove(&coord.0);
                }
            }
            if let Some(d) = &dug {
                stats.digs_baked += 1;
                stats.dig_bricks += d.slots.len() as u64;
            }
            // A whole chunk's block carries its job's reach; a dig's keeps the rest of the
            // block's, and with it the flag.
            if dug.is_none() {
                if from_partial {
                    partial.insert(coord.0);
                } else {
                    partial.remove(&coord.0);
                }
            }
            let dug = dug.is_some();
            stats.baked += 1;
            // The table word's flag follows at the put below.
            store.mark_coarse(coord, coarse);
            let old = store
                .blocks
                .get(&coord.0)
                .and_then(|&(f, _)| store.words.get(&coord.0).map(|w| (f, w.clone())));
            match old {
                Some((f, from))
                    if from.len() == block.len()
                        && from[..CHUNK_BRICKS] == block[..CHUNK_BRICKS] =>
                {
                    // The same layout: the block takes its old place, shows its old values and
                    // fades to the new ones. Under a running fade the old values are what that
                    // fade shows now, not its target: a refinement landing while a first block
                    // fades from its fallback goes on from where the pool is.
                    let mut from = match fades.get(&coord.0) {
                        Some(fd)
                            if fd.first == f
                                && fd.frame < fd.frames
                                && fd.from.len() == from.len()
                                && fd.from[..CHUNK_BRICKS] == from[..CHUNK_BRICKS] =>
                        {
                            let mut now = Vec::new();
                            let t = fd.frame as f32 / fd.frames as f32;
                            fade_mix(&mut now, &fd.from, &from, t, record);
                            now
                        }
                        _ => from,
                    };
                    store.release(first, block.len() as u64);
                    store.put(queue, coord, f, block);
                    store.show(queue, coord, f, &mut from);
                    fades.insert(
                        coord.0,
                        Fade {
                            first: f,
                            from,
                            frame: 0,
                            frames: if dug { DIG_FADE } else { FADE },
                        },
                    );
                }
                Some((f, from)) => {
                    store.release(f, from.len() as u64);
                    fades.remove(&coord.0);
                    store.put(queue, coord, first, block);
                }
                None => match store.light.chain(coord.0, store.missing) {
                    // A first block under a fallback mode fades from the chain's value, read
                    // before the block's own summary joins the chain (ADR-0053).
                    Some(chain) => {
                        let mut from = fallback_block(&block, &chain, record);
                        store.put(queue, coord, first, block);
                        store.show(queue, coord, first, &mut from);
                        fades.insert(
                            coord.0,
                            Fade {
                                first,
                                from,
                                frame: 0,
                                frames: FADE,
                            },
                        );
                    }
                    None => store.put(queue, coord, first, block),
                },
            }
        };
        let mut sink = |store: &mut Store, coord: ChunkCoord, first: u64, block: Vec<u32>| {
            let t = stopwatch();
            place(store, coord, first, block);
            place_ms.set(place_ms.get() + seconds_since(t) * 1e3);
        };
        let settings = BakeSettings {
            round: if view.samples.is_some() {
                UNSLICED_ROUND
            } else {
                self.settings.round
            },
            ..self.settings.clone()
        };
        let mut d = self
            .lp
            .collect(run, store, &settings, &mut sink, &mut |_| {}, IN_FLIGHT);
        // Each lap's placement moves from its stage to the placement's.
        let mut placed = 0.0;
        let mut lap = |laps: &mut Laps, stage: usize| {
            laps.lap(stage);
            let p = place_ms.get() - placed;
            placed = place_ms.get();
            // The placement ran inside the lap; the clamp only absorbs rounding.
            let p = p.min(laps.ms[stage]);
            laps.ms[stage] -= p;
            laps.ms[S_PLACE] += p;
        };
        lap(&mut self.laps, S_COLLECT);
        let sent = self.lp.in_flight() < IN_FLIGHT
            && self
                .lp
                .dispatch(run, bind, store, &settings, cap, &mut sink, &mut |_| {});
        lap(&mut self.laps, S_SUBMIT);
        let submitted = if sent { self.lp.last_samples } else { 0 };
        if unread < IN_FLIGHT {
            // With nothing left to send, the pre-roll waits out what is in flight.
            let left = if sent { unread } else { 0 };
            let w = self
                .lp
                .collect(run, store, &settings, &mut sink, &mut |_| {}, left);
            d.samples += w.samples;
            d.seconds += w.seconds;
            if let Some(ms) = w.gpu_ms {
                *d.gpu_ms.get_or_insert(0.0) += ms;
            }
            lap(&mut self.laps, S_COLLECT);
        }
        // The slice is sized by the GPU's own time for the submits read back, a frame or two
        // late; without timestamps it stays at the first guess, a fixed count.
        self.stats.cap = cap;
        self.stats.submitted = submitted;
        self.stats.samples = d.samples;
        self.stats.ms = d.gpu_ms.unwrap_or(d.seconds * 1e3);
        self.stats.total_samples += d.samples;
        if let Some(ms) = d.gpu_ms
            && d.samples >= 4096
        {
            let ns = ms * 1e6 / d.samples as f64;
            self.stats.ns_per_sample = 0.8 * self.stats.ns_per_sample + 0.2 * ns;
        }
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
        self.advance_fades(run.queue, store, record);
        self.stats.jobs = self.lp.chunks().count() as u64;
        self.stats.fading = self.fades.len() as u64;
        self.stats.coarse = store.coarse.len() as u64;
        self.stats.digs = (self.digs.len() + self.dig_open.len()) as u64;
        self.stats.reach = self.reach.len() as u64;
        self.stats.partial = self.partial.len() as u64;
        self.laps.lap(S_FADES);
        // The summaries' and parents' time ran inside the placement, the fades, the evictions
        // and the window's move: it moves to its own stage, from those in that order.
        let mut summaries = store.take_summary_ms();
        for stage in [S_PLACE, S_FADES, S_GONE, S_REWINDOW] {
            let p = summaries.min(self.laps.ms[stage]);
            self.laps.ms[stage] -= p;
            self.laps.ms[S_SUMMARIES] += p;
            summaries -= p;
        }
        self.stats.stages = StepStages {
            ms: self.laps.ms,
            total_ms: stopwatch().duration_since(t0).as_secs_f64() * 1e3,
            opened: opened_step,
        };
        Ok(self.stats)
    }
}
