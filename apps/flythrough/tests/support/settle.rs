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
//! With `STILL_FILMSTRIP` set to a tag, the runs write contact sheets under the worktree's
//! `live/filmstrip/<tag>/`.

use std::collections::HashMap;

use super::stillness::{FULL_LOG2, PATCH_POSE, Pose, SIZE, TILE, VEIN_POSE, VeinLevel, meter_log2};
use super::*;
use flythrough::LookPreset;
use vxe_render::exposure::ExposureState;
use vxe_render::glam::Vec3;
use vxe_render::{
    AutoExposure, Exposure, Frame, Fsr2Settings, LightParams, Lighting, LookParams, UpscalerKind,
    readback,
};

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
/// The strip tool's flight (`tests/strip.rs`): voxels a second along the view, its yaw rate in
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
/// `marcher.rs`'s `HALF_OFFSET`: the internal pixel of each 2 x 2 the half-resolution
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
/// Plan 0049's tables were read at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Setting {
    pub lamp: bool,
    pub indirect: bool,
    pub auto: bool,
    pub upscaler: bool,
    pub bridge: bool,
    pub steady: Steady,
}

/// The probe knobs a run reads (the steady spike's A1, A2 and C), and whether the temporal pass
/// reads the emissive table (ADR-0043; off, every hit takes the fallback).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Steady {
    /// A1: the probes' freeze count.
    pub a1: Option<u32>,
    /// A2: inside probes skipped.
    pub a2: bool,
    /// C: the lamp out of the probes.
    pub c: bool,
    /// The table's read (on by default).
    pub table: bool,
}

impl Default for Steady {
    fn default() -> Self {
        Self {
            a1: None,
            a2: false,
            c: false,
            table: true,
        }
    }
}

impl Steady {
    /// `table` (the default; `off` and `b` name it too, as the readings before ADR-0043 did),
    /// `noread` (the table's read off), `a` (A1 and A2), `c`, `ac`, `a1`, `a2`, and `aN` with a
    /// freeze of `N`.
    pub fn named(name: &str) -> Steady {
        let a1 = Some(vxe_render::marcher::DEFAULT_PROBE_FREEZE);
        let all = Steady {
            a1,
            a2: true,
            ..Steady::default()
        };
        match name {
            "table" | "off" | "b" => Steady::default(),
            "noread" => Steady {
                table: false,
                ..Steady::default()
            },
            "a" => all,
            "ac" => Steady { c: true, ..all },
            "c" => Steady {
                c: true,
                ..Steady::default()
            },
            "a1" => Steady {
                a1,
                ..Steady::default()
            },
            "a2" => Steady {
                a2: true,
                ..Steady::default()
            },
            other => {
                let n = other
                    .strip_prefix('a')
                    .and_then(|n| n.parse().ok())
                    .unwrap_or_else(|| panic!("unknown steady preset {other}"));
                Steady {
                    a1: Some(n),
                    ..Steady::default()
                }
            }
        }
    }

    pub fn apply(self, lighting: Lighting) -> Lighting {
        Lighting {
            probe_freeze: self.a1,
            skip_inside_probes: self.a2,
            probe_lamp: !self.c,
            table_read: self.table,
            ..lighting
        }
    }

    /// The name for a run's label, empty at the default.
    pub fn words(self) -> String {
        if self == Steady::default() {
            return String::new();
        }
        let mut w = Vec::new();
        if let Some(f) = self.a1 {
            w.push(format!("a1={f}"));
        }
        if self.a2 {
            w.push("a2".to_owned());
        }
        if self.c {
            w.push("c".to_owned());
        }
        if !self.table {
            w.push("noread".to_owned());
        }
        format!(" steady {}", w.join("+"))
    }
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
            self.steady.words()
        )
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
            self.steady.words().replace(' ', "-")
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
        f.set_upscaler_kind(
            device,
            UpscalerKind::Own(vxe_render::marcher::Upscaler::default()),
        );
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
    setting.steady.apply(Lighting {
        lamp: setting.lamp,
        indirect: setting.indirect,
        exposure: if setting.auto {
            Exposure::Auto(AutoExposure::eye())
        } else {
            Exposure::Manual(pose.exposure)
        },
        ..Lighting::default()
    })
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
    let b = readback::read_buffer(device, queue, frame.exposure_tiles(), 0, 8 * n);
    let meter = b
        .chunks_exact(8)
        .map(|c| {
            let sum = f32::from_le_bytes(c[0..4].try_into().unwrap());
            let k = f32::from_le_bytes(c[4..8].try_into().unwrap());
            let m = sum / k.max(1.0);
            (m, k >= (TILE * TILE) as f32 && m > FULL_LOG2)
        })
        .collect();
    let st = readback::read_buffer(device, queue, frame.exposure_state(), 0, 32);
    let exposure = ExposureState::from_bytes(&st).previous;
    let acc = frame.accumulated();
    let grid = [acc.width(), acc.height()];
    let t = frame.target();
    Pixels {
        camera,
        exposure,
        meter,
        display: readback::read_rgba8(device, queue, t),
        display_size: [t.width(), t.height()],
        composite: if want.composite {
            read_composite(level, frame)
        } else {
            Vec::new()
        },
        grid,
        accum: readback::read_float(device, queue, acc),
        filtered: readback::read_float(device, queue, frame.filtered()),
        gather: readback::read_float(device, queue, frame.indirect_irradiance()),
        moments: if want.moments {
            readback::read_float(device, queue, frame.moments())
        } else {
            Vec::new()
        },
        hit_pos: if want.hits {
            readback::read_float(device, queue, frame.hit_positions())
        } else {
            Vec::new()
        },
        hit_info: if want.hits {
            readback::read_rgba16u(device, queue, frame.hit_records())
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
        return readback::read_float(device, queue, fsr.colour_in())
            .into_iter()
            .map(|c| [c[0], c[1], c[2]].map(|v| aces(v).powf(1.0 / 2.2)))
            .collect();
    }
    let capture = frame.composite_capture().expect("captured");
    readback::read_rgba8(device, queue, capture)
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
    /// Whether the run read the table, and the dots by their pixel's cell (`dots_by_cell`),
    /// summed over the dot frames, and the floor's means: young (no cell, or under
    /// `YOUNG_COUNT` samples), mature. `no_key` counts the young dots with no face key.
    pub cell_split: bool,
    pub by_cell: [usize; 2],
    pub by_cell_floor: [f64; 2],
    pub no_key: usize,
}

/// The sample count under which the dots' split calls a cell young (the spike's `C_MIN`).
pub const YOUNG_COUNT: u32 = 16;

/// Of a frame's display dots `dots` (indices into the display), how many lie on a pixel whose
/// cell is young (no face key, no cell, or under `YOUNG_COUNT` samples) and how many on a
/// mature cell, and how many of the young have no face key. Each dot takes the internal
/// pixel under it (`Pixels::internal_of`) and that pixel's hit's face key.
pub fn dots_by_cell(px: &Pixels, dots: &[usize], words: &[u32]) -> ([usize; 2], usize) {
    let [ow, _] = px.display_size;
    let (mut split, mut no_key) = ([0usize; 2], 0usize);
    for &i in dots {
        let (x, y) = (i as u32 % ow, i as u32 / ow);
        let (ix, iy) = px.internal_of(px.display_size, x, y);
        let j = (iy * SIZE[0] + ix) as usize;
        match face_key(px.camera.position, px.hit_pos[j], px.hit_info[j]) {
            None => {
                split[0] += 1;
                no_key += 1;
            }
            Some(k) => split[usize::from(cache_count(words, k) >= YOUNG_COUNT)] += 1,
        }
    }
    (split, no_key)
}

