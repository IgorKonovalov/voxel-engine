//! The component observes nothing but its seed and its inputs. Each knob of the host's
//! configuration has a guest that would notice its absence: `probe` reads NaN bits, both
//! clocks, `getrandom`, `HashMap` order, the filesystem and the environment; `spin` never
//! returns; `hog` applies its inputs, then allocates 1 GiB; `timer` sleeps and polls a timer;
//! `chatty` writes 1 KiB of stderr per tick and panics at tick 100.
//! A logic that fails a call restarts from its seed on the next tick, up to the configured
//! number of times, and is then stopped for the rest of the run; with no restarts allowed it
//! is stopped at once. The ops it applied before failing stay.

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use support::guest;
use vxe_net::test_logic::*;
use vxe_net::*;
use vxe_script_wasm::{WasmConfig, WasmLogic, WorkRates};
use vxe_world::hash::mix64;
use vxe_world::{
    Anchors, CellChange, CellCoord, ChunkExtent, CoarseGrid, EditError, EditOp, Field, Fixed3, Hit,
    MaterialId, Mode, Op, OpResult, PrefabRegistry, Region, UniformSource, VoxelCoord, World,
    centre_of_voxel, find_unsupported_with_stats,
};

/// A host straight on a world, for calling a [`WasmLogic`] without a server.
struct WorldHost<'a>(&'a World);

impl Host for WorldHost<'_> {
    fn apply(&mut self, op: Op) -> Result<OpResult, EditError> {
        op.apply(self.0, &PrefabRegistry::new())
    }

    fn material_at(&self, v: VoxelCoord) -> Option<MaterialId> {
        self.0.material_at(v)
    }

    fn raycast(&self, origin: Fixed3, dir: Fixed3, max: i64) -> Option<Hit> {
        self.0.raycast(origin, dir.0, max).ok().flatten()
    }

    fn raycast_with_steps(&self, origin: Fixed3, dir: Fixed3, max: i64) -> (Option<Hit>, u64) {
        self.0
            .raycast_with_stats(origin, dir.0, max)
            .unwrap_or((None, 0))
    }

    fn count_in_box(&self, lo: VoxelCoord, hi: VoxelCoord) -> Vec<(MaterialId, u64)> {
        self.0.count_in_box(lo, hi).unwrap_or_default()
    }

    /// This host keeps no coarse grid; nothing here asks the support query.
    fn unsupported(&self, _changed: &[CellCoord]) -> Vec<Region> {
        Vec::new()
    }
}

#[test]
fn wasm_config_defaults() {
    assert_eq!(
        WasmConfig::default(),
        WasmConfig {
            fuel_per_tick: 10_000_000_000,
            memory_bytes: 64 << 20,
            frame_ns: 16_666_667,
            max_restarts: 8,
            rates: WorkRates {
                per_voxel: 10,
                per_step: 10,
                per_call: 100,
                per_byte: 1,
            },
        }
    );
    assert_eq!(WorkRates::default(), WorkRates::DEFAULT);
}

/// `config` with no restarts: the fail-stop host.
fn fail_stop(config: WasmConfig) -> WasmConfig {
    WasmConfig {
        max_restarts: 0,
        ..config
    }
}

/// A server on `solid3_world()` running the component `name` under `config` with `seed`,
/// and one attached shared client.
fn session(name: &str, config: &WasmConfig, seed: u64) -> (Server, Client) {
    let logic = WasmLogic::load(&guest(name), config).expect("the component loads");
    let world = Arc::new(solid3_world());
    let mut server = Server::new(world.clone(), Box::new(logic), seed);
    let (near, far) = MemChannel::pair();
    server.attach(far);
    (server, Client::shared(world, near))
}

/// One tick's `probe` broadcast.
#[derive(Debug, PartialEq, Eq)]
struct Probe {
    nan_bits: u32,
    monotonic_ns: u64,
    wall_ns: u64,
    random: [u8; 8],
    map_order: [u8; 8],
    opened_root: u8,
    read_env: u8,
}

impl Probe {
    fn parse(b: &[u8]) -> Self {
        assert_eq!(b.len(), 4 + 8 + 8 + 8 + 8 + 2, "{b:?}");
        Self {
            nan_bits: u32::from_le_bytes(b[0..4].try_into().unwrap()),
            monotonic_ns: u64::from_le_bytes(b[4..12].try_into().unwrap()),
            wall_ns: u64::from_le_bytes(b[12..20].try_into().unwrap()),
            random: b[20..28].try_into().unwrap(),
            map_order: b[28..36].try_into().unwrap(),
            opened_root: b[36],
            read_env: b[37],
        }
    }
}

/// `ticks` ticks of the probe; each tick's report.
fn probe_run(config: &WasmConfig, seed: u64, ticks: u64) -> Vec<Probe> {
    let (mut server, mut client) = session("probe", config, seed);
    (0..ticks)
        .map(|tick| {
            let report = server.tick();
            assert_eq!(report.logic_error, None);
            match &client.poll()[..] {
                [Event::Logic { tick: t, bytes }] if *t == tick => Probe::parse(bytes),
                e => panic!("{e:?}"),
            }
        })
        .collect()
}

