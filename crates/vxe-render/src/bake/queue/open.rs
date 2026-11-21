//! The queue's job opening, its view census and the withdrawal of passed faces.

use super::*;
use crate::gpu_world::COLUMN_NONE;

impl BakeQueue {
    /// Opens jobs in [`BakeQueue::order`] while the open ones hold under two slices of `cap`
    /// samples: each a ready candidate's faces at the brick level.
    pub(in crate::bake) fn open(
        &mut self,
        run: &BakeRun<'_>,
        store: &mut Store,
        view: &QueueView<'_>,
        cap: u64,
    ) -> Result<(), BakeError> {
        let gpu = run.gpu;
        let eye = view.eye.map(|v| (v / 64.0).floor() as i32);
        let order = self.order;
        self.score_and_census(run, store, view, eye);
        let steps = self.steps;
        let recent = |at: Option<&u64>| at.is_some_and(|&at| steps - at < QUEUE_RECHECK);
        let checks = if view.samples.is_some() {
            QUEUE_CHECKS_UNSLICED
        } else {
            QUEUE_CHECKS
        };
        let classes = self.classes(run.world.materials());
        let lr = gpu.light_radius();
        // Tiered (the distance order, the score's tier): class 1 (refinement) is opened only by
        // an opening that found no class 0 candidate ready, so every chunk with no block that
        // can bake comes first; those whose reach or lights are not ready (a streamed world's
        // edge) cannot starve the refinement behind them, each class counting its own checks.
        // Under the score's refine weight the class is in the key: one count of checks runs
        // over the sorted list, and a candidate found blocked is skipped for QUEUE_RECHECK
        // steps, so the next openings check the candidates behind it.
        let tiered = !matches!(
            order,
            QueueOrder::Score {
                refine: Refine::Weight(_),
                ..
            }
        );
        let (mut checked, mut reach, mut lights) = ([0usize; 2], 0u64, 0u64);
        let mut ready0 = 0usize;
        // A step without a slice (the pre-roll) gates up to `checks` candidates and lists them
        // across threads; a sliced step lists them in batches and stops at two slices of
        // samples.
        let parallel = view.samples.is_some();
        let mut ready: Vec<(u8, [i32; 3])> = Vec::new();
        // The loop's samples left, read once (it walks every open face) and then counted on.
        let mut left = self.lp.samples_left();
        let angle_needs = self.census_every_opening;
        let nearest_in_view = angle_needs
            .then(|| self.nearest_in_view(eye, view))
            .flatten();
        // Each lead in turn (the class when tiered): a lead whose checks are spent is left for
        // the next, as a walk through the rest of it would skip them all.
        let mut lead = 0usize;
        let mut next = 0;
        // A sliced opening lists its ready candidates in batches: a candidate joins the batch
        // while the loop's samples left plus the batch's bounds ([`BakeQueue::listing_bound`],
        // at least its listing's samples) stay under two slices, the batch is listed across
        // threads and pushed in the walk's order, and the walk goes on from the samples the
        // pushes left. Every candidate the one-at-a-time walk would push is pushed, in its order.
        let mut batch: Vec<(u8, [i32; 3])> = Vec::new();
        let mut est = left;
        let mut full_blank = false;
        let stops = |left: u64, checked: &[usize; 2], class: u8, ready0: usize| {
            if tiered {
                left >= 2 * cap || checked[1] >= checks || (class == 1 && ready0 > 0)
            } else {
                left >= 2 * cap || checked[0] >= checks
            }
        };
        while let Some(Cand {
            class, coarse, c, ..
        }) = self.walk_next(&mut lead, &mut next, tiered, checked[0] >= checks)
        {
            // A kept candidate pushed, or listed faceless, since the order was scored has left
            // the pool; a class change re-scores the order before the walk.
            if self.pool.get(&c) != Some(&(class, coarse)) {
                continue;
            }
            if stops(est, &checked, class, ready0) {
                if batch.is_empty() {
                    next -= 1;
                    break;
                }
                self.laps.lap(S_CHECKS);
                let pushed = self.push_batch(run, store, eye, &classes, &mut batch)?;
                self.laps.lap(S_LIST);
                left += pushed.samples;
                ready0 -= pushed.refused0;
                full_blank |= pushed.full_blank;
                est = left;
                if stops(est, &checked, class, ready0) {
                    next -= 1;
                    break;
                }
            }
            if tiered {
                if checked[usize::from(class)] >= checks || recent(self.blocked.get(&c)) {
                    continue;
                }
                checked[usize::from(class)] += 1;
            } else {
                if recent(self.blocked.get(&c)) {
                    continue;
                }
                checked[0] += 1;
            }
            let coord = ChunkCoord(c);
            let Some((partial, provisional)) = self.bakeable(run, coord, lr, view, eye) else {
                reach += 1;
                self.blocked.insert(c, self.steps);
                continue;
            };
            if !gpu.lights_ready_near(coord, lr) {
                lights += 1;
                self.blocked.insert(c, self.steps);
                continue;
            }
            self.blocked.remove(&c);
            if partial {
                self.partial_checked.insert(c);
            } else {
                self.partial_checked.remove(&c);
            }
            if provisional {
                self.provisional_checked.insert(c);
            } else {
                self.provisional_checked.remove(&c);
            }
            if self.zero_rule
                && class == 0
                && !partial
                && !provisional
                && self.rule_holds(gpu, coord, lr)
            {
                self.ruled_checked.insert(c);
            } else {
                self.ruled_checked.remove(&c);
            }
            if class == 0 {
                ready0 += 1;
            }
            if angle_needs {
                self.note_angle_need(c, eye, view, nearest_in_view);
            }
            if parallel {
                ready.push((class, c));
                continue;
            }
            est += self.listing_bound(c, class, eye);
            batch.push((class, c));
        }
        self.laps.lap(S_CHECKS);
        if !batch.is_empty() {
            let pushed = self.push_batch(run, store, eye, &classes, &mut batch)?;
            full_blank |= pushed.full_blank;
            self.laps.lap(S_LIST);
        }
        if !ready.is_empty() {
            self.push_batch(run, store, eye, &classes, &mut ready)?;
        }
        if full_blank && !parallel {
            // The class 0 walk's place, or past its end once the walk reached class 1.
            let next = if lead == 0 { next } else { usize::MAX };
            self.note_starved(run, view, eye, next, checks);
        }
        self.stats.waiting_reach = reach;
        self.stats.waiting_lights = lights;
        // A checked candidate not listed (a refused place) is checked again before its listing.
        self.partial_checked.clear();
        self.provisional_checked.clear();
        self.ruled_checked.clear();
        Ok(())
    }

