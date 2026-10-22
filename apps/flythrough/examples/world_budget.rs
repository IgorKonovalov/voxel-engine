//! Prints the cave's storage figures (`nfr.md` §3, §5), bounded or streamed, and for the
//! bounded cave the coarse grid's bytes (4 per brick) and single-threaded build time.
//!
//! ```text
//! cargo run --release -p flythrough --example world_budget -- --chunks 20x20x8 --seed 1
//! cargo run --release -p flythrough --example world_budget -- --stream 16,24 --seed 1
//! cargo run --release -p flythrough --example world_budget -- --stream 16,24 --surface
//! cargo run --release -p flythrough --example world_budget -- --veins 16 --census
//! ```
//!
//! `--chunks` (`AxBxC` or `A,B,C`) generates the bounded cave over an extent. `--stream
//! R_FULL,R_LOD` builds the residency the fly-through streams around the unbounded cave's
//! spawn (hysteresis 1), on one worker per core, and waits until it is complete. `--census` adds the brick census
//! (`World::brick_census`): the `Full` bricks and mip-bricks by count of non-air materials, and
//! their payload bytes today, under the packed class ladder and under exact planes (ADR-0012).
//! `--veins N` generates, over the `--chunks` extent, a synthetic world of `N - 1` materials in
//! 2^3 cells with about a third of the voxels air, and prints its census.
//!
//! Mirror bytes use the uploader's layout (ADR-0012, `gpu_world::slot_units`): each `Full`
//! brick or mip-brick takes a slot of its bin's 64-byte units (mask, header, planes or raw
//! ids, a translucent unit when it has glass), each mirrored chunk (full or LOD-only) a
//! 2112-byte record (16 words of brick bits, 512 entries), each chunk-table entry 4 bytes (the
//! window's `W^3` when streaming), and each material row `MATERIAL_FLOATS` `f32`s. CPU bytes
//! count a full
//! chunk's slot array and its `Full` bricks' class boxes, and a LOD-only chunk's mip-brick slots
//! and `Full` mip-bricks' boxes.
//!
//! The budget lines are the smallest `--mirror-budget` (ADR-0013) that reserves this content
//! at rest (`gpu_world::budget_for`): each bin's slots in whole pages of that bin and a record
//! for every chunk of the extent, or when streaming the records the fly-through reserves for
//! the residency's ball (`flythrough::stream_record_chunks`). Contour adds the attribute units
//! to full chunks' bricks. Neither counts the streamer's hysteresis or the dig headroom.
//!
//! `--surface` (with `--stream`) adds the contour-attribute census of the `Full` bricks of
//! full chunks: their count, their *plane voxels* (solid voxels with at least one air face
//! neighbour, neighbours read across chunk edges from the resident chunk or, past the
//! residency, from the source; an upper bound on the attribute pass's planes, which also drop
//! voxels whose occupancy gradient vanishes), the plane voxels per brick in 32-voxel bins, the
//! 2 x 2 x 2 blocks holding a plane voxel, and the attribute bytes of each footprint layout.
//!
//! The size-classed slot projection (ADR-0012's mirror format) follows `--census` and
//! `--surface`. A `Full` brick's slot is a run of 64-byte units: the opaque mask, a header,
//! then 0, 1, 2 or 4 plane units (one, two, three to four, five to sixteen materials) or 16
//! units of raw ids, so 2, 3, 4, 6 or 18 units; it is allocated from the smallest bin of
//! `gpu_world::BINS` units that holds it. A record is 2112 B (no palette). With contour
//! attributes a full chunk's brick with `n > 0` *chunk-local surface* voxels
//! (`vxe_world::surface_mask`: non-air voxels with an air face neighbour inside the chunk, a
//! chunk face counting as air) adds a unit of surface mask and `ceil(n / 32)` units of `u16`
//! attributes. `--surface` prints, per packed class, its units, bricks, exact plane voxels,
//! chunk-local surface voxels and attribute units, then the projected mirror of each look:
//! pool by bin, records, table and material rows.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard};

