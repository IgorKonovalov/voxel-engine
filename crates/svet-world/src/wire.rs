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
//! | 5    | drop through | max drop `i32`, yield threshold `u16` (never 0), cell count `u32`, the cells |
//! | 6    | pile     | rubble `u16`, count `u32`, max depth `i32`, cell count `u32`, the cells |
//!
//! A drop is kind 3 when its yield threshold is 0 and kind 5 otherwise, so each drop has one
//! encoding: a kind-5 entry with a threshold of 0 is [`WireError::NonCanonical`].
//!
//! An edit's modes and shapes:
//!
//! | mode      | payload           | shape     | payload                 |
//! |-----------|-------------------|-----------|-------------------------|
//! | `Carve`   | none              | sphere    | centre, radius: 4 × i64 |
//! | `Fill`    | material `u16`    | box       | min, max: 6 × i64       |
//! | `Replace` | from, to: 2 × u16 | capsule   | a, b, radius: 7 × i64   |
//!
//! An op result is a kind `u8` ([`OP_EDIT`] for an edit's or a stamp's, [`OP_DROP`] for
//! either drop's, [`OP_REMOVE`], [`OP_PILE`]) then the result ([`encode_op_result`]).
//!
//! Tag value 0 is never assigned, so a zeroed buffer never decodes as a valid op.

use crate::coarse::CellCoord;
use crate::collapse::{DropResult, PileResult, RemoveResult};
use crate::coords::{ChunkCoord, VoxelCoord};
use crate::edit::{EditError, EditOp, EditResult, Mode};
use crate::fixed::Fixed3;
use crate::limits::Field;
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
/// Op kind of a region drop through the cells under a yield threshold (ADR-0016); a drop
/// with a threshold of 0 is [`OP_DROP`].
pub const OP_DROP_THROUGH: u8 = 5;
/// Op kind of a rubble pile on a region.
pub const OP_PILE: u8 = 6;
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
/// Error tag of [`EditError::OutOfRange`]: the field `u8` follows, its index in
/// [`Field::ALL`] plus 1.
pub const ERROR_OUT_OF_RANGE: u8 = 2;
/// Error tag of [`EditError::UnknownPrefab`]: the id `u64` follows.
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
    /// An entry, starting at `at`, in an encoding the format leaves to another kind: a kind-5
    /// drop whose yield threshold is 0, which only kind 3 encodes.
    NonCanonical { at: usize },
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
            Self::NonCanonical { at } => {
                write!(f, "the entry at offset {at} has a shorter encoding")
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

/// Appends little-endian fields to a byte buffer: the writing side of [`Reader`], field for
/// field.
#[derive(Debug)]
pub struct Writer<'a> {
    out: &'a mut Vec<u8>,
}

impl<'a> Writer<'a> {
    pub fn new(out: &'a mut Vec<u8>) -> Self {
        Self { out }
    }

    /// Appends `b` as it is.
    pub fn bytes(&mut self, b: &[u8]) {
        self.out.extend_from_slice(b);
    }

    pub fn u8(&mut self, v: u8) {
        self.out.push(v);
    }

    pub fn u16(&mut self, v: u16) {
        self.bytes(&v.to_le_bytes());
    }

    pub fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }

    pub fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }

    pub fn i32(&mut self, v: i32) {
        self.bytes(&v.to_le_bytes());
    }

    pub fn i64(&mut self, v: i64) {
        self.bytes(&v.to_le_bytes());
    }

    /// Three `i64`.
    pub fn fixed3(&mut self, p: Fixed3) {
        for v in p.0 {
            self.i64(v);
        }
    }

    /// Three `i32`.
    pub fn i32x3(&mut self, p: [i32; 3]) {
        for v in p {
            self.i32(v);
        }
    }
}

/// Appends the little-endian bytes of three `i64`.
pub fn put_fixed3(out: &mut Vec<u8>, p: Fixed3) {
    Writer::new(out).fixed3(p);
}

/// Appends the little-endian bytes of three `i32`.
pub fn put_i32x3(out: &mut Vec<u8>, p: [i32; 3]) {
    Writer::new(out).i32x3(p);
}

