//! Shared fixtures for the `vxe-sim` integration tests: six closure worlds over chunks
//! `[-2, 2)³`, material 1 solid, and the test body.
#![allow(dead_code, reason = "shared across test binaries; each uses a subset")]

use vxe_sim::{
    Body, CharacterController, CharacterState, ControllerParams, MoveInput, MoveResult, overlaps,
};
use vxe_world::*;

/// The solid material.
pub const ROCK: MaterialId = 1;
/// The default start: voxel 10's centre in plan, resting on the F1 floor.
pub const S: [i64; 3] = [2688, 2688, 0];
pub const RADIUS: i64 = 768;
pub const HEIGHT: i64 = 4608;

pub fn body_at(foot: [i64; 3]) -> Body {
    Body {
        foot: Fixed3(foot),
        radius: RADIUS,
        height: HEIGHT,
    }
}

pub fn table() -> MaterialTable {
    let mut t = MaterialTable::new();
    assert_eq!(t.push(Material::named("rock")), ROCK);
    t
}

/// A source evaluated per voxel from a solidity predicate; bricks collapse when uniform.
pub struct Solid<F>(pub F);

impl<F: Fn([i32; 3]) -> bool> WorldSource for Solid<F> {
    fn chunk(&self, coord: ChunkCoord, out: &mut Chunk) {
        let base = coord.min_voxel().0;
        for slot in 0..CHUNK_BRICKS {
            let b = brick_of_slot(slot);
            out.set_brick(
                slot,
                Brick::from_fn(|l| {
                    let v = [0, 1, 2].map(|k| base[k] + b[k] * BRICK_EDGE + l[k]);
                    if (self.0)(v) { ROCK } else { AIR }
                }),
            );
        }
    }
}

pub fn world(f: impl Fn([i32; 3]) -> bool + Sync) -> World {
    World::generate(
        ChunkExtent::new([-2, -2, -2], [2, 2, 2]),
        &Solid(f),
        &table(),
    )
}

/// F1 floor: solid iff `z < 0`.
pub fn f1() -> World {
    world(|[_, _, z]| z < 0)
}

/// F2 wall: F1 plus `x in [20, 22)`, `z in [0, 40)`, all `y`.
pub fn f2() -> World {
    world(|[x, _, z]| z < 0 || ((20..22).contains(&x) && (0..40).contains(&z)))
}

/// F3 step: F1 plus `x in [30, 40)`, `z in [0, 4)`.
pub fn f3() -> World {
    world(|[x, _, z]| z < 0 || ((30..40).contains(&x) && (0..4).contains(&z)))
}

/// F4 tall step: F1 plus `x in [30, 50)`, `z in [0, 5)`.
pub fn f4() -> World {
    world(|[x, _, z]| z < 0 || ((30..50).contains(&x) && (0..5).contains(&z)))
}

/// F5 pit: F1 with air where `x, y in [50, 58)`, `z in [-8, 0)`.
pub fn f5() -> World {
    world(|[x, y, z]| z < 0 && !((50..58).contains(&x) && (50..58).contains(&y) && z >= -8))
}
pub const F5_START: [i64; 3] = [13952, 13952, -2048];

/// F6 staircase: F1 plus `z < x - 60` for `60 <= x < 80`, and `z < 20` for `x >= 80`.
pub fn f6() -> World {
    world(|[x, _, z]| z < 0 || ((60..80).contains(&x) && z < x - 60) || (x >= 80 && z < 20))
}
pub const F6_START: [i64; 3] = [12928, 2688, 0];

/// The plan's test parameters.
pub fn params() -> ControllerParams {
    ControllerParams {
        radius: RADIUS,
        height: HEIGHT,
        step_height: 1024,
        climb_height: 3072,
        climb_speed: 64,
        gravity: -8,
        max_fall: 1024,
        jump_speed: 160,
        ground_probe: 2,
    }
}

/// 0.5 voxel per tick along +x: 3 m/s at 60 Hz.
pub const WISH: [i64; 2] = [128, 0];

/// One tick's record: the result and the state after it.
pub struct Step {
    pub r: MoveResult,
    pub s: CharacterState,
}

/// Runs `ticks` ticks from `start`, `input(tick)` giving each tick's input (ticks count from
/// 1), and asserts the body never overlaps the world after a tick.
pub fn run(
    world: &World,
    start: [i64; 3],
    ticks: u32,
    mut input: impl FnMut(u32) -> MoveInput,
) -> Vec<Step> {
    let cc = CharacterController::new(params());
    let mut s = CharacterState::new(Fixed3(start));
    let mut out = Vec::new();
    for t in 1..=ticks {
        let r = cc.tick(world, &mut s, &input(t));
        assert!(
            !overlaps(world, &body_at(s.foot.0)),
            "tick {t}: overlapping at {:?}",
            s.foot
        );
        out.push(Step { r, s });
    }
    out
}

pub fn walk(wish: [i64; 2]) -> impl FnMut(u32) -> MoveInput {
    move |_| MoveInput {
        wish,
        ..MoveInput::default()
    }
}
