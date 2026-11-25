//! Plan 0100 Phase 8b: the wobble in the owner's move, a reading. The window's move (the
//! fly-through's unshifted fly speed, 3 m/s, forward across the ground for 5 s, a stop held 5 s,
//! a turn of 60 degrees over 40 frames, held 5 s) at three poses, at the size the `SVET_WOBBLE`
//! arm's `out=WxH` names (the window's 2530 x 1378). The measure sees a flicker, not the
//! picture's motion: a display pixel **reverses** at frame t when its luma moved by over 4
//! levels from t-1 to t and by over 4 the other way from t to t+1. Classes come from `calm`'s
//! run of the same pose and move (the same camera path and jitter, so the same hits), at the
//! middle frame's lighting pixel: sunlit (the key light falls and its value is at least 0.5),
//! shadowed (it falls, under 0.5), turned away; each split by the hit's material (ground, trunk,
//! leaf) and by the far normal's weight (0, between, 1). `calm` writes the class maps under
//! `live/p8b/classes/`; every other arm reads them. The bake as the fly-through configures it;
//! before the move the queue steps at the pose until it has drained (600 frames at least), then
//! the exposure is pinned at its settled value; the queue steps through the move as the
//! fly-through steps it. Nothing is asserted.
//!
//! `SVET_POSE`: `terraces`, `canopy` or `still`. `SVET_MOVE`: `owner` (the move) or `approach`
//! (from 20 m back along the view's heading, flown at the same speed to the pose, held 60).

use super::*;

/// The settle at the pose before the move, the window's pre-roll drain: at least `SETTLE_T`
/// frames, then until no block is fading and either no chunk in view waits for its record or no
/// chunk has baked over `SETTLE_FLAT` frames (a bounded world's edge chunks may never land), at
/// most `SETTLE_MAX`. Each settle frame waits for the device, as the window's frame does, so
/// the queue's readbacks land.
const SETTLE_T: usize = 720;
const SETTLE_FLAT: usize = 1200;
const SETTLE_MAX: usize = 36000;
/// The move's segments in frames at 60 Hz: the fly, the stop, the turn, the hold after.
const FLY_T: usize = 300;
const STOP_T: usize = 300;
const TURN_T: usize = 40;
const HOLD_T: usize = 300;
/// The stop's first frames, read apart.
const AFTER_STOP: usize = 30;
/// The turn in degrees.
const TURN_DEG: f32 = 60.0;
/// The fly-through's unshifted fly speed (`fly.rs`'s `SPEED`), voxels a second.
pub(super) const FLY_VOX_S: f32 = 30.0;
/// The approach's length in voxels (20 m) and the hold after it.
const APPROACH_VOX: f32 = 200.0;
const APPROACH_HOLD: usize = 60;

/// A pose: the eye in voxels, yaw and pitch in degrees, the clock's minutes, the trees.
pub(super) struct Pose {
    pub(super) name: &'static str,
    pub(super) eye: [f32; 3],
    pub(super) yaw: f32,
    pub(super) pitch: f32,
    pub(super) minutes: f32,
    pub(super) forest: bool,
}

/// The poses Phase 8b names. The still's view angles were set by eye against the owner's still
/// (`live/window100/3d-record-sunspots.png`, the HUD gives no angles).
pub(super) const POSES: [Pose; 3] = [
    Pose {
        name: "terraces",
        eye: [496.0, -648.7, 456.7],
        yaw: 34.5,
        pitch: -46.2,
        minutes: 12.0 * 60.0,
        forest: false,
    },
    Pose {
        name: "canopy",
        eye: [-9.5, 915.1, 595.8],
        yaw: 90.0,
        pitch: -31.8,
        minutes: 15.0 * 60.0,
        forest: true,
    },
    Pose {
        name: "still",
        eye: [-414.7, 1036.3, 520.0],
        yaw: STILL_YAW,
        pitch: STILL_PITCH,
        minutes: 15.0 * 60.0,
        forest: true,
    },
];

/// The still's view, by eye against the owner's still.
const STILL_YAW: f32 = 200.0;
const STILL_PITCH: f32 = -44.0;

pub(super) fn camera_of(eye: [f32; 3], yaw: f32, pitch: f32) -> Camera {
    Camera {
        position: eye.into(),
        yaw: yaw.to_radians(),
        pitch: pitch.to_radians(),
        vfov: 70f32.to_radians(),
    }
}

