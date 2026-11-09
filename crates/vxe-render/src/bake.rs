//! The loaded light's store and its bake (ADR-0046): every face of a chunk holds its emissive
//! irradiance at unit emission in a cell of one pool, computed ahead of the frames that read it.
//!
//! **The list.** [`list_chunk_faces`] is a pure function of a chunk's voxels, its six neighbours'
//! and each material's class ([`face_classes`]): each surface brick's sorted face keys, an opaque
//! voxel's faces toward a voxel that is not opaque and a translucent voxel's faces toward air
//! (ADR-0047: a pane's two sides), the latter flagged [`CELL_TRANSLUCENT`] in the cell's tail.
//!
//! **The pool** ([`Store`]), in `u32` words: the frame's counters (words 0 to 2: no value, brick
//! reads, [`STORE_COARSE_READS`]), the mirror window's origin (word 4) and size (word 8) in
//! chunks, then from word [`STORE_TABLE`] one word per window chunk naming its block's first word
//! (0 none), bit 31 ([`TABLE_COARSE`]) set on the live queue's coarse blocks. A block is 512 directory words, a brick's
//! `run offset from the block's start << 11 | faces` (0 no surface), then the runs: two words a
//! cell, its [`rgb9e5_pack`] irradiance, then `key | flags << 12 | dominant material << 16`, with
//! the key `voxel in the brick << 3 | face`, sorted by key, and flag bit 0 ([`CELL_TRANSLUCENT`])
//! set on a translucent voxel's face.
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
/// The pool before the first bake or live queue: the header's words only, which hold no block.
pub const MINIMAL_POOL_BYTES: u64 = 64;
/// The word of the pool's chunk table.
pub const STORE_TABLE: u64 = 16;
/// A chunk table word's flag: the chunk's block is the live queue's coarse pass, refined later.
/// The block's first word is the rest (a pool is under 2^31 words).
pub const TABLE_COARSE: u32 = 1 << 31;
/// The header's counter of the lighting pixels that read a coarse block (word 2).
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
}

/// The stream workers' hook (ADR-0017, `vxe_world::ChunkPrep`): each full chunk's
/// [`ChunkMasks`] under the material classes it was made with, for [`list_faces_by`].
pub struct FacePrep {
    pub classes: Arc<Vec<u8>>,
}

impl vxe_world::ChunkPrep for FacePrep {
    fn prepare(&self, _coord: ChunkCoord, chunk: &Chunk) -> vxe_world::Prepared {
        vxe_world::Prepared(Arc::new(PreparedMasks {
            classes: self.classes.clone(),
            masks: ChunkMasks::of(chunk, &self.classes),
        }))
    }
}

/// What [`FacePrep`] hands back: the masks and the classes they were taken under.
pub struct PreparedMasks {
    pub classes: Arc<Vec<u8>>,
    pub masks: ChunkMasks,
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
    let (dir, words) = layout(faces, Some(Tier::Brick), record);
    let mut block = dir;
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
    /// The settings name no brick level for the live queue (`BakeSettings::rings`).
    NotBaked,
    /// The pool cannot hold the extent's blocks.
    PoolFull { needed: u64, capacity: u64 },
    /// The light-sample buffer refused a chunk in the bake's reach.
    LightsRefused(u64),
}

