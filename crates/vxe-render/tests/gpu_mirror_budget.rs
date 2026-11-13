//! The GPU mirror's budgets: a world past the reservation fails untouched, the pressure follows
//! whichever of the units and the records runs out, a sync short of pages defers whole chunks,
//! and a sync stops at its time budget keeping what it sized.

mod support;

use support::*;

use std::sync::Arc;
use vxe_render::{GpuWorld, MirrorConfig, SyncPoint, Window, wgpu};
use vxe_render::{
    internals::FULL_BIT, internals::PAGE_BYTES, internals::PLACEHOLDER_BYTES,
    internals::RECORD_MASK, internals::RECORD_WORDS, internals::bin_of,
};
use vxe_world::packed::PackedClass;
use vxe_world::*;

/// The same 36 chunks under a budget of four pages and their records, on uncapped shards:
/// the budget is the limit; the first sync uploads 10 chunks, and the next, with nothing
/// freed, writes nothing.
#[test]
fn a_world_past_the_budget_fails_and_leaves_the_mirror_untouched() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [4, 3, 3]),
        &Checker,
        &numbered_table(2),
    );
    let budget = pages_budget(4, 36);
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            budget_bytes: budget,
            ..MirrorConfig::new(Window::Fixed(world.extent()))
        },
    );
    assert!(gpu.shard_layout().capacity() > 4 * 4096);
    assert_eq!(gpu.reserved_units(), 4 * 4096);
    assert_eq!(gpu.reserved_records(), 36);
    assert!(gpu.allocated_bytes() <= budget);
    assert_eq!(gpu.allocated_bytes(), gpu.shard_layout().reserved_bytes());
    partial_then_empty(
        &mut gpu,
        &world,
        &device,
        &queue,
        10,
        26,
        vxe_render::MirrorError::PoolTooLarge {
            needed: 5 * PAGE_BYTES,
            limit: 4 * PAGE_BYTES,
        },
    );

    // Exactly the reservation: 8 chunks of 512 `Full` bricks take 4096 slots, three pages of
    // 1365 and one slot of a fourth.
    let fits = World::generate(
        ChunkExtent::new([0, 0, 0], [4, 2, 1]),
        &Checker,
        &numbered_table(2),
    );
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            budget_bytes: pages_budget(4, 8),
            ..MirrorConfig::new(Window::Fixed(fits.extent()))
        },
    );
    let st = gpu.sync_now(&fits, &device, &queue).unwrap();
    assert_eq!(st.bricks, 4096);
    assert_eq!(gpu.pool_units(), gpu.reserved_units());
    assert_eq!(gpu.bin_stats()[bin_of(3)].spare, 4 * 1365 - 4096);
}

/// Pressure on units: a fixed window whose `Full` bricks fill more than 0.9 of a six-page
/// reservation, with records far from full.
#[test]
fn pressure_follows_the_units_when_they_run_out() {
    let Some((device, queue)) = device() else {
        return;
    };
    // 15 chunks of 512 `Full` bricks of 3 units: 23 040 of 24 576 units.
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [5, 3, 1]),
        &Checker,
        &numbered_table(2),
    );
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            budget_bytes: pages_budget(6, 40),
            record_chunks: Some(40),
            ..MirrorConfig::new(Window::Fixed(world.extent()))
        },
    );
    assert_eq!(
        (gpu.reserved_units(), gpu.reserved_records()),
        (6 * 4096, 40)
    );
    gpu.sync_now(&world, &device, &queue).unwrap();
    let (units, records) = pressure_terms(&gpu);
    assert_eq!(units, 23_040.0 / 24_576.0);
    assert!(units > 0.9 && records < 0.5, "{units} {records}");
    assert_pressure(&gpu);
    assert!((f64::from(gpu.pressure()) - units).abs() < 1e-6);
}

