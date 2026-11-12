//! The settle, dot and flicker rig: scripted camera runs over `stillness`'s level and poses,
//! every part of the light read per frame, and the figures below from them.
//!
//! **Parts.** The composite (the meter's 8 x 8 tiles of the shaded radiance's clamped log2,
//! `Frame::exposure_tiles`, for the settle; its display-encoded image from
//! `Frame::composite_capture`, or under FSR 2 its linear input through the composite's own tone
//! map, for the dots and the flicker), the display image (the upscaled
//! frame target), and on the lighting grid the emissive accumulation (`Frame::accumulated`
//! rgb), the lamp's part (its a), the a-trous output (`Frame::filtered`) and the gather
//! (`Frame::indirect_irradiance`), plus the pixel age (`Frame::moments` z) and the exposure the
//! frame used (`ExposureState::previous`). The emissive, lamp and a-trous parts are
//! pre-exposed, so the rig divides them by the frame's exposure; the gather is absolute (the
//! composite multiplies it by the exposure). Their figures are of absolute light. The composite's
//! image and the display are read as the eye sees them: the luma of `pow(c, 2.2)`.
//!
//! Every grid's pixel maps to one internal pixel, whose hit (`Frame::hit_positions`,
//! `Frame::hit_records`) and meter tile it takes: a lighting pixel to the pixel of its block
//! the half-resolution hit buffer samples (`HALF_OFFSET`), a display pixel to the internal
//! pixel under it. A tile's value is the mean of `meter_log2` over its pixels (the stillness
//! rig's transform), and a tile counts where the meter's tile is full (every pixel hit, mean
//! above `FULL_LOG2`) and the part's mean is above `FULL_LOG2` too.
//!
//! **The settle** of a part after an event: the reference is each tile's mean over frames
//! `REF.0 ..= REF.1` after it, camera still; a tile is settled within `log2 1.1` of it; `S` is
//! the last frame up to `UNTIL` at which under 90 % of the counted tiles are settled (0 when
//! none is; `None` when frame `UNTIL` itself is under 90 %), so every frame after `S` is
//! settled. The mean-luminance frame is the first after which the mean over the counted tiles
//! of `2^tile` stays within 10 % of its change from frame 0 (the last frame before the event).
//!
//! **Dots** in a display frame: a pixel whose encoded luma (in /255) is over its 5 x 5
//! median by more than 24 and more than twice the median. The same rule on a lighting part's
//! pre-exposed luminance (24/255 as 0.094) marks its outliers, and a dot is attributed to
//! every part with an outlier in the 3 x 3 lighting pixels under it; with none, to the
//! composite when its own image has an outlier in the 3 x 3 internal pixels under it, else to
//! the upscaler. The floor is the mean count over the reference frames, settled and still.
//!
//! **The world-keyed flicker** `K`: each internal pixel of the composite with a hit is keyed by
//! its voxel `floor(p - 0.5 nf)` and its entry face; a key seen in two consecutive frames
//! contributes `|log2 mean_f - log2 mean_f-1|` of its pixels' mean luma, under `meter_log2`'s
//! floor; `K` is the mean over frames and keys of a run's frames, and its p95 ([`KeyFlicker`]).
//!
//! Given a filmstrip tag ([`settle_run`]'s `tag`), the runs write contact sheets under the
//! worktree's `live/filmstrip/<tag>/`.

use std::collections::HashMap;

use super::stillness::{FULL_LOG2, PATCH_POSE, Pose, SIZE, TILE, VEIN_POSE, VeinLevel, meter_log2};
use super::*;
use flythrough::LookPreset;
use vxe_render::ExposureState;
use vxe_render::glam::Vec3;
use vxe_render::{
    AutoExposure, Exposure, Frame, Fsr2Settings, LightParams, Lighting, LookParams, UpscalerKind,
};

mod runs;

pub use runs::*;

/// The display size with the upscaler at the fly-through's default scale 1.5 over `SIZE`.
pub const OUTPUT: [u32; 2] = [288, 162];
/// Frames a run settles still before its event.
pub const SETTLE: usize = 1024;
/// The frames after an event whose tile means are the settle's reference, inclusive.
pub const REF: (usize, usize) = (2048, 2111);
/// The last frame the settle's 90 % must hold through.
pub const UNTIL: usize = 2048;
/// Frames after an event the dots are read on, from 1.
pub const DOT_FRAMES: usize = 60;
/// The fly-through's flying speed (`fly.rs`'s `SPEED`), voxels a second.
pub const FLY_SPEED: f32 = 30.0;
/// The fly-in's speed in voxels a second and its length in voxels (10 probe spacings).
pub const FLY_IN_SPEED: f32 = 40.0;
pub const FLY_IN_LENGTH: f32 = 160.0;
/// The steady flight's frames, and the first frame the flicker's `K` reads.
pub const FLIGHT_FRAMES: usize = 360;
pub const SHIMMER_FROM: usize = 64;
/// The strip tool's flight (`tests/gpu_strip.rs`): voxels a second along the view, its yaw rate in
/// degrees a second, and its frames.
pub const STRIP_SPEED: f32 = 40.0;
pub const STRIP_YAW: f32 = 12.0;
pub const STRIP_FRAMES_FLOWN: usize = 180;
/// The still frames before the strip flight whose dots are its floor.
pub const STRIP_FLOOR_FRAMES: usize = 64;
/// The warp's acceptance (the dots' reject share): the f-1 hit within this many voxels, its
/// normal within this cosine.
pub const WARP_DIST: f32 = 0.5;
pub const WARP_COS: f32 = 0.9;
/// The dig's carve radius in voxels.
pub const DIG_RADIUS: i64 = 10;
/// The far dig's distance from the eye, in voxels: down the passage.
pub const DIG_FAR: u32 = 100;
/// `frame/settings.rs`'s `HALF_OFFSET`: the internal pixel of each 2 x 2 the half-resolution
/// lighting grid samples.
const HALF_OFFSET: [u32; 2] = [0, 1];

