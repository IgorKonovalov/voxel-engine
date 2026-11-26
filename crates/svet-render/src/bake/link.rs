//! The live queue's bake half on the bake thread (ADR-0083): what the thread owns, the render
//! half it steps against (one that records, never calls), what each step hands back, and the
//! render thread's side of the frame.
//!
//! On the bake thread the step's [`RenderHalf`] is a [`Recorder`]: the candidate tables, the
//! parameters and the planned submits are recorded in order ([`Op`]), the read-backs are the
//! ones the message carried, and the write lists go out whole. The render thread takes the
//! outbox each frame, replays the ops through its own half (every `wgpu` call is its own),
//! applies the lists under the cap, and sends the next message with the landing words and the
//! read-backs that mapped. Nothing it calls waits on a step.

use super::thread::{BakeThread, Hand, Picked, Weighed};
use super::*;
use std::collections::VecDeque;

/// The bytes the outbox holds before the bake thread waits (ADR-0083: bounded, the bake thread
/// waits when it is full).
pub(crate) const OUTBOX_BYTES: usize = 8 << 20;

/// The bytes a frame applies at most with the bake thread on ([`Lists::cap`]).
pub(crate) const THREAD_LIST_CAP: usize = 1 << 20;

/// One render-half call the bake half made, in order.
pub(crate) enum Op {
    Tables(Vec<(usize, ChunkCoord)>),
    Refresh(Vec<(usize, ChunkCoord)>),
    Params,
    Issue(Prepared),
}

/// The bake thread's render half: records every call; its read-backs are the message's.
pub(crate) struct Recorder {
    pub(crate) ops: Vec<Op>,
    pub(crate) read_backs: VecDeque<ReadBack>,
    pub(crate) cap: Option<usize>,
    pub(crate) lists: Vec<WriteList>,
}

impl RenderHalf for Recorder {
    fn poll(&mut self) {}

    fn tables(&mut self, opened: &[(usize, ChunkCoord)]) {
        if !opened.is_empty() {
            self.ops.push(Op::Tables(opened.to_vec()));
        }
    }

    fn refresh_tables(&mut self, open: &[(usize, ChunkCoord)]) {
        self.ops.push(Op::Refresh(open.to_vec()));
    }

    fn write_params(&mut self) {
        self.ops.push(Op::Params);
    }

    fn issue(&mut self, p: Prepared) {
        self.ops.push(Op::Issue(p));
    }

    fn collect(&mut self, _unread: usize) -> Option<ReadBack> {
        // Never waits: the render thread sends what mapped.
        self.read_backs.pop_front()
    }

    fn cap(&self) -> Option<usize> {
        self.cap
    }

    fn apply(&mut self, lists: Vec<WriteList>, _frame: u64) -> Vec<([i32; 3], u32)> {
        // The words written come back in a later message ([`FrameMessage::landed`]).
        self.lists.extend(lists);
        Vec::new()
    }
}

/// What the bake thread owns: the books, the store's copy, the shadow of the mirror and the
/// world it lists from.
pub(crate) struct BakeSide {
    pub(crate) queue: BakeQueue,
    pub(crate) store: Store,
    pub(crate) shadow: Shadow,
    pub(crate) world: Arc<svet_world::World>,
    /// A test's hook, run at the start of every step (a hold, a panic).
    #[cfg(test)]
    pub(crate) hook: Option<Hook>,
    /// A test's hook, run after a step's results are handed in and before its books' snapshot,
    /// with the step's message's frame.
    #[cfg(test)]
    pub(crate) books_hook: Option<BooksHook>,
}

/// A test's hook on the bake thread's step.
#[cfg(test)]
pub(crate) type Hook = Box<dyn FnMut(&BakeQueue, &FrameMessage) + Send>;

/// A test's hook before a step's books' snapshot.
#[cfg(test)]
pub(crate) type BooksHook = Box<dyn FnMut(u64) + Send>;

