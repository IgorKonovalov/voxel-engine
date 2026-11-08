//! The loaded light's store and its bake (ADR-0046): every face of a chunk holds its emissive
//! irradiance at unit emission in a cell of one pool, computed ahead of the frames that read it.
//!
//! **The list.** [`list_chunk_faces`] is a pure function of a chunk's voxels, its six neighbours'
//! and each material's class ([`face_classes`]): each surface brick's sorted face keys, an opaque
//! voxel's faces toward a voxel that is not opaque and a translucent voxel's faces toward air
//! (ADR-0047: a pane's two sides), the latter flagged [`CELL_TRANSLUCENT`] in the cell's tail.
//!
//! **The pool** ([`Store`]), in `u32` words: the missing count (word 0), the mirror window's
//! origin (word 4) and size (word 8) in chunks, then from word [`STORE_TABLE`] one word per
//! window chunk naming its block's first word (0 none). A block is 512 directory words, a brick's
//! `run offset from the block's start << 11 | faces` (0 no surface), then the runs: two words a
//! cell, its [`rgb9e5_pack`] irradiance, then `key | flags << 12 | dominant material << 16`, with
//! the key `voxel in the brick << 3 | face`, sorted by key, and flag bit 0 ([`CELL_TRANSLUCENT`])
//! set on a translucent voxel's face.
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
//!
//! **The cuts** (Plan 0059 Phase 4). [`SeedForm`] names how faces share their random sequences:
//! the reference's independent sequences, or one a chunk and side, one a side, or one a side
//! rotated per face by a lattice of its plane coordinates (`bake_seeds.wgsl`). The independent
//! form's module is the reference's, unchanged, so its [`source_hash`] and its files stand.
//! [`BakeSettings::tent`] passes each cell through [`tent`] after the bake, from the bake's own
//! values ([`Store`] keeps them), which a dig's re-bake filters again around its ring.

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

/// A cell tail's flag: the face is a translucent voxel's (ADR-0047).
pub const CELL_TRANSLUCENT: u32 = 1 << 12;

/// [`face_classes`]' class of air.
pub const CLASS_AIR: u8 = 0;
/// [`face_classes`]' class of a material that stops light.
pub const CLASS_OPAQUE: u8 = 1;
/// [`face_classes`]' class of a translucent material: air to the walk and the shadow rays, a
/// filter to the view.
pub const CLASS_TRANSLUCENT: u8 = 2;

/// Which materials stop light: not air, not translucent.
pub fn opaque_table(materials: &MaterialTable) -> Vec<bool> {
    face_classes(materials)
        .iter()
        .map(|&c| c == CLASS_OPAQUE)
        .collect()
}

/// Each material id's class: [`CLASS_AIR`], [`CLASS_OPAQUE`] or [`CLASS_TRANSLUCENT`].
pub fn face_classes(materials: &MaterialTable) -> Vec<u8> {
    let mut t = vec![CLASS_AIR; 1 << 16];
    for (id, m) in materials.rows().iter().enumerate() {
        if id != usize::from(AIR) {
            t[id] = if m.is_translucent() {
                CLASS_TRANSLUCENT
            } else {
                CLASS_OPAQUE
            };
        }
    }
    t
}

/// A chunk's exposed faces, by surface brick in slot order, each brick's keys ascending. A key is
/// `voxel in the brick << 3 | face`, face `2 axis + 1` toward `+axis`. `glass` holds the
/// `(brick slot, key)` of every translucent voxel's face among them, ascending.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChunkFaces {
    pub bricks: Vec<(u16, Vec<u16>)>,
    pub glass: Vec<(u16, u16)>,
}

impl ChunkFaces {
    /// Faces over the chunk's bricks.
    pub fn faces(&self) -> usize {
        self.bricks.iter().map(|(_, k)| k.len()).sum()
    }

