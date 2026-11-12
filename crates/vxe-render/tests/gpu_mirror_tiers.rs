//! The GPU mirror's tiers: a following window mirroring the streamed residency, LOD records
//! and demotions that free full slots, and the order promotions, demotions and digs upload in.

mod support;

use support::*;

use std::collections::HashSet;
use std::sync::Arc;
use vxe_render::{GpuWorld, MirrorConfig, SyncPoint, Window, wgpu};
use vxe_render::{
    internals::ABSENT, internals::BINS, internals::FULL_BIT, internals::PAGE_BYTES,
    internals::PAGE_UNITS, internals::PLACEHOLDER_BYTES, internals::RECORD_BYTES,
    internals::UNIT_BYTES, internals::UNIT_MASK, internals::bin_of,
};
use vxe_world::*;

/// Whether the slot at unit `u` is free: on its bin's free list, or in a free page.
fn is_free(gpu: &GpuWorld, u: u32) -> bool {
    let p = u / PAGE_UNITS;
    if gpu.free_pages().contains(&p) {
        return true;
    }
    gpu.bin_of_unit(u)
        .is_some_and(|k| gpu.free_slots(k).contains(&u))
}

/// The buffers a mirror reserved: its four shards and records.
fn reserved_buffers(gpu: &GpuWorld) -> Vec<wgpu::Buffer> {
    (0..4)
        .map(|k| gpu.pool_shard(k).clone())
        .chain([gpu.records().clone()])
        .collect()
}

/// The streamer's radius-2 scenario through a following window of side 7: after the move the
/// mirror holds the 42 resident chunks, each decoding to its CPU voxels, and the evicted
/// chunks' slots went to the free lists. The 64 MiB reservation is made once: its bytes are
/// the budget rounded down to whole pages and the window's 343 records, and no sync, eviction
/// or window move creates, copies or resizes a buffer.
#[test]
fn a_following_window_mirrors_the_streamed_residency() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = World::new(&mixed_table());
    let mut s = Streamer::new(
        Arc::new(Mixed),
        StreamConfig {
            residency: Residency {
                r_full: 2,
                r_lod: 2,
                hysteresis: 1,
            },
            workers: 4,
            persist: PersistConfig::default(),
        },
    );
    let mut gpu = GpuWorld::with_config(&device, small(Window::Follow { side: 7 }));
    // 343 records, then whole pages of what is left past the placeholders: 253 pages on one
    // shard (lavapipe's 2 097 152 units), the other three shards one-unit placeholders.
    assert_eq!(gpu.reserved_records(), 343);
    let pages = (SMALL_BUDGET - 343 * RECORD_BYTES - PLACEHOLDER_BYTES) / PAGE_BYTES;
    assert_eq!(pages, 253);
    assert_eq!(u64::from(gpu.reserved_units()), pages * 4096);
    let reserved = 343 * RECORD_BYTES + pages * PAGE_BYTES + 3 * UNIT_BYTES;
    assert_eq!(gpu.allocated_bytes(), reserved);
    assert!(SMALL_BUDGET - reserved < PAGE_BYTES + PLACEHOLDER_BYTES);
    let buffers = reserved_buffers(&gpu);

    let focus = VoxelCoord([0, 0, 0]);
    s.update(&world, &[focus]);
    let ev = s.wait_idle(&world);
    gpu.apply_events(&ev);
    gpu.set_focus(focus);
    let st = gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(gpu.allocated_bytes(), reserved);
    assert_pressure(&gpu);
    assert_eq!(st.chunks, 33);
    assert_eq!(gpu.mirrored_chunks(), 33);
    let first = world.store().coords();
    let rb = Readback::new(&device, &queue, &gpu);
    let mut old_units = HashSet::new();
    for &c in &first {
        old_units.extend(rb.assert_chunk(&gpu, &world, c));
    }

    let focus = VoxelCoord([192, 0, 0]);
    s.update(&world, &[focus]);
    let ev = s.wait_idle(&world);
    assert_eq!(ev.evicted.len(), 22);
    let evicted_units: HashSet<u32> = ev
        .evicted
        .iter()
        .flat_map(|&c| (0..512).map(move |slot| (c, slot)))
        .filter_map(|(c, slot)| gpu.entry(c, slot))
        .filter(|e| e & FULL_BIT != 0)
        .map(|e| e & UNIT_MASK)
        .collect();
    assert!(!evicted_units.is_empty());
    let (live_before, allocated_before) = (gpu.mirror_bytes(), gpu.allocated_bytes());
    gpu.apply_events(&ev);
    // Eviction lowers the live accounting; the reservation stays.
    assert!(gpu.mirror_bytes() < live_before);
    assert_eq!(gpu.allocated_bytes(), allocated_before);
    assert_eq!(gpu.allocated_bytes(), reserved);
    assert_pressure(&gpu);
    // Eviction returns exactly the evicted chunks' slots: to their bins' free lists, or with
    // a page that emptied, to the free pages.
    for &u in &evicted_units {
        assert!(is_free(&gpu, u), "unit {u}");
    }
    assert_units_accounted(&gpu);

    gpu.set_focus(focus);
    let st = gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(st.pending_chunks, 0);
    // The window moved by three chunks; nothing was created or resized.
    assert_eq!(gpu.allocated_bytes(), reserved);
    assert_eq!(reserved_buffers(&gpu), buffers);
    assert_pressure(&gpu);
    let resident = world.store().coords();
    assert_eq!(resident.len(), 42);
    assert_eq!(gpu.mirrored_chunks(), 42);
    for &c in &ev.evicted {
        assert!(!gpu.is_mirrored(c));
    }
    let rb = Readback::new(&device, &queue, &gpu);
    let mut live = HashSet::new();
    let mut units = 0u64;
    for &c in &resident {
        for u in rb.assert_chunk(&gpu, &world, c) {
            assert!(live.insert(u), "unit {u} held twice");
            assert!(!is_free(&gpu, u), "unit {u} is live and free");
            units += u64::from(BINS[gpu.bin_of_unit(u).unwrap()]);
        }
    }
    assert_eq!(gpu.live_bricks(), full_bricks(&world, &resident));
    assert_eq!(live.len() as u32, gpu.live_bricks());
    assert_eq!(units, gpu.live_units());
    assert_units_accounted(&gpu);
    // The evicted chunks' table entries are gone: their wrapped indices now hold new chunks
    // or nothing.
    for &c in &ev.evicted {
        let t = gpu.table_index(c) as usize;
        let holder = resident.iter().find(|&&r| gpu.table_index(r) as usize == t);
        match holder {
            Some(&r) => assert_eq!(Some(rb.table[t]), gpu.record_index(r)),
            None => assert_eq!(rb.table[t], ABSENT),
        }
    }
    assert!(old_units.len() > 100);
}

