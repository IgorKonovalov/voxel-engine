//! A reading-only strip tool: the showcase or the level rendered headless, with history, on the
//! adapter `VXE_TEST_ADAPTER` picks (`support::reading_device`), along a scripted camera. It
//! writes frames, crops, a contact sheet and a per-pixel flicker map under
//! `renders/strips/<tag>/`, and prints each frame's mean display luma. Nothing is asserted: the
//! one test is `#[ignore]`d, and the environment narrows it:
//!
//! - `VXE_STRIP_TAG`: the output directory's name (default `strip`).
//! - `VXE_STRIP_SOURCE`: `showcase` (default) or `level` (seed 1, the ore emissive).
//! - `VXE_STRIP_SHOT`: `hall` (default), `chasm` or `crystals` on the showcase; `vein`
//!   (default), `patch` or `spawn` (the fly-through's start, facing +x) on the level.
//! - `VXE_STRIP_CAMERA`: `x,y,z,yaw,pitch` (voxels, degrees) in place of the shot's camera;
//!   the shot still names the exposure.
//! - `VXE_STRIP_VIEW`: `shaded` (default), `normals`, `albedo`, `hitmask` or `steps`.
//! - `VXE_STRIP_SIZE`: the output size `WxH` (default `1920x1080`).
//! - `VXE_STRIP_UPSCALER`: `fsr2` (default), `own` or `off`; `fsr2` and `own` at scale 1.5
//!   with sharpening 0.5, `off` at scale 1 with no jitter.
//! - `VXE_STRIP_LOOK`: a `LookPreset` name (default: `LookPreset::DEFAULT`, the fly-through's
//!   start, far normal `[2, 6]`; `face` is the same cubes with the far normal off).
//! - `VXE_STRIP_FAR`: `LO,HI`, `LookParams::far_normal` (default: the preset's).
//! - `VXE_STRIP_ENERGY`: crops `x,y,w,h` separated by `/`: the last frame's mean horizontal and
//!   vertical neighbour `|dY|` is printed for each.
//! - `VXE_STRIP_LAMP`: `on` or `off` at the start (default off on the showcase, on on the level).
//! - `VXE_STRIP_INDIRECT`, `VXE_STRIP_FILTER`: `on` (default) or `off`.
//! - `VXE_STRIP_SAMPLER` and `VXE_STRIP_STEADY`: refused (the reservoir sampler and the steady
//!   presets are gone, ADR-0043).
//! - `VXE_STRIP_CANDIDATES`: emissive candidates per pixel (default 8).
//! - `VXE_STRIP_SHADOW`: the shadow walk, `coarse` (default) or `full`.
//! - `VXE_STRIP_MEAN`: `on` writes `mean.png` (and its crop), the run's frames averaged.
//! - `VXE_STRIP_EXPOSURE`: a number, or `auto` (default: the shot's or pose's manual value).
//! - `VXE_STRIP_SETTLE`: frames rendered still before the run (default 120).
//! - `VXE_STRIP_FRAMES`: the run's frames (default 240).
//! - `VXE_STRIP_EVERY`: a frame is written every N run frames, from frame 0 (default 24).
//! - `VXE_STRIP_SPEED`: forward speed in voxels a second (default 0).
//! - `VXE_STRIP_YAW`: yaw rate in degrees a second, positive to the left (default 0).
//! - `VXE_STRIP_LAMP_AT`: the run frame the lamp switches (default: never).
//! - `VXE_STRIP_CROP`: `x,y,w,h` of a crop written beside every frame (default: none).
//! - `VXE_STRIP_FLICKER`: `from,to` run frames the flicker map averages over (default: the
//!   whole run). The map is the mean per pixel of `|dY|` between consecutive frames, `Y` the
//!   display-encoded luma in 0..1, written times 8 as grey.
//! - `VXE_STRIP_LIGHT`: `hero` (the variables above), `eye` (`aliasing::eye_lamp` at
//!   `VXE_STRIP_LAMP_POWER`, default 4000 (the hall's dome crop at a mean display luma of 30 to 120 at exposure 8), with `aliasing::eye_lighting`: the lamp on, no
//!   indirect, no emissive candidates, the mirror's light radius 0) or `eye0` (`eye` with every emitting material's
//!   emission scaled to 0). Given at all, the probe spacing defaults to the hero's 32.
//! - `VXE_STRIP_LIGHT_RADIUS`: the mirror's `light_radius` (default: 0 under `eye`, else the
//!   source's). `VXE_STRIP_PROBE_SPACING`: the probe volume's spacing.
//! - `VXE_STRIP_SHARPEN`: the upscaler's sharpening (default 0.5).
//! - FSR 2's candidate knobs (`Fsr2Settings`): `VXE_STRIP_JITTER_PHASES` (the jitter's phase
//!   count; the fold's default follows it), `VXE_STRIP_EMITTER_CAP` (a pre-exposed level) and
//!   `VXE_STRIP_DITHER=on` (the tone pass's static one-level dither).
//! - `VXE_STRIP_READ=fold`: after the settle, `VXE_STRIP_FRAMES` still frames folded over
//!   `VXE_STRIP_PHASES` (default: `fsr2::jitter_phases`, `own`'s `JITTER_PHASES`, 18 for
//!   `off`): `T`, `L`, `J` (`support::aliasing`), the guard's energy, the jumps by class and
//!   footprint over the first cycle's mask, and `jumps.png` (T's jumps per pixel, white at `JUMPS_WHITE`
//!   jumps), `jumps-l.png`, `mask.png`, `last.png` and `clusters.png` (the 8 worst 32-pixel tiles).
//! - `VXE_STRIP_SCALE`: the upscalers' internal scale, `1`, `1.5` (default) or `2`.
//! - `VXE_STRIP_SHRINK=K`: with `off`, renders `K` times the size on each edge and box-filters
//!   every read frame down to the output size (`R` at `K` 2 and `VXE_STRIP_FAR=4,12`).
//! - `VXE_STRIP_REF`: an absolute path to a PNG of the output size; a fold prints its last
//!   frame's mean level distance from it.
//! - `VXE_STRIP_RINGS=x,y,w,h`: a fold reads the rings on that crop of its last frame
//!   (`aliasing::Rings`) and writes `zlevel.png`, `rings-zlevel.png` and `rings-crop.png`.
//! - `VXE_STRIP_PAN=PX_PER_FRAME`: after the settle, `VXE_STRIP_FRAMES` frames of a pure yaw to
//!   the left at that many display pixels a frame at the image centre, `C` read in the centre
//!   band against the frame `lag` before shifted `shift` pixels (`aliasing::lag_shift`), with its
//!   jumps by class; `VXE_STRIP_DOTS=on` adds the dot and yellow-dot count and their anchoring.
//! - `VXE_STRIP_READ=drift`: step `settle + 1` against step `VXE_STRIP_DRIFT_TO` (default 8192).
//!
//! Example (the RTX, under the perf lock and the memory cap):
//! `VXE_TEST_ADAPTER=rtx VXE_STRIP_TAG=lamp-off VXE_STRIP_SOURCE=level VXE_STRIP_LAMP_AT=60
//! cargo test --release -p flythrough --test strip -- --ignored --nocapture`.

mod support;

use std::path::PathBuf;

use flythrough::showcase::{SHOTS, ShowcaseSource};
use flythrough::{LookPreset, Pose};
use support::aliasing::{self, ByClass, Hits, Mask};
use support::{reading_device, write_png};
use vxe_render::gpu_world::{GpuWorld, MirrorConfig, UNIT_BYTES, Window};
use vxe_render::{
    AutoExposure, Camera, Exposure, Frame, Fsr2Settings, LightParams, Lighting, LookParams,
    RenderScale, UpscalerKind, readback, wgpu,
};
use vxe_world::*;

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn env_or<T: std::str::FromStr>(name: &str, default: T) -> T {
    env(name)
        .map(|v| {
            v.parse()
                .unwrap_or_else(|_| panic!("{name}={v} does not parse"))
        })
        .unwrap_or(default)
}

fn on_off(name: &str, default: bool) -> bool {
    match env(name).as_deref() {
        Some("on") => true,
        Some("off") => false,
        Some(v) => panic!("{name} takes on or off, not {v}"),
        None => default,
    }
}

/// A comma list of `N` numbers.
fn list<const N: usize>(name: &str) -> Option<[f32; N]> {
    let v = env(name)?;
    let parts: Vec<f32> = v
        .split(',')
        .map(|p| {
            p.trim()
                .parse()
                .unwrap_or_else(|_| panic!("{name}={v} does not parse"))
        })
        .collect();
    assert!(parts.len() == N, "{name} takes {N} numbers, not {v}");
    Some(std::array::from_fn(|i| parts[i]))
}

/// The world mirrored whole on the reading adapter, and the camera the run starts at.
struct Scene {
    device: wgpu::Device,
    queue: wgpu::Queue,
    gpu: GpuWorld,
    camera: Camera,
    exposure: f32,
    lamp: bool,
    name: String,
    /// The materials that emit.
    emits: Vec<u16>,
    /// The mirror's `light_radius`.
    light_radius: i32,
}

/// The hero's probe spacing (`examples/hero.rs`), the default under `VXE_STRIP_LIGHT`.
const HERO_PROBE_SPACING: u32 = 32;

fn emitting(table: &MaterialTable) -> Vec<u16> {
    (1..table.len() as u16)
        .filter(|&m| {
            table
                .get(m)
                .is_some_and(|r| r.emission.iter().any(|&e| e > 0.0))
        })
        .collect()
}

fn sync_whole(scene: &mut Scene, world: &World) {
    let mut last = None;
    for _ in 0..256 {
        let mut enc = scene
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        scene
            .gpu
            .sync(world, &scene.queue, &mut enc)
            .expect("the mirror fits");
        scene.queue.submit([enc.finish()]);
        scene
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("device poll after a sync");
        if last == Some(scene.gpu.light_samples()) {
            break;
        }
        last = Some(scene.gpu.light_samples());
    }
}

/// The mirror's light radius: `VXE_STRIP_LIGHT_RADIUS`, else 0 under the eye lamp, else the
/// source's `default`.
fn light_radius(default: i32) -> i32 {
    let eye = matches!(env("VXE_STRIP_LIGHT").as_deref(), Some("eye" | "eye0"));
    env_or("VXE_STRIP_LIGHT_RADIUS", if eye { 0 } else { default })
}

