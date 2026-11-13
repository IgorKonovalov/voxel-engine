//! What the run feeds the stats HUD and the banner: the samples, the rows and the lines.

use crate::app::ms_since;
use crate::audio::room_line;
use crate::cli::Args;
use crate::edits::{HIT_FOUND_BIT, voxel_of};
use crate::hud::{self, Cadence, FrameStats, HudStats, WorldSample};
use crate::now;
use crate::report::Series;
use crate::run::Run;
use crate::setup::GfxSample;
use crate::{LookPreset, percentile};
use std::fmt::Write as _;
use vxe_app::{Gpu, Pacing};
use vxe_render::debug::StepHistogram;
use vxe_render::{
    BELT_CHUNK_BYTES, FRAME_PASSES, GpuWorld, HudColour, LookParams, NormalMode, ShadowWalk,
    SyncStats,
};

/// Frames `F1` averages over.
pub(crate) const SAMPLES: usize = 30;

/// The stats HUD's glyph scale: an 8 x 8 glyph covers 16 x 16 pixels.
pub(crate) const HUD_SCALE: u32 = 2;

/// Visible frames between two samples of the world's counts; see [`Cadence`]. One sample per
/// frame-time window, because one `World::stats` call on the streamed cave at full residency
/// costs more than the 1 ms streaming line (measured by the ignored test
/// `world_stats_call_on_the_streamed_world_at_full_residency`), so its frame reaches at most
/// the window's max and not its p99.
pub(crate) const HUD_STATS_EVERY: u32 = 240;

/// While the exposure moves by more than 0.01 EV per half second, prints the exposure row every
/// half second: `last` is when it was last checked (ms since the start) and its EV.
pub(crate) fn print_moving_exposure(
    last: &mut Option<(f64, f32)>,
    modes: &hud::Modes,
    readout: Option<vxe_render::ExposureReadout>,
    now_ms: f64,
) {
    let Some(r) = readout else { return };
    let ev = r.exposure.log2();
    match *last {
        Some((at, _)) if now_ms - at < 500.0 => {}
        Some((_, was)) => {
            if (ev - was).abs() > 0.01 {
                println!("{}", modes.exposure_row());
            }
            *last = Some((now_ms, ev));
        }
        None => *last = Some((now_ms, ev)),
    }
}

/// The probe volume's bytes inside `Frame::frame_bytes`, by part.
pub(crate) fn probe_bytes_line(p: &vxe_render::ProbeVolume) -> String {
    format!(
        "probe volume {} B in frame_bytes (atlases {} B, state {} B, update list, rays and uniform {} B), spacing {} voxels",
        p.bytes(),
        p.atlas_bytes(),
        p.state_bytes(),
        p.bytes() - p.atlas_bytes() - p.state_bytes(),
        p.spacing()
    )
}

/// The probe update and gather passes' GPU time in a frame's `FRAME_PASSES` times.
pub(crate) fn probe_passes_ms(passes: &[f64]) -> f64 {
    FRAME_PASSES
        .iter()
        .zip(passes)
        .filter(|(name, _)| matches!(**name, "probes" | "gather"))
        .map(|(_, ms)| ms)
        .sum()
}

/// `--lamp-toggle`'s exit line: the probes plus gather median of the last [`SAMPLES`] frames
/// outside and inside a light-change window.
pub(crate) fn window_line(every: u32, ms: &[Vec<f64>; 2]) -> String {
    let tail = |v: &Vec<f64>| {
        let mut t = v[v.len().saturating_sub(SAMPLES)..].to_vec();
        if t.is_empty() {
            "n/a".to_owned()
        } else {
            format!("{:.3} ms", percentile(&mut t, 50.0))
        }
    };
    format!(
        "lamp toggled every {every} frames: probes + gather median of the last {SAMPLES} outside a light-change window {} ({} frames), inside {} ({} frames)",
        tail(&ms[0]),
        ms[0].len(),
        tail(&ms[1]),
        ms[1].len()
    )
}

/// The field surface's band slots in the mirror: the band bricks holding one and their units'
/// bytes (each slot's whole bin, a uniform band brick's own slot included), inside the
/// mirror's live bytes.
pub(crate) fn field_line(mirror: &GpuWorld) -> String {
    format!(
        "field band: {} band bricks, {} B of band slots, mirror {} B live, {} B allocated",
        mirror.band_bricks(),
        mirror.band_units() * vxe_render::internals::UNIT_BYTES,
        mirror.mirror_bytes(),
        mirror.allocated_bytes()
    )
}

