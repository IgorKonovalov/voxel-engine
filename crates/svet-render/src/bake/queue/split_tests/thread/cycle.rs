//! The bake thread's cycle by its parts (Plan 0099 Phase 4d), read on the threaded device-less
//! rig: the parts are spans between instants the thread's one clock took in order.

use super::*;

/// Done when (2) of Plan 0099 Phase 4d. Twelve frames on the threaded rig, each waiting for the
/// bake thread to have stepped once more (no step held by a sleep): every cycle the takes
/// completed has its instants in order (free, the message taken, the step's return, the books'
/// snapshot's start and end, free again), follows the one before it (its `free` is the earlier
/// one's next), and its four parts (the wait for a message, the step, the snapshot, the
/// hand-over) sum to the time from its pick-up's `free` to the next step's to the clock's tick.
#[test]
fn the_cycle_parts_sum_to_the_time_between_two_steps() {
    let mut t = ThreadRig::new(None, OUTBOX_BYTES);
    let mut cycles = Vec::new();
    for k in 1..=12 {
        t.frame_stepped(k);
        cycles.extend(t.link.cycles.iter().copied());
    }
    assert!(
        cycles.len() >= 8,
        "at least 8 of 12 cycles completed: {}",
        cycles.len()
    );
    for (k, c) in cycles.iter().enumerate() {
        let parts = c.parts();
        let sum: f64 = parts.iter().sum();
        println!(
            "cycle {k}: parts {parts:?} sum {sum:.6} ms, span {:.6} ms",
            c.span()
        );
        assert!(
            c.free <= c.took
                && c.took <= c.step_end
                && c.step_end <= c.at
                && c.at <= c.end
                && c.end <= c.next_free,
            "cycle {k}: the instants in the order the thread takes them: {c:?}"
        );
        assert!(
            (sum - c.span()).abs() <= TICK_MS,
            "cycle {k}: the parts {parts:?} sum to {sum:.6} ms against {:.6} ms from one free to the next",
            c.span()
        );
        if let Some(next) = cycles.get(k + 1) {
            assert_eq!(
                c.next_free, next.free,
                "cycle {k}: the next cycle starts where this one ends"
            );
        }
    }
}
