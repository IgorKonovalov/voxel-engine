//! The bake half on the bake thread (Plan 0099 Phase 3a), read on the device-less rig: the
//! rig's recording half stands in for the render half's frame ([`render_frame`]), and the real
//! queue steps on the thread against the world it owns a handle to.

use super::*;
use crate::bake::{
    BakeSide, Hook, OUTBOX_BYTES, Op, THREAD_LIST_CAP, ThreadLink, carried_first, render_frame,
    replay, replay_a_frame, seconds_since, stopwatch,
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
        Self::sized(hook, outbox, CHUNKS[0], CHUNKS[0], |_| {})
    }

    /// The rig over [`world_of`]`(size)`, its first `held` chunks in the mirror, with `prep`
    /// run on the queue before the thread takes it.
    fn sized(
        hook: Option<Hook>,
        outbox: usize,
        size: i32,
        held: i32,
        prep: impl FnOnce(&mut BakeQueue),
    ) -> Self {
        let mut rig = Rig::sized(None, size, held);
        let world = Arc::new(world_of(size));
        let settings = rig.queue.settings.clone();
        prep(&mut rig.queue);
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

/// What [`first_opening`] read: the rig after the box's step (or the opening's, with no box),
/// the candidates the first step that listed any pushed into the loop, and the box's message.
struct Opening {
    t: ThreadRig,
    listed: usize,
    /// The frame of the message the step after that one read.
    next_frame: u64,
    applied_before_box: usize,
    box_frame: Option<u64>,
}

/// Chunks of [`a_waiting_box_ends_the_opening`]'s world: three held from the start, 32 that
/// stream in once those are baked.
const OPENING_SIZE: i32 = 35;
const OPENING_HELD: i32 = 3;

/// The slab of [`OPENING_SIZE`] chunks with the first [`OPENING_HELD`] in the mirror, drained;
/// then the other 32 arrive, every one a ready candidate. With `post`, a hook on the bake
/// thread stops the first opening after its second listing until the render side has carved
/// chunk 0's slab top and sent that frame's message (its box), as an edit in the window does
/// while a step runs; then both steps are run and taken.
fn first_opening(post: bool) -> Opening {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
    let listed = Arc::new(AtomicUsize::new(0));
    let armed = Arc::new(AtomicBool::new(false));
    // At each step's start once armed: the frame of the message it reads, and the candidates
    // the step before it listed.
    let counts = Arc::new(Mutex::new(Vec::<(u64, usize)>::new()));
    let (ask, asked) = mpsc::channel::<()>();
    let (ack, acked) = mpsc::channel::<()>();
    let acked = Mutex::new(acked);
    let (l, a) = (listed.clone(), armed.clone());
    let once = AtomicBool::new(post);
    let on_list = move || {
        if a.load(SeqCst) && l.fetch_add(1, SeqCst) + 1 == 2 && once.swap(false, SeqCst) {
            ask.send(()).expect("the test listens");
            acked.lock().expect("one listing").recv().expect("acked");
        }
    };
    let (l, a, c) = (listed.clone(), armed.clone(), counts.clone());
    let on_step: Hook = Box::new(move |_, m| {
        if a.load(SeqCst) {
            c.lock()
                .expect("one step")
                .push((m.frame, l.swap(0, SeqCst)));
        }
    });
    let mut t = ThreadRig::sized(
        Some(on_step),
        OUTBOX_BYTES,
        OPENING_SIZE,
        OPENING_HELD,
        move |q| q.listed_hook = Some(Box::new(on_list)),
    );
    let drained = |t: &ThreadRig| {
        let b = &t.link.books;
        b.idle && (0..OPENING_HELD).all(|x| b.blocks.contains_key(&[x, 0, 0]))
    };
    for _ in 0..400 {
        if drained(&t) {
            break;
        }
        t.frame().expect("the step runs");
        within(|| t.link.peek(|m| m.is_none()));
    }
    assert!(drained(&t), "the held chunks drain");
    armed.store(true, SeqCst);
    t.rig.mirror.held = OPENING_SIZE;
    t.rig
        .mirror
        .changed
        .extend((OPENING_HELD..OPENING_SIZE).map(|x| ChunkCoord([x, 0, 0])));
    let mut box_frame = None;
    let mut applied_before_box = 0;
    // The first step that listed: its count, and the frame the step after it read.
    let first = |c: &Arc<Mutex<Vec<(u64, usize)>>>| {
        let c = c.lock().expect("read");
        c.iter().find(|&&(_, n)| n > 0).map(|&(f, n)| (n, f))
    };
    for _ in 0..400 {
        t.frame().expect("the step runs");
        if post {
            if asked
                .recv_timeout(std::time::Duration::from_millis(50))
                .is_ok()
            {
                // The step that listed twice waits (not yet counted): an edit, and the frame's
                // message sent. The edit's frame waits for the box's step (Phase 3b), so the
                // step is let go from a second thread once the box is stamped.
                let steps = t.link.steps();
                let sender = t.link.box_sender();
                let before = sender.sent();
                let ack = ack.clone();
                let release = std::thread::spawn(move || {
                    assert!(within(|| sender.sent() > before), "the box is sent");
                    ack.send(()).expect("the hook waits");
                });
                applied_before_box = t.applied.len();
                t.carve([20, 20, 4], [23, 23, 7]);
                t.frame().expect("the frame returns");
                box_frame = Some(t.rig.half.frame);
                release.join().expect("released");
                assert!(
                    within(|| t.link.steps() >= steps + 2),
                    "the cut step and the box's step run"
                );
                // A step counts before its results reach the outbox: frames until the box's
                // step's books are taken.
                for _ in 0..400 {
                    t.frame().expect("the frame takes both");
                    if t.link.books.frame >= box_frame.unwrap_or(0) {
                        break;
                    }
                    std::thread::yield_now();
                }
                break;
            }
        } else {
            within(|| t.link.peek(|m| m.is_none()));
            // A step's count is filed at the next step's start.
            if first(&counts).is_some() {
                break;
            }
        }
    }
    let (listed, next_frame) = first(&counts).expect("an opening listed the arrivals");
    Opening {
        t,
        listed,
        next_frame,
        applied_before_box,
        box_frame,
    }
}

/// Done when (5a) of Plan 0099 Phase 3 as amended (Phase 3a). 32 candidates ready and a hook
/// that posts a box after the opening's second listing: that step lists at most 3, and the
/// next step takes the box, its list holding chunk 0's held block's words and its table word.
/// With no box the same opening lists more than 3.
#[test]
fn a_waiting_box_ends_the_opening() {
    let control = first_opening(false);
    assert!(
        control.listed > 3,
        "with no box the opening lists {} of the 32",
        control.listed
    );
    let control_listed = control.listed;
    drop(control);
    let o = first_opening(true);
    assert!(
        o.listed <= 3,
        "the step lists {} with a box waiting",
        o.listed
    );
    println!(
        "listed: {} with no box, {} with one",
        control_listed, o.listed
    );
    let box_frame = o.box_frame.expect("the box was sent");
    let books = &o.t.link.books;
    assert_eq!(
        o.next_frame, box_frame,
        "the next step read the box's message"
    );
    assert!(!books.boxes, "and took the box");
    let (first, words) = *books.blocks.get(&A).expect("chunk 0 keeps a block");
    let mut table = Store::new(64 << 20);
    table.start_queue(o.t.rig.mirror.window(), o.t.rig.queue.settings.cell);
    let table = table.table_word(ChunkCoord(A)) * 4;
    let applied = &o.t.applied[o.applied_before_box..];
    assert!(
        applied
            .iter()
            .any(|a| a.kind == W_POOL && a.offset == first * 4 && a.bytes as u64 == words * 4),
        "the held block's words"
    );
    assert!(
        applied
            .iter()
            .any(|a| a.kind == W_TABLE && a.offset == table && a.bytes == 4),
        "its table word"
    );
}

/// Done when (2) of Plan 0099 Phase 2e, with the thread on. The rig starts at 1.0 (set by name
/// before the thread takes the books: the default is 1.5). The stretch set to 1.5
/// between frames 3 and 4 goes into the mailbox: the books on the bake thread hold 1.0 when the
/// step on message 4 starts (no setter writes the bake half directly), and that step plans by
/// 1.5: its dig's ring 1 (280 samples in this rig) against a cap of 200 goes whole.
#[test]
fn the_stretch_reaches_the_bake_thread_in_the_next_message() {
    let seen: Arc<Mutex<Option<f64>>> = Arc::new(Mutex::new(None));
    let s = seen.clone();
    let hook: Hook = Box::new(move |q, m| {
        if m.frame == 4 {
            *s.lock().expect("one step") = Some(q.ring1_stretch);
        }
    });
    let mut t = ThreadRig::sized(Some(hook), OUTBOX_BYTES, CHUNKS[0], CHUNKS[0], |q| {
        q.ring1_stretch = 1.0;
    });
    for s in 1..=3 {
        t.frame_stepped(s);
    }
    t.rig
        .inbox
        .push_set(Box::new(|q: &mut BakeQueue| q.ring1_stretch = 1.5));
    t.rig.half.hold = true;
    t.carve([0, 20, 4], [3, 23, 7]);
    t.rig.samples = Some(200);
    t.frame_stepped(4);
    let old = seen.lock().expect("read").take().expect("step 4 ran");
    assert_eq!(old, 1.0, "the set wrote nothing before the step");
    t.frame().expect("the frame takes step 4");
    let books = t.link.books.clone();
    assert_eq!(books.frame, 4, "the newest step taken read message 4");
    let st = books.stats;
    assert_eq!((st.cap, st.stretched), (280, true), "the step plans by 1.5");
    assert_eq!(st.submitted, 280, "ring 1 whole");
}

/// A render half that logs its calls by frame: `"F tables S"` and `"F issue S"`, the slot
/// the call names (a submit's, its seed).
#[derive(Default)]
struct Calls {
    frame: u64,
    log: Vec<String>,
}

impl RenderHalf for Calls {
    fn poll(&mut self) {}
    fn tables(&mut self, opened: &[(usize, ChunkCoord)]) {
        for (slot, _) in opened {
            self.log.push(format!("{} tables {slot}", self.frame));
        }
    }
    fn refresh_tables(&mut self, _: &[(usize, ChunkCoord)]) {}
    fn write_params(&mut self) {}
    fn issue(&mut self, p: Prepared) {
        self.log.push(format!("{} issue {}", self.frame, p.seed));
    }
    fn collect(&mut self, _: usize) -> Option<ReadBack> {
        None
    }
    fn cap(&self) -> Option<usize> {
        None
    }
    fn apply(&mut self, _: Vec<WriteList>, _: u64) -> Vec<([i32; 3], u32)> {
        Vec::new()
    }
}

/// A step's ops: job slot `s`'s table, then a submit for it.
fn step_ops(s: u32) -> Vec<Op> {
    vec![
        Op::Tables(vec![(s as usize, ChunkCoord([0, 0, 0]))]),
        Op::Issue(Prepared {
            input: Vec::new(),
            seed: s,
            clamp: false,
            sky: 0,
            key: None,
            timer: None,
        }),
    ]
}

/// Done when (1) of Plan 0099 Phase 4c. Two steps taken in one frame (tables A, issue A,
/// tables B, issue B) replay as tables A, issue A in that frame and tables B, issue B in the
/// next, ahead of that frame's own step; no frame issues two; a join with a submit carried
/// issues it, ahead of the steps it had not taken.
#[test]
fn a_frame_issues_one_submit_and_carries_the_rest_in_order() {
    let mut calls = Calls::default();
    let mut carried = Vec::new();
    calls.frame = 1;
    assert_eq!(
        replay_a_frame(
            &mut carried,
            vec![step_ops(1), step_ops(2)],
            &mut calls,
            None,
            None
        ),
        1
    );
    calls.frame = 2;
    assert_eq!(
        replay_a_frame(&mut carried, vec![step_ops(3)], &mut calls, None, None),
        1
    );
    assert_eq!(
        calls.log,
        ["1 tables 1", "1 issue 1", "2 tables 2", "2 issue 2"],
        "B in the next frame, ahead of its own step"
    );
    // A frame that took nothing issues what it carried.
    calls.frame = 3;
    assert_eq!(
        replay_a_frame(&mut carried, Vec::new(), &mut calls, None, None),
        1
    );
    assert!(carried.is_empty());
    assert_eq!(
        replay_a_frame(&mut carried, Vec::new(), &mut calls, None, None),
        0
    );
    // A join: the carried submit first, then the steps the thread had not handed over, all
    // replayed by the next lockstep step.
    calls.frame = 4;
    assert_eq!(
        replay_a_frame(
            &mut carried,
            vec![step_ops(4), step_ops(5)],
            &mut calls,
            None,
            None
        ),
        1
    );
    calls.frame = 5;
    for (ops, _) in carried_first(
        std::mem::take(&mut carried),
        vec![(step_ops(6), Vec::new())],
    ) {
        replay(ops, &mut calls, None);
    }
    assert_eq!(
        calls.log[4..],
        [
            "3 tables 3",
            "3 issue 3",
            "4 tables 4",
            "4 issue 4",
            "5 tables 5",
            "5 issue 5",
            "5 tables 6",
            "5 issue 6"
        ]
    );
    // No threaded frame (1 to 4) issued two.
    for f in 1..=4 {
        let n = calls
            .log
            .iter()
            .filter(|l| l.starts_with(&format!("{f} issue")))
            .count();
        assert_eq!(n, 1, "frame {f} issues one");
    }
}

/// A hook that holds every step whose message's frame is `from` or later until the sender is
/// dropped; `from` is set at run time (`u64::MAX`: none held).
fn hold_armed() -> (Hook, Arc<std::sync::atomic::AtomicU64>, mpsc::Sender<()>) {
    use std::sync::atomic::{AtomicU64, Ordering::SeqCst};
    let from = Arc::new(AtomicU64::new(u64::MAX));
    let (tx, rx) = mpsc::channel::<()>();
    let rx = Mutex::new(rx);
    let f = from.clone();
    let hook: Hook = Box::new(move |_, m| {
        if m.frame >= f.load(SeqCst) {
            let _ = rx.lock().expect("one step at a time").recv();
        }
    });
    (hook, from, tx)
}

/// Frames until the queue's books read idle with no list waiting (at most 400).
fn drain(t: &mut ThreadRig) {
    for s in 1..=400 {
        t.frame_stepped(s);
        if t.link.books.idle && t.lists.waiting.is_empty() {
            break;
        }
    }
    t.frame().expect("the last step taken");
    assert!(t.link.books.idle, "the queue drains within 400 frames");
}

/// Whether the writes since `from` hold `c`'s block's words (the books' place) and its table
/// word.
fn block_and_table(t: &ThreadRig, from: usize, c: [i32; 3]) -> bool {
    let (first, words) = *t
        .link
        .books
        .blocks
        .get(&c)
        .expect("the chunk keeps a block");
    let mut table = Store::new(64 << 20);
    table.start_queue(t.rig.mirror.window(), t.rig.queue.settings.cell);
    let table = table.table_word(ChunkCoord(c)) * 4;
    let applied = &t.applied[from..];
    applied
        .iter()
        .any(|a| a.kind == W_POOL && a.offset == first * 4 && a.bytes as u64 == words * 4)
        && applied
            .iter()
            .any(|a| a.kind == W_TABLE && a.offset == table && a.bytes == 4)
}

/// Done when (1) of Plan 0099 Phase 3b (ADR-0083's one wait). The slab drained, then every
/// step held by a hook: 100 frames with no box return within 2 s and apply nothing (Phase 3's
/// (1) beside it). A box across chunks 0 and 1 is then carved and the hook let go from a second
/// thread 100 ms on: the box's frame returns only after that, with the books of the step that
/// read its message taken, both chunks' held blocks and table words applied in that frame, and
/// that step's submit issued in it.
#[test]
fn the_edits_frame_waits_for_its_list_and_no_other_frame_does() {
    use std::sync::atomic::Ordering::SeqCst;
    let (hook, from, gate) = hold_armed();
    let mut t = ThreadRig::new(Some(hook), OUTBOX_BYTES);
    drain(&mut t);
    from.store(t.rig.half.frame + 1, SeqCst);
    t.frame().expect("the held step's message goes out");
    assert!(
        within(|| t.link.peek(|m| m.is_none())),
        "the step took its message and is held"
    );
    t.frame().expect("the frame returns");
    let applied = t.applied.len();
    let at = stopwatch();
    for f in 0..100 {
        t.rig.mirror.changed.push(ChunkCoord([f % 2, 0, 0]));
        t.frame().expect("the frame returns");
    }
    assert!(seconds_since(at) < 2.0, "100 frames within 2 s");
    assert_eq!(t.applied.len(), applied, "nothing applied while held");
    let (before, issued) = (t.applied.len(), t.rig.half.issued.len());
    t.carve([60, 20, 4], [67, 23, 7]);
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(100));
        drop(gate);
    });
    let at = stopwatch();
    t.frame().expect("the edit's frame returns with its list");
    let waited = seconds_since(at);
    release.join().expect("released");
    let box_frame = t.rig.half.frame;
    assert!(waited >= 0.1, "returned after the release: {waited} s");
    assert!(
        t.link.books.frame >= box_frame,
        "the box's step taken: books of message {} at frame {box_frame}",
        t.link.books.frame
    );
    for c in [[0, 0, 0], [1, 0, 0]] {
        assert!(
            block_and_table(&t, before, c),
            "chunk {c:?}'s held block and table word in the edit's frame"
        );
    }
    assert!(
        t.rig.half.issued[issued..]
            .iter()
            .any(|(f, _)| *f == box_frame),
        "the dig's submit issued in the edit's frame: {:?}",
        t.rig.half.issued[issued..]
            .iter()
            .map(|(f, s)| (*f, s.len()))
            .collect::<Vec<_>>()
    );
}

