//! A channel: two byte queues, one per direction. The in-memory implementation is the only
//! one; a network transport is another implementation of the same trait (ADR-0007).
//!
//! A message is an `Arc<[u8]>`, so one encoded message can go to every client without a copy
//! per client; a transport that writes to a socket reads the slice.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};

/// One end of a two-way byte channel. Messages arrive whole and in send order.
pub trait Channel {
    /// Queues `bytes` for the other end. Bytes sent after the other end is dropped are lost.
    fn send(&self, bytes: Arc<[u8]>);
    /// The next message from the other end, or `None` at once when there is none.
    fn try_recv(&self) -> Option<Arc<[u8]>>;
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
    tx: Sender<Arc<[u8]>>,
    rx: Mutex<Receiver<Arc<[u8]>>>,
}

impl Endpoint {
    fn new(tx: Sender<Arc<[u8]>>, rx: Receiver<Arc<[u8]>>) -> Self {
        Self {
            tx,
            rx: Mutex::new(rx),
        }
    }
}

impl Channel for Endpoint {
    fn send(&self, bytes: Arc<[u8]>) {
        // A send fails only when the other end is gone; nobody is left to read the bytes.
        let _ = self.tx.send(bytes);
    }

    fn try_recv(&self) -> Option<Arc<[u8]>> {
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
        let m = |bytes: &[u8]| Arc::<[u8]>::from(bytes);
        assert_eq!(a.try_recv(), None);
        assert_eq!(b.try_recv(), None);
        a.send(m(&[1]));
        a.send(m(&[2, 2]));
        b.send(m(&[]));
        b.send(m(&[9]));
        assert_eq!(b.try_recv(), Some(m(&[1])));
        assert_eq!(b.try_recv(), Some(m(&[2, 2])));
        assert_eq!(b.try_recv(), None);
        assert_eq!(a.try_recv(), Some(m(&[])));
        assert_eq!(a.try_recv(), Some(m(&[9])));
        assert_eq!(a.try_recv(), None);
    }

    #[test]
    fn a_send_to_a_dropped_end_is_lost_without_panicking() {
        let (a, b) = MemChannel::pair();
        drop(b);
        a.send(Arc::from([1u8]));
        assert_eq!(a.try_recv(), None);
    }

    #[test]
    fn the_bytes_received_are_the_bytes_sent_not_a_copy() {
        let (a, b) = MemChannel::pair();
        let sent: Arc<[u8]> = Arc::from([1u8, 2, 3]);
        a.send(sent.clone());
        let got = b.try_recv().expect("sent");
        assert!(Arc::ptr_eq(&sent, &got));
    }
}
