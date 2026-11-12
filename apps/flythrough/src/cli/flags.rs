//! The fly-through's flag table: every flag the parser reads, one row each, in the sections
//! `--help` prints. The rows write into [`Parsing`]; [`super::parse::settle`] then applies
//! the defaults that depend on other flags and refuses the combinations that do not hold.

use super::table::{Flag, Section, Table, Val};
use super::{Args, LightingArgs};
use crate::audio::{METRES_PER_VOXEL, Mix};
use crate::castle;
use crate::fly::CutawayMode;
use crate::hud;
use crate::path::{CameraPath, PathMode};
use crate::setup::CrispRows;
use crate::showcase::{SHOTS, Shot};
use crate::{LogicChoice, LookPreset};
use vxe_app::Pacing;
use vxe_render::{HistoryKill, LightingResolution, RenderScale, ShadowWalk, ViewMode};

/// What the rows write: the arguments, and the flags as given that [`super::parse::settle`]
/// resolves against each other once every flag is read.
pub(crate) struct Parsing {
    pub(crate) args: Args,
    pub(crate) given: Given,
}

/// The flags as given whose meaning depends on others: the world flags (`--level`,
/// `--source`, `--room`, the rigs) that settle into one [`super::RunMode`], the flags whose
/// default depends on the source, and the look's part flags, applied to the `--look` preset
/// after every flag is read.
#[derive(Default)]
pub(crate) struct Given {
    pub(crate) level: bool,
    pub(crate) showcase: bool,
    pub(crate) thin_rig: bool,
    pub(crate) seam_rig: bool,
    /// `--room`: the cube's edge in voxels.
    pub(crate) room: Option<i32>,
    pub(crate) shot: bool,
    pub(crate) exposure: bool,
    pub(crate) path_mode: bool,
    pub(crate) face_mix: Option<f32>,
    pub(crate) far_normal: Option<[f32; 2]>,
}

/// The refusal of a flag that chose the per-face table, the reference or a cut: the binary has
/// no such light, the live queue's store is the only one (ADR-0046), so the flag is an error
/// rather than ignored, and an old runner script fails loudly instead of measuring the wrong
/// light.
pub(crate) fn gone_since_0059(flag: &str) -> String {
    format!(
        "{flag} is gone since Plan 0059: the per-face table, the reference and the cuts were \
         deleted, and the light is the live queue's store (ADR-0046)"
    )
}

/// The refusal of the steady spike's flags and the screen path's: emissive light is read from
/// the store.
pub fn gone_since_0057(flag: &str) -> String {
    format!("{flag} is gone since Plan 0057: emissive light is read from the store (ADR-0046)")
}

/// The flags Plan 0057 removed, refused by the fly-through and its examples alike.
pub const GONE_SINCE_0057: &[&str] = &[
    "--steady",
    "--cache-read",
    "--sampler",
    "--emissive-history",
    "--emissive-guard",
    "--reuse-m-cap",
];

/// The flags Plan 0059 removed.
const GONE_SINCE_0059: &[&str] = &[
    "--light",
    "--cut",
    "--show",
    "--reference",
    "--bake-file",
    "--queue-file",
    "--bake-radius",
    "--blend",
    "--preload",
    "--preload-claim",
];

