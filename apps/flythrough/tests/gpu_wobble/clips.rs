//! Plan 0104 Phase 1: the owner's move rendered as lossless clips at the window's size, a
//! reading for his eye (nothing is asserted). The rig's frame takes the fly-through's render
//! settings for the window's own flags (`flythrough::cli::parse_args_from(..).render_settings()`
//! applied over [`frame_for`]'s frame; the fields that differ are printed), the exposure metered
//! as the window meters it (not pinned), no HUD. Before frame 1 the bake queue steps at the
//! start with a device wait every frame until it has drained and the census is whole: every
//! chunk a lighting pixel's hit lies in holds its block at its tier's count
//! (`BakeQueue::settled`). Through the move every frame steps the queue (the step on the calling
//! thread, the only bake mode of this base: "lockstep") and waits on the device.
//!
//! The move (`SVET_MOVE=window` in Plan 0104's words): W as the fly-through flies it, along the
//! view with its pitch at the unshifted 30 voxels a second, until the first solid along the view
//! ray is [`RAY_CLEAR`] voxels off or 2 s have passed; then level along the heading to 5 s in
//! all, cut before the first frame within `CLEAR_VOX` of a solid (where the cut leaves under 4 s
//! the level leg's heading turns by the smallest 15-degree step that fits 5 s, else 4 s, else
//! not); a stop held 3 s; a turn of 60 degrees over 40 frames; held 3 s. The frame step is 1/55 s.
//!
//! `SVET_CLIP=<subject>:<arm>[:<stem>]`: subject `T` (the terraces eye), `C` (the canopy start
//! [`CANOPY_START`]) or `S` (the tunnel's slot); arm `default`, `native` (`T` and `S` only: the
//! upscaler off at [`native_size`], stretched to the window by the present's bilinear blit) or
//! `ray` (`C` only: `--key-cover ray`); `stem` the output under `live/anchor/` (the subject and arm when
//! not given). Frames stream to `ffmpeg` (`libx264rgb -qp 0`, Matroska) as `<stem>.mkv`; beside
//! it `<stem>.txt` (a frame a line: eye, yaw, pitch, exposure, the queue's chunks baked so far
//! and blocks fading), frame 1 as `<stem>-f1.png` and raw `<stem>-f1.rgb`. `SVET_CLIP=search` picks the canopy start (see [`search`]).
//! A fourth field takes [`clip`]'s options (Plan 0104 Phase 4: `keys`, `pin<N>`).

use super::owner_move::{FLY_VOX_S, POSES, Pose, camera_of, class_map, clear};
use super::*;

/// The window's size.
const WINDOW: [u32; 2] = [2530, 1378];
/// The frame step: three refreshes of a 165 Hz panel.
const HZ: f32 = 55.0;
/// The move in frames: the fly (frame 0 at the start), the pitched leg's longest, the shortest
/// fly before the heading turns, the stop's hold, the turn, the hold after.
const FLY_N: usize = 275;
const PITCHED_N: usize = 110;
const FLY_MIN: usize = 220;
const HOLD_N: usize = 165;
const TURN_N: usize = 40;
const TURN_DEG: f32 = 60.0;
/// The pitched leg ends when the first solid along the view ray is this close, in voxels.
const RAY_CLEAR: f32 = 60.0;
/// The settle: at least this many frames, then until drained and whole, at most the last.
const SETTLE_MIN: usize = 720;
const SETTLE_MAX: usize = 36000;

/// The canopy start Plan 0104 Phase 1 picks with [`search`] (eye in voxels, yaw, pitch): no
/// start of the two searches (800 read, 360 of them not clear) holds 10 / 5 / 20 %; this is the
/// nearest to holding them (sunlit ground 5.3 %, sunlit leaf 2.4 %, shadowed 68.4 %), 100 voxels
/// from the canopy eye at its lift over the ground. Its fly is cut at 1.85 s by the clearance
/// rule, and no turn of the level leg fits 4 s.
const CANOPY_START: ([f32; 3], f32, f32) = ([-9.5, 815.1, 628.8], 0.0, -5.0);

