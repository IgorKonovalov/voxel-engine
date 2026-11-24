//! The key light's motion bars' small scene (ADR-0079): the forest of seed 1 over 3 x 3 chunks
//! around the spawn's (2 in z), rendered at 320 x 180 through FSR 2 at 1.5 to 480 x 270, the
//! queue stepping as the fly-through's does from the first frame (no drain), each display pixel
//! classed near or far by its hit's distance, and the share of a class's pixels whose luma moved
//! by over 4 levels between two frames.

use super::defects::{grid_of, queue_step};
use super::terms::Rig;
use svet_render::{Camera, Frame};

/// The frame's sizes: FSR 2 at 1.5.
pub const INTERNAL: [u32; 2] = [320, 180];
pub const OUT: [u32; 2] = [480, 270];

/// Metres a voxel.
pub const VOXEL_M: f32 = 0.1;

/// A hit nearer than this many metres is near; one at it or past it is far. At 320 pixels across
/// the 70-degree view a voxel 6 m out spans 2.14 pixels, so every far pixel sits at the far
/// normal's weight 0.996 or over (`far_normal_weight` at `[2, 6]`): the far estimate's pixels.
pub const NEAR_M: f32 = 6.0;

/// The luma change a pixel may make between two frames before it counts.
pub const T: u8 = 4;

/// The classes [`classes`] gives.
pub const SKY: u8 = 0;
pub const NEAR: u8 = 1;
pub const FAR: u8 = 2;

/// The sun of the scene: elevation 40, azimuth 200 (the wobble rig's).
pub const SUN: [f32; 2] = [40.0, 200.0];

/// The lighting the scene runs under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arm {
    /// The fly-through's start: the far reads the record, else the column map's horizon, blended
    /// by the far normal's weight; the approach penumbra near.
    Picks,
    /// The far's ray and the disc penumbra (`key far ray`, `penumbra disc`): the before.
    Before,
    /// The picks under the albedo view: no lighting in the image.
    Albedo,
}

impl Arm {
    pub fn name(self) -> &'static str {
        match self {
            Arm::Picks => "picks",
            Arm::Before => "before (ray, disc)",
            Arm::Albedo => "lighting off (albedo)",
        }
    }
}

/// The forest of seed 1 at `LEAF_FILL` over the chunks within 1 of the spawn's in x and y, the
/// spawn's and the one under it in z, mirrored whole; the spawn's camera at pitch -4 degrees.
pub fn forest() -> Option<(Rig, Camera)> {
    use flythrough::outdoor::{OutdoorSource, Spawn};
    use flythrough::trees::{ForestSource, LEAF_FILL, TreeDensity};
    use svet_world::{ChunkExtent, World};
    let source = ForestSource::new(OutdoorSource::new(1), TreeDensity::Forest, LEAF_FILL);
    let (eye, facing) = source.base().spawn(Spawn::Forest);
    let c = eye.chunk().0;
    let extent = ChunkExtent::new(
        [c[0] - 1, c[1] - 1, c[2] - 1],
        [c[0] + 2, c[1] + 2, c[2] + 1],
    );
    let world = World::generate(extent, &source, &flythrough::outdoor_materials());
    let rig = super::terms::mirrored(world, eye)?;
    let p = eye.0.map(|v| v as f32 + 0.5);
    let d = facing.map(|v| v as f32);
    let cam = flythrough::camera_looking_at(p, [p[0] + d[0], p[1] + d[1], p[2] + d[2]]);
    Some((
        rig,
        Camera {
            pitch: (-4f32).to_radians(),
            ..cam
        },
    ))
}

/// A frame of `arm` on `rig`: FSR 2 at 1.5 (sharpness 0.5), the fly-through's look (the far
/// normal at `[2, 6]`) and leaf style (`voxel`), the outdoor auto exposure, the lamp off, the
/// sun at [`SUN`], the store counted and the live queue started at the default budget.
pub fn frame(rig: &Rig, arm: Arm) -> Frame {
    let mut f = Frame::with_sizes(&rig.device, INTERNAL, OUT);
    f.set_upscaler_kind(
        &rig.device,
        svet_render::UpscalerKind::Fsr2(svet_render::Fsr2Settings {
            sharpness: 0.5,
            ..Default::default()
        }),
    );
    f.set_count_store(true);
    f.set_look(svet_render::LookParams {
        far_normal: flythrough::LookPreset::DEFAULT.far_normal,
        view: if arm == Arm::Albedo {
            svet_render::ViewMode::Albedo
        } else {
            svet_render::ViewMode::Shaded
        },
        ..svet_render::LookParams::default()
    });
    f.set_material_styles(
        &rig.queue,
        &flythrough::outdoor_styles(flythrough::LeafShadow::Voxel),
    );
    let base = flythrough::default_lighting();
    let (key_far, penumbra) = match arm {
        Arm::Picks | Arm::Albedo => (base.key_far, flythrough::DEFAULT_PENUMBRA),
        Arm::Before => (svet_render::KeyFar::Ray, svet_render::Penumbra::Disc),
    };
    f.set_lighting(svet_render::Lighting {
        lamp: false,
        key_far,
        penumbra,
        exposure: svet_render::Exposure::Auto(svet_render::AutoExposure {
            min: flythrough::cli::OUTDOOR_MIN,
            max: flythrough::cli::OUTDOOR_MAX,
            ..svet_render::AutoExposure::default()
        }),
        ..base
    });
    let sky = flythrough::daylight::Clock {
        fixed_sun: Some(SUN),
        ..Default::default()
    }
    .sky(flythrough::outdoor::BASE_Z as f32);
    f.set_sky(Some(sky));
    f.set_dt(1.0 / 60.0);
    f.start_bake_queue(
        &rig.queue,
        &rig.gpu,
        svet_render::queue_settings(),
        svet_render::DEFAULT_BAKE_SLICE_MS,
    );
    f.set_bake_coarse(8, 2);
    f
}

