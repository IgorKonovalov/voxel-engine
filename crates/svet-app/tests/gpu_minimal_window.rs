//! The minimal window example without a window: its app, as the example's `main` builds it, run
//! by `run_headless` on the software adapter (lavapipe), drawing into a target the test reads.

use std::sync::{Arc, Mutex};

use svet_app::{App, Flow, Gpu, HeadlessConfig, Input, wgpu};

#[allow(
    dead_code,
    reason = "the example's `main` opens a window; the test runs its app headless"
)]
mod example {
    include!("../examples/minimal_window.rs");

    /// The app the example's `main` runs.
    pub(super) fn app() -> impl svet_app::App {
        Demo(None)
    }
}

/// Frames the test runs before it reads the image.
const FRAMES: u32 = 3;

/// The example's app drawing each frame into a texture of the test's own, which it reads back
/// after [`FRAMES`] frames: what the window would show.
struct Probe<A> {
    app: A,
    frames: u32,
    target: Option<wgpu::Texture>,
    image: Arc<Mutex<Option<Vec<u8>>>>,
}

impl<A: App> App for Probe<A> {
    fn device_request(&mut self, adapter: &wgpu::Adapter) -> (wgpu::Features, wgpu::Limits) {
        self.app.device_request(adapter)
    }

    fn init(&mut self, gpu: &Gpu) {
        self.app.init(gpu);
        self.target = Some(gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("probe target"),
            size: wgpu::Extent3d {
                width: gpu.size[0],
                height: gpu.size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: gpu.surface_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        }));
    }

    fn frame(&mut self, gpu: &Gpu, input: &Input, dt: f32, _: &wgpu::TextureView) -> Flow {
        let target = self.target.as_ref().expect("init ran");
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        assert_eq!(self.app.frame(gpu, input, dt, &view), Flow::Continue);
        self.frames += 1;
        if self.frames < FRAMES {
            return Flow::Continue;
        }
        let pixels = svet_render::debug::read_rgba8(&gpu.device, &gpu.queue, target);
        *self.image.lock().expect("one writer") = Some(pixels);
        Flow::Exit
    }
}

/// Three frames of the example at 320 x 180 on lavapipe: the presented image holds more than
/// one colour (the sky, the lit stone and its shade), so the world, its light and the blit
/// reached the target.
#[test]
fn the_example_draws_a_frame_of_more_than_one_colour() {
    let Some(adapter) = svet_render::test_support::software_adapter() else {
        return;
    };
    let image = Arc::new(Mutex::new(None));
    let probe = Probe {
        app: example::app(),
        frames: 0,
        target: None,
        image: Arc::clone(&image),
    };
    let config = HeadlessConfig {
        size: [320, 180],
        adapter: Some(adapter.get_info().name),
        ..HeadlessConfig::default()
    };
    svet_app::run_headless(probe, config).expect("the headless run");
    let pixels = image.lock().expect("one writer").take().expect("an image");
    assert_eq!(pixels.len(), 320 * 180 * 4);
    let mut colours: Vec<[u8; 3]> = pixels.chunks(4).map(|p| [p[0], p[1], p[2]]).collect();
    let mean = colours
        .iter()
        .map(|c| c.iter().map(|&v| f64::from(v)).sum::<f64>() / 3.0)
        .sum::<f64>()
        / colours.len() as f64;
    colours.sort_unstable();
    colours.dedup();
    println!(
        "{} distinct colours, mean level {mean:.1} of 255",
        colours.len()
    );
    assert!(colours.len() > 1, "one colour: {:?}", colours[0]);
}