/// The window's flags of a subject and arm (the poses of Plan 0100's Phase 8 runs).
fn window_flags(subject: &str, arm: &str) -> Vec<&'static str> {
    let mut v = match subject {
        "T" => vec!["--source", "outdoor", "--time", "12:00"],
        "C" => vec![
            "--source", "outdoor", "--spawn", "forest", "--trees", "forest", "--time", "15:00",
        ],
        "S" => vec![
            "--source", "outdoor", "--spawn", "inside", "--time", "12:00",
        ],
        other => panic!("subject {other}: T, C or S"),
    };
    match (subject, arm) {
        (_, "default") => {}
        ("T", "native") => v.extend(["--upscaler", "off", "--render", "1280x720"]),
        ("S", "native") => v.extend(["--upscaler", "off", "--render", "1687x919"]),
        ("C", "ray") => v.extend(["--key-cover", "ray"]),
        _ => panic!("arm {arm} is not one of subject {subject}'s"),
    }
    v
}

/// The window's render settings for `flags`.
fn window_settings(flags: &[&str]) -> svet_render::RenderSettings {
    flythrough::cli::parse_args_from(flags.iter().map(|s| (*s).to_owned()))
        .expect("the window's flags parse")
        .render_settings()
}

/// The first solid voxel along `dir` (unit) from `p` within `max` voxels: its distance.
fn first_solid(world: &svet_world::World, p: [f32; 3], dir: [f32; 3], max: f32) -> Option<f32> {
    let n = (max / 0.05) as usize;
    (0..=n).map(|i| i as f32 * 0.05).find(|&t| {
        let v = [0, 1, 2].map(|k| (p[k] + dir[k] * t).floor() as i32);
        world
            .material_at(svet_world::VoxelCoord(v))
            .is_some_and(|m| m != svet_world::AIR)
    })
}

/// The move's cameras from `cam0`, the fly's frames, the pitched leg's, and the level leg's
/// heading offset in degrees.
struct Path {
    cams: Vec<Camera>,
    fly: usize,
    pitched: usize,
    heading: f32,
}

fn plan_path(world: &svet_world::World, cam0: &Camera) -> Path {
    let step = FLY_VOX_S / HZ;
    let fwd = cam0.basis().0.to_array();
    let at = |p: [f32; 3], d: [f32; 3], t: f32| [0, 1, 2].map(|k| p[k] + d[k] * t);
    let with = |p: [f32; 3]| {
        let mut c = *cam0;
        c.position = p.into();
        c
    };
    let p0 = cam0.position.to_array();
    // The pitched leg: frame i is i steps along the view.
    let mut pitched = 0;
    while pitched + 1 < PITCHED_N {
        let p = at(p0, fwd, step * pitched as f32);
        if first_solid(world, p, fwd, RAY_CLEAR).is_some() || !clear(world, &with(p)) {
            break;
        }
        pitched += 1;
    }
    let p1 = at(p0, fwd, step * pitched as f32);
    let h = (fwd[0] * fwd[0] + fwd[1] * fwd[1]).sqrt().max(1e-6);
    let level = |deg: f32| -> Vec<Camera> {
        let (s, c) = deg.to_radians().sin_cos();
        let d = [
            (fwd[0] * c - fwd[1] * s) / h,
            (fwd[0] * s + fwd[1] * c) / h,
            0.0,
        ];
        (1..FLY_N - pitched)
            .map(|i| with(at(p1, d, step * i as f32)))
            .take_while(|c| clear(world, c))
            .collect()
    };
    let head: Vec<Camera> = (0..=pitched)
        .map(|i| with(at(p0, fwd, step * i as f32)))
        .collect();
    // The heading turns only where the fly at heading 0 is under 4 s: the smallest turn (15-degree
    // steps, + before -) whose fly is 5 s, else the smallest whose fly is at least 4 s, else none.
    let mut best = (0.0, level(0.0));
    if head.len() + best.1.len() < FLY_MIN {
        let turns: Vec<(f32, Vec<Camera>)> = (1..=12)
            .flat_map(|k| [1.0, -1.0].map(|sgn| sgn * 15.0 * k as f32))
            .map(|deg| (deg, level(deg)))
            .collect();
        let pick = |n: usize| turns.iter().position(|t| head.len() + t.1.len() >= n);
        if let Some(i) = pick(FLY_N).or_else(|| pick(FLY_MIN)) {
            best = turns.into_iter().nth(i).expect("a turn");
        }
    }
    let mut cams = head;
    cams.extend(best.1);
    let fly = cams.len();
    let end = *cams.last().expect("frame 0");
    cams.extend(std::iter::repeat_n(end, HOLD_N));
    cams.extend((1..=TURN_N).map(|k| yawed(&end, TURN_DEG * k as f32 / TURN_N as f32)));
    let turned = *cams.last().expect("a turn frame");
    cams.extend(std::iter::repeat_n(turned, HOLD_N));
    Path {
        cams,
        fly,
        pitched,
        heading: best.0,
    }
}