fn scene() -> Option<Scene> {
    let base = flythrough::with_emissive_ore(&flythrough::cave_materials());
    let source = env("VXE_STRIP_SOURCE").unwrap_or_else(|| "showcase".to_owned());
    if source == "level" {
        let source = flythrough::cave::CaveSource::new(
            1,
            ChunkExtent::new([0, 0, 0], flythrough::cave::LEVEL_CHUNKS),
        );
        let world = World::generate(source.extent, &source, &base);
        let pose = env("VXE_STRIP_SHOT").unwrap_or_else(|| "vein".to_owned());
        let pose = if pose == "spawn" {
            Pose {
                name: "spawn",
                position: source.spawn().0.map(|v| v as f32 + 0.5),
                yaw_deg: 0.0,
                pitch_deg: 0.0,
                exposure: 1.0,
            }
        } else {
            Pose::named(&pose).expect("VXE_STRIP_SHOT takes vein, patch or spawn on the level")
        };
        let pool = flythrough::level_pool_units(&world, false) * UNIT_BYTES;
        let (device, queue, adapter) = reading_device(pool)?;
        let base_cfg = MirrorConfig::new(Window::Fixed(world.extent()));
        let cfg = MirrorConfig {
            budget_bytes: flythrough::level_mirror_budget(
                &world,
                world.extent().len() as u64,
                false,
            ),
            light_radius: light_radius(base_cfg.light_radius),
            ..base_cfg
        };
        let radius = cfg.light_radius;
        let mut gpu = GpuWorld::with_config(&device, cfg);
        gpu.set_focus(VoxelCoord(pose.position.map(|v| v.floor() as i32)));
        let mut scene = Scene {
            device,
            queue,
            gpu,
            camera: pose.camera(),
            exposure: pose.exposure,
            lamp: true,
            name: format!("level at {} on {adapter}", pose.name),
            emits: emitting(&base),
            light_radius: radius,
        };
        sync_whole(&mut scene, &world);
        return Some(scene);
    }
    assert!(
        source == "showcase",
        "VXE_STRIP_SOURCE takes showcase or level"
    );
    let source = ShowcaseSource::new(1);
    let table = flythrough::with_showcase_rows(&base);
    let world = World::generate(source.extent(), &source, &table);
    let shot = env("VXE_STRIP_SHOT").unwrap_or_else(|| "hall".to_owned());
    let shot = SHOTS
        .iter()
        .find(|s| s.name == shot)
        .expect("VXE_STRIP_SHOT takes hall, chasm or crystals on the showcase");
    let pool = flythrough::level_pool_units(&world, false) * UNIT_BYTES;
    let (device, queue, adapter) = reading_device(pool)?;
    let cfg = MirrorConfig {
        budget_bytes: flythrough::level_mirror_budget(&world, world.extent().len() as u64, false),
        light_radius: light_radius(19),
        ..MirrorConfig::new(Window::Fixed(world.extent()))
    };
    let radius = cfg.light_radius;
    let mut gpu = GpuWorld::with_config(&device, cfg);
    gpu.set_focus(shot.eye);
    let camera = Camera {
        position: shot.eye.0.map(|v| v as f32 + 0.5).into(),
        yaw: (shot.yaw_deg as f32).to_radians(),
        pitch: (shot.pitch_deg as f32).to_radians(),
        vfov: 70f32.to_radians(),
    };
    let mut scene = Scene {
        device,
        queue,
        gpu,
        camera,
        exposure: shot.exposure_milli as f32 / 1000.0,
        lamp: false,
        name: format!("showcase at {} on {adapter}", shot.name),
        emits: emitting(&table),
        light_radius: radius,
    };
    sync_whole(&mut scene, &world);
    Some(scene)
}

/// The display-encoded luma of an rgba8 pixel, 0 to 1.
fn luma(p: &[u8]) -> f32 {
    (0.2126 * f32::from(p[0]) + 0.7152 * f32::from(p[1]) + 0.0722 * f32::from(p[2])) / 255.0
}

/// The mean `|dY|` between horizontal and between vertical neighbours in the `[x, y, w, h]`
/// crop of an rgba8 image of width `w`.
fn neighbour_energy(rgba: &[u8], w: u32, c: [u32; 4]) -> (f64, f64) {
    let y = |px: u32, py: u32| luma(&rgba[((py * w + px) * 4) as usize..]) as f64;
    let (mut h_sum, mut v_sum) = (0.0, 0.0);
    for py in c[1]..c[1] + c[3] {
        for px in c[0]..c[0] + c[2] {
            if px + 1 < c[0] + c[2] {
                h_sum += (y(px + 1, py) - y(px, py)).abs();
            }
            if py + 1 < c[1] + c[3] {
                v_sum += (y(px, py + 1) - y(px, py)).abs();
            }
        }
    }
    let (cw, ch) = (f64::from(c[2]), f64::from(c[3]));
    (h_sum / ((cw - 1.0) * ch), v_sum / (cw * (ch - 1.0)))
}

/// The `[x, y, w, h]` crop of an rgba8 image of width `w`.
fn crop(rgba: &[u8], w: u32, c: [u32; 4]) -> Vec<u8> {
    let mut out = Vec::with_capacity((c[2] * c[3] * 4) as usize);
    for y in c[1]..c[1] + c[3] {
        let s = ((y * w + c[0]) * 4) as usize;
        out.extend_from_slice(&rgba[s..s + (c[2] * 4) as usize]);
    }
    out
}

/// An rgba8 image box-filtered down by `k` on each edge.
fn shrink_image(rgba: &[u8], w: u32, h: u32, k: u32) -> (Vec<u8>, u32, u32) {
    let (ow, oh) = (w / k, h / k);
    let mut out = vec![0u8; (ow * oh * 4) as usize];
    for oy in 0..oh {
        for ox in 0..ow {
            let mut acc = [0u32; 4];
            for dy in 0..k {
                for dx in 0..k {
                    let s = (((oy * k + dy) * w + ox * k + dx) * 4) as usize;
                    for (a, &v) in acc.iter_mut().zip(&rgba[s..s + 4]) {
                        *a += u32::from(v);
                    }
                }
            }
            let d = ((oy * ow + ox) * 4) as usize;
            for (i, a) in acc.iter().enumerate() {
                out[d + i] = (a / (k * k)) as u8;
            }
        }
    }
    (out, ow, oh)
}

/// Panels of one size tiled `per_row` to a row, a 2-pixel gap between.
fn sheet(panels: &[Vec<u8>], w: u32, h: u32, per_row: u32) -> (Vec<u8>, u32, u32) {
    const GAP: u32 = 2;
    let rows = (panels.len() as u32).div_ceil(per_row).max(1);
    let (sw, sh) = (per_row * (w + GAP), rows * (h + GAP));
    let mut out = vec![0u8; (sw * sh * 4) as usize];
    for (i, p) in panels.iter().enumerate() {
        let (ox, oy) = (
            (i as u32 % per_row) * (w + GAP),
            (i as u32 / per_row) * (h + GAP),
        );
        for y in 0..h {
            let s = (y * w * 4) as usize;
            let d = (((oy + y) * sw + ox) * 4) as usize;
            out[d..d + (w * 4) as usize].copy_from_slice(&p[s..s + (w * 4) as usize]);
        }
    }
    (out, sw, sh)
}

fn out_dir(tag: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = root.canonicalize().unwrap_or(root);
    let dir = root.join("renders/strips").join(tag);
    std::fs::create_dir_all(&dir).expect("create the strip directory");
    dir
}

/// The guard's crops at 1920 x 1080: the dome, the near pillar, the far wall.
const GUARD_CROPS: [[u32; 4]; 3] = [
    [800, 0, 700, 250],
    [560, 300, 200, 400],
    [780, 230, 640, 360],
];

