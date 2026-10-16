//! Renders one still of the bounded level under the headlamp, for the README.
//!
//! `cargo run --release -p flythrough --example hero -- [--seed N] [--size WxH] [--yaw DEG]
//! [--pitch DEG] [--at X,Y,Z] [--carve R] [--out PATH]`. `--at` is a voxel coordinate (the
//! level's spawn by default); `--carve R` digs a sphere of radius `R` voxels where the view ray
//! meets rock before the frame is drawn. Runs on the high-performance adapter with no window.

use flythrough::cave::{CaveSource, LEVEL_CHUNKS};
use flythrough::cave_materials;
use vxe_render::{Camera, GpuWorld, LightParams, Marcher, readback, wgpu};
use vxe_world::*;

struct Args {
    seed: u64,
    size: [u32; 2],
    yaw: f32,
    pitch: f32,
    at: Option<[i32; 3]>,
    carve: i64,
    out: String,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        seed: 1,
        size: [1600, 900],
        yaw: 0.0,
        pitch: 0.0,
        at: None,
        carve: 0,
        out: "docs/images/hero.png".to_owned(),
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let v = it.next().ok_or(format!("{flag} needs a value"))?;
        let bad = |e: &dyn std::fmt::Display| format!("{flag}: {e}");
        match flag.as_str() {
            "--seed" => args.seed = v.parse().map_err(|e| bad(&e))?,
            "--size" => {
                let (w, h) = v.split_once('x').ok_or("--size takes WxH")?;
                args.size = [
                    w.parse().map_err(|e| bad(&e))?,
                    h.parse().map_err(|e| bad(&e))?,
                ];
            }
            "--yaw" => args.yaw = v.parse().map_err(|e| bad(&e))?,
            "--pitch" => args.pitch = v.parse().map_err(|e| bad(&e))?,
            "--at" => {
                let p: Vec<i32> = v
                    .split(',')
                    .map(str::parse)
                    .collect::<Result<_, _>>()
                    .map_err(|e| bad(&e))?;
                args.at = Some(p.try_into().map_err(|_| "--at takes X,Y,Z")?);
            }
            "--carve" => args.carve = v.parse().map_err(|e| bad(&e))?,
            "--out" => args.out = v,
            _ => return Err(format!("unknown argument {flag}")),
        }
    }
    Ok(args)
}

fn main() -> Result<(), String> {
    let args = parse_args()?;
    let source = CaveSource::new(args.seed, ChunkExtent::new([0, 0, 0], LEVEL_CHUNKS));
    let world = World::generate(source.extent, &source, &cave_materials());
    let eye = VoxelCoord(args.at.unwrap_or(source.spawn().0));
    let camera = Camera {
        position: eye.0.map(|v| v as f32 + 0.5).into(),
        yaw: args.yaw.to_radians(),
        pitch: args.pitch.to_radians(),
        vfov: 70f32.to_radians(),
    };

    if args.carve > 0 {
        let (dir, _, _) = camera.basis();
        let dir = Fixed3([dir.x, dir.y, dir.z].map(|c| (c * FIXED_ONE as f32).round() as i64));
        let o = centre_of_voxel(eye);
        let hit = world
            .raycast(o, dir.0, 400 * FIXED_ONE)
            .ok_or("the view ray meets no rock within 40 m")?;
        let t = hit.t + args.carve * FIXED_ONE / 2;
        let centre = Fixed3([0, 1, 2].map(|k| o.0[k] + dir.0[k] * t / FIXED_ONE));
        let r = world
            .apply(&EditOp::sphere(centre, args.carve * FIXED_ONE, Mode::Carve))
            .map_err(|e| format!("carve: {e:?}"))?;
        let removed: u64 = r.changed_from.iter().map(|&(_, n)| u64::from(n)).sum();
        println!("carved {removed} voxels");
    }

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .map_err(|e| format!("no adapter: {e}"))?;
    println!("rendering on {}", adapter.get_info().name);
    let pool_bytes = (world.stats().full_bricks as u64 + 4096) * 512;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: vxe_render::limits_for(&adapter, pool_bytes),
        ..Default::default()
    }))
    .map_err(|e| format!("no device: {e}"))?;

    let mut gpu = GpuWorld::new(&device, world.extent());
    gpu.sync(&world, &queue)
        .map_err(|e| format!("mirror: {e:?}"))?;
    let [w, h] = args.size;
    let marcher = Marcher::new(&device, w, h);
    marcher.render(&device, &queue, &gpu, &camera, &LightParams::headlamp());
    let rgba = readback::read_rgba8(&device, &queue, marcher.target());

    let file = std::fs::File::create(&args.out).map_err(|e| format!("{}: {e}", args.out))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .and_then(|mut wr| wr.write_image_data(&rgba))
        .map_err(|e| format!("png: {e}"))?;
    println!("wrote {}", args.out);
    Ok(())
}
