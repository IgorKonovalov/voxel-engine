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

/// Plan 0086 Phase 9 bar 3: the underground eye reads nothing. [`reading::underground_eye`] at
/// `r` 7 (the owner's run-12 eye in a closed cavern at noon, the world generated and mirrored
/// whole, the store baked whole): the near-eye baked visibility's mean per face is under 0.001 in
/// every direction, and at both sky-ray counts the stored and direct parts' mean luminance at
/// exposure 1 are under 0.02 in the six views, with the surface reference's within 5 % of
/// 149.7 and 1345 (its reading at the diagnosis, `2fe4598c`).
#[test]
#[ignore = "soak"]
fn the_underground_eye_reads_no_sky() {
    let Some(u) = reading::underground_eye(7) else {
        return;
    };
    for (f, v) in u.near_vis.iter().enumerate() {
        assert!(*v < 0.001, "face {f}: near-eye visibility {v}");
    }
    for (rays, name, stored, direct) in &u.parts {
        if name == "surface ref" {
            if *rays == 1 {
                assert!(
                    (stored / 149.7 - 1.0).abs() <= 0.05,
                    "surface stored {stored}"
                );
                assert!(
                    (direct / 1345.0 - 1.0).abs() <= 0.05,
                    "surface direct {direct}"
                );
            }
            continue;
        }
        assert!(*stored < 0.02, "rays {rays} {name}: stored {stored}");
        assert!(*direct < 0.02, "rays {rays} {name}: direct {direct}");
    }
}

/// Plan 0086 Phase 10 bar 1: the young visibility pair forgets the leak.
/// [`reading::underground_streamed`] at full radius 6 and one sky ray (the run-12 eye underground
/// at noon, reached as by a jump) under the column wait's default, off (Phase 18 bar 4: a chunk
/// may bake provisionally before its columns are final): the hit pixels' mean direct part reads
/// under 0.05 by frame 60 and under 0.02 at frame 240; frames 1 to 30 are printed (a brief
/// provisional leak is allowed there).
#[test]
#[ignore = "soak"]
fn the_underground_eye_streamed() {
    let frames: Vec<u32> = (1..=30).chain([60, 240]).collect();
    let reads = reading::underground_streamed(6, 1, &frames);
    let Some(last) = reads.last() else {
        return;
    };
    let at = |f: u32| reads.iter().find(|r| r.frame == f).expect("read").direct;
    let early: Vec<String> = (1..=30).map(|f| format!("{:.4}", at(f))).collect();
    println!(
        "direct at frames 1 to 30 [{}], 60 {:.4}, 240 {:.4}",
        early.join(", "),
        at(60),
        last.direct
    );
    assert!(at(60) < 0.05, "frame 60: {}", at(60));
    assert!(at(240) < 0.02, "frame 240: {}", at(240));
}

