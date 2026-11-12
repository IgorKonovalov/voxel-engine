//! The room fan: the room the listener sees, which sizes the late tail.
//!
//! # The room fan
//!
//! The late tail is sized by the room the listener sees (ADR-0015). [`ROOM_RAYS`] rays leave the
//! listener along a fixed Fibonacci sphere ([`room_directions`]), [`ROOM_RAYS_PER_UPDATE`] per
//! [`Acoustics::update`]. A ray's first solid voxel ([`World::raycast`], which stops there)
//! gives its hit distance `r` and the face it entered, and `cos(theta) = |dir_axis| / |dir|`
//! for that face's axis, floored at [`ROOM_COS_FLOOR`]. A ray that hits nothing
//! within [`REFLECTION_RANGE_METRES`] counts as a fully absorbing face at the range with
//! `cos = 1`. Over the `N` rays of a complete fan:
//! - `V = (4 pi / 3N) sum r^3`, `S = (4 pi / N) sum r^2 / cos`, the visible volume and surface;
//! - `alpha`, the `r^2 / cos`-weighted mean of `1 - keep(m)^2` (1 for a miss), clamped to
//!   `[0.02, 0.98]`;
//! - mean free path `4V / S`, Eyring's `T60 = 0.161 V / (-S ln(1 - alpha))` clamped to
//!   `[0.05 s, MAX_T60_SECONDS]`, and the tail's energy against a unit-energy direct impulse,
//!   `min(16 pi (1 - alpha) / (S alpha), 16)`. A fan whose rays all miss (open air) or that
//!   measures no surface (`S = 0`) gives energy 0: there is no room to ring.
//!
//! A completed fan sends [`Command::SetRoom`] and is readable as [`Acoustics::room`]. A fan due
//! while the listener's own voxel is solid (a camera flying through rock) is discarded before
//! any ray: every ray would hit at `t = 0`. Nothing is sent, [`Acoustics::room`] keeps the last
//! estimate, and the next update tries again.

use vxe_world::{AIR, FIXED_ONE, Face, MaterialTable, World};

use super::reflections::{REFLECTION_RANGE_METRES, reflection_keep};
use super::{Acoustics, fixed_ray};
use crate::command::RoomTargets;
use crate::{BLOCK, Command, RATE, length, sub};

/// Rays in a room fan.
pub const ROOM_RAYS: usize = 128;
/// Room rays traced per [`Acoustics::update`] while a fan is in progress.
pub const ROOM_RAYS_PER_UPDATE: usize = 16;
/// The floor of a room ray's `cos(theta)`, so a grazing hit counts at most ten times its area.
pub const ROOM_COS_FLOOR: f32 = 0.1;
/// Listener movement, metres, after which a new room fan begins.
pub const ROOM_MOVE_METRES: f32 = 1.0;
/// Audio time after which a new room fan begins even if the listener stood still.
pub const ROOM_INTERVAL_SECONDS: f32 = 0.5;
/// The longest decay time a room estimate gives.
pub const MAX_T60_SECONDS: f32 = 8.0;

/// The room fan's directions: a Fibonacci sphere in world axes with its poles on `x`, built as
/// [`reflection_directions`] is, at [`ROOM_RAYS`] points.
pub fn room_directions() -> [[f32; 3]; ROOM_RAYS] {
    let golden = std::f32::consts::PI * (3.0 - 5.0f32.sqrt());
    std::array::from_fn(|i| {
        let x = 1.0 - 2.0 * i as f32 / (ROOM_RAYS - 1) as f32;
        let r = (1.0 - x * x).max(0.0).sqrt();
        let phi = i as f32 * golden;
        [x, r * phi.cos(), r * phi.sin()]
    })
}

/// The room the listener sees, from a complete fan.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoomEstimate {
    pub volume_m3: f32,
    pub surface_m2: f32,
    pub mean_free_path_m: f32,
    /// The area-weighted energy absorption of the faces, clamped to `[0.02, 0.98]`.
    pub absorption: f32,
    pub t60_s: f32,
    /// The tail's energy relative to a unit-energy direct impulse.
    pub energy: f32,
}

impl RoomEstimate {
    /// The estimate from a fan's sums over `n` rays, `hits` of which hit a surface: `sum r^3`,
    /// `sum r^2 / cos` and the absorption sum `sum (r^2 / cos) (1 - keep^2)`, metres.
    fn from_sums(n: usize, hits: usize, r3: f64, w: f64, wa: f64) -> Self {
        let n = n as f64;
        let four_pi = 4.0 * std::f64::consts::PI;
        let v = four_pi / (3.0 * n) * r3;
        let s = four_pi / n * w;
        let alpha = if w > 0.0 { wa / w } else { 1.0 }.clamp(0.02, 0.98);
        let mfp = if s > 0.0 { 4.0 * v / s } else { 0.0 };
        let t60 = if s > 0.0 {
            0.161 * v / (-s * (1.0 - alpha).ln())
        } else {
            0.0
        };
        let energy = if s > 0.0 && hits > 0 {
            (16.0 * std::f64::consts::PI * (1.0 - alpha) / (s * alpha)).min(16.0)
        } else {
            0.0
        };
        Self {
            volume_m3: v as f32,
            surface_m2: s as f32,
            mean_free_path_m: mfp as f32,
            absorption: alpha as f32,
            t60_s: (t60 as f32).clamp(0.05, MAX_T60_SECONDS),
            energy: energy as f32,
        }
    }
}