/// One message to the bake thread: the frame's and the read-backs mapped since the last.
pub(crate) struct ThreadMsg {
    pub(crate) msg: FrameMessage,
    pub(crate) read_backs: Vec<ReadBack>,
    /// The render side's count of messages sent with a box, as of this one ([`BoxSender`]).
    pub(crate) boxes: u64,
}

impl ThreadMsg {
    /// `self` followed by `newer`, as one: the read-backs appended in order.
    fn coalesce(self, newer: ThreadMsg) -> ThreadMsg {
        let mut read_backs = self.read_backs;
        read_backs.extend(newer.read_backs);
        ThreadMsg {
            msg: self.msg.coalesce(newer.msg),
            read_backs,
            boxes: self.boxes.max(newer.boxes),
        }
    }
}

/// One entry of the bake thread's outbox (ADR-0083): a step's results, handed in when the step
/// returns, then the books' snapshot after that step as an entry of its own.
pub(crate) enum Made {
    Step(Box<Stepped>),
    Books(Snapshot),
}

/// One bake step's results: the render-half calls in order, the write lists, the step's stats
/// or error, the frame of the message it read, and whether its queue traced digs.
pub(crate) struct Stepped {
    pub(crate) ops: Vec<Op>,
    pub(crate) lists: Vec<WriteList>,
    pub(crate) stats: Result<QueueStats, BakeError>,
    pub(crate) frame: u64,
    /// [`BakeQueue::trace_digs`] as the step ran: an edit's frame then waits for its books too.
    pub(crate) trace: bool,
    /// When the step ran (a reading of the edit's wait by its parts).
    pub(crate) times: StepTimes,
}

/// The books after a step, with when their snapshot started and ended and its ms (a reading).
pub(crate) struct Snapshot {
    pub(crate) books: Arc<Books>,
    pub(crate) at: std::time::Instant,
    pub(crate) end: std::time::Instant,
    pub(crate) ms: f64,
}

/// When a step on the bake thread ran: when the thread was free and took its message, where
/// the step's clock started, the step's milliseconds to the end of its `boxes` stage, the
/// step's return, and the end of the part of the hand-back the edit's frame waits for: the
/// step's return, or under [`BakeQueue::trace_digs`] the end of its books' snapshot.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StepTimes {
    pub(crate) picked: Picked,
    pub(crate) start: std::time::Instant,
    pub(crate) boxes_ms: f64,
    pub(crate) step_end: std::time::Instant,
    pub(crate) books_end: std::time::Instant,
}

/// The parts of an edit's frame's wait, in ms (a reading, ADR-0083's one wait), in order: the
/// rest of a step in progress at the send (the thread not free), the pick-up (from the send, or
/// the thread free if later, to the dig's step taking its message), the step to the end of its
/// `boxes` stage, the step's rest, the books' snapshot (0 unless the queue traces digs
/// ([`BakeQueue::trace_digs`]), when the frame waits for the dig's books and their trace), and
/// the hand-back (from the step's return, or the snapshot's end, to the render thread holding
/// the take). `span` is the wait itself, from the send's instant to the take's return: the
/// parts are spans between instants in order, so they sum to it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct DigWaitParts {
    pub(crate) ms: [f64; 6],
    pub(crate) span: f64,
}

impl DigWaitParts {
    /// The parts of a wait from `send` (just before the send) to `woke` (the take returned),
    /// for the step whose times are `t`.
    pub(crate) fn of(send: std::time::Instant, woke: std::time::Instant, t: &StepTimes) -> Self {
        let ms = |a: std::time::Instant, b: std::time::Instant| {
            b.saturating_duration_since(a).as_secs_f64() * 1e3
        };
        let ready = send.max(t.picked.free);
        let boxes_end = t.start + std::time::Duration::from_secs_f64(t.boxes_ms.max(0.0) * 1e-3);
        let boxes_end = boxes_end.min(t.step_end);
        Self {
            ms: [
                ms(send, t.picked.free),
                ms(ready, t.picked.took),
                ms(t.picked.took.max(send), boxes_end),
                ms(boxes_end, t.step_end),
                ms(t.step_end, t.books_end),
                ms(t.books_end, woke),
            ],
            span: ms(send, woke),
        }
    }

