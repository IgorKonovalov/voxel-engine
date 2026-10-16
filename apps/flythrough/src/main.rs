//! The engine's testbed (milestone 1 in `docs/roadmap.md`): a window showing the ray-marched
//! cave lit by a headlamp. Fly, dig, and read the per-material counts each edit returns.
//!
//! `flythrough [--seed N] [--render WxH] [--frames N]`. `--render` fixes the marcher's
//! resolution (the window scales it); `--frames` exits after N frames, printing the `F1`
//! readings over the last ones.

mod fly;

use std::collections::{BTreeMap, BTreeSet};
use std::thread::JoinHandle;
use std::time::Instant;

use fly::Fly;
use flythrough::cave::{CaveSource, LEVEL_CHUNKS, ROCK};
use flythrough::cave_materials;
use vxe_app::{App, Flow, Gpu, Input, KeyCode, MouseButton, ShellConfig};
use vxe_render::{Camera, GpuWorld, LightParams, MarchTimer, Marcher, Presenter, wgpu};
use vxe_world::hash::mix64;
use vxe_world::{
    ChunkExtent, EditError, EditOp, EditResult, FIXED_ONE, Fixed3, MaterialId, MaterialTable, Mode,
    World,
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
/// Free pool slots requested beyond the generated `Full` bricks, so digging can grow the pool:
/// 64 Ki bricks, 64 MiB.
const POOL_HEADROOM_BRICKS: u64 = 65_536;
/// Frames `F1` averages over.
const SAMPLES: usize = 30;

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
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        seed: 1,
        render: None,
        frames: None,
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
            _ => return Err(format!("unknown argument {flag}")),
        }
    }
    Ok(args)
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

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

struct Gfx {
    mirror: GpuWorld,
    marcher: Marcher,
    presenter: Presenter,
    timer: Option<MarchTimer>,
}

#[derive(Default)]
struct Samples {
    cpu_ms: Vec<f64>,
    gpu_ms: Vec<f64>,
}

/// The running fly-through, once the world exists.
struct Run {
    start: Instant,
    world: World,
    args: Args,
    fly: Fly,
    light: LightParams,
    dig_radius: i64,
    bench_runs: u64,
    frame: u32,
    gfx: Option<Gfx>,
    samples: Option<Samples>,
}

impl Run {
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

        let t = now();
        let results: Vec<Result<EditResult, EditError>> =
            ops.iter().map(|op| self.world.apply(op)).collect();
        let edit_ms = ms_since(t);

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
                 edits {edit_ms:.2} ms CPU; sync uploaded {} B ({} bricks, {} chunks) in {:.2} ms",
                s.bytes,
                s.bricks,
                s.chunks,
                ms_since(t)
            ),
            Err(e) => println!("bench: sync failed, {e}"),
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
            "frame ({w}x{h}, median of {}): cpu {:.3} ms, gpu march {gpu}; full bricks {}, mirror {} B",
            cpu_ms.len(),
            median(&mut cpu_ms),
            self.world.stats().full_bricks,
            gfx.mirror.mirror_bytes()
        );
    }
}

impl Run {
    fn device_request(&mut self, adapter: &wgpu::Adapter) -> (wgpu::Features, wgpu::Limits) {
        let full = self.world.stats().full_bricks as u64;
        let pool = (full + POOL_HEADROOM_BRICKS) * 1024;
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
        let mut mirror = GpuWorld::new(&gpu.device, self.world.extent());
        let t = now();
        match mirror.sync(&self.world, &gpu.queue) {
            Ok(s) => println!(
                "mirror: uploaded {} B ({} bricks, {} chunks) in {:.0} ms",
                s.bytes,
                s.bricks,
                s.chunks,
                ms_since(t)
            ),
            Err(e) => eprintln!("mirror sync failed: {e}"),
        }
        self.gfx = Some(Gfx {
            mirror,
            marcher: Marcher::new(&gpu.device, w, h),
            presenter: Presenter::new(&gpu.device, gpu.surface_format),
            timer,
        });
    }

    fn resized(&mut self, gpu: &Gpu) {
        if let (Some(gfx), None) = (self.gfx.as_mut(), self.args.render) {
            gfx.marcher.resize(&gpu.device, gpu.size[0], gpu.size[1]);
        }
    }

    fn frame(&mut self, gpu: &Gpu, input: &Input, dt: f32, surface: &wgpu::TextureView) -> Flow {
        let t0 = now();
        if input.pressed(KeyCode::Escape) {
            return Flow::Exit;
        }
        self.fly.update(input, dt);
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
        if input.pressed(KeyCode::F1) {
            self.samples = Some(Samples::default());
        }
        if let Some(n) = self.args.frames
            && self.frame + SAMPLES as u32 == n
        {
            self.samples = Some(Samples::default());
        }

        let Some(gfx) = self.gfx.as_mut() else {
            return Flow::Exit;
        };
        if let Err(e) = gfx.mirror.sync(&self.world, &gpu.queue) {
            eprintln!("mirror sync failed: {e}");
            return Flow::Exit;
        }
        let params = gfx
            .marcher
            .params(&gfx.mirror, &self.fly.camera, &self.light);
        let timer = gfx.timer.as_ref().filter(|_| self.samples.is_some());
        let mut enc = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
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
        if let Some(s) = self.samples.as_mut() {
            s.cpu_ms.push(cpu_ms);
            if let Some(t) = timer {
                s.gpu_ms.push(t.read_ms(&gpu.device, &gpu.queue));
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

/// The app around [`Run`]: the world generates on its own threads while the shell opens the
/// window and creates the device (instance creation alone can take over a second), and is
/// joined when the device limits need its size.
struct Flythrough {
    start: Instant,
    args: Args,
    camera: Camera,
    generation: Option<JoinHandle<World>>,
    run: Option<Run>,
}

impl App for Flythrough {
    fn device_request(&mut self, adapter: &wgpu::Adapter) -> (wgpu::Features, wgpu::Limits) {
        if let Some(g) = self.generation.take() {
            let world = g.join().unwrap_or_else(|e| std::panic::resume_unwind(e));
            self.run = Some(Run {
                start: self.start,
                world,
                args: self.args,
                fly: Fly::new(self.camera),
                light: LightParams::headlamp(),
                dig_radius: DIG_RADIUS,
                bench_runs: 0,
                frame: 0,
                gfx: None,
                samples: None,
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

    let source = CaveSource::new(args.seed, ChunkExtent::new([0, 0, 0], LEVEL_CHUNKS));
    let seed = args.seed;
    let generation = std::thread::spawn(move || {
        let t = now();
        let world = World::generate(source.extent, &source, &cave_materials());
        println!(
            "generated seed {seed} over {LEVEL_CHUNKS:?} chunks in {:.0} ms: {} full bricks",
            ms_since(t),
            world.stats().full_bricks
        );
        world
    });

    let spawn = source.spawn().0.map(|v| v as f32 + 0.5);
    let camera = Camera {
        position: spawn.into(),
        yaw: 0.0,
        pitch: 0.0,
        vfov: 70f32.to_radians(),
    };
    let size = args.render.unwrap_or([1280, 720]);
    let app = Flythrough {
        start,
        args,
        camera,
        generation: Some(generation),
        run: None,
    };
    let config = ShellConfig {
        title: "flythrough".to_owned(),
        size,
        lock_cursor: true,
    };
    if let Err(e) = vxe_app::run(config, app) {
        eprintln!("flythrough: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vxe_world::ChunkCoord;

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
}
