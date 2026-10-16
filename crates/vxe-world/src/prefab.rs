//! Prefabs and the 24 axis-aligned rotations.
//!
//! A [`Prefab`] is a dense box of voxels, x fastest, with one write bit per voxel: a voxel
//! whose bit is clear is left alone by a stamp. A [`Rot`] is a signed axis permutation with
//! determinant +1. Rotating a prefab materialises a new prefab whose minimum corner is again
//! at local `(0, 0, 0)`, so a stamp's origin is always the rotated volume's minimum corner.
//! Everything here is integer arithmetic.

use crate::hash::Fnv1a;
use crate::material::{AIR, MaterialId};

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

    /// The matrix times `v`.
    pub fn apply(self, v: [i32; 3]) -> [i32; 3] {
        [0, 1, 2].map(|k| {
            let c = v[usize::from(self.axis[k])];
            if self.neg[k] { -c } else { c }
        })
    }

    /// The extent of a `size` box after this rotation (every component non-negative).
    pub fn apply_size(self, size: [u32; 3]) -> [u32; 3] {
        [0, 1, 2].map(|k| size[usize::from(self.axis[k])])
    }
}

/// A dense box of voxels with a write mask, x fastest (`x + sx (y + sy z)`).
///
/// Invariants kept by every constructor: `voxels.len() == sx * sy * sz`, `mask` holds
/// `voxels.len().div_ceil(64)` words with every bit past the last voxel clear, and a voxel
/// whose mask bit is clear stores [`AIR`]. Two prefabs that write the same materials to the
/// same cells are therefore equal, and have the same [`Prefab::id`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Prefab {
    size: [u32; 3],
    voxels: Vec<MaterialId>,
    mask: Vec<u64>,
}

impl Prefab {
    /// A prefab of `size` from `voxels` (x fastest) and a write mask (bit `i % 64` of word
    /// `i / 64` for voxel `i`); `None` writes every voxel. A masked-out voxel's value is
    /// dropped (stored as [`AIR`]).
    ///
    /// # Panics
    ///
    /// When `voxels` does not hold exactly `sx * sy * sz` values, or `mask` does not hold
    /// exactly `voxels.len().div_ceil(64)` words.
    pub fn new(size: [u32; 3], mut voxels: Vec<MaterialId>, mask: Option<Vec<u64>>) -> Self {
        let n = size.iter().map(|&s| s as usize).product::<usize>();
        assert_eq!(
            voxels.len(),
            n,
            "prefab voxel count does not match its size"
        );
        let words = n.div_ceil(64);
        let mut mask = mask.unwrap_or_else(|| vec![u64::MAX; words]);
        assert_eq!(
            mask.len(),
            words,
            "prefab mask length does not match its size"
        );
        if n % 64 != 0
            && let Some(last) = mask.last_mut()
        {
            *last &= (1u64 << (n % 64)) - 1;
        }
        for (i, v) in voxels.iter_mut().enumerate() {
            if mask[i / 64] >> (i % 64) & 1 == 0 {
                *v = AIR;
            }
        }
        Self { size, voxels, mask }
    }

    /// A cube of edge `edge` whose surface voxels are `shell` and whose interior is air,
    /// every voxel written. A fixture for tests, benches and testbeds.
    pub fn vault(edge: u32, shell: MaterialId) -> Self {
        let e = edge as usize;
        let mut voxels = Vec::with_capacity(e * e * e);
        for z in 0..edge {
            for y in 0..edge {
                for x in 0..edge {
                    let surface = [x, y, z].iter().any(|&c| c == 0 || c + 1 == edge);
                    voxels.push(if surface { shell } else { AIR });
                }
            }
        }
        Self::new([edge; 3], voxels, None)
    }

    pub fn size(&self) -> [u32; 3] {
        self.size
    }

    /// Voxels in the box, written or not.
    pub fn len(&self) -> usize {
        self.voxels.len()
    }

    pub fn is_empty(&self) -> bool {
        self.voxels.is_empty()
    }

    /// The index of local voxel `l`, or `None` outside the box.
    fn index(&self, l: [i32; 3]) -> Option<usize> {
        let mut i = 0usize;
        for k in (0..3).rev() {
            let c = u32::try_from(l[k]).ok().filter(|&c| c < self.size[k])?;
            i = i * self.size[k] as usize + c as usize;
        }
        Some(i)
    }

    /// Whether voxel index `i` is written.
    #[inline]
    pub(crate) fn written(&self, i: usize) -> bool {
        self.mask[i / 64] >> (i % 64) & 1 != 0
    }

    /// The material at voxel index `i`; [`AIR`] where the mask is clear.
    #[inline]
    pub(crate) fn raw(&self, i: usize) -> MaterialId {
        self.voxels[i]
    }

    /// The material a stamp writes at local voxel `l`: `None` outside the box and where the
    /// mask is clear.
    pub fn get(&self, l: [i32; 3]) -> Option<MaterialId> {
        let i = self.index(l)?;
        self.written(i).then(|| self.voxels[i])
    }

    /// Written voxels.
    pub fn written_count(&self) -> usize {
        self.mask.iter().map(|w| w.count_ones() as usize).sum()
    }

    /// FNV-1a 64 over the size (three little-endian `u32`), the voxels (little-endian `u16`,
    /// x fastest) and the mask words (little-endian `u64`).
    pub fn id(&self) -> u64 {
        let mut h = Fnv1a::default();
        for s in self.size {
            h.write(&s.to_le_bytes());
        }
        for v in &self.voxels {
            h.write(&v.to_le_bytes());
        }
        for w in &self.mask {
            h.write(&w.to_le_bytes());
        }
        h.finish()
    }

    /// This prefab turned by `rot` and moved back so its minimum corner is local
    /// `(0, 0, 0)`. Destination component `k` reads source axis `axis[k]`, mirrored across
    /// that axis when the row is negative.
    pub fn rotated(&self, rot: Rot) -> Self {
        if rot == Rot::IDENTITY {
            return self.clone();
        }
        let size = rot.apply_size(self.size);
        let n = self.voxels.len();
        let mut voxels = vec![AIR; n];
        let mut mask = vec![0u64; n.div_ceil(64)];
        // Source stride of each source axis, so a destination step is one signed add.
        let stride = [
            1i64,
            i64::from(self.size[0]),
            i64::from(self.size[0]) * i64::from(self.size[1]),
        ];
        let mut step = [0i64; 3];
        let mut start = 0i64;
        for (k, step) in step.iter_mut().enumerate() {
            let (a, neg) = rot.row(k);
            let a = usize::from(a);
            if neg {
                *step = -stride[a];
                start += (i64::from(self.size[a]) - 1).max(0) * stride[a];
            } else {
                *step = stride[a];
            }
        }
        let mut d = 0usize;
        for z in 0..i64::from(size[2]) {
            for y in 0..i64::from(size[1]) {
                let mut s = start + y * step[1] + z * step[2];
                for _ in 0..size[0] {
                    let si = s as usize;
                    voxels[d] = self.voxels[si];
                    if self.written(si) {
                        mask[d / 64] |= 1 << (d % 64);
                    }
                    s += step[0];
                    d += 1;
                }
            }
        }
        Self { size, voxels, mask }
    }
}
