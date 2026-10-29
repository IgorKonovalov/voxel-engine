//! The rigs of the upscalers' tests (Plan 0030's, shared with FSR 2's): the level at its
//! spawn, the presenter's blit, image distances, and the stripe wall with its sharpness
//! readings; and the crack scenes of the contour look's crack classes (Plan 0050): analytic
//! half-space worlds and the level at the settle rig's poses, mirrored with contour attributes.

use super::*;
use vxe_render::{
    Frame, GpuWorld, LightParams, MirrorConfig, Presenter, UpscalerKind, Window, readback, wgpu,
};
use vxe_world::*;

/// The internal and output sizes of the upscaler's tests: scale 1.5.
pub const INTERNAL: [u32; 2] = [256, 144];
pub const OUTPUT: [u32; 2] = [384, 216];

/// Frames the upscaler's still-camera tests accumulate.
pub const SETTLE: u32 = 32;

/// The level the fly-through generates with `--level --seed 1`, its ore emissive (the
/// fly-through's default), and its spawn pose.
pub fn level() -> (World, vxe_render::Camera) {
    let source = flythrough::cave::CaveSource::new(
        1,
        ChunkExtent::new([0, 0, 0], flythrough::cave::LEVEL_CHUNKS),
    );
    let table = flythrough::with_emissive_ore(&flythrough::cave_materials());
    let world = World::generate(source.extent, &source, &table);
    let camera = camera_looking_x(source.spawn().0.map(|v| v as f32 + 0.5));
    (world, camera)
}

/// A device and the level's mirror on it, focused on `camera` with the light ranges written.
pub fn level_rig(
    world: &World,
    camera: &vxe_render::Camera,
) -> Option<(wgpu::Device, wgpu::Queue, GpuWorld)> {
    let pool = flythrough::level_pool_units(world, true) * vxe_render::gpu_world::UNIT_BYTES;
    let (device, queue) = software_device(pool)?;
    let cfg = MirrorConfig {
        budget_bytes: 256 << 20,
        ..MirrorConfig::new(Window::Fixed(world.extent()))
    };
    let mut gpu = GpuWorld::with_config(&device, cfg);
    gpu.set_focus(VoxelCoord(
        camera.position.to_array().map(|c| c.floor() as i32),
    ));
    for _ in 0..4 {
        gpu.sync_now(world, &device, &queue)
            .expect("mirror fits the adapter");
    }
    Some((device, queue, gpu))
}

/// A frame of `internal` pixels for an `output` image, jittered or not, at the Plan 0024
/// defaults (lamp on, filter on, half-resolution lighting, coarse shadow walk, reuse).
pub fn frame_of(
    device: &wgpu::Device,
    internal: [u32; 2],
    output: [u32; 2],
    jitter: bool,
) -> Frame {
    let mut frame = Frame::with_sizes(device, internal, output);
    frame.set_jitter(device, jitter);
    frame
}

/// The presenter's bilinear blit of `source` onto a fresh `rgba8unorm` texture of `size`.
pub fn blit_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source: &wgpu::Texture,
    size: [u32; 2],
) -> Vec<u8> {
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("blit target"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let presenter = Presenter::new(device, format);
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    presenter.encode(device, &mut enc, source, &view);
    queue.submit([enc.finish()]);
    readback::read_rgba8(device, queue, &target)
}

/// Mean absolute difference per channel (r, g, b) of two RGBA8 images, in 1/255.
pub fn mad(a: &[u8], b: &[u8]) -> [f64; 3] {
    assert_eq!(a.len(), b.len());
    let n = (a.len() / 4) as f64;
    let mut s = [0.0f64; 3];
    for (p, q) in a.chunks(4).zip(b.chunks(4)) {
        for k in 0..3 {
            s[k] += f64::from(p[k].abs_diff(q[k]));
        }
    }
    s.map(|v| v / n)
}

/// `img`, `2 w` x `2 h`, box-filtered to `w` x `h`.
pub fn downsample_2x2(img: &[u8], [w, h]: [u32; 2]) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let mut out = vec![255u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            for k in 0..3 {
                let at = |xx: usize, yy: usize| u32::from(img[(yy * 2 * w + xx) * 4 + k]);
                let s = at(2 * x, 2 * y)
                    + at(2 * x + 1, 2 * y)
                    + at(2 * x, 2 * y + 1)
                    + at(2 * x + 1, 2 * y + 1);
                out[(y * w + x) * 4 + k] = ((s + 2) / 4) as u8;
            }
        }
    }
    out
}

// ---- The sharpness rig: a wall of one-voxel stripes facing the camera ----

/// The stripe wall's face: the plane x = `WALL_X`, the wall two voxels thick behind it.
pub const WALL_X: i64 = 160;
/// The stripes' materials: albedo 0.8 and 0.1, alternating along y one voxel at a time.
pub const BRIGHT: u16 = 1;
pub const DARK: u16 = 2;
/// Arm's length (0.7 m at 10 cm voxels) and the far distance, in voxels from the wall's face.
pub const NEAR: f32 = 7.0;
pub const FAR: f32 = 88.0;

/// Air over chunks `[0, 3)^3` with the stripe wall over y and z `[0, 192)`: material
/// `BRIGHT` on even y, `DARK` on odd.
pub fn stripe_world() -> World {
    let mut table = MaterialTable::new();
    for (name, albedo) in [("bright", 0.8f32), ("dark", 0.1)] {
        table.push(Material {
            albedo: [albedo; 3],
            ..Material::named(name)
        });
    }
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [3, 3, 3]),
        &UniformSource(AIR),
        &table,
    );
    let fill = |y0: i64, y1: i64, m: u16| {
        let op = EditOp::box_(
            Fixed3([WALL_X * FIXED_ONE, y0 * FIXED_ONE, 0]),
            Fixed3([(WALL_X + 2) * FIXED_ONE, y1 * FIXED_ONE, 192 * FIXED_ONE]),
            Mode::Fill(m),
        );
        world.apply(&op).expect("the wall is resident");
    };
    for y in 0..192 {
        fill(y, y + 1, if y % 2 == 0 { BRIGHT } else { DARK });
    }
    world
}

/// Sub-pixel phases each stripe reading averages over: the stripes' period in pixels is
/// close to whole at both distances, so one camera would see every edge at one sub-pixel
/// offset.
pub const PHASES: u32 = 5;

/// `dist` voxels in front of the wall, looking +x at it (the camera's right is -y, so the
/// stripes run vertically on screen), moved `(phase + 0.5) / PHASES` output pixels along its
/// right.
pub fn stripe_camera(dist: f32, phase: u32) -> vxe_render::Camera {
    let shift = (phase as f32 + 0.5) / PHASES as f32 / px_per_voxel(dist);
    camera_looking_x([WALL_X as f32 - dist, 96.3 - shift, 96.4])
}

