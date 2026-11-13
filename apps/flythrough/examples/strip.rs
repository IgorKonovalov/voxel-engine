//! A reading-only strip tool: the showcase or the level rendered headless, with history, on the
//! adapter `--adapter` picks (lavapipe by default), along a scripted camera. It writes frames,
//! crops, a contact sheet and a per-pixel flicker map under `renders/strips/<tag>/`, and prints
//! each frame's mean display luma. Nothing is asserted. `--help` lists every flag; the readings:
//!
//! - `--reading strip-run` (the default): the strip, narrowed by the flags below.
//! - `--reading hall-floor-terraces`: R1's CPU half, the hall floor's terraces (no device).
//!
//! The strip's flags:
//!
//! - `--tag`: the output directory's name (default `strip`).
//! - `--source`: `showcase` (default) or `level` (seed 1, the ore emissive).
//! - `--shot`: `hall` (default), `chasm` or `crystals` on the showcase; `vein` (default),
//!   `patch` or `spawn` (the fly-through's start, facing +x) on the level.
//! - `--camera`: `x,y,z,yaw,pitch` (voxels, degrees) in place of the shot's camera; the shot
//!   still names the exposure.
//! - `--view`: `shaded` (default), `normals`, `albedo`, `hitmask` or `steps`.
//! - `--size`: the output size `WxH` (default `1920x1080`).
//! - `--upscaler`: `fsr2` (default), `own` or `off`; `fsr2` and `own` at scale 1.5 with
//!   sharpening 0.5, `off` at scale 1 with no jitter.
//! - `--look`: a `LookPreset` name (default: `LookPreset::DEFAULT`, the fly-through's start, far
//!   normal `[2, 6]`; `face` is the same cubes with the far normal off).
//! - `--far`: `LO,HI`, `LookParams::far_normal` (default: the preset's).
//! - `--energy`: crops `x,y,w,h` separated by `/`: the last frame's mean horizontal and vertical
//!   neighbour `|dY|` is printed for each.
//! - `--lamp`: `on` or `off` at the start (default off on the showcase, on on the level).
//! - `--indirect`, `--filter`: `on` (default) or `off`.
//! - `--candidates`: emissive candidates per pixel (default 8).
//! - `--shadow`: the shadow walk, `coarse` (default) or `full`.
//! - `--mean`: `on` writes `mean.png` (and its crop), the run's frames averaged.
//! - `--exposure`: a number, or `auto` (default: the shot's or pose's manual value).
//! - `--settle`: frames rendered still before the run (default 120).
//! - `--frames`: the run's frames (default 240).
//! - `--every`: a frame is written every N run frames, from frame 0 (default 24).
//! - `--speed`: forward speed in voxels a second (default 0).
//! - `--yaw`: yaw rate in degrees a second, positive to the left (default 0).
//! - `--lamp-at`: the run frame the lamp switches (default: never).
//! - `--crop`: `x,y,w,h` of a crop written beside every frame (default: none).
//! - `--flicker`: `from,to` run frames the flicker map averages over (default: the whole run).
//!   The map is the mean per pixel of `|dY|` between consecutive frames, `Y` the
//!   display-encoded luma in 0..1, written times 8 as grey.
//! - `--light`: `hero` (the flags above), `eye` (`aliasing::eye_lamp` at `--lamp-power`, default
//!   4000 (the hall's dome crop at a mean display luma of 30 to 120 at exposure 8), with
//!   `aliasing::eye_lighting`: the lamp on, no indirect, no emissive candidates, the mirror's
//!   light radius 0), `eye0` (`eye` with every emitting material's emission scaled to 0) or
//!   `baked` (the hero's flags over the store, ADR-0046: the chunks within `--bake-radius`
//!   (default 12) of the eye's chunk baked before the settle at `BakeSettings::default()` with
//!   rounds of 512, as `gpu_bake.rs`'s `brick_bake`, the fly-through's default probe spacing).
//!   Given as `hero`, `eye` or `eye0`, the probe spacing defaults to the hero's 32.
//! - `--light-radius`: the mirror's `light_radius` (default: 0 under `eye`, else the source's).
//!   `--probe-spacing`: the probe volume's spacing.
//! - `--sharpen`: the upscaler's sharpening (default 0.5).
//! - `--lighting`: the light passes' resolution, `full` or `half` (default: the frame's, half).
//! - `--store-read`: `face` (default) or `normal`, `Lighting::store_read` (ADR-0049);
//!   `--weights`: `normal` (default) or `face`, `Lighting::light_weights`.
//! - FSR 2's candidate knobs (`Fsr2Settings`): `--jitter-phases` (the jitter's phase count; the
//!   fold's default follows it) and `--dither on` (the tone pass's static one-level dither).
//! - `--read fold`: after the settle, `--frames` still frames folded over `--phases` (default:
//!   `fsr2::jitter_phases`, `own`'s `JITTER_PHASES`, 18 for `off`): `T`, `L`, `J`
//!   (`support::aliasing`), the guard's energy, the jumps by class and footprint over the first
//!   cycle's mask, and `jumps.png` (T's jumps per pixel, white at `JUMPS_WHITE` jumps),
//!   `jumps-l.png`, `mask.png`, `last.png` and `clusters.png` (the 8 worst 32-pixel tiles).
//! - `--scale`: the upscalers' internal scale, `1`, `1.5` (default) or `2`.
//! - `--shrink K`: with `off`, renders `K` times the size on each edge and box-filters every read
//!   frame down to the output size (`R` at `K` 2 and `--far 4,12`).
//! - `--ref`: absolute paths to PNGs of the output size, separated by `,`; a fold prints its last
//!   frame's mean level distance from each, on the whole frame and on the far crops, and the
//!   distance by class (split by the direction map).
//! - A fold also reads the **far crops** (the dome and the far wall, `GUARD_CROPS` 0 and 2 at
//!   1920 x 1080, scaled to the output size): their guard energy and `T`; the class tables split
//!   by the direction map (`aliasing::directions`), on the frame and on the far crops; the
//!   direction map's edge ratio by 10 m bins of `t` (`aliasing::DirEdges`), from the far normal's
//!   band start (`--band-hi`, default the look's `far_normal[1]`, in internal pixels); and writes
//!   `dirmap.png`.
//! - `--rings x,y,w,h`: a fold reads the rings on that crop of its last frame (`aliasing::Rings`)
//!   and writes `zlevel.png`, `rings-zlevel.png` and `rings-crop.png`.
//! - `--pan PX_PER_FRAME`: after the settle, `--frames` frames of a pure yaw to the left at that
//!   many display pixels a frame at the image centre, `C` read in the centre band against the
//!   frame `lag` before shifted `shift` pixels (`aliasing::lag_shift`), with its jumps by class;
//!   `--dots on` adds the dot and yellow-dot count and their anchoring.
//! - `--read drift`: step `settle + 1` against step `--drift-to` (default 8192).
//!
//! Example (the RTX, under the perf lock and the memory cap):
//! `cargo run --release -p flythrough --example strip -- --adapter rtx --tag lamp-off --source
//! level --lamp-at 60`.

