//! The light-sample buffer (ADR-0025): each mirrored chunk's exposed emitters, and their
//! ranges of the buffer by the light radius.

use super::fit::FreeRanges;
use super::*;

/// The light-sample buffer and each chunk's samples.
pub(super) struct LightMirror {
    /// The buffer and its ranges.
    pub(super) buf: wgpu::Buffer,
    pub(super) ranges: Ranges,
    /// Ranges are held out to at least this many chunks of the focus ([`GpuWorld::hold_lights`]).
    pub(super) hold: i32,
    /// Each chunk's samples.
    pub(super) chunks: HashMap<ChunkCoord, ChunkLights>,
    pub(super) live: u64,
    /// Bumped whenever a chunk's range is written, released or dropped: what a reader of
    /// [`GpuWorld::light_chunks`] keys a cache on.
    pub(super) epoch: u64,
    /// Ranges of chunks dropped outside a sync, zeroed and freed at the next one.
    pub(super) dropped: Vec<(u32, u32)>,
    /// Chunks whose lists changed since their range was written; chunks inside the radius
    /// waiting for a range, with the sync they started waiting at; syncs so far.
    pub(super) dirty: HashSet<ChunkCoord>,
    pub(super) wait: HashMap<ChunkCoord, u64>,
    pub(super) syncs: u64,
    /// The focus chunk and held radius of `place_lights`' last full walk; `None` before it.
    pub(super) walked: Option<(ChunkCoord, i64)>,
    /// Whether an input of `place_lights`' walks changed since its last full walk: a chunk's
    /// lists or dirty mark, a chunk entering or leaving `chunks`, a range released, or a chunk
    /// the last walk refused.
    pub(super) stale: bool,
    /// Bumped with every change to `chunks`' membership or `dirty`; with `epoch`, the key of
    /// [`GpuWorld::lights_ready_near`]'s answers.
    pub(super) changes: u64,
    pub(super) ready: std::sync::Mutex<ReadyCache>,
    /// Every sync walks in full ([`GpuWorld::force_light_walks`]).
    pub(super) force_walks: bool,
    /// The write walks run so far ([`GpuWorld::light_walks`]).
    pub(super) walks: u64,
}

/// [`GpuWorld::lights_ready_near`]'s answers by chunk and reach, valid while `epoch` and
/// `changes` are those of `key`.
#[derive(Default)]
pub(super) struct ReadyCache {
    pub(super) key: (u64, u64),
    pub(super) answers: HashMap<(ChunkCoord, i32), bool>,
}

impl LightMirror {
    /// A buffer of `samples` light samples and the candidate table's slots past them.
    pub(super) fn new(device: &wgpu::Device, samples: u32) -> Self {
        Self {
            buf: GpuWorld::storage_buffer(
                device,
                "light_samples",
                (u64::from(samples) + LIGHT_TABLE_SLOTS) * LIGHT_SAMPLE_BYTES,
            ),
            ranges: Ranges::new(samples),
            hold: 0,
            chunks: HashMap::new(),
            live: 0,
            epoch: 0,
            dropped: Vec::new(),
            dirty: HashSet::new(),
            wait: HashMap::new(),
            syncs: 0,
            walked: None,
            stale: true,
            changes: 0,
            ready: Default::default(),
            force_walks: false,
            walks: 0,
        }
    }

    /// Notes a change to an input of the walks and of `lights_ready_near`.
    pub(super) fn touch(&mut self) {
        self.stale = true;
        self.changes += 1;
    }

    /// Notes range `r` released from its chunk (freed, its zeroing the caller's).
    fn released(&mut self, r: (u32, u32)) {
        self.touch();
        self.epoch += 1;
        self.live -= u64::from(r.1);
    }
}

/// The light-sample buffer's ranges: the free ranges below `high` (the first sample never
/// handed out), each placed chunk's range and power ([`GpuWorld::light_chunks`]), and the placed
/// chunks by squared chunk distance to `focus`, ties by coordinate (the eviction's index: its
/// last entry is the farthest placed chunk, the victim a scan of every placed chunk would pick).
/// The index is kept on placement and release, and rebuilt when an eviction asks for it at a
/// focus other than its own.
pub(super) struct Ranges {
    samples: u32,
    free: FreeRanges,
    high: u32,
    held: HashMap<ChunkCoord, (u32, u32, f32)>,
    by_distance: BTreeMap<(i64, [i32; 3]), ChunkCoord>,
    focus: Option<ChunkCoord>,
    /// The first fit and the victim by a walk of every free range and a scan of every placed
    /// chunk: the reference the indexed placement is tested against.
    #[cfg(test)]
    pub(super) linear: bool,
}

