//! The fly-through's logic, native and as a component (ADR-0010): the same save re-simulates
//! to the same world through either, and the same bench applies the same ops. Needs the
//! `wasm` feature and the component `scripts/build-guests.sh` builds.

use std::path::PathBuf;
use std::sync::Arc;

use flythrough::cave::CaveSource;
use flythrough::logic::{BENCH_CARVES, DigInput, DigLogic};
use flythrough::{LogicChoice, Mode as Replay, cave_materials, replay_file_with};
use vxe_net::{Client, Event, Logic, MemChannel, Server};
use vxe_world::*;

const SEED: u64 = 0x5EED_0001;

fn extent() -> ChunkExtent {
    ChunkExtent::new([0, 0, 0], [4, 4, 4])
}

fn generate() -> World {
    World::generate(
        extent(),
        &CaveSource::new(SEED, extent()),
        &cave_materials(),
    )
}

/// `target/guests/flythrough-logic.wasm` under the workspace root, loaded. Panics, naming the
/// script, when the component has not been built: a missing component fails the test rather
/// than skipping it.
fn component() -> Box<dyn Logic> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(flythrough::DEFAULT_COMPONENT);
    assert!(
        path.is_file(),
        "{} is missing: run scripts/build-guests.sh first",
        path.display()
    );
    LogicChoice::Wasm(path)
        .load()
        .expect("the flythrough-logic component loads")
}

/// The replay fixture of `tests/replay.rs`: twenty digs from the spawn pocket's centre along
/// ten fixed rays, each ray twice, so the second dig's raycast depends on the first's carve.
fn dig_inputs() -> Vec<DigInput> {
    let dirs: [[i64; 3]; 10] = [
        [256, 0, 0],
        [-256, 0, 0],
        [0, 256, 0],
        [0, -256, 0],
        [0, 0, 256],
        [0, 0, -256],
        [181, 181, 0],
        [-148, 148, 148],
        [100, -200, 120],
        [-30, -60, 247],
    ];
    (0..20)
        .map(|i| DigInput::Dig {
            origin: centre_of_voxel([128, 128, 128]),
            dir: dirs[i % 10],
            radius: (3 + (i as i64 % 3)) * FIXED_ONE,
        })
        .collect()
}

/// A server on a fresh cave running `logic`, fed `ticks` of inputs by one shared client;
/// returns the server and every event the client received.
fn drive(logic: Box<dyn Logic>, ticks: &[Vec<DigInput>]) -> (Server, Vec<Event>) {
    let world = Arc::new(generate());
    let mut server = Server::new(world.clone(), logic, SEED);
    assert_eq!(server.init_error(), None);
    let (near, far) = MemChannel::pair();
    server.attach(far);
    let mut client = Client::shared(world, near);
    let mut events = Vec::new();
    for batch in ticks {
        for input in batch {
            client.send_input(input.encode());
        }
        let report = server.tick();
        assert_eq!(report.logic_error, None);
        events.extend(client.poll());
    }
    (server, events)
}

#[test]
fn twenty_digs_replay_to_the_same_hash_natively_and_as_a_component() {
    let ticks: Vec<Vec<DigInput>> = dig_inputs().chunks(4).map(<[_]>::to_vec).collect();
    let (native, native_events) = drive(Box::new(DigLogic::default()), &ticks);
    assert_eq!(native.ops().len(), 20);
    let live = native.world().voxel_hash();
    assert_ne!(live, generate().voxel_hash());

    // The live session through the component requests the same ops.
    let (wasm, wasm_events) = drive(component(), &ticks);
    assert_eq!(wasm.ops().encode(), native.ops().encode());
    assert_eq!(wasm_events, native_events);
    assert_eq!(wasm.world().voxel_hash(), live);

    let file = LogFile {
        seed: SEED,
        source_version: CaveSource::VERSION,
        extent: extent(),
        ops: native.ops().clone(),
        inputs: native.inputs().clone(),
        prefabs: PrefabRegistry::new(),
    };
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("component-digs.log");
    let mut f = std::fs::File::create(&path).unwrap();
    file.write_to(&mut f).unwrap();

    let (_, by_native) = replay_file_with(
        &path,
        Replay::Inputs,
        extent(),
        Box::new(DigLogic::default()),
    )
    .unwrap();
    let (_, by_component) = replay_file_with(&path, Replay::Inputs, extent(), component()).unwrap();
    assert_eq!(by_native, live);
    assert_eq!(by_component, live);
}