// The GPU tests' support tree: the strip takes its lavapipe device, its PNG files and its
// aliasing reads (`support::aliasing`); the tree allows its own dead code.
#[path = "../tests/support/mod.rs"]
mod support;

#[path = "strip/args.rs"]
mod args;

use std::path::{Path, PathBuf};

use args::{Args, LightMode, Reading};
use flythrough::LookPreset;
use flythrough::Pose;
use flythrough::showcase::{SHOTS, ShowcaseSource};
use support::aliasing::{self, ByClass, Hits, Mask};
use support::{read_png, write_png};
use vxe_render::{
    Camera, Exposure, Frame, Fsr2Settings, LightParams, Lighting, LookParams, UpscalerKind, wgpu,
};
use vxe_render::{GpuWorld, MirrorConfig, Window, internals::UNIT_BYTES};
use vxe_world::*;

fn main() {
    let args = match args::parse(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("strip: {e}");
            std::process::exit(2);
        }
    };
    if args.help {
        print!("{}", args::table().help());
        return;
    }
    match args.reading {
        Reading::StripRun => strip_run(&args),
        Reading::HallFloorTerraces => hall_floor_terraces(),
    }
}

/// A device for a reading, chosen by `--adapter`: unset, empty or `fallback` is lavapipe,
/// exactly as `support::software_device` picks it (`None` with a printed skip when the machine
/// has none); any other value is the first Vulkan adapter whose name contains it, ignoring case
/// (`rtx`, `radeon`); with no match it panics and lists the adapters. The adapter's name labels
/// the readings.
fn reading_device(
    adapter: Option<&str>,
    pool_bytes: u64,
) -> Option<(wgpu::Device, wgpu::Queue, String)> {
    let want = adapter.unwrap_or_default();
    if want.is_empty() || want.eq_ignore_ascii_case("fallback") {
        let (device, queue) = support::software_device(pool_bytes)?;
        return Some((device, queue, "llvmpipe (lavapipe)".to_owned()));
    }
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN));
    let names: Vec<String> = adapters.iter().map(|a| a.get_info().name).collect();
    let needle = want.to_lowercase();
    let Some(adapter) = adapters
        .into_iter()
        .find(|a| a.get_info().name.to_lowercase().contains(&needle))
    else {
        panic!("--adapter {want} matches no adapter; the adapters are {names:?}");
    };
    let info = adapter.get_info();
    println!(
        "adapter: {} ({:?}, driver {} {}), chosen by --adapter {want}",
        info.name, info.backend, info.driver, info.driver_info
    );
    let desc = wgpu::DeviceDescriptor {
        // The per-pass readings time the frame where the adapter can.
        required_features: adapter.features() & wgpu::Features::TIMESTAMP_QUERY,
        required_limits: vxe_render::limits_for(&adapter, pool_bytes)
            .expect("the frame's limits on the chosen adapter"),
        ..Default::default()
    };
    let (device, queue) =
        pollster::block_on(adapter.request_device(&desc)).expect("device on the chosen adapter");
    Some((device, queue, info.name))
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
    /// The voxels the mirror holds, for a bake.
    world: World,
}

/// The hero's probe spacing (`examples/hero.rs`), the default under `--light`.
const HERO_PROBE_SPACING: u32 = 32;

/// The run's step, a 60 Hz frame.
const DT: f32 = 1.0 / 60.0;

