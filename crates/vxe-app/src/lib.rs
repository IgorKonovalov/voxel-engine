//! Application shell: window, input, surface and frame loop on `winit` + `wgpu`.
//!
//! Thin and optional. A game that owns its own loop composes `vxe-world` and `vxe-render`
//! directly and never links this crate (ADR-0001: the engine is a library, not a framework).
//!
//! [`run`] opens a window and calls an [`App`] once per frame with the [`Input`] gathered
//! since the previous one.

pub mod input;
pub mod shell;

pub use input::{Axis, Input, KeyCode, MouseButton, PadButton};
pub use shell::{App, Flow, Gpu, ShellConfig, ShellError, run};
pub use {wgpu, winit};
