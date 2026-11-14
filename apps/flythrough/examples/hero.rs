//! Renders the README's still: by default the showcase cavern's `hall` shot, lit by its four
//! ores alone (the lamp off), with the fly-through's lighting and upscaler defaults.
//!
//! `cargo run --release -p flythrough --example hero -- [--source showcase|level]
//! [--shot hall|chasm|crystals|all] [--seed N] [--size WxH] [--at X,Y,Z] [--yaw DEG]
//! [--pitch DEG] [--exposure auto|E] [--carve R] [--out PATH] [--out-dir DIR] [--frames N]
//! [--lamp-bounce on|off] [--probe-spacing N] [--internal-scale 1|1.5|2] [--jitter on|off]
//! [--sharpen S] [--light-radius N] [--adapter SUBSTRING] [--lighting full|half]
//! [--shadow-walk full|coarse] [--history edits|gradient]
//! [--lamp on|off] [--filter on|off] [--look PRESET]`.
//!
//! `--source showcase` (the default) generates `flythrough::showcase` and frames one of its
//! shots (`--shot`, default `hall`); `--at`, `--yaw`, `--pitch` and `--exposure` override the
//! shot's fields; `--exposure auto` adapts like an eye (`AutoExposure::eye`, jumping to the
//! first frame's meter), and a number is a manual exposure (each shot's own by default). `--shot all` writes `hero-<name>.png` for every shot into `--out-dir`
//! (default `target/hero/`), and `hall` again at the fly-through's probe spacing (16) as
//! `hero-hall-spacing16.png`, the review set. `--source level` frames the bounded level from
//! its spawn (`--at`, `--yaw`, `--pitch`), with no shots. `--carve R` digs a sphere of radius
//! `R` voxels where the view ray meets rock before the frame is drawn. The lamp is off by
//! default for both sources; the lamp's bounce is off (`--lamp-bounce on`, or its alias
//! `--indirect on`, runs ADR-0043's lamp channel with the lamp on at the hero's probe spacing,
//! `HERO_PROBE_SPACING`, 32 voxels, `--probe-spacing` overrides it), emissive
//! light is read from the store, the whole world baked at the live queue's settings before each
//! still (ADR-0046; `--steady`, `--cache-read`, `--sampler`, `--emissive-history` and
//! `--emissive-guard` are refused, naming Plan 0057), and the upscaler at
//! the fly-through's defaults (`--internal-scale 1.5`: FSR 2; `--jitter` starts our upscaler,
//! jittered or not; `--internal-scale 1` without `--jitter on` is the native frame).
//! The look is a preset of `flythrough::LookPreset`, the fly-through's default (`face` with the
//! far normal) unless
//! `--look` names another; only `--look field` mirrors the
//! field's band slots and runs the field pass; the ores emit, and
//! the light radius is the source's (19 chunks on the showcase, 4 on the level). The mirror
//! syncs until its light-sample count stops changing, and the still is the last of
//! `--frames` frames (default 64) from the same pose, so the accumulation, the filter and the
//! probes settle. Each still prints its adapter, light samples, frames and mean sRGB luma.
//! Runs on the high-performance adapter (or `--adapter`'s) with no window.

use std::path::{Path, PathBuf};

use flythrough::cave::{CaveSource, LEVEL_CHUNKS};
use flythrough::cli::table::{Flag, Section, Table};
use flythrough::cli::{GONE_SINCE_0057, LightingArgs, gone_since_0057, lighting_flags};
use flythrough::showcase::{SHOTS, Shot, ShowcaseSource};
use vxe_render::{
    AutoExposure, Camera, Exposure, Frame, GpuWorld, LightParams, Lighting, MirrorConfig,
    RenderScale, wgpu,
};
use vxe_world::*;

/// The world the still frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    Level,
    Showcase,
}

/// One shot, or the review set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shots {
    One(Shot),
    All,
}