/// The first eight bytes of the stream the host serves at `tick`: xorshift64 (13, 7, 17)
/// from `mix64(seed ^ tick)`, its first output little-endian.
fn first_random_bytes(seed: u64, tick: u64) -> [u8; 8] {
    let mut state = mix64(seed ^ tick);
    assert_ne!(state, 0);
    xorshift64(&mut state).to_le_bytes()
}

#[test]
fn probe_sees_canonical_nans_tick_clocks_seeded_random_and_no_filesystem_or_environment() {
    const SEED: u64 = 0x5EED_0011;
    let config = WasmConfig::default();
    let run = probe_run(&config, SEED, 4);
    for (tick, p) in run.iter().enumerate() {
        let tick = tick as u64;
        assert_eq!(p.nan_bits, 0x7fc0_0000, "tick {tick}");
        assert_eq!(p.monotonic_ns, tick * 16_666_667, "tick {tick}");
        assert_eq!(p.wall_ns, tick * 16_666_667, "tick {tick}");
        assert_eq!(p.random, first_random_bytes(SEED, tick), "tick {tick}");
        let mut keys = p.map_order;
        keys.sort_unstable();
        assert_eq!(keys, [1, 2, 3, 4, 5, 6, 7, 8], "tick {tick}");
        assert_eq!((p.opened_root, p.read_env), (0, 0), "tick {tick}");
    }
    // The random bytes differ from tick to tick: the stream restarts from the tick.
    assert_ne!(run[0].random, run[1].random);

    // A second load with the same seed observes the same, `HashMap` order included.
    assert_eq!(probe_run(&config, SEED, 4), run);

    // The clocks step by the configured frame.
    let fast = WasmConfig {
        frame_ns: 1_000,
        ..config
    };
    let run = probe_run(&fast, SEED, 3);
    assert_eq!(
        run.iter()
            .map(|p| (p.monotonic_ns, p.wall_ns))
            .collect::<Vec<_>>(),
        [(0, 0), (1_000, 1_000), (2_000, 2_000)]
    );
}

/// A smaller budget than the default keeps the spinning tests fast; the mechanism is the same.
const SPIN_FUEL: WasmConfig = WasmConfig {
    fuel_per_tick: 50_000_000,
    memory_bytes: 64 << 20,
    frame_ns: 16_666_667,
    max_restarts: 8,
    rates: WorkRates::DEFAULT,
};

#[test]
fn spin_runs_out_of_fuel() {
    let mut logic = WasmLogic::load(&guest("spin"), &SPIN_FUEL).expect("spin loads");
    let world = solid3_world();
    let mut host = WorldHost(&world);
    logic.try_init(&mut host, 0).expect("spin's init returns");
    assert_eq!(
        logic.try_tick(&mut host, 0, &[]),
        Err(LogicError::Fuel { tick: 0 })
    );
}

/// Each tick of `logic` from 0 to `ticks - 1`: the restart it made, then its result.
fn restart_run(
    logic: &mut WasmLogic,
    host: &mut dyn Host,
    ticks: u64,
) -> Vec<(Option<LogicError>, Result<TickOutput, LogicError>)> {
    (0..ticks)
        .map(|tick| {
            let r = logic.try_tick(host, tick, &[]);
            (logic.take_restart(), r)
        })
        .collect()
}

/// After the fuel trap at tick 0 each of the next eight ticks starts a fresh instance from
/// the seed and runs out of fuel again (`spin` never returns from `tick`); the ninth tick
/// finds the restarts spent and reports `Stopped` with the last failing tick, without
/// entering the component.
#[test]
fn after_running_out_of_fuel_the_logic_restarts_then_stops() {
    let mut logic = WasmLogic::load(&guest("spin"), &SPIN_FUEL).expect("spin loads");
    let world = solid3_world();
    let mut host = WorldHost(&world);
    logic.try_init(&mut host, 0).expect("spin's init returns");
    let restarted = |tick, after, restarts| {
        Some(LogicError::Restarted {
            tick,
            after,
            restarts,
        })
    };
    assert_eq!(
        restart_run(&mut logic, &mut host, 11),
        [
            (None, Err(LogicError::Fuel { tick: 0 })),
            (restarted(1, 0, 1), Err(LogicError::Fuel { tick: 1 })),
            (restarted(2, 1, 2), Err(LogicError::Fuel { tick: 2 })),
            (restarted(3, 2, 3), Err(LogicError::Fuel { tick: 3 })),
            (restarted(4, 3, 4), Err(LogicError::Fuel { tick: 4 })),
            (restarted(5, 4, 5), Err(LogicError::Fuel { tick: 5 })),
            (restarted(6, 5, 6), Err(LogicError::Fuel { tick: 6 })),
            (restarted(7, 6, 7), Err(LogicError::Fuel { tick: 7 })),
            (restarted(8, 7, 8), Err(LogicError::Fuel { tick: 8 })),
            (None, Err(LogicError::Stopped { tick: 9, since: 8 })),
            (None, Err(LogicError::Stopped { tick: 10, since: 8 })),
        ]
    );
}

