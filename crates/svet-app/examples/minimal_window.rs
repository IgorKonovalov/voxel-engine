// A window on a small voxel world: a stone floor with a wall, an arch and a crater carved into
// it, ray-marched under a sunlit sky with the settings of the renderer's default profile.
// Escape quits.
//
//     cargo run --release -p svet-app --example minimal_window
//
// The span between the `readme:` markers is quoted in the README's `Use it as a library`
// section; `tests/readme_example.rs` holds the two byte-equal, and
// `tests/gpu_minimal_window.rs` includes this file and runs it without a window.

use svet_app::{App, Flow, Gpu, Input, KeyCode, ShellConfig, wgpu};
use svet_render::{BakeSettings, Camera, Frame, GpuWorld, LightParams, Presenter, glam::Vec3};
use svet_render::{Profile, RenderSettings, sky::Body, sky::Sky};
use svet_world::{AIR, ChunkExtent, EditOp, FIXED_ONE, Fixed3, FnSource, Material, Mode, World};

/// The eye, in voxels: over the floor's corner, looking at the arch.
const EYE: Camera = Camera {
    position: Vec3::new(10.0, 8.0, 30.0),
    yaw: 0.78,
    pitch: -0.2,
    vfov: 1.0,
};
/// Toward the sun, high in the south-east.
const SUN: [f32; 3] = [0.48, -0.29, 0.83];

/// A stone floor 8 voxels deep over 128 x 128 voxels with a wall on it, then three carves: a
/// box and a sphere make the arch, a sphere the crater.
fn world() -> World {
    let table = [Material::named("stone")].into_iter().collect();
    let wall = |v: [i32; 3]| (40..88).contains(&v[0]) && (60..68).contains(&v[1]) && v[2] < 40;
    let solid = FnSource(move |v: [i32; 3]| if v[2] < 8 || wall(v) { 1 } else { AIR });
    let world = World::generate(ChunkExtent::new([0, 0, 0], [2, 2, 1]), &solid, &table);
    let at = |v: [i64; 3]| Fixed3(v.map(|c| c * FIXED_ONE));
    for op in [
        EditOp::box_(at([54, 56, 8]), at([74, 72, 26]), Mode::Carve),
        EditOp::sphere(at([64, 64, 26]), 10 * FIXED_ONE, Mode::Carve),
        EditOp::sphere(at([36, 30, 8]), 8 * FIXED_ONE, Mode::Carve),
    ] {
        world.apply(&op).expect("the shape lies in resident chunks");
    }
    world
}

/// The mirror of the world on the GPU, the frame that ray-marches it and the blit to the window.
struct Demo(Option<(GpuWorld, Frame, Presenter)>);

// readme:begin
impl App for Demo {
    fn device_request(&mut self, adapter: &wgpu::Adapter) -> (wgpu::Features, wgpu::Limits) {
        let limits = svet_render::limits_for(adapter, 16 << 20).expect("a capable adapter");
        (wgpu::Features::empty(), limits)
    }
    fn init(&mut self, gpu: &Gpu) {
        let (dev, queue, world) = (&gpu.device, &gpu.queue, &world());
        let mut mirror = GpuWorld::new(dev, world.extent());
        let mut enc = dev.create_command_encoder(&Default::default());
        mirror.sync(world, queue, &mut enc).expect("the world fits");
        queue.submit([enc.finish()]);
        let mut frame = Frame::new(dev, gpu.size[0], gpu.size[1]);
        frame.apply_settings(dev, queue, &RenderSettings::profile(Profile::Floor));
        frame.set_sky(Some(Sky::new(Body::sun(SUN), Body::moon([0.0, 0.0, -1.0]))));
        let (all, bake) = (world.extent(), BakeSettings::default());
        let baked = frame.bake_until(dev, queue, &mut mirror, world, all, &bake, &mut |_| {});
        baked.expect("the light store holds the world");
        self.0 = Some((mirror, frame, Presenter::new(dev, gpu.surface_format)));
    }
    fn frame(&mut self, gpu: &Gpu, input: &Input, dt: f32, out: &wgpu::TextureView) -> Flow {
        let (mirror, frame, presenter) = self.0.as_mut().expect("init ran");
        let (lamp, quit) = (LightParams::headlamp(), input.pressed(KeyCode::Escape));
        frame.set_dt(dt);
        frame.step(&gpu.device, &gpu.queue, mirror, &EYE, &lamp);
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        presenter.encode(&gpu.device, &mut enc, frame.target(), out);
        gpu.queue.submit([enc.finish()]);
        if quit { Flow::Exit } else { Flow::Continue }
    }
}
// readme:end

fn main() -> Result<(), svet_app::ShellError> {
    svet_app::run(ShellConfig::default(), Demo(None))
}
