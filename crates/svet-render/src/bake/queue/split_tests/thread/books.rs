//! The edit's frame and the books' snapshot on the bake thread (Plan 0099 Phase 3f), read on
//! the threaded device-less rig: a step's results and its books are two outbox entries, the
//! edit's frame waits for the first, and the mailbox answers by the older of the two.

use super::*;

/// A books hook that holds the snapshot after the step on message `from` (set at run time;
/// `u64::MAX`: none) until the sender is dropped, telling the test on `reached` when it holds.
/// A test binds the sender after its rig, so a failed test drops the sender (the hold let go)
/// before the rig's join.
fn hold_books() -> (
    BooksHook,
    Arc<std::sync::atomic::AtomicU64>,
    mpsc::Sender<()>,
    mpsc::Receiver<()>,
) {
    use std::sync::atomic::{AtomicU64, Ordering::SeqCst};
    let from = Arc::new(AtomicU64::new(u64::MAX));
    let (tx, rx) = mpsc::channel::<()>();
    let (reached_tx, reached) = mpsc::channel::<()>();
    let rx = Mutex::new(rx);
    let f = from.clone();
    let hook: BooksHook = Box::new(move |frame| {
        if frame == f.load(SeqCst) {
            let _ = reached_tx.send(());
            let _ = rx.lock().expect("one step at a time").recv();
        }
    });
    (hook, from, tx, reached)
}

/// Done when (1) of Plan 0099 Phase 3f. The slab drained, the dig's step's books' snapshot held
/// by a hook: the edit's frame returns while it is held, its lists taken; the books it holds are
/// the step before's; a read of the dug chunk still finds the box in the mailbox. The snapshot
/// let go, the next take holds the dig's books, the held block's words and table word are among
/// the edit's frame's writes at those books' place, and the next frame's mailbox has answered.
#[test]
fn the_edits_frame_does_not_wait_for_the_books() {
    use std::sync::atomic::Ordering::SeqCst;
    let (books_hook, from, hold, reached) = hold_books();
    let mut t = ThreadRig::with_books(None, books_hook, |_| {});
    // Bound after the rig: a failed test drops it first, letting the hold go before the join.
    let gate = hold;
    drain(&mut t);
    let before_books = t.link.books.frame;
    let before = t.applied.len();
    t.carve([60, 20, 4], [67, 23, 7]);
    let box_frame = t.rig.half.frame + 1;
    from.store(box_frame, SeqCst);
    t.frame().expect("the edit's frame returns with its list");
    assert_eq!(t.rig.half.frame, box_frame);
    reached.recv().expect("the dig's snapshot is held");
    // The gate is read, not a clock: the edit's frame returned and the dig's books are not
    // taken, their snapshot still held (the gate is let go below).
    assert!(
        t.link.books.frame < box_frame,
        "the edit's frame returned with the dig's snapshot held"
    );
    assert_eq!(t.link.lists_frame, box_frame, "the dig's lists taken");
    // The drain's last message may have gone into the dig's (coalesced): the books held are
    // the newest made before the dig's step.
    assert!(
        (before_books..box_frame).contains(&t.link.books.frame),
        "the books held ({}) are a step's before the dig's ({box_frame}; drained at {before_books})",
        t.link.books.frame
    );
    for c in [[0, 0, 0], [1, 0, 0]] {
        assert!(
            t.rig.inbox.near(c),
            "a read of chunk {c:?} still finds the box in the mailbox"
        );
    }
    let after = t.applied.len();
    assert!(after > before, "the edit's frame applied its lists");
    drop(gate);
    t.books_of(box_frame);
    for c in [[0, 0, 0], [1, 0, 0]] {
        assert!(
            block_and_table(&t, before, after, c),
            "chunk {c:?}'s held block and table word in the edit's frame"
        );
    }
    t.frame().expect("the next frame");
    assert!(
        !t.rig.inbox.has_box(),
        "the books of the dig's step held, the mailbox has answered"
    );
}

/// Prepared masks of `c` in `world`, as the stream workers hand them in.
fn masks_of(q: &mut BakeQueue, world: &World, c: [i32; 3]) -> svet_world::Prepared {
    let classes = q.classes(world.materials());
    let chunk = world.store().get(ChunkCoord(c)).expect("resident");
    let chunk = chunk.read().expect("unpoisoned");
    let m = PreparedMasks {
        classes: classes.clone(),
        masks: ChunkMasks::of(&chunk, &classes),
        ns: 0,
        seal_ns: 0,
        verdicts: Vec::new(),
        shells: Default::default(),
    };
    svet_world::Prepared(Arc::new(m))
}

/// Done when (2) of Plan 0099 Phase 3f. The queue still baking, message F carries chunk 1's
/// masks and the snapshot after step F is held by a hook: the next frame (no edit) takes step
/// F's lists and applies them, not its books, and keeps message F unanswered (chunk 1 unknown to
/// the reads); the frame after the snapshot is let go takes the books and answers it. In a
/// second run a stop between the two hands lockstep step F's lists none of them again: the
/// frame applied them once.
#[test]
fn the_mailbox_answers_by_the_older_of_lists_and_books() {
    for stop in [false, true] {
        answers_by_the_older(stop);
    }
}

/// [`the_mailbox_answers_by_the_older_of_lists_and_books`], ending at the stop or not.
fn answers_by_the_older(stop: bool) {
    use std::sync::atomic::Ordering::SeqCst;
    const B: [i32; 3] = [1, 0, 0];
    let (books_hook, from, hold, reached) = hold_books();
    let mut t = ThreadRig::with_books(None, books_hook, |_| {});
    // Bound after the rig: a failed test drops it first, letting the hold go before the join.
    let gate = hold;
    for s in 1..=2 {
        t.frame_stepped(s);
    }
    let masks = masks_of(&mut t.rig.queue, &t.world, B);
    t.rig.inbox.push_masks(&[(ChunkCoord(B), masks)]);
    let f = t.rig.half.frame + 1;
    from.store(f, SeqCst);
    t.frame().expect("message F goes out");
    assert_eq!(t.rig.half.frame, f);
    assert!(t.rig.inbox.unknown(B), "message F's masks are counted");
    reached.recv().expect("step F's snapshot is held");
    t.frame().expect("a frame with no edit");
    assert_eq!(t.link.lists_frame, f, "step F's lists taken");
    assert_eq!(t.lists.newest, f, "and applied");
    assert!(t.link.books.frame < f, "not its books");
    assert!(
        t.rig.inbox.unknown(B),
        "message F unanswered while the books held are older"
    );
    drop(gate);
    if stop {
        let ThreadRig { link, .. } = t;
        let st = link.stop().expect("the thread stops");
        let again = st
            .steps
            .iter()
            .flat_map(|(_, lists)| lists)
            .filter(|l| l.frame == f)
            .count();
        assert_eq!(again, 0, "step F's lists are not handed back to lockstep");
        return;
    }
    t.link.wait_step();
    assert!(t.link.steps() >= 3, "step F's books handed in");
    t.frame().expect("the frame that takes the books");
    assert!(t.link.books.frame >= f, "step F's books taken");
    assert!(!t.rig.inbox.unknown(B), "message F answered");
}