/// The world of `pose` over the chunks within `r` of the eye's in x and y (any heading of a
/// 5 s fly lies inside 3), from 2 under the lower of the eye's and the ground's chunk to 3
/// over the higher; not mirrored.
fn world_at(pose: &Pose, eye: [f32; 3], r: i32) -> svet_world::World {
    use flythrough::outdoor::OutdoorSource;
    use flythrough::trees::{ForestSource, LEAF_FILL, TreeDensity};
    use svet_world::{CHUNK_EDGE, ChunkExtent, World};
    let density = if pose.forest {
        TreeDensity::Forest
    } else {
        TreeDensity::Sparse
    };
    let source = ForestSource::new(OutdoorSource::new(1), density, LEAF_FILL);
    let ch = |v: f32| (v.floor() as i32).div_euclid(CHUNK_EDGE);
    let g = source
        .base()
        .height(eye[0].floor() as i32, eye[1].floor() as i32) as f32;
    let [cx, cy] = [ch(eye[0]), ch(eye[1])];
    let (zl, zh) = (ch(eye[2]).min(ch(g)), ch(eye[2]).max(ch(g)));
    let extent = ChunkExtent::new([cx - r, cy - r, zl - 2], [cx + r + 1, cy + r + 1, zh + 4]);
    let t = std::time::Instant::now();
    let world = World::generate(extent, &source, &flythrough::outdoor_materials());
    println!(
        "{}: chunks {:?}..{:?} generated in {:.1} s",
        pose.name,
        [cx - r, cy - r, zl - 2],
        [cx + r + 1, cy + r + 1, zh + 4],
        t.elapsed().as_secs_f64()
    );
    world
}

/// The subject's pose and start camera.
fn subject_pose(subject: &str) -> (Pose, Camera) {
    let named = |n: &str| POSES.iter().find(|p| p.name == n).expect("a pose");
    match subject {
        "T" => {
            let p = named("terraces");
            let pose = Pose { ..*p };
            (pose, camera_of(p.eye, p.yaw, p.pitch))
        }
        "C" => {
            let p = named("canopy");
            let (eye, yaw, pitch) = CANOPY_START;
            (
                Pose {
                    eye,
                    yaw,
                    pitch,
                    ..*p
                },
                camera_of(eye, yaw, pitch),
            )
        }
        "S" => {
            use flythrough::outdoor::{OutdoorSource, Spawn};
            let (eye, facing) = OutdoorSource::new(1).spawn(Spawn::Inside);
            let p = eye.0.map(|v| v as f32 + 0.5);
            let d = facing.map(|v| v as f32);
            let cam = flythrough::camera_looking_at(p, [p[0] + d[0], p[1] + d[1], p[2] + d[2]]);
            let cam = camera_of(p, cam.yaw.to_degrees(), cam.pitch.to_degrees());
            (
                Pose {
                    name: "slot",
                    eye: p,
                    yaw: cam.yaw.to_degrees(),
                    pitch: cam.pitch.to_degrees(),
                    minutes: 12.0 * 60.0,
                    forest: false,
                },
                cam,
            )
        }
        other => panic!("subject {other}"),
    }
}