impl Ranges {
    /// No range placed in a buffer of `samples` samples.
    pub(super) fn new(samples: u32) -> Self {
        Self {
            samples,
            free: FreeRanges::new(samples),
            high: 0,
            held: HashMap::new(),
            by_distance: BTreeMap::new(),
            focus: None,
            #[cfg(test)]
            linear: false,
        }
    }

    /// `coord`'s range, start and length.
    pub(super) fn get(&self, coord: ChunkCoord) -> Option<(u32, u32)> {
        self.held.get(&coord).map(|&(s, n, _)| (s, n))
    }

    /// Sets placed chunk `coord`'s power.
    pub(super) fn set_power(&mut self, coord: ChunkCoord, power: f32) {
        if let Some(h) = self.held.get_mut(&coord) {
            h.2 = power;
        }
    }

    /// Free ranges below the high-water mark.
    pub(super) fn free_count(&self) -> usize {
        self.free.len()
    }

    /// The samples from 0 to the high-water mark.
    pub(super) fn high(&self) -> u32 {
        self.high
    }

    /// Whether `n` samples could be placed without an eviction: a free range holds them, or
    /// the samples above the high-water mark do. No samples always fit.
    pub(super) fn fits(&self, n: u32) -> bool {
        self.free.longest() >= n.max(1)
            || u64::from(self.high) + u64::from(n) <= u64::from(self.samples)
    }

    /// Places `n` samples of `coord`, which holds no range, at squared chunk distance `d2` from
    /// `focus`: the lowest free run that holds them, else the samples above the high-water mark;
    /// while neither fits, the farthest placed chunk beyond `d2` is released and pushed to
    /// `evicted` with its range. `None` (refused) when nothing farther is left. The first fit's
    /// steps go to `st.light_fit_steps`, the victims' to `st.light_victim_steps`.
    pub(super) fn place(
        &mut self,
        coord: ChunkCoord,
        n: u32,
        d2: i64,
        focus: ChunkCoord,
        evicted: &mut Vec<(ChunkCoord, (u32, u32))>,
        st: &mut SyncStats,
    ) -> Option<u32> {
        debug_assert!(!self.held.contains_key(&coord));
        loop {
            if let Some(start) = self.alloc(n, &mut st.light_fit_steps) {
                self.held.insert(coord, (start, n, 0.0));
                if let Some(f) = self.focus {
                    self.by_distance
                        .insert((coord.distance2(f), coord.0), coord);
                }
                return Some(start);
            }
            let v = self.victim(focus, d2, &mut st.light_victim_steps)?;
            let r = self.release(v).expect("a victim holds a range");
            evicted.push((v, r));
        }
    }

    /// The placed chunk farthest from `focus` (the greatest coordinate among the farthest),
    /// when it lies beyond `d2`.
    fn victim(&mut self, focus: ChunkCoord, d2: i64, steps: &mut u64) -> Option<ChunkCoord> {
        #[cfg(test)]
        if self.linear {
            *steps += self.held.len() as u64;
            return self
                .held
                .keys()
                .map(|c| (c.distance2(focus), c.0, *c))
                .filter(|&(d, _, _)| d > d2)
                .max()
                .map(|(_, _, c)| c);
        }
        if self.focus != Some(focus) {
            *steps += self.held.len() as u64;
            self.by_distance = self
                .held
                .keys()
                .map(|c| ((c.distance2(focus), c.0), *c))
                .collect();
            self.focus = Some(focus);
        }
        *steps += 1;
        self.by_distance
            .last_key_value()
            .filter(|((d, _), _)| *d > d2)
            .map(|(_, c)| *c)
    }