/// Fail-stop with `max_restarts: 0`: after the fuel trap at tick 0 the logic is stopped, and
/// every later tick reports `Stopped` with the failing tick. A later tick that entered the
/// component would run out of fuel again and report `Fuel`.
#[test]
fn with_no_restarts_a_logic_out_of_fuel_is_stopped() {
    let mut logic = WasmLogic::load(&guest("spin"), &fail_stop(SPIN_FUEL)).expect("spin loads");
    let world = solid3_world();
    let mut host = WorldHost(&world);
    logic.try_init(&mut host, 0).expect("spin's init returns");
    assert_eq!(
        restart_run(&mut logic, &mut host, 3),
        [
            (None, Err(LogicError::Fuel { tick: 0 })),
            (None, Err(LogicError::Stopped { tick: 1, since: 0 })),
            (None, Err(LogicError::Stopped { tick: 2, since: 0 })),
        ]
    );
}

/// A logic whose `init` fails (here: one unit of fuel) reports it through
/// `Server::init_error()` and, with no restarts, is stopped from tick 0; the server still
/// logs inputs.
#[test]
fn a_failed_init_stops_the_logic_from_tick_zero() {
    let starved = fail_stop(WasmConfig {
        fuel_per_tick: 1,
        ..WasmConfig::default()
    });
    let (mut server, mut client) = session("echo", &starved, 0);
    assert_eq!(server.init_error(), Some(&LogicError::Fuel { tick: 0 }));
    let carve = op_bytes(seeded_carves(0x0011, 1)[0]);
    for tick in 0..3 {
        client.send_input(carve.clone());
        assert_eq!(
            server.tick(),
            TickReport {
                tick,
                applied: 0,
                refused: 0,
                inputs: 1,
                logic_error: Some(LogicError::Stopped { tick, since: 0 }),
                restart: None,
            }
        );
    }
    assert!(client.poll().is_empty());
    assert_eq!(server.ops().len(), 0);
    assert_eq!(server.inputs().len(), 3);
    assert_eq!(server.world().voxel_hash(), solid3_world().voxel_hash());
}

/// With the default eight restarts, a logic whose `init` always fails restarts once per
/// tick, each attempt failing in `init` (labelled with its tick), until the cap; then it is
/// stopped since the last failing tick.
#[test]
fn a_failed_init_restarts_once_per_tick_until_the_cap() {
    let starved = WasmConfig {
        fuel_per_tick: 1,
        ..WasmConfig::default()
    };
    let (mut server, mut client) = session("echo", &starved, 0);
    assert_eq!(server.init_error(), Some(&LogicError::Fuel { tick: 0 }));
    let carve = op_bytes(seeded_carves(0x0011, 1)[0]);
    for tick in 0..10u64 {
        client.send_input(carve.clone());
        let (logic_error, restart) = if tick < 8 {
            (
                LogicError::Fuel { tick },
                Some(LogicError::Restarted {
                    tick,
                    after: tick.saturating_sub(1),
                    restarts: tick as u32 + 1,
                }),
            )
        } else {
            (LogicError::Stopped { tick, since: 7 }, None)
        };
        assert_eq!(
            server.tick(),
            TickReport {
                tick,
                applied: 0,
                refused: 0,
                inputs: 1,
                logic_error: Some(logic_error),
                restart,
            },
            "tick {tick}"
        );
    }
    assert!(client.poll().is_empty());
    assert_eq!(server.ops().len(), 0);
    assert_eq!(server.world().voxel_hash(), solid3_world().voxel_hash());
}

#[test]
fn hog_hits_the_memory_cap() {
    let mut logic = WasmLogic::load(&guest("hog"), &WasmConfig::default()).expect("hog loads");
    let world = solid3_world();
    let mut host = WorldHost(&world);
    logic.try_init(&mut host, 0).expect("hog's init returns");
    assert_eq!(
        logic.try_tick(&mut host, 0, &[]),
        Err(LogicError::Memory { tick: 0 })
    );
}

#[test]
fn a_logic_error_is_a_tick_without_logic_ops_whose_inputs_are_logged() {
    let (mut server, mut client) = session("spin", &SPIN_FUEL, 0);
    let carve = op_bytes(seeded_carves(0x0011, 1)[0]);
    client.send_input(carve.clone());
    client.send_input(carve.clone());
    let report = server.tick();
    assert_eq!(
        report,
        TickReport {
            tick: 0,
            applied: 0,
            refused: 0,
            inputs: 2,
            logic_error: Some(LogicError::Fuel { tick: 0 }),
            restart: None,
        }
    );
    assert!(client.poll().is_empty());
    assert_eq!(server.ops().len(), 0);
    let logged: Vec<(u64, u32, Vec<u8>)> = server
        .inputs()
        .entries()
        .iter()
        .map(|e| (e.tick, e.client, e.bytes.clone()))
        .collect();
    assert_eq!(logged, [(0, 0, carve.clone()), (0, 0, carve)]);
    assert_eq!(server.world().voxel_hash(), solid3_world().voxel_hash());
}