fn emitting(table: &MaterialTable) -> Vec<u16> {
    (1..table.len() as u16)
        .filter(|&m| {
            table
                .get(m)
                .is_some_and(|r| r.emission.iter().any(|&e| e > 0.0))
        })
        .collect()
}

fn sync_whole(scene: &mut Scene) {
    let mut last = None;
    for _ in 0..256 {
        let mut enc = scene
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        scene
            .gpu
            .sync(&scene.world, &scene.queue, &mut enc)
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

/// The mirror's light radius: `--light-radius`, else 0 under the eye lamp, else the source's
/// `default`.
fn light_radius(args: &Args, default: i32) -> i32 {
    let eye = matches!(args.light, Some(LightMode::Eye | LightMode::Eye0));
    args.light_radius.unwrap_or(if eye { 0 } else { default })
}

/// The level (seed 1) at the `--shot` pose.
fn level_scene(args: &Args, base: &MaterialTable) -> Option<Scene> {
    let source = flythrough::cave::CaveSource::new(
        1,
        ChunkExtent::new([0, 0, 0], flythrough::cave::LEVEL_CHUNKS),
    );
    let world = World::generate(source.extent, &source, base);
    let pose = args.shot.as_deref().unwrap_or("vein");
    let pose = if pose == "spawn" {
        Pose {
            name: "spawn",
            position: source.spawn().0.map(|v| v as f32 + 0.5),
            yaw_deg: 0.0,
            pitch_deg: 0.0,
            exposure: 1.0,
        }
    } else {
        Pose::named(pose).expect("--shot takes vein, patch or spawn on the level")
    };
    let pool = flythrough::level_pool_units(&world, false) * UNIT_BYTES;
    let (device, queue, adapter) = reading_device(args.adapter.as_deref(), pool)?;
    let base_cfg = MirrorConfig::new(Window::Fixed(world.extent()));
    let cfg = MirrorConfig {
        budget_bytes: flythrough::level_mirror_budget(&world, world.extent().len() as u64, false),
        light_radius: light_radius(args, base_cfg.light_radius),
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
        emits: emitting(base),
        light_radius: radius,
        world,
    };
    sync_whole(&mut scene);
    Some(scene)
}

fn scene(args: &Args) -> Option<Scene> {
    let base = flythrough::with_emissive_ore(&flythrough::cave_materials());
    if args.level {
        return level_scene(args, &base);
    }
    let source = ShowcaseSource::new(1);
    let table = flythrough::with_showcase_rows(&base);
    let world = World::generate(source.extent(), &source, &table);
    let shot = args.shot.as_deref().unwrap_or("hall");
    let shot = SHOTS
        .iter()
        .find(|s| s.name == shot)
        .expect("--shot takes hall, chasm or crystals on the showcase");
    let pool = flythrough::level_pool_units(&world, false) * UNIT_BYTES;
    let (device, queue, adapter) = reading_device(args.adapter.as_deref(), pool)?;
    let cfg = MirrorConfig {
        budget_bytes: flythrough::level_mirror_budget(&world, world.extent().len() as u64, false),
        light_radius: light_radius(args, 19),
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
        world,
    };
    sync_whole(&mut scene);
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

/// The neighbour energy on `--energy`'s crops, else the guard's at 1920 x 1080, else the whole
/// image.
fn guard(energy: Option<&[[u32; 4]]>, rgba: &[u8], w: u32, h: u32) -> String {
    let crops: Vec<[u32; 4]> = match energy {
        Some(crops) => crops.to_vec(),
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
        vxe_render::debug::read_float(&scene.device, &scene.queue, frame.hit_positions()),
        vxe_render::debug::read_rgba16u(&scene.device, &scene.queue, frame.hit_records()),
    )
}

/// The display image: the target, box-filtered down by `shrink` (`R`).
fn display(scene: &Scene, frame: &Frame, shrink: u32) -> Vec<u8> {
    let t = frame.target();
    let rgba = vxe_render::debug::read_rgba8(&scene.device, &scene.queue, t);
    if shrink > 1 {
        shrink_image(&rgba, t.width(), t.height(), shrink).0
    } else {
        rgba
    }
}

/// `--light baked`: the chunks within `--bake-radius` (default 12) of the camera's chunk baked
/// into the store at `BakeSettings::default()` with rounds of 512 (`gpu_bake.rs`'s
/// `brick_bake`), before any frame reads it.
fn bake_view(scene: &mut Scene, frame: &mut Frame, radius: i32) {
    let settings = vxe_render::BakeSettings {
        round: 512,
        ..vxe_render::BakeSettings::default()
    };
    let world = scene.world.extent();
    let size = world.size();
    let centre = scene
        .camera
        .position
        .to_array()
        .map(|v| (v / 64.0).floor() as i32);
    let lo = [0, 1, 2].map(|k| (centre[k] - radius).max(world.min.0[k]));
    let hi = [0, 1, 2].map(|k| (centre[k] + radius + 1).min(world.min.0[k] + size[k]));
    let extent = ChunkExtent::new(lo, [0, 1, 2].map(|k| hi[k].max(lo[k])));
    #[allow(
        clippy::disallowed_methods,
        reason = "a reading times its bake on the wall clock"
    )]
    let t = std::time::Instant::now();
    let r = frame
        .bake_until(
            &scene.device,
            &scene.queue,
            &mut scene.gpu,
            &scene.world,
            extent,
            &settings,
            &mut |_| {},
        )
        .expect("the view's chunks fit the store's pool");
    println!(
        "strip: baked chunks {lo:?}..{hi:?} (radius {radius}) at rounds of 512: {} chunks, {} faces, {:.3e} samples in {:.0} ms",
        r.chunks,
        r.faces,
        r.samples as f64,
        elapsed_ms(t)
    );
}

#[allow(
    clippy::disallowed_methods,
    reason = "a reading times its bake on the wall clock"
)]
fn elapsed_ms(t: std::time::Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

/// The far crops at `w` x `h`: the dome and the far wall (`GUARD_CROPS` 0 and 2) scaled from
/// 1920 x 1080.
fn far_crops(w: u32, h: u32) -> [[u32; 4]; 2] {
    [GUARD_CROPS[0], GUARD_CROPS[2]].map(|c| {
        [
            c[0] * w / 1920,
            c[1] * h / 1080,
            (c[2] * w / 1920).max(2),
            (c[3] * h / 1080).max(2),
        ]
    })
}

fn in_crop(i: usize, w: u32, c: [u32; 4]) -> bool {
    let (x, y) = (i as u32 % w, i as u32 / w);
    x >= c[0] && x < c[0] + c[2] && y >= c[1] && y < c[1] + c[3]
}

/// `T` over consecutive `frames` on the `c` crop of images `w` wide.
fn crop_t(frames: &[Vec<u8>], w: u32, c: [u32; 4]) -> aliasing::Change {
    let mut hist = aliasing::Hist::default();
    for pair in frames.windows(2) {
        let (a, b) = (crop(&pair[0], w, c), crop(&pair[1], w, c));
        aliasing::levels(&a, &b)
            .into_iter()
            .for_each(|d| hist.add(d));
    }
    hist.change()
}

/// `--pan PX_PER_FRAME`: a pure yaw to the left from the settled still, `C` read in the centre
/// band each frame against the frame `lag` before shifted `shift` pixels.
struct PanRead<'a> {
    size: [u32; 2],
    shrink: u32,
    rate: f32,
    frames: u32,
    dots: bool,
    tag: &'a str,
    dir: &'a Path,
    energy: Option<&'a [[u32; 4]]>,
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
            let mut mask = Mask::of(&hits, [w, h], tan_sum, &is_emit);
            mask.set_directions(&hits, camera.position.to_array());
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
        println!(
            "pan guard energy (last frame): {}",
            guard(self.energy, &last, w, h)
        );
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

/// `--read fold`: a still camera's frames folded over the jitter's cycle.
struct FoldRead<'a> {
    size: [u32; 2],
    shrink: u32,
    phases: u32,
    frames: u32,
    /// The far normal's band's upper end in internal pixels: the direction read's `t` from.
    band_hi: f32,
    tag: &'a str,
    dir: &'a Path,
    energy: Option<&'a [[u32; 4]]>,
    /// `--ref`'s PNGs.
    refs: &'a [PathBuf],
    /// `--rings`' crop.
    rings: Option<[u32; 4]>,
}

