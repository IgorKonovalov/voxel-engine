//! A box waiting for the bake thread (ADR-0083, Plan 0099 Phase 3 as amended): the render side
//! counts each message it sends that holds an edit's box, and each message carries the count
//! as of its send; the bake side knows the count of the newest message it took. A step whose
//! opening reads the sent count past the taken one admits no further candidate, and the next
//! step takes the box first. Lockstep has no signal: its box is in the step's own message.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// The render side's half: counts the messages sent with a box.
#[derive(Clone, Default)]
pub(crate) struct BoxSender {
    sent: Arc<AtomicU64>,
}

impl BoxSender {
    /// The count to stamp on a message about to be sent: one more when it holds a box. The
    /// count rises before the message is in the inbox, so a step that reads it may stop its
    /// opening a moment early, never late.
    pub(crate) fn stamp(&self, holds_box: bool) -> u64 {
        if holds_box {
            self.sent.fetch_add(1, Ordering::AcqRel) + 1
        } else {
            self.sent.load(Ordering::Acquire)
        }
    }

    /// The bake side's half, reading this sender's count.
    pub(crate) fn signal(&self) -> BoxSignal {
        BoxSignal {
            sent: self.sent.clone(),
            taken: self.sent.load(Ordering::Acquire),
        }
    }
}

/// The bake side's half: whether a message with a box was sent and not yet taken.
pub(crate) struct BoxSignal {
    sent: Arc<AtomicU64>,
    taken: u64,
}

impl BoxSignal {
    /// A message stamped `count` was taken.
    pub(crate) fn took(&mut self, count: u64) {
        self.taken = self.taken.max(count);
    }

    /// Whether a box waits in a message not yet taken.
    pub(crate) fn waits(&self) -> bool {
        self.sent.load(Ordering::Acquire) > self.taken
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_box_waits_from_its_stamp_until_its_message_is_taken() {
        let tx = BoxSender::default();
        let mut rx = tx.signal();
        let plain = tx.stamp(false);
        assert!(!rx.waits(), "no box sent");
        let boxed = tx.stamp(true);
        assert!(rx.waits(), "a box sent, not taken");
        rx.took(plain);
        assert!(rx.waits(), "an older message taken");
        rx.took(boxed);
        assert!(!rx.waits(), "its message taken");
        let late = tx.stamp(false);
        rx.took(late);
        assert!(!rx.waits());
    }
}
