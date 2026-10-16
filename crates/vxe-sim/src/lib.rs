//! Character collision against a `vxe-world` brick map: a vertical cylinder swept by integer
//! conservative advancement (ADR-0005).
//!
//! Like `vxe-world`, this crate is headless and deterministic: every position, distance and
//! velocity is an integer in fixed-point voxel units (1/256 voxel), nothing reads a clock, and
//! every tie is broken by a fixed key, so the same inputs give bit-identical results on every
//! platform.
//!
//! ```
//! use vxe_sim::{Body, sweep};
//! use vxe_world::*;
//!
//! let mut table = MaterialTable::new();
//! let rock = table.push(Material::named("rock"));
//! let world = World::generate(ChunkExtent::new([-1, -1, -1], [1, 1, 0]), &UniformSource(rock), &table);
//! // A body standing on the top face of the solid half-space z < 0, dropped from 1 voxel up.
//! let body = Body { foot: Fixed3([128, 128, 256]), radius: 64, height: 512 };
//! let s = sweep(&world, &body, Fixed3([0, 0, -1000]));
//! assert_eq!(s.moved, Fixed3([0, 0, -256]));
//! assert!(s.contact.unwrap().is_floor());
//! ```

mod body;
mod controller;
mod sweep;

pub use body::{Body, Contact, isqrt};
pub use controller::{
    CharacterController, CharacterState, ControllerParams, MAX_CONTACTS, MoveInput, MoveResult,
};
pub use sweep::{
    MAX_SWEEP_ITERATIONS, Sweep, distance2, distance2_with_stats, overlaps, sweep, sweep_with_stats,
};