    /// The opening's next candidate in the kept order, `next` past it: lead `lead`'s, or once
    /// that lead is out (or `spent`, its checks used) under a `tiered` order, lead 1's.
    fn walk_next(
        &mut self,
        lead: &mut usize,
        next: &mut usize,
        tiered: bool,
        spent: bool,
    ) -> Option<Cand> {
        if tiered && *lead == 0 && spent {
            (*lead, *next) = (1, 0);
        }
        loop {
            if let Some(k) = self.ordered.get(*lead, *next, &self.pool) {
                *next += 1;
                return Some(k);
            }
            if *lead == 0 && tiered {
                (*lead, *next) = (1, 0);
            } else {
                return None;
            }
        }
    }

    /// The squared chunk distance of the nearest candidate in the view cone or the near shell.
    fn nearest_in_view(&self, eye: [i32; 3], view: &QueueView<'_>) -> Option<i64> {
        let (order, near) = (self.order, self.near);
        self.pool
            .iter()
            .map(|(&c, &(class, coarse))| cand_of(c, class, coarse, eye, view, order, near))
            .filter(|k| k.a <= 0.0)
            .map(|k| k.d2)
            .min()
    }

    /// The opening's candidates, census and sort stages: the kept order scored again when a
    /// candidate joined or changed class or the view it reads changed, and the census of the
    /// candidates by view and reach when one is due.
    fn score_and_census(
        &mut self,
        run: &BakeRun<'_>,
        store: &mut Store,
        view: &QueueView<'_>,
        eye: [i32; 3],
    ) {
        let gpu = run.gpu;
        let order = self.order;
        let near = self.near;
        self.sync_pool(run, store);
        // The kept order is scored again when a candidate joined or changed class, or the view
        // the scores read changed; else it is walked as it stands.
        let key = OrderView {
            eye,
            forward: view.forward.map(f32::to_bits),
            half_angle: view.half_angle.to_bits(),
            order,
            near,
        };
        if self.ordered.set_view(key, view.forward, view.half_angle) {
            self.ordered.rebuild(self.pool.iter());
            self.stats.rebuilt += 1;
        }
        self.laps.lap(S_CANDIDATES);
        let census = self.census_every_opening || std::mem::take(&mut self.census_due);
        let steps = self.steps;
        let recent = |at: Option<&u64>| at.is_some_and(|&at| steps - at < QUEUE_RECHECK);
        if census {
            let t = stopwatch();
            self.entered
                .retain(|c, _| gpu.is_mirrored(ChunkCoord(*c)) && !gpu.is_lod(ChunkCoord(*c)));
            let cands: Vec<Cand> = self
                .pool
                .iter()
                .map(|(&c, &(class, coarse))| cand_of(c, class, coarse, eye, view, order, near))
                .collect();
            // The chunks in a job still wait for their light: those with no block or a
            // coarse one, not a dig's or a key-only re-bake's.
            let jobs: Vec<([i32; 3], u8, bool)> = self
                .lp
                .chunks()
                .map(|c| c.0)
                .filter(|c| !self.dig_open.contains_key(c) && !self.keys.holds(c))
                .filter_map(|c| {
                    let has = store.blocks.contains_key(&c);
                    let coarse = has && self.lesser(store, c);
                    (!has || coarse).then_some((c, u8::from(has), coarse))
                })
                .collect();
            self.view_census(eye, view, &cands, &jobs, gpu.light_radius());
            self.view_reach_census(run, eye, view, &cands);
            self.stats.unbaked = cands
                .iter()
                .filter(|k| k.class == 0 && !recent(self.blocked.get(&k.c)))
                .count() as u64;
            self.stats.census_ms = seconds_since(t) * 1e3;
        }
        self.laps.lap(S_CENSUS);
        self.stats.depth = self.pool.len() as u64;
        self.stats.sealed = self.sealed.count();
        self.counted_idle = self.lp.chunks().next().is_none();
        self.laps.lap(S_SORT);
    }

