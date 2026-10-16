//! The byte encoding of ops, edit results and errors, shared by the logs, the save file and
//! the server's messages (ADR-0007).
//!
//! Little-endian throughout, no padding, no alignment, no varints. A `Fixed3` is 3 × `i64`, a
//! `ChunkCoord` and a `VoxelCoord` 3 × `i32`, a list count a `u16`. An op is its kind `u8`,
//! its mode `u8` plus the mode's payload, then its shape `u8` plus the shape's payload:
//!
//! | mode      | payload           | shape     | payload                 |
//! |-----------|-------------------|-----------|-------------------------|
//! | `Carve`   | none              | sphere    | centre, radius: 4 × i64 |
//! | `Fill`    | material `u16`    | box       | min, max: 6 × i64       |
//! | `Replace` | from, to: 2 × u16 | capsule   | a, b, radius: 7 × i64   |
//!
//! Tag value 0 is never assigned, so a zeroed buffer never decodes as a valid op.

use crate::coords::{ChunkCoord, VoxelCoord};
use crate::edit::{EditError, EditOp, EditResult, Mode};
use crate::fixed::Fixed3;
use crate::log::Op;
use crate::shape::Shape;

/// Op kind of an edit (shape plus mode).
pub const OP_EDIT: u8 = 1;
/// Op kinds reserved for the stamp op (Plan 0008) and the drop and remove ops (Plan 0009).
/// Until a format claims them they decode as [`WireError::UnknownKind`].
pub const OP_RESERVED_STAMP: u8 = 2;
pub const OP_RESERVED_DROP: u8 = 3;
pub const OP_RESERVED_REMOVE: u8 = 4;

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

/// Error tag of [`EditError::NotResident`].
pub const ERROR_NOT_RESIDENT: u8 = 1;

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

/// Appends the encoding of `op`: kind, mode and payload, shape and payload.
pub fn encode_op(out: &mut Vec<u8>, op: &Op) {
    let Op::Edit(e) = op;
    out.push(OP_EDIT);
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

/// Reads one op as [`encode_op`] writes it.
pub fn decode_op(r: &mut Reader<'_>) -> Result<Op, WireError> {
    let kind = r.u8()?;
    if kind != OP_EDIT {
        return Err(WireError::UnknownKind(kind));
    }
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
    Ok(Op::Edit(EditOp { shape, mode }))
}

/// Appends an edit result: a `u16` count of `(material u16, voxels u32)` pairs, a `u16` count
/// of chunk coordinates, then a presence `u8` and, when 1, the aabb as 6 × `i32`.
pub fn encode_result(out: &mut Vec<u8>, r: &EditResult) {
    put_count(out, r.changed_from.len());
    for &(m, n) in &r.changed_from {
        out.extend_from_slice(&m.to_le_bytes());
        out.extend_from_slice(&n.to_le_bytes());
    }
    put_count(out, r.touched.len());
    for c in &r.touched {
        put_i32x3(out, c.0);
    }
    match r.aabb {
        None => out.push(0),
        Some((lo, hi)) => {
            out.push(1);
            put_i32x3(out, lo.0);
            put_i32x3(out, hi.0);
        }
    }
}

/// Reads one edit result as [`encode_result`] writes it.
pub fn decode_result(r: &mut Reader<'_>) -> Result<EditResult, WireError> {
    let n = r.u16()?;
    let mut changed_from = Vec::with_capacity(usize::from(n));
    for _ in 0..n {
        changed_from.push((r.u16()?, r.u32()?));
    }
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
    Ok(EditResult {
        changed_from,
        touched,
        aabb,
    })
}

/// Appends an edit error: its tag, then the chunk as 3 × `i32`.
pub fn encode_error(out: &mut Vec<u8>, e: &EditError) {
    match e {
        EditError::NotResident(c) => {
            out.push(ERROR_NOT_RESIDENT);
            put_i32x3(out, c.0);
        }
    }
}

/// Reads one edit error as [`encode_error`] writes it.
pub fn decode_error(r: &mut Reader<'_>) -> Result<EditError, WireError> {
    match r.u8()? {
        ERROR_NOT_RESIDENT => Ok(EditError::NotResident(ChunkCoord(r.i32x3()?))),
        k => Err(WireError::UnknownKind(k)),
    }
}
