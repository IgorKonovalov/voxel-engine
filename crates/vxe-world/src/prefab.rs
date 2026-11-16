//! Prefabs and the 24 axis-aligned rotations.
//!
//! A [`Prefab`] is a box of voxels with one write bit per voxel: a voxel whose bit is clear is
//! left alone by a stamp. It is stored as 8³ bricks, a brick whose voxels are all the same
//! material and the same bit collapsed to one word (ADR-0066). A [`Rot`] is a signed axis
//! permutation with determinant +1. Rotating a prefab materialises a new prefab whose minimum
//! corner is again at local `(0, 0, 0)`, so a stamp's origin is always the rotated volume's
//! minimum corner. Everything here is integer arithmetic.
//!
//! A [`PrefabRegistry`] maps a prefab's content id to the prefab: a stamp op names its prefab
//! by that id, and the server, the clients and a save's header resolve it there (ADR-0014).

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use crate::hash::Fnv1a;
use crate::material::{AIR, MaterialId};
use crate::wire::{Reader, WireError};

/// One of the 24 rotations of the cube: a signed permutation matrix with determinant +1.
///
/// Row `k` of the matrix is `±e[axis[k]]`, negative when `neg[k]`, so component `k` of
/// [`Rot::apply`]'s result is `±v[axis[k]]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rot {
    axis: [u8; 3],
    neg: [bool; 3],
}

impl Default for Rot {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Rot {
    pub const IDENTITY: Self = Self {
        axis: [0, 1, 2],
        neg: [false; 3],
    };

    /// A quarter turn about +z: `(x, y, z) -> (-y, x, z)`.
    pub const Z90: Self = Self {
        axis: [1, 0, 2],
        neg: [true, false, false],
    };

    /// The rotation whose row `k` is `±e[axis[k]]` (negative when `neg[k]`), or `None` when
    /// `axis` is not a permutation of `0, 1, 2` or the matrix is a reflection (determinant -1).
    pub fn from_rows(axis: [u8; 3], neg: [bool; 3]) -> Option<Self> {
        let mut seen = [false; 3];
        for &a in &axis {
            let slot = seen.get_mut(usize::from(a))?;
            if *slot {
                return None;
            }
            *slot = true;
        }
        let r = Self { axis, neg };
        (r.determinant() == 1).then_some(r)
    }

    /// Row `k`: the index of its non-zero entry and whether that entry is -1.
    pub fn row(self, k: usize) -> (u8, bool) {
        (self.axis[k], self.neg[k])
    }

    /// The determinant of the matrix: the permutation's parity times the product of the signs.
    /// Always +1 for a value built by this module; exposed for tests.
    pub fn determinant(self) -> i32 {
        let a = self.axis;
        // A permutation of three elements is even exactly when it is a cyclic shift.
        let even = (a[1] + 3 - a[0]) % 3 == 1 && (a[2] + 3 - a[1]) % 3 == 1;
        let mut d = if even { 1 } else { -1 };
        for &n in &self.neg {
            if n {
                d = -d;
            }
        }
        d
    }

    /// All 24 rotations, in one fixed order: axis permutations in lexicographic order, and
    /// within each, the sign patterns `neg` read as a 3-bit number (bit `k` for row `k`)
    /// ascending, keeping only those with determinant +1. The first is [`Rot::IDENTITY`].
    pub fn all() -> impl Iterator<Item = Self> {
        const PERMS: [[u8; 3]; 6] = [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ];
        PERMS.into_iter().flat_map(|axis| {
            (0u8..8).filter_map(move |bits| {
                Self::from_rows(axis, [0, 1, 2].map(|k| bits >> k & 1 != 0))
            })
        })
    }

    /// This rotation's position in [`Rot::all`], 0 to 23: the wire's rotation index.
    pub fn index(self) -> u8 {
        let i = Self::all()
            .position(|r| r == self)
            .expect("every value of Rot is one of the 24 rotations");
        i as u8
    }

    /// The rotation at position `i` of [`Rot::all`]; `None` from 24 on.
    pub fn from_index(i: u8) -> Option<Self> {
        Self::all().nth(usize::from(i))
    }