/// The band census (`flythrough::band_census`) over the chunks `mirror` holds full, against
/// the mirror's live bytes.
pub(crate) fn band_census_line(world: &vxe_world::World, mirror: &GpuWorld) -> String {
    let chunks: Vec<vxe_world::ChunkCoord> = world
        .store()
        .coords()
        .into_iter()
        .filter(|&c| mirror.is_mirrored(c) && !mirror.is_lod(c))
        .collect();
    let t = now();
    let cs = crate::band_census(world, &chunks);
    format!(
        "band census over {} full chunks in {:.0} ms: band voxels {} solid, {} air; band bricks {} Full, {} uniform solid, {} uniform air ({} in all); Full bricks with a surface voxel {}, of them band {}; bytes packed {}, dense {}; mirror {} B live",
        chunks.len(),
        ms_since(t),
        cs.solid_voxels,
        cs.air_voxels,
        cs.full_bricks,
        cs.solid_bricks,
        cs.air_bricks,
        cs.bricks(),
        cs.surface_full_bricks,
        cs.surface_full_band_bricks,
        cs.packed_bytes,
        cs.dense_bytes,
        mirror.mirror_bytes()
    )
}

/// The pool's bins at a moment: per bin with a page, its slot size, live slots, spare units
/// (free slots and the rest of its cursor's page) and pages, then the spare units over every
/// bin against the live units, e.g. `bins: 2u 10 live 6 spare 1p, 3u 4 live 0 spare 1p;
/// spare 6 of 32 live units (18.75 %)`.
pub(crate) fn bins_line(mirror: &GpuWorld) -> String {
    let bins: Vec<String> = mirror
        .bin_stats()
        .iter()
        .filter(|b| b.pages > 0)
        .map(|b| {
            format!(
                "{}u {} live {} spare {}p",
                b.units,
                b.live,
                b.spare * u64::from(b.units),
                b.pages
            )
        })
        .collect();
    let (spare, live) = (mirror.spare_units(), mirror.live_units());
    format!(
        "bins: {}; spare {spare} of {live} live units ({:.2} %)",
        bins.join(", "),
        100.0 * spare as f64 / live.max(1) as f64
    )
}

/// `field pass: 383608 band slots in 12.345 ms GPU (initial fill)`.
pub(crate) fn field_pass_line(what: &str, (n, ms): (usize, Option<f64>)) -> String {
    let ms = ms.map_or("n/a (no TIMESTAMP_QUERY)".to_owned(), |ms| {
        format!("{ms:.3} ms")
    });
    format!("field pass: {n} band slots in {ms} GPU ({what})")
}

#[derive(Default)]
pub(crate) struct Samples {
    pub(crate) cpu_ms: Vec<f64>,
    /// The frame's GPU time: the sum of its passes.
    pub(crate) gpu_ms: Vec<f64>,
    /// Each pass's GPU time, `FRAME_PASSES` in order.
    pub(crate) pass_ms: Vec<Vec<f64>>,
    /// The HUD pass's GPU time, on frames that draw it.
    pub(crate) hud_ms: Vec<f64>,
}

/// Every frame's wall time since the frame before, its render-thread CPU, its `update + poll + sync` (the sync alone without
/// streaming) and, on frames the march timer is on, the march's GPU time: the `frames:` line
/// printed at exit. Each series keeps the newest [`RUN_SERIES_FRAMES`] values in a buffer
/// allocated once (`report::Series`); the counts are of the whole run.
pub(crate) struct RunTimes {
    /// The wall time between frame starts, from the second frame on.
    pub(crate) wall_ms: Series,
    pub(crate) cpu_ms: Series,
    pub(crate) stream_ms: Series,
    pub(crate) gpu_ms: Series,
    /// Frames pushed, and of them those with a GPU time, over the whole run.
    pub(crate) frames: u64,
    pub(crate) gpu_frames: u64,
    /// Reads on the frame path that waited on the GPU over the run: the pass timers' and the
    /// store's counters'; `None` without the GPU's passes.
    pub(crate) blocking_reads: Option<u64>,
}

/// The frames each of [`RunTimes`]' series keeps: about 18 minutes at 60 frames a second, 512
/// KiB a series.
pub(crate) const RUN_SERIES_FRAMES: usize = 1 << 16;

impl Default for RunTimes {
    fn default() -> Self {
        let series = || Series::with_capacity(RUN_SERIES_FRAMES);
        Self {
            wall_ms: series(),
            cpu_ms: series(),
            stream_ms: series(),
            gpu_ms: series(),
            frames: 0,
            gpu_frames: 0,
            blocking_reads: None,
        }
    }
}

impl RunTimes {
    pub(crate) fn push(&mut self, cpu_ms: f64, stream_ms: Option<f64>, gpu_ms: Option<f64>) {
        self.frames += 1;
        self.cpu_ms.push(cpu_ms);
        if let Some(s) = stream_ms {
            self.stream_ms.push(s);
        }
        if let Some(g) = gpu_ms {
            self.gpu_ms.push(g);
            self.gpu_frames += 1;
        }
    }

