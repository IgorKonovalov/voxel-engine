//! The patch bar's rig (ADR-0036): the level with its ore emissive, a still camera facing a
//! vein, the lamp switched off, and the statistics of the light's tile means while it should
//! hold still.
//!
//! A run renders `LAMP_ON` frames with the lamp on and then `LAMP_OFF` with it off, under a
//! manual exposure pinned per pose. Off frames are numbered from 1, the first frame the lamp
//! is off. From off frame `WINDOW.0` to `WINDOW.1` it reads, per frame, the exposure meter's
//! 8 x 8 tiles (`Frame::exposure_tiles`: the sum of the clamped log2 luminance of the shaded
//! radiance and the pixel count), the display image, and on the lighting grid the emissive
//! accumulation (`Frame::accumulated`, rgb) and the probe gather
//! (`Frame::indirect_irradiance`), each reduced to the same tiles: the mean over the tile's
//! lighting pixels of `clamp(log2(max(Y / exposure, 2^-16)), -16, 16)`, the meter's own
//! transform of the absolute value.
//!
//! A tile is **full** in a frame when its 64 pixels are all hits and the composite's mean is
//! above `FULL_LOG2`; a part's tile also needs its own mean above it. A **patch** is a tile
//! full at both ends of a `STEP`-frame step whose mean moved by more than `log2 1.1`. A
//! **sudden step** is the same between consecutive frames. **Flicker** is the mean |d log2|
//! between consecutive frames over the tiles full in both, and for the composite the display
//! luma's mean |d| per frame in /255.
//!
//! With `STILL_FILMSTRIP` set to a tag, each run also writes contact sheets of its window
//! (`filmstrip`) under the worktree's `live/filmstrip/<tag>/`; unset or empty, it writes
//! nothing.

use super::*;
use vxe_render::{
    Exposure, Frame, GpuWorld, LightParams, Lighting, MirrorConfig, Window, readback,
};

/// Frames with the lamp on before it switches off.
pub const LAMP_ON: usize = 120;
/// Frames with the lamp off.
pub const LAMP_OFF: usize = 720;
/// The off frames the statistics read, inclusive.
pub const WINDOW: (usize, usize) = (120, 720);
/// The frames between a patch step's two ends (0.5 s at 60 fps).
pub const STEP: usize = 30;
/// The composite's (and a part's) tile mean a full tile lies above, in log2.
pub const FULL_LOG2: f32 = -15.0;
/// The meter's tile edge in internal pixels.
pub const TILE: u32 = 8;
/// The bar's frame size.
pub const SIZE: [u32; 2] = [192, 108];

/// A still camera on the level, with the manual exposure its run is pinned at.
#[derive(Clone, Copy, Debug)]
pub struct Pose {
    pub name: &'static str,
    pub position: [f32; 3],
    pub yaw_deg: f32,
    pub pitch_deg: f32,
    pub exposure: f32,
}

/// Facing the wall where the lamp-off patches were first counted. The exposure is the eye's
/// adapted value after 600 frames with the lamp on and 900 off at 192 x 108.
pub const PATCH_POSE: Pose = Pose {
    name: "patch",
    position: [676.2, 514.8, 193.1],
    yaw_deg: 59.0,
    pitch_deg: -9.7,
    exposure: 10.9624,
};

/// Out from an ore vein along its normal, looking at it (`exposure.rs`'s pose), at its eye's
/// adapted value, read the same way.
pub const VEIN_POSE: Pose = Pose {
    name: "vein",
    position: [727.5, 644.3, 208.5],
    yaw_deg: 153.4,
    pitch_deg: 0.0,
    exposure: VEIN_EXPOSURE,
};

/// `VEIN_POSE`'s adapted exposure.
pub const VEIN_EXPOSURE: f32 = 5.3720;

