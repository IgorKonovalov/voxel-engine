//! The component observes nothing but its seed and its inputs. Each knob of the host's
//! configuration has a guest that would notice its absence: `probe` reads NaN bits, both
//! clocks, `getrandom`, `HashMap` order, the filesystem and the environment; `spin` never
//! returns; `hog` allocates 1 GiB.

mod support;

use std::sync::Arc;

use support::guest;
use vxe_net::test_logic::*;
use vxe_net::*;
use vxe_script_wasm::{WasmConfig, WasmLogic};
use vxe_world::hash::mix64;
use vxe_world::{EditError, EditOp, EditResult, Fixed3, Hit, MaterialId, VoxelCoord, World};

/// A host straight on a world, for calling a [`WasmLogic`] without a server.
struct WorldHost<'a>(&'a World);

impl Host for WorldHost<'_> {
    fn apply(&mut self, op: EditOp) -> Result<EditResult, EditError> {
        self.0.apply(&op)
    }

    fn material_at(&self, v: VoxelCoord) -> Option<MaterialId> {
        self.0.material_at(v)
    }

    fn raycast(&self, origin: Fixed3, dir: Fixed3, max: i64) -> Option<Hit> {
        self.0.raycast(origin, dir.0, max)
    }

    fn count_in_box(&self, lo: VoxelCoord, hi: VoxelCoord) -> Vec<(MaterialId, u64)> {
        self.0.count_in_box(lo, hi)
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
        }
    );
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

/// Blocked: wasmtime refuses to enter a component instance again after any trap ("cannot
/// enter component instance"), so the tick after one fails with `LogicError::Trap` instead
/// of running on fresh fuel. What the logic's state should be after a trap is open.
#[test]
#[ignore = "blocked: a trapped component instance cannot be re-entered; the recovery is a design call"]
fn after_running_out_of_fuel_the_next_tick_runs_again() {
    let mut logic = WasmLogic::load(&guest("spin"), &SPIN_FUEL).expect("spin loads");
    let world = solid3_world();
    let mut host = WorldHost(&world);
    logic.try_init(&mut host, 0).expect("spin's init returns");
    for tick in 0..3 {
        assert_eq!(
            logic.try_tick(&mut host, tick, &[]),
            Err(LogicError::Fuel { tick })
        );
    }
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
