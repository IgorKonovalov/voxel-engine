//! The missing-light chain: record means, block summaries, the parent table, the provisional
//! and fallback blocks and the fade between them. Pure CPU.

use super::*;

/// What a cube face with no brick value reads (ADR-0053): black as before, or the first value
/// for its direction along a chain of summaries, nearest first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MissingLight {
    /// 0, as before the summaries.
    Black,
    /// The chunk's own summary, then the mean of its six face neighbours' summaries.
    Neighbour,
    /// The chunk's own summary, then the level-1 and the level-2 parent records.
    Parent,
    /// Own, neighbours, level 1, level 2: the default.
    #[default]
    Chain,
}

impl MissingLight {
    /// Every mode, in the order key `7` cycles them.
    pub const ALL: [Self; 4] = [Self::Black, Self::Neighbour, Self::Parent, Self::Chain];

    /// The mode's value in the flags word's bits 2 and 3 (`temporal.wgsl`'s `store_mode`).
    pub fn bits(self) -> u32 {
        match self {
            Self::Black => 0,
            Self::Neighbour => 1,
            Self::Parent => 2,
            Self::Chain => 3,
        }
    }

    /// The mode's name, as `--missing-light` takes it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Black => "black",
            Self::Neighbour => "neighbour",
            Self::Parent => "parent",
            Self::Chain => "chain",
        }
    }

    /// The mode named `s`.
    pub fn parse(s: &str) -> Result<Self, String> {
        Self::ALL.into_iter().find(|m| m.name() == s).ok_or(format!(
            "the missing light is black, neighbour, parent or chain, not {s}"
        ))
    }

    pub(super) fn neighbours(self) -> bool {
        matches!(self, Self::Neighbour | Self::Chain)
    }

    pub(super) fn parents(self) -> bool {
        matches!(self, Self::Parent | Self::Chain)
    }
}

/// Whether the live queue bakes a chunk whose reach is not resident full (ADR-0046's reach
/// rule, Plan 0067 Phase 5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ReachMode {
    /// A chunk waits until every chunk within its light reach is mirrored full and current.
    #[default]
    Full,
    /// A chunk whose reach crosses the streamer's full radius ([`QueueView::full_radius`]),
    /// with nothing in its reach waiting to upload, bakes from the chunks resident full and is
    /// flagged partial; a chunk within its reach that later arrives full re-queues it as a
    /// refinement.
    Partial,
}

impl ReachMode {
    /// Both modes, in the order a key cycles them.
    pub const ALL: [Self; 2] = [Self::Full, Self::Partial];

    /// The mode's name, as `--reach` takes it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Partial => "partial",
        }
    }

    /// The mode named `s`.
    pub fn parse(s: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|m| m.name() == s)
            .ok_or(format!("the reach is full or partial, not {s}"))
    }
}

/// A chunk's reach ([`reach_ready`]): every chunk within it mirrored full and current; nothing
/// waiting to upload but a chunk not resident full (a streamed world's edge); or a chunk
/// waiting to upload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Reach {
    Ready,
    Partial,
    Waiting,
}

/// Whether offset `o` (chunks) lies within a chunk's light reach of `lr` chunks: the gap
/// between the two chunks' boxes under `lr` chunks.
pub(super) fn in_reach(o: [i32; 3], lr: i32) -> bool {
    let gap2: i64 = o
        .iter()
        .map(|&d| i64::from((d.abs() - 1).max(0)).pow(2))
        .sum();
    gap2 < i64::from(lr).pow(2)
}

/// The offsets (chunks) within a light reach of `lr` chunks ([`in_reach`]).
pub(super) fn reach_offsets(lr: i32) -> impl Iterator<Item = [i32; 3]> {
    let r = lr + 1;
    (-r..=r)
        .flat_map(move |z| (-r..=r).flat_map(move |y| (-r..=r).map(move |x| [x, y, z])))
        .filter(move |&o| in_reach(o, lr))
}

