//! The light-sample buffer's free ranges with an index for the first fit: the lowest-start free
//! range of at least `n` samples, found by a descent bounded by the tree's depth and one block's
//! ranges instead of a walk of every free range below it.

use std::collections::BTreeMap;

/// Samples a block of the index covers: a block's entry is the longest free range starting in
/// it.
const BLOCK_SHIFT: u32 = 8;

/// The free ranges (start to length) and, over blocks of `1 << BLOCK_SHIFT` samples, a max tree
/// of the longest range starting in each block. A query descends to the leftmost block whose
/// longest range fits, then walks that block's ranges by start: the first that fits is the
/// lowest-start fit of the whole buffer, the answer a walk of every range by start gives (a
/// range's start is where its samples land in the buffer, so the answer must be that one, not any
/// fit).
pub(super) struct FreeRanges {
    map: BTreeMap<u32, u32>,
    /// Leaves from `leaves`; node `i`'s children are `2 i` and `2 i + 1`.
    tree: Vec<u32>,
    leaves: usize,
}

impl FreeRanges {
    /// An empty set over a buffer of `samples` samples.
    pub(super) fn new(samples: u32) -> Self {
        let blocks = ((u64::from(samples) >> BLOCK_SHIFT) + 1).next_power_of_two() as usize;
        Self {
            map: BTreeMap::new(),
            tree: vec![0; 2 * blocks],
            leaves: blocks,
        }
    }

    pub(super) fn len(&self) -> usize {
        self.map.len()
    }

    pub(super) fn get(&self, start: u32) -> Option<u32> {
        self.map.get(&start).copied()
    }

    /// The free range ending just at `start`, if one does.
    pub(super) fn before(&self, start: u32) -> Option<(u32, u32)> {
        self.map
            .range(..start)
            .next_back()
            .map(|(&s, &l)| (s, l))
            .filter(|&(s, l)| s + l == start)
    }

    pub(super) fn insert(&mut self, start: u32, len: u32) {
        self.map.insert(start, len);
        self.refresh(start);
    }

    pub(super) fn remove(&mut self, start: u32) {
        self.map.remove(&start);
        self.refresh(start);
    }

    /// Recomputes the block of `start` and its path to the root.
    fn refresh(&mut self, start: u32) {
        let b = (start >> BLOCK_SHIFT) as usize;
        let lo = (b as u32) << BLOCK_SHIFT;
        let hi = lo.saturating_add(1 << BLOCK_SHIFT);
        let best = self.map.range(lo..hi).map(|(_, &l)| l).max().unwrap_or(0);
        let mut i = self.leaves + b;
        self.tree[i] = best;
        while i > 1 {
            i /= 2;
            self.tree[i] = self.tree[2 * i].max(self.tree[2 * i + 1]);
        }
    }

    /// The lowest-start free range of at least `n` samples, its start and length; adds the tree
    /// nodes and ranges visited to `steps`.
    pub(super) fn first_fit(&self, n: u32, steps: &mut u64) -> Option<(u32, u32)> {
        // A run of no samples takes the lowest free range, as the walk by start does.
        let n = n.max(1);
        *steps += 1;
        if self.tree[1] < n {
            return None;
        }
        let mut i = 1;
        while i < self.leaves {
            *steps += 1;
            i = if self.tree[2 * i] >= n {
                2 * i
            } else {
                2 * i + 1
            };
        }
        let lo = ((i - self.leaves) as u32) << BLOCK_SHIFT;
        let hi = lo.saturating_add(1 << BLOCK_SHIFT);
        self.map
            .range(lo..hi)
            .inspect(|_| *steps += 1)
            .find(|&(_, &l)| l >= n)
            .map(|(&s, &l)| (s, l))
    }

    /// [`FreeRanges::first_fit`] by a walk of every range by start: the tests' reference.
    #[cfg(test)]
    pub(super) fn first_fit_linear(&self, n: u32) -> Option<(u32, u32)> {
        self.map
            .iter()
            .find(|&(_, &l)| l >= n)
            .map(|(&s, &l)| (s, l))
    }

    /// The ranges by start, for the tests.
    #[cfg(test)]
    pub(super) fn ranges(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        self.map.iter().map(|(&s, &l)| (s, l))
    }
}
