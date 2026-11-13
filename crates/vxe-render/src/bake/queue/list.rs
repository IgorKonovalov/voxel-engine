//! The queue's candidate listing: the batches it pushes and the starved ones it notes.

use super::*;

impl BakeQueue {
    /// An upper bound on the samples of `c`'s job before it is listed: its brick directions'
    /// bound from its prepared masks ([`ChunkMasks::dirs_bound`]), or six for every brick of
    /// the chunk where none arrived, at the count [`BakeQueue::push_listed`] would give it.
    pub(in crate::bake) fn listing_bound(&self, c: [i32; 3], class: u8, eye: [i32; 3]) -> u64 {
        let coarse = class == 0 && self.coarse_samples > 0 && !self.full_first_at(c, eye);
        let brick_samples = if coarse {
            self.coarse_samples
        } else {
            self.settings.brick_samples
        };
        let dirs = self
            .prepared
            .get(&c)
            .map_or(CHUNK_BRICKS as u64 * 6, |m| m.masks.dirs_bound());
        dirs * u64::from(brick_samples)
    }

    /// Lists `batch`'s chunks (across scoped threads when there are several) and pushes their
    /// jobs in the batch's order ([`BakeQueue::push_listed`]); a chunk the pool refuses waits.
    /// Empties `batch`.
    pub(in crate::bake) fn push_batch(
        &mut self,
        run: &BakeRun<'_>,
        store: &mut Store,
        eye: [i32; 3],
        classes: &[u8],
        batch: &mut Vec<(u8, [i32; 3])>,
    ) -> Result<Pushed, BakeError> {
        let listed: Vec<(ChunkFaces, bool)> = if batch.len() == 1 {
            vec![self.list(run.world, ChunkCoord(batch[0].1), classes)]
        } else {
            let threads = std::thread::available_parallelism()
                .map_or(4, std::num::NonZero::get)
                .clamp(1, 16);
            let per = batch.len().div_ceil(threads);
            let this = &*self;
            std::thread::scope(|scope| {
                let handles: Vec<_> = batch
                    .chunks(per)
                    .map(|part| {
                        scope.spawn(move || {
                            part.iter()
                                .map(|&(_, c)| this.list(run.world, ChunkCoord(c), classes))
                                .collect::<Vec<_>>()
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .flat_map(|h| h.join().expect("a listing thread panicked"))
                    .collect()
            })
        };
        let mut out = Pushed::default();
        for ((class, c), l) in batch.drain(..).zip(listed) {
            match self.push_listed(store, eye, class, ChunkCoord(c), l) {
                Ok(samples) => {
                    out.samples += samples;
                    out.full_blank |= class == 0
                        && samples > 0
                        && self.job_coarse.get(&c).is_some_and(|coarse| !coarse);
                }
                // A chunk the pool refuses waits, and the opening goes on to the next one: a
                // smaller block (a refinement) may fit, and what is in flight is still read
                // back and faded, so an eviction frees room.
                Err(BakeError::PoolFull { .. }) => {
                    self.stats.pool_full += 1;
                    out.refused0 += usize::from(class == 0);
                }
                Err(e) => return Err(e),
            }
        }
        Ok(out)
    }

    /// After a sliced opening that pushed a full job on a chunk with no block: counts it in
    /// [`QueueStats::starved_openings`] if, among up to `checks` candidates of the order from
    /// `next` not found blocked in the last [`QUEUE_RECHECK`] steps, one with no block outside
    /// [`BakeQueue::full_first`] is ready and was left unpushed. Nothing with no coarse pass,
    /// or with every blank chunk's first pass coarse.
    pub(in crate::bake) fn note_starved(
        &mut self,
        run: &BakeRun<'_>,
        view: &QueueView<'_>,
        eye: [i32; 3],
        mut next: usize,
        checks: usize,
    ) {
        if self.coarse_samples == 0 || self.full_first.is_none() {
            return;
        }
        let lr = run.gpu.light_radius();
        let steps = self.steps;
        let mut looked = 0;
        while let Some(Cand {
            class, coarse, c, ..
        }) = self.ordered.get(next)
        {
            next += 1;
            if looked >= checks {
                break;
            }
            if class != 0
                || self.pool.get(&c) != Some(&(class, coarse))
                || self.full_first_at(c, eye)
                || self
                    .blocked
                    .get(&c)
                    .is_some_and(|&at| steps - at < QUEUE_RECHECK)
            {
                continue;
            }
            looked += 1;
            let coord = ChunkCoord(c);
            if self.bakeable(run, coord, lr, view, eye).is_some()
                && run.gpu.lights_ready_near(coord, lr)
            {
                self.stats.starved_openings += 1;
                return;
            }
        }
    }

    /// Opens `coord`'s job from its listed faces (and whether its masks were prepared): its
    /// place allocated, its count the coarse pass's for a chunk of `class` 0 (no block) outside
    /// [`BakeQueue::full_first`] of `eye` (chunks), else the settings'. A chunk with no face is marked
    /// faceless instead. The job's samples at most (0 for none).
    pub(in crate::bake) fn push_listed(
        &mut self,
        store: &mut Store,
        eye: [i32; 3],
        class: u8,
        coord: ChunkCoord,
        (faces, prepared): (ChunkFaces, bool),
    ) -> Result<u64, BakeError> {
        let c = coord.0;
        self.stats.prepared += u64::from(prepared);
        // Faceless or in a job, it is no candidate either way; a refused place keeps it one.
        let candidacy = self.pool.remove(&c);
        if let Some(m) = self.prepared.remove(&c) {
            self.prepared_bytes -= m.masks.bytes();
        }
        let partial = self.partial_checked.remove(&c);
        if faces.faces() == 0 {
            self.reach.remove(&c);
            self.partial_due.remove(&c);
            self.partial.remove(&c);
            self.faceless.insert(c);
            return Ok(0);
        }
        let words = layout(&faces, self.record_words()).1;
        let first_word = match store.alloc(words) {
            Ok(w) => w,
            Err(e) => {
                if let Some(v) = candidacy {
                    self.pool.insert(c, v);
                }
                return Err(e);
            }
        };
        self.reach.remove(&c);
        self.partial_due.remove(&c);
        if partial {
            self.job_partial.insert(c);
            self.stats.partial_opened += 1;
        } else {
            self.job_partial.remove(&c);
        }
        let coarse = class == 0 && self.coarse_samples > 0 && !self.full_first_at(c, eye);
        let brick_samples = if coarse {
            self.coarse_samples
        } else {
            self.settings.brick_samples
        };
        let samples = faces
            .bricks
            .iter()
            .map(|(_, keys)| {
                let mut dirs = [false; 6];
                for k in keys {
                    dirs[usize::from(k & 7)] = true;
                }
                dirs.iter().filter(|d| **d).count() as u64 * u64::from(brick_samples)
            })
            .sum();
        self.lp.push(Job {
            coord,
            faces,
            first_word,
            kept_bricks: KeptBricks::new(),
            samples,
            brick_samples: coarse.then_some(brick_samples),
        });
        self.job_coarse.insert(c, coarse);
        self.job_eye.insert(c, self.eye_moves);
        self.stats.coarse_opened += u64::from(coarse);
        Ok(samples)
    }

    /// Whether a chunk with no block at `c` takes the full count directly: within
    /// [`BakeQueue::full_first`] of `eye` (Chebyshev, chunks).
    pub(in crate::bake) fn full_first_at(&self, c: [i32; 3], eye: [i32; 3]) -> bool {
        self.full_first
            .is_some_and(|r| (0..3).map(|k| (c[k] - eye[k]).abs()).max().unwrap_or(0) <= r)
    }

    /// `coord`'s faces: from its prepared masks and its neighbours' where they are held, the
    /// rest decoded from the world's chunks; and whether its own masks were prepared.
    pub(in crate::bake) fn list(
        &self,
        world: &World,
        coord: ChunkCoord,
        classes: &[u8],
    ) -> (ChunkFaces, bool) {
        let Some(own) = self.prepared.get(&coord.0) else {
            return (
                list_world_chunk(world, coord, classes).unwrap_or_default(),
                false,
            );
        };
        let store = world.store();
        let neighbour = |face: usize| {
            let mut c = coord.0;
            c[face / 2] += if face % 2 == 1 { 1 } else { -1 };
            c
        };
        let held: [_; 6] = std::array::from_fn(|f| {
            let c = neighbour(f);
            match self.prepared.get(&c) {
                Some(m) => Some(Err(m.clone())),
                None => store.get(ChunkCoord(c)).map(Ok),
            }
        });
        let locks: Vec<_> = held
            .iter()
            .map(|h| match h {
                Some(Ok(c)) => Some(Ok(c
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner))),
                Some(Err(m)) => Some(Err(m.clone())),
                None => None,
            })
            .collect();
        let faces = list_faces_by(&|slot| own.masks.get(slot), &|face, slot| {
            locks[face].as_ref().map(|n| match n {
                Ok(c) => class_masks(c.brick(slot), classes),
                Err(m) => m.masks.get(slot),
            })
        });
        (faces, true)
    }
}