/// A running mean of records by direction: each direction over the records that hold it, the
/// material the most of them keep (the lowest on a tie), the mask their union.
#[derive(Default)]
pub(super) struct RecordMean {
    pub(super) sums: [[f64; 3]; 6],
    pub(super) counts: [u32; 6],
    /// Material and how many records keep it.
    pub(super) materials: Vec<(u32, u32)>,
}

impl RecordMean {
    pub(super) fn add(&mut self, r: &[u32]) {
        let mask = r[6] >> 16;
        if mask == 0 {
            return;
        }
        for (f, (sum, count)) in self.sums.iter_mut().zip(&mut self.counts).enumerate() {
            if mask & 1 << f != 0 {
                let v = rgb9e5_unpack(r[f]);
                for (s, x) in sum.iter_mut().zip(v) {
                    *s += f64::from(x);
                }
                *count += 1;
            }
        }
        let m = r[6] & 0xFFFF;
        match self.materials.iter_mut().find(|e| e.0 == m) {
            Some(e) => e.1 += 1,
            None => self.materials.push((m, 1)),
        }
    }

    /// The mean as a record; `None` when no record held a direction.
    pub(super) fn record(&self) -> Option<[u32; BRICK_WORDS]> {
        let (material, _) = self
            .materials
            .iter()
            .copied()
            .max_by_key(|&(m, n)| (n, std::cmp::Reverse(m)))?;
        let mut out = [0u32; BRICK_WORDS];
        let mut mask = 0;
        for (f, (w, (sum, &count))) in out
            .iter_mut()
            .zip(self.sums.iter().zip(&self.counts))
            .enumerate()
        {
            if count > 0 {
                let n = f64::from(count);
                *w = rgb9e5_pack(sum.map(|s| (s / n) as f32));
                mask |= 1 << f;
            }
        }
        out[6] = material | mask << 16;
        Some(out)
    }
}

/// `block`'s light summary (ADR-0053) from its brick records of `record` words: a direction's
/// value the mean over the cells that hold it, the material the most of those cells keep, the
/// mask their union; `None` for a block with no lit direction.
pub(crate) fn block_summary(block: &[u32], record: usize) -> Option<[u32; BRICK_WORDS]> {
    let mut mean = RecordMean::default();
    for &dir in &block[..CHUNK_BRICKS] {
        if dir == 0 {
            continue;
        }
        let run = (dir >> 11) as usize;
        for cell in (run - record..run).step_by(BRICK_WORDS) {
            mean.add(&block[cell..cell + BRICK_WORDS]);
        }
    }
    mean.record()
}

/// The mean by direction of `records` ([`block_summary`]'s rule over whole records): a parent
/// cell's record from its children's.
pub(crate) fn mean_record<'a>(
    records: impl IntoIterator<Item = &'a [u32; BRICK_WORDS]>,
) -> Option<[u32; BRICK_WORDS]> {
    let mut mean = RecordMean::default();
    for r in records {
        mean.add(r);
    }
    mean.record()
}

/// The two-level parent table's shape over a mirror window (ADR-0053): level `l` (1 or 2) of
/// cells `1 << l` chunks a side, world-aligned (`c >> l`), indexed from the window's origin's
/// cell over `(size + (1 << l) - 1) >> l` plus one cells a side, level 2 after level 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParentTable {
    /// The level-1 table's first word.
    pub first: u64,
    pub(super) origin: [[i32; 3]; 2],
    pub(super) side: [[i32; 3]; 2],
}

impl ParentTable {
    /// The table after `window`'s chunk table.
    pub fn of(window: ChunkExtent) -> Self {
        let size = window.size();
        let o = window.min.0;
        let table_words = (size[0] as u64) * (size[1] as u64) * (size[2] as u64);
        let lv = |l: i32| {
            (
                o.map(|v| v.div_euclid(1 << l)),
                size.map(|s| ((s + (1 << l) - 1) >> l) + 1),
            )
        };
        let (o1, s1) = lv(1);
        let (o2, s2) = lv(2);
        Self {
            first: STORE_TABLE + table_words,
            origin: [o1, o2],
            side: [s1, s2],
        }
    }

    pub(super) fn cells(&self, level: usize) -> u64 {
        self.side[level].iter().map(|&s| s as u64).product()
    }