/// A frame's display dots and their split by cell (`dots_by_cell`).
pub fn frame_dot_split(px: &Pixels, words: &[u32]) -> (usize, [usize; 2]) {
    let d = frame_dots(px).0;
    (d.len(), dots_by_cell(px, &d, words).0)
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

    /// The excess over the dot frames by cell: young, mature. They sum to `excess_sum`.
    pub fn cell_excess(&self) -> [f64; 2] {
        let n = self.counts.len() as f64;
        [0, 1].map(|k| self.by_cell[k] as f64 - n * self.by_cell_floor[k])
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
        ) + &if self.cell_split {
            let [y, m] = self.cell_excess();
            format!(
                "; by cell [young {y:.0}, mature {m:.0}] (young share of the excess {:.3}; {} young dots with no face key; floor young {:.1}, mature {:.1}/frame)",
                y / self.excess_sum(),
                self.no_key,
                self.by_cell_floor[0],
                self.by_cell_floor[1],
            )
        } else {
            "; by cell n/a (no table)".to_owned()
        }
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

fn camera_at(position: [f32; 3], yaw_deg: f32, pitch_deg: f32) -> Camera {
    Camera {
        position: position.into(),
        yaw: yaw_deg.to_radians(),
        pitch: pitch_deg.to_radians(),
        vfov: 70f32.to_radians(),
    }
}

/// A settle run: settle `SETTLE` frames, the event, then frames 1 ..= `REF.1` still, its
/// random numbers from frame index `seed` (seed k is `k * SEED_STRIDE`). Only seed 0 reads the
/// dots and writes the filmstrips.
pub fn settle_run(
    level: &mut VeinLevel,
    pose: &Pose,
    setting: Setting,
    event: Event,
    seed: u32,
    strip_tag: Option<&str>,
) -> SettleReading {
    let strip_tag = strip_tag.filter(|_| seed == 0);
    let mut frame = make_frame(level, pose, setting);
    frame.set_frame_index(seed);
    let light = LightParams::headlamp();
    let end = dig_camera(pose, event);
    if let Event::FlyIn { pinned: true } = event {
        let o = frame.probes().origin_for(pose.position);
        frame.probes_mut().set_origin(Some(o));
    }
    // Before the event: settle, then the motion that ends on frame 0.
    let want_none = Want {
        hits: false,
        composite: false,
        moments: false,
    };
    let mut prev: Option<Pixels> = None;
    let dots_on = event.dots() && seed == 0;
    let want_dots = Want {
        hits: true,
        composite: true,
        moments: true,
    };
    match event {
        Event::FlyIn { .. } => {
            let start = Vec3::from(fly_in_start(pose));
            let to = Vec3::from(pose.position);
            let start_cam = Camera {
                position: start,
                ..end
            };
            render(level, &mut frame, &start_cam, SETTLE);
            let steps = ((to - start).length() / (FLY_IN_SPEED / 60.0)).round() as usize;
            for i in 1..=steps {
                let c = Camera {
                    position: start.lerp(to, i as f32 / steps as f32),
                    ..end
                };
                frame.step(&level.device, &level.queue, &level.gpu, &c, &light);
            }
        }
        Event::Toggle | Event::Dig | Event::DigFar => render(level, &mut frame, &end, SETTLE),
        Event::Yaw { frames, degrees } => {
            let degrees = degrees as f32;
            let start = camera_at(pose.position, pose.yaw_deg - degrees, pose.pitch_deg);
            render(level, &mut frame, &start, SETTLE);
            for i in 1..=frames {
                let c = camera_at(
                    pose.position,
                    pose.yaw_deg - degrees + degrees * i as f32 / frames as f32,
                    pose.pitch_deg,
                );
                let last = i == frames;
                frame.step(&level.device, &level.queue, &level.gpu, &c, &light);
                if last && dots_on {
                    prev = Some(read(level, &frame, c, want_dots));
                }
            }
        }
    }
    let want0 = if dots_on || event.is_dig() {
        want_dots
    } else {
        Want {
            moments: true,
            ..want_none
        }
    };
    let f0 = read(level, &frame, end, want0);
    if prev.is_none() && dots_on {
        prev = Some(read(level, &frame, end, want_dots));
    }
    // The event.
    match event {
        Event::Toggle => frame.set_lighting(Lighting {
            lamp: !setting.lamp,
            ..lighting(pose, setting)
        }),
        Event::Dig | Event::DigFar => {
            dig(level, &mut frame, &f0, event == Event::DigFar);
        }
        _ => {}
    }
    let mut series: Vec<Vec<Vec<f32>>> = vec![Vec::new(); PARTS.len()];
    for (k, &p) in PARTS.iter().enumerate() {
        series[k].push(f0.tiles(p));
    }
    let mut exposure = vec![f0.exposure.log2()];
    let mut window = Vec::new();
    let mut dots = Dots::default();
    let (mut rej, mut age0) = (0u64, 0u64);
    let mut floor_n = 0usize;
    let mut floor_part = [0usize; 7];
    let mut floor_age = [0usize; 5];
    let mut floor_cell = [0usize; 2];
    let split_on = dots_on && setting.steady.table;
    let mut floor_count = 0usize;
    let mut floor_counts: Vec<f64> = Vec::new();
    let mut strip = Vec::new();
    let mut strip_exposure = Vec::new();
    let mut strip_gather = Vec::new();
    let mut strip_age = Vec::new();
    let mut strip_dots: Vec<Vec<usize>> = Vec::new();
    let mut last_full = Vec::new();
    let keep_strip = |f: usize| STRIP_FRAMES.contains(&f);
    if keep_strip(0) {
        strip.push(f0.display.clone());
        strip_exposure.push(f0.exposure);
        strip_gather.push(f0.tiles(Part::Gather));
        strip_age.push(f0.moments.iter().map(|m| m[2]).collect::<Vec<f32>>());
        strip_dots.push(Vec::new());
    }
    for f in 1..=REF.1 {
        frame.step(&level.device, &level.queue, &level.gpu, &end, &light);
        if f <= 64 {
            window.push(frame.probe_light_window());
        }
        let dot_frame = dots_on && (f <= DOT_FRAMES || f >= REF.0);
        let sampled = settle_sampled(f);
        if !(sampled || dot_frame || keep_strip(f)) {
            for s in series.iter_mut() {
                let last = s.last().expect("frame 0 read").clone();
                s.push(last);
            }
            let last = *exposure.last().expect("frame 0 read");
            exposure.push(last);
            continue;
        }
        let want = if dot_frame {
            want_dots
        } else {
            Want {
                moments: keep_strip(f),
                ..want_none
            }
        };
        let px = read(level, &frame, end, want);
        for (k, &p) in PARTS.iter().enumerate() {
            let v = if sampled {
                px.tiles(p)
            } else {
                series[k].last().expect("frame 0 read").clone()
            };
            series[k].push(v);
        }
        let e = if sampled {
            px.exposure.log2()
        } else {
            *exposure.last().expect("frame 0 read")
        };
        exposure.push(e);
        let mut fd = Vec::new();
        if dot_frame {
            let (d, bp, ba) = frame_dots(&px);
            let (bc, nk) = if split_on {
                dots_by_cell(&px, &d, &cache_words(level, &frame))
            } else {
                ([0; 2], 0)
            };
            if f <= DOT_FRAMES {
                dots.counts.push(d.len());
                dots.no_key += nk;
                for (a, b) in dots.by_cell.iter_mut().zip(bc) {
                    *a += b;
                }
                for (a, b) in dots.by_part.iter_mut().zip(bp) {
                    *a += b;
                }
                for (a, b) in dots.by_age.iter_mut().zip(ba) {
                    *a += b;
                }
                let s = warp_shares(&px, prev.as_ref().expect("previous frame read"));
                rej += s[2];
                age0 += s[3];
            } else {
                floor_n += 1;
                floor_count += d.len();
                floor_counts.push(d.len() as f64);
                for (a, b) in floor_cell.iter_mut().zip(bc) {
                    *a += b;
                }
                for (a, b) in floor_part.iter_mut().zip(bp) {
                    *a += b;
                }
                for (a, b) in floor_age.iter_mut().zip(ba) {
                    *a += b;
                }
            }
            fd = d;
        }
        if keep_strip(f) {
            strip.push(px.display.clone());
            strip_exposure.push(px.exposure);
            strip_gather.push(px.tiles(Part::Gather));
            strip_age.push(px.moments.iter().map(|m| m[2]).collect());
            strip_dots.push(fd);
        }
        if f == REF.1 {
            last_full = px.meter.iter().map(|m| m.1).collect();
        }
        if dot_frame {
            prev = Some(px);
        }
    }
    let settle = PARTS
        .iter()
        .enumerate()
        .map(|(k, &p)| (p, settle_of(&series[k], &last_full)))
        .collect();
    let exposure_series: Vec<Vec<f32>> = exposure.iter().map(|&e| vec![e]).collect();
    let exposure_settle = settle_of(&exposure_series, &[true]);
    let dots = dots_on.then(|| {
        let n = floor_n.max(1) as f64;
        dots.floor = floor_count as f64 / n;
        dots.floor_sd = sd(&floor_counts);
        dots.by_part_floor = floor_part.map(|c| c as f64 / n);
        dots.by_age_floor = floor_age.map(|c| c as f64 / n);
        dots.cell_split = split_on;
        dots.by_cell_floor = floor_cell.map(|c| c as f64 / n);
        dots.reject_share = rej as f64 / age0.max(1) as f64;
        dots.age0 = age0 as usize;
        dots
    });
    if let Some(tag) = strip_tag {
        let name = format!("{}-{}-{}", pose.name, event.tag(), setting.tag());
        let size = f0.display_size;
        settle_sheet(
            tag,
            &format!("{name}-display"),
            size,
            &strip,
            Some(&strip_dots),
        );
        grey_sheet(tag, &format!("{name}-gather"), &strip_gather, &last_full);
        age_sheet(tag, &format!("{name}-age"), f0.grid, &strip_age);
    }
    SettleReading {
        settle,
        exposure: exposure_settle,
        window,
        dots,
        strip,
        strip_exposure,
        series,
        last_full,
        exposure_log2: exposure,
    }
}

/// The settle's series reads every frame to `SETTLE_DENSE` after the event, then every
/// `SETTLE_SPARSE`-th frame and `REF.1`, holding the last read value between: past
/// `SETTLE_DENSE` a settle frame is late by at most `SETTLE_SPARSE - 1`.
pub const SETTLE_DENSE: usize = 256;
pub const SETTLE_SPARSE: usize = 4;

fn settle_sampled(f: usize) -> bool {
    f <= SETTLE_DENSE || f.is_multiple_of(SETTLE_SPARSE) || f == REF.1
}

/// Seed k's frame index: a multiple of the jitter's 16 phases, so every seed's frames take
/// the same jitter offsets.
pub const SEED_STRIDE: u32 = 1 << 20;

/// The step settle's seeds: the smallest power of two from 4 to 32 with
/// `3 sigma_view / sqrt(N) <= |D| / 10`, i.e. `N >= (30 sigma_view / |D|)^2`; `None` (under the
/// wander) when even 32 is short.
pub fn n_step(sigma_view: f32, d: f32) -> Option<u32> {
    let need = (30.0 * sigma_view / d.abs().max(1e-9)).powi(2);
    [4u32, 8, 16, 32].into_iter().find(|&n| n as f32 >= need)
}

/// A part's step settle on the seed-averaged view-mean `M(f)`, frames 0 ..= `REF.1`, log2.
#[derive(Clone, Debug)]
pub struct Step {
    pub m_pre: f32,
    pub m_post: f32,
    /// `M_post - M_pre`.
    pub d: f32,
    /// The first frame after which `M` stays within `0.1 |D|` of `M_post` up to `UNTIL`;
    /// `None` when frame `UNTIL` is outside.
    pub s90: Option<usize>,
    /// `M` every 8 frames from 0 to `UNTIL`.
    pub curve: Vec<f32>,
}

impl Step {
    pub fn of(m: &[f32]) -> Step {
        let m_pre = m[0];
        let m_post = m[REF.0..=REF.1].iter().sum::<f32>() / (REF.1 - REF.0 + 1) as f32;
        let d = m_post - m_pre;
        let tol = 0.1 * d.abs();
        let s90 = match (0..=UNTIL).rev().find(|&f| (m[f] - m_post).abs() > tol) {
            Some(UNTIL) => None,
            Some(f) => Some(f + 1),
            None => Some(0),
        };
        Step {
            m_pre,
            m_post,
            d,
            s90,
            curve: (0..=UNTIL).step_by(8).map(|f| m[f]).collect(),
        }
    }

    pub fn cell(&self) -> String {
        format!(
            "S90 {} D {:+.4} (pre {:.4} post {:.4})",
            self.s90.map_or("none".into(), |s| s.to_string()),
            self.d,
            self.m_pre,
            self.m_post
        )
    }

    pub fn curve_line(&self) -> String {
        let c: Vec<String> = self.curve.iter().map(|v| format!("{v:.3}")).collect();
        c.join(" ")
    }
}

/// The seeds' runs of one event and setting, summed: each part's tile series, and the
/// exposure's log2.
pub struct SeedSum {
    pub n: u32,
    sums: Vec<Vec<Vec<f64>>>,
    exposure: Vec<f64>,
    /// The counted tiles: full in the meter at frame `REF.1` of every seed.
    counted: Vec<bool>,
}

impl SeedSum {
    pub fn new() -> SeedSum {
        SeedSum {
            n: 0,
            sums: Vec::new(),
            exposure: Vec::new(),
            counted: Vec::new(),
        }
    }

    pub fn add(&mut self, r: &SettleReading) {
        if self.n == 0 {
            self.sums = r
                .series
                .iter()
                .map(|s| s.iter().map(|t| vec![0.0; t.len()]).collect())
                .collect();
            self.exposure = vec![0.0; r.exposure_log2.len()];
            self.counted = r.last_full.clone();
        }
        for (a, s) in self.sums.iter_mut().zip(&r.series) {
            for (a, t) in a.iter_mut().zip(s) {
                for (a, &v) in a.iter_mut().zip(t) {
                    *a += f64::from(v);
                }
            }
        }
        for (a, &e) in self.exposure.iter_mut().zip(&r.exposure_log2) {
            *a += f64::from(e);
        }
        for (c, &f) in self.counted.iter_mut().zip(&r.last_full) {
            *c &= f;
        }
        self.n += 1;
    }

    fn mean_series(&self, k: usize) -> Vec<Vec<f32>> {
        let n = f64::from(self.n.max(1));
        self.sums[k]
            .iter()
            .map(|t| t.iter().map(|&v| (v / n) as f32).collect())
            .collect()
    }

    /// Each part's step settle on the seeds' mean view-mean (the mean of the counted tiles
    /// whose seed-mean reference is above `FULL_LOG2`), and its per-tile settle on the seeds'
    /// mean tile series; then the exposure's step settle (log2).
    pub fn settles(&self) -> (Vec<(Part, Step, Settle)>, Step) {
        let parts = PARTS
            .iter()
            .enumerate()
            .map(|(k, &p)| {
                let series = self.mean_series(k);
                let tiles = self.counted.len();
                let reference: Vec<f32> = (0..tiles)
                    .map(|t| {
                        (REF.0..=REF.1).map(|f| series[f][t]).sum::<f32>()
                            / (REF.1 - REF.0 + 1) as f32
                    })
                    .collect();
                let idx: Vec<usize> = (0..tiles)
                    .filter(|&t| self.counted[t] && reference[t] > FULL_LOG2)
                    .collect();
                let m: Vec<f32> = series
                    .iter()
                    .map(|f| idx.iter().map(|&t| f[t]).sum::<f32>() / idx.len().max(1) as f32)
                    .collect();
                (p, Step::of(&m), settle_of(&series, &self.counted))
            })
            .collect();
        let n = f64::from(self.n.max(1));
        let e: Vec<f32> = self.exposure.iter().map(|&v| (v / n) as f32).collect();
        (parts, Step::of(&e))
    }
}

impl Default for SeedSum {
    fn default() -> Self {
        Self::new()
    }
}

/// A long series: settle `SETTLE` frames, the event (toggle or dig), then `frames` still
/// frames; every 128 the gather's, the composite's and the emissive part's mean tile log2
/// over the composite's full tiles.
pub fn long_run(
    level: &mut VeinLevel,
    pose: &Pose,
    setting: Setting,
    event: Event,
    frames: usize,
) -> Vec<(usize, f32, f32, f32)> {
    let mut frame = make_frame(level, pose, setting);
    let cam = pose.camera();
    render(level, &mut frame, &cam, SETTLE);
    match event {
        Event::Toggle => frame.set_lighting(Lighting {
            lamp: !setting.lamp,
            ..lighting(pose, setting)
        }),
        other => panic!("the long series takes the toggle, not {other:?}"),
    }
    let want = Want {
        hits: false,
        composite: false,
        moments: false,
    };
    let mut rows = Vec::new();
    for f in 1..=frames {
        render(level, &mut frame, &cam, 1);
        if f % 128 == 0 {
            let px = read(level, &frame, cam, want);
            let mean = |p: Part| {
                let t = px.tiles(p);
                let full: Vec<f32> = t
                    .iter()
                    .zip(&px.meter)
                    .filter(|(v, m)| m.1 && **v > FULL_LOG2)
                    .map(|(v, _)| *v)
                    .collect();
                full.iter().sum::<f32>() / full.len().max(1) as f32
            };
            rows.push((
                f,
                mean(Part::Gather),
                mean(Part::Composite),
                mean(Part::Emissive),
            ));
        }
    }
    rows
}

/// The wander's series after the toggle: `WANDER_FRAMES` still frames, read from
/// `WANDER_FROM + 1` in blocks of `WANDER_BLOCK` (one jitter cycle, `JITTER_PHASES`).
pub const WANDER_FRAMES: usize = 8192;
pub const WANDER_FROM: usize = 1024;
pub const WANDER_BLOCK: usize = 16;

/// One part's still-camera wander (run (vi)), log2: over the blocks, the standard deviation
/// and range of the view-mean (the mean of the counted tiles' block values), each tile's
/// standard deviation and range (their mean, and the standard deviation's 90th percentile),
/// and the tiles' standard deviation over single frames.
#[derive(Clone, Copy, Debug)]
pub struct Wander {
    pub tiles: usize,
    pub sigma_view: f32,
    pub range_view: f32,
    pub sigma_tile: f32,
    pub sigma_tile_p90: f32,
    pub range_tile: f32,
    pub sigma_tile_frames: f32,
}

impl Wander {
    pub fn line(&self) -> String {
        format!(
            "sigma_view {:.4} R_view {:.4} sigma_tile {:.4} p90 {:.4} R_tile {:.4} sigma_tile(frames) {:.4} ({} tiles)",
            self.sigma_view,
            self.range_view,
            self.sigma_tile,
            self.sigma_tile_p90,
            self.range_tile,
            self.sigma_tile_frames,
            self.tiles
        )
    }
}

/// Each part's wander, `None` where it has no counted tile.
pub type Wanders = Vec<(Part, Option<Wander>)>;

fn sd(v: &[f64]) -> f64 {
    let n = v.len().max(1) as f64;
    let m = v.iter().sum::<f64>() / n;
    (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / n).sqrt()
}

fn range(v: &[f64]) -> f64 {
    let (lo, hi) = v
        .iter()
        .fold((f64::MAX, f64::MIN), |(lo, hi), &x| (lo.min(x), hi.max(x)));
    hi - lo
}

/// Run (vi): settle `SETTLE` frames with the lamp in the other state, switch it into the
/// setting's state (the window opens), then `WANDER_FRAMES` still frames; each part's
/// [`Wander`] over frames `WANDER_FROM + 1 ..= WANDER_FRAMES`, and the frames a second of the
/// read frames (rendering and reading back). A tile counts where the meter's tile is full at
/// the first and the last read frame and the part's mean over the read frames is above
/// `FULL_LOG2`; a part with no counted tile is `None`.
pub fn wander_run(level: &VeinLevel, pose: &Pose, setting: Setting, seed: u32) -> (Wanders, f64) {
    let off = Setting {
        lamp: !setting.lamp,
        ..setting
    };
    let mut frame = make_frame(level, pose, off);
    frame.set_frame_index(seed);
    let cam = pose.camera();
    render(level, &mut frame, &cam, SETTLE);
    frame.set_lighting(lighting(pose, setting));
    render(level, &mut frame, &cam, WANDER_FROM);
    let want = Want {
        hits: false,
        composite: false,
        moments: false,
    };
    let [tw, th] = SIZE.map(|n| n.div_ceil(TILE));
    let nt = (tw * th) as usize;
    let blocks = (WANDER_FRAMES - WANDER_FROM) / WANDER_BLOCK;
    // Per part: block sums per tile, and per-tile sum and sum of squares over single frames.
    let mut block = vec![vec![vec![0.0f64; nt]; blocks]; PARTS.len()];
    let mut s1 = vec![vec![0.0f64; nt]; PARTS.len()];
    let mut s2 = vec![vec![0.0f64; nt]; PARTS.len()];
    let mut full_first = Vec::new();
    let mut full_last = Vec::new();
    #[allow(
        clippy::disallowed_methods,
        reason = "a reading test's frames a second, not simulation"
    )]
    let t0 = std::time::Instant::now();
    for i in 0..blocks * WANDER_BLOCK {
        render(level, &mut frame, &cam, 1);
        let px = read(level, &frame, cam, want);
        let full: Vec<bool> = px.meter.iter().map(|m| m.1).collect();
        if i == 0 {
            full_first = full.clone();
        }
        full_last = full;
        for (k, &p) in PARTS.iter().enumerate() {
            for (t, v) in px.tiles(p).into_iter().enumerate() {
                let v = f64::from(v);
                block[k][i / WANDER_BLOCK][t] += v / WANDER_BLOCK as f64;
                s1[k][t] += v;
                s2[k][t] += v * v;
            }
        }
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a reading test's frames a second, not simulation"
    )]
    let fps = (blocks * WANDER_BLOCK) as f64 / t0.elapsed().as_secs_f64();
    let n = (blocks * WANDER_BLOCK) as f64;
    let out = PARTS
        .iter()
        .enumerate()
        .map(|(k, &p)| {
            let idx: Vec<usize> = (0..nt)
                .filter(|&t| full_first[t] && full_last[t] && s1[k][t] / n > f64::from(FULL_LOG2))
                .collect();
            if idx.is_empty() {
                return (p, None);
            }
            let view: Vec<f64> = (0..blocks)
                .map(|b| idx.iter().map(|&t| block[k][b][t]).sum::<f64>() / idx.len() as f64)
                .collect();
            let per: Vec<(f64, f64)> = idx
                .iter()
                .map(|&t| {
                    let x: Vec<f64> = (0..blocks).map(|b| block[k][b][t]).collect();
                    (sd(&x), range(&x))
                })
                .collect();
            let mut sds: Vec<f32> = per.iter().map(|&(s, _)| s as f32).collect();
            let k_t = idx.len() as f64;
            let frames_sd = idx
                .iter()
                .map(|&t| {
                    let m = s1[k][t] / n;
                    (s2[k][t] / n - m * m).max(0.0).sqrt()
                })
                .sum::<f64>()
                / k_t;
            let w = Wander {
                tiles: idx.len(),
                sigma_view: sd(&view) as f32,
                range_view: range(&view) as f32,
                sigma_tile: (per.iter().map(|p| p.0).sum::<f64>() / k_t) as f32,
                sigma_tile_p90: {
                    sds.sort_by(f32::total_cmp);
                    sds[((sds.len() - 1) as f32 * 0.9).round() as usize]
                },
                range_tile: (per.iter().map(|p| p.1).sum::<f64>() / k_t) as f32,
                sigma_tile_frames: frames_sd as f32,
            };
            (p, Some(w))
        })
        .collect();
    (out, fps)
}

