//! The op log, the input log and the log file: the world's history as bytes (ADR-0007).
//!
//! An op entry is `tick: u64`, `seq: u32`, then the op as `wire` encodes it. The collapse
//! kinds' entries, for `n` cells:
//!
//! | kind | op                                | entry bytes |
//! |------|-----------------------------------|-------------|
//! | 3    | drop (yield threshold 0)          | `21 + 12n`  |
//! | 4    | remove with rubble                | `31 + 12n`  |
//! | 5    | drop through a yield threshold    | `23 + 12n`  |
//! | 6    | rubble pile                       | `27 + 12n`  |
//!
//! A build that predates kinds 5 and 6 refuses an entry of either with
//! [`WireError::UnknownKind`] at that entry, not at the header. A boundary entry
//! is `tick: u64`, `seq: u32`, then the kind [`KIND_BOUNDARY`]: 13 bytes. An input entry
//! is `tick: u64`, `client: u32`, `len: u16`, then `len` opaque bytes. A log encodes as its
//! entries back to back with no count; the log file's header carries the counts.
//!
//! A log file is a header, then the op entries, then the input entries. The header is 60
//! fixed bytes, then (from format 2) the prefab table:
//!
//! | bytes | field                                   |
//! |-------|-----------------------------------------|
//! | 4     | magic `VXEL`                            |
//! | 4     | format `u32`, [`LOG_FORMAT`]            |
//! | 8     | seed `u64`                              |
//! | 4     | source version `u32`                    |
//! | 24    | extent min then max, 6 × `i32`          |
//! | 8     | op log entry count `u64`, boundaries in |
//! | 8     | input entry count `u64`                 |
//! | 4     | prefab count `u32` (format 2)           |
//! | ...   | per prefab: id `u64`, then its bytes as [`Prefab::encode`] writes them (format 2) |
//!
//! The table holds the prefabs the log's stamps name (ADR-0014); decoding re-hashes each one
//! and refuses a file whose content does not hash to its id. A format-1 file has no table.

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::sync::Arc;

use crate::coarse::CellCoord;
use crate::collapse::{DropResult, PileResult, RemoveResult};
use crate::coords::{ChunkCoord, ChunkExtent, VoxelCoord};
use crate::edit::{EditError, EditOp, EditResult};
use crate::material::MaterialId;
use crate::prefab::{Prefab, PrefabRegistry, Rot};
use crate::store::World;
use crate::wire::{KIND_BOUNDARY, Reader, WireError, decode_op, encode_op, put_i32x3};

/// Magic at the start of a log file.
pub const LOG_MAGIC: [u8; 4] = *b"VXEL";
/// The log file format this build writes. It also reads format 1, which has no prefab table.
pub const LOG_FORMAT: u32 = 2;
/// Bytes in a log file header's fixed part; from format 2 the prefab table follows it.
pub const LOG_HEADER_BYTES: usize = 60;

/// One change to the world, as logged and announced.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Op {
    /// A shape edit ([`World::apply`]).
    Edit(EditOp),
    /// The prefab with content id `prefab`, turned by `rot`, its minimum corner at `origin`
    /// ([`World::stamp`]).
    Stamp {
        prefab: u64,
        origin: VoxelCoord,
        rot: Rot,
    },
    /// A rigid drop of the cells' solid voxels by at most `max_drop`, through the voxels of
    /// cells holding fewer than `yield_below` solid voxels ([`World::drop_region_through`];
    /// `yield_below == 0` is [`World::drop_region`]). Wire kind 3 when `yield_below == 0`,
    /// kind 5 otherwise.
    Drop {
        cells: Vec<CellCoord>,
        max_drop: i32,
        yield_below: u16,
    },
    /// The cells' voxels carved, with `fraction` of them piled as `rubble` on the ground
    /// within `max_depth` below ([`World::remove_region`]).
    Remove {
        cells: Vec<CellCoord>,
        rubble: MaterialId,
        fraction: (u32, u32),
        max_depth: i32,
    },
    /// `count` voxels of `rubble` spread over the cells' footprint columns, on the ground
    /// within `max_depth` below the top of each ([`World::pile_rubble`]). Wire kind 6.
    Pile {
        cells: Vec<CellCoord>,
        rubble: MaterialId,
        count: u32,
        max_depth: i32,
    },
}