    /// The table's words, both levels.
    pub fn words(&self) -> u64 {
        (self.cells(0) + self.cells(1)) * BRICK_WORDS as u64
    }

    /// The first word of level `level` (1 or 2) cell `p`'s record, `None` outside the table.
    pub fn word(&self, level: usize, p: [i32; 3]) -> Option<u64> {
        let i = level - 1;
        let w = [0, 1, 2].map(|k| p[k] - self.origin[i][k]);
        if (0..3).any(|k| w[k] < 0 || w[k] >= self.side[i][k]) {
            return None;
        }
        let s = self.side[i];
        let index = (w[0] + s[0] * (w[1] + s[1] * w[2])) as u64;
        let base = self.first
            + if i == 1 {
                self.cells(0) * BRICK_WORDS as u64
            } else {
                0
            };
        Some(base + index * BRICK_WORDS as u64)
    }
}

/// The level-`level` cell (1 or 2) holding chunk `c`.
pub(super) fn parent_of(c: [i32; 3], level: usize) -> [i32; 3] {
    c.map(|v| v.div_euclid(1 << level))
}

/// The blocks' light summaries and the two parent levels over them, as the CPU keeps them
/// (ADR-0053): each change returns the parent records to write. No GPU here.
#[derive(Clone, Debug, Default)]
pub struct LightSummaries {
    pub(super) summaries: std::collections::HashMap<[i32; 3], [u32; BRICK_WORDS]>,
    pub(super) levels: [std::collections::HashMap<[i32; 3], [u32; BRICK_WORDS]>; 2],
}

impl LightSummaries {
    /// Chunk `c`'s summary.
    pub fn summary(&self, c: [i32; 3]) -> Option<&[u32; BRICK_WORDS]> {
        self.summaries.get(&c)
    }

    /// Level `level`'s (1 or 2) record of cell `p`.
    pub fn parent(&self, level: usize, p: [i32; 3]) -> Option<&[u32; BRICK_WORDS]> {
        self.levels[level - 1].get(&p)
    }

    /// Empties every summary and parent.
    pub fn clear(&mut self) {
        self.summaries.clear();
        self.levels.iter_mut().for_each(|l| l.clear());
    }