use flythrough::cave::CaveSource;
use flythrough::{cave_materials, chunk_ram, mips_ram, stream_record_chunks, window_side};
use vxe_render::gpu_world::{
    BIN_COUNT, BINS, MATERIAL_FLOATS, PAGE_UNITS, RECORD_BYTES, UNIT_BYTES, bin_of, budget_for,
    chunk_surface, class_code, payload_units, slot_units, slots_per_page, surface_attr_units,
};
use vxe_world::hash::{ValueNoise, hash3};
use vxe_world::packed::PackedClass;
use vxe_world::{
    AIR, BRICK_EDGE, BRICK_VOXELS, Brick, BrickCensus, CHUNK_BRICKS, CHUNK_EDGE, Chunk, ChunkCoord,
    ChunkExtent, ChunkMips, CoarseGrid, Material, MaterialFlags, MaterialId, MaterialTable,
    PersistConfig, Residency, StreamConfig, Streamer, World, WorldCensus, WorldLimits, WorldSource,
    brick_of_slot, brick_slot, index_in_brick, solid_mask, surface_mask,
};

/// Units of a `Full` brick's slot without attributes or glass, by packed class.
fn class_units(c: PackedClass) -> u64 {
    u64::from(2 + payload_units(class_code(c)))
}

/// Units of a full chunk's brick's slot with `n` chunk-local surface voxels' attributes.
fn attr_units(n: u64) -> u64 {
    u64::from(surface_attr_units(n as u32))
}

/// `Full` bricks of a census bin of materials (`BrickCensus::BINS`) as slots of each bin.
fn census_bins(c: &BrickCensus) -> [u64; 10] {
    const UNITS: [u64; 6] = [2, 3, 4, 6, 18, 18];
    let mut out = [0u64; 10];
    for (n, units) in c.full_by_materials.iter().zip(UNITS) {
        out[bin_of(units as u32)] += n;
    }
    out
}

/// Prints one look's projected mirror: the pool by bin (slots of each bin times its units),
/// `records` records, `table` chunk-table entries and the material rows.
fn projection(look: &str, bins: &[u64; 10], records: u64, table: u64, materials: usize) {
    let pool = pool_bytes(bins);
    let rec = records * RECORD_BYTES;
    let rows = (materials * MATERIAL_FLOATS * 4) as u64;
    let total = pool + rec + table * 4 + rows;
    let by_bin: Vec<String> = bins
        .iter()
        .zip(BINS)
        .filter(|(n, _)| **n > 0)
        .map(|(n, b)| format!("{b}u x {n} = {} B", n * u64::from(b) * UNIT_BYTES))
        .collect();
    println!(
        "{:LABEL$}pool {pool} B ({:.1} MiB) [{}]; records {records} x {RECORD_BYTES} = {rec} B; \
         table {} B; materials {rows} B; total {total} B ({:.1} MiB)",
        format!("sized ({look}):"),
        mib(pool),
        by_bin.join(", "),
        table * 4,
        mib(total)
    );
}

/// Bytes of the slots in `bins`, each its bin's units.
fn pool_bytes(bins: &[u64; BIN_COUNT]) -> u64 {
    bins.iter()
        .zip(BINS)
        .map(|(n, b)| n * u64::from(b) * UNIT_BYTES)
        .sum()
}

/// Pool units of `bins` in whole pages of each bin.
fn page_units(bins: &[u64; BIN_COUNT]) -> u64 {
    let pages: u64 = bins
        .iter()
        .enumerate()
        .map(|(k, &n)| n.div_ceil(u64::from(slots_per_page(k))))
        .sum();
    pages * u64::from(PAGE_UNITS)
}

/// Adds brick `i` of `bricks` (a chunk's, or a mip level's), when `Full`, to `bins`: its
/// units, with its chunk-local surface's contour attributes when `attrs`.
fn add_slot(
    bins: &mut [u64; BIN_COUNT],
    bricks: &[Brick],
    i: usize,
    table: &MaterialTable,
    attrs: bool,
) {
    if let Brick::Full(p) = &bricks[i] {
        let glass = |m| table.flags(m).contains(MaterialFlags::TRANSLUCENT);
        let surface = attrs.then(|| {
            let s = chunk_surface(bricks, i);
            s.iter().map(|w| w.count_ones()).sum()
        });
        bins[bin_of(slot_units(p, glass, surface))] += 1;
    }
}

/// Width of the label column the figures are printed after.
const LABEL: usize = 19;

#[allow(
    clippy::disallowed_methods,
    reason = "a budget tool timing generation; the world itself reads no clock"
)]
fn now() -> std::time::Instant {
    std::time::Instant::now()
}

