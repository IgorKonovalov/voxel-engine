//! A brick: 8 × 8 × 8 voxels stored either as one material or as 512 raw ids (ADR-0002).

use crate::coords::{BRICK_EDGE, BRICK_VOXELS, index_in_brick};
use crate::material::{AIR, MaterialId};

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

    pub fn is_full(&self) -> bool {
        matches!(self, Self::Full(_))
    }

    /// The raw voxels of a `Full` brick, writable. Expands a `Uniform` brick first.
    pub(crate) fn expand(&mut self) -> &mut [MaterialId; BRICK_VOXELS] {
        if let Self::Uniform(m) = *self {
            *self = Self::Full(Box::new([m; BRICK_VOXELS]));
        }
        match self {
            Self::Full(v) => v,
            Self::Uniform(_) => unreachable!("expanded above"),
        }
    }

    /// Collapses a `Full` brick whose voxels are all equal back to `Uniform`.
    pub(crate) fn collapse(&mut self) {
        if let Self::Full(v) = self {
            let first = v[0];
            if v.iter().all(|&m| m == first) {
                *self = Self::Uniform(first);
            }
        }
    }
}