/// The neighbour energy on `VXE_STRIP_ENERGY`'s crops, else the guard's at 1920 x 1080, else
/// the whole image.
fn guard(rgba: &[u8], w: u32, h: u32) -> String {
    let crops: Vec<[u32; 4]> = match env("VXE_STRIP_ENERGY") {
        Some(spec) => spec
            .split('/')
            .map(|c| {
                let c: Vec<u32> = c.split(',').map(|v| v.parse().expect("a crop")).collect();
                [c[0], c[1], c[2], c[3]]
            })
            .collect(),
        None if (w, h) == (1920, 1080) => GUARD_CROPS.to_vec(),
        None => vec![[0, 0, w, h]],
    };
    crops
        .iter()
        .map(|c| {
            let (eh, ev) = neighbour_energy(rgba, w, *c);
            format!("{c:?} h {eh:.5} v {ev:.5}")
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// An rgba8 image magnified `k` times, nearest.
fn magnify(rgba: &[u8], w: u32, h: u32, k: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((w * h * k * k * 4) as usize);
    for y in 0..h * k {
        for x in 0..w * k {
            let s = (((y / k) * w + x / k) * 4) as usize;
            out.extend_from_slice(&rgba[s..s + 4]);
        }
    }
    out
}

/// The frame's internal hit buffer: positions with `t`, and records.
fn read_hits(scene: &Scene, frame: &Frame) -> (Vec<[f32; 4]>, Vec<[u16; 4]>) {
    (
        readback::read_float(&scene.device, &scene.queue, frame.hit_positions()),
        readback::read_rgba16u(&scene.device, &scene.queue, frame.hit_records()),
    )
}

/// The display image: the target, box-filtered down by `shrink` (`R`).
fn display(scene: &Scene, frame: &Frame, shrink: u32) -> Vec<u8> {
    let t = frame.target();
    let rgba = readback::read_rgba8(&scene.device, &scene.queue, t);
    if shrink > 1 {
        shrink_image(&rgba, t.width(), t.height(), shrink).0
    } else {
        rgba
    }
}

/// `VXE_STRIP_PAN=PX_PER_FRAME`: a pure yaw to the left from the settled still, `C` read in the
/// centre band each frame against the frame `lag` before shifted `shift` pixels.
struct PanRead<'a> {
    size: [u32; 2],
    shrink: u32,
    rate: f32,
    frames: u32,
    dots: bool,
    tag: &'a str,
    dir: &'a std::path::Path,
}

impl PanRead<'_> {
    fn run(&self, scene: &mut Scene, frame: &mut Frame, camera: &mut Camera, light: &LightParams) {
        use std::collections::VecDeque;
        let [w, h] = self.size;
        let internal = frame.internal_size();
        let tan_sum = 2.0 * (camera.vfov * 0.5).tan() * w as f32 / h as f32;
        let emits = scene.emits.clone();
        let is_emit = |m: u16| emits.contains(&m);
        let step = aliasing::yaw_per_pixel(h, camera.vfov, camera.pitch);
        let (lag, shift) = aliasing::lag_shift(self.rate);
        let cols = aliasing::band(w);
        println!(
            "pan {}: {} px a frame, {:.5} degrees a frame, {:.3} degrees a second at 60 frames a second; C lags {lag} frames, shift {shift} px, columns {}..{}; {} frames",
            self.tag,
            self.rate,
            (self.rate * step).to_degrees(),
            (self.rate * step).to_degrees() * 60.0,
            cols.start,
            cols.end,
            self.frames
        );
        let mut hist = aliasing::Hist::default();
        let mut edge = aliasing::Hist::default();
        let mut by = ByClass::default();
        let mut past: VecDeque<(Vec<u8>, Vec<bool>)> = VecDeque::new();
        // Dots: all, yellow, yellow world-anchored (a dot within a pixel of `x - shift` in the
        // frame `lag` before), yellow camera-anchored (a dot within a pixel of `x`).
        let mut dot = [0u64; 4];
        let mut dot_class = ByClass::default();
        let mut dot_frames = 0u64;
        let mut first_dots = None;
        let mut last = Vec::new();
        for _ in 0..self.frames {
            frame.step(&scene.device, &scene.queue, &scene.gpu, camera, light);
            let rgba = display(scene, frame, self.shrink);
            let (pos, info) = read_hits(scene, frame);
            let hits = Hits {
                size: internal,
                pos: &pos,
                info: &info,
            };
            let mask = Mask::of(&hits, [w, h], tan_sum, &is_emit);
            let d = if self.dots {
                aliasing::dots(&rgba, w, h)
            } else {
                Vec::new()
            };
            if self.dots && first_dots.is_none() {
                first_dots = Some(d.iter().filter(|&&v| v).count());
            }
            if past.len() == lag {
                let (prev, prev_dots) = &past[0];
                aliasing::crawl(
                    &rgba,
                    prev,
                    w,
                    shift,
                    cols.clone(),
                    &mut hist,
                    Some((&mask, &mut by, &mut edge)),
                );
                if self.dots {
                    dot_frames += 1;
                    for (i, _) in d.iter().enumerate().filter(|(_, v)| **v) {
                        dot[0] += 1;
                        if !aliasing::yellow(&rgba[i * 4..i * 4 + 4]) {
                            continue;
                        }
                        dot[1] += 1;
                        let (x, y) = ((i as u32 % w) as i32, (i as u32 / w) as i32);
                        dot_class.add(mask.class[i], mask.footprint[i], 1);
                        if aliasing::near(prev_dots, w, h, x - shift, y) {
                            dot[2] += 1;
                        }
                        if aliasing::near(prev_dots, w, h, x, y) {
                            dot[3] += 1;
                        }
                    }
                }
                past.pop_front();
            }
            past.push_back((rgba.clone(), d));
            last = rgba;
            camera.yaw += self.rate * step;
        }
        println!(
            "pan C (mean, p99.9, max, over 2, jumps over {}): {}",
            aliasing::JUMP,
            hist.change().cell()
        );
        println!("pan C on creases and silhouettes: {}", edge.change().cell());
        println!("pan C jumps by class:\n{}", by.table("C jumps"));
        println!("pan guard energy (last frame): {}", guard(&last, w, h));
        if self.dots {
            println!(
                "pan dots: still frame {}; over {dot_frames} frames {} dots ({:.2} a frame), {} yellow ({:.2} a frame), of the yellow {:.3} world-anchored, {:.3} camera-anchored",
                first_dots.unwrap_or(0),
                dot[0],
                dot[0] as f64 / dot_frames.max(1) as f64,
                dot[1],
                dot[1] as f64 / dot_frames.max(1) as f64,
                dot[2] as f64 / dot[1].max(1) as f64,
                dot[3] as f64 / dot[1].max(1) as f64
            );
            println!(
                "pan yellow dots by class:\n{}",
                dot_class.table("yellow dots")
            );
        }
        write_png(&self.dir.join("last.png"), &last, w, h);
        println!("pan: written under {}", self.dir.display());
    }
}

/// Jumps per pixel at which `jumps.png` is white.
const JUMPS_WHITE: u32 = 8;

/// Each frame of the last whole cycle against that cycle's per-pixel mean, rounded to a level:
/// how far one frame lies from the image a still view would hold if it showed the mean, which
/// is also the jump a pan's first frame would make from it. `None` without a whole cycle.
fn from_cycle_mean(frames: &[Vec<u8>], phases: u32) -> Option<aliasing::Change> {
    let p = phases.max(1) as usize;
    let cycles = frames.len() / p;
    if cycles == 0 {
        return None;
    }
    let cycle = &frames[(cycles - 1) * p..cycles * p];
    let len = cycle[0].len();
    let mean: Vec<u8> = (0..len)
        .map(|i| {
            let sum: u32 = cycle.iter().map(|f| u32::from(f[i])).sum();
            ((sum + p as u32 / 2) / p as u32) as u8
        })
        .collect();
    let mut hist = aliasing::Hist::default();
    for f in cycle {
        for d in aliasing::levels(f, &mean) {
            hist.add(d);
        }
    }
    Some(hist.change())
}

/// `VXE_STRIP_READ=fold`: a still camera's frames folded over the jitter's cycle.
struct FoldRead<'a> {
    size: [u32; 2],
    shrink: u32,
    phases: u32,
    frames: u32,
    tag: &'a str,
    dir: &'a std::path::Path,
}

impl FoldRead<'_> {
    fn run(&self, scene: &mut Scene, frame: &mut Frame, camera: &Camera, light: &LightParams) {
        let [w, h] = self.size;
        let internal = frame.internal_size();
        let tan_sum = 2.0 * (camera.vfov * 0.5).tan() * w as f32 / h as f32;
        let emits = scene.emits.clone();
        let is_emit = |m: u16| emits.contains(&m);
        let mut frames = Vec::with_capacity(self.frames as usize);
        let mut mask: Option<Mask> = None;
        for f in 0..self.frames {
            frame.step(&scene.device, &scene.queue, &scene.gpu, camera, light);
            frames.push(display(scene, frame, self.shrink));
            if f < self.phases.max(1) {
                let (pos, info) = read_hits(scene, frame);
                let hits = Hits {
                    size: internal,
                    pos: &pos,
                    info: &info,
                };
                let m = Mask::of(&hits, [w, h], tan_sum, &is_emit);
                match &mut mask {
                    Some(a) => a.merge(&m),
                    None => mask = Some(m),
                }
            }
        }
        let fold = aliasing::fold(&frames, self.phases);
        let mask = mask.expect("a fold reads at least one frame");
        let last = frames.last().expect("a frame");
        println!(
            "fold {}: {} frames, {} phases, {} whole cycles",
            self.tag,
            frames.len(),
            fold.phases,
            fold.cycles
        );
        println!(
            "fold T (mean, p99.9, max, over 2, jumps over {}): {}",
            aliasing::JUMP,
            fold.t.cell()
        );
        println!("fold L: {}", fold.l.cell());
        println!(
            "fold J {:.4}; unlocked share of jumps (L / T) {:.4}",
            fold.j,
            fold.unlocked()
        );
        println!("fold guard energy: {}", guard(last, w, h));
        if let Some(c) = from_cycle_mean(&frames, self.phases) {
            println!(
                "fold one frame from its cycle's mean (mean, p99.9, max, over 2, jumps): {}",
                c.cell()
            );
        }
        if (w, h) == (1920, 1080) {
            let dome = crop(last, w, GUARD_CROPS[0]);
            let n = (dome.len() / 4) as f32;
            println!(
                "fold dome crop mean display luma {:.1} /255",
                dome.chunks_exact(4).map(luma).sum::<f32>() / n * 255.0
            );
        }
        let ones = vec![1u32; mask.class.len()];
        println!(
            "fold pixels by class:\n{}",
            ByClass::of(&mask, &ones).table("pixels")
        );
        println!(
            "fold T jumps by class:\n{}",
            ByClass::of(&mask, &fold.t_jumps).table("T jumps")
        );
        println!(
            "fold L jumps by class:\n{}",
            ByClass::of(&mask, &fold.l_jumps).table("L jumps")
        );
        write_png(
            &self.dir.join("jumps.png"),
            &aliasing::count_image(&fold.t_jumps, JUMPS_WHITE),
            w,
            h,
        );
        write_png(
            &self.dir.join("jumps-l.png"),
            &aliasing::count_image(&fold.l_jumps, JUMPS_WHITE),
            w,
            h,
        );
        let mask_image = mask.image();
        write_png(&self.dir.join("mask.png"), &mask_image, w, h);
        write_png(&self.dir.join("last.png"), last, w, h);
        // The worst clusters: 32-pixel tiles by T jumps, a 96-pixel crop around each, magnified
        // twice, beside the same crop of the mask.
        const TILE: u32 = 32;
        const CROP: u32 = 96;
        let mut panels = Vec::new();
        for (k, (tx, ty, sum)) in aliasing::worst_tiles(&fold.t_jumps, [w, h], TILE, 8)
            .into_iter()
            .enumerate()
        {
            let cx = (tx + TILE / 2)
                .saturating_sub(CROP / 2)
                .min(w.saturating_sub(CROP));
            let cy = (ty + TILE / 2)
                .saturating_sub(CROP / 2)
                .min(h.saturating_sub(CROP));
            let c = [cx, cy, CROP.min(w), CROP.min(h)];
            let mut mix = ByClass::default();
            for y in ty..(ty + TILE).min(h) {
                for x in tx..(tx + TILE).min(w) {
                    let i = (y * w + x) as usize;
                    mix.add(mask.class[i], mask.footprint[i], u64::from(fold.t_jumps[i]));
                }
            }
            let words: Vec<String> = aliasing::Class::ALL
                .iter()
                .filter(|&&cl| mix.class_sum(cl) > 0)
                .map(|&cl| format!("{} {}", cl.name(), mix.class_sum(cl)))
                .collect();
            println!(
                "fold cluster {k}: tile at ({tx}, {ty}), {sum} T jumps; crop {c:?}; {}",
                words.join(", ")
            );
            panels.push(magnify(&crop(last, w, c), c[2], c[3], 2));
            panels.push(magnify(&crop(&mask_image, w, c), c[2], c[3], 2));
        }
        if !panels.is_empty() {
            let side = CROP.min(w).min(h) * 2;
            let (px, pw, ph) = sheet(&panels, side, side, 4);
            write_png(&self.dir.join("clusters.png"), &px, pw, ph);
        }
        if let Some(path) = env("VXE_STRIP_REF") {
            let (r, rw, rh) = support::read_png(&PathBuf::from(&path));
            assert_eq!((rw, rh), (w, h), "VXE_STRIP_REF is {rw}x{rh}");
            let d = aliasing::levels(last, &r);
            println!(
                "fold distance from {path}: mean {:.3} levels, over 8 {:.2} %",
                d.iter().map(|&v| f64::from(v)).sum::<f64>() / d.len() as f64,
                d.iter().filter(|&&v| v > 8).count() as f64 / d.len() as f64 * 100.0
            );
        }
        if let Some(c) = list::<4>("VXE_STRIP_RINGS").map(|c| c.map(|v| v as u32)) {
            let (pos, info) = read_hits(scene, frame);
            let hits = Hits {
                size: internal,
                pos: &pos,
                info: &info,
            };
            let z = aliasing::z_levels(&hits, [w, h]);
            let zi = aliasing::z_image(&z);
            write_png(&self.dir.join("zlevel.png"), &zi, w, h);
            write_png(
                &self.dir.join("rings-zlevel.png"),
                &crop(&zi, w, c),
                c[2],
                c[3],
            );
            write_png(
                &self.dir.join("rings-crop.png"),
                &crop(last, w, c),
                c[2],
                c[3],
            );
            print!(
                "fold {c:?} {}",
                aliasing::Rings::read(last, &z, w, c).lines()
            );
        }
        println!("fold: written under {}", self.dir.display());
    }
}