/// The worktree's `live/filmstrip/<tag>/` directory.
pub fn strip_dir(tag: &str) -> std::path::PathBuf {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = root.canonicalize().unwrap_or(root);
    let dir = root.join("live/filmstrip").join(tag);
    std::fs::create_dir_all(&dir).expect("create the filmstrip directory");
    dir
}

const GAP: u32 = 2;
const DOT_MARK: [u8; 4] = [255, 0, 255, 255];

/// A sheet of `rows` of images, each row a list of (`w`, `h`, rgba) panels, laid out left to
/// right with `GAP` between.
fn sheet(rows: &[Vec<(u32, u32, Vec<u8>)>]) -> (Vec<u8>, u32, u32) {
    let sw = rows
        .iter()
        .map(|r| r.iter().map(|p| p.0 + GAP).sum::<u32>())
        .max()
        .unwrap_or(1);
    let sh = rows
        .iter()
        .map(|r| r.iter().map(|p| p.1).max().unwrap_or(0) + GAP)
        .sum::<u32>()
        .max(1);
    let mut out = vec![0u8; (sw * sh * 4) as usize];
    let mut oy = 0;
    for r in rows {
        let mut ox = 0;
        for (w, h, px) in r {
            for y in 0..*h {
                let s = ((y * w) * 4) as usize;
                let d = (((oy + y) * sw + ox) * 4) as usize;
                out[d..d + (*w * 4) as usize].copy_from_slice(&px[s..s + (*w * 4) as usize]);
            }
            ox += w + GAP;
        }
        oy += r.iter().map(|p| p.1).max().unwrap_or(0) + GAP;
    }
    (out, sw, sh)
}

