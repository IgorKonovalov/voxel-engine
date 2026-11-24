//! Plan 0086 Phase 16's readings of the second window's defects: a frame stepped on a rig
//! with the light pass timed, the per-frame read-backs (display, hit positions, direct
//! samples, the sky rays cast, the pair's age), the fired lights' placement, a dig at the
//! view's centre, and the live queue stepped on a bounded world.

use super::terms::Rig;
use vxe_render::{Camera, Frame, LightParams, QueueStats, QueueView};

/// One frame's read-backs ([`Stepper::step`]).
pub struct Shot {
    /// The display, rgba8, at the output size.
    pub display: Vec<u8>,
    /// The lighting grid's hit positions (`w` 1e30 or more at a miss).
    pub hits: Vec<[f32; 4]>,
    /// The light pass's direct sample's luminance per lighting pixel.
    pub direct: Vec<f32>,
    /// Whether the lighting pixel cast the sky ray this frame.
    pub cast: Vec<bool>,
    /// The history's age per lighting pixel (the moments' `z`).
    pub age: Vec<f32>,
    /// The sky term's source per lighting pixel ([`source_of`]; with the store counted, else
    /// [`SOURCE_NONE`]).
    pub source: Vec<u8>,
    /// The light pass's ms, where the adapter has timestamps.
    pub light_ms: Option<f64>,
    /// Every pass's ms in [`vxe_render::FRAME_PASSES`]' order, where the adapter has
    /// timestamps; empty from [`read`].
    pub passes: Vec<f64>,
    /// The store's black and missing counts.
    pub black: u32,
    pub missing: u32,
    /// Lighting pixels with no brick value that took a value from the missing-light chain.
    pub chained: u32,
}

/// A frame on a rig stepped by hand so its passes are timed ([`vxe_render::PassTimer`]).
pub struct Stepper {
    pub frame: Frame,
    timer: Option<vxe_render::PassTimer>,
}

impl Stepper {
    pub fn new(rig: &Rig, frame: Frame) -> Self {
        let n = vxe_render::FRAME_PASSES.len() as u32;
        Self {
            frame,
            timer: vxe_render::PassTimer::with_passes(&rig.device, &rig.queue, n),
        }
    }

    /// One frame at `cam`, unread.
    pub fn run(&mut self, rig: &Rig, cam: &Camera) {
        self.frame.step(
            &rig.device,
            &rig.queue,
            &rig.gpu,
            cam,
            &LightParams::headlamp(),
        );
    }

    /// One frame at `cam`, timed and not read back: the light pass's ms, where the adapter
    /// has timestamps.
    pub fn timed(&mut self, rig: &Rig, cam: &Camera) -> Option<f64> {
        let f = &mut self.frame;
        f.advance();
        let params = f.params(&rig.gpu, cam, &LightParams::headlamp());
        let mut enc = rig.device.create_command_encoder(&Default::default());
        f.encode(
            &rig.device,
            &rig.queue,
            &mut enc,
            &rig.gpu,
            &params,
            self.timer.as_ref(),
        );
        if let Some(t) = &self.timer {
            t.resolve(&mut enc);
        }
        rig.queue.submit([enc.finish()]);
        let passes = self.timer.as_ref()?.read_all_ms(&rig.device);
        vxe_render::FRAME_PASSES
            .iter()
            .position(|&n| n == "light")
            .and_then(|slot| passes.get(slot).copied())
    }

    /// One frame at `cam`, timed and read back.
    pub fn step(&mut self, rig: &Rig, cam: &Camera) -> Shot {
        let f = &mut self.frame;
        f.advance();
        let params = f.params(&rig.gpu, cam, &LightParams::headlamp());
        let mut enc = rig.device.create_command_encoder(&Default::default());
        f.encode(
            &rig.device,
            &rig.queue,
            &mut enc,
            &rig.gpu,
            &params,
            self.timer.as_ref(),
        );
        if let Some(t) = &self.timer {
            t.resolve(&mut enc);
        }
        rig.queue.submit([enc.finish()]);
        let passes = self
            .timer
            .as_ref()
            .map(|t| t.read_all_ms(&rig.device))
            .unwrap_or_default();
        let light_ms = vxe_render::FRAME_PASSES
            .iter()
            .position(|&n| n == "light")
            .and_then(|slot| passes.get(slot).copied());
        Shot {
            passes,
            ..read(rig, f, light_ms)
        }
    }
}

/// The read-backs of `f`'s last frame.
pub fn read(rig: &Rig, f: &Frame, light_ms: Option<f64>) -> Shot {
    use vxe_render::debug::{read_rgba8, read_rgba16f, read_rgba32f};
    let (d, q) = (&rig.device, &rig.queue);
    let lum = |p: &[f32; 4]| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
    let ss = f.read_store_stats(d, q);
    let mom = read_rgba16f(d, q, f.moments());
    Shot {
        display: read_rgba8(d, q, f.target()),
        hits: read_rgba32f(d, q, f.light_hit_positions()),
        direct: read_rgba16f(d, q, f.direct()).iter().map(lum).collect(),
        cast: read_rgba8(d, q, f.visibility_samples())
            .chunks(4)
            .map(|p| p[3] > 0)
            .collect(),
        age: mom.iter().map(|p| p[2]).collect(),
        source: mom.iter().map(|p| source_of(p[3])).collect(),
        light_ms,
        passes: Vec::new(),
        black: ss.still_black,
        missing: ss.missing,
        chained: ss.fallback,
    }
}

/// The sky term's source words (temporal.wgsl's `SOURCE_*`, written with the store counted).
pub const SOURCE_NONE: u8 = 0;
pub const SOURCE_RECORD: u8 = 1;
pub const SOURCE_RECORD_STALE: u8 = 2;
pub const SOURCE_OWN_SUMMARY: u8 = 3;
pub const SOURCE_COLUMN: u8 = 4;
pub const SOURCE_NO_VALUE: u8 = 5;
pub const SOURCE_MISMATCH: u8 = 6;
/// The source words' names, by word.
pub const SOURCE_NAMES: [&str; 7] = [
    "none",
    "record",
    "record stale",
    "own summary",
    "column",
    "no value",
    "mismatch",
];

/// The source word in a moments' `w` (its kind plus the word over 16).
pub fn source_of(w: f32) -> u8 {
    ((w - w.floor()) * 16.0).round() as u8
}

/// The display's per-pixel level: the largest channel.
pub fn level(display: &[u8]) -> Vec<u8> {
    display
        .chunks(4)
        .map(|p| p[0].max(p[1]).max(p[2]))
        .collect()
}

/// The lighting pixel of display pixel `i` of a `dw`-wide display over a `gw`-wide grid of
/// `gl` pixels.
pub fn grid_of(i: usize, dw: usize, dh: usize, gw: usize, gl: usize) -> usize {
    let gh = gl / gw;
    let (x, y) = (i % dw, i / dw);
    (y * gh / dh) * gw + x * gw / dw
}

