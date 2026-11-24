//! The surface mover: a kinematic sphere attached to the surface it touches (ADR-0062).
//!
//! A body is a sphere with an integer contact normal `n` (pointing from solid toward air) and
//! a forward `f` in the tangent plane, both kept at length [`DIR_ONE`]. One tick, in a fixed
//! order:
//!
//! 1. `f` is turned about `n` by the input's heading step, projected onto the tangent plane
//!    and renormalised.
//! 2. When attached, a sweep along `f` by the speed. A contact that stops it is a **concave**
//!    edge: its normal becomes `n`, `f` is carried across by the shortest arc from the old
//!    normal to the new one, and what is left of the speed is swept once more along the new
//!    forward.
//! 3. When attached, a sweep along `-n` by the attach reach. A contact snaps the centre to it
//!    and its normal becomes `n` (a **convex** edge when that differs from the tick's starting
//!    normal and step 2 met nothing); no contact detaches the body and leaves the centre where
//!    step 2 put it.
//! 4. When detached, the vertical speed gains gravity (before the move), clamped to the
//!    fastest fall, and a sweep along it; a contact attaches the body with the contact's
//!    normal and reports the landing speed.
//!
//! An attached tick ends by reading the smoothed normal ([`SurfaceState::smooth_normal`]).
//!
//! A body that starts a tick overlapping solid (an edit filled into it) is first moved to the
//! first free offset of the controller's candidate order, or left where it is and reported
//! embedded. Gravity is the world's `-z`. Every quantity is an integer.

use svet_world::{FIXED_ONE, Fixed3, VoxelCoord, World};

use crate::arc::{DIR_ONE, cross, length, normalise, project, rdiv, rotate, shortest_arc};
use crate::body::{Contact, Sphere};
use crate::controller::{MAX_CONTACTS, depenetration_offsets};
use crate::sweep::{Sweep, sphere_overlaps, sweep_sphere};

/// The sphere's size and motion limits, in fixed units (1/256 voxel) and fixed units per tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SurfaceParams {
    pub radius: i64,
    /// How far along `-normal` a tick looks for the surface; past it the body detaches.
    pub attach_reach: i64,
    /// The fastest move a tick takes along the forward.
    pub speed_max: i64,
    /// Added to the vertical speed every detached tick; negative.
    pub gravity: i64,
    /// The fastest fall, as a positive speed.
    pub max_fall: i64,
    /// Read the smoothed normal each attached tick ([`SurfaceState::smooth_normal`]); off,
    /// it is the contact normal.
    pub smooth: bool,
}

/// A surface body's motion state between ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SurfaceState {
    pub centre: Fixed3,
    /// The contact normal, solid toward air, length [`DIR_ONE`] (exact on an axis).
    pub normal: [i64; 3],
    /// The surface's normal smoothed at the body's scale, length [`DIR_ONE`]: the
    /// solid-count gradient ([`World::solid_gradient`]) at the air voxel holding the contact
    /// point pushed half a voxel along the normal, over a cube of half-width `radius / 256`
    /// (at least 1). The contact normal where the gradient is zero, the query is refused or
    /// [`SurfaceParams::smooth`] is off; kept while detached. For aligning a body's pose; the
    /// motion follows the contact normal.
    pub smooth_normal: [i64; 3],
    /// The direction of motion in the tangent plane, length [`DIR_ONE`].
    pub forward: [i64; 3],
    pub attached: bool,
    /// Vertical speed while detached, fixed units per tick, up positive.
    pub vz: i64,
}

impl SurfaceState {
    /// Attached at `centre` with `normal` and `forward` (both normalised here; the forward is
    /// projected onto the tangent plane), at rest.
    pub fn new(centre: Fixed3, normal: [i64; 3], forward: [i64; 3]) -> Self {
        let normal = normalise(normal);
        Self {
            centre,
            normal,
            smooth_normal: normal,
            forward: tangent(forward, normal),
            attached: true,
            vz: 0,
        }
    }
}

/// One tick's input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SurfaceInput {
    /// The turn about the normal this tick, as a Q8 `(cos, sin)` pair: `(256, 0)` is straight
    /// on, a positive sine turns counter-clockwise seen from the normal's side.
    pub heading: [i32; 2],
    /// The move along the forward this tick, clamped to `[0, speed_max]`.
    pub speed: i64,
}

impl Default for SurfaceInput {
    fn default() -> Self {
        Self {
            heading: [256, 0],
            speed: 0,
        }
    }
}