    /// Sets `c`'s summary (`None`: its block was freed or holds no light) and recomputes the two
    /// parent records above it: the level-1 and level-2 cells and their records (zero for none),
    /// `None` where the summary did not change.
    #[allow(
        clippy::type_complexity,
        reason = "two (level, cell, record) writes, named in the doc"
    )]
    pub fn set(
        &mut self,
        c: [i32; 3],
        summary: Option<[u32; BRICK_WORDS]>,
    ) -> Option<[(usize, [i32; 3], [u32; BRICK_WORDS]); 2]> {
        if self.summaries.get(&c).copied() == summary {
            return None;
        }
        match summary {
            Some(s) => self.summaries.insert(c, s),
            None => self.summaries.remove(&c),
        };
        Some([
            self.recompute(1, parent_of(c, 1)),
            self.recompute(2, parent_of(c, 2)),
        ])
    }

    /// Recomputes level `level`'s record of cell `p` from its eight children (chunks for level
    /// 1, level-1 cells for level 2).
    pub(super) fn recompute(
        &mut self,
        level: usize,
        p: [i32; 3],
    ) -> (usize, [i32; 3], [u32; BRICK_WORDS]) {
        let children = (0..8).map(|i| [0, 1, 2].map(|k| p[k] * 2 + (i >> k & 1)));
        let below = if level == 1 {
            &self.summaries
        } else {
            &self.levels[0]
        };
        let rec = mean_record(children.filter_map(|c| below.get(&c)));
        match rec {
            Some(r) => self.levels[level - 1].insert(p, r),
            None => self.levels[level - 1].remove(&p),
        };
        (level, p, rec.unwrap_or_default())
    }

    /// Drops the summaries of `dropped` and recomputes every parent.
    pub fn drop_chunks(&mut self, dropped: &[[i32; 3]]) {
        for c in dropped {
            self.summaries.remove(c);
        }
        self.levels[0].clear();
        self.levels[1].clear();
        let cells1: std::collections::BTreeSet<[i32; 3]> =
            self.summaries.keys().map(|&c| parent_of(c, 1)).collect();
        for p in cells1 {
            self.recompute(1, p);
        }
        let cells2: std::collections::BTreeSet<[i32; 3]> =
            self.levels[0].keys().map(|&p| parent_of(p, 1)).collect();
        for p in cells2 {
            self.recompute(2, p);
        }
    }

    /// The parent table's words for `table` (both levels, zero where a cell holds no value).
    pub fn table_words(&self, table: &ParentTable) -> Vec<u32> {
        let mut out = vec![0u32; table.words() as usize];
        for level in 1..=2 {
            for (&p, r) in &self.levels[level - 1] {
                if let Some(w) = table.word(level, p) {
                    let at = (w - table.first) as usize;
                    out[at..at + BRICK_WORDS].copy_from_slice(r);
                }
            }
        }
        out
    }

    /// The value the read chain (`temporal.wgsl`'s `store_fallback`) gives chunk `c` under
    /// `mode`, direction by direction, at emission scale 1: its own summary, the mean of its six
    /// face neighbours' that hold the direction, level 1's record, level 2's; the mask the
    /// directions found. `None` under [`MissingLight::Black`] or where no direction is found.
    pub fn chain(&self, c: [i32; 3], mode: MissingLight) -> Option<[u32; BRICK_WORDS]> {
        if mode == MissingLight::Black {
            return None;
        }
        let own = self.summaries.get(&c);
        let neighbours: Vec<&[u32; BRICK_WORDS]> = if mode.neighbours() {
            (0..6)
                .filter_map(|f| {
                    let mut n = c;
                    n[f / 2] += if f % 2 == 0 { -1 } else { 1 };
                    self.summaries.get(&n)
                })
                .collect()
        } else {
            Vec::new()
        };
        let near = mean_record(neighbours.iter().copied());
        let parents = [1, 2].map(|l| {
            mode.parents()
                .then(|| self.levels[l - 1].get(&parent_of(c, l)))
                .flatten()
        });
        let mut out = [0u32; BRICK_WORDS];
        let mut mask = 0u32;
        let mut material = None;
        let sources = [own, near.as_ref(), parents[0], parents[1]];
        for (f, w) in out.iter_mut().take(6).enumerate() {
            if let Some(r) = sources.iter().flatten().find(|r| r[6] >> (16 + f) & 1 != 0) {
                *w = r[f];
                mask |= 1 << f;
                material.get_or_insert(r[6] & 0xFFFF);
            }
        }
        out[6] = material? | mask << 16;
        Some(out)
    }
}

/// The block's directory and its words: the summary's place after the directory, then each
/// surface brick's record of `record` words, the directory pointing past it (`run offset << 11`,
/// no cell after it). The returned head is the directory and the (zero) summary.
pub(super) fn layout(faces: &ChunkFaces, record: usize) -> (Vec<u32>, u64) {
    let mut dir = vec![0u32; BLOCK_HEAD_WORDS as usize];
    let mut at = BLOCK_HEAD_WORDS as u32;
    for (slot, _) in &faces.bricks {
        at += record as u32;
        dir[usize::from(*slot)] = at << 11;
    }
    (dir, u64::from(at))
}

/// The brick records of `record` words of a block with them, by brick slot.
pub(super) fn records_of(block: &[u32], record: usize) -> KeptBricks {
    let mut out = KeptBricks::new();
    for slot in 0..CHUNK_BRICKS {
        let dir = block[slot];
        if dir == 0 {
            continue;
        }
        let run = (dir >> 11) as usize;
        out.insert(slot as u16, block[run - record..run].to_vec());
    }
    out
}

