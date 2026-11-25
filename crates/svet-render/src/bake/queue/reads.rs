//! The public reads of the queue's books (ADR-0083): what the render thread asks between two
//! steps. They consult the render half's mailbox ([`Inbox`]), so an arrival the books have not
//! taken (a box, masks) reads as if they had. In lockstep they read the books themselves; with
//! the bake thread on, the snapshot it published with the newest step taken ([`Books`]).

use super::*;

/// Where a [`QueueRead`] reads the books.
#[derive(Clone, Copy)]
pub(crate) enum Source<'a> {
    /// Lockstep: the books and the store's copy.
    Live {
        queue: &'a BakeQueue,
        store: &'a Store,
    },
    /// The bake thread's snapshot.
    Books(&'a Books),
}

/// The live queue as the frame's callers read it: the books (or the bake thread's snapshot of
/// them), and the render half's mailbox. In lockstep every other read of the books goes through
/// [`std::ops::Deref`]; with the bake thread on only this type's own reads answer.
#[derive(Clone, Copy)]
pub struct QueueRead<'a> {
    pub(crate) src: Source<'a>,
    pub(crate) inbox: &'a Inbox,
}

impl std::ops::Deref for QueueRead<'_> {
    type Target = BakeQueue;
    /// The books in lockstep.
    ///
    /// # Panics
    ///
    /// With the bake thread on: the books are its own, and only [`QueueRead`]'s reads answer.
    fn deref(&self) -> &BakeQueue {
        match self.src {
            Source::Live { queue, .. } => queue,
            Source::Books(_) => {
                panic!("the queue's books are the bake thread's: read them through QueueRead")
            }
        }
    }
}

