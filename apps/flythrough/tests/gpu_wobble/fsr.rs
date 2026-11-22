//! Plan 0098 Phase 2's spike: where FSR 2 loses the far pixels after a turn. One protocol per
//! arm on the wobble rig's forest: the settle, eight still frames whose render-size hits are
//! compared phase to phase, a 60-degree turn over 40 frames whose last 10 have their motion
//! vectors checked against the hits' exact reprojection, then a hold of 120 frames whose first
//! 31 read the accumulate pass's stage code (`vxe_render::Fsr2Stage`) per display pixel. Each
//! row is printed; nothing is asserted.

use super::*;
use vxe_render::{Fsr2Stage, View, wgpu};

/// The hold's frames whose stage code is read (+0 to +30).
const STAGED: usize = 31;
/// The hold's frames.
const HOLD: usize = 120;
/// The still frames whose hits are compared phase to phase.
const STILL: usize = 8;
/// The turn's frames, and the last ones whose motion is checked.
const TURN: usize = 40;
const MOTION_FROM: usize = 30;

/// The far class of one frame's stage codes: shares in percent of the far display pixels (a
/// new sample, a depth clip over 0.1, a lock released, the history clamped), and the means of
/// the clamp's strength and the history's weight over them.
#[derive(Clone, Copy, Debug, Default)]
struct Stages {
    new: f64,
    clip: f64,
    released: f64,
    locked: f64,
    clamped: f64,
    clamp: f64,
    weight: f64,
}

fn far_stages(codes: &[f32], cls: &[u8]) -> Stages {
    let mut s = Stages::default();
    let mut n = 0usize;
    for (v, c) in codes.iter().zip(cls) {
        if *c < 3 {
            continue;
        }
        let Some(st) = Fsr2Stage::decode(*v) else {
            continue;
        };
        n += 1;
        s.new += f64::from(u8::from(st.new_sample));
        s.clip += f64::from(u8::from(st.depth_clip > 0.1));
        s.released += f64::from(u8::from(st.released));
        s.locked += f64::from(u8::from(st.was_locked));
        s.clamped += f64::from(u8::from(st.clamped));
        s.clamp += f64::from(st.clamp);
        s.weight += f64::from(st.history_weight);
    }
    let k = n.max(1) as f64;
    Stages {
        new: 100.0 * s.new / k,
        clip: 100.0 * s.clip / k,
        released: 100.0 * s.released / k,
        locked: 100.0 * s.locked / k,
        clamped: 100.0 * s.clamped / k,
        clamp: s.clamp / k,
        weight: s.weight / k,
    }
}

fn fmt_stages(s: &Stages) -> String {
    format!(
        "new {:.2} % clip {:.2} % locked {:.2} % released {:.2} % clamped {:.2} % (clamp {:.3}) weight {:.3}",
        s.new, s.clip, s.locked, s.released, s.clamped, s.clamp, s.weight
    )
}

/// The eye-distance in metres of a render-size hit, `None` for a miss.
fn hit_m(h: [f32; 4], eye: [f32; 3]) -> Option<f64> {
    if h[3] > 1e30 || !h[3].is_finite() {
        return None;
    }
    let d = (0..3)
        .map(|i| f64::from(h[i] - eye[i]).powi(2))
        .sum::<f64>()
        .sqrt();
    Some(d * f64::from(VOXEL_M))
}

/// The share of far render pixels (over 15 m in either frame) whose hit moves by more than a
/// voxel, or turns from a hit to a miss or back, between two frames of a still camera.
fn hit_change(a: &[[f32; 4]], b: &[[f32; 4]], eye: [f32; 3]) -> f64 {
    let (mut n, mut moved) = (0usize, 0usize);
    for (p, q) in a.iter().zip(b) {
        let (da, db) = (hit_m(*p, eye), hit_m(*q, eye));
        if !(da.is_some_and(|d| d > 15.0) || db.is_some_and(|d| d > 15.0)) {
            continue;
        }
        n += 1;
        let changed = match (da, db) {
            (Some(_), Some(_)) => (0..3).any(|i| (p[i] - q[i]).abs() > 1.0),
            _ => true,
        };
        moved += usize::from(changed);
    }
    100.0 * moved as f64 / n.max(1) as f64
}

