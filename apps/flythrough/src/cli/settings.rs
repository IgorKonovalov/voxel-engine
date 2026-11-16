//! The render settings the fly-through runs with (ADR-0055): `--settings` prints them, one
//! line per field.

use super::Args;
use super::flags::Parsing;
use super::table::{Flag, Section};
use vxe_render::{BakeKnobs, BuildSettings, RenderSettings, UpscalerChoice, UpscalerKind};

/// The sections `--help` lists after the audio's: the loading screen's, then the settings'.
pub(super) fn late_sections() -> Vec<Section<Parsing>> {
    let mut s = Vec::from(super::loading::sections());
    s.extend(sections());
    s
}

/// The settings rows.
fn sections() -> Vec<Section<Parsing>> {
    vec![Section {
        title: "Render settings",
        flags: vec![Flag::switch(
            &["--settings"],
            "print every render setting the flags give, one line per field (name, apply class, \
             value), and exit",
            |p: &mut Parsing| p.args.settings = true,
        )],
    }]
}

impl Args {
    /// The render settings the flags give, the scene's own values aside: a level's mirror
    /// budget follows its content, and its LOD rule and records its extent.
    pub fn render_settings(&self) -> RenderSettings {
        let (kind, scale, jitter) = self.upscaler_settings(self.start_upscaler());
        let fsr2 = match self.upscaler_settings(crate::hud::UpscalerMode::Fsr2).0 {
            UpscalerKind::Fsr2(s) => s,
            _ => unreachable!("the FSR 2 mode runs FSR 2"),
        };
        let b = &self.bake;
        RenderSettings {
            lighting: self.start_lighting(),
            look: self.start_look(),
            lod: vxe_render::LodParams {
                r_full: self.residency.r_full,
                forced_level: None,
            },
            lighting_resolution: self.light.resolution,
            scale,
            upscaler: UpscalerChoice::of(&kind),
            jitter,
            sharpness: self.look.sharpen,
            fsr2,
            own: vxe_render::Upscaler::default(),
            light_cap: vxe_render::MAX_LIGHTS as u32,
            metering: self.light.metering,
            field_split: true,
            probe_spacing: self.light.probe_spacing,
            probe_axis: None,
            missing_light: b.missing_light,
            bake: BakeKnobs {
                budget: b.budget,
                coarse_samples: b.coarse,
                near: b.near,
                full_first: self.full_first(),
                reach: b.reach,
                edit_ms: b.edit_ms,
                dig_reach: b.dig_reach,
            },
            build: BuildSettings {
                mirror_budget_bytes: self.mirror_budget.unwrap_or(crate::STREAM_MIRROR_BUDGET),
                record_chunks: Some(crate::stream_record_chunks(self.residency)),
                light_radius: self.light.radius(),
                sync_budget_bytes: self.sync_budget,
                sync_budget_time: self.sync_time,
                field_surface: self.field_surface(),
                residency: self.residency,
                workers: self.workers,
                ..BuildSettings::default()
            },
            output_hint: self.render,
        }
    }
}

/// `--settings`' text: one line per field of `s`, its name, its apply class and its value.
pub fn settings_lines(s: &RenderSettings) -> String {
    let width = RenderSettings::FIELDS
        .iter()
        .map(|(n, _)| n.len())
        .max()
        .unwrap_or(0);
    s.table()
        .into_iter()
        .map(|(name, class, value)| format!("{name:width$}  {:7}  {value}\n", class.name()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::parse_args_from;

    fn argv(a: &[&str]) -> Vec<String> {
        a.iter().copied().map(String::from).collect()
    }

    /// `--settings` prints one line per field: its name, its class and its value.
    #[test]
    fn the_settings_flag_prints_a_line_per_field() {
        let a = parse_args_from(argv(&["--settings"])).expect("--settings parses");
        assert!(a.settings);
        let text = settings_lines(&a.render_settings());
        assert_eq!(text.lines().count(), RenderSettings::FIELDS.len());
        for (line, (name, class)) in text.lines().zip(RenderSettings::FIELDS) {
            let words: Vec<&str> = line.split_whitespace().collect();
            assert_eq!((words[0], words[1]), (*name, class.name()), "{line}");
            assert!(words.len() > 2, "{line} has no value");
        }
    }
}
