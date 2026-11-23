//! The render settings (ADR-0055): one plain struct holding every setting of the frame and the
//! mirror by value, the apply class of each of its fields ([`RenderSettings::FIELDS`]), and
//! what applying a value changed ([`SettingsChange`]). [`crate::Frame::settings`] reads a
//! frame's settings back and [`crate::Frame::apply_settings`] applies them through the frame's
//! setters.

use crate::bake::{Budget, MissingLight, ReachMode};
use crate::exposure::Metering;
use crate::frame::{
    Lighting, LightingResolution, LodParams, LookParams, MAX_LIGHTS, RenderScale, Upscaler,
};
use crate::fsr2::{Fsr2Settings, UpscalerKind};
use crate::gpu_world::{MirrorConfig, Window};

/// When a changed field takes effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ApplyClass {
    /// From the next frame, with no visible change but the setting's own.
    Live,
    /// From the next frame, with the temporal history restarted: the image settles again.
    Settle,
    /// The caller recreates the mirror, the streamer or the frame;
    /// [`crate::Frame::apply_settings`] reports the field and leaves it.
    Rebuild,
}

impl ApplyClass {
    /// The lower-case name `--settings` prints.
    pub fn name(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Settle => "settle",
            Self::Rebuild => "rebuild",
        }
    }
}

/// Which upscaler a frame runs, without its tuning ([`RenderSettings::fsr2`] and
/// [`RenderSettings::own`] carry that).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UpscalerChoice {
    /// No upscaler: at equal sizes the composite's image is the frame's.
    #[default]
    Off,
    /// The upscaler of ADR-0024.
    Own,
    /// FSR 2.
    Fsr2,
}

impl UpscalerChoice {
    /// The [`UpscalerKind`] this choice runs with `own`'s or `fsr2`'s tuning.
    pub fn kind(self, own: Upscaler, fsr2: Fsr2Settings) -> UpscalerKind {
        match self {
            Self::Off => UpscalerKind::Off,
            Self::Own => UpscalerKind::Own(own),
            Self::Fsr2 => UpscalerKind::Fsr2(fsr2),
        }
    }

    /// The choice `kind` makes.
    pub fn of(kind: &UpscalerKind) -> Self {
        match kind {
            UpscalerKind::Off => Self::Off,
            UpscalerKind::Own(_) => Self::Own,
            UpscalerKind::Fsr2(_) => Self::Fsr2,
        }
    }
}

/// The live queue's knobs ([`crate::BakeQueue`]); each is `Live`. A frame keeps them while no
/// queue runs and hands them to the queue [`crate::Frame::start_bake_queue`] starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BakeKnobs {
    /// The bake budget (ADR-0052; [`crate::Frame::set_bake_budget`]).
    pub budget: Budget,
    /// Samples a brick direction of the coarse pass, 0 for none
    /// ([`crate::Frame::set_bake_coarse`]).
    pub coarse_samples: u32,
    /// The chunks (Chebyshev) around the camera's past which a chunk's first block takes the
    /// coarse pass.
    pub near: i32,
    /// The full-first radius ([`crate::Frame::set_bake_full_first`]).
    pub full_first: Option<i32>,
    /// The reach mode ([`crate::Frame::set_bake_reach`]).
    pub reach: ReachMode,
    /// The slice's growth a frame while a dig's bricks wait, in ms
    /// ([`crate::Frame::set_bake_digs`]).
    pub edit_ms: f64,
    /// Whether a dig that touched lit bricks re-bakes the chunks within the light radius.
    pub dig_reach: bool,
    /// The sample tiers a brick direction by band, near, mid and far (ADR-0080;
    /// [`crate::Frame::set_bake_tiers`]).
    pub tiers: [u32; 3],
    /// The slice's drain and line values (ADR-0080 rule 3; [`crate::Frame::set_bake_drain`]);
    /// `None` is the budget alone.
    pub drain: Option<crate::bake::Drain>,
}

impl Default for BakeKnobs {
    /// The live queue's own defaults ([`crate::BakeQueue::new`]).
    fn default() -> Self {
        Self {
            budget: Budget::default(),
            coarse_samples: crate::bake::DEFAULT_COARSE_SAMPLES,
            near: crate::bake::DEFAULT_NEAR,
            full_first: Some(crate::bake::DEFAULT_NEAR),
            reach: ReachMode::default(),
            edit_ms: crate::bake::DEFAULT_BAKE_EDIT_MS,
            dig_reach: true,
            tiers: crate::bake::DEFAULT_TIERS,
            drain: Some(crate::bake::DEFAULT_DRAIN),
        }
    }
}

