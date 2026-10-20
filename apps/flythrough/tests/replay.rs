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
        Err(EditError::UnknownPrefab(id)) => {
            h.write(&[3]);
            h.write(&id.to_le_bytes());
        }
        Err(e @ EditError::OutOfRange { .. }) => {
            let mut bytes = Vec::new();
            vxe_world::wire::encode_error(&mut bytes, e);
            h.write(&[4]);
            h.write(&bytes);
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
        prefabs: PrefabRegistry::new(),
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
    let mut server = Server::new(world.clone(), Box::new(DigLogic::default()), SEED);
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
                assert!(!result.edit_result().changed_from.is_empty(), "{result:?}");
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
    let mut server = Server::new(world.clone(), Box::new(DigLogic::default()), SEED);
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

/// The level's extent, as `--level` generates it.
fn level_extent() -> ChunkExtent {
    ChunkExtent::new([0, 0, 0], flythrough::cave::LEVEL_CHUNKS)
}

/// The seed of `--level --seed 1`.
const LEVEL_SEED: u64 = 1;

fn level() -> World {
    World::generate(
        level_extent(),
        &CaveSource::new(LEVEL_SEED, level_extent()),
        &cave_materials(),
    )
}

/// The centre of a chamber the session digs in solid rock 100 voxels above and 32 along -x
/// of the level's spawn: the box of 56 voxels around it holds rock and ore only.
const CHAMBER: [i32; 3] = [608, 640, 352];

/// The scripted session, one entry per tick, as the fly-through's keys would send it:
///
/// - tick 0: a dig from inside the rock at [`CHAMBER`], a sphere of radius 40;
/// - tick 1: `P`, the vault stamped where the ray along +y meets the chamber's wall;
/// - ticks 2 and 3: a ledge and the dig under it: a fill of radius 16 where the ray up meets
///   the ceiling (z = 393), then a dig of radius 22 from 10 voxels above that point, inside
///   the rock, which takes the blob's upper part and leaves its lower cap hanging free;
/// - ticks 4 to 23: 20 ticks of falling;
/// - tick 24: `R`;
/// - ticks 25 and 26: the same on the -x wall (entered at x = 568), the second dig;
/// - ticks 27 to 31: 5 ticks.
fn session_ticks() -> Vec<Vec<DigInput>> {
    let k = centre_of_voxel(CHAMBER);
    let up = [0, 0, FIXED_ONE];
    let west = [-FIXED_ONE, 0, 0];
    let fill = |dir| DigInput::Fill {
        origin: k,
        dir,
        radius: 16 * FIXED_ONE,
        material: flythrough::cave::ROCK,
    };
    let cut = |origin, dir| DigInput::Dig {
        origin,
        dir,
        radius: 22 * FIXED_ONE,
    };
    let mut ticks = vec![
        vec![DigInput::Dig {
            origin: k,
            dir: up,
            radius: 40 * FIXED_ONE,
        }],
        vec![DigInput::Stamp {
            origin: k,
            dir: [0, FIXED_ONE, 0],
            prefab: flythrough::cave::vault().id(),
        }],
        vec![fill(up)],
        vec![cut(Fixed3([k.0[0], k.0[1], 403 * FIXED_ONE]), up)],
    ];
    ticks.extend((0..20).map(|_| Vec::new()));
    ticks.push(vec![DigInput::Crumble]);
    ticks.push(vec![fill(west)]);
    ticks.push(vec![cut(Fixed3([558 * FIXED_ONE, k.0[1], k.0[2]]), west)]);
    ticks.extend((0..5).map(|_| Vec::new()));
    ticks
}

/// The session on a fresh level through `logic`, on a server set up as the fly-through's
/// (the vault registered, the default support), with the log file `--save` writes, and the
/// logic's broadcast lines by tick.
fn run_session(logic: Box<dyn vxe_net::Logic>) -> (World, LogFile, Vec<(usize, String)>) {
    let world = Arc::new(level());
    let mut prefabs = PrefabRegistry::new();
    prefabs
        .register(Arc::new(flythrough::cave::vault()))
        .unwrap();
    let mut server = Server::with_prefabs(world.clone(), prefabs, logic, LEVEL_SEED);
    server.set_support(vxe_net::DEFAULT_SUPPORT, &[]);
    let (near, far) = MemChannel::pair();
    server.attach(far);
    let mut client = Client::shared(world.clone(), near);
    let mut lines = Vec::new();
    for (tick, inputs) in session_ticks().iter().enumerate() {
        for input in inputs {
            client.send_input(input.encode());
        }
        let report = server.tick();
        assert_eq!(report.logic_error, None, "tick {tick}");
        for e in client.poll() {
            if let Event::Logic { bytes, .. } = e {
                for l in String::from_utf8(bytes).unwrap().lines() {
                    lines.push((tick, l.to_owned()));
                }
            }
        }
    }
    let file = LogFile {
        seed: LEVEL_SEED,
        source_version: CaveSource::VERSION,
        extent: level_extent(),
        ops: server.ops().clone(),
        inputs: server.inputs().clone(),
        prefabs: server.prefabs().subset(server.ops().prefab_ids()),
    };
    drop(server);
    drop(client);
    let world = Arc::into_inner(world).expect("the server and client are dropped");
    (world, file, lines)
}

/// The world hash after the scripted session on the seed-1 level. Blessed on Linux x86_64,
/// asserted on every arm.
const SESSION_HASH: u64 = 0x422b_7654_317b_145d;

/// A session with a `P` stamp, a ledge that falls, `R` and a ledge removed with rubble: every
/// edit is an op in the log, and the save replays to the same world from its ops and from its
/// inputs.
#[test]
fn a_stamp_and_a_collapse_replay_from_the_save() {
    let (world, file, lines) = run_session(Box::new(DigLogic::default()));
    let live = world.voxel_hash();
    let at = |tick: usize| -> Vec<&str> {
        lines
            .iter()
            .filter(|(t, _)| *t == tick)
            .map(|(_, l)| l.as_str())
            .collect()
    };
    assert_eq!(at(0), ["op carved"]);
    assert_eq!(at(1), ["op stamped"]);
    assert_eq!(at(3), ["op carved", "region 8 1:1248"]);
    for tick in 4..12 {
        assert_eq!(at(tick), ["op dropped"], "tick {tick}");
    }
    assert_eq!(
        at(12),
        ["op dropped", "landed after 8 ticks, fell 64 voxels"]
    );
    assert!((13..24).all(|t| at(t).is_empty()));
    assert_eq!(at(24), ["collapse: remove unsupported regions with rubble"]);
    assert_eq!(at(26), ["op carved", "region 12 1:1602", "op removed"]);
    assert!((27..32).all(|t| at(t).is_empty()));

    // Every edit is in the log: five edits, the stamp, nine drops and the removal.
    let kinds =
        file.ops
            .entries()
            .iter()
            .filter_map(LogEntry::op)
            .fold([0usize; 4], |mut n, op| {
                n[match op {
                    Op::Edit(_) => 0,
                    Op::Stamp { .. } => 1,
                    Op::Drop { .. } => 2,
                    Op::Remove { .. } => 3,
                }] += 1;
                n
            });
    assert_eq!(kinds, [5, 1, 9, 1]);
    // The header carries the vault the stamp names.
    assert_eq!(file.prefabs.len(), 1);
    assert_eq!(file.header_len(), 60 + 4 + 8 + 8_716);

    let path = write(&file, "session.log");
    let (_, by_ops) = replay_file(&path, Replay::Ops, level_extent()).unwrap();
    let (_, by_inputs) = replay_file(&path, Replay::Inputs, level_extent()).unwrap();
    assert_eq!(by_ops, live);
    assert_eq!(by_inputs, live);
    println!("session hash {live:#018x}");
    assert_eq!(live, SESSION_HASH);
}

/// The same session through the logic component: the same save, byte for byte.
#[cfg(feature = "wasm")]
#[test]
fn the_session_through_the_component_saves_the_same_bytes() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(flythrough::DEFAULT_COMPONENT);
    assert!(
        path.is_file(),
        "{} is missing: run scripts/build-guests.sh first",
        path.display()
    );
    let component = flythrough::LogicChoice::Wasm(path)
        .load()
        .expect("the flythrough-logic component loads");
    let (world, file, lines) = run_session(component);
    let (native_world, native_file, native_lines) = run_session(Box::new(DigLogic::default()));
    assert_eq!(file.encode(), native_file.encode());
    assert_eq!(lines, native_lines);
    assert_eq!(world.voxel_hash(), native_world.voxel_hash());
    assert_eq!(world.voxel_hash(), SESSION_HASH);
}