fn write_sheet(tag: &str, name: &str, rows: &[Vec<(u32, u32, Vec<u8>)>], what: &str) {
    let (px, w, h) = sheet(rows);
    let path = strip_dir(tag).join(format!("{name}.png"));
    write_png(&path, &px, w, h);
    println!("filmstrip: {} ({what})", path.display());
}

/// The display images at `STRIP_FRAMES`, 7 to a row, dots marked.
fn settle_sheet(
    tag: &str,
    name: &str,
    size: [u32; 2],
    images: &[Vec<u8>],
    dots: Option<&[Vec<usize>]>,
) {
    let panels: Vec<(u32, u32, Vec<u8>)> = images
        .iter()
        .enumerate()
        .map(|(k, im)| {
            let mut im = im.clone();
            if let Some(d) = dots {
                for &i in &d[k] {
                    im[i * 4..i * 4 + 4].copy_from_slice(&DOT_MARK);
                }
            }
            (size[0], size[1], im)
        })
        .collect();
    let rows: Vec<_> = panels.chunks(7).map(<[_]>::to_vec).collect();
    write_sheet(
        tag,
        name,
        &rows,
        &format!("display after the event at frames {STRIP_FRAMES:?}, row by row, dots magenta"),
    );
}

/// Tiles as grey from the sheet's lowest counted mean to its highest.
fn grey_sheet(tag: &str, name: &str, tiles: &[Vec<f32>], counted: &[bool]) {
    let [tw, _] = SIZE.map(|n| n.div_ceil(TILE));
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for t in tiles {
        for (i, &m) in t.iter().enumerate() {
            if counted[i] && m > FULL_LOG2 {
                lo = lo.min(m);
                hi = hi.max(m);
            }
        }
    }
    let panels: Vec<(u32, u32, Vec<u8>)> = tiles
        .iter()
        .map(|t| {
            let mut im = vec![0u8; (SIZE[0] * SIZE[1] * 4) as usize];
            for y in 0..SIZE[1] {
                for x in 0..SIZE[0] {
                    let k = ((y / TILE) * tw + x / TILE) as usize;
                    let g = if counted[k] {
                        ((t[k] - lo) / (hi - lo).max(1e-6) * 255.0).clamp(0.0, 255.0) as u8
                    } else {
                        0
                    };
                    let i = ((y * SIZE[0] + x) * 4) as usize;
                    im[i..i + 4].copy_from_slice(&[g, g, g, 255]);
                }
            }
            (SIZE[0], SIZE[1], im)
        })
        .collect();
    let rows: Vec<_> = panels.chunks(7).map(<[_]>::to_vec).collect();
    write_sheet(
        tag,
        name,
        &rows,
        &format!("gather tile means grey from {lo:.3} to {hi:.3} log2, frames {STRIP_FRAMES:?}"),
    );
}

