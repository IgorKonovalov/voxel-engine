//! The fly-through's stats HUD: what it reads each frame, the cadence at which it samples the
//! world, and the rows it prints, on screen through `vxe_render::HudOverlay` and at exit as
//! text. The overlay draws whatever rows it is handed; the rows here are the testbed's.
//!
//! Per-frame fields come only from sources that cost nothing to read: the [`FrameStats`]
//! ring, the pass timers, the last mirror sync, the mirror's counters, the last edit and the
//! camera. The world's chunk and brick counts walk every resident chunk, and reading the
//! process's RSS from `/proc` is not free either, so both are sampled at a [`Cadence`] and
//! shown with the sample's age.

mod modes;
mod stats;
mod upscaler;

pub use modes::*;
pub use stats::*;
pub use upscaler::*;
