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
    rig.frame.set_jitter(jitter);
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

/// At scale 1.5 for a 384 x 216 output the hit pass and the composite run on 256 x 144 pixels,
/// and the presenter's blit fills the 384 x 216 output: every output pixel is the bilinear
/// sample of the internal image within 3/255 in every channel (the cavity scene, shaded).
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
    assert_eq!(src.len(), (256 * 144 * 4) as usize, "composite size");
    let out = blit(&rig, output);
    assert_eq!(out.len(), (384 * 216 * 4) as usize);
    let mut worst = 0.0f32;
    for y in 0..output[1] {
        for x in 0..output[0] {
            for k in 0..3 {
                let want = bilinear(&src, internal, output, x, y, k);
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