/// The `Rebuild` group: what the mirror, the streamer or the entity pool is created with. A
/// frame applies none of it; the caller recreates what a change needs and then tells the frame
/// ([`crate::Frame::built_with`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BuildSettings {
    /// The mirror's VRAM reservation ([`MirrorConfig::budget_bytes`]).
    pub mirror_budget_bytes: u64,
    /// Chunk records the mirror reserves ([`MirrorConfig::record_chunks`]).
    pub record_chunks: Option<u64>,
    /// Light samples the mirror's light-sample buffer holds ([`MirrorConfig::light_samples`]).
    pub light_samples: u32,
    /// Most light samples one chunk keeps ([`MirrorConfig::light_chunk_cap`]).
    pub light_chunk_cap: u32,
    /// Chunks around the focus whose emitters light the frame ([`MirrorConfig::light_radius`]).
    pub light_radius: i32,
    /// Light samples one sync writes ([`MirrorConfig::light_sync_samples`]).
    pub light_sync_samples: u32,
    /// Bytes a sync writes ([`MirrorConfig::sync_budget_bytes`]).
    pub sync_budget_bytes: u64,
    /// Time a sync spends ([`MirrorConfig::sync_budget_time`];
    /// [`crate::GpuWorld::set_sync_budget_time`]).
    pub sync_budget_time: Option<std::time::Duration>,
    /// The mirror holds the field surface's band slots ([`crate::GpuWorld::set_field_surface`]).
    pub field_surface: bool,
    /// Bytes of the entity pool ([`crate::Frame::set_entity_mirror`]).
    pub entity_pool_bytes: u64,
    /// The streamer's radii, for the caller's `vxe_world::StreamConfig`.
    pub residency: vxe_world::Residency,
    /// The streamer's generation workers; `None` for the caller's own count.
    pub workers: Option<usize>,
}

impl Default for BuildSettings {
    /// A mirror of [`MirrorConfig::new`] with no field slots, the default entity pool, and a
    /// residency of 16 chunks full, 24 in mips, one of hysteresis.
    fn default() -> Self {
        let m = MirrorConfig::new(Window::Follow { side: 1 });
        Self {
            mirror_budget_bytes: m.budget_bytes,
            record_chunks: m.record_chunks,
            light_samples: m.light_samples,
            light_chunk_cap: m.light_chunk_cap,
            light_radius: m.light_radius,
            light_sync_samples: m.light_sync_samples,
            sync_budget_bytes: m.sync_budget_bytes,
            sync_budget_time: m.sync_budget_time,
            field_surface: false,
            entity_pool_bytes: crate::entities::DEFAULT_ENTITY_POOL_BYTES,
            residency: vxe_world::Residency {
                r_full: 16,
                r_lod: 24,
                hysteresis: 1,
            },
            workers: None,
        }
    }
}

impl BuildSettings {
    /// A mirror configuration of `window` with these values.
    pub fn mirror_config(&self, window: Window) -> MirrorConfig {
        MirrorConfig {
            sync_budget_bytes: self.sync_budget_bytes,
            sync_budget_time: self.sync_budget_time,
            budget_bytes: self.mirror_budget_bytes,
            record_chunks: self.record_chunks,
            light_samples: self.light_samples,
            light_chunk_cap: self.light_chunk_cap,
            light_radius: self.light_radius,
            light_sync_samples: self.light_sync_samples,
            ..MirrorConfig::new(window)
        }
    }
}

/// Every setting of the frame and the mirror, by value (ADR-0055). [`RenderSettings::FIELDS`]
/// names each leaf and its [`ApplyClass`].
#[derive(Clone, Debug, PartialEq)]
pub struct RenderSettings {
    /// The lighting ([`crate::Frame::set_lighting`]); `Live`, but for
    /// [`Lighting::bake_pool_bytes`], which is `Rebuild` (the store is allocated once).
    pub lighting: Lighting,
    /// The surface look ([`crate::Frame::set_look`]); `Live`, but for the view, which is
    /// `Settle` (it drops the upscaler's history).
    pub look: LookParams,
    /// The LOD rule ([`crate::Frame::set_lod`]); `Live`.
    pub lod: LodParams,
    /// The lighting passes' resolution ([`crate::Frame::set_resolution`]); `Settle`.
    pub lighting_resolution: LightingResolution,
    /// The output size over the internal size ([`crate::Frame::set_sizes`]); `Settle`.
    pub scale: RenderScale,
    /// The upscaler ([`crate::Frame::set_upscaler_kind`]); `Settle`.
    pub upscaler: UpscalerChoice,
    /// The views' sub-pixel jitter ([`crate::Frame::set_jitter`]); `Settle`.
    pub jitter: bool,
    /// The own upscaler's sharpening pass, 0 to 1 ([`crate::Frame::set_sharpness`]); `Live`.
    pub sharpness: f32,
    /// FSR 2's knobs (`UpscalerKind::Fsr2`); `Live` (a change of them keeps the history).
    pub fsr2: Fsr2Settings,
    /// The own upscaler's tuning ([`crate::Frame::set_upscaler`]); `Live`.
    pub own: Upscaler,
    /// Slots of the list of lights the frame shades, the headlamp's included, 1 to
    /// [`MAX_LIGHTS`] ([`crate::Frame::set_light_cap`]); `Live`.
    pub light_cap: u32,
    /// How the exposure pass weights the meter under a manual exposure
    /// ([`crate::Frame::set_metering`]); `Live`.
    pub metering: Metering,
    /// The field look's split walk ([`crate::Frame::set_field_split`]); `Live`.
    pub field_split: bool,
    /// Voxels between the probe volume's probes ([`crate::Frame::set_probe_spacing`]); `Live`.
    pub probe_spacing: u32,
    /// The probe volume's fixed side, `None` to follow the lamp's reach
    /// ([`crate::Frame::set_probe_axis`]); `Live`.
    pub probe_axis: Option<u32>,
    /// What a cube face with no brick value reads ([`crate::Frame::set_missing_light`]);
    /// `Live`.
    pub missing_light: MissingLight,
    /// The live queue's knobs; `Live`.
    pub bake: BakeKnobs,
    /// The mirror's, the streamer's and the entity pool's creation values; `Rebuild`.
    pub build: BuildSettings,
    /// The output size the profile is stated at. The engine never applies it: the consumer
    /// picks its output (a headless one may take the hint, a windowed one its window's size).
    /// A profile's frame line and its sample tiers (the bands read the output's focal length,
    /// ADR-0080 rule 2) hold at the hint; a consumer that draws the Portable profile at a
    /// larger output gets neither. The fly-through holds Portable at its hint in a window and
    /// scales the frame up. `Rebuild`.
    pub output_hint: Option<[u32; 2]>,
    /// The colours a consumer marks entities with, the first and the second
    /// ([`crate::Frame::set_mark_colours`], ADR-0074); `Live`, the same in every profile.
    pub mark_colours: [[f32; 3]; 2],
}

