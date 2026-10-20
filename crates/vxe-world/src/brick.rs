//! A brick: 8 × 8 × 8 voxels stored either as one material or as a packed box of occupancy
//! mask, palette and bit planes (ADR-0012; the box is `packed.rs`).

use crate::coords::{BRICK_EDGE, BRICK_VOXELS, index_in_brick};
use crate::material::{AIR, MaterialId};
use crate::packed::{Packed, PackedClass};

/// The 512 voxels of a brick, decoded.
pub(crate) type Voxels = [MaterialId; BRICK_VOXELS];

/// A free list of boxed id arrays for [`RawBrick`]s. Each stays boxed because it moves into
/// [`RawBrick::Full`] as the box it is, without a copy or a new allocation.
#[allow(
    clippy::vec_box,
    reason = "the boxes move into RawBrick::Full; unboxed arrays would be copied into a new box"
)]
pub(crate) type SpareArrays = Vec<Box<Voxels>>;

/// 8³ voxels, indexed x-fastest (`x + 8 y + 64 z`).
///
/// Invariant kept by every constructor and mutator: a brick whose voxels are all one material
/// is `Uniform`, and a `Full` brick's box is in the canonical form of `packed.rs` (its class
/// is the function of its material count, its palette ascending with every entry used). So
/// the derived equality is exact voxel equality. A `Full` brick is built only by
/// [`Brick::from_voxels`], [`Brick::from_fn`] and [`Brick::modify`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Brick {
    Uniform(MaterialId),
    Full(Packed),
}

impl Default for Brick {
    fn default() -> Self {
        Self::Uniform(AIR)
    }
}

impl Brick {
    /// A brick from 512 voxels, `Uniform` when they are all equal.
    pub fn from_voxels(v: Box<[MaterialId; BRICK_VOXELS]>) -> Self {
        Self::encode(&v)
    }

    /// A brick evaluated per voxel from brick-local coordinates (components in `0..8`).
    pub fn from_fn(mut f: impl FnMut([i32; 3]) -> MaterialId) -> Self {
        let mut v = [AIR; BRICK_VOXELS];
        for z in 0..BRICK_EDGE {
            for y in 0..BRICK_EDGE {
                for x in 0..BRICK_EDGE {
                    v[index_in_brick([x, y, z])] = f([x, y, z]);
                }
            }
        }
        Self::encode(&v)
    }

    /// The material at brick index `i`.
    pub fn get(&self, i: usize) -> MaterialId {
        match self {
            Self::Uniform(m) => *m,
            Self::Full(p) => p.get(i),
        }
    }

    /// Writes all 512 voxels into `out`.
    pub fn decode(&self, out: &mut [MaterialId; BRICK_VOXELS]) {
        match self {
            Self::Uniform(m) => out.fill(*m),
            Self::Full(p) => p.decode(out),
        }
    }

    /// Runs `f` over the brick's 512 voxels, decoded into a scratch array, then stores the
    /// result in canonical form (`Uniform` when all equal). The cold write path: it decodes
    /// and encodes the whole brick whatever `f` changes.
    pub fn modify<R>(&mut self, f: impl FnOnce(&mut [MaterialId; BRICK_VOXELS]) -> R) -> R {
        let mut v = [AIR; BRICK_VOXELS];
        self.decode(&mut v);
        let r = f(&mut v);
        *self = Self::encode(&v);
        r
    }

    /// One bit per 2 × 2 × 2 block, set when the block holds a non-air voxel. Block
    /// `(bx, by, bz)` (components in `0..4`) is bit `bx + 4 by + 16 bz`. Derived from
    /// [`Brick::solid_bits`].
    pub fn occupancy_mask(&self) -> u64 {
        let bits = self.solid_bits();
        let mut mask = 0u64;
        for bz in 0..4 {
            // The two layers of block row `bz`, folded.
            let w = bits[2 * bz] | bits[2 * bz + 1];
            for by in 0..4 {
                // The two rows of the block row, folded into one byte.
                let r = ((w >> (16 * by)) | (w >> (16 * by + 8))) as u8;
                for bx in 0..4 {
                    if r >> (2 * bx) & 0b11 != 0 {
                        mask |= 1 << (bx + 4 * by + 16 * bz);
                    }
                }
            }
        }
        mask
    }

    /// One bit per voxel, set when the voxel is not air: bit `i % 64` of word `i / 64` for
    /// brick index `i`, so word `z` is layer `z` with bit `x + 8 y`. A `Full` brick's mask.
    pub fn solid_bits(&self) -> [u64; 8] {
        match self {
            Self::Uniform(AIR) => [0; 8],
            Self::Uniform(_) => [u64::MAX; 8],
            Self::Full(p) => p.mask(),
        }
    }