fn main() {
    let mut chunks = [20, 20, 8];
    let mut seed = 1u64;
    let mut stream = None;
    let mut census = false;
    let mut veins = None;
    let mut surface = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--census" {
            census = true;
            continue;
        }
        if a == "--surface" {
            surface = true;
            continue;
        }
        let v = args.next().unwrap_or_else(|| usage(&a));
        match a.as_str() {
            "--veins" => {
                let n: u64 = v.parse().unwrap_or_else(|_| usage(&v));
                veins = Some(if n >= 2 { n } else { usage(&v) });
            }
            "--chunks" => {
                let parts: Vec<i32> = v.split(['x', ',']).filter_map(|p| p.parse().ok()).collect();
                chunks = parts.try_into().unwrap_or_else(|_| usage(&v));
            }
            "--seed" => seed = v.parse().unwrap_or_else(|_| usage(&v)),
            "--stream" => {
                let parts: Vec<i32> = v.split(',').filter_map(|p| p.parse().ok()).collect();
                let [r_full, r_lod]: [i32; 2] = parts.try_into().unwrap_or_else(|_| usage(&v));
                stream = Some(Residency {
                    r_full,
                    r_lod,
                    hysteresis: 1,
                });
            }
            _ => usage(&a),
        }
    }
    match (veins, stream) {
        (Some(n), _) => veined(seed, n, chunks),
        (None, Some(r)) => streamed(seed, r, census, surface),
        (None, None) if surface => usage("--surface"),
        (None, None) => bounded(seed, chunks, census),
    }
}

/// `--veins N`: air where a seeded value noise of 16-voxel cells is below -4600 (about a third
/// of the voxels), otherwise material `1 + hash3(seed, cell) % (N - 1)` per 2^3 cell. 64 cells
/// drawing from `N - 1` ids give a solid brick about 7, 15 and 40 distinct materials at `N` 8,
/// 16 and 64. `vxe-world`'s `tests/census.rs` holds the same source and asserts the majority
/// bin.
struct Veins {
    seed: u64,
    n: u64,
    noise: ValueNoise,
}

impl WorldSource for Veins {
    fn chunk(&self, coord: ChunkCoord, out: &mut Chunk) {
        let base = coord.min_voxel().0;
        for slot in 0..CHUNK_BRICKS {
            let b = brick_of_slot(slot);
            out.set_brick(
                slot,
                Brick::from_fn(|l| {
                    let v = [0, 1, 2].map(|k| base[k] + b[k] * BRICK_EDGE + l[k]);
                    if self.noise.sample(v) < -4600 {
                        AIR
                    } else {
                        (1 + hash3(self.seed, v.map(|c| c >> 1)) % (self.n - 1)) as MaterialId
                    }
                }),
            );
        }
    }
}

fn veined(seed: u64, n: u64, chunks: [i32; 3]) {
    let extent = ChunkExtent::new([0, 0, 0], chunks);
    let mut table = MaterialTable::new();
    for i in 1..n {
        table.push(Material::named(format!("m{i}")));
    }
    let source = Veins {
        seed,
        n,
        noise: ValueNoise::new(seed ^ 0xA1A1, 4),
    };
    // The air count spans the whole extent, wider than a default box query allows.
    let size = chunks.map(|c| c.unsigned_abs() * CHUNK_EDGE.unsigned_abs());
    let limits = WorldLimits {
        query_axis_max: size.into_iter().max().unwrap_or(0),
        query_voxels_max: size.iter().map(|&n| u64::from(n)).product(),
        ..WorldLimits::DEFAULT
    };
    let world = World::generate(extent, &source, &table).with_limits(limits);
    let s = world.stats();
    let air = world
        .count_in_box(
            extent.min.min_voxel(),
            vxe_world::VoxelCoord(extent.max.min_voxel().0.map(|v| v - 1)),
        )
        .expect("the world's limits hold its whole extent");
    let air = air.iter().find(|(m, _)| *m == AIR).map_or(0, |&(_, n)| n);
    println!(
        "{:LABEL$}{}x{}x{} chunks, seed {seed}, {} materials",
        "veins:",
        chunks[0],
        chunks[1],
        chunks[2],
        n - 1
    );
    println!(
        "{:LABEL$}{} Full, {} Uniform",
        "bricks:", s.full_bricks, s.uniform_bricks
    );
    println!(
        "air fraction:      {:.2} %",
        100.0 * air as f64 / (s.chunks as f64 * 64.0 * 64.0 * 64.0)
    );
    let census = world.brick_census();
    println!("{census}");
    if let Some(b) = census.bricks.majority_bin() {
        println!("{:LABEL$}{}", "majority bin:", BrickCensus::BINS[b]);
    }
}