/// A move's segments: a name and its frame range (into the cameras).
type Segments = Vec<(&'static str, std::ops::Range<usize>)>;

/// The cameras of the move from `cam0`, and the segments' frame ranges (into the cameras): the
/// owner's move with a fly of `fly` frames, or an approach of `approach` frames ending at `cam0`.
fn path(cam0: &Camera, mv: &str, fly: usize, approach: usize) -> (Camera, Vec<Camera>, Segments) {
    let step = FLY_VOX_S / 60.0 * VOXEL_M;
    if mv == "approach" {
        let n = approach;
        let start = moved(cam0, -step * n as f32);
        let mut cams: Vec<Camera> = (1..=n).map(|k| moved(&start, step * k as f32)).collect();
        cams.extend(std::iter::repeat_n(
            *cams.last().expect("a frame"),
            APPROACH_HOLD,
        ));
        let segs = vec![
            ("approach", 0..n),
            ("approach last 2 s", n.saturating_sub(120)..n),
            ("held", n..n + APPROACH_HOLD),
        ];
        return (start, cams, segs);
    }
    let mut cams: Vec<Camera> = (1..=fly).map(|k| moved(cam0, step * k as f32)).collect();
    let end = cams.last().copied().unwrap_or(*cam0);
    cams.extend(std::iter::repeat_n(end, STOP_T));
    cams.extend((1..=TURN_T).map(|k| yawed(&end, TURN_DEG * k as f32 / TURN_T as f32)));
    let turned = *cams.last().expect("a turn frame");
    cams.extend(std::iter::repeat_n(turned, HOLD_T));
    let (a, b, c) = (fly, fly + STOP_T, fly + STOP_T + TURN_T);
    let segs = vec![
        ("flying", 0..a),
        ("stop first 30", a..a + AFTER_STOP),
        ("held", a + AFTER_STOP..b),
        ("turning", b..c),
        ("held after", c..c + HOLD_T),
    ];
    (*cam0, cams, segs)
}

/// The clearance the fly keeps from every solid voxel, in voxels (a box this far each way).
pub(super) const CLEAR_VOX: i32 = 8;

/// Whether no solid voxel of `world` lies within [`CLEAR_VOX`] of `cam`'s eye.
pub(super) fn clear(world: &svet_world::World, cam: &Camera) -> bool {
    let e = cam.position.to_array().map(|v| v.floor() as i32);
    let r = CLEAR_VOX;
    (-r..=r).all(|dz| {
        (-r..=r).all(|dy| {
            (-r..=r).all(|dx| {
                world
                    .material_at(svet_world::VoxelCoord([e[0] + dx, e[1] + dy, e[2] + dz]))
                    .is_none_or(|m| m == svet_world::AIR)
            })
        })
    })
}

/// The world of `pose` over the chunks within 5 of every camera on the path in x and y, from 2
/// under the lowest of the eye's and the ground's chunks to 3 over the highest; the lowest
/// clearance over the ground along the path and the cameras inside a solid voxel.
fn pose_rig(pose: &Pose, cams: &[Camera], start: &Camera) -> Option<Rig> {
    use flythrough::outdoor::OutdoorSource;
    use flythrough::trees::{ForestSource, LEAF_FILL, TreeDensity};
    use svet_world::{CHUNK_EDGE, ChunkExtent, VoxelCoord, World};
    let density = if pose.forest {
        TreeDensity::Forest
    } else {
        TreeDensity::Sparse
    };
    let source = ForestSource::new(OutdoorSource::new(1), density, LEAF_FILL);
    let pts: Vec<[f32; 3]> = std::iter::once(start)
        .chain(cams.iter())
        .map(|c| c.position.to_array())
        .collect();
    let h = |p: &[f32; 3]| {
        source
            .base()
            .height(p[0].floor() as i32, p[1].floor() as i32)
    };
    let ch = |v: f32| (v.floor() as i32).div_euclid(CHUNK_EDGE);
    let (mut lo, mut hi) = ([i32::MAX; 3], [i32::MIN; 3]);
    let mut clear = f32::MAX;
    for p in &pts {
        let g = h(p);
        clear = clear.min(p[2] - g as f32);
        for k in 0..2 {
            lo[k] = lo[k].min(ch(p[k]));
            hi[k] = hi[k].max(ch(p[k]));
        }
        lo[2] = lo[2].min(ch(p[2]).min(ch(g as f32)));
        hi[2] = hi[2].max(ch(p[2]).max(ch(g as f32)));
    }
    let extent = ChunkExtent::new(
        [lo[0] - 5, lo[1] - 5, lo[2] - 2],
        [hi[0] + 6, hi[1] + 6, hi[2] + 4],
    );
    let t = std::time::Instant::now();
    let world = World::generate(extent, &source, &flythrough::outdoor_materials());
    let inside = pts
        .iter()
        .filter(|p| {
            world
                .material_at(VoxelCoord(p.map(|v| v.floor() as i32)))
                .is_some_and(|m| m != svet_world::AIR)
        })
        .count();
    println!(
        "{}: chunks {:?}..{:?} generated in {:.1} s; the lowest clearance over the terrain along \
         the path {clear:.1} voxels; cameras inside a solid voxel {inside} of {}",
        pose.name,
        [lo[0] - 5, lo[1] - 5, lo[2] - 2],
        [hi[0] + 6, hi[1] + 6, hi[2] + 4],
        t.elapsed().as_secs_f64(),
        pts.len()
    );
    let eye = VoxelCoord(start.position.to_array().map(|v| v.floor() as i32));
    support::terms::mirrored(world, eye)
}

/// The classes: sun state (0 sunlit, 1 shadowed, 2 turned away) * 9 + material (0 ground, 1
/// trunk, 2 leaf) * 3 + the far normal's weight bin (0, between, 1); 255 a miss.
const NCLS: usize = 27;

/// One frame's class map over `calm`'s lighting grid.
pub(super) fn class_map(
    rig: &Rig,
    f: &Frame,
    cam: &Camera,
    arm: &Arm,
    internal: [u32; 2],
) -> Vec<u8> {
    use flythrough::outdoor::{BARK, LEAF, LEAF_2, LEAF_3};
    use svet_render::debug::{read_rgba8, read_rgba16f, read_rgba32f};
    let (d, q) = (&rig.device, &rig.queue);
    let pair = read_rgba16f(d, q, f.visibility_pair());
    let vis = read_rgba8(d, q, f.visibility_samples());
    let hits = read_rgba32f(d, q, f.light_hit_positions());
    let eye = cam.position.to_array();
    let tan_h = (cam.vfov * 0.5).tan() * internal[0] as f32 / internal[1] as f32;
    let [lo, hi] = arm.far_normal;
    hits.iter()
        .enumerate()
        .map(|(g, h)| {
            if h[3] >= 1e30 || !h[3].is_finite() {
                return 255;
            }
            let dv = [h[0] - eye[0], h[1] - eye[1], h[2] - eye[2]];
            let t = (dv[0] * dv[0] + dv[1] * dv[1] + dv[2] * dv[2])
                .sqrt()
                .max(1e-3);
            let w = if hi <= lo {
                0.0
            } else {
                let px = internal[0] as f32 / (2.0 * tan_h * t);
                let x = ((px - lo) / (hi - lo)).clamp(0.0, 1.0);
                1.0 - x * x * (3.0 - 2.0 * x)
            };
            let wb = if w <= 0.01 {
                0
            } else if w >= 0.99 {
                2
            } else {
                1
            };
            let v =
                svet_world::VoxelCoord([0, 1, 2].map(|k| (h[k] + dv[k] / t * 0.01).floor() as i32));
            let m = match rig.world.material_at(v) {
                Some(BARK) => 1,
                Some(m) if [LEAF, LEAF_2, LEAF_3].contains(&m) => 2,
                _ => 0,
            };
            let s = if vis[4 * g + 2] == 0 {
                2
            } else if support::defects::key_slot(pair[g][1]).0 >= 0.5 {
                0
            } else {
                1
            };
            (s * 9 + m * 3 + wb) as u8
        })
        .collect()
}

/// One frame's reversal counts per class, and the display's.
#[derive(Clone, Copy)]
struct Rev {
    n: [u32; NCLS],
    rev: [u32; NCLS],
    display: u32,
    pixels: u32,
}

fn reversals(a: &[u8], b: &[u8], c: &[u8], dims: [usize; 2], cls: &[u8], gw: usize) -> Rev {
    let [dw, dh] = dims;
    let mut r = Rev {
        n: [0; NCLS],
        rev: [0; NCLS],
        display: 0,
        pixels: (dw * dh) as u32,
    };
    for i in 0..dw * dh {
        let d1 = i32::from(b[i]) - i32::from(a[i]);
        let d2 = i32::from(c[i]) - i32::from(b[i]);
        let rv = (d1 > 4 && d2 < -4) || (d1 < -4 && d2 > 4);
        r.display += u32::from(rv);
        let k = cls[grid_of(i, dw, dh, gw, cls.len())];
        if k == 255 {
            continue;
        }
        r.n[usize::from(k)] += 1;
        r.rev[usize::from(k)] += u32::from(rv);
    }
    r
}

/// The reported groups: a name and the classes it sums.
fn groups() -> Vec<(String, Vec<usize>)> {
    let sun = ["sunlit", "shadowed", "away"];
    let mat = ["ground", "trunk", "leaf"];
    let wb = ["w0", "wmid", "w1"];
    let mut g = Vec::new();
    for (s, sn) in sun.iter().enumerate() {
        g.push(((*sn).to_string(), (0..9).map(|k| s * 9 + k).collect()));
    }
    for (s, sn) in sun.iter().enumerate().take(2) {
        for (m, mn) in mat.iter().enumerate() {
            g.push((
                format!("{sn} {mn}"),
                (0..3).map(|w| s * 9 + m * 3 + w).collect(),
            ));
        }
        for (w, wn) in wb.iter().enumerate() {
            g.push((
                format!("{sn} {wn}"),
                (0..3).map(|m| s * 9 + m * 3 + w).collect(),
            ));
        }
    }
    g.push(("sunlit ground w0".into(), vec![0]));
    g
}

/// Per segment and group: the mean share reversing a frame, the largest, the group's mean
/// pixels a frame; and the display's mean and largest share.
fn segment_lines(
    pose: &str,
    arm: &str,
    rows: &[Rev],
    segs: &[(&str, std::ops::Range<usize>)],
) -> String {
    let mut out = String::new();
    for (name, r) in segs {
        let rs = &rows[r.clone()];
        let dm: Vec<f64> = rs
            .iter()
            .map(|x| f64::from(x.display) / f64::from(x.pixels))
            .collect();
        out.push_str(&format!(
            "{pose}\t{arm}\t{name}\tdisplay\t{:.4}\t{:.4}\t{}\n",
            100.0 * dm.iter().sum::<f64>() / dm.len().max(1) as f64,
            100.0 * dm.iter().copied().fold(0.0, f64::max),
            rs.first().map_or(0, |x| x.pixels),
        ));
        for (g, ks) in groups() {
            let sh: Vec<(f64, u32)> = rs
                .iter()
                .map(|x| {
                    let n: u32 = ks.iter().map(|&k| x.n[k]).sum();
                    let v: u32 = ks.iter().map(|&k| x.rev[k]).sum();
                    (
                        if n > 0 {
                            f64::from(v) / f64::from(n)
                        } else {
                            0.0
                        },
                        n,
                    )
                })
                .collect();
            let mean = sh.iter().map(|s| s.0).sum::<f64>() / sh.len().max(1) as f64;
            let max = sh.iter().map(|s| s.0).fold(0.0, f64::max);
            let npx = sh.iter().map(|s| u64::from(s.1)).sum::<u64>() / sh.len().max(1) as u64;
            out.push_str(&format!(
                "{pose}\t{arm}\t{name}\t{g}\t{:.4}\t{:.4}\t{npx}\n",
                100.0 * mean,
                100.0 * max
            ));
        }
    }
    out
}

mod reading {
    use super::*;

    fn dir_of(sub: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../live/p8b")
            .join(sub);
        std::fs::create_dir_all(&dir).expect("the reading's directory");
        dir
    }

    /// The reading: `SVET_POSE`, `SVET_MOVE`, `SVET_WOBBLE`; the rows to
    /// `live/p8b/rows/<pose>-<move>-<arm>.tsv`.
    #[test]
    #[ignore = "reading"]
    fn the_owners_move() {
        let pose_name = std::env::var("SVET_POSE").unwrap_or_else(|_| "terraces".into());
        let mv = std::env::var("SVET_MOVE").unwrap_or_else(|_| "owner".into());
        let pose = POSES
            .iter()
            .find(|p| p.name == pose_name)
            .expect("SVET_POSE names a pose");
        let arm = Arm::parse();
        let cam0 = camera_of(pose.eye, pose.yaw, pose.pitch);
        let n_app = (APPROACH_VOX / (FLY_VOX_S / 60.0)).round() as usize;
        let (start, cams, _) = path(&cam0, &mv, FLY_T, n_app);
        let Some(mut rig) = pose_rig(pose, &cams, &start) else {
            return;
        };
        let rig = &mut rig;
        // The fly stops before the first frame within CLEAR_VOX of a solid; the approach starts
        // after the last such frame.
        let (fly, app) = if mv == "approach" {
            let last = cams[..n_app].iter().rposition(|c| !clear(&rig.world, c));
            (FLY_T, last.map_or(n_app, |j| n_app - j - 1))
        } else {
            let first = cams[..FLY_T].iter().position(|c| !clear(&rig.world, c));
            (first.unwrap_or(FLY_T), n_app)
        };
        println!(
            "the fly {fly} of {FLY_T} frames, the approach {app} of {n_app} (clear of solids by \
             {CLEAR_VOX} voxels)"
        );
        let (start, cams, segs) = path(&cam0, &mv, fly, app);
        let is_calm = arm.name.starts_with("calm")
            && !arm.name.contains("native")
            && arm
                .name
                .split(',')
                .filter(|t| !t.starts_with("out"))
                .count()
                == 1;
        let internal = if arm.upscaler == "off" {
            arm.out
        } else {
            arm.out.map(|n| (n as f32 / arm.scale).round() as u32)
        };
        let mut f = frame_for(rig, &arm);
        f.set_sky(Some(
            flythrough::daylight::Clock {
                minutes: f64::from(pose.minutes),
                ..Default::default()
            }
            .sky(flythrough::outdoor::BASE_Z as f32),
        ));
        // The bake as the fly-through configures it (Plan 0100 Phase 6's cover rig).
        f.set_bake_coarse(
            svet_render::DEFAULT_COARSE_SAMPLES,
            svet_render::DEFAULT_NEAR,
        );
        f.set_bake_full_first(Some(svet_render::DEFAULT_NEAR));
        f.set_bake_tiers(svet_render::DEFAULT_TIERS);
        f.set_bake_drain(Some(svet_render::DEFAULT_DRAIN));
        let mut s = Stepper::new(rig, f);
        QUEUE_ON.with(|q| q.set(true));
        let t0 = std::time::Instant::now();
        let mut settled = 0usize;
        let (mut last_baked, mut flat_since) = (u64::MAX, 0usize);
        loop {
            let q = coarse::step_queue(rig, &mut s.frame, &start);
            s.run(rig, &start);
            // A frame waits for the last, as the window's does: the queue's readbacks land.
            rig.device
                .poll(svet_render::wgpu::PollType::wait_indefinitely())
                .expect("the device polls");
            settled += 1;
            if q.baked != last_baked {
                (last_baked, flat_since) = (q.baked, settled);
            }
            if settled.is_multiple_of(1200) {
                println!(
                    "settle {settled}: baked {} fading {} coarse {} depth {} waiting in view {} \
                     (reach {} lights {} unbaked {} pool full {} plan stops {:?} in flight full {} \
                     samples {} cap {})",
                    q.baked,
                    q.fading,
                    q.coarse,
                    q.depth,
                    q.drain_waiting,
                    q.waiting_reach,
                    q.waiting_lights,
                    q.unbaked,
                    q.pool_full,
                    q.plan_stops,
                    q.in_flight_full,
                    q.total_samples,
                    q.cap
                );
            }
            let drained = q.fading == 0
                && ((q.drain_waiting == 0 && !q.draining)
                    || (q.baked > 0 && settled - flat_since >= SETTLE_FLAT));
            if (settled >= SETTLE_T && drained) || settled >= SETTLE_MAX {
                println!(
                    "settled {settled} frames, {:.1} s; the queue: depth {} jobs {} coarse {} \
                     waiting in view {} draining {}",
                    t0.elapsed().as_secs_f64(),
                    q.depth,
                    q.jobs,
                    q.coarse,
                    q.drain_waiting,
                    q.draining
                );
                break;
            }
        }
        let st = svet_render::debug::read_buffer(
            &rig.device,
            &rig.queue,
            s.frame.exposure_state(),
            0,
            32,
        );
        let e = svet_render::ExposureState::from_bytes(&st).current;
        s.frame.set_lighting(svet_render::Lighting {
            exposure: svet_render::Exposure::Manual(e),
            ..s.frame.lighting()
        });
        let dims = [arm.out[0] as usize, arm.out[1] as usize];
        let read_luma = |rig: &Rig, s: &Stepper| {
            lumas(&svet_render::debug::read_rgba8(
                &rig.device,
                &rig.queue,
                s.frame.target(),
            ))
        };
        let cls_path = dir_of("classes").join(format!("{}-{mv}.bin", pose.name));
        let mut maps: Vec<Vec<u8>> = Vec::new();
        let mut gw = 0usize;
        if !is_calm {
            let bytes = std::fs::read(&cls_path).expect("calm's class maps: run calm first");
            gw = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
            let gl = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
            maps = bytes[8..].chunks(gl).map(<[u8]>::to_vec).collect();
            assert_eq!(maps.len(), cams.len() + 1, "calm's maps cover the path");
        }
        // Frame 0 at the start, then every camera of the path.
        coarse::step_queue(rig, &mut s.frame, &start);
        s.run(rig, &start);
        let rgba0 = svet_render::debug::read_rgba8(&rig.device, &rig.queue, s.frame.target());
        support::write_png(
            &dir_of("stills").join(format!("{}-{mv}-{}.png", pose.name, arm.name)),
            &rgba0,
            arm.out[0],
            arm.out[1],
        );
        let mut ring = vec![lumas(&rgba0)];
        if is_calm {
            gw = s.frame.light_hit_positions().width() as usize;
            maps.push(class_map(rig, &s.frame, &start, &arm, internal));
        }
        let mut rows = Vec::new();
        for (k, cam) in cams.iter().enumerate() {
            coarse::step_queue(rig, &mut s.frame, cam);
            s.run(rig, cam);
            ring.push(read_luma(rig, &s));
            if is_calm {
                maps.push(class_map(rig, &s.frame, cam, &arm, internal));
            }
            if ring.len() == 3 {
                // The middle frame is camera k-1 (frame k of the maps).
                rows.push(reversals(&ring[0], &ring[1], &ring[2], dims, &maps[k], gw));
                ring.remove(0);
            }
        }
        if is_calm {
            let mut bytes = Vec::with_capacity(8 + maps.len() * maps[0].len());
            bytes.extend_from_slice(&(gw as u32).to_le_bytes());
            bytes.extend_from_slice(&(maps[0].len() as u32).to_le_bytes());
            for m in &maps {
                bytes.extend_from_slice(m);
            }
            std::fs::write(&cls_path, bytes).expect("the class maps");
        }
        // rows[j] is the reversal at camera j (frame j+1); the last camera has none.
        let segs: Vec<(&str, std::ops::Range<usize>)> = segs
            .into_iter()
            .map(|(n, r)| (n, r.start..r.end.min(rows.len())))
            .collect();
        let head = format!(
            "# {} {mv} {} on {}: out {:?} internal {:?}, exposure pinned {e:.3e}, {} frames read \
             (fly {fly}, approach {app}), settled {settled} frames, {:.0} s\n",
            pose.name,
            arm.name,
            rig.adapter,
            arm.out,
            internal,
            cams.len(),
            t0.elapsed().as_secs_f64()
        );
        let body = segment_lines(pose.name, &arm.name, &rows, &segs);
        print!("{head}");
        for l in body.lines().filter(|l| {
            l.contains("\tdisplay\t") || l.contains("\tsunlit\t") || l.contains("\tshadowed\t")
        }) {
            println!("{l}");
        }
        std::fs::write(
            dir_of("rows").join(format!("{}-{mv}-{}.tsv", pose.name, arm.name)),
            format!("{head}{body}"),
        )
        .expect("the rows");
    }
}