/// Pressure on records: uniform chunks take no slot, and 37 of 40 records read past 0.9
/// with units at 0. A mirror given `record_chunks` reserves exactly that many, and its
/// reservation is still the budget rounded down.
#[test]
fn pressure_follows_the_records_when_they_run_out() {
    let Some((device, queue)) = device() else {
        return;
    };
    let mut t = MaterialTable::new();
    let rock = t.push(Material::named("rock"));
    let world = World::generate(
        ChunkExtent::new([0, 0, 0], [37, 1, 1]),
        &UniformSource(rock),
        &t,
    );
    let budget = pages_budget(1, 40);
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            budget_bytes: budget,
            record_chunks: Some(40),
            ..MirrorConfig::new(Window::Fixed(world.extent()))
        },
    );
    assert_eq!(gpu.reserved_records(), 40);
    assert_eq!(gpu.allocated_bytes(), gpu.shard_layout().reserved_bytes());
    assert!(gpu.allocated_bytes() <= budget);
    assert!(budget - gpu.allocated_bytes() < PAGE_BYTES + PLACEHOLDER_BYTES);
    gpu.sync_now(&world, &device, &queue).unwrap();
    let (units, records) = pressure_terms(&gpu);
    assert_eq!(units, 0.0);
    assert_eq!(records, 37.0 / 40.0);
    assert!(records > 0.9);
    assert!((f64::from(gpu.pressure()) - records).abs() < 1e-6);
}

/// A selection whose net need exceeds the reservation writes what fits net of its frees: ten
/// `Checker` chunks on four pages of 3-unit slots (340 spare), one of them demoted (freeing
/// 512) while two new full chunks need 1024. The demotion and the nearer new chunk upload, the
/// farther waits; the next sync, its nearest chunk alone not fitting and no demotion behind
/// it, is empty and writes nothing.
#[test]
fn a_selection_past_the_reservation_net_of_its_frees_still_fails_untouched() {
    let Some((device, queue)) = device() else {
        return;
    };
    let (world, chunks, _) = tier_world();
    for x in 0..10 {
        to_full(&world, x, &chunks);
    }
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            budget_bytes: pages_budget(4, 12),
            ..MirrorConfig::new(Window::Fixed(world.extent()))
        },
    );
    gpu.set_focus(VoxelCoord([1, 1, 1]));
    gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(gpu.bin_stats()[bin_of(3)].spare, 4 * 1365 - 5120);
    let own = Arc::new(ChunkMips::build(&chunks[0].read().unwrap()));
    to_lod(&world, 0, &own);
    for x in 10..12 {
        to_full(&world, x, &chunks);
    }
    let st = gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(
        (st.chunks, st.deferred_chunks, st.pending_chunks),
        (2, 1, 1)
    );
    assert!(gpu.is_lod(ChunkCoord([0, 0, 0])));
    assert!(gpu.is_mirrored(ChunkCoord([10, 0, 0])));
    assert!(!gpu.is_mirrored(ChunkCoord([11, 0, 0])));
    assert_eq!(gpu.live_bricks(), 5120);
    let (bytes, mirrored, live) = (gpu.bytes_written(), gpu.mirrored_chunks(), gpu.live_units());
    let err = gpu.sync_now(&world, &device, &queue).unwrap_err();
    assert_eq!(
        err,
        vxe_render::MirrorError::PoolTooLarge {
            needed: 5 * PAGE_BYTES,
            limit: 4 * PAGE_BYTES,
        }
    );
    assert_eq!(gpu.bytes_written(), bytes);
    assert_eq!(gpu.mirrored_chunks(), mirrored);
    assert_eq!(gpu.live_units(), live);
    assert_eq!(gpu.pending_chunks(), 1);
}

/// Record words and table entry the GPU holds for `coord`.
fn record_and_entry(rb: &Readback, gpu: &GpuWorld, coord: ChunkCoord) -> (Vec<u32>, u32) {
    let t = rb.table[gpu.table_index(coord) as usize];
    let rec = (t & RECORD_MASK) as usize;
    (rb.records[rec * RECORD_WORDS..][..RECORD_WORDS].to_vec(), t)
}

/// Five materials in turn, `1 + (x + 2 y + 3 z) % 5`: every brick four planes, 6 units, 682
/// slots a page.
struct Five;

impl WorldSource for Five {
    fn chunk(&self, _: ChunkCoord, out: &mut Chunk) {
        for slot in 0..CHUNK_BRICKS {
            out.set_brick(
                slot,
                Brick::from_fn(|l| 1 + ((l[0] + 2 * l[1] + 3 * l[2]) % 5) as u16),
            );
        }
    }
}