    /// The rotation that undoes this one: the transpose.
    pub fn inverse(self) -> Self {
        let mut axis = [0; 3];
        let mut neg = [false; 3];
        for k in 0..3 {
            let a = usize::from(self.axis[k]);
            axis[a] = k as u8;
            neg[a] = self.neg[k];
        }
        Self { axis, neg }
    }

    /// This rotation followed by `other`: `self.then(other).apply(v) ==
    /// other.apply(self.apply(v))`.
    pub fn then(self, other: Self) -> Self {
        let mut axis = [0; 3];
        let mut neg = [false; 3];
        for k in 0..3 {
            let a = usize::from(other.axis[k]);
            axis[k] = self.axis[a];
            neg[k] = other.neg[k] ^ self.neg[a];
        }
        Self { axis, neg }
    }

    /// The matrix times `v`. Negation wraps, so `i32::MIN` maps to itself on a negated row
    /// rather than panicking.
    pub fn apply(self, v: [i32; 3]) -> [i32; 3] {
        [0, 1, 2].map(|k| {
            let c = v[usize::from(self.axis[k])];
            if self.neg[k] { c.wrapping_neg() } else { c }
        })
    }

    /// The extent of a `size` box after this rotation (every component non-negative).
    pub fn apply_size(self, size: [u32; 3]) -> [u32; 3] {
        [0, 1, 2].map(|k| size[usize::from(self.axis[k])])
    }
}

/// A prefab brick's edge in voxels, the world's brick edge.
pub const PREFAB_BRICK_EDGE: u32 = 8;

/// Voxels in a prefab brick.
const BRICK_CELLS: usize = 512;

/// A brick word with this bit set is the index of a [`FullBrick`] in [`Prefab`]'s list.
const FULL: u32 = 1 << 31;
/// A uniform brick word with this bit set writes its material; clear, it writes nothing.
const WRITTEN: u32 = 1 << 16;

/// A brick that is not uniform: 512 materials and a write mask, x fastest (`x + 8 (y + 8 z)`),
/// one mask word per z layer. A cell outside the prefab's box (an edge brick's overhang) is
/// air and unwritten, and so is every masked-out cell. 1088 bytes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct FullBrick {
    pub(crate) voxels: [MaterialId; BRICK_CELLS],
    pub(crate) mask: [u64; 8],
}

/// One brick of a prefab, as a stamp reads it.
#[derive(Clone, Copy, Debug)]
pub(crate) enum BrickRef<'a> {
    /// Every cell of the brick inside the box is this material, written or not (an unwritten
    /// one is [`AIR`]).
    Uniform(MaterialId, bool),
    Full(&'a FullBrick),
}

/// The index of local voxel `l` (each component under 8) in a brick.
#[inline]
fn cell_index(l: [u32; 3]) -> usize {
    (l[0] + PREFAB_BRICK_EDGE * (l[1] + PREFAB_BRICK_EDGE * l[2])) as usize
}

/// A box of voxels with a write mask, stored as 8³ bricks in a grid of `ceil(size / 8)` per
/// axis, x fastest; a brick whose voxels inside the box all hold one material and one write bit
/// is one word (ADR-0066).
///
/// The form is canonical: a brick is uniform exactly when its cells inside the box agree, a
/// masked-out voxel stores [`AIR`], and cells past the box are air and unwritten. Two prefabs
/// that write the same materials to the same cells are therefore equal, and have the same
/// [`Prefab::id`], the hash of the dense form (size, voxels, mask), so the id does not depend
/// on the storage.
#[derive(Clone, Debug)]
pub struct Prefab {
    size: [u32; 3],
    grid: [u32; 3],
    /// One word per brick: a uniform brick's material (low 16 bits) and its [`WRITTEN`] bit,
    /// or [`FULL`] and the index of its [`FullBrick`].
    bricks: Vec<u32>,
    /// The bricks that are not uniform, in grid order.
    full: Vec<FullBrick>,
    /// [`Prefab::id`], computed on first use.
    id: OnceLock<u64>,
}

