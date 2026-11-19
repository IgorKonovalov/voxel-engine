//! The fly-through's entities (ADR-0050, ADR-0051, ADR-0062): the library of predefined
//! models ([`library`]), the belt and placement mode ([`belt`]), the chooser's rows and the
//! ghost ([`picker`]), each entry's still icon ([`icon`]), the built-in walker made in code
//! ([`walker`]), the placed entities and their playback ([`placed`]), a body's mover
//! ([`mover`]), its wander ([`wander`]) and the movers' tick ([`tick`]), and the models kept
//! in the frame's entity pool ([`pool`]). The engine
//! knows none of them: these are the testbed's.

pub mod belt;
pub mod icon;
pub mod library;
pub mod mover;
pub mod picker;
pub mod placed;
pub mod pool;
pub mod tick;
pub mod walker;
pub mod wander;

pub use placed::*;
pub use pool::*;
