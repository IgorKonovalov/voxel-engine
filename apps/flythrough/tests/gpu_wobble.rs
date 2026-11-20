//! The motion wobble's reading rig (ADR-0079; from the `spike-motion-wobble` branch): the forest
//! of seed 1, a still strip, a slow fly, a stop, a turn and a hold, each read as the display's
//! frame-to-frame change by hit distance, and the far pixels whose hit's record is fresh (the
//! "far-baked" class) beside all far ones. Nothing is asserted: a reading in `mod reading`.
//! `VXE_WOBBLE` names the arm (see `Arm::parse`), `VXE_TEST_ADAPTER` the adapter. Frames and
//! maps go under `live/wobble/<arm>/`.
#![allow(
    dead_code,
    clippy::too_many_lines,
    clippy::disallowed_methods,
    reason = "a reading rig: its arms leave fields unread, its runs time themselves"
)]

mod support;

use std::path::{Path, PathBuf};

use support::defects::{Stepper, grid_of};

std::thread_local! {
    static QUEUE_ON: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

fn queue_step(rig: &mut Rig, f: &mut Frame, cam: &Camera) {
    if QUEUE_ON.with(std::cell::Cell::get) {
        support::defects::queue_step(rig, f, cam);
    }
}
use support::terms::Rig;
use vxe_render::{Camera, Frame};

/// The frame's sizes: FSR 2 and our upscaler at 1.5, `off` at 1.
const INTERNAL: [u32; 2] = [1280, 720];
const OUT: [u32; 2] = [1920, 1080];

/// Metres a voxel.
const VOXEL_M: f32 = 0.1;

#[derive(Clone, Debug)]
struct Arm {
    name: String,
    upscaler: &'static str,
    sharpen: f32,
    albedo: bool,
    lighting_full: bool,
    filter: bool,
    far_normal: [f32; 2],
    pin_exposure: bool,
    any_history: bool,
    key_off: bool,
    sky_rays: u32,
    leaves_brick: bool,
    sun: [f32; 2],
    speed_m_s: f32,
    turn_deg: f32,
    drain: usize,
    store_read_face: bool,
    jitter_phases: Option<u32>,
    dither: bool,
    no_stair: bool,
    weights_normal: bool,
    key_no_jitter: bool,
    yaw: f32,
    pitch: f32,
    up: f32,
    probe: bool,
    nqueue: bool,
    hold: usize,
    key_history: u32,
    sky_history: u32,
    key_far: vxe_render::KeyFar,
    key_far_mix: vxe_render::KeyFarMix,
    /// The queue's budget a frame through the drain, in ms (`pre=MS`); `None` the default.
    pre_ms: Option<f64>,
}

impl Arm {
    /// `VXE_WOBBLE`: comma-separated tokens over the defaults (`default` alone is the
    /// fly-through's frame): `off1` (no upscaler, scale 1), `own` (ours at 1.5), `s0`
    /// (sharpen 0), `albedo`, `full`, `nofilter`, `farn0`, `pin`, `anyhist`, `keyoff`,
    /// `sky0`, `brick`, `sun=EL/AZ`, `speed=M_S`, `turn=DEG`, `drain=N`, `face` (store read
    /// face), `phases=N`, `dither`, `nostair`, `wnormal`, `ray` (`key_far ray`), `step`
    /// (`key_far_mix step`), `pre=MS` (the queue's budget through the drain, the default's
    /// after it).
    fn parse() -> Self {
        let spec = std::env::var("VXE_WOBBLE").unwrap_or_else(|_| "default".into());
        let mut a = Arm {
            name: spec.replace(['/', '='], "-"),
            upscaler: "fsr2",
            sharpen: 0.5,
            albedo: false,
            lighting_full: false,
            filter: true,
            far_normal: flythrough::LookPreset::DEFAULT.far_normal,
            pin_exposure: false,
            any_history: false,
            key_off: false,
            sky_rays: flythrough::default_lighting().sky_rays,
            leaves_brick: false,
            sun: [40.0, 200.0],
            speed_m_s: 1.0,
            turn_deg: 60.0,
            drain: 2000,
            store_read_face: false,
            jitter_phases: None,
            dither: false,
            no_stair: false,
            weights_normal: false,
            key_no_jitter: false,
            yaw: 90.0,
            pitch: -4.0,
            up: 0.0,
            probe: false,
            nqueue: false,
            hold: 300,
            key_history: flythrough::default_lighting().key_history,
            sky_history: flythrough::default_lighting().sky_history,
            key_far: flythrough::default_lighting().key_far,
            key_far_mix: flythrough::default_lighting().key_far_mix,
            pre_ms: None,
        };
        for t in spec.split(',') {
            let (k, v) = t.split_once('=').unwrap_or((t, ""));
            match k {
                "default" => {}
                "tag" => {}
                "off1" => a.upscaler = "off",
                "own" => a.upscaler = "own",
                "s0" => a.sharpen = 0.0,
                "albedo" => a.albedo = true,
                "full" => a.lighting_full = true,
                "nofilter" => a.filter = false,
                "farn0" => a.far_normal = [0.0; 2],
                "pin" => a.pin_exposure = true,
                "anyhist" => a.any_history = true,
                "keyoff" => a.key_off = true,
                "sky0" => a.sky_rays = 0,
                "brick" => a.leaves_brick = true,
                "face" => a.store_read_face = true,
                "dither" => a.dither = true,
                "nostair" => a.no_stair = true,
                "knj" => a.key_no_jitter = true,
                "wnormal" => a.weights_normal = true,
                "sun" => {
                    let (e, z) = v.split_once('/').expect("sun=EL/AZ");
                    a.sun = [e.parse().unwrap(), z.parse().unwrap()];
                }
                "speed" => a.speed_m_s = v.parse().unwrap(),
                "turn" => a.turn_deg = v.parse().unwrap(),
                "drain" => a.drain = v.parse().unwrap(),
                "phases" => a.jitter_phases = Some(v.parse().unwrap()),
                "yaw" => a.yaw = v.parse().unwrap(),
                "pitch" => a.pitch = v.parse().unwrap(),
                "up" => a.up = v.parse().unwrap(),
                "probe" => a.probe = true,
                "nqueue" => a.nqueue = true,
                "hold" => a.hold = v.parse().unwrap(),
                "kh" => a.key_history = v.parse().unwrap(),
                "sh" => a.sky_history = v.parse().unwrap(),
                "ray" => a.key_far = vxe_render::KeyFar::Ray,
                "step" => a.key_far_mix = vxe_render::KeyFarMix::Step,
                "pre" => a.pre_ms = Some(v.parse().unwrap()),
                other => panic!("unknown arm token {other}"),
            }
        }
        a
    }
}

/// The forest of seed 1 at `LEAF_FILL`, the chunks within `r` of the spawn's in x and y (2
/// down, 3 up), the spawn's camera.
fn forest_rig(r: i32) -> Option<(Rig, Camera)> {
    use flythrough::outdoor::{OutdoorSource, Spawn};
    use flythrough::trees::{ForestSource, LEAF_FILL, TreeDensity};
    use vxe_world::{ChunkExtent, World};
    let source = ForestSource::new(OutdoorSource::new(1), TreeDensity::Forest, LEAF_FILL);
    let (eye, facing) = source.base().spawn(Spawn::Forest);
    let c = eye.chunk().0;
    let extent = ChunkExtent::new(
        [c[0] - r, c[1] - r, c[2] - 2],
        [c[0] + r + 1, c[1] + r + 1, c[2] + 3],
    );
    let t = std::time::Instant::now();
    let world = World::generate(extent, &source, &flythrough::outdoor_materials());
    println!(
        "forest r {r}: generated in {:.1} s",
        t.elapsed().as_secs_f64()
    );
    let rig = support::terms::mirrored(world, eye)?;
    let p = eye.0.map(|v| v as f32 + 0.5);
    let d = facing.map(|v| v as f32);
    let cam = flythrough::camera_looking_at(p, [p[0] + d[0], p[1] + d[1], p[2] + d[2]]);
    Some((rig, cam))
}

fn frame_for(rig: &Rig, arm: &Arm) -> Frame {
    let internal = if arm.upscaler == "off" { OUT } else { INTERNAL };
    let mut f = Frame::with_sizes(&rig.device, internal, OUT);
    match arm.upscaler {
        "fsr2" => f.set_upscaler_kind(
            &rig.device,
            vxe_render::UpscalerKind::Fsr2(vxe_render::Fsr2Settings {
                sharpness: arm.sharpen,
                jitter_phases: arm.jitter_phases,
                dither: arm.dither,
                ..Default::default()
            }),
        ),
        "own" => {
            f.set_upscaler_kind(
                &rig.device,
                vxe_render::UpscalerKind::Own(Default::default()),
            );
            f.set_jitter(&rig.device, true);
            f.set_sharpness(&rig.queue, arm.sharpen);
        }
        _ => {
            f.set_upscaler_kind(&rig.device, vxe_render::UpscalerKind::Off);
            f.set_jitter(&rig.device, false);
        }
    }
    f.set_count_store(true);
    f.set_look(vxe_render::LookParams {
        far_normal: arm.far_normal,
        view: if arm.albedo {
            vxe_render::ViewMode::Albedo
        } else {
            vxe_render::ViewMode::Shaded
        },
        ..vxe_render::LookParams::default()
    });
    let leaf = if arm.leaves_brick {
        flythrough::LeafShadow::Brick
    } else {
        flythrough::LeafShadow::Voxel
    };
    f.set_material_styles(&rig.queue, &flythrough::outdoor_styles(leaf));
    f.set_lighting(vxe_render::Lighting {
        lamp: false,
        filter: arm.filter,
        sky_rays: arm.sky_rays,
        key_history: arm.key_history,
        sky_history: arm.sky_history,
        key_far: arm.key_far,
        key_far_mix: arm.key_far_mix,
        store_read: if arm.store_read_face {
            vxe_render::StoreRead::Face
        } else {
            flythrough::default_lighting().store_read
        },
        light_weights: if arm.weights_normal {
            vxe_render::LightWeights::Normal
        } else {
            flythrough::default_lighting().light_weights
        },
        exposure: vxe_render::Exposure::Auto(vxe_render::AutoExposure {
            min: flythrough::cli::OUTDOOR_MIN,
            max: flythrough::cli::OUTDOOR_MAX,
            ..vxe_render::AutoExposure::default()
        }),
        ..flythrough::default_lighting()
    });
    if arm.lighting_full {
        f.set_resolution(&rig.device, vxe_render::LightingResolution::Full);
    }
    f.set_light_hooks(vxe_render::LightHooks {
        any_history: arm.any_history,
        key_off: arm.key_off,
        no_stair: arm.no_stair,
        key_no_jitter: arm.key_no_jitter,
        ..Default::default()
    });
    let sky = flythrough::daylight::Clock {
        fixed_sun: Some(arm.sun),
        ..Default::default()
    }
    .sky(flythrough::outdoor::BASE_Z as f32);
    f.set_sky(Some(sky));
    f.set_dt(1.0 / 60.0);
    f.start_bake_queue(
        &rig.queue,
        &rig.gpu,
        vxe_render::queue_settings(),
        vxe_render::DEFAULT_BAKE_SLICE_MS,
    );
    f.set_bake_coarse(8, 2);
    f
}

/// Display luma, 0..255.
fn lumas(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks(4)
        .map(|p| {
            (0.2126 * f32::from(p[0]) + 0.7152 * f32::from(p[1]) + 0.0722 * f32::from(p[2])).round()
                as u8
        })
        .collect()
}

/// Per display pixel: 0 sky, 1 near (under 6 m), 2 mid, 3 far (over 15 m) whose hit's record is
/// not fresh, 4 far whose record is (its lighting pixel's sky source word `record`: the key words
/// bake in the same job and share the stale bit, so a fresh sky record is a fresh key record).
fn classes(hits: &[[f32; 4]], source: &[u8], gw: usize, eye: [f32; 3]) -> Vec<u8> {
    let (dw, dh) = (OUT[0] as usize, OUT[1] as usize);
    let gl = hits.len();
    (0..dw * dh)
        .map(|i| {
            let g = grid_of(i, dw, dh, gw, gl);
            let h = hits[g];
            if h[3] > 1e30 || !h[3].is_finite() {
                return 0;
            }
            let d = ((h[0] - eye[0]).powi(2) + (h[1] - eye[1]).powi(2) + (h[2] - eye[2]).powi(2))
                .sqrt()
                * VOXEL_M;
            if d < 6.0 {
                1
            } else if d < 15.0 {
                2
            } else if source[g] == support::defects::SOURCE_RECORD {
                4
            } else {
                3
            }
        })
        .collect()
}

/// The reported classes: far is every far pixel (3 and 4 of [`classes`]), far-baked the far
/// pixels whose hit's record is fresh (4).
const CLASS_NAMES: [&str; 5] = ["sky", "near", "mid", "far", "far-baked"];

/// Per reported class: the share of pixels whose luma changed by over `t` levels, and the mean
/// |dY|.
fn change_by_class(a: &[u8], b: &[u8], cls: &[u8], t: u8) -> [(f64, f64); 5] {
    let mut n = [0usize; 5];
    let mut over = [0usize; 5];
    let mut sum = [0f64; 5];
    for i in 0..a.len() {
        let d = a[i].abs_diff(b[i]);
        let c = cls[i] as usize;
        let bins: &[usize] = if c == 4 {
            &[3, 4]
        } else {
            std::slice::from_ref(&c)
        };
        for &k in bins {
            n[k] += 1;
            sum[k] += f64::from(d);
            if d > t {
                over[k] += 1;
            }
        }
    }
    let mut out = [(0.0, 0.0); 5];
    for c in 0..5 {
        let k = n[c].max(1) as f64;
        out[c] = (100.0 * over[c] as f64 / k, sum[c] / k);
    }
    out
}

fn fmt_classes(v: &[(f64, f64); 5]) -> String {
    (1..5)
        .map(|c| format!("{} {:.2} % / {:.3}", CLASS_NAMES[c], v[c].0, v[c].1))
        .collect::<Vec<_>>()
        .join(", ")
}

fn write_map(dir: &Path, name: &str, map: &[f64], scale: f64) {
    let px: Vec<u8> = map
        .iter()
        .flat_map(|v| {
            let g = (v * scale).clamp(0.0, 255.0) as u8;
            [g, g, g, 255]
        })
        .collect();
    support::write_png(&dir.join(name), &px, OUT[0], OUT[1]);
}

fn moved(cam: &Camera, metres: f32) -> Camera {
    let f = cam.basis().0;
    let h = (f.x * f.x + f.y * f.y).sqrt().max(1e-6);
    let mut c = *cam;
    c.position.x += f.x / h * metres / VOXEL_M;
    c.position.y += f.y / h * metres / VOXEL_M;
    c
}

fn yawed(cam: &Camera, deg: f32) -> Camera {
    let mut c = *cam;
    c.yaw += deg.to_radians();
    c
}

const DIAG_AT: [usize; 6] = [0, 8, 30, 60, 120, 240];

struct Segment {
    name: &'static str,
    /// Per pair: the change by class over 4 levels and the mean |dY|.
    pairs: Vec<[(f64, f64); 5]>,
    /// The per-pixel mean |dY| over the segment.
    map: Vec<f64>,
    /// The last frame's luma.
    last: Vec<u8>,
    frames: Vec<Vec<u8>>,
    /// At the sampled frames: (k, age per lighting pixel, source word, cast, kind).
    diag: Vec<Diag>,
    gw: usize,
}

/// A sampled frame's diagnostic: its index, and per lighting pixel the age, the source word,
/// whether the sky ray was cast and the moments' kind.
type Diag = (usize, Vec<f32>, Vec<u8>, Vec<bool>, Vec<u8>);

/// Runs `n` frames along `cams`, reading every frame; `keep` keeps the lumas.
#[allow(
    clippy::too_many_arguments,
    reason = "a reading's segment: the rig, the stepper, the path and its outputs"
)]
fn run_segment(
    rig: &mut Rig,
    s: &mut Stepper,
    cams: impl Iterator<Item = Camera>,
    name: &'static str,
    keep: bool,
    dir: &Path,
    every: usize,
    prev: Option<Vec<u8>>,
) -> Segment {
    let n = OUT[0] as usize * OUT[1] as usize;
    let mut map = vec![0f64; n];
    let mut pairs = Vec::new();
    let mut last = prev;
    let mut frames = Vec::new();
    let mut diag = Vec::new();
    let mut gw = 1usize;
    let mut k = 0usize;
    let mut sum_pass = Vec::<f64>::new();
    for cam in cams {
        queue_step(rig, &mut s.frame, &cam);
        let shot = s.step(rig, &cam);
        if sum_pass.len() < shot.passes.len() {
            sum_pass.resize(shot.passes.len(), 0.0);
        }
        for (a, b) in sum_pass.iter_mut().zip(&shot.passes) {
            *a += b;
        }
        gw = s.frame.light_hit_positions().width() as usize;
        let cls = classes(&shot.hits, &shot.source, gw, cam.position.to_array());
        let y = lumas(&shot.display);
        if keep && DIAG_AT.contains(&k) {
            let mom = vxe_render::debug::read_rgba16f(&rig.device, &rig.queue, s.frame.moments());
            let kind: Vec<u8> = mom.iter().map(|p| p[3].floor() as u8).collect();
            diag.push((
                k,
                shot.age.clone(),
                shot.source.clone(),
                shot.cast.clone(),
                kind,
            ));
        }
        if let Some(p) = &last {
            pairs.push(change_by_class(p, &y, &cls, 4));
            for i in 0..n {
                map[i] += f64::from(p[i].abs_diff(y[i]));
            }
        }
        if (every > 0 && k.is_multiple_of(every))
            || (every == 0 && [0usize, 8, 30, 60, 120].contains(&k))
        {
            support::write_png(
                &dir.join(format!("{name}-{k:03}.png")),
                &shot.display,
                OUT[0],
                OUT[1],
            );
        }
        if keep {
            frames.push(y.clone());
        }
        last = Some(y);
        k += 1;
    }
    let pairs_n = pairs.len().max(1) as f64;
    for v in &mut map {
        *v /= pairs_n;
    }
    write_map(dir, &format!("{name}-diff.png"), &map, 8.0);
    let names = vxe_render::FRAME_PASSES;
    let ms: Vec<String> = names
        .iter()
        .zip(&sum_pass)
        .filter(|(_, v)| **v / k as f64 > 0.05)
        .map(|(n, v)| format!("{n} {:.2}", v / k as f64))
        .collect();
    println!(
        "  {name}: {k} frames; pass ms: {}; at its end {}",
        ms.join(", "),
        depth(&s.frame)
    );
    Segment {
        name,
        pairs,
        map,
        last: last.unwrap(),
        frames,
        diag,
        gw,
    }
}