    /// Whether the face `key` of brick `slot` is a translucent voxel's.
    pub fn is_glass(&self, slot: u16, key: u16) -> bool {
        self.glass.binary_search(&(slot, key)).is_ok()
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

/// A brick's opaque voxels and its solid ones (opaque or translucent): bit `x + 8 y` of word `z`.
fn class_masks(brick: &Brick, classes: &[u8]) -> ([u64; 8], [u64; 8]) {
    match brick {
        Brick::Uniform(m) => match classes[usize::from(*m)] {
            CLASS_OPAQUE => (ALL_VOXELS, ALL_VOXELS),
            CLASS_TRANSLUCENT => (NO_VOXEL, ALL_VOXELS),
            _ => (NO_VOXEL, NO_VOXEL),
        },
        Brick::Full(p) => {
            let mut v = [AIR; BRICK_VOXELS];
            p.decode(&mut v);
            let (mut opaque, mut solid) = (NO_VOXEL, NO_VOXEL);
            for (i, m) in v.iter().enumerate() {
                let c = classes[usize::from(*m)];
                if c == CLASS_OPAQUE {
                    opaque[i / 64] |= 1 << (i % 64);
                }
                if c != CLASS_AIR {
                    solid[i / 64] |= 1 << (i % 64);
                }
            }
            (opaque, solid)
        }
    }
}

/// Whether the voxel beside `l` (in the brick) across `face` is set, in `mask` within the brick or
/// in `side`, the neighbour brick's mask across that face.
fn beside_bit(mask: &[u64; 8], side: &[u64; 8], l: [usize; 3], face: usize) -> bool {
    let k = face / 2;
    let mut q = l;
    if face % 2 == 1 {
        if l[k] == 7 {
            q[k] = 0;
            return mask_bit(side, q);
        }
        q[k] += 1;
    } else {
        if l[k] == 0 {
            q[k] = 7;
            return mask_bit(side, q);
        }
        q[k] -= 1;
    }
    mask_bit(mask, q)
}

fn mask_bit(mask: &[u64; 8], l: [usize; 3]) -> bool {
    mask[l[2]] >> (l[0] + 8 * l[1]) & 1 != 0
}

/// `chunk`'s faces, across brick and chunk borders: an opaque voxel's toward a voxel that is not
/// opaque, and a translucent voxel's toward air (`classes`, [`face_classes`]). The neighbours are
/// in face order (`-x, +x, -y, +y, -z, +z`); across a neighbour that is `None` no ray arrives (an
/// absent chunk is rock to the walk), so no face is listed there.
pub fn list_chunk_faces(
    chunk: &Chunk,
    neighbours: &[Option<&Chunk>; 6],
    classes: &[u8],
) -> ChunkFaces {
    let masks: Vec<([u64; 8], [u64; 8])> = chunk
        .bricks()
        .iter()
        .map(|b| class_masks(b, classes))
        .collect();
    let mut out = ChunkFaces::default();
    for slot in 0..CHUNK_BRICKS {
        let (opaque, solid) = masks[slot];
        if solid == NO_VOXEL {
            continue;
        }
        let b = brick_of_slot(slot);
        let beside: [([u64; 8], [u64; 8]); 6] = std::array::from_fn(|face| {
            let (k, d) = (face / 2, if face % 2 == 1 { 1 } else { -1 });
            let mut n = b;
            n[k] += d;
            if (0..8).contains(&n[k]) {
                return masks[brick_slot(n)];
            }
            n[k] = n[k].rem_euclid(8);
            match neighbours[face] {
                Some(c) => class_masks(c.brick(brick_slot(n)), classes),
                None => (ALL_VOXELS, ALL_VOXELS),
            }
        });
        if opaque == ALL_VOXELS && beside.iter().all(|m| m.0 == ALL_VOXELS) {
            continue;
        }
        let mut keys = Vec::new();
        for i in 0..BRICK_VOXELS {
            let l = [i % 8, i / 8 % 8, i / 64];
            // An opaque voxel's face is closed by an opaque one; a translucent voxel's by any
            // solid one, so a pane lists its two sides toward air and nothing inside it.
            let (own, glass) = if mask_bit(&opaque, l) {
                (&opaque, false)
            } else if mask_bit(&solid, l) {
                (&solid, true)
            } else {
                continue;
            };
            for (face, side) in beside.iter().enumerate() {
                let side = if glass { &side.1 } else { &side.0 };
                if !beside_bit(own, side, l, face) {
                    let key = (i as u16) << 3 | face as u16;
                    keys.push(key);
                    if glass {
                        out.glass.push((slot as u16, key));
                    }
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
pub fn list_world_chunk(world: &World, coord: ChunkCoord, classes: &[u8]) -> Option<ChunkFaces> {
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
    Some(list_chunk_faces(&c, &refs, classes))
}

/// A test of a voxel and its face.
pub type FaceFilter = Arc<dyn Fn([i32; 3], u8) -> bool + Send + Sync>;

/// Words of a brick-level cell's record ([`Rings`]): six direction values in [`rgb9e5_pack`], face
/// order, then `material | direction mask << 16`, the mask's bit `face` set where the cell has an
/// opaque face of that direction. A brick's record is its cells' records in cell order
/// ([`Rings::record_words`]): one cell of 8 voxels, or eight of 4 (`x + 2 y + 4 z`).
pub const BRICK_WORDS: usize = 7;
/// The store header's flags word: bit 0 ([`FLAG_BRICKS`]) where every block carries brick records,
/// bit 1 ([`FLAG_CELL4`]) where their cells are 4 voxels.
pub const STORE_FLAGS: u64 = 12;
/// The blocks carry a brick record before each surface brick's run.
pub const FLAG_BRICKS: u32 = 1;
/// The brick records hold eight cells of 4 voxels.
pub const FLAG_CELL4: u32 = 2;
/// A brick of more faces than this holds no cells: its faces read its brick record.
pub const BRICK_FACES_MAX: usize = 512;
/// Samples a brick-level direction draws by default, spread over its faces (at least one a face).
pub const BRICK_LEVEL_SAMPLES: u32 = 64;
/// The brick level's cell by default, in voxels: the brick.
pub const BRICK_CELL: u32 = 8;

/// What a chunk of a ring store holds: face cells at a count, or the brick level alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    Face(u32),
    Brick,
}

/// The rings of a reach cut (ADR-0046): around `centre`, chunks within `faces[i].0` chunks
/// (Chebyshev, the first ring that holds them) hold face cells at `faces[i].1` samples, and the
/// rest within `brick` hold the brick level. Every block carries brick records, each of cells of
/// `cell` voxels (8 or 4), each brick direction drawing `brick_samples` at the brick level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rings {
    pub centre: [i32; 3],
    pub brick: i32,
    /// `(radius, samples)`, inner rings first.
    pub faces: Vec<(i32, u32)>,
    /// The brick level's cell in voxels: 8 (the brick) or 4.
    pub cell: u32,
    /// Samples a brick direction draws at the brick level, spread over its faces.
    pub brick_samples: u32,
}

impl Rings {
    /// Cells a brick side holds: 1 or 2.
    pub fn cells_a_side(&self) -> u32 {
        8 / self.cell
    }

    /// Words of a brick's record: [`BRICK_WORDS`] a cell.
    pub fn record_words(&self) -> usize {
        record_words(self.cell)
    }

    /// The tier of `c`, `None` beyond the brick ring.
    pub fn tier(&self, c: [i32; 3]) -> Option<Tier> {
        let d = (0..3).map(|k| (c[k] - self.centre[k]).abs()).max()?;
        if d > self.brick {
            return None;
        }
        Some(
            self.faces
                .iter()
                .find(|(r, _)| d <= *r)
                .map_or(Tier::Brick, |&(_, n)| Tier::Face(n)),
        )
    }

    /// The chunks within the brick ring, clipped to `world`.
    pub fn extent(&self, world: ChunkExtent) -> ChunkExtent {
        let size = world.size();
        let lo = [0, 1, 2].map(|k| (self.centre[k] - self.brick).max(world.min.0[k]));
        let hi = [0, 1, 2].map(|k| (self.centre[k] + self.brick + 1).min(world.min.0[k] + size[k]));
        ChunkExtent::new(lo, [0, 1, 2].map(|k| hi[k].max(lo[k])))
    }
}

/// Words of a brick's record of cells of `cell` voxels: [`BRICK_WORDS`] a cell.
pub fn record_words(cell: u32) -> usize {
    BRICK_WORDS * (8 / cell).pow(3) as usize
}

/// Whether brick `keys` of a chunk at `tier` holds cells.
fn holds_cells(tier: Option<Tier>, keys: usize) -> bool {
    match tier {
        None => true,
        Some(Tier::Brick) => false,
        Some(Tier::Face(_)) => keys <= BRICK_FACES_MAX,
    }
}

/// The block's directory and its words: plain ([`ChunkFaces::directory`]) at `tier` `None`; else
/// each surface brick's record of `record` words, then its cells where [`holds_cells`], the
/// directory pointing at the cells (a count of 0 for a brick at the brick level).
fn layout(faces: &ChunkFaces, tier: Option<Tier>, record: usize) -> (Vec<u32>, u64) {
    if tier.is_none() {
        return (faces.directory(), faces.block_words());
    }
    let mut dir = vec![0u32; DIRECTORY_WORDS as usize];
    let mut at = DIRECTORY_WORDS as u32;
    for (slot, keys) in &faces.bricks {
        at += record as u32;
        let n = if holds_cells(tier, keys.len()) {
            keys.len() as u32
        } else {
            0
        };
        dir[usize::from(*slot)] = at << 11 | n;
        at += 2 * n;
    }
    (dir, u64::from(at))
}

/// The brick records of `record` words of a block with them, by brick slot.
fn records_of(block: &[u32], record: usize) -> KeptBricks {
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
    /// The faces' random sequences.
    pub form: SeedForm,
    /// Each cell passes through [`tent`] once the bake ends.
    pub tent: bool,
    /// Times each submit's dispatch on the GPU into [`BakeReport::dispatch_seconds`] (a reading's
    /// cost rows).
    pub timer: Option<Arc<crate::marcher::PassTimer>>,
    /// A reach cut: each chunk at its ring's tier, every block with brick records; `n_min` and
    /// `n_max` are then ignored, each face taking its tier's count.
    pub rings: Option<Rings>,
}

/// Candidates a sample resamples in the reference: the arm the count reading keeps.
pub const KEPT_CANDIDATES: u32 = 32;
/// Whether the reference clamps a sample: the arm the count reading keeps.
pub const KEPT_CLAMP: bool = false;

/// How the faces' random sequences are seeded (Plan 0059 Phase 4's arms).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SeedForm {
    /// A sequence per face, keyed by its key: the reference's.
    #[default]
    Independent,
    /// One sequence per chunk and side.
    ChunkSide,
    /// One sequence per side, for every chunk.
    GlobalSide,
    /// The global-side sequence rotated per face by a lattice of its plane coordinates.
    Lattice,
}

impl SeedForm {
    pub const ALL: [Self; 4] = [
        Self::Independent,
        Self::ChunkSide,
        Self::GlobalSide,
        Self::Lattice,
    ];

    /// The form's name, as a flag takes it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Independent => "independent",
            Self::ChunkSide => "chunk-side",
            Self::GlobalSide => "global-side",
            Self::Lattice => "lattice",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.name() == s)
    }

    fn index(self) -> usize {
        self as usize
    }
}

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
            form: SeedForm::Independent,
            tent: false,
            timer: None,
            rings: None,
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
    /// Seconds of the dispatches on the GPU's timestamps, under [`BakeSettings::timer`].
    pub dispatch_seconds: f64,
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
    /// Lighting pixels of the last encode whose cube face had no cell (nor brick record).
    pub missing: u32,
    /// Lighting pixels of the last encode whose cube face read its brick record.
    pub brick_reads: u32,
}

/// The pool and where its blocks lie, with a copy of every block's words for a dig's kept cells
/// and the store file.
pub struct Store {
    pool: wgpu::Buffer,
    blocks: BTreeMap<[i32; 3], (u64, u64)>,
    words: BTreeMap<[i32; 3], Vec<u32>>,
    /// The header's and the chunk table's words.
    head_words: u64,
    /// The first word no block holds: a block that outgrows its place moves here.
    end_words: u64,
    /// The chunks the last bake or load covered; a dig re-bakes only inside them.
    piece: Option<ChunkExtent>,
    /// The mirror window the chunk table indexes.
    window: Option<ChunkExtent>,
    /// Under a bake with [`BakeSettings::tent`], every block's words before the filter, from
    /// which [`Store::apply_tent`] filters and a dig keeps its cells; else empty.
    raw: BTreeMap<[i32; 3], Vec<u32>>,
    /// The blocks are filtered ([`BakeSettings::tent`]).
    tent: bool,
    /// The blocks carry brick records ([`Rings`]) of cells of this many voxels.
    bricks: Option<u32>,
}

/// A store's content off the pool: its piece and blocks, and the raw words under the filter, for
/// a frame to swap stores in place ([`Frame::swap_store_image`](crate::Frame::swap_store_image)).
#[derive(Clone, Debug, Default)]
pub struct StoreImage {
    pub piece: Option<ChunkExtent>,
    pub blocks: Vec<(ChunkCoord, Vec<u32>)>,
    pub raw: Vec<(ChunkCoord, Vec<u32>)>,
    pub tent: bool,
    /// The blocks carry brick records of cells of this many voxels.
    pub bricks: Option<u32>,
}

impl StoreImage {
    /// Bytes of the blocks.
    pub fn bytes(&self) -> u64 {
        self.blocks.iter().map(|(_, b)| b.len() as u64 * 4).sum()
    }
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
            words: BTreeMap::new(),
            head_words: 0,
            end_words: 0,
            piece: None,
            window: None,
            raw: BTreeMap::new(),
            tent: false,
            bricks: None,
        }
    }

    /// The pool buffer.
    pub fn pool(&self) -> &wgpu::Buffer {
        &self.pool
    }

    /// The counters, `missing` as the caller read it.
    pub fn stats(&self, missing: u32, brick_reads: u32) -> StoreStats {
        StoreStats {
            chunks: self.blocks.len() as u64,
            bytes: (self.head_words + self.blocks.values().map(|b| b.1).sum::<u64>()) * 4,
            missing,
            brick_reads,
        }
    }

    /// Whether the blocks carry brick records.
    pub fn bricks(&self) -> bool {
        self.bricks.is_some()
    }

    /// The brick records' cell in voxels, `None` without records.
    pub fn brick_cell(&self) -> Option<u32> {
        self.bricks
    }

    /// Sets the header's flags word for blocks with brick records of cells of `cell` voxels, or
    /// without.
    fn set_bricks(&mut self, queue: &wgpu::Queue, cell: Option<u32>) {
        self.bricks = cell;
        let flags = match cell {
            None => 0,
            Some(4) => FLAG_BRICKS | FLAG_CELL4,
            Some(_) => FLAG_BRICKS,
        };
        queue.write_buffer(&self.pool, STORE_FLAGS * 4, bytemuck::bytes_of(&flags));
    }

    /// `coord`'s block: its first word and its words.
    pub fn block(&self, coord: ChunkCoord) -> Option<(u64, u64)> {
        self.blocks.get(&coord.0).copied()
    }

    /// `coord`'s block as its words, the directory first, from the store's own copy.
    pub fn block_words(&self, coord: ChunkCoord) -> Option<&[u32]> {
        self.words.get(&coord.0).map(Vec::as_slice)
    }

    /// The chunks the last bake or load covered.
    pub fn piece(&self) -> Option<ChunkExtent> {
        self.piece
    }

    /// Whether the blocks are filtered ([`BakeSettings::tent`]).
    pub fn tent(&self) -> bool {
        self.tent
    }

    /// The store's content, for [`StoreImage`]'s swap.
    pub fn image(&self) -> StoreImage {
        let list = |m: &BTreeMap<[i32; 3], Vec<u32>>| {
            m.iter().map(|(c, b)| (ChunkCoord(*c), b.clone())).collect()
        };
        StoreImage {
            piece: self.piece,
            blocks: list(&self.words),
            raw: list(&self.raw),
            tent: self.tent,
            bricks: self.bricks,
        }
    }

    /// Takes `raw` as the blocks' words before the filter and writes each of `chunks`' blocks
    /// through [`tent`] in its place; the rest of the pool is untouched.
    fn apply_tent(&mut self, queue: &wgpu::Queue, chunks: impl IntoIterator<Item = [i32; 3]>) {
        self.tent = true;
        let mut out = Vec::new();
        for c in chunks {
            if let Some(block) = tent_block(&self.raw, c) {
                out.push((c, block));
            }
        }
        for (c, block) in out {
            let (first, words) = self.blocks[&c];
            debug_assert_eq!(words, block.len() as u64);
            queue.write_buffer(&self.pool, first * 4, bytemuck::cast_slice(&block));
            self.words.insert(c, block);
        }
    }

    /// Empties the store for `window`'s chunk table: the header written, no block.
    fn reset(&mut self, queue: &wgpu::Queue, window: ChunkExtent) {
        let size = window.size();
        let table_words = (size[0] as u64) * (size[1] as u64) * (size[2] as u64);
        let mut header = vec![0u32; (STORE_TABLE + table_words) as usize];
        for k in 0..3 {
            header[4 + k] = window.min.0[k] as u32;
            header[8 + k] = size[k] as u32;
        }
        queue.write_buffer(&self.pool, 0, bytemuck::cast_slice(&header));
        self.blocks.clear();
        self.words.clear();
        self.raw.clear();
        self.tent = false;
        self.bricks = None;
        self.head_words = STORE_TABLE + table_words;
        self.end_words = self.head_words;
        self.piece = None;
        self.window = Some(window);
    }

    /// The chunk table's word for `coord`.
    fn table_word(&self, coord: ChunkCoord) -> u64 {
        let window = self.window.expect("a reset store");
        let size = window.size();
        let w = [0, 1, 2].map(|k| i64::from(coord.0[k]) - i64::from(window.min.0[k]));
        let index = w[0] + i64::from(size[0]) * (w[1] + i64::from(size[1]) * w[2]);
        STORE_TABLE + index as u64
    }

    /// Writes `block` as `coord`'s at `first`, and points the chunk table at it.
    fn put(&mut self, queue: &wgpu::Queue, coord: ChunkCoord, first: u64, block: Vec<u32>) {
        queue.write_buffer(&self.pool, first * 4, bytemuck::cast_slice(&block));
        queue.write_buffer(
            &self.pool,
            self.table_word(coord) * 4,
            bytemuck::bytes_of(&(first as u32)),
        );
        self.end_words = self.end_words.max(first + block.len() as u64);
        self.blocks.insert(coord.0, (first, block.len() as u64));
        self.words.insert(coord.0, block);
    }

    /// Drops `coord`'s block: the chunk table reads none.
    fn remove(&mut self, queue: &wgpu::Queue, coord: ChunkCoord) {
        if self.blocks.remove(&coord.0).is_some() {
            self.words.remove(&coord.0);
            self.raw.remove(&coord.0);
            queue.write_buffer(
                &self.pool,
                self.table_word(coord) * 4,
                bytemuck::bytes_of(&0u32),
            );
        }
    }

    /// Where a block of `words` for `coord` goes: its old place when it fits, else at `end`,
    /// which moves past it.
    fn place(&self, coord: ChunkCoord, words: u64, end: &mut u64) -> Result<u64, BakeError> {
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

/// What a store file was baked from: a file loads only where every field matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StoreKey {
    /// The world's generator seed.
    pub world_seed: u64,
    /// The world's chunks, `min` and exclusive `max`.
    pub world: [[i32; 3]; 2],
    /// The piece's chunks, `min` and exclusive `max`.
    pub piece: [[i32; 3]; 2],
    /// The piece's centre chunk and its radius in chunks.
    pub centre: [i32; 3],
    pub radius: i32,
    /// The mirror's light radius, in chunks.
    pub light_radius: i32,
    /// The arm: candidates and the clamp.
    pub candidates: u32,
    pub clamp: bool,
    /// The stop: the standard error's bits, `n_min`, `n_max`, the round and the seed.
    pub se_bits: u32,
    pub n_min: u32,
    pub n_max: u32,
    pub round: u32,
    pub seed: u32,
    /// [`source_hash_for`] the form.
    pub source_hash: u64,
    /// The cut: the seed form and the filter.
    pub form: SeedForm,
    pub tent: bool,
    /// A reach cut's rings ([`RingKey::of`]).
    pub rings: Option<RingKey>,
}

