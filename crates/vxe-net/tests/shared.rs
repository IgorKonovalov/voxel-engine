//! Messages, the server's tick with directly submitted ops, and a shared client.

mod support;

use std::sync::Arc;

use support::*;
use vxe_net::*;
use vxe_world::*;

fn origin_sphere(mode: Mode) -> EditOp {
    EditOp::sphere(centre_of_voxel([0, 0, 0]), 10 * FIXED_ONE, mode)
}

#[test]
fn message_sizes_are_the_documented_ones() {
    let applied = Message::Applied {
        tick: 1,
        seq: 2,
        op: Op::Edit(origin_sphere(Mode::Carve)),
        result: EditResult {
            changed_from: vec![(3, 4169)],
            touched: vec![ChunkCoord([0, 0, 0])],
            aabb: Some((VoxelCoord([-10, -10, -10]), VoxelCoord([10, 10, 10]))),
        },
    };
    // tag 1 + tick 8 + seq 4 + op 35 + (2 + 6) + (2 + 12) + (1 + 24).
    assert_eq!(applied.encode().len(), 95);
    for n in [0usize, 1, 40, 65535] {
        assert_eq!(
            Message::Input { bytes: vec![7; n] }.encode().len(),
            1 + 2 + n
        );
    }
}

#[test]
fn every_message_tag_round_trips() {
    let op = Op::Edit(origin_sphere(Mode::Fill(2)));
    let messages = [
        Message::Applied {
            tick: u64::MAX,
            seq: 9,
            op,
            result: EditResult::default(),
        },
        Message::Refused {
            tick: 3,
            seq: u32::MAX,
            op,
            error: EditError::NotResident(ChunkCoord([156, -1, 0])),
        },
        Message::Event {
            tick: 4,
            bytes: vec![1, 2, 3],
        },
        Message::Input { bytes: vec![] },
    ];
    let tags = [MSG_APPLIED, MSG_REFUSED, MSG_EVENT, MSG_INPUT];
    for (m, tag) in messages.into_iter().zip(tags) {
        let bytes = m.encode();
        assert_eq!(bytes[0], tag);
        assert_eq!(Message::decode(&bytes), Ok(m));
    }
    assert_eq!(Message::decode(&[0]), Err(WireError::UnknownKind(0)));
    assert_eq!(Message::decode(&[]), Err(WireError::Truncated { at: 0 }));
    assert_eq!(
        Message::decode(&[MSG_INPUT, 2, 0, 1]),
        Err(WireError::Truncated { at: 0 })
    );
    assert_eq!(
        Message::decode(&[MSG_INPUT, 0, 0, 1]),
        Err(WireError::Trailing { at: 3 })
    );
}

#[test]
fn one_tick_applies_submitted_ops_in_order_and_the_shared_client_sees_them() {
    let world = Arc::new(solid3_world());
    let mut server = Server::new(world.clone(), Box::new(NoLogic), 0);
    let (near, far) = MemChannel::pair();
    assert_eq!(server.attach(far), 0);
    let mut client = Client::shared(world, near);

    let seqs = [
        server.submit(origin_sphere(Mode::Carve)),
        server.submit(origin_sphere(Mode::Fill(2))),
        server.submit(origin_sphere(Mode::Replace { from: 2, to: 5 })),
        server.submit(EditOp::sphere(
            centre_of_voxel([10000, 0, 0]),
            10 * FIXED_ONE,
            Mode::Carve,
        )),
    ];
    assert_eq!(seqs, [0, 1, 2, 3]);
    assert!(
        client.poll().is_empty(),
        "nothing is announced before the tick"
    );

    let report = server.tick();
    assert_eq!(
        report,
        TickReport {
            tick: 0,
            applied: 3,
            refused: 1,
            inputs: 0,
            logic_error: None,
            restart: None,
        }
    );
    let events = client.poll();
    assert_eq!(events.len(), 4, "{events:?}");
    let expected_changes = [[(3, 4169)], [(0, 4169)], [(2, 4169)]];
    for (i, want) in expected_changes.iter().enumerate() {
        match &events[i] {
            Event::Applied {
                tick: 0,
                seq,
                result,
                ..
            } => {
                assert_eq!(*seq, i as u32);
                assert_eq!(result.changed_from, want);
            }
            e => panic!("event {i}: {e:?}"),
        }
    }
    match &events[3] {
        Event::Refused {
            tick: 0,
            seq: 3,
            error: EditError::NotResident(_),
            ..
        } => {}
        e => panic!("event 3: {e:?}"),
    }
    // Three op entries, then the boundary after the tick's submitted ops at the next seq.
    assert_eq!(server.ops().len(), 3);
    let logged: Vec<(u32, bool)> = server
        .ops()
        .entries()
        .iter()
        .map(|e| (e.seq(), e.op().is_some()))
        .collect();
    assert_eq!(logged, [(0, true), (1, true), (2, true), (4, false)]);
    assert_eq!(
        server.ops().entries()[3],
        LogEntry::Boundary { tick: 0, seq: 4 }
    );
    assert_eq!(server.current_tick(), 1);
    assert!(Arc::ptr_eq(client.shared_world().unwrap(), server.world()));
    assert_eq!(server.world().material_at([0, 0, 0]), Some(5));
}

#[test]
fn submissions_from_two_threads_are_logged_once_each_in_seq_order() {
    let world = Arc::new(solid3_world());
    let mut server = Server::new(world, Box::new(NoLogic), 0);
    let carves = seeded_carves(0xC0FFEE, 200);
    let (left, right) = carves.split_at(100);
    let mut submitted: Vec<(u32, EditOp)> = std::thread::scope(|s| {
        let server = &server;
        let a = s.spawn(move || {
            left.iter()
                .map(|op| (server.submit(*op), *op))
                .collect::<Vec<_>>()
        });
        let b = s.spawn(move || {
            right
                .iter()
                .map(|op| (server.submit(*op), *op))
                .collect::<Vec<_>>()
        });
        let mut v = a.join().unwrap();
        v.extend(b.join().unwrap());
        v
    });
    let report = server.tick();
    assert_eq!((report.applied, report.refused), (200, 0));

    let (entries, boundary) = server.ops().entries().split_at(200);
    assert_eq!(boundary, [LogEntry::Boundary { tick: 0, seq: 200 }]);
    let seqs: Vec<u32> = entries.iter().map(LogEntry::seq).collect();
    assert_eq!(seqs, (0..200).collect::<Vec<u32>>());
    // Each op exactly once, under the seq submit returned for it.
    submitted.sort_by_key(|(seq, _)| *seq);
    let logged: Vec<(u32, EditOp)> = entries
        .iter()
        .map(|e| match *e {
            LogEntry::Op {
                seq,
                op: Op::Edit(op),
                ..
            } => (seq, op),
            LogEntry::Boundary { .. } => panic!("boundary among the op entries: {e:?}"),
        })
        .collect();
    assert_eq!(logged, submitted);
    let mut all: Vec<EditOp> = logged.iter().map(|(_, op)| *op).collect();
    all.sort_by_key(|op| format!("{op:?}"));
    let mut want = carves.clone();
    want.sort_by_key(|op| format!("{op:?}"));
    assert_eq!(all, want);

    let fresh = solid3_world();
    for (_, op) in &logged {
        fresh.apply(op).unwrap();
    }
    assert_eq!(fresh.voxel_hash(), server.world().voxel_hash());
    assert_ne!(fresh.voxel_hash(), solid3_world().voxel_hash());
}
