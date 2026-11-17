//! The outdoor source's bars that open a device: the in-process source switch leaves nothing
//! of the outdoor world behind. Headless runs of the fly-through binary on lavapipe. The
//! readings in `mod reading` run by hand.
#![allow(
    clippy::disallowed_methods,
    reason = "the runs are bounded by wall-clock time"
)]

mod support;

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The stdout of a headless run on lavapipe at 320 x 180 with the live queue's pre-roll at
/// its smallest, the residency at 6 and 8 chunks (the spawn's block of 2 inside it) and `extra` arguments, or `None` with a printed skip when there is no such
/// adapter. Panics past `bound` or on a failed exit, with both streams.
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

    /// Plan 0032 Phase 8 bar 5, as a reading: a 1024 x 1024 rock floor whose lit plane is z
    /// = 0 and one tree from the builder (trunk radius 2 on column (0, 0) up to z = 80, a
    /// solid canopy of radius 12 on voxel (0, 0, 80)), the camera at (0.5, 0.5, 250.5) looking
    /// straight down at 640 x 360; lamp off, no list light, the sky rays and the filter off,
    /// manual exposure 1, `E_sun = (1, 1, 1)` at the real angular radius, the direct target's
    /// luminance a 16-frame mean per lighting pixel. Each floor pixel's occlusion `w = 1 - I /
    /// I_open` against the same pose without the tree; the centroid of `w`, at the pixels'
    /// hit positions, over floor pixels at least 4 voxels to either side of the shadow's axis;
    /// at 60/180 against (0.5, 46.98) and 30/270 against (139.93, 0.5); and the sum of `w` at
    /// 30 over 60 against `sin 60 / sin 30 = 1.732`. Under the full walk and the coarse walk.
    /// `VXE_TEST_ADAPTER` picks the adapter (unset: lavapipe).
    #[test]
    #[ignore = "reading"]
    fn a_tree_shadow_travels_with_the_sun() {
        use flythrough::trees::{TRUNK_SINK, TreeShape, build_tree};
        use support::SyncNow;
        use vxe_render::sky::{Body, SUN_ANGULAR_RADIUS, Sky};
        use vxe_render::{
            Camera, Frame, GpuWorld, LightingResolution, MirrorConfig, ShadowWalk, Window,
            glam::Vec3,
        };
        use vxe_world::{AIR, ChunkExtent, FnSource, World};
        const SIZE: [u32; 2] = [640, 360];
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
                ChunkExtent::new([-8, -8, -1], [8, 8, 4]),
                &FnSource(f),
                &flythrough::outdoor_materials(),
            )
        };
        let worlds = [world_of(false), world_of(true)];
        let pool =
            flythrough::level_pool_units(&worlds[1], false) * vxe_render::internals::UNIT_BYTES;
        let Some((device, queue, adapter)) = support::reading_device(pool) else {
            return;
        };
        let gpus: Vec<GpuWorld> = worlds
            .iter()
            .map(|w| {
                let mut gpu =
                    GpuWorld::with_config(&device, MirrorConfig::new(Window::Fixed(w.extent())));
                for _ in 0..4 {
                    gpu.sync_now(w, &device, &queue).expect("the mirror fits");
                }
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
        let read = |gpu: &GpuWorld, sun: [f32; 3], walk: ShadowWalk| {
            let mut frame = Frame::new(&device, SIZE[0], SIZE[1]);
            frame.set_resolution(&device, LightingResolution::Full);
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
            let n = (SIZE[0] * SIZE[1]) as usize;
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
        for walk in [ShadowWalk::Full, ShadowWalk::Coarse] {
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
            }
            println!(
                "reading: tree shadow, {} walk: sum of w at 30 over 60 {:.3}, want 1.732",
                walk.name(),
                w_sums[1] / w_sums[0]
            );
        }
    }
}