/// A reach cut's rings in a store file's key: the brick ring, up to three face rings inner first
/// (radius -1 and 0 samples where absent), and the brick level's samples a direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RingKey {
    pub brick: i32,
    pub radii: [i32; 3],
    pub samples: [u32; 3],
    pub brick_samples: u32,
    /// The brick level's cell in voxels; a field of the file only where it is not 8.
    pub cell: u32,
}

impl RingKey {
    /// `rings`' key; `None` for more than three face rings.
    pub fn of(rings: &Rings) -> Option<Self> {
        if rings.faces.len() > 3 {
            return None;
        }
        let mut radii = [-1; 3];
        let mut samples = [0; 3];
        for (i, &(r, n)) in rings.faces.iter().enumerate() {
            radii[i] = r;
            samples[i] = n;
        }
        Some(Self {
            brick: rings.brick,
            radii,
            samples,
            brick_samples: rings.brick_samples,
            cell: rings.cell,
        })
    }
}

/// The reference's files (independent seeds, no filter): the key's fields up to the source hash.
const STORE_MAGIC: &[u8; 8] = b"VXEBAKE1";
/// A cut's files: the seed form and the filter follow the source hash, and the filter's files
/// hold each block's words before the filter after the blocks.
const CUT_MAGIC: &[u8; 8] = b"VXEBAKE2";
/// A reach cut's files: a cut's fields, then the rings', and blocks with brick records.
const RING_MAGIC: &[u8; 8] = b"VXEBAKE3";