impl Op {
    /// Applies the op to `world`, resolving a stamp's prefab in `prefabs`; a stamp of an id
    /// not there is refused with [`EditError::UnknownPrefab`] and changes nothing.
    pub fn apply(&self, world: &World, prefabs: &PrefabRegistry) -> Result<OpResult, EditError> {
        match self {
            Self::Edit(e) => world.apply(e).map(OpResult::Edit),
            Self::Stamp {
                prefab,
                origin,
                rot,
            } => {
                let p = prefabs
                    .get(*prefab)
                    .ok_or(EditError::UnknownPrefab(*prefab))?;
                world.stamp(p, *origin, *rot).map(OpResult::Edit)
            }
            Self::Drop {
                cells,
                max_drop,
                yield_below,
            } => world
                .drop_region_through(cells, *max_drop, *yield_below)
                .map(OpResult::Drop),
            Self::Remove {
                cells,
                rubble,
                fraction,
                max_depth,
            } => world
                .remove_region(cells, *rubble, *fraction, *max_depth)
                .map(OpResult::Remove),
            Self::Pile {
                cells,
                rubble,
                count,
                max_depth,
            } => world
                .pile_rubble(cells, *rubble, *count, *max_depth)
                .map(OpResult::Pile),
        }
    }
}

/// What an applied op changed: an edit's or a stamp's [`EditResult`], or a collapse op's
/// own result with the fields an edit result lacks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpResult {
    Edit(EditResult),
    Drop(DropResult),
    Remove(RemoveResult),
    Pile(PileResult),
}

impl OpResult {
    /// The result in the edit-result vocabulary (per-material counts, touched chunks,
    /// aabb), for [`crate::CoarseGrid::update`] and anything else that reads only those.
    pub fn edit_result(&self) -> EditResult {
        match self {
            Self::Edit(e) => e.clone(),
            Self::Drop(d) => d.edit_result(),
            Self::Remove(r) => r.edit_result(),
            Self::Pile(p) => p.edit_result(),
        }
    }

    /// Inclusive bounds of the changed voxels; `None` when nothing changed.
    pub fn aabb(&self) -> Option<(VoxelCoord, VoxelCoord)> {
        match self {
            Self::Edit(e) => e.aabb,
            Self::Drop(d) => d.aabb,
            Self::Remove(r) => r.aabb,
            Self::Pile(p) => p.aabb,
        }
    }

    /// The edit result of an edit or a stamp.
    pub fn as_edit(&self) -> Option<&EditResult> {
        match self {
            Self::Edit(e) => Some(e),
            _ => None,
        }
    }
}

impl From<EditResult> for OpResult {
    fn from(r: EditResult) -> Self {
        Self::Edit(r)
    }
}

impl From<EditOp> for Op {
    fn from(e: EditOp) -> Self {
        Self::Edit(e)
    }
}

/// One op log entry: an applied op, or the boundary a server logs after the directly
/// submitted ops of a tick. A boundary's seq is the first one left to the logic in that tick;
/// re-simulation applies the tick's op entries below it before running the logic.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum LogEntry {
    Op { tick: u64, seq: u32, op: Op },
    Boundary { tick: u64, seq: u32 },
}

impl LogEntry {
    pub fn tick(&self) -> u64 {
        match *self {
            Self::Op { tick, .. } | Self::Boundary { tick, .. } => tick,
        }
    }

    pub fn seq(&self) -> u32 {
        match *self {
            Self::Op { seq, .. } | Self::Boundary { seq, .. } => seq,
        }
    }

