//! The boundary between the engine's server and the game's logic (ADR-0010).
//!
//! Both traits are object-safe and free of generics: a logic is driven as `Box<dyn Logic>`,
//! and a logic compiled as a component receives the host as imports. Inputs and the
//! broadcast are opaque bytes the logic defines.

use vxe_world::{CellCoord, EditError, Fixed3, Hit, MaterialId, Op, OpResult, Region, VoxelCoord};

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
    /// a failed `init`) and its restarts are spent; it stays stopped for the rest of the run.
    Stopped { tick: u64, since: u64 },
    /// Before its call for `tick`, the logic was started afresh from its seed (a new
    /// instance and `init`) because its call for tick `after` failed; `restarts` counts the
    /// restarts so far, this one included. Not a failure of `tick` itself.
    Restarted {
        tick: u64,
        after: u64,
        restarts: u32,
    },
}

impl std::fmt::Display for LogicError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Load(m) => write!(f, "logic failed to load: {m}"),
            Self::Trap { tick, message } => write!(f, "logic trapped at tick {tick}: {message}"),
            Self::Fuel { tick } => write!(f, "logic ran out of fuel at tick {tick}"),
            Self::Memory { tick } => write!(f, "logic exceeded its memory cap at tick {tick}"),
            Self::Restarted {
                tick,
                after,
                restarts,
            } => write!(
                f,
                "logic restarted from its seed at tick {tick} after it failed at tick {after} \
                 (restart {restarts})"
            ),
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
    /// Applies an op of any kind now. An applied op is logged and announced as applied; a
    /// refused one is announced as refused and not logged. Either way the logic receives the
    /// outcome.
    fn apply(&mut self, op: Op) -> Result<OpResult, EditError>;
    /// The material of a voxel, or `None` when its chunk is not resident.
    fn material_at(&self, v: VoxelCoord) -> Option<MaterialId>;
    /// The first non-air voxel along `origin + dir * t / 256`, `0 <= t <= max`.
    fn raycast(&self, origin: Fixed3, dir: Fixed3, max: i64) -> Option<Hit>;
    /// [`Host::raycast`] plus the steps the world's walk took
    /// (`vxe_world::World::raycast_with_stats`), 0 for a ray the world refused. A host that
    /// charges a logic for its calls' work reads the steps here. The default reports 0.
    fn raycast_with_steps(&self, origin: Fixed3, dir: Fixed3, max: i64) -> (Option<Hit>, u64) {
        (self.raycast(origin, dir, max), 0)
    }
    /// Voxels per material in the inclusive box `lo..=hi`, ascending by material.
    fn count_in_box(&self, lo: VoxelCoord, hi: VoxelCoord) -> Vec<(MaterialId, u64)>;
    /// The regions that lost their support after an edit that changed the cells `changed`
    /// (`vxe_world::find_unsupported` over the server's coarse grid, with the support
    /// parameters and anchors the server is configured with). A caller that knows only an
    /// op's aabb passes [`cells_in`] of it.
    fn unsupported(&self, changed: &[CellCoord]) -> Vec<Region>;
}

/// The cells (8³-voxel bricks) overlapping the inclusive voxel box `aabb`, ascending; none
/// for `None`.
pub fn cells_in(aabb: Option<(VoxelCoord, VoxelCoord)>) -> Vec<CellCoord> {
    let Some((lo, hi)) = aabb else {
        return Vec::new();
    };
    let e = vxe_world::BRICK_EDGE;
    let (lo, hi) = (lo.0.map(|v| v.div_euclid(e)), hi.0.map(|v| v.div_euclid(e)));
    let mut out = Vec::new();
    for x in lo[0]..=hi[0] {
        for y in lo[1]..=hi[1] {
            for z in lo[2]..=hi[2] {
                out.push(CellCoord([x, y, z]));
            }
        }
    }
    out
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
    /// The restart made before the last `tick`, if one was ([`LogicError::Restarted`]),
    /// cleared by reading it. The server asks after every tick. The default is `None`; a
    /// hosted logic that starts a fresh instance after a failure reports it here.
    fn take_restart(&mut self) -> Option<LogicError> {
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