/// Ops a logic applied before it failed stay applied and logged. With one restart, `hog`
/// fails at tick 0 after applying that tick's input, restarts at tick 1 and applies that
/// tick's input before it fails again: the log holds both edits in that order. (`hog`'s
/// `init` applies nothing, so the restart's own `init` adds no op.) Re-simulation with a
/// fresh component under the same configuration reproduces the log and the world, and a
/// second run of the session gives the same reports and log.
#[test]
fn edits_applied_before_a_failure_stay_and_resimulate() {
    let config = WasmConfig {
        max_restarts: 1,
        ..WasmConfig::default()
    };
    let carves = seeded_carves(0x0011, 2);
    let run = |server: &mut Server, client: &Client| -> Vec<TickReport> {
        carves
            .iter()
            .map(|op| {
                client.send_input(op_bytes(*op));
                server.tick()
            })
            .collect()
    };
    let (mut server, client) = session("hog", &config, 0);
    let reports = run(&mut server, &client);
    assert_eq!(
        reports,
        [
            TickReport {
                tick: 0,
                applied: 1,
                refused: 0,
                inputs: 1,
                logic_error: Some(LogicError::Memory { tick: 0 }),
                restart: None,
            },
            TickReport {
                tick: 1,
                applied: 1,
                refused: 0,
                inputs: 1,
                logic_error: Some(LogicError::Memory { tick: 1 }),
                restart: Some(LogicError::Restarted {
                    tick: 1,
                    after: 0,
                    restarts: 1,
                }),
            },
        ]
    );
    let logged: Vec<(u64, Option<Op>)> = server
        .ops()
        .entries()
        .iter()
        .map(|e| (e.tick(), e.op().cloned()))
        .collect();
    assert_eq!(
        logged,
        [
            (0, Some(Op::Edit(carves[0]))),
            (1, Some(Op::Edit(carves[1])))
        ]
    );

    let direct = solid3_world();
    for op in &carves {
        direct.apply(op).expect("the carve applies");
    }
    assert_eq!(server.world().voxel_hash(), direct.voxel_hash());

    let fresh = Arc::new(solid3_world());
    let logic = WasmLogic::load(&guest("hog"), &config).expect("hog loads");
    let ops = Server::resimulate(
        fresh.clone(),
        Box::new(logic),
        0,
        &vxe_world::PrefabRegistry::new(),
        server.ops(),
        server.inputs(),
    );
    assert_eq!(ops.encode(), server.ops().encode());
    assert_eq!(fresh.voxel_hash(), direct.voxel_hash());

    // A second run of the same session fails and restarts the same way, after the same ops.
    let (mut again, again_client) = session("hog", &config, 0);
    assert_eq!(run(&mut again, &again_client), reports);
    assert_eq!(again.ops().encode(), server.ops().encode());
}

/// The component's timers are always ready: `timer` sleeps 30 s and then polls a fresh 1 s
/// timer every tick, yet each tick broadcasts `[1]` (ready) and two loads of three ticks take
/// well under one sleep of wall time. A real timer would block the host thread for each sleep.
#[test]
#[allow(
    clippy::disallowed_methods,
    reason = "the test measures that a guest's sleep spends no wall time"
)]
fn timers_are_always_ready_and_never_wait() {
    let start = Instant::now();
    for _ in 0..2 {
        let mut logic =
            WasmLogic::load(&guest("timer"), &WasmConfig::default()).expect("timer loads");
        let world = solid3_world();
        let mut host = WorldHost(&world);
        logic.try_init(&mut host, 0).expect("timer's init returns");
        let broadcasts: Vec<_> = (0..3)
            .map(|tick| logic.try_tick(&mut host, tick, &[]))
            .collect();
        assert_eq!(
            broadcasts,
            [0, 1, 2].map(|_| Ok(TickOutput { broadcast: vec![1] }))
        );
    }
    let elapsed = start.elapsed();
    assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
}

/// Stderr never fills: `chatty` writes 1 KiB per tick, 100 KiB over ticks 0 to 99 (more than
/// the 64 KiB the host keeps for one call), and every one of those ticks returns. Its panic at
/// tick 100 becomes a trap whose message carries the panic's, and the logic (fail-stop here)
/// stops.
#[test]
fn stderr_never_fills_and_a_guest_panic_is_a_trap_with_its_message() {
    let mut logic =
        WasmLogic::load(&guest("chatty"), &fail_stop(WasmConfig::default())).expect("chatty loads");
    let world = solid3_world();
    let mut host = WorldHost(&world);
    logic.try_init(&mut host, 0).expect("chatty's init returns");
    for tick in 0..100 {
        assert_eq!(
            logic.try_tick(&mut host, tick, &[]),
            Ok(TickOutput::default()),
            "tick {tick}"
        );
    }
    match logic.try_tick(&mut host, 100, &[]) {
        Err(LogicError::Trap { tick: 100, message }) => {
            assert!(message.contains("chatty stops at tick 100"), "{message}");
        }
        r => panic!("{r:?}"),
    }
    assert_eq!(
        logic.try_tick(&mut host, 101, &[]),
        Err(LogicError::Stopped {
            tick: 101,
            since: 100
        })
    );
}

