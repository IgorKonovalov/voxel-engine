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

/// Where the pool's blocks lie, with a copy of every block's words for a dig's kept records and
/// its provisional blocks. The pool buffer is the render half's (ADR-0083): the store holds its
/// size and hands every write out, in a write list or as a direct write ([`Store::take_direct`]).
pub struct Store {
    /// The pool's size in bytes.
    pub(super) capacity: u64,
    /// Writes made outside a queue step (a one-shot bake, the queue's start, a mode's flags), in
    /// order, for the render half to write before the pool is next read.
    pub(super) direct: Vec<Write>,
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
    /// Milliseconds spent on summaries and parents since [`Store::take_summary_ms`], and of
    /// them in `wgpu` calls (the parents' writes).
    pub(super) summary_ms: f64,
    pub(super) summary_render_ms: f64,
    /// The live queue's write lists (ADR-0083): while set, the store's writes go into it, a
    /// unit at a time, and the render half applies them ([`Lists`]); `None` writes to
    /// [`Store::direct`] (a one-shot bake, a bake file's load).
    pub(crate) out: Option<ListMaker>,
    /// Nesting of [`Store::begin_unit`]: the writes inside the outermost pair are one unit.
    pub(super) unit_depth: u32,
    /// A first block's landing word is marked on the next block write ([`Store::put`] or
    /// [`Store::show`]) while set.
    pub(super) mark_landing: bool,
    /// Chunks that took their first block as a whole dig's provisional block (Plan 0105 F2):
    /// [`Store::summarise`] marks their summary [`SKY_STALE`] until their job's block lands
    /// (the queue removes them there), so the light pass reads no sky from a summary made of
    /// held and guess records. Through every rewrite in between: [`Store::put`] (a second box's
    /// provisional block), [`Store::show`] and [`Store::mark_sky_stale`] all summarise.
    pub(super) held_whole: std::collections::BTreeSet<[i32; 3]>,
}

