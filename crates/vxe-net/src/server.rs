//! The server: the authoritative world, its op and input logs, and the tick (ADR-0007).
//!
//! One [`Server::tick`] runs, in order: every directly submitted op in `seq` order, then,
//! when there was at least one, a boundary entry in the op log that takes the next seq; then
//! the inputs drained from every attached endpoint (endpoints in attach order, each in arrival
//! order), appended to the input log; then the logic's tick, whose host applies, logs and
//! announces each op it requests; then the logic's broadcast to every client. Seqs come from
//! one counter shared by submission and the logic's host, so the op log's order is the order
//! the ops were applied in.
//!
//! Every op kind applies the same way: an edit, a stamp (its prefab resolved in the server's
//! [`PrefabRegistry`], ADR-0014), a drop or a removal. After each applied op the server's
//! [`CoarseGrid`] is updated over the result's aabb, on the server thread in op order, so it
//! always describes the authoritative world.
//!
//! A logic that fails a call ([`Logic::take_error`], a hosted logic's trap, fuel or memory
//! cap) leaves its tick without a broadcast and the error in [`TickReport::logic_error`]; the
//! tick's inputs are logged all the same, and ops the logic applied before it failed stay
//! applied and logged. The next tick calls the logic again; a hosted logic that failed
//! restarts from its seed first ([`Logic::take_restart`], reported in
//! [`TickReport::restart`]) or, its restarts spent, answers with [`LogicError::Stopped`]
//! without running.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use vxe_world::{
    Anchors, BRICK_EDGE, CellChange, CellCoord, CoarseGrid, EditError, Fixed3, Hit, InputLog,
    LogEntry, MaterialId, Op, OpLog, OpResult, Prefab, PrefabCollision, PrefabRegistry, Region,
    SupportParams, VoxelCoord, World, find_unsupported_with_stats,
};

use crate::channel::{Channel, Endpoint};
use crate::logic::{ClientInput, Host, Logic, LogicError};
use crate::message::Message;

/// The support parameters a server starts with: a cell is load-bearing at 64 solid voxels
/// (an eighth of its brick), a flood gives up past 4 096 cells, and the cells on the world
/// extent's faces are anchors.
pub const DEFAULT_SUPPORT: SupportParams = SupportParams {
    solid_min: 64,
    max_region: 4096,
    boundary_anchors: true,
};

/// What one tick did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TickReport {
    /// The tick that ran.
    pub tick: u64,
    /// Ops applied and logged, submitted and the logic's together.
    pub applied: u32,
    /// Ops refused, submitted and the logic's together.
    pub refused: u32,
    /// Inputs consumed.
    pub inputs: u32,
    /// Why the logic's tick failed, when it did; the tick then has no broadcast.
    pub logic_error: Option<LogicError>,
    /// The restart the logic made before this tick's call ([`LogicError::Restarted`]), when
    /// it made one; independent of `logic_error`.
    pub restart: Option<LogicError>,
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
    init_error: Option<LogicError>,
    prefabs: PrefabRegistry,
    coarse: CoarseGrid,
    support: Support,
}

/// The support query's configuration: its parameters and the explicit anchors, ascending.
#[derive(Clone, Debug)]
struct Support {
    params: SupportParams,
    anchors: Vec<CellCoord>,
}

impl Default for Support {
    fn default() -> Self {
        Self {
            params: DEFAULT_SUPPORT,
            anchors: Vec::new(),
        }
    }
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
    prefabs: &'a PrefabRegistry,
    coarse: &'a mut CoarseGrid,
    support: &'a Support,
    ops: &'a mut OpLog,
    clients: &'a [Endpoint],
    next_seq: &'a AtomicU32,
    tick: u64,
    applied: u32,
    refused: u32,
}

