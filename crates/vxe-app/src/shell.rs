//! The window and frame loop: a `winit` 0.30 `ApplicationHandler` that opens one window,
//! creates the `wgpu` device and surface on first resume, and hands each frame to an [`App`].

use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::PhysicalKey;
use winit::window::{CursorGrabMode, Window, WindowId};

use crate::input::Input;

/// What the shell created for the app: the device, its queue and the surface's state.
pub struct Gpu {
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    /// Format of the surface textures handed to [`App::frame`].
    pub surface_format: wgpu::TextureFormat,
    /// The colour space the surface was configured with; `None` headless, where no display
    /// reads the target. `Auto` when the surface offers no `Srgb` space for the format.
    pub surface_color_space: Option<wgpu::SurfaceColorSpace>,
    /// Surface size in physical pixels.
    pub size: [u32; 2],
}

/// Whether the loop keeps running after a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Exit,
}

/// The user side of the shell.
pub trait App {
    /// Features and limits to request from `adapter`. The default asks for none beyond the
    /// `wgpu` defaults.
    fn device_request(&mut self, adapter: &wgpu::Adapter) -> (wgpu::Features, wgpu::Limits) {
        let _ = adapter;
        (wgpu::Features::empty(), wgpu::Limits::default())
    }

    /// Called once the device exists and the surface is configured, before [`init`](Self::init):
    /// draws the window's first frame into `surface` and submits it, so the window shows
    /// something while `init` and the rest of start-up run. Returns whether it drew; the shell
    /// presents the frame only then. The default draws nothing. Not called without a window.
    fn splash(&mut self, gpu: &Gpu, surface: &wgpu::TextureView) -> bool {
        let _ = (gpu, surface);
        false
    }

    /// Called once the device exists, before the first frame.
    fn init(&mut self, gpu: &Gpu);

    /// Called after the surface was reconfigured to `gpu.size`.
    fn resized(&mut self, gpu: &Gpu) {
        let _ = gpu;
    }

    /// Called before every [`frame`](Self::frame) with the input the frame will see, so the
    /// app can feed sources the shell does not read (a gamepad's axes and buttons). The
    /// default does nothing.
    fn poll(&mut self, input: &mut Input) {
        let _ = input;
    }

    /// Draws one frame into `surface` and submits the work; the shell presents afterwards.
    /// `dt` is the time since the previous frame in seconds.
    fn frame(&mut self, gpu: &Gpu, input: &Input, dt: f32, surface: &wgpu::TextureView) -> Flow;
}

#[derive(Clone, Debug)]
pub struct ShellConfig {
    pub title: String,
    /// Requested inner size in physical pixels; the window manager may override it.
    pub size: [u32; 2],
    /// Hide and lock the cursor so mouse motion is relative (mouse look).
    pub lock_cursor: bool,
    /// The surface's present mode. `AutoNoVsync` presents as fast as frames finish;
    /// `AutoVsync` waits for the display.
    pub present_mode: wgpu::PresentMode,
    /// Which adapter the shell asks for.
    pub power_preference: wgpu::PowerPreference,
    /// A substring of the adapter's name: the first adapter that can present to the surface
    /// and whose name contains it ([`pick_adapter`]); `None`, or no such adapter, asks by
    /// `power_preference`.
    pub adapter: Option<String>,
    /// Pick an sRGB surface format, where the hardware encodes on write. `false` picks a
    /// non-sRGB format, for an app that writes display-encoded values itself;
    /// [`pick_surface_format`] orders the offered formats.
    pub prefer_srgb: bool,
    /// When the next frame starts; the default, [`Pacing::Unthrottled`], asks the compositor
    /// for no frame callback.
    pub pacing: Pacing,
    /// The instant the shell's `start-up:` lines (window, instance, surface, adapter, device,
    /// the first present) are timed from: the process's start when the app passes it, else
    /// [`run`]'s entry.
    pub start: Option<Instant>,
}