/// Done when (2) of Plan 0099 Phase 3b. A panic in the step that takes the box is the box
/// frame's error, returned within 2 s, not a hang. With the outbox at its cap when the box is
/// sent (the render side sending and not taking until the bake side stops stepping), the box
/// frame returns with the dig's held block and table word applied.
#[test]
fn a_wait_ends_on_a_panic_and_a_full_outbox_does_not_hold_it() {
    use std::sync::atomic::{AtomicU64, Ordering::SeqCst};
    let from = Arc::new(AtomicU64::new(u64::MAX));
    let f = from.clone();
    let hook: Hook =
        Box::new(move |_, m| assert!(m.frame < f.load(SeqCst), "the dig's step breaks"));
    let mut t = ThreadRig::new(Some(hook), OUTBOX_BYTES);
    drain(&mut t);
    from.store(t.rig.half.frame + 1, SeqCst);
    t.carve([0, 20, 4], [3, 23, 7]);
    let at = stopwatch();
    let got = t.frame();
    assert!(seconds_since(at) < 2.0);
    assert_eq!(
        got.err(),
        Some(BakeError::Thread("the dig's step breaks".into()))
    );

    const CAP: usize = 16 << 10;
    let mut t = ThreadRig::new(None, CAP);
    drain(&mut t);
    let mut last = u64::MAX;
    let mut flat = 0;
    let at = stopwatch();
    while flat < 200 {
        assert!(seconds_since(at) < 1.8, "the steps stop within 1.8 s");
        let msg = t.rig.message();
        t.link.send(msg, Vec::new());
        std::thread::sleep(std::time::Duration::from_millis(1));
        let s = t.link.steps();
        flat = if s == last { flat + 1 } else { 0 };
        last = s;
    }
    assert!(t.link.outbox_bytes() > CAP / 2, "the outbox is at its cap");
    let before = t.applied.len();
    t.carve([0, 20, 4], [3, 23, 7]);
    let at = stopwatch();
    t.frame().expect("the edit's frame returns");
    assert!(seconds_since(at) < 2.0);
    assert!(
        t.link.books.frame >= t.rig.half.frame,
        "the box's step taken"
    );
    assert!(
        block_and_table(&t, before, [0, 0, 0]),
        "the dig's held block and table word applied"
    );
}