/// A two-tier residency through the mirror: LOD records decode to the world's mips, and a
/// demoted chunk trades its full record for a LOD one, its slots going back to the pool.
#[test]
fn lod_records_decode_to_the_mips_and_demotion_frees_the_full_slots() {
    let Some((device, queue)) = device() else {
        return;
    };
    let world = World::new(&mixed_table());
    let mut s = Streamer::new(
        Arc::new(Mixed),
        StreamConfig {
            residency: Residency {
                r_full: 1,
                r_lod: 2,
                hysteresis: 0,
            },
            workers: 4,
            persist: PersistConfig::default(),
        },
    );
    let mut gpu = GpuWorld::with_config(&device, small(Window::Follow { side: 7 }));
    let step = |s: &mut Streamer, gpu: &mut GpuWorld, focus: VoxelCoord| {
        s.update(&world, &[focus]);
        let ev = s.wait_idle(&world);
        gpu.apply_events(&ev);
        gpu.set_focus(focus);
        gpu.sync_now(&world, &device, &queue).unwrap();
        ev
    };
    step(&mut s, &mut gpu, VoxelCoord([0, 0, 0]));
    assert_eq!(world.lod_coords().len(), 26);
    let rb = Readback::new(&device, &queue, &gpu);
    for c in world.store().coords() {
        assert!(!gpu.is_lod(c));
        rb.assert_chunk(&gpu, &world, c);
    }
    let mut lod_slots = 0;
    for c in world.lod_coords() {
        assert!(gpu.is_lod(c));
        lod_slots += rb.assert_lod(&gpu, &world, c).len();
    }
    assert!(
        lod_slots > 26,
        "the ring holds Full mip-bricks: {lod_slots}"
    );

    let demoted = ChunkCoord([1, 0, 0]);
    let full_units: HashSet<u32> = (0..512)
        .filter_map(|slot| gpu.entry(demoted, slot))
        .filter(|e| e & FULL_BIT != 0)
        .map(|e| e & UNIT_MASK)
        .collect();
    assert!(!full_units.is_empty());

    let ev = step(&mut s, &mut gpu, VoxelCoord([-64, 0, 0]));
    assert!(ev.demoted.contains(&demoted));
    assert!(gpu.is_lod(demoted));
    let rb = Readback::new(&device, &queue, &gpu);
    let mut live = HashSet::new();
    for c in world.store().coords() {
        live.extend(rb.assert_chunk(&gpu, &world, c));
    }
    for c in world.lod_coords() {
        live.extend(rb.assert_lod(&gpu, &world, c));
    }
    assert_eq!(live.len() as u32, gpu.live_bricks());
    // Each slot the demoted chunk's full record held is free (alone or in a free page), or
    // reused by a live record; every unit handed out is accounted for.
    for &u in &full_units {
        assert!(is_free(&gpu, u) != live.contains(&u), "unit {u}");
    }
    assert!(full_units.iter().any(|&u| is_free(&gpu, u)));
    assert_units_accounted(&gpu);
    let lod_full = world.lod(demoted).unwrap().full_bricks();
    assert!(
        lod_full < full_units.len(),
        "{lod_full} mip-bricks for {} bricks",
        full_units.len()
    );
}

