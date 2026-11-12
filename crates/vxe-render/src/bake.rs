//! The loaded light's store and its bake (ADR-0046): every surface brick of a chunk holds its
//! emissive irradiance at unit emission, a value a direction for each cell of [`BRICK_CELL`] (or
//! 4) voxels, in one pool, computed ahead of the frames that read it.
//!
//! **The list.** [`list_chunk_faces`] is a pure function of a chunk's voxels, its six neighbours'
//! and each material's class ([`face_classes`]): each surface brick's sorted face keys, an opaque
//! voxel's faces toward a voxel that is not opaque and a translucent voxel's faces toward air
//! (ADR-0047: a pane's two sides), the latter listed in [`ChunkFaces::glass`] and baked as the
//! opaque ones are: a pane's entry face reads its brick's record (`march.wgsl`'s
//! `glass_reflected`).
//!
//! **The pool** ([`Store`]), in `u32` words: words 0 to 2 unused (the frame's read counters,
//! no value, brick reads and [`STORE_COARSE_READS`], are a buffer of their own that only the
//! counting temporal pass writes), the mirror window's origin (word 4) and size (word 8) in
//! chunks, the flags ([`STORE_FLAGS`]), then from word [`STORE_TABLE`] one word per window chunk
//! naming its block's first word (0 none), bit 31 ([`TABLE_COARSE`]) set on the live queue's
//! coarse blocks, then the two levels of the [`ParentTable`] (ADR-0053). A block is 512 directory
//! words, a brick's `offset from the block's start << 11` just past its record (0 no surface),
//! then at [`SUMMARY_AT`], just past the directory, the block's light summary (one record in the
//! cell's format: each direction the mean over the cells that have it), then each surface
//! brick's record ([`BRICK_WORDS`] a cell, [`record_words`]). A record's direction is the mean of the cell's faces of that direction,
//! opaque and translucent, each face drawing its share of [`BakeSettings::brick_samples`].
//!
//! **The job** ([`Frame::bake_until`](crate::Frame::bake_until)): a chunk's faces, its own
//! candidate table ([`job_table`]) and its frame parameters, staged once in a slot when the job
//! opens. A submit holds the live faces of up to [`OPEN_JOBS`] open jobs, at most
//! [`SUBMIT_SAMPLES`] samples, in one dispatch whose entries name their jobs' slots, so every face
//! samples its own job's table. A one-off bake waits on each submit; the live queue reads a submit
//! back frames later, once mapped, with at most [`IN_FLIGHT`] unread, and a face's next round
//! starts after the samples it has in flight. An entry of the working buffer is one round of one
//! face, its samples spread over a workgroup's [`BAKE_LANES`] lanes: a
//! face takes the rounds that cannot end it (those up to `n_min`) at once, one round past that,
//! and, when the submit still has room and no job is left to open, further rounds speculatively.
//! `bake.wgsl` returns each entry's sums; the running sums are kept here in `f64` and added round
//! by round in order, the stop is read after each round, and a round past a face's stop is thrown
//! away ([`BakeReport::wasted`]). A face's samples are a function of its key and the sample's
//! index alone, so its rounds, its stop and its record depend neither on the batching, nor on the
//! rounds a submit, nor on which jobs share a submit, nor on the jobs' order.
//!
//! The stop ([`se_levels`]): a face ends when the standard error of its mean luminance, through the
//! display transform at [`BAKE_EXPOSURE`], is at most [`BAKE_SE`] levels, not before `n_min` samples
//! and not after `n_max`. A face's `n_min` and `n_max` are both its share of its brick direction's
//! samples, so it ends at that share.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use vxe_world::{
    AIR, BRICK_VOXELS, Brick, CHUNK_BRICKS, Chunk, ChunkCoord, ChunkExtent, MaterialTable, World,
    brick_of_slot, brick_slot,
};

use crate::gpu_world::{CHANGES_MAX, GpuWorld};
use crate::marcher::{LIGHT_TABLE_ENTRIES, LightTableUniform, Params};

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
/// Samples one submit holds at most, so the driver sees short dispatches with a wait between.
pub const SUBMIT_SAMPLES: u64 = 10_000_000;
/// The exposure the stop reads the display at (the owner's windows' fixed exposure).
pub const BAKE_EXPOSURE: f32 = 8.0;
/// [`Lighting::bake_pool_bytes`](crate::Lighting) by default; the device's storage binding clamps it.
pub const DEFAULT_BAKE_POOL_BYTES: u64 = 256 << 20;
/// The pool before the first bake or live queue: the header's words only, which hold no block.
pub const MINIMAL_POOL_BYTES: u64 = 64;
/// The word of the pool's chunk table.
pub const STORE_TABLE: u64 = 16;
/// A chunk table word's flag: the chunk's block is the live queue's coarse pass, refined later.
/// The block's first word is the rest (a pool is under 2^31 words).
pub const TABLE_COARSE: u32 = 1 << 31;
/// The frame's read counters' word that counts the lighting pixels that read a coarse block.
pub const STORE_COARSE_READS: u64 = 2;
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
}

const NO_VOXEL: [u64; 8] = [0; 8];
const ALL_VOXELS: [u64; 8] = [u64::MAX; 8];