impl Pose {
    pub fn camera(&self) -> Camera {
        Camera {
            position: self.position.into(),
            yaw: self.yaw_deg.to_radians(),
            pitch: self.pitch_deg.to_radians(),
            vfov: 70f32.to_radians(),
        }
    }
}

/// The level (seed 1, the ore emissive) mirrored whole on the software adapter before the run,
/// so no upload lands inside it.
pub struct VeinLevel {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub gpu: GpuWorld,
}

pub fn vein_level() -> Option<VeinLevel> {
    let source = flythrough::cave::CaveSource::new(
        1,
        ChunkExtent::new([0, 0, 0], flythrough::cave::LEVEL_CHUNKS),
    );
    let world = World::generate(
        source.extent,
        &source,
        &flythrough::with_emissive_ore(&flythrough::cave_materials()),
    );
    let pool = flythrough::level_pool_units(&world, false) * vxe_render::gpu_world::UNIT_BYTES;
    let (device, queue) = software_device(pool)?;
    let cfg = MirrorConfig {
        budget_bytes: 256 << 20,
        ..MirrorConfig::new(Window::Fixed(world.extent()))
    };
    let mut gpu = GpuWorld::with_config(&device, cfg);
    gpu.set_focus(source.spawn());
    for _ in 0..4 {
        gpu.sync_now(&world, &device, &queue)
            .expect("mirror fits the adapter");
    }
    Some(VeinLevel { device, queue, gpu })
}

/// One part's statistics over the window.
#[derive(Clone, Debug)]
pub struct Stats {
    /// Patches on each of the window's steps.
    pub patches: Vec<usize>,
    /// Sudden steps over the window, per second at 60 fps.
    pub sudden_per_s: f64,
    /// The tiles' mean |d log2| per frame.
    pub flicker: f64,
    /// The display luma's mean |d| per frame in /255 (the composite only).
    pub luma_flicker: Option<f64>,
    /// Tiles full in the window's last frame.
    pub full_tiles: usize,
}

impl Stats {
    pub fn mean_patches(&self) -> f64 {
        self.patches.iter().sum::<usize>() as f64 / self.patches.len().max(1) as f64
    }

    pub fn max_patches(&self) -> usize {
        self.patches.iter().copied().max().unwrap_or(0)
    }

    pub fn line(&self) -> String {
        let steps: Vec<String> = self.patches.iter().map(usize::to_string).collect();
        format!(
            "patches/step mean {:.2} max {} [{}], sudden {:.2}/s, flicker {:.5} log2/frame{}, {} full tiles",
            self.mean_patches(),
            self.max_patches(),
            steps.join(" "),
            self.sudden_per_s,
            self.flicker,
            self.luma_flicker
                .map_or(String::new(), |l| format!(", luma {l:.3}/255")),
            self.full_tiles,
        )
    }
}

/// A run's statistics: the composite's tiles, the emissive accumulation's and the gather's.
#[derive(Clone, Debug)]
pub struct Reading {
    pub composite: Stats,
    pub emissive: Stats,
    pub gather: Stats,
    /// The gather's mean linear luminance over the lighting grid and the window's frames.
    pub gather_luminance: f64,
}

impl Reading {
    /// The `reading:` lines of a run named `name`.
    pub fn print(&self, name: &str) {
        println!("reading: {name} composite: {}", self.composite.line());
        println!("reading: {name} emissive: {}", self.emissive.line());
        println!("reading: {name} gather: {}", self.gather.line());
        println!(
            "reading: {name} gather mean luminance {:.6}",
            self.gather_luminance
        );
    }
}

/// Per tile: the mean and whether every pixel of it counted (the composite), or the mean alone.
type Tiles = Vec<(f32, bool)>;

/// The meter's transform of an absolute luminance.
fn meter_log2(y: f32) -> f32 {
    y.max(2f32.powi(-16)).log2().clamp(-16.0, 16.0)
}

