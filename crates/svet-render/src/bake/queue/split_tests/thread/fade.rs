//! A dig into a fading chunk on the threaded device-less rig: its frames stepped by count, one
//! step a frame as lockstep runs them, and the step before the dig's held on a gate the box's
//! send releases. No sleep paces a frame or a release.

use super::*;
use std::sync::atomic::{AtomicU64, Ordering::SeqCst};

/// The steps a [`gated`] hook entered: how many, and the newest message frame among them (the
/// count rises first, so a frame read here is counted).
#[derive(Clone, Default)]
struct Entered {
    steps: Arc<AtomicU64>,
    newest: Arc<AtomicU64>,
}

/// A hook that counts every step it enters ([`Entered`]) and holds the step of message `held`
/// (none while `u64::MAX`) until the queue's box signal reads a box sent and not taken
/// (`BoxSignal::waits`); a guard of one second fails the test.
fn gated() -> (Hook, Arc<AtomicU64>, Entered) {
    let held = Arc::new(AtomicU64::new(u64::MAX));
    let entered = Entered::default();
    let (h, e) = (held.clone(), entered.clone());
    let hook: Hook = Box::new(move |q, m| {
        e.steps.fetch_add(1, SeqCst);
        e.newest.fetch_max(m.frame, SeqCst);
        if m.frame != h.load(SeqCst) {
            return;
        }
        let signal = q
            .box_signal
            .as_ref()
            .expect("the thread's queue has a box signal");
        let t = stopwatch();
        while !signal.waits() {
            assert!(
                seconds_since(t) < 1.0,
                "the gate: no box sent within 1 s of the held step"
            );
            std::thread::yield_now();
        }
    });
    (hook, held, entered)
}

/// One frame, then waits up to a second for its message's step to have run: the message (or
/// the one it was merged into) entered and every entered step's entries in the outbox. Frames
/// so paced step once each, at their own frame numbers.
fn frame_settled(t: &mut ThreadRig, e: &Entered) {
    t.frame().expect("the frame returns");
    let frame = t.rig.half.frame;
    assert!(
        within(|| e.newest.load(SeqCst) >= frame && t.link.steps() == e.steps.load(SeqCst)),
        "frame {frame}'s step within a second"
    );
}

/// Done when (3) of Plan 0099 Phase 3c, the case of [`the_edits_frame_waits_for_its_list_and_no_other_frame_does`]:
/// a dig into chunk 0 while its ring 1's fade runs, with the step before the dig's held so that
/// its lists (a fade's show of chunk 0) and the dig's come in one frame. The frame returns with
/// chunk 0's held block's words at the place the pool's chunk table names, as the store's copy
/// holds them. The frames up to the held step each wait for their step (the fade is paced by
/// the frame number, so a step that comes late moves it further); the held step is let go by
/// the dig's box being sent.
pub(super) fn a_dig_into_a_fading_chunk_shows_its_held_block() {
    const A: [i32; 3] = [0, 0, 0];
    let (hook, held, entered) = gated();
    let mut t = ThreadRig::new(Some(hook), OUTBOX_BYTES);
    t.pool = Some(Vec::new());
    drain(&mut t);
    t.carve([20, 20, 4], [23, 23, 7]);
    frame_settled(&mut t, &entered);
    let mut fading = false;
    for _ in 0..400 {
        frame_settled(&mut t, &entered);
        if t.link.books.stats.fading > 0 {
            fading = true;
            break;
        }
    }
    assert!(fading, "the first dig's ring 1 lands and fades");
    // The next step (S) shows the fade and is held; the dig's step (D) reads the box after it.
    held.store(t.rig.half.frame + 1, SeqCst);
    t.frame().expect("the held step's message goes out");
    assert!(
        within(|| t.link.peek(|m| m.is_none())),
        "the step took its message and is held"
    );
    t.frame().expect("the frame returns");
    let before = t.applied.len();
    t.carve([24, 20, 4], [27, 23, 7]);
    t.frame().expect("the edit's frame returns with its list");
    let after = t.applied.len();
    t.books_of(t.rig.half.frame);
    let (first, words) = *t.link.books.blocks.get(&A).expect("chunk 0 keeps a block");
    let shows = t.applied[before..after]
        .iter()
        .filter(|a| a.kind == W_POOL && a.offset == first * 4)
        .count();
    println!(
        "held step at frame {}, the edit's frame {}, {shows} writes at chunk 0's place",
        held.load(SeqCst),
        t.rig.half.frame
    );
    assert!(
        shows >= 2,
        "the fade's show and the held block both written at chunk 0's place in the edit's frame ({shows})"
    );
    let ThreadRig {
        link, rig, pool, ..
    } = t;
    let pool = pool.expect("kept");
    let st = link.stop().expect("the thread stops");
    let held = st
        .side
        .store
        .block_words(ChunkCoord(A))
        .expect("the store's copy");
    assert_eq!(held.len() as u64, words);
    let mut store = Store::new(64 << 20);
    store.start_queue(rig.mirror.window(), rig.queue.settings.cell);
    let table = store.table_word(ChunkCoord(A)) as usize * 4;
    let word = u32::from_le_bytes(pool[table..table + 4].try_into().expect("4 bytes"));
    assert_eq!(
        u64::from(word & !crate::bake::TABLE_COARSE),
        first,
        "the pool's table names the books' place"
    );
    let at = first as usize * 4;
    let shown: Vec<u32> = bytemuck::pod_collect_to_vec(&pool[at..at + held.len() * 4]);
    assert!(
        shown == held,
        "chunk 0's place holds the held block after the edit's frame"
    );
}
