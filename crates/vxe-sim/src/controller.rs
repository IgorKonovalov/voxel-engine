//! A character controller composed of sweeps in a fixed per-tick order (ADR-0005).
//!
//! One tick: a horizontal move by the wish vector, with a step attempt and up to two slides
//! when a wall blocks it and a probe for a climbable ledge when it stays blocked; then a
//! vertical move under gravity. Every parameter is in fixed units per tick; the caller owns the
//! tick rate. A slope is a staircase of voxels and is climbed as steps.

use vxe_world::{Fixed3, World};

use crate::body::{Body, Contact, isqrt};
use crate::sweep::{Sweep, sweep};

/// Contacts a [`MoveResult`] keeps.
pub const MAX_CONTACTS: usize = 4;

/// Body size and motion limits, in fixed units (1/256 voxel) and fixed units per tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ControllerParams {
    pub radius: i64,
    pub height: i64,
    /// The highest rise a horizontal move climbs without stopping.
    pub step_height: i64,
    /// The highest ledge, above the foot, a climb reaches.
    pub climb_height: i64,
    /// Rise per tick while climbing.
    pub climb_speed: i64,
    /// Added to the vertical speed every airborne tick; negative.
    pub gravity: i64,
    /// The fastest fall, as a positive speed.
    pub max_fall: i64,
    pub jump_speed: i64,
    /// A downward sweep this long that hits a floor makes the body grounded.
    pub ground_probe: i64,
}

/// One tick's input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct MoveInput {
    /// Horizontal motion wanted this tick, in fixed units.
    pub wish: [i64; 2],
    pub jump: bool,
    /// Held to climb a ledge the body is blocked by, and to keep climbing it.
    pub climb: bool,
}

/// A body's motion state between ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CharacterState {
    pub foot: Fixed3,
    /// Vertical speed, fixed units per tick, up positive.
    pub vz: i64,
    pub grounded: bool,
}

impl CharacterState {
    /// At rest at `foot`, not yet grounded; the first tick's probe decides.
    pub fn new(foot: Fixed3) -> Self {
        Self {
            foot,
            vz: 0,
            grounded: false,
        }
    }
}

/// What one tick did.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct MoveResult {
    pub moved: Fixed3,
    pub grounded: bool,
    /// The horizontal move stayed blocked by a wall after the step attempt and the slides.
    pub blocked: bool,
    /// The horizontal move stepped up onto a rise of at most `step_height`.
    pub stepped: bool,
    pub climbed: bool,
    /// Where the foot would land climbing the ledge the body is blocked by: a floor within
    /// `(step_height, climb_height]` above the foot.
    pub climb_available: Option<Fixed3>,
    /// The first [`MAX_CONTACTS`] contacts of the tick's sweeps, in sweep order.
    pub contacts: Vec<Contact>,
    /// The impact speed on the tick the body comes to rest on a floor while falling.
    pub landed: Option<i64>,
}

impl MoveResult {
    fn touch(&mut self, c: Option<Contact>) {
        if let Some(c) = c
            && self.contacts.len() < MAX_CONTACTS
        {
            self.contacts.push(c);
        }
    }
}

fn add(a: [i64; 3], b: Fixed3) -> [i64; 3] {
    [a[0] + b.0[0], a[1] + b.0[1], a[2] + b.0[2]]
}

/// `w - n (w . n) / |n|²` over the horizontal components of `n`, in `i128`, each component
/// rounded toward zero. Zero when `n` has no horizontal part.
fn project_on_wall(w: [i64; 2], n: [i64; 3]) -> [i64; 2] {
    let (n, w) = ([n[0], n[1]].map(i128::from), w.map(i128::from));
    let nn = n[0] * n[0] + n[1] * n[1];
    if nn == 0 {
        return [0, 0];
    }
    let dot = w[0] * n[0] + w[1] * n[1];
    [0, 1].map(|k| (w[k] - n[k] * dot / nn) as i64)
}

/// Whether a sweep stopped on a floor contact.
fn on_floor(s: &Sweep) -> bool {
    s.contact.is_some_and(|c| c.is_floor())
}

/// Moves a body through a world, one tick at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CharacterController {
    pub params: ControllerParams,
}

impl CharacterController {
    pub fn new(params: ControllerParams) -> Self {
        Self { params }
    }

    /// The body with its foot at `foot`.
    pub fn body(&self, foot: [i64; 3]) -> Body {
        Body {
            foot: Fixed3(foot),
            radius: self.params.radius,
            height: self.params.height,
        }
    }

    /// Whether a downward sweep of `ground_probe` from `foot` hits a floor.
    fn probe_floor(&self, world: &World, foot: [i64; 3]) -> bool {
        let s = sweep(
            world,
            &self.body(foot),
            Fixed3([0, 0, -self.params.ground_probe]),
        );
        s.contact.is_some_and(|c| c.is_floor())
    }

    /// Whether a downward sweep that ended at `foot` ends on a floor: it stopped on a floor
    /// contact, or it spent its delta and the ground probe from there hits one (a sweep that
    /// spends its delta reports no contact even when it ends touching, and a fall can end
    /// exactly on the floor).
    fn lands_on_floor(&self, world: &World, s: &Sweep, foot: [i64; 3]) -> bool {
        match s.contact {
            Some(c) => c.is_floor(),
            None => self.probe_floor(world, foot),
        }
    }

