//! The controller's walk, step, slide, fall and jump contracts on the F1 to F6 fixtures.
//! `run` asserts the body never overlaps the world after any tick.

mod support;

use support::*;
use vxe_sim::*;
use vxe_world::*;

/// Ticks (counting from 1) on which `f` holds.
fn ticks_where(steps: &[Step], f: impl Fn(&Step) -> bool) -> Vec<u32> {
    (1..)
        .zip(steps)
        .filter(|(_, s)| f(s))
        .map(|(t, _)| t)
        .collect()
}

fn foot(steps: &[Step], tick: u32) -> [i64; 3] {
    steps[tick as usize - 1].s.foot.0
}

#[test]
fn walking_on_a_floor_moves_half_a_voxel_per_tick() {
    let steps = run(&f1(), S, 60, walk(WISH));
    assert_eq!(foot(&steps, 60), [10368, 2688, 0]);
    for (t, st) in (1..).zip(&steps) {
        assert!(st.r.grounded && st.s.grounded, "tick {t}");
        assert_eq!(st.s.vz, 0, "tick {t}");
        assert_eq!(st.r.landed, None, "tick {t}");
        assert!(!st.r.blocked && !st.r.stepped, "tick {t}");
    }
}

#[test]
fn a_wall_blocks_the_walk_at_its_face() {
    let steps = run(&f2(), S, 60, walk(WISH));
    for t in 1..=13 {
        assert_eq!(
            foot(&steps, t),
            [2688 + 128 * i64::from(t), 2688, 0],
            "tick {t}"
        );
        assert!(!steps[t as usize - 1].r.blocked, "tick {t}");
    }
    assert_eq!(foot(&steps, 13), [4352, 2688, 0]);
    for t in 14..=60 {
        let st = &steps[t as usize - 1];
        assert!(st.r.blocked, "tick {t}");
        assert_eq!(st.s.foot.0, [4352, 2688, 0], "tick {t}");
    }
    assert!(ticks_where(&steps, |s| s.r.stepped).is_empty());
}

#[test]
fn a_diagonal_walk_slides_along_the_wall() {
    let steps = run(&f2(), S, 60, walk([128, 128]));
    let [x, y, z] = foot(&steps, 60);
    assert_eq!(x, 4352);
    assert!(y > 2688 + 40 * 128, "y = {y}");
    assert_eq!(z, 0);
}

#[test]
fn a_one_voxel_rise_is_stepped_onto_without_stopping() {
    let steps = run(&f3(), S, 60, walk(WISH));
    assert_eq!(ticks_where(&steps, |s| s.r.stepped), [34]);
    assert_eq!(foot(&steps, 34), [7040, 2688, 1024]);
    assert_eq!(foot(&steps, 60), [10368, 2688, 1024]);
    assert!(steps[59].s.grounded);
}

#[test]
fn a_tall_step_blocks_and_offers_a_climb() {
    let steps = run(&f4(), S, 60, walk(WISH));
    assert!(
        ticks_where(&steps, |s| s.r.blocked)
            .iter()
            .all(|&t| t >= 34)
    );
    for t in 34..=60 {
        let st = &steps[t as usize - 1];
        assert!(st.r.blocked, "tick {t}");
        assert_eq!(st.s.foot.0, [6912, 2688, 0], "tick {t}");
        assert_eq!(
            st.r.climb_available,
            Some(Fixed3([7680, 2688, 1280])),
            "tick {t}"
        );
    }
    assert!(ticks_where(&steps, |s| s.r.stepped).is_empty());
}

#[test]
fn a_staircase_is_climbed_as_steps() {
    let steps = run(&f6(), F6_START, 60, walk(WISH));
    let expected: Vec<u32> = (16..=54).step_by(2).collect();
    assert_eq!(expected.len(), 20);
    assert_eq!(ticks_where(&steps, |s| s.r.stepped), expected);
    assert_eq!(foot(&steps, 60), [20608, 2688, 5120]);
    assert!(steps[59].s.grounded);
}

#[test]
fn a_jump_rises_and_lands_back_on_the_floor() {
    let steps = run(&f1(), S, 60, |t| MoveInput {
        jump: t == 1,
        ..MoveInput::default()
    });
    assert!(foot(&steps, 1)[2] > 0, "left the floor on tick 1");
    let top = steps.iter().map(|s| s.s.foot.0[2]).max().unwrap();
    assert!((1500..=1700).contains(&top), "max z {top}");
    assert_eq!(top, 1680);
    assert_eq!(foot(&steps, 21)[2], 1680);
    let back = ticks_where(&steps, |s| s.s.grounded && s.s.foot.0[2] == 0);
    assert!(
        back.iter().any(|&t| t > 1 && t <= 45),
        "grounded again by tick 45"
    );
    assert!((41..=60).all(|t| foot(&steps, t)[2] == 0 && steps[t as usize - 1].s.grounded));
    assert_eq!(ticks_where(&steps, |s| s.r.landed.is_some()), [41]);
    assert_eq!(steps[40].r.landed, Some(160));
}