    /// p50, p99 and max of each series, `None` before the first frame.
    pub(crate) fn line(&self) -> Option<String> {
        if self.frames == 0 {
            return None;
        }
        let stats = |v: &Series| {
            let mut v = v.values().to_vec();
            format!(
                "p50 {:.3} ms, p99 {:.3} ms, max {:.3} ms",
                percentile(&mut v, 50.0),
                percentile(&mut v, 99.0),
                percentile(&mut v, 100.0)
            )
        };
        let gpu = if self.gpu_frames == 0 {
            "n/a (timer off)".to_owned()
        } else {
            format!(
                "{} over {} timed frames",
                stats(&self.gpu_ms),
                self.gpu_frames
            )
        };
        let wall = if self.wall_ms.values().is_empty() {
            "n/a".to_owned()
        } else {
            stats(&self.wall_ms)
        };
        let blocking = self
            .blocking_reads
            .map_or_else(|| "n/a".to_owned(), |n| n.to_string());
        Some(format!(
            "frames: {} frames; wall {wall}; frame CPU {}; update + poll + sync {}; march GPU {gpu}; blocking reads {blocking}",
            self.frames,
            stats(&self.cpu_ms),
            stats(&self.stream_ms)
        ))
    }
}

/// The stats HUD's state. The frame-time ring and the last sync and edit are kept on every
/// frame; the world's counts are sampled at the cadence, and the rows formatted, only while
/// the HUD is shown.
pub(crate) struct Hud {
    pub(crate) visible: bool,
    pub(crate) frames: FrameStats,
    pub(crate) cadence: Cadence,
    pub(crate) world: WorldSample,
    /// The process's RSS, read with the world sample: reading `/proc/self/statm` costs a
    /// tenth of a millisecond or more in this process, too much for every frame.
    pub(crate) rss_bytes: Option<u64>,
    /// The last mirror sync and its render-thread time in milliseconds.
    pub(crate) sync: (SyncStats, f64),
    /// The rows' values, refreshed in place on every shown frame. Its `last_edit` is the only
    /// copy of the last edit, so a frame clones nothing.
    pub(crate) stats: HudStats,
    /// Last frame's pass readings, shown on the next frame's rows: the frame's total and
    /// each pass's, `FRAME_PASSES` in order.
    pub(crate) march_ms: Option<f32>,
    pub(crate) pass_ms: Option<Vec<f64>>,
    pub(crate) hud_ms: Option<f32>,
    pub(crate) cpu_ms: f32,
    /// The probe volume's live probes at the last world sample, with the lamp's bounce on.
    pub(crate) live_probes: Option<u32>,
    /// The store's counts at the last sample.
    pub(crate) store: Option<vxe_render::StoreStats>,
    /// The last frame's steps view, read back while that view and the HUD are on.
    pub(crate) steps: Option<hud::StepsSample>,
    /// The last frame's cracks view, read back while that view and the HUD are on.
    pub(crate) cracks: Option<vxe_render::CrackShares>,
    /// The exposure state as last read back (`Frame::exposure_readout`), a few frames late.
    pub(crate) exposure: Option<vxe_render::ExposureReadout>,
    pub(crate) format_us: u32,
    pub(crate) page_bytes: u64,
    /// Reused buffers: the rows' text, their colours, the strip's samples and the banner's
    /// rows.
    pub(crate) text: String,
    pub(crate) colours: Vec<HudColour>,
    pub(crate) strip: Vec<f32>,
    pub(crate) banner: String,
    /// The far normal's words and the live queue's order and withdrawal in words, remade
    /// only when what they name changes.
    pub(crate) far_normal_words: Cached<(NormalMode, [f32; 2])>,
    pub(crate) queue_words: Cached<(vxe_render::QueueOrder, bool)>,
    /// The whole run's frame times, shown or not.
    pub(crate) run: RunTimes,
}

/// A string kept with the key it was made from, remade only when the key changes: a row's
/// words that change on a key press, not every frame.
#[derive(Debug, Default)]
pub(crate) struct Cached<K> {
    pub(crate) key: Option<K>,
    pub(crate) text: String,
}

impl<K: PartialEq + Copy> Cached<K> {
    /// The text for `key`, made by `make` when the key differs from the last one's.
    pub(crate) fn get(&mut self, key: K, make: impl FnOnce() -> String) -> &str {
        if self.key != Some(key) {
            self.text = make();
            self.key = Some(key);
        }
        &self.text
    }
}

impl Hud {
    pub(crate) fn new(visible: bool) -> Self {
        Self {
            visible,
            frames: FrameStats::new(),
            cadence: Cadence::new(HUD_STATS_EVERY),
            world: WorldSample::default(),
            rss_bytes: None,
            sync: (SyncStats::default(), 0.0),
            stats: HudStats::default(),
            march_ms: None,
            pass_ms: None,
            hud_ms: None,
            cpu_ms: 0.0,
            live_probes: None,
            store: None,
            steps: None,
            cracks: None,
            exposure: None,
            format_us: 0,
            page_bytes: hud::page_bytes(),
            text: String::new(),
            colours: Vec::new(),
            strip: Vec::with_capacity(hud::RING),
            banner: String::new(),
            far_normal_words: Cached::default(),
            queue_words: Cached::default(),
            run: RunTimes::default(),
        }
    }

