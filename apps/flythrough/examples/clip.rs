//! Records a rig's clip headlessly as numbered PNGs, for articles and the README:
//! `scripts/record-clip.sh` builds this, runs it and encodes the frames to GIF or MP4.
//!
//! `cargo run --release -p flythrough --example clip -- [--rig PATH] [--clip NAME]
//! [--source showcase|level] [--shot hall|chasm|crystals] [--seed N] [--place X,Y,Z]
//! [--ahead D] [--facing DEG] [--orbit DIST,HEIGHT,ANGLE] [--aim H] [--at X,Y,Z] [--yaw DEG]
//! [--pitch DEG] [--exposure auto|E] [--size WxH] [--fps N] [--loops N | --seconds S]
//! [--settle N] [--warmup N] [--out-dir DIR] [--adapter SUBSTRING] [--look PRESET]
//! [--internal-scale 1|1.5|2] [--jitter on|off] [--sharpen S] [lighting flags]`;
//! `--help` lists every flag.
//!
//! The world is the hero's (`--source`, `--shot`, `--seed`; the showcase's `hall` by default)
//! with the walker's rows appended to its table, so a library entry's `material` lines
//! resolve (`apps/flythrough/assets/entities/`). The rig (the spider by default) stands with
//! its feet at `--place`, or on the floor below the point `--ahead` voxels along the shot's
//! heading from its eye; it faces `--facing` degrees (yaw about z from facing -y), by default
//! towards the shot's eye. The camera orbits the model: `--orbit DIST,HEIGHT,ANGLE` puts it
//! `DIST` voxels from the feet across the floor, `HEIGHT` above them, `ANGLE` degrees round
//! from the model's front, looking at the point `--aim` voxels over the feet; `--at` (with
//! `--yaw` and `--pitch`, the shot's by default) fixes the camera instead. The exposure is a
//! manual 10 unless `--exposure` says otherwise.
//!
//! Output frame `n` shows the clip at tick `n * rate / fps` (the rig's rate), drawn between
//! that integer tick and the next by the remainder (`DisplayPose::between`): a function of
//! `n` alone, never of the wall clock. `--loops N` (1 by default) records `N` passes of the
//! clip; `--seconds S` records `S * fps` frames instead. Before frame 0 the frame runs
//! `--warmup` frames (64) at the first pose, and before each frame `--settle` frames (16) at
//! that frame's pose, so the light, the filter and the upscaler's history settle around the
//! moved parts; frame `n` is the last of those. The files are `frame-00000.png` on, in
//! `--out-dir` (`target/clip/<rig>-<clip>/` by default), whose old `frame-*.png` are removed
//! first. The lighting, look and upscaler are the hero's: the fly-through's defaults with the
//! lamp and its bounce off and the probes at 32 voxels.

use std::path::PathBuf;
use std::sync::Arc;

use flythrough::cli::table::{Flag, Section, Table, Val};
use flythrough::cli::{GONE_SINCE_0057, LightingArgs, gone_since_0057, lighting_flags};
use flythrough::showcase::{SHOTS, Shot};
use flythrough::viewer::{Viewer, placed};
use vxe_entity::{EntitySet, sample};
use vxe_render::{AutoExposure, Camera, DisplayPose, Exposure, LightParams, Lighting, RenderScale};
use vxe_world::*;

#[path = "shared/still.rs"]
mod still;

use still::{FrameSetup, Source, mean_luma, write_png};

/// The rig recorded when `--rig` is not given.
const DEFAULT_RIG: &str = "apps/flythrough/assets/entities/spider/spider.rig";

/// The manual exposure when `--exposure` is not given: brighter than the shots' 1.5, so the
/// model's dark shell reads against the hall (the hero renders at 8).
const DEFAULT_EXPOSURE: f32 = 10.0;

/// The probe spacing in voxels, the hero's.
const PROBE_SPACING: u32 = 32;

/// How far down from `--ahead`'s point the floor is looked for, in voxels.
const FLOOR_SEARCH: i64 = 400;

