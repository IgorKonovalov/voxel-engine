//! The fly-through's stats HUD: what it reads each frame, the cadence at which it samples the
//! world, and the rows it prints, on screen through `vxe_render::HudOverlay` and at exit as
//! text. The overlay draws whatever rows it is handed; the rows here are the testbed's.
//!
//! Per-frame fields come only from sources that cost nothing to read: the [`FrameStats`]
//! ring, the pass timers, the last mirror sync, the mirror's counters, the last edit and the
//! camera. The world's chunk and brick counts walk every resident chunk, and reading the
//! process's RSS from `/proc` is not free either, so both are sampled at a [`Cadence`] and
//! shown with the sample's age.

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
    /// The mirror's pressure against its reservation; `None` hides its row.
    pub pressure: Option<Pressure>,
    /// The gamepad row; `None` (no pad source in this run) hides it.
    pub pad: Option<PadRow>,
    /// The frame's passes and light samples; `None` hides their two rows.
    pub lighting: Option<LightingRow>,
}

/// The frame passes' GPU times (name and milliseconds, in pass order), the light samples the
/// mirror holds, and the lighting modes in words.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LightingRow {
    pub passes: Vec<(&'static str, f32)>,
    pub light_samples: u64,
    pub modes: String,
}

/// The mirror's pressure (the fuller of its pool units and records), its reservation in bytes,
/// and while streaming the full radius the pressure rule holds with the configured one.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pressure {
    pub pressure: f32,
    pub budget_bytes: u64,
    pub r_full: Option<[i32; 2]>,
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
    pub fn from_histogram(h: &vxe_render::readback::StepHistogram) -> Self {
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
            pressure: None,
            pad: None,
            lighting: None,
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
        row(
            out,
            self.march_ms
                .map_or(HudColour::White, |v| over(v, MARCH_LINE_MS)),
            format_args!("gpu march {}  hud {}", Ms(self.march_ms), Ms(self.hud_ms)),
        )?;
        if let Some(l) = &self.lighting {
            // Yellow when light and temporal exceed the lighting line or the filter the
            // denoise line.
            let ms = |name: &str| {
                l.passes
                    .iter()
                    .filter(|(n, _)| *n == name)
                    .map(|&(_, v)| v)
                    .sum::<f32>()
            };
            let colour = if ms("light") + ms("temporal") > LIGHTING_LINE_MS
                || ms("atrous") > DENOISE_LINE_MS
            {
                HudColour::Yellow
            } else {
                HudColour::White
            };
            out.write_str("gpu")?;
            for (name, v) in &l.passes {
                write!(out, " {name} {v:.2}")?;
            }
            out.write_str(" ms")?;
            row(out, colour, format_args!(""))?;
            row(
                out,
                HudColour::White,
                format_args!("lights {} samples; {}", l.light_samples, l.modes),
            )?;
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

/// The process's resident set in bytes: the second field of `/proc/self/statm` (pages) times
/// `page_bytes`. `None` where the file does not exist.
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
mod tests {
    use super::*;

    fn fed(ms: impl IntoIterator<Item = f32>) -> FrameStats {
        let mut s = FrameStats::new();
        for v in ms {
            s.record(v / 1000.0);
        }
        s
    }

    #[test]
    fn equal_deltas_read_as_mean_median_and_p99() {
        let s = fed([5.0; RING]);
        let f = s.frame_ms();
        assert!((f.mean - 5.0).abs() < 1e-3, "mean {}", f.mean);
        assert!((f.p50 - 5.0).abs() < 1e-4, "p50 {}", f.p50);
        assert!((f.p99 - 5.0).abs() < 1e-4, "p99 {}", f.p99);
        assert!((s.fps() - 200.0).abs() < 0.1, "fps {}", s.fps());
    }

    /// One spike in a full window: index `round(0.99 x 239) = 237` of the sorted window is
    /// still a 1 ms sample, so the spike shows in the max and not in the p99.
    #[test]
    fn one_spike_among_239_reads_in_the_max_not_the_p99() {
        let s = fed(std::iter::once(100.0).chain([1.0; RING - 1]));
        let f = s.frame_ms();
        assert!((f.p50 - 1.0).abs() < 1e-4, "p50 {}", f.p50);
        assert!((f.p99 - 1.0).abs() < 1e-4, "p99 {}", f.p99);
        assert!((f.max - 100.0).abs() < 1e-3, "max {}", f.max);
    }

    /// Three spikes in a full window sort to indices 237 to 239, so the p99 index
    /// `round(0.99 x 239) = 237` holds a spike; the median index `round(0.5 x 239) = 120`
    /// still holds 1 ms. Two spikes would leave index 237 at 1 ms.
    #[test]
    fn three_spikes_among_237_read_in_the_p99() {
        let s = fed([100.0; 3].into_iter().chain([1.0; RING - 3]));
        let f = s.frame_ms();
        assert!((f.p99 - 100.0).abs() < 1e-3, "p99 {}", f.p99);
        assert!((f.p50 - 1.0).abs() < 1e-4, "p50 {}", f.p50);
        assert!((f.max - 100.0).abs() < 1e-3, "max {}", f.max);
    }

    #[test]
    fn a_known_sequence_reads_nearest_rank() {
        let s = fed((1..=100).map(|v| v as f32));
        assert_eq!(s.frames_total(), 100);
        assert!((s.fps() - 19.801_98).abs() < 1e-2, "fps {}", s.fps());
        assert!((s.frame_ms_avg() - 50.5).abs() < 1e-3);
        // Sorted 1..=100: index round(0.99 x 99) = 98 holds 99 ms.
        assert!((s.frame_ms_p99() - 99.0).abs() < 1e-3);
    }

    #[test]
    fn an_empty_window_reads_zero() {
        let s = FrameStats::new();
        assert_eq!(s.fps(), 0.0);
        assert_eq!(s.frame_ms(), FrameMs::default());
        assert_eq!(s.samples().count(), 0);
    }

    #[test]
    fn samples_are_chronological_after_wrap() {
        let mut s = FrameStats::new();
        for i in 0..(RING + 10) {
            s.record(i as f32);
        }
        let got: Vec<f32> = s.samples().collect();
        assert_eq!(got.len(), RING);
        assert_eq!(got.first().copied(), Some(10.0));
        assert_eq!(got.last().copied(), Some((RING + 9) as f32));
        assert!(got.windows(2).all(|w| w[1] == w[0] + 1.0));
    }

    /// The cadence driving a stub counter: no call while hidden, one on turning visible, then
    /// one every `every`th visible frame.
    #[test]
    fn the_world_is_sampled_on_toggle_and_every_nth_visible_frame() {
        for every in [60, 240] {
            let mut cadence = Cadence::new(every);
            let mut calls = 0u32;
            let mut frame = |visible: bool, calls: &mut u32| {
                if cadence.due(visible) {
                    *calls += 1;
                }
            };
            for _ in 0..300 {
                frame(false, &mut calls);
            }
            assert_eq!(calls, 0, "every {every}: hidden frames sample nothing");
            frame(true, &mut calls);
            assert_eq!(calls, 1, "every {every}: turning visible samples once");
            for _ in 0..120 {
                frame(true, &mut calls);
            }
            assert_eq!(calls, 1 + 120 / every, "every {every}: 120 visible frames");
        }
    }

    #[test]
    fn hiding_and_showing_again_samples_on_the_toggle() {
        let mut c = Cadence::new(60);
        assert!(c.due(true));
        assert!(!c.due(true));
        assert_eq!(c.age(), 1);
        assert!(!c.due(false));
        assert!(c.due(true));
        assert_eq!(c.age(), 0);
    }

    #[test]
    fn cpu_bytes_counts_full_brick_voxels_and_chunks_and_the_pool() {
        assert_eq!(cpu_bytes_estimate(0, 1, 0), 1024);
        let old = 3 * 1024 + 2 * size_of::<Chunk>() as u64;
        assert_eq!(cpu_bytes_estimate(2, 3, 0), old);
        assert_eq!(cpu_bytes_estimate(2, 3, 4096), old + 4_194_304);
    }

    #[test]
    fn an_edit_line_reads_like_the_printed_one() {
        let e = EditLine {
            verb: "carved",
            counts: vec![("rock".to_owned(), 1234), ("ore".to_owned(), 12)],
            chunks: 3,
            ms: 0.214,
        };
        assert_eq!(
            e.to_string(),
            "carved: rock 1234, ore 12 (3 chunks, 0.21 ms)"
        );
        let none = EditLine {
            verb: "filled",
            ..EditLine::default()
        };
        assert_eq!(none.to_string(), "filled: nothing (0 chunks, 0.00 ms)");
    }

    #[test]
    fn a_row_over_its_line_is_yellow_and_one_per_line() {
        let stats = HudStats {
            frame_ms: FrameMs {
                mean: 10.0,
                p50: 10.0,
                p99: 20.0,
                max: 20.0,
            },
            march_ms: Some(1.0),
            upload_ms: 2.0,
            ..HudStats::default()
        };
        let (mut out, mut colours) = (String::new(), Vec::new());
        stats.write_rows(&mut out, &mut colours);
        assert_eq!(out.lines().count(), colours.len());
        assert_eq!(colours[0], HudColour::Yellow, "frame p99 over 16.67 ms");
        assert_eq!(colours[2], HudColour::White, "march under 3 ms");
        assert_eq!(colours[3], HudColour::Yellow, "upload over 1 ms");
        assert!(out.lines().any(|l| l == "cpu bytes 0 (estimate)"));
        assert_eq!(stats.to_string(), out.trim_end());
        let spike = HudStats {
            frame_ms: FrameMs {
                max: 40.0,
                ..FrameMs::default()
            },
            ..HudStats::default()
        };
        spike.write_rows(&mut out, &mut colours);
        assert_eq!(colours[0], HudColour::Red, "a frame over 33.3 ms");
    }

    /// The steps row appears only with a steps sample, before the format row, and reads the
    /// sample's five figures.
    #[test]
    fn the_steps_row_prints_the_last_frame_s_sample() {
        let stats = HudStats {
            steps: Some(StepsSample {
                mean_steps: 15.55,
                p99_steps: 84,
                block_max_mean: 31.2,
                mean_loads: 12.4,
                mean_pool_reads: 2.45,
            }),
            ..HudStats::default()
        };
        let lines = stats.to_string();
        let lines: Vec<&str> = lines.lines().collect();
        assert_eq!(
            lines[lines.len() - 2],
            "steps per ray mean 15.55 p99 84 block-max mean 31.20; loads mean 12.40, pool 2.45"
        );
        assert!(lines[lines.len() - 1].starts_with("format "));
        assert!(
            !HudStats::default()
                .to_string()
                .lines()
                .any(|l| l.starts_with("steps per ray")),
            "no row without a sample"
        );
    }

    /// The cutaway row follows the camera row while the view is on, and is absent otherwise.
    #[test]
    fn the_cutaway_row_shows_while_the_view_is_on() {
        let stats = HudStats {
            cutaway: Some("auto"),
            ..HudStats::default()
        };
        let text = stats.to_string();
        let lines: Vec<&str> = text.lines().collect();
        let i = lines
            .iter()
            .position(|l| l.starts_with("fly at "))
            .expect("the camera row");
        assert_eq!(lines[i + 1], "cutaway (auto)");
        assert!(
            !HudStats::default()
                .to_string()
                .lines()
                .any(|l| l.starts_with("cutaway")),
            "no row while the view is off"
        );
    }

    /// The pressure row follows the mirror row when there is a reading: yellow past 0.9, and
    /// with the full radius while streaming.
    #[test]
    fn a_pressure_reading_adds_its_row() {
        let (mut out, mut colours) = (String::new(), Vec::new());
        let mut s = HudStats::fixed();
        s.pressure = Some(Pressure {
            pressure: 0.921_875,
            budget_bytes: 3 << 30,
            r_full: Some([15, 16]),
        });
        s.write_rows(&mut out, &mut colours);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 13);
        assert_eq!(
            lines[5],
            "mirror pressure 0.922 of 3221225472 B, r_full 15 of 16"
        );
        assert_eq!(colours[5], HudColour::Yellow);
        s.pressure = Some(Pressure {
            pressure: 0.25,
            budget_bytes: 1 << 20,
            r_full: None,
        });
        s.write_rows(&mut out, &mut colours);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[5], "mirror pressure 0.250 of 1048576 B");
        assert_eq!(colours[5], HudColour::White);
    }

    /// The golden's fixture: twelve rows, the frame and upload rows yellow, the rest white.
    #[test]
    fn fixed_stats_format_to_their_rows() {
        let (mut out, mut colours) = (String::new(), Vec::new());
        HudStats::fixed().write_rows(&mut out, &mut colours);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 12);
        assert_eq!(
            lines[0],
            "fps 58.8  frame mean 17.00 p50 16.50 p99 24.25 max 31.50 ms (240 frames)"
        );
        assert_eq!(lines[2], "gpu march 1.750 ms  hud 0.038 ms");
        assert_eq!(lines[3], "upload 1310720 B, 3 chunks, 1.500 ms");
        assert_eq!(
            lines[4],
            "mirror 268435456 B live, gpu pool 245760 live / 16384 free slots"
        );
        assert_eq!(lines[9], "carved: rock 4169 (2 chunks, 0.25 ms)");
        let yellow: Vec<usize> = (0..colours.len())
            .filter(|&i| colours[i] != HudColour::White)
            .collect();
        assert_eq!(yellow, [0, 3]);
        assert!(colours.iter().all(|&c| c != HudColour::Red));
    }

    /// The lighting rows follow the march row: every pass's time, yellow when light and
    /// temporal pass the 6.0 ms line or the filter the 3.0 ms one, then the light samples and
    /// the modes.
    #[test]
    fn the_lighting_rows_follow_the_march_row() {
        let (mut out, mut colours) = (String::new(), Vec::new());
        let mut s = HudStats::fixed();
        s.lighting = Some(LightingRow {
            passes: vec![
                ("hit", 1.25),
                ("light", 4.5),
                ("temporal", 0.5),
                ("atrous", 2.5),
            ],
            light_samples: 18_522,
            modes: "lighting full".to_owned(),
        });
        s.write_rows(&mut out, &mut colours);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 14);
        assert_eq!(
            lines[3],
            "gpu hit 1.25 light 4.50 temporal 0.50 atrous 2.50 ms"
        );
        assert_eq!(lines[4], "lights 18522 samples; lighting full");
        assert_eq!(colours[3], HudColour::White);
        s.lighting.as_mut().unwrap().passes[1].1 = 6.0;
        s.write_rows(&mut out, &mut colours);
        assert_eq!(
            colours[3],
            HudColour::Yellow,
            "light + temporal over 6.0 ms"
        );
    }

    /// A pad row sits before the format row, and a run without a pad source shows none.
    #[test]
    fn the_pad_row_shows_before_the_format_row() {
        let (mut out, mut colours) = (String::new(), Vec::new());
        let mut s = HudStats::fixed();
        s.pad = Some(PadRow {
            name: "PS5 Controller".to_owned(),
            power: Some(PadPower::Charging(5)),
        });
        s.write_rows(&mut out, &mut colours);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 13);
        assert_eq!(lines[11], "pad PS5 Controller, charging 5%");
        assert_eq!(lines[12], "format 6 us");
        assert_eq!(colours[11], HudColour::White);
    }

    #[test]
    fn rss_reads_on_linux() {
        if cfg!(target_os = "linux") {
            let page = page_bytes();
            assert!(page.is_power_of_two() && page >= 4096, "page {page}");
            assert!(rss_bytes(page).is_some_and(|b| b > 0));
        }
    }
}