/// The first solid voxel of `world` along `dir` from `eye`, by steps of 0.05 voxel, within
/// `reach` voxels: its distance.
pub fn march(world: &vxe_world::World, eye: [f32; 3], dir: [f32; 3], reach: f32) -> Option<f32> {
    let mut t = 0.0;
    while t < reach {
        let p = [0, 1, 2].map(|k| (eye[k] + dir[k] * t).floor() as i32);
        if world
            .material_at(vxe_world::VoxelCoord(p))
            .is_some_and(|m| m != vxe_world::AIR)
        {
            return Some(t);
        }
        t += 0.05;
    }
    None
}

/// The fly-through's `6` five times from `cam`, along the view turned by `yaws` and `down`
/// (degrees): the lights as [`flythrough::lights::TestLights::fire`] sends them.
pub fn fire_five(
    rig: &Rig,
    cam: &Camera,
    yaws: [f32; 5],
    down: f32,
) -> flythrough::lights::TestLights {
    let mut lights = flythrough::lights::TestLights {
        lights: Vec::new(),
        carry: false,
        colour: 0,
    };
    let eye = cam.position.to_array();
    let (fwd, _, _) = cam.basis();
    let f = fwd.to_array();
    let base_yaw = f[1].atan2(f[0]);
    let pitch = f[2].asin() - down.to_radians();
    for y in yaws {
        let a = base_yaw + y.to_radians();
        let dir = [a.cos() * pitch.cos(), a.sin() * pitch.cos(), pitch.sin()];
        let hit = march(&rig.world, eye, dir, flythrough::lights::SHOT_RANGE);
        lights
            .fire(false, eye, dir, hit)
            .expect("the list has room");
    }
    lights
}

/// A 10-voxel box carved at the first solid voxel along `cam`'s view: applied, mirrored and
/// handed to the frame's queue. Returns the box.
pub fn dig_ahead(rig: &mut Rig, f: &mut Frame, cam: &Camera) -> ([i64; 3], [i64; 3]) {
    use super::SyncNow;
    let eye = cam.position.to_array();
    let fwd = cam.basis().0.to_array();
    let t = march(&rig.world, eye, fwd, 400.0).expect("the view hits ground");
    let c = [0, 1, 2].map(|k| (eye[k] + fwd[k] * (t + 0.5)).floor() as i64);
    let (min, max) = (c.map(|v| v - 5), c.map(|v| v + 5));
    let r = rig
        .world
        .apply(&super::queue::box_op(min, max, vxe_world::Mode::Carve))
        .expect("the dig is resident");
    let (lo, hi) = r.aabb.expect("the dig changed voxels");
    rig.gpu
        .sync_now(&rig.world, &rig.device, &rig.queue)
        .expect("the mirror fits");
    f.invalidate(lo.0, hi.0);
    (min, max)
}

/// One step of `f`'s live queue on the bounded world of `rig`, as the fly-through takes it
/// (a fixed 60 Hz frame).
pub fn queue_step(rig: &mut Rig, f: &mut Frame, cam: &Camera) -> QueueStats {
    let view = QueueView {
        eye: cam.position.to_array(),
        forward: cam.basis().0.to_array(),
        resident: None,
        samples: None,
        interval_s: 1.0 / 60.0,
        hold: None,
        half_angle: {
            let [w, h] = f.output_size();
            vxe_render::internals::view_half_angle(cam.vfov, w as f32 / h as f32)
        },
        frustum: None,
        full_radius: None,
        lod_radius: None,
        focal_px: None,
    };
    f.bake_queue_step(&rig.device, &rig.queue, &mut rig.gpu, &rig.world, &view)
        .expect("the queue runs")
        .expect("the queue's step")
}

/// Plan 0098 Phase 8's dig run, from `VXE_DIG` (`SCENE/ARM/DRAIN`, each part optional, else
/// the part of `default`): the scene (`hills` or `forest`), the arm's comma-separated tokens,
/// and the most frames the queue drains before the settle.
pub fn dig_spec(default: (&str, &str, u32)) -> (String, String, u32) {
    let v = std::env::var("VXE_DIG").unwrap_or_default();
    let mut it = v.split('/');
    let mut part = |d: &str| it.next().filter(|s| !s.is_empty()).unwrap_or(d).to_string();
    let scene = part(default.0);
    let arm = part(default.1);
    let drain = part(&default.2.to_string())
        .parse()
        .expect("VXE_DIG's drain");
    (scene, arm, drain)
}

/// The dig's two classes of display pixels (Plan 0098 Phase 8): the hole, a hit inside the dug
/// box dilated by one voxel (the faces the dig made; the edit's history kill's box), and the
/// ring, a hit inside the box dilated by [`DIG_RING`] voxels and not in the hole.
pub const DIG_RING: i64 = 64;
pub const DIG_NONE: u8 = 0;
pub const DIG_RING_CLASS: u8 = 1;
pub const DIG_HOLE_CLASS: u8 = 2;

/// The dig class of hit `h` for the dug box `min ..= max`.
pub fn dig_class(h: &[f32; 4], min: [i64; 3], max: [i64; 3]) -> u8 {
    if h[3] >= 1e30 || !h[3].is_finite() {
        return DIG_NONE;
    }
    let within = |d: i64| {
        (0..3).all(|k| {
            let v = f64::from(h[k]);
            v >= (min[k] - d) as f64 && v <= (max[k] + 1 + d) as f64
        })
    };
    if within(1) {
        DIG_HOLE_CLASS
    } else if within(DIG_RING) {
        DIG_RING_CLASS
    } else {
        DIG_NONE
    }
}

/// The key slot's value and source (pair.wgsl's `key_slot_decode`): 0 the mean, 1 a record, 2
/// the horizon, 3 a landing's fade, 4 none.
pub fn key_slot(y: f32) -> (f32, usize) {
    let e = 1.0 / 512.0;
    if (-e..=1.0 + e).contains(&y) {
        (y.clamp(0.0, 1.0), 0)
    } else if (-2.0 - e..=-1.0 + e).contains(&y) {
        ((-1.0 - y).clamp(0.0, 1.0), 1)
    } else if (2.0 - e..=3.0 + e).contains(&y) {
        ((y - 2.0).clamp(0.0, 1.0), 2)
    } else if (4.0 - e..=5.0 + e).contains(&y) {
        ((y - 4.0).clamp(0.0, 1.0), 3)
    } else {
        (0.0, 4)
    }
}

/// One class's lighting pixels in one frame: their count, the sky source words, the sky rays
/// cast, the shown sky visibility (the pair's x), the key slot's value and source, and the
/// stored part's and the accumulated direct part's luminance over the exposure (radiance).
#[derive(Clone, Copy, Default)]
pub struct DigGrid {
    pub n: u64,
    pub sources: [u64; 7],
    pub cast: u64,
    pub sky_x: f64,
    pub key: f64,
    pub key_src: [u64; 5],
    pub stored: f64,
    pub direct: f64,
}

/// One frame of the dig's reading: the display's luma and class per pixel, the two classes'
/// lighting pixels ([`DigGrid`], ring then hole), the exposure the frame used, the queue's
/// stats of the step before it and the light and temporal passes' ms.
pub struct DigFrame {
    pub label: String,
    pub luma: Vec<u8>,
    pub class: Vec<u8>,
    pub grid: [DigGrid; 2],
    pub exposure: f32,
    pub q: QueueStats,
    pub light_ms: f64,
    pub temporal_ms: f64,
    /// The frame's passes summed (the pass timer's ms).
    pub gpu_ms: f64,
}