#[test]
#[ignore = "reading only: run with --ignored"]
fn strip_run() {
    let look = env("VXE_STRIP_LOOK");
    let mut preset = look.as_deref().map_or(LookPreset::DEFAULT, |name| {
        LookPreset::from_name(name).expect("VXE_STRIP_LOOK takes a look preset")
    });
    let look_source = if look.is_some() { "" } else { " (default)" };
    if let Some(f) = list::<2>("VXE_STRIP_FAR") {
        preset.far_normal = f;
    }
    let Some(mut scene) = scene() else {
        println!("strip: no adapter; skipped");
        return;
    };
    let (device, queue) = (&scene.device, &scene.queue);
    if let Some(c) = list::<5>("VXE_STRIP_CAMERA") {
        scene.camera = Camera {
            position: [c[0], c[1], c[2]].into(),
            yaw: c[3].to_radians(),
            pitch: c[4].to_radians(),
            vfov: 70f32.to_radians(),
        };
    }
    let view = env("VXE_STRIP_VIEW").unwrap_or_else(|| "shaded".to_owned());
    let view = vxe_render::ViewMode::from_name(&view)
        .expect("VXE_STRIP_VIEW takes shaded, normals, albedo, hitmask or steps");

    let size = env("VXE_STRIP_SIZE").unwrap_or_else(|| "1920x1080".to_owned());
    let (w, h) = size
        .split_once('x')
        .and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))
        .expect("VXE_STRIP_SIZE takes WxH");
    let output = [w, h];
    let upscaler = env("VXE_STRIP_UPSCALER").unwrap_or_else(|| "fsr2".to_owned());
    let sharpen: f32 = env_or("VXE_STRIP_SHARPEN", 0.5);
    let scale = env("VXE_STRIP_SCALE").map_or(RenderScale::OneHalf, |v| {
        RenderScale::ALL
            .into_iter()
            .find(|s| s.name() == v)
            .unwrap_or_else(|| panic!("VXE_STRIP_SCALE takes 1, 1.5 or 2, not {v}"))
    });
    let shrink: u32 = env_or("VXE_STRIP_SHRINK", 1);
    assert!(
        shrink == 1 || upscaler == "off",
        "VXE_STRIP_SHRINK renders the native frame larger: VXE_STRIP_UPSCALER=off"
    );
    let mut frame = match upscaler.as_str() {
        "off" => Frame::new(device, w * shrink, h * shrink),
        "own" => {
            let mut f = Frame::with_sizes(device, scale.internal(output), output);
            f.set_jitter(device, true);
            f.set_sharpness(queue, sharpen);
            f
        }
        "fsr2" => {
            let mut f = Frame::with_sizes(device, scale.internal(output), output);
            f.set_upscaler_kind(
                device,
                UpscalerKind::Fsr2(Fsr2Settings {
                    sharpness: sharpen,
                    jitter_phases: env("VXE_STRIP_JITTER_PHASES").map(|v| {
                        v.parse()
                            .expect("VXE_STRIP_JITTER_PHASES takes a phase count")
                    }),
                    emitter_cap: env("VXE_STRIP_EMITTER_CAP").map(|v| {
                        v.parse()
                            .expect("VXE_STRIP_EMITTER_CAP takes a pre-exposed level")
                    }),
                    dither: on_off("VXE_STRIP_DITHER", false),
                    ..Fsr2Settings::default()
                }),
            );
            f
        }
        v => panic!("VXE_STRIP_UPSCALER takes fsr2, own or off, not {v}"),
    };
    let internal = frame.internal_size();
    let phases = match upscaler.as_str() {
        "fsr2" => frame
            .fsr2()
            .map_or(0, |f| f.settings().phases(internal[0], w)),
        "own" => vxe_render::marcher::JITTER_PHASES,
        _ => 18,
    };
    let phases: u32 = env_or("VXE_STRIP_PHASES", phases);
    let exposure = match env("VXE_STRIP_EXPOSURE").as_deref() {
        Some("auto") => Exposure::Auto(AutoExposure::eye()),
        Some(v) => Exposure::Manual(
            v.parse()
                .expect("VXE_STRIP_EXPOSURE takes a number or auto"),
        ),
        None => Exposure::Manual(scene.exposure),
    };
    let mut lighting = Lighting {
        lamp: on_off("VXE_STRIP_LAMP", scene.lamp),
        lamp_bounce: on_off("VXE_STRIP_INDIRECT", true),
        filter: on_off("VXE_STRIP_FILTER", true),
        candidates: env_or("VXE_STRIP_CANDIDATES", 8),
        shadow_walk: match env("VXE_STRIP_SHADOW").as_deref() {
            Some("full") => vxe_render::ShadowWalk::Full,
            Some("coarse") | None => vxe_render::ShadowWalk::Coarse,
            Some(v) => panic!("VXE_STRIP_SHADOW takes coarse or full, not {v}"),
        },
        exposure,
        ..Lighting::default()
    };
    let light_mode = env("VXE_STRIP_LIGHT");
    for gone in ["VXE_STRIP_SAMPLER", "VXE_STRIP_STEADY"] {
        assert!(
            env(gone).is_none(),
            "{gone} is gone: the emissive table is the only emissive store (ADR-0043)"
        );
    }
    let lamp_power: f32 = env_or("VXE_STRIP_LAMP_POWER", 4000.0);
    let light = match light_mode.as_deref() {
        None | Some("hero") => LightParams::headlamp(),
        Some("eye" | "eye0") => {
            lighting = aliasing::eye_lighting(lighting);
            aliasing::eye_lamp(lamp_power)
        }
        Some(v) => panic!("VXE_STRIP_LIGHT takes hero, eye or eye0, not {v}"),
    };
    if light_mode.as_deref() == Some("eye0") {
        for &m in &scene.emits {
            frame.set_emission_scale(queue, m, 0.0);
        }
    }
    let spacing: u32 = env_or(
        "VXE_STRIP_PROBE_SPACING",
        if light_mode.is_some() {
            HERO_PROBE_SPACING
        } else {
            vxe_render::probes::DEFAULT_PROBE_SPACING
        },
    );
    frame.set_probe_spacing(device, spacing);
    let mean_image = on_off("VXE_STRIP_MEAN", false);
    frame.set_lighting(lighting);
    frame.set_look(LookParams {
        normal_mode: preset.normal_mode,
        view,
        tuning: preset.tuning,
        far_normal: preset.far_normal,
        ..LookParams::default()
    });
    const DT: f32 = 1.0 / 60.0;
    frame.set_dt(DT);

    let settle: u32 = env_or("VXE_STRIP_SETTLE", 120);
    let frames: u32 = env_or("VXE_STRIP_FRAMES", 240);
    let every: u32 = env_or("VXE_STRIP_EVERY", 24).max(1);
    let speed: f32 = env_or("VXE_STRIP_SPEED", 0.0);
    let yaw_rate: f32 = env_or("VXE_STRIP_YAW", 0.0);
    let lamp_at: Option<u32> = env("VXE_STRIP_LAMP_AT").map(|v| v.parse().expect("a frame"));
    let crop_box = list::<4>("VXE_STRIP_CROP").map(|c| c.map(|v| v as u32));
    let flicker_window =
        list::<2>("VXE_STRIP_FLICKER").map_or((1, frames), |f| (f[0] as u32, f[1] as u32));
    let tag = env("VXE_STRIP_TAG").unwrap_or_else(|| "strip".to_owned());
    let dir = out_dir(&tag);
    println!(
        "strip {tag}: {} | {w}x{h} {upscaler} | look {} {}{look_source} | lamp {} indirect {} filter {} candidates {} exposure {:?} | settle {settle} frames {frames} every {every} speed {speed} yaw {yaw_rate} lamp_at {lamp_at:?} crop {crop_box:?} flicker {flicker_window:?}",
        scene.name,
        preset.name,
        LookPreset::far_normal_words(preset.normal_mode, preset.far_normal),
        lighting.lamp,
        lighting.lamp_bounce,
        lighting.filter,
        lighting.candidates,
        lighting.exposure
    );
    println!(
        "strip {tag}: light {} (lamp power {}, light radius {}, light samples {}), probe spacing {spacing}, sharpen {sharpen}, internal {}x{}, jitter phases {phases}",
        light_mode.as_deref().unwrap_or("hero"),
        light.intensity,
        scene.light_radius,
        scene.gpu.light_samples(),
        internal[0],
        internal[1]
    );

    let mut camera = scene.camera;
    for _ in 0..settle {
        frame.step(device, queue, &scene.gpu, &camera, &light);
    }
    if let Some(rate) = env("VXE_STRIP_PAN") {
        let read = PanRead {
            size: [w, h],
            shrink,
            rate: rate.parse().expect("VXE_STRIP_PAN takes pixels a frame"),
            frames,
            dots: on_off("VXE_STRIP_DOTS", false),
            tag: &tag,
            dir: &dir,
        };
        read.run(&mut scene, &mut frame, &mut camera, &light);
        return;
    }
    if env("VXE_STRIP_READ").as_deref() == Some("drift") {
        // Step `settle + 1` against step `VXE_STRIP_DRIFT_TO`.
        frame.step(device, queue, &scene.gpu, &camera, &light);
        let a = display(&scene, &frame, shrink);
        let to: u32 = env_or("VXE_STRIP_DRIFT_TO", 8192);
        for _ in settle + 1..to {
            frame.step(device, queue, &scene.gpu, &camera, &light);
        }
        let b = display(&scene, &frame, shrink);
        let mut hist = aliasing::Hist::default();
        aliasing::levels(&a, &b)
            .into_iter()
            .for_each(|d| hist.add(d));
        let c = hist.change();
        let over_8 =
            aliasing::levels(&a, &b).iter().filter(|&&d| d > 8).count() as f64 / (w * h) as f64;
        println!(
            "drift step {} vs {to}: {}; over 8 {:.2} %",
            settle + 1,
            c.cell(),
            over_8 * 100.0
        );
        return;
    }
    if env("VXE_STRIP_READ").as_deref() == Some("fold") {
        let read = FoldRead {
            size: [w, h],
            shrink,
            phases,
            frames,
            tag: &tag,
            dir: &dir,
        };
        read.run(&mut scene, &mut frame, &camera, &light);
        return;
    }

    let n = (w * h) as usize;
    let mut prev: Option<Vec<f32>> = None;
    let mut flick = vec![0f32; n];
    let mut flick_frames = 0u32;
    let mut panels = Vec::new();
    let mut frame_flicker = Vec::new();
    let mut mean_acc = vec![0f32; n * 4];
    for f in 0..frames {
        if lamp_at == Some(f) {
            lighting.lamp = !lighting.lamp;
            frame.set_lighting(lighting);
            println!(
                "frame {f}: lamp {}",
                if lighting.lamp { "on" } else { "off" }
            );
        }
        frame.step(device, queue, &scene.gpu, &camera, &light);
        let rgba = readback::read_rgba8(device, queue, frame.target());
        let y: Vec<f32> = rgba.chunks_exact(4).map(luma).collect();
        let mean = y.iter().sum::<f32>() / n as f32;
        let mut d_mean = 0.0;
        if let Some(p) = &prev {
            let mut d_sum = 0.0;
            for (i, (a, b)) in y.iter().zip(p).enumerate() {
                let d = (a - b).abs();
                d_sum += d;
                if (flicker_window.0..=flicker_window.1).contains(&f) {
                    flick[i] += d;
                }
            }
            d_mean = d_sum / n as f32;
            if (flicker_window.0..=flicker_window.1).contains(&f) {
                flick_frames += 1;
            }
        }
        frame_flicker.push(d_mean);
        if mean_image {
            for (a, &v) in mean_acc.iter_mut().zip(&rgba) {
                *a += f32::from(v);
            }
        }
        println!(
            "frame {f:4}: mean luma {mean:.4}, mean |dY| {d_mean:.5}, camera {:.1?} yaw {:.1}",
            camera.position.to_array(),
            camera.yaw.to_degrees()
        );
        if f + 1 == frames
            && let Some(spec) = env("VXE_STRIP_ENERGY")
        {
            for c in spec.split('/') {
                let c: Vec<u32> = c.split(',').map(|v| v.parse().expect("a crop")).collect();
                let c = [c[0], c[1], c[2], c[3]];
                let (eh, ev) = neighbour_energy(&rgba, w, c);
                println!("energy {c:?}: h {eh:.5} v {ev:.5}");
            }
        }
        if f % every == 0 || f + 1 == frames {
            write_png(&dir.join(format!("f{f:04}.png")), &rgba, w, h);
            if let Some(c) = crop_box {
                write_png(
                    &dir.join(format!("f{f:04}-crop.png")),
                    &crop(&rgba, w, c),
                    c[2],
                    c[3],
                );
            }
            panels.push(shrink_image(&rgba, w, h, 4).0);
        }
        prev = Some(y);
        let (fwd, _, _) = camera.basis();
        camera.position += fwd * speed * DT;
        camera.yaw += yaw_rate.to_radians() * DT;
        if speed != 0.0 {
            scene.gpu.set_focus(VoxelCoord(
                camera.position.to_array().map(|v| v.floor() as i32),
            ));
        }
    }
    let (sw, sh) = (w / 4, h / 4);
    let (px, pw, ph) = sheet(&panels, sw, sh, 4);
    write_png(&dir.join("sheet.png"), &px, pw, ph);
    if mean_image && frames > 0 {
        let mean: Vec<u8> = mean_acc
            .iter()
            .map(|a| (a / frames as f32).round().clamp(0.0, 255.0) as u8)
            .collect();
        write_png(&dir.join("mean.png"), &mean, w, h);
        if let Some(c) = crop_box {
            write_png(&dir.join("mean-crop.png"), &crop(&mean, w, c), c[2], c[3]);
        }
    }
    if flick_frames > 0 {
        let map: Vec<u8> = flick
            .iter()
            .flat_map(|&d| {
                let g = (d / flick_frames as f32 * 8.0 * 255.0).min(255.0) as u8;
                [g, g, g, 255]
            })
            .collect();
        write_png(&dir.join("flicker.png"), &map, w, h);
        if let Some(c) = crop_box {
            write_png(&dir.join("flicker-crop.png"), &crop(&map, w, c), c[2], c[3]);
        }
        let mut sorted: Vec<f32> = flick.iter().map(|d| d / flick_frames as f32).collect();
        sorted.sort_by(f32::total_cmp);
        let at = |p: f64| sorted[((sorted.len() - 1) as f64 * p) as usize];
        println!(
            "flicker over run frames {}..={} ({flick_frames} pairs): per-pixel mean |dY| median {:.5}, p95 {:.5}, p99 {:.5}, max {:.5}",
            flicker_window.0,
            flicker_window.1,
            at(0.5),
            at(0.95),
            at(0.99),
            sorted[sorted.len() - 1]
        );
    }
    let series: Vec<String> = frame_flicker
        .iter()
        .enumerate()
        .step_by(8)
        .map(|(f, d)| format!("{f}:{d:.4}"))
        .collect();
    println!("frame |dY| every 8: {}", series.join(" "));
    println!(
        "strip: {} frames written under {}",
        panels.len(),
        dir.display()
    );
}