/// How long the mode banner shows after the start or a change, in milliseconds.
pub const BANNER_MS: f64 = 3000.0;
/// The banner's widest row in characters: it fits a 1280-pixel window at the HUD's scale 2
/// (80 cells) and the golden's 320 pixels at scale 1 (40 cells).
pub const BANNER_COLS: usize = 40;
/// Rows the banner wraps into at most.
pub const BANNER_ROWS: usize = 6;

/// A key that changes the lighting mode: `F3` to `F7` and `M`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeKey {
    /// `F3`: full or coarse shadow walk.
    ShadowWalk,
    /// `F4`: the flat list or reservoir reuse.
    Sampler,
    /// `F5`: edit boxes, or edit boxes and the gradient.
    History,
    /// `F6`: full or half lighting resolution.
    Resolution,
    /// `F7`: the headlamp.
    Lamp,
    /// `M`: the ore's emission pulse.
    Pulse,
}

impl ModeKey {
    pub const ALL: [Self; 6] = [
        Self::ShadowWalk,
        Self::Sampler,
        Self::History,
        Self::Resolution,
        Self::Lamp,
        Self::Pulse,
    ];
}

/// The lighting modes the window's keys switch, and the banner that names them: shown for
/// [`BANNER_MS`] after the start and after every change, by the frame loop's clock (`now_ms`),
/// so a test drives it without a window.
#[derive(Clone, Debug, PartialEq)]
pub struct Modes {
    pub lighting: vxe_render::Lighting,
    pub resolution: vxe_render::LightingResolution,
    /// The ore's emission scale pulses.
    pub pulse: bool,
    banner_at: f64,
}