impl Default for ShellConfig {
    /// A 1280 x 720 window titled `vxe`, cursor free, presenting without vsync on the
    /// high-performance adapter, a non-sRGB surface and unthrottled frames.
    fn default() -> Self {
        Self {
            title: "vxe".to_owned(),
            size: [1280, 720],
            lock_cursor: false,
            present_mode: wgpu::PresentMode::AutoNoVsync,
            power_preference: wgpu::PowerPreference::HighPerformance,
            adapter: None,
            prefer_srgb: false,
            pacing: Pacing::Unthrottled,
            start: None,
        }
    }
}

/// When the shell starts the next frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pacing {
    /// Request the next frame as soon as the event loop is idle (`ControlFlow::Poll`), with
    /// no frame callback: the next frame does not wait for the compositor to show this one.
    #[default]
    Unthrottled,
    /// Request the next frame as soon as the event loop is idle, but ask the compositor for a
    /// frame callback before each present (`Window::pre_present_notify`). On Wayland winit
    /// delivers no redraw while that callback is outstanding, so the next frame starts only
    /// after the compositor showed this one.
    Compositor,
    /// Start a frame at most `hz` times a second: the loop sleeps
    /// (`ControlFlow::WaitUntil`) until `1 / hz` after the last frame started. `hz` 0 is
    /// treated as 1. Each present also asks for a frame callback, as [`Pacing::Compositor`].
    Cap { hz: u32 },
}

impl Pacing {
    /// When the frame after one started at `last_start` may start; `None` for at once.
    pub fn next_deadline(self, last_start: Instant) -> Option<Instant> {
        match self {
            Pacing::Unthrottled | Pacing::Compositor => None,
            Pacing::Cap { hz } => {
                Some(last_start + Duration::from_micros(1_000_000 / u64::from(hz.max(1))))
            }
        }
    }

    /// Whether the shell asks the compositor for a frame callback before each present.
    pub fn frame_callback(self) -> bool {
        match self {
            Pacing::Unthrottled => false,
            Pacing::Compositor | Pacing::Cap { .. } => true,
        }
    }

    /// The mode's word: `free`, `compositor` or `cap` (without its rate).
    pub fn word(self) -> &'static str {
        match self {
            Pacing::Unthrottled => "free",
            Pacing::Compositor => "compositor",
            Pacing::Cap { .. } => "cap",
        }
    }

    /// The mode a word names: `free` ([`Pacing::Unthrottled`]) or `compositor`.
    pub fn from_word(word: &str) -> Option<Self> {
        match word {
            "free" => Some(Pacing::Unthrottled),
            "compositor" => Some(Pacing::Compositor),
            _ => None,
        }
    }
}

/// The present mode `wgpu` configures for a `requested` one among the surface's `offered`
/// modes: the requested mode when offered, else for `AutoVsync` the first offered of
/// `FifoRelaxed` and `Fifo`, for `AutoNoVsync` of `Immediate`, `Mailbox` and `Fifo`
/// (wgpu-core's fallback rule); `None` when nothing fits.
fn granted_present_mode(
    requested: wgpu::PresentMode,
    offered: &[wgpu::PresentMode],
) -> Option<wgpu::PresentMode> {
    use wgpu::PresentMode as P;
    if offered.contains(&requested) {
        return Some(requested);
    }
    let fallbacks: &[P] = match requested {
        P::AutoVsync => &[P::FifoRelaxed, P::Fifo],
        P::AutoNoVsync => &[P::Immediate, P::Mailbox, P::Fifo],
        _ => &[],
    };
    fallbacks.iter().copied().find(|m| offered.contains(m))
}

#[derive(Debug)]
pub enum ShellError {
    EventLoop(winit::error::EventLoopError),
    /// Window, surface, adapter or device creation failed.
    Setup(String),
}

impl std::fmt::Display for ShellError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShellError::EventLoop(e) => write!(f, "event loop: {e}"),
            ShellError::Setup(e) => write!(f, "setup: {e}"),
        }
    }
}

impl std::error::Error for ShellError {}

