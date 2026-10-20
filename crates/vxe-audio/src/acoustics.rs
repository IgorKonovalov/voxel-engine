//! What the voxel world does to each voice: occlusion and transmission through the material
//! table, probed on the caller's thread (ADR-0006).
//!
//! # The acoustic triple
//!
//! A material row's `acoustic` is `[A, T, R]`:
//! - `A`, absorption per metre of path: a ray keeps `(1 - A)` of its amplitude per metre it
//!   travels inside the material;
//! - `T`, transmission: the amplitude factor of entering the material, applied once per run;
//! - `R`, resonance: unused here.
//!
//! Air contributes nothing; a row of zeros (the default) blocks completely.
//!
//! # The occlusion model
//!
//! Seven probe rays leave the listener: one to the source and six to the points 0.5 m from the
//! source along each world axis. Each ray crosses runs of material ([`World::trace_materials`]);
//! its gain is the product over its solid runs of `T(m) * (1 - A(m)) ^ (L / 1 m)`, with `L` the
//! run's length in metres. The source's gain multiplier is the mean of the seven gains, and its
//! cutoff is `20 000 Hz / (1 + 10 * thickness)`, with `thickness` the summed solid length in
//! metres along the direct ray. The mixer applies both (see `mixer.rs`).
//!
//! # Early reflections
//!
//! Sixteen rays leave the listener along a fixed Fibonacci sphere in world axes, its poles on
//! the world `x` axis ([`reflection_directions`]). A ray that enters a solid within
//! [`REFLECTION_RANGE_METRES`] hits at `H`, the entry of its first solid run (`origin + dir *
//! t0`, exact in fixed point), with that run's material `m`. Each hit gives every voice at `S`
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
//! its new target (see `mixer.rs`). With reflections switched off ([`Acoustics::set_reflections`])
//! no ray is traced and every slot's gain is 0, so the echoes fade out.
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
//!   `min(16 pi (1 - alpha) / (S alpha), 16)`.
//!
//! A completed fan sends [`Command::SetRoom`] and is readable as [`Acoustics::room`].
//!
//! # Cadence
//!
//! [`Acoustics::update`] probes a voice when it or the listener moved more than 0.1 m since its
//! last probe, and at least once per 500 ms of audio time, counted in mixer blocks through a
//! [`BlockClock`] (no wall clock). A room fan begins at the first update, and again once the
//! listener has moved more than [`ROOM_MOVE_METRES`] from where the last fan began or
//! [`ROOM_INTERVAL_SECONDS`] of audio time has passed since; otherwise no room ray is traced.
//! A fan in progress traces from where it began and starts over if the listener moves more
//! than [`ROOM_MOVE_METRES`] from there.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use vxe_world::{AIR, FIXED_ONE, Face, Fixed3, MaterialTable, World};

use crate::command::RoomTargets;
use crate::voice::{AcousticTargets, MAX_DELAY_FRAMES, MAX_TAPS, OPEN_CUTOFF_HZ, Tap, VoiceId};
use crate::{BLOCK, Command, Listener, RATE, SPEED_OF_SOUND, length, sub};

/// Movement, in metres, of a voice or the listener that triggers a fresh probe.
pub const PROBE_MOVE_METRES: f32 = 0.1;
/// Audio time after which a voice is probed even if nothing moved.
pub const PROBE_INTERVAL_SECONDS: f32 = 0.5;
/// Offset of the six side probes from the source, in metres.
const SIDE_PROBE_METRES: f32 = 0.5;
/// How far a reflection ray looks for a surface.
pub const REFLECTION_RANGE_METRES: f32 = 50.0;
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
    let golden = std::f32::consts::PI * (3.0 - 5.0f32.sqrt());
    std::array::from_fn(|i| {
        let x = 1.0 - 2.0 * i as f32 / (REFLECTION_RAYS - 1) as f32;
        let r = (1.0 - x * x).max(0.0).sqrt();
        let phi = i as f32 * golden;
        [x, r * phi.cos(), r * phi.sin()]
    })
}

/// A reflection ray's surface: where it entered the first solid (voxel units) and the
/// amplitude its face keeps ([`reflection_keep`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reflection {
    pub point: [f32; 3],
    pub keep: f32,
}