impl Default for RenderSettings {
    /// [`Profile::Floor`]'s settings.
    fn default() -> Self {
        Self::profile(Profile::Floor)
    }
}

/// A named quality profile (ADR-0055): a starting value of [`RenderSettings`] the consumer
/// picks and may edit. The engine never picks one by itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Profile {
    /// The floor class at 1080p output (`docs/nfr.md` section 1): the fly-through's look.
    #[default]
    Floor,
    /// The portable class at 720p output: fewer translucent layers and lights, no lamp
    /// bounce, a smaller residency and a halved bake slice.
    Portable,
    /// A fast machine past 60 fps: the floor at internal scale 2.
    Performance,
    /// A capable GPU at 1080p output: the floor with FSR 2 at internal scale 1. Not for the
    /// slow adapter (`docs/nfr.md` section 1).
    High,
}

crate::frame::named_enum!(Profile {
    Floor => "floor",
    Portable => "portable",
    Performance => "performance",
    High => "high",
});

/// The far normal's band every profile holds, in pixels a voxel (ADR-0082: window W1's pick, a
/// part of the calm light by default).
pub const DEFAULT_FAR_NORMAL: [f32; 2] = [4.0, 8.0];

/// Bytes the fly-through's streamed mirror reserves for the floor's residency of 16 chunks
/// full ([`crate::DEFAULT_BUDGET_BYTES`]).
const FLOOR_MIRROR_BYTES: u64 = crate::gpu_world::DEFAULT_BUDGET_BYTES;

impl RenderSettings {
    /// `p`'s settings. Every profile keeps the floor's image-stability choices (the far
    /// normal, the shadow walk, the filter, the temporal and FSR 2's settings).
    ///
    /// | field | Floor | Portable | Performance | High |
    /// |---|---|---|---|---|
    /// | `output_hint` | 1920 x 1080 | 1280 x 720 | none | 1920 x 1080 |
    /// | `scale` | 1.5 | 1.5 | 2 | 1 |
    /// | `upscaler` | FSR 2 | FSR 2 | FSR 2 | FSR 2 |
    /// | `look.translucent_layers` | 4 | 2 | 4 | 4 |
    /// | `light_cap` | 16 | 4 | 16 | 16 |
    /// | `lighting.lamp_bounce` | on | off | on | on |
    /// | `build.residency` | 16, 24, 1 | 12, 18, 1 | 16, 24, 1 | 16, 24, 1 |
    /// | `lod.r_full` | 16 | 12 | 16 | 16 |
    /// | `build.mirror_budget_bytes` | 3 GiB | 3 GiB x (12 / 16)^3 | 3 GiB | 3 GiB |
    /// | `bake.budget` | 1.5 ms a frame | 0.75 ms a frame | 1.5 ms a frame | 1.5 ms a frame |
    /// | `bake.tiers` | 4096, 1024, 256 | 1024, 256, 256 | 4096, 1024, 256 | 4096, 1024, 256 |
    /// | `bake.drain` | 1.5, 1.0 ms | 6.0, 1.0 ms | 1.5, 1.0 ms | 1.5, 1.0 ms |
    pub fn profile(p: Profile) -> Self {
        let floor = Self {
            lighting: Lighting {
                exposure: crate::exposure::Exposure::Auto(crate::AutoExposure::default()),
                lamp_bounce: true,
                store_read: crate::frame::StoreRead::Normal,
                light_weights: crate::frame::LightWeights::Face,
                ..Lighting::default()
            },
            look: LookParams {
                far_normal: DEFAULT_FAR_NORMAL,
                ..LookParams::default()
            },
            lod: LodParams::default(),
            lighting_resolution: LightingResolution::Half,
            scale: RenderScale::OneHalf,
            upscaler: UpscalerChoice::Fsr2,
            jitter: false,
            sharpness: 0.5,
            fsr2: Fsr2Settings {
                sharpness: 0.5,
                ..Fsr2Settings::default()
            },
            own: Upscaler::default(),
            light_cap: MAX_LIGHTS as u32,
            metering: Metering::default(),
            field_split: true,
            probe_spacing: crate::probes::DEFAULT_PROBE_SPACING,
            probe_axis: None,
            missing_light: MissingLight::default(),
            bake: BakeKnobs::default(),
            build: BuildSettings {
                mirror_budget_bytes: FLOOR_MIRROR_BYTES,
                sync_budget_bytes: 2 << 20,
                ..BuildSettings::default()
            },
            output_hint: Some([1920, 1080]),
            mark_colours: crate::entities::MARK_COLOURS,
        };
        match p {
            Profile::Floor => floor,
            Profile::Portable => {
                let residency = vxe_world::Residency {
                    r_full: 12,
                    r_lod: 18,
                    hysteresis: 1,
                };
                let mut s = floor;
                s.output_hint = Some([1280, 720]);
                s.look.translucent_layers = 2;
                s.light_cap = 4;
                s.lighting.lamp_bounce = false;
                s.build.residency = residency;
                s.lod.r_full = residency.r_full;
                s.build.mirror_budget_bytes = FLOOR_MIRROR_BYTES / 64 * 27;
                s.bake.budget = crate::bake::Budget::PerFrame {
                    ms: crate::bake::DEFAULT_BAKE_SLICE_MS / 2.0,
                };
                s.bake.tiers = [1024, 256, 256];
                s.bake.drain = Some(crate::bake::Drain {
                    drain_ms: 6.0,
                    line_ms: 1.0,
                });
                s
            }
            Profile::Performance => Self {
                scale: RenderScale::Two,
                output_hint: None,
                ..floor
            },
            Profile::High => Self {
                scale: RenderScale::One,
                ..floor
            },
        }
    }
}