/// A host whose `apply` panics, as the world does on some shapes a guest can choose.
struct PanicHost;

impl Host for PanicHost {
    fn apply(&mut self, _op: Op) -> Result<OpResult, EditError> {
        panic!("host refuses");
    }

    fn material_at(&self, _v: VoxelCoord) -> Option<MaterialId> {
        None
    }

    fn raycast(&self, _origin: Fixed3, _dir: Fixed3, _max: i64) -> Option<Hit> {
        None
    }

    fn count_in_box(&self, _lo: VoxelCoord, _hi: VoxelCoord) -> Vec<(MaterialId, u64)> {
        Vec::new()
    }

    fn unsupported(&self, _changed: &[CellCoord]) -> Vec<Region> {
        Vec::new()
    }
}

/// A panic in a host import traps the call with the panic's message and stops the logic
/// (fail-stop here); it does not unwind into the caller (this test would not get to its
/// assertions if it did).
#[test]
fn a_host_import_that_panics_traps_and_stops_the_logic() {
    let mut logic =
        WasmLogic::load(&guest("echo"), &fail_stop(WasmConfig::default())).expect("echo loads");
    let mut host = PanicHost;
    logic.try_init(&mut host, 0).expect("echo's init returns");
    let carve = ClientInput {
        client: 0,
        bytes: op_bytes(seeded_carves(0x0011, 1)[0]),
    };
    match logic.try_tick(&mut host, 0, std::slice::from_ref(&carve)) {
        Err(LogicError::Trap { tick: 0, message }) => {
            assert!(message.contains("host refuses"), "{message}");
        }
        r => panic!("{r:?}"),
    }
    assert_eq!(
        logic.try_tick(&mut host, 1, &[carve]),
        Err(LogicError::Stopped { tick: 1, since: 0 })
    );
}

/// A sphere of radius `i64::MAX` from a component is refused by the world with
/// `OutOfRange` before any fixed-point math, in every build profile: no trap, the logic
/// carries on, and the world is unchanged.
#[test]
fn an_overflowing_shape_from_a_component_is_refused_not_a_trap() {
    let (mut server, mut client) = session("echo", &WasmConfig::default(), 0);
    client.send_input(op_bytes(EditOp::sphere(
        Fixed3([0; 3]),
        i64::MAX,
        Mode::Carve,
    )));
    let report = server.tick();
    assert_eq!(report.logic_error, None, "{report:?}");
    assert_eq!((report.applied, report.refused), (0, 1), "{report:?}");
    let refused: Vec<_> = client
        .poll()
        .into_iter()
        .filter_map(|e| match e {
            Event::Refused { error, .. } => Some(error),
            _ => None,
        })
        .collect();
    assert_eq!(
        refused,
        [EditError::OutOfRange {
            what: Field::Radius
        }]
    );
    assert_eq!(server.world().voxel_hash(), solid3_world().voxel_hash());
}