impl QueueRead<'_> {
    /// Whether the books are the bake thread's snapshot.
    pub fn threaded(&self) -> bool {
        matches!(self.src, Source::Books(_))
    }

    /// The newest step's stats ([`BakeQueue::stats`]).
    pub fn stats(&self) -> QueueStats {
        match self.src {
            Source::Live { queue, .. } => queue.stats,
            Source::Books(b) => b.stats,
        }
    }

    /// [`BakeQueue::shells`].
    pub fn shells(&self) -> (u64, u64) {
        match self.src {
            Source::Live { queue, .. } => queue.shells(),
            Source::Books(b) => b.shells,
        }
    }

    /// [`BakeQueue::view_ages`].
    pub fn view_ages(&self) -> (&[u32], &[u32]) {
        match self.src {
            Source::Live { queue, .. } => queue.view_ages(),
            Source::Books(b) => (&b.ages.0, &b.ages.1),
        }
    }

    /// The names of the per-chunk sets that hold `c`: what a chunk leaving the mirror must not
    /// leave behind. With the bake thread on, the snapshot's sets only (`busy` for every set a
    /// job, a dig or a due chunk is in).
    pub fn tracks(&self, c: ChunkCoord) -> Vec<&'static str> {
        let c = c.0;
        let q = match self.src {
            Source::Live { queue, .. } => queue,
            Source::Books(b) => {
                return [
                    ("busy", b.busy.contains(&c)),
                    ("faceless", self.faceless_now(c)),
                    ("sealed", self.sealed_now(c)),
                    ("blocked", self.blocked_now(c)),
                ]
                .into_iter()
                .filter_map(|(name, held)| held.then_some(name))
                .collect();
            }
        };
        [
            ("fades", q.fades.contains_key(&c)),
            ("digs", q.digs.contains_key(&c)),
            ("dig_open", q.dig_open.contains_key(&c)),
            ("reach", q.reach.contains(&c)),
            ("listed", q.listed.contains_key(&c)),
            ("faceless", self.faceless_now(c)),
            ("sealed", self.sealed_now(c)),
            ("blocked", self.blocked_now(c)),
            ("job_coarse", q.job_coarse.contains_key(&c)),
            ("stale", q.stale.contains_key(&c)),
            ("partial", q.partial.contains(&c)),
            ("job_partial", q.job_partial.contains(&c)),
            ("partial_due", q.partial_due.contains(&c)),
            ("provisional", q.provisional.contains(&c)),
            ("job_provisional", q.job_provisional.contains(&c)),
            ("provisional_due", q.provisional_due.contains(&c)),
            ("keys", q.keys.holds(&c)),
            ("zeroed", q.zeroed.contains(&c)),
        ]
        .into_iter()
        .filter_map(|(name, held)| held.then_some(name))
        .collect()
    }

    /// No job pending or open, no block fading, and no candidate at the last count, which ran
    /// with no job in the loop.
    pub fn empty(&self) -> bool {
        let idle = match self.src {
            Source::Live { queue: q, .. } => {
                q.lp.idle()
                    && q.fades.is_empty()
                    && q.counted_idle
                    && q.stats.depth == 0
                    && q.digs.is_empty()
                    && q.reach.is_empty()
                    && q.partial_due.is_empty()
                    && q.provisional_due.is_empty()
            }
            Source::Books(b) => b.idle,
        };
        idle && !self.boxes_waiting()
    }

    /// Whether an edit box waits for a step.
    pub fn boxes_waiting(&self) -> bool {
        let held = match self.src {
            Source::Live { queue, .. } => !queue.boxes.is_empty(),
            Source::Books(b) => b.boxes,
        };
        held || self.inbox.has_box()
    }

    /// No edit box, dig brick or ring-3 chunk waits, and no dig job runs.
    pub fn digs_settled(&self) -> bool {
        let quiet = match self.src {
            Source::Live { queue: q, .. } => {
                q.digs.is_empty() && q.dig_open.is_empty() && q.reach.is_empty()
            }
            Source::Books(b) => b.digs_quiet,
        };
        !self.boxes_waiting() && quiet
    }

    /// Whether `c` holds its block at the full count or is known to have none: no face, or
    /// (`absent`) no chunk.
    pub fn settled(&self, c: ChunkCoord, absent: bool) -> bool {
        let c = c.0;
        let (full, busy) = match self.src {
            Source::Live { queue: q, store } => (
                store.blocks.contains_key(&c) && !q.lesser(store, c),
                q.lp.chunks().any(|j| j.0 == c)
                    || q.digs.contains_key(&c)
                    || q.reach.contains(&c)
                    || q.partial_due.contains(&c)
                    || q.provisional_due.contains(&c),
            ),
            Source::Books(b) => (
                b.blocks.contains_key(&c) && !b.lesser.contains(&c),
                b.busy.contains(&c),
            ),
        };
        (full || self.faceless_now(c) || self.sealed_now(c) || absent) && !busy
    }

    /// Whether `c` has had its first pass: it holds a block (coarse or full), is known to have
    /// no face, or its reach or lights were not ready at its last check.
    pub fn passed(&self, c: ChunkCoord) -> bool {
        let held = match self.src {
            Source::Live { store, .. } => store.blocks.contains_key(&c.0),
            Source::Books(b) => b.blocks.contains_key(&c.0),
        };
        held || self.faceless_now(c.0) || self.sealed_now(c.0) || self.blocked_now(c.0)
    }

    /// Whether `c` is known faceless, with the mailbox's arrivals taken.
    fn faceless_now(&self, c: [i32; 3]) -> bool {
        let held = match self.src {
            Source::Live { queue, .. } => queue.faceless.contains(&c),
            Source::Books(b) => b.faceless.contains(&c),
        };
        held && !self.inbox.unknown(c)
    }

    /// Whether `c` is held sealed, with the mailbox's boxes taken.
    fn sealed_now(&self, c: [i32; 3]) -> bool {
        let held = match self.src {
            Source::Live { queue, .. } => queue.sealed.holds(c),
            Source::Books(b) => b.sealed.contains(&c),
        };
        held && !self.inbox.near(c)
    }

    /// Whether `c`'s listing is blocked (reach or lights not ready), with the mailbox's arrivals
    /// taken.
    fn blocked_now(&self, c: [i32; 3]) -> bool {
        let held = match self.src {
            Source::Live { queue, .. } => queue.blocked.contains_key(&c),
            Source::Books(b) => b.blocked.contains(&c),
        };
        held && !self.inbox.unknown(c)
    }
}