/// The hash a store file keys the bake's module by: FNV-1a of [`bake_source`].
pub fn source_hash() -> u64 {
    source_hash_for(SeedForm::Independent)
}

/// FNV-1a of [`bake_source_for`] `form`.
pub fn source_hash_for(form: SeedForm) -> u64 {
    let mut h = vxe_world::hash::Fnv1a::default();
    h.write(bake_source_for(form).as_bytes());
    h.finish()
}

impl StoreKey {
    fn magic(&self) -> &'static [u8; 8] {
        if self.rings.is_some() {
            RING_MAGIC
        } else if self.form == SeedForm::Independent && !self.tent {
            STORE_MAGIC
        } else {
            CUT_MAGIC
        }
    }

    /// The fields in file order, each with its name.
    fn fields(&self) -> Vec<(&'static str, u64)> {
        let w = |v: i32| u64::from(v as u32);
        let mut f = vec![("world seed", self.world_seed)];
        for (name, v) in [("world", self.world), ("piece", self.piece)] {
            f.extend(v.iter().flatten().map(|x| (name, w(*x))));
        }
        f.extend(self.centre.map(|v| ("piece centre", w(v))));
        f.extend([
            ("piece radius", w(self.radius)),
            ("light radius", w(self.light_radius)),
            ("candidates", u64::from(self.candidates)),
            ("clamp", u64::from(self.clamp)),
            ("stop's standard error", u64::from(self.se_bits)),
            ("n_min", u64::from(self.n_min)),
            ("n_max", u64::from(self.n_max)),
            ("round", u64::from(self.round)),
            ("seed", u64::from(self.seed)),
            ("bake source hash", self.source_hash),
        ]);
        if self.magic() != STORE_MAGIC {
            f.extend([
                ("seed form", self.form.index() as u64),
                ("filter", u64::from(self.tent)),
            ]);
        }
        if let Some(r) = self.rings {
            f.push(("brick ring", w(r.brick)));
            f.extend(r.radii.map(|v| ("face rings", w(v))));
            f.extend(r.samples.map(|v| ("face rings' samples", u64::from(v))));
            f.push(("brick level's samples", u64::from(r.brick_samples)));
            if r.cell != BRICK_CELL {
                f.push(("brick level's cell", u64::from(r.cell)));
            }
        }
        f
    }
}

/// Why a store file did not load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreFileError {
    /// The file could not be read or written.
    Io(String),
    /// Not a store file, or cut short.
    Malformed,
    /// The named field differs from what this run would bake.
    Mismatch(&'static str),
    /// The blocks do not fit the pool.
    PoolFull { needed: u64, capacity: u64 },
}

impl std::fmt::Display for StoreFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "store file: {e}"),
            Self::Malformed => write!(f, "store file: not a store file, or cut short"),
            Self::Mismatch(field) => write!(f, "store file: the {field} differs"),
            Self::PoolFull { needed, capacity } => {
                write!(
                    f,
                    "store file: the blocks need {needed} B and the pool holds {capacity} B"
                )
            }
        }
    }
}

impl std::error::Error for StoreFileError {}

impl Store {
    /// The store as a file's bytes: the magic, `key`'s fields, then each block's chunk, words and
    /// words, in chunk order.
    pub fn to_file_bytes(&self, key: &StoreKey) -> Vec<u8> {
        let mut out = key.magic().to_vec();
        for (_, v) in key.fields() {
            out.extend(v.to_le_bytes());
        }
        let mut put = |m: &BTreeMap<[i32; 3], Vec<u32>>| {
            out.extend((m.len() as u64).to_le_bytes());
            for (c, block) in m {
                for v in c {
                    out.extend(v.to_le_bytes());
                }
                out.extend((block.len() as u64).to_le_bytes());
                out.extend(bytemuck::cast_slice::<u32, u8>(block));
            }
        };
        put(&self.words);
        if key.tent {
            put(&self.raw);
        }
        out
    }

    /// `bytes`' blocks, where its fields match `key`; else the first field that differs.
    pub(crate) fn parse_file(key: &StoreKey, bytes: &[u8]) -> Result<StoreImage, StoreFileError> {
        let mut at = 0usize;
        let mut take = |n: usize| -> Result<&[u8], StoreFileError> {
            let s = bytes.get(at..at + n).ok_or(StoreFileError::Malformed)?;
            at += n;
            Ok(s)
        };
        let magic = take(8)?;
        if magic != key.magic() {
            return Err(
                if magic == STORE_MAGIC || magic == CUT_MAGIC || magic == RING_MAGIC {
                    StoreFileError::Mismatch("seed form, filter or rings")
                } else {
                    StoreFileError::Malformed
                },
            );
        }
        let u64_of = |b: &[u8]| u64::from_le_bytes(b.try_into().expect("8 bytes"));
        for (name, want) in key.fields() {
            if u64_of(take(8)?) != want {
                return Err(StoreFileError::Mismatch(name));
            }
        }
        let mut list = || -> Result<Vec<(ChunkCoord, Vec<u32>)>, StoreFileError> {
            let count = u64_of(take(8)?);
            let mut blocks = Vec::new();
            for _ in 0..count {
                let mut c = [0i32; 3];
                for v in &mut c {
                    *v = i32::from_le_bytes(take(4)?.try_into().expect("4 bytes"));
                }
                let words = u64_of(take(8)?) as usize;
                let raw = take(words * 4)?;
                blocks.push((ChunkCoord(c), bytemuck::pod_collect_to_vec(raw)));
            }
            Ok(blocks)
        };
        let blocks = list()?;
        let raw = if key.tent { list()? } else { Vec::new() };
        Ok(StoreImage {
            piece: Some(ChunkExtent::new(key.piece[0], key.piece[1])),
            blocks,
            raw,
            tent: key.tent,
            bricks: key.rings.map(|r| r.cell),
        })
    }

