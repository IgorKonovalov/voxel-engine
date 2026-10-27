//! Shared fixtures for the GPU tests: the software adapter, the golden scene, PNG files and
//! image comparisons.
#![allow(dead_code, reason = "shared across test binaries; each uses a subset")]

pub mod rigs;
pub mod sharpness;

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
        let mut frame = Frame::new(&device, WIDTH, HEIGHT);
        // The walk and the look at full resolution: lighting at half would blur them.
        frame.set_resolution(&device, vxe_render::LightingResolution::Full);
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

// ---- Contour attributes and the sharpness scenes (tests/look.rs, tests/sharpness.rs) ----

use vxe_render::gpu_world::{PAGE_UNITS, UNIT_BYTES};
use vxe_render::{Lighting, LightingResolution, NormalMode, ViewMode};

/// Pool bytes a mirror with attributes needs for `world`'s bricks, with a page of headroom
/// for each bin's partly filled page.
pub fn attr_pool_bytes(world: &World) -> u64 {
    (flythrough::level_pool_units(world, true) + 10 * u64::from(PAGE_UNITS)) * UNIT_BYTES
}

/// `world` mirrored with contour attributes over a fixed window, shards capped at
/// `max_shard_units`, its attributes filled; lighting at full resolution.
pub fn attr_rig_on(
    device: wgpu::Device,
    queue: wgpu::Queue,
    world: &World,
    max_shard_units: Option<u32>,
) -> LookRig {
    let cfg = MirrorConfig {
        max_shard_units,
        ..MirrorConfig::new(vxe_render::Window::Fixed(world.extent()))
    };
    let mut gpu = GpuWorld::with_surface_attrs(&device, cfg);
    gpu.sync_now(world, &device, &queue)
        .expect("mirror fits the adapter");
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let bricks = gpu.update_attrs(&mut enc);
    queue.submit([enc.finish()]);
    assert_eq!(
        bricks,
        world.stats().full_bricks,
        "every Full brick has its attribute"
    );
    let mut frame = Frame::new(&device, WIDTH, HEIGHT);
    // The walk and the look at full resolution: lighting at half would blur them.
    frame.set_resolution(&device, LightingResolution::Full);
    LookRig {
        device,
        queue,
        gpu,
        frame,
    }
}

/// [`attr_rig_on`] on the software adapter; `None` with a printed skip when there is none.
pub fn attr_rig(world: &World) -> Option<LookRig> {
    let (device, queue) = software_device(attr_pool_bytes(world).max(64 << 20))?;
    Some(attr_rig_on(device, queue, world, None))
}

/// Static frames a sharpness render runs; the last is read.
pub const SHARP_FRAMES: usize = 64;

/// One frame setting of the sharpness harness: the look and the lighting path.
#[derive(Clone, Copy, Debug)]
pub struct Setting {
    pub look: LookParams,
    pub tier: LightingResolution,
    /// The a-trous filter runs.
    pub filter: bool,
    /// Radiance is multiplied by this before the tone map (the frame's default 1).
    pub exposure: f32,
}

impl Setting {
    /// `mode` with detail `detail`, shaded, at the default tier with the filter on.
    pub fn new(mode: NormalMode, detail: bool) -> Self {
        Self {
            look: LookParams {
                normal_mode: mode,
                detail,
                view: ViewMode::Shaded,
                ..LookParams::default()
            },
            tier: LightingResolution::default(),
            filter: true,
            exposure: 1.0,
        }
    }

    pub fn at(self, tier: LightingResolution, filter: bool) -> Self {
        Self {
            tier,
            filter,
            ..self
        }
    }

    /// `face` or `contour`, `+ detail`, the tier, `filter on|off` and the exposure when it
    /// is not 1.
    pub fn name(&self) -> String {
        format!(
            "{}{} {} filter {}{}",
            self.look.normal_mode.name(),
            if self.look.detail { " + detail" } else { "" },
            self.tier.name(),
            if self.filter { "on" } else { "off" },
            if self.exposure == 1.0 {
                String::new()
            } else {
                format!(" exposure {}", self.exposure)
            }
        )
    }
}