fn bounded(seed: u64, chunks: [i32; 3], census: bool) {
    let extent = ChunkExtent::new([0, 0, 0], chunks);
    let source = CaveSource::new(seed, extent);
    let table = cave_materials();
    let threads = std::thread::available_parallelism().map_or(1, usize::from);

    let start = now();
    // The air count below spans the whole extent, wider than a default box query allows.
    let size = chunks.map(|c| c.unsigned_abs() * CHUNK_EDGE.unsigned_abs());
    let limits = WorldLimits {
        query_axis_max: size.into_iter().max().unwrap_or(0),
        query_voxels_max: size.iter().map(|&n| u64::from(n)).product(),
        ..WorldLimits::DEFAULT
    };
    let world = World::generate_with_threads(extent, &source, &table, threads).with_limits(limits);
    let secs = (now() - start).as_secs_f64();

    let start = now();
    let grid = CoarseGrid::build(&world);
    let grid_secs = (now() - start).as_secs_f64();

    let s = world.stats();
    let bricks = s.full_bricks + s.uniform_bricks;
    let slot_array = CHUNK_BRICKS * size_of::<Brick>();
    let mut payload = 0usize;
    let (mut face, mut contour) = ([0u64; BIN_COUNT], [0u64; BIN_COUNT]);
    for c in world.store().coords() {
        let chunk = world.store().get(c).expect("resident");
        let chunk = chunk.read().expect("chunk lock");
        payload += chunk
            .bricks()
            .iter()
            .map(Brick::payload_bytes)
            .sum::<usize>();
        for i in 0..CHUNK_BRICKS {
            add_slot(&mut face, chunk.bricks(), i, &table, false);
            add_slot(&mut contour, chunk.bricks(), i, &table, true);
        }
    }
    let cpu = payload + s.chunks * slot_array;
    let mirror = pool_bytes(&face) as usize
        + s.chunks * (RECORD_BYTES as usize + 4)
        + table.len() * MATERIAL_FLOATS * 4;
    let voxels = s.chunks as f64 * 64.0 * 64.0 * 64.0;
    let air = world
        .count_in_box(
            extent.min.min_voxel(),
            vxe_world::VoxelCoord(extent.max.min_voxel().0.map(|v| v - 1)),
        )
        .expect("the world's limits hold its whole extent");
    let air = air.iter().find(|(m, _)| *m == 0).map_or(0, |&(_, n)| n);

    println!(
        "{:LABEL$}{}x{}x{} chunks, seed {seed}",
        "extent:", chunks[0], chunks[1], chunks[2]
    );
    println!("{:LABEL$}{}", "chunks:", s.chunks);
    println!(
        "{:LABEL$}{} Full, {} Uniform",
        "bricks:", s.full_bricks, s.uniform_bricks
    );
    println!(
        "full fraction:     {:.2} %",
        100.0 * s.full_bricks as f64 / bricks as f64
    );
    println!("air fraction:      {:.2} %", 100.0 * air as f64 / voxels);
    println!("cpu bytes:         {} ({:.1} MiB)", cpu, mib(cpu as u64));
    println!(
        "mirror bytes:      {} ({:.1} MiB)",
        mirror,
        mib(mirror as u64)
    );
    budget_lines(&face, &contour, s.chunks as u64);
    println!("generation:        {:.3} s on {threads} threads", secs);
    println!("voxels/s/thread:   {:.3e}", voxels / secs / threads as f64);
    println!(
        "coarse grid:       {} bytes ({:.1} MiB), {} per chunk",
        grid.bytes(),
        mib(grid.bytes() as u64),
        grid.bytes() / grid.len().max(1)
    );
    println!("coarse build:      {:.1} ms on 1 thread", grid_secs * 1e3);
    // The mip build reads each `Full` brick through `decode`: timed per chunk, one thread.
    let start = now();
    for c in world.store().coords() {
        let chunk = world.store().get(c).expect("resident");
        let chunk = chunk.read().expect("chunk lock");
        std::hint::black_box(ChunkMips::build(&chunk));
    }
    let mips_secs = (now() - start).as_secs_f64();
    println!(
        "mip build:         {:.3} ms per chunk on 1 thread ({} chunks)",
        mips_secs * 1e3 / s.chunks as f64,
        s.chunks
    );
    if census {
        let c = world.brick_census();
        println!("{c}");
        let n = chunks.iter().map(|&v| v as u64).product::<u64>();
        projection(
            "plain",
            &census_bins(&c.total()),
            s.chunks as u64,
            n,
            table.len(),
        );
    }
}