mod reading {
    use super::support;
    use support::stillness::{Pose, SIZE, outdoor_level_on, run_under_sky};
    use support::terms::*;
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
                // The sky rays' share that reached the sky (ADR-0070), in r of 255.
                let px = vxe_render::debug::read_rgba8(&device, &queue, f.visibility_samples());
                sum.resize(px.len() / 4, 0.0);
                for (s, p) in sum.iter_mut().zip(px.chunks(4)) {
                    *s += f64::from(p[0]) / 255.0 / 64.0;
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
        if let Some((mut rig, views)) = tunnel_rig() {
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
                    let r = run_cell(&mut rig, cam, noon, cell);
                    print_row(pose, cell, &r, &[("rim", &rim), ("far", &far)]);
                }
            }
        }
        let clock = flythrough::daylight::Clock {
            fixed_sun: Some([60.0, 180.0]),
            ..Default::default()
        };
        let sky = clock.sky(flythrough::outdoor::BASE_Z as f32);
        if let Some((mut rig, cam)) = forest_rig() {
            let (rate, gw) = hit_rates(&rig, &cam);
            let gh = rate.len() / gw;
            let hit = |x: usize, y: usize| rate[(y * gh / 180) * gw + x * gw / 320].is_some();
            for cell in cells() {
                let r = run_cell(&mut rig, &cam, sky, cell);
                print_row("forest", cell, &r, &[("ground", &hit)]);
            }
        }
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
        let Some((mut rig, cam)) = forest_rig() else {
            return;
        };
        let (rate, gw) = hit_rates(&rig, &cam);
        let gh = rate.len() / gw;
        let hit = |x: usize, y: usize| rate[(y * gh / 180) * gw + x * gw / 320].is_some();
        println!("reading: forest_leaf_shadow on {}", rig.adapter);
        for leaf in [flythrough::LeafShadow::Brick, flythrough::LeafShadow::Voxel] {
            let (mean, p99, _) = key_steps(
                &rig,
                &cam,
                sky_at([10.0, 180.0]),
                leaf,
                vxe_render::Penumbra::Disc,
            );
            println!(
                "reading: forest 10/180 leaf {}: key ray steps mean {mean:.2} p99 {p99}",
                leaf.name()
            );
            let cell = default_cell(leaf);
            let r = run_cell(&mut rig, &cam, sky_at([60.0, 180.0]), cell);
            print_row(
                &format!("leaf {}", leaf.name()),
                cell,
                &r,
                &[("ground", &hit)],
            );
        }
    }

    /// Plan 0086 Phase 3 bar 4: the forest's statistics at `60,180` with the leaf's
    /// SHADOW_AS_BRICK set, with the far normal's stair rules hooked off (before the phase) and
    /// on. `VXE_TEST_ADAPTER` picks the adapter (unset: lavapipe).
    #[test]
    #[ignore = "reading"]
    fn forest_stairs() {
        let sky = flythrough::daylight::Clock {
            fixed_sun: Some([60.0, 180.0]),
            ..Default::default()
        }
        .sky(flythrough::outdoor::BASE_Z as f32);
        let Some((mut rig, cam)) = forest_rig() else {
            return;
        };
        let (rate, gw) = hit_rates(&rig, &cam);
        let gh = rate.len() / gw;
        let hit = |x: usize, y: usize| rate[(y * gh / 180) * gw + x * gw / 320].is_some();
        println!("reading: forest_stairs on {}", rig.adapter);
        for no_stair in [true, false] {
            let cell = Cell {
                no_stair,
                ..default_cell(flythrough::LeafShadow::Brick)
            };
            let r = run_cell(&mut rig, &cam, sky, cell);
            print_row(
                if no_stair { "stair off" } else { "stair on" },
                cell,
                &r,
                &[("ground", &hit)],
            );
        }
    }

    /// Plan 0086 Phase 4 bar 6: the tunnel at rest under the bake. The tunnel (`inside`, seed
    /// 1, 12:00, lamp off) out of and into it, Phase 1's regions (the rim band and the far
    /// end by the sky ray's hit rate), the defaults' cell (FSR 2 at 1.5, the filter on): the
    /// ray alone as before (no bake), the store baked whole with `sky_rays` 0 (the bake only),
    /// and baked with the ray on (cast, and dropped where the store holds a visibility). Per
    /// row Phase 1's statistics and each region's mean display level. `VXE_TEST_ADAPTER`
    /// picks the adapter (unset: lavapipe).
    #[test]
    #[ignore = "reading"]
    fn the_tunnel_at_rest_under_the_bake() {
        let noon = flythrough::daylight::Clock::default().sky(flythrough::outdoor::BASE_Z as f32);
        let Some((mut rig, views)) = tunnel_rig() else {
            return;
        };
        println!(
            "reading: the_tunnel_at_rest_under_the_bake on {}",
            rig.adapter
        );
        for (pose, cam) in &views {
            let (rate, gw) = hit_rates(&rig, cam);
            let gh = rate.len() / gw;
            let at = |x: usize, y: usize| rate[(y * gh / 180) * gw + x * gw / 320];
            let rim = |x: usize, y: usize| at(x, y).is_some_and(|r| (0.3..0.6).contains(&r));
            let far = |x: usize, y: usize| at(x, y).is_some_and(|r| r < 0.05);
            for (name, cell) in [
                ("ray", default_cell(flythrough::LeafShadow::Brick)),
                (
                    "bake only",
                    Cell {
                        term: "key",
                        bake: true,
                        ..default_cell(flythrough::LeafShadow::Brick)
                    },
                ),
                (
                    "bake + ray",
                    Cell {
                        bake: true,
                        ..default_cell(flythrough::LeafShadow::Brick)
                    },
                ),
            ] {
                let r = run_cell(&mut rig, cam, noon, cell);
                print_row(
                    &format!("{pose} {name}"),
                    cell,
                    &r,
                    &[("rim", &rim), ("far", &far)],
                );
                println!(
                    "reading: {pose} {name}: mean level rim {:.1} far {:.1}",
                    region_level(&r, &rim),
                    region_level(&r, &far)
                );
            }
        }
    }

    /// Plan 0086 Phase 5 bar 4: an open rock floor (4 x 4 chunks, rock below z 16) under
    /// `Sky::uniform(1.0)` through the live queue (`support::queue`'s rig at its counts), a
    /// camera 40 voxels up looking down at 60 degrees. Settled, then a 10-voxel box dug into the
    /// floor ahead: per frame, the lighting pixels where the sky rays were cast, until three
    /// frames in a row read none (or 240 frames). Then the same with `sky_rays` 0, where the
    /// count stays 0, and the dug floor's centre pixel's stored part per frame. Printed.
    #[test]
    #[ignore = "reading"]
    fn the_sky_ray_runs_only_where_stale() {
        use support::queue::{Setup, box_op, fill, rig, world_of};
        use vxe_world::{AIR, Mode};
        let floor = |rays: u32| {
            let world = world_of([4, 4, 1], AIR);
            fill(&world, [0, 0, 0], [255, 255, 15], flythrough::cave::ROCK);
            let eye = [80.0, 128.0, 56.0];
            let mut q = rig(world, &Setup::at(eye))?;
            q.camera.pitch = -60f32.to_radians();
            q.frame.set_lighting(Lighting {
                sky_rays: rays,
                ..q.frame.lighting()
            });
            q.frame.set_sky(Some(vxe_render::sky::Sky {
                altitude_zero_z: 16.0,
                ..vxe_render::sky::Sky::uniform(1.0)
            }));
            Some(q)
        };
        for rays in [1u32, 0] {
            let Some(mut q) = floor(rays) else {
                return;
            };
            let settled = q.settle();
            let cast = |q: &mut support::queue::Q| {
                q.step();
                q.render();
                let v = vxe_render::debug::read_rgba8(
                    &q.device,
                    &q.queue,
                    q.frame.visibility_samples(),
                );
                let n = v.chunks(4).filter(|p| p[3] > 0).count();
                let a = vxe_render::debug::read_rgba16f(&q.device, &q.queue, q.frame.accumulated());
                let (w, h) = (
                    q.frame.accumulated().width() as usize,
                    q.frame.accumulated().height() as usize,
                );
                (n, a[(h / 2) * w + w / 2][1])
            };
            let at_rest: Vec<usize> = (0..4).map(|_| cast(&mut q).0).collect();
            println!(
                "reading: sky rays {rays}: settled in {settled} steps; at rest {at_rest:?} pixels cast"
            );
            // The camera's centre ray meets the floor about 23 voxels ahead: dig 10 voxels there.
            q.edit(&box_op(
                [98, 123, 6],
                [107, 132, 15],
                Mode::Replace {
                    from: flythrough::cave::ROCK,
                    to: AIR,
                },
            ));
            let mut row = Vec::new();
            let mut quiet = 0;
            for f in 1..=240 {
                let (n, centre) = cast(&mut q);
                row.push(format!("{f}:{n}:{centre:.3}"));
                quiet = if n == 0 { quiet + 1 } else { 0 };
                if quiet >= 3 && f > 8 {
                    break;
                }
            }
            println!(
                "reading: sky rays {rays}: after the dig, frame:cast pixels:centre stored g {}",
                row.join(" ")
            );
        }
    }

    /// Plan 0086 Phase 7 bar 2: the forest at `60,180` and `10,180` with the penumbra as the
    /// disc's and the closest approach's: the key ray's steps and loads, and the statistics of
    /// the defaults' cell (the leaf's SHADOW_AS_BRICK set) at `60,180`. `VXE_TEST_ADAPTER` picks
    /// the adapter (unset: lavapipe).
    #[test]
    #[ignore = "reading"]
    fn forest_approach() {
        use vxe_render::Penumbra;
        let sky_at = |sun: [f32; 2]| {
            flythrough::daylight::Clock {
                fixed_sun: Some(sun),
                ..Default::default()
            }
            .sky(flythrough::outdoor::BASE_Z as f32)
        };
        let Some((mut rig, cam)) = forest_rig() else {
            return;
        };
        let (rate, gw) = hit_rates(&rig, &cam);
        let gh = rate.len() / gw;
        let hit = |x: usize, y: usize| rate[(y * gh / 180) * gw + x * gw / 320].is_some();
        println!("reading: forest_approach on {}", rig.adapter);
        for penumbra in [Penumbra::Disc, Penumbra::Approach] {
            for sun in [[60.0, 180.0], [10.0, 180.0]] {
                let (mean, p99, loads) = key_steps(
                    &rig,
                    &cam,
                    sky_at(sun),
                    flythrough::LeafShadow::Brick,
                    penumbra,
                );
                println!(
                    "reading: forest {sun:?} {}: key ray steps mean {mean:.2} p99 {p99}, loads mean {loads:.1}",
                    penumbra.name()
                );
            }
            let cell = Cell {
                approach: penumbra == Penumbra::Approach,
                ..default_cell(flythrough::LeafShadow::Brick)
            };
            let r = run_cell(&mut rig, &cam, sky_at([60.0, 180.0]), cell);
            print_row(penumbra.name(), cell, &r, &[("ground", &hit)]);
        }
    }

    /// The lighting grid's hit pixels of `f`'s last frame: their count, the mean luminance of
    /// the accumulated stored part, and the mean luminance of the accumulated direct part.
    fn hit_parts(
        device: &vxe_render::wgpu::Device,
        queue: &vxe_render::wgpu::Queue,
        f: &Frame,
    ) -> (usize, f64, f64) {
        let hits = vxe_render::debug::read_rgba32f(device, queue, f.light_hit_positions());
        let acc = vxe_render::debug::read_rgba16f(device, queue, f.accumulated());
        let dir = vxe_render::debug::read_rgba16f(device, queue, f.direct_accumulated());
        let lum = |p: &[f32; 4]| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
        let (mut n, mut a, mut d) = (0usize, 0.0f64, 0.0f64);
        for i in 0..hits.len() {
            if hits[i][3] < 1.0e30 {
                n += 1;
                a += f64::from(lum(&acc[i]));
                d += f64::from(dir[i][3]);
            }
        }
        let k = n.max(1) as f64;
        (n, a / k, d / k)
    }

    /// Plan 0086 Phase 8 diagnosis: the owner's run-12 eye underground (743, 131, 14.7; seed 1,
    /// no trees, caverns closed to the sky). The world generated whole over chunks
    /// `x, y` within `VXE_UNDER_R` (default 3) of the eye's chunk and `z` -3..9 (over the
    /// surface at about 501), mirrored whole. Per view (six axes and a
    /// reference over the surface looking down): the sky ray's hit rate (`hit_rates`); then the
    /// store baked whole under the noon sky and the baked visibility of every brick face within
    /// 6 bricks of the eye's brick; then the noon frame at manual exposure 1 with the bake, at
    /// `sky_rays` 1 and 0, the mean luminance of the accumulated stored part and direct part at
    /// the hits. Printed. `VXE_TEST_ADAPTER` picks the adapter (unset: lavapipe).
    #[test]
    #[ignore = "reading"]
    fn the_underground_eye() {
        let r = std::env::var("VXE_UNDER_R")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3);
        underground_eye(r);
    }

    /// What [`underground_eye`] read: the near-eye baked visibility's mean per face (within 6
    /// bricks of the eye's brick), and per sky-ray count (1, then 0) and view (the six axes,
    /// then the surface reference) the stored and direct parts' mean luminance at the hits.
    pub(super) struct UnderEye {
        pub(super) near_vis: [f64; 6],
        pub(super) parts: Vec<(u32, String, f64, f64)>,
    }

    /// [`the_underground_eye`]'s rig at `r`, printed; `None` with no adapter.
    pub(super) fn underground_eye(r: i32) -> Option<UnderEye> {
        use flythrough::outdoor::OutdoorSource;
        use vxe_world::{ChunkExtent, VoxelCoord, World};
        let zhi = 9;
        let source = OutdoorSource::new(1);
        let eye = VoxelCoord([743, 131, 15]);
        let c = eye.chunk().0;
        let extent = ChunkExtent::new([c[0] - r, c[1] - r, -3], [c[0] + r + 1, c[1] + r + 1, zhi]);
        let t = std::time::Instant::now();
        let world = World::generate(extent, &source, &flythrough::outdoor_materials());
        println!(
            "reading: the_underground_eye: extent {extent:?} ({} chunks) generated in {:.1} s",
            extent.len(),
            t.elapsed().as_secs_f64()
        );
        let mut rig = mirrored(world, eye)?;
        let mut out = UnderEye {
            near_vis: [0.0; 6],
            parts: Vec::new(),
        };
        println!("reading: adapter {}", rig.adapter);
        let p = [743.5f32, 131.5, 14.7];
        let top = source.height(743, 131) as f32 + 20.0;
        let mut views: Vec<(String, vxe_render::Camera)> = [
            ("+x", [1.0, 0.0, 0.0]),
            ("-x", [-1.0, 0.0, 0.0]),
            ("+y", [0.0, 1.0, 0.0]),
            ("-y", [0.0, -1.0, 0.0]),
            ("up", [0.2, 0.0, 1.0]),
            ("down", [0.2, 0.0, -1.0]),
        ]
        .iter()
        .map(|(n, d)| {
            (
                n.to_string(),
                flythrough::camera_looking_at(p, [p[0] + d[0], p[1] + d[1], p[2] + d[2]]),
            )
        })
        .collect();
        views.push((
            "surface ref".into(),
            flythrough::camera_looking_at([p[0], p[1], top], [p[0] + 30.0, p[1], top - 20.0]),
        ));
        for (name, cam) in &views {
            let (rate, _) = hit_rates(&rig, cam);
            let hits: Vec<f64> = rate.iter().flatten().copied().collect();
            let mean = hits.iter().sum::<f64>() / hits.len().max(1) as f64;
            let lit = hits.iter().filter(|&&v| v > 0.0).count();
            let max = hits.iter().copied().fold(0.0, f64::max);
            println!(
                "reading: view {name}: sky ray hit rate over {} hit pixels mean {mean:.4} max {max:.3}, pixels with any hit {lit}",
                hits.len()
            );
        }
        let noon = flythrough::daylight::Clock::default().sky(flythrough::outdoor::BASE_Z as f32);
        let mut f = Frame::new(&rig.device, 320, 180);
        f.set_sky(Some(noon));
        let t = std::time::Instant::now();
        let report = f
            .bake_until(
                &rig.device,
                &rig.queue,
                &mut rig.gpu,
                &rig.world,
                rig.world.extent(),
                &vxe_render::BakeSettings::default(),
                &mut |_| {},
            )
            .expect("the store fits");
        println!(
            "reading: bake {} chunks, {} faces, {} sky rays in {:.1} s",
            report.chunks,
            report.faces,
            report.sky_rays,
            t.elapsed().as_secs_f64()
        );
        let blocks = support::bake::frame_blocks(&rig.device, &rig.queue, &f, rig.world.extent());
        let (vis, _) = support::bake::sky_cells_of(&blocks);
        let eb = eye.0.map(|v| v.div_euclid(8));
        for (label, near) in [
            ("within 6 bricks of the eye", 6),
            ("under z 400 (any)", 1000),
        ] {
            let mut by_face = [(0usize, 0.0f64, 0.0f32, 0usize); 6];
            let mut none = 0usize;
            for ((b, f), v) in &vis {
                let close = (0..3).all(|k| (b[k] - eb[k]).abs() <= near);
                if !close || b[2] * 8 >= 400 {
                    continue;
                }
                match v {
                    Some(v) => {
                        let e = &mut by_face[usize::from(*f)];
                        e.0 += 1;
                        e.1 += f64::from(*v);
                        e.2 = e.2.max(*v);
                        if *v > 0.05 {
                            e.3 += 1;
                        }
                    }
                    None => none += 1,
                }
            }
            for (fi, (n, sum, max, over)) in by_face.iter().enumerate() {
                if near == 6 {
                    out.near_vis[fi] = sum / (*n).max(1) as f64;
                }
                println!(
                    "reading: baked visibility {label}, face {fi}: {n} faces, mean {:.4}, max {max:.3}, over 0.05: {over}",
                    sum / (*n).max(1) as f64
                );
            }
            println!("reading: baked visibility {label}: {none} faces without a value");
        }
        for rays in [1u32, 0] {
            for (name, cam) in &views {
                f.set_lighting(Lighting {
                    lamp: false,
                    exposure: Exposure::Manual(1.0),
                    sky_rays: rays,
                    ..flythrough::default_lighting()
                });
                for _ in 0..64 {
                    f.step(
                        &rig.device,
                        &rig.queue,
                        &rig.gpu,
                        cam,
                        &LightParams::headlamp(),
                    );
                }
                let (n, a, d) = hit_parts(&rig.device, &rig.queue, &f);
                println!(
                    "reading: noon bake, sky rays {rays}, view {name}: {n} hits, stored part luminance mean {a:.4e}, direct part mean {d:.4e}"
                );
                out.parts.push((rays, name.clone(), a, d));
            }
        }
        Some(out)
    }

    /// Plan 0086 Phase 8 diagnosis, streamed: the run-12 eye underground (743, 131, 14.7;
    /// seed 1, no trees) reached as by a jump (`support::streamed::streamed`), `VXE_UNDER_R` full (default 6),
    /// `sky_rays` `VXE_UNDER_RAYS` (default 1), facing +x, at 12:00. Printed at listed frames.
    /// `VXE_TEST_ADAPTER` picks the adapter.
    #[test]
    #[ignore = "reading"]
    fn the_underground_eye_streamed() {
        let env = |k: &str, d: i32| {
            std::env::var(k)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(d)
        };
        underground_streamed(
            env("VXE_UNDER_R", 6),
            env("VXE_UNDER_RAYS", 1) as u32,
            &[1, 3, 10, 30, 60, 120, 240, 480, 720, 960, 1440, 1920],
        );
    }

    /// Plan 0086 Phase 10 bar 3: the forest after a jump (seed 1, `--trees forest`, the leaf
    /// fill's default, the forest spawn facing its way, the sun at `10,180`, the leaf shadowing
    /// as its bricks), streamed as by a jump at full radius 6 (`support::streamed::streamed`) and read every
    /// frame for 600: the frames until the store's black count falls under 1000 and until its
    /// missing count falls under 5000 (at 320 x 180, 57 600 lighting pixels). Printed.
    #[test]
    #[ignore = "reading"]
    fn the_forest_after_a_jump() {
        use flythrough::outdoor::{OutdoorSource, Spawn};
        use flythrough::trees::{ForestSource, LEAF_FILL, TreeDensity};
        let source = ForestSource::new(OutdoorSource::new(1), TreeDensity::Forest, LEAF_FILL);
        let (eye, facing) = source.base().spawn(Spawn::Forest);
        let p = eye.0.map(|v| v as f32 + 0.5);
        let d = facing.map(|v| v as f32);
        let camera = flythrough::camera_looking_at(p, [p[0] + d[0], p[1] + d[1], p[2] + d[2]]);
        let sky = flythrough::daylight::Clock {
            fixed_sun: Some([10.0, 180.0]),
            ..Default::default()
        }
        .sky(flythrough::outdoor::BASE_Z as f32);
        let reads = support::streamed::streamed(support::streamed::Streamed {
            label: "the_forest_after_a_jump",
            source: std::sync::Arc::new(source),
            eye,
            camera,
            sky,
            styles: flythrough::outdoor_styles(flythrough::LeafShadow::Brick),
            r: 6,
            rays: 1,
            reads: (0..600).collect(),
            hold: None,
        });
        let first = |f: &dyn Fn(&support::streamed::StreamedRead) -> bool| {
            reads
                .iter()
                .position(f)
                .map_or(-1, |i| reads[i].frame as i64)
        };
        let settled = |f: &dyn Fn(&support::streamed::StreamedRead) -> bool| {
            reads
                .iter()
                .rposition(|r| !f(r))
                .map_or(0, |i| reads[i].frame as i64 + 1)
        };
        println!(
            "reading: the forest after a jump: black under 1000 first at frame {}, for good from {}; missing under 5000 first at {}, for good from {}",
            first(&|r| r.black < 1000),
            settled(&|r| r.black < 1000),
            first(&|r| r.missing < 5000),
            settled(&|r| r.missing < 5000),
        );
    }

    /// [`streamed`] at the run-12 eye underground at noon, facing +x, full radius `r`.
    pub(super) fn underground_streamed(
        r: i32,
        rays: u32,
        reads: &[u32],
    ) -> Vec<support::streamed::StreamedRead> {
        let source = std::sync::Arc::new(flythrough::outdoor::OutdoorSource::new(1));
        let camera = flythrough::Pose {
            name: "underground",
            position: [743.5, 131.5, 14.7],
            yaw_deg: 0.0,
            pitch_deg: 0.0,
            exposure: 1.0,
        }
        .camera();
        support::streamed::streamed(support::streamed::Streamed {
            label: "the_underground_eye_streamed",
            source,
            eye: vxe_world::VoxelCoord([743, 131, 15]),
            camera,
            sky: flythrough::daylight::Clock::default().sky(flythrough::outdoor::BASE_Z as f32),
            styles: Vec::new(),
            r,
            rays,
            reads: reads.to_vec(),
            hold: None,
        })
    }

    /// Plan 0086 Phase 8 diagnosis: the tunnel rig (`tunnel_rig`) baked whole under the noon
    /// sky at the defaults; at eleven stations along the tunnel's axis (fraction 0 at the mouth,
    /// 1 at the chamber's centre), the baked visibility of the brick faces within 4 bricks of
    /// the station, by face direction: count and mean. Printed. Run once as built and once with
    /// the bake's sky cap raised by hand, to split the cap's share. `VXE_TEST_ADAPTER` picks the
    /// adapter.
    #[test]
    #[ignore = "reading"]
    fn the_tunnel_bake_by_station() {
        let Some((mut rig, _)) = tunnel_rig() else {
            return;
        };
        let source = flythrough::outdoor::OutdoorSource::new(1);
        let [a, c] = source.tunnel();
        let noon = flythrough::daylight::Clock::default().sky(flythrough::outdoor::BASE_Z as f32);
        let mut f = Frame::new(&rig.device, 320, 180);
        f.set_sky(Some(noon));
        f.bake_until(
            &rig.device,
            &rig.queue,
            &mut rig.gpu,
            &rig.world,
            rig.world.extent(),
            &vxe_render::BakeSettings::default(),
            &mut |_| {},
        )
        .expect("the store fits");
        let blocks = support::bake::frame_blocks(&rig.device, &rig.queue, &f, rig.world.extent());
        let (vis, _) = support::bake::sky_cells_of(&blocks);
        println!("reading: the_tunnel_bake_by_station on {}", rig.adapter);
        for station in 0..=10i64 {
            let centre = [0, 1, 2].map(|k| (a[k] + (c[k] - a[k]) * station / 10) as i32);
            let cb = centre.map(|v| v.div_euclid(8));
            let mut by = [(0usize, 0.0f64); 6];
            for ((b, face), v) in &vis {
                if (0..3).all(|k| (b[k] - cb[k]).abs() <= 4)
                    && let Some(v) = v
                {
                    by[usize::from(*face)].0 += 1;
                    by[usize::from(*face)].1 += f64::from(*v);
                }
            }
            let row: Vec<String> = ["-x", "+x", "-y", "+y", "-z", "+z"]
                .iter()
                .zip(by)
                .map(|(n, (k, s))| format!("{n} {k}:{:.4}", s / k.max(1) as f64))
                .collect();
            println!(
                "reading: station {station} at {centre:?}: {}",
                row.join(", ")
            );
        }
    }

    /// Plan 0086 Phase 12 bar 2: the eye's ceiling under each arm. The defaults' cell (FSR 2
    /// at 1.5, the filter on, the store baked whole) under the outdoor auto exposure (instant,
    /// `OUTDOOR_MIN` and `OUTDOOR_MAX`) with the sky anchor `none`, 0, 2, 4 and 6: the tunnel at
    /// noon out of and into it (the metered exposure and the ceiling in EV, the rim band's and
    /// the far end's mean display level), the hills at noon (`outside`; the whole frame's mean
    /// level) and the forest at 17:45 (`outdoor_forest_sunset`'s clock). Printed; the window
    /// picks. `VXE_TEST_ADAPTER` picks the adapter (unset: lavapipe).
    #[test]
    #[ignore = "reading"]
    fn the_tunnel_under_each_ceiling() {
        let clock = |minutes: f64| flythrough::daylight::Clock {
            minutes,
            ..Default::default()
        };
        let base = flythrough::outdoor::BASE_Z as f32;
        let arms = [None, Some(0.0), Some(2.0), Some(4.0), Some(6.0)];
        let cell = |anchor| Cell {
            bake: true,
            anchor,
            ..default_cell(flythrough::LeafShadow::Voxel)
        };
        let all = |_: usize, _: usize| true;
        let name = |a: Option<f32>| a.map_or("none".to_owned(), |n| format!("N {n}"));
        if let Some((mut rig, views)) = tunnel_rig() {
            println!("reading: the_tunnel_under_each_ceiling on {}", rig.adapter);
            for (pose, cam) in &views {
                let (rate, gw) = hit_rates(&rig, cam);
                let gh = rate.len() / gw;
                let at = |x: usize, y: usize| rate[(y * gh / 180) * gw + x * gw / 320];
                let rim = |x: usize, y: usize| at(x, y).is_some_and(|r| (0.3..0.6).contains(&r));
                let far = |x: usize, y: usize| at(x, y).is_some_and(|r| r < 0.05);
                for a in arms {
                    let r = run_cell(&mut rig, cam, clock(720.0).sky(base), cell(a));
                    println!(
                        "reading: {pose} noon, anchor {}: exposure {:+.2} EV, ceiling {:+.2} EV, level rim {:.1} far {:.1} all {:.1}",
                        name(a),
                        r.exposure.log2(),
                        r.ceiling.log2(),
                        region_level(&r, &rim),
                        region_level(&r, &far),
                        region_level(&r, &all)
                    );
                }
            }
        }
        for (pose, rig, minutes) in [
            ("hills", hills_rig(), 720.0),
            ("forest", forest_rig(), 17.0 * 60.0 + 45.0),
        ] {
            let Some((mut rig, cam)) = rig else {
                return;
            };
            for a in arms {
                let r = run_cell(&mut rig, &cam, clock(minutes).sky(base), cell(a));
                println!(
                    "reading: {pose} at {minutes} min, anchor {}: exposure {:+.2} EV, ceiling {:+.2} EV, level all {:.1}",
                    name(a),
                    r.exposure.log2(),
                    r.ceiling.log2(),
                    region_level(&r, &all)
                );
            }
        }
    }

    /// Plan 0086 Phase 13 bar 2: the stripes under each arm. The defaults' cell (FSR 2 at 1.5,
    /// the filter on, the store baked whole) with `sky_normal` face and smoothed, each at
    /// `sky_bounce` 0 and 0.3: the tunnel at noon out of and into it, and the hills at noon
    /// (`outside`). Per row Phase 1's statistics (`print_row`; the tunnel's rim band and far
    /// end, the hills' every hit) and the stripe metric: the treads' over the risers' mean
    /// light within 48 voxels (`CellReading::stripe`). `VXE_TEST_ADAPTER` picks the adapter
    /// (unset: lavapipe).
    #[test]
    #[ignore = "reading"]
    fn the_stripes_under_each_arm() {
        use vxe_render::SkyNormal;
        let noon = flythrough::daylight::Clock::default().sky(flythrough::outdoor::BASE_Z as f32);
        let arms = [
            (SkyNormal::Face, 0.0),
            (SkyNormal::Smoothed, 0.0),
            (SkyNormal::Face, 0.3),
            (SkyNormal::Smoothed, 0.3),
        ];
        let row = |pose: &str,
                   (n, b): (SkyNormal, f32),
                   r: &CellReading,
                   regions: &[Region<'_>]| {
            let cell = default_cell(flythrough::LeafShadow::Voxel);
            print_row(&format!("{pose} {} {b}", n.name()), cell, r, regions);
            let [t, k] = r.stripe;
            let mean = |s: (f64, u32)| s.0 / f64::from(s.1.max(1));
            println!(
                "reading: {pose} sky normal {}, bounce {b}: tread {:.4} ({} px), riser {:.4} ({} px), stripe ratio {:.3}",
                n.name(),
                mean(t),
                t.1,
                mean(k),
                k.1,
                mean(t) / mean(k).max(1e-9)
            );
        };
        let cell = |(n, b): (SkyNormal, f32)| Cell {
            bake: true,
            sky_normal: n,
            sky_bounce: b,
            ..default_cell(flythrough::LeafShadow::Voxel)
        };
        if let Some((mut rig, views)) = tunnel_rig() {
            println!("reading: the_stripes_under_each_arm on {}", rig.adapter);
            for (pose, cam) in &views {
                let (rate, gw) = hit_rates(&rig, cam);
                let gh = rate.len() / gw;
                let at = |x: usize, y: usize| rate[(y * gh / 180) * gw + x * gw / 320];
                let rim = |x: usize, y: usize| at(x, y).is_some_and(|r| (0.3..0.6).contains(&r));
                let far = |x: usize, y: usize| at(x, y).is_some_and(|r| r < 0.05);
                for a in arms {
                    let r = run_cell(&mut rig, cam, noon, cell(a));
                    row(pose, a, &r, &[("rim", &rim), ("far", &far)]);
                }
            }
        }
        if let Some((mut rig, cam)) = hills_rig() {
            let all = |_: usize, _: usize| true;
            for a in arms {
                let r = run_cell(&mut rig, &cam, noon, cell(a));
                row("hills", a, &r, &[("all", &all)]);
            }
        }
    }

    /// Plan 0086 Phase 14 bar 1: the forest floor's grain by term. The forest (`--trees
    /// forest`, `LEAF_FILL`) at `--sun 60,180` and `10,180`, the defaults' cell (FSR 2 at 1.5,
    /// the filter on) under each `--leaf-shadow` (brick, voxel, absorb): the key light
    /// alone (`sky_rays` 0, no bake), the sky alone (the key light hooked off, the store
    /// baked) and both (baked). Per row Phase 1's statistics over every hit, the share of the
    /// lighting pixels whose direct sample changes between lit and 0 between two frames at
    /// least once (the key ray's flips; the key-alone rows), and the last frame's stored and
    /// direct parts' mean luminance; then the hills (`outside`) under both as the open
    /// ground's. `VXE_TEST_ADAPTER` picks the adapter (unset: lavapipe).
    #[test]
    #[ignore = "reading"]
    fn the_forest_floor_by_term() {
        let sky_at = |el: f32| {
            flythrough::daylight::Clock {
                fixed_sun: Some([el, 180.0]),
                ..Default::default()
            }
            .sky(flythrough::outdoor::BASE_Z as f32)
        };
        let all = |_: usize, _: usize| true;
        let report = |pose: &str, cell: Cell, r: &CellReading| {
            print_row(pose, cell, r, &[("ground", &all)]);
            let px = r.samples[0].len();
            let flips = (0..px)
                .filter(|&i| {
                    r.samples
                        .windows(2)
                        .any(|w| (w[0][i] > 0.0) != (w[1][i] > 0.0))
                })
                .count();
            println!(
                "reading: {pose} {}: flipping {:.4} of {px} lighting px; stored {:.4} direct {:.4}",
                cell.term,
                flips as f64 / px.max(1) as f64,
                r.parts[0],
                r.parts[1]
            );
        };
        let cells = |leaf: flythrough::LeafShadow| {
            [("key", false), ("sky", true), ("both", true)].map(|(term, bake)| Cell {
                term,
                bake,
                ..default_cell(leaf)
            })
        };
        if let Some((mut rig, cam)) = forest_rig() {
            println!("reading: the_forest_floor_by_term on {}", rig.adapter);
            for el in [60.0, 10.0] {
                for leaf in flythrough::LeafShadow::ALL {
                    let pose = format!("forest {el} {}", leaf.name());
                    for cell in cells(leaf) {
                        let r = run_cell(&mut rig, &cam, sky_at(el), cell);
                        report(&pose, cell, &r);
                    }
                }
            }
        }
        if let Some((mut rig, cam)) = hills_rig() {
            for el in [60.0, 10.0] {
                let cell = cells(flythrough::LeafShadow::Voxel)[2];
                let r = run_cell(&mut rig, &cam, sky_at(el), cell);
                report(&format!("hills {el}"), cell, &r);
            }
        }
    }
}

include!("gpu_outdoor/defects.rs");
