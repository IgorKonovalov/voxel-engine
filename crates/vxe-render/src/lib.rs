//! Renderer: uploads the world's bricks to the GPU and ray-marches them in compute shaders
//! through `wgpu` (ADR-0001). Shaders are WGSL under `src/shaders/`. Nothing here branches
//! on the graphics backend.
//!
//! [`GpuWorld`] mirrors a `vxe_world::World` into the ADR-0002 buffers; [`Frame`] draws it
//! (its hit pass is the [`Marcher`]) and [`Presenter`] puts the result on a surface; [`HudOverlay`] draws text over it.

pub mod attr_work;
pub mod attrib;
pub mod camera;
pub mod font8x8;
pub mod gpu_world;
pub mod hud;
pub mod light;
pub mod marcher;
pub mod present;
pub mod readback;

pub use attr_work::{AttrWorkStats, MirrorView};
pub use camera::{Camera, View};
pub use gpu_world::{
    BELT_CHUNK_BYTES, GpuWorld, MirrorConfig, MirrorError, ShardLayout, SyncStats, Window,
};
pub use hud::{HudColour, HudOverlay};
pub use light::LightParams;
pub use marcher::{
    CandidateDraw, FRAME_PASSES, Frame, GRADIENT_STRATUM, HistoryKill, Lighting,
    LightingResolution, LodParams, LookParams, MAX_EDITS, MAX_VIEWS, MarchTimer, Marcher,
    NormalMode, PassTimer, Sampler, ShadowWalk, ToneMap, ViewMode,
};
pub use present::Presenter;
pub use {glam, wgpu};

/// The source of the hit pass, whose walk and helpers the other frame shaders are compiled
/// after; for headless validation.
pub const MARCH_WGSL: &str = include_str!("shaders/march.wgsl");
/// The light pass, compiled after [`MARCH_WGSL`].
pub const LIGHT_WGSL: &str = include_str!("shaders/light.wgsl");
/// The gradient pass, compiled after [`MARCH_WGSL`] and [`LIGHT_WGSL`].
pub const GRADIENT_WGSL: &str = include_str!("shaders/gradient.wgsl");
/// The temporal pass, compiled after [`MARCH_WGSL`].
pub const TEMPORAL_WGSL: &str = include_str!("shaders/temporal.wgsl");
/// The variance estimate and the a-trous filter, compiled after [`MARCH_WGSL`].
pub const ATROUS_WGSL: &str = include_str!("shaders/atrous.wgsl");
/// The composite pass, compiled after [`MARCH_WGSL`].
pub const COMPOSITE_WGSL: &str = include_str!("shaders/composite.wgsl");
/// The source of the contour-attribute pass, for headless validation.
pub const ATTRIB_WGSL: &str = include_str!("shaders/attrib.wgsl");

/// Limits to request from `adapter`: the defaults, with the storage-binding and buffer-size
/// limits raised toward what a brick pool of `pool_bytes` needs in one shard, capped at what
/// the adapter offers; a pool past the cap spreads over more shards (`ShardLayout`). The
/// storage buffers per stage are raised to 10: four pool shards and the six other bindings
/// (`OTHER_STORAGE_BUFFERS`).
pub fn limits_for(adapter: &wgpu::Adapter, pool_bytes: u64) -> wgpu::Limits {
    let offered = adapter.limits();
    let mut l = wgpu::Limits::default();
    l.max_storage_buffers_per_shader_stage = l
        .max_storage_buffers_per_shader_stage
        .max(gpu_world::MAX_SHARDS + gpu_world::OTHER_STORAGE_BUFFERS);
    l.max_storage_buffer_binding_size = offered
        .max_storage_buffer_binding_size
        .min(pool_bytes.max(l.max_storage_buffer_binding_size));
    l.max_buffer_size = offered
        .max_buffer_size
        .min(pool_bytes.max(l.max_buffer_size));
    l
}
