//! Shared fixtures for the GPU tests: the software adapter, the golden scene, PNG files and
//! image comparisons.
#![allow(dead_code, reason = "shared across test binaries; each uses a subset")]

use std::path::PathBuf;

use flythrough::cave::CaveSource;
use flythrough::cave_materials;
use vxe_render::{Camera, wgpu};
use vxe_world::*;

pub const WIDTH: u32 = 320;
pub const HEIGHT: u32 = 180;

/// A device on the software Vulkan adapter (lavapipe), or `None` with a printed skip when the
/// machine has none. Goldens are blessed on lavapipe, so no other adapter stands in for it.
pub fn software_device(pool_bytes: u64) -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        force_fallback_adapter: true,
        ..Default::default()
    }));
    let adapter = match adapter {
        Ok(a) if a.get_info().device_type == wgpu::DeviceType::Cpu => a,
        _ => {
            println!("skipped: no software adapter");
            return None;
        }
    };
    let info = adapter.get_info();
    let offered = adapter.limits();
    println!(
        "adapter: {} ({:?}, driver {} {}); max_storage_buffer_binding_size {}, max_buffer_size {}",
        info.name,
        info.backend,
        info.driver,
        info.driver_info,
        offered.max_storage_buffer_binding_size,
        offered.max_buffer_size
    );
    let desc = wgpu::DeviceDescriptor {
        required_limits: vxe_render::limits_for(&adapter, pool_bytes),
        ..Default::default()
    };
    Some(pollster::block_on(adapter.request_device(&desc)).expect("device on the software adapter"))
}

/// The golden scene: cave seed 7 over chunks `[0, 6)^3`, spawn at its centre.
pub fn scene_source() -> CaveSource {
    let extent = ChunkExtent::new([0, 0, 0], [6, 6, 6]);
    CaveSource::new(7, extent)
}

pub fn scene_world() -> World {
    let source = scene_source();
    World::generate(source.extent, &source, &cave_materials())
}

/// The carve the golden shows: radius 10 voxels, centred where the camera's view ray (+x from
/// the spawn voxel's centre) enters the first rock.
pub fn scene_carve(world: &World) -> EditOp {
    let o = centre_of_voxel(scene_source().spawn());
    let hit = world
        .raycast(o, [FIXED_ONE, 0, 0], 400 * FIXED_ONE)
        .expect("the view ray meets rock inside the shell");
    EditOp::sphere(
        Fixed3([o.0[0] + hit.t, o.0[1], o.0[2]]),
        10 * FIXED_ONE,
        Mode::Carve,
    )
}

/// The fixed spawn pose: at the spawn voxel's centre, looking along +x.
pub fn scene_camera() -> Camera {
    let s = scene_source().spawn().0.map(|v| v as f32 + 0.5);
    Camera {
        position: s.into(),
        yaw: 0.0,
        pitch: 0.0,
        vfov: 70f32.to_radians(),
    }
}

pub fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name)
}

pub fn write_png(path: &PathBuf, rgba: &[u8], w: u32, h: u32) {
    let file = std::fs::File::create(path).expect("create golden");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .expect("png header")
        .write_image_data(rgba)
        .expect("png data");
}

pub fn read_png(path: &PathBuf) -> (Vec<u8>, u32, u32) {
    let file = std::fs::File::open(path).unwrap_or_else(|e| panic!("open {path:?}: {e}"));
    let mut reader = png::Decoder::new(std::io::BufReader::new(file))
        .read_info()
        .expect("png info");
    let mut buf = vec![0; reader.output_buffer_size().expect("png size")];
    let info = reader.next_frame(&mut buf).expect("png frame");
    assert_eq!(info.color_type, png::ColorType::Rgba);
    buf.truncate(info.buffer_size());
    (buf, info.width, info.height)
}

/// Fraction of pixels whose largest channel difference exceeds `tolerance`.
pub fn fraction_differing(a: &[u8], b: &[u8], tolerance: u8) -> f64 {
    assert_eq!(a.len(), b.len());
    let n = a.len() / 4;
    let differing = a
        .chunks(4)
        .zip(b.chunks(4))
        .filter(|(p, q)| (0..3).any(|k| p[k].abs_diff(q[k]) > tolerance))
        .count();
    differing as f64 / n as f64
}

/// Colour bins (top 3 bits of r, g and b) covering at least `share` of the pixels.
pub fn bins_covering(rgba: &[u8], share: f64) -> usize {
    let mut bins = [0usize; 512];
    for p in rgba.chunks(4) {
        bins[usize::from(p[0] >> 5) | usize::from(p[1] >> 5) << 3 | usize::from(p[2] >> 5) << 6] +=
            1;
    }
    let n = (rgba.len() / 4) as f64;
    bins.iter().filter(|&&c| c as f64 >= share * n).count()
}

/// Compares `rgba` with the golden `name`, or rewrites the golden when `VXE_BLESS=1`. At most
/// 0.1 % of pixels may differ by more than 2/255 in any channel.
pub fn assert_golden(name: &str, rgba: &[u8]) {
    let path = golden_path(name);
    if std::env::var("VXE_BLESS").as_deref() == Ok("1") {
        write_png(&path, rgba, WIDTH, HEIGHT);
        println!("blessed {path:?}");
        return;
    }
    let (golden, w, h) = read_png(&path);
    assert_eq!((w, h), (WIDTH, HEIGHT));
    let f = fraction_differing(rgba, &golden, 2);
    println!(
        "{name}: {:.4} % of pixels differ by more than 2/255",
        f * 100.0
    );
    assert!(f <= 0.001, "{name}: {:.4} % of pixels differ", f * 100.0);
}