/// How many frames the run records.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Length {
    Loops(u32),
    Seconds(f64),
}

#[derive(Clone, Debug, PartialEq)]
struct Args {
    rig: PathBuf,
    clip: Option<String>,
    seed: u64,
    source: Source,
    shot: Shot,
    place: Option<[i32; 3]>,
    ahead: i32,
    facing: Option<f32>,
    /// Distance across the floor, height over the feet, degrees round from the front.
    orbit: [f32; 3],
    aim: f32,
    at: Option<[i32; 3]>,
    yaw: Option<f32>,
    pitch: Option<f32>,
    exposure: Option<Exposure>,
    size: [u32; 2],
    fps: u32,
    length: Length,
    settle: u32,
    warmup: u32,
    out_dir: Option<PathBuf>,
    light: LightingArgs,
    internal_scale: RenderScale,
    jitter: Option<bool>,
    sharpen: f32,
    adapter: Option<String>,
    look: flythrough::LookPreset,
    help: bool,
}

/// `X,Y,Z` as three integers.
fn triple<T: std::str::FromStr>(flag: &str, text: &str) -> Result<[T; 3], String>
where
    T::Err: std::fmt::Display,
{
    let v: Vec<T> = text
        .split(',')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .map_err(|e| format!("{flag}: {e}"))?;
    v.try_into()
        .map_err(|_| format!("{flag} takes three numbers"))
}

fn value(
    names: &'static [&'static str],
    arg: &'static str,
    help: &'static str,
    set: fn(&mut Args, &Val<'_>) -> Result<(), String>,
) -> Flag<Args> {
    Flag::value(names, arg, help, set)
}

/// The clip's flag table: its own rows, the fly-through's shared lighting rows and its
/// refusals (`flythrough::cli::table`).
fn table() -> Table<Args> {
    let mut light = lighting_flags(|a: &mut Args| &mut a.light);
    light.push(Flag::refused(GONE_SINCE_0057, gone_since_0057));
    Table {
        usage: "clip [FLAGS]: records a rig's clip as PNG frames (see the example's doc comment)",
        sections: vec![
            Section {
                title: "Model",
                flags: model_flags(),
            },
            Section {
                title: "Camera",
                flags: camera_flags(),
            },
            Section {
                title: "Output",
                flags: output_flags(),
            },
            Section {
                title: "World and frame",
                flags: world_flags(),
            },
            Section {
                title: "Lighting",
                flags: light,
            },
        ],
    }
}

/// The model's rows: the rig, its clip, its place and facing.
fn model_flags() -> Vec<Flag<Args>> {
    vec![
        Flag::switch(
            &["--help", "-h"],
            "print this help and exit",
            |a: &mut Args| a.help = true,
        ),
        value(
            &["--rig"],
            "PATH",
            "the rig to record (the spider)",
            |a, v| {
                a.rig = v.text.into();
                Ok(())
            },
        ),
        value(&["--clip"], "NAME", "the clip (the rig's first)", |a, v| {
            a.clip = Some(v.string());
            Ok(())
        }),
        value(&["--place"], "X,Y,Z", "the model's feet voxel", |a, v| {
            a.place = Some(triple("--place", v.text)?);
            Ok(())
        }),
        value(
            &["--ahead"],
            "D",
            "unplaced: the floor D voxels along the shot's heading (80)",
            |a, v| {
                a.ahead = v.parse()?;
                Ok(())
            },
        ),
        value(
            &["--facing"],
            "DEG",
            "the model's yaw from facing -y (towards the shot's eye)",
            |a, v| {
                a.facing = Some(v.parse()?);
                Ok(())
            },
        ),
    ]
}

