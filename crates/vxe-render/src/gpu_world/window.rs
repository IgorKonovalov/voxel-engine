//! The window and its queue: chunks waiting for an upload nearest the focus first, the
//! window's moves, and the selection and upload of each sync.

use super::*;

/// The window, the chunk table wrapped into it, and the chunks waiting for an upload.
pub(super) struct WindowQueue {
    pub(super) min: [i32; 3],
    pub(super) size: [i32; 3],
    pub(super) focus: ChunkCoord,
    pub(super) initialized: bool,
    /// CPU copy of the chunk table, and the indices changed since the last write.
    pub(super) table: Vec<u32>,
    pub(super) table_changed: Vec<u32>,
    /// Chunks waiting for an upload, keyed by squared distance to `pending_focus`.
    pub(super) pending: BTreeSet<(i64, ChunkCoord)>,
    pub(super) pending_d2: HashMap<ChunkCoord, i64>,
    pub(super) pending_focus: ChunkCoord,
    /// The chunks whose mirror state changed since the last [`GpuWorld::take_changes`].
    pub(super) changes: std::sync::Mutex<Changes>,
    /// Kept sizes of pending chunks ([`Sizing`]).
    pub(super) sizes: HashMap<ChunkCoord, Sizing>,
    /// The chunks `select` chose and the stale ones it unqueues, cleared at the start of
    /// each `select` and kept for their capacity.
    pub(super) selected: Vec<Selected>,
    pub(super) stale: Vec<ChunkCoord>,
}

impl WindowQueue {
    /// A window at `min` of `size` chunks, around `focus`, with every one of its `n` table
    /// entries `ABSENT`.
    pub(super) fn new(min: [i32; 3], size: [i32; 3], focus: ChunkCoord, n: usize) -> Self {
        Self {
            min,
            size,
            focus,
            initialized: false,
            table: vec![ABSENT; n],
            table_changed: Vec::new(),
            pending: BTreeSet::new(),
            pending_d2: HashMap::new(),
            pending_focus: focus,
            changes: Default::default(),
            sizes: HashMap::new(),
            selected: Vec::new(),
            stale: Vec::new(),
        }
    }
}

/// The clock the sync's time budget reads: time since some fixed start; and its reading at
/// the current sync's start.
pub(super) struct SyncTimer {
    pub(super) read: SyncClock,
    pub(super) start: std::time::Duration,
}

impl SyncTimer {
    pub(super) fn new() -> Self {
        Self {
            read: wall_clock(),
            start: std::time::Duration::ZERO,
        }
    }
}

/// What a chosen chunk is uploaded from.
pub(super) enum Src {
    Full(std::sync::Arc<std::sync::RwLock<vxe_world::Chunk>>),
    Lod(std::sync::Arc<ChunkMips>),
}

/// A pending chunk's size as selection estimated it, kept until the chunk uploads, leaves the
/// queue or changes (the world reports it dirty, or it changes tier or record), so a chunk
/// deferred by the time budget is not sized again. Only selection reads it: the upload's
/// backstop sizes the chunk again under its write lock, exactly, so a kept size that went
/// stale (a neighbour's edit moving a band) can defer a chunk for space, never mis-write it.
#[derive(Clone, Copy)]
pub(super) struct Sizing {
    pub(super) lod: bool,
    pub(super) rec: Option<u32>,
    /// New slots of each bin, and the bytes the upload writes, record included.
    pub(super) new: [u64; BIN_COUNT],
    pub(super) bytes: u64,
}

/// A chunk chosen for this sync's upload.
pub(super) struct Selected {
    pub(super) coord: ChunkCoord,
    pub(super) src: Src,
    /// New slots of each bin its upload takes.
    pub(super) new: [u64; BIN_COUNT],
}

/// Changes [`GpuWorld::take_changes`] keeps at most between two calls.
pub const CHANGES_MAX: usize = 1 << 16;

/// The mirror's changed chunks between two [`GpuWorld::take_changes`]: kept once a reader took
/// them once, and given up past [`CHANGES_MAX`].
#[derive(Debug, Default)]
pub(super) struct Changes {
    pub(super) on: bool,
    pub(super) overflow: bool,
    pub(super) list: Vec<ChunkCoord>,
}