/// Output pixels one voxel of the wall spans at `dist` voxels.
pub fn px_per_voxel(dist: f32) -> f32 {
    OUTPUT[1] as f32 / (2.0 * (35f32.to_radians()).tan()) / dist
}

/// The exposure that puts the bright stripe under the lamp's centre near the tone map's
/// knee at `dist`: the lamp's radiance falls as `1 / dist^2`.
pub fn stripe_exposure(dist: f32) -> f32 {
    dist * dist / 1200.0
}

/// The sharpness readings of one image: edge spread in output pixels (arm's length only),
/// gradient energy and Phase 4's rim gradient, over the image's middle half.
#[derive(Clone, Copy, Debug)]
pub struct Sharpness {
    pub es: f64,
    pub ge: f64,
    pub rim: f64,
}

impl std::fmt::Display for Sharpness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ES {:.3} px, GE {:.5}, rim {:.3}",
            self.es, self.ge, self.rim
        )
    }
}

/// Luminance (Rec. 709 weights on the display-encoded bytes), in 0..=255.
pub fn luma(img: &[u8], x: usize, y: usize) -> f64 {
    let p = &img[(y * OUTPUT[0] as usize + x) * 4..];
    0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2])
}

/// `img`'s sharpness seen from `camera` at `dist` voxels from the wall. Edge spread: per row
/// of the middle half and per stripe boundary whose projection lies in the middle half's
/// columns at least a quarter stripe in, the 10 to 90 % rise between the pixels a quarter
/// stripe either side of the boundary (by linear interpolation between pixel centres; the
/// 10 % crossing the last before the first 90 % one), over boundaries whose step exceeds
/// 32/255 and whose point on the wall lies inside the headlamp's inner cone, the mean (the
/// quarter stripe and the cone keep the lamp's falloff out of the step); NaN when no stripe
/// is over 6 pixels wide. Gradient energy: the mean of the squared forward
/// differences on both axes of luminance in 0..=1. Rim gradient: the mean of `|dx| + |dy|`
/// of luminance over the pixels whose 3 x 3 spans 16/255 or more in a channel.
pub fn sharpness(img: &[u8], camera: &vxe_render::Camera, dist: f32) -> Sharpness {
    let [w, h] = OUTPUT.map(|n| n as usize);
    let (x0, x1, y0, y1) = (w / 4, 3 * w / 4, h / 4, 3 * h / 4);
    let (mut ge, mut n) = (0.0f64, 0usize);
    let (mut rim, mut rim_n) = (0.0f64, 0usize);
    for y in y0..y1 {
        for x in x0..x1 {
            let l = luma(img, x, y);
            let dx = luma(img, x + 1, y) - l;
            let dy = luma(img, x, y + 1) - l;
            ge += (dx * dx + dy * dy) / (255.0 * 255.0);
            n += 1;
            let spans = (0..3).any(|k| {
                let (mut lo, mut hi) = (255u8, 0u8);
                for yy in y - 1..=y + 1 {
                    for xx in x - 1..=x + 1 {
                        let v = img[(yy * w + xx) * 4 + k];
                        lo = lo.min(v);
                        hi = hi.max(v);
                    }
                }
                hi - lo >= 16
            });
            if spans {
                rim += dx.abs() + dy.abs();
                rim_n += 1;
            }
        }
    }
    let s = px_per_voxel(dist);
    let (mut es, mut es_n) = (0.0f64, 0usize);
    if s > 6.0 {
        let (_, right, _) = camera.basis();
        let tan = (camera.vfov * 0.5).tan();
        let aspect = w as f32 / h as f32;
        let z = WALL_X as f32 - camera.position.x;
        // The boundary between voxel rows y = k - 1 and k, projected to a column.
        let col = |k: f32| {
            let d = vxe_render::glam::Vec3::new(z, k - camera.position.y, 0.0);
            ((d.dot(right) / z / (tan * aspect) + 1.0) * 0.5 * w as f32) as f64
        };
        let half = f64::from(s) / 4.0;
        for k in 0..=192 {
            let c = col(k as f32);
            if c - half < x0 as f64 || c + half >= x1 as f64 {
                continue;
            }
            let (ia, ib) = ((c - half).floor() as usize, (c + half).floor() as usize);
            for y in y0..y1 {
                // The boundary's point on the wall at this row lies inside the lamp's inner
                // cone: the lamp's falloff leaves the step alone.
                let up_t = (1.0 - 2.0 * (y as f32 + 0.5) / h as f32) * tan;
                let p = vxe_render::glam::Vec3::new(
                    WALL_X as f32,
                    k as f32,
                    camera.position.z + z * up_t,
                );
                let lamp = LightParams::headlamp();
                let (fwd, _, up) = camera.basis();
                let at = camera.position + right * lamp.offset_right - up * lamp.offset_down;
                if (p - at).normalize().dot(fwd) < lamp.inner.cos() {
                    continue;
                }
                let (a, b) = (luma(img, ia, y), luma(img, ib, y));
                let step = b - a;
                if step.abs() <= 32.0 {
                    continue;
                }
                let t = |i: usize| (luma(img, i, y) - a) / step;
                let Some(i90) = (ia..=ib).find(|&i| t(i) >= 0.9) else {
                    continue;
                };
                let cross = |i: usize, level: f64| {
                    let (t0, t1) = (t(i), t(i + 1));
                    i as f64 + 0.5 + (level - t0) / (t1 - t0)
                };
                let x90 = if i90 == ia {
                    ia as f64 + 0.5
                } else {
                    cross(i90 - 1, 0.9)
                };
                let Some(i10) = (ia..i90).rev().find(|&i| t(i) < 0.1) else {
                    continue;
                };
                es += x90 - cross(i10, 0.1);
                es_n += 1;
            }
        }
    }
    Sharpness {
        es: if es_n > 0 { es / es_n as f64 } else { f64::NAN },
        ge: ge / n as f64,
        rim: rim / rim_n.max(1) as f64,
    }
}

/// The stripe wall on the software adapter.
pub struct StripeRig {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub gpu: GpuWorld,
}

pub fn stripe_rig() -> Option<StripeRig> {
    let world = stripe_world();
    let full = world.stats().full_bricks as u64;
    let (device, queue) = software_device(((full + 4096) * 512).max(64 << 20))?;
    let mut gpu = GpuWorld::with_config(&device, MirrorConfig::new(Window::Fixed(world.extent())));
    gpu.sync_now(&world, &device, &queue)
        .expect("mirror fits the adapter");
    Some(StripeRig { device, queue, gpu })
}