/// Runs `app` in a window until it returns [`Flow::Exit`] or the window closes.
///
/// Teardown runs after the event loop returns, in a fixed order with each stage logged at
/// `info` (target `vxe_app::shell`), so a hang at exit names the stage it hangs in: the app,
/// the surface (its swapchain waits for the device to idle), the queue (waits for submitted
/// work and pending presents), the device and adapter, then the window. `run_app` consumes
/// the event loop, so it is gone before the first stage.
pub fn run<A: App>(config: ShellConfig, app: A) -> Result<(), ShellError> {
    let event_loop = EventLoop::new().map_err(ShellError::EventLoop)?;
    log::info!(
        "pacing: {} ({})",
        config.pacing.word(),
        if config.pacing.frame_callback() {
            "a frame callback before each present"
        } else {
            "no frame callback"
        }
    );
    event_loop.set_control_flow(match config.pacing {
        Pacing::Unthrottled | Pacing::Compositor => ControlFlow::Poll,
        // `about_to_wait` sets the deadline once a frame has started.
        Pacing::Cap { .. } => ControlFlow::Wait,
    });
    let started = config.start.unwrap_or_else(now);
    let mut shell = Shell {
        started,
        presented: false,
        config,
        app,
        state: None,
        input: Input::default(),
        last_frame: None,
        error: None,
    };
    let ran = event_loop.run_app(&mut shell);
    log::info!("teardown: event loop returned");
    let error = shell.teardown();
    ran.map_err(ShellError::EventLoop)?;
    error.map_or(Ok(()), Err)
}

/// Whether this build links DXC into wgpu's DX12 backend: the workspace builds `wgpu` with
/// `static-dxc` (ADR-0035), which `wgpu-hal` honours only on x86_64 and i686 Windows MSVC.
const STATIC_DXC: bool = cfg!(all(
    windows,
    target_env = "msvc",
    not(target_arch = "aarch64")
));

/// The backend for a startup line: its name, and on DX12 the shader compiler the build and the
/// environment request (`WGPU_DX12_COMPILER` when set, else static DXC where it is linked, else
/// `Dx12Compiler::Auto`'s fallback). It reports the request, not what `wgpu-hal` loaded.
pub fn backend_label(backend: wgpu::Backend) -> String {
    if backend != wgpu::Backend::Dx12 {
        return format!("{backend:?}");
    }
    let compiler = match std::env::var("WGPU_DX12_COMPILER") {
        Ok(value) => format!("WGPU_DX12_COMPILER={value}"),
        Err(_) if STATIC_DXC => "static DXC".to_owned(),
        Err(_) => "dxcompiler.dll if found, else FXC".to_owned(),
    };
    format!("{backend:?}, shader compiler: {compiler}")
}

/// What [`run_headless`] creates: the target's size and format, and the adapter it asks for.
#[derive(Clone, Debug)]
pub struct HeadlessConfig {
    /// The offscreen target's size in pixels, handed to the app as [`Gpu::size`].
    pub size: [u32; 2],
    /// The offscreen target's format, handed to the app as [`Gpu::surface_format`].
    pub format: wgpu::TextureFormat,
    /// Which adapter to ask for when `adapter` names none.
    pub power_preference: wgpu::PowerPreference,
    /// A substring of the adapter's name: the first adapter whose name contains it; `None`,
    /// or no such adapter, asks by `power_preference`.
    pub adapter: Option<String>,
}

impl Default for HeadlessConfig {
    /// A 1280 x 720 `Rgba8Unorm` target on the high-performance adapter.
    fn default() -> Self {
        Self {
            size: [1280, 720],
            format: wgpu::TextureFormat::Rgba8Unorm,
            power_preference: wgpu::PowerPreference::HighPerformance,
            adapter: None,
        }
    }
}