/// The chunks held (`held(c)`) that leave the window when it moves from `old` to `new`. Every
/// held chunk lies in `old`. When the slab `old - new` has no more chunks than the `count`
/// held, it is looked up chunk by chunk; otherwise the held chunks (`keys`, possibly with
/// repeats) are filtered. Either way the result holds each leaving chunk once.
pub(super) fn leaving_window<K: Iterator<Item = ChunkCoord>>(
    old: ChunkExtent,
    new: ChunkExtent,
    count: usize,
    held: impl Fn(ChunkCoord) -> bool,
    keys: impl FnOnce() -> K,
) -> Vec<ChunkCoord> {
    let overlap: i64 = (0..3)
        .map(|k| {
            let lo = old.min.0[k].max(new.min.0[k]);
            let hi = old.max.0[k].min(new.max.0[k]);
            i64::from((hi - lo).max(0))
        })
        .product();
    let volume: i64 = old.size().iter().map(|&v| i64::from(v)).product();
    if (volume - overlap) as u64 <= count as u64 {
        box_difference(old, new)
            .into_iter()
            .filter(|&c| held(c))
            .collect()
    } else {
        let set: HashSet<ChunkCoord> = keys().filter(|&c| !new.contains(c)).collect();
        set.into_iter().collect()
    }
}

/// The chunks of `a` that are not in `b`.
pub(super) fn box_difference(a: ChunkExtent, b: ChunkExtent) -> Vec<ChunkCoord> {
    let inside = |k: usize, v: i32| v >= b.min.0[k] && v < b.max.0[k];
    let mut out = Vec::new();
    for x in a.min.0[0]..a.max.0[0] {
        for y in a.min.0[1]..a.max.0[1] {
            if inside(0, x) && inside(1, y) {
                for z in a.min.0[2]..a.max.0[2] {
                    if !inside(2, z) {
                        out.push(ChunkCoord([x, y, z]));
                    }
                }
            } else {
                for z in a.min.0[2]..a.max.0[2] {
                    out.push(ChunkCoord([x, y, z]));
                }
            }
        }
    }
    out
}

impl GpuWorld {
    pub(super) fn enqueue(&mut self, c: ChunkCoord) {
        if self.window.pending_d2.contains_key(&c) {
            return;
        }
        self.note_change(c);
        self.enqueue_keyed(c);
    }

    /// Queues `c` under its distance to `pending_focus`, as no change: the re-keying of a
    /// focus move.
    pub(super) fn enqueue_keyed(&mut self, c: ChunkCoord) {
        let d = c.distance2(self.window.pending_focus);
        self.window.pending_d2.insert(c, d);
        self.window.pending.insert((d, c));
    }

    pub(super) fn unqueue(&mut self, c: ChunkCoord) {
        self.window.sizes.remove(&c);
        if let Some(d) = self.window.pending_d2.remove(&c) {
            self.window.pending.remove(&(d, c));
            self.note_change(c);
        }
    }

    /// Notes that `c`'s mirror state changed (mirrored, dropped, its tier, queued or uploaded),
    /// once a reader takes the changes.
    pub(super) fn note_change(&self, c: ChunkCoord) {
        let mut ch = self
            .window
            .changes
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if ch.on && !ch.overflow {
            if ch.list.len() >= CHANGES_MAX {
                ch.overflow = true;
                ch.list = Vec::new();
            } else {
                ch.list.push(c);
            }
        }
    }

    /// The chunks whose mirror state changed since the last call: mirrored, dropped, turned
    /// LOD-only or full, queued for an upload or uploaded, in no order and possibly repeated.
    /// `None` at the first call, and after more than [`CHANGES_MAX`] changes went untaken:
    /// anything may have changed.
    pub fn take_changes(&self) -> Option<Vec<ChunkCoord>> {
        let mut ch = self
            .window
            .changes
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let out = (ch.on && !ch.overflow).then(|| std::mem::take(&mut ch.list));
        *ch = Changes {
            on: true,
            overflow: false,
            list: Vec::new(),
        };
        out
    }