/// 72 frames of 10 000 pixels (100 x 100) at 18 phases.
const FOLD_SIDE: usize = 100;
const FOLD_FRAMES: usize = 72;
const FOLD_PHASES: u32 = 18;

fn series(value: impl Fn(usize, usize) -> u8) -> Vec<Vec<u8>> {
    (0..FOLD_FRAMES)
        .map(|f| {
            (0..FOLD_SIDE * FOLD_SIDE)
                .flat_map(|i| {
                    let v = value(f, i);
                    [v, v.wrapping_add(17), v.wrapping_mul(3), 255]
                })
                .collect()
        })
        .collect()
}

#[test]
fn the_phase_fold_splits_a_locked_series_from_noise() {
    // A series of period 18: each pixel takes a seeded value per phase.
    let mut mix = aliasing::Mix(56);
    let table: Vec<u8> = (0..FOLD_PHASES as usize * FOLD_SIDE * FOLD_SIDE)
        .map(|_| mix.byte())
        .collect();
    let locked = aliasing::fold(
        &series(|f, i| table[(f % FOLD_PHASES as usize) * FOLD_SIDE * FOLD_SIDE + i]),
        FOLD_PHASES,
    );
    println!(
        "locked: T {} | L {} | J {:.6}",
        locked.t.cell(),
        locked.l.cell(),
        locked.j
    );
    assert_eq!(locked.cycles, 4);
    assert!(
        locked.t.mean > 0.0 && locked.t.jumps > 0,
        "T sees the cycle"
    );
    assert_eq!(locked.l.max, 0, "L sees nothing of a locked series");
    assert!(
        (locked.j - 1.0).abs() < 1e-9,
        "J of a locked series: {}",
        locked.j
    );

    // Seeded white noise.
    let mut mix = aliasing::Mix(0x0056);
    let noise: Vec<u8> = (0..FOLD_FRAMES * FOLD_SIDE * FOLD_SIDE)
        .map(|_| mix.byte())
        .collect();
    let noisy = aliasing::fold(
        &series(|f, i| noise[f * FOLD_SIDE * FOLD_SIDE + i]),
        FOLD_PHASES,
    );
    println!(
        "noise: T {} | L {} | J {:.6}",
        noisy.t.cell(),
        noisy.l.cell(),
        noisy.j
    );
    assert!(noisy.j.abs() <= 0.1, "J of noise: {}", noisy.j);
    assert!(
        (noisy.l.mean - noisy.t.mean).abs() <= 0.2 * noisy.t.mean,
        "L's mean {} against T's {}",
        noisy.l.mean,
        noisy.t.mean
    );

    // A constant series.
    let still = aliasing::fold(&series(|_, i| (i % 251) as u8), FOLD_PHASES);
    let zero = aliasing::Change {
        samples: still.t.samples,
        ..aliasing::Change::default()
    };
    assert_eq!(still.t, zero);
    assert_eq!(
        still.l,
        aliasing::Change {
            samples: still.l.samples,
            ..zero
        }
    );
    assert_eq!(still.j, 0.0);
    assert!(still.t_jumps.iter().chain(&still.l_jumps).all(|&c| c == 0));
}

/// A hit on the `axis` face (0 x, 1 y, 2 z) at `p`, of material `m`.
fn hit(p: [f32; 3], axis: u16, m: u16) -> ([f32; 4], [u16; 4]) {
    let t = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
    ([p[0], p[1], p[2], t], [0, 0, m, 1 | (axis + 1) << 3])
}

const MISS: ([f32; 4], [u16; 4]) = ([0.0, 0.0, 0.0, 3e38], [0, 0, 0, 0]);

/// The class of the centre of a 3 x 3 grid whose centre is a hit on the z = 10 plane at
/// (5.5, 7.5) and whose right neighbour is `right`; the other neighbours share the centre's
/// face.
fn centre_class(right: ([f32; 4], [u16; 4])) -> (aliasing::Class, Mask) {
    let centre = hit([5.5, 7.5, 10.0], 2, 3);
    let near = hit([5.2, 7.2, 10.0], 2, 3);
    let mut grid = vec![near; 9];
    grid[4] = centre;
    grid[5] = right;
    let pos: Vec<[f32; 4]> = grid.iter().map(|g| g.0).collect();
    let info: Vec<[u16; 4]> = grid.iter().map(|g| g.1).collect();
    let hits = Hits {
        size: [3, 3],
        pos: &pos,
        info: &info,
    };
    let emits = |m: u16| m == 9;
    let mask = Mask::of(&hits, [3, 3], 2.0, &emits);
    (hits.class(1, 1, &emits), mask)
}

#[test]
fn the_class_mask_names_each_class() {
    use aliasing::Class;
    let cases = [
        ("interior", hit([5.9, 7.1, 10.0], 2, 3), Class::Interior),
        ("seam", hit([6.2, 7.5, 10.0], 2, 3), Class::Seam),
        ("crease", hit([6.0, 7.5, 10.6], 0, 3), Class::Crease),
        ("silhouette", hit([6.4, 7.5, 18.0], 2, 3), Class::Silhouette),
        ("silhouette by a miss", MISS, Class::Silhouette),
        ("emitter", hit([6.2, 7.5, 10.0], 2, 9), Class::Emitter),
    ];
    for (name, right, want) in cases {
        let (got, mask) = centre_class(right);
        assert_eq!(got, want, "{name}");
        assert_eq!(
            mask.class[4], want,
            "{name}: the display pixel over the centre"
        );
    }
    let pos = vec![MISS.0; 9];
    let info = vec![MISS.1; 9];
    let all_miss = Hits {
        size: [3, 3],
        pos: &pos,
        info: &info,
    };
    assert_eq!(all_miss.class(1, 1, &|_| false), Class::Miss);
    // Inside a cycle the pixel was interior, then a silhouette: it takes the silhouette.
    let (_, mut cycle) = centre_class(cases[0].1);
    let (_, edge) = centre_class(cases[3].1);
    let (_, back) = centre_class(cases[0].1);
    cycle.merge(&edge);
    cycle.merge(&back);
    assert_eq!(cycle.class[4], Class::Silhouette);
}

/// The first frame of the eye lamp's native run from which every frame to frame 128 is
/// bit-identical, read on lavapipe (`cavity_world`, 192 x 108, `off`): frame 4.
const EYE_STILL_FROM: usize = 4;

