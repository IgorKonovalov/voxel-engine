//! The byte encoding of ops, op results and errors, shared by the logs, the save file and
//! the server's messages (ADR-0007).
//!
//! Little-endian throughout, no padding, no alignment, no varints. A `Fixed3` is 3 × `i64`, a
//! `ChunkCoord`, a `VoxelCoord` and a `CellCoord` 3 × `i32`, a result list count a `u16`, a
//! cell list count a `u32`. An op is its kind `u8`, then the kind's payload:
//!
//! | kind | op       | payload                                                              |
//! |------|----------|----------------------------------------------------------------------|
//! | 1    | edit     | mode `u8` and the mode's payload, shape `u8` and the shape's payload |
//! | 2    | stamp    | prefab id `u64`, origin `VoxelCoord`, rotation index `u8` (`Rot::all`) |
//! | 3    | drop     | max drop `i32`, cell count `u32`, the cells                          |
//! | 4    | remove   | rubble `u16`, fraction 2 × `u32`, max depth `i32`, cell count `u32`, the cells |
//!
//! An edit's modes and shapes:
//!
//! | mode      | payload           | shape     | payload                 |
//! |-----------|-------------------|-----------|-------------------------|
//! | `Carve`   | none              | sphere    | centre, radius: 4 × i64 |
//! | `Fill`    | material `u16`    | box       | min, max: 6 × i64       |
//! | `Replace` | from, to: 2 × u16 | capsule   | a, b, radius: 7 × i64   |
//!
//! An op result is a kind `u8` ([`OP_EDIT`] for an edit's or a stamp's, [`OP_DROP`],
//! [`OP_REMOVE`]) then the result ([`encode_op_result`]).
//!
//! Tag value 0 is never assigned, so a zeroed buffer never decodes as a valid op.

use crate::coarse::CellCoord;
use crate::collapse::{DropResult, RemoveResult};
use crate::coords::{ChunkCoord, VoxelCoord};
use crate::edit::{EditError, EditOp, EditResult, Mode};
use crate::fixed::Fixed3;
use crate::log::{Op, OpResult};
use crate::material::MaterialId;
use crate::prefab::Rot;
use crate::shape::Shape;

/// Op kind of an edit (shape plus mode).
pub const OP_EDIT: u8 = 1;
/// Op kind of a prefab stamp.
pub const OP_STAMP: u8 = 2;
/// Op kind of a region drop.
pub const OP_DROP: u8 = 3;
/// Op kind of a region removal with rubble.
pub const OP_REMOVE: u8 = 4;
/// Kind of an op log boundary entry, which has no payload. It sits clear of the op kinds so
/// later op kinds can be claimed below it.
pub const KIND_BOUNDARY: u8 = 0x80;

/// Mode tag of [`Mode::Carve`].
pub const MODE_CARVE: u8 = 1;
/// Mode tag of [`Mode::Fill`].
pub const MODE_FILL: u8 = 2;
/// Mode tag of [`Mode::Replace`].
pub const MODE_REPLACE: u8 = 3;

/// Shape tag of [`Shape::Sphere`].
pub const SHAPE_SPHERE: u8 = 1;
/// Shape tag of [`Shape::Box_`].
pub const SHAPE_BOX: u8 = 2;
/// Shape tag of [`Shape::Capsule`].
pub const SHAPE_CAPSULE: u8 = 3;

/// Error tag of [`EditError::NotResident`]: the chunk as 3 × `i32` follows.
pub const ERROR_NOT_RESIDENT: u8 = 1;
/// Error tag of [`EditError::UnknownPrefab`]: the id `u64` follows. Tag 2 is left to the
/// out-of-range refusal of shapes and queries.
pub const ERROR_UNKNOWN_PREFAB: u8 = 3;

