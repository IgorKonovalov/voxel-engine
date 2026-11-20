//! What the headless stills and clips share (`hero`, `clip`): the world, the device, the
//! mirror synced until its light samples settle, the frame at the fly-through's upscaler and
//! look, and the PNG writer.

use std::path::Path;

use flythrough::cave::{CaveSource, LEVEL_CHUNKS};
use flythrough::outdoor::{OutdoorSource, Spawn};
use flythrough::showcase::ShowcaseSource;
use vxe_render::sky::Sky;
use vxe_render::{
    Camera, Frame, GpuWorld, Lighting, LightingResolution, MirrorConfig, RenderScale, wgpu,
};
use vxe_world::*;

/// The world a still or clip frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Level,
    Showcase,
    /// The outdoor hills, bounded to [`outdoor_extent`] around the mouth.
    #[allow(dead_code, reason = "the clip frames the showcase and the level only")]
    Outdoor,
}

/// The chunks a still of the outdoor hills generates: from 7 chunks (448 voxels) toward -x of
/// the mouth to 12 past it (the chamber lies 600 voxels along +x), 4 to either side in y, and
/// z 256 to 831, which holds the hills' surface (about 200 voxels either side of 512) and the
/// chamber 150 under it.
pub fn outdoor_extent() -> ChunkExtent {
    ChunkExtent::new([-7, -4, 4], [12, 4, 13])
}

/// The light radius of the showcase: its farthest pair of chunks lies 18.06 chunks apart.
pub const SHOWCASE_LIGHT_RADIUS: i32 = 19;

/// The world `source` generates from `seed`, its spawn, and the source's light radius. `rows`
/// maps the source's table before generation (the clip appends the walker's rows).
pub fn build_world(
    source: Source,
    seed: u64,
    rows: impl Fn(MaterialTable) -> MaterialTable,
) -> (World, VoxelCoord, i32) {
    match source {
        Source::Showcase => {
            let source = ShowcaseSource::new(seed);
            let world = World::generate(
                source.extent(),
                &source,
                &rows(flythrough::showcase_materials()),
            );
            (world, source.spawn(), SHOWCASE_LIGHT_RADIUS)
        }
        Source::Outdoor => {
            let source = OutdoorSource::new(seed);
            let world = World::generate(
                outdoor_extent(),
                &source,
                &rows(flythrough::outdoor_materials()),
            );
            let (eye, _) = source.spawn(Spawn::Outside);
            (world, eye, vxe_render::DEFAULT_LIGHT_RADIUS)
        }
        Source::Level => {
            let source = CaveSource::new(seed, ChunkExtent::new([0, 0, 0], LEVEL_CHUNKS));
            let world = World::generate(
                source.extent,
                &source,
                &rows(flythrough::with_emissive_ore(&flythrough::cave_materials())),
            );
            (world, source.spawn(), vxe_render::DEFAULT_LIGHT_RADIUS)
        }
    }
}

/// A camera at the centre of voxel `eye`, turned `yaw_deg` and `pitch_deg`, 70 degrees high.
pub fn camera_at(eye: [i32; 3], yaw_deg: f32, pitch_deg: f32) -> Camera {
    Camera {
        position: eye.map(|v| v as f32 + 0.5).into(),
        yaw: yaw_deg.to_radians(),
        pitch: pitch_deg.to_radians(),
        vfov: 70f32.to_radians(),
    }
}

/// The mean sRGB luma of an RGBA8 image, Rec. 709 weights on the encoded values, over 255.
pub fn mean_luma(rgba: &[u8]) -> f64 {
    let sum: f64 = rgba
        .chunks(4)
        .map(|p| 0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2]))
        .sum();
    sum / (rgba.len() / 4) as f64
}

pub fn write_png(path: &Path, rgba: &[u8], [w, h]: [u32; 2]) -> Result<(), String> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .and_then(|mut wr| wr.write_image_data(rgba))
        .map_err(|e| format!("png: {e}"))
}