fn streamed(seed: u64, r: Residency, census: bool, surface: bool) {
    let source = CaveSource::unbounded(seed);
    let table = cave_materials();
    let workers = std::thread::available_parallelism().map_or(1, usize::from);
    let world = World::new(&table);
    let mut streamer = Streamer::new(
        Arc::new(source),
        StreamConfig {
            residency: r,
            workers,
            persist: PersistConfig::default(),
        },
    );
    let start = now();
    streamer.update(&world, &[source.spawn()]);
    streamer.wait_idle(&world);
    let secs = (now() - start).as_secs_f64();

    let st = streamer.stats();
    let (mut full_bricks, mut uniform_bricks, mut cpu_full) = (0usize, 0usize, 0u64);
    let (mut face, mut contour, mut mip_bins) =
        ([0u64; BIN_COUNT], [0u64; BIN_COUNT], [0u64; BIN_COUNT]);
    for c in world.store().coords() {
        let chunk = world.store().get(c).expect("resident");
        let chunk = chunk.read().expect("chunk lock");
        let full = chunk.bricks().iter().filter(|b| b.is_full()).count();
        full_bricks += full;
        uniform_bricks += CHUNK_BRICKS - full;
        cpu_full += chunk_ram(&chunk);
        for i in 0..CHUNK_BRICKS {
            add_slot(&mut face, chunk.bricks(), i, &table, false);
            add_slot(&mut contour, chunk.bricks(), i, &table, true);
        }
    }
    let (mut mip_full, mut cpu_lod) = (0usize, 0u64);
    for c in world.lod_coords() {
        let mips = world.lod(c).expect("LOD-only");
        mip_full += mips.full_bricks();
        cpu_lod += mips_ram(&mips);
        for n in 1..=vxe_world::MIP_LEVELS {
            let level = mips.level(n);
            for i in 0..level.len() {
                add_slot(&mut mip_bins, level, i, &table, false);
            }
        }
    }
    let w = window_side(r) as usize;
    let mirror_full = pool_bytes(&face) as usize + st.resident_full * RECORD_BYTES as usize;
    let mirror_lod = pool_bytes(&mip_bins) as usize + st.resident_lod * RECORD_BYTES as usize;
    let mirror = mirror_full + mirror_lod + w * w * w * 4 + table.len() * MATERIAL_FLOATS * 4;
    let generated = st.generated_chunks as f64 * 64.0 * 64.0 * 64.0;

    println!(
        "residency:         r_full {}, r_lod {}, h {}, seed {seed}, window {w}",
        r.r_full, r.r_lod, r.hysteresis
    );
    println!(
        "{:LABEL$}{} full, {} LOD-only",
        "chunks:", st.resident_full, st.resident_lod
    );
    println!(
        "full fraction:     {:.2} % of full chunks' bricks",
        100.0 * full_bricks as f64 / (full_bricks + uniform_bricks) as f64
    );
    println!(
        "mip-bricks:        {mip_full} Full of {} ({:.2} %)",
        st.resident_lod * 73,
        100.0 * mip_full as f64 / (st.resident_lod * 73).max(1) as f64
    );
    println!(
        "cpu bytes:         {} ({:.1} MiB): full {}, LOD {}",
        cpu_full + cpu_lod,
        mib(cpu_full + cpu_lod),
        cpu_full,
        cpu_lod
    );
    println!(
        "mirror bytes:      {} ({:.1} MiB): full {}, LOD {}",
        mirror,
        mib(mirror as u64),
        mirror_full,
        mirror_lod
    );
    for k in 0..BIN_COUNT {
        face[k] += mip_bins[k];
        contour[k] += mip_bins[k];
    }
    budget_lines(&face, &contour, stream_record_chunks(r));
    println!("generation:        {secs:.3} s on {workers} workers");
    println!(
        "voxels/s/worker:   {:.3e}",
        generated / secs / workers as f64
    );
    println!(
        "| {} | {} | {} | {} | {:.1} | {:.1} | {:.1} | {:.1} |",
        r.r_full,
        r.r_lod,
        st.resident_full,
        st.resident_lod,
        mib(cpu_full + cpu_lod),
        mib(mirror_full as u64),
        mib(mirror_lod as u64),
        mib(mirror as u64)
    );
    let records = (st.resident_full + st.resident_lod) as u64;
    let table_entries = (w * w * w) as u64;
    if census || surface {
        let c = world.brick_census();
        if census {
            println!("{c}");
        }
        projection(
            "plain",
            &census_bins(&c.total()),
            records,
            table_entries,
            table.len(),
        );
        if surface {
            let t = surface_census(&world, &source, workers, mirror as u64);
            sized_contour(&t, &c, records, table_entries, table.len());
        }
    }
}

