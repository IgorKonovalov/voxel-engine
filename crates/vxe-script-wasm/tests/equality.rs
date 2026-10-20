//! The walking skeleton of the host (ADR-0010): one logic, native and as a component, driven
//! by the same inputs through `Server`, gives the same op-log bytes and the same world, for
//! every op kind and the support query.

mod support;

use std::sync::Arc;

use support::guest;
use vxe_net::test_logic::*;
use vxe_net::*;
use vxe_script_wasm::{ANSWER_REFUSED, WasmConfig, WasmLogic, decode_answer, encode_answer};
use vxe_world::{
    Anchors, CellChange, CoarseGrid, EditError, Mode, Op, PrefabRegistry, Rot, VoxelCoord,
    find_unsupported,
};

/// The world hash of `EchoLogic`'s run on `seeded_carves(0xEC40, 100)`, ten a tick, from
/// `solid3_world()`: the inputs of `vxe-net`'s `EchoLogic` test.
const ECHO_HASH: u64 = 0x8f7d_4016_15d9_5c4d;

/// Ten ticks of ten inputs, each a wire-encoded 1 m carve at a seeded centre, sent by one
/// shared client; returns the server and every event the client received.
fn drive(logic: Box<dyn Logic>) -> (Server, Vec<Event>) {
    let world = Arc::new(solid3_world());
    let mut server = Server::new(world.clone(), logic, 0);
    let (near, far) = MemChannel::pair();
    server.attach(far);
    let mut client = Client::shared(world, near);
    let carves = seeded_carves(0xEC40, 100);
    let mut events = Vec::new();
    for (tick, batch) in carves.chunks(10).enumerate() {
        for op in batch {
            client.send_input(op_bytes(*op));
        }
        let report = server.tick();
        assert_eq!(
            (report.tick, report.applied, report.refused, report.inputs),
            (tick as u64, 10, 0, 10)
        );
        events.extend(client.poll());
    }
    (server, events)
}

fn echo_component() -> WasmLogic {
    WasmLogic::load(&guest("echo"), &WasmConfig::default()).expect("the echo component loads")
}

#[test]
fn echo_component_equals_native_echo_logic() {
    let (native, native_events) = drive(Box::new(EchoLogic));
    let (wasm, wasm_events) = drive(Box::new(echo_component()));

    assert_eq!(native.ops().len(), 100);
    assert_eq!(wasm.ops().encode(), native.ops().encode());
    assert_eq!(wasm.inputs().encode(), native.inputs().encode());
    assert_eq!(wasm_events, native_events);
    assert_eq!(native.world().voxel_hash(), ECHO_HASH);
    assert_eq!(wasm.world().voxel_hash(), ECHO_HASH);

    // Re-simulating the component's session through the component reproduces it.
    let fresh = Arc::new(solid3_world());
    let ops = Server::resimulate(
        fresh.clone(),
        Box::new(echo_component()),
        0,
        &PrefabRegistry::new(),
        wasm.ops(),
        wasm.inputs(),
    );
    assert_eq!(ops.encode(), native.ops().encode());
    assert_eq!(fresh.voxel_hash(), ECHO_HASH);
}

/// The slab session's ops as inputs, one tick each, with a support query about pillar A's
/// cells after its carve, then a stamp of an id nobody registered; returns the server and
/// every event the client received.
fn drive_kinds(logic: Box<dyn Logic>) -> (Server, Vec<Event>) {
    let world = Arc::new(slab_world());
    let mut server = Server::new(world.clone(), logic, 0);
    let id = server.register_prefab(slab_vault()).unwrap();
    let (near, far) = MemChannel::pair();
    server.attach(far);
    let mut client = Client::shared(world, near);
    let pillar = box_voxels(PILLAR_A.0, PILLAR_A.1, Mode::Carve);
    let mut events = Vec::new();
    let mut ticks: Vec<Vec<Vec<u8>>> = slab_session(id)
        .into_iter()
        .map(|op| vec![op_bytes(op)])
        .collect();
    let aabb = Some((
        VoxelCoord(PILLAR_A.0),
        VoxelCoord(PILLAR_A.1.map(|v| v - 1)),
    ));
    ticks[1].push(query_bytes(&cells_in(aabb)));
    ticks.push(vec![op_bytes(Op::Stamp {
        prefab: id ^ 1,
        origin: VoxelCoord([0, 0, 64]),
        rot: Rot::IDENTITY,
    })]);
    assert_eq!(ticks[1][0], op_bytes(pillar));
    for inputs in ticks {
        for bytes in inputs {
            client.send_input(bytes);
        }
        let report = server.tick();
        assert_eq!(report.logic_error, None, "{report:?}");
        events.extend(client.poll());
    }
    (server, events)
}

