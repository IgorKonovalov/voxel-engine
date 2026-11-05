//! The loaded light's store and its bake (ADR-0046): every face of a chunk holds its emissive
//! irradiance at unit emission in a cell of one pool, computed ahead of the frames that read it.
//!
//! **The list.** [`list_chunk_faces`] is a pure function of a chunk's voxels, its six neighbours'
//! and which materials are opaque: each surface brick's sorted face keys.
//!
//! **The pool** ([`Store`]), in `u32` words: the missing count (word 0), the mirror window's
//! origin (word 4) and size (word 8) in chunks, then from word [`STORE_TABLE`] one word per
//! window chunk naming its block's first word (0 none). A block is 512 directory words, a brick's
//! `run offset from the block's start << 11 | faces` (0 no surface), then the runs: two words a
//! cell, its [`rgb9e5_pack`] irradiance, then `key | flags << 12 | dominant material << 16`, with
//! the key `voxel in the brick << 3 | face`, sorted by key.
//!
//! **The job** ([`Frame::bake_until`](crate::Frame::bake_until)): a chunk's faces, its own
//! candidate table ([`job_table`]) and its frame parameters, staged once in a slot when the job
//! opens. A submit holds the live faces of up to [`OPEN_JOBS`] open jobs, at most
//! [`SUBMIT_SAMPLES`] samples, and is waited on. Each job is one dispatch, preceded by copies of
//! its slot to where the frame's table, the frame's parameters and the job uniform go, so every
//! face samples its own job's table. An entry of the working buffer is one round of one face: a
//! face takes the rounds that cannot end it (those up to `n_min`) at once, one round past that,
//! and, when the submit still has room and no job is left to open, further rounds speculatively.
//! `bake.wgsl` returns each entry's sums; the running sums are kept here in `f64` and added round
//! by round in order, the stop is read after each round, and a round past a face's stop is thrown
//! away ([`BakeReport::wasted`]). A face's samples are a function of its key and the sample's
//! index alone, so its rounds, its stop and its cell depend neither on the batching, nor on the
//! rounds a submit, nor on which jobs share a submit, nor on the jobs' order.
//!
//! The stop ([`se_levels`]): a face ends when the standard error of its mean luminance, through the
//! display transform at [`BAKE_EXPOSURE`], is at most [`BAKE_SE`] levels, not before `n_min` samples
//! and not after `n_max`.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use vxe_world::{
    AIR, BRICK_VOXELS, Brick, CHUNK_BRICKS, Chunk, ChunkCoord, ChunkExtent, MaterialTable, World,
    brick_of_slot, brick_slot,
};

use crate::gpu_world::GpuWorld;
use crate::marcher::{LIGHT_TABLE_ENTRIES, LightTableUniform, Params};
use crate::readback;

/// The sampler, compiled after [`crate::MARCH_WGSL`] and [`crate::LIGHT_WGSL`].
pub const BAKE_WGSL: &str = include_str!("shaders/bake.wgsl");

/// Samples a face takes between two looks at its stop.
pub const BAKE_ROUND: u32 = 1024;
/// Entries (one round of one face) one submit's working buffer holds at most.
pub const BAKE_BATCH_FACES: usize = 65_536;
/// Jobs open at once, each with a slot of the tables buffer.
pub const OPEN_JOBS: usize = 256;
/// The bytes of the job uniform (`BakeJob` in `bake.wgsl`).
const JOB_BYTES: u64 = 32;
/// A slot's 16-byte words (`TABLE_SLOT` in `bake.wgsl`): the candidate table (a header, then
/// [`LIGHT_TABLE_ENTRIES`] entries), then the job's chunk.
const TABLE_SLOT_WORDS: u64 = 2 + LIGHT_TABLE_ENTRIES as u64;
const TABLE_SLOT_BYTES: u64 = TABLE_SLOT_WORDS * 16;
const _: () = assert!(TABLE_SLOT_WORDS == 1026, "TABLE_SLOT in bake.wgsl");
/// The stop's standard error, in display levels.
pub const BAKE_SE: f32 = 0.5;
/// Samples a face takes at least.
pub const N_MIN: u32 = 4096;
/// Samples a face takes at most.
pub const N_MAX: u32 = 1 << 20;
/// Samples one submit holds at most, so the driver sees short dispatches with a wait between.
pub const SUBMIT_SAMPLES: u64 = 10_000_000;
/// The exposure the stop reads the display at (the owner's windows' fixed exposure).
pub const BAKE_EXPOSURE: f32 = 8.0;
/// [`Lighting::bake_pool_bytes`](crate::Lighting) by default; the device's storage binding clamps it.
pub const DEFAULT_BAKE_POOL_BYTES: u64 = 256 << 20;
/// The pool under `Light::Table`: the header's words only, which nothing reads.
pub const MINIMAL_POOL_BYTES: u64 = 64;
/// The word of the pool's chunk table.
pub const STORE_TABLE: u64 = 16;
/// The words of a block's directory.
pub const DIRECTORY_WORDS: u64 = CHUNK_BRICKS as u64;
/// The bytes of one face in the working buffer (`BakeFace` in `bake.wgsl`).
pub const FACE_BYTES: u64 = 80;

/// The display's level, 0 to 255, of luminance `l` at unit exposure: `tone.wgsl`'s ACES and 2.2
/// encode at [`BAKE_EXPOSURE`], unrounded.
pub fn tone_level(l: f64) -> f64 {
    let x = (l * f64::from(BAKE_EXPOSURE)).max(0.0);
    let m = ((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14)).clamp(0.0, 1.0);
    m.powf(1.0 / 2.2) * 255.0
}