/// One page short: eight LOD-only chunks (their `Mixed` mips in 2- and 3-unit pages) are
/// promoted, nearest first, to `Five` chunks onto a reservation of those pages and four more.
/// Five chunks' 2560 bricks of 6 units take the four pages (682 a page); the sixth would need a
/// fifth. The sync returns `Ok`: the five nearest read back to their voxels, the other three
/// are deferred and pending, and nothing of theirs was written (their LOD records and table
/// entries read back as before, still decoding to their mips; their chunks keep their dirty
/// bits). After three demotions free their slots, the next sync uploads them.
#[test]
fn one_page_short_uploads_the_nearest_and_defers_the_rest_untouched() {
    let Some((device, queue)) = device() else {
        return;
    };
    let (world, checker, mixed) = tier_world();
    let five = World::generate(world.extent(), &Five, &numbered_table(5));
    let chunks: Vec<SharedChunk> = (0..12)
        .map(|x| five.store().remove(ChunkCoord([x, 0, 0])).unwrap())
        .collect();
    for x in 0..8 {
        world.insert_lod(ChunkCoord([x, 0, 0]), mixed[x as usize].clone());
    }
    let mip_pages = pages_taken(&device, &queue, &world, false);
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            budget_bytes: pages_budget(u64::from(mip_pages) + 4, 12),
            ..MirrorConfig::new(Window::Fixed(world.extent()))
        },
    );
    gpu.set_focus(VoxelCoord([1, 1, 1]));
    gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(gpu.pages_issued(), mip_pages);
    let mips_before = gpu.live_bricks();
    let rb = Readback::new(&device, &queue, &gpu);
    let before: Vec<_> = (5..8)
        .map(|x| record_and_entry(&rb, &gpu, ChunkCoord([x, 0, 0])))
        .collect();

    for x in 0..8 {
        world.remove_lod(ChunkCoord([x, 0, 0]));
        to_full(&world, x, &chunks);
    }
    let st = gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(st.chunks, 5);
    assert_eq!(st.deferred_chunks, 3);
    assert_eq!(st.pending_chunks, 3);
    assert_eq!(gpu.bin_stats()[bin_of(6)].live, 5 * 512);
    let freed: u32 = (0..5).map(|x| mixed[x].full_bricks() as u32).sum();
    assert_eq!(gpu.live_bricks(), mips_before - freed + 5 * 512);
    let rb = Readback::new(&device, &queue, &gpu);
    for x in 0..5 {
        rb.assert_chunk(&gpu, &world, ChunkCoord([x, 0, 0]));
    }
    for (i, x) in (5..8).enumerate() {
        let c = ChunkCoord([x, 0, 0]);
        assert!(gpu.is_lod(c), "{c:?} keeps its LOD record");
        assert_eq!(record_and_entry(&rb, &gpu, c), before[i], "{c:?}");
        // The world holds it full now; its record still decodes to the mips it was built from.
        let lod_world = World::new(&mixed_table());
        lod_world.insert_lod(c, mixed[x as usize].clone());
        rb.assert_lod(&gpu, &lod_world, c);
        assert!(chunks[x as usize].read().unwrap().any_dirty(), "{c:?}");
    }

    // Demote the three nearest (to a checkerboard's uniform mips): their slots free.
    for x in 0..3 {
        let own = Arc::new(ChunkMips::build(&checker[x as usize].read().unwrap()));
        to_lod(&world, x, &own);
    }
    let st = gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(st.deferred_chunks, 0);
    assert_eq!(st.pending_chunks, 0);
    let rb = Readback::new(&device, &queue, &gpu);
    for x in 3..8 {
        rb.assert_chunk(&gpu, &world, ChunkCoord([x, 0, 0]));
    }
    for x in 0..3 {
        rb.assert_lod(&gpu, &world, ChunkCoord([x, 0, 0]));
    }
}

