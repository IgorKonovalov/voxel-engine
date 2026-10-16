//! World sources: pure functions from a chunk coordinate to that chunk's voxels.

use crate::brick::Brick;
use crate::chunk::Chunk;
use crate::coords::{CHUNK_BRICKS, ChunkCoord};
use crate::material::MaterialId;

/// Produces the voxels of one chunk. Implementations must be pure functions of `coord` (and
/// their own fields, such as a seed), because chunks are generated in parallel in any order.
///
/// `out` arrives as all air with every slot dirty.
pub trait WorldSource {
    fn chunk(&self, coord: ChunkCoord, out: &mut Chunk);
}

/// Every voxel is one material.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UniformSource(pub MaterialId);

impl WorldSource for UniformSource {
    fn chunk(&self, _coord: ChunkCoord, out: &mut Chunk) {
        for slot in 0..CHUNK_BRICKS {
            out.set_brick(slot, Brick::Uniform(self.0));
        }
    }
}