impl TickHost<'_> {
    /// Applies `op` under `seq`, keeps the coarse grid current, logs the op when it applied,
    /// and announces the outcome.
    fn apply_seq(&mut self, seq: u32, op: Op) -> Result<OpResult, EditError> {
        let result = op.apply(self.world, self.prefabs);
        let msg = match &result {
            Ok(r) => {
                self.coarse.update_aabb(self.world, r.aabb());
                self.applied += 1;
                Message::Applied {
                    tick: self.tick,
                    seq,
                    op: op.clone(),
                    result: r.clone(),
                }
            }
            Err(e) => {
                self.refused += 1;
                Message::Refused {
                    tick: self.tick,
                    seq,
                    op: op.clone(),
                    error: *e,
                }
            }
        };
        if result.is_ok() {
            self.ops.record(self.tick, seq, op);
        }
        broadcast(self.clients, &msg);
        result
    }
}

impl Host for TickHost<'_> {
    fn apply(&mut self, op: Op) -> Result<OpResult, EditError> {
        let seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
        self.apply_seq(seq, op)
    }

    fn material_at(&self, v: VoxelCoord) -> Option<MaterialId> {
        self.world.material_at(v)
    }

    /// A ray the world refuses as out of range reads as no hit.
    fn raycast(&self, origin: Fixed3, dir: Fixed3, max: i64) -> Option<Hit> {
        self.world.raycast(origin, dir.0, max).ok().flatten()
    }

    fn raycast_with_steps(&self, origin: Fixed3, dir: Fixed3, max: i64) -> (Option<Hit>, u64) {
        self.world
            .raycast_with_stats(origin, dir.0, max)
            .unwrap_or((None, 0))
    }

    /// A box the world refuses as out of range reads as an empty count.
    fn count_in_box(&self, lo: VoxelCoord, hi: VoxelCoord) -> Vec<(MaterialId, u64)> {
        self.world.count_in_box(lo, hi).unwrap_or_default()
    }

    fn unsupported(&self, changed: &[CellCoord]) -> Vec<Region> {
        self.unsupported_with_steps(changed).0
    }

    /// A `changed` list longer than one in-range edit can change (see
    /// [`max_changed_cells`]) reads as no regions, and the query does not run.
    fn unsupported_with_steps(&self, changed: &[CellCoord]) -> (Vec<Region>, u64) {
        if changed.len() as u64 > max_changed_cells(self.world) {
            return (Vec::new(), 0);
        }
        // The query reads only each change's cell; the grid holds the cells' current state.
        let changes: Vec<CellChange> = changed
            .iter()
            .map(|&c| {
                let (chunk, slot) = c.split();
                let now = self.coarse.cell(chunk, slot).unwrap_or_default();
                CellChange {
                    chunk,
                    slot: slot as u16,
                    before: now,
                    after: now,
                }
            })
            .collect();
        let anchors = Anchors {
            explicit: &self.support.anchors,
        };
        find_unsupported_with_stats(
            self.world,
            self.coarse,
            &self.support.params,
            &anchors,
            &changes,
        )
    }
}