/// A demotion that frees its pages in the same sync fits a full pool, with the field's band
/// slots: three `Checker` chunks take exactly the reserved pages (their bricks in the 3-unit
/// bin, their band bricks' band slots in the band bins); one sync demotes them to
/// their own mips (a checkerboard's mip-bricks are uniform, so they take no slot) and adds a
/// farther LOD-only chunk whose `Mixed` mips need 2- and 3-unit slots. The demotions empty
/// the pages first, so those slots come from freed pages. A pool check that credited no tier
/// change's frees would ask for a page past the reservation and fail the sync with
/// `PoolTooLarge`.
#[test]
fn a_demotion_that_empties_pages_fits_a_full_pool() {
    let Some((device, queue)) = device() else {
        return;
    };
    let (world, chunks, mixed) = tier_world();
    for x in 0..3 {
        to_full(&world, x, &chunks);
    }
    let pages = pages_taken(&device, &queue, &world, true);
    let mut gpu = GpuWorld::with_field_surface(
        &device,
        MirrorConfig {
            budget_bytes: pages_budget(u64::from(pages), 12),
            ..MirrorConfig::new(Window::Fixed(world.extent()))
        },
    );
    gpu.set_focus(VoxelCoord([1, 1, 1]));
    gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(gpu.reserved_units(), pages * 4096);
    // The bricks' slots and a band slot for each band brick (the bricks on the world's faces).
    assert_eq!(gpu.live_bricks(), 3 * 512 + gpu.band_bricks() as u32);
    assert_eq!(gpu.pages_issued(), pages);
    assert!(pages >= 4, "{pages} pages over four bins");

    for x in 0..3 {
        let own = Arc::new(ChunkMips::build(&chunks[x as usize].read().unwrap()));
        assert_eq!(own.full_bricks(), 0, "a checkerboard's mips are uniform");
        to_lod(&world, x, &own);
    }
    world.insert_lod(ChunkCoord([3, 0, 0]), mixed[3].clone());
    let need = mixed[3].full_bricks() as u32;
    assert!(need > 0);
    let st = gpu
        .sync_now(&world, &device, &queue)
        .expect("the demotions free the pages first");
    assert_eq!(st.chunks, 4);
    assert_eq!(gpu.live_bricks(), need);
    assert_eq!(gpu.pages_issued(), pages);
    let rb = Readback::new(&device, &queue, &gpu);
    for x in 0..4 {
        rb.assert_lod(&gpu, &world, ChunkCoord([x, 0, 0]));
    }
}

