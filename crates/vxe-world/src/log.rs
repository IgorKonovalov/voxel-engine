//! The op log, the input log and the log file: the world's history as bytes (ADR-0007).
//!
//! An op entry is `tick: u64`, `seq: u32`, then the op as `wire` encodes it. An input entry
//! is `tick: u64`, `client: u32`, `len: u16`, then `len` opaque bytes. A log encodes as its
//! entries back to back with no count; the log file's header carries the counts.
//!
//! A log file is a 60-byte header, then the op entries, then the input entries:
//!
//! | bytes | field                                   |
//! |-------|-----------------------------------------|
//! | 4     | magic `VXEL`                            |
//! | 4     | format `u32`, [`LOG_FORMAT`]            |
//! | 8     | seed `u64`                              |
//! | 4     | source version `u32`                    |
//! | 24    | extent min then max, 6 × `i32`          |
//! | 8     | op entry count `u64`                    |
//! | 8     | input entry count `u64`                 |

use std::io::{Read, Write};

use crate::coords::{ChunkCoord, ChunkExtent};
use crate::edit::{EditError, EditOp, EditResult};
use crate::store::World;
use crate::wire::{Reader, WireError, decode_op, encode_op, put_i32x3};

/// Magic at the start of a log file.
pub const LOG_MAGIC: [u8; 4] = *b"VXEL";
/// The log file format this build writes and reads.
pub const LOG_FORMAT: u32 = 1;
/// Bytes in a log file's header.
pub const LOG_HEADER_BYTES: usize = 60;

/// One change to the world, as logged and announced. Kinds 2 to 4 of the wire format are
/// reserved for later variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Op {
    Edit(EditOp),
}

impl Op {
    /// Applies the op to `world`.
    pub fn apply(&self, world: &World) -> Result<EditResult, EditError> {
        match self {
            Self::Edit(e) => world.apply(e),
        }
    }
}

impl From<EditOp> for Op {
    fn from(e: EditOp) -> Self {
        Self::Edit(e)
    }
}

/// One applied op: the tick it ran in and its sequence number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LogEntry {
    pub tick: u64,
    pub seq: u32,
    pub op: Op,
}

impl LogEntry {
    /// Appends the entry's bytes.
    pub fn encode(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.tick.to_le_bytes());
        out.extend_from_slice(&self.seq.to_le_bytes());
        encode_op(out, &self.op);
    }

    /// Reads one entry; a short read reports the entry's start offset.
    pub fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        r.begin();
        Ok(Self {
            tick: r.u64()?,
            seq: r.u32()?,
            op: decode_op(r)?,
        })
    }
}

/// Applied ops in ascending `seq`. Seqs may have gaps: a server assigns a seq to every op it
/// is asked to apply and logs only the ones that applied.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OpLog {
    entries: Vec<LogEntry>,
}

impl OpLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends `op` at `tick` with the seq after the last entry's (0 for an empty log), and
    /// returns that seq.
    pub fn push(&mut self, tick: u64, op: impl Into<Op>) -> u32 {
        let seq = self.entries.last().map_or(0, |e| e.seq + 1);
        self.entries.push(LogEntry {
            tick,
            seq,
            op: op.into(),
        });
        seq
    }

    /// Appends an entry with a seq the caller assigned. Panics when `seq` does not exceed the
    /// last entry's: the log's order is the order of truth and is never rewritten.
    pub fn record(&mut self, tick: u64, seq: u32, op: impl Into<Op>) {
        if let Some(last) = self.entries.last() {
            assert!(
                seq > last.seq,
                "op log seqs ascend: {seq} after {}",
                last.seq
            );
        }
        self.entries.push(LogEntry {
            tick,
            seq,
            op: op.into(),
        });
    }

    pub fn entries(&self) -> &[LogEntry] {
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
            entries.push(LogEntry::decode(&mut r)?);
        }
        Ok(Self { entries })
    }

    fn decode_n(r: &mut Reader<'_>, n: u64) -> Result<Self, WireError> {
        let mut entries = Vec::new();
        for _ in 0..n {
            entries.push(LogEntry::decode(r)?);
        }
        Ok(Self { entries })
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
}

impl LogFile {
    /// The file's bytes: header, op entries, input entries.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(LOG_HEADER_BYTES);
        out.extend_from_slice(&LOG_MAGIC);
        out.extend_from_slice(&LOG_FORMAT.to_le_bytes());
        out.extend_from_slice(&self.seed.to_le_bytes());
        out.extend_from_slice(&self.source_version.to_le_bytes());
        put_i32x3(&mut out, self.extent.min.0);
        put_i32x3(&mut out, self.extent.max.0);
        out.extend_from_slice(&(self.ops.len() as u64).to_le_bytes());
        out.extend_from_slice(&(self.inputs.len() as u64).to_le_bytes());
        debug_assert_eq!(out.len(), LOG_HEADER_BYTES);
        for e in self.ops.entries() {
            e.encode(&mut out);
        }
        for e in self.inputs.entries() {
            e.encode(&mut out);
        }
        out
    }

    /// Reads a whole file's bytes. Bytes after the last counted entry are an error.
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut r = Reader::new(bytes);
        if r.take(4)? != LOG_MAGIC {
            return Err(WireError::BadMagic);
        }
        let format = r.u32()?;
        if format != LOG_FORMAT {
            return Err(WireError::UnsupportedFormat(format));
        }
        let seed = r.u64()?;
        let source_version = r.u32()?;
        let extent = ChunkExtent::new(ChunkCoord(r.i32x3()?), ChunkCoord(r.i32x3()?));
        let n_ops = r.u64()?;
        let n_inputs = r.u64()?;
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
    /// Applies the log's entries in `seq` order and returns each result, in that order.
    pub fn replay(&self, log: &OpLog) -> Vec<Result<EditResult, EditError>> {
        let mut entries: Vec<&LogEntry> = log.entries().iter().collect();
        entries.sort_by_key(|e| e.seq);
        entries.iter().map(|e| e.op.apply(self)).collect()
    }
}