/// The images the sharpness bars compare.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Image {
    /// Plan 0024's frame: 384 x 216, jitter off.
    N,
    /// 768 x 432, jitter off, box-filtered to 384 x 216.
    R,
    /// The bilinear blit of a 256 x 144 render, jitter off.
    B,
    /// The upscaler at scale 1 (384 x 216 internal), jittered.
    U1,
    /// The upscaler at scale 1.5 (256 x 144 internal), jittered.
    U15,
}

/// The settings of one stripe reading: the distance, the upscaler U1 and U15 run (ADR-0024's
/// with its tuning, or FSR 2), the frames panned after the 32 still ones (0: still), and the
/// sub-pixel phase.
#[derive(Clone, Copy)]
pub struct Shot {
    pub dist: f32,
    pub kind: UpscalerKind,
    pub pan: u32,
    pub phase: u32,
}

/// Output pixels a panning camera moves per frame.
pub const PAN_PX: f32 = 0.37;

/// `image` of the stripe wall under the headlamp at the Plan 0024 defaults (the exposure
/// [`stripe_exposure`]) after 32 still frames and `shot.pan` frames translating along the
/// camera's right at [`PAN_PX`] output pixels a frame, read at sharpening 0 and 0.5 (the
/// same for N, R and B; under FSR 2 both are its image at its settings' sharpness), with the
/// last frame's camera.
pub fn stripe_image(
    rig: &StripeRig,
    image: Image,
    shot: Shot,
) -> ([Vec<u8>; 2], vxe_render::Camera) {
    stripe_image_at(rig, image, shot, PAN_PX)
}

/// [`stripe_image`] with the camera panning at `pan_px` output pixels a frame.
pub fn stripe_image_at(
    rig: &StripeRig,
    image: Image,
    shot: Shot,
    pan_px: f32,
) -> ([Vec<u8>; 2], vxe_render::Camera) {
    let (internal, output, jitter) = match image {
        Image::N => (OUTPUT, OUTPUT, false),
        Image::R => (OUTPUT.map(|n| 2 * n), OUTPUT.map(|n| 2 * n), false),
        Image::B => (INTERNAL, INTERNAL, false),
        Image::U1 => (OUTPUT, OUTPUT, true),
        Image::U15 => (INTERNAL, OUTPUT, true),
    };
    let (device, queue) = (&rig.device, &rig.queue);
    let mut f = frame_of(device, internal, output, jitter);
    f.set_lighting(vxe_render::Lighting {
        exposure: vxe_render::Exposure::Manual(stripe_exposure(shot.dist)),
        ..vxe_render::Lighting::default()
    });
    let fsr = matches!(image, Image::U1 | Image::U15) && shot.kind.is_fsr2();
    match shot.kind {
        UpscalerKind::Own(u) => f.set_upscaler(u),
        k @ UpscalerKind::Fsr2(_) if fsr => f.set_upscaler_kind(device, k),
        _ => {}
    }
    f.set_sharpness(queue, 0.5);
    let light = LightParams::headlamp();
    let mut camera = stripe_camera(shot.dist, shot.phase);
    let (_, right, _) = camera.basis();
    let step = right * (pan_px / px_per_voxel(shot.dist));
    for i in 0..SETTLE + shot.pan {
        if i >= SETTLE {
            camera.position += step;
        }
        f.step(device, queue, &rig.gpu, &camera, &light);
    }
    let img = readback::read_rgba8(device, queue, f.target());
    let images = match image {
        Image::N => [img.clone(), img],
        Image::R => {
            let d = downsample_2x2(&img, OUTPUT);
            [d.clone(), d]
        }
        Image::B => {
            let b = blit_texture(device, queue, f.target(), OUTPUT);
            [b.clone(), b]
        }
        Image::U1 | Image::U15 if fsr => [img.clone(), img],
        Image::U1 | Image::U15 => {
            let hist = readback::read_rgba16f(device, queue, f.upscale_history().unwrap());
            let plain = hist
                .iter()
                .flat_map(|t| {
                    [t[0], t[1], t[2]]
                        .map(|c| (c * 255.0).round() as u8)
                        .into_iter()
                        .chain([255])
                })
                .collect();
            [plain, img]
        }
    };
    (images, camera)
}

/// Sharpness of `image` in `shot` at sharpening 0 and 0.5, the mean over the [`PHASES`]
/// sub-pixel phases.
pub fn stripe_sharpness(rig: &StripeRig, image: Image, shot: Shot) -> [Sharpness; 2] {
    stripe_sharpness_at(rig, image, shot, PAN_PX)
}

/// [`stripe_sharpness`] with the camera panning at `pan_px` output pixels a frame.
pub fn stripe_sharpness_at(
    rig: &StripeRig,
    image: Image,
    shot: Shot,
    pan_px: f32,
) -> [Sharpness; 2] {
    let mut sum = [Sharpness {
        es: 0.0,
        ge: 0.0,
        rim: 0.0,
    }; 2];
    for phase in 0..PHASES {
        let (imgs, camera) = stripe_image_at(rig, image, Shot { phase, ..shot }, pan_px);
        for (acc, img) in sum.iter_mut().zip(&imgs) {
            let r = sharpness(img, &camera, shot.dist);
            acc.es += r.es / f64::from(PHASES);
            acc.ge += r.ge / f64::from(PHASES);
            acc.rim += r.rim / f64::from(PHASES);
        }
    }
    sum
}

/// The first upscaler's (7cc58a4, [`Upscaler::lanczos`]) last panning frame at arm's length,
/// sharpening 0: edge spread and gradient energy at scale 1 and at 1.5.
pub const LANCZOS_PAN_U1: (f64, f64) = (2.020, 0.00357);
pub const LANCZOS_PAN_U15: (f64, f64) = (2.404, 0.00281);

/// `a <= b`, false when either is NaN.
pub fn at_most(a: f64, b: f64) -> bool {
    a <= b
}

/// `a >= b`, false when either is NaN.
pub fn at_least(a: f64, b: f64) -> bool {
    a >= b
}

