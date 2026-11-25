//! The motion wobble's reading rig (ADR-0079; from the `spike-motion-wobble` branch): the forest
//! of seed 1, a still strip, a slow fly, a stop, a turn and a hold, each read as the display's
//! frame-to-frame change by hit distance, and the far pixels whose hit's record is fresh (the
//! "far-baked" class) beside all far ones, and the far's key source per frame (the record, the
//! column map's horizon, a landing's fade, the ray). Nothing is asserted: a reading in `mod
//! reading`. The protocol (Plan 0095 Phase 4): no pre-roll drain (the queue steps at the
//! fly-through's budget from the first frame), 120 settle frames, a still strip of 33, a fly of
//! 180 at 1 m/s, a stop held 300, a 60-degree turn over 40, a hold of 300; `drain=N` is the
//! drained control.
//! `SVET_WOBBLE` names the arm (see `Arm::parse`), `SVET_TEST_ADAPTER` the adapter. Frames and
//! maps go under `live/wobble/<arm>/`.
#![allow(
    dead_code,
    clippy::too_many_lines,
    clippy::disallowed_methods,
    reason = "a reading rig: its arms leave fields unread, its runs time themselves"
)]

#[path = "gpu_wobble/coarse.rs"]
mod coarse;
#[path = "gpu_wobble/fsr.rs"]
mod fsr;
mod support;
#[path = "gpu_wobble/terraces.rs"]
mod terraces;

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
use svet_render::{Camera, Frame};

/// The frame's sizes: FSR 2 at 1.5, `off` at 1.
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
    key_far: svet_render::KeyFar,
    key_far_mix: svet_render::KeyFarMix,
    /// The queue's budget a frame through the drain, in ms (`pre=MS`); `None` the default.
    pre_ms: Option<f64>,
    penumbra: svet_render::Penumbra,
    leaves_absorb: bool,
    /// The turn's frames (`turnf=N`, 40 by default): 240 is a slow pan.
    turn_frames: usize,
    /// The light passes' screen history (`Lighting::light_history`; `nohist` or `calm` clear it).
    light_history: bool,
    /// The upscaler's scale, output over internal width (`scale=X`, 1.5 by default); `off1`
    /// renders at the output.
    scale: f32,
    /// `Lighting::landing_fade` (`lfade`): a landing block's sky visibilities ride the store's
    /// fade.
    landing_fade: bool,
    /// `Lighting::source_fade` (`sfade`): a landing's switch of source fades by the block's
    /// landing frame.
    source_fade: bool,
    /// The fly's frames (`fly=N`, 180 by default; 20 is the short fly of Plan 0097's finding).
    fly: usize,
    /// The normals view (`normals`): the shading normal as colour, for the term split.
    normals: bool,
    /// Plan 0098 Phase 5's arms: `Lighting::far_light_wide` (`wide`) and
    /// `Lighting::light_history_far` (`hfar`).
    far_light_wide: bool,
    light_history_far: bool,
    /// Plan 0100 Phase 1's arms: where on a cube face the key light's column horizon is read
    /// (`keyat=hit|face|face-smooth`).
    key_at: svet_render::KeyAt,
    /// Plan 0100 Phase 3's arms, read by `coarse::reading` alone: the coarse pass's samples a
    /// brick direction (`coarse=N`, the fly-through's by default), the full-first radius
    /// (`ff=N|none`, the fly-through's by default) and the baked sky term read by the face's
    /// own normal (`skyface`, `--sky-normal face`).
    coarse: u32,
    full_first: Option<i32>,
    sky_face: bool,
}

impl Arm {
    /// `SVET_WOBBLE`: comma-separated tokens over the defaults (`default` alone is the
    /// fly-through's frame): `off1` (no upscaler, scale 1), `s0`
    /// (sharpen 0), `albedo`, `full`, `nofilter`, `farn0`, `pin`, `anyhist`, `keyoff`,
    /// `sky0`, `brick`, `sun=EL/AZ`, `speed=M_S`, `turn=DEG`, `drain=N`, `face` (store read
    /// face), `phases=N`, `dither`, `nostair`, `wnormal`, `ray` (`key_far ray`), `step`
    /// (`key_far_mix step`), `column` (`key_far column`), `pre=MS` (the queue's budget through
    /// the drain, the default's after it), `approach` / `disc` (the penumbra; the fly-through's
    /// default unless named), `absorb` (the leaves absorb, `--leaf-shadow absorb`), `turnf=N`
    /// (the turn's frames), `nohist` (`light_history` off), `calm` (window W1's five
    /// picks named, the fly-through's defaults (ADR-0082): `nohist`, `column`,
    /// `step`, `disc` and the far normal 4..8), `native` (as `off1`), `scale=X` (the upscaler's scale), `farn=LO/HI` (the far
    /// normal's band), `lfade` (`Lighting::landing_fade` on), `sfade` (`Lighting::source_fade`
    /// on), `fly=N` (the fly's frames) and
    /// `normals` (the normals view); Plan 0098 Phase 5's arms `wide` (`far_light_wide`) and
    /// `hfar` (`light_history_far`); Plan 0098 Phase 9's `hist` (`light_history` on, after
    /// `calm`) and `baked` (`key_far baked`); Plan 0100 Phase 1's `keyat=hit|face|face-smooth`
    /// (`Lighting::key_at`); Plan 0100 Phase 3's `coarse=N`, `ff=N|none` and `skyface`.
    fn parse() -> Self {
        Self::from_spec(&std::env::var("SVET_WOBBLE").unwrap_or_else(|_| "default".into()))
    }