    /// The parts' sum.
    #[cfg(test)]
    pub(crate) fn sum(&self) -> f64 {
        self.ms.iter().sum()
    }
}

/// One cycle of the bake thread from the instants its clock took in order (a reading): free
/// for a message, the message taken, the step's return, the books' snapshot's start and end,
/// and free for the next message once both entries are in the outbox.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Cycle {
    pub(crate) free: std::time::Instant,
    pub(crate) took: std::time::Instant,
    pub(crate) step_end: std::time::Instant,
    pub(crate) at: std::time::Instant,
    pub(crate) end: std::time::Instant,
    pub(crate) next_free: std::time::Instant,
}

impl Cycle {
    /// The parts in ms, in order: the wait for a message, the step, the books' snapshot, and
    /// the hand-over (the step's results put in the outbox before the snapshot, the books
    /// after it). They are spans between the instants, so they sum to [`Cycle::span`].
    pub(crate) fn parts(&self) -> [f64; 4] {
        let ms = |a: std::time::Instant, b: std::time::Instant| {
            b.saturating_duration_since(a).as_secs_f64() * 1e3
        };
        [
            ms(self.free, self.took),
            ms(self.took, self.step_end),
            ms(self.at, self.end),
            ms(self.step_end, self.at) + ms(self.end, self.next_free),
        ]
    }

    /// From one step's pick-up's `free` to the next step's, in ms.
    #[cfg(test)]
    pub(crate) fn span(&self) -> f64 {
        self.next_free
            .saturating_duration_since(self.free)
            .as_secs_f64()
            * 1e3
    }
}

impl Weighed for Made {
    fn bytes(&self) -> usize {
        match self {
            Made::Step(s) => {
                let lists: usize = s.lists.iter().map(|l| l.bytes).sum();
                let input: usize = s
                    .ops
                    .iter()
                    .map(|o| match o {
                        Op::Issue(p) => p.input.len() * FACE_WORDS * 4,
                        _ => 0,
                    })
                    .sum();
                lists + input
            }
            Made::Books(b) => b.books.bytes(),
        }
    }
}

/// One step on the bake thread: the message's read-backs into a recording half and the step,
/// whose results are handed in through `hand` when it returns; then the books' snapshot,
/// returned as an entry of its own.
pub(crate) fn bake_step(
    side: &mut BakeSide,
    mut m: ThreadMsg,
    cap: Option<usize>,
    picked: Picked,
    hand: Hand<'_, Made>,
) -> Vec<Made> {
    // The step's laps read this thread's clock and `wgpu` call tally (thread-local): its start
    // is the bake thread's, taken when it picks the message up, not the render thread's send.
    m.msg.start = StepStart::now();
    let start = m.msg.start.at;
    if let Some(s) = side.queue.box_signal.as_mut() {
        s.took(m.boxes);
    }
    #[cfg(test)]
    if let Some(hook) = side.hook.as_mut() {
        hook(&side.queue, &m.msg);
    }
    let (frame, eye) = (m.msg.frame, m.msg.view.eye);
    let mut half = Recorder {
        ops: Vec::new(),
        read_backs: m.read_backs.into(),
        cap,
        lists: Vec::new(),
    };
    let stats = side.queue.step(
        &mut side.shadow,
        &side.world,
        &mut side.store,
        m.msg,
        &mut half,
    );
    debug_assert!(
        half.read_backs.is_empty(),
        "every read-back sent answers a submit the bake half planned"
    );
    let step_end = stopwatch();
    // The stages from the step's start to the end of `boxes` are laps of one clock, in order;
    // the summaries' later move out of `gone` and `rewindow` can only shorten this sum.
    let boxes_ms = stats
        .as_ref()
        .map_or(0.0, |s| s.stages.ms[..=super::stages::S_BOXES].iter().sum());
    let times = StepTimes {
        picked,
        start,
        boxes_ms,
        step_end,
        books_end: step_end,
    };
    hand(vec![Made::Step(Box::new(Stepped {
        ops: half.ops,
        lists: half.lists,
        stats,
        frame,
        trace: side.queue.trace_digs,
        times,
    }))]);
    #[cfg(test)]
    if let Some(hook) = side.books_hook.as_mut() {
        hook(frame);
    }
    let at = stopwatch();
    let books = Arc::new(Books::of(&side.queue, &side.store, frame, eye));
    let end = stopwatch();
    vec![Made::Books(Snapshot {
        books,
        at,
        end,
        ms: end.duration_since(at).as_secs_f64() * 1e3,
    })]
}