/// The adapter whose name contains `adapter` (or the high-performance default), and a device
/// sized for `world`'s pool (`field`: the field look's band slots).
pub fn open_device(
    adapter: Option<&str>,
    field: bool,
    world: &World,
) -> Result<(wgpu::Device, wgpu::Queue, String), String> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let named = adapter.and_then(|want| {
        let all = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()));
        let found = all.into_iter().find(|a| a.get_info().name.contains(want));
        if found.is_none() {
            eprintln!("--adapter {want:?}: no adapter's name contains it; the default is used");
        }
        found
    });
    let adapter = match named {
        Some(a) => a,
        None => pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .map_err(|e| format!("no adapter: {e}"))?,
    };
    let adapter_name = adapter.get_info().name;
    println!("rendering on {adapter_name}");
    let pool_bytes = flythrough::level_pool_units(world, field) * vxe_render::internals::UNIT_BYTES;
    let (device, queue) =
        pollster::block_on(
            adapter.request_device(&wgpu::DeviceDescriptor {
                required_limits: vxe_render::limits_for(&adapter, pool_bytes)
                    .map_err(|e| format!("no device: {e}"))?,
                ..Default::default()
            }),
        )
        .map_err(|e| format!("no device: {e}"))?;
    Ok((device, queue, adapter_name))
}

/// The mirror over the whole world, focused on `eye` and synced until its light samples stop
/// changing.
pub fn fill_mirror(
    world: &World,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    field: bool,
    light_radius: i32,
    eye: &Camera,
) -> Result<GpuWorld, String> {
    // The bounded path's mirror budget, with the field's band slots under the field look.
    let chunks = world.extent().len() as u64;
    let cfg = MirrorConfig {
        budget_bytes: flythrough::level_mirror_budget(world, chunks, field),
        light_radius,
        ..MirrorConfig::new(vxe_render::Window::Fixed(world.extent()))
    };
    let mut gpu = if field {
        GpuWorld::with_field_surface(device, cfg)
    } else {
        GpuWorld::with_config(device, cfg)
    };
    // The light ranges around the eye take several syncs of the sample budget: sync until the
    // count stops changing.
    let focus = VoxelCoord(eye.position.to_array().map(|v| v.floor() as i32));
    gpu.set_focus(focus);
    let mut syncs = 0;
    let mut last = None;
    while syncs < 256 {
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        gpu.sync(world, queue, &mut enc)
            .map_err(|e| format!("mirror: {e:?}"))?;
        gpu.update_field(&mut enc);
        queue.submit([enc.finish()]);
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| format!("poll: {e}"))?;
        syncs += 1;
        let n = gpu.light_samples();
        if last == Some(n) {
            break;
        }
        last = Some(n);
    }
    println!(
        "mirror: {} light samples after {syncs} syncs",
        gpu.light_samples()
    );
    Ok(gpu)
}

/// How the frame is set up: its sizes and upscaler, its lighting and look.
pub struct FrameSetup {
    pub size: [u32; 2],
    pub internal_scale: RenderScale,
    /// `--jitter on|off` as given (our upscaler); unset, FSR 2 above scale 1.
    pub jitter: Option<bool>,
    pub sharpen: f32,
    pub probe_spacing: u32,
    pub lighting: Lighting,
    pub look: flythrough::LookPreset,
    pub resolution: LightingResolution,
    /// The sky over the world; `None`, the sky-less frame.
    pub sky: Option<Sky>,
}

impl FrameSetup {
    /// Whether the frame runs FSR 2: above scale 1 without `--jitter`, as the fly-through.
    pub fn fsr2(&self) -> bool {
        self.jitter.is_none() && self.internal_scale != RenderScale::One
    }

    /// A frame set up so, with the whole world baked at the live queue's settings (ADR-0046).
    pub fn frame(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gpu: &mut GpuWorld,
        world: &World,
    ) -> Result<Frame, String> {
        let mut frame =
            Frame::with_sizes(device, self.internal_scale.internal(self.size), self.size);
        frame.set_probe_spacing(device, self.probe_spacing);
        if self.fsr2() {
            frame.set_upscaler_kind(
                device,
                vxe_render::UpscalerKind::Fsr2(vxe_render::Fsr2Settings {
                    sharpness: self.sharpen,
                    ..vxe_render::Fsr2Settings::default()
                }),
            );
        } else {
            frame.set_jitter(device, self.jitter.unwrap_or(false));
            frame.set_sharpness(queue, self.sharpen);
        }
        frame.set_lighting(self.lighting);
        frame.set_look(vxe_render::LookParams {
            normal_mode: self.look.normal_mode,
            tuning: self.look.tuning,
            far_normal: self.look.far_normal,
            ..vxe_render::LookParams::default()
        });
        frame.set_resolution(device, self.resolution);
        frame.set_sky(self.sky);
        let r = frame
            .bake_until(
                device,
                queue,
                gpu,
                world,
                world.extent(),
                &vxe_render::queue_settings(),
                &mut |_| {},
            )
            .map_err(|e| format!("bake: {e:?}"))?;
        println!("baked {} chunks, {} faces", r.chunks, r.faces);
        Ok(frame)
    }
}
