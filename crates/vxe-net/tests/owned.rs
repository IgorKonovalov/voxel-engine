//! The owned client: its own world kept in step by applying every announced op, and desync
//! detection when a result differs from the server's.

mod support;

use std::sync::Arc;

use support::*;
use vxe_net::*;
use vxe_world::*;

/// A server with `EchoLogic` on one `solid3_world()` and an owned client on another.
fn owned_session() -> (Server, Client) {
    let mut server = Server::new(Arc::new(solid3_world()), Box::new(EchoLogic), 0);
    let (near, far) = MemChannel::pair();
    server.attach(far);
    (server, Client::owned(solid3_world(), near))
}

/// Sends `ops` as inputs, ten per tick, and returns every event the client polled.
fn drive(server: &mut Server, client: &mut Client, ops: &[EditOp]) -> Vec<Event> {
    let mut events = Vec::new();
    for chunk in ops.chunks(10) {
        for op in chunk {
            client.send_input(op_bytes(*op));
        }
        server.tick();
        events.extend(client.poll());
    }
    events
}

#[test]
fn an_owned_client_stays_in_step_over_100_seeded_carves() {
    let (mut server, mut client) = owned_session();
    assert!(client.shared_world().is_none());
    let carves = seeded_carves(0x0D0E, 100);
    let events = drive(&mut server, &mut client, &carves);

    let applied: Vec<u32> = events
        .iter()
        .filter_map(|e| match e {
            Event::Applied { seq, .. } => Some(*seq),
            _ => None,
        })
        .collect();
    assert_eq!(applied, (0..100).collect::<Vec<u32>>());
    assert!(!events.iter().any(|e| matches!(e, Event::Desync { .. })));
    // `EchoLogic`'s outcome bytes pass through untouched: one event a tick.
    let logic: Vec<&Event> = events
        .iter()
        .filter(|e| matches!(e, Event::Logic { .. }))
        .collect();
    assert_eq!(logic.len(), 10);
    assert!(logic.iter().all(|e| matches!(
        e,
        Event::Logic { bytes, .. } if *bytes == [ECHO_APPLIED; 10]
    )));
    assert!(!client.desynced());
    assert_eq!(client.world().voxel_hash(), server.world().voxel_hash());
    assert_ne!(client.world().voxel_hash(), solid3_world().voxel_hash());
}

#[test]
fn a_result_that_differs_raises_desync_and_the_client_stops_applying() {
    let (mut server, mut client) = owned_session();
    let fault = EditOp::sphere(centre_of_voxel([40, 40, 40]), 5 * FIXED_ONE, Mode::Carve);
    // 1 m carves whose spheres stay clear of the fault sphere (6 + 10 voxels and a margin).
    let carves: Vec<EditOp> = seeded_carves(0xFA17, 200)
        .into_iter()
        .filter(|op| {
            let Shape::Sphere { c, .. } = op.shape else {
                unreachable!()
            };
            let d = c.0.map(|v| v / FIXED_ONE - 40);
            d.iter().map(|v| v * v).sum::<i64>() > 20 * 20
        })
        .take(20)
        .collect();
    assert_eq!(carves.len(), 20);

    let events = drive(&mut server, &mut client, &carves[..10]);
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::Applied { .. }))
            .count(),
        10
    );
    assert_eq!(client.world().voxel_hash(), server.world().voxel_hash());

    // After 10 ops the client's world diverges: the fault sphere is carved on it alone.
    let local = client.world().apply(&fault).unwrap();
    assert!(!local.changed_from.is_empty());

    // Input 10 is an ordinary carve, input 11 the fault sphere, then inputs 12 to 19.
    let mut rest = vec![carves[10], fault];
    rest.extend_from_slice(&carves[12..20]);
    client.send_input(op_bytes(rest[0]));
    client.send_input(op_bytes(rest[1]));
    server.tick();
    let events = client.poll();
    assert!(
        matches!(&events[0], Event::Applied { seq: 10, .. }),
        "{events:?}"
    );
    match &events[1] {
        Event::Desync {
            seq: 11,
            ours,
            theirs,
        } => {
            // The client's world already lacks the fault sphere: its carve changed nothing.
            assert_eq!(*ours, Some(OpResult::Edit(EditResult::default())));
            let OpResult::Edit(theirs) = theirs else {
                panic!("{theirs:?}")
            };
            assert_eq!(theirs.changed_from.len(), 1);
            let (m, n) = theirs.changed_from[0];
            assert_eq!(m, 3);
            assert!(n > 0);
        }
        e => panic!("{e:?}"),
    }
    assert!(client.desynced());
    let after_desync = client.world().voxel_hash();

    // Ops 12 onward arrive with the server's results and are not applied.
    let events = drive(&mut server, &mut client, &rest[2..]);
    let applied: Vec<u32> = events
        .iter()
        .filter_map(|e| match e {
            Event::Applied { seq, .. } => Some(*seq),
            _ => None,
        })
        .collect();
    assert_eq!(applied, (12..20).collect::<Vec<u32>>());
    assert!(!events.iter().any(|e| matches!(e, Event::Desync { .. })));
    assert_eq!(client.world().voxel_hash(), after_desync);
    assert_ne!(client.world().voxel_hash(), server.world().voxel_hash());
}

#[test]
fn an_op_the_owned_world_refuses_is_a_desync() {
    let mut server = Server::new(Arc::new(solid3_world()), Box::new(EchoLogic), 0);
    let (near, far) = MemChannel::pair();
    server.attach(far);
    // The client's world lacks the server's chunks on +x.
    let small = World::generate(
        ChunkExtent::new([-2, -2, -2], [1, 2, 2]),
        &UniformSource(3),
        &test_table(),
    );
    let mut client = Client::owned(small, near);
    client.send_input(op_bytes(EditOp::sphere(
        centre_of_voxel([100, 0, 0]),
        FIXED_ONE,
        Mode::Carve,
    )));
    server.tick();
    let events = client.poll();
    assert!(
        matches!(
            &events[0],
            Event::Desync {
                seq: 0,
                ours: None,
                ..
            }
        ),
        "{events:?}"
    );
    assert!(client.desynced());
}