/// The standard error, in display levels, of the mean of `n` samples whose luminances sum to `sum`
/// and whose squares sum to `sum2`.
pub fn se_levels(n: u64, sum: f64, sum2: f64) -> f64 {
    if n == 0 {
        return f64::INFINITY;
    }
    let n = n as f64;
    let mean = sum / n;
    let se = ((sum2 / n - mean * mean).max(0.0) / n).sqrt();
    0.5 * (tone_level(mean + se) - tone_level((mean - se).max(0.0)))
}

/// `c` as an `rgb9e5` word: 9-bit mantissas over a shared exponent, red in the low bits, the
/// exponent (bias 15) in the top five.
pub fn rgb9e5_pack(c: [f32; 3]) -> u32 {
    const MAX: f32 = 65408.0;
    let c = c.map(|v| if v.is_nan() { 0.0 } else { v.clamp(0.0, MAX) });
    let top = c[0].max(c[1]).max(c[2]);
    if top <= 0.0 {
        return 0;
    }
    let e2 = (top.to_bits() >> 23) as i32 - 127;
    let mut exp = e2.max(-16) + 1 + 15;
    let mut scale = 2f64.powi(exp - 24);
    if (f64::from(top) / scale + 0.5).floor() >= 512.0 {
        exp += 1;
        scale *= 2.0;
    }
    let m = c.map(|v| (f64::from(v) / scale + 0.5).floor() as u32);
    m[0] | m[1] << 9 | m[2] << 18 | (exp as u32) << 27
}

/// The value of an `rgb9e5` word.
pub fn rgb9e5_unpack(w: u32) -> [f32; 3] {
    let scale = 2f64.powi((w >> 27) as i32 - 24);
    [w & 511, (w >> 9) & 511, (w >> 18) & 511].map(|m| (f64::from(m) * scale) as f32)
}

/// Which materials stop light: not air, not translucent.
pub fn opaque_table(materials: &MaterialTable) -> Vec<bool> {
    let mut t = vec![false; 1 << 16];
    for (id, m) in materials.rows().iter().enumerate() {
        t[id] = id != usize::from(AIR) && !m.is_translucent();
    }
    t
}

/// A chunk's exposed faces, by surface brick in slot order, each brick's keys ascending. A key is
/// `voxel in the brick << 3 | face`, face `2 axis + 1` toward `+axis`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChunkFaces {
    pub bricks: Vec<(u16, Vec<u16>)>,
}

impl ChunkFaces {
    /// Faces over the chunk's bricks.
    pub fn faces(&self) -> usize {
        self.bricks.iter().map(|(_, k)| k.len()).sum()
    }

    /// The block's words: the directory, then two words a cell.
    pub fn block_words(&self) -> u64 {
        DIRECTORY_WORDS + 2 * self.faces() as u64
    }

    /// The block's directory words.
    pub fn directory(&self) -> Vec<u32> {
        let mut dir = vec![0u32; DIRECTORY_WORDS as usize];
        let mut at = DIRECTORY_WORDS as u32;
        for (slot, keys) in &self.bricks {
            dir[usize::from(*slot)] = at << 11 | keys.len() as u32;
            at += 2 * keys.len() as u32;
        }
        dir
    }
}

const NO_VOXEL: [u64; 8] = [0; 8];
const ALL_VOXELS: [u64; 8] = [u64::MAX; 8];

/// A brick's opaque voxels: bit `x + 8 y` of word `z`.
fn opaque_mask(brick: &Brick, opaque: &[bool]) -> [u64; 8] {
    match brick {
        Brick::Uniform(m) => {
            if opaque[usize::from(*m)] {
                ALL_VOXELS
            } else {
                NO_VOXEL
            }
        }
        Brick::Full(p) => {
            let mut v = [AIR; BRICK_VOXELS];
            p.decode(&mut v);
            let mut mask = NO_VOXEL;
            for (i, m) in v.iter().enumerate() {
                if opaque[usize::from(*m)] {
                    mask[i / 64] |= 1 << (i % 64);
                }
            }
            mask
        }
    }
}

fn mask_bit(mask: &[u64; 8], l: [usize; 3]) -> bool {
    mask[l[2]] >> (l[0] + 8 * l[1]) & 1 != 0
}

/// `chunk`'s faces toward a voxel that is not opaque, across brick and chunk borders. The
/// neighbours are in face order (`-x, +x, -y, +y, -z, +z`); across a neighbour that is `None` no
/// ray arrives (an absent chunk is rock to the walk), so no face is listed there.
pub fn list_chunk_faces(
    chunk: &Chunk,
    neighbours: &[Option<&Chunk>; 6],
    opaque: &[bool],
) -> ChunkFaces {
    let masks: Vec<[u64; 8]> = chunk
        .bricks()
        .iter()
        .map(|b| opaque_mask(b, opaque))
        .collect();
    let mut out = ChunkFaces::default();
    for slot in 0..CHUNK_BRICKS {
        let mask = masks[slot];
        if mask == NO_VOXEL {
            continue;
        }
        let b = brick_of_slot(slot);
        let beside: [[u64; 8]; 6] = std::array::from_fn(|face| {
            let (k, d) = (face / 2, if face % 2 == 1 { 1 } else { -1 });
            let mut n = b;
            n[k] += d;
            if (0..8).contains(&n[k]) {
                return masks[brick_slot(n)];
            }
            n[k] = n[k].rem_euclid(8);
            match neighbours[face] {
                Some(c) => opaque_mask(c.brick(brick_slot(n)), opaque),
                None => ALL_VOXELS,
            }
        });
        if mask == ALL_VOXELS && beside.iter().all(|m| *m == ALL_VOXELS) {
            continue;
        }
        let mut keys = Vec::new();
        for i in 0..BRICK_VOXELS {
            if mask[i / 64] >> (i % 64) & 1 == 0 {
                continue;
            }
            let l = [i % 8, i / 8 % 8, i / 64];
            for (face, side) in beside.iter().enumerate() {
                let k = face / 2;
                let solid = if face % 2 == 1 {
                    if l[k] == 7 {
                        let mut q = l;
                        q[k] = 0;
                        mask_bit(side, q)
                    } else {
                        let mut q = l;
                        q[k] += 1;
                        mask_bit(&mask, q)
                    }
                } else if l[k] == 0 {
                    let mut q = l;
                    q[k] = 7;
                    mask_bit(side, q)
                } else {
                    let mut q = l;
                    q[k] -= 1;
                    mask_bit(&mask, q)
                };
                if !solid {
                    keys.push((i as u16) << 3 | face as u16);
                }
            }
        }
        if !keys.is_empty() {
            out.bricks.push((slot as u16, keys));
        }
    }
    out
}