/// The frame `s` steps at `cam` read for the dig `min ..= max` at the display size `out`.
pub fn dig_frame(
    rig: &Rig,
    s: &mut Stepper,
    cam: &Camera,
    q: QueueStats,
    (min, max): ([i64; 3], [i64; 3]),
    label: String,
    out: [u32; 2],
) -> DigFrame {
    use vxe_render::debug::{read_buffer, read_rgba16f};
    let shot = s.step(rig, cam);
    let (d, qu) = (&rig.device, &rig.queue);
    let f = &s.frame;
    let exposure =
        vxe_render::ExposureState::from_bytes(&read_buffer(d, qu, f.exposure_state(), 0, 32))
            .previous;
    let lum = |p: &[f32; 4]| f64::from(0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]);
    let pair = read_rgba16f(d, qu, f.visibility_pair());
    let acc = read_rgba16f(d, qu, f.accumulated());
    let dir = read_rgba16f(d, qu, f.direct_accumulated());
    let gl = shot.hits.len();
    let gcls: Vec<u8> = shot.hits.iter().map(|h| dig_class(h, min, max)).collect();
    let mut grid = [DigGrid::default(); 2];
    let e = f64::from(exposure.max(1e-12));
    for g in 0..gl {
        let c = gcls[g];
        if c == DIG_NONE {
            continue;
        }
        let a = &mut grid[usize::from(c - 1)];
        a.n += 1;
        a.sources[usize::from(shot.source[g]).min(6)] += 1;
        a.cast += u64::from(shot.cast[g]);
        a.sky_x += f64::from(pair[g][0]);
        let (k, ks) = key_slot(pair[g][1]);
        a.key += f64::from(k);
        a.key_src[ks] += 1;
        a.stored += lum(&acc[g]) / e;
        a.direct += lum(&dir[g]) / e;
    }
    let (dw, dh) = (out[0] as usize, out[1] as usize);
    let gw = ((gl as f64 * dw as f64 / dh as f64).sqrt().round() as usize).max(1);
    let class = (0..dw * dh)
        .map(|i| gcls[grid_of(i, dw, dh, gw, gl)])
        .collect();
    let luma = shot
        .display
        .chunks(4)
        .map(|p| {
            (0.2126 * f32::from(p[0]) + 0.7152 * f32::from(p[1]) + 0.0722 * f32::from(p[2])).round()
                as u8
        })
        .collect();
    let ms = |name: &str| {
        vxe_render::FRAME_PASSES
            .iter()
            .position(|&p| p == name)
            .and_then(|i| shot.passes.get(i).copied())
            .unwrap_or(f64::NAN)
    };
    DigFrame {
        label,
        luma,
        class,
        grid,
        exposure,
        q,
        light_ms: ms("light"),
        temporal_ms: ms("temporal"),
        gpu_ms: shot.passes.iter().filter(|v| v.is_finite()).sum(),
    }
}

/// A dig frame's display figures against the frame before the dig (`n1`) and the settled frame
/// (the last): the ring's mean luma, its share darker than at N - 1 by over 4 levels and its
/// share darker than both N - 1 and the settled frame by over 4; the hole's mean luma, its share
/// brighter than the settled frame by over 4, and its mean's excess over the settled hole's mean.
#[derive(Clone, Copy, Debug, Default)]
pub struct DigRow {
    pub ring_mean: f64,
    pub ring_dark: f64,
    pub ring_dark_both: f64,
    pub hole_mean: f64,
    pub hole_bright: f64,
    pub hole_excess: f64,
}

/// [`DigRow`]s of `frames`, `n1` the index of N - 1.
pub fn dig_rows(frames: &[DigFrame], n1: usize) -> Vec<DigRow> {
    let last = frames.last().expect("a settled frame");
    let before = &frames[n1];
    let mean_of = |f: &DigFrame, c: u8| {
        let (mut s, mut n) = (0u64, 0u64);
        for (l, k) in f.luma.iter().zip(&f.class) {
            if *k == c {
                s += u64::from(*l);
                n += 1;
            }
        }
        s as f64 / n.max(1) as f64
    };
    let settled_hole = mean_of(last, DIG_HOLE_CLASS);
    frames
        .iter()
        .map(|f| {
            let (mut rn, mut rd, mut rb, mut hn, mut hb) = (0u64, 0u64, 0u64, 0u64, 0u64);
            for i in 0..f.luma.len() {
                let l = i32::from(f.luma[i]);
                if f.class[i] == DIG_RING_CLASS && before.class[i] == DIG_RING_CLASS {
                    rn += 1;
                    let dark_n1 = l < i32::from(before.luma[i]) - 4;
                    rd += u64::from(dark_n1);
                    rb += u64::from(
                        dark_n1
                            && last.class[i] == DIG_RING_CLASS
                            && l < i32::from(last.luma[i]) - 4,
                    );
                }
                if f.class[i] == DIG_HOLE_CLASS && last.class[i] == DIG_HOLE_CLASS {
                    hn += 1;
                    hb += u64::from(l > i32::from(last.luma[i]) + 4);
                }
            }
            let hole_mean = mean_of(f, DIG_HOLE_CLASS);
            DigRow {
                ring_mean: mean_of(f, DIG_RING_CLASS),
                ring_dark: 100.0 * rd as f64 / rn.max(1) as f64,
                ring_dark_both: 100.0 * rb as f64 / rn.max(1) as f64,
                hole_mean,
                hole_bright: 100.0 * hb as f64 / hn.max(1) as f64,
                hole_excess: hole_mean - settled_hole,
            }
        })
        .collect()
}

/// One printed line of a dig frame and its row.
pub fn dig_line(f: &DigFrame, r: &DigRow) -> String {
    let g = |a: &DigGrid| {
        let n = a.n.max(1) as f64;
        let src: Vec<String> = a
            .sources
            .iter()
            .enumerate()
            .filter(|(_, c)| **c > 0)
            .map(|(k, c)| format!("{} {:.0}", SOURCE_NAMES[k], 100.0 * *c as f64 / n))
            .collect();
        let ks: Vec<String> = ["mean", "rec", "hor", "fade", "none"]
            .iter()
            .zip(a.key_src)
            .filter(|(_, c)| *c > 0)
            .map(|(k, c)| format!("{k} {:.0}", 100.0 * c as f64 / n))
            .collect();
        format!(
            "n {} [{}] cast {:.0} % sky x {:.3} key {:.3} [{}] stored {:.1} direct {:.1}",
            a.n,
            src.join(", "),
            100.0 * a.cast as f64 / n,
            a.sky_x / n,
            a.key / n,
            ks.join(", "),
            a.stored / n,
            a.direct / n
        )
    };
    format!(
        "{}: ring luma {:.1} dark {:.2} % both {:.2} %; hole luma {:.1} bright {:.2} % excess {:+.1}; \
         exposure {:.3e}; queue digs {} baked {} fading {} depth {}; ms light {:.3} temporal {:.3}\n  \
         ring {}\n  hole {}",
        f.label,
        r.ring_mean,
        r.ring_dark,
        r.ring_dark_both,
        r.hole_mean,
        r.hole_bright,
        r.hole_excess,
        f.exposure,
        f.q.digs,
        f.q.digs_baked,
        f.q.fading,
        f.q.depth,
        f.light_ms,
        f.temporal_ms,
        g(&f.grid[0]),
        g(&f.grid[1]),
    )
}