/// What the render thread took of the outbox in one frame.
#[derive(Default)]
pub(crate) struct Taken {
    /// The ops and lists in order, step by step, and the message's frame each step read.
    pub(crate) steps: Vec<(Vec<Op>, Vec<WriteList>)>,
    pub(crate) frames: Vec<u64>,
    /// The newest step's stats, its per-step counts summed over the steps taken; `None` when
    /// no step was taken.
    pub(crate) stats: Option<Result<QueueStats, BakeError>>,
}

/// The bake thread's books' snapshots a [`ThreadLink`] keeps the ms of, about two minutes of
/// steps at 500 a second.
const SNAPSHOT_STEPS: usize = 1 << 16;

/// The render thread's handle on the bake thread, and the newest books it took.
pub(crate) struct ThreadLink {
    thread: BakeThread<BakeSide, ThreadMsg, Made>,
    pub(crate) books: Arc<Books>,
    /// The newest message frame whose step's lists were taken.
    pub(crate) lists_frame: u64,
    /// The newest message frame the applied lists answer (the mailbox answers up to the older of
    /// this and the books' frame).
    lists_answered: u64,
    /// The ms of each books' snapshot taken, in order (a reading; at most [`SNAPSHOT_STEPS`]).
    pub(crate) snapshot_ms: Vec<f32>,
    /// The caller's dispatch timer ([`BakeSettings::timer`]): a `wgpu` type, kept on the render
    /// thread and put on each submit as it is issued.
    pub(crate) timer: Option<Arc<crate::frame::PassTimer>>,
    /// The queue's settings as the thread started with them: the render half's parameters
    /// read them.
    pub(crate) settings: BakeSettings,
    /// The newest stats taken, given again (per-step counts zeroed) on a frame that takes none.
    last: QueueStats,
    /// Counts the messages sent with a box, for the bake thread's opening.
    boxes: BoxSender,
    /// The last frame's render-side laps in ms ([`RENDER_LAPS`]), the steps it took, and the
    /// blocks fading in the books it held before it took them (a reading).
    pub(crate) laps: [f64; RENDER_LAPS.len()],
    pub(crate) took: usize,
    pub(crate) fading_before: u64,
    /// The ops taken past a frame's first submit, in order: the next frame replays them first
    /// (ADR-0083: a frame issues at most one prepared submit).
    pub(crate) carried: Vec<Op>,
    /// The frame each submit issued and not yet read back went out in, oldest first.
    issued_at: VecDeque<u64>,
    /// The round trips read since the frame last took them ([`BakeRoundTrip`]).
    pub(crate) round_trip: BakeRoundTrip,
    /// The submits the last frame issued.
    pub(crate) issued_now: usize,
    /// Whether the last frame carried an edit's box, and so waited for its step.
    pub(crate) waited: bool,
    /// The last edit's frame's wait by its parts ([`DigWaitParts`]); `None` before any.
    pub(crate) parts: Option<DigWaitParts>,
    /// The newest step taken, its times and its books' snapshot's start and end once taken:
    /// its cycle ends at the next step's `free` ([`Cycle`]).
    open_cycle: Option<(StepTimes, Option<(std::time::Instant, std::time::Instant)>)>,
    /// The cycles the last take completed, in order (a reading).
    pub(crate) cycles: Vec<Cycle>,
}