impl PartialEq for Prefab {
    fn eq(&self, other: &Self) -> bool {
        self.size == other.size && self.bricks == other.bricks && self.full == other.full
    }
}

impl Eq for Prefab {}

impl std::hash::Hash for Prefab {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.size.hash(state);
        self.bricks.hash(state);
        self.full.hash(state);
    }
}

/// The voxel count of `size`, or `None` when it overflows `usize`.
fn voxel_count(size: [u32; 3]) -> Option<usize> {
    size.iter()
        .try_fold(1usize, |n, &s| n.checked_mul(s as usize))
}

impl Prefab {
    /// The grid of a prefab of `size`: `ceil(size / 8)` bricks per axis.
    fn grid_of(size: [u32; 3]) -> [u32; 3] {
        size.map(|s| s.div_ceil(PREFAB_BRICK_EDGE))
    }

    /// Builds a prefab of `size` brick by brick, x fastest. For each brick, `hint` (its brick
    /// coordinate and the inclusive local voxel box inside the prefab it covers) may answer
    /// for every cell at once with `(material, written)`; otherwise `cell` is asked per voxel
    /// inside the box (`None` leaves the voxel unwritten).
    fn build(
        size: [u32; 3],
        mut hint: impl FnMut([u32; 3], [u32; 3], [u32; 3]) -> Option<(MaterialId, bool)>,
        mut cell: impl FnMut([u32; 3]) -> Option<MaterialId>,
    ) -> Self {
        let grid = Self::grid_of(size);
        let n = grid.iter().map(|&g| g as usize).product::<usize>();
        let mut bricks = Vec::with_capacity(n);
        let mut full = Vec::new();
        let e = PREFAB_BRICK_EDGE;
        let mut fb = FullBrick {
            voxels: [AIR; BRICK_CELLS],
            mask: [0; 8],
        };
        for bz in 0..grid[2] {
            for by in 0..grid[1] {
                for bx in 0..grid[0] {
                    let b = [bx, by, bz];
                    let lo = b.map(|v| v * e);
                    let hi = [0, 1, 2].map(|k| (lo[k] + e).min(size[k]) - 1);
                    if let Some((m, w)) = hint(b, lo, hi) {
                        bricks.push(uniform_word(m, w));
                        continue;
                    }
                    fb.voxels = [AIR; BRICK_CELLS];
                    fb.mask = [0; 8];
                    let mut first = None;
                    let mut same = true;
                    for z in lo[2]..=hi[2] {
                        for y in lo[1]..=hi[1] {
                            for x in lo[0]..=hi[0] {
                                let c = cell([x, y, z]);
                                let v = (c.unwrap_or(AIR), c.is_some());
                                match first {
                                    None => first = Some(v),
                                    Some(f) if f != v => same = false,
                                    Some(_) => {}
                                }
                                if let Some(m) = c {
                                    let i = cell_index([x - lo[0], y - lo[1], z - lo[2]]);
                                    fb.voxels[i] = m;
                                    fb.mask[i / 64] |= 1 << (i % 64);
                                }
                            }
                        }
                    }
                    if same {
                        let (m, w) = first.expect("a brick of the grid holds a voxel");
                        bricks.push(uniform_word(m, w));
                    } else {
                        bricks.push(FULL | full.len() as u32);
                        full.push(fb.clone());
                    }
                }
            }
        }
        Self {
            size,
            grid,
            bricks,
            full,
            id: OnceLock::new(),
        }
    }

    /// A prefab of `size` whose voxel at local `l` is `cell(l)`, `None` leaving it unwritten.
    pub fn from_fn(size: [u32; 3], cell: impl FnMut([u32; 3]) -> Option<MaterialId>) -> Self {
        Self::build(size, |_, _, _| None, cell)
    }