#[derive(Clone, Debug, PartialEq)]
struct Args {
    seed: u64,
    source: Source,
    shots: Shots,
    size: [u32; 2],
    at: Option<[i32; 3]>,
    yaw: Option<f32>,
    pitch: Option<f32>,
    exposure: Option<Exposure>,
    carve: i64,
    out: Option<PathBuf>,
    out_dir: PathBuf,
    frames: u32,
    /// The lighting rows the fly-through shares (`flythrough::cli::lighting_flags`).
    light: LightingArgs,
    internal_scale: RenderScale,
    /// `--jitter on|off` as given, which selects our upscaler; unset, FSR 2 above scale 1.
    jitter: Option<bool>,
    sharpen: f32,
    adapter: Option<String>,
    look: flythrough::LookPreset,
    /// `--help`: print the flag table and exit.
    help: bool,
}

impl Args {
    /// The still's lighting: the fly-through's (`flythrough::default_lighting`) with the
    /// shared rows applied.
    fn lighting(&self) -> Lighting {
        self.light.apply(flythrough::default_lighting())
    }

    /// The frame's jitter: `--jitter` when given (our upscaler), else off (FSR 2 jitters by
    /// its own sequence, and scale 1 has no upscaler).
    fn jitter(&self) -> bool {
        self.jitter.unwrap_or(false)
    }

    /// Whether the frame runs FSR 2: above scale 1 without `--jitter`, as the fly-through.
    fn fsr2(&self) -> bool {
        self.jitter.is_none() && self.internal_scale != RenderScale::One
    }
}

/// The light radius of the showcase: its farthest pair of chunks lies 18.06 chunks apart.
const SHOWCASE_LIGHT_RADIUS: i32 = 19;

/// What the rows write: the arguments, and whether `--shot` was given.
struct Parsing {
    args: Args,
    shot_given: bool,
}

