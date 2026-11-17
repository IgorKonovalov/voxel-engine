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
use crate::run::Run;
use crate::run::hud_feed::{HUD_SCALE, Hud};
use crate::run::loading::{draw_view, scene_title};
use crate::run::queue::QueueRun;
use crate::setup::{castle_banner, shell_config};
use crate::streaming::{STREAM_POOL_BYTES, Stream};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;
use vxe_app::{App, Flow, Gpu, Input};
use vxe_net::Logic;
use vxe_render::{Camera, LightParams, LoadingScreen, LoadingView, wgpu};
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
    /// The streaming world whose source builds on its own thread first (the castle's
    /// prefabs); its workers start when the build ends.
    StreamBuilding(JoinHandle<Box<(World, Streamer, usize)>>),
}

/// The app around [`Run`]: the world generates on its own threads while the shell opens the
/// window and creates the device (instance creation alone can take over a second). A bounded
/// level is joined when the device's limits need its size; a streamed world, whose limits do
/// not read its content, after the device and the first loading frame.
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
    /// The loading screen [`App::splash`] drew the first frame with, handed to the run's
    /// loading frames.
    pub(crate) splash: Option<LoadingScreen>,
}

impl Flythrough {
    /// The run over `world`, streamed when `stream` is given, built from the app's arguments.
    pub(crate) fn start_run(&mut self, world: World, stream: Option<Stream>) {
        let pilot = self
            .args
            .autopilot
            .then(|| Autopilot::new(self.start, self.camera.position, self.args.residency.r_full));
        diag(format_args!("generation joined"));
        let coarse = stream.is_none().then(|| Coarse::build(&world));
        diag(format_args!("coarse grid built"));
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
            modes: self
                .args
                .start_modes(0.0)
                .with_lights(
                    crate::lights::TestLights::ring(
                        self.args.light.lights,
                        self.camera.position.to_array(),
                    )
                    .bouncing_all(self.args.light.lights_bounce),
                )
                .with_walkers(
                    crate::walker::Walkers::with_ring_walking(
                        self.args.entities,
                        self.args.entities_walk,
                    )
                    .with_viewer(self.args.model.as_ref().and_then(|path| {
                        crate::viewer::Viewer::open(path, self.args.clip.clone())
                            .inspect_err(|e| println!("{e}"))
                            .ok()
                    })),
                ),
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
            surface: None,
            help: false,
            menu: Default::default(),
            notice: None,
            loading: None,
            switch: false,
            retree: false,
        });
    }

    /// The streamed run, its world's build joined first when it is still on its thread.
    pub(crate) fn start_streamed_run(&mut self, pending: Pending) {
        let boxed = match pending {
            Pending::Stream(boxed) => boxed,
            Pending::StreamBuilding(build) => build
                .join()
                .unwrap_or_else(|e| std::panic::resume_unwind(e)),
            Pending::Level(_) | Pending::Failed => return,
        };
        let (world, streamer, workers) = *boxed;
        self.start_run(world, Some(Stream::new(streamer, workers)));
    }
}

impl App for Flythrough {
    fn device_request(&mut self, adapter: &wgpu::Adapter) -> (wgpu::Features, wgpu::Limits) {
        diag(format_args!("window and adapter ready"));
        match self.pending.take() {
            Some(Pending::Level(g)) => {
                match g.join().unwrap_or_else(|e| std::panic::resume_unwind(e)) {
                    Ok(world) => self.start_run(world, None),
                    Err(e) => {
                        self.failure.set(e);
                        return (wgpu::Features::empty(), wgpu::Limits::default());
                    }
                }
            }
            Some(Pending::Failed) => {
                return (wgpu::Features::empty(), wgpu::Limits::default());
            }
            Some(streamed) => {
                // A streamed world's limits do not read its content: its run is built in
                // `init`, after the device and the first loading frame.
                self.pending = Some(streamed);
                let request = crate::run::device_request_for(adapter, STREAM_POOL_BYTES);
                diag(format_args!("device limits sized"));
                return request;
            }
            None => {}
        }
        match self.run.as_mut() {
            Some(run) => {
                let request = run.device_request(adapter);
                diag(format_args!("device limits sized"));
                request
            }
            None => (wgpu::Features::empty(), wgpu::Limits::default()),
        }
    }

    /// The first frame, before the world's run is built: a static loading frame (the
    /// background, the scene's name, an empty bar) when the loading screen is on.
    fn splash(&mut self, gpu: &Gpu, surface: &wgpu::TextureView) -> bool {
        if !self.args.loading.screen_on(self.args.headless) || self.failure.is_some() {
            return false;
        }
        let mut screen = LoadingScreen::new(&gpu.device, gpu.surface_format, HUD_SCALE);
        let title = format!("loading {}", scene_title(&self.args));
        draw_view(
            gpu,
            surface,
            &mut screen,
            &LoadingView {
                title: &title,
                stage: "starting",
                detail: "",
                progress: 0.0,
                seconds: 0.0,
                alpha: 1.0,
            },
        );
        diag(format_args!("splash drawn"));
        self.splash = Some(screen);
        true
    }

    fn init(&mut self, gpu: &Gpu) {
        if self.args.headless {
            let info = gpu.adapter.get_info();
            println!(
                "headless on {} ({})",
                info.name,
                vxe_app::backend_label(info.backend)
            );
        }
        if let Some(pending) = self.pending.take() {
            self.start_streamed_run(pending);
        }
        if let Some(run) = self.run.as_mut() {
            run.init(gpu, self.splash.take());
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
        let flow = match self.run.as_mut() {
            Some(run) => run.frame(gpu, input, dt, surface),
            None => Flow::Exit,
        };
        if flow == Flow::Continue && self.run.as_ref().is_some_and(|r| r.switch || r.retree) {
            self.switch_source(gpu);
        }
        flow
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
    let size = app
        .args
        .render
        .or(app.args.output_hint)
        .unwrap_or([1280, 720]);
    let failure = app.failure.clone();
    if app.args.headless {
        // On the high-performance adapter, or with `--adapter` the first whose name contains
        // its substring, and an offscreen target in place of the surface.
        let config = vxe_app::HeadlessConfig {
            size,
            adapter: app.args.adapter.clone(),
            ..vxe_app::HeadlessConfig::default()
        };
        vxe_app::run_headless(app, config)
            .map_err(|e| e.to_string())
            .and_then(|()| session_outcome(&failure))
    } else {
        let config = vxe_app::ShellConfig {
            start: Some(app.start),
            ..shell_config(&app.args, size, app.args.pace)
        };
        diag(format_args!("set up, the shell starting"));
        vxe_app::run(config, app)
            .map_err(|e| e.to_string())
            .and_then(|()| session_outcome(&failure))
    }
}

/// `Err` with the session's failure, if one was recorded; the headless runner's and the
/// windowed shell's result once their teardown is done.
pub(crate) fn session_outcome(failure: &Failure) -> Result<(), String> {
    failure.take().map_or(Ok(()), Err)
}
