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
    /// The output size a headless consumer renders at with none of its own; the consumer's
    /// window wins. `Rebuild`: the frame never applies it.
    pub output_hint: Option<[u32; 2]>,
}

impl Default for RenderSettings {
    /// What a new [`crate::Frame`] reports from [`crate::Frame::settings`].
    fn default() -> Self {
        Self {
            lighting: Lighting::default(),
            look: LookParams::default(),
            lod: LodParams::default(),
            lighting_resolution: LightingResolution::default(),
            scale: RenderScale::One,
            upscaler: UpscalerChoice::Off,
            jitter: false,
            sharpness: 0.0,
            fsr2: Fsr2Settings::default(),
            own: Upscaler::default(),
            light_cap: MAX_LIGHTS as u32,
            metering: Metering::default(),
            field_split: true,
            probe_spacing: crate::probes::DEFAULT_PROBE_SPACING,
            probe_axis: None,
            missing_light: MissingLight::default(),
            bake: BakeKnobs::default(),
            build: BuildSettings::default(),
            output_hint: None,
        }
    }
}

use ApplyClass::{Live, Rebuild, Settle};

impl RenderSettings {
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
    ];

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
mod tests {
    use super::*;

    /// Every leaf of the pretty `Debug` text with no stops.
    fn all_leaves(s: &RenderSettings) -> Vec<String> {
        debug_leaves(&format!("{s:#?}"), &[])
            .into_iter()
            .map(|(n, _)| n)
            .collect()
    }

    /// The field names `FIELDS` lists are the leaves of `RenderSettings`'s `Debug` output,
    /// each once: every leaf is a field or lies under exactly one (a struct-like enum
    /// variant's fields, such as the bake budget's `ms`, or the residency's radii), and every
    /// field holds a leaf. Read on the default settings and on settings whose enums take their
    /// other variants.
    #[test]
    fn the_fields_are_the_leaves_of_the_debug_output_each_once() {
        let names: Vec<&str> = RenderSettings::FIELDS.iter().map(|&(n, _)| n).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "a field is listed twice");
        let mut other = RenderSettings::default();
        other.bake.budget = Budget::PerSecond {
            ms_per_s: 2.0,
            cap: 1.0,
        };
        other.metering = Metering::Average;
        other.lighting.exposure = crate::exposure::Exposure::Auto(Default::default());
        other.output_hint = Some([1280, 720]);
        other.build.sync_budget_time = Some(std::time::Duration::from_millis(3));
        for s in [RenderSettings::default(), other] {
            let leaves = all_leaves(&s);
            for leaf in &leaves {
                let under: Vec<&&str> = names
                    .iter()
                    .filter(|n| leaf == *n || leaf.starts_with(&format!("{n}.")))
                    .collect();
                assert_eq!(under.len(), 1, "leaf {leaf} lies under {under:?}");
            }
            for n in &names {
                assert!(
                    leaves
                        .iter()
                        .any(|l| l == n || l.starts_with(&format!("{n}."))),
                    "field {n} holds no leaf of {leaves:?}"
                );
            }
            assert_eq!(s.table().len(), names.len());
            assert!(s.table().iter().all(|(_, _, v)| !v.is_empty()));
        }
    }

    /// A changed leaf is named by its field, an enum's variant change too; equal settings
    /// change nothing.
    #[test]
    fn a_changed_value_names_its_field() {
        let a = RenderSettings::default();
        assert!(a.changed_fields(&a.clone()).is_empty());
        let mut b = a.clone();
        b.look.translucent_layers = 2;
        b.bake.budget = Budget::PerSecond {
            ms_per_s: 2.0,
            cap: 1.0,
        };
        b.build.residency.r_full = 12;
        assert_eq!(
            a.changed_fields(&b),
            ["look.translucent_layers", "bake.budget", "build.residency"]
        );
        let row = |s: &RenderSettings, n: &str| {
            s.table()
                .into_iter()
                .find(|(f, _, _)| *f == n)
                .expect("a row")
                .2
        };
        assert_eq!(row(&b, "look.translucent_layers"), "2");
        assert_eq!(
            row(&b, "bake.budget"),
            "PerSecond { ms_per_s: 2.0, cap: 1.0 }"
        );
        assert_eq!(row(&a, "output_hint"), "None");
    }
}