/// Done when (3) of Plan 0099 Phase 3b. A carried submit (tables A, issue A) and the dig's step
/// (tables B, issue B) replay as all four in the edit's frame, in order; the same two with no
/// box replay as Phase 4c's (1) has them, one submit a frame.
#[test]
fn a_dig_frame_issues_a_carried_submit_and_its_own_in_order() {
    let mut calls = Calls::default();
    let mut carried = step_ops(1);
    calls.frame = 1;
    assert_eq!(
        replay_a_frame(&mut carried, vec![step_ops(2)], &mut calls, None, Some(0)),
        2
    );
    assert!(carried.is_empty());
    assert_eq!(
        calls.log,
        ["1 tables 1", "1 issue 1", "1 tables 2", "1 issue 2"],
        "the carried submit, then the dig's, in the edit's frame"
    );
    let mut calls = Calls::default();
    let mut carried = step_ops(1);
    calls.frame = 1;
    assert_eq!(
        replay_a_frame(&mut carried, vec![step_ops(2)], &mut calls, None, None),
        1
    );
    calls.frame = 2;
    assert_eq!(
        replay_a_frame(&mut carried, Vec::new(), &mut calls, None, None),
        1
    );
    assert_eq!(
        calls.log,
        ["1 tables 1", "1 issue 1", "2 tables 2", "2 issue 2"],
        "with no box one submit a frame"
    );
}