/// The lighting rows the fly-through and its examples share, writing the [`LightingArgs`]
/// that `part` reaches in `T`.
pub fn lighting_flags<T: 'static>(part: fn(&mut T) -> &mut LightingArgs) -> Vec<Flag<T>> {
    vec![
        Flag::value(
            &["--lamp"],
            "on|off",
            "the headlamp lights the frame (on; off on the showcase)",
            move |t, v| {
                part(t).lamp = Some(v.on_off()?);
                Ok(())
            },
        ),
        Flag::value(
            &["--lamp-bounce", "--indirect"],
            "on|off",
            "the lamp's bounce from the probe volume (ADR-0043)",
            move |t, v| {
                part(t).lamp_bounce = v.on_off()?;
                Ok(())
            },
        ),
        Flag::value(
            &["--shadow-walk"],
            "full|coarse",
            "the walk shadow rays take",
            move |t, v| {
                part(t).shadow_walk = v.named(ShadowWalk::from_name, "full or coarse")?;
                Ok(())
            },
        ),
        Flag::value(
            &["--store-read"],
            "face|normal",
            "how a cube face reads the store (ADR-0049)",
            move |t, v| {
                part(t).store_read = v.named(vxe_render::StoreRead::from_name, "face or normal")?;
                Ok(())
            },
        ),
        Flag::value(
            &["--light-weights"],
            "normal|face",
            "the normal the light's filters compare",
            move |t, v| {
                part(t).light_weights =
                    v.named(vxe_render::LightWeights::from_name, "normal or face")?;
                Ok(())
            },
        ),
        Flag::value(
            &["--filter"],
            "on|off",
            "the a-trous filter runs",
            move |t, v| {
                part(t).filter = v.on_off()?;
                Ok(())
            },
        ),
        Flag::value(
            &["--light-radius"],
            "N",
            "chunks whose emitters light the frame, 0 to 64",
            move |t, v| {
                part(t).radius =
                    Some(v.parse_if(|r: &i32| (0..=64).contains(r), "0 to 64 chunks")?);
                Ok(())
            },
        ),
        Flag::value(
            &["--history"],
            "edits|gradient",
            "what kills the temporal history",
            move |t, v| {
                part(t).history = v.named(HistoryKill::from_name, "edits or gradient")?;
                Ok(())
            },
        ),
        Flag::value(
            &["--lighting"],
            "full|half",
            "the lighting passes' resolution",
            move |t, v| {
                part(t).resolution = v.named(LightingResolution::from_name, "full or half")?;
                Ok(())
            },
        ),
        Flag::value(
            &["--probe-spacing"],
            "N",
            "voxels between probes, 4 to 64",
            move |t, v| {
                part(t).probe_spacing =
                    v.parse_if(|n: &u32| (4..=64).contains(n), "4 to 64 voxels")?;
                Ok(())
            },
        ),
        Flag::value(
            &["--probe-rays"],
            "N",
            "rays each updated probe traces, 1 to 64",
            move |t, v| {
                part(t).probe_rays = v.parse_if(
                    |n: &u32| (1..=vxe_render::internals::RAYS_PER_PROBE).contains(n),
                    "1 to 64 rays",
                )?;
                Ok(())
            },
        ),
        Flag::value(
            &["--probe-slices"],
            "N",
            "the probes refreshed a frame, one in N (8, 16 or 32)",
            move |t, v| {
                part(t).probe_slices =
                    v.parse_if(|n: &u32| [8, 16, 32].contains(n), "8, 16 or 32")?;
                Ok(())
            },
        ),
    ]
}

/// A comma-separated list of frame numbers.
fn frames(v: &Val<'_>) -> Result<Vec<u32>, String> {
    v.text
        .split(',')
        .map(str::parse::<u32>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("{}: frames, comma-separated, not {}: {e}", v.flag, v.text))
}