impl LookRig {
    /// [`SHARP_FRAMES`] static frames of `camera` in `setting`, lit by the headlamp alone (no
    /// emissive material in the scenes' tables), with history from a reset; returns the
    /// last, after asserting the last two differ by at most 1/255 in every channel.
    pub fn converge(&mut self, camera: &Camera, setting: Setting) -> Vec<u8> {
        // A fresh frame for every reading: a frame's lighting targets carry state past
        // `reset_history` that moves a converged reading by about 0.5 % of `S_fine`.
        self.frame = Frame::new(&self.device, WIDTH, HEIGHT);
        self.frame.set_look(setting.look);
        self.frame.set_resolution(&self.device, setting.tier);
        self.frame.set_lighting(Lighting {
            lamp: true,
            filter: setting.filter,
            exposure: vxe_render::Exposure::Manual(setting.exposure),
            ..Lighting::default()
        });
        let light = LightParams::headlamp();
        let mut prev = Vec::new();
        for i in 0..SHARP_FRAMES {
            self.frame
                .step(&self.device, &self.queue, &self.gpu, camera, &light);
            if i + 2 >= SHARP_FRAMES {
                let img = readback::read_rgba8(&self.device, &self.queue, self.frame.target());
                if !prev.is_empty() {
                    let d = sharpness::max_channel_diff(&prev, &img);
                    assert!(
                        d <= 1,
                        "{}: the last two frames differ by {d}/255",
                        setting.name()
                    );
                }
                prev = img;
            }
        }
        prev
    }
}

/// The sharpness scenes' table: the fly-through's look table (no emissive material) with
/// rock's detail at `rock_detail`.
pub fn sharp_table(rock_detail: [f32; 2]) -> MaterialTable {
    flythrough::look_materials_with(rock_detail)
}

/// The wall's roughening: 24 spheres from a test-local LCG seeded 39, each `(centre voxel,
/// radius in voxels, carve)`: even indices carve, odd ones fill; radius 3 to 8, centre x
/// within 2 voxels of the face plane `x = 74`, y and z in `[44, 85)`.
pub fn wall_spheres() -> Vec<([i32; 3], i64, bool)> {
    let mut state: u64 = 39;
    let mut next = |n: u64| {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) % n
    };
    (0..24)
        .map(|i| {
            let r = 3 + next(6) as i64;
            let x = 72 + next(5) as i32;
            let y = 44 + next(41) as i32;
            let z = 44 + next(41) as i32;
            ([x, y, z], r, i % 2 == 0)
        })
        .collect()
}

/// Air over chunks `[0, 2)^3` under `table`, rock filled at `x >= 74` and roughened by
/// [`wall_spheres`] in order.
pub fn wall_world_with(table: MaterialTable) -> World {
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [2, 2, 2]),
        &UniformSource(AIR),
        &table,
    );
    world
        .apply(&EditOp::box_(
            Fixed3([74 * FIXED_ONE, 0, 0]),
            Fixed3([128 * FIXED_ONE; 3]),
            Mode::Fill(ROCK),
        ))
        .expect("the wall is resident");
    for (c, r, carve) in wall_spheres() {
        let mode = if carve { Mode::Carve } else { Mode::Fill(ROCK) };
        world
            .apply(&EditOp::sphere(centre_of_voxel(c), r * FIXED_ONE, mode))
            .expect("the wall is resident");
    }
    world
}

/// At `(64.5, 64.5, 64.5)` looking +x: the mean face 9.5 voxels away.
pub fn wall_camera() -> Camera {
    camera_looking_x([64.5, 64.5, 64.5])
}

/// The edge scene's convex edge: the vertical line `x = y = EDGE`. Off the brick grid, so the
/// bricks on both faces are `Full` and carry contour attributes: a face on a brick boundary
/// leaves uniform solid bricks behind it, which have no attributes and hit as cubes.
pub const EDGE: i64 = 63;

/// Air over chunks `[0, 2)^3` under `table` with a rock block at x and y in `[EDGE, 96)`, z
/// in `[32, 96)`.
pub fn edge_world_with(table: MaterialTable) -> World {
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [2, 2, 2]),
        &UniformSource(AIR),
        &table,
    );
    world
        .apply(&EditOp::box_(
            Fixed3([EDGE * FIXED_ONE, EDGE * FIXED_ONE, 32 * FIXED_ONE]),
            Fixed3([96 * FIXED_ONE, 96 * FIXED_ONE, 96 * FIXED_ONE]),
            Mode::Fill(ROCK),
        ))
        .expect("the block is resident");
    world
}

/// 10 voxels from the edge, at `(EDGE - 10 cos 30, EDGE - 10 sin 30, 64.5)` looking along
/// `(cos 30, sin 30, 0)`: the edge is the image's centre line, the -x face on its left and
/// the -y face on its right.
pub fn edge_camera() -> Camera {
    let a = 30f32.to_radians();
    let e = EDGE as f32;
    Camera {
        position: [e - 10.0 * a.cos(), e - 10.0 * a.sin(), 64.5].into(),
        yaw: a,
        pitch: 0.0,
        vfov: 70f32.to_radians(),
    }
}
