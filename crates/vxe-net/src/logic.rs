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

/// Why a logic could not be loaded or could not finish a call. A native logic never fails;
/// a hosted one (`vxe-script-wasm`) fails when its component cannot load, traps (a panic is
/// a trap), runs out of its fuel for the tick, or outgrows its memory cap. A hosted logic
/// that failed a call is stopped: every later tick reports [`LogicError::Stopped`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogicError {
    /// The logic could not be loaded, linked or instantiated.
    Load(String),
    /// The logic trapped during the call for `tick`.
    Trap { tick: u64, message: String },
    /// The logic used up its fuel for `tick`.
    Fuel { tick: u64 },
    /// The logic tried to grow its memory past its cap during `tick`.
    Memory { tick: u64 },
    /// The logic was not called for `tick` because its call for tick `since` failed (0 for
    /// a failed `init`); it stays stopped for the rest of the run.
    Stopped { tick: u64, since: u64 },
}

impl std::fmt::Display for LogicError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Load(m) => write!(f, "logic failed to load: {m}"),
            Self::Trap { tick, message } => write!(f, "logic trapped at tick {tick}: {message}"),
            Self::Fuel { tick } => write!(f, "logic ran out of fuel at tick {tick}"),
            Self::Memory { tick } => write!(f, "logic exceeded its memory cap at tick {tick}"),
            Self::Stopped { tick, since } => {
                write!(
                    f,
                    "logic not run at tick {tick}: stopped since it failed at tick {since}"
                )
            }
        }
    }
}

impl std::error::Error for LogicError {}

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
    /// Why the last `init` or `tick` failed, if it did, cleared by reading it. The server
    /// asks after every call. A native logic never fails (a panic is a panic), so the
    /// default is `None`; a hosted logic reports its trap, fuel or memory cap here.
    fn take_error(&mut self) -> Option<LogicError> {
        None
    }
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