/// The run's rows: the seed, the size, the frame count, the pacing, the bench, the streamer's
/// budgets and the run's kind.
fn run_flags() -> Vec<Flag<Parsing>> {
    vec![
        Flag::switch(
            &["--help", "-h"],
            "print this help and exit",
            |p: &mut Parsing| p.args.help = true,
        ),
        Flag::switch(
            &["--check-args"],
            "parse the arguments, print `arguments ok` and exit",
            |p: &mut Parsing| {
                p.args.check_args = true;
            },
        ),
        Flag::value(
            &["--seed"],
            "N",
            "the world's seed (1)",
            |p: &mut Parsing, v| {
                p.args.seed = v.parse()?;
                Ok(())
            },
        ),
        Flag::value(
            &["--render"],
            "WxH",
            "the frame's output resolution",
            |p: &mut Parsing, v| {
                let (w, h) = v.text.split_once('x').ok_or("--render takes WxH")?;
                let parse = |s: &str| s.parse::<u32>().map_err(|e| format!("--render: {e}"));
                p.args.render = Some([parse(w)?, parse(h)?]);
                Ok(())
            },
        ),
        Flag::value(
            &["--frames"],
            "N",
            "exit after N frames, printing the readings",
            |p: &mut Parsing, v| {
                p.args.frames = Some(v.parse()?);
                Ok(())
            },
        ),
        Flag::value(
            &["--pace"],
            "free|compositor",
            "wait for the compositor's frame callback or not",
            |p: &mut Parsing, v| {
                p.args.pace = v.named(Pacing::from_word, "free or compositor")?;
                Ok(())
            },
        ),
        Flag::value(
            &["--bench"],
            "N",
            "send N bench inputs on the first frame",
            |p: &mut Parsing, v| {
                p.args.bench = v.parse()?;
                Ok(())
            },
        ),
        Flag::value(
            &["--dig-at"],
            "F,..",
            "dig at each of these frames",
            |p: &mut Parsing, v| {
                p.args.dig_at = frames(v)?;
                Ok(())
            },
        ),
        Flag::value(
            &["--bench-at"],
            "F,..",
            "a bench input at each of these frames",
            |p: &mut Parsing, v| {
                p.args.bench_at = frames(v)?;
                Ok(())
            },
        ),
        Flag::value(
            &["--sync-budget"],
            "MIB",
            "the mirror's upload bytes a frame",
            |p: &mut Parsing, v| {
                p.args.sync_budget = v.parse::<u64>()? << 20;
                Ok(())
            },
        ),
        Flag::value(
            &["--sync-ms"],
            "MS",
            "the mirror's upload time a frame (0 none)",
            |p: &mut Parsing, v| {
                let ms: u64 = v.parse()?;
                p.args.sync_time = (ms > 0).then(|| std::time::Duration::from_millis(ms));
                Ok(())
            },
        ),
        Flag::value(
            &["--workers"],
            "N",
            "the streamer's generation workers, at least 1",
            |p: &mut Parsing, v| {
                let n: usize = v.parse()?;
                if n == 0 {
                    return Err("--workers takes at least 1".to_owned());
                }
                p.args.workers = Some(n);
                Ok(())
            },
        ),
        Flag::value(
            &["--residency"],
            "R_FULL,R_LOD",
            "the streamed radii in chunks",
            |p: &mut Parsing, v| {
                p.args.residency = super::parse::parse_residency(v.text)?;
                Ok(())
            },
        ),
        Flag::value(
            &["--mirror-budget"],
            "MIB",
            "the VRAM the mirror reserves at startup",
            |p: &mut Parsing, v| {
                p.args.mirror_budget = Some(v.parse::<u64>()? << 20);
                Ok(())
            },
        ),
    ]
}

/// The session's rows: the run's kind, the adapter, the HUD, the save, the replay, the logic, the camera path and its report, the
/// tester mode and the pad.
fn session_flags() -> Vec<Flag<Parsing>> {
    vec![
        Flag::switch(
            &["--autopilot"],
            "dig, fly 300 m out and back, print the streaming readings",
            |p: &mut Parsing| {
                p.args.autopilot = true;
            },
        ),
        Flag::switch(
            &["--headless"],
            "render offscreen with no window",
            |p: &mut Parsing| p.args.headless = true,
        ),
        Flag::value(
            &["--adapter"],
            "SUBSTRING",
            "the first adapter whose name contains it",
            |p: &mut Parsing, v| {
                p.args.adapter = Some(v.string());
                Ok(())
            },
        ),
        Flag::switch(
            &["--hud"],
            "start with the stats HUD shown",
            |p: &mut Parsing| p.args.hud = true,
        ),
        Flag::switch(
            &["--census"],
            "print the brick census on exit",
            |p: &mut Parsing| p.args.census = true,
        ),
        Flag::value(
            &["--save"],
            "PATH",
            "write the session's log file on exit (with --level)",
            |p: &mut Parsing, v| {
                p.args.save = Some(v.text.into());
                Ok(())
            },
        ),
        Flag::value(
            &["--replay"],
            "PATH",
            "re-simulate a saved session before the first frame",
            |p: &mut Parsing, v| {
                p.args.replay = Some(v.text.into());
                Ok(())
            },
        ),
        Flag::value(
            &["--logic"],
            "native|wasm[=PATH]",
            "the server's logic",
            |p: &mut Parsing, v| {
                p.args.logic = LogicChoice::parse(v.text)?;
                Ok(())
            },
        ),
        Flag::value(
            &["--path"],
            "cave|showcase|castle",
            "fly that scene's camera path",
            |p: &mut Parsing, v| {
                p.args.path =
                    Some(CameraPath::named(v.text).ok_or_else(|| v.takes(&CameraPath::names()))?);
                Ok(())
            },
        ),
        Flag::value(
            &["--path-mode"],
            "bench|tour",
            "fly the path once, or loop it",
            |p: &mut Parsing, v| {
                p.args.path_mode = v.named(PathMode::from_name, "bench or tour")?;
                p.given.path_mode = true;
                Ok(())
            },
        ),
        Flag::value(
            &["--report"],
            "DIR",
            "write DIR/run.json at exit (with --path)",
            |p: &mut Parsing, v| {
                p.args.report = Some(v.text.into());
                Ok(())
            },
        ),
        Flag::switch(
            &["--tester"],
            "only movement, look, the dig, F1, T and Esc act",
            |p: &mut Parsing| {
                p.args.tester = true;
            },
        ),
        Flag::value(
            &["--pad"],
            "on|off",
            "read a gamepad in a window",
            |p: &mut Parsing, v| {
                p.args.pad = v.on_off()?;
                Ok(())
            },
        ),
    ]
}