/// The most cells one edit within `world`'s limits can change: its voxel bounds span at most
/// `edit_axis_max` voxels per axis, so at most `edit_axis_max / BRICK_EDGE + 1` cells per axis
/// (129³ = 2 146 689 at the defaults).
pub fn max_changed_cells(world: &World) -> u64 {
    let per_axis = u64::from(world.limits().edit_axis_max / BRICK_EDGE.unsigned_abs()) + 1;
    per_axis.saturating_mul(per_axis).saturating_mul(per_axis)
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
    /// A server on `world` at tick 0 with no prefabs registered. Calls `logic.init` with a
    /// host and `seed`; ops the logic applies there are logged at tick 0.
    pub fn new(world: Arc<World>, logic: Box<dyn Logic>, seed: u64) -> Self {
        Self::with_prefabs(world, PrefabRegistry::new(), logic, seed)
    }

    /// [`Server::new`] with `prefabs` registered before `logic.init` runs, so stamps from
    /// `init` resolve. Builds the coarse grid of `world`'s resident chunks first.
    pub fn with_prefabs(
        world: Arc<World>,
        prefabs: PrefabRegistry,
        mut logic: Box<dyn Logic>,
        seed: u64,
    ) -> Self {
        let mut ops = OpLog::new();
        let next_seq = AtomicU32::new(0);
        let mut coarse = CoarseGrid::build(&world);
        let support = Support::default();
        {
            let mut host = TickHost {
                world: &world,
                prefabs: &prefabs,
                coarse: &mut coarse,
                support: &support,
                ops: &mut ops,
                clients: &[],
                next_seq: &next_seq,
                tick: 0,
                applied: 0,
                refused: 0,
            };
            logic.init(&mut host, seed);
        }
        let init_error = logic.take_error();
        Self {
            world,
            logic: Mutex::new(logic),
            ops,
            inputs: InputLog::new(),
            pending: Mutex::new(Vec::new()),
            next_seq,
            tick: 0,
            clients: Vec::new(),
            init_error,
            prefabs,
            coarse,
            support,
        }
    }

    /// Sets what the support query ([`Host::unsupported`]) holds as anchored: `params`, and
    /// `anchors` on top of the anchors every query knows. Until called, the query runs with
    /// [`DEFAULT_SUPPORT`] and no explicit anchors.
    pub fn set_support(&mut self, params: SupportParams, anchors: &[CellCoord]) {
        let mut anchors = anchors.to_vec();
        anchors.sort_unstable();
        anchors.dedup();
        self.support = Support { params, anchors };
    }

    /// Registers `prefab` under its content id for stamp ops (ADR-0014) and returns the id;
    /// a different prefab under an id already registered is refused.
    pub fn register_prefab(&mut self, prefab: Arc<Prefab>) -> Result<u64, PrefabCollision> {
        self.prefabs.register(prefab)
    }

    /// The prefabs stamp ops resolve against.
    pub fn prefabs(&self) -> &PrefabRegistry {
        &self.prefabs
    }

    /// The coarse grid of the authoritative world, current after every applied op.
    pub fn coarse(&self) -> &CoarseGrid {
        &self.coarse
    }

    /// Why the logic's `init` failed, when it did.
    pub fn init_error(&self) -> Option<&LogicError> {
        self.init_error.as_ref()
    }

    /// Queues `op` for the next tick and returns its seq. Callers on several threads get
    /// seqs in the order they take the queue's lock, and the ops apply in that order.
    pub fn submit(&self, op: impl Into<Op>) -> u32 {
        let op = op.into();
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        let seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
        pending.push((seq, op));
        seq
    }

    /// Attaches a client's endpoint and returns its index, from 0 in attach order.
    pub fn attach(&mut self, endpoint: Endpoint) -> u32 {
        self.clients.push(endpoint);
        u32::try_from(self.clients.len() - 1).expect("client count fits u32")
    }

    /// Runs one tick (see the module documentation) and advances the tick counter.
    pub fn tick(&mut self) -> TickReport {
        let mut inputs = Vec::new();
        for (i, c) in self.clients.iter().enumerate() {
            let client = u32::try_from(i).expect("client count fits u32");
            while let Some(bytes) = c.try_recv() {
                // A client sends only inputs; anything else is not addressed to the server.
                if let Ok(Message::Input { bytes }) = Message::decode(&bytes) {
                    inputs.push(ClientInput { client, bytes });
                }
            }
        }
        let pending = std::mem::take(
            self.pending
                .get_mut()
                .unwrap_or_else(PoisonError::into_inner),
        );
        let boundary = (!pending.is_empty()).then(|| self.next_seq.fetch_add(1, Ordering::Relaxed));
        self.step(inputs, pending, boundary)
    }

    /// One tick on `inputs`: the submitted ops `pending` under their seqs, the `boundary`
    /// entry (not announced, not counted in the report), the inputs into the input log, the
    /// logic, the broadcast. Draining the channels and the submission queue touches no world
    /// state, so [`Server::tick`] drains before calling this and [`Server::resimulate`]
    /// passes logged inputs and logged submitted ops instead. The caller has already moved
    /// `next_seq` past `boundary`.
    fn step(
        &mut self,
        inputs: Vec<ClientInput>,
        pending: Vec<(u32, Op)>,
        boundary: Option<u32>,
    ) -> TickReport {
        let tick = self.tick;
        let logic = self.logic.get_mut().unwrap_or_else(PoisonError::into_inner);
        let mut host = TickHost {
            world: &self.world,
            prefabs: &self.prefabs,
            coarse: &mut self.coarse,
            support: &self.support,
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
        if let Some(seq) = boundary {
            host.ops.record_boundary(tick, seq);
        }
        for input in &inputs {
            self.inputs.push(tick, input.client, input.bytes.clone());
        }
        let out = logic.tick(&mut host, tick, &inputs);
        let logic_error = logic.take_error();
        let restart = logic.take_restart();
        let (applied, refused) = (host.applied, host.refused);
        if logic_error.is_none() && !out.broadcast.is_empty() {
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
            logic_error,
            restart,
        }
    }

    /// Re-runs a session from its seed, its op log and its input log: a server on `world`
    /// with `prefabs` registered and `logic` initialised from `seed` runs ticks 0 through the later of the last input's
    /// and the last op log entry's tick, each on the inputs logged for it, and the op log it
    /// builds is returned. At a tick with a boundary it first applies that tick's logged op
    /// entries whose seqs lie at or above its own next seq and below the boundary's (the
    /// directly submitted ops that applied; at tick 0 the lower bound skips the ops `init`
    /// re-produced), records them and the boundary under their logged seqs, and continues the
    /// seq count after the boundary. Equal to the live op log when the logic is a pure
    /// function of its seed and inputs.
    pub fn resimulate(
        world: Arc<World>,
        logic: Box<dyn Logic>,
        seed: u64,
        prefabs: &PrefabRegistry,
        ops: &OpLog,
        inputs: &InputLog,
    ) -> OpLog {
        let mut server = Self::with_prefabs(world, prefabs.clone(), logic, seed);
        let logged = ops.entries();
        let entries = inputs.entries();
        let (mut i, mut j) = (0, 0);
        let last = entries
            .last()
            .map(|e| e.tick)
            .max(logged.last().map(LogEntry::tick));
        while last.is_some_and(|last| server.tick <= last) {
            let tick = server.tick;
            let mut batch = Vec::new();
            while let Some(e) = entries.get(i).filter(|e| e.tick == tick) {
                batch.push(ClientInput {
                    client: e.client,
                    bytes: e.bytes.clone(),
                });
                i += 1;
            }
            // Entries ascend in seq and so in tick: this tick's are the run from `j`.
            let start = j;
            while logged.get(j).is_some_and(|e| e.tick() == tick) {
                j += 1;
            }
            let this_tick = &logged[start..j];
            let boundary = this_tick.iter().find_map(|e| match e {
                LogEntry::Boundary { seq, .. } => Some(*seq),
                LogEntry::Op { .. } => None,
            });
            let mut pending = Vec::new();
            if let Some(b) = boundary {
                let next = *server.next_seq.get_mut();
                pending.extend(this_tick.iter().filter_map(|e| match e {
                    LogEntry::Op { seq, op, .. } if (next..b).contains(seq) => {
                        Some((*seq, op.clone()))
                    }
                    _ => None,
                }));
                *server.next_seq.get_mut() = b + 1;
            }
            server.step(batch, pending, boundary);
        }
        server.ops
    }

    /// The authoritative world.
    pub fn world(&self) -> &Arc<World> {
        &self.world
    }

    /// Every applied op and every boundary, in seq order.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::MemChannel;
    use crate::client::{Client, Event};
    use crate::logic::TickOutput;
    use crate::test_logic::{EchoLogic, seeded_carves, solid3_world};
    use vxe_world::{EditOp, Field, Mode, centre_of_voxel};

    /// A sphere whose radius is past the world's limits.
    fn out_of_range() -> Op {
        Op::Edit(EditOp::sphere(Fixed3([0; 3]), (1 << 30) + 1, Mode::Carve))
    }

    /// An out-of-range op between two in-range carves is refused on the server and announced
    /// as refused; an owned client stays in step and the op is not logged. Replaying a log
    /// that holds it anyway refuses it again on the world, and re-simulating that log on a
    /// fresh server gives the live log and the live world.
    #[test]
    fn an_out_of_range_op_is_refused_on_the_server_and_an_owned_client() {
        let carves = seeded_carves(0x0019, 2);
        let mut server = Server::new(Arc::new(solid3_world()), Box::new(EchoLogic), 0);
        let (near, far) = MemChannel::pair();
        server.attach(far);
        let mut client = Client::owned(solid3_world(), near);
        server.submit(carves[0]);
        server.submit(out_of_range());
        server.submit(carves[1]);
        let report = server.tick();
        assert_eq!((report.applied, report.refused), (2, 1), "{report:?}");
        let events = client.poll();
        let refused: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                Event::Refused { seq, error, .. } => Some((*seq, *error)),
                _ => None,
            })
            .collect();
        assert_eq!(
            refused,
            [(
                1,
                EditError::OutOfRange {
                    what: Field::Radius
                }
            )]
        );
        assert!(!client.desynced(), "{events:?}");
        assert!(!events.iter().any(|e| matches!(e, Event::Desync { .. })));
        assert_eq!(client.world().voxel_hash(), server.world().voxel_hash());
        let logged: Vec<u32> = server
            .ops()
            .entries()
            .iter()
            .filter(|e| e.op().is_some())
            .map(LogEntry::seq)
            .collect();
        assert_eq!(logged, [0, 2]);

        // A log holding the out-of-range op under its seq, as a submitted op of tick 0.
        let mut log = OpLog::new();
        log.record(0, 0, carves[0]);
        log.record(0, 1, out_of_range());
        log.record(0, 2, carves[1]);
        log.record_boundary(0, 3);
        let replayed = solid3_world().replay(&log, &PrefabRegistry::new());
        assert!(replayed[0].is_ok() && replayed[2].is_ok(), "{replayed:?}");
        assert_eq!(
            replayed[1],
            Err(EditError::OutOfRange {
                what: Field::Radius
            })
        );
        let world = Arc::new(solid3_world());
        let rebuilt = Server::resimulate(
            world.clone(),
            Box::new(EchoLogic),
            0,
            &PrefabRegistry::new(),
            &log,
            &InputLog::new(),
        );
        assert_eq!(rebuilt.entries(), server.ops().entries());
        assert_eq!(world.voxel_hash(), server.world().voxel_hash());
    }

    /// Asks the host, at tick 0, for a ray and a box the world refuses, and records the
    /// answers.
    struct AskOutOfRange(Arc<Mutex<Option<Answers>>>);

    /// The ray's and the count's answers.
    type Answers = (Option<Hit>, Vec<(MaterialId, u64)>);

    impl Logic for AskOutOfRange {
        fn init(&mut self, _: &mut dyn Host, _: u64) {}
        fn tick(&mut self, host: &mut dyn Host, _: u64, _: &[ClientInput]) -> TickOutput {
            let o = centre_of_voxel([0, 0, 0]);
            let ray = host.raycast(o, Fixed3([256, 0, 0]), (1 << 22) + 1);
            let count = host.count_in_box(VoxelCoord([0; 3]), VoxelCoord([1024, 0, 0]));
            *self.0.lock().unwrap() = Some((ray, count));
            TickOutput::default()
        }
    }

    /// Through the server's `Host` a refused ray reads as no hit and a refused box as an
    /// empty count, though the same in-range ray meets rock at once.
    #[test]
    fn refused_queries_read_as_no_hit_and_an_empty_count() {
        let world = Arc::new(solid3_world());
        let o = centre_of_voxel([0, 0, 0]);
        assert!(world.raycast(o, [256, 0, 0], 1 << 22).unwrap().is_some());
        assert!(world.raycast(o, [256, 0, 0], (1 << 22) + 1).is_err());
        let seen = Arc::new(Mutex::new(None));
        let mut server = Server::new(world, Box::new(AskOutOfRange(seen.clone())), 0);
        server.tick();
        assert_eq!(*seen.lock().unwrap(), Some((None, Vec::new())));
    }

    /// A drop and a removal of 4 097 cells are refused on the server, announced as refused,
    /// and an owned client stays in step.
    #[test]
    fn region_ops_over_the_cell_cap_are_refused_on_the_server_and_an_owned_client() {
        let cells: Vec<CellCoord> = (0..4097)
            .map(|i| CellCoord([i % 16 - 8, (i / 16) % 16 - 8, i / 256 - 8]))
            .collect();
        let mut server = Server::new(Arc::new(solid3_world()), Box::new(EchoLogic), 0);
        let (near, far) = MemChannel::pair();
        server.attach(far);
        let mut client = Client::owned(solid3_world(), near);
        server.submit(Op::Drop {
            cells: cells.clone(),
            max_drop: 8,
        });
        server.submit(Op::Remove {
            cells,
            rubble: 3,
            fraction: (1, 4),
            max_depth: 16,
        });
        let report = server.tick();
        assert_eq!((report.applied, report.refused), (0, 2), "{report:?}");
        let events = client.poll();
        let refused: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                Event::Refused { error, .. } => Some(*error),
                _ => None,
            })
            .collect();
        let cap = EditError::OutOfRange {
            what: Field::RegionCells,
        };
        assert_eq!(refused, [cap, cap]);
        assert!(!client.desynced(), "{events:?}");
        assert_eq!(client.world().voxel_hash(), server.world().voxel_hash());
        assert_eq!(server.world().voxel_hash(), solid3_world().voxel_hash());
    }

    /// Asks the host's support query with `n` copies of one floor cell and records the
    /// steps it reports.
    struct AskSupport(u64, Arc<Mutex<Option<SupportAnswer>>>);

    /// The query's regions and steps.
    type SupportAnswer = (Vec<Region>, u64);

    impl Logic for AskSupport {
        fn init(&mut self, _: &mut dyn Host, _: u64) {}
        fn tick(&mut self, host: &mut dyn Host, _: u64, _: &[ClientInput]) -> TickOutput {
            let changed = vec![CellCoord([2, 2, 0]); self.0 as usize];
            *self.1.lock().unwrap() = Some(host.unsupported_with_steps(&changed));
            TickOutput::default()
        }
    }

    /// The server's support query refuses a `changed` list longer than one in-range edit can
    /// change, `(1 024 / 8 + 1)^3 = 2 146 689` cells at the default limits: one cell more
    /// reads as no regions and no steps; at the cap the query runs.
    #[test]
    fn a_changed_list_past_the_cap_reads_as_no_regions() {
        let world = Arc::new(crate::test_logic::slab_world());
        let cap = max_changed_cells(&world);
        assert_eq!(cap, 2_146_689);
        for (n, runs) in [(cap + 1, false), (cap, true)] {
            let seen = Arc::new(Mutex::new(None));
            let mut server = Server::new(world.clone(), Box::new(AskSupport(n, seen.clone())), 0);
            server.tick();
            let (regions, steps) = seen.lock().unwrap().take().unwrap();
            assert!(regions.is_empty());
            // The cell and its six neighbours are 7 candidates; it and its four floor
            // neighbours each start a flood that is anchored at once on the extent's bottom
            // face (1 cell each); the cells below and above are outside the extent and air.
            assert_eq!(steps, if runs { 7 + 5 } else { 0 }, "{n} cells");
        }
    }
}
