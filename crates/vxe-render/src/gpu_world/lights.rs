//! The light-sample buffer (ADR-0025): each mirrored chunk's exposed emitters, and their
//! ranges of the buffer by the light radius.

use super::*;

/// The light-sample buffer and each chunk's samples.
pub(super) struct LightMirror {
    /// The buffer, its free ranges (start to length) below `high`, the first sample never
    /// handed out.
    pub(super) buf: wgpu::Buffer,
    pub(super) free: BTreeMap<u32, u32>,
    pub(super) high: u32,
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
            free: BTreeMap::new(),
            high: 0,
            hold: 0,
            chunks: HashMap::new(),
            live: 0,
            epoch: 0,
            dropped: Vec::new(),
            dirty: HashSet::new(),
            wait: HashMap::new(),
            syncs: 0,
        }
    }

    /// The start of a free run of `n` samples: the lowest free range that holds it, else the
    /// samples above the high-water mark; `None` when the buffer has no such run.
    pub(super) fn alloc_range(&mut self, n: u32, samples: u32) -> Option<u32> {
        if let Some((&start, &len)) = self.free.iter().find(|&(_, &len)| len >= n) {
            self.free.remove(&start);
            if len > n {
                self.free.insert(start + n, len - n);
            }
            return Some(start);
        }
        let start = self.high;
        if u64::from(start) + u64::from(n) > u64::from(samples) {
            return None;
        }
        self.high += n;
        Some(start)
    }

    /// Returns a range to the free ranges, merged with its neighbours; a free range that
    /// reaches the high-water mark lowers it instead.
    pub(super) fn free_range(&mut self, (mut start, mut len): (u32, u32)) {
        if let Some((&s, &l)) = self.free.range(..start).next_back()
            && s + l == start
        {
            self.free.remove(&s);
            start = s;
            len += l;
        }
        if let Some(&l) = self.free.get(&(start + len)) {
            self.free.remove(&(start + len));
            len += l;
        }
        if start + len == self.high {
            self.high = start;
        } else {
            self.free.insert(start, len);
        }
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
    let Brick::Full(p) = brick else {
        return Vec::new();
    };
    if !materials_of(p, |m| emissive[usize::from(m)]) {
        return Vec::new();
    }
    let mut v = [AIR; BRICK_VOXELS];
    p.decode(&mut v);
    let b = brick_of_slot(slot);
    let base = [0, 1, 2].map(|k| coord.0[k] * CHUNK_EDGE_BRICKS * BRICK_EDGE + b[k] * BRICK_EDGE);
    let e = BRICK_EDGE as usize;
    let beside = [0, 1, 2].map(|k| [neighbour(k, -1), neighbour(k, 1)]);
    // Whether the voxel across the brick's face from local `l` on axis `k` toward `dir` lets
    // light out: no brick there (the chunk's face) or a voxel that is not opaque.
    let across = |l: [usize; 3], k: usize, dir: usize| match beside[k][dir] {
        None => true,
        Some(b) => {
            let mut q = l;
            q[k] = if dir == 0 { e - 1 } else { 0 };
            !opaque[usize::from(b.get(q[0] + e * (q[1] + e * q[2])))]
        }
    };
    let mut out = Vec::new();
    for (i, &m) in v.iter().enumerate() {
        if !emissive[usize::from(m)] || !opaque[usize::from(m)] {
            continue;
        }
        let l = [i % e, i / e % e, i / (e * e)];
        let mut faces = 0u32;
        for k in 0..3 {
            let stride = [1, e, e * e][k];
            let low = if l[k] == 0 {
                across(l, k, 0)
            } else {
                !opaque[usize::from(v[i - stride])]
            };
            let high = if l[k] == e - 1 {
                across(l, k, 1)
            } else {
                !opaque[usize::from(v[i + stride])]
            };
            faces |= u32::from(low) << (2 * k) | u32::from(high) << (2 * k + 1);
        }
        if faces != 0 {
            out.push(LightSample {
                voxel: [0, 1, 2].map(|k| base[k] + l[k] as i32),
                material_faces: u32::from(m) | faces << 16,
            });
        }
    }
    out
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

/// A chunk's light samples: each emissive brick's list by brick slot, and its range of the
/// light-sample buffer.
#[derive(Default)]
pub(super) struct ChunkLights {
    pub(super) bricks: BTreeMap<u16, Vec<LightSample>>,
    pub(super) range: Option<(u32, u32)>,
    /// The power of the samples last written ([`GpuWorld::light_chunks`]).
    pub(super) power: f32,
}

impl GpuWorld {
    /// Replaces brick `slot` of `coord`'s light lists with its exposed emissive voxels.
    pub(super) fn note_lights(
        &mut self,
        coord: ChunkCoord,
        slot: usize,
        bricks: &Bricks<'_>,
        st: &mut SyncStats,
    ) {
        let brick = bricks.get(slot);
        let found = if brick.is_full() {
            st.light_bricks += 1;
            let opaque: &[bool] = &self.mats.opaque;
            let b = brick_of_slot(slot);
            let neighbour = |k: usize, dir: i32| {
                let mut n = b;
                n[k] += dir;
                (0..CHUNK_EDGE_BRICKS)
                    .contains(&n[k])
                    .then(|| bricks.get(brick_slot(n)))
            };
            exposed_emitters(coord, slot, brick, &self.mats.emissive, opaque, neighbour)
        } else {
            Vec::new()
        };
        match self.lights.chunks.get_mut(&coord) {
            Some(l) => {
                if found.is_empty() {
                    l.bricks.remove(&(slot as u16));
                } else {
                    l.bricks.insert(slot as u16, found);
                }
            }
            None if !found.is_empty() => {
                let mut l = ChunkLights::default();
                l.bricks.insert(slot as u16, found);
                self.lights.chunks.insert(coord, l);
            }
            None => {}
        }
    }

    /// The ranges of the light-sample buffer by the light radius (ADR-0025): a range more
    /// than `light_radius + 1` chunks from the focus chunk is released; a chunk within
    /// `light_radius` whose lists changed or that holds no range is written, nearest first,
    /// until the sync's sample budget is spent (each chunk at most once; the rest wait for the
    /// next sync). A chunk the buffer cannot fit evicts the farthest range beyond itself.
    pub(super) fn place_lights(&mut self, enc: &mut wgpu::CommandEncoder, st: &mut SyncStats) {
        self.lights.syncs += 1;
        let r = i64::from(self.cfg.light_radius.max(self.lights.hold));
        let f = self.window.focus;
        let d2 = |c: ChunkCoord| -> i64 { c.distance2(f) };
        let far: Vec<ChunkCoord> = self
            .lights
            .chunks
            .iter()
            .filter(|(c, l)| l.range.is_some() && d2(**c) > (r + 1) * (r + 1))
            .map(|(c, _)| *c)
            .collect();
        for c in far {
            self.release_lights(c, enc);
        }
        let mut wanted: Vec<(i64, ChunkCoord)> = self
            .lights
            .chunks
            .iter()
            .filter(|(c, l)| {
                d2(**c) <= r * r && (l.range.is_none() || self.lights.dirty.contains(*c))
            })
            .map(|(c, _)| (d2(*c), *c))
            .collect();
        wanted.sort_unstable_by_key(|&(d, c)| (d, c.0));
        let mut written = 0u64;
        for (d, c) in wanted {
            if written >= u64::from(self.cfg.light_sync_samples) {
                self.lights.wait.entry(c).or_insert(self.lights.syncs);
                continue;
            }
            written += self.write_chunk_lights(c, d, enc, st);
            self.lights.dirty.remove(&c);
            self.lights.wait.remove(&c);
        }
        // A dirty chunk outside the radius is rewritten when it comes back inside it.
        self.lights.wait.retain(|c, _| d2(*c) <= r * r);
        st.light_oldest_wait = self
            .lights
            .wait
            .values()
            .map(|&since| self.lights.syncs - since)
            .max()
            .unwrap_or(0);
    }

    /// Writes `coord`'s samples (thinned to the cap) into a new range, its old range zeroed and
    /// freed; `d2` is its squared chunk distance to the focus. When the buffer has no run, the
    /// farthest range beyond `d2` is evicted until one fits; with none left the chunk is
    /// refused and holds no range. The samples written.
    pub(super) fn write_chunk_lights(
        &mut self,
        coord: ChunkCoord,
        d2: i64,
        enc: &mut wgpu::CommandEncoder,
        st: &mut SyncStats,
    ) -> u64 {
        self.release_lights(coord, enc);
        let Some(l) = self.lights.chunks.get(&coord) else {
            return 0;
        };
        if l.bricks.is_empty() {
            self.lights.chunks.remove(&coord);
            return 0;
        }
        let all: Vec<LightSample> = l.bricks.values().flatten().copied().collect();
        let power: f32 = all
            .iter()
            .map(|s| self.mats.emit_lum[usize::from(s.material())] * s.faces().count_ones() as f32)
            .sum();
        let samples = thin_samples(all, self.cfg.light_chunk_cap);
        let n = samples.len() as u32;
        let start = loop {
            if let Some(start) = self.lights.alloc_range(n, self.cfg.light_samples) {
                break Some(start);
            }
            let f = self.window.focus;
            let victim = self
                .lights
                .chunks
                .iter()
                .filter(|(c, l)| **c != coord && l.range.is_some())
                .map(|(c, _)| {
                    let d: i64 = (0..3).map(|k| i64::from(c.0[k] - f.0[k]).pow(2)).sum();
                    (d, c.0, *c)
                })
                .filter(|&(d, _, _)| d > d2)
                .max();
            match victim {
                Some((_, _, v)) => {
                    self.release_lights(v, enc);
                    self.lights.wait.entry(v).or_insert(self.lights.syncs);
                    st.light_evicted += 1;
                }
                None => break None,
            }
        };
        let Some(start) = start else {
            st.light_refused += 1;
            return 0;
        };
        self.up.belt.write(
            enc,
            &self.lights.buf,
            u64::from(start) * LIGHT_SAMPLE_BYTES,
            bytemuck::cast_slice(&samples),
        );
        st.writes += 1;
        st.bytes += u64::from(n) * LIGHT_SAMPLE_BYTES;
        st.light_samples += u64::from(n);
        self.lights.live += u64::from(n);
        if let Some(l) = self.lights.chunks.get_mut(&coord) {
            l.range = Some((start, n));
            l.power = power;
        }
        self.lights.epoch += 1;
        u64::from(n)
    }

    /// Zeroes and frees `coord`'s range, keeping its lists.
    pub(super) fn release_lights(&mut self, coord: ChunkCoord, enc: &mut wgpu::CommandEncoder) {
        if let Some(r) = self
            .lights
            .chunks
            .get_mut(&coord)
            .and_then(|l| l.range.take())
        {
            self.lights.epoch += 1;
            self.lights.live -= u64::from(r.1);
            self.zero_light_range(r, enc);
            self.lights.free_range(r);
        }
    }

    /// The chunks holding a range of the light-sample buffer: coordinate, start, length and
    /// power (the sum over the chunk's samples, before thinning, of its material's emitted
    /// luminance times its exposed faces; the render-side scale left out).
    pub fn light_chunks(&self) -> impl Iterator<Item = (ChunkCoord, u32, u32, f32)> + '_ {
        self.lights
            .chunks
            .iter()
            .filter_map(|(c, l)| l.range.map(|(s, n)| (*c, s, n, l.power)))
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
            d2 > r * r || (l.range.is_some() && !self.lights.dirty.contains(c))
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
        self.lights.dirty.remove(&coord);
        self.lights.wait.remove(&coord);
        if let Some(l) = self.lights.chunks.remove(&coord)
            && let Some(r) = l.range
        {
            self.lights.epoch += 1;
            self.lights.live -= u64::from(r.1);
            self.lights.dropped.push(r);
        }
    }

    /// Zeroes and frees the ranges `drop_lights` set aside.
    pub(super) fn flush_dropped_lights(&mut self, enc: &mut wgpu::CommandEncoder) {
        for r in std::mem::take(&mut self.lights.dropped) {
            self.zero_light_range(r, enc);
            self.lights.free_range(r);
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
        self.lights.high
    }

    /// `coord`'s range of the light-sample buffer, start and length.
    pub fn light_range(&self, coord: ChunkCoord) -> Option<(u32, u32)> {
        self.lights.chunks.get(&coord).and_then(|l| l.range)
    }
}