/// One queue step at `cam` as the fly-through steps it: the view's frustum and focal length,
/// the frame step's interval, after a device wait ([`support::baked::queue_step_at`]).
fn step_queue(rig: &mut Rig, f: &mut Frame, cam: &Camera) -> svet_render::QueueStats {
    support::baked::queue_step_at(rig, f, cam, 1.0 / f64::from(HZ))
}

/// One frame at `cam`: the queue's step, the frame, a wait on the device.
fn frame_at(rig: &mut Rig, s: &mut Stepper, cam: &Camera) -> svet_render::QueueStats {
    let q = step_queue(rig, &mut s.frame, cam);
    s.run(rig, cam);
    rig.device
        .poll(svet_render::wgpu::PollType::wait_indefinitely())
        .expect("the device polls");
    q
}

/// The census ([`support::baked::census`]): chunks settled, chunks the lighting pixels' hits lie
/// in, coarse blocks among them.
fn census(rig: &Rig, f: &Frame, cam: &Camera) -> (usize, usize, usize) {
    let c = support::baked::census(rig, f, cam);
    (c.settled, c.chunks, c.coarse)
}

/// The shares of all lighting pixels: sunlit ground, sunlit leaf, shadowed (any material).
fn shares(rig: &Rig, f: &Frame, cam: &Camera, arm: &Arm, internal: [u32; 2]) -> [f64; 3] {
    let m = class_map(rig, f, cam, arm, internal);
    let n = m.len().max(1) as f64;
    let count = |p: &dyn Fn(u8) -> bool| m.iter().filter(|&&k| k != 255 && p(k)).count() as f64;
    [
        count(&|k| k / 9 == 0 && (k % 9) / 3 == 0) / n,
        count(&|k| k / 9 == 0 && (k % 9) / 3 == 2) / n,
        count(&|k| k / 9 == 1) / n,
    ]
}

/// The `native` arm's internal size: `T`'s 1280 x 720 (Plan 0100 Phase 8's native run); `S`'s
/// the default arm's internal size at the window (1687 x 919, the window over the scale of 1.5),
/// so the two differ only by the upscaler.
fn native_size(subject: &str) -> [u32; 2] {
    match subject {
        "T" => [1280, 720],
        "S" => [1687, 919],
        other => panic!("subject {other} has no native arm"),
    }
}

/// The rig's frame for `subject` and `arm`, the window's settings applied over it; the fields
/// that changed.
fn window_frame(rig: &Rig, pose: &Pose, subject: &str, arm_name: &str) -> (Frame, Arm, String) {
    let spec = if arm_name == "native" {
        let [w, h] = native_size(subject);
        format!("off1,out={w}x{h}")
    } else {
        format!("default,out={}x{}", WINDOW[0], WINDOW[1])
    };
    let arm = Arm::from_spec(&spec);
    let mut f = frame_for(rig, &arm);
    f.set_sky(Some(
        flythrough::daylight::Clock {
            minutes: f64::from(pose.minutes),
            ..Default::default()
        }
        .sky(flythrough::outdoor::BASE_Z as f32),
    ));
    let flags = window_flags(subject, arm_name);
    let ch = f.apply_settings(&rig.device, &rig.queue, &window_settings(&flags));
    f.set_dt(1.0 / HZ);
    let note = format!(
        "the window's flags {flags:?}: live {:?} settled {:?} rebuild (not applied) {:?}",
        ch.live, ch.settled, ch.rebuild
    );
    (f, arm, note)
}

