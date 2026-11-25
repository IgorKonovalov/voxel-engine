//! The tier by size on screen (ADR-0080 rule 2): a chunk's record is baked at its band's count,
//! the band read at the centre of the camera's chunk through the far-normal band at the view's
//! focal length ([`Bands`]). A block below its band's count is a candidate for refinement; the
//! bands are read again when the camera's chunk changes, and a pending job above its new band's
//! count is withdrawn and listed again at that count.

use super::*;
use crate::bake::record::{TIER_FAR, TIER_MID};

/// The live queue's sample tiers a brick direction, near, mid and far ([`BakeQueue::tiers`]).
pub const DEFAULT_TIERS: [u32; 3] = [QUEUE_BRICK_SAMPLES, 1024, DEFAULT_COARSE_SAMPLES];

impl BakeQueue {
    /// Chunk `c`'s band at the last step's bands, near without them.
    pub(super) fn band_of(&self, c: [i32; 3]) -> Band {
        self.bands.map_or(Band::Near, |b| b.of_chunk(c, self.eye_c))
    }

    /// The samples a brick direction `c`'s record wants: its band's tier, or the settings' count
    /// without a focal length.
    pub(super) fn want(&self, c: [i32; 3]) -> u32 {
        if self.bands.is_some() {
            self.tiers[self.band_of(c).index()]
        } else {
            self.settings.brick_samples
        }
    }

    /// The near tier's count, the settings' without a focal length.
    pub(super) fn near_count(&self) -> u32 {
        if self.bands.is_some() {
            self.tiers[0]
        } else {
            self.settings.brick_samples
        }
    }

    /// Whether `c`'s block (it holds one) is below its band's count: a refinement's candidate.
    /// A block of unknown count reads the store's coarse flag.
    pub(super) fn lesser(&self, store: &Store, c: [i32; 3]) -> bool {
        self.below(c)
            .unwrap_or_else(|| store.is_coarse(ChunkCoord(c)))
    }

    /// Whether `c`'s block of known count is below its band's count; `None` for an unknown
    /// count.
    pub(super) fn below(&self, c: [i32; 3]) -> Option<bool> {
        self.block_count.get(&c).map(|&n| n < self.want(c))
    }

    /// A job's count for `c` of `class` (0, no block): the coarse pass beyond the full-first
    /// shell where it is below the band's count, else the band's count; and the record's tier
    /// bits ([`crate::bake::record::TIER_SHIFT`]).
    pub(super) fn count_for(&self, c: [i32; 3], class: u8, eye: [i32; 3]) -> (u32, u8) {
        let want = self.want(c);
        let count = if class == 0
            && self.coarse_samples > 0
            && self.coarse_samples < want
            && !self.full_first_at(c, eye)
        {
            self.coarse_samples
        } else {
            want
        };
        let tier = if count >= self.near_count() {
            0
        } else if count <= self.tiers[2] {
            TIER_FAR
        } else {
            TIER_MID
        };
        (count, tier)
    }

    /// The step's bands from `view` (its focal length and the queue's far-normal band) at the
    /// camera's chunk `eye`: when they change, every block whose band changed is touched (its
    /// candidacy read again) and every pending job above its new band's count is withdrawn, its
    /// chunk listed again.
    pub(super) fn retier(&mut self, store: &mut Store, view: &QueueView, eye: [i32; 3]) {
        let bands = view.focal_px.map(|focal_px| Bands {
            focal_px,
            lo: self.far_normal[0],
            hi: self.far_normal[1],
        });
        let eye_c = eye.map(|v| (v * 64 + 32) as f32);
        if bands == self.bands && eye_c == self.eye_c {
            return;
        }
        let (old_bands, old_eye) = (self.bands, self.eye_c);
        let old = |c: [i32; 3]| old_bands.map_or(Band::Near, |b| b.of_chunk(c, old_eye));
        self.bands = bands;
        self.eye_c = eye_c;
        let moved: Vec<[i32; 3]> = store
            .blocks
            .keys()
            .filter(|c| old(**c) != self.band_of(**c))
            .copied()
            .collect();
        self.stats.retiered += moved.len() as u64;
        if !moved.is_empty() {
            // A block whose band changed may be a refinement's candidate: the depth is counted
            // again and the next step opens.
            self.counted_idle = false;
            self.quiet_until = self.steps;
        }
        self.touched.extend(moved);
        let pending: ChunkSet = self.lp.pending_chunks().map(|c| c.0).collect();
        let job_count = &self.job_count;
        let fell: ChunkSet = pending
            .into_iter()
            .filter(|c| {
                job_count.get(c).is_some_and(|&n| {
                    let want = bands.map_or(self.settings.brick_samples, |b| {
                        self.tiers[b.of_chunk(*c, eye_c).index()]
                    });
                    n > want
                })
            })
            .collect();
        if fell.is_empty() {
            return;
        }
        let record = self.record_words();
        for w in self.lp.withdraw(|c| fell.contains(&c.0), record) {
            let c = w.coord.0;
            store.release(w.first_word, w.words);
            self.job_coarse.remove(&c);
            self.job_count.remove(&c);
            self.job_eye.remove(&c);
            self.job_partial.remove(&c);
            self.job_provisional.remove(&c);
            self.stale.remove(&c);
            self.touched.insert(c);
            self.stats.withdrawn_band += 1;
        }
        self.counted_idle = false;
        self.quiet_until = self.steps;
    }