impl Modes {
    /// The modes at the start, the banner shown from `now_ms`.
    pub fn new(
        lighting: vxe_render::Lighting,
        resolution: vxe_render::LightingResolution,
        now_ms: f64,
    ) -> Self {
        Self {
            lighting,
            resolution,
            pulse: false,
            banner_at: now_ms,
        }
    }

    /// Switches the mode `key` changes and shows the banner from `now_ms`.
    pub fn press(&mut self, key: ModeKey, now_ms: f64) {
        use vxe_render::{HistoryKill, LightingResolution, Sampler, ShadowWalk};
        let l = &mut self.lighting;
        match key {
            ModeKey::ShadowWalk => {
                l.shadow_walk = match l.shadow_walk {
                    ShadowWalk::Full => ShadowWalk::Coarse,
                    ShadowWalk::Coarse => ShadowWalk::Full,
                }
            }
            ModeKey::Sampler => {
                l.sampler = match l.sampler {
                    Sampler::List => Sampler::Reuse,
                    Sampler::Reuse => Sampler::List,
                }
            }
            ModeKey::History => {
                l.history = match l.history {
                    HistoryKill::Edits => HistoryKill::Gradient,
                    HistoryKill::Gradient => HistoryKill::Edits,
                }
            }
            ModeKey::Resolution => {
                self.resolution = match self.resolution {
                    LightingResolution::Full => LightingResolution::Half,
                    LightingResolution::Half => LightingResolution::Full,
                }
            }
            ModeKey::Lamp => l.lamp = !l.lamp,
            ModeKey::Pulse => self.pulse = !self.pulse,
        }
        self.banner_at = now_ms;
    }