/// The fly-in's start for each pose, flown in a straight line to the pose holding its
/// orientation (found by `find_paths`: every half-voxel step has a cube of 5 voxels of air
/// around it at the vein, of 3 at the patch pose). At `VEIN_POSE` it is `FLY_IN_LENGTH` (10
/// probe spacings) out, the clear direction nearest the camera's backward one. `PATCH_POSE`
/// sits in a pocket whose longest clear line at that clearance is 156 voxels; its fly-in is
/// 152 voxels along yaw -50, pitch -20 degrees (9.5 spacings).
pub const PATCH_FLY_IN_START: [f32; 3] = [
    676.2 + 152.0 * 0.604_022_7,
    514.8 - 152.0 * 0.719_846_4,
    193.1 - 152.0 * 0.342_020_1,
];
pub const VEIN_FLY_IN_START: [f32; 3] = [877.2787, 631.196, 263.2232];
/// The steady flight's ends for each pose: 180 voxels flown facing the direction of travel
/// (found by `find_paths`, a cube of 5 voxels of air around every step). At `VEIN_POSE` it
/// ends at the pose; no clear 180-voxel line ends within 60 voxels of `PATCH_POSE`, so its
/// flight ends 62 voxels short of it, flying toward the patch pose's wall (the direction is
/// within 14 degrees of the pose's forward one).
pub const PATCH_FLIGHT: ([f32; 3], [f32; 3]) =
    ([507.4759, 395.22766, 264.66364], [616.2, 524.8, 203.1]);
pub const VEIN_FLIGHT: ([f32; 3], [f32; 3]) =
    ([882.7914, 733.9575, 192.81197], [727.5, 644.3, 208.5]);

/// A run's light: the lamp, indirect light, the eye adaptation (`Auto`, the fly-through's
/// eye) or the pose's pinned exposure, and the upscaler at 1.5 (the fly-through's default:
/// FSR 2, sharpening 0.5, over `LookPreset::DEFAULT`) or the frame at scale 1 with no upscaler.
/// `bridge` renders `LookPreset::FACE` under our upscaler at 1.5 jittered instead, the setting
/// Plan 0049's tables were read at. `probe_freeze` and `probe_move` override the lamp channel's
/// freeze count and moving-lamp blend (`h_move`); `None` keeps the renderer's defaults.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Setting {
    pub lamp: bool,
    pub indirect: bool,
    pub auto: bool,
    pub upscaler: bool,
    pub bridge: bool,
    pub probe_freeze: Option<u32>,
    pub probe_move: Option<f32>,
}

impl Setting {
    pub fn name(&self) -> String {
        let on = |b: bool| if b { "on" } else { "off" };
        format!(
            "lamp {} indirect {} {} {}{}",
            on(self.lamp),
            on(self.indirect),
            if self.auto { "auto" } else { "pinned" },
            self.upscaler_name(),
            self.freeze_words()
        )
    }

    /// The freeze count for a run's label, empty at the default.
    pub fn freeze_words(&self) -> String {
        match self.probe_freeze {
            Some(f) => format!(" probe-freeze {f}"),
            None => String::new(),
        }
    }

    pub fn upscaler_name(&self) -> &'static str {
        match (self.bridge, self.upscaler) {
            (true, _) => "bridge",
            (false, true) => "up1.5",
            (false, false) => "scale1",
        }
    }

    fn tag(&self) -> String {
        format!(
            "lamp{}-ind{}-{}-{}{}",
            u8::from(self.lamp),
            u8::from(self.indirect),
            if self.auto { "auto" } else { "pinned" },
            self.upscaler_name(),
            self.freeze_words().replace(' ', "-")
        )
    }
}

/// `preset`'s look, shaded.
pub fn look_of(preset: LookPreset) -> LookParams {
    LookParams {
        normal_mode: preset.normal_mode,
        tuning: preset.tuning,
        far_normal: preset.far_normal,
        ..LookParams::default()
    }
}

/// FSR 2 as the fly-through starts it (`--sharpen`'s default 0.5).
pub fn start_upscaler() -> UpscalerKind {
    UpscalerKind::Fsr2(Fsr2Settings {
        sharpness: 0.5,
        ..Fsr2Settings::default()
    })
}

