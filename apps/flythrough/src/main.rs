//! The engine's testbed (milestone 1 in `docs/roadmap.md`): a window showing the ray-marched
//! cave lit by a headlamp. Fly, dig, and read the per-material counts each edit returns.
//!
//! By default the cave is unbounded and streams around the camera (ADR-0003): full detail
//! within `r_full` chunks, mips out to `r_lod`. `--level` generates the bounded level instead.
//!
//! `flythrough [--seed N] [--level] [--vault] [--walk] [--render WxH] [--frames N] [--bench N]
//! [--autopilot] [--sync-budget MIB] [--headless] [--look MODE] [--rock-detail SCALE,STRENGTH]
//! [--view VIEW] [--detail]`. `--render` fixes the marcher's resolution (the window
//! scales it); `--frames` exits after N frames, printing the `F1` readings over the last
//! ones; `--bench` runs the `B` bench N times right after the first mirror upload, before
//! the first frame; `--autopilot` digs, flies 300 m out and back and prints the streaming
//! readings (see [`Autopilot`]); `--sync-budget` sets the mirror's upload bytes per frame;
//! `--headless` runs the same frames on an offscreen target with no window; `--walk` (with
//! `--level`) starts in walk mode, the body spawned in the spawn pocket; `--look` starts
//! in a normal mode (`face`, `trilinear8`, `gradient27`, `contour`) and gives the mirror
//! attribute pages, so `N` can reach `contour` (without `--look` it skips it), and gives rock
//! its procedural detail (`--rock-detail` sets rock's feature scale in voxels and strength,
//! default `flythrough::ROCK_DETAIL`); `--view` starts in a view (`shaded`, `normals`, `hitmask`) and
//! `--detail` with detail on. `N` cycles the normal mode and `V` toggles detail. `--vault`
//! places a 16-voxel room with an indestructible shell 10 m along +x from the spawn, written
//! over the cave as it generates (on the level, clamped inside the extent).
//!
//! `P` stamps that 16-voxel room at runtime, unrotated, with its minimum corner at the voxel
//! the dig ray hits, and prints what it changed like a dig does.
//!
//! Keys: `G` toggles fly and walk mode (on the bounded level only; streaming refuses it, as it
//! refuses `--walk`); in walk mode `W A S D` walk at 3 m/s and `Space` jumps, or, held against
//! a wall with a standable top within 1.2 m, climbs it. On the bounded level the fly-through
//! keeps a coarse grid and walkable map updated from every edit; `K` prints them.
//!
//! Collapse (bounded level, ADR-0008): after every dig the fly-through asks the grid which
//! regions lost their support and prints each as `unsupported: N cells, {material: count}`.
//! Each region falls 8 voxels a frame until it lands (`landed after N frames, fell M voxels`);
//! `R` toggles removing it with rubble (a quarter of its volume) instead. `F1` adds the last
//! query's time and region count.

mod fly;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;

use fly::{Fly, Walk};
use flythrough::cave::{CaveSource, LEVEL_CHUNKS, ROCK, VAULT_EDGE, vault};
use flythrough::{RESIDENCY, RUBBLE, cave_materials, chunk_ram, chunks_hash, mips_ram, percentile};
use vxe_app::{App, Flow, Gpu, Input, KeyCode, MouseButton, ShellConfig};
use vxe_render::glam::Vec3;
use vxe_render::gpu_world;
use vxe_render::{
    Camera, GpuWorld, LightParams, LodParams, LookParams, MarchTimer, Marcher, MirrorConfig,
    NormalMode, Presenter, ViewMode, Window, wgpu,
};
use vxe_world::hash::mix64;
use vxe_world::{
    Anchors, CellChange, CellCoord, ChunkCoord, ChunkExtent, CoarseGrid, EditError, EditOp,
    EditResult, FIXED_ONE, Fixed3, MaterialId, MaterialTable, Mode, PersistConfig, Rot,
    StreamConfig, Streamer, SupportParams, Tier, VoxelCoord, WalkableMap, WalkableParams, World,
    find_unsupported,
};

/// Reach of the dig and fill rays: 10 m.
const REACH: i64 = 100 * FIXED_ONE;
/// Dig radius at start (0.5 m), its scroll step (0.1 m) and its range.
const DIG_RADIUS: i64 = 5 * FIXED_ONE;
const DIG_STEP: i64 = FIXED_ONE;
const DIG_RANGE: (i64, i64) = (FIXED_ONE, 30 * FIXED_ONE);
/// Radius of the right-click fill: 0.3 m.
const FILL_RADIUS: i64 = 3 * FIXED_ONE;
/// The `B` bench: this many radius-1 m carves, centred within 10 m of the camera.
const BENCH_CARVES: usize = 100;
const BENCH_RADIUS: i64 = 10 * FIXED_ONE;
const BENCH_SPREAD: i64 = 100;
/// Free pool slots requested beyond the bounded level's `Full` bricks, so digging can grow
/// the pool: 64 Ki slots.
const POOL_HEADROOM_BRICKS: u64 = 65_536;
/// The pool size a streaming device's limits are requested for; capped at the adapter.
const STREAM_POOL_BYTES: u64 = 2 << 30;
/// Frames `F1` averages over.
const SAMPLES: usize = 30;
/// The first frame waits for the chunks within this squared distance of the spawn chunk.
const NEAR_D2: i32 = 4;
/// Default mirror upload budget per frame.
const SYNC_BUDGET_MIB: u64 = 2;
/// Autopilot: dig reach (30 m) and radius (1 m), flight speed (10 m/s) and leg length.
const PILOT_REACH: i64 = 300 * FIXED_ONE;
const PILOT_RADIUS: i64 = 10 * FIXED_ONE;
const PILOT_SPEED: f32 = 100.0;
const PILOT_LEG_SECS: f64 = 30.0;
/// Walkable cells: at most 64 solid voxels, on a floor cell of at least 448, under two open
/// cells (1.6 m).
const WALKABLE: WalkableParams = WalkableParams {
    max_solid: 64,
    min_floor: 448,
    clearance: 2,
};
/// The support query after a dig: cells of at least 64 solid voxels bear load, a flood gives
/// up past 4096 cells, and the level's faces anchor (ADR-0008).
const SUPPORT: SupportParams = SupportParams {
    solid_min: 64,
    max_region: 4096,
    boundary_anchors: true,
};
/// Voxels a falling region drops per frame: one cell, so it stays on the cell lattice.
const FALL_PER_FRAME: i32 = 8;
/// A removed region's rubble: a quarter of each column's volume, on ground within 6.4 m.
const RUBBLE_FRACTION: (u32, u32) = (1, 4);
const RUBBLE_DEPTH: i32 = 64;

#[allow(
    clippy::disallowed_methods,
    reason = "the fly-through times startup, edits and frames for its printed readings"
)]
fn now() -> Instant {
    Instant::now()
}

fn ms_since(t: Instant) -> f64 {
    (now() - t).as_secs_f64() * 1e3
}

#[derive(Clone, Copy)]
struct Args {
    seed: u64,
    render: Option<[u32; 2]>,
    frames: Option<u32>,
    bench: u32,
    level: bool,
    autopilot: bool,
    headless: bool,
    walk: bool,
    /// `--vault`: the cave generates with the vault room ahead of the spawn.
    vault: bool,
    sync_budget: u64,
    /// `--look`: the starting normal mode.
    look: Option<NormalMode>,
    /// `--detail`: start with procedural detail on.
    detail: bool,
    /// `--rock-detail`: rock's feature scale (voxels) and strength under `--look`.
    rock_detail: [f32; 2],
    view: ViewMode,
}

/// Why walk mode is refused while streaming, for `--walk` without `--level` and for `G`.
const WALK_NEEDS_LEVEL: &str =
    "walk mode needs --level (the coarse grid is not wired to streaming)";

/// Walk mode is available only on the bounded level.
fn walk_allowed(streaming: bool) -> Result<(), &'static str> {
    if streaming {
        Err(WALK_NEEDS_LEVEL)
    } else {
        Ok(())
    }
}

fn parse_args() -> Result<Args, String> {
    parse_args_from(std::env::args().skip(1))
}