    /// Formats `stats` into the reused row and colour buffers, timing it, and copies the
    /// frame window into the reused strip.
    pub(crate) fn format_rows(&mut self) {
        let t = now();
        self.stats.write_rows(&mut self.text, &mut self.colours);
        self.format_us = ((now() - t).as_secs_f64() * 1e6) as u32;
        self.strip.clear();
        self.strip.extend(self.frames.samples().map(|s| s * 1000.0));
    }
}

/// Per-frame readings of the field pass over the autopilot's flight: the band slots it
/// filled and the render thread's CPU time in `update_field` every frame, and the pass's GPU
/// time on the frames with band slots to fill.
#[derive(Default)]
pub(crate) struct FieldSamples {
    pub(crate) items: Vec<f64>,
    pub(crate) cpu_ms: Vec<f64>,
    pub(crate) gpu_ms: Vec<f64>,
}

/// The mirror's staging belt: its chunks, the host bytes they map at the chunk size, and how
/// often a burst replaced it.
pub(crate) fn belt_line(mirror: &GpuWorld) -> String {
    let n = mirror.belt_chunks();
    format!(
        "staging belt {n} chunks ({} B mapped), {} trims",
        n as u64 * BELT_CHUNK_BYTES,
        mirror.belt_trims()
    )
}

impl FieldSamples {
    /// Records one flight frame's pass.
    pub(crate) fn push(&mut self, items: usize, cpu_ms: f64, gpu_ms: Option<f64>) {
        self.items.push(items as f64);
        self.cpu_ms.push(cpu_ms);
        self.gpu_ms.extend(gpu_ms);
    }

    /// The report's lines: p50, p99 and max of each reading.
    pub(crate) fn lines(&mut self) -> Vec<String> {
        let stats = |v: &mut Vec<f64>| {
            [50.0, 99.0, 100.0].map(|p| if v.is_empty() { 0.0 } else { percentile(v, p) })
        };
        let frames = self.items.len();
        let busy = self.items.iter().filter(|&&n| n > 0.0).count();
        let [i50, i99, imax] = stats(&mut self.items);
        let [c50, c99, cmax] = stats(&mut self.cpu_ms);
        let mut out = vec![
            format!(
                "autopilot: field pass items: p50 {i50:.0}, p99 {i99:.0}, max {imax:.0} band slots over {frames} frames ({busy} with work)"
            ),
            format!(
                "autopilot: field pass CPU (update_field, render thread): p50 {c50:.3} ms, p99 {c99:.3} ms, max {cmax:.3} ms over {frames} frames"
            ),
        ];
        if self.gpu_ms.is_empty() {
            out.push("autopilot: field pass GPU n/a (no TIMESTAMP_QUERY or no work)".into());
        } else {
            let n = self.gpu_ms.len();
            let [g50, g99, gmax] = stats(&mut self.gpu_ms);
            out.push(format!(
                "autopilot: field pass GPU: p50 {g50:.3} ms, p99 {g99:.3} ms, max {gmax:.3} ms over the {n} frames with work"
            ));
        }
        out
    }
}

/// The steps view's reading of the lamp shadow rays, from the light pass's counters (one
/// texel per hit, alpha 1): `shadow rays (full walk, 1 234 567 rays): steps mean 8.12, loads
/// mean 20.31, p99 64, pool reads mean 3.10`.
pub(crate) fn shadow_steps_line(texels: &[[f32; 4]], walk: ShadowWalk) -> String {
    let rays: Vec<&[f32; 4]> = texels.iter().filter(|t| t[3] > 0.5).collect();
    let n = rays.len().max(1) as f64;
    let mean = |k: usize| rays.iter().map(|t| f64::from(t[k])).sum::<f64>() / n;
    let mut loads: Vec<f64> = rays.iter().map(|t| f64::from(t[1])).collect();
    format!(
        "shadow rays ({} walk, {} rays): steps mean {:.2}, loads mean {:.2}, p99 {:.0}, pool reads mean {:.2}",
        walk.name(),
        rays.len(),
        mean(0),
        mean(1),
        percentile(&mut loads, 99.0),
        mean(2)
    )
}

/// The steps view's reading of one frame: `steps per ray (2073600 rays): mean 41.27, p99 133,
/// block-max mean 88.10; loads mean 12.40, p99 61; pool reads mean 3.02, p99 18`. Counts clamp
/// at 255 as the view does; the block-max mean is over 8 × 8 pixel blocks of each block's
/// maximum steps; loads are the ray's 32-bit storage loads from the mirror, pool reads the
/// pool loads among them.
/// The cracks view's shares of the frame `frame` last rendered, read back from its hit pass's
/// layers and records: a blocking copy of each.
pub(crate) fn crack_shares(gpu: &Gpu, frame: &vxe_render::Frame) -> vxe_render::CrackShares {
    let layers = vxe_render::debug::read_rgba16f(&gpu.device, &gpu.queue, frame.layers());
    let records = vxe_render::debug::read_rgba16u(&gpu.device, &gpu.queue, frame.hit_records());
    vxe_render::CrackShares::from_layers(&layers, &records)
}

