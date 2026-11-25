//! The routes Plan 0099 Phase 3a needs before the bake half leaves the render thread, read on
//! the device-less rig: the messages' frames, a held bake half's mailbox, and the knob sets.

use super::*;

/// Moves the rig's frame count so its next message is `next` (frames may pass with no message;
/// the count only rises).
fn next_message_is(rig: &mut Rig, next: u64) {
    assert!(
        rig.half.frame < next,
        "the rig is already at {}",
        rig.half.frame
    );
    rig.half.frame = next - 1;
}

/// Done when (10) of Plan 0099 Phase 3a. Bake steps 1 and 2 read the messages of frames 10 and
/// 14 (11 to 13 coalesced into 14). The job that joins in step 2 (the dig ring of a box carried
/// in message 12) holds 14, and step 2's list names 14; a box in message 15, taken in step 3,
/// stales chunk 0 at 15 (the message's frame, not a step count) and its job's result is thrown
/// away; the change carried in message 14 does not stale the job stamped 14.
#[test]
fn a_job_and_a_box_are_stamped_by_the_messages_frame() {
    let mut rig = Rig::new(None);
    // The rig's first steps place every chunk's block, at most 9 frames in, so the steps the
    // done-when names start at 10 (a slower settle moves the frames on, the rule the same).
    rig.until("every chunk holds a block", |r| {
        (0..CHUNKS[0]).all(|x| r.store.block(ChunkCoord([x, 0, 0])).is_some())
            && r.half.flights.is_empty()
            && r.read().digs_settled()
    });
    let base = rig.half.frame.max(9) - 9;
    next_message_is(&mut rig, base + 10);
    rig.half.named.clear();
    // Step 1: message 10.
    let m = rig.message();
    assert_eq!(m.frame, base + 10);
    rig.run(m);
    assert_eq!(rig.queue.frame, base + 10, "step 1 read message 10");
    assert!(rig.half.named.iter().all(|&f| f == base + 10));
    // Messages 11 to 14, the box in 12, read as one by step 2; the GPU holds what it draws.
    rig.half.hold = true;
    rig.half.named.clear();
    let mut msg = rig.message();
    let (lo, hi) = ([0, 20, 4], [3, 23, 7]);
    rig.carve(lo, hi);
    for _ in 12..=14 {
        let newer = rig.message();
        msg = msg.coalesce(newer);
    }
    assert_eq!(msg.frame, base + 14);
    rig.run(msg);
    assert_eq!(rig.queue.frame, base + 14, "step 2 read message 14");
    assert_eq!(
        rig.queue.lp.stamps.get(&A),
        Some(&(base + 14)),
        "chunk 0's dig job joined in step 2, stamped 14"
    );
    assert!(
        !rig.half.named.is_empty() && rig.half.named.iter().all(|&f| f == base + 14),
        "step 2's lists name 14: {:?}",
        rig.half.named
    );
    // The change message 14 carried does not stale the job it made.
    assert!(
        rig.queue.stale.get(&A).is_none_or(|&s| s <= base + 14),
        "a change in message 14 stales no job stamped 14"
    );
    // Step 3: a box in message 15 near chunk 0's bricks, while its job is held.
    let (lo, hi) = NEAR_A;
    rig.carve(lo, hi);
    let m = rig.message();
    assert_eq!(m.frame, base + 15);
    rig.run(m);
    // The job stamped 14 is thrown away: staled at 15, or (a dig's ring 1 on a chunk whose dig
    // job runs) withdrawn and listed again at 15. Either way the mark is the message's frame.
    let staled = rig.queue.stale.get(&A) == Some(&(base + 15));
    let relisted = rig.queue.lp.stamps.get(&A) == Some(&(base + 15));
    assert!(
        staled || relisted,
        "the box holds its message's frame 15, not the step count {}: stale {:?}, stamp {:?}",
        rig.queue.steps,
        rig.queue.stale.get(&A),
        rig.queue.lp.stamps.get(&A)
    );
    assert_ne!(rig.queue.steps, base + 15);
    rig.half.hold = false;
    rig.until("the job stamped 14 gone", |r| {
        !r.queue.lp.stamps.get(&A).is_some_and(|&s| s == base + 14)
    });
    rig.until("chunk 0's dig settled", |r| {
        r.store.block(ChunkCoord(A)).is_some() && r.read().digs_settled()
    });
    assert!(!rig.queue.stale.contains_key(&A));
}