    /// Removes `coord`'s range from the placed chunks, without freeing it.
    pub(super) fn forget(&mut self, coord: ChunkCoord) -> Option<(u32, u32)> {
        let (s, n, _) = self.held.remove(&coord)?;
        if let Some(f) = self.focus {
            self.by_distance.remove(&(coord.distance2(f), coord.0));
        }
        Some((s, n))
    }

    /// Removes `coord`'s range from the placed chunks and frees it.
    pub(super) fn release(&mut self, coord: ChunkCoord) -> Option<(u32, u32)> {
        let r = self.forget(coord)?;
        self.free_range(r);
        Some(r)
    }

    /// The start of a free run of `n` samples: the lowest free range that holds it, else the
    /// samples above the high-water mark; `None` when the buffer has no such run.
    fn alloc(&mut self, n: u32, steps: &mut u64) -> Option<u32> {
        #[cfg(test)]
        let fit = if self.linear {
            self.free.first_fit_linear(n)
        } else {
            self.free.first_fit(n, steps)
        };
        #[cfg(not(test))]
        let fit = self.free.first_fit(n, steps);
        if let Some((start, len)) = fit {
            self.free.remove(start);
            if len > n {
                self.free.insert(start + n, len - n);
            }
            return Some(start);
        }
        let start = self.high;
        if u64::from(start) + u64::from(n) > u64::from(self.samples) {
            return None;
        }
        self.high += n;
        Some(start)
    }

    /// Returns a range to the free ranges, merged with its neighbours; a free range that
    /// reaches the high-water mark lowers it instead.
    pub(super) fn free_range(&mut self, (mut start, mut len): (u32, u32)) {
        if let Some((s, l)) = self.free.before(start) {
            self.free.remove(s);
            start = s;
            len += l;
        }
        if let Some(l) = self.free.get(start + len) {
            self.free.remove(start + len);
            len += l;
        }
        if start + len == self.high {
            self.high = start;
        } else {
            self.free.insert(start, len);
        }
    }

    /// The placed chunks and their ranges.
    pub(super) fn placed(&self) -> impl Iterator<Item = (ChunkCoord, (u32, u32))> + '_ {
        self.held.iter().map(|(c, &(s, n, _))| (*c, (s, n)))
    }

    /// The placed chunks: coordinate, start, length and power.
    pub(super) fn placed_power(&self) -> impl Iterator<Item = (ChunkCoord, u32, u32, f32)> + '_ {
        self.held.iter().map(|(c, &(s, n, p))| (*c, s, n, p))
    }

    /// The free ranges by start, for the tests.
    #[cfg(test)]
    pub(super) fn free_ranges(&self) -> Vec<(u32, u32)> {
        self.free.ranges().collect()
    }
}

/// A chunk `place_lights`' write walk wants written: its squared chunk distance to the focus,
/// whether it holds a range (a dirty placed chunk) and its thinned count (0 for empty lists).
pub(super) struct Wanted {
    pub(super) d2: i64,
    pub(super) coord: ChunkCoord,
    pub(super) placed: bool,
    pub(super) n: u32,
}

/// What [`write_walk`] does to the buffer and the light mirror's sets.
pub(super) trait WriteWalk {
    /// Writes `coord` at squared distance `d2`, its old range released first
    /// ([`GpuWorld::write_chunk_lights`]): the samples written, `None` when refused.
    fn write(&mut self, coord: ChunkCoord, d2: i64) -> Option<u64>;
    /// Whether `n` samples fit the buffer without an eviction ([`Ranges::fits`]).
    fn fits(&self, n: u32) -> bool;
    /// Counts `coord` refused without trying to place it.
    fn refuse(&mut self, coord: ChunkCoord);
    /// `coord` waits for the next sync's sample budget.
    fn wait(&mut self, coord: ChunkCoord);
    /// `coord` was walked: it leaves the dirty and waiting sets.
    fn walked(&mut self, coord: ChunkCoord);
}