/// A chunk whose bricks move bins past the pool is deferred whole: on a two-page pool (a page
/// of 2-unit slots holding a `Floor` chunk, a page of 3-unit slots holding two `Checker`
/// chunks), two new materials above the floor turn the floor's 64 bricks into 4-unit bricks,
/// for which no page is left. The sync is empty; the floor chunk's record and table entry
/// read back as before and still decode to the voxels it had, and it keeps its dirty bits.
#[test]
fn a_chunk_whose_bricks_move_past_the_pool_is_deferred_whole() {
    let Some((device, queue)) = device() else {
        return;
    };
    let extent = ChunkExtent::new([0, 0, 0], [3, 1, 1]);
    let world = World::generate(extent, &FloorAndChecker, &numbered_table(3));
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            budget_bytes: pages_budget(2, 3),
            ..MirrorConfig::new(Window::Fixed(extent))
        },
    );
    let c = ChunkCoord([0, 0, 0]);
    gpu.set_focus(VoxelCoord([1, 1, 1]));
    gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(gpu.pages_issued(), 2);
    let live = gpu.live_units();
    assert_eq!(live, 64 * 2 + 1024 * 3);
    let snapshot = |gpu: &GpuWorld| {
        let rb = Readback::new(&device, &queue, gpu);
        let voxels: Vec<Vec<u16>> = (0..CHUNK_BRICKS)
            .filter_map(|i| gpu.entry(c, i).filter(|e| e & FULL_BIT != 0))
            .map(|e| rb.voxels(e))
            .collect();
        (record_and_entry(&rb, gpu, c), voxels)
    };
    let before = snapshot(&gpu);

    for (m, z) in [(2, 4), (3, 5)] {
        let r = world
            .apply(&EditOp::box_(
                centre_of_voxel([0, 0, z]),
                centre_of_voxel([64, 64, z + 1]),
                Mode::Fill(m),
            ))
            .unwrap();
        assert_eq!(r.changed_from, [(0, 64 * 64)]);
    }
    let written = gpu.bytes_written();
    let err = gpu.sync_now(&world, &device, &queue).unwrap_err();
    assert_eq!(gpu.bytes_written(), written, "an empty sync writes nothing");
    assert_eq!(
        err,
        vxe_render::MirrorError::PoolTooLarge {
            needed: 3 * PAGE_BYTES,
            limit: 2 * PAGE_BYTES,
        }
    );
    assert_eq!(snapshot(&gpu), before);
    assert_eq!(gpu.pending_chunks(), 1);
    assert_eq!(gpu.live_units(), live);
    let chunk = world.store().get(c).unwrap();
    assert!(
        chunk.read().unwrap().any_dirty(),
        "the deferred chunk keeps its dirty bits"
    );
}

/// A page the sync allocates into is not credited free by a later demotion's frees. Two pages
/// of 3-unit slots: x = 0 LOD-only with uniform mips, x = 1..4 full (1536 bricks; page 1 holds
/// 171 of x = 3's, its cursor short of the page end). One sync promotes x = 0 to a chunk of 64
/// two-material bricks (onto page 1's cursor), demotes x = 3 (freeing its slots, page 1's 171
/// among them) and adds a LOD-only x = 4 whose `Mixed` mips need a page of 2-unit slots. The
/// bin took spare slots first, so the demotion's frees are credited as slots, not as an
/// emptied page 1: x = 4 is deferred whole, and the sync is `Ok`.
#[test]
fn a_page_the_sync_allocated_into_is_not_credited_free() {
    let Some((device, queue)) = device() else {
        return;
    };
    let (world, chunks, mixed) = tier_world();
    let own: Vec<Arc<ChunkMips>> = chunks
        .iter()
        .map(|c| Arc::new(ChunkMips::build(&c.read().unwrap())))
        .collect();
    world.insert_lod(ChunkCoord([0, 0, 0]), own[0].clone());
    for x in 1..4 {
        to_full(&world, x, &chunks);
    }
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            budget_bytes: pages_budget(2, 12),
            ..MirrorConfig::new(Window::Fixed(world.extent()))
        },
    );
    gpu.set_focus(VoxelCoord([1, 1, 1]));
    gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(gpu.live_bricks(), 3 * 512);
    assert_eq!(gpu.pages_issued(), 2);

    // Material 1 below z = 2 and 2 at z = 2: 64 two-material bricks.
    struct TwoLayer;
    impl WorldSource for TwoLayer {
        fn chunk(&self, _: ChunkCoord, out: &mut Chunk) {
            for slot in 0..CHUNK_BRICKS {
                let b = brick_of_slot(slot);
                out.set_brick(
                    slot,
                    Brick::from_fn(|l| match b[2] * BRICK_EDGE + l[2] {
                        0 | 1 => 1,
                        2 => 2,
                        _ => AIR,
                    }),
                );
            }
        }
    }
    let two = World::generate(
        ChunkExtent::new([0, 0, 0], [1, 1, 1]),
        &TwoLayer,
        &numbered_table(2),
    );
    world.remove_lod(ChunkCoord([0, 0, 0]));
    world.store().write_lock().insert(
        ChunkCoord([0, 0, 0]),
        two.store().get(ChunkCoord([0, 0, 0])).unwrap(),
    );
    to_lod(&world, 3, &own[3]);
    world.insert_lod(ChunkCoord([4, 0, 0]), mixed[4].clone());
    let small_mips = (1..=MIP_LEVELS)
        .flat_map(|n| mixed[4].level(n))
        .filter(|b| b.class() == Some(PackedClass::Mask))
        .count();
    assert!(small_mips > 0, "x = 4's mips need 2-unit slots");
    let st = gpu
        .sync_now(&world, &device, &queue)
        .expect("x = 4 waits; the rest uploads");
    assert_eq!((st.chunks, st.deferred_chunks), (2, 1));
    assert!(!gpu.is_mirrored(ChunkCoord([4, 0, 0])));
    assert_eq!(gpu.pending_chunks(), 1);
    assert_eq!(gpu.live_bricks(), 2 * 512 + 64);
    let rb = Readback::new(&device, &queue, &gpu);
    rb.assert_chunk(&gpu, &world, ChunkCoord([0, 0, 0]));
    rb.assert_lod(&gpu, &world, ChunkCoord([3, 0, 0]));
}