/// A frame at `setting` for `pose`, its composite readable ([`Pixels::composite`]).
pub fn make_frame(level: &VeinLevel, pose: &Pose, setting: Setting) -> Frame {
    let (device, queue) = (&level.device, &level.queue);
    let mut frame = if setting.bridge {
        let mut f = Frame::with_sizes(device, SIZE, OUTPUT);
        f.set_jitter(device, true);
        f.set_upscaler_kind(device, UpscalerKind::Own(vxe_render::Upscaler::default()));
        f.set_sharpness(queue, 0.5);
        f.set_look(look_of(LookPreset::FACE));
        f
    } else if setting.upscaler {
        let mut f = Frame::with_sizes(device, SIZE, OUTPUT);
        f.set_upscaler_kind(device, start_upscaler());
        f.set_look(look_of(LookPreset::DEFAULT));
        f
    } else {
        let mut f = Frame::new(device, SIZE[0], SIZE[1]);
        f.set_look(look_of(LookPreset::DEFAULT));
        f
    };
    if frame.fsr2().is_none() {
        frame.set_composite_capture(device, true);
    }
    frame.set_lighting(lighting(pose, setting));
    frame.set_dt(1.0 / 60.0);
    frame
}

fn lighting(pose: &Pose, setting: Setting) -> Lighting {
    Lighting {
        // The fly-through's store read and filter weights, with its look.
        store_read: flythrough::DEFAULT_STORE_READ,
        light_weights: flythrough::DEFAULT_LIGHT_WEIGHTS,
        lamp: setting.lamp,
        lamp_bounce: setting.indirect,
        probe_freeze: setting
            .probe_freeze
            .unwrap_or(vxe_render::DEFAULT_PROBE_FREEZE),
        probe_move_blend: setting.probe_move.unwrap_or(vxe_render::DEFAULT_H_MOVE),
        exposure: if setting.auto {
            Exposure::Auto(AutoExposure::eye())
        } else {
            Exposure::Manual(pose.exposure)
        },
        ..Lighting::default()
    }
}

/// One frame's read-back.
pub struct Pixels {
    pub camera: Camera,
    pub exposure: f32,
    /// The meter's tiles: mean, and whether full.
    pub meter: Vec<(f32, bool)>,
    pub display: Vec<u8>,
    pub display_size: [u32; 2],
    /// The composite as seen: tone mapped and display-encoded, rgb in 0..1.
    pub composite: Vec<[f32; 3]>,
    pub grid: [u32; 2],
    pub accum: Vec<[f32; 4]>,
    pub filtered: Vec<[f32; 4]>,
    pub gather: Vec<[f32; 4]>,
    pub moments: Vec<[f32; 4]>,
    /// The internal hit buffer, read for the warp and the reject share only.
    pub hit_pos: Vec<[f32; 4]>,
    pub hit_info: Vec<[u16; 4]>,
}

/// What a frame's read-back includes beyond the tiles' sources.
#[derive(Clone, Copy)]
struct Want {
    hits: bool,
    composite: bool,
    moments: bool,
}

fn read(level: &VeinLevel, frame: &Frame, camera: Camera, want: Want) -> Pixels {
    let (device, queue) = (&level.device, &level.queue);
    let [tw, th] = SIZE.map(|n| n.div_ceil(TILE));
    let n = (tw * th) as u64;
    let b = vxe_render::debug::read_buffer(device, queue, frame.exposure_tiles(), 0, 8 * n);
    let meter = b
        .chunks_exact(8)
        .map(|c| {
            let sum = f32::from_le_bytes(c[0..4].try_into().unwrap());
            let k = f32::from_le_bytes(c[4..8].try_into().unwrap());
            let m = sum / k.max(1.0);
            (m, k >= (TILE * TILE) as f32 && m > FULL_LOG2)
        })
        .collect();
    let st = vxe_render::debug::read_buffer(device, queue, frame.exposure_state(), 0, 32);
    let exposure = ExposureState::from_bytes(&st).previous;
    let acc = frame.accumulated();
    let grid = [acc.width(), acc.height()];
    let t = frame.target();
    Pixels {
        camera,
        exposure,
        meter,
        display: vxe_render::debug::read_rgba8(device, queue, t),
        display_size: [t.width(), t.height()],
        composite: if want.composite {
            read_composite(level, frame)
        } else {
            Vec::new()
        },
        grid,
        accum: vxe_render::debug::read_float(device, queue, acc),
        filtered: vxe_render::debug::read_float(device, queue, frame.filtered()),
        gather: vxe_render::debug::read_float(device, queue, frame.indirect_irradiance()),
        moments: if want.moments {
            vxe_render::debug::read_float(device, queue, frame.moments())
        } else {
            Vec::new()
        },
        hit_pos: if want.hits {
            vxe_render::debug::read_float(device, queue, frame.hit_positions())
        } else {
            Vec::new()
        },
        hit_info: if want.hits {
            vxe_render::debug::read_rgba16u(device, queue, frame.hit_records())
        } else {
            Vec::new()
        },
    }
}