/// Pixel ages as grey, black at 0 to white at 64 and over.
fn age_sheet(tag: &str, name: &str, grid: [u32; 2], ages: &[Vec<f32>]) {
    let panels: Vec<(u32, u32, Vec<u8>)> = ages
        .iter()
        .map(|a| {
            let im = a
                .iter()
                .flat_map(|&v| {
                    let g = (v.min(64.0) / 64.0 * 255.0) as u8;
                    [g, g, g, 255]
                })
                .collect();
            (grid[0], grid[1], im)
        })
        .collect();
    let rows: Vec<_> = panels.chunks(7).map(<[_]>::to_vec).collect();
    write_sheet(
        tag,
        name,
        &rows,
        &format!("lighting-grid age, black 0 to white 64+, frames {STRIP_FRAMES:?}"),
    );
}

/// The Auto run's display row beneath the pinned run's.
pub fn auto_sheet(tag: &str, name: &str, size: [u32; 2], pinned: &[Vec<u8>], auto: &[Vec<u8>]) {
    let row = |im: &[Vec<u8>]| -> Vec<(u32, u32, Vec<u8>)> {
        im.iter().map(|i| (size[0], size[1], i.clone())).collect()
    };
    write_sheet(
        tag,
        name,
        &[row(pinned), row(auto)],
        &format!("pinned exposure row over the Auto row, frames {STRIP_FRAMES:?}"),
    );
}

/// The pose list.
pub const POSES: [Pose; 2] = [PATCH_POSE, VEIN_POSE];

/// A hit's cube face: its voxel `floor(p - 0.5 nf)` and entry face (2 axis + 1 toward +axis).
pub type FaceKey = ([i32; 3], u8);

/// The face key of a hit (position with t, record) seen from `eye`; `None` for a miss, a ray
/// that started in rock (no entry face), a field hit or a hit behind translucent layers. The entry face's axis is the record's
/// (flag bits 3..5, less 1), its outward side the one holding the eye: the shading normal can
/// lean away from the face under the far normal.
pub fn face_key(eye: Vec3, pos: [f32; 4], info: [u16; 4]) -> Option<FaceKey> {
    let flags = info[3];
    // A miss, a field-surface hit or a hit behind translucent layers has no cell.
    if flags & 1 == 0 || flags & 6 != 0 {
        return None;
    }
    let axis = i32::from((flags >> 3) & 3) - 1;
    if axis < 0 {
        return None;
    }
    let axis = axis as usize;
    let positive = eye.to_array()[axis] > pos[axis];
    let mut p = [pos[0], pos[1], pos[2]];
    p[axis] -= if positive { 0.5 } else { -0.5 };
    Some((
        p.map(|v| v.floor() as i32),
        (2 * axis + usize::from(positive)) as u8,
    ))
}

/// The world-keyed flicker over a run: frames go in as (key, mean luma), each key's mean luma
/// that frame. Reads of a key absent from the frame before count nothing.
#[derive(Default)]
pub struct KeyFlicker {
    prev: HashMap<FaceKey, f32>,
    samples: Vec<f32>,
    frame_keys: Vec<usize>,
}

impl KeyFlicker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a frame; `counted` false only updates the previous frame's means (a frame before
    /// the run's reading window).
    pub fn add(&mut self, keys: HashMap<FaceKey, f32>, counted: bool) {
        if counted {
            let mut n = 0;
            for (k, v) in &keys {
                if let Some(q) = self.prev.get(k) {
                    self.samples.push((meter_log2(*v) - meter_log2(*q)).abs());
                    n += 1;
                }
            }
            self.frame_keys.push(n);
        }
        self.prev = keys;
    }

    /// `K`: the mean over frames and keys.
    pub fn k(&self) -> f32 {
        (self.samples.iter().map(|&s| f64::from(s)).sum::<f64>() / self.samples.len().max(1) as f64)
            as f32
    }

    pub fn p95(&self) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let mut v = self.samples.clone();
        let k = ((v.len() as f64 * 0.95) as usize).min(v.len() - 1);
        *v.select_nth_unstable_by(k, f32::total_cmp).1
    }

    pub fn cell(&self) -> String {
        format!(
            "K {:.4} p95 {:.4} ({} key-frames, {:.0} keys a frame)",
            self.k(),
            self.p95(),
            self.samples.len(),
            self.samples.len() as f64 / self.frame_keys.len().max(1) as f64,
        )
    }
}

/// The frame's face keys with their pixels' mean luma as seen.
fn frame_keys(px: &Pixels) -> HashMap<FaceKey, f32> {
    let mut sums: HashMap<FaceKey, (f64, u32)> = HashMap::new();
    for (i, c) in px.composite.iter().enumerate() {
        if let Some(k) = face_key(px.camera.position, px.hit_pos[i], px.hit_info[i]) {
            let e = sums.entry(k).or_default();
            e.0 += f64::from(seen_lum(c));
            e.1 += 1;
        }
    }
    sums.into_iter()
        .map(|(k, (s, n))| (k, (s / f64::from(n)) as f32))
        .collect()
}

/// A run the flicker reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum KRun {
    /// The rig's fast yaw: 90 degrees onto the pose over 10 frames, then still.
    Yaw,
    /// `Flight::Path`: the steady flight, facing the travel.
    Path,
}

impl KRun {
    pub fn name(self) -> &'static str {
        match self {
            KRun::Yaw => "yaw 10f",
            KRun::Path => "flight",
        }
    }

    /// The camera at run frame `f` (0 the start, settled).
    pub fn camera(self, pose: &Pose, f: usize) -> Camera {
        match self {
            KRun::Yaw => camera_at(
                pose.position,
                pose.yaw_deg - 90.0 + 90.0 * (f.min(10) as f32) / 10.0,
                pose.pitch_deg,
            ),
            KRun::Path => {
                let (start, end) = flight_path(pose);
                let (start, end) = (Vec3::from(start), Vec3::from(end));
                let dir = (end - start).normalize();
                let len = (end - start).length();
                let s = (FLY_SPEED / 60.0 * f as f32).min(len);
                Camera {
                    position: start + dir * s,
                    yaw: dir.y.atan2(dir.x),
                    pitch: dir.z.asin(),
                    vfov: 70f32.to_radians(),
                }
            }
        }
    }
}