/// The render side's laps in [`render_frame`]: the take (the previous books dropped with it),
/// the submits issued, the lists applied under the cap, the read-backs collected, the send, and
/// on an edit's frame the wait for its step (the take then holds only what came after it).
pub(crate) const RENDER_LAPS: [&str; 6] = ["take", "issue", "apply", "collect", "send", "wait"];

impl ThreadLink {
    /// Starts the bake thread on `side`, its lists made under `cap`, its outbox holding
    /// `outbox` bytes before it waits ([`OUTBOX_BYTES`]).
    pub(crate) fn start(
        mut side: BakeSide,
        cap: Option<usize>,
        outbox: usize,
    ) -> std::io::Result<Self> {
        let timer = side.queue.settings.timer.take();
        let settings = side.queue.settings.clone();
        let books = Arc::new(Books::of(
            &side.queue,
            &side.store,
            side.queue.frame,
            [f32::NAN; 3],
        ));
        let last = side.queue.stats;
        let boxes = BoxSender::default();
        side.queue.box_signal = Some(boxes.signal());
        let thread = BakeThread::start(side, outbox, ThreadMsg::coalesce, move |s, m, p, h| {
            bake_step(s, m, cap, p, h)
        })?;
        let frame = books.frame;
        Ok(Self {
            thread,
            books,
            lists_frame: frame,
            lists_answered: 0,
            snapshot_ms: Vec::new(),
            timer,
            settings,
            last,
            boxes,
            laps: [0.0; RENDER_LAPS.len()],
            took: 0,
            fading_before: 0,
            carried: Vec::new(),
            issued_at: VecDeque::new(),
            round_trip: BakeRoundTrip::default(),
            issued_now: 0,
            waited: false,
            parts: None,
            open_cycle: None,
            cycles: Vec::new(),
        })
    }

    /// The bake thread's panic, once, if it ended by one.
    pub(crate) fn failed(&mut self) -> Option<BakeError> {
        self.thread.failed().map(BakeError::Thread)
    }

    /// Takes what the bake thread made since the last call; never waits on a step.
    pub(crate) fn take(&mut self) -> Taken {
        let made = self.thread.take();
        self.absorb(made)
    }

    /// The edit's frame's take (ADR-0083's one wait): what the bake thread made, waiting until
    /// the results of a step made from the message of `frame` or a later one are among it, and,
    /// where that step's queue traces digs, its books too. `Err` when the thread ended first (its
    /// panic).
    /// The wait's parts from `send`, the instant before the frame's send, are kept in
    /// [`ThreadLink::parts`].
    pub(crate) fn take_waiting(
        &mut self,
        frame: u64,
        send: std::time::Instant,
    ) -> Result<Taken, BakeError> {
        // The books follow their step's results in the outbox.
        let mut traced = false;
        let (made, found) = self.thread.take_until(|m| match m {
            Made::Step(s) if s.frame >= frame => {
                traced = s.trace;
                !s.trace
            }
            Made::Books(b) => traced && b.books.frame >= frame,
            Made::Step(_) => false,
        });
        let woke = stopwatch();
        let step = made.iter().find_map(|m| match m {
            Made::Step(s) if s.frame >= frame => Some((s.times, s.trace)),
            _ => None,
        });
        if let Some((mut times, trace)) = step {
            if trace
                && let Some(end) = made.iter().find_map(|m| match m {
                    Made::Books(b) if b.books.frame >= frame => Some(b.end),
                    _ => None,
                })
            {
                times.books_end = end;
            }
            self.parts = Some(DigWaitParts::of(send, woke, &times));
        }
        let taken = self.absorb(made);
        if found {
            return Ok(taken);
        }
        Err(self
            .failed()
            .unwrap_or_else(|| BakeError::Thread("the bake thread ended in a wait".into())))
    }

