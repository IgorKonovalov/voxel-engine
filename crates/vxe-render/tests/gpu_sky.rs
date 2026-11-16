//! The sky (ADR-0026): the LUTs read back after a frame of an empty world, and the composite's
//! miss through FSR 2's HDR variant. Lavapipe, like every render test.

use std::f32::consts::{FRAC_PI_2, PI};

use vxe_render::sky::{Body, Sky, body_direction};
use vxe_render::{
    Camera, Exposure, Frame, Fsr2Settings, GpuWorld, LightParams, Lighting, UpscalerKind,
    glam::Vec3, wgpu,
};
use vxe_world::*;

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    vxe_render::test_support::any_device(16 << 20, wgpu::Features::empty())
}

/// A mirror of one all-air chunk at the origin: every primary ray misses.
fn empty(device: &wgpu::Device, queue: &wgpu::Queue) -> GpuWorld {
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [1, 1, 1]),
        &FnSource(|_: [i32; 3]| AIR),
        &MaterialTable::new(),
    );
    let mut gpu = GpuWorld::new(device, world.extent());
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    gpu.sync(&world, queue, &mut enc).expect("sync");
    queue.submit([enc.finish()]);
    gpu
}

/// The noon sky of the testbed: the sun 45 degrees up in the south, the moon opposite.
fn noon() -> Sky {
    let sun = body_direction(0.0, 0.0, 45f32.to_radians());
    Sky::new(Body::sun(sun), Body::moon(sun.map(|c| -c)))
}

/// The camera at the origin's chunk, 1 m up, looking south at the horizon.
fn south() -> Camera {
    Camera {
        position: Vec3::new(32.0, 32.0, 10.0),
        yaw: -FRAC_PI_2,
        pitch: 0.0,
        vfov: 60f32.to_radians(),
    }
}

fn frame(device: &wgpu::Device, size: [u32; 2], exposure: f32) -> Frame {
    let mut frame = Frame::new(device, size[0], size[1]);
    frame.set_lighting(Lighting {
        lamp: false,
        exposure: Exposure::Manual(exposure),
        ..Lighting::default()
    });
    frame
}

/// The LUT `i` ([`Frame::sky_luts`]) read back, with its width.
fn lut(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    frame: &Frame,
    i: usize,
) -> (Vec<[f32; 4]>, usize) {
    let t = frame.sky_luts().expect("a sky is set")[i];
    (
        vxe_render::debug::read_rgba16f(device, queue, t),
        t.width() as usize,
    )
}

fn lum(c: [f32; 4]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// With the default atmosphere, the transmittance LUT's texel at altitude 0 looking at the
/// zenith is `exp(-(0.0614, 0.1420, 0.2714))` within 1 % per channel (Rayleigh 8 km, Mie
/// 1.2 km and ozone's 15 km tent of optical depth), and the texel at the top looking up 1.0
/// within 0.1 %.
#[test]
fn the_transmittance_lut_matches_the_closed_form() {
    let Some((device, queue)) = device() else {
        return;
    };
    let gpu = empty(&device, &queue);
    let mut f = frame(&device, [64, 36], 1.0);
    f.set_sky(Some(noon()));
    f.render(&device, &queue, &gpu, &south(), &LightParams::at_eye());
    let (t, w) = lut(&device, &queue, &f, 0);
    let ground_up = t[0];
    let want = [0.9404, 0.8677, 0.7623];
    for k in 0..3 {
        assert!(
            (ground_up[k] / want[k] - 1.0).abs() < 0.01,
            "channel {k}: {:?} against {want:?}",
            ground_up
        );
    }
    let top_up = t[(t.len() / w - 1) * w];
    for k in 0..3 {
        assert!((top_up[k] - 1.0).abs() < 0.001, "the top: {top_up:?}");
    }
}

/// The sky-view LUT's row of a zenith angle, for a camera at the surface (10 m, the LUT's
/// floor): `sky_view_v` of `sky_atmosphere.wgsl` and the texel-centre mapping.
fn sky_view_row(zenith: f32, rows: usize) -> f32 {
    let (b, r) = (6360.0f32, 6360.01f32);
    let beta = ((r * r - b * b).sqrt() / r).acos();
    let zha = PI - beta;
    let v = if zenith < zha {
        0.5 * (1.0 - (1.0 - zenith / zha).max(0.0).sqrt())
    } else {
        0.5 + 0.5 * ((zenith - zha) / beta).clamp(0.0, 1.0).sqrt()
    };
    v * (rows as f32 - 1.0)
}

/// The sky-view LUT bilinear at direction `d`, as `sky_radiance` taps it (the azimuth wraps).
fn sky_view_at(t: &[[f32; 4]], w: usize, d: Vec3) -> [f32; 3] {
    let h = t.len() / w;
    let mut az = d.y.atan2(d.x);
    if az < 0.0 {
        az += 2.0 * PI;
    }
    let x = az / (2.0 * PI) * w as f32 - 0.5;
    let y = sky_view_row(d.z.clamp(-1.0, 1.0).acos(), h);
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let mut out = [0.0; 3];
    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
        let xi = (x0 as i32 + dx).rem_euclid(w as i32) as usize;
        let yi = (y0 as i32 + dy).clamp(0, h as i32 - 1) as usize;
        let wgt = if dx == 1 { fx } else { 1.0 - fx } * if dy == 1 { fy } else { 1.0 - fy };
        for k in 0..3 {
            out[k] += wgt * t[yi * w + xi][k];
        }
    }
    out
}

