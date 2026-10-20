//! Edited-chunk persistence (ADR-0003): an evicted chunk that was ever edited is kept in RAM
//! as it is, and past a RAM cap its oldest entries spill to one file per chunk.
//!
//! The spill file is a cache for one process, not a save format, and makes no compatibility
//! promise across engine versions. Layout, little-endian:
//!
//! ```text
//! magic     8 B   "VXECHNK\0"
//! version   u32   SPILL_VERSION
//! world key u64   caller-supplied, so a later world never reads an earlier world's files
//! coord     3 x i32
//! bricks    512 x (tag u8, then the brick)
//! trailer   u64   FNV-1a 64 over every byte before it
//! ```
//!
//! A brick's tag is 0 for `Uniform`, followed by its `u16` material; 1 to 5 for a `Full`
//! brick's packed class (`Mask`, `Plane1`, `Plane2`, `Plane4`, `Wide`; ADR-0012), followed by
//! the class's `u64` words (72, 136, 200 or 352 B) or, for `Wide`, its 512 `u16` ids
//! x-fastest. Decoding checks each box's canonical form and reports a non-canonical one as
//! `Corrupt`.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::brick::Brick;
use crate::chunk::Chunk;
use crate::coords::{BRICK_VOXELS, CHUNK_BRICKS, ChunkCoord};
use crate::hash::Fnv1a;
use crate::material::{AIR, MaterialId};
use crate::packed::{Packed, PackedClass};

const MAGIC: &[u8; 8] = b"VXECHNK\0";
/// Format version of the spill file.
pub const SPILL_VERSION: u32 = 2;
const HEADER: usize = 8 + 4 + 8 + 12;

/// Where edited chunks go when they leave residency.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersistConfig {
    /// Bytes of persisted chunks held in RAM before the oldest spill to `spill_dir`.
    pub ram_cap_bytes: u64,
    /// Directory for spill files; without one, persisted chunks stay in RAM past the cap.
    pub spill_dir: Option<PathBuf>,
    /// Written into every spill file and checked on reload.
    pub world_key: u64,
}

impl Default for PersistConfig {
    /// A 1 GiB RAM cap, no spill directory, world key 0.
    fn default() -> Self {
        Self {
            ram_cap_bytes: 1 << 30,
            spill_dir: None,
            world_key: 0,
        }
    }
}

/// Why a spill file could not be written or read back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpillError {
    /// The file system refused a read, write, rename or delete.
    Io,
    /// The trailer, magic, coordinate or length does not check out.
    Corrupt,
    /// The file was written under another world key.
    WorldMismatch,
    /// The file was written by another format version.
    Version,
}

/// Encodes `chunk` at `coord` as a spill file's bytes.
pub fn encode_spill(coord: ChunkCoord, chunk: &Chunk, world_key: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER + CHUNK_BRICKS * 3 + 8);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&SPILL_VERSION.to_le_bytes());
    out.extend_from_slice(&world_key.to_le_bytes());
    for v in coord.0 {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for b in chunk.bricks() {
        match b {
            Brick::Uniform(m) => {
                out.push(0);
                out.extend_from_slice(&m.to_le_bytes());
            }
            Brick::Full(p) => {
                let tag = PackedClass::ALL
                    .iter()
                    .position(|&c| c == p.class())
                    .expect("one of the five classes");
                out.push(1 + tag as u8);
                match (p.words(), p.wide_ids()) {
                    (Some(w), _) => w
                        .iter()
                        .for_each(|x| out.extend_from_slice(&x.to_le_bytes())),
                    (None, Some(v)) => v
                        .iter()
                        .for_each(|m| out.extend_from_slice(&m.to_le_bytes())),
                    (None, None) => unreachable!("a box is planar or wide"),
                }
            }
        }
    }
    let mut h = Fnv1a::default();
    h.write(&out);
    out.extend_from_slice(&h.finish().to_le_bytes());
    out
}

