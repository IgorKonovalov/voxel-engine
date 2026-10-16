//! The server: the authoritative world, its op and input logs, and the tick (ADR-0007).
//!
//! One [`Server::tick`] runs, in order: every directly submitted op in `seq` order; then the
//! drain of every attached endpoint's inputs (endpoints in attach order, each in arrival
//! order), appended to the input log; then the logic's tick, whose host applies, logs and
//! announces each op it requests; then the logic's broadcast to every client. Seqs come from
//! one counter shared by submission and the logic's host, so the op log's order is the order
//! the ops were applied in.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use vxe_world::{
    EditError, EditOp, EditResult, Fixed3, Hit, InputLog, MaterialId, Op, OpLog, VoxelCoord, World,
};

use crate::channel::{Channel, Endpoint};
use crate::logic::{ClientInput, Host, Logic};
use crate::message::Message;

/// What one tick did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TickReport {
    /// The tick that ran.
    pub tick: u64,
    /// Ops applied and logged, submitted and the logic's together.
    pub applied: u32,
    /// Ops refused, submitted and the logic's together.
    pub refused: u32,
    /// Inputs consumed.
    pub inputs: u32,
}

pub struct Server {
    world: Arc<World>,
    /// Behind a mutex only so the server is `Sync` for threads calling [`Server::submit`];
    /// every use goes through `&mut self` and `get_mut`, which takes no lock.
    logic: Mutex<Box<dyn Logic>>,
    ops: OpLog,
    inputs: InputLog,
    pending: Mutex<Vec<(u32, Op)>>,
    next_seq: AtomicU32,
    tick: u64,
    clients: Vec<Endpoint>,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server")
            .field("tick", &self.tick)
            .field("ops", &self.ops.len())
            .field("inputs", &self.inputs.len())
            .field("clients", &self.clients.len())
            .finish_non_exhaustive()
    }
}

/// The host a logic sees during [`Logic::init`] and [`Logic::tick`].
struct TickHost<'a> {
    world: &'a World,
    ops: &'a mut OpLog,
    clients: &'a [Endpoint],
    next_seq: &'a AtomicU32,
    tick: u64,
    applied: u32,
    refused: u32,
}

impl TickHost<'_> {
    /// Applies `op` under `seq`, logs it when it applied, and announces the outcome.
    fn apply_seq(&mut self, seq: u32, op: Op) -> Result<EditResult, EditError> {
        let result = op.apply(self.world);
        let msg = match &result {
            Ok(r) => {
                self.ops.record(self.tick, seq, op);
                self.applied += 1;
                Message::Applied {
                    tick: self.tick,
                    seq,
                    op,
                    result: r.clone(),
                }
            }
            Err(e) => {
                self.refused += 1;
                Message::Refused {
                    tick: self.tick,
                    seq,
                    op,
                    error: *e,
                }
            }
        };
        broadcast(self.clients, &msg);
        result
    }
}

impl Host for TickHost<'_> {
    fn apply(&mut self, op: EditOp) -> Result<EditResult, EditError> {
        let seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
        self.apply_seq(seq, Op::Edit(op))
    }

    fn material_at(&self, v: VoxelCoord) -> Option<MaterialId> {
        self.world.material_at(v)
    }

    fn raycast(&self, origin: Fixed3, dir: Fixed3, max: i64) -> Option<Hit> {
        self.world.raycast(origin, dir.0, max)
    }

    fn count_in_box(&self, lo: VoxelCoord, hi: VoxelCoord) -> Vec<(MaterialId, u64)> {
        self.world.count_in_box(lo, hi)
    }
}

fn broadcast(clients: &[Endpoint], msg: &Message) {
    if clients.is_empty() {
        return;
    }
    let bytes = msg.encode();
    for c in clients {
        c.send(bytes.clone());
    }
}

impl Server {
    /// A server on `world` at tick 0. Calls `logic.init` with a host and `seed`; ops the logic
    /// applies there are logged at tick 0.
    pub fn new(world: Arc<World>, mut logic: Box<dyn Logic>, seed: u64) -> Self {
        let mut ops = OpLog::new();
        let next_seq = AtomicU32::new(0);
        {
            let mut host = TickHost {
                world: &world,
                ops: &mut ops,
                clients: &[],
                next_seq: &next_seq,
                tick: 0,
                applied: 0,
                refused: 0,
            };
            logic.init(&mut host, seed);
        }
        Self {
            world,
            logic: Mutex::new(logic),
            ops,
            inputs: InputLog::new(),
            pending: Mutex::new(Vec::new()),
            next_seq,
            tick: 0,
            clients: Vec::new(),
        }
    }

    /// Queues `op` for the next tick and returns its seq. Callers on several threads get
    /// seqs in the order they take the queue's lock, and the ops apply in that order.
    pub fn submit(&self, op: EditOp) -> u32 {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        let seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
        pending.push((seq, Op::Edit(op)));
        seq
    }

    /// Attaches a client's endpoint and returns its index, from 0 in attach order.
    pub fn attach(&mut self, endpoint: Endpoint) -> u32 {
        self.clients.push(endpoint);
        u32::try_from(self.clients.len() - 1).expect("client count fits u32")
    }

    /// Runs one tick (see the module documentation) and advances the tick counter.
    pub fn tick(&mut self) -> TickReport {
        let tick = self.tick;
        let pending = std::mem::take(
            self.pending
                .get_mut()
                .unwrap_or_else(PoisonError::into_inner),
        );

        let logic = self.logic.get_mut().unwrap_or_else(PoisonError::into_inner);
        let mut host = TickHost {
            world: &self.world,
            ops: &mut self.ops,
            clients: &self.clients,
            next_seq: &self.next_seq,
            tick,
            applied: 0,
            refused: 0,
        };
        for (seq, op) in pending {
            let _ = host.apply_seq(seq, op);
        }
        let mut inputs = Vec::new();
        for (i, c) in self.clients.iter().enumerate() {
            let client = u32::try_from(i).expect("client count fits u32");
            while let Some(bytes) = c.try_recv() {
                // A client sends only inputs; anything else is not addressed to the server.
                if let Ok(Message::Input { bytes }) = Message::decode(&bytes) {
                    self.inputs.push(tick, client, bytes.clone());
                    inputs.push(ClientInput { client, bytes });
                }
            }
        }
        let out = logic.tick(&mut host, tick, &inputs);
        let (applied, refused) = (host.applied, host.refused);
        if !out.broadcast.is_empty() {
            broadcast(
                &self.clients,
                &Message::Event {
                    tick,
                    bytes: out.broadcast,
                },
            );
        }
        self.tick += 1;
        TickReport {
            tick,
            applied,
            refused,
            inputs: u32::try_from(inputs.len()).expect("input count fits u32"),
        }
    }

    /// The authoritative world.
    pub fn world(&self) -> &Arc<World> {
        &self.world
    }

    /// Every applied op, in seq order.
    pub fn ops(&self) -> &OpLog {
        &self.ops
    }

    /// Every consumed input, in consumption order.
    pub fn inputs(&self) -> &InputLog {
        &self.inputs
    }

    /// The tick the next [`Server::tick`] will run.
    pub fn current_tick(&self) -> u64 {
        self.tick
    }
}