    /// Moves the window to follow the focus, and re-keys the pending set when the focus
    /// chunk changed. Chunks leaving the window are dropped; resident chunks entering it are
    /// queued.
    pub(super) fn reconcile_window(&mut self, world: &World) {
        if self.window.pending_focus != self.window.focus {
            self.window.pending_focus = self.window.focus;
            let keys: Vec<ChunkCoord> = self.window.pending_d2.keys().copied().collect();
            self.window.pending.clear();
            self.window.pending_d2.clear();
            for c in keys {
                self.enqueue_keyed(c);
            }
        }
        let new_min = match self.cfg.window {
            Window::Fixed(e) => e.min.0,
            Window::Follow { side } => self.window.focus.0.map(|v| v - side.max(1) / 2),
        };
        if !self.window.initialized {
            self.window.initialized = true;
            self.window.min = new_min;
            for c in world.store().coords().into_iter().chain(world.lod_coords()) {
                if self.in_window(c) {
                    self.enqueue(c);
                }
            }
            return;
        }
        if new_min == self.window.min {
            return;
        }
        let old = self.window();
        let new = ChunkExtent::new(new_min, [0, 1, 2].map(|k| new_min[k] + self.window.size[k]));
        let leaving = leaving_window(
            old,
            new,
            self.recs.mirrored.len() + self.window.pending_d2.len(),
            |c| self.recs.mirrored.contains_key(&c) || self.window.pending_d2.contains_key(&c),
            || {
                self.recs
                    .mirrored
                    .keys()
                    .chain(self.window.pending_d2.keys())
                    .copied()
            },
        );
        // Dropped while the window is still the old one, so their table entries are cleared;
        // the entering chunks wrap onto the same entries.
        for c in leaving {
            self.drop_chunk(c);
        }
        self.window.min = new_min;
        for c in box_difference(new, old) {
            if world.store().contains(c) || world.lod(c).is_some() {
                self.enqueue(c);
            }
        }
    }

    /// Drops the kept size of `c`, which changed, and with the band on those of its 26
    /// neighbours, whose band masks read it.
    pub(super) fn forget_size(&mut self, c: ChunkCoord) {
        if self.window.sizes.is_empty() {
            return;
        }
        if self.field.band.is_none() {
            self.window.sizes.remove(&c);
            return;
        }
        for k in 0..27 {
            let d = [k % 3 - 1, (k / 3) % 3 - 1, k / 9 - 1];
            self.window
                .sizes
                .remove(&ChunkCoord([0, 1, 2].map(|a| c.0[a] + d[a])));
        }
    }

    /// Whether the sync's time budget is spent, reading the clock at `at`.
    pub(super) fn time_spent(&mut self, at: SyncPoint) -> bool {
        self.cfg
            .sync_budget_time
            .is_some_and(|budget| (self.clock.read)(at).saturating_sub(self.clock.start) >= budget)
    }

    /// Walks the pending set nearest first and picks chunks into `self.window.selected` until the
    /// estimated bytes reach the budget, replaying the fit as it goes ([`Fit`]): a chunk
    /// changing tier is credited its old record's frees, then its new slots and record are
    /// checked against what is left. The first chunk that does not fit is deferred, with every
    /// later chunk but a demotion that fits; deferred chunks stay pending and do not count
    /// against the budget. A chunk with a kept size ([`Sizing`]) is not sized again; once the
    /// time budget is spent no further chunk is sized, and the walk stops there. Returns the
    /// error of the first deferral, if any.
    pub(super) fn select(&mut self, world: &World, st: &mut SyncStats) -> Option<MirrorError> {
        let budget = self.cfg.sync_budget_bytes;
        // Writes that happen whatever is chosen count against the budget first.
        let mut bytes = self.window.table_changed.len() as u64 * 4;
        if !self.mats.written {
            bytes += (world.materials().len() * MATERIAL_FLOATS * 4) as u64;
        }
        self.window.selected.clear();
        self.window.stale.clear();
        let mut fit = Fit::new(self);
        let mut short = None;
        let mut bands = std::mem::take(&mut self.field.bands);
        let mut sizes = std::mem::take(&mut self.window.sizes);
        let mut out_of_time = false;
        let pending = std::mem::take(&mut self.window.pending);
        for &(_, coord) in &pending {
            if bytes >= budget && !self.window.selected.is_empty() {
                break;
            }
            st.chunks_visited += 1;
            // The full chunk wins over mips: a promoted chunk replaces its LOD record.
            let src = match world.store().get(coord) {
                Some(chunk) => Src::Full(chunk),
                None => match world.lod(coord) {
                    Some(mips) => Src::Lod(mips),
                    None => {
                        self.window.stale.push(coord);
                        continue;
                    }
                },
            };
            let mirrored = self.recs.mirrored.get(&coord);
            let lod = matches!(src, Src::Lod(_));
            // A record of the other tier is replaced whole.
            let rec = mirrored.filter(|m| m.lod == lod).map(|m| m.record);
            let demotion = mirrored.is_some_and(|m| !m.lod) && lod;
            // The frees of a tier change: the old record's slots.
            let frees = mirrored.filter(|m| m.lod != lod).map(|m| m.record);
            let new_record = mirrored.is_none();
            let kept = sizes
                .get(&coord)
                .filter(|z| z.lod == lod && z.rec == rec)
                .copied();
            let stale = match &src {
                Src::Full(chunk) => {
                    rec.is_some()
                        && !chunk
                            .read()
                            .unwrap_or_else(PoisonError::into_inner)
                            .any_dirty()
                }
                // Mips never change: a chunk already mirrored LOD-only needs nothing.
                Src::Lod(_) => rec.is_some(),
            };
            if stale {
                self.window.stale.push(coord);
                continue;
            }
            let sizing = match kept {
                Some(z) => {
                    st.kept_sizes += 1;
                    z
                }
                None => {
                    if out_of_time {
                        break;
                    }
                    let (new, need_bytes) = match &src {
                        Src::Full(chunk) => {
                            let ch = chunk.read().unwrap_or_else(PoisonError::into_inner);
                            self.need(world, coord, &Bricks::Chunk(&ch), rec, &mut bands)
                        }
                        Src::Lod(mips) => {
                            self.need(world, coord, &Bricks::Mips(mips), None, &mut bands)
                        }
                    };
                    let z = Sizing {
                        lod,
                        rec,
                        new,
                        bytes: RECORD_BYTES + if rec.is_none() { 4 } else { 0 } + need_bytes,
                    };
                    sizes.insert(coord, z);
                    st.sized_chunks += 1;
                    out_of_time = self.time_spent(SyncPoint::Sized);
                    z
                }
            };
            let (new, chunk_bytes) = (sizing.new, sizing.bytes);
            if short.is_some() && !demotion {
                st.deferred_chunks += 1;
                continue;
            }
            match fit.place(self, frees, new, new_record) {
                Ok(()) => {
                    bytes += chunk_bytes;
                    self.window.selected.push(Selected { coord, src, new });
                }
                Err(e) => {
                    st.deferred_chunks += 1;
                    short.get_or_insert(e);
                }
            }
        }
        self.field.bands = bands;
        self.window.sizes = sizes;
        self.window.pending = pending;
        let mut stale = std::mem::take(&mut self.window.stale);
        for &c in &stale {
            self.unqueue(c);
        }
        stale.clear();
        self.window.stale = stale;
        short
    }