    /// The op of an op entry; `None` for a boundary.
    pub fn op(&self) -> Option<&Op> {
        match self {
            Self::Op { op, .. } => Some(op),
            Self::Boundary { .. } => None,
        }
    }

    /// Appends the entry's bytes: tick, seq, then the op (whose first byte is its kind) or the
    /// boundary kind [`KIND_BOUNDARY`] alone.
    pub fn encode(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.tick().to_le_bytes());
        out.extend_from_slice(&self.seq().to_le_bytes());
        match self {
            Self::Op { op, .. } => encode_op(out, op),
            Self::Boundary { .. } => out.push(KIND_BOUNDARY),
        }
    }

    /// Reads one entry; a short read reports the entry's start offset.
    pub fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        r.begin();
        let tick = r.u64()?;
        let seq = r.u32()?;
        if r.peek_u8()? == KIND_BOUNDARY {
            r.u8()?;
            return Ok(Self::Boundary { tick, seq });
        }
        Ok(Self::Op {
            tick,
            seq,
            op: decode_op(r)?,
        })
    }
}

/// Applied ops and boundaries in ascending `seq`. Seqs may have gaps: a server assigns a seq
/// to every op it is asked to apply and logs only the ones that applied.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OpLog {
    entries: Vec<LogEntry>,
    /// Op entries among `entries`, boundaries excluded.
    ops: usize,
}

impl OpLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends `op` at `tick` with the seq after the last entry's (0 for an empty log), and
    /// returns that seq.
    pub fn push(&mut self, tick: u64, op: impl Into<Op>) -> u32 {
        let seq = self.entries.last().map_or(0, |e| e.seq() + 1);
        self.record(tick, seq, op);
        seq
    }

    /// Appends an op entry with a seq the caller assigned. Panics when `seq` does not exceed
    /// the last entry's: the log's order is the order of truth and is never rewritten.
    pub fn record(&mut self, tick: u64, seq: u32, op: impl Into<Op>) {
        self.append(LogEntry::Op {
            tick,
            seq,
            op: op.into(),
        });
        self.ops += 1;
    }

    /// Appends a boundary with a seq the caller assigned; panics like [`OpLog::record`].
    pub fn record_boundary(&mut self, tick: u64, seq: u32) {
        self.append(LogEntry::Boundary { tick, seq });
    }

    fn append(&mut self, e: LogEntry) {
        if let Some(last) = self.entries.last() {
            assert!(
                e.seq() > last.seq(),
                "op log seqs ascend: {} after {}",
                e.seq(),
                last.seq()
            );
        }
        self.entries.push(e);
    }

    /// Every entry, boundaries included.
    pub fn entries(&self) -> &[LogEntry] {
        &self.entries
    }

    /// Op entries, boundaries excluded.
    pub fn len(&self) -> usize {
        self.ops
    }

    /// True when the log holds no op entry (it may hold boundaries).
    pub fn is_empty(&self) -> bool {
        self.ops == 0
    }

    /// The entries' bytes, back to back.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for e in &self.entries {
            e.encode(&mut out);
        }
        out
    }

    /// The prefab ids the log's stamps name, ascending.
    pub fn prefab_ids(&self) -> BTreeSet<u64> {
        self.entries
            .iter()
            .filter_map(|e| match e.op() {
                Some(Op::Stamp { prefab, .. }) => Some(*prefab),
                _ => None,
            })
            .collect()
    }

    /// Reads entries until the bytes end. Seqs must strictly ascend across ops and
    /// boundaries, as [`OpLog::record`] enforces; otherwise [`WireError::OutOfOrder`].
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut r = Reader::new(bytes);
        let mut log = Self::new();
        while !r.is_empty() {
            log.decode_entry(&mut r)?;
        }
        Ok(log)
    }

    fn decode_n(r: &mut Reader<'_>, n: u64) -> Result<Self, WireError> {
        let mut log = Self::new();
        for _ in 0..n {
            log.decode_entry(r)?;
        }
        Ok(log)
    }

    /// Decodes one entry and appends it, refusing a seq that does not exceed the last one.
    fn decode_entry(&mut self, r: &mut Reader<'_>) -> Result<(), WireError> {
        let at = r.pos();
        let e = LogEntry::decode(r)?;
        if self
            .entries
            .last()
            .is_some_and(|last| e.seq() <= last.seq())
        {
            return Err(WireError::OutOfOrder { at });
        }
        if e.op().is_some() {
            self.ops += 1;
        }
        self.entries.push(e);
        Ok(())
    }
}