pub(crate) fn steps_line(h: &StepHistogram) -> String {
    format!(
        "steps per ray ({} rays): mean {:.2}, p99 {}, block-max mean {:.2}; loads mean {:.2}, p99 {}; pool reads mean {:.2}, p99 {}",
        h.rays(),
        h.mean_steps(),
        h.p99_steps(),
        h.block_max_mean(),
        h.mean_loads(),
        h.p99_loads(),
        h.mean_pool_reads(),
        h.p99_pool_reads()
    )
}

/// The preset's name and parts, the far normal and the view: `look field (normals contour,
/// face mix 0, far normal off), view shaded`.
pub(crate) fn look_line(look: &LookParams, preset: &LookPreset) -> String {
    format!(
        "look {} (normals {}, {}, {}), view {}",
        preset.name,
        look.normal_mode.name(),
        preset.parts(),
        LookPreset::far_normal_words(look.normal_mode, look.far_normal),
        look.view.name()
    )
}

/// Appends the banner's pacing row, `pace free`, to `out` as a banner row (`hud::start_row`)
/// for [`hud::BANNER_MS`] after the start (`now_ms` on the frame loop's clock); `false`, and
/// nothing written, after that.
pub(crate) fn write_pace_banner(pace: Pacing, now_ms: f64, out: &mut String) -> bool {
    let shows = now_ms < hud::BANNER_MS;
    if shows {
        hud::start_row(out);
        out.push_str("pace ");
        out.push_str(pace.word());
    }
    shows
}

/// The full-first radius in words, written without allocating: `off` or the radius, `2`.
pub(crate) struct FullFirstWords(pub(crate) Option<i32>);

impl std::fmt::Display for FullFirstWords {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            Some(r) => write!(f, "{r}"),
            None => f.write_str("off"),
        }
    }
}

/// Appends the HUD queue row's reach part to `out`: `; reach full`, or `; reach partial (N
/// partial)` with the chunks whose block was baked from a partial reach.
pub(crate) fn write_reach_note(out: &mut String, reach: vxe_render::ReachMode, partial: u64) {
    let _ = write!(out, "; reach {}", reach.name());
    if reach == vxe_render::ReachMode::Partial {
        let _ = write!(out, " ({partial} partial)");
    }
}

/// Appends the HUD queue row's budget part to `out`: `; budget frame:1.5`, and the last step's
/// slice under a per-second budget, `; budget second:150,0.25 (slice 2.31 ms)`.
pub(crate) fn write_budget_note(out: &mut String, budget: vxe_render::Budget, slice_ms: f64) {
    let _ = write!(out, "; budget {budget}");
    if matches!(budget, vxe_render::Budget::PerSecond { .. }) {
        let _ = write!(out, " (slice {slice_ms:.2} ms)");
    }
}

/// Appends the live queue's banner row to `out`: the mode, its settings and slice, then
/// `words` (its order and withdrawal) and its keys.
pub(crate) fn write_queue_banner_row(out: &mut String, args: &Args, words: &str) {
    let _ = write!(
        out,
        "light: live queue, bricks {}/{}, ",
        vxe_render::internals::BRICK_CELL,
        vxe_render::QUEUE_BRICK_SAMPLES,
    );
    if args.bake.coarse > 0 {
        let _ = write!(
            out,
            "coarse {}/{} first, near {} at {}",
            vxe_render::internals::BRICK_CELL,
            args.bake.coarse,
            args.bake.near,
            vxe_render::QUEUE_BRICK_SAMPLES
        );
    } else {
        let _ = write!(out, "no coarse pass, near {}", args.bake.near);
    }
    let _ = write!(
        out,
        ", full-first {}, budget {}, missing light {}, reach {}, digs +{} ms, ring 3 {}; {words} (1 order, 2/3 angle, 4 withdraw, 5 budget, 6 full-first, 7 missing light, 8 reach)",
        FullFirstWords(args.full_first()),
        args.bake.budget,
        args.bake.missing_light.name(),
        args.bake.reach.name(),
        args.bake.edit_ms,
        if args.bake.dig_reach { "on" } else { "off" },
    );
}

/// Appends the HUD store row's queue part to `out`: `, queue depth N, coarse C, jobs J, slice
/// X ms` and `, pool full P` when the pool refused any; nothing without the queue's stats.
pub(crate) fn write_queue_note(out: &mut String, stats: Option<vxe_render::QueueStats>) {
    if let Some(st) = stats {
        let _ = write!(
            out,
            ", queue depth {}, coarse {}, jobs {}, slice {:.2} ms",
            st.depth, st.coarse, st.jobs, st.ms
        );
        if st.pool_full > 0 {
            let _ = write!(out, ", pool full {}", st.pool_full);
        }
    }
}

