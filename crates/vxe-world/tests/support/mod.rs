//! Shared fixtures for the integration tests.
#![allow(dead_code, reason = "shared across test binaries; each uses a subset")]

use vxe_world::*;

/// Rows 0 (air), 1..=5 (plain), with 4 flagged indestructible.
pub fn test_table() -> MaterialTable {
    let mut t = MaterialTable::new();
    for i in 1..=5u16 {
        let mut m = Material::named(format!("m{i}"));
        if i == 4 {
            m.flags = MaterialFlags::INDESTRUCTIBLE;
        }
        assert_eq!(t.push(m), i);
    }
    t
}

/// Chunks `[-2, 2)³`.
pub fn test_extent() -> ChunkExtent {
    ChunkExtent::new([-2, -2, -2], [2, 2, 2])
}

/// `UniformSource(3)` over chunks `[-2, 2)³`.
pub fn solid3_world() -> World {
    World::generate(test_extent(), &UniformSource(3), &test_table())
}

/// A source evaluated per voxel from world voxel coordinates; bricks collapse when uniform.
pub struct FnSource<F>(pub F);

impl<F: Fn(VoxelCoord) -> MaterialId> WorldSource for FnSource<F> {
    fn chunk(&self, coord: ChunkCoord, out: &mut Chunk) {
        let base = coord.min_voxel().0;
        for slot in 0..CHUNK_BRICKS {
            let b = brick_of_slot(slot);
            out.set_brick(
                slot,
                Brick::from_fn(|l| {
                    (self.0)(VoxelCoord(
                        [0, 1, 2].map(|k| base[k] + b[k] * BRICK_EDGE + l[k]),
                    ))
                }),
            );
        }
    }
}