/// The per-class table of the full chunks' `Full` bricks and the contour look's projection:
/// their slots with attributes, plus the mip-bricks' plain slots.
fn sized_contour(t: &Census, c: &WorldCensus, records: u64, table: u64, materials: usize) {
    println!(
        "| class | units | bricks | plane voxels (exact) | surface voxels (chunk-local) | surface / exact | attribute units (mask + attrs) |"
    );
    println!("|---|---|---|---|---|---|---|");
    let (mut planes, mut surface, mut units) = (0u64, 0u64, 0u64);
    for (k, row) in t.by_class.iter().enumerate() {
        let class = PackedClass::ALL[k];
        planes += row.planes;
        surface += row.surface;
        units += row.attr_units;
        println!(
            "| {class:?} | {} | {} | {} | {} | {:.3} | {} |",
            class_units(class),
            row.bricks,
            row.planes,
            row.surface,
            row.surface as f64 / row.planes.max(1) as f64,
            row.attr_units
        );
    }
    println!(
        "| all | | {} | {planes} | {surface} | {:.3} | {units} |",
        t.bricks,
        surface as f64 / planes.max(1) as f64
    );
    println!(
        "{:LABEL$}{:.1} per brick, {:.2} attribute units per brick ({:.0} B)",
        "surface voxels:",
        surface as f64 / t.bricks.max(1) as f64,
        units as f64 / t.bricks.max(1) as f64,
        (units * UNIT_BYTES) as f64 / t.bricks.max(1) as f64
    );
    let mips = census_bins(&c.mip_bricks);
    let mut bins = t.contour_bins;
    for (a, b) in bins.iter_mut().zip(mips) {
        *a += b;
    }
    projection("contour", &bins, records, table, materials);
}

/// Bytes of the contour attribute per `Full` brick of a full chunk in attribute pages, before
/// size-classed slots: two 512-byte slots and their two 72-byte mask entries.
const ATTR_PER_BRICK: u64 = 2 * (512 + 72);
/// The same with the 8-byte mask entries the mirror had when Plan 0005 read its live rise.
const ATTR_PER_BRICK_8B_MASK: u64 = 2 * (512 + 8);
/// Plan 0005's live rise of the streamed mirror at r_full 16, r_lod 24 with attribute pages.
const PLAN_0005_RISE: u64 = 1_705_903_808;

/// The size-classed census of one packed class's bricks.
#[derive(Clone, Copy, Default)]
struct ClassRow {
    bricks: u64,
    planes: u64,
    /// Chunk-local surface voxels.
    surface: u64,
    /// Units of surface mask and attributes.
    attr_units: u64,
}

/// What the census counts over a set of bricks.
#[derive(Clone, Copy, Default)]
struct Census {
    bricks: u64,
    planes: u64,
    blocks: u64,
    /// Bricks by plane voxels, in bins of 32 (bin 16 holds exactly 512).
    hist: [u64; 17],
    /// By `PackedClass::ALL`.
    by_class: [ClassRow; 5],
    /// Slots of each bin of `MIRROR_BINS` the bricks take with their attributes.
    contour_bins: [u64; 10],
}

impl Census {
    fn add(&mut self, o: &Census) {
        self.bricks += o.bricks;
        self.planes += o.planes;
        self.blocks += o.blocks;
        for (a, b) in self.hist.iter_mut().zip(&o.hist) {
            *a += b;
        }
        for (a, b) in self.by_class.iter_mut().zip(&o.by_class) {
            a.bricks += b.bricks;
            a.planes += b.planes;
            a.surface += b.surface;
            a.attr_units += b.attr_units;
        }
        for (a, b) in self.contour_bins.iter_mut().zip(&o.contour_bins) {
            *a += b;
        }
    }
}

/// Chunks generated from the source past the residency, for neighbours across its edge.
type SourceCache = Mutex<HashMap<ChunkCoord, Arc<RwLock<Chunk>>>>;

/// The resident chunk at `c`, or the source's, generated once.
fn chunk_or_source(
    world: &World,
    source: &CaveSource,
    cache: &SourceCache,
    c: ChunkCoord,
) -> Arc<RwLock<Chunk>> {
    if let Some(chunk) = world.store().get(c) {
        return chunk;
    }
    let mut cache = cache.lock().expect("cache lock");
    cache
        .entry(c)
        .or_insert_with(|| {
            let mut chunk = Chunk::default();
            source.chunk(c, &mut chunk);
            Arc::new(RwLock::new(chunk))
        })
        .clone()
}

