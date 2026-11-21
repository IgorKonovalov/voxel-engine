//! The render settings the fly-through runs with (ADR-0055): `--settings` prints them, one
//! line per field.

use super::Args;
use super::flags::Parsing;
use super::table::{Flag, Section};
use vxe_render::{BakeKnobs, BuildSettings, Profile, RenderSettings, UpscalerChoice, UpscalerKind};

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
        flags: vec![
            Flag::switch(
                &["--settings"],
                "print every render setting the flags give, one line per field (name, apply \
                 class, value), and exit",
                |p: &mut Parsing| p.args.settings = true,
            ),
            Flag::value(
                &["--profile"],
                "floor|portable|performance|high",
                "the render profile (vxe_render::Profile) every other flag edits; F4 (or \
                 the menu's profile entry) cycles it, applying its live and settle fields [floor]",
                |p: &mut Parsing, v| {
                    p.args.profile = v.named(Profile::from_name, PROFILE_NAMES)?;
                    Ok(())
                },
            ),
            Flag::value(
                &["--light-cap"],
                "N",
                "slots of the list of lights shaded, the headlamp's included, 1 to 16 [the \
                 profile's]",
                |p: &mut Parsing, v| {
                    p.args.light_cap = v.parse_if(
                        |n: &u32| (1..=vxe_render::MAX_LIGHTS as u32).contains(n),
                        "1 to 16",
                    )?;
                    Ok(())
                },
            ),
            Flag::value(
                &["--light-samples"],
                "N",
                "the mirror's light-sample buffer in samples, 1024 to 1048576 (a reading lever: \
                 a smaller buffer fills and evicts) [1048576]",
                |p: &mut Parsing, v| {
                    p.args.light_samples = Some(v.parse_if(
                        |n: &u32| (1024..=vxe_render::DEFAULT_LIGHT_SAMPLES).contains(n),
                        "1024 to 1048576",
                    )?);
                    Ok(())
                },
            ),
        ],
    }]
}

/// The names `--profile` takes.
const PROFILE_NAMES: &str = "floor, portable, performance or high";

/// The profile `argv` names with `--profile NAME`, the last one given; `None` without one.
/// The parser seeds the other flags' defaults from it before it reads them.
pub(super) fn profile_in(argv: &[String]) -> Result<Option<Profile>, String> {
    let mut found = None;
    for (i, a) in argv.iter().enumerate() {
        if a == "--profile" {
            let name = argv.get(i + 1).map_or("", String::as_str);
            found = Some(
                Profile::from_name(name)
                    .ok_or_else(|| format!("--profile takes {PROFILE_NAMES}, not {name}"))?,
            );
        }
    }
    Ok(found)
}

impl Args {
    /// Sets every flag's default that `p` gives a value of its own: the translucent layers,
    /// the lamp's bounce, the internal scale, the residency, the bake budget, the light cap
    /// and the output hint, and at internal scale 1 the upscaler (the fly-through's own
    /// default there is `off`). The mirror budget of a streamed run follows the profile too
    /// (`Args::stream_mirror_budget`).
    pub(crate) fn seed_profile(&mut self, p: Profile) {
        let s = RenderSettings::profile(p);
        self.profile = p;
        self.look.translucent_layers = s.look.translucent_layers;
        self.light.lamp_bounce = s.lighting.lamp_bounce;
        self.look.internal_scale = s.scale;
        self.look.upscaler =
            (s.scale == vxe_render::RenderScale::One).then_some(match s.upscaler {
                UpscalerChoice::Off => crate::hud::UpscalerMode::Off,
                UpscalerChoice::Own => crate::hud::UpscalerMode::Own,
                UpscalerChoice::Fsr2 => crate::hud::UpscalerMode::Fsr2,
            });
        self.residency = s.build.residency;
        self.bake.budget = s.bake.budget;
        self.light_cap = s.light_cap;
        self.output_hint = s.output_hint;
    }

    /// The streamed mirror's budget unless `--mirror-budget` is given: the profile's.
    pub(crate) fn stream_mirror_budget(&self) -> u64 {
        RenderSettings::profile(self.profile)
            .build
            .mirror_budget_bytes
    }

