//! Renders the README's still: by default the showcase cavern's `hall` shot, lit by its four
//! ores alone (the lamp off), with indirect light and the fly-through's upscaler defaults.
//!
//! `cargo run --release -p flythrough --example hero -- [--source showcase|level]
//! [--shot hall|chasm|crystals|all] [--seed N] [--size WxH] [--at X,Y,Z] [--yaw DEG]
//! [--pitch DEG] [--exposure auto|E] [--carve R] [--out PATH] [--out-dir DIR] [--frames N]
//! [--indirect on|off] [--probe-spacing N] [--internal-scale 1|1.5|2] [--jitter on|off]
//! [--sharpen S] [--light-radius N] [--adapter SUBSTRING] [--lighting full|half]
//! [--shadow-walk full|coarse] [--sampler list|reuse] [--history edits|gradient]
//! [--lamp on|off] [--filter on|off] [--emissive-history N] [--look PRESET]`.
//!
//! `--source showcase` (the default) generates `flythrough::showcase` and frames one of its
//! shots (`--shot`, default `hall`); `--at`, `--yaw`, `--pitch` and `--exposure` override the
//! shot's fields; `--exposure auto` adapts like an eye (`AutoExposure::eye`, jumping to the
//! first frame's meter), and a number is a manual exposure (each shot's own by default). `--shot all` writes `hero-<name>.png` for every shot into `--out-dir`
//! (default `target/hero/`), and `hall` again at the fly-through's probe spacing (16) as
//! `hero-hall-spacing16.png`, the review set. `--source level` frames the bounded level from
//! its spawn (`--at`, `--yaw`, `--pitch`), with no shots. `--carve R` digs a sphere of radius
//! `R` voxels where the view ray meets rock before the frame is drawn. The lamp is off by
//! default for both sources; indirect light is on at the hero's probe spacing
//! (`HERO_PROBE_SPACING`, 32 voxels, `--probe-spacing` overrides it) and the upscaler at
//! the fly-through's defaults (`--internal-scale 1`, `--jitter off`: none).
//! The look is a contour preset of `flythrough::LookPreset`, the fly-through's default unless
//! `--look` names another (its fit, tuning and rock detail); the ores emit, and
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
    LightingResolution, MirrorConfig, RenderScale, Sampler, ShadowWalk, readback, wgpu,
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
    jitter: bool,
    sharpen: f32,
    light_radius: Option<i32>,
    adapter: Option<String>,
    look: flythrough::LookPreset,
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
            indirect: true,
            ..Lighting::default()
        },
        probe_spacing: HERO_PROBE_SPACING,
        internal_scale: RenderScale::One,
        jitter: false,
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
                args.look = flythrough::LookPreset::from_name(&v)
                    .filter(|p| p.normal_mode == vxe_render::NormalMode::Contour)
                    .ok_or(format!("--look takes a contour preset, not {v}"))?
            }
            "--frames" => args.frames = v.parse().map_err(|e| bad(&e))?,
            "--indirect" => args.lighting.indirect = on_off(&flag, &v)?,
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
            "--jitter" => args.jitter = on_off(&flag, &v)?,
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
            "--sampler" => {
                args.lighting.sampler = Sampler::from_name(&v)
                    .ok_or(format!("--sampler takes list or reuse, not {v}"))?;
            }
            "--history" => {
                args.lighting.history = HistoryKill::from_name(&v)
                    .ok_or(format!("--history takes edits or gradient, not {v}"))?;
            }
            "--lamp" => args.lighting.lamp = on_off(&flag, &v)?,
            "--filter" => args.lighting.filter = on_off(&flag, &v)?,
            "--emissive-history" => {
                args.lighting.emissive_history = v.parse().map_err(|e| bad(&e))?;
            }
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
                    &args.look.materials(),
                )),
            );
            (world, source.spawn(), SHOWCASE_LIGHT_RADIUS)
        }
        Source::Level => {
            let source = CaveSource::new(args.seed, ChunkExtent::new([0, 0, 0], LEVEL_CHUNKS));
            let world = World::generate(
                source.extent,
                &source,
                &flythrough::with_emissive_ore(&args.look.materials()),
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
    let pool_bytes = flythrough::level_pool_units(&world, true) * vxe_render::gpu_world::UNIT_BYTES;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: vxe_render::limits_for(&adapter, pool_bytes),
        ..Default::default()
    }))
    .map_err(|e| format!("no device: {e}"))?;

    // Contour attributes on the bounded path's mirror budget, fitted as the look says.
    let chunks = world.extent().len() as u64;
    let mut gpu = GpuWorld::with_surface_attrs(
        &device,
        MirrorConfig {
            budget_bytes: flythrough::level_mirror_budget(&world, chunks, true),
            light_radius,
            ..MirrorConfig::new(vxe_render::Window::Fixed(world.extent()))
        },
    );
    // The light ranges around the eye take several syncs of the sample budget: sync until the
    // count stops changing.
    let focus = VoxelCoord(
        stills[0]
            .camera
            .position
            .to_array()
            .map(|v| v.floor() as i32),
    );
    gpu.set_surface_fit(args.look.tuning.fit);
    gpu.set_focus(focus);
    let mut syncs = 0;
    let mut last = None;
    while syncs < 256 {
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        gpu.sync(&world, &queue, &mut enc)
            .map_err(|e| format!("mirror: {e:?}"))?;
        gpu.update_attrs(&mut enc);
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
        frame.set_jitter(&device, args.jitter);
        frame.set_sharpness(&queue, args.sharpen);
        frame.set_lighting(Lighting {
            exposure: still.exposure,
            ..args.lighting
        });
        frame.set_look(vxe_render::LookParams {
            normal_mode: args.look.normal_mode,
            detail: true,
            tuning: args.look.tuning,
            ..vxe_render::LookParams::default()
        });
        frame.set_resolution(&device, args.resolution);
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

    /// The defaults are the showcase's `hall` shot at 1920 x 1080 with the lamp off and
    /// indirect light on; `--shot` takes each preset and `all`, `--source` the level, and an
    /// unknown shot or source is refused with the names it takes.
    #[test]
    fn hero_parses_its_shots_and_sources() {
        let d = parse(&[]).expect("defaults");
        assert_eq!(d.source, Source::Showcase);
        assert_eq!(d.shots, Shots::One(SHOTS[0]));
        assert_eq!(d.size, [1920, 1080]);
        assert!(!d.lighting.lamp && d.lighting.indirect);
        assert_eq!((d.internal_scale, d.jitter), (RenderScale::One, false));
        assert_eq!(d.probe_spacing, 32);
        for s in SHOTS {
            assert_eq!(parse(&["--shot", s.name]).unwrap().shots, Shots::One(s));
        }
        let all = parse(&["--shot", "all"]).unwrap();
        assert_eq!(all.shots, Shots::All);
        let names: Vec<String> = all
            .stills(VoxelCoord([0; 3]))
            .iter()
            .map(|s| s.path.display().to_string())
            .collect();
        assert_eq!(
            names,
            [
                "target/hero/hero-hall.png",
                "target/hero/hero-chasm.png",
                "target/hero/hero-crystals.png",
                "target/hero/hero-hall-spacing16.png"
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
