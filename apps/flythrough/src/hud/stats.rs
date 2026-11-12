//! The HUD's frame stats, its cadence, the world sample, the rows it prints and [`HudStats`].

use std::fmt::{self, Write as _};

use vxe_render::HudColour;
use vxe_world::{BRICK_VOXELS, Chunk, MaterialId, WorldStats};

/// Frame durations the window holds: 240 frames, 4 s at 60 fps.
pub const RING: usize = 240;

/// The strip's ceiling: two 60 Hz frames.
pub const CEILING_MS: f32 = 33.3;

/// `nfr.md` §2 lines the rows are coloured against, in milliseconds.
const FRAME_LINE_MS: f32 = 1000.0 / 60.0;
const MARCH_LINE_MS: f32 = 3.0;
const UPLOAD_LINE_MS: f32 = 1.0;
/// `nfr.md` §2's lighting and denoise lines.
const LIGHTING_LINE_MS: f32 = 6.0;
const DENOISE_LINE_MS: f32 = 3.0;
/// `nfr.md` §2's upscale, tone map and post line: the composite, upscale and sharpen passes.
const UPSCALE_LINE_MS: f32 = 1.5;
/// Mirror pressure above which the fly-through lowers its full radius (ADR-0013); the
/// pressure row turns yellow past it.
pub const PRESSURE_HIGH: f32 = 0.9;

/// A rolling window of frame durations fed explicit deltas, holding no clock: fps, mean,
/// median, p99 and max over the last [`RING`] frames, with no allocation per frame. Copied
/// from Ritmolux (`core/src/diag/mod.rs`, `FrameStats`, commit `ad375596`); percentiles are
/// nearest-rank at index `round(q (n - 1))` of the sorted window, so a reading is always a
/// sample that occurred.
pub struct FrameStats {
    ring: [f32; RING],
    /// Next write position. The valid set is the first `len` slots before the ring wraps and
    /// all of them after, so `ring.iter().take(len)` yields exactly the retained samples.
    head: usize,
    len: usize,
    frames_total: u64,
}

impl Default for FrameStats {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameStats {
    pub fn new() -> Self {
        Self {
            ring: [0.0; RING],
            head: 0,
            len: 0,
            frames_total: 0,
        }
    }

    /// Records one frame's duration in seconds.
    pub fn record(&mut self, dt_secs: f32) {
        if let Some(slot) = self.ring.get_mut(self.head) {
            *slot = dt_secs;
        }
        self.head = (self.head + 1) % RING;
        if self.len < RING {
            self.len += 1;
        }
        self.frames_total = self.frames_total.saturating_add(1);
    }

    fn sum(&self) -> f32 {
        self.ring.iter().take(self.len).sum()
    }

    /// Frames per second over the window; zero until the first sample.
    pub fn fps(&self) -> f32 {
        let sum = self.sum();
        if self.len == 0 || sum <= 0.0 {
            return 0.0;
        }
        self.len as f32 / sum
    }

    /// Mean frame time over the window, in milliseconds.
    pub fn frame_ms_avg(&self) -> f32 {
        if self.len == 0 {
            return 0.0;
        }
        self.sum() / self.len as f32 * 1000.0
    }

    /// Median frame time over the window, in milliseconds.
    pub fn frame_ms_p50(&self) -> f32 {
        self.frame_ms_percentile(0.5)
    }

    /// 99th-percentile frame time over the window, in milliseconds.
    pub fn frame_ms_p99(&self) -> f32 {
        self.frame_ms_percentile(0.99)
    }

    /// Longest frame time over the window, in milliseconds.
    pub fn frame_ms_max(&self) -> f32 {
        self.frame_ms_percentile(1.0)
    }

    /// The frame time at quantile `q`, in milliseconds, from a sorted stack-local copy of the
    /// window.
    fn frame_ms_percentile(&self, q: f32) -> f32 {
        if self.len == 0 {
            return 0.0;
        }
        let mut buf = [0.0f32; RING];
        let n = self.len;
        buf[..n].copy_from_slice(&self.ring[..n]);
        let slice = &mut buf[..n];
        slice.sort_by(f32::total_cmp);
        let idx = ((n - 1) as f32 * q.clamp(0.0, 1.0)).round() as usize;
        slice.get(idx).copied().unwrap_or(0.0) * 1000.0
    }

