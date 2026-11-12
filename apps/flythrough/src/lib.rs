//! The fly-through's reusable parts: the cave world source its binary, tests and examples
//! share, its server logic (the `flythrough-logic` crate, as [`logic`]) and the choice of its
//! native or hosted build, save replay, the streaming defaults, and the small measurement
//! helpers its readings use. The engine ships no terrain; this cave is the testbed's own.
//! The binary's run lives here too: [`cli`] parses its flags, [`setup`] builds its world and
//! [`app`] runs it.

pub mod app;
mod audio;
mod autopilot;
pub mod castle;
pub mod cave;
mod census;
pub mod cli;
pub(crate) mod edits;
mod fly;
pub mod hud;
mod look;
mod pad;
pub mod path;
mod replay;
pub mod report;
mod run;
mod scene;
pub mod setup;
pub mod showcase;
mod streaming;
#[cfg(test)]
mod test_support;
pub mod thin_rig;
pub mod toggles;
pub use flythrough_logic as logic;

pub use census::*;
pub use edits::ScrollSteps;
pub use look::*;
pub use replay::*;
pub use report::{now, percentile};
pub use scene::*;

/// The rubble a collapsing region leaves when `R` has the fly-through's logic remove
/// unsupported regions instead of dropping them. The cave never generates it.
pub use logic::RUBBLE;
