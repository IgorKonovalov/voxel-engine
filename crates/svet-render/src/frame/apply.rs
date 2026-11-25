//! The frame's settings read back and applied as one value ([`crate::RenderSettings`],
//! ADR-0055).

use super::*;
use crate::settings::{ApplyClass, BuildSettings, RenderSettings, SettingsChange, UpscalerChoice};

impl Frame {
    /// Every setting the frame holds: what its setters last set, the scale its sizes make (1
    /// without FSR 2, which renders at the output size), and the `Rebuild` values of the last
    /// [`Frame::built_with`].
    pub fn settings(&self) -> RenderSettings {
        let [internal, output] = [self.internal_size(), self.output];
        let scale = if self.scale.internal(output) == internal {
            self.scale
        } else {
            RenderScale::ALL
                .into_iter()
                .find(|s| s.internal(output) == internal)
                .unwrap_or(self.scale)
        };
        RenderSettings {
            lighting: self.state.lighting,
            look: self.state.look,
            lod: self.state.lod,
            lighting_resolution: self.resolution,
            scale,
            upscaler: UpscalerChoice::of(&self.up.kind),
            fsr2: self.up.fsr2,
            light_cap: self.light_cap(),
            metering: self.metering,
            field_split: self.state.field_split,
            probe_spacing: self.probe.spacing,
            probe_axis: self.probe.axis_fixed,
            missing_light: self.missing_light(),
            bake: self.bake_knobs(),
            bake_thread: self.bake_thread(),
            build: self.build,
            output_hint: self.output_hint,
            mark_colours: self.mark_colours,
        }
    }

    /// Applies every `Live` and `Settle` field of `s` as the frame takes it
    /// ([`RenderSettings::clamped`]: no FSR 2, scale 1) that differs from [`Frame::settings`]
    /// through the frame's setters, and names each changed field by its class. A change of the
    /// upscaler and the scale together replaces the targets once. The `Rebuild` fields are
    /// left as they are: the caller recreates what they need and then calls
    /// [`Frame::built_with`].
    pub fn apply_settings(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        s: &RenderSettings,
    ) -> SettingsChange {
        let s = &s.clamped();
        let old = self.settings();
        let changed = old.changed_fields(s);
        let mut out = SettingsChange::default();
        for &name in &changed {
            match RenderSettings::class_of(name).expect("a field of FIELDS") {
                ApplyClass::Live => out.live.push(name),
                ApplyClass::Settle => out.settled.push(name),
                ApplyClass::Rebuild => out.rebuild.push(name),
            }
        }
        let any = |prefix: &str| {
            changed.iter().any(|n| {
                n.starts_with(prefix) && RenderSettings::class_of(n) != Some(ApplyClass::Rebuild)
            })
        };
        if any("lighting.") {
            self.set_lighting(Lighting {
                bake_pool_bytes: old.lighting.bake_pool_bytes,
                ..s.lighting
            });
        }
        if any("look.") {
            self.set_look(s.look);
        }
        if any("lod.") {
            self.set_lod(s.lod);
        }
        if any("lighting_resolution") {
            self.set_resolution(device, s.lighting_resolution);
        }
        if any("fsr2.") {
            self.up.fsr2 = s.fsr2;
            if self.up.kind.is_fsr2() && !any("upscaler") {
                self.set_upscaler_kind(device, UpscalerKind::Fsr2(s.fsr2));
            }
        }
        if any("upscaler") || any("scale") {
            self.scale = s.scale;
            self.up.requested = s.scale.internal(self.output);
            if any("upscaler") {
                // A change of upscaler replaces the targets at the requested size.
                self.set_upscaler_kind(device, s.upscaler.kind(s.fsr2));
            } else {
                self.set_sizes(device, self.up.requested, self.output);
            }
        }
        if any("light_cap") {
            self.set_light_cap(s.light_cap);
        }
        if any("metering") {
            self.set_metering(s.metering);
        }
        if any("field_split") {
            self.set_field_split(s.field_split);
        }
        if any("probe_spacing") {
            self.set_probe_spacing(device, s.probe_spacing);
        }
        if any("probe_axis") {
            self.set_probe_axis(s.probe_axis);
        }
        if any("missing_light") {
            self.set_missing_light(queue, s.missing_light);
        }
        if any("mark_colours") {
            self.set_mark_colours(s.mark_colours);
        }
        if any("bake.") {
            let b = s.bake;
            self.set_bake_budget(b.budget);
            self.set_bake_coarse(b.coarse_samples, b.near);
            self.set_bake_full_first(b.full_first);
            self.set_bake_reach(b.reach);
            self.set_bake_digs(b.edit_ms, b.dig_reach);
            self.set_bake_tiers(b.tiers);
            self.set_bake_drain(b.drain);
        }
        if any("bake_thread") {
            self.set_bake_thread(queue, s.bake_thread);
        }
        out
    }

    /// Records the `Rebuild` fields of `s` as the ones the caller built the mirror, the
    /// streamer and the entity pool with: [`Frame::settings`] reads them from now on. A
    /// changed [`Lighting::bake_pool_bytes`] is applied here, which rebuilds an allocated store
    /// empty.
    pub fn built_with(&mut self, s: &RenderSettings) {
        self.build = s.build;
        self.output_hint = s.output_hint;
        if s.lighting.bake_pool_bytes != self.state.lighting.bake_pool_bytes {
            self.set_lighting(Lighting {
                bake_pool_bytes: s.lighting.bake_pool_bytes,
                ..self.state.lighting
            });
        }
    }

    /// The `Rebuild` group the frame last recorded ([`Frame::built_with`]).
    pub fn build_settings(&self) -> BuildSettings {
        self.build
    }
}