/// The p99 and mean of `changes`.
pub fn p99_mean(mut changes: Vec<u8>) -> (u8, f64) {
    if changes.is_empty() {
        return (0, 0.0);
    }
    changes.sort_unstable();
    let p = changes[(changes.len() * 99 / 100).min(changes.len() - 1)];
    let m = changes.iter().map(|&v| f64::from(v)).sum::<f64>() / changes.len() as f64;
    (p, m)
}

/// The fly-through's dig radius (`edits::DIG_RADIUS`, crate-private there): 0.5 m.
pub const FLY_DIG_RADIUS: i64 = 5 * vxe_world::FIXED_ONE;

/// The fly-through's dig (`flythrough_logic`'s `Dig` input, Plan 0098 Phase 14): a capsule of
/// [`FLY_DIG_RADIUS`] from the eye to the first solid voxel along the view (the logic's reach),
/// carved, mirrored and handed to the frame as the fly-through hands an applied op. The logic's
/// collapse after the carve is not run. Returns the changed voxels' inclusive bounds; `None`
/// where nothing changed.
pub fn dig_along(rig: &mut Rig, f: &mut Frame, cam: &Camera) -> Option<([i64; 3], [i64; 3])> {
    use super::SyncNow;
    use vxe_world::{EditOp, FIXED_ONE, Fixed3, Mode};
    let one = FIXED_ONE as f32;
    let origin = cam.position.to_array().map(|v| (v * one).floor() as i64);
    let dir = cam.basis().0.to_array().map(|v| (v * one).round() as i64);
    let reach = flythrough_logic::REACH;
    let t = rig
        .world
        .raycast(Fixed3(origin), dir, reach)
        .ok()
        .flatten()
        .map_or(reach, |h| h.t);
    let end = Fixed3([0, 1, 2].map(|k| origin[k] + dir[k] * t / FIXED_ONE));
    let r = rig
        .world
        .apply(&EditOp::capsule(
            Fixed3(origin),
            end,
            FLY_DIG_RADIUS,
            Mode::Carve,
        ))
        .expect("the dig is resident");
    let (lo, hi) = r.aabb?;
    rig.gpu
        .sync_now(&rig.world, &rig.device, &rig.queue)
        .expect("the mirror fits");
    f.invalidate(lo.0, hi.0);
    Some((lo.0.map(i64::from), hi.0.map(i64::from)))
}

/// One frame of a dig run from above (Plan 0098 Phase 14): the display's luma, and per lighting
/// pixel the hit, the visibility pair (x the shown sky visibility, y the key slot, w the age),
/// the sky source word, whether the sky ray was cast and the store's table word for the hit's
/// chunk (0 none, 1 a block, 2 a coarse block); with the stages read, the filter's output's
/// luminance (lighting grid) and the composite's (FSR 2's input, the march's size); the
/// exposure the frame used, the queue's stats of the step before it, and the light and temporal
/// passes' ms.
pub struct HoleFrame {
    pub label: String,
    pub luma: Vec<u8>,
    pub hits: Vec<[f32; 4]>,
    pub pair: Vec<[f32; 4]>,
    pub source: Vec<u8>,
    pub cast: Vec<bool>,
    pub table: Vec<u8>,
    pub filtered: Vec<f32>,
    pub composite: Vec<f32>,
    pub stored: Vec<f32>,
    pub direct: Vec<f32>,
    pub march: [u32; 2],
    pub exposure: f32,
    pub q: QueueStats,
    pub light_ms: f64,
    pub temporal_ms: f64,
    /// The frame's passes summed (the pass timer's ms).
    pub gpu_ms: f64,
}

/// The frame `s` steps at `cam`, read ([`HoleFrame`]) at the display size `out`; the stages
/// with `stages`; the display written to `png` where given.
#[allow(
    clippy::too_many_arguments,
    reason = "a reading's frame and its switches"
)]
pub fn hole_frame(
    rig: &Rig,
    s: &mut Stepper,
    cam: &Camera,
    q: QueueStats,
    label: String,
    out: [u32; 2],
    stages: bool,
    png: Option<&std::path::Path>,
) -> HoleFrame {
    use vxe_render::debug::{read_buffer, read_rgba16f, read_rgba32f};
    let shot = s.step(rig, cam);
    let (d, qu) = (&rig.device, &rig.queue);
    let f = &s.frame;
    let exposure =
        vxe_render::ExposureState::from_bytes(&read_buffer(d, qu, f.exposure_state(), 0, 32))
            .previous;
    let lum = |p: &[f32; 4]| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
    let pair = read_rgba16f(d, qu, f.visibility_pair());
    let store = f.store();
    let table = shot
        .hits
        .iter()
        .map(|h| {
            if h[3] >= 1e30 || !h[3].is_finite() {
                return 0;
            }
            let c = vxe_world::ChunkCoord([0, 1, 2].map(|k| (h[k].floor() as i32).div_euclid(64)));
            match (store.block(c).is_some(), store.is_coarse(c)) {
                (_, true) => 2,
                (true, false) => 1,
                _ => 0,
            }
        })
        .collect();
    let march = f.internal_size();
    let e = exposure.max(1e-12);
    let (stored, direct) = if stages {
        let st = read_rgba16f(d, qu, f.accumulated())
            .iter()
            .map(|p| lum(p) / e)
            .collect();
        let dr = read_rgba16f(d, qu, f.direct_accumulated())
            .iter()
            .map(|p| lum(p) / e)
            .collect();
        (st, dr)
    } else {
        (Vec::new(), Vec::new())
    };
    let (filtered, composite) = if stages {
        let fl = read_rgba16f(d, qu, f.filtered()).iter().map(lum).collect();
        let cp = f
            .fsr2()
            .map(|x| read_rgba32f(d, qu, x.colour_in()).iter().map(lum).collect())
            .unwrap_or_default();
        (fl, cp)
    } else {
        (Vec::new(), Vec::new())
    };
    if let Some(p) = png {
        super::write_png(&p.to_path_buf(), &shot.display, out[0], out[1]);
    }
    let ms = |name: &str| {
        vxe_render::FRAME_PASSES
            .iter()
            .position(|&p| p == name)
            .and_then(|i| shot.passes.get(i).copied())
            .unwrap_or(f64::NAN)
    };
    HoleFrame {
        label,
        luma: shot
            .display
            .chunks(4)
            .map(|p| {
                (0.2126 * f32::from(p[0]) + 0.7152 * f32::from(p[1]) + 0.0722 * f32::from(p[2]))
                    .round() as u8
            })
            .collect(),
        hits: shot.hits,
        pair,
        source: shot.source,
        cast: shot.cast,
        table,
        filtered,
        composite,
        stored,
        direct,
        march,
        exposure,
        q,
        light_ms: ms("light"),
        temporal_ms: ms("temporal"),
        gpu_ms: shot.passes.iter().filter(|v| v.is_finite()).sum(),
    }
}