/// The camera's, the output's and the frame's rows.
fn camera_flags() -> Vec<Flag<Parsing>> {
    vec![
        Flag::value(
            &["--yaw"],
            "DEG",
            "the camera's yaw",
            |p: &mut Parsing, v| {
                p.args.yaw = Some(v.parse()?);
                Ok(())
            },
        ),
        Flag::value(
            &["--pitch"],
            "DEG",
            "the camera's pitch",
            |p: &mut Parsing, v| {
                p.args.pitch = Some(v.parse()?);
                Ok(())
            },
        ),
        Flag::value(
            &["--exposure"],
            "auto|E",
            "adapt like an eye, or a fixed exposure",
            |p: &mut Parsing, v| {
                p.args.exposure = Some(if v.text == "auto" {
                    Exposure::Auto(AutoExposure::eye())
                } else {
                    Exposure::Manual(v.parse()?)
                });
                Ok(())
            },
        ),
        Flag::value(
            &["--carve"],
            "R",
            "dig a sphere of radius R where the view meets rock",
            |p: &mut Parsing, v| {
                p.args.carve = v.parse()?;
                Ok(())
            },
        ),
        Flag::value(
            &["--out"],
            "PATH",
            "the still's file",
            |p: &mut Parsing, v| {
                p.args.out = Some(v.text.into());
                Ok(())
            },
        ),
        Flag::value(
            &["--out-dir"],
            "DIR",
            "where --shot all writes",
            |p: &mut Parsing, v| {
                p.args.out_dir = v.text.into();
                Ok(())
            },
        ),
        Flag::value(
            &["--frames"],
            "N",
            "frames from the pose before the still (64)",
            |p: &mut Parsing, v| {
                p.args.frames = v.parse()?;
                Ok(())
            },
        ),
        Flag::value(
            &["--look"],
            "PRESET",
            "the look preset",
            |p: &mut Parsing, v| {
                p.args.look = flythrough::LookPreset::from_name(v.text)
                    .ok_or_else(|| v.takes(&flythrough::LookPreset::names()))?;
                Ok(())
            },
        ),
        Flag::value(
            &["--internal-scale"],
            "1|1.5|2",
            "the output size over the internal size",
            |p: &mut Parsing, v| {
                p.args.internal_scale = v.named(RenderScale::from_name, "1, 1.5 or 2")?;
                Ok(())
            },
        ),
        Flag::value(
            &["--jitter"],
            "on|off",
            "our upscaler, jittered or not",
            |p: &mut Parsing, v| {
                p.args.jitter = Some(v.on_off()?);
                Ok(())
            },
        ),
        Flag::value(
            &["--sharpen"],
            "S",
            "the upscaler's sharpening, 0 to 1",
            |p: &mut Parsing, v| {
                p.args.sharpen = v.parse_if(|s: &f32| (0.0..=1.0).contains(s), "0 to 1")?;
                Ok(())
            },
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
    ]
}

/// The hero's flag table: its own rows, the fly-through's shared lighting rows and its
/// refusals (`flythrough::cli::table`).
fn table() -> Table<Parsing> {
    let mut light = lighting_flags(|p: &mut Parsing| &mut p.args.light);
    light.push(Flag::refused(GONE_SINCE_0057, gone_since_0057));
    Table {
        usage: "hero [FLAGS]: renders the README's still (see the example's doc comment)",
        sections: vec![
            Section {
                title: "Still",
                flags: vec![
                    Flag::switch(
                        &["--help", "-h"],
                        "print this help and exit",
                        |p: &mut Parsing| p.args.help = true,
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
                        &["--source"],
                        "showcase|level",
                        "the world the still frames",
                        |p: &mut Parsing, v| {
                            p.args.source = v.word(
                                &[("showcase", Source::Showcase), ("level", Source::Level)],
                                "showcase or level",
                            )?;
                            Ok(())
                        },
                    ),
                    Flag::value(
                        &["--shot"],
                        "NAME|all",
                        "the showcase shot, or the review set",
                        |p: &mut Parsing, v| {
                            p.shot_given = true;
                            p.args.shots = if v.text == "all" {
                                Shots::All
                            } else {
                                Shots::One(Shot::named(v.text).ok_or_else(|| {
                                    let names: Vec<&str> = SHOTS.iter().map(|s| s.name).collect();
                                    v.takes(&format!("{} or all", names.join(", ")))
                                })?)
                            };
                            Ok(())
                        },
                    ),
                    Flag::value(
                        &["--size"],
                        "WxH",
                        "the still's size (1920x1080)",
                        |p: &mut Parsing, v| {
                            let (w, h) = v.text.split_once('x').ok_or("--size takes WxH")?;
                            let parse =
                                |s: &str| s.parse::<u32>().map_err(|e| format!("--size: {e}"));
                            p.args.size = [parse(w)?, parse(h)?];
                            Ok(())
                        },
                    ),
                    Flag::value(
                        &["--at"],
                        "X,Y,Z",
                        "the eye's voxel",
                        |p: &mut Parsing, v| {
                            let at: Vec<i32> = v
                                .text
                                .split(',')
                                .map(str::parse)
                                .collect::<Result<_, _>>()
                                .map_err(|e| format!("--at: {e}"))?;
                            p.args.at = Some(at.try_into().map_err(|_| "--at takes X,Y,Z")?);
                            Ok(())
                        },
                    ),
                ]
                .into_iter()
                .chain(camera_flags())
                .collect(),
            },
            Section {
                title: "Lighting",
                flags: light,
            },
        ],
    }
}

fn parse_args_from(argv: impl IntoIterator<Item = String>) -> Result<Args, String> {
    // The fly-through's lighting (its store read and filter weights, ADR-0049) with the lamp
    // and its bounce off: the still is lit by the world's emitters alone.
    let base = Lighting {
        lamp: false,
        lamp_bounce: false,
        ..flythrough::default_lighting()
    };
    let mut light = LightingArgs::from_lighting(&base, HERO_PROBE_SPACING);
    light.lamp = Some(false);
    let mut p = Parsing {
        args: Args {
            seed: 1,
            source: Source::Showcase,
            shots: Shots::One(SHOTS[0]),
            size: [1920, 1080],
            at: None,
            yaw: None,
            pitch: None,
            exposure: None,
            carve: 0,
            out: None,
            out_dir: PathBuf::from("target/hero"),
            frames: 64,
            light,
            internal_scale: flythrough::cli::DEFAULT_SCALE,
            jitter: None,
            sharpen: flythrough::cli::DEFAULT_SHARPEN,
            adapter: None,
            look: flythrough::LookPreset::DEFAULT,
            help: false,
        },
        shot_given: false,
    };
    table().parse_into(argv, &mut p)?;
    let args = p.args;
    if args.source == Source::Level && p.shot_given {
        return Err("--shot frames the showcase; the level takes --at, --yaw and --pitch".into());
    }
    if args.shots == Shots::All && args.out.is_some() {
        return Err("--shot all writes into --out-dir, not --out".into());
    }
    Ok(args)
}

/// The hero's probe spacing in voxels, wider than the fly-through's 16.
const HERO_PROBE_SPACING: u32 = 32;

/// One still to render: its file, its camera and exposure, and the probe spacing.
struct Still {
    path: PathBuf,
    camera: Camera,
    exposure: Exposure,
    probe_spacing: u32,
}

fn camera_at(eye: [i32; 3], yaw_deg: f32, pitch_deg: f32) -> Camera {
    Camera {
        position: eye.map(|v| v as f32 + 0.5).into(),
        yaw: yaw_deg.to_radians(),
        pitch: pitch_deg.to_radians(),
        vfov: 70f32.to_radians(),
    }
}

impl Args {
    /// The stills this run writes.
    fn stills(&self, level_spawn: VoxelCoord) -> Vec<Still> {
        let exposure = self.exposure.unwrap_or(self.lighting().exposure);
        let pose = |shot: &Shot| {
            let camera = camera_at(
                self.at.unwrap_or(shot.eye.0),
                self.yaw.unwrap_or(shot.yaw_deg as f32),
                self.pitch.unwrap_or(shot.pitch_deg as f32),
            );
            let exposure = self
                .exposure
                .unwrap_or(Exposure::Manual(shot.exposure_milli as f32 / 1000.0));
            (camera, exposure)
        };
        let single = |path: PathBuf, shot: &Shot| {
            let (camera, exposure) = pose(shot);
            Still {
                path,
                camera,
                exposure,
                probe_spacing: self.light.probe_spacing,
            }
        };
        match (self.source, self.shots) {
            (Source::Level, _) => vec![Still {
                path: self.out_path(),
                camera: camera_at(
                    self.at.unwrap_or(level_spawn.0),
                    self.yaw.unwrap_or(0.0),
                    self.pitch.unwrap_or(0.0),
                ),
                exposure,
                probe_spacing: self.light.probe_spacing,
            }],
            (Source::Showcase, Shots::One(shot)) => vec![single(self.out_path(), &shot)],
            (Source::Showcase, Shots::All) => {
                let mut v: Vec<Still> = SHOTS
                    .iter()
                    .map(|s| single(self.out_dir.join(format!("hero-{}.png", s.name)), s))
                    .collect();
                let mut fine = single(self.out_dir.join("hero-hall-spacing16.png"), &SHOTS[0]);
                fine.probe_spacing = vxe_render::DEFAULT_PROBE_SPACING;
                v.push(fine);
                v
            }
        }
    }

    fn out_path(&self) -> PathBuf {
        self.out
            .clone()
            .unwrap_or_else(|| PathBuf::from("docs/images/hero.png"))
    }
}

/// `exposure` as the still's line prints it: the manual multiplier, or `auto`.
fn exposure_words(exposure: Exposure) -> String {
    match exposure {
        Exposure::Manual(e) => e.to_string(),
        Exposure::Auto(_) => "auto".to_owned(),
    }
}

/// The mean sRGB luma of an RGBA8 image, Rec. 709 weights on the encoded values, over 255.
fn mean_luma(rgba: &[u8]) -> f64 {
    let sum: f64 = rgba
        .chunks(4)
        .map(|p| 0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2]))
        .sum();
    sum / (rgba.len() / 4) as f64
}