/// The camera's rows: the orbit, or a fixed eye, and the exposure.
fn camera_flags() -> Vec<Flag<Args>> {
    vec![
        value(
            &["--orbit"],
            "DIST,HEIGHT,ANGLE",
            "the camera round the model (60,12,10)",
            |a, v| {
                a.orbit = triple("--orbit", v.text)?;
                Ok(())
            },
        ),
        value(
            &["--aim"],
            "H",
            "the orbit looks H voxels over the feet (14)",
            |a, v| {
                a.aim = v.parse()?;
                Ok(())
            },
        ),
        value(&["--at"], "X,Y,Z", "a fixed camera's eye voxel", |a, v| {
            a.at = Some(triple("--at", v.text)?);
            Ok(())
        }),
        value(
            &["--yaw"],
            "DEG",
            "the fixed camera's yaw (the shot's)",
            |a, v| {
                a.yaw = Some(v.parse()?);
                Ok(())
            },
        ),
        value(
            &["--pitch"],
            "DEG",
            "the fixed camera's pitch (the shot's)",
            |a, v| {
                a.pitch = Some(v.parse()?);
                Ok(())
            },
        ),
        value(
            &["--exposure"],
            "auto|E",
            "adapt like an eye, or a fixed exposure (10)",
            |a, v| {
                a.exposure = Some(if v.text == "auto" {
                    Exposure::Auto(AutoExposure::eye())
                } else {
                    Exposure::Manual(v.parse()?)
                });
                Ok(())
            },
        ),
    ]
}

/// The output's rows: the size, the rate, the length, the settling and the directory.
fn output_flags() -> Vec<Flag<Args>> {
    vec![
        value(&["--size"], "WxH", "the frames' size (1280x720)", |a, v| {
            let (w, h) = v.text.split_once('x').ok_or("--size takes WxH")?;
            let parse = |s: &str| s.parse::<u32>().map_err(|e| format!("--size: {e}"));
            a.size = [parse(w)?, parse(h)?];
            Ok(())
        }),
        value(&["--fps"], "N", "output frames a second (30)", |a, v| {
            a.fps = v.parse_if(|n: &u32| *n > 0, "a count above 0")?;
            Ok(())
        }),
        value(
            &["--loops"],
            "N",
            "passes of the clip to record (1)",
            |a, v| {
                a.length = Length::Loops(v.parse_if(|n: &u32| *n > 0, "a count above 0")?);
                Ok(())
            },
        ),
        value(
            &["--seconds"],
            "S",
            "seconds to record instead of loops",
            |a, v| {
                a.length = Length::Seconds(v.parse_if(|s: &f64| *s > 0.0, "seconds above 0")?);
                Ok(())
            },
        ),
        value(
            &["--settle"],
            "N",
            "frames drawn at each pose before it is kept (16)",
            |a, v| {
                a.settle = v.parse()?;
                Ok(())
            },
        ),
        value(
            &["--warmup"],
            "N",
            "frames drawn before the first pose's settle (64)",
            |a, v| {
                a.warmup = v.parse()?;
                Ok(())
            },
        ),
        value(
            &["--out-dir"],
            "DIR",
            "where the frames go (target/clip/<rig>-<clip>)",
            |a, v| {
                a.out_dir = Some(v.text.into());
                Ok(())
            },
        ),
    ]
}

/// The world's and the frame's rows.
fn world_flags() -> Vec<Flag<Args>> {
    vec![
        value(&["--seed"], "N", "the world's seed (1)", |a, v| {
            a.seed = v.parse()?;
            Ok(())
        }),
        value(
            &["--source"],
            "showcase|level",
            "the world (showcase)",
            |a, v| {
                a.source = v.word(
                    &[("showcase", Source::Showcase), ("level", Source::Level)],
                    "showcase or level",
                )?;
                Ok(())
            },
        ),
        value(
            &["--shot"],
            "NAME",
            "the showcase shot placing the model (hall)",
            |a, v| {
                a.shot = Shot::named(v.text).ok_or_else(|| {
                    let names: Vec<&str> = SHOTS.iter().map(|s| s.name).collect();
                    v.takes(&names.join(", "))
                })?;
                Ok(())
            },
        ),
        value(&["--look"], "PRESET", "the look preset", |a, v| {
            a.look = flythrough::LookPreset::from_name(v.text)
                .ok_or_else(|| v.takes(&flythrough::LookPreset::names()))?;
            Ok(())
        }),
        value(
            &["--internal-scale"],
            "1|1.5|2",
            "the output size over the internal size",
            |a, v| {
                a.internal_scale = v.named(RenderScale::from_name, "1, 1.5 or 2")?;
                Ok(())
            },
        ),
        value(
            &["--jitter"],
            "on|off",
            "our upscaler, jittered or not",
            |a, v| {
                a.jitter = Some(v.on_off()?);
                Ok(())
            },
        ),
        value(
            &["--sharpen"],
            "S",
            "the upscaler's sharpening, 0 to 1",
            |a, v| {
                a.sharpen = v.parse_if(|s: &f32| (0.0..=1.0).contains(s), "0 to 1")?;
                Ok(())
            },
        ),
        value(
            &["--adapter"],
            "SUBSTRING",
            "the first adapter whose name contains it",
            |a, v| {
                a.adapter = Some(v.string());
                Ok(())
            },
        ),
    ]
}