/// The live queue's depth in words: its jobs and the chunks it waits on.
fn depth(f: &Frame) -> String {
    f.bake_queue().map_or("no queue".into(), |q| {
        let st = q.stats();
        format!("queue depth {} ({} jobs)", st.depth, st.jobs)
    })
}

fn summarise(seg: &Segment, cls_note: &str) {
    let m = seg.pairs.len();
    if m == 0 {
        return;
    }
    let mut mean = [(0.0, 0.0); 5];
    let mut max = [(0.0f64, 0.0f64); 5];
    for p in &seg.pairs {
        for c in 0..5 {
            mean[c].0 += p[c].0 / m as f64;
            mean[c].1 += p[c].1 / m as f64;
            max[c].0 = max[c].0.max(p[c].0);
            max[c].1 = max[c].1.max(p[c].1);
        }
    }
    println!(
        "  {} ({cls_note}): over 4 % / mean |dY| per pair, mean: {}",
        seg.name,
        fmt_classes(&mean)
    );
    println!("  {} max: {}", seg.name, fmt_classes(&max));
}

/// The series of a hold: per frame, the far class's over-4 share and mean |dY| against the
/// previous frame, and against the hold's last frame.
fn hold_series(seg: &Segment, cls: &[u8], still_far_max: f64) {
    let last = seg.frames.last().unwrap();
    let mut back_pair = None;
    let mut back_final = None;
    let mut rows = Vec::new();
    for (i, f) in seg.frames.iter().enumerate() {
        let to_final = change_by_class(f, last, cls, 4);
        let pair = if i > 0 {
            Some(change_by_class(&seg.frames[i - 1], f, cls, 4))
        } else {
            None
        };
        if let Some(p) = pair
            && back_pair.is_none()
            && p[3].0 <= still_far_max
        {
            back_pair = Some(i);
        }
        if back_final.is_none() && to_final[3].0 <= still_far_max && i < seg.frames.len() - 1 {
            back_final = Some(i);
        }
        if i < 16 || (i < 64 && i % 8 == 0) || i % 30 == 0 {
            rows.push(format!(
                "{i}: pair far {:.2} %/{:.3} near {:.2} %; to-final far {:.2} %/{:.3} far-baked {:.2} %/{:.3} near {:.2} %",
                pair.map_or(0.0, |p| p[3].0),
                pair.map_or(0.0, |p| p[3].1),
                pair.map_or(0.0, |p| p[1].0),
                to_final[3].0,
                to_final[3].1,
                to_final[4].0,
                to_final[4].1,
                to_final[1].0
            ));
        }
    }
    println!(
        "  {}: far pair share back under the still max {still_far_max:.2} % after {back_pair:?} frames; within it of the final frame after {back_final:?} frames",
        seg.name
    );
    for r in rows {
        println!("    {r}");
    }
    // The far pixels over 4 from the final frame at the sampled frames: their lighting pixels'
    // age, source word, cast share and kind.
    let (dw, dh) = (OUT[0] as usize, OUT[1] as usize);
    for (k, age, source, cast, kind) in &seg.diag {
        let f = &seg.frames[*k];
        let gl = age.len();
        let mut n = 0usize;
        let mut ages = [0usize; 5];
        let mut srcs = [0usize; 7];
        let mut cast_n = 0usize;
        let mut kinds = [0usize; 3];
        let mut all_far = 0usize;
        let mut all_ages = [0usize; 5];
        for i in 0..f.len() {
            if cls[i] < 3 {
                continue;
            }
            let g = grid_of(i, dw, dh, seg.gw, gl);
            let a = age[g];
            let b = if a < 4.0 {
                0
            } else if a < 16.0 {
                1
            } else if a < 64.0 {
                2
            } else if a < 256.0 {
                3
            } else {
                4
            };
            all_far += 1;
            all_ages[b] += 1;
            if f[i].abs_diff(last[i]) <= 4 {
                continue;
            }
            n += 1;
            ages[b] += 1;
            srcs[(source[g] as usize).min(6)] += 1;
            cast_n += usize::from(cast[g]);
            kinds[(kind[g] as usize).min(2)] += 1;
        }
        let pct = |v: usize, d: usize| 100.0 * v as f64 / d.max(1) as f64;
        println!(
            "    diag {} +{k}: far px over 4 from final {n} of {all_far}; their age <4 {:.1} % <16 {:.1} % <64 {:.1} % <256 {:.1} % >=256 {:.1} % (all far: {:.1} / {:.1} / {:.1} / {:.1} / {:.1}); sources {}; cast {:.1} %; kind 0/1/2 {:.1} / {:.1} / {:.1} %",
            seg.name,
            pct(ages[0], n),
            pct(ages[1], n),
            pct(ages[2], n),
            pct(ages[3], n),
            pct(ages[4], n),
            pct(all_ages[0], all_far),
            pct(all_ages[1], all_far),
            pct(all_ages[2], all_far),
            pct(all_ages[3], all_far),
            pct(all_ages[4], all_far),
            support::defects::SOURCE_NAMES
                .iter()
                .zip(srcs)
                .map(|(nm, v)| format!("{nm} {:.1} %", pct(v, n)))
                .collect::<Vec<_>>()
                .join(", "),
            pct(cast_n, n),
            pct(kinds[0], n),
            pct(kinds[1], n),
            pct(kinds[2], n)
        );
    }
}