    /// A prefab of `size` from `voxels` (x fastest) and a write mask (bit `i % 64` of word
    /// `i / 64` for voxel `i`); `None` writes every voxel. A masked-out voxel's value is
    /// dropped (stored as [`AIR`]).
    ///
    /// # Panics
    ///
    /// When `voxels` does not hold exactly `sx * sy * sz` values, or `mask` does not hold
    /// exactly `voxels.len().div_ceil(64)` words, or when `sx * sy * sz` overflows `usize`.
    pub fn new(size: [u32; 3], voxels: Vec<MaterialId>, mask: Option<Vec<u64>>) -> Self {
        let n = voxel_count(size)
            .expect("prefab voxel count does not match its size: the size overflows usize");
        assert_eq!(
            voxels.len(),
            n,
            "prefab voxel count does not match its size"
        );
        if let Some(m) = &mask {
            assert_eq!(
                m.len(),
                n.div_ceil(64),
                "prefab mask length does not match its size"
            );
        }
        let s = size.map(|v| v as usize);
        Self::from_fn(size, |l| {
            let i = l[0] as usize + s[0] * (l[1] as usize + s[1] * l[2] as usize);
            let written = mask.as_ref().is_none_or(|m| m[i / 64] >> (i % 64) & 1 != 0);
            written.then(|| voxels[i])
        })
    }

    /// A cube of edge `edge` whose surface voxels are `shell` and whose interior is air,
    /// every voxel written. A fixture for tests, benches and testbeds.
    pub fn vault(edge: u32, shell: MaterialId) -> Self {
        Self::from_fn([edge; 3], |l| {
            let surface = l.iter().any(|&c| c == 0 || c + 1 == edge);
            Some(if surface { shell } else { AIR })
        })
    }

    pub fn size(&self) -> [u32; 3] {
        self.size
    }

    /// Voxels in the box, written or not.
    pub fn len(&self) -> usize {
        voxel_count(self.size).unwrap_or(usize::MAX)
    }

    pub fn is_empty(&self) -> bool {
        self.size.contains(&0)
    }

    /// The brick grid: bricks per axis.
    pub fn grid(&self) -> [u32; 3] {
        self.grid
    }

    /// The bricks stored as one word and the bricks stored whole.
    pub fn brick_counts(&self) -> (usize, usize) {
        (self.bricks.len() - self.full.len(), self.full.len())
    }

    /// Bytes the prefab holds in RAM: the struct, a word per brick and 1088 per stored brick.
    pub fn bytes(&self) -> usize {
        size_of::<Self>()
            + self.bricks.capacity() * size_of::<u32>()
            + self.full.capacity() * size_of::<FullBrick>()
    }