fn parse_args_from(argv: impl IntoIterator<Item = String>) -> Result<Args, String> {
    // The hero's lighting: the fly-through's with the lamp and its bounce off.
    let base = Lighting {
        lamp: false,
        lamp_bounce: false,
        ..flythrough::default_lighting()
    };
    let mut light = LightingArgs::from_lighting(&base, PROBE_SPACING);
    light.lamp = Some(false);
    let mut a = Args {
        rig: PathBuf::from(DEFAULT_RIG),
        clip: None,
        seed: 1,
        source: Source::Showcase,
        shot: SHOTS[0],
        place: None,
        ahead: 80,
        facing: None,
        orbit: [60.0, 12.0, 10.0],
        aim: 14.0,
        at: None,
        yaw: None,
        pitch: None,
        exposure: None,
        size: [1280, 720],
        fps: 30,
        length: Length::Loops(1),
        settle: 16,
        warmup: 64,
        out_dir: None,
        light,
        internal_scale: flythrough::cli::DEFAULT_SCALE,
        jitter: None,
        sharpen: flythrough::cli::DEFAULT_SHARPEN,
        adapter: None,
        look: flythrough::LookPreset::DEFAULT,
        help: false,
    };
    table().parse_into(argv, &mut a)?;
    if a.source == Source::Level && a.place.is_none() {
        return Err("--source level has no shot: give the model's feet with --place".into());
    }
    if a.source == Source::Level && a.at.is_none() && (a.yaw.is_some() || a.pitch.is_some()) {
        return Err("--yaw and --pitch turn a fixed camera: give its eye with --at".into());
    }
    Ok(a)
}

/// The output frames `length` takes of a clip `ticks` long at `rate` ticks a second, at `fps`,
/// and whether the last frame runs into the first (a loop's frames span whole ticks).
fn frame_count(length: Length, ticks: u32, rate: u32, fps: u32) -> (u64, bool) {
    match length {
        Length::Loops(n) => {
            let span = u64::from(n) * u64::from(ticks) * u64::from(fps);
            (span.div_ceil(u64::from(rate)), span % u64::from(rate) == 0)
        }
        Length::Seconds(s) => ((s * f64::from(fps)).round().max(1.0) as u64, false),
    }
}

/// Output frame `n`'s clip time at `fps` and `rate` ticks a second: the integer tick and the
/// fraction on to the next, from integers alone.
fn tick_of(n: u64, rate: u32, fps: u32) -> (u64, f32) {
    let t = n * u64::from(rate);
    let fps = u64::from(fps);
    (t / fps, (t % fps) as f32 / fps as f32)
}

/// The camera `DIST` voxels across the floor from `feet`, `HEIGHT` over them, `ANGLE` degrees
/// round from the front of a model turned `facing` radians, looking at `aim` over the feet.
fn orbit_camera(feet: [f32; 3], facing: f32, [dist, height, angle]: [f32; 3], aim: f32) -> Camera {
    // The model's front is -y turned by its yaw: (sin yaw, -cos yaw).
    let a = facing + angle.to_radians();
    let eye = [
        feet[0] + dist * a.sin(),
        feet[1] - dist * a.cos(),
        feet[2] + height,
    ];
    let d = [feet[0] - eye[0], feet[1] - eye[1], feet[2] + aim - eye[2]];
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    Camera {
        position: eye.into(),
        yaw: d[1].atan2(d[0]),
        pitch: (d[2] / len).asin(),
        vfov: 70f32.to_radians(),
    }
}