    /// Once in [`VIEW_REACH_EVERY`] steps, at an opening: up to [`VIEW_REACH_SAMPLE`] of the
    /// candidates in the view cone beyond the shell, spread evenly by distance, checked for
    /// their reach ([`QueueStats::view_reach`]).
    pub(in crate::bake) fn view_reach_census(
        &mut self,
        run: &BakeRun<'_>,
        eye: [i32; 3],
        view: &QueueView<'_>,
        cands: &[Cand],
    ) {
        if self.steps < self.reach_census_at + VIEW_REACH_EVERY && self.reach_census_at > 0 {
            return;
        }
        self.reach_census_at = self.steps;
        let mut in_view: Vec<(i64, [i32; 3])> = cands
            .iter()
            .filter(|k| {
                let o = [0, 1, 2].map(|i| k.c[i] - eye[i]);
                k.a <= 0.0 && !in_near_shell(o, self.near)
            })
            .map(|k| (k.d2, k.c))
            .collect();
        in_view.sort_unstable();
        let n = in_view.len();
        let take = n.min(VIEW_REACH_SAMPLE);
        let lr = run.gpu.light_radius();
        let mut not_ready = 0;
        for i in 0..take {
            let (_, c) = in_view[i * n / take.max(1)];
            not_ready += u64::from(!reach_ready(run, ChunkCoord(c), lr, view));
        }
        self.stats.view_reach = [take as u64, not_ready];
    }