/// The arguments after the program name.
fn parse_args_from(argv: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut args = Args {
        seed: 1,
        render: None,
        frames: None,
        bench: 0,
        level: false,
        autopilot: false,
        headless: false,
        walk: false,
        vault: false,
        sync_budget: SYNC_BUDGET_MIB << 20,
        look: None,
        detail: false,
        rock_detail: flythrough::ROCK_DETAIL,
        view: ViewMode::Shaded,
    };
    let mut it = argv.into_iter();
    while let Some(flag) = it.next() {
        let mut value = || it.next().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--seed" => args.seed = value()?.parse().map_err(|e| format!("--seed: {e}"))?,
            "--render" => {
                let v = value()?;
                let (w, h) = v.split_once('x').ok_or("--render takes WxH")?;
                let parse = |s: &str| s.parse::<u32>().map_err(|e| format!("--render: {e}"));
                args.render = Some([parse(w)?, parse(h)?]);
            }
            "--frames" => {
                args.frames = Some(value()?.parse().map_err(|e| format!("--frames: {e}"))?)
            }
            "--bench" => args.bench = value()?.parse().map_err(|e| format!("--bench: {e}"))?,
            "--sync-budget" => {
                let mib: u64 = value()?
                    .parse()
                    .map_err(|e| format!("--sync-budget: {e}"))?;
                args.sync_budget = mib << 20;
            }
            "--look" => {
                let v = value()?;
                args.look = Some(NormalMode::from_name(&v).ok_or(format!(
                    "--look takes face, trilinear8, gradient27 or contour, not {v}"
                ))?);
            }
            "--rock-detail" => args.rock_detail = parse_rock_detail(&value()?)?,
            "--view" => {
                let v = value()?;
                args.view = ViewMode::from_name(&v)
                    .ok_or(format!("--view takes shaded, normals or hitmask, not {v}"))?;
            }
            "--level" => args.level = true,
            "--detail" => args.detail = true,
            "--autopilot" => args.autopilot = true,
            "--headless" => args.headless = true,
            "--walk" => args.walk = true,
            "--vault" => args.vault = true,
            _ => return Err(format!("unknown argument {flag}")),
        }
    }
    if args.rock_detail != flythrough::ROCK_DETAIL && args.look.is_none() {
        return Err("--rock-detail applies under --look".to_owned());
    }
    if args.autopilot && args.level {
        return Err("--autopilot streams; it does not take --level".to_owned());
    }
    if args.walk {
        walk_allowed(!args.level).map_err(|e| format!("--walk: {e}"))?;
    }
    Ok(args)
}

/// `SCALE,STRENGTH`: a feature scale in voxels above 0 and a strength in `[0, 1]`.
fn parse_rock_detail(v: &str) -> Result<[f32; 2], String> {
    let err = || format!("--rock-detail takes SCALE,STRENGTH (e.g. 4,0.3), not {v}");
    let (scale, strength) = v.split_once(',').ok_or_else(err)?;
    let parse = |s: &str| s.trim().parse::<f32>().map_err(|_| err());
    let [scale, strength] = [parse(scale)?, parse(strength)?];
    if !(scale > 0.0 && scale.is_finite()) {
        return Err(format!("--rock-detail: scale must be above 0, not {scale}"));
    }
    if !(0.0..=1.0).contains(&strength) {
        return Err(format!(
            "--rock-detail: strength must be in [0, 1], not {strength}"
        ));
    }
    Ok([scale, strength])
}

/// One printed line per edit: `carved: rock 1234, ore 12 (3 chunks, 0.21 ms)`.
fn edit_line(verb: &str, r: &EditResult, table: &MaterialTable, ms: f64) -> String {
    let counts = if r.changed_from.is_empty() {
        "nothing".to_owned()
    } else {
        r.changed_from
            .iter()
            .map(|&(m, n)| format!("{} {n}", material_name(table, m)))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!("{verb}: {counts} ({} chunks, {ms:.2} ms)", r.touched.len())
}

/// `P`'s edit: the vault stamped unrotated with its minimum corner at the first solid voxel
/// the ray enters within [`REACH`]; `None` when it enters none.
fn stamp_at_aim(
    world: &World,
    origin: Fixed3,
    dir: [i64; 3],
) -> Option<Result<EditResult, EditError>> {
    let hit = world.raycast(origin, dir, REACH)?;
    Some(world.stamp(&vault(), hit.voxel, Rot::IDENTITY))
}

/// The `P` line: [`edit_line`] on success, the refusal, or the miss.
fn stamp_line(
    out: &Option<Result<EditResult, EditError>>,
    table: &MaterialTable,
    ms: f64,
) -> String {
    match out {
        None => "stamped: no surface within 10 m".to_owned(),
        Some(Ok(r)) => edit_line("stamped", r, table, ms),
        Some(Err(e)) => format!("stamped: refused, {e}"),
    }
}

fn material_name(table: &MaterialTable, m: MaterialId) -> String {
    table
        .get(m)
        .map_or_else(|| format!("#{m}"), |row| row.name.clone())
}

/// The process's peak resident set (`VmHWM`) in bytes, read from `/proc/self/status`; `None`
/// where that file does not exist or does not carry the line.
fn peak_rss_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("VmHWM:"))?;
    let kb: u64 = line
        .trim_start_matches("VmHWM:")
        .trim()
        .trim_end_matches("kB")
        .trim()
        .parse()
        .ok()?;
    Some(kb * 1024)
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn voxel_of(p: Vec3) -> VoxelCoord {
    VoxelCoord(p.to_array().map(|v| v.floor() as i32))
}

/// Chunk offsets within squared distance `r2`.
fn ball(r2: i32) -> Vec<[i32; 3]> {
    let r = (0..).find(|r| r * r > r2).unwrap_or(1);
    let mut v = Vec::new();
    for x in -r..=r {
        for y in -r..=r {
            for z in -r..=r {
                if x * x + y * y + z * z <= r2 {
                    v.push([x, y, z]);
                }
            }
        }
    }
    v
}

struct Gfx {
    mirror: GpuWorld,
    marcher: Marcher,
    presenter: Presenter,
    timer: Option<MarchTimer>,
    /// Times the attribute pass, with attribute pages and `TIMESTAMP_QUERY`.
    attr_timer: Option<MarchTimer>,
}

impl Gfx {
    /// Runs the attribute pass over what changed, alone, and waits for it: the bricks in its
    /// work list and its GPU time when timed.
    fn timed_attrs(&mut self, gpu: &Gpu) -> (usize, Option<f64>) {
        let mut enc = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let timer = self.attr_timer.as_ref();
        let n = self
            .mirror
            .update_attrs_timed(&mut enc, timer.map(MarchTimer::writes));
        if let Some(t) = timer.filter(|_| n > 0) {
            t.resolve(&mut enc);
        }
        gpu.queue.submit([enc.finish()]);
        let ms = timer
            .filter(|_| n > 0)
            .map(|t| t.read_ms(&gpu.device, &gpu.queue));
        (n, ms)
    }
}

/// `attrib: 383608 bricks in 12.345 ms GPU (initial fill)`.
fn attr_line(what: &str, (n, ms): (usize, Option<f64>)) -> String {
    let ms = ms.map_or("n/a (no TIMESTAMP_QUERY)".to_owned(), |ms| {
        format!("{ms:.3} ms")
    });
    format!("attrib: {n} bricks in {ms} GPU ({what})")
}

#[derive(Default)]
struct Samples {
    cpu_ms: Vec<f64>,
    gpu_ms: Vec<f64>,
}

/// The streaming state of the default mode.
struct Stream {
    streamer: Streamer,
    /// Milliseconds since start at each `update` call, `tick_ms[t - 1]` for tick `t`.
    tick_ms: Vec<f64>,
    /// Chunks loaded full and not yet mirrored, with the tick that first wanted them.
    awaiting: HashMap<ChunkCoord, u64>,
    /// Wanted-to-mirrored latency of each chunk mirrored full, in milliseconds.
    latency_ms: Vec<f64>,
    /// Render-thread CPU of `update + poll + sync`, per frame.
    cpu_ms: Vec<f64>,
    full_residency: bool,
    workers: usize,
}

impl Stream {
    /// Records every chunk `loaded` full with its wanted tick, and moves the ones the mirror
    /// now holds full into `latency_ms`.
    fn track(&mut self, ev: &vxe_world::StreamEvents, mirror: &GpuWorld) {
        let tick = self.streamer.stats().ticks;
        let full = ev.loaded.iter().filter(|(_, t)| *t == Tier::Full);
        for (&(c, _), &lat) in full.zip(&ev.latency_ticks) {
            self.awaiting.insert(c, tick.saturating_sub(u64::from(lat)));
        }
        let now_ms = self.tick_ms.last().copied().unwrap_or(0.0);
        let tick_ms = &self.tick_ms;
        let latency = &mut self.latency_ms;
        self.awaiting.retain(|&c, &mut wanted| {
            if mirror.is_mirrored(c) && !mirror.is_lod(c) {
                let at = tick_ms
                    .get(wanted.max(1) as usize - 1)
                    .copied()
                    .unwrap_or(now_ms);
                latency.push(now_ms - at);
                false
            } else {
                true
            }
        });
    }
}

/// The autopilot's legs (`--autopilot`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Leg {
    /// Waiting for full residency, then digging.
    Settle,
    Out,
    Back,
    /// Waiting for the dug chunks to be full and mirrored again.
    Return,
}

/// `--autopilot`: waits for full residency, carves 1 m at the first rock within 30 m along +x
/// and records the touched chunks' hash, flies +x at 10 m/s for 30 s (beyond the eviction
/// radius) and back, waits for the touched chunks to be full again, and compares the hash.
/// Over the flight it counts unready-near frames (a chunk wanted full within `r_full - 2`
/// chunks of the camera's not yet resident and mirrored full), and samples streaming CPU,
/// latency, memory and march-pass GPU time.
struct Autopilot {
    leg: Leg,
    leg_start: Instant,
    origin: Vec3,
    touched: Vec<ChunkCoord>,
    hash: u64,
    near: Vec<[i32; 3]>,
    flight_frames: u64,
    unready_frames: u64,
    gpu_ms: Vec<f64>,
    cpu_ms: Vec<f64>,
    latency_from: usize,
    peak_mirror: u64,
    peak_allocated: u64,
    peak_resident: u64,
    peak_persisted: u64,
    generated_at_takeoff: u64,
}

