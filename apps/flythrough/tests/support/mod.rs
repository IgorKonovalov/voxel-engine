//! Shared fixtures for the GPU tests: the software adapter, the golden scene, PNG files and
//! image comparisons.
#![allow(dead_code, reason = "shared across test binaries; each uses a subset")]

use std::path::PathBuf;

use flythrough::cave::CaveSource;
use flythrough::cave_materials;
use vxe_render::{Camera, MirrorError, SyncStats, wgpu};
use vxe_world::*;

pub const WIDTH: u32 = 320;
pub const HEIGHT: u32 = 180;

/// A sync outside a frame: records into an encoder of its own, submits it and waits for the
/// copies, so the belt's chunks come back before the next sync.
pub trait SyncNow {
    fn sync_now(
        &mut self,
        world: &World,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<SyncStats, MirrorError>;
}

impl SyncNow for GpuWorld {
    fn sync_now(
        &mut self,
        world: &World,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<SyncStats, MirrorError> {
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let r = self.sync(world, queue, &mut enc);
        queue.submit([enc.finish()]);
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("device poll after a sync");
        r
    }
}

/// A device on the software Vulkan adapter (lavapipe), or `None` with a printed skip when the
/// machine has none. Goldens are blessed on lavapipe, so no other adapter stands in for it.
/// With `VXE_REQUIRE_SOFTWARE_ADAPTER=1` a missing adapter panics instead, so a CI arm that
/// promises image comparisons cannot pass by skipping them.
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
        other => {
            let found = match other {
                Ok(a) => format!(
                    "{:?} adapter {}",
                    a.get_info().device_type,
                    a.get_info().name
                ),
                Err(e) => format!("no adapter ({e})"),
            };
            if std::env::var("VXE_REQUIRE_SOFTWARE_ADAPTER").as_deref() == Ok("1") {
                panic!("VXE_REQUIRE_SOFTWARE_ADAPTER=1 but the fallback is {found}");
            }
            println!("skipped: no software adapter ({found})");
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
        .expect("the view ray is in range")
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

// ---- The surface-look harness (tests/look.rs) ----

use flythrough::cave::ROCK;
use vxe_render::glam::Vec3;
use vxe_render::{Frame, GpuWorld, LightParams, LodParams, LookParams, MirrorConfig, readback};

/// Rows 0 (air) and 1..=5, plain, row 3 with the cave's rock albedo: the solid material-3
/// world of the cavity scene.
pub fn look_table() -> MaterialTable {
    let rock = cave_materials().get(ROCK).expect("rock row").clone();
    let mut t = MaterialTable::new();
    for i in 1..=5u16 {
        let m = if i == 3 {
            Material {
                name: "m3".to_owned(),
                ..rock.clone()
            }
        } else {
            Material::named(format!("m{i}"))
        };
        assert_eq!(t.push(m), i);
    }
    t
}

/// The cavity's centre and radius in voxels: the centre of voxel `[0, 0, 0]`.
pub const CAVITY_CENTRE: [f32; 3] = [0.5, 0.5, 0.5];
pub const CAVITY_RADIUS: i64 = 40;

/// Solid material 3 over chunks `[-2, 2)^3`, carved by a radius-40 sphere centred on voxel
/// `[0, 0, 0]`.
pub fn cavity_world() -> World {
    cavity_world_with(look_table())
}

pub fn cavity_world_with(table: MaterialTable) -> World {
    let world = World::generate(
        ChunkExtent::new([-2, -2, -2], [2, 2, 2]),
        &UniformSource(3),
        &table,
    );
    let r = world
        .apply(&EditOp::sphere(
            centre_of_voxel([0, 0, 0]),
            CAVITY_RADIUS * FIXED_ONE,
            Mode::Carve,
        ))
        .expect("the cavity is resident");
    assert!(!r.changed_from.is_empty());
    world
}

/// At the cavity's centre looking +x.
pub fn cavity_camera() -> Camera {
    Camera {
        position: CAVITY_CENTRE.into(),
        yaw: 0.0,
        pitch: 0.0,
        vfov: 70f32.to_radians(),
    }
}

/// The pillar sphere's centre and radius in voxels: the centre of voxel `[96, 64, 64]`.
pub const PILLAR_CENTRE: [f32; 3] = [96.5, 64.5, 64.5];
pub const PILLAR_RADIUS: f32 = 20.0;

/// Air over chunks `[0, 2)^3` under `table`, with a rock sphere of radius 20 filled at the
/// centre of voxel `[96, 64, 64]`.
pub fn pillar_world_with(table: MaterialTable) -> World {
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [2, 2, 2]),
        &UniformSource(AIR),
        &table,
    );
    world
        .apply(&EditOp::sphere(
            centre_of_voxel([96, 64, 64]),
            20 * FIXED_ONE,
            Mode::Fill(ROCK),
        ))
        .expect("the pillar is resident");
    world
}

pub fn pillar_world() -> World {
    pillar_world_with(cave_materials())
}

/// At `(16.5, 64.5, 64.5)` looking +x.
pub fn pillar_camera() -> Camera {
    Camera {
        position: [16.5, 64.5, 64.5].into(),
        yaw: 0.0,
        pitch: 0.0,
        vfov: 70f32.to_radians(),
    }
}

/// The occluder scene: air over chunks `[0, 2)^3`, a rock wall at x `[44, 46)` and a slab at
/// x `[28, 30)`, y and z `[56, 68)`, both facing -x toward the camera.
pub fn occluder_world() -> World {
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [2, 2, 2]),
        &UniformSource(AIR),
        &cave_materials(),
    );
    for (min, max) in [([44, 0, 0], [46, 128, 128]), ([28, 56, 56], [30, 68, 68])] {
        let op = EditOp::box_(
            Fixed3(min.map(|v: i64| v * FIXED_ONE)),
            Fixed3(max.map(|v: i64| v * FIXED_ONE)),
            Mode::Fill(ROCK),
        );
        world.apply(&op).expect("the occluder scene is resident");
    }
    world
}

