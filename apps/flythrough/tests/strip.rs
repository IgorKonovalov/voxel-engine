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
//! - `VXE_STRIP_LOOK`: a `LookPreset` name (default `face`).
//! - `VXE_STRIP_FAR`: `LO,HI`, `LookParams::far_normal` (default: the preset's).
//! - `VXE_STRIP_ENERGY`: crops `x,y,w,h` separated by `/`: the last frame's mean horizontal and
//!   vertical neighbour `|dY|` is printed for each.
//! - `VXE_STRIP_LAMP`: `on` or `off` at the start (default off on the showcase, on on the level).
//! - `VXE_STRIP_INDIRECT`, `VXE_STRIP_FILTER`: `on` (default) or `off`.
//! - `VXE_STRIP_SAMPLER`: `reuse` (default) or `list` (no reservoir reuse).
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
//!
//! Example (the RTX, under the perf lock and the memory cap):
//! `VXE_TEST_ADAPTER=rtx VXE_STRIP_TAG=lamp-off VXE_STRIP_SOURCE=level VXE_STRIP_LAMP_AT=60
//! cargo test --release -p flythrough --test strip -- --ignored --nocapture`.

mod support;

use std::path::PathBuf;

use flythrough::showcase::{SHOTS, ShowcaseSource};
use flythrough::{LookPreset, Pose};
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
        let cfg = MirrorConfig {
            budget_bytes: flythrough::level_mirror_budget(
                &world,
                world.extent().len() as u64,
                false,
            ),
            ..MirrorConfig::new(Window::Fixed(world.extent()))
        };
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
        light_radius: 19,
        ..MirrorConfig::new(Window::Fixed(world.extent()))
    };
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
fn shrink(rgba: &[u8], w: u32, h: u32, k: u32) -> (Vec<u8>, u32, u32) {
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

#[test]
#[ignore = "reading only: run with --ignored"]
fn strip_run() {
    let look = env("VXE_STRIP_LOOK").unwrap_or_else(|| "face".to_owned());
    let mut preset = LookPreset::from_name(&look).expect("VXE_STRIP_LOOK takes a look preset");
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
    let mut frame = match upscaler.as_str() {
        "off" => Frame::new(device, w, h),
        "own" => {
            let mut f = Frame::with_sizes(device, RenderScale::OneHalf.internal(output), output);
            f.set_jitter(device, true);
            f.set_sharpness(queue, 0.5);
            f
        }
        "fsr2" => {
            let mut f = Frame::with_sizes(device, RenderScale::OneHalf.internal(output), output);
            f.set_upscaler_kind(
                device,
                UpscalerKind::Fsr2(Fsr2Settings {
                    sharpness: 0.5,
                    ..Fsr2Settings::default()
                }),
            );
            f
        }
        v => panic!("VXE_STRIP_UPSCALER takes fsr2, own or off, not {v}"),
    };
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
        indirect: on_off("VXE_STRIP_INDIRECT", true),
        filter: on_off("VXE_STRIP_FILTER", true),
        sampler: match env("VXE_STRIP_SAMPLER").as_deref() {
            Some("list") => vxe_render::Sampler::List,
            Some("reuse") | None => vxe_render::Sampler::Reuse,
            Some(v) => panic!("VXE_STRIP_SAMPLER takes reuse or list, not {v}"),
        },
        candidates: env_or("VXE_STRIP_CANDIDATES", 8),
        shadow_walk: match env("VXE_STRIP_SHADOW").as_deref() {
            Some("full") => vxe_render::ShadowWalk::Full,
            Some("coarse") | None => vxe_render::ShadowWalk::Coarse,
            Some(v) => panic!("VXE_STRIP_SHADOW takes coarse or full, not {v}"),
        },
        exposure,
        ..Lighting::default()
    };
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
        "strip {tag}: {} | {w}x{h} {upscaler} | look {} {} | lamp {} indirect {} filter {} sampler {:?} candidates {} exposure {:?} | settle {settle} frames {frames} every {every} speed {speed} yaw {yaw_rate} lamp_at {lamp_at:?} crop {crop_box:?} flicker {flicker_window:?}",
        scene.name,
        preset.name,
        LookPreset::far_normal_words(preset.normal_mode, preset.far_normal),
        lighting.lamp,
        lighting.indirect,
        lighting.filter,
        lighting.sampler,
        lighting.candidates,
        lighting.exposure
    );

    let light = LightParams::headlamp();
    let mut camera = scene.camera;
    for _ in 0..settle {
        frame.step(device, queue, &scene.gpu, &camera, &light);
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
            panels.push(shrink(&rgba, w, h, 4).0);
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