#[test]
fn a_bench_applies_the_same_ops_natively_and_as_a_component() {
    let ticks = vec![vec![DigInput::Bench {
        origin: [128, 128, 128],
        seed: 9,
    }]];
    let (native, native_events) = drive(Box::new(DigLogic::default()), &ticks);
    let (wasm, wasm_events) = drive(component(), &ticks);
    assert_eq!(native.ops().len(), BENCH_CARVES);
    assert_eq!(wasm.ops().encode(), native.ops().encode());
    assert_eq!(wasm_events, native_events);
    assert_eq!(wasm.world().voxel_hash(), native.world().voxel_hash());
}

/// A host that carves for any edit, reports one region after it, and answers drops and piles
/// with the results it holds, of whatever kind.
struct Answers {
    drop: OpResult,
    pile: OpResult,
}

impl vxe_net::Host for Answers {
    fn apply(&mut self, op: Op) -> Result<OpResult, EditError> {
        Ok(match op {
            Op::Drop { .. } => self.drop.clone(),
            Op::Pile { .. } => self.pile.clone(),
            _ => OpResult::Edit(EditResult {
                changed_from: vec![(3, 1)],
                touched: Vec::new(),
                aabb: Some((VoxelCoord([0; 3]), VoxelCoord([7; 3]))),
            }),
        })
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
        vec![Region {
            cells: vec![CellCoord([0, 0, 1])],
            counts: vec![(3, 512)],
            aabb: (VoxelCoord([0, 0, 8]), VoxelCoord([7, 7, 15])),
        }]
    }
}

/// The component's lines for a dig tick, then for the tick its region first falls in, on
/// `host`; the component reports no error.
fn component_fall_lines(host: &mut Answers) -> Vec<String> {
    let mut logic = component();
    logic.init(host, 0);
    assert_eq!(logic.take_error(), None);
    let dig = DigInput::Dig {
        origin: Fixed3([0; 3]),
        dir: [256, 0, 0],
        radius: FIXED_ONE,
    };
    let input = vxe_net::ClientInput {
        client: 0,
        bytes: dig.encode(),
    };
    let out = logic.tick(host, 0, &[input]);
    assert_eq!(logic.take_error(), None);
    assert!(String::from_utf8_lossy(&out.broadcast).contains("region "));
    let out = logic.tick(host, 1, &[]);
    assert_eq!(logic.take_error(), None, "the component trapped");
    String::from_utf8(out.broadcast)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// As a component, a drop answered with a pile result and a pile answered with a drop result
/// are refused in the logic's lines, as natively, and the component does not trap.
#[test]
fn the_component_refuses_a_result_of_the_wrong_kind_without_trapping() {
    let mut host = Answers {
        drop: OpResult::Pile(PileResult::default()),
        pile: OpResult::Pile(PileResult::default()),
    };
    assert_eq!(
        component_fall_lines(&mut host),
        [
            "op dropped",
            "drop refused: the host answered a drop with a pile result, after 0 ticks, fell 0 \
             voxels"
        ]
    );
    let mut host = Answers {
        drop: OpResult::Drop(DropResult {
            dropped: 1,
            crushed: vec![(3, 5)],
            ..DropResult::default()
        }),
        pile: OpResult::Drop(DropResult::default()),
    };
    assert_eq!(
        component_fall_lines(&mut host),
        [
            "op dropped",
            "op piled",
            "landed after 1 ticks, fell 1 voxels, crushed 5 voxels, pile refused: the host \
             answered a pile with a drop result"
        ]
    );
}