    /// Empties the store for `window` and loads `blocks` ([`Store::parse_file`]), laid in chunk
    /// order, as `key`'s piece.
    pub(crate) fn load_blocks(
        &mut self,
        queue: &wgpu::Queue,
        window: ChunkExtent,
        image: StoreImage,
    ) -> Result<(), StoreFileError> {
        let StoreImage {
            piece,
            blocks,
            raw,
            tent,
            bricks,
        } = image;
        let size = window.size();
        let head = STORE_TABLE + (size[0] as u64) * (size[1] as u64) * (size[2] as u64);
        let needed = (head + blocks.iter().map(|(_, b)| b.len() as u64).sum::<u64>()) * 4;
        if needed > self.pool.size() {
            return Err(StoreFileError::PoolFull {
                needed,
                capacity: self.pool.size(),
            });
        }
        self.reset(queue, window);
        let mut first = self.head_words;
        for (c, block) in blocks {
            let words = block.len() as u64;
            self.put(queue, c, first, block);
            first += words;
        }
        self.raw = raw.into_iter().map(|(c, b)| (c.0, b)).collect();
        self.tent = tent;
        self.set_bricks(queue, bricks);
        self.piece = piece;
        Ok(())
    }
}

/// The compute pipeline and the buffers of a bake, built on the first bake.
pub struct BakePipeline {
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    /// By [`SeedForm`], each built on its first bake ([`BakePipeline::ensure`]).
    pipelines: [Option<wgpu::ComputePipeline>; 4],
    job: wgpu::Buffer,
    faces: wgpu::Buffer,
    /// [`OPEN_JOBS`] slots of [`TABLE_SLOT_BYTES`]: an open job's candidate table and chunk.
    tables: wgpu::Buffer,
}

/// The light pass's table read, which the bake's module reads from the face's slot instead.
const TABLE_READ: &str = "return bitcast<vec4<u32>>(light_samples[params.light_table_base + i]);";
/// The LOD walk's camera chunk, which the bake's module takes from the face's job instead.
const LOD_ORIGIN: &str = "let d = chunk - floor_div(vec3<i32>(floor(params.views[0].origin)), 64);";

/// bake.wgsl's stream, which a shared-seed form's module keys by chunk and side instead.
const STREAM: &str = "let stream = hash_u32(job.seed ^ hash_u32(key.x ^ hash_u32(key.y)));";
/// The hashed `rand`'s value, which the lattice form's module rotates by `bake_shift`.
const RAND_HASHED_VALUE: &str =
    "return f32(hash_u32(px.x ^ hash_u32(px.y ^ (dim * 0x9E3779B9u))) >> 8u) / 16777216.0;";

/// The shared-seed forms' helpers (`bake_side_key`, `bake_lattice`, `bake_shift`).
pub const BAKE_SEEDS_WGSL: &str = include_str!("shaders/bake_seeds.wgsl");

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

