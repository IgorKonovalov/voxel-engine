//! The world source's row (`--source`) and the outdoor testbed's: the spawn, the time of day,
//! the day's length, the sky, and the in-process source switch's frames.

use super::RunMode;
use super::flags::Parsing;
use super::table::{Flag, Section};
use crate::daylight::{Clock, parse_sun, parse_time};
use crate::outdoor::Spawn;
use crate::trees::{LEAF_FILL, TreeDensity};

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
    /// `--sky-history N`: the visibility pair's cap in frames; unset, the renderer's default.
    pub(crate) sky_history: Option<u32>,
    /// `--key-history N`: the key light's mean's cap in frames; unset, the renderer's default.
    pub(crate) key_history: Option<u32>,
    /// `--trees none|sparse|forest`: the trees stood on the hills.
    pub(crate) trees: TreeDensity,
    /// `--leaf-fill N`: a canopy voxel is kept under this of 256; unset, [`LEAF_FILL`].
    pub(crate) leaf_fill: Option<u32>,
    /// `--leaf-shadow brick|voxel|absorb`: how the canopy shadows.
    pub(crate) leaf_shadow: crate::LeafShadow,
    /// `--penumbra disc|approach`: how the key light's shadow ray makes its penumbra.
    pub(crate) penumbra: vxe_render::Penumbra,
    /// `--sky-normal face|smoothed`: the normal the baked sky term reads its visibilities by.
    pub(crate) sky_normal: vxe_render::SkyNormal,
    /// `--sky-bounce B`: the baked sky term's bounce floor, 0 to 1.
    pub(crate) sky_bounce: f32,
    /// `--column-wait on|off`: the live queue's column wait (`Lighting::column_wait`).
    pub(crate) column_wait: bool,
    /// `--dump-black`: once a second while the store's black count is over 0, the lighting
    /// pixels with no record value printed by the chunk they hit (`Run::dump_black`).
    pub(crate) dump_black: bool,
    /// `--sky-anchor N|none`: the auto exposure's ceiling anchored `N` EV over the sunlit
    /// ground's exposure while a sky is set (`AutoExposure::sky_anchor_ev`), `Some(None)` for
    /// none; unset, the preset's (`vxe_render::DEFAULT_SKY_ANCHOR_EV`).
    pub(crate) sky_anchor: Option<Option<f32>>,
    /// `--exposure-max EV`: the auto exposure's constant ceiling in EV; unset, the start's
    /// ([`OUTDOOR_MAX`] outdoors, the preset's elsewhere).
    pub(crate) exposure_max_ev: Option<f32>,
    /// `--exposure-rate EV/s`: the auto exposure's rate cap (`AutoExposure::rate_ev_s`, 0 for
    /// none); unset, the preset's (`vxe_render::DEFAULT_EXPOSURE_RATE_EV_S` for the eye).
    pub(crate) exposure_rate: Option<f32>,
}

impl OutdoorArgs {
    /// `--leaf-fill`, else [`LEAF_FILL`].
    pub fn leaf_fill(&self) -> u32 {
        self.leaf_fill.unwrap_or(LEAF_FILL)
    }

    pub fn trees(&self) -> TreeDensity {
        self.trees
    }

    pub fn spawn(&self) -> Spawn {
        self.spawn
    }