mod reading {
    use super::*;

    #[test]
    #[ignore = "reading"]
    fn wobble() {
        let arm = Arm::parse();
        let Some((mut rig, cam0)) = forest_rig(8) else {
            return;
        };
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../live/wobble")
            .join(&arm.name);
        std::fs::create_dir_all(&dir).unwrap();
        println!("arm {} on {}: {arm:?}", arm.name, rig.adapter);
        let mut cam0 = Camera {
            pitch: arm.pitch.to_radians(),
            yaw: cam0.yaw + arm.yaw.to_radians(),
            ..cam0
        };
        cam0.position.z += arm.up / VOXEL_M;
        let f = frame_for(&rig, &arm);
        let mut s = Stepper::new(&rig, f);
        if arm.probe {
            for k in 0..8 {
                let c = yawed(&cam0, 45.0 * k as f32);
                for _ in 0..30 {
                    queue_step(&mut rig, &mut s.frame, &c);
                    s.run(&rig, &c);
                }
                let shot = s.step(&rig, &c);
                let gw = s.frame.light_hit_positions().width() as usize;
                let cls = classes(&shot.hits, &shot.source, gw, c.position.to_array());
                let n = cls.len() as f64;
                let share: Vec<String> = (0..5)
                    .map(|k| {
                        format!(
                            "{} {:.1} %",
                            ["sky", "near", "mid", "far (not baked)", "far-baked"][k],
                            100.0 * cls.iter().filter(|c| **c == k as u8).count() as f64 / n
                        )
                    })
                    .collect();
                println!("  probe yaw +{}: {}", 45 * k, share.join(", "));
                support::write_png(
                    &dir.join(format!("probe-{:03}.png", 45 * k)),
                    &shot.display,
                    OUT[0],
                    OUT[1],
                );
            }
            return;
        }
        if let Some(ms) = arm.pre_ms {
            s.frame.set_bake_budget(vxe_render::Budget::PerFrame { ms });
        }
        let t = std::time::Instant::now();
        let mut n = 0;
        while n < arm.drain && !s.frame.bake_queue().is_some_and(|q| q.empty()) {
            queue_step(&mut rig, &mut s.frame, &cam0);
            s.run(&rig, &cam0);
            n += 1;
        }
        let drained = s.frame.bake_queue().is_some_and(|q| q.empty());
        s.frame.set_bake_budget(vxe_render::Budget::default());
        println!(
            "  queue drained {drained} after {n} frames ({:.1} s)",
            t.elapsed().as_secs_f64()
        );
        for _ in 0..240 {
            queue_step(&mut rig, &mut s.frame, &cam0);
            s.run(&rig, &cam0);
        }
        if arm.nqueue {
            QUEUE_ON.with(|q| q.set(false));
        }
        if arm.pin_exposure {
            let e = s
                .frame
                .exposure_readout(&rig.device)
                .map(|r| r.exposure)
                .expect("a read-back");
            s.frame.set_lighting(vxe_render::Lighting {
                exposure: vxe_render::Exposure::Manual(e),
                ..s.frame.lighting()
            });
            for _ in 0..32 {
                queue_step(&mut rig, &mut s.frame, &cam0);
                s.run(&rig, &cam0);
            }
        }
        // The still strip.
        let still = run_segment(
            &mut rig,
            &mut s,
            std::iter::repeat_n(cam0, 33),
            "still",
            false,
            &dir,
            32,
            None,
        );
        summarise(&still, "classes per frame");
        {
            let shot = support::defects::read(&rig, &s.frame, None);
            let gw = s.frame.light_hit_positions().width() as usize;
            let cls = classes(&shot.hits, &shot.source, gw, cam0.position.to_array());
            let n = cls.len() as f64;
            let share: Vec<String> = (0..5)
                .map(|k| {
                    format!(
                        "{} {:.1} %",
                        ["sky", "near", "mid", "far (not baked)", "far-baked"][k],
                        100.0 * cls.iter().filter(|c| **c == k as u8).count() as f64 / n
                    )
                })
                .collect();
            println!("  classes at the still: {}", share.join(", "));
        }
        let still_far_max = still.pairs.iter().map(|p| p[3].0).fold(0.0, f64::max);
        // The slow fly: 180 frames forward.
        let step = arm.speed_m_s / 60.0;
        let fly_cams: Vec<Camera> = (1..=180).map(|k| moved(&cam0, step * k as f32)).collect();
        let cam1 = *fly_cams.last().unwrap();
        let fly = run_segment(
            &mut rig,
            &mut s,
            fly_cams.into_iter(),
            "fly",
            false,
            &dir,
            30,
            Some(still.last.clone()),
        );
        summarise(&fly, "classes per frame, the camera moving");
        // The stop: 180 frames held at the fly's end.
        let stop = run_segment(
            &mut rig,
            &mut s,
            std::iter::repeat_n(cam1, arm.hold),
            "stop",
            true,
            &dir,
            0,
            Some(fly.last.clone()),
        );
        summarise(&stop, "classes per frame");
        {
            let shot = support::defects::read(&rig, &s.frame, None);
            let gw = s.frame.light_hit_positions().width() as usize;
            let cls = classes(&shot.hits, &shot.source, gw, cam1.position.to_array());
            hold_series(&stop, &cls, still_far_max);
            support::write_png(&dir.join("stop-final.png"), &shot.display, OUT[0], OUT[1]);
        }
        // The turn: `turn_deg` over 40 frames, then 180 held.
        let turn_cams: Vec<Camera> = (1..=40)
            .map(|k| yawed(&cam1, arm.turn_deg * k as f32 / 40.0))
            .collect();
        let cam2 = *turn_cams.last().unwrap();
        let turn = run_segment(
            &mut rig,
            &mut s,
            turn_cams.into_iter(),
            "turn",
            false,
            &dir,
            10,
            Some(stop.last.clone()),
        );
        summarise(&turn, "classes per frame, the camera turning");
        let hold = run_segment(
            &mut rig,
            &mut s,
            std::iter::repeat_n(cam2, arm.hold),
            "hold",
            true,
            &dir,
            0,
            Some(turn.last.clone()),
        );
        summarise(&hold, "classes per frame");
        {
            let shot = support::defects::read(&rig, &s.frame, None);
            let gw = s.frame.light_hit_positions().width() as usize;
            let cls = classes(&shot.hits, &shot.source, gw, cam2.position.to_array());
            hold_series(&hold, &cls, still_far_max);
            support::write_png(&dir.join("hold-final.png"), &shot.display, OUT[0], OUT[1]);
            let e = s.frame.exposure_readout(&rig.device).map(|r| r.exposure);
            println!(
                "  exposure at the end {e:?}; store missing {} black {}",
                shot.missing, shot.black
            );
        }
        println!("done: {}", dir.display());
    }
}