/// One client input, opaque to the engine, and the tick it was consumed in.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct InputEntry {
    pub tick: u64,
    pub client: u32,
    pub bytes: Vec<u8>,
}

impl InputEntry {
    /// Appends the entry's bytes. Panics when the payload exceeds `u16::MAX` bytes; the
    /// server's input message carries a `u16` length, so no consumed input can.
    pub fn encode(&self, out: &mut Vec<u8>) {
        let len = u16::try_from(self.bytes.len()).expect("input payloads fit u16");
        out.extend_from_slice(&self.tick.to_le_bytes());
        out.extend_from_slice(&self.client.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&self.bytes);
    }

    /// Reads one entry; a short read, including a `len` past the end, reports the entry's
    /// start offset.
    pub fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        r.begin();
        let tick = r.u64()?;
        let client = r.u32()?;
        let len = r.u16()?;
        Ok(Self {
            tick,
            client,
            bytes: r.take(usize::from(len))?.to_vec(),
        })
    }
}

/// Consumed client inputs in the order the server consumed them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InputLog {
    entries: Vec<InputEntry>,
}

impl InputLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends an input consumed at `tick` from `client`. Panics when `bytes` exceeds
    /// `u16::MAX` bytes (see [`InputEntry::encode`]).
    pub fn push(&mut self, tick: u64, client: u32, bytes: Vec<u8>) {
        assert!(
            bytes.len() <= usize::from(u16::MAX),
            "input payloads fit u16"
        );
        self.entries.push(InputEntry {
            tick,
            client,
            bytes,
        });
    }

    pub fn entries(&self) -> &[InputEntry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The entries' bytes, back to back.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for e in &self.entries {
            e.encode(&mut out);
        }
        out
    }

    /// Reads entries until the bytes end.
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut r = Reader::new(bytes);
        let mut entries = Vec::new();
        while !r.is_empty() {
            entries.push(InputEntry::decode(&mut r)?);
        }
        Ok(Self { entries })
    }

    fn decode_n(r: &mut Reader<'_>, n: u64) -> Result<Self, WireError> {
        let mut entries = Vec::new();
        for _ in 0..n {
            entries.push(InputEntry::decode(r)?);
        }
        Ok(Self { entries })
    }
}

/// A save: what to regenerate the world from, and the history to replay onto it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogFile {
    /// The seed of the world source and of the logic.
    pub seed: u64,
    /// The world source's own version; a different source makes the log meaningless.
    pub source_version: u32,
    pub extent: ChunkExtent,
    pub ops: OpLog,
    pub inputs: InputLog,
    /// The prefab table: the prefabs the op log's stamps name, so a replay resolves them
    /// without the application's assets ([`OpLog::prefab_ids`], [`PrefabRegistry::subset`]).
    pub prefabs: PrefabRegistry,
}

impl LogFile {
    /// Bytes of the header: the fixed part and the prefab table.
    pub fn header_len(&self) -> usize {
        LOG_HEADER_BYTES
            + 4
            + self
                .prefabs
                .iter()
                .map(|(_, p)| 8 + p.encoded_len())
                .sum::<usize>()
    }

