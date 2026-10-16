//! Edited-chunk persistence: an evicted edited chunk comes back byte-identical, from RAM or
//! from a spill file, and no `Ok` edit is lost to a racing eviction.

mod support;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};

use support::*;
use vxe_world::*;

const ORIGIN: ChunkCoord = ChunkCoord([0, 0, 0]);

fn config(persist: PersistConfig, workers: usize) -> StreamConfig {
    StreamConfig {
        residency: Residency {
            r_full: 1,
            r_lod: 1,
            hysteresis: 1,
        },
        workers,
        persist,
    }
}

/// A fresh directory under the system temp dir, named from the test and the process.
struct TempDir(PathBuf);

impl TempDir {
    fn new(test: &str) -> Self {
        let p = std::env::temp_dir().join(format!("vxe-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }

    fn files(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(&self.0)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        v.sort();
        v
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn at_chunk(c: [i32; 3]) -> VoxelCoord {
    VoxelCoord(c.map(|v| v * CHUNK_EDGE))
}

/// The 7 chunks of a radius-1 residency around `[0, 0, 0]` on a solid material-3 world, with
/// the radius-10 carve at voxel `[32, 32, 32]` applied. Returns the post-carve hash of
/// `[0, 0, 0]`.
fn carve_at_origin(world: &World, s: &mut Streamer) -> u64 {
    s.update(world, &[at_chunk([0, 0, 0])]);
    s.wait_idle(world);
    assert_eq!(world.store().coords(), ball([0, 0, 0], 1));
    let r = world
        .apply(&EditOp::sphere(
            centre_of_voxel([32, 32, 32]),
            10 * FIXED_ONE,
            Mode::Carve,
        ))
        .unwrap();
    assert_eq!(r.changed_from, [(3, 4169)]);
    assert_eq!(r.touched, [ORIGIN]);
    chunks_hash(world, &[ORIGIN])
}

/// Moves the focus to chunk `[10, 0, 0]` and checks that all 7 chunks were evicted.
fn move_away(world: &World, s: &mut Streamer) -> StreamEvents {
    s.update(world, &[at_chunk([10, 0, 0])]);
    let ev = s.wait_idle(world);
    let mut evicted = ev.evicted.clone();
    evicted.sort_unstable();
    assert_eq!(evicted, ball([0, 0, 0], 1));
    ev
}

#[test]
fn an_evicted_edited_chunk_comes_back_from_ram() {
    let world = World::new(&test_table());
    let mut s = Streamer::new(
        Arc::new(UniformSource(3)),
        config(PersistConfig::default(), 2),
    );
    let carved = carve_at_origin(&world, &mut s);

    move_away(&world, &mut s);
    let st = s.stats();
    assert_eq!(st.persisted_ram_chunks, 1);
    assert!(s.is_persisted(ORIGIN));
    for c in ball([0, 0, 0], 1).into_iter().filter(|&c| c != ORIGIN) {
        assert!(!s.is_persisted(c), "{c:?} was not edited");
    }

    let generated = s.stats().generated_chunks;
    s.update(&world, &[at_chunk([0, 0, 0])]);
    s.wait_idle(&world);
    let st = s.stats();
    assert_eq!(st.restored_chunks, 1);
    assert_eq!(st.generated_chunks - generated, 6);
    assert_eq!(st.persisted_ram_chunks, 0);
    assert_eq!(chunks_hash(&world, &[ORIGIN]), carved);
    assert_eq!(world.material_at([32, 32, 32]), Some(AIR));
    assert_eq!(world.material_at([32, 32, 43]), Some(3));
}

#[test]
fn past_the_cap_an_edited_chunk_spills_to_one_file_and_comes_back() {
    let dir = TempDir::new("spill-roundtrip");
    let world = World::new(&test_table());
    let persist = PersistConfig {
        ram_cap_bytes: 0,
        spill_dir: Some(dir.0.clone()),
        world_key: 1,
    };
    let mut s = Streamer::new(Arc::new(UniformSource(3)), config(persist, 2));
    let carved = carve_at_origin(&world, &mut s);

    move_away(&world, &mut s);
    assert_eq!(dir.files().len(), 1);
    let st = s.stats();
    assert_eq!(st.persisted_ram_bytes, 0);
    assert_eq!(st.persisted_ram_chunks, 0);
    assert_eq!(st.spilled_chunks, 1);
    assert!(!st.persist_over_cap);

    s.update(&world, &[at_chunk([0, 0, 0])]);
    let ev = s.wait_idle(&world);
    assert!(ev.errors.is_empty(), "{:?}", ev.errors);
    assert!(dir.files().is_empty());
    assert_eq!(s.stats().spilled_chunks, 0);
    assert_eq!(chunks_hash(&world, &[ORIGIN]), carved);
}

#[test]
fn without_a_spill_dir_the_chunk_stays_in_ram_over_the_cap() {
    let world = World::new(&test_table());
    let persist = PersistConfig {
        ram_cap_bytes: 0,
        spill_dir: None,
        world_key: 1,
    };
    let mut s = Streamer::new(Arc::new(UniformSource(3)), config(persist, 2));
    carve_at_origin(&world, &mut s);
    move_away(&world, &mut s);
    let st = s.stats();
    assert_eq!(st.persisted_ram_chunks, 1);
    assert!(st.persisted_ram_bytes > 0);
    assert!(st.persist_over_cap);
}

#[test]
fn a_spill_file_round_trips_byte_exactly_and_checks_its_key() {
    let world = solid3_world();
    world
        .apply(&EditOp::sphere(
            centre_of_voxel([32, 32, 32]),
            10 * FIXED_ONE,
            Mode::Carve,
        ))
        .unwrap();
    let chunk = world.store().get(ORIGIN).unwrap();
    let chunk = chunk.read().unwrap();
    let bytes = encode_spill(ORIGIN, &chunk, 1);
    let (coord, back) = decode_spill(&bytes, 1).unwrap();
    assert_eq!(coord, ORIGIN);
    assert_eq!(back.bricks(), chunk.bricks());
    assert!(back.is_edited());
    assert_eq!(encode_spill(coord, &back, 1), bytes);

    assert_eq!(
        decode_spill(&bytes, 2).err(),
        Some(SpillError::WorldMismatch)
    );
}

#[test]
fn a_corrupt_spill_file_is_reported_and_its_chunk_stays_out() {
    let dir = TempDir::new("spill-corrupt");
    let world = World::new(&test_table());
    let persist = PersistConfig {
        ram_cap_bytes: 0,
        spill_dir: Some(dir.0.clone()),
        world_key: 1,
    };
    let mut s = Streamer::new(Arc::new(UniformSource(3)), config(persist, 2));
    carve_at_origin(&world, &mut s);
    move_away(&world, &mut s);

    let files = dir.files();
    assert_eq!(files.len(), 1);
    let mut bytes = std::fs::read(&files[0]).unwrap();
    // Header is 32 bytes and the trailer 8: the middle byte is brick payload.
    let i = bytes.len() / 2;
    assert!(i > 32 && i < bytes.len() - 8);
    bytes[i] ^= 0x40;
    std::fs::write(&files[0], &bytes).unwrap();

    s.update(&world, &[at_chunk([0, 0, 0])]);
    let ev = s.wait_idle(&world);
    assert_eq!(
        ev.errors,
        [StreamError::Spill {
            coord: ORIGIN,
            kind: SpillError::Corrupt
        }]
    );
    assert!(!world.store().contains(ORIGIN));
    let others: Vec<ChunkCoord> = ball([0, 0, 0], 1)
        .into_iter()
        .filter(|&c| c != ORIGIN)
        .collect();
    assert_eq!(world.store().coords(), others);
    // The file is left for the caller; nothing regenerated over it.
    assert_eq!(dir.files(), files);
}

/// One thread carves while another moves the focus back and forth; at the end the 7 chunks
/// around `[0, 0, 0]` equal a bounded world with exactly the `Ok` ops applied in issue order.
#[test]
fn no_ok_edit_is_lost_to_a_racing_eviction() {
    const OPS: usize = 2000;
    const ROUND: usize = 100;
    let mut rng = SplitMix64(0xACE0_0002);
    let ops: Vec<EditOp> = (0..OPS)
        .map(|_| {
            let v = [rng.range(-64, 191), rng.range(0, 63), rng.range(0, 63)];
            EditOp::sphere(
                Fixed3(v.map(|c| c * FIXED_ONE + FIXED_HALF)),
                2 * FIXED_ONE,
                Mode::Carve,
            )
        })
        .collect();

    let world = Arc::new(World::new(&test_table()));
    let mut s = Streamer::new(
        Arc::new(UniformSource(3)),
        config(PersistConfig::default(), 4),
    );
    let done_ops = Arc::new(AtomicUsize::new(0));
    let finished = Arc::new(AtomicBool::new(false));
    let barrier = Arc::new(Barrier::new(2));

    let editor = {
        let (world, done_ops, finished, barrier) = (
            world.clone(),
            done_ops.clone(),
            finished.clone(),
            barrier.clone(),
        );
        let ops = ops.clone();
        std::thread::spawn(move || {
            let mut ok = Vec::new();
            for round in ops.chunks(ROUND) {
                barrier.wait();
                for op in round {
                    if world.apply(op).is_ok() {
                        ok.push(*op);
                    }
                    done_ops.fetch_add(1, Ordering::SeqCst);
                }
                barrier.wait();
            }
            finished.store(true, Ordering::SeqCst);
            ok
        })
    };

    // Each round starts settled at one focus. Once half the round's ops have run, the focus
    // moves to the other end and the rest of the round races the evictions and loads.
    let foci = [at_chunk([0, 0, 0]), at_chunk([4, 0, 0])];
    for r in 0..OPS / ROUND {
        s.update(&world, &[foci[r % 2]]);
        s.wait_idle(&world);
        barrier.wait();
        let half = r * ROUND + ROUND / 2;
        while done_ops.load(Ordering::SeqCst) < half {
            std::thread::yield_now();
        }
        s.update(&world, &[foci[(r + 1) % 2]]);
        while done_ops.load(Ordering::SeqCst) < (r + 1) * ROUND {
            s.poll(&world);
            std::thread::yield_now();
        }
        barrier.wait();
    }
    let ok = editor.join().unwrap();
    assert!(finished.load(Ordering::SeqCst));
    s.update(&world, &[foci[0]]);
    s.wait_idle(&world);

    let seven = ball([0, 0, 0], 1);
    assert_eq!(world.store().coords(), seven);
    // Even rounds run their first half at focus [0, 0, 0]; odd rounds their first half at
    // [4, 0, 0], where every op is refused.
    assert!(ok.len() >= 10 * ROUND / 4, "{} ops succeeded", ok.len());
    assert!(ok.len() < OPS, "every op succeeded");

    let reference = World::generate(
        ChunkExtent::new([-1, -1, -1], [2, 2, 2]),
        &UniformSource(3),
        &test_table(),
    );
    for op in &ok {
        reference.apply(op).unwrap();
    }
    assert_eq!(chunks_hash(&world, &seven), chunks_hash(&reference, &seven));
}
