//! The bake thread's frame (ADR-0083): one named thread that owns the bake half, a one-slot
//! inbox that coalesces the render thread's messages, and an outbox bounded in bytes.
//!
//! The render thread only swaps and tries: [`BakeThread::send`] replaces or merges the waiting
//! message, [`BakeThread::take`] drains what the bake half made. Neither waits on a step: each
//! lock is held for a swap, never across a step. The bake thread waits when no message waits
//! or when the outbox is at its cap. A panic on it ends the thread; the render thread reads it
//! as an error at its next call ([`BakeThread::failed`]). Dropping the handle stops and joins.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;

/// What the outbox weighs an entry by.
pub(crate) trait Weighed {
    fn bytes(&self) -> usize;
}

struct Outbox<O> {
    entries: Vec<O>,
    bytes: usize,
}

struct Shared<M, O> {
    inbox: Mutex<Option<M>>,
    outbox: Mutex<Outbox<O>>,
    /// The bake thread's wait: a message arrived, room was made, or the stop.
    wake: Condvar,
    stop: AtomicBool,
    cap: usize,
    /// Steps the bake thread has run.
    steps: AtomicU64,
}

impl<M, O> Shared<M, O> {
    /// Sets the stop and wakes the thread: under each lock in turn, so it is either waiting
    /// (and woken) or past its check of the stop.
    fn halt(&self) {
        self.stop.store(true, Ordering::Release);
        {
            let _slot = Self::lock(&self.inbox);
            self.wake.notify_all();
        }
        let _ob = Self::lock(&self.outbox);
        self.wake.notify_all();
    }

    fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        // A panic on the bake thread never holds a lock: it runs only inside the step.
        m.lock().unwrap_or_else(PoisonError::into_inner)
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
    /// go to the outbox, which holds at most `cap` bytes before the thread waits (an entry
    /// larger than the cap goes alone into an empty outbox). `coalesce` merges a waiting
    /// message with a newer one.
    pub(crate) fn start(
        mut side: S,
        cap: usize,
        coalesce: fn(M, M) -> M,
        mut step: impl FnMut(&mut S, M) -> Vec<O> + Send + 'static,
    ) -> std::io::Result<Self> {
        let shared = Arc::new(Shared {
            inbox: Mutex::new(None),
            outbox: Mutex::new(Outbox {
                entries: Vec::new(),
                bytes: 0,
            }),
            wake: Condvar::new(),
            stop: AtomicBool::new(false),
            cap,
            steps: AtomicU64::new(0),
        });
        let sh = shared.clone();
        let handle = std::thread::Builder::new()
            .name("svet bake".into())
            .spawn(move || {
                loop {
                    let msg = {
                        let mut slot = Shared::<M, O>::lock(&sh.inbox);
                        loop {
                            if sh.stop.load(Ordering::Acquire) {
                                return side;
                            }
                            if let Some(m) = slot.take() {
                                break m;
                            }
                            slot = sh.wake.wait(slot).unwrap_or_else(PoisonError::into_inner);
                        }
                    };
                    let out = step(&mut side, msg);
                    sh.steps.fetch_add(1, Ordering::Release);
                    let bytes: usize = out.iter().map(Weighed::bytes).sum();
                    let mut ob = Shared::<M, O>::lock(&sh.outbox);
                    while ob.bytes > 0 && ob.bytes + bytes > sh.cap {
                        // At the stop the step's entries still go in: the books already hold
                        // them, so the joiner applies them ([`BakeThread::stop_all`]).
                        if sh.stop.load(Ordering::Acquire) {
                            break;
                        }
                        ob = sh.wake.wait(ob).unwrap_or_else(PoisonError::into_inner);
                    }
                    ob.bytes += bytes;
                    ob.entries.extend(out);
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
        self.shared.wake.notify_all();
    }

    /// What the bake half made since the last call, in order.
    pub(crate) fn take(&self) -> Vec<O> {
        let mut ob = Shared::<M, O>::lock(&self.shared.outbox);
        ob.bytes = 0;
        let out = std::mem::take(&mut ob.entries);
        drop(ob);
        self.shared.wake.notify_all();
        out
    }

    /// The bytes waiting in the outbox.
    #[cfg(test)]
    pub(crate) fn outbox_bytes(&self) -> usize {
        Shared::<M, O>::lock(&self.shared.outbox).bytes
    }

    /// The steps the bake thread has run.
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
        if !self.handle.as_ref().is_some_and(JoinHandle::is_finished) {
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
            move |seen, m| {
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
        let t = BakeThread::start((), 1000, |_a: u32, b: u32| b, |_, _| vec![Entry(300)])
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
        let t = BakeThread::start(Flag(a), 10, |_a: u32, b: u32| b, |_, _| vec![Entry(10)])
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
            |_, m: u32| {
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