/// The write walk over `wanted` (nearest first) until `budget` samples are written; the rest
/// wait. A refusal means no placed chunk lay farther than the refused one, and the walk only
/// places chunks at least as far after it, so for the rest of the walk no later chunk has a
/// victim (a victim lies strictly farther): a later unplaced chunk is placed only if a free
/// range or the headroom holds it. One that neither holds is counted refused without a fit or
/// a victim lookup; one that fits, and a placed chunk (a dirty one, whose release frees its
/// range), is written as before. The outcome is the full walk's.
pub(super) fn write_walk(wanted: &[Wanted], budget: u64, w: &mut impl WriteWalk) {
    let mut written = 0u64;
    let mut refused = false;
    for x in wanted {
        if written >= budget {
            w.wait(x.coord);
            continue;
        }
        if refused && !x.placed && !w.fits(x.n) {
            w.refuse(x.coord);
        } else {
            let wrote = w.write(x.coord, x.d2);
            refused |= wrote.is_none();
            written += wrote.unwrap_or(0);
        }
        w.walked(x.coord);
    }
}

/// [`WriteWalk`] on the mirror inside a sync.
struct SyncWalk<'a> {
    gw: &'a mut GpuWorld,
    enc: &'a mut wgpu::CommandEncoder,
    st: &'a mut SyncStats,
}

impl WriteWalk for SyncWalk<'_> {
    fn write(&mut self, coord: ChunkCoord, d2: i64) -> Option<u64> {
        self.gw.write_chunk_lights(coord, d2, self.enc, self.st)
    }

    fn fits(&self, n: u32) -> bool {
        self.gw.lights.ranges.fits(n)
    }

    fn refuse(&mut self, _: ChunkCoord) {
        self.st.light_refused += 1;
    }

    fn wait(&mut self, coord: ChunkCoord) {
        let syncs = self.gw.lights.syncs;
        self.gw.lights.wait.entry(coord).or_insert(syncs);
    }

    fn walked(&mut self, coord: ChunkCoord) {
        self.gw.lights.dirty.remove(&coord);
        self.gw.lights.wait.remove(&coord);
    }
}

/// Light samples [`MirrorConfig::new`] reserves, and the samples one chunk keeps at most.
pub const DEFAULT_LIGHT_SAMPLES: u32 = 1 << 20;
pub const DEFAULT_LIGHT_CHUNK_CAP: u32 = 8192;
/// The light radius in chunks and the light samples one sync writes, by default.
pub const DEFAULT_LIGHT_RADIUS: i32 = 4;
pub(crate) const DEFAULT_LIGHT_SYNC_SAMPLES: u32 = 16_384;

/// One exposed emissive voxel in the light-sample buffer, 16 bytes: its world voxel
/// coordinate, then its material in bits 0..16 and the faces it shows in bits 16..22 (bit
/// `16 + 2 k` the face toward -k, the next toward +k). Material 0 is no sample.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LightSample {
    pub voxel: [i32; 3],
    pub material_faces: u32,
}

impl LightSample {
    pub fn material(&self) -> MaterialId {
        (self.material_faces & 0xFFFF) as MaterialId
    }

    /// The faces the voxel shows: bit `2 k` toward -k, bit `2 k + 1` toward +k.
    pub fn faces(&self) -> u32 {
        self.material_faces >> 16
    }
}

/// Bytes of one [`LightSample`].
pub const LIGHT_SAMPLE_BYTES: u64 = 16;

/// Sample-sized slots past the reservation that hold the frame's candidate table: a header
/// and up to 1024 entries (`Frame` writes them; ADR-0025).
pub(crate) const LIGHT_TABLE_SLOTS: u64 = 1025;

/// The exposed emissive voxels of `Full` brick `brick` at brick slot `slot` of chunk `coord`,
/// in voxel-index order (x fastest); `emissive` and `opaque` are by material id. A voxel on
/// the brick's face looks across it into `neighbour(k, dir)`, the chunk's brick beside this
/// one on axis `k` toward `dir` (-1 or 1); with `None` (the chunk's face) that face counts
/// as exposed.
pub fn exposed_emitters<'a>(
    coord: ChunkCoord,
    slot: usize,
    brick: &Brick,
    emissive: &[bool],
    opaque: &[bool],
    neighbour: impl Fn(usize, i32) -> Option<&'a Brick>,
) -> Vec<LightSample> {
    scan_emitters(coord, slot, brick, emissive, opaque, neighbour).unwrap_or_default()
}