/// The composite's image as seen: the capture, or under FSR 2 its linear input through
/// `composite.wgsl`'s ACES tone map and encode.
fn read_composite(level: &VeinLevel, frame: &Frame) -> Vec<[f32; 3]> {
    let (device, queue) = (&level.device, &level.queue);
    if let Some(fsr) = frame.fsr2() {
        let aces = |x: f32| {
            let x = x.max(0.0);
            ((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14)).clamp(0.0, 1.0)
        };
        return vxe_render::debug::read_float(device, queue, fsr.colour_in())
            .into_iter()
            .map(|c| [c[0], c[1], c[2]].map(|v| aces(v).powf(1.0 / 2.2)))
            .collect();
    }
    let capture = frame.composite_capture().expect("captured");
    vxe_render::debug::read_rgba8(device, queue, capture)
        .chunks_exact(4)
        .map(|c| [c[0], c[1], c[2]].map(|v| f32::from(v) / 255.0))
        .collect()
}

/// The parts the rig reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Composite,
    Display,
    Emissive,
    Lamp,
    Filtered,
    Gather,
}

pub const PARTS: [Part; 6] = [
    Part::Composite,
    Part::Display,
    Part::Emissive,
    Part::Lamp,
    Part::Filtered,
    Part::Gather,
];
pub const LIGHTING_PARTS: [Part; 4] = [Part::Emissive, Part::Lamp, Part::Filtered, Part::Gather];

impl Part {
    pub fn name(self) -> &'static str {
        match self {
            Part::Composite => "composite",
            Part::Display => "display",
            Part::Emissive => "emissive",
            Part::Lamp => "lamp",
            Part::Filtered => "filtered",
            Part::Gather => "gather",
        }
    }
}

fn lum(p: [f32; 4]) -> f32 {
    0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
}

/// The luma of a display-encoded rgba8 pixel, decoded by `pow(c, 2.2)`.
fn display_lum(p: &[u8]) -> f32 {
    let d = |c: u8| (f32::from(c) / 255.0).powf(2.2);
    0.2126 * d(p[0]) + 0.7152 * d(p[1]) + 0.0722 * d(p[2])
}

/// The luma of an encoded rgb, decoded by `pow(c, 2.2)`.
fn seen_lum(c: &[f32; 3]) -> f32 {
    0.2126 * c[0].powf(2.2) + 0.7152 * c[1].powf(2.2) + 0.0722 * c[2].powf(2.2)
}

/// The encoded luma in /255.
fn byte_luma(p: &[u8]) -> f32 {
    0.2126 * f32::from(p[0]) + 0.7152 * f32::from(p[1]) + 0.0722 * f32::from(p[2])
}

impl Pixels {
    /// Part `p`'s grid and its linear value per pixel: the lighting parts absolute (the
    /// pre-exposed emissive, lamp and a-trous parts divided by the frame's exposure, the
    /// absolute gather as read) unless `pre_exposed` (the gather times the exposure), the
    /// composite and the display as seen.
    fn values(&self, p: Part, pre_exposed: bool) -> ([u32; 2], Vec<f32>) {
        let e = if pre_exposed {
            1.0
        } else {
            1.0 / self.exposure
        };
        match p {
            Part::Composite => (SIZE, self.composite.iter().map(seen_lum).collect()),
            Part::Display => (
                self.display_size,
                self.display.chunks_exact(4).map(display_lum).collect(),
            ),
            Part::Emissive => (self.grid, self.accum.iter().map(|&a| lum(a) * e).collect()),
            Part::Lamp => (self.grid, self.accum.iter().map(|a| a[3] * e).collect()),
            Part::Filtered => (
                self.grid,
                self.filtered.iter().map(|&a| lum(a) * e).collect(),
            ),
            // The gather is absolute: the composite multiplies it by the frame's exposure.
            Part::Gather => {
                let g = if pre_exposed { self.exposure } else { 1.0 };
                (self.grid, self.gather.iter().map(|&a| lum(a) * g).collect())
            }
        }
    }

    fn scale(&self) -> u32 {
        SIZE[0].div_ceil(self.grid[0])
    }

    /// The internal pixel a pixel of `grid` takes its hit and tile from.
    fn internal_of(&self, grid: [u32; 2], x: u32, y: u32) -> (u32, u32) {
        if grid == self.grid {
            let s = self.scale();
            if s == 1 {
                (x, y)
            } else {
                (
                    (s * x + HALF_OFFSET[0]).min(SIZE[0] - 1),
                    (s * y + HALF_OFFSET[1]).min(SIZE[1] - 1),
                )
            }
        } else if grid == SIZE {
            (x, y)
        } else {
            (x * SIZE[0] / grid[0], y * SIZE[1] / grid[1])
        }
    }

    /// The lighting pixel under internal pixel `(x, y)`.
    fn lighting_of(&self, x: u32, y: u32) -> usize {
        let s = self.scale();
        ((y / s).min(self.grid[1] - 1) * self.grid[0] + (x / s).min(self.grid[0] - 1)) as usize
    }

    /// The age of lighting pixel `i`.
    fn age(&self, i: usize) -> u32 {
        self.moments[i][2].max(0.0) as u32
    }

    /// Part `p` reduced to the meter's tiles.
    fn tiles(&self, p: Part) -> Vec<f32> {
        let [tw, th] = SIZE.map(|n| n.div_ceil(TILE));
        if p == Part::Composite {
            return self.meter.iter().map(|m| m.0).collect();
        }
        let (grid, v) = self.values(p, false);
        let mut s = vec![0.0f64; (tw * th) as usize];
        let mut c = vec![0u32; (tw * th) as usize];
        for y in 0..grid[1] {
            for x in 0..grid[0] {
                let (ix, iy) = self.internal_of(grid, x, y);
                let t = ((iy / TILE) * tw + ix / TILE) as usize;
                s[t] += f64::from(meter_log2(v[(y * grid[0] + x) as usize]));
                c[t] += 1;
            }
        }
        s.iter()
            .zip(&c)
            .map(|(&s, &c)| (s / f64::from(c.max(1))) as f32)
            .collect()
    }
}

