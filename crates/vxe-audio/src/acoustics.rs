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
//! one tap ([`crate::Tap`]):
//! - delay `(|L - H| + |H - S| - |L - S|) / 343 m/s`, the excess over the direct path, in whole
//!   frames, since the mixer gives the direct path no propagation delay; a tap past
//!   [`crate::MAX_DELAY_FRAMES`] is dropped;
//! - gain `(1 - A(m)) / max(|L - H| + |H - S|, 1 m)` times the source's occlusion multiplier
//!   (the mean of its seven probe rays above). The leg from `H` to `S` is not traced: a source
//!   occluded from the listener is taken as equally occluded from every reflector, so an echo
//!   off the near face of a wall that hides the source is no louder than the source through it.
//!
//! The mixer feeds the taps, times the voice's own gain, into the voice's mono signal ahead of
//! its HRIR, so echoes share the voice's direction (see `mixer.rs`). With reflections switched
//! off ([`Acoustics::set_reflections`]) no ray is traced and every voice gets no taps.
//!
//! # Cadence
//!
//! [`Acoustics::update`] probes a voice when it or the listener moved more than 0.1 m since its
//! last probe, and at least once per 500 ms of audio time, counted in mixer blocks through a
//! [`BlockClock`] (no wall clock).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use vxe_world::{AIR, FIXED_ONE, Fixed3, MaterialTable, World};

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

/// A reflection ray's surface: where it entered the first solid (voxel units) and that solid's
/// `1 - A`.
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

/// The surfaces the reflection rays from `listener` (voxel units) hit within
/// [`REFLECTION_RANGE_METRES`], in direction order; a ray that hits nothing is left out.
pub fn reflections(
    world: &World,
    table: &MaterialTable,
    listener: [f32; 3],
    directions: &[[f32; 3]; REFLECTION_RAYS],
    metres_per_voxel: f32,
) -> Vec<Reflection> {
    let one = FIXED_ONE as f64;
    let range = REFLECTION_RANGE_METRES / metres_per_voxel;
    let mut hits = Vec::with_capacity(REFLECTION_RAYS);
    for &d in directions {
        let (origin, dir, dir_len) = fixed_ray(listener, d);
        let max_t = (range * FIXED_ONE as f32 * FIXED_ONE as f32 / dir_len).round() as i64;
        let Some(run) = world
            .trace_materials(origin, dir, max_t)
            .into_iter()
            .find(|r| r.material != AIR)
        else {
            continue;
        };
        // origin + dir * t0 / 256 in fixed units, then voxels.
        let point = std::array::from_fn(|k| {
            ((origin.0[k] as f64 + dir[k] as f64 * run.t0 as f64 / one) / one) as f32
        });
        let a = table.get(run.material).map_or(0.0, |m| m.acoustic[0]);
        hits.push(Reflection {
            point,
            keep: (1.0 - a).clamp(0.0, 1.0),
        });
    }
    hits
}

/// The taps the `hits` give a voice at `source` heard at `listener` (voxel units), each gain
/// scaled by the source's occlusion multiplier `occlusion_gain`, at most [`MAX_TAPS`]; taps
/// past [`MAX_DELAY_FRAMES`] are dropped.
pub fn taps(
    hits: &[Reflection],
    listener: [f32; 3],
    source: [f32; 3],
    metres_per_voxel: f32,
    occlusion_gain: f32,
) -> ([Tap; MAX_TAPS], u8) {
    let mut out = [Tap::default(); MAX_TAPS];
    let mut n = 0;
    let direct = length(sub(source, listener)) * metres_per_voxel;
    for h in hits {
        if n == MAX_TAPS {
            break;
        }
        let path =
            (length(sub(h.point, listener)) + length(sub(source, h.point))) * metres_per_voxel;
        let delay = ((path - direct).max(0.0) / SPEED_OF_SOUND * RATE as f32).round();
        if delay > MAX_DELAY_FRAMES as f32 {
            continue;
        }
        out[n] = Tap {
            delay_frames: delay as u32,
            gain: h.keep / path.max(1.0) * occlusion_gain,
        };
        n += 1;
    }
    (out, n as u8)
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
        }
    }

    /// Switches the early reflections on (the default) or off. Off, no reflection ray is
    /// traced and every probe sends no taps; a voice keeps its old taps until its next probe.
    pub fn set_reflections(&mut self, on: bool) {
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

    /// Taps the voices carry as of their last probes.
    pub fn active_taps(&self) -> usize {
        self.last.values().map(|s| s.taps).sum()
    }

    /// Probes each of `voices` (id and position, voxel units) that is due and sends a
    /// [`Command::SetAcoustics`] for it through `send`. Voices missing from `voices` are
    /// forgotten.
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
            let (taps, tap_count) = if self.reflections {
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
                ([Tap::default(); MAX_TAPS], 0)
            };
            send(Command::SetAcoustics(
                id,
                AcousticTargets {
                    gain: o.gain,
                    cutoff_hz: o.cutoff_hz,
                    taps,
                    tap_count,
                },
            ));
            self.last.insert(
                id,
                ProbeState {
                    source,
                    listener: listener.position,
                    block: now,
                    taps: usize::from(tap_count),
                },
            );
            self.probed += 1;
        }
    }
}