/// The motion vectors' error against the exact reprojection, in display pixels, over the far
/// render pixels: the input pass's motion (UV) against `reproject_centred` of each hit through
/// `prev` in f64, less the pixel's centre. Returns (mean, p99, max).
fn motion_error(
    hits: &[[f32; 4]],
    motion: &[[f32; 2]],
    prev: &View,
    jitter: [f32; 2],
    size: [u32; 2],
    eye: [f32; 3],
) -> (f64, f64, f64) {
    let v = |a: [f32; 3]| a.map(f64::from);
    let (o, r, u, fw) = (v(prev.origin), v(prev.right), v(prev.up), v(prev.fwd));
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let (tl, tr, tu, td) = (
        f64::from(prev.tan_left),
        f64::from(prev.tan_right),
        f64::from(prev.tan_up),
        f64::from(prev.tan_down),
    );
    let (w, h) = (f64::from(size[0]), f64::from(size[1]));
    let to_display = f64::from(OUT[0]);
    let mut errs = Vec::new();
    for (i, (p, m)) in hits.iter().zip(motion).enumerate() {
        if !hit_m(*p, eye).is_some_and(|d| d > 15.0) {
            continue;
        }
        let d = [0, 1, 2].map(|k| f64::from(p[k]) - o[k]);
        let z = dot(d, fw);
        if z <= 1e-4 {
            continue;
        }
        let x = dot(d, r) / z;
        let y = dot(d, u) / z;
        let qx = (x + tl) / (tl + tr) * w - f64::from(jitter[0]);
        let qy = (tu - y) / (tu + td) * h - f64::from(jitter[1]);
        let (px, py) = ((i % size[0] as usize) as f64, (i / size[0] as usize) as f64);
        let ex = (qx - (px + 0.5)) / w;
        let ey = (qy - (py + 0.5)) / h;
        let dx = (f64::from(m[0]) - ex) * to_display;
        let dy = (f64::from(m[1]) - ey) * f64::from(OUT[1]);
        errs.push((dx * dx + dy * dy).sqrt());
    }
    if errs.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    errs.sort_by(f64::total_cmp);
    let mean = errs.iter().sum::<f64>() / errs.len() as f64;
    let p99 = errs[(errs.len() - 1) * 99 / 100];
    (mean, p99, *errs.last().unwrap())
}

/// Reads an rg32float texture as pairs.
fn read_rg32f(rig: &Rig, t: &wgpu::Texture) -> Vec<[f32; 2]> {
    support::rigs::read_f32_texels(&rig.device, &rig.queue, t)
        .chunks(2)
        .map(|c| [c[0], c[1]])
        .collect()
}

/// Reads an r32float texture.
fn read_r32f(rig: &Rig, t: &wgpu::Texture) -> Vec<f32> {
    support::rigs::read_f32_texels(&rig.device, &rig.queue, t)
}