/// The census of chunk `c`'s `Full` bricks.
fn census_chunk(world: &World, source: &CaveSource, cache: &SourceCache, c: ChunkCoord) -> Census {
    const FACES: [[i32; 3]; 6] = [
        [-1, 0, 0],
        [1, 0, 0],
        [0, -1, 0],
        [0, 1, 0],
        [0, 0, -1],
        [0, 0, 1],
    ];
    let here = world.store().get(c).expect("resident");
    let here = here.read().expect("chunk lock");
    if !here.bricks().iter().any(Brick::is_full) {
        return Census::default();
    }
    // The chunk and its six face neighbours, indexed 0 (here) and 1 + face.
    let arcs: Vec<Arc<RwLock<Chunk>>> = FACES
        .iter()
        .map(|d| {
            chunk_or_source(
                world,
                source,
                cache,
                ChunkCoord([0, 1, 2].map(|k| c.0[k] + d[k])),
            )
        })
        .collect();
    let guards: Vec<RwLockReadGuard<'_, Chunk>> =
        arcs.iter().map(|a| a.read().expect("chunk lock")).collect();
    let e = BRICK_EDGE;
    let chunk_bricks = 64 / e;
    // The brick at chunk-local brick coordinate `b`, which may lie one brick past a face.
    let brick_at = |b: [i32; 3]| -> &Brick {
        let d = b.map(|v| v.div_euclid(chunk_bricks));
        let local = b.map(|v| v.rem_euclid(chunk_bricks));
        let slot = brick_slot(local);
        match FACES.iter().position(|f| *f == d) {
            Some(k) => guards[k].brick(slot),
            None => here.brick(slot),
        }
    };
    let mut out = Census::default();
    for slot in 0..CHUNK_BRICKS {
        let brick = here.brick(slot);
        if !brick.is_full() {
            continue;
        }
        let b = brick_of_slot(slot);
        // Occupancy of the brick and its six face layers, 10 voxels to an edge.
        let mut occ = [false; 1000];
        let at = |x: i32, y: i32, z: i32| ((x + 1) + 10 * ((y + 1) + 10 * (z + 1))) as usize;
        for z in 0..e {
            for y in 0..e {
                for x in 0..e {
                    occ[at(x, y, z)] = brick.get(index_in_brick([x, y, z])) != AIR;
                }
            }
        }
        for d in FACES {
            let nb = brick_at([0, 1, 2].map(|k| b[k] + d[k]));
            for u in 0..e {
                for w in 0..e {
                    // The neighbour's layer touching this brick, and where it sits in `occ`.
                    let mut l = [0; 3];
                    let mut o = [0; 3];
                    let axis = d.iter().position(|&v| v != 0).expect("a face");
                    let (a1, a2) = ((axis + 1) % 3, (axis + 2) % 3);
                    l[a1] = u;
                    l[a2] = w;
                    o[a1] = u;
                    o[a2] = w;
                    l[axis] = if d[axis] > 0 { 0 } else { e - 1 };
                    o[axis] = if d[axis] > 0 { e } else { -1 };
                    occ[at(o[0], o[1], o[2])] = nb.get(index_in_brick(l)) != AIR;
                }
            }
        }
        let mut planes = 0u64;
        let mut blocks = 0u64;
        for i in 0..64 {
            let k = [i % 4, (i / 4) % 4, i / 16].map(|v| v * 2);
            let mut any = false;
            for j in 0..8 {
                let v = [k[0] + (j & 1), k[1] + ((j >> 1) & 1), k[2] + (j >> 2)];
                if !occ[at(v[0], v[1], v[2])] {
                    continue;
                }
                let exposed = FACES
                    .iter()
                    .any(|f| !occ[at(v[0] + f[0], v[1] + f[1], v[2] + f[2])]);
                if exposed {
                    planes += 1;
                    any = true;
                }
            }
            blocks += u64::from(any);
        }
        out.bricks += 1;
        out.planes += planes;
        out.blocks += blocks;
        out.hist[(planes / 32) as usize] += 1;
        // The chunk-local surface: neighbours inside this chunk only.
        let beside: [[Option<[u64; 8]>; 2]; 3] = std::array::from_fn(|k| {
            std::array::from_fn(|d| {
                let mut n = b;
                n[k] += if d == 0 { -1 } else { 1 };
                (0..chunk_bricks)
                    .contains(&n[k])
                    .then(|| solid_mask(here.brick(brick_slot(n))))
            })
        });
        let n = surface_mask(&solid_mask(brick), &beside)
            .iter()
            .map(|w| u64::from(w.count_ones()))
            .sum::<u64>();
        let Brick::Full(p) = brick else {
            unreachable!("only Full bricks are counted")
        };
        let class = p.class();
        let row = &mut out.by_class[PackedClass::ALL
            .iter()
            .position(|&c| c == class)
            .expect("a class")];
        row.bricks += 1;
        row.planes += planes;
        row.surface += n;
        row.attr_units += attr_units(n);
        out.contour_bins[bin_of((class_units(class) + attr_units(n)) as u32)] += 1;
    }
    out
}