/// A dig run from above: its frames from N - 2, N - 1's index, and per dig the index of the
/// first frame after it and its changed voxels' bounds; the display size, whether the arm
/// holds the dig (`Lighting::dig_hold` not off) and the camera's position.
pub struct HoleRun {
    pub frames: Vec<HoleFrame>,
    pub n1: usize,
    pub digs: Vec<(usize, [i64; 3], [i64; 3])>,
    pub out: [u32; 2],
    pub hold: bool,
    pub eye: [f32; 3],
}

/// The display pixel classes of a frame's lighting hits against every dig of the run: the hole
/// (a hit inside a dig's bounds dilated by one voxel), the ring (inside one dilated by
/// [`DIG_RING`], not the hole), none.
pub fn hole_classes(run: &HoleRun, f: &HoleFrame) -> (Vec<u8>, Vec<u8>) {
    let gl = f.hits.len();
    let grid: Vec<u8> = f
        .hits
        .iter()
        .map(|h| {
            run.digs
                .iter()
                .map(|&(_, lo, hi)| dig_class(h, lo, hi))
                .max()
                .unwrap_or(DIG_NONE)
        })
        .collect();
    let (dw, dh) = (run.out[0] as usize, run.out[1] as usize);
    let gw = ((gl as f64 * dw as f64 / dh as f64).sqrt().round() as usize).max(1);
    let display = (0..dw * dh)
        .map(|i| grid[grid_of(i, dw, dh, gw, gl)])
        .collect();
    (grid, display)
}

/// The landing of each dig: the index of the first frame after it whose queue step holds no
/// dig (the frame after the dig's own excluded, as `dig_run` reads it).
pub fn hole_landings(run: &HoleRun) -> Vec<Option<usize>> {
    run.digs
        .iter()
        .map(|&(at, _, _)| (at + 1..run.frames.len()).find(|&i| run.frames[i].q.digs == 0))
        .collect()
}

/// Whether display pixel `i` of `luma` (`w` x `h`) is a dot: over 8 levels from the median of
/// the 5 x 5 display pixels about it (about two march pixels across under FSR 2 at 1.5), with at
/// most two of its 8 neighbours past the median by over 8 levels the same way (an edge's pixels
/// stand out together; a dot stands alone).
pub fn is_dot(luma: &[u8], w: usize, h: usize, i: usize) -> bool {
    let (x, y) = (i % w, i / w);
    let at = |dx: i64, dy: i64| {
        let (xx, yy) = (x as i64 + dx, y as i64 + dy);
        (xx >= 0 && yy >= 0 && xx < w as i64 && yy < h as i64)
            .then(|| i32::from(luma[yy as usize * w + xx as usize]))
    };
    let mut v = [0i32; 25];
    let mut n = 0;
    for dy in -2i64..=2 {
        for dx in -2i64..=2 {
            if let Some(l) = at(dx, dy) {
                v[n] = l;
                n += 1;
            }
        }
    }
    let v = &mut v[..n];
    v.sort_unstable();
    let med = v[n / 2];
    let d = i32::from(luma[i]) - med;
    if d.abs() <= 8 {
        return false;
    }
    let mut along = 0;
    for dy in -1i64..=1 {
        for dx in -1i64..=1 {
            if (dx, dy) != (0, 0)
                && at(dx, dy)
                    .is_some_and(|l| (l - med).signum() == d.signum() && (l - med).abs() > 8)
            {
                along += 1;
            }
        }
    }
    along <= 2
}

/// The run's dig groups: from a dig's frame to the frame before the next group's dig (or the
/// last frame); a dig made before the one ahead of it landed joins that one's group.
pub fn hole_groups(run: &HoleRun) -> Vec<(usize, usize)> {
    let landings = hole_landings(run);
    let mut groups = Vec::new();
    let mut open = None;
    for (k, d) in run.digs.iter().enumerate() {
        let start = *open.get_or_insert(d.0);
        let next = run.digs.get(k + 1);
        if landings[k].is_some_and(|l| next.is_some_and(|n| n.0 <= l)) {
            continue;
        }
        open = None;
        groups.push((start, next.map_or(run.frames.len() - 1, |n| n.0 - 1)));
    }
    groups
}

/// The frames frame `fi` reads as its before and after: the frame before its group's first dig
/// and its group's last frame ([`hole_groups`]; N - 1 and the first group's last before any dig).
pub fn before_after(run: &HoleRun, groups: &[(usize, usize)], fi: usize) -> (usize, usize) {
    let g = groups
        .iter()
        .find(|g| fi <= g.1)
        .copied()
        .unwrap_or((run.n1 + 1, run.frames.len() - 1));
    (g.0 - 1, g.1)
}

/// One frame's figures of Phase 14's image test: the hole's and the ring's shares (%) of white
/// (brighter than the larger of its before and its after ([`before_after`]: the frame before
/// its dig group and the group's settled last) by over 4 levels) and black (darker than the
/// smaller by over 4); the hole's and the ring's dots' share (%, [`is_dot`]); the
/// ring's mean luma over the pixels ring at N - 1 and now, less its mean then; the same of the
/// frame outside the ring (no class at N - 1, now and settled); the hole's mean luma.
#[derive(Clone, Copy, Debug, Default)]
pub struct HoleFigures {
    pub white_hole: f64,
    pub black_hole: f64,
    pub white_ring: f64,
    pub black_ring: f64,
    pub dots: f64,
    pub ring_shift: f64,
    pub out_shift: f64,
    pub hole_mean: f64,
}

/// [`HoleFigures`] of every frame of `run`, with each frame's display classes.
pub fn hole_figures(run: &HoleRun) -> (Vec<HoleFigures>, Vec<Vec<u8>>) {
    hole_figures_at(run, 4)
}