    /// Withdraws the passed jobs ([`BakeQueue::withdraw`]) when a candidate in the near shell or
    /// the view cone is ready: up to the opening's count of checks of those candidates, nearest
    /// first, each not checked in the last [`QUEUE_RECHECK`] steps. A step with a sample count
    /// (the pre-roll) withdraws nothing: the camera has passed nothing yet, and the coarse pass
    /// opens chunks in every direction, so withdrawing there takes back most of what it opens.
    pub(in crate::bake) fn withdraw_passed(
        &mut self,
        run: &BakeRun<'_>,
        store: &mut Store,
        view: &QueueView<'_>,
    ) {
        if !self.withdraw || view.samples.is_some() {
            return;
        }
        let (eye, near, moves) = (self.eye, self.near, self.eye_moves);
        let keep = view.half_angle + QUEUE_VIEW_MARGIN_DEG.to_radians();
        let job_eye = &self.job_eye;
        let passed: std::collections::HashSet<[i32; 3]> = self
            .lp
            .chunks()
            .map(|c| c.0)
            .filter(|c| {
                let o = [0, 1, 2].map(|k| c[k] - eye[k]);
                let full = store.blocks.contains_key(c) && !self.lesser(store, *c);
                // Only a job opened before the camera's chunk last changed: the camera has moved
                // since, so it can have passed the chunk.
                job_eye.get(c).is_some_and(|&at| at < moves)
                    && !self.dig_open.contains_key(c)
                    && !full
                    && !in_near_shell(o, near)
                    && angle_term(o, view.forward, keep, near) > 0.0
            })
            .collect();
        if passed.is_empty() {
            return;
        }
        let gpu = run.gpu;
        self.sync_pool(run, store);
        let mut seen: Vec<(i64, [i32; 3])> = self
            .pool
            .keys()
            .filter_map(|c| {
                let o = [0, 1, 2].map(|k| c[k] - eye[k]);
                (angle_term(o, view.forward, view.half_angle, near) <= 0.0).then(|| {
                    let d2: i64 = o.iter().map(|&v| i64::from(v).pow(2)).sum();
                    (d2, *c)
                })
            })
            .collect();
        seen.sort_unstable();
        let steps = self.steps;
        let lr = gpu.light_radius();
        let checks = QUEUE_CHECKS;
        let mut ready = false;
        let mut checked = 0;
        for (_, c) in seen {
            if checked >= checks {
                break;
            }
            if self
                .blocked
                .get(&c)
                .is_some_and(|&at| steps - at < QUEUE_RECHECK)
            {
                continue;
            }
            checked += 1;
            let coord = ChunkCoord(c);
            if self.bakeable(run, coord, lr, view, eye).is_some()
                && gpu.lights_ready_near(coord, lr)
            {
                ready = true;
                break;
            }
            self.blocked.insert(c, steps);
        }
        if !ready {
            return;
        }
        let record = self.record_words();
        for w in self.lp.withdraw(|c| passed.contains(&c.0), record) {
            store.release(w.first_word, w.words);
            self.job_coarse.remove(&w.coord.0);
            self.job_count.remove(&w.coord.0);
            self.job_eye.remove(&w.coord.0);
            self.job_partial.remove(&w.coord.0);
            self.job_provisional.remove(&w.coord.0);
            self.stale.remove(&w.coord.0);
            self.stats.withdrawn += 1;
            self.stats.withdrawn_samples += w.drawn;
        }
        // The candidates are counted and sorted again this step.
        self.counted_idle = false;
        self.quiet_until = self.steps;
    }

    /// The view's counters over the candidates of an opening ([`QueueStats::view_waiting`] and
    /// the rest), and the chunks entering the view cone or the near shell with no full block.
    /// `jobs` (each chunk, its class and whether its block is coarse) join the in-view count
    /// ([`QueueStats::in_view`]) only: a chunk in a job still waits for its light.
    pub(in crate::bake) fn view_census(
        &mut self,
        eye: [i32; 3],
        view: &QueueView<'_>,
        cands: &[Cand],
        jobs: &[([i32; 3], u8, bool)],
        lr: i32,
    ) {
        let mut waiting = [[0u64; 2]; 3];
        let mut reach_bound = [0u64; 2];
        let mut census = [[[0u32; 4]; 4]; 2];
        let mut cone_only = 0u64;
        let mut waiting_all: Vec<([i32; 3], u8, bool, u64)> = Vec::new();
        let mut lens: [Vec<f32>; 2] = [Vec::new(), Vec::new()];
        let (offsets, extent): (Vec<[i32; 3]>, f64) = if view.full_radius.is_some() {
            (reach_offsets(lr).collect(), reach_extent(lr))
        } else {
            (Vec::new(), 0.0)
        };
        for &Cand {
            d2,
            c,
            a,
            class: has,
            coarse,
            ..
        } in cands
        {
            if has == 1 && !coarse {
                // A full block re-baking whole (ring 3): not waiting for its light.
                continue;
            }
            let o = [0, 1, 2].map(|k| c[k] - eye[k]);
            let class = if in_near_shell(o, self.near) {
                VIEW_SHELL
            } else if a <= 0.0 {
                VIEW_IN
            } else {
                VIEW_OUT
            };
            let len = (d2 as f32).sqrt();
            waiting[class][usize::from(coarse)] += 1;
            census[usize::from(coarse)][distance_band(len)][angle_band(a)] += 1;
            if class != VIEW_OUT {
                waiting_all.push((c, has, coarse, self.need_samples(c, has, eye)));
            }
            if class == VIEW_IN {
                if view
                    .full_radius
                    .is_some_and(|r| is_reach_bound(o, &offsets, extent, r))
                {
                    reach_bound[usize::from(self.band_of(c) != Band::Near)] += 1;
                }
                lens[usize::from(coarse)].push(len);
                if view
                    .frustum
                    .is_some_and(|f| !in_widened_frustum(o, view.forward, f))
                {
                    cone_only += 1;
                }
            }
            if class != VIEW_OUT && !self.entered.contains_key(&c) {
                self.entered.insert(c, (self.steps, coarse));
                if coarse {
                    self.ages_first.push(0);
                }
            }
        }
        self.stats.view_waiting = waiting;
        self.stats.reach_bound = reach_bound;
        self.stats.weight_census = census;
        for &(c, has, coarse) in jobs {
            waiting_all.push((c, has, coarse, self.need_samples(c, has, eye)));
        }
        self.stats.cone_only = cone_only;
        // The drain's set (ADR-0080 rule 3): the chunks the census counts in view waiting.
        let eye_c = view.eye.map(|v| (v / 64.0).floor() as i32);
        self.view_waiting = waiting_all
            .iter()
            .filter(|&&(c, has, coarse, _)| {
                let o = [0, 1, 2].map(|k| c[k] - eye_c[k]);
                !(has == 1 && !coarse)
                    && angle_term(o, view.forward, view.half_angle, self.near) <= 0.0
            })
            .map(|w| w.0)
            .collect();
        self.stats.in_view = in_view_census(view, self.near, self.far_normal, waiting_all);
        self.stats.view_median = lens.map(|mut l| {
            l.sort_unstable_by(f32::total_cmp);
            l.get(l.len() / 2).copied().unwrap_or(0.0)
        });
    }