impl Run {
    /// Samples the world's counts for the HUD: `World::stats`, and `Streamer::stats` for the
    /// LOD-only chunks while streaming, timed together; then the process's RSS.
    pub(crate) fn sample_world(&mut self) {
        let t = now();
        let s = self.world.stats();
        let lod = self
            .stream
            .as_ref()
            .map(|st| st.stats.unwrap_or_else(|| st.streamer.stats()).resident_lod as u32);
        // The world keeps no brick-array pool: no spare arrays to count.
        self.hud.world = WorldSample::from_stats(&s, lod, 0, ms_since(t) as f32);
        self.hud.rss_bytes = hud::rss_bytes(self.hud.page_bytes);
    }

    /// Takes the store's counters as last read back (a frame or two old, without waiting) and,
    /// with the lamp's bounce on, counts the probe volume's live probes for the HUD: a blocking
    /// read, paid at the world sample's cadence.
    pub(crate) fn sample_probes(&mut self, gpu: &Gpu) {
        if let Some((stats, _)) = self
            .gfx
            .as_ref()
            .and_then(|g| g.frame.try_store_stats(&gpu.device))
        {
            self.hud.store = Some(stats);
        }
        self.hud.live_probes = self.gfx.as_ref().and_then(|g| {
            g.frame.lighting().lamp_bounce.then(|| {
                let probes = g.frame.probes();
                let states = probes.read_states(&gpu.device, &gpu.queue);
                let origin = probes.origin_for(self.fly.camera.position.to_array());
                probes.live_count(&states, origin)
            })
        });
    }

    /// Refreshes the HUD's values in place for this frame, from the O(1) sources, the GPU
    /// side's values `g` and the last world sample; the last edit is already there.
    pub(crate) fn refresh_hud_stats(&mut self, g: Option<GfxSample>) {
        let (mirror_bytes, pool_live, pool_free) =
            g.map_or((0, 0, 0), |g| (g.mirror_bytes, g.pool_live, g.pool_free));
        let pressure = g.map(|g| hud::Pressure {
            pressure: g.pressure,
            budget_bytes: g.budget_bytes,
            r_full: self.stream.as_ref().map(|s| [s.rule.r_full, s.rule.max]),
        });
        // The lighting row is rewritten in place, its vector and strings reused.
        match (g, self.hud.pass_ms.as_ref()) {
            (Some(g), Some(passes)) => {
                let row = self.hud.stats.lighting.get_or_insert_with(Default::default);
                row.passes.clear();
                row.passes.extend(
                    FRAME_PASSES
                        .iter()
                        .zip(passes)
                        .map(|(&n, &ms)| (n, ms as f32)),
                );
                row.light_samples = g.light_samples;
                row.live_probes = self
                    .hud
                    .live_probes
                    .filter(|_| self.modes.lighting.lamp_bounce);
                row.store = self.hud.store;
                row.missing_light = self.args.bake.missing_light.name();
                row.store_note.clear();
                write_queue_note(&mut row.store_note, self.queue.stats);
                row.modes.clear();
                self.modes.write_line(&mut row.modes);
                row.upscale = g.upscale;
            }
            _ => self.hud.stats.lighting = None,
        }
        let pos = self.fly.camera.position;
        let mode = if self.walk.is_some() {
            hud::Mode::Walk
        } else {
            hud::Mode::Fly
        };
        let h = &mut self.hud;
        let (sync, sync_ms) = h.sync;
        let s = &mut h.stats;
        s.fps = h.frames.fps();
        s.frame_ms = h.frames.frame_ms();
        s.cpu_ms = h.cpu_ms;
        s.pace = Some(self.args.pace.word());
        s.march_ms = h.march_ms;
        s.hud_ms = h.hud_ms;
        s.upload_bytes = sync.bytes;
        s.upload_ms = sync_ms as f32;
        s.upload_chunks = sync.chunks as u32;
        s.mirror_bytes = mirror_bytes;
        s.pool_live = pool_live;
        s.pool_free = pool_free;
        s.pressure = pressure;
        s.world = WorldSample {
            age_frames: h.cadence.age(),
            ..h.world
        };
        s.rss_bytes = h.rss_bytes;
        s.steps = h.steps;
        s.cracks = h.cracks;
        s.camera = pos.to_array();
        s.cutaway = self.modes.look.cutaway.then_some(self.modes.cutaway.name());
        s.chunk = voxel_of(pos).chunk().0;
        s.mode = mode;
        s.format_us = h.format_us;
        self.modes
            .fill_exposure_row(s.exposure.get_or_insert_with(Default::default));
        let args = &self.args;
        let words = h
            .queue_words
            .get((args.queue_order(), args.bake.queue_withdraw), || {
                args.queue_words()
            });
        let queue = s.queue.get_or_insert_with(String::new);
        queue.clear();
        hud::write_queue_row(
            queue,
            &self.queue.stats.unwrap_or_default(),
            self.queue.line_step_p50,
        );
        queue.push_str("; ");
        queue.push_str(words);
        match args.full_first() {
            Some(r) => {
                let _ = write!(queue, "; full-first {r}");
            }
            None => queue.push_str("; full-first off"),
        }
        write_reach_note(
            queue,
            args.bake.reach,
            self.queue.stats.map_or(0, |st| st.partial),
        );
        write_budget_note(
            queue,
            args.bake.budget,
            self.queue.stats.map_or(0.0, |st| st.slice_ms),
        );
        s.modes.clear();
        self.modes.write_rows(&mut s.modes);
        let look = &self.modes.look;
        let far = h
            .far_normal_words
            .get((look.normal_mode, look.far_normal), || {
                LookPreset::far_normal_words(look.normal_mode, look.far_normal)
            });
        let aliasing = s.aliasing.get_or_insert_with(String::new);
        aliasing.clear();
        hud::write_aliasing_row(
            aliasing,
            &self.modes.upscaler,
            far,
            &self.modes,
            self.modes.pan.as_ref(),
        );
    }