/// Runs `app` without a window until it returns [`Flow::Exit`]: a device on `config`'s
/// adapter and an offscreen target in place of the surface, the same calls as in a window
/// ([`App::device_request`], [`App::init`], then [`App::frame`] with an empty [`Input`] and
/// the measured `dt`), the device polled to idle after each frame as presenting would wait.
/// The instance reads `wgpu`'s environment variables (`WGPU_BACKEND`, `WGPU_DX12_COMPILER`,
/// ...) as [`run`]'s does.
///
/// Teardown follows [`run`]'s order, each stage logged at `info` alike: the app, then the
/// queue, the device and the adapter.
pub fn run_headless<A: App>(mut app: A, config: HeadlessConfig) -> Result<(), ShellError> {
    let desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
    // Names the flags `WGPU_VALIDATION`, `WGPU_GPU_BASED_VALIDATION` and the build left on, so
    // a debug-layer run shows the request reached wgpu.
    log::info!("headless: instance flags {:?}", desc.flags);
    let instance = wgpu::Instance::new(desc);
    let named = config.adapter.as_deref().and_then(|want| {
        let all = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()));
        let names: Vec<String> = all.iter().map(|a| a.get_info().name).collect();
        let found = pick_adapter(&names, want).map(|i| all[i].clone());
        if found.is_none() {
            log::warn!("adapter {want:?}: no adapter's name contains it; the default is used");
        }
        found
    });
    let adapter = match named {
        Some(a) => a,
        None => pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: config.power_preference,
            ..Default::default()
        }))
        .map_err(|e| ShellError::Setup(format!("no adapter: {e}")))?,
    };
    let info = adapter.get_info();
    log::info!(
        "headless: adapter {} ({}), driver {:?} {:?}, max storage binding {} B, max buffer {} B, timestamps {}",
        info.name,
        backend_label(info.backend),
        info.driver,
        info.driver_info,
        adapter.limits().max_storage_buffer_binding_size,
        adapter.limits().max_buffer_size,
        adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY)
    );
    let (required_features, required_limits) = app.device_request(&adapter);
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features,
        required_limits,
        ..Default::default()
    }))
    .map_err(|e| ShellError::Setup(format!("no device: {e}")))?;
    let gpu = Gpu {
        adapter,
        device,
        queue,
        surface_format: config.format,
        surface_color_space: None,
        size: config.size,
    };
    app.init(&gpu);
    let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("headless surface"),
        size: wgpu::Extent3d {
            width: config.size[0],
            height: config.size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: config.format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let mut input = Input::default();
    let mut last = now();
    loop {
        let t = now();
        let dt = (t - last).as_secs_f32();
        last = t;
        app.poll(&mut input);
        if app.frame(&gpu, &input, dt, &view) == Flow::Exit {
            log::info!("teardown: the app asked to exit");
            break;
        }
        // Keep the queue from running ahead of the GPU, as presenting would.
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    }
    drop(view);
    drop(target);
    stage("app", || drop(app));
    let Gpu {
        adapter,
        device,
        queue,
        ..
    } = gpu;
    stage("queue", || drop(queue));
    stage("device", || drop(device));
    stage("adapter", || drop(adapter));
    Ok(())
}

/// The surface format the window configures, from `offered` in the surface's order of
/// preference; `None` when nothing is offered. The present pass writes display-encoded values
/// (`fs_main`) to a non-sRGB format and decodes them first (`fs_srgb`) for an sRGB one, which
/// only a fixed-point format with the `Srgb` colour space shows as written. So the order is:
///
/// 1. `Bgra8Unorm` or `Rgba8Unorm` (their `Srgb` forms with `prefer_srgb`), the first offered;
/// 2. the other encoding's 8-bit pair, the first offered;
/// 3. any other format that is not floating point, encoding as preferred first (a 10-bit
///    `Rgb10a2Unorm` shows display-encoded values as an 8-bit one does);
/// 4. the first offered.
///
/// A floating-point format comes last because `wgpu` configures `Rgba16Float` with `Auto`
/// as scRGB, whose values are linear: display-encoded values written there would show too
/// bright (0.5 shows as 0.73 encoded).
pub fn pick_surface_format(
    offered: &[wgpu::TextureFormat],
    prefer_srgb: bool,
) -> Option<wgpu::TextureFormat> {
    use wgpu::TextureFormat as F;
    let eight_bit = |srgb: bool| {
        if srgb {
            [F::Bgra8UnormSrgb, F::Rgba8UnormSrgb]
        } else {
            [F::Bgra8Unorm, F::Rgba8Unorm]
        }
    };
    let fixed = |f: &F| !is_float_format(*f);
    let first = |want: &dyn Fn(&F) -> bool| offered.iter().copied().find(|f| want(f));
    first(&|f| eight_bit(prefer_srgb).contains(f))
        .or_else(|| first(&|f| eight_bit(!prefer_srgb).contains(f)))
        .or_else(|| first(&|f| fixed(f) && f.is_srgb() == prefer_srgb))
        .or_else(|| first(&|f| fixed(f)))
        .or_else(|| offered.first().copied())
}