/// One frame's display luma and each display pixel's class.
pub struct Read {
    pub luma: Vec<u8>,
    pub class: Vec<u8>,
}

/// One frame of `f` at `cam`, the queue stepped first when `queue`, read back.
pub fn step(rig: &mut Rig, f: &mut Frame, cam: &Camera, queue: bool) -> Read {
    if queue {
        queue_step(rig, f, cam);
    }
    f.step(
        &rig.device,
        &rig.queue,
        &rig.gpu,
        cam,
        &svet_render::LightParams::headlamp(),
    );
    read(rig, f, cam)
}

/// The display luma of `f`'s last frame (0..255, Rec. 709 weights) and the class of each
/// display pixel by its lighting pixel's hit: [`SKY`] at a miss, [`NEAR`] under [`NEAR_M`],
/// else [`FAR`].
pub fn read(rig: &Rig, f: &Frame, cam: &Camera) -> Read {
    let rgba = svet_render::debug::read_rgba8(&rig.device, &rig.queue, f.target());
    let hits = svet_render::debug::read_rgba32f(&rig.device, &rig.queue, f.light_hit_positions());
    let luma = rgba
        .chunks(4)
        .map(|p| {
            (0.2126 * f32::from(p[0]) + 0.7152 * f32::from(p[1]) + 0.0722 * f32::from(p[2])).round()
                as u8
        })
        .collect();
    let (dw, dh) = (OUT[0] as usize, OUT[1] as usize);
    let gw = f.light_hit_positions().width() as usize;
    let eye = cam.position.to_array();
    let class = (0..dw * dh)
        .map(|i| {
            let h = hits[grid_of(i, dw, dh, gw, hits.len())];
            if !h[3].is_finite() || h[3] > 1e30 {
                return SKY;
            }
            let d = ((h[0] - eye[0]).powi(2) + (h[1] - eye[1]).powi(2) + (h[2] - eye[2]).powi(2))
                .sqrt()
                * VOXEL_M;
            if d < NEAR_M { NEAR } else { FAR }
        })
        .collect();
    Read { luma, class }
}

/// The percentage of the pixels of `class` in `classes` whose luma moved by over [`T`] from `a`
/// to `b`; 0 with no such pixel.
pub fn over(a: &[u8], b: &[u8], classes: &[u8], class: u8) -> f64 {
    let (mut n, mut o) = (0usize, 0usize);
    for i in 0..a.len() {
        if classes[i] == class {
            n += 1;
            o += usize::from(a[i].abs_diff(b[i]) > T);
        }
    }
    100.0 * o as f64 / n.max(1) as f64
}

/// The percentage of the display's pixels in `class`.
pub fn share(classes: &[u8], class: u8) -> f64 {
    100.0 * classes.iter().filter(|&&c| c == class).count() as f64 / classes.len().max(1) as f64
}

/// `cam` moved `metres` along its heading in the horizontal plane.
pub fn moved(cam: &Camera, metres: f32) -> Camera {
    let f = cam.basis().0;
    let h = (f.x * f.x + f.y * f.y).sqrt().max(1e-6);
    let mut c = *cam;
    c.position.x += f.x / h * metres / VOXEL_M;
    c.position.y += f.y / h * metres / VOXEL_M;
    c
}

/// `cam` turned `deg` degrees in yaw.
pub fn yawed(cam: &Camera, deg: f32) -> Camera {
    let mut c = *cam;
    c.yaw += deg.to_radians();
    c
}

/// The live queue's depth in words.
pub fn depth(f: &Frame) -> String {
    f.bake_queue().map_or("no queue".into(), |q| {
        let st = q.stats();
        format!("queue depth {} ({} jobs)", st.depth, st.jobs)
    })
}
