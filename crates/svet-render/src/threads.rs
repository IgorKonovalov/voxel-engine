//! The process's busy threads in one place: the stream workers, the bake's listing pool, the
//! render thread and the bake thread. One function hands out the workers' default and the
//! listers' count for the workers chosen, so the two fill the logical CPUs exactly at the
//! default and the sum says by how much a chosen count runs over.

use std::fmt;

/// The busy threads a run starts on a machine of `logical` CPUs ([`ThreadPlan::for_logical`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThreadPlan {
    /// The logical CPUs the threads fill.
    pub logical: usize,
    /// The stream workers: the chosen count, or half the logical CPUs and at least 1. Half is a
    /// proxy for the physical cores on an SMT machine: workers on both siblings of a core
    /// starve the render thread and the light bake that share them, and the frame tail grows.
    pub workers: usize,
    /// The bake's listing threads: what the workers and the render and bake threads leave, at
    /// least 1.
    pub listers: usize,
}

/// The render thread and the bake thread, counted beside the pools.
const FIXED: usize = 2;

impl ThreadPlan {
    /// The threads on `logical` CPUs with `workers` stream workers chosen (`None`: the default).
    pub fn for_logical(logical: usize, workers: Option<usize>) -> Self {
        let workers = workers.unwrap_or((logical / 2).max(1));
        let listers = logical.saturating_sub(workers).saturating_sub(FIXED).max(1);
        Self {
            logical,
            workers,
            listers,
        }
    }

    /// The threads on this machine's logical CPUs ([`logical_cpus`]).
    pub fn here(workers: Option<usize>) -> Self {
        Self::for_logical(logical_cpus(), workers)
    }

    /// The busy threads: the workers, the listers, the render and the bake threads.
    pub fn total(&self) -> usize {
        self.workers + self.listers + FIXED
    }

    /// By how many busy threads the sum runs over the logical CPUs (0: it fits).
    pub fn over(&self) -> usize {
        self.total().saturating_sub(self.logical)
    }
}

/// `threads: 8 workers + 6 listers + render + bake = 16 of 16 logical CPUs`, with
/// ` (over by N)` when the sum runs over.
impl fmt::Display for ThreadPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "threads: {} workers + {} listers + render + bake = {} of {} logical CPUs",
            self.workers,
            self.listers,
            self.total(),
            self.logical
        )?;
        match self.over() {
            0 => Ok(()),
            n => write!(f, " (over by {n})"),
        }
    }
}

/// This machine's logical CPUs, 1 when the count cannot be read.
pub fn logical_cpus() -> usize {
    std::thread::available_parallelism().map_or(1, std::num::NonZero::get)
}

#[cfg(test)]
mod tests {
    use super::ThreadPlan;

    /// The defaults fill the logical CPUs exactly where the floors allow, a chosen count runs
    /// over by what it takes past them, and the defaults are the two formulas the fly-through
    /// and the listing pool held before (half the CPUs, at least 1; the CPUs less that half and
    /// the render and bake threads, at least 1) on every count from 1 to 64.
    #[test]
    fn the_thread_plan_fills_the_floor_exactly() {
        let p = ThreadPlan::for_logical(8, None);
        assert_eq!((p.workers, p.listers, p.total(), p.over()), (4, 2, 8, 0));
        let p = ThreadPlan::for_logical(16, None);
        assert_eq!((p.workers, p.listers, p.total(), p.over()), (8, 6, 16, 0));
        let p = ThreadPlan::for_logical(4, None);
        assert_eq!((p.workers, p.listers, p.total(), p.over()), (2, 1, 5, 1));
        let p = ThreadPlan::for_logical(8, Some(6));
        assert_eq!((p.workers, p.listers, p.total(), p.over()), (6, 1, 9, 1));
        for logical in 1..=64usize {
            let p = ThreadPlan::for_logical(logical, None);
            assert_eq!(p.workers, (logical / 2).max(1), "workers on {logical}");
            let old_listers = logical.saturating_sub(logical / 2).saturating_sub(2).max(1);
            assert_eq!(p.listers, old_listers, "listers on {logical}");
        }
    }
}
