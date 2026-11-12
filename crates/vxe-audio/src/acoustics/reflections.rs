//! Early reflections: the listener's reflection rays and the taps their hits give each voice.
//!
//! # Early reflections
//!
//! Sixteen rays leave the listener along a fixed Fibonacci sphere in world axes, its poles on
//! the world `x` axis ([`reflection_directions`]). A ray that enters a solid within
//! [`REFLECTION_RANGE_METRES`] hits at `H`, the entry of its first solid voxel
//! ([`World::raycast`], which stops there: `origin + dir * t`, exact in fixed point), with that
//! voxel's material `m`. Each hit gives every voice at `S`
//! the tap ([`crate::Tap`]) in the slot of its ray:
//! - delay `(|L - H| + |H - S| - |L - S|) / 343 m/s`, the excess over the direct path, in whole
//!   frames, since the mixer gives the direct path no propagation delay; a tap past
//!   [`crate::MAX_DELAY_FRAMES`] is left silent;
//! - gain `keep(m) / max(|L - H| + |H - S|, 1 m)` times the source's occlusion multiplier
//!   (the mean of its seven probe rays above). The leg from `H` to `S` is not traced: a source
//!   occluded from the listener is taken as equally occluded from every reflector, so an echo
//!   off the near face of a wall that hides the source is no louder than the source through it.
//!
//! A face keeps `keep(m) = (1 - A(m)) ^ (2 * REFLECTION_SKIN_METRES / 1 m)` of the amplitude
//! (ADR-0015): the wave enters one skin of the material and comes back out, absorbed per
//! metre as a transmitted ray is.
//!
//! A slot whose ray hits nothing has gain 0. The mixer glides each slot from what it plays to
//! its new target (see `mixer.rs`). With reflections switched off
//! ([`crate::Acoustics::set_reflections`]) no ray is traced and every slot's gain is 0, so the
//! echoes fade out.

use vxe_world::{FIXED_ONE, MaterialTable, World};

use super::{fibonacci_sphere, fixed_ray};
use crate::voice::{MAX_DELAY_FRAMES, MAX_TAPS, Tap};
use crate::{RATE, SPEED_OF_SOUND, length, sub};

/// How far a reflection or room ray looks for a surface.
pub const REFLECTION_RANGE_METRES: f32 = 100.0;
/// Reflection rays per listener.
pub const REFLECTION_RAYS: usize = 16;
/// The depth of material a reflection crosses on the way in, and again on the way out, for
/// its keep factor.
pub const REFLECTION_SKIN_METRES: f32 = 0.1;

// One tap slot per reflection ray.
const _: () = assert!(MAX_TAPS == REFLECTION_RAYS);

/// The amplitude a face of a material with absorption `a` per metre keeps on reflection:
/// `(1 - a) ^ (2 * REFLECTION_SKIN_METRES / 1 m)`.
pub fn reflection_keep(a: f32) -> f32 {
    (1.0 - a).clamp(0.0, 1.0).powf(2.0 * REFLECTION_SKIN_METRES)
}

/// The sixteen reflection directions: a Fibonacci sphere in world axes with its poles on `x`,
/// `x_i = 1 - 2i / 15`, `r_i = sqrt(1 - x_i^2)`, `phi_i = i pi (3 - sqrt 5)`,
/// `d_i = (x_i, r_i cos phi_i, r_i sin phi_i)`. `d_0` is `+x` and `d_15` is `-x` exactly.
pub fn reflection_directions() -> [[f32; 3]; REFLECTION_RAYS] {
    fibonacci_sphere::<REFLECTION_RAYS>()
}

/// A reflection ray's surface: where it entered the first solid (voxel units) and the
/// amplitude its face keeps ([`reflection_keep`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reflection {
    pub point: [f32; 3],
    pub keep: f32,
}

/// The surface each reflection ray from `listener` (voxel units) hits within
/// [`REFLECTION_RANGE_METRES`], by ray; `None` for a ray that hits nothing.
pub fn reflections(
    world: &World,
    table: &MaterialTable,
    listener: [f32; 3],
    directions: &[[f32; 3]; REFLECTION_RAYS],
    metres_per_voxel: f32,
) -> [Option<Reflection>; REFLECTION_RAYS] {
    let one = FIXED_ONE as f64;
    let range = REFLECTION_RANGE_METRES / metres_per_voxel;
    directions.map(|d| {
        let (origin, dir, dir_len) = fixed_ray(listener, d);
        let max_t = (range * FIXED_ONE as f32 * FIXED_ONE as f32 / dir_len).round() as i64;
        // A ray the world refuses (outside its limits) hits nothing.
        let hit = world.raycast(origin, dir, max_t).ok().flatten()?;
        // origin + dir * t / 256 in fixed units, then voxels.
        let point = std::array::from_fn(|k| {
            ((origin.0[k] as f64 + dir[k] as f64 * hit.t as f64 / one) / one) as f32
        });
        let a = table.get(hit.material).map_or(0.0, |m| m.acoustic[0]);
        Some(Reflection {
            point,
            keep: reflection_keep(a),
        })
    })
}

/// The tap each of `hits` gives a voice at `source` heard at `listener` (voxel units), slot
/// for slot, each gain scaled by the source's occlusion multiplier `occlusion_gain`. A slot
/// without a hit, or whose delay is past [`MAX_DELAY_FRAMES`], is silent (gain 0, delay 0).
pub fn taps(
    hits: &[Option<Reflection>; REFLECTION_RAYS],
    listener: [f32; 3],
    source: [f32; 3],
    metres_per_voxel: f32,
    occlusion_gain: f32,
) -> [Tap; MAX_TAPS] {
    let direct = length(sub(source, listener)) * metres_per_voxel;
    hits.map(|h| {
        let Some(h) = h else {
            return Tap::default();
        };
        let path =
            (length(sub(h.point, listener)) + length(sub(source, h.point))) * metres_per_voxel;
        let delay = ((path - direct).max(0.0) / SPEED_OF_SOUND * RATE as f32).round();
        if delay > MAX_DELAY_FRAMES as f32 {
            return Tap::default();
        }
        Tap {
            delay_frames: delay as u32,
            gain: h.keep / path.max(1.0) * occlusion_gain,
        }
    })
}