fn write_png(path: &Path, rgba: &[u8], [w, h]: [u32; 2]) -> Result<(), String> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .and_then(|mut wr| wr.write_image_data(rgba))
        .map_err(|e| format!("png: {e}"))
}

/// The world the run renders, its spawn, and the source's light radius.
fn build_world(args: &Args) -> (World, VoxelCoord, i32) {
    match args.source {
        Source::Showcase => {
            let source = ShowcaseSource::new(args.seed);
            let world = World::generate(
                source.extent(),
                &source,
                &flythrough::with_showcase_rows(&flythrough::with_emissive_ore(
                    &flythrough::cave_materials(),
                )),
            );
            (world, source.spawn(), SHOWCASE_LIGHT_RADIUS)
        }
        Source::Level => {
            let source = CaveSource::new(args.seed, ChunkExtent::new([0, 0, 0], LEVEL_CHUNKS));
            let world = World::generate(
                source.extent,
                &source,
                &flythrough::with_emissive_ore(&flythrough::cave_materials()),
            );
            (world, source.spawn(), vxe_render::DEFAULT_LIGHT_RADIUS)
        }
    }
}

/// With `--carve R`, carves a sphere of radius `R` voxels just past where the first still's view ray meets rock.
fn carve(args: &Args, world: &World, camera: &Camera) -> Result<(), String> {
    if args.carve > 0 {
        let eye = VoxelCoord(camera.position.to_array().map(|v| v.floor() as i32));
        let (dir, _, _) = camera.basis();
        let dir = Fixed3([dir.x, dir.y, dir.z].map(|c| (c * FIXED_ONE as f32).round() as i64));
        let o = centre_of_voxel(eye);
        let hit = world
            .raycast(o, dir.0, 400 * FIXED_ONE)
            .map_err(|e| format!("the view ray is refused: {e}"))?
            .ok_or("the view ray meets no rock within 40 m")?;
        let t = hit.t + args.carve * FIXED_ONE / 2;
        let centre = Fixed3([0, 1, 2].map(|k| o.0[k] + dir.0[k] * t / FIXED_ONE));
        let r = world
            .apply(&EditOp::sphere(centre, args.carve * FIXED_ONE, Mode::Carve))
            .map_err(|e| format!("carve: {e:?}"))?;
        let removed: u64 = r.changed_from.iter().map(|&(_, n)| u64::from(n)).sum();
        println!("carved {removed} voxels");
    }
    Ok(())
}