/// Lighting-grid pixels reduced to the meter's tiles of an internal `size` frame, `scale`
/// internal pixels per lighting pixel on an edge.
fn grid_tiles(px: &[[f32; 4]], grid: [u32; 2], size: [u32; 2], scale: u32, exposure: f32) -> Tiles {
    let [tw, th] = size.map(|n| n.div_ceil(TILE));
    let mut out = Vec::with_capacity((tw * th) as usize);
    for ty in 0..th {
        for tx in 0..tw {
            let (mut s, mut c) = (0.0f64, 0u32);
            let x0 = tx * TILE / scale;
            let y0 = ty * TILE / scale;
            let x1 = ((tx + 1) * TILE).div_ceil(scale).min(grid[0]);
            let y1 = ((ty + 1) * TILE).div_ceil(scale).min(grid[1]);
            for y in y0..y1 {
                for x in x0..x1 {
                    let p = px[(y * grid[0] + x) as usize];
                    let lum = 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
                    s += f64::from(meter_log2(lum / exposure));
                    c += 1;
                }
            }
            out.push(((s / f64::from(c.max(1))) as f32, c > 0));
        }
    }
    out
}

/// The statistics of one part's per-frame tiles over the window, `full[f][t]` being the
/// composite's full mask, and the display images for the luma flicker.
/// Whether tile `t` of frame `f` counts: full in the composite, and its own mean above
/// `FULL_LOG2`.
fn counted(tiles: &[Tiles], full: &[Vec<bool>], f: usize, t: usize) -> bool {
    full[f][t] && tiles[f][t].1 && tiles[f][t].0 > FULL_LOG2
}

/// Whether tile `t` is a patch between frames `a` and `b`: counted at both, its mean moved by
/// more than `log2 1.1`.
fn patch(tiles: &[Tiles], full: &[Vec<bool>], a: usize, b: usize, t: usize) -> bool {
    counted(tiles, full, a, t)
        && counted(tiles, full, b, t)
        && (tiles[b][t].0 - tiles[a][t].0).abs() > 1.1f32.log2()
}

fn stats(tiles: &[Tiles], full: &[Vec<bool>], images: Option<&[Vec<u8>]>) -> Stats {
    let n = tiles.len();
    let ok = |f: usize, t: usize| counted(tiles, full, f, t);
    let mean = |f: usize, t: usize| tiles[f][t].0;
    let count = |a: usize, b: usize| {
        (0..tiles[0].len())
            .filter(|&t| patch(tiles, full, a, b, t))
            .count()
    };
    let patches = (0..(n - 1) / STEP)
        .map(|k| count(k * STEP, (k + 1) * STEP))
        .collect();
    let sudden: usize = (1..n).map(|f| count(f - 1, f)).sum();
    let (mut s, mut c) = (0.0f64, 0u64);
    for f in 1..n {
        for t in 0..tiles[0].len() {
            if ok(f, t) && ok(f - 1, t) {
                s += f64::from((mean(f, t) - mean(f - 1, t)).abs());
                c += 1;
            }
        }
    }
    let luma_flicker = images.map(|im| {
        let l = |x: &[u8]| {
            0.2126 * f64::from(x[0]) + 0.7152 * f64::from(x[1]) + 0.0722 * f64::from(x[2])
        };
        let (mut s, mut c) = (0.0f64, 0u64);
        for f in 1..n {
            for (p, q) in im[f].chunks_exact(4).zip(im[f - 1].chunks_exact(4)) {
                s += (l(p) - l(q)).abs();
                c += 1;
            }
        }
        s / c.max(1) as f64
    });
    Stats {
        patches,
        sudden_per_s: sudden as f64 / ((n - 1) as f64 / 60.0),
        flicker: s / c.max(1) as f64,
        luma_flicker,
        full_tiles: (0..tiles[0].len()).filter(|&t| ok(n - 1, t)).count(),
    }
}

