//! Streaming residency: which chunks a `Streamer` loads and evicts around a focus, that a
//! streamed chunk equals the generated one, and that edits and evictions exclude each other.

mod support;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::TryRecvError;
use std::sync::{Arc, Barrier, mpsc};

use support::*;
use vxe_world::*;

fn config(r_full: i32, r_lod: i32, hysteresis: i32, workers: usize) -> StreamConfig {
    StreamConfig {
        residency: Residency {
            r_full,
            r_lod,
            hysteresis,
        },
        workers,
        persist: PersistConfig::default(),
    }
}

fn solid3_streamer(workers: usize) -> (World, Streamer) {
    (
        World::new(&test_table()),
        Streamer::new(Arc::new(UniformSource(3)), config(2, 2, 1, workers)),
    )
}

/// Counts of each coordinate in `loaded`.
fn tally(loaded: &[(ChunkCoord, Tier)], into: &mut BTreeMap<ChunkCoord, u32>) {
    for &(c, tier) in loaded {
        assert_eq!(tier, Tier::Full);
        *into.entry(c).or_default() += 1;
    }
}

#[test]
fn a_radius_2_residency_loads_33_chunks_then_keeps_42_after_a_move() {
    let (world, mut s) = solid3_streamer(4);
    let mut loads = BTreeMap::new();

    s.update(&world, &[VoxelCoord([0, 0, 0])]);
    let ev = s.wait_idle(&world);
    let first = ball([0, 0, 0], 4);
    assert_eq!(first.len(), 33);
    assert_eq!(world.store().coords(), first);
    assert!(ev.evicted.is_empty());
    tally(&ev.loaded, &mut loads);

    s.update(&world, &[VoxelCoord([192, 0, 0])]);
    let ev = s.wait_idle(&world);
    tally(&ev.loaded, &mut loads);

    let near = ball([3, 0, 0], 4);
    let kept: Vec<ChunkCoord> = first
        .iter()
        .copied()
        .filter(|c| !near.contains(c) && ball([3, 0, 0], 9).contains(c))
        .collect();
    assert_eq!(near.len(), 33);
    assert_eq!(kept.len(), 9);
    let mut expect: Vec<ChunkCoord> = near.iter().chain(&kept).copied().collect();
    expect.sort_unstable();
    assert_eq!(world.store().coords(), expect);
    assert_eq!(world.store().len(), 42);

    let mut evicted = ev.evicted.clone();
    evicted.sort_unstable();
    let far: Vec<ChunkCoord> = first
        .iter()
        .copied()
        .filter(|c| !ball([3, 0, 0], 9).contains(c))
        .collect();
    assert_eq!(far.len(), 22);
    assert_eq!(evicted, far);

    // Every chunk resident now was loaded exactly once over both steps; so was every evicted
    // one, and nothing else was loaded.
    for c in &expect {
        assert_eq!(loads.get(c), Some(&1), "{c:?}");
    }
    for c in &far {
        assert_eq!(loads.get(c), Some(&1), "{c:?}");
    }
    assert_eq!(loads.len(), 42 + 22);
    assert_eq!(s.stats().resident_full, 42);
    assert_eq!(s.stats().in_flight, 0);
}

#[test]
fn streamed_chunks_equal_generated_chunks() {
    let world = World::new(&test_table());
    let mut s = Streamer::new(Arc::new(cave_like_source()), config(1, 1, 0, 4));
    s.update(&world, &[VoxelCoord([10, 20, 30])]);
    s.wait_idle(&world);
    let coords = world.store().coords();
    assert_eq!(coords, ball([0, 0, 0], 1));

    let reference = World::generate(
        ChunkExtent::new([-1, -1, -1], [2, 2, 2]),
        &cave_like_source(),
        &test_table(),
    );
    let mut mixed = 0;
    for &c in &coords {
        assert_eq!(
            chunks_hash(&world, &[c]),
            chunks_hash(&reference, &[c]),
            "{c:?}"
        );
        let chunk = world.store().get(c).unwrap();
        let chunk = chunk.read().unwrap();
        mixed += usize::from(chunk.bricks().iter().any(Brick::is_full));
    }
    // The source is not trivially uniform: most chunks hold Full bricks.
    assert!(mixed >= 4, "{mixed} chunks with Full bricks");
}

