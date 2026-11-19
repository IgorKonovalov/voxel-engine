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
pub const SOURCE_NEIGHBOURS: u8 = 4;
pub const SOURCE_NO_VALUE: u8 = 5;
pub const SOURCE_MISMATCH: u8 = 6;
/// The source words' names, by word.
pub const SOURCE_NAMES: [&str; 7] = [
    "none",
    "record",
    "record stale",
    "own summary",
    "neighbours",
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
