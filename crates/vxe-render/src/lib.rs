//! Renderer: uploads the world's bricks to the GPU and ray-marches them in compute shaders
//! through `wgpu` (ADR-0001). Shaders are WGSL under `src/shaders/`. Nothing here branches
//! on the graphics backend.
//!
//! [`GpuWorld`] mirrors a `vxe_world::World` into the ADR-0002 buffers; [`Frame`] draws it
//! (its hit pass is the [`Marcher`]) and [`Presenter`] puts the result on a surface; [`HudOverlay`] draws text over it.

#![deny(unreachable_pub, unnameable_types)]

mod bake;
mod camera;
mod exposure;
mod field;
mod font8x8;
mod frame;
mod fsr2;
mod gpu_util;
mod gpu_world;
mod hud;
mod light;
mod present;
mod probes;
mod readback;

/// Readings: the readbacks of a texture or buffer, the march's step histogram, the bake
/// queue step's stages and regimes, and the light summaries a getter returns. For tools and
/// tests that measure the renderer; the API it reads may change with what is measured.
pub mod debug {
    pub use crate::bake::{
        LightSummaries, ParentTable, Regime, STEP_STAGES, StepStages, StepTally,
    };
    pub use crate::probes::ProbeRead;
    pub use crate::readback::{
        StepHistogram, f16_to_f32, read_buffer, read_float, read_rgba8, read_rgba16f, read_rgba16u,
        read_rgba32f,
    };
}

/// Not API, and no semver promise: the GPU contract (the mirror's record and slot layout, the
/// bake's record and view constants, the field's band units), the WGSL sources and composers
/// not at the root, and pure functions the white-box tests assert against.
#[doc(hidden)]
pub mod internals {
    pub use crate::bake::{
        BAKE_ROUND, BRICK_CELL, BRICK_WORDS, ChunkFaces, DIG_FADE, FADE, FacePrep,
        QUEUE_VIEW_MARGIN_DEG, STORE_COARSE_READS, SUMMARY_AT, VIEW_IN, VIEW_OUT, VIEW_SHELL,
        angle_term, dig_ring, face_classes, in_near_shell, list_world_chunk, rgb9e5_unpack,
        tone_level, view_half_angle, view_half_angles,
    };
    pub use crate::exposure::{METER_LOG2_LIMIT, STATE_HELD};
    pub use crate::field::{
        BAND_CUBES_BIT, CRISP_COUNT_BIT, MAX_BAND_UNITS, SMOOTH_COUNT_BIT, band_index,
        band_slot_units,
    };
    pub use crate::font8x8::{FONT, glyph};
    pub use crate::frame::{JITTER_PHASES, composite_hdr_source};
    pub use crate::fsr2::{
        FSR2_ACCUMULATE_WGSL, FSR2_DEPTH_CLIP_WGSL, FSR2_INPUT_WGSL, FSR2_LOCK_WGSL,
        FSR2_LUMINANCE_PYRAMID_WGSL, FSR2_PASSES, FSR2_RCAS_WGSL, FSR2_RECONSTRUCT_WGSL,
        FSR2_TONE_WGSL, fsr2_source, jitter_offset, jitter_phases,
    };
    pub use crate::gpu_world::{
        ABSENT, BELT_KEEP_CHUNKS, BIN_COUNT, BINS, BITS_WORDS, CLASS_MASK, CLASS_SHIFT,
        CLASS_UNIFORM, CLASS_WIDE, EMPTY_BIT, ENTRY_WORD, FULL_BIT, HEADER_BAND_BIT,
        HEADER_BAND_PREFIX_WORD, HEADER_BAND_WORD, HEADER_TRANSLUCENT_WORD, LIGHT_SAMPLE_BYTES,
        LOD_BIT, LOD_LEVEL_ENTRY, LightSample, MATERIAL_FLOATS, MATERIAL_MASK, PAGE_BYTES,
        PAGE_UNITS, PALETTE_ENTRIES, PLACEHOLDER_BYTES, RECORD_BYTES, RECORD_MASK, RECORD_WORDS,
        SLOT_BINS, TRANSLUCENT_BIT, UNIT_BYTES, UNIT_MASK, UNIT_WORDS, band_bin_of, bin_of,
        budget_for, class_code, exposed_emitters, layer_prefix, payload_units, slot_units,
        slots_per_page,
    };
    pub use crate::probes::{
        MAX_AGE, ProbeUniform, RAYS_PER_PROBE, SLICES, TestProbe, WINDOW_FRAMES,
    };
}