use ApplyClass::{Live, Rebuild, Settle};

impl RenderSettings {
    /// The owner's picks (Plan 0069 Phase 5), one row per field a profile sets, each value
    /// as [`RenderSettings::table`] prints it, in [`Profile::ALL`]'s order. Every field not
    /// listed is the floor's in every profile. The profiles' tests and the fly-through's
    /// `--settings` test read it.
    #[doc(hidden)]
    pub const PROFILE_PICKS: &'static [(&'static str, [&'static str; Profile::ALL.len()])] = &[
        ("scale", ["OneHalf", "OneHalf", "Two", "One"]),
        ("upscaler", ["Fsr2", "Fsr2", "Fsr2", "Fsr2"]),
        ("look.translucent_layers", ["4", "2", "4", "4"]),
        ("light_cap", ["16", "4", "16", "16"]),
        ("lighting.lamp_bounce", ["true", "false", "true", "true"]),
        ("lod.r_full", ["16", "12", "16", "16"]),
        (
            "build.residency",
            [
                "Residency { r_full: 16, r_lod: 24, hysteresis: 1 }",
                "Residency { r_full: 12, r_lod: 18, hysteresis: 1 }",
                "Residency { r_full: 16, r_lod: 24, hysteresis: 1 }",
                "Residency { r_full: 16, r_lod: 24, hysteresis: 1 }",
            ],
        ),
        (
            "build.mirror_budget_bytes",
            ["3221225472", "1358954496", "3221225472", "3221225472"],
        ),
        (
            "bake.budget",
            [
                "PerFrame { ms: 1.5 }",
                "PerFrame { ms: 0.75 }",
                "PerFrame { ms: 1.5 }",
                "PerFrame { ms: 1.5 }",
            ],
        ),
        (
            "bake.tiers",
            [
                "[4096, 1024, 256]",
                "[1024, 256, 256]",
                "[4096, 1024, 256]",
                "[4096, 1024, 256]",
            ],
        ),
        (
            "bake.drain",
            [
                "Some(Drain { drain_ms: 1.5, line_ms: 1.0 })",
                "Some(Drain { drain_ms: 6.0, line_ms: 1.0 })",
                "Some(Drain { drain_ms: 1.5, line_ms: 1.0 })",
                "Some(Drain { drain_ms: 1.5, line_ms: 1.0 })",
            ],
        ),
        (
            "output_hint",
            [
                "Some([1920, 1080])",
                "Some([1280, 720])",
                "None",
                "Some([1920, 1080])",
            ],
        ),
    ];