/// The flicker of a run: settle `SETTLE` frames at its start, then frames 1 .. `FLIGHT_FRAMES`;
/// `K` over frames `SHIMMER_FROM ..`, and over the frames before them (the yaw's own motion).
pub fn flicker_run(
    level: &VeinLevel,
    pose: &Pose,
    setting: Setting,
    run: KRun,
) -> (KeyFlicker, KeyFlicker) {
    let mut frame = make_frame(level, pose, setting);
    let light = LightParams::headlamp();
    render(level, &mut frame, &run.camera(pose, 0), SETTLE);
    let want = Want {
        hits: true,
        composite: true,
        moments: false,
    };
    let (mut late, mut early) = (KeyFlicker::new(), KeyFlicker::new());
    for f in 1..FLIGHT_FRAMES {
        let cam = run.camera(pose, f);
        frame.step(&level.device, &level.queue, &level.gpu, &cam, &light);
        let keys = frame_keys(&read(level, &frame, cam, want));
        early.add(keys.clone(), f < SHIMMER_FROM);
        late.add(keys, f >= SHIMMER_FROM);
    }
    (late, early)
}

/// The strip flight's dots: the display's count per flown frame, the floor's mean and spread
/// over `STRIP_FLOOR_FRAMES` still frames at the start, and the counts by part over the flight
/// and per floor frame.
pub struct StripDots {
    pub counts: Vec<usize>,
    pub floor: f64,
    pub floor_sd: f64,
    pub by_part: [usize; 7],
    pub by_part_floor: [f64; 7],
}

impl StripDots {
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
        let every: Vec<String> = self
            .counts
            .iter()
            .step_by(12)
            .map(|c| c.to_string())
            .collect();
        format!(
            "excess sum {:.0} (floor {:.1}/frame, sd {:.1}); by part [{}]; dots every 12 frames [{}]",
            self.counts
                .iter()
                .map(|&c| c as f64 - self.floor)
                .sum::<f64>(),
            self.floor,
            self.floor_sd,
            part.join(", "),
            every.join(" ")
        )
    }
}

/// The strip tool's flight (`tests/strip.rs`): along the view at `STRIP_SPEED` with the yaw at
/// `STRIP_YAW`, from a start settled `SETTLE` frames. The camera may enter rock, as the tool's
/// does.
pub fn strip_dots(level: &VeinLevel, pose: &Pose, setting: Setting) -> StripDots {
    let mut camera = pose.camera();
    const DT: f32 = 1.0 / 60.0;
    flown_dots(level, pose, setting, camera, STRIP_FRAMES_FLOWN, |_| {
        let (fwd, _, _) = camera.basis();
        camera.position += fwd * STRIP_SPEED * DT;
        camera.yaw += STRIP_YAW.to_radians() * DT;
        camera
    })
}

/// The same read along the rig's steady flight, whose path has air around every step.
pub fn path_dots(level: &VeinLevel, pose: &Pose, setting: Setting) -> StripDots {
    let start = KRun::Path.camera(pose, 0);
    flown_dots(level, pose, setting, start, FLIGHT_FRAMES - 1, |f| {
        KRun::Path.camera(pose, f + 1)
    })
}

fn flown_dots(
    level: &VeinLevel,
    pose: &Pose,
    setting: Setting,
    start: Camera,
    frames: usize,
    mut at: impl FnMut(usize) -> Camera,
) -> StripDots {
    let mut frame = make_frame(level, pose, setting);
    let camera = start;
    render(level, &mut frame, &camera, SETTLE);
    let want = Want {
        hits: false,
        composite: true,
        moments: true,
    };
    let mut floor = Vec::new();
    let mut part_floor = [0usize; 7];
    for _ in 0..STRIP_FLOOR_FRAMES {
        render(level, &mut frame, &camera, 1);
        let (d, bp, _) = frame_dots(&read(level, &frame, camera, want));
        floor.push(d.len() as f64);
        for (a, b) in part_floor.iter_mut().zip(bp) {
            *a += b;
        }
    }
    let mut out = StripDots {
        counts: Vec::new(),
        floor: floor.iter().sum::<f64>() / floor.len() as f64,
        floor_sd: sd(&floor),
        by_part: [0; 7],
        by_part_floor: part_floor.map(|c| c as f64 / STRIP_FLOOR_FRAMES as f64),
    };
    for f in 0..frames {
        let camera = at(f);
        render(level, &mut frame, &camera, 1);
        let (d, bp, _) = frame_dots(&read(level, &frame, camera, want));
        out.counts.push(d.len());
        for (a, b) in out.by_part.iter_mut().zip(bp) {
            *a += b;
        }
    }
    out
}

/// The level's spawn as a pose facing +x, as the strip tool has it, at `exposure`.
pub fn spawn_pose(exposure: f32) -> Pose {
    let source = flythrough::cave::CaveSource::new(
        1,
        ChunkExtent::new([0, 0, 0], flythrough::cave::LEVEL_CHUNKS),
    );
    Pose {
        name: "spawn",
        position: source.spawn().0.map(|v| v as f32 + 0.5),
        yaw_deg: 0.0,
        pitch_deg: 0.0,
        exposure,
    }
}

/// The eye's exposure at `pose` after `frames` still frames at `setting`, `Auto` whatever the
/// setting says.
pub fn eye_exposure(level: &VeinLevel, pose: &Pose, setting: Setting, frames: usize) -> f32 {
    let auto = Setting {
        auto: true,
        ..setting
    };
    let mut frame = make_frame(level, pose, auto);
    render(level, &mut frame, &pose.camera(), frames);
    let st = readback::read_buffer(&level.device, &level.queue, frame.exposure_state(), 0, 32);
    ExposureState::from_bytes(&st).previous
}

/// The eye's exposure at `pose` after 600 frames lamp on and 900 off, under `Auto`, at the
/// default look.
pub fn eye_after_lamp_off(level: &VeinLevel, pose: &Pose) -> f32 {
    let on = Setting {
        lamp: true,
        indirect: true,
        auto: true,
        upscaler: true,
        bridge: false,
        steady: Steady::default(),
    };
    let mut frame = make_frame(level, pose, on);
    render(level, &mut frame, &pose.camera(), 600);
    frame.set_lighting(lighting(pose, Setting { lamp: false, ..on }));
    render(level, &mut frame, &pose.camera(), 900);
    let st = readback::read_buffer(&level.device, &level.queue, frame.exposure_state(), 0, 32);
    ExposureState::from_bytes(&st).previous
}

/// The probe volume's state at rest: of the slots holding their probe (traced), how many are
/// inside rock.
pub fn probe_census(level: &VeinLevel, frame: &Frame) -> (u32, u32) {
    use vxe_render::probes::{MAX_AGE, PROBE_COUNT, STATE_BASE};
    let b = readback::read_buffer(
        &level.device,
        &level.queue,
        frame.probes().data(),
        u64::from(STATE_BASE) * 4,
        u64::from(PROBE_COUNT) * 4,
    );
    let (mut traced, mut inside) = (0, 0);
    for c in b.chunks_exact(4) {
        let s = u32::from_le_bytes(c.try_into().expect("a word"));
        if (s >> 1) & MAX_AGE > 0 {
            traced += 1;
            inside += s & 1;
        }
    }
    (traced, inside)
}

/// The volume's state against a freeze count: traced, inside rock, frozen (at the count or
/// over, not inside), and of the probes in `pose`'s view within `VIEW_REACH` voxels the same
/// three.
pub struct FreezeCensus {
    pub all: [u32; 3],
    pub in_view: [u32; 3],
}

/// How far a probe counts as in view: the gather's reach at the volume's spacing.
pub const VIEW_REACH: f32 = 128.0;