    /// The modes in words: `lighting half, shadow walk coarse, sampler reuse, history edits,
    /// lamp on, filter on, ore pulse off, emissive history 32`.
    pub fn line(&self) -> String {
        let l = &self.lighting;
        let on = |b: bool| if b { "on" } else { "off" };
        format!(
            "lighting {}, shadow walk {}, sampler {}, history {}, lamp {}, filter {}, ore pulse {}, emissive history {}",
            self.resolution.name(),
            l.shadow_walk.name(),
            l.sampler.name(),
            l.history.name(),
            on(l.lamp),
            on(l.filter),
            on(self.pulse),
            l.emissive_history
        )
    }

    /// The banner at `now_ms`: the modes' line within [`BANNER_MS`] of the start or the last
    /// change, wrapped at its commas into rows of at most [`BANNER_COLS`] characters, one per
    /// text line; else `None`.
    pub fn banner(&self, now_ms: f64) -> Option<String> {
        (now_ms - self.banner_at < BANNER_MS).then(|| {
            let mut out = String::new();
            let mut row = String::new();
            for part in self.line().split(", ") {
                if !row.is_empty() && row.len() + 2 + part.len() > BANNER_COLS {
                    out.push_str(&row);
                    out.push('\n');
                    row.clear();
                }
                if !row.is_empty() {
                    row.push_str(", ");
                }
                row.push_str(part);
            }
            out.push_str(&row);
            out
        })
    }
}