    /// Each setting's name, a leaf of the struct (an enum, an `Option` or an array counts as
    /// one), and its apply class.
    pub const FIELDS: &'static [(&'static str, ApplyClass)] = &[
        ("lighting.exposure", Live),
        ("lighting.tone_map", Live),
        ("lighting.lamp", Live),
        ("lighting.candidates", Live),
        ("lighting.shadow_walk", Live),
        ("lighting.filter", Live),
        ("lighting.atrous_iterations", Live),
        ("lighting.history", Live),
        ("lighting.lamp_bounce", Live),
        ("lighting.probe_rays", Live),
        ("lighting.probe_slices", Live),
        ("lighting.probe_freeze", Live),
        ("lighting.lamp_reach", Live),
        ("lighting.probe_move_blend", Live),
        ("lighting.bake_pool_bytes", Rebuild),
        ("lighting.store_read", Live),
        ("lighting.light_floor", Live),
        ("lighting.light_weights", Live),
        ("lighting.young_filter", Live),
        ("lighting.entity_light", Live),
        ("lighting.sky_rays", Live),
        ("lighting.sky_history", Live),
        ("lighting.key_history", Live),
        ("lighting.light_history", Live),
        ("lighting.landing_fade", Live),
        ("lighting.source_fade", Live),
        ("lighting.dig_hold", Live),
        ("lighting.light_history_far", Live),
        ("lighting.far_light_wide", Live),
        ("lighting.penumbra", Live),
        ("lighting.key_far", Live),
        ("lighting.key_far_mix", Live),
        ("lighting.sky_normal", Live),
        ("lighting.sky_bounce", Live),
        ("lighting.key_cap", Live),
        ("lighting.sky_down_skip", Live),
        ("lighting.column_wait", Live),
        ("lighting.key_step", Live),
        ("look.normal_mode", Live),
        ("look.view", Settle),
        ("look.translucent_layers", Live),
        ("look.cutaway", Live),
        ("look.tuning.face_mix", Live),
        ("look.far_normal", Live),
        ("lod.r_full", Live),
        ("lod.forced_level", Live),
        ("lighting_resolution", Settle),
        ("scale", Settle),
        ("upscaler", Settle),
        ("jitter", Settle),
        ("sharpness", Live),
        ("fsr2.sharpness", Live),
        ("fsr2.reactive_translucent", Live),
        ("fsr2.reactive_light", Live),
        ("fsr2.metres_per_voxel", Live),
        ("fsr2.jitter_phases", Live),
        ("fsr2.dither", Live),
        ("own.alpha", Live),
        ("own.alpha_moving", Live),
        ("own.change_rel", Live),
        ("own.change_abs", Live),
        ("own.change_frames", Live),
        ("light_cap", Live),
        ("metering", Live),
        ("field_split", Live),
        ("probe_spacing", Live),
        ("probe_axis", Live),
        ("missing_light", Live),
        ("bake.budget", Live),
        ("bake.coarse_samples", Live),
        ("bake.near", Live),
        ("bake.full_first", Live),
        ("bake.reach", Live),
        ("bake.edit_ms", Live),
        ("bake.dig_reach", Live),
        ("bake.tiers", Live),
        ("bake.drain", Live),
        ("build.mirror_budget_bytes", Rebuild),
        ("build.record_chunks", Rebuild),
        ("build.light_samples", Rebuild),
        ("build.light_chunk_cap", Rebuild),
        ("build.light_radius", Rebuild),
        ("build.light_sync_samples", Rebuild),
        ("build.sync_budget_bytes", Rebuild),
        ("build.sync_budget_time", Rebuild),
        ("build.field_surface", Rebuild),
        ("build.entity_pool_bytes", Rebuild),
        ("build.residency", Rebuild),
        ("build.workers", Rebuild),
        ("output_hint", Rebuild),
        ("mark_colours", Live),
    ];

    /// Each `pub fn set_*` of the crate that sets a field, and the fields it sets: a name, or a
    /// group's prefix ending in a dot.
    pub const SETTERS: &'static [(&'static str, &'static [&'static str])] = &[
        ("set_lighting", &["lighting."]),
        ("set_look", &["look."]),
        ("set_lod", &["lod."]),
        ("set_resolution", &["lighting_resolution"]),
        ("set_upscaler_kind", &["upscaler", "fsr2.", "own."]),
        ("set_jitter", &["jitter"]),
        ("set_sharpness", &["sharpness"]),
        ("set_upscaler", &["own."]),
        ("set_light_cap", &["light_cap"]),
        ("set_metering", &["metering"]),
        ("set_field_split", &["field_split"]),
        ("set_probe_spacing", &["probe_spacing"]),
        ("set_probe_axis", &["probe_axis"]),
        ("set_missing_light", &["missing_light"]),
        ("set_bake_budget", &["bake.budget"]),
        (
            "set_bake_coarse",
            &["bake.coarse_samples", "bake.near", "bake.full_first"],
        ),
        ("set_bake_full_first", &["bake.full_first"]),
        ("set_bake_reach", &["bake.reach"]),
        ("set_bake_digs", &["bake.edit_ms", "bake.dig_reach"]),
        ("set_bake_tiers", &["bake.tiers"]),
        ("set_bake_drain", &["bake.drain"]),
        ("set_entity_mirror", &["build.entity_pool_bytes"]),
        ("set_field_surface", &["build.field_surface"]),
        ("set_sync_budget_time", &["build.sync_budget_time"]),
        ("set_mark_colours", &["mark_colours"]),
    ];

    /// Each `pub fn set_*` of the crate that is not a setting, and why.
    pub const NOT_SETTINGS: &'static [(&'static str, &'static str)] = &[
        ("set_dt", "a per-frame input: the frame's step in seconds"),
        (
            "set_pipeline_cache",
            "a test hook: the pipeline cache the test devices build through",
        ),
        (
            "set_sky",
            "a per-frame input: the game's sky, its bodies moved by the game's clock",
        ),
        (
            "set_sky_multiscatter",
            "a test hook: the sky-view LUT without its multiple-scattering term",
        ),
        (
            "set_frame_index",
            "a per-frame input: the frame index the noise seeds with",
        ),
        (
            "set_focus",
            "a per-frame input: the mirror's focus, the camera's chunk",
        ),
        (
            "set_lights",
            "a per-frame input: the consumer's list of lights",
        ),
        (
            "set_entities",
            "a per-frame input: the entities drawn, on the frame and the entity mirror",
        ),
        (
            "set_marks",
            "a per-frame input: the entities marked, on the entity mirror",
        ),
        (
            "set_entity_displays",
            "a per-frame input: the entities drawn by display poses, on the frame and the entity mirror",
        ),
        (
            "set_emission_scale",
            "a per-frame input: an emitter's current scale (the world's look, not the renderer's)",
        ),
        (
            "set_emission_scales",
            "a per-frame input: as set_emission_scale, with the probes' scale",
        ),
        (
            "set_sizes",
            "the consumer's output size (its window); the scale field sets the internal size through it",
        ),
        (
            "set_composite_capture",
            "a test and tool hook: copies the composite's image",
        ),
        (
            "set_count_store",
            "a reading hook: counts the store's reads for the HUD and readings",
        ),
        (
            "set_material_styles",
            "a per-material input: the consumer's render-side style of each material (SHADOW_AS_BRICK, SHADOW_ABSORB)",
        ),
        (
            "set_light_hooks",
            "a reading hook: takes the key light or the history's material and normal tests out",
        ),
        (
            "set_bypass",
            "a test hook on Fsr2: skips its passes at equal sizes",
        ),
        (
            "set_debug",
            "a test hook on Fsr2: writes its history weight",
        ),
        (
            "set_debug_stages",
            "a reading hook on Fsr2: writes its accumulate stage code for the weight",
        ),
        (
            "set_sync_clock",
            "a test hook on GpuWorld: the clock its time budget reads",
        ),
        (
            "set_reach_mode",
            "BakeQueue's own method, which Frame::set_bake_reach calls",
        ),
        (
            "set_origin",
            "ProbeVolume's own method, a test hook: pins the volume's origin",
        ),
        (
            "set_updates",
            "ProbeVolume's own method, a test hook: the probe passes' switches",
        ),
        ("set_text", "HudOverlay's own method: the overlay's text"),
        ("set_lines", "HudOverlay's own method: the overlay's lines"),
        (
            "set_scale",
            "HudOverlay's own method: the overlay's glyph scale",
        ),
        (
            "set_lines_over",
            "HudOverlay's own method: the overlay's lines over a dimmed box",
        ),
        (
            "set_tile",
            "HudOverlay's own method: an image tile of the overlay",
        ),
        (
            "set_lines_painted",
            "HudOverlay's own method: the overlay's lines, spans and tiles",
        ),
        (
            "set_rows",
            "HudOverlay's own method: the overlay's coloured rows",
        ),
        (
            "set_samples",
            "HudOverlay's own method: the overlay's graph samples",
        ),
    ];

    /// `self` with field `name` given a value other than its own, the others as they are: the
    /// test value of each row of [`RenderSettings::FIELDS`], for the tests that prove each
    /// field's class. `None` for no field.
    #[doc(hidden)]
    pub fn with_test_value(&self, name: &str) -> Option<Self> {
        use crate::frame::{NormalMode, ViewMode};
        let mut s = self.clone();
        if let Some(field) = name.strip_prefix("lighting.") {
            return lighting_test_value(&mut s.lighting, field).then_some(s);
        }
        match name {
            "look.normal_mode" => other(
                &mut s.look.normal_mode,
                NormalMode::Face,
                NormalMode::Trilinear8,
            ),
            "look.view" => other(&mut s.look.view, ViewMode::Shaded, ViewMode::Normals),
            "look.translucent_layers" => {
                s.look.translucent_layers = if s.look.translucent_layers == 2 { 4 } else { 2 }
            }
            "look.cutaway" => flip(&mut s.look.cutaway),
            "look.tuning.face_mix" => s.look.tuning.face_mix = 0.5 - s.look.tuning.face_mix,
            "look.far_normal" => s.look.far_normal = s.look.far_normal.map(|v| v + 1.0),
            "lod.r_full" => s.lod.r_full += 1,
            "lod.forced_level" => {
                s.lod.forced_level = if s.lod.forced_level.is_some() {
                    None
                } else {
                    Some(1)
                }
            }
            "lighting_resolution" => other(
                &mut s.lighting_resolution,
                LightingResolution::Full,
                LightingResolution::Half,
            ),
            "scale" => other(&mut s.scale, RenderScale::OneHalf, RenderScale::Two),
            "upscaler" => other(&mut s.upscaler, UpscalerChoice::Own, UpscalerChoice::Off),
            "jitter" => flip(&mut s.jitter),
            "sharpness" => s.sharpness = if s.sharpness == 0.25 { 0.5 } else { 0.25 },
            "fsr2.sharpness" => {
                s.fsr2.sharpness = if s.fsr2.sharpness == 0.25 { 0.5 } else { 0.25 }
            }
            "fsr2.reactive_translucent" => s.fsr2.reactive_translucent /= 2.0,
            "fsr2.reactive_light" => s.fsr2.reactive_light /= 2.0,
            "fsr2.metres_per_voxel" => s.fsr2.metres_per_voxel *= 2.0,
            "fsr2.jitter_phases" => {
                s.fsr2.jitter_phases = if s.fsr2.jitter_phases.is_some() {
                    None
                } else {
                    Some(16)
                }
            }
            "fsr2.dither" => flip(&mut s.fsr2.dither),
            "own.alpha" => s.own.alpha /= 2.0,
            "own.alpha_moving" => s.own.alpha_moving /= 2.0,
            "own.change_rel" => s.own.change_rel *= 2.0,
            "own.change_abs" => s.own.change_abs *= 2.0,
            "own.change_frames" => s.own.change_frames = s.own.change_frames / 2 + 1,
            "light_cap" => s.light_cap = if s.light_cap == 4 { 16 } else { 4 },
            "metering" => {
                s.metering = match s.metering {
                    Metering::Average => Metering::default(),
                    Metering::CentreWeighted { .. } => Metering::Average,
                }
            }
            "field_split" => flip(&mut s.field_split),
            "probe_spacing" => s.probe_spacing = if s.probe_spacing == 8 { 16 } else { 8 },
            "probe_axis" => {
                s.probe_axis = if s.probe_axis.is_some() {
                    None
                } else {
                    Some(4)
                }
            }
            "missing_light" => other(
                &mut s.missing_light,
                MissingLight::Chain,
                MissingLight::Black,
            ),
            "bake.budget" => {
                s.bake.budget = match s.bake.budget {
                    Budget::PerFrame { ms } => Budget::PerFrame { ms: ms + 1.0 },
                    Budget::PerSecond { .. } => Budget::default(),
                }
            }
            "bake.coarse_samples" => s.bake.coarse_samples = s.bake.coarse_samples / 2 + 1,
            "bake.near" => s.bake.near += 1,
            "bake.full_first" => {
                s.bake.full_first = if s.bake.full_first.is_some() {
                    None
                } else {
                    Some(2)
                }
            }
            "bake.reach" => other(&mut s.bake.reach, ReachMode::Full, ReachMode::Partial),
            "bake.edit_ms" => s.bake.edit_ms += 1.0,
            "bake.dig_reach" => flip(&mut s.bake.dig_reach),
            "bake.tiers" => s.bake.tiers[1] /= 2,
            "bake.drain" => {
                s.bake.drain = if s.bake.drain.is_some() {
                    None
                } else {
                    Some(crate::bake::DEFAULT_DRAIN)
                }
            }
            "build.mirror_budget_bytes" => s.build.mirror_budget_bytes /= 2,
            "build.record_chunks" => {
                s.build.record_chunks = if s.build.record_chunks.is_some() {
                    None
                } else {
                    Some(64)
                }
            }
            "build.light_samples" => s.build.light_samples /= 2,
            "build.light_chunk_cap" => s.build.light_chunk_cap /= 2,
            "build.light_radius" => s.build.light_radius += 1,
            "build.light_sync_samples" => s.build.light_sync_samples /= 2,
            "build.sync_budget_bytes" => s.build.sync_budget_bytes /= 2,
            "build.sync_budget_time" => {
                s.build.sync_budget_time = if s.build.sync_budget_time.is_some() {
                    None
                } else {
                    Some(std::time::Duration::from_millis(2))
                }
            }
            "build.field_surface" => flip(&mut s.build.field_surface),
            "build.entity_pool_bytes" => s.build.entity_pool_bytes /= 2,
            "build.residency" => s.build.residency.r_full -= 1,
            "build.workers" => {
                s.build.workers = if s.build.workers.is_some() {
                    None
                } else {
                    Some(2)
                }
            }
            "output_hint" => {
                s.output_hint = if s.output_hint.is_some() {
                    None
                } else {
                    Some([1280, 720])
                }
            }
            "mark_colours" => s.mark_colours.swap(0, 1),
            _ => return None,
        }
        Some(s)
    }

    /// The class of the field `name` names, `None` for no field.
    pub fn class_of(name: &str) -> Option<ApplyClass> {
        Self::FIELDS
            .iter()
            .find(|(n, _)| *n == name)
            .map(|&(_, c)| c)
    }

    /// One row per [`RenderSettings::FIELDS`] entry, in its order: the field's name, its class
    /// and its value as `Debug` prints it on one line.
    pub fn table(&self) -> Vec<(&'static str, ApplyClass, String)> {
        let values = self.values();
        Self::FIELDS
            .iter()
            .map(|&(name, class)| {
                let v = values
                    .iter()
                    .find(|(n, _)| n == name)
                    .map_or_else(String::new, |(_, v)| v.clone());
                (name, class, v)
            })
            .collect()
    }

    /// The fields whose values differ between `self` and `other`, in
    /// [`RenderSettings::FIELDS`]' order.
    pub fn changed_fields(&self, other: &Self) -> Vec<&'static str> {
        let (a, b) = (self.values(), other.values());
        Self::FIELDS
            .iter()
            .map(|&(name, _)| name)
            .filter(|name| {
                let find = |vs: &[(String, String)]| {
                    vs.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone())
                };
                find(&a) != find(&b)
            })
            .collect()
    }

    /// Each field's value: the leaves of the pretty `Debug` text, stopped at the
    /// [`RenderSettings::FIELDS`] names.
    fn values(&self) -> Vec<(String, String)> {
        let names: Vec<&str> = Self::FIELDS.iter().map(|&(n, _)| n).collect();
        debug_leaves(&format!("{self:#?}"), &names)
    }
}