/// A dug chunk's block before its job ends: the layout of `faces` at the brick level, each
/// brick's record the store's (a brick the edit kept), else the mean, direction by direction, of
/// the records of the bricks around it within one brick (two where one holds none), across
/// chunks, a direction none of them holds their mean over every direction; a brick with none
/// around it holds no light. A kept brick in `touched` holds its record with every direction it
/// held none for filled ([`fill_directions`]). Records of one cell a brick only; at
/// other cells a new brick holds none.
pub(super) fn provisional_block(
    store: &Store,
    c: [i32; 3],
    faces: &ChunkFaces,
    record: usize,
    touched: &[u16],
) -> Vec<u32> {
    let (head, words) = layout(faces, record);
    let mut block = head;
    block.reserve(words as usize);
    let record_at = |chunk: [i32; 3], slot: usize| -> Option<&[u32]> {
        let b = store.words.get(&chunk)?;
        let d = b[slot];
        if d == 0 {
            return None;
        }
        let run = (d >> 11) as usize;
        Some(&b[run - record..run])
    };
    for (slot, _) in &faces.bricks {
        let touched = touched.contains(slot);
        let slot = usize::from(*slot);
        if let Some(r) = record_at(c, slot) {
            let mut r = r.to_vec();
            if touched && record == BRICK_WORDS {
                fill_directions(&mut r);
            }
            block.extend(r);
            continue;
        }
        let mut guess = vec![0u32; record];
        if record == BRICK_WORDS {
            let b = brick_of_slot(slot);
            let g = [0, 1, 2].map(|k| c[k] * 8 + b[k]);
            for r in 1..=2 {
                let mut sums = [[0f64; 3]; 6];
                let mut counts = [0u32; 6];
                let mut material = None;
                for dz in -r..=r {
                    for dy in -r..=r {
                        for dx in -r..=r {
                            if dx == 0 && dy == 0 && dz == 0 {
                                continue;
                            }
                            let n = [g[0] + dx, g[1] + dy, g[2] + dz];
                            let nc = n.map(|v| v.div_euclid(8));
                            let Some(rec) = record_at(nc, brick_slot(n.map(|v| v.rem_euclid(8))))
                            else {
                                continue;
                            };
                            let mask = rec[6] >> 16;
                            material.get_or_insert(rec[6] & 0xFFFF);
                            for f in 0..6 {
                                if mask & 1 << f != 0 {
                                    let v = rgb9e5_unpack(rec[f]);
                                    for k in 0..3 {
                                        sums[f][k] += f64::from(v[k]);
                                    }
                                    counts[f] += 1;
                                }
                            }
                        }
                    }
                }
                let Some(material) = material else {
                    continue;
                };
                // A direction no brick around holds (a dug hole's faces point where the wall's
                // did not) takes the mean of every direction they hold.
                let n_all: u32 = counts.iter().sum();
                let all = [0, 1, 2].map(|k| {
                    (sums.iter().map(|d| d[k]).sum::<f64>() / f64::from(n_all.max(1))) as f32
                });
                for f in 0..6 {
                    guess[f] = if counts[f] > 0 {
                        let n = f64::from(counts[f]);
                        rgb9e5_pack(sums[f].map(|x| (x / n) as f32))
                    } else {
                        rgb9e5_pack(all)
                    };
                }
                guess[6] = material | 0x3F << 16;
                break;
            }
        }
        block.extend(guess);
    }
    debug_assert_eq!(block.len() as u64, words);
    block
}

/// The block a first block fades from under a fallback mode (ADR-0053): `block`'s layout and
/// tails, every cell's direction values the chain's `chain` for that direction (0 where the chain
/// holds none).
pub(super) fn fallback_block(block: &[u32], chain: &[u32; BRICK_WORDS], record: usize) -> Vec<u32> {
    let mut from = block.to_vec();
    let mask = chain[6] >> 16;
    for &dir in &block[..CHUNK_BRICKS] {
        if dir == 0 {
            continue;
        }
        let run = (dir >> 11) as usize;
        for cell in (run - record..run).step_by(BRICK_WORDS) {
            for f in 0..6 {
                from[cell + f] = if mask & 1 << f != 0 { chain[f] } else { 0 };
            }
        }
    }
    from
}