/// RGBA to RGB.
fn rgb_of(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect()
}

/// The present's blit of `src` onto a texture of the window's size, read back (RGBA).
fn presented(
    rig: &Rig,
    presenter: &svet_render::Presenter,
    src: &svet_render::wgpu::Texture,
) -> Vec<u8> {
    use svet_render::wgpu;
    let out = rig.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("clip present"),
        size: wgpu::Extent3d {
            width: WINDOW[0],
            height: WINDOW[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = out.create_view(&Default::default());
    let mut enc = rig.device.create_command_encoder(&Default::default());
    presenter.encode(&rig.device, &mut enc, src, &view);
    rig.queue.submit([enc.finish()]);
    svet_render::debug::read_rgba8(&rig.device, &rig.queue, &out)
}

fn anchor_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../live/anchor");
    std::fs::create_dir_all(&dir).expect("live/anchor");
    dir
}

/// The 1-minute load average.
fn load1() -> String {
    std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|s| s.split_whitespace().next().map(str::to_owned))
        .unwrap_or_default()
}

/// One clip. `opts` (Plan 0104 Phase 4), comma-separated: `keys` writes `<stem>.keys` beside it
/// (`keys.rs`); `pin<N>` pins the exposure from frame `N + 1` on at the value metered after
/// frame `N` (`lighting.exposure` manual).
fn clip(subject: &str, arm_name: &str, stem: &str, opts: &str) {
    let (pose, cam0) = subject_pose(subject);
    let world = world_at(&pose, cam0.position.to_array(), 8);
    let path = plan_path(&world, &cam0);
    println!(
        "{stem}: eye {:?} yaw {:.1} pitch {:.1}; fly {} frames ({:.2} s; pitched {}), level leg's \
         heading offset {} deg; {} frames",
        cam0.position.to_array(),
        cam0.yaw.to_degrees(),
        cam0.pitch.to_degrees(),
        path.fly,
        path.fly as f32 / HZ,
        path.pitched,
        path.heading,
        path.cams.len()
    );
    let eye = svet_world::VoxelCoord(cam0.position.to_array().map(|v| v.floor() as i32));
    let Some(mut rig) = support::terms::mirrored(world, eye) else {
        return;
    };
    let rig = &mut rig;
    let (f, arm, note) = window_frame(rig, &pose, subject, arm_name);
    println!("{note}");
    let internal = f.internal_size();
    let mut s = Stepper::new(rig, f);
    let t0 = std::time::Instant::now();
    let load0 = load1();
    // The settle at the start, a device wait every frame: until no chunk in view waits, no job
    // is open and no block fades, and the census is whole; or, where the bounded world's edge
    // keeps a chunk in view waiting, until no job is open, no block fades and the census is
    // whole, read every 600 frames.
    let mut n = 0usize;
    let cen = loop {
        let q = frame_at(rig, &mut s, &cam0);
        n += 1;
        let quiet = q.fading == 0 && q.jobs == 0;
        let drained = quiet && q.drain_waiting == 0 && !q.draining;
        if n >= SETTLE_MIN && (drained || n.is_multiple_of(600)) {
            let cen = census(rig, &s.frame, &cam0);
            if n.is_multiple_of(1200) {
                println!(
                    "settle {n}: census {}/{} coarse {}; depth {} jobs {} waiting in view {} \
                     fading {} in flight full {}",
                    cen.0,
                    cen.1,
                    cen.2,
                    q.depth,
                    q.jobs,
                    q.drain_waiting,
                    q.fading,
                    q.in_flight_full
                );
            }
            if (quiet && cen.0 == cen.1) || n >= SETTLE_MAX {
                println!(
                    "settled {n} frames, {:.1} s: depth {} jobs {} coarse {} waiting in view {} \
                     fading {}",
                    t0.elapsed().as_secs_f64(),
                    q.depth,
                    q.jobs,
                    q.coarse,
                    q.drain_waiting,
                    q.fading
                );
                break cen;
            }
        }
    };
    let whole = if cen.0 == cen.1 { "whole" } else { "NOT WHOLE" };
    let census_line = format!("census {}/{} ({whole}; coarse {})", cen.0, cen.1, cen.2);
    println!("{census_line}");
    let dir = anchor_dir();
    let mkv = dir.join(format!("{stem}.mkv"));
    let mut ff = std::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
        ])
        .args([
            "-s",
            &format!("{}x{}", WINDOW[0], WINDOW[1]),
            "-framerate",
            "55",
            "-i",
            "-",
        ])
        .args([
            "-c:v",
            "libx264rgb",
            "-qp",
            "0",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "rgb24",
        ])
        .args(["-map_metadata", "-1", "-f", "matroska"])
        .arg(&mkv)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .expect("ffmpeg starts");
    let mut pipe = ff.stdin.take().expect("ffmpeg's stdin");
    let presenter = (arm_name == "native").then(|| {
        svet_render::Presenter::new(&rig.device, svet_render::wgpu::TextureFormat::Rgba8Unorm)
    });
    let header = format!(
        "# clip {stem}  {}x{}  step 1/55  bake lockstep  {census_line}\n",
        WINDOW[0], WINDOW[1]
    );
    let mut side = header.clone();
    side.push_str("frame eye_x eye_y eye_z yaw pitch exposure baked fading\n");
    let mut sh = Vec::new();
    let mut keys = opts
        .split(',')
        .any(|o| o == "keys")
        .then(|| super::keys::Keys::create(&dir.join(format!("{stem}.keys")), WINDOW));
    let pin: Option<usize> = opts
        .split(',')
        .find_map(|o| o.strip_prefix("pin"))
        .map(|n| n.parse().expect("pin<frame>"));
    let t1 = std::time::Instant::now();
    for (i, cam) in path.cams.iter().enumerate() {
        let q = frame_at(rig, &mut s, cam);
        if let Some(k) = keys.as_mut() {
            k.frame(rig, &s.frame, cam);
        }
        if i == 30 {
            // The owner's cap: project the run from its first 30 frames, stop past 20 minutes.
            let proj = t1.elapsed().as_secs_f64() * path.cams.len() as f64 / 31.0;
            println!(
                "projection after 31 frames: {proj:.0} s for {} frames (settle {:.1} s before)",
                path.cams.len(),
                t1.duration_since(t0).as_secs_f64()
            );
            assert!(proj < 1200.0, "projected past 20 minutes: stopped");
        }
        let rgba = match &presenter {
            Some(p) => presented(rig, p, s.frame.target()),
            None => svet_render::debug::read_rgba8(&rig.device, &rig.queue, s.frame.target()),
        };
        let rgb = rgb_of(&rgba);
        if i == 0 {
            support::write_png(
                &dir.join(format!("{stem}-f1.png")),
                &rgba,
                WINDOW[0],
                WINDOW[1],
            );
            std::fs::write(dir.join(format!("{stem}-f1.rgb")), &rgb).expect("frame 1");
        }
        std::io::Write::write_all(&mut pipe, &rgb).expect("ffmpeg takes the frame");
        let st = svet_render::debug::read_buffer(
            &rig.device,
            &rig.queue,
            s.frame.exposure_state(),
            0,
            32,
        );
        let e = svet_render::ExposureState::from_bytes(&st).current;
        if pin == Some(i) {
            let mut set = s.frame.settings();
            set.lighting.exposure = svet_render::Exposure::Manual(e);
            let ch = s.frame.apply_settings(&rig.device, &rig.queue, &set);
            println!(
                "exposure pinned at {e:.6e} after frame {i}: live {:?} settled {:?} rebuild {:?}",
                ch.live, ch.settled, ch.rebuild
            );
        }
        let p = cam.position.to_array();
        side.push_str(&format!(
            "{i} {:.3} {:.3} {:.3} {:.3} {:.3} {e:.6e} {} {}\n",
            p[0],
            p[1],
            p[2],
            cam.yaw.to_degrees(),
            cam.pitch.to_degrees(),
            q.baked,
            q.fading
        ));
        if i % 55 == 0 {
            sh.push(shares(rig, &s.frame, cam, &arm, internal));
        }
    }
    drop(pipe);
    if let Some(k) = keys {
        println!(
            "keys: {} units, {} samples a frame, {:.1} s spent",
            k.units(),
            k.samples,
            k.secs
        );
        k.finish();
    }
    let st = ff.wait().expect("ffmpeg ends");
    assert!(st.success(), "ffmpeg failed");
    std::fs::write(dir.join(format!("{stem}.txt")), &side).expect("the side file");
    let mean = |k: usize| 100.0 * sh.iter().map(|v| v[k]).sum::<f64>() / sh.len().max(1) as f64;
    let min = |k: usize| 100.0 * sh.iter().map(|v| v[k]).fold(f64::MAX, f64::min);
    print!("{header}");
    println!(
        "{stem} on {}: internal {internal:?}; {} frames in {:.1} s; shares of lighting pixels, \
         a frame a second, mean (least): sunlit ground {:.1} % ({:.1}), sunlit leaf {:.1} % \
         ({:.1}), shadowed {:.1} % ({:.1}); load {load0} at the start, {} at the end",
        rig.adapter,
        path.cams.len(),
        t1.elapsed().as_secs_f64(),
        mean(0),
        min(0),
        mean(1),
        min(1),
        mean(2),
        min(2),
        load1()
    );
}