#[test]
fn a_long_fall_is_capped_and_lands_once() {
    let steps = run(&f1(), [2688, 2688, 25600], 100, |_| MoveInput::default());
    for (t, st) in (1..).zip(&steps) {
        assert!(st.s.vz.abs() <= params().max_fall, "tick {t}");
        let n = i64::from(t);
        if t < 80 {
            assert_eq!(st.s.foot.0[2], 25600 - 4 * n * (n + 1), "tick {t}");
        } else {
            assert_eq!(st.s.foot.0[2], 0, "tick {t}");
            assert!(st.s.grounded, "tick {t}");
        }
    }
    assert_eq!(foot(&steps, 79)[2], 320);
    assert_eq!(ticks_where(&steps, |s| s.r.landed.is_some()), [80]);
    assert_eq!(steps[79].r.landed, Some(640));
}

fn climb_held(wish: [i64; 2], release_on: u32) -> impl FnMut(u32) -> MoveInput {
    move |t| MoveInput {
        wish,
        jump: false,
        climb: t < release_on,
    }
}

#[test]
fn a_tall_step_is_climbed_while_climb_is_held() {
    let steps = run(&f4(), S, 60, climb_held(WISH, u32::MAX));
    let st34 = &steps[33];
    assert!(st34.r.blocked);
    assert!(st34.s.is_climbing(), "climbing begins on tick 34");
    assert_eq!(st34.s.foot.0, [6912, 2688, 64]);
    assert!(
        ticks_where(&steps, |s| s.r.blocked)
            .iter()
            .all(|&t| t >= 34)
    );
    assert_eq!(ticks_where(&steps, |s| s.r.climbed), [53]);
    assert_eq!(
        ticks_where(&steps, |s| s.s.is_climbing()),
        (34..=52).collect::<Vec<_>>()
    );
    assert_eq!(foot(&steps, 53), [7680, 2688, 1280]);
    assert_eq!(foot(&steps, 60), [8576, 2688, 1280]);
    assert!(steps[59].s.grounded);
}

#[test]
fn a_dug_pit_is_climbed_out_of() {
    let steps = run(&f5(), F5_START, 60, climb_held(WISH, u32::MAX));
    assert_eq!(foot(&steps, 1), [14080, 13952, -2048]);
    assert!(!steps[0].r.blocked);
    assert!(steps[1].r.blocked);
    assert!(steps[1].s.is_climbing(), "tick 2 begins the climb");
    assert_eq!(ticks_where(&steps, |s| s.r.climbed), [33]);
    assert_eq!(foot(&steps, 33), [14848, 13952, 0]);
    assert_eq!(foot(&steps, 60), [18304, 13952, 0]);
    assert!(steps[59].s.grounded);
}

#[test]
fn releasing_climb_drops_the_body_back_down() {
    let steps = run(&f4(), S, 60, climb_held(WISH, 40));
    assert!(steps[38].s.is_climbing());
    assert_eq!(foot(&steps, 39)[2], 6 * 64);
    assert!(
        ticks_where(&steps, |s| s.s.is_climbing())
            .iter()
            .all(|&t| t < 40)
    );
    assert!(ticks_where(&steps, |s| s.r.climbed).is_empty());
    let st60 = &steps[59];
    assert_eq!(st60.s.foot.0, [6912, 2688, 0]);
    assert!(st60.s.grounded);
    let landed = ticks_where(&steps, |s| s.r.landed.is_some());
    assert_eq!(landed.len(), 1);
    let land = landed[0];
    assert!(land > 40 && land <= 60);
    for t in land..=60 {
        let st = &steps[t as usize - 1];
        assert_eq!(st.s.foot.0, [6912, 2688, 0], "tick {t}");
        assert_eq!(
            st.r.climb_available,
            Some(Fixed3([7680, 2688, 1280])),
            "tick {t}"
        );
    }
}

/// A block one voxel tall, `x, y in [20, 30)`, on the F1 floor: its `(-x, -y)` corner sits at
/// `(5120, 5120)`, and a step attempt's forward move at `step_height` clears it.
fn low_block() -> World {
    world(|[x, y, z]| z < 0 || ((20..30).contains(&x) && (20..30).contains(&y) && z == 0))
}