/// The faces of `coord` in `world`, whose absent neighbours list nothing across.
pub fn list_world_chunk(world: &World, coord: ChunkCoord, opaque: &[bool]) -> Option<ChunkFaces> {
    let store = world.store();
    let chunk = store.get(coord)?;
    let neighbour = |face: usize| {
        let mut c = coord.0;
        c[face / 2] += if face % 2 == 1 { 1 } else { -1 };
        store.get(ChunkCoord(c))
    };
    let held: [_; 6] = std::array::from_fn(neighbour);
    let locks: Vec<_> = held
        .iter()
        .map(|h| {
            h.as_ref()
                .map(|c| c.read().unwrap_or_else(std::sync::PoisonError::into_inner))
        })
        .collect();
    let refs: [Option<&Chunk>; 6] = std::array::from_fn(|f| locks[f].as_deref());
    let c = chunk
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Some(list_chunk_faces(&c, &refs, opaque))
}

/// A test of a voxel and its face.
pub type FaceFilter = Arc<dyn Fn([i32; 3], u8) -> bool + Send + Sync>;

/// The bake's settings. The default is the reference's: [`KEPT_CANDIDATES`] candidates, the clamp
/// as [`KEPT_CLAMP`], each face to its own stop.
#[derive(Clone)]
pub struct BakeSettings {
    /// Emissive candidates a sample resamples.
    pub candidates: u32,
    /// The estimate is clamped at the light pass's `FIREFLY_CLAMP`.
    pub clamp: bool,
    /// The random sequences' seed.
    pub seed: u32,
    /// Samples between two looks at a face's stop.
    pub round: u32,
    pub n_min: u32,
    pub n_max: u32,
    /// The stop's standard error, in display levels.
    pub se: f32,
    /// Entries (one round of one face) a submit holds at most; `SUBMIT_SAMPLES / round` and
    /// [`BAKE_BATCH_FACES`] cap it too.
    pub batch_faces: usize,
    /// Rounds one face takes in a submit at most; 0 is as many as the submit's room allows.
    pub submit_rounds: u32,
    /// A submit holds one job's faces only.
    pub jobs_alone: bool,
    /// The chunks in descending coordinate order, for the test that a result ignores the order.
    pub reversed: bool,
    /// [`BakeReport::faces`] holds every face.
    pub keep_faces: bool,
    /// Sample counts at which [`FaceOutcome::snapshots`] take the running means.
    pub snapshots: Vec<u32>,
    /// Bakes only the faces this passes (a reading's sphere); the others stay 0.
    pub filter: Option<FaceFilter>,
}

/// Candidates a sample resamples in the reference: the arm the count reading keeps.
pub const KEPT_CANDIDATES: u32 = 32;
/// Whether the reference clamps a sample: the arm the count reading keeps.
pub const KEPT_CLAMP: bool = false;

impl Default for BakeSettings {
    fn default() -> Self {
        Self {
            candidates: KEPT_CANDIDATES,
            clamp: KEPT_CLAMP,
            seed: 0,
            round: BAKE_ROUND,
            n_min: N_MIN,
            n_max: N_MAX,
            se: BAKE_SE,
            batch_faces: BAKE_BATCH_FACES,
            submit_rounds: 0,
            jobs_alone: false,
            reversed: false,
            keep_faces: false,
            snapshots: Vec::new(),
            filter: None,
        }
    }
}

/// One face after its bake.
#[derive(Clone, Debug)]
pub struct FaceOutcome {
    pub voxel: [i32; 3],
    pub face: u8,
    pub samples: u32,
    /// The mean irradiance, unclamped and clamped.
    pub mean: [[f32; 3]; 2],
    /// The stop's standard error in levels at the end.
    pub se: f32,
    /// The dominant material.
    pub material: u16,
    /// `(samples, mean unclamped, mean clamped)` at each of [`BakeSettings::snapshots`] reached.
    pub snapshots: Vec<(u32, [f32; 3], [f32; 3])>,
}

/// What a bake did.
#[derive(Clone, Debug, Default)]
pub struct BakeReport {
    pub chunks: u64,
    pub faces: u64,
    /// Samples kept: the rounds up to each face's stop.
    pub samples: u64,
    /// Samples of rounds drawn past a face's stop in the same submit, thrown away.
    pub wasted: u64,
    pub submits: u64,
    /// The most jobs one submit held.
    pub max_jobs_in_submit: u64,
    /// The most rounds one face took in one submit.
    pub max_rounds_in_submit: u32,
    /// Seconds from each submit to its readback, over the bake.
    pub gpu_seconds: f64,
    pub faces_at_min: u64,
    pub faces_at_max: u64,
    pub rays: u64,
    pub ray_len: f64,
    /// Every face, under [`BakeSettings::keep_faces`].
    pub outcomes: Vec<FaceOutcome>,
}