/// Appends a list count as `u16`. A list longer than `u16::MAX` is an invariant violation:
/// no edit result or payload the engine produces has one.
fn put_count(w: &mut Writer<'_>, n: usize) {
    w.u16(u16::try_from(n).expect("wire list counts fit u16"));
}

/// Appends a cell list: its count as `u32`, then each cell as 3 × `i32`. Panics past
/// `u32::MAX` cells.
fn put_cells(w: &mut Writer<'_>, cells: &[CellCoord]) {
    w.u32(u32::try_from(cells.len()).expect("wire cell lists fit u32"));
    for c in cells {
        w.i32x3(c.0);
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

fn put_counts(w: &mut Writer<'_>, counts: &[(MaterialId, u32)]) {
    put_count(w, counts.len());
    for &(m, n) in counts {
        w.u16(m);
        w.u32(n);
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
fn put_extent(w: &mut Writer<'_>, touched: &[ChunkCoord], aabb: Option<(VoxelCoord, VoxelCoord)>) {
    put_count(w, touched.len());
    for c in touched {
        w.i32x3(c.0);
    }
    match aabb {
        None => w.u8(0),
        Some((lo, hi)) => {
            w.u8(1);
            w.i32x3(lo.0);
            w.i32x3(hi.0);
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
    let w = &mut Writer::new(out);
    match op {
        Op::Edit(e) => {
            w.u8(OP_EDIT);
            encode_edit(w, e);
        }
        Op::Stamp {
            prefab,
            origin,
            rot,
        } => {
            w.u8(OP_STAMP);
            w.u64(*prefab);
            w.i32x3(origin.0);
            w.u8(rot.index());
        }
        Op::Drop {
            cells,
            max_drop,
            yield_below: 0,
        } => {
            w.u8(OP_DROP);
            w.i32(*max_drop);
            put_cells(w, cells);
        }
        Op::Drop {
            cells,
            max_drop,
            yield_below,
        } => {
            w.u8(OP_DROP_THROUGH);
            w.i32(*max_drop);
            w.u16(*yield_below);
            put_cells(w, cells);
        }
        Op::Remove {
            cells,
            rubble,
            fraction,
            max_depth,
        } => {
            w.u8(OP_REMOVE);
            w.u16(*rubble);
            w.u32(fraction.0);
            w.u32(fraction.1);
            w.i32(*max_depth);
            put_cells(w, cells);
        }
        Op::Pile {
            cells,
            rubble,
            count,
            max_depth,
        } => {
            w.u8(OP_PILE);
            w.u16(*rubble);
            w.u32(*count);
            w.i32(*max_depth);
            put_cells(w, cells);
        }
    }
}

/// An edit's mode and payload, then its shape and payload.
fn encode_edit(w: &mut Writer<'_>, e: &EditOp) {
    match e.mode {
        Mode::Carve => w.u8(MODE_CARVE),
        Mode::Fill(m) => {
            w.u8(MODE_FILL);
            w.u16(m);
        }
        Mode::Replace { from, to } => {
            w.u8(MODE_REPLACE);
            w.u16(from);
            w.u16(to);
        }
    }
    match e.shape {
        Shape::Sphere { c, r } => {
            w.u8(SHAPE_SPHERE);
            w.fixed3(c);
            w.i64(r);
        }
        Shape::Box_ { min, max } => {
            w.u8(SHAPE_BOX);
            w.fixed3(min);
            w.fixed3(max);
        }
        Shape::Capsule { a, b, r } => {
            w.u8(SHAPE_CAPSULE);
            w.fixed3(a);
            w.fixed3(b);
            w.i64(r);
        }
    }
}

/// Reads one op as [`encode_op`] writes it. A rotation index past 23 is
/// [`WireError::UnknownKind`]; a kind-5 drop with a yield threshold of 0 is
/// [`WireError::NonCanonical`].
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
                yield_below: 0,
            })
        }
        OP_DROP_THROUGH => {
            let max_drop = r.i32()?;
            let yield_below = r.u16()?;
            if yield_below == 0 {
                return Err(WireError::NonCanonical { at: r.entry() });
            }
            Ok(Op::Drop {
                cells: read_cells(r)?,
                max_drop,
                yield_below,
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
        OP_PILE => {
            let rubble = r.u16()?;
            let count = r.u32()?;
            let max_depth = r.i32()?;
            Ok(Op::Pile {
                cells: read_cells(r)?,
                rubble,
                count,
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
    put_result(&mut Writer::new(out), r);
}

fn put_result(w: &mut Writer<'_>, r: &EditResult) {
    put_counts(w, &r.changed_from);
    put_extent(w, &r.touched, r.aabb);
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
/// [`encode_result`] writes it; for [`OP_DROP`] the moved counts, the crushed counts, `dropped`
/// as `i32`, then the touched chunks and the aabb; for [`OP_REMOVE`] the removed counts,
/// `placed` as `u32`, then the touched chunks and the aabb; for [`OP_PILE`] `placed` as `u32`,
/// then the touched chunks and the aabb. Counts, chunks and the aabb are encoded as in an edit
/// result.
pub fn encode_op_result(out: &mut Vec<u8>, r: &OpResult) {
    let w = &mut Writer::new(out);
    match r {
        OpResult::Edit(e) => {
            w.u8(OP_EDIT);
            put_result(w, e);
        }
        OpResult::Drop(d) => {
            w.u8(OP_DROP);
            put_counts(w, &d.moved);
            put_counts(w, &d.crushed);
            w.i32(d.dropped);
            put_extent(w, &d.touched, d.aabb);
        }
        OpResult::Remove(m) => {
            w.u8(OP_REMOVE);
            put_counts(w, &m.removed);
            w.u32(m.placed);
            put_extent(w, &m.touched, m.aabb);
        }
        OpResult::Pile(p) => {
            w.u8(OP_PILE);
            w.u32(p.placed);
            put_extent(w, &p.touched, p.aabb);
        }
    }
}

/// Reads one op result as [`encode_op_result`] writes it.
pub fn decode_op_result(r: &mut Reader<'_>) -> Result<OpResult, WireError> {
    match r.u8()? {
        OP_EDIT => Ok(OpResult::Edit(decode_result(r)?)),
        OP_DROP => {
            let moved = read_counts(r)?;
            let crushed = read_counts(r)?;
            let dropped = r.i32()?;
            let (touched, aabb) = read_extent(r)?;
            Ok(OpResult::Drop(DropResult {
                moved,
                dropped,
                crushed,
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
        OP_PILE => {
            let placed = r.u32()?;
            let (touched, aabb) = read_extent(r)?;
            Ok(OpResult::Pile(PileResult {
                placed,
                touched,
                aabb,
            }))
        }
        k => Err(WireError::UnknownKind(k)),
    }
}

/// Appends an edit error: its tag, then the chunk as 3 × `i32`, the field as `u8` or the
/// prefab id as `u64`.
pub fn encode_error(out: &mut Vec<u8>, e: &EditError) {
    let w = &mut Writer::new(out);
    match e {
        EditError::NotResident(c) => {
            w.u8(ERROR_NOT_RESIDENT);
            w.i32x3(c.0);
        }
        EditError::UnknownPrefab(id) => {
            w.u8(ERROR_UNKNOWN_PREFAB);
            w.u64(*id);
        }
        EditError::OutOfRange { what } => {
            w.u8(ERROR_OUT_OF_RANGE);
            let i = Field::ALL
                .iter()
                .position(|f| f == what)
                .expect("every field is listed");
            w.u8(i as u8 + 1);
        }
    }
}

/// Reads one edit error as [`encode_error`] writes it.
pub fn decode_error(r: &mut Reader<'_>) -> Result<EditError, WireError> {
    match r.u8()? {
        ERROR_NOT_RESIDENT => Ok(EditError::NotResident(ChunkCoord(r.i32x3()?))),
        ERROR_UNKNOWN_PREFAB => Ok(EditError::UnknownPrefab(r.u64()?)),
        ERROR_OUT_OF_RANGE => {
            let k = r.u8()?;
            let what = usize::from(k)
                .checked_sub(1)
                .and_then(|i| Field::ALL.get(i))
                .ok_or(WireError::UnknownKind(k))?;
            Ok(EditError::OutOfRange { what: *what })
        }
        k => Err(WireError::UnknownKind(k)),
    }
}