/// What changed the body's surface this tick: the earliest in the tick's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SurfaceEvent {
    /// The move ran into a surface and the body took its normal.
    Concave,
    /// The attach step found the surface with a different normal (an edge rounded).
    Convex,
    /// The attach step found nothing within reach.
    Detached,
    /// A falling body touched a surface, at this speed.
    Landed(i64),
}

/// What one tick did.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SurfaceResult {
    pub moved: Fixed3,
    pub event: Option<SurfaceEvent>,
    /// The first [`MAX_CONTACTS`] contacts of the tick's sweeps, in sweep order.
    pub contacts: Vec<Contact>,
    /// The body overlapped solid at the start and no free offset was found; nothing moved.
    pub embedded: bool,
    /// The offset the body was lifted by before moving, zero when none.
    pub lifted: Fixed3,
}

impl SurfaceResult {
    fn touch(&mut self, s: &Sweep) {
        if let Some(c) = s.contact
            && self.contacts.len() < MAX_CONTACTS
        {
            self.contacts.push(c);
        }
    }

    fn event(&mut self, e: SurfaceEvent) {
        self.event.get_or_insert(e);
    }
}

/// `f` projected onto the plane of `n` and normalised; when that is zero (`f` along `n`), the
/// projection of the axis direction of lowest index that gives a non-zero one.
fn tangent(f: [i64; 3], n: [i64; 3]) -> [i64; 3] {
    let t = normalise(project(f, n));
    if t != [0; 3] {
        return t;
    }
    (0..3)
        .map(|k| {
            let mut e = [0; 3];
            e[k] = DIR_ONE;
            normalise(project(e, n))
        })
        .find(|t| *t != [0; 3])
        .unwrap_or([DIR_ONE, 0, 0])
}

/// `f` turned about `n` (length [`DIR_ONE`]) by the Q8 pair `[c, s]`: `(f c + (n x f) s /
/// 256) / 256`, rounded to nearest.
fn turn(f: [i64; 3], n: [i64; 3], h: [i32; 2]) -> [i64; 3] {
    if h == [256, 0] {
        return f;
    }
    let nf = cross(n, f);
    let (c, s) = (i128::from(h[0]), i128::from(h[1]));
    [0, 1, 2].map(|k| {
        let v = i128::from(f[k]) * c * 256 + i128::from(nf[k]) * s;
        rdiv(v, 256 * 256) as i64
    })
}

/// `d` scaled to length `len` (`d` of length [`DIR_ONE`]), each component toward zero.
fn along(d: [i64; 3], len: i64) -> Fixed3 {
    Fixed3(d.map(|c| (i128::from(c) * i128::from(len) / i128::from(DIR_ONE)) as i64))
}

fn add(a: [i64; 3], b: Fixed3) -> [i64; 3] {
    [0, 1, 2].map(|k| a[k] + b.0[k])
}

/// Moves a sphere over the surfaces of a world, one tick at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SurfaceMover {
    pub params: SurfaceParams,
}

impl SurfaceMover {
    pub fn new(params: SurfaceParams) -> Self {
        Self { params }
    }

    /// The body centred at `centre`.
    pub fn sphere(&self, centre: [i64; 3]) -> Sphere {
        Sphere {
            centre: Fixed3(centre),
            radius: self.params.radius,
        }
    }

    fn sweep(&self, world: &World, at: [i64; 3], delta: Fixed3) -> Sweep {
        sweep_sphere(world, &self.sphere(at), delta)
    }

    /// The first offset from `centre` at which the sphere overlaps nothing, in the
    /// controller's candidate order with both limits at twice the radius.
    fn depenetrate(&self, world: &World, centre: [i64; 3]) -> Option<[i64; 3]> {
        let lim = 2 * self.params.radius;
        depenetration_offsets(lim, lim).find(|off| {
            !sphere_overlaps(world, &self.sphere([0, 1, 2].map(|k| centre[k] + off[k])))
        })
    }

    /// Takes the contact normal `c` as the body's normal, carrying the forward across by the
    /// shortest arc from the old normal. Whether the normal changed.
    fn take_normal(state: &mut SurfaceState, c: [i64; 3]) -> bool {
        let n = normalise(c);
        if n == state.normal || n == [0; 3] {
            return false;
        }
        let f = rotate(shortest_arc(state.normal, n), state.forward);
        state.forward = tangent(f, n);
        state.normal = n;
        true
    }

    /// The smoothed normal at `state`'s contact ([`SurfaceState::smooth_normal`]).
    fn smooth_normal(&self, world: &World, state: &SurfaceState) -> [i64; 3] {
        if !self.params.smooth {
            return state.normal;
        }
        let r = self.params.radius;
        let p = add(
            state.centre.0,
            along(state.normal.map(|v| -v), r - FIXED_ONE / 2),
        );
        let voxel = VoxelCoord(p.map(|v| v.div_euclid(FIXED_ONE) as i32));
        let half = (r / FIXED_ONE).max(1) as u32;
        match world.solid_gradient(voxel, half) {
            Ok(g) if g != [0; 3] => normalise(g),
            _ => state.normal,
        }
    }