/// The texels of a 32-bit float texture (r32float, rg32float or rgba32float) as `f32`, rows
/// top to bottom, every channel.
pub fn read_f32_texels(device: &wgpu::Device, queue: &wgpu::Queue, t: &wgpu::Texture) -> Vec<f32> {
    let channels = match t.format() {
        wgpu::TextureFormat::R32Float => 1,
        wgpu::TextureFormat::Rg32Float => 2,
        wgpu::TextureFormat::Rgba32Float => 4,
        f => panic!("{f:?} is not a 32-bit float format"),
    };
    let row = 4 * channels * t.width();
    let padded = row.div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("f32 texels"),
        size: u64::from(padded * t.height()),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    enc.copy_texture_to_buffer(
        t.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(t.height()),
            },
        },
        t.size(),
    );
    queue.submit([enc.finish()]);
    let bytes = readback::read_buffer(device, queue, &buffer, 0, buffer.size());
    bytes
        .chunks(padded as usize)
        .flat_map(|r| r[..row as usize].chunks(4))
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect()
}

/// FSR 2 at its default settings.
pub fn fsr2_kind() -> UpscalerKind {
    UpscalerKind::Fsr2(vxe_render::Fsr2Settings::default())
}

/// `p` projected through `v` to a fractional pixel of the target, pixel centres at integer +
/// 0.5, the jitter ignored (`project_prev` in `march.wgsl`); `None` behind the view.
pub fn project(v: &vxe_render::View, p: [f32; 3]) -> Option<[f64; 2]> {
    let d: Vec<f64> = (0..3).map(|k| f64::from(p[k] - v.origin[k])).collect();
    let dot = |a: [f32; 3]| (0..3).map(|k| d[k] * f64::from(a[k])).sum::<f64>();
    let z = dot(v.fwd);
    if z <= 1e-4 {
        return None;
    }
    let (x, y) = (dot(v.right) / z, dot(v.up) / z);
    let (tl, tr, tu, td) = (
        f64::from(v.tan_left),
        f64::from(v.tan_right),
        f64::from(v.tan_up),
        f64::from(v.tan_down),
    );
    let u = (x + tl) / (tl + tr);
    let w = (tu - y) / (tu + td);
    Some([
        f64::from(v.rect[0]) + u * f64::from(v.rect[2]),
        f64::from(v.rect[1]) + w * f64::from(v.rect[3]),
    ])
}

/// A radius-6 carve where the view axis of `camera` (looking +x) meets rock, applied to
/// `world`: the carve's `aabb` and its distance along the axis in voxels.
pub fn carve_in_view(
    world: &World,
    camera: &vxe_render::Camera,
) -> ((VoxelCoord, VoxelCoord), f64) {
    let o = Fixed3(
        camera
            .position
            .to_array()
            .map(|c| (c * FIXED_ONE as f32) as i64),
    );
    let hit = world
        .raycast(o, [FIXED_ONE, 0, 0], 400 * FIXED_ONE)
        .expect("the view ray is in range")
        .expect("the view ray meets rock");
    let carve = EditOp::sphere(
        Fixed3([o.0[0] + hit.t, o.0[1], o.0[2]]),
        6 * FIXED_ONE,
        Mode::Carve,
    );
    let aabb = world
        .apply(&carve)
        .expect("the carve applies")
        .aabb
        .expect("the carve removes rock");
    (aabb, hit.t as f64 / FIXED_ONE as f64)
}

/// Whether `p` lies in `aabb` dilated by one voxel, faces included (the passes' edit test).
pub fn in_dilated(p: [f32; 3], (min, max): (VoxelCoord, VoxelCoord)) -> bool {
    (0..3).all(|k| p[k] >= (min.0[k] - 1) as f32 && p[k] <= (max.0[k] + 2) as f32)
}

/// The octahedral snorm16 normal of a hit record (`octa16_decode` in `march.wgsl`).
pub fn octa16_normal(u: u16, v: u16) -> [f32; 3] {
    let p = [u as i16, v as i16].map(|q| f32::from(q) / 32767.0);
    let mut n = [p[0], p[1], 1.0 - p[0].abs() - p[1].abs()];
    if n[2] < 0.0 {
        let s = |x: f32| if x >= 0.0 { 1.0 } else { -1.0 };
        n = [
            (1.0 - n[1].abs()) * s(n[0]),
            (1.0 - n[0].abs()) * s(n[1]),
            n[2],
        ];
    }
    let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    n.map(|c| c / l)
}

/// The headlamp's unshadowed term of `params` at `p`, normal `n` (`up_lamp` in
/// `upscale.wgsl`): `intensity * cone * max(n.l, 0) / dist^2`.
pub fn lamp_term(params: &vxe_render::marcher::Params, p: [f32; 3], n: [f32; 3]) -> f32 {
    let to: Vec<f32> = (0..3).map(|k| params.light_pos[k] - p[k]).collect();
    let dist2 = (to[0] * to[0] + to[1] * to[1] + to[2] * to[2]).max(1e-4);
    let l: Vec<f32> = to.iter().map(|c| c / dist2.sqrt()).collect();
    let along = -(0..3).map(|k| l[k] * params.light_dir[k]).sum::<f32>();
    let (e0, e1) = (params.cos_outer, params.cos_inner);
    let t = ((along - e0) / (e1 - e0)).clamp(0.0, 1.0);
    let cone = t * t * (3.0 - 2.0 * t);
    let ndl = (0..3).map(|k| n[k] * l[k]).sum::<f32>().max(0.0);
    params.light_intensity * cone * ndl / dist2
}

/// The cave's table plus a translucent red glass row (opacity 0.5), and the glass's id.
pub fn glass_table() -> (MaterialTable, MaterialId) {
    let mut t = flythrough::cave_materials();
    let glass = t.push(Material {
        albedo: [0.9, 0.1, 0.1],
        flags: MaterialFlags::TRANSLUCENT,
        opacity: 0.5,
        ..Material::named("glass")
    });
    (t, glass)
}

/// `translucent.png`'s scene: air over chunks `[0, 2)^3`, a rock wall at x `[44, 46)` and a
/// glass slab at x `[36, 38)`, y and z `[56, 72)`; seen from [`translucent_camera`].
pub fn translucent_world() -> World {
    let (table, glass) = glass_table();
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [2, 2, 2]),
        &UniformSource(0),
        &table,
    );
    let fill = |min: [i64; 3], max: [i64; 3], m: MaterialId| {
        let op = EditOp::box_(
            Fixed3(min.map(|v| v * FIXED_ONE)),
            Fixed3(max.map(|v| v * FIXED_ONE)),
            Mode::Fill(m),
        );
        world.apply(&op).expect("the box is resident");
    };
    fill([44, 0, 0], [46, 128, 128], flythrough::cave::ROCK);
    fill([36, 56, 56], [38, 72, 72], glass);
    world
}

pub fn translucent_camera() -> vxe_render::Camera {
    camera_looking_x([8.5, 64.5, 64.5])
}

