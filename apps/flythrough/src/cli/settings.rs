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
                &["--bake-tiers"],
                "NEAR,MID,FAR",
                "the live queue's samples a brick direction by a chunk's band on screen (the \
                 far-normal band at the output's focal length), each 16 to 65536 [the profile's: \
                 4096,1024,256 at floor]",
                |p: &mut Parsing, v| {
                    let n: Vec<u32> = v
                        .string()
                        .split(',')
                        .map(|t| t.parse::<u32>().map_err(|e| e.to_string()))
                        .collect::<Result<_, _>>()?;
                    match n[..] {
                        [a, b, c] if [a, b, c].iter().all(|x| (16..=65536).contains(x)) => {
                            p.args.bake.tiers = [a, b, c];
                            Ok(())
                        }
                        _ => Err("--bake-tiers takes NEAR,MID,FAR, each 16 to 65536".into()),
                    }
                },
            ),
            Flag::value(
                &["--bake-drain"],
                "off|DRAIN,LINE",
                "the live queue's slice a frame in ms while a chunk in view waits for its tier's \
                 record (at least the budget's) and once none does, each 0.1 to 100; off is the \
                 budget alone [the profile's: 4,1 at floor, 6,1 at portable]",
                |p: &mut Parsing, v| {
                    p.args.bake.drain = vxe_render::Drain::parse(v.text)?;
                    Ok(())
                },
            ),
            Flag::value(
                &["--light-hooks"],
                "NAME[,NAME]",
                "the light passes' reading hooks (a reading lever): keyoff (no key light ray), \
                 skyalways (the sky ray cast whatever the record) [none]",
                |p: &mut Parsing, v| {
                    for name in v.string().split(',') {
                        match name {
                            "keyoff" => p.args.light_hooks.key_off = true,
                            "skyalways" => p.args.light_hooks.sky_ray_always = true,
                            _ => {
                                return Err(format!(
                                    "--light-hooks takes keyoff or skyalways, not {name}"
                                ));
                            }
                        }
                    }
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

/// The window's first size, and a headless target's, with no `--render` and no profile hint.
const DEFAULT_SURFACE: [u32; 2] = [1280, 720];

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
        self.bake.tiers = s.bake.tiers;
        self.bake.drain = s.bake.drain;
        self.light_cap = s.light_cap;
        self.output_hint = s.output_hint;
    }

    /// `--render WxH`'s size; none without the flag or under `--render window`.
    pub(crate) fn render_fixed(&self) -> Option<[u32; 2]> {
        match self.render {
            Some(super::RenderSize::Fixed(s)) => Some(s),
            _ => None,
        }
    }

    /// The output size a window does not change, the window scaling the frame to it:
    /// `--render WxH`'s; none under `--render window`; without the flag the portable
    /// profile's hint (it renders at 720p in a window too, so the slow adapter draws the
    /// pixels its frame line names), none for the other profiles (they draw at the window's).
    pub(crate) fn fixed_output(&self) -> Option<[u32; 2]> {
        match self.render {
            Some(super::RenderSize::Fixed(s)) => Some(s),
            Some(super::RenderSize::Window) => None,
            None => self
                .output_hint
                .filter(|_| self.profile == Profile::Portable),
        }
    }

    /// The frame's output size on a surface (a window, or a headless target) of `surface`.
    pub(crate) fn output_size(&self, surface: [u32; 2]) -> [u32; 2] {
        self.fixed_output().unwrap_or(surface)
    }

    /// The size the window opens at, or the headless target's: `--render WxH`'s, else the
    /// profile's hint, else 1280 x 720.
    pub(crate) fn surface_size(&self) -> [u32; 2] {
        self.render_fixed()
            .or(self.output_hint)
            .unwrap_or(DEFAULT_SURFACE)
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
                tiers: b.tiers,
                drain: b.drain,
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
            output_hint: self.render_fixed().or(self.output_hint),
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

    /// In a window the portable profile renders at its hint, 1280 x 720, and the window
    /// scales it; the other profiles, and no profile, at the window's size; `--render WxH`
    /// fixes the output under any profile and `--render window` gives the portable profile
    /// the window's size. The window opens at the fixed size or the hint.
    #[test]
    fn the_output_follows_the_window_but_for_portable_and_render_overrides_both_ways() {
        let window = [2530, 1378];
        let out = |a: &[&str]| {
            let a = parse_args_from(argv(a)).expect("parses");
            (a.output_size(window), a.surface_size())
        };
        assert_eq!(out(&[]), (window, [1280, 720]), "no profile");
        for p in Profile::ALL {
            let hint = RenderSettings::profile(p).output_hint;
            let (output, surface) = out(&["--profile", p.name()]);
            let want = if p == Profile::Portable {
                [1280, 720]
            } else {
                window
            };
            assert_eq!(output, want, "{}", p.name());
            assert_eq!(surface, hint.unwrap_or([1280, 720]), "{}", p.name());
            assert_eq!(
                out(&["--profile", p.name(), "--render", "800x600"]),
                ([800, 600], [800, 600]),
                "{} with --render 800x600",
                p.name()
            );
            assert_eq!(
                out(&["--render", "window", "--profile", p.name()]).0,
                window,
                "{} with --render window",
                p.name()
            );
        }
        assert!(parse_args_from(argv(&["--render", "wide"])).is_err());
    }
}