/// One off frame of a traced run (`run_traced`): whether the probe volume's light-change
/// window was open, how many probes' ages restarted (an age below the slot's last frame's),
/// how many probes are young (age 1 to `YOUNG_UPDATES`), and the gather's tile mean |d log2|
/// from the frame before over the tiles full in both.
#[derive(Clone, Copy, Debug)]
pub struct TraceFrame {
    pub off: usize,
    pub window: bool,
    pub restarts: u32,
    pub young: u32,
    pub gather_dlog2: f64,
}

/// One run at `pose` with `lighting` (indirect light on and the pose's manual exposure
/// imposed), `size` internal pixels, the lamp switched off after `LAMP_ON` frames.
pub fn run(level: &VeinLevel, pose: &Pose, lighting: Lighting, size: [u32; 2]) -> Reading {
    run_inner(level, pose, lighting, size, false).0
}

/// `run`, plus a `TraceFrame` for every off frame of the window.
pub fn run_traced(
    level: &VeinLevel,
    pose: &Pose,
    lighting: Lighting,
    size: [u32; 2],
) -> (Reading, Vec<TraceFrame>) {
    run_inner(level, pose, lighting, size, true)
}

fn run_inner(
    level: &VeinLevel,
    pose: &Pose,
    lighting: Lighting,
    size: [u32; 2],
    traced: bool,
) -> (Reading, Vec<TraceFrame>) {
    let (device, queue) = (&level.device, &level.queue);
    let lit = Lighting {
        indirect: true,
        exposure: Exposure::Manual(pose.exposure),
        ..lighting
    };
    let mut frame = Frame::new(device, size[0], size[1]);
    frame.set_lighting(lit);
    frame.set_dt(1.0 / 60.0);
    let camera = pose.camera();
    let light = LightParams::headlamp();
    let [tw, th] = size.map(|n| n.div_ceil(TILE));
    let tiles_n = (tw * th) as usize;
    let mut composite: Vec<Tiles> = Vec::new();
    let mut full: Vec<Vec<bool>> = Vec::new();
    let mut emissive: Vec<Tiles> = Vec::new();
    let mut gather: Vec<Tiles> = Vec::new();
    let mut images: Vec<Vec<u8>> = Vec::new();
    let mut gather_lum = 0.0f64;
    let mut trace: Vec<TraceFrame> = Vec::new();
    let mut ages_before: Vec<u32> = Vec::new();
    let age_of = |w: u32| (w >> 1) & vxe_render::probes::MAX_AGE;
    for i in 0..LAMP_ON + LAMP_OFF {
        if i == LAMP_ON {
            frame.set_lighting(Lighting { lamp: false, ..lit });
        }
        frame.step(device, queue, &level.gpu, &camera, &light);
        if i < LAMP_ON {
            continue;
        }
        let off = i + 1 - LAMP_ON;
        if traced && off + 1 >= WINDOW.0 && off <= WINDOW.1 {
            let ages: Vec<u32> = frame
                .probes()
                .read_states(device, queue)
                .iter()
                .map(|&w| age_of(w))
                .collect();
            if off >= WINDOW.0 {
                let restarts = ages.iter().zip(&ages_before).filter(|(a, b)| a < b).count() as u32;
                let young = ages
                    .iter()
                    .filter(|&&a| (1..=vxe_render::probes::YOUNG_UPDATES).contains(&a))
                    .count() as u32;
                trace.push(TraceFrame {
                    off,
                    window: frame.probe_light_window(),
                    restarts,
                    young,
                    gather_dlog2: 0.0,
                });
            }
            ages_before = ages;
        }
        if off < WINDOW.0 || off > WINDOW.1 {
            continue;
        }
        let b = readback::read_buffer(device, queue, frame.exposure_tiles(), 0, 8 * tiles_n as u64);
        let t: Tiles = b
            .chunks_exact(8)
            .map(|c| {
                let sum = f32::from_le_bytes(c[0..4].try_into().unwrap());
                let n = f32::from_le_bytes(c[4..8].try_into().unwrap());
                (sum / n.max(1.0), n >= (TILE * TILE) as f32)
            })
            .collect();
        full.push(t.iter().map(|&(m, all)| all && m > FULL_LOG2).collect());
        composite.push(t);
        let acc = frame.accumulated();
        let grid = [acc.width(), acc.height()];
        let scale = size[0].div_ceil(grid[0]);
        let e = readback::read_float(device, queue, acc);
        emissive.push(grid_tiles(&e, grid, size, scale, pose.exposure));
        let g = readback::read_float(device, queue, frame.indirect_irradiance());
        gather_lum += g
            .iter()
            .map(|p| f64::from(0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]))
            .sum::<f64>()
            / g.len() as f64;
        gather.push(grid_tiles(&g, grid, size, scale, pose.exposure));
        images.push(readback::read_rgba8(device, queue, frame.target()));
    }
    for f in 1..trace.len().min(gather.len()) {
        let (mut s, mut c) = (0.0f64, 0u32);
        for t in 0..tiles_n {
            let ok = |k: usize| full[k][t] && gather[k][t].1 && gather[k][t].0 > FULL_LOG2;
            if ok(f) && ok(f - 1) {
                s += f64::from((gather[f][t].0 - gather[f - 1][t].0).abs());
                c += 1;
            }
        }
        trace[f].gather_dlog2 = s / f64::from(c.max(1));
    }
    if let Some(tag) = std::env::var("STILL_FILMSTRIP")
        .ok()
        .filter(|t| !t.is_empty())
    {
        let mut name = pose.name.to_owned();
        if size != SIZE {
            name = format!("{name}-{}x{}", size[0], size[1]);
        }
        filmstrip(
            &tag,
            &format!("{name}-composite"),
            size,
            &composite,
            &full,
            Some(&images),
        );
        filmstrip(
            &tag,
            &format!("{name}-emissive"),
            size,
            &emissive,
            &full,
            None,
        );
        filmstrip(&tag, &format!("{name}-gather"), size, &gather, &full, None);
    }
    (
        Reading {
            composite: stats(&composite, &full, Some(&images)),
            emissive: stats(&emissive, &full, None),
            gather: stats(&gather, &full, None),
            gather_luminance: gather_lum / gather.len().max(1) as f64,
        },
        trace,
    )
}

