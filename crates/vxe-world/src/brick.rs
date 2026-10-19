//! A brick: 8 × 8 × 8 voxels stored either as one material or as 512 raw ids (ADR-0002).

use crate::coords::{BRICK_EDGE, BRICK_VOXELS, index_in_brick};
use crate::material::{AIR, MaterialId};

/// The 512 raw voxels of a `Full` brick.
pub(crate) type Voxels = [MaterialId; BRICK_VOXELS];

/// Arrays ready to back expanded bricks. Each stays boxed in the list because it moves into
/// [`Brick::Full`] as the box it is, without a copy or a new allocation.
#[allow(
    clippy::vec_box,
    reason = "the boxes move into Brick::Full; unboxed arrays would be copied into a new box"
)]
pub(crate) type SpareArrays = Vec<Box<Voxels>>;

/// 8³ voxels, indexed x-fastest (`x + 8 y + 64 z`).
///
/// Invariant kept by every mutator in this crate: a brick whose voxels are all one material
/// is `Uniform`, never a `Full` brick of equal values. Callers that build `Full` bricks by
/// hand go through [`Brick::from_voxels`], which collapses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Brick {
    Uniform(MaterialId),
    Full(Box<[MaterialId; BRICK_VOXELS]>),
}

impl Default for Brick {
    fn default() -> Self {
        Self::Uniform(AIR)
    }
}

impl Brick {
    /// A brick from 512 voxels, collapsed to `Uniform` when they are all equal.
    pub fn from_voxels(v: Box<[MaterialId; BRICK_VOXELS]>) -> Self {
        let first = v[0];
        if v.iter().all(|&m| m == first) {
            Self::Uniform(first)
        } else {
            Self::Full(v)
        }
    }

    /// A brick evaluated per voxel from brick-local coordinates (components in `0..8`).
    pub fn from_fn(mut f: impl FnMut([i32; 3]) -> MaterialId) -> Self {
        let mut v = Box::new([AIR; BRICK_VOXELS]);
        for z in 0..BRICK_EDGE {
            for y in 0..BRICK_EDGE {
                for x in 0..BRICK_EDGE {
                    v[index_in_brick([x, y, z])] = f([x, y, z]);
                }
            }
        }
        Self::from_voxels(v)
    }

    /// The material at brick index `i`.
    pub fn get(&self, i: usize) -> MaterialId {
        match self {
            Self::Uniform(m) => *m,
            Self::Full(v) => v[i],
        }
    }

    /// One bit per 2 × 2 × 2 block, set when the block holds a non-air voxel. Block
    /// `(bx, by, bz)` (components in `0..4`) is bit `bx + 4 by + 16 bz`.
    pub fn occupancy_mask(&self) -> u64 {
        match self {
            Self::Uniform(AIR) => 0,
            Self::Uniform(_) => u64::MAX,
            Self::Full(v) => {
                let mut mask = 0;
                for (i, &m) in v.iter().enumerate() {
                    if m != AIR {
                        let (x, y, z) = (i % 8 / 2, i / 8 % 8 / 2, i / 64 / 2);
                        mask |= 1 << (x + 4 * y + 16 * z);
                    }
                }
                mask
            }
        }
    }

    /// One bit per voxel, set when the voxel is not air: bit `i % 64` of word `i / 64` for
    /// brick index `i`, so word `z` is layer `z` with bit `x + 8 y`.
    pub fn solid_bits(&self) -> [u64; 8] {
        match self {
            Self::Uniform(AIR) => [0; 8],
            Self::Uniform(_) => [u64::MAX; 8],
            Self::Full(v) => {
                let mut bits = [0u64; 8];
                for (i, &m) in v.iter().enumerate() {
                    bits[i / 64] |= u64::from(m != AIR) << (i % 64);
                }
                bits
            }
        }
    }

    pub fn is_full(&self) -> bool {
        matches!(self, Self::Full(_))
    }

    /// The raw voxels of a `Full` brick, writable. Expands a `Uniform` brick first, into an
    /// array taken from `spare` when it holds one (every voxel is overwritten), else into a
    /// fresh allocation.
    pub(crate) fn expand(&mut self, spare: &mut SpareArrays) -> &mut Voxels {
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

    /// Collapses a `Full` brick whose voxels are all equal back to `Uniform`, and pushes its
    /// array onto `spare`.
    pub(crate) fn collapse(&mut self, spare: &mut SpareArrays) {
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
}