    /// Uploads the chunks `select` chose, stopping at the first failed allocation, and
    /// empties `self.window.selected`. Each chunk is checked against the allocator as it is then
    /// before anything of it is written or its old record dropped: one that does not fit is
    /// deferred whole, and so is every later chunk but a demotion.
    pub(super) fn upload(
        &mut self,
        world: &World,
        short: &mut Option<MirrorError>,
        enc: &mut wgpu::CommandEncoder,
        st: &mut SyncStats,
    ) -> Result<(), MirrorError> {
        let mut selected = std::mem::take(&mut self.window.selected);
        let mut result = Ok(());
        let mut defer_rest = false;
        let mut out_of_time = false;
        for Selected { coord, src, new } in selected.drain(..) {
            if out_of_time {
                st.time_deferred_chunks += 1;
                continue;
            }
            let lod = matches!(src, Src::Lod(_));
            let mirrored = self.recs.mirrored.get(&coord);
            let demotion = lod && mirrored.is_some_and(|m| !m.lod);
            if defer_rest && !demotion {
                st.deferred_chunks += 1;
                continue;
            }
            let frees = mirrored.filter(|m| m.lod != lod).map(|m| m.record);
            let new_record = mirrored.is_none();
            let rec = mirrored.filter(|m| m.lod == lod).map(|m| m.record);
            // A full chunk is held under its write lock from the backstop to its upload, so
            // the need checked is the need written.
            let mut guard = match &src {
                Src::Full(chunk) => Some(chunk.write().unwrap_or_else(PoisonError::into_inner)),
                Src::Lod(_) => None,
            };
            let mut bands = std::mem::take(&mut self.field.bands);
            let need = match &guard {
                Some(ch) => {
                    self.need(world, coord, &Bricks::Chunk(ch), rec, &mut bands)
                        .0
                }
                None => {
                    bands.clear();
                    new
                }
            };
            self.field.bands = bands;
            // The backstop: this chunk alone against the allocator as it is now. It must be
            // exact for every need it admits, so that no admitted chunk fails part-way: the
            // state is exact since a chunk's own tier-change frees precede its allocations,
            // and a moved brick's free, which follows its allocation, is never credited.
            if let Err(e) = Fit::new(self).place(self, frees, need, new_record) {
                short.get_or_insert(e);
                st.deferred_chunks += 1;
                defer_rest = true;
                continue;
            }
            if self.recs.mirrored.get(&coord).is_some_and(|m| m.lod != lod) {
                self.drop_record(coord);
            }
            result = match (&src, guard.as_mut()) {
                (_, Some(ch)) => {
                    let r = self.upload_record(coord, Bricks::Chunk(ch), enc, st);
                    if r.is_ok() {
                        ch.clear_dirty();
                    }
                    r
                }
                (Src::Lod(mips), None) => self.upload_record(coord, Bricks::Mips(mips), enc, st),
                (Src::Full(_), None) => unreachable!("a full chunk is locked above"),
            };
            if result.is_err() {
                break;
            }
            // The time budget is read after each chunk written: the chunk that crosses it is
            // whole, and the rest stay pending, nearest first, with their kept sizes, for the
            // next sync. A sync whose selection spent the budget still writes its first chunk.
            out_of_time = self.time_spent(SyncPoint::Uploaded);
        }
        self.window.selected = selected;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 50 seeded shifts of -2..=2 chunks per axis of a 7-chunk window holding a seeded mix
    /// of mirrored and pending chunks: the slab lookup leaves the chunks the filter over every
    /// held key leaves, and clears the same wrapped table entries.
    #[test]
    fn the_window_slab_finds_the_chunks_the_key_filter_finds() {
        let side = 7;
        let mut z = 0x0023_0004_u64;
        let mut next = |n: u64| {
            z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
            vxe_world::hash::mix64(z) % n
        };
        let wrap = |c: ChunkCoord| {
            let r = c.0.map(|v| v.rem_euclid(side) as usize);
            r[0] + 7 * (r[1] + 7 * r[2])
        };
        let mut min = [0i32; 3];
        let mut slab_used = 0;
        for shift in 0..50 {
            let old = ChunkExtent::new(min, min.map(|v| v + side));
            let mut mirrored = HashSet::new();
            let mut pending = HashSet::new();
            for x in old.min.0[0]..old.max.0[0] {
                for y in old.min.0[1]..old.max.0[1] {
                    for zz in old.min.0[2]..old.max.0[2] {
                        let c = ChunkCoord([x, y, zz]);
                        match next(4) {
                            0 => {}
                            1 => {
                                mirrored.insert(c);
                            }
                            2 => {
                                pending.insert(c);
                            }
                            _ => {
                                mirrored.insert(c);
                                pending.insert(c);
                            }
                        }
                    }
                }
            }
            let d = [0; 3].map(|_| next(5) as i32 - 2);
            min = [0, 1, 2].map(|k| min[k] + d[k]);
            let new = ChunkExtent::new(min, min.map(|v| v + side));
            let count = mirrored.len() + pending.len();
            let got = leaving_window(
                old,
                new,
                count,
                |c| mirrored.contains(&c) || pending.contains(&c),
                || mirrored.iter().chain(pending.iter()).copied(),
            );
            let slab = box_difference(old, new).len();
            if slab <= count {
                slab_used += 1;
            }
            let got_set: HashSet<ChunkCoord> = got.iter().copied().collect();
            assert_eq!(got.len(), got_set.len(), "shift {shift}: a repeat");
            let want: HashSet<ChunkCoord> = mirrored
                .iter()
                .chain(pending.iter())
                .copied()
                .filter(|&c| !new.contains(c))
                .collect();
            assert_eq!(got_set, want, "shift {shift} by {d:?}");
            let table = |leave: &HashSet<ChunkCoord>| {
                let mut t = vec![None; 343];
                for &c in mirrored.iter().chain(pending.iter()) {
                    t[wrap(c)] = Some(c);
                }
                for &c in leave {
                    t[wrap(c)] = None;
                }
                t
            };
            assert_eq!(table(&got_set), table(&want), "shift {shift}");
        }
        assert!(slab_used >= 25, "the slab path ran {slab_used} of 50 times");
    }

    #[test]
    fn box_difference_is_the_entering_slab() {
        let a = ChunkExtent::new([1, 0, 0], [4, 3, 3]);
        let b = ChunkExtent::new([0, 0, 0], [3, 3, 3]);
        let d = box_difference(a, b);
        assert_eq!(d.len(), 9);
        assert!(d.iter().all(|c| c.0[0] == 3));
        assert!(box_difference(b, b).is_empty());
        assert_eq!(
            box_difference(a, ChunkExtent::new([9, 9, 9], [10, 10, 10])).len(),
            27
        );
    }
}