    /// Brick `b` (inside the grid).
    #[inline]
    pub(crate) fn brick(&self, b: [u32; 3]) -> BrickRef<'_> {
        let i = b[0] as usize
            + self.grid[0] as usize * (b[1] as usize + self.grid[1] as usize * b[2] as usize);
        let w = self.bricks[i];
        if w & FULL != 0 {
            BrickRef::Full(&self.full[(w & !FULL) as usize])
        } else {
            BrickRef::Uniform(w as u16, w & WRITTEN != 0)
        }
    }

    /// The material and write bit of local voxel `l`, inside the box.
    #[inline]
    pub(crate) fn cell(&self, l: [u32; 3]) -> (MaterialId, bool) {
        let e = PREFAB_BRICK_EDGE;
        match self.brick(l.map(|v| v / e)) {
            BrickRef::Uniform(m, w) => (m, w),
            BrickRef::Full(f) => {
                let i = cell_index(l.map(|v| v % e));
                (f.voxels[i], f.mask[i / 64] >> (i % 64) & 1 != 0)
            }
        }
    }

    /// The material a stamp writes at local voxel `l`: `None` outside the box and where the
    /// mask is clear.
    pub fn get(&self, l: [i32; 3]) -> Option<MaterialId> {
        let mut u = [0u32; 3];
        for k in 0..3 {
            u[k] = u32::try_from(l[k]).ok().filter(|&c| c < self.size[k])?;
        }
        let (m, w) = self.cell(u);
        w.then_some(m)
    }

    /// Written voxels.
    pub fn written_count(&self) -> usize {
        let e = PREFAB_BRICK_EDGE;
        let mut n = 0;
        for bz in 0..self.grid[2] {
            for by in 0..self.grid[1] {
                for bx in 0..self.grid[0] {
                    let b = [bx, by, bz];
                    n += match self.brick(b) {
                        BrickRef::Uniform(_, true) => (0..3)
                            .map(|k| ((b[k] * e + e).min(self.size[k]) - b[k] * e) as usize)
                            .product(),
                        BrickRef::Uniform(_, false) => 0,
                        BrickRef::Full(f) => f.mask.iter().map(|w| w.count_ones() as usize).sum(),
                    };
                }
            }
        }
        n
    }

    /// Visits every voxel in the dense order `x + sx (y + sy z)`, a row of up to eight at a
    /// time: `f(materials, written bits)`, bit `k` for the row's `k`-th voxel.
    fn rows(&self, mut f: impl FnMut(&[MaterialId], u8)) {
        let e = PREFAB_BRICK_EDGE;
        let mut row = [AIR; 8];
        for z in 0..self.size[2] {
            for y in 0..self.size[1] {
                for bx in 0..self.grid[0] {
                    let len = ((bx * e + e).min(self.size[0]) - bx * e) as usize;
                    let bits = match self.brick([bx, y / e, z / e]) {
                        BrickRef::Uniform(m, w) => {
                            row[..len].fill(m);
                            if w { ((1u16 << len) - 1) as u8 } else { 0 }
                        }
                        BrickRef::Full(fb) => {
                            let base = cell_index([0, y % e, z % e]);
                            row[..len].copy_from_slice(&fb.voxels[base..base + len]);
                            (fb.mask[base / 64] >> (base % 64)) as u8 & ((1u16 << len) - 1) as u8
                        }
                    };
                    f(&row[..len], bits);
                }
            }
        }
    }

    /// FNV-1a 64 over the dense form: the size (three little-endian `u32`), the voxels
    /// (little-endian `u16`, x fastest, [`AIR`] where the mask is clear) and the mask words
    /// (little-endian `u64`, bit `i % 64` of word `i / 64` for voxel `i`, bits past the last
    /// voxel clear). Streamed from the bricks; computed once and kept.
    pub fn id(&self) -> u64 {
        *self.id.get_or_init(|| {
            let mut h = Fnv1a::default();
            for s in self.size {
                h.write(&s.to_le_bytes());
            }
            let mut buf = [0u8; 16];
            self.rows(|row, _| {
                for (k, v) in row.iter().enumerate() {
                    buf[2 * k..2 * k + 2].copy_from_slice(&v.to_le_bytes());
                }
                h.write(&buf[..2 * row.len()]);
            });
            let (mut word, mut filled) = (0u64, 0u32);
            self.rows(|row, bits| {
                let len = row.len() as u32;
                word |= u64::from(bits) << filled;
                if filled + len >= 64 {
                    h.write(&word.to_le_bytes());
                    let used = 64 - filled;
                    word = if used < len {
                        u64::from(bits) >> used
                    } else {
                        0
                    };
                    filled = len - used;
                } else {
                    filled += len;
                }
            });
            if filled > 0 {
                h.write(&word.to_le_bytes());
            }
            h.finish()
        })
    }

    /// Bytes of [`Prefab::encode`]'s output: 12, then per brick a tag byte and 0 (unwritten),
    /// 2 (one material) or 1088 (a stored brick) bytes.
    pub fn encoded_len(&self) -> usize {
        12 + self
            .bricks
            .iter()
            .map(|&w| match w {
                w if w & FULL != 0 => 1 + 2 * BRICK_CELLS + 64,
                w if w & WRITTEN != 0 => 3,
                _ => 1,
            })
            .sum::<usize>()
    }

    /// Appends the brick form (`LOG_FORMAT` 3): the size as three little-endian `u32`, then
    /// each brick in grid order (x fastest) as a tag byte: 0 for a brick that writes nothing;
    /// 1 and a little-endian `u16` for a brick that writes one material; 2, the 512 materials
    /// as little-endian `u16` (x fastest) and the 8 mask words as little-endian `u64` for a
    /// stored brick.
    pub fn encode(&self, out: &mut Vec<u8>) {
        for s in self.size {
            out.extend_from_slice(&s.to_le_bytes());
        }
        for &w in &self.bricks {
            if w & FULL != 0 {
                out.push(2);
                let f = &self.full[(w & !FULL) as usize];
                for v in &f.voxels {
                    out.extend_from_slice(&v.to_le_bytes());
                }
                for m in &f.mask {
                    out.extend_from_slice(&m.to_le_bytes());
                }
            } else if w & WRITTEN != 0 {
                out.push(1);
                out.extend_from_slice(&(w as u16).to_le_bytes());
            } else {
                out.push(0);
            }
        }
    }

    /// Reads a prefab as [`Prefab::encode`] writes it (`LOG_FORMAT` 3). Bytes that end inside
    /// it are [`WireError::Truncated`], a brick tag past 2 [`WireError::UnknownKind`]; the value
    /// is made canonical (a stored brick that is uniform collapses, masked-out and overhanging
    /// cells become unwritten air), so a caller that knows the id checks it against
    /// [`Prefab::id`].
    pub fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        let size = [r.u32()?, r.u32()?, r.u32()?];
        let grid = Self::grid_of(size);
        let truncated = WireError::Truncated { at: r.entry() };
        let n = grid
            .iter()
            .try_fold(1usize, |n, &g| n.checked_mul(g as usize))
            .ok_or(truncated)?;
        let mut words = Vec::new();
        let mut stored = Vec::new();
        for _ in 0..n {
            match r.u8()? {
                0 => words.push(uniform_word(AIR, false)),
                1 => words.push(uniform_word(r.u16()?, true)),
                2 => {
                    let raw = r.take(2 * BRICK_CELLS + 64)?;
                    let mut f = FullBrick {
                        voxels: [AIR; BRICK_CELLS],
                        mask: [0; 8],
                    };
                    for (v, b) in f.voxels.iter_mut().zip(raw.chunks_exact(2)) {
                        *v = u16::from_le_bytes([b[0], b[1]]);
                    }
                    for (m, b) in f
                        .mask
                        .iter_mut()
                        .zip(raw[2 * BRICK_CELLS..].chunks_exact(8))
                    {
                        *m = u64::from_le_bytes(b.try_into().expect("chunks of 8"));
                    }
                    words.push(FULL | stored.len() as u32);
                    stored.push(f);
                }
                t => return Err(WireError::UnknownKind(t)),
            }
        }
        let raw = Self {
            size,
            grid,
            bricks: words,
            full: stored,
            id: OnceLock::new(),
        };
        // Canonical again: re-built from its own cells.
        Ok(Self::build(
            size,
            |b, _, _| match raw.brick(b) {
                BrickRef::Uniform(m, w) => Some((if w { m } else { AIR }, w)),
                BrickRef::Full(_) => None,
            },
            |l| {
                let (m, w) = raw.cell(l);
                w.then_some(m)
            },
        ))
    }

    /// Reads a prefab in the dense form of `LOG_FORMAT` 2: the size as three little-endian
    /// `u32`, the voxels as little-endian `u16` (x fastest), the mask words as little-endian
    /// `u64`. A size whose voxels or mask would run past the bytes is
    /// [`WireError::Truncated`]; the value is made canonical as by [`Prefab::new`].
    pub fn decode_dense(r: &mut Reader<'_>) -> Result<Self, WireError> {
        let size = [r.u32()?, r.u32()?, r.u32()?];
        let truncated = WireError::Truncated { at: r.entry() };
        let n = voxel_count(size).ok_or(truncated)?;
        let raw = r.take(n.checked_mul(2).ok_or(truncated)?)?;
        let voxels = raw
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        let raw = r.take(n.div_ceil(64) * 8)?;
        let mask: Vec<u64> = raw
            .chunks_exact(8)
            .map(|b| u64::from_le_bytes(b.try_into().expect("chunks of 8")))
            .collect();
        let mut mask = mask;
        if n % 64 != 0
            && let Some(last) = mask.last_mut()
        {
            *last &= (1u64 << (n % 64)) - 1;
        }
        Ok(Self::new(size, voxels, Some(mask)))
    }

    /// This prefab turned by `rot` and moved back so its minimum corner is local
    /// `(0, 0, 0)`. Destination component `k` reads source axis `axis[k]`, mirrored across
    /// that axis when the row is negative. A destination brick whose source voxels all lie in
    /// uniform bricks of one value is one word without a voxel visited.
    pub fn rotated(&self, rot: Rot) -> Self {
        if rot == Rot::IDENTITY {
            return self.clone();
        }
        let size = rot.apply_size(self.size);
        let src = |d: [u32; 3]| {
            let mut s = [0u32; 3];
            for (k, &dk) in d.iter().enumerate() {
                let (a, neg) = rot.row(k);
                let a = usize::from(a);
                s[a] = if neg { self.size[a] - 1 - dk } else { dk };
            }
            s
        };
        let e = PREFAB_BRICK_EDGE;
        Self::build(
            size,
            |_, lo, hi| {
                // The source box the destination box reads, then every source brick it meets.
                let (a, b) = (src(lo), src(hi));
                let slo = [0, 1, 2].map(|k| a[k].min(b[k]) / e);
                let shi = [0, 1, 2].map(|k| a[k].max(b[k]) / e);
                let mut seen: Option<(MaterialId, bool)> = None;
                for z in slo[2]..=shi[2] {
                    for y in slo[1]..=shi[1] {
                        for x in slo[0]..=shi[0] {
                            match self.brick([x, y, z]) {
                                BrickRef::Uniform(m, w) if seen.is_none_or(|s| s == (m, w)) => {
                                    seen = Some((m, w));
                                }
                                _ => return None,
                            }
                        }
                    }
                }
                seen
            },
            |d| {
                let (m, w) = self.cell(src(d));
                w.then_some(m)
            },
        )
    }
}