/// A brick's opaque voxels and its solid ones (opaque or translucent): bit `x + 8 y` of word `z`.
fn class_masks(brick: &Brick, classes: &[u8]) -> Masks {
    match brick {
        Brick::Uniform(m) => uniform_masks(classes[usize::from(*m)]),
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

/// A brick's opaque and solid masks ([`class_masks`]).
pub type Masks = ([u64; 8], [u64; 8]);

/// The masks of a uniform brick of class `c`.
fn uniform_masks(c: u8) -> Masks {
    match c {
        CLASS_OPAQUE => (ALL_VOXELS, ALL_VOXELS),
        CLASS_TRANSLUCENT => (NO_VOXEL, ALL_VOXELS),
        _ => (NO_VOXEL, NO_VOXEL),
    }
}

/// A uniform brick's class in [`ChunkMasks`]: this marks a brick held by its masks.
const MASKED: u8 = u8::MAX;

/// A chunk's brick masks, the listing's decode done ahead of it (on a stream worker, through
/// [`FacePrep`]): a uniform brick by its class, any other by its masks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkMasks {
    classes: Box<[u8; CHUNK_BRICKS]>,
    masked: Vec<(u16, Masks)>,
}

impl ChunkMasks {
    /// `chunk`'s masks under `classes` ([`face_classes`]).
    pub fn of(chunk: &Chunk, classes: &[u8]) -> Self {
        let mut out = Self {
            classes: Box::new([CLASS_AIR; CHUNK_BRICKS]),
            masked: Vec::new(),
        };
        for (slot, b) in chunk.bricks().iter().enumerate() {
            match b {
                Brick::Uniform(m) => out.classes[slot] = classes[usize::from(*m)],
                Brick::Full(_) => {
                    out.classes[slot] = MASKED;
                    out.masked.push((slot as u16, class_masks(b, classes)));
                }
            }
        }
        out
    }

    /// Brick `slot`'s masks.
    pub fn get(&self, slot: usize) -> Masks {
        match self.classes[slot] {
            MASKED => {
                let i = self
                    .masked
                    .binary_search_by_key(&(slot as u16), |(s, _)| *s)
                    .expect("a masked brick holds its masks");
                self.masked[i].1
            }
            c => uniform_masks(c),
        }
    }

    /// Bytes held, for the prepared results' cap.
    pub fn bytes(&self) -> usize {
        CHUNK_BRICKS + self.masked.len() * std::mem::size_of::<(u16, Masks)>()
    }

    /// An upper bound on the brick directions a listing of the chunk finds: six for a masked
    /// brick; for a uniform opaque or translucent brick, its sides toward the chunk's border or
    /// toward a brick that is not uniform opaque (or, for a translucent one, toward one that
    /// is masked or air); none for air.
    pub fn dirs_bound(&self) -> u64 {
        let mut dirs = 0u64;
        for slot in 0..CHUNK_BRICKS {
            let own = self.classes[slot];
            if own == MASKED {
                dirs += 6;
                continue;
            }
            if own == CLASS_AIR {
                continue;
            }
            let b = vxe_world::brick_of_slot(slot);
            for face in 0..6 {
                let mut n = b;
                n[face / 2] += if face % 2 == 1 { 1 } else { -1 };
                if !(0..8).contains(&n[face / 2]) {
                    dirs += 1;
                    continue;
                }
                let other = self.classes[vxe_world::brick_slot(n)];
                let exposed = match own {
                    CLASS_OPAQUE => other != CLASS_OPAQUE,
                    _ => other == MASKED || other == CLASS_AIR,
                };
                dirs += u64::from(exposed);
            }
        }
        dirs
    }
}

/// The stream workers' hook (ADR-0017, `vxe_world::ChunkPrep`): each full chunk's
/// [`ChunkMasks`] under the material classes it was made with, for [`list_faces_by`].
pub struct FacePrep {
    pub classes: Arc<Vec<u8>>,
}

impl vxe_world::ChunkPrep for FacePrep {
    fn prepare(&self, _coord: ChunkCoord, chunk: &Chunk) -> vxe_world::Prepared {
        let t = stopwatch();
        let masks = ChunkMasks::of(chunk, &self.classes);
        vxe_world::Prepared(Arc::new(PreparedMasks {
            classes: self.classes.clone(),
            masks,
            ns: (seconds_since(t) * 1e9) as u64,
        }))
    }
}

/// What [`FacePrep`] hands back: the masks and the classes they were taken under, and the
/// nanoseconds the worker took over them.
pub struct PreparedMasks {
    pub classes: Arc<Vec<u8>>,
    pub masks: ChunkMasks,
    pub ns: u64,
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
    let masks: Vec<Masks> = chunk
        .bricks()
        .iter()
        .map(|b| class_masks(b, classes))
        .collect();
    list_faces_by(&|slot| masks[slot], &|face, slot| {
        neighbours[face].map(|c| class_masks(c.brick(slot), classes))
    })
}

/// A chunk's faces as [`list_chunk_faces`] lists them, from its bricks' masks (`own`, by slot)
/// and its neighbours' (`beside`, by face and slot; `None` where that neighbour is absent).
pub fn list_faces_by(
    own: &dyn Fn(usize) -> Masks,
    beside: &dyn Fn(usize, usize) -> Option<Masks>,
) -> ChunkFaces {
    let masks: Vec<Masks> = (0..CHUNK_BRICKS).map(own).collect();
    let mut out = ChunkFaces::default();
    for slot in 0..CHUNK_BRICKS {
        let (opaque, solid) = masks[slot];
        if solid == NO_VOXEL {
            continue;
        }
        let b = brick_of_slot(slot);
        let beside: [Masks; 6] = std::array::from_fn(|face| {
            let (k, d) = (face / 2, if face % 2 == 1 { 1 } else { -1 });
            let mut n = b;
            n[k] += d;
            if (0..8).contains(&n[k]) {
                return masks[brick_slot(n)];
            }
            n[k] = n[k].rem_euclid(8);
            beside(face, brick_slot(n)).unwrap_or((ALL_VOXELS, ALL_VOXELS))
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

/// Words of a brick-level cell's record: six direction values in [`rgb9e5_pack`], face order, then
/// `material | direction mask << 16`, the mask's bit `face` set where the cell has a face
/// of that direction. A brick's record is its cells' records in cell order ([`record_words`]):
/// one cell of 8 voxels, or eight of 4 (`x + 2 y + 4 z`).
pub const BRICK_WORDS: usize = 7;
/// The store header's flags word: bit 1 ([`FLAG_CELL4`]) where the records' cells are 4 voxels.
pub const STORE_FLAGS: u64 = 12;
/// The brick records hold eight cells of 4 voxels.
pub const FLAG_CELL4: u32 = 2;
/// The brick level's cell by default, in voxels: the brick.
pub const BRICK_CELL: u32 = 8;

/// Words of a brick's record of cells of `cell` voxels: [`BRICK_WORDS`] a cell.
pub fn record_words(cell: u32) -> usize {
    BRICK_WORDS * (8 / cell).pow(3) as usize
}

/// A block's word holding its light summary (ADR-0053): one record in the cell record's format,
/// just past the directory, so the chunk table's word reaches it with no second read.
pub const SUMMARY_AT: u64 = DIRECTORY_WORDS;
/// A block's words before its first brick record: the directory, then the summary.
pub const BLOCK_HEAD_WORDS: u64 = DIRECTORY_WORDS + BRICK_WORDS as u64;
/// The flags word's bits 2 and 3: the missing-light mode ([`MissingLight::bits`]).
pub const FLAG_MISSING_SHIFT: u32 = 2;

/// What a cube face with no brick value reads (ADR-0053): black as before, or the first value
/// for its direction along a chain of summaries, nearest first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MissingLight {
    /// 0, as before the summaries.
    #[default]
    Black,
    /// The chunk's own summary, then the mean of its six face neighbours' summaries.
    Neighbour,
    /// The chunk's own summary, then the level-1 and the level-2 parent records.
    Parent,
    /// Own, neighbours, level 1, level 2.
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

    fn neighbours(self) -> bool {
        matches!(self, Self::Neighbour | Self::Chain)
    }

    fn parents(self) -> bool {
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
enum Reach {
    Ready,
    Partial,
    Waiting,
}

/// Whether offset `o` (chunks) lies within a chunk's light reach of `lr` chunks: the gap
/// between the two chunks' boxes under `lr` chunks.
fn in_reach(o: [i32; 3], lr: i32) -> bool {
    let gap2: i64 = o
        .iter()
        .map(|&d| i64::from((d.abs() - 1).max(0)).pow(2))
        .sum();
    gap2 < i64::from(lr).pow(2)
}

/// The offsets (chunks) within a light reach of `lr` chunks ([`in_reach`]).
fn reach_offsets(lr: i32) -> impl Iterator<Item = [i32; 3]> {
    let r = lr + 1;
    (-r..=r)
        .flat_map(move |z| (-r..=r).flat_map(move |y| (-r..=r).map(move |x| [x, y, z])))
        .filter(move |&o| in_reach(o, lr))
}

/// A running mean of records by direction: each direction over the records that hold it, the
/// material the most of them keep (the lowest on a tie), the mask their union.
#[derive(Default)]
struct RecordMean {
    sums: [[f64; 3]; 6],
    counts: [u32; 6],
    /// Material and how many records keep it.
    materials: Vec<(u32, u32)>,
}

impl RecordMean {
    fn add(&mut self, r: &[u32]) {
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
    fn record(&self) -> Option<[u32; BRICK_WORDS]> {
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
pub fn block_summary(block: &[u32], record: usize) -> Option<[u32; BRICK_WORDS]> {
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
pub fn mean_record<'a>(
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
    origin: [[i32; 3]; 2],
    side: [[i32; 3]; 2],
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

    fn cells(&self, level: usize) -> u64 {
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
fn parent_of(c: [i32; 3], level: usize) -> [i32; 3] {
    c.map(|v| v.div_euclid(1 << level))
}

/// The blocks' light summaries and the two parent levels over them, as the CPU keeps them
/// (ADR-0053): each change returns the parent records to write. No GPU here.
#[derive(Clone, Debug, Default)]
pub struct LightSummaries {
    summaries: std::collections::HashMap<[i32; 3], [u32; BRICK_WORDS]>,
    levels: [std::collections::HashMap<[i32; 3], [u32; BRICK_WORDS]>; 2],
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
    fn recompute(&mut self, level: usize, p: [i32; 3]) -> (usize, [i32; 3], [u32; BRICK_WORDS]) {
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
fn layout(faces: &ChunkFaces, record: usize) -> (Vec<u32>, u64) {
    let mut dir = vec![0u32; BLOCK_HEAD_WORDS as usize];
    let mut at = BLOCK_HEAD_WORDS as u32;
    for (slot, _) in &faces.bricks {
        at += record as u32;
        dir[usize::from(*slot)] = at << 11;
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

/// A dug chunk's block before its job ends: the layout of `faces` at the brick level, each
/// brick's record the store's (a brick the edit kept), else the mean, direction by direction, of
/// the records of the bricks around it within one brick (two where one holds none), across
/// chunks, a direction none of them holds their mean over every direction; a brick with none
/// around it holds no light. A kept brick in `touched` holds its record with every direction it
/// held none for filled ([`fill_directions`]). Records of one cell a brick only; at
/// other cells a new brick holds none.
fn provisional_block(
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
fn fallback_block(block: &[u32], chain: &[u32; BRICK_WORDS], record: usize) -> Vec<u32> {
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

/// A brick record's directions it holds no value for take the mean of those it does: a brick a
/// dig touched may have faces now where it had none.
fn fill_directions(r: &mut [u32]) {
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
fn brick_lit(block: &[u32], slot: u16, record: usize) -> bool {
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

/// The bake's settings. The default is the brick level the owner picked (Plan 0059 Phase 7b):
/// cells of [`BRICK_CELL`] voxels, [`QUEUE_BRICK_SAMPLES`] a brick direction,
/// [`KEPT_CANDIDATES`] candidates, the clamp as [`KEPT_CLAMP`].
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
    /// Times each submit's dispatch on the GPU into [`BakeReport::dispatch_seconds`] (a reading's
    /// cost rows).
    pub timer: Option<Arc<crate::marcher::PassTimer>>,
    /// The brick records' cell in voxels: 8 (the brick) or 4.
    pub cell: u32,
    /// Samples a brick direction draws, spread over its faces (at least one a face).
    pub brick_samples: u32,
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
            se: BAKE_SE,
            batch_faces: BAKE_BATCH_FACES,
            submit_rounds: 0,
            jobs_alone: false,
            reversed: false,
            keep_faces: false,
            snapshots: Vec::new(),
            filter: None,
            timer: None,
            cell: BRICK_CELL,
            brick_samples: QUEUE_BRICK_SAMPLES,
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
    /// The pool cannot hold the extent's blocks.
    PoolFull { needed: u64, capacity: u64 },
    /// The light-sample buffer refused a chunk in the bake's reach.
    LightsRefused(u64),
}

impl std::fmt::Display for BakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
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
fn alloc_place(
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
fn free_place(free: &mut Vec<(u64, u64)>, end: &mut u64, mut first: u64, mut words: u64) {
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
    pool: wgpu::Buffer,
    blocks: BTreeMap<[i32; 3], (u64, u64)>,
    words: BTreeMap<[i32; 3], Vec<u32>>,
    /// The header's and the chunk table's words.
    head_words: u64,
    /// The first word no block holds: a block that outgrows its place moves here.
    end_words: u64,
    /// The chunks the last bake covered; a dig re-bakes only inside them.
    piece: Option<ChunkExtent>,
    /// The mirror window the chunk table indexes.
    window: Option<ChunkExtent>,
    /// The brick records' cells, in voxels: 8 or 4.
    cell: u32,
    /// Freed places, first word and words, for [`Store::alloc`]: no two touch, and none ends at
    /// `end_words` ([`Store::free_place`]).
    free: Vec<(u64, u64)>,
    /// Chunks whose block is the live queue's coarse pass: their table word carries
    /// [`TABLE_COARSE`].
    coarse: std::collections::HashSet<[i32; 3]>,
    /// Chunks whose block or coarse mark changed since the live queue last took them
    /// ([`Store::take_touched`]); `None` once more than [`CHANGES_MAX`] went untaken.
    touched: Option<Vec<[i32; 3]>>,
    /// The blocks' summaries as shown and the parent levels over them (ADR-0053).
    light: LightSummaries,
    /// The parent table's shape over the window.
    parents: Option<ParentTable>,
    /// What a cube face with no brick value reads; in the flags word.
    missing: MissingLight,
    /// Milliseconds spent on summaries and parents since [`Store::take_summary_ms`].
    summary_ms: f64,
}

impl Store {
    /// The flags word: the cell bit and the missing-light mode.
    fn flags(&self) -> u32 {
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
    fn set_summary(
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
    fn summarise(&mut self, queue: &wgpu::Queue, c: [i32; 3], block: &mut [u32]) {
        let t = stopwatch();
        let record = record_words(self.cell);
        let s = block_summary(block, record);
        let at = SUMMARY_AT as usize;
        block[at..at + BRICK_WORDS].copy_from_slice(&s.unwrap_or_default());
        self.set_summary(queue, c, s);
        self.summary_ms += seconds_since(t) * 1e3;
    }

    /// Notes that `c`'s block or its coarse mark changed.
    fn touch(&mut self, c: [i32; 3]) {
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
            missing: MissingLight::Black,
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
    fn mark_coarse(&mut self, coord: ChunkCoord, coarse: bool) {
        self.touch(coord.0);
        if coarse {
            self.coarse.insert(coord.0);
        } else {
            self.coarse.remove(&coord.0);
        }
    }

    /// The chunk table's word for `coord`'s block at `first`: the first word, flagged
    /// [`TABLE_COARSE`] where the block is coarse.
    fn table_value(&self, coord: ChunkCoord, first: u64) -> u32 {
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
    fn set_cell(&mut self, queue: &wgpu::Queue, cell: u32) {
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
    fn reset(&mut self, queue: &wgpu::Queue, window: ChunkExtent) {
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
    fn free_place(&mut self, first: u64, words: u64) {
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
    fn table_word(&self, coord: ChunkCoord) -> u64 {
        let window = self.window.expect("a reset store");
        let size = window.size();
        let w = [0, 1, 2].map(|k| i64::from(coord.0[k]) - i64::from(window.min.0[k]));
        let index = w[0] + i64::from(size[0]) * (w[1] + i64::from(size[1]) * w[2]);
        STORE_TABLE + index as u64
    }

    /// Writes `block` as `coord`'s at `first`, and points the chunk table at it.
    fn put(&mut self, queue: &wgpu::Queue, coord: ChunkCoord, first: u64, mut block: Vec<u32>) {
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

/// The compute pipeline and the buffers of a bake, built on the first bake.
pub struct BakePipeline {
    pipeline: wgpu::ComputePipeline,
    job: wgpu::Buffer,
    faces: wgpu::Buffer,
    /// [`OPEN_JOBS`] slots of [`TABLE_SLOT_BYTES`]: an open job's candidate table and chunk.
    tables: wgpu::Buffer,
    /// Group 3 over the three buffers above, which the pipeline never replaces.
    bind: wgpu::BindGroup,
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
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bake"),
            source: wgpu::ShaderSource::Wgsl(bake_source().into()),
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
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bake"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: job.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: faces.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: tables.as_entire_binding(),
                },
            ],
        });
        Self {
            pipeline,
            job,
            faces,
            tables,
            bind,
        }
    }

    /// Group 3 of a bake's dispatch: the job uniform, the working buffer and the table slots.
    pub(crate) fn bind(&self) -> &wgpu::BindGroup {
        &self.bind
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
pub(crate) fn stopwatch() -> Instant {
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
    /// Its own stop's bounds: its brick direction's share of the samples.
    n_min: u32,
    n_max: u32,
    n: u32,
    /// Samples submitted and not read back yet: the next round starts at `n + queued`.
    queued: u32,
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

    /// The first sample no submit has drawn.
    fn next(&self) -> u32 {
        self.n + self.queued
    }

    /// Samples its round `k` from the next draws.
    fn round_samples(&self, k: u32, round: u32) -> u32 {
        round.min(self.n_max.saturating_sub(self.next() + k * round))
    }

    /// The rounds left to submit before its `n_max`.
    fn rounds_left(&self, round: u32) -> u32 {
        self.n_max.saturating_sub(self.next()).div_ceil(round)
    }

    /// The rounds its stop is expected to need, from its standard error now, which falls as
    /// `1 / sqrt(n)`: 0 before its first round or once it reads under the stop.
    fn expected_rounds(&self, round: u32, clamp: bool, se: f32) -> u32 {
        if self.n == 0 {
            return 0;
        }
        let ratio = self.se(clamp) / f64::from(se);
        let wanted = (f64::from(self.n) * ratio * ratio).min(f64::from(self.n_max));
        let more = (wanted - f64::from(self.next())).max(0.0);
        (more / f64::from(round)).ceil() as u32
    }

    /// The rounds none of which can end the face before the last: those up to its `n_min`, or one.
    fn certain_rounds(&self, round: u32) -> u32 {
        if self.next() < self.n_min {
            (self.n_min - self.next()).div_ceil(round)
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
/// cell order: each direction the mean of the cell's baked faces' values, the mask of the
/// directions it has, and the material of the sample that read the largest luminance over the
/// cell.
fn brick_record(states: &[FaceState], clamp: bool, cell: u32) -> Vec<u32> {
    let side = (8 / cell) as usize;
    let cells = side * side * side;
    let mut sums = vec![[[0f64; 3]; 6]; cells];
    let mut counts = vec![[0u32; 6]; cells];
    let mut best = vec![(-1f32, 0u16); cells];
    for s in states {
        let l = s.voxel.map(|x| (x.rem_euclid(8) as u32 / cell) as usize);
        let c = l[0] + side * (l[1] + side * l[2]);
        let f = usize::from(s.face);
        if s.n == 0 {
            continue;
        }
        if s.best > best[c].0 {
            best[c] = (s.best, s.material);
        }
        let v = s.mean(s.kept(clamp));
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
pub const DIG_REACH: i32 = 32;

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
fn run_jobs(
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

/// Slots of the candidate tables a one-off bake's jobs take ([`run_jobs`]); the live queue's
/// take the rest ([`QUEUE_SLOTS`]), so a dig's re-bake between two of its submits leaves its open
/// jobs' tables.
const LOOP_SLOTS: usize = OPEN_JOBS - QUEUE_SLOTS;
/// Slots the live queue's open jobs take, the last of [`OPEN_JOBS`].
pub const QUEUE_SLOTS: usize = 16;

/// What one submit of a [`JobLoop`] did.
#[derive(Clone, Copy, Debug, Default)]
pub struct SubmitDone {
    pub samples: u64,
    /// From the submit to the readback.
    pub seconds: f64,
    /// The dispatch's milliseconds on the GPU's timestamps, where the loop times its submits.
    pub gpu_ms: Option<f64>,
}

/// Lanes of a workgroup that share an entry's samples (`BAKE_LANES` in bake.wgsl).
pub const BAKE_LANES: u32 = 64;
/// Workgroups a dispatch row holds (`BAKE_ROW` in bake.wgsl): the entries span x then y.
const BAKE_ROW: u32 = 32_768;
/// Submits a loop holds unread at most ([`JobLoop::dispatch`]).
pub const IN_FLIGHT: usize = 2;
/// Bytes at the head of a submit's readback: its dispatch's two timestamps.
const STAMP_BYTES: u64 = 16;

/// A submit not read back yet: its entries in dispatch order (job slot, face, rounds) and the
/// buffer its results are copied to, mapped when `ready` reads 1.
struct InFlight {
    entries: Vec<(usize, usize, u32)>,
    count: u64,
    staging: wgpu::Buffer,
    ready: Arc<std::sync::atomic::AtomicU8>,
    timed: bool,
    t: Instant,
}

/// The loop's own timestamps, resolved into each submit's readback.
struct LoopTimer {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    period_ns: f32,
}

/// The job loop, resumable: jobs pending and open, the table slots free, and what it did. Each
/// [`JobLoop::dispatch`] is one dispatch, read back by a later [`JobLoop::collect`] once the GPU
/// is done with it; [`JobLoop::submit`] does both, waited on.
pub(crate) struct JobLoop {
    pending: VecDeque<Job>,
    open: Vec<OpenJob>,
    /// Withdrawn open jobs with rounds still in flight ([`JobLoop::withdraw`]): read back into
    /// nothing, each slot freed once its rounds are.
    withdrawn: Vec<OpenJob>,
    free: Vec<usize>,
    in_flight: VecDeque<InFlight>,
    staging: Vec<wgpu::Buffer>,
    /// Whether the loop times its submits on its own timestamps, and the timer once made
    /// (`Some(None)`: the device has no timestamps).
    timed: bool,
    timer: Option<Option<LoopTimer>>,
    /// The samples the last [`JobLoop::dispatch`] planned: 0 when it dispatched nothing.
    pub last_samples: u64,
    /// The light epoch the open jobs' tables were last written at ([`JobLoop::refresh_tables`]).
    tables_at: Option<u64>,
    /// Each ended job's chunk and the samples its faces drew, where the loop keeps them
    /// (`Some`); the owner drains it.
    pub ended: Option<Vec<(ChunkCoord, u64)>>,
    pub report: BakeReport,
    pub done: BakeProgress,
}

impl JobLoop {
    /// A loop over `jobs`, its open jobs' tables in `slots`.
    pub fn new(jobs: Vec<Job>, slots: std::ops::Range<usize>) -> Self {
        let faces: u64 = jobs.iter().map(|j| j.faces.faces() as u64).sum();
        Self {
            report: BakeReport {
                chunks: jobs.len() as u64,
                ..BakeReport::default()
            },
            done: BakeProgress {
                chunks: jobs.len() as u64,
                faces,
                ..BakeProgress::default()
            },
            pending: jobs.into(),
            open: Vec::new(),
            withdrawn: Vec::new(),
            free: slots.rev().collect(),
            in_flight: VecDeque::new(),
            staging: Vec::new(),
            timed: false,
            timer: None,
            last_samples: 0,
            tables_at: None,
            ended: None,
        }
    }

    /// The loop, timing each submit on its own GPU timestamps where the device has them
    /// ([`SubmitDone::gpu_ms`]).
    pub fn timed(mut self) -> Self {
        self.timed = true;
        self
    }

    /// Adds a job behind the pending ones.
    pub fn push(&mut self, job: Job) {
        self.report.chunks += 1;
        self.done.chunks += 1;
        self.done.faces += job.faces.faces() as u64;
        self.pending.push_back(job);
    }

    /// Adds a job ahead of the pending ones.
    pub fn push_front(&mut self, job: Job) {
        self.report.chunks += 1;
        self.done.chunks += 1;
        self.done.faces += job.faces.faces() as u64;
        self.pending.push_front(job);
    }

    /// Moves the open jobs `first` holds ahead of the others, each part in its order: a dispatch
    /// plans the open jobs' rounds in order. A submit in flight finds its jobs by slot.
    pub fn promote(&mut self, first: impl Fn(ChunkCoord) -> bool) {
        self.open.sort_by_key(|j| !first(j.coord));
    }

    /// No job pending or open.
    pub fn idle(&self) -> bool {
        self.pending.is_empty() && self.open.is_empty()
    }

    /// Submits not read back yet.
    pub fn in_flight(&self) -> usize {
        self.in_flight.len()
    }

    /// The chunks of the jobs pending or open.
    pub fn chunks(&self) -> impl Iterator<Item = ChunkCoord> + '_ {
        self.pending
            .iter()
            .map(|j| j.coord)
            .chain(self.open.iter().map(|j| j.coord))
    }

    /// Takes out the pending and open jobs whose chunks `pred` holds: a pending job dropped, an
    /// open job closed with no block, its rounds in flight read back into nothing and its table
    /// slot freed once they are. Each job's chunk, place (first word and words, its block's
    /// layout at `record` words a brick record) and the samples its faces drew (read back or
    /// in flight).
    pub fn withdraw(&mut self, pred: impl Fn(ChunkCoord) -> bool, record: usize) -> Vec<Withdrawn> {
        let mut out = Vec::new();
        let pending = std::mem::take(&mut self.pending);
        for job in pending {
            if pred(job.coord) {
                out.push(Withdrawn {
                    coord: job.coord,
                    first_word: job.first_word,
                    words: layout(&job.faces, record).1,
                    drawn: 0,
                });
            } else {
                self.pending.push_back(job);
            }
        }
        let mut k = 0;
        while k < self.open.len() {
            if !pred(self.open[k].coord) {
                k += 1;
                continue;
            }
            let job = self.open.remove(k);
            out.push(Withdrawn {
                coord: job.coord,
                first_word: job.first_word,
                words: layout(&job.faces, record).1,
                drawn: job
                    .states
                    .iter()
                    .map(|s| u64::from(s.n) + u64::from(s.queued))
                    .sum(),
            });
            if job.states.iter().all(|s| s.queued == 0) {
                self.free.push(job.slot);
            } else {
                self.withdrawn.push(job);
            }
        }
        out
    }

    /// The pool words the pending and open jobs' places hold, at `record` words a brick record.
    pub fn place_words(&self, record: usize) -> u64 {
        let pending: u64 = self
            .pending
            .iter()
            .map(|j| layout(&j.faces, record).1)
            .sum();
        let open: u64 = self.open.iter().map(|j| layout(&j.faces, record).1).sum();
        pending + open
    }

    /// Samples the pending and open jobs still have to submit, at most.
    pub fn samples_left(&self) -> u64 {
        let live = |s: &FaceState| {
            if s.done {
                0
            } else {
                u64::from(s.n_max.saturating_sub(s.n + s.queued))
            }
        };
        let open: u64 = self
            .open
            .iter()
            .map(|j| j.states.iter().map(live).sum::<u64>())
            .sum();
        open + self.pending.iter().map(|j| j.samples).sum::<u64>()
    }

    /// [`JobLoop::samples_left`] of the jobs whose chunks `of` holds.
    fn dig_samples_left<V>(&self, of: &std::collections::HashMap<[i32; 3], V>) -> u64 {
        let live = |s: &FaceState| {
            if s.done {
                0
            } else {
                u64::from(s.n_max.saturating_sub(s.n + s.queued))
            }
        };
        let open: u64 = self
            .open
            .iter()
            .filter(|j| of.contains_key(&j.coord.0))
            .map(|j| j.states.iter().map(live).sum::<u64>())
            .sum();
        let pending: u64 = self
            .pending
            .iter()
            .filter(|j| of.contains_key(&j.coord.0))
            .map(|j| j.samples)
            .sum();
        open + pending
    }

    /// Writes each open job's candidate table again from the ranges `run.gpu` holds now, when a
    /// sync since the last write moved, wrote or dropped a range ([`GpuWorld::light_epoch`]). A
    /// job opened since then wrote its own table from the same ranges.
    pub fn refresh_tables(&mut self, run: &BakeRun<'_>) {
        let epoch = run.gpu.light_epoch();
        if self.tables_at == Some(epoch) {
            return;
        }
        self.tables_at = Some(epoch);
        for job in &self.open {
            write_job_table(run, job.coord, job.slot);
        }
    }

    /// One submit of at most about `sample_cap` samples (and at least one entry), waited on and
    /// read back, with any submit before it: see [`JobLoop::dispatch`] and [`JobLoop::collect`].
    /// `None` when no entry was left to dispatch.
    #[allow(
        clippy::too_many_arguments,
        reason = "the frame's parts, the store, the settings, the cap and the two callbacks are independent"
    )]
    pub fn submit(
        &mut self,
        run: &BakeRun<'_>,
        bind: &wgpu::BindGroup,
        store: &mut Store,
        settings: &BakeSettings,
        sample_cap: u64,
        sink: &mut BlockSink<'_>,
        progress: &mut dyn FnMut(&BakeProgress),
    ) -> Option<SubmitDone> {
        self.collect(run, store, settings, sink, progress, 0);
        if !self.dispatch(run, bind, store, settings, sample_cap, sink, progress) {
            return None;
        }
        Some(self.collect(run, store, settings, sink, progress, 0))
    }

    /// Plans and submits one dispatch of at most about `sample_cap` samples (and at least one
    /// entry): open jobs first, then pending ones opened into free slots, each face's rounds
    /// from the first sample no submit has drawn yet. Its results are copied to a readback
    /// buffer and mapped; nothing waits. A job with nothing to draw ends here, its block to
    /// `sink`. False when no entry was left to dispatch.
    #[allow(
        clippy::too_many_arguments,
        reason = "the frame's parts, the store, the settings, the cap and the two callbacks are independent"
    )]
    pub fn dispatch(
        &mut self,
        run: &BakeRun<'_>,
        bind: &wgpu::BindGroup,
        store: &mut Store,
        settings: &BakeSettings,
        sample_cap: u64,
        sink: &mut BlockSink<'_>,
        progress: &mut dyn FnMut(&BakeProgress),
    ) -> bool {
        let (device, queue) = (run.device, run.queue);
        if self.timed && self.timer.is_none() {
            self.timer = Some(
                device
                    .features()
                    .contains(wgpu::Features::TIMESTAMP_QUERY)
                    .then(|| LoopTimer {
                        queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                            label: Some("bake loop timer"),
                            ty: wgpu::QueryType::Timestamp,
                            count: 2,
                        }),
                        resolve: device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some("bake loop timer resolve"),
                            size: STAMP_BYTES,
                            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                            mapped_at_creation: false,
                        }),
                        period_ns: queue.get_timestamp_period(),
                    }),
            );
        }
        let round = settings.round.max(1);
        let per_submit = (SUBMIT_SAMPLES / u64::from(round)).max(1) as usize;
        let batch = settings
            .batch_faces
            .clamp(1, BAKE_BATCH_FACES)
            .min(per_submit);
        let ctx = JobCtx {
            run,
            settings,
            round,
            rounds_cap: if settings.submit_rounds == 0 {
                u32::MAX
            } else {
                settings.submit_rounds
            },
        };
        let report = &mut self.report;
        let done = &mut self.done;
        let open = &mut self.open;
        // The rounds each live face takes in this submit: first those that cannot end it.
        let mut budget = SubmitRoom {
            entries: 0,
            batch,
            samples: 0,
            cap: sample_cap,
        };
        let mut plan: Vec<Vec<(usize, u32)>> =
            open.iter().map(|job| ctx.plan(job, &mut budget)).collect();
        while budget.room() {
            let Some(slot) = self.free.pop() else { break };
            let Some(next) = self.pending.pop_front() else {
                self.free.push(slot);
                break;
            };
            let job = ctx.open(next, slot);
            if job.live.is_empty() {
                self.free.push(slot);
                let (coord, first, block) = ctx.close(job, report);
                sink(store, coord, first, block);
                done.chunks_done += 1;
                progress(done);
                continue;
            }
            plan.push(ctx.plan(&job, &mut budget));
            open.push(job);
        }
        // Room left and no job to open: further rounds, a layer at a time, in order, each face
        // up to the rounds its standard error now expects it to need.
        while budget.room() && self.pending.is_empty() {
            let before = budget.entries;
            for (job, p) in open.iter().zip(&mut plan) {
                for (i, r) in p.iter_mut() {
                    if !budget.room() {
                        break;
                    }
                    let s = &job.states[*i];
                    let expected = s.expected_rounds(round, settings.clamp, settings.se).max(1);
                    if *r < s.rounds_left(round).min(ctx.rounds_cap).min(expected) {
                        budget.take(s.round_samples(*r, round));
                        *r += 1;
                    }
                }
            }
            if budget.entries == before {
                break;
            }
        }
        if budget.entries == 0 {
            self.last_samples = 0;
            return false;
        }
        self.last_samples = budget.samples;
        // One dispatch: each entry names its job's slot, and its face's samples from the first
        // no submit has drawn.
        let mut input: Vec<[u32; 20]> = Vec::with_capacity(budget.entries);
        let mut entries = Vec::new();
        let mut jobs_in = 0u64;
        for (job, p) in open.iter_mut().zip(&plan) {
            jobs_in += u64::from(p.iter().any(|&(_, r)| r > 0));
            for &(i, r) in p {
                if r == 0 {
                    continue;
                }
                let s = &mut job.states[i];
                report.max_rounds_in_submit = report.max_rounds_in_submit.max(r);
                for _ in 0..r {
                    let first = s.n + s.queued;
                    let samples = round.min(s.n_max - first);
                    input.push(s.input(first, samples, job.slot));
                    s.queued += samples;
                }
                entries.push((job.slot, i, r));
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
        queue.write_buffer(&run.pipeline.faces, 0, bytemuck::cast_slice(&input));
        let own = self.timer.as_ref().and_then(Option::as_ref);
        let size = STAMP_BYTES + u64::from(count) * FACE_BYTES;
        let staging = match self.staging.iter().position(|b| b.size() >= size) {
            Some(k) => self.staging.swap_remove(k),
            None => device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("bake readback"),
                size: STAMP_BYTES + BAKE_BATCH_FACES as u64 * FACE_BYTES,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
        };
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let writes = match own {
                Some(t) => Some(wgpu::ComputePassTimestampWrites {
                    query_set: &t.queries,
                    beginning_of_pass_write_index: Some(0),
                    end_of_pass_write_index: Some(1),
                }),
                None => settings.timer.as_ref().map(|t| t.writes()),
            };
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("bake"),
                timestamp_writes: writes,
            });
            pass.set_pipeline(&run.pipeline.pipeline);
            pass.set_bind_group(0, run.world_bind, &[]);
            pass.set_bind_group(3, bind, &[]);
            pass.dispatch_workgroups(count.min(BAKE_ROW), count.div_ceil(BAKE_ROW), 1);
        }
        match own {
            Some(t) => {
                enc.resolve_query_set(&t.queries, 0..2, &t.resolve, 0);
                enc.copy_buffer_to_buffer(&t.resolve, 0, &staging, 0, STAMP_BYTES);
            }
            None => {
                if let Some(timer) = &settings.timer {
                    timer.resolve(&mut enc);
                }
            }
        }
        enc.copy_buffer_to_buffer(
            &run.pipeline.faces,
            0,
            &staging,
            STAMP_BYTES,
            u64::from(count) * FACE_BYTES,
        );
        report.max_jobs_in_submit = report.max_jobs_in_submit.max(jobs_in);
        let t = stopwatch();
        queue.submit([enc.finish()]);
        let ready = Arc::new(std::sync::atomic::AtomicU8::new(0));
        let flag = ready.clone();
        staging.map_async(wgpu::MapMode::Read, ..size, move |r| {
            flag.store(
                if r.is_ok() { 1 } else { 2 },
                std::sync::atomic::Ordering::Release,
            );
        });
        self.in_flight.push_back(InFlight {
            entries,
            count: u64::from(count),
            staging,
            ready,
            timed: own.is_some(),
            t,
        });
        true
    }

    /// Reads back the submits whose results are mapped, oldest first, stopping at the first
    /// that is not, after polling the device until at most `unread` are left unread: each face's
    /// rounds added in sample order, its stop read after each, and each ended job's block to
    /// `sink`. The sum of what the submits read did; zero samples when none was read.
    pub fn collect(
        &mut self,
        run: &BakeRun<'_>,
        store: &mut Store,
        settings: &BakeSettings,
        sink: &mut BlockSink<'_>,
        progress: &mut dyn FnMut(&BakeProgress),
        unread: usize,
    ) -> SubmitDone {
        use std::sync::atomic::Ordering;
        let device = run.device;
        let mut out = SubmitDone::default();
        while let Some(front) = self.in_flight.front() {
            if front.ready.load(Ordering::Acquire) == 0 {
                if self.in_flight.len() <= unread {
                    break;
                }
                let _ = device.poll(wgpu::PollType::wait_indefinitely());
                continue;
            }
            let f = self.in_flight.pop_front().expect("front read above");
            let mapped = f.ready.load(Ordering::Acquire) == 1;
            let seconds = seconds_since(f.t);
            let size = STAMP_BYTES + f.count * FACE_BYTES;
            let bytes: Vec<u8> = if mapped {
                let b = f
                    .staging
                    .get_mapped_range(..size)
                    .expect("bake readback mapped")
                    .to_vec();
                f.staging.unmap();
                b
            } else {
                vec![0; size as usize]
            };
            self.staging.push(f.staging);
            let gpu_ms = (f.timed && mapped).then(|| {
                let t: &[u64] = bytemuck::cast_slice(&bytes[..STAMP_BYTES as usize]);
                let period = self
                    .timer
                    .as_ref()
                    .and_then(Option::as_ref)
                    .map_or(1.0, |t| t.period_ns);
                t[1].saturating_sub(t[0]) as f64 * f64::from(period) / 1e6
            });
            let report = &mut self.report;
            report.gpu_seconds += seconds;
            if !f.timed
                && let Some(timer) = &settings.timer
            {
                report.dispatch_seconds += timer.read_ms(device) / 1e3;
            }
            report.submits += 1;
            let samples_before = report.samples;
            // Each face's rounds in order: the stop is read after each, and a round past it is
            // waste.
            let out_words: &[u32] = bytemuck::cast_slice(&bytes[STAMP_BYTES as usize..]);
            let mut e = 0usize;
            let mut at = 0usize;
            for &(slot, i, r) in &f.entries {
                if self.open.get(at).is_none_or(|j| j.slot != slot) {
                    match self.open.iter().position(|j| j.slot == slot) {
                        Some(k) => at = k,
                        None => {
                            // A withdrawn job's rounds: read into nothing.
                            let z = self
                                .withdrawn
                                .iter_mut()
                                .find(|j| j.slot == slot)
                                .expect("a job with entries in flight stays open or withdrawn");
                            for _ in 0..r {
                                let samples = out_words[e * 20 + 5];
                                e += 1;
                                z.states[i].queued -= samples;
                                report.wasted += u64::from(samples);
                            }
                            continue;
                        }
                    }
                }
                let s = &mut self.open[at].states[i];
                for _ in 0..r {
                    let words = &out_words[e * 20..(e + 1) * 20];
                    e += 1;
                    let samples = words[5];
                    s.queued -= samples;
                    // A readback that failed to map draws its rounds again in a later submit,
                    // from the face's first sample not drawn.
                    if s.done || !mapped {
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
                        self.done.faces_done += 1;
                    }
                }
            }
            // A withdrawn job whose rounds are all read back frees its slot.
            let free = &mut self.free;
            self.withdrawn.retain(|j| {
                let drained = j.states.iter().all(|s| s.queued == 0);
                if drained {
                    free.push(j.slot);
                }
                !drained
            });
            self.done.samples = report.samples;
            self.done.gpu_seconds = report.gpu_seconds;
            out.samples += report.samples - samples_before;
            out.seconds += seconds;
            if let Some(ms) = gpu_ms {
                *out.gpu_ms.get_or_insert(0.0) += ms;
            }
            self.close_ended(run, store, settings, sink);
            progress(&self.done);
        }
        out
    }

    /// Closes each open job whose faces are all done and none of whose rounds is in flight.
    fn close_ended(
        &mut self,
        run: &BakeRun<'_>,
        store: &mut Store,
        settings: &BakeSettings,
        sink: &mut BlockSink<'_>,
    ) {
        let ctx = JobCtx {
            run,
            settings,
            round: settings.round.max(1),
            rounds_cap: 0,
        };
        for job in self.open.iter_mut() {
            let states = &job.states;
            job.live.retain(|&i| !states[i].done);
        }
        let mut k = 0;
        while k < self.open.len() {
            let j = &self.open[k];
            if j.live.is_empty() && j.states.iter().all(|s| s.queued == 0) {
                let job = self.open.remove(k);
                self.free.push(job.slot);
                if let Some(ended) = self.ended.as_mut() {
                    let drawn = job.states.iter().map(|s| u64::from(s.n)).sum();
                    ended.push((job.coord, drawn));
                }
                let (coord, first, block) = ctx.close(job, &mut self.report);
                sink(store, coord, first, block);
                self.done.chunks_done += 1;
            } else {
                k += 1;
            }
        }
    }
}

/// A job taken out of a [`JobLoop`] ([`JobLoop::withdraw`]): its chunk, its place and the
/// samples its faces drew.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Withdrawn {
    pub coord: ChunkCoord,
    pub first_word: u64,
    pub words: u64,
    pub drawn: u64,
}

/// Where an ended job's block goes: the store, its chunk, its place and its words.
pub(crate) type BlockSink<'a> = dyn FnMut(&mut Store, ChunkCoord, u64, Vec<u32>) + 'a;

/// A submit's room: its entries against the batch, its samples against the cap.
struct SubmitRoom {
    entries: usize,
    batch: usize,
    samples: u64,
    cap: u64,
}

impl SubmitRoom {
    /// Another entry fits: under the batch, and under the cap or with none taken yet.
    fn room(&self) -> bool {
        self.entries < self.batch && (self.samples < self.cap || self.entries == 0)
    }

    fn take(&mut self, samples: u32) {
        self.entries += 1;
        self.samples += u64::from(samples);
    }
}

/// Stages `coord`'s candidate table and its chunk in table slot `slot`.
fn write_job_table(run: &BakeRun<'_>, coord: ChunkCoord, slot: usize) {
    let table = job_table(run.gpu, coord);
    let used = (1 + table.count as usize) * 16;
    let at = slot as u64 * TABLE_SLOT_BYTES;
    let tables = &run.pipeline.tables;
    run.queue
        .write_buffer(tables, at, &bytemuck::bytes_of(&table)[..used]);
    let chunk = [coord.0[0], coord.0[1], coord.0[2], 0];
    run.queue.write_buffer(
        tables,
        at + (TABLE_SLOT_WORDS - 1) * 16,
        bytemuck::cast_slice(&chunk),
    );
}

/// A chunk's job while its faces run.
struct OpenJob {
    coord: ChunkCoord,
    faces: ChunkFaces,
    first_word: u64,
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
    rounds_cap: u32,
}

impl JobCtx<'_, '_> {
    /// The job at `coord`: its faces' states, and its parameters and table staged in `slot`.
    fn open(&self, job: Job, slot: usize) -> OpenJob {
        let Job {
            coord,
            faces,
            first_word,
            kept_bricks,
            samples: _,
            brick_samples,
        } = job;
        write_job_table(self.run, coord, slot);
        // A job's own count (the live queue's coarse pass) on cells of a brick draws each
        // direction's samples on `ceil(count / BAKE_LANES)` of its faces, spread evenly through
        // them in key order, so each entry fills its workgroup; the record is their mean.
        let subset = brick_samples.is_some() && self.settings.cell == 8;
        let brick_samples = brick_samples.unwrap_or(self.settings.brick_samples);
        let mut states: Vec<FaceState> = Vec::with_capacity(faces.faces());
        for (brick_at, keys) in &faces.bricks {
            let b = brick_of_slot(usize::from(*brick_at));
            let record_kept = kept_bricks.contains_key(brick_at);
            // A brick-level direction's samples, spread over its faces, a translucent voxel's
            // among them.
            let mut per_dir = [0u32; 6];
            for &key in keys {
                per_dir[usize::from(key & 7)] += 1;
            }
            // Under `subset`, a direction's `m` faces drawn of its `k`: the i-th face is
            // drawn where `(i m) mod k < m`, which holds for exactly `m` of them.
            let drawn = per_dir.map(|k| {
                if subset {
                    brick_samples.div_ceil(BAKE_LANES).min(k)
                } else {
                    k
                }
            });
            let mut at_dir = [0u32; 6];
            for &key in keys {
                let i = i32::from(key >> 3);
                let l = [i % 8, i / 8 % 8, i / 64];
                let voxel = [0, 1, 2].map(|k| coord.0[k] * 64 + b[k] * 8 + l[k]);
                let face = (key & 7) as u8;
                let d = usize::from(face);
                let n = brick_samples.div_ceil(drawn[d].max(1));
                let in_subset = {
                    let i = at_dir[d];
                    at_dir[d] += 1;
                    u64::from(i) * u64::from(drawn[d]) % u64::from(per_dir[d].max(1))
                        < u64::from(drawn[d])
                };
                let wanted = in_subset
                    && !record_kept
                    && self.settings.filter.as_ref().is_none_or(|f| f(voxel, face));
                states.push(FaceState {
                    voxel,
                    face,
                    n_min: n,
                    n_max: n,
                    n: 0,
                    queued: 0,
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
            kept_bricks,
            states,
            live,
            slot,
        }
    }

    /// The rounds of `job`'s live faces that cannot end them, while `budget` has room.
    fn plan(&self, job: &OpenJob, budget: &mut SubmitRoom) -> Vec<(usize, u32)> {
        let mut p = Vec::new();
        for &i in &job.live {
            if !budget.room() {
                break;
            }
            let s = &job.states[i];
            let want = s
                .certain_rounds(self.round)
                .min(s.rounds_left(self.round))
                .min(self.rounds_cap);
            let mut r = 0;
            while r < want && budget.room() {
                budget.take(s.round_samples(r, self.round));
                r += 1;
            }
            p.push((i, r));
        }
        p
    }

    /// The ended job's chunk, place and block: the directory, then each surface brick's record (a
    /// kept one, or the mean of its baked faces).
    fn close(&self, job: OpenJob, report: &mut BakeReport) -> (ChunkCoord, u64, Vec<u32>) {
        let clamp = self.settings.clamp;
        let cell = self.settings.cell;
        let (dir, words) = layout(&job.faces, record_words(cell));
        let mut block: Vec<u32> = dir;
        block.reserve(words as usize);
        let mut at = 0usize;
        for (slot, keys) in &job.faces.bricks {
            let states = &job.states[at..at + keys.len()];
            at += keys.len();
            let record = job
                .kept_bricks
                .get(slot)
                .cloned()
                .unwrap_or_else(|| brick_record(states, clamp, cell));
            block.extend(record);
        }
        debug_assert_eq!(block.len() as u64, words);
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
        (job.coord, job.first_word, block)
    }
}

/// Frames a re-baked block's values move over, from what it showed to its new values.
pub const FADE: u32 = 16;
/// Frames a dig job's block moves over, from its provisional block: short, so a dug hole's light
/// is seen settling for a moment, not dissolving.
pub const DIG_FADE: u32 = 4;
/// The live queue's slice by default, in milliseconds of a frame.
pub const DEFAULT_BAKE_SLICE_MS: f64 = 1.5;
/// The least slice a per-second budget gives a step, in milliseconds: what a step with no
/// interval yet (the first) or a very short one bakes.
pub const BAKE_SLICE_FLOOR_MS: f64 = 0.25;

/// The live queue's bake budget (ADR-0052): a slice a frame, or milliseconds of GPU time a
/// second of wall time spent in slices sized by the frame's interval.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Budget {
    /// Milliseconds of bake a frame, whatever the frame's length.
    PerFrame { ms: f64 },
    /// Milliseconds of bake a second: a step's slice is `ms_per_s` times the last frame's
    /// interval in seconds, at most `cap` of that interval and at least
    /// [`BAKE_SLICE_FLOOR_MS`].
    PerSecond { ms_per_s: f64, cap: f64 },
}

impl Default for Budget {
    fn default() -> Self {
        Budget::PerFrame {
            ms: DEFAULT_BAKE_SLICE_MS,
        }
    }
}

impl Budget {
    /// The step's slice in milliseconds after a frame of `interval_s` seconds.
    pub fn slice_ms(&self, interval_s: f64) -> f64 {
        match *self {
            Budget::PerFrame { ms } => ms,
            Budget::PerSecond { ms_per_s, cap } => {
                let interval_ms = interval_s.max(0.0) * 1e3;
                (ms_per_s * interval_s.max(0.0))
                    .min(cap * interval_ms)
                    .max(BAKE_SLICE_FLOOR_MS)
            }
        }
    }

    /// Parses `frame:MS` or `second:MS,CAP` (the form [`std::fmt::Display`] writes).
    pub fn parse(s: &str) -> Result<Self, String> {
        let num = |v: &str| {
            v.trim()
                .parse::<f64>()
                .ok()
                .filter(|x| x.is_finite() && *x >= 0.0)
                .ok_or_else(|| format!("bake budget: {v:?} is not a non-negative number"))
        };
        match s.split_once(':') {
            Some(("frame", ms)) => Ok(Budget::PerFrame { ms: num(ms)? }),
            Some(("second", rest)) => {
                let (ms, cap) = rest
                    .split_once(',')
                    .ok_or_else(|| format!("bake budget: {s:?} wants second:MS,CAP"))?;
                let cap = num(cap)?;
                if cap <= 0.0 || cap > 1.0 {
                    return Err(format!("bake budget: the cap {cap} is not in (0, 1]"));
                }
                Ok(Budget::PerSecond {
                    ms_per_s: num(ms)?,
                    cap,
                })
            }
            _ => Err(format!(
                "bake budget: {s:?} is neither frame:MS nor second:MS,CAP"
            )),
        }
    }
}

impl std::fmt::Display for Budget {
    /// `frame:1.5` or `second:150,0.25`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Budget::PerFrame { ms } => write!(f, "frame:{ms}"),
            Budget::PerSecond { ms_per_s, cap } => write!(f, "second:{ms_per_s},{cap}"),
        }
    }
}
/// Samples a brick direction draws in the live queue: the owner's pick (Plan 0059 Phase 7b).
pub const QUEUE_BRICK_SAMPLES: u32 = 4096;
/// Candidates whose reach the queue checks at most in a frame that opens jobs.
const QUEUE_CHECKS: usize = 16;
/// The same in a step without a slice (the pre-roll), where the coarse pass opens many small jobs.
const QUEUE_CHECKS_UNSLICED: usize = 64;
/// A brick direction's samples in the live queue's coarse pass by default (`--coarse`).
pub const DEFAULT_COARSE_SAMPLES: u32 = 256;
/// Milliseconds a frame the live queue's slice grows by while a dig's bricks wait (ADR-0046's
/// edit cap, by time on the slice's own ns a sample).
pub const DEFAULT_BAKE_EDIT_MS: f64 = 3.0;
/// Chunks (Chebyshev) around the camera's whose first pass is the full count, not the coarse one
/// (`--near`).
pub const DEFAULT_NEAR: i32 = 2;
/// Frames before a candidate whose reach or lights were not ready is checked again.
const QUEUE_RECHECK: u64 = 30;
/// Steps an opening that found nothing ready waits before sorting the candidates again.
const QUEUE_QUIET: u64 = 8;

/// A hasher for chunk coordinates in the queue step's per-change sets: one multiply and rotate
/// a word (FxHash's mix), where std's SipHash costs several times as much a key, and a flight
/// hashes about a thousand mirror changes a step. No key comes from outside the process and no
/// set's iteration order is read.
#[derive(Clone, Copy, Debug, Default)]
struct ChunkHasher(u64);

impl ChunkHasher {
    fn add(&mut self, w: u64) {
        self.0 = (self.0.rotate_left(5) ^ w).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}

impl std::hash::Hasher for ChunkHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for w in bytes.chunks(4) {
            let mut b = [0u8; 4];
            b[..w.len()].copy_from_slice(w);
            self.add(u64::from(u32::from_le_bytes(b)));
        }
    }

    fn write_u32(&mut self, i: u32) {
        self.add(u64::from(i));
    }

    fn write_i32(&mut self, i: i32) {
        self.add(u64::from(i as u32));
    }

    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }
}

/// A set of chunk coordinates under [`ChunkHasher`].
type ChunkSet = std::collections::HashSet<[i32; 3], std::hash::BuildHasherDefault<ChunkHasher>>;
/// A map from chunk coordinates under [`ChunkHasher`].
type ChunkMap<V> =
    std::collections::HashMap<[i32; 3], V, std::hash::BuildHasherDefault<ChunkHasher>>;
/// Bytes of prepared masks the queue holds at most; the farthest from the camera go first.
const PREPARED_BYTES_MAX: usize = 64 << 20;

/// The live queue's settings: the default's brick level ([`BRICK_CELL`], [`QUEUE_BRICK_SAMPLES`]
/// a brick direction), a face's round one entry's lanes.
pub fn queue_settings() -> BakeSettings {
    BakeSettings {
        round: QUEUE_ROUND,
        ..BakeSettings::default()
    }
}

/// Samples a face draws in one entry of the live queue's slices: one a lane of the entry's
/// workgroup ([`BAKE_LANES`]).
pub const QUEUE_ROUND: u32 = BAKE_LANES;
/// Samples a face draws in one entry of a step without a slice ([`QueueView::samples`]).
pub const UNSLICED_ROUND: u32 = BAKE_LANES;

/// The live queue's counters, for the HUD and stdout.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct QueueStats {
    /// Full chunks in the mirror with no block, not known faceless and not in a job: the depth.
    pub depth: u64,
    /// Chunks in a job, pending or open.
    pub jobs: u64,
    /// Of the candidates checked at the last opening, those whose reach was not ready, and those
    /// whose emitters' ranges were not.
    pub waiting_reach: u64,
    pub waiting_lights: u64,
    /// Chunks baked, and of them listed from a prepared result.
    pub baked: u64,
    pub prepared: u64,
    /// Blocks fading.
    pub fading: u64,
    /// The samples read back in the last step and their milliseconds on the GPU's timestamps
    /// (submit to readback where there are none).
    pub samples: u64,
    pub ms: f64,
    /// The last sliced step's slice in milliseconds, from the budget ([`Budget::slice_ms`]),
    /// before a dig's growth.
    pub slice_ms: f64,
    /// Samples since the queue started.
    pub total_samples: u64,
    /// Chunks whose block is the coarse pass, waiting for their refinement.
    pub coarse: u64,
    /// Of the depth, the chunks with no block yet, but those whose reach or lights were not ready
    /// at a recent check.
    pub unbaked: u64,
    /// The ns a sample the slice is sized by.
    pub ns_per_sample: f64,
    /// Chunks with dig bricks waiting or in a job, and chunks waiting to re-bake whole (ring 3).
    pub digs: u64,
    pub reach: u64,
    /// Dig jobs ended, and the bricks they re-baked, since the queue started.
    pub digs_baked: u64,
    pub dig_bricks: u64,
    /// Places the pool refused since the queue started: each a job or a provisional block left
    /// waiting for an eviction to free room.
    pub pool_full: u64,
    /// The last step's sample cap, and the samples its one submit planned (0 for none): a submit
    /// passes the cap by less than one entry's round.
    pub cap: u64,
    pub submitted: u64,
    /// The stream workers' results taken since the queue started, and their time on the
    /// workers in total and at most, in nanoseconds ([`FacePrep`]).
    pub hook_chunks: u64,
    pub hook_ns: u64,
    pub hook_max_ns: u64,
    /// The candidates at the last opening by where they lie against the view ([`VIEW_SHELL`],
    /// [`VIEW_IN`], [`VIEW_OUT`]): those with no block, and those with a coarse one.
    pub view_waiting: [[u64; 2]; 3],
    /// Of the candidates in the view cone beyond the shell at the last opening, those outside
    /// the frustum widened by the same margin ([`QueueView::frustum`]).
    pub cone_only: u64,
    /// The median distance in chunks of the candidates in the view cone beyond the shell at the
    /// last opening, with no block and coarse (0 for none).
    pub view_median: [f32; 2],
    /// The samples read back for the jobs ended since the queue started, by where their chunk
    /// lay against the view as the job ended.
    pub view_samples: [u64; 3],
    /// Of them, the samples of jobs whose chunk was out of view and behind the camera (its
    /// centre's view cosine under 0) as the job ended: passed.
    pub passed_samples: u64,
    /// The candidates at the last opening by class (no block, coarse), distance band (chunks 0
    /// to 2, 3 to 5, 6 to 10, over 10) and angle band (`a` 0, under 0.2, 0.2 to 0.4, over 0.4).
    pub weight_census: [[[u32; 4]; 4]; 2],
    /// Of the candidates in the view cone beyond the shell, the ones read at the last reach
    /// census ([`VIEW_REACH_SAMPLE`] at most, spread by distance, every [`VIEW_REACH_EVERY`]
    /// steps) and those whose reach was not ready.
    pub view_reach: [u64; 2],
    /// The CPU milliseconds of the view's counters at the last opening.
    pub census_ms: f64,
    /// Jobs withdrawn since the queue started ([`BakeQueue::withdraw`]), and the samples their
    /// faces had drawn: lost.
    pub withdrawn: u64,
    pub withdrawn_samples: u64,
    /// The last step's CPU time by stage ([`StepStages`]).
    pub stages: StepStages,
    /// Steps that ran an opening since the queue started.
    pub openings: u64,
    /// In the last step: the chunks the mirror filter and a whole reading of the candidates
    /// visited, the reach walks run ([`reach_ready`], not answered from the cache), and
    /// whether the candidates' order was scored again (1) or walked as kept (0).
    pub scanned: u64,
    pub reach_checks: u64,
    pub rebuilt: u64,
    /// Of the candidates in the view cone beyond the shell at the last census, waiting for
    /// their light (blank or coarse), those whose reach crosses the streamer's full radius
    /// ([`QueueView::full_radius`]): their distance in chunks plus the light radius exceeds it,
    /// so a neighbour within their reach is not resident full while the camera stands still.
    pub reach_bound: u64,
    /// Openings since the queue started that pushed a full job on a chunk with no block while a
    /// blank candidate outside [`BakeQueue::full_first`] was ready and left unpushed: the near
    /// shell starving the coarse front.
    pub starved_openings: u64,
    /// Coarse-pass jobs pushed since the queue started.
    pub coarse_opened: u64,
    /// Chunks whose block was baked from a partial reach ([`ReachMode::Partial`]).
    pub partial: u64,
    /// Jobs pushed from a partial reach since the queue started.
    pub partial_opened: u64,
    /// Flagged chunks re-queued since the queue started, a chunk within their reach having
    /// arrived full.
    pub partial_requeued: u64,
}

/// What [`BakeQueue::push_batch`] pushed: the jobs' samples, the chunks with no block the pool
/// refused, and whether a job was a full first pass on a chunk with no block.
#[derive(Default)]
struct Pushed {
    samples: u64,
    refused0: usize,
    full_blank: bool,
}

/// The stages a queue step is timed in ([`StepStages::ms`]), in the order they run: the
/// frame's setup before the step (the light hold, the parameters, the bind group); the mirror
/// filter in five (the store following the window, the mirror's change list and the blocks of
/// chunks it does not hold full, the per-chunk sets' removals, the reach verdicts dropped
/// near a change, the changes marked for the candidate pool); the edit boxes, the withdrawal,
/// the digs' jobs, then the opening's candidate build, census, sort, reach and light checks and
/// listing (each 0 on a step that does not open), the job tables, the device poll, the
/// read-back's copy and the blocks' placement, the submit, the fades and the light summaries.
pub const STEP_STAGES: [&str; 21] = [
    "setup",
    "rewindow",
    "gone",
    "forget",
    "drop_reach",
    "touched",
    "boxes",
    "withdraw",
    "digs",
    "candidates",
    "census",
    "sort",
    "checks",
    "list",
    "refresh",
    "poll",
    "collect",
    "place",
    "submit",
    "fades",
    "summaries",
];
const S_SETUP: usize = 0;
const S_REWINDOW: usize = 1;
const S_GONE: usize = 2;
const S_FORGET: usize = 3;
const S_DROP_REACH: usize = 4;
const S_TOUCHED: usize = 5;
const S_BOXES: usize = 6;
const S_WITHDRAW: usize = 7;
const S_DIGS: usize = 8;
const S_CANDIDATES: usize = 9;
const S_CENSUS: usize = 10;
const S_SORT: usize = 11;
const S_CHECKS: usize = 12;
const S_LIST: usize = 13;
const S_REFRESH: usize = 14;
const S_POLL: usize = 15;
const S_COLLECT: usize = 16;
const S_PLACE: usize = 17;
const S_SUBMIT: usize = 18;
const S_FADES: usize = 19;
const S_SUMMARIES: usize = 20;

/// One queue step's CPU time by stage.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StepStages {
    /// Each stage's milliseconds, [`STEP_STAGES`] in order. The stages are laps of one clock,
    /// so they cover the step from the frame's call to the last stage's end.
    pub ms: [f64; STEP_STAGES.len()],
    /// The step's own milliseconds, from the frame's call to the step's return.
    pub total_ms: f64,
    /// Whether the step ran an opening.
    pub opened: bool,
}

impl StepStages {
    /// The stages' sum.
    pub fn sum(&self) -> f64 {
        self.ms.iter().sum()
    }
}

/// The laps of a step's stages: each lap adds the time since the last to a stage.
#[derive(Clone, Copy, Debug)]
struct Laps {
    at: Instant,
    ms: [f64; STEP_STAGES.len()],
}

impl Laps {
    fn new(at: Instant) -> Self {
        Self {
            at,
            ms: [0.0; STEP_STAGES.len()],
        }
    }

    fn lap(&mut self, stage: usize) {
        let now = stopwatch();
        self.ms[stage] += now.duration_since(self.at).as_secs_f64() * 1e3;
        self.at = now;
    }
}

/// The live queue's operating point over a second of a flight, read from the samples a chunk
/// baked that second drew: [`Regime::KeepingUp`] (B) when the coarse front keeps up, every
/// opening a small coarse job; [`Regime::Starved`] (A) when the near shell's full jobs hold the
/// loop, few chunks baked a second at many samples each; [`Regime::Mixed`] between.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Regime {
    Starved,
    KeepingUp,
    Mixed,
}

/// Samples a baked chunk under which a second reads [`Regime::KeepingUp`].
pub const REGIME_B_MAX: f64 = 3e5;
/// Samples a baked chunk over which a second reads [`Regime::Starved`].
pub const REGIME_A_MIN: f64 = 6e5;

impl Regime {
    /// A second's regime from the samples it read back and the chunks it baked; `None` for a
    /// second that read no sample (an idle queue). A second that drew samples and baked no
    /// chunk is starved.
    pub fn of(samples: u64, chunks: u64) -> Option<Self> {
        if samples == 0 {
            return None;
        }
        if chunks == 0 {
            return Some(Self::Starved);
        }
        let per = samples as f64 / chunks as f64;
        Some(if per < REGIME_B_MAX {
            Self::KeepingUp
        } else if per > REGIME_A_MIN {
            Self::Starved
        } else {
            Self::Mixed
        })
    }

    /// `A`, `B` or `mixed`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Starved => "A",
            Self::KeepingUp => "B",
            Self::Mixed => "mixed",
        }
    }
}

/// A run's queue steps, each tagged with its second's regime once the second ends: the
/// per-regime summary of the step's time, its opening share and its stages.
#[derive(Clone, Debug, Default)]
pub struct StepTally {
    /// The steps of the second not yet tagged.
    pending: Vec<StepStages>,
    /// Tagged steps: A, B, mixed, and idle (a second that read no sample).
    tagged: [Vec<StepStages>; 4],
}

/// The tally's buckets in order, by name.
const TALLY_NAMES: [&str; 4] = ["A", "B", "mixed", "idle"];

fn tally_index(r: Option<Regime>) -> usize {
    match r {
        Some(Regime::Starved) => 0,
        Some(Regime::KeepingUp) => 1,
        Some(Regime::Mixed) => 2,
        None => 3,
    }
}

/// The `p`th percentile of `v` by nearest rank (0 for none), `v` sorted in place.
fn nearest_rank(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    let i = ((p / 100.0) * v.len() as f64).ceil() as usize;
    v[i.clamp(1, v.len()) - 1]
}

impl StepTally {
    /// A step of the current second.
    pub fn push(&mut self, s: StepStages) {
        self.pending.push(s);
    }

    /// Ends the second: its steps take `regime`.
    pub fn tag(&mut self, regime: Option<Regime>) {
        let i = tally_index(regime);
        self.tagged[i].append(&mut self.pending);
    }

    /// The steps tagged `regime`.
    pub fn count(&self, regime: Option<Regime>) -> usize {
        self.tagged[tally_index(regime)].len()
    }

    /// The step time's `p`th percentile over the steps tagged `regime`, of those that opened
    /// (`Some(true)`), did not (`Some(false)`) or all (`None`).
    pub fn percentile(&self, regime: Option<Regime>, opened: Option<bool>, p: f64) -> f64 {
        let mut v: Vec<f64> = self.tagged[tally_index(regime)]
            .iter()
            .filter(|s| opened.is_none_or(|o| s.opened == o))
            .map(|s| s.total_ms)
            .collect();
        nearest_rank(&mut v, p)
    }

    /// The summary: a line per bucket that holds a step, then the pooled line, each `queue
    /// step <bucket>: N steps, p50 X ms, p99 Y ms, opened K (share), open p50 X ms, not p50 Y
    /// ms; stages p50/p99 ms (open p50 / not p50): setup a/b (c/d), ...`.
    pub fn summary(&self) -> Vec<String> {
        let all: Vec<StepStages> = self.tagged.iter().flatten().copied().collect();
        let mut out = Vec::new();
        for (name, steps) in TALLY_NAMES.iter().zip(&self.tagged) {
            if !steps.is_empty() {
                out.push(Self::line(name, steps));
            }
        }
        if !all.is_empty() {
            out.push(Self::line("pooled", &all));
        }
        out
    }

    fn line(name: &str, steps: &[StepStages]) -> String {
        use std::fmt::Write as _;
        let pick = |opened: Option<bool>, f: &dyn Fn(&StepStages) -> f64, p: f64| {
            let mut v: Vec<f64> = steps
                .iter()
                .filter(|s| opened.is_none_or(|o| s.opened == o))
                .map(f)
                .collect();
            nearest_rank(&mut v, p)
        };
        let opened = steps.iter().filter(|s| s.opened).count();
        let mut line = format!(
            "queue step {name}: {} steps, p50 {:.3} ms, p99 {:.3} ms, opened {opened} ({:.3}), open p50 {:.3} ms, not p50 {:.3} ms; stages p50/p99 ms (open p50 / not p50):",
            steps.len(),
            pick(None, &|s| s.total_ms, 50.0),
            pick(None, &|s| s.total_ms, 99.0),
            opened as f64 / steps.len() as f64,
            pick(Some(true), &|s| s.total_ms, 50.0),
            pick(Some(false), &|s| s.total_ms, 50.0),
        );
        for (k, stage) in STEP_STAGES.iter().enumerate() {
            let _ = write!(
                line,
                " {stage} {:.3}/{:.3} ({:.3}/{:.3})",
                pick(None, &|s| s.ms[k], 50.0),
                pick(None, &|s| s.ms[k], 99.0),
                pick(Some(true), &|s| s.ms[k], 50.0),
                pick(Some(false), &|s| s.ms[k], 50.0),
            );
        }
        line
    }
}

/// A block fading from what it showed to its target over `frames` ([`FADE`], or [`DIG_FADE`]
/// for a dig job's).
struct Fade {
    first: u64,
    from: Vec<u32>,
    frame: u32,
    frames: u32,
}

/// A chunk's dig bricks (slots), and whether one of them is in ring 1.
#[derive(Clone, Debug, Default)]
struct DigChunk {
    slots: std::collections::BTreeSet<u16>,
    near: bool,
}

/// What the frame hands the queue's step.
pub struct QueueView<'a> {
    /// The camera's position and forward direction, in voxels.
    pub eye: [f32; 3],
    pub forward: [f32; 3],
    /// Whether a chunk is resident full (a streamed world); `None` for a bounded world, whose
    /// mirror holds every chunk it has and whose absent chunks are empty.
    pub resident: Option<&'a dyn Fn(ChunkCoord) -> bool>,
    /// Samples this step draws at most; `None` sizes it by [`BakeQueue::budget`].
    pub samples: Option<u64>,
    /// The last frame's interval in seconds, which a per-second budget sizes the slice by
    /// ([`Budget::slice_ms`]); 0 when unknown (the first step), which gives its floor.
    pub interval_s: f64,
    /// Chunks from the focus the light-sample ranges are held out to; `None` the window's far
    /// corner plus the light radius.
    pub hold: Option<i32>,
    /// The wider of the view's horizontal and vertical half-angles, in radians
    /// ([`view_half_angle`]); the queue's view cone adds [`QUEUE_VIEW_MARGIN_DEG`] to it.
    pub half_angle: f32,
    /// The view's own horizontal and vertical half-angles, in radians, for the reading of the
    /// chunks the cone holds outside the frustum widened by the same margin
    /// ([`QueueStats::cone_only`]); `None` reads none. The frustum's up is the camera's, with
    /// no roll: `right = forward x Z`, `up = right x forward`.
    pub frustum: Option<[f32; 2]>,
    /// The streamer's full radius in chunks about the camera's chunk, for the census's count
    /// of the candidates whose reach crosses it ([`QueueStats::reach_bound`]); `None` (a
    /// bounded world) counts none.
    pub full_radius: Option<i32>,
}

/// The live queue's angle weight by default (ADR-0048's `W_ANGLE`, [`QueueOrder::Score`]).
pub const QUEUE_ANGLE_WEIGHT: f32 = 4.0;
/// The live queue's refine weight when the score folds the class in ([`Refine::Weight`],
/// ADR-0048's `W_REFINE`). The default order keeps the class as a tier ([`Refine::Tier`]);
/// this is the comparison arm's weight.
pub const QUEUE_REFINE_WEIGHT: f32 = 2.0;

/// How the score treats a chunk that holds a block (a coarse one to refine, or a whole re-bake
/// of ring 3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Refine {
    /// The score is multiplied by the weight: one list, whatever the class.
    Weight(f32),
    /// The class is the leading key, the score inside each class: every chunk with no block
    /// before any refinement (ADR-0046's coarse-first rule), with its break and its per-class
    /// reach checks.
    Tier,
}

/// The order the live queue opens its candidates in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum QueueOrder {
    /// ADR-0048's score ([`queue_score`]), ties by the view cosine then the coordinate.
    Score { angle: f32, refine: Refine },
    /// The class first, then the squared chunk distance, ties by the view (ADR-0046 as built).
    Distance,
}

/// ADR-0048's score at [`QUEUE_ANGLE_WEIGHT`] with the class as the leading key
/// ([`Refine::Tier`], ADR-0046's coarse-first rule).
impl Default for QueueOrder {
    fn default() -> Self {
        Self::Score {
            angle: QUEUE_ANGLE_WEIGHT,
            refine: Refine::Tier,
        }
    }
}

impl QueueOrder {
    /// The order in words: `order score, angle 4, refine 2`, `order score, angle 4, refine
    /// tier` or `order distance`.
    pub fn words(&self) -> String {
        match self {
            Self::Score { angle, refine } => format!(
                "order score, angle {angle}, refine {}",
                match refine {
                    Refine::Weight(r) => r.to_string(),
                    Refine::Tier => "tier".to_owned(),
                }
            ),
            Self::Distance => "order distance".to_owned(),
        }
    }
}

/// A candidate of an opening: its sort key (the leading class or 0, the score's bits or the
/// squared distance, the view tie, the coordinate), its class (1 when it holds a block), its
/// squared chunk distance and its angle term.
#[derive(Clone, Copy, Debug)]
struct Cand {
    lead: u8,
    key: u64,
    tie: i32,
    c: [i32; 3],
    class: u8,
    /// Whether its block is coarse.
    coarse: bool,
    d2: i64,
    a: f32,
}

impl Cand {
    /// The order's key: unique, since it ends in the coordinate.
    fn sort_key(&self) -> (u8, u64, i32, [i32; 3]) {
        (self.lead, self.key, self.tie, self.c)
    }
}

/// A candidate under its key, least first in a [`std::collections::BinaryHeap`].
#[derive(Clone, Copy, Debug)]
struct HeapCand(Cand);

impl PartialEq for HeapCand {
    fn eq(&self, other: &Self) -> bool {
        self.0.sort_key() == other.0.sort_key()
    }
}
impl Eq for HeapCand {}
impl PartialOrd for HeapCand {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for HeapCand {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other.0.sort_key().cmp(&self.0.sort_key())
    }
}

/// The kept candidates in their order, sorted only as far as an opening walks: the walked
/// head in order, the rest a heap popped on demand. The keys are unique, so the walk sees the
/// order a full sort gives.
#[derive(Debug, Default)]
struct Ordered {
    done: Vec<Cand>,
    heap: std::collections::BinaryHeap<HeapCand>,
}

impl Ordered {
    fn new(cands: Vec<Cand>) -> Self {
        Self {
            done: Vec::new(),
            heap: cands.into_iter().map(HeapCand).collect(),
        }
    }

    /// The `i`th candidate in order, sorting the heap's head as far as `i`.
    fn get(&mut self, i: usize) -> Option<Cand> {
        while self.done.len() <= i {
            self.done.push(self.heap.pop()?.0);
        }
        Some(self.done[i])
    }

    /// Every candidate, in no order.
    fn all(&self) -> impl Iterator<Item = &Cand> + '_ {
        self.done.iter().chain(self.heap.iter().map(|h| &h.0))
    }
}

/// What the kept order's scores read: the camera's chunk, the forward direction and the
/// half-angle (bits), the order and the near shell.
#[derive(Clone, Copy, Debug, PartialEq)]
struct OrderView {
    eye: [i32; 3],
    forward: [u32; 3],
    half_angle: u32,
    order: QueueOrder,
    near: i32,
}

/// The candidate `c` of `class` (and whether its block is `coarse`) scored under `order` from
/// the camera's chunk `eye` and `view`.
fn cand_of(
    c: [i32; 3],
    class: u8,
    coarse: bool,
    eye: [i32; 3],
    view: &QueueView<'_>,
    order: QueueOrder,
    near: i32,
) -> Cand {
    let d = [0, 1, 2].map(|k| c[k] - eye[k]);
    let d2: i64 = d.iter().map(|&v| i64::from(v).pow(2)).sum();
    let len = (d2 as f32).sqrt().max(1.0);
    let dot: f32 = (0..3).map(|k| d[k] as f32 * view.forward[k]).sum::<f32>() / len;
    let a = angle_term(d, view.forward, view.half_angle, near);
    let (lead, key) = match order {
        QueueOrder::Distance => (class, d2 as u64),
        QueueOrder::Score {
            angle,
            refine: Refine::Weight(r),
        } => (
            0,
            u64::from(queue_score(d, a, class == 1, angle, r).to_bits()),
        ),
        QueueOrder::Score {
            angle,
            refine: Refine::Tier,
        } => (
            class,
            u64::from(queue_score(d, a, false, angle, 1.0).to_bits()),
        ),
    };
    Cand {
        lead,
        key,
        tie: -(dot * 1000.0) as i32,
        c,
        class,
        coarse,
        d2,
        a,
    }
}

/// Candidates in the view the reach census reads at most ([`QueueStats::view_reach`]).
pub const VIEW_REACH_SAMPLE: usize = 64;
/// Steps between two reach censuses of the view.
pub const VIEW_REACH_EVERY: u64 = 60;

/// Degrees the queue's view cone widens the view's half-angle by (ADR-0048's margin).
pub const QUEUE_VIEW_MARGIN_DEG: f32 = 15.0;

/// The wider of a view's half-angles, in radians: the vertical one (half of `vfov`) or the
/// horizontal one at `aspect` (width over height), never the corner's.
pub fn view_half_angle(vfov: f32, aspect: f32) -> f32 {
    let v = vfov * 0.5;
    let h = (v.tan() * aspect).atan();
    v.max(h)
}

/// The horizontal and vertical half-angles of a view of `vfov` at `aspect`, in radians.
pub fn view_half_angles(vfov: f32, aspect: f32) -> [f32; 2] {
    let v = vfov * 0.5;
    [(v.tan() * aspect).atan(), v]
}

/// Whether a chunk at integer chunk offset `offset` from the camera's lies in the near shell:
/// within `near` chunks (Chebyshev) every way.
pub fn in_near_shell(offset: [i32; 3], near: i32) -> bool {
    offset.iter().all(|v| v.abs() <= near)
}

/// ADR-0048's angle term `a` of a chunk at integer chunk offset `offset` from the camera's: its
/// angle to `forward` beyond the view cone (`half_angle` plus [`QUEUE_VIEW_MARGIN_DEG`]), over
/// pi; 0 inside the cone and in the near shell ([`in_near_shell`]) whatever its direction.
pub fn angle_term(offset: [i32; 3], forward: [f32; 3], half_angle: f32, near: i32) -> f32 {
    if in_near_shell(offset, near) {
        return 0.0;
    }
    let o = offset.map(|v| v as f32);
    let len = (o[0] * o[0] + o[1] * o[1] + o[2] * o[2]).sqrt();
    let flen = (forward[0] * forward[0] + forward[1] * forward[1] + forward[2] * forward[2])
        .sqrt()
        .max(f32::MIN_POSITIVE);
    let cos = ((o[0] * forward[0] + o[1] * forward[1] + o[2] * forward[2]) / (len * flen))
        .clamp(-1.0, 1.0);
    let cone = half_angle + QUEUE_VIEW_MARGIN_DEG.to_radians();
    if cos >= cone.cos() {
        return 0.0;
    }
    ((cos.acos() - cone) / std::f32::consts::PI).max(0.0)
}

/// ADR-0048's score of a candidate at integer chunk offset `offset` with angle term `a`
/// ([`angle_term`]): `|o| x (1 + angle_weight x a) x (refine_weight for a coarse block, else
/// 1)`. Lower opens first.
pub fn queue_score(
    offset: [i32; 3],
    a: f32,
    coarse: bool,
    angle_weight: f32,
    refine_weight: f32,
) -> f32 {
    let o = offset.map(|v| v as f32);
    let len = (o[0] * o[0] + o[1] * o[1] + o[2] * o[2]).sqrt();
    len * (1.0 + angle_weight * a) * if coarse { refine_weight } else { 1.0 }
}

/// Where a chunk lies against the queue's view ([`QueueStats::view_waiting`]): the near shell,
/// in the view cone beyond it, or out of view.
pub const VIEW_SHELL: usize = 0;
pub const VIEW_IN: usize = 1;
pub const VIEW_OUT: usize = 2;

/// Whether `offset` lies inside the frustum of half-angles `half` (horizontal, vertical, each
/// widened by [`QUEUE_VIEW_MARGIN_DEG`]) about `forward`, its up the camera's with no roll.
fn in_widened_frustum(offset: [i32; 3], forward: [f32; 3], half: [f32; 2]) -> bool {
    let f = glam::Vec3::from(forward).normalize_or_zero();
    let right = f.cross(glam::Vec3::Z).normalize_or(glam::Vec3::NEG_Y);
    let up = right.cross(f);
    let o = glam::Vec3::from(offset.map(|v| v as f32));
    let z = o.dot(f);
    if z <= 0.0 {
        return false;
    }
    let m = QUEUE_VIEW_MARGIN_DEG.to_radians();
    o.dot(right).abs().atan2(z) <= half[0] + m && o.dot(up).abs().atan2(z) <= half[1] + m
}

/// The distance bands of the weight log ([`QueueStats::weight_census`]): chunks 0 to 2, 3 to
/// 5, 6 to 10, over 10.
fn distance_band(len: f32) -> usize {
    if len < 2.5 {
        0
    } else if len < 5.5 {
        1
    } else if len < 10.5 {
        2
    } else {
        3
    }
}

/// The angle bands of the weight log: 0, under 0.2, 0.2 to 0.4, over 0.4.
fn angle_band(a: f32) -> usize {
    if a <= 0.0 {
        0
    } else if a < 0.2 {
        1
    } else if a <= 0.4 {
        2
    } else {
        3
    }
}

/// The live queue (ADR-0046's scheduler, Plan 0059 Phase 8 as re-read at Phase 7b): the brick
/// level of every full chunk in the mirror, baked a slice a frame on the render thread. A chunk
/// is opened in [`BakeQueue::order`] (ADR-0048's score of distance and angle to the view by
/// default; [`QueueOrder`]), once every chunk within the light radius of it is mirrored or known
/// empty and every emitter range its table reads is held. Its faces are
/// listed from the stream workers' masks ([`FacePrep`]) where they arrived, else inline. A first
/// block is written directly; a block that replaces one of the same layout fades over [`FADE`]
/// frames. A chunk leaving the mirror frees its block.
///
/// **Coarse first** (Plan 0059 Phase 8c). With `coarse_samples` above 0, a chunk with no block
/// beyond `near` chunks (Chebyshev) of the camera's is first baked at `coarse_samples` a brick
/// direction, its block flagged coarse ([`TABLE_COARSE`]); one within `near` at the full count.
/// Under the distance order and the score's tier ([`Refine::Tier`]) every chunk with no block
/// comes before every coarse one, so a chunk the mirror gains is lit coarse at the head of the
/// queue, and the coarse ones are then refined in order; under the score's refine weight a
/// coarse chunk takes its place in the one list, its score times the weight. A
/// refinement lists the same faces, so its block has the coarse one's layout and fades from it.
///
/// **Digs** (ADR-0046's rings, Plan 0059 Phase 9 as re-read at Phase 7b). An edit's box
/// ([`BakeQueue::invalidate`]) is read at the next step against the store as it is then: the
/// bricks within [`DIG_REACH`] voxels of it ([`dig_ring`]) re-bake at the brick level ahead of
/// every other job, the chunks holding a brick the box dilated by one voxel touches first (ring
/// 1), the rest by distance (ring 2); each chunk is listed again and keeps every other brick's
/// record. While a dig's bricks wait the slice grows by [`BakeQueue::edit_ms`]. A chunk with no
/// block is left to its first pass. When a touched brick held light before the edit, every chunk
/// with a block within the light radius of the box re-bakes whole (ring 3), after the first
/// passes and with the refinements, fading. A chunk re-dug while its dig job runs throws the
/// job's block away and re-bakes the union of its bricks.
pub struct BakeQueue {
    pub settings: BakeSettings,
    /// The bake budget: the slice a frame, or milliseconds a second sized by the view's
    /// interval ([`QueueView::interval_s`]).
    pub budget: Budget,
    /// A brick direction's samples in the coarse pass; 0 bakes every chunk at the full count.
    pub coarse_samples: u32,
    /// Chunks (Chebyshev) around the camera's that the score's angle term and the withdrawal
    /// treat as the near shell.
    pub near: i32,
    /// Chunks (Chebyshev) around the camera's within which a chunk with no block takes the full
    /// count directly; beyond it, or everywhere when `None`, its first pass is the coarse pass
    /// and a chunk refines as class 1. [`Frame::set_bake_coarse`](crate::Frame::set_bake_coarse)
    /// sets it to `Some(near)`.
    pub full_first: Option<i32>,
    /// Milliseconds a frame the slice grows by while a dig's bricks wait.
    pub edit_ms: f64,
    /// Whether a dig that touched lit bricks re-bakes the chunks within the light radius (ring 3).
    pub reach_ring: bool,
    /// The order candidates open in.
    pub order: QueueOrder,
    /// Whether the queue withdraws the jobs of chunks the camera has passed (ADR-0048): at a
    /// step with a candidate in the near shell or the view cone ready, a job that is not a
    /// dig's and not a whole re-bake of a full block, whose chunk lies outside the near shell
    /// and outside the keep cone (the view cone's margin doubled), and that opened before the
    /// camera's chunk last changed, is taken out of the loop, its place freed; its chunk keeps
    /// the block it had and is a candidate again. A still camera withdraws nothing. A step with
    /// a sample count (the pre-roll) withdraws nothing.
    pub withdraw: bool,
    /// Whether a chunk whose reach is not resident full bakes from the chunks that are
    /// ([`ReachMode`]).
    pub reach_mode: ReachMode,
    /// Edit boxes since the last step.
    boxes: Vec<[[i32; 3]; 2]>,
    /// Chunks with dig bricks to re-bake, not yet in a job; and those in a job, with the bricks
    /// the job re-bakes.
    digs: BTreeMap<[i32; 3], DigChunk>,
    dig_open: std::collections::HashMap<[i32; 3], DigChunk>,
    /// Chunks with a block that re-bake whole (ring 3, or a job thrown away by an edit).
    reach: std::collections::BTreeSet<[i32; 3]>,
    /// Dug chunks' faces, listed at the edit for their provisional block, for their dig job.
    listed: std::collections::HashMap<[i32; 3], ChunkFaces>,
    /// The jobs pending or open, and whether each is a coarse pass.
    job_coarse: std::collections::HashMap<[i32; 3], bool>,
    lp: JobLoop,
    prepared: std::collections::HashMap<[i32; 3], Arc<PreparedMasks>>,
    prepared_bytes: usize,
    /// The camera's chunk at the last step.
    eye: [i32; 3],
    /// The steps at which the camera's chunk changed, counted since the queue started.
    eye_moves: u64,
    /// Each listed job's [`BakeQueue::eye_moves`] when it opened: a job is withdrawn only once
    /// the camera's chunk has changed since (ADR-0048's Outcome), so a still camera withdraws
    /// nothing and the queue drains.
    job_eye: std::collections::HashMap<[i32; 3], u64>,
    faceless: std::collections::HashSet<[i32; 3]>,
    /// Chunks an edit touched while their job ran: the job's block is thrown away.
    stale: std::collections::HashSet<[i32; 3]>,
    /// Candidates not ready at a check, and the step that checked them.
    blocked: std::collections::HashMap<[i32; 3], u64>,
    fades: BTreeMap<[i32; 3], Fade>,
    steps: u64,
    /// The step before which no candidate is sorted: set when none was ready.
    quiet_until: u64,
    /// Whether the last count of the depth ran with no job pending or open: a job read back
    /// after the count can leave a candidate it did not see (a coarse block to refine, a chunk
    /// whose job an edit threw away).
    counted_idle: bool,
    /// The mirror's window and chunk count at the last step: a change counts the depth again.
    mirror_seen: (Option<ChunkExtent>, usize),
    /// [`face_classes`] of the world's table, and each row's translucency it was made from.
    classes: Option<(Vec<bool>, Arc<Vec<u8>>)>,
    /// The block a fade shows this step, kept between steps.
    fade_scratch: Vec<u32>,
    /// Chunks that entered the view cone or the near shell with no full block: the step they
    /// entered at, and whether a block has landed since. It can hold most of the mirror's full
    /// chunks, so it is pruned to the chunks the mirror holds full at each census, not at every
    /// step: an entry whose chunk left and came back between two censuses keeps its old step.
    entered: std::collections::HashMap<[i32; 3], (u64, bool)>,
    /// Steps from entering to the first block and to the full block, since the queue started.
    ages_first: Vec<u32>,
    ages_full: Vec<u32>,
    /// For each candidate found ready since the last [`BakeQueue::take_angle_needs`], the angle
    /// weight at which the nearest candidate in the cone or shell would have outranked it.
    angle_needs: Vec<f32>,
    /// The step of the last reach census of the view ([`QueueStats::view_reach`]).
    reach_census_at: u64,
    /// Whether the next opening reads the view's census ([`BakeQueue::request_census`]).
    census_due: bool,
    /// Every opening reads the view's census, the in-view ages and the angle weights, not only
    /// one after a [`BakeQueue::request_census`]: a reading, which costs the step about a
    /// census (0.8 ms over the streamed cave's 17 000 candidates) at each opening.
    pub census_every_opening: bool,
    stats: QueueStats,
    /// The current step's stage laps.
    laps: Laps,
    /// The candidates, kept between steps: every full chunk in the mirror with no block, a
    /// coarse one or a whole re-bake waiting (ring 3), not known faceless, not in a job and
    /// not a dig's, with its class (1 when it holds a block) and whether its block is coarse.
    /// Re-read for the chunks the mirror, the store or the queue changed ([`BakeQueue::touched`]).
    pool: std::collections::HashMap<[i32; 3], (u8, bool)>,
    /// Chunks whose candidacy is read again at the next opening.
    touched: ChunkSet,
    /// The pool is read again whole at the next opening: at the start, and when the mirror's
    /// or the store's changes went untracked.
    pool_stale: bool,
    /// The jobs' chunks at the last reading of the pool: a chunk joining or leaving a job is
    /// read again.
    pool_jobs: std::collections::HashSet<[i32; 3]>,
    /// The pool's candidates in their order, and the view they were scored under; dirty when
    /// a candidate joined the pool or changed class.
    ordered: Ordered,
    order_view: Option<OrderView>,
    order_dirty: bool,
    /// Each checked candidate's reach verdict ([`reach_ready`]), dropped when a chunk within
    /// the light radius and one of it changes in the mirror.
    reach_cache: ChunkMap<Reach>,
    /// Chunks whose block was baked from a partial reach ([`ReachMode::Partial`]).
    partial: ChunkSet,
    /// The jobs pending or open that bake from a partial reach.
    job_partial: ChunkSet,
    /// Chunks the opening's checks found bakeable from a partial reach, read when listed.
    partial_checked: ChunkSet,
    /// Flagged chunks (block or job) a chunk within whose reach arrived full: candidates again,
    /// as a refinement.
    partial_due: ChunkSet,
}

impl BakeQueue {
    /// A queue whose budget is `slice_ms` a frame ([`Budget::PerFrame`]).
    pub fn new(settings: BakeSettings, slice_ms: f64) -> Self {
        Self {
            settings,
            budget: Budget::PerFrame { ms: slice_ms },
            coarse_samples: DEFAULT_COARSE_SAMPLES,
            near: DEFAULT_NEAR,
            full_first: Some(DEFAULT_NEAR),
            edit_ms: DEFAULT_BAKE_EDIT_MS,
            reach_ring: true,
            order: QueueOrder::default(),
            withdraw: true,
            boxes: Vec::new(),
            digs: BTreeMap::new(),
            dig_open: Default::default(),
            reach: Default::default(),
            listed: Default::default(),
            job_coarse: Default::default(),
            lp: {
                let mut lp = JobLoop::new(Vec::new(), LOOP_SLOTS..OPEN_JOBS).timed();
                lp.ended = Some(Vec::new());
                lp
            },
            prepared: Default::default(),
            prepared_bytes: 0,
            eye: [0; 3],
            eye_moves: 0,
            job_eye: Default::default(),
            faceless: Default::default(),
            stale: Default::default(),
            blocked: Default::default(),
            fades: BTreeMap::new(),
            steps: 0,
            quiet_until: 0,
            counted_idle: false,
            mirror_seen: (None, 0),
            classes: None,
            fade_scratch: Vec::new(),
            entered: Default::default(),
            ages_first: Vec::new(),
            ages_full: Vec::new(),
            angle_needs: Vec::new(),
            reach_census_at: 0,
            census_due: true,
            census_every_opening: false,
            stats: QueueStats {
                ns_per_sample: 20.0,
                ..QueueStats::default()
            },
            laps: Laps::new(stopwatch()),
            pool: Default::default(),
            touched: Default::default(),
            pool_stale: true,
            pool_jobs: Default::default(),
            ordered: Ordered::default(),
            order_view: None,
            order_dirty: true,
            reach_cache: Default::default(),
            reach_mode: ReachMode::Full,
            partial: Default::default(),
            job_partial: Default::default(),
            partial_checked: Default::default(),
            partial_due: Default::default(),
        }
    }

    /// Whether `c` is a candidate ([`BakeQueue::pool`]): its class and whether its block is
    /// coarse.
    fn candidacy(
        &self,
        store: &Store,
        gpu: &GpuWorld,
        in_job: &std::collections::HashSet<[i32; 3]>,
        c: [i32; 3],
    ) -> Option<(u8, bool)> {
        let cc = ChunkCoord(c);
        if !gpu.is_mirrored(cc) || gpu.is_lod(cc) {
            return None;
        }
        let has = store.blocks.contains_key(&c);
        let coarse = has && store.is_coarse(cc);
        let wants = !has || coarse || self.reach.contains(&c) || self.partial_due.contains(&c);
        (wants
            && !self.faceless.contains(&c)
            && !in_job.contains(&c)
            && !self.digs.contains_key(&c))
        .then_some((u8::from(has), coarse))
    }

    /// Brings the pool up to date: whole when it is stale, else for the chunks touched since,
    /// the store's changed blocks and the chunks that joined or left a job. A candidate that
    /// joins or changes class leaves the kept order dirty; one that leaves is skipped by the
    /// walk.
    fn sync_pool(&mut self, run: &BakeRun<'_>, store: &mut Store) {
        let gpu = run.gpu;
        let in_job: std::collections::HashSet<[i32; 3]> = self.lp.chunks().map(|c| c.0).collect();
        match store.take_touched() {
            Some(t) => self.touched.extend(t),
            None => self.pool_stale = true,
        }
        if self.pool_stale {
            let pool: std::collections::HashMap<[i32; 3], (u8, bool)> = gpu
                .full_chunks()
                .filter_map(|c| self.candidacy(store, gpu, &in_job, c.0).map(|v| (c.0, v)))
                .collect();
            self.stats.scanned += gpu.mirrored_chunks() as u64;
            self.pool = pool;
            self.pool_stale = false;
            self.order_dirty = true;
            self.touched.clear();
        } else {
            let moved: Vec<[i32; 3]> = self
                .pool_jobs
                .symmetric_difference(&in_job)
                .copied()
                .collect();
            self.touched.extend(moved);
            for c in std::mem::take(&mut self.touched) {
                let now = self.candidacy(store, gpu, &in_job, c);
                let was = self.pool.get(&c).copied();
                if now == was {
                    continue;
                }
                match now {
                    Some(v) => {
                        self.pool.insert(c, v);
                        self.order_dirty = true;
                    }
                    None => {
                        self.pool.remove(&c);
                    }
                }
            }
        }
        self.pool_jobs = in_job;
        #[cfg(debug_assertions)]
        {
            let jobs = &self.pool_jobs;
            let whole: std::collections::HashMap<[i32; 3], (u8, bool)> = gpu
                .full_chunks()
                .filter_map(|c| self.candidacy(store, gpu, jobs, c.0).map(|v| (c.0, v)))
                .collect();
            debug_assert_eq!(self.pool, whole, "the kept pool against a whole reading");
        }
    }

    /// [`reach_ready`] through the cache of verdicts; a bounded world with nothing waiting to
    /// upload is ready without a walk. A miss counts in [`QueueStats::reach_checks`].
    fn reach_cached(
        &mut self,
        run: &BakeRun<'_>,
        coord: ChunkCoord,
        lr: i32,
        resident: Option<&dyn Fn(ChunkCoord) -> bool>,
    ) -> Reach {
        if resident.is_none() && run.gpu.pending_chunks() == 0 {
            return Reach::Ready;
        }
        if let Some(&v) = self.reach_cache.get(&coord.0) {
            // A streamer's residency can move a frame ahead of the mirror; a bounded world's
            // verdict reads the mirror alone.
            debug_assert!(
                resident.is_some() || v == reach_of(run, coord, lr, resident),
                "the cached reach of {coord:?}"
            );
            return v;
        }
        self.stats.reach_checks += 1;
        let v = reach_of(run, coord, lr, resident);
        self.reach_cache.insert(coord.0, v);
        v
    }

    /// Whether `coord` may bake under [`BakeQueue::reach`], and whether from a partial reach:
    /// `Some(false)` when its reach is ready, `Some(true)` under [`ReachMode::Partial`] when
    /// nothing in its reach waits to upload and the reach crosses the streamer's full radius
    /// about `eye` (its distance in chunks plus `lr` exceeds it), else `None`.
    fn bakeable(
        &mut self,
        run: &BakeRun<'_>,
        coord: ChunkCoord,
        lr: i32,
        view: &QueueView<'_>,
        eye: [i32; 3],
    ) -> Option<bool> {
        match self.reach_cached(run, coord, lr, view.resident) {
            Reach::Ready => Some(false),
            Reach::Partial
                if self.reach_mode == ReachMode::Partial
                    && view.full_radius.is_some_and(|r| {
                        let d2: i64 = (0..3).map(|k| i64::from(coord.0[k] - eye[k]).pow(2)).sum();
                        (d2 as f32).sqrt() + lr as f32 > r as f32
                    }) =>
            {
                Some(true)
            }
            _ => None,
        }
    }

    /// Re-queues as a refinement each chunk flagged partial (its block, or its job's) within
    /// whose reach a chunk of `changed` is now held full: a chunk arriving, or a held one
    /// changing. `None` (the changes went untracked) re-queues every flagged chunk. Nothing to
    /// do under [`ReachMode::Full`], which flags none.
    fn requeue_partial(&mut self, gpu: &GpuWorld, changed: Option<&[ChunkCoord]>) {
        if self.partial.is_empty() && self.job_partial.is_empty() {
            return;
        }
        let due: Vec<[i32; 3]> = match changed {
            None => self
                .partial
                .iter()
                .chain(self.job_partial.iter())
                .copied()
                .collect(),
            Some(ch) => {
                let lr = gpu.light_radius();
                let held: ChunkSet = ch
                    .iter()
                    .filter(|c| gpu.is_mirrored(**c) && !gpu.is_lod(**c))
                    .map(|c| c.0)
                    .collect();
                if held.is_empty() {
                    return;
                }
                let flagged =
                    |c: &[i32; 3]| self.partial.contains(c) || self.job_partial.contains(c);
                let mut due = Vec::new();
                // Each side walks its reach over the other's set: the smaller side walks.
                if self.partial.len() + self.job_partial.len() <= held.len() {
                    for p in self.partial.iter().chain(self.job_partial.iter()) {
                        let near = reach_offsets(lr)
                            .any(|o| held.contains(&[p[0] + o[0], p[1] + o[1], p[2] + o[2]]));
                        if near {
                            due.push(*p);
                        }
                    }
                } else {
                    for n in &held {
                        for o in reach_offsets(lr) {
                            let p = [n[0] + o[0], n[1] + o[1], n[2] + o[2]];
                            if flagged(&p) {
                                due.push(p);
                            }
                        }
                    }
                }
                due
            }
        };
        for p in due {
            if self.partial_due.insert(p) {
                self.stats.partial_requeued += 1;
                self.touched.insert(p);
                self.counted_idle = false;
                self.quiet_until = self.steps;
            }
        }
    }

    /// Drops the cached reach verdicts within `lr + 1` chunks (Chebyshev) of a chunk of
    /// `changed`; too many pairs to compare drop them all.
    fn drop_reach(&mut self, changed: &[ChunkCoord], lr: i32) {
        if changed.is_empty() || self.reach_cache.is_empty() {
            return;
        }
        if changed.len().saturating_mul(self.reach_cache.len()) > 1 << 22 {
            self.reach_cache.clear();
            return;
        }
        let r = lr + 1;
        // The verdicts within `r` (Chebyshev) of a changed chunk go. Removing each changed
        // chunk's neighbourhood by key costs `(2 r + 1)^3` lookups a change; a scan of the cache
        // costs its length times the changes. In a flight a few chunks change a step against
        // thousands of cached verdicts, so the keys are removed directly unless the scan is
        // smaller. Either way the same verdicts go.
        let side = (2 * r + 1) as usize;
        if changed.len().saturating_mul(side * side * side) < self.reach_cache.len() {
            for n in changed {
                for dz in -r..=r {
                    for dy in -r..=r {
                        for dx in -r..=r {
                            self.reach_cache
                                .remove(&[n.0[0] + dx, n.0[1] + dy, n.0[2] + dz]);
                        }
                    }
                }
            }
            return;
        }
        self.reach_cache.retain(|c, _| {
            !changed
                .iter()
                .any(|n| (0..3).all(|k| (n.0[k] - c[k]).abs() <= r))
        });
    }

    pub fn stats(&self) -> QueueStats {
        self.stats
    }

    /// Steps (frames) from a chunk entering the view cone or the near shell with no full block
    /// to its first block, and to its full block, one entry a chunk since the queue started. A
    /// chunk that entered with a coarse block reads 0 to its first. Entry is read at the
    /// openings that read the census ([`BakeQueue::census_every_opening`] for all of them), so
    /// it lags by the steps between two.
    pub fn view_ages(&self) -> (&[u32], &[u32]) {
        (&self.ages_first, &self.ages_full)
    }

    /// The pool words the places of the jobs pending and open hold: with the store's blocks,
    /// its header and its free words, the whole pool.
    pub fn job_words(&self) -> u64 {
        self.lp.place_words(self.record_words())
    }

    /// The angle weights read since the last call: for each candidate found ready, the weight
    /// above which the nearest candidate in the view cone or the near shell at that opening
    /// (ready or not) would have outranked it under the score at a refine weight of 1; 0 for
    /// one in the cone or shell, or with no nearer such candidate.
    pub fn take_angle_needs(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.angle_needs)
    }

    /// Asks the next opening for the view's census: [`QueueStats::view_waiting`],
    /// [`QueueStats::weight_census`], [`QueueStats::cone_only`], [`QueueStats::view_median`]
    /// and [`QueueStats::view_reach`] keep the last census's values until then. A caller that
    /// shows them once a second asks once a second; [`BakeQueue::census_every_opening`] reads
    /// them at every opening.
    pub fn request_census(&mut self) {
        self.census_due = true;
    }

    /// Where `c` lies against `view` from the camera's chunk `eye`: [`VIEW_SHELL`],
    /// [`VIEW_IN`] or [`VIEW_OUT`].
    fn view_class(&self, c: [i32; 3], eye: [i32; 3], view: &QueueView<'_>) -> usize {
        let o = [0, 1, 2].map(|k| c[k] - eye[k]);
        if in_near_shell(o, self.near) {
            VIEW_SHELL
        } else if angle_term(o, view.forward, view.half_angle, self.near) <= 0.0 {
            VIEW_IN
        } else {
            VIEW_OUT
        }
    }

    /// The chunks of the jobs open and then pending, each part in its order: the order the
    /// queue opened them in, but where a dig was promoted ahead.
    pub fn jobs(&self) -> Vec<ChunkCoord> {
        let open = self.lp.open.iter().map(|j| j.coord);
        open.chain(self.lp.pending.iter().map(|j| j.coord))
            .collect()
    }

    /// The names of the per-chunk sets that hold `c`: what a chunk leaving the mirror must not
    /// leave behind.
    pub fn tracks(&self, c: ChunkCoord) -> Vec<&'static str> {
        let c = c.0;
        [
            ("fades", self.fades.contains_key(&c)),
            ("digs", self.digs.contains_key(&c)),
            ("dig_open", self.dig_open.contains_key(&c)),
            ("reach", self.reach.contains(&c)),
            ("listed", self.listed.contains_key(&c)),
            ("faceless", self.faceless.contains(&c)),
            ("blocked", self.blocked.contains_key(&c)),
            ("job_coarse", self.job_coarse.contains_key(&c)),
            ("stale", self.stale.contains(&c)),
            ("partial", self.partial.contains(&c)),
            ("job_partial", self.job_partial.contains(&c)),
            ("partial_due", self.partial_due.contains(&c)),
        ]
        .into_iter()
        .filter_map(|(name, held)| held.then_some(name))
        .collect()
    }

    /// No job pending or open, no block fading, and no candidate at the last count, which ran
    /// with no job in the loop.
    pub fn empty(&self) -> bool {
        self.lp.idle()
            && self.fades.is_empty()
            && self.counted_idle
            && self.stats.depth == 0
            && self.boxes.is_empty()
            && self.digs.is_empty()
            && self.reach.is_empty()
            && self.partial_due.is_empty()
    }

    /// Whether an edit box waits for the next step.
    pub fn boxes_waiting(&self) -> bool {
        !self.boxes.is_empty()
    }

    /// No edit box, dig brick or ring-3 chunk waits, and no dig job runs.
    pub fn digs_settled(&self) -> bool {
        self.boxes.is_empty()
            && self.digs.is_empty()
            && self.dig_open.is_empty()
            && self.reach.is_empty()
    }

    /// Whether `c` holds its block at the full count or is known to have none: no face, or
    /// (`absent`) no chunk.
    pub fn settled(&self, store: &Store, c: ChunkCoord, absent: bool) -> bool {
        ((store.blocks.contains_key(&c.0) && !store.is_coarse(c))
            || self.faceless.contains(&c.0)
            || absent)
            && !self.lp.chunks().any(|j| j == c)
            && !self.digs.contains_key(&c.0)
            && !self.reach.contains(&c.0)
            && !self.partial_due.contains(&c.0)
    }

    /// Whether `c` has had its first pass: it holds a block (coarse or full), is known to have
    /// no face, or its reach or lights were not ready at its last check.
    pub fn passed(&self, store: &Store, c: ChunkCoord) -> bool {
        store.blocks.contains_key(&c.0)
            || self.faceless.contains(&c.0)
            || self.blocked.contains_key(&c.0)
    }

    /// [`face_classes`] of `materials`, made again only when a row's translucency changed.
    fn classes(&mut self, materials: &MaterialTable) -> Arc<Vec<u8>> {
        let rows = materials.rows();
        let same = self.classes.as_ref().is_some_and(|(k, _)| {
            k.len() == rows.len() && k.iter().zip(rows).all(|(t, m)| *t == m.is_translucent())
        });
        if !same {
            let key = rows.iter().map(|m| m.is_translucent()).collect();
            self.classes = Some((key, Arc::new(face_classes(materials))));
        }
        self.classes.as_ref().expect("made above").1.clone()
    }

    /// Takes the stream workers' results; one made under other classes than `materials`' is
    /// dropped.
    pub fn take_prepared(
        &mut self,
        prepared: &[(ChunkCoord, vxe_world::Prepared)],
        materials: &MaterialTable,
    ) {
        let classes = self.classes(materials);
        let classes = classes.as_slice();
        for (c, p) in prepared {
            let Ok(m) = p.0.clone().downcast::<PreparedMasks>() else {
                continue;
            };
            self.stats.hook_chunks += 1;
            self.stats.hook_ns += m.ns;
            self.stats.hook_max_ns = self.stats.hook_max_ns.max(m.ns);
            if m.classes.as_slice() != classes {
                continue;
            }
            self.prepared_bytes += m.masks.bytes();
            if let Some(old) = self.prepared.insert(c.0, m) {
                self.prepared_bytes -= old.masks.bytes();
            }
            self.faceless.remove(&c.0);
            self.touched.insert(c.0);
            self.blocked.remove(&c.0);
        }
        if self.prepared_bytes > PREPARED_BYTES_MAX {
            // The farthest from the last step's eye go first, to nine tenths of the cap.
            let eye = self.eye;
            let mut by: Vec<(i64, [i32; 3])> = self
                .prepared
                .keys()
                .map(|c| {
                    let d: i64 = (0..3).map(|k| i64::from(c[k] - eye[k]).pow(2)).sum();
                    (d, *c)
                })
                .collect();
            by.sort_unstable();
            while self.prepared_bytes > PREPARED_BYTES_MAX / 10 * 9 {
                let Some((_, c)) = by.pop() else { break };
                if let Some(m) = self.prepared.remove(&c) {
                    self.prepared_bytes -= m.masks.bytes();
                }
            }
        }
    }

    /// An edit's box (`min ..= max`, voxels): the chunks within a voxel of it lose their prepared
    /// masks and their known facelessness, and a running job's block for them is thrown away; the
    /// box's rings are read at the next step.
    pub fn invalidate(&mut self, min: [i32; 3], max: [i32; 3]) {
        self.boxes.push([min, max]);
        let lo = min.map(|v| (v - 1).div_euclid(64));
        let hi = max.map(|v| (v + 1).div_euclid(64));
        for z in lo[2]..=hi[2] {
            for y in lo[1]..=hi[1] {
                for x in lo[0]..=hi[0] {
                    let c = [x, y, z];
                    if let Some(m) = self.prepared.remove(&c) {
                        self.prepared_bytes -= m.masks.bytes();
                    }
                    self.faceless.remove(&c);
                    self.touched.insert(c);
                    self.blocked.remove(&c);
                    if self.lp.chunks().any(|j| j.0 == c) {
                        self.stale.insert(c);
                    }
                }
            }
        }
    }

    /// One frame's step: follow the window, free the blocks of chunks the mirror dropped, open
    /// jobs while the open ones hold less than two slices, one submit within the slice, and the
    /// fades a frame on. `None` from the submit when nothing was dispatched. `t0` is when the
    /// frame's call began: the time to here is the step's setup stage ([`StepStages`]).
    pub(crate) fn step(
        &mut self,
        run: &BakeRun<'_>,
        store: &mut Store,
        view: &QueueView<'_>,
        t0: Instant,
    ) -> Result<QueueStats, BakeError> {
        self.laps = Laps::new(t0);
        // Summary time spent outside a step (a one-shot bake, a dig's ring) is not this step's.
        store.take_summary_ms();
        self.laps.lap(S_SETUP);
        self.steps += 1;
        let eye = view.eye.map(|v| (v / 64.0).floor() as i32);
        if eye != self.eye {
            self.eye_moves += 1;
        }
        self.eye = eye;
        let gpu = run.gpu;
        self.stats.scanned = 0;
        self.stats.reach_checks = 0;
        self.stats.rebuilt = 0;
        let window = gpu.window();
        let dropped = store.rewindow(run.queue, window);
        self.touched.extend(dropped);
        self.laps.lap(S_REWINDOW);
        let seen = (Some(window), gpu.mirrored_chunks());
        let moved = seen != self.mirror_seen;
        self.mirror_seen = seen;
        store.piece = Some(window);
        // The mirror's changes since the last step; untracked (the first step), everything is
        // read again.
        // The list repeats a chunk once per event (queued, uploaded, dropped); everything below
        // reads each changed chunk once.
        let changes = gpu.take_changes().map(|mut ch| {
            self.stats.scanned += ch.len() as u64;
            let mut seen = ChunkSet::with_capacity_and_hasher(ch.len(), Default::default());
            ch.retain(|c| seen.insert(c.0));
            ch
        });
        let unheld = |c: &[i32; 3]| !gpu.is_mirrored(ChunkCoord(*c)) || gpu.is_lod(ChunkCoord(*c));
        // A chunk the mirror does not hold full (evicted, or LOD-only) frees its block. Every
        // such chunk is among the mirror's changes since the step that last read them.
        // In coordinate order, so the freed places return to the free list in the same order
        // run to run.
        let gone: Vec<[i32; 3]> = match &changes {
            Some(ch) => {
                let mut g: Vec<[i32; 3]> = ch
                    .iter()
                    .map(|c| c.0)
                    .filter(|c| store.blocks.contains_key(c) && unheld(c))
                    .collect();
                g.sort_unstable();
                g
            }
            None => {
                self.stats.scanned += store.blocks.len() as u64;
                store.blocks.keys().copied().filter(unheld).collect()
            }
        };
        if moved || !gone.is_empty() {
            // The mirror moved, gained or lost chunks: the depth is counted again this step.
            self.counted_idle = false;
            self.quiet_until = self.steps;
        }
        for c in gone {
            store.remove(run.queue, ChunkCoord(c));
        }
        self.laps.lap(S_GONE);
        // What the queue knows of a chunk the mirror does not hold full goes with it, block or
        // none (a faceless or blocked chunk has none): the chunk is listed afresh if it returns.
        // A job in flight for it ends in the sink, which drops its block.
        let held = |c: &[i32; 3]| gpu.is_mirrored(ChunkCoord(*c)) && !gpu.is_lod(ChunkCoord(*c));
        // The sets hold only held chunks, and only a changed chunk can have stopped being held,
        // so removing the changed unheld chunks and keeping the held entries leave the same sets.
        // A key costs two lookups and six removals, an entry two lookups: the cheaper goes. In a
        // flight the changes a step outnumber the sets' entries.
        let entries = self.fades.len()
            + self.faceless.len()
            + self.blocked.len()
            + self.digs.len()
            + self.reach.len()
            + self.listed.len()
            + self.partial.len()
            + self.partial_due.len();
        let by_keys = changes.as_ref().is_some_and(|ch| 4 * ch.len() < entries);
        if !by_keys {
            self.fades.retain(|c, _| held(c));
            self.faceless.retain(held);
            self.blocked.retain(|c, _| held(c));
            self.digs.retain(|c, _| held(c));
            self.reach.retain(held);
            self.listed.retain(|c, _| held(c));
            self.partial.retain(held);
            self.partial_due.retain(held);
        }
        match changes {
            Some(ch) => {
                if by_keys {
                    for c in ch.iter().map(|c| c.0).filter(|c| !held(c)) {
                        self.fades.remove(&c);
                        self.faceless.remove(&c);
                        self.blocked.remove(&c);
                        self.digs.remove(&c);
                        self.reach.remove(&c);
                        self.listed.remove(&c);
                        self.partial.remove(&c);
                        self.partial_due.remove(&c);
                    }
                }
                self.laps.lap(S_FORGET);
                self.drop_reach(&ch, gpu.light_radius());
                self.laps.lap(S_DROP_REACH);
                self.touched.extend(ch.iter().map(|c| c.0));
                self.requeue_partial(gpu, Some(ch.as_slice()));
            }
            None => {
                self.laps.lap(S_FORGET);
                self.reach_cache.clear();
                self.laps.lap(S_DROP_REACH);
                self.pool_stale = true;
                self.requeue_partial(gpu, None);
            }
        }
        self.laps.lap(S_TOUCHED);
        self.take_boxes(run, store)?;
        self.laps.lap(S_BOXES);
        let dig = !self.digs.is_empty() || !self.dig_open.is_empty();
        let cap = view.samples.unwrap_or_else(|| {
            let slice = self.budget.slice_ms(view.interval_s);
            self.stats.slice_ms = slice;
            let ms = slice + if dig { self.edit_ms } else { 0.0 };
            ((ms * 1e6 / self.stats.ns_per_sample.max(0.1)) as u64).clamp(16_384, SUBMIT_SAMPLES)
        });
        self.withdraw_passed(run, store, view);
        self.laps.lap(S_WITHDRAW);
        self.open_digs(run, store, cap)?;
        self.laps.lap(S_DIGS);
        let mut opened_step = false;
        if self.lp.samples_left() < 2 * cap && self.steps >= self.quiet_until {
            opened_step = true;
            self.stats.openings += 1;
            let opened = self.lp.chunks().count();
            self.open(run, store, view, cap)?;
            if self.lp.chunks().count() == opened && self.lp.idle() {
                // Nothing was ready: the candidates are sorted again a few frames on.
                self.quiet_until = self.steps + QUEUE_QUIET;
            }
            self.laps.lap(S_LIST);
        }
        self.lp.refresh_tables(run);
        let bind = run.pipeline.bind();
        run.queue
            .write_buffer(run.params_buffer, 0, bytemuck::bytes_of(&run.params));
        self.laps.lap(S_REFRESH);
        // Nothing waits on the GPU in a sliced step: the submits read back are those whose
        // results are mapped by now, and a new one goes out while fewer than IN_FLIGHT are
        // unread. A step without a slice (the pre-roll) waits for its oldest submit once
        // IN_FLIGHT are out, so the CPU lists and plans while the GPU draws.
        let unread = if view.samples.is_some() {
            IN_FLIGHT - 1
        } else {
            IN_FLIGHT
        };
        let _ = run.device.poll(wgpu::PollType::Poll);
        self.laps.lap(S_POLL);
        // The sink's own time, the blocks' placement, taken out of the collect's and the
        // submit's laps.
        let place_ms = std::cell::Cell::new(0.0f64);
        let queue = run.queue;
        let stale = &mut self.stale;
        let fades = &mut self.fades;
        let stats = &mut self.stats;
        let job_coarse = &mut self.job_coarse;
        let job_eye = &mut self.job_eye;
        let dig_open = &mut self.dig_open;
        let reach = &mut self.reach;
        let partial = &mut self.partial;
        let job_partial = &mut self.job_partial;
        let entered = &mut self.entered;
        let ages_first = &mut self.ages_first;
        let ages_full = &mut self.ages_full;
        let steps = self.steps;
        let record = record_words(self.settings.cell);
        let mut place = |store: &mut Store, coord: ChunkCoord, first: u64, block: Vec<u32>| {
            let coarse = job_coarse.remove(&coord.0).unwrap_or(false);
            job_eye.remove(&coord.0);
            let from_partial = job_partial.remove(&coord.0);
            let dug = dig_open.remove(&coord.0);
            if stale.remove(&coord.0) || !run.gpu.is_mirrored(coord) {
                store.release(first, block.len() as u64);
                // A dig job's bricks went back to the waiting digs at the edit; a whole chunk's
                // job that would have replaced a full block re-bakes whole.
                if dug.is_none()
                    && run.gpu.is_mirrored(coord)
                    && store.blocks.contains_key(&coord.0)
                    && !store.is_coarse(coord)
                {
                    reach.insert(coord.0);
                }
                return;
            }
            if let Some(e) = entered.get_mut(&coord.0) {
                let age = u32::try_from(steps - e.0).unwrap_or(u32::MAX);
                if !e.1 {
                    e.1 = true;
                    ages_first.push(age);
                }
                if !coarse {
                    ages_full.push(age);
                    entered.remove(&coord.0);
                }
            }
            if let Some(d) = &dug {
                stats.digs_baked += 1;
                stats.dig_bricks += d.slots.len() as u64;
            }
            // A whole chunk's block carries its job's reach; a dig's keeps the rest of the
            // block's, and with it the flag.
            if dug.is_none() {
                if from_partial {
                    partial.insert(coord.0);
                } else {
                    partial.remove(&coord.0);
                }
            }
            let dug = dug.is_some();
            stats.baked += 1;
            // The table word's flag follows at the put below.
            store.mark_coarse(coord, coarse);
            let old = store
                .blocks
                .get(&coord.0)
                .and_then(|&(f, _)| store.words.get(&coord.0).map(|w| (f, w.clone())));
            match old {
                Some((f, from))
                    if from.len() == block.len()
                        && from[..CHUNK_BRICKS] == block[..CHUNK_BRICKS] =>
                {
                    // The same layout: the block takes its old place, shows its old values and
                    // fades to the new ones.
                    store.release(first, block.len() as u64);
                    store.put(queue, coord, f, block);
                    let mut from = from;
                    store.show(queue, coord, f, &mut from);
                    fades.insert(
                        coord.0,
                        Fade {
                            first: f,
                            from,
                            frame: 0,
                            frames: if dug { DIG_FADE } else { FADE },
                        },
                    );
                }
                Some((f, from)) => {
                    store.release(f, from.len() as u64);
                    fades.remove(&coord.0);
                    store.put(queue, coord, first, block);
                }
                None => match store.light.chain(coord.0, store.missing) {
                    // A first block under a fallback mode fades from the chain's value, read
                    // before the block's own summary joins the chain (ADR-0053).
                    Some(chain) => {
                        let mut from = fallback_block(&block, &chain, record);
                        store.put(queue, coord, first, block);
                        store.show(queue, coord, first, &mut from);
                        fades.insert(
                            coord.0,
                            Fade {
                                first,
                                from,
                                frame: 0,
                                frames: FADE,
                            },
                        );
                    }
                    None => store.put(queue, coord, first, block),
                },
            }
        };
        let mut sink = |store: &mut Store, coord: ChunkCoord, first: u64, block: Vec<u32>| {
            let t = stopwatch();
            place(store, coord, first, block);
            place_ms.set(place_ms.get() + seconds_since(t) * 1e3);
        };
        let settings = BakeSettings {
            round: if view.samples.is_some() {
                UNSLICED_ROUND
            } else {
                self.settings.round
            },
            ..self.settings.clone()
        };
        let mut d = self
            .lp
            .collect(run, store, &settings, &mut sink, &mut |_| {}, IN_FLIGHT);
        // Each lap's placement moves from its stage to the placement's.
        let mut placed = 0.0;
        let mut lap = |laps: &mut Laps, stage: usize| {
            laps.lap(stage);
            let p = place_ms.get() - placed;
            placed = place_ms.get();
            // The placement ran inside the lap; the clamp only absorbs rounding.
            let p = p.min(laps.ms[stage]);
            laps.ms[stage] -= p;
            laps.ms[S_PLACE] += p;
        };
        lap(&mut self.laps, S_COLLECT);
        let sent = self.lp.in_flight() < IN_FLIGHT
            && self
                .lp
                .dispatch(run, bind, store, &settings, cap, &mut sink, &mut |_| {});
        lap(&mut self.laps, S_SUBMIT);
        let submitted = if sent { self.lp.last_samples } else { 0 };
        if unread < IN_FLIGHT {
            // With nothing left to send, the pre-roll waits out what is in flight.
            let left = if sent { unread } else { 0 };
            let w = self
                .lp
                .collect(run, store, &settings, &mut sink, &mut |_| {}, left);
            d.samples += w.samples;
            d.seconds += w.seconds;
            if let Some(ms) = w.gpu_ms {
                *d.gpu_ms.get_or_insert(0.0) += ms;
            }
            lap(&mut self.laps, S_COLLECT);
        }
        // The slice is sized by the GPU's own time for the submits read back, a frame or two
        // late; without timestamps it stays at the first guess, a fixed count.
        self.stats.cap = cap;
        self.stats.submitted = submitted;
        self.stats.samples = d.samples;
        self.stats.ms = d.gpu_ms.unwrap_or(d.seconds * 1e3);
        self.stats.total_samples += d.samples;
        if let Some(ms) = d.gpu_ms
            && d.samples >= 4096
        {
            let ns = ms * 1e6 / d.samples as f64;
            self.stats.ns_per_sample = 0.8 * self.stats.ns_per_sample + 0.2 * ns;
        }
        // The ended jobs' samples by where their chunk lies now.
        let ended = self
            .lp
            .ended
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default();
        for (c, drawn) in ended {
            let class = self.view_class(c.0, self.eye, view);
            self.stats.view_samples[class] += drawn;
            let centre = [0, 1, 2].map(|k| (c.0[k] * 64 + 32) as f32 - view.eye[k]);
            let cos: f32 = (0..3).map(|k| centre[k] * view.forward[k]).sum();
            if class == VIEW_OUT && cos < 0.0 {
                self.stats.passed_samples += drawn;
            }
        }
        self.advance_fades(run.queue, store, record);
        self.stats.jobs = self.lp.chunks().count() as u64;
        self.stats.fading = self.fades.len() as u64;
        self.stats.coarse = store.coarse.len() as u64;
        self.stats.digs = (self.digs.len() + self.dig_open.len()) as u64;
        self.stats.reach = self.reach.len() as u64;
        self.stats.partial = self.partial.len() as u64;
        self.laps.lap(S_FADES);
        // The summaries' and parents' time ran inside the placement, the fades, the evictions
        // and the window's move: it moves to its own stage, from those in that order.
        let mut summaries = store.take_summary_ms();
        for stage in [S_PLACE, S_FADES, S_GONE, S_REWINDOW] {
            let p = summaries.min(self.laps.ms[stage]);
            self.laps.ms[stage] -= p;
            self.laps.ms[S_SUMMARIES] += p;
            summaries -= p;
        }
        self.stats.stages = StepStages {
            ms: self.laps.ms,
            total_ms: stopwatch().duration_since(t0).as_secs_f64() * 1e3,
            opened: opened_step,
        };
        Ok(self.stats)
    }

    /// Reads the edit boxes since the last step into the waiting digs, against the store as the
    /// edits left it (no re-bake has run since): each ring chunk with a block takes its ring's
    /// bricks, merged with what a dig job running there re-bakes (that job is thrown away), and a
    /// job of any kind running in a ring chunk is thrown away. A touched brick that held light
    /// opens ring 3. Each chunk holding a touched brick is listed again at once and takes a
    /// provisional block ([`provisional_block`]): its kept bricks' records, and a brick the edit
    /// opened its neighbours' mean, so no dug face reads no value while its job waits; the job
    /// then fades from it.
    fn take_boxes(&mut self, run: &BakeRun<'_>, store: &mut Store) -> Result<(), BakeError> {
        if self.boxes.is_empty() {
            return Ok(());
        }
        let boxes = std::mem::take(&mut self.boxes);
        let Some(piece) = store.piece else {
            return Ok(());
        };
        let record = self.record_words();
        let touches = |c: [i32; 3], slot: u16| {
            let b = brick_of_slot(usize::from(slot));
            boxes.iter().any(|[min, max]| {
                (0..3).all(|k| {
                    let lo = c[k] * 64 + b[k] * 8;
                    lo + 7 >= min[k] - 1 && lo <= max[k] + 1
                })
            })
        };
        let in_job: std::collections::HashSet<[i32; 3]> = self.lp.chunks().map(|c| c.0).collect();
        let mut lit = false;
        let mut dug = Vec::new();
        for (c, slots) in dig_ring(&boxes, piece) {
            let near: Vec<u16> = slots.iter().copied().filter(|&s| touches(c, s)).collect();
            if !lit && let Some(block) = store.words.get(&c) {
                lit = near.iter().any(|&s| brick_lit(block, s, record));
            }
            if in_job.contains(&c) {
                self.stale.insert(c);
            }
            if !store.blocks.contains_key(&c) {
                continue;
            }
            let e = self.digs.entry(c).or_default();
            self.touched.insert(c);
            if let Some(open) = self.dig_open.get(&c) {
                e.slots.extend(open.slots.iter().copied());
                e.near |= open.near;
            }
            e.slots.extend(slots);
            e.near |= !near.is_empty();
            if !near.is_empty() {
                dug.push((c, near));
            }
        }
        let classes = self.classes(run.world.materials());
        // A `B` press touches tens of chunks: listed across threads, as the pre-roll's are.
        let threads = std::thread::available_parallelism()
            .map_or(4, std::num::NonZero::get)
            .clamp(1, 16);
        let per = dug.len().div_ceil(threads).max(1);
        let this = &*self;
        let listed: Vec<ChunkFaces> = std::thread::scope(|scope| {
            let handles: Vec<_> = dug
                .chunks(per)
                .map(|part| {
                    let classes = &classes;
                    scope.spawn(move || {
                        part.iter()
                            .map(|(c, _)| this.list(run.world, ChunkCoord(*c), classes).0)
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|h| h.join().expect("a listing thread panicked"))
                .collect()
        });
        for ((c, near), faces) in dug.into_iter().zip(listed) {
            let coord = ChunkCoord(c);
            if let Some(m) = self.prepared.remove(&c) {
                self.prepared_bytes -= m.masks.bytes();
            }
            let Some(&(first, words)) = store.blocks.get(&c) else {
                continue;
            };
            if faces.faces() == 0 {
                self.fades.remove(&c);
                store.remove(run.queue, coord);
                self.faceless.insert(c);
                self.touched.insert(c);
                self.digs.remove(&c);
                continue;
            }
            let block = provisional_block(store, c, &faces, record, &near);
            let len = block.len() as u64;
            // The old place is freed only once the new one is held: a refused place leaves the
            // old block where the chunk table points, and its dig waits in `digs`.
            let at = if len <= words {
                store.release(first + len, words - len);
                first
            } else {
                match store.alloc(len) {
                    Ok(at) => {
                        store.release(first, words);
                        at
                    }
                    Err(BakeError::PoolFull { .. }) => {
                        self.stats.pool_full += 1;
                        continue;
                    }
                    Err(e) => return Err(e),
                }
            };
            self.fades.remove(&c);
            store.put(run.queue, coord, at, block);
            self.listed.insert(c, faces);
        }
        if !(lit && self.reach_ring) {
            return Ok(());
        }
        let lr = run.gpu.light_radius();
        for [min, max] in &boxes {
            let lo = min.map(|v| (v - 1).div_euclid(64));
            let hi = max.map(|v| (v + 1).div_euclid(64));
            for z in lo[2] - lr..=hi[2] + lr {
                for y in lo[1] - lr..=hi[1] + lr {
                    for x in lo[0] - lr..=hi[0] + lr {
                        let n = [x, y, z];
                        let gap2: i64 = (0..3)
                            .map(|k| i64::from((lo[k] - n[k]).max(n[k] - hi[k]).max(0)).pow(2))
                            .sum();
                        if gap2 <= i64::from(lr).pow(2) && store.blocks.contains_key(&n) {
                            self.reach.insert(n);
                            self.touched.insert(n);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Opens jobs for the waiting dig chunks not in a job, ahead of every pending job, while the
    /// loop holds under two slices of `cap` samples (one at least): ring-1 chunks first, then by
    /// distance from the camera's chunk. Each is listed again on this thread and keeps every
    /// brick record outside its dig bricks; a chunk left with no face drops its block.
    fn open_digs(
        &mut self,
        run: &BakeRun<'_>,
        store: &mut Store,
        cap: u64,
    ) -> Result<(), BakeError> {
        if self.digs.is_empty() {
            return Ok(());
        }
        let eye = self.eye;
        let in_job: std::collections::HashSet<[i32; 3]> = self.lp.chunks().map(|c| c.0).collect();
        let mut order: Vec<(bool, i64, [i32; 3])> = self
            .digs
            .iter()
            .filter(|(c, _)| !in_job.contains(*c))
            .map(|(c, d)| {
                let d2: i64 = (0..3).map(|k| i64::from(c[k] - eye[k]).pow(2)).sum();
                (!d.near, d2, *c)
            })
            .collect();
        order.sort_unstable();
        // The dig jobs already pending or open count against the two slices; the rest of the
        // loop does not, so a dig never waits behind the queue's own depth.
        let mut left: u64 = self.lp.dig_samples_left(&self.dig_open);
        let classes = self.classes(run.world.materials());
        let record = self.record_words();
        let brick_samples = self.settings.brick_samples;
        let mut jobs = Vec::new();
        for (_, _, c) in order {
            if left >= 2 * cap && !jobs.is_empty() {
                break;
            }
            self.touched.insert(c);
            let Some(d) = self.digs.remove(&c) else {
                continue;
            };
            let coord = ChunkCoord(c);
            let Some(block) = store.words.get(&c) else {
                continue;
            };
            let kept: KeptBricks = records_of(block, record)
                .into_iter()
                .filter(|(slot, _)| !d.slots.contains(slot))
                .collect();
            let faces = match self.listed.remove(&c) {
                Some(f) => f,
                None => self.list(run.world, coord, &classes).0,
            };
            if let Some(m) = self.prepared.remove(&c) {
                self.prepared_bytes -= m.masks.bytes();
            }
            if faces.faces() == 0 {
                store.remove(run.queue, coord);
                self.fades.remove(&c);
                self.faceless.insert(c);
                self.touched.insert(c);
                continue;
            }
            let samples = faces
                .bricks
                .iter()
                .filter(|(slot, _)| d.slots.contains(slot))
                .map(|(_, keys)| {
                    let mut dirs = [false; 6];
                    for k in keys {
                        dirs[usize::from(k & 7)] = true;
                    }
                    dirs.iter().filter(|d| **d).count() as u64 * u64::from(brick_samples)
                })
                .sum();
            let first_word = match store.alloc(layout(&faces, record).1) {
                Ok(first) => first,
                Err(BakeError::PoolFull { .. }) => {
                    // The dig waits for a place, listed: no later dig opens ahead of it.
                    self.stats.pool_full += 1;
                    self.digs.insert(c, d);
                    self.touched.insert(c);
                    self.listed.insert(c, faces);
                    break;
                }
                Err(e) => return Err(e),
            };
            self.job_coarse.insert(c, store.is_coarse(coord));
            left += samples;
            jobs.push(Job {
                coord,
                faces,
                first_word,
                kept_bricks: kept,
                samples,
                brick_samples: None,
            });
            self.dig_open.insert(c, d);
        }
        // Pushed at the front in reverse, so the pending jobs start in `order`.
        for job in jobs.into_iter().rev() {
            self.lp.push_front(job);
        }
        let dig_open = &self.dig_open;
        self.lp.promote(|c| dig_open.contains_key(&c.0));
        Ok(())
    }

    /// A brick record's words under the settings' cell.
    fn record_words(&self) -> usize {
        record_words(self.settings.cell)
    }

    /// Moves every fade a frame on: each record's direction values at
    /// `frame / FADE` of the way from what it showed to the store's copy; the last frame writes
    /// the copy itself.
    fn advance_fades(&mut self, queue: &wgpu::Queue, store: &mut Store, record: usize) {
        let mut ended = Vec::new();
        let mut shown = std::mem::take(&mut self.fade_scratch);
        for (c, f) in &mut self.fades {
            f.frame += 1;
            let Some(to) = store.words.get(c) else {
                ended.push(*c);
                continue;
            };
            // A dig re-bakes outside the queue's sink and can relist the chunk: a block at
            // another place or with another directory is not the one this fade started on, and
            // its old place may already hold another chunk's block. The dig's put showed the
            // new block, so the fade ends without a write.
            let same = store
                .blocks
                .get(c)
                .is_some_and(|&(first, _)| first == f.first)
                && to.len() == f.from.len()
                && to[..CHUNK_BRICKS] == f.from[..CHUNK_BRICKS];
            if !same {
                ended.push(*c);
                continue;
            }
            if f.frame >= f.frames {
                shown.clear();
                shown.extend_from_slice(to);
                store.show(queue, ChunkCoord(*c), f.first, &mut shown);
                ended.push(*c);
                continue;
            }
            let t = f.frame as f32 / f.frames as f32;
            let lerp = |a: u32, b: u32| {
                let (x, y) = (rgb9e5_unpack(a), rgb9e5_unpack(b));
                rgb9e5_pack([0, 1, 2].map(|k| x[k] + (y[k] - x[k]) * t))
            };
            shown.clear();
            shown.extend_from_slice(to);
            for slot in 0..CHUNK_BRICKS {
                let dir = to[slot];
                if dir == 0 {
                    continue;
                }
                let run = (dir >> 11) as usize;
                for cell in (run - record..run).step_by(BRICK_WORDS) {
                    for w in cell..cell + 6 {
                        shown[w] = lerp(f.from[w], to[w]);
                    }
                }
            }
            store.show(queue, ChunkCoord(*c), f.first, &mut shown);
        }
        self.fade_scratch = shown;
        for c in ended {
            self.fades.remove(&c);
        }
    }

    /// Opens jobs in [`BakeQueue::order`] while the open ones hold under two slices of `cap`
    /// samples: each a ready candidate's faces at the brick level.
    fn open(
        &mut self,
        run: &BakeRun<'_>,
        store: &mut Store,
        view: &QueueView<'_>,
        cap: u64,
    ) -> Result<(), BakeError> {
        let gpu = run.gpu;
        let eye = view.eye.map(|v| (v / 64.0).floor() as i32);
        let order = self.order;
        let near = self.near;
        self.sync_pool(run, store);
        // The kept order is scored again when a candidate joined or changed class, or the view
        // the scores read changed; else it is walked as it stands.
        let key = OrderView {
            eye,
            forward: view.forward.map(f32::to_bits),
            half_angle: view.half_angle.to_bits(),
            order,
            near,
        };
        if self.order_dirty || self.order_view != Some(key) {
            let cands: Vec<Cand> = self
                .pool
                .iter()
                .map(|(&c, &(class, coarse))| cand_of(c, class, coarse, eye, view, order, near))
                .collect();
            self.ordered = Ordered::new(cands);
            self.order_dirty = false;
            self.order_view = Some(key);
            self.stats.rebuilt += 1;
        }
        self.laps.lap(S_CANDIDATES);
        let census = self.census_every_opening || std::mem::take(&mut self.census_due);
        let steps = self.steps;
        let recent = |at: Option<&u64>| at.is_some_and(|&at| steps - at < QUEUE_RECHECK);
        if census {
            let t = stopwatch();
            self.entered
                .retain(|c, _| gpu.is_mirrored(ChunkCoord(*c)) && !gpu.is_lod(ChunkCoord(*c)));
            let cands: Vec<Cand> = self
                .ordered
                .all()
                .filter(|k| self.pool.contains_key(&k.c))
                .copied()
                .collect();
            self.view_census(eye, view, &cands, gpu.light_radius());
            self.view_reach_census(run, eye, view, &cands);
            self.stats.unbaked = cands
                .iter()
                .filter(|k| k.class == 0 && !recent(self.blocked.get(&k.c)))
                .count() as u64;
            self.stats.census_ms = seconds_since(t) * 1e3;
        }
        self.laps.lap(S_CENSUS);
        self.stats.depth = self.pool.len() as u64;
        self.counted_idle = self.lp.chunks().next().is_none();
        self.laps.lap(S_SORT);
        let checks = if view.samples.is_some() {
            QUEUE_CHECKS_UNSLICED
        } else {
            QUEUE_CHECKS
        };
        let classes = self.classes(run.world.materials());
        let lr = gpu.light_radius();
        // Tiered (the distance order, the score's tier): class 1 (refinement) is opened only by
        // an opening that found no class 0 candidate ready, so every chunk with no block that
        // can bake comes first; those whose reach or lights are not ready (a streamed world's
        // edge) cannot starve the refinement behind them, each class counting its own checks.
        // Under the score's refine weight the class is in the key: one count of checks runs
        // over the sorted list, and a candidate found blocked is skipped for QUEUE_RECHECK
        // steps, so the next openings check the candidates behind it.
        let tiered = !matches!(
            order,
            QueueOrder::Score {
                refine: Refine::Weight(_),
                ..
            }
        );
        let (mut checked, mut reach, mut lights) = ([0usize; 2], 0u64, 0u64);
        let mut ready0 = 0usize;
        // A step without a slice (the pre-roll) gates up to `checks` candidates and lists them
        // across threads; a sliced step lists them in batches and stops at two slices of
        // samples.
        let parallel = view.samples.is_some();
        let mut ready: Vec<(u8, [i32; 3])> = Vec::new();
        // The loop's samples left, read once (it walks every open face) and then counted on.
        let mut left = self.lp.samples_left();
        let angle_needs = self.census_every_opening;
        let nearest_in_view = if angle_needs {
            self.ordered
                .all()
                .filter(|k| k.a <= 0.0 && self.pool.contains_key(&k.c))
                .map(|k| k.d2)
                .min()
        } else {
            None
        };
        let mut next = 0;
        // A sliced opening lists its ready candidates in batches: a candidate joins the batch
        // while the loop's samples left plus the batch's bounds ([`BakeQueue::listing_bound`],
        // at least its listing's samples) stay under two slices, the batch is listed across
        // threads and pushed in the walk's order, and the walk goes on from the samples the
        // pushes left. Every candidate the one-at-a-time walk would push is pushed, in its order.
        let mut batch: Vec<(u8, [i32; 3])> = Vec::new();
        let mut est = left;
        let mut full_blank = false;
        let stops = |left: u64, checked: &[usize; 2], class: u8, ready0: usize| {
            if tiered {
                left >= 2 * cap || checked[1] >= checks || (class == 1 && ready0 > 0)
            } else {
                left >= 2 * cap || checked[0] >= checks
            }
        };
        while let Some(Cand {
            class, coarse, c, ..
        }) = self.ordered.get(next)
        {
            next += 1;
            // A kept candidate pushed, or listed faceless, since the order was scored has left
            // the pool; a class change re-scores the order before the walk.
            if self.pool.get(&c) != Some(&(class, coarse)) {
                continue;
            }
            if stops(est, &checked, class, ready0) {
                if batch.is_empty() {
                    next -= 1;
                    break;
                }
                self.laps.lap(S_CHECKS);
                let pushed = self.push_batch(run, store, eye, &classes, &mut batch)?;
                self.laps.lap(S_LIST);
                left += pushed.samples;
                ready0 -= pushed.refused0;
                full_blank |= pushed.full_blank;
                est = left;
                if stops(est, &checked, class, ready0) {
                    next -= 1;
                    break;
                }
            }
            if tiered {
                if checked[usize::from(class)] >= checks || recent(self.blocked.get(&c)) {
                    continue;
                }
                checked[usize::from(class)] += 1;
            } else {
                if recent(self.blocked.get(&c)) {
                    continue;
                }
                checked[0] += 1;
            }
            let coord = ChunkCoord(c);
            let Some(partial) = self.bakeable(run, coord, lr, view, eye) else {
                reach += 1;
                self.blocked.insert(c, self.steps);
                continue;
            };
            if !gpu.lights_ready_near(coord, lr) {
                lights += 1;
                self.blocked.insert(c, self.steps);
                continue;
            }
            self.blocked.remove(&c);
            if partial {
                self.partial_checked.insert(c);
            } else {
                self.partial_checked.remove(&c);
            }
            if class == 0 {
                ready0 += 1;
            }
            if angle_needs {
                self.note_angle_need(c, eye, view, nearest_in_view);
            }
            if parallel {
                ready.push((class, c));
                continue;
            }
            est += self.listing_bound(c, class, eye);
            batch.push((class, c));
        }
        self.laps.lap(S_CHECKS);
        if !batch.is_empty() {
            let pushed = self.push_batch(run, store, eye, &classes, &mut batch)?;
            full_blank |= pushed.full_blank;
            self.laps.lap(S_LIST);
        }
        if !ready.is_empty() {
            self.push_batch(run, store, eye, &classes, &mut ready)?;
        }
        if full_blank && !parallel {
            self.note_starved(run, view, eye, next, checks);
        }
        self.stats.waiting_reach = reach;
        self.stats.waiting_lights = lights;
        // A checked candidate not listed (a refused place) is checked again before its listing.
        self.partial_checked.clear();
        Ok(())
    }

    /// Once in [`VIEW_REACH_EVERY`] steps, at an opening: up to [`VIEW_REACH_SAMPLE`] of the
    /// candidates in the view cone beyond the shell, spread evenly by distance, checked for
    /// their reach ([`QueueStats::view_reach`]).
    fn view_reach_census(
        &mut self,
        run: &BakeRun<'_>,
        eye: [i32; 3],
        view: &QueueView<'_>,
        cands: &[Cand],
    ) {
        if self.steps < self.reach_census_at + VIEW_REACH_EVERY && self.reach_census_at > 0 {
            return;
        }
        self.reach_census_at = self.steps;
        let mut in_view: Vec<(i64, [i32; 3])> = cands
            .iter()
            .filter(|k| {
                let o = [0, 1, 2].map(|i| k.c[i] - eye[i]);
                k.a <= 0.0 && !in_near_shell(o, self.near)
            })
            .map(|k| (k.d2, k.c))
            .collect();
        in_view.sort_unstable();
        let n = in_view.len();
        let take = n.min(VIEW_REACH_SAMPLE);
        let lr = run.gpu.light_radius();
        let mut not_ready = 0;
        for i in 0..take {
            let (_, c) = in_view[i * n / take.max(1)];
            not_ready += u64::from(!reach_ready(run, ChunkCoord(c), lr, view.resident));
        }
        self.stats.view_reach = [take as u64, not_ready];
    }

    /// Withdraws the passed jobs ([`BakeQueue::withdraw`]) when a candidate in the near shell or
    /// the view cone is ready: up to the opening's count of checks of those candidates, nearest
    /// first, each not checked in the last [`QUEUE_RECHECK`] steps. A step with a sample count
    /// (the pre-roll) withdraws nothing: the camera has passed nothing yet, and the coarse pass
    /// opens chunks in every direction, so withdrawing there takes back most of what it opens.
    fn withdraw_passed(&mut self, run: &BakeRun<'_>, store: &mut Store, view: &QueueView<'_>) {
        if !self.withdraw || view.samples.is_some() {
            return;
        }
        let (eye, near, moves) = (self.eye, self.near, self.eye_moves);
        let keep = view.half_angle + QUEUE_VIEW_MARGIN_DEG.to_radians();
        let job_eye = &self.job_eye;
        let passed: std::collections::HashSet<[i32; 3]> = self
            .lp
            .chunks()
            .map(|c| c.0)
            .filter(|c| {
                let o = [0, 1, 2].map(|k| c[k] - eye[k]);
                let full = store.blocks.contains_key(c) && !store.is_coarse(ChunkCoord(*c));
                // Only a job opened before the camera's chunk last changed: the camera has moved
                // since, so it can have passed the chunk.
                job_eye.get(c).is_some_and(|&at| at < moves)
                    && !self.dig_open.contains_key(c)
                    && !full
                    && !in_near_shell(o, near)
                    && angle_term(o, view.forward, keep, near) > 0.0
            })
            .collect();
        if passed.is_empty() {
            return;
        }
        let gpu = run.gpu;
        self.sync_pool(run, store);
        let mut seen: Vec<(i64, [i32; 3])> = self
            .pool
            .keys()
            .filter_map(|c| {
                let o = [0, 1, 2].map(|k| c[k] - eye[k]);
                (angle_term(o, view.forward, view.half_angle, near) <= 0.0).then(|| {
                    let d2: i64 = o.iter().map(|&v| i64::from(v).pow(2)).sum();
                    (d2, *c)
                })
            })
            .collect();
        seen.sort_unstable();
        let steps = self.steps;
        let lr = gpu.light_radius();
        let checks = QUEUE_CHECKS;
        let mut ready = false;
        let mut checked = 0;
        for (_, c) in seen {
            if checked >= checks {
                break;
            }
            if self
                .blocked
                .get(&c)
                .is_some_and(|&at| steps - at < QUEUE_RECHECK)
            {
                continue;
            }
            checked += 1;
            let coord = ChunkCoord(c);
            if self.bakeable(run, coord, lr, view, eye).is_some()
                && gpu.lights_ready_near(coord, lr)
            {
                ready = true;
                break;
            }
            self.blocked.insert(c, steps);
        }
        if !ready {
            return;
        }
        let record = self.record_words();
        for w in self.lp.withdraw(|c| passed.contains(&c.0), record) {
            store.release(w.first_word, w.words);
            self.job_coarse.remove(&w.coord.0);
            self.job_eye.remove(&w.coord.0);
            self.job_partial.remove(&w.coord.0);
            self.stale.remove(&w.coord.0);
            self.stats.withdrawn += 1;
            self.stats.withdrawn_samples += w.drawn;
        }
        // The candidates are counted and sorted again this step.
        self.counted_idle = false;
        self.quiet_until = self.steps;
    }

    /// The view's counters over the candidates of an opening ([`QueueStats::view_waiting`] and
    /// the rest), and the chunks entering the view cone or the near shell with no full block.
    fn view_census(&mut self, eye: [i32; 3], view: &QueueView<'_>, cands: &[Cand], lr: i32) {
        let mut waiting = [[0u64; 2]; 3];
        let mut reach_bound = 0u64;
        let mut census = [[[0u32; 4]; 4]; 2];
        let mut cone_only = 0u64;
        let mut lens: [Vec<f32>; 2] = [Vec::new(), Vec::new()];
        for &Cand {
            d2,
            c,
            a,
            class: has,
            coarse,
            ..
        } in cands
        {
            if has == 1 && !coarse {
                // A full block re-baking whole (ring 3): not waiting for its light.
                continue;
            }
            let o = [0, 1, 2].map(|k| c[k] - eye[k]);
            let class = if in_near_shell(o, self.near) {
                VIEW_SHELL
            } else if a <= 0.0 {
                VIEW_IN
            } else {
                VIEW_OUT
            };
            let len = (d2 as f32).sqrt();
            waiting[class][usize::from(coarse)] += 1;
            census[usize::from(coarse)][distance_band(len)][angle_band(a)] += 1;
            if class == VIEW_IN {
                if view.full_radius.is_some_and(|r| len + lr as f32 > r as f32) {
                    reach_bound += 1;
                }
                lens[usize::from(coarse)].push(len);
                if view
                    .frustum
                    .is_some_and(|f| !in_widened_frustum(o, view.forward, f))
                {
                    cone_only += 1;
                }
            }
            if class != VIEW_OUT && !self.entered.contains_key(&c) {
                self.entered.insert(c, (self.steps, coarse));
                if coarse {
                    self.ages_first.push(0);
                }
            }
        }
        self.stats.view_waiting = waiting;
        self.stats.reach_bound = reach_bound;
        self.stats.weight_census = census;
        self.stats.cone_only = cone_only;
        self.stats.view_median = lens.map(|mut l| {
            l.sort_unstable_by(f32::total_cmp);
            l.get(l.len() / 2).copied().unwrap_or(0.0)
        });
    }

    /// Notes the angle weight at which the nearest candidate in the view cone or the near shell
    /// (squared distance `nearest`) would have outranked `c`, found ready
    /// ([`BakeQueue::take_angle_needs`]).
    fn note_angle_need(
        &mut self,
        c: [i32; 3],
        eye: [i32; 3],
        view: &QueueView<'_>,
        nearest: Option<i64>,
    ) {
        let o = [0, 1, 2].map(|k| c[k] - eye[k]);
        let a = angle_term(o, view.forward, view.half_angle, self.near);
        let len = queue_score(o, 0.0, false, 0.0, 1.0);
        let need = match nearest {
            Some(n) if a > 0.0 && len > 0.0 => ((n as f32).sqrt() / len - 1.0).max(0.0) / a,
            _ => 0.0,
        };
        // Bounded: one second of openings in a pre-roll is a few thousand.
        if self.angle_needs.len() < 1 << 16 {
            self.angle_needs.push(need);
        }
    }

    /// An upper bound on the samples of `c`'s job before it is listed: its brick directions'
    /// bound from its prepared masks ([`ChunkMasks::dirs_bound`]), or six for every brick of
    /// the chunk where none arrived, at the count [`BakeQueue::push_listed`] would give it.
    fn listing_bound(&self, c: [i32; 3], class: u8, eye: [i32; 3]) -> u64 {
        let coarse = class == 0 && self.coarse_samples > 0 && !self.full_first_at(c, eye);
        let brick_samples = if coarse {
            self.coarse_samples
        } else {
            self.settings.brick_samples
        };
        let dirs = self
            .prepared
            .get(&c)
            .map_or(CHUNK_BRICKS as u64 * 6, |m| m.masks.dirs_bound());
        dirs * u64::from(brick_samples)
    }

    /// Lists `batch`'s chunks (across scoped threads when there are several) and pushes their
    /// jobs in the batch's order ([`BakeQueue::push_listed`]); a chunk the pool refuses waits.
    /// Empties `batch`.
    fn push_batch(
        &mut self,
        run: &BakeRun<'_>,
        store: &mut Store,
        eye: [i32; 3],
        classes: &[u8],
        batch: &mut Vec<(u8, [i32; 3])>,
    ) -> Result<Pushed, BakeError> {
        let listed: Vec<(ChunkFaces, bool)> = if batch.len() == 1 {
            vec![self.list(run.world, ChunkCoord(batch[0].1), classes)]
        } else {
            let threads = std::thread::available_parallelism()
                .map_or(4, std::num::NonZero::get)
                .clamp(1, 16);
            let per = batch.len().div_ceil(threads);
            let this = &*self;
            std::thread::scope(|scope| {
                let handles: Vec<_> = batch
                    .chunks(per)
                    .map(|part| {
                        scope.spawn(move || {
                            part.iter()
                                .map(|&(_, c)| this.list(run.world, ChunkCoord(c), classes))
                                .collect::<Vec<_>>()
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .flat_map(|h| h.join().expect("a listing thread panicked"))
                    .collect()
            })
        };
        let mut out = Pushed::default();
        for ((class, c), l) in batch.drain(..).zip(listed) {
            match self.push_listed(store, eye, class, ChunkCoord(c), l) {
                Ok(samples) => {
                    out.samples += samples;
                    out.full_blank |= class == 0
                        && samples > 0
                        && self.job_coarse.get(&c).is_some_and(|coarse| !coarse);
                }
                // A chunk the pool refuses waits, and the opening goes on to the next one: a
                // smaller block (a refinement) may fit, and what is in flight is still read
                // back and faded, so an eviction frees room.
                Err(BakeError::PoolFull { .. }) => {
                    self.stats.pool_full += 1;
                    out.refused0 += usize::from(class == 0);
                }
                Err(e) => return Err(e),
            }
        }
        Ok(out)
    }

    /// After a sliced opening that pushed a full job on a chunk with no block: counts it in
    /// [`QueueStats::starved_openings`] if, among up to `checks` candidates of the order from
    /// `next` not found blocked in the last [`QUEUE_RECHECK`] steps, one with no block outside
    /// [`BakeQueue::full_first`] is ready and was left unpushed. Nothing with no coarse pass,
    /// or with every blank chunk's first pass coarse.
    fn note_starved(
        &mut self,
        run: &BakeRun<'_>,
        view: &QueueView<'_>,
        eye: [i32; 3],
        mut next: usize,
        checks: usize,
    ) {
        if self.coarse_samples == 0 || self.full_first.is_none() {
            return;
        }
        let lr = run.gpu.light_radius();
        let steps = self.steps;
        let mut looked = 0;
        while let Some(Cand {
            class, coarse, c, ..
        }) = self.ordered.get(next)
        {
            next += 1;
            if looked >= checks {
                break;
            }
            if class != 0
                || self.pool.get(&c) != Some(&(class, coarse))
                || self.full_first_at(c, eye)
                || self
                    .blocked
                    .get(&c)
                    .is_some_and(|&at| steps - at < QUEUE_RECHECK)
            {
                continue;
            }
            looked += 1;
            let coord = ChunkCoord(c);
            if self.bakeable(run, coord, lr, view, eye).is_some()
                && run.gpu.lights_ready_near(coord, lr)
            {
                self.stats.starved_openings += 1;
                return;
            }
        }
    }

    /// Opens `coord`'s job from its listed faces (and whether its masks were prepared): its
    /// place allocated, its count the coarse pass's for a chunk of `class` 0 (no block) outside
    /// [`BakeQueue::full_first`] of `eye` (chunks), else the settings'. A chunk with no face is marked
    /// faceless instead. The job's samples at most (0 for none).
    fn push_listed(
        &mut self,
        store: &mut Store,
        eye: [i32; 3],
        class: u8,
        coord: ChunkCoord,
        (faces, prepared): (ChunkFaces, bool),
    ) -> Result<u64, BakeError> {
        let c = coord.0;
        self.stats.prepared += u64::from(prepared);
        // Faceless or in a job, it is no candidate either way; a refused place keeps it one.
        let candidacy = self.pool.remove(&c);
        if let Some(m) = self.prepared.remove(&c) {
            self.prepared_bytes -= m.masks.bytes();
        }
        let partial = self.partial_checked.remove(&c);
        if faces.faces() == 0 {
            self.reach.remove(&c);
            self.partial_due.remove(&c);
            self.partial.remove(&c);
            self.faceless.insert(c);
            return Ok(0);
        }
        let words = layout(&faces, self.record_words()).1;
        let first_word = match store.alloc(words) {
            Ok(w) => w,
            Err(e) => {
                if let Some(v) = candidacy {
                    self.pool.insert(c, v);
                }
                return Err(e);
            }
        };
        self.reach.remove(&c);
        self.partial_due.remove(&c);
        if partial {
            self.job_partial.insert(c);
            self.stats.partial_opened += 1;
        } else {
            self.job_partial.remove(&c);
        }
        let coarse = class == 0 && self.coarse_samples > 0 && !self.full_first_at(c, eye);
        let brick_samples = if coarse {
            self.coarse_samples
        } else {
            self.settings.brick_samples
        };
        let samples = faces
            .bricks
            .iter()
            .map(|(_, keys)| {
                let mut dirs = [false; 6];
                for k in keys {
                    dirs[usize::from(k & 7)] = true;
                }
                dirs.iter().filter(|d| **d).count() as u64 * u64::from(brick_samples)
            })
            .sum();
        self.lp.push(Job {
            coord,
            faces,
            first_word,
            kept_bricks: KeptBricks::new(),
            samples,
            brick_samples: coarse.then_some(brick_samples),
        });
        self.job_coarse.insert(c, coarse);
        self.job_eye.insert(c, self.eye_moves);
        self.stats.coarse_opened += u64::from(coarse);
        Ok(samples)
    }

    /// Whether a chunk with no block at `c` takes the full count directly: within
    /// [`BakeQueue::full_first`] of `eye` (Chebyshev, chunks).
    fn full_first_at(&self, c: [i32; 3], eye: [i32; 3]) -> bool {
        self.full_first
            .is_some_and(|r| (0..3).map(|k| (c[k] - eye[k]).abs()).max().unwrap_or(0) <= r)
    }

    /// `coord`'s faces: from its prepared masks and its neighbours' where they are held, the
    /// rest decoded from the world's chunks; and whether its own masks were prepared.
    fn list(&self, world: &World, coord: ChunkCoord, classes: &[u8]) -> (ChunkFaces, bool) {
        let Some(own) = self.prepared.get(&coord.0) else {
            return (
                list_world_chunk(world, coord, classes).unwrap_or_default(),
                false,
            );
        };
        let store = world.store();
        let neighbour = |face: usize| {
            let mut c = coord.0;
            c[face / 2] += if face % 2 == 1 { 1 } else { -1 };
            c
        };
        let held: [_; 6] = std::array::from_fn(|f| {
            let c = neighbour(f);
            match self.prepared.get(&c) {
                Some(m) => Some(Err(m.clone())),
                None => store.get(ChunkCoord(c)).map(Ok),
            }
        });
        let locks: Vec<_> = held
            .iter()
            .map(|h| match h {
                Some(Ok(c)) => Some(Ok(c
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner))),
                Some(Err(m)) => Some(Err(m.clone())),
                None => None,
            })
            .collect();
        let faces = list_faces_by(&|slot| own.masks.get(slot), &|face, slot| {
            locks[face].as_ref().map(|n| match n {
                Ok(c) => class_masks(c.brick(slot), classes),
                Err(m) => m.masks.get(slot),
            })
        });
        (faces, true)
    }
}

/// Whether every chunk whose box lies within `lr` chunks of `coord`'s (the candidate table's
/// reach) is mirrored full and current, or known empty: in a bounded world (`resident` `None`) a
/// chunk the mirror does not wait to upload; in a streamed one a chunk resident full whose
/// record is current.
fn reach_ready(
    run: &BakeRun<'_>,
    coord: ChunkCoord,
    lr: i32,
    resident: Option<&dyn Fn(ChunkCoord) -> bool>,
) -> bool {
    reach_of(run, coord, lr, resident) == Reach::Ready
}

/// `coord`'s reach as [`reach_ready`] reads it: [`Reach::Waiting`] at the first chunk within it
/// waiting to upload, else [`Reach::Partial`] if a chunk within it is not resident full (a
/// streamed world only), else [`Reach::Ready`].
fn reach_of(
    run: &BakeRun<'_>,
    coord: ChunkCoord,
    lr: i32,
    resident: Option<&dyn Fn(ChunkCoord) -> bool>,
) -> Reach {
    let gpu = run.gpu;
    if resident.is_none() && gpu.pending_chunks() == 0 {
        return Reach::Ready;
    }
    let r = lr + 1;
    let mut partial = false;
    for z in -r..=r {
        for y in -r..=r {
            for x in -r..=r {
                if !in_reach([x, y, z], lr) {
                    continue;
                }
                let n = ChunkCoord([coord.0[0] + x, coord.0[1] + y, coord.0[2] + z]);
                if gpu.is_pending(n) {
                    return Reach::Waiting;
                }
                if let Some(res) = resident
                    && !(res(n) && gpu.is_mirrored(n) && !gpu.is_lod(n))
                {
                    partial = true;
                }
            }
        }
    }
    if partial {
        Reach::Partial
    } else {
        Reach::Ready
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

    /// Three neighbouring places freed in any order merge into one, which a place of their sum
    /// takes back at its first word; the pool's end is untouched while a block lies past them,
    /// and moves back over them once that block is freed too.
    /// Seconds of 1.4e7 samples over 190 chunks, 1.5e7 over 12 and 1.5e7 over 35 read B, A and
    /// mixed; 20 tagged steps, 12 in B and 8 in A, count 12 and 8 and take each regime's p50
    /// from its own steps alone.
    /// At 150 ms a second a 10 ms interval gives a 1.5 ms slice and a 66 ms one 9.9 ms; at 300
    /// ms a second and a cap of 0.25 the 66 ms interval gives the cap's 16.5 ms; a 0 interval
    /// gives the floor. A per-frame budget ignores the interval; both forms parse back from
    /// their text.
    #[test]
    fn a_per_second_budget_sizes_the_slice_by_the_interval() {
        let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
        let s150 = Budget::PerSecond {
            ms_per_s: 150.0,
            cap: 0.25,
        };
        assert!(near(s150.slice_ms(0.010), 1.5), "{}", s150.slice_ms(0.010));
        assert!(near(s150.slice_ms(0.066), 9.9), "{}", s150.slice_ms(0.066));
        let s300 = Budget::PerSecond {
            ms_per_s: 300.0,
            cap: 0.25,
        };
        assert!(near(s300.slice_ms(0.066), 16.5), "{}", s300.slice_ms(0.066));
        assert_eq!(s150.slice_ms(0.0), BAKE_SLICE_FLOOR_MS);
        assert_eq!(s300.slice_ms(0.0), BAKE_SLICE_FLOOR_MS);
        let f = Budget::PerFrame { ms: 1.5 };
        assert_eq!(f.slice_ms(0.0), 1.5);
        assert_eq!(f.slice_ms(0.066), 1.5);
        assert_eq!(f.to_string(), "frame:1.5");
        assert_eq!(s150.to_string(), "second:150,0.25");
        assert_eq!(Budget::parse("frame:1.5"), Ok(f));
        assert_eq!(Budget::parse("second:150,0.25"), Ok(s150));
        assert!(Budget::parse("second:150").is_err());
        assert!(Budget::parse("second:150,0").is_err());
        assert!(Budget::parse("minute:3").is_err());
    }

    #[test]
    fn the_regime_line_reads_fixed_seconds() {
        assert_eq!(Regime::of(14_000_000, 190), Some(Regime::KeepingUp));
        assert_eq!(Regime::of(15_000_000, 12), Some(Regime::Starved));
        assert_eq!(Regime::of(15_000_000, 35), Some(Regime::Mixed));
        assert_eq!(Regime::of(0, 0), None);
        assert_eq!(Regime::of(1_000, 0), Some(Regime::Starved));
        let step = |total_ms: f64, opened: bool| StepStages {
            total_ms,
            opened,
            ..StepStages::default()
        };
        let mut t = StepTally::default();
        // B's 12 steps: 9 to 9.11 ms, all opening but the last two at 1 ms.
        for i in 0..12 {
            let open = i < 10;
            t.push(step(
                if open { 9.0 + f64::from(i) * 0.01 } else { 1.0 },
                open,
            ));
        }
        t.tag(Some(Regime::KeepingUp));
        // A's 8 steps: 0.5 to 0.57 ms, none opening.
        for i in 0..8 {
            t.push(step(0.5 + f64::from(i) * 0.01, false));
        }
        t.tag(Some(Regime::Starved));
        assert_eq!(t.count(Some(Regime::KeepingUp)), 12);
        assert_eq!(t.count(Some(Regime::Starved)), 8);
        assert_eq!(t.count(Some(Regime::Mixed)), 0);
        // Nearest rank: the 6th of B's 12 sorted (1, 1, 9.00 .. 9.09) is 9.03; of A's 8 the
        // 4th, 0.53.
        assert!((t.percentile(Some(Regime::KeepingUp), None, 50.0) - 9.03).abs() < 1e-9);
        assert!((t.percentile(Some(Regime::Starved), None, 50.0) - 0.53).abs() < 1e-9);
        assert!((t.percentile(Some(Regime::KeepingUp), Some(false), 50.0) - 1.0).abs() < 1e-9);
        let lines = t.summary();
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(
            lines[0].starts_with("queue step A: 8 steps, p50 0.530 ms,"),
            "{}",
            lines[0]
        );
        assert!(
            lines[1].starts_with(
                "queue step B: 12 steps, p50 9.030 ms, p99 9.090 ms, opened 10 (0.833), open p50 9.040 ms, not p50 1.000 ms;"
            ),
            "{}",
            lines[1]
        );
        assert!(
            lines[2].starts_with("queue step pooled: 20 steps,"),
            "{}",
            lines[2]
        );
    }

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
        .unwrap_or_else(|e| panic!("the bake module validates: {e:?}"));
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
        // The stream workers' path lists the same faces from prepared masks: the chunk's and
        // its neighbours' all prepared, and again with half the neighbours decoded inline.
        let own = ChunkMasks::of(&chunk, &classes);
        let near: [ChunkMasks; 6] = std::array::from_fn(|f| ChunkMasks::of(&beside[f], &classes));
        let prepared = list_faces_by(&|slot| own.get(slot), &|face, slot| {
            Some(near[face].get(slot))
        });
        assert_eq!(prepared, faces, "all prepared");
        let mixed = list_faces_by(&|slot| own.get(slot), &|face, slot| {
            Some(if face % 2 == 0 {
                near[face].get(slot)
            } else {
                class_masks(beside[face].brick(slot), &classes)
            })
        });
        assert_eq!(mixed, faces, "half inline");
        // An absent neighbour lists nothing across, by either path.
        let absent: [Option<&Chunk>; 6] = std::array::from_fn(|f| (f != 3).then(|| &beside[f]));
        let inline = list_chunk_faces(&chunk, &absent, &classes);
        let by_masks = list_faces_by(&|slot| own.get(slot), &|face, slot| {
            (face != 3).then(|| near[face].get(slot))
        });
        assert_eq!(by_masks, inline, "one neighbour absent");
        assert_ne!(inline, faces, "the absent neighbour changes the list");
    }

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
        let wgsl = include_str!("shaders/temporal.wgsl");
        assert!(wgsl.contains(&format!("const STORE_SUMMARY_AT: u32 = {SUMMARY_AT}u;")));
        assert!(wgsl.contains("(store[STORE_FLAGS] >> 2u) & 3u"));
        assert_eq!(FLAG_MISSING_SHIFT, 2);
        assert_eq!(MissingLight::Neighbour.bits(), 1);
        assert_eq!(MissingLight::Parent.bits(), 2);
        assert!(wgsl.contains("const MISSING_NEIGHBOUR: u32 = 1u;"));
        assert!(wgsl.contains("const MISSING_PARENT: u32 = 2u;"));
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

    /// ADR-0048's angle term at the fly-through's view: a 70 degree vertical field at 16:9 is
    /// 51.2 degrees horizontally, the cone 66.2 degrees every way. A chunk ahead reads 0, one
    /// straight behind (180 - 66.2) / 180, one behind in the near shell 0; at an angle weight
    /// of 0 every score is the distance (times the refine weight for a coarse block).
    #[test]
    fn the_view_cone_scores_a_chunk_by_its_angle() {
        let half = view_half_angle(70f32.to_radians(), 16.0 / 9.0);
        let want = (35f32.to_radians().tan() * 16.0 / 9.0).atan();
        assert!((half - want).abs() < 1e-6, "{half} {want}");
        assert!(
            (half.to_degrees() - 51.2).abs() < 0.05,
            "{}",
            half.to_degrees()
        );
        // The wider of the two, not the corner's: a tall view takes its vertical half-angle.
        assert!((view_half_angle(70f32.to_radians(), 0.5) - 35f32.to_radians()).abs() < 1e-6);
        assert!((view_half_angles(70f32.to_radians(), 16.0 / 9.0)[0] - want).abs() < 1e-6);
        let cam = [2, 0, 0];
        let forward = [1.0, 0.0, 0.0];
        let off = |c: [i32; 3]| [0, 1, 2].map(|k| c[k] - cam[k]);
        let a = |c: [i32; 3]| angle_term(off(c), forward, half, 1);
        assert_eq!(a([7, 0, 0]), 0.0);
        let behind = a([-3, 0, 0]);
        let cone = half.to_degrees() + QUEUE_VIEW_MARGIN_DEG;
        assert!((cone - 66.2).abs() < 0.05, "{cone}");
        assert!((behind - (180.0 - 66.2) / 180.0).abs() < 0.005, "{behind}");
        assert!((behind - 0.632).abs() < 0.005, "{behind}");
        assert_eq!(a([1, 0, 0]), 0.0, "behind, in the shell");
        // Just inside and just outside the cone, off to the side.
        let side = |deg: f32| {
            let r = deg.to_radians();
            angle_term(
                [(100.0 * r.cos()) as i32, (100.0 * r.sin()) as i32, 0],
                forward,
                half,
                1,
            )
        };
        assert_eq!(side(64.0), 0.0);
        assert!(side(70.0) > 0.0 && side(70.0) < 0.03, "{}", side(70.0));
        for c in [[7, 0, 0], [-3, 0, 0], [1, 0, 0], [2, 5, -3]] {
            let o = off(c);
            let len = (o.iter().map(|v| (v * v) as f32).sum::<f32>()).sqrt();
            let t = angle_term(o, forward, half, 1);
            assert_eq!(queue_score(o, t, false, 0.0, 2.0), len, "{c:?}");
            assert_eq!(queue_score(o, t, true, 0.0, 2.0), 2.0 * len, "{c:?}");
        }
        // The weight scales the angle: behind at |o| 5 and weight 4 scores 5 x (1 + 4 x 0.632).
        let s = queue_score(off([-3, 0, 0]), behind, false, 4.0, 2.0);
        assert!((s - 5.0 * (1.0 + 4.0 * behind)).abs() < 1e-4, "{s}");
        // The widened frustum is narrower than the cone up and down: 60 degrees up is inside
        // the cone (66.2) and outside the frustum (35 + 15).
        let up = [100, 0, 173];
        assert_eq!(angle_term(up, forward, half, 1), 0.0);
        assert!(!in_widened_frustum(
            up,
            forward,
            view_half_angles(70f32.to_radians(), 16.0 / 9.0)
        ));
        assert!(in_widened_frustum(
            [100, 0, 50],
            forward,
            view_half_angles(70f32.to_radians(), 16.0 / 9.0)
        ));
    }
}