    /// The CPU half of [`Run::update_hud`]: refreshes the values (with the GPU side's `g`),
    /// formats the rows and the strip, and puts the banner's rows under them in yellow.
    /// Allocates nothing once the buffers have grown to the rows' size.
    pub(crate) fn fill_hud(&mut self, g: Option<GfxSample>) {
        self.refresh_hud_stats(g);
        self.forget_pose_once_moved();
        self.write_banner();
        let h = &mut self.hud;
        h.format_rows();
        // The mode banner goes under the rows, hiding none of them.
        for line in h.banner.lines() {
            h.text.push_str(line);
            h.text.push('\n');
            h.colours.push(HudColour::Yellow);
        }
    }

    /// Drops `--pose`'s banner row once the camera has left the pose.
    pub(crate) fn forget_pose_once_moved(&mut self) {
        if self
            .pose
            .as_ref()
            .is_some_and(|(_, at)| self.fly.camera != *at)
        {
            self.pose = None;
        }
    }

    /// The banner at the frame loop's clock: `--pose`'s row until the camera first moves, the
    /// scene's row while it shows (for `hud::BANNER_MS` after it was last shown), over the
    /// lighting modes' banner and the upscaler's, whichever show.
    ///
    /// Written into the HUD's reused `banner` buffer, row by row; the live queue's row always
    /// shows, so the banner is never empty.
    pub(crate) fn write_banner(&mut self) {
        let mut out = std::mem::take(&mut self.hud.banner);
        let mut words = std::mem::take(&mut self.hud.queue_words);
        let args = &self.args;
        let w = words.get((args.queue_order(), args.bake.queue_withdraw), || {
            args.queue_words()
        });
        out.clear();
        self.compose_banner(&mut out, w);
        self.hud.queue_words = words;
        self.hud.banner = out;
    }

    /// [`Run::write_banner`]'s rows as a new string.
    #[cfg(test)]
    pub(crate) fn banner(&self) -> Option<String> {
        let mut out = String::new();
        self.compose_banner(&mut out, &self.args.queue_words());
        Some(out)
    }

    /// Appends the banner's rows to `out`, the live queue's order and withdrawal in `words`.
    pub(crate) fn compose_banner(&self, out: &mut String, words: &str) {
        let now = ms_since(self.start);
        if let Some(p) = &self.player {
            hud::write_path_row(out, p, self.args.tester);
            if self.args.tester {
                if self.help {
                    hud::write_help_panel(out, p);
                }
                return;
            }
        }
        if let Some((pose, _)) = &self.pose {
            out.push_str(pose);
        }
        write_pace_banner(self.args.pace, now, out);
        hud::start_row(out);
        write_queue_banner_row(out, &self.args, words);
        if let Some((scene, _)) = self
            .scene
            .as_ref()
            .filter(|(_, at)| now - at < hud::BANNER_MS)
        {
            hud::start_row(out);
            out.push_str(scene);
        }
        self.modes.write_banner(now, out);
        self.modes.upscaler.write_banner(now, out);
        if let Some(p) = &self.modes.pan {
            p.write_banner(now, out);
        }
    }

    /// Formats this frame's rows and strip into the overlay, timing the formatting.
    pub(crate) fn update_hud(&mut self, gpu: &Gpu) {
        self.fill_hud(self.gfx.as_ref().map(GfxSample::of));
        let h = &mut self.hud;
        let Some(gfx) = self.gfx.as_mut() else { return };
        gfx.hud.set_lines(&gpu.queue, &h.text, &h.colours);
        gfx.hud.set_samples(&gpu.queue, &h.strip, hud::CEILING_MS);
    }