/// The bounded level's coarse grid and walkable map, updated from every edit, and the regions
/// falling after a dig.
struct Coarse {
    grid: CoarseGrid,
    map: WalkableMap,
    /// Cells the last edit changed, and the grid plus walkable update time in microseconds.
    last: Option<(usize, f64)>,
    /// Regions falling, in the order they were found.
    falling: Vec<Falling>,
    /// `R`: remove unsupported regions with rubble instead of dropping them.
    crumble: bool,
    /// The last support query's time in microseconds and the regions it found.
    query: Option<(f64, usize)>,
}

/// A region on its way down: its cells (moved one cell down per full-step frame), the frames
/// it moved on and the voxels it fell.
struct Falling {
    cells: Vec<CellCoord>,
    frames: u32,
    fell: i32,
}

impl Coarse {
    fn build(world: &World) -> Self {
        let t = now();
        let grid = CoarseGrid::build(world);
        let map = WalkableMap::build(&grid, WALKABLE);
        println!(
            "coarse: {} chunks, {} bytes, {} walkable cells, built in {:.0} ms",
            grid.len(),
            grid.bytes(),
            map.count(),
            ms_since(t)
        );
        Self {
            grid,
            map,
            last: None,
            falling: Vec::new(),
            crumble: false,
            query: None,
        }
    }

    /// Updates the grid and the walkable map from `r`; returns the grid's changed cells.
    fn update(&mut self, world: &World, r: &EditResult) -> Vec<CellChange> {
        let t = now();
        let changes = self.grid.update(world, r);
        self.map.update(&self.grid, &changes);
        self.last = Some((changes.len(), ms_since(t) * 1e3));
        changes
    }

    /// The `K` line.
    fn line(&self) -> String {
        match self.last {
            Some((m, us)) => format!(
                "coarse: {} walkable cells, last edit changed {m} cells in {us:.0} us",
                self.map.count()
            ),
            None => format!("coarse: {} walkable cells, no edit yet", self.map.count()),
        }
    }

    /// After a dig that changed `changes`: queries the support, prints each unsupported region
    /// and either queues it to fall or removes it with rubble. A region overlapping one already
    /// falling is that one, and is skipped.
    fn collapse(&mut self, world: &World, changes: &[CellChange]) -> Vec<String> {
        let t = now();
        let regions = find_unsupported(world, &self.grid, &SUPPORT, &Anchors::default(), changes);
        self.query = Some((ms_since(t) * 1e3, regions.len()));
        let table = world.materials();
        let mut lines = Vec::new();
        for region in regions {
            if self.falling.iter().any(|f| {
                region
                    .cells
                    .iter()
                    .any(|c| f.cells.binary_search(c).is_ok())
            }) {
                continue;
            }
            lines.push(format!(
                "unsupported: {} cells, {{{}}}",
                region.cells.len(),
                counts_line(table, &region.counts)
            ));
            if self.crumble {
                match world.remove_region(&region.cells, RUBBLE, RUBBLE_FRACTION, RUBBLE_DEPTH) {
                    Ok(r) => {
                        lines.push(format!(
                            "removed: {{{}}}, {} rubble",
                            counts_line(table, &r.removed),
                            r.placed
                        ));
                        self.update(world, &r.edit_result());
                    }
                    Err(e) => lines.push(format!("removed: refused, {e}")),
                }
            } else {
                self.falling.push(Falling {
                    cells: region.cells,
                    frames: 0,
                    fell: 0,
                });
            }
        }
        lines
    }

    /// One frame of falling: every falling region drops by up to [`FALL_PER_FRAME`]. A drop
    /// of a whole step moves the region's cells one cell down and it falls on; a shorter drop
    /// (or none) means a column met solid ground, so the next drop would be zero, and the
    /// region has landed.
    fn fall(&mut self, world: &World) -> Vec<String> {
        let mut lines = Vec::new();
        for mut f in std::mem::take(&mut self.falling) {
            match world.drop_region(&f.cells, FALL_PER_FRAME) {
                Ok(r) => {
                    if r.dropped > 0 {
                        f.frames += 1;
                        f.fell += r.dropped;
                        self.update(world, &r.edit_result());
                    }
                    if r.dropped == FALL_PER_FRAME {
                        for c in &mut f.cells {
                            c.0[2] -= 1;
                        }
                        self.falling.push(f);
                    } else {
                        lines.push(format!(
                            "landed after {} frames, fell {} voxels",
                            f.frames, f.fell
                        ));
                    }
                }
                Err(e) => lines.push(format!("fall: refused, {e}")),
            }
        }
        lines
    }

    /// The `R` toggle.
    fn toggle_crumble(&mut self) -> String {
        self.crumble = !self.crumble;
        if self.crumble {
            "collapse: remove unsupported regions with rubble".to_owned()
        } else {
            "collapse: drop unsupported regions".to_owned()
        }
    }

    /// The `F1` collapse line.
    fn query_line(&self) -> String {
        match self.query {
            Some((us, n)) => format!("collapse: last query {us:.0} us, {n} regions"),
            None => "collapse: no query yet".to_owned(),
        }
    }
}

