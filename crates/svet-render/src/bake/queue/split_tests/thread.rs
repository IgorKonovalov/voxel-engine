//! The bake half on the bake thread (Plan 0099 Phase 3a), read on the device-less rig: the
//! rig's recording half stands in for the render half's frame ([`render_frame`]), and the real
//! queue steps on the thread against the world it owns a handle to.

use super::*;
use crate::bake::{
    BakeSide, Hook, OUTBOX_BYTES, THREAD_LIST_CAP, ThreadLink, render_frame, seconds_since,
    stopwatch,
};
use std::sync::Mutex;
use std::sync::mpsc;

/// The rig with its bake half on the thread: the mirror, the settings, the mailbox and the
/// recording half stay here.
struct ThreadRig {
    rig: Rig,
    world: Arc<World>,
    link: ThreadLink,
    lists: Lists,
    applied: Vec<Applied>,
}

impl ThreadRig {
    fn new(hook: Option<Hook>, outbox: usize) -> Self {
        let mut rig = Rig::new(None);
        let world = Arc::new(super::world());
        let settings = rig.queue.settings.clone();
        let side = BakeSide {
            queue: std::mem::replace(&mut rig.queue, BakeQueue::new(settings, 4.0)),
            store: std::mem::replace(&mut rig.store, Store::new(64 << 20)),
            shadow: std::mem::take(&mut rig.shadow),
            world: world.clone(),
            hook,
        };
        let link = ThreadLink::start(side, Some(THREAD_LIST_CAP), outbox).expect("spawned");
        Self {
            rig,
            world,
            link,
            lists: Lists {
                cap: Some(THREAD_LIST_CAP),
                ..Lists::default()
            },
            applied: Vec::new(),
        }
    }

    /// One render frame: the message made, the outbox taken, its calls replayed on the
    /// recording half, its lists applied under the cap, the message sent.
    fn frame(&mut self) -> Result<QueueStats, BakeError> {
        let msg = self.rig.message();
        let mut out = Recorder {
            out: &mut self.applied,
        };
        render_frame(
            &mut self.link,
            &mut self.rig.half,
            &mut self.lists,
            &mut out,
            &mut self.rig.inbox,
            msg,
        )
    }

    /// One frame, then waits up to a second for the bake thread to have stepped `steps` times.
    fn frame_stepped(&mut self, steps: u64) {
        self.frame().expect("the step runs");
        assert!(
            within(|| self.link.steps() >= steps),
            "step {steps} within a second"
        );
    }

    /// An edit's box on the shared world, mirrored and handed to the mailbox as the frame does.
    fn carve(&mut self, min: [i32; 3], max: [i32; 3]) {
        let r = self
            .world
            .apply(&box_op(min.map(i64::from), max.map(i64::from), Mode::Carve))
            .expect("resident");
        let (lo, hi) = r.aabb.expect("voxels changed");
        for x in lo.0[0].div_euclid(64)..=hi.0[0].div_euclid(64) {
            self.rig.mirror.changed.push(ChunkCoord([x, 0, 0]));
        }
        self.rig.inbox.push_box(lo.0, hi.0);
    }
}

/// Waits up to a second for `pred`.
fn within(pred: impl Fn() -> bool) -> bool {
    let t = stopwatch();
    while seconds_since(t) < 1.0 {
        if pred() {
            return true;
        }
        std::thread::yield_now();
    }
    pred()
}

/// A hook that holds every step from message `from` on until the sender is dropped.
fn hold_from(from: u64) -> (Hook, mpsc::Sender<()>) {
    let (tx, rx) = mpsc::channel::<()>();
    let rx = Mutex::new(rx);
    let hook: Hook = Box::new(move |_, m| {
        if m.frame >= from {
            let _ = rx.lock().expect("one step at a time").recv();
        }
    });
    (hook, tx)
}

/// Done when (1) of Plan 0099 Phase 3 (Phase 3a). The bake half held by a hook from its second
/// step: 100 frame calls return, apply nothing, and leave one message waiting whose change set
/// holds no more than the distinct chunks the frames sent.
#[test]
fn the_render_side_returns_with_the_bake_side_stalled() {
    let (hook, gate) = hold_from(2);
    let mut t = ThreadRig::new(Some(hook), OUTBOX_BYTES);
    t.frame_stepped(1);
    t.frame().expect("the step runs");
    assert!(
        within(|| t.link.peek(|m| m.is_none())),
        "the second step took its message and is held"
    );
    // What step 1 made lands now; nothing after it while the step is held.
    t.frame().expect("the frame returns");
    let applied = t.applied.len();
    let at = stopwatch();
    for f in 0..100 {
        let c = ChunkCoord([f % 2, 0, 0]);
        t.rig.mirror.changed.push(c);
        t.frame().expect("the frame returns");
    }
    assert!(seconds_since(at) < 2.0, "100 frames within 2 s");
    assert_eq!(
        t.applied.len(),
        applied,
        "nothing applied while the step is held"
    );
    t.link.peek(|m| {
        let m = m.expect("one message waits");
        let changed = m.msg.notes.changed.as_ref().expect("tracked");
        assert!(changed.len() <= 2, "changes {changed:?}");
        assert!(
            m.msg.notes.chunks.len() <= 2,
            "{} chunk notes",
            m.msg.notes.chunks.len()
        );
        assert!(
            m.msg.notes.columns.len() <= 2,
            "{} columns",
            m.msg.notes.columns.len()
        );
        assert_eq!(m.msg.frame, t.rig.half.frame, "the newest view");
    });
    drop(gate);
}