/// The trace's lines for each gather step over `over` patches: the 30 frames before the step
/// and the step's own, one `off:window/restarts/young/dlog2` entry a frame.
pub fn print_trace(name: &str, gather: &Stats, trace: &[TraceFrame], over: usize) {
    for (k, &p) in gather.patches.iter().enumerate() {
        if p <= over {
            continue;
        }
        let (a, b) = (WINDOW.0 + k * STEP, WINDOW.0 + (k + 1) * STEP);
        let rows: Vec<String> = trace
            .iter()
            .filter(|t| t.off + STEP >= a && t.off <= b)
            .map(|t| {
                format!(
                    "{}:{}/{}/{}/{:.4}",
                    t.off,
                    u8::from(t.window),
                    t.restarts,
                    t.young,
                    t.gather_dlog2
                )
            })
            .collect();
        println!(
            "reading: {name} trace, gather step {k} (off {a} to {b}, {p} patches): {}",
            rows.join(" ")
        );
    }
    let windows = trace.iter().filter(|t| t.window).count();
    let restarts: u32 = trace.iter().map(|t| t.restarts).sum();
    println!(
        "reading: {name} trace over off {} to {}: window open on {windows} frames, {restarts} probe age restarts",
        WINDOW.0, WINDOW.1
    );
}

/// Columns of a filmstrip sheet; the frames run row by row.
const SHEET_COLUMNS: usize = 7;
/// Pixels between a sheet's frames.
const SHEET_GAP: u32 = 2;
/// The outline colour of a tile counted as a patch on the step into its frame.
const PATCH_OUTLINE: [u8; 4] = [255, 0, 255, 255];