/// At `(8.5, 64.5, 64.5)` looking +x.
pub fn occluder_camera() -> Camera {
    Camera {
        position: [8.5, 64.5, 64.5].into(),
        yaw: 0.0,
        pitch: 0.0,
        vfov: 70f32.to_radians(),
    }
}

/// The primary ray of pixel (`col`, `row`) of a `WIDTH` x `HEIGHT` target, as `march.wgsl`
/// builds it.
pub fn pixel_ray(camera: &Camera, col: u32, row: u32) -> Vec3 {
    let (fwd, right, up) = camera.basis();
    let tan = (camera.vfov * 0.5).tan();
    let aspect = WIDTH as f32 / HEIGHT as f32;
    let nx = (col as f32 + 0.5) / WIDTH as f32 * 2.0 - 1.0;
    let ny = (row as f32 + 0.5) / HEIGHT as f32 * 2.0 - 1.0;
    (fwd + right * (nx * tan * aspect) - up * (ny * tan)).normalize()
}

/// The normal a `view = Normals` pixel encodes.
pub fn decode_normal(p: &[u8]) -> Vec3 {
    Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32) / 255.0 * 2.0 - 1.0
}

/// Mean angle in degrees between each pixel's decoded normal and the cavity's analytic
/// normal. The camera sits at the centre `c`, so a hit `p = c + d t` has the inward normal
/// `(c - p) / |c - p| = -d`. A pixel that missed counts as 180 degrees.
pub fn cavity_normal_error(rgba: &[u8], camera: &Camera) -> f64 {
    let mut sum = 0.0f64;
    for (i, p) in rgba.chunks(4).enumerate() {
        let (col, row) = (i as u32 % WIDTH, i as u32 / WIDTH);
        let want = -pixel_ray(camera, col, row);
        let got = decode_normal(p);
        let angle = if got.length() < 0.5 {
            180.0
        } else {
            f64::from(got.normalize().dot(want).clamp(-1.0, 1.0))
                .acos()
                .to_degrees()
        };
        sum += angle;
    }
    sum / (rgba.len() / 4) as f64
}