/// The adapter `--adapter` names (or the default), and a device sized for `world`'s pool.
fn open_device(args: &Args, world: &World) -> Result<(wgpu::Device, wgpu::Queue, String), String> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let named = args.adapter.as_deref().and_then(|want| {
        let all = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()));
        let found = all.into_iter().find(|a| a.get_info().name.contains(want));
        if found.is_none() {
            eprintln!("--adapter {want:?}: no adapter's name contains it; the default is used");
        }
        found
    });
    let adapter = match named {
        Some(a) => a,
        None => pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .map_err(|e| format!("no adapter: {e}"))?,
    };
    let adapter_name = adapter.get_info().name;
    println!("rendering on {adapter_name}");
    let field = args.look.normal_mode == vxe_render::NormalMode::Contour;
    let pool_bytes = flythrough::level_pool_units(world, field) * vxe_render::internals::UNIT_BYTES;
    let (device, queue) =
        pollster::block_on(
            adapter.request_device(&wgpu::DeviceDescriptor {
                required_limits: vxe_render::limits_for(&adapter, pool_bytes)
                    .map_err(|e| format!("no device: {e}"))?,
                ..Default::default()
            }),
        )
        .map_err(|e| format!("no device: {e}"))?;
    Ok((device, queue, adapter_name))
}

/// The mirror over the whole world, synced until its light samples stop changing.
fn fill_mirror(
    args: &Args,
    world: &World,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    light_radius: i32,
    eye: &Camera,
) -> Result<GpuWorld, String> {
    let field = args.look.normal_mode == vxe_render::NormalMode::Contour;
    // The bounded path's mirror budget, with the field's band slots under the field look.
    let chunks = world.extent().len() as u64;
    let cfg = MirrorConfig {
        budget_bytes: flythrough::level_mirror_budget(world, chunks, field),
        light_radius,
        ..MirrorConfig::new(vxe_render::Window::Fixed(world.extent()))
    };
    let mut gpu = if field {
        GpuWorld::with_field_surface(device, cfg)
    } else {
        GpuWorld::with_config(device, cfg)
    };
    // The light ranges around the eye take several syncs of the sample budget: sync until the
    // count stops changing.
    let focus = VoxelCoord(eye.position.to_array().map(|v| v.floor() as i32));
    gpu.set_focus(focus);
    let mut syncs = 0;
    let mut last = None;
    while syncs < 256 {
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        gpu.sync(world, queue, &mut enc)
            .map_err(|e| format!("mirror: {e:?}"))?;
        gpu.update_field(&mut enc);
        queue.submit([enc.finish()]);
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| format!("poll: {e}"))?;
        syncs += 1;
        let n = gpu.light_samples();
        if last == Some(n) {
            break;
        }
        last = Some(n);
    }
    println!(
        "mirror: {} light samples after {syncs} syncs",
        gpu.light_samples()
    );
    Ok(gpu)
}

