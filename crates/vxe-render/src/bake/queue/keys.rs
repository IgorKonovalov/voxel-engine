//! The queue's key-only re-bakes (ADR-0079): a block whose key visibilities were baked under a
//! key light that has turned since re-draws its key rays alone, its other words kept.

use super::*;

/// Degrees the key light turns between two scans of the blocks' sun stamps: a scan reads every
/// block, so a light drifting a little each frame is read about once a quarter degree.
const KEY_SCAN_DEG: f32 = 0.25;

/// Key-only jobs a step opens at most: each lists its chunk again on this thread.
pub(crate) const KEY_OPENS: usize = 16;

/// The key-only re-bakes' books.
#[derive(Default)]
pub(in crate::bake) struct KeyJobs {
    /// The key light's direction at the last scan; `None` before one or with no key light.
    scanned: Option<[f32; 3]>,
    /// Chunks whose block waits for its key-only job.
    waiting: std::collections::BTreeSet<[i32; 3]>,
    /// Chunks whose key-only job is pending or open.
    pub(super) open: std::collections::HashMap<[i32; 3], ()>,
}

impl KeyJobs {
    /// Whether `c` waits for a key-only job or has one pending or open.
    pub(super) fn holds(&self, c: &[i32; 3]) -> bool {
        self.waiting.contains(c) || self.open.contains_key(c)
    }

    /// Forgets the waiting chunks `held` does not hold (the mirror no longer holds them full);
    /// a job of such a chunk ends in the sink, which drops its block.
    pub(super) fn retain(&mut self, held: impl Fn(&[i32; 3]) -> bool) {
        self.waiting.retain(held);
    }

    /// Chunks waiting or in a key-only job.
    pub(super) fn count(&self) -> u64 {
        (self.waiting.len() + self.open.len()) as u64
    }
}

/// The angle in radians between unit directions `a` and `b`.
fn angle(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] * b[0] + a[1] * b[1] + a[2] * b[2])
        .clamp(-1.0, 1.0)
        .acos()
}

/// Whether a block whose sun stamp word is `stamp` ([`KEY_SUN_AT`]) re-bakes its key words
/// under the key light at `dir` of angular radius `radius`, at a step of `step` radians: the
/// light has turned from the stamp by more than both, or the block was baked with none.
pub(crate) fn key_turned(stamp: u32, dir: [f32; 3], radius: f32, step: f32) -> bool {
    unpack_direction(stamp).is_none_or(|d| angle(d, dir) > radius.max(step))
}

impl BakeQueue {
    /// Marks every full block whose key visibilities were baked under a key light the current
    /// one has turned past ([`key_turned`] at [`BakeQueue::key_step`]) as waiting for a key-only
    /// job, read whenever the light has moved [`KEY_SCAN_DEG`] since the last scan. With no
    /// key light above the horizon nothing waits.
    pub(in crate::bake) fn mark_keys(&mut self, run: &BakeRun<'_>, store: &Store) {
        let Some((dir, radius)) = run.key else {
            self.keys.scanned = None;
            self.keys.waiting.clear();
            return;
        };
        if self
            .keys
            .scanned
            .is_some_and(|s| angle(s, dir) <= KEY_SCAN_DEG.to_radians())
        {
            return;
        }
        self.keys.scanned = Some(dir);
        let step = self.key_step.max(0.0).to_radians();
        for (c, block) in &store.words {
            if self.keys.open.contains_key(c)
                || store.is_coarse(ChunkCoord(*c))
                || !key_turned(block[KEY_SUN_AT as usize], dir, radius, step)
            {
                continue;
            }
            self.keys.waiting.insert(*c);
        }
    }

    /// Opens key-only jobs for the waiting chunks not in a job and with no dig waiting, in the
    /// view's order (ADR-0048's score of a chunk with a block), at most [`KEY_OPENS`] a step
    /// and while the key-only jobs pending or open hold under one slice of `cap` samples. Each
    /// is listed again and keeps every brick record; a chunk whose block is gone, coarse, or
    /// re-baked under the current light since it was marked leaves the wait, and one whose
    /// listing no longer matches its block's bricks (an edit its dig has not re-baked) is left
    /// to the dig. Pushed at the front: [`BakeQueue::open_digs`] runs after and goes ahead.
    pub(in crate::bake) fn open_keys(
        &mut self,
        run: &BakeRun<'_>,
        store: &mut Store,
        view: &QueueView<'_>,
        cap: u64,
    ) -> Result<(), BakeError> {
        let Some((dir, radius)) = run.key else {
            return Ok(());
        };
        if self.keys.waiting.is_empty() {
            return Ok(());
        }
        let mut left = self.lp.dig_samples_left(&self.keys.open);
        if left >= cap {
            return Ok(());
        }
        let step = self.key_step.max(0.0).to_radians();
        let in_job: std::collections::HashSet<[i32; 3]> = self.lp.chunks().map(|c| c.0).collect();
        let (eye, order, near) = (self.eye, self.order, self.near);
        let mut gone = Vec::new();
        // Each candidate's sort key, which ends in its coordinate.
        let mut cands: Vec<(u8, u64, i32, [i32; 3])> = Vec::new();
        for c in &self.keys.waiting {
            let current = store.words.get(c).is_some_and(|b| {
                !store.is_coarse(ChunkCoord(*c))
                    && key_turned(b[KEY_SUN_AT as usize], dir, radius, step)
            });
            if !current {
                gone.push(*c);
            } else if !in_job.contains(c) && !self.digs.contains_key(c) {
                cands.push(cand_of(*c, 1, false, eye, view, order, near).sort_key());
            }
        }
        for c in gone {
            self.keys.waiting.remove(&c);
        }
        if cands.len() > KEY_OPENS {
            cands.select_nth_unstable(KEY_OPENS);
            cands.truncate(KEY_OPENS);
        }
        cands.sort_unstable();
        let classes = self.classes(run.world.materials());
        let record = self.record_words();
        let mut jobs = Vec::new();
        for (_, _, _, c) in cands {
            if left >= cap && !jobs.is_empty() {
                break;
            }
            self.keys.waiting.remove(&c);
            let coord = ChunkCoord(c);
            let Some(block) = store.words.get(&c) else {
                continue;
            };
            let kept: KeptBricks = records_of(block, record).into_iter().collect();
            let faces = self.list(run.world, coord, &classes).0;
            if faces.bricks.len() != kept.len()
                || faces
                    .bricks
                    .iter()
                    .any(|(slot, _)| !kept.contains_key(slot))
            {
                continue;
            }
            let samples: u64 = faces
                .bricks
                .iter()
                .map(|(_, keys)| {
                    let mut dirs = [false; 6];
                    for k in keys {
                        dirs[usize::from(k & 7)] = true;
                    }
                    dirs.iter().filter(|d| **d).count() as u64 * u64::from(KEY_BAKE_RAYS)
                })
                .sum();
            let first_word = match store.alloc(layout(&faces, record).1) {
                Ok(first) => first,
                Err(BakeError::PoolFull { .. }) => {
                    self.stats.pool_full += 1;
                    self.keys.waiting.insert(c);
                    break;
                }
                Err(e) => return Err(e),
            };
            left += samples;
            jobs.push(Job {
                coord,
                faces,
                first_word,
                kept_bricks: kept,
                samples,
                brick_samples: None,
                key_only: true,
            });
            self.keys.open.insert(c, ());
        }
        for job in jobs.into_iter().rev() {
            self.lp.push_front(job);
        }
        Ok(())
    }
}