/// The canopy start: eyes on a 100-voxel grid within 300 voxels (30 m) of the canopy eye, at
/// its height over the ground, yaw in `yaw_step`-degree steps, each pitch of `pitches`; each start's
/// move planned, its view read under the defaults (three frames at each of five cameras: the
/// start, the fly's middle, the stop, the turn's middle, the last) and the shares of sunlit
/// ground, sunlit leaf and shadowed lighting pixels averaged. The starts that hold at least 10,
/// 5 and 20 % are printed nearest first; else the ten nearest to holding them. No bake: the
/// sun's state is the column map's.
fn search(yaw_step: f32, pitches: &[f32]) {
    // About the canopy eye of Plan 0100's window, not the start picked from it.
    let pose = Pose {
        ..*POSES
            .iter()
            .find(|p| p.name == "canopy")
            .expect("the canopy pose")
    };
    let e = pose.eye;
    let source = flythrough::outdoor::OutdoorSource::new(1);
    let ground = |x: f32, y: f32| source.height(x.floor() as i32, y.floor() as i32) as f32;
    let lift = e[2] - ground(e[0], e[1]);
    let world = world_at(&pose, e, 10);
    let mut starts = Vec::new();
    for dx in -3..=3 {
        for dy in -3..=3 {
            if dx * dx + dy * dy > 9 {
                continue;
            }
            let (x, y) = (e[0] + 100.0 * dx as f32, e[1] + 100.0 * dy as f32);
            for k in 0..(360.0 / yaw_step) as usize {
                for &pitch in pitches {
                    starts.push(([x, y, ground(x, y) + lift], yaw_step * k as f32, pitch));
                }
            }
        }
    }
    let eye = svet_world::VoxelCoord(e.map(|v| v.floor() as i32));
    let plans: Vec<_> = starts
        .iter()
        .map(|&(p, yaw, pitch)| {
            let c = camera_of(p, yaw, pitch);
            clear(&world, &c).then(|| plan_path(&world, &c))
        })
        .collect();
    let Some(mut rig) = support::terms::mirrored(world, eye) else {
        return;
    };
    let rig = &mut rig;
    let (f, arm, note) = window_frame(rig, &pose, "C", "default");
    println!("{note}");
    let internal = f.internal_size();
    let mut s = Stepper::new(rig, f);
    let mut rows = Vec::new();
    for (&(p, yaw, pitch), plan) in starts.iter().zip(&plans) {
        let Some(plan) = plan else { continue };
        let n = plan.cams.len();
        let picks = [
            0,
            plan.fly / 2,
            plan.fly,
            plan.fly + HOLD_N + TURN_N / 2,
            n - 1,
        ];
        let mut acc = [0.0; 3];
        for &j in &picks {
            for _ in 0..3 {
                s.run(rig, &plan.cams[j]);
            }
            let v = shares(rig, &s.frame, &plan.cams[j], &arm, internal);
            (0..3).for_each(|k| acc[k] += v[k] / picks.len() as f64);
        }
        let short = (0.10 - acc[0]).max(0.0) + (0.05 - acc[1]).max(0.0) + (0.20 - acc[2]).max(0.0);
        let d = ((p[0] - e[0]).powi(2) + (p[1] - e[1]).powi(2)).sqrt();
        rows.push((short, d, p, yaw, pitch, acc, plan.fly));
    }
    rows.sort_by(|a, b| {
        (a.0 > 0.0, a.1, a.0)
            .partial_cmp(&(b.0 > 0.0, b.1, b.0))
            .expect("finite")
    });
    let ok = rows.iter().filter(|r| r.0 == 0.0).count();
    println!(
        "{} starts read ({} not clear), {ok} hold 10 / 5 / 20 %",
        rows.len(),
        starts.len() - rows.len()
    );
    if ok == 0 {
        rows.sort_by(|a, b| (a.0, a.1).partial_cmp(&(b.0, b.1)).expect("finite"));
    }
    let long = rows.iter().filter(|r| r.6 >= FLY_MIN).take(6);
    for (k, r) in rows.iter().take(12).chain(long).enumerate() {
        if k == 12 {
            println!("of those whose fly is at least 4 s:");
        }
        println!(
            "start {:?} yaw {} pitch {}: {:.0} voxels off; sunlit ground {:.1} %, sunlit leaf \
             {:.1} %, shadowed {:.1} %; fly {} frames",
            r.2,
            r.3,
            r.4,
            r.1,
            100.0 * r.5[0],
            100.0 * r.5[1],
            100.0 * r.5[2],
            r.6
        );
    }
}