/// A fade's block at `t` of the way from `from` to `to` (one layout): `to`'s words, each cell's
/// direction values mixed in linear light and packed again. The fades write it each frame, and a
/// block landing on a running fade starts its own from it.
pub(super) fn fade_mix(shown: &mut Vec<u32>, from: &[u32], to: &[u32], t: f32, record: usize) {
    let lerp = |a: u32, b: u32| {
        let (x, y) = (rgb9e5_unpack(a), rgb9e5_unpack(b));
        rgb9e5_pack([0, 1, 2].map(|k| x[k] + (y[k] - x[k]) * t))
    };
    shown.clear();
    shown.extend_from_slice(to);
    for &dir in &to[..CHUNK_BRICKS] {
        if dir == 0 {
            continue;
        }
        let run = (dir >> 11) as usize;
        for cell in (run - record..run).step_by(BRICK_WORDS) {
            for w in cell..cell + 6 {
                shown[w] = lerp(from[w], to[w]);
            }
        }
    }
}

/// A brick record's directions it holds no value for take the mean of those it does: a brick a
/// dig touched may have faces now where it had none.
pub(super) fn fill_directions(r: &mut [u32]) {
    let mask = r[6] >> 16;
    let held: Vec<[f32; 3]> = (0..6)
        .filter(|f| mask & 1 << f != 0)
        .map(|f| rgb9e5_unpack(r[f]))
        .collect();
    if held.is_empty() {
        return;
    }
    let n = held.len() as f32;
    let mean = [0, 1, 2].map(|k| held.iter().map(|v| v[k]).sum::<f32>() / n);
    for (f, w) in r.iter_mut().take(6).enumerate() {
        if mask & 1 << f == 0 {
            *w = rgb9e5_pack(mean);
        }
    }
    r[6] = (r[6] & 0xFFFF) | 0x3F << 16;
}