    /// Frames recorded since creation.
    pub fn frames_total(&self) -> u64 {
        self.frames_total
    }

    /// The retained durations in seconds, oldest first, without allocating.
    pub fn samples(&self) -> impl Iterator<Item = f32> + '_ {
        let (a, b) = if self.len == RING {
            (&self.ring[self.head..], &self.ring[..self.head])
        } else {
            (&self.ring[..self.len], &self.ring[..0])
        };
        a.iter().chain(b).copied()
    }

    /// The window's summary.
    pub fn frame_ms(&self) -> FrameMs {
        FrameMs {
            mean: self.frame_ms_avg(),
            p50: self.frame_ms_p50(),
            p99: self.frame_ms_p99(),
            max: self.frame_ms_max(),
        }
    }
}

/// Frame time over the [`FrameStats`] window, in milliseconds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameMs {
    pub mean: f32,
    pub p50: f32,
    pub p99: f32,
    pub max: f32,
}

/// When the world's stats are sampled: once on the frame the HUD turns visible, then every
/// `every`th visible frame; never on a hidden frame. The `--frames` print at exit samples
/// on its own, outside the cadence.
#[derive(Clone, Copy, Debug)]
pub struct Cadence {
    every: u32,
    /// Visible frames since the last sample.
    age: u32,
    was_visible: bool,
}

impl Cadence {
    pub fn new(every: u32) -> Self {
        Self {
            every: every.max(1),
            age: 0,
            was_visible: false,
        }
    }

    /// Advances one frame; true when this frame takes a sample.
    pub fn due(&mut self, visible: bool) -> bool {
        let turned_on = visible && !self.was_visible;
        self.was_visible = visible;
        if !visible {
            return false;
        }
        if turned_on {
            self.age = 0;
            return true;
        }
        self.age += 1;
        if self.age >= self.every {
            self.age = 0;
            return true;
        }
        false
    }

    /// Frames since the last sample.
    pub fn age(&self) -> u32 {
        self.age
    }
}

/// A sample of the world's counts, taken at the [`Cadence`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WorldSample {
    /// Resident full chunks.
    pub chunks: u32,
    /// LOD-only resident chunks, when streaming.
    pub lod_chunks: Option<u32>,
    pub full_bricks: u64,
    /// Spare voxel arrays in the world's brick-array pool (`World::pool_len`).
    pub pool_arrays: u64,
    /// An estimate: [`cpu_bytes_estimate`] of the counts above.
    pub cpu_bytes: u64,
    /// Frames since the sample was taken.
    pub age_frames: u32,
    /// What the sampling call cost, in milliseconds.
    pub call_ms: f32,
}

impl WorldSample {
    pub fn from_stats(
        s: &WorldStats,
        lod_chunks: Option<u32>,
        pool_arrays: usize,
        call_ms: f32,
    ) -> Self {
        let (chunks, full_bricks, pool_arrays) =
            (s.chunks as u64, s.full_bricks as u64, pool_arrays as u64);
        Self {
            chunks: s.chunks as u32,
            lod_chunks,
            full_bricks,
            pool_arrays,
            cpu_bytes: cpu_bytes_estimate(chunks, full_bricks, pool_arrays),
            age_frames: 0,
            call_ms,
        }
    }
}

/// RAM the resident full chunks hold, estimated from counts alone: each `Full` brick's voxel
/// array, each spare array in the brick-array pool, and each chunk's `Chunk`. Uniform bricks
/// live inside `Chunk`.
pub fn cpu_bytes_estimate(chunks: u64, full_bricks: u64, pool_arrays: u64) -> u64 {
    (full_bricks + pool_arrays) * (BRICK_VOXELS * size_of::<MaterialId>()) as u64
        + chunks * size_of::<Chunk>() as u64
}

/// A pass time in milliseconds to three places, or `n/a` without a reading; formats in place.
struct Ms(Option<f32>);

impl fmt::Display for Ms {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(v) => write!(f, "{v:.3} ms"),
            None => f.write_str("n/a"),
        }
    }
}

