//! Clients: the receiving end of the server's announcements (ADR-0007).
//!
//! A shared client holds the server's own `Arc<World>` and reads results from the messages:
//! solo play, with no second world. An owned client holds its own world, applies every
//! applied op to it, and compares its result with the server's.

use std::sync::Arc;

use vxe_world::{EditError, EditResult, Op, World};

use crate::channel::{Channel, Endpoint};
use crate::message::Message;

/// The world a client reads.
#[derive(Debug)]
pub enum ClientWorld {
    /// The server's world itself.
    Shared(Arc<World>),
    /// A copy the client keeps in step by applying every announced op.
    Owned(Box<World>),
}

/// What a client learns from one message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The server applied this op; `result` is the server's.
    Applied {
        tick: u64,
        seq: u32,
        op: Op,
        result: EditResult,
    },
    /// The server refused this op; nothing changed.
    Refused {
        tick: u64,
        seq: u32,
        op: Op,
        error: EditError,
    },
    /// The logic's broadcast for a tick.
    Logic { tick: u64, bytes: Vec<u8> },
}

#[derive(Debug)]
pub struct Client {
    world: ClientWorld,
    endpoint: Endpoint,
    desynced: bool,
}

impl Client {
    /// A client that reads the server's world, `world`, which must be the server's own.
    pub fn shared(world: Arc<World>, endpoint: Endpoint) -> Self {
        Self {
            world: ClientWorld::Shared(world),
            endpoint,
            desynced: false,
        }
    }

    /// The world this client reads.
    pub fn world(&self) -> &World {
        match &self.world {
            ClientWorld::Shared(w) => w,
            ClientWorld::Owned(w) => w,
        }
    }

    /// The server's world, when this client shares it.
    pub fn shared_world(&self) -> Option<&Arc<World>> {
        match &self.world {
            ClientWorld::Shared(w) => Some(w),
            ClientWorld::Owned(_) => None,
        }
    }

    /// Sends one input to the server's logic. Panics when `bytes` exceeds `u16::MAX` bytes.
    pub fn send_input(&self, bytes: Vec<u8>) {
        self.endpoint.send(Message::Input { bytes }.encode());
    }

    /// True once an owned client has seen a result differ from the server's.
    pub fn desynced(&self) -> bool {
        self.desynced
    }

    /// Every pending message, decoded, in arrival order. Bytes that do not decode as a
    /// server message are dropped.
    pub fn poll(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        while let Some(bytes) = self.endpoint.try_recv() {
            let Ok(msg) = Message::decode(&bytes) else {
                continue;
            };
            let event = match msg {
                Message::Applied {
                    tick,
                    seq,
                    op,
                    result,
                } => Event::Applied {
                    tick,
                    seq,
                    op,
                    result,
                },
                Message::Refused {
                    tick,
                    seq,
                    op,
                    error,
                } => Event::Refused {
                    tick,
                    seq,
                    op,
                    error,
                },
                Message::Event { tick, bytes } => Event::Logic { tick, bytes },
                Message::Input { .. } => continue,
            };
            events.push(event);
        }
        events
    }
}
