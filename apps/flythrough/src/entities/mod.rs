//! The fly-through's entities (ADR-0050, ADR-0051): the library of predefined models
//! ([`library`]), the belt and placement mode ([`belt`]), the chooser's rows and the ghost
//! ([`picker`]), each entry's still icon ([`icon`]), the built-in walker made in code ([`walker`]), the placed entities and their
//! playback ([`placed`]), and the models kept in the frame's entity pool ([`pool`]). The engine
//! knows none of them: these are the testbed's.

pub mod belt;
pub mod icon;
pub mod library;
pub mod picker;
pub mod placed;
pub mod pool;
pub mod walker;

pub use placed::*;
pub use pool::*;