    pub fn is_full(&self) -> bool {
        matches!(self, Self::Full(_))
    }

    /// The class of a `Full` brick's box; `None` for `Uniform`.
    pub fn class(&self) -> Option<PackedClass> {
        match self {
            Self::Uniform(_) => None,
            Self::Full(p) => Some(p.class()),
        }
    }

    /// Bytes the brick holds beyond its slot: its box's class size, 0 for `Uniform`.
    pub fn payload_bytes(&self) -> usize {
        match self {
            Self::Uniform(_) => 0,
            Self::Full(p) => p.bytes(),
        }
    }
}

/// A brick as raw ids, for code that writes voxel by voxel before storing: a uniform value or
/// 512 ids, not canonical until [`RawBrick::into_brick`]. The collapse ops compose in it, and
/// the kernel bench's row writer runs on it as the reference write.
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RawBrick {
    Uniform(MaterialId),
    Full(Box<Voxels>),
}

impl RawBrick {
    pub fn of(b: &Brick) -> Self {
        match b {
            Brick::Uniform(m) => Self::Uniform(*m),
            Brick::Full(_) => {
                let mut v = Box::new([AIR; BRICK_VOXELS]);
                b.decode(&mut v);
                Self::Full(v)
            }
        }
    }

    /// The canonical brick of these voxels.
    pub fn into_brick(self) -> Brick {
        match self {
            Self::Uniform(m) => Brick::Uniform(m),
            Self::Full(v) => Brick::encode(&v),
        }
    }

    pub fn get(&self, i: usize) -> MaterialId {
        match self {
            Self::Uniform(m) => *m,
            Self::Full(v) => v[i],
        }
    }

    /// The writable ids, expanding a `Uniform` brick first into an array from `spare` when it
    /// holds one (every id is overwritten), else into a fresh allocation.
    pub fn expand(&mut self, spare: &mut SpareArrays) -> &mut Voxels {
        if let Self::Uniform(m) = *self {
            let v = match spare.pop() {
                Some(mut v) => {
                    v.fill(m);
                    v
                }
                None => Box::new([m; BRICK_VOXELS]),
            };
            *self = Self::Full(v);
        }
        match self {
            Self::Full(v) => v,
            Self::Uniform(_) => unreachable!("expanded above"),
        }
    }

    /// Back to `Uniform` when every id is equal, the array pushed onto `spare`.
    pub fn collapse(&mut self, spare: &mut SpareArrays) {
        if let Self::Full(v) = self {
            let first = v[0];
            if v.iter().all(|&m| m == first)
                && let Self::Full(v) = std::mem::replace(self, Self::Uniform(first))
            {
                spare.push(v);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solid_bits_hold_one_bit_per_non_air_voxel_layer_by_layer() {
        assert_eq!(Brick::Uniform(AIR).solid_bits(), [0; 8]);
        assert_eq!(Brick::Uniform(3).solid_bits(), [u64::MAX; 8]);
        // Voxels (1, 2, 0), (7, 7, 7) and (0, 4, 3): bits x + 8 y of layer z.
        let b = Brick::from_fn(|l| match l {
            [1, 2, 0] | [7, 7, 7] => 2,
            [0, 4, 3] => 5,
            _ => AIR,
        });
        let mut want = [0u64; 8];
        want[0] = 1 << 17;
        want[7] = 1 << 63;
        want[3] = 1 << 32;
        assert_eq!(b.solid_bits(), want);
    }

    /// The 2^3 occupancy mask derived from the voxel mask equals the per-voxel definition.
    #[test]
    fn the_occupancy_mask_follows_the_voxels() {
        let mut s = 0x0CC0u64;
        for _ in 0..64 {
            let b = Brick::from_fn(|_| {
                s = crate::hash::mix64(s);
                if s.is_multiple_of(13) { 2 } else { AIR }
            });
            let mut want = 0u64;
            for i in 0..BRICK_VOXELS {
                if b.get(i) != AIR {
                    let (x, y, z) = (i % 8 / 2, i / 8 % 8 / 2, i / 64 / 2);
                    want |= 1 << (x + 4 * y + 16 * z);
                }
            }
            assert_eq!(b.occupancy_mask(), want);
        }
    }

    #[test]
    fn a_brick_is_sixteen_bytes() {
        assert_eq!(size_of::<Brick>(), 16);
    }
}
