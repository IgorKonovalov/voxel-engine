//! The outdoor source's bars that open a device: the in-process source switch leaves nothing
//! of the outdoor world behind (headless runs of the fly-through binary on lavapipe), and a
//! tree's shadow travels with the sun. The readings in `mod reading` run by hand.
#![allow(
    clippy::disallowed_methods,
    reason = "the runs are bounded by wall-clock time"
)]

mod support;

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The stdout of a headless run on lavapipe at 320 x 180 with the live queue's pre-roll at its
/// smallest, the residency at 6 and 8 chunks (the spawn's block of 2 inside it) and `extra`
/// arguments, or `None` with a printed skip when there is no such adapter. Panics past `bound` or
/// on a failed exit, with both streams.
fn run(extra: &[&str], bound: Duration) -> Option<String> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_flythrough"))
        .args([
            "--headless",
            "--adapter",
            "llvmpipe",
            "--render",
            "320x180",
            "--near",
            "0",
            "--coarse",
            "0",
            "--residency",
            "6,8",
        ])
        .args(extra)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the fly-through starts");
    let t = Instant::now();
    while child.try_wait().expect("waits").is_none() {
        if t.elapsed() > bound {
            let _ = child.kill();
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let out = child.wait_with_output().expect("the run's output");
    let (stdout, stderr) = (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    );
    if !out.status.success() && stderr.contains("no adapter") {
        if std::env::var("VXE_REQUIRE_SOFTWARE_ADAPTER").as_deref() == Ok("1") {
            panic!("VXE_REQUIRE_SOFTWARE_ADAPTER=1 but no adapter: {stderr}");
        }
        println!("skipped: no adapter ({})", stderr.trim());
        return None;
    }
    assert!(
        out.status.success() && t.elapsed() <= bound,
        "the run failed or ran over {bound:?}; stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    Some(stdout)
}

/// The last line of `out` that starts with `prefix`.
fn last_line<'a>(out: &'a str, prefix: &str) -> &'a str {
    out.lines()
        .rev()
        .find(|l| l.starts_with(prefix))
        .unwrap_or_else(|| panic!("no `{prefix}` line in:\n{out}"))
}

/// A run that starts on `--source outdoor` and switches at frame `at` (the outdoor source's
/// next is the cave) ends, at frame `frames`, with the voxel hash of the chunks within 2 of
/// the cave's spawn chunk equal to a fresh `--source cave` run's, none of their voxels of the
/// outdoor rows' ids, and the new server's op log empty at the switch.
fn switch_leaves_nothing_behind(at: u32, frames: u32) {
    let bound = Duration::from_secs(300);
    let (frames, at) = (frames.to_string(), at.to_string());
    let common = ["--frames", frames.as_str(), "--spawn-hash"];
    let Some(switched) = run(
        &[&common[..], &["--source", "outdoor", "--switch-at", &at]].concat(),
        bound,
    ) else {
        return;
    };
    let fresh = run(&[&common[..], &["--source", "cave"]].concat(), bound)
        .expect("the adapter of the first run");
    // Frames 0 to `at` were drawn on the outdoor source.
    let drawn = at.parse::<u32>().expect("a frame") + 1;
    assert_eq!(
        last_line(&switched, "switch: "),
        format!("switch: source outdoor to cave after {drawn} frames, op log 0")
    );
    let hash = last_line(&switched, "spawn hash");
    assert!(hash.starts_with("spawn hash 0x"), "{hash}");
    assert!(hash.ends_with(", 0 voxels of ids 5 and 6"), "{hash}");
    assert_eq!(hash, last_line(&fresh, "spawn hash"));
}

/// Plan 0032 Phase 3 bar 6: the switch at frame 200 of 600.
#[test]
#[ignore = "soak"]
fn a_source_switch_leaves_nothing_behind() {
    switch_leaves_nothing_behind(200, 600);
}

/// The smaller variant: the switch at frame 20 of 60. Three world loads (outdoor, the
/// switched cave and a fresh cave) dominate its time, over the soak line on lavapipe.
#[test]
#[ignore = "soak"]
fn a_source_switch_leaves_nothing_behind_in_60_frames() {
    switch_leaves_nothing_behind(20, 60);
}

/// Plan 0032 Phase 8 bar 5: one tree from the builder (trunk radius 2 to z = 80, a solid
/// canopy of radius 12 at z = 80) over a rock floor at z = 0, the camera straight down from
/// z = 250.5 at 640 x 360, Phase 5's conditions (lamp off, no list light, the sky rays and the
/// filter off, manual exposure 1, `E_sun = (1, 1, 1)` at the real radius, 16-frame means, the
/// full walk): the centroid of the occlusion `w = 1 - I / I_open` away from the shadow's axis
/// lies at (0.5, 46.98) under a sun at 60/180 and at (139.93, 0.5) under one at 30/270, each
/// within 1.5 voxels on each axis, and the sum of `w` at 30 over 60 is `sin 60 / sin 30 =
/// 1.732` within 10 %. Over the soak line; the gate keeps the 320 x 180 variant.
#[test]
#[ignore = "soak"]
fn a_tree_shadow_travels_with_the_sun() {
    tree_shadow_bar([640, 360], 1.5);
}

/// Bar 5 at 320 x 180 (1.52 voxels a pixel at the floor), each centroid within 3 voxels.
#[test]
fn a_tree_shadow_travels_with_the_sun_at_320x180() {
    tree_shadow_bar([320, 180], 3.0);
}

fn tree_shadow_bar(size: [u32; 2], tolerance: f64) {
    let Some(r) = reading::tree_shadow(&[vxe_render::ShadowWalk::Full], size) else {
        return;
    };
    for (got, want) in [(r[0], [0.5, 46.98]), (r[1], [139.93, 0.5])] {
        assert!(
            (0..2).all(|k| (got[k] - want[k]).abs() <= tolerance),
            "{got:?} against {want:?}"
        );
    }
    assert!((r[2][0] / 1.732 - 1.0).abs() <= 0.10, "ratio {}", r[2][0]);
}