/// The 64 chunks of an 8 x 8 x 1 floor, each holding 64 `Full` bricks, mirrored under a time
/// budget of 8 steps of a clock that advances one step per uploaded chunk: the first sync sizes
/// all 64, uploads exactly 8 and leaves 56 pending as `time_deferred_chunks`; the second sizes
/// none (56 kept sizes) and uploads the 9th to the 16th of the unbudgeted sync's order (its
/// record indices 8 to 15). With the budget unset the same world uploads all 64 in one sync.
/// Once the budgeted mirror has drained in 8 syncs, every chunk has the same record index in
/// both, and the pool shards and the records read back byte-identical: the slot words do not
/// depend on how the uploads were split.
#[test]
fn a_sync_stops_at_its_time_budget() {
    let Some((device, queue)) = device() else {
        return;
    };
    let mut t = MaterialTable::new();
    t.push(Material::named("floor"));
    let extent = ChunkExtent::new([0, 0, 0], [8, 8, 1]);
    let fresh = || World::generate(extent, &Floor, &t);
    let step = std::time::Duration::from_micros(250);

    let whole_world = fresh();
    let mut whole = GpuWorld::with_config(&device, small(Window::Fixed(extent)));
    let s = whole.sync_now(&whole_world, &device, &queue).unwrap();
    assert_eq!(s.chunks, 64);
    assert_eq!(s.time_deferred_chunks, 0);
    assert_eq!(s.pending_chunks, 0);
    let coords = whole_world.store().coords();
    assert_eq!(coords.len(), 64);
    let order = |gpu: &GpuWorld| -> Vec<Option<u32>> {
        coords.iter().map(|&c| gpu.record_index(c)).collect()
    };
    let whole_order = order(&whole);
    let first_n = |n: u32| -> Vec<bool> {
        whole_order
            .iter()
            .map(|r| r.expect("mirrored") < n)
            .collect()
    };

    let split_world = fresh();
    let mut split = GpuWorld::with_config(
        &device,
        MirrorConfig {
            sync_budget_time: Some(8 * step),
            ..small(Window::Fixed(extent))
        },
    );
    split.set_sync_clock(stepping_clock(step, SyncPoint::Uploaded));
    let s = split.sync_now(&split_world, &device, &queue).unwrap();
    assert_eq!(s.chunks, 8, "8 chunks in 8 steps");
    assert_eq!(s.time_deferred_chunks, 56);
    assert_eq!(s.deferred_chunks, 0, "none deferred for space");
    assert_eq!(s.pending_chunks, 56);
    assert_eq!((s.sized_chunks, s.kept_sizes), (64, 0));
    let held: Vec<bool> = order(&split).iter().map(Option::is_some).collect();
    assert_eq!(
        held,
        first_n(8),
        "the nearest 8, as the whole sync ordered them"
    );
    let s = split.sync_now(&split_world, &device, &queue).unwrap();
    assert_eq!(s.chunks, 8);
    assert_eq!(s.time_deferred_chunks, 48);
    assert_eq!(
        (s.sized_chunks, s.kept_sizes),
        (0, 56),
        "the deferred chunks are not sized again"
    );
    let held: Vec<bool> = order(&split).iter().map(Option::is_some).collect();
    assert_eq!(held, first_n(16), "the second sync starts with the 9th");
    let mut syncs = 2;
    while split.pending_chunks() > 0 {
        split.sync_now(&split_world, &device, &queue).unwrap();
        syncs += 1;
    }
    assert_eq!(syncs, 8);
    assert_eq!(order(&split), whole_order);
    let read = |b: &wgpu::Buffer| vxe_render::debug::read_buffer(&device, &queue, b, 0, b.size());
    for k in 0..4 {
        assert!(
            read(split.pool_shard(k)) == read(whole.pool_shard(k)),
            "pool shard {k} differs"
        );
    }
    assert!(read(split.records()) == read(whole.records()));
}