    /// The fields the flags edited on top of the profile: those whose value differs from the
    /// profile's as the fly-through reads it (`--profile NAME` and no other flag).
    pub(crate) fn profile_edits(&self, now: &RenderSettings) -> Vec<&'static str> {
        let mut base = super::Args::defaults();
        base.seed_profile(self.profile);
        // The hint is the profile's only under an explicit `--profile`; not an edit.
        base.output_hint = self.output_hint;
        base.render_settings().changed_fields(now)
    }

    /// The HUD's profile row: `profile: portable`, or `profile: portable +2` with two fields
    /// edited, and `, restart for: build.residency` while the fields `pending` wait for a
    /// restart (they are not counted as edits).
    pub(crate) fn profile_row(&self, now: &RenderSettings, pending: &[&str]) -> String {
        let mut row = format!("profile: {}", self.profile.name());
        let n = self
            .profile_edits(now)
            .iter()
            .filter(|f| !pending.contains(f))
            .count();
        if n > 0 {
            row.push_str(&format!(" +{n}"));
        }
        if !pending.is_empty() {
            row.push_str(&format!(", restart for: {}", pending.join(", ")));
        }
        row
    }

    /// The render settings the flags give, the scene's own values aside: a level's mirror
    /// budget follows its content, and its LOD rule and records its extent. With no flag they
    /// are the floor's but for the fly-through's own `build.record_chunks` (sized to the
    /// residency) and `output_hint` (none: a window's size, or 1280 x 720 headless).
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
            light_cap: self.light_cap,
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
                mirror_budget_bytes: self
                    .mirror_budget
                    .unwrap_or_else(|| self.stream_mirror_budget()),
                record_chunks: Some(crate::stream_record_chunks(self.residency)),
                light_radius: self.light.radius(),
                sync_budget_bytes: self.sync_budget,
                sync_budget_time: self.sync_time,
                field_surface: self.field_surface(),
                residency: self.residency,
                workers: self.workers,
                ..BuildSettings::default()
            },
            output_hint: self.render.or(self.output_hint),
            mark_colours: vxe_render::MARK_COLOURS,
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

    /// The fly-through's own values over a profile's: the records sized to the residency,
    /// the key light's penumbra ([`crate::DEFAULT_PENUMBRA`]), and no output hint without
    /// `--profile`.
    fn fly_through(p: Profile, hint: bool) -> RenderSettings {
        let mut s = RenderSettings::profile(p);
        s.lighting.penumbra = crate::DEFAULT_PENUMBRA;
        s.build.record_chunks = Some(crate::stream_record_chunks(s.build.residency));
        if !hint {
            s.output_hint = None;
        }
        s
    }

    /// With no flag the fly-through runs the floor (the library's default) but for its own
    /// records and output hint; `--profile floor` takes the floor's hint too.
    #[test]
    fn no_flags_give_the_floor() {
        assert_eq!(
            RenderSettings::default(),
            RenderSettings::profile(Profile::Floor)
        );
        let none = parse_args_from(argv(&[])).expect("no flags parse");
        assert_eq!(none.render_settings(), fly_through(Profile::Floor, false));
        assert_eq!(
            none.profile_row(&none.render_settings(), &[]),
            "profile: floor"
        );
        let floor = parse_args_from(argv(&["--profile", "floor"])).expect("--profile parses");
        assert_eq!(floor.render_settings(), fly_through(Profile::Floor, true));
    }

    /// `--profile portable --translucent-layers 4` is the portable profile with 4 layers, one
    /// field edited; the flags after a profile edit it whatever their order.
    #[test]
    fn a_flag_edits_the_profile_on_top() {
        for argv_ in [
            &["--profile", "portable", "--translucent-layers", "4"][..],
            &["--translucent-layers", "4", "--profile", "portable"][..],
        ] {
            let a = parse_args_from(argv(argv_)).expect("parses");
            let mut want = fly_through(Profile::Portable, true);
            want.look.translucent_layers = 4;
            assert_eq!(a.render_settings(), want);
            assert_eq!(
                a.profile_row(&a.render_settings(), &[]),
                "profile: portable +1"
            );
        }
        let perf = parse_args_from(argv(&["--profile", "performance"])).expect("parses");
        assert_eq!(
            perf.render_settings(),
            fly_through(Profile::Performance, true)
        );
        let high = parse_args_from(argv(&["--profile", "high"])).expect("parses");
        assert_eq!(high.render_settings(), fly_through(Profile::High, true));
        assert_eq!(
            high.profile_row(&high.render_settings(), &[]),
            "profile: high"
        );
        let Err(e) = parse_args_from(argv(&["--profile", "fast"])) else {
            panic!("--profile fast parsed");
        };
        assert!(e.contains("floor, portable, performance or high"), "{e}");
    }

    /// `--settings --profile P` prints each profile's picks (`RenderSettings::PROFILE_PICKS`,
    /// the readings log's Phase 5 table), and the floor's value in every other field but the
    /// fly-through's own records.
    #[test]
    fn the_settings_flag_prints_each_profiles_picks() {
        let floor = settings_lines(&fly_through(Profile::Floor, true));
        for (i, p) in Profile::ALL.into_iter().enumerate() {
            let a = parse_args_from(argv(&["--settings", "--profile", p.name()])).expect("parses");
            let text = settings_lines(&a.render_settings());
            for (line, floor_line) in text.lines().zip(floor.lines()) {
                let name = line.split_whitespace().next().expect("a name");
                let value = line
                    .split_once(name)
                    .and_then(|(_, rest)| rest.trim_start().split_once(' '))
                    .map(|(_, v)| v.trim());
                match RenderSettings::PROFILE_PICKS
                    .iter()
                    .find(|(n, _)| *n == name)
                {
                    Some((_, picks)) => assert_eq!(value, Some(picks[i]), "{} {line}", p.name()),
                    // The fly-through sizes its records to the profile's residency.
                    None if name == "build.record_chunks" => {}
                    None => assert_eq!(line, floor_line, "{} differs from the floor", p.name()),
                }
            }
        }
    }
}