/// After one frame of an empty world with the sun 45 degrees up, the sky-view texel of the
/// zenith reads blue > green > red, and the texel 5 degrees up opposite the sun is brighter
/// than the zenith's.
#[test]
fn the_noon_sky_is_blue_and_bright_at_the_horizon() {
    let Some((device, queue)) = device() else {
        return;
    };
    let gpu = empty(&device, &queue);
    let mut f = frame(&device, [64, 36], 1.0);
    let sky = noon();
    f.set_sky(Some(sky));
    f.render(&device, &queue, &gpu, &south(), &LightParams::at_eye());
    let (t, w) = lut(&device, &queue, &f, 1);
    let zenith = t[0];
    assert!(
        zenith[2] > zenith[1] && zenith[1] > zenith[0] && zenith[0] > 0.0,
        "the zenith: {zenith:?}"
    );
    let s = sky.sun.direction;
    let away = Vec3::new(-s[0], -s[1], 0.0).normalize();
    let el = 5f32.to_radians();
    let low = sky_view_at(&t, w, away * el.cos() + Vec3::Z * el.sin());
    assert!(
        lum([low[0], low[1], low[2], 0.0]) > lum(zenith),
        "5 degrees up opposite the sun {low:?}, the zenith {zenith:?}"
    );
}

/// Through FSR 2's HDR composite, the top row's centre pixel of a miss reads blue > green >
/// red, and each channel within 1 % of the sky-view radiance for its direction over the view
/// scale (one over the sun's illuminance) times the manual exposure.
#[test]
fn the_hdr_composite_carries_the_sky() {
    let Some((device, queue)) = device() else {
        return;
    };
    let gpu = empty(&device, &queue);
    let (size, ex) = ([320u32, 180u32], 1e-3f32);
    let mut f = frame(&device, size, ex);
    f.set_upscaler_kind(&device, UpscalerKind::Fsr2(Fsr2Settings::default()));
    let sky = noon();
    f.set_sky(Some(sky));
    let cam = south();
    f.render(&device, &queue, &gpu, &cam, &LightParams::at_eye());
    let fsr = f.fsr2().expect("FSR 2 runs");
    let colour = vxe_render::debug::read_float(&device, &queue, fsr.colour_in());
    let cw = fsr.colour_in().width() as usize;
    let px = colour[cw / 2];
    assert!(
        px[2] > px[1] && px[1] > px[0],
        "the top row's centre: {px:?}"
    );
    // The pixel's ray (`pixel_ray`, the jitter left out: under a pixel's width).
    let ch = colour.len() / cw;
    let (fwd, right, up) = cam.basis();
    let hv = (cam.vfov * 0.5).tan();
    let nx = ((cw / 2) as f32 + 0.5) / cw as f32 * 2.0 - 1.0;
    let ny = 0.5 / ch as f32 * 2.0 - 1.0;
    let d = (fwd + right * (nx * hv * cw as f32 / ch as f32) - up * (ny * hv)).normalize();
    let (t, w) = lut(&device, &queue, &f, 1);
    let l = sky_view_at(&t, w, d);
    for k in 0..3 {
        let want = l[k] * sky.sun.illuminance[k] * ex;
        assert!(
            (px[k] / want - 1.0).abs() < 0.01,
            "channel {k}: {px:?} against {want} (LUT {l:?})"
        );
    }
}
