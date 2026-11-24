//! The wander: each placed body's input while the menu's `wander` is on, from a SplitMix64
//! stream of its own. Every 300 to 900 ticks it picks a new heading (a turn of up to a
//! quarter circle either way, one pick in [`WIDE_TURN_ONE_IN`] up to half a circle) and turns toward it one 5-degree step a tick while it moves at its
//! body's speed. A ground body probes ahead each tick and turns away from a near wall or a
//! drop, turns round when blocked for 10 ticks and takes a climb one time in eight; a surface
//! body does not avoid (a wall ahead is a concave edge it climbs, a drop a convex edge it
//! rounds). Everything is an integer, so a seed gives the same walk on every platform.

use svet_sim::{
    Ahead, CharacterController, CharacterState, MoveInput, MoveResult, SurfaceInput, normalise,
    probe_ahead,
};
use svet_world::{Fixed3, World};

/// A ground body's speed: 43 fixed units a tick, 1 m/s at 60 Hz (`2560 / 60 = 42.7`).
pub const GROUND_SPEED: i64 = 43;
/// A surface body's speed: 64 fixed units a tick, 1.5 m/s at 60 Hz.
pub const SURFACE_SPEED: i64 = 64;
/// One turn step, about 5 degrees, as a Q8 `(cos, sin)` pair: `(255, 22)` or its mirror.
pub const TURN_STEP: [i32; 2] = [255, 22];
/// The turn steps in a half circle.
pub const HALF_TURN_STEPS: i32 = 36;
/// The turn steps in a quarter circle: the most a picked heading turns, but for the wide
/// picks.
pub const QUARTER_TURN_STEPS: i32 = 18;
/// One pick of a heading in this many may turn up to a half circle.
pub const WIDE_TURN_ONE_IN: u64 = 8;
/// The shortest and longest ticks between two picks of a heading: 5 to 15 s at 60 Hz.
pub const PICK_TICKS: (u32, u32) = (300, 900);
/// How far a ground body probes ahead: 20 voxels.
pub const PROBE_REACH: i64 = 20 * 256;
/// A wall nearer than this ahead (10 voxels) turns a ground body away.
pub const WALL_NEAR: i64 = 10 * 256;
/// Ticks blocked before a ground body turns round.
pub const BLOCKED_TICKS: u32 = 10;
/// One climb offered in this many is taken.
pub const CLIMB_ONE_IN: u64 = 8;

/// One body's wander: its random stream, the turn steps still to take (signed, positive
/// counter-clockwise seen from above the surface), the ticks until the next pick, the ticks it
/// has been blocked, and whether it holds a climb.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Wander {
    state: u64,
    pub turn: i32,
    pub next_pick: u32,
    pub blocked: u32,
    pub climbing: bool,
}

impl Wander {
    /// The stream of placement `index` under the run's `seed`; the first pick at once.
    pub fn new(seed: u64, index: u64) -> Self {
        let mut w = Self {
            state: seed ^ index.wrapping_mul(0xD1B5_4A32_D192_ED03),
            turn: 0,
            next_pick: 0,
            blocked: 0,
            climbing: false,
        };
        // One draw mixes the seed and the index before the first pick.
        w.draw();
        w
    }

