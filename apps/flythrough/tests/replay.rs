//! Saves and replay: a log file of the determinism vector replays to the vector's hash, and a
//! session of `Dig` inputs through `DigLogic` rebuilds to the same world from its ops and
//! from its inputs.

use std::path::PathBuf;
use std::sync::Arc;

use flythrough::cave::{CaveSource, SHELL};
use flythrough::logic::{BENCH_CARVES, DigInput, DigLogic};
use flythrough::{Mode as Replay, ReplayError, cave_materials, replay_file};
use vxe_net::{Client, Event, MemChannel, Server};
use vxe_world::hash::Fnv1a;
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

fn temp_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name)
}

fn write(file: &LogFile, name: &str) -> PathBuf {
    let path = temp_path(name);
    let mut f = std::fs::File::create(&path).unwrap();
    file.write_to(&mut f).unwrap();
    path
}

/// The determinism vector's 24 ops (`tests/determinism.rs`). The copy is held to the
/// original by the op log's blessed hash below.
fn vector_ops(world: &World) -> Vec<EditOp> {
    let c = centre_of_voxel;
    let s = [128, 128, 128];
    let at = |d: [i32; 3]| c([0, 1, 2].map(|k| s[k] + d[k]));
    let tool = |d: [i64; 3], max_t: i64, r: i64, mode| {
        EditOp::tool_along_ray(world, c(s), d, max_t, r, mode)
    };
    vec![
        EditOp::sphere(at([0, 0, 35]), 10 * FIXED_ONE, Mode::Carve),
        EditOp::sphere(at([-40, 5, 0]), 7 * FIXED_ONE + 100, Mode::Carve),
        EditOp::box_(at([20, -10, -10]), at([50, 10, 10]), Mode::Carve),
        EditOp::capsule(at([0, 0, 0]), at([60, 60, 0]), 4 * FIXED_ONE, Mode::Carve),
        tool([256, 0, 0], 100 * 256, 5 * FIXED_ONE, Mode::Carve),
        tool([-200, 90, 31], 120 * 256, 3 * FIXED_ONE, Mode::Carve),
        tool([0, -256, 0], 10 * 256, 2 * FIXED_ONE, Mode::Carve),
        EditOp::sphere(at([0, 0, 0]), 6 * FIXED_ONE, Mode::Fill(1)),
        EditOp::box_(at([-35, -35, -35]), at([-20, -25, -28]), Mode::Fill(2)),
        EditOp::capsule(
            at([-10, 30, 0]),
            at([10, 30, 20]),
            3 * FIXED_ONE,
            Mode::Fill(2),
        ),
        EditOp::sphere(
            at([0, 0, 0]),
            40 * FIXED_ONE,
            Mode::Replace { from: 1, to: 2 },
        ),
        EditOp::box_(
            at([-60, -60, -60]),
            at([60, 60, 60]),
            Mode::Replace { from: 2, to: 1 },
        ),
        EditOp::capsule(
            at([-50, 0, 0]),
            at([50, 0, 0]),
            9 * FIXED_ONE,
            Mode::Replace { from: 0, to: 2 },
        ),
        EditOp::sphere(c([6, 64, 64]), 6 * FIXED_ONE, Mode::Carve),
        EditOp::box_(c([0, 100, 100]), c([8, 110, 110]), Mode::Carve),
        EditOp::sphere(
            c([251, 200, 200]),
            3 * FIXED_ONE,
            Mode::Replace { from: SHELL, to: 1 },
        ),
        EditOp::sphere(c([250, 128, 128]), 10 * FIXED_ONE, Mode::Carve),
        EditOp::sphere(c([3, 3, 3]), 2 * FIXED_ONE, Mode::Carve),
        EditOp::capsule(
            c([10, 10, 10]),
            c([245, 245, 245]),
            FIXED_ONE + 77,
            Mode::Carve,
        ),
        EditOp::box_(
            Fixed3([0; 3]),
            Fixed3([256 * 256; 3]),
            Mode::Replace { from: 2, to: 5 },
        ),
        EditOp::sphere(at([1, -2, 3]), 25 * FIXED_ONE, Mode::Fill(3)),
        EditOp::capsule(at([5, 5, 5]), at([5, 5, 5]), 12 * FIXED_ONE, Mode::Carve),
        EditOp::box_(at([-3, -3, -3]), at([4, 4, 4]), Mode::Fill(0)),
        EditOp::sphere(at([0, 0, 0]), -1, Mode::Carve),
    ]
}

/// As `tests/determinism.rs` hashes one result.
fn hash_result(h: &mut Fnv1a, r: &Result<EditResult, EditError>) {
    match r {
        Ok(r) => {
            h.write(&[1]);
            h.write(&(r.changed_from.len() as u32).to_le_bytes());
            for &(m, n) in &r.changed_from {
                h.write(&m.to_le_bytes());
                h.write(&n.to_le_bytes());
            }
            h.write(&(r.touched.len() as u32).to_le_bytes());
            for c in &r.touched {
                for v in c.0 {
                    h.write(&v.to_le_bytes());
                }
            }
            match r.aabb {
                None => h.write(&[0]),
                Some((lo, hi)) => {
                    h.write(&[1]);
                    for v in lo.0.iter().chain(&hi.0) {
                        h.write(&v.to_le_bytes());
                    }
                }
            }
        }
        Err(EditError::NotResident(c)) => {
            h.write(&[2]);
            for v in c.0 {
                h.write(&v.to_le_bytes());
            }
        }
    }
}

fn file_of(ops: OpLog, inputs: InputLog) -> LogFile {
    LogFile {
        seed: SEED,
        source_version: CaveSource::VERSION,
        extent: extent(),
        ops,
        inputs,
    }
}