/// `tone.wgsl`'s arithmetic on the CPU (the shaded view) on pre-exposed colour `c`: ACES unless
/// the identity tone map, the 2.2 encode, rounded to a byte.
pub fn tone_bytes(c: [f32; 4], lighting: &vxe_render::Lighting) -> [u8; 3] {
    let identity = lighting.tone_map == vxe_render::ToneMap::Identity;
    [0, 1, 2].map(|k| {
        let x = c[k];
        let m = if identity {
            x.clamp(0.0, 1.0)
        } else {
            let x = x.max(0.0);
            ((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14)).clamp(0.0, 1.0)
        };
        (m.powf(1.0 / 2.2) * 255.0).round() as u8
    })
}

/// Puts `frame` under `kind`: ADR-0024's upscaler with its tuning (the jitter as the frame
/// has it), FSR 2, or none.
pub fn set_kind(device: &wgpu::Device, frame: &mut Frame, kind: UpscalerKind) {
    match kind {
        UpscalerKind::Own(u) => frame.set_upscaler(u),
        k => frame.set_upscaler_kind(device, k),
    }
}

// ---- Light changes under a still camera: the headlamp's sweep and step, and flicker ----

/// The pipelines the lamp bars compare: Plan 0024's frame (scale 1, jitter off) and the
/// upscaler at scale 1 and 1.5, jittered, all at a 384 x 216 output.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pipe {
    Reference,
    U1,
    U15,
}

impl Pipe {
    /// The frames of one jitter cycle of the pipeline under `kind`: FSR 2's own count at the
    /// pipeline's sizes, else [`vxe_render::marcher::JITTER_PHASES`].
    pub fn phases(self, kind: UpscalerKind) -> u32 {
        match (self, kind) {
            (Pipe::U1, UpscalerKind::Fsr2(_)) => {
                vxe_render::fsr2::jitter_phases(OUTPUT[0], OUTPUT[0])
            }
            (Pipe::U15, UpscalerKind::Fsr2(_)) => {
                vxe_render::fsr2::jitter_phases(INTERNAL[0], OUTPUT[0])
            }
            _ => vxe_render::marcher::JITTER_PHASES,
        }
    }

    /// The pipeline's frame under `kind` (Plan 0024's frame runs none), the lamp on or off.
    pub fn frame(self, device: &wgpu::Device, kind: UpscalerKind, lamp: bool) -> Frame {
        let mut f = match self {
            Pipe::Reference => frame_of(device, OUTPUT, OUTPUT, false),
            Pipe::U1 => frame_of(device, OUTPUT, OUTPUT, true),
            Pipe::U15 => frame_of(device, INTERNAL, OUTPUT, true),
        };
        if self != Pipe::Reference {
            set_kind(device, &mut f, kind);
        }
        f.set_lighting(vxe_render::Lighting {
            lamp,
            ..vxe_render::Lighting::default()
        });
        f
    }
}

/// The lamp's turn a frame during the sweep, in radians: about 2 output pixels a frame near
/// the image's centre (384 / 2 / tan(35 degrees) / (16 / 9) = 154 pixels a radian).
pub const SWEEP_RATE: f32 = 0.013;

/// Per-pixel luminance of an `OUTPUT` image.
pub fn lumas(img: &[u8]) -> Vec<f64> {
    let [w, h] = OUTPUT.map(|n| n as usize);
    (0..w * h).map(|i| luma(img, i % w, i / w)).collect()
}

/// The luminance-weighted mean column of `max(0, lit - off)`, and that contribution.
pub fn lamp_centroid(lit: &[f64], off: &[f64]) -> (f64, Vec<f64>) {
    let w = OUTPUT[0] as usize;
    let c: Vec<f64> = lit.iter().zip(off).map(|(a, b)| (a - b).max(0.0)).collect();
    let (mut s, mut sx) = (0.0, 0.0);
    for (i, v) in c.iter().enumerate() {
        s += v;
        sx += v * ((i % w) as f64 + 0.5);
    }
    (sx / s.max(1e-9), c)
}

/// The mean absolute luminance difference of two images, in 1/255.
pub fn luma_mad(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<f64>() / a.len() as f64
}

/// The frames, from the first after a change, until `series` (each frame's mean absolute
/// difference from the settled image) falls below 10 % of `step` (the change's size);
/// `None` if it does not within the series.
pub fn settle_frames(series: &[f64], step: f64) -> Option<usize> {
    series.iter().position(|&d| d < 0.1 * step).map(|i| i + 1)
}

/// One pipeline's lamp readings.
#[derive(Debug, Default)]
pub struct LampReading {
    /// The sweep's centroid over its last 16 frames, one per frame.
    pub centroids: Vec<f64>,
    /// The lamp's contribution over the sweep's last 16 frames.
    pub contributions: Vec<Vec<f64>>,
    /// Frames for the lamp switched off to settle within 10 % of the lamp's settled
    /// contribution.
    pub step: Option<usize>,
    /// Frames for an emission step (ore 1.0 to 0.25, lamp off) to settle within 10 %.
    pub emission: Option<usize>,
    /// The emission step's first and last difference from its settled image, in 1/255.
    pub emission_series: (f64, f64),
    /// The mean over pixels of a still lamp-lit image's luminance range over one jitter cycle.
    pub wobble: f64,
}

