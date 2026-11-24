//! The pool's allocator (bins, pages and their live counts), the records it hands out, and
//! the fit a sync's selection replays before anything is written.

use super::*;

/// The pool's pages and the allocator of each bin.
pub(super) struct PoolAlloc {
    /// The device's layout and the reservation: `reserved_units` and `reserved_records`.
    pub(super) layout: ShardLayout,
    /// The allocator of each bin.
    pub(super) bins: [BinAlloc; BIN_COUNT],
    /// The bin of every page handed out, `None` while it is free, and its live slots.
    pub(super) page_bin: Vec<Option<u8>>,
    pub(super) page_live: Vec<u16>,
    /// Pages handed out whose live slots fell to zero, taken lowest first by any bin.
    pub(super) free_pages: BTreeSet<u32>,
    /// The next page never handed out: the pool's high-water mark in pages.
    pub(super) next_page: u32,
}

impl PoolAlloc {
    pub(super) fn new(layout: ShardLayout) -> Self {
        Self {
            layout,
            bins: Default::default(),
            page_bin: Vec::new(),
            page_live: Vec::new(),
            free_pages: BTreeSet::new(),
            next_page: 0,
        }
    }
}

/// The chunk records: each mirrored chunk's, the CPU copy of their entries, and the free ones.
pub(super) struct Records {
    pub(super) mirrored: HashMap<ChunkCoord, Mirrored>,
    /// CPU copy of every reserved record's entries, `reserved_records * RECORD_WORDS`, zeroed
    /// at creation (untouched pages cost no RAM until a record is written).
    pub(super) entries: Vec<u32>,
    pub(super) free: Vec<u32>,
    pub(super) next: u32,
}

impl Records {
    pub(super) fn new(reserved: u32) -> Self {
        Self {
            mirrored: HashMap::new(),
            entries: vec![0; reserved as usize * RECORD_WORDS],
            free: Vec::new(),
            next: 0,
        }
    }

    /// Sets the brick bits of the record at word `base` from its entries; true when they are
    /// all clear.
    pub(super) fn write_bits(&mut self, base: usize) -> bool {
        let (bits, rest) = self.entries[base..base + RECORD_WORDS].split_at_mut(ENTRY_WORD);
        for (w, entries) in bits.iter_mut().zip(rest.chunks_exact(32)) {
            *w = entries
                .iter()
                .enumerate()
                .fold(0, |acc, (j, &e)| acc | u32::from(e != 0) << j);
        }
        bits.iter().all(|&w| w == 0)
    }
}

/// The slot allocator of one bin: a cursor through its current page and a free list.
#[derive(Debug, Default)]
pub(super) struct BinAlloc {
    /// The first unit of the next never-used slot of the current page, and the end of the
    /// page's last slot.
    pub(super) next: u32,
    pub(super) end: u32,
    /// Slots of this bin holding a brick.
    pub(super) live: u32,
    /// First units of free slots in pages of this bin that still hold a live slot.
    pub(super) free: Vec<u32>,
}

impl BinAlloc {
    /// Slots this bin can hand out without a new page.
    pub(super) fn available(&self, k: usize) -> u64 {
        u64::from((self.end - self.next) / SLOT_BINS[k]) + self.free.len() as u64
    }
}

/// One bin's occupancy ([`GpuWorld::bin_stats`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BinStats {
    /// Units per slot.
    pub units: u32,
    /// Slots holding a brick.
    pub live: u32,
    /// Slots handed out to no brick: the free list and the rest of the cursor's page.
    pub spare: u64,
    /// Pages of this bin.
    pub pages: u32,
}

/// What the mirror keeps for a mirrored chunk besides its record entries.
pub(super) struct Mirrored {
    pub(super) record: u32,
    /// A LOD record, from the chunk's mips.
    pub(super) lod: bool,
    /// Whether a brick uploaded into this record has held a material with emission: its
    /// bricks are then scanned for light samples whenever they or a neighbour change.
    pub(super) emits: bool,
}