#[test]
fn the_eye_lamp_holds_the_native_frame_bit_identical() {
    let world = support::cavity_world();
    let full = world.stats().full_bricks as u64;
    let Some((device, queue)) = support::software_device(((full + 4096) * 512).max(64 << 20))
    else {
        return;
    };
    let cfg = MirrorConfig {
        light_radius: 0,
        ..MirrorConfig::new(Window::Fixed(world.extent()))
    };
    let mut gpu = GpuWorld::with_config(&device, cfg);
    support::SyncNow::sync_now(&mut gpu, &world, &device, &queue).expect("the mirror fits");
    let mut frame = Frame::new(&device, 192, 108);
    frame.set_lighting(aliasing::eye_lighting(Lighting {
        exposure: Exposure::Manual(1.0),
        ..Lighting::default()
    }));
    frame.set_dt(1.0 / 60.0);
    let light = aliasing::eye_lamp(1200.0);
    let camera = support::cavity_camera();
    let mut frames = Vec::new();
    for _ in 0..=128 {
        frame.step(&device, &queue, &gpu, &camera, &light);
        frames.push(readback::read_rgba8(&device, &queue, frame.target()));
    }
    let last = frames.last().expect("frames");
    let from = (0..frames.len())
        .rev()
        .take_while(|&f| frames[f] == *last)
        .last()
        .expect("the last frame equals itself");
    let lit = last.chunks_exact(4).filter(|p| p[0] > 8).count();
    println!(
        "eye lamp, native: every frame from {from} to 128 is bit-identical; {lit} of {} pixels over 8",
        last.len() / 4
    );
    assert!(lit > last.len() / 8, "the lamp lights the cavity");
    assert!(
        from <= EYE_STILL_FROM,
        "the frame changes until {from}, past {EYE_STILL_FROM}"
    );
}

#[test]
fn the_pan_moves_the_centre_band_one_pixel() {
    let world = support::pillar_world();
    let Some((device, queue)) = support::software_device(64 << 20) else {
        return;
    };
    let cfg = MirrorConfig {
        light_radius: 0,
        ..MirrorConfig::new(Window::Fixed(world.extent()))
    };
    let mut gpu = GpuWorld::with_config(&device, cfg);
    support::SyncNow::sync_now(&mut gpu, &world, &device, &queue).expect("the mirror fits");
    let (w, h) = (support::WIDTH, support::HEIGHT);
    let mut frame = Frame::new(&device, w, h);
    frame.set_lighting(aliasing::eye_lighting(Lighting {
        exposure: Exposure::Manual(1.0),
        ..Lighting::default()
    }));
    frame.set_dt(1.0 / 60.0);
    let light = aliasing::eye_lamp(1200.0);
    let mut camera = support::pillar_camera();
    let step = aliasing::yaw_per_pixel(h, camera.vfov, camera.pitch);
    let (lag, shift) = aliasing::lag_shift(1.0);
    assert_eq!((lag, shift), (1, 1));
    let cols = aliasing::band(w);
    let mut frames = Vec::new();
    for _ in 0..8 {
        frame.step(&device, &queue, &gpu, &camera, &light);
        frames.push(readback::read_rgba8(&device, &queue, frame.target()));
        camera.yaw += step;
    }
    let residual: Vec<(i32, f64)> = (-2..=2)
        .map(|s| {
            let mut hist = aliasing::Hist::default();
            for p in frames.windows(2) {
                aliasing::crawl(&p[1], &p[0], w, s, cols.clone(), &mut hist, None);
            }
            (s, hist.change().mean)
        })
        .collect();
    println!("the band's mean residual by shift: {residual:?}");
    let best = residual
        .iter()
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .expect("five shifts");
    assert_eq!(best.0, shift, "the residual is least at the rate's shift");
    let unshifted = residual.iter().find(|r| r.0 == 0).expect("shift 0").1;
    assert!(
        best.1 < unshifted,
        "the shifted residual {} is under the unshifted {unshifted}",
        best.1
    );
}

/// R1's CPU half: the hall floor's terraces over the visible floor, x and y in 192..704 and
/// 192..576 (whole 64-voxel cells), from the showcase's floor rule (`FLOOR_Z` plus the seed-1
/// floor noise at cell 64, shifted down 11): terrace widths along rows and columns, the riser
/// heights, and the lattice corners that are extrema of their 8 neighbours with the floor on a
/// ring of radius 32 around them all below (a maximum) or all above (a minimum), so that every
/// level between closes around the corner.
#[test]
#[ignore = "reading only: run with --ignored"]
fn hall_floor_terraces() {
    use flythrough::showcase::FLOOR_Z;
    let noise = vxe_world::hash::ValueNoise::new(1 ^ 0x5A0C_A5E0_0000_0002, 6);
    let floor = |x: i32, y: i32| FLOOR_Z + (noise.sample([x, y, 0]) >> 11);
    let (x0, x1, y0, y1) = (192, 704, 192, 576);
    let mut runs = (0u64, 0u64);
    let mut steps = [0u64; 4];
    for y in y0..y1 {
        let mut run = 1u64;
        for x in x0 + 1..x1 {
            let d = (floor(x, y) - floor(x - 1, y)).unsigned_abs();
            if d == 0 {
                run += 1;
            } else {
                runs.0 += run;
                runs.1 += 1;
                run = 1;
                steps[(d as usize).min(3)] += 1;
            }
        }
    }
    let mut cruns = (0u64, 0u64);
    for x in x0..x1 {
        let mut run = 1u64;
        for y in y0 + 1..y1 {
            if floor(x, y) == floor(x, y - 1) {
                run += 1;
            } else {
                cruns.0 += run;
                cruns.1 += 1;
                run = 1;
            }
        }
    }
    let mut corner_diff = (0i64, 0u64);
    let mut extrema = Vec::new();
    for cy in (y0..=y1).step_by(64) {
        for cx in (x0..=x1).step_by(64) {
            let c = floor(cx, cy);
            let ns: Vec<i64> = [(-64, 0), (64, 0), (0, -64), (0, 64)]
                .iter()
                .map(|&(dx, dy)| floor(cx + dx, cy + dy))
                .collect();
            for &n in &ns[..2] {
                corner_diff.0 += (n - c).abs();
                corner_diff.1 += 1;
            }
            let all: Vec<i64> = (-1..=1)
                .flat_map(|j| (-1..=1).map(move |i| (i, j)))
                .filter(|&(i, j)| (i, j) != (0, 0))
                .map(|(i, j)| floor(cx + 64 * i, cy + 64 * j))
                .collect();
            let ring: Vec<i64> = (0..64)
                .map(|k| {
                    let a = k as f32 / 64.0 * std::f32::consts::TAU;
                    floor(
                        cx + (32.0 * a.cos()).round() as i32,
                        cy + (32.0 * a.sin()).round() as i32,
                    )
                })
                .collect();
            let max = all.iter().all(|&n| n < c) && ring.iter().all(|&n| n < c);
            let min = all.iter().all(|&n| n > c) && ring.iter().all(|&n| n > c);
            if max || min {
                let lo = ring.iter().min().copied().unwrap_or(c);
                let hi = ring.iter().max().copied().unwrap_or(c);
                extrema.push(format!(
                    "({cx}, {cy}) {} at z {c}, ring of radius 32 at {lo}..{hi}",
                    if max { "max" } else { "min" }
                ));
            }
        }
    }
    println!(
        "hall floor x {x0}..{x1}, y {y0}..{y1}: terraces along x mean {:.2} voxels ({} runs), along y mean {:.2} ({} runs); risers of 1, 2, 3+ voxels: {}, {}, {}; mean corner difference over 64 voxels {:.2}",
        runs.0 as f64 / runs.1.max(1) as f64,
        runs.1,
        cruns.0 as f64 / cruns.1.max(1) as f64,
        cruns.1,
        steps[1],
        steps[2],
        steps[3],
        corner_diff.0 as f64 / corner_diff.1.max(1) as f64
    );
    println!(
        "hall floor lattice extrema with closed contours: {}",
        extrema.len()
    );
    for e in &extrema {
        println!("  {e}");
    }
}

/// Plan 0057 Phase 7's light strip and window-size table runs: reading-only, `#[ignore]`d,
/// on the adapter `VXE_TEST_ADAPTER` picks. Written under the worktree's
/// `live/strip-0057/<VXE_LIGHT_TAG>/` (default `tree`).
///
/// - `light_strip`: `VXE_LIGHT_SOURCE` `vein` (default; the level at `VEIN_POSE`) or `hall`
///   (the showcase's hall shot); `VXE_LIGHT_ROW` `fsr2` (default; the owner's window,
///   2530 x 1378 from 1687 x 919, FSR 2 at 1.5, sharpening 0.5) or `native` (1687 x 919 at
///   scale 1, no upscaler). Half-resolution lighting, the fly-through's default light with
///   the lamp off, manual exposure `VXE_LIGHT_EXPOSURE` (default 8), nothing moving, a fresh
///   frame. Frames `STRIP_AT` are saved with a x8 difference image against frame 4096 and
///   the shares within 1, 2 and 4 levels of it (the worst channel). Then four bench inputs
///   (`flythrough_logic::bench_ops` at the camera's voxel, seeds as the fly-through's `B`
///   draws them from `--seed 1`), each followed by frames +1, +4, +16, +64 and +256 against
///   the frame before the press, and the share of the carved region's pixels within 2 levels
///   of frame +256. A second run at light seed 1 (`set_frame_index(SEED_STRIDE)`) gives the
///   grain figures at each strip frame from the cell dump: over the faces with a pixel in
///   view, `D_f` the luma of the cell's mean irradiance times the exposure through the
///   display transform (`rigs::tone_bytes`), in levels; `G_plane` p50 and p90 of
///   `|D_f - mean(D_n)|` over faces with at least two coplanar edge neighbours, the
///   neighbours the 3 x 3 of the same axis and side (the coplanar voxel, then one toward the
///   air, then one into the rock, the first with a sampled cell); `G_noise` p50 and p90 of
///   `|D_f(seed 0) - D_f(seed 1)| / sqrt 2`.
/// - `light_table`: `VXE_LIGHT_POSE` `hall`, `vein` or `patch`, at the window's size under
///   FSR 2, the fly-through's default light (the lamp on on the level, off in the hall; auto
///   exposure on the level, 8 in the hall): 600 still frames, four turns of 90 degrees over 10
///   frames each held 50, a slow 360 over 600, and the flight there and back (the rig's
///   `flight_path` on the level; in the hall along the shot's forward ray to 10 voxels short
///   of rock, at most 160) at 30 voxels a second facing the travel. Per 60 frames the occupied
///   cells, the most misses in a frame, and the preload's claims and refusals summed; the
///   peak; the occupied keys at the end dumped as little-endian word pairs to `keys-<pose>.bin`.
mod light_strip {
    use super::*;
    use std::collections::HashMap;
    use support::SyncNow;
    use support::settle::{FaceKey, SEED_STRIDE, cache_cell, face_key, flight_path};
    use vxe_render::glam::Vec3;
    use vxe_render::{LightingResolution, marcher};

    const WINDOW: [u32; 2] = [2530, 1378];
    const STRIP_AT: [usize; 7] = [1, 4, 16, 64, 256, 1024, 4096];
    const AFTER_PRESS: [usize; 5] = [1, 4, 16, 64, 256];
    const PRESSES: u64 = 4;

    struct Rig {
        device: wgpu::Device,
        queue: wgpu::Queue,
        gpu: GpuWorld,
        world: World,
        emits: Vec<u16>,
        adapter: String,
    }