/// On the level at the spawn pose, a still camera: `pipe`'s lamp sweep (32 frames settled,
/// then the lamp's turn raised by [`SWEEP_RATE`] a frame for 32 frames, its contribution
/// against a settled lamp-off image of the same pipeline) and lamp step (32 frames settled,
/// then the lamp off for 40 frames); with `full` the emission step and the wobble too.
pub fn lamp_reading(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    gpu: &GpuWorld,
    camera: &vxe_render::Camera,
    pipe: Pipe,
    kind: UpscalerKind,
    full: bool,
) -> LampReading {
    let light = LightParams::headlamp();
    let settled = |lamp: bool, frames: u32| {
        let mut f = pipe.frame(device, kind, lamp);
        for _ in 0..frames {
            f.step(device, queue, gpu, camera, &light);
        }
        f
    };
    let off = lumas(&readback::read_rgba8(
        device,
        queue,
        settled(false, 48).target(),
    ));
    let mut r = LampReading::default();
    let mut f = settled(true, SETTLE);
    let mut swept = light;
    for i in 0..32 {
        swept.turn += SWEEP_RATE;
        f.step(device, queue, gpu, camera, &swept);
        if i >= 16 {
            let lit = lumas(&readback::read_rgba8(device, queue, f.target()));
            let (c, contribution) = lamp_centroid(&lit, &off);
            r.centroids.push(c);
            r.contributions.push(contribution);
        }
    }
    let mut f = settled(true, SETTLE);
    // The step's size: the settled lamp-lit image's difference from the lamp-off one. The
    // temporal pass drops the lamp's history on the switch, so the first frame after it is
    // already near the settled image and cannot be the measure.
    let step = luma_mad(
        &lumas(&readback::read_rgba8(device, queue, f.target())),
        &off,
    );
    f.set_lighting(vxe_render::Lighting {
        lamp: false,
        ..f.lighting()
    });
    let series: Vec<f64> = (0..40)
        .map(|_| {
            f.step(device, queue, gpu, camera, &light);
            luma_mad(
                &lumas(&readback::read_rgba8(device, queue, f.target())),
                &off,
            )
        })
        .collect();
    r.step = settle_frames(&series, step);
    if !full {
        return r;
    }
    let ore = flythrough::cave::ORE;
    let mut dim = settled(false, 0);
    dim.set_emission_scale(queue, ore, 0.25);
    for _ in 0..64 {
        dim.step(device, queue, gpu, camera, &light);
    }
    let target = lumas(&readback::read_rgba8(device, queue, dim.target()));
    let mut f = settled(false, 64);
    f.set_emission_scale(queue, ore, 0.25);
    let series: Vec<f64> = (0..64)
        .map(|_| {
            f.step(device, queue, gpu, camera, &light);
            luma_mad(
                &lumas(&readback::read_rgba8(device, queue, f.target())),
                &target,
            )
        })
        .collect();
    r.emission = settle_frames(&series, series[0]);
    r.emission_series = (series[0], series[series.len() - 1]);
    let mut f = settled(true, SETTLE);
    let n = (OUTPUT[0] * OUTPUT[1]) as usize;
    let (mut lo, mut hi) = (vec![f64::MAX; n], vec![f64::MIN; n]);
    for _ in 0..pipe.phases(kind) {
        f.step(device, queue, gpu, camera, &light);
        for (i, v) in lumas(&readback::read_rgba8(device, queue, f.target()))
            .into_iter()
            .enumerate()
        {
            lo[i] = lo[i].min(v);
            hi[i] = hi[i].max(v);
        }
    }
    r.wobble = hi.iter().zip(&lo).map(|(a, b)| a - b).sum::<f64>() / n as f64;
    r
}

/// The sweep's ideal centroids: for each of the sweep's last 16 frames, a single frame of
/// Plan 0024's pipeline with no history, lamp on at that turn, against one with the lamp off.
pub fn ideal_centroids(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    gpu: &GpuWorld,
    camera: &vxe_render::Camera,
) -> Vec<f64> {
    let light = LightParams::headlamp();
    let off_frame = Pipe::Reference.frame(device, UpscalerKind::Off, false);
    off_frame.render(device, queue, gpu, camera, &light);
    let off = lumas(&readback::read_rgba8(device, queue, off_frame.target()));
    let on = Pipe::Reference.frame(device, UpscalerKind::Off, true);
    (16..32)
        .map(|i| {
            let swept = LightParams {
                turn: SWEEP_RATE * (i + 1) as f32,
                ..light
            };
            on.render(device, queue, gpu, camera, &swept);
            lamp_centroid(
                &lumas(&readback::read_rgba8(device, queue, on.target())),
                &off,
            )
            .0
        })
        .collect()
}

/// The least-squares slope of `ys` against their index.
pub fn slope(ys: &[f64]) -> f64 {
    let n = ys.len() as f64;
    let mx = (n - 1.0) / 2.0;
    let my = ys.iter().sum::<f64>() / n;
    let (mut sxy, mut sxx) = (0.0, 0.0);
    for (i, y) in ys.iter().enumerate() {
        let dx = i as f64 - mx;
        sxy += dx * (y - my);
        sxx += dx * dx;
    }
    sxy / sxx
}

/// Every pipeline's lamp readings under `kind`, printed, with Phase 7's bars checked; the
/// return value lists the failures.
pub fn lamp_readings(kind: UpscalerKind, label: &str, full: bool) -> Option<Vec<String>> {
    let (world, camera) = level();
    let (device, queue, gpu) = level_rig(&world, &camera)?;
    let ideal = ideal_centroids(&device, &queue, &gpu, &camera);
    let speed = slope(&ideal);
    let lag = |r: &LampReading| {
        ideal
            .iter()
            .zip(&r.centroids)
            .map(|(a, b)| (a - b) / speed)
            .sum::<f64>()
            / ideal.len() as f64
    };
    println!(
        "[{label}] lamp sweep: {SWEEP_RATE} rad a frame, the ideal centroid moves {speed:.3} px a frame"
    );
    let reference = lamp_reading(&device, &queue, &gpu, &camera, Pipe::Reference, kind, full);
    let ref_lag = lag(&reference);
    println!(
        "[{label}] reference: lag {ref_lag:.2} frames, lamp step settles in {:?} frames, emission step {:?} ({:.3?}), wobble {:.3}/255",
        reference.step, reference.emission, reference.emission_series, reference.wobble
    );
    let mut failures = Vec::new();
    for pipe in [Pipe::U1, Pipe::U15] {
        let r = lamp_reading(&device, &queue, &gpu, &camera, pipe, kind, full);
        let l = lag(&r);
        // The trail: pixels the reference leaves under 4/255 that the upscaler lights over it.
        let (mut trail, mut dark) = (0usize, 0usize);
        for (c, rc) in r.contributions.iter().zip(&reference.contributions) {
            for (v, rv) in c.iter().zip(rc) {
                if *rv < 4.0 {
                    dark += 1;
                    trail += usize::from(*v > 4.0);
                }
            }
        }
        println!(
            "[{label}] {pipe:?}: lag {l:.2} frames, lamp step settles in {:?} frames, emission step {:?} ({:.3?}), wobble {:.3}/255, trail {:.3} % of the reference's dark pixels",
            r.step,
            r.emission,
            r.emission_series,
            r.wobble,
            trail as f64 * 100.0 / dark.max(1) as f64
        );
        if !at_most(l, ref_lag + 2.0) {
            failures.push(format!("{pipe:?}: lag {l:.2} over {:.2}", ref_lag + 2.0));
        }
        match (r.step, reference.step) {
            (Some(s), Some(rs)) if s <= rs + 2 => {}
            (s, rs) => failures.push(format!("{pipe:?}: lamp step {s:?} against {rs:?} + 2")),
        }
    }
    for f in &failures {
        println!("[{label}] over a bar: {f}");
    }
    Some(failures)
}