/// Pixels where a `view = HitMask` render disagrees with the pillar's analytic disc: the
/// pixel's centre ray meets the sphere of radius `PILLAR_RADIUS` or not.
pub fn pillar_silhouette_error(rgba: &[u8], camera: &Camera) -> usize {
    let c = Vec3::from(PILLAR_CENTRE);
    let o = camera.position;
    rgba.chunks(4)
        .enumerate()
        .filter(|(i, p)| {
            let d = pixel_ray(camera, *i as u32 % WIDTH, *i as u32 / WIDTH);
            let oc = c - o;
            let along = oc.dot(d);
            let disc = along > 0.0 && oc.length_squared() - along * along <= PILLAR_RADIUS.powi(2);
            let hit = p[0] > 127;
            hit != disc
        })
        .count()
}

/// Makes every chunk of `world` for which `lod` holds LOD-only: out of the store, into the
/// world's mips.
pub fn make_lod(world: &World, lod: impl Fn(ChunkCoord) -> bool) {
    for c in world.store().coords() {
        if lod(c) {
            let chunk = world.store().remove(c).unwrap();
            let mips = ChunkMips::build(&chunk.read().unwrap());
            world.insert_lod(c, std::sync::Arc::new(mips));
        }
    }
}

/// The golden scene with its carve.
pub fn carved_world() -> World {
    let world = scene_world();
    let r = world.apply(&scene_carve(&world)).unwrap();
    assert!(!r.changed_from.is_empty(), "the carve removes rock");
    world
}

/// The carved golden scene with every chunk farther than squared distance 1 from the
/// camera's chunk LOD-only (`lod.png`'s scene).
pub fn lod_scene() -> World {
    let world = carved_world();
    let cam = VoxelCoord(scene_source().spawn().0).chunk();
    make_lod(&world, |c| {
        (0..3).map(|k| (c.0[k] - cam.0[k]).pow(2)).sum::<i32>() > 1
    });
    world
}

/// The LOD walk `lod.png` uses: every LOD-only chunk at level 1.
pub fn lod_forced_1() -> LodParams {
    LodParams {
        r_full: 1,
        forced_level: Some(1),
    }
}

/// A software device and the mirror of one world on it, rendering in any look.
pub struct LookRig {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub gpu: GpuWorld,
    pub frame: Frame,
}

impl LookRig {
    /// `world` mirrored under `cfg` (`None`: a fixed window over its extent) on the software
    /// adapter; `None` with a printed skip when there is none.
    pub fn new(world: &World, cfg: Option<MirrorConfig>) -> Option<Self> {
        let full = world.stats().full_bricks as u64;
        let (device, queue) = software_device(((full + 4096) * 512).max(64 << 20))?;
        Some(Self::on(device, queue, world, cfg))
    }

    pub fn on(
        device: wgpu::Device,
        queue: wgpu::Queue,
        world: &World,
        cfg: Option<MirrorConfig>,
    ) -> Self {
        let cfg =
            cfg.unwrap_or_else(|| MirrorConfig::new(vxe_render::Window::Fixed(world.extent())));
        let mut gpu = GpuWorld::with_config(&device, cfg);
        gpu.sync_now(world, &device, &queue)
            .expect("mirror fits the adapter");
        let frame = Frame::new(&device, WIDTH, HEIGHT);
        Self {
            device,
            queue,
            gpu,
            frame,
        }
    }

    /// One render of `camera` lit by `light` in `look`, read back as RGBA8.
    pub fn render(&mut self, camera: &Camera, light: &LightParams, look: LookParams) -> Vec<u8> {
        self.frame.set_look(look);
        self.frame
            .render(&self.device, &self.queue, &self.gpu, camera, light);
        readback::read_rgba8(&self.device, &self.queue, self.frame.target())
    }
}

/// At `position` (voxels) looking +x, 70 degrees vertical field of view.
pub fn camera_looking_x(position: [f32; 3]) -> Camera {
    Camera {
        position: position.into(),
        yaw: 0.0,
        pitch: 0.0,
        vfov: 70f32.to_radians(),
    }
}

/// The RGB bytes of the centre pixel `(WIDTH / 2, HEIGHT / 2)` of a `WIDTH` x `HEIGHT` image.
pub fn centre_pixel(rgba: &[u8]) -> [u8; 3] {
    let i = ((HEIGHT / 2) * WIDTH + WIDTH / 2) as usize * 4;
    [rgba[i], rgba[i + 1], rgba[i + 2]]
}