    /// The source's world mirrored whole, focused at `focus`.
    fn rig(hall: bool, focus: [f32; 3]) -> Option<Rig> {
        let base = flythrough::with_emissive_ore(&flythrough::cave_materials());
        let (world, table, radius) = if hall {
            let source = ShowcaseSource::new(1);
            let table = flythrough::with_showcase_rows(&base);
            (World::generate(source.extent(), &source, &table), table, 19)
        } else {
            let source = flythrough::cave::CaveSource::new(
                1,
                ChunkExtent::new([0, 0, 0], flythrough::cave::LEVEL_CHUNKS),
            );
            let w = World::generate(source.extent, &source, &base);
            (w, base, vxe_render::gpu_world::DEFAULT_LIGHT_RADIUS)
        };
        let pool = flythrough::level_pool_units(&world, false) * UNIT_BYTES;
        let (device, queue, adapter) = reading_device(pool)?;
        let cfg = MirrorConfig {
            budget_bytes: flythrough::level_mirror_budget(
                &world,
                world.extent().len() as u64,
                false,
            ),
            light_radius: radius,
            ..MirrorConfig::new(Window::Fixed(world.extent()))
        };
        let mut gpu = GpuWorld::with_config(&device, cfg);
        gpu.set_focus(VoxelCoord(focus.map(|v| v.floor() as i32)));
        let mut last = None;
        for _ in 0..256 {
            let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            gpu.sync(&world, &queue, &mut enc).expect("the mirror fits");
            queue.submit([enc.finish()]);
            device
                .poll(wgpu::PollType::wait_indefinitely())
                .expect("device poll after a sync");
            if last == Some(gpu.light_samples()) {
                break;
            }
            last = Some(gpu.light_samples());
        }
        println!(
            "light strip: {} on {adapter}, light radius {radius}, {} light samples",
            if hall { "hall" } else { "level" },
            gpu.light_samples()
        );
        Some(Rig {
            device,
            queue,
            gpu,
            world,
            emits: emitting(&table),
            adapter,
        })
    }

    fn hall_camera() -> Camera {
        let s = SHOTS[0];
        Camera {
            position: s.eye.0.map(|v| v as f32 + 0.5).into(),
            yaw: (s.yaw_deg as f32).to_radians(),
            pitch: (s.pitch_deg as f32).to_radians(),
            vfov: 70f32.to_radians(),
        }
    }

    fn make_frame(rig: &Rig, fsr2: bool, lighting: Lighting) -> Frame {
        let device = &rig.device;
        let internal = RenderScale::OneHalf.internal(WINDOW);
        let mut f = if fsr2 {
            let mut f = Frame::with_sizes(device, internal, WINDOW);
            f.set_upscaler_kind(
                device,
                UpscalerKind::Fsr2(Fsr2Settings {
                    sharpness: 0.5,
                    ..Fsr2Settings::default()
                }),
            );
            f
        } else {
            Frame::new(device, internal[0], internal[1])
        };
        f.set_probe_spacing(device, vxe_render::probes::DEFAULT_PROBE_SPACING);
        f.set_lighting(lighting);
        let p = LookPreset::DEFAULT;
        f.set_look(LookParams {
            normal_mode: p.normal_mode,
            tuning: p.tuning,
            far_normal: p.far_normal,
            ..LookParams::default()
        });
        f.set_resolution(device, LightingResolution::Half);
        f.set_dt(1.0 / 60.0);
        f
    }

    fn worst(a: &[u8], b: &[u8], i: usize) -> u8 {
        (0..3)
            .map(|k| a[i * 4 + k].abs_diff(b[i * 4 + k]))
            .max()
            .unwrap()
    }

    /// Shares of pixels (all, or those `keep` names) within 1, 2 and 4 levels; the mean.
    fn shares(a: &[u8], b: &[u8], keep: Option<&[bool]>) -> ([f64; 3], f64, usize) {
        let (mut w, mut n, mut s) = ([0usize; 3], 0usize, 0u64);
        for i in 0..a.len() / 4 {
            if keep.is_some_and(|k| !k[i]) {
                continue;
            }
            let d = worst(a, b, i);
            n += 1;
            s += u64::from(d);
            for (j, lim) in [1, 2, 4].into_iter().enumerate() {
                w[j] += usize::from(d <= lim);
            }
        }
        let n1 = n.max(1) as f64;
        (w.map(|v| v as f64 / n1), s as f64 / n1, n)
    }

    fn diff_x8(a: &[u8], b: &[u8]) -> Vec<u8> {
        a.chunks(4)
            .zip(b.chunks(4))
            .flat_map(|(p, q)| {
                let c = |k: usize| p[k].abs_diff(q[k]).saturating_mul(8);
                [c(0), c(1), c(2), 255]
            })
            .collect()
    }

    fn table_words(rig: &Rig, frame: &Frame) -> Vec<u32> {
        let t = frame.emissive_table();
        let b = readback::read_buffer(&rig.device, &rig.queue, t, 0, t.size());
        b.chunks_exact(4)
            .map(|c| u32::from_le_bytes(c.try_into().expect("a word")))
            .collect()
    }

    /// `D_f` of a key's cell in levels, `None` with no sampled cell.
    fn d_of(words: &[u32], key: FaceKey, lighting: &Lighting, exposure: f32) -> Option<f32> {
        let w = cache_cell(words, key)?;
        let c = marcher::CACHE_COUNT_WORD;
        let count = words[w + c];
        if count == 0 {
            return None;
        }
        let n = count.min(lighting.table_cap) as f32 * marcher::CACHE_Q;
        let rgb = [1, 2, 3].map(|k| words[w + c + k] as f32 / n * exposure);
        let t = support::rigs::tone_bytes([rgb[0], rgb[1], rgb[2], 1.0], lighting);
        Some(0.2126 * f32::from(t[0]) + 0.7152 * f32::from(t[1]) + 0.0722 * f32::from(t[2]))
    }

    /// The faces with a pixel in view, emitters excluded, and their `D_f`.
    fn faces_in_view(
        rig: &Rig,
        frame: &Frame,
        words: &[u32],
        eye: Vec3,
        lighting: &Lighting,
        exposure: f32,
    ) -> HashMap<FaceKey, f32> {
        let pos = readback::read_float(&rig.device, &rig.queue, frame.hit_positions());
        let info = readback::read_rgba16u(&rig.device, &rig.queue, frame.hit_records());
        let mut out = HashMap::new();
        for (p, i) in pos.iter().zip(&info) {
            if rig.emits.contains(&i[2]) {
                continue;
            }
            if let Some(k) = face_key(eye, *p, *i)
                && !out.contains_key(&k)
                && let Some(d) = d_of(words, k, lighting, exposure)
            {
                out.insert(k, d);
            }
        }
        out
    }

    fn p50_p90(mut v: Vec<f32>) -> String {
        if v.is_empty() {
            return "- / - (0)".to_owned();
        }
        v.sort_by(f32::total_cmp);
        let q = |f: f64| v[((v.len() - 1) as f64 * f).round() as usize];
        format!("{:.2} / {:.2} ({})", q(0.5), q(0.9), v.len())
    }

    /// `G_plane` over the faces in view.
    fn g_plane(
        words: &[u32],
        faces: &HashMap<FaceKey, f32>,
        lighting: &Lighting,
        exposure: f32,
    ) -> String {
        let mut out = Vec::new();
        for (&(v, side), &d) in faces {
            let axis = usize::from(side / 2);
            let sign = if side % 2 == 1 { 1 } else { -1 };
            let (u, w) = ((axis + 1) % 3, (axis + 2) % 3);
            let (mut sum, mut n, mut edges) = (0f32, 0u32, 0u32);
            for du in -1..=1i32 {
                for dw in -1..=1i32 {
                    if du == 0 && dw == 0 {
                        continue;
                    }
                    for (step, off) in [0, sign, -sign].into_iter().enumerate() {
                        let mut c = v;
                        c[u] += du;
                        c[w] += dw;
                        c[axis] += off;
                        if let Some(dn) = d_of(words, (c, side), lighting, exposure) {
                            sum += dn;
                            n += 1;
                            edges += u32::from(step == 0 && du.abs() + dw.abs() == 1);
                            break;
                        }
                    }
                }
            }
            if edges >= 2 {
                out.push((d - sum / n as f32).abs());
            }
        }
        p50_p90(out)
    }

    fn g_noise(a: &HashMap<FaceKey, f32>, b: &HashMap<FaceKey, f32>) -> String {
        p50_p90(
            a.iter()
                .filter_map(|(k, &da)| b.get(k).map(|&db| (da - db).abs() / 2f32.sqrt()))
                .collect(),
        )
    }

    fn stats(rig: &Rig, frame: &Frame) -> (marcher::TableStats, marcher::PreloadStats) {
        (
            frame.read_table_stats(&rig.device, &rig.queue),
            frame.read_preload_stats(&rig.device, &rig.queue),
        )
    }

    fn stats_line(t: &marcher::TableStats, p: &marcher::PreloadStats) -> String {
        format!(
            "table {} {} {} ({:.4}), preload {} bricks {} listed, claimed r0 {} lit {} unlit, to R {}, refused load {} miss {} unlit {}",
            t.cells,
            t.occupied,
            t.misses,
            f64::from(t.occupied) / f64::from(t.cells),
            p.edit_bricks + p.visited,
            p.listed,
            p.claimed_near_lit,
            p.claimed_near_unlit,
            p.claimed
                .saturating_sub(p.claimed_near_lit + p.claimed_near_unlit),
            p.refused_load,
            p.refused_miss,
            p.refused_unlit
        )
    }

    fn out_dir(sub: &str) -> PathBuf {
        let tag = env("VXE_LIGHT_TAG").unwrap_or_else(|| "tree".to_owned());
        let d = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../live/strip-0057")
            .join(tag)
            .join(sub);
        std::fs::create_dir_all(&d).expect("create the strip directory");
        d
    }

    fn display(rig: &Rig, frame: &Frame) -> Vec<u8> {
        readback::read_rgba8(&rig.device, &rig.queue, frame.target())
    }