    /// The steps `made` holds as a frame's take, the newest books and stats kept.
    fn absorb(&mut self, made: Vec<Made>) -> Taken {
        let mut out = Taken::default();
        self.cycles.clear();
        for m in made {
            let m = match m {
                Made::Step(m) => *m,
                Made::Books(b) => {
                    if let Some((_, books)) = self.open_cycle.as_mut() {
                        *books = Some((b.at, b.end));
                    }
                    self.books = b.books;
                    if self.snapshot_ms.len() < SNAPSHOT_STEPS {
                        self.snapshot_ms.push(b.ms as f32);
                    }
                    continue;
                }
            };
            if let Some((t, Some((at, end)))) = self.open_cycle.take() {
                self.cycles.push(Cycle {
                    free: t.picked.free,
                    took: t.picked.took,
                    step_end: t.step_end,
                    at,
                    end,
                    next_free: m.times.picked.free,
                });
            }
            self.open_cycle = Some((m.times, None));
            out.frames.push(m.frame);
            self.lists_frame = m.frame;
            out.stats = Some(match (out.stats.take(), m.stats) {
                (_, Err(e)) | (Some(Err(e)), _) => Err(e),
                (Some(Ok(a)), Ok(b)) => Ok(b.after(&a)),
                (None, Ok(b)) => Ok(b),
            });
            out.steps.push((m.ops, m.lists));
        }
        if let Some(Ok(s)) = &out.stats {
            self.last = *s;
        }
        out
    }

    /// The stats a frame that took no step reads: the newest, with no per-step count.
    pub(crate) fn idle_stats(&self) -> QueueStats {
        self.last.idle()
    }

    /// Hands the bake thread the frame's message; it merges with one not taken yet.
    pub(crate) fn send(&self, msg: FrameMessage, read_backs: Vec<ReadBack>) {
        let holds_box = msg.arrivals.iter().any(|a| matches!(a, Arrival::Box(_)));
        let boxes = self.boxes.stamp(holds_box);
        self.thread.send(ThreadMsg {
            msg,
            read_backs,
            boxes,
        });
    }

    /// Stops and joins the bake thread: what it owns, the message it had not taken (merged into
    /// the next step's), and what it made that was not taken, in order. `Err` with the panic
    /// that ended it.
    pub(crate) fn stop(self) -> Result<Stopped, BakeError> {
        let timer = self.timer;
        let carried = self.carried;
        let (mut side, msg, made) = self.thread.stop_all().map_err(BakeError::Thread)?;
        side.queue.settings.timer = timer;
        // Lockstep's box is in its step's own message.
        side.queue.box_signal = None;
        Ok(Stopped {
            side,
            msg,
            steps: carried_first(
                carried,
                made.into_iter()
                    .filter_map(|m| match m {
                        Made::Step(m) => {
                            let m = *m;
                            Some((m.ops, m.lists))
                        }
                        Made::Books(_) => None,
                    })
                    .collect(),
            ),
        })
    }

    /// Blocks until the bake thread has stepped every message sent and handed in each step's
    /// entries, or waits at the outbox's cap, or has ended ([`BakeThread::wait_step`]). Takes
    /// nothing: the next frame's take holds what it made.
    pub(crate) fn wait_step(&self) {
        self.thread.wait_step();
    }

    /// The bake thread's steps so far.
    #[cfg(test)]
    pub(crate) fn steps(&self) -> u64 {
        self.thread.steps()
    }

    /// The bytes waiting in the outbox.
    #[cfg(test)]
    pub(crate) fn outbox_bytes(&self) -> usize {
        self.thread.outbox_bytes()
    }

    /// The render side's count of messages sent with a box, shared.
    #[cfg(test)]
    pub(crate) fn box_sender(&self) -> BoxSender {
        self.boxes.clone()
    }

    /// The message waiting for the bake thread, read in place.
    #[cfg(test)]
    pub(crate) fn peek<R>(&self, read: impl FnOnce(Option<&ThreadMsg>) -> R) -> R {
        self.thread.peek(read)
    }
}

