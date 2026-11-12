//! Fixtures the `vxe-world` benches share: the clock, the solid rock world and the collapse
//! bench's box op and slab.
#![allow(
    dead_code,
    unused_imports,
    reason = "shared across bench binaries; each uses a subset"
)]

mod clock;

pub use clock::{now, timed};
use vxe_world::*;

/// Rock (id 1) over chunks `[-2, 2)³`.
pub fn solid_world() -> World {
    let table: MaterialTable = [Material::named("rock")].into_iter().collect();
    World::generate(
        ChunkExtent::new([-2, -2, -2], [2, 2, 2]),
        &UniformSource(1),
        &table,
    )
}

/// An axis box op over voxel corners `lo` and `hi`.
pub fn box_op(lo: [i32; 3], hi: [i32; 3], mode: Mode) -> EditOp {
    let f = |v: [i32; 3]| Fixed3(v.map(|c| i64::from(c) * FIXED_ONE));
    EditOp::box_(f(lo), f(hi), mode)
}

/// The 3 x 3 cells `x, y in 3..=5` at `z = 5`.
pub fn slab_cells() -> Vec<CellCoord> {
    let mut v = Vec::new();
    for x in 3..=5 {
        for y in 3..=5 {
            v.push(CellCoord([x, y, 5]));
        }
    }
    v
}