/// The protocol for `arm` from the spawn's camera `cam0` (turned by the arm's yaw and pitch):
/// the rows printed, prefixed `fsr <arm>`.
pub(super) fn fsr_split(rig: &mut Rig, cam0: Camera, arm: &Arm) {
    QUEUE_ON.with(|q| q.set(true));
    let cam0 = Camera {
        pitch: arm.pitch.to_radians(),
        yaw: cam0.yaw + arm.yaw.to_radians(),
        ..cam0
    };
    let mut f = frame_for(rig, arm);
    let fsr = arm.upscaler == "fsr2";
    if let Some(x) = f.fsr2_mut() {
        x.set_debug(&rig.device, true);
        x.set_debug_stages(true);
    }
    let size = f.internal_size();
    let mut s = Stepper::new(rig, f);
    let name = &arm.name;
    println!("fsr {name} on {}: internal {size:?}", rig.adapter);
    for _ in 0..SETTLE {
        queue_step(rig, &mut s.frame, &cam0);
        s.run(rig, &cam0);
    }
    // Still: the far hits phase to phase.
    let eye0 = cam0.position.to_array();
    let mut last: Option<Vec<[f32; 4]>> = None;
    let mut changes = Vec::new();
    for _ in 0..STILL {
        queue_step(rig, &mut s.frame, &cam0);
        s.run(rig, &cam0);
        let hits =
            vxe_render::debug::read_rgba32f(&rig.device, &rig.queue, s.frame.hit_positions());
        if let Some(a) = &last {
            changes.push(hit_change(a, &hits, eye0));
        }
        last = Some(hits);
    }
    let mean_change = changes.iter().sum::<f64>() / changes.len().max(1) as f64;
    println!(
        "fsr {name}: still, far hits changed phase to phase {mean_change:.2} % (pairs {})",
        changes
            .iter()
            .map(|c| format!("{c:.2}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    // The turn; the motion checked over its last frames.
    let mut prev = cam0;
    let mut motion = Vec::new();
    for k in 1..=TURN {
        let c = yawed(&cam0, 60.0 * k as f32 / TURN as f32);
        queue_step(rig, &mut s.frame, &c);
        s.run(rig, &c);
        if fsr && k > MOTION_FROM {
            let x = s.frame.fsr2().unwrap();
            let hits =
                vxe_render::debug::read_rgba32f(&rig.device, &rig.queue, s.frame.hit_positions());
            let mv = read_rg32f(rig, x.motion());
            let jitter = s
                .frame
                .params(&rig.gpu, &c, &vxe_render::LightParams::headlamp())
                .views[0]
                .jitter;
            let rect = [0, 0, size[0], size[1]];
            motion.push(motion_error(
                &hits,
                &mv,
                &View::of_camera(&prev, rect),
                jitter,
                size,
                c.position.to_array(),
            ));
        }
        prev = c;
    }
    let cam2 = prev;
    if !motion.is_empty() {
        let n = motion.len() as f64;
        let mean = motion.iter().map(|m| m.0).sum::<f64>() / n;
        let p99 = motion.iter().map(|m| m.1).fold(0.0, f64::max);
        let max = motion.iter().map(|m| m.2).fold(0.0, f64::max);
        println!(
            "fsr {name}: turn frames {}..{TURN}, far motion error in display px: mean {mean:.2e}, p99 (worst frame) {p99:.2e}, max {max:.2e}",
            MOTION_FROM + 1
        );
    }
    // The hold: stage codes for +0..+30, the display for the wobble against the hold's end.
    let mut stages = Vec::new();
    let mut frames = Vec::new();
    let mut ms = Vec::new();
    let mut cls = Vec::new();
    for k in 0..HOLD {
        queue_step(rig, &mut s.frame, &cam2);
        let shot = s.step(rig, &cam2);
        ms.push(shot.passes.iter().sum::<f64>());
        let gw = s.frame.light_hit_positions().width() as usize;
        cls = classes(&shot.hits, &shot.source, gw, cam2.position.to_array());
        if fsr && k < STAGED {
            let codes = read_r32f(rig, s.frame.fsr2().unwrap().history_weight());
            stages.push(far_stages(&codes, &cls));
        }
        if [0, 10, 30].contains(&k) || k == HOLD - 1 {
            frames.push(lumas(&shot.display));
        }
    }
    let end = frames.last().unwrap();
    let wob: Vec<String> = [0, 10, 30]
        .iter()
        .zip(&frames)
        .map(|(k, y)| format!("+{k} {:.2} %", change_by_class(y, end, &cls, 4)[3].0))
        .collect();
    let gpu = ms.iter().sum::<f64>() / ms.len() as f64;
    println!(
        "fsr {name}: far over 4 levels against the hold's end: {}; GPU frame {gpu:.2} ms (passes summed, hold mean)",
        wob.join(", ")
    );
    for (k, st) in stages.iter().enumerate() {
        if k <= 10 || k % 5 == 0 {
            println!("fsr {name}: +{k} {}", fmt_stages(st));
        }
    }
}

mod reading {
    use super::*;

    /// Plan 0098 Phase 2's table: [`fsr_split`] over `VXE_WOBBLE_FSR` (arms separated by `;`,
    /// in [`Arm::from_spec`]'s tokens), by default the `calm` arm, its levers (the scale 1.3 and
    /// 1, 8 and 32 jitter phases, the far normal 2..6) and `albedo` at 1.5 and 1, on the
    /// wobble rig's forest. `VXE_TEST_ADAPTER` picks the adapter.
    #[test]
    #[ignore = "reading"]
    fn the_fsr2_stages_after_the_turn() {
        let specs = std::env::var("VXE_WOBBLE_FSR").unwrap_or_else(|_| {
            "calm;calm,scale=1.3;calm,scale=1;calm,phases=8;calm,phases=32;calm,farn=2/6;\
             albedo;albedo,scale=1"
                .into()
        });
        let Some((mut rig, cam0)) = forest_rig(8) else {
            return;
        };
        for spec in specs.split(';') {
            fsr_split(&mut rig, cam0, &Arm::from_spec(spec));
        }
    }
}