    /// Prints the frame line over `samples`, under the field look with the smooth share of the
    /// last frame's hits read back from `gpu`'s hit records.
    pub(crate) fn report(&mut self, gpu: &Gpu, samples: Samples) {
        let Samples {
            mut cpu_ms,
            mut gpu_ms,
            mut pass_ms,
            mut hud_ms,
        } = samples;
        let Some(gfx) = &self.gfx else { return };
        let [w, h] = [gfx.frame.target().width(), gfx.frame.target().height()];
        let [iw, ih] = gfx.frame.internal_size();
        let gpu_line = if gpu_ms.is_empty() {
            "n/a (no TIMESTAMP_QUERY)".to_owned()
        } else {
            let passes: Vec<String> = FRAME_PASSES
                .iter()
                .zip(pass_ms.iter_mut())
                .map(|(name, ms)| format!("{name} {:.3}", percentile(ms, 50.0)))
                .collect();
            format!(
                "{:.3} ms ({} ms)",
                percentile(&mut gpu_ms, 50.0),
                passes.join(", ")
            )
        };
        let hud = if hud_ms.is_empty() {
            String::new()
        } else {
            format!(", gpu hud {:.4} ms", percentile(&mut hud_ms, 50.0))
        };
        // Under the field look, the smooth share of the last frame's hit pixels: hits whose
        // material is not crisp. No other look reads the hit records back.
        let share = if self.modes.look.normal_mode == NormalMode::Contour
            && gfx.mirror.field_surface()
        {
            let records =
                vxe_render::debug::read_rgba16u(&gpu.device, &gpu.queue, gfx.frame.hit_records());
            let table = self.world.materials();
            let (mut hits, mut smooth) = (0u64, 0u64);
            for r in &records {
                if r[3] & HIT_FOUND_BIT != 0 {
                    hits += 1;
                    smooth += u64::from(!table.is_crisp(r[2]));
                }
            }
            let s = smooth as f64 / hits.max(1) as f64;
            format!("; smooth share s {s:.4} of {hits} hits")
        } else {
            String::new()
        };
        println!(
            "frame ({w}x{h} from {iw}x{ih}, median of {}, {}): cpu {:.3} ms, gpu frame {gpu_line}{hud}{share}; frame targets {} B; full bricks {}, mirror {} B live, {} B allocated",
            cpu_ms.len(),
            look_line(&self.modes.look, &self.modes.preset),
            percentile(&mut cpu_ms, 50.0),
            gfx.frame.frame_bytes(),
            self.world.stats().full_bricks,
            gfx.mirror.mirror_bytes(),
            gfx.mirror.allocated_bytes()
        );
        if let Some(audio) = &self.audio {
            println!(
                "audio: {} voices, callback max {:.3} ms over the last second, {} active taps, \
                 last acoustics update {:.3} ms",
                audio.voices.len(),
                audio.callback_max_ms,
                audio.acoustics.active_taps(),
                audio.update_ms
            );
            println!("{}", audio.mix.line());
            if let Some(mic) = &audio.mic {
                println!(
                    "mic: {}, {} frames dropped",
                    mic.input.device(),
                    mic.input.dropped_frames()
                );
            }
        }
        let room = self
            .audio
            .as_ref()
            .and_then(|a| a.acoustics.room())
            .or_else(|| self.room_probe.as_ref().and_then(|p| p.acoustics.room()));
        if let Some(r) = room {
            println!("{}", room_line(&r));
        }
        if let Some(p) = &self.room_probe {
            println!("{}", p.mix.line());
        }
        if let Some(c) = &self.coarse {
            println!("{}", c.query_line());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::hud::{self, HudStats};

    use crate::test_support::counting;

    /// A shown HUD allocates nothing per frame once its buffers have grown: 100 frames of
    /// recording a frame time, refreshing the values a frame changes, and formatting the
    /// rows and the strip, with a last edit held; frames 2 to 100 allocate 0 bytes. The
    /// overlay's half (rows and samples written, the pass encoded) is
    /// `vxe-render`'s `the_overlay_allocates_nothing_after_its_first_frame`.
    #[test]
    fn a_shown_hud_allocates_nothing_after_its_first_frame() {
        let mut hud = Hud::new(true);
        hud.stats = HudStats::fixed();
        hud.stats.pad = Some(hud::PadRow {
            name: "PS5 Controller".to_owned(),
            power: Some(hud::PadPower::Charging(5)),
        });
        let mut per_frame = Vec::with_capacity(100);
        for frame in 0..100u32 {
            let before = counting::bytes();
            hud.frames.record(0.016 + frame as f32 * 1e-5);
            hud.stats.fps = hud.frames.fps();
            hud.stats.frame_ms = hud.frames.frame_ms();
            hud.stats.format_us = hud.format_us;
            hud.format_rows();
            per_frame.push(counting::bytes() - before);
        }
        assert!(per_frame[0] > 0, "the first frame grows the buffers");
        assert_eq!(per_frame[1..], [0; 99]);
        assert_eq!(hud.text.lines().count(), 13);
        assert_eq!(hud.strip.len(), 100);
    }
}
