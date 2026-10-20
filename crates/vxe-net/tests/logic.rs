//! The logic boundary: inputs through the channel to a `Logic`, the `Host` handle, and
//! re-simulation from a seed and an input log.

mod support;

use std::sync::Arc;

use support::*;
use vxe_net::*;
use vxe_world::*;

/// A server on a fresh `solid3_world()` with one attached shared client.
fn session(logic: Box<dyn Logic>, seed: u64) -> (Server, Client) {
    let world = Arc::new(solid3_world());
    let mut server = Server::new(world.clone(), logic, seed);
    let (near, far) = MemChannel::pair();
    assert_eq!(server.attach(far), 0);
    (server, Client::shared(world, near))
}

/// Answers every host query once in `init`, so a test can compare them with the world's.
struct Probe(Arc<std::sync::Mutex<Vec<String>>>);

impl Logic for Probe {
    fn init(&mut self, host: &mut dyn Host, _seed: u64) {
        let mut out = self.0.lock().unwrap();
        let r = host.apply(Op::Edit(EditOp::sphere(
            centre_of_voxel([0, 0, 0]),
            10 * FIXED_ONE,
            Mode::Carve,
        )));
        out.push(format!("{r:?}"));
        out.push(format!("{:?}", host.material_at(VoxelCoord([0, 0, 0]))));
        out.push(format!("{:?}", host.material_at(VoxelCoord([0, 0, 11]))));
        out.push(format!("{:?}", host.material_at(VoxelCoord([999, 0, 0]))));
        out.push(format!(
            "{:?}",
            host.raycast(centre_of_voxel([0, 0, 0]), Fixed3([256, 0, 0]), 40 * 256)
        ));
        out.push(format!(
            "{:?}",
            host.count_in_box(VoxelCoord([-12, -12, -12]), VoxelCoord([12, 12, 12]))
        ));
    }

    fn tick(&mut self, _: &mut dyn Host, _: u64, _: &[ClientInput]) -> TickOutput {
        TickOutput::default()
    }
}

#[test]
fn host_queries_answer_from_the_server_world() {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let world = Arc::new(solid3_world());
    let server = Server::new(world.clone(), Box::new(Probe(seen.clone())), 0);

    let reference = solid3_world();
    let r = reference.apply(&EditOp::sphere(
        centre_of_voxel([0, 0, 0]),
        10 * FIXED_ONE,
        Mode::Carve,
    ));
    assert_eq!(r.as_ref().unwrap().changed_from, [(3, 4169)]);
    let r = r.map(OpResult::Edit);
    let hit = reference
        .raycast(centre_of_voxel([0, 0, 0]), [256, 0, 0], 40 * 256)
        .unwrap();
    assert_eq!(hit.unwrap().voxel, VoxelCoord([11, 0, 0]));
    let want = vec![
        format!("{r:?}"),
        format!("{:?}", Some(AIR)),
        format!("{:?}", Some(3u16)),
        format!("{:?}", None::<MaterialId>),
        format!("{hit:?}"),
        format!(
            "{:?}",
            reference
                .count_in_box([-12, -12, -12], [12, 12, 12])
                .unwrap()
        ),
    ];
    assert_eq!(*seen.lock().unwrap(), want);
    // The op init applied is logged at tick 0 under seq 0.
    assert_eq!(server.ops().len(), 1);
    assert_eq!(
        (
            server.ops().entries()[0].tick(),
            server.ops().entries()[0].seq()
        ),
        (0, 0)
    );
    assert_eq!(world.voxel_hash(), reference.voxel_hash());
}

