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
    /// An estimate: [`cpu_bytes_estimate`] of the counts above.
    pub cpu_bytes: u64,
    /// Frames since the sample was taken.
    pub age_frames: u32,
    /// What the sampling call cost, in milliseconds.
    pub call_ms: f32,
}

impl WorldSample {
    pub fn from_stats(s: &WorldStats, lod_chunks: Option<u32>, call_ms: f32) -> Self {
        Self {
            chunks: s.chunks as u32,
            lod_chunks,
            full_bricks: s.full_bricks as u64,
            cpu_bytes: cpu_bytes_estimate(s.chunks as u64, s.full_bricks as u64),
            age_frames: 0,
            call_ms,
        }
    }
}

/// RAM the resident full chunks hold, estimated from counts alone: each `Full` brick's voxel
/// array plus each chunk's `Chunk`. Uniform bricks live inside `Chunk`.
pub fn cpu_bytes_estimate(chunks: u64, full_bricks: u64) -> u64 {
    full_bricks * (BRICK_VOXELS * size_of::<MaterialId>()) as u64
        + chunks * size_of::<Chunk>() as u64
}

/// The movement mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Fly,
    Walk,
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
    pub pool_live: u32,
    pub pool_free: u32,
    pub world: WorldSample,
    /// Process RSS, read with the world sample; `None` off Linux.
    pub rss_bytes: Option<u64>,
    pub last_edit: Option<EditLine>,
    pub camera: [f32; 3],
    pub chunk: [i32; 3],
    pub mode: Mode,
    /// What formatting the previous rows cost, in microseconds.
    pub format_us: u32,
}

impl HudStats {
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
        let ms = |v: Option<f32>| v.map_or_else(|| "n/a".to_owned(), |v| format!("{v:.3} ms"));
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
            format_args!("gpu march {}  hud {}", ms(self.march_ms), ms(self.hud_ms)),
        )?;
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
                "mirror {} B live, pool {} live / {} free slots",
                self.mirror_bytes, self.pool_live, self.pool_free
            ),
        )?;
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
    fn cpu_bytes_counts_full_brick_voxels_and_chunks() {
        assert_eq!(cpu_bytes_estimate(0, 1), 1024);
        assert_eq!(
            cpu_bytes_estimate(2, 3),
            3 * 1024 + 2 * size_of::<Chunk>() as u64
        );
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

    #[test]
    fn rss_reads_on_linux() {
        if cfg!(target_os = "linux") {
            let page = page_bytes();
            assert!(page.is_power_of_two() && page >= 4096, "page {page}");
            assert!(rss_bytes(page).is_some_and(|b| b > 0));
        }
    }
}