/// Gives `v` the value of `a` and `b` it does not hold.
fn other<T: PartialEq + Copy>(v: &mut T, a: T, b: T) {
    *v = if *v == a { b } else { a };
}

fn flip(b: &mut bool) {
    *b = !*b;
}

/// [`RenderSettings::with_test_value`] for the lighting's field `name`; false for no field.
fn lighting_test_value(l: &mut Lighting, name: &str) -> bool {
    use crate::exposure::Exposure;
    use crate::frame::{
        EntityLight, HistoryKill, KeyFar, KeyFarMix, LightWeights, Penumbra, ShadowWalk, SkyNormal,
        StoreRead, ToneMap, YoungFilter,
    };
    match name {
        "exposure" => {
            l.exposure = match l.exposure {
                Exposure::Manual(e) => Exposure::Manual(e * 2.0),
                Exposure::Auto(_) => Exposure::Manual(1.0),
            }
        }
        "tone_map" => other(&mut l.tone_map, ToneMap::Aces, ToneMap::Identity),
        "lamp" => flip(&mut l.lamp),
        "sky_rays" => l.sky_rays = (l.sky_rays + 1) % (super::MAX_SKY_RAYS + 1),
        "sky_history" => l.sky_history = if l.sky_history == 64 { 128 } else { 64 },
        "key_history" => l.key_history = if l.key_history == 256 { 64 } else { 256 },
        "candidates" => l.candidates = if l.candidates == 4 { 8 } else { 4 },
        "shadow_walk" => other(&mut l.shadow_walk, ShadowWalk::Full, ShadowWalk::Coarse),
        "light_history" => flip(&mut l.light_history),
        "landing_fade" => flip(&mut l.landing_fade),
        "source_fade" => flip(&mut l.source_fade),
        "dig_hold" => flip(&mut l.dig_hold),
        "light_history_far" => flip(&mut l.light_history_far),
        "far_light_wide" => flip(&mut l.far_light_wide),
        "penumbra" => other(&mut l.penumbra, Penumbra::Disc, Penumbra::Approach),
        "key_far" => other(&mut l.key_far, KeyFar::Baked, KeyFar::Ray),
        "key_far_mix" => other(&mut l.key_far_mix, KeyFarMix::Blend, KeyFarMix::Step),
        "sky_normal" => other(&mut l.sky_normal, SkyNormal::Face, SkyNormal::Smoothed),
        "sky_bounce" => l.sky_bounce = if l.sky_bounce == 0.0 { 0.3 } else { 0.0 },
        "column_wait" => flip(&mut l.column_wait),
        "key_cap" => l.key_cap = if l.key_cap == 0.0 { 1536.0 } else { 0.0 },
        "sky_down_skip" => flip(&mut l.sky_down_skip),
        "key_step" => l.key_step = if l.key_step == 3.0 { 6.0 } else { 3.0 },
        "filter" => flip(&mut l.filter),
        "atrous_iterations" => l.atrous_iterations = if l.atrous_iterations == 2 { 4 } else { 2 },
        "history" => other(&mut l.history, HistoryKill::Edits, HistoryKill::Gradient),
        "lamp_bounce" => flip(&mut l.lamp_bounce),
        "probe_rays" => l.probe_rays = (l.probe_rays / 2).max(1),
        "probe_slices" => l.probe_slices *= 2,
        "probe_freeze" => l.probe_freeze += 1,
        "lamp_reach" => l.lamp_reach *= 2.0,
        "probe_move_blend" => l.probe_move_blend /= 2.0,
        "bake_pool_bytes" => l.bake_pool_bytes *= 2,
        "store_read" => other(&mut l.store_read, StoreRead::Face, StoreRead::Normal),
        "light_floor" => l.light_floor *= 2.0,
        "light_weights" => other(
            &mut l.light_weights,
            LightWeights::Normal,
            LightWeights::Face,
        ),
        "young_filter" => other(&mut l.young_filter, YoungFilter::Luma, YoungFilter::Edges),
        "entity_light" => other(&mut l.entity_light, EntityLight::Own, EntityLight::World),
        _ => return false,
    }
    true
}

