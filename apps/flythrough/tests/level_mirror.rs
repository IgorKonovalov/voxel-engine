//! The mirror on the level-size cave (seed 1, 20 x 20 x 8 chunks) on the software adapter:
//! how many writes the first sync issues, what the size-classed mirror occupies, that it
//! spreads over capped pool shards, and that a byte budget bounds every sync.

mod support;

use flythrough::cave::{CaveSource, LEVEL_CHUNKS};
use flythrough::cave_materials;
use support::SyncNow;
use vxe_render::gpu_world::{
    ABSENT, BELT_CHUNK_BYTES, BELT_KEEP_CHUNKS, BINS, BITS_WORDS, CLASS_MASK, CLASS_SHIFT,
    EMPTY_BIT, ENTRY_WORD, FULL_BIT, HEADER_TRANSLUCENT_WORD, LOD_BIT, PAGE_BYTES, PAGE_UNITS,
    PLACEHOLDER_BYTES, RECORD_BYTES, RECORD_MASK, RECORD_WORDS, UNIT_BYTES, UNIT_MASK, UNIT_WORDS,
    payload_units,
};
use vxe_render::{GpuWorld, MirrorConfig, Window, readback, wgpu};
use vxe_world::*;

fn level() -> (World, CaveSource) {
    let source = CaveSource::new(1, ChunkExtent::new([0, 0, 0], LEVEL_CHUNKS));
    (
        World::generate(source.extent, &source, &cave_materials()),
        source,
    )
}

/// The reservation the level's mirrors make: 256 MiB.
const LEVEL_BUDGET: u64 = 256 << 20;

/// A fixed window over the level's extent reserving [`LEVEL_BUDGET`].
fn level_config(world: &World) -> MirrorConfig {
    MirrorConfig {
        budget_bytes: LEVEL_BUDGET,
        ..MirrorConfig::new(Window::Fixed(world.extent()))
    }
}

/// The software adapter with limits for the level's pool.
fn device(world: &World) -> Option<(wgpu::Device, wgpu::Queue)> {
    support::software_device(flythrough::level_pool_units(world, false) * UNIT_BYTES)
}

#[test]
fn the_first_sync_issues_at_most_three_writes_per_chunk() {
    let (world, source) = level();
    let s = world.stats();
    let Some((device, queue)) = device(&world) else {
        return;
    };
    let mut gpu = GpuWorld::with_config(&device, level_config(&world));
    gpu.set_focus(source.spawn());
    let st = gpu.sync_now(&world, &device, &queue).unwrap();
    println!(
        "first sync: {} writes, {} chunks, {} bricks, {} B",
        st.writes, st.chunks, st.bricks, st.bytes
    );
    assert_eq!(st.chunks, 3200);
    assert_eq!(st.bricks, s.full_bricks as u64);
    assert!(st.writes <= 3 * 3200 + 8, "{} writes", st.writes);
    assert_eq!(st.pending_chunks, 0);

    // Size-classed (ADR-0012): 275 466 one-material bricks of 2 units, 107 325 two-material
    // bricks of 3, 817 of three or four materials of 4 (Plan 0021's census), and a 2112-byte
    // record per mirrored chunk.
    assert_eq!(s.full_bricks, 383_608);
    let bins = gpu.bin_stats();
    let live: Vec<(u32, u32)> = bins
        .iter()
        .filter(|b| b.live > 0)
        .map(|b| (b.units, b.live))
        .collect();
    assert_eq!(live, [(2, 275_466), (3, 107_325), (4, 817)]);
    assert_eq!(gpu.live_units() * UNIT_BYTES, 56_075_200);
    assert_eq!(RECORD_BYTES, 2112);
    assert!(gpu.records().size() >= 3200 * RECORD_BYTES);
    let rows = cave_materials().len() as u64;
    assert_eq!(gpu.materials().size(), rows * 64);
    assert_eq!(
        gpu.mirror_bytes(),
        56_075_200 + 3200 * 2112 + 3200 * 4 + rows * 64
    );
    assert!(gpu.mirror_bytes() < 64_000_000, "{} B", gpu.mirror_bytes());
    // The 256 MiB reservation holds it: the budget rounded down to whole pages after the
    // extent's 3200 records, unchanged by the sync.
    assert_eq!(gpu.reserved_records(), 3200);
    assert_eq!(gpu.allocated_bytes(), gpu.shard_layout().reserved_bytes());
    assert!(gpu.allocated_bytes() <= LEVEL_BUDGET);
    assert!(LEVEL_BUDGET - gpu.allocated_bytes() < PAGE_BYTES + PLACEHOLDER_BYTES);
    assert_eq!(st.buffers_created, 0);
    assert_eq!(st.copies, 0);
    // The unbudgeted sync took more belt chunks than the belt keeps, so it was replaced.
    assert_eq!(gpu.belt_trims(), 1);
    assert_eq!(gpu.belt_chunks(), 0);
    println!(
        "mirror bytes {} live, {} allocated",
        gpu.mirror_bytes(),
        gpu.allocated_bytes()
    );
}