/// Why bytes did not decode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireError {
    /// The bytes end inside the entry (or message) that starts at byte offset `at`.
    Truncated { at: usize },
    /// A kind, mode, shape, error or message tag with no meaning in this format.
    UnknownKind(u8),
    /// A log file that does not start with `VXEL`.
    BadMagic,
    /// A log file of a format version this build does not read.
    UnsupportedFormat(u32),
    /// Bytes left over after the last entry a log file's header counts, starting at `at`.
    Trailing { at: usize },
    /// An op log entry, starting at `at`, whose seq does not exceed the previous entry's.
    OutOfOrder { at: usize },
    /// A log file's prefab table holds content whose [`crate::Prefab::id`] is not the id
    /// the table gives it.
    PrefabMismatch { id: u64 },
    /// Reading or writing the underlying stream failed.
    Io(std::io::ErrorKind),
}

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated { at } => write!(f, "bytes end inside the entry at offset {at}"),
            Self::UnknownKind(k) => write!(f, "unknown tag {k}"),
            Self::BadMagic => write!(f, "not a voxel log file (bad magic)"),
            Self::UnsupportedFormat(v) => write!(f, "unsupported log format {v}"),
            Self::Trailing { at } => write!(f, "unexpected bytes after the last entry at {at}"),
            Self::OutOfOrder { at } => write!(f, "op log seq out of order at offset {at}"),
            Self::PrefabMismatch { id } => {
                write!(
                    f,
                    "the prefab table's content for id {id:#018x} does not hash to it"
                )
            }
            Self::Io(k) => write!(f, "i/o error: {k}"),
        }
    }
}

impl std::error::Error for WireError {}

impl From<std::io::Error> for WireError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.kind())
    }
}

/// A cursor over bytes being decoded. Every short read fails with
/// [`WireError::Truncated`] carrying the offset set by [`Reader::begin`], the start of the
/// entry being read, rather than the offset of the field that ran out.
#[derive(Clone, Debug)]
pub struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
    entry: usize,
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            pos: 0,
            entry: 0,
        }
    }

    /// Marks the current offset as the start of the next entry, for error reports.
    pub fn begin(&mut self) {
        self.entry = self.pos;
    }

    /// Byte offset of the entry being read, as [`Reader::begin`] set it.
    pub fn entry(&self) -> usize {
        self.entry
    }

    /// Bytes not yet read.
    pub fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.pos)
    }

    /// Byte offset of the next read.
    pub fn pos(&self) -> usize {
        self.pos
    }

    /// True when every byte has been read.
    pub fn is_empty(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    /// The next `n` bytes.
    pub fn take(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&e| e <= self.bytes.len())
            .ok_or(WireError::Truncated { at: self.entry })?;
        let s = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], WireError> {
        let mut a = [0; N];
        a.copy_from_slice(self.take(N)?);
        Ok(a)
    }

    /// The next byte, without consuming it.
    pub fn peek_u8(&self) -> Result<u8, WireError> {
        self.bytes
            .get(self.pos)
            .copied()
            .ok_or(WireError::Truncated { at: self.entry })
    }

    pub fn u8(&mut self) -> Result<u8, WireError> {
        Ok(self.array::<1>()?[0])
    }

    pub fn u16(&mut self) -> Result<u16, WireError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    pub fn u32(&mut self) -> Result<u32, WireError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    pub fn u64(&mut self) -> Result<u64, WireError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    pub fn i32(&mut self) -> Result<i32, WireError> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    pub fn i64(&mut self) -> Result<i64, WireError> {
        Ok(i64::from_le_bytes(self.array()?))
    }

    pub fn fixed3(&mut self) -> Result<Fixed3, WireError> {
        Ok(Fixed3([self.i64()?, self.i64()?, self.i64()?]))
    }

    pub fn i32x3(&mut self) -> Result<[i32; 3], WireError> {
        Ok([self.i32()?, self.i32()?, self.i32()?])
    }
}