/// Whether `f` stores floating-point channels (`Rgba16Float`, `Rg11b10Ufloat`, ...).
fn is_float_format(f: wgpu::TextureFormat) -> bool {
    use wgpu::TextureFormat as F;
    matches!(
        f,
        F::R16Float
            | F::Rg16Float
            | F::Rgba16Float
            | F::R32Float
            | F::Rg32Float
            | F::Rgba32Float
            | F::Rg11b10Ufloat
            | F::Rgb9e5Ufloat
    )
}

/// The colour space the window configures for a format whose surface offers `spaces`: `Srgb`
/// when offered, so a non-sRGB format's values are read as sRGB-encoded and an sRGB format's
/// as encoded on write; else `Auto`, the backend's choice.
pub fn surface_color_space(spaces: wgpu::SurfaceColorSpaces) -> wgpu::SurfaceColorSpace {
    if spaces.contains(wgpu::SurfaceColorSpaces::SRGB) {
        wgpu::SurfaceColorSpace::Srgb
    } else {
        wgpu::SurfaceColorSpace::Auto
    }
}

/// The index of the first of `names` that contains `want`, as `--adapter` picks in a window and
/// headless.
pub fn pick_adapter(names: &[String], want: &str) -> Option<usize> {
    names.iter().position(|n| n.contains(want))
}

/// Runs one teardown stage between two log lines, so the last line before a hang is the
/// stage that hangs.
fn stage(what: &str, f: impl FnOnce()) {
    log::info!("teardown: dropping the {what}");
    let t = now();
    f();
    log::info!(
        "teardown: {what} dropped in {:.1} ms",
        (now() - t).as_secs_f64() * 1e3
    );
}

struct State {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    gpu: Gpu,
}

struct Shell<A> {
    /// What the `start-up:` lines are timed from ([`ShellConfig::start`]).
    started: Instant,
    /// Whether a frame was presented yet: the first present logs its time.
    presented: bool,
    config: ShellConfig,
    app: A,
    state: Option<State>,
    input: Input,
    last_frame: Option<Instant>,
    error: Option<ShellError>,
}

#[allow(
    clippy::disallowed_methods,
    reason = "frame pacing: the shell measures the time between frames for the app's dt"
)]
fn now() -> Instant {
    Instant::now()
}

impl<A: App> Shell<A> {
    /// Logs `start-up: <what> at <ms> ms` at info, timed from [`ShellConfig::start`].
    fn mark(&self, what: &str) {
        log::info!(
            "start-up: {what} at {:.0} ms",
            (now() - self.started).as_secs_f64() * 1e3
        );
    }

    /// Drops the app, then the window state in the order [`run`] documents; returns the
    /// setup error, if any.
    fn teardown(self) -> Option<ShellError> {
        let Shell {
            app, state, error, ..
        } = self;
        stage("app", || drop(app));
        if let Some(State {
            window,
            surface,
            config: _,
            gpu,
        }) = state
        {
            let Gpu {
                adapter,
                device,
                queue,
                ..
            } = gpu;
            stage("surface", || drop(surface));
            stage("queue", || drop(queue));
            stage("device", || drop(device));
            stage("adapter", || drop(adapter));
            stage("window", || drop(window));
        }
        error
    }

