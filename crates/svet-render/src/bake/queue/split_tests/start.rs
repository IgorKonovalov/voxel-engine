//! What the start's pre-roll counts as a near chunk holding its light, on the rule's provisional
//! rig: `C` bakes once under a column not final (its block provisional, at the full count), the
//! waiting chunk arrives, and `C` is owed one catch-up, which waits behind the first bakes
//! (ADR-0070's amendment). The pre-roll counts `C` held while the catch-up is due and while its
//! job is in the loop ([`QueueRead::held_for_start`]); [`QueueRead::settled`] reads it busy
//! through both, for its other callers.

use super::rule::{BOX_TOP, C, WAITING, in_job, jobs, provisional_rig, settle};
use super::*;
use crate::bake::{PreRoll, PreRollConfig, PreRollState};

/// A pre-roll whose one near chunk is `C`, with a coarse pass.
fn pre_roll() -> PreRoll {
    PreRoll::new(PreRollConfig {
        near: vec![ChunkCoord(C)],
        coarse: 16,
        box_ms: 5000.0,
        cap_ms: 120_000.0,
        samples: 1,
    })
}

/// The pre-roll's state after one step at which `held` of its one near chunk hold their light.
fn after_a_step(held: bool) -> PreRollState {
    pre_roll().decide(
        10.0,
        usize::from(held),
        (1, 1),
        || vec![ChunkCoord([0, 0, 0])],
        |_| 0,
    )
}

/// `C`'s provisional block at the full count, the waiting chunk arrived: the rig after it.
fn provisional_c() -> Rig {
    let mut rig = provisional_rig();
    // The column's top in `C`'s reach box: the rule does not answer `C`, so its catch-up is a job.
    rig.mirror.tops.insert([1, 1], BOX_TOP);
    settle(&mut rig, "C's provisional block");
    assert_eq!(jobs(&rig), 1, "C's first bake is a job");
    assert!(rig.queue.provisional.contains(&C), "flagged provisional");
    assert!(
        rig.store.blocks.contains_key(&C) && !rig.queue.lesser(&rig.store, C),
        "C holds its block at the full count"
    );
    let r = rig.read();
    assert!(r.settled(ChunkCoord(C), false) && r.held_for_start(ChunkCoord(C), false));
    rig.mirror.absent.remove(&WAITING);
    rig.mirror.changed.push(ChunkCoord(WAITING));
    rig
}

/// What the three reads give for `C`: `settled`, and `held_for_start` from the books and from
/// the bake thread's snapshot of them.
fn reads(rig: &Rig) -> (bool, bool, bool) {
    let c = ChunkCoord(C);
    let books = Books::of(&rig.queue, &rig.store, 0, rig.view.eye);
    let snapshot = QueueRead {
        src: ReadSource::Books(&books),
        inbox: &rig.inbox,
    };
    assert_eq!(
        snapshot.settled(c, false),
        rig.read().settled(c, false),
        "the snapshot's settled"
    );
    (
        rig.read().settled(c, false),
        rig.read().held_for_start(c, false),
        snapshot.held_for_start(c, false),
    )
}

/// Plan 0109 Phase 2d, done-when (a) and (c). `C`'s columns turn final: its catch-up is due and
/// not yet in a job. The pre-roll counts `C` held and leaves its near stage at that step;
/// `settled` reads it busy as before. On the base the pre-roll counted through `settled`, so it
/// read 0 of 1 and stayed in the near stage (`Near { settled: 0, of: 1 }`).
#[test]
fn a_due_catch_up_does_not_hold_the_start() {
    let mut rig = provisional_c();
    rig.until("C's catch-up due", |r| {
        r.queue.provisional_due.contains(&C) || in_job(r)
    });
    assert!(
        rig.queue.provisional_due.contains(&C) && !in_job(&rig),
        "due, not yet in a job"
    );
    assert_eq!(
        reads(&rig),
        (false, true, true),
        "(settled, held for the start, the snapshot's)"
    );
    assert_eq!(
        after_a_step(false),
        PreRollState::Near { settled: 0, of: 1 }
    );
    assert_eq!(
        after_a_step(rig.read().held_for_start(ChunkCoord(C), false)),
        PreRollState::Coarse { passed: 0, of: 1 },
        "the pre-roll leaves its near stage"
    );
}

/// Plan 0109 Phase 2d, done-when (b) and (c). `C`'s catch-up job is in the loop, its block still
/// the provisional one: the pre-roll counts `C` held and leaves its near stage; `settled` reads
/// it busy as before. On the base it read 0 of 1 and stayed in the near stage.
#[test]
fn a_catch_up_in_a_job_does_not_hold_the_start() {
    let mut rig = provisional_c();
    rig.until("C's catch-up in a job", in_job);
    assert!(
        rig.queue.provisional.contains(&C) && !rig.queue.provisional_due.contains(&C),
        "the provisional block held, its catch-up out of the due set"
    );
    assert_eq!(jobs(&rig), 1, "the catch-up not yet placed");
    assert_eq!(
        reads(&rig),
        (false, true, true),
        "(settled, held for the start, the snapshot's)"
    );
    assert_eq!(
        after_a_step(rig.read().held_for_start(ChunkCoord(C), false)),
        PreRollState::Coarse { passed: 0, of: 1 },
        "the pre-roll leaves its near stage"
    );
}
