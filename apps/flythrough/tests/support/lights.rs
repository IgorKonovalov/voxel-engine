//! The list of analytic lights' rigs (ADR-0044), shared by `tests/gpu_dynamic_lights.rs` and
//! `tests/gpu_fsr2.rs`: a hand-built closed box with no emitter, its frames at manual exposure
//! 1 with the lamp off, and its floor hits. Not the emissive light-sample mirror's
//! (`gpu_lights.rs`).

use flythrough::cave::ROCK;
use flythrough::cave_materials;
use vxe_render::{
    Camera, Exposure, Frame, LightParams, Lighting, LightingResolution, UpscalerKind, wgpu,
};
use vxe_world::{ChunkExtent, EditOp, FIXED_ONE, Fixed3, Mode, UniformSource, World};

use super::bake::{Rig, rig_on};

/// The box's floor: the top face of the voxels at z 1.
pub const FLOOR_Z: f32 = 2.0;

/// The floor's normal.
pub const UP: [f32; 3] = [0.0, 0.0, 1.0];

/// A closed rock box over chunks `[0, 2) x [0, 1) x [0, 1)`, hollow over `[2, 126) x [2, 62) x
/// [2, 50)`: 124 voxels long, its floor at z 2, no emitter.
pub fn light_box() -> World {
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [2, 1, 1]),
        &UniformSource(ROCK),
        &cave_materials(),
    );
    world
        .apply(&EditOp::box_(
            Fixed3([2, 2, 2].map(|v: i64| v * FIXED_ONE)),
            Fixed3([126, 62, 50].map(|v: i64| v * FIXED_ONE)),
            Mode::Carve,
        ))
        .expect("the box is resident");
    world
}

/// The box's lighting: lamp off, no bounce, manual exposure 1.
pub fn box_lighting() -> Lighting {
    Lighting {
        lamp: false,
        lamp_bounce: false,
        exposure: Exposure::Manual(1.0),
        bake_pool_bytes: 16 << 20,
        ..Lighting::default()
    }
}

/// The box under a 192 x 108 frame, lamp off, manual exposure 1, no jitter, full tier.
pub fn box_rig() -> Option<Rig> {
    let mut rig = rig_on(light_box(), [192, 108], None)?;
    rig.frame.set_lighting(box_lighting());
    Some(rig)
}

/// A frame over the box of `internal` pixels for an `output` image under `kind` (none: no
/// upscaler, no jitter), at the full tier with [`box_lighting`].
pub fn box_frame(
    device: &wgpu::Device,
    internal: [u32; 2],
    output: [u32; 2],
    kind: Option<UpscalerKind>,
) -> Frame {
    let mut frame = Frame::with_sizes(device, internal, output);
    frame.set_jitter(device, false);
    if let Some(k) = kind {
        frame.set_upscaler_kind(device, k);
        frame.set_sizes(device, internal, output);
    }
    frame.set_resolution(device, LightingResolution::Full);
    frame.set_lighting(box_lighting());
    frame.set_dt(1.0 / 60.0);
    frame
}

/// Above the floor at x 64, looking down the box's length toward +x.
pub fn box_camera() -> Camera {
    Camera {
        position: [60.0, 32.0, 45.0].into(),
        yaw: 0.0,
        pitch: -1.2,
        vfov: 1.2,
    }
}

/// Near the box's -x end, looking down its length: the floor from x 23 to the far wall.
pub fn box_long_camera() -> Camera {
    Camera {
        position: [20.0, 32.0, 45.0].into(),
        yaw: 0.0,
        pitch: -0.9,
        vfov: 1.2,
    }
}

/// One frame of `frame` over `rig`'s mirror from `camera`, the headlamp's parameters default.
pub fn step_frame(rig: &Rig, frame: &mut Frame, camera: &Camera) {
    frame.step(
        &rig.device,
        &rig.queue,
        &rig.gpu,
        camera,
        &LightParams::headlamp(),
    );
}

/// One frame of the rig's own frame from `camera`.
pub fn step(rig: &mut Rig, camera: &Camera) {
    rig.frame.step(
        &rig.device,
        &rig.queue,
        &rig.gpu,
        camera,
        &LightParams::headlamp(),
    );
}

/// The hits of `frame`'s last frame: (pixel, hit point, on the floor), for each pixel that hit.
pub fn hits_of(rig: &Rig, frame: &Frame) -> Vec<(usize, [f32; 3], bool)> {
    let pos = vxe_render::debug::read_rgba32f(&rig.device, &rig.queue, frame.hit_positions());
    pos.iter()
        .enumerate()
        .filter(|(_, p)| p[3] > 0.0 && p[3] < 1e30)
        .map(|(i, p)| (i, [p[0], p[1], p[2]], (p[2] - FLOOR_Z).abs() < 1e-3))
        .collect()
}

/// The last frame's floor hits: (pixel, hit point) for each pixel whose hit is on the floor.
pub fn floor_hits(rig: &Rig) -> Vec<(usize, [f32; 3])> {
    hits_of(rig, &rig.frame)
        .into_iter()
        .filter(|h| h.2)
        .map(|(i, p, _)| (i, p))
        .collect()
}

/// The floor pixel whose hit is nearest `(x, y)` on the floor.
pub fn nearest(hits: &[(usize, [f32; 3])], x: f32, y: f32) -> (usize, [f32; 3]) {
    *hits
        .iter()
        .min_by(|a, b| {
            let d = |p: [f32; 3]| (p[0] - x).powi(2) + (p[1] - y).powi(2);
            d(a.1).total_cmp(&d(b.1))
        })
        .expect("the floor is in view")
}

/// The distance between two points, in voxels.
pub fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f32>().sqrt()
}