/// The movement mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Fly,
    Walk,
}

/// A gamepad's power as `gilrs` reports it, the fly-through's own copy so the HUD names no
/// gamepad library.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PadPower {
    #[default]
    Unknown,
    Wired,
    Discharging(u8),
    Charging(u8),
    Charged,
}

impl fmt::Display for PadPower {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PadPower::Unknown => f.write_str("power unknown"),
            PadPower::Wired => f.write_str("wired"),
            PadPower::Discharging(n) => write!(f, "battery {n}%"),
            PadPower::Charging(n) => write!(f, "charging {n}%"),
            PadPower::Charged => f.write_str("charged"),
        }
    }
}

/// The pad row: the active pad's name and power; an empty name reads `pad none`. The name
/// is rewritten in place only when it changes, so a shown row allocates nothing per frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PadRow {
    pub name: String,
    pub power: Option<PadPower>,
}

impl fmt::Display for PadRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.name.is_empty(), self.power) {
            (true, _) => f.write_str("pad none"),
            (false, Some(p)) => write!(f, "pad {}, {p}", self.name),
            (false, None) => write!(f, "pad {}", self.name),
        }
    }
}

/// The last edit: its verb, the count changed per material name, the chunks it touched and
/// its cost.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EditLine {
    pub verb: &'static str,
    pub counts: Vec<(String, u32)>,
    pub chunks: u32,
    pub ms: f32,
}

impl fmt::Display for EditLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: ", self.verb)?;
        if self.counts.is_empty() {
            f.write_str("nothing")?;
        }
        for (i, (name, n)) in self.counts.iter().enumerate() {
            let sep = if i == 0 { "" } else { ", " };
            write!(f, "{sep}{name} {n}")?;
        }
        write!(f, " ({} chunks, {:.2} ms)", self.chunks, self.ms)
    }
}

/// Everything the HUD shows for one frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HudStats {
    pub fps: f32,
    pub frame_ms: FrameMs,
    /// Render-thread CPU of this frame, from its start to its submit.
    pub cpu_ms: f32,
    /// The window's pacing word (`vxe_app::Pacing::word`); `None` hides the `pace` row.
    pub pace: Option<&'static str>,
    /// GPU time of the march and the HUD pass; `None` without `TIMESTAMP_QUERY`.
    pub march_ms: Option<f32>,
    pub hud_ms: Option<f32>,
    /// The last mirror sync.
    pub upload_bytes: u64,
    pub upload_ms: f32,
    pub upload_chunks: u32,
    pub mirror_bytes: u64,
    /// Slots holding a brick, and spare slots, over every bin.
    pub pool_live: u32,
    pub pool_free: u32,
    pub world: WorldSample,
    /// Process RSS, read with the world sample; `None` off Linux.
    pub rss_bytes: Option<u64>,
    pub last_edit: Option<EditLine>,
    pub camera: [f32; 3],
    pub chunk: [i32; 3],
    pub mode: Mode,
    /// The cutaway mode's name while the cutaway view is on; `None` hides its row.
    pub cutaway: Option<&'static str>,
    /// What formatting the previous rows cost, in microseconds.
    pub format_us: u32,
    /// The steps view's reading of the last frame, while that view is on.
    pub steps: Option<StepsSample>,
    /// The cracks view's reading of the last frame, while that view is on.
    pub cracks: Option<vxe_render::CrackShares>,
    /// The mirror's pressure against its reservation; `None` hides its row.
    pub pressure: Option<Pressure>,
    /// The gamepad row; `None` (no pad source in this run) hides it.
    pub pad: Option<PadRow>,
    /// The frame's passes and light samples; `None` hides their two rows.
    pub lighting: Option<LightingRow>,
    /// The exposure; `None` hides its row.
    pub exposure: Option<ExposureRow>,
    /// The aliasing toggles' row ([`aliasing_row`]); `None` hides it.
    pub aliasing: Option<String>,
    /// The live queue's row ([`queue_row`]); `None` hides it.
    pub queue: Option<String>,
}

