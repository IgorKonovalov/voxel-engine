//! The argument parser: `parse_args_from` reads the flags into [`Args`].

use crate::audio::{CAVE_ACOUSTIC, METRES_PER_VOXEL, Mix};
use crate::castle;
use crate::cli::{
    Args, DEFAULT_JITTER, DEFAULT_SCALE, DEFAULT_SHARPEN, SAVE_NEEDS_LEVEL, SHOWCASE_LIGHT_RADIUS,
    walk_allowed,
};
use crate::fly::CutawayMode;
use crate::hud::{self};
use crate::path::{CameraPath, PATH_SEED, PathMode, Scene};
use crate::setup::CrispRows;
use crate::showcase::{SHOTS, Shot};
use crate::streaming::SYNC_BUDGET_MIB;
use crate::{LogicChoice, LookPreset, RESIDENCY};
use vxe_app::Pacing;
use vxe_render::{HistoryKill, LightingResolution, LookParams, RenderScale, ShadowWalk, ViewMode};
use vxe_world::Residency;

pub fn parse_args() -> Result<Args, String> {
    parse_args_from(std::env::args().skip(1))
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

/// The arguments after the program name.
pub(crate) fn parse_args_from(argv: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut args = Args::defaults();
    let mut given = Given::default();
    let mut it = argv.into_iter();
    while let Some(flag) = it.next() {
        let mut value = || it.next().ok_or(format!("{flag} needs a value"));
        let taken = run_flag(&mut args, &flag, &mut value)?
            || look_flag(&mut args, &mut given, &flag, &mut value)?
            || audio_flag(&mut args, &flag, &mut value)?
            || lighting_flag(&mut args, &mut given, &flag, &mut value)?
            || bake_flag(&mut args, &flag, &mut value)?
            || scene_flag(&mut args, &mut given, &flag, &mut value)?;
        if !taken {
            return Err(format!("unknown argument {flag}"));
        }
    }
    settle(&mut args, &given)?;
    Ok(args)
}

/// The flags as given whose defaults depend on other flags (`--lamp` and `--light-radius` on
/// the source), and the look's part flags, applied to the `--look` preset after every flag is
/// read.
#[derive(Default)]
struct Given {
    lamp_flag: Option<bool>,
    shot_flag: bool,
    exposure_flag: bool,
    light_radius_flag: Option<i32>,
    path_mode_flag: bool,
    face_mix: Option<f32>,
    far_normal: Option<[f32; 2]>,
}

impl Args {
    /// Every flag's default.
    fn defaults() -> Self {
        Args {
            seed: 1,
            render: None,
            frames: None,
            pace: Pacing::Unthrottled,
            bench: 0,
            level: false,
            showcase: false,
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
            audio: false,
            audio_debug: false,
            look: LookPreset::DEFAULT,
            view: ViewMode::Shaded,
            translucent_layers: LookParams::default().translucent_layers,
            hud: false,
            census: false,
            band_census: false,
            save: None,
            replay: None,
            logic: LogicChoice::Native,
            room: None,
            thin_rig: false,
            seam_rig: false,
            crisp: CrispRows::None,
            room_line: false,
            absorption: CAVE_ACOUSTIC[0],
            mic: false,
            mix: Mix::default(),
            cutaway: CutawayMode::Auto,
            pad: true,
            emissive_ore: true,
            lamp: true,
            lamp_toggle: None,
            fade_reading: false,
            lamp_bounce: true,
            bake_budget: vxe_render::Budget::default(),
            bake_coarse: vxe_render::DEFAULT_COARSE_SAMPLES,
            bake_near: vxe_render::DEFAULT_NEAR,
            bake_full_first: None,
            missing_light: vxe_render::MissingLight::Chain,
            reach: vxe_render::ReachMode::Full,
            queue_distance: false,
            queue_angle: vxe_render::QUEUE_ANGLE_WEIGHT,
            queue_refine: vxe_render::QUEUE_REFINE_WEIGHT,
            queue_tier: true,
            queue_withdraw: true,
            queue_census: false,
            bake_edit_ms: vxe_render::DEFAULT_BAKE_EDIT_MS,
            dig_reach: true,
            dig_at: Vec::new(),
            bench_at: Vec::new(),
            read_missing: false,
            glide: None,
            probe_spacing: vxe_render::DEFAULT_PROBE_SPACING,
            probe_rays: vxe_render::internals::RAYS_PER_PROBE,
            probe_slices: vxe_render::internals::SLICES,
            shadow_walk: ShadowWalk::default(),
            store_read: crate::DEFAULT_STORE_READ,
            light_weights: crate::DEFAULT_LIGHT_WEIGHTS,
            filter: true,
            light_radius: vxe_render::DEFAULT_LIGHT_RADIUS,
            history: HistoryKill::default(),
            lighting: LightingResolution::default(),
            internal_scale: DEFAULT_SCALE,
            jitter: DEFAULT_JITTER,
            sharpen: DEFAULT_SHARPEN,
            pan: None,
            jitter_phases: None,
            dither: false,
            exposure: None,
            adaptation: hud::Adaptation::Eye,
            adapt: None,
            metering: vxe_render::Metering::default(),
            jitter_set: false,
            upscaler: None,
            path: None,
            path_mode: PathMode::Bench,
            report: None,
            tester: false,
            check_args: false,
        }
    }
}

/// The run's flags: the seed, the size, the frame count, the pacing, the bench and the
/// streamer's budgets. `Ok(false)` for a flag not of this group.
fn run_flag(
    args: &mut Args,
    flag: &str,
    value: &mut dyn FnMut() -> Result<String, String>,
) -> Result<bool, String> {
    match flag {
        "--seed" => args.seed = value()?.parse().map_err(|e| format!("--seed: {e}"))?,
        "--render" => {
            let v = value()?;
            let (w, h) = v.split_once('x').ok_or("--render takes WxH")?;
            let parse = |s: &str| s.parse::<u32>().map_err(|e| format!("--render: {e}"));
            args.render = Some([parse(w)?, parse(h)?]);
        }
        "--frames" => args.frames = Some(value()?.parse().map_err(|e| format!("--frames: {e}"))?),
        "--pace" => {
            let v = value()?;
            args.pace =
                Pacing::from_word(&v).ok_or(format!("--pace takes free or compositor, not {v}"))?;
        }
        "--bench" => args.bench = value()?.parse().map_err(|e| format!("--bench: {e}"))?,
        "--sync-budget" => {
            let mib: u64 = value()?
                .parse()
                .map_err(|e| format!("--sync-budget: {e}"))?;
            args.sync_budget = mib << 20;
        }
        "--sync-ms" => {
            let ms: u64 = value()?.parse().map_err(|e| format!("--sync-ms: {e}"))?;
            args.sync_time = (ms > 0).then(|| std::time::Duration::from_millis(ms));
        }
        "--workers" => {
            let n: usize = value()?.parse().map_err(|e| format!("--workers: {e}"))?;
            if n == 0 {
                return Err("--workers takes at least 1".to_owned());
            }
            args.workers = Some(n);
        }
        "--residency" => args.residency = parse_residency(&value()?)?,
        "--mirror-budget" => {
            let mib: u64 = value()?
                .parse()
                .map_err(|e| format!("--mirror-budget: {e}"))?;
            args.mirror_budget = Some(mib << 20);
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// The look's flags and the session's: the preset and its parts, the view, the translucent
/// layers, the cutaway, the save, the replay and the logic. `Ok(false)` for a flag not of this group.
fn look_flag(
    args: &mut Args,
    given: &mut Given,
    flag: &str,
    value: &mut dyn FnMut() -> Result<String, String>,
) -> Result<bool, String> {
    match flag {
        "--look" => {
            let v = value()?;
            args.look = LookPreset::from_name(&v)
                .ok_or(format!("--look takes {}, not {v}", LookPreset::names()))?;
        }
        "--face-mix" => given.face_mix = Some(parse_unit("--face-mix", &value()?, 0.5)?),
        "--far-normal" => given.far_normal = Some(parse_far_normal(&value()?)?),
        "--band-census" => args.band_census = true,
        "--view" => {
            let v = value()?;
            args.view = ViewMode::from_name(&v).ok_or(format!(
                "--view takes shaded, normals, hitmask, steps, cracks or albedo, not {v}"
            ))?;
        }
        "--translucent-layers" => {
            let v = value()?;
            args.translucent_layers = v
                .parse()
                .map_err(|_| format!("--translucent-layers takes a count, not {v}"))?;
        }
        "--cutaway" => {
            let v = value()?;
            args.cutaway = CutawayMode::from_name(&v)
                .ok_or(format!("--cutaway takes auto, on or off, not {v}"))?;
        }
        "--save" => args.save = Some(value()?.into()),
        "--replay" => args.replay = Some(value()?.into()),
        "--logic" => args.logic = LogicChoice::parse(&value()?)?,
        _ => return Ok(false),
    }
    Ok(true)
}

/// The audio's flags and the pad's: the room, the absorption, the mix and the microphone. `Ok(false)` for a flag not of this group.
fn audio_flag(
    args: &mut Args,
    flag: &str,
    value: &mut dyn FnMut() -> Result<String, String>,
) -> Result<bool, String> {
    match flag {
        "--room" => {
            let v = value()?;
            let m: f32 = v
                .parse()
                .map_err(|_| format!("--room takes an edge in metres, not {v}"))?;
            if !(1.0..=100.0).contains(&m) {
                return Err(format!("--room takes 1 to 100 metres, not {v}"));
            }
            args.room = Some((m / METRES_PER_VOXEL).round() as i32);
            args.room_line = true;
        }
        "--absorption" => {
            let v = value()?;
            let a: f32 = v
                .parse()
                .map_err(|_| format!("--absorption takes a value in [0, 1], not {v}"))?;
            if !(0.0..=1.0).contains(&a) {
                return Err(format!("--absorption takes a value in [0, 1], not {v}"));
            }
            args.absorption = a;
        }
        "--room-line" => args.room_line = true,
        "--echo-db" => args.mix.echo_db = Mix::parse(flag, &value()?)?,
        "--tail-db" => args.mix.tail_db = Mix::parse(flag, &value()?)?,
        "--predelay-ms" => args.mix.predelay_ms = Mix::parse(flag, &value()?)?,
        "--tail-glide" => args.mix.glide = true,
        "--flat-mix" => args.mix.flat = true,
        "--direct-delay" => args.mix.direct_delay = true,
        "--mic" => {
            args.mic = true;
            args.audio = true;
        }
        "--pad" => {
            args.pad = match value()?.as_str() {
                "on" => true,
                "off" => false,
                v => return Err(format!("--pad takes on or off, not {v}")),
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// The lighting's and the upscaler's flags, and the lamp's. `Ok(false)` for a flag not of this group.
fn lighting_flag(
    args: &mut Args,
    given: &mut Given,
    flag: &str,
    value: &mut dyn FnMut() -> Result<String, String>,
) -> Result<bool, String> {
    match flag {
        "--shadow-walk" => {
            let v = value()?;
            args.shadow_walk = ShadowWalk::from_name(&v)
                .ok_or(format!("--shadow-walk takes full or coarse, not {v}"))?;
        }
        "--store-read" => {
            let v = value()?;
            args.store_read = vxe_render::StoreRead::from_name(&v)
                .ok_or(format!("--store-read takes face or normal, not {v}"))?;
        }
        "--light-weights" => {
            let v = value()?;
            args.light_weights = vxe_render::LightWeights::from_name(&v)
                .ok_or(format!("--light-weights takes normal or face, not {v}"))?;
        }
        "--probe-rays" => {
            let v = value()?;
            args.probe_rays = v
                .parse()
                .ok()
                .filter(|n| (1..=vxe_render::internals::RAYS_PER_PROBE).contains(n))
                .ok_or(format!("--probe-rays takes 1 to 64 rays, not {v}"))?;
        }
        "--probe-slices" => {
            let v = value()?;
            args.probe_slices = v
                .parse()
                .ok()
                .filter(|n| [8, 16, 32].contains(n))
                .ok_or(format!("--probe-slices takes 8, 16 or 32, not {v}"))?;
        }
        "--probe-spacing" => {
            let v = value()?;
            args.probe_spacing = v
                .parse()
                .ok()
                .filter(|n| (4..=64).contains(n))
                .ok_or(format!("--probe-spacing takes 4 to 64 voxels, not {v}"))?;
        }
        "--internal-scale" => {
            let v = value()?;
            args.internal_scale = RenderScale::from_name(&v)
                .ok_or(format!("--internal-scale takes 1, 1.5 or 2, not {v}"))?;
        }
        "--pan" => {
            let v = value()?;
            args.pan = Some(parse_pan(&v)?);
        }
        "--jitter-phases" => {
            let v = value()?;
            args.jitter_phases = Some(
                v.parse()
                    .ok()
                    .filter(|n| (1..=1024).contains(n))
                    .ok_or(format!("--jitter-phases takes 1 to 1024, not {v}"))?,
            );
        }
        "--dither" => {
            args.dither = match value()?.as_str() {
                "on" => true,
                "off" => false,
                v => return Err(format!("--dither takes on or off, not {v}")),
            };
        }
        "--sharpen" => {
            let v = value()?;
            args.sharpen = v
                .parse()
                .ok()
                .filter(|s: &f32| (0.0..=1.0).contains(s))
                .ok_or(format!("--sharpen takes 0 to 1, not {v}"))?;
        }
        "--jitter" => {
            args.jitter = match value()?.as_str() {
                "on" => true,
                "off" => false,
                v => return Err(format!("--jitter takes on or off, not {v}")),
            };
            args.jitter_set = true;
        }
        "--upscaler" => {
            let v = value()?;
            args.upscaler = Some(
                hud::UpscalerMode::from_name(&v)
                    .ok_or(format!("--upscaler takes off, own or fsr2, not {v}"))?,
            );
        }
        "--lighting" => {
            let v = value()?;
            args.lighting = LightingResolution::from_name(&v)
                .ok_or(format!("--lighting takes full or half, not {v}"))?;
        }
        "--history" => {
            let v = value()?;
            args.history = HistoryKill::from_name(&v)
                .ok_or(format!("--history takes edits or gradient, not {v}"))?;
        }
        "--light-radius" => {
            let v = value()?;
            given.light_radius_flag = v
                .parse()
                .ok()
                .filter(|r| (0..=64).contains(r))
                .ok_or(format!("--light-radius takes 0 to 64 chunks, not {v}"))
                .map(Some)?;
        }
        "--filter" => {
            args.filter = match value()?.as_str() {
                "on" => true,
                "off" => false,
                v => return Err(format!("--filter takes on or off, not {v}")),
            }
        }
        "--lamp-toggle" => {
            let v = value()?;
            args.lamp_toggle = match v.parse::<u32>() {
                Ok(n) if n > 0 => Some(n),
                _ => return Err(format!("--lamp-toggle takes a frame count, not {v}")),
            }
        }
        "--fade-reading" => args.fade_reading = true,
        "--lamp" => {
            given.lamp_flag = match value()?.as_str() {
                "on" => Some(true),
                "off" => Some(false),
                v => return Err(format!("--lamp takes on or off, not {v}")),
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// The light store's and the live queue's flags, and the refusals of the removed ones. `Ok(false)` for a flag not of this group.
fn bake_flag(
    args: &mut Args,
    flag: &str,
    value: &mut dyn FnMut() -> Result<String, String>,
) -> Result<bool, String> {
    match flag {
        "--steady" | "--cache-read" | "--sampler" | "--emissive-history" | "--emissive-guard"
        | "--reuse-m-cap" => {
            return Err(format!(
                "{flag} is gone since Plan 0057: emissive light is read from the store (ADR-0046)"
            ));
        }
        "--light" | "--cut" | "--show" | "--reference" | "--bake-file" | "--queue-file"
        | "--bake-radius" | "--blend" | "--preload" | "--preload-claim" => {
            return Err(gone_since_0059(flag));
        }
        "--bake-slice" => {
            let v = value()?;
            let ms = v
                .parse()
                .ok()
                .filter(|ms: &f64| (0.1..=100.0).contains(ms))
                .ok_or(format!("--bake-slice takes 0.1 to 100 ms, not {v}"))?;
            args.bake_budget = vxe_render::Budget::PerFrame { ms };
        }
        "--bake-budget" => {
            let v = value()?;
            args.bake_budget = vxe_render::Budget::parse(&v)?;
        }
        "--coarse" => {
            let v = value()?;
            args.bake_coarse = v
                .parse()
                .ok()
                .filter(|n: &u32| *n <= vxe_render::QUEUE_BRICK_SAMPLES)
                .ok_or(format!("--coarse takes 0 to 4096 samples, not {v}"))?;
        }
        "--missing-light" => {
            args.missing_light = vxe_render::MissingLight::parse(&value()?)?;
        }
        "--reach" => {
            args.reach = vxe_render::ReachMode::parse(&value()?)?;
        }
        "--full-first" => {
            let v = value()?;
            args.bake_full_first = Some(match v.as_str() {
                "off" => None,
                _ => Some(
                    v.parse()
                        .ok()
                        .filter(|r: &i32| (0..=8).contains(r))
                        .ok_or(format!("--full-first takes off or 0 to 8 chunks, not {v}"))?,
                ),
            });
        }
        "--near" => {
            let v = value()?;
            args.bake_near = v
                .parse()
                .ok()
                .filter(|r: &i32| (0..=8).contains(r))
                .ok_or(format!("--near takes 0 to 8 chunks, not {v}"))?;
        }
        "--queue-order" => {
            let v = value()?;
            args.queue_distance = match v.as_str() {
                "score" => false,
                "distance" => true,
                _ => return Err(format!("--queue-order takes score or distance, not {v}")),
            };
        }
        "--queue-census" => args.queue_census = true,
        "--queue-withdraw" => {
            let v = value()?;
            args.queue_withdraw = match v.as_str() {
                "on" => true,
                "off" => false,
                _ => return Err(format!("--queue-withdraw takes on or off, not {v}")),
            };
        }
        "--queue-weights" => {
            let v = value()?;
            let bad = || format!("--queue-weights takes A,R or A,tier, not {v}");
            let (a, r) = v.split_once(',').ok_or_else(bad)?;
            args.queue_angle = a
                .parse()
                .ok()
                .filter(|x: &f32| x.is_finite() && *x >= 0.0)
                .ok_or_else(bad)?;
            if r == "tier" {
                args.queue_tier = true;
            } else {
                args.queue_refine = r
                    .parse()
                    .ok()
                    .filter(|x: &f32| x.is_finite() && *x > 0.0)
                    .ok_or_else(bad)?;
                args.queue_tier = false;
            }
        }
        "--edit-ms" => {
            let v = value()?;
            args.bake_edit_ms = v
                .parse()
                .ok()
                .filter(|ms: &f64| (0.0..=100.0).contains(ms))
                .ok_or(format!("--edit-ms takes 0 to 100 ms, not {v}"))?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// The scene's flags and the run's modes: the digs and benches at frames, the glide, the
/// source, the shot, the pose, the exposure, the castle, the path and the audio. `Ok(false)` for a flag not of this group.
fn scene_flag(
    args: &mut Args,
    given: &mut Given,
    flag: &str,
    value: &mut dyn FnMut() -> Result<String, String>,
) -> Result<bool, String> {
    match flag {
        "--read-missing" => args.read_missing = true,
        "--dig-at" | "--bench-at" => {
            let v = value()?;
            let frames = v
                .split(',')
                .map(|f| f.parse::<u32>())
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("{flag}: frames, comma-separated, not {v}: {e}"))?;
            if flag == "--dig-at" {
                args.dig_at = frames;
            } else {
                args.bench_at = frames;
            }
        }
        "--dig-reach" => {
            args.dig_reach = match value()?.as_str() {
                "on" => true,
                "off" => false,
                v => return Err(format!("--dig-reach takes on or off, not {v}")),
            };
        }
        "--glide" => {
            let v = value()?;
            args.glide = Some(
                v.parse()
                    .ok()
                    .filter(|g: &f32| (0.0..=1000.0).contains(g))
                    .ok_or(format!("--glide takes 0 to 1000 voxels a second, not {v}"))?,
            );
        }
        "--lamp-bounce" | "--indirect" => {
            args.lamp_bounce = match value()?.as_str() {
                "on" => true,
                "off" => false,
                v => return Err(format!("{flag} takes on or off, not {v}")),
            }
        }
        "--emissive-ore" => {
            args.emissive_ore = match value()?.as_str() {
                "on" => true,
                "off" => false,
                v => return Err(format!("--emissive-ore takes on or off, not {v}")),
            }
        }
        "--level" => args.level = true,
        "--thin-rig" => args.thin_rig = true,
        "--seam-rig" => args.seam_rig = true,
        "--crisp" => {
            let v = value()?;
            args.crisp = CrispRows::named(&v)
                .ok_or_else(|| format!("--crisp takes none, rock or all, not {v}"))?;
        }
        "--shot" => {
            let v = value()?;
            given.shot_flag = true;
            args.shot = Shot::named(&v).ok_or_else(|| {
                let names: Vec<&str> = SHOTS.iter().map(|s| s.name).collect();
                format!("--shot takes {}, not {v}", names.join(", "))
            })?;
        }
        "--pose" => {
            let v = value()?;
            args.pose = Some(crate::Pose::named(&v).ok_or_else(|| {
                let names: Vec<&str> = crate::START_POSES.iter().map(|p| p.name).collect();
                format!("--pose takes {}, not {v}", names.join(", "))
            })?);
        }
        "--source" => {
            let v = value()?;
            (args.level, args.showcase) = match v.as_str() {
                "cave" => (false, false),
                "level" => (true, false),
                "showcase" => (false, true),
                _ => return Err(format!("--source takes cave, level or showcase, not {v}")),
            };
        }
        "--autopilot" => args.autopilot = true,
        "--headless" => args.headless = true,
        "--adapter" => args.adapter = Some(value()?),
        "--walk" => args.walk = true,
        "--hud" => args.hud = true,
        "--exposure" => {
            args.exposure = parse_exposure(&value()?)?;
            given.exposure_flag = true;
        }
        "--adaptation" => {
            let v = value()?;
            args.adaptation = hud::Adaptation::from_name(&v).ok_or(format!(
                "--adaptation takes eye, symmetric or instant, not {v}"
            ))?;
        }
        "--adapt" => args.adapt = Some(parse_adapt(&value()?)?),
        "--metering" => {
            args.metering = match value()?.as_str() {
                "centre" => vxe_render::Metering::default(),
                "average" => vxe_render::Metering::Average,
                v => return Err(format!("--metering takes centre or average, not {v}")),
            }
        }
        "--census" => args.census = true,
        "--vault" => args.vault = true,
        "--castle" => args.castle = true,
        "--castle-out" => {
            args.castle = true;
            args.castle_out = Some(value()?.into());
        }
        "--castle-shot" => {
            args.castle = true;
            let v = value()?;
            args.castle_shot = castle::CastleShot::parse(&v).ok_or_else(|| {
                format!("--castle-shot takes gate, nave, window or dungeon, not {v}")
            })?;
        }
        "--path" => {
            let v = value()?;
            args.path = Some(
                CameraPath::named(&v)
                    .ok_or_else(|| format!("--path takes {}, not {v}", CameraPath::names()))?,
            );
        }
        "--path-mode" => {
            let v = value()?;
            args.path_mode = PathMode::from_name(&v)
                .ok_or(format!("--path-mode takes bench or tour, not {v}"))?;
            given.path_mode_flag = true;
        }
        "--report" => args.report = Some(value()?.into()),
        "--tester" => args.tester = true,
        "--check-args" => args.check_args = true,
        "--audio" => args.audio = true,
        "--audio-debug" => {
            args.audio = true;
            args.audio_debug = true;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// What every flag read settles: the defaults that depend on the source, the look's parts,
/// and the refusals of flags that do not combine.
fn settle(args: &mut Args, given: &Given) -> Result<(), String> {
    // The showcase is lit by its ores alone, and its light radius keeps every emitter of the
    // hall in the buffer from any eye.
    args.lamp = given.lamp_flag.unwrap_or(!args.showcase);
    args.light_radius = given.light_radius_flag.unwrap_or(if args.showcase {
        SHOWCASE_LIGHT_RADIUS
    } else {
        vxe_render::DEFAULT_LIGHT_RADIUS
    });
    // The showcase starts at its shot's manual exposure unless `--exposure` is given.
    if args.showcase && !given.exposure_flag {
        args.exposure = Some(args.shot.exposure_milli as f32 / 1000.0);
    }
    if args.pose.is_some() && !args.level {
        let names: Vec<&str> = crate::START_POSES.iter().map(|p| p.name).collect();
        return Err(format!(
            "--pose places the camera on the bounded level ({}); it needs --level",
            names.join(", ")
        ));
    }
    if given.shot_flag && !args.showcase {
        return Err("--shot frames the showcase; it needs --source showcase".to_owned());
    }
    if args.showcase
        && (args.save.is_some()
            || args.replay.is_some()
            || args.vault
            || args.autopilot
            || args.room.is_some()
            || args.castle)
    {
        return Err(
            "--source showcase builds its own bounded hall; it does not take --save, --replay, \
             --vault, --autopilot, --room or --castle"
                .to_owned(),
        );
    }
    args.look.tuning.face_mix = given.face_mix.unwrap_or(args.look.tuning.face_mix);
    args.look.far_normal = given.far_normal.unwrap_or(args.look.far_normal);
    if args.autopilot && args.level {
        return Err("--autopilot streams; it does not take --level".to_owned());
    }
    if args.room.is_some()
        && (args.level
            || args.autopilot
            || args.vault
            || args.save.is_some()
            || args.replay.is_some())
    {
        return Err(
            "--room builds its own world; it does not take --level, --autopilot, --vault, \
             --save or --replay"
                .to_owned(),
        );
    }
    if args.thin_rig && args.seam_rig {
        return Err("--thin-rig and --seam-rig are two scenes; pick one".to_owned());
    }
    if (args.thin_rig || args.seam_rig)
        && (args.level
            || args.showcase
            || args.castle
            || args.autopilot
            || args.vault
            || args.room.is_some()
            || args.save.is_some()
            || args.replay.is_some())
    {
        return Err(
            "--thin-rig and --seam-rig build their own world; they do not take --level, --source showcase, \
             --castle, --autopilot, --vault, --room, --save or --replay"
                .to_owned(),
        );
    }
    if args.castle && (args.level || args.vault || args.room.is_some()) {
        return Err(
            "--castle stands in the streamed cave; it does not take --level (too short for \
             it), --vault (inside its box) or --room"
                .to_owned(),
        );
    }
    if args.walk {
        walk_allowed(
            !args.level
                && !args.showcase
                && args.room.is_none()
                && !args.thin_rig
                && !args.seam_rig,
        )
        .map_err(|e| format!("--walk: {e}"))?;
    }
    if (args.save.is_some() || args.replay.is_some()) && !args.level {
        return Err(SAVE_NEEDS_LEVEL.to_owned());
    }
    if args.save.is_some() && args.replay.is_some() {
        return Err("--save and --replay do not combine".to_owned());
    }
    if matches!(args.logic, LogicChoice::Wasm(_)) && !cfg!(feature = "wasm") {
        return Err(crate::WASM_NEEDS_FEATURE.to_owned());
    }
    args.mix.check()?;
    check_path(args, given.path_mode_flag)?;
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
    } else if args.showcase {
        Some(Scene::Showcase)
    } else if args.level || args.room.is_some() || args.thin_rig || args.seam_rig {
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
    if args.autopilot || args.walk || args.glide.is_some() || args.pan.is_some() {
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