/// A uniform brick's word.
fn uniform_word(m: MaterialId, written: bool) -> u32 {
    if written { u32::from(m) | WRITTEN } else { 0 }
}

/// Two different prefabs with one content id: the registry keeps the first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PrefabCollision {
    pub id: u64,
}

impl std::fmt::Display for PrefabCollision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "another prefab is registered under id {:#018x}", self.id)
    }
}

impl std::error::Error for PrefabCollision {}

/// Prefabs by content id ([`Prefab::id`]), ascending by id. A stamp op names its prefab by
/// id; whoever applies it resolves the id here (ADR-0014).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrefabRegistry {
    by_id: BTreeMap<u64, Arc<Prefab>>,
}

impl PrefabRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `prefab` under its id and returns the id. Registering an equal prefab again
    /// is a no-op; a different prefab with the same id is refused and the first one kept.
    pub fn register(&mut self, prefab: Arc<Prefab>) -> Result<u64, PrefabCollision> {
        let id = prefab.id();
        match self.by_id.get(&id) {
            Some(have) if **have != *prefab => Err(PrefabCollision { id }),
            Some(_) => Ok(id),
            None => {
                self.by_id.insert(id, prefab);
                Ok(id)
            }
        }
    }

    /// The prefab registered under `id`.
    pub fn get(&self, id: u64) -> Option<&Arc<Prefab>> {
        self.by_id.get(&id)
    }

    pub fn contains(&self, id: u64) -> bool {
        self.by_id.contains_key(&id)
    }

    /// Every registered prefab with its id, ascending by id.
    pub fn iter(&self) -> impl Iterator<Item = (u64, &Arc<Prefab>)> {
        self.by_id.iter().map(|(&id, p)| (id, p))
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// The registered prefabs whose ids are in `ids`; ids not registered are skipped.
    pub fn subset(&self, ids: impl IntoIterator<Item = u64>) -> Self {
        let by_id = ids
            .into_iter()
            .filter_map(|id| self.by_id.get(&id).map(|p| (id, p.clone())))
            .collect();
        Self { by_id }
    }
}