/// The live queue's row: the chunks in the view cone beyond the near shell still waiting for
/// their light, and of them those with a coarse block, the jobs withdrawn since the start, the
/// queue step's CPU p50 over the last second, then `mode` (the order and the withdrawal in
/// words) after a semicolon when it is not empty:
/// `in view: 12 waiting (5 coarse), withdrawn 3, step 1.25 ms; order distance; withdraw on`.
pub fn queue_row(st: &vxe_render::QueueStats, step_ms: f64, mode: &str) -> String {
    let mut row = String::new();
    write_queue_row(&mut row, st, step_ms);
    if !mode.is_empty() {
        row.push_str("; ");
        row.push_str(mode);
    }
    row
}

/// Appends [`queue_row`]'s text before its mode to `out`:
/// `in view: 12 waiting (5 coarse), withdrawn 3, step 1.25 ms`.
pub fn write_queue_row(out: &mut String, st: &vxe_render::QueueStats, step_ms: f64) {
    let [blank, coarse] = st.view_waiting[vxe_render::internals::VIEW_IN];
    let _ = write!(
        out,
        "in view: {} waiting ({coarse} coarse), withdrawn {}, step {step_ms:.2} ms",
        blank + coarse,
        st.withdrawn
    );
}

/// The exposure row: the mode in words, the exposure the next frame uses in EV (`log2 e`), the
/// metered scene in EV (log2 of its luminance; `None` before the first read-back or while the
/// meter holds), and in auto the exposure the meter settles at, in EV.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExposureRow {
    pub mode: String,
    pub ev: f32,
    pub scene_ev: Option<f32>,
    pub target_ev: Option<f32>,
}

impl fmt::Display for ExposureRow {
    /// `exposure manual +0.00 EV  scene -7.9 EV`, with `  settles +5.53 EV` in auto.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "exposure {} {:+.2} EV", self.mode, self.ev)?;
        match self.scene_ev {
            Some(v) => write!(f, "  scene {v:+.1} EV")?,
            None => f.write_str("  scene n/a")?,
        }
        if let Some(t) = self.target_ev {
            write!(f, "  settles {t:+.2} EV")?;
        }
        Ok(())
    }
}

/// The frame passes' GPU times (name and milliseconds, in pass order), the light samples the
/// mirror holds, the probe volume's live probes at the last sample (`None` with the lamp's
/// bounce off), the store's counts, and the lighting modes in words.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LightingRow {
    pub passes: Vec<(&'static str, f32)>,
    pub light_samples: u64,
    pub live_probes: Option<u32>,
    /// The store's counts at the last sample.
    pub store: Option<vxe_render::StoreStats>,
    /// The light queue's state in words (the fly-through's queue note), after the store's counts.
    pub store_note: String,
    /// The missing-light mode's name (ADR-0053), after the missing count with the pixels that
    /// took a fallback and those left black; empty, nothing.
    pub missing_light: &'static str,
    pub modes: String,
    /// The internal and output sizes while the upscaler runs: a row then reads its passes.
    pub upscale: Option<[[u32; 2]; 2]>,
}

/// The mirror's pressure (the fuller of its pool units and records), its reservation in bytes,
/// and while streaming the full radius the pressure rule holds with the configured one.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pressure {
    pub pressure: f32,
    pub budget_bytes: u64,
    pub r_full: Option<[i32; 2]>,
}

/// The cracks row: the field walk's leaks' share of the hit pixels, in percent:
/// `cracks leaks 0.31 %`.
pub fn cracks_row(c: &vxe_render::CrackShares) -> String {
    format!("cracks leaks {:.2} %", 100.0 * c.leak_share())
}

/// What the steps view read for one frame: the mean, p99 and block-max mean of steps per
/// ray, and the mean storage loads and pool loads per ray.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StepsSample {
    pub mean_steps: f32,
    pub p99_steps: u32,
    pub block_max_mean: f32,
    pub mean_loads: f32,
    pub mean_pool_reads: f32,
}

impl StepsSample {
    pub fn from_histogram(h: &vxe_render::debug::StepHistogram) -> Self {
        Self {
            mean_steps: h.mean_steps() as f32,
            p99_steps: h.p99_steps(),
            block_max_mean: h.block_max_mean() as f32,
            mean_loads: h.mean_loads() as f32,
            mean_pool_reads: h.mean_pool_reads() as f32,
        }
    }
}

