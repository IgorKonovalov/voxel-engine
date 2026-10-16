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