/// [`hole_figures`] with white and black past `level` levels in place of 4.
pub fn hole_figures_at(run: &HoleRun, level: i32) -> (Vec<HoleFigures>, Vec<Vec<u8>>) {
    let classes: Vec<Vec<u8>> = run.frames.iter().map(|f| hole_classes(run, f).1).collect();
    let (w, h) = (run.out[0] as usize, run.out[1] as usize);
    let groups = hole_groups(run);
    let figs = run
        .frames
        .iter()
        .zip(&classes)
        .enumerate()
        .map(|(fi, (f, c))| {
            let (bi, ai) = before_after(run, &groups, fi);
            let (before, after) = (&run.frames[bi].luma, &run.frames[ai].luma);
            let (cb, ca) = (&classes[bi], &classes[ai]);
            let mut fig = HoleFigures::default();
            let (mut nh, mut nr, mut nd, mut dots) = (0u64, 0u64, 0u64, 0u64);
            let (mut wh, mut bh, mut wr, mut br) = (0u64, 0u64, 0u64, 0u64);
            let (mut rs, mut rn, mut os, mut on, mut hs) = (0i64, 0i64, 0i64, 0i64, 0u64);
            for i in 0..f.luma.len() {
                let l = i32::from(f.luma[i]);
                let (b, a) = (i32::from(before[i]), i32::from(after[i]));
                let white = l > b.max(a) + level;
                let black = l < b.min(a) - level;
                if c[i] == DIG_HOLE_CLASS {
                    hs += u64::from(f.luma[i]);
                }
                if c[i] == DIG_HOLE_CLASS && ca[i] == DIG_HOLE_CLASS {
                    nh += 1;
                    wh += u64::from(white);
                    bh += u64::from(black);
                }
                if c[i] == DIG_RING_CLASS && ca[i] == DIG_RING_CLASS && cb[i] == DIG_RING_CLASS {
                    nr += 1;
                    wr += u64::from(white);
                    br += u64::from(black);
                }
                if c[i] != DIG_NONE {
                    nd += 1;
                    dots += u64::from(is_dot(&f.luma, w, h, i));
                }
                if c[i] == DIG_RING_CLASS && cb[i] == DIG_RING_CLASS {
                    rs += i64::from(l - b);
                    rn += 1;
                }
                if c[i] == DIG_NONE && cb[i] == DIG_NONE && ca[i] == DIG_NONE {
                    os += i64::from(l - b);
                    on += 1;
                }
            }
            let pct = |a: u64, n: u64| 100.0 * a as f64 / n.max(1) as f64;
            fig.white_hole = pct(wh, nh);
            fig.black_hole = pct(bh, nh);
            fig.white_ring = pct(wr, nr);
            fig.black_ring = pct(br, nr);
            fig.dots = pct(dots, nd);
            fig.ring_shift = rs as f64 / rn.max(1) as f64;
            fig.out_shift = os as f64 / on.max(1) as f64;
            let nhole = c.iter().filter(|&&k| k == DIG_HOLE_CLASS).count() as f64;
            fig.hole_mean = hs as f64 / nhole.max(1.0);
            fig
        })
        .collect();
    (figs, classes)
}

/// One kind of display pixel in the hole and the ring over a frame, by its lighting pixel: the
/// count, the sky source words, the rays cast, the shown sky visibility and the pair's age, the
/// seed the last dig's frame left (by its age: the record's 8 samples, the column's 2, other),
/// the seed's value, the store's table word (none, a block, coarse), the key slot's value and
/// source, and the shares in the kill box, the held box and the ring's hold.
#[derive(Clone, Copy, Default)]
struct KindRow {
    n: u64,
    sources: [u64; 7],
    cast: u64,
    x: f64,
    age: f64,
    seed: [u64; 3],
    seed_x: f64,
    table: [u64; 3],
    key: f64,
    key_src: [u64; 5],
    kill: u64,
    held: u64,
    ring_hold: u64,
    stored: f64,
    direct: f64,
    filtered: f64,
    after_x: f64,
    after_stored: f64,
}

impl KindRow {
    fn line(&self, name: &str) -> String {
        let n = self.n.max(1) as f64;
        let pc = |c: u64| 100.0 * c as f64 / n;
        let src: Vec<String> = self
            .sources
            .iter()
            .enumerate()
            .filter(|(_, c)| **c > 0)
            .map(|(k, c)| format!("{} {:.0}", SOURCE_NAMES[k], pc(*c)))
            .collect();
        let ks: Vec<String> = ["mean", "rec", "hor", "fade", "none"]
            .iter()
            .zip(self.key_src)
            .filter(|(_, c)| *c > 0)
            .map(|(k, c)| format!("{k} {:.0}", pc(c)))
            .collect();
        format!(
            "    {name:<6} n {:>6} stored {:.2} (after {:.2}) direct {:.2} filtered {:.2} x after {:.3} [{}] cast {:.0} % x {:.3} age {:.1} seed rec {:.0} col {:.0} other {:.0} (x {:.3}) table blk {:.0} coarse {:.0} none {:.0} key {:.3} [{}] kill {:.0} held {:.0} ring hold {:.0}",
            self.n,
            self.stored / n,
            self.after_stored / n,
            self.direct / n,
            self.filtered / n,
            self.after_x / n,
            src.join(", "),
            pc(self.cast),
            self.x / n,
            self.age / n,
            pc(self.seed[0]),
            pc(self.seed[1]),
            pc(self.seed[2]),
            self.seed_x / n,
            pc(self.table[1]),
            pc(self.table[2]),
            pc(self.table[0]),
            self.key / n,
            ks.join(", "),
            pc(self.kill),
            pc(self.held),
            pc(self.ring_hold),
        )
    }
}

/// Whether hit `h` lies in bounds `lo ..= hi` dilated by one voxel (temporal.wgsl's `in_edit`
/// and `in_dig_hold`).
fn in_box(h: &[f32; 4], lo: [i64; 3], hi: [i64; 3]) -> bool {
    (0..3).all(|k| f64::from(h[k]) >= (lo[k] - 1) as f64 && f64::from(h[k]) <= (hi[k] + 2) as f64)
}

/// The median of the 3 x 3 values of `v` (`w` wide) about `i`, and `v[i]`'s distance from it
/// over it (relative; 0 where the median is under 1e-4).
fn rel_from_median(v: &[f32], w: usize, i: usize) -> f32 {
    let h = v.len() / w.max(1);
    let (x, y) = (i % w, i / w);
    let mut m = Vec::with_capacity(9);
    for dy in -1i64..=1 {
        for dx in -1i64..=1 {
            let (xx, yy) = (x as i64 + dx, y as i64 + dy);
            if xx >= 0 && yy >= 0 && (xx as usize) < w && (yy as usize) < h {
                m.push(v[yy as usize * w + xx as usize]);
            }
        }
    }
    m.sort_by(f32::total_cmp);
    let med = m[m.len() / 2];
    if med.abs() < 1e-4 {
        return if v[i].abs() < 1e-4 { 0.0 } else { 1.0 };
    }
    (v[i] - med).abs() / med.abs()
}

/// The top of solid of the column map's cell (`COLUMN_CELL` 2 voxels an edge) holding voxel
/// column `xy`, scanned from `z0` down 256 voxels: the top voxel's z plus 1, or none.
fn cell_top(
    world: &vxe_world::World,
    cache: &mut std::collections::HashMap<[i32; 2], Option<i32>>,
    xy: [i32; 2],
    z0: i32,
) -> Option<i32> {
    let c = [xy[0].div_euclid(2), xy[1].div_euclid(2)];
    *cache.entry(c).or_insert_with(|| {
        (z0 - 256..=z0).rev().find_map(|z| {
            let solid = (0..4).any(|k| {
                let v = vxe_world::VoxelCoord([c[0] * 2 + (k & 1), c[1] * 2 + (k >> 1), z]);
                world.material_at(v).is_some_and(|m| m != vxe_world::AIR)
            });
            solid.then_some(z + 1)
        })
    })
}