impl std::fmt::Display for BakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotBaked => write!(f, "the settings name no brick level for the live queue"),
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
    /// Lighting pixels of the last encode whose cube face read a value in a coarse block.
    pub coarse_reads: u32,
    /// Chunks whose block is coarse.
    pub coarse_chunks: u64,
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
    /// The chunks the last bake covered; a dig re-bakes only inside them.
    piece: Option<ChunkExtent>,
    /// The mirror window the chunk table indexes.
    window: Option<ChunkExtent>,
    /// The blocks carry brick records ([`Rings`]) of cells of this many voxels.
    bricks: Option<u32>,
    /// Places freed by [`Store::remove`]: first word and words, for [`Store::alloc`].
    free: Vec<(u64, u64)>,
    /// Chunks whose block is the live queue's coarse pass: their table word carries
    /// [`TABLE_COARSE`].
    coarse: std::collections::HashSet<[i32; 3]>,
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
            bricks: None,
            free: Vec::new(),
            coarse: Default::default(),
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
        }
    }

    /// Whether `coord`'s block is the live queue's coarse pass.
    pub fn is_coarse(&self, coord: ChunkCoord) -> bool {
        self.coarse.contains(&coord.0)
    }

    /// Marks `coord`'s next [`Store::put`] as coarse or not; the table word follows at the put.
    fn mark_coarse(&mut self, coord: ChunkCoord, coarse: bool) {
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
        self.bricks = None;
        self.head_words = STORE_TABLE + table_words;
        self.end_words = self.head_words;
        self.piece = None;
        self.window = Some(window);
        self.free.clear();
        self.coarse.clear();
    }

    /// Empties the store for `window` for the live queue: blocks with brick records of cells of
    /// `cell` voxels, the piece the window.
    pub(crate) fn start_queue(&mut self, queue: &wgpu::Queue, window: ChunkExtent, cell: u32) {
        self.reset(queue, window);
        self.set_bricks(queue, Some(cell));
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
            let bricks = self.bricks;
            self.reset(queue, window);
            self.set_bricks(queue, bricks);
            return all;
        } else {
            let inside =
                |c: &[i32; 3]| (0..3).all(|k| (0..size[k]).contains(&(c[k] - window.min.0[k])));
            self.blocks.keys().copied().filter(|c| !inside(c)).collect()
        };
        for c in &dropped {
            if let Some((first, words)) = self.blocks.remove(c) {
                self.free.push((first, words));
            }
            self.words.remove(c);
            self.coarse.remove(c);
        }
        self.window = Some(window);
        let table_words = (size[0] as u64) * (size[1] as u64) * (size[2] as u64);
        let mut header = vec![0u32; (STORE_TABLE + table_words) as usize];
        for k in 0..3 {
            header[4 + k] = window.min.0[k] as u32;
            header[8 + k] = size[k] as u32;
        }
        header[STORE_FLAGS as usize] = match self.bricks {
            None => 0,
            Some(4) => FLAG_BRICKS | FLAG_CELL4,
            Some(_) => FLAG_BRICKS,
        };
        for (c, (first, _)) in &self.blocks {
            header[self.table_word(ChunkCoord(*c)) as usize] =
                self.table_value(ChunkCoord(*c), *first);
        }
        // Words 0 to 2 are the frame's counters, which the temporal pass writes.
        queue.write_buffer(&self.pool, 12, bytemuck::cast_slice(&header[3..]));
        dropped
    }

    /// A place of `words` words: the smallest freed place that holds it (its rest freed again),
    /// else past the last block.
    pub(crate) fn alloc(&mut self, words: u64) -> Result<u64, BakeError> {
        let fit = (0..self.free.len())
            .filter(|&i| self.free[i].1 >= words)
            .min_by_key(|&i| (self.free[i].1, self.free[i].0));
        if let Some(i) = fit {
            let (first, have) = self.free.swap_remove(i);
            if have > words {
                self.free.push((first + words, have - words));
            }
            return Ok(first);
        }
        let capacity = self.pool.size();
        if (self.end_words + words) * 4 > capacity {
            return Err(BakeError::PoolFull {
                needed: (self.end_words + words) * 4,
                capacity,
            });
        }
        let first = self.end_words;
        self.end_words += words;
        Ok(first)
    }

    /// Frees a place [`Store::alloc`] handed out that no block took.
    pub(crate) fn release(&mut self, first: u64, words: u64) {
        self.free.push((first, words));
    }

    /// Writes `words` at `first` in the pool alone: what a fading block shows while the store's
    /// copy holds its target.
    pub(crate) fn show(&self, queue: &wgpu::Queue, first: u64, words: &[u32]) {
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
    fn put(&mut self, queue: &wgpu::Queue, coord: ChunkCoord, first: u64, block: Vec<u32>) {
        queue.write_buffer(&self.pool, first * 4, bytemuck::cast_slice(&block));
        queue.write_buffer(
            &self.pool,
            self.table_word(coord) * 4,
            bytemuck::bytes_of(&self.table_value(coord, first)),
        );
        self.end_words = self.end_words.max(first + block.len() as u64);
        self.blocks.insert(coord.0, (first, block.len() as u64));
        self.words.insert(coord.0, block);
    }

    /// Drops `coord`'s block, its place freed: the chunk table reads none.
    pub(crate) fn remove(&mut self, queue: &wgpu::Queue, coord: ChunkCoord) {
        if let Some(place) = self.blocks.remove(&coord.0) {
            self.free.push(place);
            self.words.remove(&coord.0);
            self.coarse.remove(&coord.0);
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
        Self {
            layout,
            pipeline,
            job,
            faces,
            tables,
        }
    }

    /// Group 3 of a bake's dispatch: the job uniform, the working buffer and the table slots.
    pub(crate) fn bind(&self, device: &wgpu::Device) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bake"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.job.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.faces.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.tables.as_entire_binding(),
                },
            ],
        })
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
    /// Samples submitted and not read back yet: the next round starts at `n + queued`.
    queued: u32,
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
    /// Samples it draws at most, for the live queue's depth ([`JobLoop::samples_left`]); 0 where
    /// nobody reads it.
    pub samples: u64,
    /// A brick direction's samples at the brick level for this job alone (the live queue's coarse
    /// pass); `None` the settings' [`Rings::brick_samples`].
    pub brick_samples: Option<u32>,
}

