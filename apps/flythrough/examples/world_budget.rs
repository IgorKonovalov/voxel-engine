//! Prints the cave's storage figures (`nfr.md` §3, §5), bounded or streamed, and for the
//! bounded cave the coarse grid's bytes (4 per brick) and single-threaded build time.
//!
//! ```text
//! cargo run --release -p flythrough --example world_budget -- --chunks 20x20x8 --seed 1
//! cargo run --release -p flythrough --example world_budget -- --stream 16,24 --seed 1
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
//! Mirror bytes use the uploader's formula (ADR-0004): each `Full` brick or mip-brick takes a
//! 512-byte slot of palette indices plus 72 bytes of masks (a 64-byte opaque mask and two
//! translucent words), each mirrored chunk (full or LOD-only) a 2624-byte record (16 words of
//! brick bits, 512 entries, the palette), each chunk-table entry 4 bytes (the window's `W^3`
//! when streaming), and each material row 8 `f32`s. CPU bytes count a full chunk's slot array and
//! its `Full` bricks' class boxes, and a LOD-only chunk's mip-brick slots and `Full` mip-bricks'
//! boxes.

use std::sync::Arc;

use flythrough::cave::CaveSource;
use flythrough::{cave_materials, chunk_ram, mips_ram, window_side};
use vxe_world::hash::{ValueNoise, hash3};
use vxe_world::{
    AIR, BRICK_EDGE, Brick, BrickCensus, CHUNK_BRICKS, Chunk, ChunkCoord, ChunkExtent, CoarseGrid,
    Material, MaterialId, MaterialTable, PersistConfig, Residency, StreamConfig, Streamer, World,
    WorldSource, brick_of_slot,
};

/// Mirror bytes per `Full` brick or mip-brick, and per mirrored chunk's record.
const SLOT_AND_MASK: usize = 512 + 72;
const RECORD: usize = 2624;

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
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--census" {
            census = true;
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
        (None, Some(r)) => streamed(seed, r, census),
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
    let world = World::generate(extent, &source, &table);
    let s = world.stats();
    let air = world.count_in_box(
        extent.min.min_voxel(),
        vxe_world::VoxelCoord(extent.max.min_voxel().0.map(|v| v - 1)),
    );
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
    let world = World::generate_with_threads(extent, &source, &table, threads);
    let secs = (now() - start).as_secs_f64();

    let start = now();
    let grid = CoarseGrid::build(&world);
    let grid_secs = (now() - start).as_secs_f64();

    let s = world.stats();
    let bricks = s.full_bricks + s.uniform_bricks;
    let slot_array = CHUNK_BRICKS * size_of::<Brick>();
    let mut payload = 0usize;
    for c in world.store().coords() {
        let chunk = world.store().get(c).expect("resident");
        let chunk = chunk.read().expect("chunk lock");
        payload += chunk
            .bricks()
            .iter()
            .map(Brick::payload_bytes)
            .sum::<usize>();
    }
    let cpu = payload + s.chunks * slot_array;
    let mirror = s.full_bricks * SLOT_AND_MASK + s.chunks * (RECORD + 4) + table.len() * 8 * 4;
    let voxels = s.chunks as f64 * 64.0 * 64.0 * 64.0;
    let air = world.count_in_box(
        extent.min.min_voxel(),
        vxe_world::VoxelCoord(extent.max.min_voxel().0.map(|v| v - 1)),
    );
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
    println!("generation:        {:.3} s on {threads} threads", secs);
    println!("voxels/s/thread:   {:.3e}", voxels / secs / threads as f64);
    println!(
        "coarse grid:       {} bytes ({:.1} MiB), {} per chunk",
        grid.bytes(),
        mib(grid.bytes() as u64),
        grid.bytes() / grid.len().max(1)
    );
    println!("coarse build:      {:.1} ms on 1 thread", grid_secs * 1e3);
    if census {
        println!("{}", world.brick_census());
    }
}

fn streamed(seed: u64, r: Residency, census: bool) {
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
    for c in world.store().coords() {
        let chunk = world.store().get(c).expect("resident");
        let chunk = chunk.read().expect("chunk lock");
        let full = chunk.bricks().iter().filter(|b| b.is_full()).count();
        full_bricks += full;
        uniform_bricks += CHUNK_BRICKS - full;
        cpu_full += chunk_ram(&chunk);
    }
    let (mut mip_full, mut cpu_lod) = (0usize, 0u64);
    for c in world.lod_coords() {
        let mips = world.lod(c).expect("LOD-only");
        mip_full += mips.full_bricks();
        cpu_lod += mips_ram(&mips);
    }
    let w = window_side(r) as usize;
    let mirror_full = full_bricks * SLOT_AND_MASK + st.resident_full * RECORD;
    let mirror_lod = mip_full * SLOT_AND_MASK + st.resident_lod * RECORD;
    let mirror = mirror_full + mirror_lod + w * w * w * 4 + table.len() * 8 * 4;
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
    if census {
        println!("{}", world.brick_census());
    }
}

fn mib(b: u64) -> f64 {
    b as f64 / (1024.0 * 1024.0)
}

fn usage(bad: &str) -> ! {
    eprintln!(
        "bad argument {bad:?}; usage: world_budget [--chunks AxBxC | --stream R_FULL,R_LOD | --veins N] [--seed N] [--census]"
    );
    std::process::exit(2)
}
