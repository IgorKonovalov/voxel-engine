//! The world source's row (`--source`) and the outdoor testbed's: the spawn, the time of day,
//! the day's length, the sky, and the in-process source switch's frames.

use super::RunMode;
use super::flags::Parsing;
use super::table::{Flag, Section};
use crate::daylight::{Clock, parse_time};
use crate::outdoor::Spawn;

/// The outdoor testbed's flags.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OutdoorArgs {
    /// `--spawn outside|inside`: where `--source outdoor` stands the camera.
    pub(crate) spawn: Spawn,
    /// `--time` and `--day-length`: the clock the run starts with, its cycle paused.
    pub(crate) clock: Clock,
    /// `--sky on|off`: a sky is set; unset, on for `--source outdoor` alone.
    pub(crate) sky: Option<bool>,
    /// The auto exposure takes [`OUTDOOR_MIN`] and [`OUTDOOR_MAX`]: `--source outdoor` with
    /// neither `--exposure` nor `--adaptation`.
    pub(crate) auto: bool,
    /// `--sky-rays N`: the sky rays a lighting pixel casts; unset, the renderer's default.
    pub(crate) sky_rays: Option<u32>,
}

/// The outdoor start exposure's floor: a sunlit ground reads about 200 to 300 frame radiance
/// units and needs about 6e-4 to 9e-4, far under `AutoExposure::eye()`'s floor of 2^-6.
pub const OUTDOOR_MIN: f32 = 1e-4;
/// The outdoor start exposure's ceiling.
pub const OUTDOOR_MAX: f32 = 4.0;

/// The `--source` values in the flag's order, which the in-process switch cycles through.
pub const SOURCES: [RunMode; 4] = [
    RunMode::Cave,
    RunMode::Level,
    RunMode::Showcase,
    RunMode::Outdoor,
];

/// The source after `mode` in [`SOURCES`], back to the first after the last; a world that is
/// no source (a room, a rig) goes to the first.
pub fn next_source(mode: RunMode) -> RunMode {
    let i = SOURCES.iter().position(|&m| m == mode);
    SOURCES[i.map_or(0, |i| (i + 1) % SOURCES.len())]
}

/// The World section's first row: `--source`.
pub(super) fn source_flags() -> Vec<Flag<Parsing>> {
    vec![Flag::value(
        &["--source"],
        "cave|level|showcase|outdoor",
        "the world: the streamed cave, the level, the showcase or the outdoor hills",
        |p: &mut Parsing, v| {
            let names = SOURCES.map(|m| (m.source_name().unwrap_or(""), m));
            let mode = v.word(&names, "cave, level, showcase or outdoor")?;
            p.given.level = mode == RunMode::Level;
            p.given.showcase = mode == RunMode::Showcase;
            p.given.outdoor = mode == RunMode::Outdoor;
            Ok(())
        },
    )]
}

/// The outdoor testbed's section.
pub(super) fn sections() -> Vec<Section<Parsing>> {
    vec![Section {
        title: "Outdoors and the sky",
        flags: vec![
            Flag::value(
                &["--spawn"],
                "outside|inside",
                "where --source outdoor starts: 30 m before the cave mouth facing it, 2 m up, \
                 or 3 m inside the tunnel facing out [outside]",
                |p: &mut Parsing, v| {
                    let names = Spawn::ALL.map(|s| (s.name(), s));
                    p.args.outdoor.spawn = v.word(&names, "outside or inside")?;
                    Ok(())
                },
            ),
            Flag::value(
                &["--time"],
                "HH:MM",
                "the time of day the sky starts at (latitude 45, an equinox); PageUp steps an \
                 hour, Shift+PageUp back [12:00]",
                |p: &mut Parsing, v| {
                    p.args.outdoor.clock.minutes = parse_time(v.text)?;
                    Ok(())
                },
            ),
            Flag::value(
                &["--day-length"],
                "S",
                "seconds for a whole day while the cycle runs (End runs or pauses it), 1 to \
                 86400 [1200]",
                |p: &mut Parsing, v| {
                    p.args.outdoor.clock.day_length_s =
                        v.parse_if(|s: &f32| (1.0..=86_400.0).contains(s), "1 to 86400 s")?;
                    Ok(())
                },
            ),
            Flag::value(
                &["--sun-radius"],
                "DEG",
                "the sun's angular radius in degrees, 0.05 to 10: a wider disc softens every \
                 sun shadow [0.2665, the real sun's]",
                |p: &mut Parsing, v| {
                    let deg: f32 = v.parse_if(|d: &f32| (0.05..=10.0).contains(d), "0.05 to 10")?;
                    p.args.outdoor.clock.sun_radius = deg.to_radians();
                    Ok(())
                },
            ),
            Flag::value(
                &["--sky-rays"],
                "N",
                "cosine-weighted sky rays a lighting pixel casts while a sky is set, 1 to 4 [1]",
                |p: &mut Parsing, v| {
                    let max = vxe_render::MAX_SKY_RAYS;
                    p.args.outdoor.sky_rays =
                        Some(v.parse_if(|n: &u32| (1..=max).contains(n), "1 to 4")?);
                    Ok(())
                },
            ),
            Flag::value(
                &["--sky"],
                "on|off",
                "a sky over the world (vxe_render::Sky) [on for --source outdoor, else off]",
                |p: &mut Parsing, v| {
                    p.args.outdoor.sky = Some(v.on_off()?);
                    Ok(())
                },
            ),
            Flag::value(
                &["--switch-at"],
                "F[,F..]",
                "switch the world source in process at these frames, as Home does, and print \
                 the spawn's hash at the end (tests)",
                |p: &mut Parsing, v| {
                    p.args.switch_at = super::flags::frames(v)?;
                    Ok(())
                },
            ),
            Flag::switch(
                &["--spawn-hash"],
                "print the voxel hash of the chunks within 2 of the spawn's chunk at the end",
                |p: &mut Parsing| p.args.spawn_hash = true,
            ),
        ],
    }]
}

/// The refusal of a world flag beside `--source outdoor`.
pub(super) const OUTDOOR_REFUSES: &str = "--source outdoor builds its own hills; it does not \
     take --level, --castle, --vault, --room, --thin-rig, --seam-rig, --save or --replay";