/// Brick records a re-baked ring job keeps, by brick slot.
pub(crate) type KeptBricks = std::collections::HashMap<u16, Vec<u32>>;

/// The cells a re-baked job keeps: by brick slot and key, the two words of the cell before, for
/// every face outside the ring that the block before holds.
pub(crate) type KeptCells = std::collections::HashMap<(u16, u16), [u32; 2]>;

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
        let source = &store.words;
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
    let bind = run.pipeline.bind(run.device);
    // The frame's parameters, once: the bake reads no view but the LOD walk's camera chunk,
    // which it takes from the face's job.
    run.queue
        .write_buffer(run.params_buffer, 0, bytemuck::bytes_of(&run.params));
    let queue = run.queue;
    let mut sink = |store: &mut Store, coord: ChunkCoord, first: u64, block: Vec<u32>| {
        store.put(queue, coord, first, block);
    };
    while lp
        .submit(run, &bind, store, settings, u64::MAX, &mut sink, progress)
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
    free: Vec<usize>,
    in_flight: VecDeque<InFlight>,
    staging: Vec<wgpu::Buffer>,
    /// Whether the loop times its submits on its own timestamps, and the timer once made
    /// (`Some(None)`: the device has no timestamps).
    timed: bool,
    timer: Option<Option<LoopTimer>>,
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
            free: slots.rev().collect(),
            in_flight: VecDeque::new(),
            staging: Vec::new(),
            timed: false,
            timer: None,
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

    /// Writes each open job's candidate table again from the ranges `run.gpu` holds now: a sync
    /// between two submits may have moved a range.
    pub fn refresh_tables(&self, run: &BakeRun<'_>) {
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
            n_max: settings.n_max.max(1),
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
        let mut budget = Budget {
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
            return false;
        }
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
                    at = self
                        .open
                        .iter()
                        .position(|j| j.slot == slot)
                        .expect("a job with entries in flight stays open");
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
            n_max: settings.n_max.max(1),
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
                let (coord, first, block) = ctx.close(job, &mut self.report);
                sink(store, coord, first, block);
                self.done.chunks_done += 1;
            } else {
                k += 1;
            }
        }
    }
}

/// Where an ended job's block goes: the store, its chunk, its place and its words.
pub(crate) type BlockSink<'a> = dyn FnMut(&mut Store, ChunkCoord, u64, Vec<u32>) + 'a;

/// A submit's room: its entries against the batch, its samples against the cap.
struct Budget {
    entries: usize,
    batch: usize,
    samples: u64,
    cap: u64,
}