/// Renders `still` after a bake and `args.frames` frames, writes it and prints its line.
fn render_still(
    args: &Args,
    world: &World,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    gpu: &mut GpuWorld,
    adapter_name: &str,
    still: &Still,
) -> Result<(), String> {
    let [w, h] = args.size;
    let frames = args.frames.max(1);
    let mut frame = Frame::with_sizes(device, args.internal_scale.internal(args.size), args.size);
    frame.set_probe_spacing(device, still.probe_spacing);
    if args.fsr2() {
        frame.set_upscaler_kind(
            device,
            vxe_render::UpscalerKind::Fsr2(vxe_render::Fsr2Settings {
                sharpness: args.sharpen,
                ..vxe_render::Fsr2Settings::default()
            }),
        );
    } else {
        frame.set_jitter(device, args.jitter());
        frame.set_sharpness(queue, args.sharpen);
    }
    frame.set_lighting(Lighting {
        exposure: still.exposure,
        ..args.lighting()
    });
    frame.set_look(vxe_render::LookParams {
        normal_mode: args.look.normal_mode,
        tuning: args.look.tuning,
        far_normal: args.look.far_normal,
        ..vxe_render::LookParams::default()
    });
    frame.set_resolution(device, args.light.resolution);
    let r = frame
        .bake_until(
            device,
            queue,
            gpu,
            world,
            world.extent(),
            &vxe_render::queue_settings(),
            &mut |_| {},
        )
        .map_err(|e| format!("bake: {e:?}"))?;
    println!("baked {} chunks, {} faces", r.chunks, r.faces);
    for _ in 0..frames {
        frame.step(device, queue, gpu, &still.camera, &LightParams::headlamp());
    }
    let rgba = vxe_render::debug::read_rgba8(device, queue, frame.target());
    write_png(&still.path, &rgba, [w, h])?;
    println!(
        "wrote {}: adapter {adapter_name}, {} light samples, {frames} frames, exposure {}, probe spacing {}, mean sRGB luma {:.1}/255",
        still.path.display(),
        gpu.light_samples(),
        exposure_words(still.exposure),
        still.probe_spacing,
        mean_luma(&rgba)
    );
    Ok(())
}