/// A stopped bake thread's state, given back to lockstep.
pub(crate) struct Stopped {
    pub(crate) side: BakeSide,
    pub(crate) msg: Option<ThreadMsg>,
    pub(crate) steps: Vec<(Vec<Op>, Vec<WriteList>)>,
}

/// Replays a step's render-half calls on `half`, in order, each submit timed on `timer`.
pub(crate) fn replay(
    ops: Vec<Op>,
    half: &mut dyn RenderHalf,
    timer: Option<&Arc<crate::frame::PassTimer>>,
) {
    for op in ops {
        replay_op(op, half, timer);
    }
}

/// A frame's replay with the bake thread on: `carried` (the ops an earlier frame left) and then
/// each step's ops, in order, replayed on `half` up to and with the first submit (ADR-0083: a
/// frame issues at most one prepared submit); the ops past it are left in `carried`, in order,
/// for the next frame. On an edit's frame `dig` is the index of the step that took the box: when
/// that step planned a submit, the replay goes on in order up to and with it, so the dig's
/// submit goes out in the edit's frame behind whatever was before it (ADR-0083's second
/// submit). The submits issued.
pub(crate) fn replay_a_frame(
    carried: &mut Vec<Op>,
    steps: Vec<Vec<Op>>,
    half: &mut dyn RenderHalf,
    timer: Option<&Arc<crate::frame::PassTimer>>,
    dig: Option<usize>,
) -> usize {
    let dig = dig.filter(|&d| {
        steps
            .get(d)
            .is_some_and(|ops| ops.iter().any(|o| matches!(o, Op::Issue(_))))
    });
    // Each op with its step's index; the carried ops are before every step.
    let mut ops = std::mem::take(carried)
        .into_iter()
        .map(|o| (None, o))
        .chain(
            steps
                .into_iter()
                .enumerate()
                .flat_map(|(k, ops)| ops.into_iter().map(move |o| (Some(k), o))),
        );
    let mut issued = 0;
    for (step, op) in ops.by_ref() {
        let issue = matches!(op, Op::Issue(_));
        replay_op(op, half, timer);
        if !issue {
            continue;
        }
        issued += 1;
        let at_end = match dig {
            Some(d) => step == Some(d),
            None => true,
        };
        if at_end {
            break;
        }
    }
    carried.extend(ops.map(|(_, o)| o));
    issued
}

/// A stopped thread's steps not taken, the ops a frame carried ahead of them.
pub(crate) fn carried_first(
    carried: Vec<Op>,
    made: Vec<(Vec<Op>, Vec<WriteList>)>,
) -> Vec<(Vec<Op>, Vec<WriteList>)> {
    std::iter::once((carried, Vec::new()))
        .filter(|(ops, _)| !ops.is_empty())
        .chain(made)
        .collect()
}

/// Replays one render-half call on `half`, a submit timed on `timer`.
fn replay_op(op: Op, half: &mut dyn RenderHalf, timer: Option<&Arc<crate::frame::PassTimer>>) {
    match op {
        Op::Tables(t) => half.tables(&t),
        Op::Refresh(t) => half.refresh_tables(&t),
        Op::Params => half.write_params(),
        Op::Issue(mut p) => {
            if p.timer.is_none() {
                p.timer = timer.cloned();
            }
            half.issue(p);
        }
    }
}

/// The read-backs that have mapped, oldest first, without waiting.
pub(crate) fn mapped(half: &mut dyn RenderHalf) -> Vec<ReadBack> {
    half.poll();
    std::iter::from_fn(|| half.collect(usize::MAX)).collect()
}