/// [`exposed_emitters`], `None` when the brick is not `Full` or its palette holds no emissive
/// material. The brick is read as masks ([`emitters::scan_masks`]), not decoded.
pub(super) fn scan_emitters<'a>(
    coord: ChunkCoord,
    slot: usize,
    brick: &Brick,
    emissive: &[bool],
    opaque: &[bool],
    neighbour: impl Fn(usize, i32) -> Option<&'a Brick>,
) -> Option<Vec<LightSample>> {
    let Brick::Full(p) = brick else {
        return None;
    };
    if !materials_of(p, |m| emissive[usize::from(m)]) {
        return None;
    }
    Some(emitters::scan_masks(
        coord, slot, p, emissive, opaque, neighbour,
    ))
}

/// The length of `n` samples thinned by [`thin_samples`] to at most `cap`.
pub(crate) fn thinned_len(n: usize, cap: u32) -> usize {
    let cap = cap.max(1) as usize;
    if n <= cap {
        n
    } else {
        n.div_ceil(n.div_ceil(cap))
    }
}

/// `samples` thinned to at most `cap` by keeping every `k`-th by index, `k` the least that
/// fits.
pub(crate) fn thin_samples(samples: Vec<LightSample>, cap: u32) -> Vec<LightSample> {
    let n = samples.len();
    let cap = cap.max(1) as usize;
    if n <= cap {
        return samples;
    }
    let k = n.div_ceil(cap);
    samples.into_iter().step_by(k).collect()
}

/// A chunk's light samples: each emissive brick's list by brick slot (its range of the
/// light-sample buffer is in [`Ranges`]).
#[derive(Default)]
pub(super) struct ChunkLights {
    pub(super) bricks: BTreeMap<u16, Vec<LightSample>>,
    /// Whether the chunk holds a range in [`Ranges`]: the walks' and the readiness checks'
    /// test without a lookup there.
    pub(super) placed: bool,
}

impl GpuWorld {
    /// Replaces the light lists of `coord`'s bricks in `rescan` with their exposed emissive
    /// voxels, slot by slot in ascending order. The chunk's lists are looked up once for the
    /// record, not per slot: a fresh record visits every slot, most of them with no emitter.
    pub(super) fn note_lights(
        &mut self,
        coord: ChunkCoord,
        rescan: RescanSet,
        bricks: &Bricks<'_>,
        st: &mut SyncStats,
    ) {
        let opaque: &[bool] = &self.mats.opaque;
        let emissive: &[bool] = &self.mats.emissive;
        let mut held = self.lights.chunks.get_mut(&coord);
        // The lists of a chunk that held none, inserted after the walk.
        let mut fresh: Option<ChunkLights> = None;
        for slot in rescan.iter() {
            let brick = bricks.get(slot);
            let found = if brick.is_full() {
                st.light_bricks += 1;
                let b = brick_of_slot(slot);
                let neighbour = |k: usize, dir: i32| {
                    let mut n = b;
                    n[k] += dir;
                    (0..CHUNK_EDGE_BRICKS)
                        .contains(&n[k])
                        .then(|| bricks.get(brick_slot(n)))
                };
                let found = scan_emitters(coord, slot, brick, emissive, opaque, neighbour);
                st.light_emitter_bricks += u64::from(found.is_some());
                found.unwrap_or_default()
            } else {
                Vec::new()
            };
            let lists = match (&mut held, &mut fresh) {
                (Some(l), _) => &mut l.bricks,
                (None, Some(l)) => &mut l.bricks,
                (None, None) if !found.is_empty() => {
                    &mut fresh.insert(ChunkLights::default()).bricks
                }
                (None, None) => continue,
            };
            if found.is_empty() {
                lists.remove(&(slot as u16));
            } else {
                lists.insert(slot as u16, found);
            }
        }
        if let Some(l) = fresh {
            self.lights.chunks.insert(coord, l);
            self.lights.touch();
        }
    }