#[test]
fn every_budgeted_sync_writes_at_most_the_budget_plus_one_chunk() {
    const BUDGET: u64 = 8 << 20;
    // The largest upload of a chunk without attributes: 512 bricks of more than 16
    // materials, each a 20-unit slot, its record and its table entry.
    const LARGEST_CHUNK: u64 = 512 * 20 * 64 + 2112 + 4;
    let (world, source) = level();
    let Some((device, queue)) = device(&world) else {
        return;
    };
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            window: Window::Fixed(world.extent()),
            sync_budget_bytes: BUDGET,
            max_shard_units: None,
            budget_bytes: LEVEL_BUDGET,
            record_chunks: None,
            attr_cap: usize::MAX,
            ..MirrorConfig::new(Window::Fixed(world.extent()))
        },
    );
    gpu.set_focus(source.spawn());
    let spawn_chunk = source.spawn().chunk();
    let mut syncs = 0;
    let mut largest = 0;
    loop {
        let st = gpu.sync_now(&world, &device, &queue).unwrap();
        syncs += 1;
        largest = largest.max(st.bytes);
        assert!(
            st.bytes <= BUDGET + LARGEST_CHUNK,
            "sync {syncs} wrote {} B",
            st.bytes
        );
        if syncs == 1 {
            assert!(gpu.is_mirrored(spawn_chunk), "the focus chunk comes first");
        }
        if st.pending_chunks == 0 {
            break;
        }
        assert!(syncs < 1000);
    }
    println!("{syncs} syncs, largest {largest} B");
    assert_eq!(gpu.mirrored_chunks(), 3200);
    assert!(syncs > 1);
}

/// The mirror's buffers, read back: each in-use shard up to the pool's high-water mark, the
/// records and the table.
fn mirror_bytes(device: &wgpu::Device, queue: &wgpu::Queue, gpu: &GpuWorld) -> Vec<Vec<u8>> {
    let ups = u64::from(gpu.shard_layout().units_per_shard);
    let pool = u64::from(gpu.pool_units());
    let mut out: Vec<Vec<u8>> = (0..gpu.shards_in_use())
        .map(|k| {
            let units = (pool - k as u64 * ups).min(ups);
            readback::read_buffer(device, queue, gpu.pool_shard(k), 0, units * UNIT_BYTES)
        })
        .collect();
    for b in [gpu.records(), gpu.table()] {
        out.push(readback::read_buffer(device, queue, b, 0, b.size()));
    }
    out
}

/// A 16 MiB sync budget makes each of the level's syncs stage several belt chunks' worth;
/// the mirror they build reads back byte for byte as the one unbudgeted sync builds.
#[test]
fn syncs_past_one_belt_chunk_build_the_same_mirror() {
    const BUDGET: u64 = 16 << 20;
    let (world, source) = level();
    let Some((device, queue)) = device(&world) else {
        return;
    };
    let mut whole = GpuWorld::with_config(&device, level_config(&world));
    whole.set_focus(source.spawn());
    whole.sync_now(&world, &device, &queue).unwrap();
    let expected = mirror_bytes(&device, &queue, &whole);
    drop(whole);

    // A second world: the first one's dirty bits were taken by the unbudgeted sync.
    let (world, _) = level();
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            sync_budget_bytes: BUDGET,
            ..level_config(&world)
        },
    );
    gpu.set_focus(source.spawn());
    let mut syncs = 0;
    loop {
        let st = gpu.sync_now(&world, &device, &queue).unwrap();
        syncs += 1;
        if st.pending_chunks > 0 {
            assert!(
                st.bytes > BELT_CHUNK_BYTES,
                "sync {syncs} wrote {} B",
                st.bytes
            );
        }
        assert!(gpu.belt_chunks() <= BELT_KEEP_CHUNKS, "sync {syncs}");
        if st.pending_chunks == 0 {
            break;
        }
        assert!(syncs < 100);
    }
    println!("{syncs} syncs, {} belt trims", gpu.belt_trims());
    assert!(syncs > 1);
    assert_eq!(gpu.mirrored_chunks(), 3200);
    let got = mirror_bytes(&device, &queue, &gpu);
    assert_eq!(got.len(), expected.len());
    for (k, (g, e)) in got.iter().zip(&expected).enumerate() {
        assert!(g == e, "buffer {k} differs");
    }
}