/// The floor of [`a_sync_stops_at_its_time_budget`] under the same budget of 8 steps, with a
/// clock that advances one step per chunk selection sizes and holds while chunks upload: the
/// first sync sizes 8 chunks, stops sizing once the 8th spends the budget, writes the first
/// of them (a sync writes at least one chunk) and leaves the 7 others sized and pending
/// (`time_deferred_chunks`), the 56 unsized behind them. Every later sync takes the kept
/// sizes without sizing them again and sizes at most 8 new chunks, so the 64 chunks are sized
/// 64 times over the whole drain; the sync after the last sizing writes the 56 left. The
/// mirror's records and pool read back byte-identical to the unbudgeted sync's.
#[test]
fn a_sync_spent_while_sizing_keeps_what_it_sized() {
    let Some((device, queue)) = device() else {
        return;
    };
    let mut t = MaterialTable::new();
    t.push(Material::named("floor"));
    let extent = ChunkExtent::new([0, 0, 0], [8, 8, 1]);
    let fresh = || World::generate(extent, &Floor, &t);
    let step = std::time::Duration::from_micros(250);

    let whole_world = fresh();
    let mut whole = GpuWorld::with_config(&device, small(Window::Fixed(extent)));
    let s = whole.sync_now(&whole_world, &device, &queue).unwrap();
    assert_eq!((s.chunks, s.sized_chunks, s.kept_sizes), (64, 64, 0));
    let coords = whole_world.store().coords();
    let order = |gpu: &GpuWorld| -> Vec<Option<u32>> {
        coords.iter().map(|&c| gpu.record_index(c)).collect()
    };

    let split_world = fresh();
    let mut split = GpuWorld::with_config(
        &device,
        MirrorConfig {
            sync_budget_time: Some(8 * step),
            ..small(Window::Fixed(extent))
        },
    );
    split.set_sync_clock(stepping_clock(step, SyncPoint::Sized));
    let s = split.sync_now(&split_world, &device, &queue).unwrap();
    assert_eq!(s.sized_chunks, 8, "sizing stops once the budget is spent");
    assert_eq!(s.kept_sizes, 0);
    assert_eq!(s.chunks, 1, "the first sized chunk is written");
    assert_eq!(s.time_deferred_chunks, 7, "the rest of what was sized");
    assert_eq!(s.deferred_chunks, 0);
    assert_eq!(s.pending_chunks, 63);
    let s = split.sync_now(&split_world, &device, &queue).unwrap();
    assert_eq!(
        (s.sized_chunks, s.kept_sizes),
        (8, 7),
        "the 7 left are not sized again"
    );
    assert_eq!((s.chunks, s.time_deferred_chunks), (1, 14));
    let mut sized = 16;
    let mut syncs = 2;
    let mut last = s;
    while split.pending_chunks() > 0 {
        last = split.sync_now(&split_world, &device, &queue).unwrap();
        assert!(last.sized_chunks <= 8);
        sized += last.sized_chunks;
        syncs += 1;
    }
    assert_eq!(sized, 64, "each chunk sized once over the drain");
    assert_eq!(syncs, 9);
    assert_eq!(
        (last.sized_chunks, last.kept_sizes, last.chunks),
        (0, 56, 56)
    );
    assert_eq!(order(&split), order(&whole));
    let read = |b: &wgpu::Buffer| vxe_render::debug::read_buffer(&device, &queue, b, 0, b.size());
    for k in 0..4 {
        assert!(
            read(split.pool_shard(k)) == read(whole.pool_shard(k)),
            "pool shard {k} differs"
        );
    }
    assert!(read(split.records()) == read(whole.records()));
}