/// The scripted events.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// The fly-in, the volume following the camera or its origin pinned at the end pose's.
    FlyIn { pinned: bool },
    /// The lamp switched from the setting's state to the other.
    Toggle,
    /// A carve of `DIG_RADIUS` at the wall under the view's centre.
    Dig,
    /// The same carve at the hit nearest `DIG_FAR` voxels from the eye in the view's central
    /// half, seen from the steady flight's start facing its travel (`dig_camera`): down the
    /// passage.
    DigFar,
    /// A yaw of `degrees` onto the pose over `frames` frames.
    Yaw { frames: usize, degrees: u32 },
}

impl Event {
    pub fn name(self) -> String {
        match self {
            Event::FlyIn { pinned: false } => "fly-in".into(),
            Event::FlyIn { pinned: true } => "fly-in pinned".into(),
            Event::Toggle => "toggle".into(),
            Event::Dig => "dig".into(),
            Event::DigFar => format!("dig at {DIG_FAR}"),
            Event::Yaw { frames, degrees } => format!("yaw {degrees}deg {frames}f"),
        }
    }

    fn tag(self) -> String {
        self.name().replace(' ', "-")
    }

    pub fn dots(self) -> bool {
        matches!(self, Event::Dig | Event::DigFar | Event::Yaw { .. })
    }

    /// A carve, which `reset_world` undoes after the run.
    pub fn is_dig(self) -> bool {
        matches!(self, Event::Dig | Event::DigFar)
    }
}

/// The fly-in's start for `pose`.
pub fn fly_in_start(pose: &Pose) -> [f32; 3] {
    if pose.name == PATCH_POSE.name {
        PATCH_FLY_IN_START
    } else {
        VEIN_FLY_IN_START
    }
}

/// The steady flight's start and end for `pose`.
pub fn flight_path(pose: &Pose) -> ([f32; 3], [f32; 3]) {
    if pose.name == PATCH_POSE.name {
        PATCH_FLIGHT
    } else {
        VEIN_FLIGHT
    }
}

/// One part's settle.
#[derive(Clone, Debug)]
pub struct Settle {
    pub frames: Option<usize>,
    pub mean_90: Option<usize>,
    pub counted: usize,
    /// The share of counted tiles settled at each frame 0 ..= `UNTIL`.
    pub curve: Vec<f32>,
    /// The mean over counted tiles of each tile's standard deviation over the reference
    /// frames, log2: the settled noise the `log2 1.1` threshold sits against.
    pub ref_noise: f32,
    /// The mean over counted tiles of tile minus reference at frame 0, log2: the event's step.
    pub step0: f32,
}

impl Settle {
    /// The settled share in % every 8 frames to 120, then its lowest in each 128 frames from
    /// 128 to `UNTIL`.
    pub fn curve_line(&self) -> String {
        let early: Vec<String> = (0..=120)
            .step_by(8)
            .map(|f| format!("{:.0}", self.curve[f] * 100.0))
            .collect();
        let late: Vec<String> = (128..UNTIL)
            .step_by(128)
            .map(|f| {
                let m = self.curve[f..(f + 128).min(UNTIL + 1)]
                    .iter()
                    .copied()
                    .fold(1.0f32, f32::min);
                format!("{:.0}", m * 100.0)
            })
            .collect();
        format!("[{}] min/128 [{}]", early.join(" "), late.join(" "))
    }

    pub fn cell(&self) -> String {
        format!(
            "S {} mean90 {} (step {:+.3} noise {:.4})",
            self.frames.map_or("none".into(), |s| s.to_string()),
            self.mean_90.map_or("none".into(), |s| s.to_string()),
            self.step0,
            self.ref_noise
        )
    }
}

/// The dots of one run.
#[derive(Clone, Debug, Default)]
pub struct Dots {
    /// The display's dots in frames 1 ..= `DOT_FRAMES` and the floor's mean.
    pub counts: Vec<usize>,
    pub floor: f64,
    /// The standard deviation of the display's dot count over the floor's frames.
    pub floor_sd: f64,
    /// By part, summed over the dot frames, and the floor's means: emissive, lamp, filtered,
    /// gather, composite only, upscaler, joint (two or more lighting parts).
    pub by_part: [usize; 7],
    pub by_part_floor: [f64; 7],
    /// By age bucket: 0, 1-3, 4-15, 16-63, 64+.
    pub by_age: [usize; 5],
    pub by_age_floor: [f64; 5],
    /// Of age-0 lighting pixels in the dot frames, the share whose hit was in view the frame
    /// before (the rig's warp accepts it).
    pub reject_share: f64,
    pub age0: usize,
}

pub const BY_PART: [&str; 7] = [
    "emissive",
    "lamp",
    "filtered",
    "gather",
    "composite-only",
    "upscaler",
    "joint",
];
pub const BY_AGE: [&str; 5] = ["0", "1-3", "4-15", "16-63", "64+"];