    /// Advances `state` by one tick of `input` and reports what happened (the module's order).
    pub fn tick(
        &self,
        world: &World,
        state: &mut SurfaceState,
        input: &SurfaceInput,
    ) -> SurfaceResult {
        let p = &self.params;
        let mut r = SurfaceResult::default();
        let start = state.centre.0;
        let mut c = start;
        if sphere_overlaps(world, &self.sphere(c)) {
            let Some(off) = self.depenetrate(world, c) else {
                r.embedded = true;
                return r;
            };
            r.lifted = Fixed3(off);
            c = add(c, Fixed3(off));
        }
        let n0 = state.normal;
        state.forward = tangent(
            turn(state.forward, state.normal, input.heading),
            state.normal,
        );
        if state.attached {
            let speed = input.speed.clamp(0, p.speed_max);
            let s = self.sweep(world, c, along(state.forward, speed));
            r.touch(&s);
            c = add(c, s.moved);
            if let Some(hit) = s.contact {
                let rest = (speed - length(s.moved.0)).max(0);
                let delta = if Self::take_normal(state, hit.normal) {
                    r.event(SurfaceEvent::Concave);
                    along(state.forward, rest)
                } else {
                    // The surface the body stands on: the rounded normal leaves the forward a
                    // fraction of a unit into it. Glide off it by `rest / 64 + 1` along the
                    // normal; the attach step takes the body back.
                    let lift = along(state.normal, rest / 64 + 1);
                    Fixed3(add(along(state.forward, rest).0, lift))
                };
                let s = self.sweep(world, c, delta);
                r.touch(&s);
                c = add(c, s.moved);
            }
            let reach = along(state.normal.map(|v| -v), p.attach_reach);
            let s = self.sweep(world, c, reach);
            r.touch(&s);
            match s.contact {
                Some(hit) => {
                    c = add(c, s.moved);
                    Self::take_normal(state, hit.normal);
                    if state.normal != n0 {
                        r.event(SurfaceEvent::Convex);
                    }
                }
                None => {
                    state.attached = false;
                    state.vz = 0;
                    r.event(SurfaceEvent::Detached);
                }
            }
        }
        if !state.attached {
            state.vz = (state.vz + p.gravity).max(-p.max_fall);
            let s = self.sweep(world, c, Fixed3([0, 0, state.vz]));
            r.touch(&s);
            c = add(c, s.moved);
            if let Some(hit) = s.contact {
                let n = normalise(hit.normal);
                if n != [0; 3] {
                    state.forward = tangent(state.forward, n);
                    state.normal = n;
                }
                state.attached = true;
                r.event(SurfaceEvent::Landed(-state.vz));
                state.vz = 0;
            }
        }
        state.centre = Fixed3(c);
        if state.attached {
            state.smooth_normal = self.smooth_normal(world, state);
        }
        r.moved = Fixed3([0, 1, 2].map(|k| c[k] - start[k]));
        r
    }
}

/// What lies ahead of a body: the parameters `t` of two rays, in the units of the ray's
/// direction (fixed units for a direction of length [`DIR_ONE`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Ahead {
    /// The first solid along `forward` within `reach`.
    pub wall: Option<i64>,
    /// The first solid along `-up` within `drop`, from the point `reach` ahead, or from one
    /// voxel short of the wall's hit when a wall is nearer. `None` is a drop deeper than
    /// `drop`.
    pub floor: Option<i64>,
}

/// Probes ahead of `from` for any steering: a wall along `forward` within `reach`, and the
/// floor under the point ahead within `drop` (two [`World::raycast`]s, no sweep). `forward`
/// and `up` are directions of length [`DIR_ONE`]. A ray the world refuses (past its limits)
/// reads as nothing found.
pub fn probe_ahead(
    world: &World,
    from: Fixed3,
    forward: [i64; 3],
    up: [i64; 3],
    reach: i64,
    drop: i64,
) -> Ahead {
    let wall = world
        .raycast(from, forward, reach)
        .ok()
        .flatten()
        .map(|h| h.t);
    let ahead = wall.map_or(reach, |t| (t - DIR_ONE).max(0));
    let point = Fixed3(add(from.0, along(forward, ahead)));
    let down = up.map(|v| -v);
    let floor = world.raycast(point, down, drop).ok().flatten().map(|h| h.t);
    Ahead { wall, floor }
}