/// What a fold read: its frames, the first cycle's merged class mask and the fold.
struct Folded {
    frames: Vec<Vec<u8>>,
    mask: Mask,
    fold: aliasing::Fold,
}

impl FoldRead<'_> {
    fn run(&self, scene: &mut Scene, frame: &mut Frame, camera: &Camera, light: &LightParams) {
        let f = self.read(scene, frame, camera, light);
        self.print_fold(&f);
        self.write_maps(&f);
        self.clusters(&f);
        self.far_and_refs(&f);
        self.directions(scene, frame, camera, &f);
        self.rings(scene, frame, &f);
        println!("fold: written under {}", self.dir.display());
    }

    /// The fold's frames, and the class mask merged over its first cycle.
    fn read(
        &self,
        scene: &mut Scene,
        frame: &mut Frame,
        camera: &Camera,
        light: &LightParams,
    ) -> Folded {
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
                let mut m = Mask::of(&hits, [w, h], tan_sum, &is_emit);
                m.set_directions(&hits, camera.position.to_array());
                match &mut mask {
                    Some(a) => a.merge(&m),
                    None => mask = Some(m),
                }
            }
        }
        let fold = aliasing::fold(&frames, self.phases);
        let mask = mask.expect("a fold reads at least one frame");
        Folded { frames, mask, fold }
    }

    fn print_fold(&self, f: &Folded) {
        let [w, h] = self.size;
        let (fold, mask) = (&f.fold, &f.mask);
        let last = f.frames.last().expect("a frame");
        println!(
            "fold {}: {} frames, {} phases, {} whole cycles",
            self.tag,
            f.frames.len(),
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
        println!("fold guard energy: {}", guard(self.energy, last, w, h));
        if let Some(c) = from_cycle_mean(&f.frames, self.phases) {
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
            ByClass::of(mask, &ones).table("pixels")
        );
        println!(
            "fold T jumps by class:\n{}",
            ByClass::of(mask, &fold.t_jumps).table("T jumps")
        );
        println!(
            "fold L jumps by class:\n{}",
            ByClass::of(mask, &fold.l_jumps).table("L jumps")
        );
    }

    fn write_maps(&self, f: &Folded) {
        let [w, h] = self.size;
        write_png(
            &self.dir.join("jumps.png"),
            &aliasing::count_image(&f.fold.t_jumps, JUMPS_WHITE),
            w,
            h,
        );
        write_png(
            &self.dir.join("jumps-l.png"),
            &aliasing::count_image(&f.fold.l_jumps, JUMPS_WHITE),
            w,
            h,
        );
        write_png(&self.dir.join("mask.png"), &f.mask.image(), w, h);
        write_png(
            &self.dir.join("last.png"),
            f.frames.last().expect("a frame"),
            w,
            h,
        );
    }

    /// The worst clusters: 32-pixel tiles by T jumps, a 96-pixel crop around each, magnified
    /// twice, beside the same crop of the mask.
    fn clusters(&self, f: &Folded) {
        const TILE: u32 = 32;
        const CROP: u32 = 96;
        let [w, h] = self.size;
        let (fold, mask) = (&f.fold, &f.mask);
        let last = f.frames.last().expect("a frame");
        let mask_image = mask.image();
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
    }

    /// The far crops: energy, T, and the class tables split by the direction map; then the
    /// distance from each `--ref`.
    fn far_and_refs(&self, f: &Folded) {
        let [w, h] = self.size;
        let last = f.frames.last().expect("a frame");
        let mask = &f.mask;
        let far = far_crops(w, h);
        let in_far = |i: usize| far.iter().any(|&c| in_crop(i, w, c));
        for (name, c) in ["dome", "far wall"].iter().zip(far) {
            let (eh, ev) = neighbour_energy(last, w, c);
            println!(
                "fold far crop {name} {c:?}: energy h {eh:.5} v {ev:.5} mean {:.5}; T {}",
                (eh + ev) / 2.0,
                crop_t(&f.frames, w, c).cell()
            );
        }
        println!(
            "fold T jumps by class, far crops:\n{}",
            ByClass::of_where(mask, &f.fold.t_jumps, &in_far).table("T jumps")
        );
        for path in self.refs {
            let (r, rw, rh) = read_png(path);
            assert_eq!((rw, rh), (w, h), "--ref is {rw}x{rh}");
            let d = aliasing::levels(last, &r);
            println!(
                "fold distance from {}: mean {:.3} levels, over 8 {:.2} %",
                path.display(),
                d.iter().map(|&v| f64::from(v)).sum::<f64>() / d.len() as f64,
                d.iter().filter(|&&v| v > 8).count() as f64 / d.len() as f64 * 100.0
            );
            for (name, c) in ["dome", "far wall"].iter().zip(far) {
                let (a, b) = (crop(last, w, c), crop(&r, w, c));
                let dc = aliasing::levels(&a, &b);
                let (rh_, rv_) = neighbour_energy(&r, w, c);
                println!(
                    "fold far crop {name} distance from R: mean {:.3} levels, over 8 {:.2} %; R's energy mean {:.5}",
                    dc.iter().map(|&v| f64::from(v)).sum::<f64>() / dc.len() as f64,
                    dc.iter().filter(|&&v| v > 8).count() as f64 / dc.len() as f64 * 100.0,
                    (rh_ + rv_) / 2.0
                );
            }
            let d32: Vec<u32> = d.iter().map(|&v| u32::from(v)).collect();
            let whole = ByClass::of(mask, &d32);
            let far_d = ByClass::of_where(mask, &d32, &in_far);
            println!(
                "fold distance from R by class:\n{}",
                whole.table("distance")
            );
            println!(
                "fold distance from R by class, far crops:\n{}",
                far_d.table("distance")
            );
            let (row, share) = far_d.largest_split_row();
            let (lit, lit_share) = far_d.largest_interior_split();
            println!(
                "fold far crops' largest distance row: {row} {:.1} %; with interior split alone: {lit} {:.1} %; at a direction change (any class) {:.1} %",
                share * 100.0,
                lit_share * 100.0,
                far_d.at_total() as f64 / far_d.total().max(1) as f64 * 100.0
            );
        }
    }

    /// The direction map of the last frame, `dirmap.png`, and its edge ratio by `t` on the
    /// frame, on each far crop and on the far crops together.
    fn directions(&self, scene: &Scene, frame: &Frame, camera: &Camera, f: &Folded) {
        let [w, h] = self.size;
        let internal = frame.internal_size();
        let last = f.frames.last().expect("a frame");
        let far = far_crops(w, h);
        let (pos, info) = read_hits(scene, frame);
        let hits = Hits {
            size: internal,
            pos: &pos,
            info: &info,
        };
        let dirs = aliasing::directions(&hits, [w, h], camera.position.to_array());
        let at = aliasing::changes(&dirs, [w, h]);
        let z = aliasing::z_levels(&hits, [w, h]);
        write_png(
            &self.dir.join("dirmap.png"),
            &aliasing::direction_image(&dirs),
            w,
            h,
        );
        // The band's start: a voxel `band_hi` internal pixels wide.
        let tan_internal = 2.0 * (camera.vfov * 0.5).tan() * w as f32 / h as f32;
        let t_from = internal[0] as f32 / (tan_internal * self.band_hi.max(1e-3));
        print!(
            "{}",
            aliasing::DirEdges::read(last, &dirs, &at, &z, w, [0, 0, w, h], t_from).lines("frame")
        );
        for (name, c) in ["dome", "far wall"].iter().zip(far) {
            print!(
                "{}",
                aliasing::DirEdges::read(last, &dirs, &at, &z, w, c, t_from).lines(name)
            );
        }
        let mut fc = aliasing::DirEdges::default();
        for c in far {
            let r = aliasing::DirEdges::read(last, &dirs, &at, &z, w, c, t_from);
            for (k, b) in r.by_t.iter().enumerate() {
                if fc.by_t.len() <= k {
                    fc.by_t.resize(k + 1, [0; 4]);
                }
                (0..4).for_each(|j| fc.by_t[k][j] += b[j]);
            }
            (0..4).for_each(|j| fc.beyond[j] += r.beyond[j]);
            fc.t_from = r.t_from;
        }
        print!("{}", fc.lines("far crops"));
    }

    /// `--rings`: the rings on that crop of the last frame, and the z-level images.
    fn rings(&self, scene: &Scene, frame: &Frame, f: &Folded) {
        let Some(c) = self.rings else {
            return;
        };
        let [w, h] = self.size;
        let last = f.frames.last().expect("a frame");
        let (pos, info) = read_hits(scene, frame);
        let hits = Hits {
            size: frame.internal_size(),
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
}

/// R1's CPU half: the hall floor's terraces over the visible floor, x and y in 192..704 and
/// 192..576 (whole 64-voxel cells), from the showcase's floor rule (`FLOOR_Z` plus the seed-1
/// floor noise at cell 64, shifted down 11): terrace widths along rows and columns, the riser
/// heights, and the lattice corners that are extrema of their 8 neighbours with the floor on a
/// ring of radius 32 around them all below (a maximum) or all above (a minimum), so that every
/// level between closes around the corner.
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

/// The strip's frame: its sizes and upscaler, with the jitter's phase count a fold reads.
fn make_frame(args: &Args, device: &wgpu::Device, queue: &wgpu::Queue) -> (Frame, u32) {
    let [w, h] = args.size;
    let output = args.size;
    let shrink = args.shrink;
    assert!(
        shrink == 1 || args.upscaler == "off",
        "--shrink renders the native frame larger: --upscaler off"
    );
    let frame = match args.upscaler {
        "off" => Frame::new(device, w * shrink, h * shrink),
        "own" => {
            let mut f = Frame::with_sizes(device, args.scale.internal(output), output);
            f.set_jitter(device, true);
            f.set_sharpness(queue, args.sharpen);
            f
        }
        _ => {
            let mut f = Frame::with_sizes(device, args.scale.internal(output), output);
            f.set_upscaler_kind(
                device,
                UpscalerKind::Fsr2(Fsr2Settings {
                    sharpness: args.sharpen,
                    jitter_phases: args.jitter_phases,
                    dither: args.dither,
                    ..Fsr2Settings::default()
                }),
            );
            f
        }
    };
    let internal = frame.internal_size();
    let phases = match args.upscaler {
        "fsr2" => frame
            .fsr2()
            .map_or(0, |f| f.settings().phases(internal[0], w)),
        "own" => vxe_render::internals::JITTER_PHASES,
        _ => 18,
    };
    (frame, args.phases.unwrap_or(phases))
}

/// The strip's lighting and light, set on the frame with its probe spacing, light resolution
/// and look; the probe spacing is returned for the banner.
fn light_frame(
    args: &Args,
    scene: &Scene,
    frame: &mut Frame,
    preset: &LookPreset,
) -> (Lighting, LightParams, u32) {
    let (device, queue) = (&scene.device, &scene.queue);
    let mut lighting = Lighting {
        lamp: args.lamp.unwrap_or(scene.lamp),
        lamp_bounce: args.indirect,
        filter: args.filter,
        candidates: args.candidates,
        shadow_walk: args.shadow,
        exposure: args.exposure.unwrap_or(Exposure::Manual(scene.exposure)),
        store_read: args.store_read,
        light_weights: args.weights,
        ..Lighting::default()
    };
    let light = match args.light {
        None | Some(LightMode::Hero | LightMode::Baked) => LightParams::headlamp(),
        Some(LightMode::Eye | LightMode::Eye0) => {
            lighting = aliasing::eye_lighting(lighting);
            aliasing::eye_lamp(args.lamp_power)
        }
    };
    if args.light == Some(LightMode::Eye0) {
        for &m in &scene.emits {
            frame.set_emission_scale(queue, m, 0.0);
        }
    }
    let baked = args.light == Some(LightMode::Baked);
    let spacing = args
        .probe_spacing
        .unwrap_or(if args.light.is_some() && !baked {
            HERO_PROBE_SPACING
        } else {
            vxe_render::DEFAULT_PROBE_SPACING
        });
    frame.set_probe_spacing(device, spacing);
    if let Some(r) = args.lighting {
        frame.set_resolution(device, r);
    }
    frame.set_lighting(lighting);
    frame.set_look(LookParams {
        normal_mode: preset.normal_mode,
        view: args.view,
        tuning: preset.tuning,
        far_normal: preset.far_normal,
        ..LookParams::default()
    });
    frame.set_dt(DT);
    (lighting, light, spacing)
}

/// The strip's run: the frame, the light, the settled camera and where the run writes.
struct Run {
    frame: Frame,
    lighting: Lighting,
    light: LightParams,
    camera: Camera,
    phases: u32,
    dir: PathBuf,
}

fn strip_run(args: &Args) {
    let mut preset = args.look.unwrap_or(LookPreset::DEFAULT);
    let look_source = if args.look.is_some() {
        ""
    } else {
        " (default)"
    };
    if let Some(f) = args.far {
        preset.far_normal = f;
    }
    let Some(mut scene) = scene(args) else {
        println!("strip: no adapter; skipped");
        return;
    };
    if let Some(c) = args.camera {
        scene.camera = Camera {
            position: [c[0], c[1], c[2]].into(),
            yaw: c[3].to_radians(),
            pitch: c[4].to_radians(),
            vfov: 70f32.to_radians(),
        };
    }
    let (mut frame, phases) = make_frame(args, &scene.device, &scene.queue);
    let (lighting, light, spacing) = light_frame(args, &scene, &mut frame, &preset);
    if args.light == Some(LightMode::Baked) {
        bake_view(&mut scene, &mut frame, args.bake_radius);
    }
    let [w, h] = args.size;
    let internal = frame.internal_size();
    let tag = &args.tag;
    let dir = out_dir(tag);
    println!(
        "strip {tag}: {} | {w}x{h} {} | look {} {}{look_source} | lamp {} indirect {} filter {} candidates {} exposure {:?} | settle {} frames {} every {} speed {} yaw {} lamp_at {:?} crop {:?} flicker {:?}",
        scene.name,
        args.upscaler,
        preset.name,
        LookPreset::far_normal_words(preset.normal_mode, preset.far_normal),
        lighting.lamp,
        lighting.lamp_bounce,
        lighting.filter,
        lighting.candidates,
        lighting.exposure,
        args.settle,
        args.frames,
        args.every,
        args.speed,
        args.yaw,
        args.lamp_at,
        args.crop,
        args.flicker_window(),
    );
    println!(
        "strip {tag}: light {} (lamp power {}, light radius {}, light samples {}), probe spacing {spacing}, sharpen {}, internal {}x{}, jitter phases {phases}",
        args.light.map_or("hero", LightMode::name),
        light.intensity,
        scene.light_radius,
        scene.gpu.light_samples(),
        args.sharpen,
        internal[0],
        internal[1]
    );
    let camera = scene.camera;
    for _ in 0..args.settle {
        frame.step(&scene.device, &scene.queue, &scene.gpu, &camera, &light);
    }
    let mut run = Run {
        frame,
        lighting,
        light,
        camera,
        phases,
        dir,
    };
    if let Some(rate) = args.pan {
        let read = PanRead {
            size: args.size,
            shrink: args.shrink,
            rate,
            frames: args.frames,
            dots: args.dots,
            tag,
            dir: &run.dir,
            energy: args.energy.as_deref(),
        };
        read.run(&mut scene, &mut run.frame, &mut run.camera, &run.light);
        return;
    }
    match args.read {
        Some("drift") => drift(args, &scene, &mut run),
        Some(_) => {
            let read = FoldRead {
                size: args.size,
                shrink: args.shrink,
                phases: run.phases,
                frames: args.frames,
                band_hi: args.band_hi.unwrap_or(preset.far_normal[1]),
                tag,
                dir: &run.dir,
                energy: args.energy.as_deref(),
                refs: &args.refs,
                rings: args.rings,
            };
            read.run(&mut scene, &mut run.frame, &run.camera, &run.light);
        }
        None => strip_frames(args, &mut scene, &mut run),
    }
}

/// `--read drift`: step `settle + 1` against step `--drift-to`.
fn drift(args: &Args, scene: &Scene, run: &mut Run) {
    let [w, h] = args.size;
    let (device, queue) = (&scene.device, &scene.queue);
    run.frame
        .step(device, queue, &scene.gpu, &run.camera, &run.light);
    let a = display(scene, &run.frame, args.shrink);
    let to = args.drift_to;
    for _ in args.settle + 1..to {
        run.frame
            .step(device, queue, &scene.gpu, &run.camera, &run.light);
    }
    let b = display(scene, &run.frame, args.shrink);
    let mut hist = aliasing::Hist::default();
    aliasing::levels(&a, &b)
        .into_iter()
        .for_each(|d| hist.add(d));
    let c = hist.change();
    let over_8 =
        aliasing::levels(&a, &b).iter().filter(|&&d| d > 8).count() as f64 / (w * h) as f64;
    println!(
        "drift step {} vs {to}: {}; over 8 {:.2} %",
        args.settle + 1,
        c.cell(),
        over_8 * 100.0
    );
}

/// What the plain strip gathers over its run: the per-pixel flicker sum, the sheet's panels,
/// each frame's mean `|dY|` and the mean image's sum.
struct StripAcc {
    flick: Vec<f32>,
    flick_frames: u32,
    panels: Vec<Vec<u8>>,
    frame_flicker: Vec<f32>,
    mean_acc: Vec<f32>,
}

/// The plain strip: `--frames` frames along the scripted camera, each one's mean luma and
/// `|dY|` printed, every `--every`th written, then the sheet, the mean and the flicker map.
fn strip_frames(args: &Args, scene: &mut Scene, run: &mut Run) {
    let [w, h] = args.size;
    let n = (w * h) as usize;
    let flicker_window = args.flicker_window();
    let in_window = |f: u32| (flicker_window.0..=flicker_window.1).contains(&f);
    let mut prev: Option<Vec<f32>> = None;
    let mut acc = StripAcc {
        flick: vec![0f32; n],
        flick_frames: 0,
        panels: Vec::new(),
        frame_flicker: Vec::new(),
        mean_acc: vec![0f32; n * 4],
    };
    for f in 0..args.frames {
        if args.lamp_at == Some(f) {
            run.lighting.lamp = !run.lighting.lamp;
            run.frame.set_lighting(run.lighting);
            println!(
                "frame {f}: lamp {}",
                if run.lighting.lamp { "on" } else { "off" }
            );
        }
        let (device, queue) = (&scene.device, &scene.queue);
        run.frame
            .step(device, queue, &scene.gpu, &run.camera, &run.light);
        let rgba = vxe_render::debug::read_rgba8(device, queue, run.frame.target());
        let y: Vec<f32> = rgba.chunks_exact(4).map(luma).collect();
        let mean = y.iter().sum::<f32>() / n as f32;
        let mut d_mean = 0.0;
        if let Some(p) = &prev {
            let mut d_sum = 0.0;
            for (i, (a, b)) in y.iter().zip(p).enumerate() {
                let d = (a - b).abs();
                d_sum += d;
                if in_window(f) {
                    acc.flick[i] += d;
                }
            }
            d_mean = d_sum / n as f32;
            if in_window(f) {
                acc.flick_frames += 1;
            }
        }
        acc.frame_flicker.push(d_mean);
        if args.mean {
            for (a, &v) in acc.mean_acc.iter_mut().zip(&rgba) {
                *a += f32::from(v);
            }
        }
        let camera = &mut run.camera;
        println!(
            "frame {f:4}: mean luma {mean:.4}, mean |dY| {d_mean:.5}, camera {:.1?} yaw {:.1}",
            camera.position.to_array(),
            camera.yaw.to_degrees()
        );
        if f + 1 == args.frames
            && let Some(crops) = &args.energy
        {
            for &c in crops {
                let (eh, ev) = neighbour_energy(&rgba, w, c);
                println!("energy {c:?}: h {eh:.5} v {ev:.5}");
            }
        }
        if f % args.every == 0 || f + 1 == args.frames {
            write_png(&run.dir.join(format!("f{f:04}.png")), &rgba, w, h);
            if let Some(c) = args.crop {
                write_png(
                    &run.dir.join(format!("f{f:04}-crop.png")),
                    &crop(&rgba, w, c),
                    c[2],
                    c[3],
                );
            }
            acc.panels.push(shrink_image(&rgba, w, h, 4).0);
        }
        prev = Some(y);
        let (fwd, _, _) = camera.basis();
        camera.position += fwd * args.speed * DT;
        camera.yaw += args.yaw.to_radians() * DT;
        if args.speed != 0.0 {
            scene.gpu.set_focus(VoxelCoord(
                camera.position.to_array().map(|v| v.floor() as i32),
            ));
        }
    }
    write_strip(args, &run.dir, &acc);
}

/// The strip's sheet, mean image and flicker map, with the flicker's percentiles and the
/// per-frame `|dY|` series.
fn write_strip(args: &Args, dir: &Path, acc: &StripAcc) {
    let [w, h] = args.size;
    let frames = args.frames;
    let flicker_window = args.flicker_window();
    let (sw, sh) = (w / 4, h / 4);
    let (px, pw, ph) = sheet(&acc.panels, sw, sh, 4);
    write_png(&dir.join("sheet.png"), &px, pw, ph);
    if args.mean && frames > 0 {
        let mean: Vec<u8> = acc
            .mean_acc
            .iter()
            .map(|a| (a / frames as f32).round().clamp(0.0, 255.0) as u8)
            .collect();
        write_png(&dir.join("mean.png"), &mean, w, h);
        if let Some(c) = args.crop {
            write_png(&dir.join("mean-crop.png"), &crop(&mean, w, c), c[2], c[3]);
        }
    }
    let flick_frames = acc.flick_frames;
    if flick_frames > 0 {
        let map: Vec<u8> = acc
            .flick
            .iter()
            .flat_map(|&d| {
                let g = (d / flick_frames as f32 * 8.0 * 255.0).min(255.0) as u8;
                [g, g, g, 255]
            })
            .collect();
        write_png(&dir.join("flicker.png"), &map, w, h);
        if let Some(c) = args.crop {
            write_png(&dir.join("flicker-crop.png"), &crop(&map, w, c), c[2], c[3]);
        }
        let mut sorted: Vec<f32> = acc.flick.iter().map(|d| d / flick_frames as f32).collect();
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
    let series: Vec<String> = acc
        .frame_flicker
        .iter()
        .enumerate()
        .step_by(8)
        .map(|(f, d)| format!("{f}:{d:.4}"))
        .collect();
    println!("frame |dY| every 8: {}", series.join(" "));
    println!(
        "strip: {} frames written under {}",
        acc.panels.len(),
        dir.display()
    );
}