/// The first sync of a 4 x 4 x 4 chunk world of [`Mixed`] bricks, timed on the wall clock
/// around the call: its six stages (`select_us` to `alloc_us`, on the mirror's stage clock)
/// sum to within 10 % of that wall, every stage the sync ran is in one of them (`pack_us`
/// holds only the band repair, off here), `light_us` is its two parts' sum, and
/// `packed_bricks` is the world's `Full` brick count (each packed once on the render thread).
#[test]
fn the_sync_stages_sum_to_its_wall() {
    let Some((device, queue)) = device() else {
        return;
    };
    let extent = ChunkExtent::new([0, 0, 0], [4, 4, 4]);
    let world = World::generate(extent, &Mixed, &mixed_table());
    let coords = world.store().coords();
    assert_eq!(coords.len(), 64);
    let full = full_bricks(&world, &coords);
    assert!(full > 1000, "the world holds {full} Full bricks");
    let mut gpu = GpuWorld::with_config(&device, small(Window::Fixed(extent)));
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    #[allow(
        clippy::disallowed_methods,
        reason = "the test reads the sync's wall against its stage clock"
    )]
    let (s, wall_us) = {
        let t = std::time::Instant::now();
        let s = gpu.sync(&world, &queue, &mut enc).unwrap();
        (s, t.elapsed().as_micros() as u64)
    };
    queue.submit([enc.finish()]);
    assert_eq!(s.chunks, 64);
    let stages = [
        s.select_us,
        s.size_us,
        s.pack_us,
        s.light_us,
        s.copy_us,
        s.alloc_us,
    ];
    let sum: u64 = stages.iter().sum();
    println!("wall {wall_us} us, stages {stages:?}, sum {sum} us");
    assert!(
        sum.abs_diff(wall_us) * 10 <= wall_us,
        "stages {stages:?} sum to {sum} us against a wall of {wall_us} us"
    );
    assert!(s.size_us > 0 && s.copy_us > 0 && s.alloc_us > 0);
    assert_eq!(s.light_place_us + s.light_note_us, s.light_us);
    assert_eq!(s.packed_bricks, u64::from(full));
    assert_eq!(s.bricks, u64::from(full));
}

/// One [`Checker`] brick at slot 0 of each chunk, air elsewhere.
struct OneFull;

impl WorldSource for OneFull {
    fn chunk(&self, _: ChunkCoord, out: &mut Chunk) {
        out.set_brick(0, Brick::from_fn(|l| 1 + ((l[0] + l[1] + l[2]) % 2) as u16));
    }
}

/// The stage clock reads per chunk and per pass, never per brick: the first syncs of two
/// 2 x 2 x 2 chunk worlds, one of 512 `Full` bricks a chunk ([`Checker`]) and one of a single
/// `Full` brick a chunk ([`OneFull`]), read the clock the same number of times.
#[test]
fn the_stage_clock_reads_per_chunk_not_per_brick() {
    let Some((device, queue)) = device() else {
        return;
    };
    fn first<S: WorldSource + Sync>(
        src: &S,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> vxe_render::SyncStats {
        let extent = ChunkExtent::new([0, 0, 0], [2, 2, 2]);
        let world = World::generate(extent, src, &numbered_table(2));
        let mut gpu = GpuWorld::with_config(device, small(Window::Fixed(extent)));
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let s = gpu.sync(&world, queue, &mut enc).unwrap();
        queue.submit([enc.finish()]);
        s
    }
    let all = first(&Checker, &device, &queue);
    let one = first(&OneFull, &device, &queue);
    println!(
        "clock reads: every brick Full {}, one a chunk {}",
        all.clock_reads, one.clock_reads
    );
    assert_eq!((all.chunks, one.chunks), (8, 8));
    assert_eq!((all.packed_bricks, one.packed_bricks), (8 * 512, 8));
    assert!(all.clock_reads > 0);
    assert_eq!(all.clock_reads, one.clock_reads);
}
