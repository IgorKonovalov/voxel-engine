//! A placed body's mover (ADR-0062): a rig with a `body` line moves through the world at the
//! movers' tick, a ground body through `vxe_sim`'s character controller and a surface body
//! through its surface mover, and its pose follows the state. Ground: the root is the foot,
//! turned to face the forward with `+z` up. Surface: the root is the contact point (the
//! centre moved `radius` against the normal), turned to face the forward with the smoothed
//! normal up. While the wander is on a moving body's `walk` clip advances by the distance it
//! moved (`distance * ticks / stride`, wrapping; by time for a stride of 0) and a body still
//! for [`STILL_TICKS`] has faded into `idle` (by time), and back into `walk` when it moves;
//! while the wander is off its playback's clip plays by time. A mover keeps its last two
//! poses, so the frame draws between ticks.

use vxe_assets::RigBody;
use vxe_entity::{Clip, Pose, Quat, blend, sample};
use vxe_sim::{
    CharacterController, CharacterState, ControllerParams, MoveInput, SurfaceEvent, SurfaceMover,
    SurfaceParams, SurfaceState, isqrt, normalise,
};
use vxe_world::{Fixed3, World};

use super::wander::{SURFACE_SPEED, Wander};
use crate::viewer::{Loaded, Play};

/// Ticks a body stands still before it has faded into `idle`; the fades between `walk` and
/// `idle` take as long.
pub const STILL_TICKS: u32 = 8;
/// The movers' ticks a second.
pub const TICK_HZ: u32 = 60;

/// The body's motion state and the sim that moves it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MoverBody {
    Ground {
        ctl: CharacterController,
        state: CharacterState,
        /// The facing, level, length 256: the last non-zero wish.
        forward: [i64; 3],
    },
    Surface {
        mover: SurfaceMover,
        state: SurfaceState,
    },
}

/// What one tick did to a body.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MoverStep {
    pub moved: Fixed3,
    pub embedded: bool,
    /// Grounded (ground) or attached (surface) after the tick.
    pub on_surface: bool,
    /// A ground body's landing speed this tick.
    pub landed: Option<i64>,
    /// A surface body's event this tick.
    pub event: Option<SurfaceEvent>,
}

/// A placed body's mover: its body, its wander, the distance it has moved on its surface
/// (the walk clip's clock), the ticks it has stood still and moved since the last change, the
/// ticks ticked (the time clips' clock), the pose a fade starts from, and the poses of the last
/// two ticks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mover {
    pub body: MoverBody,
    pub wander: Wander,
    pub distance: i64,
    pub still: u32,
    pub moving: u32,
    pub ticks: u64,
    /// The parts' pose (root at the origin) a fade starts from.
    from: Option<Pose>,
    /// The parts' pose (root at the origin) the last tick drew.
    parts: Pose,
    pub prev: Pose,
    pub now: Pose,
}

/// The controller's params for a ground body of `radius`, `height` and `step`: the rest
/// (climb, gravity, the fastest fall, the ground probe) the fly-through's walk mode's.
pub fn ground_params(radius: i64, height: i64, step: i64) -> ControllerParams {
    ControllerParams {
        radius,
        height,
        step_height: step,
        ..crate::fly::walk_params()
    }
}

/// The surface mover's params for a surface body of `radius` and `reach`: the wander's speed,
/// the walk mode's gravity and fastest fall, the smoothed normal on.
pub fn surface_params(radius: i64, reach: i64) -> SurfaceParams {
    let w = crate::fly::walk_params();
    SurfaceParams {
        radius,
        attach_reach: reach,
        speed_max: SURFACE_SPEED,
        gravity: w.gravity,
        max_fall: w.max_fall,
        smooth: true,
    }
}

/// World voxels to fixed units, rounded.
pub fn fixed(v: [f32; 3]) -> [i64; 3] {
    v.map(|c| (c * 256.0).round() as i64)
}

/// The clip named `name` in `l`.
fn clip<'a>(l: &'a Loaded, name: &str) -> Option<&'a Clip> {
    l.clips.iter().find(|(n, _)| n == name).map(|(_, c)| &**c)
}

/// The walk clip's tick after `distance` fixed units moved: `distance * length / stride`
/// (wrapped or held by [`Clip::clip_tick`] when sampled); `None` for a stride of 0.
pub fn walk_tick(distance: i64, clip: &Clip) -> Option<u64> {
    let stride = i64::from(clip.stride());
    (stride > 0).then(|| (distance.max(0) * i64::from(clip.length()) / stride) as u64)
}