/// The pool and records as a sync's selection leaves them, replayed chunk by chunk in the
/// order `upload` places them. A chunk changing tier first frees its old record's slots:
/// credited to their bin's spare slots, or, for a page those frees empty, as a free page less
/// the page's spare slots. Then its new slots take spare slots of their bin, then free pages,
/// then pages never handed out. No free is credited before the chunk that makes it. Once a
/// bin has taken spare slots (its free list or cursor), a later free of that bin is credited
/// as a slot, never as an emptied page: the allocator hands just-freed slots back first, so an
/// earlier allocation may sit on the page. A brick that moves bins inside a record frees its
/// old slot after taking its new one; that free is never credited.
pub(super) struct Fit {
    /// Spare slots of each bin.
    pub(super) avail: [u64; BIN_COUNT],
    pub(super) free_pages: u64,
    /// Pages past the pages handed out.
    pub(super) fresh: u64,
    pub(super) records: u64,
    /// Slots the placed chunks free, per page.
    pub(super) freed: HashMap<u32, u32>,
    /// Whether each bin has taken spare slots.
    pub(super) took: [bool; BIN_COUNT],
}

impl Fit {
    pub(super) fn new(gw: &GpuWorld) -> Self {
        Self {
            avail: std::array::from_fn(|k| gw.pool.bins[k].available(k)),
            free_pages: gw.pool.free_pages.len() as u64,
            fresh: 0,
            records: gw.recs.mirrored.len() as u64,
            freed: HashMap::new(),
            took: [false; BIN_COUNT],
        }
    }

    /// Places one chunk: `frees` (the record of a tier change's old record), `new` slots of
    /// each bin, and a new record when `new_record`. Changes nothing when the chunk does not
    /// fit, and returns what it would have needed.
    pub(super) fn place(
        &mut self,
        gw: &GpuWorld,
        frees: Option<u32>,
        new: [u64; BIN_COUNT],
        new_record: bool,
    ) -> Result<(), MirrorError> {
        let reserved = u64::from(gw.pool.layout.reserved_records);
        if new_record && self.records + 1 > reserved {
            return Err(MirrorError::RecordsTooLarge {
                needed: (self.records + 1) * RECORD_BYTES,
                limit: reserved * RECORD_BYTES,
            });
        }
        let (mut avail, mut free_pages, mut fresh) = (self.avail, self.free_pages, self.fresh);
        let mut took = self.took;
        let mut local: HashMap<u32, u32> = HashMap::new();
        if let Some(rec) = frees {
            let base = rec as usize * RECORD_WORDS;
            for &e in &gw.recs.entries[base + ENTRY_WORD..base + RECORD_WORDS] {
                if e & FULL_BIT == 0 {
                    continue;
                }
                let p = (e & UNIT_MASK) / PAGE_UNITS;
                let k =
                    gw.pool.page_bin[p as usize].expect("a live slot's page has a bin") as usize;
                let f = local.entry(p).or_insert(0);
                *f += 1;
                let total = *f + self.freed.get(&p).copied().unwrap_or(0);
                if !self.took[k] && u32::from(gw.pool.page_live[p as usize]) == total {
                    // The page empties: its spare slots, and the ones credited from it so
                    // far, leave the bin; the page becomes free for any bin.
                    let credited = total - 1;
                    avail[k] =
                        avail[k].saturating_sub(gw.pool.spare_in_page(k, p) + u64::from(credited));
                    free_pages += 1;
                } else {
                    avail[k] += 1;
                }
            }
        }
        for (k, &n) in new.iter().enumerate() {
            let take = n.min(avail[k]);
            avail[k] -= take;
            took[k] |= take > 0;
            let mut rest = n - take;
            let per = u64::from(slots_per_page(k));
            while rest > 0 {
                if free_pages > 0 {
                    free_pages -= 1;
                } else {
                    fresh += 1;
                }
                let used = rest.min(per);
                avail[k] += per - used;
                rest -= used;
            }
        }
        let need = u64::from(gw.pool.next_page) + fresh;
        let limit = u64::from(gw.pool.layout.reserved_pages());
        if need > limit {
            return Err(MirrorError::PoolTooLarge {
                needed: need * PAGE_BYTES,
                limit: limit * PAGE_BYTES,
            });
        }
        (self.avail, self.free_pages, self.fresh, self.took) = (avail, free_pages, fresh, took);
        for (p, n) in local {
            *self.freed.entry(p).or_insert(0) += n;
        }
        self.records += u64::from(new_record);
        Ok(())
    }
}

impl PoolAlloc {
    /// Spare slots of bin `k` in page `p`: its free slots, and the rest of the bin's cursor
    /// when that is in `p`.
    pub(super) fn spare_in_page(&self, k: usize, p: u32) -> u64 {
        let a = &self.bins[k];
        let free = a.free.iter().filter(|&&u| u / PAGE_UNITS == p).count() as u64;
        let cursor = if a.next < a.end && (a.end - 1) / PAGE_UNITS == p {
            u64::from((a.end - a.next) / SLOT_BINS[k])
        } else {
            0
        };
        free + cursor
    }