/// The dark cave's golden pose (the carved golden scene, ore emissive, voxel (264, 120, 376),
/// 15 degrees of yaw) with the lamp off and the filter on at the Plan 0024 defaults, at a
/// 320 x 180 output and scale 1.5 (213 x 120 internal), under `kind`, jittered or not (FSR 2
/// always jitters): settled over 64
/// still frames, then 32 more upscaled frames read back. Returns the mean over hit pixels
/// (output pixels whose nearest internal sample hit) and the 31 consecutive pairs of the
/// largest channel difference, and the share of (pixel, pair) values above 4/255.
pub fn still_vein_flicker(jitter: bool, kind: UpscalerKind) -> Option<(f64, f64)> {
    let source = scene_source();
    let world = World::generate(
        source.extent,
        &source,
        &flythrough::with_emissive_ore(&flythrough::cave_materials()),
    );
    world.apply(&scene_carve(&world)).unwrap();
    let output = [WIDTH, HEIGHT];
    let internal = vxe_render::RenderScale::OneHalf.internal(output);
    assert_eq!(internal, [213, 120]);
    let full = world.stats().full_bricks as u64;
    let (device, queue) = software_device(((full + 4096) * 512).max(64 << 20))?;
    let mut gpu = GpuWorld::with_config(&device, MirrorConfig::new(Window::Fixed(world.extent())));
    gpu.set_focus(VoxelCoord([264, 120, 376]));
    for _ in 0..4 {
        gpu.sync_now(&world, &device, &queue).unwrap();
    }
    let mut frame = frame_of(&device, internal, output, jitter);
    set_kind(&device, &mut frame, kind);
    frame.set_lighting(vxe_render::Lighting {
        lamp: false,
        ..vxe_render::Lighting::default()
    });
    let camera = vxe_render::Camera {
        position: [264.5, 120.5, 376.5].into(),
        yaw: 15f32.to_radians(),
        pitch: 0.0,
        vfov: 70f32.to_radians(),
    };
    let light = LightParams::headlamp();
    for _ in 0..64 {
        frame.step(&device, &queue, &gpu, &camera, &light);
    }
    let mut frames = Vec::new();
    let mut hits = Vec::new();
    for _ in 0..32 {
        frame.step(&device, &queue, &gpu, &camera, &light);
        frames.push(readback::read_rgba8(&device, &queue, frame.target()));
        if hits.is_empty() {
            let rec = readback::read_rgba16u(&device, &queue, frame.hit_records());
            let j = frame.params(&gpu, &camera, &light).views[0].jitter;
            for y in 0..output[1] {
                for x in 0..output[0] {
                    let n = [(x, 0), (y, 1)].map(|(c, k)| {
                        let centre = (c as f32 + 0.5) * internal[k] as f32 / output[k] as f32;
                        ((centre - j[k]).floor() as i32).clamp(0, internal[k] as i32 - 1) as usize
                    });
                    hits.push(rec[n[1] * internal[0] as usize + n[0]][3] & 1 == 1);
                }
            }
        }
    }
    let (mut sum, mut over, mut n) = (0.0f64, 0usize, 0usize);
    for pair in frames.windows(2) {
        for (i, &hit) in hits.iter().enumerate() {
            if !hit {
                continue;
            }
            let d = (0..3)
                .map(|k| pair[0][4 * i + k].abs_diff(pair[1][4 * i + k]))
                .max()
                .unwrap_or(0);
            sum += f64::from(d);
            over += usize::from(d > 4);
            n += 1;
        }
    }
    Some((sum / n as f64, over as f64 / n as f64))
}

// ---- The crack scenes (tests/look.rs's crack classes) ----

/// Air over chunks `[0, 2)^3` with `ROCK` wherever voxel `v` satisfies any of the half-spaces
/// `a . (2 v + 1) <= c`: integer tests on twice the voxel centre, so each scene's plane cuts
/// the voxel grid exactly where its formula says.
pub struct HalfSpaces(pub Vec<([i64; 3], i64)>);

impl WorldSource for HalfSpaces {
    fn chunk(&self, coord: ChunkCoord, out: &mut Chunk) {
        let base = coord.min_voxel().0;
        for slot in 0..CHUNK_BRICKS {
            let b = brick_of_slot(slot);
            out.set_brick(
                slot,
                Brick::from_fn(|l| {
                    let v = [0, 1, 2].map(|k| i64::from(base[k] + b[k] * BRICK_EDGE + l[k]));
                    let solid = self
                        .0
                        .iter()
                        .any(|(a, c)| (0..3).map(|k| a[k] * (2 * v[k] + 1)).sum::<i64>() <= *c);
                    if solid { ROCK } else { AIR }
                }),
            );
        }
    }
}

/// The crack scenes' box centre, in voxels.
pub const CRACK_CENTRE: [f32; 3] = [64.0, 64.0, 64.0];

/// `HalfSpaces` over chunks `[0, 2)^3` under `table`.
pub fn half_space_world(spaces: Vec<([i64; 3], i64)>, table: &MaterialTable) -> World {
    let source = HalfSpaces(spaces);
    World::generate(ChunkExtent::new([0, 0, 0], [2, 2, 2]), &source, table)
}

/// Rock below the plane through [`CRACK_CENTRE`] with normal `normalize(1, 2, 3)`: neighbouring
/// surface voxels see different 27-neighbourhoods, so their planes' offsets disagree.
pub fn tilted_slab_world(table: &MaterialTable) -> World {
    // a . (2 v + 1 - 128) <= 0 with a = (1, 2, 3).
    half_space_world(vec![([1, 2, 3], 128 * 6)], table)
}

/// Rock below the plane `x + z = 128` through [`CRACK_CENTRE`], normal `normalize(1, 0, 1)`: a
/// 45-degree staircase whose surface voxels all see the same 27-neighbourhood.
pub fn axis_slab_world(table: &MaterialTable) -> World {
    half_space_world(vec![([1, 0, 1], 256)], table)
}

/// Rock below the plane `x + y + z = 192` through [`CRACK_CENTRE`], normal
/// `normalize(1, 1, 1)`: its surface voxels (those with `x + y + z = 191`) all see the same
/// 27-neighbourhood, and the attribute's 6-bit octahedral code holds that normal exactly
/// (`(1/3, 1/3)` is code 42 of 63 on each axis), so every surface voxel's plane is the same
/// plane.
pub fn diagonal_slab_world(table: &MaterialTable) -> World {
    half_space_world(vec![([1, 1, 1], 384)], table)
}

/// The diagonal slab from its air side, 30 voxels out along its normal and 5 along
/// `(1, -1, 0)`, looking at the centre: nearly head-on, so every hit lies within about 40
/// voxels of the centre and the window's faces, where the surface voxels' neighbourhoods are
/// cut, stay out of the frame.
pub fn diagonal_slab_camera() -> vxe_render::Camera {
    let n = Vec3::new(1.0, 1.0, 1.0).normalize();
    let t = Vec3::new(1.0, -1.0, 0.0).normalize();
    let p = Vec3::from(CRACK_CENTRE) + 30.0 * n + 5.0 * t;
    camera_toward(p.to_array(), CRACK_CENTRE)
}

