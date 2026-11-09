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
use flythrough::showcase::{SHOTS, Shot, ShowcaseSource};
use vxe_render::{
    AutoExposure, Camera, Exposure, Frame, GpuWorld, HistoryKill, LightParams, Lighting,
    LightingResolution, MirrorConfig, RenderScale, ShadowWalk, readback, wgpu,
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
    resolution: LightingResolution,
    lighting: Lighting,
    probe_spacing: u32,
    internal_scale: RenderScale,
    /// `--jitter on|off` as given, which selects our upscaler; unset, FSR 2 above scale 1.
    jitter: Option<bool>,
    sharpen: f32,
    light_radius: Option<i32>,
    adapter: Option<String>,
    look: flythrough::LookPreset,
}

impl Args {
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

fn parse_args_from(argv: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut args = Args {
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
        resolution: LightingResolution::default(),
        lighting: Lighting {
            lamp: false,
            ..Lighting::default()
        },
        probe_spacing: HERO_PROBE_SPACING,
        internal_scale: RenderScale::OneHalf,
        jitter: None,
        sharpen: 0.5,
        light_radius: None,
        adapter: None,
        look: flythrough::LookPreset::DEFAULT,
    };
    let mut shot_given = false;
    let on_off = |flag: &str, v: &str| match v {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err(format!("{flag} takes on or off, not {v}")),
    };
    let mut it = argv.into_iter();
    while let Some(flag) = it.next() {
        let v = it.next().ok_or(format!("{flag} needs a value"))?;
        let bad = |e: &dyn std::fmt::Display| format!("{flag}: {e}");
        match flag.as_str() {
            "--seed" => args.seed = v.parse().map_err(|e| bad(&e))?,
            "--source" => {
                args.source = match v.as_str() {
                    "showcase" => Source::Showcase,
                    "level" => Source::Level,
                    _ => return Err(format!("--source takes showcase or level, not {v}")),
                }
            }
            "--shot" => {
                shot_given = true;
                args.shots = if v == "all" {
                    Shots::All
                } else {
                    Shots::One(Shot::named(&v).ok_or_else(|| {
                        let names: Vec<&str> = SHOTS.iter().map(|s| s.name).collect();
                        format!("--shot takes {} or all, not {v}", names.join(", "))
                    })?)
                };
            }
            "--size" => {
                let (w, h) = v.split_once('x').ok_or("--size takes WxH")?;
                args.size = [
                    w.parse().map_err(|e| bad(&e))?,
                    h.parse().map_err(|e| bad(&e))?,
                ];
            }
            "--yaw" => args.yaw = Some(v.parse().map_err(|e| bad(&e))?),
            "--pitch" => args.pitch = Some(v.parse().map_err(|e| bad(&e))?),
            "--exposure" => {
                args.exposure = Some(if v == "auto" {
                    Exposure::Auto(AutoExposure::eye())
                } else {
                    Exposure::Manual(v.parse().map_err(|e| bad(&e))?)
                });
            }
            "--at" => {
                let p: Vec<i32> = v
                    .split(',')
                    .map(str::parse)
                    .collect::<Result<_, _>>()
                    .map_err(|e| bad(&e))?;
                args.at = Some(p.try_into().map_err(|_| "--at takes X,Y,Z")?);
            }
            "--carve" => args.carve = v.parse().map_err(|e| bad(&e))?,
            "--out" => args.out = Some(v.into()),
            "--out-dir" => args.out_dir = v.into(),
            "--look" => {
                args.look = flythrough::LookPreset::from_name(&v).ok_or(format!(
                    "--look takes {}, not {v}",
                    flythrough::LookPreset::names()
                ))?
            }
            "--steady" | "--cache-read" | "--sampler" | "--emissive-history"
            | "--emissive-guard" => {
                return Err(format!(
                    "{flag} is gone since Plan 0057: emissive light is read from the store (ADR-0046)"
                ));
            }
            "--frames" => args.frames = v.parse().map_err(|e| bad(&e))?,
            "--lamp-bounce" | "--indirect" => args.lighting.lamp_bounce = on_off(&flag, &v)?,
            "--probe-spacing" => {
                args.probe_spacing = v
                    .parse()
                    .ok()
                    .filter(|n| (4..=64).contains(n))
                    .ok_or(format!("--probe-spacing takes 4 to 64 voxels, not {v}"))?;
            }
            "--internal-scale" => {
                args.internal_scale = RenderScale::from_name(&v)
                    .ok_or(format!("--internal-scale takes 1, 1.5 or 2, not {v}"))?;
            }
            "--jitter" => args.jitter = Some(on_off(&flag, &v)?),
            "--sharpen" => {
                args.sharpen = v
                    .parse()
                    .ok()
                    .filter(|s: &f32| (0.0..=1.0).contains(s))
                    .ok_or(format!("--sharpen takes 0 to 1, not {v}"))?;
            }
            "--light-radius" => {
                args.light_radius = Some(
                    v.parse()
                        .ok()
                        .filter(|r| (0..=64).contains(r))
                        .ok_or(format!("--light-radius takes 0 to 64 chunks, not {v}"))?,
                );
            }
            "--adapter" => args.adapter = Some(v),
            "--lighting" => {
                args.resolution = LightingResolution::from_name(&v)
                    .ok_or(format!("--lighting takes full or half, not {v}"))?;
            }
            "--shadow-walk" => {
                args.lighting.shadow_walk = ShadowWalk::from_name(&v)
                    .ok_or(format!("--shadow-walk takes full or coarse, not {v}"))?;
            }
            "--history" => {
                args.lighting.history = HistoryKill::from_name(&v)
                    .ok_or(format!("--history takes edits or gradient, not {v}"))?;
            }
            "--lamp" => args.lighting.lamp = on_off(&flag, &v)?,
            "--filter" => args.lighting.filter = on_off(&flag, &v)?,
            _ => return Err(format!("unknown argument {flag}")),
        }
    }
    if args.source == Source::Level && shot_given {
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
        let exposure = self.exposure.unwrap_or(self.lighting.exposure);
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
                probe_spacing: self.probe_spacing,
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
                probe_spacing: self.probe_spacing,
            }],
            (Source::Showcase, Shots::One(shot)) => vec![single(self.out_path(), &shot)],
            (Source::Showcase, Shots::All) => {
                let mut v: Vec<Still> = SHOTS
                    .iter()
                    .map(|s| single(self.out_dir.join(format!("hero-{}.png", s.name)), s))
                    .collect();
                let mut fine = single(self.out_dir.join("hero-hall-spacing16.png"), &SHOTS[0]);
                fine.probe_spacing = vxe_render::probes::DEFAULT_PROBE_SPACING;
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

fn main() -> Result<(), String> {
    let args = parse_args_from(std::env::args().skip(1))?;
    let (world, level_spawn, light_radius) = match args.source {
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
            (
                world,
                source.spawn(),
                vxe_render::gpu_world::DEFAULT_LIGHT_RADIUS,
            )
        }
    };
    let light_radius = args.light_radius.unwrap_or(light_radius);
    let stills = args.stills(level_spawn);

    if args.carve > 0 {
        let camera = &stills[0].camera;
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
    let pool_bytes =
        flythrough::level_pool_units(&world, field) * vxe_render::gpu_world::UNIT_BYTES;
    let (device, queue) =
        pollster::block_on(
            adapter.request_device(&wgpu::DeviceDescriptor {
                required_limits: vxe_render::limits_for(&adapter, pool_bytes)
                    .map_err(|e| format!("no device: {e}"))?,
                ..Default::default()
            }),
        )
        .map_err(|e| format!("no device: {e}"))?;

    // The bounded path's mirror budget, with the field's band slots under the field look.
    let chunks = world.extent().len() as u64;
    let cfg = MirrorConfig {
        budget_bytes: flythrough::level_mirror_budget(&world, chunks, field),
        light_radius,
        ..MirrorConfig::new(vxe_render::Window::Fixed(world.extent()))
    };
    let mut gpu = if field {
        GpuWorld::with_field_surface(&device, cfg)
    } else {
        GpuWorld::with_config(&device, cfg)
    };
    // The light ranges around the eye take several syncs of the sample budget: sync until the
    // count stops changing.
    let focus = VoxelCoord(
        stills[0]
            .camera
            .position
            .to_array()
            .map(|v| v.floor() as i32),
    );
    gpu.set_focus(focus);
    let mut syncs = 0;
    let mut last = None;
    while syncs < 256 {
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        gpu.sync(&world, &queue, &mut enc)
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

    let [w, h] = args.size;
    let frames = args.frames.max(1);
    for still in &stills {
        let mut frame =
            Frame::with_sizes(&device, args.internal_scale.internal(args.size), args.size);
        frame.set_probe_spacing(&device, still.probe_spacing);
        if args.fsr2() {
            frame.set_upscaler_kind(
                &device,
                vxe_render::UpscalerKind::Fsr2(vxe_render::Fsr2Settings {
                    sharpness: args.sharpen,
                    ..vxe_render::Fsr2Settings::default()
                }),
            );
        } else {
            frame.set_jitter(&device, args.jitter());
            frame.set_sharpness(&queue, args.sharpen);
        }
        frame.set_lighting(Lighting {
            exposure: still.exposure,
            ..args.lighting
        });
        frame.set_look(vxe_render::LookParams {
            normal_mode: args.look.normal_mode,
            tuning: args.look.tuning,
            far_normal: args.look.far_normal,
            ..vxe_render::LookParams::default()
        });
        frame.set_resolution(&device, args.resolution);
        let r = frame
            .bake_until(
                &device,
                &queue,
                &mut gpu,
                &world,
                world.extent(),
                &vxe_render::bake::queue_settings(),
                &mut |_| {},
            )
            .map_err(|e| format!("bake: {e:?}"))?;
        println!("baked {} chunks, {} faces", r.chunks, r.faces);
        for _ in 0..frames {
            frame.step(
                &device,
                &queue,
                &gpu,
                &still.camera,
                &LightParams::headlamp(),
            );
        }
        let rgba = readback::read_rgba8(&device, &queue, frame.target());
        write_png(&still.path, &rgba, [w, h])?;
        println!(
            "wrote {}: adapter {adapter_name}, {} light samples, {frames} frames, exposure {}, probe spacing {}, mean sRGB luma {:.1}/255",
            still.path.display(),
            gpu.light_samples(),
            exposure_words(still.exposure),
            still.probe_spacing,
            mean_luma(&rgba)
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(v: &[&str]) -> Result<Args, String> {
        parse_args_from(v.iter().map(|s| s.to_string()))
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
        assert!(!d.lighting.lamp && !d.lighting.lamp_bounce);
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
        assert_eq!(d.probe_spacing, 32);
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
