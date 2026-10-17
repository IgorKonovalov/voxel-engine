//! A channel: two byte queues, one per direction. The in-memory implementation is the only
//! one; a network transport is another implementation of the same trait (ADR-0007).

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Mutex, PoisonError};

/// One end of a two-way byte channel. Messages arrive whole and in send order.
pub trait Channel {
    /// Queues `bytes` for the other end. Bytes sent after the other end is dropped are lost.
    fn send(&self, bytes: Vec<u8>);
    /// The next message from the other end, or `None` at once when there is none.
    fn try_recv(&self) -> Option<Vec<u8>>;
}

/// In-memory channels on `std::sync::mpsc` queues.
pub struct MemChannel;

impl MemChannel {
    /// Two connected endpoints: what one sends, the other receives.
    pub fn pair() -> (Endpoint, Endpoint) {
        let (a_tx, b_rx) = channel();
        let (b_tx, a_rx) = channel();
        (Endpoint::new(a_tx, a_rx), Endpoint::new(b_tx, b_rx))
    }
}

/// One end of a [`MemChannel`]. The receiver sits behind a mutex so an endpoint can be shared
/// between threads; only one thread receives at a time.
#[derive(Debug)]
pub struct Endpoint {
    tx: Sender<Vec<u8>>,
    rx: Mutex<Receiver<Vec<u8>>>,
}

impl Endpoint {
    fn new(tx: Sender<Vec<u8>>, rx: Receiver<Vec<u8>>) -> Self {
        Self {
            tx,
            rx: Mutex::new(rx),
        }
    }
}

impl Channel for Endpoint {
    fn send(&self, bytes: Vec<u8>) {
        // A send fails only when the other end is gone; nobody is left to read the bytes.
        let _ = self.tx.send(bytes);
    }

    fn try_recv(&self) -> Option<Vec<u8>> {
        self.rx
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .try_recv()
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_arrive_in_send_order_in_each_direction() {
        let (a, b) = MemChannel::pair();
        assert_eq!(a.try_recv(), None);
        assert_eq!(b.try_recv(), None);
        a.send(vec![1]);
        a.send(vec![2, 2]);
        b.send(vec![]);
        b.send(vec![9]);
        assert_eq!(b.try_recv(), Some(vec![1]));
        assert_eq!(b.try_recv(), Some(vec![2, 2]));
        assert_eq!(b.try_recv(), None);
        assert_eq!(a.try_recv(), Some(vec![]));
        assert_eq!(a.try_recv(), Some(vec![9]));
        assert_eq!(a.try_recv(), None);
    }

    #[test]
    fn a_send_to_a_dropped_end_is_lost_without_panicking() {
        let (a, b) = MemChannel::pair();
        drop(b);
        a.send(vec![1]);
        assert_eq!(a.try_recv(), None);
    }
}