/// A `greedy` input asking `count_in_box` over the inclusive box `lo..=hi`.
fn count_input(lo: [i32; 3], hi: [i32; 3]) -> ClientInput {
    let mut bytes = vec![1];
    for v in lo.iter().chain(&hi) {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    ClientInput { client: 0, bytes }
}

/// A `greedy` input casting `n` rays `origin + dir * t / 256`, `t <= max`.
fn rays_input(n: u32, origin: Fixed3, dir: [i64; 3], max: i64) -> ClientInput {
    let mut bytes = vec![2];
    bytes.extend_from_slice(&n.to_le_bytes());
    for v in origin.0.iter().chain(&dir).chain([&max]) {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    ClientInput { client: 0, bytes }
}

/// One `greedy` tick on `world` under `config`: its outcome and the fuel left after it.
fn greedy_tick(
    world: &World,
    config: &WasmConfig,
    input: ClientInput,
) -> (Result<TickOutput, LogicError>, Option<u64>) {
    greedy_tick_on(&mut WorldHost(world), config, input)
}

/// [`greedy_tick`] on any host.
fn greedy_tick_on(
    host: &mut dyn Host,
    config: &WasmConfig,
    input: ClientInput,
) -> (Result<TickOutput, LogicError>, Option<u64>) {
    let mut logic = WasmLogic::load(&guest("greedy"), &fail_stop(*config)).expect("greedy loads");
    logic.try_init(host, 0).expect("greedy's init returns");
    let out = logic.try_tick(host, 0, &[input]);
    (out, logic.fuel_left())
}

/// A host straight on a world with a coarse grid, answering the support query as the
/// server's host does: `DEFAULT_SUPPORT`, no explicit anchors, a `changed` list past
/// `max_changed_cells` read as no regions.
struct GridHost<'a> {
    world: WorldHost<'a>,
    grid: CoarseGrid,
}

impl<'a> GridHost<'a> {
    fn new(world: &'a World) -> Self {
        Self {
            world: WorldHost(world),
            grid: CoarseGrid::build(world),
        }
    }
}

impl Host for GridHost<'_> {
    fn apply(&mut self, op: Op) -> Result<OpResult, EditError> {
        self.world.apply(op)
    }

    fn material_at(&self, v: VoxelCoord) -> Option<MaterialId> {
        self.world.material_at(v)
    }

    fn raycast(&self, origin: Fixed3, dir: Fixed3, max: i64) -> Option<Hit> {
        self.world.raycast(origin, dir, max)
    }

    fn count_in_box(&self, lo: VoxelCoord, hi: VoxelCoord) -> Vec<(MaterialId, u64)> {
        self.world.count_in_box(lo, hi)
    }

    fn unsupported(&self, changed: &[CellCoord]) -> Vec<Region> {
        self.unsupported_with_steps(changed).0
    }

    fn unsupported_with_steps(&self, changed: &[CellCoord]) -> (Vec<Region>, u64) {
        let world = self.world.0;
        if changed.len() as u64 > max_changed_cells(world) {
            return (Vec::new(), 0);
        }
        let changes: Vec<CellChange> = changed
            .iter()
            .map(|&c| {
                let (chunk, slot) = c.split();
                let now = self.grid.cell(chunk, slot).unwrap_or_default();
                CellChange {
                    chunk,
                    slot: slot as u16,
                    before: now,
                    after: now,
                }
            })
            .collect();
        find_unsupported_with_stats(
            world,
            &self.grid,
            &DEFAULT_SUPPORT,
            &Anchors::default(),
            &changes,
        )
    }
}

/// `count_in_box` is charged 10 fuel per voxel counted: `1 024^3` resident voxels cost
/// `10 x 2^30`, past the default `10^10` per tick, and the tick traps on fuel; `64^3` costs
/// `10 x 2^18 + 100` and completes, as does `1 024^3` over a world with nothing resident.
#[test]
fn a_greedy_count_in_box_runs_out_of_fuel() {
    let config = WasmConfig::default();
    assert!(10 * (1u64 << 30) > config.fuel_per_tick);
    let solid = World::generate(
        ChunkExtent::new([0, 0, 0], [16, 16, 16]),
        &UniformSource(3),
        &test_table(),
    );
    let (out, _) = greedy_tick(&solid, &config, count_input([0; 3], [1023; 3]));
    assert_eq!(out, Err(LogicError::Fuel { tick: 0 }));

    let (out, left) = greedy_tick(&solid, &config, count_input([0; 3], [63; 3]));
    assert_eq!(out.unwrap().broadcast, (1u64 << 18).to_le_bytes());
    let (zero, zero_left) = greedy_tick(
        &solid,
        &WasmConfig {
            rates: WorkRates::ZERO,
            ..config
        },
        count_input([0; 3], [63; 3]),
    );
    assert!(zero.is_ok());
    assert_eq!(zero_left.unwrap() - left.unwrap(), 10 * (1 << 18) + 100);

    let empty = World::new(&test_table());
    let (out, _) = greedy_tick(&empty, &config, count_input([0; 3], [1023; 3]));
    assert_eq!(out.unwrap().broadcast, 0u64.to_le_bytes());
}

/// 100 rays of `max_t = 2^22` through an empty world are charged `100 + 10 s` each, `s` the
/// steps `raycast_with_stats` reports; with fuel for the guest's own instructions plus half
/// that charge, the tick traps on fuel. A refused ray is charged `per_call` only, and under
/// `WorkRates::ZERO` nothing is charged at all.
#[test]
fn rays_are_charged_by_their_steps() {
    let world = World::new(&test_table());
    let o = centre_of_voxel([0, 0, 0]);
    let (dir, max) = ([256, 0, 0], 1i64 << 22);
    let (hit, s) = world.raycast_with_stats(o, dir, max).unwrap();
    assert_eq!(hit, None);
    assert!((256..=258).contains(&s), "{s} steps");
    let config = WasmConfig::default();
    let zero = WasmConfig {
        rates: WorkRates::ZERO,
        ..config
    };
    let left = |config: &WasmConfig, max: i64| {
        let (out, left) = greedy_tick(&world, config, rays_input(100, o, dir, max));
        assert_eq!(out.unwrap().broadcast, 0u32.to_le_bytes());
        left.unwrap()
    };
    let charge = 100 * (100 + 10 * s);
    let zero_long = left(&zero, max);
    assert_eq!(zero_long - left(&config, max), charge);

    // Refused rays: one `per_call` each, and the guest's own work is the same as for the
    // long rays, so under ZERO the two runs use equal fuel.
    let zero_refused = left(&zero, max + 1);
    assert_eq!(zero_refused, zero_long);
    assert_eq!(zero_refused - left(&config, max + 1), 100 * 100);

    let used = config.fuel_per_tick - zero_long;
    let tight = WasmConfig {
        fuel_per_tick: used + charge / 2,
        ..config
    };
    let (out, _) = greedy_tick(&world, &tight, rays_input(100, o, dir, max));
    assert_eq!(out, Err(LogicError::Fuel { tick: 0 }));
    let tight_zero = WasmConfig {
        rates: WorkRates::ZERO,
        ..tight
    };
    let (out, _) = greedy_tick(&world, &tight_zero, rays_input(100, o, dir, max));
    assert!(out.is_ok(), "{out:?}");
}