/// The bake's progress, after a submit.
#[derive(Clone, Copy, Debug, Default)]
pub struct BakeProgress {
    pub chunks_done: u64,
    pub chunks: u64,
    pub faces_done: u64,
    pub faces: u64,
    pub samples: u64,
    pub gpu_seconds: f64,
}

/// Why a bake did not run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BakeError {
    /// The frame's light is not `Light::Baked`.
    NotBaked,
    /// The pool cannot hold the extent's blocks.
    PoolFull { needed: u64, capacity: u64 },
    /// The light-sample buffer refused a chunk in the bake's reach.
    LightsRefused(u64),
}

impl std::fmt::Display for BakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotBaked => write!(f, "the frame's light is not baked"),
            Self::PoolFull { needed, capacity } => {
                write!(f, "the bake pool needs {needed} B and holds {capacity} B")
            }
            Self::LightsRefused(n) => {
                write!(f, "the light-sample buffer refused {n} chunks in reach")
            }
        }
    }
}

impl std::error::Error for BakeError {}

/// The store's counters, for the HUD.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StoreStats {
    /// Chunks holding a block.
    pub chunks: u64,
    /// Bytes of the pool in use.
    pub bytes: u64,
    /// Lighting pixels of the last encode whose cube face had no cell.
    pub missing: u32,
}

/// The pool and where its blocks lie.
pub struct Store {
    pool: wgpu::Buffer,
    blocks: BTreeMap<[i32; 3], (u64, u64)>,
    used_words: u64,
}

impl Store {
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
            used_words: 0,
        }
    }

    /// The pool buffer.
    pub fn pool(&self) -> &wgpu::Buffer {
        &self.pool
    }

    pub(crate) fn stats(&self, missing: u32) -> StoreStats {
        StoreStats {
            chunks: self.blocks.len() as u64,
            bytes: self.used_words * 4,
            missing,
        }
    }

    /// `coord`'s block: its first word and its words.
    pub fn block(&self, coord: ChunkCoord) -> Option<(u64, u64)> {
        self.blocks.get(&coord.0).copied()
    }
}

/// The compute pipeline and the buffers of a bake, built on the first bake.
pub struct BakePipeline {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    job: wgpu::Buffer,
    faces: wgpu::Buffer,
    /// [`OPEN_JOBS`] slots of [`TABLE_SLOT_BYTES`]: an open job's candidate table and chunk.
    tables: wgpu::Buffer,
}

/// The light pass's table read, which the bake's module reads from the face's slot instead.
const TABLE_READ: &str = "return bitcast<vec4<u32>>(light_samples[params.light_table_base + i]);";
/// The LOD walk's camera chunk, which the bake's module takes from the face's job instead.
const LOD_ORIGIN: &str = "let d = chunk - floor_div(vec3<i32>(floor(params.views[0].origin)), 64);";

fn replace_once(source: &str, from: &str, to: &str) -> String {
    assert_eq!(
        source.matches(from).count(),
        1,
        "the bake's module replaces `{from}` once"
    );
    source.replace(from, to)
}

/// The bake's module: march.wgsl and light.wgsl, with the candidate table and the LOD walk's
/// camera chunk read from the face's job slot (`bake_table_slot`, `bake_origin_chunk`), so one
/// dispatch serves faces of many jobs; then bake.wgsl.
pub fn bake_source() -> String {
    let march = replace_once(
        crate::MARCH_WGSL,
        LOD_ORIGIN,
        "let d = chunk - bake_origin_chunk();",
    );
    let light = replace_once(crate::LIGHT_WGSL, TABLE_READ, "return bake_table_slot(i);");
    format!("{march}\n{light}\n{BAKE_WGSL}")
}

impl BakePipeline {
    pub(crate) fn new(device: &wgpu::Device, world_layout: &wgpu::BindGroupLayout) -> Self {
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bake"),
            entries: &[
                entry(0, wgpu::BufferBindingType::Uniform),
                entry(1, wgpu::BufferBindingType::Storage { read_only: false }),
                entry(2, wgpu::BufferBindingType::Storage { read_only: true }),
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bake"),
            source: wgpu::ShaderSource::Wgsl(bake_source().into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bake"),
            bind_group_layouts: &[Some(world_layout), None, None, Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("bake"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("bake_round"),
            compilation_options: wgpu::PipelineCompilationOptions {
                constants: &[("RAND_HASHED", 1.0), ("EMISSION_UNIT", 1.0)],
                ..Default::default()
            },
            cache: None,
        });
        let job = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bake job"),
            size: JOB_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let tables = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bake job tables"),
            size: OPEN_JOBS as u64 * TABLE_SLOT_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let faces = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bake faces"),
            size: BAKE_BATCH_FACES as u64 * FACE_BYTES,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            layout,
            pipeline,
            job,
            faces,
            tables,
        }
    }
}

/// The candidate table of the job at `coord` (ADR-0025, centred on the job's chunk): the chunks
/// holding a range whose box lies within `light_radius` voxels of the job's box, the nearest
/// [`LIGHT_TABLE_ENTRIES`], in chunk-coordinate order, each weighing its power over
/// `max(d^2, 64^2)`, `d` the distance between the boxes.
pub(crate) fn job_table(world: &GpuWorld, coord: ChunkCoord) -> LightTableUniform {
    let reach = i64::from(world.light_radius()) * 64;
    let lo = coord.0.map(|v| i64::from(v) * 64);
    let gap2 = |c: ChunkCoord| -> i64 {
        (0..3)
            .map(|k| {
                let o = i64::from(c.0[k]) * 64;
                let gap = (o - (lo[k] + 64)).max(lo[k] - (o + 64)).max(0);
                gap * gap
            })
            .sum()
    };
    let mut near: Vec<(i64, ChunkCoord, u32, u32, f32)> = world
        .light_chunks()
        .map(|(c, start, len, power)| (gap2(c), c, start, len, power))
        .filter(|&(d2, ..)| d2 < reach * reach)
        .collect();
    near.sort_unstable_by_key(|&(d2, c, ..)| (d2, c.0));
    near.truncate(LIGHT_TABLE_ENTRIES);
    near.sort_unstable_by_key(|&(_, c, ..)| c.0);
    let mut t = LightTableUniform {
        count: near.len() as u32,
        total: 0.0,
        _pad: [0; 2],
        chunks: [[0; 4]; LIGHT_TABLE_ENTRIES],
    };
    let mut cdf = 0f32;
    for (i, &(d2, _, start, len, power)) in near.iter().enumerate() {
        cdf += power / (d2.max(64 * 64) as f32);
        t.chunks[i] = [start, len, cdf.to_bits(), 0];
    }
    t.total = cdf;
    t
}

#[allow(
    clippy::disallowed_methods,
    reason = "the bake times its submits for the count reading's cost rows and its progress"
)]
fn stopwatch() -> Instant {
    Instant::now()
}

