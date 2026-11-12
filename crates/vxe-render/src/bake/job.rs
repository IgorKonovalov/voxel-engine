//! A bake job: the per-face state, the run over a list of faces, and the dig ring's rebake.

use super::*;

#[allow(
    clippy::disallowed_methods,
    reason = "the bake times its submits for the count reading's cost rows and its progress"
)]
pub(crate) fn stopwatch() -> Instant {
    Instant::now()
}

#[allow(
    clippy::disallowed_methods,
    reason = "the bake times its submits for the count reading's cost rows and its progress"
)]
pub(super) fn seconds_since(t: Instant) -> f64 {
    t.elapsed().as_secs_f64()
}

pub(super) struct FaceState {
    pub(super) voxel: [i32; 3],
    pub(super) face: u8,
    /// Its own stop's bounds: its brick direction's share of the samples.
    pub(super) n_min: u32,
    pub(super) n_max: u32,
    pub(super) n: u32,
    /// Samples submitted and not read back yet: the next round starts at `n + queued`.
    pub(super) queued: u32,
    pub(super) sum: [f64; 3],
    pub(super) clamped: [f64; 3],
    pub(super) lum2: f64,
    pub(super) best: f32,
    pub(super) material: u16,
    pub(super) snapshots: Vec<(u32, [f32; 3], [f32; 3])>,
    pub(super) done: bool,
}

pub(super) fn lum3(c: [f64; 3]) -> f64 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

impl FaceState {
    pub(super) fn kept(&self, clamp: bool) -> [f64; 3] {
        if clamp { self.clamped } else { self.sum }
    }

    pub(super) fn mean(&self, sums: [f64; 3]) -> [f32; 3] {
        let n = f64::from(self.n.max(1));
        sums.map(|s| (s / n) as f32)
    }

    pub(super) fn se(&self, clamp: bool) -> f64 {
        se_levels(u64::from(self.n), lum3(self.kept(clamp)), self.lum2)
    }

    /// The first words of the working buffer's entry (`BakeFace` in `bake.wgsl`) for the round
    /// from sample `first`.
    pub(super) fn input(&self, first: u32, samples: u32, slot: usize) -> [u32; 20] {
        let mut w = [0u32; 20];
        w[0] = self.voxel[0] as u32;
        w[1] = self.voxel[1] as u32;
        w[2] = self.voxel[2] as u32;
        w[3] = u32::from(self.face);
        w[4] = first;
        w[5] = samples;
        w[17] = slot as u32;
        w
    }

    /// The first sample no submit has drawn.
    pub(super) fn next(&self) -> u32 {
        self.n + self.queued
    }

    /// Samples its round `k` from the next draws.
    pub(super) fn round_samples(&self, k: u32, round: u32) -> u32 {
        round.min(self.n_max.saturating_sub(self.next() + k * round))
    }

    /// The rounds left to submit before its `n_max`.
    pub(super) fn rounds_left(&self, round: u32) -> u32 {
        self.n_max.saturating_sub(self.next()).div_ceil(round)
    }

    /// The rounds its stop is expected to need, from its standard error now, which falls as
    /// `1 / sqrt(n)`: 0 before its first round or once it reads under the stop.
    pub(super) fn expected_rounds(&self, round: u32, clamp: bool, se: f32) -> u32 {
        if self.n == 0 {
            return 0;
        }
        let ratio = self.se(clamp) / f64::from(se);
        let wanted = (f64::from(self.n) * ratio * ratio).min(f64::from(self.n_max));
        let more = (wanted - f64::from(self.next())).max(0.0);
        (more / f64::from(round)).ceil() as u32
    }

    /// The rounds none of which can end the face before the last: those up to its `n_min`, or one.
    pub(super) fn certain_rounds(&self, round: u32) -> u32 {
        if self.next() < self.n_min {
            (self.n_min - self.next()).div_ceil(round)
        } else {
            1
        }
    }

    pub(super) fn add(&mut self, out: &[u32], samples: u32) {
        let f = |i: usize| f64::from(f32::from_bits(out[i]));
        for k in 0..3 {
            self.sum[k] += f(6 + k);
            self.clamped[k] += f(9 + k);
        }
        self.lum2 += f(12);
        let best = f32::from_bits(out[13]);
        if best > self.best {
            self.best = best;
            self.material = out[14] as u16;
        }
        self.n += samples;
    }
}