/// The first solid voxel's distance straight down from `p` within `search` voxels.
fn floor_below(world: &World, p: [f32; 3], search: i64) -> Option<f32> {
    let o = Fixed3(p.map(|v| (v * 256.0).round() as i64));
    let hit = world
        .raycast(o, [0, 0, -256], search * 256)
        .ok()
        .flatten()?;
    Some(hit.t as f32 / 256.0)
}

/// Whether the voxel holding `p` is solid (a ray from it starts inside a voxel).
fn inside_rock(world: &World, p: [f32; 3]) -> bool {
    let o = Fixed3(p.map(|v| (v * 256.0).round() as i64));
    matches!(world.raycast(o, [0, 0, 256], 256), Ok(Some(h)) if h.face.is_none())
}

/// The model's feet and facing (radians), and the camera: `--place` or the floor `--ahead`
/// along the shot's heading; `--facing` or towards the shot's eye; `--at` or the orbit.
fn stage(args: &Args, world: &World) -> Result<([f32; 3], f32, Camera), String> {
    let shot_eye = args.shot.eye.0.map(|v| v as f32 + 0.5);
    let feet = match args.place {
        Some([x, y, z]) => [x as f32 + 0.5, y as f32 + 0.5, z as f32],
        None => {
            let yaw = (args.shot.yaw_deg as f32).to_radians();
            let p = [
                shot_eye[0] + args.ahead as f32 * yaw.cos(),
                shot_eye[1] + args.ahead as f32 * yaw.sin(),
                shot_eye[2],
            ];
            let drop = floor_below(world, p, FLOOR_SEARCH)
                .ok_or("no floor below --ahead's point: give --place")?;
            [p[0], p[1], (p[2] - drop).floor()]
        }
    };
    let facing = args.facing.map_or_else(
        || flythrough::walker::facing(feet, shot_eye),
        f32::to_radians,
    );
    let camera = match args.at {
        Some(at) => still::camera_at(
            at,
            args.yaw.unwrap_or(args.shot.yaw_deg as f32),
            args.pitch.unwrap_or(args.shot.pitch_deg as f32),
        ),
        None => orbit_camera(feet, facing, args.orbit, args.aim),
    };
    Ok((feet, facing, camera))
}

