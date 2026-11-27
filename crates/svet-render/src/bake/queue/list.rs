//! The queue's candidate listing: the batches it pushes and the starved ones it notes.

use super::*;

impl BakeQueue {
    /// An upper bound on the samples of `c`'s job before it is listed: its brick directions'
    /// bound from its prepared masks ([`ChunkMasks::dirs_bound`]), or six for every brick of
    /// the chunk where none arrived, at the count [`BakeQueue::push_listed`] would give it.
    pub(in crate::bake) fn listing_bound(&self, c: [i32; 3], class: u8, eye: [i32; 3]) -> u64 {
        let brick_samples = self.count_for(c, class, eye).0;
        let dirs = self
            .prepared
            .get(&c)
            .map_or(CHUNK_BRICKS as u64 * 6, |m| m.masks.dirs_bound());
        dirs * u64::from(brick_samples)
    }

    /// Whether a box waits for the bake thread in a message its step has not taken
    /// ([`BoxSignal`]); never in lockstep.
    pub(in crate::bake) fn box_waits(&self) -> bool {
        self.box_signal.as_ref().is_some_and(BoxSignal::waits)
    }

    /// Lists `batch`'s chunks (across scoped threads when there are several) and pushes their
    /// jobs in the batch's order ([`BakeQueue::push_listed`]); a chunk the pool refuses waits,
    /// and once a box waits ([`BakeQueue::box_waits`]) the rest wait. Empties `batch`.
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
            // A box waiting: the candidates listed and not pushed stay in the pool.
            if self.box_waits() {
                break;
            }
            match self.push_listed(run.mirror, store, eye, class, ChunkCoord(c), l) {
                Ok(samples) => {
                    #[cfg(test)]
                    if let Some(hook) = self.listed_hook.as_mut() {
                        hook();
                    }
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
    /// `next` not skipped as blocked ([`BlockedMarks::recent`]), one with no block outside
    /// [`BakeQueue::full_first`] is ready and was left unpushed. Nothing with no coarse pass,
    /// or with every blank chunk's first pass coarse.
    pub(in crate::bake) fn note_starved(
        &mut self,
        run: &BakeRun<'_>,
        view: &QueueView,
        eye: [i32; 3],
        mut next: usize,
        checks: usize,
    ) {
        if self.coarse_samples == 0 || self.full_first.is_none() {
            return;
        }
        let lr = run.mirror.light_radius();
        let steps = self.steps;
        let mut looked = 0;
        while let Some(Cand {
            class, coarse, c, ..
        }) = self.ordered.get(0, next, &self.pool)
        {
            next += 1;
            if looked >= checks {
                break;
            }
            if class != 0
                || self.pool.get(&c) != Some(&(class, coarse))
                || self.full_first_at(c, eye)
                || self.blocked.recent(&c, steps)
            {
                continue;
            }
            looked += 1;
            let coord = ChunkCoord(c);
            if self.bakeable(run, coord, lr, view, eye).is_ok()
                && run.mirror.lights_ready_near(coord, lr)
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
        gpu: &Shadow,
        store: &mut Store,
        eye: [i32; 3],
        class: u8,
        coord: ChunkCoord,
        (faces, prepared): (ChunkFaces, bool),
    ) -> Result<u64, BakeError> {
        let c = coord.0;
        self.stats.prepared += u64::from(prepared);
        // Faceless or in a job, it is no candidate either way; a refused place keeps it one.
        let candidacy = self.pool_remove(c);
        if let Some(m) = self.prepared.remove(&c) {
            self.prepared_bytes -= m.masks.bytes();
        }
        let partial = self.partial_checked.remove(&c);
        let provisional = self.provisional_checked.remove(&c);
        let ruled = self.ruled_checked.remove(&c);
        let missed = self.rule_missed_checked.remove(&c);
        let audit = self.audit_checked.remove(&c);
        self.audit_jobs.remove(&c);
        self.zeroed.remove(&c);
        self.zeroed_open.remove(&c);
        if faces.faces() == 0 {
            self.reach.remove(&c);
            self.partial_due.remove(&c);
            self.partial.remove(&c);
            self.provisional_due.remove(&c);
            self.provisional.remove(&c);
            self.faceless.insert(c);
            return Ok(0);
        }
        let words = layout(&faces, self.record_words()).1;
        let first_word = match store.alloc(words) {
            Ok(w) => w,
            Err(e) => {
                if let Some(v) = candidacy {
                    self.pool_insert(c, v);
                }
                return Err(e);
            }
        };
        self.reach.remove(&c);
        self.partial_due.remove(&c);
        self.provisional_due.remove(&c);
        let (count, tier) = self.count_for(c, class, eye);
        let near = self.near_count();
        if ruled {
            // The rule's zero record: no job; its block is placed at this step's submit. It is
            // what a bake at any count writes, so it holds the near tier and no band's
            // promotion refines it (`u32::MAX` is never below a band's count). It is final: read
            // on the reach as it stands, it is flagged neither partial nor provisional (its
            // placement clears both), so no catch-up or arrival re-queues it; the re-list does
            // ([`BakeQueue::relist_zeroed`]). Over a held block it lands as a class 1 job's does.
            self.job_partial.remove(&c);
            self.job_provisional.remove(&c);
            self.job_coarse.insert(c, false);
            self.job_count.insert(c, u32::MAX);
            self.ruled.push((coord, first_word, faces, 0));
            self.zeroed.insert(c);
            self.stats.zeroed += 1;
            // Its faces were listed with each neighbour the store did not hold read as rock: the
            // neighbours the shadow does not hold full are noted, and the chunk is listed again
            // when one of them is (ADR-0085).
            let open = not_held_full(gpu, coord);
            if open != 0 {
                self.zeroed_open.insert(c, open);
            }
            return Ok(0);
        }
        if partial {
            self.job_partial.insert(c);
            self.stats.partial_opened += 1;
        } else {
            self.job_partial.remove(&c);
        }
        if provisional {
            self.job_provisional.insert(c);
            self.stats.provisional_opened += 1;
        } else {
            self.job_provisional.remove(&c);
        }
        let coarse = count < near;
        let brick_samples = count;
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
        if let Some(case) = audit {
            self.audit_jobs.insert(c, (case, faces.clone()));
        }
        self.lp.push(Job {
            coord,
            faces,
            first_word,
            kept_bricks: KeptBricks::new(),
            samples,
            brick_samples: (count != self.settings.brick_samples).then_some(count),
            key_only: false,
            tier,
            class: JobClass::Other,
        });
        self.job_coarse.insert(c, coarse);
        self.job_count.insert(c, count);
        self.job_eye.insert(c, self.eye_moves);
        self.stats.coarse_opened += u64::from(coarse);
        if self.rule_reading {
            self.stats.rule_missed.note(missed, samples);
        }
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
        let (own, held) = self.listing_masks(coord);
        list_chunk(world, coord, classes, own.as_deref(), &held)
    }

    /// `coord`'s own prepared masks and its six neighbours', where the stream workers' arrived:
    /// what [`list_chunk`] reads beside the world.
    pub(in crate::bake) fn listing_masks(
        &self,
        coord: ChunkCoord,
    ) -> (Option<Arc<PreparedMasks>>, Held) {
        let own = self.prepared.get(&coord.0).cloned();
        let held = std::array::from_fn(|f| {
            own.as_ref()
                .and_then(|_| self.prepared.get(&neighbour(coord, f)).cloned())
        });
        (own, held)
    }
}

/// A chunk's six neighbours' prepared masks, by face, where they arrived.
pub(in crate::bake) type Held = [Option<Arc<PreparedMasks>>; 6];

/// The chunk across `face` of `coord` (`face / 2` the axis, odd the positive side).
pub(super) fn neighbour(coord: ChunkCoord, face: usize) -> [i32; 3] {
    let mut c = coord.0;
    c[face / 2] += if face % 2 == 1 { 1 } else { -1 };
    c
}

/// `coord`'s faces: from its prepared masks `own` and its neighbours' `held` where they are
/// given, the rest decoded from the world's chunks under their read locks; and whether its own
/// masks were prepared. A pure function of its inputs and the world, so the listing pool
/// ([`super::lister::Lister`]) runs it off the bake loop.
pub(in crate::bake) fn list_chunk(
    world: &World,
    coord: ChunkCoord,
    classes: &[u8],
    own: Option<&PreparedMasks>,
    held: &Held,
) -> (ChunkFaces, bool) {
    list_chunk_read(world, coord, classes, own, held).0
}

/// [`list_chunk`] and the neighbours it read absent (neither held nor in the store, so read as
/// rock), a bit per face: the listing pool's, whose push reads the shadow a step later
/// ([`BakeQueue::take_listed`]).
pub(in crate::bake) fn list_chunk_read(
    world: &World,
    coord: ChunkCoord,
    classes: &[u8],
    own: Option<&PreparedMasks>,
    held: &Held,
) -> ((ChunkFaces, bool), u8) {
    {
        let Some(own) = own else {
            let (faces, absent) = list_world_chunk_read(world, coord, classes).unwrap_or_default();
            return ((faces, false), absent);
        };
        let store = world.store();
        let held: [_; 6] = std::array::from_fn(|f| match &held[f] {
            Some(m) => Some(Err(m.clone())),
            None => store.get(ChunkCoord(neighbour(coord, f))).map(Ok),
        });
        let absent = (0..6).fold(0u8, |m, f| m | u8::from(held[f].is_none()) << f);
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
        ((faces, true), absent)
    }
}

/// The faces of `coord` whose neighbour `gpu` does not hold full (absent or LOD), a bit per face:
/// where a ruled record's border waits for an arrival ([`BakeQueue::zeroed_open`]).
pub(in crate::bake) fn not_held_full(gpu: &Shadow, coord: ChunkCoord) -> u8 {
    (0..6).fold(0u8, |m, f| {
        let n = ChunkCoord(neighbour(coord, f));
        if gpu.is_mirrored(n) && !gpu.is_lod(n) {
            m
        } else {
            m | 1 << f
        }
    })
}
