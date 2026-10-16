//! Renderer: uploads the world's bricks to the GPU and ray-marches them in compute shaders
//! through `wgpu` (ADR-0001). Shaders are WGSL under `src/shaders/`. Nothing here branches
//! on the graphics backend.
//!
//! The first real contents land with Plan 0001 (`docs/plans/`).