/// `rock: 4608, ore: 12`: per-material counts by name.
fn counts_line(table: &MaterialTable, counts: &[(MaterialId, u32)]) -> String {
    counts
        .iter()
        .map(|&(m, n)| format!("{}: {n}", material_name(table, m)))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The running fly-through, once the world exists.
struct Run {
    start: Instant,
    world: World,
    /// The voxel the camera starts in; the mirror uploads nearest it first.
    spawn: VoxelCoord,
    args: Args,
    fly: Fly,
    light: LightParams,
    look: LookParams,
    dig_radius: i64,
    bench_runs: u64,
    frame: u32,
    gfx: Option<Gfx>,
    samples: Option<Samples>,
    stream: Option<Stream>,
    pilot: Option<Autopilot>,
    /// Walk mode's body, `None` in fly mode.
    walk: Option<Walk>,
    coarse: Option<Coarse>,
}

impl Run {
    /// Keeps the coarse grid current with `r`; returns its changed cells (none while
    /// streaming).
    fn edited(&mut self, r: &EditResult) -> Vec<CellChange> {
        match self.coarse.as_mut() {
            Some(c) => c.update(&self.world, r),
            None => Vec::new(),
        }
    }

    /// `G`: walk mode with the body under the camera, or back to flight.
    fn toggle_walk(&mut self) {
        if self.walk.take().is_some() {
            println!("fly mode");
            return;
        }
        if let Err(e) = walk_allowed(self.stream.is_some()) {
            println!("walk: {e}; still flying");
            return;
        }
        match Walk::at_camera(&self.world, &self.fly.camera) {
            Some(w) => {
                self.walk = Some(w);
                println!("walk mode");
            }
            None => println!("walk: no room for the body here; still flying"),
        }
    }

    fn coarse_line(&self) -> String {
        match &self.coarse {
            None => "coarse: not kept while streaming".to_owned(),
            Some(c) => c.line(),
        }
    }

    /// Rock's detail pair when the table carries it (under `--look`).
    fn rock_detail(&self) -> Option<[f32; 2]> {
        self.args.look.is_some().then_some(self.args.rock_detail)
    }

    fn camera_ray(&self) -> (Fixed3, [i64; 3]) {
        let c = &self.fly.camera;
        let origin = c
            .position
            .to_array()
            .map(|v| (v * FIXED_ONE as f32).floor() as i64);
        let dir = c
            .basis()
            .0
            .to_array()
            .map(|v| (v * FIXED_ONE as f32).round() as i64);
        (Fixed3(origin), dir)
    }

    fn apply(&mut self, verb: &str, op: &EditOp) {
        let t = now();
        let r = self.world.apply(op);
        let ms = ms_since(t);
        match r {
            Ok(r) => {
                println!("{}", edit_line(verb, &r, self.world.materials(), ms));
                self.edited(&r);
            }
            Err(e) => println!("{verb}: refused, {e}"),
        }
    }

    fn dig(&mut self) {
        let (origin, dir) = self.camera_ray();
        let t = now();
        let op = EditOp::tool_along_ray(
            &self.world,
            origin,
            dir,
            REACH,
            self.dig_radius,
            Mode::Carve,
        );
        let resolve_ms = ms_since(t);
        let t = now();
        let r = self.world.apply(&op);
        let ms = resolve_ms + ms_since(t);
        match r {
            Ok(r) => {
                println!("{}", edit_line("carved", &r, self.world.materials(), ms));
                let changes = self.edited(&r);
                if let Some(c) = self.coarse.as_mut() {
                    for line in c.collapse(&self.world, &changes) {
                        println!("{line}");
                    }
                }
            }
            Err(e) => println!("carved: refused, {e}"),
        }
    }

    /// `P`: the vault at the aim point, through the same post-edit path as a dig.
    fn stamp(&mut self) {
        let (origin, dir) = self.camera_ray();
        let t = now();
        let out = stamp_at_aim(&self.world, origin, dir);
        let ms = ms_since(t);
        println!("{}", stamp_line(&out, self.world.materials(), ms));
        if let Some(Ok(r)) = &out {
            self.edited(r);
        }
    }

    fn fill(&mut self) {
        let (origin, dir) = self.camera_ray();
        let Some(hit) = self.world.raycast(origin, dir, REACH) else {
            println!("filled: no surface within 10 m");
            return;
        };
        let at = Fixed3([0, 1, 2].map(|k| origin.0[k] + dir[k] * hit.t / FIXED_ONE));
        self.apply("filled", &EditOp::sphere(at, FILL_RADIUS, Mode::Fill(ROCK)));
    }

    /// `B`: carves at seeded positions within 10 m, then one mirror sync, both timed.
    fn bench(&mut self, gpu: &Gpu) {
        let centre = self
            .fly
            .camera
            .position
            .to_array()
            .map(|v| v.floor() as i64);
        let mut state = mix64(self.args.seed ^ self.bench_runs.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        self.bench_runs += 1;
        let mut next = || {
            state = mix64(state.wrapping_add(1));
            (state % (2 * BENCH_SPREAD as u64 + 1)) as i64 - BENCH_SPREAD
        };
        let mut ops = Vec::with_capacity(BENCH_CARVES);
        while ops.len() < BENCH_CARVES {
            let d = [next(), next(), next()];
            if d.iter().map(|v| v * v).sum::<i64>() <= BENCH_SPREAD * BENCH_SPREAD {
                let c = [0, 1, 2].map(|k| (centre[k] + d[k]) * FIXED_ONE + FIXED_ONE / 2);
                ops.push(EditOp::sphere(Fixed3(c), BENCH_RADIUS, Mode::Carve));
            }
        }

        // With attribute pages, the first carve is synced and its attribute pass timed alone.
        let attrs = self.gfx.as_ref().is_some_and(|g| g.mirror.surface_attrs());
        let mut results: Vec<Result<EditResult, EditError>> = Vec::with_capacity(ops.len());
        let mut edit_ms = 0.0;
        if attrs && let Some(gfx) = self.gfx.as_mut() {
            let t = now();
            results.push(self.world.apply(&ops[0]));
            edit_ms += ms_since(t);
            match gfx.mirror.sync(&self.world, &gpu.queue) {
                Ok(_) => println!(
                    "{}",
                    attr_line("one carve of radius 1 m", gfx.timed_attrs(gpu))
                ),
                Err(e) => println!("bench: sync failed, {e}"),
            }
        }
        let t = now();
        results.extend(ops[results.len()..].iter().map(|op| self.world.apply(op)));
        edit_ms += ms_since(t);

        let mut counts = BTreeMap::<MaterialId, u32>::new();
        let mut touched = BTreeSet::new();
        let mut refused = 0;
        for r in &results {
            match r {
                Ok(r) => {
                    for &(m, n) in &r.changed_from {
                        *counts.entry(m).or_default() += n;
                    }
                    touched.extend(r.touched.iter().copied());
                }
                Err(_) => refused += 1,
            }
        }
        let total = EditResult {
            changed_from: counts.into_iter().collect(),
            touched: touched.into_iter().collect(),
            aabb: None,
        };
        for r in results.iter().flatten() {
            self.edited(r);
        }
        println!(
            "{}",
            edit_line("carved", &total, self.world.materials(), edit_ms)
        );

        let Some(gfx) = self.gfx.as_mut() else { return };
        let t = now();
        match gfx.mirror.sync(&self.world, &gpu.queue) {
            Ok(s) => println!(
                "bench: {BENCH_CARVES} carves of radius 1 m within 10 m ({refused} refused): \
                 edits {edit_ms:.2} ms CPU; sync uploaded {} B ({} bricks, {} chunks, {} visited, \
                 {} writes) in {:.2} ms",
                s.bytes,
                s.bricks,
                s.chunks,
                s.chunks_visited,
                s.writes,
                ms_since(t)
            ),
            Err(e) => println!("bench: sync failed, {e}"),
        }
        if attrs {
            let what = format!("the other {} carves", BENCH_CARVES - 1);
            println!("{}", attr_line(&what, gfx.timed_attrs(gpu)));
        }
    }

    fn report(&mut self, samples: Samples) {
        let Samples {
            mut cpu_ms,
            mut gpu_ms,
        } = samples;
        let Some(gfx) = &self.gfx else { return };
        let [w, h] = [gfx.marcher.target().width(), gfx.marcher.target().height()];
        let gpu = if gpu_ms.is_empty() {
            "n/a (no TIMESTAMP_QUERY)".to_owned()
        } else {
            format!("{:.3} ms", median(&mut gpu_ms))
        };
        println!(
            "frame ({w}x{h}, median of {}, {}): cpu {:.3} ms, gpu march {gpu}; full bricks {}, mirror {} B live, {} B allocated",
            cpu_ms.len(),
            look_line(&self.look, self.rock_detail()),
            median(&mut cpu_ms),
            self.world.stats().full_bricks,
            gfx.mirror.mirror_bytes(),
            gfx.mirror.allocated_bytes()
        );
        if let Some(c) = &self.coarse {
            println!("{}", c.query_line());
        }
    }

    /// RAM held by resident chunks: full chunks' voxels and LOD-only chunks' mips.
    fn resident_ram(&self) -> u64 {
        let full: u64 = self
            .world
            .store()
            .coords()
            .into_iter()
            .filter_map(|c| self.world.store().get(c))
            .map(|c| chunk_ram(&c.read().unwrap_or_else(std::sync::PoisonError::into_inner)))
            .sum();
        let lod: u64 = self
            .world
            .lod_coords()
            .into_iter()
            .filter_map(|c| self.world.lod(c))
            .map(|m| mips_ram(&m))
            .sum();
        full + lod
    }
}

impl Run {
    fn device_request(&mut self, adapter: &wgpu::Adapter) -> (wgpu::Features, wgpu::Limits) {
        let pool = if self.stream.is_some() {
            STREAM_POOL_BYTES
        } else {
            let bricks = self.world.stats().full_bricks as u64 + POOL_HEADROOM_BRICKS;
            // With attribute pages each page holds `ATTR_PAGE_BRICK_SLOTS` bricks.
            let slots = if self.args.look.is_some() {
                bricks.div_ceil(u64::from(gpu_world::ATTR_PAGE_BRICK_SLOTS))
                    * u64::from(gpu_world::POOL_PAGE_SLOTS)
            } else {
                bricks
            };
            slots * gpu_world::SLOT_BYTES
        };
        (
            adapter.features() & wgpu::Features::TIMESTAMP_QUERY,
            vxe_render::limits_for(adapter, pool),
        )
    }

    fn init(&mut self, gpu: &Gpu) {
        let [w, h] = self.args.render.unwrap_or(gpu.size);
        let timer = gpu
            .device
            .features()
            .contains(wgpu::Features::TIMESTAMP_QUERY)
            .then(|| MarchTimer::new(&gpu.device));
        println!("device ready after {:.0} ms", ms_since(self.start));
        let attr_timer =
            (self.args.look.is_some() && timer.is_some()).then(|| MarchTimer::new(&gpu.device));
        let mut marcher = Marcher::new(&gpu.device, w, h);
        let mirror = if self.stream.is_some() {
            marcher.set_lod(LodParams {
                r_full: RESIDENCY.r_full,
                forced_level: None,
            });
            self.init_stream(gpu)
        } else {
            let mut mirror =
                self.new_mirror(gpu, MirrorConfig::new(Window::Fixed(self.world.extent())));
            mirror.set_focus(self.spawn);
            let t = now();
            match mirror.sync(&self.world, &gpu.queue) {
                Ok(s) => println!(
                    "mirror: uploaded {} B ({} bricks, {} chunks, {} writes) in {:.0} ms; mirror {} B live, {} B allocated",
                    s.bytes,
                    s.bricks,
                    s.chunks,
                    s.writes,
                    ms_since(t),
                    mirror.mirror_bytes(),
                    mirror.allocated_bytes()
                ),
                Err(e) => eprintln!("mirror sync failed: {e}"),
            }
            mirror
        };
        let mut gfx = Gfx {
            mirror,
            marcher,
            presenter: Presenter::new(&gpu.device, gpu.surface_format),
            timer,
            attr_timer,
        };
        if gfx.mirror.surface_attrs() {
            println!("{}", attr_line("initial fill", gfx.timed_attrs(gpu)));
        }
        self.gfx = Some(gfx);
        for _ in 0..self.args.bench {
            self.bench(gpu);
        }
    }

    /// A mirror under `cfg`, with attribute pages when `--look` was given.
    fn new_mirror(&self, gpu: &Gpu, cfg: MirrorConfig) -> GpuWorld {
        if self.args.look.is_some() {
            GpuWorld::with_surface_attrs(&gpu.device, cfg)
        } else {
            GpuWorld::with_config(&gpu.device, cfg)
        }
    }

    /// The streaming mirror, returned once the chunks within squared distance `NEAR_D2` of
    /// the spawn chunk are resident and mirrored full.
    fn init_stream(&mut self, gpu: &Gpu) -> GpuWorld {
        let mut mirror = self.new_mirror(
            gpu,
            MirrorConfig {
                sync_budget_bytes: self.args.sync_budget,
                ..MirrorConfig::new(Window::Follow {
                    side: flythrough::window_side(RESIDENCY),
                })
            },
        );
        let Some(stream) = self.stream.as_mut() else {
            return mirror;
        };
        let spawn = self.spawn.chunk();
        let near: Vec<ChunkCoord> = ball(NEAR_D2)
            .into_iter()
            .map(|o| ChunkCoord([0, 1, 2].map(|k| spawn.0[k] + o[k])))
            .collect();
        mirror.set_focus(self.spawn);
        loop {
            stream.streamer.update(&self.world, &[self.spawn]);
            stream.tick_ms.push(ms_since(self.start));
            let ev = stream.streamer.poll(&self.world);
            mirror.apply_events(&ev);
            if let Err(e) = mirror.sync(&self.world, &gpu.queue) {
                eprintln!("mirror sync failed: {e}");
                break;
            }
            stream.track(&ev, &mirror);
            if near
                .iter()
                .all(|&c| mirror.is_mirrored(c) && !mirror.is_lod(c))
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        println!(
            "near residency ({} chunks resident and mirrored) after {:.0} ms",
            near.len(),
            ms_since(self.start)
        );
        mirror
    }

    fn resized(&mut self, gpu: &Gpu) {
        if let (Some(gfx), None) = (self.gfx.as_mut(), self.args.render) {
            gfx.marcher.resize(&gpu.device, gpu.size[0], gpu.size[1]);
        }
    }

    /// One frame of streaming: update, poll, apply events and sync, timed together.
    fn stream_step(&mut self, gpu: &Gpu) -> Result<(), String> {
        let (Some(stream), Some(gfx)) = (self.stream.as_mut(), self.gfx.as_mut()) else {
            return Ok(());
        };
        let cam = voxel_of(self.fly.camera.position);
        let t = now();
        stream.streamer.update(&self.world, &[cam]);
        let ev = stream.streamer.poll(&self.world);
        gfx.mirror.apply_events(&ev);
        gfx.mirror.set_focus(cam);
        let synced = gfx.mirror.sync(&self.world, &gpu.queue);
        let cpu = ms_since(t);
        stream.tick_ms.push(ms_since(self.start));
        stream.cpu_ms.push(cpu);
        for e in &ev.errors {
            eprintln!("stream: {e:?}");
        }
        synced.map_err(|e| e.to_string())?;
        stream.track(&ev, &gfx.mirror);
        let st = stream.streamer.stats();
        if !stream.full_residency
            && st.in_flight == 0
            && st.queued == 0
            && gfx.mirror.pending_chunks() == 0
        {
            stream.full_residency = true;
            println!(
                "full residency after {:.0} ms: {} full, {} LOD-only chunks, mirror {} B live, {} B allocated",
                ms_since(self.start),
                st.resident_full,
                st.resident_lod,
                gfx.mirror.mirror_bytes(),
                gfx.mirror.allocated_bytes()
            );
        }
        Ok(())
    }

    /// Moves the camera along the autopilot's legs; returns `Flow::Exit` once it has printed
    /// its readings.
    fn pilot_step(&mut self) -> Flow {
        let (Some(p), Some(stream), Some(gfx)) =
            (self.pilot.as_mut(), self.stream.as_ref(), self.gfx.as_ref())
        else {
            return Flow::Continue;
        };
        let elapsed = (now() - p.leg_start).as_secs_f64();
        match p.leg {
            Leg::Settle => {
                if !stream.full_residency {
                    return Flow::Continue;
                }
                let o = centre_fixed(self.fly.camera.position);
                let op = EditOp::tool_along_ray(
                    &self.world,
                    o,
                    [FIXED_ONE, 0, 0],
                    PILOT_REACH,
                    PILOT_RADIUS,
                    Mode::Carve,
                );
                match self.world.apply(&op) {
                    Ok(r) => {
                        println!(
                            "autopilot: {}",
                            edit_line("carved", &r, self.world.materials(), 0.0)
                        );
                        p.touched = r.touched.clone();
                    }
                    Err(e) => println!("autopilot: carve refused, {e}"),
                }
                if p.touched.is_empty() {
                    println!("autopilot: nothing carved within 30 m along +x; no return check");
                }
                p.hash = chunks_hash(&self.world, &p.touched).unwrap_or(0);
                p.origin = self.fly.camera.position;
                p.leg = Leg::Out;
                p.leg_start = now();
                p.latency_from = stream.latency_ms.len();
                p.generated_at_takeoff = stream.streamer.stats().generated_chunks;
                println!("autopilot: flying +x at 10 m/s for 30 s");
            }
            Leg::Out | Leg::Back => {
                let s = elapsed.min(PILOT_LEG_SECS) as f32;
                let along = if p.leg == Leg::Out {
                    s
                } else {
                    PILOT_LEG_SECS as f32 - s
                };
                self.fly.camera.position = p.origin + Vec3::X * (PILOT_SPEED * along);
                self.fly.camera.yaw = if p.leg == Leg::Out {
                    0.0
                } else {
                    std::f32::consts::PI
                };
                self.fly.camera.pitch = 0.0;
                if elapsed >= PILOT_LEG_SECS {
                    if p.leg == Leg::Out {
                        p.leg = Leg::Back;
                        println!("autopilot: flying back for 30 s");
                    } else {
                        self.fly.camera.yaw = 0.0;
                        p.leg = Leg::Return;
                        println!("autopilot: back at the dig; waiting for its chunks");
                    }
                    p.leg_start = now();
                }
            }
            Leg::Return => {
                let back = p.touched.iter().all(|&c| {
                    stream.streamer.tier(c) == Some(Tier::Full)
                        && gfx.mirror.is_mirrored(c)
                        && !gfx.mirror.is_lod(c)
                });
                if back {
                    let hash = chunks_hash(&self.world, &p.touched);
                    let verdict = if hash == Some(p.hash) {
                        "hash match"
                    } else {
                        "hash MISMATCH"
                    };
                    println!("return: {} chunks, {verdict}", p.touched.len());
                    self.pilot_report();
                    return Flow::Exit;
                }
            }
        }
        Flow::Continue
    }

    /// Per-frame flight readings: unready-near count, streaming CPU, memory peaks.
    fn pilot_sample(&mut self) {
        let in_flight = self
            .pilot
            .as_ref()
            .is_some_and(|p| matches!(p.leg, Leg::Out | Leg::Back));
        if !in_flight {
            return;
        }
        let resident = if self.frame.is_multiple_of(60) {
            Some(self.resident_ram())
        } else {
            None
        };
        let (Some(p), Some(stream), Some(gfx)) =
            (self.pilot.as_mut(), self.stream.as_ref(), self.gfx.as_ref())
        else {
            return;
        };
        p.flight_frames += 1;
        let cam = voxel_of(self.fly.camera.position).chunk();
        let unready = p.near.iter().any(|o| {
            let c = ChunkCoord([0, 1, 2].map(|k| cam.0[k] + o[k]));
            !(stream.streamer.tier(c) == Some(Tier::Full)
                && gfx.mirror.is_mirrored(c)
                && !gfx.mirror.is_lod(c))
        });
        p.unready_frames += u64::from(unready);
        if let Some(&cpu) = stream.cpu_ms.last() {
            p.cpu_ms.push(cpu);
        }
        p.peak_mirror = p.peak_mirror.max(gfx.mirror.mirror_bytes());
        p.peak_allocated = p.peak_allocated.max(gfx.mirror.allocated_bytes());
        p.peak_persisted = p
            .peak_persisted
            .max(stream.streamer.stats().persisted_ram_bytes);
        if let Some(r) = resident {
            p.peak_resident = p.peak_resident.max(r);
        }
    }

    fn pilot_report(&mut self) {
        let resident = self.resident_ram();
        let (Some(p), Some(stream), Some(gfx)) =
            (self.pilot.as_mut(), self.stream.as_mut(), self.gfx.as_ref())
        else {
            return;
        };
        let r = RESIDENCY;
        println!(
            "autopilot: unready-near frames {} of {} (chunks wanted full within {} of the camera's)",
            p.unready_frames,
            p.flight_frames,
            r.r_full - 2
        );
        let mut cpu = p.cpu_ms.clone();
        println!(
            "autopilot: streaming CPU per frame (update + poll + sync): p50 {:.3} ms, p99 {:.3} ms, max {:.3} ms over {} frames (line 1.0 ms)",
            percentile(&mut cpu, 50.0),
            percentile(&mut cpu, 99.0),
            percentile(&mut cpu, 100.0),
            cpu.len()
        );
        let mut lat = stream.latency_ms[p.latency_from..].to_vec();
        println!(
            "autopilot: residency latency wanted to mirrored: p95 {:.0} ms, max {:.0} ms over {} chunks (proposal 500 ms)",
            percentile(&mut lat, 95.0),
            percentile(&mut lat, 100.0),
            lat.len()
        );
        let st = stream.streamer.stats();
        println!(
            "autopilot: peak mirror {} B live, {} B allocated, resident RAM peak {} B (now {} B), process peak RSS {} (dev reference), persisted RAM peak {} B",
            p.peak_mirror,
            p.peak_allocated,
            p.peak_resident.max(resident),
            resident,
            peak_rss_bytes().map_or_else(|| "n/a".to_owned(), |b| format!("{b} B")),
            p.peak_persisted.max(st.persisted_ram_bytes)
        );
        if p.gpu_ms.is_empty() {
            println!("autopilot: march GPU n/a (no TIMESTAMP_QUERY)");
        } else {
            let [w, h] = [gfx.marcher.target().width(), gfx.marcher.target().height()];
            println!(
                "autopilot: march GPU at {w}x{h} over the flight: p50 {:.3} ms, p99 {:.3} ms, max {:.3} ms over {} frames",
                percentile(&mut p.gpu_ms, 50.0),
                percentile(&mut p.gpu_ms, 99.0),
                percentile(&mut p.gpu_ms, 100.0),
                p.gpu_ms.len()
            );
        }
        let generated = st.generated_chunks - p.generated_at_takeoff;
        let secs = 2.0 * PILOT_LEG_SECS + (now() - p.leg_start).as_secs_f64();
        println!(
            "autopilot: generated {generated} chunks in about {secs:.1} s on {} workers: {:.3e} voxels/s/worker",
            stream.workers,
            generated as f64 * 262_144.0 / secs / stream.workers as f64
        );
    }

    fn frame(&mut self, gpu: &Gpu, input: &Input, dt: f32, surface: &wgpu::TextureView) -> Flow {
        let t0 = now();
        if input.pressed(KeyCode::Escape) {
            return Flow::Exit;
        }
        if self.pilot.is_some() {
            if self.pilot_step() == Flow::Exit {
                return Flow::Exit;
            }
        } else {
            match self.walk.as_mut() {
                Some(w) => {
                    for line in w.update(&self.world, input, &mut self.fly.camera, dt) {
                        println!("{line}");
                    }
                }
                None => self.fly.update(input, dt),
            }
            self.handle_input(gpu, input);
        }
        if let Some(c) = self.coarse.as_mut() {
            for line in c.fall(&self.world) {
                println!("{line}");
            }
        }
        if let Some(n) = self.args.frames
            && self.frame + SAMPLES as u32 == n
        {
            self.samples = Some(Samples::default());
        }
        self.draw(gpu, surface, t0)
    }

    /// The keys and buttons of interactive flight; the autopilot takes none of them, so a
    /// stray click cannot change the world it measures.
    fn handle_input(&mut self, gpu: &Gpu, input: &Input) {
        if input.pressed(KeyCode::KeyL) {
            self.light.shadows = !self.light.shadows;
            println!(
                "shadow ray {}",
                if self.light.shadows { "on" } else { "off" }
            );
        }
        if input.scroll() != 0.0 {
            let steps = input.scroll().round() as i64;
            self.dig_radius = (self.dig_radius + steps * DIG_STEP).clamp(DIG_RANGE.0, DIG_RANGE.1);
            println!(
                "dig radius {:.1} m",
                self.dig_radius as f64 / (10 * FIXED_ONE) as f64
            );
        }
        if input.clicked(MouseButton::Left) {
            self.dig();
        }
        if input.clicked(MouseButton::Right) {
            self.fill();
        }
        if input.pressed(KeyCode::KeyB) {
            self.bench(gpu);
        }
        if input.pressed(KeyCode::KeyP) {
            self.stamp();
        }
        if input.pressed(KeyCode::KeyG) {
            self.toggle_walk();
        }
        if input.pressed(KeyCode::KeyK) {
            println!("{}", self.coarse_line());
        }
        if input.pressed(KeyCode::KeyR) {
            match self.coarse.as_mut() {
                Some(c) => println!("{}", c.toggle_crumble()),
                None => println!("collapse: not kept while streaming"),
            }
        }
        if input.pressed(KeyCode::KeyN) {
            let contour = self.gfx.as_ref().is_some_and(|g| g.mirror.surface_attrs());
            self.look.normal_mode = self.look.normal_mode.next(contour);
            println!("{}", look_line(&self.look, self.rock_detail()));
        }
        if input.pressed(KeyCode::KeyV) {
            self.look.detail = !self.look.detail;
            println!("{}", look_line(&self.look, self.rock_detail()));
        }
        if input.pressed(KeyCode::F1) {
            self.samples = Some(Samples::default());
        }
    }

    /// Streams, syncs, marches and presents one frame begun at `t0`.
    fn draw(&mut self, gpu: &Gpu, surface: &wgpu::TextureView, t0: Instant) -> Flow {
        if self.stream.is_some() {
            if let Err(e) = self.stream_step(gpu) {
                eprintln!("mirror sync failed: {e}");
                return Flow::Exit;
            }
        } else if let Some(gfx) = self.gfx.as_mut()
            && let Err(e) = gfx.mirror.sync(&self.world, &gpu.queue)
        {
            eprintln!("mirror sync failed: {e}");
            return Flow::Exit;
        }
        self.pilot_sample();
        let flying = self
            .pilot
            .as_ref()
            .is_some_and(|p| matches!(p.leg, Leg::Out | Leg::Back));

        let Some(gfx) = self.gfx.as_mut() else {
            return Flow::Exit;
        };
        gfx.marcher.set_look(self.look);
        let params = gfx
            .marcher
            .params(&gfx.mirror, &self.fly.camera, &self.light);
        let timer = gfx
            .timer
            .as_ref()
            .filter(|_| self.samples.is_some() || flying);
        let mut enc = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        gfx.mirror.update_attrs(&mut enc);
        gfx.marcher.encode(
            &gpu.device,
            &gpu.queue,
            &mut enc,
            &gfx.mirror,
            &params,
            timer.map(MarchTimer::writes),
        );
        if let Some(t) = timer {
            t.resolve(&mut enc);
        }
        gfx.presenter
            .encode(&gpu.device, &mut enc, gfx.marcher.target(), surface);
        gpu.queue.submit([enc.finish()]);
        let cpu_ms = ms_since(t0);

        if self.frame == 0 {
            // The first frame counts once the GPU has finished it.
            let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
            println!("first frame after {:.0} ms", ms_since(self.start));
        }
        let gpu_ms = timer.map(|t| t.read_ms(&gpu.device, &gpu.queue));
        if flying && let (Some(p), Some(ms)) = (self.pilot.as_mut(), gpu_ms) {
            p.gpu_ms.push(ms);
        }
        if let Some(s) = self.samples.as_mut() {
            s.cpu_ms.push(cpu_ms);
            if let Some(ms) = gpu_ms {
                s.gpu_ms.push(ms);
            }
            if s.cpu_ms.len() == SAMPLES {
                let s = self.samples.take().unwrap_or_default();
                self.report(s);
            }
        }
        self.frame += 1;
        if self.args.frames.is_some_and(|n| self.frame >= n) {
            if let Some(w) = &self.walk {
                println!(
                    "walk: foot at {:?} (fixed), {}",
                    w.state.foot.0,
                    if w.state.grounded {
                        "grounded"
                    } else {
                        "airborne"
                    }
                );
            }
            if self.coarse.is_some() {
                println!("{}", self.coarse_line());
            }
            return Flow::Exit;
        }
        Flow::Continue
    }
}

/// `look face, detail off, view shaded`, or with rock's detail under `--look`:
/// `look face, detail off (rock scale 4, strength 0.3), view shaded`.
fn look_line(look: &LookParams, rock_detail: Option<[f32; 2]>) -> String {
    let rock = rock_detail.map_or_else(String::new, |[scale, strength]| {
        format!(" (rock scale {scale}, strength {strength})")
    });
    format!(
        "look {}, detail {}{rock}, view {}",
        look.normal_mode.name(),
        if look.detail { "on" } else { "off" },
        look.view.name()
    )
}

/// Logs a teardown stage before and after it runs, like the shell's, so a hang at exit
/// names its stage.
fn teardown_stage(what: &str, f: impl FnOnce()) {
    log::info!("teardown: dropping the {what}");
    let t = now();
    f();
    log::info!("teardown: {what} dropped in {:.1} ms", ms_since(t));
}

impl Drop for Run {
    /// Stops and joins the streamer's workers first, then drops the GPU mirror and passes,
    /// then the world (swapped for an empty one, whose drop is free).
    fn drop(&mut self) {
        if let Some(stream) = self.stream.take() {
            teardown_stage("streamer", || drop(stream));
        }
        if let Some(gfx) = self.gfx.take() {
            teardown_stage("GPU mirror and passes", || drop(gfx));
        }
        let world = std::mem::replace(&mut self.world, World::new(&MaterialTable::new()));
        teardown_stage("world", || drop(world));
    }
}

/// The fixed-point centre of the voxel holding `p`.
fn centre_fixed(p: Vec3) -> Fixed3 {
    vxe_world::centre_of_voxel(voxel_of(p))
}

/// What the app builds its [`Run`] from once the device's limits are asked for.
enum Pending {
    /// The bounded level, generating on its own threads.
    Level(JoinHandle<World>),
    /// The streaming world, its workers already generating around the spawn.
    Stream(Box<(World, Streamer, usize)>),
}

/// The app around [`Run`]: the world generates on its own threads while the shell opens the
/// window and creates the device (instance creation alone can take over a second), and is
/// joined when the device limits need its size.
struct Flythrough {
    start: Instant,
    args: Args,
    camera: Camera,
    pending: Option<Pending>,
    run: Option<Run>,
}

impl App for Flythrough {
    fn device_request(&mut self, adapter: &wgpu::Adapter) -> (wgpu::Features, wgpu::Limits) {
        if let Some(p) = self.pending.take() {
            let (world, stream) = match p {
                Pending::Level(g) => (
                    g.join().unwrap_or_else(|e| std::panic::resume_unwind(e)),
                    None,
                ),
                Pending::Stream(boxed) => {
                    let (world, streamer, workers) = *boxed;
                    (
                        world,
                        Some(Stream {
                            streamer,
                            tick_ms: Vec::new(),
                            awaiting: HashMap::new(),
                            latency_ms: Vec::new(),
                            cpu_ms: Vec::new(),
                            full_residency: false,
                            workers,
                        }),
                    )
                }
            };
            let pilot = self.args.autopilot.then(|| Autopilot {
                leg: Leg::Settle,
                leg_start: self.start,
                origin: self.camera.position,
                touched: Vec::new(),
                hash: 0,
                near: ball((RESIDENCY.r_full - 2).pow(2)),
                flight_frames: 0,
                unready_frames: 0,
                gpu_ms: Vec::new(),
                cpu_ms: Vec::new(),
                latency_from: 0,
                peak_mirror: 0,
                peak_allocated: 0,
                peak_resident: 0,
                peak_persisted: 0,
                generated_at_takeoff: 0,
            });
            let coarse = stream.is_none().then(|| Coarse::build(&world));
            let walk = if self.args.walk {
                let w = Walk::at_camera(&world, &self.camera);
                if w.is_none() {
                    println!("walk: no room for the body at the spawn; flying");
                }
                w
            } else {
                None
            };
            self.run = Some(Run {
                start: self.start,
                world,
                spawn: voxel_of(self.camera.position),
                args: self.args,
                fly: Fly::new(self.camera),
                light: LightParams::headlamp(),
                look: LookParams {
                    normal_mode: self.args.look.unwrap_or_default(),
                    detail: self.args.detail,
                    view: self.args.view,
                },
                dig_radius: DIG_RADIUS,
                bench_runs: 0,
                frame: 0,
                gfx: None,
                samples: None,
                stream,
                pilot,
                walk,
                coarse,
            });
        }
        match self.run.as_mut() {
            Some(run) => run.device_request(adapter),
            None => (wgpu::Features::empty(), wgpu::Limits::default()),
        }
    }

    fn init(&mut self, gpu: &Gpu) {
        if let Some(run) = self.run.as_mut() {
            run.init(gpu);
        }
    }

    fn resized(&mut self, gpu: &Gpu) {
        if let Some(run) = self.run.as_mut() {
            run.resized(gpu);
        }
    }

    fn frame(&mut self, gpu: &Gpu, input: &Input, dt: f32, surface: &wgpu::TextureView) -> Flow {
        match self.run.as_mut() {
            Some(run) => run.frame(gpu, input, dt, surface),
            None => Flow::Exit,
        }
    }
}

/// Runs `app` without a window: a device on the high-performance adapter and an offscreen
/// target in place of the surface, the same frames until the app exits. Teardown follows
/// the windowed shell's order (the app, then the queue, device and adapter), traced alike.
fn run_headless(mut app: Flythrough, size: [u32; 2]) -> Result<(), String> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster_block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .map_err(|e| format!("no adapter: {e}"))?;
    println!("headless on {}", adapter.get_info().name);
    let (required_features, required_limits) = app.device_request(&adapter);
    let (device, queue) = pollster_block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features,
        required_limits,
        ..Default::default()
    }))
    .map_err(|e| format!("no device: {e}"))?;
    let surface_format = wgpu::TextureFormat::Rgba8Unorm;
    let gpu = Gpu {
        adapter,
        device,
        queue,
        surface_format,
        size,
    };
    app.init(&gpu);
    let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("headless surface"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: surface_format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let input = Input::default();
    let mut last = now();
    loop {
        let t = now();
        let dt = (t - last).as_secs_f32();
        last = t;
        if app.frame(&gpu, &input, dt, &view) == Flow::Exit {
            log::info!("teardown: the app asked to exit");
            break;
        }
        // Keep the queue from running ahead of the GPU, as presenting would.
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    }
    drop(view);
    drop(target);
    teardown_stage("app", || drop(app));
    let Gpu {
        adapter,
        device,
        queue,
        ..
    } = gpu;
    teardown_stage("queue", || drop(queue));
    teardown_stage("device", || drop(device));
    teardown_stage("adapter", || drop(adapter));
    Ok(())
}

