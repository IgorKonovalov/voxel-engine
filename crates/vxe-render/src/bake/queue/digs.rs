//! The queue's digs: the boxes to rebake, their opening, and the fades they start.

use super::*;
use crate::bake::held::{Opaque, apply_held, held_visibilities};
use crate::bake::job_loop::JobCtx;

/// A chunk's held visibilities ([`held_visibilities`]): by touched brick, a value a cell.
type Held = Vec<(u16, Vec<Option<[f32; 6]>>)>;

impl BakeQueue {
    /// Reads the edit boxes since the last step into the waiting digs, against the store as the
    /// edits left it (no re-bake has run since): a job of any kind but a key-only one in a ring-1
    /// chunk (one holding a brick a box dilated by one voxel touches), and a dig's job in any
    /// ring chunk, is withdrawn ([`BakeQueue::withdraw_at`]): a dig's bricks rejoin its chunk's
    /// digs, any other job's chunk is a candidate again. Each ring chunk with a block then takes
    /// its ring's bricks, its ring-1 bricks marked; a job still running in a ring chunk is
    /// thrown away when it ends. A touched brick that held light opens ring 3. Each chunk
    /// holding a touched brick is listed again at once and takes a provisional block
    /// ([`provisional_block`]): its kept bricks' records, and a brick the edit opened its
    /// neighbours' mean, so no dug face reads no value while its job waits; the job then fades
    /// from it. Under [`BakeQueue::dig_store_hold`] each touched brick's cells hold the held
    /// visibilities instead ([`held_visibilities`]: the removed surface's times a near openness
    /// read against the world), which the light pass reads as a record.
    pub(in crate::bake) fn take_boxes(
        &mut self,
        run: &BakeRun<'_>,
        store: &mut Store,
    ) -> Result<(), BakeError> {
        if self.boxes.is_empty() {
            return Ok(());
        }
        let boxes = std::mem::take(&mut self.boxes);
        let Some(piece) = store.piece else {
            return Ok(());
        };
        let record = self.record_words();
        let touches = |c: [i32; 3], slot: u16| {
            let b = brick_of_slot(usize::from(slot));
            boxes.iter().any(|[min, max]| {
                (0..3).all(|k| {
                    let lo = c[k] * 64 + b[k] * 8;
                    lo + 7 >= min[k] - 1 && lo <= max[k] + 1
                })
            })
        };
        let ring = dig_ring(&boxes, piece);
        let withdrawn: std::collections::HashSet<[i32; 3]> = ring
            .iter()
            .filter(|(c, slots)| {
                !self.keys.open.contains(*c)
                    && (self.dig_open.contains_key(*c) || slots.iter().any(|&s| touches(**c, s)))
            })
            .map(|(c, _)| *c)
            .collect();
        self.withdraw_at(store, &withdrawn);
        let in_job: std::collections::HashSet<[i32; 3]> = self.lp.chunks().map(|c| c.0).collect();
        let mut lit = false;
        let mut dug = Vec::new();
        for (c, slots) in ring {
            // The ring's visibilities predate the edit: the light pass casts the sky rays at
            // their faces until the re-bake lands.
            store.mark_sky_stale(run.queue, ChunkCoord(c), &slots, record);
            let near: Vec<u16> = slots.iter().copied().filter(|&s| touches(c, s)).collect();
            if !lit && let Some(block) = store.words.get(&c) {
                lit = near.iter().any(|&s| brick_lit(block, s, record));
            }
            if in_job.contains(&c) {
                self.stale.insert(c);
            }
            if !store.blocks.contains_key(&c) {
                // A chunk the box touched that holds no block (air before a stamp) is a whole
                // dig: it opens in the digs' order instead of waiting in the pool. A ring chunk
                // the box does not touch keeps the pool's path.
                let coord = ChunkCoord(c);
                if !near.is_empty() && run.gpu.is_mirrored(coord) && !run.gpu.is_lod(coord) {
                    self.stats.digs_whole += 1;
                    let e = self.digs.entry(c).or_default();
                    e.whole = true;
                    e.near = true;
                    self.touched.insert(c);
                    self.faceless.remove(&c);
                }
                continue;
            }
            let e = self.digs.entry(c).or_default();
            self.touched.insert(c);
            if let Some(open) = self.dig_open.get(&c) {
                e.slots.extend(open.slots.iter().copied());
                e.near |= open.near;
            }
            e.slots.extend(slots);
            e.ring1.extend(near.iter().copied());
            e.near |= !near.is_empty();
            if !near.is_empty() {
                dug.push((c, near));
            }
        }
        let classes = self.classes(run.world.materials());
        let listed = self.list_dug(run, &store.words, &boxes, &dug, &classes);
        for ((c, near), (faces, held)) in dug.into_iter().zip(listed) {
            let coord = ChunkCoord(c);
            if let Some(m) = self.prepared.remove(&c) {
                self.prepared_bytes -= m.masks.bytes();
            }
            let Some(&(first, words)) = store.blocks.get(&c) else {
                continue;
            };
            if faces.faces() == 0 {
                self.fades.remove(&c);
                store.remove(run.queue, coord);
                self.block_count.remove(&c);
                self.faceless.insert(c);
                self.touched.insert(c);
                self.digs.remove(&c);
                continue;
            }
            let mut block = provisional_block(store, c, &faces, record, &near);
            apply_held(&mut block, &held, record);
            let len = block.len() as u64;
            // The old place is freed only once the new one is held: a refused place leaves the
            // old block where the chunk table points, and its dig waits in `digs`.
            let at = if len <= words {
                store.release(first + len, words - len);
                first
            } else {
                match store.alloc(len) {
                    Ok(at) => {
                        store.release(first, words);
                        at
                    }
                    Err(BakeError::PoolFull { .. }) => {
                        self.stats.pool_full += 1;
                        continue;
                    }
                    Err(e) => return Err(e),
                }
            };
            self.fades.remove(&c);
            store.put(run.queue, coord, at, block);
            self.listed.insert(c, faces);
        }
        if !(lit && self.reach_ring) {
            return Ok(());
        }
        let lr = run.gpu.light_radius();
        for [min, max] in &boxes {
            let lo = min.map(|v| (v - 1).div_euclid(64));
            let hi = max.map(|v| (v + 1).div_euclid(64));
            for z in lo[2] - lr..=hi[2] + lr {
                for y in lo[1] - lr..=hi[1] + lr {
                    for x in lo[0] - lr..=hi[0] + lr {
                        let n = [x, y, z];
                        let gap2: i64 = (0..3)
                            .map(|k| i64::from((lo[k] - n[k]).max(n[k] - hi[k]).max(0)).pow(2))
                            .sum();
                        if gap2 <= i64::from(lr).pow(2) && store.blocks.contains_key(&n) {
                            self.reach.insert(n);
                            self.touched.insert(n);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// The dug chunks' listings, across threads (a `B` press touches tens of chunks, listed as
    /// the pre-roll's are), each with its touched bricks' held visibilities under
    /// [`BakeQueue::dig_store_hold`] ([`held_visibilities`], made against the world from the
    /// blocks as the edit found them, `words`), else none.
    fn list_dug(
        &self,
        run: &BakeRun<'_>,
        words: &BTreeMap<[i32; 3], Vec<u32>>,
        boxes: &[[[i32; 3]; 2]],
        dug: &[([i32; 3], Vec<u16>)],
        classes: &Arc<Vec<u8>>,
    ) -> Vec<(ChunkFaces, Held)> {
        let threads = std::thread::available_parallelism()
            .map_or(4, std::num::NonZero::get)
            .clamp(1, 16);
        let per = dug.len().div_ceil(threads).max(1);
        let (cell, hold) = (self.settings.cell, self.dig_store_hold);
        std::thread::scope(|scope| {
            let handles: Vec<_> = dug
                .chunks(per)
                .map(|part| {
                    scope.spawn(move || {
                        part.iter()
                            .map(|(c, near)| {
                                let faces = self.list(run.world, ChunkCoord(*c), classes).0;
                                let held = if hold {
                                    let mut solid =
                                        Opaque::about(run.world, &classes[..], *c, near);
                                    held_visibilities(
                                        &mut solid, words, boxes, *c, &faces, near, cell,
                                    )
                                } else {
                                    Vec::new()
                                };
                                (faces, held)
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|h| h.join().expect("a listing thread panicked"))
                .collect()
        })
    }

    /// Opens jobs for the waiting dig chunks not in a job, ahead of every pending job: ring-1
    /// chunks first, then by distance from the camera's chunk. A ring-1 chunk's job re-bakes
    /// its ring-1 bricks alone and keeps every other record (ring 2's stale ones too, which the
    /// ring's hold shows); its other dig bricks wait in `digs` until it is placed, then open as
    /// a second job. Every ring-1 job opens in this step, outside the two slices, into a table
    /// slot (where none is free, the last open job that is not a dig's is withdrawn to the
    /// candidates), and the submit plans its rounds first. The rest open in a step that opened
    /// no ring-1 job, while the loop holds under two slices of `cap` samples (one at least).
    /// Each chunk is listed again on this thread (a split chunk's second job reuses its first's
    /// listing) and keeps every brick record outside its job's bricks (a whole dig, which held
    /// no block, keeps none and bakes every listed brick) and bakes them at the near tier's
    /// count, whatever its band; a chunk left with no face drops its block.
    pub(in crate::bake) fn open_digs(
        &mut self,
        run: &BakeRun<'_>,
        store: &mut Store,
        cap: u64,
    ) -> Result<(), BakeError> {
        if self.digs.is_empty() {
            return Ok(());
        }
        let eye = self.eye;
        let in_job: std::collections::HashSet<[i32; 3]> = self.lp.chunks().map(|c| c.0).collect();
        let mut order: Vec<(bool, i64, [i32; 3])> = self
            .digs
            .iter()
            .filter(|(c, _)| !in_job.contains(*c))
            .map(|(c, d)| {
                let d2: i64 = (0..3).map(|k| i64::from(c[k] - eye[k]).pow(2)).sum();
                (!d.near, d2, *c)
            })
            .collect();
        order.sort_unstable();
        // The dig jobs already pending or open count against the two slices; the rest of the
        // loop does not, so a dig never waits behind the queue's own depth.
        let mut left: u64 = self.lp.dig_samples_left(|c| self.dig_open.contains_key(c));
        let classes = self.classes(run.world.materials());
        let record = self.record_words();
        // A dig's ring bakes at the near tier's count whatever its chunk's band: the eye is on
        // it (ADR-0080 rule 2).
        let brick_samples = self.near_count();
        let own_count = (brick_samples != self.settings.brick_samples).then_some(brick_samples);
        let mut jobs = Vec::new();
        let mut first = 0;
        for (_, _, c) in order {
            let ring1 = self.digs.get(&c).is_some_and(|d| d.near);
            // A step that opens a ring-1 job opens no other dig's: their listings wait a step,
            // so the edit's frame lists no more than before.
            if !ring1 && (first > 0 || (left >= 2 * cap && !jobs.is_empty())) {
                break;
            }
            self.touched.insert(c);
            let Some(mut d) = self.digs.remove(&c) else {
                continue;
            };
            // A ring-1 chunk's first job: its ring-1 bricks; the rest wait for its placing, and
            // open from this listing (the box touched no brick of theirs).
            let split = !d.whole && !d.ring1.is_empty() && d.ring1.len() < d.slots.len();
            if split {
                let rest: std::collections::BTreeSet<u16> =
                    d.slots.difference(&d.ring1).copied().collect();
                self.digs.insert(
                    c,
                    DigChunk {
                        slots: rest,
                        ..DigChunk::default()
                    },
                );
                d.slots = std::mem::take(&mut d.ring1);
                d.ring1 = d.slots.clone();
            }
            let coord = ChunkCoord(c);
            let kept: KeptBricks = match store.words.get(&c) {
                Some(block) => records_of(block, record)
                    .into_iter()
                    .filter(|(slot, _)| !d.slots.contains(slot))
                    .collect(),
                None if d.whole => KeptBricks::default(),
                None => continue,
            };
            let faces = match self.listed.remove(&c) {
                Some(f) => f,
                None => self.list(run.world, coord, &classes).0,
            };
            if d.whole {
                d.slots = faces.bricks.iter().map(|(slot, _)| *slot).collect();
            }
            if let Some(m) = self.prepared.remove(&c) {
                self.prepared_bytes -= m.masks.bytes();
            }
            if faces.faces() == 0 {
                store.remove(run.queue, coord);
                self.block_count.remove(&c);
                self.fades.remove(&c);
                self.faceless.insert(c);
                self.touched.insert(c);
                self.digs.remove(&c);
                continue;
            }
            let samples = faces
                .bricks
                .iter()
                .filter(|(slot, _)| d.slots.contains(slot))
                .map(|(_, keys)| {
                    let mut dirs = [false; 6];
                    for k in keys {
                        dirs[usize::from(k & 7)] = true;
                    }
                    dirs.iter().filter(|d| **d).count() as u64 * u64::from(brick_samples)
                })
                .sum();
            let first_word = match store.alloc(layout(&faces, record).1) {
                Ok(first) => first,
                Err(BakeError::PoolFull { .. }) => {
                    // The dig waits for a place, listed: no later dig opens ahead of it. A
                    // split chunk's bricks join again.
                    self.stats.pool_full += 1;
                    if let Some(rest) = self.digs.remove(&c) {
                        d.slots.extend(rest.slots);
                    }
                    self.digs.insert(c, d);
                    self.touched.insert(c);
                    self.listed.insert(c, faces);
                    break;
                }
                Err(e) => return Err(e),
            };
            if split {
                self.listed.insert(c, faces.clone());
            }
            self.job_coarse.insert(c, store.is_coarse(coord));
            left += samples;
            first += usize::from(d.near);
            jobs.push(Job {
                coord,
                faces,
                first_word,
                kept_bricks: kept,
                samples,
                brick_samples: own_count,
                key_only: false,
                tier: 0,
            });
            self.dig_open.insert(c, d);
        }
        // Pushed at the front in reverse, so the pending jobs start in `order`.
        for job in jobs.into_iter().rev() {
            self.lp.push_front(job);
        }
        self.open_now(run, store, first);
        let dig_open = &self.dig_open;
        self.lp.promote(|c| dig_open.contains_key(&c.0));
        self.lp
            .promote(|c| dig_open.get(&c.0).is_some_and(|d| d.near));
        Ok(())
    }

    /// Opens the first `n` pending jobs (the ring-1 jobs [`BakeQueue::open_digs`] pushed) into
    /// table slots now: where too few are free, the last open jobs that are neither a dig's nor
    /// key-only are withdrawn to the candidates, those with no rounds in flight first (a job's
    /// slot with rounds in flight frees once they are read back). A job not opened for want of
    /// a slot stays at the front of the pending ones, which the next submit opens first.
    fn open_now(&mut self, run: &BakeRun<'_>, store: &mut Store, n: usize) {
        if n == 0 {
            return;
        }
        let want = n.saturating_sub(self.lp.free.len());
        if want > 0 {
            let (dig_open, keys) = (&self.dig_open, &self.keys.open);
            let mut out: Vec<(bool, usize, [i32; 3])> = self
                .lp
                .open
                .iter()
                .enumerate()
                .filter(|(_, j)| !dig_open.contains_key(&j.coord.0) && !keys.contains(&j.coord.0))
                .map(|(k, j)| {
                    (
                        j.states.iter().any(|s| s.queued > 0),
                        usize::MAX - k,
                        j.coord.0,
                    )
                })
                .collect();
            out.sort_unstable();
            let chunks: std::collections::HashSet<[i32; 3]> =
                out.iter().take(want).map(|o| o.2).collect();
            self.withdraw_at(store, &chunks);
        }
        let settings = self.settings.clone();
        let ctx = JobCtx {
            run,
            settings: &settings,
            round: settings.round.max(1),
            rounds_cap: 0,
        };
        for _ in 0..n {
            let Some(slot) = self.lp.free.pop() else {
                break;
            };
            let Some(job) = self.lp.pending.pop_front() else {
                self.lp.free.push(slot);
                break;
            };
            // A job with no live face closes in this step's submit, its block to the sink.
            self.lp.open.push(ctx.open(job, slot));
        }
    }

    /// Withdraws the pending and open jobs in `chunks` (`JobLoop::withdraw`): each place is
    /// released and the job's books dropped; a dig's bricks (and which of them are ring 1)
    /// rejoin its chunk's waiting digs; any other job's chunk keeps the block it had and is a
    /// candidate again.
    pub(in crate::bake) fn withdraw_at(
        &mut self,
        store: &mut Store,
        chunks: &std::collections::HashSet<[i32; 3]>,
    ) {
        if chunks.is_empty() {
            return;
        }
        let record = self.record_words();
        let mut candidate = false;
        for w in self.lp.withdraw(|c| chunks.contains(&c.0), record) {
            let c = w.coord.0;
            store.release(w.first_word, w.words);
            self.job_coarse.remove(&c);
            self.job_count.remove(&c);
            self.job_eye.remove(&c);
            self.job_partial.remove(&c);
            self.job_provisional.remove(&c);
            self.stale.remove(&c);
            self.stats.dig_withdrawn += 1;
            self.stats.dig_withdrawn_samples += w.drawn;
            if let Some(d) = self.dig_open.remove(&c) {
                let e = self.digs.entry(c).or_default();
                e.slots.extend(d.slots);
                e.ring1.extend(d.ring1);
                e.near |= d.near;
                e.whole |= d.whole;
            } else {
                candidate = true;
            }
            self.touched.insert(c);
        }
        // A chunk back among the candidates is counted and sorted again this step; a dig's
        // bricks wait in `digs`, which opens apart from the candidates.
        if candidate {
            self.counted_idle = false;
            self.quiet_until = self.steps;
        }
    }

    /// A brick record's words under the settings' cell.
    pub(in crate::bake) fn record_words(&self) -> usize {
        record_words(self.settings.cell)
    }

    /// Moves every fade a frame on: each record's direction values at
    /// `frame / FADE` of the way from what it showed to the store's copy; the last frame writes
    /// the copy itself.
    pub(in crate::bake) fn advance_fades(
        &mut self,
        queue: &wgpu::Queue,
        store: &mut Store,
        record: usize,
    ) {
        let mut ended = Vec::new();
        let mut shown = std::mem::take(&mut self.fade_scratch);
        for (c, f) in &mut self.fades {
            f.frame += 1;
            let Some(to) = store.words.get(c) else {
                ended.push(*c);
                continue;
            };
            // A dig re-bakes outside the queue's sink and can relist the chunk: a block at
            // another place or with another directory is not the one this fade started on, and
            // its old place may already hold another chunk's block. The dig's put showed the
            // new block, so the fade ends without a write.
            let same = store
                .blocks
                .get(c)
                .is_some_and(|&(first, _)| first == f.first)
                && to.len() == f.from.len()
                && to[..CHUNK_BRICKS] == f.from[..CHUNK_BRICKS];
            if !same {
                ended.push(*c);
                continue;
            }
            if f.frame >= f.frames {
                shown.clear();
                shown.extend_from_slice(to);
                store.show(queue, ChunkCoord(*c), f.first, &mut shown);
                ended.push(*c);
                continue;
            }
            let t = f.frame as f32 / f.frames as f32;
            fade_mix(&mut shown, &f.from, to, t, record, f.vis);
            store.show(queue, ChunkCoord(*c), f.first, &mut shown);
        }
        self.fade_scratch = shown;
        for c in ended {
            self.fades.remove(&c);
        }
    }
}