/// A ray from `from` (voxel units) along `unit` in fixed point: the origin, the direction of
/// length close to [`FIXED_ONE`] (so `t` counts fixed units of distance), and its length.
fn fixed_ray(from: [f32; 3], unit: [f32; 3]) -> (Fixed3, [i64; 3], f32) {
    let one = FIXED_ONE as f32;
    let origin = Fixed3(from.map(|c| (c * one).floor() as i64));
    let dir = unit.map(|c| (c * one).round() as i64);
    (origin, dir, length(dir.map(|c| c as f32)))
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
        let run = world
            .trace_materials(origin, dir, max_t)
            .into_iter()
            .find(|r| r.material != AIR)?;
        // origin + dir * t0 / 256 in fixed units, then voxels.
        let point = std::array::from_fn(|k| {
            ((origin.0[k] as f64 + dir[k] as f64 * run.t0 as f64 / one) / one) as f32
        });
        let a = table.get(run.material).map_or(0.0, |m| m.acoustic[0]);
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

/// Mixer blocks rendered so far, shared between the mixer and the game thread.
#[derive(Clone, Debug, Default)]
pub struct BlockClock(Arc<AtomicU64>);

impl BlockClock {
    pub fn blocks(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }

    pub(crate) fn tick(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

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
    for run in world.trace_materials(origin, dir, max_t) {
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
    /// The estimate from a fan's sums over `n` rays: `sum r^3`, `sum r^2 / cos` and the
    /// absorption sum `sum (r^2 / cos) (1 - keep^2)`, metres.
    fn from_sums(n: usize, r3: f64, w: f64, wa: f64) -> Self {
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
        let energy = if s > 0.0 {
            (16.0 * std::f64::consts::PI * (1.0 - alpha) / (s * alpha)).min(16.0)
        } else {
            16.0
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
/// `cos` of its incidence on the face it entered (floored), and the face's energy absorption.
/// A miss is a fully absorbing face at the range, square on.
fn room_ray(
    world: &World,
    table: &MaterialTable,
    from: [f32; 3],
    d: [f32; 3],
    metres_per_voxel: f32,
) -> (f64, f64, f64) {
    let one = FIXED_ONE as f32;
    let range = REFLECTION_RANGE_METRES / metres_per_voxel;
    let (origin, dir, dir_len) = fixed_ray(from, d);
    let max_t = (range * one * one / dir_len).round() as i64;
    // A ray the world refuses (outside its limits) counts as a miss.
    let Some(hit) = world.raycast(origin, dir, max_t).ok().flatten() else {
        return (f64::from(REFLECTION_RANGE_METRES), 1.0, 1.0);
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
    (r, f64::from(cos), f64::from(1.0 - keep * keep))
}

/// A room fan in progress: where it began, how many rays are done and their sums.
struct Fan {
    origin: [f32; 3],
    next: usize,
    r3: f64,
    w: f64,
    wa: f64,
}

/// Where a voice and the listener were at the voice's last probe.
struct ProbeState {
    source: [f32; 3],
    listener: [f32; 3],
    block: u64,
    /// Taps sent at that probe.
    taps: usize,
}

/// Probes the world for every voice and sends the mixer its targets.
pub struct Acoustics {
    last: HashMap<VoiceId, ProbeState>,
    clock: BlockClock,
    probed: usize,
    directions: [[f32; 3]; REFLECTION_RAYS],
    reflections: bool,
    room_dirs: [[f32; 3]; ROOM_RAYS],
    /// The fan being traced.
    fan: Option<Fan>,
    /// Where and when the last fan began.
    fan_began: Option<([f32; 3], u64)>,
    room: Option<RoomEstimate>,
    reverb: bool,
    /// The room targets must go out again at the next update (the tail was switched).
    resend_room: bool,
    room_rays: usize,
}

impl Acoustics {
    /// Probes on the audio time `clock` counts (from [`crate::Mixer::clock`]).
    pub fn new(clock: BlockClock) -> Self {
        Self {
            last: HashMap::new(),
            clock,
            probed: 0,
            directions: reflection_directions(),
            reflections: true,
            room_dirs: room_directions(),
            fan: None,
            fan_began: None,
            room: None,
            reverb: true,
            resend_room: false,
            room_rays: 0,
        }
    }

    /// Switches the late tail on (the default) or off. Off, the room targets carry energy 0;
    /// a change sends the last estimate's targets again at the next [`Acoustics::update`].
    pub fn set_reverb(&mut self, on: bool) {
        self.resend_room |= on != self.reverb;
        self.reverb = on;
    }

    /// Whether the late tail is on.
    pub fn reverb_on(&self) -> bool {
        self.reverb
    }

    /// The room as the last complete fan saw it; `None` before the first completes.
    pub fn room(&self) -> Option<RoomEstimate> {
        self.room
    }

    /// Room rays traced by the last [`Acoustics::update`].
    pub fn last_room_rays(&self) -> usize {
        self.room_rays
    }

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
    fn step_room(
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
            if due {
                self.fan = Some(Fan {
                    origin: listener,
                    next: 0,
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
                let (r, cos, absorb) = room_ray(world, table, fan.origin, d, metres_per_voxel);
                let w = r * r / cos;
                fan.r3 += r * r * r;
                fan.w += w;
                fan.wa += w * absorb;
            }
            self.room_rays = end - fan.next;
            fan.next = end;
            if end == ROOM_RAYS {
                let room = RoomEstimate::from_sums(ROOM_RAYS, fan.r3, fan.w, fan.wa);
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

    /// Switches the early reflections on (the default) or off. Off, no reflection ray is
    /// traced and every probe sends silent taps, which the mixer fades out. A change makes
    /// every voice due at the next [`Acoustics::update`].
    pub fn set_reflections(&mut self, on: bool) {
        if on != self.reflections {
            self.last.clear();
        }
        self.reflections = on;
    }

    /// Whether the early reflections are on.
    pub fn reflections_on(&self) -> bool {
        self.reflections
    }

    /// Voices probed by the last [`Acoustics::update`].
    pub fn last_probed(&self) -> usize {
        self.probed
    }

    /// Tap slots with a nonzero gain the voices carry as of their last probes.
    pub fn active_taps(&self) -> usize {
        self.last.values().map(|s| s.taps).sum()
    }

    /// Probes each of `voices` (id and position, voxel units) that is due and sends a
    /// [`Command::SetAcoustics`] for it through `send`, then advances the room fan, sending a
    /// [`Command::SetRoom`] when a fan completes. Voices missing from `voices` are forgotten.
    pub fn update(
        &mut self,
        world: &World,
        table: &MaterialTable,
        listener: &Listener,
        voices: &[(VoiceId, [f32; 3])],
        metres_per_voxel: f32,
        send: &mut dyn FnMut(Command),
    ) {
        let now = self.clock.blocks();
        let interval = (PROBE_INTERVAL_SECONDS * RATE as f32 / BLOCK as f32).ceil() as u64;
        let moved = PROBE_MOVE_METRES / metres_per_voxel;
        self.last
            .retain(|id, _| voices.iter().any(|(v, _)| v == id));
        self.probed = 0;
        // The listener's reflections, traced once per update and only when a voice is due.
        let mut hits = None;
        for &(id, source) in voices {
            let due = self.last.get(&id).is_none_or(|s| {
                length(sub(source, s.source)) > moved
                    || length(sub(listener.position, s.listener)) > moved
                    || now.saturating_sub(s.block) >= interval
            });
            if !due {
                continue;
            }
            let o = occlusion(world, table, listener.position, source, metres_per_voxel);
            let taps = if self.reflections {
                let hits = hits.get_or_insert_with(|| {
                    reflections(
                        world,
                        table,
                        listener.position,
                        &self.directions,
                        metres_per_voxel,
                    )
                });
                taps(hits, listener.position, source, metres_per_voxel, o.gain)
            } else {
                [Tap::default(); MAX_TAPS]
            };
            send(Command::SetAcoustics(
                id,
                AcousticTargets {
                    gain: o.gain,
                    cutoff_hz: o.cutoff_hz,
                    taps,
                    send: o.gain,
                },
            ));
            self.last.insert(
                id,
                ProbeState {
                    source,
                    listener: listener.position,
                    block: now,
                    taps: taps.iter().filter(|t| t.gain != 0.0).count(),
                },
            );
            self.probed += 1;
        }
        self.step_room(world, table, listener.position, metres_per_voxel, send);
    }
}
