//! Renderer: uploads the world's bricks to the GPU and ray-marches them in compute shaders
//! through `wgpu` (ADR-0001). Shaders are WGSL under `src/shaders/`. Nothing here branches
//! on the graphics backend.
//!
//! [`GpuWorld`] mirrors a `vxe_world::World` into the ADR-0002 buffers; [`Marcher`] draws it
//! and [`Presenter`] puts the result on a surface.

pub mod camera;
pub mod gpu_world;
pub mod light;
pub mod marcher;
pub mod present;
pub mod readback;

pub use camera::Camera;
pub use gpu_world::{GpuWorld, MirrorConfig, MirrorError, ShardLayout, SyncStats, Window};
pub use light::LightParams;
pub use marcher::{LodParams, MarchTimer, Marcher};
pub use present::Presenter;
pub use {glam, wgpu};

/// The source of the primary-ray shader, for headless validation.
pub const MARCH_WGSL: &str = include_str!("shaders/march.wgsl");

/// Limits to request from `adapter`: the defaults, with the storage-binding and buffer-size
/// limits raised toward what a brick pool of `pool_bytes` needs in one shard, capped at what
/// the adapter offers; a pool past the cap spreads over more shards (`ShardLayout`). The
/// storage buffers per stage stay at the WebGPU default of 8: four pool shards and the four
/// other bindings.
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
