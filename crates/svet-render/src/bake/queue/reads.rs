//! The public reads of the queue's books (ADR-0083): what the render thread asks between two
//! steps. They consult the render half's mailbox ([`Inbox`]), so an arrival the books have not
//! taken (a box, masks) reads as if they had.

use super::*;

/// The live queue as the frame's callers read it: the books, the store's copy, and the render
/// half's mailbox. Every other read of the books goes through [`std::ops::Deref`].
#[derive(Clone, Copy)]
pub struct QueueRead<'a> {
    pub(crate) queue: &'a BakeQueue,
    pub(crate) store: &'a Store,
    pub(crate) inbox: &'a Inbox,
}

impl std::ops::Deref for QueueRead<'_> {
    type Target = BakeQueue;
    fn deref(&self) -> &BakeQueue {
        self.queue
    }
}

impl QueueRead<'_> {
    /// The names of the per-chunk sets that hold `c`: what a chunk leaving the mirror must not
    /// leave behind.
    pub fn tracks(&self, c: ChunkCoord) -> Vec<&'static str> {
        let (q, c) = (self.queue, c.0);
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
        let q = self.queue;
        q.lp.idle()
            && q.fades.is_empty()
            && q.counted_idle
            && q.stats.depth == 0
            && q.digs.is_empty()
            && q.reach.is_empty()
            && q.partial_due.is_empty()
            && q.provisional_due.is_empty()
            && !self.boxes_waiting()
    }

    /// Whether an edit box waits for a step.
    pub fn boxes_waiting(&self) -> bool {
        !self.queue.boxes.is_empty() || self.inbox.has_box()
    }

    /// No edit box, dig brick or ring-3 chunk waits, and no dig job runs.
    pub fn digs_settled(&self) -> bool {
        let q = self.queue;
        !self.boxes_waiting() && q.digs.is_empty() && q.dig_open.is_empty() && q.reach.is_empty()
    }

    /// Whether `c` holds its block at the full count or is known to have none: no face, or
    /// (`absent`) no chunk.
    pub fn settled(&self, c: ChunkCoord, absent: bool) -> bool {
        let (q, store) = (self.queue, self.store);
        ((store.blocks.contains_key(&c.0) && !q.lesser(store, c.0))
            || self.faceless_now(c.0)
            || self.sealed_now(c.0)
            || absent)
            && !q.lp.chunks().any(|j| j == c)
            && !q.digs.contains_key(&c.0)
            && !q.reach.contains(&c.0)
            && !q.partial_due.contains(&c.0)
            && !q.provisional_due.contains(&c.0)
    }

    /// Whether `c` has had its first pass: it holds a block (coarse or full), is known to have
    /// no face, or its reach or lights were not ready at its last check.
    pub fn passed(&self, c: ChunkCoord) -> bool {
        self.store.blocks.contains_key(&c.0)
            || self.faceless_now(c.0)
            || self.sealed_now(c.0)
            || self.blocked_now(c.0)
    }

    /// Whether `c` is known faceless, with the mailbox's arrivals taken.
    fn faceless_now(&self, c: [i32; 3]) -> bool {
        self.queue.faceless.contains(&c) && !self.inbox.unknown(c)
    }

    /// Whether `c` is held sealed, with the mailbox's boxes taken.
    fn sealed_now(&self, c: [i32; 3]) -> bool {
        self.queue.sealed.holds(c) && !self.inbox.near(c)
    }

    /// Whether `c`'s listing is blocked (reach or lights not ready), with the mailbox's arrivals
    /// taken.
    fn blocked_now(&self, c: [i32; 3]) -> bool {
        self.queue.blocked.contains_key(&c) && !self.inbox.unknown(c)
    }
}