impl HudStats {
    /// Hand-set constants, none of them measured, so the rows render the same on every run:
    /// the fixture of the golden image with the HUD drawn. The frame row is yellow (p99 above
    /// 16.67 ms) and so is the upload row (above 1 ms); the rest are white.
    pub fn fixed() -> Self {
        Self {
            fps: 58.8,
            frame_ms: FrameMs {
                mean: 17.0,
                p50: 16.5,
                p99: 24.25,
                max: 31.5,
            },
            cpu_ms: 4.125,
            pace: None,
            march_ms: Some(1.75),
            hud_ms: Some(0.0375),
            upload_bytes: 1_310_720,
            upload_ms: 1.5,
            upload_chunks: 3,
            mirror_bytes: 268_435_456,
            pool_live: 245_760,
            pool_free: 16_384,
            world: WorldSample {
                chunks: 512,
                lod_chunks: None,
                full_bricks: 245_760,
                pool_arrays: 0,
                cpu_bytes: 252_182_528,
                age_frames: 12,
                call_ms: 0.875,
            },
            rss_bytes: Some(734_003_200),
            last_edit: Some(EditLine {
                verb: "carved",
                counts: vec![("rock".to_owned(), 4169)],
                chunks: 2,
                ms: 0.25,
            }),
            camera: [64.5, 64.5, 64.5],
            chunk: [1, 1, 1],
            mode: Mode::Fly,
            cutaway: None,
            format_us: 6,
            steps: None,
            cracks: None,
            pressure: None,
            pad: None,
            lighting: None,
            exposure: None,
            aliasing: None,
            queue: None,
        }
    }

    /// Writes the rows into `out`, one per line, and each row's colour into `colours`: yellow
    /// for a time row above its `nfr.md` line, red for a frame over the window longer than two
    /// 60 Hz frames. Both are cleared first.
    pub fn write_rows(&self, out: &mut String, colours: &mut Vec<HudColour>) {
        out.clear();
        colours.clear();
        // `fmt::Write` for `String` never fails.
        let _ = self.rows(out, colours);
    }