#[allow(
    clippy::disallowed_methods,
    reason = "the bake times its submits for the count reading's cost rows and its progress"
)]
fn seconds_since(t: Instant) -> f64 {
    t.elapsed().as_secs_f64()
}

struct FaceState {
    voxel: [i32; 3],
    face: u8,
    key: u16,
    n: u32,
    sum: [f64; 3],
    clamped: [f64; 3],
    lum2: f64,
    best: f32,
    material: u16,
    snapshots: Vec<(u32, [f32; 3], [f32; 3])>,
    done: bool,
}

fn lum3(c: [f64; 3]) -> f64 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

impl FaceState {
    fn kept(&self, clamp: bool) -> [f64; 3] {
        if clamp { self.clamped } else { self.sum }
    }

    fn mean(&self, sums: [f64; 3]) -> [f32; 3] {
        let n = f64::from(self.n.max(1));
        sums.map(|s| (s / n) as f32)
    }

    fn se(&self, clamp: bool) -> f64 {
        se_levels(u64::from(self.n), lum3(self.kept(clamp)), self.lum2)
    }

    /// The first words of the working buffer's entry (`BakeFace` in `bake.wgsl`) for the round
    /// from sample `first`.
    fn input(&self, first: u32, samples: u32, slot: usize) -> [u32; 20] {
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

    /// The rounds left before `n_max`.
    fn rounds_left(&self, round: u32, n_max: u32) -> u32 {
        (n_max - self.n).div_ceil(round)
    }

    /// The rounds its stop is expected to need, from its standard error now, which falls as
    /// `1 / sqrt(n)`: 0 before its first round or once it reads under the stop.
    fn expected_rounds(&self, round: u32, n_max: u32, clamp: bool, se: f32) -> u32 {
        if self.n == 0 {
            return 0;
        }
        let ratio = self.se(clamp) / f64::from(se);
        let wanted = (f64::from(self.n) * ratio * ratio).min(f64::from(n_max));
        let more = (wanted - f64::from(self.n)).max(0.0);
        (more / f64::from(round)).ceil() as u32
    }

    /// The rounds none of which can end the face before the last: those up to `n_min`, or one.
    fn certain_rounds(&self, round: u32, n_min: u32) -> u32 {
        if self.n < n_min {
            (n_min - self.n).div_ceil(round)
        } else {
            1
        }
    }

    fn add(&mut self, out: &[u32], samples: u32) {
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

/// Bakes `extent`'s chunks into `store`, emptied first.
pub(crate) fn run(
    run: &BakeRun<'_>,
    store: &mut Store,
    extent: ChunkExtent,
    settings: &BakeSettings,
    progress: &mut dyn FnMut(&BakeProgress),
) -> Result<BakeReport, BakeError> {
    let (device, queue) = (run.device, run.queue);
    let opaque = opaque_table(run.world.materials());
    // The list, first: the pool's blocks are known before a sample is drawn.
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
                if let Some(faces) = list_world_chunk(run.world, c, &opaque)
                    && faces.faces() > 0
                {
                    jobs.push((c, faces));
                }
            }
        }
    }
    jobs.sort_unstable_by_key(|(c, _)| c.0);
    if settings.reversed {
        jobs.reverse();
    }
    let window = run.gpu.window();
    let size = window.size();
    let table_words = (size[0] as u64) * (size[1] as u64) * (size[2] as u64);
    let needed_words =
        STORE_TABLE + table_words + jobs.iter().map(|(_, f)| f.block_words()).sum::<u64>();
    let capacity = store.pool.size();
    if needed_words * 4 > capacity {
        return Err(BakeError::PoolFull {
            needed: needed_words * 4,
            capacity,
        });
    }
    // The header and an empty table.
    let mut header = vec![0u32; (STORE_TABLE + table_words) as usize];
    for k in 0..3 {
        header[4 + k] = window.min.0[k] as u32;
        header[8 + k] = size[k] as u32;
    }
    queue.write_buffer(&store.pool, 0, bytemuck::cast_slice(&header));
    store.blocks.clear();
    store.used_words = STORE_TABLE + table_words;

    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("bake"),
        layout: &run.pipeline.layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: run.pipeline.job.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: run.pipeline.faces.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: run.pipeline.tables.as_entire_binding(),
            },
        ],
    });
    // The frame's parameters, once: the bake reads no view but the LOD walk's camera chunk,
    // which it takes from the face's job.
    queue.write_buffer(run.params_buffer, 0, bytemuck::bytes_of(&run.params));
    let mut report = BakeReport {
        chunks: jobs.len() as u64,
        ..BakeReport::default()
    };
    let total_faces: u64 = jobs.iter().map(|(_, f)| f.faces() as u64).sum();
    let mut done = BakeProgress {
        chunks: jobs.len() as u64,
        faces: total_faces,
        ..BakeProgress::default()
    };
    let round = settings.round.max(1);
    let per_submit = (SUBMIT_SAMPLES / u64::from(round)).max(1) as usize;
    let batch = settings
        .batch_faces
        .clamp(1, BAKE_BATCH_FACES)
        .min(per_submit);
    let n_max = settings.n_max.max(1);
    let rounds_cap = if settings.submit_rounds == 0 {
        u32::MAX
    } else {
        settings.submit_rounds
    };
    let ctx = JobCtx {
        run,
        settings,
        round,
        n_max,
        rounds_cap,
        window_min: window.min,
        size,
    };
    // The blocks lie in the jobs' order, whatever order the jobs end in.
    let mut pending: VecDeque<(ChunkCoord, ChunkFaces, u64)> = VecDeque::new();
    let mut at = store.used_words;
    for (coord, faces) in jobs {
        let words = faces.block_words();
        pending.push_back((coord, faces, at));
        at += words;
    }
    let open_cap = if settings.jobs_alone { 1 } else { OPEN_JOBS };
    let mut free: Vec<usize> = (0..open_cap).rev().collect();
    let mut open: Vec<OpenJob> = Vec::new();
    loop {
        // The rounds each live face takes in this submit: first those that cannot end it.
        let mut entries = 0usize;
        let mut plan: Vec<Vec<(usize, u32)>> = open
            .iter()
            .map(|job| ctx.plan(job, batch, &mut entries))
            .collect();
        while entries < batch {
            let Some(slot) = free.pop() else { break };
            let Some((coord, faces, first_word)) = pending.pop_front() else {
                free.push(slot);
                break;
            };
            let job = ctx.open(coord, faces, first_word, slot);
            if job.live.is_empty() {
                free.push(slot);
                ctx.close(store, job, &mut report, &mut done);
                progress(&done);
                continue;
            }
            plan.push(ctx.plan(&job, batch, &mut entries));
            open.push(job);
        }
        // Room left and no job to open: further rounds, a layer at a time, in order, each face
        // up to the rounds its standard error now expects it to need.
        while entries < batch {
            let before = entries;
            for (job, p) in open.iter().zip(&mut plan) {
                for (i, r) in p.iter_mut() {
                    if entries == batch {
                        break;
                    }
                    let s = &job.states[*i];
                    let expected = s
                        .expected_rounds(round, n_max, settings.clamp, settings.se)
                        .max(1);
                    if *r < s.rounds_left(round, n_max).min(rounds_cap).min(expected) {
                        *r += 1;
                        entries += 1;
                    }
                }
            }
            if entries == before {
                break;
            }
        }
        if entries == 0 {
            break;
        }
        // One dispatch: each entry names its job's slot.
        let mut input: Vec<[u32; 20]> = Vec::with_capacity(entries);
        let mut jobs_in = 0u64;
        for (job, p) in open.iter().zip(&plan) {
            jobs_in += u64::from(!p.is_empty());
            for &(i, r) in p {
                let s = &job.states[i];
                report.max_rounds_in_submit = report.max_rounds_in_submit.max(r);
                for k in 0..r {
                    let first = s.n + k * round;
                    input.push(s.input(first, round.min(n_max - first), job.slot));
                }
            }
        }
        let count = input.len() as u32;
        let uniform = [
            count,
            settings.seed,
            ((run.gpu.light_radius() * 64) as f32).to_bits(),
            u32::from(settings.clamp),
            0,
            0,
            0,
            0,
        ];
        queue.write_buffer(&run.pipeline.job, 0, bytemuck::cast_slice(&uniform));
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
            pass.set_pipeline(&run.pipeline.pipeline);
            pass.set_bind_group(0, run.world_bind, &[]);
            pass.set_bind_group(3, &bind, &[]);
            pass.dispatch_workgroups(count.div_ceil(64), 1, 1);
        }
        report.max_jobs_in_submit = report.max_jobs_in_submit.max(jobs_in);
        queue.write_buffer(&run.pipeline.faces, 0, bytemuck::cast_slice(&input));
        let t = stopwatch();
        queue.submit([enc.finish()]);
        let bytes = readback::read_buffer(
            device,
            queue,
            &run.pipeline.faces,
            0,
            input.len() as u64 * FACE_BYTES,
        );
        report.gpu_seconds += seconds_since(t);
        report.submits += 1;
        // Each face's rounds in order: the stop is read after each, and a round past it is waste.
        let out: &[u32] = bytemuck::cast_slice(&bytes);
        let mut e = 0usize;
        for (job, p) in open.iter_mut().zip(&plan) {
            for &(i, r) in p {
                let s = &mut job.states[i];
                for _ in 0..r {
                    let words = &out[e * 20..(e + 1) * 20];
                    e += 1;
                    let samples = words[5];
                    if s.done {
                        report.wasted += u64::from(samples);
                        continue;
                    }
                    report.rays += u64::from(words[15]);
                    report.ray_len += f64::from(f32::from_bits(words[16]));
                    report.samples += u64::from(samples);
                    s.add(words, samples);
                    if settings.snapshots.contains(&s.n) {
                        let (a, b) = (s.mean(s.sum), s.mean(s.clamped));
                        s.snapshots.push((s.n, a, b));
                    }
                    let reached =
                        s.n >= settings.n_min && s.se(settings.clamp) <= f64::from(settings.se);
                    if reached || s.n >= n_max {
                        s.done = true;
                        report.faces_at_min += u64::from(s.n <= settings.n_min);
                        report.faces_at_max += u64::from(s.n >= n_max);
                        done.faces_done += 1;
                    }
                }
            }
        }
        done.samples = report.samples;
        done.gpu_seconds = report.gpu_seconds;
        for job in &mut open {
            let states = &job.states;
            job.live.retain(|&i| !states[i].done);
        }
        let mut k = 0;
        while k < open.len() {
            if open[k].live.is_empty() {
                let job = open.remove(k);
                free.push(job.slot);
                ctx.close(store, job, &mut report, &mut done);
            } else {
                k += 1;
            }
        }
        progress(&done);
    }
    queue.submit([]);
    Ok(report)
}