#[test]
fn echo_logic_applies_inputs_in_order_and_resimulates_to_equal_bytes() {
    let (mut server, mut client) = session(Box::new(EchoLogic), 0);
    let carves = seeded_carves(0xEC40, 100);
    let mut events = Vec::new();
    for tick in 0..10u64 {
        for op in &carves[tick as usize * 10..tick as usize * 10 + 10] {
            client.send_input(op_bytes(*op));
        }
        let report = server.tick();
        assert_eq!((report.tick, report.applied, report.inputs), (tick, 10, 10));
        events.extend(client.poll());
    }

    let applied: Vec<(u64, u32, EditOp)> = events
        .iter()
        .filter_map(|e| match e {
            Event::Applied {
                tick,
                seq,
                op: Op::Edit(op),
                ..
            } => Some((*tick, *seq, *op)),
            _ => None,
        })
        .collect();
    let want: Vec<(u64, u32, EditOp)> = carves
        .iter()
        .enumerate()
        .map(|(i, op)| (i as u64 / 10, i as u32, *op))
        .collect();
    assert_eq!(applied, want);

    let inputs = server.inputs().entries();
    assert_eq!(inputs.len(), 100);
    for (i, e) in inputs.iter().enumerate() {
        assert_eq!((e.tick, e.client), (i as u64 / 10, 0));
        assert_eq!(e.bytes, op_bytes(carves[i]));
    }

    let fresh = Arc::new(solid3_world());
    let ops = Server::resimulate(
        fresh.clone(),
        Box::new(EchoLogic),
        0,
        &PrefabRegistry::new(),
        server.ops(),
        server.inputs(),
    );
    assert_eq!(ops.encode(), server.ops().encode());
    assert_eq!(ops.len(), 100);
    assert_eq!(fresh.voxel_hash(), server.world().voxel_hash());
    assert_ne!(fresh.voxel_hash(), solid3_world().voxel_hash());
}

/// Ten inputs per tick for ten ticks; the payloads are ignored by `SeededLogic`.
fn drive_seeded(seed: u64) -> (Server, Vec<Event>) {
    let (mut server, mut client) = session(Box::new(SeededLogic::default()), seed);
    let mut events = Vec::new();
    for tick in 0..10u8 {
        for i in 0..10u8 {
            client.send_input(vec![tick, i]);
        }
        server.tick();
        events.extend(client.poll());
    }
    (server, events)
}

#[test]
fn seeded_logic_resimulates_from_its_seed_and_a_different_seed_differs() {
    let (server, events) = drive_seeded(7);
    assert_eq!(server.ops().len(), 100);
    assert_eq!(server.inputs().len(), 100);

    // One logic event per tick, carrying the state after that tick's three draws per input.
    let logic: Vec<(u64, Vec<u8>)> = events
        .iter()
        .filter_map(|e| match e {
            Event::Logic { tick, bytes } => Some((*tick, bytes.clone())),
            _ => None,
        })
        .collect();
    let mut state = seeded_state(7);
    let want: Vec<(u64, Vec<u8>)> = (0..10)
        .map(|tick| {
            for _ in 0..30 {
                xorshift64(&mut state);
            }
            (tick, state.to_le_bytes().to_vec())
        })
        .collect();
    assert_eq!(logic, want);

    let same = Arc::new(solid3_world());
    let ops7 = Server::resimulate(
        same.clone(),
        Box::new(SeededLogic::default()),
        7,
        &PrefabRegistry::new(),
        server.ops(),
        server.inputs(),
    );
    assert_eq!(ops7.encode(), server.ops().encode());
    assert_eq!(same.voxel_hash(), server.world().voxel_hash());

    let other = Arc::new(solid3_world());
    let ops8 = Server::resimulate(
        other.clone(),
        Box::new(SeededLogic::default()),
        8,
        &PrefabRegistry::new(),
        server.ops(),
        server.inputs(),
    );
    assert_ne!(other.voxel_hash(), server.world().voxel_hash());
    assert_eq!(ops8.len(), 100);
    let differ = ops8
        .entries()
        .iter()
        .zip(server.ops().entries())
        .filter(|(a, b)| a != b)
        .count();
    assert!(differ >= 90, "{differ} of 100 entries differ");
}

