//! The internal resolution, the jitter and the upscaler (ADR-0024), on the software adapter
//! (lavapipe).

mod support;

use support::*;
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
    let full = world.stats().full_bricks as u64;
    let (device, queue) = software_device(full * vxe_render::gpu_world::SLOT_BYTES)?;
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

/// On the level from its spawn, a still camera for 32 frames at the Plan 0024 defaults: the
/// upscaled image (256 x 144 internal, 384 x 216 output, jittered) is nearer a native 384 x
/// 216 render (scale 1, jitter off, the same 32 frames) by the mean absolute difference per
/// channel than the presenter's bilinear blit of a 256 x 144 render (jitter off, the same 32
/// frames) is. The test prints both figures and their ratio.
#[test]
fn a_still_camera_upscales_nearer_the_native_render_than_the_bilinear_blit() {
    let (world, camera) = level();
    let Some((device, queue, gpu)) = level_rig(&world, &camera) else {
        return;
    };
    let light = LightParams::headlamp();
    let mut native = frame_of(&device, OUTPUT, OUTPUT, false);
    let mut up = frame_of(&device, INTERNAL, OUTPUT, true);
    let mut low = frame_of(&device, INTERNAL, INTERNAL, false);
    assert!(up.upscaling() && !native.upscaling() && !low.upscaling());
    for _ in 0..SETTLE {
        for f in [&mut native, &mut up, &mut low] {
            f.step(&device, &queue, &gpu, &camera, &light);
        }
    }
    let reference = readback::read_rgba8(&device, &queue, native.target());
    let upscaled = readback::read_rgba8(&device, &queue, up.target());
    let bilinear = blit_texture(&device, &queue, low.target(), OUTPUT);
    let lit = reference.chunks(4).filter(|p| p[..3] != [0, 0, 0]).count();
    let m_up = mad(&upscaled, &reference);
    let m_bi = mad(&bilinear, &reference);
    let (s_up, s_bi) = (
        m_up.iter().sum::<f64>() / 3.0,
        m_bi.iter().sum::<f64>() / 3.0,
    );
    println!(
        "level, {SETTLE} frames: mean absolute difference from native (r, g, b; mean) upscaled {m_up:.3?} {s_up:.3}, bilinear {m_bi:.3?} {s_bi:.3}, ratio {:.3}; {lit} lit native pixels",
        s_up / s_bi
    );
    assert!(
        lit > (OUTPUT[0] * OUTPUT[1] / 4) as usize,
        "{lit} lit pixels"
    );
    assert!(
        s_up < s_bi,
        "upscaled {s_up:.3} not below bilinear {s_bi:.3}"
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

/// On the level, after 32 still frames, a radius-6 carve where the view axis meets rock: on
/// the frame after it (its box given to both frames), the upscaled image inside the carve's
/// projected box reads within 8/255 of a native 384 x 216 render that took the same frames and
/// the same carve: no ghost of the rock, since the edit box rejects the history.
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
        "carve at t = {:.1} voxels, box {rect:.1?}: {inside} pixels, largest difference from native {worst}/255, mean {:.2}/255, {over} over 8/255; {moved} changed by more than 8/255 from the frame before",
        hit.t as f64 / FIXED_ONE as f64,
        sum as f64 / inside.max(1) as f64
    );
    assert!(inside > 0 && moved > 0, "the carve shows in its box");
    assert!(
        worst <= 8,
        "{over} of {inside} pixels over 8/255, largest {worst}/255"
    );
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
/// same material, a normal within 0.99 and 1.5 voxels) blend history at alpha 0.1, and every
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
                kept += usize::from((alpha[o] - 0.1).abs() < 1e-3);
            }
        }
    }
    let share = kept as f64 / same as f64;
    println!(
        "sideways step: {kept} of {same} unchanged output pixels blend history at 0.1 ({:.2} %), {changed} changed pixels take alpha 1",
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