#[cfg(test)]
mod mode_tests {
    use super::*;

    /// Each of `F3` to `F7` and `M` changes the banner's text to the new mode; the banner is
    /// empty 3 s after the last change and shows again on the next one.
    #[test]
    fn the_banner_names_each_change_and_fades_after_three_seconds() {
        let mut m = Modes::new(
            vxe_render::Lighting::default(),
            vxe_render::LightingResolution::default(),
            0.0,
        );
        let start = m.banner(0.0).expect("the banner shows at the start");
        assert_eq!(
            start,
            "lighting half, shadow walk coarse\nsampler reuse, history edits, lamp on\nfilter on, ore pulse off\nemissive history 32"
        );
        assert!(start.lines().all(|l| l.len() <= BANNER_COLS));
        let mut t = 1000.0;
        let mut last = start;
        for (key, word) in [
            (ModeKey::ShadowWalk, "shadow walk full"),
            (ModeKey::Sampler, "sampler list"),
            (ModeKey::History, "history gradient"),
            (ModeKey::Resolution, "lighting full"),
            (ModeKey::Lamp, "lamp off"),
            (ModeKey::Pulse, "ore pulse on"),
        ] {
            m.press(key, t);
            let now = m.banner(t).expect("a change shows the banner");
            assert_ne!(now, last, "{key:?} changes the text");
            assert!(now.contains(word), "{key:?}: {now}");
            last = now;
            t += 500.0;
        }
        let changed = t - 500.0;
        assert!(m.banner(changed + BANNER_MS - 1.0).is_some());
        assert_eq!(
            m.banner(changed + BANNER_MS),
            None,
            "empty 3 s after the last change"
        );
        m.press(ModeKey::Lamp, changed + 10_000.0);
        let again = m
            .banner(changed + 10_000.0)
            .expect("the next change shows it again");
        assert!(again.contains("lamp on"), "{again}");
    }
}