    /// The ranges of the light-sample buffer by the light radius (ADR-0025): a range more
    /// than `light_radius + 1` chunks from the focus chunk is released; a chunk within
    /// `light_radius` whose lists changed or that holds no range is written, nearest first,
    /// until the sync's sample budget is spent (each chunk at most once; the rest wait for the
    /// next sync). A chunk the buffer cannot fit evicts the farthest range beyond itself.
    ///
    /// The walks over `chunks` run only when their result can differ from the last full
    /// walk's: the release walk when the focus chunk or the held radius moved, the write walk
    /// also when an input changed since ([`LightMirror::stale`]) or a chunk waits for the
    /// sample budget. Otherwise the last walk left every chunk inside the radius written and
    /// nothing outside `r + 1` holding a range, and nothing since changed that.
    pub(super) fn place_lights(&mut self, enc: &mut wgpu::CommandEncoder, st: &mut SyncStats) {
        self.lights.syncs += 1;
        let r = i64::from(self.cfg.light_radius.max(self.lights.hold));
        let f = self.window.focus;
        let moved = self.lights.walked != Some((f, r));
        let force = self.lights.force_walks;
        if !force && !moved && !self.lights.stale && self.lights.wait.is_empty() {
            st.light_oldest_wait = 0;
            return;
        }
        self.lights.walks += 1;
        let refused = st.light_refused;
        let d2 = |c: ChunkCoord| -> i64 { c.distance2(f) };
        if moved || force {
            let far: Vec<ChunkCoord> = self
                .lights
                .ranges
                .placed()
                .filter(|(c, _)| d2(*c) > (r + 1) * (r + 1))
                .map(|(c, _)| c)
                .collect();
            for c in far {
                self.release_lights(c, enc);
            }
        }
        let cap = self.cfg.light_chunk_cap;
        let mut wanted: Vec<Wanted> = self
            .lights
            .chunks
            .iter()
            .filter(|(c, l)| d2(**c) <= r * r && (!l.placed || self.lights.dirty.contains(*c)))
            .map(|(c, l)| Wanted {
                d2: d2(*c),
                coord: *c,
                placed: l.placed,
                n: thinned_len(l.bricks.values().map(Vec::len).sum(), cap) as u32,
            })
            .collect();
        wanted.sort_unstable_by_key(|w| (w.d2, w.coord.0));
        let budget = u64::from(self.cfg.light_sync_samples);
        write_walk(&wanted, budget, &mut SyncWalk { gw: self, enc, st });
        // A dirty chunk outside the radius is rewritten when it comes back inside it.
        self.lights.wait.retain(|c, _| d2(*c) <= r * r);
        st.light_oldest_wait = self
            .lights
            .wait
            .values()
            .map(|&since| self.lights.syncs - since)
            .max()
            .unwrap_or(0);
        self.lights.walked = Some((f, r));
        // A refused chunk holds no range and stays wanted: the next sync walks again.
        self.lights.stale = st.light_refused != refused;
        self.lights.changes += 1;
    }

    /// Writes `coord`'s samples (thinned to the cap) into a new range, its old range zeroed and
    /// freed; `d2` is its squared chunk distance to the focus. When the buffer has no run, the
    /// farthest range beyond `d2` is evicted until one fits; with none left the chunk is
    /// refused (`None`) and holds no range. The samples written.
    pub(super) fn write_chunk_lights(
        &mut self,
        coord: ChunkCoord,
        d2: i64,
        enc: &mut wgpu::CommandEncoder,
        st: &mut SyncStats,
    ) -> Option<u64> {
        self.release_lights(coord, enc);
        let Some(l) = self.lights.chunks.get(&coord) else {
            return Some(0);
        };
        if l.bricks.is_empty() {
            self.lights.chunks.remove(&coord);
            self.lights.touch();
            return Some(0);
        }
        // The thinned count first: a chunk the buffer refuses is not gathered.
        let total: usize = l.bricks.values().map(Vec::len).sum();
        let n = thinned_len(total, self.cfg.light_chunk_cap) as u32;
        let mut evicted = Vec::new();
        let placed = self
            .lights
            .ranges
            .place(coord, n, d2, self.window.focus, &mut evicted, st);
        for (v, r) in evicted {
            if let Some(l) = self.lights.chunks.get_mut(&v) {
                l.placed = false;
            }
            self.lights.released(r);
            self.zero_light_range(r, enc);
            self.lights.wait.entry(v).or_insert(self.lights.syncs);
            st.light_evicted += 1;
        }
        let Some(start) = placed else {
            st.light_refused += 1;
            return None;
        };
        let l = self
            .lights
            .chunks
            .get_mut(&coord)
            .expect("a placed chunk has lists");
        l.placed = true;
        let all: Vec<LightSample> = l.bricks.values().flatten().copied().collect();
        let power: f32 = all
            .iter()
            .map(|s| self.mats.emit_lum[usize::from(s.material())] * s.faces().count_ones() as f32)
            .sum();
        let samples = thin_samples(all, self.cfg.light_chunk_cap);
        debug_assert_eq!(samples.len() as u32, n);
        self.up.belt.write(
            enc,
            &self.lights.buf,
            u64::from(start) * LIGHT_SAMPLE_BYTES,
            bytemuck::cast_slice(&samples),
        );
        st.writes += 1;
        st.bytes += u64::from(n) * LIGHT_SAMPLE_BYTES;
        st.light_samples += u64::from(n);
        st.light_chunks_written += 1;
        self.lights.live += u64::from(n);
        self.lights.ranges.set_power(coord, power);
        self.lights.epoch += 1;
        Some(u64::from(n))
    }

