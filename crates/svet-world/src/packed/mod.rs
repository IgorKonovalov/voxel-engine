//! The packed brick of ADR-0012: a 512-bit occupancy mask, a palette of the brick's non-air
//! materials in ascending id, and bit planes that index the palette, in one of five fixed-size
//! classes chosen by the count `n` of non-air materials.
//!
//! [`crate::Brick::Full`] holds a [`Packed`]. This module carries the encoding, `decode` and
//! `get`, and the planar edit kernel (carve, fill, replace as word operations with popcount
//! tallies).
//!
//! Word layout of a planar class with `p` planes (`Wide` is 512 raw ids instead):
//!
//! ```text
//! words 0..8          mask: word z, bit x + 8 y, set when the voxel is not air
//! words 8..8+8p       plane k at 8 + 8k .. 16 + 8k, the same bit layout; a set voxel's
//!                     palette index is the integer whose bit k is its plane-k bit
//! words 8+8p..        palette: entry j at bits 16 (j % 4) .. of word 8 + 8p + j / 4,
//!                     ascending material id, air never, unused entries zero
//! ```
//!
//! Canonical form, which `encode` produces and every kernel call leaves:
//!
//! 1. the class is [`PackedClass::of`] the count `n` of palette entries;
//! 2. the palette is strictly ascending and every entry has at least one voxel;
//! 3. a mask bit is set exactly when the voxel is not air, and plane bits are zero under a
//!    clear mask bit;
//! 4. a brick whose 512 voxels are equal is [`crate::Brick::Uniform`].
//!
//! So equal voxels have equal bytes, and the derived equality is exact voxel equality. Every
//! step is integer: shifts, masks and popcounts; nothing reads a clock or a hash map.

mod compose;
mod format;
mod kernel;

pub use format::{Packed, PackedClass, RowSpans, bounds_of, box_words, shape_words};
pub use kernel::{KernelMode, PackedStats};
