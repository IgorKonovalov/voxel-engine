//! The look harness's shared helpers: the carved sphere, the rock ball and the error readings both
//! `gpu_look` binaries take.

use super::*;
use vxe_render::{LightParams, LookParams, NormalMode, ViewMode};

pub fn look(normal_mode: NormalMode, view: ViewMode) -> LookParams {
    LookParams {
        normal_mode,
        view,
        ..LookParams::default()
    }
}

/// Cavity normal error (degrees) and pillar silhouette error (pixels) of `mode`, over mirrors
/// without the field's band slots.
pub fn metrics(mode: NormalMode) -> Option<(f64, usize)> {
    metrics_with(mode, false)
}

/// [`metrics`], over mirrors with the field's band slots when `attrs` is set.
pub fn metrics_with(mode: NormalMode, attrs: bool) -> Option<(f64, usize)> {
    guardrails_of(mode, attrs).map(|g| (g.normal_error, g.silhouette))
}

/// The guardrails of the "not a Lego look" bar: the cavity's normal error (degrees), the
/// pillar's silhouette error (pixels) and the cavity's axis fraction `A`, the share of its
/// hit pixels whose shading normal lies within 5 degrees of a coordinate axis.
#[derive(Clone, Copy, Debug)]
pub struct Guardrails {
    pub normal_error: f64,
    pub silhouette: usize,
    pub axis_fraction: f64,
}

/// [`Guardrails`] of `mode`, over mirrors with the field's band slots when `attrs` is set.
pub fn guardrails_of(mode: NormalMode, attrs: bool) -> Option<Guardrails> {
    let look = |view| LookParams {
        normal_mode: mode,
        view,
        ..LookParams::default()
    };
    let cavity = cavity_world_with(look_table());
    let mut rig = if attrs {
        field_mirror_rig(&cavity)?
    } else {
        LookRig::new(&cavity, None)?
    };
    let cam = cavity_camera();
    let normals = rig.render(&cam, &LightParams::at_eye(), look(ViewMode::Normals));
    let normal_error = cavity_normal_error(&normals, &cam);
    let axis = axis_fraction(&normals);

    let pillar = pillar_world_with(flythrough::cave_materials());
    let mut rig = if attrs {
        field_mirror_rig_on(rig.device, rig.queue, &pillar, None)
    } else {
        LookRig::on(rig.device, rig.queue, &pillar, None)
    };
    let cam = pillar_camera();
    let mask = rig.render(&cam, &LightParams::at_eye(), look(ViewMode::HitMask));
    let silhouette = pillar_silhouette_error(&mask, &cam);
    println!(
        "{}: cavity normal error {normal_error:.2} degrees, pillar silhouette error {silhouette} pixels, axis fraction {:.2} %",
        mode.name(),
        axis * 100.0
    );
    Some(Guardrails {
        normal_error,
        silhouette,
        axis_fraction: axis,
    })
}

/// Pixels of a `WIDTH` x `HEIGHT` RGBA image whose Rec. 709 luma is below half the median luma
/// of their 8 neighbours; border pixels are not counted.
pub fn dark_specks(rgba: &[u8]) -> Vec<usize> {
    let (w, h) = (WIDTH as usize, HEIGHT as usize);
    let luma = |i: usize| {
        let p = &rgba[i * 4..i * 4 + 3];
        0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2])
    };
    let mut specks = Vec::new();
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let mut n = [0.0; 8];
            let mut k = 0;
            for dy in [-1, 0, 1] {
                for dx in [-1, 0, 1] {
                    if (dx, dy) != (0, 0) {
                        n[k] = luma((y as isize + dy) as usize * w + (x as isize + dx) as usize);
                        k += 1;
                    }
                }
            }
            n.sort_by(f64::total_cmp);
            // The median of eight: the mean of the middle two.
            let median = 0.5 * (n[3] + n[4]);
            if luma(y * w + x) < 0.5 * median {
                specks.push(y * w + x);
            }
        }
    }
    specks
}

/// The share of the hit pixels of a `view = Normals` render whose decoded normal lies within
/// 5 degrees of a coordinate axis, a miss (a decoded length under 0.5) not counted.
pub fn axis_fraction(rgba: &[u8]) -> f64 {
    let cos5 = 5f32.to_radians().cos();
    let (mut hits, mut axial) = (0usize, 0usize);
    for p in rgba.chunks(4) {
        let n = decode_normal(p);
        if n.length() < 0.5 {
            continue;
        }
        hits += 1;
        let n = n.normalize();
        if n.x.abs().max(n.y.abs()).max(n.z.abs()) >= cos5 {
            axial += 1;
        }
    }
    axial as f64 / hits.max(1) as f64
}