/// One room ray from `from` (voxel units) along unit `d`: its hit distance in metres, the
/// `cos` of its incidence on the face it entered (floored), the face's energy absorption, and
/// whether it hit. A miss is a fully absorbing face at the range, square on.
fn room_ray(
    world: &World,
    table: &MaterialTable,
    from: [f32; 3],
    d: [f32; 3],
    metres_per_voxel: f32,
) -> (f64, f64, f64, bool) {
    let one = FIXED_ONE as f32;
    let range = REFLECTION_RANGE_METRES / metres_per_voxel;
    let (origin, dir, dir_len) = fixed_ray(from, d);
    let max_t = (range * one * one / dir_len).round() as i64;
    // A ray the world refuses (outside its limits) counts as a miss.
    let Some(hit) = world.raycast(origin, dir, max_t).ok().flatten() else {
        return (f64::from(REFLECTION_RANGE_METRES), 1.0, 1.0, false);
    };
    let r = hit.t as f64 * f64::from(dir_len) / f64::from(one * one) * f64::from(metres_per_voxel);
    // The face the ray entered, exact from the walk; a ray that starts inside a solid takes
    // the axis of its largest component.
    let axis = match hit.face {
        Some(Face::NegX | Face::PosX) => 0,
        Some(Face::NegY | Face::PosY) => 1,
        Some(Face::NegZ | Face::PosZ) => 2,
        None => (0..3).max_by_key(|&k| dir[k].abs()).unwrap_or(0),
    };
    let cos = (dir[axis].abs() as f32 / dir_len).max(ROOM_COS_FLOOR);
    let a = table.get(hit.material).map_or(0.0, |m| m.acoustic[0]);
    let keep = reflection_keep(a);
    (r, f64::from(cos), f64::from(1.0 - keep * keep), true)
}

/// A room fan in progress: where it began, how many rays are done and their sums.
pub(super) struct Fan {
    origin: [f32; 3],
    next: usize,
    hits: usize,
    r3: f64,
    w: f64,
    wa: f64,
}
impl Acoustics {
    /// The targets `room` gives the mixer, energy 0 with the tail off.
    fn room_targets(&self, room: RoomEstimate) -> RoomTargets {
        RoomTargets {
            t60_s: room.t60_s,
            mean_free_path_m: room.mean_free_path_m,
            energy: if self.reverb { room.energy } else { 0.0 },
        }
    }

    /// Advances the room fan by one slice, beginning or restarting it as the cadence says, and
    /// sends [`Command::SetRoom`] when it completes.
    pub(super) fn step_room(
        &mut self,
        world: &World,
        table: &MaterialTable,
        listener: [f32; 3],
        metres_per_voxel: f32,
        send: &mut dyn FnMut(Command),
    ) {
        let now = self.clock.blocks();
        let moved = ROOM_MOVE_METRES / metres_per_voxel;
        let interval = (ROOM_INTERVAL_SECONDS * RATE as f32 / BLOCK as f32).ceil() as u64;
        self.room_rays = 0;
        if self
            .fan
            .as_ref()
            .is_some_and(|f| length(sub(listener, f.origin)) > moved)
        {
            self.fan = None;
        }
        if self.fan.is_none() {
            let due = self.fan_began.is_none_or(|(at, block)| {
                length(sub(listener, at)) > moved || now.saturating_sub(block) >= interval
            });
            // From inside a solid every ray hits at once: no room to measure.
            let voxel = listener.map(|c| c.floor() as i32);
            let solid = world.material_at(voxel).is_some_and(|m| m != AIR);
            if due && solid {
                self.discarded += 1;
            } else if due {
                self.fan = Some(Fan {
                    origin: listener,
                    next: 0,
                    hits: 0,
                    r3: 0.0,
                    w: 0.0,
                    wa: 0.0,
                });
                self.fan_began = Some((listener, now));
            }
        }
        if let Some(fan) = self.fan.as_mut() {
            let end = (fan.next + ROOM_RAYS_PER_UPDATE).min(ROOM_RAYS);
            for &d in &self.room_dirs[fan.next..end] {
                let (r, cos, absorb, hit) = room_ray(world, table, fan.origin, d, metres_per_voxel);
                fan.hits += usize::from(hit);
                let w = r * r / cos;
                fan.r3 += r * r * r;
                fan.w += w;
                fan.wa += w * absorb;
            }
            self.room_rays = end - fan.next;
            fan.next = end;
            if end == ROOM_RAYS {
                let room = RoomEstimate::from_sums(ROOM_RAYS, fan.hits, fan.r3, fan.w, fan.wa);
                self.fan = None;
                self.room = Some(room);
                self.resend_room = false;
                send(Command::SetRoom(self.room_targets(room)));
            }
        }
        if self.resend_room {
            self.resend_room = false;
            if let Some(room) = self.room {
                send(Command::SetRoom(self.room_targets(room)));
            }
        }
    }
}
