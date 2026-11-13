//! The argument parser: [`parse_args_from`] reads the flags through the flag table
//! ([`super::flags`]) into [`Args`], then settles the defaults that depend on other flags.

use super::flags::{Given, Parsing, table};
use super::{AudioArgs, BakeArgs, LightingArgs, LookArgs, RunMode};
use crate::audio::{CAVE_ACOUSTIC, Mix};
use crate::castle;
use crate::cli::{
    Args, DEFAULT_JITTER, DEFAULT_SCALE, DEFAULT_SHARPEN, SAVE_NEEDS_LEVEL, SHOWCASE_LIGHT_RADIUS,
    walk_allowed,
};
use crate::fly::CutawayMode;
use crate::path::{PATH_SEED, PathMode, Scene};
use crate::setup::CrispRows;
use crate::showcase::SHOTS;
use crate::streaming::SYNC_BUDGET_MIB;
use crate::{LogicChoice, LookPreset, RESIDENCY};
use vxe_app::Pacing;
use vxe_render::{Lighting, LookParams, ViewMode};
use vxe_world::Residency;

pub fn parse_args() -> Result<Args, String> {
    parse_args_from(std::env::args().skip(1))
}

/// The arguments after the program name.
pub(crate) fn parse_args_from(argv: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut p = Parsing {
        args: Args::defaults(),
        given: Given::default(),
    };
    table().parse_into(argv, &mut p)?;
    settle(&mut p.args, &p.given)?;
    Ok(p.args)
}

impl Args {
    /// Every flag's default.
    fn defaults() -> Self {
        let lighting = Lighting {
            store_read: crate::DEFAULT_STORE_READ,
            light_weights: crate::DEFAULT_LIGHT_WEIGHTS,
            ..Lighting::default()
        };
        Args {
            seed: 1,
            render: None,
            frames: None,
            pace: Pacing::Unthrottled,
            bench: 0,
            mode: RunMode::Cave,
            shot: SHOTS[0],
            pose: None,
            autopilot: false,
            headless: false,
            adapter: None,
            walk: false,
            vault: false,
            castle: false,
            castle_out: None,
            castle_shot: castle::CastleShot::Gate,
            sync_budget: SYNC_BUDGET_MIB << 20,
            sync_time: None,
            workers: None,
            mirror_budget: None,
            residency: RESIDENCY,
            hud: false,
            census: false,
            save: None,
            replay: None,
            logic: LogicChoice::Native,
            pad: true,
            dig_at: Vec::new(),
            bench_at: Vec::new(),
            glide: None,
            path: None,
            path_mode: PathMode::Bench,
            report: None,
            tester: false,
            check_args: false,
            help: false,
            audio: AudioArgs {
                on: false,
                debug: false,
                room_line: false,
                absorption: CAVE_ACOUSTIC[0],
                mic: false,
                mix: Mix::default(),
            },
            light: LightingArgs {
                lamp_bounce: true,
                ..LightingArgs::from_lighting(&lighting, vxe_render::DEFAULT_PROBE_SPACING)
            },
            look: LookArgs {
                preset: LookPreset::DEFAULT,
                view: ViewMode::Shaded,
                translucent_layers: LookParams::default().translucent_layers,
                cutaway: CutawayMode::Auto,
                crisp: CrispRows::None,
                band_census: false,
                internal_scale: DEFAULT_SCALE,
                jitter: DEFAULT_JITTER,
                jitter_set: false,
                sharpen: DEFAULT_SHARPEN,
                pan: None,
                jitter_phases: None,
                dither: false,
                upscaler: None,
            },
            bake: BakeArgs {
                budget: vxe_render::Budget::default(),
                coarse: vxe_render::DEFAULT_COARSE_SAMPLES,
                near: vxe_render::DEFAULT_NEAR,
                full_first: None,
                missing_light: vxe_render::MissingLight::Chain,
                reach: vxe_render::ReachMode::Full,
                queue_distance: false,
                queue_angle: vxe_render::QUEUE_ANGLE_WEIGHT,
                queue_refine: vxe_render::QUEUE_REFINE_WEIGHT,
                queue_tier: true,
                queue_withdraw: true,
                queue_census: false,
                edit_ms: vxe_render::DEFAULT_BAKE_EDIT_MS,
                dig_reach: true,
                read_missing: false,
            },
        }
    }
}

/// The world the world flags pick, refusing two that exclude each other.
fn run_mode(g: &Given) -> Result<RunMode, String> {
    if g.thin_rig && g.seam_rig {
        return Err("--thin-rig and --seam-rig are two scenes; pick one".to_owned());
    }
    if g.showcase && g.room.is_some() {
        return Err(SHOWCASE_REFUSES.to_owned());
    }
    if g.room.is_some() && g.level {
        return Err(ROOM_REFUSES.to_owned());
    }
    if (g.thin_rig || g.seam_rig) && (g.level || g.showcase || g.room.is_some()) {
        return Err(RIGS_REFUSE.to_owned());
    }
    Ok(if g.thin_rig {
        RunMode::ThinRig
    } else if g.seam_rig {
        RunMode::SeamRig
    } else if let Some(edge) = g.room {
        RunMode::Room(edge)
    } else if g.showcase {
        RunMode::Showcase
    } else if g.level {
        RunMode::Level
    } else {
        RunMode::Cave
    })
}

