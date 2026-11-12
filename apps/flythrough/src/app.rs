//! The fly-through as a `vxe_app` app ([`Flythrough`]), its headless loop and the session's
//! outcome.

use crate::ScrollSteps;
use crate::audio::{Audio, RoomProbe};
use crate::autopilot::Autopilot;
use crate::cli::Args;
use crate::edits::{Coarse, Failure, Net, voxel_of};
use crate::fly::{Fly, Walk};
use crate::hud::{self};
use crate::now;
use crate::pad::Pad;
use crate::path::PathPlayer;
use crate::report::Recorder;
use crate::run::hud_feed::Hud;
use crate::run::queue::QueueRun;
use crate::run::{Run, teardown_stage};
use crate::setup::{castle_banner, shell_config};
use crate::streaming::Stream;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;
use vxe_app::{App, Flow, Gpu, Input};
use vxe_net::Logic;
use vxe_render::{Camera, LightParams, wgpu};
use vxe_world::{Streamer, World};

#[allow(
    clippy::disallowed_methods,
    reason = "the fly-through times startup, edits and frames for its printed readings"
)]
pub(crate) fn ms_since(t: Instant) -> f64 {
    (now() - t).as_secs_f64() * 1e3
}

// DIAGNOSTIC: the headless start-up and frame markers (the `diag:` lines) read on CI's WARP
// adapter, where a 3-frame headless run hangs before its teardown (the backlog's WARP
// exposure-hang entry). They come out with the fix for that hang.
static DIAG_START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

/// DIAGNOSTIC: logs `diag: <what> at <ms> ms` at info, timed from the process start, so a
/// run killed mid-way shows the last stage it reached and when.
pub(crate) fn diag(what: std::fmt::Arguments<'_>) {
    let start = *DIAG_START.get_or_init(now);
    log::info!("diag: {what} at {:.0} ms", ms_since(start));
}

/// The process's peak resident set in bytes: on Windows `GetProcessMemoryInfo`'s
/// `PeakWorkingSetSize`, elsewhere `VmHWM` from `/proc/self/status`; `None` where neither reads.
#[cfg(windows)]
pub(crate) fn peak_rss_bytes() -> Option<u64> {
    hud::process_memory().map(|m| m.PeakWorkingSetSize as u64)
}

