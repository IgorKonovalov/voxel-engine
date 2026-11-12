//! The store: its placement allocator, its buffers and its statistics.

use super::*;

/// [`Lighting::bake_pool_bytes`](crate::Lighting) by default; the device's storage binding clamps it.
pub(crate) const DEFAULT_BAKE_POOL_BYTES: u64 = 256 << 20;

/// The pool before the first bake or live queue: the header's words only, which hold no block.
pub(crate) const MINIMAL_POOL_BYTES: u64 = 64;

/// The store's counters, for the HUD.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StoreStats {
    /// Chunks holding a block.
    pub chunks: u64,
    /// Bytes of the pool in use.
    pub bytes: u64,
    /// Lighting pixels of the last encode whose cube face read no brick record value.
    pub missing: u32,
    /// Lighting pixels of the last encode whose cube face read its brick record.
    pub brick_reads: u32,
    /// Lighting pixels of the last encode whose cube face read a value in a coarse block.
    pub coarse_reads: u32,
    /// Chunks whose block is coarse.
    pub coarse_chunks: u64,
    /// Lighting pixels of the last encode with no brick value that took a value from the
    /// missing-light chain (ADR-0053; 0 under [`MissingLight::Black`]).
    pub fallback: u32,
    /// Lighting pixels of the last encode with no brick value that the chain left black (under
    /// [`MissingLight::Black`], every pixel with no value).
    pub still_black: u32,
    /// Bytes of the blocks' summaries (one record a block).
    pub summary_bytes: u64,
    /// Bytes of the parent table (both levels).
    pub parent_bytes: u64,
}

/// A place of `words` words from `free`'s smallest place that holds it (its rest left free),
/// else at `end`, which moves past it; `None` when that would pass `capacity` words.
pub(super) fn alloc_place(
    free: &mut Vec<(u64, u64)>,
    end: &mut u64,
    words: u64,
    capacity: u64,
) -> Option<u64> {
    let fit = (0..free.len())
        .filter(|&i| free[i].1 >= words)
        .min_by_key(|&i| (free[i].1, free[i].0));
    if let Some(i) = fit {
        let (first, have) = free.swap_remove(i);
        if have > words {
            free.push((first + words, have - words));
        }
        return Some(first);
    }
    if *end + words > capacity {
        return None;
    }
    let first = *end;
    *end += words;
    Some(first)
}

/// Frees `first .. first + words` into `free`, merged with the places it touches on either side;
/// a place that reaches `end` moves `end` back to its first word instead. So no two free places
/// touch and none ends at `end`.
pub(super) fn free_place(
    free: &mut Vec<(u64, u64)>,
    end: &mut u64,
    mut first: u64,
    mut words: u64,
) {
    if words == 0 {
        return;
    }
    if let Some(i) = free.iter().position(|&(f, w)| f + w == first) {
        let (f, w) = free.swap_remove(i);
        first = f;
        words += w;
    }
    if let Some(i) = free.iter().position(|&(f, _)| f == first + words) {
        words += free.swap_remove(i).1;
    }
    if first + words == *end {
        *end = first;
    } else {
        free.push((first, words));
    }
}

/// The pool and where its blocks lie, with a copy of every block's words for a dig's kept
/// records and its provisional blocks.
pub struct Store {
    pub(super) pool: wgpu::Buffer,
    pub(super) blocks: BTreeMap<[i32; 3], (u64, u64)>,
    pub(super) words: BTreeMap<[i32; 3], Vec<u32>>,
    /// The header's and the chunk table's words.
    pub(super) head_words: u64,
    /// The first word no block holds: a block that outgrows its place moves here.
    pub(super) end_words: u64,
    /// The chunks the last bake covered; a dig re-bakes only inside them.
    pub(super) piece: Option<ChunkExtent>,
    /// The mirror window the chunk table indexes.
    pub(super) window: Option<ChunkExtent>,
    /// The brick records' cells, in voxels: 8 or 4.
    pub(super) cell: u32,
    /// Freed places, first word and words, for [`Store::alloc`]: no two touch, and none ends at
    /// `end_words` ([`Store::free_place`]).
    pub(super) free: Vec<(u64, u64)>,
    /// Chunks whose block is the live queue's coarse pass: their table word carries
    /// [`TABLE_COARSE`].
    pub(super) coarse: std::collections::HashSet<[i32; 3]>,
    /// Chunks whose block or coarse mark changed since the live queue last took them
    /// ([`Store::take_touched`]); `None` once more than [`CHANGES_MAX`] went untaken.
    pub(super) touched: Option<Vec<[i32; 3]>>,
    /// The blocks' summaries as shown and the parent levels over them (ADR-0053).
    pub(super) light: LightSummaries,
    /// The parent table's shape over the window.
    pub(super) parents: Option<ParentTable>,
    /// What a cube face with no brick value reads; in the flags word.
    pub(super) missing: MissingLight,
    /// Milliseconds spent on summaries and parents since [`Store::take_summary_ms`].
    pub(super) summary_ms: f64,
}