#[test]
fn a_refused_host_apply_is_announced_not_logged_and_returned_to_the_logic() {
    let (mut server, mut client) = session(Box::new(EchoLogic), 0);
    let near = EditOp::sphere(centre_of_voxel([0, 0, 0]), 10 * FIXED_ONE, Mode::Carve);
    let far = EditOp::sphere(centre_of_voxel([10000, 0, 0]), 10 * FIXED_ONE, Mode::Carve);
    let after = EditOp::sphere(centre_of_voxel([40, 0, 0]), 10 * FIXED_ONE, Mode::Carve);
    for op in [near, far, after] {
        client.send_input(op_bytes(op));
    }
    let report = server.tick();
    assert_eq!((report.applied, report.refused, report.inputs), (2, 1, 3));

    let events = client.poll();
    assert_eq!(events.len(), 4, "{events:?}");
    assert!(matches!(&events[0], Event::Applied { seq: 0, .. }));
    match &events[1] {
        Event::Refused {
            seq: 1,
            op: Op::Edit(op),
            error: EditError::NotResident(_),
            ..
        } => assert_eq!(*op, far),
        e => panic!("{e:?}"),
    }
    assert!(matches!(&events[2], Event::Applied { seq: 2, .. }));
    // The logic saw the `Err`: its outcome bytes say so.
    assert_eq!(
        events[3],
        Event::Logic {
            tick: 0,
            bytes: vec![ECHO_APPLIED, ECHO_REFUSED, ECHO_APPLIED]
        }
    );
    let logged: Vec<u32> = server.ops().entries().iter().map(LogEntry::seq).collect();
    assert_eq!(logged, [0, 2]);
    assert!(
        server
            .ops()
            .entries()
            .iter()
            .all(|e| e.op() != Some(&Op::Edit(far)))
    );

    // Re-simulation reproduces the gap the refused op left.
    let fresh = Arc::new(solid3_world());
    let ops = Server::resimulate(
        fresh,
        Box::new(EchoLogic),
        0,
        &PrefabRegistry::new(),
        server.ops(),
        server.inputs(),
    );
    assert_eq!(ops.encode(), server.ops().encode());
}

/// Five direct submissions at tick 3 of a ten-tick `EchoLogic` run; the third is refused.
/// The submitted ops overlap that tick's inputs, so applying them after the logic's ops
/// instead of before would leave a different world.
fn submissions_at_tick_3() -> [EditOp; 5] {
    let at = |v: [i32; 3]| centre_of_voxel(v);
    [
        EditOp::sphere(at([0, 0, 0]), 2 * METRE, Mode::Replace { from: 3, to: 5 }),
        EditOp::sphere(at([20, 0, 0]), METRE, Mode::Carve),
        EditOp::sphere(at([10000, 0, 0]), METRE, Mode::Carve),
        EditOp::sphere(at([0, 0, 0]), 2 * METRE, Mode::Replace { from: 5, to: 2 }),
        EditOp::sphere(at([-20, 0, 0]), METRE, Mode::Fill(1)),
    ]
}

