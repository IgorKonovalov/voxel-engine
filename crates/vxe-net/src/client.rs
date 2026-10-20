//! Clients: the receiving end of the server's announcements (ADR-0007).
//!
//! A shared client holds the server's own `Arc<World>` and reads results from the messages:
//! solo play, with no second world. An owned client holds its own world, applies every
//! applied op to it, and compares its result with the server's. On the first difference it
//! reports [`Event::Desync`] and stops applying: its world differs from the server's, and
//! every later result would differ for the same reason. An owned client resolves stamps in
//! its own [`PrefabRegistry`], which the application fills with the same prefabs as the
//! server's; a stamp of an id it lacks is refused on its side, which is a desync (ADR-0014).

use std::sync::Arc;

use vxe_world::{EditError, Op, OpResult, Prefab, PrefabCollision, PrefabRegistry, World};

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
        result: OpResult,
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
    /// An owned client's result for op `seq` differs from the server's. `ours` is `None`
    /// when the client's world refused the op. The client applies nothing after this.
    Desync {
        seq: u32,
        ours: Option<OpResult>,
        theirs: OpResult,
    },
}

#[derive(Debug)]
pub struct Client {
    world: ClientWorld,
    endpoint: Endpoint,
    desynced: bool,
    prefabs: PrefabRegistry,
}

impl Client {
    /// A client that reads the server's world, `world`, which must be the server's own.
    pub fn shared(world: Arc<World>, endpoint: Endpoint) -> Self {
        Self {
            world: ClientWorld::Shared(world),
            endpoint,
            desynced: false,
            prefabs: PrefabRegistry::new(),
        }
    }

    /// A client that keeps its own `world` in step with the server's: it must start equal to
    /// the server's world, and every applied op is applied to it and its result compared.
    pub fn owned(world: World, endpoint: Endpoint) -> Self {
        Self {
            world: ClientWorld::Owned(Box::new(world)),
            endpoint,
            desynced: false,
            prefabs: PrefabRegistry::new(),
        }
    }

    /// Registers `prefab` for the stamps an owned client applies, and returns its id; a
    /// different prefab under an id already registered is refused. A shared client applies
    /// nothing and keeps the registry unused.
    pub fn register_prefab(&mut self, prefab: Arc<Prefab>) -> Result<u64, PrefabCollision> {
        self.prefabs.register(prefab)
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
    /// server message are dropped. An owned client applies each applied op to its world
    /// first and reports [`Event::Desync`] instead of [`Event::Applied`] when its result
    /// differs; once desynced, applied ops pass through as [`Event::Applied`] with the
    /// server's result and are not applied.
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
                } => match self.check(&op, &result) {
                    Some(ours) => {
                        self.desynced = true;
                        Event::Desync {
                            seq,
                            ours,
                            theirs: result,
                        }
                    }
                    None => Event::Applied {
                        tick,
                        seq,
                        op,
                        result,
                    },
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

    /// For an owned, in-step client: applies `op` to its world and returns `Some` of its own
    /// result when that differs from `theirs`, or `Some(None)` when its world refuses the op.
    /// `None` otherwise, and always for a shared or desynced client.
    fn check(&self, op: &Op, theirs: &OpResult) -> Option<Option<OpResult>> {
        let ClientWorld::Owned(world) = &self.world else {
            return None;
        };
        if self.desynced {
            return None;
        }
        match op.apply(world, &self.prefabs) {
            Ok(ours) => (ours != *theirs).then_some(Some(ours)),
            // The server applied what this world refuses: the worlds differ in residency, or
            // this client lacks the stamp's prefab.
            Err(_) => Some(None),
        }
    }
}