/// The world's rows: the source, the rigs, the room, the castle, the vault and the camera's
/// start.
fn scene_flags() -> Vec<Flag<Parsing>> {
    vec![
        Flag::value(
            &["--source"],
            "cave|level|showcase",
            "the world: the streamed cave, the level or the showcase",
            |p: &mut Parsing, v| {
                (p.given.level, p.given.showcase) = v.word(
                    &[
                        ("cave", (false, false)),
                        ("level", (true, false)),
                        ("showcase", (false, true)),
                    ],
                    "cave, level or showcase",
                )?;
                Ok(())
            },
        ),
        Flag::switch(
            &["--level"],
            "the bounded level (as --source level)",
            |p: &mut Parsing| p.given.level = true,
        ),
        Flag::switch(
            &["--thin-rig"],
            "the bounded thin-feature scene",
            |p: &mut Parsing| p.given.thin_rig = true,
        ),
        Flag::switch(
            &["--seam-rig"],
            "the bounded seam scene",
            |p: &mut Parsing| p.given.seam_rig = true,
        ),
        Flag::value(
            &["--room"],
            "METRES",
            "a closed cube of air of that edge, 1 to 100 m",
            |p: &mut Parsing, v| {
                let m: f32 = v.text.parse().map_err(|_| v.takes("an edge in metres"))?;
                if !(1.0..=100.0).contains(&m) {
                    return Err(v.takes("1 to 100 metres"));
                }
                p.given.room = Some((m / METRES_PER_VOXEL).round() as i32);
                p.args.audio.room_line = true;
                Ok(())
            },
        ),
        Flag::value(
            &["--shot"],
            "NAME",
            "the showcase shot the camera starts at",
            |p: &mut Parsing, v| {
                p.given.shot = true;
                p.args.shot = Shot::named(v.text).ok_or_else(|| {
                    let names: Vec<&str> = SHOTS.iter().map(|s| s.name).collect();
                    v.takes(&names.join(", "))
                })?;
                Ok(())
            },
        ),
        Flag::value(
            &["--pose"],
            "vein|patch",
            "the settle rig's still camera on the level",
            |p: &mut Parsing, v| {
                p.args.pose = Some(crate::Pose::named(v.text).ok_or_else(|| {
                    let names: Vec<&str> = crate::START_POSES.iter().map(|p| p.name).collect();
                    v.takes(&names.join(", "))
                })?);
                Ok(())
            },
        ),
        Flag::switch(
            &["--walk"],
            "start in walk mode (on a bounded world)",
            |p: &mut Parsing| p.args.walk = true,
        ),
        Flag::value(
            &["--glide"],
            "V",
            "move along the heading at V voxels a second",
            |p: &mut Parsing, v| {
                p.args.glide = Some(v.parse_if(
                    |g: &f32| (0.0..=1000.0).contains(g),
                    "0 to 1000 voxels a second",
                )?);
                Ok(())
            },
        ),
        Flag::switch(
            &["--vault"],
            "the 16-voxel vault room 10 m ahead of the spawn",
            |p: &mut Parsing| p.args.vault = true,
        ),
        Flag::switch(
            &["--castle"],
            "the castle in the streamed cave's hall",
            |p: &mut Parsing| p.args.castle = true,
        ),
        Flag::value(
            &["--castle-out"],
            "PATH",
            "write the castle's .vox (implies --castle)",
            |p: &mut Parsing, v| {
                p.args.castle = true;
                p.args.castle_out = Some(v.text.into());
                Ok(())
            },
        ),
        Flag::value(
            &["--castle-shot"],
            "gate|nave|window|dungeon",
            "the castle pose to start at (implies --castle)",
            |p: &mut Parsing, v| {
                p.args.castle = true;
                p.args.castle_shot = castle::CastleShot::parse(v.text)
                    .ok_or_else(|| v.takes("gate, nave, window or dungeon"))?;
                Ok(())
            },
        ),
        Flag::value(
            &["--crisp"],
            "none|rock|all",
            "the table rows marked crisp",
            |p: &mut Parsing, v| {
                p.args.look.crisp =
                    CrispRows::named(v.text).ok_or_else(|| v.takes("none, rock or all"))?;
                Ok(())
            },
        ),
    ]
}