    fn create(&mut self, event_loop: &ActiveEventLoop) -> Result<State, String> {
        let attrs = Window::default_attributes()
            .with_title(self.config.title.clone())
            .with_inner_size(winit::dpi::PhysicalSize::new(
                self.config.size[0],
                self.config.size[1],
            ));
        let window = Arc::new(event_loop.create_window(attrs).map_err(|e| e.to_string())?);
        self.mark("window created");
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(
                Box::new(event_loop.owned_display_handle()),
            ));
        self.mark("instance created");
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| e.to_string())?;
        self.mark("surface created");
        let named = self.config.adapter.as_deref().and_then(|want| {
            let all: Vec<wgpu::Adapter> =
                pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()))
                    .into_iter()
                    .filter(|a| a.is_surface_supported(&surface))
                    .collect();
            let names: Vec<String> = all.iter().map(|a| a.get_info().name).collect();
            let found = pick_adapter(&names, want);
            if found.is_none() {
                log::warn!(
                    "adapter {want:?}: no adapter that presents to the window has it in its name ({names:?}); asking by power preference"
                );
            }
            found.map(|i| all[i].clone())
        });
        let adapter = match named {
            Some(a) => a,
            None => pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: self.config.power_preference,
                compatible_surface: Some(&surface),
                ..Default::default()
            }))
            .map_err(|e| e.to_string())?,
        };
        self.mark("adapter ready");
        let (required_features, required_limits) = self.app.device_request(&adapter);
        self.mark("device requested");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features,
            required_limits,
            ..Default::default()
        }))
        .map_err(|e| e.to_string())?;
        self.mark("device created");

        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or("the surface does not support this adapter")?;
        let caps = surface.get_capabilities(&adapter);
        if let Some(f) = pick_surface_format(&caps.formats, self.config.prefer_srgb) {
            config.format = f;
        }
        config.color_space = surface_color_space(caps.color_spaces(config.format));
        config.present_mode = self.config.present_mode;
        surface.configure(&device, &config);
        self.mark("surface configured");
        log::info!(
            "surface: format {:?}, colour space {:?}, alpha {:?}; offered formats {:?}, colour spaces {:?}, alpha modes {:?}",
            config.format,
            config.color_space,
            config.alpha_mode,
            caps.formats,
            caps.format_capabilities
                .iter()
                .map(|c| (c.format, c.color_spaces))
                .collect::<Vec<_>>(),
            caps.alpha_modes
        );
        log::info!(
            "present mode: requested {:?}, granted {}, offered {:?}",
            self.config.present_mode,
            granted_present_mode(self.config.present_mode, &caps.present_modes)
                .map_or_else(|| "none".to_owned(), |m| format!("{m:?}")),
            caps.present_modes
        );

        if self.config.lock_cursor {
            let grabbed = window
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| window.set_cursor_grab(CursorGrabMode::Confined));
            if let Err(e) = grabbed {
                log::warn!("cursor grab refused: {e}");
            }
            window.set_cursor_visible(false);
        }

        let info = adapter.get_info();
        log::info!("adapter: {} ({})", info.name, backend_label(info.backend));
        let gpu = Gpu {
            adapter,
            device,
            queue,
            surface_format: config.format,
            surface_color_space: Some(config.color_space),
            size: [config.width, config.height],
        };
        Ok(State {
            window,
            surface,
            config,
            gpu,
        })
    }

    /// The app's [`App::splash`] frame, presented when the app drew one; skipped when the
    /// surface gives no texture.
    fn splash(&mut self, state: &State) {
        let texture = match state.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            _ => return,
        };
        let view = texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        if !self.app.splash(&state.gpu, &view) {
            return;
        }
        if self.config.pacing.frame_callback() {
            state.window.pre_present_notify();
        }
        state.gpu.queue.present(texture);
        self.presented = true;
        self.mark("first frame presented (the splash)");
    }

    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        let texture = match state.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return,
            _ => {
                state.surface.configure(&state.gpu.device, &state.config);
                return;
            }
        };
        let t = now();
        let dt = self.last_frame.map_or(0.0, |last| (t - last).as_secs_f32());
        self.last_frame = Some(t);
        let view = texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.app.poll(&mut self.input);
        let flow = self.app.frame(&state.gpu, &self.input, dt, &view);
        self.input.end_frame();
        if self.config.pacing.frame_callback() {
            state.window.pre_present_notify();
        }
        state.gpu.queue.present(texture);
        if !self.presented {
            self.presented = true;
            self.mark("first frame presented");
        }
        if flow == Flow::Exit {
            log::info!("teardown: the app asked to exit");
            event_loop.exit();
        }
    }
}