    pub fn leaf_shadow(&self) -> crate::LeafShadow {
        self.leaf_shadow
    }
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
    let mut flags = vec![
        Flag::value(
            &["--spawn"],
            "outside|inside|forest",
            "where --source outdoor starts: 30 m before the cave mouth facing it, 2 m up, \
                 3 m inside the tunnel facing out, or 64 m north of the mouth facing north, \
                 2 m up, 10 degrees down [outside]",
            |p: &mut Parsing, v| {
                let names = Spawn::ALL.map(|s| (s.name(), s));
                p.args.outdoor.spawn = v.word(&names, "outside, inside or forest")?;
                Ok(())
            },
        ),
        Flag::value(
            &["--trees"],
            "none|sparse|forest",
            "trees on the outdoor hills, about 9 % or 94 % of the 4.8 m cells of a 307 m \
                 box; the menu's trees entry cycles them in process [sparse]",
            |p: &mut Parsing, v| {
                let names = TreeDensity::ALL.map(|d| (d.name(), d));
                p.args.outdoor.trees = v.word(&names, "none, sparse or forest")?;
                Ok(())
            },
        ),
        Flag::value(
            &["--leaf-fill"],
            "N",
            "the canopy voxels kept, of 256, 1 to 256 [64]",
            |p: &mut Parsing, v| {
                p.args.outdoor.leaf_fill =
                    Some(v.parse_if(|n: &u32| (1..=256).contains(n), "1 to 256")?);
                Ok(())
            },
        ),
        Flag::value(
            &["--leaf-shadow"],
            "brick|voxel|absorb",
            "how a canopy shadows the sun and the sky: as its 8^3 bricks (no fleck through \
                 it), voxel by voxel, or as a transmittance by the path through its leaves \
                 [voxel]",
            |p: &mut Parsing, v| {
                let names = crate::LeafShadow::ALL.map(|d| (d.name(), d));
                p.args.outdoor.leaf_shadow = v.word(&names, "brick, voxel or absorb")?;
                Ok(())
            },
        ),
        Flag::value(
            &["--sun"],
            "EL,AZ",
            "fix the sun at this elevation and azimuth in degrees (clockwise from north, \
                 180 south), the moon opposite; the clock's keys then leave it",
            |p: &mut Parsing, v| {
                p.args.outdoor.clock.fixed_sun = Some(parse_sun(v.text)?);
                Ok(())
            },
        ),
        Flag::value(
            &["--time"],
            "HH:MM",
            "the time of day the sky starts at (latitude 45, an equinox); the menu's \
                 time entry steps it an hour [12:00]",
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
                 sun shadow (the real sun's is 0.2665) [3]",
            |p: &mut Parsing, v| {
                let deg: f32 = v.parse_if(|d: &f32| (0.05..=10.0).contains(d), "0.05 to 10")?;
                p.args.outdoor.clock.sun_radius = deg.to_radians();
                Ok(())
            },
        ),
        Flag::value(
            &["--moon-radius"],
            "DEG",
            "the moon's angular radius in degrees, 0.05 to 10: its disc in the sky and its \
                 shadows' softness [3]",
            |p: &mut Parsing, v| {
                let deg: f32 = v.parse_if(|d: &f32| (0.05..=10.0).contains(d), "0.05 to 10")?;
                p.args.outdoor.clock.moon_radius = deg.to_radians();
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
            "switch the world source in process at these frames, as the menu's source entry \
                 does, and print the spawn's hash at the end (tests)",
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
    ];
    flags.extend(sky_light_flags());
    flags.extend(ceiling_flags());
    vec![Section {
        title: "Outdoors and the sky",
        flags,
    }]
}

/// The section's flags of the sky's light at a hit: the sky rays, their history, the penumbra,
/// the baked term's normal and bounce floor.
fn sky_light_flags() -> Vec<Flag<Parsing>> {
    vec![
        Flag::value(
            &["--sky-rays"],
            "N",
            "cosine-weighted sky rays a lighting pixel casts where no baked sky visibility \
                 is fresh, 0 (bake only) to 4 [1]",
            |p: &mut Parsing, v| {
                let max = vxe_render::MAX_SKY_RAYS;
                p.args.outdoor.sky_rays =
                    Some(v.parse_if(|n: &u32| (0..=max).contains(n), "0 to 4")?);
                Ok(())
            },
        ),
        Flag::value(
            &["--sky-history"],
            "N",
            "frames the sky rays' running mean holds at most, 1 to 1024 [64]",
            |p: &mut Parsing, v| {
                let max = vxe_render::MAX_SKY_HISTORY;
                p.args.outdoor.sky_history =
                    Some(v.parse_if(|n: &u32| (1..=max).contains(n), "1 to 1024")?);
                Ok(())
            },
        ),
        Flag::value(
            &["--key-history"],
            "N",
            "frames the key light's visibility mean holds at most, 1 to 1024 [256]",
            |p: &mut Parsing, v| {
                let max = vxe_render::MAX_SKY_HISTORY;
                p.args.outdoor.key_history =
                    Some(v.parse_if(|n: &u32| (1..=max).contains(n), "1 to 1024")?);
                Ok(())
            },
        ),
        Flag::value(
            &["--penumbra"],
            "disc|approach",
            "the key light's penumbra: a ray to a random point of its disc a frame, or one \
                 to its centre scored by how close it passes the rock beside it (a spike) [disc]",
            |p: &mut Parsing, v| {
                p.args.outdoor.penumbra =
                    v.named(vxe_render::Penumbra::from_name, "disc or approach")?;
                Ok(())
            },
        ),
        Flag::value(
            &["--sky-normal"],
            "face|smoothed",
            "the normal a face's baked sky light reads its visibilities by: the face's own, \
                 or the 27-count gradient's at every distance (a one-voxel stair reads one \
                 slope) [smoothed]",
            |p: &mut Parsing, v| {
                p.args.outdoor.sky_normal =
                    v.named(vxe_render::SkyNormal::from_name, "face or smoothed")?;
                Ok(())
            },
        ),
        Flag::value(
            &["--sky-bounce"],
            "B",
            "the baked sky light's bounce floor, 0 to 1: this share of a face's visibility is \
                 its brick's six visibilities' mean about the normal [0]",
            |p: &mut Parsing, v| {
                p.args.outdoor.sky_bounce =
                    v.parse_if(|b: &f32| (0.0..=1.0).contains(b), "0 to 1")?;
                Ok(())
            },
        ),
    ]
}

/// The section's flags of the auto exposure's ceiling: anchored to the sky, or a constant.
fn ceiling_flags() -> Vec<Flag<Parsing>> {
    vec![
        Flag::value(
            &["--sky-anchor"],
            "N|none",
            "while a sky is set, the auto exposure opens at most N EV (-4 to 12) over the \
                 exposure that puts sunlit ground at mid-grey, so a noon tunnel stays dark and \
                 the night still opens; none keeps the constant ceiling [4]",
            |p: &mut Parsing, v| {
                p.args.outdoor.sky_anchor = Some(match v.text {
                    "none" => None,
                    _ => Some(v.parse_if(|n: &f32| (-4.0..=12.0).contains(n), "-4 to 12 or none")?),
                });
                Ok(())
            },
        ),
        Flag::value(
            &["--column-wait"],
            "on|off",
            "on, the live queue bakes a chunk once every column over its reach holds all the \
                 chunks the streamer will hold; off, once none of those chunks waits to \
                 upload, and a chunk baked under a column not yet final bakes again once \
                 when it is (faster first light, a provisional sky meanwhile) [off]",
            |p: &mut Parsing, v| {
                p.args.outdoor.column_wait = v.on_off()?;
                Ok(())
            },
        ),
        Flag::switch(
            &["--dump-black"],
            "once a second while the store's black count is over 0, print the lighting pixels \
                 with no record value grouped by the chunk they hit: its distance in chunks, \
                 whether it holds a block, its queue tracks, the chunks that hold its columns \
                 from final and whether its reach crosses the full radius (a reading lever; \
                 counts the store's reads)",
            |p: &mut Parsing| {
                p.args.outdoor.dump_black = true;
            },
        ),
        Flag::value(
            &["--exposure-max"],
            "EV",
            "the auto exposure's constant ceiling in EV, -16 to 8 [+2 for --source outdoor, \
                 else +4]",
            |p: &mut Parsing, v| {
                p.args.outdoor.exposure_max_ev =
                    Some(v.parse_if(|n: &f32| (-16.0..=8.0).contains(n), "-16 to 8")?);
                Ok(())
            },
        ),
        Flag::value(
            &["--exposure-rate"],
            "EV/s",
            "the most the auto exposure moves toward its target a second, 0 to 64, 0 for no \
                 cap (a step is capped at a 15 fps frame's) [3.75: 1/16 EV a frame at 60 Hz]",
            |p: &mut Parsing, v| {
                p.args.outdoor.exposure_rate =
                    Some(v.parse_if(|n: &f32| (0.0..=64.0).contains(n), "0 to 64")?);
                Ok(())
            },
        ),
    ]
}

/// The refusal of a world flag beside `--source outdoor`.
pub(super) const OUTDOOR_REFUSES: &str = "--source outdoor builds its own hills; it does not \
     take --level, --castle, --vault, --room, --thin-rig, --seam-rig, --save or --replay";