/// The look's rows: the preset and its parts, the view, the cutaway and the upscaler.
fn look_flags() -> Vec<Flag<Parsing>> {
    vec![
        Flag::value(
            &["--look"],
            "PRESET",
            "the starting look preset",
            |p: &mut Parsing, v| {
                p.args.look.preset =
                    LookPreset::from_name(v.text).ok_or_else(|| v.takes(&LookPreset::names()))?;
                Ok(())
            },
        ),
        Flag::value(
            &["--face-mix"],
            "K",
            "the preset's face mix, 0 to 0.5",
            |p: &mut Parsing, v| {
                p.given.face_mix = Some(super::parse::parse_unit(v.flag, v.text, 0.5)?);
                Ok(())
            },
        ),
        Flag::value(
            &["--far-normal"],
            "LO,HI",
            "face mode's far normal band in pixels a voxel",
            |p: &mut Parsing, v| {
                p.given.far_normal = Some(super::parse::parse_far_normal(v.text)?);
                Ok(())
            },
        ),
        Flag::switch(
            &["--band-census"],
            "print the field surface's band census",
            |p: &mut Parsing| {
                p.args.look.band_census = true;
            },
        ),
        Flag::value(
            &["--view"],
            "VIEW",
            "shaded, normals, hitmask, steps, cracks or albedo",
            |p: &mut Parsing, v| {
                p.args.look.view = v.named(
                    ViewMode::from_name,
                    "shaded, normals, hitmask, steps, cracks or albedo",
                )?;
                Ok(())
            },
        ),
        Flag::value(
            &["--translucent-layers"],
            "N",
            "translucent voxels composited before the hit",
            |p: &mut Parsing, v| {
                p.args.look.translucent_layers = v.text.parse().map_err(|_| v.takes("a count"))?;
                Ok(())
            },
        ),
        Flag::value(
            &["--cutaway"],
            "auto|on|off",
            "the cutaway view's start state",
            |p: &mut Parsing, v| {
                p.args.look.cutaway = v.named(CutawayMode::from_name, "auto, on or off")?;
                Ok(())
            },
        ),
        Flag::value(
            &["--internal-scale"],
            "1|1.5|2",
            "the output size over the internal size",
            |p: &mut Parsing, v| {
                p.args.look.internal_scale = v.named(RenderScale::from_name, "1, 1.5 or 2")?;
                Ok(())
            },
        ),
        Flag::value(
            &["--jitter"],
            "on|off",
            "the primary rays' sub-pixel jitter (selects own)",
            |p: &mut Parsing, v| {
                p.args.look.jitter = v.on_off()?;
                p.args.look.jitter_set = true;
                Ok(())
            },
        ),
        Flag::value(
            &["--sharpen"],
            "S",
            "the upscaler's sharpening, 0 to 1",
            |p: &mut Parsing, v| {
                p.args.look.sharpen = v.parse_if(|s: &f32| (0.0..=1.0).contains(s), "0 to 1")?;
                Ok(())
            },
        ),
        Flag::value(
            &["--upscaler"],
            "off|own|fsr2",
            "the upscaler",
            |p: &mut Parsing, v| {
                p.args.look.upscaler =
                    Some(v.named(hud::UpscalerMode::from_name, "off, own or fsr2")?);
                Ok(())
            },
        ),
        Flag::value(
            &["--pan"],
            "DEG_PER_S",
            "turn the camera at a constant yaw rate",
            |p: &mut Parsing, v| {
                p.args.look.pan = Some(super::parse::parse_pan(v.text)?);
                Ok(())
            },
        ),
        Flag::value(
            &["--jitter-phases"],
            "N",
            "FSR 2's jitter phase count, 1 to 1024",
            |p: &mut Parsing, v| {
                p.args.look.jitter_phases =
                    Some(v.parse_if(|n: &u32| (1..=1024).contains(n), "1 to 1024")?);
                Ok(())
            },
        ),
        Flag::value(
            &["--dither"],
            "on|off",
            "FSR 2's static one-level dither",
            |p: &mut Parsing, v| {
                p.args.look.dither = v.on_off()?;
                Ok(())
            },
        ),
    ]
}