/// One grounded tick of `wish` from `start`.
fn one_tick(world: &World, start: [i64; 3], wish: [i64; 2]) -> (MoveResult, CharacterState) {
    let cc = CharacterController::new(params());
    let mut s = CharacterState::new(Fixed3(start));
    s.grounded = true;
    let r = cc.tick(
        world,
        &mut s,
        &MoveInput {
            wish,
            ..MoveInput::default()
        },
    );
    assert!(!overlaps(world, &body_at(s.foot.0)));
    (r, s)
}

/// A sweep can stop with a wall contact and delta left over without running into the box:
/// here the iteration cap, on an oblique move that grazes an outside corner the body starts 2
/// units from. The step attempt that follows must not report a step unless the foot rose:
/// here it carries the body over the corner onto the block, its axis within the radius of
/// the block's top.
#[test]
fn a_move_past_a_corner_never_steps_without_rising() {
    let world = low_block();
    let start = [5120 - RADIUS, 5120 - 56, 0];
    let wish = [30, -120];
    let s = sweep(&world, &body_at(start), Fixed3([wish[0], wish[1], 0]));
    assert_eq!(s.iterations, MAX_SWEEP_ITERATIONS);
    assert_eq!(s.moved, Fixed3([12, -88, 0]));
    assert!(s.contact.is_some_and(|c| c.is_wall()));
    let (r, st) = one_tick(&world, start, wish);
    assert!(r.stepped);
    assert_eq!(st.foot.0, [4382, 4944, 256]);

    // The same corner at clearances 0 to 8 and twelve directions: whenever a tick steps, the
    // foot rose; and the iteration cap is among the cases.
    let mut capped = 0;
    for dy in 38..=64 {
        let start = [5120 - RADIUS, 5120 - dy, 0];
        for wish in [
            [5, -128],
            [13, -127],
            [20, -126],
            [30, -120],
            [45, -120],
            [64, -111],
            [91, -91],
            [111, -64],
            [120, -45],
            [126, -20],
            [128, 0],
            [127, 13],
        ] {
            let s = sweep(&world, &body_at(start), Fixed3([wish[0], wish[1], 0]));
            capped += u32::from(s.iterations == MAX_SWEEP_ITERATIONS);
            let (r, st) = one_tick(&world, start, wish);
            assert!(
                !r.stepped || st.foot.0[2] > 0,
                "dy {dy} wish {wish:?}: stepped at z {}",
                st.foot.0[2]
            );
        }
    }
    assert!(capped > 0);
}

/// The owner's frozen body: airborne under one unit beside a voxel edge (the pose of the
/// sweep test `a_box_under_one_unit_away_stops_only_a_move_that_closes_on_it`), it must fall
/// to the floor and walk away from the edge, not hang there with every sweep moving nothing.
#[test]
fn a_body_beside_a_voxel_edge_falls_and_walks_away() {
    let w = world(|[x, y, z]| z < 0 || (x == 20 && y == 20));
    let start = [5120 - 763, 5120 - 95, 1000];
    let steps = run(&w, start, 60, |_| MoveInput::default());
    assert_eq!(foot(&steps, 60), [start[0], start[1], 0]);
    assert!(steps[59].s.grounded);
    let steps = run(&w, start, 60, walk([-128, 0]));
    let [x, _, z] = foot(&steps, 60);
    assert_eq!(x, start[0] - 60 * 128);
    assert_eq!(z, 0);
}

/// A wall of 45-degree voxel stairs, `x + y >= 60`, on the F1 floor: every contact is a voxel
/// edge with an oblique normal.
fn oblique_wall() -> World {
    world(|[x, y, z]| z < 0 || x + y >= 60)
}

/// A wish 30 degrees off head-on into the oblique wall slides along it at every tick after
/// the first contact. With the projection onto the wall rounded toward zero per component,
/// the slide pointed a fraction of a unit into the edge it touched, the sweep stopped it at
/// once, and the body stood still against the wall about half the ticks.
#[test]
fn an_oblique_walk_slides_along_a_voxel_staircase_wall() {
    let steps = run(&oblique_wall(), S, 120, walk([124, 33]));
    let first = (1..)
        .zip(&steps)
        .find(|(_, s)| !s.r.contacts.is_empty())
        .map(|(t, _)| t);
    assert_eq!(first, Some(57));
    for (t, st) in (1..).zip(&steps).skip(56) {
        let [mx, my, _] = st.r.moved.0;
        assert!(mx * mx + my * my >= 32 * 32, "tick {t}: moved {mx}, {my}");
        assert!(!st.r.blocked, "tick {t}");
    }
    assert_eq!(foot(&steps, 120), [12424, 1862, 0]);
}