impl<A: App> ApplicationHandler for Shell<A> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        match self.create(event_loop) {
            Ok(state) => {
                self.splash(&state);
                self.app.init(&state.gpu);
                self.mark("app initialised");
                state.window.request_redraw();
                self.state = Some(state);
            }
            Err(e) => {
                self.error = Some(ShellError::Setup(e));
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                log::info!("teardown: close requested");
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let Some(state) = self.state.as_mut()
                    && size.width > 0
                    && size.height > 0
                {
                    state.config.width = size.width;
                    state.config.height = size.height;
                    state.surface.configure(&state.gpu.device, &state.config);
                    state.gpu.size = [size.width, size.height];
                    self.app.resized(&state.gpu);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    match event.state {
                        ElementState::Pressed => self.input.key_down(code),
                        ElementState::Released => self.input.key_up(code),
                    }
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button,
                ..
            } => self.input.button_down(button),
            WindowEvent::MouseWheel { delta, .. } => self.input.add_scroll(match delta {
                MouseScrollDelta::LineDelta(_, y) => y,
                MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
            }),
            WindowEvent::Focused(false) => self.input.release_all(),
            WindowEvent::RedrawRequested => self.redraw(event_loop),
            _ => {}
        }
    }

    fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event {
            self.input.add_mouse_delta(delta.0, delta.1);
        }
    }

    fn exiting(&mut self, _: &ActiveEventLoop) {
        log::info!("teardown: event loop exiting");
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(state) = &self.state else {
            return;
        };
        let deadline = self
            .last_frame
            .and_then(|last| self.config.pacing.next_deadline(last));
        match deadline {
            Some(at) if now() < at => event_loop.set_control_flow(ControlFlow::WaitUntil(at)),
            _ => state.window.request_redraw(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_config_presents_unthrottled_without_vsync() {
        let c = ShellConfig::default();
        assert_eq!(c.present_mode, wgpu::PresentMode::AutoNoVsync);
        assert_eq!(c.power_preference, wgpu::PowerPreference::HighPerformance);
        assert!(!c.prefer_srgb);
        assert_eq!(c.pacing, Pacing::Unthrottled);
    }

    #[test]
    fn a_60_hz_cap_waits_16_666_us_after_the_last_frame_start() {
        let start = now();
        let next = Pacing::Cap { hz: 60 }.next_deadline(start);
        assert_eq!(next, Some(start + Duration::from_micros(16_666)));
        assert_eq!(Pacing::Unthrottled.next_deadline(start), None);
        assert_eq!(Pacing::Compositor.next_deadline(start), None);
    }

    /// `free` requests no frame callback; `compositor`, the shell's earlier behaviour, does,
    /// and so does a cap. The words round-trip; anything else is refused.
    #[test]
    fn free_pacing_requests_no_frame_callback() {
        assert!(!Pacing::Unthrottled.frame_callback());
        assert!(Pacing::Compositor.frame_callback());
        assert!(Pacing::Cap { hz: 60 }.frame_callback());
        assert_eq!(Pacing::from_word("free"), Some(Pacing::Unthrottled));
        assert_eq!(Pacing::from_word("compositor"), Some(Pacing::Compositor));
        assert_eq!(Pacing::from_word("vsync"), None);
        assert_eq!(Pacing::from_word("Free"), None);
        for p in [Pacing::Unthrottled, Pacing::Compositor] {
            assert_eq!(Pacing::from_word(p.word()), Some(p));
        }
        assert_eq!(Pacing::Cap { hz: 60 }.word(), "cap");
    }

    /// The granted mode follows wgpu-core's fallback: no vsync prefers `Immediate`, then
    /// `Mailbox`, then `Fifo`; vsync prefers `FifoRelaxed`, then `Fifo`; a concrete mode is
    /// granted only when offered.
    #[test]
    fn the_granted_present_mode_follows_wgpu_s_fallback() {
        use wgpu::PresentMode as P;
        let all = [P::Fifo, P::FifoRelaxed, P::Immediate, P::Mailbox];
        assert_eq!(
            granted_present_mode(P::AutoNoVsync, &all),
            Some(P::Immediate)
        );
        assert_eq!(
            granted_present_mode(P::AutoNoVsync, &[P::Fifo, P::Mailbox]),
            Some(P::Mailbox)
        );
        assert_eq!(
            granted_present_mode(P::AutoNoVsync, &[P::Fifo]),
            Some(P::Fifo)
        );
        assert_eq!(
            granted_present_mode(P::AutoVsync, &all),
            Some(P::FifoRelaxed)
        );
        assert_eq!(
            granted_present_mode(P::AutoVsync, &[P::Fifo]),
            Some(P::Fifo)
        );
        assert_eq!(granted_present_mode(P::Mailbox, &[P::Fifo]), None);
        assert_eq!(granted_present_mode(P::Fifo, &[P::Fifo]), Some(P::Fifo));
    }

    /// An 8-bit UNORM format of the preferred encoding wins wherever it is offered, ahead of
    /// a 10-bit or a half-float format listed first; without one, the other encoding's 8-bit
    /// pair, then a fixed-point format, and a float format only when nothing else is offered.
    #[test]
    fn the_surface_format_prefers_8_bit_unorm_of_the_preferred_encoding() {
        use wgpu::TextureFormat as F;
        let windows_like = [
            F::Rgba16Float,
            F::Rgb10a2Unorm,
            F::Bgra8UnormSrgb,
            F::Bgra8Unorm,
        ];
        assert_eq!(
            pick_surface_format(&windows_like, false),
            Some(F::Bgra8Unorm)
        );
        assert_eq!(
            pick_surface_format(&windows_like, true),
            Some(F::Bgra8UnormSrgb)
        );
        let linux_like = [F::Bgra8UnormSrgb, F::Bgra8Unorm, F::Rgba8Unorm];
        assert_eq!(pick_surface_format(&linux_like, false), Some(F::Bgra8Unorm));
        assert_eq!(
            pick_surface_format(&[F::Rgba8Unorm], false),
            Some(F::Rgba8Unorm)
        );
        assert_eq!(
            pick_surface_format(&[F::Rgba16Float, F::Rgba8UnormSrgb], false),
            Some(F::Rgba8UnormSrgb)
        );
        assert_eq!(
            pick_surface_format(&[F::Rgba16Float, F::Rgb10a2Unorm], false),
            Some(F::Rgb10a2Unorm)
        );
        assert_eq!(
            pick_surface_format(&[F::Rgba16Float, F::Rgb10a2Unorm], true),
            Some(F::Rgb10a2Unorm)
        );
        assert_eq!(
            pick_surface_format(&[F::Rgba16Float], false),
            Some(F::Rgba16Float)
        );
        assert_eq!(pick_surface_format(&[], false), None);
    }

    /// `Srgb` is asked for whenever the format offers it; otherwise the backend picks.
    #[test]
    fn the_colour_space_is_srgb_when_offered() {
        use wgpu::SurfaceColorSpaces as S;
        assert_eq!(
            surface_color_space(S::SRGB | S::EXTENDED_SRGB_LINEAR),
            wgpu::SurfaceColorSpace::Srgb
        );
        assert_eq!(
            surface_color_space(S::EXTENDED_SRGB_LINEAR),
            wgpu::SurfaceColorSpace::Auto
        );
        assert_eq!(
            surface_color_space(S::empty()),
            wgpu::SurfaceColorSpace::Auto
        );
    }
}