/// The lighting's rows: the shared ones, then the ore, the lamp's readings and the exposure.
fn light_flags() -> Vec<Flag<Parsing>> {
    let mut rows = lighting_flags(|p: &mut Parsing| &mut p.args.light);
    rows.extend([
        Flag::value(
            &["--emissive-ore"],
            "on|off",
            "the ore glows and lights the cave",
            |p: &mut Parsing, v| {
                p.args.light.emissive_ore = v.on_off()?;
                Ok(())
            },
        ),
        Flag::value(
            &["--lamp-toggle"],
            "N",
            "switch the lamp every N frames",
            |p: &mut Parsing, v| {
                p.args.light.lamp_toggle = Some(v.parse_if(|n: &u32| *n > 0, "a frame count")?);
                Ok(())
            },
        ),
        Flag::switch(
            &["--fade-reading"],
            "dig, settle, switch the lamp off and print the fade",
            |p: &mut Parsing| {
                p.args.light.fade_reading = true;
            },
        ),
        Flag::value(
            &["--exposure"],
            "auto|E",
            "adapt like an eye, or a fixed exposure",
            |p: &mut Parsing, v| {
                p.args.light.exposure = super::parse::parse_exposure(v.text)?;
                p.given.exposure = true;
                Ok(())
            },
        ),
        Flag::value(
            &["--adaptation"],
            "eye|symmetric|instant",
            "the auto exposure's preset",
            |p: &mut Parsing, v| {
                p.args.light.adaptation =
                    v.named(hud::Adaptation::from_name, "eye, symmetric or instant")?;
                Ok(())
            },
        ),
        Flag::value(
            &["--adapt"],
            "LIGHT,CONE,ROD,FLOOR",
            "the preset's time constants and cone floor",
            |p: &mut Parsing, v| {
                p.args.light.adapt = Some(super::parse::parse_adapt(v.text)?);
                Ok(())
            },
        ),
        Flag::value(
            &["--metering"],
            "centre|average",
            "how the meter weights the frame",
            |p: &mut Parsing, v| {
                p.args.light.metering = v.word(
                    &[
                        ("centre", vxe_render::Metering::default()),
                        ("average", vxe_render::Metering::Average),
                    ],
                    "centre or average",
                )?;
                Ok(())
            },
        ),
    ]);
    rows
}