/// SplitMix64: a seeded, platform-independent stream for test inputs.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        vxe_world::hash::mix64(self.0)
    }
}

/// Little-endian `u32` words of `bytes`.
fn le_words(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks_exact(4)
        .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
        .collect()
}

/// The voxels of the `Full` brick at entry `e` of a slot whose words start at `w`, decoded as
/// the shader decodes them (the level has no translucent material and no brick past four
/// materials): a voxel set in the opaque mask reads its palette entry by the planes' bits.
fn decode(w: &[u32], e: u32) -> Vec<u16> {
    let planes = payload_units(e >> CLASS_SHIFT & CLASS_MASK) as usize;
    let bit = |at: usize, i: usize| w[at + i / 32] >> (i % 32) & 1;
    (0..BRICK_VOXELS)
        .map(|i| {
            if bit(0, i) == 0 {
                return AIR;
            }
            let idx = (0..planes).fold(0, |a, k| a | (bit(32 + 16 * k, i) as usize) << k);
            (w[16 + idx / 2] >> (16 * (idx & 1))) as u16
        })
        .collect()
}

/// With shards capped at 64 pages (262 144 units), the level's 215 pages take four shards,
/// and bricks in every shard read back as their CPU voxels.
#[test]
fn the_level_spreads_over_capped_shards() {
    let (world, source) = level();
    let Some((device, queue)) = device(&world) else {
        return;
    };
    let mut gpu = GpuWorld::with_config(
        &device,
        MirrorConfig {
            max_shard_units: Some(64 * PAGE_UNITS),
            ..level_config(&world)
        },
    );
    let layout = gpu.shard_layout();
    println!("layout {layout:?}");
    assert_eq!(layout.units_per_shard, 64 * PAGE_UNITS);
    assert_eq!(layout.shards, 4);
    gpu.set_focus(source.spawn());
    gpu.sync_now(&world, &device, &queue)
        .expect("no PoolTooLarge over four shards");
    assert_eq!(gpu.live_bricks(), 383_608);
    assert_eq!(gpu.pages_issued(), 215);
    assert_eq!(gpu.shards_in_use(), 4);

    // Every Full brick by (chunk, slot), ascending; 1000 drawn with a seeded stream.
    let mut full = Vec::new();
    for c in world.store().coords() {
        let chunk = world.store().get(c).unwrap();
        let chunk = chunk.read().unwrap();
        for (slot, b) in chunk.bricks().iter().enumerate() {
            if b.is_full() {
                full.push((c, slot));
            }
        }
    }
    let mut rng = SplitMix64(0x5A4D_0005);
    let picks: Vec<(ChunkCoord, usize)> = (0..1000)
        .map(|_| full[(rng.next_u64() % full.len() as u64) as usize])
        .collect();

    let shards: Vec<Vec<u32>> = (0..4)
        .map(|k| {
            let b = gpu.pool_shard(k);
            le_words(&readback::read_buffer(&device, &queue, b, 0, b.size()))
        })
        .collect();
    let records = readback::read_buffer(&device, &queue, gpu.records(), 0, gpu.records().size());
    let records: Vec<u32> = le_words(&records);
    let mut by_shard = [0; 4];
    for &(c, slot) in &picks {
        let rec = gpu.record_index(c).unwrap() as usize * RECORD_WORDS;
        let e = records[rec + ENTRY_WORD + slot];
        assert_eq!(Some(e), gpu.entry(c, slot));
        assert_eq!(e & FULL_BIT, FULL_BIT);
        let u = e & UNIT_MASK;
        let (k, local) = (u / layout.units_per_shard, u % layout.units_per_shard);
        by_shard[k as usize] += 1;
        let words = &shards[k as usize][local as usize * UNIT_WORDS..];
        let chunk = world.store().get(c).unwrap();
        let chunk = chunk.read().unwrap();
        let mut v = [AIR; BRICK_VOXELS];
        chunk.brick(slot).decode(&mut v);
        assert_eq!(decode(words, e), v, "{c:?} brick {slot}");
    }
    println!("picks by shard {by_shard:?}");
    assert!(by_shard.iter().all(|&n| n >= 1), "{by_shard:?}");
}