#[test]
fn the_determinism_vector_replays_from_a_log_file_to_its_hash() {
    let direct = generate();
    let before = direct.voxel_hash();
    let ops = vector_ops(&direct);
    let mut log = OpLog::new();
    for (tick, op) in ops.iter().enumerate() {
        log.push(tick as u64, *op);
    }
    let mut lh = Fnv1a::default();
    lh.write(&log.encode());
    assert_eq!(lh.finish(), 0x5ac8_737d_db0f_ccec, "the copied ops drifted");
    let results: Vec<_> = ops.iter().map(|op| direct.apply(op)).collect();

    let path = write(&file_of(log, InputLog::new()), "vector.log");
    let (world, hash) = replay_file(&path, Replay::Ops, extent()).unwrap();
    assert_eq!(hash, world.voxel_hash());
    assert_eq!(hash, direct.voxel_hash());

    // The vector's own hash: voxels before, every result, voxels after (from the replay).
    let mut h = Fnv1a::default();
    h.write(&before.to_le_bytes());
    for r in &results {
        hash_result(&mut h, r);
    }
    h.write(&hash.to_le_bytes());
    assert_eq!(h.finish(), 0x2b6b_6a0d_1f9b_8d75);
}

/// Twenty digs from the spawn pocket's centre along ten fixed rays, each ray twice, so the
/// second dig's raycast depends on the first's carve.
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

#[test]
fn twenty_digs_rebuild_to_the_same_hash_from_ops_and_from_inputs() {
    let world = Arc::new(generate());
    let fresh_hash = world.voxel_hash();
    let mut server = Server::new(world.clone(), Box::new(DigLogic), SEED);
    let (near, far) = MemChannel::pair();
    server.attach(far);
    let mut client = Client::shared(world.clone(), near);
    let mut applied = 0;
    for tick in dig_inputs().chunks(4) {
        for input in tick {
            client.send_input(input.encode());
        }
        server.tick();
        for e in client.poll() {
            if let Event::Applied { result, .. } = e {
                assert!(!result.changed_from.is_empty(), "{result:?}");
                applied += 1;
            }
        }
    }
    assert_eq!(applied, 20);
    assert_eq!(server.ops().len(), 20);
    assert_eq!(server.inputs().len(), 20);
    let live = world.voxel_hash();
    assert_ne!(live, fresh_hash);

    let path = write(
        &file_of(server.ops().clone(), server.inputs().clone()),
        "digs.log",
    );
    let (_, by_ops) = replay_file(&path, Replay::Ops, extent()).unwrap();
    let (_, by_inputs) = replay_file(&path, Replay::Inputs, extent()).unwrap();
    assert_eq!(by_ops, live);
    assert_eq!(by_inputs, live);
}

#[test]
fn a_save_of_another_cave_version_is_refused() {
    let mut file = file_of(OpLog::new(), InputLog::new());
    file.source_version = CaveSource::VERSION + 1;
    let path = write(&file, "version.log");
    match replay_file(&path, Replay::Ops, extent()) {
        Err(ReplayError::SourceVersion { file, ours }) => {
            assert_eq!((file, ours), (2, 1));
        }
        r => panic!("{:?}", r.map(|(_, h)| h)),
    }
}

/// A save over another extent than the one the caller replays is refused before anything is
/// generated, so a corrupt header cannot ask for an unbounded world.
#[test]
fn a_save_of_another_extent_is_refused() {
    let mut file = file_of(OpLog::new(), InputLog::new());
    file.extent = ChunkExtent::new([-1, -1, -1], [1, 1, 1]);
    let path = write(&file, "extent.log");
    for mode in [Replay::Ops, Replay::Inputs] {
        match replay_file(&path, mode, extent()) {
            Err(e @ ReplayError::Extent { file }) => {
                assert_eq!(file, ChunkExtent::new([-1, -1, -1], [1, 1, 1]));
                assert_eq!(
                    e.to_string(),
                    "saved over chunks [-1, -1, -1]..[1, 1, 1], not the level's extent"
                );
            }
            r => panic!("{mode:?}: {:?}", r.map(|(_, h)| h)),
        }
    }
}

#[test]
fn inputs_round_trip_and_a_bench_applies_its_carves() {
    let inputs = [
        DigInput::Dig {
            origin: Fixed3([1, -2, 3]),
            dir: [256, -7, 0],
            radius: 5 * FIXED_ONE,
        },
        DigInput::Fill {
            origin: Fixed3([4, 5, -6]),
            dir: [0, 0, -256],
            radius: 3 * FIXED_ONE,
            material: 1,
        },
        DigInput::Bench {
            origin: [128, 128, 128],
            seed: 9,
        },
    ];
    let sizes: Vec<usize> = inputs.iter().map(|i| i.encode().len()).collect();
    assert_eq!(sizes, [57, 59, 33]);
    for i in inputs {
        assert_eq!(DigInput::decode(&i.encode()), Ok(i));
    }

    let world = Arc::new(generate());
    let mut server = Server::new(world.clone(), Box::new(DigLogic), SEED);
    let (near, far) = MemChannel::pair();
    server.attach(far);
    let mut client = Client::shared(world, near);
    client.send_input(inputs[2].encode());
    let report = server.tick();
    assert_eq!(
        (report.applied + report.refused) as usize,
        BENCH_CARVES,
        "{report:?}"
    );
    let events = client.poll();
    let Some(Event::Logic { bytes, .. }) = events.last() else {
        panic!("{events:?}");
    };
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert_eq!(text.lines().count(), BENCH_CARVES);
    assert!(text.lines().all(|l| l == "op bench"));
}