/// The light store's and the live queue's rows.
fn bake_flags() -> Vec<Flag<Parsing>> {
    vec![
        Flag::value(
            &["--bake-slice"],
            "MS",
            "the queue's slice a frame, 0.1 to 100 ms",
            |p: &mut Parsing, v| {
                let ms = v.parse_if(|ms: &f64| (0.1..=100.0).contains(ms), "0.1 to 100 ms")?;
                p.args.bake.budget = vxe_render::Budget::PerFrame { ms };
                Ok(())
            },
        ),
        Flag::value(
            &["--bake-budget"],
            "frame:MS|second:MS,CAP",
            "the queue's budget (ADR-0052)",
            |p: &mut Parsing, v| {
                p.args.bake.budget = vxe_render::Budget::parse(v.text)?;
                Ok(())
            },
        ),
        Flag::value(
            &["--coarse"],
            "N",
            "the coarse pass's samples a brick direction, 0 to 4096",
            |p: &mut Parsing, v| {
                p.args.bake.coarse = v.parse_if(
                    |n: &u32| *n <= vxe_render::QUEUE_BRICK_SAMPLES,
                    "0 to 4096 samples",
                )?;
                Ok(())
            },
        ),
        Flag::value(
            &["--near"],
            "R",
            "chunks baked in full before the first frame, 0 to 8",
            |p: &mut Parsing, v| {
                p.args.bake.near = v.parse_if(|r: &i32| (0..=8).contains(r), "0 to 8 chunks")?;
                Ok(())
            },
        ),
        Flag::value(
            &["--full-first"],
            "off|N",
            "the queue's full-first radius after the pre-roll",
            |p: &mut Parsing, v| {
                p.args.bake.full_first = Some(match v.text {
                    "off" => None,
                    _ => Some(v.parse_if(|r: &i32| (0..=8).contains(r), "off or 0 to 8 chunks")?),
                });
                Ok(())
            },
        ),
        Flag::value(
            &["--missing-light"],
            "black|neighbour|parent|chain",
            "what a face with no baked value reads (ADR-0053)",
            |p: &mut Parsing, v| {
                p.args.bake.missing_light = vxe_render::MissingLight::parse(v.text)?;
                Ok(())
            },
        ),
        Flag::value(
            &["--reach"],
            "full|partial",
            "whether a chunk waits for its whole light reach",
            |p: &mut Parsing, v| {
                p.args.bake.reach = vxe_render::ReachMode::parse(v.text)?;
                Ok(())
            },
        ),
        Flag::value(
            &["--queue-order"],
            "score|distance",
            "the live queue's order",
            |p: &mut Parsing, v| {
                p.args.bake.queue_distance =
                    v.word(&[("score", false), ("distance", true)], "score or distance")?;
                Ok(())
            },
        ),
        Flag::value(
            &["--queue-weights"],
            "A,R|A,tier",
            "the score's angle and refine weights",
            |p: &mut Parsing, v| {
                let bad = || v.takes("A,R or A,tier");
                let (a, r) = v.text.split_once(',').ok_or_else(bad)?;
                let b = &mut p.args.bake;
                b.queue_angle = a
                    .parse()
                    .ok()
                    .filter(|x: &f32| x.is_finite() && *x >= 0.0)
                    .ok_or_else(bad)?;
                if r == "tier" {
                    b.queue_tier = true;
                } else {
                    b.queue_refine = r
                        .parse()
                        .ok()
                        .filter(|x: &f32| x.is_finite() && *x > 0.0)
                        .ok_or_else(bad)?;
                    b.queue_tier = false;
                }
                Ok(())
            },
        ),
        Flag::value(
            &["--queue-withdraw"],
            "on|off",
            "withdraw the jobs of chunks the camera passed",
            |p: &mut Parsing, v| {
                p.args.bake.queue_withdraw = v.on_off()?;
                Ok(())
            },
        ),
        Flag::switch(
            &["--queue-census"],
            "read the queue's view census at every opening",
            |p: &mut Parsing| {
                p.args.bake.queue_census = true;
            },
        ),
        Flag::value(
            &["--edit-ms"],
            "MS",
            "the slice's growth while a dig waits, 0 to 100 ms",
            |p: &mut Parsing, v| {
                p.args.bake.edit_ms =
                    v.parse_if(|ms: &f64| (0.0..=100.0).contains(ms), "0 to 100 ms")?;
                Ok(())
            },
        ),
        Flag::value(
            &["--dig-reach"],
            "on|off",
            "a dig re-bakes the chunks within the light radius",
            |p: &mut Parsing, v| {
                p.args.bake.dig_reach = v.on_off()?;
                Ok(())
            },
        ),
        Flag::switch(
            &["--read-missing"],
            "read the store's no-value count while a dig waits",
            |p: &mut Parsing| {
                p.args.bake.read_missing = true;
            },
        ),
    ]
}