    /// Sweep up by `step_height` (fully), forward by `rem` (more than zero), down by
    /// `step_height`; the landing foot when that ends on a floor at most `step_height` up.
    fn step_attempt(&self, world: &World, foot: [i64; 3], rem: [i64; 2]) -> Option<[i64; 3]> {
        let h = self.params.step_height;
        let up = sweep(world, &self.body(foot), Fixed3([0, 0, h]));
        if up.moved != Fixed3([0, 0, h]) {
            return None;
        }
        let f1 = add(foot, up.moved);
        let fw = sweep(world, &self.body(f1), Fixed3([rem[0], rem[1], 0]));
        if fw.moved == Fixed3::default() {
            return None;
        }
        let f2 = add(f1, fw.moved);
        let down = sweep(world, &self.body(f2), Fixed3([0, 0, -h]));
        let f3 = add(f2, down.moved);
        (on_floor(&down) && f3[2] - foot[2] <= h).then_some(f3)
    }

    /// `radius` units horizontally into the wall whose contact normal is `n`.
    fn wall_direction(&self, n: [i64; 3]) -> [i64; 2] {
        let (x, y) = (i128::from(n[0]), i128::from(n[1]));
        let len = isqrt((x * x + y * y) as u128) as i128;
        if len == 0 {
            return [0, 0];
        }
        let r = i128::from(self.params.radius);
        [(-x * r / len) as i64, (-y * r / len) as i64]
    }

    /// Sweep a copy up by `climb_height` (fully), into the wall by `radius` (fully), down by
    /// `climb_height`; the landing foot when that ends on a floor risen by more than
    /// `step_height` and at most `climb_height`.
    fn climb_probe(&self, world: &World, foot: [i64; 3], dir: [i64; 2]) -> Option<[i64; 3]> {
        let p = &self.params;
        let up = sweep(world, &self.body(foot), Fixed3([0, 0, p.climb_height]));
        if up.moved != Fixed3([0, 0, p.climb_height]) {
            return None;
        }
        let f1 = add(foot, up.moved);
        let fw_delta = Fixed3([dir[0], dir[1], 0]);
        let fw = sweep(world, &self.body(f1), fw_delta);
        if fw.moved != fw_delta || dir == [0, 0] {
            return None;
        }
        let f2 = add(f1, fw.moved);
        let down = sweep(world, &self.body(f2), Fixed3([0, 0, -p.climb_height]));
        let f3 = add(f2, down.moved);
        let rise = f3[2] - foot[2];
        (on_floor(&down) && rise > p.step_height && rise <= p.climb_height).then_some(f3)
    }

    /// Advances `state` by one tick of `input` and reports what happened.
    pub fn tick(&self, world: &World, state: &mut CharacterState, input: &MoveInput) -> MoveResult {
        let mut r = MoveResult::default();
        let start = state.foot.0;
        let mut foot = start;
        if input.wish != [0, 0] {
            self.horizontal(world, state, &mut foot, input, &mut r);
        }
        self.vertical(world, state, &mut foot, input, &mut r);
        state.foot = Fixed3(foot);
        r.grounded = state.grounded;
        r.moved = Fixed3([0, 1, 2].map(|k| foot[k] - start[k]));
        r
    }

    fn horizontal(
        &self,
        world: &World,
        state: &mut CharacterState,
        foot: &mut [i64; 3],
        input: &MoveInput,
        r: &mut MoveResult,
    ) {
        let wish = input.wish;
        let s = sweep(world, &self.body(*foot), Fixed3([wish[0], wish[1], 0]));
        r.touch(s.contact);
        *foot = add(*foot, s.moved);
        let mut rem = [wish[0] - s.moved.0[0], wish[1] - s.moved.0[1]];
        let Some(mut wall) = s.contact.filter(|c| c.is_wall()) else {
            return;
        };
        if rem == [0, 0] {
            return;
        }
        if state.grounded
            && let Some(f) = self.step_attempt(world, *foot, rem)
        {
            *foot = f;
            r.stepped = true;
            return;
        }
        for _ in 0..2 {
            let p = project_on_wall(rem, wall.normal);
            if p == [0, 0] {
                break;
            }
            let s = sweep(world, &self.body(*foot), Fixed3([p[0], p[1], 0]));
            r.touch(s.contact);
            *foot = add(*foot, s.moved);
            rem = [p[0] - s.moved.0[0], p[1] - s.moved.0[1]];
            match s.contact {
                Some(c) if c.is_wall() && rem != [0, 0] => wall = c,
                _ => {
                    rem = [0, 0];
                    break;
                }
            }
        }
        if rem == [0, 0] {
            return;
        }
        r.blocked = true;
        let dir = self.wall_direction(wall.normal);
        r.climb_available = self.climb_probe(world, *foot, dir).map(Fixed3);
    }

    fn vertical(
        &self,
        world: &World,
        state: &mut CharacterState,
        foot: &mut [i64; 3],
        input: &MoveInput,
        r: &mut MoveResult,
    ) {
        let p = &self.params;
        state.grounded = state.vz <= 0 && self.probe_floor(world, *foot);
        if state.grounded && input.jump {
            state.vz = p.jump_speed;
            state.grounded = false;
        } else if state.grounded {
            state.vz = 0;
        } else {
            state.vz = (state.vz + p.gravity).max(-p.max_fall);
        }
        if state.vz == 0 {
            return;
        }
        let s = sweep(world, &self.body(*foot), Fixed3([0, 0, state.vz]));
        r.touch(s.contact);
        *foot = add(*foot, s.moved);
        if state.vz < 0 {
            if self.lands_on_floor(world, &s, *foot) {
                state.grounded = true;
                r.landed = Some(-state.vz);
                state.vz = 0;
            }
        } else if s.contact.is_some_and(|c| c.is_ceiling()) {
            state.vz = 0;
        }
    }
}
