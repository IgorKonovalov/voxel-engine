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