/// A `greedy` input calling the `apply` import `n` times with `bytes` as the op.
fn raw_apply_input(n: u32, bytes: &[u8]) -> ClientInput {
    let mut out = vec![3];
    out.extend_from_slice(&n.to_le_bytes());
    out.extend_from_slice(bytes);
    ClientInput {
        client: 0,
        bytes: out,
    }
}

/// A `greedy` input calling the `apply` import `n` times with `len` bytes of `0xFF`.
fn junk_apply_input(n: u32, len: u32) -> ClientInput {
    let mut bytes = vec![4];
    bytes.extend_from_slice(&n.to_le_bytes());
    bytes.extend_from_slice(&len.to_le_bytes());
    ClientInput { client: 0, bytes }
}

/// A `greedy` input asking `unsupported` over `cells`.
fn unsupported_input(cells: &[CellCoord]) -> ClientInput {
    let mut bytes = vec![5];
    for c in cells {
        for v in c.0 {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
    }
    ClientInput { client: 0, bytes }
}

/// The fuel `input` costs under the default rates beyond what it costs under
/// `WorkRates::ZERO`, both ticks completing on `host`.
fn charged(host: &mut dyn Host, input: &ClientInput) -> u64 {
    charged_under(host, &WasmConfig::default(), input)
}

/// [`charged`] with `config`'s fuel and memory.
fn charged_under(host: &mut dyn Host, config: &WasmConfig, input: &ClientInput) -> u64 {
    let config = *config;
    let zero = WasmConfig {
        rates: WorkRates::ZERO,
        ..config
    };
    let (out, left) = greedy_tick_on(host, &config, input.clone());
    assert!(out.is_ok(), "{out:?}");
    let (out, zero_left) = greedy_tick_on(host, &zero, input.clone());
    assert!(out.is_ok(), "{out:?}");
    zero_left.unwrap() - left.unwrap()
}

/// `apply`'s op bytes are charged one fuel each before the host decodes them: an
/// undecodable op of 4 096 bytes costs `100 + 4 096`, and a drop of 4 097 cells, decoded and
/// refused with `RegionCells`, costs `100 +` its encoded length.
#[test]
fn apply_is_charged_by_its_op_bytes() {
    let world = solid3_world();
    assert_eq!(
        charged(&mut WorldHost(&world), &junk_apply_input(1, 4096)),
        100 + 4096
    );

    let drop = Op::Drop {
        cells: (0..4097)
            .map(|i| CellCoord([i % 16, i / 16 % 16, i / 256]))
            .collect(),
        max_drop: 8,
        yield_below: 0,
    };
    let mut bytes = Vec::new();
    vxe_world::wire::encode_op(&mut bytes, &drop);
    assert_eq!(bytes.len(), 1 + 4 + 4 + 12 * 4097);
    assert_eq!(
        drop.apply(&world, &PrefabRegistry::new()),
        Err(EditError::OutOfRange {
            what: Field::RegionCells
        })
    );
    let input = raw_apply_input(1, &bytes);
    let (out, _) = greedy_tick(&world, &WasmConfig::default(), input.clone());
    assert_eq!(out.unwrap().broadcast, [vxe_script_wasm::ANSWER_REFUSED]);
    assert_eq!(
        charged(&mut WorldHost(&world), &input),
        100 + bytes.len() as u64
    );
}

/// A guest calling `apply` with one 1 MiB undecodable buffer over and over pays `2^20 + 100`
/// per call. Under `10^8` fuel (a hundredth of the default, so the debug build lifts 100 MiB
/// rather than 10 GiB; the default covers at most 9 535 calls the same way) at most 95 calls
/// fit: 90 complete, 200 end the tick on fuel.
#[test]
fn a_loop_of_large_undecodable_applies_runs_out_of_fuel() {
    let world = solid3_world();
    let config = WasmConfig {
        fuel_per_tick: 100_000_000,
        ..WasmConfig::default()
    };
    assert_eq!(config.fuel_per_tick / ((1 << 20) + 100), 95);
    let (out, _) = greedy_tick(&world, &config, junk_apply_input(90, 1 << 20));
    assert_eq!(out.unwrap().broadcast, 90u32.to_le_bytes());
    let (out, _) = greedy_tick(&world, &config, junk_apply_input(200, 1 << 20));
    assert_eq!(out, Err(LogicError::Fuel { tick: 0 }));
}

/// `unsupported` over `K` cells is charged `12 K` before the query and `100 + 10 s` after it,
/// `s` the cells the query looked at; a list past `max_changed_cells` (129^3 at the default
/// limits) reads as no regions and costs only `100 + 12 K`.
#[test]
fn unsupported_is_charged_by_its_cells_and_its_flood() {
    let world = slab_world();
    let mut host = GridHost::new(&world);
    let cells = slab_cells(2, 5);
    let (regions, s) = host.unsupported_with_steps(&cells);
    assert!(regions.is_empty(), "{regions:?}");
    assert!(s > cells.len() as u64, "{s} steps");
    let k = cells.len() as u64;
    assert_eq!(
        charged(&mut host, &unsupported_input(&cells)),
        100 + 12 * k + 10 * s
    );

    let cap = max_changed_cells(&world);
    assert_eq!(cap, 129 * 129 * 129);
    let over = vec![CellCoord([9, 3, 5]); cap as usize + 1];
    assert_eq!(host.unsupported_with_steps(&over), (Vec::new(), 0));
    // The 2 146 690-cell list and its input take about 50 MiB of guest memory.
    let roomy = WasmConfig {
        memory_bytes: 256 << 20,
        ..WasmConfig::default()
    };
    assert_eq!(
        charged_under(&mut host, &roomy, &unsupported_input(&over)),
        100 + 12 * (cap + 1)
    );
}

/// `WorkRates::apply_cost` at the default rates: `100 + 10 x (bounds voxels + 64 x touched
/// chunks)` for an applied op, an edit's bounds being its shape's and a stamp's or a drop's
/// its result's aabb; `100` alone for a refusal.
#[test]
fn apply_cost_charges_the_bounds_and_the_touched_chunks() {
    let rates = WorkRates::DEFAULT;
    let registry = PrefabRegistry::new();

    // A carve of the box [60, 68) x [0, 8) x [0, 8): 512 voxels across chunks x = 0 and 1.
    let world = solid3_world();
    let carve = Op::Edit(box_voxels([60, 0, 0], [68, 8, 8], Mode::Carve));
    let r = carve.apply(&world, &registry);
    assert_eq!(r.as_ref().unwrap().edit_result().touched.len(), 2);
    assert_eq!(rates.apply_cost(&carve, &r), 100 + 10 * (512 + 64 * 2));

    // A stamp of the 16^3 vault into air at [100, 100, 56]: its aabb is the vault's 4 096
    // voxels, across chunks z = 0 and 1.
    let world = slab_world();
    let mut prefabs = PrefabRegistry::new();
    let vault = prefabs.register(slab_vault()).unwrap();
    let stamp = Op::Stamp {
        prefab: vault,
        origin: VoxelCoord([100, 100, 56]),
        rot: vxe_world::Rot::all().next().unwrap(),
    };
    let r = stamp.apply(&world, &prefabs);
    let res = r.as_ref().unwrap();
    assert_eq!(
        res.aabb(),
        Some((VoxelCoord([100, 100, 56]), VoxelCoord([115, 115, 71])))
    );
    assert_eq!(res.edit_result().touched.len(), 2);
    assert_eq!(rates.apply_cost(&stamp, &r), 100 + 10 * (4096 + 64 * 2));

    // With pillar A carved, the slab over it [64, 88) x [16, 40) x [40, 48) drops by its
    // `max_drop` of 8: the changed voxels span z 32..=47, 24 x 24 x 16 = 9 216 voxels, all in
    // chunk (1, 0, 0).
    let world = slab_world();
    Op::Edit(box_voxels(PILLAR_A.0, PILLAR_A.1, Mode::Carve))
        .apply(&world, &registry)
        .unwrap();
    let drop = Op::Drop {
        cells: slab_cells(2, 5),
        max_drop: 8,
        yield_below: 0,
    };
    let r = drop.apply(&world, &registry);
    let res = r.as_ref().unwrap();
    assert_eq!(
        res.aabb(),
        Some((VoxelCoord([64, 16, 32]), VoxelCoord([87, 39, 47])))
    );
    assert_eq!(res.edit_result().touched.len(), 1);
    assert_eq!(rates.apply_cost(&drop, &r), 100 + 10 * (9216 + 64));

    // Refusals: per_call only.
    let world = solid3_world();
    let outside = Op::Edit(box_voxels([200, 0, 0], [208, 8, 8], Mode::Carve));
    let r = outside.apply(&world, &registry);
    assert!(matches!(r, Err(EditError::NotResident(_))), "{r:?}");
    assert_eq!(rates.apply_cost(&outside, &r), 100);
    let huge = Op::Edit(EditOp::sphere(Fixed3([0; 3]), i64::MAX, Mode::Carve));
    let r = huge.apply(&world, &registry);
    assert_eq!(
        r,
        Err(EditError::OutOfRange {
            what: Field::Radius
        })
    );
    assert_eq!(rates.apply_cost(&huge, &r), 100);
}
