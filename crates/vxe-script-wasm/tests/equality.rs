//! The walking skeleton of the host (ADR-0010): one logic, native and as a component, driven
//! by the same inputs through `Server`, gives the same op-log bytes and the same world.

mod support;

use std::sync::Arc;

use support::guest;
use vxe_net::test_logic::*;
use vxe_net::*;
use vxe_script_wasm::{WasmConfig, WasmLogic};

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
        wasm.ops(),
        wasm.inputs(),
    );
    assert_eq!(ops.encode(), native.ops().encode());
    assert_eq!(fresh.voxel_hash(), ECHO_HASH);
}