#[test]
fn residency_hash_does_not_depend_on_worker_count() {
    let hash = |workers| {
        let world = World::new(&test_table());
        let mut s = Streamer::new(Arc::new(cave_like_source()), config(2, 2, 1, workers));
        s.update(&world, &[VoxelCoord([0, 0, 0])]);
        s.wait_idle(&world);
        assert_eq!(world.store().len(), 33);
        world_hash(&world)
    };
    assert_eq!(hash(1), hash(8));
}

#[test]
fn a_carve_on_an_evicted_chunk_is_refused_and_changes_nothing() {
    let (world, mut s) = solid3_streamer(2);
    s.update(&world, &[VoxelCoord([0, 0, 0])]);
    s.wait_idle(&world);
    s.update(&world, &[VoxelCoord([192, 0, 0])]);
    let ev = s.wait_idle(&world);
    let gone = ChunkCoord([-2, 0, 0]);
    assert!(ev.evicted.contains(&gone));

    let before = world_hash(&world);
    let op = EditOp::sphere(centre_of_voxel([-96, 32, 32]), 4 * FIXED_ONE, Mode::Carve);
    assert_eq!(world.apply(&op), Err(EditError::NotResident(gone)));
    assert_eq!(world_hash(&world), before);
}

/// `apply` takes the store map's read lock: while another thread holds the write lock, an
/// op does not run. The holder evicts the op's chunk before releasing, so an op that checked
/// residency without waiting would have succeeded; one that waited is refused.
#[test]
fn apply_waits_for_the_store_write_lock() {
    let (world, mut s) = solid3_streamer(2);
    s.update(&world, &[VoxelCoord([0, 0, 0])]);
    s.wait_idle(&world);
    let world = Arc::new(world);
    let target = ChunkCoord([0, 0, 0]);

    let barrier = Arc::new(Barrier::new(2));
    let (started_tx, started_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let mut guard = world.store().write_lock();
    let editor = {
        let world = world.clone();
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            barrier.wait();
            started_tx.send(()).unwrap();
            let r = world.apply(&EditOp::sphere(
                centre_of_voxel([32, 32, 32]),
                3 * FIXED_ONE,
                Mode::Carve,
            ));
            done_tx.send(r).unwrap();
        })
    };
    barrier.wait();
    started_rx.recv().unwrap();
    // The editor is running and cannot finish while the guard lives.
    for _ in 0..10_000 {
        std::thread::yield_now();
        assert_eq!(done_rx.try_recv(), Err(TryRecvError::Empty));
    }
    assert!(guard.remove(target).is_some());
    drop(guard);

    let r = done_rx.recv().unwrap();
    editor.join().unwrap();
    assert_eq!(r, Err(EditError::NotResident(target)));
}

/// `apply` keeps the map read lock until its last write: a thread that takes the store's
/// write lock never sees an op half done. The main thread holds the op's second chunk, so the
/// op stalls after writing its first; were the map lock released before the writes, the
/// locker would get in there and find the first chunk carved while the op is still stalled.
#[test]
fn the_store_write_lock_never_sees_half_an_op() {
    let (world, mut s) = solid3_streamer(2);
    s.update(&world, &[VoxelCoord([0, 0, 0])]);
    s.wait_idle(&world);
    let world = Arc::new(world);
    let second = world.store().get(ChunkCoord([1, 0, 0])).unwrap();
    let held = second.write().unwrap();
    let released = Arc::new(AtomicBool::new(false));

    let editor = {
        let world = world.clone();
        std::thread::spawn(move || {
            world
                .apply(&EditOp::sphere(
                    centre_of_voxel([64, 32, 32]),
                    4 * FIXED_ONE,
                    Mode::Carve,
                ))
                .unwrap()
        })
    };
    for _ in 0..10_000 {
        std::thread::yield_now();
    }
    let locker = {
        let (world, released) = (world.clone(), released.clone());
        std::thread::spawn(move || {
            let map = world.store().write_lock();
            let chunk = map.get(ChunkCoord([0, 0, 0])).unwrap();
            let chunk = chunk.read().unwrap();
            let v = VoxelCoord([62, 32, 32]);
            let first = chunk.brick(v.brick_slot()).get(v.index_in_brick());
            (first, released.load(Ordering::SeqCst))
        })
    };
    for _ in 0..10_000 {
        std::thread::yield_now();
    }
    released.store(true, Ordering::SeqCst);
    drop(held);
    let r = editor.join().unwrap();
    assert_eq!(r.touched, [ChunkCoord([0, 0, 0]), ChunkCoord([1, 0, 0])]);
    let (first, was_released) = locker.join().unwrap();
    assert!(
        first == 3 || was_released,
        "the locker saw the first chunk carved while the op was stalled on the second"
    );
}