/// The openness of arm (a1'), the hole's own (pair.wgsl's `dig_openness`): at hit `p` seen from
/// `eye`, lifted 2.5 voxels along its face's normal, the horizon's tangent the cells' tops make
/// in each of 8 horizontal directions, the largest over the distances 2, 4 .. 12 voxels; the
/// smallest of the 8 (a hole closes every way, a tree beside a face one way); `1 / (1 + t^2)`.
fn openness_min(
    world: &vxe_world::World,
    cache: &mut std::collections::HashMap<[i32; 2], Option<i32>>,
    p: [f32; 3],
    eye: [f32; 3],
) -> f32 {
    let axis = (0..3)
        .min_by(|&a, &b| {
            (p[a] - p[a].round())
                .abs()
                .total_cmp(&(p[b] - p[b].round()).abs())
        })
        .unwrap_or(2);
    let mut q = p;
    q[axis] += 2.5 * (eye[axis] - p[axis]).signum();
    let z0 = q[2] as i32 + 160;
    let mut t_min = f32::MAX;
    for k in 0..8 {
        let a = k as f32 * std::f32::consts::FRAC_PI_4;
        let mut t = 0.0f32;
        for d in (2..=12).step_by(2) {
            let d = d as f32;
            let xy = [q[0] + a.cos() * d, q[1] + a.sin() * d].map(|v| v.floor() as i32);
            if let Some(top) = cell_top(world, cache, xy, z0) {
                t = t.max((top as f32 - q[2]) / d);
            }
        }
        t_min = t_min.min(t);
    }
    1.0 / (1.0 + t_min * t_min)
}

/// The openness of arm (a1) at hit `p` seen from `eye`: the hit lifted 2.5 voxels along its
/// face's normal (the column test's lift), 0 under its own cell's top, else the mean over 8
/// horizontal directions of `cos^2` of the horizon the cells' tops make within 32 voxels (the
/// cosine-weighted share of the sky above a horizon at that elevation).
fn openness(
    world: &vxe_world::World,
    cache: &mut std::collections::HashMap<[i32; 2], Option<i32>>,
    p: [f32; 3],
    eye: [f32; 3],
) -> f32 {
    let axis = (0..3)
        .min_by(|&a, &b| {
            (p[a] - p[a].round())
                .abs()
                .total_cmp(&(p[b] - p[b].round()).abs())
        })
        .unwrap_or(2);
    let mut q = p;
    q[axis] += 2.5 * (eye[axis] - p[axis]).signum();
    let z0 = q[2] as i32 + 160;
    let own =
        |xy: [f32; 2], cache: &mut _| cell_top(world, cache, xy.map(|v| v.floor() as i32), z0);
    if own([q[0], q[1]], cache).is_some_and(|t| t as f32 > q[2]) {
        return 0.0;
    }
    let mut sum = 0.0;
    for k in 0..8 {
        let a = k as f32 * std::f32::consts::FRAC_PI_4;
        let (dx, dy) = (a.cos(), a.sin());
        let mut tan_h = 0.0f32;
        for d in 1..=32 {
            let d = d as f32;
            if let Some(t) = own([q[0] + dx * d, q[1] + dy * d], cache) {
                tan_h = tan_h.max((t as f32 - q[2]) / d);
            }
        }
        sum += 1.0 / (1.0 + tan_h * tan_h);
    }
    sum / 8.0
}

/// Phase 14 step 1's tables of `run` (`world` the run's world after the digs): per frame from N - 1 the figures ([`HoleFigures`]), the exposure, the
/// queue, the passes' ms, and per kind of hole and ring pixel (white, black, dot, the rest) a
/// [`KindRow`]; for the dots the share whose lighting pixel's pair, filter output or composite
/// (FSR 2's input) already stands out of its 3 x 3 (a pair over 0.1 from its median, the others
/// over 25 %); the settled frames' figures twice; and the error table: per dig group, for each
/// candidate value of step 2 at each hole pixel, the display levels it would show against
/// what landed (the mean and the largest distance), by the landed pixel's luma scaled by the
/// candidate's visibility over the landed one to the power 1 / 2.2 (a guess at the display's
/// transfer; pixels whose landed visibility is under 0.02 or luma under 4 are left out).
pub fn hole_tables(run: &HoleRun, world: &vxe_world::World) -> String {
    use std::fmt::Write;
    let (figs, classes) = hole_figures(run);
    let landings = hole_landings(run);
    let groups = hole_groups(run);
    let mut out = String::new();
    let _ = writeln!(
        out,
        "digs (first frame, bounds) {:?}; landings {landings:?}",
        run.digs
    );
    // The held window of each dig: its frame to its landing plus the hold's fade (17 frames).
    let held_until: Vec<usize> = landings
        .iter()
        .map(|l| l.map_or(run.frames.len(), |l| l + 17))
        .collect();
    for (fi, f) in run.frames.iter().enumerate().skip(run.n1) {
        let fig = &figs[fi];
        let _ = writeln!(
            out,
            "{}: white hole {:.2} % ring {:.2} %, black hole {:.2} % ring {:.2} %, dots {:.2} %, ring shift {:+.2}, outside shift {:+.2}, hole mean {:.1}; exposure {:.3e}; queue digs {} in view {} coarse {} depth {}; ms light {:.3} temporal {:.3}",
            f.label,
            fig.white_hole,
            fig.white_ring,
            fig.black_hole,
            fig.black_ring,
            fig.dots,
            fig.ring_shift,
            fig.out_shift,
            fig.hole_mean,
            f.exposure,
            f.q.digs,
            f.q.drain_waiting,
            f.q.coarse,
            f.q.depth,
            f.light_ms,
            f.temporal_ms
        );
        let (rows, stage) = kind_rows(run, fi, &classes, &groups, &held_until);
        for (k, name) in ["white", "black", "dot", "rest"].iter().enumerate() {
            if rows[k].n > 0 {
                let _ = writeln!(out, "{}", rows[k].line(name));
            }
        }
        if rows[2].n > 0 {
            let n = rows[2].n as f64;
            let _ = writeln!(
                out,
                "    dots standing out at: the pair {:.0} %, the filter {:.0} %, the composite {:.0} %",
                100.0 * stage[0] as f64 / n,
                100.0 * stage[1] as f64 / n,
                100.0 * stage[2] as f64 / n
            );
        }
    }
    // The noise of the figures: the last two settled frames read against each other.
    let last = run.frames.len() - 1;
    let _ = writeln!(
        out,
        "settled: white hole {:.2} / {:.2} %, black hole {:.2} / {:.2} %, white ring {:.2} / {:.2} %, black ring {:.2} / {:.2} %, dots {:.2} / {:.2} %",
        figs[last - 1].white_hole,
        figs[last].white_hole,
        figs[last - 1].black_hole,
        figs[last].black_hole,
        figs[last - 1].white_ring,
        figs[last].white_ring,
        figs[last - 1].black_ring,
        figs[last].black_ring,
        figs[last - 1].dots,
        figs[last].dots
    );
    error_table(run, world, &classes, &groups, &mut out);
    out
}