pub fn freeze_census(
    level: &VeinLevel,
    frame: &Frame,
    camera: &Camera,
    freeze: u32,
) -> FreezeCensus {
    use vxe_render::probes::{MAX_AGE, PROBE_COUNT, PROBES_PER_AXIS, STATE_BASE, probe_slot};
    let b = readback::read_buffer(
        &level.device,
        &level.queue,
        frame.probes().data(),
        u64::from(STATE_BASE) * 4,
        u64::from(PROBE_COUNT) * 4,
    );
    let word = |s: u32| {
        u32::from_le_bytes(
            b[s as usize * 4..s as usize * 4 + 4]
                .try_into()
                .expect("a word"),
        )
    };
    let origin = frame.probes().origin_for(camera.position.to_array());
    let n = PROBES_PER_AXIS as i32;
    let mut c = FreezeCensus {
        all: [0; 3],
        in_view: [0; 3],
    };
    for z in 0..n {
        for y in 0..n {
            for x in 0..n {
                let w = [origin[0] + x, origin[1] + y, origin[2] + z];
                let s = word(probe_slot(w));
                let age = (s >> 1) & MAX_AGE;
                if age == 0 {
                    continue;
                }
                let inside = s & 1 != 0;
                let flags = [true, inside, !inside && age >= freeze];
                let p = Vec3::from(frame.probes().probe_position(w));
                let seen = (p - camera.position).length() < VIEW_REACH
                    && project(camera, SIZE, p).is_some();
                for (i, &f) in flags.iter().enumerate() {
                    c.all[i] += u32::from(f);
                    c.in_view[i] += u32::from(f && seen);
                }
            }
        }
    }
    c
}

/// One frame at `camera` under `timer`; the milliseconds of each of `FRAME_PASSES`.
pub fn timed_step(
    level: &VeinLevel,
    frame: &mut Frame,
    timer: &vxe_render::PassTimer,
    camera: &Camera,
) -> Vec<f64> {
    let light = LightParams::headlamp();
    frame.advance();
    let params = frame.params(&level.gpu, camera, &light);
    let mut enc = level
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    frame.encode(
        &level.device,
        &level.queue,
        &mut enc,
        &level.gpu,
        &params,
        Some(timer),
    );
    timer.resolve(&mut enc);
    level.queue.submit([enc.finish()]);
    timer.read_all_ms(&level.device)
}

/// The per-pass cost of `run` at 1080p from 720p (the frame `docs/nfr/02-frame-budget.md`
/// ledgers): each pass's median over the run's frames, from a start settled `settle` frames.
pub fn flight_cost(
    level: &VeinLevel,
    pose: &Pose,
    lighting: Lighting,
    run: KRun,
    settle: usize,
) -> Option<Vec<(&'static str, f64)>> {
    use vxe_render::marcher::FRAME_PASSES;
    let (device, queue) = (&level.device, &level.queue);
    let timer = vxe_render::PassTimer::with_passes(device, queue, FRAME_PASSES.len() as u32)?;
    let output = [1920, 1080];
    let mut frame = Frame::with_sizes(
        device,
        vxe_render::RenderScale::OneHalf.internal(output),
        output,
    );
    frame.set_upscaler_kind(device, start_upscaler());
    frame.set_look(look_of(LookPreset::DEFAULT));
    frame.set_lighting(lighting);
    frame.set_dt(1.0 / 60.0);
    render(level, &mut frame, &run.camera(pose, 0), settle);
    let mut ms = vec![Vec::new(); FRAME_PASSES.len()];
    for f in 1..FLIGHT_FRAMES {
        let t = timed_step(level, &mut frame, &timer, &run.camera(pose, f));
        for (a, v) in ms.iter_mut().zip(t) {
            a.push(v);
        }
    }
    Some(
        FRAME_PASSES
            .iter()
            .zip(ms)
            .map(|(&n, mut v)| {
                v.sort_by(f64::total_cmp);
                (n, v[v.len() / 2])
            })
            .collect(),
    )
}

/// The emissive table as words.
pub fn cache_words(level: &VeinLevel, frame: &Frame) -> Vec<u32> {
    let table = frame.emissive_table();
    let b = readback::read_buffer(&level.device, &level.queue, table, 0, table.size());
    b.chunks_exact(4)
        .map(|c| u32::from_le_bytes(c.try_into().expect("a word")))
        .collect()
}

/// The table's cells, from its words.
pub fn cache_cells(words: &[u32]) -> u32 {
    (words.len() / vxe_render::marcher::CACHE_CELL_WORDS as usize) as u32
}

/// The word index of each of a key's probed slots, in probe order.
fn cache_slots(words: &[u32], key: FaceKey) -> impl Iterator<Item = usize> {
    use vxe_render::marcher::{CACHE_CELL_WORDS, CACHE_PROBES, cache_slot};
    let cells = cache_cells(words);
    let slot = cache_slot(key.0, u32::from(key.1), cells);
    (0..CACHE_PROBES)
        .map(move |i| (slot.wrapping_add(i) & (cells - 1)) as usize * CACHE_CELL_WORDS as usize)
}

/// The cell of a key in the table's words: its word index, or `None` when no probed slot holds
/// its key.
pub fn cache_cell(words: &[u32], key: FaceKey) -> Option<usize> {
    let k = vxe_render::marcher::cache_key(key.0, u32::from(key.1));
    cache_slots(words, key).find(|&w| words[w] == k[0] && words[w + 1] == k[1])
}

/// Whether a key with no cell found none of its probed slots free: the table's slot-probe
/// failure, which the light pass counts as a miss and shows through the fallback.
pub fn cache_full(words: &[u32], key: FaceKey) -> bool {
    cache_cell(words, key).is_none() && cache_slots(words, key).all(|w| words[w + 1] != 0)
}

/// The sample count of a key's cell, 0 with none.
pub fn cache_count(words: &[u32], key: FaceKey) -> u32 {
    cache_cell(words, key).map_or(0, |w| words[w + vxe_render::marcher::CACHE_COUNT_WORD])
}

/// The table's cells in use and of them how many have reached `c_min` and `c_cap`.
pub fn cache_census(words: &[u32], c_min: u32, c_cap: u32) -> [u64; 3] {
    let mut c = [0u64; 3];
    let n = vxe_render::marcher::CACHE_COUNT_WORD;
    for cell in words.chunks_exact(vxe_render::marcher::CACHE_CELL_WORDS as usize) {
        if cell[1] != 0 {
            c[0] += 1;
            c[1] += u64::from(cell[n] >= c_min);
            c[2] += u64::from(cell[n] >= c_cap);
        }
    }
    c
}

/// Of a frame's internal hit pixels on cube faces, the share whose cell is young or absent
/// (the young share), the share with no cell at all, and of those the share whose probed slots
/// were all taken (`cache_full`, the full table's).
pub fn young_share(px: &Pixels, words: &[u32], c_min: u32) -> (f64, f64, f64) {
    let (mut hits, mut young, mut absent, mut full) = (0u64, 0u64, 0u64, 0u64);
    for i in 0..px.hit_pos.len() {
        if let Some(k) = face_key(px.camera.position, px.hit_pos[i], px.hit_info[i]) {
            hits += 1;
            match cache_cell(words, k) {
                None => {
                    absent += 1;
                    young += 1;
                    full += u64::from(cache_full(words, k));
                }
                Some(w) => {
                    young += u64::from(words[w + vxe_render::marcher::CACHE_COUNT_WORD] < c_min)
                }
            }
        }
    }
    let h = hits.max(1) as f64;
    (young as f64 / h, absent as f64 / h, full as f64 / h)
}

/// A frame's read-back for the dots and the young share.
pub fn read_dots(level: &VeinLevel, frame: &Frame, camera: Camera) -> Pixels {
    read(
        level,
        frame,
        camera,
        Want {
            hits: true,
            composite: true,
            moments: true,
        },
    )
}

/// One frame's display dots at `level`.
pub fn dots_of(px: &Pixels) -> usize {
    frame_dots(px).0.len()
}

/// The camera `degrees` of yaw from `pose`.
pub fn yawed(pose: &Pose, degrees: f32) -> Camera {
    camera_at(pose.position, pose.yaw_deg + degrees, pose.pitch_deg)
}