    /// The file's bytes: header, op entries, input entries.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.header_len());
        out.extend_from_slice(&LOG_MAGIC);
        out.extend_from_slice(&LOG_FORMAT.to_le_bytes());
        out.extend_from_slice(&self.seed.to_le_bytes());
        out.extend_from_slice(&self.source_version.to_le_bytes());
        put_i32x3(&mut out, self.extent.min.0);
        put_i32x3(&mut out, self.extent.max.0);
        // Every entry, boundaries included: the decoder reads this many.
        out.extend_from_slice(&(self.ops.entries().len() as u64).to_le_bytes());
        out.extend_from_slice(&(self.inputs.len() as u64).to_le_bytes());
        debug_assert_eq!(out.len(), LOG_HEADER_BYTES);
        let n = u32::try_from(self.prefabs.len()).expect("prefab table counts fit u32");
        out.extend_from_slice(&n.to_le_bytes());
        for (id, p) in self.prefabs.iter() {
            out.extend_from_slice(&id.to_le_bytes());
            p.encode(&mut out);
        }
        debug_assert_eq!(out.len(), self.header_len());
        for e in self.ops.entries() {
            e.encode(&mut out);
        }
        for e in self.inputs.entries() {
            e.encode(&mut out);
        }
        out
    }

    /// Reads a whole file's bytes, of format 1 or [`LOG_FORMAT`]. Bytes after the last
    /// counted entry are an error; a table entry whose content does not hash to its id is
    /// [`WireError::PrefabMismatch`].
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut r = Reader::new(bytes);
        if r.take(4)? != LOG_MAGIC {
            return Err(WireError::BadMagic);
        }
        let format = r.u32()?;
        if format != 1 && format != LOG_FORMAT {
            return Err(WireError::UnsupportedFormat(format));
        }
        let seed = r.u64()?;
        let source_version = r.u32()?;
        let extent = ChunkExtent::new(ChunkCoord(r.i32x3()?), ChunkCoord(r.i32x3()?));
        let n_ops = r.u64()?;
        let n_inputs = r.u64()?;
        let mut prefabs = PrefabRegistry::new();
        if format >= 2 {
            for _ in 0..r.u32()? {
                let id = r.u64()?;
                let p = Prefab::decode(&mut r)?;
                if p.id() != id {
                    return Err(WireError::PrefabMismatch { id });
                }
                prefabs
                    .register(Arc::new(p))
                    .map_err(|e| WireError::PrefabMismatch { id: e.id })?;
            }
        }
        let ops = OpLog::decode_n(&mut r, n_ops)?;
        let inputs = InputLog::decode_n(&mut r, n_inputs)?;
        if !r.is_empty() {
            return Err(WireError::Trailing { at: r.pos() });
        }
        Ok(Self {
            seed,
            source_version,
            extent,
            ops,
            inputs,
            prefabs,
        })
    }

    pub fn write_to(&self, w: &mut impl Write) -> std::io::Result<()> {
        w.write_all(&self.encode())
    }

    /// Reads the stream to its end and decodes it as one file.
    pub fn read_from(r: &mut impl Read) -> Result<Self, WireError> {
        let mut bytes = Vec::new();
        r.read_to_end(&mut bytes)?;
        Self::decode(&bytes)
    }
}

impl World {
    /// Applies the log's op entries in `seq` order, resolving stamps in `prefabs`, and
    /// returns each result, in that order. Boundaries are skipped and have no result.
    pub fn replay(
        &self,
        log: &OpLog,
        prefabs: &PrefabRegistry,
    ) -> Vec<Result<OpResult, EditError>> {
        let mut entries: Vec<(u32, &Op)> = log
            .entries()
            .iter()
            .filter_map(|e| e.op().map(|op| (e.seq(), op)))
            .collect();
        entries.sort_by_key(|(seq, _)| *seq);
        entries
            .iter()
            .map(|(_, op)| op.apply(self, prefabs))
            .collect()
    }
}