impl Budget {
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
            samples: _,
            brick_samples,
        } = job;
        write_job_table(self.run, coord, slot);
        // A job's own count (the live queue's coarse pass) on cells of a brick draws each
        // direction's samples on `ceil(count / BAKE_LANES)` of its faces, spread evenly through
        // them in key order, so each entry fills its workgroup; the record is their mean.
        let subset =
            brick_samples.is_some() && self.settings.rings.as_ref().is_some_and(|r| r.cell == 8);
        let brick_samples = brick_samples.unwrap_or_else(|| {
            self.settings
                .rings
                .as_ref()
                .map_or(BRICK_LEVEL_SAMPLES, |r| r.brick_samples)
        });
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
            // Under `subset`, a direction's `m` faces drawn of its `k`: the i-th opaque face is
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
                let glass = faces.is_glass(*brick_at, key);
                let keep = kept
                    .as_ref()
                    .and_then(|k| k.get(&(*brick_at, key)).copied());
                let (n_min, n_max) = match tier {
                    None => (self.settings.n_min, self.n_max),
                    Some(Tier::Face(n)) if cell => (n, n),
                    Some(_) => {
                        let n = brick_samples.div_ceil(drawn[usize::from(face)].max(1));
                        (n, n)
                    }
                };
                let d = usize::from(face);
                let in_subset = glass || {
                    let i = at_dir[d];
                    at_dir[d] += 1;
                    u64::from(i) * u64::from(drawn[d]) % u64::from(per_dir[d].max(1))
                        < u64::from(drawn[d])
                };
                let wanted = in_subset
                    && keep.is_none()
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
                    queued: 0,
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

    /// The rounds of `job`'s live faces that cannot end them, while `budget` has room.
    fn plan(&self, job: &OpenJob, budget: &mut Budget) -> Vec<(usize, u32)> {
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

    /// The ended job's chunk, place and block: the directory then its cells in key order.
    fn close(&self, job: OpenJob, report: &mut BakeReport) -> (ChunkCoord, u64, Vec<u32>) {
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
/// Bytes of prepared masks the queue holds at most; the farthest from the camera go first.
const PREPARED_BYTES_MAX: usize = 64 << 20;

/// The live queue's settings: the brick level everywhere, cells of 8 voxels, [`QUEUE_BRICK_SAMPLES`]
/// a brick direction, independent seeds, no filter (no face holds a cell).
pub fn queue_settings() -> BakeSettings {
    BakeSettings {
        rings: Some(Rings {
            centre: [0; 3],
            brick: i32::MAX,
            faces: Vec::new(),
            cell: BRICK_CELL,
            brick_samples: QUEUE_BRICK_SAMPLES,
        }),
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
    /// Samples this step draws at most; `None` sizes it by [`BakeQueue::slice_ms`].
    pub samples: Option<u64>,
    /// Chunks from the focus the light-sample ranges are held out to; `None` the window's far
    /// corner plus the light radius.
    pub hold: Option<i32>,
}

/// The live queue (ADR-0046's scheduler, Plan 0059 Phase 8 as re-read at Phase 7b): the brick
/// level of every full chunk in the mirror, baked a slice a frame on the render thread. A chunk
/// is opened nearest the camera first, ties by the view, once every chunk within the light radius
/// of it is mirrored or known empty and every emitter range its table reads is held. Its faces are
/// listed from the stream workers' masks ([`FacePrep`]) where they arrived, else inline. A first
/// block is written directly; a block that replaces one of the same layout fades over [`FADE`]
/// frames. A chunk leaving the mirror frees its block.
///
/// **Coarse first** (Plan 0059 Phase 8c). With `coarse_samples` above 0, a chunk with no block
/// beyond `near` chunks (Chebyshev) of the camera's is first baked at `coarse_samples` a brick
/// direction, its block flagged coarse ([`TABLE_COARSE`]); one within `near` at the full count.
/// Every chunk with no block comes before every coarse one, so a chunk the mirror gains is lit
/// coarse at the head of the queue, and the coarse ones are then refined nearest first. A
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
    /// The slice, in milliseconds a frame.
    pub slice_ms: f64,
    /// A brick direction's samples in the coarse pass; 0 bakes every chunk at the full count.
    pub coarse_samples: u32,
    /// Chunks (Chebyshev) around the camera's whose first pass is the full count.
    pub near: i32,
    /// Milliseconds a frame the slice grows by while a dig's bricks wait.
    pub edit_ms: f64,
    /// Whether a dig that touched lit bricks re-bakes the chunks within the light radius (ring 3).
    pub reach_ring: bool,
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
    faceless: std::collections::HashSet<[i32; 3]>,
    /// Chunks an edit touched while their job ran: the job's block is thrown away.
    stale: std::collections::HashSet<[i32; 3]>,
    /// Candidates not ready at a check, and the step that checked them.
    blocked: std::collections::HashMap<[i32; 3], u64>,
    fades: BTreeMap<[i32; 3], Fade>,
    steps: u64,
    /// The step before which no candidate is sorted: set when none was ready.
    quiet_until: u64,
    stats: QueueStats,
}

impl BakeQueue {
    pub fn new(settings: BakeSettings, slice_ms: f64) -> Self {
        Self {
            settings,
            slice_ms,
            coarse_samples: DEFAULT_COARSE_SAMPLES,
            near: DEFAULT_NEAR,
            edit_ms: DEFAULT_BAKE_EDIT_MS,
            reach_ring: true,
            boxes: Vec::new(),
            digs: BTreeMap::new(),
            dig_open: Default::default(),
            reach: Default::default(),
            listed: Default::default(),
            job_coarse: Default::default(),
            lp: JobLoop::new(Vec::new(), LOOP_SLOTS..OPEN_JOBS).timed(),
            prepared: Default::default(),
            prepared_bytes: 0,
            eye: [0; 3],
            faceless: Default::default(),
            stale: Default::default(),
            blocked: Default::default(),
            fades: BTreeMap::new(),
            steps: 0,
            quiet_until: 0,
            stats: QueueStats {
                ns_per_sample: 20.0,
                ..QueueStats::default()
            },
        }
    }

    pub fn stats(&self) -> QueueStats {
        self.stats
    }

    /// No job pending or open, no block fading, and no candidate at the last count.
    pub fn empty(&self) -> bool {
        self.lp.idle()
            && self.fades.is_empty()
            && self.stats.depth == 0
            && self.boxes.is_empty()
            && self.digs.is_empty()
            && self.reach.is_empty()
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
    }

    /// Whether `c` has had its first pass: it holds a block (coarse or full), is known to have
    /// no face, or its reach or lights were not ready at its last check.
    pub fn passed(&self, store: &Store, c: ChunkCoord) -> bool {
        store.blocks.contains_key(&c.0)
            || self.faceless.contains(&c.0)
            || self.blocked.contains_key(&c.0)
    }

    /// Takes the stream workers' results; one made under other classes than `classes` is dropped.
    pub fn take_prepared(
        &mut self,
        prepared: &[(ChunkCoord, vxe_world::Prepared)],
        classes: &[u8],
    ) {
        for (c, p) in prepared {
            let Ok(m) = p.0.clone().downcast::<PreparedMasks>() else {
                continue;
            };
            if m.classes.as_slice() != classes {
                continue;
            }
            self.prepared_bytes += m.masks.bytes();
            if let Some(old) = self.prepared.insert(c.0, m) {
                self.prepared_bytes -= old.masks.bytes();
            }
            self.faceless.remove(&c.0);
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
    /// fades a frame on. `None` from the submit when nothing was dispatched.
    pub(crate) fn step(
        &mut self,
        run: &BakeRun<'_>,
        store: &mut Store,
        view: &QueueView<'_>,
    ) -> Result<QueueStats, BakeError> {
        self.steps += 1;
        self.eye = view.eye.map(|v| (v / 64.0).floor() as i32);
        let gpu = run.gpu;
        let window = gpu.window();
        for c in store.rewindow(run.queue, window) {
            self.fades.remove(&c);
        }
        store.piece = Some(window);
        // A chunk the mirror does not hold full (evicted, or LOD-only) frees its block.
        let gone: Vec<[i32; 3]> = store
            .blocks
            .keys()
            .copied()
            .filter(|c| !gpu.is_mirrored(ChunkCoord(*c)) || gpu.is_lod(ChunkCoord(*c)))
            .collect();
        for c in gone {
            store.remove(run.queue, ChunkCoord(c));
            self.fades.remove(&c);
            self.faceless.remove(&c);
            self.digs.remove(&c);
            self.reach.remove(&c);
            self.listed.remove(&c);
        }
        self.take_boxes(run, store)?;
        let dig = !self.digs.is_empty() || !self.dig_open.is_empty();
        let cap = view.samples.unwrap_or_else(|| {
            let ms = self.slice_ms + if dig { self.edit_ms } else { 0.0 };
            ((ms * 1e6 / self.stats.ns_per_sample.max(0.1)) as u64).clamp(16_384, SUBMIT_SAMPLES)
        });
        self.open_digs(run, store, cap)?;
        if self.lp.samples_left() < 2 * cap && self.steps >= self.quiet_until {
            let opened = self.lp.chunks().count();
            self.open(run, store, view, cap)?;
            if self.lp.chunks().count() == opened && self.lp.idle() {
                // Nothing was ready: the candidates are sorted again a few frames on.
                self.quiet_until = self.steps + QUEUE_QUIET;
            }
        }
        self.lp.refresh_tables(run);
        let bind = run.pipeline.bind(run.device);
        run.queue
            .write_buffer(run.params_buffer, 0, bytemuck::bytes_of(&run.params));
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
        let queue = run.queue;
        let stale = &mut self.stale;
        let fades = &mut self.fades;
        let stats = &mut self.stats;
        let job_coarse = &mut self.job_coarse;
        let dig_open = &mut self.dig_open;
        let reach = &mut self.reach;
        let record = self
            .settings
            .rings
            .as_ref()
            .map_or(BRICK_WORDS, Rings::record_words);
        let mut sink = |store: &mut Store, coord: ChunkCoord, first: u64, block: Vec<u32>| {
            let coarse = job_coarse.remove(&coord.0).unwrap_or(false);
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
            if let Some(d) = &dug {
                stats.digs_baked += 1;
                stats.dig_bricks += d.slots.len() as u64;
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
                    store.show(queue, f, &from);
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
                None => store.put(queue, coord, first, block),
            }
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
        let sent = self.lp.in_flight() < IN_FLIGHT
            && self
                .lp
                .dispatch(run, &bind, store, &settings, cap, &mut sink, &mut |_| {});
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
        }
        // The slice is sized by the GPU's own time for the submits read back, a frame or two
        // late; without timestamps it stays at the first guess, a fixed count.
        self.stats.samples = d.samples;
        self.stats.ms = d.gpu_ms.unwrap_or(d.seconds * 1e3);
        self.stats.total_samples += d.samples;
        if let Some(ms) = d.gpu_ms
            && d.samples >= 4096
        {
            let ns = ms * 1e6 / d.samples as f64;
            self.stats.ns_per_sample = 0.8 * self.stats.ns_per_sample + 0.2 * ns;
        }
        self.advance_fades(run.queue, store, record);
        self.stats.jobs = self.lp.chunks().count() as u64;
        self.stats.fading = self.fades.len() as u64;
        self.stats.coarse = store.coarse.len() as u64;
        self.stats.digs = (self.digs.len() + self.dig_open.len()) as u64;
        self.stats.reach = self.reach.len() as u64;
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
        let classes = face_classes(run.world.materials());
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
            self.fades.remove(&c);
            if faces.faces() == 0 {
                store.remove(run.queue, coord);
                self.faceless.insert(c);
                self.digs.remove(&c);
                continue;
            }
            let block = provisional_block(store, c, &faces, record, &near);
            store.release(first, words);
            let at = store.alloc(block.len() as u64)?;
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
        let classes = face_classes(run.world.materials());
        let record = self.record_words();
        let brick_samples = self
            .settings
            .rings
            .as_ref()
            .map_or(BRICK_LEVEL_SAMPLES, |r| r.brick_samples);
        let mut jobs = Vec::new();
        for (_, _, c) in order {
            if left >= 2 * cap && !jobs.is_empty() {
                break;
            }
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
            let first_word = store.alloc(layout(&faces, Some(Tier::Brick), record).1)?;
            self.job_coarse.insert(c, store.is_coarse(coord));
            left += samples;
            jobs.push(Job {
                coord,
                faces,
                first_word,
                kept: None,
                tier: Some(Tier::Brick),
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
        self.settings
            .rings
            .as_ref()
            .map_or(BRICK_WORDS, Rings::record_words)
    }

    /// Moves every fade a frame on: each record's direction values (and each cell's value) at
    /// `frame / FADE` of the way from what it showed to the store's copy; the last frame writes
    /// the copy itself.
    fn advance_fades(&mut self, queue: &wgpu::Queue, store: &Store, record: usize) {
        let mut ended = Vec::new();
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
                store.show(queue, f.first, to);
                ended.push(*c);
                continue;
            }
            let t = f.frame as f32 / f.frames as f32;
            let lerp = |a: u32, b: u32| {
                let (x, y) = (rgb9e5_unpack(a), rgb9e5_unpack(b));
                rgb9e5_pack([0, 1, 2].map(|k| x[k] + (y[k] - x[k]) * t))
            };
            let mut shown = to.clone();
            for slot in 0..CHUNK_BRICKS {
                let dir = to[slot];
                if dir == 0 {
                    continue;
                }
                let (run, n) = ((dir >> 11) as usize, (dir & 0x7FF) as usize);
                for cell in (run - record..run).step_by(BRICK_WORDS) {
                    for w in cell..cell + 6 {
                        shown[w] = lerp(f.from[w], to[w]);
                    }
                }
                for k in 0..n {
                    shown[run + 2 * k] = lerp(f.from[run + 2 * k], to[run + 2 * k]);
                }
            }
            store.show(queue, f.first, &shown);
        }
        for c in ended {
            self.fades.remove(&c);
        }
    }

    /// Opens jobs, nearest the camera first and ties by the view, while the open ones hold under
    /// two slices of `cap` samples: each a ready candidate's faces at the brick level.
    fn open(
        &mut self,
        run: &BakeRun<'_>,
        store: &mut Store,
        view: &QueueView<'_>,
        cap: u64,
    ) -> Result<(), BakeError> {
        let gpu = run.gpu;
        let eye = view.eye.map(|v| (v / 64.0).floor() as i32);
        let in_job: std::collections::HashSet<[i32; 3]> = self.lp.chunks().map(|c| c.0).collect();
        // Class 0: no block yet; class 1: a coarse block to refine. Every class 0 comes first.
        let mut cands: Vec<(u8, i64, i32, [i32; 3])> = gpu
            .full_chunks()
            .filter(|c| {
                (!store.blocks.contains_key(&c.0)
                    || store.is_coarse(*c)
                    || self.reach.contains(&c.0))
                    && !self.faceless.contains(&c.0)
                    && !in_job.contains(&c.0)
                    && !self.digs.contains_key(&c.0)
            })
            .map(|c| {
                let d = [0, 1, 2].map(|k| c.0[k] - eye[k]);
                let d2: i64 = d.iter().map(|&v| i64::from(v).pow(2)).sum();
                let len = (d2 as f32).sqrt().max(1.0);
                let dot: f32 = (0..3).map(|k| d[k] as f32 * view.forward[k]).sum::<f32>() / len;
                let class = u8::from(store.blocks.contains_key(&c.0));
                (class, d2, -(dot * 1000.0) as i32, c.0)
            })
            .collect();
        self.stats.depth = cands.len() as u64;
        let steps = self.steps;
        let recent = |at: Option<&u64>| at.is_some_and(|&at| steps - at < QUEUE_RECHECK);
        self.stats.unbaked = cands
            .iter()
            .filter(|c| c.0 == 0 && !recent(self.blocked.get(&c.3)))
            .count() as u64;
        cands.sort_unstable();
        let checks = if view.samples.is_some() {
            QUEUE_CHECKS_UNSLICED
        } else {
            QUEUE_CHECKS
        };
        let classes = face_classes(run.world.materials());
        let lr = gpu.light_radius();
        // Class 1 (refinement) is opened only by an opening that found no class 0 candidate
        // ready, so every chunk with no block that can bake comes first; those whose reach or
        // lights are not ready (a streamed world's edge) cannot starve the refinement behind them.
        let (mut checked, mut reach, mut lights) = ([0usize; 2], 0u64, 0u64);
        let mut ready0 = 0usize;
        // A step without a slice (the pre-roll) gates up to `checks` candidates and lists them
        // across threads; a sliced step lists each in turn and stops at two slices of samples.
        let parallel = view.samples.is_some();
        let mut ready: Vec<(u8, [i32; 3])> = Vec::new();
        // The loop's samples left, read once (it walks every open face) and then counted on.
        let mut left = self.lp.samples_left();
        for (class, _, _, c) in cands {
            if left >= 2 * cap || checked[1] >= checks || (class == 1 && ready0 > 0) {
                break;
            }
            if checked[usize::from(class)] >= checks || recent(self.blocked.get(&c)) {
                continue;
            }
            checked[usize::from(class)] += 1;
            let coord = ChunkCoord(c);
            if !reach_ready(run, coord, lr, view.resident) {
                reach += 1;
                self.blocked.insert(c, self.steps);
                continue;
            }
            if !gpu.lights_ready_near(coord, lr) {
                lights += 1;
                self.blocked.insert(c, self.steps);
                continue;
            }
            self.blocked.remove(&c);
            if class == 0 {
                ready0 += 1;
            }
            if parallel {
                ready.push((class, c));
                continue;
            }
            let listed = self.list(run.world, coord, &classes);
            left += self.push_listed(store, eye, class, coord, listed)?;
        }
        if !ready.is_empty() {
            let threads = std::thread::available_parallelism()
                .map_or(4, std::num::NonZero::get)
                .clamp(1, 16);
            let per = ready.len().div_ceil(threads);
            let this = &*self;
            let listed: Vec<(ChunkFaces, bool)> = std::thread::scope(|scope| {
                let handles: Vec<_> = ready
                    .chunks(per)
                    .map(|part| {
                        let classes = &classes;
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
            });
            for ((class, c), l) in ready.into_iter().zip(listed) {
                self.push_listed(store, eye, class, ChunkCoord(c), l)?;
            }
        }
        self.stats.waiting_reach = reach;
        self.stats.waiting_lights = lights;
        Ok(())
    }

    /// Opens `coord`'s job from its listed faces (and whether its masks were prepared): its
    /// place allocated, its count the coarse pass's for a chunk of `class` 0 (no block) beyond
    /// [`BakeQueue::near`] of `eye` (chunks), else the settings'. A chunk with no face is marked
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
        self.reach.remove(&c);
        self.stats.prepared += u64::from(prepared);
        if let Some(m) = self.prepared.remove(&c) {
            self.prepared_bytes -= m.masks.bytes();
        }
        if faces.faces() == 0 {
            self.faceless.insert(c);
            return Ok(0);
        }
        let tier = Some(Tier::Brick);
        let record = self
            .settings
            .rings
            .as_ref()
            .map_or(BRICK_WORDS, Rings::record_words);
        let words = layout(&faces, tier, record).1;
        let first_word = store.alloc(words)?;
        let far = (0..3).map(|k| (c[k] - eye[k]).abs()).max().unwrap_or(0) > self.near;
        let coarse = class == 0 && self.coarse_samples > 0 && far;
        let brick_samples = if coarse {
            self.coarse_samples
        } else {
            self.settings
                .rings
                .as_ref()
                .map_or(BRICK_LEVEL_SAMPLES, |r| r.brick_samples)
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
            kept: None,
            tier,
            kept_bricks: KeptBricks::new(),
            samples,
            brick_samples: coarse.then_some(brick_samples),
        });
        self.job_coarse.insert(c, coarse);
        Ok(samples)
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
    let gpu = run.gpu;
    if resident.is_none() && gpu.pending_chunks() == 0 {
        return true;
    }
    let r = lr + 1;
    let rr = i64::from(lr).pow(2);
    for z in -r..=r {
        for y in -r..=r {
            for x in -r..=r {
                let gap2: i64 = [x, y, z]
                    .iter()
                    .map(|&d| i64::from((d.abs() - 1).max(0)).pow(2))
                    .sum();
                if gap2 >= rr {
                    continue;
                }
                let n = ChunkCoord([coord.0[0] + x, coord.0[1] + y, coord.0[2] + z]);
                if gpu.is_pending(n) {
                    return false;
                }
                if let Some(res) = resident
                    && !(res(n) && gpu.is_mirrored(n) && !gpu.is_lod(n))
                {
                    return false;
                }
            }
        }
    }
    true
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