/// Writes the contact sheet `live/filmstrip/<tag>/<name>.png` (in the worktree's root) of the
/// window's frames at every `STEP` off frames from `WINDOW.0` (`WINDOW.0`, `WINDOW.0 + STEP`,
/// ... `WINDOW.1`), `SHEET_COLUMNS` to a row, each at `size`: the display image when `images`
/// is given, else each tile's mean as grey, from black at the sheet's lowest counted tile
/// mean to white at its highest (uncounted tiles black). The tiles that are patches on the
/// step into a frame (from the frame `STEP` before it) carry a one-pixel `PATCH_OUTLINE`.
/// Prints the path and the frame order.
fn filmstrip(
    tag: &str,
    name: &str,
    size: [u32; 2],
    tiles: &[Tiles],
    full: &[Vec<bool>],
    images: Option<&[Vec<u8>]>,
) {
    let frames: Vec<usize> = (0..tiles.len()).step_by(STEP).collect();
    let rows = frames.len().div_ceil(SHEET_COLUMNS);
    let [w, h] = size;
    let sw = SHEET_COLUMNS as u32 * (w + SHEET_GAP) - SHEET_GAP;
    let sh = rows as u32 * (h + SHEET_GAP) - SHEET_GAP;
    let mut sheet = vec![0u8; (sw * sh * 4) as usize];
    let tw = w.div_ceil(TILE);
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for &f in &frames {
        for (t, &(m, _)) in tiles[f].iter().enumerate() {
            if counted(tiles, full, f, t) {
                lo = lo.min(m);
                hi = hi.max(m);
            }
        }
    }
    for (k, &f) in frames.iter().enumerate() {
        let ox = (k % SHEET_COLUMNS) as u32 * (w + SHEET_GAP);
        let oy = (k / SHEET_COLUMNS) as u32 * (h + SHEET_GAP);
        for y in 0..h {
            for x in 0..w {
                let t = ((y / TILE) * tw + x / TILE) as usize;
                let px = match images {
                    Some(im) => {
                        let i = ((y * w + x) * 4) as usize;
                        [im[f][i], im[f][i + 1], im[f][i + 2], 255]
                    }
                    None if counted(tiles, full, f, t) => {
                        let g = ((tiles[f][t].0 - lo) / (hi - lo).max(1e-6) * 255.0)
                            .round()
                            .clamp(0.0, 255.0) as u8;
                        [g, g, g, 255]
                    }
                    None => [0, 0, 0, 255],
                };
                let edge = x % TILE == 0
                    || y % TILE == 0
                    || x % TILE == TILE - 1
                    || y % TILE == TILE - 1
                    || x == w - 1
                    || y == h - 1;
                let outlined = f >= STEP && edge && patch(tiles, full, f - STEP, f, t);
                let i = (((oy + y) * sw + ox + x) * 4) as usize;
                sheet[i..i + 4].copy_from_slice(if outlined { &PATCH_OUTLINE } else { &px });
            }
        }
    }
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = root.canonicalize().unwrap_or(root);
    let dir = root.join("live/filmstrip").join(tag);
    std::fs::create_dir_all(&dir).expect("create the filmstrip directory");
    let path = dir.join(format!("{name}.png"));
    write_png(&path, &sheet, sw, sh);
    let order: Vec<String> = frames.iter().map(|f| (WINDOW.0 + f).to_string()).collect();
    println!(
        "filmstrip: {} ({} x {} frames of {w} x {h}, row by row, off frames {}{})",
        path.display(),
        SHEET_COLUMNS,
        rows,
        order.join(" "),
        if images.is_some() {
            String::new()
        } else {
            format!("; grey from tile mean {lo:.3} to {hi:.3} log2")
        }
    );
}