/// The render half's frame with the bake thread on: its panic, if it ended by one; else the
/// calls the last frame carried and each step's, replayed on `half` in order up to and with the
/// first submit (the rest carried to the next frame), each step's lists taken, the lists
/// applied under the cap into `out`, the mailbox answered up to the older of what the applied
/// lists were made from and the books held, and `msg` sent with the landing words written and
/// the read-backs that mapped. The stats are the steps' taken, or the newest's with no per-step
/// count when none was.
///
/// A frame whose message holds an edit's box (ADR-0083's one wait) sends first, with the
/// read-backs mapped so far, then waits for the results of the step made from that message or
/// a later one (not for its books, unless the queue traces digs), and replays and applies as
/// any frame does, the dig step's submit with it (behind any carried): the box's held block is
/// in the pool when the frame returns. The landing words it writes go with the next frame's
/// message.
pub(crate) fn render_frame(
    link: &mut ThreadLink,
    half: &mut dyn RenderHalf,
    lists: &mut Lists,
    out: &mut dyn Applier,
    inbox: &mut Inbox,
    msg: FrameMessage,
) -> Result<QueueStats, BakeError> {
    if let Some(e) = link.failed() {
        return Err(e);
    }
    let mut lap = stopwatch();
    let mut laps = [0.0; RENDER_LAPS.len()];
    let mut mark = |k: usize| {
        let now = stopwatch();
        laps[k] += now.duration_since(lap).as_secs_f64() * 1e3;
        lap = now;
    };
    link.fading_before = link.last.fading;
    let frame = msg.frame;
    let holds_box = msg.arrivals.iter().any(|a| matches!(a, Arrival::Box(_)));
    link.waited = holds_box;
    // A first block's landing word is this frame's plus one, as lockstep writes it.
    let landed_at = msg
        .settings
        .landed_word
        .filter(|w| *w != 0)
        .map_or(0, |w| u64::from(w) - 1);
    let mut pending = Some(msg);
    let issued_at = |link: &mut ThreadLink, read_backs: &[ReadBack]| {
        for _ in read_backs {
            if let Some(at) = link.issued_at.pop_front() {
                link.round_trip.count(frame.saturating_sub(at));
            }
        }
    };
    let taken = if holds_box {
        let read_backs = mapped(half);
        issued_at(link, &read_backs);
        mark(3);
        let send = stopwatch();
        link.send(pending.take().expect("the frame's message"), read_backs);
        mark(4);
        let taken = link.take_waiting(frame, send);
        mark(5);
        taken?
    } else {
        link.take()
    };
    link.took = taken.steps.len();
    mark(0);
    let mut all_empty = None;
    let dig = holds_box
        .then(|| taken.frames.iter().position(|f| *f >= frame))
        .flatten();
    let mut ops = Vec::with_capacity(taken.steps.len());
    for (step, (step_ops, made)) in taken.steps.into_iter().enumerate() {
        ops.push(step_ops);
        if made.is_empty() {
            all_empty = Some(taken.frames[step]);
        }
        let ok = lists.take(made);
        debug_assert!(ok.is_ok(), "a write list over the frame's cap");
    }
    link.issued_now = replay_a_frame(&mut link.carried, ops, half, link.timer.as_ref(), dig);
    for _ in 0..link.issued_now {
        link.issued_at.push_back(frame);
        link.round_trip.issued += 1;
    }
    link.round_trip.carried += u64::from(link.carried.iter().any(|o| matches!(o, Op::Issue(_))));
    mark(1);
    let _ = lists.apply(landed_at, out);
    mark(2);
    // The mailbox answers from the books once the lists made from its messages are applied and
    // the books held were made after them: a read finds a box in the mailbox or in the books,
    // never in neither. A step that made no list answers its message once nothing waits.
    let mut answered = lists.newest;
    if lists.waiting.is_empty()
        && let Some(f) = all_empty
    {
        answered = answered.max(f);
    }
    link.lists_answered = link.lists_answered.max(answered);
    inbox.answered(link.lists_answered.min(link.books.frame));
    if let Some(mut msg) = pending.take() {
        let read_backs = mapped(half);
        issued_at(link, &read_backs);
        mark(3);
        msg.landed.extend(lists.report(frame));
        link.send(msg, read_backs);
        mark(4);
    }
    link.laps = laps;
    taken.stats.unwrap_or_else(|| Ok(link.idle_stats()))
}