/// The same without band slots: eight `Checker` chunks fill a four-page reservation of
/// 3-unit slots; demoting them and adding a farther LOD-only chunk fits, its mip-bricks taking
/// what the demotions free. A pool check that credited no tier change's frees would fail it
/// with `PoolTooLarge`.
#[test]
fn a_plain_demotion_fits_a_full_pool() {
    let Some((device, queue)) = device() else {
        return;
    };
    let (world, chunks, mixed) = tier_world();
    for x in 0..8 {
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
    assert_eq!(gpu.live_bricks(), 4096);
    assert_eq!(gpu.pages_issued(), 4);
    assert_eq!(gpu.reserved_units(), 4 * 4096);

    for x in 0..8 {
        let own = Arc::new(ChunkMips::build(&chunks[x as usize].read().unwrap()));
        to_lod(&world, x, &own);
    }
    world.insert_lod(ChunkCoord([8, 0, 0]), mixed[8].clone());
    let st = gpu
        .sync_now(&world, &device, &queue)
        .expect("the demotions free slots first");
    assert_eq!(st.chunks, 9);
    assert_eq!(gpu.live_bricks(), mixed[8].full_bricks() as u32);
    let rb = Readback::new(&device, &queue, &gpu);
    for x in 0..9 {
        rb.assert_lod(&gpu, &world, ChunkCoord([x, 0, 0]));
    }
}

/// A dug chunk goes first: on a four-page pool holding a `Floor` chunk at the focus (a page
/// of 2-unit slots) and four far `Checker` chunks (2048 3-unit slots, two pages, 682 spare),
/// five nearer chunks are promoted from LOD-only (their checkerboard mips take no slot) while
/// a fill at the focus gains `Full` bricks. The dug chunk uploads and reads back to its
/// edited voxels; the promotions that fit (682 spare and a fourth page of 1365: three of 512)
/// follow nearest first, and only chunks farther than every uploaded one are deferred.
#[test]
fn a_dug_chunk_uploads_ahead_of_farther_promotions() {
    let Some((device, queue)) = device() else {
        return;
    };
    let extent = ChunkExtent::new([0, 0, 0], [10, 1, 1]);
    let world = World::generate(extent, &FloorAndChecker, &numbered_table(2));
    let mut held = Vec::new();
    for x in 1..6 {
        let c = ChunkCoord([x, 0, 0]);
        let chunk = world.store().remove(c).unwrap();
        world.insert_lod(c, Arc::new(ChunkMips::build(&chunk.read().unwrap())));
        held.push((c, chunk));
    }
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            budget_bytes: pages_budget(4, 10),
            ..MirrorConfig::new(Window::Fixed(extent))
        },
    );
    gpu.set_focus(VoxelCoord([1, 1, 1]));
    gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(gpu.live_bricks(), 64 + 4 * 512);
    assert_eq!(gpu.pages_issued(), 3);

    let r = world
        .apply(&EditOp::box_(
            centre_of_voxel([0, 0, 8]),
            centre_of_voxel([16, 8, 10]),
            Mode::Fill(1),
        ))
        .unwrap();
    assert_eq!(r.touched, [ChunkCoord([0, 0, 0])]);
    for (c, chunk) in held {
        world.remove_lod(c);
        world.store().write_lock().insert(c, chunk);
    }
    let st = gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!(st.chunks, 4);
    assert_eq!(st.deferred_chunks, 2);
    let rb = Readback::new(&device, &queue, &gpu);
    rb.assert_chunk(&gpu, &world, ChunkCoord([0, 0, 0]));
    assert_eq!(full_bricks(&world, &[ChunkCoord([0, 0, 0])]), 66);
    for x in 1..4 {
        rb.assert_chunk(&gpu, &world, ChunkCoord([x, 0, 0]));
    }
    for x in 4..6 {
        assert!(gpu.is_lod(ChunkCoord([x, 0, 0])), "x {x} deferred");
    }
}