fn main() -> Result<(), String> {
    let args = parse_args_from(std::env::args().skip(1))?;
    if args.help {
        print!("{}", table().help());
        return Ok(());
    }
    let mut viewer = Viewer::open(&args.rig, args.clip.clone())?;
    let (world, _, light_radius) = still::build_world(args.source, args.seed, |t| {
        flythrough::walker::with_walker(&t)
    });
    let light_radius = args.light.radius.unwrap_or(light_radius);
    if let Some(e) = viewer.build_pending(world.materials()) {
        return Err(format!("{}: {e}", args.rig.display()));
    }
    let loaded = viewer.loaded.clone().ok_or("the rig did not build")?;
    let clip_name = viewer.clip_name().unwrap_or("rest").to_owned();
    let clip = loaded.clips.get(viewer.clip).map(|(_, c)| c.clone());

    let (feet, facing, camera) = stage(&args, &world)?;
    let eye = camera.position.to_array();
    if inside_rock(&world, eye) {
        eprintln!("warning: the camera's eye {eye:?} is inside rock; move it with --orbit or --at");
    }
    println!(
        "{} clip {clip_name}: feet {:?}, facing {:.1} deg, eye [{:.1}, {:.1}, {:.1}] yaw {:.1} pitch {:.1}",
        args.rig.display(),
        feet,
        facing.to_degrees(),
        eye[0],
        eye[1],
        eye[2],
        camera.yaw.to_degrees(),
        camera.pitch.to_degrees()
    );

    let (ticks, rate) = (clip.as_ref().map_or(1, |c| c.length()), loaded.rate.max(1));
    let (frames, seamless) = frame_count(args.length, ticks, rate, args.fps);
    if matches!(args.length, Length::Loops(_)) && !seamless {
        eprintln!(
            "note: {ticks} ticks at {rate} a second do not fill whole frames at {} fps; the loop is not seamless",
            args.fps
        );
    }
    let out_dir = args.out_dir.clone().unwrap_or_else(|| {
        let stem = args
            .rig
            .file_stem()
            .map_or_else(|| "model".to_owned(), |s| s.to_string_lossy().into_owned());
        PathBuf::from(format!("target/clip/{stem}-{clip_name}"))
    });
    std::fs::create_dir_all(&out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    for e in std::fs::read_dir(&out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))? {
        let p = e.map_err(|e| e.to_string())?.path();
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned());
        if name.is_some_and(|n| n.starts_with("frame-") && n.ends_with(".png")) {
            std::fs::remove_file(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        }
    }

    let field = args.look.normal_mode == vxe_render::NormalMode::Contour;
    let (device, queue, adapter_name) = still::open_device(args.adapter.as_deref(), field, &world)?;
    let mut gpu = still::fill_mirror(&world, &device, &queue, field, light_radius, &camera)?;
    let exposure = args.exposure.unwrap_or(Exposure::Manual(DEFAULT_EXPOSURE));
    let mut frame = FrameSetup {
        size: args.size,
        internal_scale: args.internal_scale,
        jitter: args.jitter,
        sharpen: args.sharpen,
        probe_spacing: args.light.probe_spacing,
        lighting: Lighting {
            exposure,
            ..args.light.apply(flythrough::default_lighting())
        },
        look: args.look,
        resolution: args.light.resolution,
    }
    .frame(&device, &queue, &mut gpu, &world)?;
    frame.prepare_entities(&device);
    let handle = frame
        .entity_mirror_mut()
        .upload_model(&queue, &loaded.model, world.materials())
        .map_err(|e| format!("upload: {e}"))?;
    let mut set = EntitySet::new();
    let id = set
        .insert(
            Arc::clone(&loaded.model),
            placed(&loaded.rest, feet, facing),
        )
        .map_err(|e| format!("the rest pose: {e:?}"))?;
    let pose_at = |tick: u64| match &clip {
        Some(c) => placed(&sample(&loaded.model, &loaded.rest, c, tick), feet, facing),
        None => placed(&loaded.rest, feet, facing),
    };

    let start = flythrough::now();
    let lamp = LightParams::headlamp();
    for n in 0..frames {
        let (tick, alpha) = tick_of(n, rate, args.fps);
        let display =
            DisplayPose::between(&loaded.model, &pose_at(tick), &pose_at(tick + 1), alpha)
                .ok_or("the clip's pose does not fit the model")?;
        frame.set_entity_displays(&queue, [(id, handle, &display)]);
        let steps = args.settle.max(1) + if n == 0 { args.warmup } else { 0 };
        for _ in 0..steps {
            frame.step(&device, &queue, &gpu, &camera, &lamp);
        }
        let rgba = vxe_render::debug::read_rgba8(&device, &queue, frame.target());
        let path = out_dir.join(format!("frame-{n:05}.png"));
        write_png(&path, &rgba, args.size)?;
        if n == 0 || n + 1 == frames {
            println!(
                "wrote {}: tick {tick} + {alpha:.3}, mean sRGB luma {:.1}/255",
                path.display(),
                mean_luma(&rgba)
            );
        }
    }
    let secs = (flythrough::now() - start).as_secs_f64();
    println!(
        "recorded {frames} frames of {clip_name} ({ticks} ticks at {rate}/s) at {} fps, {}x{}, into {}: adapter {adapter_name}, settle {}, warmup {}, {:.1} s ({:.0} ms a frame), {} drawn",
        args.fps,
        args.size[0],
        args.size[1],
        out_dir.display(),
        args.settle.max(1),
        args.warmup,
        secs,
        1000.0 * secs / frames as f64,
        frame.entity_mirror().entities_drawn(),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(v: &[&str]) -> Result<Args, String> {
        parse_args_from(v.iter().map(|s| s.to_string()))
    }

    /// Frame `n` is at tick `n * rate / fps`, the remainder over `fps` the fraction on: the
    /// spider's walk (48 ticks at 30 a second) is 48 frames a loop at 30 fps and seamless,
    /// 39 at 24 (38.4 rounded up) and not; frame 5 at 24 fps is tick 6 and a quarter.
    #[test]
    fn frames_follow_the_clip_by_number() {
        assert_eq!(frame_count(Length::Loops(1), 48, 30, 30), (48, true));
        assert_eq!(frame_count(Length::Loops(2), 48, 30, 30), (96, true));
        assert_eq!(frame_count(Length::Loops(1), 48, 30, 24), (39, false));
        assert_eq!(frame_count(Length::Loops(5), 48, 30, 24), (192, true));
        assert_eq!(frame_count(Length::Seconds(2.0), 48, 30, 25), (50, false));
        assert_eq!(tick_of(0, 30, 30), (0, 0.0));
        assert_eq!(tick_of(47, 30, 30), (47, 0.0));
        assert_eq!(tick_of(5, 30, 24), (6, 0.25));
    }

    /// The defaults record one loop of the spider's first clip at 30 fps, 1280 x 720, from the
    /// hall, with the hero's lighting; the level needs `--place`, and a fixed camera's turn
    /// needs its eye there.
    #[test]
    fn clip_defaults_and_refusals() {
        let d = parse(&[]).expect("defaults");
        assert_eq!(d.rig, PathBuf::from(DEFAULT_RIG));
        assert_eq!((d.clip.as_deref(), d.fps, d.size), (None, 30, [1280, 720]));
        assert_eq!(d.length, Length::Loops(1));
        assert_eq!((d.settle, d.warmup), (16, 64));
        assert_eq!(d.shot, SHOTS[0]);
        assert_eq!(
            (d.orbit, d.aim, d.exposure),
            ([60.0, 12.0, 10.0], 14.0, None)
        );
        assert!(!d.light.apply(flythrough::default_lighting()).lamp);
        let s = parse(&["--seconds", "2.5", "--clip", "idle", "--orbit", "40,10,90"]).unwrap();
        assert_eq!(s.length, Length::Seconds(2.5));
        assert_eq!(s.clip.as_deref(), Some("idle"));
        assert_eq!(s.orbit, [40.0, 10.0, 90.0]);
        assert!(parse(&["--source", "level"]).is_err());
        assert!(parse(&["--source", "level", "--place", "1,2,3"]).is_ok());
        assert!(parse(&["--source", "level", "--place", "1,2,3", "--yaw", "4"]).is_err());
        assert!(parse(&["--fps", "0"]).is_err());
        assert!(parse(&["--orbit", "1,2"]).is_err());
    }

    /// The orbit at angle 0 stands in front of a model facing -y (yaw 0), so it looks along +y;
    /// at 90 it stands on the model's +x side, looking along -x.
    #[test]
    fn the_orbit_circles_the_model() {
        let front = orbit_camera([0.0; 3], 0.0, [10.0, 0.0, 0.0], 0.0);
        let p = front.position.to_array();
        assert!(p[0].abs() < 1e-4 && (p[1] + 10.0).abs() < 1e-4);
        assert!((front.yaw - 90f32.to_radians()).abs() < 1e-4);
        let side = orbit_camera([0.0; 3], 0.0, [10.0, 0.0, 90.0], 0.0);
        let p = side.position.to_array();
        assert!((p[0] - 10.0).abs() < 1e-4 && p[1].abs() < 1e-4);
        assert!((side.yaw.abs() - 180f32.to_radians()).abs() < 1e-4);
    }
}
