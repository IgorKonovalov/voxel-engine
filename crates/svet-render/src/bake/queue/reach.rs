//! The reach's verdicts (ADR-0046's reach rule, ADR-0080 rule 4 by tier): the walk over a
//! chunk's light reach, its cache, the re-queue of partial records when their reach fills, and
//! the blocked marks a refused candidate carries until what refused it changes.

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
    reach_walk_of(run, coord, lr, resident, first).0
}

/// [`reach_of`] with the chunk its verdict stops at: the chunk waiting to upload under
/// [`Reach::Waiting`], the first chunk not resident full under [`Reach::Partial`]; `None` when
/// the reach is ready.
fn reach_walk_of(
    run: &BakeRun<'_>,
    coord: ChunkCoord,
    lr: i32,
    resident: Option<&dyn Fn(ChunkCoord) -> bool>,
    first: bool,
) -> (Reach, Option<ChunkCoord>) {
    let gpu = run.mirror;
    if resident.is_none() && gpu.pending_chunks() == 0 {
        return (Reach::Ready, None);
    }
    reach_walk(
        coord,
        lr,
        |n| gpu.is_pending(n),
        |n| resident.is_none_or(|res| res(n) && gpu.is_mirrored(n) && !gpu.is_lod(n)),
        first,
    )
}

