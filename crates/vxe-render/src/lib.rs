//! Renderer: uploads the world's bricks to the GPU and ray-marches them in compute shaders
//! through `wgpu` (ADR-0001). Shaders are WGSL under `src/shaders/`. Nothing here branches
//! on the graphics backend.
//!
//! [`GpuWorld`] mirrors a `vxe_world::World` into the ADR-0002 buffers; [`Frame`] draws it
//! (its hit pass is the [`Marcher`]) and [`Presenter`] puts the result on a surface; [`HudOverlay`] draws text over it.

pub mod attr_work;
pub mod attrib;
pub mod camera;
pub mod exposure;
pub mod font8x8;
pub mod fsr2;
pub mod gpu_world;
pub mod hud;
pub mod light;
pub mod marcher;
pub mod present;
pub mod probes;
pub mod readback;

pub use attr_work::{AttrWorkStats, MirrorView};
pub use attrib::{PlaneFit, PlaneOffset, SurfaceFit};
pub use camera::{Camera, View};
pub use exposure::{
    AutoExposure, EXPOSURE_WGSL, Exposure, ExposureReadout, ExposureState, Metering,
};
pub use fsr2::{Fsr2, Fsr2Settings, UpscalerKind};
pub use gpu_world::{
    BELT_CHUNK_BYTES, BinStats, GpuWorld, MirrorConfig, MirrorError, ShardLayout, SyncStats, Window,
};
pub use hud::{HudColour, HudOverlay};
pub use light::LightParams;
pub use marcher::{
    CandidateDraw, DEFAULT_EMISSIVE_HISTORY, DEFAULT_REUSE_M_CAP, FRAME_PASSES, Frame,
    GRADIENT_STRATUM, HistoryKill, Lighting, LightingResolution, LodParams, LookParams, MAX_EDITS,
    MAX_VIEWS, MarchTimer, Marcher, NormalMode, PassTimer, RenderScale, Sampler, ShadowWalk,
    SurfaceTuning, ToneMap, ViewMode, jitter_at,
};
pub use present::Presenter;
pub use probes::ProbeVolume;
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
/// The probe volume's reads and the gather pass, compiled after [`MARCH_WGSL`].
pub const PROBE_GATHER_WGSL: &str = include_str!("shaders/probe_gather.wgsl");
/// The probe update's trace and blend, compiled after [`MARCH_WGSL`], [`LIGHT_WGSL`] and
/// [`PROBE_GATHER_WGSL`].
pub const PROBE_UPDATE_WGSL: &str = include_str!("shaders/probe_update.wgsl");
/// The source of the contour-attribute pass, for headless validation.
pub const ATTRIB_WGSL: &str = include_str!("shaders/attrib.wgsl");
pub use marcher::{SHARPEN_WGSL, UPSCALE_WGSL};

/// A limit the frame's shaders need past what the adapter offers.
#[derive(Clone, Debug)]
pub struct LimitsError {
    /// The `wgpu::Limits` field.
    pub limit: &'static str,
    pub needed: u64,
    pub offered: u64,
    /// The request with the limit at what the shaders need: a device request with it fails
    /// in `wgpu` on the same limit.
    pub requested: Box<wgpu::Limits>,
}

impl std::fmt::Display for LimitsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the adapter offers {} {}, the frame's shaders need {}",
            self.offered, self.limit, self.needed
        )
    }
}

impl std::error::Error for LimitsError {}

/// Storage buffers one shader stage of the frame binds at most: four pool shards
/// (`gpu_world::MAX_SHARDS`), the five other bindings (`OTHER_STORAGE_BUFFERS`) and the probe
/// passes' two (`probes::PROBE_STORAGE_BUFFERS`).
pub const STORAGE_BUFFERS_NEEDED: u32 =
    gpu_world::MAX_SHARDS + gpu_world::OTHER_STORAGE_BUFFERS + probes::PROBE_STORAGE_BUFFERS;

/// Limits to request from `adapter`: [`limits_within`] its offered limits.
pub fn limits_for(adapter: &wgpu::Adapter, pool_bytes: u64) -> Result<wgpu::Limits, LimitsError> {
    limits_within(&adapter.limits(), pool_bytes)
}

/// Limits to request from an adapter offering `offered`: the defaults, with the
/// storage-binding and buffer-size limits raised toward what a brick pool of `pool_bytes`
/// needs in one shard and the storage buffers per stage raised to
/// [`STORAGE_BUFFERS_NEEDED`], each capped at what is offered; a pool past the size cap
/// spreads over the shards (`ShardLayout`). `Err` names the storage-buffer count when the
/// offer is below what the shaders bind.
pub fn limits_within(offered: &wgpu::Limits, pool_bytes: u64) -> Result<wgpu::Limits, LimitsError> {
    let mut l = wgpu::Limits::default();
    let needed = l
        .max_storage_buffers_per_shader_stage
        .max(STORAGE_BUFFERS_NEEDED);
    l.max_storage_buffer_binding_size = offered
        .max_storage_buffer_binding_size
        .min(pool_bytes.max(l.max_storage_buffer_binding_size));
    l.max_buffer_size = offered
        .max_buffer_size
        .min(pool_bytes.max(l.max_buffer_size));
    l.max_storage_buffers_per_shader_stage =
        needed.min(offered.max_storage_buffers_per_shader_stage);
    if offered.max_storage_buffers_per_shader_stage < STORAGE_BUFFERS_NEEDED {
        return Err(LimitsError {
            limit: "max_storage_buffers_per_shader_stage",
            needed: u64::from(STORAGE_BUFFERS_NEEDED),
            offered: u64::from(offered.max_storage_buffers_per_shader_stage),
            requested: Box::new(wgpu::Limits {
                max_storage_buffers_per_shader_stage: needed,
                ..l
            }),
        });
    }
    Ok(l)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer(storage_buffers: u32, binding: u64) -> wgpu::Limits {
        wgpu::Limits {
            max_storage_buffers_per_shader_stage: storage_buffers,
            max_storage_buffer_binding_size: binding,
            max_buffer_size: binding,
            ..wgpu::Limits::default()
        }
    }

    #[test]
    fn the_limits_are_capped_at_the_offer() {
        let l = limits_within(&offer(16, 1 << 30), 4 << 30).expect("16 storage buffers");
        assert_eq!(l.max_storage_buffers_per_shader_stage, 11);
        assert_eq!(l.max_storage_buffer_binding_size, 1 << 30);
        assert_eq!(l.max_buffer_size, 1 << 30);
        let l = limits_within(&offer(11, 1 << 30), 16 << 20).expect("11 storage buffers");
        assert_eq!(l.max_storage_buffers_per_shader_stage, 11);
        assert_eq!(
            l.max_storage_buffer_binding_size,
            wgpu::Limits::default().max_storage_buffer_binding_size
        );
    }

    #[test]
    fn an_offer_below_the_shaders_bindings_is_refused_by_name() {
        let e = limits_within(&offer(8, 1 << 30), 16 << 20).expect_err("8 storage buffers");
        assert_eq!(e.limit, "max_storage_buffers_per_shader_stage");
        assert_eq!((e.needed, e.offered), (11, 8));
        assert_eq!(e.requested.max_storage_buffers_per_shader_stage, 11);
        assert_eq!(
            e.to_string(),
            "the adapter offers 8 max_storage_buffers_per_shader_stage, the frame's shaders need 11"
        );
    }
}