/// The chunks of `held` a move of the store's window from `old` to `window` drops: those outside
/// `window`, or every one when the size changes (the store empties).
pub(crate) fn dropped_by_move(
    old: Option<ChunkExtent>,
    window: ChunkExtent,
    held: impl Iterator<Item = [i32; 3]>,
) -> Vec<[i32; 3]> {
    let size = window.size();
    if old.map(|w| w.size()) != Some(size) {
        return held.collect();
    }
    let inside = |c: &[i32; 3]| (0..3).all(|k| (0..size[k]).contains(&(c[k] - window.min.0[k])));
    held.filter(|c| !inside(c)).collect()
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
    pub(crate) fn set_missing_light(&mut self, mode: MissingLight) {
        self.missing = mode;
        let flags = self.flags();
        self.emit(STORE_FLAGS * 4, bytemuck::bytes_of(&flags), W_TABLE, None);
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

    /// The milliseconds spent on summaries and parents since the last call, and of them in
    /// `wgpu` calls.
    pub(crate) fn take_summary_ms(&mut self) -> (f64, f64) {
        (
            std::mem::take(&mut self.summary_ms),
            std::mem::take(&mut self.summary_render_ms),
        )
    }

    /// Writes `data` at byte `offset` of the pool: at once, or into the write list.
    fn emit(&mut self, offset: u64, data: &[u8], kind: usize, landing: Option<Landing>) {
        match self.out.as_mut() {
            None => self.direct.push(Write {
                offset,
                data: data.to_vec(),
                kind,
                landing: None,
            }),
            Some(m) => {
                m.push(Write {
                    offset,
                    data: data.to_vec(),
                    kind,
                    landing,
                });
                if self.unit_depth == 0 {
                    Self::seal(m);
                }
            }
        }
    }

    fn seal(m: &mut ListMaker) {
        if m.seal().is_err() {
            // A unit over the frame's cap: the books hold it, so it goes whole, never carried.
            m.force();
        }
    }

    /// Opens a unit: the writes until the matching [`Store::end_unit`] land in one frame.
    pub(crate) fn begin_unit(&mut self) {
        self.unit_depth += 1;
    }

    /// Closes the unit [`Store::begin_unit`] opened.
    pub(crate) fn end_unit(&mut self) {
        self.unit_depth = self.unit_depth.saturating_sub(1);
        if self.unit_depth == 0
            && let Some(m) = self.out.as_mut()
        {
            Self::seal(m);
        }
    }

    /// The landing word of a block write into a list: a first block's once after
    /// [`Store::mark_landing`], else the word the copy carries ([`Landing`]).
    fn landing_mark(&self, coord: [i32; 3]) -> Option<Landing> {
        self.out.is_some().then_some(Landing {
            chunk: coord,
            at: LANDED_AT as usize,
            first: self.mark_landing,
        })
    }

    /// Sets `c`'s landing word in the store's copy to what the render half wrote at apply.
    pub(crate) fn landed(&mut self, c: [i32; 3], word: u32) {
        if let Some(w) = self.words.get_mut(&c)
            && let Some(v) = w.get_mut(LANDED_AT as usize)
        {
            *v = word;
        }
    }

    /// Sets `c`'s summary to `block`'s (`None`: no block) and writes the parent records above
    /// it that changed.
    pub(super) fn set_summary(&mut self, c: [i32; 3], summary: Option<[u32; BRICK_WORDS]>) {
        let Some(writes) = self.light.set(c, summary) else {
            return;
        };
        let Some(table) = self.parents else {
            return;
        };
        for (level, p, rec) in writes {
            if let Some(w) = table.word(level, p) {
                self.emit(w * 4, bytemuck::cast_slice(&rec), W_SUMMARY, None);
            }
        }
    }

    /// Writes `block`'s summary into its words and records it: the parents follow.
    pub(super) fn summarise(&mut self, c: [i32; 3], block: &mut [u32]) {
        let (t, calls_at) = (stopwatch(), calls().ms);
        let record = record_words(self.cell);
        let s = block_summary(block, record);
        let at = SUMMARY_AT as usize;
        // The summary's first key word is the block's sun stamp (`KEY_SUN_AT`), kept.
        let stamp = block[KEY_SUN_AT as usize];
        block[at..at + BRICK_WORDS].copy_from_slice(&s.unwrap_or_default());
        block[KEY_SUN_AT as usize] = stamp;
        // A block whose cells hold a dig's stale visibilities, or a whole dig's provisional
        // block: its summary waits for the bake too (the no-record prior does not read it).
        if self.held_whole.contains(&c) || any_cell_stale(block, record) {
            mark_summary_stale(block);
        }
        self.set_summary(c, s);
        self.summary_ms += seconds_since(t) * 1e3;
        self.summary_render_ms += calls().ms - calls_at;
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

    /// An empty store over a pool of `bytes`.
    pub(crate) fn new(bytes: u64) -> Self {
        Self {
            capacity: bytes,
            direct: Vec::new(),
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
            summary_render_ms: 0.0,
            out: None,
            unit_depth: 0,
            mark_landing: false,
            held_whole: std::collections::BTreeSet::new(),
        }
    }

    /// The pool's size in bytes.
    pub fn capacity(&self) -> u64 {
        self.capacity
    }

    /// The writes made outside a queue step since the last call, in order: the render half
    /// writes them into the pool before it is next read.
    pub(crate) fn take_direct(&mut self) -> Vec<Write> {
        std::mem::take(&mut self.direct)
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
    pub(super) fn set_cell(&mut self, cell: u32) {
        self.cell = cell;
        let flags = self.flags();
        self.emit(STORE_FLAGS * 4, bytemuck::bytes_of(&flags), W_TABLE, None);
    }

    /// `coord`'s block: its first word and its words.
    pub fn block(&self, coord: ChunkCoord) -> Option<(u64, u64)> {
        self.blocks.get(&coord.0).copied()
    }

    /// The pool's words no block holds and no job has been handed: its freed places and the
    /// room past the last block. With no job open, this plus [`StoreStats::bytes`]' words is the
    /// pool's size.
    pub fn free_words(&self) -> u64 {
        self.free.iter().map(|f| f.1).sum::<u64>() + self.capacity / 4 - self.end_words
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
    pub(super) fn reset(&mut self, window: ChunkExtent) {
        let size = window.size();
        let parents = ParentTable::of(window);
        let mut header = vec![0u32; (parents.first + parents.words()) as usize];
        for k in 0..3 {
            header[4 + k] = window.min.0[k] as u32;
            header[8 + k] = size[k] as u32;
        }
        header[STORE_FLAGS as usize] = self.flags();
        self.emit(0, bytemuck::cast_slice(&header), W_TABLE, None);
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
        self.held_whole.clear();
        // Every chunk may have changed: the queue reads them all again.
        self.touched = None;
    }

    /// Empties the store for `window` for the live queue: blocks with brick records of cells of
    /// `cell` voxels, the piece the window.
    pub(crate) fn start_queue(&mut self, window: ChunkExtent, cell: u32) {
        self.reset(window);
        self.set_cell(cell);
        self.piece = Some(window);
    }

    /// Follows the mirror's window: blocks of chunks outside `window` are dropped and their places
    /// freed, the header's origin moves and the chunk table is written again. A window of another
    /// size empties the store (the table's words change), keeping its flags. The chunks dropped.
    pub(crate) fn rewindow(&mut self, window: ChunkExtent) -> Vec<[i32; 3]> {
        // The header, the chunk table and the parents move in one frame.
        self.begin_unit();
        let dropped = self.rewindow_unit(window);
        self.end_unit();
        dropped
    }

    fn rewindow_unit(&mut self, window: ChunkExtent) -> Vec<[i32; 3]> {
        if self.window == Some(window) {
            return Vec::new();
        }
        let size = window.size();
        let dropped = dropped_by_move(self.window, window, self.blocks.keys().copied());
        if self.window.map(|w| w.size()) != Some(size) {
            let cell = self.cell;
            self.reset(window);
            self.set_cell(cell);
            return dropped;
        }
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
        self.emit(12, bytemuck::cast_slice(&header[3..]), W_TABLE, None);
        dropped
    }

    /// A place of `words` words: the smallest freed place that holds it (its rest freed again),
    /// else past the last block.
    pub(crate) fn alloc(&mut self, words: u64) -> Result<u64, BakeError> {
        let capacity = self.capacity;
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
    pub(crate) fn show(&mut self, coord: ChunkCoord, first: u64, words: &mut [u32]) {
        self.begin_unit();
        self.summarise(coord.0, words);
        let mark = self.landing_mark(coord.0);
        self.emit(first * 4, bytemuck::cast_slice(words), W_POOL, mark);
        self.end_unit();
    }

    /// Marks `slots`' records of `coord`'s block [`SKY_STALE`] in the store's copy and the
    /// pool (a dig's ring, ADR-0070), and its summary record ([`mark_summary_stale`]: a dig
    /// job is queued for the chunk); a chunk with no block or no baked visibility there is
    /// left as it is.
    pub(super) fn mark_sky_stale(&mut self, coord: ChunkCoord, slots: &[u16], record: usize) {
        let Some(&(first, _)) = self.blocks.get(&coord.0) else {
            return;
        };
        let Some(block) = self.words.get_mut(&coord.0) else {
            return;
        };
        let cells = mark_sky_stale(block, slots, record);
        if !cells && !block_summary_baked(block) {
            return;
        }
        let mut shown = block.clone();
        self.begin_unit();
        self.summarise(coord.0, &mut shown);
        mark_summary_stale(&mut shown);
        let at = SUMMARY_AT as usize + VIS_WORD + 1;
        if let Some(block) = self.words.get_mut(&coord.0) {
            block[at] = shown[at];
        }
        self.emit(first * 4, bytemuck::cast_slice(&shown), W_POOL, None);
        self.end_unit();
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
    pub(super) fn put(&mut self, coord: ChunkCoord, first: u64, mut block: Vec<u32>) {
        self.begin_unit();
        self.summarise(coord.0, &mut block);
        let mark = self.landing_mark(coord.0);
        self.emit(first * 4, bytemuck::cast_slice(&block), W_POOL, mark);
        self.emit(
            self.table_word(coord) * 4,
            bytemuck::bytes_of(&self.table_value(coord, first)),
            W_TABLE,
            None,
        );
        self.end_words = self.end_words.max(first + block.len() as u64);
        self.blocks.insert(coord.0, (first, block.len() as u64));
        self.words.insert(coord.0, block);
        self.touch(coord.0);
        self.end_unit();
    }

    /// Drops `coord`'s block, its place freed: the chunk table reads none.
    pub(crate) fn remove(&mut self, coord: ChunkCoord) {
        if let Some((first, words)) = self.blocks.remove(&coord.0) {
            self.begin_unit();
            self.touch(coord.0);
            self.free_place(first, words);
            self.words.remove(&coord.0);
            self.coarse.remove(&coord.0);
            self.held_whole.remove(&coord.0);
            self.emit(
                self.table_word(coord) * 4,
                bytemuck::bytes_of(&0u32),
                W_TABLE,
                None,
            );
            let (t, calls_at) = (stopwatch(), calls().ms);
            self.set_summary(coord.0, None);
            self.summary_ms += seconds_since(t) * 1e3;
            self.summary_render_ms += calls().ms - calls_at;
            self.end_unit();
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
        let capacity = self.capacity;
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