/// Everything the job loop needs of the frame.
pub(crate) struct BakeRun<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub gpu: &'a GpuWorld,
    pub world: &'a World,
    pub pipeline: &'a BakePipeline,
    pub world_bind: &'a wgpu::BindGroup,
    pub params_buffer: &'a wgpu::Buffer,
    pub params: Params,
}

/// A chunk's job: its faces, where its block goes, and the brick records it keeps from the block
/// before, whose bricks it does not bake.
pub(crate) struct Job {
    pub coord: ChunkCoord,
    pub faces: ChunkFaces,
    pub first_word: u64,
    /// Brick records kept from the block before, by slot: their bricks' faces are not baked.
    pub kept_bricks: KeptBricks,
    /// Samples it draws at most, for the live queue's depth ([`JobLoop::samples_left`]); 0 where
    /// nobody reads it.
    pub samples: u64,
    /// A brick direction's samples at the brick level for this job alone (the live queue's coarse
    /// pass); `None` the settings' [`BakeSettings::brick_samples`].
    pub brick_samples: Option<u32>,
}

/// Brick records a re-baked ring job keeps, by brick slot.
pub(crate) type KeptBricks = std::collections::HashMap<u16, Vec<u32>>;

/// The surface chunks of `extent` and their faces, in coordinate order.
pub(super) fn list_extent(
    world: &World,
    extent: ChunkExtent,
    classes: &[u8],
) -> Vec<(ChunkCoord, ChunkFaces)> {
    let mut jobs: Vec<(ChunkCoord, ChunkFaces)> = Vec::new();
    let [ex, ey, ez] = extent.size();
    for z in 0..ez {
        for y in 0..ey {
            for x in 0..ex {
                let c = ChunkCoord([
                    extent.min.0[0] + x,
                    extent.min.0[1] + y,
                    extent.min.0[2] + z,
                ]);
                if let Some(faces) = list_world_chunk(world, c, classes)
                    && faces.faces() > 0
                {
                    jobs.push((c, faces));
                }
            }
        }
    }
    jobs.sort_unstable_by_key(|(c, _)| c.0);
    jobs
}

/// Bakes `extent`'s chunks into `store`, emptied first.
pub(crate) fn run(
    run: &BakeRun<'_>,
    store: &mut Store,
    extent: ChunkExtent,
    settings: &BakeSettings,
    progress: &mut dyn FnMut(&BakeProgress),
) -> Result<BakeReport, BakeError> {
    let classes = face_classes(run.world.materials());
    // The list, first: the pool's blocks are known before a sample is drawn.
    let mut listed = list_extent(run.world, extent, &classes);
    if settings.reversed {
        listed.reverse();
    }
    let record = record_words(settings.cell);
    let window = run.gpu.window();
    let parents = ParentTable::of(window);
    let needed_words = parents.first
        + parents.words()
        + listed.iter().map(|(_, f)| layout(f, record).1).sum::<u64>();
    let capacity = store.pool.size();
    if needed_words * 4 > capacity {
        return Err(BakeError::PoolFull {
            needed: needed_words * 4,
            capacity,
        });
    }
    store.reset(run.queue, window);
    store.set_cell(run.queue, settings.cell);
    // The blocks lie in the jobs' order, whatever order the jobs end in.
    let mut at = store.head_words;
    let jobs: Vec<Job> = listed
        .into_iter()
        .map(|(coord, faces)| {
            let words = layout(&faces, record).1;
            let job = Job {
                coord,
                faces,
                first_word: at,
                kept_bricks: KeptBricks::new(),
                samples: 0,
                brick_samples: None,
            };
            at += words;
            job
        })
        .collect();
    let report = run_jobs(run, store, jobs, settings, progress)?;
    store.piece = Some(extent);
    Ok(report)
}