    /// SplitMix64's next value.
    pub fn draw(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A value in `lo..=hi`.
    fn between(&mut self, lo: u32, hi: u32) -> u32 {
        lo + (self.draw() % u64::from(hi - lo + 1)) as u32
    }

    /// A turn of `lo..=hi` steps, either way.
    fn pick_turn(&mut self, lo: u32, hi: u32) {
        let steps = self.between(lo, hi) as i32;
        self.turn = if self.draw() & 1 == 0 { steps } else { -steps };
    }

    /// This tick's turn about the up: a new heading when the pick is due (up to a quarter
    /// circle, one pick in [`WIDE_TURN_ONE_IN`] up to a half), then one step
    /// toward it, or straight on once there.
    pub fn heading(&mut self) -> [i32; 2] {
        if self.next_pick == 0 {
            let most = if self.draw().is_multiple_of(WIDE_TURN_ONE_IN) {
                HALF_TURN_STEPS
            } else {
                QUARTER_TURN_STEPS
            };
            self.pick_turn(0, most as u32);
            self.next_pick = self.between(PICK_TICKS.0, PICK_TICKS.1);
        }
        self.next_pick -= 1;
        match self.turn.signum() {
            1 => {
                self.turn -= 1;
                TURN_STEP
            }
            -1 => {
                self.turn += 1;
                [TURN_STEP[0], -TURN_STEP[1]]
            }
            _ => [256, 0],
        }
    }

    /// A surface body's input this tick.
    pub fn surface_input(&mut self) -> SurfaceInput {
        SurfaceInput {
            heading: self.heading(),
            speed: SURFACE_SPEED,
        }
    }

    /// A ground body's input this tick: `forward` (level, length 256) turned by the heading,
    /// the probe ahead from the foot lifted half a voxel (a wall within [`WALL_NEAR`] or no
    /// floor within the step height turns it 90 to 180 degrees away when it is not turning
    /// already), the wish along it at [`GROUND_SPEED`] and the climb while one is held.
    pub fn ground_input(
        &mut self,
        world: &World,
        ctl: &CharacterController,
        state: &CharacterState,
        forward: &mut [i64; 3],
    ) -> MoveInput {
        *forward = turn_level(*forward, self.heading());
        let f = state.foot.0;
        let from = Fixed3([f[0], f[1], f[2] + 128]);
        let Ahead { wall, floor } = probe_ahead(
            world,
            from,
            *forward,
            [0, 0, 256],
            PROBE_REACH,
            ctl.params.step_height,
        );
        if self.turn == 0 && (wall.is_some_and(|t| t < WALL_NEAR) || floor.is_none()) {
            self.pick_turn(HALF_TURN_STEPS as u32 / 2, HALF_TURN_STEPS as u32);
        }
        MoveInput {
            wish: [
                forward[0] * GROUND_SPEED / 256,
                forward[1] * GROUND_SPEED / 256,
            ],
            jump: false,
            climb: self.climbing,
        }
    }

    /// After a ground tick: blocked for [`BLOCKED_TICKS`] turns it round; a climb offered is
    /// taken one time in [`CLIMB_ONE_IN`] and held until the climb ends.
    pub fn after_ground(&mut self, r: &MoveResult, state: &CharacterState) {
        if r.blocked && !state.is_climbing() {
            self.blocked += 1;
            if self.blocked >= BLOCKED_TICKS {
                self.blocked = 0;
                self.turn = HALF_TURN_STEPS;
            }
        } else {
            self.blocked = 0;
        }
        self.climbing = state.is_climbing()
            || (r.climb_available.is_some() && self.draw().is_multiple_of(CLIMB_ONE_IN));
    }
}

/// A level direction of length 256 turned about `+z` by the Q8 `(cos, sin)` pair `h`, kept
/// at length 256; `-y` for a zero direction.
pub fn turn_level(f: [i64; 3], h: [i32; 2]) -> [i64; 3] {
    let (c, s) = (i64::from(h[0]), i64::from(h[1]));
    let t = [f[0] * c - f[1] * s, f[0] * s + f[1] * c, 0];
    match normalise(t) {
        [0, 0, 0] => [0, -256, 0],
        n => n,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One stream per seed and index: the same pair gives the same draws, another index
    /// other draws; the picks stay within their ticks, their turns within a half circle, and
    /// most of them within a quarter.
    #[test]
    fn a_stream_is_its_seed_and_index() {
        let draws = |seed, index| {
            let mut w = Wander::new(seed, index);
            (0..8).map(|_| w.draw()).collect::<Vec<_>>()
        };
        assert_eq!(draws(1, 3), draws(1, 3));
        assert_ne!(draws(1, 3), draws(1, 4));
        assert_ne!(draws(1, 3), draws(2, 3));
        let mut w = Wander::new(7, 0);
        let (mut picks, mut wide) = (0, 0);
        for _ in 0..400_000 {
            let at_pick = w.next_pick == 0;
            w.heading();
            assert!(w.turn.abs() <= HALF_TURN_STEPS);
            if at_pick {
                assert!((PICK_TICKS.0 - 1..PICK_TICKS.1).contains(&w.next_pick));
                picks += 1;
                // The step taken this tick is off the picked turn.
                wide += i32::from(w.turn.abs() + 1 > QUARTER_TURN_STEPS);
            }
        }
        assert!(picks > 500, "{picks}");
        assert!(wide > 0 && wide * 6 < picks, "{wide} of {picks}");
    }

    /// A turn step keeps a level direction at length 256 within a unit and turns it about 5
    /// degrees counter-clockwise seen from above.
    #[test]
    fn a_turn_step_turns_five_degrees() {
        let f = turn_level([0, -256, 0], TURN_STEP);
        assert_eq!(f[2], 0);
        assert!(f[0] > 0, "{f:?}");
        let deg = (f[0] as f64).atan2(-f[1] as f64).to_degrees();
        assert!((deg - 4.93).abs() < 0.3, "{deg}");
        let mut g = [0, -256, 0];
        for _ in 0..72 {
            g = turn_level(g, TURN_STEP);
        }
        let len = ((g[0] * g[0] + g[1] * g[1]) as f64).sqrt();
        assert!((len - 256.0).abs() <= 1.0, "{g:?}");
    }
}
