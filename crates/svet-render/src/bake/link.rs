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

use super::thread::{BakeThread, Weighed};
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
}

/// A test's hook on the bake thread's step.
#[cfg(test)]
pub(crate) type Hook = Box<dyn FnMut(&BakeQueue, &FrameMessage) + Send>;

/// One message to the bake thread: the frame's and the read-backs mapped since the last.
pub(crate) struct ThreadMsg {
    pub(crate) msg: FrameMessage,
    pub(crate) read_backs: Vec<ReadBack>,
}

impl ThreadMsg {
    /// `self` followed by `newer`, as one: the read-backs appended in order.
    fn coalesce(self, newer: ThreadMsg) -> ThreadMsg {
        let mut read_backs = self.read_backs;
        read_backs.extend(newer.read_backs);
        ThreadMsg {
            msg: self.msg.coalesce(newer.msg),
            read_backs,
        }
    }
}

/// One bake step's results: the render-half calls in order, the write lists, the step's
/// stats or error, and the books' snapshot after it.
pub(crate) struct Made {
    pub(crate) ops: Vec<Op>,
    pub(crate) lists: Vec<WriteList>,
    pub(crate) stats: Result<QueueStats, BakeError>,
    pub(crate) books: Arc<Books>,
}

impl Weighed for Made {
    fn bytes(&self) -> usize {
        let lists: usize = self.lists.iter().map(|l| l.bytes).sum();
        let input: usize = self
            .ops
            .iter()
            .map(|o| match o {
                Op::Issue(p) => p.input.len() * FACE_WORDS * 4,
                _ => 0,
            })
            .sum();
        lists + input + self.books.bytes()
    }
}

/// One step on the bake thread: the message's read-backs into a recording half, the step, and
/// the books' snapshot.
pub(crate) fn bake_step(side: &mut BakeSide, m: ThreadMsg, cap: Option<usize>) -> Vec<Made> {
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
    let books = Arc::new(Books::of(&side.queue, &side.store, frame, eye));
    vec![Made {
        ops: half.ops,
        lists: half.lists,
        stats,
        books,
    }]
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

/// The render thread's handle on the bake thread, and the newest books it took.
pub(crate) struct ThreadLink {
    thread: BakeThread<BakeSide, ThreadMsg, Made>,
    pub(crate) books: Arc<Books>,
    /// The caller's dispatch timer ([`BakeSettings::timer`]): a `wgpu` type, kept on the render
    /// thread and put on each submit as it is issued.
    pub(crate) timer: Option<Arc<crate::frame::PassTimer>>,
    /// The queue's settings as the thread started with them: the render half's parameters
    /// read them.
    pub(crate) settings: BakeSettings,
    /// The newest stats taken, given again (per-step counts zeroed) on a frame that takes none.
    last: QueueStats,
}

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
        let thread = BakeThread::start(side, outbox, ThreadMsg::coalesce, move |s, m| {
            bake_step(s, m, cap)
        })?;
        Ok(Self {
            thread,
            books,
            timer,
            settings,
            last,
        })
    }

    /// The bake thread's panic, once, if it ended by one.
    pub(crate) fn failed(&mut self) -> Option<BakeError> {
        self.thread.failed().map(BakeError::Thread)
    }

    /// Takes what the bake thread made since the last call; never waits on a step.
    pub(crate) fn take(&mut self) -> Taken {
        let mut out = Taken::default();
        for m in self.thread.take() {
            out.frames.push(m.books.frame);
            self.books = m.books;
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
        self.thread.send(ThreadMsg { msg, read_backs });
    }

    /// Stops and joins the bake thread: what it owns, the message it had not taken (merged into
    /// the next step's), and what it made that was not taken, in order. `Err` with the panic
    /// that ended it.
    pub(crate) fn stop(self) -> Result<Stopped, BakeError> {
        let timer = self.timer;
        let (mut side, msg, made) = self.thread.stop_all().map_err(BakeError::Thread)?;
        side.queue.settings.timer = timer;
        Ok(Stopped {
            side,
            msg,
            steps: made.into_iter().map(|m| (m.ops, m.lists)).collect(),
        })
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
}

/// The read-backs that have mapped, oldest first, without waiting.
pub(crate) fn mapped(half: &mut dyn RenderHalf) -> Vec<ReadBack> {
    half.poll();
    std::iter::from_fn(|| half.collect(usize::MAX)).collect()
}

/// The render half's frame with the bake thread on: its panic, if it ended by one; else each
/// step's calls replayed
/// on `half` in order and its lists taken, the lists applied under the cap into `out`, the
/// mailbox answered up to what the applied lists were made from, and `msg` sent with the
/// landing words written and the read-backs that mapped. The stats are the steps' taken, or
/// the newest's with no per-step count when none was.
pub(crate) fn render_frame(
    link: &mut ThreadLink,
    half: &mut dyn RenderHalf,
    lists: &mut Lists,
    out: &mut dyn Applier,
    inbox: &mut Inbox,
    mut msg: FrameMessage,
) -> Result<QueueStats, BakeError> {
    if let Some(e) = link.failed() {
        return Err(e);
    }
    let taken = link.take();
    let frame = msg.frame;
    let mut all_empty = None;
    for (step, (ops, made)) in taken.steps.into_iter().enumerate() {
        replay(ops, half, link.timer.as_ref());
        if made.is_empty() {
            all_empty = Some(taken.frames[step]);
        }
        let ok = lists.take(made);
        debug_assert!(ok.is_ok(), "a write list over the frame's cap");
    }
    // A first block's landing word is this frame's plus one, as lockstep writes it.
    let at = msg
        .settings
        .landed_word
        .filter(|w| *w != 0)
        .map_or(0, |w| u64::from(w) - 1);
    let _ = lists.apply(at, out);
    // The mailbox answers from the books once the lists made from its messages are applied; a
    // step that made none answers its message once nothing waits.
    let mut answered = lists.newest;
    if lists.waiting.is_empty()
        && let Some(f) = all_empty
    {
        answered = answered.max(f);
    }
    inbox.answered(answered);
    let read_backs = mapped(half);
    msg.landed.extend(lists.report(frame));
    link.send(msg, read_backs);
    taken.stats.unwrap_or_else(|| Ok(link.idle_stats()))
}