    /// The arm `spec` names ([`Arm::parse`]'s tokens).
    fn from_spec(spec: &str) -> Self {
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
            drain: 0,
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
            penumbra: flythrough::DEFAULT_PENUMBRA,
            leaves_absorb: false,
            turn_frames: 40,
            light_history: flythrough::default_lighting().light_history,
            scale: 1.5,
            landing_fade: flythrough::default_lighting().landing_fade,
            source_fade: flythrough::default_lighting().source_fade,
            fly: 180,
            normals: false,
            far_light_wide: false,
            light_history_far: false,
            key_at: flythrough::default_lighting().key_at,
            coarse: svet_render::DEFAULT_COARSE_SAMPLES,
            full_first: Some(svet_render::DEFAULT_NEAR),
            sky_face: false,
        };
        for t in spec.split(',') {
            let (k, v) = t.split_once('=').unwrap_or((t, ""));
            match k {
                "default" => {}
                "tag" => {}
                "off1" => a.upscaler = "off",
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
                "ray" => a.key_far = svet_render::KeyFar::Ray,
                "column" => a.key_far = svet_render::KeyFar::Column,
                "step" => a.key_far_mix = svet_render::KeyFarMix::Step,
                "pre" => a.pre_ms = Some(v.parse().unwrap()),
                "approach" => a.penumbra = svet_render::Penumbra::Approach,
                "disc" => a.penumbra = svet_render::Penumbra::Disc,
                "absorb" => a.leaves_absorb = true,
                "turnf" => a.turn_frames = v.parse().unwrap(),
                "nohist" => a.light_history = false,
                "hist" => a.light_history = true,
                "baked" => a.key_far = svet_render::KeyFar::Baked,
                "scale" => a.scale = v.parse().unwrap(),
                "farn" => {
                    let (lo, hi) = v.split_once('/').expect("farn=LO/HI");
                    a.far_normal = [lo.parse().unwrap(), hi.parse().unwrap()];
                }
                "native" => a.upscaler = "off",
                "calm" => {
                    a.light_history = false;
                    a.key_far = svet_render::KeyFar::Column;
                    a.key_far_mix = svet_render::KeyFarMix::Step;
                    a.penumbra = svet_render::Penumbra::Disc;
                    a.far_normal = [4.0, 8.0];
                }
                "lfade" => a.landing_fade = true,
                "sfade" => a.source_fade = true,
                "normals" => a.normals = true,
                "wide" => a.far_light_wide = true,
                "hfar" => a.light_history_far = true,
                "fly" => a.fly = v.parse().unwrap(),
                "keyat" => a.key_at = svet_render::KeyAt::from_name(v).expect("keyat=NAME"),
                "coarse" => a.coarse = v.parse().unwrap(),
                "ff" => a.full_first = (v != "none").then(|| v.parse().unwrap()),
                "skyface" => a.sky_face = true,
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
    use svet_world::{ChunkExtent, World};
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
    let internal = if arm.upscaler == "off" {
        OUT
    } else if arm.scale == 1.5 {
        INTERNAL
    } else {
        OUT.map(|n| (n as f32 / arm.scale).round() as u32)
    };
    let mut f = Frame::with_sizes(&rig.device, internal, OUT);
    match arm.upscaler {
        "fsr2" => f.set_upscaler_kind(
            &rig.device,
            svet_render::UpscalerKind::Fsr2(svet_render::Fsr2Settings {
                sharpness: arm.sharpen,
                jitter_phases: arm.jitter_phases,
                dither: arm.dither,
                ..Default::default()
            }),
        ),
        _ => f.set_upscaler_kind(&rig.device, svet_render::UpscalerKind::Off),
    }
    f.set_count_store(true);
    f.set_look(svet_render::LookParams {
        far_normal: arm.far_normal,
        view: if arm.albedo {
            svet_render::ViewMode::Albedo
        } else if arm.normals {
            svet_render::ViewMode::Normals
        } else {
            svet_render::ViewMode::Shaded
        },
        ..svet_render::LookParams::default()
    });
    let leaf = if arm.leaves_brick {
        flythrough::LeafShadow::Brick
    } else if arm.leaves_absorb {
        flythrough::LeafShadow::Absorb
    } else {
        flythrough::LeafShadow::Voxel
    };
    f.set_material_styles(&rig.queue, &flythrough::outdoor_styles(leaf));
    f.set_lighting(svet_render::Lighting {
        lamp: false,
        filter: arm.filter,
        sky_rays: arm.sky_rays,
        key_history: arm.key_history,
        sky_history: arm.sky_history,
        key_far: arm.key_far,
        key_far_mix: arm.key_far_mix,
        penumbra: arm.penumbra,
        light_history: arm.light_history,
        landing_fade: arm.landing_fade,
        source_fade: arm.source_fade,
        far_light_wide: arm.far_light_wide,
        light_history_far: arm.light_history_far,
        key_at: arm.key_at,
        sky_normal: if arm.sky_face {
            svet_render::SkyNormal::Face
        } else {
            flythrough::default_lighting().sky_normal
        },
        store_read: if arm.store_read_face {
            svet_render::StoreRead::Face
        } else {
            flythrough::default_lighting().store_read
        },
        light_weights: if arm.weights_normal {
            svet_render::LightWeights::Normal
        } else {
            flythrough::default_lighting().light_weights
        },
        exposure: svet_render::Exposure::Auto(svet_render::AutoExposure {
            min: flythrough::cli::OUTDOOR_MIN,
            max: flythrough::cli::OUTDOOR_MAX,
            ..svet_render::AutoExposure::default()
        }),
        ..flythrough::default_lighting()
    });
    if arm.lighting_full {
        f.set_resolution(&rig.device, svet_render::LightingResolution::Full);
    }
    f.set_light_hooks(svet_render::LightHooks {
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
        svet_render::queue_settings(),
        svet_render::DEFAULT_BAKE_SLICE_MS,
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

/// The frames after the drain (none by default) before the segments, for the exposure.
const SETTLE: usize = 120;

/// A frame pair's near share over 4 levels, in %, over which [`pop_line`] prints it.
const POP_PRINT: f64 = 2.0;

/// Prints a pair of segment `name` at frame `k` whose near share over 4 levels (`c`, from
/// [`change_by_class`]) is over [`POP_PRINT`] %: that share, the near's mean |dY|, and the
/// queue's blocks landed between the counters `a` and `b`.
fn pop_line(
    name: &str,
    k: usize,
    c: &[(f64, f64); 5],
    a: &svet_render::QueueStats,
    b: &svet_render::QueueStats,
) {
    if c[1].0 > POP_PRINT {
        println!(
            "    pop {name} +{k}: near {:.2} % / {:.3}; landed baked {} (how {})",
            c[1].0,
            c[1].1,
            b.baked - a.baked,
            (0..4)
                .map(|i| (b.landings[i] - a.landings[i]).to_string())
                .collect::<Vec<_>>()
                .join("/")
        );
    }
}

/// The settle's first frames timed pass by pass: a fresh start, its chunks landing and fading.
const FRESH: usize = 60;

/// The far's key source over the far lighting pixels (hit over 15 m) where the key light falls:
/// shares of the record, the horizon, a landing's fade and the ray (the light pass's sample's b
/// says a ray was cast; else the key slot's tag, pair.wgsl's `key_slot_decode`).
fn far_sources(rig: &Rig, f: &Frame, eye: [f32; 3]) -> [f64; 4] {
    let hits = svet_render::debug::read_rgba32f(&rig.device, &rig.queue, f.light_hit_positions());
    let pair = svet_render::debug::read_rgba16f(&rig.device, &rig.queue, f.visibility_pair());
    let vis = svet_render::debug::read_rgba8(&rig.device, &rig.queue, f.visibility_samples());
    let mut n = [0usize; 4];
    for (i, h) in hits.iter().enumerate() {
        if h[3] > 1e30 || !h[3].is_finite() {
            continue;
        }
        let d = ((h[0] - eye[0]).powi(2) + (h[1] - eye[1]).powi(2) + (h[2] - eye[2]).powi(2))
            .sqrt()
            * VOXEL_M;
        let b = vis[4 * i + 2];
        if d < 15.0 || b < 32 {
            continue;
        }
        let y = pair[i][1];
        let k = if b > 127 {
            3
        } else if y <= -0.99 {
            0
        } else if (1.99..=3.01).contains(&y) {
            1
        } else {
            2
        };
        n[k] += 1;
    }
    let t = n.iter().sum::<usize>().max(1) as f64;
    n.map(|v| 100.0 * v as f64 / t)
}

fn fmt_sources(v: &[f64; 4]) -> String {
    format!(
        "record {:.1} % horizon {:.1} % fade {:.1} % ray {:.1} %",
        v[0], v[1], v[2], v[3]
    )
}

/// A lighting pixel's state for the near class's source reading ([`SrcRow`]): its sky source
/// word, whether the sky ray was cast, the visibility pair's mean, and whether its hit's chunk
/// holds a block in the store.
struct SrcState {
    source: Vec<u8>,
    cast: Vec<bool>,
    pair: Vec<f32>,
    block: Vec<bool>,
}

/// One stop pair's near sources (Plan 0098 Phase 6b's reading): over the near display pixels
/// (this frame's classes) whose luma moved over 4 levels, how many; of them, those whose hit's
/// chunk had no block the frame before and holds one now (in a landed chunk), and those whose
/// sky source went from no value with the ray cast to the record; the pair's mean before and
/// after over the moved pixels and its share at exactly 0 and 1 before (the column's answer);
/// and over every lighting pixel that cast the sky ray this frame, the share whose pair is
/// neither 0 nor 1 (a shown value other than the column's: a cast sample or a record).
#[derive(Clone, Copy, Default)]
struct SrcRow {
    moved: usize,
    landed: usize,
    switched: usize,
    vis_before: f64,
    vis_after: f64,
    before0: usize,
    before1: usize,
    cast: usize,
    cast_other: usize,
}

/// The [`SrcState`] of `f`'s last frame: the shot's source and cast words, the pair read back,
/// and each hit's chunk (the hit nudged along the view ray into the solid) looked up in the
/// store.
fn src_state(rig: &Rig, f: &Frame, shot: &support::defects::Shot, eye: [f32; 3]) -> SrcState {
    let pair = svet_render::debug::read_rgba16f(&rig.device, &rig.queue, f.visibility_pair());
    let mut held = std::collections::HashMap::<[i32; 3], bool>::new();
    let block = shot
        .hits
        .iter()
        .map(|h| {
            if h[3] > 1e30 || !h[3].is_finite() {
                return false;
            }
            let d = [h[0] - eye[0], h[1] - eye[1], h[2] - eye[2]];
            let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-6);
            let c = [0, 1, 2].map(|k| ((h[k] + 0.01 * d[k] / l) / 64.0).floor() as i32);
            *held
                .entry(c)
                .or_insert_with(|| f.store().block(svet_world::ChunkCoord(c)).is_some())
        })
        .collect();
    SrcState {
        source: shot.source.clone(),
        cast: shot.cast.clone(),
        pair: pair.iter().map(|p| p[0]).collect(),
        block,
    }
}

/// The [`SrcRow`] of the pair `a` to `b` (display lumas) over classes `cls`, lighting states
/// `pa` to `pb` on a grid `gw` wide.
fn src_row(a: &[u8], b: &[u8], cls: &[u8], pa: &SrcState, pb: &SrcState, gw: usize) -> SrcRow {
    let (dw, dh) = (OUT[0] as usize, OUT[1] as usize);
    let gl = pb.source.len();
    let mut r = SrcRow::default();
    for i in 0..a.len() {
        if cls[i] != 1 || a[i].abs_diff(b[i]) <= 4 {
            continue;
        }
        let g = grid_of(i, dw, dh, gw, gl);
        r.moved += 1;
        r.landed += usize::from(!pa.block[g] && pb.block[g]);
        r.switched += usize::from(
            pa.source[g] == support::defects::SOURCE_NO_VALUE
                && pa.cast[g]
                && pb.source[g] == support::defects::SOURCE_RECORD,
        );
        r.vis_before += f64::from(pa.pair[g]);
        r.vis_after += f64::from(pb.pair[g]);
        r.before0 += usize::from(pa.pair[g] == 0.0);
        r.before1 += usize::from(pa.pair[g] == 1.0);
    }
    for g in 0..gl {
        if pb.cast[g] {
            r.cast += 1;
            r.cast_other += usize::from(pb.pair[g] != 0.0 && pb.pair[g] != 1.0);
        }
    }
    r
}

fn fmt_src(r: &SrcRow) -> String {
    let m = r.moved.max(1) as f64;
    format!(
        "near moved {}: in a landed chunk {:.1} %, no value (cast) to record {:.1} %; pair before {:.3} after {:.3}, before at 0 {:.1} % at 1 {:.1} %; cast px {} of them pair not 0 or 1 {:.2} %",
        r.moved,
        100.0 * r.landed as f64 / m,
        100.0 * r.switched as f64 / m,
        r.vis_before / m,
        r.vis_after / m,
        100.0 * r.before0 as f64 / m,
        100.0 * r.before1 as f64 / m,
        r.cast,
        100.0 * r.cast_other as f64 / r.cast.max(1) as f64
    )
}

struct Segment {
    name: &'static str,
    /// Per frame of a kept segment, against the frame before (none at its first): the near
    /// class's sources ([`SrcRow`]).
    src: Vec<Option<SrcRow>>,
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
    /// Per frame: the far's key sources ([`far_sources`]).
    sources: Vec<[f64; 4]>,
    /// Per frame after its queue step: the live queue's counters (the default's with no queue).
    queue: Vec<svet_render::QueueStats>,
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
    let mut frames: Vec<Vec<u8>> = Vec::new();
    let mut diag = Vec::new();
    let mut gw = 1usize;
    let mut k = 0usize;
    let mut sources = Vec::new();
    let mut sum_pass = Vec::<f64>::new();
    let mut queue = Vec::new();
    let mut src = Vec::new();
    let mut src_prev: Option<SrcState> = None;
    for cam in cams {
        queue_step(rig, &mut s.frame, &cam);
        queue.push(s.frame.bake_queue().map(|q| q.stats()).unwrap_or_default());
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
        sources.push(far_sources(rig, &s.frame, cam.position.to_array()));
        if keep && DIAG_AT.contains(&k) {
            let mom = svet_render::debug::read_rgba16f(&rig.device, &rig.queue, s.frame.moments());
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
            if let (Some(a), Some(b)) =
                (queue.len().checked_sub(2).map(|i| &queue[i]), queue.last())
            {
                pop_line(name, k, pairs.last().unwrap(), a, b);
            }
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
            let now = src_state(rig, &s.frame, &shot, cam.position.to_array());
            let row = match (&src_prev, frames.last()) {
                (Some(pa), Some(a)) => Some(src_row(a, &y, &cls, pa, &now, gw)),
                _ => None,
            };
            src.push(row);
            src_prev = Some(now);
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
    let names = svet_render::FRAME_PASSES;
    let ms: Vec<String> = names
        .iter()
        .zip(&sum_pass)
        .filter(|(_, v)| **v / k as f64 > 0.05)
        .map(|(n, v)| format!("{n} {:.2}", v / k as f64))
        .collect();
    let src_mean = sources.iter().fold([0.0; 4], |a, v| {
        [0, 1, 2, 3].map(|c| a[c] + v[c] / sources.len().max(1) as f64)
    });
    println!(
        "  {name}: {k} frames; pass ms: {}; at its end {}; far key sources, mean: {}; first: {}; last: {}",
        ms.join(", "),
        depth(&s.frame),
        fmt_sources(&src_mean),
        sources.first().map_or("-".into(), fmt_sources),
        sources.last().map_or("-".into(), fmt_sources)
    );
    Segment {
        name,
        src,
        pairs,
        map,
        last: last.unwrap(),
        frames,
        diag,
        gw,
        sources,
        queue,
    }
}

/// The stop's frames `LANDING` one by one, and every other frame of the stop in whose step a
/// block landed or whose near pair is over [`LANDING_PAIR`]: the near class's over-4 share
/// against the previous frame and against the segment's last, beside the queue's landings in
/// that frame's step (chunks baked and key-only jobs ended; the blocks landed by how, refaded /
/// over the fallback / another layout unfaded / bare unfaded; the landed cells whose words
/// changed by read, directions / sky visibilities / sky flags / key visibilities / key flags /
/// tail, then every cell they hold) and the blocks fading after it, each pair with its mean
/// |dY|. Then the stop's largest near pair and its frame; [`landing_rows`] returns that pair.
const LANDING: std::ops::RangeInclusive<usize> = 36..=64;

/// A stop frame's near pair over 4 levels, in %, over which [`landing_rows`] prints it.
const LANDING_PAIR: f64 = 0.5;

fn landing_rows(seg: &Segment, cls: &[u8]) -> f64 {
    let last = seg.frames.last().unwrap();
    let mut max = (0.0f64, 0usize);
    for i in 1..seg.frames.len() {
        let (pair, mean) = change_by_class(&seg.frames[i - 1], &seg.frames[i], cls, 4)[1];
        if pair > max.0 {
            max = (pair, i);
        }
        let (a, b) = (&seg.queue[i - 1], &seg.queue[i]);
        let landed = b.baked - a.baked + b.keys_baked - a.keys_baked;
        if !LANDING.contains(&i) && landed == 0 && pair <= LANDING_PAIR {
            continue;
        }
        let to_final = change_by_class(&seg.frames[i], last, cls, 4)[1].0;
        let how: Vec<String> = (0..4)
            .map(|k| (b.landings[k] - a.landings[k]).to_string())
            .collect();
        let cells: Vec<String> = (0..7)
            .map(|k| (b.landing_cells[k] - a.landing_cells[k]).to_string())
            .collect();
        println!(
            "    landing {} +{i}: pair near {pair:.2} % (mean |dY| {mean:.2}); to-final near {to_final:.2} %; landed baked {} keys {}; how {}; cells {}; fading {}",
            seg.name,
            b.baked - a.baked,
            b.keys_baked - a.keys_baked,
            how.join("/"),
            cells.join("/"),
            b.fading
        );
        if let Some(Some(r)) = seg.src.get(i) {
            println!("      sources +{i}: {}", fmt_src(r));
        }
    }
    println!(
        "  {}: largest near pair {:.2} % at +{}",
        seg.name, max.0, max.1
    );
    if let Some(Some(r)) = seg.src.get(max.1) {
        println!("  {}: the largest pair's sources: {}", seg.name, fmt_src(r));
    }
    max.0
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
/// previous frame, and against the hold's last frame. Returns the near class's over-4 share at
/// +0 (the hold's first frame against its last).
fn hold_series(seg: &Segment, cls: &[u8], still_far_max: f64) -> f64 {
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
    let at: Vec<String> = [0usize, 30, 60, 120]
        .iter()
        .filter(|&&i| i < seg.frames.len())
        .map(|&i| {
            let c = change_by_class(&seg.frames[i], last, cls, 4);
            let src = seg.sources.get(i).map_or("-".into(), fmt_sources);
            format!(
                "+{i}: far {:.2} %/{:.3} near {:.2} %/{:.3} mid {:.2} % ({src})",
                c[3].0, c[3].1, c[1].0, c[1].1, c[2].0
            )
        })
        .collect();
    println!("  {} to-final over 4: {}", seg.name, at.join("; "));
    let near0 = change_by_class(&seg.frames[0], last, cls, 4)[1].0;
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
    near0
}

mod reading {
    use super::*;

    /// Plan 0097 Phase 3d bar 1: the auto exposure across a stop on the wobble rig's forest
    /// (`forest_rig(8)`, the `SVET_WOBBLE` arm, `default` unset): 120 settle frames at the
    /// spawn, a fly of `SVET_EYE_FLY` frames (20 unset) at the arm's speed, the stop held 30.
    /// Per frame from the fly's last to stop +29: the exposure the frame used (`previous`,
    /// log2) and its step from the frame before, the next frame's (`current`), the adapted and
    /// metered levels, the exposure's target (`key / 2^metered`), the ceiling in force and
    /// its target (`AutoExposure::ceiling_target` over the pass's ground, the metered sky
    /// share and the smoothed floor share), the highlight's tail level, its smoothed level and
    /// cap, which bound sets `current`, and the display's near pixels over 4 levels against
    /// the frame before. `SVET_TEST_ADAPTER` picks the adapter.
    #[test]
    #[ignore = "reading"]
    fn the_eye_across_a_stop() {
        let arm = Arm::parse();
        let fly_n: usize = std::env::var("SVET_EYE_FLY")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(20);
        let Some((mut rig, cam0)) = forest_rig(8) else {
            return;
        };
        let rig = &mut rig;
        println!(
            "the eye across a stop, arm {} on {}, fly {fly_n}",
            arm.name, rig.adapter
        );
        let cam0 = Camera {
            pitch: arm.pitch.to_radians(),
            yaw: cam0.yaw + arm.yaw.to_radians(),
            ..cam0
        };
        let f = frame_for(rig, &arm);
        let a = match f.lighting().exposure {
            svet_render::Exposure::Auto(a) => a,
            svet_render::Exposure::Manual(_) => panic!("the rig's exposure is auto"),
        };
        let mut s = Stepper::new(rig, f);
        for _ in 0..SETTLE {
            queue_step(rig, &mut s.frame, &cam0);
            s.run(rig, &cam0);
        }
        let step = arm.speed_m_s / 60.0;
        let mut cams: Vec<(String, Camera)> = (1..=fly_n)
            .map(|k| (format!("fly {k}"), moved(&cam0, step * k as f32)))
            .collect();
        let cam1 = cams.last().unwrap().1;
        cams.extend((0..30).map(|k| (format!("stop +{k}"), cam1)));
        let mut last: Option<(Vec<u8>, f64)> = None;
        let mut max_step = (0.0f64, String::new());
        for (i, (label, cam)) in cams.iter().enumerate() {
            queue_step(rig, &mut s.frame, cam);
            let shot = s.step(rig, cam);
            let f = &s.frame;
            let st = svet_render::ExposureState::from_bytes(&svet_render::debug::read_buffer(
                &rig.device,
                &rig.queue,
                f.exposure_state(),
                0,
                32,
            ));
            let hi = svet_render::ExposureHighlight::from_bytes(&svet_render::debug::read_buffer(
                &rig.device,
                &rig.queue,
                f.exposure_highlight(),
                0,
                16,
            ));
            let b =
                svet_render::debug::read_buffer(&rig.device, &rig.queue, f.sky_irradiance(), 0, 96);
            let up: Vec<f32> = b[80..96]
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect();
            let ground = svet_render::ground_irradiance([up[0], up[1], up[2]], up[3]);
            let floor = f.sky_floor();
            let gw = f.light_hit_positions().width() as usize;
            let cls = classes(&shot.hits, &shot.source, gw, cam.position.to_array());
            let y = lumas(&shot.display);
            let used = f64::from(st.previous).log2();
            let (near, d) = match &last {
                Some((p, u)) => (change_by_class(p, &y, &cls, 4)[1].0, used - u),
                None => (0.0, 0.0),
            };
            if i > 0 && d.abs() > max_step.0 {
                max_step = (d.abs(), label.clone());
            }
            let target =
                f64::from(a.key * a.compensation_ev.exp2()).log2() - f64::from(st.adapted_log2);
            let goal =
                f64::from(a.key * a.compensation_ev.exp2()).log2() - f64::from(st.metered_log2);
            let c_target = a.ceiling_target(Some(ground), st.sky, floor);
            let cap = hi.cap();
            let cur = f64::from(st.current).log2();
            let bound = if (cur - f64::from(a.min).log2()).abs() < 1e-4 {
                "min"
            } else if cap
                .is_some_and(|c| (cur - f64::from(c).log2()).abs() < 1e-4 && c <= st.ceiling)
            {
                "cap"
            } else if (cur - f64::from(st.ceiling).log2()).abs() < 1e-4 {
                "ceiling"
            } else {
                "adapted"
            };
            if i + 1 >= fly_n && i < fly_n + 16 || i + 1 == cams.len() {
                println!(
                    "eye {label}: used {used:+.4} step {d:+.4} EV, next {cur:+.4}, adapted {:+.4} (key/2^a {target:+.4}), metered {:+.4} (goal {goal:+.4}), ceiling {:+.4} target {:+.4}, sky {:.5}, floor {}, hi now {:+.4} smooth {:+.4} cap {}, bound {bound}, ground {ground:.2}, near over 4 {near:.2} %",
                    st.adapted_log2,
                    st.metered_log2,
                    f64::from(st.ceiling).log2(),
                    f64::from(c_target).log2(),
                    st.sky,
                    floor.map_or("none".into(), |v| format!("{v:.4}")),
                    hi.now_log2,
                    hi.log2,
                    cap.map_or("-".into(), |c| format!("{:+.4}", f64::from(c).log2())),
                );
            }
            last = Some((y, used));
        }
        println!(
            "eye: the largest one-frame step of the exposure used {:.4} EV at {}; the rate's {:.4} EV",
            max_step.0,
            max_step.1,
            f64::from(a.rate_ev_s) / 60.0
        );
    }

    /// Plan 0095 Phase 10's soak reading, run by hand: the rig's protocol ([`wobble_on`]) for
    /// Plan 0098 Phase 1's arm set `default`, `albedo`, `calm` and `calm,native` (the last with
    /// no upscaler, at 1080p native), no drain, on one forest (`r` 8) at 1080p through FSR 2 at
    /// 1.5: each arm's rows printed with the queue's depth and the far's key source split, and
    /// the stop +0 near of each in one row, so a later plan re-reads the wobble with the same
    /// instrument. Nothing is asserted: the claim it carried (the near does not go on settling
    /// after a stop, at most `albedo`'s plus 2 points) is asserted with the queue stopped and the
    /// exposure pinned, where nothing lands and the exposure takes no step, by
    /// [`super::the_defaults_hold_still_at_rest_and_the_far_holds`]. `SVET_TEST_ADAPTER` picks
    /// the adapter (the RTX by hand; unset, lavapipe).
    #[test]
    #[ignore = "reading"]
    fn the_wobble_table_over_the_arm_set() {
        let Some((mut rig, cam0)) = forest_rig(8) else {
            return;
        };
        let mut near = Vec::new();
        for spec in ["default", "albedo", "calm", "calm,native"] {
            near.push((spec, wobble_on(&mut rig, cam0, &Arm::from_spec(spec)).near0));
        }
        let row: Vec<String> = near.iter().map(|(a, v)| format!("{a} {v:.2} %")).collect();
        println!("stop +0 near on {}: {}", rig.adapter, row.join(", "));
    }

    #[test]
    #[ignore = "reading"]
    fn wobble() {
        let arm = Arm::parse();
        let Some((mut rig, cam0)) = forest_rig(8) else {
            return;
        };
        wobble_on(&mut rig, cam0, &arm);
    }
}

/// What [`wobble_on`] reads of a run's stop, in % of the class's pixels over 4 levels: the near
/// class from the stop's first frame to its last (`near0`), the stop's largest near pair
/// (`near_pair`), the far class from the stop's first frame (`far0`) and 30 frames into it
/// (`far30`) against its last, and the live queue's depth at the stop's end in words
/// (`depth`). Every figure is NaN for a `probe` arm, which runs no stop.
#[derive(Clone, Debug)]
struct StopRead {
    near0: f64,
    near_pair: f64,
    far0: f64,
    far30: f64,
    depth: String,
}

/// The rig's protocol for `arm` on `rig` from the spawn's camera `cam0`: the table's rows
/// printed, frames and maps under `live/wobble/<arm>/`. Returns what it read of the stop.
fn wobble_on(rig: &mut Rig, cam0: Camera, arm: &Arm) -> StopRead {
    QUEUE_ON.with(|q| q.set(true));
    {
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
        let f = frame_for(rig, arm);
        let mut s = Stepper::new(rig, f);
        if arm.probe {
            for k in 0..8 {
                let c = yawed(&cam0, 45.0 * k as f32);
                for _ in 0..30 {
                    queue_step(rig, &mut s.frame, &c);
                    s.run(rig, &c);
                }
                let shot = s.step(rig, &c);
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
            return StopRead {
                near0: f64::NAN,
                near_pair: f64::NAN,
                far0: f64::NAN,
                far30: f64::NAN,
                depth: depth(&s.frame),
            };
        }
        if let Some(ms) = arm.pre_ms {
            s.frame
                .set_bake_budget(svet_render::Budget::PerFrame { ms });
        }
        let t = std::time::Instant::now();
        let mut n = 0;
        while n < arm.drain && !s.frame.bake_queue().is_some_and(|q| q.empty()) {
            queue_step(rig, &mut s.frame, &cam0);
            s.run(rig, &cam0);
            n += 1;
        }
        let drained = s.frame.bake_queue().is_some_and(|q| q.empty());
        s.frame.set_bake_budget(svet_render::Budget::default());
        println!(
            "  queue drained {drained} after {n} frames ({:.1} s)",
            t.elapsed().as_secs_f64()
        );
        // The settle's first FRESH frames are timed (every chunk in view fading as it lands).
        // Every settle frame is read: a near pair over POP_PRINT % is printed (the spawn's own
        // chunks land here).
        let mut fresh = Vec::<f64>::new();
        let mut prev: Option<Vec<u8>> = None;
        let mut q_prev = s.frame.bake_queue().map(|q| q.stats()).unwrap_or_default();
        for k in 0..SETTLE {
            queue_step(rig, &mut s.frame, &cam0);
            let shot = s.step(rig, &cam0);
            if k < FRESH {
                fresh.resize(fresh.len().max(shot.passes.len()), 0.0);
                for (a, b) in fresh.iter_mut().zip(&shot.passes) {
                    *a += b / FRESH as f64;
                }
            }
            let gw = s.frame.light_hit_positions().width() as usize;
            let cls = classes(&shot.hits, &shot.source, gw, cam0.position.to_array());
            let y = lumas(&shot.display);
            let q = s.frame.bake_queue().map(|q| q.stats()).unwrap_or_default();
            if let Some(p) = &prev {
                pop_line("settle", k, &change_by_class(p, &y, &cls, 4), &q_prev, &q);
            }
            q_prev = q;
            prev = Some(y);
        }
        let fresh_ms: Vec<String> = svet_render::FRAME_PASSES
            .iter()
            .zip(&fresh)
            .filter(|(n, _)| ["light", "temporal", "atrous", "hit"].contains(n))
            .map(|(n, v)| format!("{n} {v:.2}"))
            .collect();
        println!(
            "  fresh start's first {FRESH} frames, pass ms: {}",
            fresh_ms.join(", ")
        );
        if arm.nqueue {
            QUEUE_ON.with(|q| q.set(false));
        }
        if arm.pin_exposure {
            let e = s
                .frame
                .exposure_readout(&rig.device)
                .map(|r| r.exposure)
                .expect("a read-back");
            s.frame.set_lighting(svet_render::Lighting {
                exposure: svet_render::Exposure::Manual(e),
                ..s.frame.lighting()
            });
            for _ in 0..32 {
                queue_step(rig, &mut s.frame, &cam0);
                s.run(rig, &cam0);
            }
        }
        // The still strip.
        let still = run_segment(
            rig,
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
            let shot = support::defects::read(rig, &s.frame, None);
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
        // The slow fly: `arm.fly` frames forward (180 by default).
        let step = arm.speed_m_s / 60.0;
        let fly_cams: Vec<Camera> = (1..=arm.fly)
            .map(|k| moved(&cam0, step * k as f32))
            .collect();
        let cam1 = *fly_cams.last().unwrap();
        let fly = run_segment(
            rig,
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
            rig,
            &mut s,
            std::iter::repeat_n(cam1, arm.hold),
            "stop",
            true,
            &dir,
            0,
            Some(fly.last.clone()),
        );
        summarise(&stop, "classes per frame");
        let stop_read;
        {
            let shot = support::defects::read(rig, &s.frame, None);
            let gw = s.frame.light_hit_positions().width() as usize;
            let cls = classes(&shot.hits, &shot.source, gw, cam1.position.to_array());
            let near0 = hold_series(&stop, &cls, still_far_max);
            let near_pair = landing_rows(&stop, &cls);
            let last = stop.frames.last().unwrap();
            let far0 = change_by_class(&stop.frames[0], last, &cls, 4)[3].0;
            let far30 = change_by_class(&stop.frames[30], last, &cls, 4)[3].0;
            stop_read = StopRead {
                near0,
                near_pair,
                far0,
                far30,
                depth: depth(&s.frame),
            };
            support::write_png(&dir.join("stop-final.png"), &shot.display, OUT[0], OUT[1]);
        }
        // The turn: `turn_deg` over `turn_frames` frames, then the hold.
        let tf = arm.turn_frames;
        let turn_cams: Vec<Camera> = (1..=tf)
            .map(|k| yawed(&cam1, arm.turn_deg * k as f32 / tf as f32))
            .collect();
        let cam2 = *turn_cams.last().unwrap();
        let turn = run_segment(
            rig,
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
            rig,
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
            let shot = support::defects::read(rig, &s.frame, None);
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
        stop_read
    }
}

/// The far class's over-4 share 30 frames into the stop against its last at rest
/// (`default,pin,nqueue`: the five picks, the exposure pinned, the queue stopped after the
/// settle), at the fly of 180 frames and of 20: the largest of three readings on the RTX 3080
/// Laptop (the readings log, Plan 0098 Phase 11: Phase 4's run and two more), which
/// [`the_defaults_hold_still_at_rest_and_the_far_holds`] bars against plus 2 points.
const REST_FAR30: [(usize, f64); 2] = [(180, REST_FAR30_180), (20, REST_FAR30_20)];
const REST_FAR30_180: f64 = 9.98;
const REST_FAR30_20: f64 = 11.39;

/// The near class's over-4 share from the stop's first frame to its last at rest, at most:
/// Plan 0095 Phase 10's bar as written, the lighting-off arm's (`albedo`) 0.05 % plus 2 points.
const REST_NEAR0: f64 = 2.05;

/// The largest near pair of the stop at rest, at most (the arm read 0.49, 0.08 and 0.01 % in
/// Plan 0098 Phases 6 and 6b; the bar is the largest plus 2 points).
const REST_NEAR_PAIR: f64 = 2.5;

/// Plan 0098 Phase 4's bars as ruled at its findings (ADR-0082): the rig's protocol
/// ([`wobble_on`]) on the forest (`r` 8) at 1080p through FSR 2 at 1.5, no drain, the exposure
/// pinned after the settle, the defaults with no token (the five picks), at the fly of 180
/// frames and of 20. Asserted at rest only, with the queue stopped after the settle
/// (`default,pin,nqueue`), where nothing lands and the exposure takes no step: no frame pair
/// of the stop moves more than [`REST_NEAR_PAIR`] of the near pixels over 4 levels, the near
/// from the stop's first frame to its last moves at most [`REST_NEAR0`], and the far 30 frames
/// into the stop against its last at most [`REST_FAR30`] plus 2 points. A figure that is not a
/// number fails. Printed, not barred: with the queue stepping (`default,pin`) and with no
/// upscaler (`default,pin,native`, 1080p native, the distance left) the far +30 and +0, the
/// queue's depth at the stop's end and every landing row with its frame: a landing's frame is
/// the machine's pace, not the rig's (accepted by eye at window W2). `SVET_TEST_ADAPTER` picks
/// the adapter (the RTX by hand; unset, lavapipe).
#[test]
#[ignore = "soak"]
fn the_defaults_hold_still_at_rest_and_the_far_holds() {
    let Some((mut rig, cam0)) = forest_rig(8) else {
        return;
    };
    let mut fails = Vec::new();
    for (fly, bar) in REST_FAR30 {
        let mut read = |spec: &str| {
            wobble_on(
                &mut rig,
                cam0,
                &Arm::from_spec(&format!("{spec},fly={fly}")),
            )
        };
        let rest = read("default,pin,nqueue");
        let picks = read("default,pin");
        let native = read("default,pin,native");
        println!(
            "fly {fly} on {}: at rest the largest stop near pair {:.2} % (bar {REST_NEAR_PAIR}), stop +0 near {:.2} % (bar {REST_NEAR0}), far +30 {:.2} % against {bar:.2} % + 2 (far +0 {:.2} %, {}); stepping (printed): far +30 {:.2} %, far +0 {:.2} %, the largest near pair {:.2} %, {}; native (printed): far +30 {:.2} %, far +0 {:.2} %, {}",
            rig.adapter,
            rest.near_pair,
            rest.near0,
            rest.far30,
            rest.far0,
            rest.depth,
            picks.far30,
            picks.far0,
            picks.near_pair,
            picks.depth,
            native.far30,
            native.far0,
            native.depth,
        );
        // A figure that is not a number fails.
        if rest.near_pair.is_nan() || rest.near_pair > REST_NEAR_PAIR {
            fails.push(format!(
                "fly {fly}: a stop pair at rest moves {:.2} % of the near over 4",
                rest.near_pair
            ));
        }
        if rest.near0.is_nan() || rest.near0 > REST_NEAR0 {
            fails.push(format!(
                "fly {fly}: the stop +0 near at rest {:.2} % against {REST_NEAR0} %",
                rest.near0
            ));
        }
        if rest.far30.is_nan() || rest.far30 > bar + 2.0 {
            fails.push(format!(
                "fly {fly}: far +30 at rest {:.2} % against {bar:.2} % + 2",
                rest.far30
            ));
        }
    }
    assert!(fails.is_empty(), "{}", fails.join("; "));
}

/// One run of the gate scene's protocol: the share of far pixels over 4 levels on each
/// segment, and the near's at the stop.
#[derive(Debug)]
struct Gate {
    /// The still strip's far pairs, mean.
    still: f64,
    /// The fly's far and near pairs, mean.
    fly: f64,
    fly_near: f64,
    /// The stop's first frame against its last: near, far.
    stop_near: f64,
    stop_far: f64,
    /// The hold's frames +0, +30 and +60 against its last, far.
    hold: [f64; 3],
    /// The far's and near's shares of the display at the hold's end.
    far_px: f64,
    near_px: f64,
}

/// The gate scene's frames: 120 settle, a still strip of 16, a fly of 20 at 1 m/s, a stop held
/// 60, a 60-degree turn over 10, a hold of 120.
const GATE_SETTLE: usize = 120;
const GATE_STILL: usize = 16;
const GATE_FLY: usize = 20;
const GATE_STOP: usize = 60;
const GATE_TURN: usize = 10;
const GATE_HOLD: usize = 120;

/// The gate scene's protocol for `arm` from `cam0`: the queue steps through the settle, and
/// after it only when `!frozen`.
fn gate_run(rig: &mut Rig, cam0: &Camera, arm: support::wobble::Arm, frozen: bool) -> Gate {
    use support::wobble::{FAR, NEAR, depth, frame, moved, over, share, step, yawed};
    let mut f = frame(rig, arm);
    for _ in 0..GATE_SETTLE {
        step(rig, &mut f, cam0, true);
    }
    let q = !frozen;
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
    let mut last = support::wobble::read(rig, &f, cam0);
    let mut still = Vec::new();
    for _ in 0..GATE_STILL {
        let r = step(rig, &mut f, cam0, q);
        still.push(over(&last.luma, &r.luma, &r.class, FAR));
        last = r;
    }
    let (mut fly, mut fly_near) = (Vec::new(), Vec::new());
    let mut cam = *cam0;
    for k in 1..=GATE_FLY {
        cam = moved(cam0, k as f32 / 60.0);
        let r = step(rig, &mut f, &cam, q);
        fly.push(over(&last.luma, &r.luma, &r.class, FAR));
        fly_near.push(over(&last.luma, &r.luma, &r.class, NEAR));
        last = r;
    }
    let stop: Vec<_> = (0..GATE_STOP).map(|_| step(rig, &mut f, &cam, q)).collect();
    let end = stop.last().unwrap();
    let stop_near = over(&stop[0].luma, &end.luma, &end.class, NEAR);
    let stop_far = over(&stop[0].luma, &end.luma, &end.class, FAR);
    let mut turned = cam;
    for k in 1..=GATE_TURN {
        turned = yawed(&cam, 60.0 * k as f32 / GATE_TURN as f32);
        step(rig, &mut f, &turned, q);
    }
    let hold: Vec<_> = (0..GATE_HOLD)
        .map(|_| step(rig, &mut f, &turned, q))
        .collect();
    let end = hold.last().unwrap();
    let g = Gate {
        still: mean(&still),
        fly: mean(&fly),
        fly_near: mean(&fly_near),
        stop_near,
        stop_far,
        hold: [0, 30, 60].map(|i| over(&hold[i].luma, &end.luma, &end.class, FAR)),
        far_px: share(&end.class, FAR),
        near_px: share(&end.class, NEAR),
    };
    println!(
        "gate, {} on {}{}: still far {:.2} %; fly far {:.2} % near {:.2} %; stop +0 near {:.2} % far {:.2} %; hold +0 / +30 / +60 far {:.2} / {:.2} / {:.2} %; far {:.1} % of the display, near {:.1} %; {}",
        arm.name(),
        rig.adapter,
        if frozen {
            ", queue frozen"
        } else {
            ", undrained"
        },
        g.still,
        g.fly,
        g.fly_near,
        g.stop_near,
        g.stop_far,
        g.hold[0],
        g.hold[1],
        g.hold[2],
        g.far_px,
        g.near_px,
        depth(&f)
    );
    g
}

/// Plan 0095 Phase 10 bar 1 (ADR-0079, as amended 2026-11-20: the far no worse than the before):
/// [`support::wobble`]'s scene, a 3 x 3 x 2 chunk forest of seed 1 at 320 x 180 through FSR 2
/// at 1.5 to 480 x 270, no drain, the queue stepping at the default budget from the first frame;
/// [`gate_run`]'s protocol (120 settle frames, a still strip of 16, a 20-frame fly at 1 m/s, a
/// stop held 60, a 60-degree turn over 10 frames, a hold of 120: the hold runs past 60 frames so
/// +60 has a final frame to read against). With the queue frozen at the settle's end, under the
/// picks against the before (`key far ray`, `penumbra disc`) read in the same run: the far
/// class's share over 4 levels on the still's pairs (mean), the fly's pairs (mean), and from the
/// hold's last frame at +30 and +60, each at or under the before's plus 1 point. Printed, not
/// asserted (as amended 2026-11-21): the near class's stop +0 beside the lighting-off (albedo)
/// run's (at this 320-pixel grid the far normal's band starts at about 2.2 m, so most near
/// pixels read the far estimate; the near bar is the 1080p soak test's at rest,
/// [`the_defaults_hold_still_at_rest_and_the_far_holds`]), and the same segments undrained with the queue's
/// depth, the picks' +60 beside the before's (Plan 0096's far tiers re-read it). Far is a hit 6 m out or more
/// ([`support::wobble::NEAR_M`]: the far normal's weight at least 0.996 at this width). Every
/// figure prints. `SVET_TEST_ADAPTER` picks the adapter (unset, lavapipe).
#[test]
#[ignore = "soak"]
fn the_far_moves_no_worse_than_the_before() {
    use support::wobble::Arm;
    let Some((mut rig, cam)) = support::wobble::forest() else {
        return;
    };
    let picks = gate_run(&mut rig, &cam, Arm::Picks, true);
    let before = gate_run(&mut rig, &cam, Arm::Before, true);
    let off = gate_run(&mut rig, &cam, Arm::Albedo, true);
    let picks_u = gate_run(&mut rig, &cam, Arm::Picks, false);
    let before_u = gate_run(&mut rig, &cam, Arm::Before, false);
    println!(
        "gate readings: stop +0 near picks {:.2} %, before {:.2} %, lighting off {:.2} %; undrained +60 far picks {:.2} %, before {:.2} %",
        picks.stop_near, before.stop_near, off.stop_near, picks_u.hold[2], before_u.hold[2]
    );
    assert!(
        picks.far_px > 5.0,
        "far {:.1} % of the display",
        picks.far_px
    );
    for (seg, p, b) in [
        ("still", picks.still, before.still),
        ("fly", picks.fly, before.fly),
        ("hold +30", picks.hold[1], before.hold[1]),
        ("hold +60", picks.hold[2], before.hold[2]),
    ] {
        assert!(
            p <= b + 1.0,
            "{seg}: far {p:.2} % over 4 against the before's {b:.2} % + 1"
        );
    }
}
