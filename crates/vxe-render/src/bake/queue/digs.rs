//! The queue's digs: the boxes to rebake, their opening, and the fades they start.

use super::*;

impl BakeQueue {
    /// Reads the edit boxes since the last step into the waiting digs, against the store as the
    /// edits left it (no re-bake has run since): each ring chunk with a block takes its ring's
    /// bricks, merged with what a dig job running there re-bakes (that job is thrown away), and a
    /// job of any kind running in a ring chunk is thrown away. A touched brick that held light
    /// opens ring 3. Each chunk holding a touched brick is listed again at once and takes a
    /// provisional block ([`provisional_block`]): its kept bricks' records, and a brick the edit
    /// opened its neighbours' mean, so no dug face reads no value while its job waits; the job
    /// then fades from it.
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
        let in_job: std::collections::HashSet<[i32; 3]> = self.lp.chunks().map(|c| c.0).collect();
        let mut lit = false;
        let mut dug = Vec::new();
        for (c, slots) in dig_ring(&boxes, piece) {
            let near: Vec<u16> = slots.iter().copied().filter(|&s| touches(c, s)).collect();
            if !lit && let Some(block) = store.words.get(&c) {
                lit = near.iter().any(|&s| brick_lit(block, s, record));
            }
            if in_job.contains(&c) {
                self.stale.insert(c);
            }
            if !store.blocks.contains_key(&c) {
                continue;
            }
            let e = self.digs.entry(c).or_default();
            self.touched.insert(c);
            if let Some(open) = self.dig_open.get(&c) {
                e.slots.extend(open.slots.iter().copied());
                e.near |= open.near;
            }
            e.slots.extend(slots);
            e.near |= !near.is_empty();
            if !near.is_empty() {
                dug.push((c, near));
            }
        }
        let classes = self.classes(run.world.materials());
        // A `B` press touches tens of chunks: listed across threads, as the pre-roll's are.
        let threads = std::thread::available_parallelism()
            .map_or(4, std::num::NonZero::get)
            .clamp(1, 16);
        let per = dug.len().div_ceil(threads).max(1);
        let this = &*self;
        let listed: Vec<ChunkFaces> = std::thread::scope(|scope| {
            let handles: Vec<_> = dug
                .chunks(per)
                .map(|part| {
                    let classes = &classes;
                    scope.spawn(move || {
                        part.iter()
                            .map(|(c, _)| this.list(run.world, ChunkCoord(*c), classes).0)
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|h| h.join().expect("a listing thread panicked"))
                .collect()
        });
        for ((c, near), faces) in dug.into_iter().zip(listed) {
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
                self.faceless.insert(c);
                self.touched.insert(c);
                self.digs.remove(&c);
                continue;
            }
            let block = provisional_block(store, c, &faces, record, &near);
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

    /// Opens jobs for the waiting dig chunks not in a job, ahead of every pending job, while the
    /// loop holds under two slices of `cap` samples (one at least): ring-1 chunks first, then by
    /// distance from the camera's chunk. Each is listed again on this thread and keeps every
    /// brick record outside its dig bricks; a chunk left with no face drops its block.
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
        let mut left: u64 = self.lp.dig_samples_left(&self.dig_open);
        let classes = self.classes(run.world.materials());
        let record = self.record_words();
        let brick_samples = self.settings.brick_samples;
        let mut jobs = Vec::new();
        for (_, _, c) in order {
            if left >= 2 * cap && !jobs.is_empty() {
                break;
            }
            self.touched.insert(c);
            let Some(d) = self.digs.remove(&c) else {
                continue;
            };
            let coord = ChunkCoord(c);
            let Some(block) = store.words.get(&c) else {
                continue;
            };
            let kept: KeptBricks = records_of(block, record)
                .into_iter()
                .filter(|(slot, _)| !d.slots.contains(slot))
                .collect();
            let faces = match self.listed.remove(&c) {
                Some(f) => f,
                None => self.list(run.world, coord, &classes).0,
            };
            if let Some(m) = self.prepared.remove(&c) {
                self.prepared_bytes -= m.masks.bytes();
            }
            if faces.faces() == 0 {
                store.remove(run.queue, coord);
                self.fades.remove(&c);
                self.faceless.insert(c);
                self.touched.insert(c);
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
                    // The dig waits for a place, listed: no later dig opens ahead of it.
                    self.stats.pool_full += 1;
                    self.digs.insert(c, d);
                    self.touched.insert(c);
                    self.listed.insert(c, faces);
                    break;
                }
                Err(e) => return Err(e),
            };
            self.job_coarse.insert(c, store.is_coarse(coord));
            left += samples;
            jobs.push(Job {
                coord,
                faces,
                first_word,
                kept_bricks: kept,
                samples,
                brick_samples: None,
            });
            self.dig_open.insert(c, d);
        }
        // Pushed at the front in reverse, so the pending jobs start in `order`.
        for job in jobs.into_iter().rev() {
            self.lp.push_front(job);
        }
        let dig_open = &self.dig_open;
        self.lp.promote(|c| dig_open.contains_key(&c.0));
        Ok(())
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
            fade_mix(&mut shown, &f.from, to, t, record);
            store.show(queue, ChunkCoord(*c), f.first, &mut shown);
        }
        self.fade_scratch = shown;
        for c in ended {
            self.fades.remove(&c);
        }
    }
}