/// The removed flags, each refused with its message; `--help` lists none.
fn removed_flags() -> Vec<Flag<Parsing>> {
    vec![
        Flag::refused(GONE_SINCE_0057, gone_since_0057),
        Flag::refused(GONE_SINCE_0059, gone_since_0059),
    ]
}

/// The audio's rows: the output, the room line, the absorption, the mix and the microphone.
fn audio_flags() -> Vec<Flag<Parsing>> {
    vec![
        Flag::switch(
            &["--audio"],
            "open the default output device",
            |p: &mut Parsing| p.args.audio.on = true,
        ),
        Flag::switch(
            &["--audio-debug"],
            "--audio plus the callback's readings and a recording",
            |p: &mut Parsing| {
                p.args.audio.on = true;
                p.args.audio.debug = true;
            },
        ),
        Flag::switch(
            &["--room-line"],
            "print the room line whenever a fan completes",
            |p: &mut Parsing| {
                p.args.audio.room_line = true;
            },
        ),
        Flag::value(
            &["--absorption"],
            "A",
            "the cave row's absorption per metre, 0 to 1",
            |p: &mut Parsing, v| {
                let a: f32 = v.text.parse().map_err(|_| v.takes("a value in [0, 1]"))?;
                if !(0.0..=1.0).contains(&a) {
                    return Err(v.takes("a value in [0, 1]"));
                }
                p.args.audio.absorption = a;
                Ok(())
            },
        ),
        Flag::switch(
            &["--mic"],
            "play the input device at the listener (implies --audio)",
            |p: &mut Parsing| {
                p.args.audio.mic = true;
                p.args.audio.on = true;
            },
        ),
        Flag::value(
            &["--echo-db"],
            "DB",
            "the echoes' trim",
            |p: &mut Parsing, v| {
                p.args.audio.mix.echo_db = Mix::parse(v.flag, v.text)?;
                Ok(())
            },
        ),
        Flag::value(
            &["--tail-db"],
            "DB",
            "the tail's trim",
            |p: &mut Parsing, v| {
                p.args.audio.mix.tail_db = Mix::parse(v.flag, v.text)?;
                Ok(())
            },
        ),
        Flag::value(
            &["--predelay-ms"],
            "MS",
            "the tail's pre-delay",
            |p: &mut Parsing, v| {
                p.args.audio.mix.predelay_ms = Mix::parse(v.flag, v.text)?;
                Ok(())
            },
        ),
        Flag::switch(
            &["--tail-glide"],
            "the tail's lines glide instead of handing over",
            |p: &mut Parsing| {
                p.args.audio.mix.glide = true;
            },
        ),
        Flag::switch(
            &["--flat-mix"],
            "both room curves flat at 0 dB",
            |p: &mut Parsing| p.args.audio.mix.flat = true,
        ),
        Flag::switch(
            &["--direct-delay"],
            "the direct path's propagation delay",
            |p: &mut Parsing| {
                p.args.audio.mix.direct_delay = true;
            },
        ),
    ]
}

/// The fly-through's table.
pub(crate) fn table() -> Table<Parsing> {
    Table {
        usage: "flythrough [FLAGS]: the engine's testbed (the flags and keys are documented on \
                `flythrough::cli`)",
        sections: vec![
            Section {
                title: "Run",
                flags: run_flags(),
            },
            Section {
                title: "Session",
                flags: session_flags(),
            },
            Section {
                title: "World",
                flags: scene_flags(),
            },
            Section {
                title: "Look",
                flags: look_flags(),
            },
            Section {
                title: "Lighting",
                flags: light_flags(),
            },
            Section {
                title: "Light store and queue",
                flags: bake_flags(),
            },
            Section {
                title: "Audio",
                flags: audio_flags(),
            },
            Section {
                title: "Removed",
                flags: removed_flags(),
            },
        ],
    }
}

/// The `--help` text.
pub fn help() -> String {
    table().help()
}
