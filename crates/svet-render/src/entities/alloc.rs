//! A free list of ranges over `[0, total)`: first fit from the lowest start, coalescing on
//! free (ADR-0060). It serves the entity pool's words and the instance table's slots.
//!
//! Every decision walks an ordered map by start, so the allocator is a function of its call
//! sequence alone: the same calls give the same starts on every platform.

use std::collections::BTreeMap;

/// Free ranges over `[0, total)`, keyed by start. Two free ranges never touch: a free merges
/// with the ranges either side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RangeAlloc {
    /// Start to length, of each free range.
    free: BTreeMap<u32, u32>,
    total: u32,
    free_total: u32,
}

impl RangeAlloc {
    /// Everything free: one range of `total`.
    pub fn new(total: u32) -> Self {
        let mut free = BTreeMap::new();
        if total > 0 {
            free.insert(0, total);
        }
        Self {
            free,
            total,
            free_total: total,
        }
    }

    /// The start of a range of `len` taken from the lowest free range that holds it; `None`
    /// when none does. A `len` of 0 takes nothing and is `None`.
    pub fn alloc(&mut self, len: u32) -> Option<u32> {
        if len == 0 {
            return None;
        }
        let (&start, &have) = self.free.iter().find(|&(_, &l)| l >= len)?;
        self.free.remove(&start);
        if have > len {
            self.free.insert(start + len, have - len);
        }
        self.free_total -= len;
        Some(start)
    }

    /// Returns `[start, start + len)` to the free list, merged with a free range ending at
    /// `start` and one starting at `start + len`.
    ///
    /// # Panics
    ///
    /// When the range leaves `[0, total)` or overlaps a free range: the caller freed what it
    /// does not hold, an invariant of the pool and the slot table.
    pub fn free(&mut self, start: u32, len: u32) {
        if len == 0 {
            return;
        }
        let end = start.checked_add(len).expect("a range past u32::MAX");
        assert!(end <= self.total, "a range past the allocator's end");
        let (mut s, mut l) = (start, len);
        if let Some((&ps, &pl)) = self.free.range(..start).next_back() {
            assert!(ps + pl <= start, "a free overlapping a free range");
            if ps + pl == start {
                self.free.remove(&ps);
                s = ps;
                l += pl;
            }
        }
        if let Some((&ns, &nl)) = self.free.range(start..).next() {
            assert!(ns >= end, "a free overlapping a free range");
            if ns == end {
                self.free.remove(&ns);
                l += nl;
            }
        }
        self.free.insert(s, l);
        self.free_total += len;
    }

    /// The size it was made with.
    pub fn total(&self) -> u32 {
        self.total
    }

    /// The sum of the free ranges.
    pub fn free_total(&self) -> u32 {
        self.free_total
    }

    /// The longest free range; 0 when nothing is free.
    pub fn largest(&self) -> u32 {
        self.free.values().copied().max().unwrap_or(0)
    }

    /// The free ranges as `(start, len)`, ascending by start.
    pub fn ranges(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        self.free.iter().map(|(&s, &l)| (s, l))
    }
}