    /// Zeroes and frees `coord`'s range, keeping its lists.
    pub(super) fn release_lights(&mut self, coord: ChunkCoord, enc: &mut wgpu::CommandEncoder) {
        if let Some(r) = self.lights.ranges.release(coord) {
            if let Some(l) = self.lights.chunks.get_mut(&coord) {
                l.placed = false;
            }
            self.lights.released(r);
            self.zero_light_range(r, enc);
        }
    }

    /// The chunks holding a range of the light-sample buffer: coordinate, start, length and
    /// power (the sum over the chunk's samples, before thinning, of its material's emitted
    /// luminance times its exposed faces; the render-side scale left out).
    pub fn light_chunks(&self) -> impl Iterator<Item = (ChunkCoord, u32, u32, f32)> + '_ {
        self.lights.ranges.placed_power()
    }

    /// The first slot of the candidate table in the light-sample buffer: just past the
    /// reservation.
    pub fn light_table_base(&self) -> u32 {
        self.cfg.light_samples
    }

    /// Holds ranges of the light-sample buffer out to `r` chunks of the focus, however small the
    /// light radius: a bake's chunk reads every emitter within the light radius of itself, not of
    /// the camera. The frame's candidate table still reads only the light radius.
    pub fn hold_lights(&mut self, r: i32) {
        self.lights.hold = r;
    }

    /// Whether every chunk with lights within the held radius holds a current range.
    pub fn lights_settled(&self) -> bool {
        let r = i64::from(self.cfg.light_radius.max(self.lights.hold));
        let f = self.window.focus;
        self.lights.chunks.iter().all(|(c, l)| {
            let d2: i64 = (0..3).map(|k| i64::from(c.0[k] - f.0[k]).pow(2)).sum();
            d2 > r * r || (l.placed && !self.lights.dirty.contains(c))
        })
    }

    /// A count that changes whenever [`GpuWorld::light_chunks`] does.
    pub fn light_epoch(&self) -> u64 {
        self.lights.epoch
    }

    /// The light radius in chunks ([`MirrorConfig::light_radius`]).
    pub fn light_radius(&self) -> i32 {
        self.cfg.light_radius
    }

    /// Forgets `coord`'s light samples; their range is zeroed and freed on the next sync.
    pub(super) fn drop_lights(&mut self, coord: ChunkCoord) {
        let dirty = self.lights.dirty.remove(&coord);
        let waited = self.lights.wait.remove(&coord).is_some();
        let held = self.lights.chunks.remove(&coord);
        if dirty || waited || held.is_some() {
            self.lights.touch();
        }
        if let Some(r) = self.lights.ranges.forget(coord) {
            self.lights.epoch += 1;
            self.lights.live -= u64::from(r.1);
            self.lights.dropped.push(r);
        }
    }

    /// Zeroes and frees the ranges `drop_lights` set aside.
    pub(super) fn flush_dropped_lights(&mut self, enc: &mut wgpu::CommandEncoder) {
        if self.lights.dropped.is_empty() {
            return;
        }
        for r in std::mem::take(&mut self.lights.dropped) {
            self.zero_light_range(r, enc);
            self.lights.ranges.free_range(r);
        }
    }

    /// Zeroes a range of the light-sample buffer in the sync's encoder, ordered with its belt
    /// copies (a queue write would run before every command buffer of the submission and
    /// could zero a range a later copy refilled).
    pub(super) fn zero_light_range(
        &self,
        (start, len): (u32, u32),
        enc: &mut wgpu::CommandEncoder,
    ) {
        enc.clear_buffer(
            &self.lights.buf,
            u64::from(start) * LIGHT_SAMPLE_BYTES,
            Some(u64::from(len) * LIGHT_SAMPLE_BYTES),
        );
    }

    /// The light-sample buffer: [`LightSample`]s, holes of material 0 between the chunks'
    /// ranges.
    pub fn light_buffer(&self) -> &wgpu::Buffer {
        &self.lights.buf
    }

    /// Light samples live in the buffer: the sum of the chunks' ranges.
    pub fn light_samples(&self) -> u64 {
        self.lights.live
    }

    /// The samples from 0 to the buffer's high-water mark; freeing the top range lowers it.
    pub fn light_span(&self) -> u32 {
        self.lights.ranges.high()
    }

    /// Whether every chunk with lights whose box lies within `r` chunks of `coord`'s box (the
    /// bake's candidate table's reach, `bake::job_table`) holds a current range of the
    /// light-sample buffer. The answer is kept by `coord` and `r` until a range, a chunk's
    /// membership or a dirty mark changes.
    pub fn lights_ready_near(&self, coord: ChunkCoord, r: i32) -> bool {
        let key = (self.lights.epoch, self.lights.changes);
        let mut cache = self
            .lights
            .ready
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if cache.key != key {
            cache.key = key;
            cache.answers.clear();
        }
        *cache
            .answers
            .entry((coord, r))
            .or_insert_with(|| self.lights_ready_near_uncached(coord, r))
    }

    /// Whether a chunk holding an exposed emitter lies with its box within `r` chunks of
    /// `coord`'s box (the bake's candidate table's reach, as [`GpuWorld::lights_ready_near`]
    /// reads it).
    pub fn emitters_near(&self, coord: ChunkCoord, r: i32) -> bool {
        self.lights
            .chunks
            .iter()
            .any(|(c, l)| boxes_within(*c, coord, r) && l.bricks.values().any(|s| !s.is_empty()))
    }

    /// [`GpuWorld::lights_ready_near`] walked in full, for tests that compare the two.
    #[doc(hidden)]
    pub fn lights_ready_near_uncached(&self, coord: ChunkCoord, r: i32) -> bool {
        let rr = i64::from(r).pow(2);
        self.lights.chunks.iter().all(|(c, l)| {
            let gap2: i64 = (0..3)
                .map(|k| i64::from(((c.0[k] - coord.0[k]).abs() - 1).max(0)).pow(2))
                .sum();
            gap2 >= rr || (l.placed && !self.lights.dirty.contains(c))
        })
    }

    /// Walks the light ranges in full at every sync, as with every input changed: a test's
    /// reference for the walks that skip.
    #[doc(hidden)]
    pub fn force_light_walks(&mut self, on: bool) {
        self.lights.force_walks = on;
    }

    /// The syncs whose `place_lights` walked the chunks' lists.
    #[doc(hidden)]
    pub fn light_walks(&self) -> u64 {
        self.lights.walks
    }

    /// `coord`'s range of the light-sample buffer, start and length.
    pub fn light_range(&self, coord: ChunkCoord) -> Option<(u32, u32)> {
        self.lights.ranges.get(coord)
    }
}

/// Whether chunk `a`'s box lies within `r` chunks of chunk `b`'s: the squared gap between the
/// boxes, in chunks, under `r^2` (the bake's candidate table's reach).
pub(crate) fn boxes_within(a: ChunkCoord, b: ChunkCoord, r: i32) -> bool {
    let gap2: i64 = (0..3)
        .map(|k| i64::from(((a.0[k] - b.0[k]).abs() - 1).max(0)).pow(2))
        .sum();
    gap2 < i64::from(r).pow(2)
}

#[cfg(test)]
mod tests;
