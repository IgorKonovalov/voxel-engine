//! The internal resolution, the jitter and the upscaler (ADR-0024), on the software adapter
//! (lavapipe).

mod support;

use support::*;
use vxe_render::marcher::Upscaler;
use vxe_render::{
    Frame, GpuWorld, LightParams, LookParams, MirrorConfig, Presenter, ViewMode, Window, readback,
    wgpu,
};
use vxe_world::*;

/// `world` mirrored over its extent on the software adapter, with a frame of `internal`
/// pixels for an `output` image, lighting at full resolution; `None` without the adapter.
fn rig(world: &World, internal: [u32; 2], output: [u32; 2]) -> Option<LookRig> {
    let full = world.stats().full_bricks as u64;
    let (device, queue) = software_device(((full + 4096) * 512).max(64 << 20))?;
    let cfg = MirrorConfig::new(Window::Fixed(world.extent()));
    let mut gpu = GpuWorld::with_config(&device, cfg);
    gpu.sync_now(world, &device, &queue)
        .expect("mirror fits the adapter");
    let mut frame = Frame::with_sizes(&device, internal, output);
    frame.set_resolution(&device, vxe_render::LightingResolution::Full);
    Some(LookRig {
        device,
        queue,
        gpu,
        frame,
    })
}

fn hitmask() -> LookParams {
    LookParams {
        view: ViewMode::HitMask,
        ..LookParams::default()
    }
}

/// The hit mask of the pillar from `pillar_camera`, frames `0..16` each rendered alone (no
/// history) and averaged per pixel: the red channel's mean, rounded.
fn averaged_mask(rig: &mut LookRig, jitter: bool) -> Vec<u8> {
    rig.frame.set_jitter(&rig.device, jitter);
    rig.frame.set_look(hitmask());
    let mut sum = vec![0u32; (WIDTH * HEIGHT) as usize];
    for f in 0..16 {
        rig.frame.set_frame_index(f);
        rig.frame.render(
            &rig.device,
            &rig.queue,
            &rig.gpu,
            &pillar_camera(),
            &LightParams::at_eye(),
        );
        let img = readback::read_rgba8(&rig.device, &rig.queue, rig.frame.target());
        for (s, p) in sum.iter_mut().zip(img.chunks(4)) {
            *s += u32::from(p[0]);
        }
    }
    sum.iter().map(|&s| ((s + 8) / 16) as u8).collect()
}

/// At scale 1 the hit mask of the pillar averaged over 16 frames is 0 or 255 in every pixel
/// with the jitter off; with it on the pixels along the pillar's edge (those of the unjittered
/// mask with a 4-neighbour on the other side of the silhouette) take at least three distinct
/// coverage values strictly between 0 and 255: the jitter resolves sub-pixel coverage.
#[test]
fn the_jitter_resolves_sub_pixel_coverage_along_the_pillars_edge() {
    let world = pillar_world();
    let Some(mut rig) = rig(&world, [WIDTH, HEIGHT], [WIDTH, HEIGHT]) else {
        return;
    };
    let still = averaged_mask(&mut rig, false);
    let jittered = averaged_mask(&mut rig, true);
    let partial_still = still.iter().filter(|&&v| v != 0 && v != 255).count();
    assert_eq!(partial_still, 0, "unjittered pixels between 0 and 255");
    let (w, h) = (WIDTH as i32, HEIGHT as i32);
    let at = |x: i32, y: i32| still[(y * w + x) as usize];
    let mut edge = Vec::new();
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let v = at(x, y);
            if [(1, 0), (-1, 0), (0, 1), (0, -1)]
                .iter()
                .any(|&(dx, dy)| at(x + dx, y + dy) != v)
            {
                edge.push(jittered[(y * w + x) as usize]);
            }
        }
    }
    let mut partial: Vec<u8> = edge
        .iter()
        .copied()
        .filter(|&v| v != 0 && v != 255)
        .collect();
    partial.sort_unstable();
    partial.dedup();
    println!(
        "edge pixels {}, distinct coverage values in (0, 255): {} {:?}",
        edge.len(),
        partial.len(),
        partial
    );
    assert!(partial.len() >= 3, "{} distinct values", partial.len());
}