/// The bricks a dig re-bakes, by chunk: those whose voxels lie within [`DIG_REACH`] voxels of a
/// box dilated by one voxel, clipped to `piece`. A box is `min ..= max` in voxels.
pub fn dig_ring(boxes: &[[[i32; 3]; 2]], piece: ChunkExtent) -> BTreeMap<[i32; 3], Vec<u16>> {
    let mut ring: BTreeMap<[i32; 3], std::collections::BTreeSet<u16>> = BTreeMap::new();
    let pmin = piece.min.0;
    let psize = piece.size();
    for [min, max] in boxes {
        let lo = min.map(|v| v - 1);
        let hi = max.map(|v| v + 1);
        let r = DIG_REACH;
        // Bricks whose box can lie within the reach, by axis.
        let blo = [0, 1, 2].map(|k| (lo[k] - r).div_euclid(8));
        let bhi = [0, 1, 2].map(|k| (hi[k] + r).div_euclid(8));
        for bz in blo[2]..=bhi[2] {
            for by in blo[1]..=bhi[1] {
                for bx in blo[0]..=bhi[0] {
                    let b = [bx, by, bz];
                    let gap2: i64 = (0..3)
                        .map(|k| {
                            let (b0, b1) = (b[k] * 8, b[k] * 8 + 7);
                            let g = i64::from((lo[k] - b1).max(b0 - hi[k]).max(0));
                            g * g
                        })
                        .sum();
                    if gap2 > i64::from(r * r) {
                        continue;
                    }
                    let c = b.map(|v| v.div_euclid(8));
                    if (0..3).any(|k| c[k] < pmin[k] || c[k] >= pmin[k] + psize[k]) {
                        continue;
                    }
                    let inner = b.map(|v| v.rem_euclid(8));
                    ring.entry(c).or_default().insert(brick_slot(inner) as u16);
                }
            }
        }
    }
    ring.into_iter()
        .map(|(c, s)| (c, s.into_iter().collect()))
        .collect()
}

/// Voxels from a dig's box within which bricks re-bake (ADR-0046's rings 1 and 2).
pub(crate) const DIG_REACH: i32 = 32;

/// What a dig's re-bake did.
#[derive(Clone, Debug, Default)]
pub struct DigReport {
    /// The ring's bricks inside the piece; 0 is a dig wholly outside it.
    pub bricks: u64,
    pub bake: BakeReport,
}

/// Re-bakes the ring of `boxes` ([`dig_ring`]) in `store`: each chunk the ring reaches is listed
/// again, the ring's bricks baked, every other brick keeping its record, and its block
/// rewritten in its place when it fits, else at the pool's end. A chunk left with no face drops
/// its block.
pub(crate) fn rebake(
    run: &BakeRun<'_>,
    store: &mut Store,
    boxes: &[[[i32; 3]; 2]],
    settings: &BakeSettings,
    progress: &mut dyn FnMut(&BakeProgress),
) -> Result<DigReport, BakeError> {
    let Some(piece) = store.piece else {
        return Ok(DigReport::default());
    };
    let ring = dig_ring(boxes, piece);
    let classes = face_classes(run.world.materials());
    let mut jobs = Vec::new();
    let mut end = store.end_words;
    let mut bricks = 0u64;
    for (c, slots) in &ring {
        bricks += slots.len() as u64;
        let coord = ChunkCoord(*c);
        let faces = list_world_chunk(run.world, coord, &classes).unwrap_or_default();
        if faces.faces() == 0 {
            store.remove(run.queue, coord);
            continue;
        }
        let record = record_words(store.cell);
        let mut kept_bricks = KeptBricks::new();
        if let Some(b) = store.words.get(c) {
            for (slot, record) in records_of(b, record) {
                if slots.binary_search(&slot).is_err() {
                    kept_bricks.insert(slot, record);
                }
            }
        }
        let first_word = store.place(coord, layout(&faces, record).1, &mut end)?;
        jobs.push(Job {
            coord,
            faces,
            first_word,
            kept_bricks,
            samples: 0,
            brick_samples: None,
        });
    }
    let bake = run_jobs(run, store, jobs, settings, progress)?;
    Ok(DigReport { bricks, bake })
}

/// Bakes `jobs` into `store`, each block written at its `first_word` as its job ends.
pub(super) fn run_jobs(
    run: &BakeRun<'_>,
    store: &mut Store,
    jobs: Vec<Job>,
    settings: &BakeSettings,
    progress: &mut dyn FnMut(&BakeProgress),
) -> Result<BakeReport, BakeError> {
    let open_cap = if settings.jobs_alone { 1 } else { LOOP_SLOTS };
    let mut lp = JobLoop::new(jobs, 0..open_cap);
    let bind = run.pipeline.bind();
    // The frame's parameters, once: the bake reads no view but the LOD walk's camera chunk,
    // which it takes from the face's job.
    run.queue
        .write_buffer(run.params_buffer, 0, bytemuck::bytes_of(&run.params));
    let queue = run.queue;
    let mut sink = |store: &mut Store, coord: ChunkCoord, first: u64, block: Vec<u32>| {
        store.put(queue, coord, first, block);
    };
    while lp
        .submit(run, bind, store, settings, u64::MAX, &mut sink, progress)
        .is_some()
    {}
    run.queue.submit([]);
    Ok(lp.report)
}