#[test]
fn submitted_ops_interleave_with_the_logic_and_resimulate_to_equal_bytes() {
    let (mut server, mut client) = session(Box::new(EchoLogic), 0);
    let mut carves = seeded_carves(0x1A7E, 100);
    // Tick 3's first input carves into what the submitted replaces made.
    carves[30] = EditOp::sphere(centre_of_voxel([0, 0, 0]), METRE, Mode::Carve);
    let submitted = submissions_at_tick_3();
    let mut events = Vec::new();
    for tick in 0..10u64 {
        if tick == 3 {
            let seqs: Vec<u32> = submitted.iter().map(|op| server.submit(*op)).collect();
            assert_eq!(seqs, [30, 31, 32, 33, 34]);
        }
        for op in &carves[tick as usize * 10..tick as usize * 10 + 10] {
            client.send_input(op_bytes(*op));
        }
        let report = server.tick();
        let want = if tick == 3 {
            TickReport {
                tick,
                applied: 14,
                refused: 1,
                inputs: 10,
                logic_error: None,
                restart: None,
            }
        } else {
            TickReport {
                tick,
                applied: 10,
                refused: 0,
                inputs: 10,
                logic_error: None,
                restart: None,
            }
        };
        assert_eq!(report, want);
        events.extend(client.poll());
    }

    // Tick 3 logs the four submitted ops that applied (32 was refused), the boundary at 35,
    // then the logic's ten ops from 36.
    let tick3: Vec<LogEntry> = server
        .ops()
        .entries()
        .iter()
        .filter(|e| e.tick() == 3)
        .cloned()
        .collect();
    let seqs: Vec<(u32, bool)> = tick3.iter().map(|e| (e.seq(), e.op().is_some())).collect();
    let mut want: Vec<(u32, bool)> = vec![(30, true), (31, true), (33, true), (34, true)];
    want.push((35, false));
    want.extend((36..46).map(|s| (s, true)));
    assert_eq!(seqs, want);
    assert_eq!(tick3[4], LogEntry::Boundary { tick: 3, seq: 35 });
    assert_eq!(server.ops().len(), 104);
    assert_eq!(server.ops().entries().len(), 105);

    // The boundary is not announced: 104 applied, 1 refused (seq 32), one logic event a tick.
    let applied: Vec<u32> = events
        .iter()
        .filter_map(|e| match e {
            Event::Applied { seq, .. } => Some(*seq),
            _ => None,
        })
        .collect();
    let mut want: Vec<u32> = (0..30).collect();
    want.extend([30, 31, 33, 34]);
    want.extend(36..106);
    assert_eq!(applied, want);
    let refused: Vec<u32> = events
        .iter()
        .filter_map(|e| match e {
            Event::Refused { seq, .. } => Some(*seq),
            _ => None,
        })
        .collect();
    assert_eq!(refused, [32]);
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Event::Applied { seq: 35, .. }))
    );

    // The logic's carve at the origin removed material 2, which the submitted replace made:
    // the submitted ops ran first.
    let first_logic = events
        .iter()
        .find_map(|e| match e {
            Event::Applied {
                seq: 36, result, ..
            } => Some(result.edit_result().changed_from),
            _ => None,
        })
        .unwrap();
    assert_eq!(first_logic.len(), 1, "{first_logic:?}");
    assert_eq!(first_logic[0].0, 2);

    let fresh = Arc::new(solid3_world());
    let ops = Server::resimulate(
        fresh.clone(),
        Box::new(EchoLogic),
        0,
        &PrefabRegistry::new(),
        server.ops(),
        server.inputs(),
    );
    assert_eq!(ops.encode(), server.ops().encode());
    assert_eq!(fresh.voxel_hash(), server.world().voxel_hash());

    // Without the boundary the submitted ops are lost and the world differs.
    let fresh = Arc::new(solid3_world());
    Server::resimulate(
        fresh.clone(),
        Box::new(EchoLogic),
        0,
        &PrefabRegistry::new(),
        &OpLog::new(),
        server.inputs(),
    );
    assert_ne!(fresh.voxel_hash(), server.world().voxel_hash());
}

/// Carves in `init`, then echoes.
struct InitThenEcho;

impl Logic for InitThenEcho {
    fn init(&mut self, host: &mut dyn Host, _seed: u64) {
        for x in [-40, 40] {
            host.apply(Op::Edit(EditOp::sphere(
                centre_of_voxel([x, 0, 0]),
                METRE,
                Mode::Carve,
            )))
            .unwrap();
        }
    }

    fn tick(&mut self, host: &mut dyn Host, tick: u64, inputs: &[ClientInput]) -> TickOutput {
        EchoLogic.tick(host, tick, inputs)
    }
}

/// At tick 0 the op log holds `init`'s ops before the submitted ones; re-simulation must not
/// apply them twice. A submission whose every op is refused still logs its boundary.
#[test]
fn submissions_at_tick_0_after_init_ops_and_a_refused_only_tick_resimulate() {
    let (mut server, client) = session(Box::new(InitThenEcho), 0);
    assert_eq!(server.submit(submissions_at_tick_3()[0]), 2);
    client.send_input(op_bytes(EditOp::sphere(
        centre_of_voxel([0, 0, 0]),
        METRE,
        Mode::Carve,
    )));
    server.tick();
    // Tick 1: one refused submission and nothing else.
    assert_eq!(server.submit(submissions_at_tick_3()[2]), 5);
    let report = server.tick();
    assert_eq!((report.applied, report.refused), (0, 1));

    let entries = server.ops().entries();
    let shape: Vec<(u64, u32, bool)> = entries
        .iter()
        .map(|e| (e.tick(), e.seq(), e.op().is_some()))
        .collect();
    assert_eq!(
        shape,
        [
            (0, 0, true),
            (0, 1, true),
            (0, 2, true),
            (0, 3, false),
            (0, 4, true),
            (1, 6, false)
        ]
    );

    let fresh = Arc::new(solid3_world());
    let ops = Server::resimulate(
        fresh.clone(),
        Box::new(InitThenEcho),
        0,
        &PrefabRegistry::new(),
        server.ops(),
        server.inputs(),
    );
    assert_eq!(ops.encode(), server.ops().encode());
    assert_eq!(fresh.voxel_hash(), server.world().voxel_hash());
}