/// The process's peak resident set (`VmHWM`) in bytes, read from `/proc/self/status`; `None`
/// where that file does not exist or does not carry the line.
#[cfg(not(windows))]
pub(crate) fn peak_rss_bytes() -> Option<u64> {
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

/// What the app builds its [`Run`] from once the device's limits are asked for.
pub(crate) enum Pending {
    /// The bounded level, generating on its own threads; `Err` when it cannot be built (a
    /// replay log that does not re-simulate).
    Level(JoinHandle<Result<World, String>>),
    /// No world: the session failed before generation (a replay log that does not read).
    Failed,
    /// The streaming world, its workers already generating around the spawn.
    Stream(Box<(World, Streamer, usize)>),
}

/// The app around [`Run`]: the world generates on its own threads while the shell opens the
/// window and creates the device (instance creation alone can take over a second), and is
/// joined when the device limits need its size.
pub struct Flythrough {
    pub(crate) start: Instant,
    pub(crate) args: Args,
    pub(crate) camera: Camera,
    /// The server's logic, loaded before the window opens; taken when the run starts.
    pub(crate) logic: Option<Box<dyn Logic>>,
    pub(crate) pending: Option<Pending>,
    pub(crate) run: Option<Run>,
    pub(crate) audio: Option<Audio>,
    pub(crate) room_probe: Option<RoomProbe>,
    /// The gamepad, `None` under `--pad off`, headless, the autopilot, or when `gilrs`
    /// cannot start.
    pub(crate) pad: Option<Pad>,
    /// The session's failure, shared with the run and `main`.
    pub(crate) failure: Failure,
}

impl App for Flythrough {
    fn device_request(&mut self, adapter: &wgpu::Adapter) -> (wgpu::Features, wgpu::Limits) {
        if let Some(p) = self.pending.take() {
            let (world, stream) = match p {
                Pending::Level(g) => {
                    match g.join().unwrap_or_else(|e| std::panic::resume_unwind(e)) {
                        Ok(world) => (world, None),
                        Err(e) => {
                            self.failure.set(e);
                            return (wgpu::Features::empty(), wgpu::Limits::default());
                        }
                    }
                }
                Pending::Failed => return (wgpu::Features::empty(), wgpu::Limits::default()),
                Pending::Stream(boxed) => {
                    let (world, streamer, workers) = *boxed;
                    (world, Some(Stream::new(streamer, workers)))
                }
            };
            let pilot = self.args.autopilot.then(|| {
                Autopilot::new(self.start, self.camera.position, self.args.residency.r_full)
            });
            let coarse = stream.is_none().then(|| Coarse::build(&world));
            let world = Arc::new(world);
            let logic = self
                .logic
                .take()
                .expect("the logic is loaded once, in main");
            let net = Net::new(&world, logic, self.args.seed);
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
                failure: self.failure.clone(),
                world,
                net: Some(net),
                spawn: voxel_of(self.camera.position),
                args: self.args.clone(),
                fly: Fly::new(self.camera),
                light: LightParams::headlamp(),
                modes: self.args.start_modes(0.0),
                upscaler_changed: false,
                sharpen_changed: false,
                scene: self
                    .args
                    .castle
                    .then(|| (castle_banner(self.args.castle_shot), ms_since(self.start))),
                castle_shot: self.args.castle.then_some(self.args.castle_shot),
                pose: self
                    .args
                    .pose
                    .map(|p| (hud::pose_banner(p.name), self.camera)),
                scroll: ScrollSteps::default(),
                triggers: [false; 2],
                bench_runs: 0,
                frame: 0,
                gfx: None,
                samples: None,
                window_ms: [Vec::new(), Vec::new()],
                fade: Vec::new(),
                stream,
                pilot,
                walk,
                coarse,
                hud: Hud::new(self.args.hud),
                audio: self.audio.take(),
                room_probe: self.room_probe.take(),
                exposure_print: None,
                queue: QueueRun::default(),
                player: self
                    .args
                    .path
                    .map(|p| PathPlayer::new(p, self.args.path_mode)),
                recorder: self.args.path.map(|p| Recorder::for_path(p.duration_s())),
                adapter: None,
                help: false,
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

    fn poll(&mut self, input: &mut Input) {
        let Some(pad) = self.pad.as_mut() else {
            return;
        };
        for line in pad.poll(input) {
            println!("{line}");
        }
        if let Some(run) = self.run.as_mut() {
            pad.write_row(run.hud.stats.pad.get_or_insert_with(Default::default));
        }
    }

    fn frame(&mut self, gpu: &Gpu, input: &Input, dt: f32, surface: &wgpu::TextureView) -> Flow {
        if self.failure.is_some() {
            return Flow::Exit;
        }
        match self.run.as_mut() {
            Some(run) => run.frame(gpu, input, dt, surface),
            None => Flow::Exit,
        }
    }
}

/// The session's start, which [`diag`]'s lines are timed from.
pub fn start_clock() -> Instant {
    let start = now();
    DIAG_START.get_or_init(|| start);
    start
}

/// Runs `app` in a window, or on an offscreen target with no window under `--headless`, at
/// `--render`'s size (1280 x 720 by default), and gives the session's outcome.
pub fn run(app: Flythrough) -> Result<(), String> {
    let size = app.args.render.unwrap_or([1280, 720]);
    let failure = app.failure.clone();
    if app.args.headless {
        run_headless(app, size)
    } else {
        let config = shell_config(&app.args, size, app.args.pace);
        vxe_app::run(config, app)
            .map_err(|e| e.to_string())
            .and_then(|()| session_outcome(&failure))
    }
}

/// Runs `app` without a window: a device on the high-performance adapter, or with
/// `--adapter` the first adapter whose name contains its substring (the high-performance one
/// when none does), and an offscreen target in place of the surface, the same frames until
/// the app exits. Teardown follows the windowed shell's order (the app, then the queue,
/// device and adapter), traced alike. The instance reads wgpu's environment variables
/// (`WGPU_BACKEND`, `WGPU_DX12_COMPILER`, ...) as the windowed shell's does.
/// Returns the session's failure as `Err` once the teardown is done.
pub(crate) fn run_headless(mut app: Flythrough, size: [u32; 2]) -> Result<(), String> {
    let failure = app.failure.clone();
    diag(format_args!("headless start"));
    let desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
    // DIAGNOSTIC, out with the WARP preload fix: names the flags `WGPU_VALIDATION`,
    // `WGPU_GPU_BASED_VALIDATION` and the build left on, so CI's debug-layer runs show the
    // request reached wgpu (whether the layer loaded is the runner's to say).
    diag(format_args!("instance flags {:?}", desc.flags));
    let instance = wgpu::Instance::new(desc);
    let named = app.args.adapter.as_deref().and_then(|want| {
        let all = pollster_block_on(instance.enumerate_adapters(wgpu::Backends::all()));
        let found = all.into_iter().find(|a| a.get_info().name.contains(want));
        if found.is_none() {
            eprintln!("--adapter {want:?}: no adapter's name contains it; the default is used");
        }
        found
    });
    let adapter = match named {
        Some(a) => a,
        None => pollster_block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .map_err(|e| format!("no adapter: {e}"))?,
    };
    let info = adapter.get_info();
    println!(
        "headless on {} ({})",
        info.name,
        vxe_app::backend_label(info.backend)
    );
    diag(format_args!(
        "adapter {} ({}), driver {:?} {:?}, max storage binding {} B, max buffer {} B, timestamps {}",
        info.name,
        vxe_app::backend_label(info.backend),
        info.driver,
        info.driver_info,
        adapter.limits().max_storage_buffer_binding_size,
        adapter.limits().max_buffer_size,
        adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY)
    ));
    let (required_features, required_limits) = app.device_request(&adapter);
    let (device, queue) = pollster_block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features,
        required_limits,
        ..Default::default()
    }))
    .map_err(|e| format!("no device: {e}"))?;
    diag(format_args!("device created"));
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
    let mut diag_frame = 0u32;
    loop {
        let t = now();
        let dt = (t - last).as_secs_f32();
        last = t;
        diag(format_args!("frame {diag_frame}: start"));
        if app.frame(&gpu, &input, dt, &view) == Flow::Exit {
            log::info!("teardown: the app asked to exit");
            break;
        }
        diag(format_args!(
            "frame {diag_frame}: returned, waiting for the GPU"
        ));
        // Keep the queue from running ahead of the GPU, as presenting would.
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        diag(format_args!("frame {diag_frame}: the GPU finished"));
        diag_frame += 1;
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
    session_outcome(&failure)
}

/// `Err` with the session's failure, if one was recorded; the headless runner's and the
/// windowed shell's result once their teardown is done.
pub(crate) fn session_outcome(failure: &Failure) -> Result<(), String> {
    failure.take().map_or(Ok(()), Err)
}

/// Drives a future to completion on this thread; `wgpu`'s native futures are ready at once
/// or after a device poll.
pub(crate) fn pollster_block_on<F: std::future::Future>(f: F) -> F::Output {
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