/// A chunk's job while its faces run.
struct OpenJob {
    coord: ChunkCoord,
    faces: ChunkFaces,
    first_word: u64,
    states: Vec<FaceState>,
    /// The faces still running.
    live: Vec<usize>,
    slot: usize,
}

/// What opening, planning and closing a job read of the bake.
struct JobCtx<'a, 'b> {
    run: &'a BakeRun<'b>,
    settings: &'a BakeSettings,
    round: u32,
    n_max: u32,
    rounds_cap: u32,
    window_min: ChunkCoord,
    size: [i32; 3],
}

impl JobCtx<'_, '_> {
    /// The job at `coord`: its faces' states, and its parameters and table staged in `slot`.
    fn open(&self, coord: ChunkCoord, faces: ChunkFaces, first_word: u64, slot: usize) -> OpenJob {
        let table = job_table(self.run.gpu, coord);
        let used = (1 + table.count as usize) * 16;
        let at = slot as u64 * TABLE_SLOT_BYTES;
        let tables = &self.run.pipeline.tables;
        let queue = self.run.queue;
        queue.write_buffer(tables, at, &bytemuck::bytes_of(&table)[..used]);
        let chunk = [coord.0[0], coord.0[1], coord.0[2], 0];
        queue.write_buffer(
            tables,
            at + (TABLE_SLOT_WORDS - 1) * 16,
            bytemuck::cast_slice(&chunk),
        );
        let mut states: Vec<FaceState> = Vec::with_capacity(faces.faces());
        for (brick_at, keys) in &faces.bricks {
            let b = brick_of_slot(usize::from(*brick_at));
            for &key in keys {
                let i = i32::from(key >> 3);
                let l = [i % 8, i / 8 % 8, i / 64];
                let voxel = [0, 1, 2].map(|k| coord.0[k] * 64 + b[k] * 8 + l[k]);
                let face = (key & 7) as u8;
                let wanted = self.settings.filter.as_ref().is_none_or(|f| f(voxel, face));
                states.push(FaceState {
                    voxel,
                    face,
                    key,
                    n: 0,
                    sum: [0.0; 3],
                    clamped: [0.0; 3],
                    lum2: 0.0,
                    best: 0.0,
                    material: 0,
                    snapshots: Vec::new(),
                    done: !wanted,
                });
            }
        }
        let live = (0..states.len()).filter(|&i| !states[i].done).collect();
        OpenJob {
            coord,
            faces,
            first_word,
            states,
            live,
            slot,
        }
    }

