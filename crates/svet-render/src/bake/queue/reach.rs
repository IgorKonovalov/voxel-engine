//! The reach's verdicts (ADR-0046's reach rule, ADR-0080 rule 4 by tier): the walk over a
//! chunk's light reach, its cache, and the re-queue of partial records when their reach fills.

use super::*;

/// `coord`'s reach as [`reach_ready`] reads it: [`Reach::Waiting`] at the first chunk within it
/// waiting to upload, else [`Reach::Partial`] if a chunk within it is not resident full (a
/// streamed world only), else [`Reach::Ready`]. With `first`, the walk stops at the first chunk
/// that is not ready either way and reads it as [`Reach::Partial`]: a caller that asks only
/// whether the reach is ready ([`ReachMode::Full`], the census) walks no further than that.
pub(super) fn reach_of(
    run: &BakeRun<'_>,
    coord: ChunkCoord,
    lr: i32,
    resident: Option<&dyn Fn(ChunkCoord) -> bool>,
    first: bool,
) -> Reach {
    let gpu = run.mirror;
    if resident.is_none() && gpu.pending_chunks() == 0 {
        return Reach::Ready;
    }
    reach_by(
        coord,
        lr,
        |n| gpu.is_pending(n),
        |n| resident.is_none_or(|res| res(n) && gpu.is_mirrored(n) && !gpu.is_lod(n)),
        first,
    )
}

/// [`reach_of`]'s walk over `coord`'s reach: [`Reach::Waiting`] at the first chunk `pending`,
/// else [`Reach::Partial`] if a chunk is not `full` (with `first`, at the first such chunk),
/// else [`Reach::Ready`].
pub(super) fn reach_by(
    coord: ChunkCoord,
    lr: i32,
    pending: impl Fn(ChunkCoord) -> bool,
    full: impl Fn(ChunkCoord) -> bool,
    first: bool,
) -> Reach {
    let r = lr + 1;
    let mut partial = false;
    for z in -r..=r {
        for y in -r..=r {
            for x in -r..=r {
                if !in_reach([x, y, z], lr) {
                    continue;
                }
                let n = ChunkCoord([coord.0[0] + x, coord.0[1] + y, coord.0[2] + z]);
                if pending(n) {
                    return Reach::Waiting;
                }
                if !full(n) {
                    if first {
                        return Reach::Partial;
                    }
                    partial = true;
                }
            }
        }
    }
    if partial {
        Reach::Partial
    } else {
        Reach::Ready
    }
}

/// The reach's verdict on a chunk under its band's mode ([`ReachMode::for_band`]): `Some(false)`
/// when its `reach` is ready, `Some(true)` (bake from the chunks resident, flagged partial) when
/// the mode is partial, nothing in the reach waits to upload and the reach `crosses` the
/// streamer's full radius ([`reach_crosses`]); else `None` (it waits).
pub(super) fn reach_verdict(reach: Reach, mode: ReachMode, crosses: bool) -> Option<bool> {
    match reach {
        Reach::Ready => Some(false),
        Reach::Partial if mode == ReachMode::Partial && crosses => Some(true),
        _ => None,
    }
}

impl BakeQueue {
    /// [`reach_ready`] through the cache of verdicts; a bounded world with nothing waiting to
    /// upload is ready without a walk. With `first` the walk may stop at the first chunk not
    /// resident full; without it a cached [`Reach::Partial`] from such a walk is walked again
    /// whole. A miss counts in [`QueueStats::reach_checks`].
    pub(super) fn reach_cached(
        &mut self,
        run: &BakeRun<'_>,
        coord: ChunkCoord,
        lr: i32,
        resident: Option<&dyn Fn(ChunkCoord) -> bool>,
        first: bool,
    ) -> Reach {
        if resident.is_none() && run.mirror.pending_chunks() == 0 {
            return Reach::Ready;
        }
        if let Some(&(v, walked_first)) = self.reach_cache.get(&coord.0)
            && (first || !walked_first || v != Reach::Partial)
        {
            // A streamer's residency can move a frame ahead of the mirror; a bounded world's
            // verdict reads the mirror alone.
            debug_assert!(
                resident.is_some() || v == reach_of(run, coord, lr, resident, walked_first),
                "the cached reach of {coord:?}"
            );
            return v;
        }
        self.stats.reach_checks += 1;
        let v = reach_of(run, coord, lr, resident, first);
        self.reach_cache.insert(coord.0, (v, first));
        v
    }

    /// Whether a chunk whose reach is not resident full bakes from the chunks that are.
    pub fn reach_mode(&self) -> ReachMode {
        self.reach_mode
    }

    /// Sets [`BakeQueue::reach_mode`]. A change drops the cached reach verdicts: under
    /// [`ReachMode::Full`] a walk stops at the first chunk not resident full, so its
    /// [`Reach::Partial`] does not say that nothing in the reach waits to upload.
    pub fn set_reach_mode(&mut self, mode: ReachMode) {
        if mode != self.reach_mode {
            self.reach_mode = mode;
            self.reach_cache.clear();
        }
    }