pub use bake::{
    BakeError, BakeProgress, BakeQueue, BakeReport, BakeSettings, Budget, DEFAULT_BAKE_EDIT_MS,
    DEFAULT_BAKE_SLICE_MS, DEFAULT_COARSE_SAMPLES, DEFAULT_NEAR, DigReport, FaceOutcome,
    MissingLight, QUEUE_ANGLE_WEIGHT, QUEUE_BRICK_SAMPLES, QUEUE_REFINE_WEIGHT, QueueOrder,
    QueueStats, QueueView, ReachMode, Refine, Store, StoreStats, queue_settings,
};
pub use camera::{Camera, View};
pub use exposure::{
    AutoExposure, EXPOSURE_WGSL, Exposure, ExposureReadout, ExposureState, Metering,
};
pub use frame::{
    CrackShares, FRAME_PASSES, Frame, GRADIENT_STRATUM, HistoryKill, LightWeights, Lighting,
    LightingResolution, LodParams, LookParams, MAX_EDITS, MAX_VIEWS, MarchTimer, Marcher,
    NormalMode, PassTimer, RenderScale, ShadowWalk, StoreRead, SurfaceTuning, ToneMap, ViewMode,
    jitter_at,
};
pub use frame::{DEFAULT_PROBE_FREEZE, Params, Upscaler};
pub use fsr2::{Fsr2, Fsr2Settings, UpscalerKind};
pub use gpu_world::{
    BELT_CHUNK_BYTES, BinStats, DEFAULT_BUDGET_BYTES, DEFAULT_LIGHT_CHUNK_CAP,
    DEFAULT_LIGHT_RADIUS, DEFAULT_LIGHT_SAMPLES, GpuWorld, MirrorConfig, MirrorError, ShardLayout,
    SyncClock, SyncPoint, SyncStats, Window,
};
pub use hud::{HudColour, HudOverlay};
pub use light::LightParams;
pub use present::Presenter;
pub use probes::{DEFAULT_H_MOVE, DEFAULT_PROBE_SPACING, ProbeVolume};
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
/// The source of the field pass (`field.rs`), for headless validation.
pub const FIELD_WGSL: &str = include_str!("shaders/field.wgsl");
pub use frame::{SHARPEN_WGSL, UPSCALE_WGSL};

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
/// (`gpu_world::MAX_SHARDS`), the six other bindings (`OTHER_STORAGE_BUFFERS`) and the probe
/// passes' two (`probes::PROBE_STORAGE_BUFFERS`).
pub const STORAGE_BUFFERS_NEEDED: u32 =
    gpu_world::MAX_SHARDS + gpu_world::OTHER_STORAGE_BUFFERS + probes::PROBE_STORAGE_BUFFERS;

/// The bytes the store's pool binds by default ([`bake::DEFAULT_BAKE_POOL_BYTES`]): 256 MiB.
pub const STORE_BINDING_BYTES: u64 = bake::DEFAULT_BAKE_POOL_BYTES;

/// Limits to request from `adapter`: [`limits_within`] its offered limits.
pub fn limits_for(adapter: &wgpu::Adapter, pool_bytes: u64) -> Result<wgpu::Limits, LimitsError> {
    limits_within(&adapter.limits(), pool_bytes)
}

/// Limits to request from an adapter offering `offered`: the defaults, with the
/// storage-binding and buffer-size limits raised toward what a brick pool of `pool_bytes`
/// needs in one shard, and at least toward the store's pool at its default size
/// ([`STORE_BINDING_BYTES`]), and the storage buffers per stage raised to
/// [`STORAGE_BUFFERS_NEEDED`], each capped at what is offered; a pool past the size cap
/// spreads over the shards (`ShardLayout`). `Err` names the storage-buffer count when the
/// offer is below what the shaders bind.
pub fn limits_within(offered: &wgpu::Limits, pool_bytes: u64) -> Result<wgpu::Limits, LimitsError> {
    let mut l = wgpu::Limits::default();
    let needed = l
        .max_storage_buffers_per_shader_stage
        .max(STORAGE_BUFFERS_NEEDED);
    let want = pool_bytes.max(STORE_BINDING_BYTES);
    l.max_storage_buffer_binding_size = offered
        .max_storage_buffer_binding_size
        .min(want.max(l.max_storage_buffer_binding_size));
    l.max_buffer_size = offered.max_buffer_size.min(want.max(l.max_buffer_size));
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
        assert_eq!(l.max_storage_buffers_per_shader_stage, 12);
        assert_eq!(l.max_storage_buffer_binding_size, 1 << 30);
        assert_eq!(l.max_buffer_size, 1 << 30);
        let l = limits_within(&offer(12, 1 << 30), 16 << 20).expect("12 storage buffers");
        assert_eq!(l.max_storage_buffers_per_shader_stage, 12);
        assert_eq!(l.max_storage_buffer_binding_size, STORE_BINDING_BYTES);
        let l = limits_within(&offer(12, 128 << 20), 16 << 20).expect("12 storage buffers");
        assert_eq!(l.max_storage_buffer_binding_size, 128 << 20);
    }

    #[test]
    fn an_offer_below_the_shaders_bindings_is_refused_by_name() {
        let e = limits_within(&offer(8, 1 << 30), 16 << 20).expect_err("8 storage buffers");
        assert_eq!(e.limit, "max_storage_buffers_per_shader_stage");
        assert_eq!((e.needed, e.offered), (12, 8));
        assert_eq!(e.requested.max_storage_buffers_per_shader_stage, 12);
        assert_eq!(
            e.to_string(),
            "the adapter offers 8 max_storage_buffers_per_shader_stage, the frame's shaders need 12"
        );
    }
}