/// Whether `slot`'s record in `block` holds light in any direction (a brick with no record holds
/// none).
pub(super) fn brick_lit(block: &[u32], slot: u16, record: usize) -> bool {
    let dir = block[usize::from(slot)];
    if dir == 0 {
        return false;
    }
    let run = (dir >> 11) as usize;
    (run - record..run).step_by(BRICK_WORDS).any(|cell| {
        block[cell..cell + 6]
            .iter()
            .any(|&w| rgb9e5_unpack(w).iter().any(|&v| v > 0.0))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_directory_points_past_each_bricks_record() {
        let faces = ChunkFaces {
            bricks: vec![(3, vec![1, 2, 3]), (9, vec![4])],
            glass: Vec::new(),
        };
        // The directory's 512 words, then the summary's 7 (ADR-0053), then the records.
        let (d, words) = layout(&faces, record_words(8));
        assert_eq!(d.len(), 512 + 7, "the head: directory and summary");
        assert_eq!(d[3], (512 + 7 + 7) << 11);
        assert_eq!(d[9], (512 + 7 + 14) << 11);
        assert_eq!(d[4], 0);
        assert_eq!(words, 512 + 7 + 14);
        let (d, words) = layout(&faces, record_words(4));
        assert_eq!(d[9], (512 + 7 + 2 * 56) << 11);
        assert_eq!(words, 512 + 7 + 2 * 56);
    }

    /// A record of one cell: `dirs` (face, value) held, material `m`.
    fn cell_record(dirs: &[(usize, [f32; 3])], m: u32) -> [u32; BRICK_WORDS] {
        let mut r = [0u32; BRICK_WORDS];
        let mut mask = 0;
        for &(f, v) in dirs {
            r[f] = rgb9e5_pack(v);
            mask |= 1 << f;
        }
        r[6] = m | mask << 16;
        r
    }

    /// Whether `w` decodes within one `rgb9e5` step of `want`, channel by channel.
    fn within_a_step(w: u32, want: [f64; 3]) -> bool {
        let got = rgb9e5_unpack(w);
        let step = 2f64.powi((w >> 27) as i32 - 24);
        (0..3).all(|k| (f64::from(got[k]) - want[k]).abs() <= step * 1.0001)
    }

    /// ADR-0053's summary and parents on fixed records: a block's summary is the mean by
    /// direction over the cells holding it, its material the one most of them keep, its mask
    /// their union; a level-1 record over two present children of eight is their mean by
    /// direction; a cell with no present child holds no value, and freeing a child's block takes
    /// it out of its parents' means. The chain reads own, neighbours, level 1, level 2 per mode.
    #[test]
    fn a_block_summary_is_its_cells_mean_by_direction() {
        // Three bricks of one cell: +x on two (1.0 and 0.5 red), -y on one, material 5 twice.
        let faces = ChunkFaces {
            bricks: vec![(3, vec![1]), (9, vec![2]), (40, vec![3])],
            glass: Vec::new(),
        };
        let (mut block, _) = layout(&faces, BRICK_WORDS);
        block.extend(cell_record(&[(1, [1.0, 0.25, 0.0])], 5));
        block.extend(cell_record(
            &[(1, [0.5, 0.75, 0.0]), (2, [0.0, 0.0, 2.0])],
            5,
        ));
        block.extend(cell_record(&[(2, [0.0, 0.0, 1.0])], 7));
        let s = block_summary(&block, BRICK_WORDS).expect("a lit block");
        assert!(
            within_a_step(s[1], [0.75, 0.5, 0.0]),
            "+x: {:?}",
            rgb9e5_unpack(s[1])
        );
        assert!(
            within_a_step(s[2], [0.0, 0.0, 1.5]),
            "-y: {:?}",
            rgb9e5_unpack(s[2])
        );
        assert_eq!(s[6] & 0xFFFF, 5, "the material most cells keep");
        assert_eq!(s[6] >> 16, 0b110, "the union of the masks");
        assert_eq!([s[0], s[3], s[4], s[5]], [0; 4], "directions no cell holds");
        // A block whose cells hold no direction has no summary.
        let (mut dark, _) = layout(&faces, BRICK_WORDS);
        dark.extend([0u32; 3 * BRICK_WORDS]);
        assert_eq!(block_summary(&dark, BRICK_WORDS), None);
        // Cells of 4 voxels: every cell of a brick's eight counts.
        let one = ChunkFaces {
            bricks: vec![(0, vec![1])],
            glass: Vec::new(),
        };
        let (mut b4, _) = layout(&one, record_words(4));
        for i in 0..8 {
            b4.extend(cell_record(&[(0, [i as f32, 0.0, 0.0])], 2));
        }
        let s4 = block_summary(&b4, record_words(4)).expect("a lit block");
        assert!(within_a_step(s4[0], [3.5, 0.0, 0.0]), "cells of 4");

        // Two present children of level-1 cell (0,0,0): their mean by direction.
        let a1 = cell_record(&[(1, [1.0, 0.0, 0.0]), (3, [0.5, 0.5, 0.5])], 5);
        let a2 = cell_record(&[(1, [0.0, 1.0, 0.0])], 5);
        let mut l = LightSummaries::default();
        let w = l.set([0, 0, 0], Some(a1)).expect("a change");
        assert_eq!(w[0].0, 1);
        assert_eq!(w[0].1, [0, 0, 0]);
        assert_eq!(w[0].2, a1, "level 1 over one child is that child");
        l.set([0, 0, 1], Some(a2)).expect("a change");
        assert_eq!(
            l.set([0, 0, 1], Some(a2)),
            None,
            "an unchanged summary writes nothing"
        );
        let p1 = *l.parent(1, [0, 0, 0]).expect("level 1 holds a value");
        assert!(
            within_a_step(p1[1], [0.5, 0.5, 0.0]),
            "+x: the two children's mean"
        );
        assert!(
            within_a_step(p1[3], [0.5, 0.5, 0.5]),
            "+y: the one child holding it"
        );
        assert_eq!(p1[6] >> 16, 0b1010);
        let p2 = *l.parent(2, [0, 0, 0]).expect("level 2 holds a value");
        assert_eq!(
            p2, p1,
            "level 2 over one level-1 cell is that cell's record"
        );
        // A cell with no present child holds no value.
        assert_eq!(l.parent(1, [1, 0, 0]), None);
        assert_eq!(l.parent(1, [-1, 0, 0]), None);

        // The chain for blockless chunks: B (1,0,0) touches A1 alone; C (1,1,0) neither but
        // shares level-1 cell (0,0,0); D (4,0,0) lies in a level-2 cell with no block.
        let b = l
            .chain([1, 0, 0], MissingLight::Chain)
            .expect("B's neighbour");
        assert_eq!(b[1], a1[1], "B under chain: A1's +x");
        assert_eq!(b[3], a1[3], "B under chain: A1's +y, not the parent's");
        let c = l.chain([1, 1, 0], MissingLight::Chain).expect("C's parent");
        assert_eq!(c[1], p1[1], "C under chain: level 1's");
        assert_eq!(
            l.chain([4, 0, 0], MissingLight::Chain),
            None,
            "D under chain"
        );
        let bp = l
            .chain([1, 0, 0], MissingLight::Parent)
            .expect("B's parent");
        assert_eq!(bp[1], p1[1], "B under parent: level 1's");
        assert_eq!(
            l.chain([1, 1, 0], MissingLight::Neighbour),
            None,
            "C under neighbour"
        );
        assert_eq!(
            l.chain([1, 0, 0], MissingLight::Black),
            None,
            "B under black"
        );
        assert_eq!(
            l.chain([0, 0, 0], MissingLight::Neighbour).map(|r| r[1]),
            Some(a1[1]),
            "a chunk with a block reads its own summary first"
        );

        // Freeing A1's block takes it out of its parents.
        l.set([0, 0, 0], None).expect("a change");
        assert_eq!(l.parent(1, [0, 0, 0]), Some(&a2), "level 1 without A1");
        l.set([0, 0, 1], None).expect("a change");
        assert_eq!(l.parent(1, [0, 0, 0]), None, "level 1 with no child");
        assert_eq!(l.parent(2, [0, 0, 0]), None, "level 2 with no child");
    }

    /// The parent table's cells cover the window at any alignment, levels 1 and 2 apart and
    /// after the chunk table; `temporal.wgsl` reads it with the same constants.
    #[test]
    fn the_parent_table_covers_the_window() {
        for (min, size) in [
            ([0, 0, 0], [3, 3, 3]),
            ([-5, 1, 7], [51, 9, 2]),
            ([1, 1, 1], [2, 2, 2]),
        ] {
            let window = ChunkExtent::new(min, [0, 1, 2].map(|k| min[k] + size[k]));
            let t = ParentTable::of(window);
            let chunks = (size[0] * size[1] * size[2]) as u64;
            assert_eq!(t.first, STORE_TABLE + chunks);
            let mut seen = std::collections::HashSet::new();
            for z in 0..size[2] {
                for y in 0..size[1] {
                    for x in 0..size[0] {
                        let c = [min[0] + x, min[1] + y, min[2] + z];
                        for level in 1..=2 {
                            let w = t.word(level, parent_of(c, level)).expect("inside");
                            assert!(w >= t.first && w + BRICK_WORDS as u64 <= t.first + t.words());
                            assert_eq!((w - t.first) % BRICK_WORDS as u64, 0);
                            seen.insert((level, w));
                        }
                    }
                }
            }
            let l1: Vec<u64> = seen.iter().filter(|e| e.0 == 1).map(|e| e.1).collect();
            let l2_min = seen
                .iter()
                .filter(|e| e.0 == 2)
                .map(|e| e.1)
                .min()
                .expect("level 2");
            assert!(l1.iter().all(|&w| w < l2_min), "level 1 before level 2");
        }
        let wgsl = include_str!("../shaders/temporal.wgsl");
        assert!(wgsl.contains(&format!("const STORE_SUMMARY_AT: u32 = {SUMMARY_AT}u;")));
        assert!(wgsl.contains("(store[STORE_FLAGS] >> 2u) & 3u"));
        assert_eq!(FLAG_MISSING_SHIFT, 2);
        assert_eq!(MissingLight::Neighbour.bits(), 1);
        assert_eq!(MissingLight::Parent.bits(), 2);
        assert!(wgsl.contains("const MISSING_NEIGHBOUR: u32 = 1u;"));
        assert!(wgsl.contains("const MISSING_PARENT: u32 = 2u;"));
    }
}