fn at_chunk(c: [i32; 3]) -> VoxelCoord {
    VoxelCoord(c.map(|v| v * CHUNK_EDGE))
}

/// Air cells of level `n` of `mips`.
fn air_cells(mips: &ChunkMips, n: usize) -> usize {
    let e = 64 >> n;
    (0..e * e * e)
        .filter(|&i| mips.cell(n, [i % e, i / e % e, i / (e * e)]) == AIR)
        .count()
}

#[test]
fn a_lod_ring_holds_26_mip_only_chunks_that_edits_and_queries_cannot_see() {
    let world = World::new(&test_table());
    let mut s = Streamer::new(Arc::new(UniformSource(3)), config(1, 2, 0, 4));
    s.update(&world, &[at_chunk([0, 0, 0])]);
    let ev = s.wait_idle(&world);
    let full = ball([0, 0, 0], 1);
    let ring: Vec<ChunkCoord> = ball([0, 0, 0], 4)
        .into_iter()
        .filter(|c| !full.contains(c))
        .collect();
    assert_eq!((full.len(), ring.len()), (7, 26));
    assert_eq!(world.store().coords(), full);
    assert_eq!(world.lod_coords(), ring);
    let lods = ev
        .loaded
        .iter()
        .filter(|(_, t)| *t == Tier::LodOnly)
        .count();
    assert_eq!(lods, 26);
    assert_eq!((s.stats().resident_full, s.stats().resident_lod), (7, 26));

    for &c in &ring {
        assert_eq!(s.tier(c), Some(Tier::LodOnly));
        let v = c.min_voxel().0.map(|v| v + 5);
        assert_eq!(world.material_at(v), None, "{c:?}");
        let op = EditOp::sphere(centre_of_voxel(v), 2 * FIXED_ONE, Mode::Carve);
        assert_eq!(world.apply(&op), Err(EditError::NotResident(c)));
        assert!(world.count_in_box(v, v).is_empty());
    }
}

#[test]
fn a_carved_chunk_demotes_to_its_mips_and_promotes_back_from_persistence() {
    let world = World::new(&test_table());
    let mut s = Streamer::new(Arc::new(UniformSource(3)), config(1, 2, 0, 4));
    s.update(&world, &[at_chunk([0, 0, 0])]);
    s.wait_idle(&world);
    let carved = ChunkCoord([1, 0, 0]);
    let r = world
        .apply(&EditOp::sphere(
            centre_of_voxel([96, 32, 32]),
            10 * FIXED_ONE,
            Mode::Carve,
        ))
        .unwrap();
    assert_eq!(r.changed_from, [(3, 4169)]);
    assert_eq!(r.touched, [carved]);
    let after_carve = chunks_hash(&world, &[carved]);

    // Squared distance 4 from [-1, 0, 0]: past r_full = 1, within r_lod = 2.
    s.update(&world, &[at_chunk([-1, 0, 0])]);
    let ev = s.wait_idle(&world);
    assert!(ev.demoted.contains(&carved), "{:?}", ev.demoted);
    assert_eq!(s.tier(carved), Some(Tier::LodOnly));
    assert!(!world.store().contains(carved));
    assert!(s.is_persisted(carved));
    let mips = world.lod(carved).expect("demoted to its mips");
    assert_eq!(
        (
            air_cells(&mips, 1),
            air_cells(&mips, 2),
            air_cells(&mips, 3)
        ),
        (410, 32, 0)
    );

    s.update(&world, &[at_chunk([0, 0, 0])]);
    let ev = s.wait_idle(&world);
    assert!(ev.loaded.contains(&(carved, Tier::Full)));
    assert_eq!(s.tier(carved), Some(Tier::Full));
    assert!(world.lod(carved).is_none());
    assert!(!s.is_persisted(carved));
    assert_eq!(chunks_hash(&world, &[carved]), after_carve);
}
