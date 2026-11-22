//! The render profile in a running fly-through (ADR-0055): the settings it runs with now, and
//! `F4` or the menu's `profile` entry cycling the profiles, applying each one's `Live` and `Settle` fields.

use super::Run;
use vxe_render::{Profile, RenderSettings, UpscalerChoice};

impl Run {
    /// The render settings the run holds now: the flags' with the keys' changes (the
    /// lighting, the look, the lighting resolution, the upscaler and the bake budget).
    pub(crate) fn live_settings(&self) -> RenderSettings {
        let mut s = self.args.render_settings();
        let (kind, scale, jitter) = self.args.upscaler_settings(self.modes.upscaler.mode);
        s.lighting = self.modes.lighting;
        s.look = self.modes.look;
        s.lighting_resolution = self.modes.resolution;
        s.upscaler = UpscalerChoice::of(&kind);
        (s.scale, s.jitter) = (scale, jitter);
        s.bake.budget = self.modes.queue.budget;
        s
    }

    /// Cycles to the next profile: its translucent layers, lamp bounce, light cap, internal
    /// scale and bake budget apply from the next frame; the fields it still differs in (its
    /// residency, its mirror budget and the LOD radius that follows the residency) wait for a
    /// restart with `--profile`, and the profile row names them.
    pub(crate) fn cycle_profile(&mut self) {
        let i = Profile::ALL
            .iter()
            .position(|&p| p == self.args.profile)
            .unwrap_or(0);
        let next = Profile::ALL[(i + 1) % Profile::ALL.len()];
        let to = RenderSettings::profile(next);
        self.args.profile = next;
        self.args.look.translucent_layers = to.look.translucent_layers;
        self.modes.look.translucent_layers = to.look.translucent_layers;
        self.args.light.lamp_bounce = to.lighting.lamp_bounce;
        self.modes.lighting.lamp_bounce = to.lighting.lamp_bounce;
        self.args.light_cap = to.light_cap;
        self.args.output_hint = to.output_hint;
        if self.args.look.internal_scale != to.scale {
            self.args.look.internal_scale = to.scale;
            self.modes.scale = to.scale.name();
            if self.modes.upscaler.mode != crate::hud::UpscalerMode::Off {
                self.upscaler_changed = true;
            }
        }
        self.args.bake.budget = to.bake.budget;
        self.modes.queue.budget = to.bake.budget;
        if let Some(g) = self.gfx.as_mut() {
            g.frame.set_bake_budget(to.bake.budget);
        }
        let pending = to.changed_fields(&self.live_settings());
        self.modes.profile = self.args.profile_row(&self.live_settings(), &pending);
        self.say(self.modes.profile.clone());
    }

    /// The debug page's `render scale`: the upscaler's internal scale steps 1.5, 1, 2 and back
    /// (FSR 2 at 1 is its antialiasing at native), applied from the next frame as a profile's
    /// scale is; the upscaler banner names it.
    pub(crate) fn cycle_scale(&mut self) {
        use vxe_render::RenderScale;
        let next = match self.args.look.internal_scale {
            RenderScale::OneHalf => RenderScale::One,
            RenderScale::One => RenderScale::Two,
            RenderScale::Two => RenderScale::OneHalf,
        };
        self.args.look.internal_scale = next;
        self.modes.scale = next.name();
        if self.modes.upscaler.mode != crate::hud::UpscalerMode::Off {
            self.upscaler_changed = true;
            let mode = self.modes.upscaler.mode;
            self.modes
                .upscaler
                .change(mode, next.name(), crate::app::ms_since(self.start));
        }
        self.say(format!("render scale {}", next.name()));
    }

    /// The debug page's `jitter phases`: FSR 2's jitter phase count steps through its own
    /// (`8 (display / render)^2`), 8 and 32, applied from the next frame; the upscaler banner
    /// names a count that is set.
    pub(crate) fn cycle_jitter_phases(&mut self) {
        let next = match self.args.look.jitter_phases {
            None => Some(8),
            Some(8) => Some(32),
            Some(_) => None,
        };
        self.args.look.jitter_phases = next;
        self.modes.upscaler.fsr2_knobs = crate::hud::fsr2_knob_words(next, self.args.look.dither);
        if self.modes.upscaler.mode == crate::hud::UpscalerMode::Fsr2 {
            self.upscaler_changed = true;
            let (mode, scale) = (self.modes.upscaler.mode, self.modes.upscaler.scale);
            self.modes
                .upscaler
                .change(mode, scale, crate::app::ms_since(self.start));
        }
        self.say(format!(
            "jitter phases {}",
            next.map_or("fsr2's".to_owned(), |n| n.to_string())
        ));
    }
}