mod reading {
    use super::*;

    /// `SVET_CLIP`: `search`, or `<subject>:<arm>[:<stem>]` (module doc).
    #[test]
    #[ignore = "reading"]
    fn the_owners_move_as_a_clip() {
        let var = std::env::var("SVET_CLIP").unwrap_or_else(|_| "T:default".into());
        // `search[:YAW_STEP:P1,P2,..]`: the canopy start's search (45 degrees, -31.8 and -15).
        if let Some(rest) = var.strip_prefix("search") {
            let mut it = rest.trim_start_matches(':').split(':');
            let step = it
                .next()
                .filter(|v| !v.is_empty())
                .map_or(45.0, |v| v.parse().expect("a yaw step"));
            let pitches: Vec<f32> = it.next().map_or(vec![-31.8, -15.0], |v| {
                v.split(',').map(|p| p.parse().expect("a pitch")).collect()
            });
            search(step, &pitches);
            return;
        }
        let mut it = var.split(':');
        let subject = it.next().expect("a subject").to_owned();
        let arm = it.next().unwrap_or("default").to_owned();
        let stem = it
            .next()
            .map_or_else(|| format!("{subject}-{arm}"), str::to_owned);
        clip(&subject, &arm, &stem, it.next().unwrap_or(""));
    }
}