/// [`reach_of`]'s walk over `coord`'s reach: [`Reach::Waiting`] at the first chunk `pending`,
/// else [`Reach::Partial`] if a chunk is not `full` (with `first`, at the first such chunk),
/// else [`Reach::Ready`]; with the chunk the verdict stops at ([`reach_walk_of`]).
pub(super) fn reach_walk(
    coord: ChunkCoord,
    lr: i32,
    pending: impl Fn(ChunkCoord) -> bool,
    full: impl Fn(ChunkCoord) -> bool,
    first: bool,
) -> (Reach, Option<ChunkCoord>) {
    let r = lr + 1;
    let mut partial = None;
    for z in -r..=r {
        for y in -r..=r {
            for x in -r..=r {
                if !in_reach([x, y, z], lr) {
                    continue;
                }
                let n = ChunkCoord([coord.0[0] + x, coord.0[1] + y, coord.0[2] + z]);
                if pending(n) {
                    return (Reach::Waiting, Some(n));
                }
                if !full(n) {
                    if first {
                        return (Reach::Partial, Some(n));
                    }
                    partial = partial.or(Some(n));
                }
            }
        }
    }
    match partial {
        Some(n) => (Reach::Partial, Some(n)),
        None => (Reach::Ready, None),
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
    ) -> (Reach, Option<ChunkCoord>) {
        if resident.is_none() && run.mirror.pending_chunks() == 0 {
            return (Reach::Ready, None);
        }
        if let Some(&(v, walked_first, at)) = self.reach_cache.get(&coord.0)
            && (first || !walked_first || v != Reach::Partial)
        {
            // A streamer's residency can move a frame ahead of the mirror; a bounded world's
            // verdict reads the mirror alone.
            debug_assert!(
                resident.is_some() || v == reach_of(run, coord, lr, resident, walked_first),
                "the cached reach of {coord:?}"
            );
            return (v, at);
        }
        self.stats.reach_checks += 1;
        let (v, at) = reach_walk_of(run, coord, lr, resident, first);
        self.reach_cache.insert(coord.0, (v, first, at));
        (v, at)
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

    /// [`BakeQueue::bakeable`]'s reach verdict, before the columns; refused, the chunk of the
    /// reach the walk stopped at.
    pub(super) fn bakeable_reach(
        &mut self,
        run: &BakeRun<'_>,
        coord: ChunkCoord,
        lr: i32,
        view: &QueueView,
        eye: [i32; 3],
    ) -> Result<bool, Blocker> {
        // ADR-0080 rule 4: a mid or far chunk bakes from the chunks resident; a near one keeps
        // the queue's mode.
        let mode = self
            .reach_mode
            .for_band(self.band_of(coord.0) == Band::Near);
        let (reach, at) =
            self.reach_cached(run, coord, lr, view.resident_fn(), mode == ReachMode::Full);
        let crosses = view
            .full_radius
            .is_some_and(|r| reach_crosses(coord, eye, lr, r));
        // A walk that is not ready always names its chunk; the candidate's own is the fallback
        // (its change drops the mark as well).
        reach_verdict(reach, mode, crosses).ok_or(Blocker::Chunk(at.unwrap_or(coord).0))
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

/// What refused a candidate at its last check ([`BakeQueue::bakeable`],
/// [`Shadow::lights_ready_near`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::bake) enum Blocker {
    /// A chunk of its reach waiting to upload or not resident full, or the lowest chunk that
    /// holds a column over its reach footprint from final.
    Chunk([i32; 3]),
    /// A light range near it not current.
    #[default]
    Lights,
}

/// A candidate's blocked mark: the step that refused it and what refused it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::bake) struct Blocked {
    pub(in crate::bake) at: u64,
    pub(in crate::bake) by: Blocker,
}

/// The candidates refused at a check ([`BakeQueue::blocked`]), each skipped by the openings until
/// its blocker changes ([`BlockedMarks::changed`], [`BlockedMarks::lights_changed`]) or
/// [`QUEUE_RECHECK`] steps pass, with the index from each blocker to the candidates it holds:
/// a step's changed chunk costs one lookup, and no walk of the marks.
#[derive(Debug, Default)]
pub(in crate::bake) struct BlockedMarks {
    marks: ChunkMap<Blocked>,
    by_chunk: ChunkMap<ChunkSet>,
    by_lights: ChunkSet,
}

impl BlockedMarks {
    /// Marks `c` as `b`, in place of any mark it held.
    pub(super) fn insert(&mut self, c: [i32; 3], b: Blocked) {
        if let Some(old) = self.marks.insert(c, b) {
            self.unindex(c, old.by);
        }
        match b.by {
            Blocker::Chunk(n) => {
                self.by_chunk.entry(n).or_default().insert(c);
            }
            Blocker::Lights => {
                self.by_lights.insert(c);
            }
        }
    }

    fn unindex(&mut self, c: [i32; 3], by: Blocker) {
        match by {
            Blocker::Chunk(n) => {
                if let Some(set) = self.by_chunk.get_mut(&n) {
                    set.remove(&c);
                    if set.is_empty() {
                        self.by_chunk.remove(&n);
                    }
                }
            }
            Blocker::Lights => {
                self.by_lights.remove(&c);
            }
        }
    }

    pub(super) fn remove(&mut self, c: &[i32; 3]) -> Option<Blocked> {
        let b = self.marks.remove(c)?;
        self.unindex(*c, b.by);
        Some(b)
    }

    #[cfg(test)]
    pub(super) fn get(&self, c: &[i32; 3]) -> Option<&Blocked> {
        self.marks.get(c)
    }

    pub(super) fn contains_key(&self, c: &[i32; 3]) -> bool {
        self.marks.contains_key(c)
    }

    pub(super) fn keys(&self) -> impl Iterator<Item = &[i32; 3]> + '_ {
        self.marks.keys()
    }

    pub(super) fn len(&self) -> usize {
        self.marks.len()
    }

    /// Keeps the marks of the candidates `keep` names.
    pub(super) fn retain(&mut self, mut keep: impl FnMut(&[i32; 3]) -> bool) {
        let gone: Vec<[i32; 3]> = self.marks.keys().filter(|c| !keep(c)).copied().collect();
        for c in gone {
            self.remove(&c);
        }
    }

    /// Whether `c` was refused fewer than [`QUEUE_RECHECK`] steps before `steps` and its blocker
    /// has not changed since: the openings skip it.
    pub(super) fn recent(&self, c: &[i32; 3], steps: u64) -> bool {
        self.marks
            .get(c)
            .is_some_and(|b| steps - b.at < QUEUE_RECHECK)
    }

    /// Drops the marks whose blocker is a chunk of `changed` (the mirror's changes of a step):
    /// those candidates are checked at the next opening. The count dropped.
    pub(super) fn changed(&mut self, changed: &[ChunkCoord]) -> usize {
        if self.by_chunk.is_empty() {
            return 0;
        }
        let mut n = 0;
        for c in changed {
            if let Some(held) = self.by_chunk.remove(&c.0) {
                for h in held {
                    self.marks.remove(&h);
                    n += 1;
                }
            }
        }
        n
    }

    /// Drops the marks whose blocker is the lights: the light key changed. The count dropped.
    pub(super) fn lights_changed(&mut self) -> usize {
        let held = std::mem::take(&mut self.by_lights);
        for h in &held {
            self.marks.remove(h);
        }
        held.len()
    }

    /// Drops every mark whose blocker is a chunk: the mirror's changes went untracked. The
    /// count dropped.
    pub(super) fn untracked(&mut self) -> usize {
        let held = std::mem::take(&mut self.by_chunk);
        let mut n = 0;
        for h in held.into_values().flatten() {
            self.marks.remove(&h);
            n += 1;
        }
        n
    }
}