const SHOWCASE_REFUSES: &str = "--source showcase builds its own bounded hall; it does not take \
     --save, --replay, --vault, --autopilot, --room or --castle";
const ROOM_REFUSES: &str = "--room builds its own world; it does not take --level, --autopilot, \
     --vault, --save or --replay";
const RIGS_REFUSE: &str = "--thin-rig and --seam-rig build their own world; they do not take \
     --level, --source showcase, --castle, --autopilot, --vault, --room, --save or --replay";

/// What every flag read settles: the world, the defaults that depend on the source, the
/// look's parts, and the refusals of flags that do not combine.
fn settle(args: &mut Args, given: &Given) -> Result<(), String> {
    args.mode = run_mode(given)?;
    let mode = args.mode;
    // The showcase is lit by its ores alone, and its light radius keeps every emitter of the
    // hall in the buffer from any eye. A benchmark path runs with the lamp off too
    // (`PathMode::flags`), so its frame times read the world's own light.
    let bench = args.path.is_some() && args.path_mode == PathMode::Bench;
    args.light.lamp = Some(args.light.lamp.unwrap_or(!mode.is_showcase() && !bench));
    args.light.radius = Some(args.light.radius.unwrap_or(if mode.is_showcase() {
        SHOWCASE_LIGHT_RADIUS
    } else {
        vxe_render::DEFAULT_LIGHT_RADIUS
    }));
    // The showcase starts at its shot's manual exposure unless `--exposure` is given.
    if mode.is_showcase() && !given.exposure {
        args.light.exposure = Some(args.shot.exposure_milli as f32 / 1000.0);
    }
    if args.pose.is_some() && !mode.is_level() {
        let names: Vec<&str> = crate::START_POSES.iter().map(|p| p.name).collect();
        return Err(format!(
            "--pose places the camera on the bounded level ({}); it needs --level",
            names.join(", ")
        ));
    }
    if given.shot && !mode.is_showcase() {
        return Err("--shot frames the showcase; it needs --source showcase".to_owned());
    }
    let saves = args.save.is_some() || args.replay.is_some();
    if mode.is_showcase() && (saves || args.vault || args.autopilot || args.castle) {
        return Err(SHOWCASE_REFUSES.to_owned());
    }
    args.look.preset.tuning.face_mix = given.face_mix.unwrap_or(args.look.preset.tuning.face_mix);
    args.look.preset.far_normal = given.far_normal.unwrap_or(args.look.preset.far_normal);
    if args.autopilot && mode.is_level() {
        return Err("--autopilot streams; it does not take --level".to_owned());
    }
    if mode.room().is_some() && (args.autopilot || args.vault || saves) {
        return Err(ROOM_REFUSES.to_owned());
    }
    if matches!(mode, RunMode::ThinRig | RunMode::SeamRig)
        && (args.castle || args.autopilot || args.vault || saves)
    {
        return Err(RIGS_REFUSE.to_owned());
    }
    if args.castle && (mode.is_level() || args.vault || mode.room().is_some()) {
        return Err(
            "--castle stands in the streamed cave; it does not take --level (too short for \
             it), --vault (inside its box) or --room"
                .to_owned(),
        );
    }
    if args.walk {
        walk_allowed(mode == RunMode::Cave).map_err(|e| format!("--walk: {e}"))?;
    }
    if saves && !mode.is_level() {
        return Err(SAVE_NEEDS_LEVEL.to_owned());
    }
    if args.save.is_some() && args.replay.is_some() {
        return Err("--save and --replay do not combine".to_owned());
    }
    if matches!(args.logic, LogicChoice::Wasm(_)) && !cfg!(feature = "wasm") {
        return Err(crate::WASM_NEEDS_FEATURE.to_owned());
    }
    args.audio.mix.check()?;
    check_path(args, given.path_mode)?;
    Ok(())
}

