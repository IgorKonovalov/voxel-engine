//! The fly-through's reusable parts: the cave world source its binary, tests and examples
//! share. The engine ships no terrain; this cave is the testbed's own.

pub mod cave;

use vxe_world::{Material, MaterialFlags, MaterialTable};

/// The cave's table: air, rock, ore, and the indestructible shell, at the ids `cave` uses.
/// It lives here rather than in `cave.rs` so that file stays free of floating point.
pub fn cave_materials() -> MaterialTable {
    let mut t = MaterialTable::new();
    let row = |name: &str, albedo: [f32; 3]| Material {
        albedo,
        ..Material::named(name)
    };
    assert_eq!(t.push(row("rock", [0.42, 0.38, 0.34])), cave::ROCK);
    assert_eq!(t.push(row("ore", [0.72, 0.45, 0.20])), cave::ORE);
    let mut shell = row("shell", [0.12, 0.12, 0.14]);
    shell.flags = MaterialFlags::INDESTRUCTIBLE;
    assert_eq!(t.push(shell), cave::SHELL);
    t
}