/// The plane-voxel census of the resident full chunks and the attribute bytes of each
/// footprint layout; `face_mirror` is the face-mode mirror estimate printed above.
fn surface_census(world: &World, source: &CaveSource, threads: usize, face_mirror: u64) -> Census {
    let start = now();
    let coords = world.store().coords();
    let cache = SourceCache::default();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let total = Mutex::new(Census::default());
    std::thread::scope(|s| {
        for _ in 0..threads.max(1) {
            s.spawn(|| {
                let mut mine = Census::default();
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(&c) = coords.get(i) else {
                        break;
                    };
                    mine.add(&census_chunk(world, source, &cache, c));
                }
                total.lock().expect("total lock").add(&mine);
            });
        }
    });
    let t = *total.lock().expect("total lock");
    let secs = (now() - start).as_secs_f64();
    let n = t.bricks;
    println!(
        "surface census:    {} Full bricks of full chunks, {} source chunks past the residency, {secs:.1} s",
        n,
        cache.lock().expect("cache lock").len()
    );
    println!(
        "attribute bytes:   {} at {ATTR_PER_BRICK} B per brick; at {ATTR_PER_BRICK_8B_MASK} B {} against Plan 0005's live rise {PLAN_0005_RISE} ({:+.3} %)",
        n * ATTR_PER_BRICK,
        n * ATTR_PER_BRICK_8B_MASK,
        100.0 * ((n * ATTR_PER_BRICK_8B_MASK) as f64 / PLAN_0005_RISE as f64 - 1.0)
    );
    println!(
        "plane voxels:      {} ({:.1} per brick, {:.2} % of the bricks' voxels)",
        t.planes,
        t.planes as f64 / n.max(1) as f64,
        100.0 * t.planes as f64 / (n.max(1) * BRICK_VOXELS as u64) as f64
    );
    println!(
        "plane blocks:      {} of {} 2x2x2 blocks ({:.2} %)",
        t.blocks,
        n * 64,
        100.0 * t.blocks as f64 / (n.max(1) * 64) as f64
    );
    println!("plane voxels per brick (bricks per 32-voxel bin):");
    for (k, &count) in t.hist.iter().enumerate() {
        if count > 0 {
            println!(
                "  {:>3}..{:<3}  {count:>9}  {:6.2} %",
                k * 32,
                k * 32 + 31,
                100.0 * count as f64 / n.max(1) as f64
            );
        }
    }
    let rows = [
        ("attribute pages (2 x 512 B + 2 x 72 B)", n * ATTR_PER_BRICK),
        ("A (64 B mask + 2 B per plane voxel)", n * 64 + 2 * t.planes),
        ("B (512 B code + 72 B mask)", n * 584),
        ("C (64 x u16 per brick)", n * 128),
    ];
    println!("| layout | attribute bytes live | per brick | face mirror + attributes |");
    println!("|---|---|---|---|");
    for (name, bytes) in rows {
        println!(
            "| {name} | {bytes} | {:.1} | {} |",
            bytes as f64 / n.max(1) as f64,
            face_mirror + bytes
        );
    }
    t
}

/// Prints the budget the slots of each look need, by bin (`face` without attributes,
/// `contour` with them on full chunks' bricks), with a record for each of `records` chunks.
fn budget_lines(face: &[u64; BIN_COUNT], contour: &[u64; BIN_COUNT], records: u64) {
    let face = budget_for(page_units(face), records);
    let contour = budget_for(page_units(contour), records);
    println!("budget (face):     {face} B ({:.1} MiB)", mib(face));
    println!("budget (contour):  {contour} B ({:.1} MiB)", mib(contour));
}

fn mib(b: u64) -> f64 {
    b as f64 / (1024.0 * 1024.0)
}

fn usage(bad: &str) -> ! {
    eprintln!(
        "bad argument {bad:?}; usage: world_budget [--chunks AxBxC | --stream R_FULL,R_LOD | --veins N] [--seed N] [--census] [--surface]"
    );
    std::process::exit(2)
}