/// `--path`'s rules: the path's scene and seed, nothing else steering the camera; `--path-mode`
/// and `--report` only with a path.
pub(crate) fn check_path(args: &Args, path_mode_flag: bool) -> Result<(), String> {
    let Some(path) = args.path else {
        if path_mode_flag || args.report.is_some() || args.tester {
            return Err(
                "--path-mode, --report and --tester play a camera path; they need --path"
                    .to_owned(),
            );
        }
        return Ok(());
    };
    if args.tester && args.hud {
        return Err("--tester keeps the stats HUD out of reach; it does not take --hud".to_owned());
    }
    let scene = if args.castle {
        Some(Scene::Castle)
    } else if args.mode.is_showcase() {
        Some(Scene::Showcase)
    } else if args.mode.is_level()
        || args.mode.room().is_some()
        || args.mode.is_thin_rig()
        || args.mode.is_seam_rig()
    {
        None
    } else {
        Some(Scene::Cave)
    };
    if scene != Some(path.scene) {
        return Err(format!(
            "--path {} flies the {} scene; it needs {}",
            path.name,
            path.scene.name(),
            path.scene.flags().join(" ")
        ));
    }
    if args.seed != PATH_SEED {
        return Err(format!(
            "--path {} is laid against seed {PATH_SEED}; it does not take --seed {}",
            path.name, args.seed
        ));
    }
    if args.autopilot || args.walk || args.glide.is_some() || args.look.pan.is_some() {
        return Err(
            "--path drives the camera; it does not take --autopilot, --walk, --glide or --pan"
                .to_owned(),
        );
    }
    Ok(())
}

/// `R_FULL,R_LOD`: radii in chunks with `1 <= R_FULL <= R_LOD`, hysteresis as
/// [`RESIDENCY`]'s.
/// `--exposure`: `auto` (`None`) or a positive finite manual exposure.
pub(crate) fn parse_exposure(v: &str) -> Result<Option<f32>, String> {
    if v == "auto" {
        return Ok(None);
    }
    v.parse::<f32>()
        .ok()
        .filter(|e| e.is_finite() && *e > 0.0)
        .map(Some)
        .ok_or(format!(
            "--exposure takes auto or a positive number, not {v}"
        ))
}

/// `--adapt LIGHT,CONE,ROD,FLOOR`: three time constants in seconds and a cone floor, each
/// finite and not negative.
pub(crate) fn parse_adapt(v: &str) -> Result<[f32; 4], String> {
    let bad =
        || format!("--adapt takes LIGHT,CONE,ROD,FLOOR: four numbers, none negative, not {v}");
    let parts: Vec<f32> = v
        .split(',')
        .map(|p| p.trim().parse::<f32>().map_err(|_| bad()))
        .collect::<Result<_, _>>()?;
    let a: [f32; 4] = parts.try_into().map_err(|_| bad())?;
    if a.iter().any(|x| !x.is_finite() || *x < 0.0) {
        return Err(bad());
    }
    Ok(a)
}

pub(crate) fn parse_residency(v: &str) -> Result<Residency, String> {
    let err = || format!("--residency takes R_FULL,R_LOD (e.g. 14,22), not {v}");
    let (full, lod) = v.split_once(',').ok_or_else(err)?;
    let parse = |s: &str| s.trim().parse::<i32>().map_err(|_| err());
    let [r_full, r_lod] = [parse(full)?, parse(lod)?];
    if r_full < 1 || r_lod < r_full {
        return Err(err());
    }
    Ok(Residency {
        r_full,
        r_lod,
        ..RESIDENCY
    })
}

/// A number in `[0, max]` for `flag`.
pub(crate) fn parse_unit(flag: &str, v: &str, max: f32) -> Result<f32, String> {
    let x: f32 = v
        .trim()
        .parse()
        .map_err(|_| format!("{flag} takes a number, not {v}"))?;
    if !(0.0..=max).contains(&x) {
        return Err(format!("{flag} must be in [0, {max}], not {x}"));
    }
    Ok(x)
}

/// `--far-normal LO,HI`: pixels per voxel, `0 <= LO < HI`.
/// `--pan`'s degrees a second: a finite number other than 0.
pub(crate) fn parse_pan(v: &str) -> Result<f32, String> {
    v.parse()
        .ok()
        .filter(|d: &f32| d.is_finite() && *d != 0.0)
        .ok_or(format!(
            "--pan takes degrees a second other than 0, not {v}"
        ))
}

/// The far normal `V` steps through: `[2, 6]`, `[1, 2]`, off, then `[2, 6]` again; any other
/// band steps to `[2, 6]`.
pub(crate) fn next_far_normal(f: [f32; 2]) -> [f32; 2] {
    if f == [2.0, 6.0] {
        [1.0, 2.0]
    } else if f == [1.0, 2.0] {
        [0.0, 0.0]
    } else {
        [2.0, 6.0]
    }
}

pub(crate) fn parse_far_normal(v: &str) -> Result<[f32; 2], String> {
    let err = || format!("--far-normal takes LO,HI with 0 <= LO < HI, not {v}");
    let (lo, hi) = v.split_once(',').ok_or_else(err)?;
    let lo: f32 = lo.trim().parse().map_err(|_| err())?;
    let hi: f32 = hi.trim().parse().map_err(|_| err())?;
    if lo >= 0.0 && lo < hi {
        Ok([lo, hi])
    } else {
        Err(err())
    }
}