    /// Notes the angle weight at which the nearest candidate in the view cone or the near shell
    /// (squared distance `nearest`) would have outranked `c`, found ready
    /// ([`BakeQueue::take_angle_needs`]).
    pub(in crate::bake) fn note_angle_need(
        &mut self,
        c: [i32; 3],
        eye: [i32; 3],
        view: &QueueView<'_>,
        nearest: Option<i64>,
    ) {
        let o = [0, 1, 2].map(|k| c[k] - eye[k]);
        let a = angle_term(o, view.forward, view.half_angle, self.near);
        let len = queue_score(o, 0.0, false, 0.0, 1.0);
        let need = match nearest {
            Some(n) if a > 0.0 && len > 0.0 => ((n as f32).sqrt() / len - 1.0).max(0.0) / a,
            _ => 0.0,
        };
        // Bounded: one second of openings in a pre-roll is a few thousand.
        if self.angle_needs.len() < 1 << 16 {
            self.angle_needs.push(need);
        }
    }
}

impl BakeQueue {
    /// A reading of what the rules would answer of each chunk the queue lists
    /// ([`ListingCensus`]): every candidate and every job's chunk with no full block, listed
    /// from its masks or the world (across threads), its reach box read against the column
    /// map ([`GpuWorld::column_lowest`]) and the emitters ([`GpuWorld::emitters_near`]), the
    /// rest counted by band and view. Costs a listing of every chunk: a reading, not a step.
    pub fn listing_census(
        &mut self,
        gpu: &GpuWorld,
        world: &World,
        view: &QueueView<'_>,
    ) -> ListingCensus {
        let t = stopwatch();
        let classes = self.classes(world.materials());
        let mut chunks: Vec<[i32; 3]> = self.pool.keys().copied().collect();
        chunks.extend(self.lp.chunks().map(|c| c.0));
        chunks.sort_unstable();
        chunks.dedup();
        let lr = gpu.light_radius();
        let threads = std::thread::available_parallelism()
            .map_or(4, std::num::NonZero::get)
            .clamp(1, 16);
        let per = chunks.len().div_ceil(threads).max(1);
        let this = &*self;
        let faces: Vec<bool> = std::thread::scope(|scope| {
            let handles: Vec<_> = chunks
                .chunks(per)
                .map(|part| {
                    let classes = classes.as_slice();
                    scope.spawn(move || {
                        part.iter()
                            .map(|&c| this.list(world, ChunkCoord(c), classes).0.faces() > 0)
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|h| h.join().expect("a census thread panicked"))
                .collect()
        });
        let eye = view.eye.map(|v| (v / 64.0).floor() as i32);
        let held = held_by_streamer(view.eye, view.lod_radius, view.resident);
        let held = held.as_ref().map(|h| h as &dyn Fn(ChunkCoord) -> bool);
        let bands = view.focal_px.map(|focal_px| Bands {
            focal_px,
            lo: self.far_normal[0],
            hi: self.far_normal[1],
        });
        let mut lowest: std::collections::HashMap<[i32; 2], i32> = Default::default();
        let mut out = ListingCensus::default();
        for (&c, has_face) in chunks.iter().zip(faces) {
            out.listed += 1;
            if !has_face {
                out.no_face += 1;
                let edge = (0..6).any(|f| {
                    let mut n = c;
                    n[f / 2] += if f % 2 == 1 { 1 } else { -1 };
                    !gpu.is_mirrored(ChunkCoord(n)) || gpu.is_lod(ChunkCoord(n))
                });
                out.no_face_edge += u64::from(edge);
                continue;
            }
            let coord = ChunkCoord(c);
            let box_top = (c[2] + lr + 1) * 64 - 1;
            let mut floor = i32::MAX;
            for y in c[1] - lr..=c[1] + lr {
                for x in c[0] - lr..=c[0] + lr {
                    let l = *lowest
                        .entry([x, y])
                        .or_insert_with(|| gpu.column_lowest(x, y));
                    floor = floor.min(l);
                }
            }
            if floor != COLUMN_NONE && box_top < floor && !gpu.emitters_near(coord, lr) {
                if reach_columns_final(coord, lr, |x, y| gpu.column_final(x, y, held)) {
                    out.buried += 1;
                } else {
                    out.buried_unfinal += 1;
                }
                continue;
            }
            let band = bands.map_or(Band::Near, |b| b.of_chunk(c, view.eye));
            let o = [0, 1, 2].map(|k| c[k] - eye[k]);
            let in_view = angle_term(o, view.forward, view.half_angle, self.near) <= 0.0;
            out.surface[band.index()][usize::from(in_view)] += 1;
            if view
                .full_radius
                .is_some_and(|r| reach_crosses(coord, eye, lr, r))
            {
                out.reach_bound[band.index()] += 1;
            }
        }
        out.ms = seconds_since(t) * 1e3;
        out
    }
}

/// Whether a chunk at offset `o` (chunks) from the eye's chunk is reach-bound: some chunk of its
/// reach (`offsets`, [`reach_offsets`] at the light radius) lies past the full radius `r_full`
/// (the streamer's `d2 > r^2`), the test [`reach_crosses`] makes; a chunk whose distance plus
/// the reach's extent (`extent`, [`reach_extent`]) stays within the radius reads no offset.
pub(in crate::bake) fn is_reach_bound(
    o: [i32; 3],
    offsets: &[[i32; 3]],
    extent: f64,
    r_full: i32,
) -> bool {
    let len = o.iter().map(|&d| f64::from(d).powi(2)).sum::<f64>().sqrt();
    if len + extent <= f64::from(r_full) {
        return false;
    }
    let r2 = i64::from(r_full).pow(2);
    offsets
        .iter()
        .any(|n| (0..3).map(|k| i64::from(o[k] + n[k]).pow(2)).sum::<i64>() > r2)
}

/// The chunks of `chunks` in view (the near shell of `near` chunks about the camera's chunk, or
/// the view cone, [`angle_term`]) that wait for their light, by band and class
/// ([`InViewWaiting`]): each chunk its coordinate, its class (1 when it holds a block), whether
/// its block is coarse, and the samples it still needs. A chunk holding a full block waits for
/// nothing and is skipped. The band is [`Bands::of_chunk`] at `view`'s focal length and
/// `far_normal`; every chunk is near without a focal length.
pub fn in_view_census(
    view: &QueueView<'_>,
    near: i32,
    far_normal: [f32; 2],
    chunks: impl IntoIterator<Item = ([i32; 3], u8, bool, u64)>,
) -> InViewWaiting {
    let eye = view.eye.map(|v| (v / 64.0).floor() as i32);
    let bands = view.focal_px.map(|focal_px| Bands {
        focal_px,
        lo: far_normal[0],
        hi: far_normal[1],
    });
    let mut out = InViewWaiting::default();
    for (c, has, coarse, need) in chunks {
        if has == 1 && !coarse {
            continue;
        }
        let o = [0, 1, 2].map(|k| c[k] - eye[k]);
        if angle_term(o, view.forward, view.half_angle, near) > 0.0 {
            continue;
        }
        let band = bands.map_or(Band::Near, |b| b.of_chunk(c, view.eye));
        out.note(band, coarse, need);
    }
    out
}