    fn rows(&self, out: &mut String, colours: &mut Vec<HudColour>) -> fmt::Result {
        let mut row = |out: &mut String, c: HudColour, args: fmt::Arguments<'_>| {
            colours.push(c);
            out.write_fmt(args)?;
            out.write_char('\n')
        };
        let over = |v: f32, line: f32| {
            if v > line {
                HudColour::Yellow
            } else {
                HudColour::White
            }
        };
        let f = self.frame_ms;
        let frame_colour = if f.max > 2.0 * FRAME_LINE_MS {
            HudColour::Red
        } else {
            over(f.p99, FRAME_LINE_MS)
        };
        row(
            out,
            frame_colour,
            format_args!(
                "fps {:.1}  frame mean {:.2} p50 {:.2} p99 {:.2} max {:.2} ms ({RING} frames)",
                self.fps, f.mean, f.p50, f.p99, f.max
            ),
        )?;
        row(
            out,
            over(self.cpu_ms, FRAME_LINE_MS),
            format_args!("cpu {:.3} ms", self.cpu_ms),
        )?;
        if let Some(pace) = self.pace {
            // The frame window's median wall time beside this frame's CPU and GPU: with the
            // compositor pacing, wall near their sum; free, near the larger of the two.
            write!(
                out,
                "pace {pace}  wall {:.2} / cpu {:.2} / gpu ",
                f.p50, self.cpu_ms
            )?;
            match self.march_ms {
                Some(g) => write!(out, "{g:.2} ms")?,
                None => out.write_str("n/a ms")?,
            }
            row(out, over(f.p50, FRAME_LINE_MS), format_args!(""))?;
        }
        row(
            out,
            self.march_ms
                .map_or(HudColour::White, |v| over(v, MARCH_LINE_MS)),
            format_args!("gpu march {}  hud {}", Ms(self.march_ms), Ms(self.hud_ms)),
        )?;
        if let Some(l) = &self.lighting {
            // Yellow when the lighting passes (light, temporal and the probe update and gather)
            // exceed the lighting line or the filter the denoise line.
            let ms = |name: &str| {
                l.passes
                    .iter()
                    .filter(|(n, _)| *n == name)
                    .map(|&(_, v)| v)
                    .sum::<f32>()
            };
            let colour = if ms("light") + ms("temporal") + ms("probes") + ms("gather")
                > LIGHTING_LINE_MS
                || ms("atrous") > DENOISE_LINE_MS
            {
                HudColour::Yellow
            } else {
                HudColour::White
            };
            out.write_str("gpu")?;
            // FSR 2's passes are summed on the upscale row instead.
            for (name, v) in l.passes.iter().filter(|(n, _)| !n.starts_with("fsr2 ")) {
                write!(out, " {name} {v:.2}")?;
            }
            out.write_str(" ms")?;
            row(out, colour, format_args!(""))?;
            // Written piece by piece into the rows' buffer: nothing allocated.
            write!(out, "lights {} samples", l.light_samples)?;
            if let Some(p) = l.live_probes {
                write!(out, ", {p} probes live")?;
            }
            if let Some(s) = l.store {
                write!(
                    out,
                    ", store {} {:.1} MB missing {}",
                    s.chunks,
                    s.bytes as f64 / 1e6,
                    s.missing
                )?;
                if !l.missing_light.is_empty() {
                    write!(
                        out,
                        " (light {}: fallback {} black {})",
                        l.missing_light, s.fallback, s.still_black
                    )?;
                }
                if s.brick_reads > 0 {
                    write!(out, " brick level {}", s.brick_reads)?;
                }
                out.write_str(&l.store_note)?;
            }
            row(out, HudColour::White, format_args!("; {}", l.modes))?;
            if let Some([[iw, ih], [ow, oh]]) = l.upscale {
                // Yellow when the composite, upscale and sharpen passes and FSR 2's exceed the
                // upscale line. FSR 2's passes, when they ran, read as one total.
                let fsr: f32 = l
                    .passes
                    .iter()
                    .filter(|(n, _)| n.starts_with("fsr2 "))
                    .map(|&(_, v)| v)
                    .sum();
                let post = ms("composite") + ms("upscale") + ms("sharpen") + fsr;
                if fsr > 0.0 {
                    write!(out, "fsr2 {fsr:.2} ms")?;
                } else {
                    write!(
                        out,
                        "upscale {:.2} sharpen {:.2} ms",
                        ms("upscale"),
                        ms("sharpen")
                    )?;
                }
                row(
                    out,
                    over(post, UPSCALE_LINE_MS),
                    format_args!(", {iw}x{ih} to {ow}x{oh}"),
                )?;
            }
        }
        if let Some(e) = &self.exposure {
            row(out, HudColour::White, format_args!("{e}"))?;
        }
        if let Some(a) = &self.aliasing {
            row(out, HudColour::White, format_args!("{a}"))?;
        }
        if let Some(q) = &self.queue {
            row(out, HudColour::White, format_args!("{q}"))?;
        }
        row(
            out,
            over(self.upload_ms, UPLOAD_LINE_MS),
            format_args!(
                "upload {} B, {} chunks, {:.3} ms",
                self.upload_bytes, self.upload_chunks, self.upload_ms
            ),
        )?;
        row(
            out,
            HudColour::White,
            format_args!(
                "mirror {} B live, gpu pool {} live / {} free slots",
                self.mirror_bytes, self.pool_live, self.pool_free
            ),
        )?;
        if let Some(p) = self.pressure {
            let colour = over(p.pressure, PRESSURE_HIGH);
            match p.r_full {
                Some([r, max]) => row(
                    out,
                    colour,
                    format_args!(
                        "mirror pressure {:.3} of {} B, r_full {r} of {max}",
                        p.pressure, p.budget_bytes
                    ),
                )?,
                None => row(
                    out,
                    colour,
                    format_args!("mirror pressure {:.3} of {} B", p.pressure, p.budget_bytes),
                )?,
            }
        }
        let w = self.world;
        match w.lod_chunks {
            Some(lod) => row(
                out,
                HudColour::White,
                format_args!(
                    "world {} full + {lod} lod chunks, {} full bricks",
                    w.chunks, w.full_bricks
                ),
            )?,
            None => row(
                out,
                HudColour::White,
                format_args!("world {} chunks, {} full bricks", w.chunks, w.full_bricks),
            )?,
        }
        row(
            out,
            HudColour::White,
            format_args!("cpu bytes {} (estimate)", w.cpu_bytes),
        )?;
        match self.rss_bytes {
            Some(b) => row(out, HudColour::White, format_args!("rss {b} B"))?,
            None => row(out, HudColour::White, format_args!("rss n/a"))?,
        }
        row(
            out,
            HudColour::White,
            format_args!(
                "world sample {} frames old, call {:.3} ms",
                w.age_frames, w.call_ms
            ),
        )?;
        match &self.last_edit {
            Some(e) => row(out, HudColour::White, format_args!("{e}"))?,
            None => row(out, HudColour::White, format_args!("no edit yet"))?,
        }
        let [x, y, z] = self.camera;
        let [cx, cy, cz] = self.chunk;
        let mode = match self.mode {
            Mode::Fly => "fly",
            Mode::Walk => "walk",
        };
        row(
            out,
            HudColour::White,
            format_args!("{mode} at {x:.1} {y:.1} {z:.1} (voxels), chunk {cx} {cy} {cz}"),
        )?;
        if let Some(c) = self.cutaway {
            row(out, HudColour::White, format_args!("cutaway ({c})"))?;
        }
        if let Some(s) = self.steps {
            row(
                out,
                HudColour::White,
                format_args!(
                    "steps per ray mean {:.2} p99 {} block-max mean {:.2}; loads mean {:.2}, pool {:.2}",
                    s.mean_steps, s.p99_steps, s.block_max_mean, s.mean_loads, s.mean_pool_reads
                ),
            )?;
        }
        if let Some(c) = self.cracks {
            row(out, HudColour::White, format_args!("{}", cracks_row(&c)))?;
        }
        if let Some(p) = &self.pad {
            row(out, HudColour::White, format_args!("{p}"))?;
        }
        row(
            out,
            HudColour::White,
            format_args!("format {} us", self.format_us),
        )
    }
}

