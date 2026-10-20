//! The window and frame loop: a `winit` 0.30 `ApplicationHandler` that opens one window,
//! creates the `wgpu` device and surface on first resume, and hands each frame to an [`App`].

use std::sync::Arc;
use std::time::Instant;

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
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut shell = Shell {
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
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(
                Box::new(event_loop.owned_display_handle()),
            ));
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| e.to_string())?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .map_err(|e| e.to_string())?;
        let (required_features, required_limits) = self.app.device_request(&adapter);
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features,
            required_limits,
            ..Default::default()
        }))
        .map_err(|e| e.to_string())?;

        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or("the surface does not support this adapter")?;
        // Prefer a non-sRGB format: the app writes display-encoded values.
        let caps = surface.get_capabilities(&adapter);
        if let Some(&f) = caps.formats.iter().find(|f| !f.is_srgb()) {
            config.format = f;
        }
        config.present_mode = wgpu::PresentMode::AutoNoVsync;
        surface.configure(&device, &config);

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
        log::info!("adapter: {} ({:?})", info.name, info.backend);
        let gpu = Gpu {
            adapter,
            device,
            queue,
            surface_format: config.format,
            size: [config.width, config.height],
        };
        Ok(State {
            window,
            surface,
            config,
            gpu,
        })
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
        state.window.pre_present_notify();
        state.gpu.queue.present(texture);
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
                self.app.init(&state.gpu);
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

    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if let Some(state) = &self.state {
            state.window.request_redraw();
        }
    }
}