    /// Removes from [`BakeQueue::view_waiting`] each chunk that waits for nothing now: its record at
    /// its band's count, out of the mirror's full chunks, or known to hold no face.
    pub(super) fn land_waiting(&mut self, gpu: &Shadow, store: &Store) {
        if self.view_waiting.is_empty() {
            self.stats.drain_waiting = 0;
            return;
        }
        let landed: Vec<[i32; 3]> = self
            .view_waiting
            .iter()
            .copied()
            .filter(|&c| self.landed(gpu, store, c))
            .collect();
        self.retain_waiting(|c| landed.contains(&c));
    }

    /// Drops the chunks `landed` names from [`BakeQueue::view_waiting`].
    pub(super) fn retain_waiting(&mut self, landed: impl Fn([i32; 3]) -> bool) {
        self.view_waiting.retain(|&c| !landed(c));
        self.stats.drain_waiting = self.view_waiting.len() as u64;
    }

    /// Reads [`BakeQueue::view_waiting`] again when it is due: `force`d (an opening's census),
    /// or when the view (the eye's chunk, the forward direction, the half-angle, the near
    /// shell) or the pool changed since the last read and [`WAITING_EVERY`] steps have passed.
    /// A read walks the pool and the jobs once, scoring each by its angle term alone
    /// ([`angle_term`]), into the set's kept allocation.
    pub(super) fn refresh_waiting(&mut self, store: &Store, view: &QueueView, force: bool) {
        let eye = view.eye.map(|v| (v / 64.0).floor() as i32);
        let key = (
            eye,
            view.forward.map(f32::to_bits),
            view.half_angle.to_bits(),
            self.near,
        );
        let changed = self.waiting_stale || self.waiting_view != Some(key);
        let spaced = self
            .waiting_at
            .is_none_or(|at| self.steps >= at + WAITING_EVERY);
        if !force && !(changed && spaced) {
            return;
        }
        let t = stopwatch();
        let (forward, half, near) = (view.forward, view.half_angle, self.near);
        let in_view = |c: [i32; 3]| {
            let o = [0, 1, 2].map(|k| c[k] - eye[k]);
            angle_term(o, forward, half, near) <= 0.0
        };
        let mut set = std::mem::take(&mut self.view_waiting);
        set.clear();
        // A candidate with no block or a lesser one; a full block re-baking whole (ring 3) is
        // not waiting for its light.
        set.extend(
            self.pool
                .iter()
                .filter(|&(&c, &(has, coarse))| (has == 0 || coarse) && in_view(c))
                .map(|(&c, _)| c),
        );
        // A chunk in a job still waits for its light: one with no block or a lesser one, not
        // a dig's or a key-only re-bake's.
        for c in self.lp.chunks().map(|c| c.0) {
            if in_view(c)
                && !self.dig_open.contains_key(&c)
                && !self.keys.holds(&c)
                && (!store.blocks.contains_key(&c) || self.lesser(store, c))
            {
                set.insert(c);
            }
        }
        self.view_waiting = set;
        self.stats.drain_waiting = self.view_waiting.len() as u64;
        self.waiting_view = Some(key);
        self.waiting_at = Some(self.steps);
        self.waiting_stale = false;
        self.stats.waiting_reads += 1;
        let ms = seconds_since(t) * 1e3;
        self.stats.waiting_ms += ms;
        self.stats.waiting_max_ms = self.stats.waiting_max_ms.max(ms);
    }

    /// The step's slice in ms after a frame of `interval_s` seconds: the drain's value while a
    /// chunk of [`BakeQueue::view_waiting`] waits, its line's once none does, the budget's
    /// without a drain.
    pub(super) fn step_slice(&mut self, interval_s: f64) -> f64 {
        let waiting = !self.view_waiting.is_empty();
        self.stats.draining = self.drain.is_some() && waiting;
        match self.drain {
            Some(d) => d.slice_ms(&self.budget, interval_s, waiting),
            None => self.budget.slice_ms(interval_s),
        }
    }

    /// Whether chunk `c` waits for no record: not mirrored full, faceless, sealed, or holding
    /// a block at its band's count.
    pub(super) fn landed(&self, gpu: &Shadow, store: &Store, c: [i32; 3]) -> bool {
        let cc = ChunkCoord(c);
        !gpu.is_mirrored(cc)
            || gpu.is_lod(cc)
            || self.faceless.contains(&c)
            || self.sealed.holds(c)
            || (store.blocks.contains_key(&c) && !self.lesser(store, c))
    }
}