/// The presenter's bilinear blit of `source` onto a fresh `rgba8unorm` texture of `size`,
/// read back.
fn blit(rig: &LookRig, size: [u32; 2]) -> Vec<u8> {
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let target = rig.device.create_texture(&wgpu::TextureDescriptor {
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
    let presenter = Presenter::new(&rig.device, format);
    let mut enc = rig
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    presenter.encode(&rig.device, &mut enc, rig.frame.target(), &view);
    rig.queue.submit([enc.finish()]);
    readback::read_rgba8(&rig.device, &rig.queue, &target)
}

/// The bilinear sample of the `w` x `h` image `src` at output pixel `(x, y)` of an
/// `ow` x `oh` image, clamped to the edge, channel `k`.
fn bilinear(src: &[u8], [w, h]: [u32; 2], [ow, oh]: [u32; 2], x: u32, y: u32, k: usize) -> f32 {
    let sx = (x as f32 + 0.5) / ow as f32 * w as f32 - 0.5;
    let sy = (y as f32 + 0.5) / oh as f32 * h as f32 - 0.5;
    let (x0, y0) = (sx.floor(), sy.floor());
    let (fx, fy) = (sx - x0, sy - y0);
    let texel = |xi: f32, yi: f32| {
        let xi = xi.clamp(0.0, (w - 1) as f32) as usize;
        let yi = yi.clamp(0.0, (h - 1) as f32) as usize;
        f32::from(src[(yi * w as usize + xi) * 4 + k])
    };
    let top = texel(x0, y0) * (1.0 - fx) + texel(x0 + 1.0, y0) * fx;
    let bottom = texel(x0, y0 + 1.0) * (1.0 - fx) + texel(x0 + 1.0, y0 + 1.0) * fx;
    top * (1.0 - fy) + bottom * fy
}

/// At scale 1.5 for a 384 x 216 output the hit pass runs on 256 x 144 pixels, the frame's
/// image is 384 x 216, and the presenter's blit fills the 384 x 216 output: every output pixel
/// is the bilinear sample of the frame's image within 3/255 in every channel (the cavity
/// scene, shaded).
#[test]
fn at_scale_one_and_a_half_the_hit_pass_runs_on_256_by_144_and_the_blit_fills_384_by_216() {
    let world = cavity_world();
    let output = [384, 216];
    let internal = vxe_render::RenderScale::OneHalf.internal(output);
    assert_eq!(internal, [256, 144]);
    let Some(rig) = rig(&world, internal, output) else {
        return;
    };
    rig.frame.render(
        &rig.device,
        &rig.queue,
        &rig.gpu,
        &cavity_camera(),
        &LightParams::headlamp(),
    );
    let hits = rig.frame.hit_positions();
    assert_eq!([hits.width(), hits.height()], internal, "hit pass size");
    assert_eq!(rig.frame.internal_size(), internal);
    assert_eq!(rig.frame.output_size(), output);
    let src = readback::read_rgba8(&rig.device, &rig.queue, rig.frame.target());
    assert_eq!(
        src.len(),
        (384 * 216 * 4) as usize,
        "the frame's image size"
    );
    let out = blit(&rig, output);
    assert_eq!(out.len(), (384 * 216 * 4) as usize);
    let mut worst = 0.0f32;
    for y in 0..output[1] {
        for x in 0..output[0] {
            for k in 0..3 {
                let want = bilinear(&src, output, output, x, y, k);
                let got = f32::from(out[((y * output[0] + x) * 4) as usize + k]);
                worst = worst.max((got - want).abs());
            }
        }
    }
    let lit = out.chunks(4).filter(|p| p[..3] != [0, 0, 0]).count();
    println!("blit: largest difference from bilinear {worst:.2}/255, {lit} lit pixels");
    assert!(worst <= 3.0, "{worst}/255");
    assert!(lit > (384 * 216) / 2, "{lit} lit pixels");
}

/// The internal and output sizes of the upscaler's tests: scale 1.5.
const INTERNAL: [u32; 2] = [256, 144];
const OUTPUT: [u32; 2] = [384, 216];

/// Frames the upscaler's still-camera tests accumulate.
const SETTLE: u32 = 32;

/// The level the fly-through generates with `--level --seed 1`, its ore emissive (the
/// fly-through's default), and its spawn pose.
fn level() -> (World, vxe_render::Camera) {
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
fn level_rig(
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
fn frame_of(device: &wgpu::Device, internal: [u32; 2], output: [u32; 2], jitter: bool) -> Frame {
    let mut frame = Frame::with_sizes(device, internal, output);
    frame.set_jitter(device, jitter);
    frame
}

/// The presenter's bilinear blit of `source` onto a fresh `rgba8unorm` texture of `size`.
fn blit_texture(
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
fn mad(a: &[u8], b: &[u8]) -> [f64; 3] {
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

/// The still camera's distances (mean of r, g, b, in 1/255) from the native render and from
/// the 2 x 2 supersampled one: of the upscaled image, of the bilinear blit, and of native.
struct StillReading {
    up_native: f64,
    bilinear_native: f64,
    up_ssaa: f64,
    bilinear_ssaa: f64,
    native_ssaa: f64,
}

/// On the level from its spawn, a still camera for 32 frames at the Plan 0024 defaults: the
/// upscaled image (256 x 144 internal, 384 x 216 output, jittered), a native 384 x 216 render
/// (jitter off), the presenter's bilinear blit of a 256 x 144 render (jitter off) and a 2 x 2
/// supersampled native render (768 x 432, jitter off, box-filtered to 384 x 216), each after
/// the same 32 frames; `None` without the software adapter.
fn still_camera_reading(upscaler: Upscaler) -> Option<StillReading> {
    let (world, camera) = level();
    let (device, queue, gpu) = level_rig(&world, &camera)?;
    let light = LightParams::headlamp();
    let big = OUTPUT.map(|n| 2 * n);
    let mut native = frame_of(&device, OUTPUT, OUTPUT, false);
    let mut up = frame_of(&device, INTERNAL, OUTPUT, true);
    up.set_upscaler(upscaler);
    let mut low = frame_of(&device, INTERNAL, INTERNAL, false);
    let mut ssaa = frame_of(&device, big, big, false);
    assert!(up.upscaling() && !native.upscaling() && !low.upscaling());
    for _ in 0..SETTLE {
        for f in [&mut native, &mut up, &mut low, &mut ssaa] {
            f.step(&device, &queue, &gpu, &camera, &light);
        }
    }
    let reference = readback::read_rgba8(&device, &queue, native.target());
    let upscaled = readback::read_rgba8(&device, &queue, up.target());
    let bilinear = blit_texture(&device, &queue, low.target(), OUTPUT);
    let truth = downsample_2x2(
        &readback::read_rgba8(&device, &queue, ssaa.target()),
        OUTPUT,
    );
    let lit = reference.chunks(4).filter(|p| p[..3] != [0, 0, 0]).count();
    assert!(
        lit > (OUTPUT[0] * OUTPUT[1] / 4) as usize,
        "{lit} lit pixels"
    );
    let mean = |m: [f64; 3]| m.iter().sum::<f64>() / 3.0;
    let r = StillReading {
        up_native: mean(mad(&upscaled, &reference)),
        bilinear_native: mean(mad(&bilinear, &reference)),
        up_ssaa: mean(mad(&upscaled, &truth)),
        bilinear_ssaa: mean(mad(&bilinear, &truth)),
        native_ssaa: mean(mad(&reference, &truth)),
    };
    println!(
        "level, {SETTLE} frames, mean absolute difference: from native upscaled {:.3}, bilinear {:.3}, ratio {:.3}; from 2 x 2 supersampled upscaled {:.3}, bilinear {:.3}, ratio {:.3}, native {:.3}; {lit} lit native pixels",
        r.up_native,
        r.bilinear_native,
        r.up_native / r.bilinear_native,
        r.up_ssaa,
        r.bilinear_ssaa,
        r.up_ssaa / r.bilinear_ssaa,
        r.native_ssaa
    );
    Some(r)
}

/// The still camera comes nearer the truth than a bilinear blit: the upscaled image's mean
/// absolute difference from the 2 x 2 supersampled native render is lower than the bilinear
/// blit's (a property, not a threshold). The distances from plain native are a reading.
#[test]
fn a_still_camera_upscales_nearer_the_supersampled_render_than_the_bilinear_blit() {
    let Some(r) = still_camera_reading(Upscaler::default()) else {
        return;
    };
    assert!(
        r.up_ssaa < r.bilinear_ssaa,
        "upscaled {:.3} not below bilinear {:.3}",
        r.up_ssaa,
        r.bilinear_ssaa
    );
}

/// The pixel rectangle (`col0, row0, col1, row1`) the voxels `min ..= max` dilated by one
/// voxel project to from `camera` in an `OUTPUT`-sized image.
fn box_projection(camera: &vxe_render::Camera, (min, max): (VoxelCoord, VoxelCoord)) -> [f32; 4] {
    let (fwd, right, up) = camera.basis();
    let tan = (camera.vfov * 0.5).tan();
    let [w, h] = OUTPUT.map(|v| v as f32);
    let aspect = w / h;
    let mut r = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for c in 0..8 {
        let corner = vxe_render::glam::Vec3::from(
            [0, 1, 2].map(|k| if c >> k & 1 == 0 { min.0[k] - 1 } else { max.0[k] + 2 } as f32),
        );
        let d = corner - camera.position;
        let z = d.dot(fwd).max(1e-3);
        let col = (d.dot(right) / z / (tan * aspect) + 1.0) * 0.5 * w;
        let row = (1.0 - d.dot(up) / z / tan) * 0.5 * h;
        r = [r[0].min(col), r[1].min(row), r[2].max(col), r[3].max(row)];
    }
    r
}

/// On the level, after 32 still frames, a radius-6 carve where the view axis meets rock, its
/// box given to the frames: on the first frame after it every output pixel whose nearest
/// internal sample hit inside the carve's `aabb` dilated by one voxel blends at alpha 1 (no
/// ghost: the edit box rejects the history), and there are such pixels. A reading beside it:
/// inside the carve's projected box, the upscaled frame's difference from a native 384 x 216
/// render that took the same frames and the same carve.
#[test]
fn a_carve_in_view_leaves_no_ghost_in_its_box() {
    let (world, camera) = level();
    let Some((device, queue, mut gpu)) = level_rig(&world, &camera) else {
        return;
    };
    let light = LightParams::headlamp();
    let mut native = frame_of(&device, OUTPUT, OUTPUT, false);
    let mut up = frame_of(&device, INTERNAL, OUTPUT, true);
    for _ in 0..SETTLE {
        for f in [&mut native, &mut up] {
            f.step(&device, &queue, &gpu, &camera, &light);
        }
    }
    let before = readback::read_rgba8(&device, &queue, up.target());
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
    gpu.sync_now(&world, &device, &queue).unwrap();
    for f in [&mut native, &mut up] {
        f.invalidate(aabb.0.0, aabb.1.0);
        f.step(&device, &queue, &gpu, &camera, &light);
    }
    let reference = readback::read_rgba8(&device, &queue, native.target());
    let upscaled = readback::read_rgba8(&device, &queue, up.target());
    let pos = readback::read_rgba32f(&device, &queue, up.hit_positions());
    let alpha = readback::read_rgba16f(&device, &queue, up.upscale_history().unwrap());
    let j = vxe_render::jitter_at(up.frame_index());
    let (mut in_box, mut in_box_blended) = (0usize, 0usize);
    for y in 0..OUTPUT[1] {
        for x in 0..OUTPUT[0] {
            let p = pos[nearest_sample(x, y, j)];
            let dilated = p[3] < 1e30
                && (0..3)
                    .all(|k| p[k] >= (aabb.0.0[k] - 1) as f32 && p[k] <= (aabb.1.0[k] + 2) as f32);
            if dilated {
                in_box += 1;
                in_box_blended += usize::from(alpha[(y * OUTPUT[0] + x) as usize][3] != 1.0);
            }
        }
    }
    println!(
        "carve: {in_box} output pixels hit inside the dilated box, {in_box_blended} of them blend history"
    );
    assert!(in_box > 0, "no output pixel hits inside the box");
    assert_eq!(in_box_blended, 0, "pixels in the box blend history");
    let rect = box_projection(&camera, aabb);
    let (mut inside, mut over, mut worst, mut sum, mut moved) = (0usize, 0usize, 0u8, 0u64, 0usize);
    for y in 0..OUTPUT[1] {
        for x in 0..OUTPUT[0] {
            let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
            if cx < rect[0] || cx > rect[2] || cy < rect[1] || cy > rect[3] {
                continue;
            }
            let i = ((y * OUTPUT[0] + x) * 4) as usize;
            let d = (0..3)
                .map(|k| upscaled[i + k].abs_diff(reference[i + k]))
                .max()
                .unwrap();
            let m = (0..3)
                .map(|k| upscaled[i + k].abs_diff(before[i + k]))
                .max()
                .unwrap();
            inside += 1;
            over += usize::from(d > 8);
            worst = worst.max(d);
            sum += u64::from(d);
            moved += usize::from(m > 8);
        }
    }
    println!(
        "carve at t = {:.1} voxels, projected box {rect:.1?}: {inside} pixels, largest difference from native {worst}/255, mean {:.2}/255, {over} over 8/255; {moved} changed by more than 8/255 from the frame before",
        hit.t as f64 / FIXED_ONE as f64,
        sum as f64 / inside.max(1) as f64
    );
    assert!(inside > 0 && moved > 0, "the carve shows in its box");
}

/// The internal pixel whose sample in a frame of jitter `j` lies nearest output pixel
/// `(x, y)`'s centre, as `upscale.wgsl` picks it.
fn nearest_sample(x: u32, y: u32, j: [f32; 2]) -> usize {
    let c = [x, y];
    let n = [0, 1].map(|k| {
        let centre = (c[k] as f32 + 0.5) * INTERNAL[k] as f32 / OUTPUT[k] as f32;
        ((centre - j[k]).floor() as i32).clamp(0, INTERNAL[k] as i32 - 1) as usize
    });
    n[1] * INTERNAL[0] as usize + n[0]
}

/// The internal pixel whose sample in a frame of `camera` and jitter `j` lies nearest the
/// projection of `p`, or `None` off the image or behind the camera.
fn reproject(camera: &vxe_render::Camera, j: [f32; 2], p: [f32; 3]) -> Option<usize> {
    let (fwd, right, up) = camera.basis();
    let tan = (camera.vfov * 0.5).tan();
    let [w, h] = INTERNAL.map(|v| v as f32);
    let d = vxe_render::glam::Vec3::from(p) - camera.position;
    let z = d.dot(fwd);
    if z <= 1e-4 {
        return None;
    }
    let u = (d.dot(right) / z / (tan * w / h) + 1.0) * 0.5 * w - j[0];
    let v = (1.0 - d.dot(up) / z / tan) * 0.5 * h - j[1];
    (u >= 0.0 && v >= 0.0 && u < w && v < h).then(|| v as usize * INTERNAL[0] as usize + u as usize)
}

/// The cavity at scale 1.5, jittered, still for 4 frames; then material 3 becomes 4 in a slab
/// of the wall ahead (a `Replace` the frame is not told of) and the camera moves one voxel
/// sideways. On the next frame at least 95 % of the output pixels whose hit surface did not
/// change (the nearest sample's hit reprojected into the frame before lands on a hit of the
/// same material, a normal within 0.99 and 1.5 voxels) blend history (the current colour's
/// weight in the blend above 0 and below 1; the nearness blend varies it per pixel), and every
/// output pixel whose nearest sample hit material 4 takes the current colour (alpha 1).
#[test]
fn a_sideways_step_keeps_history_and_a_material_change_rejects_it() {
    let world = cavity_world();
    let Some(mut rig) = rig(&world, INTERNAL, OUTPUT) else {
        return;
    };
    rig.frame
        .set_resolution(&rig.device, vxe_render::LightingResolution::default());
    rig.frame.set_jitter(&rig.device, true);
    let light = LightParams::headlamp();
    let mut camera = cavity_camera();
    for _ in 0..4 {
        rig.frame
            .step(&rig.device, &rig.queue, &rig.gpu, &camera, &light);
    }
    let read_hits = |rig: &LookRig| {
        (
            readback::read_rgba32f(&rig.device, &rig.queue, rig.frame.hit_positions()),
            readback::read_rgba16u(&rig.device, &rig.queue, rig.frame.hit_records()),
            vxe_render::jitter_at(rig.frame.frame_index()),
        )
    };
    let (pos0, info0, j0) = read_hits(&rig);
    let camera0 = camera;
    world
        .apply(&EditOp::box_(
            Fixed3([30 * FIXED_ONE, 2 * FIXED_ONE, -10 * FIXED_ONE]),
            Fixed3([64 * FIXED_ONE, 12 * FIXED_ONE, 10 * FIXED_ONE]),
            Mode::Replace { from: 3, to: 4 },
        ))
        .unwrap();
    rig.gpu.sync_now(&world, &rig.device, &rig.queue).unwrap();
    let (_, right, _) = camera.basis();
    camera.position += right;
    rig.frame
        .step(&rig.device, &rig.queue, &rig.gpu, &camera, &light);
    let (pos1, info1, j1) = read_hits(&rig);
    assert_ne!(j0, j1, "the jitter moved");
    let alpha: Vec<f32> = readback::read_rgba16f(
        &rig.device,
        &rig.queue,
        rig.frame.upscale_history().expect("the upscaler runs"),
    )
    .iter()
    .map(|t| t[3])
    .collect();
    let (mut same, mut kept, mut changed) = (0, 0, 0);
    for y in 0..OUTPUT[1] {
        for x in 0..OUTPUT[0] {
            let o = (y * OUTPUT[0] + x) as usize;
            let i = nearest_sample(x, y, j1);
            if info1[i][3] & 1 == 0 {
                continue;
            }
            if info1[i][2] == 4 {
                changed += 1;
                assert_eq!(alpha[o], 1.0, "output pixel ({x}, {y}) changed material");
                continue;
            }
            let p = [pos1[i][0], pos1[i][1], pos1[i][2]];
            let Some(k) = reproject(&camera0, j0, p) else {
                continue;
            };
            let n = |info: &[u16; 4]| octa16(info[0], info[1]);
            let (n0, n1) = (n(&info0[k]), n(&info1[i]));
            let dist = (0..3)
                .map(|c| (pos0[k][c] - p[c]).powi(2))
                .sum::<f32>()
                .sqrt();
            let unchanged = info0[k][3] & 1 == 1
                && info0[k][2] == info1[i][2]
                && (0..3).map(|c| n0[c] * n1[c]).sum::<f32>() >= 0.99
                && dist <= 1.5;
            if unchanged {
                same += 1;
                kept += usize::from(alpha[o] > 0.0 && alpha[o] < 0.999);
            }
        }
    }
    let share = kept as f64 / same as f64;
    println!(
        "sideways step: {kept} of {same} unchanged output pixels blend history ({:.2} %), {changed} changed pixels take alpha 1",
        share * 100.0
    );
    assert!(changed > 0 && same > 0);
    assert!(share >= 0.95, "{:.2} %", share * 100.0);
}

/// The unit normal a hit record's snorm16 octahedral pair encodes.
fn octa16(u: u16, v: u16) -> [f32; 3] {
    let p = [u as i16 as f32 / 32767.0, v as i16 as f32 / 32767.0];
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

/// `img`, `2 w` x `2 h`, box-filtered to `w` x `h`.
fn downsample_2x2(img: &[u8], [w, h]: [u32; 2]) -> Vec<u8> {
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

/// The still-camera reading of the level under each lighting mode: the mean absolute
/// difference (mean of r, g, b, in 1/255) of the upscaled image and of the bilinear blit
/// from the native render, and of those two, the native render and the jittered render at
/// scale 1 from a 2 x 2 supersampled native render (768 x 432, box-filtered). A reading, not
/// a check: run with `--ignored`.
#[test]
#[ignore = "a diagnostic reading of the upscaler under each lighting mode; run by hand with --ignored"]
fn the_upscalers_distance_from_native_under_each_lighting_mode() {
    let (world, camera) = level();
    let Some((device, queue, gpu)) = level_rig(&world, &camera) else {
        return;
    };
    let light = LightParams::headlamp();
    let mean = |m: [f64; 3]| m.iter().sum::<f64>() / 3.0;
    for (label, candidates, filter, resolution) in [
        ("defaults", 8, true, vxe_render::LightingResolution::Half),
        ("no emissive", 0, true, vxe_render::LightingResolution::Half),
        (
            "full-res lighting",
            8,
            true,
            vxe_render::LightingResolution::Full,
        ),
        ("filter off", 8, false, vxe_render::LightingResolution::Half),
        (
            "full-res lighting, filter off",
            8,
            false,
            vxe_render::LightingResolution::Full,
        ),
    ] {
        let render = |internal: [u32; 2], output: [u32; 2], jitter: bool| {
            let mut f = frame_of(&device, internal, output, jitter);
            f.set_resolution(&device, resolution);
            f.set_lighting(vxe_render::Lighting {
                candidates,
                filter,
                ..vxe_render::Lighting::default()
            });
            for _ in 0..SETTLE {
                f.step(&device, &queue, &gpu, &camera, &light);
            }
            f
        };
        let native = readback::read_rgba8(&device, &queue, render(OUTPUT, OUTPUT, false).target());
        let up = readback::read_rgba8(&device, &queue, render(INTERNAL, OUTPUT, true).target());
        let low = render(INTERNAL, INTERNAL, false);
        let bilinear = blit_texture(&device, &queue, low.target(), OUTPUT);
        let taa = readback::read_rgba8(&device, &queue, render(OUTPUT, OUTPUT, true).target());
        let big = OUTPUT.map(|n| 2 * n);
        let ssaa = downsample_2x2(
            &readback::read_rgba8(&device, &queue, render(big, big, false).target()),
            OUTPUT,
        );
        let (u, b) = (mean(mad(&up, &native)), mean(mad(&bilinear, &native)));
        println!(
            "{label}: from native: upscaled {u:.3}, bilinear {b:.3}, ratio {:.3}; from 2x2 supersampled: upscaled {:.3}, bilinear {:.3}, native {:.3}, jittered at scale 1 {:.3}",
            u / b,
            mean(mad(&up, &ssaa)),
            mean(mad(&bilinear, &ssaa)),
            mean(mad(&native, &ssaa)),
            mean(mad(&taa, &ssaa)),
        );
    }
}

/// The dark cave's golden pose (the carved golden scene, ore emissive, voxel (264, 120, 376),
/// 15 degrees of yaw) with the lamp off and the filter on at the Plan 0024 defaults, at a
/// 320 x 180 output and scale 1.5 (213 x 120 internal), jittered or not: settled over 64
/// still frames, then 32 more upscaled frames read back. Returns the mean over hit pixels
/// (output pixels whose nearest internal sample hit) and the 31 consecutive pairs of the
/// largest channel difference, and the share of (pixel, pair) values above 4/255.
fn still_vein_flicker(jitter: bool, upscaler: Upscaler) -> Option<(f64, f64)> {
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
    frame.set_upscaler(upscaler);
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
            let j = if jitter {
                vxe_render::jitter_at(frame.frame_index())
            } else {
                [0.0; 2]
            };
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

/// Jitter adds little flicker: on the dark cave's still rig at scale 1.5, the upscaled frames'
/// mean step with jitter on is at most 0.40/255 above jitter off's, and its share of values
/// over 4/255 at most 1.15 percentage points above it (Plan 0024 Phase 13's gain, from the
/// stillest image the owner judged to its default).
#[test]
fn jitter_adds_little_flicker_on_a_still_vein() {
    let Some((off_mean, off_over)) = still_vein_flicker(false, Upscaler::default()) else {
        return;
    };
    let (on_mean, on_over) = still_vein_flicker(true, Upscaler::default()).unwrap();
    println!(
        "still vein at scale 1.5: jitter off mean {off_mean:.4}/255, {:.4} % over 4/255; jitter on mean {on_mean:.4}/255, {:.4} % over 4/255",
        off_over * 100.0,
        on_over * 100.0
    );
    assert!(
        on_mean <= off_mean + 0.40,
        "mean {on_mean:.4} against {off_mean:.4}"
    );
    assert!(
        on_over * 100.0 <= off_over * 100.0 + 1.15,
        "{:.4} % against {:.4} %",
        on_over * 100.0,
        off_over * 100.0
    );
}

/// A reading, no bar: on the level from its spawn at scale 1 (384 x 216), lighting at full
/// resolution so the temporal pass's ages align with the hit buffer, a still camera for 16
/// frames with the jitter on and off; the share of hit pixels whose temporal age is below 15,
/// the age of a history unbroken since the first frame (which reads 0).
#[test]
fn temporal_age_under_the_jitter() {
    let (world, camera) = level();
    let Some((device, queue, gpu)) = level_rig(&world, &camera) else {
        return;
    };
    let light = LightParams::headlamp();
    for jitter in [false, true] {
        let mut f = frame_of(&device, OUTPUT, OUTPUT, jitter);
        f.set_resolution(&device, vxe_render::LightingResolution::Full);
        for _ in 0..16 {
            f.step(&device, &queue, &gpu, &camera, &light);
        }
        let rec = readback::read_rgba16u(&device, &queue, f.hit_records());
        let age = readback::read_rgba16f(&device, &queue, f.moments());
        let hits = rec.iter().filter(|r| r[3] & 1 == 1).count();
        let young = rec
            .iter()
            .zip(&age)
            .filter(|(r, a)| r[3] & 1 == 1 && a[2] < 15.0)
            .count();
        println!(
            "jitter {}: {young} of {hits} hit pixels below age 15 after 16 frames ({:.3} %)",
            if jitter { "on" } else { "off" },
            young as f64 * 100.0 / hits as f64
        );
        if !jitter {
            assert_eq!(young, 0, "jitter off ages every hit");
        }
    }
}

/// `frame_bytes` at scale 1.5 and 1080p output is Plan 0024's frame with its targets at the
/// internal 1280 x 720, its `rgba8` target grown to the output size, plus two output-size
/// `rgba16float` histories, and no more.
#[test]
fn frame_bytes_add_the_histories_and_the_output_target() {
    let Some((device, _queue)) = software_device(64 << 20) else {
        return;
    };
    let output = [1920u32, 1080];
    let internal = vxe_render::RenderScale::OneHalf.internal(output);
    let plain = Frame::with_sizes(&device, internal, internal).frame_bytes();
    let up = Frame::with_sizes(&device, internal, output).frame_bytes();
    let px = |[w, h]: [u32; 2]| u64::from(w) * u64::from(h);
    let histories = 2 * 8 * px(output);
    let grown = 4 * (px(output) - px(internal));
    println!(
        "frame_bytes at 1080p output: scale 1.5 {up} B, its targets at 1280x720 without the upscaler {plain} B, histories {histories} B, rgba8 growth {grown} B; scale 1 {} B",
        Frame::with_sizes(&device, output, output).frame_bytes()
    );
    assert_eq!(up, plain + histories + grown);
}

// ---- The sharpness rig: a wall of one-voxel stripes facing the camera ----

/// The stripe wall's face: the plane x = `WALL_X`, the wall two voxels thick behind it.
const WALL_X: i64 = 160;
/// The stripes' materials: albedo 0.8 and 0.1, alternating along y one voxel at a time.
const BRIGHT: u16 = 1;
const DARK: u16 = 2;
/// Arm's length (0.7 m at 10 cm voxels) and the far distance, in voxels from the wall's face.
const NEAR: f32 = 7.0;
const FAR: f32 = 88.0;

/// Air over chunks `[0, 3)^3` with the stripe wall over y and z `[0, 192)`: material
/// `BRIGHT` on even y, `DARK` on odd.
fn stripe_world() -> World {
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
const PHASES: u32 = 5;

/// `dist` voxels in front of the wall, looking +x at it (the camera's right is -y, so the
/// stripes run vertically on screen), moved `(phase + 0.5) / PHASES` output pixels along its
/// right.
fn stripe_camera(dist: f32, phase: u32) -> vxe_render::Camera {
    let shift = (phase as f32 + 0.5) / PHASES as f32 / px_per_voxel(dist);
    camera_looking_x([WALL_X as f32 - dist, 96.3 - shift, 96.4])
}

/// Output pixels one voxel of the wall spans at `dist` voxels.
fn px_per_voxel(dist: f32) -> f32 {
    OUTPUT[1] as f32 / (2.0 * (35f32.to_radians()).tan()) / dist
}

/// The exposure that puts the bright stripe under the lamp's centre near the tone map's
/// knee at `dist`: the lamp's radiance falls as `1 / dist^2`.
fn stripe_exposure(dist: f32) -> f32 {
    dist * dist / 1200.0
}

/// The sharpness readings of one image: edge spread in output pixels (arm's length only),
/// gradient energy and Phase 4's rim gradient, over the image's middle half.
#[derive(Clone, Copy, Debug)]
struct Sharpness {
    es: f64,
    ge: f64,
    rim: f64,
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
fn luma(img: &[u8], x: usize, y: usize) -> f64 {
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
fn sharpness(img: &[u8], camera: &vxe_render::Camera, dist: f32) -> Sharpness {
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
struct StripeRig {
    device: wgpu::Device,
    queue: wgpu::Queue,
    gpu: GpuWorld,
}

fn stripe_rig() -> Option<StripeRig> {
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
enum Image {
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

/// The settings of one stripe reading: the distance, the upscaler's tuning, the frames
/// panned after the 32 still ones (0: still), and the sub-pixel phase.
#[derive(Clone, Copy)]
struct Shot {
    dist: f32,
    upscaler: Upscaler,
    pan: u32,
    phase: u32,
}

/// Output pixels a panning camera moves per frame.
const PAN_PX: f32 = 0.37;

/// `image` of the stripe wall under the headlamp at the Plan 0024 defaults (the exposure
/// [`stripe_exposure`]) after 32 still frames and `shot.pan` frames translating along the
/// camera's right at [`PAN_PX`] output pixels a frame, read at sharpening 0 and 0.5 (the
/// same for N, R and B), with the last frame's camera.
fn stripe_image(rig: &StripeRig, image: Image, shot: Shot) -> ([Vec<u8>; 2], vxe_render::Camera) {
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
    f.set_upscaler(shot.upscaler);
    f.set_sharpness(queue, 0.5);
    let light = LightParams::headlamp();
    let mut camera = stripe_camera(shot.dist, shot.phase);
    let (_, right, _) = camera.basis();
    let step = right * (PAN_PX / px_per_voxel(shot.dist));
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
fn stripe_sharpness(rig: &StripeRig, image: Image, shot: Shot) -> [Sharpness; 2] {
    let mut sum = [Sharpness {
        es: 0.0,
        ge: 0.0,
        rim: 0.0,
    }; 2];
    for phase in 0..PHASES {
        let (imgs, camera) = stripe_image(rig, image, Shot { phase, ..shot });
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
const LANCZOS_PAN_U1: (f64, f64) = (2.020, 0.00357);
const LANCZOS_PAN_U15: (f64, f64) = (2.404, 0.00281);

/// `a <= b`, false when either is NaN.
fn at_most(a: f64, b: f64) -> bool {
    a <= b
}

/// `a >= b`, false when either is NaN.
fn at_least(a: f64, b: f64) -> bool {
    a >= b
}

/// Every image's sharpness on the stripe wall at both distances, still and panning at arm's
/// length, under `upscaler`, printed with Phase 6's bars checked; the return value lists the
/// failures. With `full` the far-distance pans and Plan 0024's frame panning are read too
/// (readings, no bar).
fn stripe_readings(rig: &StripeRig, upscaler: Upscaler, label: &str, full: bool) -> Vec<String> {
    let mut failures = Vec::new();
    for dist in [NEAR, FAR] {
        let still = Shot {
            dist,
            upscaler,
            pan: 0,
            phase: 0,
        };
        println!(
            "[{label}] stripes at {dist} voxels ({:.3} px per voxel):",
            px_per_voxel(dist)
        );
        let read = |image| stripe_sharpness(rig, image, still);
        let [n, _] = read(Image::N);
        let [r, _] = read(Image::R);
        let [b, _] = read(Image::B);
        let u1 = read(Image::U1);
        let u15 = read(Image::U15);
        println!("  N    {n}");
        println!("  R    {r}");
        println!("  B    {b}");
        println!("  U1   sharpen 0: {}; 0.5: {}", u1[0], u1[1]);
        println!("  U1.5 sharpen 0: {}; 0.5: {}", u15[0], u15[1]);
        let near = dist == NEAR;
        // Done-when 1: edges no wider than anti-aliasing over one pixel makes them.
        if near && !at_most(u1[0].es, n.es + 0.53) {
            failures.push(format!(
                "{dist}: U1 ES {:.3} over N's {:.3} + 0.53",
                u1[0].es, n.es
            ));
        }
        if !at_least(u1[0].ge, 0.85 * r.ge) {
            failures.push(format!(
                "{dist}: U1 GE {:.5} under 0.85 of R's {:.5}",
                u1[0].ge, r.ge
            ));
        }
        // Done-when 2: the midpoint between the bilinear blit and the reference.
        let ge_bar = (1.0 + b.ge / r.ge) / 2.0;
        if !at_least(u15[0].ge, ge_bar * r.ge) {
            failures.push(format!(
                "{dist}: U1.5 GE {:.5} under {ge_bar:.3} of R's {:.5}",
                u15[0].ge, r.ge
            ));
        }
        let es_bar = (b.es + r.es) / 2.0;
        if near && !at_most(u15[0].es, es_bar) {
            failures.push(format!("{dist}: U1.5 ES {:.3} over {es_bar:.3}", u15[0].es));
        }
        println!(
            "  bars: U1 ES <= {:.3}, U1 GE >= {:.5}, U1.5 GE >= {:.5} ({ge_bar:.3} of R), U1.5 ES <= {es_bar:.3}",
            n.es + 0.53,
            0.85 * r.ge,
            ge_bar * r.ge
        );
        if !near && !full {
            continue;
        }
        // Done-when 3, at arm's length: the last panning frame's edge spread at least 0.3 px
        // below the first upscaler's, its gradient energy no lower.
        let pan = Shot { pan: 64, ..still };
        for (name, image, (es, ge)) in [
            ("U1", Image::U1, LANCZOS_PAN_U1),
            ("U1.5", Image::U15, LANCZOS_PAN_U15),
        ] {
            let p = stripe_sharpness(rig, image, pan);
            println!(
                "  {name} panning 64 frames: sharpen 0: {}; 0.5: {}",
                p[0], p[1]
            );
            if near && !at_most(p[0].es, es - 0.3) {
                failures.push(format!(
                    "{dist}: {name} panning ES {:.3} over {:.3}",
                    p[0].es,
                    es - 0.3
                ));
            }
            if near && !at_least(p[0].ge, ge) {
                failures.push(format!(
                    "{dist}: {name} panning GE {:.5} under {ge:.5}",
                    p[0].ge
                ));
            }
        }
        if full {
            let [np, _] = stripe_sharpness(rig, Image::N, pan);
            println!("  N panning 64 frames: {np}");
        }
    }
    for f in &failures {
        println!("[{label}] over a bar: {f}");
    }
    failures
}

/// The upscaler is as sharp as Plan 0024's frame allows, on the stripe wall with the headlamp
/// (sharpening 0; edge spread at arm's length, gradient energy at arm's length and at 88
/// voxels): at scale 1 the edge spread is at most native's plus 0.53 px and the gradient
/// energy at least 0.85 of the 2 x 2 supersampled render's; at scale 1.5 both lie on the
/// reference's side of the midpoint between the bilinear blit and the reference; panning
/// along the wall at 0.37 output pixels a frame for 64 frames, the last frame's edge spread is
/// at least 0.3 px below the first upscaler's and its gradient energy no lower, at both
/// scales.
#[test]
fn the_upscaler_keeps_stripe_edges_sharp_still_and_panning() {
    let Some(rig) = stripe_rig() else {
        return;
    };
    let failures = stripe_readings(&rig, Upscaler::default(), "default", false);
    assert!(failures.is_empty(), "{failures:?}");
}

/// A reading, no bar: the stripe readings in full (the far pans too) under the default
/// upscaler, under the first one ([`Upscaler::lanczos`]), and the first one's blur
/// decomposed: its current colour from the nearest sample alone, then its history fetched by
/// nearest texel, each switched alone. Run with `--ignored`.
#[test]
#[ignore = "a diagnostic reading of the upscaler's sharpness; run by hand with --ignored"]
fn the_upscalers_sharpness_readings() {
    use vxe_render::marcher::{HistoryFetch, Reconstruction};
    let Some(rig) = stripe_rig() else {
        return;
    };
    let first = Upscaler::lanczos();
    for (label, u) in [
        ("default", Upscaler::default()),
        (
            "default, nearness unnormalised, 0.5 in motion",
            Upscaler {
                blend: vxe_render::marcher::Blend::Nearness,
                alpha_moving: 0.5,
                ..Upscaler::default()
            },
        ),
        ("first upscaler", first),
        (
            "first, nearest sample",
            Upscaler {
                reconstruction: Reconstruction::Nearest,
                ..first
            },
        ),
        (
            "first, nearest history texel",
            Upscaler {
                history_fetch: HistoryFetch::Nearest,
                ..first
            },
        ),
    ] {
        stripe_readings(&rig, u, label, true);
    }
}

// ---- Light changes under a still camera: the headlamp's sweep and step ----

/// The pipelines the lamp bars compare: Plan 0024's frame (scale 1, jitter off) and the
/// upscaler at scale 1 and 1.5, jittered, all at a 384 x 216 output.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Pipe {
    Reference,
    U1,
    U15,
}

impl Pipe {
    fn frame(self, device: &wgpu::Device, upscaler: Upscaler, lamp: bool) -> Frame {
        let mut f = match self {
            Pipe::Reference => frame_of(device, OUTPUT, OUTPUT, false),
            Pipe::U1 => frame_of(device, OUTPUT, OUTPUT, true),
            Pipe::U15 => frame_of(device, INTERNAL, OUTPUT, true),
        };
        f.set_upscaler(upscaler);
        f.set_lighting(vxe_render::Lighting {
            lamp,
            ..vxe_render::Lighting::default()
        });
        f
    }
}

/// The lamp's turn a frame during the sweep, in radians: about 2 output pixels a frame near
/// the image's centre (384 / 2 / tan(35 degrees) / (16 / 9) = 154 pixels a radian).
const SWEEP_RATE: f32 = 0.013;

/// Per-pixel luminance of an `OUTPUT` image.
fn lumas(img: &[u8]) -> Vec<f64> {
    let [w, h] = OUTPUT.map(|n| n as usize);
    (0..w * h).map(|i| luma(img, i % w, i / w)).collect()
}

/// The luminance-weighted mean column of `max(0, lit - off)`, and that contribution.
fn lamp_centroid(lit: &[f64], off: &[f64]) -> (f64, Vec<f64>) {
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
fn luma_mad(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<f64>() / a.len() as f64
}

/// The frames, from the first after a change, until `series` (each frame's mean absolute
/// difference from the settled image) falls below 10 % of `step` (the change's size);
/// `None` if it does not within the series.
fn settle_frames(series: &[f64], step: f64) -> Option<usize> {
    series.iter().position(|&d| d < 0.1 * step).map(|i| i + 1)
}

/// One pipeline's lamp readings.
#[derive(Debug, Default)]
struct LampReading {
    /// The sweep's centroid over its last 16 frames, one per frame.
    centroids: Vec<f64>,
    /// The lamp's contribution over the sweep's last 16 frames.
    contributions: Vec<Vec<f64>>,
    /// Frames for the lamp switched off to settle within 10 % of the lamp's settled
    /// contribution.
    step: Option<usize>,
    /// Frames for an emission step (ore 1.0 to 0.25, lamp off) to settle within 10 %.
    emission: Option<usize>,
    /// The emission step's first and last difference from its settled image, in 1/255.
    emission_series: (f64, f64),
    /// The mean over pixels of a still lamp-lit image's luminance range over one jitter cycle.
    wobble: f64,
}

/// On the level at the spawn pose, a still camera: `pipe`'s lamp sweep (32 frames settled,
/// then the lamp's turn raised by [`SWEEP_RATE`] a frame for 32 frames, its contribution
/// against a settled lamp-off image of the same pipeline) and lamp step (32 frames settled,
/// then the lamp off for 40 frames); with `full` the emission step and the wobble too.
fn lamp_reading(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    gpu: &GpuWorld,
    camera: &vxe_render::Camera,
    pipe: Pipe,
    upscaler: Upscaler,
    full: bool,
) -> LampReading {
    let light = LightParams::headlamp();
    let settled = |lamp: bool, frames: u32| {
        let mut f = pipe.frame(device, upscaler, lamp);
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
    for _ in 0..vxe_render::marcher::JITTER_PHASES {
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
fn ideal_centroids(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    gpu: &GpuWorld,
    camera: &vxe_render::Camera,
) -> Vec<f64> {
    let light = LightParams::headlamp();
    let off_frame = Pipe::Reference.frame(device, Upscaler::default(), false);
    off_frame.render(device, queue, gpu, camera, &light);
    let off = lumas(&readback::read_rgba8(device, queue, off_frame.target()));
    let on = Pipe::Reference.frame(device, Upscaler::default(), true);
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
fn slope(ys: &[f64]) -> f64 {
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

/// Every pipeline's lamp readings under `upscaler`, printed, with Phase 7's bars checked; the
/// return value lists the failures.
fn lamp_readings(upscaler: Upscaler, label: &str, full: bool) -> Option<Vec<String>> {
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
    let reference = lamp_reading(
        &device,
        &queue,
        &gpu,
        &camera,
        Pipe::Reference,
        upscaler,
        full,
    );
    let ref_lag = lag(&reference);
    println!(
        "[{label}] reference: lag {ref_lag:.2} frames, lamp step settles in {:?} frames, emission step {:?} ({:.3?}), wobble {:.3}/255",
        reference.step, reference.emission, reference.emission_series, reference.wobble
    );
    let mut failures = Vec::new();
    for pipe in [Pipe::U1, Pipe::U15] {
        let r = lamp_reading(&device, &queue, &gpu, &camera, pipe, upscaler, full);
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

/// A sweeping headlamp and a lamp switched off reach the upscaled image about as fast as
/// Plan 0024's frame: on the level at the spawn pose with a still camera, at scale 1 and 1.5
/// (jittered, 384 x 216 output), the lamp's centroid lags the ideal (a single frame with no
/// history at each turn) by at most 2 frames more than Plan 0024's frame does over the
/// sweep's last 16 frames, and the lamp step settles within 10 % of the lamp's settled
/// contribution in at most 2 frames more than that frame takes.
#[test]
fn a_sweeping_headlamp_and_a_lamp_step_keep_pace_with_plan_0024s_frame() {
    let Some(failures) = lamp_readings(Upscaler::default(), "default", false) else {
        return;
    };
    assert!(failures.is_empty(), "{failures:?}");
}

/// A reading, no bar: the lamp readings in full (the emission step, the wobble and the trail
/// too) under the default upscaler, with still pixels never clipped, and with the still
/// blend at 1/16, which also reads the still camera, the flicker rig and the stripes. Run
/// with `--ignored`.
#[test]
#[ignore = "a diagnostic reading of light changes and the history weight; run by hand with --ignored"]
fn the_upscalers_lamp_and_history_weight_readings() {
    use vxe_render::marcher::StillClip;
    let base = Upscaler::default();
    let never = Upscaler {
        still_clip: StillClip::Never,
        ..base
    };
    let sixteenth = Upscaler {
        alpha: 0.0625,
        ..base
    };
    if lamp_readings(base, "default", true).is_none() {
        return;
    }
    lamp_readings(never, "still never clipped", true);
    for (label, u) in [("default", base), ("still alpha 1/16", sixteenth)] {
        let r = still_camera_reading(u).unwrap();
        let (m0, o0) = still_vein_flicker(false, u).unwrap();
        let (m1, o1) = still_vein_flicker(true, u).unwrap();
        println!(
            "[{label}] still camera from supersampled {:.3} (bilinear {:.3}); flicker at 1.5 jitter off {m0:.4}/255 {:.4} %, on {m1:.4}/255 {:.4} %",
            r.up_ssaa,
            r.bilinear_ssaa,
            o0 * 100.0,
            o1 * 100.0
        );
    }
    let rig = stripe_rig().unwrap();
    stripe_readings(&rig, sixteenth, "still alpha 1/16", false);
    lamp_readings(sixteenth, "still alpha 1/16", true);
}

/// The mean Rec. 709 luma of an RGBA8 image, in 1/255.
fn mean_luma(img: &[u8]) -> f64 {
    let n = (img.len() / 4) as f64;
    img.chunks(4)
        .map(|p| 0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2]))
        .sum::<f64>()
        / n
}

/// The upscaler follows a moving exposure (ADR-0033): on the golden scene's vein pose (the
/// carved cave, the ore emissive, the lamp off, so the ore's light is the meter's), at scale 1.5
/// (213 x 120 for 320 x 180), jittered, under `AutoExposure::eye()`, a still camera settles 64
/// frames, then every ore's emission scale steps from 1 to 1/16. The upscaled output's mean luma
/// at each of the 30 frames after the step lies within 2/255 of the same run with the upscale
/// pass's history off (the current colour weighing 1 still and moving); the exposure moves by
/// more than 1/64 EV a frame over the first frames, where the clip applies.
#[test]
fn the_upscaler_follows_an_exposure_step() {
    let source = scene_source();
    let world = World::generate(
        source.extent,
        &source,
        &flythrough::with_emissive_ore(&flythrough::cave_materials()),
    );
    world.apply(&scene_carve(&world)).unwrap();
    let output = [WIDTH, HEIGHT];
    let internal = vxe_render::RenderScale::OneHalf.internal(output);
    let full = world.stats().full_bricks as u64;
    let Some((device, queue)) = software_device(((full + 4096) * 512).max(64 << 20)) else {
        return;
    };
    let mut gpu = GpuWorld::with_config(&device, MirrorConfig::new(Window::Fixed(world.extent())));
    gpu.set_focus(VoxelCoord([264, 120, 376]));
    for _ in 0..4 {
        gpu.sync_now(&world, &device, &queue).unwrap();
    }
    let camera = vxe_render::Camera {
        position: [264.5, 120.5, 376.5].into(),
        yaw: 15f32.to_radians(),
        pitch: 0.0,
        vfov: 70f32.to_radians(),
    };
    let light = LightParams::headlamp();
    let run = |upscaler: Upscaler| {
        let mut frame = frame_of(&device, internal, output, true);
        frame.set_upscaler(upscaler);
        frame.set_lighting(vxe_render::Lighting {
            lamp: false,
            exposure: vxe_render::Exposure::Auto(vxe_render::AutoExposure::eye()),
            ..vxe_render::Lighting::default()
        });
        for _ in 0..64 {
            frame.step(&device, &queue, &gpu, &camera, &light);
        }
        frame.set_emission_scale(&queue, flythrough::cave::ORE, 1.0 / 16.0);
        (0..30)
            .map(|_| {
                frame.step(&device, &queue, &gpu, &camera, &light);
                let state = vxe_render::ExposureState::from_bytes(&readback::read_buffer(
                    &device,
                    &queue,
                    frame.exposure_state(),
                    0,
                    32,
                ));
                (
                    mean_luma(&readback::read_rgba8(&device, &queue, frame.target())),
                    f64::from(state.current).log2(),
                )
            })
            .collect::<Vec<(f64, f64)>>()
    };
    let kept = run(Upscaler::default());
    let off = run(Upscaler {
        alpha: 1.0,
        alpha_moving: 1.0,
        ..Upscaler::default()
    });
    let lumas = |r: &[(f64, f64)]| r.iter().map(|p| p.0).collect::<Vec<f64>>();
    let evs: Vec<f64> = kept.iter().map(|p| p.1).collect();
    println!("log2 e after the step: {evs:.3?}");
    println!(
        "mean luma after the step, history kept: {:.2?}",
        lumas(&kept)
    );
    println!(
        "mean luma after the step, history off:  {:.2?}",
        lumas(&off)
    );
    for (i, (a, b)) in kept.iter().zip(&off).enumerate() {
        assert!(
            (a.0 - b.0).abs() <= 2.0,
            "frame {}: {:.2} against {:.2}",
            i + 1,
            a.0,
            b.0
        );
    }
    assert!(evs[1] - evs[0] > 1.0 / 64.0, "the exposure moves: {evs:?}");
}