    /// [`BakeQueue::bakeable`]'s reach verdict, before the columns.
    pub(super) fn bakeable_reach(
        &mut self,
        run: &BakeRun<'_>,
        coord: ChunkCoord,
        lr: i32,
        view: &QueueView,
        eye: [i32; 3],
    ) -> Option<bool> {
        // ADR-0080 rule 4: a mid or far chunk bakes from the chunks resident; a near one keeps
        // the queue's mode.
        let mode = self
            .reach_mode
            .for_band(self.band_of(coord.0) == Band::Near);
        let reach = self.reach_cached(run, coord, lr, view.resident_fn(), mode == ReachMode::Full);
        let crosses = view
            .full_radius
            .is_some_and(|r| reach_crosses(coord, eye, lr, r));
        reach_verdict(reach, mode, crosses)
    }

    /// Re-queues as a refinement each chunk flagged partial (its block, or its job's) within
    /// whose reach a chunk of `changed` is now held full: a chunk arriving, or a held one
    /// changing. `None` (the changes went untracked) re-queues every flagged chunk. Nothing to
    /// do under [`ReachMode::Full`], which flags none.
    pub(super) fn requeue_partial(&mut self, gpu: &Shadow, changed: Option<&[ChunkCoord]>) {
        if self.partial.is_empty() && self.job_partial.is_empty() {
            return;
        }
        let held: Option<ChunkSet> = changed.map(|ch| {
            ch.iter()
                .filter(|c| gpu.is_mirrored(**c) && !gpu.is_lod(**c))
                .map(|c| c.0)
                .collect()
        });
        self.requeue_arrived(held.as_ref(), gpu.light_radius());
    }

    /// [`BakeQueue::requeue_partial`] over `held`, the changed chunks now held full (`None`:
    /// untracked, every flagged chunk). A flagged chunk whose record (or job) is below the near
    /// tier's count is a mid or far record, refined by its band's promotion, not by its reach
    /// filling (ADR-0080 rule 4), and is not re-queued; a near one (a dig's ring) is.
    pub(super) fn requeue_arrived(&mut self, held: Option<&ChunkSet>, lr: i32) {
        let due: Vec<[i32; 3]> = match held {
            None => self
                .partial
                .iter()
                .chain(self.job_partial.iter())
                .copied()
                .collect(),
            Some(held) => {
                if held.is_empty() {
                    return;
                }
                let flagged =
                    |c: &[i32; 3]| self.partial.contains(c) || self.job_partial.contains(c);
                let mut due = Vec::new();
                // Each side walks its reach over the other's set: the smaller side walks.
                if self.partial.len() + self.job_partial.len() <= held.len() {
                    for p in self.partial.iter().chain(self.job_partial.iter()) {
                        let near = reach_offsets(lr)
                            .any(|o| held.contains(&[p[0] + o[0], p[1] + o[1], p[2] + o[2]]));
                        if near {
                            due.push(*p);
                        }
                    }
                } else {
                    for n in held {
                        for o in reach_offsets(lr) {
                            let p = [n[0] + o[0], n[1] + o[1], n[2] + o[2]];
                            if flagged(&p) {
                                due.push(p);
                            }
                        }
                    }
                }
                due
            }
        };
        let near = self.near_count();
        for p in due {
            let count = self.job_count.get(&p).or_else(|| self.block_count.get(&p));
            if count.is_some_and(|&n| n < near) {
                continue;
            }
            if self.partial_due.insert(p) {
                self.stats.partial_requeued += 1;
                self.touched.insert(p);
                self.counted_idle = false;
                self.quiet_until = self.steps;
            }
        }
    }

    /// Drops the cached reach verdicts within `lr + 1` chunks (Chebyshev) of a chunk of
    /// `changed`; too many pairs to compare drop them all.
    pub(super) fn drop_reach(&mut self, changed: &[ChunkCoord], lr: i32) {
        if changed.is_empty() || self.reach_cache.is_empty() {
            return;
        }
        if changed.len().saturating_mul(self.reach_cache.len()) > 1 << 22 {
            self.reach_cache.clear();
            return;
        }
        let r = lr + 1;
        // The verdicts within `r` (Chebyshev) of a changed chunk go. Removing each changed
        // chunk's neighbourhood by key costs `(2 r + 1)^3` lookups a change; a scan of the cache
        // costs its length times the changes. In a flight a few chunks change a step against
        // thousands of cached verdicts, so the keys are removed directly unless the scan is
        // smaller. Either way the same verdicts go.
        let side = (2 * r + 1) as usize;
        if changed.len().saturating_mul(side * side * side) < self.reach_cache.len() {
            for n in changed {
                for dz in -r..=r {
                    for dy in -r..=r {
                        for dx in -r..=r {
                            self.reach_cache
                                .remove(&[n.0[0] + dx, n.0[1] + dy, n.0[2] + dz]);
                        }
                    }
                }
            }
            return;
        }
        self.reach_cache.retain(|c, _| {
            !changed
                .iter()
                .any(|n| (0..3).all(|k| (n.0[k] - c[k]).abs() <= r))
        });
    }
}