impl fmt::Display for HudStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = String::new();
        self.write_rows(&mut out, &mut Vec::new());
        f.write_str(out.trim_end_matches('\n'))
    }
}

/// The process's memory counters (`GetProcessMemoryInfo`); `None` when the call fails.
#[cfg(windows)]
pub fn process_memory() -> Option<windows_sys::Win32::System::ProcessStatus::PROCESS_MEMORY_COUNTERS>
{
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let size = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    // SAFETY: the counters are zeroed with their size given, which bounds what the call
    // writes; `GetCurrentProcess` is a pseudo-handle that needs no closing.
    unsafe {
        let mut m: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        m.cb = size;
        (GetProcessMemoryInfo(GetCurrentProcess(), &mut m, size) != 0).then_some(m)
    }
}

/// The process's resident set in bytes: on Windows the working set (`process_memory`), the
/// page size unused.
#[cfg(windows)]
pub fn rss_bytes(_page_bytes: u64) -> Option<u64> {
    process_memory().map(|m| m.WorkingSetSize as u64)
}

/// The process's resident set in bytes: the second field of `/proc/self/statm` (pages) times
/// `page_bytes`. `None` where the file does not exist.
#[cfg(not(windows))]
pub fn rss_bytes(page_bytes: u64) -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    Some(pages * page_bytes)
}

/// The page size from the process's auxiliary vector (`AT_PAGESZ`, read from
/// `/proc/self/auxv` as native-endian 64-bit pairs); 4096 where it cannot be read.
pub fn page_bytes() -> u64 {
    const AT_PAGESZ: u64 = 6;
    let Ok(auxv) = std::fs::read("/proc/self/auxv") else {
        return 4096;
    };
    auxv.chunks_exact(16)
        .map(|p| {
            let word = |b: &[u8]| u64::from_ne_bytes(b.try_into().unwrap_or([0; 8]));
            (word(&p[..8]), word(&p[8..]))
        })
        .find(|&(k, _)| k == AT_PAGESZ)
        .map_or(4096, |(_, v)| v)
}

#[cfg(test)]
mod tests;