/// Decodes a spill file written under `world_key`. The chunk comes back edited, with every
/// slot dirty.
pub fn decode_spill(bytes: &[u8], world_key: u64) -> Result<(ChunkCoord, Chunk), SpillError> {
    if bytes.len() < HEADER + 8 {
        return Err(SpillError::Corrupt);
    }
    let (body, trailer) = bytes.split_at(bytes.len() - 8);
    let mut h = Fnv1a::default();
    h.write(body);
    if h.finish().to_le_bytes() != trailer {
        return Err(SpillError::Corrupt);
    }
    if &body[..8] != MAGIC {
        return Err(SpillError::Corrupt);
    }
    let u32_at = |i: usize| u32::from_le_bytes([body[i], body[i + 1], body[i + 2], body[i + 3]]);
    if u32_at(8) != SPILL_VERSION {
        return Err(SpillError::Version);
    }
    let mut key = [0; 8];
    key.copy_from_slice(&body[12..20]);
    if u64::from_le_bytes(key) != world_key {
        return Err(SpillError::WorldMismatch);
    }
    let coord = ChunkCoord([20, 24, 28].map(|i| u32_at(i) as i32));
    let mut chunk = Chunk::default();
    let mut at = HEADER;
    let u16_at = |at: &mut usize| -> Result<MaterialId, SpillError> {
        let b = body.get(*at..*at + 2).ok_or(SpillError::Corrupt)?;
        *at += 2;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    };
    for slot in 0..CHUNK_BRICKS {
        let tag = *body.get(at).ok_or(SpillError::Corrupt)?;
        at += 1;
        let brick = match tag {
            0 => Brick::Uniform(u16_at(&mut at)?),
            1..=5 => {
                let class = PackedClass::ALL[usize::from(tag) - 1];
                let (mut words, mut ids) = (Vec::new(), Vec::new());
                if class == PackedClass::Wide {
                    for _ in 0..BRICK_VOXELS {
                        ids.push(u16_at(&mut at)?);
                    }
                } else {
                    for _ in 0..class.bytes() / 8 {
                        let b = body.get(at..at + 8).ok_or(SpillError::Corrupt)?;
                        words.push(u64::from_le_bytes(b.try_into().expect("8 bytes")));
                        at += 8;
                    }
                }
                let p = Packed::from_parts(class, &words, &ids).ok_or(SpillError::Corrupt)?;
                let b = Brick::Full(p);
                // A box of one material over every voxel is a `Uniform` brick.
                let mut v = Box::new([AIR; BRICK_VOXELS]);
                b.decode(&mut v);
                if Brick::from_voxels(v) != b {
                    return Err(SpillError::Corrupt);
                }
                b
            }
            _ => return Err(SpillError::Corrupt),
        };
        chunk.set_brick(slot, brick);
    }
    if at != body.len() {
        return Err(SpillError::Corrupt);
    }
    chunk.mark_edited();
    Ok((coord, chunk))
}

/// Bytes a persisted chunk holds in RAM by this module's accounting: the slot array and the
/// class boxes of its `Full` bricks.
pub(crate) fn chunk_bytes(chunk: &Chunk) -> u64 {
    let payload: usize = chunk.bricks().iter().map(Brick::payload_bytes).sum();
    (size_of::<Chunk>() + payload) as u64
}

pub(crate) fn spill_path(dir: &Path, coord: ChunkCoord, seq: u64) -> PathBuf {
    let [x, y, z] = coord.0;
    dir.join(format!("chunk_{x}_{y}_{z}_{seq}.vxs"))
}

/// Writes `bytes` to `path` through a temporary file and a rename, so a reader never sees a
/// half-written file.
pub(crate) fn write_file(path: &Path, bytes: &[u8]) -> Result<(), SpillError> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).map_err(|_| SpillError::Io)?;
    std::fs::rename(&tmp, path).map_err(|_| {
        let _ = std::fs::remove_file(&tmp);
        SpillError::Io
    })
}

/// One chunk persisted in RAM.
pub(crate) struct Entry {
    pub seq: u64,
    pub chunk: Arc<RwLock<Chunk>>,
    pub bytes: u64,
}

/// The persistence map, shared between the streamer and its workers behind one mutex.
#[derive(Default)]
pub(crate) struct Persisted {
    pub ram: HashMap<ChunkCoord, Entry>,
    /// RAM entries by eviction sequence number, oldest first.
    pub by_seq: BTreeMap<u64, ChunkCoord>,
    pub spilled: HashMap<ChunkCoord, PathBuf>,
    pub ram_bytes: u64,
    next_seq: u64,
}

impl Persisted {
    /// Keeps `chunk` in RAM under the next sequence number.
    pub fn insert(&mut self, coord: ChunkCoord, chunk: Arc<RwLock<Chunk>>) {
        let bytes = chunk_bytes(&crate::store::read(&chunk));
        let seq = self.next_seq;
        self.next_seq += 1;
        if let Some(old) = self.ram.insert(coord, Entry { seq, chunk, bytes }) {
            self.by_seq.remove(&old.seq);
            self.ram_bytes -= old.bytes;
        }
        self.by_seq.insert(seq, coord);
        self.ram_bytes += bytes;
    }

    /// Takes `coord`'s RAM entry out.
    pub fn take(&mut self, coord: ChunkCoord) -> Option<Entry> {
        let e = self.ram.remove(&coord)?;
        self.by_seq.remove(&e.seq);
        self.ram_bytes -= e.bytes;
        Some(e)
    }
}