    /// The rounds of `job`'s live faces that cannot end them, while `entries` is under `batch`.
    fn plan(&self, job: &OpenJob, batch: usize, entries: &mut usize) -> Vec<(usize, u32)> {
        let mut p = Vec::new();
        for &i in &job.live {
            if *entries == batch {
                break;
            }
            let s = &job.states[i];
            let r = s
                .certain_rounds(self.round, self.settings.n_min)
                .min(s.rounds_left(self.round, self.n_max))
                .min(self.rounds_cap)
                .min((batch - *entries) as u32);
            p.push((i, r));
            *entries += r as usize;
        }
        p
    }

    /// Writes the ended job's block, the directory then its cells in key order, at its place.
    fn close(
        &self,
        store: &mut Store,
        job: OpenJob,
        report: &mut BakeReport,
        done: &mut BakeProgress,
    ) {
        let queue = self.run.queue;
        let mut block: Vec<u32> = job.faces.directory();
        block.reserve(job.states.len() * 2);
        for s in &job.states {
            let mean = s.mean(s.kept(self.settings.clamp));
            block.push(rgb9e5_pack(mean));
            block.push(u32::from(s.key) | u32::from(s.material) << 16);
        }
        debug_assert_eq!(block.len() as u64, job.faces.block_words());
        queue.write_buffer(
            &store.pool,
            job.first_word * 4,
            bytemuck::cast_slice(&block),
        );
        let w = [0, 1, 2].map(|k| i64::from(job.coord.0[k]) - i64::from(self.window_min.0[k]));
        let index = w[0] + i64::from(self.size[0]) * (w[1] + i64::from(self.size[1]) * w[2]);
        queue.write_buffer(
            &store.pool,
            (STORE_TABLE + index as u64) * 4,
            bytemuck::bytes_of(&(job.first_word as u32)),
        );
        store
            .blocks
            .insert(job.coord.0, (job.first_word, block.len() as u64));
        store.used_words += block.len() as u64;
        report.faces += job.states.iter().filter(|s| s.n > 0).count() as u64;
        if self.settings.keep_faces {
            report
                .outcomes
                .extend(job.states.iter().filter(|s| s.n > 0).map(|s| FaceOutcome {
                    voxel: s.voxel,
                    face: s.face,
                    samples: s.n,
                    mean: [s.mean(s.sum), s.mean(s.clamped)],
                    se: s.se(self.settings.clamp) as f32,
                    material: s.material,
                    snapshots: s.snapshots.clone(),
                }));
        }
        done.chunks_done += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vxe_world::MaterialId;

    /// An xorshift, seeded.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    /// A chunk whose bricks mix air, glass, rock and uniform runs by the seed. Material 1 and 2
    /// are opaque, 3 is translucent.
    fn mixed_chunk(seed: u64) -> Chunk {
        let mut rng = Rng(seed | 1);
        let mut c = Chunk::uniform(AIR);
        for slot in 0..CHUNK_BRICKS {
            let brick = match rng.next() % 5 {
                0 => Brick::Uniform(AIR),
                1 => Brick::Uniform(1),
                2 => Brick::Uniform(3),
                _ => {
                    let density = rng.next() % 4;
                    Brick::from_fn(|_| match (rng.next() % 4).max(density) {
                        0 => AIR,
                        1 => 1 as MaterialId,
                        2 => 2,
                        _ => {
                            if rng.next().is_multiple_of(2) {
                                3
                            } else {
                                AIR
                            }
                        }
                    })
                }
            };
            c.set_brick(slot, brick);
        }
        c
    }

    fn material(chunk: &Chunk, l: [i32; 3]) -> MaterialId {
        let b = l.map(|v| v / 8);
        let i = l.map(|v| v % 8);
        chunk
            .brick(brick_slot(b))
            .get((i[0] + 8 * (i[1] + 8 * i[2])) as usize)
    }

    /// The bake's module, with the table and LOD reads replaced, parses and validates.
    #[test]
    fn the_bake_module_reads_the_entrys_job() {
        let source = bake_source();
        assert!(!source.contains(TABLE_READ) && !source.contains(LOD_ORIGIN));
        let module = wgpu::naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("the bake's module validates");
    }

    #[test]
    fn every_exposed_face_has_one_key() {
        let mut opaque = vec![false; 1 << 16];
        opaque[1] = true;
        opaque[2] = true;
        let chunk = mixed_chunk(0x9E37_79B9_7F4A_7C15);
        let beside: [Chunk; 6] = std::array::from_fn(|f| mixed_chunk(0x1234_5678 + 97 * f as u64));
        let refs: [Option<&Chunk>; 6] = std::array::from_fn(|f| Some(&beside[f]));
        let faces = list_chunk_faces(&chunk, &refs, &opaque);
        let mut listed: Vec<(usize, u16)> = Vec::new();
        for (slot, keys) in &faces.bricks {
            assert!(!keys.is_empty(), "brick {slot} listed with no face");
            assert!(
                keys.windows(2).all(|w| w[0] < w[1]),
                "brick {slot} unsorted"
            );
            listed.extend(keys.iter().map(|&k| (usize::from(*slot), k)));
        }
        let mut scan: Vec<(usize, u16)> = Vec::new();
        for z in 0..64 {
            for y in 0..64 {
                for x in 0..64 {
                    let l = [x, y, z];
                    if !opaque[usize::from(material(&chunk, l))] {
                        continue;
                    }
                    let inner = l.map(|v| v % 8);
                    let key = ((inner[0] + 8 * (inner[1] + 8 * inner[2])) << 3) as u16;
                    for (face, side) in beside.iter().enumerate() {
                        let k = face / 2;
                        let mut n = l;
                        n[k] += if face % 2 == 1 { 1 } else { -1 };
                        let m = if (0..64).contains(&n[k]) {
                            material(&chunk, n)
                        } else {
                            n[k] = n[k].rem_euclid(64);
                            material(side, n)
                        };
                        if !opaque[usize::from(m)] {
                            scan.push((brick_slot(l.map(|v| v / 8)), key | face as u16));
                        }
                    }
                }
            }
        }
        scan.sort_unstable();
        listed.sort_unstable();
        assert!(!scan.is_empty());
        assert_eq!(
            listed.len(),
            scan.len(),
            "listed {} found {}",
            listed.len(),
            scan.len()
        );
        assert_eq!(listed, scan);
    }

    #[test]
    fn the_directory_points_at_each_bricks_run() {
        let faces = ChunkFaces {
            bricks: vec![(3, vec![1, 2, 3]), (9, vec![4])],
        };
        let d = faces.directory();
        assert_eq!(d[3], 512 << 11 | 3);
        assert_eq!(d[9], 518 << 11 | 1);
        assert_eq!(d[4], 0);
        assert_eq!(faces.block_words(), 512 + 8);
    }

    #[test]
    fn rgb9e5_holds_a_value_within_its_step() {
        for c in [
            [1.0f32, 0.5, 0.25],
            [0.003, 0.002, 0.001],
            [1234.5, 3.0, 800.0],
            [1e-6, 0.0, 0.0],
        ] {
            let d = rgb9e5_unpack(rgb9e5_pack(c));
            let top = c[0].max(c[1]).max(c[2]);
            for k in 0..3 {
                // Half a step: a 9-bit mantissa, over the format's floor of 2^-24.
                let half_step = (top / 256.0).max(2f32.powi(-25));
                assert!((d[k] - c[k]).abs() <= half_step, "{c:?} read {d:?}");
            }
        }
        assert_eq!(rgb9e5_pack([0.0; 3]), 0);
        assert_eq!(rgb9e5_unpack(0), [0.0; 3]);
    }

    #[test]
    fn the_standard_error_reads_zero_for_a_constant_and_falls_with_the_count() {
        assert_eq!(se_levels(1024, 0.0, 0.0), 0.0);
        let constant = se_levels(1024, 1024.0 * 0.01, 1024.0 * 0.0001);
        assert!(constant.abs() < 1e-3, "{constant}");
        let at = |n: u64| se_levels(n, n as f64 * 0.01, n as f64 * 0.0002);
        assert!(at(4096) < at(1024) * 0.51, "{} {}", at(4096), at(1024));
    }
}
