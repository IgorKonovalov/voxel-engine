//! The engine's testbed (milestone 1 in `docs/roadmap.md`): a window showing the ray-marched
//! cave lit by a headlamp. Fly, dig, and read the per-material counts each edit returns.
//!
//! By default the cave is unbounded and streams around the camera (ADR-0003): full detail
//! within `r_full` chunks, mips out to `r_lod`. `--level` generates the bounded level instead.
//!
//! `flythrough [--seed N] [--level] [--render WxH] [--frames N] [--bench N] [--autopilot]
//! [--sync-budget MIB] [--headless] [--look MODE] [--rock-detail SCALE,STRENGTH] [--view VIEW]
//! [--detail]`. `--render` fixes the marcher's resolution (the window
//! scales it); `--frames` exits after N frames, printing the `F1` readings over the last
//! ones; `--bench` runs the `B` bench N times right after the first mirror upload, before
//! the first frame; `--autopilot` digs, flies 300 m out and back and prints the streaming
//! readings (see [`Autopilot`]); `--sync-budget` sets the mirror's upload bytes per frame;
//! `--headless` runs the same frames on an offscreen target with no window; `--look` starts
//! in a normal mode (`face`, `trilinear8`, `gradient27`, `contour`) and gives the mirror
//! attribute pages, so `N` can reach `contour` (without `--look` it skips it), and gives rock
//! its procedural detail (`--rock-detail` sets rock's feature scale in voxels and strength,
//! default `flythrough::ROCK_DETAIL`); `--view` starts in a view (`shaded`, `normals`, `hitmask`) and
//! `--detail` with detail on. `N` cycles the normal mode and `V` toggles detail.

mod fly;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;

use fly::Fly;
use flythrough::cave::{CaveSource, LEVEL_CHUNKS, ROCK};
use flythrough::{RESIDENCY, cave_materials, chunk_ram, chunks_hash, mips_ram, percentile};
use vxe_app::{App, Flow, Gpu, Input, KeyCode, MouseButton, ShellConfig};
use vxe_render::glam::Vec3;
use vxe_render::gpu_world;
use vxe_render::{
    Camera, GpuWorld, LightParams, LodParams, LookParams, MarchTimer, Marcher, MirrorConfig,
    NormalMode, Presenter, ViewMode, Window, wgpu,
};
use vxe_world::hash::mix64;
use vxe_world::{
    ChunkCoord, ChunkExtent, EditError, EditOp, EditResult, FIXED_ONE, Fixed3, MaterialId,
    MaterialTable, Mode, PersistConfig, StreamConfig, Streamer, Tier, VoxelCoord, World,
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
    sync_budget: u64,
    /// `--look`: the starting normal mode.
    look: Option<NormalMode>,
    /// `--detail`: start with procedural detail on.
    detail: bool,
    /// `--rock-detail`: rock's feature scale (voxels) and strength under `--look`.
    rock_detail: [f32; 2],
    view: ViewMode,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        seed: 1,
        render: None,
        frames: None,
        bench: 0,
        level: false,
        autopilot: false,
        headless: false,
        sync_budget: SYNC_BUDGET_MIB << 20,
        look: None,
        detail: false,
        rock_detail: flythrough::ROCK_DETAIL,
        view: ViewMode::Shaded,
    };
    let mut it = std::env::args().skip(1);
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
            _ => return Err(format!("unknown argument {flag}")),
        }
    }
    if args.rock_detail != flythrough::ROCK_DETAIL && args.look.is_none() {
        return Err("--rock-detail applies under --look".to_owned());
    }
    if args.autopilot && args.level {
        return Err("--autopilot streams; it does not take --level".to_owned());
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
}

impl Run {
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

    fn apply(&self, verb: &str, op: &EditOp) {
        let t = now();
        let r = self.world.apply(op);
        let ms = ms_since(t);
        match r {
            Ok(r) => println!("{}", edit_line(verb, &r, self.world.materials(), ms)),
            Err(e) => println!("{verb}: refused, {e}"),
        }
    }

    fn dig(&self) {
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
            Ok(r) => println!("{}", edit_line("carved", &r, self.world.materials(), ms)),
            Err(e) => println!("carved: refused, {e}"),
        }
    }

    fn fill(&self) {
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
            self.fly.update(input, dt);
            self.handle_input(gpu, input);
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
/// target in place of the surface, the same frames until the app exits.
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
            return Ok(());
        }
        // Keep the queue from running ahead of the GPU, as presenting would.
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    }
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
    env_logger::init();
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
        let generation = std::thread::spawn(move || {
            let t = now();
            let world = World::generate(source.extent, &source, &materials(args));
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
        let mut streamer = Streamer::new(
            Arc::new(source),
            StreamConfig {
                residency: RESIDENCY,
                workers,
                persist: PersistConfig::default(),
            },
        );
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
