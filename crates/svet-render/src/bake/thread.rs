//! The bake thread's frame (ADR-0083): one named thread that owns the bake half, a one-slot
//! inbox that coalesces the render thread's messages, and an outbox bounded in bytes.
//!
//! The render thread only swaps and tries: [`BakeThread::send`] replaces or merges the waiting
//! message, [`BakeThread::take`] drains what the bake half made. Neither waits on a step: each
//! lock is held for a swap, never across a step. A step may hand entries in before it returns
//! (its [`Hand`]); what it returns follows them. The bake thread waits when no message waits
//! or when the outbox is at its cap. A panic on it ends the thread; the render thread reads it
//! as an error at its next call ([`BakeThread::failed`]). Dropping the handle stops and joins.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Instant;

/// When the bake thread took a message: `free` is when it was last ready for one (its start, or
/// the end of the previous step's hand-in to the outbox) and `took` when it took this one from
/// the inbox (a reading of the edit's wait by its parts).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Picked {
    pub(crate) free: Instant,
    pub(crate) took: Instant,
}

/// A step's hand into the outbox: the entries go in at once, in order, ahead of what the step
/// returns (the bake thread waits here when the outbox is at its cap).
pub(crate) type Hand<'a, O> = &'a mut dyn FnMut(Vec<O>);

/// What the outbox weighs an entry by.
pub(crate) trait Weighed {
    fn bytes(&self) -> usize;
}

struct Outbox<O> {
    entries: Vec<O>,
    bytes: usize,
    /// Steps whose last entries are in (the step whole, its returned entries after any it
    /// handed in): with [`Shared::picked`] the bake thread's idle state.
    done: u64,
    /// Whether a put waits at the cap now.
    at_cap: bool,
}

struct Shared<M, O> {
    inbox: Mutex<Option<M>>,
    outbox: Mutex<Outbox<O>>,
    /// The bake thread's wait for a message, waited under the inbox's lock: a message arrived,
    /// or the stop.
    arrived: Condvar,
    /// The bake thread's wait at the outbox's cap, waited under the outbox's lock: room was
    /// made, or the stop.
    room: Condvar,
    /// The render thread's waits ([`BakeThread::take_until`], [`BakeThread::wait_step`]): an
    /// entry made, a put waiting at the cap, or the thread ended.
    made: Condvar,
    stop: AtomicBool,
    /// Set as the bake thread ends, by its stop or a panic, under the outbox's lock.
    ended: AtomicBool,
    cap: usize,
    /// Steps the bake thread has run: counted as a step's last entries go in, or as one of its
    /// entries starts to wait at the cap.
    steps: AtomicU64,
    /// Messages the bake thread has taken from the inbox: added under the inbox's lock as it
    /// takes one, so with the inbox empty `picked == done` holds only once every message sent
    /// has been stepped whole.
    picked: AtomicU64,
}

impl<M, O> Shared<M, O> {
    /// Sets the stop and wakes the thread: under each lock in turn, each wait's condition
    /// variable under its own lock, so the thread is either waiting (and woken) or past its
    /// check of the stop.
    fn halt(&self) {
        self.stop.store(true, Ordering::Release);
        {
            let _slot = Self::lock(&self.inbox);
            self.arrived.notify_all();
        }
        let _ob = Self::lock(&self.outbox);
        self.room.notify_all();
    }

    fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        // A panic on the bake thread never holds a lock: it runs only inside the step.
        m.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl<M, O: Weighed> Shared<M, O> {
    /// Puts `out` in the outbox, waiting while it is at its cap, and wakes a render thread
    /// waiting on an entry. The step is counted under the outbox's lock (`counted` once it is):
    /// as one of its puts starts to wait at the cap, or once its `last` entries are in.
    fn put(&self, out: Vec<O>, counted: &mut bool, last: bool) {
        let bytes: usize = out.iter().map(Weighed::bytes).sum();
        let mut ob = Self::lock(&self.outbox);
        while ob.bytes > 0 && ob.bytes + bytes > self.cap {
            if !*counted {
                self.steps.fetch_add(1, Ordering::Release);
                *counted = true;
            }
            // At the stop the step's entries still go in: the books already hold them, so the
            // joiner applies them ([`BakeThread::stop_all`]).
            if self.stop.load(Ordering::Acquire) {
                break;
            }
            if !ob.at_cap {
                ob.at_cap = true;
                self.made.notify_all();
            }
            ob = self.room.wait(ob).unwrap_or_else(PoisonError::into_inner);
        }
        ob.at_cap = false;
        if last {
            ob.done += 1;
        }
        ob.bytes += bytes;
        ob.entries.extend(out);
        if last && !*counted {
            self.steps.fetch_add(1, Ordering::Release);
            *counted = true;
        }
        drop(ob);
        self.made.notify_all();
    }
}

/// The handle the render thread holds; `S` is the bake half's state, given back at the stop.
pub(crate) struct BakeThread<S, M, O> {
    shared: Arc<Shared<M, O>>,
    handle: Option<JoinHandle<S>>,
    coalesce: fn(M, M) -> M,
}

impl<S, M, O> BakeThread<S, M, O>
where
    S: Send + 'static,
    M: Send + 'static,
    O: Weighed + Send + 'static,
{
    /// Starts the thread on `side`: each message it takes is stepped with `step`, whose entries
    /// (those it hands in through its [`Hand`], then those it returns) go to the outbox, which
    /// holds at most `cap` bytes before the thread waits (an entry larger than the cap goes
    /// alone into an empty outbox). `coalesce` merges a waiting message with a newer one. The
    /// step is told when the thread was free and when it took the message ([`Picked`]).
    pub(crate) fn start(
        mut side: S,
        cap: usize,
        coalesce: fn(M, M) -> M,
        mut step: impl FnMut(&mut S, M, Picked, Hand<'_, O>) -> Vec<O> + Send + 'static,
    ) -> std::io::Result<Self> {
        let shared = Arc::new(Shared {
            inbox: Mutex::new(None),
            outbox: Mutex::new(Outbox {
                entries: Vec::new(),
                bytes: 0,
                done: 0,
                at_cap: false,
            }),
            arrived: Condvar::new(),
            room: Condvar::new(),
            made: Condvar::new(),
            stop: AtomicBool::new(false),
            ended: AtomicBool::new(false),
            cap,
            steps: AtomicU64::new(0),
            picked: AtomicU64::new(0),
        });
        let sh = shared.clone();
        let handle = std::thread::Builder::new()
            .name("svet bake".into())
            .spawn(move || {
                // Wakes a render thread waiting on an entry when the thread ends, by its stop or
                // by a panic (dropped as the panic unwinds).
                let _ended = Ended(&*sh);
                let mut free = crate::bake::stopwatch();
                loop {
                    let msg = {
                        let mut slot = Shared::<M, O>::lock(&sh.inbox);
                        loop {
                            if sh.stop.load(Ordering::Acquire) {
                                return side;
                            }
                            if let Some(m) = slot.take() {
                                sh.picked.fetch_add(1, Ordering::Release);
                                break m;
                            }
                            slot = sh
                                .arrived
                                .wait(slot)
                                .unwrap_or_else(PoisonError::into_inner);
                        }
                    };
                    let took = crate::bake::stopwatch();
                    let mut counted = false;
                    let out = step(&mut side, msg, Picked { free, took }, &mut |o| {
                        sh.put(o, &mut counted, false);
                    });
                    sh.put(out, &mut counted, true);
                    free = crate::bake::stopwatch();
                }
            })?;
        Ok(Self {
            shared,
            handle: Some(handle),
            coalesce,
        })
    }

    /// Hands the bake half `msg`: it replaces no message, it is merged after the one waiting.
    pub(crate) fn send(&self, msg: M) {
        let mut slot = Shared::<M, O>::lock(&self.shared.inbox);
        let next = match slot.take() {
            Some(old) => (self.coalesce)(old, msg),
            None => msg,
        };
        *slot = Some(next);
        drop(slot);
        self.shared.arrived.notify_all();
    }

    /// What the bake half made since the last call, in order.
    pub(crate) fn take(&self) -> Vec<O> {
        let mut ob = Shared::<M, O>::lock(&self.shared.outbox);
        ob.bytes = 0;
        let out = std::mem::take(&mut ob.entries);
        drop(ob);
        self.shared.room.notify_all();
        out
    }

    /// What the bake half made since the last call, in order, waiting until an entry for which
    /// `done` holds is among them or the thread has ended (by a panic, read with
    /// [`BakeThread::failed`]). The outbox is taken as it fills, so the bake thread never waits on
    /// its cap against this wait. `true` when such an entry was taken.
    pub(crate) fn take_until(&self, mut done: impl FnMut(&O) -> bool) -> (Vec<O>, bool) {
        let mut out = Vec::new();
        let mut ob = Shared::<M, O>::lock(&self.shared.outbox);
        loop {
            if !ob.entries.is_empty() {
                ob.bytes = 0;
                let took = std::mem::take(&mut ob.entries);
                let found = took.iter().any(&mut done);
                out.extend(took);
                self.shared.room.notify_all();
                if found {
                    return (out, true);
                }
            }
            if self.shared.ended.load(Ordering::Acquire) {
                return (out, false);
            }
            ob = self
                .shared
                .made
                .wait(ob)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// Blocks until the bake thread has stepped every message sent to it and handed in each
    /// step's last entries, or waits at the outbox's cap, or has ended (by its stop or a panic,
    /// read with [`BakeThread::failed`]); at once when nothing was sent. It takes nothing from
    /// the outbox. No timeout: it waits on [`Shared::made`], which a put, a put's wait at the cap
    /// and the thread's end notify under the outbox's lock. A test's and a headless tool's wait:
    /// no frame path calls it.
    pub(crate) fn wait_step(&self) {
        let sh = &*self.shared;
        let mut ob = Shared::<M, O>::lock(&sh.outbox);
        loop {
            if sh.ended.load(Ordering::Acquire) || ob.at_cap {
                return;
            }
            // The inbox's lock is taken under the outbox's here and nowhere the other way
            // round. `done` moves only under the outbox's lock, held; `picked` only under the
            // inbox's, held for the read.
            let idle = {
                let slot = Shared::<M, O>::lock(&sh.inbox);
                slot.is_none() && sh.picked.load(Ordering::Acquire) == ob.done
            };
            if idle {
                return;
            }
            ob = sh.made.wait(ob).unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// The bytes waiting in the outbox.
    #[cfg(test)]
    pub(crate) fn outbox_bytes(&self) -> usize {
        Shared::<M, O>::lock(&self.shared.outbox).bytes
    }

    /// The steps the bake thread has run: counted as a step's last entries go in, or as one of
    /// its entries starts to wait at the outbox's cap.
    #[cfg(test)]
    pub(crate) fn steps(&self) -> u64 {
        self.shared.steps.load(Ordering::Acquire)
    }

    /// Whether a message waits for the bake thread.
    #[cfg(test)]
    pub(crate) fn waiting(&self) -> bool {
        Shared::<M, O>::lock(&self.shared.inbox).is_some()
    }

    /// `Some` once the bake thread has ended by a panic: its message. The handle is spent then.
    pub(crate) fn failed(&mut self) -> Option<String> {
        // Ended is set as the thread unwinds: the join below waits out the rest of it.
        let ended = self.shared.ended.load(Ordering::Acquire);
        if !self
            .handle
            .as_ref()
            .is_some_and(|h| ended || h.is_finished())
        {
            return None;
        }
        match self.handle.take()?.join() {
            Ok(_) => Some("the bake thread ended without a stop".into()),
            Err(p) => Some(panic_text(&*p)),
        }
    }

    /// Stops the thread and joins it: the bake half back, or the panic that ended it.
    #[cfg(test)]
    pub(crate) fn stop(mut self) -> Result<S, String> {
        self.halt()
            .unwrap_or_else(|| Err("the bake thread was already joined".into()))
    }

    /// Stops the thread and joins it: the bake half back with the message it had not taken and
    /// the entries the render thread had not taken, in order; or the panic that ended it.
    pub(crate) fn stop_all(mut self) -> Result<(S, Option<M>, Vec<O>), String> {
        let side = self
            .halt()
            .unwrap_or_else(|| Err("the bake thread was already joined".into()))?;
        let msg = Shared::<M, O>::lock(&self.shared.inbox).take();
        let out = std::mem::take(&mut Shared::<M, O>::lock(&self.shared.outbox).entries);
        Ok((side, msg, out))
    }

    /// The message waiting for the bake thread, read in place under the inbox's lock.
    #[cfg(test)]
    pub(crate) fn peek<R>(&self, read: impl FnOnce(Option<&M>) -> R) -> R {
        read(Shared::<M, O>::lock(&self.shared.inbox).as_ref())
    }

    fn halt(&mut self) -> Option<Result<S, String>> {
        self.shared.halt();
        let h = self.handle.take()?;
        Some(h.join().map_err(|p| panic_text(&*p)))
    }
}

impl<S, M, O> Drop for BakeThread<S, M, O> {
    fn drop(&mut self) {
        self.shared.halt();
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// The bake thread's guard: sets [`Shared::ended`] and wakes the render thread's wait as the
/// thread ends.
struct Ended<'a, M, O>(&'a Shared<M, O>);

impl<M, O> Drop for Ended<'_, M, O> {
    fn drop(&mut self) {
        let _ob = Shared::<M, O>::lock(&self.0.outbox);
        self.0.ended.store(true, Ordering::Release);
        self.0.made.notify_all();
    }
}

/// A panic's payload as text.
pub(crate) fn panic_text(p: &(dyn std::any::Any + Send)) -> String {
    p.downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| p.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic with no message".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bake::{seconds_since, stopwatch};
    use std::time::Duration;

    struct Entry(usize);
    impl Weighed for Entry {
        fn bytes(&self) -> usize {
            self.0
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

    /// The render thread's calls return while the step is held, and the messages sent
    /// meanwhile wait as one, merged in order.
    #[test]
    fn the_render_calls_return_while_a_step_is_held() {
        let (gate_tx, gate_rx) = std::sync::mpsc::channel::<()>();
        let gate = Mutex::new(gate_rx);
        let t = BakeThread::start(
            Vec::<Vec<u32>>::new(),
            1 << 20,
            |mut a: Vec<u32>, b: Vec<u32>| {
                a.extend(b);
                a
            },
            move |seen, m, _, _| {
                let _ = gate.lock().expect("one step at a time").recv();
                seen.push(m);
                vec![Entry(1)]
            },
        )
        .expect("spawned");
        t.send(vec![0]);
        assert!(
            within(|| !t.waiting()),
            "the step took message 0 and is held"
        );
        let at = stopwatch();
        for f in 1..=100 {
            t.send(vec![f]);
            assert!(
                t.take().is_empty(),
                "nothing is made while the step is held"
            );
        }
        assert!(seconds_since(at) < 0.5);
        gate_tx.send(()).expect("open");
        gate_tx.send(()).expect("open");
        assert!(within(|| t.steps() == 2));
        let seen = t.stop().expect("no panic");
        assert_eq!(seen, vec![vec![0], (1..=100).collect::<Vec<_>>()]);
    }

    /// With the render thread not taking, the outbox stops at its cap and the step count stops
    /// rising; a take makes room and the steps go on.
    #[test]
    fn the_outbox_stops_at_its_cap_and_the_bake_thread_waits() {
        let t = BakeThread::start((), 1000, |_a: u32, b: u32| b, |_, _, _, _| vec![Entry(300)])
            .expect("spawned");
        for _ in 0..10 {
            t.send(0);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(within(|| t.steps() >= 4));
        std::thread::sleep(Duration::from_millis(20));
        let held = t.steps();
        t.send(0);
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(t.steps(), held, "the step count stops at the cap");
        assert!(t.outbox_bytes() <= 1000);
        assert_eq!(t.take().len(), 3);
        assert!(
            within(|| t.steps() > held),
            "room made, the waiting step lands"
        );
        drop(t);
    }

    /// The wait for the step blocks while a step is held and returns only after the release
    /// from a second thread, with the step's entries in; it returns at once with nothing sent,
    /// when the thread waits at the cap, and after a panic ended the thread.
    #[test]
    fn the_step_wait_returns_after_the_held_step_and_never_before() {
        let (gate_tx, gate_rx) = std::sync::mpsc::channel::<()>();
        let (held_tx, held_rx) = std::sync::mpsc::channel::<()>();
        let gate = Mutex::new(gate_rx);
        let held = Mutex::new(held_tx);
        let t = BakeThread::start(
            (),
            1 << 20,
            |_a: u32, b: u32| b,
            move |_, _, _, hand| {
                let _ = held.lock().expect("one step at a time").send(());
                let _ = gate.lock().expect("one step at a time").recv();
                hand(vec![Entry(1)]);
                vec![Entry(2)]
            },
        )
        .expect("spawned");
        t.wait_step();
        assert_eq!(t.steps(), 0, "nothing sent: at once, no step");

        let released = Arc::new(AtomicBool::new(false));
        let at = Arc::new(Mutex::new(None::<Instant>));
        t.send(0);
        let releaser = {
            let (released, at) = (released.clone(), at.clone());
            std::thread::spawn(move || {
                held_rx.recv().expect("the step holds");
                *at.lock().expect("one writer") = Some(stopwatch());
                released.store(true, Ordering::Release);
                gate_tx.send(()).expect("open");
                // The second message's step goes through at once.
                held_rx.recv().expect("the second step holds");
                gate_tx.send(()).expect("open");
            })
        };
        t.wait_step();
        let back = stopwatch();
        assert!(
            released.load(Ordering::Acquire),
            "the wait returned only after the release"
        );
        let at = at.lock().expect("written").expect("released");
        assert!(
            at <= back,
            "the release's instant is at or before the return"
        );
        assert_eq!(t.steps(), 1);
        let made: Vec<usize> = t.take().iter().map(|e| e.0).collect();
        assert_eq!(made, vec![1, 2], "the step's entries, its last among them");

        // Three sends merge into at most two steps; the wait covers every message sent.
        for m in 1..=3 {
            t.send(m);
        }
        t.wait_step();
        let made = t.take().len();
        assert!(t.steps() >= 2, "the second step ran");
        assert_eq!(made as u64, (t.steps() - 1) * 2, "each step whole");
        assert!(!t.waiting(), "no message waits");
        releaser.join().expect("the releaser ran");
        drop(t);

        // At the cap: one step fills the outbox, the next waits at the cap and so does the wait.
        let t = BakeThread::start((), 10, |_a: u32, b: u32| b, |_, _, _, _| vec![Entry(10)])
            .expect("spawned");
        t.send(0);
        t.wait_step();
        assert_eq!(t.steps(), 1);
        t.send(1);
        t.wait_step();
        assert_eq!(t.steps(), 2, "the second step waits at the cap");
        assert_eq!(t.outbox_bytes(), 10);
        drop(t);

        // A panic ends the thread: the wait returns and the panic is read.
        let mut t = BakeThread::start(
            (),
            10,
            |_a: u32, b: u32| b,
            |_, m: u32, _, _| {
                assert!(m < 3, "message {m} breaks the step");
                Vec::<Entry>::new()
            },
        )
        .expect("spawned");
        t.send(3);
        t.wait_step();
        assert_eq!(t.failed().as_deref(), Some("message 3 breaks the step"));
    }

    /// Dropping the handle joins the thread, waiting or held at the cap; a panic on the thread
    /// is read at the next call, with its message.
    #[test]
    fn a_drop_joins_and_a_panic_is_read() {
        let alive = Arc::new(AtomicBool::new(true));
        let a = alive.clone();
        struct Flag(Arc<AtomicBool>);
        impl Drop for Flag {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        let t = BakeThread::start(
            Flag(a),
            10,
            |_a: u32, b: u32| b,
            |_, _, _, _| vec![Entry(10)],
        )
        .expect("spawned");
        t.send(0);
        assert!(within(|| t.steps() == 1));
        t.send(0);
        drop(t);
        assert!(
            !alive.load(Ordering::Acquire),
            "joined: the bake half dropped with it"
        );

        let mut t = BakeThread::start(
            (),
            10,
            |_a: u32, b: u32| b,
            |_, m: u32, _, _| {
                assert!(m < 3, "message {m} breaks the step");
                Vec::<Entry>::new()
            },
        )
        .expect("spawned");
        assert_eq!(t.failed(), None);
        t.send(3);
        let at = stopwatch();
        let mut read = None;
        while read.is_none() && seconds_since(at) < 1.0 {
            read = t.failed();
            std::thread::yield_now();
        }
        assert_eq!(read.as_deref(), Some("message 3 breaks the step"));
        assert_eq!(t.failed(), None, "read once; the thread is joined");
        assert!(t.stop().is_err());
    }
}