impl Mover {
    /// `body` standing at `point` (world voxels) on the face whose outward normal is `up`,
    /// facing `forward`: a ground body with its foot at the point, grounded, its forward
    /// levelled; a surface body with its centre `radius` out along the up, attached to it.
    /// Wandering on placement `index`'s stream of `seed`; posed from `parts` (root at the
    /// origin).
    pub fn new(
        body: RigBody,
        point: [f32; 3],
        up: [f32; 3],
        forward: [f32; 3],
        (seed, index): (u64, u64),
        parts: Pose,
    ) -> Self {
        let p = fixed(point);
        let f = fixed(forward);
        let body = match body {
            RigBody::Ground {
                radius,
                height,
                step,
            } => {
                let mut state = CharacterState::new(Fixed3(p));
                state.grounded = true;
                let level = normalise([f[0], f[1], 0]);
                MoverBody::Ground {
                    ctl: CharacterController::new(ground_params(radius, height, step)),
                    state,
                    forward: if level == [0; 3] { [0, -256, 0] } else { level },
                }
            }
            RigBody::Surface { radius, reach } => {
                let n = normalise(fixed(up));
                let n = if n == [0; 3] { [0, 0, 256] } else { n };
                let centre = [0, 1, 2].map(|k| p[k] + n[k] * radius / 256);
                MoverBody::Surface {
                    mover: SurfaceMover::new(surface_params(radius, reach)),
                    state: SurfaceState::new(Fixed3(centre), n, f),
                }
            }
        };
        let mut m = Self {
            body,
            wander: Wander::new(seed, index),
            distance: 0,
            still: 0,
            moving: 0,
            ticks: 0,
            from: None,
            parts: parts.clone(),
            prev: parts.clone(),
            now: parts,
        };
        m.now = m.rooted(&m.parts);
        m.prev = m.now.clone();
        m
    }

    /// The root and its turn from the state (the module's rule).
    pub fn root(&self) -> (Fixed3, Quat) {
        match &self.body {
            MoverBody::Ground { state, forward, .. } => (
                state.foot,
                Quat::from_frame(*forward, [0, 0, 256]).unwrap_or(Quat::IDENTITY),
            ),
            MoverBody::Surface { mover, state } => {
                let (c, n, r) = (state.centre.0, state.normal, mover.params.radius);
                let len = (isqrt((0..3).map(|k| (n[k] * n[k]) as u128).sum()) as i64).max(1);
                let root = Fixed3([0, 1, 2].map(|k| c[k] - n[k] * r / len));
                let q = Quat::from_frame(state.forward, state.smooth_normal)
                    .or_else(|| Quat::from_frame(state.forward, state.normal))
                    .unwrap_or(Quat::IDENTITY);
                (root, q)
            }
        }
    }

    /// `parts` (root at the origin) placed at the state's root.
    fn rooted(&self, parts: &Pose) -> Pose {
        let (root, root_q) = self.root();
        Pose {
            root,
            root_q,
            parts: parts.parts.clone(),
        }
    }

    /// The poses re-based on `parts` (a reload's model): both ticks' poses and the fade's
    /// start dropped, since the new model may have other parts.
    pub fn rebase(&mut self, parts: Pose) {
        self.from = None;
        self.parts = parts;
        self.now = self.rooted(&self.parts);
        self.prev = self.now.clone();
    }

    /// The pose of the state now on `parts` (root at the origin), as the last tick's too: the
    /// ghost's, which never ticks.
    pub fn hold(&mut self, parts: Pose) {
        self.rebase(parts);
    }

    /// One tick against `world`: the wander's input when `wandering`, else none; then the
    /// distance and the stillness, the parts from `rig` (the walk and idle clips by the
    /// module's rule while wandering, else `play`'s clip by time), and the two poses.
    pub fn tick(&mut self, world: &World, wandering: bool, rig: (&Loaded, &Play)) -> MoverStep {
        let mut out = MoverStep::default();
        match &mut self.body {
            MoverBody::Ground {
                ctl,
                state,
                forward,
            } => {
                let input = if wandering {
                    self.wander.ground_input(world, ctl, state, forward)
                } else {
                    MoveInput::default()
                };
                let r = ctl.tick(world, state, &input);
                if wandering {
                    self.wander.after_ground(&r, state);
                }
                out.moved = r.moved;
                out.embedded = r.embedded;
                out.on_surface = state.grounded;
                out.landed = r.landed;
            }
            MoverBody::Surface { mover, state } => {
                let input = if wandering {
                    self.wander.surface_input()
                } else {
                    Default::default()
                };
                let r = mover.tick(world, state, &input);
                out.moved = r.moved;
                out.embedded = r.embedded;
                out.on_surface = state.attached;
                out.event = r.event;
            }
        }
        let m = out.moved.0;
        let len = isqrt((0..3).map(|k| (m[k] * m[k]) as u128).sum()) as i64;
        self.animate(len, out.on_surface, wandering, rig);
        out
    }

    /// The parts' pose (root at the origin) the last tick drew.
    pub fn parts(&self) -> &Pose {
        &self.parts
    }

    /// The pose after a tick that moved `len` (on its surface or not): the parts by the
    /// module's rule, placed at the state's root, the last tick's pose kept as the previous.
    pub fn animate(&mut self, len: i64, on_surface: bool, wandering: bool, rig: (&Loaded, &Play)) {
        self.ticks += 1;
        self.parts = self.parts_after(len, on_surface, wandering, rig);
        let now = self.rooted(&self.parts);
        self.prev = std::mem::replace(&mut self.now, now);
    }