/// B's young share and table census along a run: `KRun` from a settled start, printed per 12
/// frames from frame 1, and the table's cells at the end, with the share of cells whose chunk
/// lies within the mirror's light radius of the pose.
pub fn cache_run(level: &VeinLevel, pose: &Pose, setting: Setting, run: KRun) -> String {
    let mut frame = make_frame(level, pose, setting);
    let light = LightParams::headlamp();
    render(level, &mut frame, &run.camera(pose, 0), SETTLE);
    let cap = vxe_render::marcher::DEFAULT_TABLE_CAP;
    let mut shares = Vec::new();
    let (mut sum, mut n) = (0.0, 0u32);
    let mut absent_max = 0.0f64;
    let mut full_max = 0.0f64;
    let mut seen = std::collections::HashSet::new();
    for f in 1..FLIGHT_FRAMES {
        let cam = run.camera(pose, f);
        frame.step(&level.device, &level.queue, &level.gpu, &cam, &light);
        if f % 4 == 1 {
            let px = read_dots(level, &frame, cam);
            for i in 0..px.hit_pos.len() {
                seen.extend(face_key(px.camera.position, px.hit_pos[i], px.hit_info[i]));
            }
            let words = cache_words(level, &frame);
            let (y, a, full) = young_share(&px, &words, YOUNG_COUNT);
            full_max = full_max.max(full);
            sum += y;
            n += 1;
            absent_max = absent_max.max(a);
            if f % 12 == 1 {
                shares.push(format!("{f}:{y:.3}"));
            }
        }
    }
    let words = cache_words(level, &frame);
    let [used, min, cap] = cache_census(&words, YOUNG_COUNT, cap);
    // The table's keys cannot be read back to voxels: this share is of the keys read in view.
    let eye = VoxelCoord(pose.position.map(|c| c.floor() as i32))
        .chunk()
        .0;
    let r = i64::from(level.gpu.light_radius());
    let near = seen
        .iter()
        .filter(|(v, _)| {
            let c = VoxelCoord(*v).chunk().0;
            (0..3)
                .map(|k| (i64::from(c[k]) - i64::from(eye[k])).pow(2))
                .sum::<i64>()
                <= r * r
        })
        .count();
    format!(
        "{} of {} keys read in view within the light radius of the pose; young share mean {:.4} (per frame {}); no-cell share max {absent_max:.5}; full-table share max {full_max:.5}; cells {used} ({:.4} of the table), at c_min {min}, at c_cap {cap}; frame_bytes {}",
        near,
        seen.len(),
        sum / f64::from(n.max(1)),
        shares.join(" "),
        used as f64 / f64::from(cache_cells(&words)),
        frame.frame_bytes()
    )
}

/// The frames after a dig at which `dig_box_run` reads the table (every frame to 64, then
/// every 8th to 512).
fn dig_table_frame(f: usize) -> bool {
    f <= 64 || (f <= 512 && f.is_multiple_of(8))
}

/// The dug hole's settle (`Event::Dig` or `Event::DigFar`, which `reset_world` undoes after):
/// settle `SETTLE` frames at `dig_camera`, the carve, then frames 1 ..= `REF.1` still. The hole's
/// pixels are the lighting pixels of frame 1 whose face key's voxel lies in the carve's box
/// dilated by one (the cells the edit-box reset clears). Each is a series of `meter_log2` of its
/// emissive part, read every frame to `SETTLE_DENSE` and sparsely after (`settle_sampled`), and
/// `settle_of` over them gives `S90` and the mean's 90 % frame; at `dig_table_frame`s, of the
/// hole pixels whose hit is still in the hole, the young share (no cell, or under
/// `YOUNG_COUNT` samples) and the share with no cell. One line.
pub fn dig_box_run(level: &mut VeinLevel, pose: &Pose, setting: Setting, far: bool) -> String {
    let mut frame = make_frame(level, pose, setting);
    let light = LightParams::headlamp();
    let cam = dig_camera(pose, if far { Event::DigFar } else { Event::Dig });
    render(level, &mut frame, &cam, SETTLE);
    let want = Want {
        hits: true,
        composite: false,
        moments: false,
    };
    let f0 = read(level, &frame, cam, want);
    let (min, max) = dig(level, &mut frame, &f0, far);
    let inside = |k: FaceKey| (0..3).all(|a| k.0[a] >= min[a] - 1 && k.0[a] <= max[a] + 1);
    let table = setting.steady.table;
    let mut pixels: Vec<usize> = Vec::new();
    let mut series: Vec<Vec<f32>> = Vec::new();
    let mut young = Vec::new();
    for f in 1..=REF.1 {
        frame.step(&level.device, &level.queue, &level.gpu, &cam, &light);
        let sampled = settle_sampled(f) || f >= REF.0;
        if !sampled && f > 1 {
            let last = series.last().expect("frame 1 read").clone();
            series.push(last);
            continue;
        }
        let px = read(level, &frame, cam, want);
        if f == 1 {
            let g = px.grid;
            pixels = (0..g[1])
                .flat_map(|y| (0..g[0]).map(move |x| (x, y)))
                .filter(|&(x, y)| {
                    let (ix, iy) = px.internal_of(g, x, y);
                    let i = (iy * SIZE[0] + ix) as usize;
                    face_key(px.camera.position, px.hit_pos[i], px.hit_info[i]).is_some_and(inside)
                })
                .map(|(x, y)| (y * g[0] + x) as usize)
                .collect();
            // Frame 0 stands for the dig's frame: the series starts at frame 1's reading.
            series.push(Vec::new());
        }
        let (_, v) = px.values(Part::Emissive, false);
        series.push(pixels.iter().map(|&i| meter_log2(v[i])).collect());
        if table && dig_table_frame(f) {
            let words = cache_words(level, &frame);
            let g = px.grid;
            let (mut y, mut a, mut n) = (0u32, 0u32, 0u32);
            for &l in &pixels {
                let (ix, iy) = px.internal_of(g, l as u32 % g[0], l as u32 / g[0]);
                let i = (iy * SIZE[0] + ix) as usize;
                // A hole pixel whose hit left the hole this frame (jitter) counts as neither.
                let Some(k) = face_key(px.camera.position, px.hit_pos[i], px.hit_info[i])
                    .filter(|&k| inside(k))
                else {
                    continue;
                };
                n += 1;
                let c =
                    cache_cell(&words, k).map(|w| words[w + vxe_render::marcher::CACHE_COUNT_WORD]);
                a += u32::from(c.is_none());
                y += u32::from(c.is_none_or(|c| c < YOUNG_COUNT));
            }
            let n = f64::from(n.max(1));
            young.push((f, f64::from(y) / n, f64::from(a) / n));
        }
    }
    series[0] = series[1].clone();
    let s = settle_of(&series, &vec![true; pixels.len()]);
    let at = [1usize, 2, 4, 8, 16, 32, 60, 64, 128, 256, 512];
    let curve: Vec<String> = at
        .iter()
        .map(|&f| format!("{f}:{:.0}", s.curve[f] * 100.0))
        .collect();
    let young: Vec<String> = young
        .iter()
        .filter(|(f, _, _)| at.contains(f))
        .map(|(f, y, a)| format!("{f}:{y:.3}/{a:.3}"))
        .collect();
    format!(
        "{} hole pixels ({} counted); emissive S90 {} mean90 {} (step {:+.3}, noise {:.4}); settled % at frame {}; young/no-cell share in the hole at frame {}",
        pixels.len(),
        s.counted,
        s.frames.map_or("none".into(), |f| f.to_string()),
        s.mean_90.map_or("none".into(), |f| f.to_string()),
        s.step0,
        s.ref_noise,
        curve.join(" "),
        if table {
            young.join(" ")
        } else {
            "n/a (no table)".to_owned()
        }
    )
}

/// The table's occupancy over `flights` repeats of the steady flight from a settled start
/// (`KRun::Path`, the camera back at the start between them, the table never cleared): per
/// flight, the cells in use, at `c_min` and at `c_cap`, and the largest no-cell and full-table
/// shares (`young_share`) over every 4th frame. One line a flight.
pub fn occupancy_run(
    level: &VeinLevel,
    pose: &Pose,
    setting: Setting,
    flights: usize,
) -> Vec<String> {
    let cap = vxe_render::marcher::DEFAULT_TABLE_CAP;
    let mut frame = make_frame(level, pose, setting);
    let light = LightParams::headlamp();
    render(level, &mut frame, &KRun::Path.camera(pose, 0), SETTLE);
    let mut out = Vec::new();
    for k in 1..=flights {
        let (mut absent_max, mut full_max) = (0.0f64, 0.0f64);
        for f in 0..FLIGHT_FRAMES {
            let cam = KRun::Path.camera(pose, f);
            frame.step(&level.device, &level.queue, &level.gpu, &cam, &light);
            if f % 4 == 1 {
                let px = read_dots(level, &frame, cam);
                let words = cache_words(level, &frame);
                let (_, a, full) = young_share(&px, &words, YOUNG_COUNT);
                absent_max = absent_max.max(a);
                full_max = full_max.max(full);
            }
        }
        let words = cache_words(level, &frame);
        let [used, min, at_cap] = cache_census(&words, YOUNG_COUNT, cap);
        out.push(format!(
            "after flight {k}: cells {used} ({:.4} of {}), at c_min {min}, at c_cap {at_cap}; no-cell share max {absent_max:.5}, full-table share max {full_max:.5}",
            used as f64 / f64::from(cache_cells(&words)),
            cache_cells(&words)
        ));
    }
    out
}
