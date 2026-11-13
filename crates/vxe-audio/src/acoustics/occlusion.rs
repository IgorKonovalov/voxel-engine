//! Occlusion and transmission through the material table, probed on the caller's thread
//! (ADR-0006).
//!
//! # The occlusion model
//!
//! Seven probe rays leave the listener: one to the source and six to the points 0.5 m from the
//! source along each world axis. Each ray crosses runs of material ([`World::trace_materials`]);
//! its gain is the product over its solid runs of `T(m) * (1 - A(m)) ^ (L / 1 m)`, with `L` the
//! run's length in metres. The source's gain multiplier is the mean of the seven gains, and its
//! cutoff is `20 000 Hz / (1 + 10 * thickness)`, with `thickness` the summed solid length in
//! metres along the direct ray. The mixer applies both (see `mixer.rs`).

use vxe_world::{AIR, FIXED_ONE, MaterialTable, World};

use super::fixed_ray;
use crate::voice::OPEN_CUTOFF_HZ;
use crate::{length, sub};

/// Offset of the six side probes from the source, in metres.
const SIDE_PROBE_METRES: f32 = 0.5;

/// The occlusion of one source.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Occlusion {
    /// Mean of the seven probe gains.
    pub gain: f32,
    /// Summed solid length along the direct ray, metres.
    pub thickness_m: f32,
    /// `20 000 Hz / (1 + 10 * thickness_m)`.
    pub cutoff_hz: f32,
}

/// The gain of one ray from `from` to `to` (voxel units) and the solid length it crosses in
/// metres.
fn ray(
    world: &World,
    table: &MaterialTable,
    from: [f32; 3],
    to: [f32; 3],
    metres_per_voxel: f32,
) -> (f32, f32) {
    let span = sub(to, from);
    let len = length(span);
    if len < 1e-6 {
        return (1.0, 0.0);
    }
    let one = FIXED_ONE as f32;
    // |dir| close to 256, so t counts fixed units (1/256 voxel) of distance.
    let (origin, dir, dir_len) = fixed_ray(from, span.map(|c| c / len));
    let max_t = (len * one * one / dir_len).round() as i64;
    // Metres per unit of t: position moves |dir| * t / 256 fixed units.
    let metres_per_t = dir_len / one / one * metres_per_voxel;
    let mut gain = 1.0f32;
    let mut thickness = 0.0f32;
    // A ray the world's limits refuse (an origin or a reach past them) traces nothing: the
    // path counts as open air.
    for run in world
        .trace_materials(origin, dir, max_t)
        .unwrap_or_default()
    {
        if run.material == AIR {
            continue;
        }
        let l = (run.t1 - run.t0) as f32 * metres_per_t;
        let [a, t, _] = table.get(run.material).map_or([0.0; 3], |m| m.acoustic);
        gain *= t * (1.0 - a).max(0.0).powf(l);
        thickness += l;
    }
    (gain, thickness)
}

/// The occlusion of a source at `source` heard at `listener` (voxel units), probed through
/// `world` with the acoustic triples of `table`.
pub fn occlusion(
    world: &World,
    table: &MaterialTable,
    listener: [f32; 3],
    source: [f32; 3],
    metres_per_voxel: f32,
) -> Occlusion {
    let (direct, thickness_m) = ray(world, table, listener, source, metres_per_voxel);
    let side = SIDE_PROBE_METRES / metres_per_voxel;
    let mut sum = direct;
    for axis in 0..3 {
        for sign in [-1.0, 1.0] {
            let mut p = source;
            p[axis] += sign * side;
            sum += ray(world, table, listener, p, metres_per_voxel).0;
        }
    }
    Occlusion {
        gain: sum / 7.0,
        thickness_m,
        cutoff_hz: OPEN_CUTOFF_HZ / (1.0 + 10.0 * thickness_m),
    }
}