/// One frame's [`KindRow`]s (white, black, dot, the rest) of [`hole_tables`] and its dots'
/// stage counts (the pair, the filter, the composite).
fn kind_rows(
    run: &HoleRun,
    fi: usize,
    classes: &[Vec<u8>],
    groups: &[(usize, usize)],
    held_until: &[usize],
) -> ([KindRow; 4], [u64; 3]) {
    let f = &run.frames[fi];
    let (w, h) = (run.out[0] as usize, run.out[1] as usize);
    let gw_of = |gl: usize| ((gl as f64 * w as f64 / h as f64).sqrt().round() as usize).max(1);
    let gl = f.hits.len();
    let gw = gw_of(gl);
    let (bi, ai) = before_after(run, groups, fi);
    let (before, after) = (&run.frames[bi].luma, &run.frames[ai].luma);
    let seed_frame = run.digs.iter().rev().find(|d| d.0 <= fi).map(|d| d.0);
    let mut rows = [KindRow::default(); 4];
    let mut stage = [0u64; 3];
    for i in 0..f.luma.len() {
        if classes[fi][i] == DIG_NONE {
            continue;
        }
        let l = i32::from(f.luma[i]);
        let (b, a) = (i32::from(before[i]), i32::from(after[i]));
        let kind = if l > b.max(a) + 4 {
            0
        } else if l < b.min(a) - 4 {
            1
        } else if is_dot(&f.luma, w, h, i) {
            2
        } else {
            3
        };
        let g = grid_of(i, w, h, gw, gl);
        let r = &mut rows[kind];
        r.n += 1;
        r.sources[usize::from(f.source[g]).min(6)] += 1;
        r.cast += u64::from(f.cast[g]);
        let af = &run.frames[ai];
        r.after_x += f64::from(af.pair[g][0]);
        if !f.stored.is_empty() {
            r.after_stored += f64::from(af.stored[g]);
            r.stored += f64::from(f.stored[g]);
            r.direct += f64::from(f.direct[g]);
            r.filtered += f64::from(f.filtered[g]) / f64::from(f.exposure.max(1e-12));
        }
        r.x += f64::from(f.pair[g][0]);
        r.age += f64::from(f.pair[g][3]);
        if let Some(sf) = seed_frame {
            let sp = run.frames[sf].pair[g];
            let k = if (sp[3] - 8.0).abs() < 0.5 {
                0
            } else if (sp[3] - 2.0).abs() < 0.5 {
                1
            } else {
                2
            };
            r.seed[k] += 1;
            r.seed_x += f64::from(sp[0]);
        } else {
            r.seed[2] += 1;
        }
        r.table[usize::from(f.table[g]).min(2)] += 1;
        let (kx, ks) = key_slot(f.pair[g][1]);
        r.key += f64::from(kx);
        r.key_src[ks] += 1;
        let hit = &f.hits[g];
        let kill = run.digs.iter().any(|d| d.0 == fi && in_box(hit, d.1, d.2));
        let held = run.hold
            && run
                .digs
                .iter()
                .zip(held_until)
                .any(|(d, &u)| fi >= d.0 && fi <= u && in_box(hit, d.1, d.2));
        r.kill += u64::from(kill);
        r.held += u64::from(held);
        r.ring_hold +=
            u64::from(run.hold && f.cast[g] && f.source[g] == SOURCE_RECORD_STALE && !held);
        if kind == 2 {
            stage[0] += u64::from({
                let xs: Vec<f32> = f.pair.iter().map(|p| p[0]).collect();
                let gh = gl / gw;
                let (gx, gy) = (g % gw, g / gw);
                let mut m = Vec::new();
                for dy in -1i64..=1 {
                    for dx in -1i64..=1 {
                        let (xx, yy) = (gx as i64 + dx, gy as i64 + dy);
                        if xx >= 0 && yy >= 0 && (xx as usize) < gw && (yy as usize) < gh {
                            m.push(xs[yy as usize * gw + xx as usize]);
                        }
                    }
                }
                m.sort_by(f32::total_cmp);
                (xs[g] - m[m.len() / 2]).abs() > 0.1
            });
            if !f.filtered.is_empty() {
                stage[1] += u64::from(rel_from_median(&f.filtered, gw, g) > 0.25);
            }
            if !f.composite.is_empty() {
                let (mw, mh) = (f.march[0] as usize, f.march[1] as usize);
                let mi = (i / w) * mh / h * mw + (i % w) * mw / w;
                stage[2] += u64::from(rel_from_median(&f.composite, mw, mi) > 0.25);
            }
        }
    }
    (rows, stage)
}

/// [`hole_tables`]' error table, per dig group ([`hole_groups`]), into `out`.
fn error_table(
    run: &HoleRun,
    world: &vxe_world::World,
    classes: &[Vec<u8>],
    groups: &[(usize, usize)],
    out: &mut String,
) {
    use std::fmt::Write;
    let eye = run.eye;
    let (w, h) = (run.out[0] as usize, run.out[1] as usize);
    let gw_of = |gl: usize| ((gl as f64 * w as f64 / h as f64).sqrt().round() as usize).max(1);
    let mut cache = std::collections::HashMap::new();
    for &(start, end) in groups {
        let landed = &run.frames[end];
        let pre = &run.frames[start - 1];
        let gl = landed.hits.len();
        let gw = gw_of(gl);
        let names = [
            "shown today",
            "seed",
            "removed",
            "removed x 0.5",
            "removed x 0.25",
            "removed x open (a1)",
            "open alone",
            "removed x 0.75",
            "removed x open-min",
        ];
        let mut sums = [(0.0f64, 0.0f64, 0.0f64); 9];
        let mut n = 0u64;
        for (i, _) in classes[end]
            .iter()
            .enumerate()
            .filter(|(_, c)| **c == DIG_HOLE_CLASS)
        {
            let g = grid_of(i, w, h, gw, gl);
            let vl = landed.pair[g][0];
            let ll = f32::from(landed.luma[i]);
            if landed.cast[g] || vl < 0.02 || ll < 4.0 {
                continue;
            }
            let hit = &landed.hits[g];
            let open = openness(world, &mut cache, [hit[0], hit[1], hit[2]], eye);
            let open_min = openness_min(world, &mut cache, [hit[0], hit[1], hit[2]], eye);
            let removed = pre.pair[g][0];
            let seed = run.frames[start].pair[g][0];
            let shown = (start..end)
                .map(|fi| (f32::from(run.frames[fi].luma[i]) - ll).abs())
                .fold(0.0f32, f32::max);
            let cand = [
                f32::NAN,
                seed,
                removed,
                0.5 * removed,
                0.25 * removed,
                removed * open,
                open,
                0.75 * removed,
                (removed * open_min).max(0.02),
            ];
            n += 1;
            for k in 0..9 {
                let (dl, dv) = if k == 0 {
                    (shown, f32::NAN)
                } else {
                    let v = cand[k].max(0.0);
                    let lv = (ll * (v / vl).powf(1.0 / 2.2)).min(255.0);
                    ((lv - ll).abs(), (v - vl).abs())
                };
                sums[k].0 += f64::from(dl);
                sums[k].1 = sums[k].1.max(f64::from(dl));
                if dv.is_finite() {
                    sums[k].2 += f64::from(dv);
                }
            }
        }
        let _ = writeln!(
            out,
            "error table, digs from frame {start} landed at frame {end} ({}), {n} hole pixels:",
            run.frames[end].label
        );
        for (k, name) in names.iter().enumerate() {
            let m = n.max(1) as f64;
            let _ = writeln!(
                out,
                "    {name:<20} levels mean {:.1} largest {:.0}; visibility mean {:.3}",
                sums[k].0 / m,
                sums[k].1,
                sums[k].2 / m
            );
        }
    }
}
