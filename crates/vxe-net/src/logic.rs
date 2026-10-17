//! The boundary between the engine's server and the game's logic (ADR-0010).
//!
//! Both traits are object-safe and free of generics: a logic is driven as `Box<dyn Logic>`,
//! and a logic compiled as a component receives the host as imports. Inputs and the
//! broadcast are opaque bytes the logic defines.

use vxe_world::{EditError, EditOp, EditResult, Fixed3, Hit, MaterialId, VoxelCoord};

/// One client input, as the server hands it to the logic.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ClientInput {
    /// The client's index, in attach order from 0.
    pub client: u32,
    pub bytes: Vec<u8>,
}

/// What a logic's tick hands back to the server.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TickOutput {
    /// Sent to every client as one event for the tick; nothing is sent when empty.
    pub broadcast: Vec<u8>,
}

/// What the logic may do to and ask of the authoritative world.
pub trait Host {
    /// Applies an edit now. An applied op is logged and announced as applied; a refused one
    /// is announced as refused and not logged. Either way the logic receives the outcome.
    fn apply(&mut self, op: EditOp) -> Result<EditResult, EditError>;
    /// The material of a voxel, or `None` when its chunk is not resident.
    fn material_at(&self, v: VoxelCoord) -> Option<MaterialId>;
    /// The first non-air voxel along `origin + dir * t / 256`, `0 <= t <= max`.
    fn raycast(&self, origin: Fixed3, dir: Fixed3, max: i64) -> Option<Hit>;
    /// Voxels per material in the inclusive box `lo..=hi`, ascending by material.
    fn count_in_box(&self, lo: VoxelCoord, hi: VoxelCoord) -> Vec<(MaterialId, u64)>;
}

/// The game's server logic. Re-simulation reproduces a world only when the logic is a pure
/// function of its seed and its inputs: no clock, no unseeded randomness, no state from
/// outside. `Send` lets a server be shared between threads that submit ops.
pub trait Logic: Send {
    /// Called once, by [`crate::Server::new`], before any tick.
    fn init(&mut self, host: &mut dyn Host, seed: u64);
    /// Called once per server tick with the inputs consumed in it, in consumption order.
    fn tick(&mut self, host: &mut dyn Host, tick: u64, inputs: &[ClientInput]) -> TickOutput;
}

/// A logic that does nothing: the server only applies directly submitted ops.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoLogic;

impl Logic for NoLogic {
    fn init(&mut self, _host: &mut dyn Host, _seed: u64) {}

    fn tick(&mut self, _host: &mut dyn Host, _tick: u64, _inputs: &[ClientInput]) -> TickOutput {
        TickOutput::default()
    }
}