fn age_bucket(a: u32) -> usize {
    match a {
        0 => 0,
        1..=3 => 1,
        4..=15 => 2,
        16..=63 => 3,
        _ => 4,
    }
}

impl Dots {
    pub fn excess_sum(&self) -> f64 {
        self.counts.iter().map(|&c| c as f64 - self.floor).sum()
    }

    pub fn excess_at(&self, f: usize) -> f64 {
        self.counts[f - 1] as f64 - self.floor
    }

    pub fn line(&self) -> String {
        let n = self.counts.len() as f64;
        let part: Vec<String> = BY_PART
            .iter()
            .enumerate()
            .map(|(i, name)| {
                format!(
                    "{name} {:.0}",
                    self.by_part[i] as f64 - n * self.by_part_floor[i]
                )
            })
            .collect();
        let age: Vec<String> = BY_AGE
            .iter()
            .enumerate()
            .map(|(i, name)| {
                format!(
                    "{name} {:.0}",
                    self.by_age[i] as f64 - n * self.by_age_floor[i]
                )
            })
            .collect();
        format!(
            "excess sum 1-{DOT_FRAMES} {:.0} (floor {:.1}/frame, sd {:.1}), at 1/8/16 {:.0}/{:.0}/{:.0}; by part [{}]; by age [{}]; reject share {:.3} of {} age-0 pixels",
            self.excess_sum(),
            self.floor,
            self.floor_sd,
            self.excess_at(1),
            self.excess_at(8),
            self.excess_at(16),
            part.join(", "),
            age.join(", "),
            self.reject_share,
            self.age0,
        )
    }
}

/// A settle run's reading.
pub struct SettleReading {
    pub settle: Vec<(Part, Settle)>,
    pub exposure: Settle,
    /// The probe volume's light-change window per frame 1 ..= 64 after the event.
    pub window: Vec<bool>,
    pub dots: Option<Dots>,
    /// Display images at `STRIP_FRAMES` for the Auto strip beside the pinned run's, and the
    /// exposure at them.
    pub strip: Vec<Vec<u8>>,
    pub strip_exposure: Vec<f32>,
    /// Each part's tile series, frames 0 ..= `REF.1`, the meter's full tiles at `REF.1`, and
    /// the exposure's log2 per frame.
    pub series: Vec<Vec<Vec<f32>>>,
    pub last_full: Vec<bool>,
    pub exposure_log2: Vec<f32>,
}

/// The settle filmstrips' frames after the event.
pub const STRIP_FRAMES: [usize; 14] = [0, 1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 1536, 2048];

/// The settle of a series of tile vectors (frames 0 ..= `REF.1`), counted where `counted`.
fn settle_of(series: &[Vec<f32>], counted: &[bool]) -> Settle {
    let tiles = counted.len();
    let reference: Vec<f32> = (0..tiles)
        .map(|t| (REF.0..=REF.1).map(|f| series[f][t]).sum::<f32>() / (REF.1 - REF.0 + 1) as f32)
        .collect();
    let idx: Vec<usize> = (0..tiles)
        .filter(|&t| counted[t] && reference[t] > FULL_LOG2)
        .collect();
    let share = |f: usize| {
        idx.iter()
            .filter(|&&t| (series[f][t] - reference[t]).abs() <= 1.1f32.log2())
            .count() as f32
            / idx.len().max(1) as f32
    };
    let shares: Vec<f32> = (0..=UNTIL).map(share).collect();
    let frames = if shares[UNTIL] < 0.9 {
        None
    } else {
        Some((0..=UNTIL).rev().find(|&f| shares[f] < 0.9).unwrap_or(0))
    };
    let mean = |f: usize| {
        idx.iter()
            .map(|&t| f64::from(series[f][t].exp2()))
            .sum::<f64>()
            / idx.len().max(1) as f64
    };
    let mref = idx
        .iter()
        .map(|&t| f64::from(reference[t].exp2()))
        .sum::<f64>()
        / idx.len().max(1) as f64;
    let change = (mean(0) - mref).abs();
    let mean_90 = if change <= 1e-4 * mref {
        Some(0)
    } else {
        let bad: Vec<usize> = (0..=UNTIL)
            .filter(|&f| (mean(f) - mref).abs() > 0.1 * change)
            .collect();
        match bad.last() {
            Some(&UNTIL) => None,
            Some(&f) => Some(f + 1),
            None => Some(0),
        }
    };
    let nref = (REF.1 - REF.0 + 1) as f32;
    let k = idx.len().max(1) as f32;
    let ref_noise = idx
        .iter()
        .map(|&t| {
            ((REF.0..=REF.1)
                .map(|f| (series[f][t] - reference[t]).powi(2))
                .sum::<f32>()
                / nref)
                .sqrt()
        })
        .sum::<f32>()
        / k;
    let step0 = idx
        .iter()
        .map(|&t| series[0][t] - reference[t])
        .sum::<f32>()
        / k;
    Settle {
        frames,
        mean_90,
        counted: idx.len(),
        curve: shares,
        ref_noise,
        step0,
    }
}