    /// The parts' pose after a tick that moved `len` (on its surface or not).
    fn parts_after(
        &mut self,
        len: i64,
        on_surface: bool,
        wandering: bool,
        (l, play): (&Loaded, &Play),
    ) -> Pose {
        if !wandering {
            (self.still, self.moving, self.from) = (0, 0, None);
            return play.poses(l).0;
        }
        let rate_ticks = |t: u64| t * u64::from(l.rate) / u64::from(TICK_HZ);
        if len > 0 {
            if on_surface {
                self.distance += len;
            }
            if self.still > 0 {
                (self.still, self.moving) = (0, 0);
                self.from = Some(self.parts.clone());
            }
            self.moving += 1;
            let walk = match clip(l, "walk") {
                Some(c) => {
                    let t = walk_tick(self.distance, c).unwrap_or_else(|| rate_ticks(self.ticks));
                    sample(&l.model, &l.rest, c, t)
                }
                None => l.rest.clone(),
            };
            return self.faded(walk, self.moving);
        }
        if self.still == 0 {
            self.moving = 0;
            self.from = Some(self.parts.clone());
        }
        self.still += 1;
        let Some(idle) = clip(l, "idle") else {
            // No idle: the last pose held.
            self.from = None;
            return self.parts.clone();
        };
        let t = rate_ticks(u64::from(self.still.saturating_sub(STILL_TICKS)));
        let idle = sample(&l.model, &l.rest, idle, t);
        self.faded(idle, self.still)
    }

    /// `to` faded in from the fade's start over [`STILL_TICKS`]: weight `32 k` at the fade's
    /// `k`-th tick, `to` itself from the 8th.
    fn faded(&mut self, to: Pose, k: u32) -> Pose {
        match &self.from {
            Some(from) if k < STILL_TICKS && from.parts.len() == to.parts.len() => {
                blend(from, &to, (k * 256 / STILL_TICKS) as u16)
            }
            _ => {
                self.from = None;
                to
            }
        }
    }

    /// The surface body's state, when it is one.
    pub fn surface(&self) -> Option<&SurfaceState> {
        match &self.body {
            MoverBody::Surface { state, .. } => Some(state),
            MoverBody::Ground { .. } => None,
        }
    }

    /// The ground body's state, when it is one.
    pub fn ground(&self) -> Option<&CharacterState> {
        match &self.body {
            MoverBody::Ground { state, .. } => Some(state),
            MoverBody::Surface { .. } => None,
        }
    }
}

/// Where and how a placement stands: the point on its face (world voxels), the face's outward
/// normal (unit; `+z` on a floor), the forward (unit, along the face) and the turn recorded
/// as the placement's yaw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stance {
    pub point: [f32; 3],
    pub up: [f32; 3],
    pub forward: [f32; 3],
    pub yaw: f32,
}

impl Stance {
    /// On the floor at `feet`, turned `yaw` from facing `-y` about `+z`.
    pub fn floor(feet: [f32; 3], yaw: f32) -> Self {
        Self {
            point: feet,
            up: [0.0, 0.0, 1.0],
            forward: [yaw.sin(), -yaw.cos(), 0.0],
            yaw,
        }
    }

    /// On the face at `point` with outward normal `up`, facing the eye at `eye` along the face
    /// (the eye's direction projected onto the face; `cam_up` projected when that is nought),
    /// turned `turn` radians about the up. On an up face it is [`Stance::floor`] facing the
    /// eye, turned.
    pub fn facing(
        point: [f32; 3],
        up: [f32; 3],
        eye: [f32; 3],
        cam_up: [f32; 3],
        turn: f32,
    ) -> Self {
        if up[2] > 0.5 {
            return Self::floor(point, crate::walker::facing(point, eye) + turn);
        }
        let dot = |a: [f32; 3], b: [f32; 3]| (0..3).map(|k| a[k] * b[k]).sum::<f32>();
        let along = |v: [f32; 3]| {
            let d = dot(v, up);
            let t: [f32; 3] = std::array::from_fn(|k| v[k] - d * up[k]);
            let len = dot(t, t).sqrt();
            (len > 1e-3).then(|| t.map(|c| c / len))
        };
        let to_eye = std::array::from_fn(|k| eye[k] - point[k]);
        let f = along(to_eye)
            .or_else(|| along(cam_up))
            .unwrap_or([0.0, 0.0, 1.0]);
        // Turned about the up: f cos t + (up x f) sin t.
        let x = [
            up[1] * f[2] - up[2] * f[1],
            up[2] * f[0] - up[0] * f[2],
            up[0] * f[1] - up[1] * f[0],
        ];
        let (s, c) = turn.sin_cos();
        Self {
            point,
            up,
            forward: std::array::from_fn(|k| f[k] * c + x[k] * s),
            yaw: turn,
        }
    }
}