/// Drives a future to completion on this thread; `wgpu`'s native futures are ready at once
/// or after a device poll.
fn pollster_block_on<F: std::future::Future>(f: F) -> F::Output {
    let waker = std::task::Waker::noop();
    let mut cx = std::task::Context::from_waker(waker);
    let mut f = std::pin::pin!(f);
    loop {
        if let std::task::Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::yield_now();
    }
}

/// The cave's material table; under `--look`, with rock's procedural detail at
/// `--rock-detail`.
fn materials(args: Args) -> MaterialTable {
    if args.look.is_some() {
        flythrough::look_materials_with(args.rock_detail)
    } else {
        cave_materials()
    }
}

fn main() {
    // The fly-through's own lines and the shell's teardown trace show by default; RUST_LOG
    // overrides.
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("error,vxe_app=info,flythrough=info"),
    )
    .init();
    let start = now();
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("flythrough: {e}");
            std::process::exit(2);
        }
    };

    let seed = args.seed;
    let (spawn, pending) = if args.level {
        let source = CaveSource::new(seed, ChunkExtent::new([0, 0, 0], LEVEL_CHUNKS));
        if args.vault {
            println!(
                "vault: {VAULT_EDGE}^3 shell room at {:?}",
                source.vault_origin().0
            );
        }
        let generation = std::thread::spawn(move || {
            let t = now();
            let world = if args.vault {
                World::generate(source.extent, &source.with_vault(), &materials(args))
            } else {
                World::generate(source.extent, &source, &materials(args))
            };
            println!(
                "generated seed {seed} over {LEVEL_CHUNKS:?} chunks in {:.0} ms: {} full bricks",
                ms_since(t),
                world.stats().full_bricks
            );
            world
        });
        (source.spawn(), Pending::Level(generation))
    } else {
        let source = CaveSource::unbounded(seed);
        let workers = std::thread::available_parallelism()
            .map_or(1, usize::from)
            .saturating_sub(1)
            .max(1);
        let world = World::new(&materials(args));
        let cfg = StreamConfig {
            residency: RESIDENCY,
            workers,
            persist: PersistConfig::default(),
        };
        let mut streamer = if args.vault {
            println!(
                "vault: {VAULT_EDGE}^3 shell room at {:?}",
                source.vault_origin().0
            );
            Streamer::new(Arc::new(source.with_vault()), cfg)
        } else {
            Streamer::new(Arc::new(source), cfg)
        };
        // Workers start on the spawn's chunks while the shell opens the window.
        streamer.update(&world, &[source.spawn()]);
        println!(
            "streaming seed {seed}: r_full {}, r_lod {}, h {}, {workers} workers, window {}",
            RESIDENCY.r_full,
            RESIDENCY.r_lod,
            RESIDENCY.hysteresis,
            flythrough::window_side(RESIDENCY)
        );
        (
            source.spawn(),
            Pending::Stream(Box::new((world, streamer, workers))),
        )
    };

    let camera = Camera {
        position: spawn.0.map(|v| v as f32 + 0.5).into(),
        yaw: 0.0,
        pitch: 0.0,
        vfov: 70f32.to_radians(),
    };
    let size = args.render.unwrap_or([1280, 720]);
    let app = Flythrough {
        start,
        args,
        camera,
        pending: Some(pending),
        run: None,
    };
    let result = if args.headless {
        run_headless(app, size)
    } else {
        let config = ShellConfig {
            title: "flythrough".to_owned(),
            size,
            lock_cursor: true,
        };
        vxe_app::run(config, app).map_err(|e| e.to_string())
    };
    if let Err(e) = result {
        eprintln!("flythrough: {e}");
        std::process::exit(1);
    }
    log::info!("teardown: done, exiting the process");
}

