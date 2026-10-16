//! Voxel world: storage, materials, edit operations, spatial queries and streaming.
//!
//! This crate is headless and deterministic. It links no GPU API, reads no wall clock and
//! owns no unseeded randomness. The same edit operation applied to the same world produces
//! identical voxels and identical edit results on every platform (CLAUDE.md, ADR-0001).
//!
//! Storage is a brick map (ADR-0002): `u16` voxels in 8³ bricks that are `Uniform` or `Full`,
//! 64³ chunks behind per-chunk locks, edits applied in place in ascending chunk order.
//!
//! ```
//! use vxe_world::*;
//!
//! let mut table = MaterialTable::new();
//! let rock = table.push(Material::named("rock"));
//! let world = World::generate(ChunkExtent::new([-1, -1, -1], [1, 1, 1]), &UniformSource(rock), &table);
//! let r = world.apply(&EditOp::sphere(centre_of_voxel([0, 0, 0]), 2 * FIXED_ONE, Mode::Carve)).unwrap();
//! assert_eq!(r.changed_from, [(rock, 33)]);
//! ```

mod brick;
mod chunk;
mod coords;
mod edit;
mod fixed;
mod material;
mod shape;
mod source;
mod store;

pub use brick::Brick;
pub use chunk::Chunk;
pub use coords::{
    BRICK_EDGE, BRICK_VOXELS, CHUNK_BRICKS, CHUNK_EDGE, CHUNK_EDGE_BRICKS, ChunkCoord, ChunkExtent,
    VoxelCoord, brick_of_slot, brick_slot, brick_slot_of_local, index_in_brick,
};
pub use edit::{EditError, EditOp, EditResult, Mode};
pub use fixed::{FIXED_HALF, FIXED_ONE, Fixed3, centre_of_voxel};
pub use material::{AIR, Material, MaterialFlags, MaterialId, MaterialTable};
pub use shape::Shape;
pub use source::{UniformSource, WorldSource};
pub use store::{ChunkStore, World, WorldStats};