    #[test]
    #[ignore = "reading only: run with --ignored"]
    fn light_strip() {
        let hall = env("VXE_LIGHT_SOURCE").as_deref() == Some("hall");
        let fsr2 = env("VXE_LIGHT_ROW").as_deref() != Some("native");
        let exposure: f32 = env_or("VXE_LIGHT_EXPOSURE", 8.0);
        let camera = if hall {
            hall_camera()
        } else {
            flythrough::VEIN_POSE.camera()
        };
        let Some(mut rig) = rig(hall, camera.position.to_array()) else {
            println!("light strip: no adapter; skipped");
            return;
        };
        let name = format!(
            "{}-{}",
            if hall { "hall" } else { "vein" },
            if fsr2 { "fsr2" } else { "native" }
        );
        let dir = out_dir(&name);
        let lighting = Lighting {
            exposure: Exposure::Manual(exposure),
            lamp: false,
            lamp_bounce: true,
            ..Lighting::default()
        };
        let light = LightParams::headlamp();
        // Seed 1 first: its faces at each strip frame, for G_noise.
        let mut seed1 = Vec::new();
        {
            let mut f = make_frame(&rig, fsr2, lighting);
            f.set_frame_index(SEED_STRIDE);
            for n in 1..=*STRIP_AT.last().unwrap() {
                f.step(&rig.device, &rig.queue, &rig.gpu, &camera, &light);
                if STRIP_AT.contains(&n) {
                    let words = table_words(&rig, &f);
                    seed1.push(faces_in_view(
                        &rig,
                        &f,
                        &words,
                        camera.position,
                        &lighting,
                        exposure,
                    ));
                }
            }
        }
        let mut frame = make_frame(&rig, fsr2, lighting);
        let size = frame.target().size();
        let (w, h) = (size.width, size.height);
        println!(
            "light strip {name}: {w} x {h}, internal {:?}",
            frame.internal_size()
        );
        let mut shots = Vec::new();
        for n in 1..=*STRIP_AT.last().unwrap() {
            frame.step(&rig.device, &rig.queue, &rig.gpu, &camera, &light);
            if STRIP_AT.contains(&n) {
                let img = display(&rig, &frame);
                let (t, p) = stats(&rig, &frame);
                let words = table_words(&rig, &frame);
                let faces =
                    faces_in_view(&rig, &frame, &words, camera.position, &lighting, exposure);
                let gp = g_plane(&words, &faces, &lighting, exposure);
                let gn = g_noise(&faces, &seed1[shots.len()]);
                write_png(&dir.join(format!("f{n:04}.png")), &img, w, h);
                shots.push((n, img, stats_line(&t, &p), gp, gn));
            } else if n % 64 == 0 {
                rig.device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .expect("poll");
            }
        }
        let reference = shots.last().unwrap().1.clone();
        for (n, img, st, gp, gn) in &shots {
            let (s, mean, _) = shares(img, &reference, None);
            if *n != 4096 {
                write_png(
                    &dir.join(format!("diff-x8-f{n:04}-vs-f4096.png")),
                    &diff_x8(img, &reference),
                    w,
                    h,
                );
            }
            println!(
                "light strip {name} frame {n}: within 1/2/4 {:.4} / {:.4} / {:.4}, mean {mean:.3}; G_plane {gp}; G_noise {gn}; {st}",
                s[0], s[1], s[2]
            );
        }
        // The owner's B: bench inputs from frame 4096.
        let mut before = reference;
        let mut frames_done = 4096usize;
        for k in 0..PRESSES {
            let origin = camera.position.to_array().map(|v| v.floor() as i64);
            let seed = 1u64 ^ k.wrapping_mul(0x9e37_79b9_7f4a_7c15);
            let mut boxes = Vec::new();
            let mut changed = 0u64;
            for op in flythrough_logic::bench_ops(origin, seed) {
                let r = rig.world.apply(&op).expect("the bench carve applies");
                changed += r.changed_from.iter().map(|c| u64::from(c.1)).sum::<u64>();
                if let Some((min, max)) = r.aabb {
                    boxes.push((min.0, max.0));
                }
            }
            rig.gpu
                .sync_now(&rig.world, &rig.device, &rig.queue)
                .expect("mirror fits");
            for &(min, max) in &boxes {
                frame.invalidate(min, max);
            }
            let mut after = Vec::new();
            for n in 1..=*AFTER_PRESS.last().unwrap() {
                frame.step(&rig.device, &rig.queue, &rig.gpu, &camera, &light);
                if AFTER_PRESS.contains(&n) {
                    let img = display(&rig, &frame);
                    let (t, p) = stats(&rig, &frame);
                    write_png(&dir.join(format!("b{k}-p{n:03}.png")), &img, w, h);
                    write_png(
                        &dir.join(format!("diff-x8-b{k}-p{n:03}-vs-before.png")),
                        &diff_x8(&img, &before),
                        w,
                        h,
                    );
                    after.push((n, img, stats_line(&t, &p)));
                }
            }
            frames_done += AFTER_PRESS.last().unwrap();
            // The carved region: display pixels whose hit (under them) lies in a box.
            let pos = readback::read_float(&rig.device, &rig.queue, frame.hit_positions());
            let [iw, ih] = frame.internal_size();
            let mut carved = vec![false; (w * h) as usize];
            for y in 0..h {
                let iy = aliasing::under(y, h, ih) as u32;
                for x in 0..w {
                    let ix = aliasing::under(x, w, iw) as u32;
                    let p = pos[(iy * iw + ix) as usize];
                    if p[3] > 1e30 {
                        continue;
                    }
                    carved[(y * w + x) as usize] = boxes.iter().any(|(a, b)| {
                        (0..3).all(|c| p[c] >= a[c] as f32 - 1.0 && p[c] <= b[c] as f32 + 2.0)
                    });
                }
            }
            let last = after.last().unwrap().1.clone();
            for (n, img, st) in &after {
                let (s, mean, px) = shares(img, &last, Some(&carved));
                let (sb, _, _) = shares(img, &before, None);
                println!(
                    "light strip {name} press {k} (seed {seed:#x}, {} carves changing {changed} voxels, frame {}) +{n}: carved region {px} px within 2 of +256 {:.4} (1/4: {:.4} / {:.4}, mean {mean:.3}); whole frame within 2 of before {:.4}; {st}",
                    boxes.len(),
                    frames_done,
                    s[1],
                    s[0],
                    s[2],
                    sb[1]
                );
            }
            before = last;
        }
        println!(
            "light strip {name}: on {}, frames under {}",
            rig.adapter,
            dir.display()
        );
    }

    #[test]
    #[ignore = "reading only: run with --ignored"]
    fn light_table() {
        let pose = env("VXE_LIGHT_POSE").unwrap_or_else(|| "vein".to_owned());
        let hall = pose == "hall";
        let start = if hall {
            hall_camera()
        } else {
            Pose::named(&pose)
                .expect("VXE_LIGHT_POSE takes hall, vein or patch")
                .camera()
        };
        let Some(rig) = rig(hall, start.position.to_array()) else {
            println!("light table: no adapter; skipped");
            return;
        };
        let lighting = Lighting {
            exposure: if hall {
                Exposure::Manual(8.0)
            } else {
                Exposure::Auto(AutoExposure::eye())
            },
            lamp: !hall,
            lamp_bounce: true,
            ..Lighting::default()
        };
        // The flight's ends.
        let (a, b) = if hall {
            let (dir, _, _) = start.basis();
            let o = centre_of_voxel(VoxelCoord(
                start.position.to_array().map(|v| v.floor() as i32),
            ));
            let d = [dir.x, dir.y, dir.z].map(|c| (c * FIXED_ONE as f32).round() as i64);
            let t = rig
                .world
                .raycast(o, d, 400 * FIXED_ONE)
                .expect("the ray is valid")
                .map_or(160.0, |hit| hit.t as f32 / FIXED_ONE as f32);
            let len = (t - 10.0).clamp(0.0, 160.0);
            (start.position, start.position + dir * len)
        } else {
            let (a, b) = flight_path(&Pose::named(&pose).unwrap());
            (Vec3::from(a), Vec3::from(b))
        };
        let len = (b - a).length();
        let step = support::settle::FLY_SPEED / 60.0;
        let leg = (len / step).ceil() as usize;
        let mut cams: Vec<Camera> = Vec::new();
        cams.extend(std::iter::repeat_n(start, 600));
        let yawed = |deg: f32| Camera {
            yaw: start.yaw + deg.to_radians(),
            ..start
        };
        for turn in 0..4 {
            for f in 1..=10 {
                cams.push(yawed(90.0 * turn as f32 + 9.0 * f as f32));
            }
            cams.extend(std::iter::repeat_n(yawed(90.0 * (turn + 1) as f32), 50));
        }
        for f in 1..=600 {
            cams.push(yawed(360.0 * f as f32 / 600.0));
        }
        for (from, to) in [(a, b), (b, a)] {
            let dir = (to - from).normalize_or_zero();
            for f in 1..=leg {
                cams.push(Camera {
                    position: from + dir * (step * f as f32).min(len),
                    yaw: dir.y.atan2(dir.x),
                    pitch: dir.z.clamp(-1.0, 1.0).asin(),
                    vfov: 70f32.to_radians(),
                });
            }
        }
        let mut frame = make_frame(&rig, true, lighting);
        let light = LightParams::headlamp();
        println!(
            "light table {pose}: {} frames (600 still, 240 turns, 600 slow, flight {len:.0} voxels {leg} frames each way), window {:?} from {:?}, on {}",
            cams.len(),
            WINDOW,
            frame.internal_size(),
            rig.adapter
        );
        let (mut peak, mut peak_at, mut peak_miss, mut miss_at) = (0u32, 0usize, 0u32, 0usize);
        let mut block = [0u64; 7];
        let mut totals = [0u64; 7];
        for (i, cam) in cams.iter().enumerate() {
            frame.step(&rig.device, &rig.queue, &rig.gpu, cam, &light);
            let (t, p) = stats(&rig, &frame);
            if t.occupied > peak {
                (peak, peak_at) = (t.occupied, i + 1);
            }
            if t.misses > peak_miss {
                (peak_miss, miss_at) = (t.misses, i + 1);
            }
            let v = [
                u64::from(t.misses),
                u64::from(p.claimed_near_lit),
                u64::from(p.claimed_near_unlit),
                u64::from(
                    p.claimed
                        .saturating_sub(p.claimed_near_lit + p.claimed_near_unlit),
                ),
                u64::from(p.refused_load),
                u64::from(p.refused_miss),
                u64::from(p.refused_unlit),
            ];
            block[0] = block[0].max(v[0]);
            for k in 1..7 {
                block[k] += v[k];
                totals[k] += v[k];
            }
            totals[0] += v[0];
            if (i + 1) % 60 == 0 || i + 1 == cams.len() {
                println!(
                    "light table {pose} frame {}: occupied {} ({:.4}), misses max {}, claimed r0 lit {} unlit {} to R {}, refused load {} miss {} unlit {}",
                    i + 1,
                    t.occupied,
                    f64::from(t.occupied) / f64::from(t.cells),
                    block[0],
                    block[1],
                    block[2],
                    block[3],
                    block[4],
                    block[5],
                    block[6]
                );
                block = [0; 7];
            }
        }
        let words = table_words(&rig, &frame);
        let keys: Vec<u32> = words
            .chunks_exact(marcher::CACHE_CELL_WORDS as usize)
            .filter(|c| c[1] != 0)
            .flat_map(|c| [c[0], c[1]])
            .collect();
        let path = out_dir("").join(format!("keys-{pose}.bin"));
        std::fs::write(
            &path,
            keys.iter()
                .flat_map(|k| k.to_le_bytes())
                .collect::<Vec<u8>>(),
        )
        .expect("write the keys");
        println!(
            "light table {pose}: peak occupied {peak} at frame {peak_at}, misses peak {peak_miss} at frame {miss_at}, misses summed {}; claimed r0 lit {} unlit {} to R {}, refused load {} miss {} unlit {} summed; {} keys at the end to {}",
            totals[0],
            totals[1],
            totals[2],
            totals[3],
            totals[4],
            totals[5],
            totals[6],
            keys.len() / 2,
            path.display()
        );
    }
}