/// The bake's module for `form`: [`bake_source`] itself for [`SeedForm::Independent`]; else with
/// bake.wgsl's stream keyed by the chunk and side (chunk-side) or the side (global-side, lattice),
/// and for the lattice the hashed `rand` rotated by the face's lattice point, then
/// `bake_seeds.wgsl`.
pub fn bake_source_for(form: SeedForm) -> String {
    let source = bake_source();
    let stream = match form {
        SeedForm::Independent => return source,
        SeedForm::ChunkSide => {
            "let stream = hash_u32(job.seed ^ bake_side_key(floor_div(v, 64), f.face));"
        }
        SeedForm::GlobalSide => {
            "let stream = hash_u32(job.seed ^ bake_side_key(vec3<i32>(0), f.face));"
        }
        SeedForm::Lattice => {
            "bake_shift = bake_lattice(v, f.face);\n    let stream = hash_u32(job.seed ^ bake_side_key(vec3<i32>(0), f.face));"
        }
    };
    let mut source = replace_once(&source, STREAM, stream);
    if form == SeedForm::Lattice {
        source = replace_once(
            &source,
            RAND_HASHED_VALUE,
            "return fract(f32(hash_u32(px.x ^ hash_u32(px.y ^ (dim * 0x9E3779B9u))) >> 8u) / 16777216.0 + fract(bake_shift + f32(dim) * 0.618034));",
        );
    }
    format!("{source}\n{BAKE_SEEDS_WGSL}")
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
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bake"),
            bind_group_layouts: &[Some(world_layout), None, None, Some(&layout)],
            immediate_size: 0,
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
        let mut out = Self {
            layout,
            pipeline_layout,
            pipelines: [None, None, None, None],
            job,
            faces,
            tables,
        };
        out.ensure(device, SeedForm::Independent);
        out
    }

    /// Builds `form`'s pipeline where it is not built yet.
    pub(crate) fn ensure(&mut self, device: &wgpu::Device, form: SeedForm) {
        if self.pipelines[form.index()].is_some() {
            return;
        }
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bake"),
            source: wgpu::ShaderSource::Wgsl(bake_source_for(form).into()),
        });
        self.pipelines[form.index()] = Some(device.create_compute_pipeline(
            &wgpu::ComputePipelineDescriptor {
                label: Some("bake"),
                layout: Some(&self.pipeline_layout),
                module: &module,
                entry_point: Some("bake_round"),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &[("RAND_HASHED", 1.0), ("EMISSION_UNIT", 1.0)],
                    ..Default::default()
                },
                cache: None,
            },
        ));
    }

    fn pipeline(&self, form: SeedForm) -> &wgpu::ComputePipeline {
        self.pipelines[form.index()]
            .as_ref()
            .expect("the form's pipeline is built before its bake")
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
    /// It holds a cell (else it feeds its brick record only).
    cell: bool,
    /// Its own stop's bounds: the settings' or its tier's.
    n_min: u32,
    n_max: u32,
    n: u32,
    sum: [f64; 3],
    clamped: [f64; 3],
    lum2: f64,
    best: f32,
    material: u16,
    /// A translucent voxel's face: its tail carries [`CELL_TRANSLUCENT`].
    glass: bool,
    snapshots: Vec<(u32, [f32; 3], [f32; 3])>,
    done: bool,
    /// The cell kept from the block before, unbaked.
    keep: Option<[u32; 2]>,
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

    /// The rounds left before its `n_max`.
    fn rounds_left(&self, round: u32) -> u32 {
        (self.n_max - self.n).div_ceil(round)
    }

    /// The rounds its stop is expected to need, from its standard error now, which falls as
    /// `1 / sqrt(n)`: 0 before its first round or once it reads under the stop.
    fn expected_rounds(&self, round: u32, clamp: bool, se: f32) -> u32 {
        if self.n == 0 {
            return 0;
        }
        let ratio = self.se(clamp) / f64::from(se);
        let wanted = (f64::from(self.n) * ratio * ratio).min(f64::from(self.n_max));
        let more = (wanted - f64::from(self.n)).max(0.0);
        (more / f64::from(round)).ceil() as u32
    }

    /// The rounds none of which can end the face before the last: those up to its `n_min`, or one.
    fn certain_rounds(&self, round: u32) -> u32 {
        if self.n < self.n_min {
            (self.n_min - self.n).div_ceil(round)
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

/// A brick's record from its faces' states, one [`BRICK_WORDS`] record a cell of `cell` voxels in
/// cell order: each direction the mean of the cell's opaque faces' values (a baked mean, or a kept
/// cell's), the mask of the directions it has, and the material of the sample that read the
/// largest luminance over the cell (a kept cell's where none was baked).
fn brick_record(states: &[FaceState], clamp: bool, cell: u32) -> Vec<u32> {
    let side = (8 / cell) as usize;
    let cells = side * side * side;
    let mut sums = vec![[[0f64; 3]; 6]; cells];
    let mut counts = vec![[0u32; 6]; cells];
    let mut best = vec![(-1f32, 0u16); cells];
    for s in states.iter().filter(|s| !s.glass) {
        let l = s.voxel.map(|x| (x.rem_euclid(8) as u32 / cell) as usize);
        let c = l[0] + side * (l[1] + side * l[2]);
        let f = usize::from(s.face);
        let v = if s.n > 0 {
            if s.best > best[c].0 {
                best[c] = (s.best, s.material);
            }
            s.mean(s.kept(clamp))
        } else if let Some(kept) = s.keep {
            if best[c].0 < 0.0 {
                best[c] = (0.0, (kept[1] >> 16) as u16);
            }
            rgb9e5_unpack(kept[0])
        } else {
            continue;
        };
        for k in 0..3 {
            sums[c][f][k] += f64::from(v[k]);
        }
        counts[c][f] += 1;
    }
    let mut out = vec![0u32; BRICK_WORDS * cells];
    for c in 0..cells {
        let r = &mut out[BRICK_WORDS * c..BRICK_WORDS * (c + 1)];
        let mut mask = 0u32;
        for f in 0..6 {
            if counts[c][f] > 0 {
                let n = f64::from(counts[c][f]);
                r[f] = rgb9e5_pack(sums[c][f].map(|x| (x / n) as f32));
                mask |= 1 << f;
            }
        }
        r[6] = u32::from(best[c].1) | mask << 16;
    }
    out
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

/// A chunk's job: its faces, where its block goes, and the cells it keeps from the block before
/// (by brick slot and key), which it does not bake.
pub(crate) struct Job {
    pub coord: ChunkCoord,
    pub faces: ChunkFaces,
    pub first_word: u64,
    pub kept: Option<KeptCells>,
    /// Under [`Rings`], the chunk's tier: its block carries brick records.
    pub tier: Option<Tier>,
    /// Brick records kept from the block before, by slot: their bricks' faces are not baked.
    pub kept_bricks: KeptBricks,
}

/// Brick records a re-baked ring job keeps, by brick slot.
pub(crate) type KeptBricks = std::collections::HashMap<u16, Vec<u32>>;

/// The cells a re-baked job keeps: by brick slot and key, the two words of the cell before, for
/// every face outside the ring that the block before holds.
pub(crate) type KeptCells = std::collections::HashMap<(u16, u16), [u32; 2]>;

/// The 3 x 3 tent's weight by the offset from the centre: 1 2 1.
fn tent3(d: i32) -> f64 {
    if d == 0 { 2.0 } else { 1.0 }
}

/// Face `face` of voxel `v` through the bake's filter: the kernel of the table's blend
/// (`cache_blend.wgsl`), 3 x 3. Each offset `(du, dw)` along the face's tangents takes one
/// neighbour face of the same axis and sign: the coplanar voxel, then the voxel a step toward
/// the air, then the voxel a step into the rock, the first `get` holds, the two stepped ones at
/// half weight. No face of another orientation is a candidate, so no light crosses a crease, and
/// there is no material test; [`tent_block`] hands it one class of face at a time. The weights are the binomial tent (4, 2, 1) over the faces found;
/// a cut's faces all hold the same count, so the blend's count weights cancel and this is a mean
/// of the faces' values. `None` where `get` holds no centre.
pub fn tent(
    get: &dyn Fn([i32; 3], u8) -> Option<[f32; 3]>,
    v: [i32; 3],
    face: u8,
) -> Option<[f32; 3]> {
    let centre = get(v, face)?;
    let axis = usize::from(face / 2);
    let air = if face % 2 == 1 { 1 } else { -1 };
    let (u, w) = ((axis + 1) % 3, (axis + 2) % 3);
    let mut sum = [0f64; 3];
    let mut weight = 0f64;
    for dw in -1..=1i32 {
        for du in -1..=1i32 {
            let t = tent3(du) * tent3(dw);
            let found = if du == 0 && dw == 0 {
                Some((centre, 1.0))
            } else {
                let mut c = v;
                c[u] += du;
                c[w] += dw;
                let mut up = c;
                up[axis] += air;
                let mut down = c;
                down[axis] -= air;
                get(c, face)
                    .map(|x| (x, 1.0))
                    .or_else(|| get(up, face).map(|x| (x, 0.5)))
                    .or_else(|| get(down, face).map(|x| (x, 0.5)))
            };
            if let Some((x, h)) = found {
                for k in 0..3 {
                    sum[k] += f64::from(x[k]) * t * h;
                }
                weight += t * h;
            }
        }
    }
    Some(sum.map(|x| (x / weight) as f32))
}

/// The cell word of face `face` of voxel `v` among `blocks`, by its chunk's directory and a
/// binary search of its brick's run.
#[cfg(test)]
fn cell_in(blocks: &BTreeMap<[i32; 3], Vec<u32>>, v: [i32; 3], face: u8) -> Option<u32> {
    cell_and_tail_in(blocks, v, face).map(|(word, _)| word)
}

/// The cell word and the tail word of face `face` of voxel `v` among `blocks`.
fn cell_and_tail_in(
    blocks: &BTreeMap<[i32; 3], Vec<u32>>,
    v: [i32; 3],
    face: u8,
) -> Option<(u32, u32)> {
    let block = blocks.get(&v.map(|x| x.div_euclid(64)))?;
    let l = v.map(|x| x.rem_euclid(64));
    let dir = block[brick_slot(l.map(|x| x / 8))];
    let (run, n) = ((dir >> 11) as usize, (dir & 0x7FF) as usize);
    let i = l.map(|x| x % 8);
    let key = (((i[0] + 8 * (i[1] + 8 * i[2])) << 3) as u32) | u32::from(face);
    let (mut lo, mut hi) = (0usize, n);
    while lo < hi {
        let mid = (lo + hi) / 2;
        let tail = block[run + 2 * mid + 1];
        let k = tail & 0xFFF;
        if k == key {
            return Some((block[run + 2 * mid], tail));
        }
        if k < key {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    None
}

/// Chunk `c`'s block of `raw` with each cell through [`tent`] over `raw`'s cells of its own class,
/// across chunks: an opaque face's neighbours are opaque faces, a translucent face's translucent
/// ones ([`CELL_TRANSLUCENT`]), so glass beside a wall leaves the wall's cells as they were. The
/// tail words as they are. `None` where `raw` holds no block for `c`.
fn tent_block(raw: &BTreeMap<[i32; 3], Vec<u32>>, c: [i32; 3]) -> Option<Vec<u32>> {
    let mut block = raw.get(&c)?.clone();
    let of_class = |glass: bool| {
        move |v: [i32; 3], f: u8| {
            cell_and_tail_in(raw, v, f)
                .filter(|&(_, tail)| (tail & CELL_TRANSLUCENT != 0) == glass)
                .map(|(word, _)| rgb9e5_unpack(word))
        }
    };
    let (get_opaque, get_glass) = (of_class(false), of_class(true));
    for slot in 0..CHUNK_BRICKS {
        let dir = block[slot];
        let (run, n) = ((dir >> 11) as usize, (dir & 0x7FF) as usize);
        let b = brick_of_slot(slot);
        for k in 0..n {
            let key = block[run + 2 * k + 1] & 0xFFF;
            let i = (key >> 3) as i32;
            let l = [i % 8, i / 8 % 8, i / 64];
            let v = [0, 1, 2].map(|a| c[a] * 64 + b[a] * 8 + l[a]);
            let face = (key & 7) as u8;
            let get: &dyn Fn([i32; 3], u8) -> Option<[f32; 3]> =
                if block[run + 2 * k + 1] & CELL_TRANSLUCENT != 0 {
                    &get_glass
                } else {
                    &get_opaque
                };
            if let Some(x) = tent(get, v, face) {
                block[run + 2 * k] = rgb9e5_pack(x);
            }
        }
    }
    Some(block)
}

/// `c` and the 26 chunks around it.
fn chunks_around(c: [i32; 3]) -> impl Iterator<Item = [i32; 3]> {
    (-1..=1).flat_map(move |z| {
        (-1..=1).flat_map(move |y| (-1..=1).map(move |x| [c[0] + x, c[1] + y, c[2] + z]))
    })
}

/// The cells of `block` by brick slot and key.
fn cells_of(block: &[u32]) -> KeptCells {
    let mut out = KeptCells::new();
    for slot in 0..CHUNK_BRICKS {
        let dir = block[slot];
        let (run, n) = ((dir >> 11) as usize, (dir & 0x7FF) as usize);
        for i in 0..n {
            let cell = [block[run + 2 * i], block[run + 2 * i + 1]];
            out.insert((slot as u16, (cell[1] & 0xFFF) as u16), cell);
        }
    }
    out
}

/// The surface chunks of `extent` and their faces, in coordinate order.
fn list_extent(
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
    let tier_of = |c: ChunkCoord| {
        settings
            .rings
            .as_ref()
            .map(|r| r.tier(c.0).unwrap_or(Tier::Brick))
    };
    if let Some(r) = &settings.rings {
        listed.retain(|(c, _)| r.tier(c.0).is_some());
    }
    let record = settings
        .rings
        .as_ref()
        .map_or(BRICK_WORDS, Rings::record_words);
    let window = run.gpu.window();
    let size = window.size();
    let table_words = (size[0] as u64) * (size[1] as u64) * (size[2] as u64);
    let needed_words = STORE_TABLE
        + table_words
        + listed
            .iter()
            .map(|(c, f)| layout(f, tier_of(*c), record).1)
            .sum::<u64>();
    let capacity = store.pool.size();
    if needed_words * 4 > capacity {
        return Err(BakeError::PoolFull {
            needed: needed_words * 4,
            capacity,
        });
    }
    store.reset(run.queue, window);
    store.set_bricks(run.queue, settings.rings.as_ref().map(|r| r.cell));
    // The blocks lie in the jobs' order, whatever order the jobs end in.
    let mut at = store.head_words;
    let jobs: Vec<Job> = listed
        .into_iter()
        .map(|(coord, faces)| {
            let tier = tier_of(coord);
            let words = layout(&faces, tier, record).1;
            let job = Job {
                coord,
                faces,
                first_word: at,
                kept: None,
                tier,
                kept_bricks: KeptBricks::new(),
            };
            at += words;
            job
        })
        .collect();
    let report = run_jobs(run, store, jobs, settings, progress)?;
    if settings.tent {
        store.raw = store.words.clone();
        let all: Vec<[i32; 3]> = store.words.keys().copied().collect();
        store.apply_tent(run.queue, all);
    }
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
pub const DIG_REACH: i32 = 32;
/// Samples a dug face takes at most.
pub const DIG_N_MAX: u32 = 1 << 16;

/// What a dig's re-bake did.
#[derive(Clone, Debug, Default)]
pub struct DigReport {
    /// The ring's bricks inside the piece; 0 is a dig wholly outside it.
    pub bricks: u64,
    pub bake: BakeReport,
}

/// Re-bakes the ring of `boxes` ([`dig_ring`]) in `store`: each chunk the ring reaches is listed
/// again, its faces in the ring's bricks baked, every other face keeping its cell, and its block
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
        let record = store.bricks.map_or(BRICK_WORDS, record_words);
        let tier = store.bricks.is_some().then(|| {
            settings
                .rings
                .as_ref()
                .and_then(|r| r.tier(*c))
                .unwrap_or(Tier::Brick)
        });
        // Under the filter a kept cell is the bake's own value, before the filter.
        let source = if store.tent { &store.raw } else { &store.words };
        let old = source.get(c).map(|b| cells_of(b)).unwrap_or_default();
        let mut kept_bricks = KeptBricks::new();
        if store.bricks.is_some()
            && let Some(b) = source.get(c)
        {
            for (slot, record) in records_of(b, record) {
                if slots.binary_search(&slot).is_err() {
                    kept_bricks.insert(slot, record);
                }
            }
        }
        let mut kept = KeptCells::new();
        for (slot, keys) in &faces.bricks {
            if slots.binary_search(slot).is_ok() {
                continue;
            }
            for &key in keys {
                if let Some(cell) = old.get(&(*slot, key)) {
                    kept.insert((*slot, key), *cell);
                }
            }
        }
        let first_word = store.place(coord, layout(&faces, tier, record).1, &mut end)?;
        jobs.push(Job {
            coord,
            faces,
            first_word,
            kept: Some(kept),
            tier,
            kept_bricks,
        });
    }
    let bake = run_jobs(run, store, jobs, settings, progress)?;
    if store.tent {
        // The ring's chunks hold their new values before the filter (or no block); every chunk
        // around them reads them through its kernel.
        let mut around = std::collections::BTreeSet::new();
        for c in ring.keys() {
            if let Some(words) = store.words.get(c) {
                store.raw.insert(*c, words.clone());
            }
            around.extend(chunks_around(*c).filter(|n| store.raw.contains_key(n)));
        }
        store.apply_tent(run.queue, around);
    }
    Ok(DigReport { bricks, bake })
}

/// Bakes `jobs` into `store`, each block written at its `first_word` as its job ends.
fn run_jobs(
    run: &BakeRun<'_>,
    store: &mut Store,
    jobs: Vec<Job>,
    settings: &BakeSettings,
    progress: &mut dyn FnMut(&BakeProgress),
) -> Result<BakeReport, BakeError> {
    let (device, queue) = (run.device, run.queue);
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
    let total_faces: u64 = jobs.iter().map(|j| j.faces.faces() as u64).sum();
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
    };
    let mut pending: VecDeque<Job> = jobs.into();
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
            let Some(next) = pending.pop_front() else {
                free.push(slot);
                break;
            };
            let job = ctx.open(next, slot);
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
                    let expected = s.expected_rounds(round, settings.clamp, settings.se).max(1);
                    if *r < s.rounds_left(round).min(rounds_cap).min(expected) {
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
                    input.push(s.input(first, round.min(s.n_max - first), job.slot));
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
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("bake"),
                timestamp_writes: settings.timer.as_ref().map(|t| t.writes()),
            });
            pass.set_pipeline(run.pipeline.pipeline(settings.form));
            pass.set_bind_group(0, run.world_bind, &[]);
            pass.set_bind_group(3, &bind, &[]);
            pass.dispatch_workgroups(count.div_ceil(64), 1, 1);
        }
        if let Some(timer) = &settings.timer {
            timer.resolve(&mut enc);
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
        if let Some(timer) = &settings.timer {
            report.dispatch_seconds += timer.read_ms(device) / 1e3;
        }
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
                    let reached = s.n >= s.n_min && s.se(settings.clamp) <= f64::from(settings.se);
                    if reached || s.n >= s.n_max {
                        s.done = true;
                        report.faces_at_min += u64::from(s.n <= s.n_min);
                        report.faces_at_max += u64::from(s.n >= s.n_max);
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
    tier: Option<Tier>,
    kept_bricks: KeptBricks,
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
}

impl JobCtx<'_, '_> {
    /// The job at `coord`: its faces' states, and its parameters and table staged in `slot`.
    fn open(&self, job: Job, slot: usize) -> OpenJob {
        let Job {
            coord,
            faces,
            first_word,
            kept,
            tier,
            kept_bricks,
        } = job;
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
        let brick_samples = self
            .settings
            .rings
            .as_ref()
            .map_or(BRICK_LEVEL_SAMPLES, |r| r.brick_samples);
        let mut states: Vec<FaceState> = Vec::with_capacity(faces.faces());
        for (brick_at, keys) in &faces.bricks {
            let b = brick_of_slot(usize::from(*brick_at));
            let cell = holds_cells(tier, keys.len());
            let record_kept = kept_bricks.contains_key(brick_at);
            // A brick-level direction's samples, spread over its opaque faces.
            let mut per_dir = [0u32; 6];
            for &key in keys {
                if !faces.is_glass(*brick_at, key) {
                    per_dir[usize::from(key & 7)] += 1;
                }
            }
            for &key in keys {
                let i = i32::from(key >> 3);
                let l = [i % 8, i / 8 % 8, i / 64];
                let voxel = [0, 1, 2].map(|k| coord.0[k] * 64 + b[k] * 8 + l[k]);
                let face = (key & 7) as u8;
                let glass = faces.is_glass(*brick_at, key);
                let keep = kept
                    .as_ref()
                    .and_then(|k| k.get(&(*brick_at, key)).copied());
                let (n_min, n_max) = match tier {
                    None => (self.settings.n_min, self.n_max),
                    Some(Tier::Face(n)) if cell => (n, n),
                    Some(_) => {
                        let n = brick_samples.div_ceil(per_dir[usize::from(face)].max(1));
                        (n, n)
                    }
                };
                let wanted = keep.is_none()
                    && !record_kept
                    && (cell || !glass)
                    && self.settings.filter.as_ref().is_none_or(|f| f(voxel, face));
                states.push(FaceState {
                    voxel,
                    face,
                    key,
                    cell,
                    n_min,
                    n_max,
                    n: 0,
                    sum: [0.0; 3],
                    clamped: [0.0; 3],
                    lum2: 0.0,
                    best: 0.0,
                    material: 0,
                    glass,
                    snapshots: Vec::new(),
                    done: !wanted,
                    keep,
                });
            }
        }
        let live = (0..states.len()).filter(|&i| !states[i].done).collect();
        OpenJob {
            coord,
            faces,
            first_word,
            tier,
            kept_bricks,
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
                .certain_rounds(self.round)
                .min(s.rounds_left(self.round))
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
        let clamp = self.settings.clamp;
        let cell_size = self.settings.rings.as_ref().map_or(BRICK_CELL, |r| r.cell);
        let (dir, words) = layout(&job.faces, job.tier, record_words(cell_size));
        let mut block: Vec<u32> = dir;
        block.reserve(words as usize);
        let cell_words = |s: &FaceState| -> [u32; 2] {
            if let Some(cell) = s.keep {
                return cell;
            }
            let flags = if s.glass { CELL_TRANSLUCENT } else { 0 };
            [
                rgb9e5_pack(s.mean(s.kept(clamp))),
                u32::from(s.key) | flags | u32::from(s.material) << 16,
            ]
        };
        if job.tier.is_none() {
            for s in &job.states {
                block.extend(cell_words(s));
            }
        } else {
            let mut at = 0usize;
            for (slot, keys) in &job.faces.bricks {
                let states = &job.states[at..at + keys.len()];
                at += keys.len();
                let record = job
                    .kept_bricks
                    .get(slot)
                    .cloned()
                    .unwrap_or_else(|| brick_record(states, clamp, cell_size));
                block.extend(record);
                if states[0].cell {
                    for s in states {
                        block.extend(cell_words(s));
                    }
                }
            }
        }
        debug_assert_eq!(block.len() as u64, words);
        store.put(queue, job.coord, job.first_word, block);
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
        // The reference's module is the independent form's, its text unchanged by the forms.
        assert_eq!(bake_source_for(SeedForm::Independent), source);
        assert!(!source.contains("bake_side_key"));
        for form in SeedForm::ALL {
            let source = bake_source_for(form);
            assert_eq!(
                source.contains(STREAM),
                form == SeedForm::Independent,
                "{form:?}"
            );
            let module = wgpu::naga::front::wgsl::parse_str(&source)
                .unwrap_or_else(|e| panic!("{form:?}: {}", e.emit_to_string(&source)));
            wgpu::naga::valid::Validator::new(
                wgpu::naga::valid::ValidationFlags::all(),
                wgpu::naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap_or_else(|e| panic!("{form:?}'s bake module validates: {e:?}"));
        }
    }

    /// A block of `faces` whose cell values are `value(voxel, face)`, at chunk `c`.
    fn block_of(c: [i32; 3], faces: &ChunkFaces, value: impl Fn([i32; 3], u8) -> f32) -> Vec<u32> {
        let mut block = faces.directory();
        for (slot, keys) in &faces.bricks {
            let b = brick_of_slot(usize::from(*slot));
            for &key in keys {
                let i = i32::from(key >> 3);
                let l = [i % 8, i / 8 % 8, i / 64];
                let v = [0, 1, 2].map(|k| c[k] * 64 + b[k] * 8 + l[k]);
                let x = value(v, (key & 7) as u8);
                block.push(rgb9e5_pack([x; 3]));
                block.push(u32::from(key));
            }
        }
        block
    }

    /// The filter's block pass reads each face through `tent` over every block's cells: across a
    /// chunk border as within, a crease's other side never, and the tail words unchanged.
    #[test]
    fn the_tent_reads_across_chunks_and_not_across_a_crease() {
        let mut classes = vec![CLASS_AIR; 4];
        classes[1] = CLASS_OPAQUE;
        // A floor of two chunks along x, its top at y = 0, and a wall at x = 70 above it.
        let mut chunks = [Chunk::uniform(AIR), Chunk::uniform(AIR)];
        for (n, chunk) in chunks.iter_mut().enumerate() {
            for slot in 0..CHUNK_BRICKS {
                let b = brick_of_slot(slot);
                chunk.set_brick(
                    slot,
                    Brick::from_fn(|l| {
                        let v = [0, 1, 2].map(|k| b[k] * 8 + l[k]);
                        let x = v[0] + 64 * n as i32;
                        if v[1] == 0 || (x == 70 && v[1] <= 3) {
                            1
                        } else {
                            AIR
                        }
                    }),
                );
            }
        }
        let left = [None, Some(&chunks[1]), None, None, None, None];
        let right = [Some(&chunks[0]), None, None, None, None, None];
        let faces = [
            list_chunk_faces(&chunks[0], &left, &classes),
            list_chunk_faces(&chunks[1], &right, &classes),
        ];
        // The floor's top faces read 1 left of x = 64 and 0 from there; the wall's -x faces 0.
        let value = |v: [i32; 3], f: u8| f32::from(f == 3 && v[0] < 64);
        let mut raw = BTreeMap::new();
        for (n, f) in faces.iter().enumerate() {
            raw.insert([n as i32, 0, 0], block_of([n as i32, 0, 0], f, value));
        }
        let cell = |m: &BTreeMap<[i32; 3], Vec<u32>>, v, f| cell_in(m, v, f).map(rgb9e5_unpack);
        let mut out = BTreeMap::new();
        for c in [[0, 0, 0], [1, 0, 0]] {
            out.insert(c, tent_block(&raw, c).expect("a block"));
        }
        // Across the chunk border, the floor's first face beyond it reads a quarter, the one
        // before it three quarters, and two beyond reads 0.
        let at = |x: i32| cell(&out, [x, 0, 20], 3).expect("a floor face")[0];
        assert_eq!(at(64), 0.25);
        assert_eq!(at(63), 0.75);
        assert_eq!(at(65), 0.0);
        assert_eq!(at(10), 1.0);
        // The wall's -x face beside the floor reads its own 0, whatever the floor holds.
        let wall = cell(&out, [70, 1, 20], 0).expect("the wall's -x face");
        assert_eq!(wall, [0.0; 3]);
        // The tail words are the block's.
        for c in [[0, 0, 0], [1, 0, 0]] {
            let (a, b) = (&raw[&c], &out[&c]);
            assert_eq!(a.len(), b.len());
            assert_eq!(a[..CHUNK_BRICKS], b[..CHUNK_BRICKS]);
        }
    }

    #[test]
    fn every_exposed_face_has_one_key() {
        let mut classes = vec![CLASS_AIR; 1 << 16];
        classes[1] = CLASS_OPAQUE;
        classes[2] = CLASS_OPAQUE;
        classes[3] = CLASS_TRANSLUCENT;
        let chunk = mixed_chunk(0x9E37_79B9_7F4A_7C15);
        let beside: [Chunk; 6] = std::array::from_fn(|f| mixed_chunk(0x1234_5678 + 97 * f as u64));
        let refs: [Option<&Chunk>; 6] = std::array::from_fn(|f| Some(&beside[f]));
        let faces = list_chunk_faces(&chunk, &refs, &classes);
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
        let mut glass: Vec<(u16, u16)> = Vec::new();
        for z in 0..64 {
            for y in 0..64 {
                for x in 0..64 {
                    let l = [x, y, z];
                    // An opaque voxel's face toward anything not opaque; a translucent voxel's
                    // toward air.
                    let class = classes[usize::from(material(&chunk, l))];
                    if class == CLASS_AIR {
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
                        let open = match class {
                            CLASS_OPAQUE => classes[usize::from(m)] != CLASS_OPAQUE,
                            _ => classes[usize::from(m)] == CLASS_AIR,
                        };
                        if open {
                            let at = (brick_slot(l.map(|v| v / 8)), key | face as u16);
                            scan.push(at);
                            if class == CLASS_TRANSLUCENT {
                                glass.push((at.0 as u16, at.1));
                            }
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
        glass.sort_unstable();
        assert!(!glass.is_empty());
        assert_eq!(faces.glass, glass);
    }

    #[test]
    fn the_directory_points_at_each_bricks_run() {
        let faces = ChunkFaces {
            bricks: vec![(3, vec![1, 2, 3]), (9, vec![4])],
            glass: Vec::new(),
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