/// Every record's 16 brick-bits words hold, for each chunk, the bits of its bricks that are
/// not `Uniform(AIR)`; the table carries `EMPTY_BIT` on exactly the entries whose bits are all
/// clear, and every entry that is not `ABSENT` names the CPU's record.
#[test]
fn the_level_s_brick_bits_and_empty_bits_match_its_bricks() {
    let (world, source) = level();
    let Some((device, queue)) = device(&world) else {
        return;
    };
    let mut gpu = GpuWorld::with_config(&device, level_config(&world));
    gpu.set_focus(source.spawn());
    gpu.sync_now(&world, &device, &queue).unwrap();
    let words = |b: &wgpu::Buffer| -> Vec<u32> {
        readback::read_buffer(&device, &queue, b, 0, b.size())
            .chunks_exact(4)
            .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
            .collect()
    };
    let records = words(gpu.records());
    let table = words(gpu.table());
    let mut empty = 0;
    for c in world.store().coords() {
        let chunk = world.store().get(c).unwrap();
        let chunk = chunk.read().unwrap();
        let mut want = [0u32; BITS_WORDS];
        for (b, brick) in chunk.bricks().iter().enumerate() {
            if *brick != Brick::Uniform(AIR) {
                want[b / 32] |= 1 << (b % 32);
            }
        }
        let rec = gpu.record_index(c).unwrap();
        let bits = &records[rec as usize * RECORD_WORDS..][..BITS_WORDS];
        assert_eq!(bits, want, "{c:?}");
        let t = table[gpu.table_index(c) as usize];
        assert_ne!(t, ABSENT, "{c:?}");
        assert_eq!(t & LOD_BIT, 0, "{c:?}");
        assert_eq!(t & RECORD_MASK, rec, "{c:?}");
        let is_empty = want.iter().all(|&w| w == 0);
        assert_eq!(t & EMPTY_BIT != 0, is_empty, "{c:?}");
        empty += usize::from(is_empty);
    }
    println!("{empty} of 3200 chunks empty");
    assert!(empty > 0 && empty < 3200);
}

/// Every `Full` brick's slot opens with its 16 opaque mask words, which read back as its CPU
/// `solid_bits()`, and its header's two translucent words are clear; each slot is its bin's
/// units, so the live units are the bins' slots.
#[test]
fn the_level_s_opaque_masks_are_its_solid_bits() {
    let (world, source) = level();
    let Some((device, queue)) = device(&world) else {
        return;
    };
    let mut gpu = GpuWorld::with_config(&device, level_config(&world));
    gpu.set_focus(source.spawn());
    gpu.sync_now(&world, &device, &queue).unwrap();
    let bins = gpu.bin_stats();
    let units: u64 = bins
        .iter()
        .map(|b| u64::from(b.live) * u64::from(b.units))
        .sum();
    assert_eq!(units, gpu.live_units());
    assert_eq!(gpu.shards_in_use(), 1);
    let pool: Vec<u32> = le_words(&readback::read_buffer(
        &device,
        &queue,
        gpu.pool_shard(0),
        0,
        u64::from(gpu.pool_units()) * UNIT_BYTES,
    ));
    let mut checked = 0;
    for c in world.store().coords() {
        let chunk = world.store().get(c).unwrap();
        let chunk = chunk.read().unwrap();
        for (slot, brick) in chunk.bricks().iter().enumerate() {
            if !brick.is_full() {
                continue;
            }
            let e = gpu.entry(c, slot).unwrap();
            let o = (e & UNIT_MASK) as usize * UNIT_WORDS;
            let got: [u64; 8] = std::array::from_fn(|z| {
                u64::from(pool[o + 2 * z]) | u64::from(pool[o + 2 * z + 1]) << 32
            });
            assert_eq!(got, brick.solid_bits(), "{c:?} brick {slot}");
            let t = o + UNIT_WORDS + HEADER_TRANSLUCENT_WORD;
            assert_eq!(pool[t..t + 2], [0, 0], "{c:?} brick {slot}");
            assert!(
                BINS.contains(&(2 + payload_units(e >> CLASS_SHIFT & CLASS_MASK))),
                "{c:?} brick {slot}"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 383_608);
}