impl Store {
    /// The flags word: the cell bit and the missing-light mode.
    pub(super) fn flags(&self) -> u32 {
        (if self.cell == 4 { FLAG_CELL4 } else { 0 }) | self.missing.bits() << FLAG_MISSING_SHIFT
    }

    /// What a cube face with no brick value reads.
    pub fn missing_light(&self) -> MissingLight {
        self.missing
    }

    /// Sets what a cube face with no brick value reads (the flags word follows at once).
    pub(crate) fn set_missing_light(&mut self, queue: &wgpu::Queue, mode: MissingLight) {
        self.missing = mode;
        let flags = self.flags();
        queue.write_buffer(&self.pool, STORE_FLAGS * 4, bytemuck::bytes_of(&flags));
    }

    /// Sets the mode without writing the flags word: a new pool's header carries it from its
    /// first reset.
    pub(crate) fn keep_missing_light(&mut self, mode: MissingLight) {
        self.missing = mode;
    }

    /// The summaries and parents the CPU keeps.
    pub fn light(&self) -> &LightSummaries {
        &self.light
    }

    /// The milliseconds spent on summaries and parents since the last call.
    pub(crate) fn take_summary_ms(&mut self) -> f64 {
        std::mem::take(&mut self.summary_ms)
    }

    /// Sets `c`'s summary to `block`'s (`None`: no block) and writes the parent records above
    /// it that changed.
    pub(super) fn set_summary(
        &mut self,
        queue: &wgpu::Queue,
        c: [i32; 3],
        summary: Option<[u32; BRICK_WORDS]>,
    ) {
        let Some(writes) = self.light.set(c, summary) else {
            return;
        };
        let Some(table) = self.parents else {
            return;
        };
        for (level, p, rec) in writes {
            if let Some(w) = table.word(level, p) {
                queue.write_buffer(&self.pool, w * 4, bytemuck::cast_slice(&rec));
            }
        }
    }

    /// Writes `block`'s summary into its words and records it: the parents follow.
    pub(super) fn summarise(&mut self, queue: &wgpu::Queue, c: [i32; 3], block: &mut [u32]) {
        let t = stopwatch();
        let record = record_words(self.cell);
        let s = block_summary(block, record);
        let at = SUMMARY_AT as usize;
        block[at..at + BRICK_WORDS].copy_from_slice(&s.unwrap_or_default());
        self.set_summary(queue, c, s);
        self.summary_ms += seconds_since(t) * 1e3;
    }

    /// Notes that `c`'s block or its coarse mark changed.
    pub(super) fn touch(&mut self, c: [i32; 3]) {
        if let Some(t) = self.touched.as_mut() {
            if t.len() >= CHANGES_MAX {
                self.touched = None;
            } else {
                t.push(c);
            }
        }
    }

    /// The chunks whose block or coarse mark changed since the last call, possibly repeated;
    /// `None` when too many went untaken to keep.
    pub(crate) fn take_touched(&mut self) -> Option<Vec<[i32; 3]>> {
        self.touched.replace(Vec::new())
    }