/// The 5 x 5 median rule's outliers of `v` over a `w` x `h` grid: over the median by more than
/// `abs` and more than twice it.
fn outliers(v: &[f32], w: u32, h: u32, abs: f32) -> Vec<bool> {
    let (w, h) = (w as i32, h as i32);
    let mut out = vec![false; v.len()];
    let mut nb = Vec::with_capacity(25);
    for y in 0..h {
        for x in 0..w {
            nb.clear();
            for dy in -2..=2 {
                for dx in -2..=2 {
                    let (sx, sy) = ((x + dx).clamp(0, w - 1), (y + dy).clamp(0, h - 1));
                    nb.push(v[(sy * w + sx) as usize]);
                }
            }
            let (_, m, _) = nb.select_nth_unstable_by(12, f32::total_cmp);
            let m = *m;
            let l = v[(y * w + x) as usize];
            out[(y * w + x) as usize] = l > m + abs && l > 2.0 * m;
        }
    }
    out
}

/// Whether any of the 3 x 3 pixels around `(x, y)` of a `w` x `h` mask is set.
fn any_near(mask: &[bool], w: u32, h: u32, x: u32, y: u32) -> bool {
    let (x0, x1) = (x.saturating_sub(1), (x + 1).min(w - 1));
    let (y0, y1) = (y.saturating_sub(1), (y + 1).min(h - 1));
    (y0..=y1).any(|yy| (x0..=x1).any(|xx| mask[(yy * w + xx) as usize]))
}

/// One frame's dots: the display's dot pixels, by part and by age.
fn frame_dots(px: &Pixels) -> (Vec<usize>, [usize; 7], [usize; 5]) {
    let [ow, oh] = px.display_size;
    let luma: Vec<f32> = px.display.chunks_exact(4).map(byte_luma).collect();
    let dots: Vec<usize> = outliers(&luma, ow, oh, 24.0)
        .iter()
        .enumerate()
        .filter_map(|(i, &d)| d.then_some(i))
        .collect();
    let [gw, gh] = px.grid;
    let masks: Vec<Vec<bool>> = LIGHTING_PARTS
        .iter()
        .map(|&p| outliers(&px.values(p, true).1, gw, gh, 24.0 / 255.0))
        .collect();
    let comp: Vec<f32> = px
        .composite
        .iter()
        .map(|c| 255.0 * (0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]))
        .collect();
    let comp_mask = outliers(&comp, SIZE[0], SIZE[1], 24.0);
    let mut by_part = [0usize; 7];
    let mut by_age = [0usize; 5];
    for &i in &dots {
        let (x, y) = (i as u32 % ow, i as u32 / ow);
        let (ix, iy) = px.internal_of(px.display_size, x, y);
        let l = px.lighting_of(ix, iy);
        let (lx, ly) = (l as u32 % gw, l as u32 / gw);
        let mut parts = 0;
        for (k, m) in masks.iter().enumerate() {
            if any_near(m, gw, gh, lx, ly) {
                by_part[k] += 1;
                parts += 1;
            }
        }
        if parts == 0 {
            if any_near(&comp_mask, SIZE[0], SIZE[1], ix, iy) {
                by_part[4] += 1;
            } else {
                by_part[5] += 1;
            }
        } else if parts >= 2 {
            by_part[6] += 1;
        }
        by_age[age_bucket(px.age(l))] += 1;
    }
    (dots, by_part, by_age)
}

/// Decodes a hit record's octahedral normal.
fn oct_normal(r: [u16; 4]) -> Vec3 {
    let s = |b: u16| (f32::from(b as i16) / 32767.0).max(-1.0);
    let (u, v) = (s(r[0]), s(r[1]));
    let mut n = Vec3::new(u, v, 1.0 - u.abs() - v.abs());
    if n.z < 0.0 {
        let (x, y) = (n.x, n.y);
        n.x = (1.0 - y.abs()) * x.signum();
        n.y = (1.0 - x.abs()) * y.signum();
    }
    n.normalize_or_zero()
}

/// The grid pixel of a `grid`-sized view of `camera` that world point `p` lands in.
fn project(camera: &Camera, grid: [u32; 2], p: Vec3) -> Option<(u32, u32)> {
    let (f, r, u) = camera.basis();
    let d = p - camera.position;
    let z = d.dot(f);
    if z <= 1e-3 {
        return None;
    }
    let t = (camera.vfov * 0.5).tan();
    let a = grid[0] as f32 / grid[1] as f32;
    let x = (d.dot(r) / z + t * a) / (2.0 * t * a) * grid[0] as f32;
    let y = (t - d.dot(u) / z) / (2.0 * t) * grid[1] as f32;
    (x >= 0.0 && y >= 0.0 && x < grid[0] as f32 && y < grid[1] as f32)
        .then_some((x as u32, y as u32))
}

fn hit(px: &Pixels, ix: u32, iy: u32) -> Option<(Vec3, [u16; 4])> {
    let i = (iy * SIZE[0] + ix) as usize;
    let info = px.hit_info[i];
    (info[3] & 1 == 1).then(|| (Vec3::from_slice(&px.hit_pos[i][..3]), info))
}