mod reading {
    use super::support;
    use support::stillness::{Pose, SIZE, outdoor_level_on, run_under_sky};
    use vxe_render::{AutoExposure, Exposure, Frame, LightParams, Lighting};

    /// Plan 0032 Phase 7 bar 4: the stillness rig's statistics at `outdoor_noon`'s pose (the
    /// `outside` spawn facing the mouth, 12:00, the day cycle off, the lamp off) under a manual
    /// exposure pinned at what the outdoor auto exposure (instant adaptation, the outdoor
    /// floor and ceiling) reads after 64 frames of the same pose. Prints them; no bar.
    /// `VXE_TEST_ADAPTER` picks the adapter (unset: lavapipe).
    #[test]
    #[ignore = "reading"]
    fn outdoor_noon_stillness() {
        let Some((mut level, eye)) = outdoor_level_on(support::reading_device) else {
            return;
        };
        let sky = flythrough::daylight::Clock::default().sky(flythrough::outdoor::BASE_Z as f32);
        let base = flythrough::default_lighting();
        let pose = |exposure: f32| Pose {
            name: "outdoor_noon",
            position: eye.0.map(|v| v as f32 + 0.5),
            yaw_deg: 0.0,
            pitch_deg: 0.0,
            exposure,
        };
        // The exposure to pin: the outdoor auto exposure's after 64 frames.
        let mut frame = Frame::new(&level.device, SIZE[0], SIZE[1]);
        frame.set_lighting(Lighting {
            lamp: false,
            exposure: Exposure::Auto(AutoExposure {
                min: flythrough::cli::OUTDOOR_MIN,
                max: flythrough::cli::OUTDOOR_MAX,
                ..AutoExposure::instant()
            }),
            ..base
        });
        frame.set_sky(Some(sky));
        frame.set_dt(1.0 / 60.0);
        let camera = pose(1.0).camera();
        for _ in 0..64 {
            frame.step(
                &level.device,
                &level.queue,
                &level.gpu,
                &camera,
                &LightParams::headlamp(),
            );
        }
        let exposure = frame
            .exposure_readout(&level.device)
            .expect("the exposure read back")
            .exposure;
        println!(
            "reading: outdoor_noon on {}, pinned exposure {exposure:.6e} (auto after 64 frames)",
            level.adapter
        );
        let r = run_under_sky(&mut level, &pose(exposure), base, SIZE, sky);
        r.print("outdoor_noon");
    }