    pub(crate) fn new(device: &wgpu::Device, bytes: u64) -> Self {
        let pool = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bake pool"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pool,
            blocks: BTreeMap::new(),
            words: BTreeMap::new(),
            head_words: 0,
            end_words: 0,
            piece: None,
            window: None,
            cell: BRICK_CELL,
            free: Vec::new(),
            coarse: Default::default(),
            touched: Some(Vec::new()),
            light: LightSummaries::default(),
            parents: None,
            missing: MissingLight::default(),
            summary_ms: 0.0,
        }
    }

    /// The pool buffer.
    pub fn pool(&self) -> &wgpu::Buffer {
        &self.pool
    }

    /// The counters, the header's (`missing`, `brick_reads`, `coarse_reads`) as the caller read
    /// them.
    pub fn stats(&self, missing: u32, brick_reads: u32, coarse_reads: u32) -> StoreStats {
        StoreStats {
            chunks: self.blocks.len() as u64,
            bytes: (self.head_words + self.blocks.values().map(|b| b.1).sum::<u64>()) * 4,
            missing,
            brick_reads,
            coarse_reads,
            coarse_chunks: self.coarse.len() as u64,
            fallback: 0,
            still_black: 0,
            summary_bytes: self.blocks.len() as u64 * BRICK_WORDS as u64 * 4,
            parent_bytes: self.parents.map_or(0, |t| t.words() * 4),
        }
    }

    /// Whether `coord`'s block is the live queue's coarse pass.
    pub fn is_coarse(&self, coord: ChunkCoord) -> bool {
        self.coarse.contains(&coord.0)
    }

    /// Marks `coord`'s next [`Store::put`] as coarse or not; the table word follows at the put.
    pub(super) fn mark_coarse(&mut self, coord: ChunkCoord, coarse: bool) {
        self.touch(coord.0);
        if coarse {
            self.coarse.insert(coord.0);
        } else {
            self.coarse.remove(&coord.0);
        }
    }

    /// The chunk table's word for `coord`'s block at `first`: the first word, flagged
    /// [`TABLE_COARSE`] where the block is coarse.
    pub(super) fn table_value(&self, coord: ChunkCoord, first: u64) -> u32 {
        first as u32
            | if self.coarse.contains(&coord.0) {
                TABLE_COARSE
            } else {
                0
            }
    }

    /// The brick records' cell in voxels.
    pub fn brick_cell(&self) -> u32 {
        self.cell
    }

    /// Sets the header's flags word for brick records of cells of `cell` voxels.
    pub(super) fn set_cell(&mut self, queue: &wgpu::Queue, cell: u32) {
        self.cell = cell;
        let flags = self.flags();
        queue.write_buffer(&self.pool, STORE_FLAGS * 4, bytemuck::bytes_of(&flags));
    }

    /// `coord`'s block: its first word and its words.
    pub fn block(&self, coord: ChunkCoord) -> Option<(u64, u64)> {
        self.blocks.get(&coord.0).copied()
    }

    /// The pool's words no block holds and no job has been handed: its freed places and the
    /// room past the last block. With no job open, this plus [`StoreStats::bytes`]' words is the
    /// pool's size.
    pub fn free_words(&self) -> u64 {
        self.free.iter().map(|f| f.1).sum::<u64>() + self.pool.size() / 4 - self.end_words
    }

    /// `coord`'s block as its words, the directory first, from the store's own copy.
    pub fn block_words(&self, coord: ChunkCoord) -> Option<&[u32]> {
        self.words.get(&coord.0).map(Vec::as_slice)
    }

    /// The chunks the last bake or load covered.
    pub fn piece(&self) -> Option<ChunkExtent> {
        self.piece
    }

    /// Empties the store for `window`'s chunk table: the header written, no block.
    pub(super) fn reset(&mut self, queue: &wgpu::Queue, window: ChunkExtent) {
        let size = window.size();
        let parents = ParentTable::of(window);
        let mut header = vec![0u32; (parents.first + parents.words()) as usize];
        for k in 0..3 {
            header[4 + k] = window.min.0[k] as u32;
            header[8 + k] = size[k] as u32;
        }
        header[STORE_FLAGS as usize] = self.flags();
        queue.write_buffer(&self.pool, 0, bytemuck::cast_slice(&header));
        self.blocks.clear();
        self.words.clear();
        self.light.clear();
        self.parents = Some(parents);
        self.head_words = header.len() as u64;
        self.end_words = self.head_words;
        self.piece = None;
        self.window = Some(window);
        self.free.clear();
        self.coarse.clear();
        // Every chunk may have changed: the queue reads them all again.
        self.touched = None;
    }

    /// Empties the store for `window` for the live queue: blocks with brick records of cells of
    /// `cell` voxels, the piece the window.
    pub(crate) fn start_queue(&mut self, queue: &wgpu::Queue, window: ChunkExtent, cell: u32) {
        self.reset(queue, window);
        self.set_cell(queue, cell);
        self.piece = Some(window);
    }

    /// Follows the mirror's window: blocks of chunks outside `window` are dropped and their places
    /// freed, the header's origin moves and the chunk table is written again. A window of another
    /// size empties the store (the table's words change), keeping its flags. The chunks dropped.
    pub(crate) fn rewindow(&mut self, queue: &wgpu::Queue, window: ChunkExtent) -> Vec<[i32; 3]> {
        if self.window == Some(window) {
            return Vec::new();
        }
        let size = window.size();
        let dropped: Vec<[i32; 3]> = if self.window.map(|w| w.size()) != Some(size) {
            let all = self.blocks.keys().copied().collect();
            let cell = self.cell;
            self.reset(queue, window);
            self.set_cell(queue, cell);
            return all;
        } else {
            let inside =
                |c: &[i32; 3]| (0..3).all(|k| (0..size[k]).contains(&(c[k] - window.min.0[k])));
            self.blocks.keys().copied().filter(|c| !inside(c)).collect()
        };
        for c in &dropped {
            if let Some((first, words)) = self.blocks.remove(c) {
                self.free_place(first, words);
            }
            self.words.remove(c);
            self.coarse.remove(c);
        }
        self.window = Some(window);
        // The parent table is world-aligned: a move re-bins no chunk, only its index moves, so
        // the whole table is written again with the chunk table.
        let t = stopwatch();
        self.light.drop_chunks(&dropped);
        let parents = ParentTable::of(window);
        self.parents = Some(parents);
        let mut header = vec![0u32; parents.first as usize];
        for k in 0..3 {
            header[4 + k] = window.min.0[k] as u32;
            header[8 + k] = size[k] as u32;
        }
        header[STORE_FLAGS as usize] = self.flags();
        for (c, (first, _)) in &self.blocks {
            header[self.table_word(ChunkCoord(*c)) as usize] =
                self.table_value(ChunkCoord(*c), *first);
        }
        header.extend(self.light.table_words(&parents));
        self.summary_ms += seconds_since(t) * 1e3;
        // Words 0 to 2 are unused; the frame's counters are a buffer of their own.
        queue.write_buffer(&self.pool, 12, bytemuck::cast_slice(&header[3..]));
        dropped
    }

    /// A place of `words` words: the smallest freed place that holds it (its rest freed again),
    /// else past the last block.
    pub(crate) fn alloc(&mut self, words: u64) -> Result<u64, BakeError> {
        let capacity = self.pool.size();
        alloc_place(&mut self.free, &mut self.end_words, words, capacity / 4).ok_or(
            BakeError::PoolFull {
                needed: (self.end_words + words) * 4,
                capacity,
            },
        )
    }

    /// Frees a place [`Store::alloc`] handed out that no block took, or one a block left.
    pub(crate) fn release(&mut self, first: u64, words: u64) {
        self.free_place(first, words);
    }

    /// Frees `first .. first + words`, merged with the freed places it touches on either side, so
    /// a long run of loads and evictions leaves few large places rather than many small ones. A
    /// place that reaches `end_words` moves that end back to its first word instead.
    pub(super) fn free_place(&mut self, first: u64, words: u64) {
        free_place(&mut self.free, &mut self.end_words, first, words);
    }

    /// Writes `words` at `first` in the pool alone: what a fading block shows while the store's
    /// copy holds its target.
    /// Its summary is written into `words` first, and the parents follow it.
    pub(crate) fn show(
        &mut self,
        queue: &wgpu::Queue,
        coord: ChunkCoord,
        first: u64,
        words: &mut [u32],
    ) {
        self.summarise(queue, coord.0, words);
        queue.write_buffer(&self.pool, first * 4, bytemuck::cast_slice(words));
    }

    /// The chunk table's word for `coord`.
    pub(super) fn table_word(&self, coord: ChunkCoord) -> u64 {
        let window = self.window.expect("a reset store");
        let size = window.size();
        let w = [0, 1, 2].map(|k| i64::from(coord.0[k]) - i64::from(window.min.0[k]));
        let index = w[0] + i64::from(size[0]) * (w[1] + i64::from(size[1]) * w[2]);
        STORE_TABLE + index as u64
    }

    /// Writes `block` as `coord`'s at `first`, and points the chunk table at it.
    pub(super) fn put(
        &mut self,
        queue: &wgpu::Queue,
        coord: ChunkCoord,
        first: u64,
        mut block: Vec<u32>,
    ) {
        self.summarise(queue, coord.0, &mut block);
        queue.write_buffer(&self.pool, first * 4, bytemuck::cast_slice(&block));
        queue.write_buffer(
            &self.pool,
            self.table_word(coord) * 4,
            bytemuck::bytes_of(&self.table_value(coord, first)),
        );
        self.end_words = self.end_words.max(first + block.len() as u64);
        self.blocks.insert(coord.0, (first, block.len() as u64));
        self.words.insert(coord.0, block);
        self.touch(coord.0);
    }

    /// Drops `coord`'s block, its place freed: the chunk table reads none.
    pub(crate) fn remove(&mut self, queue: &wgpu::Queue, coord: ChunkCoord) {
        if let Some((first, words)) = self.blocks.remove(&coord.0) {
            self.touch(coord.0);
            self.free_place(first, words);
            self.words.remove(&coord.0);
            self.coarse.remove(&coord.0);
            queue.write_buffer(
                &self.pool,
                self.table_word(coord) * 4,
                bytemuck::bytes_of(&0u32),
            );
            let t = stopwatch();
            self.set_summary(queue, coord.0, None);
            self.summary_ms += seconds_since(t) * 1e3;
        }
    }

    /// Where a block of `words` for `coord` goes: its old place when it fits, else at `end`,
    /// which moves past it.
    pub(super) fn place(
        &self,
        coord: ChunkCoord,
        words: u64,
        end: &mut u64,
    ) -> Result<u64, BakeError> {
        if let Some(&(first, old)) = self.blocks.get(&coord.0)
            && words <= old
        {
            return Ok(first);
        }
        let capacity = self.pool.size();
        if (*end + words) * 4 > capacity {
            return Err(BakeError::PoolFull {
                needed: (*end + words) * 4,
                capacity,
            });
        }
        let first = *end;
        *end += words;
        Ok(first)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three neighbouring places freed in any order merge into one, which a place of their sum
    /// takes back at its first word; the pool's end is untouched while a block lies past them,
    /// and moves back over them once that block is freed too.
    #[test]
    fn freed_neighbours_merge_and_their_sum_reuses_them() {
        for order in [[0, 1, 2], [2, 0, 1], [1, 2, 0], [2, 1, 0]] {
            let (mut free, mut end) = (Vec::new(), 100u64);
            let places: Vec<u64> = [10, 20, 30, 5]
                .iter()
                .map(|&w| alloc_place(&mut free, &mut end, w, 1000).expect("room"))
                .collect();
            assert_eq!(places, [100, 110, 130, 160]);
            assert_eq!(end, 165);
            let sizes = [10, 20, 30];
            for i in order {
                free_place(&mut free, &mut end, places[i], sizes[i]);
            }
            assert_eq!(free, [(100, 60)], "order {order:?}");
            assert_eq!(end, 165);
            assert_eq!(alloc_place(&mut free, &mut end, 60, 1000), Some(100));
            assert!(free.is_empty());
            for i in order {
                free_place(&mut free, &mut end, places[i], sizes[i]);
            }
            free_place(&mut free, &mut end, places[3], 5);
            assert!(free.is_empty(), "the tail folds into the end");
            assert_eq!(end, 100);
        }
        // Without the merge the 60 would not fit under a capacity of 165 words.
        let (mut free, mut end) = (vec![], 165u64);
        for (f, w) in [(100, 10), (130, 30), (110, 20)] {
            free_place(&mut free, &mut end, f, w);
        }
        assert_eq!(alloc_place(&mut free, &mut end, 60, 165), Some(100));
        assert_eq!(alloc_place(&mut free, &mut end, 1, 165), None, "full");
    }
}