/// The warp of `grid`'s pixel `(x, y)` of frame `cur` into frame `prev`: the prev grid pixel
/// it lands in when accepted; `Some(None)` for a hit not accepted, `None` for a miss.
fn warp(cur: &Pixels, prev: &Pixels, grid: [u32; 2], x: u32, y: u32) -> Option<Option<usize>> {
    let (ix, iy) = cur.internal_of(grid, x, y);
    let (p, info) = hit(cur, ix, iy)?;
    let Some((qx, qy)) = project(&prev.camera, grid, p) else {
        return Some(None);
    };
    let (jx, jy) = prev.internal_of(grid, qx, qy);
    let Some((q, qinfo)) = hit(prev, jx, jy) else {
        return Some(None);
    };
    let face = |r: [u16; 4]| (r[3] >> 3) & 7;
    let ok = (p - q).length() <= WARP_DIST
        && face(info) == face(qinfo)
        && oct_normal(info).dot(oct_normal(qinfo)) >= WARP_COS;
    Some(ok.then_some((qy * grid[0] + qx) as usize))
}

/// The lighting grid's warp shares of `cur` from `prev`: (accepted, hit, age-0 accepted,
/// age-0 hit).
fn warp_shares(cur: &Pixels, prev: &Pixels) -> [u64; 4] {
    let mut s = [0u64; 4];
    let g = cur.grid;
    for y in 0..g[1] {
        for x in 0..g[0] {
            let Some(w) = warp(cur, prev, g, x, y) else {
                continue;
            };
            let young = cur.age((y * g[0] + x) as usize) == 0;
            s[1] += 1;
            s[0] += u64::from(w.is_some());
            if young {
                s[3] += 1;
                s[2] += u64::from(w.is_some());
            }
        }
    }
    s
}

/// Renders `frames` frames of `camera` (the frame index from 0), returning nothing.
fn render(level: &VeinLevel, frame: &mut Frame, camera: &Camera, frames: usize) {
    let light = LightParams::headlamp();
    for _ in 0..frames {
        frame.step(&level.device, &level.queue, &level.gpu, camera, &light);
    }
}

/// Regenerates the level's world and its mirror after a dig.
pub fn reset_world(level: &mut VeinLevel) {
    let source = flythrough::cave::CaveSource::new(
        1,
        ChunkExtent::new([0, 0, 0], flythrough::cave::LEVEL_CHUNKS),
    );
    level.world = World::generate(
        source.extent,
        &source,
        &flythrough::with_emissive_ore(&flythrough::cave_materials()),
    );
    let cfg = vxe_render::MirrorConfig {
        budget_bytes: 256 << 20,
        ..vxe_render::MirrorConfig::new(vxe_render::Window::Fixed(level.world.extent()))
    };
    level.gpu = vxe_render::GpuWorld::with_config(&level.device, cfg);
    level.gpu.set_focus(source.spawn());
    for _ in 0..4 {
        level
            .gpu
            .sync_now(&level.world, &level.device, &level.queue)
            .expect("mirror fits the adapter");
    }
}

/// The still camera of a run: the pose's, or for `Event::DigFar` the steady flight's start
/// facing its travel, which looks down the passage.
pub fn dig_camera(pose: &Pose, event: Event) -> Camera {
    if event == Event::DigFar {
        KRun::Path.camera(pose, 0)
    } else {
        pose.camera()
    }
}

/// The voxel a dig carves around: under the view's centre, or `far` the hit whose distance
/// from the eye is nearest `DIG_FAR`, with that distance.
fn dig_target(px: &Pixels, far: bool) -> ([i32; 3], f32) {
    let eye = px.camera.position;
    let p = if far {
        // The view's central half on each axis, so the hole lies in view.
        (SIZE[1] / 4..3 * SIZE[1] / 4)
            .flat_map(|y| (SIZE[0] / 4..3 * SIZE[0] / 4).map(move |x| (x, y)))
            .filter_map(|(x, y)| hit(px, x, y).map(|h| h.0))
            .min_by(|a, b| {
                let d = |p: &Vec3| ((*p - eye).length() - DIG_FAR as f32).abs();
                d(a).total_cmp(&d(b))
            })
            .expect("the view hits rock")
    } else {
        hit(px, SIZE[0] / 2, SIZE[1] / 2)
            .expect("the view's centre hits the wall")
            .0
    };
    (
        [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32],
        (p - eye).length(),
    )
}

/// Carves `DIG_RADIUS` at `dig_target` of `px`, syncs the mirror and invalidates the frame's
/// history over the changed box; the box.
fn dig(level: &mut VeinLevel, frame: &mut Frame, px: &Pixels, far: bool) -> ([i32; 3], [i32; 3]) {
    let (v, dist) = dig_target(px, far);
    let r = level
        .world
        .apply(&EditOp::sphere(
            centre_of_voxel(v),
            DIG_RADIUS * FIXED_ONE,
            Mode::Carve,
        ))
        .expect("the carve applies");
    let (min, max) = r.aabb.expect("the carve changes voxels");
    level
        .gpu
        .sync_now(&level.world, &level.device, &level.queue)
        .expect("mirror fits");
    frame.invalidate(min.0, max.0);
    println!(
        "dig at {v:?}, {dist:.1} voxels from the eye: box {:?} to {:?}, {} voxels changed",
        min.0,
        max.0,
        r.changed_from.iter().map(|c| c.1).sum::<u32>()
    );
    (min.0, max.0)
}