    /// Plan 0032 Phase 8 bar 5's rig: a 512 x 384 rock floor (what the view's floor spans) whose
    /// lit plane is z = 0 and one tree from the builder (trunk radius 2 on column (0, 0) up to z =
    /// 80, a solid canopy of radius 12 on voxel (0, 0, 80)), the camera at (0.5, 0.5, 250.5)
    /// looking straight down at `size`; lamp off, no list light, the sky rays and the filter off,
    /// manual exposure 1, `E_sun = (1, 1, 1)` at the real angular radius, the direct target's
    /// luminance a 16-frame mean per lighting pixel. Each floor pixel's occlusion `w = 1 - I /
    /// I_open` against the same pose without the tree; the centroid of `w`, at the pixels' hit
    /// positions, over floor pixels at least 4 voxels to either side of the shadow's axis; at
    /// 60/180 against (0.5, 46.98) and 30/270 against (139.93, 0.5); and the sum of `w` at 30 over
    /// 60 against `sin 60 / sin 30 = 1.732`, under each of `walks`: per walk the two centroids,
    /// then `[ratio, 0]`. `None` without an adapter. `VXE_TEST_ADAPTER` picks the adapter (unset:
    /// lavapipe).
    pub(super) fn tree_shadow(
        walks: &[vxe_render::ShadowWalk],
        size: [u32; 2],
    ) -> Option<Vec<[f64; 2]>> {
        use flythrough::trees::{TRUNK_SINK, TreeShape, build_tree};
        use support::SyncNow;
        use vxe_render::sky::{Body, SUN_ANGULAR_RADIUS, Sky};
        use vxe_render::{
            Camera, Frame, GpuWorld, LightingResolution, MirrorConfig, ShadowWalk, Window,
            glam::Vec3,
        };
        use vxe_world::{AIR, ChunkExtent, FnSource, World};
        let shape = TreeShape {
            trunk_r: 2,
            height: 80,
            canopy_r: 12,
        };
        let tree = build_tree(shape, 0, 256);
        let lo = [-12, -12, -TRUNK_SINK];
        let world_of = |with_tree: bool| {
            let tree = &tree;
            let f = move |p: [i32; 3]| {
                if with_tree && let Some(m) = tree.get([0, 1, 2].map(|k| p[k] - lo[k])) {
                    return m;
                }
                if p[2] < 0 {
                    flythrough::cave::ROCK
                } else {
                    AIR
                }
            };
            World::generate(
                // The view's floor (half-extents of 243 and 137 voxels) and the tree.
                ChunkExtent::new([-4, -3, -1], [4, 3, 2]),
                &FnSource(f),
                &flythrough::outdoor_materials(),
            )
        };
        let worlds = [world_of(false), world_of(true)];
        let pool =
            flythrough::level_pool_units(&worlds[1], false) * vxe_render::internals::UNIT_BYTES;
        let (device, queue, adapter) = support::reading_device(pool)?;
        let gpus: Vec<GpuWorld> = worlds
            .iter()
            .map(|w| {
                let mut gpu =
                    GpuWorld::with_config(&device, MirrorConfig::new(Window::Fixed(w.extent())));
                while gpu
                    .sync_now(w, &device, &queue)
                    .expect("the mirror fits")
                    .pending_chunks
                    > 0
                {}
                gpu
            })
            .collect();
        let camera = Camera {
            position: Vec3::new(0.5, 0.5, 250.5),
            // Facing +y before the pitch, so +y runs up the screen and x across it.
            yaw: std::f32::consts::FRAC_PI_2,
            pitch: -std::f32::consts::FRAC_PI_2 + 1e-4,
            vfov: 1.0,
        };
        let dir = |el: f32, az: f32| {
            let (e, a) = (el.to_radians(), az.to_radians());
            [e.cos() * a.sin(), e.cos() * a.cos(), e.sin()]
        };
        // Per lighting pixel: the 16-frame mean luminance of the direct target and the hit.
        // One frame for every run: its pipelines are built once.
        let mut frame = Frame::new(&device, size[0], size[1]);
        frame.set_resolution(&device, LightingResolution::Full);
        let mut read = |gpu: &GpuWorld, sun: [f32; 3], walk: ShadowWalk| {
            frame.set_lighting(Lighting {
                lamp: false,
                filter: false,
                shadow_walk: walk,
                exposure: Exposure::Manual(1.0),
                sky_rays: 0,
                ..Lighting::default()
            });
            let body = |direction| Body {
                direction,
                illuminance: [1.0; 3],
                angular_radius: SUN_ANGULAR_RADIUS,
            };
            frame.set_sky(Some(Sky {
                altitude_zero_z: 0.0,
                ..Sky::new(body(sun), body(sun.map(|c| -c)))
            }));
            let n = (size[0] * size[1]) as usize;
            let mut sum = vec![0.0f64; n];
            for _ in 0..16 {
                frame.step(&device, &queue, gpu, &camera, &LightParams::headlamp());
                let d = vxe_render::debug::read_rgba16f(&device, &queue, frame.direct());
                for (s, p) in sum.iter_mut().zip(&d) {
                    *s += f64::from(0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]) / 16.0;
                }
            }
            let hits =
                vxe_render::debug::read_rgba32f(&device, &queue, frame.light_hit_positions());
            (sum, hits)
        };
        let mut out = Vec::new();
        for &walk in walks {
            let mut w_sums = Vec::new();
            for (el, az, want) in [
                (60.0f32, 180.0f32, [0.5, 46.98]),
                (30.0, 270.0, [139.93, 0.5]),
            ] {
                let sun = dir(el, az);
                let (open, _) = read(&gpus[0], sun, walk);
                let (lit, hits) = read(&gpus[1], sun, walk);
                // The shadow's axis: from (0.5, 0.5) away from the sun.
                let away = [-sun[0], -sun[1]];
                let len = (away[0] * away[0] + away[1] * away[1]).sqrt();
                let away = away.map(|c| f64::from(c / len));
                let (mut sw, mut sx, mut sy) = (0.0f64, 0.0f64, 0.0f64);
                for i in 0..open.len() {
                    let h = hits[i];
                    if h[3] <= 0.0 || h[2].abs() > 0.01 || open[i] <= 0.0 {
                        continue;
                    }
                    let (x, y) = (f64::from(h[0]) - 0.5, f64::from(h[1]) - 0.5);
                    if (x * away[1] - y * away[0]).abs() < 4.0 {
                        continue;
                    }
                    let w = 1.0 - lit[i] / open[i];
                    sw += w;
                    sx += w * f64::from(h[0]);
                    sy += w * f64::from(h[1]);
                }
                let c = [sx / sw, sy / sw];
                println!(
                    "reading: tree shadow on {adapter}, {} walk, sun {el}/{az}: centroid ({:.2}, {:.2}), want ({}, {}), off ({:+.2}, {:+.2}); sum of w {sw:.1}",
                    walk.name(),
                    c[0],
                    c[1],
                    want[0],
                    want[1],
                    c[0] - want[0],
                    c[1] - want[1],
                );
                w_sums.push(sw);
                out.push(c);
            }
            println!(
                "reading: tree shadow, {} walk: sum of w at 30 over 60 {:.3}, want 1.732",
                walk.name(),
                w_sums[1] / w_sums[0]
            );
            out.push([w_sums[1] / w_sums[0], 0.0]);
        }
        Some(out)
    }

    /// Plan 0032 Phase 8 bar 5 under the coarse walk, printed (the bar is on the full walk).
    /// `VXE_TEST_ADAPTER` picks the adapter (unset: lavapipe).
    #[test]
    #[ignore = "reading"]
    fn a_tree_shadow_travels_with_the_sun_under_the_coarse_walk() {
        tree_shadow(&[vxe_render::ShadowWalk::Coarse], [640, 360]);
    }

    /// Plan 0032 Phase 10 bar 3, as a reading: the outdoor hills of seed 1 streamed in
    /// process at residency 6 and 8 chunks (hysteresis 1, 4 workers, a following window of
    /// side 19) around the `outside` spawn, facing the mouth at noon (the cycle off, the lamp
    /// off, manual exposure 1, otherwise the fly-through's default lighting), 320 x 180. The
    /// spawn's block (the chunks within 1 of its chunk) is streamed and mirrored first, as the
    /// binary's loading screen waits for it; from frame 0 the radii are 6 and 8 and each frame
    /// updates, polls and syncs the stream with no time budget, then renders. At
    /// frames 2, 30 and 120 it prints the direct target's mean luminance over the farthest
    /// third of the lighting grid's hit pixels (by `t`), with the mirrored chunk count. No
    /// bar. `VXE_TEST_ADAPTER` picks the adapter (unset: lavapipe).
    #[test]
    #[ignore = "reading"]
    fn the_far_ground_is_lit_while_the_stream_fills() {
        use flythrough::outdoor::{OutdoorSource, Spawn};
        use std::sync::Arc;
        use support::SyncNow;
        use vxe_render::{GpuWorld, LodParams, MirrorConfig, Window};
        use vxe_world::{PersistConfig, Residency, StreamConfig, Streamer, World};
        const SIZE: [u32; 2] = [320, 180];
        const READS: [u32; 3] = [2, 30, 120];
        let residency = Residency {
            r_full: 6,
            r_lod: 8,
            hysteresis: 1,
        };
        let side = 2 * (residency.r_lod + residency.hysteresis) + 1;
        let budget = 256u64 << 20;
        let Some((device, queue, adapter)) = support::reading_device(budget) else {
            return;
        };
        let source = OutdoorSource::new(1);
        let (eye, _) = source.spawn(Spawn::Outside);
        let world = World::new(&flythrough::outdoor_materials());
        let mut streamer = Streamer::new(
            Arc::new(source),
            StreamConfig {
                residency,
                workers: 4,
                persist: PersistConfig::default(),
            },
        );
        let mut gpu = GpuWorld::with_config(
            &device,
            MirrorConfig {
                budget_bytes: budget,
                ..MirrorConfig::new(Window::Follow { side })
            },
        );
        // The spawn's block first (radius 1, as the loading screen waits for it), then the
        // full radii from frame 0, so the frames see the shell arrive.
        streamer.set_radii(1, 1);
        streamer.update(&world, &[eye]);
        let ev = streamer.wait_idle(&world);
        gpu.apply_events(&ev);
        gpu.set_focus(eye);
        gpu.sync_now(&world, &device, &queue)
            .expect("the mirror fits");
        streamer.set_radii(residency.r_full, residency.r_lod);
        let mut stream = |gpu: &mut GpuWorld| {
            streamer.update(&world, &[eye]);
            let ev = streamer.poll(&world);
            gpu.apply_events(&ev);
            gpu.set_focus(eye);
            let mut enc = device.create_command_encoder(&Default::default());
            gpu.sync(&world, &queue, &mut enc).expect("the mirror fits");
            queue.submit([enc.finish()]);
        };
        let mut frame = Frame::new(&device, SIZE[0], SIZE[1]);
        frame.set_lod(LodParams {
            r_full: residency.r_full,
            forced_level: None,
        });
        frame.set_lighting(Lighting {
            lamp: false,
            exposure: Exposure::Manual(1.0),
            ..flythrough::default_lighting()
        });
        frame.set_sky(Some(
            flythrough::daylight::Clock::default().sky(flythrough::outdoor::BASE_Z as f32),
        ));
        frame.set_dt(1.0 / 60.0);
        let camera = flythrough::Pose {
            name: "outside",
            position: eye.0.map(|v| v as f32 + 0.5),
            yaw_deg: 0.0,
            pitch_deg: 0.0,
            exposure: 1.0,
        }
        .camera();
        for f in 0..=READS[2] {
            stream(&mut gpu);
            frame.step(&device, &queue, &gpu, &camera, &LightParams::headlamp());
            if !READS.contains(&f) {
                continue;
            }
            let d = vxe_render::debug::read_rgba16f(&device, &queue, frame.direct());
            let h = vxe_render::debug::read_rgba32f(&device, &queue, frame.light_hit_positions());
            let mut ground: Vec<(f32, f32)> = h
                .iter()
                .zip(&d)
                .filter(|(h, _)| h[3] < 1.0e30)
                .map(|(h, p)| (h[3], 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]))
                .collect();
            ground.sort_by(|a, b| a.0.total_cmp(&b.0));
            let far = &ground[ground.len() * 2 / 3..];
            let mean = far.iter().map(|g| f64::from(g.1)).sum::<f64>() / far.len().max(1) as f64;
            println!(
                "reading: far ground on {adapter}, frame {f}: mean luminance {mean:.4} over {} of {} hit pixels (t {:.0} to {:.0}), {} chunks mirrored",
                far.len(),
                ground.len(),
                far.first().map_or(0.0, |g| g.0),
                far.last().map_or(0.0, |g| g.0),
                gpu.mirrored_chunks(),
            );
        }
    }

    /// Plan 0032 Phase 6's restated stop rule, read on the tunnel (the bar is Plan 0086's): the
    /// outdoor hills of seed 1 around the `inside` spawn and the whole tunnel, facing out and
    /// facing in. The sky ray's hit rate per lighting pixel is the 64-frame mean of the direct
    /// target over `pi` under `Sky::uniform(1.0)` (one ray, the filter off, the lamp off, exposure
    /// 1); the rim band is the hits at a rate of 0.3 to 0.6, the far floor the hits under 0.05.
    /// Then FSR 2 at 1.5 (320 x 180 to 480 x 270), the default lighting: at 12:00 with the lamp
    /// off, and with no sky and the lamp on, each settled 64 frames under its auto exposure
    /// (instant; the outdoor floor and ceiling under the sky) and then pinned there for 64 frames:
    /// the 99th percentile over each region of the per-pixel change of the displayed image between
    /// consecutive frames (the largest channel, levels of 255). Prints them. `VXE_TEST_ADAPTER`
    /// picks the adapter (unset: lavapipe).
    #[test]
    #[ignore = "reading"]
    fn the_tunnel_under_one_sky_ray() {
        use flythrough::outdoor::{BASE_Z, OutdoorSource, Spawn};
        use support::SyncNow;
        use vxe_render::sky::Sky;
        use vxe_render::{Camera, GpuWorld, MirrorConfig, UpscalerKind, Window};
        use vxe_world::{ChunkExtent, World};
        let source = OutdoorSource::new(1);
        let (eye, facing) = source.spawn(Spawn::Inside);
        // 3 chunks around the spawn's chunk, and on to 10 along +x and 4 down: the whole tunnel
        // and its chamber are resident (a lighting ray that leaves the world is sky).
        let c = eye.chunk().0;
        let extent = ChunkExtent::new(
            [c[0] - 3, c[1] - 3, c[2] - 4],
            [c[0] + 11, c[1] + 4, c[2] + 2],
        );
        let world = World::generate(extent, &source, &flythrough::outdoor_materials());
        let pool = flythrough::level_pool_units(&world, false) * vxe_render::internals::UNIT_BYTES;
        let Some((device, queue, adapter)) = support::reading_device(pool) else {
            return;
        };
        let mut gpu = GpuWorld::with_config(&device, MirrorConfig::new(Window::Fixed(extent)));
        gpu.set_focus(eye);
        // Until every chunk is mirrored: an absent chunk is air, and sky to a lighting ray.
        while gpu
            .sync_now(&world, &device, &queue)
            .expect("the mirror fits")
            .pending_chunks
            > 0
        {}
        let light = LightParams::headlamp();
        let p = eye.0.map(|v| v as f32 + 0.5);
        let d = facing.map(|v| v as f32);
        let out = flythrough::camera_looking_at(p, [p[0] + d[0], p[1] + d[1], p[2] + d[2]]);
        let inward = flythrough::camera_looking_at(p, [p[0] - d[0], p[1] - d[1], p[2] - d[2]]);
        // The hit rate per lighting pixel, and the grid's width.
        let hit_rate = |cam: &Camera| {
            let mut f = Frame::new(&device, 320, 180);
            f.set_lighting(Lighting {
                lamp: false,
                filter: false,
                exposure: Exposure::Manual(1.0),
                sky_rays: 1,
                ..flythrough::default_lighting()
            });
            f.set_sky(Some(Sky {
                altitude_zero_z: BASE_Z as f32,
                ..Sky::uniform(1.0)
            }));
            let mut sum: Vec<f64> = Vec::new();
            for _ in 0..64 {
                f.step(&device, &queue, &gpu, cam, &light);
                let px = vxe_render::debug::read_rgba16f(&device, &queue, f.direct());
                sum.resize(px.len(), 0.0);
                for (s, p) in sum.iter_mut().zip(&px) {
                    *s += f64::from(p[1]) / 64.0 / std::f64::consts::PI;
                }
            }
            let hits = vxe_render::debug::read_rgba32f(&device, &queue, f.light_hit_positions());
            let rate: Vec<Option<f64>> = sum
                .iter()
                .zip(&hits)
                .map(|(s, h)| (h[3] < 1.0e30).then_some(*s))
                .collect();
            (rate, f.direct().width() as usize)
        };
        // The per-pixel changes of the displayed image over 64 still frames, by display pixel.
        let changes = |cam: &Camera, sky: bool| {
            let mut f = Frame::with_sizes(&device, [320, 180], [480, 270]);
            f.set_upscaler_kind(
                &device,
                UpscalerKind::Fsr2(vxe_render::Fsr2Settings::default()),
            );
            let auto = if sky {
                AutoExposure {
                    min: flythrough::cli::OUTDOOR_MIN,
                    max: flythrough::cli::OUTDOOR_MAX,
                    ..AutoExposure::instant()
                }
            } else {
                AutoExposure::instant()
            };
            let base = Lighting {
                lamp: !sky,
                ..flythrough::default_lighting()
            };
            f.set_lighting(Lighting {
                exposure: Exposure::Auto(auto),
                ..base
            });
            f.set_sky(sky.then(|| flythrough::daylight::Clock::default().sky(BASE_Z as f32)));
            f.set_dt(1.0 / 60.0);
            for _ in 0..64 {
                f.step(&device, &queue, &gpu, cam, &light);
            }
            let e = f.exposure_readout(&device).expect("the exposure").exposure;
            f.set_lighting(Lighting {
                exposure: Exposure::Manual(e),
                ..base
            });
            let read = |f: &Frame| vxe_render::debug::read_rgba8(&device, &queue, f.target());
            let mut last = read(&f);
            let mut per_px: Vec<Vec<u8>> = vec![Vec::new(); last.len() / 4];
            for _ in 0..64 {
                f.step(&device, &queue, &gpu, cam, &light);
                let now = read(&f);
                for (i, (a, b)) in now.chunks(4).zip(last.chunks(4)).enumerate() {
                    per_px[i].push((0..3).map(|k| a[k].abs_diff(b[k])).max().unwrap_or(0));
                }
                last = now;
            }
            (per_px, e)
        };
        for (view, cam) in [("facing out", &out), ("facing in", &inward)] {
            let (rate, gw) = hit_rate(cam);
            let gh = rate.len() / gw;
            let rate = &rate;
            let region = |lo: f64, hi: f64| {
                move |x: usize, y: usize| {
                    let (gx, gy) = (x * gw / 480, y * gh / 270);
                    rate[gy * gw + gx].is_some_and(|r| r >= lo && r < hi)
                }
            };
            let (sky_px, sky_e) = changes(cam, true);
            let (lamp_px, lamp_e) = changes(cam, false);
            for (name, lo, hi) in [
                ("rim band 0.3..0.6", 0.3, 0.6),
                ("far floor < 0.05", 0.0, 0.05),
            ] {
                let inside = region(lo, hi);
                let p99 = |px: &[Vec<u8>]| {
                    let mut v: Vec<u8> = (0..px.len())
                        .filter(|&i| inside(i % 480, i / 480))
                        .flat_map(|i| px[i].iter().copied())
                        .collect();
                    v.sort_unstable();
                    (v.get(v.len() * 99 / 100).copied(), v.len() / 64)
                };
                let (s, n) = p99(&sky_px);
                let (l, _) = p99(&lamp_px);
                println!(
                    "reading: tunnel on {adapter}, {view}, {name}: {n} display pixels; p99 change sky {s:?} (exposure {sky_e:.3e}), headlamp {l:?} (exposure {lamp_e:.3e})"
                );
            }
        }
    }

    /// A world mirrored whole for the term matrix: the device, the mirror and the adapter's
    /// name.
    pub(super) struct Rig {
        pub device: vxe_render::wgpu::Device,
        pub queue: vxe_render::wgpu::Queue,
        pub gpu: vxe_render::GpuWorld,
        pub adapter: String,
    }

    /// `world` mirrored whole (an absent chunk is sky to a lighting ray) with the focus at `eye`.
    pub(super) fn mirrored(world: vxe_world::World, eye: vxe_world::VoxelCoord) -> Option<Rig> {
        use support::SyncNow;
        use vxe_render::{GpuWorld, MirrorConfig, Window};
        let pool = flythrough::level_pool_units(&world, false) * vxe_render::internals::UNIT_BYTES;
        let (device, queue, adapter) = support::reading_device(pool)?;
        let cfg = MirrorConfig {
            budget_bytes: 512 << 20,
            ..MirrorConfig::new(Window::Fixed(world.extent()))
        };
        let mut gpu = GpuWorld::with_config(&device, cfg);
        gpu.set_focus(eye);
        while gpu
            .sync_now(&world, &device, &queue)
            .expect("the mirror fits")
            .pending_chunks
            > 0
        {}
        Some(Rig {
            device,
            queue,
            gpu,
            adapter,
        })
    }

    /// The tunnel of seed 1 (`--source outdoor --spawn inside`): 3 chunks around the spawn's
    /// chunk and on to 10 along +x and 4 down, the eye and its view out of the tunnel and into
    /// it.
    pub(super) fn tunnel_rig() -> Option<(Rig, [(&'static str, vxe_render::Camera); 2])> {
        use flythrough::outdoor::{OutdoorSource, Spawn};
        use vxe_world::{ChunkExtent, World};
        let source = OutdoorSource::new(1);
        let (eye, facing) = source.spawn(Spawn::Inside);
        let c = eye.chunk().0;
        let extent = ChunkExtent::new(
            [c[0] - 3, c[1] - 3, c[2] - 4],
            [c[0] + 11, c[1] + 4, c[2] + 2],
        );
        let world = World::generate(extent, &source, &flythrough::outdoor_materials());
        let rig = mirrored(world, eye)?;
        let p = eye.0.map(|v| v as f32 + 0.5);
        let d = facing.map(|v| v as f32);
        let out = flythrough::camera_looking_at(p, [p[0] + d[0], p[1] + d[1], p[2] + d[2]]);
        let inward = flythrough::camera_looking_at(p, [p[0] - d[0], p[1] - d[1], p[2] - d[2]]);
        Some((rig, [("tunnel out", out), ("tunnel in", inward)]))
    }

    /// The forest of seed 1 at `LEAF_FILL` (`--spawn forest --trees forest`): the chunks within
    /// 4 of the spawn's (2 in z), the spawn's camera.
    pub(super) fn forest_rig() -> Option<(Rig, vxe_render::Camera)> {
        use flythrough::outdoor::{OutdoorSource, Spawn};
        use flythrough::trees::{ForestSource, LEAF_FILL, TreeDensity};
        use vxe_world::{ChunkExtent, World};
        let source = ForestSource::new(OutdoorSource::new(1), TreeDensity::Forest, LEAF_FILL);
        let (eye, facing) = source.base().spawn(Spawn::Forest);
        let c = eye.chunk().0;
        let extent = ChunkExtent::new(
            [c[0] - 4, c[1] - 4, c[2] - 2],
            [c[0] + 5, c[1] + 5, c[2] + 3],
        );
        let world = World::generate(extent, &source, &flythrough::outdoor_materials());
        let rig = mirrored(world, eye)?;
        let p = eye.0.map(|v| v as f32 + 0.5);
        let d = facing.map(|v| v as f32);
        let cam = flythrough::camera_looking_at(p, [p[0] + d[0], p[1] + d[1], p[2] + d[2]]);
        Some((rig, cam))
    }

    /// The sky ray's hit rate per lighting pixel of a native 320 x 180 frame of `cam` (64 frames
    /// under `Sky::uniform(1.0)`, the filter and the key light off, exposure 1), `None` at a
    /// miss, and the grid's width.
    pub(super) fn hit_rates(rig: &Rig, cam: &vxe_render::Camera) -> (Vec<Option<f64>>, usize) {
        use vxe_render::sky::Sky;
        let mut f = Frame::new(&rig.device, 320, 180);
        f.set_lighting(Lighting {
            lamp: false,
            filter: false,
            exposure: Exposure::Manual(1.0),
            sky_rays: 1,
            ..flythrough::default_lighting()
        });
        f.set_sky(Some(Sky {
            altitude_zero_z: flythrough::outdoor::BASE_Z as f32,
            ..Sky::uniform(1.0)
        }));
        let mut sum: Vec<f64> = Vec::new();
        for _ in 0..64 {
            f.step(
                &rig.device,
                &rig.queue,
                &rig.gpu,
                cam,
                &LightParams::headlamp(),
            );
            let px = vxe_render::debug::read_rgba16f(&rig.device, &rig.queue, f.direct());
            sum.resize(px.len(), 0.0);
            for (s, p) in sum.iter_mut().zip(&px) {
                *s += f64::from(p[1]) / 64.0 / std::f64::consts::PI;
            }
        }
        let hits =
            vxe_render::debug::read_rgba32f(&rig.device, &rig.queue, f.light_hit_positions());
        let rate = sum
            .iter()
            .zip(&hits)
            .map(|(s, h)| (h[3] < 1.0e30).then_some(*s))
            .collect();
        (rate, f.direct().width() as usize)
    }

    /// One cell of the term matrix.
    #[derive(Clone, Copy, Debug)]
    pub(super) struct Cell {
        /// "key": `sky_rays` 0; "sky": the key light hooked off; "both".
        pub term: &'static str,
        pub filter: bool,
        /// The history's material and normal tests hooked off.
        pub any_history: bool,
        pub fsr2: bool,
        /// The testbed's `leaf` shadows as its bricks (`--leaf-shadow brick`).
        pub leaf_brick: bool,
    }

    /// What a cell read over its 64 still frames.
    pub(super) struct CellReading {
        /// Per display pixel, its frame-to-frame changes (levels of 255, largest channel).
        pub changes: Vec<Vec<u8>>,
        /// The display's width.
        pub width: usize,
        /// Per frame, the history counters: tested, material, normal, other.
        pub rejections: Vec<[u32; 4]>,
        /// The light pass's direct samples' luminance per frame and lighting pixel.
        pub samples: Vec<Vec<f32>>,
        pub grid_width: usize,
        pub exposure: f32,
    }

    /// `cell` at `cam` under `sky` (the key light and the sky rays at the defaults otherwise, the
    /// lamp off): 64 frames under the outdoor auto exposure (instant), then 64 at that exposure
    /// pinned, reading the display's changes, the history counters and the direct samples.
    pub(super) fn run_cell(
        rig: &Rig,
        cam: &vxe_render::Camera,
        sky: vxe_render::sky::Sky,
        cell: Cell,
    ) -> CellReading {
        use vxe_render::{LightHooks, UpscalerKind};
        let mut f = if cell.fsr2 {
            let mut f = Frame::with_sizes(&rig.device, [320, 180], [480, 270]);
            f.set_upscaler_kind(
                &rig.device,
                UpscalerKind::Fsr2(vxe_render::Fsr2Settings::default()),
            );
            f
        } else {
            Frame::new(&rig.device, 320, 180)
        };
        let base = Lighting {
            lamp: false,
            filter: cell.filter,
            sky_rays: if cell.term == "key" { 0 } else { 1 },
            ..flythrough::default_lighting()
        };
        f.set_light_hooks(LightHooks {
            key_off: cell.term == "sky",
            any_history: cell.any_history,
        });
        f.set_count_store(true);
        if cell.leaf_brick {
            f.set_material_styles(
                &rig.queue,
                &flythrough::outdoor_styles(flythrough::LeafShadow::Brick),
            );
        }
        f.set_lighting(Lighting {
            exposure: Exposure::Auto(AutoExposure {
                min: flythrough::cli::OUTDOOR_MIN,
                max: flythrough::cli::OUTDOOR_MAX,
                ..AutoExposure::instant()
            }),
            ..base
        });
        f.set_sky(Some(sky));
        f.set_dt(1.0 / 60.0);
        let light = LightParams::headlamp();
        for _ in 0..64 {
            f.step(&rig.device, &rig.queue, &rig.gpu, cam, &light);
        }
        let exposure = f
            .exposure_readout(&rig.device)
            .expect("the exposure")
            .exposure;
        f.set_lighting(Lighting {
            exposure: Exposure::Manual(exposure),
            ..base
        });
        let read = |f: &Frame| vxe_render::debug::read_rgba8(&rig.device, &rig.queue, f.target());
        let mut last = read(&f);
        let mut changes: Vec<Vec<u8>> = vec![Vec::new(); last.len() / 4];
        let mut rejections = Vec::new();
        let mut samples = Vec::new();
        for _ in 0..64 {
            f.step(&rig.device, &rig.queue, &rig.gpu, cam, &light);
            let now = read(&f);
            for (i, (a, b)) in now.chunks(4).zip(last.chunks(4)).enumerate() {
                changes[i].push((0..3).map(|k| a[k].abs_diff(b[k])).max().unwrap_or(0));
            }
            last = now;
            rejections.push(f.read_history_rejections(&rig.device, &rig.queue));
            let d = vxe_render::debug::read_rgba16f(&rig.device, &rig.queue, f.direct());
            samples.push(d.iter().map(|p| p[3]).collect());
        }
        CellReading {
            width: f.target().width() as usize,
            changes,
            rejections,
            samples,
            grid_width: f.direct().width() as usize,
            exposure,
        }
    }

    /// Every cell of the matrix: the term (key, sky, both) by the filter (off, on) by the history
    /// (as built, any) by the upscaler (native, FSR 2 at 1.5).
    pub(super) fn cells() -> Vec<Cell> {
        let mut v = Vec::new();
        for fsr2 in [false, true] {
            for term in ["key", "sky", "both"] {
                for filter in [false, true] {
                    for any_history in [false, true] {
                        v.push(Cell {
                            term,
                            filter,
                            any_history,
                            fsr2,
                            leaf_brick: false,
                        });
                    }
                }
            }
        }
        v
    }

    /// A named region of a 320 x 180 frame's pixels.
    pub(super) type Region<'a> = (&'a str, &'a dyn Fn(usize, usize) -> bool);

    /// One table row: the cell, then per region its p99 and mean change, then the region's
    /// firefly rate (direct samples over four times the region's mean sample), then the
    /// history's reset share by cause over the frame's tested hits.
    pub(super) fn print_row(pose: &str, cell: Cell, r: &CellReading, regions: &[Region<'_>]) {
        use support::stillness::change_stats;
        let mut line = format!(
            "{pose:10} | {:4} | filter {:3} | history {:5} | {:6}",
            cell.term,
            if cell.filter { "on" } else { "off" },
            if cell.any_history { "any" } else { "built" },
            if cell.fsr2 { "fsr2" } else { "native" },
        );
        let height = r.changes.len() / r.width;
        let gh = r.samples[0].len() / r.grid_width;
        for (name, inside) in regions {
            let (p99, mean, n) = change_stats(&r.changes, |i| {
                inside(i % r.width * 320 / r.width, i / r.width * 180 / height)
            });
            // The region's direct samples on the lighting grid.
            let grid: Vec<usize> = (0..r.grid_width * gh)
                .filter(|&i| {
                    inside(
                        i % r.grid_width * 320 / r.grid_width,
                        i / r.grid_width * 180 / gh,
                    )
                })
                .collect();
            let all: Vec<f32> = r
                .samples
                .iter()
                .flat_map(|s| grid.iter().map(|&i| s[i]))
                .collect();
            let m = all.iter().map(|&v| f64::from(v)).sum::<f64>() / all.len().max(1) as f64;
            let ff = all
                .iter()
                .filter(|&&v| f64::from(v) > 4.0 * m && m > 0.0)
                .count() as f64
                / all.len().max(1) as f64;
            line += &format!(
                " | {name} p99 {} mean {mean:.2} ({n} px) ff {ff:.4}",
                p99.map_or("-".into(), |v| v.to_string())
            );
        }
        let t: [u64; 4] =
            std::array::from_fn(|k| r.rejections.iter().map(|w| u64::from(w[k])).sum());
        let share = |k: usize| t[k] as f64 / t[0].max(1) as f64;
        line += &format!(
            " | reset {:.4} (material {:.4} normal {:.4} other {:.4}) | exposure {:.3e}",
            share(1) + share(2) + share(3),
            share(1),
            share(2),
            share(3),
            r.exposure
        );
        println!("reading: {line}");
    }

    /// Plan 0086 Phase 1: the flicker split by term. The tunnel (`inside`, seed 1, 12:00, lamp
    /// off) out of and into the tunnel, and the forest (`--trees forest`, `--sun 60,180`), each
    /// through `cells()`: per region (in the tunnel the rim band, the hits at a sky-ray rate of
    /// 0.3 to 0.6, and the far end, under 0.05; in the forest every hit) the p99 and mean
    /// frame-to-frame change of the display, the firefly rate, and the history's reset share
    /// by cause. Prints the table and each region's hit rate. `VXE_TEST_ADAPTER` picks the
    /// adapter (unset: lavapipe).
    #[test]
    #[ignore = "reading"]
    fn flicker_by_term() {
        let noon = flythrough::daylight::Clock::default().sky(flythrough::outdoor::BASE_Z as f32);
        if let Some((rig, views)) = tunnel_rig() {
            println!("reading: flicker_by_term on {}", rig.adapter);
            for (pose, cam) in &views {
                let (rate, gw) = hit_rates(&rig, cam);
                let gh = rate.len() / gw;
                let at = |x: usize, y: usize| rate[(y * gh / 180) * gw + x * gw / 320];
                let rim = |x: usize, y: usize| at(x, y).is_some_and(|r| (0.3..0.6).contains(&r));
                let far = |x: usize, y: usize| at(x, y).is_some_and(|r| r < 0.05);
                for (name, inside) in [
                    ("rim", &rim as &dyn Fn(usize, usize) -> bool),
                    ("far", &far),
                ] {
                    let rs: Vec<f64> = rate
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| inside(i % gw * 320 / gw, i / gw * 180 / gh))
                        .filter_map(|(_, r)| *r)
                        .collect();
                    // The region's bounding rectangle at 320 x 180.
                    let (mut x0, mut y0, mut x1, mut y1) = (320, 180, 0, 0);
                    for y in 0..180 {
                        for x in 0..320 {
                            if inside(x, y) {
                                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
                            }
                        }
                    }
                    println!(
                        "reading: {pose} {name}: {} lighting px, mean hit rate {:.4}, rect [{x0}, {y0}, {x1}, {y1}]",
                        rs.len(),
                        rs.iter().sum::<f64>() / rs.len().max(1) as f64
                    );
                }
                for cell in cells() {
                    let r = run_cell(&rig, cam, noon, cell);
                    print_row(pose, cell, &r, &[("rim", &rim), ("far", &far)]);
                }
            }
        }
        let clock = flythrough::daylight::Clock {
            fixed_sun: Some([60.0, 180.0]),
            ..Default::default()
        };
        let sky = clock.sky(flythrough::outdoor::BASE_Z as f32);
        if let Some((rig, cam)) = forest_rig() {
            let (rate, gw) = hit_rates(&rig, &cam);
            let gh = rate.len() / gw;
            let hit = |x: usize, y: usize| rate[(y * gh / 180) * gw + x * gw / 320].is_some();
            for cell in cells() {
                let r = run_cell(&rig, &cam, sky, cell);
                print_row("forest", cell, &r, &[("ground", &hit)]);
            }
        }
    }

    /// The defaults' cell: both terms, the filter on, the history as built, FSR 2 at 1.5.
    pub(super) fn default_cell(leaf_brick: bool) -> Cell {
        Cell {
            term: "both",
            filter: true,
            any_history: false,
            fsr2: true,
            leaf_brick,
        }
    }

    /// The key light's shadow ray's steps in the steps view at `cam` under `sky` (native 320 x
    /// 180, the defaults, the lamp off), over the lighting pixels that cast one: the mean and the
    /// 99th percentile, over 4 frames.
    pub(super) fn key_steps(
        rig: &Rig,
        cam: &vxe_render::Camera,
        sky: vxe_render::sky::Sky,
        leaf_brick: bool,
    ) -> (f64, f32) {
        use vxe_render::{LookParams, ViewMode};
        let mut f = Frame::new(&rig.device, 320, 180);
        f.set_lighting(Lighting {
            lamp: false,
            ..flythrough::default_lighting()
        });
        f.set_look(LookParams {
            view: ViewMode::Steps,
            ..LookParams::default()
        });
        if leaf_brick {
            f.set_material_styles(
                &rig.queue,
                &flythrough::outdoor_styles(flythrough::LeafShadow::Brick),
            );
        }
        f.set_sky(Some(sky));
        let mut steps: Vec<f32> = Vec::new();
        for _ in 0..4 {
            f.step(
                &rig.device,
                &rig.queue,
                &rig.gpu,
                cam,
                &LightParams::headlamp(),
            );
            let d = vxe_render::debug::read_rgba16f(&rig.device, &rig.queue, f.direct());
            steps.extend(d.iter().filter(|p| p[3] > 0.5).map(|p| p[0]));
        }
        steps.sort_by(f32::total_cmp);
        let mean = steps.iter().map(|&v| f64::from(v)).sum::<f64>() / steps.len().max(1) as f64;
        (
            mean,
            steps.get(steps.len() * 99 / 100).copied().unwrap_or(0.0),
        )
    }

    /// Plan 0086 Phase 2 bar 5: in the forest, the key ray's steps at `--sun 10,180` and the
    /// stillness at `60,180` (the defaults' cell) with the leaf's SHADOW_AS_BRICK set and clear.
    /// `VXE_TEST_ADAPTER` picks the adapter (unset: lavapipe).
    #[test]
    #[ignore = "reading"]
    fn forest_leaf_shadow() {
        let sky_at = |sun: [f32; 2]| {
            flythrough::daylight::Clock {
                fixed_sun: Some(sun),
                ..Default::default()
            }
            .sky(flythrough::outdoor::BASE_Z as f32)
        };
        let Some((rig, cam)) = forest_rig() else {
            return;
        };
        let (rate, gw) = hit_rates(&rig, &cam);
        let gh = rate.len() / gw;
        let hit = |x: usize, y: usize| rate[(y * gh / 180) * gw + x * gw / 320].is_some();
        println!("reading: forest_leaf_shadow on {}", rig.adapter);
        for leaf_brick in [true, false] {
            let (mean, p99) = key_steps(&rig, &cam, sky_at([10.0, 180.0]), leaf_brick);
            println!(
                "reading: forest 10/180 leaf {}: key ray steps mean {mean:.2} p99 {p99}",
                if leaf_brick { "brick" } else { "voxel" }
            );
            let cell = default_cell(leaf_brick);
            let r = run_cell(&rig, &cam, sky_at([60.0, 180.0]), cell);
            print_row(
                if leaf_brick {
                    "leaf brick"
                } else {
                    "leaf voxel"
                },
                cell,
                &r,
                &[("ground", &hit)],
            );
        }
    }
}