/// The echo logic hosted as a component and linked natively stamp, carve, drop, remove and
/// ask the support query alike, and a stamp of an unknown id is refused without a trap.
#[test]
fn echo_component_equals_native_echo_logic_on_every_kind() {
    let (native, native_events) = drive_kinds(Box::new(EchoLogic));
    let (wasm, wasm_events) = drive_kinds(Box::new(echo_component()));

    assert_eq!(native.ops().len(), 8);
    assert_eq!(wasm.ops().encode(), native.ops().encode());
    assert_eq!(wasm.inputs().encode(), native.inputs().encode());
    assert_eq!(wasm_events, native_events);
    assert_eq!(native.world().voxel_hash(), SLAB_SESSION_HASH);
    assert_eq!(wasm.world().voxel_hash(), SLAB_SESSION_HASH);

    // Tick 1's broadcast: the carve applied, then the one region the query found, slab A's
    // 9 cells and its rock.
    let broadcasts: Vec<&Vec<u8>> = wasm_events
        .iter()
        .filter_map(|e| match e {
            Event::Logic { bytes, .. } => Some(bytes),
            _ => None,
        })
        .collect();
    assert_eq!(broadcasts.len(), 9);
    let tick1 = broadcasts[1];
    assert_eq!(&tick1[..2], &[ECHO_APPLIED, ECHO_REGIONS]);
    let mut r = vxe_world::wire::Reader::new(&tick1[2..]);
    let regions = decode_regions(&mut r).unwrap();
    assert!(r.is_empty());
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].cells, slab_cells(2, 5));
    assert_eq!(regions[0].counts, [(SLAB_ROCK, 4608)]);
    assert_eq!(
        regions[0].aabb,
        (VoxelCoord([64, 16, 40]), VoxelCoord([87, 39, 47]))
    );
    // The same query straight on a world in that state.
    let direct = slab_world();
    let mut ops = slab_session(slab_vault().id());
    ops.truncate(2);
    let reg = {
        let mut reg = PrefabRegistry::new();
        reg.register(slab_vault()).unwrap();
        reg
    };
    let mut changes = Vec::new();
    let mut grid = CoarseGrid::build(&direct);
    for op in &ops {
        let r = op.apply(&direct, &reg).unwrap();
        changes = grid.update_aabb(&direct, r.aabb());
    }
    let changed: Vec<CellChange> = changes;
    let expected = find_unsupported(
        &direct,
        &grid,
        &DEFAULT_SUPPORT,
        &Anchors::default(),
        &changed,
    );
    assert_eq!(regions, expected);

    // The unknown stamp: refused with its id, answered with tag 1, no trap.
    assert_eq!(broadcasts[8], &vec![ECHO_REFUSED]);
    let unknown = slab_vault().id() ^ 1;
    assert!(wasm_events.iter().any(|e| matches!(
        e,
        Event::Refused { error: EditError::UnknownPrefab(u), .. } if *u == unknown
    )));
    let answer = encode_answer(&Err(EditError::UnknownPrefab(unknown)));
    assert_eq!(answer[0], ANSWER_REFUSED);
    assert_eq!(
        decode_answer(&answer),
        Ok(Some(Err(EditError::UnknownPrefab(unknown))))
    );

    // Re-simulating through the component, with the save's table, reproduces it.
    let fresh = Arc::new(slab_world());
    let table = wasm.prefabs().subset(wasm.ops().prefab_ids());
    let ops = Server::resimulate(
        fresh.clone(),
        Box::new(echo_component()),
        0,
        &table,
        wasm.ops(),
        wasm.inputs(),
    );
    assert_eq!(ops.encode(), native.ops().encode());
    assert_eq!(fresh.voxel_hash(), SLAB_SESSION_HASH);
}