fn main() -> Result<(), String> {
    let args = parse_args_from(std::env::args().skip(1))?;
    if args.help {
        print!("{}", table().help());
        return Ok(());
    }
    let (world, level_spawn, light_radius) = build_world(&args);
    let light_radius = args.light.radius.unwrap_or(light_radius);
    let stills = args.stills(level_spawn);
    carve(&args, &world, &stills[0].camera)?;
    let (device, queue, adapter_name) = open_device(&args, &world)?;
    let mut gpu = fill_mirror(
        &args,
        &world,
        &device,
        &queue,
        light_radius,
        &stills[0].camera,
    )?;
    for still in &stills {
        render_still(
            &args,
            &world,
            &device,
            &queue,
            &mut gpu,
            &adapter_name,
            still,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(v: &[&str]) -> Result<Args, String> {
        parse_args_from(v.iter().map(|s| s.to_string()))
    }

    /// Without flags the still renders at the fly-through's defaults: its lighting
    /// (`flythrough::default_lighting`: the store read by the shading normal, the filters
    /// weighted by the face normal) with only the lamp and its bounce off, its look preset, and
    /// its upscaler (FSR 2 at 1.5, sharpness 0.5).
    #[test]
    fn hero_defaults_are_the_fly_throughs() {
        let d = parse(&[]).expect("defaults");
        let fly = flythrough::default_lighting();
        assert_eq!(
            d.lighting(),
            Lighting {
                lamp: false,
                lamp_bounce: false,
                ..fly
            }
        );
        assert_eq!(fly.store_read, flythrough::DEFAULT_STORE_READ);
        assert_eq!(fly.light_weights, flythrough::DEFAULT_LIGHT_WEIGHTS);
        assert_eq!(d.lighting().store_read, vxe_render::StoreRead::Normal);
        assert_eq!(d.lighting().light_weights, vxe_render::LightWeights::Face);
        assert_eq!(d.look, flythrough::LookPreset::DEFAULT);
        assert_eq!(d.internal_scale, flythrough::cli::DEFAULT_SCALE);
        assert_eq!(d.sharpen, flythrough::cli::DEFAULT_SHARPEN);
        assert!(d.fsr2() && !d.jitter());
    }

    /// The defaults are the showcase's `hall` shot at 1920 x 1080 with the lamp and its bounce
    /// off; `--shot` takes each preset and `all`, `--source` the level, and an
    /// unknown shot or source is refused with the names it takes.
    #[test]
    fn hero_parses_its_shots_and_sources() {
        let d = parse(&[]).expect("defaults");
        assert_eq!(d.source, Source::Showcase);
        assert_eq!(d.shots, Shots::One(SHOTS[0]));
        assert_eq!(d.size, [1920, 1080]);
        assert!(!d.lighting().lamp && !d.lighting().lamp_bounce);
        assert!(parse(&["--light", "baked"]).is_err());
        assert_eq!(
            (d.internal_scale, d.jitter(), d.fsr2()),
            (RenderScale::OneHalf, false, true)
        );
        assert_eq!(d.look, flythrough::LookPreset::DEFAULT);
        let native = parse(&["--internal-scale", "1"]).unwrap();
        assert!(!native.jitter() && !native.fsr2());
        let ours = parse(&["--jitter", "off"]).unwrap();
        assert!(!ours.jitter() && !ours.fsr2());
        // Each shot keeps its manual exposure; `--exposure auto` is the eye.
        for s in SHOTS {
            let one = parse(&["--shot", s.name]).unwrap();
            assert_eq!(
                one.stills(VoxelCoord([0; 3]))[0].exposure,
                Exposure::Manual(s.exposure_milli as f32 / 1000.0)
            );
        }
        assert_eq!(
            parse(&["--exposure", "auto"])
                .unwrap()
                .stills(VoxelCoord([0; 3]))[0]
                .exposure,
            Exposure::Auto(AutoExposure::eye())
        );
        assert_eq!(d.light.probe_spacing, 32);
        for s in SHOTS {
            assert_eq!(parse(&["--shot", s.name]).unwrap().shots, Shots::One(s));
        }
        let all = parse(&["--shot", "all"]).unwrap();
        assert_eq!(all.shots, Shots::All);
        // Compared as paths, not strings: `PathBuf::join` writes the platform separator, so the
        // display form differs on Windows while the components are the same.
        let stills = all.stills(VoxelCoord([0; 3]));
        let paths: Vec<&Path> = stills.iter().map(|s| s.path.as_path()).collect();
        assert_eq!(
            paths,
            [
                Path::new("target/hero/hero-hall.png"),
                Path::new("target/hero/hero-chasm.png"),
                Path::new("target/hero/hero-crystals.png"),
                Path::new("target/hero/hero-hall-spacing16.png")
            ]
        );
        assert_eq!(
            parse(&["--shot", "keep"]).err().as_deref(),
            Some("--shot takes hall, chasm, crystals or all, not keep")
        );
        assert_eq!(parse(&["--source", "level"]).unwrap().source, Source::Level);
        assert_eq!(
            parse(&["--source", "cave"]).err().as_deref(),
            Some("--source takes showcase or level, not cave")
        );
        assert!(parse(&["--source", "level", "--shot", "hall"]).is_err());
        assert!(parse(&["--shot", "all", "--out", "x.png"]).is_err());
        let o = parse(&["--shot", "chasm", "--yaw", "45", "--exposure", "2"]).unwrap();
        let s = &o.stills(VoxelCoord([0; 3]))[0];
        assert_eq!(s.exposure, Exposure::Manual(2.0));
        assert!((s.camera.yaw - 45f32.to_radians()).abs() < 1e-6);
        assert!((s.camera.pitch - (-30f32).to_radians()).abs() < 1e-6);
    }
}