#[cfg(test)]
mod tests {
    use super::*;
    use flythrough::cave::SHELL;

    fn argv(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| (*s).to_owned()).collect()
    }

    /// `--walk` and `G` refuse walk mode alike while streaming; the bounded level allows it.
    #[test]
    fn walk_mode_is_refused_while_streaming() {
        assert_eq!(
            parse_args_from(argv(&["--walk"])).err().as_deref(),
            Some("--walk: walk mode needs --level (the coarse grid is not wired to streaming)")
        );
        let a = parse_args_from(argv(&["--walk", "--level"])).expect("--walk --level parses");
        assert!(a.walk && a.level);
        assert_eq!(walk_allowed(true), Err(WALK_NEEDS_LEVEL));
        assert_eq!(walk_allowed(false), Ok(()));
    }

    #[test]
    fn an_edit_prints_one_line_with_material_names() {
        let r = EditResult {
            changed_from: vec![(1, 1234), (2, 12)],
            touched: vec![
                ChunkCoord([0, 0, 0]),
                ChunkCoord([1, 0, 0]),
                ChunkCoord([1, 1, 0]),
            ],
            aabb: None,
        };
        assert_eq!(
            edit_line("carved", &r, &cave_materials(), 0.214),
            "carved: rock 1234, ore 12 (3 chunks, 0.21 ms)"
        );
        assert_eq!(
            edit_line("filled", &EditResult::default(), &cave_materials(), 0.0),
            "filled: nothing (0 chunks, 0.00 ms)"
        );
    }

    /// The `K` bookkeeping, scripted: an edit updates the kept grid and walkable map to what
    /// a rebuild from the edited world gives, and records the changed-cell count.
    #[test]
    fn an_edit_keeps_the_coarse_grid_and_walkable_map_current() {
        let world = World::generate(
            ChunkExtent::new([-1, -1, -1], [1, 1, 1]),
            &vxe_world::UniformSource(ROCK),
            &cave_materials(),
        );
        let mut c = Coarse::build(&world);
        assert_eq!(c.map.count(), 0, "a solid world has no walkable cell");
        // A 2 m box carved out of the solid: 27 bricks touched, a floor under open cells.
        let r = world
            .apply(&EditOp::box_(
                Fixed3([-8 * FIXED_ONE, -8 * FIXED_ONE, -8 * FIXED_ONE]),
                Fixed3([16 * FIXED_ONE, 16 * FIXED_ONE, 16 * FIXED_ONE]),
                Mode::Carve,
            ))
            .unwrap();
        c.update(&world, &r);
        let rebuilt = CoarseGrid::build(&world);
        let expected = WalkableMap::build(&rebuilt, WALKABLE);
        assert_eq!(c.last.map(|(m, _)| m), Some(27));
        assert!(expected.count() > 0);
        assert_eq!(c.map.cells(), expected.cells());
        assert!(c.line().starts_with(&format!(
            "coarse: {} walkable cells, last edit changed 27 cells in ",
            expected.count()
        )));
    }

    /// `P` headless: from inside a carved 16-voxel box in solid rock, the ray along +x enters
    /// rock at x = 8 and the vault lands there; aimed so the vault crosses the extent's edge it
    /// is refused; aimed out of the world it misses.
    #[test]
    fn p_stamps_the_vault_at_the_aim_point() {
        let table = cave_materials();
        let world = World::generate(
            ChunkExtent::new([-1, -1, -1], [1, 1, 1]),
            &vxe_world::UniformSource(ROCK),
            &table,
        );
        world
            .apply(&EditOp::box_(
                Fixed3([-8 * FIXED_ONE; 3]),
                Fixed3([8 * FIXED_ONE; 3]),
                Mode::Carve,
            ))
            .unwrap();
        let centre = Fixed3([FIXED_ONE / 2; 3]);
        let out = stamp_at_aim(&world, centre, [FIXED_ONE, 0, 0]);
        let r = out.as_ref().unwrap().as_ref().unwrap();
        assert_eq!(r.changed_from, vec![(ROCK, 4096)]);
        assert_eq!(r.touched, vec![ChunkCoord([0, 0, 0])]);
        assert_eq!(
            world.count_in_box([8, 0, 0], [23, 15, 15]),
            vec![(0, 2744), (SHELL, 1352)]
        );
        assert_eq!(
            world.count_in_box([8, 0, 0], [24, 16, 16]),
            vec![(0, 2744), (ROCK, 817), (SHELL, 1352)]
        );
        assert_eq!(
            stamp_line(&out, &table, 0.5),
            "stamped: rock 4096 (1 chunks, 0.50 ms)"
        );

        // From inside rock at x = 51 the ray hits that voxel, and a vault there would reach
        // x = 66, past the resident x <= 63: refused, nothing written.
        let past = Fixed3([51 * FIXED_ONE + FIXED_ONE / 2, FIXED_ONE / 2, FIXED_ONE / 2]);
        let before = world.count_in_box([-64; 3], [63; 3]);
        let out = stamp_at_aim(&world, past, [FIXED_ONE, 0, 0]);
        assert_eq!(
            stamp_line(&out, &table, 0.0),
            "stamped: refused, chunk [1, 0, 0] is not resident"
        );
        assert_eq!(world.count_in_box([-64; 3], [63; 3]), before);

        let outside = Fixed3([200 * FIXED_ONE, 0, 0]);
        let out = stamp_at_aim(&world, outside, [FIXED_ONE, 0, 0]);
        assert!(out.is_none());
        assert_eq!(
            stamp_line(&out, &table, 0.0),
            "stamped: no surface within 10 m"
        );
    }

    /// A box op over the half-open voxel bounds `lo..hi`.
    fn box_voxels(lo: [i32; 3], hi: [i32; 3], mode: Mode) -> EditOp {
        let f = |v: [i32; 3]| Fixed3(v.map(|c| i64::from(c) * FIXED_ONE));
        EditOp::box_(f(lo), f(hi), mode)
    }

    const PILLAR: ([i32; 3], [i32; 3]) = ([32, 32, 8], [40, 40, 40]);

    /// One chunk of air holding a rock floor, a pillar and a 3 x 3-cell ledge on it.
    fn ledge_world() -> World {
        let world = World::generate(
            ChunkExtent::new([0, 0, 0], [1, 1, 1]),
            &vxe_world::UniformSource(vxe_world::AIR),
            &cave_materials(),
        );
        for (lo, hi) in [
            ([0, 0, 0], [64, 64, 8]),
            PILLAR,
            ([24, 24, 40], [48, 48, 48]),
        ] {
            world.apply(&box_voxels(lo, hi, Mode::Fill(ROCK))).unwrap();
        }
        world
    }

    /// The dig's collapse, scripted: digging out the pillar reports the ledge, which falls 8
    /// voxels a frame for 4 frames and lands on the floor.
    #[test]
    fn digging_out_the_pillar_drops_the_ledge_8_voxels_a_frame() {
        let world = ledge_world();
        let mut c = Coarse::build(&world);
        let r = world
            .apply(&box_voxels(PILLAR.0, PILLAR.1, Mode::Carve))
            .unwrap();
        let changes = c.update(&world, &r);
        assert_eq!(
            c.collapse(&world, &changes),
            ["unsupported: 9 cells, {rock: 4608}"]
        );
        assert_eq!(c.query.map(|(_, n)| n), Some(1));
        assert!(c.query_line().starts_with("collapse: last query "));
        for frame in 1..=4 {
            assert_eq!(c.fall(&world), Vec::<String>::new(), "frame {frame}");
            assert_eq!(c.falling[0].fell, 8 * frame);
        }
        assert_eq!(c.fall(&world), ["landed after 4 frames, fell 32 voxels"]);
        assert!(c.falling.is_empty());
        assert_eq!(
            world.count_in_box([24, 24, 8], [47, 47, 15]),
            [(ROCK, 4608)]
        );
        assert_eq!(
            world.count_in_box([24, 24, 16], [47, 47, 47]),
            [(vxe_world::AIR, 18_432)]
        );
        assert_eq!(c.grid, CoarseGrid::build(&world));
    }

    /// `R`: the ledge is removed instead, a quarter of it piled as rubble on the floor.
    #[test]
    fn with_r_the_ledge_is_removed_leaving_a_quarter_as_rubble() {
        let world = ledge_world();
        let mut c = Coarse::build(&world);
        assert_eq!(
            c.toggle_crumble(),
            "collapse: remove unsupported regions with rubble"
        );
        let r = world
            .apply(&box_voxels(PILLAR.0, PILLAR.1, Mode::Carve))
            .unwrap();
        let changes = c.update(&world, &r);
        assert_eq!(
            c.collapse(&world, &changes),
            [
                "unsupported: 9 cells, {rock: 4608}",
                "removed: {rock: 4608}, 1152 rubble"
            ]
        );
        assert!(c.falling.is_empty());
        assert_eq!(
            world.count_in_box([24, 24, 8], [47, 47, 9]),
            [(RUBBLE, 1152)]
        );
        assert_eq!(c.grid, CoarseGrid::build(&world));
        assert_eq!(c.toggle_crumble(), "collapse: drop unsupported regions");
    }

    /// The support query after digs from the spawn of the seed-1 level, one along each axis
    /// at the default radius, printed. Generating the level takes minutes unoptimized; run
    /// with `cargo test --release -p flythrough -- --ignored --nocapture
    /// a_dig_in_the_level_prints_the_query_time`.
    #[test]
    #[ignore = "a reading on the generated level, run in release"]
    fn a_dig_in_the_level_prints_the_query_time() {
        let source = CaveSource::new(1, ChunkExtent::new([0, 0, 0], LEVEL_CHUNKS));
        let world = World::generate(source.extent, &source, &cave_materials());
        let mut c = Coarse::build(&world);
        let origin = vxe_world::centre_of_voxel(source.spawn());
        let f = FIXED_ONE;
        for dir in [
            [f, 0, 0],
            [-f, 0, 0],
            [0, f, 0],
            [0, -f, 0],
            [0, 0, f],
            [0, 0, -f],
        ] {
            let op = EditOp::tool_along_ray(&world, origin, dir, REACH, DIG_RADIUS, Mode::Carve);
            let r = world.apply(&op).unwrap();
            let changes = c.update(&world, &r);
            let lines = c.collapse(&world, &changes);
            println!(
                "dig {dir:?}: {} voxels, {} changed cells; {}; {lines:?}",
                r.changed_from.iter().map(|(_, n)| n).sum::<u32>(),
                changes.len(),
                c.query_line()
            );
        }
    }

    #[test]
    fn rock_detail_parses_scale_and_strength() {
        assert_eq!(parse_rock_detail("8,0.6"), Ok([8.0, 0.6]));
        assert_eq!(parse_rock_detail("2.5, 0"), Ok([2.5, 0.0]));
        assert!(parse_rock_detail("4").is_err());
        assert!(parse_rock_detail("0,0.3").is_err());
        assert!(parse_rock_detail("4,1.5").is_err());
        assert!(parse_rock_detail("4,x").is_err());
    }

    #[test]
    fn the_look_line_names_rock_detail_under_look() {
        let look = LookParams {
            normal_mode: NormalMode::Face,
            detail: true,
            view: ViewMode::Shaded,
        };
        assert_eq!(
            look_line(&look, Some([4.0, 0.3])),
            "look face, detail on (rock scale 4, strength 0.3), view shaded"
        );
        assert_eq!(look_line(&look, None), "look face, detail on, view shaded");
    }

    #[test]
    fn the_first_frame_waits_for_33_chunks() {
        assert_eq!(ball(NEAR_D2).len(), 33);
    }
}