/// Done when (2) of Plan 0099 Phase 3 (Phase 3a). With the render half sending but not taking,
/// the outbox stops at its cap in bytes (an entry over the cap goes alone into an empty
/// outbox, so the drained queue's small entries read it) and the bake half's step count stops rising; a take
/// makes room and the steps go on.
#[test]
fn the_outbox_is_bounded_and_the_bake_side_waits() {
    const CAP: usize = 16 << 10;
    let mut t = ThreadRig::new(None, CAP);
    // The world's blocks first, taken as they come (a step's lists can pass this small cap);
    // then each step makes only its books' snapshot.
    for s in 1..=400 {
        t.frame_stepped(s);
        if t.link.books.idle && t.lists.waiting.is_empty() {
            break;
        }
    }
    assert!(t.link.books.idle, "the queue drains within 400 frames");
    t.frame().expect("the last step taken");
    let send = |t: &mut ThreadRig| {
        let msg = t.rig.message();
        t.link.send(msg, Vec::new());
    };
    let mut last = u64::MAX;
    let mut flat = 0;
    let at = stopwatch();
    // Flat over 200 sends a millisecond apart: longer than any step of this world.
    while flat < 200 {
        assert!(
            seconds_since(at) < 1.8,
            "the steps stop rising within 1.8 s"
        );
        send(&mut t);
        std::thread::sleep(std::time::Duration::from_millis(1));
        let s = t.link.steps();
        flat = if s == last { flat + 1 } else { 0 };
        last = s;
    }
    assert!(
        t.link.outbox_bytes() <= CAP,
        "{} B over the cap",
        t.link.outbox_bytes()
    );
    assert!(
        t.link.outbox_bytes() > CAP / 2,
        "the outbox fills to its cap: {} B after {last} steps",
        t.link.outbox_bytes()
    );
    t.frame().expect("the frame takes the outbox");
    assert!(
        within(|| t.link.steps() > last),
        "room made, the steps go on"
    );
}

/// Done when (3) of Plan 0099 Phase 3 (Phase 3a), its first part. Dropping the render side's
/// handle stops and joins the bake thread, the bake half dropped with it, held at a step's wait
/// or not.
#[test]
fn dropping_the_queue_joins_the_bake_thread() {
    let alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
    struct Flag(Arc<std::sync::atomic::AtomicBool>);
    impl Drop for Flag {
        fn drop(&mut self) {
            self.0.store(false, std::sync::atomic::Ordering::Release);
        }
    }
    let flag = Flag(alive.clone());
    let hook: Hook = Box::new(move |_, _| {
        let _ = &flag;
    });
    let at = stopwatch();
    let mut t = ThreadRig::new(Some(hook), OUTBOX_BYTES);
    for s in 1..=3 {
        t.frame_stepped(s);
    }
    t.frame().expect("a message waits or steps");
    drop(t);
    assert!(
        !alive.load(std::sync::atomic::Ordering::Acquire),
        "joined: the bake half dropped with the thread"
    );
    assert!(seconds_since(at) < 1.0);
}

/// Done when (3) of Plan 0099 Phase 3 (Phase 3a), its second part. A panic on the bake thread
/// is the error of the next frame's call after it, with its message, and once read the thread
/// is joined.
#[test]
fn a_panic_on_the_bake_thread_is_the_next_steps_error() {
    let hook: Hook = Box::new(|_, m| assert!(m.frame < 3, "a broken step"));
    let at = stopwatch();
    let mut t = ThreadRig::new(Some(hook), OUTBOX_BYTES);
    t.frame_stepped(1);
    t.frame_stepped(2);
    t.frame().expect("message 3 goes out");
    let mut read = None;
    while read.is_none() && seconds_since(at) < 1.0 {
        if let Err(e) = t.frame() {
            read = Some(e);
        }
    }
    assert_eq!(read, Some(BakeError::Thread("a broken step".into())));
    assert!(seconds_since(at) < 1.0);
}

/// Done when (12) of Plan 0099 Phase 3a, with the thread on. `set_bake_digs(5.0, false)`
/// between frames 3 and 4 goes into the mailbox: the books on the bake thread hold the old
/// value when the step on message 4 starts (no direct write), read 5.0 in it before its opening
/// sizes the cap, and that step's cap is the one 5.0 ms buys on top of the slice.
#[test]
fn a_knob_set_reaches_the_bake_thread_in_the_next_message() {
    type Seen = Arc<Mutex<Option<(f64, bool, SampleCost)>>>;
    let seen: Seen = Arc::new(Mutex::new(None));
    let s = seen.clone();
    let hook: Hook = Box::new(move |q, m| {
        if m.frame == 4 {
            *s.lock().expect("one step") = Some((q.edit_ms, q.reach_ring, q.cost.clone()));
        }
    });
    let mut t = ThreadRig::new(Some(hook), OUTBOX_BYTES);
    for s in 1..=3 {
        t.frame_stepped(s);
    }
    t.rig.inbox.push_set(Box::new(|q: &mut BakeQueue| {
        (q.edit_ms, q.reach_ring) = (5.0, false);
    }));
    t.rig.half.hold = true;
    t.carve([0, 20, 4], [3, 23, 7]);
    t.frame_stepped(4);
    let (old, ring, cost) = seen.lock().expect("read").take().expect("step 4 ran");
    assert_ne!(old, 5.0, "the set wrote nothing before the step");
    assert!(ring, "the default reach ring stood until the step");
    t.frame().expect("the frame takes step 4");
    let books = t.link.books.clone();
    assert_eq!(books.frame, 4, "the newest step taken read message 4");
    let st = books.stats;
    assert_eq!(
        st.cap,
        cost.cap(st.slice_ms + 5.0),
        "the cap sized with 5.0"
    );
    assert_ne!(
        st.cap,
        cost.cap(st.slice_ms + old),
        "and not with the old value"
    );
}