/// The leaves of a pretty (`{:#?}`) `Debug` text with their values on one line: every `name:
/// value` line, its name dotted under the structs that hold it. A value that opens a struct
/// (`Name {`) is descended into unless its dotted name is in `stops`; one that opens a tuple, an
/// `Option` or an array (`(`, `[`) is a leaf whose value runs to the matching close.
pub(crate) fn debug_leaves(text: &str, stops: &[&str]) -> Vec<(String, String)> {
    let mut path: Vec<String> = Vec::new();
    let mut leaves: Vec<(String, String)> = Vec::new();
    // Open brackets of the leaf whose value spans lines, and the struct depth of the open
    // lines descended into.
    let mut open = 0i32;
    for line in text.lines() {
        let t = line.trim();
        let delta = t.chars().fold(0i32, |d, c| match c {
            '{' | '(' | '[' => d + 1,
            '}' | ')' | ']' => d - 1,
            _ => d,
        });
        if open > 0 {
            let v = &mut leaves.last_mut().expect("an open leaf").1;
            v.push(' ');
            v.push_str(t);
            open += delta;
            continue;
        }
        let Some((name, value)) = t.split_once(": ") else {
            if t.starts_with('}') {
                path.pop();
            }
            continue;
        };
        let full = if path.is_empty() {
            name.to_string()
        } else {
            format!("{}.{name}", path.join("."))
        };
        if value.ends_with('{') && !stops.contains(&full.as_str()) {
            path.push(name.to_string());
            continue;
        }
        leaves.push((full, value.to_string()));
        open = delta.max(0);
    }
    for (_, v) in &mut leaves {
        let tidy = v
            .replace("( ", "(")
            .replace("[ ", "[")
            .replace(", )", ")")
            .replace(", ]", "]")
            .replace(", }", " }");
        *v = tidy.trim_end_matches(',').to_string();
    }
    leaves
}

/// What [`crate::Frame::apply_settings`] did with each changed field.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SettingsChange {
    /// `Live` fields applied.
    pub live: Vec<&'static str>,
    /// `Settle` fields applied: the history restarted.
    pub settled: Vec<&'static str>,
    /// `Rebuild` fields left: the caller recreates what they need, then calls
    /// [`crate::Frame::built_with`].
    pub rebuild: Vec<&'static str>,
}

impl SettingsChange {
    /// Whether no field changed.
    pub fn is_empty(&self) -> bool {
        self.live.is_empty() && self.settled.is_empty() && self.rebuild.is_empty()
    }
}

#[cfg(test)]
mod tests;