/// Done when (11) of Plan 0099 Phase 3a. The bake half held, a box sent in message 7: over
/// frames 7 to 9 every public read that consults the mailbox (a dig waiting, drained, settled,
/// passed, the sets that hold a chunk) answers as a queue that took the box into its books at
/// once; once a list made from message 7 or later is applied they answer from the bake half's
/// books alone.
#[test]
fn the_reads_count_a_box_in_flight() {
    let ready = |r: &Rig| {
        (0..CHUNKS[0]).all(|x| r.store.block(ChunkCoord([x, 0, 0])).is_some())
            && r.half.flights.is_empty()
            && r.read().digs_settled()
    };
    let (mut held, mut eager) = (Rig::new(None), Rig::new(None));
    held.until("every chunk holds a block", ready);
    eager.until("every chunk holds a block", ready);
    let beside = [-1, 0, 0];
    for r in [&mut held, &mut eager] {
        r.queue.faceless.insert(beside);
        r.queue.blocked.insert(beside, Default::default());
    }
    let base = held.half.frame.max(eager.half.frame).max(6) - 6;
    next_message_is(&mut held, base + 7);
    let (lo, hi) = ([0, 20, 4], [3, 23, 7]);
    held.carve(lo, hi);
    eager.carve(lo, hi);
    let arrivals = eager.inbox.take(base + 7);
    eager.inbox.answered(base + 7);
    eager
        .queue
        .take_message(base + 7, eager.settings, arrivals, eager.world.materials());
    // Messages 7, 8 and 9 go out; the bake half reads none of them.
    let mut sent = Vec::new();
    for f in 7..=9 {
        let m = held.message();
        assert_eq!(m.frame, base + f);
        sent.push(m);
        let (w, e) = (held.read(), eager.read());
        assert!(
            !w.digs_settled() && w.boxes_waiting(),
            "frame {f}: the dig waits"
        );
        assert_eq!(w.digs_settled(), e.digs_settled());
        assert_eq!(w.boxes_waiting(), e.boxes_waiting());
        assert_eq!(w.empty(), e.empty());
        for z in -2..=2 {
            for y in -2..=3 {
                for x in -2..=4 {
                    let c = ChunkCoord([x, y, z]);
                    for absent in [false, true] {
                        assert_eq!(
                            w.settled(c, absent),
                            e.settled(c, absent),
                            "frame {f}: settled {c:?} {absent}"
                        );
                    }
                    assert_eq!(w.passed(c), e.passed(c), "frame {f}: passed {c:?}");
                    assert_eq!(w.tracks(c), e.tracks(c), "frame {f}: tracks {c:?}");
                }
            }
        }
        assert!(
            !w.passed(ChunkCoord(beside)),
            "frame {f}: the neighbour waits"
        );
    }
    // The bake half reads the three as one; the list made from message 9 is applied.
    let mut msgs = sent.into_iter();
    let first = msgs.next().expect("three sent");
    let msg = msgs.fold(first, FrameMessage::coalesce);
    held.run(msg);
    assert!(!held.inbox.has_box(), "every message answered");
    assert!(!held.queue.faceless.contains(&beside) && !held.queue.blocked.contains_key(&beside));
    // The books alone: the box is the books' now, the reads theirs.
    let w = held.read();
    for x in -2..=4 {
        let c = ChunkCoord([x, 0, 0]);
        assert_eq!(
            w.passed(c),
            held.store.blocks.contains_key(&c.0)
                || held.queue.faceless.contains(&c.0)
                || held.queue.sealed.holds(c.0)
                || held.queue.blocked.contains_key(&c.0),
            "passed {c:?} from the books"
        );
    }
}

/// Done when (12) of Plan 0099 Phase 3a, its route. A knob set between frames 3 and 4 (the
/// frame's `set_bake_digs(5.0, false)`) waits in the mailbox: the books keep the old value
/// through the step on message 3's frame and read 5.0 in the step on message 4, before its
/// opening sizes the cap; the books are not written in between. With a dig waiting, that step's
/// cap is the one 5.0 ms buys on top of the slice.
#[test]
fn a_knob_set_reaches_the_bake_side_in_the_next_message() {
    let mut rig = Rig::new(None);
    let old = rig.queue.edit_ms;
    assert_ne!(old, 5.0);
    for _ in 0..3 {
        rig.step();
    }
    assert_eq!(rig.half.frame, 3);
    rig.inbox.push_set(Box::new(|q: &mut BakeQueue| {
        (q.edit_ms, q.reach_ring) = (5.0, false);
    }));
    assert_eq!(
        rig.queue.edit_ms, old,
        "the set writes nothing before the step"
    );
    assert!(
        rig.queue.reach_ring,
        "the default reach ring stands until the step"
    );
    // A dig waits in the same message, so the opening adds the edit's grant; the GPU holds
    // what it draws, so the dig's jobs stay open over the step.
    rig.half.hold = true;
    let (lo, hi) = ([0, 20, 4], [3, 23, 7]);
    rig.carve(lo, hi);
    let msg = rig.message();
    assert_eq!(msg.frame, 4);
    // The price as the step's opening reads it: a read-back updates it only after the opening.
    let cost = rig.queue.cost.clone();
    rig.run(msg);
    assert_eq!((rig.queue.edit_ms, rig.queue.reach_ring), (5.0, false));
    assert!(
        !rig.queue.digs.is_empty() || !rig.queue.dig_open.is_empty(),
        "the dig is in the books"
    );
    let st = rig.queue.stats;
    assert_eq!(
        st.cap,
        cost.cap(st.slice_ms + 5.0),
        "the step on message 4 sizes its cap with 5.0"
    );
    assert_ne!(
        st.cap,
        cost.cap(st.slice_ms + old),
        "and not with the old value"
    );
}