/// A demotion is not blocked: the nearest selected chunk, a promotion of 512 bricks, does not
/// fit a two-page pool with 170 spare 3-unit slots; two farther demotions behind it do (they
/// need nothing), upload and read back to their mips, and free 1024 slots, so the promotion
/// uploads in the next sync.
#[test]
fn a_demotion_behind_a_deferred_chunk_still_uploads() {
    let Some((device, queue)) = device() else {
        return;
    };
    let (world, chunks, _) = tier_world();
    let own: Vec<Arc<ChunkMips>> = chunks
        .iter()
        .map(|c| Arc::new(ChunkMips::build(&c.read().unwrap())))
        .collect();
    world.insert_lod(ChunkCoord([0, 0, 0]), own[0].clone());
    for x in 1..6 {
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
    assert_eq!(gpu.live_bricks(), 5 * 512);
    assert_eq!(gpu.pages_issued(), 2);
    assert_eq!(gpu.bin_stats()[bin_of(3)].spare, 2 * 1365 - 5 * 512);

    world.remove_lod(ChunkCoord([0, 0, 0]));
    to_full(&world, 0, &chunks);
    for x in 4..6 {
        to_lod(&world, x, &own[x as usize]);
    }
    let st = gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!((st.chunks, st.deferred_chunks), (2, 1));
    assert!(gpu.is_lod(ChunkCoord([0, 0, 0])), "the promotion waits");
    let rb = Readback::new(&device, &queue, &gpu);
    for x in 4..6 {
        rb.assert_lod(&gpu, &world, ChunkCoord([x, 0, 0]));
    }
    let st = gpu.sync_now(&world, &device, &queue).unwrap();
    assert_eq!((st.chunks, st.deferred_chunks), (1, 0));
    let rb = Readback::new(&device, &queue, &gpu);
    rb.assert_chunk(&gpu, &world, ChunkCoord([0, 0, 0]));
}

/// A chunk edited while it waits with a kept size uploads its edited words: the floor of
/// [`a_sync_stops_at_its_time_budget`] under its budget of 8 uploads a sync; after the first
/// sync the farthest chunk (the unbudgeted order's last) gains a 3 x 3 x 2 block of the floor
/// material above the floor. The next sync sizes that chunk again and only it (55 kept), and
/// once drained the mirror's records and pool read back byte-identical to an unbudgeted sync
/// of a world edited the same way before its first sync.
#[test]
fn a_chunk_edited_while_deferred_uploads_its_edited_words() {
    let Some((device, queue)) = device() else {
        return;
    };
    let mut t = MaterialTable::new();
    t.push(Material::named("floor"));
    let extent = ChunkExtent::new([0, 0, 0], [8, 8, 1]);
    let fresh = || World::generate(extent, &Floor, &t);
    let step = std::time::Duration::from_micros(250);

    // The unbudgeted order's last chunk, from an unedited mirror.
    let probe_world = fresh();
    let mut probe = GpuWorld::with_config(&device, small(Window::Fixed(extent)));
    probe.sync_now(&probe_world, &device, &queue).unwrap();
    let last = *probe_world
        .store()
        .coords()
        .iter()
        .max_by_key(|&&c| probe.record_index(c))
        .unwrap();
    let edit = |w: &mut World| {
        let v = last.min_voxel().0;
        let r = w
            .apply(&EditOp::box_(
                centre_of_voxel([v[0] + 1, v[1] + 1, v[2] + 3]),
                centre_of_voxel([v[0] + 4, v[1] + 4, v[2] + 5]),
                Mode::Fill(1),
            ))
            .unwrap();
        assert_eq!(r.changed_from, [(0, 18)]);
    };

    let mut whole_world = fresh();
    edit(&mut whole_world);
    let mut whole = GpuWorld::with_config(&device, small(Window::Fixed(extent)));
    whole.sync_now(&whole_world, &device, &queue).unwrap();

    let mut split_world = fresh();
    let mut split = GpuWorld::with_config(
        &device,
        MirrorConfig {
            sync_budget_time: Some(8 * step),
            ..small(Window::Fixed(extent))
        },
    );
    split.set_sync_clock(stepping_clock(step, SyncPoint::Uploaded));
    let s = split.sync_now(&split_world, &device, &queue).unwrap();
    assert_eq!((s.chunks, s.sized_chunks, s.pending_chunks), (8, 64, 56));
    assert_eq!(
        split.record_index(last),
        None,
        "the edited chunk is still deferred"
    );
    edit(&mut split_world);
    let s = split.sync_now(&split_world, &device, &queue).unwrap();
    assert_eq!(
        (s.sized_chunks, s.kept_sizes),
        (1, 55),
        "only the edited chunk is sized again"
    );
    while split.pending_chunks() > 0 {
        split.sync_now(&split_world, &device, &queue).unwrap();
    }
    let coords = whole_world.store().coords();
    let order = |gpu: &GpuWorld| -> Vec<Option<u32>> {
        coords.iter().map(|&c| gpu.record_index(c)).collect()
    };
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