/// A flat floor, rock at `z < 64`.
pub fn floor_world(table: &MaterialTable) -> World {
    half_space_world(vec![([0, 0, 1], 127)], table)
}

/// A flat vertical wall, rock at `x >= 80`, its face toward `-x`.
pub fn wall_world(table: &MaterialTable) -> World {
    half_space_world(vec![([-1, 0, 0], -161)], table)
}

/// The floor obliquely: 20 voxels above `(64, 64, 64)` and 30 back along `-x`, looking at it.
pub fn floor_camera() -> vxe_render::Camera {
    camera_toward([34.0, 64.0, 84.0], [64.0, 64.0, 64.0])
}

/// The wall obliquely: 20 voxels out from `(80, 64, 64)` along `-x` and 30 along `-y`,
/// looking at it.
pub fn wall_camera() -> vxe_render::Camera {
    camera_toward([60.0, 34.0, 64.0], [80.0, 64.0, 64.0])
}

/// A floor (rock at z < 64) meeting a wall (rock at x >= 80) at 90 degrees: the concave corner
/// along the line x = 80, z = 64.
pub fn corner_world(table: &MaterialTable) -> World {
    half_space_world(vec![([0, 0, 1], 127), ([-1, 0, 0], -161)], table)
}

/// At `position` looking at `target`, 70 degrees vertical field of view.
pub fn camera_toward(position: [f32; 3], target: [f32; 3]) -> vxe_render::Camera {
    let d = Vec3::from(target) - Vec3::from(position);
    vxe_render::Camera {
        position: position.into(),
        yaw: d.y.atan2(d.x),
        pitch: (d.z / d.length()).asin(),
        vfov: 70f32.to_radians(),
    }
}

/// 30 voxels out from [`CRACK_CENTRE`] along the slab's normal `n` and 20 along `t`, looking
/// at the centre: an oblique view of the slab's surface.
pub fn slab_camera(n: [f32; 3], t: [f32; 3]) -> vxe_render::Camera {
    let (n, t) = (Vec3::from(n).normalize(), Vec3::from(t).normalize());
    let p = Vec3::from(CRACK_CENTRE) + 30.0 * n + 20.0 * t;
    camera_toward(p.to_array(), CRACK_CENTRE)
}

/// The tilted slab from its air side, obliquely.
pub fn tilted_slab_camera() -> vxe_render::Camera {
    slab_camera([1.0, 2.0, 3.0], [2.0, -1.0, 0.0])
}

/// The 45-degree slab from its air side, obliquely.
pub fn axis_slab_camera() -> vxe_render::Camera {
    slab_camera([1.0, 0.0, 1.0], [0.0, 1.0, 0.0])
}

/// Inside the corner, at `(50.5, 64.5, 82.5)` looking +x at a pitch of -30 degrees: the
/// centre ray meets the floor at the foot of the wall.
pub fn corner_camera() -> vxe_render::Camera {
    vxe_render::Camera {
        position: [50.5, 64.5, 82.5].into(),
        yaw: 0.0,
        pitch: (-30f32).to_radians(),
        vfov: 70f32.to_radians(),
    }
}

/// `world` mirrored with contour attributes under plane fit `fit`, every attribute computed,
/// over a fixed window, with a `size` frame lighting at full resolution.
pub fn fit_rig_on(
    device: wgpu::Device,
    queue: wgpu::Queue,
    world: &World,
    fit: vxe_render::SurfaceFit,
    size: [u32; 2],
) -> LookRig {
    let cfg = MirrorConfig {
        budget_bytes: 512 << 20,
        ..MirrorConfig::new(Window::Fixed(world.extent()))
    };
    let mut gpu = GpuWorld::with_surface_attrs(&device, cfg);
    gpu.set_surface_fit(fit);
    gpu.set_focus(VoxelCoord(CRACK_CENTRE.map(|c| c as i32)));
    for _ in 0..4 {
        gpu.sync_now(world, &device, &queue)
            .expect("mirror fits the adapter");
    }
    drain_attrs(&device, &queue, &mut gpu);
    let mut frame = Frame::new(&device, size[0], size[1]);
    frame.set_resolution(&device, vxe_render::LightingResolution::Full);
    LookRig {
        device,
        queue,
        gpu,
        frame,
    }
}

/// Runs the attribute pass until its work list is empty.
pub fn drain_attrs(device: &wgpu::Device, queue: &wgpu::Queue, gpu: &mut GpuWorld) {
    loop {
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let items = gpu.update_attrs(&mut enc);
        queue.submit([enc.finish()]);
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("device poll after the attribute pass");
        if items == 0 {
            break;
        }
    }
}

/// The level the fly-through generates with `--level --seed 1` under its default look's
/// materials, the ore emissive: the world the owner's `--pose` runs show.
pub fn pose_level() -> World {
    let source = flythrough::cave::CaveSource::new(
        1,
        ChunkExtent::new([0, 0, 0], flythrough::cave::LEVEL_CHUNKS),
    );
    let table = flythrough::with_emissive_ore(&flythrough::LookPreset::DEFAULT.materials());
    World::generate(source.extent, &source, &table)
}

/// The level's frame size in the crack readings.
pub const POSE_SIZE: [u32; 2] = [384, 216];

/// The level mirrored whole with contour attributes under `fit` on `device`, focused on
/// `pose`, with a [`POSE_SIZE`] frame at the fly-through's lighting defaults (no upscaler).
pub fn pose_rig_on(
    device: wgpu::Device,
    queue: wgpu::Queue,
    world: &World,
    pose: &flythrough::Pose,
    fit: vxe_render::SurfaceFit,
) -> LookRig {
    let cfg = MirrorConfig {
        budget_bytes: 1 << 30,
        ..MirrorConfig::new(Window::Fixed(world.extent()))
    };
    let mut gpu = GpuWorld::with_surface_attrs(&device, cfg);
    gpu.set_surface_fit(fit);
    gpu.set_focus(VoxelCoord(pose.position.map(|c| c.floor() as i32)));
    for _ in 0..4 {
        gpu.sync_now(world, &device, &queue)
            .expect("mirror fits the adapter");
    }
    drain_attrs(&device, &queue, &mut gpu);
    let frame = Frame::new(&device, POSE_SIZE[0], POSE_SIZE[1]);
    LookRig {
        device,
        queue,
        gpu,
        frame,
    }
}