/// Appends the little-endian bytes of three `i64`.
pub fn put_fixed3(out: &mut Vec<u8>, p: Fixed3) {
    for v in p.0 {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

/// Appends the little-endian bytes of three `i32`.
pub fn put_i32x3(out: &mut Vec<u8>, p: [i32; 3]) {
    for v in p {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

/// Appends a list count as `u16`. A list longer than `u16::MAX` is an invariant violation:
/// no edit result or payload the engine produces has one.
fn put_count(out: &mut Vec<u8>, n: usize) {
    let n = u16::try_from(n).expect("wire list counts fit u16");
    out.extend_from_slice(&n.to_le_bytes());
}

/// Appends a cell list: its count as `u32`, then each cell as 3 × `i32`. Panics past
/// `u32::MAX` cells.
fn put_cells(out: &mut Vec<u8>, cells: &[CellCoord]) {
    let n = u32::try_from(cells.len()).expect("wire cell lists fit u32");
    out.extend_from_slice(&n.to_le_bytes());
    for c in cells {
        put_i32x3(out, c.0);
    }
}

/// Reads a cell list as [`put_cells`] writes it. A count the bytes cannot hold is
/// [`WireError::Truncated`] before anything is allocated for it.
fn read_cells(r: &mut Reader<'_>) -> Result<Vec<CellCoord>, WireError> {
    let n = r.u32()? as usize;
    if n.saturating_mul(12) > r.remaining() {
        return Err(WireError::Truncated { at: r.entry() });
    }
    let mut cells = Vec::with_capacity(n);
    for _ in 0..n {
        cells.push(CellCoord(r.i32x3()?));
    }
    Ok(cells)
}

fn put_counts(out: &mut Vec<u8>, counts: &[(MaterialId, u32)]) {
    put_count(out, counts.len());
    for &(m, n) in counts {
        out.extend_from_slice(&m.to_le_bytes());
        out.extend_from_slice(&n.to_le_bytes());
    }
}

fn read_counts(r: &mut Reader<'_>) -> Result<Vec<(MaterialId, u32)>, WireError> {
    let n = r.u16()?;
    let mut counts = Vec::with_capacity(usize::from(n));
    for _ in 0..n {
        counts.push((r.u16()?, r.u32()?));
    }
    Ok(counts)
}

/// Appends the touched chunks and the aabb of a result: a `u16` count of chunk coordinates,
/// then a presence `u8` and, when 1, the aabb as 6 × `i32`.
fn put_extent(out: &mut Vec<u8>, touched: &[ChunkCoord], aabb: Option<(VoxelCoord, VoxelCoord)>) {
    put_count(out, touched.len());
    for c in touched {
        put_i32x3(out, c.0);
    }
    match aabb {
        None => out.push(0),
        Some((lo, hi)) => {
            out.push(1);
            put_i32x3(out, lo.0);
            put_i32x3(out, hi.0);
        }
    }
}

type Extent = (Vec<ChunkCoord>, Option<(VoxelCoord, VoxelCoord)>);

fn read_extent(r: &mut Reader<'_>) -> Result<Extent, WireError> {
    let n = r.u16()?;
    let mut touched = Vec::with_capacity(usize::from(n));
    for _ in 0..n {
        touched.push(ChunkCoord(r.i32x3()?));
    }
    let aabb = match r.u8()? {
        0 => None,
        1 => Some((VoxelCoord(r.i32x3()?), VoxelCoord(r.i32x3()?))),
        k => return Err(WireError::UnknownKind(k)),
    };
    Ok((touched, aabb))
}

/// Appends the encoding of `op`: its kind, then the kind's payload (see the module table).
pub fn encode_op(out: &mut Vec<u8>, op: &Op) {
    match op {
        Op::Edit(e) => {
            out.push(OP_EDIT);
            encode_edit(out, e);
        }
        Op::Stamp {
            prefab,
            origin,
            rot,
        } => {
            out.push(OP_STAMP);
            out.extend_from_slice(&prefab.to_le_bytes());
            put_i32x3(out, origin.0);
            out.push(rot.index());
        }
        Op::Drop { cells, max_drop } => {
            out.push(OP_DROP);
            out.extend_from_slice(&max_drop.to_le_bytes());
            put_cells(out, cells);
        }
        Op::Remove {
            cells,
            rubble,
            fraction,
            max_depth,
        } => {
            out.push(OP_REMOVE);
            out.extend_from_slice(&rubble.to_le_bytes());
            out.extend_from_slice(&fraction.0.to_le_bytes());
            out.extend_from_slice(&fraction.1.to_le_bytes());
            out.extend_from_slice(&max_depth.to_le_bytes());
            put_cells(out, cells);
        }
    }
}

/// An edit's mode and payload, then its shape and payload.
fn encode_edit(out: &mut Vec<u8>, e: &EditOp) {
    match e.mode {
        Mode::Carve => out.push(MODE_CARVE),
        Mode::Fill(m) => {
            out.push(MODE_FILL);
            out.extend_from_slice(&m.to_le_bytes());
        }
        Mode::Replace { from, to } => {
            out.push(MODE_REPLACE);
            out.extend_from_slice(&from.to_le_bytes());
            out.extend_from_slice(&to.to_le_bytes());
        }
    }
    match e.shape {
        Shape::Sphere { c, r } => {
            out.push(SHAPE_SPHERE);
            put_fixed3(out, c);
            out.extend_from_slice(&r.to_le_bytes());
        }
        Shape::Box_ { min, max } => {
            out.push(SHAPE_BOX);
            put_fixed3(out, min);
            put_fixed3(out, max);
        }
        Shape::Capsule { a, b, r } => {
            out.push(SHAPE_CAPSULE);
            put_fixed3(out, a);
            put_fixed3(out, b);
            out.extend_from_slice(&r.to_le_bytes());
        }
    }
}

/// Reads one op as [`encode_op`] writes it. A rotation index past 23 is
/// [`WireError::UnknownKind`].
pub fn decode_op(r: &mut Reader<'_>) -> Result<Op, WireError> {
    match r.u8()? {
        OP_EDIT => Ok(Op::Edit(decode_edit(r)?)),
        OP_STAMP => {
            let prefab = r.u64()?;
            let origin = VoxelCoord(r.i32x3()?);
            let i = r.u8()?;
            let rot = Rot::from_index(i).ok_or(WireError::UnknownKind(i))?;
            Ok(Op::Stamp {
                prefab,
                origin,
                rot,
            })
        }
        OP_DROP => {
            let max_drop = r.i32()?;
            Ok(Op::Drop {
                cells: read_cells(r)?,
                max_drop,
            })
        }
        OP_REMOVE => {
            let rubble = r.u16()?;
            let fraction = (r.u32()?, r.u32()?);
            let max_depth = r.i32()?;
            Ok(Op::Remove {
                cells: read_cells(r)?,
                rubble,
                fraction,
                max_depth,
            })
        }
        k => Err(WireError::UnknownKind(k)),
    }
}

fn decode_edit(r: &mut Reader<'_>) -> Result<EditOp, WireError> {
    let mode = match r.u8()? {
        MODE_CARVE => Mode::Carve,
        MODE_FILL => Mode::Fill(r.u16()?),
        MODE_REPLACE => Mode::Replace {
            from: r.u16()?,
            to: r.u16()?,
        },
        k => return Err(WireError::UnknownKind(k)),
    };
    let shape = match r.u8()? {
        SHAPE_SPHERE => Shape::Sphere {
            c: r.fixed3()?,
            r: r.i64()?,
        },
        SHAPE_BOX => Shape::Box_ {
            min: r.fixed3()?,
            max: r.fixed3()?,
        },
        SHAPE_CAPSULE => Shape::Capsule {
            a: r.fixed3()?,
            b: r.fixed3()?,
            r: r.i64()?,
        },
        k => return Err(WireError::UnknownKind(k)),
    };
    Ok(EditOp { shape, mode })
}

/// Appends an edit result: a `u16` count of `(material u16, voxels u32)` pairs, a `u16` count
/// of chunk coordinates, then a presence `u8` and, when 1, the aabb as 6 × `i32`.
pub fn encode_result(out: &mut Vec<u8>, r: &EditResult) {
    put_counts(out, &r.changed_from);
    put_extent(out, &r.touched, r.aabb);
}

/// Reads one edit result as [`encode_result`] writes it.
pub fn decode_result(r: &mut Reader<'_>) -> Result<EditResult, WireError> {
    let changed_from = read_counts(r)?;
    let (touched, aabb) = read_extent(r)?;
    Ok(EditResult {
        changed_from,
        touched,
        aabb,
    })
}

/// Appends an op result: its kind, then for [`OP_EDIT`] the edit result as
/// [`encode_result`] writes it; for [`OP_DROP`] the moved counts, `dropped` as `i32`, then the
/// touched chunks and the aabb; for [`OP_REMOVE`] the removed counts, `placed` as `u32`, then
/// the touched chunks and the aabb. Counts, chunks and the aabb are encoded as in an edit
/// result.
pub fn encode_op_result(out: &mut Vec<u8>, r: &OpResult) {
    match r {
        OpResult::Edit(e) => {
            out.push(OP_EDIT);
            encode_result(out, e);
        }
        OpResult::Drop(d) => {
            out.push(OP_DROP);
            put_counts(out, &d.moved);
            out.extend_from_slice(&d.dropped.to_le_bytes());
            put_extent(out, &d.touched, d.aabb);
        }
        OpResult::Remove(m) => {
            out.push(OP_REMOVE);
            put_counts(out, &m.removed);
            out.extend_from_slice(&m.placed.to_le_bytes());
            put_extent(out, &m.touched, m.aabb);
        }
    }
}

/// Reads one op result as [`encode_op_result`] writes it.
pub fn decode_op_result(r: &mut Reader<'_>) -> Result<OpResult, WireError> {
    match r.u8()? {
        OP_EDIT => Ok(OpResult::Edit(decode_result(r)?)),
        OP_DROP => {
            let moved = read_counts(r)?;
            let dropped = r.i32()?;
            let (touched, aabb) = read_extent(r)?;
            Ok(OpResult::Drop(DropResult {
                moved,
                dropped,
                touched,
                aabb,
            }))
        }
        OP_REMOVE => {
            let removed = read_counts(r)?;
            let placed = r.u32()?;
            let (touched, aabb) = read_extent(r)?;
            Ok(OpResult::Remove(RemoveResult {
                removed,
                placed,
                touched,
                aabb,
            }))
        }
        k => Err(WireError::UnknownKind(k)),
    }
}

/// Appends an edit error: its tag, then the chunk as 3 × `i32` or the prefab id as `u64`.
pub fn encode_error(out: &mut Vec<u8>, e: &EditError) {
    match e {
        EditError::NotResident(c) => {
            out.push(ERROR_NOT_RESIDENT);
            put_i32x3(out, c.0);
        }
        EditError::UnknownPrefab(id) => {
            out.push(ERROR_UNKNOWN_PREFAB);
            out.extend_from_slice(&id.to_le_bytes());
        }
    }
}

/// Reads one edit error as [`encode_error`] writes it.
pub fn decode_error(r: &mut Reader<'_>) -> Result<EditError, WireError> {
    match r.u8()? {
        ERROR_NOT_RESIDENT => Ok(EditError::NotResident(ChunkCoord(r.i32x3()?))),
        ERROR_UNKNOWN_PREFAB => Ok(EditError::UnknownPrefab(r.u64()?)),
        k => Err(WireError::UnknownKind(k)),
    }
}
