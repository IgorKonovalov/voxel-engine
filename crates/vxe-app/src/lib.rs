//! Application shell: window, input, surface and frame loop on `winit` + `wgpu`.
//!
//! Thin and optional. A game that owns its own loop composes `vxe-world` and `vxe-render`
//! directly and never links this crate (ADR-0001: the engine is a library, not a framework).
//!
//! The first real contents land with Plan 0001 (`docs/plans/`).