    /// Returns the slot at unit `u` to its bin's free list, or releases its page when it held
    /// the page's last live slot.
    pub(super) fn free_slot(&mut self, u: u32) {
        let p = u / PAGE_UNITS;
        let k = self.page_bin[p as usize].expect("a live slot's page has a bin") as usize;
        if self.unlive(u) {
            self.release_page(p);
            return;
        }
        self.bins[k].free.push(u);
    }

    /// Takes the slot at unit `u` off its page's and its bin's live counts; true when the page
    /// holds no live slot after it.
    pub(super) fn unlive(&mut self, u: u32) -> bool {
        let p = (u / PAGE_UNITS) as usize;
        let k = self.page_bin[p].expect("a live slot's page has a bin") as usize;
        self.page_live[p] -= 1;
        self.bins[k].live -= 1;
        self.page_live[p] == 0
    }

    /// Frees page `p`, which holds no live slot: its slots leave its bin's free list, the
    /// bin's cursor leaves it, and it joins the free pages.
    pub(super) fn release_page(&mut self, p: u32) {
        let k = self.page_bin[p as usize]
            .take()
            .expect("a page released once") as usize;
        let a = &mut self.bins[k];
        a.free.retain(|&u| u / PAGE_UNITS != p);
        if a.next < a.end && (a.end - 1) / PAGE_UNITS == p {
            a.next = a.end;
        }
        self.free_pages.insert(p);
    }

    /// Moves bin `k`'s cursor onto a page: the lowest free page, of any former bin, else the
    /// next page never handed out, failing with `PoolTooLarge` when that one lies past the
    /// reservation. The current page's unused slots go to the free list.
    pub(super) fn take_page(&mut self, k: usize) -> Result<(), MirrorError> {
        let page = match self.free_pages.pop_first() {
            Some(page) => page,
            None => {
                let limit = self.layout.reserved_pages();
                if self.next_page + 1 > limit {
                    return Err(MirrorError::PoolTooLarge {
                        needed: u64::from(self.next_page + 1) * PAGE_BYTES,
                        limit: u64::from(limit) * PAGE_BYTES,
                    });
                }
                self.page_bin.push(None);
                self.page_live.push(0);
                self.next_page += 1;
                self.next_page - 1
            }
        };
        self.page_bin[page as usize] = Some(k as u8);
        let b = SLOT_BINS[k];
        let a = &mut self.bins[k];
        // Pushed highest first, so they pop ascending.
        a.free.extend((a.next..a.end).step_by(b as usize).rev());
        a.next = page * PAGE_UNITS;
        a.end = a.next + slots_per_page(k) * b;
        Ok(())
    }

    /// One slot of bin `k`: a free one, else the cursor, else a new page.
    pub(super) fn alloc(&mut self, k: usize) -> Result<u32, MirrorError> {
        let a = &mut self.bins[k];
        let u = if let Some(u) = a.free.pop() {
            u
        } else {
            if a.next >= a.end {
                self.take_page(k)?;
            }
            let a = &mut self.bins[k];
            a.next += SLOT_BINS[k];
            a.next - SLOT_BINS[k]
        };
        self.page_live[(u / PAGE_UNITS) as usize] += 1;
        self.bins[k].live += 1;
        Ok(u)
    }

    /// The bin of the page holding unit `u`, `None` for a free or never-issued page.
    pub(super) fn bin_of_unit(&self, u: u32) -> Option<usize> {
        self.page_bin
            .get((u / PAGE_UNITS) as usize)
            .copied()
            .flatten()
            .map(usize::from)
    }
}

impl Records {
    /// A free record, or the next never-used one of the reservation. `limit` is the
    /// reservation's records; `sync` checks it before uploading, so the error here is a backstop.
    pub(super) fn alloc(&mut self, limit: u32) -> Result<u32, MirrorError> {
        if let Some(r) = self.free.pop() {
            return Ok(r);
        }
        if self.next >= limit {
            return Err(MirrorError::RecordsTooLarge {
                needed: (u64::from(self.next) + 1) * RECORD_BYTES,
                limit: u64::from(limit) * RECORD_BYTES,
            });
        }
        self.next += 1;
        Ok(self.next - 1)
    }
}
